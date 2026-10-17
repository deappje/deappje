# Testing

## Unit tests (per nanoservice crate)

- `group`: invite acceptance rules, subgroup subset rule, last-admin rule.
- `device`: rotation batch acceptance, member checks on wrap.
- `ledger`: the CAS matrix, cascade collection, `reconcile_required` boundary, response
  shaping for pull/backfill.
- `storage`: the reconcile pass as a pure function of `(spec, status, bucket snapshot, now)` →
  `Outcome`, table-driven over every phase.

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
- **Delete semantics**: tombstone cascade; delete wins over a concurrent edit; gc respects the
  window; a stale cursor gets `reconcile_required`.
- **Key lifecycle**: register two devices, request keys, fulfil, remove member, rotation batch,
  `GetWrappedKeys` returns both key ids for a surviving device and nothing for the removed one.
- **Blob lifecycle**: against the bucket simulator, the full state machine including expiry and
  re-request, on two worker replicas.
- **Cross-nanoservice fencing**: `ObjectDeleted` arriving for a blob already deleting is a no-op;
  `GroupHeadAdvanced` fan-out excludes the writer device.
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

The Rust client core carries vectors for the key-schema upgrade steps (an old writer and a new
reader on one document, both directions), the keyring (wrap/unwrap round trips), the merge loop
(two documents edited offline, pushed in both orders, end identical and keep both edits; a
delete during a merge retry wins; an invariant broken by a merge is flagged, not rewritten;
a concurrent reorder of a list never duplicates an item), and a simulated multi-client sync
harness that drives random push/pull/offline interleavings against an in-memory model of
`ledger`'s contract and asserts that every client's document for every object converges. These
run in the same Bazel test target as the server suites; the wasm build of the crate is a build
target the web client depends on, so a core that does not compile to wasm fails CI. The server-side contract those vectors assume is this plan's `02-ledger.md`.

## CI gate

The scaffold's workflow: lint → Bazel build and test (every unit and integration suite above,
the core conformance vectors, the wasm build; testcontainers runs on `ubuntu-latest`) → image push → render `k8s/` → upload. A deploy to preview
or live happens only after every suite is green. `.basable/verify.sh` runs the same locally.
