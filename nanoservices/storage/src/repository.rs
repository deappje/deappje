//! Typed sqlx queries over this nanoservice's own table (schema
//! `nano_storage`): the upload counter per group per hour. Only this crate
//! queries it (the Directive §2). Errors are `sqlx::Error` as they come.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

/// Adds `bytes` to the group's counter for `hour`, creating the row; the
/// row's lock serializes the quota check of concurrent presigns in the
/// same hour. Answers the hour's total after.
pub(crate) async fn add_usage<'e>(e: impl PgExecutor<'e>, group_id: Uuid, hour: DateTime<Utc>, bytes: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO upload_usage (group_id, hour, bytes) VALUES ($1, $2, $3) \
         ON CONFLICT (group_id, hour) DO UPDATE SET bytes = upload_usage.bytes + EXCLUDED.bytes, updated_at = clock_timestamp() \
         RETURNING bytes",
    )
    .bind(group_id)
    .bind(hour)
    .bind(bytes)
    .fetch_one(e)
    .await
}

/// The bytes the group declared since `since`.
pub(crate) async fn usage_since<'e>(e: impl PgExecutor<'e>, group_id: Uuid, since: DateTime<Utc>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(bytes), 0)::bigint FROM upload_usage WHERE group_id = $1 AND hour > $2")
        .bind(group_id)
        .bind(since)
        .fetch_one(e)
        .await
}

/// The sweep: counters older than the window.
pub(crate) async fn delete_usage_before<'e>(e: impl PgExecutor<'e>, cutoff: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM upload_usage WHERE hour <= $1").bind(cutoff).execute(e).await?;
    Ok(done.rows_affected())
}
