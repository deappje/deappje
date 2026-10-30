//! `StorageService`: the Connect service of `storage`. Each method takes
//! the caller from the validated session, converts the buffa message into
//! the plain `messages` struct, sends it through the api sender, and
//! converts the answer back. Errors map to Connect codes through
//! basable_connect.

use std::sync::Arc;

use basable_connect::{ConnectRouter, IntoConnect, request_ctx};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use proto::connect::storage::v1::StorageService;
use proto::proto::storage::v1::*;

use super::{caller, millis, uuid_field};
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
        let req = request.to_owned_message();
        let msg = messages::PresignUploadRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            key: req.key,
            size: req.size,
        };
        let g = self.sender.send_presign_upload_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(PresignUploadResponse { key: g.key, url: g.url, expires_at_ms: millis(g.expires_at), ..Default::default() }))
    }

    async fn presign_download(&self, ctx: RequestContext, request: ServiceRequest<'_, PresignDownloadRequest>) -> ServiceResult<PresignDownloadResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::PresignDownloadRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            key: req.key,
        };
        let g = self.sender.send_presign_download_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(PresignDownloadResponse { key: g.key, url: g.url, expires_at_ms: millis(g.expires_at), ..Default::default() }))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(StorageServiceImpl { sender: ApiSender::new(router) }));
}
