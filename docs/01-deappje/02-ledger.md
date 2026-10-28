# `ledger`

Owns the relay: a window of ciphertext rows per group, tombstones included, that some device
has not yet acked, 672 hours at most; the group's counter; one acked cursor per device; and a
row per pending history request. Plain executor with two hourly sweeps. Member devices are the
store, each of them holding the whole group; this nanoservice forgets everything after 672
hours. Directive §2 per table, §7 (counter row under
`FOR UPDATE`; a boolean or a number is all that crosses) for its messages.

## Tables

### `object`

| column | type | notes |
|---|---|---|
| id | uuid | primary key, client-minted |
| group_id | uuid | |
| value | bytes, nullable | ≤ 1 MiB; null on a tombstone. A plaintext header (`type`, `schema_version`, `key_id`) followed by the ciphertext; the server never parses it. A blob reference (slot, hash, size, content key) is inside the ciphertext, and so is a photo's thumbnail |
| seq | int64 | per group, strictly increasing; an edit takes a new seq, so a row's seq identifies its state |
| created_by_device | uuid | |
| deleted | bool | tombstone flag; the tombstone's own `seq` is its deletion mark; a tombstone is evicted like any row |
| uploaded_at | timestamp | when this row landed on the server, by push or by a history re-insert; evicted 672 hours later |

Indexes: unique `(group_id, seq)`; `(uploaded_at)` for eviction. The parent of an object lives inside its encrypted value, and
its type, schema version and key id in the value's plaintext header (`06-object-types.md`): the
server never sees the structure of a group's data, nor what kind of thing a row is.

### `group_head`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key |
| head_seq | int64 | last assigned sequence |
| evicted_through_seq | int64 | the eviction floor: every row written at `seq ≤` this has been evicted once, by age or because every device had acked it (a history re-insert may put a copy back); a device whose ack is at or above it is a holder, one whose cursor is below it starts over |

Created with zeros on the first push to a group.

### `group_device`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key (with device_id) |
| device_id | uuid | |
| acked_seq | int64 | the device holds every row of the group with `seq ≤` this, from seq 1 up; 0 for a device that has just joined or started over |
| updated_at | timestamp | the last ack |

Index `(device_id)`. A device's first act in a group is `AckSeqRequest { seq: 0 }`, which creates
the row; a device that starts over sends the same. Deleted on `DeviceLeftGroup` from `device`,
and by the age rule of `evict_objects` when the device's ack is below the floor and it has not
acked for 672 hours: a device that has not acked a row that gets evicted is evicted with it. A
row with `acked_seq ≥ evicted_through_seq` is a **holder**: it has the whole group. A row below
the floor is a device catching up, and its ack pins `min_acked_seq`, the minimum over the
group's rows, so nothing it has not acked can be evicted by ack from under it.

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

Index `(group_id, status)`. Rows older than 672 hours are dropped by the sweep.

## Push

`PushObjectsRequest { group_id, objects: [{ id, value?, base_seq, deleted }] }`. `base_seq` is the seq of the row the device edited
from, 0 for a new object.

Before the transaction: `CheckMembershipRequest`, and the role must be `admin` or `member` (a
`viewer` is refused). A `value` over 1 MiB refuses the whole batch.

One transaction:

1. `SELECT ... FROM group_head WHERE group_id = $1 FOR UPDATE` (insert if absent), and the
   device's `group_device` row.
2. **Not a holder?** If there is no row, or its `acked_seq < evicted_through_seq`, refuse the
   whole batch with `behind_floor { evicted_through_seq }`, whatever it holds: the device is
   still catching up, or an eviction passed its ack; either way it has not seen everything the
   server has evicted, so its word is worth nothing, and `base_seq = 0` is its word too. The
   server cannot tell a create from an edit of a row it no longer has, so a batch of "creates"
   from such a device could carry a hard-deleted object straight back in. A new member therefore
   gets the history and acks past the floor before its first post.
3. Per object, in request order, read the current row:
   - **No row** (new, or evicted) ⇒ accept.
   - **`base_seq == seq`** ⇒ accept: write the object at `seq = head + 1`, head advances. If
     `deleted`: null `value`, `deleted = true`.
   - **Mismatch** ⇒ conflict `{ id, current }` with the stored row as it is, tombstone or not.
4. Commit.

That is the whole rule. A tombstone is not special on the server: a holder that has not seen the
delete holds an older seq and conflicts while the tombstone is here, and once the tombstone has
gone every device has acked it, so no holder can be in that state; a device that has seen it
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
until every device has acked it, 672 hours at most, and a device keeps the ones it pulled until
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
  more }`. A device that pulls with `since_seq < evicted_through_seq` and an ack at or above
  the floor has gone backwards (a restored backup) or was evicted while away: rows in
  `(since_seq, evicted_through_seq]` are gone from the server, tombstones among them, so it
  cannot know which of the objects it holds were deleted meanwhile. It starts over: it throws
  the group away (objects and tombstones; files stay as a content-addressed cache, since a file
  is reachable only through an object's reference and brings nothing back by itself; unpushed
  writes stay in the outbox) and takes the path of a new device, below.
- `AckSeqRequest { group_id, seq }` → `AckResult { evicted_through_seq, min_acked_seq }`. `seq`
  is the cursor through which the device holds every row of the group from seq 1. Acks are
  monotone: a value below the stored one is refused, except 0, which a device sends as its first
  act in a group and whenever it starts over, and which creates or resets its row. `seq >
  head_seq` is refused. A device acks after applying a `ChangeSet` with `more = false`, with
  that response's `head_seq`, and a device catching up acks each page of history as it applies
  it (below). Upserts `group_device` on the device's word.
- `GetHeadsRequest { group_ids }` → `HeadList [{ group_id, head_seq, evicted_through_seq,
  min_acked_seq? }]`, one `CheckMembershipRequest` per id, so a device sees at once where it
  stands in every group.

**The path of a new device**, taken on joining and on starting over: ack 0, which gives it a row
and pins the group's minimum ack at 0; pull from 0 and apply the window, to have something to
show, and follow the head from there like any device, without acking; ask a holder for
everything older with `RequestHistoryRequest { from_seq: 1, to_seq: evicted_through_seq }`; as
`HistoryReady` comes, page the restored rows from its ack up to the floor it asked for, acking
each page, which is honest, since the holder pushes oldest first and every row with a seq at or
below the page's last is then in; when the request is complete and the last page is acked it
holds everything through the floor and everything after it, and acks the head: it is a holder.
Then it replays its outbox against the store it just fetched: a create goes as it is; an edit
or delete goes only if its object is still there, rebased onto the current row with that row's
seq as its base, and is dropped when the object is gone, since a holder pushing a stale base
into an empty slot would be the silent overwrite the holder check exists to prevent. The seq
check catches whatever raced it. Objects come whole, from seq 1, thumbnails inside them, so
every grid is complete as soon as the objects are; full-size files come lazily, when viewed.
Nothing can be evicted by ack from under a
device on this path, because its own ack is the minimum, and the rows it has acked, restored
ones included, go as soon as everyone else is past them too. A device drops its local
tombstones with `seq ≤ min_acked_seq`: every device has applied them, and a device that starts
over drops everything, so no device can ever hand it a live copy of those again.

`min_acked_seq` is `MIN(acked_seq)` over the group's `group_device` rows, null when there are
none. Nothing waits for a slow device beyond the window: a device that neither reaches the
floor nor acks for 672 hours loses its row, the minimum moves on, and the device starts over
when it is back. The window is therefore the longest a device may stay away without starting
over, and the longest any row stays here; a row every device has acked goes sooner. One number
per device, the ack, says everything: a device holds the group from seq 1 through its ack, it
is a holder once that reaches the floor, and there is nothing to hash. A hash of the set could
only say whether two devices agree, and finding what one lacks would then need a tree or a
filter on top; here the seq names every write, every holder has all of them, and the request
for what the window no longer has is always "everything from seq 1".

## History requests (member devices are the store)

`RequestHistoryRequest { group_id, from_seq, to_seq, object_id? }` → `HistoryRequest`. The
candidates are the group's holders: the other `group_device` rows with `acked_seq ≥
evicted_through_seq`, every one of which holds the whole group. When there are none the answer
is `unfillable`, nothing is stored, and the requester knows at once that no device holds
anything older than the window: it acks the head with what the window gave it, and the group's
history is what the window had. Otherwise insert a `pending` row and send `HistoryRequested
{ request_id, group_id, requester_device, from_seq, to_seq, object_id, devices }` to `device`,
which pushes a `history` wake-up to the listed devices.

A holder that wakes lists the group's open requests (`ListHistoryRequestsRequest { group_id }`:
`pending`, plus `claimed` where the claim is older than 5 minutes) and claims one:
`ClaimHistoryRequest { request_id }` is a single conditional update (`status = pending`, or
`claimed` with a stale `claimed_at`) that records the device and the time; the first caller wins
and the rest are answered `taken`. A caller that is not a holder is refused. Only the claimer
may then push, so one holder uploads, not every holder. A claimer that dies is replaced after 5
minutes by whoever claims next, and the rows it already restored are skipped on the re-push.
Client policy: a holder claims a one-object request only if it still has the file; the
requester's own devices claim at once, everyone else waits a random few seconds first.

The claimer pushes the rows it holds with seq in `[from_seq, to_seq]` back to the server,
exactly as they were in the log, tombstones included while it still has them:
`PushHistoryRequest { request_id, entries: [{ id, value?, seq, deleted }], complete }`, **oldest first**, a few thousand entries per call, `complete = true` on
the last call. A row belongs to the range when its seq, the seq of the object's current state,
is in it. The handler, under the group's counter lock, re-inserts each entry **with its original
seq** and `uploaded_at = now`, skipping any id that already has a row, at any seq, live or
tombstone: the current row always wins, so a later edit, a delete, or a second holder answering
the same request cannot be overridden. No new seq is handed out and the head does not move.
After each call it sends `HistoryReady { request_id, requester_device, through_seq, complete }`
to `device`, which wakes the requester; `through_seq` is the last seq of the call, so the
requester knows how far it may page and ack; `complete` marks the row `fulfilled`, and a call
from a device that does not hold the claim, or for a request already fulfilled, is refused. Any
role may answer: serving history is reading, not writing. An answer marked complete is
complete: every holder holds the whole group, so what the claimer lacked no device has.

The requester pages the restored rows from its ack, which is below them, and acks as it goes;
every other device's cursor is already past those numbers, so nobody else sees any traffic.
Restored rows are ordinary rows: they go by the ack rule as soon as the requester, the last
device below them, has acked past them, and by age 672 hours after their re-insert if it never
does.

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
re-inserts skip existing rows. A thumbnail lives inside its object's value, so an album keeps
its grid wherever the object is, even when a full size is gone for good. A group loses an object only when every holder is
gone or wiped; export is the backup, and a member may keep a device that reduces nothing
(`00-scope.md`, keeper device). Old messages are tombstoned by the clients on the group's
retention setting (`06-object-types.md`), which is what keeps "everything from seq 1" a bounded
download.

## Sweeps (schedules, hourly, idempotent)

- `evict_objects`, two rules. **By ack**: per group with a non-null `min_acked_seq`, delete rows
  with `seq ≤ min_acked_seq`, live and tombstones alike, restored ones included, and set
  `evicted_through_seq = GREATEST(evicted_through_seq, min_acked_seq)`: every device has them,
  so the cache has no reader left for them. **By age**: delete rows with
  `uploaded_at < now - 672h`, batched 10k per statement; per group set `evicted_through_seq =
  GREATEST(evicted_through_seq, MAX(seq) evicted)`, then delete the group's `group_device` rows
  with `acked_seq` below the new floor and `updated_at < now - 672h`: a device that had not
  acked an evicted row is evicted with it, unless it is catching up, which its acks show. By
  age, not by position, so a restored row the requester never acked still goes.
- `expire_history_requests`: delete rows older than 672 hours.

Postgres therefore holds, per group, what some device has not acked yet, for 28 days at most.
No device can hold anything back: a device that does not keep up is evicted with the rows it did
not ack, and starts over.

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
  the holder check (no `group_device` row, a row below the floor, a row at it; a batch of
  creates from a non-holder is refused like any other); the ack rules (monotone, 0 resets, above
  the head refused); eviction floor arithmetic for both rules.
- Integration: two replicas pushing to one group concurrently produce a gap-free sequence; a pull
  during a concurrent push never returns a seq the next pull skips; a batch of a parent and its
  children takes consecutive seqs in one transaction; **the holder invariant**: after N devices
  interleave pushes, deletes, pulls, acks and both evictions at random, every holder has applied
  every row the server evicted, every row the ack rule removed was at or below every device's
  ack, and no holder holds a live copy of an object whose tombstone is gone; the lost-update
  case (an edit evicted while a device was offline) ends in a refusal, never a replace; the age
  rule deletes the rows of devices below the floor that have not acked for 672 hours and leaves
  a device that is catching up; a backwards ack is refused and 0 resets the row; a joining
  device's ack 0 pins the minimum, so nothing it has not acked is evicted by ack while it
  catches up, and the restored rows go as it acks past them; a request from a group with no
  other holder is answered `unfillable` without a row; a claim from a non-holder is refused; two
  holders claiming at once leave exactly one claimer; a claim older than 5 minutes is taken over
  and the second claimer's re-push skips what the first restored; a history re-insert keeps the
  original seqs, moves neither head nor floor, skips ids that have a row (live or tombstone),
  restores a tombstone as a tombstone, arrives oldest first, and is refused once the request is
  fulfilled or from a device without the claim.
- Property test: random interleavings of push/pull/ack/evict across N simulated devices, with
  history re-inserted by simulated holders and a device that falls behind starting over,
  converge to the same row per object on every holder; every device holds exactly the group's
  rows from seq 1 through its ack; and no deleted object is ever live on a holder after its
  tombstone has gone.
