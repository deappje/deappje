//! `RelayService`: the Connect service of `relay`. Each method takes the
//! caller from the validated session, converts the buffa message into the
//! plain `messages` struct one to one (`value` passes through untouched),
//! sends it through the api sender, and converts the answer back. Errors
//! map to Connect codes through basable_connect.

use std::sync::Arc;

use basable_connect::{ConnectRouter, IntoConnect, request_ctx};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use interfaces::{ApiRoutes, ApiSender};
use proto::connect::relay::v1::RelayService;
use proto::proto::relay::v1::*;

use super::{caller, millis, optional_millis, optional_uuid_field, uuid_field};
use crate::Api;

/// The service holds the api sender (exactly the sends `api` declares),
/// built once from the router at mount.
pub struct RelayServiceImpl<R: ApiRoutes + Send + Sync + 'static> {
    sender: ApiSender<'static, R>,
}

fn entry_out(e: messages::Entry) -> Entry {
    Entry {
        id: e.id.to_string(),
        value: e.value.unwrap_or_default(),
        seq: e.seq,
        deleted: e.deleted,
        ..Default::default()
    }
}

fn entry_in(e: Entry) -> Result<messages::Entry, basable_connect::connectrpc::ConnectError> {
    Ok(messages::Entry {
        id: uuid_field(&e.id, "entries.id").into_connect()?,
        value: if e.value.is_empty() { None } else { Some(e.value) },
        seq: e.seq,
        deleted: e.deleted,
    })
}

fn history(r: messages::HistoryRequest) -> HistoryRequest {
    HistoryRequest {
        id: if r.id.is_nil() { String::new() } else { r.id.to_string() },
        group_id: r.group_id.to_string(),
        requester_device: r.requester_device.to_string(),
        from_seq: r.from_seq,
        to_seq: r.to_seq,
        object_id: r.object_id.map(|id| id.to_string()).unwrap_or_default(),
        claimed_by: r.claimed_by.map(|id| id.to_string()).unwrap_or_default(),
        claimed_at_ms: optional_millis(r.claimed_at),
        created_at_ms: millis(r.created_at),
        status: r.status.as_str().to_owned(),
        through_seq: r.through_seq,
        ..Default::default()
    }
}

// The generated trait returns `impl Encodable`; answering with the owned
// message is the refinement this allow names.
#[allow(refining_impl_trait)]
impl<R: ApiRoutes + Send + Sync + 'static> RelayService for RelayServiceImpl<R> {
    async fn push_objects(&self, ctx: RequestContext, request: ServiceRequest<'_, PushObjectsRequest>) -> ServiceResult<PushObjectsResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let mut objects = Vec::with_capacity(req.objects.len());
        for o in req.objects {
            objects.push(messages::PushEntry {
                id: uuid_field(&o.id, "objects.id").into_connect()?,
                value: if o.value.is_empty() { None } else { Some(o.value) },
                base_seq: o.base_seq,
                deleted: o.deleted,
            });
        }
        let msg = messages::PushObjectsRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            objects,
        };
        let out = match self.sender.send_push_objects_request(&ctx, msg).await.into_connect()? {
            messages::PushResult::Accepted { accepted, conflicts, head_seq } => PushObjectsResponse {
                accepted: accepted.into_iter().map(|a| Accepted { id: a.id.to_string(), seq: a.seq, ..Default::default() }).collect(),
                conflicts: conflicts
                    .into_iter()
                    .map(|c| Conflict { id: c.id.to_string(), current: Some(entry_out(c.current)).into(), ..Default::default() })
                    .collect(),
                head_seq,
                ..Default::default()
            },
            messages::PushResult::BehindFloor { evicted_through_seq } => PushObjectsResponse {
                behind_floor: true,
                evicted_through_seq,
                ..Default::default()
            },
        };
        Ok(Response::new(out))
    }

    async fn pull_changes(&self, ctx: RequestContext, request: ServiceRequest<'_, PullChangesRequest>) -> ServiceResult<PullChangesResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::PullChangesRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            since_seq: req.since_seq,
            limit: req.limit,
        };
        let set = self.sender.send_pull_changes_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(PullChangesResponse {
            entries: set.entries.into_iter().map(entry_out).collect(),
            head_seq: set.head_seq,
            evicted_through_seq: set.evicted_through_seq,
            min_acked_seq: set.min_acked_seq.unwrap_or_default(),
            has_min_acked_seq: set.min_acked_seq.is_some(),
            more: set.more,
            ..Default::default()
        }))
    }

    async fn ack_seq(&self, ctx: RequestContext, request: ServiceRequest<'_, AckSeqRequest>) -> ServiceResult<AckSeqResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::AckSeqRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            seq: req.seq,
        };
        let r = self.sender.send_ack_seq_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(AckSeqResponse {
            evicted_through_seq: r.evicted_through_seq,
            min_acked_seq: r.min_acked_seq.unwrap_or_default(),
            has_min_acked_seq: r.min_acked_seq.is_some(),
            ..Default::default()
        }))
    }

    async fn get_heads(&self, ctx: RequestContext, request: ServiceRequest<'_, GetHeadsRequest>) -> ServiceResult<GetHeadsResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let mut group_ids = Vec::with_capacity(req.group_ids.len());
        for id in &req.group_ids {
            group_ids.push(uuid_field(id, "group_ids").into_connect()?);
        }
        let msg = messages::GetHeadsRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_ids,
        };
        let list = self.sender.send_get_heads_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(GetHeadsResponse {
            heads: list
                .heads
                .into_iter()
                .map(|h| Head {
                    group_id: h.group_id.to_string(),
                    head_seq: h.head_seq,
                    evicted_through_seq: h.evicted_through_seq,
                    min_acked_seq: h.min_acked_seq.unwrap_or_default(),
                    has_min_acked_seq: h.min_acked_seq.is_some(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
    }

    async fn request_history(&self, ctx: RequestContext, request: ServiceRequest<'_, RequestHistoryRequest>) -> ServiceResult<RequestHistoryResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::RequestHistoryRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
            from_seq: req.from_seq,
            to_seq: req.to_seq,
            object_id: optional_uuid_field(&req.object_id, "object_id").into_connect()?,
        };
        let r = self.sender.send_request_history_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(RequestHistoryResponse { request: Some(history(r)).into(), ..Default::default() }))
    }

    async fn get_history_request(&self, ctx: RequestContext, request: ServiceRequest<'_, GetHistoryRequestRequest>) -> ServiceResult<GetHistoryRequestResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::GetHistoryRequestRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            request_id: uuid_field(&req.request_id, "request_id").into_connect()?,
        };
        let r = self.sender.send_get_history_request_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(GetHistoryRequestResponse { request: Some(history(r)).into(), ..Default::default() }))
    }

    async fn list_history_requests(&self, ctx: RequestContext, request: ServiceRequest<'_, ListHistoryRequestsRequest>) -> ServiceResult<ListHistoryRequestsResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::ListHistoryRequestsRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            group_id: uuid_field(&req.group_id, "group_id").into_connect()?,
        };
        let list = self.sender.send_list_history_requests_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(ListHistoryRequestsResponse { requests: list.requests.into_iter().map(history).collect(), ..Default::default() }))
    }

    async fn claim_history(&self, ctx: RequestContext, request: ServiceRequest<'_, ClaimHistoryRequest>) -> ServiceResult<ClaimHistoryResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let msg = messages::ClaimHistoryRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            request_id: uuid_field(&req.request_id, "request_id").into_connect()?,
        };
        let r = self.sender.send_claim_history_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(ClaimHistoryResponse { request: Some(history(r)).into(), ..Default::default() }))
    }

    async fn push_history(&self, ctx: RequestContext, request: ServiceRequest<'_, PushHistoryRequest>) -> ServiceResult<PushHistoryResponse> {
        let ctx = request_ctx(&ctx);
        let req = request.to_owned_message();
        let mut entries = Vec::with_capacity(req.entries.len());
        for e in req.entries {
            entries.push(entry_in(e)?);
        }
        let msg = messages::PushHistoryRequest {
            caller: caller(&ctx).into_connect()?,
            device_id: uuid_field(&req.device_id, "device_id").into_connect()?,
            request_id: uuid_field(&req.request_id, "request_id").into_connect()?,
            entries,
            complete: req.complete,
        };
        let r = self.sender.send_push_history_request(&ctx, msg).await.into_connect()?;
        Ok(Response::new(PushHistoryResponse {
            inserted: r.inserted,
            skipped: r.skipped,
            through_seq: r.through_seq,
            complete: r.complete,
            ..Default::default()
        }))
    }
}

pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, router: &'static R)
where
    R: ApiRoutes + Send + Sync + 'static,
{
    connect.add_service(Arc::new(RelayServiceImpl { sender: ApiSender::new(router) }));
}
