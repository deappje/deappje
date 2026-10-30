//! The in-memory simulator of the bucket: `put`, `get`, `list`, `delete`,
//! idempotent by key, with ack loss and refusal on delete. Drives the
//! effect audit and the integration tests; the receiver's store the
//! audit's `landed_count` observes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use basable_core::{BoxError, Ctx};
use basable_externaleffect::{TransportError, definitive};

use crate::effects::*;
use crate::provider::Provider;

#[derive(Default)]
pub struct Simulator {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The bucket: key to bytes.
    objects: BTreeMap<String, Vec<u8>>,
    /// The distinct keys that landed, per operation.
    landed: HashMap<&'static str, HashSet<String>>,
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

    /// What a presigned PUT does: the bytes land under the key.
    pub fn put(&self, key: &str, bytes: Vec<u8>) {
        self.lock().objects.insert(key.to_owned(), bytes);
    }

    /// What a presigned GET does.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.lock().objects.get(key).cloned()
    }

    /// Every key, in order.
    pub fn keys(&self) -> Vec<String> {
        self.lock().objects.keys().cloned().collect()
    }

    /// How many distinct keys landed for an operation.
    pub fn landed_count(&self, op: &'static str) -> usize {
        self.lock().landed.get(op).map_or(0, HashSet::len)
    }

    /// Whether `key` landed for an operation.
    pub fn landed(&self, op: &'static str, key: &str) -> bool {
        self.lock().landed.get(op).is_some_and(|k| k.contains(key))
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
        inner.objects.remove(key);
        if let Some(Fault::AckLoss) = fault {
            return Err(TransportError::wrap(format!("{op}: connection reset after the write")));
        }
        Ok(())
    }
}

impl Provider for Simulator {
    async fn delete_object(&self, _ctx: Ctx, args: DeleteObjectArgs) -> Result<DeleteObjectResult, BoxError> {
        self.apply("delete_object", &args.key)?;
        Ok(DeleteObjectResult { key: args.key })
    }

    async fn list_keys(&self, _ctx: Ctx, prefix: String, limit: usize) -> Result<Vec<String>, BoxError> {
        Ok(self.lock().objects.keys().filter(|k| k.starts_with(&prefix)).take(limit).cloned().collect())
    }

    fn presign(&self, args: PresignArgs) -> Result<String, BoxError> {
        let method = match args.method {
            PresignMethod::Put { size } => format!("put&size={size}"),
            PresignMethod::Get => "get".to_owned(),
        };
        Ok(format!("sim://bucket/{}?{method}&expires={}", args.key, args.expires.as_secs()))
    }
}
