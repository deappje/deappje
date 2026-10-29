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
    pub send_apns: Arc<Call<SendApnsArgs, SendApnsArgs, SendApnsResult>>,
    pub send_fcm: Arc<Call<SendFcmArgs, SendFcmArgs, SendFcmResult>>,
}

/// Arguments of `send_apns` (apns). Everything `key` and `send`
/// derive from must be here, durable before the send.
#[derive(Debug, Clone)]
pub struct SendApnsArgs {
    /// The deterministic identity of this effect (never a retry count).
    pub key: String,
    // TODO: the payload.
}

/// What `send_apns` returns.
#[derive(Debug, Clone)]
pub struct SendApnsResult {
    /// The provider's id for what landed (its receipt, its object id).
    pub provider_id: String,
    // TODO: the rest of the provider's answer.
}

/// Arguments of `send_fcm` (fcm). Everything `key` and `send`
/// derive from must be here, durable before the send.
#[derive(Debug, Clone)]
pub struct SendFcmArgs {
    /// The deterministic identity of this effect (never a retry count).
    pub key: String,
    // TODO: the payload.
}

/// What `send_fcm` returns.
#[derive(Debug, Clone)]
pub struct SendFcmResult {
    /// The provider's id for what landed (its receipt, its object id).
    pub provider_id: String,
    // TODO: the rest of the provider's answer.
}

impl Calls {
    /// Picks the provider from the environment, once, at construction: the
    /// HTTP client when the credential is set, the noop otherwise.
    pub fn from_env() -> Self {
        match std::env::var("DEVICE_PROVIDER_TOKEN") {
            Ok(token) if !token.is_empty() => Calls::new(Arc::new(HttpProvider {
                base_url: std::env::var("DEVICE_PROVIDER_URL").unwrap_or_default(),
                client: reqwest::Client::new(),
                token,
            })),
            _ => Calls::new(Arc::new(NoopProvider)),
        }
    }

    /// Builds every adapter over one provider; the tests pass the simulator.
    pub fn new<P: Provider>(provider: Arc<P>) -> Self {
        Self {
            send_apns: Arc::new(
                Call::new(Adapter {
                    operation: "send_apns".into(),
                    key: Some(key_fn(|a: &SendApnsArgs| a.key.clone())),
                    send: {
                        let p = Arc::clone(&provider);
                        send_fn(move |ctx, args: SendApnsArgs| {
                            let p = Arc::clone(&p);
                            async move { p.send_apns(ctx, args).await }
                        })
                    },
                    classify: Some(Classifier::transport()),
                    call_timeout: Duration::from_secs(30),
                    irreversible: false,
                    late_call: LateCall::Convergent,
                    strategy: Strategy::Idempotent,
                })
                .expect("device.send_apns: invalid adapter"),
            ),
            send_fcm: Arc::new(
                Call::new(Adapter {
                    operation: "send_fcm".into(),
                    key: Some(key_fn(|a: &SendFcmArgs| a.key.clone())),
                    send: {
                        let p = Arc::clone(&provider);
                        send_fn(move |ctx, args: SendFcmArgs| {
                            let p = Arc::clone(&p);
                            async move { p.send_fcm(ctx, args).await }
                        })
                    },
                    classify: Some(Classifier::transport()),
                    call_timeout: Duration::from_secs(30),
                    irreversible: false,
                    late_call: LateCall::Convergent,
                    strategy: Strategy::Idempotent,
                })
                .expect("device.send_fcm: invalid adapter"),
            ),
        }
    }
}
