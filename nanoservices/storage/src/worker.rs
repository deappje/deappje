//! Schedules are WORKERS this nanoservice owns: a ticker in its `loops()`
//! (lib.rs), which the app starts and joins on shutdown — immediate first
//! tick then one per interval, failures logged and retried next tick.
//! Never a CronJob.

use basable_core::{BoxError, Ctx};
use chrono::Utc;

use crate::Storage;
use crate::handlers::USAGE_WINDOW;
use crate::repository as repo;

impl Storage {
    /// `expire_usage`, every 1h: the upload counters older than the window
    /// stop counting.
    pub async fn tick_expire_usage<R>(&self, _ctx: &Ctx, _s: interfaces::StorageSender<'static, R>) -> Result<(), BoxError>
    where
        R: interfaces::StorageRoutes + Send + Sync + 'static,
    {
        let expired = repo::delete_usage_before(&*self.pool, Utc::now() - USAGE_WINDOW).await?;
        if expired > 0 {
            tracing::info!(expired, "expired upload counters");
        }
        Ok(())
    }
}
