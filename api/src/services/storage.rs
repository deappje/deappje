//! `StorageService`: the Connect service of `storage`. Each
//! method validates the request, converts the buffa message into the plain
//! `messages` struct, sends it through the api sender, and converts the
//! answer back. Errors map to Connect codes through basable_connect.

use std::sync::Arc;

use basable_connect::{ConnectRouter, into_connect_error, request_ctx};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use proto::connect::storage::v1::StorageService;
use proto::proto::storage::v1::*;

use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct StorageServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> StorageService for StorageServiceImpl<R> {
    async fn presign_upload(&self, ctx: RequestContext, request: ServiceRequest<'_, PresignUploadRequest>) -> ServiceResult<PresignUploadResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::PresignUploadRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `PresignUploadResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.presign_upload")))
    }
    async fn presign_download(&self, ctx: RequestContext, request: ServiceRequest<'_, PresignDownloadRequest>) -> ServiceResult<PresignDownloadResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::PresignDownloadRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `PresignDownloadResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.presign_download")))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(StorageServiceImpl { sender: ApiSender::new(router) }));
}
