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

**Fits.** The backend is a durable store in Postgres (groups, memberships, device keys, an
append-only object log with tombstones) plus blobs in the platform's private bucket, with one
genuine lifecycle (a blob's retention and re-upload cycle). Everything that needs plaintext —
merge, validation, derived state, rendering — runs on the device, which is the point of the
design. The server is small and blind.

## Decisions that shape the whole design

These were settled in discussion before this plan was written; they are not open.

| Decision | Choice | Why |
|---|---|---|
| Durability | The server is the durable store, not a cache. No fill-on-miss for objects. | Fill-on-miss needs presence, a request queue and a holder of last resort; encrypted JSON is nearly free to keep. |
| Blobs | The bucket is a 48-hour cache for full-size blobs and attachments, extended on every download and emptied early once every key-holding device of the group has downloaded it; after that a blob can be re-requested from any device that downloaded it (the server records holders; a device that evicts under storage pressure tells the server; the author's camera roll is the holder of last resort). Thumbnails are kept long-term. Each group has a 20 GiB cap on what is in the bucket. | Photos are the only real cost line, and every device that viewed a photo already holds it. |
| Change detection | A per-group commit log: every accepted write takes the next sequence number of its group, assigned under a locked counter row. Clients keep a cursor per group. | One query answers "anything new?"; batched across groups in one call. |
| Conflicts | Per-object version with compare-and-set on push; a conflict returns the current row. Every value is a `yrs` CRDT document, so the client merges the returned row into its local document and pushes again. No conflicted copies anywhere. Deletes always win. | The server cannot merge ciphertext, so the merge runs on the device; a CRDT makes it automatic and deterministic for every type, not only notes. |
| Deletes | Tombstones in the log, garbage-collected after a retention window; a client with an older cursor does a full id/version reconciliation. | The only way an offline client learns about a deletion. |
| Children | No child lists or counts on parents; `parent_id` on the child, indexed. | Derived state on the parent is a conflict magnet. |
| Fetch shape | Catch-up (all types since cursor) plus per-type backfill (e.g. the last 200 messages). | Message-like types must not force a full history pull. |
| Keys | A group key per group, a keyring with key ids; rotate on member removal; new devices get keys wrapped for them by an existing member's device. New members get full history. | MLS is designed to deny history to new members, the opposite of the requirement. |
| Subgroups | A separate group with its own key and `parent_group_id`; members ⊆ parent, enforced server-side; leaving the parent cascades. | A key boundary is a group boundary. |
| Identity | Kratos, email code and passkeys. Identity never carries keys. | Platform default; exactly the requested login. |
| Payloads | Every value is a `yrs` document: a `Map` at the root whose keys are the type's fields, `Array` for lists, `Text` for prose. Per type a key schema (name, kind, required, default) with `schema_version` in plaintext metadata. Add keys, never repurpose them; a breaking change is a new type number. Old clients read newer documents (unknown keys are ignored for display and preserved by the CRDT) and may write to them, touching only keys they know. Upgrade steps are declarative key operations (add with default, rename by copy, drop), replacing the rootline `JsonPatch` port. | The CRDT preserves unknown keys by construction, which is what made the protobuf plan need a dynamic-message layer; with every value a document the layer is gone and old writers are safe. |
| Client core | One Rust core (sync, store, `yrs` documents and their typed projections, post-merge validation, keyring, crypto, key-schema upgrades) bound to Swift and Kotlin through UniFFI and to the browser through wasm. Three UI shells. | Three sync engines and three crypto implementations diverge. |
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

### Object log

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Push with CAS, pull since cursor, heads for all groups | include | | The core. |
| Per-type backfill, and across types | include | `type` optional on backfill | Messages; the timeline scrolling back across every type. |
| Tombstones with retention + full reconciliation | include | 90 days | |
| Server-side cascade on parent delete | include | tombstones the subtree in the same transaction | The server sees `parent_id`. |
| Per-object ACL within a group | exclude | | A group is the permission boundary. |

### Blobs

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Presigned upload/download via the private bucket | include | | Traffic never touches the app. |
| Retention and expiry per blob kind | include | full-size and attachments 48 hours after upload or last download; thumbnails long-term | Round 4: the user wants the bucket to be a short cache. |
| Re-upload request to a holder device | include | the server records which devices downloaded a blob and wakes all of them plus the uploader; a device drops its hold when it evicts, or when woken for bytes it no longer has | The one place fill-on-miss pays; with a 48-hour cache the holder set must be wider than the uploader, and phones with little storage must be free to evict. |
| Early eviction once every device holds it | include | the pass asks `device` whether the holders cover every key-holding device of the group; if so the bucket copy goes before the 48 hours | Round 4, the user's rule: a blob everyone has is pure cost. A device joining later re-requests. |
| Per-group storage cap | include | 20 GiB in the bucket per group, a `storage_quota` config value; `CreateUpload` refuses beyond it | Round 4. Bounds the bill; retention keeps real usage far below it. |
| Wifi-only sync, local resize, native album export | n/a | client-side preferences | Nothing for the backend. |

### Feature types (all client-side; the server sees only a type number)

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Timeline posts, photo album, todo lists, calendar events and birthdays, polls, datepicker (scheduling poll), messaging, venue recommendations on a map, splitser expenses, tickets as expenses with per-share blob | include | catalogue with type numbers and keys in `06-object-types.md` | All are object types; none need backend logic. |
| Timeline shows every kind of activity (photos added, threads, todos ticked, expenses, polls) | include | derived from the object log on the device, collapsed per author/kind/parent/hour, interleaved with posts; no `activity` type (`06-object-types.md`) | Round 6 (2026-10-17). The log is already the activity stream; storing it twice would break "inputs only". |
| CRDT (`yrs`) for every type, merge-on-conflict | include | every value is a `yrs` document, full state per push, merged on the client when the push conflicts; nothing on the server beyond the conflict carrying the current row | Round 2 (2026-10-15) added it for notes; round 3 (2026-10-17) made it the only model. Two members editing different fields of one object offline both keep their edits, and one code path serves all types. |
| Post-merge validation ("needs attention") | include | documents store inputs only (an expense stores amount, payer, participants and a split rule; shares and balances are derived), so most invariants hold by construction; the rest (dates ordered, a vote naming a deleted option) are checked after every merge and a failing object is flagged for a person to fix, never silently accepted or rewritten | A CRDT guarantees convergence, not validity. |
| Conflicted copy on collision | exclude | | Round 3. Superseded by the CRDT merge. |
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
the log and the backfill type optional. What remains is tuning (worker policy, the value size
cap), which the implementing session owns.

## Status

ready

