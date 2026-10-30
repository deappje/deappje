//! The conflict rebase (`docs/01-deappje/01-architecture.md`, "Values and
//! conflicts"): the fields a device changed, taken against the row it
//! edited from, re-applied onto the row the server holds now. A singular
//! field is changed as a whole; a repeated field per element, added,
//! removed or replaced by its key. The same field or element changed on
//! both sides goes to the later push, which is the one doing the rebase.
//! Works at the wire level, so fields this client does not know in the
//! current row pass through untouched.

use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{ElementKey, FieldKind, ObjectType, Schema, Strategy};
use crate::wire::{self, Field, Value};

/// The changes a device made to one object, by field number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// A singular field set to a value, or cleared (`None`).
    singular: BTreeMap<u32, Option<Value>>,
    /// A repeated field's elements added or replaced, and removed, by key.
    elements: BTreeMap<u32, ElementChanges>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ElementChanges {
    upserts: Vec<(Vec<u8>, Value)>,
    removed: BTreeSet<Vec<u8>>,
}

impl ChangeSet {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.singular.is_empty() && self.elements.is_empty()
    }
}

/// The key of one element.
fn element_key(key: ElementKey, value: &Value) -> Vec<u8> {
    let whole = || match value {
        Value::Bytes(b) => b.clone(),
        Value::Varint(v) => v.to_le_bytes().to_vec(),
        Value::Fixed64(b) => b.to_vec(),
        Value::Fixed32(b) => b.to_vec(),
    };
    match (key, value) {
        (ElementKey::SubField(n), Value::Bytes(b)) => match wire::decode(b) {
            Ok(fields) => fields.iter().rfind(|f| f.number == n).map_or_else(whole, |f| {
                let mut k = Vec::new();
                wire::put_field(&mut k, f.number, &f.value);
                k
            }),
            Err(_) => whole(),
        },
        _ => whole(),
    }
}

fn elements_by_key(fields: &[Field], number: u32, key: ElementKey) -> Vec<(Vec<u8>, Value)> {
    fields.iter().filter(|f| f.number == number).map(|f| (element_key(key, &f.value), f.value.clone())).collect()
}

fn last(fields: &[Field], number: u32) -> Option<&Value> {
    fields.iter().rfind(|f| f.number == number).map(|f| &f.value)
}

/// The changes from `base` (the row the device edited from) to `local`
/// (what it wrote), field by field under `schema`.
pub fn diff(schema: &Schema, base: &[Field], local: &[Field]) -> ChangeSet {
    let numbers: BTreeSet<u32> = base.iter().chain(local.iter()).map(|f| f.number).collect();
    let mut changes = ChangeSet::default();
    for number in numbers {
        match schema.kind(number) {
            FieldKind::Singular => {
                let (b, l) = (last(base, number), last(local, number));
                if b != l {
                    changes.singular.insert(number, l.cloned());
                }
            }
            FieldKind::Elements(key) => {
                let before: BTreeMap<Vec<u8>, Value> = elements_by_key(base, number, key).into_iter().collect();
                let after = elements_by_key(local, number, key);
                let mut ec = ElementChanges::default();
                for (k, v) in &after {
                    if before.get(k) != Some(v) {
                        ec.upserts.push((k.clone(), v.clone()));
                    }
                }
                let kept: BTreeSet<&Vec<u8>> = after.iter().map(|(k, _)| k).collect();
                for k in before.keys() {
                    if !kept.contains(k) {
                        ec.removed.insert(k.clone());
                    }
                }
                if !ec.upserts.is_empty() || !ec.removed.is_empty() {
                    changes.elements.insert(number, ec);
                }
            }
        }
    }
    changes
}

/// `changes` applied onto `current` (the row the server holds now): the
/// result to push with the current row's seq as its base. Fields the
/// changes do not touch stay as `current` has them, unknown ones included.
pub fn apply(schema: &Schema, current: &[Field], changes: &ChangeSet) -> Vec<Field> {
    let numbers: BTreeSet<u32> = current
        .iter()
        .map(|f| f.number)
        .chain(changes.singular.keys().copied())
        .chain(changes.elements.keys().copied())
        .collect();
    let mut out = Vec::new();
    for number in numbers {
        match schema.kind(number) {
            FieldKind::Singular => match changes.singular.get(&number) {
                Some(Some(v)) => out.push(Field { number, value: v.clone() }),
                Some(None) => {}
                None => out.extend(current.iter().filter(|f| f.number == number).cloned()),
            },
            FieldKind::Elements(key) => {
                let mut elements = elements_by_key(current, number, key);
                if let Some(ec) = changes.elements.get(&number) {
                    elements.retain(|(k, _)| !ec.removed.contains(k));
                    for (k, v) in &ec.upserts {
                        match elements.iter_mut().find(|(ek, _)| ek == k) {
                            Some(slot) => slot.1 = v.clone(),
                            None => elements.push((k.clone(), v.clone())),
                        }
                    }
                }
                out.extend(elements.into_iter().map(|(_, value)| Field { number, value }));
            }
        }
    }
    out
}

/// The value to push again after a conflict on a live row: `local` rebased
/// onto `current` by the type's strategy. A tombstone is the caller's to
/// notice first (the edit is dropped); the result is the caller's to
/// validate (`ObjectType::validate_value`) before it is pushed.
pub fn rebase(object_type: ObjectType, base: &[Field], local: &[Field], current: &[Field]) -> Vec<Field> {
    match object_type.strategy() {
        Strategy::Rebase => {
            let schema = object_type.schema();
            apply(schema, current, &diff(schema, base, local))
        }
        // The later push wins whole: the owner's own devices disagree, and
        // an append-only object meets a conflict only as a stale create.
        Strategy::PerUserSingleton | Strategy::AppendOnly => local.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::schema::Document;
    use crate::types::{Event, Expense, Participant, Poll, PollOption, Reaction, Split, TodoItem};

    fn user(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn two_devices_editing_different_fields_both_keep_their_edits() {
        let base = Document::child_of(user(9), user(1), 100, TodoItem { text: "milk".into(), pos: 1.0, ..TodoItem::default() });
        let mut a = base.clone();
        a.body.done = true;
        a.meta.edited_at = 200;
        let mut b = base.clone();
        b.body.assignee = Some(user(2));
        b.meta.edited_at = 150;
        // B got there first; A rebases onto B's row.
        let merged = rebase(ObjectType::TodoItem, &base.to_fields(), &a.to_fields(), &b.to_fields());
        let merged = Document::<TodoItem>::from_fields(&merged).unwrap();
        assert!(merged.body.done);
        assert_eq!(merged.body.assignee, Some(user(2)));
        assert_eq!(merged.body.text, "milk");
        assert_eq!(merged.meta.edited_at, 200, "the later edit's time");
    }

    #[test]
    fn the_same_field_goes_to_the_later_push() {
        let base = Document::new(user(1), 100, Event { title: "Dinner".into(), starts: 10, ends: 20, ..Event::default() });
        let mut a = base.clone();
        a.body.title = "Dinner at 8".into();
        let mut b = base.clone();
        b.body.title = "Supper".into();
        b.body.location = "Home".into();
        let merged = Document::<Event>::from_fields(&rebase(ObjectType::Event, &base.to_fields(), &a.to_fields(), &b.to_fields())).unwrap();
        assert_eq!(merged.body.title, "Dinner at 8", "the rebasing push is the later one");
        assert_eq!(merged.body.location, "Home", "what it did not touch is the current row's");
    }

    #[test]
    fn a_cleared_field_stays_cleared_and_an_untouched_one_stays() {
        let base = Document::child_of(user(9), user(1), 100, TodoItem { text: "milk".into(), assignee: Some(user(2)), due: Some(5), pos: 1.0, ..TodoItem::default() });
        let mut local = base.clone();
        local.body.assignee = None;
        let mut current = base.clone();
        current.body.due = Some(7);
        let merged = Document::<TodoItem>::from_fields(&rebase(ObjectType::TodoItem, &base.to_fields(), &local.to_fields(), &current.to_fields())).unwrap();
        assert_eq!(merged.body.assignee, None);
        assert_eq!(merged.body.due, Some(7));
    }

    #[test]
    fn elements_are_added_removed_and_replaced_by_key_on_both_sides() {
        let p = |n: u128, share: i64| Participant { user_id: user(n), share };
        let base = Document::child_of(user(9), user(1), 100, Expense { amount: 300, currency: "EUR".into(), paid_by: user(1), participants: vec![p(1, 100), p(2, 100), p(3, 100)], split: Split::Exact, date: 1, ..Expense::default() });
        // Local: removes 3, adds 4, changes 1's share. Current: removed 2, changed 3's share.
        let mut local = base.clone();
        local.body.participants = vec![p(1, 150), p(2, 100), p(4, 50)];
        let mut current = base.clone();
        current.body.participants = vec![p(1, 100), p(3, 200)];
        let merged = Document::<Expense>::from_fields(&rebase(ObjectType::Expense, &base.to_fields(), &local.to_fields(), &current.to_fields())).unwrap();
        assert_eq!(merged.body.participants, vec![p(1, 150), p(4, 50)], "3 removed by local, 2 removed by current, 1 replaced, 4 added");
        let options = |ids: &[u128]| ids.iter().map(|n| PollOption { id: user(*n), text: format!("o{n}"), pos: *n as f64, ..PollOption::default() }).collect::<Vec<_>>();
        let base = Document::new(user(1), 100, Poll { question: "Which?".into(), options: options(&[1, 2]), ..Poll::default() });
        let mut local = base.clone();
        local.body.options = options(&[1, 2, 3]);
        let mut current = base.clone();
        current.body.options[0].text = "first".into();
        let merged = Document::<Poll>::from_fields(&rebase(ObjectType::Poll, &base.to_fields(), &local.to_fields(), &current.to_fields())).unwrap();
        assert_eq!(merged.body.options.iter().map(|o| o.text.as_str()).collect::<Vec<_>>(), vec!["first", "o2", "o3"], "current's change to 1 kept, local's 3 added");
    }

    #[test]
    fn unknown_fields_of_the_current_row_pass_through() {
        let base = Document::new(user(1), 100, Event { title: "Dinner".into(), starts: 10, ends: 20, ..Event::default() });
        let mut local = base.clone();
        local.body.title = "Lunch".into();
        let mut current = base.to_fields();
        current.push(Field { number: 99, value: Value::Bytes(b"newer".to_vec()) });
        let merged = rebase(ObjectType::Event, &base.to_fields(), &local.to_fields(), &current);
        assert!(merged.contains(&Field { number: 99, value: Value::Bytes(b"newer".to_vec()) }));
        assert_eq!(Document::<Event>::from_fields(&merged).unwrap().body.title, "Lunch");
    }

    #[test]
    fn a_per_user_singleton_goes_whole_to_the_later_push() {
        let base = Document::child_of(user(9), user(1), 100, Reaction { emoji: "👍".into() });
        let mut local = base.clone();
        local.body.emoji = "❤️".into();
        let mut current = base.clone();
        current.body.emoji = "😂".into();
        current.meta.edited_at = 150;
        assert_eq!(rebase(ObjectType::Reaction, &base.to_fields(), &local.to_fields(), &current.to_fields()), local.to_fields());
    }

    #[test]
    fn a_concurrent_create_rebases_onto_the_row_that_got_there_first() {
        // base_seq = 0 on both: the base is the empty row.
        let first = Document::new(user(1), 100, crate::types::GroupProfile { name: "Us".into(), description: "the first".into(), ..Default::default() });
        let second = Document::new(user(2), 120, crate::types::GroupProfile { name: "Us".into(), avatar: vec![1, 2, 3], ..Default::default() });
        let merged = Document::<crate::types::GroupProfile>::from_fields(&rebase(ObjectType::GroupProfile, &[], &second.to_fields(), &first.to_fields())).unwrap();
        assert_eq!(merged.body.description, "the first");
        assert_eq!(merged.body.avatar, vec![1, 2, 3]);
        assert_eq!(merged.meta.author, user(2), "every field the second set, it set");
    }

    #[test]
    fn an_invalid_result_is_caught_by_the_type() {
        let base = Document::new(user(1), 100, Event { title: "Trip".into(), starts: 10, ends: 20, ..Event::default() });
        let mut local = base.clone();
        local.body.starts = 25;
        local.body.ends = 30;
        local.body.starts = 25;
        // Local moved the whole event later; current moved only the end earlier.
        let mut local_start_only = base.clone();
        local_start_only.body.starts = 18;
        let mut current = base.clone();
        current.body.ends = 15;
        let merged = rebase(ObjectType::Event, &base.to_fields(), &local_start_only.to_fields(), &current.to_fields());
        assert_eq!(ObjectType::Event.validate_value(&wire::encode(&merged)), Err(crate::types::ValueError::Invalid(crate::schema::Invalid("the event ends before it starts"))));
        let fine = rebase(ObjectType::Event, &base.to_fields(), &local.to_fields(), &current.to_fields());
        assert!(ObjectType::Event.validate_value(&wire::encode(&fine)).is_ok(), "both ends moved locally: both win");
    }

    #[test]
    fn no_change_is_an_empty_set_and_applies_as_identity() {
        let doc = Document::new(user(1), 100, Event { title: "Trip".into(), starts: 10, ends: 20, ..Event::default() });
        let fields = doc.to_fields();
        let changes = diff(ObjectType::Event.schema(), &fields, &fields);
        assert!(changes.is_empty());
        assert_eq!(apply(ObjectType::Event.schema(), &fields, &changes), fields);
        assert_eq!(Document::<Event>::decode(&doc.encode()).unwrap(), doc);
    }
}
