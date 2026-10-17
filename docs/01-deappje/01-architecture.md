# Architecture

One Rust binary, four nanoservices, one Postgres database with a schema per nanoservice, blobs
in the platform's private bucket, Kratos for identity. The server never holds a group key and
never reads a payload: every `value` is ciphertext, every blob is ciphertext, and the plaintext
columns are the minimum needed to route, order, authorise and expire.

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
- **Trust rule**: the server never seeds trust. A device holds a group's keys only because a
  person on a device that already holds them approved its key request (the requester's own
  devices that hold the key approve silently). A rotation wraps the new key only for devices that
  hold the previous one. So a device row the server inserted on its own never receives a key: no
  human sees it, and `ListDevicesRequest` for a group lists only key holders. The approver sees
  the requesting device's platform and the fingerprint of its public key, which the requester
  reads off their own screen.
- **External**: APNs and FCM, one `send_push` call each, strategy `idempotent`: the only push the
  server sends is a silent "group X advanced" or "key request pending" wake-up, and a duplicate is
  harmless. Directive §6.
- **Handles**: device registration, push token, device listing (so members can wrap keys for
  new devices), key requests and fulfilment, publishing a rotated key set, fetching wrapped keys;
  plus three void events: `GroupHeadAdvanced` (from `ledger`), `MemberRemoved` (from `group`),
  `BlobRequested` (from `storage`), each turned into pushes; and `PurgeUserDevicesRequest` from
  `group`'s account purge.
- **API**: `DeviceService`.

### `ledger`

Owns the object log. Plain executor; the one piece of concurrency control in the system is here.

- **Tables**: `object` (id, group, parent, type, schema version, key id, value, blob id, version,
  seq, creator device, deleted flag and time), `group_head` (group, head seq): the counter row.
  Directive §2, §7 (serialize on a counter row under `FOR UPDATE`).
- **Push**: one transaction per batch: lock `group_head`, then per object compare the caller's
  `base_version` with the stored version (absent ⇒ 0); on match write the row with
  `version + 1` and `seq = head + 1`, advance head; on mismatch report a conflict for that object
  carrying the current row, and leave it. A delete is a push with `deleted = true`: the row keeps id, group, parent, type
  and seq, drops `value` and `blob_id`, and the same transaction tombstones every descendant by
  `parent_id`. Commit, then send `GroupHeadAdvanced` and, per deleted object with a blob,
  `ObjectDeleted` (both after the transaction: Directive invariant 7).
- **Pull**: `PullChangesRequest` returns rows with `seq > cursor` in seq order, tombstones
  included, paged; `BackfillRequest` returns rows of one type with `seq < before`, newest first,
  limited; `GetHeadsRequest` returns `(group, head)` for every group the caller is in;
  `ListObjectIdsRequest` returns `(id, version, deleted)` for a group for full reconciliation.
- **Schedule**: `gc_tombstones` daily, deletes tombstone rows older than the retention window.
  A client whose cursor is older than the window is told so and reconciles.
- **Sends**: `CheckMembershipRequest`, `CheckBlobRequest` (a pushed object may only reference a
  blob of its own group that is uploaded), `GroupHeadAdvanced`, `ObjectDeleted`.
- **API**: `LedgerService`.

### `storage`

Owns blobs. This is the one nanoservice with a processing-object type, because a blob has a
lifecycle the server drives against a third party: wait for the upload to land in the bucket,
expire it after its retention, delete it from the bucket, and wait for a re-upload when a member
asks for it again.

- **Processing object `blob`** (prefix `blb`). Spec: group, object, kind, storage key, expected
  hash, uploader device, retention days. Status: phase (`pending_upload`, `available`, `expired`,
  `requested`), size, uploaded at, last access, expires at, requested at. Directive §3, §5, §6.
- **Reconcile pass**: deleting ⇒ confirm absence in the bucket, delete if present, `Delete`.
  `pending_upload` ⇒ `HEAD` the key; present and hash matches ⇒ `available` with
  `expires_at = now + retention`, `Converged.after(retention)`; absent ⇒ `Converged.after(5m)`
  (the `ConfirmUploadRequest` handler nudges it, so the poll is a fallback). `available` past
  `expires_at` ⇒ `delete_object`, phase `expired`, `Settled`. `requested` ⇒ `Settled` until
  `ConfirmUploadRequest` nudges it back through `pending_upload`. Downloads and re-upload
  requests are unclaimed input, so the request path never writes status (invariant 3): it writes
  `last_access_at` / `requested_at` into the plain `blob_access` side table and nudges; the
  pass reads that table and extends `expires_at` or moves to `requested`.
- **Side table `blob_holder`**: which devices downloaded a blob. Written by `GetDownloadRequest`,
  removed by `DropBlobHoldRequest` (a phone evicting under storage pressure, or woken for bytes it
  no longer has). `BlobRequested` wakes every holder and the uploader, since after the cache
  window the bucket no longer has the bytes and any device that viewed them does. After each
  download the pass asks `device` `CheckHolderCoverageRequest` → `HolderCoverage { complete }`;
  complete means every key-holding device of the group has the blob and the bucket copy is
  deleted at once.
- **Config type `blob_kind`**: `full` (2 days, 25 MB max), `thumbnail` (3650 days, 200 KB max),
  `attachment` (2 days, 25 MB max). Retention after upload or last download, and size cap, per
  kind. **Config type `storage_quota`**: `max_group_bytes` 20 GiB; `CreateUploadRequest` refuses
  when the group's blobs in `pending_upload` or `available` already sum to it. Edited by a human.
- **External**: the platform's object storage. `delete_object` is the only effect
  (`idempotent`: S3 delete of a missing key succeeds). Presigning and `HEAD` are reads.
- **Handles**: `CreateUploadRequest` (creates the object in `pending_upload`, returns a presigned
  PUT moved onto `S3_PRIVATE_URL`), `ConfirmUploadRequest` (nudge), `GetDownloadRequest`
  (presigned GET if `available`, otherwise the phase), `RequestBlobRequest` (`expired` ⇒
  `requested`, sends `BlobRequested`), `CheckBlobRequest` (from `ledger`), `ObjectDeleted` (void,
  from `ledger`: `mark_deleted`).
- **Sends**: `CheckMembershipRequest`, `BlobRequested`.
- **API**: `StorageService`.

## Values are CRDT documents (client core)

Every `value` is one `yrs` document (the Rust port of Yjs), encrypted. The server has one rule,
compare-and-set on `version` with the current row returned on a conflict, and the client core has
one policy for every type:

1. Decrypt the returned row and apply its state to the local document. The merge is commutative
   and idempotent, so order and repeats do not matter.
2. Validate the typed projection against the type's invariants that live in inputs (an event
   ending before it starts, a vote naming an option deleted concurrently, a participant no longer
   in the group). A failure does not block the push; it flags the object "needs attention" for a
   person to fix.
3. Encode the full state, encrypt, push again with the returned `version` as `base_version`. A
   few immediate retries, then the object waits in the outbox for the next sync. The loop
   converges because every retry carries strictly more of the group's edits.

Deletes still win: a tombstone in the returned row ends the loop and the local edit is dropped.

### Document shape per type

A `Map` at the root; each key is a field of the type. Scalars are last-writer-wins registers (two
concurrent edits to the same key pick one deterministically, edits to different keys both
survive). Lists (todo items, poll options) are a `Map` keyed by item id, each item a nested map
with a `pos` key holding a fractional index (a string that sorts between its neighbours), and the
order is derived on projection. A move is one LWW write to `pos`, so two concurrent moves of the
same item pick one position and a concurrent edit of the item's fields survives the move. A `yrs`
`Array` is not used for user-ordered lists: it has no move, so a move is delete plus insert and
two concurrent moves leave two copies. Prose (note body, post text) is `Text`
with attributes. Documents store inputs, never derived values: an expense stores amount, payer,
participants and a split rule (`equal`, `weights`, `exact`), and the shares are computed in the
projection, so a merge cannot produce shares that do not sum (in `exact` mode the total is the
sum of the stored amounts). Balances, tallies and sort orders are derived the same way. The few
inputs that must change together live under one key as a nested map replaced atomically.

The encrypted bytes are `Payload { uint32 format; bytes state }`, `format = 1` meaning the `yrs`
v1 state encoding. Deleted content is garbage-collected inside the document, so an object stays
about the size of its fields plus roughly a hundred bytes of CRDT metadata. Offline edits
accumulate locally and go up as one state on the next sync; the server sees one ciphertext per
push.

### Key schemas and versions

Per type the core holds a key schema: key name, kind (`string`, `int`, `bool`, `bytes`, `map`,
`array`, `text`), required, default. `schema_version` in plaintext metadata says which schema a
writer used. Rules: add keys freely; never repurpose a key; a breaking change is a new type
number. An old client reads a newer document by ignoring keys it does not know (the CRDT keeps
them) and may write to it, touching only keys it knows. Upgrade steps are declarative key
operations: add with default, rename by copy (the old key stays until every writer is past the
old version), drop. No executable converters, so no code travels. The projection from document to
typed struct is where defaults are filled and invariants checked.

Why `yrs` and not automerge: automerge keeps the full history of every object forever, so every
edit grows the value; `yrs` garbage-collects, has native rich text, and compiles to wasm for the
browser shell. Time travel is not a requirement.

## Deployment shape

- One binary, `replicas: 1` in preview and `2` in live (deviation from the default of 2
  everywhere, for cost; the frameworks are multi-replica safe so live gets two).
- `frontend: react`: the web client is an application, not a page, with local-first state in a
  wasm build of the shared Rust core and React rendering it. The core is a `core/` crate in this
  repository's Bazel workspace (sync, store, `yrs` documents and projections, keyring, crypto, key
  schemas), built to wasm for the web client in the same workflow; the Swift and Kotlin
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
| GroupHeadAdvanced | (void) | ledger | device |
| BlobRequested | (void) | storage | device |
| CheckHolderCoverageRequest | HolderCoverage | storage | device |
| PushObjectsRequest | PushResult | api | ledger |
| PullChangesRequest | ChangeSet | api | ledger |
| BackfillRequest | ChangeSet | api | ledger |
| GetHeadsRequest | HeadList | api | ledger |
| ListObjectIdsRequest | ObjectIdList | api | ledger |
| ObjectDeleted | (void) | ledger | storage |
| CheckBlobRequest | Blob | ledger | storage |
| CreateUploadRequest | UploadGrant | api | storage |
| ConfirmUploadRequest | Blob | api | storage |
| GetDownloadRequest | DownloadGrant | api | storage |
| RequestBlobRequest | Blob | api | storage |
| DropBlobHoldRequest | Blob | api | storage |

Every cross-nanoservice message binds Directive §7. The only derived state that crosses is a
boolean (`MembershipCheck`, `Blob.phase`) or an id; no nanoservice re-derives another's rule.

## What is deliberately not on the server

The CRDT merge, validation of payloads, invariants inside a type (one vote per poll, shares
summing to the total), derived state (balances, tallies), schema conversion, thumbnails, resizing, map tiles,
geocoding. All of it is the client's, because all of it needs plaintext.
