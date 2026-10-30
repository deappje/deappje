//! Integration tests on the testkit: a database per test with every
//! migration applied, skipped without TEST_DATABASE_URL. The handlers are
//! called as the messenger would call them; the router is a stub that
//! answers `CheckMembershipRequest` from the roles a test set and records
//! `DeviceLeftGroup`; the push services are the simulator.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use basable_core::{AppError, Code, Ctx};
use basable_messenger::Route;
use basable_testkit::TestDb;
use device::effects::Calls;
use device::push::reason;
use device::simulator::{Fault, Push, Simulator};
use device::{Device, Schema};
use interfaces::{DeviceHandler, DeviceSender, source};
use messages::*;
use uuid::Uuid;

#[tokio::test]
async fn migrations_apply_and_schema_exists() {
    let Some(db) = TestDb::from_env().await else { return };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'nano_device')")
        .fetch_one(db.migrator())
        .await
        .unwrap();
    assert!(exists, "schema nano_device must exist after the migrations");
    db.finish().await;
}

// --- the stub router ---

/// What `device` may send: membership is answered from the roles a test
/// set, `DeviceLeftGroup` is recorded.
#[derive(Default)]
struct TestRouter {
    roles: Mutex<HashMap<(Uuid, Uuid), Role>>,
    left: Mutex<Vec<DeviceLeftGroup>>,
    checks: Mutex<usize>,
}

impl TestRouter {
    fn member(&self, user_id: Uuid, group_id: Uuid, role: Role) {
        self.roles.lock().unwrap().insert((user_id, group_id), role);
    }

    fn not_member(&self, user_id: Uuid, group_id: Uuid) {
        self.roles.lock().unwrap().remove(&(user_id, group_id));
    }

    fn left(&self) -> Vec<DeviceLeftGroup> {
        self.left.lock().unwrap().clone()
    }
}

#[allow(clippy::manual_async_fn)]
impl Route<CheckMembershipRequest, Result<MembershipCheck, AppError>, source::Device, Ctx> for TestRouter {
    fn route<'a>(&'a self, _ctx: &'a Ctx, msg: CheckMembershipRequest) -> impl Future<Output = Result<MembershipCheck, AppError>> + Send + 'a {
        async move {
            *self.checks.lock().unwrap() += 1;
            Ok(MembershipCheck { role: self.roles.lock().unwrap().get(&(msg.user_id, msg.group_id)).copied() })
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl Route<DeviceLeftGroup, (), source::Device, Ctx> for TestRouter {
    fn route<'a>(&'a self, _ctx: &'a Ctx, msg: DeviceLeftGroup) -> impl Future<Output = ()> + Send + 'a {
        async move { self.left.lock().unwrap().push(msg) }
    }
}

// --- the fixture ---

struct Fx {
    db: TestDb,
    device: &'static Device,
    router: &'static TestRouter,
    sim: Arc<Simulator>,
    ctx: Ctx,
}

async fn fx() -> Option<Fx> {
    let db = TestDb::from_env().await?;
    let sim = Arc::new(Simulator::new());
    let device = Device::with_calls(db.nano_pool::<Schema>().await, Calls::new(sim.clone()));
    Some(Fx {
        db,
        device: Box::leak(Box::new(device)),
        router: Box::leak(Box::new(TestRouter::default())),
        sim,
        ctx: Ctx::background(),
    })
}

fn user() -> Uuid {
    Uuid::new_v4()
}

fn key_bytes(seed: u8) -> Vec<u8> {
    vec![seed; 32]
}

fn code_of(e: &AppError) -> Code {
    e.code()
}

fn reasons(pushes: &[Push]) -> Vec<String> {
    pushes.iter().map(|p| p.data["reason"].clone()).collect()
}

impl Fx {
    fn sender(&self) -> DeviceSender<'static, TestRouter> {
        DeviceSender::new(self.router)
    }

    async fn register(&self, caller: Uuid, device_id: Uuid, seed: u8, platform: Platform) -> Result<messages::Device, AppError> {
        self.device
            .handle_register_device_request(
                &self.ctx,
                RegisterDeviceRequest { caller, device_id, public_key: key_bytes(seed), platform },
                self.sender(),
            )
            .await
    }

    /// A registered device with a push token; the token is the device id,
    /// which is how a test finds its pushes.
    async fn device(&self, caller: Uuid, platform: Platform) -> Uuid {
        let id = Uuid::new_v4();
        self.register(caller, id, id.as_bytes()[0], platform).await.unwrap();
        let provider = match platform {
            Platform::Ios => PushProvider::Apns,
            _ => PushProvider::Fcm,
        };
        self.set_push(caller, id, Some(PushToken { provider, token: id.to_string() })).await.unwrap();
        id
    }

    async fn set_push(&self, caller: Uuid, device_id: Uuid, push: Option<PushToken>) -> Result<messages::Device, AppError> {
        self.device
            .handle_set_push_token_request(&self.ctx, SetPushTokenRequest { caller, device_id, push }, self.sender())
            .await
    }

    async fn publish(&self, caller: Uuid, device_id: Uuid, group_id: Uuid, key_id: i32, targets: &[Uuid]) -> Result<Vec<Uuid>, AppError> {
        let wraps = targets.iter().map(|d| WrapFor { device_id: *d, wrapped: vec![key_id as u8; 8] }).collect();
        let list = self
            .device
            .handle_publish_wrapped_keys_request(
                &self.ctx,
                PublishWrappedKeysRequest { caller, device_id, group_id, key_id, wraps },
                self.sender(),
            )
            .await?;
        Ok(list.keys.iter().map(|k| k.device_id).collect())
    }

    async fn request_keys(&self, caller: Uuid, device_id: Uuid, group_id: Uuid) -> Result<KeyRequest, AppError> {
        self.device
            .handle_request_group_keys_request(
                &self.ctx,
                RequestGroupKeysRequest { caller, device_id, group_id, invite_id: None, invite_tag: Some(vec![9; 32]) },
                self.sender(),
            )
            .await
    }

    async fn list_requests(&self, caller: Uuid, device_id: Uuid) -> Vec<KeyRequest> {
        self.device
            .handle_list_key_requests_request(&self.ctx, ListKeyRequestsRequest { caller, device_id }, self.sender())
            .await
            .unwrap()
            .requests
    }

    async fn fulfil(&self, caller: Uuid, device_id: Uuid, request_id: Uuid, key_ids: &[i32]) -> Result<KeyRequest, AppError> {
        let keys = key_ids.iter().map(|k| WrappedKeyInput { key_id: *k, wrapped: vec![*k as u8; 8] }).collect();
        self.device
            .handle_fulfil_key_request_request(&self.ctx, FulfilKeyRequestRequest { caller, device_id, request_id, keys }, self.sender())
            .await
    }

    async fn keys_of(&self, caller: Uuid, device_id: Uuid, group_id: Option<Uuid>) -> Vec<(Uuid, i32)> {
        self.device
            .handle_get_wrapped_keys_request(&self.ctx, GetWrappedKeysRequest { caller, device_id, group_id }, self.sender())
            .await
            .unwrap()
            .keys
            .iter()
            .map(|k| (k.group_id, k.key_id))
            .collect()
    }

    async fn list_devices(&self, caller: Uuid, device_id: Uuid, group_id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let list = self
            .device
            .handle_list_devices_request(&self.ctx, ListDevicesRequest { caller, device_id, group_id }, self.sender())
            .await?;
        let mut ids: Vec<Uuid> = list.devices.iter().map(|d| d.id).collect();
        ids.sort();
        Ok(ids)
    }

    async fn remove(&self, caller: Uuid, device_id: Uuid) -> Result<messages::Device, AppError> {
        self.device
            .handle_remove_device_request(&self.ctx, RemoveDeviceRequest { caller, device_id }, self.sender())
            .await
    }

    async fn member_removed(&self, group_id: Uuid, user_id: Uuid) {
        self.device
            .handle_member_removed(&self.ctx, MemberRemoved { group_id, user_id, removed_at: chrono::Utc::now() }, self.sender())
            .await
    }

    async fn head_advanced(&self, group_id: Uuid, head_seq: i64, writer_device: Uuid) {
        self.device
            .handle_group_head_advanced(&self.ctx, GroupHeadAdvanced { group_id, head_seq, writer_device }, self.sender())
            .await
    }

    async fn purge_group(&self, group_id: Uuid) -> i64 {
        self.device
            .handle_purge_group_keys_request(&self.ctx, PurgeGroupKeysRequest { group_id }, self.sender())
            .await
            .unwrap()
            .remaining
    }

    async fn purge_user(&self, user_id: Uuid) -> i64 {
        self.device
            .handle_purge_user_devices_request(&self.ctx, PurgeUserDevicesRequest { user_id }, self.sender())
            .await
            .unwrap()
            .remaining
    }

    async fn push_token(&self, device_id: Uuid) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>("SELECT push_token FROM nano_device.device WHERE id = $1")
            .bind(device_id)
            .fetch_one(self.db.superuser())
            .await
            .unwrap()
    }

    async fn key_rows(&self, group_id: Uuid, key_id: i32) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM nano_device.wrapped_key WHERE group_id = $1 AND key_id = $2")
            .bind(group_id)
            .bind(key_id)
            .fetch_one(self.db.superuser())
            .await
            .unwrap()
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

// --- devices ---

#[tokio::test]
async fn devices_belong_to_their_user_and_keep_their_key() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let id = Uuid::new_v4();
    let bad = fx
        .device
        .handle_register_device_request(&fx.ctx, RegisterDeviceRequest { caller: a, device_id: id, public_key: vec![1; 31], platform: Platform::Ios }, fx.sender())
        .await;
    assert_eq!(code_of(&bad.unwrap_err()), Code::InvalidArgument);

    let d = fx.register(a, id, 7, Platform::Ios).await.unwrap();
    assert_eq!((d.id, d.user_id, d.platform, d.push), (id, a, Platform::Ios, None));
    let again = fx.register(a, id, 7, Platform::Web).await.unwrap();
    assert_eq!(again.platform, Platform::Web, "a repeat refreshes the platform");
    assert!(again.last_seen_at >= d.last_seen_at);
    assert_eq!(code_of(&fx.register(b, id, 7, Platform::Ios).await.unwrap_err()), Code::PermissionDenied, "another user's id");
    assert_eq!(code_of(&fx.register(a, id, 8, Platform::Ios).await.unwrap_err()), Code::FailedPrecondition, "a new key is a new device");

    let token = PushToken { provider: PushProvider::Apns, token: "abc".into() };
    assert_eq!(fx.set_push(a, id, Some(token.clone())).await.unwrap().push, Some(token));
    assert_eq!(code_of(&fx.set_push(b, id, None).await.unwrap_err()), Code::PermissionDenied);
    assert_eq!(fx.set_push(a, id, None).await.unwrap().push, None);
    assert_eq!(code_of(&fx.set_push(a, Uuid::new_v4(), None).await.unwrap_err()), Code::NotFound);

    let removed = fx.remove(a, id).await.unwrap();
    assert_eq!(removed.id, id);
    assert_eq!(code_of(&fx.remove(a, id).await.unwrap_err()), Code::NotFound);
    assert!(fx.router.left().is_empty(), "no keys were held");
    fx.finish().await;
}

// --- keys ---

#[tokio::test]
async fn key_requests_are_fulfilled_by_a_holder_who_may_write() {
    let Some(fx) = fx().await else { return };
    let (a, b, c) = (user(), user(), user());
    let g = Uuid::new_v4();
    fx.router.member(a, g, Role::Admin);
    fx.router.member(b, g, Role::Member);
    fx.router.member(c, g, Role::Viewer);
    let a1 = fx.device(a, Platform::Ios).await;
    let b1 = fx.device(b, Platform::Android).await;
    let c1 = fx.device(c, Platform::Android).await;
    let a2 = fx.device(a, Platform::Web).await; // registered, never a holder

    // The creator's device mints key 1 and wraps it for itself; the viewer's
    // device is handed it too (its user is a member).
    assert_eq!(fx.publish(a, a1, g, 1, &[a1, c1]).await.unwrap().len(), 2);
    assert_eq!(code_of(&fx.list_devices(b, b1, g).await.unwrap_err()), Code::PermissionDenied, "a non-holder lists nobody");
    let mut holders = vec![a1, c1];
    holders.sort();
    assert_eq!(fx.list_devices(a, a1, g).await.unwrap(), holders, "a2 holds nothing and is not listed");

    // b1 asks; every holder is woken; the request shows b1's key.
    let req = fx.request_keys(b, b1, g).await.unwrap();
    assert_eq!((req.device.id, req.device.user_id, req.device.public_key.len()), (b1, b, 32));
    assert_eq!(req.invite_tag.as_deref(), Some(&[9u8; 32][..]));
    assert_eq!(reasons(&fx.sim.pushes_to(&a1.to_string())), vec![reason::KEY_REQUEST]);
    assert_eq!(reasons(&fx.sim.pushes_to(&c1.to_string())), vec![reason::KEY_REQUEST]);
    assert!(fx.sim.pushes_to(&a2.to_string()).is_empty());
    assert_eq!(fx.request_keys(b, b1, g).await.unwrap().id, req.id, "a repeat returns the open request");
    assert_eq!(fx.list_requests(a, a1).await.iter().map(|r| r.id).collect::<Vec<_>>(), vec![req.id]);
    assert!(fx.list_requests(b, b1).await.is_empty(), "b1 holds no key, so sees no request");

    // Who may fulfil: a holder, writing; one's own device aside.
    assert_eq!(code_of(&fx.fulfil(b, b1, req.id, &[1]).await.unwrap_err()), Code::PermissionDenied, "not a holder");
    assert_eq!(code_of(&fx.fulfil(c, c1, req.id, &[1]).await.unwrap_err()), Code::PermissionDenied, "a viewer approves no other user");
    assert_eq!(code_of(&fx.fulfil(a, a1, req.id, &[]).await.unwrap_err()), Code::InvalidArgument);
    let done = fx.fulfil(a, a1, req.id, &[1]).await.unwrap();
    assert_eq!(done.id, req.id);
    assert_eq!(reasons(&fx.sim.pushes_to(&b1.to_string())), vec![reason::KEYS]);
    assert_eq!(fx.keys_of(b, b1, Some(g)).await, vec![(g, 1)]);
    assert!(fx.list_requests(a, a1).await.is_empty(), "fulfilled: gone");
    assert_eq!(code_of(&fx.fulfil(a, a1, req.id, &[1]).await.unwrap_err()), Code::NotFound);
    assert_eq!(code_of(&fx.request_keys(b, b1, g).await.unwrap_err()), Code::FailedPrecondition, "already a holder");
    let mut all = vec![a1, b1, c1];
    all.sort();
    assert_eq!(fx.list_devices(b, b1, g).await.unwrap(), all);

    // A viewer may fulfil for its own second device.
    let c2 = fx.device(c, Platform::Ios).await;
    let own = fx.request_keys(c, c2, g).await.unwrap();
    fx.fulfil(c, c1, own.id, &[1]).await.unwrap();
    assert_eq!(fx.keys_of(c, c2, None).await, vec![(g, 1)]);
    fx.finish().await;
}

#[tokio::test]
async fn a_key_id_has_one_publisher_and_a_rotation_reaches_previous_holders_only() {
    let Some(fx) = fx().await else { return };
    let (a, b, x) = (user(), user(), user());
    let g = Uuid::new_v4();
    fx.router.member(a, g, Role::Admin);
    fx.router.member(b, g, Role::Member);
    let a1 = fx.device(a, Platform::Ios).await;
    let b1 = fx.device(b, Platform::Android).await;
    let b2 = fx.device(b, Platform::Web).await;
    let x1 = fx.device(x, Platform::Web).await; // not a member

    assert_eq!(code_of(&fx.publish(a, a1, g, 0, &[a1]).await.unwrap_err()), Code::InvalidArgument);
    assert_eq!(code_of(&fx.publish(a, a1, g, 2, &[a1]).await.unwrap_err()), Code::FailedPrecondition, "key 2 before holding key 1");
    assert_eq!(code_of(&fx.publish(x, x1, g, 1, &[x1]).await.unwrap_err()), Code::PermissionDenied, "not a member");
    let mut landed = fx.publish(a, a1, g, 1, &[a1, b1, x1, Uuid::new_v4()]).await.unwrap();
    landed.sort();
    let mut expected = vec![a1, b1];
    expected.sort();
    assert_eq!(landed, expected, "the non-member's and the unknown device's wraps are dropped");

    // Rotation: a1 publishes key 2 for the holders of key 1; b2 holds no key
    // 1 and is skipped; b1 then publishing key 2 is refused; a1 again is a
    // no-op.
    let mut landed = fx.publish(a, a1, g, 2, &[a1, b1, b2]).await.unwrap();
    landed.sort();
    assert_eq!(landed, expected);
    assert_eq!(code_of(&fx.publish(b, b1, g, 2, &[a1, b1]).await.unwrap_err()), Code::FailedPrecondition);
    fx.publish(a, a1, g, 2, &[a1, b1]).await.unwrap();
    assert_eq!(fx.key_rows(g, 2).await, 2, "republishing lands nothing twice");
    assert_eq!(fx.keys_of(b, b1, Some(g)).await, vec![(g, 1), (g, 2)]);
    assert!(fx.keys_of(b, b2, Some(g)).await.is_empty());
    // b1, a holder of key 2, rotates to 3.
    assert_eq!(fx.publish(b, b1, g, 3, &[a1, b1]).await.unwrap().len(), 2);
    fx.finish().await;
}

#[tokio::test]
async fn a_removed_member_loses_every_key_and_the_rest_rotate() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let (g, h) = (Uuid::new_v4(), Uuid::new_v4());
    for grp in [g, h] {
        fx.router.member(a, grp, Role::Admin);
        fx.router.member(b, grp, Role::Member);
    }
    let a1 = fx.device(a, Platform::Ios).await;
    let b1 = fx.device(b, Platform::Android).await;
    let b2 = fx.device(b, Platform::Ios).await;
    fx.publish(a, a1, g, 1, &[a1, b1, b2]).await.unwrap();
    fx.publish(a, a1, h, 1, &[a1, b1]).await.unwrap();
    let b3 = fx.device(b, Platform::Web).await;
    fx.request_keys(b, b3, g).await.unwrap();

    fx.router.not_member(b, g);
    fx.member_removed(g, b).await;
    assert!(fx.keys_of(b, b1, Some(g)).await.is_empty() && fx.keys_of(b, b2, Some(g)).await.is_empty());
    assert_eq!(fx.keys_of(b, b1, Some(h)).await, vec![(h, 1)], "the other group is untouched");
    assert_eq!(fx.keys_of(a, a1, Some(g)).await, vec![(g, 1)], "a's keys stay");
    assert!(fx.list_requests(a, a1).await.is_empty(), "b3's request went with the membership");
    let mut left: Vec<(Uuid, Uuid)> = fx.router.left().iter().map(|l| (l.group_id, l.device_id)).collect();
    left.sort();
    let mut expected = vec![(g, b1), (g, b2), (g, b3)];
    expected.sort();
    assert_eq!(left, expected, "relay hears of every device of the user");
    let rotations: Vec<String> = fx.sim.pushes().into_iter().filter(|p| p.data["reason"] == reason::ROTATE).map(|p| p.token).collect();
    assert_eq!(rotations, vec![a1.to_string()], "the remaining holder is asked to rotate, once");

    // A second event finds nothing to delete and tells relay again (every
    // device of the user, keys or not: relay may hold a row for a device
    // that acked before its keys came), and asks a1 to rotate again.
    fx.member_removed(g, b).await;
    assert_eq!(fx.router.left().len(), 6);
    fx.finish().await;
}

#[tokio::test]
async fn events_wake_the_right_devices() {
    let Some(fx) = fx().await else { return };
    let (a, b, c) = (user(), user(), user());
    let g = Uuid::new_v4();
    fx.router.member(a, g, Role::Admin);
    fx.router.member(b, g, Role::Member);
    fx.router.member(c, g, Role::Member);
    let a1 = fx.device(a, Platform::Ios).await;
    let b1 = fx.device(b, Platform::Android).await;
    let c1 = fx.device(c, Platform::Android).await;
    fx.publish(a, a1, g, 1, &[a1, b1, c1]).await.unwrap();
    fx.router.not_member(c, g); // c lost membership but its rows are still here

    fx.head_advanced(g, 5, a1).await;
    assert!(fx.sim.pushes_to(&a1.to_string()).is_empty(), "the writer is not woken");
    let b_pushes = fx.sim.pushes_to(&b1.to_string());
    assert_eq!(reasons(&b_pushes), vec![reason::HEAD]);
    assert_eq!(b_pushes[0].data["head_seq"], "5");
    assert!(fx.sim.pushes_to(&c1.to_string()).is_empty(), "a non-member holder is not woken");

    let r = Uuid::new_v4();
    fx.device
        .handle_history_requested(
            &fx.ctx,
            HistoryRequested { request_id: r, group_id: g, requester_device: b1, from_seq: 1, to_seq: 4, object_id: None, devices: vec![a1, b1, c1] },
            fx.sender(),
        )
        .await;
    assert_eq!(reasons(&fx.sim.pushes_to(&a1.to_string())), vec![reason::HISTORY]);
    assert_eq!(reasons(&fx.sim.pushes_to(&c1.to_string())), vec![reason::HISTORY], "relay chose the devices");
    assert_eq!(fx.sim.pushes_to(&b1.to_string()).len(), 1, "the requester is not asked to answer itself");
    fx.device
        .handle_history_ready(&fx.ctx, HistoryReady { request_id: r, requester_device: b1, through_seq: 4, complete: true }, fx.sender())
        .await;
    let b_pushes = fx.sim.pushes_to(&b1.to_string());
    assert_eq!(reasons(&b_pushes), vec![reason::HEAD, reason::HISTORY_READY]);
    assert_eq!(b_pushes[1].data["complete"], "true");
    fx.finish().await;
}

#[tokio::test]
async fn a_dead_token_is_forgotten() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let g = Uuid::new_v4();
    fx.router.member(a, g, Role::Admin);
    fx.router.member(b, g, Role::Member);
    let a1 = fx.device(a, Platform::Ios).await;
    let b1 = fx.device(b, Platform::Android).await;
    fx.publish(a, a1, g, 1, &[a1, b1]).await.unwrap();
    fx.sim.inject("send_fcm", Fault::Unregistered, 1);
    fx.head_advanced(g, 1, a1).await;
    assert_eq!(fx.push_token(b1).await, None, "FCM said unregistered");
    fx.sim.inject("send_fcm", Fault::Refused, 1);
    fx.set_push(b, b1, Some(PushToken { provider: PushProvider::Fcm, token: "new".into() })).await.unwrap();
    fx.head_advanced(g, 2, a1).await;
    assert_eq!(fx.push_token(b1).await.as_deref(), Some("new"), "any other refusal keeps the token");
    fx.finish().await;
}

#[tokio::test]
async fn purges_empty_a_group_and_a_user() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let (g, h) = (Uuid::new_v4(), Uuid::new_v4());
    for grp in [g, h] {
        fx.router.member(a, grp, Role::Admin);
        fx.router.member(b, grp, Role::Member);
    }
    let a1 = fx.device(a, Platform::Ios).await;
    let b1 = fx.device(b, Platform::Android).await;
    fx.publish(a, a1, g, 1, &[a1, b1]).await.unwrap();
    fx.publish(a, a1, h, 1, &[a1, b1]).await.unwrap();
    let b2 = fx.device(b, Platform::Web).await;
    fx.request_keys(b, b2, g).await.unwrap();

    assert_eq!(fx.purge_group(g).await, 0);
    assert_eq!(fx.purge_group(g).await, 0, "idempotent");
    assert!(fx.keys_of(a, a1, Some(g)).await.is_empty());
    assert_eq!(fx.keys_of(a, a1, Some(h)).await, vec![(h, 1)]);
    assert!(fx.list_requests(a, a1).await.is_empty());

    assert_eq!(fx.purge_user(b).await, 0);
    assert_eq!(fx.purge_user(b).await, 0, "idempotent");
    assert_eq!(code_of(&fx.keys_of_result(b, b1).await.unwrap_err()), Code::NotFound, "b's devices are gone");
    let mut left: Vec<(Uuid, Uuid)> = fx.router.left().iter().map(|l| (l.group_id, l.device_id)).collect();
    left.sort();
    assert_eq!(left, vec![(h, b1)], "relay hears of the one group b1 still held");
    assert_eq!(fx.keys_of(a, a1, None).await, vec![(h, 1)]);
    fx.finish().await;
}

impl Fx {
    async fn keys_of_result(&self, caller: Uuid, device_id: Uuid) -> Result<WrappedKeyList, AppError> {
        self.device
            .handle_get_wrapped_keys_request(&self.ctx, GetWrappedKeysRequest { caller, device_id, group_id: None }, self.sender())
            .await
    }
}

#[tokio::test]
async fn removing_a_device_tells_relay_per_group_it_held() {
    let Some(fx) = fx().await else { return };
    let a = user();
    let (g, h) = (Uuid::new_v4(), Uuid::new_v4());
    fx.router.member(a, g, Role::Admin);
    fx.router.member(a, h, Role::Admin);
    let a1 = fx.device(a, Platform::Ios).await;
    let a2 = fx.device(a, Platform::Web).await;
    fx.publish(a, a1, g, 1, &[a1, a2]).await.unwrap();
    fx.publish(a, a1, h, 1, &[a1]).await.unwrap();
    fx.remove(a, a2).await.unwrap();
    assert_eq!(fx.router.left().iter().map(|l| (l.group_id, l.device_id)).collect::<Vec<_>>(), vec![(g, a2)]);
    assert_eq!(fx.list_devices(a, a1, g).await.unwrap(), vec![a1]);
    fx.finish().await;
}
