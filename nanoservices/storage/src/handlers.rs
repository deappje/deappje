//! The handler trait implementation: one method per message this
//! nanoservice handles (routing.yaml). A plain executor: it checks who is
//! asking (`group` answers membership), where (the key under the caller's
//! group prefix, the slot's cap, the group's quota), and hands out a
//! presigned URL; on a tombstone or a purge it lists a prefix and deletes
//! what it finds, `Unfenced` (no claim on a request path).

use basable_core::{AppError, Code, Ctx};
use basable_externaleffect::{DispatchError, Unfenced};
use chrono::{DateTime, Duration, Utc};
use interfaces::{StorageHandler, StorageRoutes, StorageSender};
use messages::{CheckMembershipRequest, Role};
use uuid::Uuid;

use crate::Storage;
use crate::config::{SLOT_TYPE, STORAGE_QUOTA_TYPE, SlotConfiguration, StorageQuotaConfiguration};
use crate::effects::{DeleteObjectArgs, PresignArgs, PresignMethod};
use crate::repository as repo;

/// How long a presigned PUT and GET are good for.
pub const UPLOAD_GRANT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
pub const DOWNLOAD_GRANT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// The quota's window: bytes declared in the last 168 hours.
pub const USAGE_WINDOW: Duration = Duration::hours(168);
/// Keys deleted per purge call, and listed per prefix.
pub const LIST_BATCH: usize = 1000;
/// Where the catalogs live and what the one quota is called.
pub const CONFIG_NAMESPACE: &str = "default";
pub const QUOTA_NAME: &str = "default";
/// A slot name: lowercase, digits, `-` and `_`, at most this long.
const MAX_SLOT_LEN: usize = 32;

fn db(op: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    move |e| AppError::wrap(Code::Internal, format!("storage: {op}"), e)
}

fn config_err(op: &'static str) -> impl FnOnce(basable_config::ConfigError) -> AppError {
    move |e| AppError::wrap(Code::Internal, format!("storage: config {op}"), e)
}

/// A key taken apart: `g/<group_id>/o/<object_id>/<slot>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKey {
    pub object_id: Uuid,
    pub slot: String,
}

/// Checks a key is exactly `g/<group>/o/<object>/<slot>` under the given
/// group: no other shape reaches the bucket, so no segment is ever empty,
/// `.` or `..`, and nothing outside the group's prefix can be named.
pub fn parse_key(group_id: Uuid, key: &str) -> Result<ObjectKey, AppError> {
    let bad = |why: &str| AppError::invalid_argument(format!("key: {why}"));
    let parts: Vec<&str> = key.split('/').collect();
    let [g, group, o, object, slot] = parts[..] else {
        return Err(bad("expected g/<group_id>/o/<object_id>/<slot>"));
    };
    if g != "g" || o != "o" {
        return Err(bad("expected g/<group_id>/o/<object_id>/<slot>"));
    }
    if group != group_id.to_string() {
        return Err(AppError::permission_denied("key: outside the group's prefix"));
    }
    let object_id = Uuid::parse_str(object).map_err(|_| bad("the object id is not a uuid"))?;
    if object_id.to_string() != object {
        return Err(bad("the object id is not in canonical form"));
    }
    if slot.is_empty() || slot.len() > MAX_SLOT_LEN || !slot.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err(bad("the slot is lowercase letters, digits, - and _"));
    }
    Ok(ObjectKey { object_id, slot: slot.to_owned() })
}

/// The prefix of one object's files.
pub fn object_prefix(group_id: Uuid, object_id: Uuid) -> String {
    format!("g/{group_id}/o/{object_id}/")
}

/// The prefix of everything of a group.
pub fn group_prefix(group_id: Uuid) -> String {
    format!("g/{group_id}/")
}

/// The hour a moment falls in.
pub fn hour_of(t: DateTime<Utc>) -> DateTime<Utc> {
    let secs = t.timestamp();
    DateTime::from_timestamp(secs - secs.rem_euclid(3600), 0).unwrap_or(t)
}

/// The quota rule: the group's declared bytes over the window, this upload
/// included, stay within the cap.
pub fn within_quota(used_including_this: i64, cap: i64) -> bool {
    used_including_this <= cap
}

impl Storage {
    async fn require_member<R: StorageRoutes>(&self, ctx: &Ctx, s: &StorageSender<'_, R>, user_id: Uuid, group_id: Uuid) -> Result<Role, AppError> {
        s.send_check_membership_request(ctx, CheckMembershipRequest { user_id, group_id })
            .await?
            .role
            .ok_or_else(|| AppError::permission_denied("not a member of this group"))
    }

    async fn namespace_id(&self) -> Result<Uuid, AppError> {
        self.config
            .namespace_id(CONFIG_NAMESPACE)
            .await
            .map_err(config_err("namespace"))?
            .ok_or_else(|| AppError::failed_precondition("storage: the config namespace is not loaded"))
    }

    /// The slot's cap, or none for a slot the catalog does not know.
    async fn slot_cap(&self, slot: &str) -> Result<Option<i64>, AppError> {
        let ns = self.namespace_id().await?;
        let slot = self.config.get::<SlotConfiguration>(SLOT_TYPE, Some(ns), slot).await.map_err(config_err("slot"))?;
        Ok(slot.map(|o| o.message.max_size_bytes))
    }

    async fn quota(&self) -> Result<i64, AppError> {
        let ns = self.namespace_id().await?;
        self.config
            .get::<StorageQuotaConfiguration>(STORAGE_QUOTA_TYPE, Some(ns), QUOTA_NAME)
            .await
            .map_err(config_err("quota"))?
            .map(|o| o.message.max_bytes_per_168h)
            .ok_or_else(|| AppError::failed_precondition("storage: no storage quota configured"))
    }

    fn presign(&self, method: PresignMethod, key: &str, expires: std::time::Duration, now: DateTime<Utc>) -> Result<String, AppError> {
        (self.calls.presign)(PresignArgs { method, key: key.to_owned(), expires, now })
            .map_err(|e| AppError::wrap(Code::Unavailable, "storage: presign", e))
    }

    /// Deletes every key under a prefix, one dispatch per key; answers how
    /// many deletions failed (the next call for the same prefix retries).
    async fn delete_prefix(&self, ctx: &Ctx, prefix: &str) -> Result<usize, AppError> {
        let keys = (self.calls.list_keys)(ctx.clone(), prefix.to_owned(), LIST_BATCH)
            .await
            .map_err(|e| AppError::wrap(Code::Unavailable, "storage: list", e))?;
        let mut failed = 0;
        for key in keys {
            match self.calls.delete_object.dispatch(ctx, &Unfenced, DeleteObjectArgs { key: key.clone() }).await {
                Ok(_) => {}
                Err(DispatchError::Ambiguous(e)) => {
                    // May have landed; the next listing says.
                    tracing::info!(key, error = %e, "delete ambiguous");
                    failed += 1;
                }
                Err(e) => {
                    tracing::warn!(key, error = %e, "delete failed");
                    failed += 1;
                }
            }
        }
        Ok(failed)
    }
}

// Messages stay qualified: a message may share its name with this
// component.
impl<R: StorageRoutes> StorageHandler<R> for Storage {
    async fn handle_presign_upload_request(&self, ctx: &Ctx, msg: messages::PresignUploadRequest, s: StorageSender<'_, R>) -> Result<messages::UploadGrant, AppError> {
        let object = parse_key(msg.group_id, &msg.key)?;
        if msg.size <= 0 {
            return Err(AppError::invalid_argument("size: a positive number of bytes"));
        }
        // Any role: a file is inert without an object, and a viewer may
        // re-upload one it holds.
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        let cap = self
            .slot_cap(&object.slot)
            .await?
            .ok_or_else(|| AppError::invalid_argument(format!("key: unknown slot {:?}", object.slot)))?;
        if msg.size > cap {
            return Err(AppError::invalid_argument(format!("size: over the slot's cap of {cap} bytes")));
        }
        let quota = self.quota().await?;
        let now = Utc::now();
        // The hour row's lock serializes the check with concurrent presigns;
        // an upload over the cap is rolled back out of the counter.
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        repo::add_usage(&mut *tx, msg.group_id, hour_of(now), msg.size).await.map_err(db("add usage"))?;
        let used = repo::usage_since(&mut *tx, msg.group_id, now - USAGE_WINDOW).await.map_err(db("usage"))?;
        if !within_quota(used, quota) {
            tx.rollback().await.map_err(db("rollback"))?;
            return Err(AppError::resource_exhausted(format!("the group's upload quota of {quota} bytes per 168 hours is used up")));
        }
        tx.commit().await.map_err(db("commit"))?;
        let url = self.presign(PresignMethod::Put { size: msg.size }, &msg.key, UPLOAD_GRANT, now)?;
        Ok(messages::UploadGrant { key: msg.key, url, expires_at: now + Duration::from_std(UPLOAD_GRANT).unwrap_or_default() })
    }

    async fn handle_presign_download_request(&self, ctx: &Ctx, msg: messages::PresignDownloadRequest, s: StorageSender<'_, R>) -> Result<messages::DownloadGrant, AppError> {
        parse_key(msg.group_id, &msg.key)?;
        self.require_member(ctx, &s, msg.caller, msg.group_id).await?;
        let now = Utc::now();
        let url = self.presign(PresignMethod::Get, &msg.key, DOWNLOAD_GRANT, now)?;
        Ok(messages::DownloadGrant { key: msg.key, url, expires_at: now + Duration::from_std(DOWNLOAD_GRANT).unwrap_or_default() })
    }

    async fn handle_object_deleted(&self, ctx: &Ctx, msg: messages::ObjectDeleted, _s: StorageSender<'_, R>) {
        // Most objects have no files and the listing is empty.
        match self.delete_prefix(ctx, &object_prefix(msg.group_id, msg.object_id)).await {
            Ok(0) => {}
            Ok(failed) => tracing::warn!(object = %msg.object_id, failed, "object deleted: files left; the next tombstone retries"),
            Err(e) => tracing::warn!(object = %msg.object_id, error = %e, "object deleted: files not listed"),
        }
    }

    async fn handle_purge_group_blobs_request(&self, ctx: &Ctx, msg: messages::PurgeGroupBlobsRequest, _s: StorageSender<'_, R>) -> Result<messages::PurgeResult, AppError> {
        let prefix = group_prefix(msg.group_id);
        self.delete_prefix(ctx, &prefix).await?;
        let left = (self.calls.list_keys)(ctx.clone(), prefix, LIST_BATCH)
            .await
            .map_err(|e| AppError::wrap(Code::Unavailable, "storage: list", e))?;
        Ok(messages::PurgeResult { remaining: left.len() as i64 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_exactly_the_group_object_slot_shape() {
        let g = Uuid::from_u128(1);
        let o = Uuid::from_u128(2);
        let ok = parse_key(g, &format!("g/{g}/o/{o}/full")).unwrap();
        assert_eq!(ok, ObjectKey { object_id: o, slot: "full".into() });
        for bad in [
            format!("g/{g}/o/{o}"),
            format!("g/{g}/o/{o}/"),
            format!("g/{g}/o/{o}/full/extra"),
            format!("g/{g}/o/{o}/FULL"),
            format!("g/{g}/o/{o}/.."),
            format!("g/{g}/o/not-a-uuid/full"),
            format!("g/{g}/o/{}/full", o.simple()),
            format!("x/{g}/o/{o}/full"),
            format!("g/{g}/x/{o}/full"),
            format!("//{g}/o/{o}/full"),
        ] {
            assert_eq!(parse_key(g, &bad).unwrap_err().code(), Code::InvalidArgument, "{bad}");
        }
        let other = Uuid::from_u128(3);
        assert_eq!(parse_key(g, &format!("g/{other}/o/{o}/full")).unwrap_err().code(), Code::PermissionDenied);
    }

    #[test]
    fn hours_are_truncated() {
        let t = DateTime::parse_from_rfc3339("2026-10-30T12:34:56Z").unwrap().with_timezone(&Utc);
        assert_eq!(hour_of(t).to_rfc3339(), "2026-10-30T12:00:00+00:00");
    }

    #[test]
    fn the_quota_is_a_closed_bound() {
        assert!(within_quota(100, 100));
        assert!(!within_quota(101, 100));
    }
}
