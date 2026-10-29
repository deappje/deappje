//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). Handlers are the INTENT boundary:
//! they validate, write desired state through the typed store (`create`,
//! `update_spec`, `mark_deleted`, `nudge`) or the repository, dispatch
//! request-path adapters `Unfenced`, and send through `s` (the sender that
//! exposes exactly the declared sends). They never write status.

use basable_core::{AppError, Ctx};
use interfaces::{GroupHandler, GroupRoutes, GroupSender};

use crate::Group;

// Messages stay qualified: a message may share its name with this
// component (an `order` nanoservice handling an `Order` response).
impl<R: GroupRoutes> GroupHandler<R> for Group {
    async fn handle_create_group_request(&self, _ctx: &Ctx, _msg: messages::CreateGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Group, AppError> {
        Err(crate::unimplemented_step("group.handle_create_group_request"))
    }
    async fn handle_get_group_request(&self, _ctx: &Ctx, _msg: messages::GetGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Group, AppError> {
        Err(crate::unimplemented_step("group.handle_get_group_request"))
    }
    async fn handle_list_groups_request(&self, _ctx: &Ctx, _msg: messages::ListGroupsRequest, _s: GroupSender<'_, R>) -> Result<messages::GroupList, AppError> {
        Err(crate::unimplemented_step("group.handle_list_groups_request"))
    }
    async fn handle_create_invite_request(&self, _ctx: &Ctx, _msg: messages::CreateInviteRequest, _s: GroupSender<'_, R>) -> Result<messages::Invite, AppError> {
        Err(crate::unimplemented_step("group.handle_create_invite_request"))
    }
    async fn handle_accept_invite_request(&self, _ctx: &Ctx, _msg: messages::AcceptInviteRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        Err(crate::unimplemented_step("group.handle_accept_invite_request"))
    }
    async fn handle_list_members_request(&self, _ctx: &Ctx, _msg: messages::ListMembersRequest, _s: GroupSender<'_, R>) -> Result<messages::MemberList, AppError> {
        Err(crate::unimplemented_step("group.handle_list_members_request"))
    }
    async fn handle_add_member_request(&self, _ctx: &Ctx, _msg: messages::AddMemberRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        Err(crate::unimplemented_step("group.handle_add_member_request"))
    }
    async fn handle_remove_member_request(&self, _ctx: &Ctx, _msg: messages::RemoveMemberRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        Err(crate::unimplemented_step("group.handle_remove_member_request"))
    }
    async fn handle_set_member_role_request(&self, _ctx: &Ctx, _msg: messages::SetMemberRoleRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        Err(crate::unimplemented_step("group.handle_set_member_role_request"))
    }
    async fn handle_leave_group_request(&self, _ctx: &Ctx, _msg: messages::LeaveGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Membership, AppError> {
        Err(crate::unimplemented_step("group.handle_leave_group_request"))
    }
    async fn handle_delete_group_request(&self, _ctx: &Ctx, _msg: messages::DeleteGroupRequest, _s: GroupSender<'_, R>) -> Result<messages::Group, AppError> {
        Err(crate::unimplemented_step("group.handle_delete_group_request"))
    }
    async fn handle_delete_account_request(&self, _ctx: &Ctx, _msg: messages::DeleteAccountRequest, _s: GroupSender<'_, R>) -> Result<messages::AccountDeletion, AppError> {
        Err(crate::unimplemented_step("group.handle_delete_account_request"))
    }
    async fn handle_check_membership_request(&self, _ctx: &Ctx, _msg: messages::CheckMembershipRequest, _s: GroupSender<'_, R>) -> Result<messages::MembershipCheck, AppError> {
        Err(crate::unimplemented_step("group.handle_check_membership_request"))
    }
}
