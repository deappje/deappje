# `storage`

Presigns bucket access and cleans up. No table of blobs: the reference to a blob (slot, hash,
size, content key) lives inside the encrypted value of the object that owns it, the storage key
is derived from the object id, and the bucket's own lifecycle rules expire what is old. The one
table is a counter of upload bytes per group per hour, for the quota. Directive §2 for it, §6 for
the delete call, §7 for the messages.

## Keys

| key | content | lifecycle |
|---|---|---|
| `g/<group_id>/o/<object_id>/<slot>` | one encrypted file of an object; slots `full`, `thumb`, `file` | `full` and `file` expire 2 days after upload; `thumb` never |
| `g/<group_id>/bundle/<request_id>` | a history bundle answering a `ledger` request | expires 2 days after upload |

The group prefix is the authorization boundary: a member may be given a presigned URL for any
key under its group's prefix and nothing else. The slot names the size cap and the retention.
Lifecycle rules per prefix are the platform's: the environment's bucket carries them, and the
app never deletes on a timer. If the platform cannot carry lifecycle rules, the fallback is an
hourly sweep over a bucket listing, which this plan does not build.

## Table `upload_usage`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key (with hour) |
| hour | timestamp | truncated to the hour |
| bytes | int64 | declared sizes of uploads presigned in that hour |

`PresignUploadRequest` sums the group's last 48 rows and refuses when the sum plus the new size
exceeds `storage_quota.max_bytes_per_48h` (20 GiB). Rows older than 48 hours are dropped by the
hourly sweep. This bounds what a group can put into the bucket per two days, which is the cost
that matters for a cache.

## Config

`slot` (prefix `slot`): `max_size_bytes` per slot: `full` 25 MiB, `thumb` 200 KiB, `file` 25
MiB, `bundle` 256 MiB. `storage_quota` (prefix `squo`): `max_bytes_per_48h`, 20 GiB.
`04-config.md`.

## External calls

| System | Call | Strategy | Irreversible | Argument |
|---|---|---|---|---|
| object_storage | delete_object | idempotent | no (holders have the bytes) | S3 `DeleteObject` on a missing key succeeds; a re-send converges. |

Presigned PUT/GET, `HEAD` and prefix listing are reads. Presigning is virtual-hosted, then the
origin is replaced by `S3_PRIVATE_URL`. Deletes are dispatched `Unfenced` from the handlers
through `Call::dispatch` with `classify_transport`; a failed delete of an object's prefix is
retried by the next `ObjectDeleted` for the same object (the deleting client re-sends its
tombstone batch on the next sync until the push is accepted, and the group purge re-sends until
the prefix is empty). The simulator models a bucket with `put`, `head`, `list`, `delete` and ack
loss on delete; the `effecttest` audit covers it.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| PresignUploadRequest | UploadGrant | `{ group_id, key, size, request_id? }`: member check, and `admin` or `member` unless the key is a bundle (a viewer is a holder and may serve history); key under the group prefix with a valid slot; slot cap; quota; record usage; presigned PUT, 15 minutes |
| ConfirmUploadRequest | (void) | `{ group_id, key, request_id? }`: `HEAD` the key; present ⇒ if `request_id` is set send `HistoryFulfilled { request_id }` to `ledger`. Nothing is recorded otherwise. |
| PresignDownloadRequest | DownloadGrant | `{ group_id, key }`: member check; key under the group prefix; `HEAD` ⇒ presigned GET, 5 minutes, or `gone` so the client requests it from a member device |
| ObjectDeleted | (void) | from `ledger`, per tombstone: list `g/<group_id>/o/<object_id>/`, `delete_object` each; most objects have no prefix and the list is empty |
| PurgeGroupBlobsRequest | PurgeResult | from `group`'s purge pass: list `g/<group_id>/`, delete a batch, answer `remaining` = what the listing still shows. Idempotent. |

A re-upload (a holder answering a request for one object) goes to the same key, so every
reference in every document stays valid and the downloader's hash check still applies.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |
| HistoryFulfilled | (void) | ledger |

## Sweep

`expire_usage`, hourly: delete `upload_usage` rows older than 48 hours.

## API

`StorageService`: PresignUpload, ConfirmUpload, PresignDownload.

## Tests

- Unit: key parsing (prefix, slot, bundle; `..` and empty segments refused); quota at the
  boundary over a rolling 48 hours; slot caps.
- Integration (testkit + bucket simulator): presign, upload, confirm, download; a confirm with a
  `request_id` sends `HistoryFulfilled` once; `ObjectDeleted` deletes every slot of the object
  and nothing else; a failed delete is retried by the next `ObjectDeleted`; purge empties the
  group prefix across several passes; a viewer is refused a `full` upload and granted a bundle.
- `effecttest`: `delete_object` with ack loss.
