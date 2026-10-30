//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). Handlers are the INTENT boundary:
//! they validate, write desired state through the typed store (`create`,
//! `update_spec`, `mark_deleted`, `nudge`) or the repository, dispatch
//! request-path adapters `Unfenced`, and send through `s` (the sender that
//! exposes exactly the declared sends). They never write status.
//!
//! Every change to a group's memberships runs in one transaction that
//! locks the group row first (`repository::lock_group`), so the counts the
//! rules read (members, admins) are what commits. Two transactions that
//! lock a group and an invite take the group first (`accept_invite` reads
//! the invite's group without a lock, then locks in that order), so a
//! deletion deleting the invites never deadlocks with an accept.

use std::str::FromStr;

use basable_core::{AppError, Code, Ctx};
use chrono::{DateTime, Utc};
use interfaces::{GroupHandler, GroupRoutes, GroupSender};
use messages::{MemberRemoved, Role};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::purge::user_purge_id;
use crate::repository as repo;
use crate::{Group, model};

/// The 32 symbols an invite code is drawn from: lowercase letters and
/// digits without the pairs a reader confuses (0 and o, 1 and l), so a code
/// can be read aloud. Ten symbols are 50 bits.
const INVITE_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";
const INVITE_CODE_LEN: usize = 10;

fn db(op: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    move |e| AppError::wrap(Code::Internal, format!("group: {op}"), e)
}

fn store(op: &'static str) -> impl FnOnce(basable_processingobject::Error) -> AppError {
    move |e| AppError::wrap(Code::Internal, format!("group: {op}"), e)
}

/// A stored role, which only this crate writes, so an unknown one is
/// corruption, not input.
pub(crate) fn role_of(m: &model::Membership) -> Result<Role, AppError> {
    Role::from_str(&m.role).map_err(|e| AppError::wrap(Code::Internal, format!("group: membership {} carries an unknown role", m.id), e))
}

fn group_msg(g: &model::Groups) -> messages::Group {
    messages::Group {
        id: g.id,
        parent_group_id: g.parent_group_id,
        created_by: g.created_by,
        created_at: g.created_at,
        deleted_at: g.deleted_at,
    }
}

fn membership_msg(m: &model::Membership) -> Result<messages::Membership, AppError> {
    Ok(messages::Membership {
        id: m.id,
        group_id: m.group_id,
        user_id: m.user_id,
        role: role_of(m)?,
        joined_at: m.joined_at,
    })
}

fn invite_msg(i: &model::Invite) -> Result<messages::Invite, AppError> {
    let role = Role::from_str(&i.role).map_err(|e| AppError::wrap(Code::Internal, format!("group: invite {} carries an unknown role", i.id), e))?;
    Ok(messages::Invite {
        id: i.id,
        group_id: i.group_id,
        code: i.code.clone(),
        role,
        created_by: i.created_by,
        expires_at: i.expires_at,
        max_uses: i.max_uses,
        uses: i.uses,
        revoked: i.revoked,
    })
}

/// A fresh invite code from the OS's random source (a v4 UUID is 122
/// random bits; the code takes 50 of them).
pub(crate) fn mint_invite_code() -> String {
    let bits = Uuid::new_v4().as_u128();
    (0..INVITE_CODE_LEN)
        .map(|i| INVITE_ALPHABET[((bits >> (5 * i)) & 31) as usize] as char)
        .collect()
}

/// A live group or `NotFound`: a group stamped `deleted_at` is gone to
/// every caller from that commit on, whatever rows still hang off it.
fn live(group: Option<model::Groups>) -> Result<model::Groups, AppError> {
    match group {
        Some(g) if g.deleted_at.is_none() => Ok(g),
        _ => Err(AppError::not_found("group not found")),
    }
}

/// Whether an invite admits a joiner now: not revoked, not expired, uses
/// left (`max_uses` 0 is unbounded).
pub(crate) fn invite_admits(inv: &model::Invite, now: DateTime<Utc>) -> Result<(), AppError> {
    if inv.revoked {
        return Err(AppError::failed_precondition("invite revoked"));
    }
    if inv.expires_at <= now {
        return Err(AppError::failed_precondition("invite expired"));
    }
    if inv.max_uses > 0 && inv.uses >= inv.max_uses {
        return Err(AppError::failed_precondition("invite used up"));
    }
    Ok(())
}

/// The last-admin rule: an admin may leave or be demoted unless they are
/// the only admin and other members remain.
pub(crate) fn admin_may_go(role: Role, admins: i64, members: i64) -> Result<(), AppError> {
    if role == Role::Admin && admins <= 1 && members > 1 {
        return Err(AppError::failed_precondition("the last admin cannot leave or be demoted while other members remain"));
    }
    Ok(())
}

/// The role an invite or `AddMember` hands out: never admin, which only
/// `SetMemberRole` grants.
pub(crate) fn joiner_role(role: Role) -> Result<Role, AppError> {
    match role {
        Role::Member | Role::Viewer => Ok(role),
        Role::Admin => Err(AppError::invalid_argument("a joiner is a member or a viewer; promote with SetMemberRole")),
    }
}

/// The caller's membership of a live group, or why not.
async fn require_member<'e>(e: impl PgExecutor<'e>, group: &model::Groups, user: Uuid) -> Result<model::Membership, AppError> {
    repo::get_membership(e, group.id, user)
        .await
        .map_err(db("membership"))?
        .ok_or_else(|| AppError::permission_denied("not a member of this group"))
}

async fn require_admin<'e>(e: impl PgExecutor<'e>, group: &model::Groups, user: Uuid) -> Result<model::Membership, AppError> {
    let m = require_member(e, group, user).await?;
    if role_of(&m)? != Role::Admin {
        return Err(AppError::permission_denied("only an admin may do this"));
    }
    Ok(m)
}

impl Group {
    /// An account being deleted makes no more calls: the purge is draining
    /// its memberships, and a new one would be gone in a moment.
    async fn refuse_deleting_account(&self, caller: Uuid) -> Result<(), AppError> {
        if repo::user_purge_exists(&*self.pool, caller).await.map_err(db("account check"))? {
            return Err(AppError::failed_precondition("account deletion in progress"));
        }
        Ok(())
    }

    /// Sends `MemberRemoved` for each membership that went, after the
    /// commit (a messenger send is remote I/O, invariant 7).
    async fn announce_removed<R: GroupRoutes>(&self, ctx: &Ctx, s: &GroupSender<'_, R>, removed: &[model::Membership], at: DateTime<Utc>) {
        for m in removed {
            s.send_member_removed(ctx, MemberRemoved { group_id: m.group_id, user_id: m.user_id, removed_at: at })
                .await;
        }
    }
}

// Messages stay qualified: a message may share its name with this
// component (an `group` nanoservice handling a `Group` response).
impl<R: GroupRoutes> GroupHandler<R> for Group {
    async fn handle_create_group_request(&self, _ctx: &Ctx, msg: messages::CreateGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Group, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let now = Utc::now();
        let group = model::Groups {
            id: Uuid::new_v4(),
            parent_group_id: msg.parent_group_id,
            created_by: msg.caller,
            created_at: now,
            deleted_at: None,
        };
        let admin = model::Membership {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: msg.caller,
            role: Role::Admin.as_str().to_owned(),
            joined_at: now,
        };
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        if let Some(parent_id) = msg.parent_group_id {
            // The parent's lock serializes this against its deletion.
            let parent = live(repo::lock_group(&mut *tx, parent_id).await.map_err(db("lock parent"))?)?;
            if parent.parent_group_id.is_some() {
                return Err(AppError::invalid_argument("a subgroup cannot have subgroups"));
            }
            if repo::get_membership(&mut *tx, parent_id, msg.caller).await.map_err(db("parent membership"))?.is_none() {
                return Err(AppError::permission_denied("not a member of the parent group"));
            }
        }
        repo::insert_group(&mut *tx, &group).await.map_err(db("insert group"))?;
        repo::insert_membership(&mut *tx, &admin).await.map_err(db("insert membership"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(group_msg(&group))
    }

    async fn handle_get_group_request(&self, _ctx: &Ctx, msg: messages::GetGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Group, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let group = live(repo::get_group(&*self.pool, msg.group_id).await.map_err(db("group"))?)?;
        require_member(&*self.pool, &group, msg.caller).await?;
        Ok(group_msg(&group))
    }

    async fn handle_list_groups_request(&self, _ctx: &Ctx, msg: messages::ListGroupsRequest, _s: GroupSender<'_, R>) -> Result<messages::GroupList, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let groups = repo::groups_of_user(&*self.pool, msg.caller).await.map_err(db("groups of user"))?;
        Ok(messages::GroupList { groups: groups.iter().map(group_msg).collect() })
    }

    async fn handle_create_invite_request(&self, _ctx: &Ctx, msg: messages::CreateInviteRequest, _s: GroupSender<'_, R>) -> Result<messages::Invite, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let role = joiner_role(msg.role)?;
        let now = Utc::now();
        if msg.max_uses < 0 {
            return Err(AppError::invalid_argument("max_uses is 0 (unbounded) or positive"));
        }
        if msg.expires_at <= now {
            return Err(AppError::invalid_argument("expires_at is in the past"));
        }
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = live(repo::lock_group(&mut *tx, msg.group_id).await.map_err(db("lock group"))?)?;
        require_admin(&mut *tx, &group, msg.caller).await?;
        let invite = model::Invite {
            id: Uuid::new_v4(),
            group_id: group.id,
            code: mint_invite_code(),
            role: role.as_str().to_owned(),
            created_by: msg.caller,
            expires_at: msg.expires_at,
            max_uses: msg.max_uses,
            uses: 0,
            revoked: false,
        };
        // A code collision is one in 2^50; the unique index refuses it and
        // the caller's retry mints another.
        repo::insert_invite(&mut *tx, &invite).await.map_err(db("insert invite"))?;
        tx.commit().await.map_err(db("commit"))?;
        invite_msg(&invite)
    }

    async fn handle_accept_invite_request(&self, _ctx: &Ctx, msg: messages::AcceptInviteRequest, _s: GroupSender<'_, R>) -> Result<messages::InviteAccepted, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let now = Utc::now();
        // The invite's group, read unlocked so the locks below are taken in
        // the one order every transaction uses: the group, then the invite.
        let peek = sqlx::query_scalar::<_, Uuid>("SELECT group_id FROM invite WHERE code = $1")
            .bind(&msg.code)
            .fetch_optional(&*self.pool)
            .await
            .map_err(db("invite"))?
            .ok_or_else(|| AppError::not_found("invite not found"))?;
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = live(repo::lock_group(&mut *tx, peek).await.map_err(db("lock group"))?)?;
        let invite = repo::lock_invite_by_code(&mut *tx, &msg.code)
            .await
            .map_err(db("lock invite"))?
            .filter(|i| i.group_id == group.id)
            .ok_or_else(|| AppError::not_found("invite not found"))?;
        if let Some(existing) = repo::get_membership(&mut *tx, group.id, msg.caller).await.map_err(db("membership"))? {
            // Already in, whatever the invite's state now: nothing to
            // change, and no use consumed.
            return Ok(messages::InviteAccepted { membership: membership_msg(&existing)?, invite: invite_msg(&invite)? });
        }
        invite_admits(&invite, now)?;
        if let Some(parent_id) = group.parent_group_id
            && repo::get_membership(&mut *tx, parent_id, msg.caller).await.map_err(db("parent membership"))?.is_none()
        {
            return Err(AppError::permission_denied("not a member of the parent group"));
        }
        let membership = model::Membership {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: msg.caller,
            role: invite.role.clone(),
            joined_at: now,
        };
        repo::insert_membership(&mut *tx, &membership).await.map_err(db("insert membership"))?;
        repo::record_invite_use(&mut *tx, invite.id).await.map_err(db("record use"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(messages::InviteAccepted { membership: membership_msg(&membership)?, invite: invite_msg(&invite)? })
    }

    async fn handle_list_members_request(&self, _ctx: &Ctx, msg: messages::ListMembersRequest, _s: GroupSender<'_, R>) -> Result<messages::MemberList, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let group = live(repo::get_group(&*self.pool, msg.group_id).await.map_err(db("group"))?)?;
        require_member(&*self.pool, &group, msg.caller).await?;
        let members = repo::list_members(&*self.pool, group.id).await.map_err(db("members"))?;
        Ok(messages::MemberList {
            group_id: group.id,
            members: members.iter().map(membership_msg).collect::<Result<_, _>>()?,
        })
    }

    async fn handle_add_member_request(&self, _ctx: &Ctx, msg: messages::AddMemberRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let role = joiner_role(msg.role)?;
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = live(repo::lock_group(&mut *tx, msg.group_id).await.map_err(db("lock group"))?)?;
        let parent_id = group
            .parent_group_id
            .ok_or_else(|| AppError::failed_precondition("members are added to a subgroup only; a group is joined by invite"))?;
        let caller = require_member(&mut *tx, &group, msg.caller).await?;
        if !role_of(&caller)?.can_write() {
            return Err(AppError::permission_denied("only an admin or member of the subgroup adds members"));
        }
        if repo::get_membership(&mut *tx, parent_id, msg.user_id).await.map_err(db("parent membership"))?.is_none() {
            return Err(AppError::failed_precondition("the user is not a member of the parent group"));
        }
        if let Some(existing) = repo::get_membership(&mut *tx, group.id, msg.user_id).await.map_err(db("membership"))? {
            return membership_msg(&existing);
        }
        let membership = model::Membership {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: msg.user_id,
            role: role.as_str().to_owned(),
            joined_at: Utc::now(),
        };
        repo::insert_membership(&mut *tx, &membership).await.map_err(db("insert membership"))?;
        tx.commit().await.map_err(db("commit"))?;
        membership_msg(&membership)
    }

    async fn handle_remove_member_request(&self, ctx: &Ctx, msg: messages::RemoveMemberRequest, s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        if msg.user_id == msg.caller {
            return Err(AppError::invalid_argument("leave the group instead of removing yourself"));
        }
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = live(repo::lock_group(&mut *tx, msg.group_id).await.map_err(db("lock group"))?)?;
        require_admin(&mut *tx, &group, msg.caller).await?;
        if repo::get_membership(&mut *tx, group.id, msg.user_id).await.map_err(db("membership"))?.is_none() {
            return Err(AppError::not_found("not a member of this group"));
        }
        // The caller stays, an admin, so no last-admin check applies.
        let removed = repo::delete_user_memberships_in_tree(&mut *tx, group.id, msg.user_id)
            .await
            .map_err(db("delete memberships"))?;
        tx.commit().await.map_err(db("commit"))?;
        let at = Utc::now();
        self.announce_removed(ctx, &s, &removed, at).await;
        let own = removed.first().ok_or_else(|| AppError::internal("group: the removed membership vanished"))?;
        membership_msg(own)
    }

    async fn handle_set_member_role_request(&self, _ctx: &Ctx, msg: messages::SetMemberRoleRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = live(repo::lock_group(&mut *tx, msg.group_id).await.map_err(db("lock group"))?)?;
        require_admin(&mut *tx, &group, msg.caller).await?;
        let target = repo::get_membership(&mut *tx, group.id, msg.user_id)
            .await
            .map_err(db("membership"))?
            .ok_or_else(|| AppError::not_found("not a member of this group"))?;
        let current = role_of(&target)?;
        if current == msg.role {
            return membership_msg(&target);
        }
        if current == Role::Admin {
            let admins = repo::count_admins(&mut *tx, group.id).await.map_err(db("count admins"))?;
            let members = repo::count_members(&mut *tx, group.id).await.map_err(db("count members"))?;
            admin_may_go(current, admins, members)?;
        }
        let updated = repo::set_role(&mut *tx, group.id, msg.user_id, msg.role)
            .await
            .map_err(db("set role"))?
            .ok_or_else(|| AppError::internal("group: the membership vanished under its lock"))?;
        tx.commit().await.map_err(db("commit"))?;
        membership_msg(&updated)
    }

    async fn handle_leave_group_request(&self, ctx: &Ctx, msg: messages::LeaveGroupRequest, s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = live(repo::lock_group(&mut *tx, msg.group_id).await.map_err(db("lock group"))?)?;
        let membership = require_member(&mut *tx, &group, msg.caller).await?;
        let members = repo::count_members(&mut *tx, group.id).await.map_err(db("count members"))?;
        if members <= 1 {
            // Nobody would be left to delete it, and an empty group is
            // unreachable forever.
            return Err(AppError::failed_precondition("the last member cannot leave; delete the group instead"));
        }
        let admins = repo::count_admins(&mut *tx, group.id).await.map_err(db("count admins"))?;
        admin_may_go(role_of(&membership)?, admins, members)?;
        let removed = repo::delete_user_memberships_in_tree(&mut *tx, group.id, msg.caller)
            .await
            .map_err(db("delete memberships"))?;
        tx.commit().await.map_err(db("commit"))?;
        let at = Utc::now();
        self.announce_removed(ctx, &s, &removed, at).await;
        membership_msg(&membership)
    }

    async fn handle_delete_group_request(&self, _ctx: &Ctx, msg: messages::DeleteGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Group, AppError> {
        self.refuse_deleting_account(msg.caller).await?;
        let now = Utc::now();
        // 1. Stamp, under the lock: from this commit on the group answers
        //    no membership. A stamped group is accepted again (the retry of
        //    a request whose purge object never got created) as long as
        //    the caller's admin row is still there.
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        let group = repo::lock_group(&mut *tx, msg.group_id)
            .await
            .map_err(db("lock group"))?
            .ok_or_else(|| AppError::not_found("group not found"))?;
        require_admin(&mut *tx, &group, msg.caller).await?;
        let descendants = repo::descendant_groups(&mut *tx, group.id).await.map_err(db("subgroups"))?;
        let ids: Vec<Uuid> = std::iter::once(group.id).chain(descendants.iter().map(|g| g.id)).collect();
        repo::stamp_groups_deleted(&mut *tx, &ids, now).await.map_err(db("stamp"))?;
        tx.commit().await.map_err(db("commit"))?;
        // 2. The intent: one purge object per group, adoptable by id.
        self.ensure_group_purges(group.id, None, &descendants).await.map_err(store("create group purge"))?;
        // 3. The rows go now rather than at the purge's first pass, so a
        //    member's next call is refused at once; the pass repeats this
        //    if this transaction never lands.
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::delete_memberships_of_groups(&mut *tx, &ids).await.map_err(db("delete memberships"))?;
        repo::delete_invites_of_groups(&mut *tx, &ids).await.map_err(db("delete invites"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(messages::Group {
            deleted_at: Some(group.deleted_at.unwrap_or(now)),
            ..group_msg(&group)
        })
    }

    async fn handle_delete_account_request(&self, _ctx: &Ctx, msg: messages::DeleteAccountRequest, _s: GroupSender<'_, R>) -> Result<messages::AccountDeletion, AppError> {
        self.ensure_user_purge(msg.caller).await.map_err(store("create user purge"))?;
        Ok(messages::AccountDeletion {
            user_id: msg.caller,
            purge_id: self.user_purge_store.public_id(user_purge_id(msg.caller)),
        })
    }

    async fn handle_check_membership_request(&self, _ctx: &Ctx, msg: messages::CheckMembershipRequest, _s: GroupSender<'_, R>) -> Result<messages::MembershipCheck, AppError> {
        let Some(group) = repo::get_group(&*self.pool, msg.group_id).await.map_err(db("group"))? else {
            return Ok(messages::MembershipCheck { role: None });
        };
        if group.deleted_at.is_some() {
            return Ok(messages::MembershipCheck { role: None });
        }
        let role = match repo::get_membership(&*self.pool, group.id, msg.user_id).await.map_err(db("membership"))? {
            Some(m) => Some(role_of(&m)?),
            None => None,
        };
        Ok(messages::MembershipCheck { role })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(expires_in_secs: i64, max_uses: i32, uses: i32, revoked: bool) -> model::Invite {
        model::Invite {
            id: Uuid::from_u128(1),
            group_id: Uuid::from_u128(2),
            code: "abcdefghjk".into(),
            role: "member".into(),
            created_by: Uuid::from_u128(3),
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in_secs),
            max_uses,
            uses,
            revoked,
        }
    }

    #[test]
    fn invite_rules() {
        let now = Utc::now();
        assert!(invite_admits(&invite(60, 0, 1000, false), now).is_ok(), "unbounded uses admit");
        assert!(invite_admits(&invite(60, 1, 0, false), now).is_ok(), "a use left admits");
        assert_eq!(invite_admits(&invite(60, 1, 1, false), now).unwrap_err().code(), Code::FailedPrecondition, "used up");
        assert_eq!(invite_admits(&invite(-1, 0, 0, false), now).unwrap_err().code(), Code::FailedPrecondition, "expired");
        assert_eq!(invite_admits(&invite(60, 0, 0, true), now).unwrap_err().code(), Code::FailedPrecondition, "revoked");
    }

    #[test]
    fn last_admin_rule() {
        assert!(admin_may_go(Role::Admin, 1, 1).is_ok(), "alone: may go");
        assert!(admin_may_go(Role::Admin, 2, 5).is_ok(), "another admin remains");
        assert!(admin_may_go(Role::Member, 1, 5).is_ok(), "not an admin");
        assert_eq!(admin_may_go(Role::Admin, 1, 2).unwrap_err().code(), Code::FailedPrecondition);
    }

    #[test]
    fn joiners_are_never_admins() {
        assert_eq!(joiner_role(Role::Member).unwrap(), Role::Member);
        assert_eq!(joiner_role(Role::Viewer).unwrap(), Role::Viewer);
        assert_eq!(joiner_role(Role::Admin).unwrap_err().code(), Code::InvalidArgument);
    }

    #[test]
    fn invite_codes_are_ten_symbols_of_the_alphabet() {
        for _ in 0..100 {
            let code = mint_invite_code();
            assert_eq!(code.len(), INVITE_CODE_LEN);
            assert!(code.bytes().all(|b| INVITE_ALPHABET.contains(&b)), "{code}");
        }
        assert_ne!(mint_invite_code(), mint_invite_code());
    }
}
