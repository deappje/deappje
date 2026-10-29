//! The adapter: a dumb column mapper between `Spec`/`Status` and the
//! `group_purge_spec` / `group_purge_status` tables. Five callbacks
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
        sqlx::query("INSERT INTO group_purge_spec (id, group_id, parent_purge_id) VALUES ($1, $2, $3)")
            .bind(r.id)
            .bind(spec.group_id)
            .bind(spec.parent_purge_id)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn insert_status(&self, tx: &mut Tx<'_>, r: &Ref, status: &Status) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO group_purge_status (id, phase, subgroups_remaining, objects_remaining, blobs_remaining, keys_remaining) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(r.id)
            .bind(&status.phase)
            .bind(status.subgroups_remaining)
            .bind(status.objects_remaining)
            .bind(status.blobs_remaining)
            .bind(status.keys_remaining)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn read_rows(&self, tx: &mut Tx<'_>, ids: &[Uuid]) -> Result<Vec<Row<Spec, Status>>, sqlx::Error> {
        // Batched by contract: one query for the whole claim batch.
        let rows = sqlx::query(
            "SELECT s.id, s.group_id, s.parent_purge_id, t.phase, t.subgroups_remaining, t.objects_remaining, t.blobs_remaining, t.keys_remaining \
             FROM group_purge_spec s JOIN group_purge_status t ON t.id = s.id WHERE s.id = ANY($1)",
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
                        group_id: row.try_get("group_id")?,
                        parent_purge_id: row.try_get("parent_purge_id")?,
                    },
                    status: Status {
                        phase: row.try_get("phase")?,
                        subgroups_remaining: row.try_get("subgroups_remaining")?,
                        objects_remaining: row.try_get("objects_remaining")?,
                        blobs_remaining: row.try_get("blobs_remaining")?,
                        keys_remaining: row.try_get("keys_remaining")?,
                    },
                })
            })
            .collect()
    }

    async fn write_spec(&self, tx: &mut Tx<'_>, r: &Ref, spec: &Spec) -> Result<(), sqlx::Error> {
        // No mutable spec columns yet: only the timestamp moves.
        sqlx::query("UPDATE group_purge_spec SET updated_at = clock_timestamp() WHERE id = $1")
            .bind(r.id)
            .execute(tx)
            .await?;
        let _ = spec;
        Ok(())
    }

    async fn write_status(&self, tx: &mut Tx<'_>, r: &Ref, status: &Status) -> Result<(), sqlx::Error> {
        // The whole row, unconditionally: single provenance (invariant 3).
        sqlx::query("UPDATE group_purge_status SET updated_at = clock_timestamp(), phase = $2, subgroups_remaining = $3, objects_remaining = $4, blobs_remaining = $5, keys_remaining = $6 WHERE id = $1")
            .bind(r.id)
            .bind(&status.phase)
            .bind(status.subgroups_remaining)
            .bind(status.objects_remaining)
            .bind(status.blobs_remaining)
            .bind(status.keys_remaining)
            .execute(tx)
            .await?;
        Ok(())
    }

    async fn finalize_delete(&self, _tx: &mut Tx<'_>, _obj: &Object<Spec, Status>) -> Result<(), sqlx::Error> {
        // Optional teardown evidence, run atomically with the envelope DELETE.
        Ok(())
    }
}
