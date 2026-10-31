# Testing

## Unit tests (per nanoservice crate)

- `group`: invite acceptance rules, subgroup subset rule, last-admin rule.
- `device`: rotation batch acceptance, member checks on wrap.
- `relay`: the CAS-on-seq matrix, eviction floor arithmetic, response shaping for pull.
- `storage`: key parsing, quota and slot cap at the boundary.

## `effecttest` audits (one per adapter, in each nanoservice's `tests/effects_audit.rs`)

| Nanoservice | Adapter | Simulator models |
|---|---|---|
| device | apns.send_apns | accepted, rejected token (410), transport failure, ack loss |
| device | fcm.send_fcm | accepted, `UNREGISTERED`, transport failure, ack loss |
| storage | object_storage.delete_object | present, absent, transport failure, ack loss |
| group | kratos.delete_identity | present, absent (404), transport failure, ack loss |

All four are `idempotent`, so the audit proves a re-send after ack loss converges and that a
definitive rejection is classified as such.

## Integration suites (`basable-testkit`, testcontainers Postgres, one database per test)

- **Membership cascade**: parent removal removes subgroup memberships atomically; concurrent
  single-use invite.
- **Viewer**: a viewer pulls, acks, downloads, answers a history request and re-uploads a file
  it holds, and is refused on push and on approving another user's device;
  promotion to member lifts the refusals without a key change.
- **Sequence integrity**: N tasks push to one group concurrently; the resulting `seq` set is
  `1..=N` with no gaps; a reader polling during the run never observes a seq it later misses.
- **Delete semantics**: a parent and its children in one batch take consecutive seqs; a delete
  and an edit racing on one object leave exactly one of them accepted and the other holding the
  current row; a stale holder's edit or re-creation of a deleted object conflicts while the
  tombstone is here, and a device that is not a holder is refused as `behind_floor`; the
  **holder invariant** under random interleavings of push, delete, pull, ack and both evictions
  across N devices (every holder has applied every row the server evicted, every row the ack
  rule removed was at or below every device's ack, and no holder holds a live copy of an object
  whose tombstone is gone); the age rule evicts a device below the floor that has not acked for
  672 hours and leaves one that is catching up; a backwards ack is refused and 0 resets the
  row; an edit evicted while a device was offline is never replaced silently; a batch of
  creates from a non-holder is refused like any other, so a new member posts only after its
  history has arrived and its ack has reached the floor; a device that falls behind starts over
  and ends with no object that was deleted while it was away.
- **History**: a joining device acks 0, pulls the window, asks for everything up to the floor,
  pages and acks the restored rows as they land, following its request through the relay with or
  without the wake-up, and acks the head; its ack 0 pins the minimum
  so nothing is evicted by ack from under it, and the restored rows go as it acks past them;
  two simulated holders claim and exactly one wins; it pushes the rows back oldest first,
  tombstones included; they land at their original seqs, the head does not move, an id with a
  current row is skipped, a push without the claim is refused, a claim from a non-holder is
  refused; a request from a group with no other holder is answered `unfillable` without a row;
  `complete` fulfils the request once and a later push for it is refused; a claimer that stops
  is taken over after 5 minutes and the take-over re-push restores only what is missing; the
  joiner ends with the same rows as the holder; a restored row the requester never acks is
  evicted 672 hours after the re-insert and no earlier.
- **Key lifecycle**: register two devices, request keys, fulfil, remove member, rotation batch,
  `GetWrappedKeys` returns both key ids for a surviving device and nothing for the removed one.
- **Files**: against the bucket simulator: presign, upload, confirm, download; a key outside the
  caller's group prefix is refused; an expired file answers `gone`, a request for the object
  wakes the holders, a re-upload to the same key makes the download work again; `ObjectDeleted`
  removes every slot of the object; purge empties the group prefix over several passes.
- **Cross-nanoservice**: `ObjectDeleted` for an object with no files is a no-op; `GroupHeadAdvanced`
  fan-out excludes the writer device; `DeviceLeftGroup` unblocks a group's gc.
- **Group purge**: a parent with a subgroup, objects, files and keys; after deletion every table
  in every nanoservice holds nothing for either group, the bucket simulator holds no keys, the
  purge objects are gone, and a member check answers "not a member" from the first commit on.
  A `PurgeResult` dropped once is retried and the end state is the same.
- **Account purge**: sole admin of one group, sole member of another, member of a third; ends with
  a promoted admin, a group purge, a rotation push, no device rows, one identity deletion.
- **Device trust**: a device row inserted directly into the table (simulating a hostile server)
  appears in no `ListDevices` answer and receives no wrapped key through a rotation.
- **Quota**: an upload that would cross the group's 168-hour cap is refused and one that lands
  exactly on it is accepted; counters older than 168 hours stop counting.

## Client core conformance (`core/` crate, in this repository)

The Rust client core carries vectors for schema upgrades (an old reader on a newer object shows
it and refuses to write it), the keyring (wrap/unwrap round trips), the rebase loop (two devices
editing different fields of one object offline both keep their edits; the same field goes to the
later push; a tombstone returned during a retry ends the edit; an invalid rebase result is
shown, not pushed), notes
(two devices typing offline produce two `note_update` children that combine to the same text in
either order), the cascade (deleting a parent batches the children the device holds; a child of
a tombstoned parent that arrives later is dead and gets tombstoned), the
invite tag, and a simulated multi-device sync harness that drives random
push/pull/ack/offline interleavings, with eviction and a simulated holder serving history,
against an in-memory model of `relay`'s contract and asserts that every device ends with the
same rows. These
run in the same Bazel test target as the server suites; the wasm build of the crate is a build
target the web client depends on, so a core that does not compile to wasm fails CI. The server-side contract those vectors assume is this plan's `02-relay.md`.

## App shells (`ios/`, `android/`, behind `--//:apps`)

The shells hold no logic the core does not, so their tests are about the binding and the
rendering. Kotlin tests are `kt_jvm_test`s against the generated bindings with the host build of
the core on the JNA library path, so a Kotlin call that no longer matches the core's exported
interface fails on the Linux runner before any emulator exists. Swift tests are `ios_unit_test`s
on the simulator against the same bindings. Each shell has one end-to-end test that signs in
against a fake Kratos, pairs, pushes an object and sees it rendered, driven through the core's
in-memory relay model from the conformance harness. Both run only in `apps.yaml`
(`05-deployment.md`), never in the server's `//...`.

## Feature suites (every shell, the same scenarios)

Each shell carries the feature suite below, written once per platform (Kotlin `kt_jvm_test`,
Swift `ios_unit_test`, TypeScript vitest) against the shell's own client code over the core's
in-memory `ModelServer` (`core/AGENTS.md`), with memory-backed seams and nothing mocked in the
core. The scenarios are the same on every platform, numbered so a gap is visible: a feature the
core has and a shell does not test is a missing `Fnn`. A shell adds a scenario here before it
adds the test, so the three stay one suite. Device A is the first user's phone, A2 the same
user's second device, B another user's device; "sign in" is `modelServer.signIn(token, user)`,
`setSessionToken(token)`, `restoreSession`; "sync" is `sync()`; a step that needs the other
device to see something syncs it first.

**Session and device**
- F01 sign in: `restoreSession` yields the identity; without a token, none.
- F02 register: `registerDevice` twice is one device; `deviceId` and `fingerprint` are stable.
- F03 reopen: a second client over the same storage and secrets has the same device id, keys
  and rows, and `listObjects` matches before and after.
- F04 logout: after `logout`, `restoreSession` yields none and a write fails `not_signed_in`.
- F05 outage: with `setOutage(true)` a `sync` fails with kind `api` / code `unavailable`, the
  store is unchanged, the outbox keeps the write; `setOutage(false)` and the next sync lands it.

**Groups and membership**
- F06 create: `createGroup` makes the group, its profile singleton (`groupProfileId`), the
  creator an admin holding key 1; `listGroups` shows it with `holds_keys`.
- F07 join by invite: A `createInvite`, B `join(link)`; A's `pendingKeyRequests` shows B with
  `invite_verified` true and B's fingerprint; A approves; B syncs, `keys_received`, reads A's
  post.
- F08 roles: a viewer's `createObject` is refused with `forbidden` before anything enters the
  outbox (the core knows the caller's role from `listGroups`, `GroupView.role`); after
  `setMemberRole(member)` and a sync the same write lands.
- F09 removal and rotation: A `removeMember(B)`, `rotateKeys`; a post A writes after is not in
  B's `listObjects` after B syncs (B's sync reports the group gone or no key); A2 receives the
  new generation.
- F10 leave: B `leaveGroup`; `listGroups` on B lacks it; `listMembers` on A lacks B.
- F11 members: `listMembers` carries every member with role and joined time, in a stable order.
- F12 profile: two admins edit different fields of the group profile offline; both edits
  survive the sync (field-level rebase); the same field goes to the later push.

**A second device, history and the window**
- F13 own device: A2 signs in as A's user, registers, syncs; A's `pendingKeyRequests` shows it
  with `invite_verified` absent (no invite) and its fingerprint; A approves; A2 holds every row.
- F14 catch-up after eviction: A posts, every device syncs, `advance(window + 1h)` and `sweep`
  empty the server; B joins; A syncs (serves history); B becomes a holder with every post.
- F15 no wake-up: the same with no `historyReady` call at all: B's next sync completes.
- F16 joiners together: B and C join within the grace; one `sync` of A serves both
  (`SyncReport.history_served` is 1); both hold everything.
- F17 start over: B edits offline, `advance(window + 1h)`, `sweep`; B's sync reports
  `started_over`, then B catches up and its offline edit lands rebased (or is dropped with
  `EditDropped` when the object was deleted meanwhile).

**Object types** (one scenario per type, each: create, `readObject`, `listObjects` by type,
edit where the strategy allows, delete, and the other device sees each step)
- F20 post: text; another member's `editObject` is refused with `forbidden` (only the author's
  devices edit; others add children); a comment and a photo under it go with its delete
  (cascade).
- F21 comment: under a post, a photo, a venue; `parent_id` kept; deleted with the parent.
- F22 reaction: `createObject` derives the id UUIDv5(parent, user); a second `createObject` by
  the same user edits the same object (one reaction per user per target); two users' reactions
  are two objects.
- F23 photo: `uploadFile` of bytes under slot `full` (`hasBlob` true for the derived key),
  thumbnail bytes in the value, `downloadFile` round trip on the other device; the file is gone
  from the bucket once the photo's tombstone is pushed (`storage` on `ObjectDeleted`, which the
  in-memory server models).
- F24 album: title and cover; photos as children; cover change by another member is a rebase.
- F25 todo list and items: `pos` ordering; two devices tick and untick the same item offline,
  later push wins; a move (`pos` edit) never duplicates.
- F26 note: `saveNote` on two devices offline with different insertions, `readNote` on both
  gives the same text; a title edit is a rebase; the note's delete cascades to its updates.
- F27 expense pool, expense, settlement: the general pool singleton (`generalPoolId`, created
  again = edited); an expense with participants (`equal`, `weights`, `exact`), edited by two
  devices (one the amount, one a participant) with both kept; a settlement is append-only (edit
  refused with `forbidden`, delete allowed); a ticket kind expense.
- F28 ticket share: holder and status; a transfer edits holder; its file through `uploadFile`.
- F29 event: start and end; an edit that crosses them is reported `invalid` and not pushed;
  `yearly`.
- F30 poll and vote: a `choice` and a `dates` poll; `createObject` derives a vote's id
  UUIDv5(poll, user) and a second one edits it; an option edit by the author never fights a
  vote.
- F31 message: a thread (`reply_to`), photos as children, edit, delete.
- F32 venue: fields and a comment and reaction under it.
- F33 member profile: display name, birthday; the singleton id (`memberProfileId`); another
  member's edit is refused with `forbidden`.
- F34 invite secret: created by `createInvite`, readable by every holder; an edit is refused
  with `forbidden` (append-only).
- F35 unknown field: an object written with a newer schema version is shown and not rewritten.
  The core's conformance vectors cover this; a shell skips it until the in-memory server can
  hand out a newer version.

**Conflicts**
- F40 different fields: two devices edit different fields of one object offline; both kept.
- F41 same field: the later push wins; the loser's local view updates on its next sync.
- F42 edit vs delete: an edit against a tombstone is dropped and reported
  (`SyncReport.edits_dropped` names the object).
- F43 invalid rebase: a rebase result that fails validation is parked and reported
  (`invalid` in the `SyncReport`), never pushed; fixing the object clears it.

**Wake-ups (the shell's dispatcher, a unit test; the phones only, the browser gets no push)**
- F50 reasons `head`, `keys`, `rotate`, `history` call `sync`; `key_request` refreshes the
  pending list; `history_ready` calls `historyReady(requestId, throughSeq, complete)` then
  `sync`; an unknown reason is ignored; a malformed payload is ignored and logged.

**Screens** (where the shell has the screen; rendered against a client over `ModelServer`)
- F60 sign-in shows the one button and, signed in, the groups list.
- F61 groups: create shows the new group; join by link shows the group without keys and the
  waiting state.
- F62 approvals: a pending request shows the fingerprint and whether the invite vouches for
  it; approving removes it.
- F63 timeline: posts newest first; compose adds one; edit and delete on own posts only.
- F64 sync indicator: an outage shows the offline state and clears when the server is back.

## CI gate

The scaffold's workflow: lint → Bazel build and test (every unit and integration suite above,
the core conformance vectors, the wasm build; testcontainers runs on `ubuntu-latest`) → image push → render `k8s/` → upload. A deploy to preview
or live happens only after every suite is green. `.basable/verify.sh` runs the same locally.
The apps have their own gate, `apps.yaml` (round 23): the Android job on Linux and the iOS job
on macOS run the shell suites above with `--//:apps`, and `buf breaking` there fails the build,
since a shipped app cannot be redeployed. Its result never gates a server deployment.
