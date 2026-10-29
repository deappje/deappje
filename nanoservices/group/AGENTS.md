# group

Owns groups, memberships and invites; answers every membership check; drives group deletion and account deletion.

Rendered by the scaffold from `docs/01-deappje/manifest.yaml`.
The Directive sections that bind this nanoservice: §2 (table ownership), §3–§5
(the invariants, the author obligations, the adapter and outcome contract — it
owns processing-object types), §6 (external-effect admission — it owns adapters), §7
(cross-nanoservice rules for every message it sends), §10 (this file and
`flows.md` must stay true).

## What it owns

| Processing-object type | Key | Prefix | Spec fields | Status fields |
|---|---|---|---|---|
| `user_purge` | 1 | `upr` | `user_id` (immutable) | `phase`, `groups_remaining`, `devices_remaining`, `identity_deleted` |
| `group_purge` | 2 | `gpr` | `group_id` (immutable), `parent_purge_id` (immutable) | `phase`, `subgroups_remaining`, `objects_remaining`, `blobs_remaining`, `keys_remaining` |

Plain tables (schema `nano_group`): `groups`, `membership`, `invite`.

| External call | System | Strategy | Irreversible |
|---|---|---|---|
| `delete_identity` | kratos | `idempotent` | no |

## Files

| File | Responsibility |
|---|---|
| `src/lib.rs` | The `Group` struct, its constructor (pools, providers, calls), the schema marker, and its `basable_app::Component` impl: `loops()` lists every worker and ticker it runs (none for a plain executor). |
| `src/handlers.rs` | `impl GroupHandler<R>`: one `handle_<snake_message>` per handled message. Intent only — `create` / `update_spec` / `mark_deleted` / `nudge`; never a status write. |
| `src/types/<type>/type.rs` | The `ProcessingObjectType` declaration: spec/status structs, `TYPE_KEY`, `PUBLIC_ID_PREFIX`. |
| `src/types/<type>/adapter.rs` | The dumb column mapper: every column in BOTH `write_spec` and `read_rows`. |
| `src/types/<type>/reconciler.rs` | The level-triggered pass and the worker policy (identical on every replica). |
| `src/repository.rs` / `src/model.rs` | Typed sqlx queries over this nanoservice's pool; the row structs. |
| `src/effects.rs` | One `Call` per external call with its strategy literal; `send` is the only dispatch path. |
| `src/provider.rs` / `src/simulator.rs` | The provider trait, the HTTP client (a parsed 4xx is `SendError::Refused`), the noop; the in-memory simulator the audit and the tests drive. |
| `tests/effects_audit.rs` | One `audit!` per adapter — the audit index. Green on day one; the first thing a real provider must keep green. |
| `tests/integration.rs` | Integration tests on the testkit (a database per test). |
| `flows.md` | State machines, sequences, edge cases. |

## Messages

Handles: `CreateGroupRequest` → `Group`, `GetGroupRequest` → `Group`, `ListGroupsRequest` → `GroupList`, `CreateInviteRequest` → `Invite`, `AcceptInviteRequest` → `Membership`, `ListMembersRequest` → `MemberList`, `AddMemberRequest` → `Membership`, `RemoveMemberRequest` → `Membership`, `SetMemberRoleRequest` → `Membership`, `LeaveGroupRequest` → `Membership`, `DeleteGroupRequest` → `Group`, `DeleteAccountRequest` → `AccountDeletion`, `CheckMembershipRequest` → `MembershipCheck`.
Sends: `MemberRemoved`, `PurgeGroupObjectsRequest`, `PurgeGroupBlobsRequest`, `PurgeGroupKeysRequest`, `PurgeUserDevicesRequest`.

API: `GroupService` in `api/src/services/group.rs` (CreateGroup, DeleteGroup, DeleteAccount, GetGroup, ListGroups, CreateInvite, AcceptInvite, ListMembers, AddMember, RemoveMember, SetMemberRole, LeaveGroup).
