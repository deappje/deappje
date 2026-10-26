# Architecture

One Rust binary, four nanoservices, one Postgres database with a schema per nanoservice, blobs
in the platform's private bucket, Kratos for identity. The server is a relay, not a store: member
devices hold the data, the server keeps 48 hours of it so devices can catch up, and anything
older is fetched from a member device on request. It never holds a group key and never reads a
payload: every `value` is ciphertext, every blob is ciphertext, and the plaintext columns are the
minimum needed to route, order, authorise and expire.

## Nanoservices

### `group`

Owns groups and who is in them, and two lifecycles: a group's deletion and a user's account
deletion.

- **Tables**: `group` (id, `parent_group_id`, creator, timestamps), `membership` (group, user,
  role, joined), `invite` (group, code, creator, expiry, max uses, uses, revoked). Directive §2.
- **Invariants**: a user is in a group at most once; a member of a subgroup is a member of its
  parent (checked on accept, cascaded on leave/remove); only an admin removes members or revokes
  invites; the creator is the first admin.
- **Processing object `group_purge`** (prefix `gpr`): created by `DeleteGroupRequest`, one per
  group being deleted. Its pass drives `ledger`, `storage` and `device` to hold nothing of the
  group, by direct requests (`PurgeGroupObjectsRequest`, `PurgeGroupBlobsRequest`,
  `PurgeGroupKeysRequest`, each → `PurgeResult { remaining }`), retrying until all three report
  zero, then removes the group row and returns `Delete`. A purge must finish, so it is a
  reconciler with requests, not an event (Directive §7: "an event is not a cascade"). Subgroups
  are purged first: deleting a parent creates one purge object per subgroup and the parent's pass
  waits for them. Directive §3, §5.
- **Processing object `user_purge`** (prefix `upr`): created by `DeleteAccountRequest`, one per
  user. Its pass leaves one group per pass (promoting the longest-standing member where the user
  was the only admin, creating a `group_purge` where they were the only member, sending
  `MemberRemoved` so keys rotate), then sends `PurgeUserDevicesRequest` → `PurgeResult` to
  `device` until zero, then calls Kratos admin `delete_identity` (external, `idempotent`), then
  `Delete`. Directive §3, §5, §6.
- **Handles**: `CreateGroupRequest`, `GetGroupRequest`, `ListGroupsRequest`, `CreateInviteRequest`,
  `AcceptInviteRequest`, `ListMembersRequest`, `RemoveMemberRequest`, `LeaveGroupRequest`,
  `DeleteGroupRequest`, `DeleteAccountRequest`, and `CheckMembershipRequest` → `MembershipCheck`,
  the authorisation question every other nanoservice asks before it touches anything in a group.
- **Sends**: `MemberRemoved` (void event) so `device` can revoke wrapped keys and push a rotation
  prompt; the three group purge requests above; `PurgeUserDevicesRequest`.
- **External**: Kratos admin API, `delete_identity`, strategy `idempotent` (a 404 on a repeat is
  success). The only server-side effect against identity.
- **API**: `GroupService`.

### `device`

Owns devices, their public keys, push tokens, the wrapped group keys, and key requests. Plain
executor. Group keys are opaque bytes to this nanoservice: it stores what a member's device wrapped
for another device and hands it back.

- **Tables**: `device` (user, public key, platform, push token), `wrapped_key` (group, key id,
  device, ciphertext, wrapped by), `key_request` (group, device, status). Directive §2.
- **Invariants**: a device belongs to one user; a wrapped key exists only for a device whose user
  is a member (checked through `CheckMembershipRequest` at write time); on `MemberRemoved` the
  removed user's wrapped keys for that group are deleted and every other member's devices are
  pushed so one of them rotates.
- **Also**: `DeviceLeftGroup` to `ledger` whenever a device loses its keys for a group, so a
  gone device stops blocking garbage collection.
- **Trust rule**: the server never seeds trust. A device holds a group's keys only because a
  person on a device that already holds them approved its key request, by scanning the new
  device's fingerprint QR or comparing the code. That includes the user's own new devices: a
  silent same-user approval would let a server-inserted row under the victim's user id obtain
  keys from the victim's own phone. A rotation wraps the new key only for devices that
  hold the previous one. So a device row the server inserted on its own never receives a key: no
  human sees it, and `ListDevicesRequest` for a group lists only key holders. The approver sees
  the requesting device's platform and the fingerprint of its public key, which the requester
  reads off their own screen. The exchange itself is HPKE Auth mode over X25519 (`02-device.md`),
  and clients pin peer device keys.
- **External**: APNs and FCM, one `send_push` call each, strategy `idempotent`: the only push the
  server sends is a silent "group X advanced" or "key request pending" wake-up, and a duplicate is
  harmless. Directive §6.
- **Handles**: device registration, push token, device listing (so members can wrap keys for
  new devices), key requests and fulfilment, publishing a rotated key set, fetching wrapped keys;
  plus three void events: `GroupHeadAdvanced` (from `ledger`), `MemberRemoved` (from `group`),
  `BlobRequested` (from `storage`), `HistoryRequested` and `HistoryReady` (from `ledger`), each
  turned into pushes; `PurgeUserDevicesRequest` from `group`'s account purge; `RemoveDeviceRequest`
  so a user can drop a dead device, which otherwise blocks garbage collection in its groups.
- **Sends**: `CheckMembershipRequest`; `DeviceLeftGroup` to `ledger` whenever a device loses its
  keys for a group (member removed, device removed, account purged).
- **API**: `DeviceService`.

### `ledger`

Owns the relay. Plain executor with three hourly sweeps; the one piece of concurrency control in
the system is here. Member devices are the store: the server keeps a live row for 48 hours and a
tombstone until every device has acked it.

- **Tables**: `object` (id, group, parent, type, schema version, key id, value, blob id, version,
  seq, creator device, `deleted`), `group_head` (group, head seq, `evicted_through_seq`): the
  counter row; `group_device` (group, device, `acked_seq`); `history_request`. Directive §2, §7
  (serialize on a counter row under `FOR UPDATE`).
- **Push**: one transaction per batch: lock `group_head`; per object compare `base_version` with
  the stored version (no row ⇒ take the client's word: deleted objects are never evicted, only
  hard-deleted after this device acked past them, so nothing can be resurrected, and the first
  re-creation of an evicted row makes every later stale base conflict), on match write
  `version + 1` and `seq = head + 1`, on mismatch report a conflict carrying the current row. A
  delete tombstones the subtree by `parent_id` in the same transaction. A tombstone's deletion
  mark is its own seq; there is no second counter and no "pull before you push" check. Commit,
  then `GroupHeadAdvanced` and, per deleted object with a blob, `ObjectDeleted` (invariant 7).
- **Pull and ack**: `PullChangesRequest` returns entries with `seq > cursor`, tombstones included,
  with the minimum acked cursor and the eviction floor; a device whose cursor is below the floor
  asks a member device for the gap. After applying a pull the device `AckSeq`s the head it
  received; the server keeps the minimum per group, hard-deletes tombstones at or below it, and
  hands the minimum back so devices drop their own tombstones. An ack that goes backwards means a
  restored backup and forces a resync. `GetHeadsRequest` returns `(group, head, min ack)`.
- **History**: `RequestHistoryRequest` records a range and wakes every key-holding device through
  `device`; one of them uploads a `bundle` blob; `storage` reports `BundleReady`; the requester is
  woken. No server-side retries: the requester asks again. This is how a new member, a new device
  or a long-offline device gets anything older than 48 hours.
- **Sweeps**: `evict_objects` (live rows older than 48h, floor advances), `gc_tombstones`
  (`seq ≤` min acked cursor), `expire_history_requests` (48h).
- **Sends**: `CheckMembershipRequest`, `CheckBlobRequest`, `GroupHeadAdvanced`, `ObjectDeleted`,
  `HistoryRequested`, `HistoryReady`.
- **API**: `LedgerService`.

### `storage`

Owns blobs: a row per encrypted file, who holds a copy, and its 48 hours in the private bucket.
Plain executor with one hourly sweep; no state machine.

- **Tables**: `blob` (group, kind, key, sha256, size, uploader, `request_id` for bundles,
  `in_bucket`, `expires_at`), `blob_holder` (blob, device). Directive §2.
- **Config type `blob_kind`**: `full` (2 days, 25 MB), `thumbnail` (3650 days, 200 KB),
  `attachment` (2 days, 25 MB), `bundle` (2 days, 256 MB). **`storage_quota`**: 20 GiB per group
  of what is in the bucket. Both edited by a human.
- **External**: `delete_object`, `idempotent`, dispatched from handlers and the sweep; a failed
  delete is retried by the next sweep. Presigning and `HEAD` are reads.
- **Handles**: `CreateUploadRequest` (row + presigned PUT), `ConfirmUploadRequest` (`HEAD`, size
  check, `in_bucket`, expiry; a `bundle` reports `BundleReady` to `ledger`), `ReuploadBlobRequest`
  (a holder re-uploads to the same key), `GetDownloadRequest` (presigned GET if in the bucket,
  record the holder, delete the bucket copy once every key-holding device has it, else `gone`),
  `RequestBlobsRequest` (wake the holders of a batch of blobs), `DropBlobHoldRequest`,
  `CheckBlobRequest` (from `ledger`), `ObjectDeleted` (from `ledger`), `PurgeGroupBlobsRequest`.
- **Sweep**: `expire_blobs`, hourly: delete expired bucket copies; rows and holders stay as the
  registry for re-requests.
- **Sends**: `CheckMembershipRequest`, `CheckHolderCoverageRequest`, `BlobRequested`,
  `BundleReady`.
- **API**: `StorageService`.

## What the server computes

Five things, and nothing else: whether a user is a member of a group; the next sequence number;
whether a version matches; the minimum acked cursor of a group; whether a blob is past its
retention. Every other byte is relayed, never read.

## Values and conflicts (client core)

Every `value` is a protobuf message of its type, encrypted (`06-object-types.md`). The server has
one rule, compare-and-set on `version` with the current row returned on a conflict, and the
client core has one way to handle it:

1. Keep, per locally edited object, the row it was edited from and the set of fields changed.
2. On a conflict, decrypt the returned row, re-apply the changed fields onto it, validate the
   result against the type's invariants (an event ending before it starts), and push again with
   the returned `version` as `base_version`. Where both sides changed the same field, the later
   push wins. A few immediate retries, then the object waits in the outbox for the next sync.
3. A tombstone in the returned row ends the loop: the local edit is dropped and the client shows
   "deleted".
4. A child whose parent the device holds as a tombstone is dead, whatever its own row says; the
   first client to notice pushes its tombstone. This is what covers a device that commented on a
   photo someone deleted while it was offline: the server does not check parents, since they may
   already be evicted.

No conflicted copies and no merge policies: a friends-group app edits small objects one field at
a time, and field-level rebase covers it.

**Notes are the exception, and the CRDT lives in the payload, not in the server.** A `note` is a
parent object (title) plus append-only `note_update` children, each holding one `yrs` (Yjs)
update, the binary delta an editing session produced. Children are never edited, so they never
conflict; two people typing offline produce two children. Opening a note loads its children and
applies them in any order into one `yrs` document; deleting the note tombstones the children by
cascade. A device batches an editing session into one child, so a note grows by one small object
per session. Compaction (one device writes a snapshot child and deletes the ones it covers) is a
later optimisation; nothing depends on it.

### Schemas and versions

`schema_version` in plaintext metadata says which schema the writer used. Add fields, never
repurpose a field number; a breaking change is a new type number. An old client reads a newer
object by ignoring fields it does not know and shows it, but never writes to an object whose
`schema_version` is above its own for that type, so no field is ever dropped by a re-serialise.
Upgrades fill defaults for new fields on read; there are no converters and no code travels.

## Deployment shape

- One binary, `replicas: 1` in preview and `2` in live (deviation from the default of 2
  everywhere, for cost; the frameworks are multi-replica safe so live gets two).
- `frontend: react`: the web client is an application, not a page, with local-first state in a
  wasm build of the shared Rust core and React rendering it. The browser is a full device of the
  user (own device key, paired by QR, syncs from the server), never a mirror of the phone; login
  is a passkey, which with WebAuthn cross-device authentication is itself a QR scanned by the
  phone. Browser storage: object and blob ciphertext at rest in the Origin Private File System
  (blobs byte for byte as the bucket holds them, so the browser is a real holder), decrypted into
  memory by the core at startup; the device key is a non-extractable WebCrypto key to which the
  core delegates the X25519 step of HPKE; `navigator.storage.persist()` is requested; eviction by
  the browser only costs a dropped hold, a re-pair, or a history request. The core is a `core/`
  crate in this repository's Bazel workspace (sync, local store, conflict rebase, the note CRDT,
  keyring, crypto, schemas), built to wasm for the web client in the same workflow; the Swift and Kotlin
  repositories pin a commit of this repository and build the crate with UniFFI. Server contract
  and client core are versioned together. Served from an nginx-unprivileged
  container by the scaffold; `05-deployment.md` records the later move to the public bucket
  (`website: true`), which the environment already has for blobs.
- CloudNativePG, `instances: 1`, one application database with schemas `group`, `device`,
  `ledger`, `storage`; a separate `kratos` database.
- Kratos: email code and passkeys. Users are Kratos identity ids.
- API: protobuf over Connect, four services. The Rust core on every client speaks it.
- Object storage declared in both environments: blobs in the private bucket, nothing in the public
  bucket until the web client moves there.

## Topology

| Message | Response | Sender(s) | Handler |
|---|---|---|---|
| CreateGroupRequest | Group | api | group |
| GetGroupRequest | Group | api | group |
| ListGroupsRequest | GroupList | api | group |
| CreateInviteRequest | Invite | api | group |
| AcceptInviteRequest | Membership | api | group |
| ListMembersRequest | MemberList | api | group |
| RemoveMemberRequest | Membership | api | group |
| LeaveGroupRequest | Membership | api | group |
| DeleteGroupRequest | Group | api | group |
| DeleteAccountRequest | AccountDeletion | api | group |
| CheckMembershipRequest | MembershipCheck | device, ledger, storage | group |
| MemberRemoved | (void) | group | device |
| PurgeGroupObjectsRequest | PurgeResult | group | ledger |
| PurgeGroupBlobsRequest | PurgeResult | group | storage |
| PurgeGroupKeysRequest | PurgeResult | group | device |
| PurgeUserDevicesRequest | PurgeResult | group | device |
| RegisterDeviceRequest | Device | api | device |
| SetPushTokenRequest | Device | api | device |
| ListDevicesRequest | DeviceList | api | device |
| RequestGroupKeysRequest | KeyRequest | api | device |
| ListKeyRequestsRequest | KeyRequestList | api | device |
| FulfilKeyRequestRequest | KeyRequest | api | device |
| PublishWrappedKeysRequest | WrappedKeyList | api | device |
| GetWrappedKeysRequest | WrappedKeyList | api | device |
| RemoveDeviceRequest | Device | api | device |
| GroupHeadAdvanced | (void) | ledger | device |
| HistoryRequested | (void) | ledger | device |
| HistoryReady | (void) | ledger | device |
| BlobRequested | (void) | storage | device |
| CheckHolderCoverageRequest | HolderCoverage | storage | device |
| DeviceLeftGroup | (void) | device | ledger |
| PushObjectsRequest | PushResult | api | ledger |
| PullChangesRequest | ChangeSet | api | ledger |
| AckSeqRequest | AckResult | api | ledger |
| GetHeadsRequest | HeadList | api | ledger |
| RequestHistoryRequest | HistoryRequest | api | ledger |
| ListHistoryRequestsRequest | HistoryRequestList | api | ledger |
| BundleReady | (void) | storage | ledger |
| ObjectDeleted | (void) | ledger | storage |
| CheckBlobRequest | Blob | ledger | storage |
| CreateUploadRequest | UploadGrant | api | storage |
| ConfirmUploadRequest | Blob | api | storage |
| ReuploadBlobRequest | UploadGrant | api | storage |
| GetDownloadRequest | DownloadGrant | api | storage |
| RequestBlobsRequest | (void) | api | storage |
| DropBlobHoldRequest | (void) | api | storage |

Every cross-nanoservice message binds Directive §7. The only derived state that crosses is a
boolean or a number (`MembershipCheck`, `Blob.present`, `HolderCoverage`) or an id; no nanoservice re-derives another's rule.

## What is deliberately not on the server

Conflict rebase, the note CRDT, validation of payloads, invariants inside a type (one vote per
poll), derived state (balances, tallies), schema conversion, thumbnails, resizing, map tiles,
geocoding. All of it is the client's, because all of it needs plaintext.
