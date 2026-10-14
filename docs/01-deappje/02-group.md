# `group`

Owns groups, memberships and invites, and one processing-object type: `group_purge`, the
deletion of a group, which drives three other nanoservices until nothing of the group remains.
Directive §2 for every table, §3 and §5 for the type, §7 for every message.

## Tables

### `group`

| column | type | notes |
|---|---|---|
| id | uuid | primary key, server-minted |
| parent_group_id | uuid, nullable | the parent for a subgroup |
| created_by | uuid | Kratos identity id |
| created_at | timestamp | |
| deleted_at | timestamp, nullable | reserved for group deletion (`later`) |

Invariants: `parent_group_id` never forms a cycle (depth is checked on create; one level is
enough for now, but the column allows deeper). No name, no avatar: those are an encrypted object
of type `group_profile` in the ledger, so the server stays blind.

Queries: by id; by `parent_group_id` (subgroups of a group); all groups a user is in (join on
membership).

### `membership`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| user_id | uuid | Kratos identity id |
| role | string | `admin` or `member` |
| joined_at | timestamp | |

Unique `(group_id, user_id)`. Index `(user_id)`.

Invariants: the creator is inserted as `admin` in the same transaction as the group; a subgroup
membership requires a parent membership (checked under the parent's membership row); removing a
user from a group removes them from every subgroup in the same transaction; the last admin cannot
leave while other members remain.

### `invite`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| code | string | unique, 10 characters from a 32-symbol alphabet, server-minted |
| created_by | uuid | |
| expires_at | timestamp | |
| max_uses | int32 | 0 = unbounded |
| uses | int32 | |
| revoked | bool | |

Accepting locks the invite row, checks expiry, revocation and `uses < max_uses`, inserts the
membership, increments `uses`, commits. A subgroup invite also checks parent membership.

## Processing object `group_purge` (prefix `gpr`)

Created by `DeleteGroupRequest` (caller must be admin): in one transaction the handler stamps
`group.deleted_at`, deletes the group's memberships and invites, and `TypedStore::create`s one
purge object for the group and one per subgroup (recursively). From that commit on,
`CheckMembershipRequest` answers "not a member" for everyone, so no client can read or write
the group; a client that was offline learns of the deletion from that answer and purges locally.

Spec: `group_id` (immutable), `parent_purge_id` (nullable, immutable: set on a subgroup's purge).
Status: `phase` (`purging`, `done`), `objects_remaining`, `blobs_remaining`, `keys_remaining`
(int64, nullable), `subgroups_remaining` (int32).

Reconcile pass (level-triggered, one `Outcome`):

1. `meta.deleting()` ⇒ `Delete` (a purge object is only ever deleted by its own pass; nothing
   external to confirm).
2. Count purge objects whose `parent_purge_id` is this one (own table, repository query). If
   any ⇒ status `subgroups_remaining = n`, `Converged.after(30s)`.
3. Send `PurgeGroupObjectsRequest`, `PurgeGroupBlobsRequest`, `PurgeGroupKeysRequest` (between
   fences, invariant 7), each returning `PurgeResult { remaining }`. A failed send ⇒ `Retry`.
4. All three zero ⇒ delete the `group` row (repository, own table), status `phase = done`,
   `Delete`. Otherwise record the counts and `Converged.after(1m)`: blobs drain through their
   own reconcilers in `storage`, so a group with many photos takes minutes.

Worker policy: `resync` 1h, `backoff` 10s to 5m, `max_attempts` 0 (unbounded: a purge must
finish and is surfaced by age if it does not), `attempt_timeout` 60s.

Every purge request is idempotent on the receiver (delete where `group_id = $1`, report what is
left), so a repeated pass after a lost response is harmless.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| CreateGroupRequest | Group | insert group + admin membership; for a subgroup, caller must be a parent member |
| DeleteGroupRequest | Group | see `group_purge` above |
| GetGroupRequest | Group | caller must be a member |
| ListGroupsRequest | GroupList | all groups of the caller, with parent ids |
| CreateInviteRequest | Invite | caller must be admin |
| AcceptInviteRequest | Membership | see invite invariants |
| ListMembersRequest | MemberList | user ids and roles; caller must be a member |
| RemoveMemberRequest | Membership | caller admin; cascades to subgroups; sends `MemberRemoved` after commit |
| LeaveGroupRequest | Membership | cascades to subgroups; sends `MemberRemoved` after commit |
| CheckMembershipRequest | MembershipCheck | `(user, group)` → member yes/no and role |

| Sends | Response | To |
|---|---|---|
| MemberRemoved | (void) | device |
| PurgeGroupObjectsRequest | PurgeResult | ledger |
| PurgeGroupBlobsRequest | PurgeResult | storage |
| PurgeGroupKeysRequest | PurgeResult | device |

`MemberRemoved` carries `(group_id, user_id, removed_at)`. It is an event (Directive §7: "an event
is not a cascade"): `device` acting on it is desirable, not required for `group`'s own
correctness. A messenger send is remote I/O, so it happens after the transaction commits
(invariant 7).

## API

`GroupService`: CreateGroup, GetGroup, ListGroups, CreateInvite, AcceptInvite, ListMembers,
RemoveMember, LeaveGroup, DeleteGroup. Every method requires a Kratos session; the api crate
passes the identity id as the caller.

## Tests

- Unit: invite acceptance under expiry, revocation and max uses; subgroup accept without parent
  membership is refused; last-admin rule; the purge pass as a function of
  `(subgroups_remaining, three counts)` → `Outcome`.
- Integration (testkit, database per test): remove from parent cascades to subgroups in one
  transaction; two concurrent accepts of a single-use invite admit exactly one; `MemberRemoved`
  is sent exactly once per removal; deleting a parent with a subgroup purges the subgroup first
  and the parent's pass waits; a lost `PurgeResult` is retried and the receiver's purge is
  idempotent.
