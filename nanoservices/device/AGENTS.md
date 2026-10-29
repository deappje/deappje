# device

Owns devices, push tokens, wrapped group keys and key requests; sends silent pushes; tells relay when a device leaves a group.

Rendered by the scaffold from `docs/01-deappje/manifest.yaml`.
The Directive sections that bind this nanoservice: §2 (table ownership), §6 (external-effect admission — it owns adapters), §7
(cross-nanoservice rules for every message it sends), §10 (this file and
`flows.md` must stay true).

## What it owns

Plain tables (schema `nano_device`): `device`, `wrapped_key`, `key_request`.

| External call | System | Strategy | Irreversible |
|---|---|---|---|
| `send_apns` | apns | `idempotent` | no |
| `send_fcm` | fcm | `idempotent` | no |

## Files

| File | Responsibility |
|---|---|
| `src/lib.rs` | The `Device` struct, its constructor (pools, providers, calls), the schema marker, and its `basable_app::Component` impl: `loops()` lists every worker and ticker it runs (none for a plain executor). |
| `src/handlers.rs` | `impl DeviceHandler<R>`: one `handle_<snake_message>` per handled message. Intent only — `create` / `update_spec` / `mark_deleted` / `nudge`; never a status write. |
| `src/repository.rs` / `src/model.rs` | Typed sqlx queries over this nanoservice's pool; the row structs. |
| `src/effects.rs` | One `Call` per external call with its strategy literal; `send` is the only dispatch path. |
| `src/provider.rs` / `src/simulator.rs` | The provider trait, the HTTP client (a parsed 4xx is `SendError::Refused`), the noop; the in-memory simulator the audit and the tests drive. |
| `tests/effects_audit.rs` | One `audit!` per adapter — the audit index. Green on day one; the first thing a real provider must keep green. |
| `tests/integration.rs` | Integration tests on the testkit (a database per test). |
| `flows.md` | State machines, sequences, edge cases. |

## Messages

Handles: `RegisterDeviceRequest` → `Device`, `SetPushTokenRequest` → `Device`, `ListDevicesRequest` → `DeviceList`, `RequestGroupKeysRequest` → `KeyRequest`, `ListKeyRequestsRequest` → `KeyRequestList`, `FulfilKeyRequestRequest` → `KeyRequest`, `PublishWrappedKeysRequest` → `WrappedKeyList`, `GetWrappedKeysRequest` → `WrappedKeyList`, `RemoveDeviceRequest` → `Device`, `GroupHeadAdvanced`, `MemberRemoved`, `HistoryRequested`, `HistoryReady`, `PurgeGroupKeysRequest` → `PurgeResult`, `PurgeUserDevicesRequest` → `PurgeResult`.
Sends: `CheckMembershipRequest`, `DeviceLeftGroup`.

API: `DeviceService` in `api/src/services/device.rs` (RegisterDevice, SetPushToken, ListDevices, RequestGroupKeys, ListKeyRequests, FulfilKeyRequest, PublishWrappedKeys, GetWrappedKeys, RemoveDevice).
