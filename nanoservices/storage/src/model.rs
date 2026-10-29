//! Row structs of this nanoservice's plain tables, one per table, mirroring
//! the columns in db/app/migrations exactly.

use serde::{Deserialize, Serialize};

/// A row of `nano_storage.upload_usage`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UploadUsage {
    pub group_id: uuid::Uuid,
    pub hour: chrono::DateTime<chrono::Utc>,
    pub bytes: i64,
}
