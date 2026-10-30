//! Integration tests on the testkit: a database per test with every
//! migration applied and the config seed loaded as the `test`
//! environment, skipped without TEST_DATABASE_URL. The handlers are called
//! as the messenger would call them; the router is a stub that answers
//! `CheckMembershipRequest` from the roles a test set; the bucket is the
//! simulator.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use basable_config::{ConfigTypesBuilder, Environment, Repository, load_seed};
use basable_core::{AppError, Code, Ctx};
use basable_messenger::Route;
use basable_testkit::{TestDb, runfile};
use chrono::{Duration, Utc};
use interfaces::{StorageHandler, StorageSender, source};
use messages::*;
use storage::effects::Calls;
use storage::handlers::USAGE_WINDOW;
use storage::simulator::{Fault, Simulator};
use storage::{Schema, Storage};
use uuid::Uuid;

#[tokio::test]
async fn migrations_apply_and_schema_exists() {
    let Some(db) = TestDb::from_env().await else { return };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'nano_storage')")
        .fetch_one(db.migrator())
        .await
        .unwrap();
    assert!(exists, "schema nano_storage must exist after the migrations");
    db.finish().await;
}

// --- the stub router ---

#[derive(Default)]
struct TestRouter {
    roles: Mutex<HashMap<(Uuid, Uuid), Role>>,
}

impl TestRouter {
    fn member(&self, user_id: Uuid, group_id: Uuid, role: Role) {
        self.roles.lock().unwrap().insert((user_id, group_id), role);
    }
}

#[allow(clippy::manual_async_fn)]
impl Route<CheckMembershipRequest, Result<MembershipCheck, AppError>, source::Storage, Ctx> for TestRouter {
    fn route<'a>(&'a self, _ctx: &'a Ctx, msg: CheckMembershipRequest) -> impl Future<Output = Result<MembershipCheck, AppError>> + Send + 'a {
        async move { Ok(MembershipCheck { role: self.roles.lock().unwrap().get(&(msg.user_id, msg.group_id)).copied() }) }
    }
}

// --- the fixture ---

struct Fx {
    db: TestDb,
    storage: &'static Storage,
    router: &'static TestRouter,
    sim: Arc<Simulator>,
    ctx: Ctx,
}

async fn fx() -> Option<Fx> {
    let db = TestDb::from_env().await?;
    let mut types = ConfigTypesBuilder::new();
    storage::config::register(&mut types);
    let types = types.build().unwrap().shared();
    load_seed(db.migrator(), types.clone(), &runfile("config/base"), Environment::Test)
        .await
        .expect("the seed applies");
    let config = Repository::new(db.migrator().clone(), types);
    let sim = Arc::new(Simulator::new());
    let storage = Storage::with_calls(db.nano_pool::<Schema>().await, Calls::new(sim.clone()), config);
    Some(Fx {
        db,
        storage: Box::leak(Box::new(storage)),
        router: Box::leak(Box::new(TestRouter::default())),
        sim,
        ctx: Ctx::background(),
    })
}

fn user() -> Uuid {
    Uuid::new_v4()
}

fn code_of(e: &AppError) -> Code {
    e.code()
}

fn key(group: Uuid, object: Uuid, slot: &str) -> String {
    format!("g/{group}/o/{object}/{slot}")
}

impl Fx {
    fn sender(&self) -> StorageSender<'static, TestRouter> {
        StorageSender::new(self.router)
    }

    async fn upload(&self, caller: Uuid, group_id: Uuid, key: &str, size: i64) -> Result<UploadGrant, AppError> {
        self.storage
            .handle_presign_upload_request(&self.ctx, PresignUploadRequest { caller, group_id, key: key.to_owned(), size }, self.sender())
            .await
    }

    async fn download(&self, caller: Uuid, group_id: Uuid, key: &str) -> Result<DownloadGrant, AppError> {
        self.storage
            .handle_presign_download_request(&self.ctx, PresignDownloadRequest { caller, group_id, key: key.to_owned() }, self.sender())
            .await
    }

    async fn object_deleted(&self, group_id: Uuid, object_id: Uuid) {
        self.storage
            .handle_object_deleted(&self.ctx, ObjectDeleted { group_id, object_id }, self.sender())
            .await
    }

    async fn purge(&self, group_id: Uuid) -> i64 {
        self.storage
            .handle_purge_group_blobs_request(&self.ctx, PurgeGroupBlobsRequest { group_id }, self.sender())
            .await
            .unwrap()
            .remaining
    }

    async fn usage_rows(&self, group_id: Uuid) -> Vec<i64> {
        sqlx::query_scalar::<_, i64>("SELECT bytes FROM nano_storage.upload_usage WHERE group_id = $1 ORDER BY hour")
            .bind(group_id)
            .fetch_all(self.db.superuser())
            .await
            .unwrap()
    }

    async fn age_usage(&self, group_id: Uuid) {
        sqlx::query("UPDATE nano_storage.upload_usage SET hour = $2 WHERE group_id = $1")
            .bind(group_id)
            .bind(Utc::now() - USAGE_WINDOW - Duration::hours(2))
            .execute(self.db.superuser())
            .await
            .unwrap();
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

const MIB: i64 = 1024 * 1024;

#[tokio::test]
async fn presigning_checks_the_member_the_key_the_slot_and_the_quota() {
    let Some(fx) = fx().await else { return };
    let (a, v, x, g) = (user(), user(), user(), Uuid::new_v4());
    fx.router.member(a, g, Role::Admin);
    fx.router.member(v, g, Role::Viewer);
    let o = Uuid::new_v4();
    let full = key(g, o, "full");

    // The grant: a PUT on the simulator's address, signed for the size.
    let grant = fx.upload(a, g, &full, MIB).await.unwrap();
    assert_eq!(grant.key, full);
    assert!(grant.url.starts_with(&format!("sim://bucket/{full}?put&size={MIB}")), "{}", grant.url);
    assert!(grant.expires_at > Utc::now() + Duration::minutes(14));
    assert_eq!(fx.usage_rows(g).await, vec![MIB], "the declared size is counted");
    let get = fx.download(v, g, &full).await.unwrap();
    assert!(get.url.starts_with(&format!("sim://bucket/{full}?get")), "a viewer downloads");
    assert_eq!(fx.upload(v, g, &key(g, o, "file"), 10).await.unwrap().key, key(g, o, "file"), "and re-uploads");

    // Who and what is refused.
    assert_eq!(code_of(&fx.upload(x, g, &full, 10).await.unwrap_err()), Code::PermissionDenied, "not a member");
    assert_eq!(code_of(&fx.download(x, g, &full).await.unwrap_err()), Code::PermissionDenied);
    let other = Uuid::new_v4();
    assert_eq!(code_of(&fx.upload(a, g, &key(other, o, "full"), 10).await.unwrap_err()), Code::PermissionDenied, "outside the prefix");
    assert_eq!(code_of(&fx.download(a, g, &key(other, o, "full")).await.unwrap_err()), Code::PermissionDenied);
    assert_eq!(code_of(&fx.upload(a, g, &format!("g/{g}/o/{o}/../full"), 10).await.unwrap_err()), Code::InvalidArgument);
    assert_eq!(code_of(&fx.upload(a, g, &format!("g/{g}//o/{o}/full"), 10).await.unwrap_err()), Code::InvalidArgument);
    assert_eq!(code_of(&fx.upload(a, g, &key(g, o, "thumb"), 10).await.unwrap_err()), Code::InvalidArgument, "unknown slot");
    assert_eq!(code_of(&fx.upload(a, g, &full, 0).await.unwrap_err()), Code::InvalidArgument);
    assert_eq!(code_of(&fx.upload(a, g, &full, 25 * MIB + 1).await.unwrap_err()), Code::InvalidArgument, "over the slot's cap");
    assert_eq!(fx.upload(a, g, &full, 2 * MIB).await.unwrap().key, full, "at the cap's side of the quota: 1 + 2 + 0 MiB");

    // The test seed's quota is 4 MiB per 168 hours: exactly at the bound
    // is accepted, one byte over is refused and not counted.
    assert_eq!(fx.upload(a, g, &key(g, o, "file"), MIB - 10).await.unwrap().key, key(g, o, "file"));
    assert_eq!(code_of(&fx.upload(a, g, &full, 1).await.unwrap_err()), Code::ResourceExhausted);
    assert_eq!(fx.usage_rows(g).await.iter().sum::<i64>(), 4 * MIB, "the refused upload left no trace");
    let h = Uuid::new_v4();
    fx.router.member(a, h, Role::Admin);
    assert!(fx.upload(a, h, &key(h, o, "full"), MIB).await.is_ok(), "another group's quota");

    // Counters older than the window stop counting.
    fx.age_usage(g).await;
    assert!(fx.upload(a, g, &full, MIB).await.is_ok(), "the window moved on");
    fx.storage.tick_expire_usage(&fx.ctx, fx.sender()).await.unwrap();
    assert_eq!(fx.usage_rows(g).await, vec![MIB], "the old rows went, the new one stays");
    fx.finish().await;
}

#[tokio::test]
async fn a_tombstone_deletes_the_objects_files_and_a_purge_the_groups() {
    let Some(fx) = fx().await else { return };
    let g = Uuid::new_v4();
    let (o1, o2) = (Uuid::new_v4(), Uuid::new_v4());
    let other = Uuid::new_v4();
    for k in [key(g, o1, "full"), key(g, o1, "file"), key(g, o2, "full"), key(other, o1, "full")] {
        fx.sim.put(&k, vec![1]);
    }

    // A failed delete leaves the file; the next tombstone retries it.
    fx.sim.inject("delete_object", Fault::Refused, 1);
    fx.object_deleted(g, o1).await;
    let mut left = fx.sim.keys();
    left.sort();
    assert_eq!(left.len(), 3, "one of o1's two files survived the refusal: {left:?}");
    fx.object_deleted(g, o1).await;
    let mut left = fx.sim.keys();
    left.sort();
    let mut expected = vec![key(g, o2, "full"), key(other, o1, "full")];
    expected.sort();
    assert_eq!(left, expected, "exactly o1's files went");
    fx.object_deleted(g, Uuid::new_v4()).await; // an object without files: a no-op

    // The purge empties the group's prefix and nothing else.
    fx.sim.put(&key(g, o1, "full"), vec![2]);
    assert_eq!(fx.purge(g).await, 0);
    assert_eq!(fx.purge(g).await, 0, "idempotent");
    assert_eq!(fx.sim.keys(), vec![key(other, o1, "full")]);
    assert_eq!(fx.sim.landed_count("delete_object"), 3, "three distinct keys were deleted, one of them twice");
    fx.finish().await;
}

#[tokio::test]
async fn a_purge_reports_what_a_lost_delete_left() {
    let Some(fx) = fx().await else { return };
    let g = Uuid::new_v4();
    fx.sim.put(&key(g, Uuid::new_v4(), "full"), vec![1]);
    fx.sim.put(&key(g, Uuid::new_v4(), "full"), vec![1]);
    fx.sim.inject("delete_object", Fault::Refused, 1);
    assert_eq!(fx.purge(g).await, 1, "one delete refused: the listing still shows it");
    assert_eq!(fx.purge(g).await, 0);
    fx.finish().await;
}
