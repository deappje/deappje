# `ledger`

Owns the relay: a 168-hour window of ciphertext rows per group, tombstones included, the group's
counter, per device one acked cursor and one coverage floor, and a row per pending history
request. Plain executor with two hourly sweeps. Member devices are the store; this nanoservice
forgets everything after 168 hours. Directive §2 per table, §7 (counter row under
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
| evicted_through_seq | int64 | the eviction floor: every row written at `seq ≤` this has been evicted once, tombstones included (a history re-insert may put a copy back); a device whose ack is below it is behind the floor |

Created with zeros on the first push to a group.

### `group_device`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key (with device_id) |
| device_id | uuid | |
| acked_seq | int64 | the cursor this device has pulled through and applied; below `evicted_through_seq` the device is behind the floor |
| covered_from_seq | int64 | the device's coverage floor: it holds every row of the group with `seq` in `[covered_from_seq, acked_seq]` |
| updated_at | timestamp | |

Index `(device_id)`. Upserted by `AckSeqRequest`, on the device's word; deleted on `DeviceLeftGroup`
from `device`, so a row exists only for a device that still holds a key. Nothing waits for a
device: one that falls behind the floor is refused edits until it has reconciled, and a history
request goes to the rows whose floor reaches it.

### `history_request`

| column | type | notes |
|---|---|---|
| id | uuid | primary key |
| group_id | uuid | |
| requester_device | uuid | |
| from_seq | int64 | |
| to_seq | int64 | lowered when a holder's own floor ended its answer and the remainder is reopened |
| object_id | uuid, nullable | a request for one object instead of a range |
| status | string | `pending`, `claimed`, `fulfilled` |
| covered_from_seq | int64, nullable | how far down the answers have got: every row any claimer held with `seq` in `[covered_from_seq, to_seq]` has been re-inserted; null before the first answer |
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
   device's `group_device` row (no row counts as acked 0).
2. **Behind the floor?** If the device's `acked_seq < evicted_through_seq`, refuse the whole
   batch with `behind_floor { evicted_through_seq }`, whatever it holds: the device has not seen
   everything the server has evicted, so its word is worth nothing, and `base_seq = 0` is its
   word too. The server cannot tell a create from an edit of a row it no longer has, so a batch
   of "creates" from behind the floor could carry a hard-deleted object straight back in. A new
   member therefore pulls and acks before its first post; one round trip.
3. Per object, in request order, read the current row:
   - **No row** (new, or evicted) ⇒ accept.
   - **`base_seq == seq`** ⇒ accept: write the object at `seq = head + 1`, head advances. If
     `deleted`: null `value`, `deleted = true`.
   - **Mismatch** ⇒ conflict `{ id, current }` with the stored row as it is, tombstone or not.
4. Commit.

That is the whole rule. A tombstone is not special on the server: a device that has not seen the
delete holds an older seq and conflicts while the tombstone is in the window, and is behind the
floor once it is not; a device that has seen it and pushes a live object with the tombstone's seq
as its base is un-deleting, which is the client's decision. A delete whose base is stale conflicts
like any edit and is pushed again with the current seq.

After commit: `GroupHeadAdvanced { group_id, head_seq, writer_device }` once; `ObjectDeleted
{ group_id, object_id }` per tombstone, so `storage` can delete the object's files; the server
does not know which objects have any.

Response: `PushResult { accepted: [{ id, seq }], conflicts: [{ id, current }], head_seq }`, or
`behind_floor`. The objects themselves come back on the next pull with everyone else's; the
device skips entries it already holds at that seq.

A tombstone's deletion mark is its seq, and a tombstone is a row like any other: it lives 48
hours on the server, and a device keeps the ones it pulled until the floor passes them. There is
no pull-to-head check before a push: the one cursor rule is the floor check above, and it is what
makes the no-row branch safe. A device at or above the floor has seen every row the server
evicted, so an evicted row it edits was last written at a seq it holds, and any later write would
still be a row here and conflict. From behind the floor the same push is a silent overwrite: a
device that went offline holding an object at seq 300 while another device edited it at seq 2000,
and that row was evicted, would drop its stale state into the empty slot at a new seq, and every
other device would apply it as the later write, with no conflict anywhere. Hence the refusal.
There is no cascade either: the deleting client puts the children it holds in the same batch (one
push is one transaction, so they take consecutive seqs together), and a child it did not hold is
tombstoned by the first device that holds it and sees the parent's tombstone. The server could
not do this completely anyway, since children older than 168 hours are already evicted from it.

Why a locked counter row and not a sequence: a sequence is non-transactional, so a lower value can
commit after a higher one and a client that pulled in between misses it forever (Directive §7).

## Pull and ack

- `PullChangesRequest { group_id, since_seq, limit }` → `ChangeSet { entries (seq ascending,
  rows still present, tombstones included), head_seq, evicted_through_seq, more }`. If
  `since_seq < evicted_through_seq` the device is behind the floor: rows in
  `(since_seq, evicted_through_seq]` are gone from the server, tombstones among them, so the
  device cannot know which of the objects it holds were deleted meanwhile. It reconciles by
  throwing the group away and starting over as a new device: objects, tombstones and every
  unpushed write are dropped (files stay as a content-addressed cache, since a file is reachable
  only through an object's reference and brings nothing back by itself), the device pulls from
  0, acks the head with floor `evicted_through_seq + 1`, and refills lazily: the last stretch of the
  group by range, an object when a reference to it is opened, a full-size file when it is
  viewed. Until it has acked again its edits are refused. The device also drops its local
  tombstones with `seq ≤ evicted_through_seq`: every device at or above the floor has applied
  them, and one below it wipes.
- `AckSeqRequest { group_id, seq, covered_from_seq }` → `AckResult { evicted_through_seq }`.
  Sent by a device after it has applied a `ChangeSet` with `more = false`, with that response's
  `head_seq` and the device's coverage floor. Upserts `group_device` on the device's word;
  `seq > head_seq` is refused. An ack below the stored one (a restored backup) is stored as it
  is: the device is where it says it is, and the rows after it are still here or it is behind
  the floor; either case is handled by the rules above.
- `GetHeadsRequest { group_ids }` → `HeadList [{ group_id, head_seq, evicted_through_seq }]`,
  one `CheckMembershipRequest` per id, so a device sees at once whether it is behind anywhere.

No minimum over acks exists and nothing waits for a slow device: it falls behind the floor and
reconciles. The window is therefore also the longest a device may stay away without starting
over.

### Coverage

A device's knowledge of a group is one interval, `[covered_from_seq, acked_seq]`: every row the
group has with a seq in it, tombstones included until the floor passes them. It is one interval
because seqs are dense and server-assigned, and the device only ever extends it backwards: a
range request ends at `covered_from_seq - 1`. After a first pull the floor is
`evicted_through_seq + 1` (1 when nothing has been evicted); a fulfilled range request lowers it
to the request's `covered_from_seq`; a device that prunes old history, a later client setting,
raises it. One number per device therefore says what it can serve and what it lacks, and there
is nothing to hash: a hash of the set could only say whether two devices agree, and finding what
one lacks would then need a tree or a filter on top; the seq already names every write, so the
floor is the whole reconciliation. A one-object request fills a reference without moving the
floor. Absence has a meaning only inside the interval: a device whose floor is 1 holds everything
the group has, so an id it lacks is lost or was never pushed; with a higher floor an id it lacks
may live below it and becomes a one-object request.

## History requests (member devices are the store)

`RequestHistoryRequest { group_id, from_seq, to_seq, object_id? }` → `HistoryRequest`. For a
range the candidates are the group's other `group_device` rows with `covered_from_seq ≤ to_seq`:
when there are none the answer is `unfillable { holders_cover_from }` (the lowest floor among the
group's other devices, null when there are none), nothing is stored, and the requester knows at
once that no device has anything in that range. Otherwise insert a `pending` row and send
`HistoryRequested { request_id, group_id, requester_device, from_seq, to_seq, object_id,
devices }` to `device`, which pushes a `history` wake-up to the listed devices. For one object
the server does not know the seq, so every other device of the group is listed.

A holder that wakes lists the group's requests (`ListHistoryRequestsRequest { group_id }`:
`pending`, `claimed` where the claim is older than 5 minutes, and the caller's own in any state,
each with its `covered_from_seq`) and claims one: `ClaimHistoryRequest { request_id }` is a
single conditional update (`status = pending`, or `claimed` with a stale `claimed_at`) that
records the device and the time; the first caller wins and the rest are answered `taken`. A
caller that is behind the floor, or whose floor is above a range request's `to_seq`, is refused:
the first must reconcile and the second holds nothing in the range. Only the claimer may then
push, so one holder uploads, not every holder. A claimer that dies is replaced after 5 minutes by
whoever claims next, who continues downward from the row's `covered_from_seq`; the rows already
restored are skipped on the re-push. Client policy: a holder claims a one-object request only if
it holds the object; the requester's own devices claim at once, everyone else waits a random few
seconds first.

The claimer pushes the rows it holds in the range back to the server, exactly as they were in the
log, tombstones included while it still has them: `PushHistoryRequest { request_id, entries: [{
id, type, schema_version, key_id, value?, seq, deleted }], covered_from_seq, complete }`, newest
first, a few thousand entries per call. A row belongs to the range when its seq, the seq of the
object's current state, is in it. `covered_from_seq` is how far down the claimer has got: every
row it holds with seq in `[covered_from_seq, to_seq]` has been pushed; it falls call by call and
ends at the claimer's own floor or at `from_seq`, whichever is higher. `complete = true` on the
claimer's last call. The handler, under the group's counter lock, re-inserts each entry **with
its original seq** and `uploaded_at = now`, skipping any id that already has a row, at any seq,
live or tombstone: the current row always wins, so a later edit, a delete, or a second holder
answering the same request cannot be overridden. No new seq is handed out and the head does not
move. It records `covered_from_seq` on the row and sends `HistoryReady { request_id,
requester_device, covered_from_seq, status }` to `device`, which wakes the requester. On
`complete`: `covered_from_seq ≤ from_seq` closes the range and the row is `fulfilled`. Otherwise
the claimer's own floor ended the answer, and the remainder `[from_seq, covered_from_seq - 1]` is
the request now: the row goes back to `pending` with the claim cleared and `to_seq` lowered, and
`HistoryRequested` goes out again to the devices whose floor reaches the new `to_seq`, which
excludes everyone who has answered; when there is no such device the row is `fulfilled` as it
stands, since every row any device has in the range has been re-inserted, and its
`covered_from_seq` says where the group's history ends. A call from a device that does not hold
the claim, or for a request already fulfilled, is refused. Any role may answer: serving history
is reading, not writing.

The requester pulls as usual: its cursor is below the restored rows, so `PullChangesRequest`
returns them in seq order with everything else. Every other device's cursor is already past
those numbers, so nobody else sees any traffic. Restored rows are ordinary rows: they live 48
hours from `uploaded_at` and are evicted again, as the user specified. The requester's floor
becomes the lower of its own and the request's `covered_from_seq` as it pulls, so a request that
expires half-answered keeps what it got, and the next one starts where it stopped.

A request for one object (`object_id` set) is answered the same way with one entry and
`complete`, and moves no floor. If the object's files have expired from the bucket, the holder
re-uploads them to their derived keys before it reports `complete`, and the requester downloads
them directly.

No retries, no timeouts on the server: a requester that hears nothing asks again, and the client
shows "waiting for a member to come online". A holder that is offline keeps its row, so a
request its floor reaches waits for it rather than closing. Nothing records who answered: the
entries were encrypted under the group key, so the answerer was a key holder, and nothing reads
it later.

This is the only way anything older than 168 hours reaches a device: a new member, a new device,
a device that started over after falling behind the floor, a device scrolling past what it
holds, or a device that finds a photo's file expired from the bucket. The lazy refill is the
same mechanism: the last stretch of the group by range, then older ranges as the user scrolls,
one-object requests for references the user opens.

When nobody holds it: a range request is answered `unfillable` on the spot, or fulfilled with a
`covered_from_seq` above `from_seq`; the requester's floor records where the group's history
ends, and it does not ask below it again until a holder with a lower floor appears. A one-object
request for an object no online holder has stays open until it expires, and the client then
treats the object as gone; asking again later is always safe because re-inserts skip existing
rows. Thumbnails never expire from the bucket, so an album keeps its grid even when a full size
is gone for good. A group loses an object only when every member device that ever synced it is
gone, wiped, or started over while it was the sole holder; export is the backup, and a member
may keep a device that reduces nothing (`00-scope.md`, keeper device).

## Sweeps (schedules, hourly, idempotent)

- `evict_objects`: delete rows with `uploaded_at < now - 168h`, tombstones included, batched 10k
  per statement; per group set `evicted_through_seq = GREATEST(evicted_through_seq, MAX(seq)
  evicted)`. By age, not by position, so a row restored by a history request at an old seq gets
  its own 168 hours.
- `expire_history_requests`: delete rows older than 168 hours.

Postgres therefore holds, per group, seven days of activity, whatever the group's age. No device
can hold anything back: a device that does not keep up falls behind the floor and starts over.

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
  the floor check (ack at, above and below the floor; a batch of creates from below it is
  refused like any other);
  eviction floor arithmetic; the candidate set of a range request from a set of floors.
- Integration: two replicas pushing to one group concurrently produce a gap-free sequence; a pull
  during a concurrent push never returns a seq the next pull skips; a batch of a parent and its
  children takes consecutive seqs in one transaction; eviction removes live rows and tombstones
  alike and sets the floor; **the floor invariant**: after N devices interleave pushes, deletes,
  pulls, acks and evictions at random, every device whose ack is at or above the floor has
  applied every row the server evicted, and a stale device's edit of a deleted object never lands
  silently: it conflicts while the tombstone is in the window and is refused as `behind_floor`
  after; the lost-update case (an edit evicted while a device was offline) ends in a refusal,
  never a replace; a backwards ack is stored; a request for `[1, 2000]` answered by a holder
  with floor 1500 reopens `[1, 1499]` for a holder with floor 1 only, and is fulfilled at 1500
  when there is none; a request nobody covers is answered `unfillable` without a row; a claim
  from a holder above the range, or behind the floor, is refused; a history re-insert keeps the
  original seqs, moves neither head nor floor, skips ids that have a row (live or tombstone),
  restores a tombstone as a tombstone, and is refused once the request is fulfilled or from a
  device without the claim; two holders claiming at once leave exactly one claimer; a claim
  older than 5 minutes is taken over and the second claimer continues from the row's
  `covered_from_seq`; a restored row is evicted 168 hours after its re-insert.
- Property test: random interleavings of push/pull/ack/evict across N simulated devices, with
  history re-inserted by simulated holders and a device that falls behind starting over and
  refilling lazily, converge to the same row per object on every device that holds it; every
  device's interval from floor to cursor holds exactly the group's rows in that range; and no
  deleted object is ever live on any device at or above the floor after its tombstone was
  evicted.
