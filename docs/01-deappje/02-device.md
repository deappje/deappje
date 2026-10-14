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
member but which holds no key; every other member device is pushed; the first device that
publishes wrapped keys for it marks it fulfilled.

## Key rotation

On `MemberRemoved`, after deleting the removed user's wrapped keys, `device` pushes a
`rotate` wake-up to every remaining member device. One of them mints key `n+1`, wraps it for
every device listed by `ListDevicesRequest`, and calls `PublishWrappedKeysRequest` with the whole
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
| ListDevicesRequest | DeviceList | all devices of all members of a group, with public keys; caller must be a member |
| RequestGroupKeysRequest | KeyRequest | insert pending; push other member devices |
| ListKeyRequestsRequest | KeyRequestList | pending requests in groups the caller is in |
| FulfilKeyRequestRequest | KeyRequest | insert the wrapped keys for the requesting device, mark fulfilled, push it |
| PublishWrappedKeysRequest | WrappedKeyList | batch insert, rotation rule above |
| GetWrappedKeysRequest | WrappedKeyList | every wrapped key for the calling device, optionally one group |
| GroupHeadAdvanced | (void) | silent push to every member device except the writer's |
| MemberRemoved | (void) | delete wrapped keys; push `rotate` to remaining devices |
| BlobRequested | (void) | push `blob` wake-up to the uploader device (and, later, other known holders) |

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
  non-member device refused.
- Integration: `MemberRemoved` deletes exactly that user's rows and pushes every other device
  once; two racing `PublishWrappedKeysRequest`s for the same key id end with one wrapper.
- `effecttest`: both push adapters, including ack loss.
