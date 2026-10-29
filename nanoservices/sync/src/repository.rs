//! Typed sqlx queries over this nanoservice's own pool (schema
//! `nano_sync`). Only this crate queries these tables; another
//! nanoservice that needs the data sends a message (the Directive §2).
//! Errors are `sqlx::Error` as they come; the handler decides what a
//! failure means to its caller (`AppError::wrap`).

use uuid::Uuid;

use crate::model::*;
use crate::Sync;

impl Sync {
    // --- object ---

    pub async fn insert_object(&self, row: &Object) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO object (id, group_id, value, seq, deleted, uploaded_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(row.id)
        .bind(row.group_id)
        .bind(&row.value)
        .bind(row.seq)
        .bind(row.deleted)
        .bind(&row.uploaded_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_object(&self, id: Uuid) -> Result<Option<Object>, sqlx::Error> {
        sqlx::query_as::<_, Object>("SELECT id, group_id, value, seq, deleted, uploaded_at FROM object WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- group_head ---

    pub async fn insert_group_head(&self, row: &GroupHead) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO group_head (group_id, head_seq, evicted_through_seq) VALUES ($1, $2, $3)",
        )
        .bind(row.group_id)
        .bind(row.head_seq)
        .bind(row.evicted_through_seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_group_head(&self, id: Uuid) -> Result<Option<GroupHead>, sqlx::Error> {
        sqlx::query_as::<_, GroupHead>("SELECT group_id, head_seq, evicted_through_seq FROM group_head WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- group_device ---

    pub async fn insert_group_device(&self, row: &GroupDevice) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO group_device (group_id, device_id, acked_seq, updated_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(row.group_id)
        .bind(row.device_id)
        .bind(row.acked_seq)
        .bind(&row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_group_device(&self, id: Uuid) -> Result<Option<GroupDevice>, sqlx::Error> {
        sqlx::query_as::<_, GroupDevice>("SELECT group_id, device_id, acked_seq, updated_at FROM group_device WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- history_request ---

    pub async fn insert_history_request(&self, row: &HistoryRequest) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO history_request (id, group_id, requester_device, from_seq, to_seq, object_id, claimed_by, claimed_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(row.id)
        .bind(row.group_id)
        .bind(row.requester_device)
        .bind(row.from_seq)
        .bind(row.to_seq)
        .bind(row.object_id)
        .bind(row.claimed_by)
        .bind(&row.claimed_at)
        .bind(&row.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_history_request(&self, id: Uuid) -> Result<Option<HistoryRequest>, sqlx::Error> {
        sqlx::query_as::<_, HistoryRequest>("SELECT id, group_id, requester_device, from_seq, to_seq, object_id, claimed_by, claimed_at, created_at FROM history_request WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
}
