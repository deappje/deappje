# sync

The relay — ciphertext per group until every holder has acked it, 672 hours at most, tombstones included; the counter; per-device acked cursor; history requests.

Rendered by the scaffold from `docs/01-deappje/manifest.yaml`.
The Directive sections that bind this nanoservice: §2 (table ownership), §7
(cross-nanoservice rules for every message it sends), §10 (this file and
`flows.md` must stay true).

## What it owns

Plain tables (schema `nano_sync`): `object`, `group_head`, `group_device`, `history_request`.

## Files

| File | Responsibility |
|---|---|
| `src/lib.rs` | The `Sync` struct, its constructor (pools, providers, calls), the schema marker, and its `basable_app::Component` impl: `loops()` lists every worker and ticker it runs (none for a plain executor). |
| `src/handlers.rs` | `impl SyncHandler<R>`: one `handle_<snake_message>` per handled message. Intent only — `create` / `update_spec` / `mark_deleted` / `nudge`; never a status write. |
| `src/repository.rs` / `src/model.rs` | Typed sqlx queries over this nanoservice's pool; the row structs. |
| `src/worker.rs` | The ticker workers (`evict_objects` every 1h, `expire_history_requests` every 1h), listed in this nanoservice's `loops()` (`src/lib.rs`), started by the app and joined on shutdown. |
| `tests/integration.rs` | Integration tests on the testkit (a database per test). |
| `flows.md` | State machines, sequences, edge cases. |

## Messages

Handles: `PushObjectsRequest` → `PushResult`, `PullChangesRequest` → `ChangeSet`, `AckSeqRequest` → `AckResult`, `GetHeadsRequest` → `HeadList`, `RequestHistoryRequest` → `HistoryRequest`, `ListHistoryRequestsRequest` → `HistoryRequestList`, `ClaimHistoryRequest` → `HistoryRequest`, `PushHistoryRequest` → `PushHistoryResult`, `DeviceLeftGroup`, `PurgeGroupObjectsRequest` → `PurgeResult`.
Sends: `CheckMembershipRequest`, `GroupHeadAdvanced`, `ObjectDeleted`, `HistoryRequested`, `HistoryReady`.

API: `SyncService` in `api/src/services/sync.rs` (PushObjects, PullChanges, AckSeq, GetHeads, RequestHistory, ListHistoryRequests, ClaimHistory, PushHistory).
