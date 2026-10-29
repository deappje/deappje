//! The declarative config types this nanoservice owns: a person edits
//! `config/base/<name>.json`, the loader applies it at boot, and the
//! binders here map each message onto its subtype table in
//! `basable_config`. The messages are the proto types in
//! `proto/storage/v1/config.proto`; a reference field arrives as the
//! referenced object's id.

use basable_config::{BinderError, ConfigHeader, ConfigTypesBuilder, TypeInfo, TypedBinder};
use sqlx::PgConnection;
use uuid::Uuid;

pub use proto::proto::storage::v1::SlotConfiguration;

/// `SlotConfiguration` (`slot_…`), id 100.
pub const SLOT_TYPE: TypeInfo = TypeInfo {
    id: 100,
    name: "SlotConfiguration",
    prefix: "slot",
};

/// The binder over `basable_config.slot_configuration`.
pub struct SlotBinder;

impl TypedBinder for SlotBinder {
    type Msg = SlotConfiguration;

    fn type_info(&self) -> TypeInfo {
        SLOT_TYPE
    }

    fn header(&self, msg: &SlotConfiguration) -> ConfigHeader {
        // The proto's header is a message field, absent when unset.
        let h = msg.header.as_option();
        ConfigHeader {
            namespace: h.map(|h| h.namespace.clone()).unwrap_or_default(),
            name: h.map(|h| h.name.clone()).unwrap_or_default(),
            external_id: h.map(|h| h.external_id.clone()).unwrap_or_default(),
            labels: h
                .map(|h| h.labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        }
    }

    async fn upsert(
        &self,
        tx: &mut PgConnection,
        id: Uuid,
        msg: &SlotConfiguration,
    ) -> Result<(), BinderError> {
        sqlx::query(
            "INSERT INTO basable_config.slot_configuration (id, max_size_bytes)
             VALUES ($1, $2)
             ON CONFLICT (id) DO UPDATE SET
                 max_size_bytes = EXCLUDED.max_size_bytes",
        )
        .bind(id)
        .bind(msg.max_size_bytes)
        .execute(tx)
        .await?;
        Ok(())
    }

    async fn delete(&self, tx: &mut PgConnection, id: Uuid) -> Result<(), BinderError> {
        // Refuse with BinderError::StillReferenced here while another
        // object still names this one; a prune retries once it is gone.
        sqlx::query("DELETE FROM basable_config.slot_configuration WHERE id = $1")
            .bind(id)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn read(
        &self,
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<SlotConfiguration>, BinderError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT max_size_bytes
             FROM basable_config.slot_configuration WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|(max_size_bytes,)| SlotConfiguration {
            max_size_bytes,
            ..Default::default()
        }))
    }
}

pub use proto::proto::storage::v1::StorageQuotaConfiguration;

/// `StorageQuotaConfiguration` (`squo_…`), id 101.
pub const STORAGE_QUOTA_TYPE: TypeInfo = TypeInfo {
    id: 101,
    name: "StorageQuotaConfiguration",
    prefix: "squo",
};

/// The binder over `basable_config.storage_quota_configuration`.
pub struct StorageQuotaBinder;

impl TypedBinder for StorageQuotaBinder {
    type Msg = StorageQuotaConfiguration;

    fn type_info(&self) -> TypeInfo {
        STORAGE_QUOTA_TYPE
    }

    fn header(&self, msg: &StorageQuotaConfiguration) -> ConfigHeader {
        // The proto's header is a message field, absent when unset.
        let h = msg.header.as_option();
        ConfigHeader {
            namespace: h.map(|h| h.namespace.clone()).unwrap_or_default(),
            name: h.map(|h| h.name.clone()).unwrap_or_default(),
            external_id: h.map(|h| h.external_id.clone()).unwrap_or_default(),
            labels: h
                .map(|h| h.labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        }
    }

    async fn upsert(
        &self,
        tx: &mut PgConnection,
        id: Uuid,
        msg: &StorageQuotaConfiguration,
    ) -> Result<(), BinderError> {
        sqlx::query(
            "INSERT INTO basable_config.storage_quota_configuration (id, max_bytes_per_168h)
             VALUES ($1, $2)
             ON CONFLICT (id) DO UPDATE SET
                 max_bytes_per_168h = EXCLUDED.max_bytes_per_168h",
        )
        .bind(id)
        .bind(msg.max_bytes_per_168h)
        .execute(tx)
        .await?;
        Ok(())
    }

    async fn delete(&self, tx: &mut PgConnection, id: Uuid) -> Result<(), BinderError> {
        // Refuse with BinderError::StillReferenced here while another
        // object still names this one; a prune retries once it is gone.
        sqlx::query("DELETE FROM basable_config.storage_quota_configuration WHERE id = $1")
            .bind(id)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn read(
        &self,
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<StorageQuotaConfiguration>, BinderError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT max_bytes_per_168h
             FROM basable_config.storage_quota_configuration WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|(max_bytes_per_168h,)| StorageQuotaConfiguration {
            max_bytes_per_168h,
            ..Default::default()
        }))
    }
}

/// Registers this nanoservice's config types with the app's builder.
pub fn register(b: &mut ConfigTypesBuilder) {
    b.register(SlotBinder);
    b.register(StorageQuotaBinder);
}
