//! `DeviceService`: the Connect service of `device`. Each
//! method validates the request, converts the buffa message into the plain
//! `messages` struct, sends it through the api sender, and converts the
//! answer back. Errors map to Connect codes through basable_connect.

use std::sync::Arc;

use basable_connect::{ConnectRouter, into_connect_error, request_ctx};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use proto::connect::device::v1::DeviceService;
use proto::proto::device::v1::*;

use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct DeviceServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> DeviceService for DeviceServiceImpl<R> {
    async fn register_device(&self, ctx: RequestContext, request: ServiceRequest<'_, RegisterDeviceRequest>) -> ServiceResult<RegisterDeviceResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::RegisterDeviceRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `RegisterDeviceResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.register_device")))
    }
    async fn set_push_token(&self, ctx: RequestContext, request: ServiceRequest<'_, SetPushTokenRequest>) -> ServiceResult<SetPushTokenResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::SetPushTokenRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `SetPushTokenResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.set_push_token")))
    }
    async fn list_devices(&self, ctx: RequestContext, request: ServiceRequest<'_, ListDevicesRequest>) -> ServiceResult<ListDevicesResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::ListDevicesRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `ListDevicesResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.list_devices")))
    }
    async fn request_group_keys(&self, ctx: RequestContext, request: ServiceRequest<'_, RequestGroupKeysRequest>) -> ServiceResult<RequestGroupKeysResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::RequestGroupKeysRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `RequestGroupKeysResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.request_group_keys")))
    }
    async fn list_key_requests(&self, ctx: RequestContext, request: ServiceRequest<'_, ListKeyRequestsRequest>) -> ServiceResult<ListKeyRequestsResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::ListKeyRequestsRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `ListKeyRequestsResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.list_key_requests")))
    }
    async fn fulfil_key_request(&self, ctx: RequestContext, request: ServiceRequest<'_, FulfilKeyRequestRequest>) -> ServiceResult<FulfilKeyRequestResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::FulfilKeyRequestRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `FulfilKeyRequestResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.fulfil_key_request")))
    }
    async fn publish_wrapped_keys(&self, ctx: RequestContext, request: ServiceRequest<'_, PublishWrappedKeysRequest>) -> ServiceResult<PublishWrappedKeysResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::PublishWrappedKeysRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `PublishWrappedKeysResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.publish_wrapped_keys")))
    }
    async fn get_wrapped_keys(&self, ctx: RequestContext, request: ServiceRequest<'_, GetWrappedKeysRequest>) -> ServiceResult<GetWrappedKeysResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::GetWrappedKeysRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `GetWrappedKeysResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.get_wrapped_keys")))
    }
    async fn remove_device(&self, ctx: RequestContext, request: ServiceRequest<'_, RemoveDeviceRequest>) -> ServiceResult<RemoveDeviceResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::RemoveDeviceRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `RemoveDeviceResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.remove_device")))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(DeviceServiceImpl { sender: ApiSender::new(router) }));
}
