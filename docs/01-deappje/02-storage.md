# `storage`

Owns blobs: the encrypted files in the environment's private bucket and their lifecycle. This is
the one nanoservice with a processing-object type, because a blob is driven against a third
party over time: wait for the upload to land, expire it, delete it from the bucket, wait for a
re-upload. Directive §3, §5, §6 bind the type; §2 the side table; §6 the delete call; §7 the
messages.

## Processing object `blob` (prefix `blb`)

### Spec (desired state)

| field | type | immutable | notes |
|---|---|---|---|
| group_id | uuid | yes | |
| object_id | uuid, nullable | no | the ledger object that references it; set on first push |
| kind | string | yes | a `blob_kind` config name: `full`, `thumbnail`, `attachment` |
| storage_key | string | yes | `g/<group>/b/<blob>` — no empty, `.` or `..` segments |
| sha256 | bytes | yes | of the ciphertext, declared at create time |
| declared_size | int64 | yes | checked against the kind's cap at create |
| uploader_device | uuid | yes | the holder of last resort |
| retention_days | int32 | no | copied from the kind at create; a re-request may extend |

### Status (observed state)

| field | type |
|---|---|
| phase | string: `pending_upload`, `available`, `expired`, `requested` |
| size_bytes | int64, nullable |
| uploaded_at | timestamp, nullable |
| expires_at | timestamp, nullable |
| requested_at | timestamp, nullable |
| last_error | string, nullable |

### Side table `blob_access`

| column | type |
|---|---|
| blob_id | uuid, primary key |
| last_access_at | timestamp |

Written by the `GetDownloadRequest` handler (unclaimed input: invariant 3 forbids writing status
from the request path), followed by `nudge`. The pass reads it and extends `expires_at`.

### Reconcile pass (level-triggered, one `Outcome` per pass)

1. `meta.deleting()` ⇒ `HEAD` the key. Present ⇒ `delete_object`, then `Retry` (confirm on the
   next pass, never settle on an accepted delete: invariant 6). Absent ⇒ `Delete`.
2. `pending_upload` ⇒ `HEAD` the key. Present with matching size and (via the stored ETag or a
   ranged read of the first bytes when the provider exposes no hash) plausible ⇒ phase
   `available`, `uploaded_at = now`, `expires_at = now + retention_days`,
   `Converged.after(expires_at - now)`. Absent ⇒ `Converged.after(5m)`; the confirm handler nudges
   so the poll is only a fallback. Pending longer than 24h ⇒ `Blocked` (abandoned upload; a new
   `CreateUploadRequest` makes a new object).
3. `available` ⇒ if `blob_access.last_access_at + retention_days > expires_at`, extend
   `expires_at` and `Converged.after(...)`. If `now >= expires_at` ⇒ `delete_object`, phase
   `expired`, `Settled`. Otherwise `Converged.after(expires_at - now)`.
4. `expired` ⇒ `Settled` (parked until a `RequestBlobRequest` nudges).
5. `requested` ⇒ `HEAD` the key. Present ⇒ as in 2, back to `available`. Absent ⇒ `Settled`
   (parked until `ConfirmUploadRequest` nudges).

Integrity: the client declares `sha256` of the ciphertext at create; the receiving client
verifies it after download and before decrypting. The server only compares size. A mismatch on a
re-upload is detected by the downloader, who calls `RequestBlobRequest` again; the uploader
device is at fault and the audit log names it.

### Worker policy

`resync` 6h (every blob is re-examined at least that often, cheap `HEAD`s), `backoff` 30s
exponential to 1h, `max_attempts` 20 before `Blocked`, `attempt_timeout` 60s. Identical on every
replica (invariant 4).

### Teardown

`ObjectDeleted` ⇒ `mark_deleted`. The pass confirms absence in the bucket before `Delete`; a
blob whose delete keeps failing stays loud by age, never parks.

## Config type `blob_kind` (prefix `bkind`)

| field | type |
|---|---|
| retention_days | int32 |
| max_size_bytes | int64 |

Seeds under `config/base/`: `full` (30, 26214400), `thumbnail` (3650, 204800), `attachment`
(365, 26214400). Read by `CreateUploadRequest`; edited by a human only.

## External calls

| System | Call | Strategy | Irreversible | Argument |
|---|---|---|---|---|
| object_storage | delete_object | idempotent | no (the ciphertext is recoverable from a holder device, and the pass re-drives) | S3 `DeleteObject` on a missing key succeeds; a re-send converges. |

Presigned PUT/GET generation and `HEAD` are reads, not adapters. Presigning is virtual-hosted,
then the origin is replaced by `S3_PRIVATE_URL` (the platform's rule). The simulator models a
bucket with `put`, `head`, `delete` and ack loss on delete; the `effecttest` audit covers it.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| CreateUploadRequest | UploadGrant | member check; kind lookup; size cap; `TypedStore::create` in `pending_upload`; presigned PUT, 15 minutes |
| ConfirmUploadRequest | Blob | `nudge`; returns current phase |
| GetDownloadRequest | DownloadGrant | member check; if `available`: presigned GET (5 minutes), write `blob_access`, `nudge`; else the phase so the client can request |
| RequestBlobRequest | Blob | member check; write `requested_at` into `blob_access`, `nudge`; the pass moves `expired` → `requested` and sends `BlobRequested` |
| CheckBlobRequest | Blob | from `ledger`: group and phase |
| ObjectDeleted | (void) | `mark_deleted` for `blob_id` |

Phase is status and status is written only under claim, so neither the download nor the
re-request handler touches it: both write the `blob_access` side table and nudge, and the next
pass observes the marker.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |
| BlobRequested | (void) | device |

`BlobRequested { group_id, blob_id, uploader_device }` is sent from the reconcile pass between
fences, after the status write that records `requested`.

## API

`StorageService`: CreateUpload, ConfirmUpload, GetDownload, RequestBlob.

## Tests

- Unit: pass outcomes per phase × bucket state (table-driven); retention extension math; size cap.
- Integration (testkit + bucket simulator): create → confirm → available → expiry deletes →
  request → re-upload → available; `ObjectDeleted` with a present key deletes and confirms on
  the next pass; two replicas never both delete.
- `effecttest`: `delete_object` with ack loss.
