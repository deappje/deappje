//! The `group_purge` type declaration: spec (desired state, the identity
//! fields immutable) and status (observed state, written only under claim
//! authority). Mirror every column of the migration here AND in adapter.rs.

use basable_processingobject::ProcessingObjectType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Adapter;

/// The registry key allocated by the scaffolder; never changes.
pub const TYPE_KEY: i16 = 2;
/// The public-id prefix: `gpr_<base62>`; a duplicate panics at boot.
pub const PUBLIC_ID_PREFIX: &str = "gpr";

/// The declaration the store binds: name, key, prefix and the adapter.
pub fn decl() -> ProcessingObjectType<Spec, Status, Adapter> {
    ProcessingObjectType::new("group_purge", TYPE_KEY, PUBLIC_ID_PREFIX, Adapter)
}

/// Desired state. Immutable fields are guarded by the migration's trigger
/// and omitted from `write_spec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub group_id: uuid::Uuid, // immutable
    pub parent_purge_id: Option<uuid::Uuid>, // immutable
}

/// Observed state, one whole row per write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub phase: String,
    pub subgroups_remaining: i32,
    pub objects_remaining: Option<i64>,
    pub blobs_remaining: Option<i64>,
    pub keys_remaining: Option<i64>,
    // Declared-effect slot columns go here when an adapter uses
    // Strategy::Declared: `effect: Option<EffectSlot>`.
}

/// The values of `Status::phase`, in the order a purge passes through them.
pub mod phase {
    /// `purging`: the other nanoservices are being emptied of the group.
    pub const PURGING: &str = "purging";
    /// `done`: nothing of the group remains; the object is being removed.
    pub const DONE: &str = "done";
}

// `Uuid` is the id type of every reference field; the import stays even
// when this type declares none.
#[allow(dead_code)]
type IdType = Uuid;
