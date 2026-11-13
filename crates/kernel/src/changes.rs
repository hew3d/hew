//! Change sets and the session change ring (docs/agents/HEW_API.md §4.5,
//! docs/agents/ARCHITECTURE.md "Change ring").
//!
//! Every history change a [`Document`](crate::Document) makes (a pushed
//! entry, an undo, a redo, an aborted bracket, a document replacement)
//! leaves one [`ChangeRecord`] naming what it touched. The records live in
//! a bounded ring that is session-only: never serialized, never part of the
//! file format, and carried across a snapshot restore so an abort never
//! loses one. Hosts read it to emit `document_changed` events and to answer
//! `hew.query.changes_since`.

use crate::document::{AttrTarget, EntityRef, HistoryOrigin, LinkedWrap};

/// What moved the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// An entry was pushed (or a coalesced commit extended the top one).
    Commit,
    /// The record's entry was undone. Its lists are the entry's forward
    /// lists; a consumer swaps `added` and `removed`.
    Undo,
    /// The record's entry was redone.
    Redo,
    /// A transaction bracket or a compound was aborted: the document went
    /// back to the snapshot taken at its begin.
    Abort,
    /// A host replaced the document (open, new, revert).
    Reset,
}

/// An entity plus the stable id it had when recorded, so a removed entity
/// can still be named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRef {
    pub entity: EntityRef,
    pub sid: u64,
}

/// Which dictionary an [`AttrChange`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrScope {
    /// The document's or an entity's own dictionary (`AttrChange::target`).
    Entity,
    /// A solid face's dictionary, reported under its Object
    /// (`AttrChange::object`).
    Face,
    /// A solid edge's dictionary, reported under its Object.
    Edge,
}

/// One dictionary namespace a history change wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrChange {
    /// The owning Object (with the stable id it had when recorded) for a
    /// face or edge dictionary; `None` for [`AttrScope::Entity`].
    pub object: Option<ChangeRef>,
    /// The document or the entity, for [`AttrScope::Entity`]; `None`
    /// otherwise.
    pub target: Option<AttrTarget>,
    pub scope: AttrScope,
    pub ns: String,
}

impl AttrChange {
    /// An entity- (or document-) scoped change.
    pub fn entity(target: AttrTarget, ns: impl Into<String>) -> AttrChange {
        AttrChange {
            object: None,
            target: Some(target),
            scope: AttrScope::Entity,
            ns: ns.into(),
        }
    }
}

/// What one history change touched, by entity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub added: Vec<ChangeRef>,
    pub removed: Vec<ChangeRef>,
    pub modified: Vec<ChangeRef>,
    /// The dictionary namespaces that changed: an entity's or the
    /// document's, or a solid face's or edge's (named by its Object).
    pub attrs: Vec<AttrChange>,
    /// Document-level state (drawing axes, rescale) changed.
    pub document: bool,
    /// The lists are empty and everything may have changed.
    pub full: bool,
    /// Every Object or Group the change wrapped as a linked set, with the
    /// placement that took its spot ([`LinkedWrap`]).
    pub linked: Vec<LinkedWrap>,
    /// `(new, source)` for every entity the change copied: an independent
    /// copy's nodes, a linked copy's new placement and the placement it was
    /// copied from (the wrap's own placement for a just-wrapped node), and
    /// every id a make-unique (Unlink) minted.
    pub copied_from: Vec<(ChangeRef, ChangeRef)>,
}

impl ChangeSet {
    /// The set a consumer re-reads everything for.
    pub fn everything() -> ChangeSet {
        ChangeSet {
            full: true,
            ..ChangeSet::default()
        }
    }

    /// `added` and `removed` exchanged: what an undo of these changes did.
    /// An undo wraps and copies nothing, so `linked` and `copied_from` are
    /// empty (the placements the undo took away are in `removed`).
    pub fn swapped(&self) -> ChangeSet {
        ChangeSet {
            added: self.removed.clone(),
            removed: self.added.clone(),
            linked: Vec::new(),
            copied_from: Vec::new(),
            ..self.clone()
        }
    }

    /// Folds `other` into `self`: each list gains the refs it does not
    /// hold yet (first occurrence kept), `document` and `full` are sticky.
    /// A full set keeps empty lists.
    pub fn union_with(&mut self, other: &ChangeSet) {
        self.document |= other.document;
        self.full |= other.full;
        if self.full {
            self.added.clear();
            self.removed.clear();
            self.modified.clear();
            self.attrs.clear();
            self.linked.clear();
            self.copied_from.clear();
            return;
        }
        extend_unique(&mut self.added, &other.added);
        extend_unique(&mut self.removed, &other.removed);
        extend_unique(&mut self.modified, &other.modified);
        extend_unique(&mut self.attrs, &other.attrs);
        extend_unique(&mut self.linked, &other.linked);
        extend_unique(&mut self.copied_from, &other.copied_from);
    }
}

fn extend_unique<T: Clone + PartialEq>(into: &mut Vec<T>, from: &[T]) {
    for item in from {
        if !into.contains(item) {
            into.push(item.clone());
        }
    }
}

/// One history change, as the ring keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRecord {
    /// [`Document::history_generation`](crate::Document::history_generation)
    /// right after the change.
    pub generation: u64,
    /// [`Document::document_epoch`](crate::Document::document_epoch) right
    /// after the change. Strictly increasing along the ring.
    pub epoch: u64,
    pub kind: ChangeKind,
    pub origin: HistoryOrigin,
    /// The entry's label as the history lists it; `""` for an abort or a
    /// reset.
    pub label: String,
    /// Entering or leaving a group or component edit.
    pub bookkeeping: bool,
    pub changes: ChangeSet,
    /// The generation right before the change (the one a client that saw
    /// everything older holds, when a refused undo or redo moved it
    /// without a record).
    pub(crate) before: u64,
}

impl ChangeRecord {
    /// A commit with `full` at the given clock, origin user and no label:
    /// what a host emits in place of records it cannot (or will not) send
    /// one by one.
    pub fn collapsed(generation: u64, epoch: u64) -> ChangeRecord {
        ChangeRecord {
            generation,
            epoch,
            kind: ChangeKind::Commit,
            origin: HistoryOrigin::User,
            label: String::new(),
            bookkeeping: false,
            changes: ChangeSet::everything(),
            before: generation,
        }
    }
}

/// How many records the ring holds before it drops the oldest.
pub const CHANGE_RING_CAP: usize = 256;

/// While a bracket is open the ring may outgrow [`CHANGE_RING_CAP`]: its
/// children are about to collapse into one record, so they must not push
/// older history out. Past this many records it drops the oldest anyway
/// (a bracket that never closes cannot grow the ring without bound).
pub const CHANGE_RING_HARD_CAP: usize = 16 * CHANGE_RING_CAP;

/// The bounded record ring. Session-only.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChangeRing {
    /// Oldest first. Epochs strictly increase along it.
    pub(crate) records: Vec<ChangeRecord>,
    /// The epoch of the newest record the cap dropped, if any: a reader
    /// whose last epoch is older than this missed it.
    pub(crate) evicted_epoch: Option<u64>,
    /// The generation right after the newest record the cap dropped (0
    /// when none was): the oldest generation the ring still reaches back
    /// to.
    pub(crate) floor_generation: u64,
}

impl ChangeRing {
    /// Appends `record`. With `held` (a bracket is open, so the records
    /// since it began are about to collapse) the ring keeps everything up
    /// to [`CHANGE_RING_HARD_CAP`]; otherwise it trims back to
    /// [`CHANGE_RING_CAP`], dropping the oldest.
    pub(crate) fn push(&mut self, record: ChangeRecord, held: bool) {
        self.records.push(record);
        let cap = if held {
            CHANGE_RING_HARD_CAP
        } else {
            CHANGE_RING_CAP
        };
        if self.records.len() > cap {
            let excess = self.records.len() - cap;
            let last = &self.records[excess - 1];
            self.evicted_epoch = Some(last.epoch);
            self.floor_generation = last.generation;
            self.records.drain(..excess);
        }
    }

    /// Drops every record newer than `epoch`: the children of a bracket
    /// that is about to be recorded once, as a whole. Epochs never run
    /// backwards, so these are exactly the records made since the bracket
    /// began, aborts inside it included.
    pub(crate) fn drop_after_epoch(&mut self, epoch: u64) {
        self.records.retain(|r| r.epoch <= epoch);
    }

    pub(crate) fn since_epoch(&self, epoch: u64) -> Option<&[ChangeRecord]> {
        if self.evicted_epoch.is_some_and(|evicted| evicted > epoch) {
            return None;
        }
        let start = self.records.partition_point(|r| r.epoch <= epoch);
        Some(&self.records[start..])
    }

    /// Walks back from the newest record to the state a client holding
    /// `generation` saw. A record whose generation is `generation` is that
    /// state (it and everything older are excluded); a record whose
    /// `before` is `generation` is the first change the client missed
    /// (included, then the walk stops). Exact matches, not comparisons: an
    /// abort rewinds the generation, so a smaller one is not an older
    /// state. A reset is a hard boundary: it is
    /// included and nothing older is compared. Running off the end is
    /// covered only when `generation` is the ring's floor.
    pub(crate) fn changes_since_generation(&self, generation: u64) -> Option<ChangeSet> {
        // Where the client's view starts, walking back from the newest.
        let mut start = None;
        for (i, record) in self.records.iter().enumerate().rev() {
            if record.kind == ChangeKind::Reset {
                start = Some(i);
                break;
            }
            if record.generation == generation {
                start = Some(i + 1);
                break;
            }
            if record.before == generation {
                start = Some(i);
                break;
            }
        }
        let start = match start {
            Some(i) => i,
            None if generation == self.floor_generation => 0,
            None => return None,
        };
        // Folded oldest first, so the lists read in the order things
        // happened.
        let mut out = ChangeSet::default();
        for record in &self.records[start..] {
            if record.kind == ChangeKind::Undo {
                out.union_with(&record.changes.swapped());
            } else {
                out.union_with(&record.changes);
            }
        }
        Some(out)
    }
}
