//! Schedules are WORKERS this nanoservice owns: a ticker in its `loops()`
//! (lib.rs), which the app starts and joins on shutdown — immediate first
//! tick then one per interval, failures logged and retried next tick.
//! Never a CronJob. A tick that needs claim authority creates or nudges a
//! processing object; it never writes status itself.

use basable_core::{BoxError, Ctx};

use crate::Storage;

impl Storage {
    /// `expire_usage`, every 1h. `_s` is this nanoservice's sender,
    /// for a tick that sends.
    pub async fn tick_expire_usage<R>(&self, _ctx: &Ctx, _s: interfaces::StorageSender<'static, R>) -> Result<(), BoxError>
    where
        R: interfaces::StorageRoutes + Send + Sync + 'static,
    {
        Err(crate::unimplemented_step("storage.tick_expire_usage").into())
    }
}
