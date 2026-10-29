//! `SyncService`: the Connect service of `sync`. Each
//! method validates the request, converts the buffa message into the plain
//! `messages` struct, sends it through the api sender, and converts the
//! answer back. Errors map to Connect codes through basable_connect.

use std::sync::Arc;

use basable_connect::{ConnectRouter, into_connect_error, request_ctx};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use proto::connect::sync::v1::SyncService;
use proto::proto::sync::v1::*;

use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct SyncServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> SyncService for SyncServiceImpl<R> {
    async fn push_objects(&self, ctx: RequestContext, request: ServiceRequest<'_, PushObjectsRequest>) -> ServiceResult<PushObjectsResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::PushObjectsRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `PushObjectsResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.push_objects")))
    }
    async fn pull_changes(&self, ctx: RequestContext, request: ServiceRequest<'_, PullChangesRequest>) -> ServiceResult<PullChangesResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::PullChangesRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `PullChangesResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.pull_changes")))
    }
    async fn ack_seq(&self, ctx: RequestContext, request: ServiceRequest<'_, AckSeqRequest>) -> ServiceResult<AckSeqResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::AckSeqRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `AckSeqResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.ack_seq")))
    }
    async fn get_heads(&self, ctx: RequestContext, request: ServiceRequest<'_, GetHeadsRequest>) -> ServiceResult<GetHeadsResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::GetHeadsRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `GetHeadsResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.get_heads")))
    }
    async fn request_history(&self, ctx: RequestContext, request: ServiceRequest<'_, RequestHistoryRequest>) -> ServiceResult<RequestHistoryResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::RequestHistoryRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `RequestHistoryResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.request_history")))
    }
    async fn list_history_requests(&self, ctx: RequestContext, request: ServiceRequest<'_, ListHistoryRequestsRequest>) -> ServiceResult<ListHistoryRequestsResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::ListHistoryRequestsRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `ListHistoryRequestsResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.list_history_requests")))
    }
    async fn claim_history(&self, ctx: RequestContext, request: ServiceRequest<'_, ClaimHistoryRequest>) -> ServiceResult<ClaimHistoryResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::ClaimHistoryRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `ClaimHistoryResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.claim_history")))
    }
    async fn push_history(&self, ctx: RequestContext, request: ServiceRequest<'_, PushHistoryRequest>) -> ServiceResult<PushHistoryResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::PushHistoryRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `PushHistoryResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.push_history")))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(SyncServiceImpl { sender: ApiSender::new(router) }));
}
