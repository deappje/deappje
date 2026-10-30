# storage

Presigns uploads and downloads under the caller's group prefix, deletes an object's files on its tombstone and a group's on purge, bounds upload bytes per group.

Design: `docs/01-deappje/02-storage.md`. Code:
`nanoservices/storage/` (read its `AGENTS.md` first).

## Owns

- Table `upload_usage` (`group_id`, `hour`, `bytes`).
- Config type `slot`.
- Config type `storage_quota`.
- External call `delete_object` on object_storage: `idempotent`.
- Schedule `expire_usage` every 1h.

## Messages

- Handles: `PresignUploadRequest`, `PresignDownloadRequest`, `ObjectDeleted`, `PurgeGroupBlobsRequest`.
- Sends: `CheckMembershipRequest`.

## Directive sections that bind

§2 always; §6 for each adapter; §7 for each message sent; §10 for the docs.

## TODO, in order

- [x] `handle_presign_upload_request` in `src/handlers.rs`.
- [x] `handle_presign_download_request` in `src/handlers.rs`.
- [x] `handle_object_deleted` in `src/handlers.rs`.
- [x] `handle_purge_group_blobs_request` in `src/handlers.rs`.
- [x] Table `upload_usage`: the queries in `repository.rs`.
- [x] `send` for `delete_object` in `effects.rs`, the real client in `provider.rs`; the audit stays green.
- [x] `tick_expire_usage` in `worker.rs`.
- [x] `StorageService` field mapping in `api/src/services/storage.rs` and the messages in `proto/storage/v1/storage.proto`.
- [x] The message structs in `crates/messages/src/lib.rs`.
- [x] `flows.md` and this file kept true.

Landed 2026-10-30 (`nanoservices/storage`, the proto and the api mapping, the
messages, the config seed and its wiring in `app/src/main.rs`). Decisions
beyond the plan's text are recorded in `docs/01-deappje/02-storage.md`,
"Implementation record", and the flows in `nanoservices/storage/flows.md`.
