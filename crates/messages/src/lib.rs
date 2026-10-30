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

// --- device ---
//
// The server stores key material it cannot open: a wrapped key is
// ciphertext one member's device produced for another device's public key
// (`docs/01-deappje/02-device.md`). A device is identified by a
// client-minted id; `caller` is the Kratos identity the api crate took
// from the session, and a device belongs to exactly one user.

/// Where a device runs. Sent and stored as its lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
    Web,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
            Platform::Web => "web",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A platform name that is none of the three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPlatform(pub String);

impl fmt::Display for UnknownPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown platform {:?} (ios, android or web)", self.0)
    }
}

impl std::error::Error for UnknownPlatform {}

impl FromStr for Platform {
    type Err = UnknownPlatform;

    fn from_str(s: &str) -> Result<Platform, UnknownPlatform> {
        match s {
            "ios" => Ok(Platform::Ios),
            "android" => Ok(Platform::Android),
            "web" => Ok(Platform::Web),
            other => Err(UnknownPlatform(other.to_owned())),
        }
    }
}

/// The push service a device's token belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushProvider {
    Apns,
    Fcm,
}

impl PushProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            PushProvider::Apns => "apns",
            PushProvider::Fcm => "fcm",
        }
    }
}

impl fmt::Display for PushProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A push provider name that is neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPushProvider(pub String);

impl fmt::Display for UnknownPushProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown push provider {:?} (apns or fcm)", self.0)
    }
}

impl std::error::Error for UnknownPushProvider {}

impl FromStr for PushProvider {
    type Err = UnknownPushProvider;

    fn from_str(s: &str) -> Result<PushProvider, UnknownPushProvider> {
        match s {
            "apns" => Ok(PushProvider::Apns),
            "fcm" => Ok(PushProvider::Fcm),
            other => Err(UnknownPushProvider(other.to_owned())),
        }
    }
}

/// `RegisterDeviceRequest` — answered with `Device`: the caller's device,
/// by the id the device minted. A repeat refreshes `last_seen_at` and the
/// platform; the public key of a device never changes (a device with a
/// new key is a new device).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterDeviceRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub public_key: Vec<u8>,
    pub platform: Platform,
}

/// `SetPushTokenRequest` — answered with `Device`: the caller's device
/// gets a push token, or loses it (`None`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPushTokenRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub push: Option<PushToken>,
}

/// A push token and the service it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushToken {
    pub provider: PushProvider,
    pub token: String,
}

/// `ListDevicesRequest` — answered with `DeviceList`: the devices holding
/// a key for the group, with their public keys, so the calling device can
/// wrap for them. The calling device must hold a key itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDevicesRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
}

/// `RequestGroupKeysRequest` — answered with `KeyRequest`: the calling
/// device, whose user is a member, asks the group's holders for its keys.
/// `invite_tag` is `HMAC-SHA256(invite_secret, device_public_key)`,
/// opaque to the server. A repeat from the same device returns the open
/// request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestGroupKeysRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
    pub invite_id: Option<Uuid>,
    pub invite_tag: Option<Vec<u8>>,
}

/// `ListKeyRequestsRequest` — answered with `KeyRequestList`: the open
/// requests in the groups the calling device holds a key for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListKeyRequestsRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
}

/// `FulfilKeyRequestRequest` — answered with `KeyRequest` (the one
/// fulfilled): the calling device, a holder, hands the requesting device
/// the group's keys, one wrap per key id. The caller must be an admin or
/// member unless the requesting device is their own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FulfilKeyRequestRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub request_id: Uuid,
    pub keys: Vec<WrappedKeyInput>,
}

/// One wrap handed over by a fulfilment: the group key `key_id`, wrapped
/// for the requesting device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKeyInput {
    pub key_id: i32,
    pub wrapped: Vec<u8>,
}

/// `PublishWrappedKeysRequest` — answered with `WrappedKeyList`: the
/// calling device publishes one group key, wrapped for each listed device
/// (the first key of a group for itself, a rotation for every holder of
/// the previous key, a subgroup's key for the parent's holders). A key id
/// already published by another device is refused: the loser re-reads and
/// adopts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishWrappedKeysRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
    pub key_id: i32,
    pub wraps: Vec<WrapFor>,
}

/// One wrap of a published key: for `device_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapFor {
    pub device_id: Uuid,
    pub wrapped: Vec<u8>,
}

/// `GetWrappedKeysRequest` — answered with `WrappedKeyList`: every key
/// wrapped for the calling device, or those of one group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetWrappedKeysRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Option<Uuid>,
}

/// `RemoveDeviceRequest` — answered with `Device` (the one removed): the
/// owner drops one of their devices with its keys and requests; `relay`
/// learns per group it held a key for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveDeviceRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
}

/// `GroupHeadAdvanced` — a void fan-out event from `relay` after a push
/// batch committed: `device` wakes every holder device but the writer's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupHeadAdvanced {
    pub group_id: Uuid,
    pub head_seq: i64,
    pub writer_device: Uuid,
}

/// `HistoryRequested` — a void fan-out event from `relay`: wake the listed
/// devices (the group's holders, the requester excluded) to answer a
/// history request for a range, or for one object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRequested {
    pub request_id: Uuid,
    pub group_id: Uuid,
    pub requester_device: Uuid,
    pub from_seq: i64,
    pub to_seq: i64,
    pub object_id: Option<Uuid>,
    pub devices: Vec<Uuid>,
}

/// `HistoryReady` — a void fan-out event from `relay`: a holder pushed
/// rows through `through_seq`; wake the requester to pull. `complete`
/// means the request is fulfilled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryReady {
    pub request_id: Uuid,
    pub requester_device: Uuid,
    pub through_seq: i64,
    pub complete: bool,
}

/// `DeviceLeftGroup` — a void fan-out event from `device` to `relay`: the
/// device lost its keys for the group (member removed, device removed,
/// account purged), so `relay` stops waiting for its ack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceLeftGroup {
    pub group_id: Uuid,
    pub device_id: Uuid,
}

// --- relay ---
//
// The relay keeps, per group, the rows some device has not acked yet, 672
// hours at most; member devices are the store. A row's `seq` identifies its
// state; a tombstone is a row with `deleted` and no value
// (`docs/01-deappje/02-relay.md`). The server never parses a value.

/// A row of the log as the server holds it: `value` is the envelope
/// (plaintext header, ciphertext), absent on a tombstone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: Uuid,
    pub value: Option<Vec<u8>>,
    pub seq: i64,
    pub deleted: bool,
}

/// One object of a push: `base_seq` is the seq of the row the device
/// edited from, 0 for a new object; a delete is `deleted` with no value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushEntry {
    pub id: Uuid,
    pub value: Option<Vec<u8>>,
    pub base_seq: i64,
    pub deleted: bool,
}

/// `PushObjectsRequest` — answered with `PushResult`: one batch, one
/// transaction, from a device that holds the group (its ack at or above
/// the eviction floor); the caller must be an admin or member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushObjectsRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
    pub objects: Vec<PushEntry>,
}

/// An accepted push: the object's new seq.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    pub id: Uuid,
    pub seq: i64,
}

/// A refused push: the row as the server holds it, for the rebase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub id: Uuid,
    pub current: Entry,
}

/// The answer to a push: per object accepted or in conflict, or the whole
/// batch refused because the device is not a holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushResult {
    Accepted { accepted: Vec<Accepted>, conflicts: Vec<Conflict>, head_seq: i64 },
    BehindFloor { evicted_through_seq: i64 },
}

/// `PullChangesRequest` — answered with `ChangeSet`: the rows after
/// `since_seq`, oldest first, at most `limit` (0 is the default page).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullChangesRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
    pub since_seq: i64,
    pub limit: i32,
}

/// A page of the log with where the group stands: its head, the eviction
/// floor and the lowest ack among its devices (none when no device has a
/// row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub entries: Vec<Entry>,
    pub head_seq: i64,
    pub evicted_through_seq: i64,
    pub min_acked_seq: Option<i64>,
    pub more: bool,
}

/// `AckSeqRequest` — answered with `AckResult`: the device holds every row
/// of the group through `seq`. Monotone, except 0, a device's first act in
/// a group and its way to start over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckSeqRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckResult {
    pub evicted_through_seq: i64,
    pub min_acked_seq: Option<i64>,
}

/// `GetHeadsRequest` — answered with `HeadList`: where the device stands
/// in every listed group it is a member of.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetHeadsRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub group_id: Uuid,
    pub head_seq: i64,
    pub evicted_through_seq: i64,
    pub min_acked_seq: Option<i64>,
}

/// The groups the caller is a member of, among those asked; a group left
/// out is one the caller is no longer in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadList {
    pub heads: Vec<Head>,
}

/// `RequestHistoryRequest` — answered with `HistoryRequest`: rows in
/// `[from_seq, to_seq]`, or one object, from a holder; `unfillable` at
/// once when the group has no other holder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHistoryRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
    pub from_seq: i64,
    pub to_seq: i64,
    pub object_id: Option<Uuid>,
}

/// Where a history request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryStatus {
    /// Stored, waiting for a holder to claim it.
    Open,
    /// Claimed by the device that asked.
    Claimed,
    /// Claimed by another device.
    Taken,
    /// Nobody holds anything older than the window: nothing was stored.
    Unfillable,
}

impl HistoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryStatus::Open => "open",
            HistoryStatus::Claimed => "claimed",
            HistoryStatus::Taken => "taken",
            HistoryStatus::Unfillable => "unfillable",
        }
    }
}

/// A history request as stored, with its status for the caller. An
/// unfillable one carries a nil id: nothing was stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRequest {
    pub id: Uuid,
    pub group_id: Uuid,
    pub requester_device: Uuid,
    pub from_seq: i64,
    pub to_seq: i64,
    pub object_id: Option<Uuid>,
    pub claimed_by: Option<Uuid>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub status: HistoryStatus,
}

/// `ListHistoryRequestsRequest` — answered with `HistoryRequestList`: the
/// group's open requests a holder may claim (unclaimed, or claimed more
/// than five minutes ago).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListHistoryRequestsRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub group_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRequestList {
    pub requests: Vec<HistoryRequest>,
}

/// `ClaimHistoryRequest` — answered with `HistoryRequest`: the calling
/// device, a holder, takes the request; the first wins, the rest are
/// answered `taken`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimHistoryRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub request_id: Uuid,
}

/// `PushHistoryRequest` — answered with `PushHistoryResult`: the claimer
/// pushes the rows it holds in the range, oldest first, as they were in
/// the log; `complete` on the last call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushHistoryRequest {
    pub caller: Uuid,
    pub device_id: Uuid,
    pub request_id: Uuid,
    pub entries: Vec<Entry>,
    pub complete: bool,
}

/// What a history push did: rows re-inserted at their original seqs, rows
/// skipped because an id already had a row, and the last seq of the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushHistoryResult {
    pub inserted: i64,
    pub skipped: i64,
    pub through_seq: i64,
    pub complete: bool,
}

/// `ObjectDeleted` — a void fan-out event from `relay` to `storage`, per
/// tombstone: the object's files go.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDeleted {
    pub group_id: Uuid,
    pub object_id: Uuid,
}

// --- storage ---
//
// A file lives in the environment's private bucket under
// `g/<group_id>/o/<object_id>/<slot>`; the group prefix is the
// authorisation boundary, the slot names the size cap
// (`docs/01-deappje/02-storage.md`). The server never sees the bytes.

/// `PresignUploadRequest` — answered with `UploadGrant`: a presigned PUT
/// for `key` under the caller's group, `size` bytes (the slot's cap and
/// the group's quota apply; the PUT is signed for exactly that length).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
    pub key: String,
    pub size: i64,
}

/// `PresignDownloadRequest` — answered with `DownloadGrant`: a presigned
/// GET for `key` under the caller's group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadRequest {
    pub caller: Uuid,
    pub group_id: Uuid,
    pub key: String,
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

/// A device as its owner sees it, push token included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: Uuid,
    pub user_id: Uuid,
    pub public_key: Vec<u8>,
    pub platform: Platform,
    pub push: Option<PushToken>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// A device as another member sees it: what wrapping a key for it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: Uuid,
    pub user_id: Uuid,
    pub public_key: Vec<u8>,
    pub platform: Platform,
}

/// The holders of a group's keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceList {
    pub group_id: Uuid,
    pub devices: Vec<DeviceInfo>,
}

/// An open key request, with what a holder needs to verify the
/// requesting device: its user, platform and public key, and the invite
/// tag when it came through an invite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRequest {
    pub id: Uuid,
    pub group_id: Uuid,
    pub device: DeviceInfo,
    pub invite_id: Option<Uuid>,
    pub invite_tag: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
}

/// The open requests in the groups the calling device holds a key for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRequestList {
    pub requests: Vec<KeyRequest>,
}

/// One copy of a group key, wrapped for one device, with the public key
/// of the device that wrapped it (empty when that device is gone): the
/// recipient needs it to open the wrap, and holds nothing yet to list the
/// holders with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    pub id: Uuid,
    pub group_id: Uuid,
    pub key_id: i32,
    pub device_id: Uuid,
    pub wrapped: Vec<u8>,
    pub wrapped_by_device: Uuid,
    pub wrapped_by_public_key: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

/// Wrapped keys, by group then key id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKeyList {
    pub keys: Vec<WrappedKey>,
}

/// A presigned PUT, valid until `expires_at`, on the environment's
/// private-bucket address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadGrant {
    pub key: String,
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

/// A presigned GET, valid until `expires_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadGrant {
    pub key: String,
    pub url: String,
    pub expires_at: DateTime<Utc>,
}
// basable:messages-end
