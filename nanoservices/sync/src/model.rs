//! Row structs of this nanoservice's plain tables, one per table, mirroring
//! the columns in db/app/migrations exactly.

use serde::{Deserialize, Serialize};

/// A row of `nano_sync.object`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Object {
    pub id: uuid::Uuid,
    pub group_id: uuid::Uuid,
    pub value: Option<Vec<u8>>,
    pub seq: i64,
    pub deleted: bool,
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
}

/// A row of `nano_sync.group_head`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GroupHead {
    pub group_id: uuid::Uuid,
    pub head_seq: i64,
    pub evicted_through_seq: i64,
}

/// A row of `nano_sync.group_device`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GroupDevice {
    pub group_id: uuid::Uuid,
    pub device_id: uuid::Uuid,
    pub acked_seq: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A row of `nano_sync.history_request`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct HistoryRequest {
    pub id: uuid::Uuid,
    pub group_id: uuid::Uuid,
    pub requester_device: uuid::Uuid,
    pub from_seq: i64,
    pub to_seq: i64,
    pub object_id: Option<uuid::Uuid>,
    pub claimed_by: Option<uuid::Uuid>,
    pub claimed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
