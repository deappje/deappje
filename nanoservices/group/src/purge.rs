//! What the two purge lifecycles share: the deterministic ids of the purge
//! objects (so a retried request adopts what an earlier attempt created,
//! the store's own contract), their creation, and the one own-table write
//! both the delete handler and the `group_purge` pass perform.
//!
//! The order of a deletion is fixed by what a crash between two commits
//! must leave behind: FIRST the group row is stamped `deleted_at` under its
//! lock (from that commit on the group answers no membership), THEN the
//! purge object is created (adoptable by id), THEN the memberships and
//! invites go. A stamped group whose purge object is missing still has its
//! membership rows, so whoever holds one (the admin retrying the delete,
//! the account purge finding the membership again) re-creates the object;
//! a purge object whose rows were not deleted yet deletes them in its
//! first pass. Nothing in either order deletes what nobody asked for.

use basable_core::{BoxError, Ctx};
use basable_processingobject::{CreateOptions, Error, NamespacedName, Ref};
use chrono::{DateTime, Utc};
use interfaces::{GroupRoutes, GroupSender};
use messages::{MemberRemoved, Role};
use uuid::Uuid;

use crate::Group;
use crate::handlers::role_of;
use crate::model;
use crate::repository as repo;
use crate::types::{group_purge, user_purge};

/// The id of the `group_purge` object of a group: one per group, forever.
pub fn group_purge_id(group_id: Uuid) -> Uuid {
    Uuid::new_v5(&group_id, b"group_purge")
}

/// The id of the `user_purge` object of a user: one per user, forever.
pub fn user_purge_id(user_id: Uuid) -> Uuid {
    Uuid::new_v5(&user_id, b"user_purge")
}

impl Group {
    /// Creates, or adopts, the `group_purge` of `root` and of every group
    /// in `descendants` (each pointing at its parent's purge, so the
    /// parent's pass waits for them). `root_parent_purge` is the purge the
    /// root itself hangs under, `None` when its parent is not being
    /// deleted. An object already finishing (`Error::Deleting`) is left
    /// alone: that group is as good as gone.
    pub(crate) async fn ensure_group_purges(
        &self,
        root: Uuid,
        root_parent_purge: Option<Uuid>,
        descendants: &[model::Groups],
    ) -> Result<Ref, Error> {
        let r = self.ensure_group_purge(root, root_parent_purge).await?;
        for g in descendants {
            // A descendant's parent is the root or another descendant, both
            // being purged, so its purge hangs under its parent's.
            let parent = g.parent_group_id.map(group_purge_id);
            self.ensure_group_purge(g.id, parent).await?;
        }
        Ok(r)
    }

    async fn ensure_group_purge(&self, group_id: Uuid, parent_purge_id: Option<Uuid>) -> Result<Ref, Error> {
        let id = group_purge_id(group_id);
        let created = self
            .group_purge_store
            .create(
                id,
                NamespacedName::new(group_id, "group_purge"),
                &group_purge::Spec { group_id, parent_purge_id },
                &group_purge::Status {
                    phase: group_purge::phase::PURGING.to_owned(),
                    subgroups_remaining: 0,
                    objects_remaining: None,
                    blobs_remaining: None,
                    keys_remaining: None,
                },
                CreateOptions::none(),
            )
            .await;
        match created {
            Ok(r) => Ok(r),
            Err(Error::Deleting(r)) => Ok(r),
            Err(e) => Err(e),
        }
    }

    /// Creates, or adopts, the `user_purge` of a user.
    pub(crate) async fn ensure_user_purge(&self, user_id: Uuid) -> Result<Ref, Error> {
        let id = user_purge_id(user_id);
        let created = self
            .user_purge_store
            .create(
                id,
                NamespacedName::new(user_id, "user_purge"),
                &user_purge::Spec { user_id },
                &user_purge::Status {
                    phase: user_purge::phase::LEAVING.to_owned(),
                    groups_remaining: 0,
                    devices_remaining: None,
                    identity_deleted: false,
                },
                CreateOptions::none(),
            )
            .await;
        match created {
            Ok(r) => Ok(r),
            Err(Error::Deleting(r)) => Ok(r),
            Err(e) => Err(e),
        }
    }

    /// Removes what a deleted group still holds in this nanoservice's own
    /// tables: its memberships and invites, under the group's lock. The
    /// row is stamped if it somehow is not (an object exists only for a
    /// stamped group, so this is belt and braces). Idempotent; a group
    /// whose row is already gone has nothing left.
    pub(crate) async fn empty_group(&self, group_id: Uuid, at: DateTime<Utc>) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        if let Some(group) = repo::lock_group(&mut *tx, group_id).await? {
            if group.deleted_at.is_none() {
                repo::stamp_groups_deleted(&mut *tx, &[group_id], at).await?;
            }
            repo::delete_memberships_of_groups(&mut *tx, &[group_id]).await?;
            repo::delete_invites_of_groups(&mut *tx, &[group_id]).await?;
        }
        tx.commit().await
    }

    /// Gives up one membership of a user, the deepest group first, the way
    /// an account purge drains them, and answers how many remain; `None`
    /// when there was none to give up. Where the user is the only member
    /// the group goes with them (stamp, purge object, rows: the order of
    /// this module's doc); where they are the only admin the
    /// longest-standing member, else viewer, inherits the role. A group
    /// already stamped has its purge object ensured and this user's rows
    /// dropped, so a crash between stamp and object converges.
    pub(crate) async fn leave_one_group<R: GroupRoutes>(&self, ctx: &Ctx, sender: &GroupSender<'_, R>, user_id: Uuid) -> Result<Option<i64>, BoxError> {
        let Some(m) = repo::deepest_membership_of_user(&*self.pool, user_id).await? else {
            return Ok(None);
        };
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        match repo::lock_group(&mut *tx, m.group_id).await? {
            None => {
                // The row is gone (a purge finished under us); nothing holds
                // the stray membership any more.
                repo::delete_memberships_of_groups(&mut *tx, &[m.group_id]).await?;
                tx.commit().await?;
            }
            Some(group) if group.deleted_at.is_some() => {
                let descendants = repo::descendant_groups(&mut *tx, group.id).await?;
                tx.commit().await?;
                self.ensure_group_purges(group.id, None, &descendants).await?;
                repo::delete_user_memberships_in_tree(&*self.pool, group.id, user_id).await?;
            }
            Some(group) => {
                let members = repo::count_members(&mut *tx, group.id).await?;
                if members <= 1 {
                    // The only member: the group goes, and its subgroups
                    // with it (their members are a subset of its own).
                    let descendants = repo::descendant_groups(&mut *tx, group.id).await?;
                    let ids: Vec<Uuid> = std::iter::once(group.id).chain(descendants.iter().map(|g| g.id)).collect();
                    repo::stamp_groups_deleted(&mut *tx, &ids, now).await?;
                    tx.commit().await?;
                    self.ensure_group_purges(group.id, None, &descendants).await?;
                    let mut tx = self.pool.begin().await?;
                    repo::delete_memberships_of_groups(&mut *tx, &ids).await?;
                    repo::delete_invites_of_groups(&mut *tx, &ids).await?;
                    tx.commit().await?;
                } else {
                    if role_of(&m)? == Role::Admin
                        && repo::count_admins(&mut *tx, group.id).await? <= 1
                        && let Some(heir) = repo::promotion_candidate(&mut *tx, group.id, user_id).await?
                    {
                        repo::set_role(&mut *tx, group.id, heir.user_id, Role::Admin).await?;
                    }
                    let removed = repo::delete_user_memberships_in_tree(&mut *tx, group.id, user_id).await?;
                    tx.commit().await?;
                    for r in &removed {
                        sender
                            .send_member_removed(ctx, MemberRemoved { group_id: r.group_id, user_id, removed_at: now })
                            .await;
                    }
                }
            }
        }
        let remaining = repo::count_memberships_of_user(&*self.pool, user_id).await?;
        Ok(Some(remaining))
    }
}
