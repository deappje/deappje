# Testing

## Unit tests (per nanoservice crate)

- `group`: invite acceptance rules, subgroup subset rule, last-admin rule.
- `device`: rotation batch acceptance, member checks on wrap.
- `ledger`: the CAS-on-seq matrix, eviction floor arithmetic, response shaping for pull.
- `storage`: key parsing, quota and slot cap at the boundary.

## `effecttest` audits (one per adapter, in each nanoservice's `tests/effects_audit.rs`)

| Nanoservice | Adapter | Simulator models |
|---|---|---|
| device | apns.send_push | accepted, rejected token (410), transport failure, ack loss |
| device | fcm.send_push | accepted, `UNREGISTERED`, transport failure, ack loss |
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
  168 hours and leaves one that is catching up; a backwards ack is refused and 0 resets the
  row; an edit evicted while a device was offline is never replaced silently; a batch of
  creates from a non-holder is refused like any other, so a new member posts only after its
  history has arrived and its ack has reached the floor; a device that falls behind starts over
  and ends with no object that was deleted while it was away.
- **History**: a joining device acks 0, pulls the window, asks for everything up to the floor,
  pages and acks the restored rows as they land, and acks the head; its ack 0 pins the minimum
  so nothing is evicted by ack from under it, and the restored rows go as it acks past them;
  two simulated holders claim and exactly one wins; it pushes the rows back oldest first,
  tombstones included; they land at their original seqs, the head does not move, an id with a
  current row is skipped, a push without the claim is refused, a claim from a non-holder is
  refused; a request from a group with no other holder is answered `unfillable` without a row;
  `complete` fulfils the request once and a later push for it is refused; a claimer that stops
  is taken over after 5 minutes and the take-over re-push restores only what is missing; the
  joiner ends with the same rows as the holder; a restored row the requester never acks is
  evicted 168 hours after the re-insert and no earlier.
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
against an in-memory model of `ledger`'s contract and asserts that every device ends with the
same rows. These
run in the same Bazel test target as the server suites; the wasm build of the crate is a build
target the web client depends on, so a core that does not compile to wasm fails CI. The server-side contract those vectors assume is this plan's `02-ledger.md`.

## CI gate

The scaffold's workflow: lint → Bazel build and test (every unit and integration suite above,
the core conformance vectors, the wasm build; testcontainers runs on `ubuntu-latest`) → image push → render `k8s/` → upload. A deploy to preview
or live happens only after every suite is green. `.basable/verify.sh` runs the same locally.
