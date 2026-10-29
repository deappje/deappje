//! The messages nanoservices exchange. Every `message:` named in
//! `routing.yaml` is a struct here — the messenger codegen references them by
//! name, so a missing one is a build error naming it.
//!
//! Rules: a message carries the SENDER's generation when it hands derived
//! state to another nanoservice (the Directive §7); it names things by the
//! caller's seam (a name, never an id the receiver mints); only derived
//! state crosses. Keep them plain data: no methods that reach a database.

#![allow(unused_imports)]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// basable:messages-modules-begin — the ledger's snapshot types, when a nanoservice owns the ledger
// basable:messages-modules-end

// basable:messages-begin — declare one struct per message in routing.yaml.

/// `CreateGroupRequest` — answered with `Group` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    // TODO: the fields.
}

/// `GetGroupRequest` — answered with `Group` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetGroupRequest {
    // TODO: the fields.
}

/// `ListGroupsRequest` — answered with `GroupList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListGroupsRequest {
    // TODO: the fields.
}

/// `CreateInviteRequest` — answered with `Invite` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInviteRequest {
    // TODO: the fields.
}

/// `AcceptInviteRequest` — answered with `Membership` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptInviteRequest {
    // TODO: the fields.
}

/// `ListMembersRequest` — answered with `MemberList` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMembersRequest {
    // TODO: the fields.
}

/// `AddMemberRequest` — answered with `Membership` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemberRequest {
    // TODO: the fields.
}

/// `RemoveMemberRequest` — answered with `Membership` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveMemberRequest {
    // TODO: the fields.
}

/// `SetMemberRoleRequest` — answered with `Membership` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMemberRoleRequest {
    // TODO: the fields.
}

/// `LeaveGroupRequest` — answered with `Membership` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveGroupRequest {
    // TODO: the fields.
}

/// `DeleteGroupRequest` — answered with `Group` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteGroupRequest {
    // TODO: the fields.
}

/// `DeleteAccountRequest` — answered with `AccountDeletion` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteAccountRequest {
    // TODO: the fields.
}

/// `CheckMembershipRequest` — answered with `MembershipCheck` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckMembershipRequest {
    // TODO: the fields.
}

/// `MemberRemoved` — a void fan-out event (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRemoved {
    // TODO: the fields.
}

/// `PurgeGroupObjectsRequest` — answered with `PurgeResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeGroupObjectsRequest {
    // TODO: the fields.
}

/// `PurgeGroupBlobsRequest` — answered with `PurgeResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeGroupBlobsRequest {
    // TODO: the fields.
}

/// `PurgeGroupKeysRequest` — answered with `PurgeResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeGroupKeysRequest {
    // TODO: the fields.
}

/// `PurgeUserDevicesRequest` — answered with `PurgeResult` (see routing.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeUserDevicesRequest {
    // TODO: the fields.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberList {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDeletion {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipCheck {
    // TODO: the fields.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurgeResult {
    // TODO: the fields.
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
