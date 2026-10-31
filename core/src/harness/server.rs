//! The whole server in memory behind the [`Transport`] seam: Connect over
//! the relay and device models and a group model, the session from the
//! token header, Kratos's `whoami`, and a bucket for the presigned URLs.
//! What the facade's tests and the shells' end-to-end tests drive
//! (`docs/01-deappje/03-testing.md`), so a shell signs in, pairs, pushes
//! an object and sees it rendered without a server.

use std::collections::BTreeMap;
use std::sync::Mutex;

use uuid::Uuid;

use crate::api::{ApiError, Code};
use crate::harness::{DevicesModel, RelayModel, Role};
use crate::keys::Devices;
use crate::proto;
use crate::relay::Relay;
use crate::schema::Message;
use crate::transport::{Header, Request, Response, Transport, TransportError};

#[derive(Debug, Clone)]
struct GroupRow {
    parent: Option<Uuid>,
    created_by: Uuid,
    created_at_ms: i64,
}

#[derive(Debug, Clone)]
struct InviteRow {
    group_id: Uuid,
    code: String,
    role: Role,
    created_by: Uuid,
    expires_at_ms: i64,
    max_uses: i32,
    uses: i32,
}

#[derive(Default)]
struct Groups {
    groups: BTreeMap<Uuid, GroupRow>,
    members: BTreeMap<(Uuid, Uuid), (Uuid, Role, i64)>,
    invites: BTreeMap<Uuid, InviteRow>,
    next_code: u64,
    next_group: u64,
}

/// The server. Off the browser also a UniFFI object that is a
/// [`Transport`], so a Swift or Kotlin test opens a
/// [`crate::facade::Core`] over it and drives a whole group without a
/// network or a sign-in ([`ModelServer::open`]).
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct ModelServer {
    /// The relay.
    pub relay: RelayModel,
    /// The device service.
    pub devices: DevicesModel,
    groups: Mutex<Groups>,
    sessions: Mutex<BTreeMap<String, Uuid>>,
    bucket: Mutex<BTreeMap<String, Vec<u8>>>,
    base_url: String,
}

fn err(code: Code, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

fn role_name(r: Role) -> &'static str {
    match r {
        Role::Admin => "admin",
        Role::Member => "member",
        Role::Viewer => "viewer",
    }
}

fn role_of(name: &str) -> Result<Role, ApiError> {
    match name {
        "admin" => Ok(Role::Admin),
        "member" => Ok(Role::Member),
        "viewer" => Ok(Role::Viewer),
        other => Err(err(Code::InvalidArgument, format!("role: {other:?}"))),
    }
}

fn uuid(s: &str, what: &str) -> Result<Uuid, ApiError> {
    s.parse().map_err(|_| err(Code::InvalidArgument, format!("{what}: not an id")))
}

fn uuid_opt(s: &str, what: &str) -> Result<Option<Uuid>, ApiError> {
    if s.is_empty() { Ok(None) } else { uuid(s, what).map(Some) }
}

impl ModelServer {
    /// A server at the origin, empty.
    pub fn new(base_url: &str, seed: [u8; 32]) -> ModelServer {
        ModelServer {
            relay: RelayModel::new(seed),
            devices: DevicesModel::new(seed),
            groups: Mutex::new(Groups::default()),
            sessions: Mutex::new(BTreeMap::new()),
            bucket: Mutex::new(BTreeMap::new()),
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// A session token that signs in as the user.
    pub fn session(&self, token: &str, user_id: Uuid) {
        self.sessions.lock().unwrap().insert(token.to_owned(), user_id);
    }

    /// Whether a blob is in the bucket.
    pub fn bucket_has(&self, key: &str) -> bool {
        self.bucket.lock().unwrap().contains_key(key)
    }
}

/// The server as the shells' tests hold it, through the bindings.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl ModelServer {
    /// A server at the origin, empty, deterministic from the seed (the
    /// first 32 bytes; shorter is padded with zeros).
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)]
    pub fn open(base_url: String, seed: Vec<u8>) -> std::sync::Arc<ModelServer> {
        let mut bytes = [0u8; 32];
        for (slot, b) in bytes.iter_mut().zip(seed.iter()) {
            *slot = *b;
        }
        std::sync::Arc::new(ModelServer::new(&base_url, bytes))
    }

    /// A session token that signs in as the user: what a test hands to
    /// `Core::set_session_token` instead of a login.
    pub fn sign_in(&self, token: String, user_id: Uuid) {
        self.session(&token, user_id);
    }

    /// Whether a blob is in the bucket.
    pub fn has_blob(&self, key: String) -> bool {
        self.bucket_has(&key)
    }

    /// Moves the relay's clock forward.
    pub fn advance(&self, ms: i64) {
        self.relay.advance(ms);
    }

    /// Runs the relay's hourly sweeps once: eviction and request expiry.
    pub fn sweep(&self) {
        self.relay.evict();
        self.relay.expire_requests();
    }

    /// The network is down (every call fails) or back.
    pub fn set_outage(&self, outage: bool) {
        self.relay.set_outage(outage);
        self.devices.set_outage(outage);
    }
}

impl ModelServer {

    /// The models all learn of a membership.
    fn set_member(&self, group_id: Uuid, user_id: Uuid, role: Role, now: i64) {
        self.groups.lock().unwrap().members.insert((group_id, user_id), (Uuid::new_v5(&group_id, user_id.as_bytes()), role, now));
        self.relay.add_member(group_id, user_id, role);
        self.devices.add_member(group_id, user_id, role);
    }

    fn drop_member(&self, group_id: Uuid, user_id: Uuid) {
        self.groups.lock().unwrap().members.remove(&(group_id, user_id));
        self.relay.remove_member(group_id, user_id);
        self.devices.remove_member(group_id, user_id);
    }

    fn membership(&self, group_id: Uuid, user_id: Uuid) -> Option<proto::Membership> {
        let g = self.groups.lock().unwrap();
        g.members.get(&(group_id, user_id)).map(|(id, role, joined)| proto::Membership { id: id.to_string(), group_id: group_id.to_string(), user_id: user_id.to_string(), role: role_name(*role).to_owned(), joined_at_ms: *joined })
    }

    fn require_role(&self, group_id: Uuid, user_id: Uuid) -> Result<Role, ApiError> {
        self.groups.lock().unwrap().members.get(&(group_id, user_id)).map(|m| m.1).ok_or_else(|| err(Code::PermissionDenied, "not a member of this group"))
    }

    fn group_msg(&self, id: Uuid, g: &GroupRow) -> proto::Group {
        proto::Group { id: id.to_string(), parent_group_id: g.parent.map(|p| p.to_string()).unwrap_or_default(), created_by: g.created_by.to_string(), created_at_ms: g.created_at_ms, deleted_at_ms: 0 }
    }

    fn invite_msg(&self, id: Uuid, i: &InviteRow) -> proto::Invite {
        proto::Invite { id: id.to_string(), group_id: i.group_id.to_string(), code: i.code.clone(), role: role_name(i.role).to_owned(), created_by: i.created_by.to_string(), expires_at_ms: i.expires_at_ms, max_uses: i.max_uses, uses: i.uses, revoked: false }
    }

    // ---- group.v1 ----

    async fn group_call(&self, user: Uuid, method: &str, body: &[u8]) -> Result<Vec<u8>, ApiError> {
        let now = self.relay.now_ms();
        let decode = |e: crate::schema::SchemaError| err(Code::InvalidArgument, e.to_string());
        Ok(match method {
            "CreateGroup" => {
                let r = proto::CreateGroupRequest::decode(body).map_err(decode)?;
                let parent = uuid_opt(&r.parent_group_id, "parent_group_id")?;
                if let Some(p) = parent {
                    self.require_role(p, user)?;
                }
                let id = {
                    let mut g = self.groups.lock().unwrap();
                    g.next_group += 1;
                    let id = Uuid::new_v5(&user, &g.next_group.to_be_bytes());
                    g.groups.insert(id, GroupRow { parent, created_by: user, created_at_ms: now });
                    id
                };
                self.set_member(id, user, Role::Admin, now);
                let g = self.groups.lock().unwrap().groups[&id].clone();
                proto::CreateGroupResponse { group: Some(self.group_msg(id, &g)) }.encode()
            }
            "ListGroups" => {
                let g = self.groups.lock().unwrap();
                let groups = g.groups.iter().filter(|(id, _)| g.members.contains_key(&(**id, user))).map(|(id, row)| self.group_msg(*id, row)).collect();
                proto::ListGroupsResponse { groups }.encode()
            }
            "GetGroup" => {
                let r = proto::GetGroupRequest::decode(body).map_err(decode)?;
                let id = uuid(&r.group_id, "group_id")?;
                self.require_role(id, user)?;
                let g = self.groups.lock().unwrap().groups.get(&id).cloned().ok_or_else(|| err(Code::NotFound, "group not found"))?;
                proto::GetGroupResponse { group: Some(self.group_msg(id, &g)) }.encode()
            }
            "CreateInvite" => {
                let r = proto::CreateInviteRequest::decode(body).map_err(decode)?;
                let group_id = uuid(&r.group_id, "group_id")?;
                if self.require_role(group_id, user)? != Role::Admin {
                    return Err(err(Code::PermissionDenied, "not an admin"));
                }
                let role = role_of(&r.role)?;
                let mut g = self.groups.lock().unwrap();
                g.next_code += 1;
                let code = format!("c{:09}", g.next_code).replace('0', "a").replace('1', "b");
                let id = Uuid::new_v5(&group_id, code.as_bytes());
                let row = InviteRow { group_id, code, role, created_by: user, expires_at_ms: r.expires_at_ms, max_uses: r.max_uses, uses: 0 };
                let msg = self.invite_msg(id, &row);
                g.invites.insert(id, row);
                proto::CreateInviteResponse { invite: Some(msg) }.encode()
            }
            "AcceptInvite" => {
                let r = proto::AcceptInviteRequest::decode(body).map_err(decode)?;
                let (id, invite) = {
                    let g = self.groups.lock().unwrap();
                    g.invites.iter().find(|(_, i)| i.code == r.code).map(|(id, i)| (*id, i.clone())).ok_or_else(|| err(Code::NotFound, "invite not found"))?
                };
                if self.membership(invite.group_id, user).is_none() {
                    if invite.expires_at_ms != 0 && invite.expires_at_ms <= now {
                        return Err(err(Code::FailedPrecondition, "the invite expired"));
                    }
                    if invite.max_uses != 0 && invite.uses >= invite.max_uses {
                        return Err(err(Code::FailedPrecondition, "the invite is used up"));
                    }
                    self.set_member(invite.group_id, user, invite.role, now);
                    self.groups.lock().unwrap().invites.get_mut(&id).expect("read above").uses += 1;
                }
                let invite = self.groups.lock().unwrap().invites[&id].clone();
                proto::AcceptInviteResponse { membership: self.membership(invite.group_id, user), invite: Some(self.invite_msg(id, &invite)) }.encode()
            }
            "ListMembers" => {
                let r = proto::ListMembersRequest::decode(body).map_err(decode)?;
                let group_id = uuid(&r.group_id, "group_id")?;
                self.require_role(group_id, user)?;
                let g = self.groups.lock().unwrap();
                let mut members: Vec<(i64, proto::Membership)> = g
                    .members
                    .iter()
                    .filter(|((gid, _), _)| *gid == group_id)
                    .map(|((_, uid), (id, role, joined))| (*joined, proto::Membership { id: id.to_string(), group_id: group_id.to_string(), user_id: uid.to_string(), role: role_name(*role).to_owned(), joined_at_ms: *joined }))
                    .collect();
                members.sort_by_key(|(j, _)| *j);
                proto::ListMembersResponse { members: members.into_iter().map(|(_, m)| m).collect() }.encode()
            }
            "RemoveMember" => {
                let r = proto::RemoveMemberRequest::decode(body).map_err(decode)?;
                let (group_id, target) = (uuid(&r.group_id, "group_id")?, uuid(&r.user_id, "user_id")?);
                if self.require_role(group_id, user)? != Role::Admin {
                    return Err(err(Code::PermissionDenied, "not an admin"));
                }
                let removed = self.membership(group_id, target).ok_or_else(|| err(Code::NotFound, "not a member"))?;
                self.drop_member(group_id, target);
                proto::RemoveMemberResponse { membership: Some(removed) }.encode()
            }
            "SetMemberRole" => {
                let r = proto::SetMemberRoleRequest::decode(body).map_err(decode)?;
                let (group_id, target) = (uuid(&r.group_id, "group_id")?, uuid(&r.user_id, "user_id")?);
                if self.require_role(group_id, user)? != Role::Admin {
                    return Err(err(Code::PermissionDenied, "not an admin"));
                }
                let role = role_of(&r.role)?;
                let joined = self.membership(group_id, target).ok_or_else(|| err(Code::NotFound, "not a member"))?.joined_at_ms;
                self.set_member(group_id, target, role, joined);
                proto::SetMemberRoleResponse { membership: self.membership(group_id, target) }.encode()
            }
            "LeaveGroup" => {
                let r = proto::LeaveGroupRequest::decode(body).map_err(decode)?;
                let group_id = uuid(&r.group_id, "group_id")?;
                let gone = self.membership(group_id, user).ok_or_else(|| err(Code::PermissionDenied, "not a member of this group"))?;
                self.drop_member(group_id, user);
                proto::LeaveGroupResponse { membership: Some(gone) }.encode()
            }
            other => return Err(err(Code::NotFound, format!("group.v1.GroupService/{other}"))),
        })
    }

    // ---- relay.v1 ----

    async fn relay_call(&self, method: &str, body: &[u8]) -> Result<Vec<u8>, ApiError> {
        let decode = |e: crate::schema::SchemaError| err(Code::InvalidArgument, e.to_string());
        let sv = "relay.v1.RelayService";
        Ok(match method {
            "PushObjects" => {
                let r = proto::PushObjectsRequest::decode(body).map_err(decode)?;
                let objects = r.objects.into_iter().map(|o| Ok(crate::relay::PushEntry { id: uuid(&o.id, "id")?, value: if o.deleted || o.value.is_empty() { None } else { Some(o.value) }, base_seq: o.base_seq, deleted: o.deleted })).collect::<Result<Vec<_>, ApiError>>()?;
                match self.relay.push_objects(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?, objects).await? {
                    crate::relay::PushResult::BehindFloor { evicted_through_seq } => proto::PushObjectsResponse { behind_floor: true, evicted_through_seq, ..Default::default() }.encode(),
                    crate::relay::PushResult::Accepted { accepted, conflicts, head_seq } => proto::PushObjectsResponse {
                        accepted: accepted.into_iter().map(|a| proto::Accepted { id: a.id.to_string(), seq: a.seq }).collect(),
                        conflicts: conflicts.into_iter().map(|c| proto::Conflict { id: c.id.to_string(), current: Some(entry_to(c.current)) }).collect(),
                        head_seq,
                        ..Default::default()
                    }
                    .encode(),
                }
            }
            "PullChanges" => {
                let r = proto::PullChangesRequest::decode(body).map_err(decode)?;
                let c = self.relay.pull_changes(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?, r.since_seq, r.limit).await?;
                proto::PullChangesResponse { entries: c.entries.into_iter().map(entry_to).collect(), head_seq: c.head_seq, evicted_through_seq: c.evicted_through_seq, min_acked_seq: c.min_acked_seq.unwrap_or(0), has_min_acked_seq: c.min_acked_seq.is_some(), more: c.more }.encode()
            }
            "AckSeq" => {
                let r = proto::AckSeqRequest::decode(body).map_err(decode)?;
                let a = self.relay.ack_seq(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?, r.seq).await?;
                proto::AckSeqResponse { evicted_through_seq: a.evicted_through_seq, min_acked_seq: a.min_acked_seq.unwrap_or(0), has_min_acked_seq: a.min_acked_seq.is_some() }.encode()
            }
            "GetHeads" => {
                let r = proto::GetHeadsRequest::decode(body).map_err(decode)?;
                let ids = r.group_ids.iter().map(|g| uuid(g, "group_ids")).collect::<Result<Vec<_>, _>>()?;
                let heads = self.relay.get_heads(uuid(&r.device_id, "device_id")?, ids).await?;
                proto::GetHeadsResponse { heads: heads.into_iter().map(|h| proto::Head { group_id: h.group_id.to_string(), head_seq: h.head_seq, evicted_through_seq: h.evicted_through_seq, min_acked_seq: h.min_acked_seq.unwrap_or(0), has_min_acked_seq: h.min_acked_seq.is_some() }).collect() }.encode()
            }
            "RequestHistory" => {
                let r = proto::RequestHistoryRequest::decode(body).map_err(decode)?;
                let h = self.relay.request_history(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?, r.from_seq, r.to_seq, uuid_opt(&r.object_id, "object_id")?).await?;
                proto::RequestHistoryResponse { request: Some(history_to(h)) }.encode()
            }
            "GetHistoryRequest" => {
                let r = proto::GetHistoryRequestRequest::decode(body).map_err(decode)?;
                let h = self.relay.get_history_request(uuid(&r.device_id, "device_id")?, uuid(&r.request_id, "request_id")?).await?;
                proto::GetHistoryRequestResponse { request: Some(history_to(h)) }.encode()
            }
            "ListHistoryRequests" => {
                let r = proto::ListHistoryRequestsRequest::decode(body).map_err(decode)?;
                let list = self.relay.list_history_requests(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?).await?;
                proto::ListHistoryRequestsResponse { requests: list.into_iter().map(history_to).collect() }.encode()
            }
            "ClaimHistory" => {
                let r = proto::ClaimHistoryRequest::decode(body).map_err(decode)?;
                let h = self.relay.claim_history(uuid(&r.device_id, "device_id")?, uuid(&r.request_id, "request_id")?).await?;
                proto::ClaimHistoryResponse { request: Some(history_to(h)) }.encode()
            }
            "PushHistory" => {
                let r = proto::PushHistoryRequest::decode(body).map_err(decode)?;
                let entries = r.entries.into_iter().map(|e| Ok(crate::relay::Entry { id: uuid(&e.id, "id")?, value: if e.deleted || e.value.is_empty() { None } else { Some(e.value) }, seq: e.seq, deleted: e.deleted })).collect::<Result<Vec<_>, ApiError>>()?;
                let p = self.relay.push_history(uuid(&r.device_id, "device_id")?, uuid(&r.request_id, "request_id")?, entries, r.complete).await?;
                proto::PushHistoryResponse { inserted: p.inserted, skipped: p.skipped, through_seq: p.through_seq, complete: p.complete }.encode()
            }
            other => return Err(err(Code::NotFound, format!("{sv}/{other}"))),
        })
    }

    // ---- device.v1 ----

    async fn device_call(&self, user: Uuid, method: &str, body: &[u8]) -> Result<Vec<u8>, ApiError> {
        let decode = |e: crate::schema::SchemaError| err(Code::InvalidArgument, e.to_string());
        Ok(match method {
            "RegisterDevice" => {
                let r = proto::RegisterDeviceRequest::decode(body).map_err(decode)?;
                let device_id = uuid(&r.device_id, "device_id")?;
                let pk: [u8; 32] = r.public_key.as_slice().try_into().map_err(|_| err(Code::InvalidArgument, "public_key: 32 bytes"))?;
                self.devices.session(device_id, user);
                self.relay.register_device(device_id, user);
                self.devices.register_device(device_id, pk, &r.platform).await?;
                proto::RegisterDeviceResponse { device: Some(proto::Device { id: r.device_id, user_id: user.to_string(), public_key: r.public_key, platform: r.platform, ..Default::default() }) }.encode()
            }
            "SetPushToken" => {
                let r = proto::SetPushTokenRequest::decode(body).map_err(decode)?;
                proto::SetPushTokenResponse { device: Some(proto::Device { id: r.device_id, user_id: user.to_string(), push_provider: r.push_provider, push_token: r.push_token, ..Default::default() }) }.encode()
            }
            "ListDevices" => {
                let r = proto::ListDevicesRequest::decode(body).map_err(decode)?;
                let list = self.devices.list_devices(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?).await?;
                proto::ListDevicesResponse { devices: list.into_iter().map(device_info_to).collect() }.encode()
            }
            "RequestGroupKeys" => {
                let r = proto::RequestGroupKeysRequest::decode(body).map_err(decode)?;
                let tag = if r.invite_tag.is_empty() { None } else { Some(r.invite_tag.as_slice().try_into().map_err(|_| err(Code::InvalidArgument, "invite_tag: 32 bytes"))?) };
                let k = self.devices.request_group_keys(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?, uuid_opt(&r.invite_id, "invite_id")?, tag).await?;
                proto::RequestGroupKeysResponse { request: Some(key_request_to(k)) }.encode()
            }
            "ListKeyRequests" => {
                let r = proto::ListKeyRequestsRequest::decode(body).map_err(decode)?;
                let list = self.devices.list_key_requests(uuid(&r.device_id, "device_id")?).await?;
                proto::ListKeyRequestsResponse { requests: list.into_iter().map(key_request_to).collect() }.encode()
            }
            "FulfilKeyRequest" => {
                let r = proto::FulfilKeyRequestRequest::decode(body).map_err(decode)?;
                let keys = r.keys.into_iter().map(|k| (k.key_id as u32, k.wrapped)).collect();
                self.devices.fulfil_key_request(uuid(&r.device_id, "device_id")?, uuid(&r.request_id, "request_id")?, keys).await?;
                proto::FulfilKeyRequestResponse { request: None }.encode()
            }
            "PublishWrappedKeys" => {
                let r = proto::PublishWrappedKeysRequest::decode(body).map_err(decode)?;
                let wraps = r.wraps.into_iter().map(|w| Ok(crate::keys::WrapFor { device_id: uuid(&w.device_id, "device_id")?, wrapped: w.wrapped })).collect::<Result<Vec<_>, ApiError>>()?;
                let landed = self.devices.publish_wrapped_keys(uuid(&r.device_id, "device_id")?, uuid(&r.group_id, "group_id")?, r.key_id as u32, wraps).await?;
                proto::PublishWrappedKeysResponse { keys: landed.into_iter().map(wrapped_key_to).collect() }.encode()
            }
            "GetWrappedKeys" => {
                let r = proto::GetWrappedKeysRequest::decode(body).map_err(decode)?;
                let keys = self.devices.get_wrapped_keys(uuid(&r.device_id, "device_id")?, uuid_opt(&r.group_id, "group_id")?).await?;
                proto::GetWrappedKeysResponse { keys: keys.into_iter().map(wrapped_key_to).collect() }.encode()
            }
            other => return Err(err(Code::NotFound, format!("device.v1.DeviceService/{other}"))),
        })
    }

    // ---- storage.v1 and the bucket ----

    fn storage_call(&self, user: Uuid, method: &str, body: &[u8]) -> Result<Vec<u8>, ApiError> {
        let decode = |e: crate::schema::SchemaError| err(Code::InvalidArgument, e.to_string());
        let now = self.relay.now_ms();
        Ok(match method {
            "PresignUpload" => {
                let r = proto::PresignUploadRequest::decode(body).map_err(decode)?;
                let group_id = uuid(&r.group_id, "group_id")?;
                self.require_role(group_id, user)?;
                if !r.key.starts_with(&format!("g/{group_id}/o/")) {
                    return Err(err(Code::PermissionDenied, "key outside the group's prefix"));
                }
                proto::PresignUploadResponse { key: r.key.clone(), url: format!("{}/bucket/{}?size={}", self.base_url, r.key, r.size), expires_at_ms: now + 900_000 }.encode()
            }
            "PresignDownload" => {
                let r = proto::PresignDownloadRequest::decode(body).map_err(decode)?;
                let group_id = uuid(&r.group_id, "group_id")?;
                self.require_role(group_id, user)?;
                proto::PresignDownloadResponse { key: r.key.clone(), url: format!("{}/bucket/{}", self.base_url, r.key), expires_at_ms: now + 900_000 }.encode()
            }
            other => return Err(err(Code::NotFound, format!("storage.v1.StorageService/{other}"))),
        })
    }

    fn bucket_request(&self, request: &Request) -> Response {
        let path = request.url.trim_start_matches(&self.base_url).trim_start_matches("/bucket/");
        let (key, query) = path.split_once('?').unwrap_or((path, ""));
        match request.method.as_str() {
            "PUT" => {
                let size: usize = query.strip_prefix("size=").and_then(|s| s.parse().ok()).unwrap_or(0);
                if size != request.body.len() {
                    return Response { status: 403, headers: Vec::new(), body: b"the PUT was signed for another size".to_vec() };
                }
                self.bucket.lock().unwrap().insert(key.to_owned(), request.body.clone());
                Response { status: 200, headers: Vec::new(), body: Vec::new() }
            }
            "GET" => match self.bucket.lock().unwrap().get(key) {
                Some(bytes) => Response { status: 200, headers: vec![Header::new("content-type", "application/octet-stream")], body: bytes.clone() },
                None => Response { status: 404, headers: Vec::new(), body: Vec::new() },
            },
            _ => Response { status: 405, headers: Vec::new(), body: Vec::new() },
        }
    }

    fn kratos_request(&self, request: &Request) -> Response {
        let path = request.url.trim_start_matches(&self.base_url);
        let json = |status: u16, body: String| Response { status, headers: vec![Header::new("content-type", "application/json")], body: body.into_bytes() };
        if path.starts_with("/.ory/sessions/whoami") {
            let token = request.headers.iter().find(|h| h.name == "x-session-token").map(|h| h.value.clone());
            return match token.and_then(|t| self.sessions.lock().unwrap().get(&t).copied()) {
                Some(user) => json(200, format!(r#"{{"id":"s","identity":{{"id":"{user}","traits":{{"email":"{user}@example"}}}}}}"#)),
                None => json(401, r#"{"error":{"code":401}}"#.to_owned()),
            };
        }
        json(404, r#"{"error":{"code":404}}"#.to_owned())
    }

    fn error_response(e: ApiError) -> Response {
        let (status, name) = match e.code {
            Code::InvalidArgument => (400, "invalid_argument"),
            Code::FailedPrecondition => (412, "failed_precondition"),
            Code::PermissionDenied => (403, "permission_denied"),
            Code::NotFound => (404, "not_found"),
            Code::Internal => (500, "internal"),
            Code::Unavailable => (503, "unavailable"),
        };
        let body = serde_json::json!({ "code": name, "message": e.message }).to_string();
        Response { status, headers: vec![Header::new("content-type", "application/json")], body: body.into_bytes() }
    }
}

fn entry_to(e: crate::relay::Entry) -> proto::Entry {
    proto::Entry { id: e.id.to_string(), value: e.value.unwrap_or_default(), seq: e.seq, deleted: e.deleted }
}

fn history_to(h: crate::relay::HistoryRequest) -> proto::HistoryRequest {
    let status = match h.status {
        crate::relay::HistoryStatus::Open => "open",
        crate::relay::HistoryStatus::Claimed => "claimed",
        crate::relay::HistoryStatus::Taken => "taken",
        crate::relay::HistoryStatus::Unfillable => "unfillable",
        crate::relay::HistoryStatus::Complete => "complete",
    };
    proto::HistoryRequest {
        id: if h.id.is_nil() { String::new() } else { h.id.to_string() },
        group_id: h.group_id.to_string(),
        requester_device: h.requester_device.to_string(),
        from_seq: h.from_seq,
        to_seq: h.to_seq,
        object_id: h.object_id.map(|o| o.to_string()).unwrap_or_default(),
        claimed_by: h.claimed_by.map(|d| d.to_string()).unwrap_or_default(),
        claimed_at_ms: 0,
        created_at_ms: 0,
        status: status.to_owned(),
        through_seq: h.through_seq,
    }
}

fn device_info_to(d: crate::keys::DeviceInfo) -> proto::DeviceInfo {
    proto::DeviceInfo { id: d.id.to_string(), user_id: d.user_id.to_string(), public_key: d.public_key, platform: d.platform }
}

fn key_request_to(k: crate::keys::KeyRequest) -> proto::KeyRequest {
    proto::KeyRequest { id: k.id.to_string(), group_id: k.group_id.to_string(), device: Some(device_info_to(k.device)), invite_id: k.invite_id.map(|i| i.to_string()).unwrap_or_default(), invite_tag: k.invite_tag, created_at_ms: k.created_at_ms }
}

fn wrapped_key_to(k: crate::keys::WrappedKey) -> proto::WrappedKey {
    proto::WrappedKey { id: k.id.to_string(), group_id: k.group_id.to_string(), key_id: k.key_id as i32, device_id: k.device_id.to_string(), wrapped: k.wrapped, wrapped_by_device: k.wrapped_by_device.to_string(), created_at_ms: 0, wrapped_by_public_key: k.wrapped_by_public_key }
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Transport for ModelServer {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        if !request.url.starts_with(&self.base_url) {
            return Err(TransportError::Network(format!("no route to {}", request.url)));
        }
        let path = &request.url[self.base_url.len()..];
        if path.starts_with("/.ory/") {
            return Ok(self.kratos_request(&request));
        }
        if path.starts_with("/bucket/") {
            return Ok(self.bucket_request(&request));
        }
        let Some(rest) = path.strip_prefix("/api/") else {
            return Ok(Response { status: 404, headers: Vec::new(), body: Vec::new() });
        };
        let (service, method) = rest.split_once('/').unwrap_or((rest, ""));
        let token = request.headers.iter().find(|h| h.name == "x-session-token").map(|h| h.value.clone());
        let user = token.and_then(|t| self.sessions.lock().unwrap().get(&t).copied());
        let Some(user) = user else {
            return Ok(Self::error_response(err(Code::PermissionDenied, "unauthenticated")));
        };
        let result = match service {
            "group.v1.GroupService" => self.group_call(user, method, &request.body).await,
            "relay.v1.RelayService" => self.relay_call(method, &request.body).await,
            "device.v1.DeviceService" => self.device_call(user, method, &request.body).await,
            "storage.v1.StorageService" => self.storage_call(user, method, &request.body),
            other => Err(err(Code::NotFound, format!("no service {other}"))),
        };
        Ok(match result {
            Ok(body) => Response { status: 200, headers: vec![Header::new("content-type", "application/proto")], body },
            Err(e) => Self::error_response(e),
        })
    }
}
