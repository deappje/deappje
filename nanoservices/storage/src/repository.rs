//! Typed sqlx queries over this nanoservice's own pool (schema
//! `nano_storage`). Only this crate queries these tables; another
//! nanoservice that needs the data sends a message (the Directive §2).
//! Errors are `sqlx::Error` as they come; the handler decides what a
//! failure means to its caller (`AppError::wrap`).

use uuid::Uuid;

// Rows are `model::<Table>`, qualified: the component struct below and a
// message type in `messages` may carry the same name as a table.
use crate::model;
use crate::Storage;

impl Storage {
    // --- upload_usage ---

    pub async fn insert_upload_usage(&self, row: &model::UploadUsage) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO upload_usage (group_id, hour, bytes) VALUES ($1, $2, $3)",
        )
        .bind(row.group_id)
        .bind(&row.hour)
        .bind(row.bytes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_upload_usage(&self, id: Uuid) -> Result<Option<model::UploadUsage>, sqlx::Error> {
        sqlx::query_as::<_, model::UploadUsage>("SELECT group_id, hour, bytes FROM upload_usage WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
}
