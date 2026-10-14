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
| Blobs | Full-size blobs expire from the bucket after a retention period and can be re-requested from a device that still holds them (the author's camera roll is the holder of last resort). Thumbnails are kept long-term. | Photos are the only real cost line. |
| Change detection | A per-group commit log: every accepted write takes the next sequence number of its group, assigned under a locked counter row. Clients keep a cursor per group. | One query answers "anything new?"; batched across groups in one call. |
| Conflicts | Per-object version with compare-and-set on push: first to sync wins, the loser is told and keeps a local copy. Deletes always win. | The server cannot merge ciphertext. |
| Deletes | Tombstones in the log, garbage-collected after a retention window; a client with an older cursor does a full id/version reconciliation. | The only way an offline client learns about a deletion. |
| Children | No child lists or counts on parents; `parent_id` on the child, indexed. | Derived state on the parent is a conflict magnet. |
| Fetch shape | Catch-up (all types since cursor) plus per-type backfill (e.g. the last 200 messages). | Message-like types must not force a full history pull. |
| Keys | A group key per group, a keyring with key ids; rotate on member removal; new devices get keys wrapped for them by an existing member's device. New members get full history. | MLS is designed to deny history to new members, the opposite of the requirement. |
| Subgroups | A separate group with its own key and `parent_group_id`; members ⊆ parent, enforced server-side; leaving the parent cascades. | A key boundary is a group boundary. |
| Identity | Kratos, email code and passkeys. Identity never carries keys. | Platform default; exactly the requested login. |
| Payloads | Protobuf per type, `schema_version` in plaintext metadata. Add fields, never move them; a breaking change is a new type number. Old clients may read newer objects (display-only downgrade) but never write them. Declarative upgrade/downgrade patches ported from rootline's `JsonPatch` vocabulary, minus `JsonTransform`, over a dynamic message that preserves unknown fields. | Preserving unknown fields is what makes old writers safe; rootline's JSON round trip would strip them. |
| Client core | One Rust core (sync, store, merge, keyring, crypto, schema patches) bound to Swift and Kotlin through UniFFI and to the browser through wasm. Three UI shells. | Three sync engines and three crypto implementations diverge. |
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
| Group metadata (name, avatar) | include | stored as an encrypted object of a `group_profile` type, not in the group table | The server stays blind. |

### Devices and keys

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Device registration with public key | include | | Required for key wrapping. |
| Key request from a new device, fulfilled by a member | include | needs another member online at some point | Accepted by the user. |
| Key rotation on removal | include | | Cheap; closes the ex-member hole. |
| Push tokens (APNs, FCM) | include | silent push on group head advance | Replaces polling. |
| Key backup / recovery without another member | exclude | | Rejoin via another member's device is the chosen path. |

### Object log

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Push with CAS, pull since cursor, heads for all groups | include | | The core. |
| Per-type backfill | include | | Messages. |
| Tombstones with retention + full reconciliation | include | 90 days | |
| Server-side cascade on parent delete | include | tombstones the subtree in the same transaction | The server sees `parent_id`. |
| Per-object ACL within a group | exclude | | A group is the permission boundary. |

### Blobs

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Presigned upload/download via the private bucket | include | | Traffic never touches the app. |
| Retention and expiry per blob kind | include | full-size 30 days, thumbnails long-term | The user proposed 3 days; 30 costs cents and avoids the holiday cliff. |
| Re-upload request to a holder device | include | | The one place fill-on-miss pays. |
| Wifi-only sync, local resize, native album export | n/a | client-side preferences | Nothing for the backend. |

### Feature types (all client-side; the server sees only a type number)

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Timeline posts, photo album, todo lists, calendar events and birthdays, polls, datepicker (scheduling poll), messaging, venue recommendations on a map, splitser expenses, tickets as expenses with per-share blob | include | | All are object types; none need backend logic. |
| Notes: conflicted copy on collision | include | | Cheapest correct behaviour. |
| Notes: CRDT (yrs/automerge) | exclude | | Round 1. Conflicted copies are enough; a CRDT note would be a new type number if ever wanted. |
| Ticket payments (moving money) | exclude | | Round 1. Tracking only; a ticket is an expense with one share per person. |
| Cross-group friend graph for recommendations | exclude | | Needs a server-visible social graph; incompatible with E2E. Recommendations are per group. |
| Read receipts, typing indicators | exclude | | Round 1. No live presence; silent push plus pull is the delivery model. |

### Platform

| Feature | Decision | Note | Rationale |
|---|---|---|---|
| Web client (React + wasm core) in this repo | include | `frontend: react` | Requested. |
| Database backups | later | private bucket exists once `objectStorage` is declared | Add before live has users. |
| Email delivery for Kratos codes | include | user-supplied SMTP secret | Required for login. |

## Open candidates (not yet asked)

None. Round 1 (2026-10-14) decided ticket payments, collaborative notes, presence and group
deletion; the invite model was defaulted (code with expiry and max uses). What remains is
tuning (retention numbers, worker policy), which the implementing session owns.

## Status

ready

