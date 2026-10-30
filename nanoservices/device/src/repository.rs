//! Typed sqlx queries over this nanoservice's own tables (schema
//! `nano_device`). Only this crate queries them; another nanoservice that
//! needs the data sends a message (the Directive §2). Every query takes the
//! executor it runs on, the pool or a transaction's connection. Errors are
//! `sqlx::Error` as they come; the handler decides what a failure means.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::model;

const DEVICE_COLUMNS: &str = "id, user_id, public_key, platform, push_provider, push_token, created_at, last_seen_at";
/// A key with the wrapping device's public key beside it (`k` the key,
/// `d` the device, left-joined: a wrapping device that is gone leaves it
/// empty).
const KEY_COPY_COLUMNS: &str =
    "k.id, k.group_id, k.key_id, k.device_id, k.wrapped, k.wrapped_by_device, k.created_at, COALESCE(d.public_key, ''::bytea) AS wrapped_by_public_key";
const REQUEST_COLUMNS: &str = "id, group_id, device_id, invite_id, invite_tag, created_at";

// --- device ---

pub(crate) async fn insert_device<'e>(e: impl PgExecutor<'e>, row: &model::Device) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO device (id, user_id, public_key, platform, push_provider, push_token, created_at, last_seen_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(row.id)
    .bind(row.user_id)
    .bind(&row.public_key)
    .bind(&row.platform)
    .bind(&row.push_provider)
    .bind(&row.push_token)
    .bind(row.created_at)
    .bind(row.last_seen_at)
    .execute(e)
    .await?;
    Ok(())
}

pub(crate) async fn get_device<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Option<model::Device>, sqlx::Error> {
    sqlx::query_as::<_, model::Device>(&format!("SELECT {DEVICE_COLUMNS} FROM device WHERE id = $1"))
        .bind(id)
        .fetch_optional(e)
        .await
}

pub(crate) async fn devices_of_user<'e>(e: impl PgExecutor<'e>, user_id: Uuid) -> Result<Vec<model::Device>, sqlx::Error> {
    sqlx::query_as::<_, model::Device>(&format!("SELECT {DEVICE_COLUMNS} FROM device WHERE user_id = $1 ORDER BY created_at, id"))
        .bind(user_id)
        .fetch_all(e)
        .await
}

/// Several devices by id, in id order; unknown ids are simply absent.
pub(crate) async fn devices_by_id<'e>(e: impl PgExecutor<'e>, ids: &[Uuid]) -> Result<Vec<model::Device>, sqlx::Error> {
    sqlx::query_as::<_, model::Device>(&format!("SELECT {DEVICE_COLUMNS} FROM device WHERE id = ANY($1) ORDER BY id"))
        .bind(ids)
        .fetch_all(e)
        .await
}

/// A repeat registration: the platform may change, the clock moves.
pub(crate) async fn touch_device<'e>(e: impl PgExecutor<'e>, id: Uuid, platform: &str, seen_at: DateTime<Utc>) -> Result<Option<model::Device>, sqlx::Error> {
    sqlx::query_as::<_, model::Device>(&format!(
        "UPDATE device SET platform = $2, last_seen_at = $3, updated_at = clock_timestamp() WHERE id = $1 RETURNING {DEVICE_COLUMNS}"
    ))
    .bind(id)
    .bind(platform)
    .bind(seen_at)
    .fetch_optional(e)
    .await
}

pub(crate) async fn set_push<'e>(e: impl PgExecutor<'e>, id: Uuid, provider: Option<&str>, token: Option<&str>) -> Result<Option<model::Device>, sqlx::Error> {
    sqlx::query_as::<_, model::Device>(&format!(
        "UPDATE device SET push_provider = $2, push_token = $3, updated_at = clock_timestamp() WHERE id = $1 RETURNING {DEVICE_COLUMNS}"
    ))
    .bind(id)
    .bind(provider)
    .bind(token)
    .fetch_optional(e)
    .await
}

/// Forgets a token the provider refused as unregistered, unless the device
/// registered a newer one meanwhile.
pub(crate) async fn clear_push_token<'e>(e: impl PgExecutor<'e>, id: Uuid, token: &str) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE device SET push_provider = NULL, push_token = NULL, updated_at = clock_timestamp() WHERE id = $1 AND push_token = $2",
    )
    .bind(id)
    .bind(token)
    .execute(e)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(crate) async fn delete_device<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("DELETE FROM device WHERE id = $1").bind(id).execute(e).await?;
    Ok(done.rows_affected() > 0)
}

pub(crate) async fn delete_devices<'e>(e: impl PgExecutor<'e>, ids: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM device WHERE id = ANY($1)").bind(ids).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn count_devices_of_user<'e>(e: impl PgExecutor<'e>, user_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(e)
        .await
}

// --- wrapped_key ---

/// Inserts what is new; a row that exists (same group, key id and device)
/// is left as it is, so two devices racing with the same batch end with
/// one row each. Answers how many landed.
pub(crate) async fn insert_wrapped_keys<'e>(e: impl PgExecutor<'e>, rows: &[model::WrappedKey]) -> Result<u64, sqlx::Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut ids = Vec::with_capacity(rows.len());
    let mut groups = Vec::with_capacity(rows.len());
    let mut key_ids = Vec::with_capacity(rows.len());
    let mut devices = Vec::with_capacity(rows.len());
    let mut wrapped = Vec::with_capacity(rows.len());
    let mut by = Vec::with_capacity(rows.len());
    let mut at = Vec::with_capacity(rows.len());
    for r in rows {
        ids.push(r.id);
        groups.push(r.group_id);
        key_ids.push(r.key_id);
        devices.push(r.device_id);
        wrapped.push(r.wrapped.clone());
        by.push(r.wrapped_by_device);
        at.push(r.created_at);
    }
    let done = sqlx::query(
        "INSERT INTO wrapped_key (id, group_id, key_id, device_id, wrapped, wrapped_by_device, created_at) \
         SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::int4[], $4::uuid[], $5::bytea[], $6::uuid[], $7::timestamptz[]) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&ids)
    .bind(&groups)
    .bind(&key_ids)
    .bind(&devices)
    .bind(&wrapped)
    .bind(&by)
    .bind(&at)
    .execute(e)
    .await?;
    Ok(done.rows_affected())
}

/// The keys wrapped for a device, all groups or one, by group then key id,
/// each with its wrapping device's public key.
pub(crate) async fn keys_for_device<'e>(e: impl PgExecutor<'e>, device_id: Uuid, group_id: Option<Uuid>) -> Result<Vec<model::WrappedKeyCopy>, sqlx::Error> {
    sqlx::query_as::<_, model::WrappedKeyCopy>(&format!(
        "SELECT {KEY_COPY_COLUMNS} FROM wrapped_key k LEFT JOIN device d ON d.id = k.wrapped_by_device \
         WHERE k.device_id = $1 AND ($2::uuid IS NULL OR k.group_id = $2) ORDER BY k.group_id, k.key_id, k.id"
    ))
    .bind(device_id)
    .bind(group_id)
    .fetch_all(e)
    .await
}

/// The rows of one key id, for the listed devices, each with its wrapping
/// device's public key.
pub(crate) async fn keys_of<'e>(e: impl PgExecutor<'e>, group_id: Uuid, key_id: i32, devices: &[Uuid]) -> Result<Vec<model::WrappedKeyCopy>, sqlx::Error> {
    sqlx::query_as::<_, model::WrappedKeyCopy>(&format!(
        "SELECT {KEY_COPY_COLUMNS} FROM wrapped_key k LEFT JOIN device d ON d.id = k.wrapped_by_device \
         WHERE k.group_id = $1 AND k.key_id = $2 AND k.device_id = ANY($3) ORDER BY k.device_id, k.id"
    ))
    .bind(group_id)
    .bind(key_id)
    .bind(devices)
    .fetch_all(e)
    .await
}

/// Whether the device holds any key of the group: a holder.
pub(crate) async fn device_holds<'e>(e: impl PgExecutor<'e>, group_id: Uuid, device_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM wrapped_key WHERE group_id = $1 AND device_id = $2)")
        .bind(group_id)
        .bind(device_id)
        .fetch_one(e)
        .await
}

/// Of the listed devices, those holding key `key_id` of the group.
pub(crate) async fn devices_holding_key<'e>(e: impl PgExecutor<'e>, group_id: Uuid, key_id: i32, devices: &[Uuid]) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>("SELECT DISTINCT device_id FROM wrapped_key WHERE group_id = $1 AND key_id = $2 AND device_id = ANY($3)")
        .bind(group_id)
        .bind(key_id)
        .bind(devices)
        .fetch_all(e)
        .await
}

/// The device that published a key id, if any did (the rotation rule
/// keeps it to one).
pub(crate) async fn wrapper_of_key<'e>(e: impl PgExecutor<'e>, group_id: Uuid, key_id: i32) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>("SELECT wrapped_by_device FROM wrapped_key WHERE group_id = $1 AND key_id = $2 ORDER BY created_at, id LIMIT 1")
        .bind(group_id)
        .bind(key_id)
        .fetch_optional(e)
        .await
}

/// The devices holding a key for the group, with their rows.
pub(crate) async fn holder_devices<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<Vec<model::Device>, sqlx::Error> {
    sqlx::query_as::<_, model::Device>(&format!(
        "SELECT {DEVICE_COLUMNS} FROM device WHERE id IN (SELECT DISTINCT device_id FROM wrapped_key WHERE group_id = $1) ORDER BY created_at, id"
    ))
    .bind(group_id)
    .fetch_all(e)
    .await
}

/// The groups a device holds a key for.
pub(crate) async fn groups_held_by_device<'e>(e: impl PgExecutor<'e>, device_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>("SELECT DISTINCT group_id FROM wrapped_key WHERE device_id = $1 ORDER BY group_id")
        .bind(device_id)
        .fetch_all(e)
        .await
}

pub(crate) async fn delete_keys_of_devices_in_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid, devices: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM wrapped_key WHERE group_id = $1 AND device_id = ANY($2)")
        .bind(group_id)
        .bind(devices)
        .execute(e)
        .await?;
    Ok(done.rows_affected())
}

pub(crate) async fn delete_keys_of_devices<'e>(e: impl PgExecutor<'e>, devices: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM wrapped_key WHERE device_id = ANY($1)").bind(devices).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn delete_keys_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM wrapped_key WHERE group_id = $1").bind(group_id).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn count_keys_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wrapped_key WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}

// --- key_request ---

/// Opens a request unless the device already has one open for the group
/// (the unique index), in which case nothing is inserted.
pub(crate) async fn insert_key_request<'e>(e: impl PgExecutor<'e>, row: &model::KeyRequest) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "INSERT INTO key_request (id, group_id, device_id, invite_id, invite_tag, created_at) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
    )
    .bind(row.id)
    .bind(row.group_id)
    .bind(row.device_id)
    .bind(row.invite_id)
    .bind(&row.invite_tag)
    .bind(row.created_at)
    .execute(e)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(crate) async fn get_key_request<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Option<model::KeyRequest>, sqlx::Error> {
    sqlx::query_as::<_, model::KeyRequest>(&format!("SELECT {REQUEST_COLUMNS} FROM key_request WHERE id = $1"))
        .bind(id)
        .fetch_optional(e)
        .await
}

/// The device's open request for the group, if any.
pub(crate) async fn open_key_request<'e>(e: impl PgExecutor<'e>, group_id: Uuid, device_id: Uuid) -> Result<Option<model::KeyRequest>, sqlx::Error> {
    sqlx::query_as::<_, model::KeyRequest>(&format!(
        "SELECT {REQUEST_COLUMNS} FROM key_request WHERE group_id = $1 AND device_id = $2 ORDER BY created_at, id LIMIT 1"
    ))
    .bind(group_id)
    .bind(device_id)
    .fetch_optional(e)
    .await
}

/// An open request with the requesting device's row.
pub(crate) struct OpenRequest {
    pub request: model::KeyRequest,
    pub device: model::Device,
}

/// The open requests in every group `holder` holds a key for, oldest
/// first, each with the device that asked.
pub(crate) async fn requests_for_holder<'e>(e: impl PgExecutor<'e>, holder: Uuid) -> Result<Vec<OpenRequest>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        group_id: Uuid,
        device_id: Uuid,
        invite_id: Option<Uuid>,
        invite_tag: Option<Vec<u8>>,
        created_at: DateTime<Utc>,
        user_id: Uuid,
        public_key: Vec<u8>,
        platform: String,
        push_provider: Option<String>,
        push_token: Option<String>,
        device_created_at: DateTime<Utc>,
        last_seen_at: DateTime<Utc>,
    }
    let rows = sqlx::query_as::<_, Row>(
        "SELECT r.id, r.group_id, r.device_id, r.invite_id, r.invite_tag, r.created_at, \
                d.user_id, d.public_key, d.platform, d.push_provider, d.push_token, d.created_at AS device_created_at, d.last_seen_at \
         FROM key_request r JOIN device d ON d.id = r.device_id \
         WHERE r.group_id IN (SELECT DISTINCT group_id FROM wrapped_key WHERE device_id = $1) \
         ORDER BY r.created_at, r.id",
    )
    .bind(holder)
    .fetch_all(e)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OpenRequest {
            request: model::KeyRequest {
                id: r.id,
                group_id: r.group_id,
                device_id: r.device_id,
                invite_id: r.invite_id,
                invite_tag: r.invite_tag,
                created_at: r.created_at,
            },
            device: model::Device {
                id: r.device_id,
                user_id: r.user_id,
                public_key: r.public_key,
                platform: r.platform,
                push_provider: r.push_provider,
                push_token: r.push_token,
                created_at: r.device_created_at,
                last_seen_at: r.last_seen_at,
            },
        })
        .collect())
}

pub(crate) async fn delete_key_request<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("DELETE FROM key_request WHERE id = $1").bind(id).execute(e).await?;
    Ok(done.rows_affected() > 0)
}

pub(crate) async fn delete_requests_of_devices_in_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid, devices: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM key_request WHERE group_id = $1 AND device_id = ANY($2)")
        .bind(group_id)
        .bind(devices)
        .execute(e)
        .await?;
    Ok(done.rows_affected())
}

pub(crate) async fn delete_requests_of_devices<'e>(e: impl PgExecutor<'e>, devices: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM key_request WHERE device_id = ANY($1)").bind(devices).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn delete_requests_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM key_request WHERE group_id = $1").bind(group_id).execute(e).await?;
    Ok(done.rows_affected())
}

pub(crate) async fn count_requests_of_group<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM key_request WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}
