//! The external-effect adapters: one validated `Call` per external call
//! this nanoservice makes, with the strategy the plan declared (the
//! Directive §6). `send` is the ONE dispatch path; a raw provider call
//! anywhere else fails review. Audited one by one in tests/effects_audit.rs.
//!
//! Both pushes are content-less wake-ups (`push.rs`): a duplicate costs one
//! extra wake and nothing a person sees, so both are `Idempotent` and
//! dispatched `Unfenced` from the request path, which holds no claim.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use basable_externaleffect::{Adapter, Call, Classifier, LateCall, Strategy, key_fn, send_fn};

use crate::provider::{ApnsClient, FcmClient, Provider, Providers};

/// The validated adapter set, built once at construction. Each call is
/// shared (`Arc`) so the audit can hold the very handle production
/// dispatches through.
pub struct Calls {
    pub send_apns: Arc<Call<SendApnsArgs, SendApnsArgs, SendApnsResult>>,
    pub send_fcm: Arc<Call<SendFcmArgs, SendFcmArgs, SendFcmResult>>,
}

/// Arguments of `send_apns` (apns). Everything `key` and `send` derive
/// from is here: the device token and the wake-up's payload.
#[derive(Debug, Clone)]
pub struct SendApnsArgs {
    /// The deterministic identity of this effect (`Wakeup::key`): the
    /// event and the device, never a retry count.
    pub key: String,
    /// The device's APNs token, hex as the device registered it.
    pub token: String,
    /// The wake-up's payload (`Wakeup::data`).
    pub data: BTreeMap<String, String>,
}

/// What `send_apns` returns.
#[derive(Debug, Clone)]
pub struct SendApnsResult {
    /// APNs's id for the notification (`apns-id`).
    pub apns_id: String,
}

/// Arguments of `send_fcm` (fcm).
#[derive(Debug, Clone)]
pub struct SendFcmArgs {
    /// The deterministic identity of this effect (`Wakeup::key`).
    pub key: String,
    /// The device's FCM registration token.
    pub token: String,
    /// The wake-up's payload (`Wakeup::data`), FCM's `data` map.
    pub data: BTreeMap<String, String>,
}

/// What `send_fcm` returns.
#[derive(Debug, Clone)]
pub struct SendFcmResult {
    /// The message's resource name (`projects/*/messages/*`).
    pub message_name: String,
}

impl Calls {
    /// Picks the providers from the environment, once, at construction:
    /// APNs when the four `APNS_*` variables are set, FCM when
    /// `FCM_SERVICE_ACCOUNT_JSON` is (`k8s/app/deployment.yaml`, each from
    /// the `app-secrets` Secret); a service without its secret refuses
    /// every send, which `push.rs` logs and drops.
    pub fn from_env() -> Self {
        let apns = match ApnsClient::from_env() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "APNs not configured: iOS devices are not woken");
                None
            }
        };
        let fcm = match FcmClient::from_env() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "FCM not configured: Android devices are not woken");
                None
            }
        };
        Calls::new(Arc::new(Providers { apns, fcm }))
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
