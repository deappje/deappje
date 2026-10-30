//! The reconciler of `group_purge` and its worker policy.
//!
//! The pass is LEVEL-TRIGGERED: read the whole claim-time snapshot
//! (`claim.object` — meta, spec, status) and do whatever that state still
//! requires. Every pass: empty the group's own rows (idempotent), wait for
//! the subgroups' purges, then ask `relay`, `storage` and `device` to drop
//! what they hold of the group, a batch per call, until all three answer
//! zero; then the group row goes, the object marks itself deleted and its
//! next pass, confirming the same, removes it (the Directive §3 invariant
//! 6: `Delete` only on a deleting object, on confirmed absence).
//!
//! An `Err` from the pass is any `BoxError` (a `?` on a query, a dispatch)
//! and resolves to `Retry { status: None }`; a pass that knows what it
//! wants written returns an `Outcome` instead.
//!
//! `WorkerConfig`'s scheduling fields (resync, backoff, max_attempts,
//! attempt_timeout) MUST be identical on every replica.

use std::time::Duration;

use basable_core::{BoxError, Ctx};
use basable_processingobject::{
    Backoff, Claim, NoAfterComplete, Outcome, Reconciler as ReconcilerTrait, Worker, WorkerConfig,
};
use chrono::Utc;
use messages::{PurgeGroupBlobsRequest, PurgeGroupKeysRequest, PurgeGroupObjectsRequest};

use super::{Adapter, Spec, Status, phase};
use crate::Group;
use crate::repository as repo;

/// How long a parent's pass waits for its subgroups' purges.
const SUBGROUP_WAIT: Duration = Duration::from_secs(30);
/// How long the pass waits for the receivers' next batch: `storage` deletes
/// a group's prefix a batch per call, so a group with many photos takes a
/// few passes.
const BATCH_WAIT: Duration = Duration::from_secs(60);

/// The pass's two handles: this nanoservice's sender (exactly its declared
/// sends, built once from the router) and the component itself.
pub struct Reconciler<R: 'static> {
    pub(crate) sender: interfaces::GroupSender<'static, R>,
    pub(crate) this: &'static Group,
}

impl<R> Reconciler<R>
where
    R: interfaces::GroupRoutes + Send + Sync + 'static,
{
    pub fn new(router: &'static R, this: &'static Group) -> Self {
        Reconciler { sender: interfaces::GroupSender::new(router), this }
    }
}

/// What one pass found, after doing what it could.
enum Drive {
    /// Subgroup purges are still running; the parent waits for them.
    Subgroups,
    /// A receiver still holds something; its next batch comes next pass.
    Pending,
    /// A send failed; the next attempt re-drives.
    SendFailed(BoxError),
    /// Every receiver holds nothing of the group, and the row is gone.
    Done,
}

impl<R> ReconcilerTrait<Spec, Status, Adapter> for Reconciler<R>
where
    R: interfaces::GroupRoutes + Send + Sync + 'static,
{
    async fn reconcile(&self, ctx: &Ctx, claim: &mut Claim<Spec, Status, Adapter>) -> Result<Outcome<Status>, BoxError> {
        // ONE working value; the completion writes the whole row.
        let mut status = claim.object.status.clone();
        let drive = self.drive(ctx, claim, &mut status).await?;
        if claim.object.meta.deleting() {
            // Teardown: the pass that marked the object confirmed every
            // receiver empty and removed the row; this one confirms the
            // same before the envelope goes (a late arrival re-drives).
            return Ok(match drive {
                Drive::Done => Outcome::delete(),
                Drive::Subgroups => Outcome::converged_after(Some(status), SUBGROUP_WAIT),
                Drive::Pending => Outcome::converged_after(Some(status), BATCH_WAIT),
                Drive::SendFailed(e) => Outcome::retry(Some(status), e),
            });
        }
        Ok(match drive {
            Drive::Subgroups => Outcome::converged_after(Some(status), SUBGROUP_WAIT),
            Drive::Pending => Outcome::converged_after(Some(status), BATCH_WAIT),
            Drive::SendFailed(e) => Outcome::retry(Some(status), e),
            Drive::Done => {
                status.phase = phase::DONE.to_owned();
                // The object's work is done: mark it, which leaves this
                // completion due at once, and the next pass removes it.
                let r = self.this.group_purge_store.r#ref(claim.object.id);
                self.this.group_purge_store.mark_deleted(&r).await?;
                Outcome::converged(Some(status))
            }
        })
    }
}

impl<R> Reconciler<R>
where
    R: interfaces::GroupRoutes + Send + Sync + 'static,
{
    /// Does what the group's state still requires and records the counts.
    /// No transaction is open while a request is sent (invariant 7).
    async fn drive(&self, ctx: &Ctx, claim: &Claim<Spec, Status, Adapter>, status: &mut Status) -> Result<Drive, BoxError> {
        let group_id = claim.object.spec.group_id;
        status.phase = phase::PURGING.to_owned();

        // 1. This nanoservice's own rows of the group, idempotent.
        self.this.empty_group(group_id, Utc::now()).await?;

        // 2. Subgroups go first: their purges name this one as parent and
        //    are hard-deleted when done, so the count falls by itself.
        let subgroups = repo::count_child_purges(&*self.this.pool, claim.object.id).await?;
        status.subgroups_remaining = i32::try_from(subgroups).unwrap_or(i32::MAX);
        if subgroups > 0 {
            return Ok(Drive::Subgroups);
        }

        // 3. The three receivers, each idempotent, each a batch per call.
        let objects = match self.sender.send_purge_group_objects_request(ctx, PurgeGroupObjectsRequest { group_id }).await {
            Ok(r) => r.remaining,
            Err(e) => return Ok(Drive::SendFailed(BoxError::from(e))),
        };
        status.objects_remaining = Some(objects);
        let blobs = match self.sender.send_purge_group_blobs_request(ctx, PurgeGroupBlobsRequest { group_id }).await {
            Ok(r) => r.remaining,
            Err(e) => return Ok(Drive::SendFailed(BoxError::from(e))),
        };
        status.blobs_remaining = Some(blobs);
        let keys = match self.sender.send_purge_group_keys_request(ctx, PurgeGroupKeysRequest { group_id }).await {
            Ok(r) => r.remaining,
            Err(e) => return Ok(Drive::SendFailed(BoxError::from(e))),
        };
        status.keys_remaining = Some(keys);
        if objects > 0 || blobs > 0 || keys > 0 {
            return Ok(Drive::Pending);
        }

        // 4. Nothing of the group remains anywhere: the row goes.
        repo::delete_group(&*self.this.pool, group_id).await?;
        Ok(Drive::Done)
    }
}

/// The worker this nanoservice's `loops()` (lib.rs) runs for this type: one
/// per replica, identical policy everywhere. The app starts it and joins it
/// on shutdown. A purge must finish, so attempts are unbounded and a stuck
/// one is surfaced by age, never parked.
pub fn worker<R>(router: &'static R, this: &'static Group) -> Worker<Spec, Status, Adapter, Reconciler<R>, NoAfterComplete>
where
    R: interfaces::GroupRoutes + Send + Sync + 'static,
{
    Worker::new(
        this.group_purge_store.clone(),
        WorkerConfig {
            resync: Duration::from_secs(60 * 60),
            backoff: Backoff { base: Duration::from_secs(10), max: Duration::from_secs(5 * 60) },
            max_attempts: 0,
            attempt_timeout: Duration::from_secs(60),
            ..WorkerConfig::default()
        },
        Reconciler::new(router, this),
        NoAfterComplete,
    )
    .expect("group.group_purge: a valid worker policy")
}
