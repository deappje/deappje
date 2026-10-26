# Object types (client-defined; the server stores a number)

Every object is one protobuf message of its type (`01-architecture.md`, "Values and
conflicts"). Common fields on every type: `author` (user id), `authored_at`, `edited_at`.
Ordering of lists is a `pos` fractional index on the item. Blob references are `{ blob_id, key,
sha256, size }` with the blob's content key inside the message. Numbers are spaced by tens per
feature area; a breaking change takes the next free number in its block.

| # | Type | Parent | Keys | Blobs | Notes |
|---|---|---|---|---|---|
| 1 | `group_profile` | none | name, description, avatar | thumbnail | singleton: id = UUIDv5(group_id, "profile") |
| 2 | `member_profile` | none | display_name, avatar, birthday (month, day, year?) | thumbnail | one per member: id = UUIDv5(group_id, user_id); the calendar derives birthdays from it |
| 3 | `invite_secret` | none | invite_id, secret | | lets any holder verify a key request that came through the invite (`02-device.md`) |
| 10 | `post` | none | text | | timeline entry; photos are children |
| 11 | `photo` | post, album or message | width, height, taken_at, caption, full, thumbnail | full, thumbnail | the one photo type everywhere |
| 12 | `album` | none | title, cover_photo_id | | |
| 13 | `comment` | any | text | | |
| 14 | `reaction` | any | emoji | | one per user per target: id = UUIDv5(parent_id, user_id); changing it is an edit |
| 20 | `todo_list` | none | title | | |
| 21 | `todo_item` | todo_list | text, done, assignee, due, pos | | |
| 30 | `note` | none | title | | the body is the combination of its `note_update` children |
| 31 | `note_update` | note | update (bytes: one `yrs` update), seq_hint | | append-only, never edited; one per editing session per device; the device applies all children in any order into one `yrs` document; a tombstoned note cascades to them |
| 40 | `expense` | none | amount (minor units), currency, paid_by, participants (repeated), split (`equal` / `weights` / `exact`), kind (`expense` / `ticket`), description, date | | shares and balances are derived; a ticket is an equal split, one share per participant |
| 41 | `settlement` | none | from, to, amount, currency, date | | records a payment made outside the app; money never moves here |
| 42 | `ticket_share` | expense | holder (member id, or a name for a plus-one), status (`held` / `offered` / `used`) | attachment | the ticket file for one participant; a transfer is a holder change plus the matching participant change on the expense; the client pins the attachment of every share whose holder is the local user (downloaded at once, never evicted) so the gate needs no network |
| 50 | `event` | none | title, starts, ends, all_day, location, geo, yearly | | a birthday for someone not in the app is a yearly all-day event |
| 60 | `poll` | none | question, kind (`choice` / `dates`), options (repeated { id, text or slot, pos }), multi, closes_at | | the datepicker is kind `dates` |
| 61 | `vote` | poll | answers (repeated { option_id, answer }) | | one per user: id = UUIDv5(poll_id, user_id) |
| 70 | `message` | none, or a message (thread) | text, reply_to | | photos as children; an edit is a new seq, deletion a tombstone |
| 80 | `venue` | none | name, category (`bar` / `pub` / `club` / `restaurant` / `other`), address, geo, url, notes | | recommendations; comments and reactions are children |

Derived on the client, never stored: balances, poll tallies, the calendar (events plus member
birthdays), unread counts, sort orders, and the timeline. Deleting a parent is the client's
cascade (`01-architecture.md`): deleting a post pushes tombstones for its photos and comments in
the same batch. The parent id is a field of the child, inside the ciphertext.

## The timeline is the log, rendered

There is no `activity` type. The object log already records every change in group order with
its author and time inside the document, so the timeline is a projection of it:

1. **Classify.** An object the device had not seen is a create, a known one with a newer
   `edited_at` is an edit, a tombstone a delete. Type and parent name the event: a `photo` under an `album` is "added a photo to <album>", a `message`
   under a `message` is "replied in a thread", a `todo_item` whose `done` flipped is "ticked off
   <text>", an `expense` create is "added <amount> for <description>", a `poll` create is
   "started a poll", a `vote` is "voted".
2. **Collapse.** Same author, same event kind, same parent, within one hour fold into one entry
   ("Anna added 12 photos to Summer"; a `note_update` child is "edited <note>", one entry per
   author per hour). Deletes are hidden by default (a tombstone carries no plaintext to describe
   anyway).
3. **Interleave** with `post` entries, sorted by `authored_at` / `edited_at`, newest first.
4. **Filter** per group by event kind (client preference). Activity in a subgroup appears in the
   parent's timeline for members of that subgroup only, since nobody else holds its key.

Scrolling back past what the device holds asks a member device for the older range
(`RequestHistoryRequest`, `02-ledger.md`), so the feed fills across types in `seq` order. The
same classification produces the local notification text after a silent push; the server never
knows what happened.
