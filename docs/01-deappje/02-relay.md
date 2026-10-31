# `relay`

Owns the relay: a window of ciphertext rows per group, tombstones included, that some device
has not yet acked, 672 hours at most; the group's counter; one acked cursor per device; and a
row per history request, open or complete, until the sweep (a window after it was made, or
after it completed). Plain executor with two hourly sweeps. Member devices are the
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
| deleted | bool | tombstone flag; the tombstone's own `seq` is its deletion mark; a tombstone is evicted like any row |
| uploaded_at | timestamp | when this row landed on the server, by push or by a history re-insert; evicted 672 hours later |

Indexes: unique `(group_id, seq)`; `(uploaded_at)` for eviction. The parent of an object lives inside its encrypted value, and
its type, schema version and key id in the value's plaintext header (`06-object-types.md`): the
server never sees the structure of a group's data, nor what kind of thing a row is, nor who
wrote it: the author is inside the ciphertext, and the writer of a push is only used to leave
its own device out of the wake-up.

### `group_head`

| column | type | notes |
|---|---|---|
| group_id | uuid | primary key |
| head_seq | int64 | last assigned sequence |
| evicted_through_seq | int64 | the eviction floor: every row written at `seq ≤` this has been evicted once, by age or because every device had acked it (a history re-insert may put a copy back); a device whose ack is at or above it is a holder, one whose cursor is below it starts over |
| restored_through_seq | int64 | the restore witness: every row at or below this seq that any holder has is in the log right now (0: none). Set by a complete restore from seq 1 to its `to_seq` (never lowered), cleared by an eviction that reaches a restored row. A whole-range request it covers is answered `complete` on the spot |
| restored_at | timestamp, nullable | when the witness was last set; eviction by ack leaves the group alone for 30 minutes after it (the grace) |

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
| claimed_by | uuid, nullable | the holder device answering it; named to that device only, never to another (a device id is the caller's word, so another member holding it could push as the claimer) |
| claimed_at | timestamp, nullable | a claim older than 5 minutes without `complete` counts as abandoned |
| through_seq | int64 | the last seq a holder pushed back so far: `from_seq - 1` before any, `to_seq` once complete; the requester acks no further |
| completed_at | timestamp, nullable | set by the claimer's last call; the row stays, marked, for the requester to read |
| created_at | timestamp | |

Index `(group_id)`. A row is a request, open or complete: `complete` marks it rather than deleting
it, so the requester can read where it stands without depending on a wake-up. The sweep drops an
open row 672 hours after it was made and a complete one 672 hours after it completed, so a request
completed late still leaves its requester the whole window to read `complete`.

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
everything older with `RequestHistoryRequest { from_seq: 1, to_seq: evicted_through_seq }`
(answered `complete` on the spot when the window still holds the range after another device's
restore; "History requests"); on every sync while it waits (`HistoryReady` hurries one), read
the request (`GetHistoryRequest`)
and page the restored rows from its ack up to its `through_seq`, acking each page, which is
honest, since the holder pushes oldest first and every row with a seq at or below `through_seq`
is then in; when the request is complete and the last page is acked it holds everything through
the floor and everything after it, and acks the head: it is a holder.
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
history is what the window had. Otherwise insert a row and send `HistoryRequested
{ request_id, group_id, requester_device, from_seq, to_seq, object_id, devices }` to `device`,
which pushes a `history` wake-up to the listed devices.

A holder that wakes lists the group's open requests (`ListHistoryRequestsRequest { group_id }`:
unclaimed, plus claimed more than 5 minutes ago) and claims one:
`ClaimHistoryRequest { request_id }` is a single conditional update (`claimed_by` null, or
`claimed_at` older than 5 minutes) that records the device and the time; the first caller wins
and the rest are answered `taken` (or `complete`, when the last push landed between the read and
the update). The claimer itself claiming again, as after a lost answer, is answered `claimed`. A
caller that is not a holder is refused. Only the claimer
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
After each call it records `through_seq` on the row (the last seq of the call; `to_seq` on the
complete one, whatever its last entry: the range may end in rows nobody holds) and, on the last
call, `completed_at`, and sends `HistoryReady { request_id, requester_device, through_seq,
complete }` to `device`, which wakes the requester. A call from a device that does not hold the
claim, or for a request that is complete or no longer exists, is refused. Any role may answer:
serving history is reading, not writing. An answer marked complete is complete: every holder
holds the whole group, so what the claimer lacked no device has.

The requester pages the restored rows from its ack, which is below them, and acks as it goes;
every other device's cursor is already past those numbers, so nobody else sees any traffic.

The requester does not depend on the wake-up. `GetHistoryRequestRequest { request_id }` →
`HistoryRequest` answers any member of the group with the row's `status` (`open`, `claimed`,
`taken`, `complete`) and `through_seq`, and a device catching up reads it on every sync, so it
pages and acks as far as `through_seq` and becomes a holder on `complete` whether the wake-up
came, came late or never came (a browser gets none). A complete row stays a window from its
completion; `not found` means the sweep dropped it, a window after the device asked, and the
requester **starts over** rather than asking again: the sweep of old rows in the same hour has
likely dropped its `group_device` row and raised the floor, so a fresh join acks 0 (the row that
holds the restored rows back from eviction) and asks up to the floor as it is now. Only a
Connect-coded `not_found` means swept: a bare HTTP 404 is a route the server lacks (a client newer
than the server mid-deploy) and is waited out like an outage. While it waits, the requester pulls
only what is new since its last pull, not the window again; complete, it pages the whole stretch
from its ack once.

Two devices joining close together cost one upload, or none. A complete restore from seq 1
leaves a **witness** on the group head (`restored_through_seq = to_seq`, `restored_at = now`):
every row at or below it that any holder has is in the log, until an eviction reaches a restored
row and clears it; eviction by ack leaves the group alone for 30 minutes after a restore (the
grace, "Sweeps"), so the rows a joiner's restore put back are still there for the next joiner. A
whole-range request whose `to_seq` the witness covers is answered `complete` on the spot, with a
nil id: nothing is stored, nobody is woken, and the requester pages what its pull already gave
it. A whole-range request made while an older open one reaches its `to_seq` is **covered**: it
is stored and the requester follows it as usual, but it is not announced and holders do not
list it, and the complete push of the covering request completes it in the same transaction
(`through_seq = to_seq`, a `HistoryReady` to its requester). A one-object request is never
covered and never covers: its holder may have a file to re-upload first.
Restored rows are ordinary rows: they go by the ack rule as soon as the requester, the last
device below them, has acked past them, and by age 672 hours after their re-insert if it never
does.

A request for one object (`object_id` set) is answered the same way with one entry and
`complete`. If the object's files have expired from the bucket, the holder re-uploads them to
their derived keys before it reports `complete`, and the requester downloads them directly.

No retries, no timeouts on the server: a requester whose request was swept starts over, and the
client shows "waiting for a member to come online" while one is open. A holder that is offline keeps its row, so a
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

- `evict_objects`, two rules. **By ack**: per group with a non-null `min_acked_seq`, unless its
  `restored_at` is within the last 30 minutes (the grace: the group is skipped whole, since
  evicting only its older rows would move the floor past rows the witness still vouches for),
  delete rows with `seq ≤ min_acked_seq`, live and tombstones alike, restored ones included, and
  set `evicted_through_seq = GREATEST(evicted_through_seq, min_acked_seq)`: every device has
  them, so the cache has no reader left for them; a pass with `min_acked_seq ≥ 1` clears the
  witness. **By age**: delete rows with `uploaded_at < now - 672h`, batched 10k per statement;
  per group set `evicted_through_seq = GREATEST(evicted_through_seq, MAX(seq) evicted)`, then
  delete the group's `group_device` rows with `acked_seq` below the new floor and `updated_at <
  now - 672h`: a device that had not acked an evicted row is evicted with it, unless it is
  catching up, which its acks show. By age, not by position, so a restored row the requester
  never acked still goes, and a pass whose lowest evicted seq is at or below the witness clears
  it.
- `expire_history_requests`: delete open rows made more than 672 hours ago and complete rows
  completed more than 672 hours ago (`COALESCE(completed_at, created_at)`).

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
| GetHistoryRequestRequest | HistoryRequest | api |
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

`RelayService`: PushObjects, PullChanges, AckSeq, GetHeads, RequestHistory, GetHistoryRequest,
ListHistoryRequests, ClaimHistory, PushHistory. The api crate maps the proto entry to the plain Rust struct one to one;
`value` passes through untouched.

## Implementation record

What the code settled beyond the text above (`nanoservices/relay`, 2026-10-30):

- **One row per object.** `object.id` is the primary key; an edit updates the row with the new
  seq and `uploaded_at`, so a row's seq is its state and an evicted object edited by a holder
  comes back as a new row. An id that belongs to another group refuses the batch.
- **The device id is the caller's word within their membership**, as the plan says; `relay` has
  no route to verify that a device belongs to the caller, and a device id is a client-minted
  random UUID. A first ack above 0 without a row is refused (`a device's first ack in a group is
  0`), so a row is only ever created at 0.
- **Pull** pages 500 by default and 2000 at most. `GetHeads` leaves out the groups the caller is
  not in rather than refusing, so a device learns of a removal while offline.
- **History**: the requester cannot claim its own request; a claim by a device below the floor
  is refused; a history push is validated whole before anything is locked (oldest first,
  strictly ascending, inside the range, one object when the request names one, never above the
  head); `through_seq` is the last entry's seq (`to_seq` on complete). `unfillable`
  answers with a nil id. Any role may answer history, as designed.
- **A request outlives its completion** (2026-10-31): `complete` marks the row (`completed_at`,
  `through_seq = to_seq`) instead of deleting it, and `GetHistoryRequest` reads it, so the
  requester follows its request through the relay and the `history_ready` wake-up is a hurry-up
  like every other push. Before, the wake-up was the only carrier of `through_seq` and
  `complete`, and a dropped one left the requester catching up for good.
- **The review of that change** (2026-10-31): the sweep keeps a complete row a window from its
  completion; a swept request makes the requester start over; a bare 404 is `Unavailable` in the
  core, not `NotFound`; a claim that loses to a completion answers `complete`, and the claimer
  claiming again answers `claimed` in the model as in the handler; `claimed_by` is named to the
  claimer only, in every answer; a device waiting on a holder skips the part of the window it
  already pulled.
- **The restore witness and the grace** (2026-10-31): two joins minutes apart used to cost two
  uploads, the second one skipped row by row by the server, because the floor never comes down
  and the second joiner could not know the rows were back. Now a complete restore from seq 1
  records how far the log is whole again, eviction by ack waits 30 minutes, a covered request is
  answered `complete` at once, and a request made while another is in progress completes with
  it. The floor's meaning is unchanged; a joiner answered `complete` acks only rows it pulled.
- **Eviction by age** raises a group's floor to the highest seq taken in the batch and drops the
  devices below it that have not acked within the window, under the counter lock, batched
  ten thousand rows per statement; a purge deletes ten thousand rows per call and the counter,
  devices and requests with the last batch.
- **Messages.** `PushResult` is an enum (`Accepted { accepted, conflicts, head_seq }` or
  `BehindFloor { evicted_through_seq }`); `HistoryRequest` carries a `status` (`open`,
  `claimed`, `taken`, `unfillable`, `complete`) and `through_seq`; `ChangeSet`, `AckResult` and `Head` carry `min_acked_seq`
  as an option, on the wire a value plus `has_min_acked_seq`. The API maps one to one; `value`
  passes through untouched and an empty value on the wire is none.

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
  gone or from a device without the claim.
- Property test: random interleavings of push/pull/ack/evict across N simulated devices, with
  history re-inserted by simulated holders and a device that falls behind starting over,
  converge to the same row per object on every holder; every device holds exactly the group's
  rows from seq 1 through its ack; and no deleted object is ever live on a holder after its
  tombstone has gone.
