//! The API's messages as the wire has them (`proto/*/v1/*.proto`), one
//! `message!` per protobuf message with the same numbers, so the core
//! speaks Connect with its own codec and no generated code. Ids are UUID
//! text, times milliseconds, enums their lowercase names, as the protos
//! say. Only what the core calls is here.

#![allow(missing_docs)] // the fields are the protos'

use crate::message;

// ---- relay.v1 ----

message! {
    Entry, key 1 {
        1 => id: String,
        2 => value: Vec<u8>,
        3 => seq: i64,
        4 => deleted: bool,
    }
}

message! {
    PushEntry, key 1 {
        1 => id: String,
        2 => value: Vec<u8>,
        3 => base_seq: i64,
        4 => deleted: bool,
    }
}

message! {
    PushObjectsRequest {
        1 => device_id: String,
        2 => group_id: String,
        3 => objects: Vec<PushEntry>,
    }
}

message! {
    Accepted, key 1 {
        1 => id: String,
        2 => seq: i64,
    }
}

message! {
    Conflict, key 1 {
        1 => id: String,
        2 => current: Option<Entry>,
    }
}

message! {
    PushObjectsResponse {
        1 => accepted: Vec<Accepted>,
        2 => conflicts: Vec<Conflict>,
        3 => head_seq: i64,
        4 => behind_floor: bool,
        5 => evicted_through_seq: i64,
    }
}

message! {
    PullChangesRequest {
        1 => device_id: String,
        2 => group_id: String,
        3 => since_seq: i64,
        4 => limit: i32,
    }
}

message! {
    PullChangesResponse {
        1 => entries: Vec<Entry>,
        2 => head_seq: i64,
        3 => evicted_through_seq: i64,
        4 => min_acked_seq: i64,
        5 => has_min_acked_seq: bool,
        6 => more: bool,
    }
}

message! {
    AckSeqRequest {
        1 => device_id: String,
        2 => group_id: String,
        3 => seq: i64,
    }
}

message! {
    AckSeqResponse {
        1 => evicted_through_seq: i64,
        2 => min_acked_seq: i64,
        3 => has_min_acked_seq: bool,
    }
}

message! {
    GetHeadsRequest {
        1 => device_id: String,
        2 => group_ids: Vec<String>,
    }
}

message! {
    Head, key 1 {
        1 => group_id: String,
        2 => head_seq: i64,
        3 => evicted_through_seq: i64,
        4 => min_acked_seq: i64,
        5 => has_min_acked_seq: bool,
    }
}

message! {
    GetHeadsResponse {
        1 => heads: Vec<Head>,
    }
}

message! {
    HistoryRequest, key 1 {
        1 => id: String,
        2 => group_id: String,
        3 => requester_device: String,
        4 => from_seq: i64,
        5 => to_seq: i64,
        6 => object_id: String,
        7 => claimed_by: String,
        8 => claimed_at_ms: i64,
        9 => created_at_ms: i64,
        10 => status: String,
    }
}

message! {
    RequestHistoryRequest {
        1 => device_id: String,
        2 => group_id: String,
        3 => from_seq: i64,
        4 => to_seq: i64,
        5 => object_id: String,
    }
}

message! {
    RequestHistoryResponse {
        1 => request: Option<HistoryRequest>,
    }
}

message! {
    ListHistoryRequestsRequest {
        1 => device_id: String,
        2 => group_id: String,
    }
}

message! {
    ListHistoryRequestsResponse {
        1 => requests: Vec<HistoryRequest>,
    }
}

message! {
    ClaimHistoryRequest {
        1 => device_id: String,
        2 => request_id: String,
    }
}

message! {
    ClaimHistoryResponse {
        1 => request: Option<HistoryRequest>,
    }
}

message! {
    PushHistoryRequest {
        1 => device_id: String,
        2 => request_id: String,
        3 => entries: Vec<Entry>,
        4 => complete: bool,
    }
}

message! {
    PushHistoryResponse {
        1 => inserted: i64,
        2 => skipped: i64,
        3 => through_seq: i64,
        4 => complete: bool,
    }
}

// ---- device.v1 ----

message! {
    Device, key 1 {
        1 => id: String,
        2 => user_id: String,
        3 => public_key: Vec<u8>,
        4 => platform: String,
        5 => push_provider: String,
        6 => push_token: String,
        7 => created_at_ms: i64,
        8 => last_seen_at_ms: i64,
    }
}

message! {
    DeviceInfo, key 1 {
        1 => id: String,
        2 => user_id: String,
        3 => public_key: Vec<u8>,
        4 => platform: String,
    }
}

message! {
    KeyRequest, key 1 {
        1 => id: String,
        2 => group_id: String,
        3 => device: Option<DeviceInfo>,
        4 => invite_id: String,
        5 => invite_tag: Vec<u8>,
        6 => created_at_ms: i64,
    }
}

message! {
    WrappedKey, key 1 {
        1 => id: String,
        2 => group_id: String,
        3 => key_id: i32,
        4 => device_id: String,
        5 => wrapped: Vec<u8>,
        6 => wrapped_by_device: String,
        7 => created_at_ms: i64,
        8 => wrapped_by_public_key: Vec<u8>,
    }
}

message! {
    RegisterDeviceRequest {
        1 => device_id: String,
        2 => public_key: Vec<u8>,
        3 => platform: String,
    }
}

message! {
    RegisterDeviceResponse {
        1 => device: Option<Device>,
    }
}

message! {
    SetPushTokenRequest {
        1 => device_id: String,
        2 => push_provider: String,
        3 => push_token: String,
    }
}

message! {
    SetPushTokenResponse {
        1 => device: Option<Device>,
    }
}

message! {
    ListDevicesRequest {
        1 => device_id: String,
        2 => group_id: String,
    }
}

message! {
    ListDevicesResponse {
        1 => devices: Vec<DeviceInfo>,
    }
}

message! {
    RequestGroupKeysRequest {
        1 => device_id: String,
        2 => group_id: String,
        3 => invite_id: String,
        4 => invite_tag: Vec<u8>,
    }
}

message! {
    RequestGroupKeysResponse {
        1 => request: Option<KeyRequest>,
    }
}

message! {
    ListKeyRequestsRequest {
        1 => device_id: String,
    }
}

message! {
    ListKeyRequestsResponse {
        1 => requests: Vec<KeyRequest>,
    }
}

message! {
    WrappedKeyInput, key 1 {
        1 => key_id: i32,
        2 => wrapped: Vec<u8>,
    }
}

message! {
    FulfilKeyRequestRequest {
        1 => device_id: String,
        2 => request_id: String,
        3 => keys: Vec<WrappedKeyInput>,
    }
}

message! {
    FulfilKeyRequestResponse {
        1 => request: Option<KeyRequest>,
    }
}

message! {
    WrapFor, key 1 {
        1 => device_id: String,
        2 => wrapped: Vec<u8>,
    }
}

message! {
    PublishWrappedKeysRequest {
        1 => device_id: String,
        2 => group_id: String,
        3 => key_id: i32,
        4 => wraps: Vec<WrapFor>,
    }
}

message! {
    PublishWrappedKeysResponse {
        1 => keys: Vec<WrappedKey>,
    }
}

message! {
    GetWrappedKeysRequest {
        1 => device_id: String,
        2 => group_id: String,
    }
}

message! {
    GetWrappedKeysResponse {
        1 => keys: Vec<WrappedKey>,
    }
}

message! {
    RemoveDeviceRequest {
        1 => device_id: String,
    }
}

message! {
    RemoveDeviceResponse {
        1 => device: Option<Device>,
    }
}

// ---- group.v1 ----

message! {
    Group, key 1 {
        1 => id: String,
        2 => parent_group_id: String,
        3 => created_by: String,
        4 => created_at_ms: i64,
        5 => deleted_at_ms: i64,
    }
}

message! {
    Membership, key 1 {
        1 => id: String,
        2 => group_id: String,
        3 => user_id: String,
        4 => role: String,
        5 => joined_at_ms: i64,
    }
}

message! {
    Invite, key 1 {
        1 => id: String,
        2 => group_id: String,
        3 => code: String,
        4 => role: String,
        5 => created_by: String,
        6 => expires_at_ms: i64,
        7 => max_uses: i32,
        8 => uses: i32,
        9 => revoked: bool,
    }
}

message! {
    CreateGroupRequest {
        1 => parent_group_id: String,
    }
}

message! {
    CreateGroupResponse {
        1 => group: Option<Group>,
    }
}

message! {
    DeleteGroupRequest {
        1 => group_id: String,
    }
}

message! {
    DeleteGroupResponse {
        1 => group: Option<Group>,
    }
}

message! {
    DeleteAccountRequest {}
}

message! {
    DeleteAccountResponse {
        1 => purge_id: String,
    }
}

message! {
    GetGroupRequest {
        1 => group_id: String,
    }
}

message! {
    GetGroupResponse {
        1 => group: Option<Group>,
    }
}

message! {
    ListGroupsRequest {}
}

message! {
    ListGroupsResponse {
        1 => groups: Vec<Group>,
    }
}

message! {
    CreateInviteRequest {
        1 => group_id: String,
        2 => role: String,
        3 => expires_at_ms: i64,
        4 => max_uses: i32,
    }
}

message! {
    CreateInviteResponse {
        1 => invite: Option<Invite>,
    }
}

message! {
    AcceptInviteRequest {
        1 => code: String,
    }
}

message! {
    AcceptInviteResponse {
        1 => membership: Option<Membership>,
        2 => invite: Option<Invite>,
    }
}

message! {
    ListMembersRequest {
        1 => group_id: String,
    }
}

message! {
    ListMembersResponse {
        1 => members: Vec<Membership>,
    }
}

message! {
    AddMemberRequest {
        1 => group_id: String,
        2 => user_id: String,
        3 => role: String,
    }
}

message! {
    AddMemberResponse {
        1 => membership: Option<Membership>,
    }
}

message! {
    RemoveMemberRequest {
        1 => group_id: String,
        2 => user_id: String,
    }
}

message! {
    RemoveMemberResponse {
        1 => membership: Option<Membership>,
    }
}

message! {
    SetMemberRoleRequest {
        1 => group_id: String,
        2 => user_id: String,
        3 => role: String,
    }
}

message! {
    SetMemberRoleResponse {
        1 => membership: Option<Membership>,
    }
}

message! {
    LeaveGroupRequest {
        1 => group_id: String,
    }
}

message! {
    LeaveGroupResponse {
        1 => membership: Option<Membership>,
    }
}

// ---- storage.v1 ----

message! {
    PresignUploadRequest {
        1 => group_id: String,
        2 => key: String,
        3 => size: i64,
    }
}

message! {
    PresignUploadResponse {
        1 => key: String,
        2 => url: String,
        3 => expires_at_ms: i64,
    }
}

message! {
    PresignDownloadRequest {
        1 => group_id: String,
        2 => key: String,
    }
}

message! {
    PresignDownloadResponse {
        1 => key: String,
        2 => url: String,
        3 => expires_at_ms: i64,
    }
}
