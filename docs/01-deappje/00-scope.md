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

**Fits.** The backend is a relay in Postgres (groups, memberships, device keys, 48 hours of
ciphertext per group with counters and acks) plus 48 hours of blobs in the platform's private
bucket. The only lifecycles are the two purges. Everything that needs plaintext — merge,
validation, derived state, rendering — runs on the device, and so does durability: member
devices are the store. The server is small, blind and forgetful.

## Decisions that shape the whole design

These were settled in discussion before this plan was written; they are not open.

| Decision | Choice | Why |
|---|---|---|
| Durability | Member devices are the store. The server relays: an object row lives 48 hours in Postgres, a blob 48 hours in the bucket. Anything older comes from a member device on request (history bundles, blob re-uploads). A group whose every device loses its data loses its history; export is the backup. | Round 7 (2026-10-26). Billions of messages must not live in Postgres, and the user wants the server as stupid as possible. Reversed the round-0 choice. |
| Blobs | The bucket is a 48-hour cache for full-size blobs and attachments, extended on every download and emptied early once every key-holding device of the group has downloaded it; after that a blob can be re-requested from any device that downloaded it (the server records holders; a device that evicts under storage pressure tells the server; the author's camera roll is the holder of last resort). Thumbnails are kept long-term. Each group has a 20 GiB cap on what is in the bucket. | Photos are the only real cost line, and every device that viewed a photo already holds it. |
| Change detection | A per-group commit log: every accepted write takes the next sequence number of its group, assigned under a locked counter row. Clients keep a cursor per group. A pull also returns the eviction floor; a cursor below it means the gap comes from a member device. | One query answers "anything new?"; batched across groups in one call. |
| Conflicts | Compare-and-set on the object's seq: a push names the seq of the row it edited from, and a conflict returns the current row, tombstone or not. The client re-applies its changed fields onto the returned row and pushes again; the same field changed on both sides goes to the later push; a returned tombstone ends the edit. No conflicted copies, no per-object version, no special case for deletes on the server. | Round 7. The server cannot merge ciphertext; a field-level rebase on the device is enough for small objects edited a field at a time, and needs no CRDT. Round 8 dropped the version: a row's seq already identifies its state. |
| Deletes | A tombstone's deletion mark is its own seq. After applying a pull a device acks its cursor; the server keeps the minimum over the group's devices, hard-deletes tombstones at or below it, and tells every device the minimum so they drop theirs. Acks are per device and happen after a pull, never by pushing. No "pull before you push" check and no server rule about editing tombstones: a stale edit or re-creation conflicts on the seq like any other, and un-deleting is a client decision. | Round 7, the user's design, folded into the sequence counter in round 8 (2026-10-26) and stripped of the push-side check: the seq check already refuses every write that could resurrect anything, and the server cannot check parents once they are evicted, so children of deleted parents are a client rule either way. |
| Children | No child lists or counts on parents; the parent id is a field inside the child's encrypted value. Deleting a parent is the client's cascade: the children it holds go in the same push, the rest are tombstoned by whoever holds them. | Derived state on the parent is a conflict magnet, and the server does not need to see structure (round 8: the server could not cascade completely anyway once children are evicted). |
| Fetch shape | Catch-up since cursor from the server's 48-hour window; everything older as a history bundle from a member device: a new member, a new device, a device offline longer than the window, or scrolling past what the device holds. | The server has nothing older to serve. |
| Keys | A group key per group, a keyring with key ids; rotate on member removal; new devices get keys wrapped for them by an existing member's device. New members get full history. | MLS is designed to deny history to new members, the opposite of the requirement. |
| Subgroups | A separate group with its own key and `parent_group_id`; members ⊆ parent, enforced server-side; leaving the parent cascades. | A key boundary is a group boundary. |
| Identity | Kratos, email code and passkeys. Identity never carries keys. | Platform default; exactly the requested login. |
| Payloads | Protobuf per type, `schema_version` in plaintext metadata. Add fields, never repurpose a number; a breaking change is a new type number. Old clients read newer objects (unknown fields ignored for display) but never write them. A note's CRDT lives inside the payload: a `note` parent plus append-only `note_update` children, each one `yrs` update, combined on the device. | Round 7. Plain values plus a rebase cover every type but notes; a CRDT for everything was more machinery than the app needs. Old clients not writing newer objects is what keeps fields from being dropped. |
| Client core | One Rust core (sync, local store, conflict rebase, the note CRDT, keyring, crypto, schemas) bound to Swift and Kotlin through UniFFI and to the browser through wasm. Three UI shells. | Three sync engines and three crypto implementations diverge. |
| Cost | Single CNPG instance, one app replica in preview, blob traffic never touches the app (presigned URLs), push instead of polling, client-side resize before upload. | |

## Scope ledger

Decision: `include` is in the backend plan now; `later` is designed around but not built; `exclude` is out.

### Groups and membership

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Create group, invite by link/code, accept | include | code with expiry and max uses; no admin approval of joins | Load-bearing. Approval of join requests was not asked for; an admin controls exposure through max uses and revocation. |
| Leave group, remove member (admin) | include | triggers key rotation | Load-bearing. |
| Subgroups (members ⊆ parent, own key) | include | | Requested; cheap once groups exist. |
| Roles beyond admin/member | exclude | | Not asked for. |
| Group deletion | include | a `group_purge` processing object in `group` drives the purge of ledger, storage and device until each reports nothing left; subgroups are purged first | Round 1. A purge must finish, so it is driven by a reconciler with direct requests, not an event. |
| Data export | include | client-side: pull since 0 plus a download per blob, bundled on the device | Round 1. No server work: the server cannot produce a readable export. |
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
| Remove a device | include | the owner drops a lost or dead device; its keys go and `ledger` stops counting its ack | Round 7. A device that never acks blocks hard deletion for its groups. |

### Object log

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Push with CAS on seq, pull since cursor, heads for all groups | include | | The core. |
| 48-hour window; member devices are the store | include | hourly eviction of live rows; the floor is returned on every pull | Round 7. Postgres holds two days of activity per group, whatever the group's age. |
| History bundles from member devices | include | `RequestHistory` wakes every key holder; one uploads a `bundle` blob; the requester is woken; no server retries | Round 7. Replaces per-type backfill and id-list reconciliation. |
| Tombstones until every device's acked cursor passes them; hard delete after | include | see the decision table | Round 7, simplified round 8. |
| Per-type backfill, id-list reconciliation, tombstone retention by time | exclude | | Round 7. Superseded. |
| Server-side cascade on parent delete | exclude | the deleting client batches the children it holds; the orphan rule covers the rest | Round 8. The server no longer sees `parent_id` and never held children older than 48 hours. |
| Per-object ACL within a group | exclude | | A group is the permission boundary. |

### Blobs

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Presigned upload/download via the private bucket | include | | Traffic never touches the app. |
| Retention and expiry per blob kind | include | full-size, attachments and bundles 48 hours after upload; thumbnails long-term (a config number; set to 2 for a pure cache) | Round 4: the user wants the bucket to be a short cache. Round 7 dropped extension on download: a blob is a row with an expiry, not a state machine. |
| Re-upload request to a holder device | include | the server records which devices downloaded a blob and wakes all of them plus the uploader, batched per request; a device drops its hold when it evicts, or when woken for bytes it no longer has; a re-upload keeps id, key and hash | With a 48-hour cache the holder set must be wider than the uploader, and phones with little storage must be free to evict. |
| Early eviction once every device holds it | include | the pass asks `device` whether the holders cover every key-holding device of the group; if so the bucket copy goes before the 48 hours | Round 4, the user's rule: a blob everyone has is pure cost. A device joining later re-requests. |
| Per-group storage cap | include | 20 GiB in the bucket per group, a `storage_quota` config value; `CreateUpload` refuses beyond it | Round 4. Bounds the bill; retention keeps real usage far below it. |
| Wifi-only sync, local resize, native album export | n/a | client-side preferences | Nothing for the backend. |

### Feature types (all client-side; the server sees only a type number)

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

### Platform

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Web client (React + wasm core) in this repo | include | `frontend: react`; the browser is a full device of the same user: its own device key, paired by QR, syncing from the server like a phone. Login is a passkey, and WebAuthn cross-device authentication is the QR-scanned-by-phone login for free. "Remember this browser" is optional: without it keys stay in memory for the session. | Round 5 (2026-10-17). A companion that mirrors the phone over wifi dies when the phone locks and needs a second sync protocol; a separate user would be another person in every group. |
| Direct device-to-device blob transfer (WebRTC) | later | an optimisation of the holder re-request path, signalled through the server so it works on any network | Not needed: a re-request through the bucket already moves photos from phone to browser. |
| Shared Rust client core in this repo | include | a `core/` crate in the Bazel workspace, compiled to wasm for the web client in the same build; the mobile repositories pin a commit of this repository and build it with UniFFI | Round 4. One repository versions server contract and client core together. |
| Database backups | later | private bucket exists once `objectStorage` is declared | Add before live has users. |
| Email delivery for Kratos codes | include | user-supplied SMTP secret | Required for login. |

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
comes from member devices as bundles, deletion is acked per device so tombstones can be hard
deleted, the blob processing object became a row with an expiry, and values went back to plain
protobuf with a field-level rebase, the CRDT living only inside notes as child objects. Round 8
(2026-10-26) folded the deletion generation into the sequence counter and dropped the push-side
cursor check. What remains is tuning (the 48-hour window, the value size cap, bundle size), which
the implementing session owns.

## Status

ready

