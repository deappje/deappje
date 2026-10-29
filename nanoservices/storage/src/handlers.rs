//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). Handlers are the INTENT boundary:
//! they validate, write desired state through the typed store (`create`,
//! `update_spec`, `mark_deleted`, `nudge`) or the repository, dispatch
//! request-path adapters `Unfenced`, and send through `s` (the sender that
//! exposes exactly the declared sends). They never write status.

use basable_core::{AppError, Ctx};
use interfaces::{StorageHandler, StorageRoutes, StorageSender};

use crate::Storage;

// Messages stay qualified: a message may share its name with this
// component (an `order` nanoservice handling an `Order` response).
impl<R: StorageRoutes> StorageHandler<R> for Storage {
    async fn handle_presign_upload_request(&self, _ctx: &Ctx, _msg: messages::PresignUploadRequest, _s: StorageSender<'_, R>) -> Result<messages::UploadGrant, AppError> {
        Err(crate::unimplemented_step("storage.handle_presign_upload_request"))
    }
    async fn handle_presign_download_request(&self, _ctx: &Ctx, _msg: messages::PresignDownloadRequest, _s: StorageSender<'_, R>) -> Result<messages::DownloadGrant, AppError> {
        Err(crate::unimplemented_step("storage.handle_presign_download_request"))
    }
    async fn handle_object_deleted(&self, _ctx: &Ctx, _msg: messages::ObjectDeleted, _s: StorageSender<'_, R>) {
        // A void fan-out event: observe it, nudge an object or write a side
        // table owned by this nanoservice; never a status write.
        tracing::warn!(step = "storage.handle_object_deleted", "unimplemented step reached");
    }
    async fn handle_purge_group_blobs_request(&self, _ctx: &Ctx, _msg: messages::PurgeGroupBlobsRequest, _s: StorageSender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        Err(crate::unimplemented_step("storage.handle_purge_group_blobs_request"))
    }
}
