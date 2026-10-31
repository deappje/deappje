//! The sync engine against the in-memory relay model: the conformance
//! cases of `docs/01-deappje/03-testing.md` and the randomised
//! multi-device harness.

use uuid::Uuid;

use super::*;
use crate::harness::{RelayModel, Role, WINDOW_MS, Wakeup, block_on};
use crate::ids;
use crate::keyring::GroupKey;
use crate::schema::Document;
use crate::shell::MemoryStorage;
use crate::types::{Comment, Event as Calendar, Post};

const HOUR: i64 = 60 * 60 * 1000;

/// A device under test: its store, keyring and generator.
struct Dev {
    id: Uuid,
    user: Uuid,
    store: Store,
    keyring: Keyring,
    rng: CoreRng,
    now: i64,
    /// Whether the device receives wake-ups; off, it only pulls, as a
    /// browser does.
    wakeups: bool,
}

impl Dev {
    fn new(n: u8, user: Uuid, model: &RelayModel, group: Uuid, key: &GroupKey) -> Dev {
        let id = Uuid::from_u128(0xd000 + u128::from(n));
        model.register_device(id, user);
        let mut keyring = Keyring::new();
        keyring.insert(group, 1, key.clone());
        Dev { id, user, store: Store::open(Box::new(MemoryStorage::new())), keyring, rng: CoreRng::from_seed([n; 32]), now: 1_000, wakeups: true }
    }

    fn syncer(&mut self) -> Syncer<'_> {
        Syncer { device_id: self.id, store: &mut self.store, keyring: &self.keyring, rng: &mut self.rng }
    }

    fn deliver(&mut self, model: &RelayModel) {
        for w in model.take_wakeups(self.id) {
            if let Wakeup::HistoryReady { request_id, through_seq, complete } = w {
                self.syncer().history_ready(request_id, through_seq, complete);
            }
        }
    }

    fn try_sync(&mut self, model: &RelayModel, group: Uuid) -> Result<Vec<Event>, SyncError> {
        if self.wakeups {
            self.deliver(model);
        } else {
            model.take_wakeups(self.id);
        }
        block_on(self.syncer().sync_group(model, group))
    }

    fn sync(&mut self, model: &RelayModel, group: Uuid) -> Vec<Event> {
        self.try_sync(model, group).unwrap_or_else(|e| panic!("sync of device {}: {e}", self.id))
    }

    fn serve(&mut self, model: &RelayModel, group: Uuid) -> Vec<Event> {
        block_on(self.syncer().serve_history(model, group)).unwrap_or_else(|e| panic!("serve of device {}: {e}", self.id))
    }

    fn post(&mut self, group: Uuid, text: &str) -> Uuid {
        let id = ids::random(&mut self.rng);
        let doc = Document::new(self.user, self.now, Post { text: text.into() });
        self.syncer().create(group, id, ObjectType::Post, doc.encode()).unwrap();
        id
    }

    fn comment(&mut self, group: Uuid, parent: Uuid, text: &str) -> Uuid {
        let id = ids::random(&mut self.rng);
        let doc = Document::child_of(parent, self.user, self.now, Comment { text: text.into() });
        self.syncer().create(group, id, ObjectType::Comment, doc.encode()).unwrap();
        id
    }

    fn event(&mut self, group: Uuid, body: Calendar) -> Uuid {
        let id = ids::random(&mut self.rng);
        let doc = Document::new(self.user, self.now, body);
        self.syncer().create(group, id, ObjectType::Event, doc.encode()).unwrap();
        id
    }

    fn read<T: crate::schema::Object>(&self, group: Uuid, id: Uuid) -> Option<Document<T>> {
        let plain = self.syncer_ro().read(group, id).unwrap()?;
        Some(Document::<T>::decode(&plain).unwrap())
    }

    fn syncer_ro(&self) -> SyncerRo<'_> {
        SyncerRo { store: &self.store, keyring: &self.keyring }
    }

    fn edit_with<T: crate::schema::Object>(&mut self, group: Uuid, id: Uuid, f: impl FnOnce(&mut T)) {
        let mut doc: Document<T> = self.read(group, id).expect("held");
        f(&mut doc.body);
        doc.meta.edited_at = self.now.max(doc.meta.authored_at);
        self.syncer().edit(group, id, doc.encode()).unwrap();
    }

    fn delete(&mut self, group: Uuid, id: Uuid) {
        self.syncer().delete(group, id).unwrap();
    }

    fn live(&self, group: Uuid) -> std::collections::BTreeMap<Uuid, (i64, Vec<u8>)> {
        self.store.group(group).map(|g| g.live()).unwrap_or_default()
    }

    fn phase(&self, group: Uuid) -> Phase {
        self.store.group(group).map(|g| g.phase.clone()).unwrap_or(Phase::New)
    }

    fn outbox_len(&self, group: Uuid) -> usize {
        self.store.group(group).map_or(0, |g| g.outbox.len())
    }
}

/// A read-only view, for `read` without a mutable borrow.
struct SyncerRo<'a> {
    store: &'a Store,
    keyring: &'a Keyring,
}

impl SyncerRo<'_> {
    fn read(&self, group: Uuid, id: Uuid) -> Result<Option<Vec<u8>>, SyncError> {
        match self.store.pending(group, id).map(|p| &p.op) {
            Some(Op::Create { plain, .. } | Op::Edit { plain, .. }) => return Ok(Some(plain.clone())),
            Some(Op::Delete { .. }) => return Ok(None),
            None => {}
        }
        match self.store.row(group, id) {
            Some(Row { deleted: false, value: Some(value), .. }) => {
                let envelope = Envelope::decode(value).map_err(SyncError::Envelope)?;
                let key = self.keyring.get(group, envelope.header.key_id).ok_or(SyncError::NoKey { group_id: group, key_id: envelope.header.key_id })?;
                Ok(Some(envelope.open(key, group, id).map_err(SyncError::Crypto)?))
            }
            _ => Ok(None),
        }
    }
}

struct World {
    model: RelayModel,
    group: Uuid,
    key: GroupKey,
}

impl World {
    fn new(seed: u8) -> World {
        let model = RelayModel::new([seed; 32]);
        let group = Uuid::from_u128(0x6);
        let key = GroupKey::mint(&mut CoreRng::from_seed([seed; 32]));
        World { model, group, key }
    }

    fn device(&self, n: u8, user: u128) -> Dev {
        let user = Uuid::from_u128(user);
        self.model.add_member(self.group, user, Role::Member);
        Dev::new(n, user, &self.model, self.group, &self.key)
    }
}

#[test]
fn two_devices_converge_on_a_post_and_a_comment() {
    let w = World::new(1);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    let post = a.post(w.group, "hello");
    let events = a.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder));
    assert!(events.contains(&Event::Accepted { id: post, seq: 1 }));
    assert_eq!(a.phase(w.group), Phase::Holder);
    assert!(b.sync(&w.model, w.group).contains(&Event::Applied { id: post, seq: 1, deleted: false }));
    assert_eq!(b.read::<Post>(w.group, post).unwrap().body.text, "hello");
    let comment = b.comment(w.group, post, "hi!");
    b.sync(&w.model, w.group);
    a.sync(&w.model, w.group);
    assert_eq!(a.read::<Comment>(w.group, comment).unwrap().meta.parent_id, Some(post));
    assert_eq!(a.live(w.group), b.live(w.group));
    assert_eq!(w.model.ack(w.group, a.id), Some(2));
    assert_eq!(w.model.ack(w.group, b.id), Some(2));
}

#[test]
fn different_fields_both_kept_and_the_same_field_to_the_later_push() {
    let w = World::new(2);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    let event = a.event(w.group, Calendar { title: "Dinner".into(), starts: 10, ends: 20, ..Calendar::default() });
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    // Offline, A changes the title and B the location; B pushes first.
    a.edit_with::<Calendar>(w.group, event, |e| e.title = "Dinner at 8".into());
    b.edit_with::<Calendar>(w.group, event, |e| e.location = "Home".into());
    b.sync(&w.model, w.group);
    let events = a.sync(&w.model, w.group);
    assert!(events.iter().any(|e| matches!(e, Event::Accepted { id, seq: 3 } if *id == event)), "{events:?}");
    b.sync(&w.model, w.group);
    for d in [&a, &b] {
        let doc = d.read::<Calendar>(w.group, event).unwrap();
        assert_eq!((doc.body.title.as_str(), doc.body.location.as_str()), ("Dinner at 8", "Home"));
    }
    // The same field: A's push comes later and wins.
    a.edit_with::<Calendar>(w.group, event, |e| e.title = "Supper".into());
    b.edit_with::<Calendar>(w.group, event, |e| e.title = "Brunch".into());
    b.sync(&w.model, w.group);
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    assert_eq!(b.read::<Calendar>(w.group, event).unwrap().body.title, "Supper");
    assert_eq!(a.live(w.group), b.live(w.group));
}

#[test]
fn a_tombstone_returned_during_a_retry_ends_the_edit() {
    let w = World::new(3);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    let post = a.post(w.group, "to go");
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    a.edit_with::<Post>(w.group, post, |p| p.text = "edited offline".into());
    b.delete(w.group, post);
    b.sync(&w.model, w.group);
    let events = a.sync(&w.model, w.group);
    assert!(events.contains(&Event::EditDropped { id: post }), "{events:?}");
    assert_eq!(a.read::<Post>(w.group, post), None, "shown as deleted");
    assert_eq!(a.outbox_len(w.group), 0);
    assert!(w.model.log(w.group).iter().all(|e| e.deleted), "nothing live came back");
}

#[test]
fn an_invalid_rebase_result_is_shown_not_pushed() {
    let w = World::new(4);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    let event = a.event(w.group, Calendar { title: "Trip".into(), starts: 10, ends: 20, ..Calendar::default() });
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    a.edit_with::<Calendar>(w.group, event, |e| e.starts = 18);
    b.edit_with::<Calendar>(w.group, event, |e| e.ends = 15);
    b.sync(&w.model, w.group);
    let events = a.sync(&w.model, w.group);
    assert!(events.contains(&Event::Invalid { id: event, reason: "the event ends before it starts".into() }), "{events:?}");
    assert_eq!(w.model.log(w.group)[0].seq, 2, "nothing was pushed");
    assert_eq!(a.outbox_len(w.group), 1, "the edit waits for the user");
    let shown = a.read::<Calendar>(w.group, event).unwrap();
    assert_eq!((shown.body.starts, shown.body.ends), (18, 15), "the user sees the merged result");
    a.edit_with::<Calendar>(w.group, event, |e| e.ends = 30);
    let events = a.sync(&w.model, w.group);
    assert!(events.iter().any(|e| matches!(e, Event::Accepted { id, .. } if *id == event)), "{events:?}");
    b.sync(&w.model, w.group);
    assert_eq!(b.read::<Calendar>(w.group, event).unwrap().body.ends, 30);
}

#[test]
fn deleting_a_parent_batches_the_children_and_a_late_child_is_dead() {
    let w = World::new(5);
    let (mut a, mut b, mut c) = (w.device(1, 1), w.device(2, 2), w.device(3, 2));
    let post = a.post(w.group, "pics");
    let c1 = a.comment(w.group, post, "one");
    let c2 = a.comment(w.group, post, "two");
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    c.sync(&w.model, w.group);
    b.delete(w.group, post);
    assert_eq!(b.outbox_len(w.group), 3, "the children B holds go in the same batch");
    b.sync(&w.model, w.group);
    let log = w.model.log(w.group);
    assert_eq!(log.iter().filter(|e| e.deleted).map(|e| e.id).collect::<std::collections::BTreeSet<_>>(), [post, c1, c2].into_iter().collect());
    // A comment pushed under the dead post by a device that had pulled
    // before the delete landed: the first device to see both tombstones it.
    let late = ids::random(&mut a.rng);
    let doc = Document::child_of(post, a.user, a.now, Comment { text: "late".into() });
    let sealed = a.syncer().seal(w.group, late, ObjectType::Comment, &doc.encode()).unwrap();
    let pushed = block_on(w.model.push_objects(a.id, w.group, vec![crate::relay::PushEntry { id: late, value: Some(sealed), base_seq: 0, deleted: false }])).unwrap();
    assert!(matches!(pushed, crate::relay::PushResult::Accepted { ref accepted, .. } if accepted.len() == 1));
    let events = c.sync(&w.model, w.group);
    assert!(events.contains(&Event::Applied { id: late, seq: 7, deleted: true }), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Accepted { id, .. } if *id == late)), "C pushed its tombstone: {events:?}");
    assert!(w.model.log(w.group).iter().find(|e| e.id == late).unwrap().deleted);
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    assert!(a.live(w.group).is_empty() && b.live(w.group).is_empty() && c.live(w.group).is_empty());
}

#[test]
fn a_new_device_takes_the_history_path_and_replays_its_outbox() {
    let w = World::new(6);
    let (mut a, mut a2) = (w.device(1, 1), w.device(2, 1));
    let posts: Vec<Uuid> = (0..3).map(|i| a.post(w.group, &format!("post {i}"))).collect();
    a.sync(&w.model, w.group);
    a2.sync(&w.model, w.group);
    w.model.evict();
    assert_eq!(w.model.head(w.group), (3, 3), "every device acked: the rows went");
    assert!(w.model.log(w.group).is_empty());
    let mut b = w.device(3, 2);
    let mine = b.post(w.group, "from the newcomer");
    let events = b.sync(&w.model, w.group);
    assert!(matches!(events.as_slice(), [Event::HistoryRequested { .. }]), "{events:?}");
    assert_eq!(b.phase(w.group), Phase::CatchingUp { request_id: match events[0] { Event::HistoryRequested { request_id } => Some(request_id), _ => None }, floor: 3, through_seq: 0, complete: false });
    assert_eq!(b.sync(&w.model, w.group), vec![], "waiting for a holder");
    assert!(matches!(w.model.take_wakeups(a.id).as_slice(), [Wakeup::HistoryRequested { .. }]));
    let served = a.serve(&w.model, w.group);
    assert!(matches!(served.as_slice(), [Event::HistoryServed { rows: 3, .. }]), "{served:?}");
    assert_eq!(a2.serve(&w.model, w.group), vec![], "nothing left to claim");
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder), "{events:?}");
    assert!(events.contains(&Event::Accepted { id: mine, seq: 4 }), "the outbox replayed: {events:?}");
    assert_eq!(b.phase(w.group), Phase::Holder);
    assert_eq!(w.model.ack(w.group, b.id), Some(4));
    for p in &posts {
        assert!(b.read::<Post>(w.group, *p).is_some());
    }
    a.sync(&w.model, w.group);
    a2.sync(&w.model, w.group);
    assert_eq!(a.live(w.group), b.live(w.group));
    assert_eq!(a2.live(w.group), b.live(w.group));
}

#[test]
fn a_device_evicted_while_away_starts_over_and_keeps_its_outbox() {
    let w = World::new(7);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    let post = a.post(w.group, "old");
    let doomed = a.post(w.group, "doomed");
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    // B edits offline and stays away for the whole window; A keeps going.
    b.edit_with::<Post>(w.group, post, |p| p.text = "old, edited away".into());
    b.edit_with::<Post>(w.group, doomed, |p| p.text = "doomed, edited away".into());
    a.delete(w.group, doomed);
    a.sync(&w.model, w.group);
    w.model.advance(WINDOW_MS + HOUR);
    a.post(w.group, "new");
    a.sync(&w.model, w.group);
    w.model.evict();
    let (head, floor) = w.model.head(w.group);
    assert_eq!((head, floor), (4, 3), "rows 1-3 went by age; the new row stays");
    assert_eq!(w.model.ack(w.group, b.id), None, "B lost its row");
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::StartedOver), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::HistoryRequested { .. })), "{events:?}");
    assert_eq!(b.outbox_len(w.group), 2, "the outbox survives");
    a.serve(&w.model, w.group);
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder), "{events:?}");
    assert!(events.contains(&Event::EditDropped { id: doomed }), "the deleted one is dropped: {events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Accepted { id, seq: 5 } if *id == post)), "the live one is rebased and pushed: {events:?}");
    a.sync(&w.model, w.group);
    assert_eq!(a.read::<Post>(w.group, post).unwrap().body.text, "old, edited away");
    assert_eq!(a.live(w.group), b.live(w.group));
}

#[test]
fn an_outage_leaves_the_store_consistent_and_the_next_sync_finishes() {
    let w = World::new(8);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    a.post(w.group, "one");
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    a.post(w.group, "two");
    w.model.set_outage(true);
    assert!(matches!(a.try_sync(&w.model, w.group), Err(SyncError::Relay(ApiError { code: crate::api::Code::Unavailable, .. }))));
    assert_eq!(a.outbox_len(w.group), 1);
    w.model.set_outage(false);
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    assert_eq!(a.live(w.group), b.live(w.group));
    assert_eq!(a.live(w.group).len(), 2);
}

/// The `history_ready` wake-up is a hurry-up: a device that never gets it
/// (a browser, a phone without push, a dropped push) reads its request on
/// its next sync and finishes the same way.
#[test]
fn a_lost_history_ready_wake_up_costs_only_the_next_sync() {
    let w = World::new(9);
    let mut a = w.device(1, 1);
    let posts: Vec<Uuid> = (0..3).map(|i| a.post(w.group, &format!("post {i}"))).collect();
    a.sync(&w.model, w.group);
    w.model.evict();
    assert_eq!(w.model.head(w.group), (3, 3), "the only device acked: the rows went");
    let mut b = w.device(2, 2);
    b.wakeups = false;
    let events = b.sync(&w.model, w.group);
    assert!(matches!(events.as_slice(), [Event::HistoryRequested { .. }]), "{events:?}");
    assert_eq!(b.sync(&w.model, w.group), vec![], "open: nothing to page yet");
    a.serve(&w.model, w.group);
    assert!(w.model.take_wakeups(b.id).iter().any(|w| matches!(w, Wakeup::HistoryReady { complete: true, .. })), "the wake-up went out, and is dropped here");
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder), "the request itself says complete: {events:?}");
    assert_eq!(w.model.ack(w.group, b.id), Some(3));
    for p in &posts {
        assert!(b.read::<Post>(w.group, *p).is_some());
    }
}

/// A request nobody answered within the window is swept, and the sweep of
/// old rows in the same hour drops the requester's ack row and moves the
/// floor: the requester finds its request gone, starts over (ack 0 again)
/// and asks up to the floor as it is now, so one upload brings everything.
#[test]
fn a_swept_request_starts_over() {
    let w = World::new(11);
    let mut a = w.device(1, 1);
    let old = a.post(w.group, "old");
    a.sync(&w.model, w.group);
    w.model.evict();
    let mut b = w.device(2, 2);
    let events = b.sync(&w.model, w.group);
    let first = match events.as_slice() {
        [Event::HistoryRequested { request_id }] => *request_id,
        other => panic!("{other:?}"),
    };
    // A writes on while B waits; nobody serves, and a window passes.
    let new = a.post(w.group, "new");
    a.sync(&w.model, w.group);
    w.model.advance(WINDOW_MS + HOUR);
    w.model.evict();
    w.model.expire_requests();
    assert_eq!(w.model.head(w.group), (2, 2), "the new row went by age");
    assert_eq!(w.model.ack(w.group, b.id), None, "B lost its row with it");
    let events = b.sync(&w.model, w.group);
    assert_eq!(events, vec![Event::StartedOver], "the request is gone");
    assert_eq!(b.phase(w.group), Phase::New);
    let events = b.sync(&w.model, w.group);
    let second = match events.as_slice() {
        [Event::HistoryRequested { request_id }] => *request_id,
        other => panic!("{other:?}"),
    };
    assert_ne!(first, second, "a new request");
    assert_eq!(b.phase(w.group), Phase::CatchingUp { request_id: Some(second), floor: 2, through_seq: 0, complete: false }, "up to the floor as it is now");
    assert_eq!(w.model.ack(w.group, b.id), Some(0), "the row is back, holding the restore");
    let served = a.serve(&w.model, w.group);
    assert!(matches!(served.as_slice(), [Event::HistoryServed { rows: 2, .. }]), "one upload: {served:?}");
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder), "{events:?}");
    assert!(b.read::<Post>(w.group, old).is_some() && b.read::<Post>(w.group, new).is_some());
}

/// A device waiting on a holder fetches only what is new since its last
/// pull, not the window again, and the whole stretch once complete.
#[test]
fn a_waiting_device_does_not_fetch_the_window_again() {
    let w = World::new(13);
    let mut a = w.device(1, 1);
    for i in 0..3 {
        a.post(w.group, &format!("old {i}"));
    }
    a.sync(&w.model, w.group);
    w.model.evict();
    for i in 0..5 {
        a.post(w.group, &format!("window {i}"));
    }
    a.sync(&w.model, w.group);
    assert_eq!(w.model.head(w.group), (8, 3));
    let mut b = w.device(2, 2);
    let events = b.sync(&w.model, w.group);
    assert!(matches!(events.last(), Some(Event::HistoryRequested { .. })), "{events:?}");
    let pulled = w.model.pulled();
    assert_eq!(b.sync(&w.model, w.group), vec![], "waiting");
    assert_eq!(w.model.pulled(), pulled, "nothing new: nothing fetched");
    a.post(w.group, "while waiting");
    a.sync(&w.model, w.group);
    let pulled = w.model.pulled();
    let events = b.sync(&w.model, w.group);
    assert!(matches!(events.as_slice(), [Event::Applied { seq: 9, .. }]), "the new row still shows: {events:?}");
    assert_eq!(w.model.pulled(), pulled + 1, "and only it is fetched");
    a.serve(&w.model, w.group);
    let pulled = w.model.pulled();
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder), "{events:?}");
    assert_eq!(w.model.pulled(), pulled + 9, "complete: the whole stretch from the cursor, once");
    assert_eq!(b.live(w.group), a.live(w.group));
}

/// A device joining shortly after another needs no upload: the first
/// restore's rows are still in the window, the relay vouches for them, and
/// eviction by ack leaves the group alone for the grace.
#[test]
fn a_second_joiner_within_the_grace_needs_no_upload() {
    let w = World::new(12);
    let mut a = w.device(1, 1);
    let posts: Vec<Uuid> = (0..3).map(|i| a.post(w.group, &format!("post {i}"))).collect();
    a.sync(&w.model, w.group);
    w.model.evict();
    assert!(w.model.log(w.group).is_empty());
    let mut b = w.device(2, 2);
    b.sync(&w.model, w.group);
    a.serve(&w.model, w.group);
    w.model.take_wakeups(a.id);
    let events = b.sync(&w.model, w.group);
    assert!(events.contains(&Event::BecameHolder), "{events:?}");

    w.model.advance(10 * 60 * 1000);
    w.model.evict();
    assert_eq!(w.model.log(w.group).len(), 3, "within the grace the restored rows stay, though every device acked them");
    let mut c = w.device(3, 3);
    let events = c.sync(&w.model, w.group);
    assert!(events.contains(&Event::HistoryPresent) && events.contains(&Event::BecameHolder), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, Event::HistoryRequested { .. })), "{events:?}");
    assert!(w.model.take_wakeups(a.id).iter().all(|w| !matches!(w, Wakeup::HistoryRequested { .. })), "no holder was asked");
    assert_eq!(w.model.ack(w.group, c.id), Some(3));
    for p in &posts {
        assert!(c.read::<Post>(w.group, *p).is_some());
    }

    // Past the grace the sweep evicts again, and the next joiner needs a holder.
    w.model.advance(HOUR);
    w.model.evict();
    assert!(w.model.log(w.group).is_empty());
    let mut d = w.device(4, 4);
    let events = d.sync(&w.model, w.group);
    assert!(events.iter().any(|e| matches!(e, Event::HistoryRequested { .. })), "{events:?}");
}

/// Two devices joining together are served by one upload: the second
/// request is covered by the first and completes with it.
#[test]
fn two_joiners_at_once_are_served_by_one_upload() {
    let w = World::new(13);
    let mut a = w.device(1, 1);
    let posts: Vec<Uuid> = (0..3).map(|i| a.post(w.group, &format!("post {i}"))).collect();
    a.sync(&w.model, w.group);
    w.model.evict();
    let (mut b, mut c) = (w.device(2, 2), w.device(3, 3));
    b.sync(&w.model, w.group);
    c.sync(&w.model, w.group);
    assert_eq!(w.model.open_requests(w.group), 2);
    assert_eq!(w.model.take_wakeups(a.id).iter().filter(|w| matches!(w, Wakeup::HistoryRequested { .. })).count(), 1, "the covered request is not announced");
    let served = a.serve(&w.model, w.group);
    assert_eq!(served.iter().filter(|e| matches!(e, Event::HistoryServed { .. })).count(), 1, "one upload: {served:?}");
    assert_eq!(w.model.open_requests(w.group), 0, "the covered request completed with the covering one");
    for dev in [&mut b, &mut c] {
        let events = dev.sync(&w.model, w.group);
        assert!(events.contains(&Event::BecameHolder), "{events:?}");
        for p in &posts {
            assert!(dev.read::<Post>(w.group, *p).is_some());
        }
    }
}

/// Random push/pull/ack/offline interleavings, with eviction and a holder
/// serving history, over four devices of two users: every device ends
/// with the same rows.
#[test]
fn the_multi_device_harness_converges() {
    for seed in 10..50u8 {
        run_harness(seed, true);
    }
}

/// The same with every wake-up dropped, as a browser or a phone without
/// push sees the world: a device catching up follows its request through
/// the relay alone.
#[test]
fn the_multi_device_harness_converges_without_wake_ups() {
    for seed in 50..70u8 {
        run_harness(seed, false);
    }
}

fn run_harness(seed: u8, wakeups: bool) {
    let w = World::new(seed);
    let mut devs = vec![w.device(1, 1), w.device(2, 1), w.device(3, 2), w.device(4, 2)];
    for dev in &mut devs {
        dev.wakeups = wakeups;
    }
    let mut rng = CoreRng::from_seed([seed; 32]);
    let pick = |n: u64, rng: &mut CoreRng| (rand_core::RngCore::next_u64(rng) % n) as usize;
    let mut away: Option<(usize, usize)> = None; // (device, steps left)
    let mut outage_steps = 0;
    for step in 0..400 {
        if step == 150 || step == 300 {
            // One device goes away for longer than the window.
            away = Some((pick(4, &mut rng), 60));
            w.model.advance(WINDOW_MS + HOUR);
            w.model.evict();
            w.model.expire_requests();
        }
        if let Some((d, left)) = away {
            away = if left == 0 { None } else { Some((d, left - 1)) };
        }
        if outage_steps > 0 {
            outage_steps -= 1;
            if outage_steps == 0 {
                w.model.set_outage(false);
            }
        }
        let i = pick(4, &mut rng);
        if away.is_some_and(|(d, _)| d == i) {
            continue;
        }
        let dev = &mut devs[i];
        let group = w.group;
        match pick(100, &mut rng) {
            0..=29 => {
                let _ = dev.try_sync(&w.model, group);
            }
            30..=49 => {
                dev.now += 1;
                let posts: Vec<Uuid> = dev.store.group(group).map(|g| g.rows.iter().filter(|(_, r)| !r.deleted && r.object_type == Some(ObjectType::Post)).map(|(id, _)| *id).collect()).unwrap_or_default();
                if !posts.is_empty() && pick(3, &mut rng) == 0 {
                    let parent = posts[pick(posts.len() as u64, &mut rng)];
                    dev.comment(group, parent, &format!("c{step}"));
                } else {
                    dev.post(group, &format!("p{step}"));
                }
            }
            50..=64 => {
                dev.now += 1;
                let posts: Vec<Uuid> = dev.store.group(group).map(|g| g.rows.iter().filter(|(_, r)| !r.deleted && r.object_type == Some(ObjectType::Post)).map(|(id, _)| *id).collect()).unwrap_or_default();
                if !posts.is_empty() {
                    let id = posts[pick(posts.len() as u64, &mut rng)];
                    if dev.read::<Post>(group, id).is_some() {
                        dev.edit_with::<Post>(group, id, |p| p.text = format!("e{step}"));
                    }
                }
            }
            65..=72 => {
                let live: Vec<Uuid> = dev.store.group(group).map(|g| g.rows.iter().filter(|(_, r)| !r.deleted).map(|(id, _)| *id).collect()).unwrap_or_default();
                if !live.is_empty() {
                    let id = live[pick(live.len() as u64, &mut rng)];
                    dev.delete(group, id);
                }
            }
            73..=84 => {
                let _ = block_on(dev.syncer().serve_history(&w.model, group));
            }
            85..=94 => {
                w.model.advance(HOUR);
                w.model.evict();
                w.model.expire_requests();
            }
            _ => {
                w.model.set_outage(true);
                outage_steps = 3;
            }
        }
    }
    w.model.set_outage(false);
    // Everyone comes back: sync and serve until nothing moves.
    let mut quiet = 0;
    for _round in 0..40 {
        let mut moved = false;
        for dev in devs.iter_mut() {
            let events = dev.sync(&w.model, w.group);
            moved |= !events.is_empty();
            moved |= !dev.serve(&w.model, w.group).is_empty();
        }
        if moved {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 2 {
                break;
            }
        }
    }
    let (head, _) = w.model.head(w.group);
    for dev in &devs {
        assert_eq!(dev.phase(w.group), Phase::Holder, "seed {seed}: device {} is not a holder", dev.id);
        assert_eq!(dev.outbox_len(w.group), 0, "seed {seed}: device {} has an outbox", dev.id);
        assert_eq!(w.model.ack(w.group, dev.id), Some(head), "seed {seed}: device {} is not at the head", dev.id);
    }
    let first = devs[0].live(w.group);
    assert!(!first.is_empty(), "seed {seed}: nothing survived");
    for dev in &devs[1..] {
        assert_eq!(dev.live(w.group), first, "seed {seed}: device {} differs", dev.id);
    }
    for entry in w.model.log(w.group) {
        if !entry.deleted {
            assert_eq!(first.get(&entry.id), Some(&(entry.seq, entry.value.clone().unwrap())), "seed {seed}: the server's row {} is not what the devices hold", entry.id);
        }
    }
    assert_eq!(w.model.open_requests(w.group), 0, "seed {seed}");
}

#[test]
fn two_devices_editing_a_note_offline_see_the_same_body() {
    let w = World::new(9);
    let (mut a, mut b) = (w.device(1, 1), w.device(2, 2));
    let (ua, ub, now) = (a.user, b.user, a.now);
    let note = ids::random(&mut a.rng);
    let doc = Document::new(ua, now, crate::types::Note { title: "Lists".into() });
    a.syncer().create(w.group, note, ObjectType::Note, doc.encode()).unwrap();
    let mut body = a.syncer().open_note(w.group, note).unwrap();
    body.set_text("Shopping:\n");
    assert!(a.syncer().save_note_session(w.group, note, &mut body, ua, now).unwrap().is_some());
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    // Offline, A adds an item and B changes the heading.
    let mut on_a = a.syncer().open_note(w.group, note).unwrap();
    on_a.set_text("Shopping:\nmilk\n");
    a.syncer().save_note_session(w.group, note, &mut on_a, ua, now).unwrap();
    let mut on_b = b.syncer().open_note(w.group, note).unwrap();
    on_b.set_text("Groceries:\n");
    b.syncer().save_note_session(w.group, note, &mut on_b, ub, now).unwrap();
    assert!(b.syncer().save_note_session(w.group, note, &mut on_b, ub, now).unwrap().is_none(), "nothing new since");
    b.sync(&w.model, w.group);
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    let text_a = a.syncer().open_note(w.group, note).unwrap().text();
    let text_b = b.syncer().open_note(w.group, note).unwrap().text();
    assert_eq!(text_a, text_b);
    assert_eq!(text_a, "Groceries:\nmilk\n");
    assert_eq!(a.store.group(w.group).unwrap().children_of(note).len(), 3, "three sessions, three children");
    // Deleting the note tombstones the children by cascade.
    a.delete(w.group, note);
    a.sync(&w.model, w.group);
    b.sync(&w.model, w.group);
    assert!(b.live(w.group).is_empty());
}
