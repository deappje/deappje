//! `device`: Owns devices, push tokens, wrapped group keys and key requests; sends silent pushes; tells relay when a device leaves a group.
//!
//! Read `AGENTS.md` in this directory and `docs/DIRECTIVE.md` before editing.
//! What this nanoservice owns is what it is: 0 processing-object
//! type(s), 3 plain table(s), 2 external call(s).

use basable_db::{NanoPool, Nanoservice, Stateful};

pub mod effects;
pub mod handlers;
pub mod model;
pub mod provider;
pub mod push;
mod repository;
pub mod simulator;

/// The schema marker: this nanoservice owns `nano_device` and gets a
/// pool bound to it (`SET ROLE nano_device`). A cross-schema query
/// from that pool is a Postgres permission error.
pub struct Schema;
impl Nanoservice for Schema {
    const NAME: &'static str = "device";
}
impl Stateful for Schema {}

/// The component. One value per process, shared by every handler and worker
/// (`&self` everywhere; in-memory state, if any, behind `std::sync::Mutex`
/// or atomics and never held across an `.await`).
pub struct Device {
    pub(crate) pool: NanoPool<Schema>,
    pub(crate) calls: effects::Calls,
}

impl Device {
    /// Builds the component. Called once from `app/src/main.rs`.
    pub fn new(_app: &basable_app::App, pool: NanoPool<Schema>) -> Self {
        // Production wires the real providers; tests wire the simulator.
        Self::with_calls(pool, effects::Calls::from_env())
    }

    /// The component over an explicit adapter set: what the tests build,
    /// with `effects::Calls::new(simulator)`.
    pub fn with_calls(pool: NanoPool<Schema>, calls: effects::Calls) -> Self {
        Self { pool, calls }
    }
}

/// This nanoservice runs no loop of its own (no processing-object type, no
/// schedule): it answers messages. A type or a schedule it gains brings a
/// `loops()` here.
impl<R: 'static> basable_app::Component<R> for Device {}
