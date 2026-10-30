//! Processing-object type `user_purge` (key 1, `upr_…`) of
//! nanoservice `group`: the declaration, the adapter, the reconciler
//! and its worker.

mod adapter;
mod reconciler;
#[path = "type.rs"]
mod type_decl;

pub use adapter::Adapter;
pub use reconciler::{Reconciler, worker};
pub use type_decl::{PUBLIC_ID_PREFIX, Spec, Status, TYPE_KEY, decl, phase};

/// The typed store of this type over the nanoservice's pool.
pub type Store = basable_processingobject::TypedStore<Spec, Status, Adapter>;
