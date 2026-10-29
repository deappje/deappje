//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). Handlers are the INTENT boundary:
//! they validate, write desired state through the typed store (`create`,
//! `update_spec`, `mark_deleted`, `nudge`) or the repository, dispatch
//! request-path adapters `Unfenced`, and send through `s` (the sender that
//! exposes exactly the declared sends). They never write status.

use basable_core::{AppError, Ctx};
use interfaces::{RelayHandler, RelayRoutes, RelaySender};

use crate::Relay;

// Messages stay qualified: a message may share its name with this
// component (an `order` nanoservice handling an `Order` response).
impl<R: RelayRoutes> RelayHandler<R> for Relay {
    async fn handle_push_objects_request(&self, _ctx: &Ctx, _msg: messages::PushObjectsRequest, _s: RelaySender<'_, R>) -> Result<messages::PushResult, AppError> {
        Err(crate::unimplemented_step("relay.handle_push_objects_request"))
    }
    async fn handle_pull_changes_request(&self, _ctx: &Ctx, _msg: messages::PullChangesRequest, _s: RelaySender<'_, R>) -> Result<messages::ChangeSet, AppError> {
        Err(crate::unimplemented_step("relay.handle_pull_changes_request"))
    }
    async fn handle_ack_seq_request(&self, _ctx: &Ctx, _msg: messages::AckSeqRequest, _s: RelaySender<'_, R>) -> Result<messages::AckResult, AppError> {
        Err(crate::unimplemented_step("relay.handle_ack_seq_request"))
    }
    async fn handle_get_heads_request(&self, _ctx: &Ctx, _msg: messages::GetHeadsRequest, _s: RelaySender<'_, R>) -> Result<messages::HeadList, AppError> {
        Err(crate::unimplemented_step("relay.handle_get_heads_request"))
    }
    async fn handle_request_history_request(&self, _ctx: &Ctx, _msg: messages::RequestHistoryRequest, _s: RelaySender<'_, R>) -> Result<messages::HistoryRequest, AppError> {
        Err(crate::unimplemented_step("relay.handle_request_history_request"))
    }
    async fn handle_list_history_requests_request(&self, _ctx: &Ctx, _msg: messages::ListHistoryRequestsRequest, _s: RelaySender<'_, R>) -> Result<messages::HistoryRequestList, AppError> {
        Err(crate::unimplemented_step("relay.handle_list_history_requests_request"))
    }
    async fn handle_claim_history_request(&self, _ctx: &Ctx, _msg: messages::ClaimHistoryRequest, _s: RelaySender<'_, R>) -> Result<messages::HistoryRequest, AppError> {
        Err(crate::unimplemented_step("relay.handle_claim_history_request"))
    }
    async fn handle_push_history_request(&self, _ctx: &Ctx, _msg: messages::PushHistoryRequest, _s: RelaySender<'_, R>) -> Result<messages::PushHistoryResult, AppError> {
        Err(crate::unimplemented_step("relay.handle_push_history_request"))
    }
    async fn handle_device_left_group(&self, _ctx: &Ctx, _msg: messages::DeviceLeftGroup, _s: RelaySender<'_, R>) {
        // A void fan-out event: observe it, nudge an object or write a side
        // table owned by this nanoservice; never a status write.
        tracing::warn!(step = "relay.handle_device_left_group", "unimplemented step reached");
    }
    async fn handle_purge_group_objects_request(&self, _ctx: &Ctx, _msg: messages::PurgeGroupObjectsRequest, _s: RelaySender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        Err(crate::unimplemented_step("relay.handle_purge_group_objects_request"))
    }
}
