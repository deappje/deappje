//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). Handlers are the INTENT boundary:
//! they validate, write through the repository, dispatch the push adapters
//! `Unfenced` (through `push.rs`), and send through `s` (the sender that
//! exposes exactly the declared sends). A plain executor: no status, no
//! processing object.
//!
//! Every request names the calling device, which must be the caller's
//! (`own_device`); membership is `group`'s to answer and is asked through
//! `CheckMembershipRequest` wherever a rule reads it. A push goes out
//! after the commit it announces (a messenger send and a push are remote
//! I/O, invariant 7).

use std::collections::HashMap;
use std::str::FromStr;

use basable_core::{AppError, Code, Ctx};
use chrono::Utc;
use interfaces::{DeviceHandler, DeviceRoutes, DeviceSender};
use messages::{CheckMembershipRequest, DeviceLeftGroup, Platform, PushProvider, Role};
use uuid::Uuid;

use crate::push::Wakeup;
use crate::repository as repo;
use crate::{Device, model};

/// An X25519 public key.
const PUBLIC_KEY_LEN: usize = 32;

fn db(op: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    move |e| AppError::wrap(Code::Internal, format!("device: {op}"), e)
}

fn platform_of(d: &model::Device) -> Result<Platform, AppError> {
    Platform::from_str(&d.platform).map_err(|e| AppError::wrap(Code::Internal, format!("device: device {} carries an unknown platform", d.id), e))
}

fn push_of(d: &model::Device) -> Result<Option<messages::PushToken>, AppError> {
    match (&d.push_provider, &d.push_token) {
        (Some(p), Some(t)) => Ok(Some(messages::PushToken {
            provider: PushProvider::from_str(p).map_err(|e| AppError::wrap(Code::Internal, format!("device: device {} carries an unknown push provider", d.id), e))?,
            token: t.clone(),
        })),
        _ => Ok(None),
    }
}

fn device_msg(d: &model::Device) -> Result<messages::Device, AppError> {
    Ok(messages::Device {
        id: d.id,
        user_id: d.user_id,
        public_key: d.public_key.clone(),
        platform: platform_of(d)?,
        push: push_of(d)?,
        created_at: d.created_at,
        last_seen_at: d.last_seen_at,
    })
}

fn device_info(d: &model::Device) -> Result<messages::DeviceInfo, AppError> {
    Ok(messages::DeviceInfo {
        id: d.id,
        user_id: d.user_id,
        public_key: d.public_key.clone(),
        platform: platform_of(d)?,
    })
}

fn key_request_msg(r: &model::KeyRequest, d: &model::Device) -> Result<messages::KeyRequest, AppError> {
    Ok(messages::KeyRequest {
        id: r.id,
        group_id: r.group_id,
        device: device_info(d)?,
        invite_id: r.invite_id,
        invite_tag: r.invite_tag.clone(),
        created_at: r.created_at,
    })
}

fn wrapped_key_msg(k: &model::WrappedKeyCopy) -> messages::WrappedKey {
    messages::WrappedKey {
        id: k.key.id,
        group_id: k.key.group_id,
        key_id: k.key.key_id,
        device_id: k.key.device_id,
        wrapped: k.key.wrapped.clone(),
        wrapped_by_device: k.key.wrapped_by_device,
        wrapped_by_public_key: k.wrapped_by_public_key.clone(),
        created_at: k.key.created_at,
    }
}

/// The rotation rule: a key id is published by one device. A batch whose
/// key id already has rows from another device is refused, since the two
/// batches wrap different keys under one name; the loser re-reads and
/// adopts.
pub(crate) fn publisher_allowed(existing_wrapper: Option<Uuid>, publisher: Uuid) -> Result<(), AppError> {
    match existing_wrapper {
        Some(other) if other != publisher => Err(AppError::failed_precondition("this key id was already published by another device; adopt it")),
        _ => Ok(()),
    }
}

/// Who may approve a key request: the requesting device's own user, or an
/// admin or member of the group; a viewer approves nothing but its own.
pub(crate) fn approver_allowed(own_device: bool, role: Option<Role>) -> Result<(), AppError> {
    if own_device {
        return Ok(());
    }
    match role {
        Some(r) if r.can_write() => Ok(()),
        Some(_) => Err(AppError::permission_denied("a viewer does not approve another user's device")),
        None => Err(AppError::permission_denied("not a member of this group")),
    }
}

impl Device {
    /// The calling device, which must be the caller's.
    async fn own_device(&self, device_id: Uuid, caller: Uuid) -> Result<model::Device, AppError> {
        let d = repo::get_device(&*self.pool, device_id)
            .await
            .map_err(db("device"))?
            .ok_or_else(|| AppError::not_found("device not registered"))?;
        if d.user_id != caller {
            return Err(AppError::permission_denied("not your device"));
        }
        Ok(d)
    }

    async fn role<R: DeviceRoutes>(&self, ctx: &Ctx, s: &DeviceSender<'_, R>, user_id: Uuid, group_id: Uuid) -> Result<Option<Role>, AppError> {
        Ok(s.send_check_membership_request(ctx, CheckMembershipRequest { user_id, group_id }).await?.role)
    }

    async fn require_member<R: DeviceRoutes>(&self, ctx: &Ctx, s: &DeviceSender<'_, R>, user_id: Uuid, group_id: Uuid) -> Result<Role, AppError> {
        self.role(ctx, s, user_id, group_id)
            .await?
            .ok_or_else(|| AppError::permission_denied("not a member of this group"))
    }

    async fn require_holder(&self, group_id: Uuid, device_id: Uuid) -> Result<(), AppError> {
        if !repo::device_holds(&*self.pool, group_id, device_id).await.map_err(db("holder"))? {
            return Err(AppError::permission_denied("the calling device holds no key for this group"));
        }
        Ok(())
    }

    /// Drops a user's keys and requests for one group and tells `relay`
    /// per device; what `MemberRemoved` and the purges share.
    async fn drop_user_from_group<R: DeviceRoutes>(&self, ctx: &Ctx, s: &DeviceSender<'_, R>, group_id: Uuid, user_id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let devices: Vec<Uuid> = repo::devices_of_user(&*self.pool, user_id)
            .await
            .map_err(db("devices of user"))?
            .into_iter()
            .map(|d| d.id)
            .collect();
        if devices.is_empty() {
            return Ok(devices);
        }
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::delete_keys_of_devices_in_group(&mut *tx, group_id, &devices).await.map_err(db("delete keys"))?;
        repo::delete_requests_of_devices_in_group(&mut *tx, group_id, &devices).await.map_err(db("delete requests"))?;
        tx.commit().await.map_err(db("commit"))?;
        for device_id in &devices {
            s.send_device_left_group(ctx, DeviceLeftGroup { group_id, device_id: *device_id }).await;
        }
        Ok(devices)
    }
}

// Messages stay qualified: a message may share its name with this
// component (the `device` nanoservice handling a `Device` response).
impl<R: DeviceRoutes> DeviceHandler<R> for Device {
    async fn handle_register_device_request(&self, _ctx: &Ctx, msg: messages::RegisterDeviceRequest, _s: DeviceSender<'_, R>) -> Result<messages::Device, AppError> {
        if msg.public_key.len() != PUBLIC_KEY_LEN {
            return Err(AppError::invalid_argument("public_key: an X25519 public key is 32 bytes"));
        }
        if msg.device_id.is_nil() {
            return Err(AppError::invalid_argument("device_id: the device mints its id"));
        }
        let now = Utc::now();
        match repo::get_device(&*self.pool, msg.device_id).await.map_err(db("device"))? {
            None => {
                let row = model::Device {
                    id: msg.device_id,
                    user_id: msg.caller,
                    public_key: msg.public_key,
                    platform: msg.platform.as_str().to_owned(),
                    push_provider: None,
                    push_token: None,
                    created_at: now,
                    last_seen_at: now,
                };
                repo::insert_device(&*self.pool, &row).await.map_err(|e| {
                    if basable_db::sqlstate::is_unique_violation(&e) {
                        AppError::already_exists("device id already registered")
                    } else {
                        db("insert device")(e)
                    }
                })?;
                device_msg(&row)
            }
            Some(existing) => {
                if existing.user_id != msg.caller {
                    return Err(AppError::permission_denied("not your device"));
                }
                if existing.public_key != msg.public_key {
                    return Err(AppError::failed_precondition("a device's public key never changes: remove the device and register a new one"));
                }
                let touched = repo::touch_device(&*self.pool, existing.id, msg.platform.as_str(), now)
                    .await
                    .map_err(db("touch device"))?
                    .unwrap_or(existing);
                device_msg(&touched)
            }
        }
    }

    async fn handle_set_push_token_request(&self, _ctx: &Ctx, msg: messages::SetPushTokenRequest, _s: DeviceSender<'_, R>) -> Result<messages::Device, AppError> {
        let device = self.own_device(msg.device_id, msg.caller).await?;
        let (provider, token) = match &msg.push {
            Some(p) if p.token.trim().is_empty() => return Err(AppError::invalid_argument("push token: empty")),
            Some(p) => (Some(p.provider.as_str()), Some(p.token.as_str())),
            None => (None, None),
        };
        let updated = repo::set_push(&*self.pool, device.id, provider, token)
            .await
            .map_err(db("set push"))?
            .unwrap_or(device);
        device_msg(&updated)
    }

    async fn handle_list_devices_request(&self, _ctx: &Ctx, msg: messages::ListDevicesRequest, _s: DeviceSender<'_, R>) -> Result<messages::DeviceList, AppError> {
        self.own_device(msg.device_id, msg.caller).await?;
        self.require_holder(msg.group_id, msg.device_id).await?;
        let holders = repo::holder_devices(&*self.pool, msg.group_id).await.map_err(db("holders"))?;
        Ok(messages::DeviceList {
            group_id: msg.group_id,
            devices: holders.iter().map(device_info).collect::<Result<_, _>>()?,
        })
    }

    async fn handle_request_group_keys_request(&self, ctx: &Ctx, msg: messages::RequestGroupKeysRequest, s: DeviceSender<'_, R>) -> Result<messages::KeyRequest, AppError> {
        let device = self.own_device(msg.device_id, msg.caller).await?;
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        if repo::device_holds(&*self.pool, msg.group_id, device.id).await.map_err(db("holder"))? {
            return Err(AppError::failed_precondition("the device already holds keys for this group"));
        }
        if let Some(open) = repo::open_key_request(&*self.pool, msg.group_id, device.id).await.map_err(db("open request"))? {
            return key_request_msg(&open, &device);
        }
        let row = model::KeyRequest {
            id: Uuid::new_v4(),
            group_id: msg.group_id,
            device_id: device.id,
            invite_id: msg.invite_id,
            invite_tag: msg.invite_tag,
            created_at: Utc::now(),
        };
        let inserted = repo::insert_key_request(&*self.pool, &row).await.map_err(db("insert request"))?;
        let row = if inserted {
            row
        } else {
            // Lost a race with our own repeat: that one is the open request.
            repo::open_key_request(&*self.pool, msg.group_id, device.id)
                .await
                .map_err(db("open request"))?
                .ok_or_else(|| AppError::internal("device: the key request vanished"))?
        };
        // Every holder is asked, the requester's own other devices included:
        // one of them is where a person approves.
        let holders = self.member_holders(ctx, &s, msg.group_id, &[]).await?;
        self.wake_all(ctx, &holders, &Wakeup::KeyRequest { group_id: msg.group_id, request_id: row.id }).await;
        key_request_msg(&row, &device)
    }

    async fn handle_list_key_requests_request(&self, _ctx: &Ctx, msg: messages::ListKeyRequestsRequest, _s: DeviceSender<'_, R>) -> Result<messages::KeyRequestList, AppError> {
        self.own_device(msg.device_id, msg.caller).await?;
        let open = repo::requests_for_holder(&*self.pool, msg.device_id).await.map_err(db("requests"))?;
        Ok(messages::KeyRequestList {
            requests: open.iter().map(|o| key_request_msg(&o.request, &o.device)).collect::<Result<_, _>>()?,
        })
    }

    async fn handle_fulfil_key_request_request(&self, ctx: &Ctx, msg: messages::FulfilKeyRequestRequest, s: DeviceSender<'_, R>) -> Result<messages::KeyRequest, AppError> {
        let fulfiller = self.own_device(msg.device_id, msg.caller).await?;
        if msg.keys.is_empty() {
            return Err(AppError::invalid_argument("keys: a fulfilment hands over at least one wrapped key"));
        }
        let mut key_ids: Vec<i32> = msg.keys.iter().map(|k| k.key_id).collect();
        key_ids.sort_unstable();
        key_ids.dedup();
        if key_ids.len() != msg.keys.len() || key_ids.first().is_some_and(|k| *k < 1) || msg.keys.iter().any(|k| k.wrapped.is_empty()) {
            return Err(AppError::invalid_argument("keys: one non-empty wrap per positive key id"));
        }
        let request = repo::get_key_request(&*self.pool, msg.request_id)
            .await
            .map_err(db("request"))?
            .ok_or_else(|| AppError::not_found("key request not found"))?;
        self.require_holder(request.group_id, fulfiller.id).await?;
        let Some(requester) = repo::get_device(&*self.pool, request.device_id).await.map_err(db("requester"))? else {
            // The device went while its request stayed: nothing to hand over.
            repo::delete_key_request(&*self.pool, request.id).await.map_err(db("delete request"))?;
            return Err(AppError::not_found("the requesting device is gone"));
        };
        let own = requester.user_id == msg.caller;
        let role = self.role(ctx, &s, msg.caller, request.group_id).await?;
        approver_allowed(own, role)?;
        if !own && self.role(ctx, &s, requester.user_id, request.group_id).await?.is_none() {
            repo::delete_key_request(&*self.pool, request.id).await.map_err(db("delete request"))?;
            return Err(AppError::failed_precondition("the requesting device's user is no longer a member"));
        }
        let now = Utc::now();
        let rows: Vec<model::WrappedKey> = msg
            .keys
            .iter()
            .map(|k| model::WrappedKey {
                id: Uuid::new_v4(),
                group_id: request.group_id,
                key_id: k.key_id,
                device_id: requester.id,
                wrapped: k.wrapped.clone(),
                wrapped_by_device: fulfiller.id,
                created_at: now,
            })
            .collect();
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::insert_wrapped_keys(&mut *tx, &rows).await.map_err(db("insert keys"))?;
        repo::delete_key_request(&mut *tx, request.id).await.map_err(db("delete request"))?;
        tx.commit().await.map_err(db("commit"))?;
        // The requester sees its keys arrive: that is the fulfilment.
        self.wake(ctx, &requester, &Wakeup::Keys { group_id: request.group_id, request_id: request.id }).await;
        key_request_msg(&request, &requester)
    }

    async fn handle_publish_wrapped_keys_request(&self, ctx: &Ctx, msg: messages::PublishWrappedKeysRequest, s: DeviceSender<'_, R>) -> Result<messages::WrappedKeyList, AppError> {
        let publisher = self.own_device(msg.device_id, msg.caller).await?;
        if msg.key_id < 1 {
            return Err(AppError::invalid_argument("key_id: key ids start at 1"));
        }
        if msg.wraps.is_empty() || msg.wraps.iter().any(|w| w.wrapped.is_empty()) {
            return Err(AppError::invalid_argument("wraps: one non-empty wrap per device"));
        }
        let mut targets: Vec<Uuid> = msg.wraps.iter().map(|w| w.device_id).collect();
        targets.sort_unstable();
        targets.dedup();
        if targets.len() != msg.wraps.len() {
            return Err(AppError::invalid_argument("wraps: a device at most once"));
        }
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        // A rotation is published by a holder of the key it replaces.
        if msg.key_id > 1 && !repo::devices_holding_key(&*self.pool, msg.group_id, msg.key_id - 1, &[publisher.id]).await.map_err(db("holder"))?.contains(&publisher.id) {
            return Err(AppError::failed_precondition("the publishing device does not hold the previous key"));
        }
        publisher_allowed(repo::wrapper_of_key(&*self.pool, msg.group_id, msg.key_id).await.map_err(db("wrapper"))?, publisher.id)?;

        // A wrap lands only for a registered device whose user is a member,
        // and, for a rotation, that holds the previous key; the rest of the
        // batch is dropped, and the answer shows what landed.
        let known = repo::devices_by_id(&*self.pool, &targets).await.map_err(db("devices"))?;
        let previous_holders = if msg.key_id > 1 {
            repo::devices_holding_key(&*self.pool, msg.group_id, msg.key_id - 1, &targets).await.map_err(db("holders"))?
        } else {
            targets.clone()
        };
        let mut roles: HashMap<Uuid, Option<Role>> = HashMap::new();
        let now = Utc::now();
        let mut rows = Vec::new();
        for w in &msg.wraps {
            let Some(d) = known.iter().find(|d| d.id == w.device_id) else { continue };
            if !previous_holders.contains(&d.id) {
                continue;
            }
            let role = match roles.get(&d.user_id) {
                Some(r) => *r,
                None => {
                    let r = self.role(ctx, &s, d.user_id, msg.group_id).await?;
                    roles.insert(d.user_id, r);
                    r
                }
            };
            if role.is_none() {
                continue;
            }
            rows.push(model::WrappedKey {
                id: Uuid::new_v4(),
                group_id: msg.group_id,
                key_id: msg.key_id,
                device_id: d.id,
                wrapped: w.wrapped.clone(),
                wrapped_by_device: publisher.id,
                created_at: now,
            });
        }
        repo::insert_wrapped_keys(&*self.pool, &rows).await.map_err(db("insert keys"))?;
        let landed = repo::keys_of(&*self.pool, msg.group_id, msg.key_id, &targets).await.map_err(db("keys"))?;
        Ok(messages::WrappedKeyList { keys: landed.iter().map(wrapped_key_msg).collect() })
    }

    async fn handle_get_wrapped_keys_request(&self, _ctx: &Ctx, msg: messages::GetWrappedKeysRequest, _s: DeviceSender<'_, R>) -> Result<messages::WrappedKeyList, AppError> {
        self.own_device(msg.device_id, msg.caller).await?;
        let keys = repo::keys_for_device(&*self.pool, msg.device_id, msg.group_id).await.map_err(db("keys"))?;
        Ok(messages::WrappedKeyList { keys: keys.iter().map(wrapped_key_msg).collect() })
    }

    async fn handle_remove_device_request(&self, ctx: &Ctx, msg: messages::RemoveDeviceRequest, s: DeviceSender<'_, R>) -> Result<messages::Device, AppError> {
        let device = self.own_device(msg.device_id, msg.caller).await?;
        let groups = repo::groups_held_by_device(&*self.pool, device.id).await.map_err(db("groups held"))?;
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::delete_keys_of_devices(&mut *tx, &[device.id]).await.map_err(db("delete keys"))?;
        repo::delete_requests_of_devices(&mut *tx, &[device.id]).await.map_err(db("delete requests"))?;
        repo::delete_device(&mut *tx, device.id).await.map_err(db("delete device"))?;
        tx.commit().await.map_err(db("commit"))?;
        for group_id in groups {
            s.send_device_left_group(ctx, DeviceLeftGroup { group_id, device_id: device.id }).await;
        }
        device_msg(&device)
    }

    async fn handle_group_head_advanced(&self, ctx: &Ctx, msg: messages::GroupHeadAdvanced, s: DeviceSender<'_, R>) {
        match self.member_holders(ctx, &s, msg.group_id, &[msg.writer_device]).await {
            Ok(holders) => {
                self.wake_all(ctx, &holders, &Wakeup::Head { group_id: msg.group_id, head_seq: msg.head_seq }).await;
            }
            Err(e) => tracing::warn!(group = %msg.group_id, error = %e, "head advanced: holders not woken"),
        }
    }

    async fn handle_member_removed(&self, ctx: &Ctx, msg: messages::MemberRemoved, s: DeviceSender<'_, R>) {
        if let Err(e) = self.drop_user_from_group(ctx, &s, msg.group_id, msg.user_id).await {
            tracing::warn!(group = %msg.group_id, user = %msg.user_id, error = %e, "member removed: keys not dropped");
            return;
        }
        // The remaining holders rotate: one of them mints the next key.
        match self.member_holders(ctx, &s, msg.group_id, &[]).await {
            Ok(holders) => {
                self.wake_all(ctx, &holders, &Wakeup::Rotate { group_id: msg.group_id, removed_user: msg.user_id }).await;
            }
            Err(e) => tracing::warn!(group = %msg.group_id, error = %e, "member removed: rotation not requested"),
        }
    }

    async fn handle_history_requested(&self, ctx: &Ctx, msg: messages::HistoryRequested, _s: DeviceSender<'_, R>) {
        // `relay` chose the devices (the holders by ack, the requester
        // excluded); this nanoservice only knows how to reach them.
        let ids: Vec<Uuid> = msg.devices.iter().copied().filter(|d| *d != msg.requester_device).collect();
        match repo::devices_by_id(&*self.pool, &ids).await {
            Ok(devices) => {
                let wakeup = Wakeup::History {
                    request_id: msg.request_id,
                    group_id: msg.group_id,
                    from_seq: msg.from_seq,
                    to_seq: msg.to_seq,
                    object_id: msg.object_id,
                };
                self.wake_all(ctx, &devices, &wakeup).await;
            }
            Err(e) => tracing::warn!(request = %msg.request_id, error = %e, "history requested: holders not woken"),
        }
    }

    async fn handle_history_ready(&self, ctx: &Ctx, msg: messages::HistoryReady, _s: DeviceSender<'_, R>) {
        match repo::get_device(&*self.pool, msg.requester_device).await {
            Ok(Some(device)) => {
                self.wake(ctx, &device, &Wakeup::HistoryReady { request_id: msg.request_id, through_seq: msg.through_seq, complete: msg.complete })
                    .await;
            }
            Ok(None) => tracing::info!(request = %msg.request_id, device = %msg.requester_device, "history ready: the requester is gone"),
            Err(e) => tracing::warn!(request = %msg.request_id, error = %e, "history ready: requester not woken"),
        }
    }

    async fn handle_purge_group_keys_request(&self, _ctx: &Ctx, msg: messages::PurgeGroupKeysRequest, _s: DeviceSender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::delete_keys_of_group(&mut *tx, msg.group_id).await.map_err(db("delete keys"))?;
        repo::delete_requests_of_group(&mut *tx, msg.group_id).await.map_err(db("delete requests"))?;
        tx.commit().await.map_err(db("commit"))?;
        let remaining = repo::count_keys_of_group(&*self.pool, msg.group_id).await.map_err(db("count keys"))?
            + repo::count_requests_of_group(&*self.pool, msg.group_id).await.map_err(db("count requests"))?;
        Ok(messages::PurgeResult { remaining })
    }

    async fn handle_purge_user_devices_request(&self, ctx: &Ctx, msg: messages::PurgeUserDevicesRequest, s: DeviceSender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        let devices = repo::devices_of_user(&*self.pool, msg.user_id).await.map_err(db("devices of user"))?;
        let ids: Vec<Uuid> = devices.iter().map(|d| d.id).collect();
        let mut left = Vec::new();
        for id in &ids {
            for group_id in repo::groups_held_by_device(&*self.pool, *id).await.map_err(db("groups held"))? {
                left.push(DeviceLeftGroup { group_id, device_id: *id });
            }
        }
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::delete_keys_of_devices(&mut *tx, &ids).await.map_err(db("delete keys"))?;
        repo::delete_requests_of_devices(&mut *tx, &ids).await.map_err(db("delete requests"))?;
        repo::delete_devices(&mut *tx, &ids).await.map_err(db("delete devices"))?;
        tx.commit().await.map_err(db("commit"))?;
        for event in left {
            s.send_device_left_group(ctx, event).await;
        }
        let remaining = repo::count_devices_of_user(&*self.pool, msg.user_id).await.map_err(db("count devices"))?;
        Ok(messages::PurgeResult { remaining })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_publisher_per_key_id() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert!(publisher_allowed(None, a).is_ok(), "the first publisher");
        assert!(publisher_allowed(Some(a), a).is_ok(), "the same device again");
        assert_eq!(publisher_allowed(Some(a), b).unwrap_err().code(), Code::FailedPrecondition);
    }

    #[test]
    fn approval_needs_a_writer_unless_the_device_is_ones_own() {
        assert!(approver_allowed(true, None).is_ok(), "one's own device, membership aside");
        assert!(approver_allowed(false, Some(Role::Admin)).is_ok());
        assert!(approver_allowed(false, Some(Role::Member)).is_ok());
        assert_eq!(approver_allowed(false, Some(Role::Viewer)).unwrap_err().code(), Code::PermissionDenied);
        assert_eq!(approver_allowed(false, None).unwrap_err().code(), Code::PermissionDenied);
    }
}
