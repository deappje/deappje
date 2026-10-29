//! The `user_purge` type declaration: spec (desired state, the identity
//! fields immutable) and status (observed state, written only under claim
//! authority). Mirror every column of the migration here AND in adapter.rs.

use basable_processingobject::ProcessingObjectType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Adapter;

/// The registry key allocated by the scaffolder; never changes.
pub const TYPE_KEY: i16 = 1;
/// The public-id prefix: `upr_<base62>`; a duplicate panics at boot.
pub const PUBLIC_ID_PREFIX: &str = "upr";

/// The declaration the store binds: name, key, prefix and the adapter.
pub fn decl() -> ProcessingObjectType<Spec, Status, Adapter> {
    ProcessingObjectType::new("user_purge", TYPE_KEY, PUBLIC_ID_PREFIX, Adapter)
}

/// Desired state. Immutable fields are guarded by the migration's trigger
/// and omitted from `write_spec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub user_id: uuid::Uuid, // immutable
}

/// Observed state, one whole row per write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub phase: String,
    pub groups_remaining: i32,
    pub devices_remaining: Option<i64>,
    pub identity_deleted: bool,
    // Declared-effect slot columns go here when an adapter uses
    // Strategy::Declared: `effect: Option<EffectSlot>`.
}

// `Uuid` is the id type of every reference field; the import stays even
// when this type declares none.
#[allow(dead_code)]
type IdType = Uuid;
