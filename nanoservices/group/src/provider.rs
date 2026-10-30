//! The provider seam: the narrow trait `effects.rs` sends through, the real
//! HTTP client, and the noop for environments without Kratos. A provider
//! answers with a `BoxError` that carries its own verdict on its own call:
//! `definitive(..)` for a parsed 4xx — the receiver's structured answer,
//! and the proof of non-execution an irreversible adapter needs — and
//! `TransportError::wrap(..)` for everything whose outcome is unknowable (a
//! transport error, a 5xx, a timeout), which every classifier holds
//! ambiguous. A plain error is neither: the reversible classifier
//! (`Classifier::transport`) takes it as the receiver's answer, the
//! irreversible one (`Classifier::fail_closed_on_definitive`) holds it
//! ambiguous, so never return one for an outcome you cannot know.

use std::future::Future;

use basable_core::{BoxError, Ctx};
use basable_externaleffect::{TransportError, definitive};
use reqwest::StatusCode;
use uuid::Uuid;

use crate::effects::*;

/// Exactly the calls this nanoservice makes — never a wrapper around the
/// whole provider API. The futures are `Send` because a worker dispatches
/// them.
pub trait Provider: Send + Sync + 'static {
    /// Deletes the identity. Absent already is success.
    fn delete_identity(&self, ctx: Ctx, args: DeleteIdentityArgs) -> impl Future<Output = Result<DeleteIdentityResult, BoxError>> + Send;
    /// Whether the identity exists: the read the pass confirms a deletion
    /// with.
    fn identity_exists(&self, ctx: Ctx, user_id: Uuid) -> impl Future<Output = Result<bool, BoxError>> + Send;
}

/// The Kratos admin API over HTTP, reached at the in-cluster service
/// address (`k8s/kratos/service.yaml`, `kratos-admin:4434`); the admin API
/// carries no credential of its own.
pub struct HttpProvider {
    pub base_url: String,
    pub client: reqwest::Client,
}

impl HttpProvider {
    fn identity_url(&self, user_id: Uuid) -> String {
        format!("{}/admin/identities/{user_id}", self.base_url)
    }
}

/// The verdict on a status Kratos answered with, for a call whose success
/// statuses the caller already handled: a 5xx is unknowable, any other
/// status is the receiver's answer.
fn refused(what: &str, status: StatusCode, body: String) -> BoxError {
    let message = format!("kratos {what}: {status}: {body}");
    if status.is_server_error() {
        TransportError::wrap(message)
    } else {
        definitive(message)
    }
}

impl Provider for HttpProvider {
    async fn delete_identity(&self, _ctx: Ctx, args: DeleteIdentityArgs) -> Result<DeleteIdentityResult, BoxError> {
        let response = self
            .client
            .delete(self.identity_url(args.user_id))
            .send()
            .await
            .map_err(TransportError::wrap)?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::OK => Ok(DeleteIdentityResult { already_absent: false }),
            StatusCode::NOT_FOUND => Ok(DeleteIdentityResult { already_absent: true }),
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(refused("delete identity", status, body))
            }
        }
    }

    async fn identity_exists(&self, _ctx: Ctx, user_id: Uuid) -> Result<bool, BoxError> {
        let response = self
            .client
            .get(self.identity_url(user_id))
            .send()
            .await
            .map_err(TransportError::wrap)?;
        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(refused("get identity", status, body))
            }
        }
    }
}

/// No Kratos configured (local dev): every call is refused, so nothing is
/// sent and nothing is ambiguous, and an account purge waits at its
/// identity step until there is one.
pub struct NoopProvider;

impl Provider for NoopProvider {
    async fn delete_identity(&self, _ctx: Ctx, _args: DeleteIdentityArgs) -> Result<DeleteIdentityResult, BoxError> {
        Err(definitive("group.delete_identity: KRATOS_ADMIN_URL not configured"))
    }

    async fn identity_exists(&self, _ctx: Ctx, _user_id: Uuid) -> Result<bool, BoxError> {
        Err("group.identity_exists: KRATOS_ADMIN_URL not configured".into())
    }
}
