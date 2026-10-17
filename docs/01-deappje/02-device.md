# `device`

Owns devices, their public keys and push tokens, the wrapped group keys, and key requests. Plain
executor. The server stores key material it cannot open: a wrapped key is ciphertext produced on
one member's device for another device's public key. Directive §2 per table, §6 for the two push
calls, §7 per message.

## Tables

### `device`

| column | type | notes |
|---|---|---|
| id | uuid | primary key, client-minted on first registration |
| user_id | uuid | Kratos identity id |
| public_key | bytes | X25519, for wrapping |
| platform | string | `ios`, `android`, `web` |
| push_provider | string, nullable | `apns`, `fcm` |
| push_token | string, nullable | |
| created_at | timestamp | |
| last_seen_at | timestamp | |

Index `(user_id)`. A device is registered by the logged-in user and only they can change it.

### `wrapped_key`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| key_id | int32 | the group's key generation; objects name it in plaintext metadata |
| device_id | uuid | the device this copy is wrapped for |
| wrapped | bytes | ciphertext |
| wrapped_by_device | uuid | provenance |
| created_at | timestamp | |

Unique `(group_id, key_id, device_id)`. Index `(device_id)`.

Invariants: a row is accepted only if the target device's user is a member of `group_id` and the
wrapping device's user is a member (`CheckMembershipRequest`, twice). On `MemberRemoved`, every
row for that user's devices in that group is deleted. The server never validates the ciphertext;
a bad wrap is detected on the receiving device and re-requested.

### `key_request`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| device_id | uuid | the device asking |
| status | string | `pending`, `fulfilled`, `cancelled` |
| created_at | timestamp | |
| fulfilled_at | timestamp, nullable | |

Unique `(group_id, device_id)` where pending. A request is created by a device whose user is a
member but which holds no key; every device that holds a key for the group is pushed. Fulfilment
is a human act: the holder's client shows the requesting user, platform and the fingerprint of
the requesting device's public key (the requester reads it off their own screen), and a tap
wraps the keys and calls `FulfilKeyRequestRequest`. The normal path is a scan: the requesting
device shows `{ device_id, fingerprint }` as a QR, the holder scans it and approves the pending
request whose public key hashes to that fingerprint. There is no silent path, not even for the
same user's devices: a server could insert a device row under the victim's own user id and the
victim's phone would wrap for it. The server cannot tell a scan from automation; what it enforces
is the trust rule below, and what the clients enforce is that nothing is wrapped for a device
whose fingerprint a person did not scan or compare.

The browser is one of these devices: it pairs by showing the QR after a passkey login (WebAuthn
cross-device authentication is already a QR the phone scans, so login and pairing are two scans
in a row, or one when the client chains them). Without "remember this browser" its device key and
local store live in memory for the session and the device row is deleted on logout.

## Trust rule

The server never seeds trust. A device holds a group's keys only through fulfilment by a device
that already holds them; a rotation wraps the new key only for devices that hold the previous
one (`ListDevicesRequest` for a group lists key holders, nothing else). A device row the server
inserted on its own therefore never receives a key: it appears in no list a wrapping client
reads, and no person approves it. The first device in a group is the creator's, which mints key
1 and wraps it for itself.

## Key exchange protocol (client core; the server relays bytes)

| Key | Where | Algorithm |
|---|---|---|
| device key | one X25519 keypair per device, generated on the device, private half in the Secure Enclave-backed keychain / Android Keystore / a non-extractable WebCrypto key; the public half is `device.public_key` | X25519 |
| group key | 32 random bytes per `(group_id, key_id)`, minted on a member's device | XChaCha20-Poly1305 for object values, random 24-byte nonce, associated data = the plaintext metadata (`group_id`, `id`, `type`, `schema_version`, `key_id`) so the server cannot re-label a ciphertext |
| blob key | 32 random bytes per blob, stored inside the referencing object's document | XChaCha20-Poly1305 over the blob; unchanged by rotation, so a holder re-uploads the same bytes |

A wrap (`wrapped_key.wrapped`) is HPKE (RFC 9180) in **Auth mode**: DHKEM(X25519, HKDF-SHA256),
HKDF-SHA256, ChaCha20-Poly1305, sender = the wrapping device's static key, recipient = the target
device's public key, `info` = `group_id ‖ key_id ‖ device_id ‖ wrapped_by_device`. Auth mode
makes the recipient verify which device produced the wrap, matching `wrapped_by_device`. A new
holder receives one wrap per `key_id`, so a new member gets the full history.

Trust in the client core: the fingerprint shown at approval is SHA-256 of the device public key
as a short code; a device **pins** the public key of every device it has wrapped for or received
from, and a different key under a known device id is refused and surfaced (a changed safety
number), which is what stops a server from swapping a holder's public key before a rotation.
Kratos sessions authorise API calls and carry no key material. Crates: `hpke`,
`chacha20poly1305`, both wasm-capable.

## Key rotation

On `MemberRemoved`, after deleting the removed user's wrapped keys, `device` pushes a
`rotate` wake-up to every remaining key-holder device. One of them mints key `n+1`, wraps it for
every device listed by `ListDevicesRequest` (holders of key `n`), and calls `PublishWrappedKeysRequest` with the whole
batch. Two devices racing produce two batches with the same `key_id`; the unique index makes the
second insert a no-op per row, and since both batches wrap *different* keys under the same id,
the handler refuses a batch whose `key_id` already has rows from a different `wrapped_by_device`.
The loser re-reads and adopts. Old objects stay under old key ids; the keyring on every device
holds all of them.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| RegisterDeviceRequest | Device | upsert for the caller |
| SetPushTokenRequest | Device | |
| ListDevicesRequest | DeviceList | the devices holding a key for a group, with public keys; caller must be a holder |
| RequestGroupKeysRequest | KeyRequest | caller's user must be a member; insert pending; push every holder device |
| ListKeyRequestsRequest | KeyRequestList | pending requests in groups the caller holds a key for, with the requesting device's platform and public key |
| FulfilKeyRequestRequest | KeyRequest | caller must hold a key for the group; insert the wrapped keys for the requesting device, mark fulfilled, push it |
| PublishWrappedKeysRequest | WrappedKeyList | batch insert, rotation rule above |
| GetWrappedKeysRequest | WrappedKeyList | every wrapped key for the calling device, optionally one group |
| GroupHeadAdvanced | (void) | silent push to every member device except the writer's |
| MemberRemoved | (void) | delete wrapped keys; push `rotate` to remaining devices |
| BlobRequested | (void) | push `blob` wake-up to every device in the message's `holders` list (the uploader and every device that downloaded it) |
| CheckHolderCoverageRequest | HolderCoverage | from `storage`'s pass: `complete` = every device holding a key for `group_id` is in `device_ids`. A boolean derived from own tables, nothing more crosses. |
| PurgeGroupKeysRequest | PurgeResult | from `group`'s purge pass: delete every `wrapped_key` and `key_request` row of the group; answer `remaining`. Idempotent. |
| PurgeUserDevicesRequest | PurgeResult | from `group`'s account purge: delete every `device`, `wrapped_key` and `key_request` row of the user's devices; answer `remaining`. Idempotent. |

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |

Push fan-out needs the member list; `device` asks `group` (`ListMembersRequest` is an api
message, so `device` uses `CheckMembershipRequest` per candidate device it already knows from
`wrapped_key` rows for the group: a device that holds a key is a device to wake). This avoids a
second membership read path and keeps `group`'s table private.

## External calls

| System | Call | Strategy | Irreversible | Argument |
|---|---|---|---|---|
| apns | send_push | idempotent | no | The payload is a content-less wake-up naming a group or a reason; a duplicate costs one extra wake. Nothing a user sees. |
| fcm | send_push | idempotent | no | Same. |

Dispatched `Unfenced` from the request-path handlers (there is no claim), through `Call::dispatch`,
with `classify_transport`. Delivery failures are logged and dropped; the client polls heads on
foreground anyway. Both adapters get an `effecttest` audit against a simulator that records
sends.

Token invalidation (APNs 410, FCM `UNREGISTERED`) clears `push_token` through the repository.

## API

`DeviceService`: RegisterDevice, SetPushToken, ListDevices, RequestGroupKeys, ListKeyRequests,
FulfilKeyRequest, PublishWrappedKeys, GetWrappedKeys.

## Tests

- Unit: rotation batch rule (same key id, different wrapper ⇒ refused); wrapped key for a
  non-member device refused; `FulfilKeyRequest` from a non-holder refused; `ListDevices` omits a
  registered device with no key.
- Integration: `MemberRemoved` deletes exactly that user's rows and pushes every other device
  once; two racing `PublishWrappedKeysRequest`s for the same key id end with one wrapper.
- `effecttest`: both push adapters, including ack loss.
