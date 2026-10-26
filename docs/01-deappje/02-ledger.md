# `ledger`

Owns the relay: a 48-hour window of ciphertext rows per group, the group's counter, the
tombstones until every device has acked them, one acked cursor per device, and a row per pending
history request. Plain executor with three hourly sweeps. Member devices are the store; this
nanoservice forgets everything live after 48 hours. Directive §2 per table, §7 (counter row under
`FOR UPDATE`; a boolean or a number is all that crosses) for its messages.

## Tables

### `object`

| column | type | notes |
|---|---|---|
| id | uuid | primary key, client-minted |
| group_id | uuid | |
| type | int32 | client-defined type number |
| schema_version | int32 | |
| key_id | int32 | which group key encrypts `value` |
| value | bytes, nullable | ciphertext (≤ 1 MiB); null on a tombstone |
| blob_id | uuid, nullable | a `storage` blob of the same group; null on a tombstone |
| seq | int64 | per group, strictly increasing; an edit takes a new seq, so a row's seq identifies its state |
| created_by_device | uuid | |
| deleted | bool | tombstone flag; the tombstone's own `seq` is its deletion mark |
| updated_at | timestamp | |

Indexes: unique `(group_id, seq)`; `(group_id, deleted, seq)` for garbage collection;
`(updated_at)` for eviction. The parent of an object lives inside its encrypted value: the server
never sees the structure of a group's data.

### `group_head`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key |
| head_seq | int64 | last assigned sequence |
| evicted_through_seq | int64 | every live row with `seq ≤` this has been evicted |

Created with zeros on the first push to a group.

### `group_device`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key (with device_id) |
| device_id | uuid | |
| acked_seq | int64, nullable | the cursor this device has pulled through and applied |
| updated_at | timestamp | |

Index `(device_id)`. Upserted by `AckSeqRequest`; deleted on `DeviceLeftGroup` from `device`.
A device that never acks blocks garbage collection for its group until it is removed.

### `history_request`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| requester_device | uuid | |
| from_seq | int64 | |
| to_seq | int64 | |
| object_id | uuid, nullable | a request for one object instead of a range |
| status | string | `pending`, `fulfilled` |
| bundle_id | uuid, nullable | the `storage` blob of kind `bundle` |
| created_at | timestamp | |

Index `(group_id, status)`. Rows older than 48 hours are dropped by the sweep.

## Push

`PushObjectsRequest { group_id, objects: [{ id, type, schema_version, key_id, value?, blob_id?,
base_seq, deleted }] }`. `base_seq` is the seq of the row the device edited
from, 0 for a new object.

Before the transaction: `CheckMembershipRequest`, and the role must be `admin` or `member` (a
`viewer` is refused); for every `blob_id`, `CheckBlobRequest` must
answer `present` for this group (remote I/O outside the transaction, invariant 7). A `value`
over 1 MiB refuses the whole batch.

One transaction:

1. `SELECT ... FROM group_head WHERE group_id = $1 FOR UPDATE` (insert if absent).
2. Per object, in request order, read the current row:
   - **No row** (new, or evicted) ⇒ accept.
   - **`base_seq == seq`** ⇒ accept: write the object at `seq = head + 1`, head advances. If
     `deleted`: null `value` and `blob_id`, `deleted = true`.
   - **Mismatch** ⇒ conflict `{ id, current }` with the stored row as it is, tombstone or not.
3. Commit.

That is the whole rule. A tombstone is not special on the server: a device that has not seen the
delete holds an older seq and conflicts; a device that has seen it and pushes a live object with
the tombstone's seq as its base is un-deleting, which is the client's decision. A delete whose
base is stale conflicts like any edit and is pushed again with the current seq.

After commit: `GroupHeadAdvanced { group_id, head_seq, writer_device }` once; `ObjectDeleted
{ group_id, object_id, blob_id }` per tombstoned row that had a blob.

Response: `PushResult { accepted: [{ id, seq }], conflicts: [{ id, current }], head_seq }`. The
objects themselves come back on the next pull with everyone else's; the device skips entries it
already holds at that seq.

A tombstone's deletion mark is its seq, so "device acked cursor `c`" means "device has applied
every tombstone with `seq ≤ c`", with no second counter. There is no "pull before you push"
check: a stale edit or re-creation of a deleted object conflicts on the seq, and the tombstone
stays until this very device has acked past it. There is no cascade either: the deleting client
puts the children it holds in the same batch (one push is one transaction, so they take
consecutive seqs together), and a child it did not hold is tombstoned by the first device that
holds it and sees the parent's tombstone. The server could not do this completely anyway, since
children older than 48 hours are already evicted from it.

Why a locked counter row and not a sequence: a sequence is non-transactional, so a lower value can
commit after a higher one and a client that pulled in between misses it forever (Directive §7).

Accepting a push for an evicted object on the client's word is safe: a deleted object is never
evicted, only hard-deleted after this device acked past its tombstone, so nothing can be
resurrected, and the first re-creation of an evicted row makes every later stale base conflict,
which the client rebases (`01-architecture.md`).

## Pull and ack

- `PullChangesRequest { group_id, since_seq, limit }` → `ChangeSet { entries (seq ascending,
  live rows still present and tombstones), head_seq, min_acked_seq?, evicted_through_seq,
  more }`. If `since_seq < evicted_through_seq`, live entries in
  `(since_seq, evicted_through_seq]` may be gone from the server: the device asks a member device
  for that range with `RequestHistoryRequest`. Tombstones in that range are still here, since
  they stay until acked.
- `AckSeqRequest { group_id, seq }` → `AckResult { min_acked_seq? }`. Sent by a device after it
  has applied a `ChangeSet` with `more = false`, with that response's `head_seq`. Upserts
  `group_device`. `seq > head_seq` is refused. A value below the device's stored ack means the
  device's local state went backwards (restored from a backup): the answer is `resync_required`
  and the device drops its local store and requests history from zero. The device drops its own
  local tombstones with `seq ≤ min_acked_seq`, since no device can ever hand it a live copy of
  those again.
- `GetHeadsRequest { group_ids }` → `HeadList [{ group_id, head_seq, min_acked_seq? }]`, one
  `CheckMembershipRequest` per id.

`min_acked_seq` is `MIN(acked_seq)` over the group's `group_device` rows; null if any row is null
or there are no rows.

## History requests (member devices are the store)

`RequestHistoryRequest { group_id, from_seq, to_seq, object_id? }` → `HistoryRequest`: insert a
`pending` row, send `HistoryRequested { request_id, group_id, requester_device, from_seq, to_seq,
object_id }` to `device`, which wakes every key-holding device of the group except the requester.
A holder that wakes lists pending requests (`ListHistoryRequestsRequest { group_id }`), packs the
entries it holds in the range (ciphertext, each with id, type, schema_version, key_id, seq,
deleted, blob_id; tombstones included), newest first up to the bundle size cap,
encrypts the bundle under the current group key named in its header, uploads it as a blob of
kind `bundle` with `request_id`, and confirms. `storage` then sends `BundleReady { request_id,
bundle_id }`; the handler marks the row `fulfilled` and sends `HistoryReady { request_id,
requester_device, bundle_id }` to `device`, which wakes the requester. A late holder sees the row
is no longer pending and does nothing; a duplicate bundle is harmless and expires in 48 hours.
The requester ingests the bundle like a `ChangeSet` (an entry it already holds at a higher
seq is ignored; a tombstone wins) and, if the bundle stops short of `from_seq`, asks again
for the rest, up to the lowest seq it received. No retries, no timeouts on the server: a requester that hears nothing
asks again, and the client shows "waiting for a member to come online".

This is the only way history older than 48 hours reaches a device: a new member, a new device, a
device that was offline longer than the window, or a device scrolling past what it holds.

## Sweeps (schedules, hourly, idempotent)

- `evict_objects`: per group, `S = MAX(seq)` of live rows with `updated_at < now - 48h`; delete
  live rows with `seq ≤ S` (an edit takes a new seq, so seq order is update order); set
  `evicted_through_seq = GREATEST(evicted_through_seq, S)`. Batched 10k rows per statement.
- `gc_tombstones`: per group with a non-null `min_acked_seq`, delete tombstones with
  `seq ≤ min_acked_seq`.
- `expire_history_requests`: delete rows older than 48 hours.

Postgres therefore holds, per group, two days of activity plus tombstones nobody has acked yet,
whatever the group's age.

## Messages

| Handles | Response | From |
|---|---|---|
| PushObjectsRequest | PushResult | api |
| PullChangesRequest | ChangeSet | api |
| AckSeqRequest | AckResult | api |
| GetHeadsRequest | HeadList | api |
| RequestHistoryRequest | HistoryRequest | api |
| ListHistoryRequestsRequest | HistoryRequestList | api |
| DeviceLeftGroup | (void) | device: delete the `group_device` row |
| BundleReady | (void) | storage |
| PurgeGroupObjectsRequest | PurgeResult | group |

`PurgeGroupObjectsRequest { group_id }` (from `group`'s purge pass): delete every `object`,
`group_head`, `group_device` and `history_request` row for the group, batched, and answer
`PurgeResult { remaining }`. No tombstones: memberships are already gone, so a client gets "not
a member" and purges locally. Idempotent.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |
| CheckBlobRequest | Blob | storage |
| GroupHeadAdvanced | (void) | device |
| ObjectDeleted | (void) | storage |
| HistoryRequested | (void) | device |
| HistoryReady | (void) | device |

## API

`LedgerService`: PushObjects, PullChanges, AckSeq, GetHeads, RequestHistory,
ListHistoryRequests. The api crate maps the proto entry to the plain Rust struct one to one;
`value` and `blob_id` pass through untouched.

## Tests

- Unit: CAS-on-seq acceptance matrix (no row, match, mismatch, the same three on a tombstone);
  eviction floor arithmetic.
- Integration: two replicas pushing to one group concurrently produce a gap-free sequence; a pull
  during a concurrent push never returns a seq the next pull skips; a batch of a parent and its
  children takes consecutive seqs in one transaction; **the ack invariant**: after N devices interleave pushes, deletes, pulls and acks
  at random, every tombstone the gc removed had `seq ≤` every device's ack at the time, every
  device that acked `c` had pulled every tombstone with `seq ≤ c`, and a stale device's edit of
  a deleted object always conflicts; eviction leaves tombstones and sets the floor; an ack below
  the stored one answers `resync_required`; a history request is fulfilled once and a late
  bundle is ignored.
- Property test: random interleavings of push/pull/ack/evict across N simulated devices, with
  history served by a simulated holder, converge to the same row per object on every device, and
  no deleted object is ever live on any device after every device has acked past its tombstone.
