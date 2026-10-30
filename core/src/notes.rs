//! The note body: a `yrs` document combined from the note's `note_update`
//! children, each one the update an editing session produced
//! (`docs/01-deappje/01-architecture.md`, "Notes are the exception"). The
//! children are append-only and apply in any order; a session's update is
//! the delta since it began, and a device batches a session into one
//! child. A shell hands the whole text after an edit; the core finds the
//! changed span and applies it, so two devices editing different parts
//! offline both keep their words.

use std::fmt;

use uuid::Uuid;
use yrs::updates::decoder::Decode;
use yrs::{ClientID, Doc, GetString, Options, ReadTxn, StateVector, Text, TextRef, Transact, Update};

use crate::rng::CoreRng;
use crate::schema::{Document, ObjectType};
use crate::store::Row;
use crate::sync::{SyncError, Syncer};
use crate::types::NoteUpdate;

/// The name of the one text in a note's document.
const BODY: &str = "body";

/// Why an update did not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateError(pub String);

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "note update: {}", self.0)
    }
}

impl std::error::Error for UpdateError {}

/// A note's body, open for one editing session.
pub struct NoteDoc {
    doc: Doc,
    text: TextRef,
    session_start: StateVector,
}

impl NoteDoc {
    /// An empty document with a client id from the generator (every open
    /// is a new client to the CRDT, so no two sessions share one). Yjs
    /// client ids are 53 bits, a JavaScript number.
    pub fn new(rng: &mut CoreRng) -> NoteDoc {
        let mut id = [0u8; 8];
        rng.fill(&mut id);
        let client_id = u64::from_le_bytes(id) & ((1 << 53) - 1);
        let doc = Doc::with_options(Options { client_id: ClientID::new(client_id), offset_kind: yrs::OffsetKind::Bytes, ..Options::default() });
        let text = doc.get_or_insert_text(BODY);
        let session_start = doc.transact().state_vector();
        NoteDoc { doc, text, session_start }
    }

    /// Applies one update (a `note_update` child), whenever it arrives. One
    /// arriving mid-session folds into the session's delta, which is
    /// harmless: applying an update twice changes nothing.
    pub fn apply(&mut self, update: &[u8]) -> Result<(), UpdateError> {
        let update = Update::decode_v1(update).map_err(|e| UpdateError(e.to_string()))?;
        self.doc.transact_mut().apply_update(update).map_err(|e| UpdateError(e.to_string()))
    }

    /// The text now.
    pub fn text(&self) -> String {
        self.text.get_string(&self.doc.transact())
    }

    /// Sets the whole text: the changed span (after the common prefix,
    /// before the common suffix) is deleted and the new one inserted, so
    /// the rest of the text keeps its identity for the merge.
    pub fn set_text(&mut self, new: &str) {
        let old = self.text();
        if old == new {
            return;
        }
        let prefix = old.char_indices().zip(new.char_indices()).take_while(|((_, a), (_, b))| a == b).count();
        let prefix_bytes = old.char_indices().nth(prefix).map_or(old.len(), |(i, _)| i);
        let old_rest = &old[prefix_bytes..];
        let new_rest = &new[old.char_indices().nth(prefix).map_or(new.len().min(prefix_bytes), |_| prefix_bytes)..];
        let suffix = old_rest.chars().rev().zip(new_rest.chars().rev()).take_while(|(a, b)| a == b).count();
        let old_suffix_bytes: usize = old_rest.chars().rev().take(suffix).map(char::len_utf8).sum();
        let new_suffix_bytes: usize = new_rest.chars().rev().take(suffix).map(char::len_utf8).sum();
        let removed = &old_rest[..old_rest.len() - old_suffix_bytes];
        let inserted = &new_rest[..new_rest.len() - new_suffix_bytes];
        let mut txn = self.doc.transact_mut();
        if !removed.is_empty() {
            self.text.remove_range(&mut txn, prefix_bytes as u32, removed.len() as u32);
        }
        if !inserted.is_empty() {
            self.text.insert(&mut txn, prefix_bytes as u32, inserted);
        }
    }

    /// The session's update: everything since the session began, or none
    /// when nothing changed.
    pub fn session_update(&self) -> Option<Vec<u8>> {
        let txn = self.doc.transact();
        if txn.state_vector() == self.session_start {
            return None;
        }
        Some(txn.encode_state_as_update_v1(&self.session_start))
    }

    /// Begins the next session where this one ends.
    pub fn end_session(&mut self) {
        self.session_start = self.doc.transact().state_vector();
    }
}

impl Syncer<'_> {
    /// Opens a note's body: every live `note_update` child the device can
    /// read, applied in any order. A child that does not parse is skipped:
    /// it is some other device's corrupt write, not this note's end.
    pub fn open_note(&mut self, group_id: Uuid, note_id: Uuid) -> Result<NoteDoc, SyncError> {
        let mut doc = NoteDoc::new(self.rng);
        let children: Vec<Uuid> = self.store.group(group_id).map(|g| g.children_of(note_id)).unwrap_or_default();
        for child in children {
            if self.store.row(group_id, child).and_then(|r| r.object_type) != Some(ObjectType::NoteUpdate) {
                continue;
            }
            let Some(plain) = self.read(group_id, child)? else { continue };
            if let Ok(update) = Document::<NoteUpdate>::decode(&plain) {
                let _ = doc.apply(&update.body.update);
            }
        }
        // The session begins at what the children say, not at nothing.
        doc.end_session();
        Ok(doc)
    }

    /// Ends an editing session: its update becomes one `note_update` child
    /// of the note, pushed at the next sync. Nothing changed, nothing is
    /// written.
    pub fn save_note_session(&mut self, group_id: Uuid, note_id: Uuid, doc: &mut NoteDoc, author: Uuid, now_ms: i64) -> Result<Option<Uuid>, SyncError> {
        let Some(update) = doc.session_update() else { return Ok(None) };
        let seq_hint = self.store.group(group_id).map_or(0, |g| g.rows.iter().filter(|(_, r)| matches!(r, Row { deleted: false, parent_id: Some(p), .. } if *p == note_id)).count()) as i64 + 1;
        let id = crate::ids::random(self.rng);
        let child = Document::child_of(note_id, author, now_ms, NoteUpdate { update, seq_hint });
        self.create(group_id, id, ObjectType::NoteUpdate, child.encode())?;
        doc.end_session();
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_sessions_offline_combine_to_the_same_text_in_either_order() {
        let mut rng = CoreRng::from_seed([1; 32]);
        let mut base = NoteDoc::new(&mut rng);
        base.set_text("Shopping:\n");
        let seed = base.session_update().unwrap();

        let mut a = NoteDoc::new(&mut rng);
        a.apply(&seed).unwrap();
        a.set_text("Shopping:\nmilk\n");
        let ua = a.session_update().unwrap();

        let mut b = NoteDoc::new(&mut rng);
        b.apply(&seed).unwrap();
        b.set_text("Groceries:\n");
        let ub = b.session_update().unwrap();

        let mut ab = NoteDoc::new(&mut rng);
        for u in [&seed, &ua, &ub] {
            ab.apply(u).unwrap();
        }
        let mut ba = NoteDoc::new(&mut rng);
        for u in [&ub, &seed, &ua] {
            ba.apply(u).unwrap();
        }
        assert_eq!(ab.text(), ba.text());
        assert_eq!(ab.text(), "Groceries:\nmilk\n", "both edits kept: the title and the item");
    }

    #[test]
    fn set_text_changes_only_the_span_that_changed() {
        let mut rng = CoreRng::from_seed([2; 32]);
        let mut doc = NoteDoc::new(&mut rng);
        doc.set_text("héllo wörld");
        doc.end_session();
        assert!(doc.session_update().is_none(), "nothing since the session began");
        doc.set_text("héllo brave wörld");
        assert_eq!(doc.text(), "héllo brave wörld");
        doc.set_text("héllo wörld!");
        assert_eq!(doc.text(), "héllo wörld!");
        doc.set_text("");
        assert_eq!(doc.text(), "");
        doc.set_text("ü");
        assert_eq!(doc.text(), "ü");
        assert!(doc.session_update().is_some());
    }

    #[test]
    fn an_update_that_is_not_one_is_refused() {
        let mut rng = CoreRng::from_seed([3; 32]);
        let mut doc = NoteDoc::new(&mut rng);
        assert!(doc.apply(&[0xff, 0xff, 0xff]).is_err());
    }
}
