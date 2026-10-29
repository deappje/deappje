//! Typed sqlx queries over this nanoservice's own pool (schema
//! `nano_group`). Only this crate queries these tables; another
//! nanoservice that needs the data sends a message (the Directive §2).
//! Errors are `sqlx::Error` as they come; the handler decides what a
//! failure means to its caller (`AppError::wrap`).

use uuid::Uuid;

// Rows are `model::<Table>`, qualified: the component struct below and a
// message type in `messages` may carry the same name as a table.
use crate::model;
use crate::Group;

impl Group {
    // --- groups ---

    pub async fn insert_groups(&self, row: &model::Groups) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO groups (id, parent_group_id, created_by, created_at, deleted_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(row.id)
        .bind(row.parent_group_id)
        .bind(row.created_by)
        .bind(&row.created_at)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_groups(&self, id: Uuid) -> Result<Option<model::Groups>, sqlx::Error> {
        sqlx::query_as::<_, model::Groups>("SELECT id, parent_group_id, created_by, created_at, deleted_at FROM groups WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- membership ---

    pub async fn insert_membership(&self, row: &model::Membership) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO membership (id, group_id, user_id, role, joined_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(row.id)
        .bind(row.group_id)
        .bind(row.user_id)
        .bind(&row.role)
        .bind(&row.joined_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_membership(&self, id: Uuid) -> Result<Option<model::Membership>, sqlx::Error> {
        sqlx::query_as::<_, model::Membership>("SELECT id, group_id, user_id, role, joined_at FROM membership WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
    // --- invite ---

    pub async fn insert_invite(&self, row: &model::Invite) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO invite (id, group_id, code, role, created_by, expires_at, max_uses, uses, revoked) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(row.id)
        .bind(row.group_id)
        .bind(&row.code)
        .bind(&row.role)
        .bind(row.created_by)
        .bind(&row.expires_at)
        .bind(row.max_uses)
        .bind(row.uses)
        .bind(row.revoked)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_invite(&self, id: Uuid) -> Result<Option<model::Invite>, sqlx::Error> {
        sqlx::query_as::<_, model::Invite>("SELECT id, group_id, code, role, created_by, expires_at, max_uses, uses, revoked FROM invite WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
}
