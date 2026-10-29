//! Row structs of this nanoservice's plain tables, one per table, mirroring
//! the columns in db/app/migrations exactly.

use serde::{Deserialize, Serialize};

/// A row of `nano_device.device`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Device {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub public_key: Vec<u8>,
    pub platform: String,
    pub push_provider: Option<String>,
    pub push_token: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

/// A row of `nano_device.wrapped_key`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WrappedKey {
    pub id: uuid::Uuid,
    pub group_id: uuid::Uuid,
    pub key_id: i32,
    pub device_id: uuid::Uuid,
    pub wrapped: Vec<u8>,
    pub wrapped_by_device: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A row of `nano_device.key_request`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct KeyRequest {
    pub id: uuid::Uuid,
    pub group_id: uuid::Uuid,
    pub device_id: uuid::Uuid,
    pub invite_id: Option<uuid::Uuid>,
    pub invite_tag: Option<Vec<u8>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
