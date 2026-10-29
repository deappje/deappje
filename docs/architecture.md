# Architecture

One binary, one Postgres, 4 nanoservices. This file is the
map; `routing.yaml` is the topology the build enforces; the plan under
`docs/01-deappje/` is the design record it was rendered from.

## Nanoservices and what they own

### `group`

Owns groups, memberships and invites; answers every membership check; drives group deletion and account deletion.

- Processing-object types (a lifecycle each, with spec + status tables, an
  adapter, a reconciler and a worker): `user_purge` (`upr_…`), `group_purge` (`gpr_…`).
- Plain tables: `group`, `membership`, `invite`.
- External systems: `kratos` (delete_identity: idempotent).
- Handles: `CreateGroupRequest` → `Group`, `GetGroupRequest` → `Group`, `ListGroupsRequest` → `GroupList`, `CreateInviteRequest` → `Invite`, `AcceptInviteRequest` → `Membership`, `ListMembersRequest` → `MemberList`, `AddMemberRequest` → `Membership`, `RemoveMemberRequest` → `Membership`, `SetMemberRoleRequest` → `Membership`, `LeaveGroupRequest` → `Membership`, `DeleteGroupRequest` → `Group`, `DeleteAccountRequest` → `AccountDeletion`, `CheckMembershipRequest` → `MembershipCheck`.
- Sends: `MemberRemoved`, `PurgeGroupObjectsRequest`, `PurgeGroupBlobsRequest`, `PurgeGroupKeysRequest`, `PurgeUserDevicesRequest`.
- API: `GroupService` (CreateGroup, DeleteGroup, DeleteAccount, GetGroup, ListGroups, CreateInvite, AcceptInvite, ListMembers, AddMember, RemoveMember, SetMemberRole, LeaveGroup).

### `device`

Owns devices, push tokens, wrapped group keys and key requests; sends silent pushes; tells relay when a device leaves a group.

- Plain tables: `device`, `wrapped_key`, `key_request`.
- External systems: `apns` (send_apns: idempotent), `fcm` (send_fcm: idempotent).
- Handles: `RegisterDeviceRequest` → `Device`, `SetPushTokenRequest` → `Device`, `ListDevicesRequest` → `DeviceList`, `RequestGroupKeysRequest` → `KeyRequest`, `ListKeyRequestsRequest` → `KeyRequestList`, `FulfilKeyRequestRequest` → `KeyRequest`, `PublishWrappedKeysRequest` → `WrappedKeyList`, `GetWrappedKeysRequest` → `WrappedKeyList`, `RemoveDeviceRequest` → `Device`, `GroupHeadAdvanced`, `MemberRemoved`, `HistoryRequested`, `HistoryReady`, `PurgeGroupKeysRequest` → `PurgeResult`, `PurgeUserDevicesRequest` → `PurgeResult`.
- Sends: `CheckMembershipRequest`, `DeviceLeftGroup`.
- API: `DeviceService` (RegisterDevice, SetPushToken, ListDevices, RequestGroupKeys, ListKeyRequests, FulfilKeyRequest, PublishWrappedKeys, GetWrappedKeys, RemoveDevice).

### `relay`

The relay — ciphertext per group until every holder has acked it, 672 hours at most, tombstones included; the counter; per-device acked cursor; history requests.

- Plain tables: `object`, `group_head`, `group_device`, `history_request`.
- Schedules (ticker workers): `evict_objects` every 1h, `expire_history_requests` every 1h.
- Handles: `PushObjectsRequest` → `PushResult`, `PullChangesRequest` → `ChangeSet`, `AckSeqRequest` → `AckResult`, `GetHeadsRequest` → `HeadList`, `RequestHistoryRequest` → `HistoryRequest`, `ListHistoryRequestsRequest` → `HistoryRequestList`, `ClaimHistoryRequest` → `HistoryRequest`, `PushHistoryRequest` → `PushHistoryResult`, `DeviceLeftGroup`, `PurgeGroupObjectsRequest` → `PurgeResult`.
- Sends: `CheckMembershipRequest`, `GroupHeadAdvanced`, `ObjectDeleted`, `HistoryRequested`, `HistoryReady`.
- API: `RelayService` (PushObjects, PullChanges, AckSeq, GetHeads, RequestHistory, ListHistoryRequests, ClaimHistory, PushHistory).

### `storage`

Presigns uploads and downloads under the caller's group prefix, deletes an object's files on its tombstone and a group's on purge, bounds upload bytes per group.

- Plain tables: `upload_usage`.
- Config catalogs: `slot`, `storage_quota`.
- External systems: `object_storage` (delete_object: idempotent).
- Schedules (ticker workers): `expire_usage` every 1h.
- Handles: `PresignUploadRequest` → `UploadGrant`, `PresignDownloadRequest` → `DownloadGrant`, `ObjectDeleted`, `PurgeGroupBlobsRequest` → `PurgeResult`.
- Sends: `CheckMembershipRequest`.
- API: `StorageService` (PresignUpload, PresignDownload).

## Topology

| Message | Response | Sent by | Handled by |
|---|---|---|---|
| `CreateGroupRequest` | `Group` |  | `group`  |
| `GetGroupRequest` | `Group` |  | `group`  |
| `ListGroupsRequest` | `GroupList` |  | `group`  |
| `CreateInviteRequest` | `Invite` |  | `group`  |
| `AcceptInviteRequest` | `Membership` |  | `group`  |
| `ListMembersRequest` | `MemberList` |  | `group`  |
| `AddMemberRequest` | `Membership` |  | `group`  |
| `RemoveMemberRequest` | `Membership` |  | `group`  |
| `SetMemberRoleRequest` | `Membership` |  | `group`  |
| `LeaveGroupRequest` | `Membership` |  | `group`  |
| `DeleteGroupRequest` | `Group` |  | `group`  |
| `DeleteAccountRequest` | `AccountDeletion` |  | `group`  |
| `CheckMembershipRequest` | `MembershipCheck` | `device` `relay` `storage`  | `group`  |
| `MemberRemoved` | (event) | `group`  | `device`  |
| `PurgeGroupObjectsRequest` | `PurgeResult` | `group`  | `relay`  |
| `PurgeGroupBlobsRequest` | `PurgeResult` | `group`  | `storage`  |
| `PurgeGroupKeysRequest` | `PurgeResult` | `group`  | `device`  |
| `PurgeUserDevicesRequest` | `PurgeResult` | `group`  | `device`  |
| `RegisterDeviceRequest` | `Device` |  | `device`  |
| `SetPushTokenRequest` | `Device` |  | `device`  |
| `ListDevicesRequest` | `DeviceList` |  | `device`  |
| `RequestGroupKeysRequest` | `KeyRequest` |  | `device`  |
| `ListKeyRequestsRequest` | `KeyRequestList` |  | `device`  |
| `FulfilKeyRequestRequest` | `KeyRequest` |  | `device`  |
| `PublishWrappedKeysRequest` | `WrappedKeyList` |  | `device`  |
| `GetWrappedKeysRequest` | `WrappedKeyList` |  | `device`  |
| `RemoveDeviceRequest` | `Device` |  | `device`  |
| `GroupHeadAdvanced` | (event) | `relay`  | `device`  |
| `HistoryRequested` | (event) | `relay`  | `device`  |
| `HistoryReady` | (event) | `relay`  | `device`  |
| `DeviceLeftGroup` | (event) | `device`  | `relay`  |
| `PushObjectsRequest` | `PushResult` |  | `relay`  |
| `PullChangesRequest` | `ChangeSet` |  | `relay`  |
| `AckSeqRequest` | `AckResult` |  | `relay`  |
| `GetHeadsRequest` | `HeadList` |  | `relay`  |
| `RequestHistoryRequest` | `HistoryRequest` |  | `relay`  |
| `ListHistoryRequestsRequest` | `HistoryRequestList` |  | `relay`  |
| `ClaimHistoryRequest` | `HistoryRequest` |  | `relay`  |
| `PushHistoryRequest` | `PushHistoryResult` |  | `relay`  |
| `ObjectDeleted` | (event) | `relay`  | `storage`  |
| `PresignUploadRequest` | `UploadGrant` |  | `storage`  |
| `PresignDownloadRequest` | `DownloadGrant` |  | `storage`  |

`api` sends every API method's request; it handles nothing.

## Deployment shape

- `app/`: the binary, `replicas: 2`, every nanoservice in every replica.
- `frontend/react/`: the React app (Vite, built by rules_js into `dist/`), uploaded by the workflow as the `static` artifact and served at `/` from the environment's public bucket (no image, no pod), calling the API through Connect clients generated from `proto/` (`frontend/react/README.md`).
- `k8s/postgres/`: one CNPG cluster, 1 instance(s), a schema and a role per stateful nanoservice (`nano_group`, `nano_device`, `nano_relay`, `nano_storage`).
- `k8s/kratos/`: Ory Kratos identity (passwordless email code + passkeys + Sign in with Basable, from the platform's `basable-login` Secret), its own database. The sign-in pages Kratos sends the browser to (`/auth/login`, `/auth/register`, `/auth/settings`, `/auth/error`) are drawn by the frontend from the flow's own nodes (`frontend/react/src/Auth.tsx`).
- `k8s/httproute.yaml`: `/api/` → app, `/.ory/` → Kratos; `/` is the platform's own route to the bucket, never claimed here.

See `docs/platform-contract.md` for the rules the platform enforces on this
tree and `docs/ci.md` for the workflow.
