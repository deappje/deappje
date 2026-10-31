//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). A plain executor over four tables;
//! the one piece of concurrency control in the system is here: every write
//! to a group's log, and every move of its floor, runs under the group's
//! counter row locked `FOR UPDATE` (`repository::lock_head`, the Directive
//! §7). Membership is `group`'s to answer and is asked per request; the
//! device id is the caller's word, within their membership, as the plan
//! says (a device id is an unguessable client-minted UUID).
//!
//! A messenger send is remote I/O (invariant 7): every event goes out
//! after the transaction it announces has committed.

use basable_core::{AppError, Code, Ctx};
use chrono::{DateTime, Duration, Utc};
use interfaces::{RelayHandler, RelayRoutes, RelaySender};
use messages::{
    Accepted, CheckMembershipRequest, Conflict, Entry, GroupHeadAdvanced, Head, HistoryReady, HistoryRequested, HistoryStatus, ObjectDeleted, PushEntry,
    PushResult, Role,
};
use uuid::Uuid;

use crate::repository as repo;
use crate::{Relay, model};

/// A value is at most this long; a larger one refuses the whole batch.
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
/// A pull's page when the request says 0, and its ceiling.
pub const DEFAULT_PAGE: i32 = 500;
pub const MAX_PAGE: i32 = 2000;
/// A claim this old without `complete` is abandoned: the next claimer
/// takes over.
pub const CLAIM_TAKEOVER: Duration = Duration::minutes(5);
/// After a complete restore from seq 1, eviction by ack leaves the group
/// alone this long, so a device joining a little after the one that asked
/// finds its history in the window and no holder uploads it again.
pub const RESTORE_GRACE: Duration = Duration::minutes(30);
/// A purge deletes this many rows per call.
const PURGE_BATCH: i64 = 10_000;

fn db(op: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    move |e| AppError::wrap(Code::Internal, format!("relay: {op}"), e)
}

fn entry(o: &model::Object) -> Entry {
    Entry { id: o.id, value: o.value.clone(), seq: o.seq, deleted: o.deleted }
}

/// A request as the calling device sees it. The claimer is named only to
/// itself: a device id is the caller's word (`02-relay.md`), so the
/// claimer's id in another member's hands would let it push as the
/// claimer.
fn history_msg(r: &model::HistoryRequest, device_id: Uuid, status: HistoryStatus) -> messages::HistoryRequest {
    messages::HistoryRequest {
        id: r.id,
        group_id: r.group_id,
        requester_device: r.requester_device,
        from_seq: r.from_seq,
        to_seq: r.to_seq,
        object_id: r.object_id,
        claimed_by: r.claimed_by.filter(|d| *d == device_id),
        claimed_at: r.claimed_at,
        created_at: r.created_at,
        through_seq: r.through_seq,
        status,
    }
}

/// Where a stored request stands for the calling device: complete once
/// the claimer's last call landed; else open when unclaimed or abandoned,
/// claimed when the caller holds it, taken when another device does.
fn status_of(r: &model::HistoryRequest, device_id: Uuid, abandoned_before: DateTime<Utc>) -> HistoryStatus {
    if r.completed_at.is_some() {
        return HistoryStatus::Complete;
    }
    match r.claimed_by {
        None => HistoryStatus::Open,
        Some(d) if d == device_id => HistoryStatus::Claimed,
        Some(_) if r.claimed_at.is_some_and(|t| t < abandoned_before) => HistoryStatus::Open,
        Some(_) => HistoryStatus::Taken,
    }
}

/// What the compare-and-set on seq decides for one pushed object, given
/// the row the server holds (`current`), as a pure function.
#[derive(Debug, PartialEq, Eq)]
pub enum Cas {
    /// No row, or the base is the current seq: write at the next seq.
    Accept,
    /// The base is stale: answer the current row.
    Conflict,
}

pub fn cas(current: Option<i64>, base_seq: i64) -> Cas {
    match current {
        None => Cas::Accept,
        Some(seq) if seq == base_seq => Cas::Accept,
        Some(_) => Cas::Conflict,
    }
}

/// Whether a device with this ack holds the group at this floor: a row at
/// or above the floor has the whole group.
pub fn is_holder(acked_seq: Option<i64>, floor: i64) -> bool {
    acked_seq.is_some_and(|a| a >= floor)
}

/// The ack rules: monotone, 0 creates or resets, never above the head,
/// never a first ack above 0 (a device starts at 0).
pub fn ack_allowed(seq: i64, current: Option<i64>, head_seq: i64) -> Result<(), AppError> {
    if seq < 0 {
        return Err(AppError::invalid_argument("seq: negative"));
    }
    if seq > head_seq {
        return Err(AppError::failed_precondition("ack above the head"));
    }
    match current {
        _ if seq == 0 => Ok(()),
        None => Err(AppError::failed_precondition("a device's first ack in a group is 0")),
        Some(c) if seq < c => Err(AppError::failed_precondition("ack below the device's own cursor")),
        Some(_) => Ok(()),
    }
}

/// The validation of a push batch before anything is locked.
fn validate_push(objects: &[PushEntry]) -> Result<(), AppError> {
    if objects.is_empty() {
        return Err(AppError::invalid_argument("objects: empty batch"));
    }
    for o in objects {
        if o.base_seq < 0 {
            return Err(AppError::invalid_argument(format!("object {}: base_seq negative", o.id)));
        }
        match (&o.value, o.deleted) {
            (Some(v), false) if v.is_empty() => return Err(AppError::invalid_argument(format!("object {}: empty value", o.id))),
            (Some(v), _) if v.len() > MAX_VALUE_BYTES => return Err(AppError::invalid_argument(format!("object {}: value over 1 MiB", o.id))),
            (None, false) => return Err(AppError::invalid_argument(format!("object {}: a live object has a value", o.id))),
            _ => {}
        }
    }
    let mut ids: Vec<Uuid> = objects.iter().map(|o| o.id).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != objects.len() {
        return Err(AppError::invalid_argument("objects: an id at most once per batch"));
    }
    Ok(())
}

/// The validation of a history push: oldest first, inside the request's
/// range, one object when the request names one.
fn validate_history_entries(req: &model::HistoryRequest, entries: &[Entry]) -> Result<(), AppError> {
    let mut last = i64::MIN;
    for e in entries {
        if e.seq <= last {
            return Err(AppError::invalid_argument("entries: oldest first, strictly ascending"));
        }
        last = e.seq;
        if e.seq < req.from_seq || e.seq > req.to_seq {
            return Err(AppError::invalid_argument(format!("entry {}: seq {} outside [{}, {}]", e.id, e.seq, req.from_seq, req.to_seq)));
        }
        if let Some(object_id) = req.object_id
            && e.id != object_id
        {
            return Err(AppError::invalid_argument(format!("entry {}: the request is for object {object_id}", e.id)));
        }
        match (&e.value, e.deleted) {
            (Some(v), _) if v.len() > MAX_VALUE_BYTES => return Err(AppError::invalid_argument(format!("entry {}: value over 1 MiB", e.id))),
            (None, false) => return Err(AppError::invalid_argument(format!("entry {}: a live object has a value", e.id))),
            _ => {}
        }
    }
    Ok(())
}

impl Relay {
    async fn role<R: RelayRoutes>(&self, ctx: &Ctx, s: &RelaySender<'_, R>, user_id: Uuid, group_id: Uuid) -> Result<Option<Role>, AppError> {
        Ok(s.send_check_membership_request(ctx, CheckMembershipRequest { user_id, group_id }).await?.role)
    }

    async fn require_member<R: RelayRoutes>(&self, ctx: &Ctx, s: &RelaySender<'_, R>, user_id: Uuid, group_id: Uuid) -> Result<Role, AppError> {
        self.role(ctx, s, user_id, group_id)
            .await?
            .ok_or_else(|| AppError::permission_denied("not a member of this group"))
    }

    /// Whether the device holds the group now: its row at or above the
    /// current floor.
    async fn holder(&self, group_id: Uuid, device_id: Uuid) -> Result<bool, AppError> {
        let floor = repo::get_head(&*self.pool, group_id).await.map_err(db("head"))?.map_or(0, |h| h.evicted_through_seq);
        let ack = repo::get_group_device(&*self.pool, group_id, device_id).await.map_err(db("group device"))?.map(|d| d.acked_seq);
        Ok(is_holder(ack, floor))
    }
}

// Messages stay qualified: a message may share its name with this
// component.
impl<R: RelayRoutes> RelayHandler<R> for Relay {
    async fn handle_push_objects_request(&self, ctx: &Ctx, msg: messages::PushObjectsRequest, s: RelaySender<'_, R>) -> Result<PushResult, AppError> {
        validate_push(&msg.objects)?;
        if !self.require_member(ctx, &s, msg.caller, msg.group_id).await?.can_write() {
            return Err(AppError::permission_denied("a viewer does not write to the log"));
        }
        let now = Utc::now();
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let head = repo::lock_head(&mut tx, msg.group_id).await.map_err(db("lock head"))?;
        let ack = repo::get_group_device(&mut *tx, msg.group_id, msg.device_id).await.map_err(db("group device"))?.map(|d| d.acked_seq);
        if !is_holder(ack, head.evicted_through_seq) {
            // Not a holder: its word is worth nothing, base_seq 0 included.
            return Ok(PushResult::BehindFloor { evicted_through_seq: head.evicted_through_seq });
        }
        let mut head_seq = head.head_seq;
        let mut accepted = Vec::new();
        let mut conflicts = Vec::new();
        let mut tombstones = Vec::new();
        for o in &msg.objects {
            let current = repo::get_object(&mut *tx, o.id).await.map_err(db("object"))?;
            if let Some(c) = &current
                && c.group_id != msg.group_id
            {
                return Err(AppError::failed_precondition(format!("object {}: the id belongs to another group", o.id)));
            }
            match cas(current.as_ref().map(|c| c.seq), o.base_seq) {
                Cas::Conflict => {
                    let c = current.expect("a conflict has a current row");
                    conflicts.push(Conflict { id: o.id, current: entry(&c) });
                }
                Cas::Accept => {
                    head_seq += 1;
                    let value = if o.deleted { None } else { o.value.as_deref() };
                    if current.is_some() {
                        repo::update_object(&mut *tx, o.id, value, head_seq, o.deleted, now).await.map_err(db("update object"))?;
                    } else {
                        let row = model::Object {
                            id: o.id,
                            group_id: msg.group_id,
                            value: value.map(<[u8]>::to_vec),
                            seq: head_seq,
                            deleted: o.deleted,
                            uploaded_at: now,
                        };
                        repo::insert_object(&mut *tx, &row).await.map_err(db("insert object"))?;
                    }
                    accepted.push(Accepted { id: o.id, seq: head_seq });
                    if o.deleted {
                        tombstones.push(o.id);
                    }
                }
            }
        }
        if head_seq != head.head_seq {
            repo::set_head_seq(&mut *tx, msg.group_id, head_seq).await.map_err(db("set head"))?;
        }
        tx.commit().await.map_err(db("commit"))?;
        if !accepted.is_empty() {
            s.send_group_head_advanced(ctx, GroupHeadAdvanced { group_id: msg.group_id, head_seq, writer_device: msg.device_id })
                .await;
        }
        for object_id in tombstones {
            s.send_object_deleted(ctx, ObjectDeleted { group_id: msg.group_id, object_id }).await;
        }
        Ok(PushResult::Accepted { accepted, conflicts, head_seq })
    }

    async fn handle_pull_changes_request(&self, ctx: &Ctx, msg: messages::PullChangesRequest, s: RelaySender<'_, R>) -> Result<messages::ChangeSet, AppError> {
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        if msg.since_seq < 0 || msg.limit < 0 {
            return Err(AppError::invalid_argument("since_seq and limit are not negative"));
        }
        let limit = if msg.limit == 0 { DEFAULT_PAGE } else { msg.limit.min(MAX_PAGE) } as i64;
        let head = repo::get_head(&*self.pool, msg.group_id).await.map_err(db("head"))?;
        let Some(head) = head else {
            return Ok(messages::ChangeSet { entries: Vec::new(), head_seq: 0, evicted_through_seq: 0, min_acked_seq: None, more: false });
        };
        let mut rows = repo::entries_after(&*self.pool, msg.group_id, msg.since_seq, limit + 1).await.map_err(db("entries"))?;
        let more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let min_acked_seq = repo::min_acked(&*self.pool, msg.group_id).await.map_err(db("min ack"))?;
        Ok(messages::ChangeSet {
            entries: rows.iter().map(entry).collect(),
            head_seq: head.head_seq,
            evicted_through_seq: head.evicted_through_seq,
            min_acked_seq,
            more,
        })
    }

    async fn handle_ack_seq_request(&self, ctx: &Ctx, msg: messages::AckSeqRequest, s: RelaySender<'_, R>) -> Result<messages::AckResult, AppError> {
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        let now = Utc::now();
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let head = repo::lock_head(&mut tx, msg.group_id).await.map_err(db("lock head"))?;
        let current = repo::get_group_device(&mut *tx, msg.group_id, msg.device_id).await.map_err(db("group device"))?.map(|d| d.acked_seq);
        ack_allowed(msg.seq, current, head.head_seq)?;
        repo::upsert_ack(&mut *tx, msg.group_id, msg.device_id, msg.seq, now).await.map_err(db("ack"))?;
        let min_acked_seq = repo::min_acked(&mut *tx, msg.group_id).await.map_err(db("min ack"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(messages::AckResult { evicted_through_seq: head.evicted_through_seq, min_acked_seq })
    }

    async fn handle_get_heads_request(&self, ctx: &Ctx, msg: messages::GetHeadsRequest, s: RelaySender<'_, R>) -> Result<messages::HeadList, AppError> {
        let mut heads = Vec::with_capacity(msg.group_ids.len());
        for group_id in msg.group_ids {
            // A group the caller is no longer in is left out: that is how a
            // device learns of a removal while offline.
            if self.role(ctx, &s, msg.caller, group_id).await?.is_none() {
                continue;
            }
            let head = repo::get_head(&*self.pool, group_id).await.map_err(db("head"))?;
            let min_acked_seq = repo::min_acked(&*self.pool, group_id).await.map_err(db("min ack"))?;
            heads.push(Head {
                group_id,
                head_seq: head.as_ref().map_or(0, |h| h.head_seq),
                evicted_through_seq: head.as_ref().map_or(0, |h| h.evicted_through_seq),
                min_acked_seq,
            });
        }
        Ok(messages::HeadList { heads })
    }

    async fn handle_request_history_request(&self, ctx: &Ctx, msg: messages::RequestHistoryRequest, s: RelaySender<'_, R>) -> Result<messages::HistoryRequest, AppError> {
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        if msg.from_seq < 1 || msg.to_seq < msg.from_seq {
            return Err(AppError::invalid_argument("from_seq is at least 1 and to_seq at least from_seq"));
        }
        let now = Utc::now();
        let head = repo::get_head(&*self.pool, msg.group_id).await.map_err(db("head"))?;
        let floor = head.as_ref().map_or(0, |h| h.evicted_through_seq);
        let restored_through = head.as_ref().map_or(0, |h| h.restored_through_seq);
        let whole_range = msg.from_seq == 1 && msg.object_id.is_none();
        let row = model::HistoryRequest {
            id: Uuid::new_v4(),
            group_id: msg.group_id,
            requester_device: msg.device_id,
            from_seq: msg.from_seq,
            to_seq: msg.to_seq,
            object_id: msg.object_id,
            claimed_by: None,
            claimed_at: None,
            through_seq: msg.from_seq - 1,
            completed_at: None,
            created_at: now,
        };
        if whole_range && msg.to_seq <= restored_through {
            // The window holds the range already: a restore for another
            // device landed and no eviction has reached it since (the
            // witness). Nothing is stored and nobody is woken; the
            // requester pages what its pull gave it.
            return Ok(history_msg(&model::HistoryRequest { id: Uuid::nil(), through_seq: msg.to_seq, ..row }, msg.device_id, HistoryStatus::Complete));
        }
        let holders = repo::holders_except(&*self.pool, msg.group_id, floor, msg.device_id).await.map_err(db("holders"))?;
        if holders.is_empty() {
            // Nobody holds anything older than the window: the window is
            // the history, and nothing is stored.
            return Ok(history_msg(&model::HistoryRequest { id: Uuid::nil(), ..row }, msg.device_id, HistoryStatus::Unfillable));
        }
        // A whole-range request an open one already reaches rides along: it
        // is stored, not announced, holders do not list it, and it
        // completes with the one that covers it (push_history), so one
        // upload serves both.
        let covered = whole_range && repo::covering_request_exists(&*self.pool, msg.group_id, msg.to_seq).await.map_err(db("covering request"))?;
        repo::insert_history_request(&*self.pool, &row).await.map_err(db("insert request"))?;
        if !covered {
            s.send_history_requested(
                ctx,
                HistoryRequested {
                    request_id: row.id,
                    group_id: msg.group_id,
                    requester_device: msg.device_id,
                    from_seq: msg.from_seq,
                    to_seq: msg.to_seq,
                    object_id: msg.object_id,
                    devices: holders,
                },
            )
            .await;
        }
        Ok(history_msg(&row, msg.device_id, HistoryStatus::Open))
    }

    async fn handle_get_history_request_request(&self, ctx: &Ctx, msg: messages::GetHistoryRequestRequest, s: RelaySender<'_, R>) -> Result<messages::HistoryRequest, AppError> {
        let request = repo::get_history_request(&*self.pool, msg.request_id)
            .await
            .map_err(db("request"))?
            .ok_or_else(|| AppError::not_found("history request not found (expired)"))?;
        self.require_member(ctx, &s, msg.caller, request.group_id).await?;
        Ok(history_msg(&request, msg.device_id, status_of(&request, msg.device_id, Utc::now() - CLAIM_TAKEOVER)))
    }

    async fn handle_list_history_requests_request(&self, ctx: &Ctx, msg: messages::ListHistoryRequestsRequest, s: RelaySender<'_, R>) -> Result<messages::HistoryRequestList, AppError> {
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        let abandoned_before = Utc::now() - CLAIM_TAKEOVER;
        let rows = repo::claimable_requests(&*self.pool, msg.group_id, abandoned_before).await.map_err(db("requests"))?;
        Ok(messages::HistoryRequestList { requests: rows.iter().map(|r| history_msg(r, msg.device_id, HistoryStatus::Open)).collect() })
    }

    async fn handle_claim_history_request(&self, ctx: &Ctx, msg: messages::ClaimHistoryRequest, s: RelaySender<'_, R>) -> Result<messages::HistoryRequest, AppError> {
        let request = repo::get_history_request(&*self.pool, msg.request_id)
            .await
            .map_err(db("request"))?
            .filter(|r| r.completed_at.is_none())
            .ok_or_else(|| AppError::not_found("history request not found (completed or expired)"))?;
        self.require_member(ctx, &s, msg.caller, request.group_id).await?;
        if request.requester_device == msg.device_id {
            return Err(AppError::invalid_argument("a device does not answer its own request"));
        }
        if !self.holder(request.group_id, msg.device_id).await? {
            return Err(AppError::permission_denied("the calling device does not hold the group"));
        }
        let now = Utc::now();
        match repo::claim_request(&*self.pool, request.id, msg.device_id, now, now - CLAIM_TAKEOVER).await.map_err(db("claim"))? {
            Some(claimed) => Ok(history_msg(&claimed, msg.device_id, HistoryStatus::Claimed)),
            None => {
                // Another device holds it, or it completed since the read
                // above: the row as it stands now says which.
                let now_row = repo::get_history_request(&*self.pool, request.id)
                    .await
                    .map_err(db("request"))?
                    .ok_or_else(|| AppError::not_found("history request not found (expired)"))?;
                Ok(history_msg(&now_row, msg.device_id, status_of(&now_row, msg.device_id, now - CLAIM_TAKEOVER)))
            }
        }
    }

    async fn handle_push_history_request(&self, ctx: &Ctx, msg: messages::PushHistoryRequest, s: RelaySender<'_, R>) -> Result<messages::PushHistoryResult, AppError> {
        let request = repo::get_history_request(&*self.pool, msg.request_id)
            .await
            .map_err(db("request"))?
            .filter(|r| r.completed_at.is_none())
            .ok_or_else(|| AppError::not_found("history request not found (completed or expired)"))?;
        // Serving history is reading: any role answers.
        self.require_member(ctx, &s, msg.caller, request.group_id).await?;
        if request.claimed_by != Some(msg.device_id) {
            return Err(AppError::permission_denied("the calling device does not hold the claim"));
        }
        validate_history_entries(&request, &msg.entries)?;
        let now = Utc::now();
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let head = repo::lock_head(&mut tx, request.group_id).await.map_err(db("lock head"))?;
        let mut inserted = 0;
        let mut skipped = 0;
        for e in &msg.entries {
            if e.seq > head.head_seq {
                return Err(AppError::invalid_argument(format!("entry {}: seq {} above the head", e.id, e.seq)));
            }
            let row = model::Object {
                id: e.id,
                group_id: request.group_id,
                value: if e.deleted { None } else { e.value.clone() },
                seq: e.seq,
                deleted: e.deleted,
                uploaded_at: now,
            };
            if repo::insert_object_if_absent(&mut *tx, &row).await.map_err(db("re-insert"))? {
                inserted += 1;
            } else {
                skipped += 1;
            }
        }
        // Complete is the whole range, whatever the last entry's seq: the
        // range may end in rows nobody holds any more. The row stays, marked,
        // so the requester reads it on its next sync whether or not the
        // wake-up below arrives.
        let through_seq = if msg.complete { request.to_seq } else { msg.entries.last().map_or(request.through_seq, |e| e.seq) };
        repo::record_history_push(&mut *tx, request.id, through_seq, msg.complete.then_some(now)).await.map_err(db("record push"))?;
        // A complete restore of the whole range leaves the witness on the
        // group head (every row at or below `to_seq` any holder has is in
        // the log, until an eviction reaches it: worker.rs) and completes
        // the whole-range requests it covers: one upload serves them all.
        let mut covered = Vec::new();
        if msg.complete && request.from_seq == 1 && request.object_id.is_none() {
            repo::set_restored(&mut *tx, request.group_id, request.to_seq, now).await.map_err(db("restored"))?;
            covered = repo::complete_covered_requests(&mut *tx, request.group_id, request.to_seq, request.id, now).await.map_err(db("covered requests"))?;
        }
        tx.commit().await.map_err(db("commit"))?;
        s.send_history_ready(
            ctx,
            HistoryReady { request_id: request.id, requester_device: request.requester_device, through_seq, complete: msg.complete },
        )
        .await;
        for r in covered {
            s.send_history_ready(ctx, HistoryReady { request_id: r.id, requester_device: r.requester_device, through_seq: r.to_seq, complete: true }).await;
        }
        Ok(messages::PushHistoryResult { inserted, skipped, through_seq, complete: msg.complete })
    }

    async fn handle_device_left_group(&self, _ctx: &Ctx, msg: messages::DeviceLeftGroup, _s: RelaySender<'_, R>) {
        if let Err(e) = repo::delete_group_device(&*self.pool, msg.group_id, msg.device_id).await {
            tracing::warn!(group = %msg.group_id, device = %msg.device_id, error = %e, "device left group: row not deleted");
        }
    }

    async fn handle_purge_group_objects_request(&self, _ctx: &Ctx, msg: messages::PurgeGroupObjectsRequest, _s: RelaySender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        repo::delete_objects_of_group(&*self.pool, msg.group_id, PURGE_BATCH).await.map_err(db("purge objects"))?;
        let objects = repo::count_objects_of_group(&*self.pool, msg.group_id).await.map_err(db("count objects"))?;
        if objects > 0 {
            return Ok(messages::PurgeResult { remaining: objects });
        }
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::delete_group_devices(&mut *tx, msg.group_id).await.map_err(db("purge devices"))?;
        repo::delete_history_requests_of_group(&mut *tx, msg.group_id).await.map_err(db("purge requests"))?;
        repo::delete_head(&mut *tx, msg.group_id).await.map_err(db("purge head"))?;
        tx.commit().await.map_err(db("commit"))?;
        let remaining = repo::count_group_devices(&*self.pool, msg.group_id).await.map_err(db("count devices"))?
            + repo::count_history_requests_of_group(&*self.pool, msg.group_id).await.map_err(db("count requests"))?
            + i64::from(repo::get_head(&*self.pool, msg.group_id).await.map_err(db("head"))?.is_some());
        Ok(messages::PurgeResult { remaining })
    }
}

/// The window: how long a row stays at most, and how long a device may
/// stay away without starting over.
pub const WINDOW: Duration = Duration::hours(672);

/// `now - WINDOW`.
pub fn window_cutoff(now: DateTime<Utc>) -> DateTime<Utc> {
    now - WINDOW
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_and_set_on_seq() {
        assert_eq!(cas(None, 0), Cas::Accept, "no row: the holder's word");
        assert_eq!(cas(None, 7), Cas::Accept, "no row, even with a base: evicted while the holder held it");
        assert_eq!(cas(Some(3), 3), Cas::Accept);
        assert_eq!(cas(Some(3), 2), Cas::Conflict);
        assert_eq!(cas(Some(3), 0), Cas::Conflict, "a create over an existing row");
    }

    #[test]
    fn holders_are_at_or_above_the_floor() {
        assert!(!is_holder(None, 0), "no row");
        assert!(is_holder(Some(0), 0), "ack 0 at floor 0: a fresh group");
        assert!(!is_holder(Some(2), 3));
        assert!(is_holder(Some(3), 3));
        assert!(is_holder(Some(9), 3));
    }

    #[test]
    fn ack_rules() {
        assert!(ack_allowed(0, None, 0).is_ok(), "the first act");
        assert!(ack_allowed(0, Some(5), 9).is_ok(), "starting over");
        assert_eq!(ack_allowed(3, None, 9).unwrap_err().code(), Code::FailedPrecondition, "a first ack above 0");
        assert_eq!(ack_allowed(2, Some(5), 9).unwrap_err().code(), Code::FailedPrecondition, "backwards");
        assert!(ack_allowed(5, Some(5), 9).is_ok(), "the same again");
        assert!(ack_allowed(9, Some(5), 9).is_ok(), "the head");
        assert_eq!(ack_allowed(10, Some(5), 9).unwrap_err().code(), Code::FailedPrecondition, "above the head");
        assert_eq!(ack_allowed(-1, None, 9).unwrap_err().code(), Code::InvalidArgument);
    }

    #[test]
    fn push_batches_are_validated_whole() {
        let id = Uuid::from_u128(1);
        let live = |value: Vec<u8>| PushEntry { id, value: Some(value), base_seq: 0, deleted: false };
        assert!(validate_push(&[]).is_err());
        assert!(validate_push(&[live(vec![1])]).is_ok());
        assert!(validate_push(&[live(vec![])]).is_err(), "empty value");
        assert!(validate_push(&[live(vec![0; MAX_VALUE_BYTES + 1])]).is_err(), "over 1 MiB");
        assert!(validate_push(&[PushEntry { id, value: None, base_seq: 1, deleted: true }]).is_ok(), "a tombstone has no value");
        assert!(validate_push(&[PushEntry { id, value: None, base_seq: 1, deleted: false }]).is_err(), "a live object has one");
        assert!(validate_push(&[live(vec![1]), live(vec![2])]).is_err(), "an id once per batch");
    }

    #[test]
    fn history_entries_are_validated_against_the_request() {
        let req = model::HistoryRequest {
            id: Uuid::from_u128(1),
            group_id: Uuid::from_u128(2),
            requester_device: Uuid::from_u128(3),
            from_seq: 1,
            to_seq: 5,
            object_id: None,
            claimed_by: None,
            claimed_at: None,
            through_seq: 0,
            completed_at: None,
            created_at: Utc::now(),
        };
        let e = |seq: i64| Entry { id: Uuid::new_v4(), value: Some(vec![1]), seq, deleted: false };
        assert!(validate_history_entries(&req, &[e(1), e(3), e(5)]).is_ok());
        assert!(validate_history_entries(&req, &[e(3), e(1)]).is_err(), "oldest first");
        assert!(validate_history_entries(&req, &[e(6)]).is_err(), "outside the range");
        let one = model::HistoryRequest { object_id: Some(Uuid::from_u128(9)), ..req };
        assert!(validate_history_entries(&one, &[e(2)]).is_err(), "another object");
        assert!(validate_history_entries(&one, &[Entry { id: Uuid::from_u128(9), value: None, seq: 2, deleted: true }]).is_ok());
    }
}
