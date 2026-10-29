# group

Owns groups, memberships and invites; answers every membership check; drives group deletion and account deletion.

Design: `docs/01-deappje/02-group.md`. Code:
`nanoservices/group/` (read its `AGENTS.md` first).

## Owns

- Processing-object type `user_purge` (key 1, `upr_…`): spec `user_id`; status `phase`, `groups_remaining`, `devices_remaining`, `identity_deleted`.
- Processing-object type `group_purge` (key 2, `gpr_…`): spec `group_id`, `parent_purge_id`; status `phase`, `subgroups_remaining`, `objects_remaining`, `blobs_remaining`, `keys_remaining`.
- Table `groups` (`id`, `parent_group_id`, `created_by`, `created_at`, `deleted_at`).
- Table `membership` (`id`, `group_id`, `user_id`, `role`, `joined_at`).
- Table `invite` (`id`, `group_id`, `code`, `role`, `created_by`, `expires_at`, `max_uses`, `uses`, `revoked`).
- External call `delete_identity` on kratos: `idempotent`.

## Messages

- Handles: `CreateGroupRequest`, `GetGroupRequest`, `ListGroupsRequest`, `CreateInviteRequest`, `AcceptInviteRequest`, `ListMembersRequest`, `AddMemberRequest`, `RemoveMemberRequest`, `SetMemberRoleRequest`, `LeaveGroupRequest`, `DeleteGroupRequest`, `DeleteAccountRequest`, `CheckMembershipRequest`.
- Sends: `MemberRemoved`, `PurgeGroupObjectsRequest`, `PurgeGroupBlobsRequest`, `PurgeGroupKeysRequest`, `PurgeUserDevicesRequest`.

## Directive sections that bind

§2 always; §3, §4, §5 for each processing-object type; §6 for each adapter; §7 for each message sent; §10 for the docs.

## TODO, in order

- [ ] `handle_create_group_request` in `src/handlers.rs`.
- [ ] `handle_get_group_request` in `src/handlers.rs`.
- [ ] `handle_list_groups_request` in `src/handlers.rs`.
- [ ] `handle_create_invite_request` in `src/handlers.rs`.
- [ ] `handle_accept_invite_request` in `src/handlers.rs`.
- [ ] `handle_list_members_request` in `src/handlers.rs`.
- [ ] `handle_add_member_request` in `src/handlers.rs`.
- [ ] `handle_remove_member_request` in `src/handlers.rs`.
- [ ] `handle_set_member_role_request` in `src/handlers.rs`.
- [ ] `handle_leave_group_request` in `src/handlers.rs`.
- [ ] `handle_delete_group_request` in `src/handlers.rs`.
- [ ] `handle_delete_account_request` in `src/handlers.rs`.
- [ ] `handle_check_membership_request` in `src/handlers.rs`.
- [ ] `user_purge`: the reconcile pass in `types/user_purge/reconciler.rs`; the worker policy.
- [ ] `group_purge`: the reconcile pass in `types/group_purge/reconciler.rs`; the worker policy.
- [ ] Table `groups`: the queries in `repository.rs`.
- [ ] Table `membership`: the queries in `repository.rs`.
- [ ] Table `invite`: the queries in `repository.rs`.
- [ ] `send` for `delete_identity` in `effects.rs`, the real client in `provider.rs`; the audit stays green.
- [ ] `GroupService` field mapping in `api/src/services/group.rs` and the messages in `proto/group/v1/group.proto`.
- [ ] The message structs in `crates/messages/src/lib.rs`.
- [ ] `flows.md` and this file kept true.
