//! Undo/redo: command pattern with inverse operations (ARCHITECTURE.md — no
//! full-document snapshots).
//!
//! Scope note: this module is the **per-Object** history. Operations that
//! create or replace whole Objects (extrusion, booleans) are document-level
//! events; the Document command log wraps this one, it does not live
//! here. Designing the per-Object layer first keeps the invariants local:
//! every [`KernelOp`] has a well-defined inverse *on the same Object*.
//!
//! # How inverses work
//!
//! [`History::apply`] runs an op, then derives the inverse **from the op's
//! report**, because the inverse must reference handles that exist in the
//! *post-apply* state (generational ids are reallocated by mutations):
//!
//! - `PushPull { face, d }`        → `PushPull { report.face, -d }`
//! - `SplitFace { face, path }`    → `MergeFaces { report.new_edges[0] }`
//! - `MergeFaces { edge }`         → `SplitFace { report.merged_face, path }`
//!   (the path is reconstructed from the dissolved edges' geometry before
//!   they die)
//!
//! The guaranteed property (see `tests/op_specs.rs`): undo restores the
//! object to *topological and geometric* equality — same polygon soup up to
//! index renaming — not necessarily the same handles. Tools must re-query
//! after undo, never hoard handles across it.
//!
//! M0 status: types and bookkeeping are real; `apply`/`undo`/`redo` are
//! `todo!()` stubs pending the ops they drive.

use crate::ids::{EdgeId, FaceId};
use crate::math::Point3;
use crate::ops::{FaceMergeReport, FaceSplitReport, PushPullError, PushPullReport, StickyError};
use crate::topo::Object;

/// A replayable, invertible mutation of one Object. Plain data — serializable
/// later for crash recovery without redesign.
#[derive(Debug, Clone, PartialEq)]
pub enum KernelOp {
    /// `Object::push_pull(face, distance)`.
    PushPull {
        /// Face to move.
        face: FaceId,
        /// Signed distance along the face normal (meters).
        distance: f64,
    },
    /// `Object::split_face(face, path)`.
    SplitFace {
        /// Face to cut.
        face: FaceId,
        /// The cut path, boundary to boundary.
        path: Vec<Point3>,
    },
    /// `Object::merge_faces(edge)`.
    MergeFaces {
        /// Edge whose two coplanar faces merge.
        edge: EdgeId,
    },
}

/// What an applied op reported; returned by apply/undo/redo so tools can
/// update selections and highlights precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelOpReport {
    /// Result of a push/pull.
    PushPull(PushPullReport),
    /// Result of a face split.
    FaceSplit(FaceSplitReport),
    /// Result of a face merge.
    FaceMerge(FaceMergeReport),
}

/// An op that failed to apply. Wraps the op-specific error unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelOpError {
    /// From `push_pull`.
    PushPull(PushPullError),
    /// From `split_face` / `merge_faces`.
    Sticky(StickyError),
}

impl std::fmt::Display for KernelOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelOpError::PushPull(e) => write!(f, "{e}"),
            KernelOpError::Sticky(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KernelOpError {}

/// Undo/redo failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    /// Undo with an empty undo stack.
    NothingToUndo,
    /// Redo with an empty redo stack.
    NothingToRedo,
    /// Replaying an inverse failed. This is a kernel bug by definition — an
    /// inverse recorded by `apply` must always succeed — surfaced as a typed
    /// error so release builds fail loudly instead of corrupting the model.
    InverseFailed(KernelOpError),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::NothingToUndo => write!(f, "nothing to undo"),
            HistoryError::NothingToRedo => write!(f, "nothing to redo"),
            HistoryError::InverseFailed(e) => {
                write!(f, "inverse op failed (kernel bug): {e}")
            }
        }
    }
}

impl std::error::Error for HistoryError {}

/// One committed step: the op as applied and its derived inverse.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// The op that ran.
    pub op: KernelOp,
    /// The op that exactly reverses it, valid against the post-`op` state.
    pub inverse: KernelOp,
}

/// The per-Object undo/redo stack. All mutation of an Object that wants undo
/// must flow through [`History::apply`]; calling `Object` methods directly
/// bypasses recording (legitimate for scratch objects, a bug otherwise).
#[derive(Debug, Clone, Default)]
pub struct History {
    applied: Vec<HistoryEntry>,
    undone: Vec<HistoryEntry>,
}

impl History {
    /// An empty history.
    pub fn new() -> History {
        History::default()
    }

    /// True if [`History::undo`] has something to do.
    pub fn can_undo(&self) -> bool {
        !self.applied.is_empty()
    }

    /// True if [`History::redo`] has something to do.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Runs `op` on `object`, records its inverse, and clears the redo stack
    /// (the universal branch-discard convention).
    ///
    /// On `Err` nothing is recorded and `object` is untouched (the ops'
    /// strong guarantee carries through).
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn apply(
        &mut self,
        object: &mut Object,
        op: KernelOp,
    ) -> Result<KernelOpReport, KernelOpError> {
        todo!("M1: dispatch op, derive inverse from report, push entry, clear redo")
    }

    /// Reverses the most recent applied op and moves it to the redo stack.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn undo(&mut self, object: &mut Object) -> Result<KernelOpReport, HistoryError> {
        todo!("M1: pop entry, run inverse, re-derive forward entry for redo")
    }

    /// Re-applies the most recently undone op.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn redo(&mut self, object: &mut Object) -> Result<KernelOpReport, HistoryError> {
        todo!("M1: pop redo entry, apply, push back onto undo stack")
    }
}
