# core

The client core: what every device runs identically, whatever its shell
(`docs/01-deappje/01-architecture.md`, "The shells"; `02-device.md` for
the keys; `02-relay.md` for the sync it will speak; `06-object-types.md`
for the values). Its own Cargo workspace and its own crate hub
(`core_crates` in `MODULE.bazel`): `core/Cargo.toml` says why it is apart
from the server's.

## Rules

1. **The core never reads a clock, draws randomness or holds a device's
   private key.** Those come through the seams in `src/shell.rs`
   (`Clock`, `Random`, `DeviceKey`); the core seeds its own generator once
   (`src/rng.rs`) and delegates the Diffie-Hellman step of HPKE to the
   device key. A test implements the seams in software, so every output is
   fixed by a seed.
2. **Every dependency builds for wasm32-unknown-unknown and the phone
   triples** (the hub is resolved for all of them) and draws no randomness
   of its own: no `getrandom`, no `rand` with an OS backend. The one
   exception is `yrs`'s `fastrand`, which the core never relies on (every
   `Doc` gets a client id from `CoreRng`); it reaches the browser's entropy
   through getrandom's `js` feature, so the wasm build still links.
3. **The server sees bytes.** An object's value leaves this crate only as
   an `Envelope` (`src/envelope.rs`); a group key leaves it only wrapped
   (`src/keyring.rs`); an invite secret leaves it only as a link fragment or
   a tag (`src/invite.rs`).
4. **A changed key under a known device id is refused and surfaced**
   (`src/trust.rs`), never adopted silently. A wrap is opened only under
   the pinned key of the device that made it, which the server hands over
   with the wrap (`wrapped_by_public_key`, since 2026-10-30): a device
   that holds nothing yet cannot list the holders to learn it.

## Files

| File | Responsibility |
|---|---|
| `src/shell.rs` | The seams: `Random`, `Clock`, `DeviceKey`, `Storage` (named blobs the core writes through); `SoftwareDeviceKey`, `MemoryStorage`, `FixedClock`, `SeededRandom` for tests and the browser without "remember this browser". |
| `src/rng.rs` | `CoreRng`: ChaCha20 seeded from the shell; `rand_core` impls for the crates that take a generator. |
| `src/wire.rs` | The protobuf wire format: encode, decode, fields by number. The envelope is written here, and the rebase will work at this level. |
| `src/crypto/` | `hash` (SHA-256, HMAC), `aead` (XChaCha20-Poly1305 for values and blobs; ChaCha20-Poly1305 for HPKE), `hpke` (RFC 9180 Auth mode, DHKEM X25519 / HKDF-SHA256 / ChaCha20-Poly1305, by hand so the device key stays behind its seam; tested against the `hpke` crate both ways). |
| `src/ids.rs` | Random ids from the generator; the UUIDv5 singletons (`group_profile`, `member_profile`, `general_pool`, `reaction`, `vote`). |
| `src/envelope.rs` | `Header { object_type, schema_version, key_id }`, `Envelope { header, nonce, ciphertext }`: seal and open under a group key with the group id, the object id and the header as associated data; `peek` reads the header without a key. |
| `src/keyring.rs` | `GroupKey`, `WrapIds`, `wrap`/`unwrap` (HPKE Auth, `info` = group ‖ key id ‖ recipient device ‖ wrapping device), `Keyring` (every generation per group, `latest`). |
| `src/trust.rs` | `fingerprint` (SHA-256 of the public key, twelve symbols `xxxx-xxxx-xxxx`), `Pins` (`check` refuses a changed key; `accept` is the person's act). |
| `src/invite.rs` | `InviteSecret` (16 bytes, the link's fragment in the code alphabet, the HMAC tag over the joining device's key), `join_link`, `parse_join_link`. |
| `src/schema.rs` | `ObjectType` (numbers, `Strategy`, `ParentRule`, `may_write`), `FieldKind`/`ElementKey`/`FieldSpec`/`Schema` for the rebase, `FieldValue`/`Scalar`/`Element`/`Message`/`Object` (typed fields over the wire codec, proto3 defaults absent, `Option` for presence), the `message!` and `wire_enum!` macros, `Meta` (fields 1-4: parent, author, authored_at, edited_at), `Document<T>` (seal/open/validate). |
| `src/types.rs` | Every object type as a `message!` body with `impl Object` (number, invariants): `FileRef`, `Geo`, `Birthday`, the enums; `ObjectType::schema()` and `validate_value()` dispatch by number. Own fields start at 10; all at schema version 1. |
| `src/rebase.rs` | `diff` (base → local, per singular field and per element by key), `apply` (onto current, unknown fields pass through), `rebase` (by strategy: singleton and append-only go whole). The conformance cases of `03-testing.md` are its tests. |
| `src/api.rs` | `Code` (the Connect answer codes) and `ApiError`: what every seam to the server answers with; the engines act on the code. |
| `src/relay.rs` | The relay's contract as values (`Entry`, `PushEntry`, `PushResult`, `ChangeSet`, `Head`, `HistoryRequest`) and the `Relay` seam (async; Connect for the shells, the model for tests). |
| `src/keys.rs` | The key exchange over the `Devices` seam (`device`'s contract): `KeyExchange::register`, `create_group_key` (generation 1 wrapped for the minting device), `request_keys` (with the invite's tag), `pending_requests` (fingerprint + whether the invite tag verifies), `approve` (one wrap per generation held, the requester pinned), `receive` (every wrap opened under the pinned wrapper's key: `Received`, `Unopenable`, `Pin` events), `rotate` (next generation for every holder; a lost race adopts). |
| `src/store.rs` | The local store: per group the rows (envelopes with the type and parent read out of the ones the device can open), the cursor, the `Phase` (New, CatchingUp, Holder), the outbox of `Pending { Op::Create / Edit / Delete }` with the sealed bytes and the base plaintext an edit was made from; written through to `Storage` and read back whole at `open`. |
| `src/sync.rs` | The engine `Syncer`: `create`/`edit`/`delete` (the cascade over the children held) and `read`; `sync_group` by phase (join: ack 0, the window, a history request; catch up: page and ack as far as `HistoryReady` says, complete = the whole range; holder: pull, ack, drop tombstones every device passed, then push the outbox with conflicts rebased up to three times, a tombstone dropping the edit, an invalid result parked for the user); a device below the floor or refused an ack starts over and keeps its outbox for the replay; `serve_history` for holders; `history_ready` for the wake-up. Its tests (`src/sync/tests.rs`) are the conformance scenarios and the randomised multi-device harness (four devices, outages, eviction, two absences longer than the window), forty seeds. |
| `src/notes.rs` | `NoteDoc` over `yrs` (53-bit client id from the generator, byte offsets): `apply` a child's update, `set_text` (only the changed span is deleted and inserted), `session_update` (the delta since the session began), `end_session`; `Syncer::open_note` (every live `note_update` child applied, any order) and `save_note_session` (one child per session). |
| `src/proto.rs` | The API's messages (`proto/*/v1`) as `message!` structs with the protos' numbers: what the core sends and reads over Connect, no generated code. |
| `src/transport.rs` | The `Transport` seam: one HTTP request, bytes in and out (`fetch`, `URLSession`, `OkHttp`; a recorder in tests). |
| `src/connect.rs` | `Connect<T>`: unary POSTs to `/api/<package>.<Service>/<Method>` as `application/proto` with `connect-protocol-version: 1` and the session in `x-session-token`; the protocol's JSON error mapped to `Code` (the status when there is none); `Relay` and `Devices` implemented over it; the group and storage services as methods (`Group`, `Membership`, `Invite`, `Presigned`). |
| `src/kratos.rs` | The native sign-in over Kratos's API flows under `/.ory/`: `start_login` (a flow with a session token exchange code, the `basable` OpenID method, the browser URL from the 422), `finish_login` (the exchange for the session token), `whoami`, `logout`. |
| `src/harness/` | `RelayModel` (`relay`'s handlers and sweeps rule for rule in memory with a movable clock, membership and the wake-ups `device` would push) and `DevicesModel` (`device`'s key handlers: registration, requests, fulfilment, publishing with the one-publisher rule, a hostile `swap_public_key` for the pin tests); `block_on` for futures that never wait. Public so the shells' tests can drive them. |

## Builds

- `//core:core` and `//core:core_test`: the host library and its tests, in
  the scaffold's `bazel test //...`.
- `//core:core_wasm`: the cdylib for wasm32 with wasm-bindgen's glue, for
  the web client; it builds in CI so a dependency without a wasm build
  fails there.
- The Android and iOS libraries and the UniFFI bindings come with the apps
  (`docs/01-deappje/05-deployment.md`, "The apps").

## Adding a field or a type

A field: add it to the `message!` body with the next number, bump that
type's `SCHEMA_VERSION`, never reuse a number. A type: a new number in its
feature block in `object_types!`, a body in `types.rs`, both dispatch
matches. A breaking change is a new type number, never a repurposed field.

## Invariants the engine keeps

- A row's value is exactly what the server has at that seq: a device's own
  write is sealed once (in the op) and the row takes those bytes on accept.
- A push is preceded by a pull in the same sync, and a device's own rows
  that are the whole stretch after its cursor are acked without a pull.
- The outbox is never thrown away: not by a start-over, not by an outage;
  only an accept, a tombstone (the edit is dropped and said so) or a
  replay onto a gone object removes an entry.
- The store is consistent between any two relay calls: an outage returns
  `SyncError::Relay(Unavailable)` and the next sync continues.

## Still to come

The blob encryption and upload through the presigned URLs, the `Core`
facade that holds the pieces together for a shell, and its UniFFI and
wasm-bindgen exports.
