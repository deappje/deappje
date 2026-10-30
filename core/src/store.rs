//! The local store: per group, the rows the device holds (envelopes, as
//! the relay has them, with the type and the parent read out of each one
//! the device could open), its cursor through the group's log, the phase
//! of its membership in the log (new, catching up on history, a holder),
//! and the outbox of writes not yet accepted. Written through to the
//! shell's [`Storage`] on every change and read back whole at open.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::message;
use crate::schema::{Message as _, ObjectType};
use crate::shell::Storage;
use crate::wire_enum;

/// A row as the device holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The seq the row has on the server; 0 for a create not yet accepted.
    pub seq: i64,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The envelope bytes; none for a tombstone.
    pub value: Option<Vec<u8>>,
    /// The type, when the device could open the envelope.
    pub object_type: Option<ObjectType>,
    /// The parent, when the device could open the envelope and there is
    /// one: the cascade's index.
    pub parent_id: Option<Uuid>,
}

/// Where the device stands in a group's log (`02-relay.md`, "The path of
/// a new device").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Joined, or starting over: acks 0 and pulls the window next.
    New,
    /// Asked a holder for everything older than the window; pages and acks
    /// the restored rows as `HistoryReady` says how far.
    CatchingUp {
        /// The request, once the server stored one.
        request_id: Option<Uuid>,
        /// The floor asked up to.
        floor: i64,
        /// How far the holder has pushed so far.
        through_seq: i64,
        /// Whether the holder is done.
        complete: bool,
    },
    /// Holds the group from seq 1 through its ack.
    Holder,
}

/// A write not yet accepted by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// What to push.
    pub op: Op,
    /// Rebases tried in the current sync; parked after a few.
    pub attempts: u32,
    /// Why the rebase result cannot be pushed, when it cannot: shown to
    /// the user, who edits again.
    pub invalid: Option<String>,
}

/// The write itself. An edit keeps the plaintext of the row it was edited
/// from: what the rebase diffs against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// A new object.
    Create {
        /// Its type.
        object_type: ObjectType,
        /// Its plaintext.
        plain: Vec<u8>,
        /// Its envelope, sealed when the op was written: what is pushed,
        /// and what the row holds once accepted.
        sealed: Vec<u8>,
    },
    /// A change to an object the device holds.
    Edit {
        /// Its type.
        object_type: ObjectType,
        /// The seq edited from.
        base_seq: i64,
        /// The plaintext edited from.
        base_plain: Vec<u8>,
        /// The plaintext now.
        plain: Vec<u8>,
        /// The envelope of `plain`.
        sealed: Vec<u8>,
    },
    /// A delete.
    Delete {
        /// The seq of the row deleted.
        base_seq: i64,
    },
}

impl Op {
    /// The seq the push names as its base.
    pub fn base_seq(&self) -> i64 {
        match self {
            Op::Create { .. } => 0,
            Op::Edit { base_seq, .. } | Op::Delete { base_seq } => *base_seq,
        }
    }
}

/// A group as the device holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The cursor: the device holds every row through it.
    pub acked_seq: i64,
    /// The head last seen.
    pub head_seq: i64,
    /// The floor last seen.
    pub floor: i64,
    /// Where the device stands.
    pub phase: Phase,
    /// The rows, by object.
    pub rows: BTreeMap<Uuid, Row>,
    /// The writes not yet accepted, by object.
    pub outbox: BTreeMap<Uuid, Pending>,
}

impl Group {
    fn new() -> Group {
        Group { acked_seq: 0, head_seq: 0, floor: 0, phase: Phase::New, rows: BTreeMap::new(), outbox: BTreeMap::new() }
    }

    /// The live rows the device holds whose parent is `parent_id`.
    pub fn children_of(&self, parent_id: Uuid) -> Vec<Uuid> {
        self.rows.iter().filter(|(_, r)| !r.deleted && r.parent_id == Some(parent_id)).map(|(id, _)| *id).collect()
    }

    /// The live rows: id, seq and envelope bytes. What two devices that
    /// hold the same group agree on.
    pub fn live(&self) -> BTreeMap<Uuid, (i64, Vec<u8>)> {
        self.rows.iter().filter_map(|(id, r)| r.value.as_ref().filter(|_| !r.deleted).map(|v| (*id, (r.seq, v.clone())))).collect()
    }
}

// ---- The records on disk ----

wire_enum! {
    pub enum PhaseKind {
        #[default]
        New = 0,
        CatchingUp = 1,
        Holder = 2,
    }
}

message! {
    StateRecord {
        1 => acked_seq: i64,
        2 => head_seq: i64,
        3 => floor: i64,
        4 => phase: PhaseKind,
        5 => request_id: Option<Uuid>,
        6 => catching_floor: i64,
        7 => through_seq: i64,
        8 => complete: bool,
    }
}

message! {
    RowRecord {
        1 => seq: i64,
        2 => deleted: bool,
        3 => value: Option<Vec<u8>>,
        4 => object_type: Option<u32>,
        5 => parent_id: Option<Uuid>,
    }
}

wire_enum! {
    pub enum OpKind {
        #[default]
        Create = 0,
        Edit = 1,
        Delete = 2,
    }
}

message! {
    PendingRecord {
        1 => kind: OpKind,
        2 => object_type: u32,
        3 => base_seq: i64,
        4 => base_plain: Vec<u8>,
        5 => plain: Vec<u8>,
        6 => attempts: u32,
        7 => invalid: Option<String>,
        8 => sealed: Vec<u8>,
    }
}

fn state_record(g: &Group) -> StateRecord {
    let (phase, request_id, catching_floor, through_seq, complete) = match &g.phase {
        Phase::New => (PhaseKind::New, None, 0, 0, false),
        Phase::CatchingUp { request_id, floor, through_seq, complete } => (PhaseKind::CatchingUp, *request_id, *floor, *through_seq, *complete),
        Phase::Holder => (PhaseKind::Holder, None, 0, 0, false),
    };
    StateRecord { acked_seq: g.acked_seq, head_seq: g.head_seq, floor: g.floor, phase, request_id, catching_floor, through_seq, complete }
}

fn row_record(r: &Row) -> RowRecord {
    RowRecord { seq: r.seq, deleted: r.deleted, value: r.value.clone(), object_type: r.object_type.map(ObjectType::number), parent_id: r.parent_id }
}

fn pending_record(p: &Pending) -> PendingRecord {
    let (kind, object_type, base_seq, base_plain, plain, sealed) = match &p.op {
        Op::Create { object_type, plain, sealed } => (OpKind::Create, object_type.number(), 0, Vec::new(), plain.clone(), sealed.clone()),
        Op::Edit { object_type, base_seq, base_plain, plain, sealed } => (OpKind::Edit, object_type.number(), *base_seq, base_plain.clone(), plain.clone(), sealed.clone()),
        Op::Delete { base_seq } => (OpKind::Delete, 0, *base_seq, Vec::new(), Vec::new(), Vec::new()),
    };
    PendingRecord { kind, object_type, base_seq, base_plain, plain, attempts: p.attempts, invalid: p.invalid.clone(), sealed }
}

fn pending_of(r: PendingRecord) -> Option<Pending> {
    let op = match r.kind {
        OpKind::Create => Op::Create { object_type: ObjectType::from_number(r.object_type)?, plain: r.plain, sealed: r.sealed },
        OpKind::Edit => Op::Edit { object_type: ObjectType::from_number(r.object_type)?, base_seq: r.base_seq, base_plain: r.base_plain, plain: r.plain, sealed: r.sealed },
        OpKind::Delete => Op::Delete { base_seq: r.base_seq },
    };
    Some(Pending { op, attempts: r.attempts, invalid: r.invalid })
}

fn state_key(group_id: Uuid) -> String {
    format!("g/{group_id}/s")
}

fn row_key(group_id: Uuid, id: Uuid) -> String {
    format!("g/{group_id}/o/{id}")
}

fn pending_key(group_id: Uuid, id: Uuid) -> String {
    format!("g/{group_id}/p/{id}")
}

/// The store: every group the device holds, written through.
pub struct Store {
    storage: Box<dyn Storage>,
    groups: BTreeMap<Uuid, Group>,
}

impl Store {
    /// Opens the store over the shell's storage, reading every group back.
    /// A blob that does not parse is dropped: the store is a cache of the
    /// relay and the outbox, and a corrupt row is re-fetched.
    pub fn open(storage: Box<dyn Storage>) -> Store {
        let mut groups: BTreeMap<Uuid, Group> = BTreeMap::new();
        for key in storage.keys("g/") {
            let parts: Vec<&str> = key.split('/').collect();
            let Some(group_id) = parts.get(1).and_then(|s| s.parse::<Uuid>().ok()) else { continue };
            let Some(bytes) = storage.get(&key) else { continue };
            let group = groups.entry(group_id).or_insert_with(Group::new);
            match (parts.get(2).copied(), parts.get(3).and_then(|s| s.parse::<Uuid>().ok())) {
                (Some("s"), None) => {
                    if let Ok(r) = StateRecord::decode(&bytes) {
                        group.acked_seq = r.acked_seq;
                        group.head_seq = r.head_seq;
                        group.floor = r.floor;
                        group.phase = match r.phase {
                            PhaseKind::New => Phase::New,
                            PhaseKind::CatchingUp => Phase::CatchingUp { request_id: r.request_id, floor: r.catching_floor, through_seq: r.through_seq, complete: r.complete },
                            PhaseKind::Holder => Phase::Holder,
                        };
                    }
                }
                (Some("o"), Some(id)) => {
                    if let Ok(r) = RowRecord::decode(&bytes) {
                        group.rows.insert(id, Row { seq: r.seq, deleted: r.deleted, value: r.value, object_type: r.object_type.and_then(ObjectType::from_number), parent_id: r.parent_id });
                    }
                }
                (Some("p"), Some(id)) => {
                    if let Some(p) = PendingRecord::decode(&bytes).ok().and_then(pending_of) {
                        group.outbox.insert(id, p);
                    }
                }
                _ => {}
            }
        }
        Store { storage, groups }
    }

    /// The groups held, ascending.
    pub fn groups(&self) -> Vec<Uuid> {
        self.groups.keys().copied().collect()
    }

    /// A group, if held.
    pub fn group(&self, group_id: Uuid) -> Option<&Group> {
        self.groups.get(&group_id)
    }

    /// The group, created empty if not held yet.
    pub fn ensure(&mut self, group_id: Uuid) -> &Group {
        let storage = &self.storage;
        self.groups.entry(group_id).or_insert_with(|| {
            storage.put(&state_key(group_id), &state_record(&Group::new()).encode());
            Group::new()
        })
    }

    fn group_mut(&mut self, group_id: Uuid) -> &mut Group {
        self.groups.entry(group_id).or_insert_with(Group::new)
    }

    fn save_state(&mut self, group_id: Uuid) {
        let record = state_record(self.group_mut(group_id)).encode();
        self.storage.put(&state_key(group_id), &record);
    }

    /// Sets the cursor and what the server last said about the group.
    pub fn set_cursor(&mut self, group_id: Uuid, acked_seq: i64, head_seq: i64, floor: i64) {
        let g = self.group_mut(group_id);
        g.acked_seq = acked_seq;
        g.head_seq = head_seq;
        g.floor = floor;
        self.save_state(group_id);
    }

    /// Sets the phase.
    pub fn set_phase(&mut self, group_id: Uuid, phase: Phase) {
        self.group_mut(group_id).phase = phase;
        self.save_state(group_id);
    }

    /// A row of the group.
    pub fn row(&self, group_id: Uuid, id: Uuid) -> Option<&Row> {
        self.groups.get(&group_id)?.rows.get(&id)
    }

    /// Writes a row.
    pub fn put_row(&mut self, group_id: Uuid, id: Uuid, row: Row) {
        let record = row_record(&row).encode();
        self.group_mut(group_id).rows.insert(id, row);
        self.storage.put(&row_key(group_id, id), &record);
    }

    /// Removes a row (a tombstone every device has applied).
    pub fn remove_row(&mut self, group_id: Uuid, id: Uuid) {
        self.group_mut(group_id).rows.remove(&id);
        self.storage.delete(&row_key(group_id, id));
    }

    /// A pending write of the group.
    pub fn pending(&self, group_id: Uuid, id: Uuid) -> Option<&Pending> {
        self.groups.get(&group_id)?.outbox.get(&id)
    }

    /// Writes a pending write.
    pub fn put_pending(&mut self, group_id: Uuid, id: Uuid, pending: Pending) {
        let record = pending_record(&pending).encode();
        self.group_mut(group_id).outbox.insert(id, pending);
        self.storage.put(&pending_key(group_id, id), &record);
    }

    /// Removes a pending write (accepted, or dropped).
    pub fn remove_pending(&mut self, group_id: Uuid, id: Uuid) {
        self.group_mut(group_id).outbox.remove(&id);
        self.storage.delete(&pending_key(group_id, id));
    }

    /// Throws the group's rows away and starts again as a new device; the
    /// outbox stays, to be replayed once the device holds the group.
    pub fn start_over(&mut self, group_id: Uuid) {
        let ids: Vec<Uuid> = self.group_mut(group_id).rows.keys().copied().collect();
        for id in ids {
            self.remove_row(group_id, id);
        }
        let g = self.group_mut(group_id);
        g.acked_seq = 0;
        g.phase = Phase::New;
        self.save_state(group_id);
    }

    /// Forgets the group entirely (left, or removed).
    pub fn forget_group(&mut self, group_id: Uuid) {
        if let Some(g) = self.groups.remove(&group_id) {
            for id in g.rows.keys() {
                self.storage.delete(&row_key(group_id, *id));
            }
            for id in g.outbox.keys() {
                self.storage.delete(&pending_key(group_id, *id));
            }
            self.storage.delete(&state_key(group_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::MemoryStorage;
    use std::rc::Rc;

    /// Storage shared between two opens of the store.
    struct Shared(Rc<MemoryStorage>);

    impl Storage for Shared {
        fn get(&self, key: &str) -> Option<Vec<u8>> {
            self.0.get(key)
        }
        fn put(&self, key: &str, value: &[u8]) {
            self.0.put(key, value)
        }
        fn delete(&self, key: &str) {
            self.0.delete(key)
        }
        fn keys(&self, prefix: &str) -> Vec<String> {
            self.0.keys(prefix)
        }
    }

    #[test]
    fn everything_written_is_read_back_at_open() {
        let storage = Rc::new(MemoryStorage::new());
        let (g, a, b, req) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let mut store = Store::open(Box::new(Shared(storage.clone())));
        store.ensure(g);
        store.set_cursor(g, 5, 9, 2);
        store.set_phase(g, Phase::CatchingUp { request_id: Some(req), floor: 2, through_seq: 1, complete: false });
        store.put_row(g, a, Row { seq: 3, deleted: false, value: Some(vec![1, 2]), object_type: Some(ObjectType::Post), parent_id: None });
        store.put_row(g, b, Row { seq: 4, deleted: true, value: None, object_type: None, parent_id: Some(a) });
        store.put_pending(g, a, Pending { op: Op::Edit { object_type: ObjectType::Post, base_seq: 3, base_plain: vec![9], plain: vec![8], sealed: vec![7] }, attempts: 2, invalid: Some("the event ends before it starts".into()) });
        store.put_pending(g, b, Pending { op: Op::Delete { base_seq: 4 }, attempts: 0, invalid: None });
        let before = store.group(g).unwrap().clone();

        let again = Store::open(Box::new(Shared(storage.clone())));
        assert_eq!(again.group(g), Some(&before));
        assert_eq!(again.groups(), vec![g]);
        assert_eq!(before.children_of(a), Vec::<Uuid>::new(), "a tombstone is not a child");

        let mut store = again;
        store.remove_pending(g, b);
        store.start_over(g);
        let after = Store::open(Box::new(Shared(storage.clone()))).group(g).cloned().unwrap();
        assert!(after.rows.is_empty());
        assert_eq!(after.acked_seq, 0);
        assert_eq!(after.phase, Phase::New);
        assert_eq!(after.outbox.len(), 1, "the outbox survives a start-over");
        store.forget_group(g);
        assert!(storage.is_empty(), "nothing of a forgotten group is left");
    }
}
