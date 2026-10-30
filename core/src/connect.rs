//! The server over Connect (`https://connectrpc.com/docs/protocol`): one
//! unary POST per call to `/api/<package>.<Service>/<Method>`, the request
//! as protobuf, the answer as protobuf on 200 and as the protocol's JSON
//! error otherwise, the session as Kratos's token header. The [`Relay`]
//! and [`Devices`] seams are implemented here over a [`Transport`], and
//! the group and storage services as plain methods.

use std::cell::RefCell;

use uuid::Uuid;

use crate::api::{ApiError, Code};
use crate::keys::{DeviceInfo, Devices, KeyRequest, WrapFor, WrappedKey};
use crate::proto;
use crate::relay::{Accepted, AckResult, ChangeSet, Conflict, Entry, Head, HistoryRequest, HistoryStatus, PushEntry, PushHistoryResult, PushResult, Relay};
use crate::schema::Message;
use crate::transport::{Request, Transport};

/// The header a native app's session travels in.
pub const SESSION_TOKEN_HEADER: &str = "x-session-token";

/// A group as the server has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The group.
    pub id: Uuid,
    /// Its parent, for a subgroup.
    pub parent_group_id: Option<Uuid>,
    /// Who created it.
    pub created_by: Uuid,
    /// When.
    pub created_at_ms: i64,
    /// Set once a deletion was accepted.
    pub deleted_at_ms: Option<i64>,
}

/// One user's membership of one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The row.
    pub id: Uuid,
    /// The group.
    pub group_id: Uuid,
    /// The user.
    pub user_id: Uuid,
    /// `admin`, `member` or `viewer`.
    pub role: String,
    /// When they joined.
    pub joined_at_ms: i64,
}

/// An invite; the shared link is `/join/<code>#<secret>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// The row.
    pub id: Uuid,
    /// The group.
    pub group_id: Uuid,
    /// The code in the link.
    pub code: String,
    /// The role a joiner gets.
    pub role: String,
    /// When it expires.
    pub expires_at_ms: i64,
    /// 0 is unbounded.
    pub max_uses: i32,
    /// How often it was used.
    pub uses: i32,
    /// Whether it was revoked.
    pub revoked: bool,
}

/// A presigned URL for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presigned {
    /// The bucket key.
    pub key: String,
    /// The URL, used as it is.
    pub url: String,
    /// When it stops working.
    pub expires_at_ms: i64,
}

/// The client.
pub struct Connect<T: Transport> {
    transport: T,
    /// The origin, without a trailing slash: `https://<host>`.
    base_url: String,
    session_token: RefCell<Option<String>>,
}

fn uuid(s: &str) -> Result<Uuid, ApiError> {
    s.parse().map_err(|_| ApiError::new(Code::Internal, format!("the server sent an id that is not one: {s:?}")))
}

fn uuid_opt(s: &str) -> Result<Option<Uuid>, ApiError> {
    if s.is_empty() { Ok(None) } else { uuid(s).map(Some) }
}

fn present<M>(m: Option<M>, what: &str) -> Result<M, ApiError> {
    m.ok_or_else(|| ApiError::new(Code::Internal, format!("the server's answer has no {what}")))
}

fn opt_i64(value: i64, has: bool) -> Option<i64> {
    if has { Some(value) } else { None }
}

/// The error the Connect protocol puts in a JSON body.
#[derive(serde::Deserialize)]
struct ConnectErrorBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

fn code_of(name: &str, status: u16) -> Code {
    match name {
        "invalid_argument" | "out_of_range" => Code::InvalidArgument,
        "failed_precondition" | "aborted" | "already_exists" => Code::FailedPrecondition,
        "permission_denied" | "unauthenticated" => Code::PermissionDenied,
        "not_found" => Code::NotFound,
        "unavailable" | "deadline_exceeded" | "canceled" => Code::Unavailable,
        "" => match status {
            400 => Code::InvalidArgument,
            401 | 403 => Code::PermissionDenied,
            404 => Code::NotFound,
            412 | 409 => Code::FailedPrecondition,
            502..=504 => Code::Unavailable,
            _ => Code::Internal,
        },
        _ => Code::Internal,
    }
}

impl<T: Transport> Connect<T> {
    /// A client for the origin.
    pub fn new(transport: T, base_url: &str) -> Connect<T> {
        Connect { transport, base_url: base_url.trim_end_matches('/').to_owned(), session_token: RefCell::new(None) }
    }

    /// The session token to send, or none (a browser sends its cookie).
    pub fn set_session_token(&self, token: Option<String>) {
        *self.session_token.borrow_mut() = token;
    }

    /// The origin.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The transport, for the parts that are not Connect (Kratos, the
    /// bucket).
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The headers every call carries.
    pub fn headers(&self, content_type: &str) -> Vec<(String, String)> {
        let mut h = vec![("content-type".to_owned(), content_type.to_owned()), ("connect-protocol-version".to_owned(), "1".to_owned())];
        if let Some(token) = self.session_token.borrow().as_ref() {
            h.push((SESSION_TOKEN_HEADER.to_owned(), token.clone()));
        }
        h
    }

    /// One unary call.
    pub async fn unary<Req: Message, Resp: Message>(&self, service: &str, method: &str, request: &Req) -> Result<Resp, ApiError> {
        let url = format!("{}/api/{service}/{method}", self.base_url);
        let response = self
            .transport
            .send(Request { method: "POST".to_owned(), url, headers: self.headers("application/proto"), body: request.encode() })
            .await
            .map_err(|e| ApiError::new(Code::Unavailable, e.to_string()))?;
        if response.status == 200 {
            return Resp::decode(&response.body).map_err(|e| ApiError::new(Code::Internal, format!("the server's answer does not parse: {e}")));
        }
        let body: ConnectErrorBody = serde_json::from_slice(&response.body).unwrap_or(ConnectErrorBody { code: String::new(), message: String::new() });
        let message = if body.message.is_empty() { format!("HTTP {}", response.status) } else { body.message };
        Err(ApiError::new(code_of(&body.code, response.status), message))
    }

    // ---- group.v1.GroupService ----

    /// Creates a group, or a subgroup of `parent`.
    pub async fn create_group(&self, parent: Option<Uuid>) -> Result<Group, ApiError> {
        let r: proto::CreateGroupResponse = self.unary("group.v1.GroupService", "CreateGroup", &proto::CreateGroupRequest { parent_group_id: parent.map(|p| p.to_string()).unwrap_or_default() }).await?;
        group_of(present(r.group, "group")?)
    }

    /// Every live group the caller is in.
    pub async fn list_groups(&self) -> Result<Vec<Group>, ApiError> {
        let r: proto::ListGroupsResponse = self.unary("group.v1.GroupService", "ListGroups", &proto::ListGroupsRequest::default()).await?;
        r.groups.into_iter().map(group_of).collect()
    }

    /// One group.
    pub async fn get_group(&self, group_id: Uuid) -> Result<Group, ApiError> {
        let r: proto::GetGroupResponse = self.unary("group.v1.GroupService", "GetGroup", &proto::GetGroupRequest { group_id: group_id.to_string() }).await?;
        group_of(present(r.group, "group")?)
    }

    /// Starts the deletion of a group.
    pub async fn delete_group(&self, group_id: Uuid) -> Result<Group, ApiError> {
        let r: proto::DeleteGroupResponse = self.unary("group.v1.GroupService", "DeleteGroup", &proto::DeleteGroupRequest { group_id: group_id.to_string() }).await?;
        group_of(present(r.group, "group")?)
    }

    /// Deletes the caller's account; answers the purge's id.
    pub async fn delete_account(&self) -> Result<String, ApiError> {
        let r: proto::DeleteAccountResponse = self.unary("group.v1.GroupService", "DeleteAccount", &proto::DeleteAccountRequest::default()).await?;
        Ok(r.purge_id)
    }

    /// An invite with a role, an expiry and a use cap (0 unbounded).
    pub async fn create_invite(&self, group_id: Uuid, role: &str, expires_at_ms: i64, max_uses: i32) -> Result<Invite, ApiError> {
        let r: proto::CreateInviteResponse = self.unary("group.v1.GroupService", "CreateInvite", &proto::CreateInviteRequest { group_id: group_id.to_string(), role: role.to_owned(), expires_at_ms, max_uses }).await?;
        invite_of(present(r.invite, "invite")?)
    }

    /// Joins the invite's group.
    pub async fn accept_invite(&self, code: &str) -> Result<Membership, ApiError> {
        let r: proto::AcceptInviteResponse = self.unary("group.v1.GroupService", "AcceptInvite", &proto::AcceptInviteRequest { code: code.to_owned() }).await?;
        membership_of(present(r.membership, "membership")?)
    }

    /// The group's members, oldest first.
    pub async fn list_members(&self, group_id: Uuid) -> Result<Vec<Membership>, ApiError> {
        let r: proto::ListMembersResponse = self.unary("group.v1.GroupService", "ListMembers", &proto::ListMembersRequest { group_id: group_id.to_string() }).await?;
        r.members.into_iter().map(membership_of).collect()
    }

    /// Adds a member of the parent to a subgroup.
    pub async fn add_member(&self, group_id: Uuid, user_id: Uuid, role: &str) -> Result<Membership, ApiError> {
        let r: proto::AddMemberResponse = self.unary("group.v1.GroupService", "AddMember", &proto::AddMemberRequest { group_id: group_id.to_string(), user_id: user_id.to_string(), role: role.to_owned() }).await?;
        membership_of(present(r.membership, "membership")?)
    }

    /// Removes a member.
    pub async fn remove_member(&self, group_id: Uuid, user_id: Uuid) -> Result<Membership, ApiError> {
        let r: proto::RemoveMemberResponse = self.unary("group.v1.GroupService", "RemoveMember", &proto::RemoveMemberRequest { group_id: group_id.to_string(), user_id: user_id.to_string() }).await?;
        membership_of(present(r.membership, "membership")?)
    }

    /// Changes a member's role.
    pub async fn set_member_role(&self, group_id: Uuid, user_id: Uuid, role: &str) -> Result<Membership, ApiError> {
        let r: proto::SetMemberRoleResponse = self.unary("group.v1.GroupService", "SetMemberRole", &proto::SetMemberRoleRequest { group_id: group_id.to_string(), user_id: user_id.to_string(), role: role.to_owned() }).await?;
        membership_of(present(r.membership, "membership")?)
    }

    /// Leaves a group.
    pub async fn leave_group(&self, group_id: Uuid) -> Result<Membership, ApiError> {
        let r: proto::LeaveGroupResponse = self.unary("group.v1.GroupService", "LeaveGroup", &proto::LeaveGroupRequest { group_id: group_id.to_string() }).await?;
        membership_of(present(r.membership, "membership")?)
    }

    // ---- storage.v1.StorageService ----

    /// A presigned PUT for a file of exactly `size` bytes.
    pub async fn presign_upload(&self, group_id: Uuid, key: &str, size: i64) -> Result<Presigned, ApiError> {
        let r: proto::PresignUploadResponse = self.unary("storage.v1.StorageService", "PresignUpload", &proto::PresignUploadRequest { group_id: group_id.to_string(), key: key.to_owned(), size }).await?;
        Ok(Presigned { key: r.key, url: r.url, expires_at_ms: r.expires_at_ms })
    }

    /// A presigned GET for a file.
    pub async fn presign_download(&self, group_id: Uuid, key: &str) -> Result<Presigned, ApiError> {
        let r: proto::PresignDownloadResponse = self.unary("storage.v1.StorageService", "PresignDownload", &proto::PresignDownloadRequest { group_id: group_id.to_string(), key: key.to_owned() }).await?;
        Ok(Presigned { key: r.key, url: r.url, expires_at_ms: r.expires_at_ms })
    }

    // ---- device.v1.DeviceService, the parts the key exchange does not call ----

    /// Hands the server the push token, or clears it with an empty one.
    pub async fn set_push_token(&self, device_id: Uuid, provider: &str, token: &str) -> Result<(), ApiError> {
        let _: proto::SetPushTokenResponse = self.unary("device.v1.DeviceService", "SetPushToken", &proto::SetPushTokenRequest { device_id: device_id.to_string(), push_provider: provider.to_owned(), push_token: token.to_owned() }).await?;
        Ok(())
    }

    /// Drops one of the caller's devices with its keys and requests.
    pub async fn remove_device(&self, device_id: Uuid) -> Result<(), ApiError> {
        let _: proto::RemoveDeviceResponse = self.unary("device.v1.DeviceService", "RemoveDevice", &proto::RemoveDeviceRequest { device_id: device_id.to_string() }).await?;
        Ok(())
    }
}

fn group_of(g: proto::Group) -> Result<Group, ApiError> {
    Ok(Group { id: uuid(&g.id)?, parent_group_id: uuid_opt(&g.parent_group_id)?, created_by: uuid(&g.created_by)?, created_at_ms: g.created_at_ms, deleted_at_ms: (g.deleted_at_ms != 0).then_some(g.deleted_at_ms) })
}

fn membership_of(m: proto::Membership) -> Result<Membership, ApiError> {
    Ok(Membership { id: uuid(&m.id)?, group_id: uuid(&m.group_id)?, user_id: uuid(&m.user_id)?, role: m.role, joined_at_ms: m.joined_at_ms })
}

fn invite_of(i: proto::Invite) -> Result<Invite, ApiError> {
    Ok(Invite { id: uuid(&i.id)?, group_id: uuid(&i.group_id)?, code: i.code, role: i.role, expires_at_ms: i.expires_at_ms, max_uses: i.max_uses, uses: i.uses, revoked: i.revoked })
}

fn entry_of(e: proto::Entry) -> Result<Entry, ApiError> {
    Ok(Entry { id: uuid(&e.id)?, value: if e.deleted || e.value.is_empty() { None } else { Some(e.value) }, seq: e.seq, deleted: e.deleted })
}

fn entry_to(e: Entry) -> proto::Entry {
    proto::Entry { id: e.id.to_string(), value: e.value.unwrap_or_default(), seq: e.seq, deleted: e.deleted }
}

fn history_request_of(r: proto::HistoryRequest) -> Result<HistoryRequest, ApiError> {
    let status = match r.status.as_str() {
        "open" => HistoryStatus::Open,
        "claimed" => HistoryStatus::Claimed,
        "taken" => HistoryStatus::Taken,
        "unfillable" => HistoryStatus::Unfillable,
        other => return Err(ApiError::new(Code::Internal, format!("a history request status the core does not know: {other:?}"))),
    };
    Ok(HistoryRequest {
        id: if r.id.is_empty() { Uuid::nil() } else { uuid(&r.id)? },
        group_id: uuid(&r.group_id)?,
        requester_device: uuid(&r.requester_device)?,
        from_seq: r.from_seq,
        to_seq: r.to_seq,
        object_id: uuid_opt(&r.object_id)?,
        claimed_by: uuid_opt(&r.claimed_by)?,
        status,
    })
}

impl<T: Transport> Relay for Connect<T> {
    async fn push_objects(&self, device_id: Uuid, group_id: Uuid, objects: Vec<PushEntry>) -> Result<PushResult, ApiError> {
        let request = proto::PushObjectsRequest {
            device_id: device_id.to_string(),
            group_id: group_id.to_string(),
            objects: objects.into_iter().map(|o| proto::PushEntry { id: o.id.to_string(), value: o.value.unwrap_or_default(), base_seq: o.base_seq, deleted: o.deleted }).collect(),
        };
        let r: proto::PushObjectsResponse = self.unary("relay.v1.RelayService", "PushObjects", &request).await?;
        if r.behind_floor {
            return Ok(PushResult::BehindFloor { evicted_through_seq: r.evicted_through_seq });
        }
        let accepted = r.accepted.into_iter().map(|a| Ok(Accepted { id: uuid(&a.id)?, seq: a.seq })).collect::<Result<_, ApiError>>()?;
        let conflicts = r.conflicts.into_iter().map(|c| Ok(Conflict { id: uuid(&c.id)?, current: entry_of(present(c.current, "conflict row")?)? })).collect::<Result<_, ApiError>>()?;
        Ok(PushResult::Accepted { accepted, conflicts, head_seq: r.head_seq })
    }

    async fn pull_changes(&self, device_id: Uuid, group_id: Uuid, since_seq: i64, limit: i32) -> Result<ChangeSet, ApiError> {
        let r: proto::PullChangesResponse = self.unary("relay.v1.RelayService", "PullChanges", &proto::PullChangesRequest { device_id: device_id.to_string(), group_id: group_id.to_string(), since_seq, limit }).await?;
        Ok(ChangeSet {
            entries: r.entries.into_iter().map(entry_of).collect::<Result<_, _>>()?,
            head_seq: r.head_seq,
            evicted_through_seq: r.evicted_through_seq,
            min_acked_seq: opt_i64(r.min_acked_seq, r.has_min_acked_seq),
            more: r.more,
        })
    }

    async fn ack_seq(&self, device_id: Uuid, group_id: Uuid, seq: i64) -> Result<AckResult, ApiError> {
        let r: proto::AckSeqResponse = self.unary("relay.v1.RelayService", "AckSeq", &proto::AckSeqRequest { device_id: device_id.to_string(), group_id: group_id.to_string(), seq }).await?;
        Ok(AckResult { evicted_through_seq: r.evicted_through_seq, min_acked_seq: opt_i64(r.min_acked_seq, r.has_min_acked_seq) })
    }

    async fn get_heads(&self, device_id: Uuid, group_ids: Vec<Uuid>) -> Result<Vec<Head>, ApiError> {
        let r: proto::GetHeadsResponse = self.unary("relay.v1.RelayService", "GetHeads", &proto::GetHeadsRequest { device_id: device_id.to_string(), group_ids: group_ids.iter().map(Uuid::to_string).collect() }).await?;
        r.heads
            .into_iter()
            .map(|h| Ok(Head { group_id: uuid(&h.group_id)?, head_seq: h.head_seq, evicted_through_seq: h.evicted_through_seq, min_acked_seq: opt_i64(h.min_acked_seq, h.has_min_acked_seq) }))
            .collect()
    }

    async fn request_history(&self, device_id: Uuid, group_id: Uuid, from_seq: i64, to_seq: i64, object_id: Option<Uuid>) -> Result<HistoryRequest, ApiError> {
        let r: proto::RequestHistoryResponse = self
            .unary("relay.v1.RelayService", "RequestHistory", &proto::RequestHistoryRequest { device_id: device_id.to_string(), group_id: group_id.to_string(), from_seq, to_seq, object_id: object_id.map(|o| o.to_string()).unwrap_or_default() })
            .await?;
        history_request_of(present(r.request, "request")?)
    }

    async fn list_history_requests(&self, device_id: Uuid, group_id: Uuid) -> Result<Vec<HistoryRequest>, ApiError> {
        let r: proto::ListHistoryRequestsResponse = self.unary("relay.v1.RelayService", "ListHistoryRequests", &proto::ListHistoryRequestsRequest { device_id: device_id.to_string(), group_id: group_id.to_string() }).await?;
        r.requests.into_iter().map(history_request_of).collect()
    }

    async fn claim_history(&self, device_id: Uuid, request_id: Uuid) -> Result<HistoryRequest, ApiError> {
        let r: proto::ClaimHistoryResponse = self.unary("relay.v1.RelayService", "ClaimHistory", &proto::ClaimHistoryRequest { device_id: device_id.to_string(), request_id: request_id.to_string() }).await?;
        history_request_of(present(r.request, "request")?)
    }

    async fn push_history(&self, device_id: Uuid, request_id: Uuid, entries: Vec<Entry>, complete: bool) -> Result<PushHistoryResult, ApiError> {
        let r: proto::PushHistoryResponse = self
            .unary("relay.v1.RelayService", "PushHistory", &proto::PushHistoryRequest { device_id: device_id.to_string(), request_id: request_id.to_string(), entries: entries.into_iter().map(entry_to).collect(), complete })
            .await?;
        Ok(PushHistoryResult { inserted: r.inserted, skipped: r.skipped, through_seq: r.through_seq, complete: r.complete })
    }
}

fn device_info_of(d: proto::DeviceInfo) -> Result<DeviceInfo, ApiError> {
    Ok(DeviceInfo { id: uuid(&d.id)?, user_id: uuid(&d.user_id)?, public_key: d.public_key, platform: d.platform })
}

fn key_request_of(r: proto::KeyRequest) -> Result<KeyRequest, ApiError> {
    Ok(KeyRequest { id: uuid(&r.id)?, group_id: uuid(&r.group_id)?, device: device_info_of(present(r.device, "device")?)?, invite_id: uuid_opt(&r.invite_id)?, invite_tag: r.invite_tag, created_at_ms: r.created_at_ms })
}

fn wrapped_key_of(k: proto::WrappedKey) -> Result<WrappedKey, ApiError> {
    Ok(WrappedKey {
        id: uuid(&k.id)?,
        group_id: uuid(&k.group_id)?,
        key_id: u32::try_from(k.key_id).map_err(|_| ApiError::new(Code::Internal, "a negative key id"))?,
        device_id: uuid(&k.device_id)?,
        wrapped: k.wrapped,
        wrapped_by_device: uuid(&k.wrapped_by_device)?,
        wrapped_by_public_key: k.wrapped_by_public_key,
    })
}

impl<T: Transport> Devices for Connect<T> {
    async fn register_device(&self, device_id: Uuid, public_key: [u8; 32], platform: &str) -> Result<(), ApiError> {
        let _: proto::RegisterDeviceResponse = self.unary("device.v1.DeviceService", "RegisterDevice", &proto::RegisterDeviceRequest { device_id: device_id.to_string(), public_key: public_key.to_vec(), platform: platform.to_owned() }).await?;
        Ok(())
    }

    async fn list_devices(&self, device_id: Uuid, group_id: Uuid) -> Result<Vec<DeviceInfo>, ApiError> {
        let r: proto::ListDevicesResponse = self.unary("device.v1.DeviceService", "ListDevices", &proto::ListDevicesRequest { device_id: device_id.to_string(), group_id: group_id.to_string() }).await?;
        r.devices.into_iter().map(device_info_of).collect()
    }

    async fn request_group_keys(&self, device_id: Uuid, group_id: Uuid, invite_id: Option<Uuid>, invite_tag: Option<[u8; 32]>) -> Result<KeyRequest, ApiError> {
        let r: proto::RequestGroupKeysResponse = self
            .unary(
                "device.v1.DeviceService",
                "RequestGroupKeys",
                &proto::RequestGroupKeysRequest { device_id: device_id.to_string(), group_id: group_id.to_string(), invite_id: invite_id.map(|i| i.to_string()).unwrap_or_default(), invite_tag: invite_tag.map(|t| t.to_vec()).unwrap_or_default() },
            )
            .await?;
        key_request_of(present(r.request, "request")?)
    }

    async fn list_key_requests(&self, device_id: Uuid) -> Result<Vec<KeyRequest>, ApiError> {
        let r: proto::ListKeyRequestsResponse = self.unary("device.v1.DeviceService", "ListKeyRequests", &proto::ListKeyRequestsRequest { device_id: device_id.to_string() }).await?;
        r.requests.into_iter().map(key_request_of).collect()
    }

    async fn fulfil_key_request(&self, device_id: Uuid, request_id: Uuid, keys: Vec<(u32, Vec<u8>)>) -> Result<(), ApiError> {
        let keys = keys.into_iter().map(|(key_id, wrapped)| proto::WrappedKeyInput { key_id: key_id as i32, wrapped }).collect();
        let _: proto::FulfilKeyRequestResponse = self.unary("device.v1.DeviceService", "FulfilKeyRequest", &proto::FulfilKeyRequestRequest { device_id: device_id.to_string(), request_id: request_id.to_string(), keys }).await?;
        Ok(())
    }

    async fn publish_wrapped_keys(&self, device_id: Uuid, group_id: Uuid, key_id: u32, wraps: Vec<WrapFor>) -> Result<Vec<WrappedKey>, ApiError> {
        let wraps = wraps.into_iter().map(|w| proto::WrapFor { device_id: w.device_id.to_string(), wrapped: w.wrapped }).collect();
        let r: proto::PublishWrappedKeysResponse = self.unary("device.v1.DeviceService", "PublishWrappedKeys", &proto::PublishWrappedKeysRequest { device_id: device_id.to_string(), group_id: group_id.to_string(), key_id: key_id as i32, wraps }).await?;
        r.keys.into_iter().map(wrapped_key_of).collect()
    }

    async fn get_wrapped_keys(&self, device_id: Uuid, group_id: Option<Uuid>) -> Result<Vec<WrappedKey>, ApiError> {
        let r: proto::GetWrappedKeysResponse = self.unary("device.v1.DeviceService", "GetWrappedKeys", &proto::GetWrappedKeysRequest { device_id: device_id.to_string(), group_id: group_id.map(|g| g.to_string()).unwrap_or_default() }).await?;
        r.keys.into_iter().map(wrapped_key_of).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::harness::block_on;
    use crate::transport::{Response, TransportError};

    /// A transport that records what it was sent and answers from a queue.
    #[derive(Default)]
    pub(crate) struct FakeTransport {
        pub sent: RefCell<Vec<Request>>,
        pub answers: RefCell<std::collections::VecDeque<Result<Response, TransportError>>>,
    }

    impl FakeTransport {
        pub(crate) fn answer(&self, status: u16, content_type: &str, body: Vec<u8>) {
            self.answers.borrow_mut().push_back(Ok(Response { status, headers: vec![("content-type".to_owned(), content_type.to_owned())], body }));
        }

        pub(crate) fn proto<M: Message>(&self, m: &M) {
            self.answer(200, "application/proto", m.encode());
        }
    }

    impl Transport for FakeTransport {
        async fn send(&self, request: Request) -> Result<Response, TransportError> {
            self.sent.borrow_mut().push(request);
            self.answers.borrow_mut().pop_front().unwrap_or_else(|| Err(TransportError("no answer queued".into())))
        }
    }

    #[test]
    fn a_unary_call_is_a_proto_post_with_the_session_header() {
        let t = FakeTransport::default();
        t.proto(&proto::AckSeqResponse { evicted_through_seq: 4, min_acked_seq: 2, has_min_acked_seq: true });
        let c = Connect::new(t, "https://app.example/");
        c.set_session_token(Some("tok".into()));
        let (d, g) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let r = block_on(c.ack_seq(d, g, 7)).unwrap();
        assert_eq!(r, AckResult { evicted_through_seq: 4, min_acked_seq: Some(2) });
        let sent = c.transport().sent.borrow();
        assert_eq!(sent[0].method, "POST");
        assert_eq!(sent[0].url, "https://app.example/api/relay.v1.RelayService/AckSeq");
        assert!(sent[0].headers.contains(&("content-type".to_owned(), "application/proto".to_owned())));
        assert!(sent[0].headers.contains(&("x-session-token".to_owned(), "tok".to_owned())));
        assert_eq!(proto::AckSeqRequest::decode(&sent[0].body).unwrap(), proto::AckSeqRequest { device_id: d.to_string(), group_id: g.to_string(), seq: 7 });
    }

    #[test]
    fn errors_come_back_with_the_protocol_code_or_the_status() {
        let t = FakeTransport::default();
        t.answer(403, "application/json", br#"{"code":"permission_denied","message":"not a member of this group"}"#.to_vec());
        t.answer(412, "application/json", br#"{"code":"failed_precondition","message":"ack above the head"}"#.to_vec());
        t.answer(502, "text/html", b"<html>bad gateway</html>".to_vec());
        t.answers.borrow_mut().push_back(Err(TransportError("offline".into())));
        let c = Connect::new(t, "https://app.example");
        let (d, g) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(block_on(c.ack_seq(d, g, 1)).unwrap_err(), ApiError::new(Code::PermissionDenied, "not a member of this group"));
        assert_eq!(block_on(c.ack_seq(d, g, 1)).unwrap_err(), ApiError::new(Code::FailedPrecondition, "ack above the head"));
        assert_eq!(block_on(c.ack_seq(d, g, 1)).unwrap_err(), ApiError::new(Code::Unavailable, "HTTP 502"));
        assert_eq!(block_on(c.ack_seq(d, g, 1)).unwrap_err().code, Code::Unavailable);
    }

    #[test]
    fn the_relay_messages_round_trip_through_the_proto_shapes() {
        let t = FakeTransport::default();
        let (d, g, o) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        t.proto(&proto::PushObjectsResponse {
            accepted: vec![proto::Accepted { id: o.to_string(), seq: 9 }],
            conflicts: vec![proto::Conflict { id: o.to_string(), current: Some(proto::Entry { id: o.to_string(), value: vec![], seq: 8, deleted: true }) }],
            head_seq: 9,
            ..Default::default()
        });
        t.proto(&proto::PushObjectsResponse { behind_floor: true, evicted_through_seq: 5, ..Default::default() });
        t.proto(&proto::PullChangesResponse { entries: vec![proto::Entry { id: o.to_string(), value: vec![1, 2], seq: 3, deleted: false }], head_seq: 3, evicted_through_seq: 1, min_acked_seq: 0, has_min_acked_seq: false, more: true });
        t.proto(&proto::RequestHistoryResponse { request: Some(proto::HistoryRequest { id: String::new(), group_id: g.to_string(), requester_device: d.to_string(), from_seq: 1, to_seq: 5, status: "unfillable".into(), ..Default::default() }) });
        let c = Connect::new(t, "https://app.example");
        let pushed = block_on(c.push_objects(d, g, vec![PushEntry { id: o, value: Some(vec![7]), base_seq: 0, deleted: false }])).unwrap();
        assert_eq!(pushed, PushResult::Accepted { accepted: vec![Accepted { id: o, seq: 9 }], conflicts: vec![Conflict { id: o, current: Entry { id: o, value: None, seq: 8, deleted: true } }], head_seq: 9 });
        assert_eq!(block_on(c.push_objects(d, g, vec![])).unwrap(), PushResult::BehindFloor { evicted_through_seq: 5 });
        let page = block_on(c.pull_changes(d, g, 0, 0)).unwrap();
        assert_eq!(page, ChangeSet { entries: vec![Entry { id: o, value: Some(vec![1, 2]), seq: 3, deleted: false }], head_seq: 3, evicted_through_seq: 1, min_acked_seq: None, more: true });
        let req = block_on(c.request_history(d, g, 1, 5, None)).unwrap();
        assert_eq!((req.id, req.status, req.object_id), (Uuid::nil(), HistoryStatus::Unfillable, None));
        let sent = c.transport().sent.borrow();
        let first = proto::PushObjectsRequest::decode(&sent[0].body).unwrap();
        assert_eq!(first.objects[0], proto::PushEntry { id: o.to_string(), value: vec![7], base_seq: 0, deleted: false });
    }

    #[test]
    fn the_group_and_key_messages_convert() {
        let t = FakeTransport::default();
        let (g, u, d, i) = (Uuid::from_u128(2), Uuid::from_u128(4), Uuid::from_u128(5), Uuid::from_u128(6));
        t.proto(&proto::CreateGroupResponse { group: Some(proto::Group { id: g.to_string(), parent_group_id: String::new(), created_by: u.to_string(), created_at_ms: 10, deleted_at_ms: 0 }) });
        t.proto(&proto::CreateInviteResponse { invite: Some(proto::Invite { id: i.to_string(), group_id: g.to_string(), code: "abcdefghjk".into(), role: "member".into(), created_by: u.to_string(), expires_at_ms: 99, max_uses: 0, uses: 0, revoked: false }) });
        t.proto(&proto::GetWrappedKeysResponse { keys: vec![proto::WrappedKey { id: i.to_string(), group_id: g.to_string(), key_id: 2, device_id: d.to_string(), wrapped: vec![1], wrapped_by_device: d.to_string(), created_at_ms: 1, wrapped_by_public_key: vec![9; 32] }] });
        t.proto(&proto::GetGroupResponse { group: None });
        let c = Connect::new(t, "https://app.example");
        let group = block_on(c.create_group(None)).unwrap();
        assert_eq!(group, Group { id: g, parent_group_id: None, created_by: u, created_at_ms: 10, deleted_at_ms: None });
        let invite = block_on(c.create_invite(g, "member", 99, 0)).unwrap();
        assert_eq!((invite.code.as_str(), invite.role.as_str(), invite.max_uses), ("abcdefghjk", "member", 0));
        let keys = block_on(c.get_wrapped_keys(d, Some(g))).unwrap();
        assert_eq!(keys[0], WrappedKey { id: i, group_id: g, key_id: 2, device_id: d, wrapped: vec![1], wrapped_by_device: d, wrapped_by_public_key: vec![9; 32] });
        assert_eq!(block_on(c.get_group(g)).unwrap_err().code, Code::Internal, "an answer without its group");
        let sent = c.transport().sent.borrow();
        assert_eq!(sent[1].url, "https://app.example/api/group.v1.GroupService/CreateInvite");
        assert_eq!(proto::GetWrappedKeysRequest::decode(&sent[2].body).unwrap().group_id, g.to_string());
    }
}
