//! The in-memory simulator of Kratos: every identity exists until its
//! deletion landed, deletion is idempotent by key, it counts what landed
//! and injects faults. Drives the effect audit and the integration tests;
//! the receiver's store the audit's `landed_count` observes.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use basable_core::{BoxError, Ctx};
use basable_externaleffect::{TransportError, definitive};
use uuid::Uuid;

use crate::effects::*;
use crate::provider::Provider;

#[derive(Default)]
pub struct Simulator {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The distinct keys that landed, per operation.
    landed: HashMap<&'static str, HashSet<String>>,
    /// How often each operation was called, faults included.
    calls: HashMap<&'static str, u32>,
    /// Per-operation fault: the next N calls fail this way.
    faults: HashMap<&'static str, (Fault, u32)>,
}

#[derive(Debug, Clone, Copy)]
pub enum Fault {
    /// A transport failure after the write landed (ack loss): a
    /// `TransportError`, which every classifier holds ambiguous.
    AckLoss,
    /// A structured refusal before anything landed: `definitive(..)`.
    Refused,
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

    /// How often an operation was called, faulted calls included.
    pub fn call_count(&self, op: &'static str) -> u32 {
        self.lock().calls.get(op).copied().unwrap_or(0)
    }

    pub fn inject(&self, op: &'static str, fault: Fault, times: u32) {
        self.lock().faults.insert(op, (fault, times));
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock means a test panicked mid-call; the state is still
        // worth reading for the assertion that follows.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn apply(&self, op: &'static str, key: &str) -> Result<(), BoxError> {
        let mut inner = self.lock();
        *inner.calls.entry(op).or_default() += 1;
        let fault = match inner.faults.get_mut(op) {
            Some((f, n)) if *n > 0 => {
                *n -= 1;
                Some(*f)
            }
            _ => None,
        };
        if let Some(Fault::Refused) = fault {
            return Err(definitive(format!("{op}: refused")));
        }
        inner.landed.entry(op).or_default().insert(key.to_owned());
        if let Some(Fault::AckLoss) = fault {
            return Err(TransportError::wrap(format!("{op}: connection reset after the write")));
        }
        Ok(())
    }
}

impl Provider for Simulator {
    async fn delete_identity(&self, _ctx: Ctx, args: DeleteIdentityArgs) -> Result<DeleteIdentityResult, BoxError> {
        let already_absent = self.landed("delete_identity", &args.key());
        self.apply("delete_identity", &args.key())?;
        Ok(DeleteIdentityResult { already_absent })
    }

    async fn identity_exists(&self, _ctx: Ctx, user_id: Uuid) -> Result<bool, BoxError> {
        Ok(!self.landed("delete_identity", &user_id.to_string()))
    }
}
