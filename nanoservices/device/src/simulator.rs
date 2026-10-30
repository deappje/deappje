//! The in-memory simulator of APNs and FCM: records every wake-up with
//! its token and payload, idempotent by key for the audit's count,
//! injects faults. Drives the effect audit and the integration tests.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use basable_core::{BoxError, Ctx};
use basable_externaleffect::{TransportError, definitive};

use crate::effects::*;
use crate::provider::{Provider, Unregistered};

#[derive(Default)]
pub struct Simulator {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The distinct keys that landed, per operation.
    landed: HashMap<&'static str, HashSet<String>>,
    /// Every push that landed, in order: the operation, the token and the
    /// payload.
    pushes: Vec<Push>,
    /// Per-operation fault: the next N calls fail this way.
    faults: HashMap<&'static str, (Fault, u32)>,
}

/// One wake-up as the simulator received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    pub op: &'static str,
    pub token: String,
    pub data: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Fault {
    /// A transport failure after the write landed (ack loss): a
    /// `TransportError`, which every classifier holds ambiguous.
    AckLoss,
    /// A structured refusal before anything landed: `definitive(..)`.
    Refused,
    /// The token is dead: `definitive(Unregistered)`.
    Unregistered,
}

impl Simulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many distinct keys landed for an operation.
    pub fn landed_count(&self, op: &'static str) -> usize {
        self.lock().landed.get(op).map_or(0, HashSet::len)
    }

    /// Whether `key` landed for an operation.
    pub fn landed(&self, op: &'static str, key: &str) -> bool {
        self.lock().landed.get(op).is_some_and(|k| k.contains(key))
    }

    /// Every push that landed, in order.
    pub fn pushes(&self) -> Vec<Push> {
        self.lock().pushes.clone()
    }

    /// The pushes that landed on one token, in order.
    pub fn pushes_to(&self, token: &str) -> Vec<Push> {
        self.lock().pushes.iter().filter(|p| p.token == token).cloned().collect()
    }

    pub fn inject(&self, op: &'static str, fault: Fault, times: u32) {
        self.lock().faults.insert(op, (fault, times));
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock means a test panicked mid-call; the state is still
        // worth reading for the assertion that follows.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn apply(&self, op: &'static str, key: &str, token: &str, data: &BTreeMap<String, String>) -> Result<(), BoxError> {
        let mut inner = self.lock();
        let fault = match inner.faults.get_mut(op) {
            Some((f, n)) if *n > 0 => {
                *n -= 1;
                Some(*f)
            }
            _ => None,
        };
        match fault {
            Some(Fault::Refused) => return Err(definitive(format!("{op}: refused"))),
            Some(Fault::Unregistered) => return Err(definitive(Unregistered)),
            _ => {}
        }
        inner.landed.entry(op).or_default().insert(key.to_owned());
        inner.pushes.push(Push { op, token: token.to_owned(), data: data.clone() });
        if let Some(Fault::AckLoss) = fault {
            return Err(TransportError::wrap(format!("{op}: connection reset after the write")));
        }
        Ok(())
    }
}

impl Provider for Simulator {
    async fn send_apns(&self, _ctx: Ctx, args: SendApnsArgs) -> Result<SendApnsResult, BoxError> {
        self.apply("send_apns", &args.key, &args.token, &args.data)?;
        Ok(SendApnsResult { apns_id: args.key.clone() })
    }

    async fn send_fcm(&self, _ctx: Ctx, args: SendFcmArgs) -> Result<SendFcmResult, BoxError> {
        self.apply("send_fcm", &args.key, &args.token, &args.data)?;
        Ok(SendFcmResult { message_name: args.key.clone() })
    }
}
