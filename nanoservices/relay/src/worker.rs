//! Schedules are WORKERS this nanoservice owns: a ticker in its `loops()`
//! (lib.rs), which the app starts and joins on shutdown — immediate first
//! tick then one per interval, failures logged and retried next tick.
//! Never a CronJob. Both sweeps are idempotent; each group's floor moves
//! under its counter lock, like every write.

use basable_core::{BoxError, Ctx};
use chrono::Utc;
use std::collections::BTreeMap;

use crate::Relay;
use crate::handlers::window_cutoff;
use crate::repository as repo;

/// Rows evicted by age per statement.
const AGE_BATCH: i64 = 10_000;

impl Relay {
    /// `evict_objects`, every 1h. By ack: per group, the rows every device
    /// has acked go and the floor follows the minimum. By age: rows older
    /// than the window go, batched, the floors follow the highest seq
    /// taken per group, and the devices below the new floor that have not
    /// acked within the window lose their row (they start over).
    pub async fn tick_evict_objects<R>(&self, ctx: &Ctx, _s: interfaces::RelaySender<'static, R>) -> Result<(), BoxError>
    where
        R: interfaces::RelayRoutes + Send + Sync + 'static,
    {
        self.evict_by_ack(ctx).await?;
        self.evict_by_age(ctx).await?;
        Ok(())
    }

    pub(crate) async fn evict_by_ack(&self, ctx: &Ctx) -> Result<(), BoxError> {
        for (group_id, min) in repo::group_minimums(&*self.pool).await? {
            if ctx.is_done() {
                return Ok(());
            }
            let mut tx = self.pool.begin().await?;
            let head = repo::lock_head(&mut tx, group_id).await?;
            let evicted = repo::delete_objects_through(&mut *tx, group_id, min).await?;
            if min > head.evicted_through_seq {
                repo::raise_floor(&mut *tx, group_id, min).await?;
            }
            tx.commit().await?;
            if evicted > 0 {
                tracing::info!(group = %group_id, evicted, floor = min.max(head.evicted_through_seq), "evicted by ack");
            }
        }
        Ok(())
    }

    pub(crate) async fn evict_by_age(&self, ctx: &Ctx) -> Result<(), BoxError> {
        let cutoff = window_cutoff(Utc::now());
        loop {
            if ctx.is_done() {
                return Ok(());
            }
            let gone = repo::delete_objects_older_than(&*self.pool, cutoff, AGE_BATCH).await?;
            let mut highest: BTreeMap<uuid::Uuid, i64> = BTreeMap::new();
            for (group_id, seq) in &gone {
                let h = highest.entry(*group_id).or_insert(*seq);
                *h = (*h).max(*seq);
            }
            for (group_id, seq) in highest {
                let mut tx = self.pool.begin().await?;
                repo::lock_head(&mut tx, group_id).await?;
                let floor = repo::raise_floor(&mut *tx, group_id, seq).await?;
                let stale = repo::delete_stale_devices(&mut *tx, group_id, floor, cutoff).await?;
                tx.commit().await?;
                tracing::info!(group = %group_id, floor, stale_devices = stale, "evicted by age");
            }
            if (gone.len() as i64) < AGE_BATCH {
                return Ok(());
            }
        }
    }

    /// `expire_history_requests`, every 1h: requests older than the window
    /// are dropped, answered or not.
    pub async fn tick_expire_history_requests<R>(&self, _ctx: &Ctx, _s: interfaces::RelaySender<'static, R>) -> Result<(), BoxError>
    where
        R: interfaces::RelayRoutes + Send + Sync + 'static,
    {
        let expired = repo::delete_history_requests_older_than(&*self.pool, window_cutoff(Utc::now())).await?;
        if expired > 0 {
            tracing::info!(expired, "expired history requests");
        }
        Ok(())
    }
}
