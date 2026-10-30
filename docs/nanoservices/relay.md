# relay

The relay — ciphertext per group until every holder has acked it, 672 hours at most, tombstones included; the counter; per-device acked cursor; history requests.

Design: `docs/01-deappje/02-relay.md`. Code:
`nanoservices/relay/` (read its `AGENTS.md` first).

## Owns

- Table `object` (`id`, `group_id`, `value`, `seq`, `deleted`, `uploaded_at`).
- Table `group_head` (`group_id`, `head_seq`, `evicted_through_seq`).
- Table `group_device` (`group_id`, `device_id`, `acked_seq`, `updated_at`).
- Table `history_request` (`id`, `group_id`, `requester_device`, `from_seq`, `to_seq`, `object_id`, `claimed_by`, `claimed_at`, `created_at`).
- Schedule `evict_objects` every 1h.
- Schedule `expire_history_requests` every 1h.

## Messages

- Handles: `PushObjectsRequest`, `PullChangesRequest`, `AckSeqRequest`, `GetHeadsRequest`, `RequestHistoryRequest`, `ListHistoryRequestsRequest`, `ClaimHistoryRequest`, `PushHistoryRequest`, `DeviceLeftGroup`, `PurgeGroupObjectsRequest`.
- Sends: `CheckMembershipRequest`, `GroupHeadAdvanced`, `ObjectDeleted`, `HistoryRequested`, `HistoryReady`.

## Directive sections that bind

§2 always; §7 for each message sent; §10 for the docs.

## TODO, in order

- [x] `handle_push_objects_request` in `src/handlers.rs`.
- [x] `handle_pull_changes_request` in `src/handlers.rs`.
- [x] `handle_ack_seq_request` in `src/handlers.rs`.
- [x] `handle_get_heads_request` in `src/handlers.rs`.
- [x] `handle_request_history_request` in `src/handlers.rs`.
- [x] `handle_list_history_requests_request` in `src/handlers.rs`.
- [x] `handle_claim_history_request` in `src/handlers.rs`.
- [x] `handle_push_history_request` in `src/handlers.rs`.
- [x] `handle_device_left_group` in `src/handlers.rs`.
- [x] `handle_purge_group_objects_request` in `src/handlers.rs`.
- [x] Table `object`: the queries in `repository.rs`.
- [x] Table `group_head`: the queries in `repository.rs`.
- [x] Table `group_device`: the queries in `repository.rs`.
- [x] Table `history_request`: the queries in `repository.rs`.
- [x] `tick_evict_objects` in `worker.rs`.
- [x] `tick_expire_history_requests` in `worker.rs`.
- [x] `RelayService` field mapping in `api/src/services/relay.rs` and the messages in `proto/relay/v1/relay.proto`.
- [x] The message structs in `crates/messages/src/lib.rs`.
- [x] `flows.md` and this file kept true.

Landed 2026-10-30 (`nanoservices/relay`, the proto and the api mapping, the
messages). Decisions beyond the plan's text are recorded in
`docs/01-deappje/02-relay.md`, "Implementation record", and the flows in
`nanoservices/relay/flows.md`.
