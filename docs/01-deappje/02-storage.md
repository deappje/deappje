# `storage`

Presigns bucket access and cleans up. No table of blobs: the reference to a file (slot, hash,
size, content key) lives inside the encrypted value of the object that owns it, the storage key
is derived from the object id, and the bucket's own lifecycle rule expires what is old. The one
table is a counter of upload bytes per group per hour, for the quota. Directive §2 for it, §6 for
the delete call, §7 for the messages.

## Keys

| key | content | lifecycle |
|---|---|---|
| `g/<group_id>/o/<object_id>/<slot>` | one encrypted file of an object; slots `full`, `file` | expires 28 days after upload |

The group prefix is the authorization boundary: a member may be given a presigned URL for any
key under its group's prefix and nothing else. The slot names the size cap. The bucket has one
lifecycle rule, expiry 28 days after upload, for everything in it; it is the platform's: the
environment's bucket carries it, and the app never deletes on a timer. If the platform cannot
carry a lifecycle rule, the fallback is an hourly sweep over a bucket listing, which this plan
does not build. Thumbnails are not in the bucket: they are a field inside the owning object's
value (`06-object-types.md`), so the bucket holds only full-size files and attachments, and
nothing in it needs to outlive the week.

An upload is a presigned PUT; a download is a presigned GET; a file that has expired is the 404
the bucket answers, and the client then asks `relay` for the object (`02-relay.md`), which
wakes every key holder, one of whom re-uploads the file to the same key. No confirm step and no
state on the server.

## Table `upload_usage`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key (with hour) |
| hour | timestamp | truncated to the hour |
| bytes | int64 | declared sizes of uploads presigned in that hour |

`PresignUploadRequest` sums the group's last 168 rows and refuses when the sum plus the new size
exceeds `storage_quota.max_bytes_per_168h` (20 GiB). Rows older than 168 hours are dropped by the
hourly sweep. This bounds what a group can put into the bucket per 168 hours, and with files
living 672 hours the bucket holds at most four times the cap per group, which is the cost
that matters for a cache.

## Config

`slot` (prefix `slot`): `max_size_bytes` per slot: `full` 25 MiB, `file` 25 MiB.
`storage_quota` (prefix `squo`): `max_bytes_per_168h`, 20 GiB. `04-config.md`.

## External calls

| System | Call | Strategy | Irreversible | Argument |
|---|---|---|---|---|
| object_storage | delete_object | idempotent | no (member devices have the bytes) | S3 `DeleteObject` on a missing key succeeds; a re-send converges. |

Presigned PUT/GET and prefix listing are reads. Presigning is virtual-hosted, then the origin is
replaced by `S3_PRIVATE_URL`. Deletes are dispatched `Unfenced` from the handlers through
`Call::dispatch` with `classify_transport`; a failed delete of an object's prefix is retried by
the next `ObjectDeleted` for the same object (the deleting client re-sends its tombstone batch
on the next sync until the push is accepted, and the group purge re-sends until the prefix is
empty). The simulator models a bucket with `put`, `get`, `list`, `delete` and ack loss on delete;
the `effecttest` audit covers it.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| PresignUploadRequest | UploadGrant | `{ group_id, key, size }`: member check (any role: a file is inert without an object, and a viewer may re-upload one it holds); key under the group prefix with a valid slot; slot cap; quota; record usage; presigned PUT, 15 minutes |
| PresignDownloadRequest | DownloadGrant | `{ group_id, key }`: member check; key under the group prefix; presigned GET, 5 minutes |
| ObjectDeleted | (void) | from `relay`, per tombstone: list `g/<group_id>/o/<object_id>/`, `delete_object` each; most objects have no prefix and the list is empty |
| PurgeGroupBlobsRequest | PurgeResult | from `group`'s purge pass: list `g/<group_id>/`, delete a batch, answer `remaining` = what the listing still shows. Idempotent. |

A re-upload goes to the same key, so every reference in every document stays valid and the
downloader's hash check still applies.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |

## Sweep

`expire_usage`, hourly: delete `upload_usage` rows older than 168 hours.

## API

`StorageService`: PresignUpload, PresignDownload.

## Implementation record

What the code settled beyond the text above (`nanoservices/storage`, 2026-10-30):

- **The config framework is wired by hand.** The scaffold renders the binders (`config.rs`) and
  the seed, but nothing loaded them: `app/src/main.rs` now registers `storage::config` on a
  `ConfigTypesBuilder`, applies `config/base` with `load_seed` at boot (`CONFIG_SEED_DIR`,
  `/config/base` in the image through a layer `config/BUILD.bazel` adds; `CONFIG_ENV`, `prod`
  by default) and hands `storage` a `basable_config::Repository` over the framework pool.
- **The seed.** `config/base/slot.json` (`full`, `file`, 25 MiB) and
  `storage_quota.dev.prod.json` (`default`, 20 GiB), beside `storage_quota.test.json`
  (`default`, 4 MiB) so the integration tests reach the quota in a few declared uploads. The
  scaffold's layout, not the `config/base/storage/` the text named.
- **Signature Version 4 by hand** over `ring` (already in the graph), no AWS SDK: signed
  path-style requests at `AWS_ENDPOINT_URL_S3` for `DeleteObject` and `ListObjectsV2` (the keys
  read out of the XML by hand; the application's keys carry no special characters), and
  virtual-hosted presigned URLs whose origin is replaced by `S3_PRIVATE_URL`, as the platform
  contract describes. A presigned PUT signs `content-length`, so the declared size is the size
  that lands. `AWS_ACCESS_KEY_ID` and friends come from the `object-storage` Secret through
  `envFrom` in `k8s/app/deployment.yaml`.
- **The quota is exact under concurrency**: the hour row's lock (`INSERT ... ON CONFLICT DO
  UPDATE`) serializes a group's presigns within the hour, and an upload over the cap is rolled
  back out of the counter. The cap is a closed bound: exactly at it is accepted.
- **Key shape is one rule** (`parse_key`): `g/<group>/o/<object>/<slot>`, the object id in
  canonical form, the slot lowercase letters, digits, `-` and `_`; nothing else reaches the
  bucket, so no segment is ever empty, `.` or `..` (the gateway normalises those and a signed
  path would no longer match).
- **Deletes**: a tombstone lists the object's prefix and deletes each key; a purge lists a
  thousand keys of the group, deletes each, and answers what a fresh listing still shows.

## Tests

- Unit: key parsing (prefix, slot; `..` and empty segments refused); quota at the boundary over
  a rolling 168 hours; slot caps.
- Integration (testkit + bucket simulator): presign, upload, download; a key outside the caller's
  group prefix is refused; `ObjectDeleted` deletes every slot of the object and nothing else; a
  failed delete is retried by the next `ObjectDeleted`; purge empties the group prefix across
  several passes.
- `effecttest`: `delete_object` with ack loss.
