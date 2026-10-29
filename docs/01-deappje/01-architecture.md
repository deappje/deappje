# Architecture

One Rust binary, four nanoservices, one Postgres database with a schema per nanoservice, blobs
in the platform's private bucket, Kratos for identity with Sign in with Basable as its only
method. The server is a relay, not a store: member devices hold the data, the server keeps up to
672 hours of it so devices can catch up, and anything older is fetched from a member device on
request. It never holds a group key and never reads a
payload: every `value` is ciphertext, every blob is ciphertext, and the plaintext columns are the
minimum needed to route, order, authorise and expire.

## Nanoservices

### `group`

Owns groups and who is in them, and two lifecycles: a group's deletion and a user's account
deletion.

- **Tables**: `group` (id, `parent_group_id`, creator, timestamps), `membership` (group, user,
  role, joined), `invite` (group, code, creator, expiry, max uses, uses, revoked). Directive §2.
- **Invariants**: a user is in a group at most once; a member of a subgroup is a member of its
  parent (checked on accept and on `AddMemberRequest`, cascaded on leave/remove); only an admin removes members, sets roles
  or revokes invites; the creator is the first admin. Roles are `admin`, `member`, `viewer`; a
  viewer holds keys and reads, and the server refuses its pushes to the log and its approval of
  another user's device (a file is inert without an object, so uploads are not role-checked). `MembershipCheck` carries the role,
  so each of those is one comparison in the handler that cares.
- **Processing object `group_purge`** (prefix `gpr`): created by `DeleteGroupRequest`, one per
  group being deleted. Its pass drives `sync`, `storage` and `device` to hold nothing of the
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
  `AcceptInviteRequest`, `ListMembersRequest`, `RemoveMemberRequest`, `SetMemberRoleRequest`,
  `LeaveGroupRequest`, `DeleteGroupRequest`, `DeleteAccountRequest`, and `CheckMembershipRequest` → `MembershipCheck`,
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
  device, ciphertext, wrapped by), `key_request` (group, device, invite id and tag). Directive §2.
- **Invariants**: a device belongs to one user; a wrapped key exists only for a device whose user
  is a member (checked through `CheckMembershipRequest` at write time); on `MemberRemoved` the
  removed user's wrapped keys for that group are deleted and every other member's devices are
  pushed so one of them rotates.
- **Also**: `DeviceLeftGroup` to `sync` whenever a device loses its keys for a group, so a
  gone device is no longer a candidate for history requests.
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
- **External**: APNs and FCM, `send_apns` and `send_fcm`, strategy `idempotent`: the only push the
  server sends is a silent "group X advanced" or "key request pending" wake-up, and a duplicate is
  harmless. Directive §6.
- **Handles**: device registration, push token, device listing (so members can wrap keys for
  new devices), key requests and fulfilment, publishing a rotated key set, fetching wrapped keys;
  plus the void events `GroupHeadAdvanced`, `HistoryRequested` and `HistoryReady` (from `sync`)
  and `MemberRemoved` (from `group`), each turned into pushes; `PurgeUserDevicesRequest` from `group`'s account purge; `RemoveDeviceRequest`
  so a user can drop a dead device, which otherwise stays a candidate for history requests.
- **Sends**: `CheckMembershipRequest`; `DeviceLeftGroup` to `sync` whenever a device loses its
  keys for a group (member removed, device removed, account purged).
- **API**: `DeviceService`.

### `sync`

Owns the relay. Plain executor with two hourly sweeps; the one piece of concurrency control in
the system is here. Member devices are the store, and every holder holds the whole group: the
server keeps a row, tombstones included, until every device has acked it, 672 hours at most.

- **Tables**: `object` (id, group, value, seq, `deleted`; the type, schema
  version and key id are a plaintext header inside the value, the parent and any file
  references are inside its ciphertext), `group_head` (group, head seq, `evicted_through_seq`): the
  counter row; `group_device` (group, device, `acked_seq`); `history_request`. Directive §2, §7
  (serialize on a counter row under `FOR UPDATE`).
- **Push**: one transaction per batch: lock `group_head`; refuse the batch as `behind_floor`
  when the device is not a holder, whatever the batch holds: no `group_device` row, or an ack
  below `evicted_through_seq`, means it is still catching up or an eviction passed it, so it
  has not seen everything the server evicted and `base_seq = 0` is its claim like any other (a
  new member gets the history and acks past the floor before its first post); then per object
  compare `base_seq`, the seq of the row the device edited from, with the stored row's seq (no
  row ⇒ take the holder's word, which the holder check just made good: any write it missed
  would still be a row here and conflict), on match write it at `seq = head + 1`, on mismatch
  report a conflict carrying the current row. There is no per-object version: a row's seq
  identifies its state. A delete is a push with `deleted = true`; the client puts the children
  it holds in the same batch, and the server does no cascade, since it neither sees parents nor
  holds children older than the window. A tombstone's deletion mark is its own seq, and a
  tombstone is a row like any other: here until every device has acked it, kept by a device
  until the minimum ack passes it. There is no pull-to-head check before a push and no rule
  about editing a tombstone: a stale base conflicts on the seq, and a deliberate un-delete is
  the client's call. Commit, then `GroupHeadAdvanced` and, per tombstone, `ObjectDeleted` so
  `storage` drops the object's files (invariant 7).
- **Pull and ack**: `PullChangesRequest` returns entries with `seq > cursor`, tombstones included,
  with the eviction floor and the minimum ack. The ack is the one number per device: a device
  holds the group from seq 1 through its ack, and is a holder once that reaches the floor. Acks
  are monotone, except 0, which is a device's first act in a group and its way to start over,
  and which gives it a row. A device's row pins the group's minimum ack, so nothing it has not
  acked can be evicted by ack while it catches up; a device that neither reaches the floor nor
  acks for 672 hours loses its row to the age rule. A holder whose cursor is below the floor
  (a restored backup, or evicted while away) throws the group away (files stay as a
  content-addressed cache, unpushed writes stay in the outbox) and takes the path of a new
  device: ack 0, pull the window, ask a holder for everything from seq 1 up to the floor, page
  and ack the restored rows as they land, ack the head, then replay the outbox against the
  fresh store: creates as they are, edits and deletes rebased onto the current row and dropped
  when the object is gone. Objects come whole; files come lazily. The server hands the minimum
  ack back so devices drop their tombstones. `GetHeadsRequest` returns `(group, head, floor,
  min ack)`.
- **History**: `RequestHistoryRequest` records a range, or one object, and wakes through
  `device` every holder (every device whose ack is at or above the floor), or answers
  `unfillable` at once when there is none; one of them claims the request (a conditional
  update; the first wins, a claim older than five minutes is taken over, a non-holder is
  refused) and pushes the rows it holds in the range back with `PushHistoryRequest`, oldest
  first, tombstones included, and the server re-inserts them **with their original seqs**,
  skipping any id that already has a row, without moving the head; the requester is woken after
  each call and pages them from its low ack, acking as it goes, while every other device's
  cursor is already past them. An answer marked complete is complete, since every holder holds
  the whole group. Restored rows go by the ack rule as the requester acks past them, or by age
  672 hours after their re-insert. No server-side retries: the requester asks again. This is how
  a new member, a new device or a device that started over gets everything from seq 1, and how
  an expired photo comes back (the holder re-uploads the file to its derived key, then reports
  the request complete).
- **Sweeps**: `evict_objects` (by ack: rows every device has acked, floor up to the minimum ack;
  by age: rows uploaded more than 672h ago, floor up to the highest seq taken, and the devices
  below the floor that have not acked for 672h lose their row), `expire_history_requests`
  (672h).
- **Sends**: `CheckMembershipRequest`, `GroupHeadAdvanced`, `ObjectDeleted`, `HistoryRequested`,
  `HistoryReady`.
- **API**: `SyncService`.

### `storage`

Presigns bucket access under the caller's group prefix and cleans up. No table of blobs: a file's
reference (slot, hash, size, content key) is inside the owning object's encrypted value, its key
is derived (`g/<group>/o/<object>/<slot>`), and the bucket's one lifecycle rule expires
everything 28 days after upload. Thumbnails are not files: a photo's thumbnail is a field
inside its encrypted value, so a holder of the object has the grid and the bucket holds only
full-size files and attachments. An upload is a presigned PUT, a download a presigned GET, an
expired file the bucket's 404.

- **Table**: `upload_usage` (group, hour, bytes): the quota is a cap on bytes uploaded per group
  per rolling 168 hours (20 GiB), which is what a cache needs bounded. Directive §2.
- **Config types**: `slot` (size cap per slot), `storage_quota`. Edited by a human.
- **External**: `delete_object`, `idempotent`, dispatched from handlers. Presigning and prefix
  listing are reads.
- **Handles**: `PresignUploadRequest` (member, key under the prefix, slot cap, quota, presigned
  PUT), `PresignDownloadRequest` (member, presigned GET), `ObjectDeleted` (from `sync`, per
  tombstone: delete the object's prefix), `PurgeGroupBlobsRequest` (delete the group's prefix in
  batches).
- **Sweep**: `expire_usage`, hourly.
- **Sends**: `CheckMembershipRequest`.
- **API**: `StorageService`.

## What the server computes

Five things, and nothing else: whether a user is a member of a group, and its role; the next
sequence number; whether an object's seq still matches the one a device edited from; whether a
device's ack has reached the eviction floor, and the lowest ack in the group; whether a key is
under the caller's group prefix and within its slot's cap and the group's upload quota. Every other byte is relayed, never read. Expiry is the
bucket's and the window's.

## Values and conflicts (client core)

Every `value` is a protobuf message of its type, encrypted, behind a plaintext header that names
the type, the schema version and the key (`06-object-types.md`). The server has
one rule, compare-and-set on the object's seq with the current row returned on a conflict, and
the client core has one way to handle it:

1. Keep, per locally edited object, the row it was edited from and the set of fields changed;
   for a repeated field (participants, poll options) the change is per element, added or
   removed by id.
2. On a conflict, decrypt the returned row, re-apply the changed fields and elements onto it,
   validate the result against the type's invariants (an event ending before it starts), and
   push again with the returned row's seq as `base_seq`. Where both sides changed the same field
   or element, the later push wins. A few immediate retries, then the object waits in the outbox
   for the next sync. An invalid result is shown to the user, not pushed.
3. A tombstone in the returned row ends the loop: the local edit is dropped and the client shows
   "deleted".
4. Deleting an object deletes its subtree: the client tombstones the children it holds in the
   same batch. A child whose parent the device holds as a tombstone is dead, whatever its own row
   says, and the first client to notice pushes its tombstone. This covers children the deleting
   device never held and a device that commented on a photo someone deleted while it was
   offline. The server sees no parents and does no cascade.

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

`schema_version` in the value's plaintext header says which schema the writer used. Add fields, never
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
  is Sign in with Basable (round 17), so pairing is the one QR the phone scans. Browser storage: object and blob ciphertext at rest in the Origin Private File System
  (files byte for byte as the bucket holds them, so the browser can re-upload them), decrypted into
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
  `sync`, `storage`; a separate `kratos` database.
- Kratos with the `oidc` method only, its one provider Basable, read from the `basable-login`
  Secret the platform writes into each environment (issuer, client id, client secret; never in
  the repository); `code` and `passkey` off, no courier. Users are Kratos identity ids; the
  identity's email trait is the verified email from Basable's id token, and the mapper refuses a
  sign-in without one. The web client runs the browser flow. A phone runs Kratos's native flow:
  it creates the login flow asking for a session token exchange code, opens Kratos's redirect in
  the system browser (ASWebAuthenticationSession, Custom Tabs), Basable signs the person in,
  Kratos returns to the app's URL with a code, and the app exchanges both codes at
  `/sessions/token-exchange` for the session token every API call carries.
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
| AddMemberRequest | Membership | api | group |
| RemoveMemberRequest | Membership | api | group |
| SetMemberRoleRequest | Membership | api | group |
| LeaveGroupRequest | Membership | api | group |
| DeleteGroupRequest | Group | api | group |
| DeleteAccountRequest | AccountDeletion | api | group |
| CheckMembershipRequest | MembershipCheck | device, sync, storage | group |
| MemberRemoved | (void) | group | device |
| PurgeGroupObjectsRequest | PurgeResult | group | sync |
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
| GroupHeadAdvanced | (void) | sync | device |
| HistoryRequested | (void) | sync | device |
| HistoryReady | (void) | sync | device |
| DeviceLeftGroup | (void) | device | sync |
| PushObjectsRequest | PushResult | api | sync |
| PullChangesRequest | ChangeSet | api | sync |
| AckSeqRequest | AckResult | api | sync |
| GetHeadsRequest | HeadList | api | sync |
| RequestHistoryRequest | HistoryRequest | api | sync |
| ListHistoryRequestsRequest | HistoryRequestList | api | sync |
| ClaimHistoryRequest | HistoryRequest | api | sync |
| PushHistoryRequest | PushHistoryResult | api | sync |
| ObjectDeleted | (void) | sync | storage |
| PresignUploadRequest | UploadGrant | api | storage |
| PresignDownloadRequest | DownloadGrant | api | storage |

Every cross-nanoservice message binds Directive §7. The only derived state that crosses is a
boolean, a role or an id (`MembershipCheck`, `DeviceLeftGroup`); no nanoservice re-derives another's rule.

## What is deliberately not on the server

Conflict rebase, cascading deletes, the note CRDT, validation of payloads, invariants inside a
type (one vote per poll), derived state (balances, tallies), schema conversion, thumbnails, resizing, map tiles,
geocoding. All of it is the client's, because all of it needs plaintext.
