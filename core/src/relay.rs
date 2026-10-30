//! The relay's contract as the core speaks it (`docs/01-deappje/02-relay.md`,
//! `proto/relay/v1/relay.proto`): the messages as plain values and the
//! [`Relay`] seam the sync engine calls, implemented over Connect by the
//! transport and over the in-memory model by the harness
//! (`crate::harness`). Errors carry the server's code, so the engine can
//! tell a refusal it must act on from an outage it waits out.

use std::fmt;

use uuid::Uuid;

/// A row of the log as the server holds it. A tombstone is `deleted` with
/// no value; its seq is its deletion mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The object.
    pub id: Uuid,
    /// The envelope bytes; none for a tombstone.
    pub value: Option<Vec<u8>>,
    /// Its seq in the group's log.
    pub seq: i64,
    /// Whether it is a tombstone.
    pub deleted: bool,
}

/// One object of a push: `base_seq` is the seq of the row the device
/// edited from, 0 for a new object; a delete is `deleted` with no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushEntry {
    /// The object.
    pub id: Uuid,
    /// The envelope bytes; none for a delete.
    pub value: Option<Vec<u8>>,
    /// The seq edited from, 0 for a create.
    pub base_seq: i64,
    /// Whether this is a delete.
    pub deleted: bool,
}

/// An object the push wrote, at its new seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// The object.
    pub id: Uuid,
    /// Its seq now.
    pub seq: i64,
}

/// An object the push refused, with the row as the server holds it, for
/// the rebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The object.
    pub id: Uuid,
    /// The current row.
    pub current: Entry,
}

/// What a push came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    /// Per object accepted or in conflict, and the head after.
    Accepted {
        /// The objects written.
        accepted: Vec<Accepted>,
        /// The objects refused.
        conflicts: Vec<Conflict>,
        /// The group's head after the push.
        head_seq: i64,
    },
    /// The whole batch refused: the device's ack is below the eviction
    /// floor, so it is not a holder and starts over.
    BehindFloor {
        /// The floor.
        evicted_through_seq: i64,
    },
}

/// A page of the log after a seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    /// The rows, seq ascending, tombstones included.
    pub entries: Vec<Entry>,
    /// The group's head.
    pub head_seq: i64,
    /// The eviction floor: rows at or below it are gone from the server.
    pub evicted_through_seq: i64,
    /// The minimum ack over the group's devices, if any has a row.
    pub min_acked_seq: Option<i64>,
    /// Whether another page follows.
    pub more: bool,
}

/// What an ack came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckResult {
    /// The floor.
    pub evicted_through_seq: i64,
    /// The minimum ack after this one.
    pub min_acked_seq: Option<i64>,
}

/// Where a group stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The group.
    pub group_id: Uuid,
    /// Its head.
    pub head_seq: i64,
    /// Its floor.
    pub evicted_through_seq: i64,
    /// The minimum ack.
    pub min_acked_seq: Option<i64>,
}

/// The status of a history request for the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryStatus {
    /// Stored, unclaimed or abandoned.
    Open,
    /// Claimed by the calling device.
    Claimed,
    /// Claimed by another device.
    Taken,
    /// Nobody holds anything older than the window; nothing was stored.
    Unfillable,
}

/// A history request as the server has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRequest {
    /// The request; nil when unfillable.
    pub id: Uuid,
    /// The group.
    pub group_id: Uuid,
    /// The device asking.
    pub requester_device: Uuid,
    /// The range asked for.
    pub from_seq: i64,
    /// The range's end.
    pub to_seq: i64,
    /// One object, when the request is for one.
    pub object_id: Option<Uuid>,
    /// The device holding the claim.
    pub claimed_by: Option<Uuid>,
    /// Its status for the caller.
    pub status: HistoryStatus,
}

/// What a history push came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushHistoryResult {
    /// Rows re-inserted at their original seqs.
    pub inserted: i64,
    /// Rows skipped because the id already had a row.
    pub skipped: i64,
    /// The last seq of the call.
    pub through_seq: i64,
    /// Whether the request is complete.
    pub complete: bool,
}

/// The server's answer code, as Connect carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// The request is malformed.
    InvalidArgument,
    /// The state does not allow it (an ack backwards, above the head).
    FailedPrecondition,
    /// Not a member, not a holder, not the claimer.
    PermissionDenied,
    /// A request that is gone.
    NotFound,
    /// The server failed.
    Internal,
    /// The server could not be reached, or the session is gone: nothing
    /// was decided.
    Unavailable,
}

/// A refusal or a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayError {
    /// The code.
    pub code: Code,
    /// The server's message.
    pub message: String,
}

impl RelayError {
    /// A refusal with the code.
    pub fn new(code: Code, message: impl Into<String>) -> RelayError {
        RelayError { code, message: message.into() }
    }
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "relay: {:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for RelayError {}

/// The relay as the sync engine calls it. Every call names the calling
/// device; the caller's user is the session's.
#[allow(async_fn_in_trait)] // the core's own seam: the engine and the two implementations are in step
pub trait Relay {
    /// One batch, one transaction.
    async fn push_objects(&self, device_id: Uuid, group_id: Uuid, objects: Vec<PushEntry>) -> Result<PushResult, RelayError>;
    /// The rows after `since_seq`, at most `limit` (0 for the default page).
    async fn pull_changes(&self, device_id: Uuid, group_id: Uuid, since_seq: i64, limit: i32) -> Result<ChangeSet, RelayError>;
    /// The device holds the group through `seq`.
    async fn ack_seq(&self, device_id: Uuid, group_id: Uuid, seq: i64) -> Result<AckResult, RelayError>;
    /// Where the groups stand; a group left out is one the caller is no
    /// longer in.
    async fn get_heads(&self, device_id: Uuid, group_ids: Vec<Uuid>) -> Result<Vec<Head>, RelayError>;
    /// Rows in the range, or one object, from a holder.
    async fn request_history(&self, device_id: Uuid, group_id: Uuid, from_seq: i64, to_seq: i64, object_id: Option<Uuid>) -> Result<HistoryRequest, RelayError>;
    /// The group's requests a holder may claim.
    async fn list_history_requests(&self, device_id: Uuid, group_id: Uuid) -> Result<Vec<HistoryRequest>, RelayError>;
    /// Takes a request; the first wins.
    async fn claim_history(&self, device_id: Uuid, request_id: Uuid) -> Result<HistoryRequest, RelayError>;
    /// The claimer's rows, oldest first; `complete` on the last call.
    async fn push_history(&self, device_id: Uuid, request_id: Uuid, entries: Vec<Entry>, complete: bool) -> Result<PushHistoryResult, RelayError>;
}
