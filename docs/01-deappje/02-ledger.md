# `ledger`

Owns the object log: every object of every group, its version, its position in the group's
sequence, and its tombstone. Plain executor; the serialization point of the whole system is the
`group_head` counter row. Directive §2 per table, §7 (counter row under `FOR UPDATE`; derived
state only) for its messages.

## Tables

### `object`

| column | type | notes |
|---|---|---|
| id | uuid | primary key, client-minted (doubles as the outbox idempotency key) |
| group_id | uuid | |
| parent_id | uuid, nullable | within the same group |
| type | int32 | client-defined type number |
| schema_version | int32 | payload schema of this row |
| key_id | int32 | which group key encrypts `value` |
| value | bytes, nullable | ciphertext; null on a tombstone |
| blob_id | uuid, nullable | a `storage` blob of the same group; null on a tombstone |
| version | int64 | per-object, starts at 1 |
| seq | int64 | per-group, strictly increasing |
| created_by_device | uuid | |
| deleted | bool | tombstone flag |
| deleted_at | timestamp, nullable | |
| updated_at | timestamp | |

Indexes: unique `(group_id, seq)`; `(group_id, type, seq)` for backfill; `(parent_id)` for
cascade; `(deleted_at)` for garbage collection.

### `group_head`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key |
| head_seq | int64 | last assigned sequence |

Created with `head_seq = 0` on the first push to a group.

## Push

`PushObjectsRequest { group_id, base_cursor?, objects: [{ id, parent_id?, type, schema_version,
key_id, value?, blob_id?, base_version, deleted }] }`.

Before the transaction: `CheckMembershipRequest`; for every `blob_id`, `CheckBlobRequest` must
answer the blob is in this group and `available` or `pending_upload` (remote I/O stays outside the
transaction, invariant 7). A `value` larger than 1 MiB is rejected for the whole batch (a CRDT
document grows with its content; anything bigger belongs in a blob).

One transaction:

1. `SELECT ... FROM group_head WHERE group_id = $1 FOR UPDATE` (insert if absent).
2. Per object, in request order: read the current row. Accept when `base_version` equals the
   stored `version` (0 when absent). On accept: `version + 1`, `seq = head + 1`, head advances.
   If `deleted`, null `value` and `blob_id`, set `deleted_at`, and tombstone every descendant
   (recursive by `parent_id`) each with its own next `seq`. On a mismatch: record a conflict
   `{ id, current: Object }` with the stored row as it is (ciphertext included), change nothing.
   The caller must learn the row either way, so returning it saves the pull a merging client
   would otherwise make before it can retry.
3. Commit.

After commit: `GroupHeadAdvanced { group_id, head_seq, writer_device }` once; `ObjectDeleted
{ group_id, object_id, blob_id }` per tombstoned row that had a blob.

Response: `PushResult { accepted: [{ id, version, seq }], conflicts: [...], head_seq }`.

Why a locked counter row and not a sequence: a sequence is non-transactional, so a lower value can
commit after a higher one and a client that pulled in between misses it forever (Directive §7).

Deletes always win: a push with `deleted = true` is accepted regardless of `base_version`. An edit
whose `base_version` matches a tombstone is a conflict; the client shows "deleted".

The server does not merge anything: every value is a CRDT document (see `01-architecture.md`),
so the client folds `current.value` into its local document and pushes again with
`current.version` as `base_version`. The retry is an ordinary push.

## Pull, backfill, heads, reconciliation

- `PullChangesRequest { group_id, since_seq, limit }` → `ChangeSet { objects (seq ascending,
  tombstones included), head_seq, more }`. If `since_seq` is older than the oldest retained
  tombstone's seq for that group, the response carries `reconcile_required = true` and the
  client runs reconciliation.
- `BackfillRequest { group_id, type?, before_seq, limit }` → `ChangeSet` with rows of that type
  (or of every type when `type` is absent: the timeline scrolling back) and `seq < before_seq`,
  descending, not deleted. Uses `(group_id, type, seq)` or `(group_id, seq)` accordingly.
- `GetHeadsRequest {}` → `HeadList [{ group_id, head_seq }]` for every group the caller is a
  member of (`ListGroupsRequest` is api-only, so `ledger` asks `group` through
  `CheckMembershipRequest` for the group ids the caller supplies; the client supplies the ids it
  knows).
- `ListObjectIdsRequest { group_id }` → `ObjectIdList [{ id, version, deleted }]`: the client
  deletes what is missing or tombstoned locally, fetches what is newer, pushes what is only local.

## Garbage collection

Schedule `gc_tombstones`, every 24h: delete `object` rows with `deleted = true` and
`deleted_at < now - 90d`. Per group, `oldest_retained_tombstone_seq` is derived at pull time
(`min(seq) where deleted`), not stored.

## Messages

| Handles | Response |
|---|---|
| PushObjectsRequest | PushResult |
| PullChangesRequest | ChangeSet |
| BackfillRequest | ChangeSet |
| GetHeadsRequest | HeadList |
| ListObjectIdsRequest | ObjectIdList |
| PurgeGroupObjectsRequest | PurgeResult |

`PurgeGroupObjectsRequest { group_id }` (from `group`'s purge pass): delete every `object` row
and the `group_head` row for the group in one transaction, batched (10k rows per statement so a
large group does not hold the lock long), and answer `PurgeResult { remaining }` with the count
still present. No tombstones are written: memberships are already gone, so no client can pull
the group; a client with a stale cursor gets "not a member" and purges locally. Idempotent.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |
| CheckBlobRequest | Blob | storage |
| GroupHeadAdvanced | (void) | device |
| ObjectDeleted | (void) | storage |

## API

`LedgerService`: PushObjects, PullChanges, Backfill, GetHeads, ListObjectIds. The api crate maps
the proto `Object` to the plain Rust struct one to one; `value` and `blob_id` are passed through
untouched.

## Tests

- Unit: CAS acceptance matrix (absent/0, match, mismatch, tombstone); cascade collects the full
  subtree; `reconcile_required` boundary.
- Integration: two replicas pushing to one group concurrently produce a gap-free sequence; a
  pull during a concurrent push never returns a seq the next pull skips; delete cascades in one
  transaction; gc removes only expired tombstones.
- Property test: random interleavings of push/pull across N simulated clients converge to the
  same id/version set after reconciliation; with several clients merging on one object, every
  retry carries a `base_version` the server has seen and the object's final version equals the
  number of accepted pushes.
- Unit: a conflict returns the stored row unchanged; a batch with an oversized `value` is refused
  as a whole.
