//! `relay`: The relay — ciphertext per group until every holder has acked it, 672 hours at most, tombstones included; the counter; per-device acked cursor; history requests.
//!
//! Read `AGENTS.md` in this directory and `docs/DIRECTIVE.md` before editing.
//! What this nanoservice owns is what it is: 0 processing-object
//! type(s), 4 plain table(s), 0 external call(s).

use basable_db::{NanoPool, Nanoservice, Stateful};

pub mod handlers;
pub mod model;
mod repository;
pub mod worker;

/// The schema marker: this nanoservice owns `nano_relay` and gets a
/// pool bound to it (`SET ROLE nano_relay`). A cross-schema query
/// from that pool is a Postgres permission error.
pub struct Schema;
impl Nanoservice for Schema {
    const NAME: &'static str = "relay";
}
impl Stateful for Schema {}

/// The component. One value per process, shared by every handler and worker
/// (`&self` everywhere; in-memory state, if any, behind `std::sync::Mutex`
/// or atomics and never held across an `.await`).
pub struct Relay {
    pub(crate) pool: NanoPool<Schema>,
}

impl Relay {
    /// Builds the component. Called once from `app/src/main.rs`.
    pub fn new(_app: &basable_app::App, pool: NanoPool<Schema>) -> Self {
        Self::with_pool(pool)
    }

    /// The component over a pool: what the tests build.
    pub fn with_pool(pool: NanoPool<Schema>) -> Self {
        Self { pool }
    }
}

impl<R> basable_app::Component<R> for Relay
where
    R: interfaces::RelayRoutes + Send + Sync + 'static,
{
    /// Every loop this nanoservice runs: a worker per processing-object
    /// type, then its schedules. The app starts every component's loops
    /// (`app/src/main.rs`: `.components(router)`) and joins them on
    /// shutdown, so a type or a schedule this nanoservice gains is added
    /// here and nowhere else.
    fn loops(&'static self, router: &'static R) -> basable_app::Loops {
        let sender = interfaces::RelaySender::new(router);
        basable_app::Loops::new()
            .ticker(basable_app::Ticker::new("evict_objects", std::time::Duration::from_secs(3600), move |ctx| Box::pin(self.tick_evict_objects(ctx, sender))))
            .ticker(basable_app::Ticker::new("expire_history_requests", std::time::Duration::from_secs(3600), move |ctx| Box::pin(self.tick_expire_history_requests(ctx, sender))))
    }
}
