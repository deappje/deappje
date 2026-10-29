//! `group`: Owns groups, memberships and invites; answers every membership check; drives group deletion and account deletion.
//!
//! Read `AGENTS.md` in this directory and `docs/DIRECTIVE.md` before editing.
//! What this nanoservice owns is what it is: 2 processing-object
//! type(s), 3 plain table(s), 1 external call(s).

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
pub mod types;

/// The schema marker: this nanoservice owns `nano_group` and gets a
/// pool bound to it (`SET ROLE nano_group`). A cross-schema query
/// from that pool is a Postgres permission error.
pub struct Schema;
impl Nanoservice for Schema {
    const NAME: &'static str = "group";
}
impl Stateful for Schema {}

/// The component. One value per process, shared by every handler and worker
/// (`&self` everywhere; in-memory state, if any, behind `std::sync::Mutex`
/// or atomics and never held across an `.await`).
pub struct Group {
    pub(crate) pool: NanoPool<Schema>,
    pub(crate) calls: effects::Calls,
    pub(crate) user_purge_store: types::user_purge::Store,
    pub(crate) group_purge_store: types::group_purge::Store,
}

impl Group {
    /// Builds the component. Called once from `app/src/main.rs`.
    /// Binding a type's store checks its migration ran; a missing one is a
    /// boot failure, which is the right time to find out.
    pub async fn new(_app: &basable_app::App, pool: NanoPool<Schema>) -> Self {
        Self {
            pool: pool.clone(),
            // Production wires the real provider; tests wire the simulator.
            calls: effects::Calls::from_env(),
            user_purge_store: types::user_purge::Store::bind(&pool, types::user_purge::decl())
                .await
                .expect("group: bind type user_purge (the migration must have run)"),
            group_purge_store: types::group_purge::Store::bind(&pool, types::group_purge::decl())
                .await
                .expect("group: bind type group_purge (the migration must have run)"),
        }
    }
}

impl<R> basable_app::Component<R> for Group
where
    R: interfaces::GroupRoutes + Send + Sync + 'static,
{
    /// Every loop this nanoservice runs: a worker per processing-object
    /// type, then its schedules. The app starts every component's loops
    /// (`app/src/main.rs`: `.components(router)`) and joins them on
    /// shutdown, so a type or a schedule this nanoservice gains is added
    /// here and nowhere else.
    fn loops(&'static self, router: &'static R) -> basable_app::Loops {
        basable_app::Loops::new()
            .worker(types::user_purge::worker(router, self))
            .worker(types::group_purge::worker(router, self))
    }
}

/// The error a step not filled in yet answers with, so the skeleton deploys
/// green instead of panicking. `regex_search unimplemented_step` lists what
/// is left.
pub(crate) fn unimplemented_step(step: &'static str) -> AppError {
    tracing::warn!(step, "unimplemented step reached");
    AppError::unimplemented(step)
}
