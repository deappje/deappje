//! Typed sqlx queries over this nanoservice's own tables (schema
//! `nano_group`). Only this crate queries them; another nanoservice that
//! needs the data sends a message (the Directive §2). Every query takes the
//! executor it runs on, the pool or a transaction's connection, so a
//! handler composes the locked transaction it needs from these pieces.
//! Errors are `sqlx::Error` as they come; the handler decides what a
//! failure means to its caller.

use chrono::{DateTime, Utc};
use messages::Role;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::model;

// --- groups ---

pub(crate) async fn insert_group<'e>(e: impl PgExecutor<'e>, row: &model::Groups) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO groups (id, parent_group_id, created_by, created_at, deleted_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(row.id)
        .bind(row.parent_group_id)
        .bind(row.created_by)
        .bind(row.created_at)
        .bind(row.deleted_at)
        .execute(e)
        .await?;
    Ok(())
}

pub(crate) async fn get_group<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Option<model::Groups>, sqlx::Error> {
    sqlx::query_as::<_, model::Groups>("SELECT id, parent_group_id, created_by, created_at, deleted_at FROM groups WHERE id = $1")
        .bind(id)
        .fetch_optional(e)
        .await
}

/// The group row under `FOR UPDATE`: every change to a group's memberships
/// takes this lock first, which is what makes the counts the rules read
/// (members, admins) true at commit.
pub(crate) async fn lock_group<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Option<model::Groups>, sqlx::Error> {
    sqlx::query_as::<_, model::Groups>(
        "SELECT id, parent_group_id, created_by, created_at, deleted_at FROM groups WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(e)
    .await
}

/// The live groups a user is a member of, oldest first.
pub(crate) async fn groups_of_user<'e>(e: impl PgExecutor<'e>, user_id: Uuid) -> Result<Vec<model::Groups>, sqlx::Error> {
    sqlx::query_as::<_, model::Groups>(
        "SELECT g.id, g.parent_group_id, g.created_by, g.created_at, g.deleted_at \
         FROM groups g JOIN membership m ON m.group_id = g.id \
         WHERE m.user_id = $1 AND g.deleted_at IS NULL \
         ORDER BY g.created_at, g.id",
    )
    .bind(user_id)
    .fetch_all(e)
    .await
}

/// Every group below `id`, at any depth, parents before their children.
pub(crate) async fn descendant_groups<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<Vec<model::Groups>, sqlx::Error> {
    sqlx::query_as::<_, model::Groups>(
        "WITH RECURSIVE sub AS ( \
             SELECT g.id, g.parent_group_id, g.created_by, g.created_at, g.deleted_at, 1 AS depth \
             FROM groups g WHERE g.parent_group_id = $1 \
             UNION ALL \
             SELECT g.id, g.parent_group_id, g.created_by, g.created_at, g.deleted_at, sub.depth + 1 \
             FROM groups g JOIN sub ON g.parent_group_id = sub.id) \
         SELECT id, parent_group_id, created_by, created_at, deleted_at FROM sub ORDER BY depth, created_at, id",
    )
    .bind(id)
    .fetch_all(e)
    .await
}

/// Stamps `deleted_at` on every listed group that is not stamped yet.
pub(crate) async fn stamp_groups_deleted<'e>(e: impl PgExecutor<'e>, ids: &[Uuid], at: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE groups SET deleted_at = $2, updated_at = clock_timestamp() WHERE id = ANY($1) AND deleted_at IS NULL")
        .bind(ids)
        .bind(at)
        .execute(e)
        .await?;
    Ok(())
}

pub(crate) async fn delete_group<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query("DELETE FROM groups WHERE id = $1").bind(id).execute(e).await?;
    Ok(done.rows_affected() > 0)
}

// --- membership ---

pub(crate) async fn insert_membership<'e>(e: impl PgExecutor<'e>, row: &model::Membership) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO membership (id, group_id, user_id, role, joined_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(row.id)
        .bind(row.group_id)
        .bind(row.user_id)
        .bind(&row.role)
        .bind(row.joined_at)
        .execute(e)
        .await?;
    Ok(())
}

pub(crate) async fn get_membership<'e>(e: impl PgExecutor<'e>, group_id: Uuid, user_id: Uuid) -> Result<Option<model::Membership>, sqlx::Error> {
    sqlx::query_as::<_, model::Membership>(
        "SELECT id, group_id, user_id, role, joined_at FROM membership WHERE group_id = $1 AND user_id = $2",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_optional(e)
    .await
}

/// The members of a group, oldest membership first.
pub(crate) async fn list_members<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<Vec<model::Membership>, sqlx::Error> {
    sqlx::query_as::<_, model::Membership>(
        "SELECT id, group_id, user_id, role, joined_at FROM membership WHERE group_id = $1 ORDER BY joined_at, id",
    )
    .bind(group_id)
    .fetch_all(e)
    .await
}

pub(crate) async fn count_members<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM membership WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(e)
        .await
}

pub(crate) async fn count_admins<'e>(e: impl PgExecutor<'e>, group_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM membership WHERE group_id = $1 AND role = $2")
        .bind(group_id)
        .bind(Role::Admin.as_str())
        .fetch_one(e)
        .await
}

pub(crate) async fn set_role<'e>(e: impl PgExecutor<'e>, group_id: Uuid, user_id: Uuid, role: Role) -> Result<Option<model::Membership>, sqlx::Error> {
    sqlx::query_as::<_, model::Membership>(
        "UPDATE membership SET role = $3, updated_at = clock_timestamp() WHERE group_id = $1 AND user_id = $2 \
         RETURNING id, group_id, user_id, role, joined_at",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(role.as_str())
    .fetch_optional(e)
    .await
}

/// Who inherits the admin role when the last admin goes: the member with
/// the earliest `joined_at`, or the earliest viewer when there is no member.
pub(crate) async fn promotion_candidate<'e>(e: impl PgExecutor<'e>, group_id: Uuid, except: Uuid) -> Result<Option<model::Membership>, sqlx::Error> {
    sqlx::query_as::<_, model::Membership>(
        "SELECT id, group_id, user_id, role, joined_at FROM membership \
         WHERE group_id = $1 AND user_id <> $2 AND role IN ($3, $4) \
         ORDER BY (role = $3) DESC, joined_at, id LIMIT 1",
    )
    .bind(group_id)
    .bind(except)
    .bind(Role::Member.as_str())
    .bind(Role::Viewer.as_str())
    .fetch_optional(e)
    .await
}

/// Deletes a user's membership of a group and of every group below it (a
/// member of a subgroup is a member of its parent, so the cascade keeps
/// that true). Returns what was deleted, the parent's row first.
pub(crate) async fn delete_user_memberships_in_tree<'e>(e: impl PgExecutor<'e>, group_id: Uuid, user_id: Uuid) -> Result<Vec<model::Membership>, sqlx::Error> {
    let mut rows = sqlx::query_as::<_, model::Membership>(
        "WITH RECURSIVE sub AS ( \
             SELECT id FROM groups WHERE id = $1 \
             UNION ALL \
             SELECT g.id FROM groups g JOIN sub ON g.parent_group_id = sub.id) \
         DELETE FROM membership m USING sub WHERE m.group_id = sub.id AND m.user_id = $2 \
         RETURNING m.id, m.group_id, m.user_id, m.role, m.joined_at",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_all(e)
    .await?;
    rows.sort_by_key(|m| m.group_id != group_id);
    Ok(rows)
}

pub(crate) async fn delete_memberships_of_groups<'e>(e: impl PgExecutor<'e>, ids: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM membership WHERE group_id = ANY($1)").bind(ids).execute(e).await?;
    Ok(done.rows_affected())
}

/// The user's membership of the deepest group they are in (ties: the
/// oldest), so an account purge leaves subgroups before their parents and
/// a parent leave never has to cascade.
pub(crate) async fn deepest_membership_of_user<'e>(e: impl PgExecutor<'e>, user_id: Uuid) -> Result<Option<model::Membership>, sqlx::Error> {
    sqlx::query_as::<_, model::Membership>(
        "WITH RECURSIVE d AS ( \
             SELECT id, 0 AS depth FROM groups WHERE parent_group_id IS NULL \
             UNION ALL \
             SELECT g.id, d.depth + 1 FROM groups g JOIN d ON g.parent_group_id = d.id) \
         SELECT m.id, m.group_id, m.user_id, m.role, m.joined_at \
         FROM membership m JOIN d ON d.id = m.group_id \
         WHERE m.user_id = $1 \
         ORDER BY d.depth DESC, m.joined_at, m.id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(e)
    .await
}

pub(crate) async fn count_memberships_of_user<'e>(e: impl PgExecutor<'e>, user_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM membership WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(e)
        .await
}

// --- invite ---

pub(crate) async fn insert_invite<'e>(e: impl PgExecutor<'e>, row: &model::Invite) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO invite (id, group_id, code, role, created_by, expires_at, max_uses, uses, revoked) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(row.id)
    .bind(row.group_id)
    .bind(&row.code)
    .bind(&row.role)
    .bind(row.created_by)
    .bind(row.expires_at)
    .bind(row.max_uses)
    .bind(row.uses)
    .bind(row.revoked)
    .execute(e)
    .await?;
    Ok(())
}

/// The invite row under `FOR UPDATE`: two accepts of one invite serialize
/// here, so `uses` is counted exactly.
pub(crate) async fn lock_invite_by_code<'e>(e: impl PgExecutor<'e>, code: &str) -> Result<Option<model::Invite>, sqlx::Error> {
    sqlx::query_as::<_, model::Invite>(
        "SELECT id, group_id, code, role, created_by, expires_at, max_uses, uses, revoked FROM invite WHERE code = $1 FOR UPDATE",
    )
    .bind(code)
    .fetch_optional(e)
    .await
}

pub(crate) async fn record_invite_use<'e>(e: impl PgExecutor<'e>, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE invite SET uses = uses + 1, updated_at = clock_timestamp() WHERE id = $1")
        .bind(id)
        .execute(e)
        .await?;
    Ok(())
}

pub(crate) async fn delete_invites_of_groups<'e>(e: impl PgExecutor<'e>, ids: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query("DELETE FROM invite WHERE group_id = ANY($1)").bind(ids).execute(e).await?;
    Ok(done.rows_affected())
}

// --- the purge types' own tables (reads only; every write is the store's) ---

/// How many `group_purge` objects name this one as their parent: the
/// subgroup purges a parent's pass waits for. A finished purge is
/// hard-deleted, so the count falls to zero by itself.
pub(crate) async fn count_child_purges<'e>(e: impl PgExecutor<'e>, parent_purge_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_purge_spec WHERE parent_purge_id = $1")
        .bind(parent_purge_id)
        .fetch_one(e)
        .await
}

/// Whether an account deletion is under way for the user.
pub(crate) async fn user_purge_exists<'e>(e: impl PgExecutor<'e>, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM user_purge_spec WHERE user_id = $1)")
        .bind(user_id)
        .fetch_one(e)
        .await
}
