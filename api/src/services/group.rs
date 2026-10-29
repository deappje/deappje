//! `GroupService`: the Connect service of `group`. Each
//! method validates the request, converts the buffa message into the plain
//! `messages` struct, sends it through the api sender, and converts the
//! answer back. Errors map to Connect codes through basable_connect.

use std::sync::Arc;

use basable_connect::{ConnectRouter, into_connect_error, request_ctx};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use proto::connect::group::v1::GroupService;
use proto::proto::group::v1::*;

use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct GroupServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> GroupService for GroupServiceImpl<R> {
    async fn create_group(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateGroupRequest>) -> ServiceResult<CreateGroupResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::CreateGroupRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `CreateGroupResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.create_group")))
    }
    async fn delete_group(&self, ctx: RequestContext, request: ServiceRequest<'_, DeleteGroupRequest>) -> ServiceResult<DeleteGroupResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::DeleteGroupRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `DeleteGroupResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.delete_group")))
    }
    async fn delete_account(&self, ctx: RequestContext, request: ServiceRequest<'_, DeleteAccountRequest>) -> ServiceResult<DeleteAccountResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::DeleteAccountRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `DeleteAccountResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.delete_account")))
    }
    async fn get_group(&self, ctx: RequestContext, request: ServiceRequest<'_, GetGroupRequest>) -> ServiceResult<GetGroupResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::GetGroupRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `GetGroupResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.get_group")))
    }
    async fn list_groups(&self, ctx: RequestContext, request: ServiceRequest<'_, ListGroupsRequest>) -> ServiceResult<ListGroupsResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::ListGroupsRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `ListGroupsResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.list_groups")))
    }
    async fn create_invite(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateInviteRequest>) -> ServiceResult<CreateInviteResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::CreateInviteRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `CreateInviteResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.create_invite")))
    }
    async fn accept_invite(&self, ctx: RequestContext, request: ServiceRequest<'_, AcceptInviteRequest>) -> ServiceResult<AcceptInviteResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::AcceptInviteRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `AcceptInviteResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.accept_invite")))
    }
    async fn list_members(&self, ctx: RequestContext, request: ServiceRequest<'_, ListMembersRequest>) -> ServiceResult<ListMembersResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::ListMembersRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `ListMembersResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.list_members")))
    }
    async fn add_member(&self, ctx: RequestContext, request: ServiceRequest<'_, AddMemberRequest>) -> ServiceResult<AddMemberResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::AddMemberRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `AddMemberResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.add_member")))
    }
    async fn remove_member(&self, ctx: RequestContext, request: ServiceRequest<'_, RemoveMemberRequest>) -> ServiceResult<RemoveMemberResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::RemoveMemberRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `RemoveMemberResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.remove_member")))
    }
    async fn set_member_role(&self, ctx: RequestContext, request: ServiceRequest<'_, SetMemberRoleRequest>) -> ServiceResult<SetMemberRoleResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::SetMemberRoleRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `SetMemberRoleResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.set_member_role")))
    }
    async fn leave_group(&self, ctx: RequestContext, request: ServiceRequest<'_, LeaveGroupRequest>) -> ServiceResult<LeaveGroupResponse> {
        let ctx = request_ctx(&ctx);
        let _req = request.to_owned_message();
        // TODO: validate `_req`, convert it into `messages::LeaveGroupRequest`,
        // send it with `self.sender.send_…(&ctx, m).await`,
        // and convert the answer into `LeaveGroupResponse` (`Ok(connectrpc::Response::new(..))`).
        let _ = (&ctx, self.sender);
        Err(into_connect_error(crate::unimplemented_step("api.leave_group")))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(GroupServiceImpl { sender: ApiSender::new(router) }));
}
