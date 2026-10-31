//! Integration tests on the testkit: a database per test with every
//! migration applied, skipped without TEST_DATABASE_URL. The handlers are
//! called as the messenger would call them; the router is a stub that
//! answers `CheckMembershipRequest` from the roles a test set and records
//! every event `relay` sends.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use basable_core::{AppError, Code, Ctx};
use basable_messenger::Route;
use basable_testkit::TestDb;
use chrono::{Duration, Utc};
use interfaces::{RelayHandler, RelaySender, source};
use messages::*;
use relay::handlers::{MAX_VALUE_BYTES, WINDOW};
use relay::{Relay, Schema};
use uuid::Uuid;

#[tokio::test]
async fn migrations_apply_and_schema_exists() {
    let Some(db) = TestDb::from_env().await else { return };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'nano_relay')")
        .fetch_one(db.migrator())
        .await
        .unwrap();
    assert!(exists, "schema nano_relay must exist after the migrations");
    db.finish().await;
}

// --- the stub router ---

#[derive(Default)]
struct TestRouter {
    roles: Mutex<HashMap<(Uuid, Uuid), Role>>,
    heads: Mutex<Vec<GroupHeadAdvanced>>,
    deleted: Mutex<Vec<ObjectDeleted>>,
    requested: Mutex<Vec<HistoryRequested>>,
    ready: Mutex<Vec<HistoryReady>>,
}

impl TestRouter {
    fn member(&self, user_id: Uuid, group_id: Uuid, role: Role) {
        self.roles.lock().unwrap().insert((user_id, group_id), role);
    }
}

#[allow(clippy::manual_async_fn)]
impl Route<CheckMembershipRequest, Result<MembershipCheck, AppError>, source::Relay, Ctx> for TestRouter {
    fn route<'a>(&'a self, _ctx: &'a Ctx, msg: CheckMembershipRequest) -> impl Future<Output = Result<MembershipCheck, AppError>> + Send + 'a {
        async move { Ok(MembershipCheck { role: self.roles.lock().unwrap().get(&(msg.user_id, msg.group_id)).copied() }) }
    }
}

macro_rules! record_route {
    ($msg:ident, $field:ident) => {
        #[allow(clippy::manual_async_fn)]
        impl Route<$msg, (), source::Relay, Ctx> for TestRouter {
            fn route<'a>(&'a self, _ctx: &'a Ctx, msg: $msg) -> impl Future<Output = ()> + Send + 'a {
                async move { self.$field.lock().unwrap().push(msg) }
            }
        }
    };
}

record_route!(GroupHeadAdvanced, heads);
record_route!(ObjectDeleted, deleted);
record_route!(HistoryRequested, requested);
record_route!(HistoryReady, ready);

// --- the fixture ---

struct Fx {
    db: TestDb,
    relay: &'static Relay,
    router: &'static TestRouter,
    ctx: Ctx,
}

async fn fx() -> Option<Fx> {
    let db = TestDb::from_env().await?;
    let relay = Relay::with_pool(db.nano_pool::<Schema>().await);
    Some(Fx { db, relay: Box::leak(Box::new(relay)), router: Box::leak(Box::new(TestRouter::default())), ctx: Ctx::background() })
}

fn user() -> Uuid {
    Uuid::new_v4()
}

fn code_of(e: &AppError) -> Code {
    e.code()
}

fn live(id: Uuid, base_seq: i64, byte: u8) -> PushEntry {
    PushEntry { id, value: Some(vec![byte; 4]), base_seq, deleted: false }
}

fn tombstone(id: Uuid, base_seq: i64) -> PushEntry {
    PushEntry { id, value: None, base_seq, deleted: true }
}

impl Fx {
    fn sender(&self) -> RelaySender<'static, TestRouter> {
        RelaySender::new(self.router)
    }

    async fn push(&self, caller: Uuid, device_id: Uuid, group_id: Uuid, objects: Vec<PushEntry>) -> Result<PushResult, AppError> {
        self.relay
            .handle_push_objects_request(&self.ctx, PushObjectsRequest { caller, device_id, group_id, objects }, self.sender())
            .await
    }

    /// A push that must be accepted whole; the seqs assigned.
    async fn push_ok(&self, caller: Uuid, device_id: Uuid, group_id: Uuid, objects: Vec<PushEntry>) -> Vec<i64> {
        match self.push(caller, device_id, group_id, objects).await.unwrap() {
            PushResult::Accepted { accepted, conflicts, .. } => {
                assert!(conflicts.is_empty(), "{conflicts:?}");
                accepted.iter().map(|a| a.seq).collect()
            }
            other => panic!("refused: {other:?}"),
        }
    }

    async fn pull(&self, caller: Uuid, device_id: Uuid, group_id: Uuid, since_seq: i64, limit: i32) -> ChangeSet {
        self.relay
            .handle_pull_changes_request(&self.ctx, PullChangesRequest { caller, device_id, group_id, since_seq, limit }, self.sender())
            .await
            .unwrap()
    }

    async fn ack(&self, caller: Uuid, device_id: Uuid, group_id: Uuid, seq: i64) -> Result<AckResult, AppError> {
        self.relay
            .handle_ack_seq_request(&self.ctx, AckSeqRequest { caller, device_id, group_id, seq }, self.sender())
            .await
    }

    async fn heads(&self, caller: Uuid, device_id: Uuid, group_ids: Vec<Uuid>) -> Vec<Head> {
        self.relay
            .handle_get_heads_request(&self.ctx, GetHeadsRequest { caller, device_id, group_ids }, self.sender())
            .await
            .unwrap()
            .heads
    }

    async fn request_history(&self, caller: Uuid, device_id: Uuid, group_id: Uuid, from_seq: i64, to_seq: i64) -> Result<HistoryRequest, AppError> {
        self.relay
            .handle_request_history_request(
                &self.ctx,
                RequestHistoryRequest { caller, device_id, group_id, from_seq, to_seq, object_id: None },
                self.sender(),
            )
            .await
    }

    async fn get(&self, caller: Uuid, device_id: Uuid, request_id: Uuid) -> Result<HistoryRequest, AppError> {
        self.relay
            .handle_get_history_request_request(&self.ctx, GetHistoryRequestRequest { caller, device_id, request_id }, self.sender())
            .await
    }

    async fn claimable(&self, caller: Uuid, device_id: Uuid, group_id: Uuid) -> Vec<Uuid> {
        self.relay
            .handle_list_history_requests_request(&self.ctx, ListHistoryRequestsRequest { caller, device_id, group_id }, self.sender())
            .await
            .unwrap()
            .requests
            .iter()
            .map(|r| r.id)
            .collect()
    }

    async fn claim(&self, caller: Uuid, device_id: Uuid, request_id: Uuid) -> Result<HistoryRequest, AppError> {
        self.relay
            .handle_claim_history_request(&self.ctx, ClaimHistoryRequest { caller, device_id, request_id }, self.sender())
            .await
    }

    async fn push_history(&self, caller: Uuid, device_id: Uuid, request_id: Uuid, entries: Vec<Entry>, complete: bool) -> Result<PushHistoryResult, AppError> {
        self.relay
            .handle_push_history_request(&self.ctx, PushHistoryRequest { caller, device_id, request_id, entries, complete }, self.sender())
            .await
    }

    async fn left(&self, group_id: Uuid, device_id: Uuid) {
        self.relay
            .handle_device_left_group(&self.ctx, DeviceLeftGroup { group_id, device_id }, self.sender())
            .await
    }

    async fn purge(&self, group_id: Uuid) -> i64 {
        self.relay
            .handle_purge_group_objects_request(&self.ctx, PurgeGroupObjectsRequest { group_id }, self.sender())
            .await
            .unwrap()
            .remaining
    }

    async fn evict(&self) {
        self.relay.tick_evict_objects(&self.ctx, self.sender()).await.unwrap();
    }

    async fn rows(&self, group_id: Uuid) -> Vec<(Uuid, i64, bool)> {
        sqlx::query_as::<_, (Uuid, i64, bool)>("SELECT id, seq, deleted FROM nano_relay.object WHERE group_id = $1 ORDER BY seq")
            .bind(group_id)
            .fetch_all(self.db.superuser())
            .await
            .unwrap()
    }

    async fn age_rows(&self, group_id: Uuid, through_seq: i64) {
        sqlx::query("UPDATE nano_relay.object SET uploaded_at = $2 WHERE group_id = $1 AND seq <= $3")
            .bind(group_id)
            .bind(Utc::now() - WINDOW - Duration::hours(1))
            .bind(through_seq)
            .execute(self.db.superuser())
            .await
            .unwrap();
    }

    async fn age_device(&self, group_id: Uuid, device_id: Uuid) {
        sqlx::query("UPDATE nano_relay.group_device SET updated_at = $3 WHERE group_id = $1 AND device_id = $2")
            .bind(group_id)
            .bind(device_id)
            .bind(Utc::now() - WINDOW - Duration::hours(1))
            .execute(self.db.superuser())
            .await
            .unwrap();
    }

    async fn age_claim(&self, request_id: Uuid) {
        sqlx::query("UPDATE nano_relay.history_request SET claimed_at = $2 WHERE id = $1")
            .bind(request_id)
            .bind(Utc::now() - Duration::minutes(6))
            .execute(self.db.superuser())
            .await
            .unwrap();
    }

    async fn device_rows(&self, group_id: Uuid) -> Vec<(Uuid, i64)> {
        sqlx::query_as::<_, (Uuid, i64)>("SELECT device_id, acked_seq FROM nano_relay.group_device WHERE group_id = $1 ORDER BY acked_seq, device_id")
            .bind(group_id)
            .fetch_all(self.db.superuser())
            .await
            .unwrap()
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

// --- push, pull, ack ---

#[tokio::test]
async fn push_is_a_compare_and_set_on_seq_by_a_holder() {
    let Some(fx) = fx().await else { return };
    let (a, v, g) = (user(), user(), Uuid::new_v4());
    fx.router.member(a, g, Role::Admin);
    fx.router.member(v, g, Role::Viewer);
    let (a1, a2, v1) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let (x, y) = (Uuid::new_v4(), Uuid::new_v4());

    // No row yet: not a holder, whatever the batch holds.
    assert_eq!(fx.push(a, a1, g, vec![live(x, 0, 1)]).await.unwrap(), PushResult::BehindFloor { evicted_through_seq: 0 });
    fx.ack(a, a1, g, 0).await.unwrap();
    assert_eq!(code_of(&fx.push(v, v1, g, vec![live(x, 0, 1)]).await.unwrap_err()), Code::PermissionDenied, "a viewer does not write");
    assert_eq!(code_of(&fx.push(a, a1, g, vec![]).await.unwrap_err()), Code::InvalidArgument);
    let big = PushEntry { id: x, value: Some(vec![0; MAX_VALUE_BYTES + 1]), base_seq: 0, deleted: false };
    assert_eq!(code_of(&fx.push(a, a1, g, vec![big]).await.unwrap_err()), Code::InvalidArgument, "over 1 MiB refuses the batch");

    // A parent and its child in one batch take consecutive seqs.
    assert_eq!(fx.push_ok(a, a1, g, vec![live(x, 0, 1), live(y, 0, 2)]).await, vec![1, 2]);
    let heads = fx.router.heads.lock().unwrap().clone();
    assert_eq!(heads.len(), 1);
    assert_eq!((heads[0].group_id, heads[0].head_seq, heads[0].writer_device), (g, 2, a1));

    // An edit from the current seq takes the next; a stale base conflicts
    // with the current row; a create over an existing row conflicts too.
    assert_eq!(fx.push_ok(a, a1, g, vec![live(x, 1, 3)]).await, vec![3]);
    match fx.push(a, a1, g, vec![live(x, 1, 4), live(y, 2, 5)]).await.unwrap() {
        PushResult::Accepted { accepted, conflicts, head_seq } => {
            assert_eq!(accepted.iter().map(|a| (a.id, a.seq)).collect::<Vec<_>>(), vec![(y, 4)]);
            assert_eq!(conflicts.len(), 1);
            assert_eq!((conflicts[0].id, conflicts[0].current.seq, conflicts[0].current.value.as_deref()), (x, 3, Some(&[3u8; 4][..])));
            assert_eq!(head_seq, 4);
        }
        other => panic!("{other:?}"),
    }
    match fx.push(a, a1, g, vec![live(x, 0, 6)]).await.unwrap() {
        PushResult::Accepted { conflicts, .. } => assert_eq!(conflicts[0].current.seq, 3),
        other => panic!("{other:?}"),
    }

    // A delete is a push: the tombstone takes a seq, has no value, and
    // storage hears of it; a stale delete conflicts; an un-delete from the
    // tombstone's seq is the client's call.
    assert_eq!(fx.push_ok(a, a1, g, vec![tombstone(x, 3)]).await, vec![5]);
    assert_eq!(fx.rows(g).await, vec![(y, 4, false), (x, 5, true)]);
    let deleted = fx.router.deleted.lock().unwrap().clone();
    assert_eq!(deleted.iter().map(|d| d.object_id).collect::<Vec<_>>(), vec![x]);
    match fx.push(a, a2, g, vec![tombstone(x, 3)]).await.unwrap() {
        PushResult::BehindFloor { .. } => {}
        other => panic!("a2 never acked: {other:?}"),
    }
    fx.ack(a, a2, g, 0).await.unwrap();
    match fx.push(a, a2, g, vec![tombstone(x, 3)]).await.unwrap() {
        PushResult::Accepted { conflicts, .. } => assert!(conflicts[0].current.deleted, "the tombstone is the current row"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.push_ok(a, a2, g, vec![live(x, 5, 7)]).await, vec![6], "un-deleted from the tombstone's seq");
    assert_eq!(fx.router.heads.lock().unwrap().len(), 5, "one GroupHeadAdvanced per batch that landed something");
    fx.finish().await;
}

#[tokio::test]
async fn pull_pages_the_log_and_acks_are_monotone() {
    let Some(fx) = fx().await else { return };
    let (a, g, a1, b1) = (user(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    fx.router.member(a, g, Role::Member);
    let empty = fx.pull(a, a1, g, 0, 0).await;
    assert_eq!((empty.entries.len(), empty.head_seq, empty.min_acked_seq, empty.more), (0, 0, None, false), "a group nobody wrote to");
    fx.ack(a, a1, g, 0).await.unwrap();
    let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    fx.push_ok(a, a1, g, ids.iter().map(|id| live(*id, 0, 1)).collect()).await;

    let page = fx.pull(a, a1, g, 0, 2).await;
    assert_eq!(page.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
    assert!(page.more);
    assert_eq!((page.head_seq, page.evicted_through_seq, page.min_acked_seq), (5, 0, Some(0)));
    let rest = fx.pull(a, a1, g, 2, 0).await;
    assert_eq!(rest.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3, 4, 5]);
    assert!(!rest.more);
    assert_eq!(rest.entries[0].value.as_deref(), Some(&[1u8; 4][..]));

    assert_eq!(code_of(&fx.ack(a, b1, g, 3).await.unwrap_err()), Code::FailedPrecondition, "a first ack is 0");
    assert_eq!(code_of(&fx.ack(a, a1, g, 6).await.unwrap_err()), Code::FailedPrecondition, "above the head");
    let r = fx.ack(a, a1, g, 5).await.unwrap();
    assert_eq!((r.evicted_through_seq, r.min_acked_seq), (0, Some(5)));
    assert_eq!(code_of(&fx.ack(a, a1, g, 3).await.unwrap_err()), Code::FailedPrecondition, "backwards");
    fx.ack(a, b1, g, 0).await.unwrap();
    assert_eq!(fx.pull(a, b1, g, 0, 0).await.min_acked_seq, Some(0), "the newcomer pins the minimum");
    assert_eq!(fx.ack(a, a1, g, 0).await.unwrap().min_acked_seq, Some(0), "0 starts over");
    assert_eq!(fx.device_rows(g).await.len(), 2);

    let other = Uuid::new_v4();
    let heads = fx.heads(a, a1, vec![g, other]).await;
    assert_eq!(heads.len(), 1, "a group the caller is not in is left out");
    assert_eq!((heads[0].group_id, heads[0].head_seq, heads[0].min_acked_seq), (g, 5, Some(0)));
    fx.finish().await;
}

#[tokio::test]
async fn concurrent_pushes_take_a_gap_free_sequence() {
    let Some(fx) = fx().await else { return };
    let (a, g, a1, a2) = (user(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    fx.router.member(a, g, Role::Member);
    fx.ack(a, a1, g, 0).await.unwrap();
    fx.ack(a, a2, g, 0).await.unwrap();
    let n = 25;
    let (left, right) = tokio::join!(
        async {
            let mut seqs = Vec::new();
            for i in 0..n {
                seqs.extend(fx.push_ok(a, a1, g, vec![live(Uuid::new_v4(), 0, i as u8)]).await);
            }
            seqs
        },
        async {
            let mut seqs = Vec::new();
            for i in 0..n {
                seqs.extend(fx.push_ok(a, a2, g, vec![live(Uuid::new_v4(), 0, i as u8)]).await);
            }
            seqs
        }
    );
    let mut all: Vec<i64> = left.into_iter().chain(right).collect();
    all.sort_unstable();
    assert_eq!(all, (1..=2 * n as i64).collect::<Vec<_>>());
    assert_eq!(fx.pull(a, a1, g, 0, 0).await.head_seq, 2 * n as i64);
    fx.finish().await;
}

// --- history ---

#[tokio::test]
async fn history_is_served_by_one_claiming_holder_at_the_original_seqs() {
    let Some(fx) = fx().await else { return };
    let (a, b, g) = (user(), user(), Uuid::new_v4());
    fx.router.member(a, g, Role::Admin);
    fx.router.member(b, g, Role::Viewer);
    let (a1, a2, b1) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    fx.ack(a, a1, g, 0).await.unwrap();
    let ids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
    fx.push_ok(a, a1, g, ids.iter().map(|id| live(*id, 0, 9)).collect()).await;
    fx.push_ok(a, a1, g, vec![tombstone(ids[1], 2)]).await; // seq 5
    fx.ack(a, a1, g, 5).await.unwrap();
    fx.ack(a, a2, g, 5).await.unwrap_err(); // a2 never acked 0
    fx.ack(a, a2, g, 0).await.unwrap();
    fx.ack(a, a2, g, 5).await.unwrap();
    // Evict by ack: b1 is not here yet, so the minimum is 5 and everything goes.
    fx.evict().await;
    assert!(fx.rows(g).await.is_empty());
    assert_eq!(fx.pull(a, a1, g, 0, 0).await.evicted_through_seq, 5);

    // b1 joins: ack 0, asks for everything up to the floor.
    fx.ack(b, b1, g, 0).await.unwrap();
    assert_eq!(code_of(&fx.request_history(b, b1, g, 0, 5).await.unwrap_err()), Code::InvalidArgument);
    let req = fx.request_history(b, b1, g, 1, 5).await.unwrap();
    assert_eq!(req.status, HistoryStatus::Open);
    assert_eq!(req.through_seq, 0, "nothing pushed yet: from_seq - 1");
    let seen = fx.get(b, b1, req.id).await.unwrap();
    assert_eq!((seen.status, seen.through_seq), (HistoryStatus::Open, 0), "the requester reads its own request");
    let asked = fx.router.requested.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    let mut devices = asked[0].devices.clone();
    devices.sort();
    let mut holders = vec![a1, a2];
    holders.sort();
    assert_eq!(devices, holders, "the holders, the requester excluded");
    assert_eq!(fx.claimable(a, a1, g).await, vec![req.id]);

    // Claims: not the requester, only a holder, first wins.
    assert_eq!(code_of(&fx.claim(b, b1, req.id).await.unwrap_err()), Code::InvalidArgument);
    let (c1, c2) = tokio::join!(fx.claim(a, a1, req.id), fx.claim(a, a2, req.id));
    let (c1, c2) = (c1.unwrap(), c2.unwrap());
    let winners = [c1.status, c2.status].iter().filter(|s| **s == HistoryStatus::Claimed).count();
    assert_eq!(winners, 1, "{c1:?} {c2:?}");
    let claimer = if c1.status == HistoryStatus::Claimed { a1 } else { a2 };
    let other = if claimer == a1 { a2 } else { a1 };
    assert_eq!(fx.claim(a, claimer, req.id).await.unwrap().status, HistoryStatus::Claimed, "the claimer again");
    assert!(fx.claimable(a, a1, g).await.is_empty(), "claimed: not listed");
    assert_eq!(fx.get(b, b1, req.id).await.unwrap().status, HistoryStatus::Taken, "claimed by another, as the requester sees it");
    assert_eq!(fx.get(a, claimer, req.id).await.unwrap().status, HistoryStatus::Claimed, "and as the claimer sees it");

    // Only the claimer pushes, oldest first, inside the range; rows land at
    // their original seqs, the tombstone as a tombstone, the head stays.
    let entry = |i: usize, seq: i64| Entry { id: ids[i], value: Some(vec![9; 4]), seq, deleted: false };
    assert_eq!(code_of(&fx.push_history(a, other, req.id, vec![entry(0, 1)], false).await.unwrap_err()), Code::PermissionDenied);
    assert_eq!(code_of(&fx.push_history(a, claimer, req.id, vec![entry(2, 3), entry(0, 1)], false).await.unwrap_err()), Code::InvalidArgument);
    assert_eq!(code_of(&fx.push_history(a, claimer, req.id, vec![entry(0, 6)], false).await.unwrap_err()), Code::InvalidArgument);
    let first = fx.push_history(a, claimer, req.id, vec![entry(0, 1), entry(2, 3)], false).await.unwrap();
    assert_eq!((first.inserted, first.skipped, first.through_seq, first.complete), (2, 0, 3, false));
    let ready = fx.router.ready.lock().unwrap().clone();
    assert_eq!((ready[0].requester_device, ready[0].through_seq, ready[0].complete), (b1, 3, false));
    let seen = fx.get(b, b1, req.id).await.unwrap();
    assert_eq!((seen.status, seen.through_seq), (HistoryStatus::Taken, 3), "the page's progress, readable without the wake-up");

    // Meanwhile a1 edits object 0 at a new seq: the re-push of its old row is
    // skipped, the current row wins.
    fx.push_ok(a, a1, g, vec![live(ids[0], 1, 1)]).await; // seq 6
    let last = fx
        .push_history(a, claimer, req.id, vec![entry(0, 1), entry(3, 4), Entry { id: ids[1], value: None, seq: 5, deleted: true }], true)
        .await
        .unwrap();
    assert_eq!((last.inserted, last.skipped, last.through_seq, last.complete), (2, 1, 5, true));
    assert_eq!(fx.rows(g).await, vec![(ids[2], 3, false), (ids[3], 4, false), (ids[1], 5, true), (ids[0], 6, false)]);
    let set = fx.pull(b, b1, g, 0, 0).await;
    assert_eq!((set.head_seq, set.evicted_through_seq), (6, 5), "neither head nor floor moved");
    assert_eq!(code_of(&fx.push_history(a, claimer, req.id, vec![], true).await.unwrap_err()), Code::NotFound, "complete: gone for the claimer");
    let done = fx.get(b, b1, req.id).await.unwrap();
    assert_eq!((done.status, done.through_seq), (HistoryStatus::Complete, 5), "complete is the whole range, and the requester reads it");
    assert_eq!(code_of(&fx.claim(a, other, req.id).await.unwrap_err()), Code::NotFound, "complete: not claimable");
    assert!(fx.claimable(a, other, g).await.is_empty(), "complete: not listed");

    // b1 pages and acks the restored rows past the floor: a holder now.
    fx.ack(b, b1, g, 6).await.unwrap();
    assert_eq!(fx.push(b, b1, g, vec![live(Uuid::new_v4(), 0, 1)]).await.unwrap_err().code(), Code::PermissionDenied, "a holder, but a viewer");

    // A request from a group with no other holder is unfillable at once.
    let lonely = Uuid::new_v4();
    fx.router.member(a, lonely, Role::Admin);
    fx.ack(a, a1, lonely, 0).await.unwrap();
    let none = fx.request_history(a, a1, lonely, 1, 1).await.unwrap();
    assert_eq!(none.status, HistoryStatus::Unfillable);
    assert!(none.id.is_nil());
    assert_eq!(fx.router.requested.lock().unwrap().len(), 1, "nothing asked, nothing stored");

    // An abandoned claim is taken over after five minutes.
    let again = fx.request_history(b, b1, g, 1, 5).await.unwrap();
    assert_eq!(fx.claim(a, a1, again.id).await.unwrap().status, HistoryStatus::Claimed);
    assert_eq!(fx.claim(a, a2, again.id).await.unwrap().status, HistoryStatus::Taken);
    assert!(fx.claimable(a, a2, g).await.is_empty());
    fx.age_claim(again.id).await;
    assert_eq!(fx.claimable(a, a2, g).await, vec![again.id], "abandoned: listed again");
    assert_eq!(fx.claim(a, a2, again.id).await.unwrap().status, HistoryStatus::Claimed);
    assert_eq!(code_of(&fx.push_history(a, a1, again.id, vec![], true).await.unwrap_err()), Code::PermissionDenied, "the first claimer lost it");
    fx.finish().await;
}

// --- eviction ---

#[tokio::test]
async fn eviction_by_ack_and_by_age_moves_the_floor_and_drops_stale_devices() {
    let Some(fx) = fx().await else { return };
    let (a, g) = (user(), Uuid::new_v4());
    fx.router.member(a, g, Role::Member);
    let (a1, a2, a3) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    for d in [a1, a2, a3] {
        fx.ack(a, d, g, 0).await.unwrap();
    }
    let ids: Vec<Uuid> = (0..6).map(|_| Uuid::new_v4()).collect();
    fx.push_ok(a, a1, g, ids.iter().map(|id| live(*id, 0, 1)).collect()).await; // seqs 1..=6

    // By ack: the minimum is a3's 0, nothing goes; then a3 catches up to 2.
    fx.ack(a, a1, g, 6).await.unwrap();
    fx.ack(a, a2, g, 4).await.unwrap();
    fx.evict().await;
    assert_eq!(fx.rows(g).await.len(), 6, "a3 at 0 pins everything");
    fx.ack(a, a3, g, 2).await.unwrap();
    fx.evict().await;
    assert_eq!(fx.rows(g).await.iter().map(|r| r.1).collect::<Vec<_>>(), vec![3, 4, 5, 6]);
    assert_eq!(fx.pull(a, a1, g, 0, 0).await.evicted_through_seq, 2);

    // By age: seqs 3 and 4 are old; a3 (acked 2, below the new floor 4)
    // has not acked within the window and is evicted; a2 (acked 4, at the
    // floor) stays; a device below the floor that acked recently stays.
    let a4 = Uuid::new_v4();
    fx.ack(a, a4, g, 0).await.unwrap();
    fx.age_rows(g, 4).await;
    fx.age_device(g, a3).await;
    fx.evict().await;
    assert_eq!(fx.rows(g).await.iter().map(|r| r.1).collect::<Vec<_>>(), vec![5, 6]);
    assert_eq!(fx.pull(a, a1, g, 0, 0).await.evicted_through_seq, 4);
    let mut devices: Vec<Uuid> = fx.device_rows(g).await.iter().map(|d| d.0).collect();
    devices.sort();
    let mut expected = vec![a1, a2, a4];
    expected.sort();
    assert_eq!(devices, expected, "a3 evicted with the rows it never acked; a4 is catching up");

    // a3 comes back: below the floor, so it starts over; a push before that
    // is refused.
    assert_eq!(fx.push(a, a3, g, vec![live(Uuid::new_v4(), 0, 1)]).await.unwrap(), PushResult::BehindFloor { evicted_through_seq: 4 });
    fx.ack(a, a3, g, 0).await.unwrap();
    assert_eq!(fx.push(a, a3, g, vec![live(Uuid::new_v4(), 0, 1)]).await.unwrap(), PushResult::BehindFloor { evicted_through_seq: 4 }, "ack 0 is below the floor too");
    fx.ack(a, a3, g, 6).await.unwrap();
    fx.push_ok(a, a3, g, vec![live(Uuid::new_v4(), 0, 1)]).await;

    // A device that left loses its row; the purge empties the group.
    fx.left(g, a4).await;
    assert_eq!(fx.device_rows(g).await.len(), 3);
    assert_eq!(fx.purge(g).await, 0);
    assert_eq!(fx.purge(g).await, 0, "idempotent");
    assert!(fx.rows(g).await.is_empty() && fx.device_rows(g).await.is_empty());
    assert_eq!(fx.pull(a, a1, g, 0, 0).await.head_seq, 0, "the counter is gone too");
    fx.finish().await;
}

#[tokio::test]
async fn old_history_requests_expire() {
    let Some(fx) = fx().await else { return };
    let (a, b, g) = (user(), user(), Uuid::new_v4());
    fx.router.member(a, g, Role::Admin);
    fx.router.member(b, g, Role::Member);
    let (a1, b1) = (Uuid::new_v4(), Uuid::new_v4());
    fx.ack(a, a1, g, 0).await.unwrap();
    fx.push_ok(a, a1, g, vec![live(Uuid::new_v4(), 0, 1)]).await;
    fx.ack(a, a1, g, 1).await.unwrap();
    fx.ack(b, b1, g, 0).await.unwrap();
    let req = fx.request_history(b, b1, g, 1, 1).await.unwrap();
    sqlx::query("UPDATE nano_relay.history_request SET created_at = $2 WHERE id = $1")
        .bind(req.id)
        .bind(Utc::now() - WINDOW - Duration::hours(1))
        .execute(fx.db.superuser())
        .await
        .unwrap();
    fx.relay.tick_expire_history_requests(&fx.ctx, fx.sender()).await.unwrap();
    assert!(fx.claimable(a, a1, g).await.is_empty());
    assert_eq!(code_of(&fx.claim(a, a1, req.id).await.unwrap_err()), Code::NotFound);
    assert_eq!(code_of(&fx.get(b, b1, req.id).await.unwrap_err()), Code::NotFound, "swept: the requester asks again");
    fx.finish().await;
}
