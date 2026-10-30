//! The reconciler of `user_purge` and its worker policy.
//!
//! The pass is LEVEL-TRIGGERED: read the whole claim-time snapshot
//! (`claim.object` — meta, spec, status) and do whatever that state still
//! requires. Every pass: a membership left? give up one, the deepest group
//! first, and come straight back. None left? ask `device` to purge the
//! user's devices until it answers zero. Then the Kratos identity: look
//! (does it exist?), act (`delete_identity` under the claim), and confirm
//! on the next pass by looking again, never by trusting the ack. Then the
//! object marks itself deleted and its next pass, confirming the same,
//! removes it (the Directive §3 invariant 6).
//!
//! An `Err` from the pass is any `BoxError` (a `?` on a query, a dispatch)
//! and resolves to `Retry { status: None }`; a pass that knows what it
//! wants written returns an `Outcome` instead.
//!
//! `WorkerConfig`'s scheduling fields (resync, backoff, max_attempts,
//! attempt_timeout) MUST be identical on every replica.

use std::time::Duration;

use basable_core::{BoxError, Ctx};
use basable_externaleffect::DispatchError;
use basable_processingobject::{
    Backoff, Claim, NoAfterComplete, Outcome, Reconciler as ReconcilerTrait, Worker, WorkerConfig,
};
use messages::PurgeUserDevicesRequest;

use super::{Adapter, Spec, Status, phase};
use crate::Group;
use crate::effects::DeleteIdentityArgs;

/// How long the pass waits for `device`'s next batch.
const DEVICE_WAIT: Duration = Duration::from_secs(30);

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

impl<R> ReconcilerTrait<Spec, Status, Adapter> for Reconciler<R>
where
    R: interfaces::GroupRoutes + Send + Sync + 'static,
{
    async fn reconcile(&self, ctx: &Ctx, claim: &mut Claim<Spec, Status, Adapter>) -> Result<Outcome<Status>, BoxError> {
        // ONE working value; the completion writes the whole row.
        let mut status = claim.object.status.clone();
        let user_id = claim.object.spec.user_id;
        let deleting = claim.object.meta.deleting();

        // 1. Memberships: one group per pass keeps each transaction small.
        if let Some(remaining) = self.this.leave_one_group(ctx, &self.sender, user_id).await? {
            status.phase = phase::LEAVING.to_owned();
            status.groups_remaining = i32::try_from(remaining).unwrap_or(i32::MAX);
            return Ok(Outcome::requeue_now(Some(status)));
        }
        status.groups_remaining = 0;

        // 2. Devices, with their keys and requests: `device` deletes a batch
        //    per call and answers what is left.
        status.phase = phase::DEVICES.to_owned();
        let devices = match self.sender.send_purge_user_devices_request(ctx, PurgeUserDevicesRequest { user_id }).await {
            Ok(r) => r.remaining,
            Err(e) => return Ok(Outcome::retry(Some(status), BoxError::from(e))),
        };
        status.devices_remaining = Some(devices);
        if devices > 0 {
            return Ok(Outcome::converged_after(Some(status), DEVICE_WAIT));
        }

        // 3. The identity: look, act, and confirm by looking again.
        status.phase = phase::IDENTITY.to_owned();
        if !status.identity_deleted {
            let exists = (self.this.calls.identity_exists)(ctx.clone(), user_id).await?.is_some();
            if exists {
                match self.this.calls.delete_identity.dispatch(ctx, &*claim, DeleteIdentityArgs { user_id }).await {
                    // Landed as far as the ack says; the next pass reads
                    // the identity and records the absence it finds.
                    Ok(_) => return Ok(Outcome::requeue_now(Some(status))),
                    Err(DispatchError::OwnershipLost) => return Ok(Outcome::retry(None, "ownership lost")),
                    // May have landed: the next pass looks before it acts.
                    Err(DispatchError::Ambiguous(e)) => return Ok(Outcome::retry(Some(status), e)),
                    Err(DispatchError::Definitive(e)) => return Ok(Outcome::blocked(Some(status), e)),
                    Err(DispatchError::ReplayWindowElapsed { .. }) => return Ok(Outcome::retry(Some(status), "replay window elapsed")),
                }
            }
            status.identity_deleted = true;
        }

        // 4. Nothing of the user remains.
        status.phase = phase::DONE.to_owned();
        if deleting {
            return Ok(Outcome::delete());
        }
        // Mark the object, which leaves this completion due at once; the
        // next pass confirms the same and removes it.
        let r = self.this.user_purge_store.r#ref(claim.object.id);
        self.this.user_purge_store.mark_deleted(&r).await?;
        Ok(Outcome::converged(Some(status)))
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
        this.user_purge_store.clone(),
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
    .expect("group.user_purge: a valid worker policy")
}
