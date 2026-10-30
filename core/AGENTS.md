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
   of its own: no `getrandom`, no `rand` with an OS backend.
3. **The server sees bytes.** An object's value leaves this crate only as
   an `Envelope` (`src/envelope.rs`); a group key leaves it only wrapped
   (`src/keyring.rs`); an invite secret leaves it only as a link fragment or
   a tag (`src/invite.rs`).
4. **A changed key under a known device id is refused and surfaced**
   (`src/trust.rs`), never adopted silently.

## Files

| File | Responsibility |
|---|---|
| `src/shell.rs` | The seams: `Random`, `Clock`, `DeviceKey`; `SoftwareDeviceKey` for tests and the browser without "remember this browser". |
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

## Still to come

The local store, the sync engine against `relay`'s contract with the
outbox and the multi-device harness, the note CRDT, the Connect and
Kratos clients behind a transport seam, the UniFFI exports.
