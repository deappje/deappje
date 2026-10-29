# device

Owns devices, push tokens, wrapped group keys and key requests; sends silent pushes; tells sync when a device leaves a group.

Design: `docs/01-deappje/02-device.md`. Code:
`nanoservices/device/` (read its `AGENTS.md` first).

## Owns

- Table `device` (`id`, `user_id`, `public_key`, `platform`, `push_provider`, `push_token`, `created_at`, `last_seen_at`).
- Table `wrapped_key` (`id`, `group_id`, `key_id`, `device_id`, `wrapped`, `wrapped_by_device`, `created_at`).
- Table `key_request` (`id`, `group_id`, `device_id`, `invite_id`, `invite_tag`, `created_at`).
- External call `send_apns` on apns: `idempotent`.
- External call `send_fcm` on fcm: `idempotent`.

## Messages

- Handles: `RegisterDeviceRequest`, `SetPushTokenRequest`, `ListDevicesRequest`, `RequestGroupKeysRequest`, `ListKeyRequestsRequest`, `FulfilKeyRequestRequest`, `PublishWrappedKeysRequest`, `GetWrappedKeysRequest`, `RemoveDeviceRequest`, `GroupHeadAdvanced`, `MemberRemoved`, `HistoryRequested`, `HistoryReady`, `PurgeGroupKeysRequest`, `PurgeUserDevicesRequest`.
- Sends: `CheckMembershipRequest`, `DeviceLeftGroup`.

## Directive sections that bind

§2 always; §6 for each adapter; §7 for each message sent; §10 for the docs.

## TODO, in order

- [ ] `handle_register_device_request` in `src/handlers.rs`.
- [ ] `handle_set_push_token_request` in `src/handlers.rs`.
- [ ] `handle_list_devices_request` in `src/handlers.rs`.
- [ ] `handle_request_group_keys_request` in `src/handlers.rs`.
- [ ] `handle_list_key_requests_request` in `src/handlers.rs`.
- [ ] `handle_fulfil_key_request_request` in `src/handlers.rs`.
- [ ] `handle_publish_wrapped_keys_request` in `src/handlers.rs`.
- [ ] `handle_get_wrapped_keys_request` in `src/handlers.rs`.
- [ ] `handle_remove_device_request` in `src/handlers.rs`.
- [ ] `handle_group_head_advanced` in `src/handlers.rs`.
- [ ] `handle_member_removed` in `src/handlers.rs`.
- [ ] `handle_history_requested` in `src/handlers.rs`.
- [ ] `handle_history_ready` in `src/handlers.rs`.
- [ ] `handle_purge_group_keys_request` in `src/handlers.rs`.
- [ ] `handle_purge_user_devices_request` in `src/handlers.rs`.
- [ ] Table `device`: the queries in `repository.rs`.
- [ ] Table `wrapped_key`: the queries in `repository.rs`.
- [ ] Table `key_request`: the queries in `repository.rs`.
- [ ] `send` for `send_apns` in `effects.rs`, the real client in `provider.rs`; the audit stays green.
- [ ] `send` for `send_fcm` in `effects.rs`, the real client in `provider.rs`; the audit stays green.
- [ ] `DeviceService` field mapping in `api/src/services/device.rs` and the messages in `proto/device/v1/device.proto`.
- [ ] The message structs in `crates/messages/src/lib.rs`.
- [ ] `flows.md` and this file kept true.
