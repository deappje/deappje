//! Typed sqlx queries over this nanoservice's own pool (schema
//! `nano_device`). Only this crate queries these tables; another
//! nanoservice that needs the data sends a message (the Directive §2).
//! Errors are `sqlx::Error` as they come; the handler decides what a
//! failure means to its caller (`AppError::wrap`).

use uuid::Uuid;

// Rows are `model::<Table>`, qualified: the component struct below and a
// message type in `messages` may carry the same name as a table.
use crate::model;
use crate::Device;

impl Device {
    // --- device ---

    pub async fn insert_device(&self, row: &model::Device) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO device (id, user_id, public_key, platform, push_provider, push_token, created_at, last_seen_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(row.id)
        .bind(row.user_id)
        .bind(&row.public_key)
        .bind(&row.platform)
        .bind(&row.push_provider)
        .bind(&row.push_token)
        .bind(&row.created_at)
        .bind(&row.last_seen_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_device(&self, id: Uuid) -> Result<Option<model::Device>, sqlx::Error> {
        sqlx::query_as::<_, model::Device>("SELECT id, user_id, public_key, platform, push_provider, push_token, created_at, last_seen_at FROM device WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- wrapped_key ---

    pub async fn insert_wrapped_key(&self, row: &model::WrappedKey) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO wrapped_key (id, group_id, key_id, device_id, wrapped, wrapped_by_device, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(row.id)
        .bind(row.group_id)
        .bind(row.key_id)
        .bind(row.device_id)
        .bind(&row.wrapped)
        .bind(row.wrapped_by_device)
        .bind(&row.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_wrapped_key(&self, id: Uuid) -> Result<Option<model::WrappedKey>, sqlx::Error> {
        sqlx::query_as::<_, model::WrappedKey>("SELECT id, group_id, key_id, device_id, wrapped, wrapped_by_device, created_at FROM wrapped_key WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- key_request ---

    pub async fn insert_key_request(&self, row: &model::KeyRequest) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO key_request (id, group_id, device_id, invite_id, invite_tag, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(row.id)
        .bind(row.group_id)
        .bind(row.device_id)
        .bind(row.invite_id)
        .bind(&row.invite_tag)
        .bind(&row.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_key_request(&self, id: Uuid) -> Result<Option<model::KeyRequest>, sqlx::Error> {
        sqlx::query_as::<_, model::KeyRequest>("SELECT id, group_id, device_id, invite_id, invite_tag, created_at FROM key_request WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
}
