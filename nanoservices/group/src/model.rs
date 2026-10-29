//! Row structs of this nanoservice's plain tables, one per table, mirroring
//! the columns in db/app/migrations exactly.

use serde::{Deserialize, Serialize};

/// A row of `nano_group.group`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Group {
    pub id: uuid::Uuid,
    pub parent_group_id: Option<uuid::Uuid>,
    pub created_by: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A row of `nano_group.membership`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Membership {
    pub id: uuid::Uuid,
    pub group_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub role: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

/// A row of `nano_group.invite`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Invite {
    pub id: uuid::Uuid,
    pub group_id: uuid::Uuid,
    pub code: String,
    pub role: String,
    pub created_by: uuid::Uuid,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub max_uses: i32,
    pub uses: i32,
    pub revoked: bool,
}
