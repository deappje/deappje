# `ledger`

Owns the relay: a window of ciphertext rows per group, tombstones included, that every holder
has not yet acked, 168 hours at most; the group's counter; one acked cursor per device; and a
row per pending history request. Plain executor with two hourly sweeps. Member devices are the
store, each of them holding the whole group; this nanoservice forgets everything after 168
hours. Directive §2 per table, §7 (counter row under
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
| value | bytes, nullable | ciphertext (≤ 1 MiB); null on a tombstone; a blob reference (slot, hash, size, content key) is inside it |
| seq | int64 | per group, strictly increasing; an edit takes a new seq, so a row's seq identifies its state |
| created_by_device | uuid | |
| deleted | bool | tombstone flag; the tombstone's own `seq` is its deletion mark; a tombstone is evicted like any row |
| uploaded_at | timestamp | when this row landed on the server, by push or by a history re-insert; evicted 168 hours later |

Indexes: unique `(group_id, seq)`; `(uploaded_at)` for eviction. The parent of an object lives inside its encrypted value: the server
never sees the structure of a group's data.

### `group_head`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key |
| head_seq | int64 | last assigned sequence |
| evicted_through_seq | int64 | the eviction floor: every row written at `seq ≤` this has been evicted once, by age or because every holder had acked it (a history re-insert may put a copy back); a device whose cursor is below it starts over |

Created with zeros on the first push to a group.

### `group_device`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key (with device_id) |
| device_id | uuid | |
| acked_seq | int64 | the device holds every row of the group with `seq ≤` this, from seq 1 up |
| updated_at | timestamp | |

Index `(device_id)`. Upserted by `AckSeqRequest`, on the device's word; deleted on `DeviceLeftGroup`
from `device`, and by `evict_objects` when the device's ack is below the new floor: a device that
has not acked a row that gets evicted is evicted with it. Every row therefore has
`acked_seq ≥ evicted_through_seq`, and a row means a complete holder: a device acks only once it
holds the whole group, so "no row" is the whole check (no push, no claim, no wake-up), and
`min_acked_seq`, the minimum over the group's rows, is the seq every holder has reached.

### `history_request`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| requester_device | uuid | |
| from_seq | int64 | 1 for a device that joins or starts over |
| to_seq | int64 | the eviction floor the requester saw; rows above it are still on the server |
| object_id | uuid, nullable | a request for one object instead of a range |
| status | string | `pending`, `claimed`, `fulfilled` |
| claimed_by | uuid, nullable | the holder device answering it |
| claimed_at | timestamp, nullable | a claim older than 5 minutes without `complete` counts as abandoned |
| created_at | timestamp | |

Index `(group_id, status)`. Rows older than 168 hours are dropped by the sweep.

## Push

`PushObjectsRequest { group_id, objects: [{ id, type, schema_version, key_id, value?, base_seq,
deleted }] }`. `base_seq` is the seq of the row the device edited
from, 0 for a new object.

Before the transaction: `CheckMembershipRequest`, and the role must be `admin` or `member` (a
`viewer` is refused). A `value` over 1 MiB refuses the whole batch.

One transaction:

1. `SELECT ... FROM group_head WHERE group_id = $1 FOR UPDATE` (insert if absent), and the
   device's `group_device` row.
2. **Not a holder?** If there is no row, refuse the whole batch with `behind_floor
   { evicted_through_seq }`, whatever it holds: the device has never acked, or an eviction passed
   its ack and took its row; either way it has not seen everything the server has evicted, so
   its word is worth nothing, and `base_seq = 0` is its word too. The server cannot tell a
   create from an edit of a row it no longer has, so a batch of "creates" from such a device
   could carry a hard-deleted object straight back in. A new member therefore pulls, gets the
   history and acks before its first post.
3. Per object, in request order, read the current row:
   - **No row** (new, or evicted) ⇒ accept.
   - **`base_seq == seq`** ⇒ accept: write the object at `seq = head + 1`, head advances. If
     `deleted`: null `value`, `deleted = true`.
   - **Mismatch** ⇒ conflict `{ id, current }` with the stored row as it is, tombstone or not.
4. Commit.

That is the whole rule. A tombstone is not special on the server: a holder that has not seen the
delete holds an older seq and conflicts while the tombstone is here, and once the tombstone has
gone every holder has acked it, so no holder can be in that state; a device that has seen it
and pushes a live object with the tombstone's seq as its base is un-deleting, which is the
client's decision. A delete whose base is stale conflicts like any edit and is pushed again with
the current seq.

After commit: `GroupHeadAdvanced { group_id, head_seq, writer_device }` once; `ObjectDeleted
{ group_id, object_id }` per tombstone, so `storage` can delete the object's files; the server
does not know which objects have any.

Response: `PushResult { accepted: [{ id, seq }], conflicts: [{ id, current }], head_seq }`, or
`behind_floor`. The objects themselves come back on the next pull with everyone else's; the
device skips entries it already holds at that seq.

A tombstone's deletion mark is its seq, and a tombstone is a row like any other: it stays here
until every holder has acked it, 168 hours at most, and a device keeps the ones it pulled until
the minimum ack passes them. There is no pull-to-head check before a push: the one rule is that
the pusher is a holder, and it is what makes the no-row branch safe. A holder has seen every
row the server evicted, so an evicted row it edits was last written at a seq it holds, and any
later write would still be a row here and conflict. From a device that is not a holder the same
push is a silent overwrite: a device that went offline holding an object at seq 300 while
another device edited it at seq 2000, and that row was evicted, would drop its stale state into
the empty slot at a new seq, and every holder would apply it as the later write, with no
conflict anywhere. Hence the refusal. There is no cascade either: the deleting client puts the
children it holds in the same batch (one push is one transaction, so they take consecutive seqs
together), and a child it did not hold is tombstoned by the first device that holds it and sees
the parent's tombstone. The server could not do this completely anyway, since children older
than the window are already evicted from it.

Why a locked counter row and not a sequence: a sequence is non-transactional, so a lower value can
commit after a higher one and a client that pulled in between misses it forever (Directive §7).

## Pull and ack

- `PullChangesRequest { group_id, since_seq, limit }` → `ChangeSet { entries (seq ascending,
  rows still present, tombstones included), head_seq, evicted_through_seq, min_acked_seq?,
  more }`. If `since_seq < evicted_through_seq` the device is behind the floor: rows in
  `(since_seq, evicted_through_seq]` are gone from the server, tombstones among them, and so is
  the device's own `group_device` row, so it cannot know which of the objects it holds were
  deleted meanwhile. It starts over: it throws the group away (objects, tombstones and every
  unpushed write; files stay as a content-addressed cache, since a file is reachable only
  through an object's reference and brings nothing back by itself) and takes the path of a new
  device. That path: pull from 0 and apply the window; ask a holder for everything older with
  `RequestHistoryRequest { from_seq: 1, to_seq: evicted_through_seq }`; when `HistoryReady`
  comes, pull from 0 again, which returns the restored rows with the window (entries it already
  holds at that seq are skipped); if that pull reports a floor above the request's `to_seq`, an
  eviction in between took rows it has not seen, so it asks for `(to_seq, floor]` and pulls
  again, until the floor it has caught up to is the floor the pull reports; then ack the head.
  Until then its pushes are refused. Objects come whole, from seq 1; files come lazily: a
  full-size file when it is viewed, a thumbnail for a grid. A device drops its local tombstones
  with `seq ≤ min_acked_seq`: every holder has applied them, and a device that is not a holder
  starts over, so no device can ever hand it a live copy of those again.
- `AckSeqRequest { group_id, seq }` → `AckResult { evicted_through_seq, min_acked_seq }`. Sent by
  a device after it has applied a `ChangeSet` with `more = false`, with that response's
  `head_seq`, and only when it holds every row of the group through that seq: for a device that
  joined or started over, after its history request is fulfilled. Upserts `group_device` on the
  device's word. `seq > head_seq` is refused, and `seq < evicted_through_seq` is refused as
  `behind_floor`: the device starts over. An ack below the stored one but at or above the floor
  (a restored backup) is stored as it is: the rows after it are still here. A device the
  eviction took whose cursor was already past the floor (it had pulled, and crashed before
  acking) acks the head and is a holder again: the row is recreated.
- `GetHeadsRequest { group_ids }` → `HeadList [{ group_id, head_seq, evicted_through_seq,
  min_acked_seq? }]`, one `CheckMembershipRequest` per id, so a device sees at once whether it
  must start over anywhere.

`min_acked_seq` is `MIN(acked_seq)` over the group's `group_device` rows, null when there are
none. Nothing waits for a slow device beyond the window: the eviction that passes its ack takes
its row, the minimum moves on, and the device starts over. The window is therefore the longest a
device may stay away without starting over, and the longest any row stays here; a row every
holder has acked goes sooner. One number per device, the ack, says everything: a device with a
row holds the group from seq 1 through its ack, and there is nothing to hash. A hash of the set
could only say whether two devices agree, and finding what one lacks would then need a tree or
a filter on top; here the seq names every write, every holder has all of them, and the request
for what the window no longer has is always "everything from seq 1".

## History requests (member devices are the store)

`RequestHistoryRequest { group_id, from_seq, to_seq, object_id? }` → `HistoryRequest`. The
candidates are the group's other `group_device` rows: every one of them holds the whole group.
When there are none the answer is `unfillable`, nothing is stored, and the requester knows at
once that no device holds anything older than the window: it acks with what the window gave it,
and the group's history is what the window had. Otherwise insert a `pending` row and send
`HistoryRequested { request_id, group_id, requester_device, from_seq, to_seq, object_id,
devices }` to `device`, which pushes a `history` wake-up to the listed devices.

A holder that wakes lists the group's open requests (`ListHistoryRequestsRequest { group_id }`:
`pending`, plus `claimed` where the claim is older than 5 minutes) and claims one:
`ClaimHistoryRequest { request_id }` is a single conditional update (`status = pending`, or
`claimed` with a stale `claimed_at`) that records the device and the time; the first caller wins
and the rest are answered `taken`. A caller without a `group_device` row is refused: it is not a
holder. Only the claimer may then push, so one holder uploads, not every holder. A claimer that
dies is replaced after 5 minutes by whoever claims next, and the rows it already restored are
skipped on the re-push. Client policy: a holder claims a one-object request only if it still has
the file; the requester's own devices claim at once, everyone else waits a random few seconds
first.

The claimer pushes the rows it holds with seq in `[from_seq, to_seq]` back to the server,
exactly as they were in the log, tombstones included while it still has them:
`PushHistoryRequest { request_id, entries: [{ id, type, schema_version, key_id, value?, seq,
deleted }], complete }`, newest first, a few thousand entries per call, `complete = true` on the
last call. A row belongs to the range when its seq, the seq of the object's current state, is
in it. The handler, under the group's counter lock, re-inserts each entry **with its original
seq** and `uploaded_at = now`, skipping any id that already has a row, at any seq, live or
tombstone: the current row always wins, so a later edit, a delete, or a second holder answering
the same request cannot be overridden. No new seq is handed out and the head does not move.
After each call it sends `HistoryReady { request_id, requester_device, complete }` to `device`,
which wakes the requester; `complete` marks the row `fulfilled`, and a call from a device that
does not hold the claim, or for a request already fulfilled, is refused. Any role may answer:
serving history is reading, not writing. An answer marked complete is complete: every holder
holds the whole group, so what the claimer lacked no device has.

The requester's cursor is still where it was, below the restored rows, so its next pull returns
them in seq order with everything else. Every other device's cursor is already past those
numbers, so nobody else sees any traffic. Restored rows are ordinary rows: they sit at or below
the floor, which the ack rule of the eviction never touches, and age out 168 hours after their
re-insert.

A request for one object (`object_id` set) is answered the same way with one entry and
`complete`. If the object's files have expired from the bucket, the holder re-uploads them to
their derived keys before it reports `complete`, and the requester downloads them directly.

No retries, no timeouts on the server: a requester that hears nothing asks again, and the client
shows "waiting for a member to come online". A holder that is offline keeps its row, so a
request waits for it rather than closing. Nothing records who answered: the entries were
encrypted under the group key, so the answerer was a key holder, and nothing reads it later.

This is the only way anything older than the window reaches a device: a new member, a new device
and a device that started over all ask for everything from seq 1, and a device that finds a
photo's file expired from the bucket asks for that object.

When nobody holds it: a range request is answered `unfillable` on the spot, and the window is
the history. A one-object request for a file no online holder kept stays open until it expires,
and the client then treats the file as gone; asking again later is always safe because
re-inserts skip existing rows. Thumbnails never expire from the bucket, so an album keeps its
grid even when a full size is gone for good. A group loses an object only when every holder is
gone or wiped; export is the backup, and a member may keep a device that reduces nothing
(`00-scope.md`, keeper device). Old messages are tombstoned by the clients on the group's
retention setting (`06-object-types.md`), which is what keeps "everything from seq 1" a bounded
download.

## Sweeps (schedules, hourly, idempotent)

- `evict_objects`, two rules. **By ack**: per group with a non-null `min_acked_seq`, delete rows
  with `evicted_through_seq < seq ≤ min_acked_seq`, live and tombstones alike, and set
  `evicted_through_seq = min_acked_seq`: every holder has them and a device that is not a
  holder starts over, so the cache has no reader left for them. A row restored by a history
  request sits at a seq at or below the floor and is not touched by this rule. **By age**:
  delete rows with `uploaded_at < now - 168h`, batched 10k per statement; per group set
  `evicted_through_seq = GREATEST(evicted_through_seq, MAX(seq) evicted)`, then delete the
  group's `group_device` rows with `acked_seq` below the new floor: a device that had not acked
  an evicted row is evicted with it. By age, not by position, so a restored row gets its own
  168 hours.
- `expire_history_requests`: delete rows older than 168 hours.

Postgres therefore holds, per group, what some holder has not acked yet, for seven days at most,
plus what a history request restored, for seven days. No device can hold anything back: a
device that does not keep up is evicted with the rows it did not ack, and starts over.

## Messages

| Handles | Response | From |
|---|---|---|
| PushObjectsRequest | PushResult | api |
| PullChangesRequest | ChangeSet | api |
| AckSeqRequest | AckResult | api |
| GetHeadsRequest | HeadList | api |
| RequestHistoryRequest | HistoryRequest | api |
| ListHistoryRequestsRequest | HistoryRequestList | api |
| ClaimHistoryRequest | HistoryRequest | api |
| PushHistoryRequest | PushHistoryResult | api |
| DeviceLeftGroup | (void) | device: delete the `group_device` row |
| PurgeGroupObjectsRequest | PurgeResult | group |

`PurgeGroupObjectsRequest { group_id }` (from `group`'s purge pass): delete every `object`,
`group_head`, `group_device` and `history_request` row for the group, batched, and answer
`PurgeResult { remaining }`. No tombstones: memberships are already gone, so a client gets "not
a member" and purges locally. Idempotent.

| Sends | Response | To |
|---|---|---|
| CheckMembershipRequest | MembershipCheck | group |
| GroupHeadAdvanced | (void) | device |
| ObjectDeleted | (void) | storage |
| HistoryRequested | (void) | device |
| HistoryReady | (void) | device |

## API

`LedgerService`: PushObjects, PullChanges, AckSeq, GetHeads, RequestHistory,
ListHistoryRequests, ClaimHistory, PushHistory. The api crate maps the proto entry to the plain Rust struct one to one;
`value` passes through untouched.

## Tests

- Unit: CAS-on-seq acceptance matrix (no row, match, mismatch, the same three on a tombstone);
  the holder check (a `group_device` row present or absent; a batch of creates without a row is
  refused like any other); eviction floor arithmetic for both rules.
- Integration: two replicas pushing to one group concurrently produce a gap-free sequence; a pull
  during a concurrent push never returns a seq the next pull skips; a batch of a parent and its
  children takes consecutive seqs in one transaction; **the holder invariant**: after N devices
  interleave pushes, deletes, pulls, acks and both evictions at random, every device with a row
  has applied every row the server evicted, every row the ack rule removed was at or below every
  holder's ack, and no holder holds a live copy of an object whose tombstone is gone; the lost-
  update case (an edit evicted while a device was offline) ends in a refusal, never a replace;
  the age rule deletes the `group_device` rows whose ack it passed and the ack rule never does;
  an ack below the floor is refused and one at or above it from a device without a row
  recreates the row; a restored row is never removed by the ack rule and is evicted 168 hours
  after its re-insert; a request from a group with no other holder is answered `unfillable`
  without a row; a claim from a device without a row is refused; two holders claiming at once
  leave exactly one claimer; a claim older than 5 minutes is taken over and the second claimer's
  re-push skips what the first restored; a history re-insert keeps the original seqs, moves
  neither head nor floor, skips ids that have a row (live or tombstone), restores a tombstone as
  a tombstone, and is refused once the request is fulfilled or from a device without the claim;
  a joining device whose history request is overtaken by an eviction asks for the difference
  and ends with every row.
- Property test: random interleavings of push/pull/ack/evict across N simulated devices, with
  history re-inserted by simulated holders and a device that falls behind starting over,
  converge to the same row per object on every holder; every holder holds exactly the group's
  rows from seq 1 through its ack; and no deleted object is ever live on a holder after its
  tombstone has gone.
