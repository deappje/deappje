//! `GroupService`: the Connect service of `group`. Each method takes the
//! caller from the validated session, converts the buffa message into the
//! plain `messages` struct, sends it through the api sender, and converts
//! the answer back. Errors map to Connect codes through basable_connect.

use std::str::FromStr;
use std::sync::Arc;

use basable_connect::{ConnectRouter, IntoConnect, into_connect_error, request_ctx};
use basable_core::AppError;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use messages::Role;
use proto::connect::group::v1::GroupService;
use proto::proto::group::v1::*;

use super::{caller, millis, optional_millis, optional_uuid_field, time_field, uuid_field};
use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct GroupServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

fn role_field(value: &str) -> Result<Role, AppError> {
    Role::from_str(value).map_err(|e| AppError::invalid_argument(format!("role: {e}")))
}

fn group(g: messages::Group) -> Group {
    Group {
        id: g.id.to_string(),
        parent_group_id: g.parent_group_id.map(|id| id.to_string()).unwrap_or_default(),
        created_by: g.created_by.to_string(),
        created_at_ms: millis(g.created_at),
        deleted_at_ms: optional_millis(g.deleted_at),
        ..Default::default()
    }
}

fn membership(m: messages::Membership) -> Membership {
    Membership {
        id: m.id.to_string(),
        group_id: m.group_id.to_string(),
        user_id: m.user_id.to_string(),
        role: m.role.as_str().to_owned(),
        joined_at_ms: millis(m.joined_at),
        ..Default::default()
    }
}

fn invite(i: messages::Invite) -> Invite {
    Invite {
        id: i.id.to_string(),
        group_id: i.group_id.to_string(),
        code: i.code,
        role: i.role.as_str().to_owned(),
        created_by: i.created_by.to_string(),
        expires_at_ms: millis(i.expires_at),
        max_uses: i.max_uses,
        uses: i.uses,
        revoked: i.revoked,
        ..Default::default()
    }
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> GroupService for GroupServiceImpl<R> {
    async fn create_group(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateGroupRequest>) -> ServiceResult<CreateGroupResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::CreateGroupRequest {
            caller: caller(&ctx).into_connect()?,
            parent_group_id: optional_uuid_field(&req.parent_group_id, "parent_group_id").into_connect()?,
        };
        let g = self.sender.send_create_group_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(CreateGroupResponse { group: Some(group(g)).into(), ..Default::default() }))
    }

    async fn delete_group(&self, ctx: RequestContext, request: ServiceRequest<'_, DeleteGroupRequest>) -> ServiceResult<DeleteGroupResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::DeleteGroupRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let g = self.sender.send_delete_group_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(DeleteGroupResponse { group: Some(group(g)).into(), ..Default::default() }))
    }

    async fn delete_account(&self, ctx: RequestContext, _request: ServiceRequest<'_, DeleteAccountRequest>) -> ServiceResult<DeleteAccountResponse> {
        let ctx = request_ctx(&ctx);
        let msg = messages::DeleteAccountRequest { caller: caller(&ctx).into_connect()? };
        let d = self.sender.send_delete_account_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(DeleteAccountResponse { purge_id: d.purge_id, ..Default::default() }))
    }

    async fn get_group(&self, ctx: RequestContext, request: ServiceRequest<'_, GetGroupRequest>) -> ServiceResult<GetGroupResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::GetGroupRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let g = self.sender.send_get_group_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(GetGroupResponse { group: Some(group(g)).into(), ..Default::default() }))
    }

    async fn list_groups(&self, ctx: RequestContext, _request: ServiceRequest<'_, ListGroupsRequest>) -> ServiceResult<ListGroupsResponse> {
        let ctx = request_ctx(&ctx);
        let msg = messages::ListGroupsRequest { caller: caller(&ctx).into_connect()? };
        let list = self.sender.send_list_groups_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(ListGroupsResponse { groups: list.groups.into_iter().map(group).collect(), ..Default::default() }))
    }

    async fn create_invite(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateInviteRequest>) -> ServiceResult<CreateInviteResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::CreateInviteRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            role: role_field(&req.role).into_connect()?,
            expires_at: time_field(req.expires_at_ms, "expires_at_ms").into_connect()?,
            max_uses: req.max_uses,
        };
        let i = self.sender.send_create_invite_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(CreateInviteResponse { invite: Some(invite(i)).into(), ..Default::default() }))
    }

    async fn accept_invite(&self, ctx: RequestContext, request: ServiceRequest<'_, AcceptInviteRequest>) -> ServiceResult<AcceptInviteResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        if req.code.is_empty() {
            return Err(into_connect_error(AppError::invalid_argument("code: empty")));
        }
        let msg = messages::AcceptInviteRequest { caller: caller(&ctx).into_connect()?, code: req.code };
        let m = self.sender.send_accept_invite_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(AcceptInviteResponse { membership: Some(membership(m)).into(), ..Default::default() }))
    }

    async fn list_members(&self, ctx: RequestContext, request: ServiceRequest<'_, ListMembersRequest>) -> ServiceResult<ListMembersResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::ListMembersRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let list = self.sender.send_list_members_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(ListMembersResponse { members: list.members.into_iter().map(membership).collect(), ..Default::default() }))
    }

    async fn add_member(&self, ctx: RequestContext, request: ServiceRequest<'_, AddMemberRequest>) -> ServiceResult<AddMemberResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::AddMemberRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            user_id: uuid_field(&req.user_id, "user_id").into_connect()?,
            role: role_field(&req.role).into_connect()?,
        };
        let m = self.sender.send_add_member_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(AddMemberResponse { membership: Some(membership(m)).into(), ..Default::default() }))
    }

    async fn remove_member(&self, ctx: RequestContext, request: ServiceRequest<'_, RemoveMemberRequest>) -> ServiceResult<RemoveMemberResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::RemoveMemberRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            user_id: uuid_field(&req.user_id, "user_id").into_connect()?,
        };
        let m = self.sender.send_remove_member_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(RemoveMemberResponse { membership: Some(membership(m)).into(), ..Default::default() }))
    }

    async fn set_member_role(&self, ctx: RequestContext, request: ServiceRequest<'_, SetMemberRoleRequest>) -> ServiceResult<SetMemberRoleResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::SetMemberRoleRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            user_id: uuid_field(&req.user_id, "user_id").into_connect()?,
            role: role_field(&req.role).into_connect()?,
        };
        let m = self.sender.send_set_member_role_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(SetMemberRoleResponse { membership: Some(membership(m)).into(), ..Default::default() }))
    }

    async fn leave_group(&self, ctx: RequestContext, request: ServiceRequest<'_, LeaveGroupRequest>) -> ServiceResult<LeaveGroupResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::LeaveGroupRequest {
            caller: caller(&ctx).into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let m = self.sender.send_leave_group_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(LeaveGroupResponse { membership: Some(membership(m)).into(), ..Default::default() }))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(GroupServiceImpl { sender: ApiSender::new(router) }));
}
