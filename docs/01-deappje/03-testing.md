# Testing

## Unit tests (per nanoservice crate)

- `group`: invite acceptance rules, subgroup subset rule, last-admin rule.
- `device`: rotation batch acceptance, member checks on wrap.
- `ledger`: the CAS matrix, the stale-cursor rule, cascade collection, eviction floor
  arithmetic, response shaping for pull.
- `storage`: quota and size cap at the boundary, expiry arithmetic, coverage ⇒ delete.

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
- **Sequence integrity**: N tasks push to one group concurrently; the resulting `seq` set is
  `1..=N` with no gaps; a reader polling during the run never observes a seq it later misses.
- **Delete semantics**: tombstone cascade; delete wins over a concurrent edit; a push from a
  cursor below the last delete is refused and succeeds after a pull; the **ack invariant** under
  random interleavings of push, delete, pull, ack and evict across N devices (every tombstone the
  gc removed was at or below every device's acked cursor, and no device ever holds a live copy of
  an object whose tombstone it has acked past); an ack that goes backwards gets
  `resync_required`.
- **History**: a device with a cursor below the floor requests the gap; a simulated holder
  uploads a bundle; `BundleReady` fulfils the request once; a late bundle is ignored; the
  requester ends with the same rows as the holder.
- **Key lifecycle**: register two devices, request keys, fulfil, remove member, rotation batch,
  `GetWrappedKeys` returns both key ids for a surviving device and nothing for the removed one.
- **Blob lifecycle**: against the bucket simulator: upload, download, expiry sweep, re-request,
  re-upload to the same key, early delete on full coverage; a failed delete is retried by the
  next sweep.
- **Cross-nanoservice**: `ObjectDeleted` for a blob already gone is a no-op; `GroupHeadAdvanced`
  fan-out excludes the writer device; `DeviceLeftGroup` unblocks a group's gc.
- **Group purge**: a parent with a subgroup, objects, blobs and keys; after deletion every table
  in every nanoservice holds nothing for either group, the bucket simulator holds no keys, the
  purge objects are gone, and a member check answers "not a member" from the first commit on.
  A `PurgeResult` dropped once is retried and the end state is the same.
- **Account purge**: sole admin of one group, sole member of another, member of a third; ends with
  a promoted admin, a group purge, a rotation push, no device rows, one identity deletion.
- **Device trust**: a device row inserted directly into the table (simulating a hostile server)
  appears in no `ListDevices` answer and receives no wrapped key through a rotation.
- **Holders and quota**: after a download, `BlobRequested` names the downloader; an upload that
  would cross the group cap is refused and one that lands exactly on it is accepted.

## Client core conformance (`core/` crate, in this repository)

The Rust client core carries vectors for schema upgrades (an old reader on a newer object shows
it and refuses to write it), the keyring (wrap/unwrap round trips), the rebase loop (two devices
editing different fields of one object offline both keep their edits; the same field goes to the
later push; a delete during a retry wins; an invalid rebase result is shown, not pushed), notes
(two devices typing offline produce two `note_update` children that combine to the same text in
either order), the invite tag, and a simulated multi-device sync harness that drives random
push/pull/ack/offline interleavings, with eviction and a simulated holder serving history,
against an in-memory model of `ledger`'s contract and asserts that every device ends with the
same rows. These
run in the same Bazel test target as the server suites; the wasm build of the crate is a build
target the web client depends on, so a core that does not compile to wasm fails CI. The server-side contract those vectors assume is this plan's `02-ledger.md`.

## CI gate

The scaffold's workflow: lint → Bazel build and test (every unit and integration suite above,
the core conformance vectors, the wasm build; testcontainers runs on `ubuntu-latest`) → image push → render `k8s/` → upload. A deploy to preview
or live happens only after every suite is green. `.basable/verify.sh` runs the same locally.
