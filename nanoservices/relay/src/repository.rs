//! Typed sqlx queries over this nanoservice's own tables (schema
//! `nano_relay`). Only this crate queries them; another nanoservice that
//! needs the data sends a message (the Directive §2). Every query takes the
//! executor it runs on; the handlers compose the counter-locked
//! transactions from these pieces. Errors are `sqlx::Error` as they come.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::model;

const OBJECT_COLUMNS: &str = "id, group_id, value, seq, deleted, uploaded_at";
const REQUEST_COLUMNS: &str = "id, group_id, requester_device, from_seq, to_seq, object_id, claimed_by, claimed_at, through_seq, completed_at, created_at";
const HEAD_COLUMNS: &str = "group_id, head_seq, evicted_through_seq, restored_through_seq, restored_at";

// --- group_head: the counter row ---

/// The group's counter row under `FOR UPDATE`, created with zeros on first
/// sight. Every write to a group's log and every move of its floor takes
/// this lock first (the Directive §7).
pub(crate) async fn lock_head(tx: &mut sqlx::PgConnection, group_id: Uuid) -> Result<model::GroupHead, sqlx::Error> {
    sqlx::query("INSERT INTO group_head (group_id, head_seq, evicted_through_seq, restored_through_seq) VALUES ($1, 0, 0, 0) ON CONFLICT DO NOTHING")
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query_as::<_, model::GroupHead>(&format!("SELECT {HEAD_COLUMNS} FROM group_head WHERE group_id = $1 FOR UPDATE"))
        .bind(group_id)
        .fetch_one(&mut *tx)
        .await
}

pub(crate) async fn get_head<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<Option<model::GroupHead>, sqlx::Error> {
    sqlx::query_as::<_, model::GroupHead>(&format!("SELECT {HEAD_COLUMNS} FROM group_head WHERE group_id = $1"))
        .bind(group_id)
        .fetch_optional(e)
        .await
}

pub(crate) async fn set_head_seq<'e>(e: impl PgExecutor<'e>, group_id: Uuid, head_seq: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE group_head SET head_seq = $2, updated_at = clock_timestamp() WHERE group_id = $1")
        .bind(group_id)
        .bind(head_seq)
        .execute(e)
        .await?;
    Ok(())
}

/// Raises the eviction floor to `to` unless it is already higher; answers
/// the floor after.
pub(crate) async fn raise_floor<'e>(e: impl PgExecutor<'e>, group_id: Uuid, to: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE group_head SET evicted_through_seq = GREATEST(evicted_through_seq, $2), updated_at = clock_timestamp() \
         WHERE group_id = $1 RETURNING evicted_through_seq",
    )
    .bind(group_id)
    .bind(to)
    .fetch_one(e)
    .await
}

/// The restore witness after a complete restore from seq 1: as far as any
/// restore got, and when the last one landed (the grace runs from it).
pub(crate) async fn set_restored<'e>(e: impl PgExecutor<'e>, group_id: Uuid, through_seq: i64, at: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE group_head SET restored_through_seq = GREATEST(restored_through_seq, $2), restored_at = $3, updated_at = clock_timestamp() WHERE group_id = $1")
        .bind(group_id)
        .bind(through_seq)
        .bind(at)
        .execute(e)
        .await?;
    Ok(())
}

/// An eviction reached the restored rows: the witness no longer holds.
pub(crate) async fn clear_restored<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE group_head SET restored_through_seq = 0, restored_at = NULL, updated_at = clock_timestamp() WHERE group_id = $1 AND restored_through_seq > 0")
        .bind(group_id)
        .execute(e)
        .await?;
    Ok(())
}

pub(crate) async fn delete_head<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("DELETE FROM group_head WHERE group_id = $1").bind(group_id).execute(e).await?;
    Ok(done.rows_affected() > 0)
}

// --- object: the log ---

pub(crate) async fn get_object<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Option<model::Object>, sqlx::Error> {
    sqlx::query_as::<_, model::Object>(&format!("SELECT {OBJECT_COLUMNS} FROM object WHERE id = $1"))
        .bind(id)
        .fetch_optional(e)
        .await
}

pub(crate) async fn insert_object<'e>(e: impl PgExecutor<'e>, row: &model::Object) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO object (id, group_id, value, seq, deleted, uploaded_at) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(row.id)
        .bind(row.group_id)
        .bind(&row.value)
        .bind(row.seq)
        .bind(row.deleted)
        .bind(row.uploaded_at)
        .execute(e)
        .await?;
    Ok(())
}

/// A history re-insert: the row at its original seq unless its id already
/// has a row, at any seq, live or tombstone (the current row always wins).
pub(crate) async fn insert_object_if_absent<'e>(e: impl PgExecutor<'e>, row: &model::Object) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("INSERT INTO object (id, group_id, value, seq, deleted, uploaded_at) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING")
        .bind(row.id)
        .bind(row.group_id)
        .bind(&row.value)
        .bind(row.seq)
        .bind(row.deleted)
        .bind(row.uploaded_at)
        .execute(e)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// A write to an existing row: the new state at a new seq.
pub(crate) async fn update_object<'e>(e: impl PgExecutor<'e>, id: Uuid, value: Option<&[u8]>, seq: i64, deleted: bool, at: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE object SET value = $2, seq = $3, deleted = $4, uploaded_at = $5, updated_at = clock_timestamp() WHERE id = $1")
        .bind(id)
        .bind(value)
        .bind(seq)
        .bind(deleted)
        .bind(at)
        .execute(e)
        .await?;
    Ok(())
}

/// The rows after `since_seq`, seq ascending, `limit` at most.
pub(crate) async fn entries_after<'e>(e: impl PgExecutor<'e>, group_id: Uuid, since_seq: i64, limit: i64) -> Result<Vec<model::Object>, sqlx::Error> {
    sqlx::query_as::<_, model::Object>(&format!("SELECT {OBJECT_COLUMNS} FROM object WHERE group_id = $1 AND seq > $2 ORDER BY seq LIMIT $3"))
        .bind(group_id)
        .bind(since_seq)
        .bind(limit)
        .fetch_all(e)
        .await
}

/// Eviction by ack: every row of the group at or below `seq`.
pub(crate) async fn delete_objects_through<'e>(e: impl PgExecutor<'e>, group_id: Uuid, seq: i64) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM object WHERE group_id = $1 AND seq <= $2")
        .bind(group_id)
        .bind(seq)
        .execute(e)
        .await?;
    Ok(done.rows_affected())
}

/// Eviction by age: one batch of rows uploaded before `cutoff`, answering
/// what went so the floors can follow.
pub(crate) async fn delete_objects_older_than<'e>(e: impl PgExecutor<'e>, cutoff: DateTime<Utc>, limit: i64) -> Result<Vec<(Uuid, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, i64)>(
        "DELETE FROM object WHERE ctid IN (SELECT ctid FROM object WHERE uploaded_at < $1 ORDER BY uploaded_at LIMIT $2) RETURNING group_id, seq",
    )
    .bind(cutoff)
    .bind(limit)
    .fetch_all(e)
    .await
}

/// A purge batch: `limit` rows of the group.
pub(crate) async fn delete_objects_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid, limit: i64) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM object WHERE ctid IN (SELECT ctid FROM object WHERE group_id = $1 LIMIT $2)")
        .bind(group_id)
        .bind(limit)
        .execute(e)
        .await?;
    Ok(done.rows_affected())
}

pub(crate) async fn count_objects_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM object WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}

// --- group_device: one ack per device ---

pub(crate) async fn get_group_device<'e>(e: impl PgExecutor<'e>, group_id: Uuid, device_id: Uuid) -> Result<Option<model::GroupDevice>, sqlx::Error> {
    sqlx::query_as::<_, model::GroupDevice>("SELECT group_id, device_id, acked_seq, updated_at FROM group_device WHERE group_id = $1 AND device_id = $2")
        .bind(group_id)
        .bind(device_id)
        .fetch_optional(e)
        .await
}

/// The device's ack, on its word: creates the row or moves it.
pub(crate) async fn upsert_ack<'e>(e: impl PgExecutor<'e>, group_id: Uuid, device_id: Uuid, seq: i64, at: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO group_device (group_id, device_id, acked_seq, updated_at) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (group_id, device_id) DO UPDATE SET acked_seq = EXCLUDED.acked_seq, updated_at = EXCLUDED.updated_at",
    )
    .bind(group_id)
    .bind(device_id)
    .bind(seq)
    .bind(at)
    .execute(e)
    .await?;
    Ok(())
}

/// The lowest ack among the group's devices, none when no device has a row.
pub(crate) async fn min_acked<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT min(acked_seq) FROM group_device WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}

/// The holders: devices whose ack is at or above the floor, `except` one.
pub(crate) async fn holders_except<'e>(e: impl PgExecutor<'e>, group_id: Uuid, floor: i64, except: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>("SELECT device_id FROM group_device WHERE group_id = $1 AND acked_seq >= $2 AND device_id <> $3 ORDER BY updated_at DESC, device_id")
        .bind(group_id)
        .bind(floor)
        .bind(except)
        .fetch_all(e)
        .await
}

pub(crate) async fn delete_group_device<'e>(e: impl PgExecutor<'e>, group_id: Uuid, device_id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("DELETE FROM group_device WHERE group_id = $1 AND device_id = $2")
        .bind(group_id)
        .bind(device_id)
        .execute(e)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The age rule's second half: devices below the floor that have not acked
/// since `cutoff` are evicted with the rows they never acked.
pub(crate) async fn delete_stale_devices<'e>(e: impl PgExecutor<'e>, group_id: Uuid, floor: i64, cutoff: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM group_device WHERE group_id = $1 AND acked_seq < $2 AND updated_at < $3")
        .bind(group_id)
        .bind(floor)
        .bind(cutoff)
        .execute(e)
        .await?;
    Ok(done.rows_affected())
}

pub(crate) async fn delete_group_devices<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM group_device WHERE group_id = $1").bind(group_id).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn count_group_devices<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_device WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}

/// Every group with devices, and its lowest ack: what eviction by ack
/// walks.
pub(crate) async fn group_minimums<'e>(e: impl PgExecutor<'e>) -> Result<Vec<(Uuid, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, i64)>("SELECT group_id, min(acked_seq) FROM group_device GROUP BY group_id ORDER BY group_id")
        .fetch_all(e)
        .await
}

// --- history_request ---

pub(crate) async fn insert_history_request<'e>(e: impl PgExecutor<'e>, row: &model::HistoryRequest) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO history_request (id, group_id, requester_device, from_seq, to_seq, object_id, claimed_by, claimed_at, through_seq, completed_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(row.id)
    .bind(row.group_id)
    .bind(row.requester_device)
    .bind(row.from_seq)
    .bind(row.to_seq)
    .bind(row.object_id)
    .bind(row.claimed_by)
    .bind(row.claimed_at)
    .bind(row.through_seq)
    .bind(row.completed_at)
    .bind(row.created_at)
    .execute(e)
    .await?;
    Ok(())
}

pub(crate) async fn get_history_request<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Option<model::HistoryRequest>, sqlx::Error> {
    sqlx::query_as::<_, model::HistoryRequest>(&format!("SELECT {REQUEST_COLUMNS} FROM history_request WHERE id = $1"))
        .bind(id)
        .fetch_optional(e)
        .await
}

/// The group's requests a holder may claim: not complete, unclaimed or
/// claimed before `abandoned_before`, and not a whole-range request that an
/// older open whole-range request reaches: that one completes with it
/// (`complete_covered_requests`), so one upload serves both.
pub(crate) async fn claimable_requests<'e>(e: impl PgExecutor<'e>, group_id: Uuid, abandoned_before: DateTime<Utc>) -> Result<Vec<model::HistoryRequest>, sqlx::Error> {
    sqlx::query_as::<_, model::HistoryRequest>(&format!(
        "SELECT {REQUEST_COLUMNS} FROM history_request r \
         WHERE r.group_id = $1 AND r.completed_at IS NULL AND (r.claimed_by IS NULL OR r.claimed_at < $2) \
         AND NOT (r.from_seq = 1 AND r.object_id IS NULL AND EXISTS ( \
             SELECT 1 FROM history_request c \
             WHERE c.group_id = r.group_id AND c.id <> r.id AND c.completed_at IS NULL \
             AND c.from_seq = 1 AND c.object_id IS NULL AND c.to_seq >= r.to_seq \
             AND (c.created_at, c.id) < (r.created_at, r.id))) \
         ORDER BY r.created_at, r.id"
    ))
    .bind(group_id)
    .bind(abandoned_before)
    .fetch_all(e)
    .await
}

/// Whether an open whole-range request of the group reaches `to_seq`: a
/// new whole-range request it covers is stored but not announced.
pub(crate) async fn covering_request_exists<'e>(e: impl PgExecutor<'e>, group_id: Uuid, to_seq: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM history_request WHERE group_id = $1 AND completed_at IS NULL AND from_seq = 1 AND object_id IS NULL AND to_seq >= $2)",
    )
    .bind(group_id)
    .bind(to_seq)
    .fetch_one(e)
    .await
}

/// The one conditional update of a claim: taken when unclaimed, already
/// the caller's, or abandoned, and not complete; the row after, or nothing
/// when another device holds it or it is complete.
pub(crate) async fn claim_request<'e>(e: impl PgExecutor<'e>, id: Uuid, device_id: Uuid, at: DateTime<Utc>, abandoned_before: DateTime<Utc>) -> Result<Option<model::HistoryRequest>, sqlx::Error> {
    sqlx::query_as::<_, model::HistoryRequest>(&format!(
        "UPDATE history_request SET claimed_by = $2, claimed_at = $3, updated_at = clock_timestamp() \
         WHERE id = $1 AND completed_at IS NULL AND (claimed_by IS NULL OR claimed_by = $2 OR claimed_at < $4) \
         RETURNING {REQUEST_COLUMNS}"
    ))
    .bind(id)
    .bind(device_id)
    .bind(at)
    .bind(abandoned_before)
    .fetch_optional(e)
    .await
}

/// What a history push leaves on its request: `through_seq` as far as any
/// call got, and the completion mark on the last one. The row stays for
/// the requester to read until the sweep drops it.
pub(crate) async fn record_history_push<'e>(e: impl PgExecutor<'e>, id: Uuid, through_seq: i64, completed_at: Option<DateTime<Utc>>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE history_request SET through_seq = GREATEST(through_seq, $2), completed_at = COALESCE(completed_at, $3), updated_at = clock_timestamp() WHERE id = $1")
        .bind(id)
        .bind(through_seq)
        .bind(completed_at)
        .execute(e)
        .await?;
    Ok(())
}

/// A complete restore from seq 1 through `to_seq` completes every other
/// open whole-range request of the group it reaches: the rows are in.
/// Answers them, for their wake-ups.
pub(crate) async fn complete_covered_requests<'e>(e: impl PgExecutor<'e>, group_id: Uuid, to_seq: i64, except: Uuid, at: DateTime<Utc>) -> Result<Vec<model::HistoryRequest>, sqlx::Error> {
    sqlx::query_as::<_, model::HistoryRequest>(&format!(
        "UPDATE history_request SET through_seq = to_seq, completed_at = $4, updated_at = clock_timestamp() \
         WHERE group_id = $1 AND id <> $3 AND completed_at IS NULL AND from_seq = 1 AND object_id IS NULL AND to_seq <= $2 \
         RETURNING {REQUEST_COLUMNS}"
    ))
    .bind(group_id)
    .bind(to_seq)
    .bind(except)
    .bind(at)
    .fetch_all(e)
    .await
}

/// The sweep: an open request goes a window after it was made, a complete
/// one a window after it completed, so a requester that completes late
/// still has the whole window to read `complete`.
pub(crate) async fn delete_history_requests_older_than<'e>(e: impl PgExecutor<'e>, cutoff: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM history_request WHERE COALESCE(completed_at, created_at) < $1").bind(cutoff).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn delete_history_requests_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM history_request WHERE group_id = $1").bind(group_id).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn count_history_requests_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM history_request WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}
