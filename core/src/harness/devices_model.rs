//! `device` in memory: the handlers of `nanoservices/device/src/handlers.rs`
//! rule for rule over maps. Membership is a table the test fills; the
//! wake-ups the real one pushes are queued per device.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use uuid::Uuid;

use crate::api::{ApiError, Code};
use crate::harness::Role;
use crate::keys::{DeviceInfo, Devices, KeyRequest, WrapFor, WrappedKey};
use crate::rng::CoreRng;

/// What `device` would push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceWakeup {
    /// A device asked for the group's keys; this holder may approve.
    KeyRequest {
        /// The group.
        group_id: Uuid,
        /// The request.
        request_id: Uuid,
    },
    /// This device's request was fulfilled: its keys are there.
    Keys {
        /// The group.
        group_id: Uuid,
    },
    /// A member was removed: a holder rotates.
    Rotate {
        /// The group.
        group_id: Uuid,
    },
}

#[derive(Debug, Clone)]
struct DeviceRow {
    user_id: Uuid,
    public_key: Vec<u8>,
    platform: String,
}

#[derive(Debug, Clone)]
struct KeyRow {
    id: Uuid,
    group_id: Uuid,
    key_id: u32,
    device_id: Uuid,
    wrapped: Vec<u8>,
    wrapped_by_device: Uuid,
    order: u64,
}

#[derive(Debug, Clone)]
struct RequestRow {
    group_id: Uuid,
    device_id: Uuid,
    invite_id: Option<Uuid>,
    invite_tag: Vec<u8>,
    created_at_ms: i64,
}

#[derive(Default)]
struct Inner {
    now_ms: i64,
    next_order: u64,
    devices: BTreeMap<Uuid, DeviceRow>,
    keys: Vec<KeyRow>,
    requests: BTreeMap<Uuid, RequestRow>,
    members: BTreeMap<(Uuid, Uuid), Role>,
    /// The session's user per device, what the real server has from Kratos.
    sessions: BTreeMap<Uuid, Uuid>,
    wakeups: BTreeMap<Uuid, VecDeque<DeviceWakeup>>,
    outage: bool,
}

/// The model.
pub struct DevicesModel {
    inner: RefCell<Inner>,
    rng: RefCell<CoreRng>,
}

fn err(code: Code, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

impl Inner {
    fn caller(&self, device_id: Uuid) -> Result<Uuid, ApiError> {
        self.sessions.get(&device_id).copied().ok_or_else(|| err(Code::PermissionDenied, "no session for this device"))
    }

    /// The calling device, which must be the caller's.
    fn own_device(&self, device_id: Uuid) -> Result<(Uuid, DeviceRow), ApiError> {
        let user = self.caller(device_id)?;
        let d = self.devices.get(&device_id).ok_or_else(|| err(Code::NotFound, "device not registered"))?;
        if d.user_id != user {
            return Err(err(Code::PermissionDenied, "not the caller's device"));
        }
        Ok((user, d.clone()))
    }

    fn role(&self, user_id: Uuid, group_id: Uuid) -> Option<Role> {
        self.members.get(&(group_id, user_id)).copied()
    }

    fn holds(&self, group_id: Uuid, device_id: Uuid) -> bool {
        self.keys.iter().any(|k| k.group_id == group_id && k.device_id == device_id)
    }

    fn holds_key(&self, group_id: Uuid, key_id: u32, device_id: Uuid) -> bool {
        self.keys.iter().any(|k| k.group_id == group_id && k.key_id == key_id && k.device_id == device_id)
    }

    fn wake(&mut self, device_id: Uuid, wakeup: DeviceWakeup) {
        self.wakeups.entry(device_id).or_default().push_back(wakeup);
    }

    fn holder_devices(&self, group_id: Uuid) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self.keys.iter().filter(|k| k.group_id == group_id).map(|k| k.device_id).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    fn info(&self, id: Uuid, d: &DeviceRow) -> DeviceInfo {
        DeviceInfo { id, user_id: d.user_id, public_key: d.public_key.clone(), platform: d.platform.clone() }
    }

    fn request_msg(&self, id: Uuid, r: &RequestRow) -> Result<KeyRequest, ApiError> {
        let d = self.devices.get(&r.device_id).ok_or_else(|| err(Code::NotFound, "the requesting device is gone"))?;
        Ok(KeyRequest { id, group_id: r.group_id, device: self.info(r.device_id, d), invite_id: r.invite_id, invite_tag: r.invite_tag.clone(), created_at_ms: r.created_at_ms })
    }

    fn key_msg(&self, k: &KeyRow) -> WrappedKey {
        WrappedKey {
            id: k.id,
            group_id: k.group_id,
            key_id: k.key_id,
            device_id: k.device_id,
            wrapped: k.wrapped.clone(),
            wrapped_by_device: k.wrapped_by_device,
            wrapped_by_public_key: self.devices.get(&k.wrapped_by_device).map(|d| d.public_key.clone()).unwrap_or_default(),
        }
    }

    fn insert_key(&mut self, group_id: Uuid, key_id: u32, device_id: Uuid, wrapped: Vec<u8>, by: Uuid, id: Uuid) {
        // ON CONFLICT (group_id, key_id, device_id) DO NOTHING.
        if self.holds_key(group_id, key_id, device_id) {
            return;
        }
        let order = self.next_order;
        self.next_order += 1;
        self.keys.push(KeyRow { id, group_id, key_id, device_id, wrapped, wrapped_by_device: by, order });
    }
}

impl DevicesModel {
    /// A model with nothing in it.
    pub fn new(seed: [u8; 32]) -> DevicesModel {
        DevicesModel { inner: RefCell::new(Inner::default()), rng: RefCell::new(CoreRng::from_seed(seed)) }
    }

    /// The session behind a device: who the server takes the caller to be.
    pub fn session(&self, device_id: Uuid, user_id: Uuid) {
        self.inner.borrow_mut().sessions.insert(device_id, user_id);
    }

    /// Makes a user a member of a group with a role.
    pub fn add_member(&self, group_id: Uuid, user_id: Uuid, role: Role) {
        self.inner.borrow_mut().members.insert((group_id, user_id), role);
    }

    /// `MemberRemoved`: the user's devices lose their keys and requests
    /// for the group; the remaining holders are told to rotate.
    pub fn remove_member(&self, group_id: Uuid, user_id: Uuid) {
        let mut inner = self.inner.borrow_mut();
        inner.members.remove(&(group_id, user_id));
        let gone: Vec<Uuid> = inner.devices.iter().filter(|(_, d)| d.user_id == user_id).map(|(id, _)| *id).collect();
        inner.keys.retain(|k| !(k.group_id == group_id && gone.contains(&k.device_id)));
        inner.requests.retain(|_, r| !(r.group_id == group_id && gone.contains(&r.device_id)));
        for d in inner.holder_devices(group_id) {
            inner.wake(d, DeviceWakeup::Rotate { group_id });
        }
    }

    /// `RemoveDevice`: the device, its keys and its requests go.
    pub fn remove_device(&self, device_id: Uuid) {
        let mut inner = self.inner.borrow_mut();
        inner.devices.remove(&device_id);
        inner.keys.retain(|k| k.device_id != device_id);
        inner.requests.retain(|_, r| r.device_id != device_id);
    }

    /// A test's hostile server: reports another public key for a device.
    pub fn swap_public_key(&self, device_id: Uuid, public_key: [u8; 32]) {
        if let Some(d) = self.inner.borrow_mut().devices.get_mut(&device_id) {
            d.public_key = public_key.to_vec();
        }
    }

    /// Whether calls fail with `Unavailable`.
    pub fn set_outage(&self, outage: bool) {
        self.inner.borrow_mut().outage = outage;
    }

    /// The wake-ups queued for a device, taken.
    pub fn take_wakeups(&self, device_id: Uuid) -> Vec<DeviceWakeup> {
        self.inner.borrow_mut().wakeups.remove(&device_id).map(Vec::from).unwrap_or_default()
    }

    /// The devices holding a key of the group, as the server counts them.
    pub fn holders(&self, group_id: Uuid) -> Vec<Uuid> {
        self.inner.borrow().holder_devices(group_id)
    }

    /// The open requests of a group.
    pub fn open_requests(&self, group_id: Uuid) -> usize {
        self.inner.borrow().requests.values().filter(|r| r.group_id == group_id).count()
    }

    fn call(&self) -> Result<std::cell::RefMut<'_, Inner>, ApiError> {
        let inner = self.inner.borrow_mut();
        if inner.outage {
            return Err(err(Code::Unavailable, "the network is down"));
        }
        Ok(inner)
    }
}

impl Devices for DevicesModel {
    async fn register_device(&self, device_id: Uuid, public_key: [u8; 32], platform: &str) -> Result<(), ApiError> {
        let mut inner = self.call()?;
        let user = inner.caller(device_id)?;
        if let Some(d) = inner.devices.get(&device_id) {
            if d.user_id != user {
                return Err(err(Code::PermissionDenied, "not the caller's device"));
            }
            if d.public_key != public_key {
                return Err(err(Code::FailedPrecondition, "a device's public key never changes"));
            }
        }
        inner.devices.insert(device_id, DeviceRow { user_id: user, public_key: public_key.to_vec(), platform: platform.to_owned() });
        Ok(())
    }

    async fn list_devices(&self, device_id: Uuid, group_id: Uuid) -> Result<Vec<DeviceInfo>, ApiError> {
        let inner = self.call()?;
        inner.own_device(device_id)?;
        if !inner.holds(group_id, device_id) {
            return Err(err(Code::PermissionDenied, "the calling device holds no key for this group"));
        }
        Ok(inner.holder_devices(group_id).into_iter().filter_map(|id| inner.devices.get(&id).map(|d| inner.info(id, d))).collect())
    }

    async fn request_group_keys(&self, device_id: Uuid, group_id: Uuid, invite_id: Option<Uuid>, invite_tag: Option<[u8; 32]>) -> Result<KeyRequest, ApiError> {
        let mut inner = self.call()?;
        let (user, _) = inner.own_device(device_id)?;
        if inner.role(user, group_id).is_none() {
            return Err(err(Code::PermissionDenied, "not a member of this group"));
        }
        if inner.holds(group_id, device_id) {
            return Err(err(Code::FailedPrecondition, "the device already holds a key for this group"));
        }
        if let Some((id, r)) = inner.requests.iter().find(|(_, r)| r.group_id == group_id && r.device_id == device_id) {
            let (id, r) = (*id, r.clone());
            return inner.request_msg(id, &r);
        }
        let id = crate::ids::random(&mut self.rng.borrow_mut());
        let row = RequestRow { group_id, device_id, invite_id, invite_tag: invite_tag.map(|t| t.to_vec()).unwrap_or_default(), created_at_ms: inner.now_ms };
        let msg = inner.request_msg(id, &row)?;
        inner.requests.insert(id, row);
        for d in inner.holder_devices(group_id) {
            inner.wake(d, DeviceWakeup::KeyRequest { group_id, request_id: id });
        }
        Ok(msg)
    }

    async fn list_key_requests(&self, device_id: Uuid) -> Result<Vec<KeyRequest>, ApiError> {
        let inner = self.call()?;
        inner.own_device(device_id)?;
        let mut out = Vec::new();
        for (id, r) in &inner.requests {
            if inner.holds(r.group_id, device_id)
                && let Ok(msg) = inner.request_msg(*id, r)
            {
                out.push(msg);
            }
        }
        Ok(out)
    }

    async fn fulfil_key_request(&self, device_id: Uuid, request_id: Uuid, keys: Vec<(u32, Vec<u8>)>) -> Result<(), ApiError> {
        let mut inner = self.call()?;
        let (user, _) = inner.own_device(device_id)?;
        if keys.is_empty() {
            return Err(err(Code::InvalidArgument, "keys: a fulfilment hands over at least one wrapped key"));
        }
        let mut ids: Vec<u32> = keys.iter().map(|k| k.0).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != keys.len() || ids.first().is_some_and(|k| *k < 1) || keys.iter().any(|k| k.1.is_empty()) {
            return Err(err(Code::InvalidArgument, "keys: one non-empty wrap per positive key id"));
        }
        let request = inner.requests.get(&request_id).cloned().ok_or_else(|| err(Code::NotFound, "key request not found"))?;
        if !inner.holds(request.group_id, device_id) {
            return Err(err(Code::PermissionDenied, "the calling device holds no key for this group"));
        }
        let Some(requester) = inner.devices.get(&request.device_id).cloned() else {
            inner.requests.remove(&request_id);
            return Err(err(Code::NotFound, "the requesting device is gone"));
        };
        let own = requester.user_id == user;
        if !own && !matches!(inner.role(user, request.group_id), Some(Role::Admin | Role::Member)) {
            return Err(err(Code::PermissionDenied, "a viewer approves no other user"));
        }
        if !own && inner.role(requester.user_id, request.group_id).is_none() {
            inner.requests.remove(&request_id);
            return Err(err(Code::FailedPrecondition, "the requesting device's user is no longer a member"));
        }
        for (key_id, wrapped) in keys {
            let id = crate::ids::random(&mut self.rng.borrow_mut());
            inner.insert_key(request.group_id, key_id, request.device_id, wrapped, device_id, id);
        }
        inner.requests.remove(&request_id);
        inner.wake(request.device_id, DeviceWakeup::Keys { group_id: request.group_id });
        Ok(())
    }

    async fn publish_wrapped_keys(&self, device_id: Uuid, group_id: Uuid, key_id: u32, wraps: Vec<WrapFor>) -> Result<Vec<WrappedKey>, ApiError> {
        let mut inner = self.call()?;
        let (user, _) = inner.own_device(device_id)?;
        if key_id < 1 {
            return Err(err(Code::InvalidArgument, "key_id: key ids start at 1"));
        }
        if wraps.is_empty() || wraps.iter().any(|w| w.wrapped.is_empty()) {
            return Err(err(Code::InvalidArgument, "wraps: one non-empty wrap per device"));
        }
        let mut targets: Vec<Uuid> = wraps.iter().map(|w| w.device_id).collect();
        targets.sort_unstable();
        targets.dedup();
        if targets.len() != wraps.len() {
            return Err(err(Code::InvalidArgument, "wraps: a device at most once"));
        }
        if inner.role(user, group_id).is_none() {
            return Err(err(Code::PermissionDenied, "not a member of this group"));
        }
        if key_id > 1 && !inner.holds_key(group_id, key_id - 1, device_id) {
            return Err(err(Code::FailedPrecondition, "the publishing device does not hold the previous key"));
        }
        let existing = inner.keys.iter().filter(|k| k.group_id == group_id && k.key_id == key_id).min_by_key(|k| k.order).map(|k| k.wrapped_by_device);
        if existing.is_some_and(|other| other != device_id) {
            return Err(err(Code::FailedPrecondition, "this key id was already published by another device; adopt it"));
        }
        for w in &wraps {
            let Some(d) = inner.devices.get(&w.device_id).cloned() else { continue };
            if key_id > 1 && !inner.holds_key(group_id, key_id - 1, d_id(w)) {
                continue;
            }
            if inner.role(d.user_id, group_id).is_none() {
                continue;
            }
            let id = crate::ids::random(&mut self.rng.borrow_mut());
            inner.insert_key(group_id, key_id, w.device_id, w.wrapped.clone(), device_id, id);
        }
        let landed: Vec<WrappedKey> = inner.keys.iter().filter(|k| k.group_id == group_id && k.key_id == key_id && targets.contains(&k.device_id)).map(|k| inner.key_msg(k)).collect();
        Ok(landed)
    }

    async fn get_wrapped_keys(&self, device_id: Uuid, group_id: Option<Uuid>) -> Result<Vec<WrappedKey>, ApiError> {
        let inner = self.call()?;
        inner.own_device(device_id)?;
        let mut rows: Vec<&KeyRow> = inner.keys.iter().filter(|k| k.device_id == device_id && group_id.is_none_or(|g| g == k.group_id)).collect();
        rows.sort_by_key(|k| (k.group_id, k.key_id, k.order));
        Ok(rows.into_iter().map(|k| inner.key_msg(k)).collect())
    }
}

fn d_id(w: &WrapFor) -> Uuid {
    w.device_id
}
