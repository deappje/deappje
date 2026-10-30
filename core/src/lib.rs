//! deappje's client core: what every device runs identically, whatever its
//! shell (`docs/01-deappje/01-architecture.md`, "The shells"). The core
//! owns the keyring and every cryptographic operation, the object envelope,
//! ids, the sync engine, the local store, the conflict rebase, the note
//! CRDT and the type schemas; a shell renders its state and does what only
//! the platform can, through the seams in [`shell`].
//!
//! Nothing here reads a clock, draws randomness or holds a device's private
//! key: those come from the shell, which is what makes the core
//! deterministic under test and buildable for the browser and both phones
//! from one source.
//!
//! Layers, bottom up: [`wire`] (protobuf wire format), [`crypto`] (HPKE,
//! AEAD, hashes), [`ids`], [`envelope`], [`keyring`], [`trust`], [`invite`],
//! [`schema`] (typed fields, documents), [`types`] (the object types),
//! [`rebase`] (the conflict rebase), [`relay`] (the relay's contract),
//! [`store`] (the local store and the outbox), [`sync`] (the engine),
//! [`harness`] (the in-memory relay model the tests drive).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod crypto;
pub mod envelope;
pub mod harness;
pub mod ids;
pub mod invite;
pub mod keyring;
pub mod rebase;
pub mod relay;
pub mod rng;
pub mod schema;
pub mod shell;
pub mod store;
pub mod sync;
pub mod trust;
pub mod types;
pub mod wire;
