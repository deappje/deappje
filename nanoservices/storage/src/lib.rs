//! `storage`: Presigns uploads and downloads under the caller's group prefix, deletes an object's files on its tombstone and a group's on purge, bounds upload bytes per group.
//!
//! Read `AGENTS.md` in this directory and `docs/DIRECTIVE.md` before editing.
//! What this nanoservice owns is what it is: 0 processing-object
//! type(s), 1 plain table(s), 1 external call(s).

// A rendered skeleton has stubs nothing calls yet; the allow goes once the
// bodies are filled.
#![allow(dead_code)]

use basable_core::AppError;
use basable_db::{NanoPool, Nanoservice, Stateful};

pub mod handlers;
pub mod effects;
pub mod provider;
pub mod simulator;
pub mod model;
pub mod repository;
pub mod worker;
pub mod config;

/// The schema marker: this nanoservice owns `nano_storage` and gets a
/// pool bound to it (`SET ROLE nano_storage`). A cross-schema query
/// from that pool is a Postgres permission error.
pub struct Schema;
impl Nanoservice for Schema {
    const NAME: &'static str = "storage";
}
impl Stateful for Schema {}

/// The component. One value per process, shared by every handler and worker
/// (`&self` everywhere; in-memory state, if any, behind `std::sync::Mutex`
/// or atomics and never held across an `.await`).
pub struct Storage {
    pub(crate) pool: NanoPool<Schema>,
    pub(crate) calls: effects::Calls,
}

impl Storage {
    /// Builds the component. Called once from `app/src/main.rs`.
    pub fn new(_app: &basable_app::App, pool: NanoPool<Schema>) -> Self {
        Self {
            pool,
            // Production wires the real provider; tests wire the simulator.
            calls: effects::Calls::from_env(),
        }
    }
}

impl<R> basable_app::Component<R> for Storage
where
    R: interfaces::StorageRoutes + Send + Sync + 'static,
{
    /// Every loop this nanoservice runs: a worker per processing-object
    /// type, then its schedules. The app starts every component's loops
    /// (`app/src/main.rs`: `.components(router)`) and joins them on
    /// shutdown, so a type or a schedule this nanoservice gains is added
    /// here and nowhere else.
    fn loops(&'static self, router: &'static R) -> basable_app::Loops {
        let sender = interfaces::StorageSender::new(router);
        basable_app::Loops::new()
            .ticker(basable_app::Ticker::new("expire_usage", std::time::Duration::from_secs(3600), move |ctx| Box::pin(self.tick_expire_usage(ctx, sender))))
    }
}

/// The error a step not filled in yet answers with, so the skeleton deploys
/// green instead of panicking. `regex_search unimplemented_step` lists what
/// is left.
pub(crate) fn unimplemented_step(step: &'static str) -> AppError {
    tracing::warn!(step, "unimplemented step reached");
    AppError::unimplemented(step)
}
