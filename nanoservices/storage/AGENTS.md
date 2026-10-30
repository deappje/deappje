# storage

Presigns uploads and downloads under the caller's group prefix, deletes an object's files on its tombstone and a group's on purge, bounds upload bytes per group.

Rendered by the scaffold from `docs/01-deappje/manifest.yaml`.
The Directive sections that bind this nanoservice: §2 (table ownership), §6 (external-effect admission — it owns adapters), §7
(cross-nanoservice rules for every message it sends), §10 (this file and
`flows.md` must stay true).

## What it owns

Plain tables (schema `nano_storage`): `upload_usage`.

Config catalogs: `slot` (`config/base/slot.json`), `storage_quota` (`config/base/storage_quota.json`).

| External call | System | Strategy | Irreversible |
|---|---|---|---|
| `delete_object` | object_storage | `idempotent` | no |

The bucket is the environment's private one, from the `object-storage`
Secret the platform writes (`envFrom` in `k8s/app/deployment.yaml`:
`AWS_ENDPOINT_URL_S3`, `AWS_REGION`, `S3_BUCKET`, `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, `S3_PRIVATE_URL`). Presigning and the prefix
listing are reads beside the adapter (`Calls::presign`, `Calls::list_keys`).

## Files

| File | Responsibility |
|---|---|
| `src/lib.rs` | The `Storage` struct, its constructor (pools, providers, calls), the schema marker, and its `basable_app::Component` impl: `loops()` lists every worker and ticker it runs (none for a plain executor). |
| `src/handlers.rs` | `impl StorageHandler<R>`: one `handle_<snake_message>` per handled message. The rules are pure functions with unit tests: `parse_key` (the one key shape), `hour_of`, `within_quota`; the constants `UPLOAD_GRANT`, `DOWNLOAD_GRANT`, `USAGE_WINDOW`, `LIST_BATCH`, `CONFIG_NAMESPACE`, `QUOTA_NAME`. Config is read per request through `basable_config::Repository`. |
| `src/repository.rs` / `src/model.rs` | Typed sqlx queries over this nanoservice's pool; the row structs. |
| `src/effects.rs` | One `Call` per external call with its strategy literal; `send` is the only dispatch path. |
| `src/provider.rs` / `src/simulator.rs` | The provider trait (`delete_object`, the `list_keys` read, the `presign` computation); `S3Client`: Signature Version 4 by hand over `ring`, path-style signed requests at `AWS_ENDPOINT_URL_S3`, virtual-hosted presigned URLs moved onto `S3_PRIVATE_URL`; `Unconfigured` when the Secret is absent; the in-memory bucket (`put`, `get`, `keys`, list, delete, faults). |
| `src/config.rs` | The binders of `slot` and `storage_quota` over `basable_config`; `register` is what `app/src/main.rs` calls before the seed loads. |
| `src/worker.rs` | `expire_usage`, hourly. |
| `tests/effects_audit.rs` | One `audit!` per adapter — the audit index. Green on day one; the first thing a real provider must keep green. |
| `src/worker.rs` | The ticker workers (`expire_usage` every 1h), listed in this nanoservice's `loops()` (`src/lib.rs`), started by the app and joined on shutdown. |
| `src/config.rs` | The config type declarations and their readers. |
| `tests/integration.rs` | Integration tests on the testkit (a database per test). |
| `flows.md` | State machines, sequences, edge cases. |

## Messages

Handles: `PresignUploadRequest` → `UploadGrant`, `PresignDownloadRequest` → `DownloadGrant`, `ObjectDeleted`, `PurgeGroupBlobsRequest` → `PurgeResult`.
Sends: `CheckMembershipRequest`.

API: `StorageService` in `api/src/services/storage.rs` (PresignUpload, PresignDownload).
