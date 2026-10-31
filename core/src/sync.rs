//! The sync engine: one device's conversation with the relay about one
//! group (`docs/01-deappje/02-relay.md`, "Push", "Pull and ack", "The path
//! of a new device", "History requests"), over the local store and the
//! keyring. Pulls apply rows and the cascade; the outbox is pushed and its
//! conflicts rebased (`crate::rebase`); a device below the floor starts
//! over and takes the new device's path; a holder serves history. The
//! shell calls [`Syncer::sync_group`] on a wake-up, a timer or a user's
//! pull, and [`Syncer::history_ready`] when the `history_ready` wake-up
//! says a holder pushed; a device catching up reads its request on every
//! sync as well, so a wake-up that never arrives (a browser gets none)
//! costs only the wait for the next sync.
//!
//! Nothing here waits: every call to the relay is one request, and the
//! engine returns between them when an outage stops it, with the store
//! consistent, to be called again.

use std::collections::BTreeMap;
use std::fmt;

use uuid::Uuid;

use crate::crypto::CryptoError;
use crate::envelope::{Envelope, EnvelopeError, Header};
use crate::keyring::Keyring;
use crate::rebase;
use crate::api::ApiError;
use crate::relay::{Conflict, Entry, HistoryStatus, PushEntry, PushResult, Relay};
use crate::rng::CoreRng;
use crate::schema::{Message as _, Meta, ObjectType};
use crate::store::{Op, Pending, Phase, Row, Store};
use crate::wire;

/// How many rebases one sync tries per push before the outbox waits for
/// the next.
pub const REBASE_ATTEMPTS: u32 = 3;
/// How many rows a holder pushes per history call.
pub const HISTORY_PAGE: usize = 2000;

/// What a sync did, for the shell to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A row changed from the server.
    Applied {
        /// The object.
        id: Uuid,
        /// Its seq.
        seq: i64,
        /// Whether it is a tombstone now.
        deleted: bool,
    },
    /// A write of this device was accepted.
    Accepted {
        /// The object.
        id: Uuid,
        /// Its seq.
        seq: i64,
    },
    /// A write was dropped because the object was deleted meanwhile.
    EditDropped {
        /// The object.
        id: Uuid,
    },
    /// A rebase produced an invalid object: the write waits for the user.
    Invalid {
        /// The object.
        id: Uuid,
        /// Why.
        reason: String,
    },
    /// The device was below the floor and threw the group away.
    StartedOver,
    /// The device holds the group from seq 1.
    BecameHolder,
    /// A holder was asked for everything older than the window.
    HistoryRequested {
        /// The request.
        request_id: Uuid,
    },
    /// Nobody holds anything older than the window.
    Unfillable,
    /// The history asked for was in the window already (a restore for
    /// another device, not yet evicted): no holder was needed.
    HistoryPresent,
    /// A row could not be opened: the key is not held yet.
    NoKey {
        /// The object.
        id: Uuid,
        /// The key generation.
        key_id: u32,
    },
    /// This device served a history request.
    HistoryServed {
        /// The request.
        request_id: Uuid,
        /// Rows pushed.
        rows: usize,
    },
}

/// Why a sync stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// The relay refused or could not be reached.
    Relay(ApiError),
    /// A row the engine had to open (a conflict's current row) is sealed
    /// under a key the device does not hold.
    NoKey {
        /// The group.
        group_id: Uuid,
        /// The key generation.
        key_id: u32,
    },
    /// A value that had to be opened did not.
    Crypto(CryptoError),
    /// A value is not an envelope.
    Envelope(EnvelopeError),
    /// A write names an object the device does not hold.
    Unknown {
        /// The object.
        id: Uuid,
    },
    /// The device holds no key of the group: it cannot write.
    NoGroupKey {
        /// The group.
        group_id: Uuid,
    },
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Relay(e) => write!(f, "{e}"),
            SyncError::NoKey { group_id, key_id } => write!(f, "no key {key_id} of group {group_id}"),
            SyncError::Crypto(e) => write!(f, "{e}"),
            SyncError::Envelope(e) => write!(f, "{e}"),
            SyncError::Unknown { id } => write!(f, "object {id} is not held"),
            SyncError::NoGroupKey { group_id } => write!(f, "no key of group {group_id}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<ApiError> for SyncError {
    fn from(e: ApiError) -> Self {
        SyncError::Relay(e)
    }
}

/// The plaintext the user sees of an object: the pending write if there
/// is one, else the row opened under its key.
pub fn read_plain(store: &Store, keyring: &Keyring, group_id: Uuid, id: Uuid) -> Result<Option<Vec<u8>>, SyncError> {
    match store.pending(group_id, id).map(|p| &p.op) {
        Some(Op::Create { plain, .. } | Op::Edit { plain, .. }) => return Ok(Some(plain.clone())),
        Some(Op::Delete { .. }) => return Ok(None),
        None => {}
    }
    match store.row(group_id, id) {
        Some(Row { deleted: false, value: Some(value), .. }) => {
            let envelope = Envelope::decode(value).map_err(SyncError::Envelope)?;
            let key = keyring.get(group_id, envelope.header.key_id).ok_or(SyncError::NoKey { group_id, key_id: envelope.header.key_id })?;
            Ok(Some(envelope.open(key, group_id, id).map_err(SyncError::Crypto)?))
        }
        _ => Ok(None),
    }
}

/// Where a catch-up stands after a holder's progress report, from the
/// wake-up or from the request itself. Complete, the whole range is in:
/// every row at or below the floor that any holder has, whatever the last
/// pushed seq was (the range may end in rows nobody holds any more). Else
/// as far as the further of the two reports.
fn progress(floor: i64, so_far: i64, done: bool, through_seq: i64, complete: bool) -> (i64, bool) {
    let complete = done || complete;
    (if complete { floor } else { so_far.max(through_seq) }, complete)
}

/// The engine, over the device's store and keyring.
pub struct Syncer<'a> {
    /// This device.
    pub device_id: Uuid,
    /// The store.
    pub store: &'a mut Store,
    /// The keyring.
    pub keyring: &'a Keyring,
    /// The generator, for nonces.
    pub rng: &'a mut CoreRng,
}

impl Syncer<'_> {
    // ---- Opening and sealing ----

    fn open(&self, group_id: Uuid, id: Uuid, value: &[u8]) -> Result<(Header, Vec<u8>), SyncError> {
        let envelope = Envelope::decode(value).map_err(SyncError::Envelope)?;
        let key = self.keyring.get(group_id, envelope.header.key_id).ok_or(SyncError::NoKey { group_id, key_id: envelope.header.key_id })?;
        let plain = envelope.open(key, group_id, id).map_err(SyncError::Crypto)?;
        Ok((envelope.header, plain))
    }

    fn seal(&mut self, group_id: Uuid, id: Uuid, object_type: ObjectType, plain: &[u8]) -> Result<Vec<u8>, SyncError> {
        let (key_id, key) = self.keyring.latest(group_id).ok_or(SyncError::NoGroupKey { group_id })?;
        let header = Header { object_type: object_type.number(), schema_version: object_type.schema().version, key_id };
        Ok(Envelope::seal(key, group_id, id, header, plain, self.rng).encode())
    }

    /// The type and the parent of a value, for the index; none when the
    /// device cannot open it.
    fn index(&self, group_id: Uuid, id: Uuid, value: &[u8]) -> Option<(ObjectType, Option<Uuid>)> {
        let (header, plain) = self.open(group_id, id, value).ok()?;
        let object_type = ObjectType::from_number(header.object_type)?;
        let meta = Meta::decode(&plain).ok()?;
        Some((object_type, meta.parent_id))
    }

    // ---- The write side ----

    /// The plaintext the user sees: the pending write if there is one,
    /// else the row.
    pub fn read(&self, group_id: Uuid, id: Uuid) -> Result<Option<Vec<u8>>, SyncError> {
        read_plain(self.store, self.keyring, group_id, id)
    }

    /// A new object: in the store at once, pushed at the next sync.
    pub fn create(&mut self, group_id: Uuid, id: Uuid, object_type: ObjectType, plain: Vec<u8>) -> Result<(), SyncError> {
        let sealed = self.seal(group_id, id, object_type, &plain)?;
        let parent_id = Meta::decode(&plain).ok().and_then(|m| m.parent_id);
        self.store.ensure(group_id);
        self.store.put_row(group_id, id, Row { seq: 0, deleted: false, value: Some(sealed.clone()), object_type: Some(object_type), parent_id });
        self.store.put_pending(group_id, id, Pending { op: Op::Create { object_type, plain, sealed }, attempts: 0, invalid: None });
        Ok(())
    }

    /// A change to an object the device holds. An edit on top of a pending
    /// create or edit replaces its plaintext; the base stays what the
    /// device last saw from the server.
    pub fn edit(&mut self, group_id: Uuid, id: Uuid, plain: Vec<u8>) -> Result<(), SyncError> {
        let op = match self.store.pending(group_id, id).map(|p| p.op.clone()) {
            Some(Op::Create { object_type, .. }) => {
                let sealed = self.seal(group_id, id, object_type, &plain)?;
                let parent_id = Meta::decode(&plain).ok().and_then(|m| m.parent_id);
                self.store.put_row(group_id, id, Row { seq: 0, deleted: false, value: Some(sealed.clone()), object_type: Some(object_type), parent_id });
                Op::Create { object_type, plain, sealed }
            }
            Some(Op::Edit { object_type, base_seq, base_plain, .. }) => {
                let sealed = self.seal(group_id, id, object_type, &plain)?;
                Op::Edit { object_type, base_seq, base_plain, plain, sealed }
            }
            Some(Op::Delete { .. }) => return Err(SyncError::Unknown { id }),
            None => {
                let row = self.store.row(group_id, id).filter(|r| !r.deleted).ok_or(SyncError::Unknown { id })?;
                let value = row.value.clone().ok_or(SyncError::Unknown { id })?;
                let base_seq = row.seq;
                let (header, base_plain) = self.open(group_id, id, &value)?;
                let object_type = ObjectType::from_number(header.object_type).ok_or(SyncError::Unknown { id })?;
                let sealed = self.seal(group_id, id, object_type, &plain)?;
                Op::Edit { object_type, base_seq, base_plain, plain, sealed }
            }
        };
        self.store.put_pending(group_id, id, Pending { op, attempts: 0, invalid: None });
        Ok(())
    }

    /// Deletes an object and the children the device holds, in the same
    /// batch (the cascade). A pending create is simply withdrawn.
    pub fn delete(&mut self, group_id: Uuid, id: Uuid) -> Result<(), SyncError> {
        let Some(group) = self.store.group(group_id) else { return Err(SyncError::Unknown { id }) };
        if !group.rows.contains_key(&id) {
            return Err(SyncError::Unknown { id });
        }
        let mut todo = vec![id];
        while let Some(id) = todo.pop() {
            let group = self.store.group(group_id).expect("ensured above");
            todo.extend(group.children_of(id));
            let row = group.rows.get(&id).cloned().expect("a child listed is held");
            if row.deleted {
                continue;
            }
            if matches!(self.store.pending(group_id, id), Some(Pending { op: Op::Create { .. }, .. })) {
                self.store.remove_pending(group_id, id);
                self.store.remove_row(group_id, id);
                continue;
            }
            self.store.put_row(group_id, id, Row { deleted: true, value: None, ..row });
            self.store.put_pending(group_id, id, Pending { op: Op::Delete { base_seq: row.seq }, attempts: 0, invalid: None });
        }
        Ok(())
    }

    // ---- Applying rows ----

    fn apply_entry(&mut self, group_id: Uuid, entry: &Entry, events: &mut Vec<Event>) {
        let known = self.store.row(group_id, entry.id).cloned();
        if known.as_ref().is_some_and(|r| r.seq != 0 && r.seq >= entry.seq) {
            return;
        }
        if entry.deleted {
            let (object_type, parent_id) = known.map_or((None, None), |r| (r.object_type, r.parent_id));
            self.store.put_row(group_id, entry.id, Row { seq: entry.seq, deleted: true, value: None, object_type, parent_id });
            match self.store.pending(group_id, entry.id).map(|p| &p.op) {
                Some(Op::Delete { .. }) => self.store.remove_pending(group_id, entry.id),
                Some(_) => {
                    self.store.remove_pending(group_id, entry.id);
                    events.push(Event::EditDropped { id: entry.id });
                }
                None => {}
            }
            events.push(Event::Applied { id: entry.id, seq: entry.seq, deleted: true });
            self.cascade(group_id, entry.id);
            return;
        }
        let Some(value) = entry.value.as_ref() else { return };
        let indexed = self.index(group_id, entry.id, value);
        if indexed.is_none()
            && let Ok(header) = Envelope::peek(value)
            && self.keyring.get(group_id, header.key_id).is_none()
        {
            events.push(Event::NoKey { id: entry.id, key_id: header.key_id });
        }
        let (object_type, parent_id) = indexed.map_or((None, None), |(t, p)| (Some(t), p));
        let dead = parent_id.is_some_and(|p| self.store.row(group_id, p).is_some_and(|r| r.deleted));
        self.store.put_row(group_id, entry.id, Row { seq: entry.seq, deleted: dead, value: if dead { None } else { Some(value.clone()) }, object_type, parent_id });
        if dead {
            // A child of a tombstoned parent is dead whatever its row says:
            // the first device to notice pushes its tombstone.
            self.store.put_pending(group_id, entry.id, Pending { op: Op::Delete { base_seq: entry.seq }, attempts: 0, invalid: None });
        }
        events.push(Event::Applied { id: entry.id, seq: entry.seq, deleted: dead });
    }

    /// The children the device holds of a tombstoned parent are tombstoned
    /// too, with their own children.
    fn cascade(&mut self, group_id: Uuid, parent_id: Uuid) {
        let mut todo = vec![parent_id];
        while let Some(parent) = todo.pop() {
            let Some(group) = self.store.group(group_id) else { return };
            let children: Vec<(Uuid, Row)> = group.children_of(parent).into_iter().map(|c| (c, group.rows[&c].clone())).collect();
            for (child, row) in children {
                todo.push(child);
                if matches!(self.store.pending(group_id, child), Some(Pending { op: Op::Create { .. }, .. })) {
                    self.store.remove_pending(group_id, child);
                    self.store.remove_row(group_id, child);
                    continue;
                }
                self.store.put_row(group_id, child, Row { deleted: true, value: None, ..row.clone() });
                self.store.put_pending(group_id, child, Pending { op: Op::Delete { base_seq: row.seq }, attempts: 0, invalid: None });
            }
        }
    }

    fn drop_old_tombstones(&mut self, group_id: Uuid, min_acked_seq: Option<i64>) {
        let Some(min) = min_acked_seq else { return };
        let Some(group) = self.store.group(group_id) else { return };
        let gone: Vec<Uuid> = group.rows.iter().filter(|(id, r)| r.deleted && r.seq <= min && !group.outbox.contains_key(id)).map(|(id, _)| *id).collect();
        for id in gone {
            self.store.remove_row(group_id, id);
        }
    }

    // ---- The sync itself ----

    /// Acks `seq`. `Ok(false)` when the server refused the ack as a
    /// precondition: the device's row is gone (evicted while away) or the
    /// cursor went backwards (a restored backup); the caller starts over.
    async fn ack<R: Relay>(&mut self, relay: &R, group_id: Uuid, seq: i64) -> Result<bool, SyncError> {
        match relay.ack_seq(self.device_id, group_id, seq).await {
            Ok(_) => Ok(true),
            Err(e) if e.code == crate::api::Code::FailedPrecondition => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// One sync of one group: where the device stands decides the path,
    /// and a holder at the end of it pushes its outbox.
    pub async fn sync_group<R: Relay>(&mut self, relay: &R, group_id: Uuid) -> Result<Vec<Event>, SyncError> {
        let mut events = Vec::new();
        let phase = self.store.ensure(group_id).phase.clone();
        match phase {
            Phase::New => self.join(relay, group_id, &mut events).await?,
            Phase::CatchingUp { .. } => self.catch_up(relay, group_id, &mut events).await?,
            Phase::Holder => self.pull_as_holder(relay, group_id, &mut events).await?,
        }
        if self.store.group(group_id).is_some_and(|g| g.phase == Phase::Holder) {
            self.push_outbox(relay, group_id, &mut events).await?;
        }
        Ok(events)
    }

    /// The `history_ready` wake-up: a holder pushed history for the
    /// request. The wake-up names no group, so the group is the one whose
    /// phase holds the request; a wake-up for a request this device no
    /// longer waits on is nothing.
    pub fn history_ready(&mut self, request_id: Uuid, through_seq: i64, complete: bool) {
        for group_id in self.store.groups() {
            if let Some(Phase::CatchingUp { request_id: Some(mine), floor, through_seq: so_far, complete: done }) = self.store.group(group_id).map(|g| g.phase.clone())
                && mine == request_id
            {
                let (through_seq, complete) = progress(floor, so_far, done, through_seq, complete);
                self.store.set_phase(group_id, Phase::CatchingUp { request_id: Some(mine), floor, through_seq, complete });
                return;
            }
        }
    }

    /// The holder's pull: everything after the cursor, then the ack. A
    /// device whose cursor is below the floor starts over and takes the new
    /// device's path in this sync.
    async fn pull_as_holder<R: Relay>(&mut self, relay: &R, group_id: Uuid, events: &mut Vec<Event>) -> Result<(), SyncError> {
        let mut since = self.store.group(group_id).map_or(0, |g| g.acked_seq);
        let acked = since;
        loop {
            let page = relay.pull_changes(self.device_id, group_id, since, 0).await?;
            if acked < page.evicted_through_seq {
                // Rows between the cursor and the floor are gone, tombstones
                // among them: the device cannot know what was deleted.
                self.store.start_over(group_id);
                events.push(Event::StartedOver);
                return self.join(relay, group_id, events).await;
            }
            for entry in &page.entries {
                self.apply_entry(group_id, entry, events);
                since = entry.seq;
            }
            if !page.more {
                if page.head_seq > acked && !self.ack(relay, group_id, page.head_seq).await? {
                    self.store.start_over(group_id);
                    events.push(Event::StartedOver);
                    return self.join(relay, group_id, events).await;
                }
                self.store.set_cursor(group_id, page.head_seq, page.head_seq, page.evicted_through_seq);
                self.drop_old_tombstones(group_id, page.min_acked_seq);
                return Ok(());
            }
        }
    }

    /// The path of a new device: ack 0, apply the window, ask a holder for
    /// the rest or, with nothing older, become a holder at once.
    async fn join<R: Relay>(&mut self, relay: &R, group_id: Uuid, events: &mut Vec<Event>) -> Result<(), SyncError> {
        relay.ack_seq(self.device_id, group_id, 0).await?;
        let mut since = 0;
        let (head_seq, floor) = loop {
            let page = relay.pull_changes(self.device_id, group_id, since, 0).await?;
            for entry in &page.entries {
                self.apply_entry(group_id, entry, events);
                since = entry.seq;
            }
            if !page.more {
                break (page.head_seq, page.evicted_through_seq);
            }
        };
        self.store.set_cursor(group_id, 0, head_seq, floor);
        if floor >= 1 {
            // Everything older than the window is a holder's to give, or
            // is in the window already: the catch-up asks.
            self.store.set_phase(group_id, Phase::CatchingUp { request_id: None, floor, through_seq: 0, complete: false });
            return self.catch_up(relay, group_id, events).await;
        }
        self.become_holder(relay, group_id, head_seq, floor, events).await
    }

    /// Pages the restored rows from the cursor as far as the holder has
    /// pushed, acking as it goes; complete and past the floor, the device
    /// holds the group.
    async fn catch_up<R: Relay>(&mut self, relay: &R, group_id: Uuid, events: &mut Vec<Event>) -> Result<(), SyncError> {
        let Some(Phase::CatchingUp { mut request_id, floor, mut through_seq, mut complete }) = self.store.group(group_id).map(|g| g.phase.clone()) else { return Ok(()) };
        if let Some(id) = request_id
            && !complete
        {
            // The request as the server has it, so the device follows the
            // holder's progress whether or not the wake-up arrived. Swept
            // (not found) is asked again below; a re-push skips what is
            // there.
            match relay.get_history_request(self.device_id, id).await {
                Ok(r) => {
                    let (t, c) = progress(floor, through_seq, complete, r.through_seq, r.status == HistoryStatus::Complete);
                    if (t, c) != (through_seq, complete) {
                        self.store.set_phase(group_id, Phase::CatchingUp { request_id: Some(id), floor, through_seq: t, complete: c });
                    }
                    (through_seq, complete) = (t, c);
                }
                Err(ApiError { code: crate::api::Code::NotFound, .. }) => request_id = None,
                Err(e) => return Err(e.into()),
            }
        }
        if request_id.is_none() && !complete {
            // No request yet (the join), or the one there was is gone
            // (swept before this device saw it complete): ask.
            let request = relay.request_history(self.device_id, group_id, 1, floor, None).await?;
            match request.status {
                HistoryStatus::Unfillable => {
                    events.push(Event::Unfillable);
                    let head = self.store.group(group_id).map_or(0, |g| g.head_seq);
                    return self.become_holder(relay, group_id, head, floor, events).await;
                }
                HistoryStatus::Complete => {
                    // The window holds the range already: a restore for
                    // another device landed and no eviction has reached it.
                    // Page it from the cursor like a complete request's
                    // rows, with nothing to wait for.
                    (through_seq, complete) = (floor, true);
                    self.store.set_phase(group_id, Phase::CatchingUp { request_id: None, floor, through_seq, complete });
                    events.push(Event::HistoryPresent);
                }
                _ => {
                    self.store.set_phase(group_id, Phase::CatchingUp { request_id: Some(request.id), floor, through_seq: 0, complete: false });
                    events.push(Event::HistoryRequested { request_id: request.id });
                    return Ok(());
                }
            }
        }
        let mut acked = self.store.group(group_id).map_or(0, |g| g.acked_seq);
        let mut since = acked;
        loop {
            let page = relay.pull_changes(self.device_id, group_id, since, 0).await?;
            for entry in &page.entries {
                self.apply_entry(group_id, entry, events);
                since = entry.seq;
            }
            let target = since.min(through_seq);
            if target > acked {
                if !self.ack(relay, group_id, target).await? {
                    // The row went while the device caught up: the next
                    // sync joins again (as `become_holder` does; the join
                    // comes through here, so it cannot be called from here).
                    self.store.start_over(group_id);
                    events.push(Event::StartedOver);
                    return Ok(());
                }
                acked = target;
            }
            if !page.more {
                self.store.set_cursor(group_id, acked, page.head_seq, page.evicted_through_seq);
                if complete {
                    // Everything through the floor that exists is in, and
                    // the pull just gave everything after the cursor: the
                    // device holds the group through the head.
                    return self.become_holder(relay, group_id, page.head_seq, page.evicted_through_seq, events).await;
                }
                return Ok(());
            }
        }
    }

    async fn become_holder<R: Relay>(&mut self, relay: &R, group_id: Uuid, head_seq: i64, floor: i64, events: &mut Vec<Event>) -> Result<(), SyncError> {
        let acked = self.store.group(group_id).map_or(0, |g| g.acked_seq);
        if head_seq > acked && !self.ack(relay, group_id, head_seq).await? {
            // The row went while the device caught up: the next sync joins
            // again.
            self.store.start_over(group_id);
            events.push(Event::StartedOver);
            return Ok(());
        }
        self.store.set_cursor(group_id, head_seq, head_seq, floor);
        self.store.set_phase(group_id, Phase::Holder);
        events.push(Event::BecameHolder);
        self.replay_outbox(group_id, events)
    }

    /// After becoming a holder: a create goes as it is; an edit or a delete
    /// goes only if its object is still there, rebased onto the current
    /// row with that row's seq as its base, and is dropped when it is gone.
    fn replay_outbox(&mut self, group_id: Uuid, events: &mut Vec<Event>) -> Result<(), SyncError> {
        let Some(group) = self.store.group(group_id) else { return Ok(()) };
        let pending: Vec<(Uuid, Pending)> = group.outbox.iter().map(|(id, p)| (*id, p.clone())).collect();
        for (id, p) in pending {
            let row = self.store.row(group_id, id).cloned();
            match p.op {
                Op::Create { .. } => {}
                Op::Edit { object_type, base_plain, plain, .. } => match row {
                    Some(Row { deleted: false, value: Some(value), seq, .. }) if seq != 0 => {
                        let (_, current_plain) = self.open(group_id, id, &value)?;
                        self.rebase_pending(group_id, id, object_type, &base_plain, &plain, seq, &current_plain, 0, events)?;
                    }
                    _ => {
                        self.store.remove_pending(group_id, id);
                        events.push(Event::EditDropped { id });
                    }
                },
                Op::Delete { .. } => match row {
                    Some(Row { deleted: true, seq, .. }) if seq != 0 && self.store.row(group_id, id).is_some() => {
                        // The local mark stands; the server's seq is what the
                        // pull applied last.
                        self.store.put_pending(group_id, id, Pending { op: Op::Delete { base_seq: seq }, attempts: 0, invalid: None });
                    }
                    _ => self.store.remove_pending(group_id, id),
                },
            }
        }
        Ok(())
    }

    /// Pushes the outbox, rebasing conflicts a few times. A batch refused
    /// as behind the floor means the device is no holder: it starts over
    /// and the outbox waits for the next sync.
    async fn push_outbox<R: Relay>(&mut self, relay: &R, group_id: Uuid, events: &mut Vec<Event>) -> Result<(), SyncError> {
        for _ in 0..REBASE_ATTEMPTS {
            let Some(group) = self.store.group(group_id) else { return Ok(()) };
            let mut batch = Vec::new();
            let mut sealed_by_id: BTreeMap<Uuid, Option<Vec<u8>>> = BTreeMap::new();
            for (id, p) in &group.outbox {
                if p.invalid.is_some() {
                    continue;
                }
                let (value, deleted) = match &p.op {
                    Op::Create { sealed, .. } | Op::Edit { sealed, .. } => (Some(sealed.clone()), false),
                    Op::Delete { .. } => (None, true),
                };
                sealed_by_id.insert(*id, value.clone());
                batch.push(PushEntry { id: *id, value, base_seq: p.op.base_seq(), deleted });
            }
            if batch.is_empty() {
                return Ok(());
            }
            match relay.push_objects(self.device_id, group_id, batch).await? {
                PushResult::BehindFloor { .. } => {
                    self.store.start_over(group_id);
                    events.push(Event::StartedOver);
                    return self.join(relay, group_id, events).await;
                }
                PushResult::Accepted { accepted, conflicts, head_seq } => {
                    let mut seqs: Vec<i64> = accepted.iter().map(|a| a.seq).collect();
                    seqs.sort_unstable();
                    for a in accepted {
                        let row = self.store.row(group_id, a.id).cloned();
                        let value = sealed_by_id.remove(&a.id).flatten();
                        let deleted = value.is_none();
                        let (object_type, parent_id) = match (&value, row) {
                            (Some(v), _) => self.index(group_id, a.id, v).map_or((None, None), |(t, p)| (Some(t), p)),
                            (None, Some(r)) => (r.object_type, r.parent_id),
                            (None, None) => (None, None),
                        };
                        self.store.put_row(group_id, a.id, Row { seq: a.seq, deleted, value, object_type, parent_id });
                        self.store.remove_pending(group_id, a.id);
                        events.push(Event::Accepted { id: a.id, seq: a.seq });
                    }
                    let group = self.store.group(group_id).expect("held");
                    let (mut acked, floor) = (group.acked_seq, group.floor);
                    // The device's own rows are the whole stretch after its
                    // cursor: it holds everything through the head, and says
                    // so without another pull.
                    let whole_stretch = seqs.first() == Some(&(acked + 1)) && seqs.last() == Some(&head_seq) && seqs.windows(2).all(|w| w[1] == w[0] + 1);
                    if whole_stretch && self.ack(relay, group_id, head_seq).await? {
                        acked = head_seq;
                    }
                    self.store.set_cursor(group_id, acked, head_seq, floor);
                    if conflicts.is_empty() {
                        return Ok(());
                    }
                    for c in conflicts {
                        self.resolve_conflict(group_id, c, events)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn resolve_conflict(&mut self, group_id: Uuid, c: Conflict, events: &mut Vec<Event>) -> Result<(), SyncError> {
        let Some(pending) = self.store.pending(group_id, c.id).cloned() else { return Ok(()) };
        let current = c.current;
        if current.deleted {
            let known = self.store.row(group_id, c.id).cloned();
            let (object_type, parent_id) = known.map_or((None, None), |r| (r.object_type, r.parent_id));
            self.store.put_row(group_id, c.id, Row { seq: current.seq, deleted: true, value: None, object_type, parent_id });
            self.store.remove_pending(group_id, c.id);
            if !matches!(pending.op, Op::Delete { .. }) {
                events.push(Event::EditDropped { id: c.id });
            }
            self.cascade(group_id, c.id);
            return Ok(());
        }
        let value = current.value.clone().unwrap_or_default();
        let (header, current_plain) = self.open(group_id, c.id, &value)?;
        let (object_type, parent_id) = ObjectType::from_number(header.object_type).map_or((None, None), |t| (Some(t), Meta::decode(&current_plain).ok().and_then(|m| m.parent_id)));
        match pending.op {
            Op::Delete { .. } => {
                // The local mark stands; the delete goes again on the row
                // as it is now.
                self.store.put_row(group_id, c.id, Row { seq: current.seq, deleted: true, value: None, object_type, parent_id });
                self.store.put_pending(group_id, c.id, Pending { op: Op::Delete { base_seq: current.seq }, attempts: pending.attempts + 1, invalid: None });
            }
            Op::Create { object_type: t, plain, .. } => {
                self.store.put_row(group_id, c.id, Row { seq: current.seq, deleted: false, value: Some(value), object_type, parent_id });
                self.rebase_pending(group_id, c.id, t, &[], &plain, current.seq, &current_plain, pending.attempts + 1, events)?;
            }
            Op::Edit { object_type: t, base_plain, plain, .. } => {
                self.store.put_row(group_id, c.id, Row { seq: current.seq, deleted: false, value: Some(value), object_type, parent_id });
                self.rebase_pending(group_id, c.id, t, &base_plain, &plain, current.seq, &current_plain, pending.attempts + 1, events)?;
            }
        }
        Ok(())
    }

    /// The rebase of one pending write onto the current row: the result
    /// becomes the pending edit, or parks it when invalid.
    #[allow(clippy::too_many_arguments)] // the rebase's inputs, each named
    fn rebase_pending(&mut self, group_id: Uuid, id: Uuid, object_type: ObjectType, base_plain: &[u8], plain: &[u8], current_seq: i64, current_plain: &[u8], attempts: u32, events: &mut Vec<Event>) -> Result<(), SyncError> {
        let decode = |bytes: &[u8]| wire::decode(bytes).unwrap_or_default();
        let merged = wire::encode(&rebase::rebase(object_type, &decode(base_plain), &decode(plain), &decode(current_plain)));
        let invalid = object_type.validate_value(&merged).err().map(|e| e.to_string());
        if let Some(reason) = &invalid {
            events.push(Event::Invalid { id, reason: reason.clone() });
        }
        let sealed = self.seal(group_id, id, object_type, &merged)?;
        self.store.put_pending(group_id, id, Pending { op: Op::Edit { object_type, base_seq: current_seq, base_plain: current_plain.to_vec(), plain: merged, sealed }, attempts, invalid });
        Ok(())
    }

    // ---- Serving history ----

    /// A holder's turn: claims the group's open requests of other devices
    /// and pushes the rows it holds in their ranges, oldest first.
    pub async fn serve_history<R: Relay>(&mut self, relay: &R, group_id: Uuid) -> Result<Vec<Event>, SyncError> {
        let mut events = Vec::new();
        if self.store.group(group_id).is_none_or(|g| g.phase != Phase::Holder) {
            return Ok(events);
        }
        for request in relay.list_history_requests(self.device_id, group_id).await? {
            if request.requester_device == self.device_id {
                continue;
            }
            let claimed = match relay.claim_history(self.device_id, request.id).await {
                Ok(c) => c,
                // Gone meanwhile, or the server does not count this device as
                // a holder (its next pull finds out): not this device's.
                Err(e) if matches!(e.code, crate::api::Code::NotFound | crate::api::Code::PermissionDenied | crate::api::Code::InvalidArgument) => continue,
                Err(e) => return Err(e.into()),
            };
            if claimed.status != HistoryStatus::Claimed {
                continue;
            }
            let group = self.store.group(group_id).expect("a holder has the group");
            let mut rows: Vec<Entry> = group
                .rows
                .iter()
                .filter(|(id, r)| r.seq >= request.from_seq && r.seq <= request.to_seq && request.object_id.is_none_or(|o| o == **id))
                .map(|(id, r)| Entry { id: *id, value: r.value.clone(), seq: r.seq, deleted: r.deleted })
                .collect();
            rows.sort_by_key(|e| e.seq);
            let total = rows.len();
            let mut pages: Vec<Vec<Entry>> = rows.chunks(HISTORY_PAGE).map(<[Entry]>::to_vec).collect();
            if pages.is_empty() {
                pages.push(Vec::new());
            }
            let last = pages.len() - 1;
            let mut served = true;
            for (i, page) in pages.into_iter().enumerate() {
                match relay.push_history(self.device_id, request.id, page, i == last).await {
                    Ok(_) => {}
                    // Completed by another holder, expired, or the claim was
                    // taken over: the rest is not this device's to push.
                    Err(e) if matches!(e.code, crate::api::Code::NotFound | crate::api::Code::PermissionDenied) => {
                        served = false;
                        break;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            if served {
                events.push(Event::HistoryServed { request_id: request.id, rows: total });
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests;
