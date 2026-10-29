# Scope

## The idea, in the user's words

> A mobile native app in Swift and Kotlin (and a browser version, Rust compiled to wasm with a
> React visualizer) for user groups: users create or join a group, and in that group there are
> functionalities like a timeline, a shared photo album, todo lists, notes, a splitser (to split
> costs), a calendar with birthdays and events, polls, messaging, a datepicker, recommendations
> of bars/pubs/parties/restaurants/venues on a map, ticket tracking for parties and festivals
> (splitser with the unit being a ticket, 1 per person), and subgroups with all the other
> functionality nested.
>
> The backend acts like a blind store: every value is end-to-end encrypted. An object has a group
> uuid, an optional parent uuid, an id uuid, a type, a value (encrypted protobuf), an optional blob
> (encrypted binary in the private bucket) and a version number. Users must be able to find out
> efficiently whether anything in a group changed, work offline and sync when they are back, and
> learn about deletions even if they were offline when they happened. Run it as cheap as possible.

## Fit gate

**Fits.** The backend is a relay in Postgres (groups, memberships, device keys, 672 hours of
ciphertext per group with counters and acks) plus 672 hours of blobs in the platform's private
bucket. The only lifecycles are the two purges. Everything that needs plaintext — merge,
validation, derived state, rendering — runs on the device, and so does durability: member
devices are the store. The server is small, blind and forgetful.

## Decisions that shape the whole design

These were settled in discussion before this plan was written; they are not open.

| Decision | Choice | Why |
|---|---|---|
| Durability | Member devices are the store. The server relays: an object row lives 672 hours in Postgres, a blob 672 hours in the bucket. Anything older comes from a member device on request (re-inserted into the log with its original numbers; files re-uploaded to their keys). A group whose every device loses its data loses its history; export is the backup. | Round 7 (2026-10-26). Billions of messages must not live in Postgres, and the user wants the server as stupid as possible. Reversed the round-0 choice. |
| Blobs | The bucket is a 672-hour cache: a file's key is derived from its object id and slot, its reference lives inside the object's encrypted value, the bucket's one lifecycle rule expires every file after 28 days, a thumbnail is a field inside the object's value rather than a file (round 14), and a presigned URL under the caller's group prefix is the whole authorization. An expired file is re-requested like old history: every key holder is woken and one re-uploads it to the same key. The server keeps no table of blobs and no holders. Each group may upload 20 GiB per 672 hours. | Round 10 (2026-10-26). Encryption protects the content and the signature protects the access; everything the blob table did besides that is either the bucket's job (expiry) or the history request's (re-upload). |
| Change detection | A per-group commit log: every accepted write takes the next sequence number of its group, assigned under a locked counter row. Clients keep a cursor per group and ack it once they hold the whole group through it; a device's first ack in a group is 0, which gives it a row and pins the group's minimum ack while it catches up. A pull also returns the eviction floor and the minimum ack; a holder whose cursor is below the floor starts over. | One query answers "anything new?"; batched across groups in one call. The ack is the one number per device: a device holds everything from seq 1 through it, and is a holder once it reaches the floor (rounds 12 and 13). |
| Conflicts | Compare-and-set on the object's seq: a push names the seq of the row it edited from, and a conflict returns the current row, tombstone or not. The client re-applies its changed fields onto the returned row and pushes again; the same field changed on both sides goes to the later push; a returned tombstone ends the edit. No conflicted copies, no per-object version, no special case for deletes on the server. | Round 7. The server cannot merge ciphertext; a field-level rebase on the device is enough for small objects edited a field at a time, and needs no CRDT. Round 8 dropped the version: a row's seq already identifies its state. |
| Deletes | A tombstone is a row with no value: it takes a seq like any write, lives 672 hours on the server like any row, and a device keeps the ones it pulled until the eviction floor passes them. After applying a pull a device acks its cursor, and only once it holds the whole group through it; a device catching up acks each page of history as it lands. A device that has not acked a row when the row is evicted is evicted with it, unless its acks show it is catching up: its ack row goes, so it can neither push nor serve history until it has thrown the group away, started over as a new device with everything from seq 1, and acked past the floor again. The server keeps the minimum ack over the group's devices, evicts every row at or below it, tombstones, live rows and restored rows alike, and tells every device the minimum so they drop their tombstones; nothing waits for a device beyond the window. No pull-to-head check before a push and no server rule about editing tombstones: a stale edit or re-creation conflicts on the seq while the tombstone is in the window and is refused from behind the floor after, and un-deleting is a client decision. | Round 7, the user's design, folded into the sequence counter in round 8 (2026-10-26) and stripped of the push-side check: the seq check already refuses every write that could resurrect anything in the window, and the server cannot check parents once they are evicted, so children of deleted parents are a client rule either way. Round 12 (2026-10-28) put the eviction floor under the acked minimum: a device the floor passes is evicted with the rows, so the minimum is never held back for longer than the window, and a device that was away cannot bring a deleted object back. Round 13 let the minimum evict live rows too. |
| Children | No child lists or counts on parents; the parent id is a field inside the child's encrypted value. Deleting a parent is the client's cascade: the children it holds go in the same push, the rest are tombstoned by whoever holds them. | Derived state on the parent is a conflict magnet, and the server does not need to see structure (round 8: the server could not cascade completely anyway once children are evicted). |
| Fetch shape | Catch-up since cursor from the server's 672-hour window; everything older is re-inserted into the window by a member device on request, with the original sequence numbers, and pulled like anything else: a new member, a new device, a device offline longer than the window, or scrolling past what the device holds. Restored rows are evicted again after 672 hours. | The server has nothing older to serve, and nothing else needs a second format: other devices' cursors are already past the restored numbers. |
| Keys | A group key per group, a keyring with key ids; rotate on member removal; new devices get keys wrapped for them by an existing member's device. New members get full history. | MLS is designed to deny history to new members, the opposite of the requirement. |
| Subgroups | A separate group with its own key and `parent_group_id`; members ⊆ parent, enforced server-side; leaving the parent cascades. | A key boundary is a group boundary. |
| Identity | Kratos with one sign-in method: Sign in with Basable, the platform's OpenID Connect login, through Kratos's `oidc` method. No email codes, no passkeys in the app's Kratos, no courier. Identity never carries keys. | Round 1 asked for email code and passkeys; round 17 (2026-10-29) made Sign in with Basable the only method: nothing to set up, the verified email arrives in the id token, and a passkey belongs on Basable's login, not on every app's. |
| Payloads | Protobuf per type, behind a plaintext header in the value that names the type, the `schema_version` and the key id; the server stores the value as one byte string and never parses it (round 16, 2026-10-28). Add fields, never repurpose a number; a breaking change is a new type number. Old clients read newer objects (unknown fields ignored for display) but never write them. A note's CRDT lives inside the payload: a `note` parent plus append-only `note_update` children, each one `yrs` update, combined on the device. | Round 7. Plain values plus a rebase cover every type but notes; a CRDT for everything was more machinery than the app needs. Old clients not writing newer objects is what keeps fields from being dropped. |
| Client core | One Rust core (sync, local store, conflict rebase, the note CRDT, keyring, crypto, schemas) bound to Swift and Kotlin through UniFFI and to the browser through wasm. Three UI shells. | Three sync engines and three crypto implementations diverge. |
| Cost | Single CNPG instance, one app replica in preview, blob traffic never touches the app (presigned URLs), push instead of polling, client-side resize before upload. | |

## Scope ledger

Decision: `include` is in the backend plan now; `later` is designed around but not built; `exclude` is out.

### Groups and membership

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Create group, invite by link/code, accept | include | code with expiry, max uses and the joiner's role; no admin approval of joins | Load-bearing. Approval of join requests was not asked for; an admin controls exposure through max uses and revocation. |
| Leave group, remove member (admin) | include | triggers key rotation | Load-bearing. |
| Subgroups (members ⊆ parent, own key) | include | since round 20 (2026-10-29): `AddMemberRequest` adds a parent member directly, no invite and no tap, the adder's device wrapping the subgroup keys for the devices that hold the parent's key; `group_profile.kind` renders a subgroup as a side chat or a full group | Requested; cheap once groups exist. A chat without one person (a present) is a subgroup: a key boundary is the only thing that hides it, and the subgroup's expenses and photos are hidden with it. |
| Roles: admin, member, viewer | include | viewer reads only: the server refuses its pushes to the log and its approval of other users' devices; it may answer history requests and re-upload files, which is serving, not writing; invites carry the role; admins change it | Round 9 (2026-10-26). A read-only member is one role check in two handlers. |
| Group deletion | include | a `group_purge` processing object in `group` drives the purge of sync, storage and device until each reports nothing left; subgroups are purged first | Round 1. A purge must finish, so it is driven by a reconciler with direct requests, not an event. |
| Data export | include | client-side: everything the device holds, bundled on the device | Round 1. No server work: the server cannot produce a readable export. |
| Account deletion | include | a `user_purge` processing object in `group`: leave every group (promoting the longest-standing member where the user was the only admin, deleting groups where they were the only member), purge the user's devices and keys, delete the Kratos identity | Round 4 (2026-10-17). App stores require it; a purge must finish, so it is a reconciler. |
| Group metadata (name, avatar) | include | stored as an encrypted object of a `group_profile` type, not in the group table | The server stays blind. |

### Devices and keys

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Device registration with public key | include | | Required for key wrapping. |
| Key request from a new device, approved by a person | include | three ways to verify the requesting device's key, all ending in a tap on a holder device: through an invite (the link carries a secret in its URL fragment, never sent to the server; the request carries an HMAC of the device key under it, so a remote join verifies itself), by QR scan in the same room (own devices), or by comparing the code over a channel the two people trust. Never silently. Rotation wraps only for devices that already hold a key. | Round 4. Automatic fulfilment would let a server that inserted a device row obtain the group key, and a silent same-user approval would let it do so under the victim's own user id; with a scan the server never seeds trust, it only relays it. Costs one scan per new device. |
| Key rotation on removal | include | | Cheap; closes the ex-member hole. |
| Push tokens (APNs, FCM) | include | silent push on group head advance | Replaces polling. |
| Key backup / recovery without another member | exclude | | Rejoin via another member's device is the chosen path. |
| Remove a device | include | the owner drops a lost or dead device; its keys go and `sync` stops asking it for history | Round 7. Since round 12 a dead device blocks nothing beyond the window: the eviction that passes its ack takes its row. |

### Object log

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Push with CAS on seq, pull since cursor, heads for all groups | include | | The core. |
| 672-hour window; member devices are the store | include | the age rule of the hourly eviction; the floor is returned on every pull | Round 7, at 48 hours; round 13 (2026-10-28) made it 168; round 15 (2026-10-28) made it 672 for rows and files alike, since the ack rule keeps the common case to hours whatever the window says, and a restart now re-fetches thumbnails too. The window is the longest absence without starting over, and the longest a slow device can cost the server. |
| History re-inserted by member devices | include | `RequestHistory` wakes every holder (every device whose ack has reached the floor holds the whole group), or is answered `unfillable` at once when there is none; one claims the request (first wins; a claim older than 5 minutes is taken over) and pushes the rows in the range back with their original seqs (`PushHistory`), oldest first, tombstones included; ids that have a row are skipped, the head does not move; the requester is woken after each call, pages the restored rows from its ack and acks as it goes; restored rows go by the ack rule as it passes them, or age out after 672 hours; no server retries | Round 7, reshaped in round 10: no bundle format, no bucket involvement. Round 13 (2026-10-28) made a holder mean the whole group, so an answer marked complete is complete. Replaces per-type backfill and id-list reconciliation. |
| Coverage floor per device; history by range, lazily | exclude | | Round 12 proposed a floor to reconcile holders with half the history; round 13 (2026-10-28) removed the half-holder instead: a device acks only once it holds the whole group from seq 1, so every ack row is a complete holder, the ack is the single number, and the request for what the window no longer has is always everything from seq 1. |
| Set reconciliation by hash (Merkle tree, invertible Bloom filter) | exclude | | Round 12. A hash says whether two devices agree, not what one lacks; here every holder holds everything through its ack, so the ack is the comparison. |
| Holder check on push | include | every push from a device that is not a holder (no ack row, or an ack below the eviction floor: still catching up, or evicted with the rows it had not acked) is refused as `behind_floor`, creates included: `base_seq = 0` is the device's claim, and the server cannot tell a create from an edit of a row it no longer has; a new member gets the history and acks past the floor before its first post | Round 12. The no-row branch is only safe when the pusher has seen everything that was evicted; otherwise an edit made while it was away is silently replaced, and a "create" could be a hard-deleted object coming back. The round-8 removal of the pull-to-head check stands: this asks whether the device is a holder, not whether it is at the head. |
| Start over when behind the floor | include | a holder whose cursor is below the eviction floor drops the group (objects and tombstones; files stay as a content-addressed cache; unpushed writes stay in the outbox) and takes the path of a new device: ack 0, which gives it a row and pins the minimum ack, pull the window, ask a holder for everything from seq 1 up to the floor, page and ack the restored rows as they land, ack the head; then it replays the outbox against the fresh store, creates as they are, edits and deletes rebased onto the current row and dropped when the object is gone; files are fetched lazily | Round 12, the user's rule: a device that kept its rows could keep a hard-deleted object and reintroduce it. Round 13: objects come whole, not lazily by range; the retention setting on messages keeps that bounded. The window is the longest absence without starting over. |
| Device eviction | include | the age rule of `evict_objects` also deletes the ack row of every device whose ack is below the new floor and who has not acked for 672 hours; such a device starts over when it is back. A device that is catching up keeps acking, so it keeps its row | Round 12 (2026-10-28), the user's rule: a device that has not acked an evicted row is evicted with it, so no device holds a deletion back for longer than the window, and everyone can hard-delete what was deleted inside it. Round 13 gave the joiner a row from its first ack, so the rule has to spare a device that is making progress. |
| Early eviction of object rows once every device has acked them | include | the ack rule of `evict_objects` deletes rows at or below the minimum ack, live, tombstones and restored rows alike, and moves the floor up to it; a joining device's row at ack 0 pins the minimum, so the rows it is paging through go only as it acks past them | Round 13 (2026-10-28), the user's rule: the ack is the holder tracking that round 10 lacked for blobs, so the cache holds only what some device has not seen yet, and a joiner's history leaves as fast as the joiner reads it. The bucket still ages out. |
| Tombstones until every holder's ack passes them, 672 hours at most; hard delete after | include | see the decision table; the age rule evicts the devices that did not keep up, so the minimum is never held back longer than the window | Round 7, simplified round 8, bounded by the floor in round 12. |
| Per-type backfill, id-list reconciliation | exclude | | Round 7. Superseded. Tombstone retention by time, excluded here in round 7, is back in round 12 as the bound on the acked minimum. |
| Server-side cascade on parent delete | exclude | the deleting client batches the children it holds; the orphan rule covers the rest | Round 8. The server no longer sees `parent_id` and never held children older than 672 hours. |
| Per-object ACL within a group | exclude | | A group is the permission boundary. |

### Blobs

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Presigned upload/download via the private bucket | include | | Traffic never touches the app. |
| Retention and expiry | include | one bucket lifecycle rule: every file 28 days after upload | Round 4 wanted a short cache; round 10 made it the bucket's job, no server timer; round 14 (2026-10-28) moved thumbnails into the value, so one rule covers the whole bucket and no prefix needs its own. |
| Thumbnail inside the object's value | include | `photo`, `group_profile` and `member_profile` carry their thumbnail as a bytes field, at most 64 KiB, inside the encrypted value; the bucket holds only full-size files and attachments | Round 14 (2026-10-28), the user's call. A holder of the object holds the grid: an album is complete wherever the object is, offline, after a restart, with the full size long expired, and a history answer brings the grids with it. The price is that every member syncs every thumbnail, bounded by the cap and the window; a push has no byte cap beyond the 1 MiB value. |
| Re-upload of an expired file | include | a history request for the object wakes every key holder; one re-uploads to the same key, confirms with the request id, and the requester is woken | Round 10. One mechanism for old history and expired files; no holder bookkeeping. |
| Early eviction of blobs once every device holds it | exclude | | Round 10. Needed holder tracking; 28 days bound the cost anyway. Object rows got it in round 13 from the ack. |
| Per-group upload cap | include | 20 GiB per rolling 168 hours, from an hourly counter table | Round 10. The server cannot know what expired without blob rows, so it bounds what goes in, which is the cost that matters. |
| Wifi-only sync, native album export | n/a | client-side preferences | Nothing for the backend. |
| Upload quality per device (and per group) | n/a | applied before encryption, so it sets what everyone gets; the uploaded bytes are the file; the thumbnail is always generated at a fixed size, under 64 KiB, and lives inside the value | Round 11 (2026-10-26). Client-side; nothing for the backend. |
| Local quality per device for downloaded photos | n/a | applied after decryption, affects only that device's copy; a device that keeps a reduced copy is no longer a holder of that file (the reference's hash is of the uploaded ciphertext), so it answers history requests but cannot re-upload it; a device always keeps its own uploads byte for byte | Round 11. Client-side; the holder rule is what keeps re-uploads correct. |

### Feature types (all client-side; the server sees only bytes)

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Timeline posts, photo album, todo lists, calendar events and birthdays, polls, datepicker (scheduling poll), messaging, venue recommendations on a map, splitser expenses, tickets as expenses with per-share blob | include | catalogue with type numbers and keys in `06-object-types.md` | All are object types; none need backend logic. |
| Timeline shows every kind of activity (photos added, threads, todos ticked, expenses, polls) | include | derived from the local store on the device, collapsed per author/kind/parent/hour, interleaved with posts; no `activity` type (`06-object-types.md`) | Round 6 (2026-10-17). The log is already the activity stream; storing it twice would break "inputs only". |
| Notes: CRDT updates as child objects | include | `note` parent plus append-only `note_update` children (one `yrs` update each, batched per editing session); combined on the device; compaction later | Round 7 (2026-10-26). Children never conflict, so collaborative notes need no merge loop at all. |
| Field-level rebase on conflict | include | re-apply the changed fields onto the current row, validate, push again; later push wins a same-field collision; documents store inputs only (an expense stores amount, payer, participants and a split rule; shares and balances are derived) | Round 7. Replaces both the conflicted copy (round 1) and CRDT for every type (round 3). |
| CRDT (`yrs`) for every type | exclude | | Round 7. More machinery than the app needs; a chat message is never merged. |
| Conflicted copy on collision | exclude | | Round 3, confirmed round 7. |
| Ticket payments (moving money) | exclude | | Round 1. Tracking only; a ticket is an expense with one share per person. |
| Cross-group friend graph for recommendations | exclude | | Needs a server-visible social graph; incompatible with E2E. Recommendations are per group. |
| Read receipts, typing indicators | exclude | | Round 1. No live presence; silent push plus pull is the delivery model. |
| Auto-delete of old messages | include | a per-group setting on `group_profile`, `message_retention_months` (default 12, 0 = off): any device that sees a `message` older than that tombstones it, with the children it holds, batched; the first to notice does it | Round 13 (2026-10-28), the user's rule, off by choice. Keeps "everything from seq 1" a bounded download; photos live in albums and posts, not only in the chat. |
| Expense pools settled independently | include | `expense_pool` (43), the parent of `expense` and `settlement`; balances derived per pool; a general pool per group as a singleton | Round 20 (2026-10-29). Same people, separate tabs: no key boundary needed, so an object type, not a subgroup. |
| Group calendar in the user's own calendar | include | client-side: the phones mirror a group's events and member birthdays into a system calendar per group (EventKit, the Android calendar provider), which the user keeps local or puts in an account calendar that syncs on its own; the web client exports an `.ics` file | Round 20 (2026-10-29). Live in practice: the mirror updates on every sync. |
| Calendar subscription URL (webcal) | exclude | | Round 20. A feed is plaintext a calendar app fetches from the server, which holds only ciphertext; a device uploading a plaintext feed would put on the server the one thing it never sees. |

### Platform

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Web client (React + wasm core) in this repo | include | `frontend: react`; the browser is a full device of the same user: its own device key, paired by QR, syncing from the server like a phone. Login is Sign in with Basable in the browser (round 17); pairing is the one QR the phone scans. "Remember this browser" is optional: without it keys stay in memory for the session. | Round 5 (2026-10-17). A companion that mirrors the phone over wifi dies when the phone locks and needs a second sync protocol; a separate user would be another person in every group. |
| Keeper device | later | a client setting: a device (a home computer, a browser profile) that never reduces local quality and keeps every object and file of its groups, so there is always a holder | Round 11. Nothing on the server; it is the user's answer to "what if nobody holds it". |
| Direct device-to-device file transfer (WebRTC) | later | an optimisation of the re-request path, signalled through the server so it works on any network | Not needed: a re-request through the bucket already moves photos from phone to browser. |
| Shared Rust client core in this repo | include | a `core/` crate in the Bazel workspace, compiled to wasm for the web client in the same build; the mobile repositories pin a commit of this repository and build it with UniFFI | Round 4. One repository versions server contract and client core together. |
| Database backups | later | private bucket exists once `objectStorage` is declared | Add before live has users. |
| Email delivery for Kratos codes | exclude | no courier: the only sign-in method sends no mail; the scaffold's optional `SMTP_CONNECTION_URI` reference goes together with the `code` method | Round 17 (2026-10-29). Was include since round 1, when email codes were the login. |

## Open candidates (not yet asked)

None. Round 1 (2026-10-14) decided ticket payments, presence and group deletion; the invite
model was defaulted (code with expiry and max uses). Round 2 (2026-10-15) reversed round 1 on
notes: a CRDT with merge-on-conflict. Round 3 (2026-10-17) extended that to every type: every
value is a `yrs` document, conflicted copies are gone, and payload schemas are key schemas over
the document instead of protobuf messages. Round 4 (2026-10-17) decided human approval of key
requests, account deletion, a 20 GiB per-group cap with 48-hour blob retention, and the client
core living in this repository. Round 5 (2026-10-17) made the browser a full device paired by
QR and removed silent same-user approval. Round 6 (2026-10-17) made the timeline a projection of
the log. Round 7 (2026-10-26) made member devices the store: the server keeps 48 hours, history
comes from member devices, deletion is acked per device so tombstones can be hard
deleted, the blob processing object became a row with an expiry, and values went back to plain
protobuf with a field-level rebase, the CRDT living only inside notes as child objects. Round 8
(2026-10-26) folded the deletion generation into the sequence counter, dropped the push-side
cursor check, the per-object version, the server-side cascade and the tombstone special cases.
Round 9 (2026-10-26) added the `viewer` role. Round 10 (2026-10-26) removed the blob and holder
tables: derived keys, references inside the value, bucket lifecycle rules, re-upload through
history requests, an upload cap per 48 hours; and history answers became re-inserts into the log
with the original seqs instead of bundle files. Round 11 (2026-10-26) noted the per-device
upload and local image quality settings as client-side. Round 12 (2026-10-28) refused every
push from a device that is not a holder, made a device the eviction passes start over, and put
the eviction floor under the acked minimum, so that no device holds deletion back for longer
than the window and none can bring a deleted object back. Round 13 (2026-10-28) moved the
window from 48 to 168 hours, let the minimum ack evict live rows as well as tombstones, dropped
the coverage floor that round 12 had proposed (a device acks only once it holds the whole group
from seq 1, so every holder is complete and a joining or restarted device asks for everything),
and added the chat's retention setting, which keeps that download bounded. Its last step gave
the joiner a row from its first ack, 0: that ack pins the group's minimum, so nothing can be
evicted from under a device that is catching up, the restored rows leave as the joiner acks past
them, and the device-eviction rule spares a device whose acks show progress. Round 14
(2026-10-28) put thumbnails inside the object's value: the bucket holds only full-size files
and attachments under one seven-day rule, and a holder of an object holds its grid. What remains
is tuning (the value size cap, the thumbnail cap, the history batch size), which the
implementing session owns. Round 15 (2026-10-28) moved the window to 672 hours for rows,
history requests and files: since round 13 a row leaves as soon as every device has acked it,
so the window only says how long a device may stay away and how much a slow one costs, and a
restart re-fetches thumbnails now. The upload cap stays at 20 GiB per 168 hours as a rate, so
the bucket holds at most four times that per group. Round 16 (2026-10-28) moved the type, the
schema version and the key id out of the `object` columns into a plaintext header inside the
value: the key id must be readable before decryption, and a plaintext header is, so the server
keeps three fewer columns it never read and the header travels with the ciphertext as its
associated data. The same round dropped `created_by_device`: nothing read it, the author is
inside the ciphertext, and the push fan-out knows its caller without a column. Round 17
(2026-10-29) made Sign in with Basable the only sign-in method: Kratos stays as the app's local
identity (user ids, sessions, the account purge are unchanged) with its `oidc` method alone,
the provider being the `basable-login` client the platform writes into each environment; email
codes, passkeys and the courier went, and with them the SMTP secret. The browser signs in with
Basable and pairs with one QR; the phones run Kratos's native OIDC flow through the system
browser and exchange the returned code for a session token. Round 18 (2026-10-29) renamed the
`ledger` nanoservice to `sync`: on this platform "ledger" is the declarative double-entry
accounting ledger a nanoservice may own (`owns.ledger`), and this one is an object log that
devices sync against. Same tables, same messages; only the name and `SyncService` changed.
Round 19 (2026-10-29), from the scaffold's manifest check: `key_request` and `history_request`
lost their `status` column, since a lifecycle column on a plain table is what a processing
object is for and neither is one: a row is an open request, a claim is the nullable
`claimed_by` and `claimed_at` pair, and fulfilment deletes the row; `cancelled` was set by no
message and went. The push calls are `send_apns` and `send_fcm`, since call names are unique per
nanoservice. Round 20 (2026-10-29): a chat without one member (a present) is a subgroup, which
already hides everything behind its key; what was missing was a way in without an invite link
and a tap, so `AddMemberRequest` adds a parent member to a subgroup directly and subgroup trust
is parent trust (the adder's device wraps the subgroup keys for the devices that hold the
parent's key, and a holder fulfils a subgroup key request from such a device without a tap);
`group_profile.kind` says whether a subgroup renders as a side chat or a full group. Expenses
got pools: `expense_pool` is the parent of `expense` and `settlement`, balances are derived per
pool, and a general pool per group is a singleton. Events reach the user's own calendar through
the phone's system calendar, mirrored per group and synced by the user's own account if they put
it there; a subscription URL is excluded because the server cannot serve plaintext.

## Status

ready

