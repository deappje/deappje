//! The external-effect adapters: one validated `Call` per external call
//! this nanoservice makes, with the strategy the plan declared (the
//! Directive §6). `send` is the ONE dispatch path; a raw provider call
//! anywhere else fails review. Audited one by one in tests/effects_audit.rs.

use std::sync::Arc;
use std::time::Duration;

#[allow(unused_imports)]
use basable_externaleffect::{
    Adapter, Call, Classifier, DeclaredResolution, LateCall, SlotIdentity, Strategy,
    intent_age_fn, key_fn, lookup_fn, provider_id_fn, resolve_by_lookup, resolve_keyed_fn,
    send_fn,
};

use crate::provider::{HttpProvider, NoopProvider, Provider};

/// The validated adapter set, built once at construction. Each call is
/// shared (`Arc`) so the audit can hold the very handle production
/// dispatches through.
pub struct Calls {
    pub delete_object: Arc<Call<DeleteObjectArgs, DeleteObjectArgs, DeleteObjectResult>>,
}

/// Arguments of `delete_object` (object_storage). Everything `key` and `send`
/// derive from must be here, durable before the send.
#[derive(Debug, Clone)]
pub struct DeleteObjectArgs {
    /// The deterministic identity of this effect (never a retry count).
    pub key: String,
    // TODO: the payload.
}

/// What `delete_object` returns.
#[derive(Debug, Clone)]
pub struct DeleteObjectResult {
    /// The provider's id for what landed (its receipt, its object id).
    pub provider_id: String,
    // TODO: the rest of the provider's answer.
}

impl Calls {
    /// Picks the provider from the environment, once, at construction: the
    /// HTTP client when the credential is set, the noop otherwise.
    pub fn from_env() -> Self {
        match std::env::var("STORAGE_PROVIDER_TOKEN") {
            Ok(token) if !token.is_empty() => Calls::new(Arc::new(HttpProvider {
                base_url: std::env::var("STORAGE_PROVIDER_URL").unwrap_or_default(),
                client: reqwest::Client::new(),
                token,
            })),
            _ => Calls::new(Arc::new(NoopProvider)),
        }
    }

    /// Builds every adapter over one provider; the tests pass the simulator.
    pub fn new<P: Provider>(provider: Arc<P>) -> Self {
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
                    irreversible: false,
                    late_call: LateCall::Convergent,
                    strategy: Strategy::Idempotent,
                })
                .expect("storage.delete_object: invalid adapter"),
            ),
        }
    }
}
