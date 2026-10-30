# Agent tasks

The ordered list of what is left after the scaffold. Each nanoservice's
items are in `docs/nanoservices/<name>.md`; this file is the whole-project
order. Tick items off as they land.

1. Read `AGENTS.md`, `docs/DIRECTIVE.md`, `docs/architecture.md`.
2. ~~Work through `docs/nanoservices/group.md` for `group`.~~ Done 2026-10-30.
3. ~~Work through `docs/nanoservices/device.md` for `device`.~~ Done 2026-10-30.
4. ~~Work through `docs/nanoservices/relay.md` for `relay`.~~ Done 2026-10-30.
5. ~~Work through `docs/nanoservices/storage.md` for `storage`.~~ Done 2026-10-30.
6. ~~Declare every message struct in `crates/messages/src/lib.rs` (the build names the missing ones).~~ Done 2026-10-30.
7. ~~Fill the proto messages under `proto/` and the field mapping in `api/src/services/`.~~ Done 2026-10-30.
8. Un-ignore the `#[ignore = "implement first"]` tests as each piece lands; keep `tests/effects_audit.rs` green.
9. ~~`regex_search unimplemented_step` — only the definitions left (one per crate) means the skeleton is filled.~~ Done 2026-10-30: none left.

The client core and the apps (`docs/01-deappje/05-deployment.md`, "The apps"), after the server:

10. ~~The toolchain check: the app modules loaded, `//...` green in the CI container.~~ Done 2026-10-30.
11. ~~`core/`, bottom up (`core/AGENTS.md`): the seams, the generator, the wire codec, the crypto, the envelope, the keyring, trust, the invite secret, the wasm build; the type schemas and the rebase, the local store and the outbox, the sync engine against `relay`'s contract with the multi-device harness, the note CRDT, the key exchange, the Connect and Kratos clients behind a transport seam, the `Core` facade, the UniFFI bindings and the in-memory server for the shells' tests.~~ Done 2026-10-30. Left in the core: the wasm-bindgen glue for the web client, the timeline projection, the calendar and balance derivations, export.
12. `android/` and `ios/` behind `--//:apps`, with `apps.yaml`.
