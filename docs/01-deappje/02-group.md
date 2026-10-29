# `group`

Owns groups, memberships and invites, and two processing-object types: `group_purge`, the
deletion of a group, which drives three other nanoservices until nothing of the group remains,
and `user_purge`, the deletion of an account. Directive §2 for every table, §3 and §5 for the
types, §6 for the Kratos call, §7 for every message.

## Tables

### `group`

| column | type | notes |
|---|---|---|
| id | uuid | primary key, server-minted |
| parent_group_id | uuid, nullable | the parent for a subgroup |
| created_by | uuid | Kratos identity id |
| created_at | timestamp | |
| deleted_at | timestamp, nullable | stamped by `DeleteGroupRequest`; the row goes when the purge finishes |

Invariants: `parent_group_id` never forms a cycle (depth is checked on create; one level is
enough for now, but the column allows deeper). No name, no avatar: those are an encrypted object
of type `group_profile` in the object log, so the server stays blind.

Queries: by id; by `parent_group_id` (subgroups of a group); all groups a user is in (join on
membership).

### `membership`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| user_id | uuid | Kratos identity id |
| role | string | `admin`, `member` or `viewer` |
| joined_at | timestamp | |

Unique `(group_id, user_id)`. Index `(user_id)`.

Invariants: the creator is inserted as `admin` in the same transaction as the group; a subgroup
membership requires a parent membership (checked under the parent's membership row, on accept
and on `AddMemberRequest`); removing a
user from a group removes them from every subgroup in the same transaction; the last admin cannot
leave while other members remain.

Roles. `admin` manages members, invites and roles and can delete the group. `member` reads and
writes. `viewer` reads: holds the group keys like anyone else, but `sync` refuses its pushes
to the log and `device` refuses it as the approver of another user's device. It may answer
history requests and re-upload files, which is serving what it holds, not writing. An invite carries
the role its joiners get (`member` or `viewer`); `SetMemberRoleRequest` (admin) changes it later.
The server enforces exactly those refusals; the clients hide the controls.

### `invite`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| code | string | unique, 10 characters from a 32-symbol alphabet, server-minted. The shared link is `/join/<code>#<secret>`: the fragment is 16 random bytes minted on the creating device, kept in that device's local store, never sent to the server; it authenticates the joiner's key request (`02-device.md`). |
| role | string | the role a joiner gets: `member` or `viewer` |
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
   `Delete`. Otherwise record the counts and `Converged.after(1m)`: `storage` deletes the group's
   bucket prefix a batch per call, so a group with many photos takes a few passes.

Worker policy: `resync` 1h, `backoff` 10s to 5m, `max_attempts` 0 (unbounded: a purge must
finish and is surfaced by age if it does not), `attempt_timeout` 60s.

Every purge request is idempotent on the receiver (delete where `group_id = $1`, report what is
left), so a repeated pass after a lost response is harmless.

## Processing object `user_purge` (prefix `upr`)

Created by `DeleteAccountRequest` (the caller deletes only themself). The handler creates the
object and returns; from then on the api crate refuses every other call from that identity
(`CheckMembershipRequest` still answers truthfully until the memberships are gone, so in-flight
sync finishes cleanly). Spec: `user_id` (immutable). Status: `phase` (`leaving`, `devices`,
`identity`, `done`), `groups_remaining`, `devices_remaining`, `identity_deleted`.

Reconcile pass:

1. `meta.deleting()` ⇒ `Delete`.
2. Memberships of the user remain ⇒ take one (deepest subgroup first, so a parent leave never
   has to cascade). If the user is the only member: stamp `deleted_at` and create a `group_purge`
   for that group (and its subgroups). Otherwise, if they are the only admin: promote the
   `member` with the earliest `joined_at`, or the earliest `viewer` if there is no member. Delete the membership, commit, send `MemberRemoved`. Status
   `groups_remaining`, `Converged.after(0)` (one group per pass keeps each transaction small).
3. Send `PurgeUserDevicesRequest { user_id }` → `PurgeResult { remaining }`; non-zero ⇒ record and
   `Converged.after(30s)`.
4. `identity_deleted` false ⇒ `kratos.delete_identity(user_id)` through `Call::dispatch`, fenced
   on the claim; `Retry` (confirm on the next pass by reading the identity: absent ⇒
   `identity_deleted = true`).
5. `Delete`.

Worker policy as `group_purge`. Objects the user authored stay: they belong to their groups and
are ciphertext, and the author inside them becomes an id without a profile, which clients
already tolerate.

## Messages

| Handles | Response | Behaviour |
|---|---|---|
| CreateGroupRequest | Group | insert group + admin membership; for a subgroup, caller must be a parent member |
| DeleteGroupRequest | Group | see `group_purge` above |
| DeleteAccountRequest | AccountDeletion | see `user_purge` above |
| GetGroupRequest | Group | caller must be a member |
| ListGroupsRequest | GroupList | all groups of the caller, with parent ids |
| CreateInviteRequest | Invite | caller must be admin |
| AcceptInviteRequest | Membership | see invite invariants |
| AddMemberRequest | Membership | subgroups only: caller is `admin` or `member` of the subgroup, the user a member of the parent; insert the membership with the given role (`member` or `viewer`); no invite, no acceptance; refused on a top-level group. The caller's device then wraps the subgroup keys for that user's devices (`02-device.md`, subgroup trust) |
| ListMembersRequest | MemberList | user ids and roles; caller must be a member |
| RemoveMemberRequest | Membership | caller admin; cascades to subgroups; sends `MemberRemoved` after commit |
| SetMemberRoleRequest | Membership | caller admin; `admin`, `member` or `viewer`; the last admin cannot be demoted while other members remain |
| LeaveGroupRequest | Membership | cascades to subgroups; sends `MemberRemoved` after commit |
| CheckMembershipRequest | MembershipCheck | `(user, group)` → member yes/no and role |

| Sends | Response | To |
|---|---|---|
| MemberRemoved | (void) | device |
| PurgeGroupObjectsRequest | PurgeResult | sync |
| PurgeGroupBlobsRequest | PurgeResult | storage |
| PurgeGroupKeysRequest | PurgeResult | device |
| PurgeUserDevicesRequest | PurgeResult | device |

`MemberRemoved` carries `(group_id, user_id, removed_at)`. It is an event (Directive §7: "an event
is not a cascade"): `device` acting on it is desirable, not required for `group`'s own
correctness. A messenger send is remote I/O, so it happens after the transaction commits
(invariant 7).

## External calls

| System | Call | Strategy | Irreversible | Argument |
|---|---|---|---|---|
| kratos | delete_identity | idempotent | yes, but the user asked for exactly this | Kratos admin `DELETE /admin/identities/{id}`; a 404 on repeat is success. Dispatched from the `user_purge` pass under its claim. |

The admin API is reached over the in-cluster service address; no secret.

## API

`GroupService`: CreateGroup, GetGroup, ListGroups, CreateInvite, AcceptInvite, AddMember, ListMembers,
RemoveMember, SetMemberRole, LeaveGroup, DeleteGroup, DeleteAccount. Every method requires a Kratos session; the api crate
passes the identity id as the caller.

## Tests

- Unit: invite acceptance under expiry, revocation and max uses; subgroup accept without parent
  membership is refused, and so is `AddMember` of a non-member of the parent or on a top-level
  group; last-admin rule, also against demotion; an invite's role lands on the
  membership; the purge pass as a function of
  `(subgroups_remaining, three counts)` → `Outcome`.
- Integration (testkit, database per test): remove from parent cascades to subgroups in one
  transaction; two concurrent accepts of a single-use invite admit exactly one; `MemberRemoved`
  is sent exactly once per removal; deleting a parent with a subgroup purges the subgroup first
  and the parent's pass waits; a lost `PurgeResult` is retried and the receiver's purge is
  idempotent; an account purge for a user who is sole admin of one group, sole member of
  another and plain member of a third ends with a promoted admin, a group purge, a rotation
  push, no device rows and one `delete_identity` call.
- `effecttest`: `kratos.delete_identity` with ack loss and a 404 on repeat.
