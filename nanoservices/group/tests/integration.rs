//! Integration tests on the testkit: a database per test with every
//! migration applied, skipped without TEST_DATABASE_URL. The handlers are
//! called as the messenger would call them; the router is a stub that
//! records what `group` sends and answers the purge requests with the
//! counts a test queued (zero by default); Kratos is the simulator. A
//! purge pass is driven by hand: one claim, one pass, one completion.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use basable_core::{AppError, Code, Ctx};
use basable_messenger::Route;
use basable_processingobject::{Completion, Object, Outcome, Reconciler as _, WorkerConfig};
use basable_testkit::TestDb;
use chrono::{Duration as Span, Utc};
use group::effects::Calls;
use group::purge::{group_purge_id, user_purge_id};
use group::simulator::Simulator;
use group::types::{group_purge, user_purge};
use group::{Group, Schema};
use interfaces::{GroupHandler, GroupSender, source};
use messages::*;
use uuid::Uuid;

#[tokio::test]
async fn migrations_apply_and_schema_exists() {
    let Some(db) = TestDb::from_env().await else { return };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'nano_group')")
        .fetch_one(db.migrator())
        .await
        .unwrap();
    assert!(exists, "schema nano_group must exist after the migrations");
    db.finish().await;
}

// --- the stub router ---

/// What `group` may send, answered by the test: `MemberRemoved` is
/// recorded; each purge request pops the next queued answer for its
/// receiver, or zero.
#[derive(Default)]
struct TestRouter {
    removed: Mutex<Vec<MemberRemoved>>,
    sent: Mutex<Vec<String>>,
    answers: Mutex<HashMap<&'static str, VecDeque<Result<i64, ()>>>>,
}

impl TestRouter {
    fn queue(&self, receiver: &'static str, answers: impl IntoIterator<Item = Result<i64, ()>>) {
        self.answers.lock().unwrap().entry(receiver).or_default().extend(answers);
    }

    fn answer(&self, receiver: &'static str, id: Uuid) -> Result<PurgeResult, AppError> {
        self.sent.lock().unwrap().push(format!("{receiver}:{id}"));
        let next = self.answers.lock().unwrap().get_mut(receiver).and_then(VecDeque::pop_front);
        match next {
            Some(Ok(remaining)) => Ok(PurgeResult { remaining }),
            Some(Err(())) => Err(AppError::unavailable(format!("{receiver}: injected failure"))),
            None => Ok(PurgeResult { remaining: 0 }),
        }
    }

    fn sent(&self, receiver: &str, id: Uuid) -> usize {
        let key = format!("{receiver}:{id}");
        self.sent.lock().unwrap().iter().filter(|s| **s == key).count()
    }

    fn removed(&self) -> Vec<MemberRemoved> {
        self.removed.lock().unwrap().clone()
    }
}

// The trait's shape, kept as declared (an `async fn` would do the same).
#[allow(clippy::manual_async_fn)]
impl Route<MemberRemoved, (), source::Group, Ctx> for TestRouter {
    fn route<'a>(&'a self, _ctx: &'a Ctx, msg: MemberRemoved) -> impl Future<Output = ()> + Send + 'a {
        async move { self.removed.lock().unwrap().push(msg) }
    }
}

macro_rules! purge_route {
    ($msg:ident, $receiver:literal, $field:ident) => {
        impl Route<$msg, Result<PurgeResult, AppError>, source::Group, Ctx> for TestRouter {
            fn route<'a>(&'a self, _ctx: &'a Ctx, msg: $msg) -> impl Future<Output = Result<PurgeResult, AppError>> + Send + 'a {
                async move { self.answer($receiver, msg.$field) }
            }
        }
    };
}

purge_route!(PurgeGroupObjectsRequest, "objects", group_id);
purge_route!(PurgeGroupBlobsRequest, "blobs", group_id);
purge_route!(PurgeGroupKeysRequest, "keys", group_id);
purge_route!(PurgeUserDevicesRequest, "devices", user_id);

// --- the fixture ---

struct Fx {
    db: TestDb,
    group: &'static Group,
    router: &'static TestRouter,
    sim: Arc<Simulator>,
    ctx: Ctx,
}

async fn fx() -> Option<Fx> {
    let db = TestDb::from_env().await?;
    let sim = Arc::new(Simulator::new());
    let group = Group::with_calls(db.nano_pool::<Schema>().await, Calls::new(sim.clone())).await;
    Some(Fx {
        db,
        group: Box::leak(Box::new(group)),
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

impl Fx {
    fn sender(&self) -> GroupSender<'static, TestRouter> {
        GroupSender::new(self.router)
    }

    async fn create_group(&self, caller: Uuid, parent: Option<Uuid>) -> Result<Group_, AppError> {
        self.group
            .handle_create_group_request(&self.ctx, CreateGroupRequest { caller, parent_group_id: parent }, self.sender())
            .await
    }

    async fn invite(&self, caller: Uuid, group_id: Uuid, role: Role, max_uses: i32) -> Result<Invite, AppError> {
        self.group
            .handle_create_invite_request(
                &self.ctx,
                CreateInviteRequest { caller, group_id, role, expires_at: Utc::now() + Span::hours(1), max_uses },
                self.sender(),
            )
            .await
    }

    async fn accept(&self, caller: Uuid, code: &str) -> Result<Membership, AppError> {
        self.group
            .handle_accept_invite_request(&self.ctx, AcceptInviteRequest { caller, code: code.to_owned() }, self.sender())
            .await
    }

    /// Admits `who` to the group through a fresh one-use invite.
    async fn join(&self, admin: Uuid, group_id: Uuid, who: Uuid, role: Role) -> Membership {
        let invite = self.invite(admin, group_id, role, 1).await.unwrap();
        self.accept(who, &invite.code).await.unwrap()
    }

    async fn add(&self, caller: Uuid, group_id: Uuid, user_id: Uuid, role: Role) -> Result<Membership, AppError> {
        self.group
            .handle_add_member_request(&self.ctx, AddMemberRequest { caller, group_id, user_id, role }, self.sender())
            .await
    }

    async fn remove(&self, caller: Uuid, group_id: Uuid, user_id: Uuid) -> Result<Membership, AppError> {
        self.group
            .handle_remove_member_request(&self.ctx, RemoveMemberRequest { caller, group_id, user_id }, self.sender())
            .await
    }

    async fn set_role(&self, caller: Uuid, group_id: Uuid, user_id: Uuid, role: Role) -> Result<Membership, AppError> {
        self.group
            .handle_set_member_role_request(&self.ctx, SetMemberRoleRequest { caller, group_id, user_id, role }, self.sender())
            .await
    }

    async fn leave(&self, caller: Uuid, group_id: Uuid) -> Result<Membership, AppError> {
        self.group
            .handle_leave_group_request(&self.ctx, LeaveGroupRequest { caller, group_id }, self.sender())
            .await
    }

    async fn delete_group(&self, caller: Uuid, group_id: Uuid) -> Result<Group_, AppError> {
        self.group
            .handle_delete_group_request(&self.ctx, DeleteGroupRequest { caller, group_id }, self.sender())
            .await
    }

    async fn members(&self, caller: Uuid, group_id: Uuid) -> Result<Vec<(Uuid, Role)>, AppError> {
        let list = self
            .group
            .handle_list_members_request(&self.ctx, ListMembersRequest { caller, group_id }, self.sender())
            .await?;
        Ok(list.members.iter().map(|m| (m.user_id, m.role)).collect())
    }

    async fn groups(&self, caller: Uuid) -> Vec<Uuid> {
        let list = self
            .group
            .handle_list_groups_request(&self.ctx, ListGroupsRequest { caller }, self.sender())
            .await
            .unwrap();
        list.groups.iter().map(|g| g.id).collect()
    }

    async fn check(&self, user_id: Uuid, group_id: Uuid) -> Option<Role> {
        self.group
            .handle_check_membership_request(&self.ctx, CheckMembershipRequest { user_id, group_id }, self.sender())
            .await
            .unwrap()
            .role
    }

    async fn delete_account(&self, caller: Uuid) -> AccountDeletion {
        self.group
            .handle_delete_account_request(&self.ctx, DeleteAccountRequest { caller }, self.sender())
            .await
            .unwrap()
    }

    /// One claim, one pass, one completion of the `group_purge` worker,
    /// or `None` when nothing is due.
    async fn drive_group_purge(&self) -> Option<(Object<group_purge::Spec, group_purge::Status>, Completion<group_purge::Status>)> {
        let store = self.group.group_purge_store();
        let mut claims = store.claim_batch(WorkerConfig { batch_size: 1, ..WorkerConfig::default() }).await.unwrap();
        let mut claim = claims.pop()?;
        let rec = group_purge::Reconciler::new(self.router, self.group);
        let out = rec.reconcile(&self.ctx, &mut claim).await;
        let snapshot = claim.object.clone();
        Some((snapshot, claim.complete(out).await.unwrap()))
    }

    async fn drive_user_purge(&self) -> Option<(Object<user_purge::Spec, user_purge::Status>, Completion<user_purge::Status>)> {
        let store = self.group.user_purge_store();
        let mut claims = store.claim_batch(WorkerConfig { batch_size: 1, ..WorkerConfig::default() }).await.unwrap();
        let mut claim = claims.pop()?;
        let rec = user_purge::Reconciler::new(self.router, self.group);
        let out = rec.reconcile(&self.ctx, &mut claim).await;
        let snapshot = claim.object.clone();
        Some((snapshot, claim.complete(out).await.unwrap()))
    }

    /// Makes a parked object due now (a `converged_after` wait is minutes).
    async fn nudge_group_purge(&self, group_id: Uuid) {
        let store = self.group.group_purge_store();
        store.nudge(&store.r#ref(group_purge_id(group_id))).await.unwrap();
    }

    async fn nudge_user_purge(&self, user_id: Uuid) {
        let store = self.group.user_purge_store();
        store.nudge(&store.r#ref(user_purge_id(user_id))).await.unwrap();
    }

    async fn group_purge_exists(&self, group_id: Uuid) -> bool {
        let store = self.group.group_purge_store();
        store.read(&store.r#ref(group_purge_id(group_id))).await.is_ok()
    }

    async fn user_purge_exists(&self, user_id: Uuid) -> bool {
        let store = self.group.user_purge_store();
        store.read(&store.r#ref(user_purge_id(user_id))).await.is_ok()
    }

    async fn group_row_exists(&self, group_id: Uuid) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nano_group.groups WHERE id = $1)")
            .bind(group_id)
            .fetch_one(self.db.superuser())
            .await
            .unwrap()
    }

    async fn membership_rows(&self, group_id: Uuid) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM nano_group.membership WHERE group_id = $1")
            .bind(group_id)
            .fetch_one(self.db.superuser())
            .await
            .unwrap()
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

// `messages::Group` beside the component `group::Group`.
type Group_ = messages::Group;

fn outcome_name<T>(c: &Completion<T>) -> &'static str {
    match c {
        Completion::Committed { outcome, .. } => outcome.name(),
        Completion::Unknown => "unknown",
    }
}

/// The status the completion committed, if the outcome carried one.
fn status<T>(c: &Completion<T>) -> Option<&T> {
    match c {
        Completion::Committed { outcome, .. } => outcome.status(),
        Completion::Unknown => None,
    }
}

fn is_delete<T>(c: &Completion<T>) -> bool {
    matches!(c, Completion::Committed { outcome: Outcome::Delete, .. })
}

// --- groups, invites, memberships ---

#[tokio::test]
async fn groups_invites_and_roles() {
    let Some(fx) = fx().await else { return };
    let (a, b, c) = (user(), user(), user());
    let g = fx.create_group(a, None).await.unwrap();
    assert_eq!(fx.check(a, g.id).await, Some(Role::Admin), "the creator is the first admin");
    assert_eq!(fx.groups(a).await, vec![g.id]);

    // A one-use invite admits one member with the invite's role.
    let invite = fx.invite(a, g.id, Role::Member, 1).await.unwrap();
    assert_eq!(invite.code.len(), 10);
    let m = fx.accept(b, &invite.code).await.unwrap();
    assert_eq!((m.group_id, m.user_id, m.role), (g.id, b, Role::Member));
    assert_eq!(fx.check(b, g.id).await, Some(Role::Member));
    assert_eq!(fx.members(a, g.id).await.unwrap(), vec![(a, Role::Admin), (b, Role::Member)]);
    assert_eq!(fx.groups(b).await, vec![g.id]);

    // Accepting again changes nothing and consumes no use; a third person
    // finds it used up.
    let again = fx.accept(b, &invite.code).await.unwrap();
    assert_eq!(again.id, m.id);
    assert_eq!(code_of(&fx.accept(c, &invite.code).await.unwrap_err()), Code::FailedPrecondition);
    assert_eq!(fx.check(c, g.id).await, None);
    assert_eq!(code_of(&fx.accept(c, "nosuchcode").await.unwrap_err()), Code::NotFound);

    // Only an admin invites, and never as admin; an admin can only be made.
    assert_eq!(code_of(&fx.invite(b, g.id, Role::Member, 0).await.unwrap_err()), Code::PermissionDenied);
    assert_eq!(code_of(&fx.invite(a, g.id, Role::Admin, 0).await.unwrap_err()), Code::InvalidArgument);
    assert_eq!(code_of(&fx.members(c, g.id).await.unwrap_err()), Code::PermissionDenied);

    // A viewer invite lands a viewer.
    let v = fx.invite(a, g.id, Role::Viewer, 0).await.unwrap();
    assert_eq!(fx.accept(c, &v.code).await.unwrap().role, Role::Viewer);
    assert_eq!(fx.set_role(a, g.id, c, Role::Member).await.unwrap().role, Role::Member);
    assert_eq!(code_of(&fx.set_role(b, g.id, c, Role::Admin).await.unwrap_err()), Code::PermissionDenied);
    fx.finish().await;
}

#[tokio::test]
async fn last_admin_rules() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let g = fx.create_group(a, None).await.unwrap();
    assert_eq!(code_of(&fx.leave(a, g.id).await.unwrap_err()), Code::FailedPrecondition, "the only member deletes instead");
    fx.join(a, g.id, b, Role::Member).await;

    assert_eq!(code_of(&fx.leave(a, g.id).await.unwrap_err()), Code::FailedPrecondition, "the last admin stays");
    assert_eq!(code_of(&fx.set_role(a, g.id, a, Role::Member).await.unwrap_err()), Code::FailedPrecondition, "and is not demoted");
    assert_eq!(code_of(&fx.remove(a, g.id, a).await.unwrap_err()), Code::InvalidArgument, "removing oneself is leaving");

    fx.set_role(a, g.id, b, Role::Admin).await.unwrap();
    let gone = fx.leave(a, g.id).await.unwrap();
    assert_eq!((gone.user_id, gone.role), (a, Role::Admin));
    assert_eq!(fx.check(a, g.id).await, None);
    assert_eq!(fx.members(b, g.id).await.unwrap(), vec![(b, Role::Admin)]);
    let removed = fx.router.removed();
    assert_eq!(removed.len(), 1, "one MemberRemoved per membership that went");
    assert_eq!((removed[0].group_id, removed[0].user_id), (g.id, a));
    fx.finish().await;
}

#[tokio::test]
async fn subgroups_take_parent_members_only() {
    let Some(fx) = fx().await else { return };
    let (a, b, c, d) = (user(), user(), user(), user());
    let g = fx.create_group(a, None).await.unwrap();
    fx.join(a, g.id, b, Role::Member).await;
    assert_eq!(code_of(&fx.create_group(c, Some(g.id)).await.unwrap_err()), Code::PermissionDenied, "a subgroup is made by a member");
    let s = fx.create_group(a, Some(g.id)).await.unwrap();
    assert_eq!(s.parent_group_id, Some(g.id));
    assert_eq!(code_of(&fx.create_group(a, Some(s.id)).await.unwrap_err()), Code::InvalidArgument, "one level for now");

    // An invite to the subgroup admits parent members only.
    let invite = fx.invite(a, s.id, Role::Member, 0).await.unwrap();
    assert_eq!(code_of(&fx.accept(c, &invite.code).await.unwrap_err()), Code::PermissionDenied);
    assert_eq!(fx.accept(b, &invite.code).await.unwrap().role, Role::Member);

    // AddMember: subgroups only, by an admin or member, of a parent member.
    assert_eq!(code_of(&fx.add(a, g.id, c, Role::Member).await.unwrap_err()), Code::FailedPrecondition, "not on a top-level group");
    assert_eq!(code_of(&fx.add(a, s.id, d, Role::Member).await.unwrap_err()), Code::FailedPrecondition, "d is not in the parent");
    fx.join(a, g.id, d, Role::Member).await;
    assert_eq!(code_of(&fx.add(c, s.id, d, Role::Member).await.unwrap_err()), Code::PermissionDenied, "c is not in the subgroup");
    let added = fx.add(b, s.id, d, Role::Viewer).await.unwrap();
    assert_eq!((added.user_id, added.role), (d, Role::Viewer));
    assert_eq!(fx.add(b, s.id, d, Role::Member).await.unwrap().id, added.id, "already in: unchanged");
    assert_eq!(code_of(&fx.add(d, s.id, c, Role::Member).await.unwrap_err()), Code::PermissionDenied, "a viewer adds nobody");
    assert_eq!(fx.groups(d).await, vec![g.id, s.id]);
    fx.finish().await;
}

#[tokio::test]
async fn removal_from_the_parent_cascades_to_the_subgroup() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let g = fx.create_group(a, None).await.unwrap();
    let s = fx.create_group(a, Some(g.id)).await.unwrap();
    fx.join(a, g.id, b, Role::Member).await;
    fx.join(a, s.id, b, Role::Member).await;
    assert_eq!(fx.check(b, s.id).await, Some(Role::Member));

    assert_eq!(code_of(&fx.remove(b, g.id, a).await.unwrap_err()), Code::PermissionDenied);
    let removed = fx.remove(a, g.id, b).await.unwrap();
    assert_eq!((removed.group_id, removed.user_id), (g.id, b));
    assert_eq!(fx.check(b, g.id).await, None);
    assert_eq!(fx.check(b, s.id).await, None, "gone from the subgroup in the same transaction");
    let mut events: Vec<Uuid> = fx.router.removed().iter().map(|e| e.group_id).collect();
    events.sort();
    let mut expected = vec![g.id, s.id];
    expected.sort();
    assert_eq!(events, expected, "one MemberRemoved per group");
    assert_eq!(code_of(&fx.remove(a, g.id, b).await.unwrap_err()), Code::NotFound);
    fx.finish().await;
}

#[tokio::test]
async fn concurrent_accepts_of_a_single_use_invite_admit_exactly_one() {
    let Some(fx) = fx().await else { return };
    let (a, b, c) = (user(), user(), user());
    let g = fx.create_group(a, None).await.unwrap();
    let invite = fx.invite(a, g.id, Role::Member, 1).await.unwrap();
    let (rb, rc) = tokio::join!(fx.accept(b, &invite.code), fx.accept(c, &invite.code));
    let admitted = [rb.is_ok(), rc.is_ok()].iter().filter(|ok| **ok).count();
    assert_eq!(admitted, 1, "b: {rb:?}, c: {rc:?}");
    assert_eq!(fx.members(a, g.id).await.unwrap().len(), 2);
    let uses: i32 = sqlx::query_scalar("SELECT uses FROM nano_group.invite WHERE id = $1")
        .bind(invite.id)
        .fetch_one(fx.db.superuser())
        .await
        .unwrap();
    assert_eq!(uses, 1);
    fx.finish().await;
}

// --- group purge ---

#[tokio::test]
async fn group_purge_waits_for_the_subgroup_then_empties_every_receiver() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    let g = fx.create_group(a, None).await.unwrap();
    let s = fx.create_group(a, Some(g.id)).await.unwrap();
    fx.join(a, g.id, b, Role::Member).await;
    fx.invite(a, g.id, Role::Member, 0).await.unwrap();
    assert_eq!(code_of(&fx.delete_group(b, g.id).await.unwrap_err()), Code::PermissionDenied);

    let deleted = fx.delete_group(a, g.id).await.unwrap();
    assert!(deleted.deleted_at.is_some());
    // From that commit on nobody is a member of either group, and both
    // purge objects exist; a second delete finds no group.
    assert_eq!(fx.check(a, g.id).await, None);
    assert_eq!(fx.check(a, s.id).await, None);
    assert!(fx.groups(a).await.is_empty());
    assert!(fx.group_purge_exists(g.id).await && fx.group_purge_exists(s.id).await);
    assert_eq!(fx.membership_rows(g.id).await + fx.membership_rows(s.id).await, 0);
    assert_eq!(code_of(&fx.delete_group(a, g.id).await.unwrap_err()), Code::PermissionDenied, "the admin's row went with the rest");

    // The parent's pass waits for the subgroup's purge and sends nothing.
    let (obj, done) = fx.drive_group_purge().await.expect("the parent's purge is due");
    assert_eq!(obj.spec.group_id, g.id);
    assert_eq!(outcome_name(&done), "converged");
    assert_eq!(status(&done).map(|s| s.subgroups_remaining), Some(1));
    assert_eq!(fx.router.sent("objects", g.id), 0);

    // The subgroup's pass: three receivers, all empty, so the row goes and
    // the object marks itself; its next pass removes it.
    fx.router.queue("blobs", [Ok(2), Ok(0)]);
    let (obj, done) = fx.drive_group_purge().await.expect("the subgroup's purge is due");
    assert_eq!(obj.spec.group_id, s.id);
    assert_eq!(status(&done).map(|s| (s.objects_remaining, s.blobs_remaining, s.keys_remaining)), Some((Some(0), Some(2), Some(0))));
    assert!(fx.group_row_exists(s.id).await, "a receiver still holds something");
    fx.nudge_group_purge(s.id).await;
    let (_, done) = fx.drive_group_purge().await.unwrap();
    assert_eq!(status(&done).map(|s| s.phase.as_str()), Some(group_purge::phase::DONE));
    assert!(!fx.group_row_exists(s.id).await, "nothing remains: the row went");
    let (obj, done) = fx.drive_group_purge().await.expect("the marked object is due at once");
    assert_eq!(obj.spec.group_id, s.id);
    assert!(is_delete(&done), "{}", outcome_name(&done));
    assert!(!fx.group_purge_exists(s.id).await);
    assert_eq!(fx.router.sent("objects", s.id), 3, "every pass re-asks: level-triggered");

    // The parent, nudged, finds no subgroup left and finishes the same way.
    fx.nudge_group_purge(g.id).await;
    let (_, done) = fx.drive_group_purge().await.unwrap();
    assert_eq!(status(&done).map(|s| (s.subgroups_remaining, s.phase.as_str())), Some((0, group_purge::phase::DONE)));
    let (_, done) = fx.drive_group_purge().await.unwrap();
    assert!(is_delete(&done));
    assert!(!fx.group_row_exists(g.id).await && !fx.group_purge_exists(g.id).await);
    assert!(fx.drive_group_purge().await.is_none(), "nothing is left to drive");
    fx.finish().await;
}

#[tokio::test]
async fn a_lost_purge_result_is_retried_and_the_receiver_asked_again() {
    let Some(fx) = fx().await else { return };
    let a = user();
    let g = fx.create_group(a, None).await.unwrap();
    fx.delete_group(a, g.id).await.unwrap();
    fx.router.queue("keys", [Err(())]);
    let (_, done) = fx.drive_group_purge().await.unwrap();
    assert_eq!(outcome_name(&done), "retry");
    assert!(fx.group_row_exists(g.id).await);
    // Backoff parks it for seconds; the test wakes it instead of waiting.
    fx.nudge_group_purge(g.id).await;
    let (_, done) = fx.drive_group_purge().await.unwrap();
    assert_eq!(outcome_name(&done), "converged");
    assert_eq!(fx.router.sent("keys", g.id), 2);
    assert!(!fx.group_row_exists(g.id).await);
    fx.finish().await;
}

// --- account purge ---

#[tokio::test]
async fn account_purge_promotes_purges_leaves_and_deletes_the_identity_once() {
    let Some(fx) = fx().await else { return };
    let (a, b) = (user(), user());
    // a: sole admin of g1 (b is a member), the only member of g2, a plain
    // member of g3 (b's group).
    let g1 = fx.create_group(a, None).await.unwrap();
    fx.join(a, g1.id, b, Role::Member).await;
    let g2 = fx.create_group(a, None).await.unwrap();
    let g3 = fx.create_group(b, None).await.unwrap();
    fx.join(b, g3.id, a, Role::Member).await;

    let deletion = fx.delete_account(a).await;
    assert_eq!(deletion.user_id, a);
    assert!(deletion.purge_id.starts_with("upr_"), "{}", deletion.purge_id);
    assert_eq!(fx.delete_account(a).await.purge_id, deletion.purge_id, "idempotent");
    assert_eq!(code_of(&fx.create_group(a, None).await.unwrap_err()), Code::FailedPrecondition, "a deleting account makes no calls");
    assert_eq!(fx.check(a, g1.id).await, Some(Role::Admin), "but its memberships still answer until they go");

    // One group per pass, each pass due at once.
    for _ in 0..3 {
        let (_, done) = fx.drive_user_purge().await.expect("due");
        assert_eq!(status(&done).map(|s| s.phase.as_str()), Some(user_purge::phase::LEAVING));
    }
    assert_eq!(fx.check(a, g1.id).await, None);
    assert_eq!(fx.members(b, g1.id).await.unwrap(), vec![(b, Role::Admin)], "the longest-standing member inherited");
    assert_eq!(fx.check(b, g2.id).await, None);
    assert!(fx.group_purge_exists(g2.id).await, "the group a was alone in is being purged");
    assert_eq!(fx.members(b, g3.id).await.unwrap(), vec![(b, Role::Admin)]);
    let mut events: Vec<Uuid> = fx.router.removed().iter().map(|e| e.group_id).collect();
    events.sort();
    let mut expected = vec![g1.id, g3.id];
    expected.sort();
    assert_eq!(events, expected, "a MemberRemoved where others remain, none for the purged group");

    // Devices: a batch left, then none.
    fx.router.queue("devices", [Ok(1), Ok(0)]);
    let (_, done) = fx.drive_user_purge().await.unwrap();
    assert_eq!(status(&done).map(|s| (s.groups_remaining, s.devices_remaining)), Some((0, Some(1))));
    fx.nudge_user_purge(a).await;
    // The identity: sent, then confirmed absent on the next pass.
    let (_, done) = fx.drive_user_purge().await.unwrap();
    assert_eq!(status(&done).map(|s| (s.phase.as_str(), s.identity_deleted)), Some((user_purge::phase::IDENTITY, false)));
    assert_eq!(fx.sim.landed_count("delete_identity"), 1);
    let (_, done) = fx.drive_user_purge().await.unwrap();
    assert_eq!(status(&done).map(|s| (s.phase.as_str(), s.identity_deleted)), Some((user_purge::phase::DONE, true)));
    let (_, done) = fx.drive_user_purge().await.unwrap();
    assert!(is_delete(&done), "{}", outcome_name(&done));
    assert!(!fx.user_purge_exists(a).await);
    assert_eq!(fx.sim.call_count("delete_identity"), 1, "deleted exactly once");
    assert!(fx.drive_user_purge().await.is_none());
    fx.finish().await;
}
