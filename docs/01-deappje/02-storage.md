# `storage`

Owns blobs: a row per encrypted file, who holds a copy, and the file's 48 hours in the private
bucket. Plain executor with one hourly sweep. Nothing here has a state machine: a blob is in the
bucket until its expiry or until everyone has it, and after that it comes from a holder device.
Directive §2 for the tables, §6 for the delete call, §7 for the messages.

## Tables

### `blob`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| kind | string | a `blob_kind` config name: `full`, `thumbnail`, `attachment`, `bundle` |
| storage_key | string | `g/<group>/b/<blob>`; no empty, `.` or `..` segments |
| sha256 | bytes | of the ciphertext, declared by the uploader; verified by the downloader |
| declared_size | int64 | checked against the kind's cap and the group quota |
| uploader_device | uuid | the holder of last resort |
| request_id | uuid, nullable | for kind `bundle`: the `ledger` history request it answers |
| in_bucket | bool | true between a confirmed upload and the bucket delete |
| expires_at | timestamp, nullable | set on confirm: `now + retention`; passed ⇒ the sweep deletes |
| created_at | timestamp | |

Indexes: `(group_id)`; `(expires_at)` where `in_bucket`.

### `blob_holder`

| column | type |
|---|---|
| blob_id | uuid |
| device_id | uuid |
| downloaded_at | timestamp |

Unique `(blob_id, device_id)`, index `(device_id)`. A device that was granted a download is a
holder until it says otherwise (`DropBlobHoldRequest`: evicted under storage pressure, or woken
for bytes it no longer has). The uploader is always a holder. If no holder is left, the blob is
gone: the thumbnail, if any, stays and the client shows the full size as unavailable.

## Config

`blob_kind` (prefix `bkind`): `retention_days`, `max_size_bytes`. Seeds: `full` (2, 25 MiB),
`thumbnail` (3650, 200 KiB), `attachment` (2, 25 MiB), `bundle` (2, 256 MiB). Thumbnails are the
one long-lived kind, so an album grid works on a new device without waking anyone; set it to 2 to
make the bucket a pure cache. `storage_quota` (prefix `squo`): `max_group_bytes`, 20 GiB.
`04-config.md`.

## External calls

| System | Call | Strategy | Irreversible | Argument |
|---|---|---|---|---|
| object_storage | delete_object | idempotent | no (holders have the bytes) | S3 `DeleteObject` on a missing key succeeds; a re-send converges. |

Presigned PUT/GET and `HEAD` are reads. Presigning is virtual-hosted, then the origin is replaced
by `S3_PRIVATE_URL`. Dispatched `Unfenced` from handlers and the sweep through `Call::dispatch`
with `classify_transport`; a failed delete leaves `in_bucket = true` with a passed `expires_at`,
so the next sweep retries. The simulator models a bucket with `put`, `head`, `delete` and ack
loss on delete; the `effecttest` audit covers it.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| CreateUploadRequest | UploadGrant | member check; kind lookup; size cap; quota (`SUM(declared_size)` of the group's rows with `in_bucket`, derived at request time); insert row with `in_bucket = false`; presigned PUT, 15 minutes |
| ConfirmUploadRequest | Blob | `HEAD` the key; size must equal `declared_size`; set `in_bucket = true`, `expires_at = now + retention`, uploader as holder; if `request_id` is set send `BundleReady` to `ledger` |
| ReuploadBlobRequest | UploadGrant | member check; a holder re-uploads to the same key; presigned PUT; then `ConfirmUpload` as above |
| GetDownloadRequest | DownloadGrant | member check; if `in_bucket` and `expires_at > now`: presigned GET (5 minutes), upsert holder, then `CheckHolderCoverageRequest { group_id, holders ∪ uploader }` → `complete` ⇒ `delete_object`, `in_bucket = false` (everyone has it; the bucket copy is pure cost); else answer `gone` so the client requests it |
| RequestBlobsRequest | (void) | member check; send `BlobRequested { group_id, blob_ids, holders }` to `device` with every holder of every listed blob; the client batches (an album scroll asks for its missing thumbnails in one call) |
| DropBlobHoldRequest | (void) | delete the caller's holder rows for the listed blobs |
| CheckBlobRequest | Blob | from `ledger`: `{ group_id, present }` where present = a row exists (the bucket copy may already be gone; the object may still reference it) |
| ObjectDeleted | (void) | from `ledger`: `delete_object`, delete the row and its holders; on a failed delete set `expires_at = now` and leave `in_bucket` for the sweep |
| PurgeGroupBlobsRequest | PurgeResult | from `group`'s purge pass: for a batch of the group's rows `delete_object` where `in_bucket`, delete rows and holders; answer `remaining`. Idempotent. |

A re-uploaded blob keeps its id, key and sha256, so every reference in every document stays
valid and the downloader's hash check still applies.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |
| CheckHolderCoverageRequest | HolderCoverage | device |
| BlobRequested | (void) | device |
| BundleReady | (void) | ledger |

## Sweep

`expire_blobs`, hourly: rows with `in_bucket` and `expires_at < now` ⇒ `delete_object`, then
`in_bucket = false`. The row and its holders stay: they are the registry a re-request needs.
Rows of kind `bundle` are deleted outright once expired (nobody re-requests a bundle; the
requester asks `ledger` for a new one).

## API

`StorageService`: CreateUpload, ConfirmUpload, ReuploadBlob, GetDownload, RequestBlobs,
DropBlobHold.

## Tests

- Unit: quota at the boundary; size cap; expiry arithmetic; coverage ⇒ delete.
- Integration (testkit + bucket simulator): create → confirm → download → expiry sweep deletes
  → request wakes holders → re-upload → download again; once the last key-holder device
  downloads, the bucket copy is deleted at once; a dropped hold leaves the next `BlobRequested`;
  `ObjectDeleted` with a failing delete is retried by the sweep; a `bundle` confirm sends
  `BundleReady` once.
- `effecttest`: `delete_object` with ack loss.
