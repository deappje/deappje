//! The external-effect adapters: one validated `Call` per external call
//! this nanoservice makes, with the strategy the plan declared (the
//! Directive §6). `send` is the ONE dispatch path; a raw provider call
//! anywhere else fails review. Audited one by one in tests/effects_audit.rs.
//!
//! Beside the one adapter, two reads that are not effects: a prefix
//! listing (a request that changes nothing) and presigning (a computation
//! over the credentials that sends nothing at all).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use basable_core::{BoxError, Ctx};
use basable_externaleffect::{Adapter, Call, Classifier, LateCall, Strategy, key_fn, send_fn};
use chrono::{DateTime, Utc};

use crate::provider::{Provider, S3Client, Unconfigured};

/// The validated adapter set, built once at construction. Each call is
/// shared (`Arc`) so the audit can hold the very handle production
/// dispatches through.
pub struct Calls {
    /// S3 `DeleteObject`: idempotent, a missing key succeeds, a re-send
    /// converges. Dispatched `Unfenced` from the handlers (no claim).
    pub delete_object: Arc<Call<DeleteObjectArgs, DeleteObjectArgs, DeleteObjectResult>>,
    /// The keys under a prefix, at most `limit`: a read.
    pub list_keys: ListFn,
    /// A presigned URL: a computation, no request.
    pub presign: PresignFn,
}

pub type ListFn = Box<dyn Fn(Ctx, String, usize) -> Pin<Box<dyn Future<Output = Result<Vec<String>, BoxError>> + Send + 'static>> + Send + Sync>;
pub type PresignFn = Box<dyn Fn(PresignArgs) -> Result<String, BoxError> + Send + Sync>;

/// Arguments of `delete_object` (object_storage): the key is the effect's
/// whole identity.
#[derive(Debug, Clone)]
pub struct DeleteObjectArgs {
    pub key: String,
}

/// What `delete_object` returns: nothing but the key, the bucket answers
/// the same for a key that was there and one that was not.
#[derive(Debug, Clone)]
pub struct DeleteObjectResult {
    pub key: String,
}

/// What to presign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresignMethod {
    /// A PUT of exactly `size` bytes.
    Put { size: i64 },
    Get,
}

#[derive(Debug, Clone)]
pub struct PresignArgs {
    pub method: PresignMethod,
    pub key: String,
    pub expires: Duration,
    pub now: DateTime<Utc>,
}

impl Calls {
    /// Picks the provider from the environment, once, at construction: the
    /// S3 client over the `object-storage` Secret's variables
    /// (`AWS_ENDPOINT_URL_S3`, `AWS_REGION`, `S3_BUCKET`, `AWS_ACCESS_KEY_ID`,
    /// `AWS_SECRET_ACCESS_KEY`, `S3_PRIVATE_URL`), or, without them, a
    /// provider that refuses everything.
    pub fn from_env() -> Self {
        match S3Client::from_env() {
            Ok(Some(client)) => Calls::new(Arc::new(client)),
            Ok(None) => {
                tracing::warn!("object storage not configured: no file can be uploaded or downloaded");
                Calls::new(Arc::new(Unconfigured))
            }
            Err(e) => {
                tracing::error!(error = %e, "object storage misconfigured: no file can be uploaded or downloaded");
                Calls::new(Arc::new(Unconfigured))
            }
        }
    }

    /// Builds every adapter over one provider; the tests pass the simulator.
    pub fn new<P: Provider>(provider: Arc<P>) -> Self {
        let lister = Arc::clone(&provider);
        let presigner = Arc::clone(&provider);
        Self {
            delete_object: Arc::new(
                Call::new(Adapter {
                    operation: "delete_object".into(),
                    key: Some(key_fn(|a: &DeleteObjectArgs| a.key.clone())),
                    send: {
                        let p = Arc::clone(&provider);
                        send_fn(move |ctx, args: DeleteObjectArgs| {
                            let p = Arc::clone(&p);
                            async move { p.delete_object(ctx, args).await }
                        })
                    },
                    classify: Some(Classifier::transport()),
                    call_timeout: Duration::from_secs(30),
                    // Member devices have the bytes: a file is a cache entry.
                    irreversible: false,
                    late_call: LateCall::Convergent,
                    strategy: Strategy::Idempotent,
                })
                .expect("storage.delete_object: invalid adapter"),
            ),
            list_keys: Box::new(move |ctx, prefix, limit| {
                let p = Arc::clone(&lister);
                Box::pin(async move { p.list_keys(ctx, prefix, limit).await })
            }),
            presign: Box::new(move |args| presigner.presign(args)),
        }
    }
}
