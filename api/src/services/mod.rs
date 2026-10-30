//! One module per nanoservice that exposes a Connect service. The scaffolder
//! adds `pub mod <name>;` between the markers and each module's `mount`
//! registers its service; the agent fills the field mapping inside.

use basable_auth::AuthCtx;
use basable_connect::ConnectRouter;
use basable_core::{AppError, Ctx};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::Api;

/// The caller: the validated session's Kratos identity id, which is the
/// user id everywhere in this application. The auth layer refused the
/// request before a handler without one could run, so a missing identity
/// is a wiring fault, answered as unauthenticated all the same.
pub(crate) fn caller(ctx: &Ctx) -> Result<Uuid, AppError> {
    let id = ctx.user_id().ok_or_else(|| AppError::unauthenticated("no session"))?;
    Uuid::parse_str(id).map_err(|_| AppError::unauthenticated("the session's identity is not a uuid"))
}

/// A UUID field of a request; `what` names it in the error.
pub(crate) fn uuid_field(value: &str, what: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value).map_err(|_| AppError::invalid_argument(format!("{what}: not a uuid")))
}

/// An optional UUID field: empty is none.
pub(crate) fn optional_uuid_field(value: &str, what: &str) -> Result<Option<Uuid>, AppError> {
    if value.is_empty() { Ok(None) } else { uuid_field(value, what).map(Some) }
}

/// A time on the wire: milliseconds since the Unix epoch, UTC.
pub(crate) fn millis(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

/// An optional time on the wire: 0 is unset.
pub(crate) fn optional_millis(t: Option<DateTime<Utc>>) -> i64 {
    t.map_or(0, millis)
}

/// A time from the wire; `what` names the field in the error.
pub(crate) fn time_field(ms: i64, what: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::<Utc>::from_timestamp_millis(ms).ok_or_else(|| AppError::invalid_argument(format!("{what}: not a time")))
}

// basable:services-begin
pub mod group;
pub mod device;
pub mod relay;
pub mod storage;
// basable:services-end

/// Mounts every service module's Connect service.
pub fn mount_all<R>(connect: &mut ConnectRouter, api: &'static Api, router: &'static R)
where
    R: interfaces::ApiRoutes + Send + Sync + 'static,
{
    health::mount(connect, api, router);
    // basable:mount-begin
    group::mount(connect, api, router);
    device::mount(connect, api, router);
    relay::mount(connect, api, router);
    storage::mount(connect, api, router);
    // basable:mount-end
}

/// The health service: the Connect face of `/readyz`, answered once the
/// app finished wiring (a request cannot arrive before that).
pub mod health {
    use std::sync::Arc;

    use basable_connect::ConnectRouter;
    use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
    use proto::connect::health::v1::HealthService;
    use proto::proto::health::v1::{CheckRequest, CheckResponse};

    use crate::Api;

    pub struct Health;

    #[allow(refining_impl_trait)]
    impl HealthService for Health {
        async fn check(&self, _ctx: RequestContext, _request: ServiceRequest<'_, CheckRequest>) -> ServiceResult<CheckResponse> {
            Ok(Response::new(CheckResponse {
                ready: true,
                ..Default::default()
            }))
        }
    }

    pub fn mount<R>(connect: &mut ConnectRouter, _api: &'static Api, _router: &'static R)
    where
        R: interfaces::ApiRoutes + Send + Sync + 'static,
    {
        connect.add_service(Arc::new(Health));
    }
}
