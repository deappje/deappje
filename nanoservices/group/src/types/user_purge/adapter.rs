//! The adapter: a dumb column mapper between `Spec`/`Status` and the
//! `user_purge_spec` / `user_purge_status` tables. Five callbacks
//! plus the optional finalize, every one database-only over the restricted
//! `Tx`; every column present in BOTH `write_spec` and `read_rows`, so a
//! column can never be silently lost. Never touches envelope columns.
//! Errors are `sqlx::Error`: a column mapper fails no other way, and the
//! framework wraps them.

use basable_processingobject::{Adapter as AdapterTrait, Object, Ref, Row, Tx};
use uuid::Uuid;

use super::{Spec, Status};

pub struct Adapter;

impl AdapterTrait<Spec, Status> for Adapter {
    async fn insert_spec(&self, tx: &mut Tx<'_>, r: &Ref, spec: &Spec) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO user_purge_spec (id, user_id) VALUES ($1, $2)")
            .bind(r.id)
            .bind(spec.user_id)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn insert_status(&self, tx: &mut Tx<'_>, r: &Ref, status: &Status) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO user_purge_status (id, phase, groups_remaining, devices_remaining, identity_deleted) VALUES ($1, $2, $3, $4, $5)")
            .bind(r.id)
            .bind(&status.phase)
            .bind(status.groups_remaining)
            .bind(status.devices_remaining)
            .bind(status.identity_deleted)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn read_rows(&self, tx: &mut Tx<'_>, ids: &[Uuid]) -> Result<Vec<Row<Spec, Status>>, sqlx::Error> {
        // Batched by contract: one query for the whole claim batch.
        let rows = sqlx::query(
            "SELECT s.id, s.user_id, t.phase, t.groups_remaining, t.devices_remaining, t.identity_deleted \
             FROM user_purge_spec s JOIN user_purge_status t ON t.id = s.id WHERE s.id = ANY($1)",
        )
        .bind(ids)
        .fetch_all(tx)
        .await?;
        rows.into_iter()
            .map(|row| {
                use sqlx::Row as _;
                Ok(Row {
                    id: row.try_get("id")?,
                    spec: Spec {
                        user_id: row.try_get("user_id")?,
                    },
                    status: Status {
                        phase: row.try_get("phase")?,
                        groups_remaining: row.try_get("groups_remaining")?,
                        devices_remaining: row.try_get("devices_remaining")?,
                        identity_deleted: row.try_get("identity_deleted")?,
                    },
                })
            })
            .collect()
    }

    async fn write_spec(&self, tx: &mut Tx<'_>, r: &Ref, spec: &Spec) -> Result<(), sqlx::Error> {
        // No mutable spec columns yet: only the timestamp moves.
        sqlx::query("UPDATE user_purge_spec SET updated_at = clock_timestamp() WHERE id = $1")
            .bind(r.id)
            .execute(tx)
            .await?;
        let _ = spec;
        Ok(())
    }

    async fn write_status(&self, tx: &mut Tx<'_>, r: &Ref, status: &Status) -> Result<(), sqlx::Error> {
        // The whole row, unconditionally: single provenance (invariant 3).
        sqlx::query("UPDATE user_purge_status SET updated_at = clock_timestamp(), phase = $2, groups_remaining = $3, devices_remaining = $4, identity_deleted = $5 WHERE id = $1")
            .bind(r.id)
            .bind(&status.phase)
            .bind(status.groups_remaining)
            .bind(status.devices_remaining)
            .bind(status.identity_deleted)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn finalize_delete(&self, _tx: &mut Tx<'_>, _obj: &Object<Spec, Status>) -> Result<(), sqlx::Error> {
        // Optional teardown evidence, run atomically with the envelope DELETE.
        Ok(())
    }
}
