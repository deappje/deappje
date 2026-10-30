//! The external-effect adapters: one validated `Call` per external call
//! this nanoservice makes, with the strategy the plan declared (the
//! Directive §6). `send` is the ONE dispatch path; a raw provider call
//! anywhere else fails review. Audited one by one in tests/effects_audit.rs.

use std::sync::Arc;
use std::time::Duration;

use basable_externaleffect::{Adapter, Call, Classifier, LateCall, LookupFn, Strategy, key_fn, lookup_fn, send_fn};
use uuid::Uuid;

use crate::provider::{HttpProvider, NoopProvider, Provider};

/// The validated adapter set, built once at construction. Each call is
/// shared (`Arc`) so the audit can hold the very handle production
/// dispatches through.
pub struct Calls {
    /// Kratos admin `DELETE /admin/identities/{id}`: idempotent, a 404 on a
    /// repeat is success. Dispatched from the `user_purge` pass under its
    /// claim, never from a request.
    pub delete_identity: Arc<Call<DeleteIdentityArgs, DeleteIdentityArgs, DeleteIdentityResult>>,
    /// The read beside the call, not an adapter (it sends nothing): whether
    /// the identity still exists, `Some(())` when it does. The pass confirms
    /// the deletion with it on its next claim instead of trusting the ack.
    pub identity_exists: LookupFn<Uuid, ()>,
}

/// Arguments of `delete_identity` (kratos). Everything `key` and `send`
/// derive from must be here, durable before the send: the Kratos identity
/// id, which is the user id everywhere in this application.
#[derive(Debug, Clone)]
pub struct DeleteIdentityArgs {
    pub user_id: Uuid,
}

impl DeleteIdentityArgs {
    /// The deterministic identity of this effect: the user, never a retry
    /// count.
    pub fn key(&self) -> String {
        self.user_id.to_string()
    }
}

/// What `delete_identity` returns.
#[derive(Debug, Clone)]
pub struct DeleteIdentityResult {
    /// The identity was already gone when the call arrived (a 404): the
    /// outcome the caller wanted either way.
    pub already_absent: bool,
}

impl Calls {
    /// Picks the provider from the environment, once, at construction: the
    /// HTTP client when `KRATOS_ADMIN_URL` names Kratos's admin API (the
    /// in-cluster service, no secret), the noop otherwise.
    pub fn from_env() -> Self {
        match std::env::var("KRATOS_ADMIN_URL") {
            Ok(url) if !url.is_empty() => Calls::new(Arc::new(HttpProvider {
                base_url: url.trim_end_matches('/').to_owned(),
                client: reqwest::Client::new(),
            })),
            _ => Calls::new(Arc::new(NoopProvider)),
        }
    }

    /// Builds every adapter over one provider; the tests pass the simulator.
    pub fn new<P: Provider>(provider: Arc<P>) -> Self {
        let lookup = Arc::clone(&provider);
        Self {
            delete_identity: Arc::new(
                Call::new(Adapter {
                    operation: "delete_identity".into(),
                    key: Some(key_fn(|a: &DeleteIdentityArgs| a.key())),
                    send: {
                        let p = Arc::clone(&provider);
                        send_fn(move |ctx, args: DeleteIdentityArgs| {
                            let p = Arc::clone(&p);
                            async move { p.delete_identity(ctx, args).await }
                        })
                    },
                    classify: Some(Classifier::transport()),
                    call_timeout: Duration::from_secs(30),
                    // Destructive, but it is exactly what the user asked for,
                    // and the receiver converges: a repeat finds nothing.
                    irreversible: false,
                    late_call: LateCall::Convergent,
                    strategy: Strategy::Idempotent,
                })
                .expect("group.delete_identity: invalid adapter"),
            ),
            identity_exists: lookup_fn(move |ctx, user_id: Uuid| {
                let p = Arc::clone(&lookup);
                async move { Ok(p.identity_exists(ctx, user_id).await?.then_some(())) }
            }),
        }
    }
}
