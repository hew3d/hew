//! The undo-memory budget: how much the undo and redo stacks hold, and
//! trimming the oldest undo steps when a dense model pushes them past a
//! budget.
//!
//! Every geometric step snapshots what it changed (rule 9: undo restores
//! recorded state, never a recomputed inverse). An Object's element tables
//! are copy-on-write ([`crate::shared`]), so a snapshot shares every table
//! the step did not write and the live document shares the tables an undo
//! restores; what the stacks really hold is the tables nothing else holds.
//! On a scan of a million triangles that is still well over a hundred
//! megabytes a Move, and the browser build has a 4 GiB address space, so
//! [`Document::enforce_history_budget`] drops the oldest undo steps once
//! the stacks hold more than [`HistoryBudget::max_bytes`], always keeping
//! [`HistoryBudget::min_steps`]. An ordinary model never comes near the
//! default budget, so for it nothing here ever changes anything.
//!
//! Trimming never runs inside a [`Document::begin_transaction`] bracket and
//! never drops the opening entry of an editing session that is open, or
//! that a closing entry still on the stack can reopen. It leaves the redo
//! stack and [`Document::history_generation`] alone (no step is recorded,
//! undone or redone), shifts the saved mark down with the stack (or loses
//! it, when the saved state was in what was dropped) and drops the
//! per-object entries only the dropped steps could have reached.

use std::collections::{BTreeMap, BTreeSet};

use super::{DocAction, Document, SavedMark, SessionFrame};
use crate::ids::ObjectId;
use crate::sketch::Sketch;
use crate::topo::Object;

/// The default [`HistoryBudget::max_bytes`]: 1 GiB, a quarter of the
/// browser build's address space.
pub const DEFAULT_HISTORY_BUDGET_BYTES: usize = 1 << 30;

/// The default [`HistoryBudget::min_steps`].
pub const DEFAULT_HISTORY_MIN_STEPS: usize = 3;

/// How much undo history a document keeps
/// ([`Document::set_history_budget`]). Session-only, never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryBudget {
    /// Trim once the stacks hold more than this many bytes (an estimate,
    /// [`Document::history_memory`]).
    pub max_bytes: usize,
    /// Never trim below this many undo steps, however large they are. At
    /// least 1: the step just committed always stays undoable.
    pub min_steps: usize,
}

impl Default for HistoryBudget {
    fn default() -> HistoryBudget {
        HistoryBudget {
            max_bytes: DEFAULT_HISTORY_BUDGET_BYTES,
            min_steps: DEFAULT_HISTORY_MIN_STEPS,
        }
    }
}

/// One trim [`Document::enforce_history_budget`] made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryTrim {
    /// Undo steps dropped, oldest first.
    pub dropped: usize,
    /// Undo steps left.
    pub kept: usize,
}

/// What the undo and redo stacks hold ([`Document::history_memory`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryMemory {
    /// An estimate of the bytes only the stacks hold: snapshot tables
    /// nothing live shares, counted once each, plus each step's own record
    /// and per-object proof.
    pub bytes: usize,
    /// The budget in force.
    pub budget: HistoryBudget,
    /// [`Document::undo_depth`].
    pub undo_steps: usize,
    /// [`Document::redo_depth`].
    pub redo_steps: usize,
    /// Undo steps trimmed since the document was created or loaded.
    pub trimmed_total: u64,
}

/// Counts each snapshot table once: the first holder to be visited owns
/// it.
struct Tally {
    seen: BTreeSet<usize>,
}

impl Tally {
    fn object(&mut self, o: &Object) -> usize {
        o.table_footprints()
            .into_iter()
            .filter(|&(addr, _)| self.seen.insert(addr))
            .map(|(_, bytes)| bytes)
            .sum::<usize>()
            + std::mem::size_of::<Object>()
    }

    fn sketch(&mut self, s: &Sketch) -> usize {
        std::mem::size_of::<Sketch>() + s.heap_bytes_estimate()
    }

    /// Every Object and Sketch snapshot `action` carries, and the face
    /// fingerprints of the paint actions. Variants not listed hold ids,
    /// transforms and small records only, which the per-step record size
    /// covers.
    fn action(&mut self, action: &DocAction) -> usize {
        let mut bytes = std::mem::size_of::<DocAction>();
        match action {
            DocAction::Compound { actions, .. } | DocAction::PlaceTextCompound(actions) => {
                bytes += actions.iter().map(|a| self.action(a)).sum::<usize>();
            }
            DocAction::Repaired { before, after, .. } => {
                bytes += self.object(after) + self.object(before);
            }
            DocAction::Reshaped { before, .. } => bytes += self.object(before),
            DocAction::Transform {
                objects, sketches, ..
            }
            | DocAction::TransformSelection {
                objects, sketches, ..
            }
            | DocAction::Rescale {
                objects, sketches, ..
            }
            | DocAction::SessionOpened {
                pristine_objects: objects,
                pristine_sketches: sketches,
                ..
            } => {
                bytes += objects.iter().map(|(_, o)| self.object(o)).sum::<usize>();
                bytes += sketches.iter().map(|(_, s)| self.sketch(s)).sum::<usize>();
            }
            DocAction::TransformSketch { prior, .. }
            | DocAction::TransformSketchIsland { prior, .. } => bytes += self.sketch(prior),
            DocAction::RearrangedSketchVertex { before, after, .. }
            | DocAction::SketchGesture { before, after, .. } => {
                bytes += self.sketch(before) + self.sketch(after);
            }
            DocAction::PaintFace { face, .. } | DocAction::SetFaceUvFrame { face, .. } => {
                bytes += face.anchor.heap_bytes();
            }
            DocAction::ReplaceMaterial {
                faces, back_faces, ..
            }
            | DocAction::MaterialDeleted {
                faces, back_faces, ..
            } => {
                let row = std::mem::size_of::<(ObjectId, super::FaceRef)>();
                bytes += (faces.capacity() + back_faces.capacity()) * row;
                bytes += faces
                    .iter()
                    .chain(back_faces)
                    .map(|(_, f)| f.anchor.heap_bytes())
                    .sum::<usize>();
            }
            _ => {}
        }
        bytes
    }
}

/// Calls `f` with every object a per-object op entry of `action` belongs
/// to, once per entry (the walk [`DocAction::object_op_count`] counts).
fn for_each_object_op(action: &DocAction, f: &mut impl FnMut(ObjectId)) {
    match action {
        DocAction::ObjectOp { object } | DocAction::DefObjectOp { object, .. } => f(*object),
        DocAction::Compound { actions, .. } | DocAction::PlaceTextCompound(actions) => {
            for a in actions {
                for_each_object_op(a, f);
            }
        }
        _ => {}
    }
}

/// Calls `f` with the stored `undo_len_at_open` of every session-closing
/// entry inside `action`.
fn for_each_closed_session(action: &mut DocAction, f: &mut impl FnMut(&mut usize)) {
    match action {
        DocAction::SessionClosed {
            undo_len_at_open, ..
        }
        | DocAction::GroupSessionClosed {
            undo_len_at_open, ..
        } => f(undo_len_at_open),
        DocAction::Compound { actions, .. } | DocAction::PlaceTextCompound(actions) => {
            for a in actions {
                for_each_closed_session(a, f);
            }
        }
        _ => {}
    }
}

fn frame_open_len(frame: &SessionFrame) -> usize {
    match frame {
        SessionFrame::Component(s) => s.undo_len_at_open,
        SessionFrame::Group(s) => s.undo_len_at_open,
    }
}

impl Document {
    /// The undo-memory budget in force ([`Document::set_history_budget`]).
    pub fn history_budget(&self) -> HistoryBudget {
        self.history_budget
    }

    /// Sets the undo-memory budget. Takes effect at the next
    /// [`Document::enforce_history_budget`]; never trims by itself.
    /// `min_steps` below 1 is raised to 1.
    pub fn set_history_budget(&mut self, budget: HistoryBudget) {
        self.history_budget = HistoryBudget {
            max_bytes: budget.max_bytes,
            min_steps: budget.min_steps.max(1),
        };
    }

    /// What the undo and redo stacks hold. Pure; O(objects + steps), with
    /// no per-element walk of any mesh.
    pub fn history_memory(&self) -> HistoryMemory {
        let (per_step, base) = self.attribute_history(true);
        HistoryMemory {
            bytes: base + per_step.iter().sum::<usize>(),
            budget: self.history_budget,
            undo_steps: self.undo.actions.len(),
            redo_steps: self.redo.actions.len(),
            trimmed_total: self.history_trimmed,
        }
    }

    /// Drops the oldest undo steps while the stacks hold more than the
    /// budget ([`HistoryBudget::max_bytes`]), keeping at least
    /// [`HistoryBudget::min_steps`] and every step an open editing session
    /// (or a closing entry still on the stack) needs. Returns what it
    /// dropped, `None` when nothing was over budget or nothing could go.
    ///
    /// The host calls this between operations, never in the middle of one;
    /// inside a [`Document::begin_transaction`] bracket, an open
    /// [`Document::begin_compound`] or a drain-and-bundle operation it does
    /// nothing (each later collapses the entries recorded since it began by
    /// stack position).
    /// What stays undoable undoes exactly as before; the redo stack and
    /// [`Document::history_generation`] are untouched. A saved state that
    /// was in the dropped steps can no longer be reached by undo, so the
    /// document stays dirty until the next [`Document::mark_saved`].
    pub fn enforce_history_budget(&mut self) -> Option<HistoryTrim> {
        if self.open_brackets.get() != 0 || !self.compounds.is_empty() || self.ring_holds.get() != 0
        {
            return None;
        }
        let budget = self.history_budget;
        let max_drop = self
            .undo
            .actions
            .len()
            .saturating_sub(budget.min_steps.max(1));
        if max_drop == 0 {
            return None;
        }
        // A cheap upper bound first (tables the live document shares are
        // counted too): an ordinary model stops here.
        let (per_step, base) = self.attribute_history(false);
        if base + per_step.iter().sum::<usize>() <= budget.max_bytes {
            return None;
        }
        let (per_step, base) = self.attribute_history(true);
        let mut total = base + per_step.iter().sum::<usize>();
        let mut k = 0;
        while k < max_drop && total > budget.max_bytes {
            total -= per_step[k];
            k += 1;
        }
        let k = self.clamp_trim_to_sessions(k);
        if k == 0 {
            return None;
        }
        self.drop_oldest_undo(k);
        Some(HistoryTrim {
            dropped: k,
            kept: self.undo.actions.len(),
        })
    }

    /// Bytes per undo step (oldest first) and the bytes of everything else
    /// the stacks hold (the redo stack). A table is attributed to the
    /// newest holder — the redo stack first, then undo from the top down —
    /// so dropping the oldest steps frees exactly what is attributed to
    /// them. With `exact`, tables the live document holds are attributed
    /// to it and not counted at all; without, the result is an upper
    /// bound that skips the walk over every live object.
    fn attribute_history(&self, exact: bool) -> (Vec<usize>, usize) {
        let mut tally = Tally {
            seen: BTreeSet::new(),
        };
        if exact {
            for rec in self.objects.values() {
                for (addr, _) in rec.object.table_footprints() {
                    tally.seen.insert(addr);
                }
            }
        }
        let base: usize = self
            .redo
            .actions
            .iter()
            .rev()
            .map(|a| tally.action(a))
            .sum();
        // Per-object undo entries pair with the document's per-object op
        // steps from the top of both stacks down.
        let mut proofs: BTreeMap<ObjectId, (Vec<usize>, usize)> = BTreeMap::new();
        let mut per_step = vec![0; self.undo.actions.len()];
        for (i, action) in self.undo.actions.iter().enumerate().rev() {
            let mut bytes = tally.action(action);
            for_each_object_op(action, &mut |object| {
                let Some(rec) = self.objects.get(object) else {
                    return;
                };
                let (sizes, next) = proofs.entry(object).or_insert_with(|| {
                    let sizes = rec.history.applied_bytes();
                    let len = sizes.len();
                    (sizes, len)
                });
                if *next > 0 {
                    *next -= 1;
                    bytes += sizes[*next];
                }
            });
            per_step[i] = bytes;
        }
        (per_step, base)
    }

    /// The largest trim no larger than `k` that keeps every session's
    /// opening entry a session could still need: an open session's, and
    /// that of a closed one whose closing entry stays on a stack (undoing
    /// it reopens the session at its recorded stack length).
    fn clamp_trim_to_sessions(&mut self, mut k: usize) -> usize {
        // Dropping `k` steps removes indices `0..k`. An opening entry sits
        // at index `u - 1`, so it survives exactly when `k < u`.
        let mut limits: Vec<(usize, usize)> = Vec::new(); // (u, last index that pins it)
        for frame in &self.sessions {
            limits.push((frame_open_len(frame), usize::MAX));
        }
        for (i, action) in self.undo.actions.iter_mut().enumerate() {
            for_each_closed_session(action, &mut |u| limits.push((*u, i)));
        }
        for action in self.redo.actions.iter_mut() {
            for_each_closed_session(action, &mut |u| limits.push((*u, usize::MAX)));
        }
        loop {
            let mut changed = false;
            for &(u, pinned_through) in &limits {
                // The closing entry at `pinned_through` survives while
                // `k <= pinned_through`; then its opening entry must too.
                if k >= u && k <= pinned_through {
                    k = u.saturating_sub(1);
                    changed = true;
                }
            }
            if !changed {
                return k;
            }
        }
    }

    /// Drops the `k` oldest undo steps and shifts everything that names a
    /// stack position.
    fn drop_oldest_undo(&mut self, k: usize) {
        let dropped: Vec<DocAction> = self.undo.actions.drain(..k).collect();
        let content = dropped.iter().filter(|a| !a.is_bookkeeping()).count();
        self.saved = SavedMark(self.saved.0.and_then(|d| d.checked_sub(content)));
        for frame in &mut self.sessions {
            match frame {
                SessionFrame::Component(s) => s.undo_len_at_open -= k,
                SessionFrame::Group(s) => s.undo_len_at_open -= k,
            }
        }
        let shift = |u: &mut usize| *u = u.saturating_sub(k);
        let mut shift = shift;
        for action in self
            .undo
            .actions
            .iter_mut()
            .chain(self.redo.actions.iter_mut())
        {
            for_each_closed_session(action, &mut shift);
        }
        // An object's undo entries pair with its per-object op steps from
        // the top down, so the entries below the steps still on the stack
        // are the ones only the dropped steps could reach.
        let mut touched: BTreeSet<ObjectId> = BTreeSet::new();
        for action in &dropped {
            for_each_object_op(action, &mut |o| {
                touched.insert(o);
            });
        }
        for object in touched {
            let remaining: usize = self
                .undo
                .actions
                .iter()
                .map(|a| a.object_op_count(object))
                .sum();
            if let Some(rec) = self.objects.get_mut(object) {
                let len = rec.history.applied_len();
                if len > remaining {
                    rec.history.drop_oldest(len - remaining);
                }
            }
        }
        self.history_trimmed += k as u64;
    }
}
