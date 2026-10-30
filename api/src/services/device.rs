//! `DeviceService`: the Connect service of `device`. Each method takes the
//! caller from the validated session, converts the buffa message into the
//! plain `messages` struct, sends it through the api sender, and converts
//! the answer back. Errors map to Connect codes through basable_connect.

use std::str::FromStr;
use std::sync::Arc;

use basable_connect::{ConnectRouter, IntoConnect, request_ctx};
use basable_core::AppError;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use messages::{Platform, PushProvider};
use proto::connect::device::v1::DeviceService;
use proto::proto::device::v1::*;

use super::{caller, millis, optional_uuid_field, uuid_field};
use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct DeviceServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

fn platform_field(value: &str) -> Result<Platform, AppError> {
    Platform::from_str(value).map_err(|e| AppError::invalid_argument(format!("platform: {e}")))
}

fn device(d: messages::Device) -> Device {
    Device {
        id: d.id.to_string(),
        user_id: d.user_id.to_string(),
        public_key: d.public_key,
        platform: d.platform.as_str().to_owned(),
        push_provider: d.push.as_ref().map(|p| p.provider.as_str().to_owned()).unwrap_or_default(),
        push_token: d.push.map(|p| p.token).unwrap_or_default(),
        created_at_ms: millis(d.created_at),
        last_seen_at_ms: millis(d.last_seen_at),
        ..Default::default()
    }
}

fn device_info(d: messages::DeviceInfo) -> DeviceInfo {
    DeviceInfo {
        id: d.id.to_string(),
        user_id: d.user_id.to_string(),
        public_key: d.public_key,
        platform: d.platform.as_str().to_owned(),
        ..Default::default()
    }
}

fn key_request(r: messages::KeyRequest) -> KeyRequest {
    KeyRequest {
        id: r.id.to_string(),
        group_id: r.group_id.to_string(),
        device: Some(device_info(r.device)).into(),
        invite_id: r.invite_id.map(|id| id.to_string()).unwrap_or_default(),
        invite_tag: r.invite_tag.unwrap_or_default(),
        created_at_ms: millis(r.created_at),
        ..Default::default()
    }
}

fn wrapped_key(k: messages::WrappedKey) -> WrappedKey {
    WrappedKey {
        id: k.id.to_string(),
        group_id: k.group_id.to_string(),
        key_id: k.key_id,
        device_id: k.device_id.to_string(),
        wrapped: k.wrapped,
        wrapped_by_device: k.wrapped_by_device.to_string(),
        wrapped_by_public_key: k.wrapped_by_public_key,
        created_at_ms: millis(k.created_at),
        ..Default::default()
    }
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> DeviceService for DeviceServiceImpl<R> {
    async fn register_device(&self, ctx: RequestContext, request: ServiceRequest<'_, RegisterDeviceRequest>) -> ServiceResult<RegisterDeviceResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::RegisterDeviceRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            public_key: req.public_key.to_vec(),
            platform: platform_field(&req.platform).into_connect()?,
        };
        let d = self.sender.send_register_device_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(RegisterDeviceResponse { device: Some(device(d)).into(), ..Default::default() }))
    }

    async fn set_push_token(&self, ctx: RequestContext, request: ServiceRequest<'_, SetPushTokenRequest>) -> ServiceResult<SetPushTokenResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let push = if req.push_token.is_empty() {
            None
        } else {
            let provider = PushProvider::from_str(&req.push_provider)
                .map_err(|e| AppError::invalid_argument(format!("push_provider: {e}")))
                .into_connect()?;
            Some(messages::PushToken { provider, token: req.push_token })
        };
        let msg = messages::SetPushTokenRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            push,
        };
        let d = self.sender.send_set_push_token_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(SetPushTokenResponse { device: Some(device(d)).into(), ..Default::default() }))
    }

    async fn list_devices(&self, ctx: RequestContext, request: ServiceRequest<'_, ListDevicesRequest>) -> ServiceResult<ListDevicesResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::ListDevicesRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let list = self.sender.send_list_devices_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(ListDevicesResponse { devices: list.devices.into_iter().map(device_info).collect(), ..Default::default() }))
    }

    async fn request_group_keys(&self, ctx: RequestContext, request: ServiceRequest<'_, RequestGroupKeysRequest>) -> ServiceResult<RequestGroupKeysResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::RequestGroupKeysRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            invite_id: optional_uuid_field(&req.invite_id, "invite_id").into_connect()?,
            invite_tag: if req.invite_tag.is_empty() { None } else { Some(req.invite_tag.to_vec()) },
        };
        let r = self.sender.send_request_group_keys_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(RequestGroupKeysResponse { request: Some(key_request(r)).into(), ..Default::default() }))
    }

    async fn list_key_requests(&self, ctx: RequestContext, request: ServiceRequest<'_, ListKeyRequestsRequest>) -> ServiceResult<ListKeyRequestsResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::ListKeyRequestsRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
        };
        let list = self.sender.send_list_key_requests_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(ListKeyRequestsResponse { requests: list.requests.into_iter().map(key_request).collect(), ..Default::default() }))
    }

    async fn fulfil_key_request(&self, ctx: RequestContext, request: ServiceRequest<'_, FulfilKeyRequestRequest>) -> ServiceResult<FulfilKeyRequestResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::FulfilKeyRequestRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            request_id: uuid_field(&req.request_id, "request_id").into_connect()?,
            keys: req
                .keys
                .into_iter()
                .map(|k| messages::WrappedKeyInput { key_id: k.key_id, wrapped: k.wrapped.to_vec() })
                .collect(),
        };
        let r = self.sender.send_fulfil_key_request_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(FulfilKeyRequestResponse { request: Some(key_request(r)).into(), ..Default::default() }))
    }

    async fn publish_wrapped_keys(&self, ctx: RequestContext, request: ServiceRequest<'_, PublishWrappedKeysRequest>) -> ServiceResult<PublishWrappedKeysResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let mut wraps = Vec::with_capacity(req.wraps.len());
        for w in req.wraps {
            wraps.push(messages::WrapFor {
                device_id: uuid_field(&w.device_id, "wraps.device_id").into_connect()?,
                wrapped: w.wrapped.to_vec(),
            });
        }
        let msg = messages::PublishWrappedKeysRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            key_id: req.key_id,
            wraps,
        };
        let list = self.sender.send_publish_wrapped_keys_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(PublishWrappedKeysResponse { keys: list.keys.into_iter().map(wrapped_key).collect(), ..Default::default() }))
    }

    async fn get_wrapped_keys(&self, ctx: RequestContext, request: ServiceRequest<'_, GetWrappedKeysRequest>) -> ServiceResult<GetWrappedKeysResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::GetWrappedKeysRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: optional_uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let list = self.sender.send_get_wrapped_keys_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(GetWrappedKeysResponse { keys: list.keys.into_iter().map(wrapped_key).collect(), ..Default::default() }))
    }

    async fn remove_device(&self, ctx: RequestContext, request: ServiceRequest<'_, RemoveDeviceRequest>) -> ServiceResult<RemoveDeviceResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::RemoveDeviceRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
        };
        let d = self.sender.send_remove_device_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(RemoveDeviceResponse { device: Some(device(d)).into(), ..Default::default() }))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(DeviceServiceImpl { sender: ApiSender::new(router) }));
}
