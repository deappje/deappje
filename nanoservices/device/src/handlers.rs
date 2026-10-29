//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). Handlers are the INTENT boundary:
//! they validate, write desired state through the typed store (`create`,
//! `update_spec`, `mark_deleted`, `nudge`) or the repository, dispatch
//! request-path adapters `Unfenced`, and send through `s` (the sender that
//! exposes exactly the declared sends). They never write status.

use basable_core::{AppError, Ctx};
use interfaces::{DeviceHandler, DeviceRoutes, DeviceSender};

use crate::Device;

// Messages stay qualified: a message may share its name with this
// component (an `order` nanoservice handling an `Order` response).
impl<R: DeviceRoutes> DeviceHandler<R> for Device {
    async fn handle_register_device_request(&self, _ctx: &Ctx, _msg: messages::RegisterDeviceRequest, _s: DeviceSender<'_, R>) -> Result<messages::Device, AppError> {
        Err(crate::unimplemented_step("device.handle_register_device_request"))
    }
    async fn handle_set_push_token_request(&self, _ctx: &Ctx, _msg: messages::SetPushTokenRequest, _s: DeviceSender<'_, R>) -> Result<messages::Device, AppError> {
        Err(crate::unimplemented_step("device.handle_set_push_token_request"))
    }
    async fn handle_list_devices_request(&self, _ctx: &Ctx, _msg: messages::ListDevicesRequest, _s: DeviceSender<'_, R>) -> Result<messages::DeviceList, AppError> {
        Err(crate::unimplemented_step("device.handle_list_devices_request"))
    }
    async fn handle_request_group_keys_request(&self, _ctx: &Ctx, _msg: messages::RequestGroupKeysRequest, _s: DeviceSender<'_, R>) -> Result<messages::KeyRequest, AppError> {
        Err(crate::unimplemented_step("device.handle_request_group_keys_request"))
    }
    async fn handle_list_key_requests_request(&self, _ctx: &Ctx, _msg: messages::ListKeyRequestsRequest, _s: DeviceSender<'_, R>) -> Result<messages::KeyRequestList, AppError> {
        Err(crate::unimplemented_step("device.handle_list_key_requests_request"))
    }
    async fn handle_fulfil_key_request_request(&self, _ctx: &Ctx, _msg: messages::FulfilKeyRequestRequest, _s: DeviceSender<'_, R>) -> Result<messages::KeyRequest, AppError> {
        Err(crate::unimplemented_step("device.handle_fulfil_key_request_request"))
    }
    async fn handle_publish_wrapped_keys_request(&self, _ctx: &Ctx, _msg: messages::PublishWrappedKeysRequest, _s: DeviceSender<'_, R>) -> Result<messages::WrappedKeyList, AppError> {
        Err(crate::unimplemented_step("device.handle_publish_wrapped_keys_request"))
    }
    async fn handle_get_wrapped_keys_request(&self, _ctx: &Ctx, _msg: messages::GetWrappedKeysRequest, _s: DeviceSender<'_, R>) -> Result<messages::WrappedKeyList, AppError> {
        Err(crate::unimplemented_step("device.handle_get_wrapped_keys_request"))
    }
    async fn handle_remove_device_request(&self, _ctx: &Ctx, _msg: messages::RemoveDeviceRequest, _s: DeviceSender<'_, R>) -> Result<messages::Device, AppError> {
        Err(crate::unimplemented_step("device.handle_remove_device_request"))
    }
    async fn handle_group_head_advanced(&self, _ctx: &Ctx, _msg: messages::GroupHeadAdvanced, _s: DeviceSender<'_, R>) {
        // A void fan-out event: observe it, nudge an object or write a side
        // table owned by this nanoservice; never a status write.
        tracing::warn!(step = "device.handle_group_head_advanced", "unimplemented step reached");
    }
    async fn handle_member_removed(&self, _ctx: &Ctx, _msg: messages::MemberRemoved, _s: DeviceSender<'_, R>) {
        // A void fan-out event: observe it, nudge an object or write a side
        // table owned by this nanoservice; never a status write.
        tracing::warn!(step = "device.handle_member_removed", "unimplemented step reached");
    }
    async fn handle_history_requested(&self, _ctx: &Ctx, _msg: messages::HistoryRequested, _s: DeviceSender<'_, R>) {
        // A void fan-out event: observe it, nudge an object or write a side
        // table owned by this nanoservice; never a status write.
        tracing::warn!(step = "device.handle_history_requested", "unimplemented step reached");
    }
    async fn handle_history_ready(&self, _ctx: &Ctx, _msg: messages::HistoryReady, _s: DeviceSender<'_, R>) {
        // A void fan-out event: observe it, nudge an object or write a side
        // table owned by this nanoservice; never a status write.
        tracing::warn!(step = "device.handle_history_ready", "unimplemented step reached");
    }
    async fn handle_purge_group_keys_request(&self, _ctx: &Ctx, _msg: messages::PurgeGroupKeysRequest, _s: DeviceSender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        Err(crate::unimplemented_step("device.handle_purge_group_keys_request"))
    }
    async fn handle_purge_user_devices_request(&self, _ctx: &Ctx, _msg: messages::PurgeUserDevicesRequest, _s: DeviceSender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        Err(crate::unimplemented_step("device.handle_purge_user_devices_request"))
    }
}
