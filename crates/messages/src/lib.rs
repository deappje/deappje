//! The messages nanoservices exchange. Every `message:` named in
//! `routing.yaml` is a struct here — the messenger codegen references them by
//! name, so a missing one is a build error naming it.
//!
//! Rules: a message carries the SENDER's generation when it hands derived
//! state to another nanoservice (the Directive §7); it names things by the
//! caller's seam (a name, never an id the receiver mints); only derived
//! state crosses. Keep them plain data: no methods that reach a database.

#![allow(unused_imports)]

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// basable:messages-modules-begin — the ledger's snapshot types, when a nanoservice owns the ledger
// basable:messages-modules-end

// basable:messages-begin — declare one struct per message in routing.yaml.

// --- group ---
//
// Every API-facing request of `group` carries `caller`, the Kratos identity
// id the api crate took from the validated session: the handler decides
// what that person may do, the boundary never does. Ids are server-minted
// UUIDs; a group's name and avatar are ciphertext in the object log, so no
// message here carries either (`docs/01-deappje/02-group.md`).

/// A member's role in a group. `admin` manages members, invites and roles
/// and can delete the group; `member` reads and writes; `viewer` holds the
/// keys and reads, and the server refuses its pushes to the log (`relay`)
/// and its approval of another user's device (`device`). Stored and sent
/// as its lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
    Viewer,
}

impl Role {
    /// The lowercase name, as stored and sent.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Viewer => "viewer",
        }
    }

    /// Whether the role may write to the group's object log.
    pub fn can_write(self) -> bool {
        !matches!(self, Role::Viewer)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A role name that is none of the three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRole(pub String);

impl fmt::Display for UnknownRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown role {:?} (admin, member or viewer)", self.0)
    }
}

impl std::error::Error for UnknownRole {}

impl FromStr for Role {
    type Err = UnknownRole;

    fn from_str(s: &str) -> Result<Role, UnknownRole> {
        match s {
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            "viewer" => Ok(Role::Viewer),
            other => Err(UnknownRole(other.to_owned())),
        }
    }
}

/// `CreateGroupRequest` — answered with `Group`. A `parent_group_id` makes a
/// subgroup; the caller must be a member of the parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub caller: Uuid,
    pub parent_group_id: Option<Uuid>,
}

/// `GetGroupRequest` — answered with `Group`; the caller must be a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetGroupRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
}

/// `ListGroupsRequest` — answered with `GroupList`: every live group the
/// caller is in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListGroupsRequest {
    pub caller: Uuid,
}

/// `CreateInviteRequest` — answered with `Invite`; the caller must be an
/// admin. `role` is what a joiner gets (`member` or `viewer`); `max_uses`
/// 0 is unbounded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInviteRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
    pub role: Role,
    pub expires_at: DateTime<Utc>,
    pub max_uses: i32,
}

/// `AcceptInviteRequest` — answered with `Membership`: the caller joins the
/// invite's group with the invite's role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptInviteRequest {
    pub caller: Uuid,
    pub code: String,
}

/// `ListMembersRequest` — answered with `MemberList`; the caller must be a
/// member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMembersRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
}

/// `AddMemberRequest` — answered with `Membership`. Subgroups only: the
/// caller is an admin or member of the subgroup and `user_id` a member of
/// the parent; no invite, no acceptance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemberRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
}

/// `RemoveMemberRequest` — answered with `Membership` (the one removed);
/// the caller must be an admin. Cascades to the group's subgroups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveMemberRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
    pub user_id: Uuid,
}

/// `SetMemberRoleRequest` — answered with `Membership`; the caller must be
/// an admin. The last admin cannot be demoted while other members remain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMemberRoleRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
}

/// `LeaveGroupRequest` — answered with `Membership` (the one given up).
/// Cascades to subgroups; the last admin cannot leave while other members
/// remain; the last member leaving deletes the group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveGroupRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
}

/// `DeleteGroupRequest` — answered with `Group` (stamped deleted); the
/// caller must be an admin. Starts a `group_purge` per group and subgroup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteGroupRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
}

/// `DeleteAccountRequest` — answered with `AccountDeletion`: the caller
/// deletes their own account, a `user_purge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteAccountRequest {
    pub caller: Uuid,
}

/// `CheckMembershipRequest` — answered with `MembershipCheck`: the
/// authorisation question every other nanoservice asks before it touches
/// anything in a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckMembershipRequest {
    pub user_id: Uuid,
    pub group_id: Uuid,
}

/// `MemberRemoved` — a void fan-out event: a user lost a membership (left,
/// removed, cascaded from a parent, or purged), sent after the commit.
/// `device` drops the user's wrapped keys for the group and asks the
/// remaining devices to rotate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRemoved {
    pub group_id: Uuid,
    pub user_id: Uuid,
    pub removed_at: DateTime<Utc>,
}

/// `PurgeGroupObjectsRequest` — answered with `PurgeResult`: `relay` drops
/// everything of the group, a batch per call. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeGroupObjectsRequest {
    pub group_id: Uuid,
}

/// `PurgeGroupBlobsRequest` — answered with `PurgeResult`: `storage`
/// deletes the group's bucket prefix, a batch per call. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeGroupBlobsRequest {
    pub group_id: Uuid,
}

/// `PurgeGroupKeysRequest` — answered with `PurgeResult`: `device` drops
/// the group's wrapped keys and key requests. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeGroupKeysRequest {
    pub group_id: Uuid,
}

/// `PurgeUserDevicesRequest` — answered with `PurgeResult`: `device` drops
/// the user's devices with their keys and requests. Idempotent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeUserDevicesRequest {
    pub user_id: Uuid,
}

/// `RegisterDeviceRequest` — answered with `Device` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterDeviceRequest {
    // TODO: the fields.
}

/// `SetPushTokenRequest` — answered with `Device` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPushTokenRequest {
    // TODO: the fields.
}

/// `ListDevicesRequest` — answered with `DeviceList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDevicesRequest {
    // TODO: the fields.
}

/// `RequestGroupKeysRequest` — answered with `KeyRequest` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestGroupKeysRequest {
    // TODO: the fields.
}

/// `ListKeyRequestsRequest` — answered with `KeyRequestList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListKeyRequestsRequest {
    // TODO: the fields.
}

/// `FulfilKeyRequestRequest` — answered with `KeyRequest` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FulfilKeyRequestRequest {
    // TODO: the fields.
}

/// `PublishWrappedKeysRequest` — answered with `WrappedKeyList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishWrappedKeysRequest {
    // TODO: the fields.
}

/// `GetWrappedKeysRequest` — answered with `WrappedKeyList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetWrappedKeysRequest {
    // TODO: the fields.
}

/// `RemoveDeviceRequest` — answered with `Device` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveDeviceRequest {
    // TODO: the fields.
}

/// `GroupHeadAdvanced` — a void fan-out event (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupHeadAdvanced {
    // TODO: the fields.
}

/// `HistoryRequested` — a void fan-out event (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRequested {
    // TODO: the fields.
}

/// `HistoryReady` — a void fan-out event (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryReady {
    // TODO: the fields.
}

/// `DeviceLeftGroup` — a void fan-out event (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceLeftGroup {
    // TODO: the fields.
}

/// `PushObjectsRequest` — answered with `PushResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushObjectsRequest {
    // TODO: the fields.
}

/// `PullChangesRequest` — answered with `ChangeSet` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullChangesRequest {
    // TODO: the fields.
}

/// `AckSeqRequest` — answered with `AckResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckSeqRequest {
    // TODO: the fields.
}

/// `GetHeadsRequest` — answered with `HeadList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetHeadsRequest {
    // TODO: the fields.
}

/// `RequestHistoryRequest` — answered with `HistoryRequest` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHistoryRequest {
    // TODO: the fields.
}

/// `ListHistoryRequestsRequest` — answered with `HistoryRequestList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListHistoryRequestsRequest {
    // TODO: the fields.
}

/// `ClaimHistoryRequest` — answered with `HistoryRequest` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimHistoryRequest {
    // TODO: the fields.
}

/// `PushHistoryRequest` — answered with `PushHistoryResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushHistoryRequest {
    // TODO: the fields.
}

/// `ObjectDeleted` — a void fan-out event (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDeleted {
    // TODO: the fields.
}

/// `PresignUploadRequest` — answered with `UploadGrant` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadRequest {
    // TODO: the fields.
}

/// `PresignDownloadRequest` — answered with `DownloadGrant` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadRequest {
    // TODO: the fields.
}

/// A group as `group` holds it. `deleted_at` is set from the moment a
/// deletion was accepted: such a group answers no membership and goes
/// when its purge finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: Uuid,
    pub parent_group_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// The live groups of one user, with their parent ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupList {
    pub groups: Vec<Group>,
}

/// An invite. The shared link is `/join/<code>#<secret>`; the secret never
/// reaches the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub id: Uuid,
    pub group_id: Uuid,
    pub code: String,
    pub role: Role,
    pub created_by: Uuid,
    pub expires_at: DateTime<Utc>,
    pub max_uses: i32,
    pub uses: i32,
    pub revoked: bool,
}

/// One user's membership of one group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub id: Uuid,
    pub group_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
    pub joined_at: DateTime<Utc>,
}

/// The members of one group, oldest membership first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberList {
    pub group_id: Uuid,
    pub members: Vec<Membership>,
}

/// An accepted account deletion: the `user_purge` driving it, by public id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDeletion {
    pub user_id: Uuid,
    pub purge_id: String,
}

/// The answer to `CheckMembershipRequest`: the role when the user is a
/// member of a live group, nothing otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipCheck {
    pub role: Option<Role>,
}

impl MembershipCheck {
    pub fn is_member(&self) -> bool {
        self.role.is_some()
    }
}

/// What a purge request leaves behind: zero means the receiver holds
/// nothing of it any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeResult {
    pub remaining: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRequest {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRequestList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedKeyList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeSet {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckResult {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRequest {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRequestList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushHistoryResult {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadGrant {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadGrant {
    // TODO: the fields.
}
// basable:messages-end
