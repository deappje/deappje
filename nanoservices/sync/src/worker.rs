//! Schedules are WORKERS this nanoservice owns: a ticker in its `loops()`
//! (lib.rs), which the app starts and joins on shutdown — immediate first
//! tick then one per interval, failures logged and retried next tick.
//! Never a CronJob. A tick that needs claim authority creates or nudges a
//! processing object; it never writes status itself.

use basable_core::{BoxError, Ctx};

use crate::Sync;

impl Sync {
    /// `evict_objects`, every 1h. `_s` is this nanoservice's sender,
    /// for a tick that sends.
    pub async fn tick_evict_objects<R>(&self, _ctx: &Ctx, _s: interfaces::SyncSender<'static, R>) -> Result<(), BoxError>
    where
        R: interfaces::SyncRoutes + Send + Sync + 'static,
    {
        Err(crate::unimplemented_step("sync.tick_evict_objects").into())
    }
    /// `expire_history_requests`, every 1h. `_s` is this nanoservice's sender,
    /// for a tick that sends.
    pub async fn tick_expire_history_requests<R>(&self, _ctx: &Ctx, _s: interfaces::SyncSender<'static, R>) -> Result<(), BoxError>
    where
        R: interfaces::SyncRoutes + Send + Sync + 'static,
    {
        Err(crate::unimplemented_step("sync.tick_expire_history_requests").into())
    }
}
