//! `relay` in memory: the handlers of `nanoservices/relay/src/handlers.rs`
//! and the sweeps of its `worker.rs`, rule for rule, over maps instead of
//! tables, with a clock the test moves. Membership is a table the test
//! fills (the real relay asks `group`); the wake-ups `device` would push
//! are queued per device for the test to deliver.

use std::sync::Mutex;
use std::collections::{BTreeMap, VecDeque};

use uuid::Uuid;

use crate::api::{ApiError, Code};
use crate::relay::{AckResult, ChangeSet, Conflict, Entry, Head, HistoryRequest, HistoryStatus, PushEntry, PushHistoryResult, PushResult, Relay};
use crate::rng::CoreRng;

/// A value is at most this long.
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
/// A pull's page when the request says 0, and its ceiling.
pub const DEFAULT_PAGE: i32 = 500;
/// The most a pull returns.
pub const MAX_PAGE: i32 = 2000;
/// A claim this old without `complete` is abandoned.
pub const CLAIM_TAKEOVER_MS: i64 = 5 * 60 * 1000;
/// The window: how long a row stays at most, and how long a device may
/// stay away without starting over.
pub const WINDOW_MS: i64 = 672 * 60 * 60 * 1000;

/// A member's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Writes and administers.
    Admin,
    /// Writes.
    Member,
    /// Reads only.
    Viewer,
}

/// What `device` would push to a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wakeup {
    /// Another device wrote to the group.
    HeadAdvanced {
        /// The group.
        group_id: Uuid,
        /// Its head now.
        head_seq: i64,
    },
    /// A device asked for history this device, a holder, may serve.
    HistoryRequested {
        /// The request.
        request_id: Uuid,
        /// The group.
        group_id: Uuid,
    },
    /// A holder pushed history for this device's request.
    HistoryReady {
        /// The request.
        request_id: Uuid,
        /// The group.
        group_id: Uuid,
        /// The last seq pushed so far.
        through_seq: i64,
        /// Whether the request is complete.
        complete: bool,
    },
}

#[derive(Debug, Clone)]
struct Object {
    group_id: Uuid,
    value: Option<Vec<u8>>,
    seq: i64,
    deleted: bool,
    uploaded_at: i64,
}

#[derive(Debug, Clone, Default)]
struct HeadRow {
    head_seq: i64,
    evicted_through_seq: i64,
}

#[derive(Debug, Clone)]
struct DeviceRow {
    acked_seq: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct RequestRow {
    group_id: Uuid,
    requester_device: Uuid,
    from_seq: i64,
    to_seq: i64,
    object_id: Option<Uuid>,
    claimed_by: Option<Uuid>,
    claimed_at: Option<i64>,
    created_at: i64,
}

#[derive(Default)]
struct Inner {
    now_ms: i64,
    objects: BTreeMap<Uuid, Object>,
    heads: BTreeMap<Uuid, HeadRow>,
    devices: BTreeMap<(Uuid, Uuid), DeviceRow>,
    requests: BTreeMap<Uuid, RequestRow>,
    members: BTreeMap<(Uuid, Uuid), Role>,
    device_user: BTreeMap<Uuid, Uuid>,
    wakeups: BTreeMap<Uuid, VecDeque<Wakeup>>,
    outage: bool,
    calls: u64,
}

/// The model.
pub struct RelayModel {
    inner: Mutex<Inner>,
    rng: Mutex<CoreRng>,
}

fn err(code: Code, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

impl Inner {
    fn user_of(&self, device_id: Uuid) -> Result<Uuid, ApiError> {
        self.device_user.get(&device_id).copied().ok_or_else(|| err(Code::PermissionDenied, "unknown device"))
    }

    fn role(&self, device_id: Uuid, group_id: Uuid) -> Result<Option<Role>, ApiError> {
        let user = self.user_of(device_id)?;
        Ok(self.members.get(&(group_id, user)).copied())
    }

    fn require_member(&self, device_id: Uuid, group_id: Uuid) -> Result<Role, ApiError> {
        self.role(device_id, group_id)?.ok_or_else(|| err(Code::PermissionDenied, "not a member of this group"))
    }

    fn head(&self, group_id: Uuid) -> HeadRow {
        self.heads.get(&group_id).cloned().unwrap_or_default()
    }

    fn ack_of(&self, group_id: Uuid, device_id: Uuid) -> Option<i64> {
        self.devices.get(&(group_id, device_id)).map(|d| d.acked_seq)
    }

    fn is_holder(&self, group_id: Uuid, device_id: Uuid) -> bool {
        let floor = self.head(group_id).evicted_through_seq;
        self.ack_of(group_id, device_id).is_some_and(|a| a >= floor)
    }

    fn min_acked(&self, group_id: Uuid) -> Option<i64> {
        self.devices.iter().filter(|((g, _), _)| *g == group_id).map(|(_, d)| d.acked_seq).min()
    }

    fn holders_except(&self, group_id: Uuid, floor: i64, device_id: Uuid) -> Vec<Uuid> {
        self.devices.iter().filter(|((g, d), row)| *g == group_id && *d != device_id && row.acked_seq >= floor).map(|((_, d), _)| *d).collect()
    }

    fn wake(&mut self, device_id: Uuid, wakeup: Wakeup) {
        self.wakeups.entry(device_id).or_default().push_back(wakeup);
    }

    /// The devices of the group's members, as `device` would find them.
    fn group_devices(&self, group_id: Uuid) -> Vec<Uuid> {
        self.device_user.iter().filter(|(_, u)| self.members.contains_key(&(group_id, **u))).map(|(d, _)| *d).collect()
    }

    fn raise_floor(&mut self, group_id: Uuid, seq: i64) -> i64 {
        let head = self.heads.entry(group_id).or_default();
        head.evicted_through_seq = head.evicted_through_seq.max(seq);
        head.evicted_through_seq
    }

    fn request_msg(&self, id: Uuid, r: &RequestRow, status: HistoryStatus) -> HistoryRequest {
        HistoryRequest { id, group_id: r.group_id, requester_device: r.requester_device, from_seq: r.from_seq, to_seq: r.to_seq, object_id: r.object_id, claimed_by: r.claimed_by, status }
    }
}

impl RelayModel {
    /// A model at time 0 with no groups.
    pub fn new(seed: [u8; 32]) -> RelayModel {
        RelayModel { inner: Mutex::new(Inner::default()), rng: Mutex::new(CoreRng::from_seed(seed)) }
    }

    /// Registers a device as a user's.
    pub fn register_device(&self, device_id: Uuid, user_id: Uuid) {
        self.inner.lock().expect("no poison").device_user.insert(device_id, user_id);
    }

    /// Makes a user a member of a group with a role.
    pub fn add_member(&self, group_id: Uuid, user_id: Uuid, role: Role) {
        self.inner.lock().expect("no poison").members.insert((group_id, user_id), role);
    }

    /// Removes a user from a group; its devices' rows go with it, as
    /// `DeviceLeftGroup` does.
    pub fn remove_member(&self, group_id: Uuid, user_id: Uuid) {
        let mut inner = self.inner.lock().expect("no poison");
        inner.members.remove(&(group_id, user_id));
        let devices: Vec<Uuid> = inner.device_user.iter().filter(|(_, u)| **u == user_id).map(|(d, _)| *d).collect();
        for d in devices {
            inner.devices.remove(&(group_id, d));
        }
    }

    /// The clock.
    pub fn now_ms(&self) -> i64 {
        self.inner.lock().expect("no poison").now_ms
    }

    /// Moves the clock forward.
    pub fn advance(&self, ms: i64) {
        self.inner.lock().expect("no poison").now_ms += ms;
    }

    /// Whether calls fail with `Unavailable` (the network is down).
    pub fn set_outage(&self, outage: bool) {
        self.inner.lock().expect("no poison").outage = outage;
    }

    /// How many calls the model has answered or refused.
    pub fn calls(&self) -> u64 {
        self.inner.lock().expect("no poison").calls
    }

    /// The wake-ups queued for a device, oldest first, taken.
    pub fn take_wakeups(&self, device_id: Uuid) -> Vec<Wakeup> {
        self.inner.lock().expect("no poison").wakeups.remove(&device_id).map(Vec::from).unwrap_or_default()
    }

    /// The group's rows as the server holds them, seq ascending.
    pub fn log(&self, group_id: Uuid) -> Vec<Entry> {
        let inner = self.inner.lock().expect("no poison");
        let mut rows: Vec<Entry> = inner.objects.iter().filter(|(_, o)| o.group_id == group_id).map(|(id, o)| Entry { id: *id, value: o.value.clone(), seq: o.seq, deleted: o.deleted }).collect();
        rows.sort_by_key(|e| e.seq);
        rows
    }

    /// Where the group stands.
    pub fn head(&self, group_id: Uuid) -> (i64, i64) {
        let h = self.inner.lock().expect("no poison").head(group_id);
        (h.head_seq, h.evicted_through_seq)
    }

    /// A device's ack in a group, if it has a row.
    pub fn ack(&self, group_id: Uuid, device_id: Uuid) -> Option<i64> {
        self.inner.lock().expect("no poison").ack_of(group_id, device_id)
    }

    /// The open history requests of a group.
    pub fn open_requests(&self, group_id: Uuid) -> usize {
        self.inner.lock().expect("no poison").requests.values().filter(|r| r.group_id == group_id).count()
    }

    /// `evict_objects`: by ack, then by age.
    pub fn evict(&self) {
        let mut inner = self.inner.lock().expect("no poison");
        let groups: Vec<Uuid> = inner.heads.keys().copied().collect();
        for group_id in groups {
            if let Some(min) = inner.min_acked(group_id) {
                inner.objects.retain(|_, o| !(o.group_id == group_id && o.seq <= min));
                inner.raise_floor(group_id, min);
            }
        }
        let cutoff = inner.now_ms - WINDOW_MS;
        let mut highest: BTreeMap<Uuid, i64> = BTreeMap::new();
        inner.objects.retain(|_, o| {
            if o.uploaded_at < cutoff {
                let h = highest.entry(o.group_id).or_insert(o.seq);
                *h = (*h).max(o.seq);
                false
            } else {
                true
            }
        });
        for (group_id, seq) in highest {
            let floor = inner.raise_floor(group_id, seq);
            inner.devices.retain(|(g, _), d| !(*g == group_id && d.acked_seq < floor && d.updated_at < cutoff));
        }
    }

    /// `expire_history_requests`.
    pub fn expire_requests(&self) {
        let mut inner = self.inner.lock().expect("no poison");
        let cutoff = inner.now_ms - WINDOW_MS;
        inner.requests.retain(|_, r| r.created_at >= cutoff);
    }

    fn call(&self) -> Result<std::sync::MutexGuard<'_, Inner>, ApiError> {
        let mut inner = self.inner.lock().expect("no poison");
        inner.calls += 1;
        if inner.outage {
            return Err(err(Code::Unavailable, "the network is down"));
        }
        Ok(inner)
    }
}

impl Relay for RelayModel {
    async fn push_objects(&self, device_id: Uuid, group_id: Uuid, objects: Vec<PushEntry>) -> Result<PushResult, ApiError> {
        let mut inner = self.call()?;
        if objects.is_empty() {
            return Err(err(Code::InvalidArgument, "objects: empty batch"));
        }
        for o in &objects {
            if o.base_seq < 0 {
                return Err(err(Code::InvalidArgument, format!("object {}: base_seq negative", o.id)));
            }
            match (&o.value, o.deleted) {
                (Some(v), false) if v.is_empty() => return Err(err(Code::InvalidArgument, format!("object {}: empty value", o.id))),
                (Some(v), _) if v.len() > MAX_VALUE_BYTES => return Err(err(Code::InvalidArgument, format!("object {}: value over 1 MiB", o.id))),
                (None, false) => return Err(err(Code::InvalidArgument, format!("object {}: a live object has a value", o.id))),
                _ => {}
            }
        }
        let mut ids: Vec<Uuid> = objects.iter().map(|o| o.id).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != objects.len() {
            return Err(err(Code::InvalidArgument, "objects: an id at most once per batch"));
        }
        if inner.require_member(device_id, group_id)? == Role::Viewer {
            return Err(err(Code::PermissionDenied, "a viewer does not write to the log"));
        }
        let now = inner.now_ms;
        let head = inner.head(group_id);
        if !inner.is_holder(group_id, device_id) {
            return Ok(PushResult::BehindFloor { evicted_through_seq: head.evicted_through_seq });
        }
        let mut head_seq = head.head_seq;
        let mut accepted = Vec::new();
        let mut conflicts = Vec::new();
        for o in &objects {
            let current = inner.objects.get(&o.id).cloned();
            if let Some(c) = &current
                && c.group_id != group_id
            {
                return Err(err(Code::FailedPrecondition, format!("object {}: the id belongs to another group", o.id)));
            }
            match current.as_ref().map(|c| c.seq) {
                Some(seq) if seq != o.base_seq => {
                    let c = current.expect("a conflict has a current row");
                    conflicts.push(Conflict { id: o.id, current: Entry { id: o.id, value: c.value, seq: c.seq, deleted: c.deleted } });
                }
                _ => {
                    head_seq += 1;
                    let value = if o.deleted { None } else { o.value.clone() };
                    inner.objects.insert(o.id, Object { group_id, value, seq: head_seq, deleted: o.deleted, uploaded_at: now });
                    accepted.push(crate::relay::Accepted { id: o.id, seq: head_seq });
                }
            }
        }
        if head_seq != head.head_seq {
            inner.heads.entry(group_id).or_default().head_seq = head_seq;
            for d in inner.group_devices(group_id) {
                if d != device_id {
                    inner.wake(d, Wakeup::HeadAdvanced { group_id, head_seq });
                }
            }
        }
        Ok(PushResult::Accepted { accepted, conflicts, head_seq })
    }

    async fn pull_changes(&self, device_id: Uuid, group_id: Uuid, since_seq: i64, limit: i32) -> Result<ChangeSet, ApiError> {
        let inner = self.call()?;
        inner.require_member(device_id, group_id)?;
        if since_seq < 0 || limit < 0 {
            return Err(err(Code::InvalidArgument, "since_seq and limit are not negative"));
        }
        let limit = if limit == 0 { DEFAULT_PAGE } else { limit.min(MAX_PAGE) } as usize;
        let Some(head) = inner.heads.get(&group_id).cloned() else {
            return Ok(ChangeSet { entries: Vec::new(), head_seq: 0, evicted_through_seq: 0, min_acked_seq: None, more: false });
        };
        let mut rows: Vec<Entry> = inner
            .objects
            .iter()
            .filter(|(_, o)| o.group_id == group_id && o.seq > since_seq)
            .map(|(id, o)| Entry { id: *id, value: o.value.clone(), seq: o.seq, deleted: o.deleted })
            .collect();
        rows.sort_by_key(|e| e.seq);
        let more = rows.len() > limit;
        rows.truncate(limit);
        Ok(ChangeSet { entries: rows, head_seq: head.head_seq, evicted_through_seq: head.evicted_through_seq, min_acked_seq: inner.min_acked(group_id), more })
    }

    async fn ack_seq(&self, device_id: Uuid, group_id: Uuid, seq: i64) -> Result<AckResult, ApiError> {
        let mut inner = self.call()?;
        inner.require_member(device_id, group_id)?;
        let head = inner.head(group_id);
        if seq < 0 {
            return Err(err(Code::InvalidArgument, "seq: negative"));
        }
        if seq > head.head_seq {
            return Err(err(Code::FailedPrecondition, "ack above the head"));
        }
        match inner.ack_of(group_id, device_id) {
            _ if seq == 0 => {}
            None => return Err(err(Code::FailedPrecondition, "a device's first ack in a group is 0")),
            Some(c) if seq < c => return Err(err(Code::FailedPrecondition, "ack below the device's own cursor")),
            Some(_) => {}
        }
        let now = inner.now_ms;
        inner.heads.entry(group_id).or_default();
        inner.devices.insert((group_id, device_id), DeviceRow { acked_seq: seq, updated_at: now });
        Ok(AckResult { evicted_through_seq: head.evicted_through_seq, min_acked_seq: inner.min_acked(group_id) })
    }

    async fn get_heads(&self, device_id: Uuid, group_ids: Vec<Uuid>) -> Result<Vec<Head>, ApiError> {
        let inner = self.call()?;
        let mut heads = Vec::new();
        for group_id in group_ids {
            if inner.role(device_id, group_id)?.is_none() {
                continue;
            }
            let h = inner.head(group_id);
            heads.push(Head { group_id, head_seq: h.head_seq, evicted_through_seq: h.evicted_through_seq, min_acked_seq: inner.min_acked(group_id) });
        }
        Ok(heads)
    }

    async fn request_history(&self, device_id: Uuid, group_id: Uuid, from_seq: i64, to_seq: i64, object_id: Option<Uuid>) -> Result<HistoryRequest, ApiError> {
        let mut inner = self.call()?;
        inner.require_member(device_id, group_id)?;
        if from_seq < 1 || to_seq < from_seq {
            return Err(err(Code::InvalidArgument, "from_seq is at least 1 and to_seq at least from_seq"));
        }
        let floor = inner.head(group_id).evicted_through_seq;
        let holders = inner.holders_except(group_id, floor, device_id);
        let row = RequestRow { group_id, requester_device: device_id, from_seq, to_seq, object_id, claimed_by: None, claimed_at: None, created_at: inner.now_ms };
        if holders.is_empty() {
            return Ok(inner.request_msg(Uuid::nil(), &row, HistoryStatus::Unfillable));
        }
        let id = crate::ids::random(&mut self.rng.lock().expect("no poison"));
        let msg = inner.request_msg(id, &row, HistoryStatus::Open);
        inner.requests.insert(id, row);
        for d in holders {
            inner.wake(d, Wakeup::HistoryRequested { request_id: id, group_id });
        }
        Ok(msg)
    }

    async fn list_history_requests(&self, device_id: Uuid, group_id: Uuid) -> Result<Vec<HistoryRequest>, ApiError> {
        let inner = self.call()?;
        inner.require_member(device_id, group_id)?;
        let abandoned_before = inner.now_ms - CLAIM_TAKEOVER_MS;
        Ok(inner
            .requests
            .iter()
            .filter(|(_, r)| r.group_id == group_id && r.claimed_at.is_none_or(|t| t < abandoned_before))
            .map(|(id, r)| inner.request_msg(*id, r, HistoryStatus::Open))
            .collect())
    }

    async fn claim_history(&self, device_id: Uuid, request_id: Uuid) -> Result<HistoryRequest, ApiError> {
        let mut inner = self.call()?;
        let request = inner.requests.get(&request_id).cloned().ok_or_else(|| err(Code::NotFound, "history request not found"))?;
        inner.require_member(device_id, request.group_id)?;
        if request.requester_device == device_id {
            return Err(err(Code::InvalidArgument, "a device does not answer its own request"));
        }
        if !inner.is_holder(request.group_id, device_id) {
            return Err(err(Code::PermissionDenied, "the calling device does not hold the group"));
        }
        let now = inner.now_ms;
        let takeover = now - CLAIM_TAKEOVER_MS;
        let row = inner.requests.get_mut(&request_id).expect("read above");
        if row.claimed_by.is_none() || row.claimed_at.is_some_and(|t| t < takeover) {
            row.claimed_by = Some(device_id);
            row.claimed_at = Some(now);
            let row = row.clone();
            Ok(inner.request_msg(request_id, &row, HistoryStatus::Claimed))
        } else {
            let row = row.clone();
            Ok(inner.request_msg(request_id, &row, HistoryStatus::Taken))
        }
    }

    async fn push_history(&self, device_id: Uuid, request_id: Uuid, entries: Vec<Entry>, complete: bool) -> Result<PushHistoryResult, ApiError> {
        let mut inner = self.call()?;
        let request = inner.requests.get(&request_id).cloned().ok_or_else(|| err(Code::NotFound, "history request not found (completed or expired)"))?;
        inner.require_member(device_id, request.group_id)?;
        if request.claimed_by != Some(device_id) {
            return Err(err(Code::PermissionDenied, "the calling device does not hold the claim"));
        }
        let mut last = i64::MIN;
        for e in &entries {
            if e.seq <= last {
                return Err(err(Code::InvalidArgument, "entries: oldest first, strictly ascending"));
            }
            last = e.seq;
            if e.seq < request.from_seq || e.seq > request.to_seq {
                return Err(err(Code::InvalidArgument, format!("entry {}: seq {} outside [{}, {}]", e.id, e.seq, request.from_seq, request.to_seq)));
            }
            if let Some(object_id) = request.object_id
                && e.id != object_id
            {
                return Err(err(Code::InvalidArgument, format!("entry {}: the request is for object {object_id}", e.id)));
            }
            match (&e.value, e.deleted) {
                (Some(v), _) if v.len() > MAX_VALUE_BYTES => return Err(err(Code::InvalidArgument, format!("entry {}: value over 1 MiB", e.id))),
                (None, false) => return Err(err(Code::InvalidArgument, format!("entry {}: a live object has a value", e.id))),
                _ => {}
            }
        }
        let head = inner.head(request.group_id);
        let now = inner.now_ms;
        let (mut inserted, mut skipped) = (0, 0);
        for e in &entries {
            if e.seq > head.head_seq {
                return Err(err(Code::InvalidArgument, format!("entry {}: seq {} above the head", e.id, e.seq)));
            }
            match inner.objects.entry(e.id) {
                std::collections::btree_map::Entry::Occupied(_) => skipped += 1,
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(Object { group_id: request.group_id, value: if e.deleted { None } else { e.value.clone() }, seq: e.seq, deleted: e.deleted, uploaded_at: now });
                    inserted += 1;
                }
            }
        }
        if complete {
            inner.requests.remove(&request_id);
        }
        let through_seq = entries.last().map_or(if complete { request.to_seq } else { request.from_seq - 1 }, |e| e.seq);
        inner.wake(request.requester_device, Wakeup::HistoryReady { request_id, group_id: request.group_id, through_seq, complete });
        Ok(PushHistoryResult { inserted, skipped, through_seq, complete })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::block_on;

    fn live(id: u128, base_seq: i64) -> PushEntry {
        PushEntry { id: Uuid::from_u128(id), value: Some(vec![id as u8]), base_seq, deleted: false }
    }

    #[test]
    fn the_push_rules_of_the_plan() {
        let m = RelayModel::new([1; 32]);
        let (g, u, d, d2) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        m.register_device(d, u);
        m.register_device(d2, u);
        m.add_member(g, u, Role::Member);
        assert_eq!(block_on(m.push_objects(d, g, vec![live(10, 0)])).unwrap(), PushResult::BehindFloor { evicted_through_seq: 0 }, "no row: not a holder");
        block_on(m.ack_seq(d, g, 0)).unwrap();
        let r = block_on(m.push_objects(d, g, vec![live(10, 0), live(11, 0)])).unwrap();
        assert!(matches!(r, PushResult::Accepted { ref accepted, head_seq: 2, .. } if accepted.len() == 2));
        let r = block_on(m.push_objects(d, g, vec![live(10, 0)])).unwrap();
        assert!(matches!(r, PushResult::Accepted { ref conflicts, head_seq: 2, .. } if conflicts.len() == 1 && conflicts[0].current.seq == 1), "a stale base conflicts");
        let r = block_on(m.push_objects(d, g, vec![PushEntry { id: Uuid::from_u128(10), value: None, base_seq: 1, deleted: true }])).unwrap();
        assert!(matches!(r, PushResult::Accepted { head_seq: 3, .. }));
        assert_eq!(m.take_wakeups(d2).len(), 2, "the other device is woken per push that wrote");
        assert!(m.take_wakeups(d).is_empty(), "never the writer");
        let page = block_on(m.pull_changes(d, g, 0, 0)).unwrap();
        assert_eq!(page.entries.iter().map(|e| (e.seq, e.deleted)).collect::<Vec<_>>(), vec![(2, false), (3, true)]);
        assert_eq!(block_on(m.ack_seq(d, g, 4)).unwrap_err().code, Code::FailedPrecondition, "above the head");
        block_on(m.ack_seq(d, g, 3)).unwrap();
        assert_eq!(block_on(m.ack_seq(d, g, 2)).unwrap_err().code, Code::FailedPrecondition, "backwards");
        assert_eq!(block_on(m.ack_seq(d2, g, 3)).unwrap_err().code, Code::FailedPrecondition, "a first ack above 0");
    }

    #[test]
    fn eviction_by_ack_and_by_age_moves_the_floor_and_drops_stale_devices() {
        let m = RelayModel::new([1; 32]);
        let (g, u, d, d2) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        m.register_device(d, u);
        m.register_device(d2, u);
        m.add_member(g, u, Role::Admin);
        block_on(m.ack_seq(d, g, 0)).unwrap();
        block_on(m.ack_seq(d2, g, 0)).unwrap();
        block_on(m.push_objects(d, g, vec![live(10, 0), live(11, 0)])).unwrap();
        block_on(m.ack_seq(d, g, 2)).unwrap();
        m.evict();
        assert_eq!(m.head(g), (2, 0), "d2 has not acked: nothing goes by ack");
        block_on(m.ack_seq(d2, g, 1)).unwrap();
        m.evict();
        assert_eq!(m.head(g), (2, 1));
        assert_eq!(m.log(g).len(), 1);
        m.advance(WINDOW_MS + 1);
        block_on(m.push_objects(d, g, vec![live(12, 0)])).unwrap();
        m.evict();
        assert_eq!(m.head(g), (3, 2), "the old row went by age; the new one stays");
        assert_eq!(m.ack(g, d2), None, "d2 was below the floor and silent for the window: gone");
        assert_eq!(m.ack(g, d), Some(2), "d is at the floor: it holds the group");
    }

    #[test]
    fn history_requests_need_a_holder_and_only_the_claimer_pushes() {
        let m = RelayModel::new([1; 32]);
        let (g, u, d, d2) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        m.register_device(d, u);
        m.register_device(d2, u);
        m.add_member(g, u, Role::Member);
        block_on(m.ack_seq(d, g, 0)).unwrap();
        block_on(m.push_objects(d, g, vec![live(10, 0)])).unwrap();
        block_on(m.ack_seq(d, g, 1)).unwrap();
        block_on(m.ack_seq(d2, g, 0)).unwrap();
        m.evict();
        assert_eq!(m.head(g), (1, 0), "d2 at 0 pins the minimum");
        block_on(m.ack_seq(d2, g, 1)).unwrap();
        m.evict();
        assert_eq!(m.head(g), (1, 1));
        // A third device joins: nothing in the window, d and d2 hold.
        let d3 = Uuid::from_u128(5);
        m.register_device(d3, u);
        block_on(m.ack_seq(d3, g, 0)).unwrap();
        let req = block_on(m.request_history(d3, g, 1, 1, None)).unwrap();
        assert_eq!(req.status, HistoryStatus::Open);
        assert_eq!(m.take_wakeups(d), vec![Wakeup::HistoryRequested { request_id: req.id, group_id: g }]);
        assert_eq!(block_on(m.claim_history(d3, req.id)).unwrap_err().code, Code::InvalidArgument, "not its own");
        assert_eq!(block_on(m.claim_history(d, req.id)).unwrap().status, HistoryStatus::Claimed);
        assert_eq!(block_on(m.claim_history(d2, req.id)).unwrap().status, HistoryStatus::Taken);
        assert_eq!(block_on(m.push_history(d2, req.id, vec![], true)).unwrap_err().code, Code::PermissionDenied);
        let r = block_on(m.push_history(d, req.id, vec![Entry { id: Uuid::from_u128(10), value: Some(vec![10]), seq: 1, deleted: false }], true)).unwrap();
        assert_eq!((r.inserted, r.skipped, r.through_seq), (1, 0, 1));
        assert_eq!(m.take_wakeups(d3), vec![Wakeup::HistoryReady { request_id: req.id, group_id: g, through_seq: 1, complete: true }]);
        assert_eq!(m.open_requests(g), 0);
        assert_eq!(m.log(g).len(), 1, "restored at its seq");
        let none = block_on(m.request_history(Uuid::from_u128(6), g, 1, 1, None));
        assert_eq!(none.unwrap_err().code, Code::PermissionDenied, "an unknown device");
    }
}
