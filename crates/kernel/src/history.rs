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
//! # Replay is guard-exempt, with proof (DEVELOPMENT.md rule 9)
//!
//! Undo and redo dispatch in the ops' replay mode: the best-effort
//! obstruction heuristics (`push_pull`'s neighbor-vertex/extent guards,
//! `extrude_sub_face`'s centroid ray) are skipped, because a LIFO replay
//! re-enters a state the kernel already accepted and a heuristic reading
//! the surrounding geometry could otherwise refuse it. In exchange, every
//! entry carries a [`StateProof`] — a geometric fingerprint of the state
//! the replay must reproduce — and the replayed op runs on a clone that is
//! committed only if it matches the proof. A mismatch is a kernel bug,
//! surfaced as [`HistoryError::InverseDiverged`] with the object untouched.
//! See ARCHITECTURE.md §5.7 for the full rationale.

use crate::ids::{EdgeId, FaceId, HalfEdgeId};
use crate::math::{Point3, Vec3};
use crate::ops::{
    CollapseSubFaceReport, FaceMergeInnerReport, FaceMergeReport, FaceSplitInnerReport,
    FaceSplitReport, PushPullError, PushPullReport, StickyError,
};
use crate::tol;
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
    /// `Object::split_face_inner(face, loop_path)` — imprint a closed loop.
    SplitFaceInner {
        /// Face to imprint into.
        face: FaceId,
        /// The closed loop, strictly inside the face.
        loop_path: Vec<Point3>,
    },
    /// `Object::merge_inner_face(sub_face)` — dissolve an imprinted sub-face.
    MergeInnerFace {
        /// The sub-face to dissolve back into its parent.
        sub_face: FaceId,
    },
    /// `Object::extrude_sub_face(sub_face, distance)` — boss/recess a sub-face.
    ExtrudeSubFace {
        /// The flat sub-face to raise.
        sub_face: FaceId,
        /// Signed distance along the face normal.
        distance: f64,
    },
    /// `Object::collapse_sub_face(sub_face)` — flatten a raised sub-face.
    CollapseSubFace {
        /// The raised sub-face to flatten.
        sub_face: FaceId,
    },
}

/// What an applied op reported; returned by apply/undo/redo so tools can
/// update selections and highlights precisely.
/// (`FaceMergeInner` carries f64 positions, so this is `PartialEq` but not `Eq`.)
#[derive(Debug, Clone, PartialEq)]
pub enum KernelOpReport {
    /// Result of a push/pull.
    PushPull(PushPullReport),
    /// Result of a face split.
    FaceSplit(FaceSplitReport),
    /// Result of a face merge.
    FaceMerge(FaceMergeReport),
    /// Result of an interior-loop imprint.
    FaceSplitInner(FaceSplitInnerReport),
    /// Result of dissolving an imprinted sub-face.
    FaceMergeInner(FaceMergeInnerReport),
    /// Result of bossing/recessing a sub-face (reuses the push/pull report).
    ExtrudeSubFace(PushPullReport),
    /// Result of flattening a raised sub-face.
    CollapseSubFace(CollapseSubFaceReport),
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
    /// A replayed inverse/redo ran, but its result did not reproduce the
    /// recorded state its [`StateProof`] fingerprints (rule 9). A kernel bug
    /// by definition; the object is left untouched (the replay ran on a
    /// clone that is discarded).
    InverseDiverged,
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::NothingToUndo => write!(f, "nothing to undo"),
            HistoryError::NothingToRedo => write!(f, "nothing to redo"),
            HistoryError::InverseFailed(e) => {
                write!(f, "inverse op failed (kernel bug): {e}")
            }
            HistoryError::InverseDiverged => {
                write!(
                    f,
                    "replayed op diverged from the recorded state (kernel bug)"
                )
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
///
/// # Stack invariants
///
/// `applied` holds full [`HistoryEntry`] records (op + derived inverse).
/// `undone` holds the operations to re-apply on redo; each inverse is
/// re-derived at redo-time from the fresh report, so stale handles never
/// appear in the wrong slot.
///
/// # Handle-staleness safety
///
/// Ops on both stacks carry generational ids, and any mutation that runs
/// between an op's derivation and its dispatch may reallocate those ids even
/// though the geometry round-trips exactly (undoing a split merges the cut
/// faces away; redoing it mints *new* face ids). So each stack entry also
/// carries a geometric [`Anchor`] for its target — captured when the op is
/// derived, resolved against the live object when the op finally dispatches.
/// This is the stack-level analogue of the module rule above: tools re-query
/// after undo, and so does the history itself.
#[derive(Debug, Clone, Default)]
pub struct History {
    applied: Vec<AppliedEntry>,
    /// Ops to re-apply on redo. Inverses are re-derived at redo-time.
    undone: Vec<RedoEntry>,
}

/// An undo-stack record: the committed step, its target's [`Anchor`], and
/// the rule-9 proof its inverse must discharge.
#[derive(Debug, Clone)]
struct AppliedEntry {
    entry: HistoryEntry,
    anchor: Anchor,
    /// Fingerprint of the state *before* `entry.op` ran — the state
    /// `entry.inverse` must reproduce for its replay to commit.
    prior: StateProof,
}

/// A redo-stack record: the re-anchored forward op, its target's [`Anchor`],
/// and the rule-9 proof its replay must discharge.
#[derive(Debug, Clone)]
struct RedoEntry {
    op: KernelOp,
    anchor: Anchor,
    /// Fingerprint of the state `op` originally produced — the state its
    /// replay must reproduce for the redo to commit.
    target: StateProof,
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

    /// The op the next [`History::undo`] would dispatch (the recorded
    /// inverse), if any. Its handle may be re-anchored before dispatch; the
    /// op KIND and parameters are what callers may rely on (undo menu
    /// labels, tooling).
    pub fn peek_undo(&self) -> Option<&KernelOp> {
        self.applied.last().map(|rec| &rec.entry.inverse)
    }

    /// The op the next [`History::redo`] would dispatch, if any. Same handle
    /// caveat as [`History::peek_undo`].
    pub fn peek_redo(&self) -> Option<&KernelOp> {
        self.undone.last().map(|rec| &rec.op)
    }

    /// Runs `op` on `object`, records its inverse, and clears the redo stack
    /// (the universal branch-discard convention).
    ///
    /// On `Err` nothing is recorded and `object` is untouched (the ops'
    /// strong guarantee carries through).
    pub fn apply(
        &mut self,
        object: &mut Object,
        op: KernelOp,
    ) -> Result<KernelOpReport, KernelOpError> {
        // For MergeFaces, capture the cut-path geometry BEFORE the op runs,
        // because the edge chain's vertices may be healed away during the merge.
        let pre_merge_path: Option<Vec<Point3>> = if let KernelOp::MergeFaces { edge } = &op {
            Some(reconstruct_merge_path(object, *edge))
        } else {
            None
        };

        // Capture the rule-9 proof for the eventual undo: the state the
        // recorded inverse must restore is the one this op is about to leave.
        let prior = StateProof::of(object);

        // Dispatch the op. On error the object is untouched (strong guarantee).
        let report = dispatch(object, &op)?;

        // Derive the inverse from the report (post-apply handles) and anchor
        // its target geometrically for dispatch after intervening mutations.
        let inverse = derive_inverse(&op, &report, pre_merge_path);
        let anchor = anchor_of(object, &inverse);

        // Push the entry and clear the redo stack (branch-discard).
        self.applied.push(AppliedEntry {
            entry: HistoryEntry { op, inverse },
            anchor,
            prior,
        });
        self.undone.clear();

        Ok(report)
    }

    /// Reverses the most recent applied op and moves it to the redo stack.
    ///
    /// Dispatches in replay mode (rule 9): heuristic guards are exempt, and
    /// the inverse runs on a clone that commits only if it reproduces the
    /// entry's recorded pre-op state ([`StateProof`]).
    ///
    /// The redo entry pushed to `undone` is the op derived from the inverse's
    /// report — i.e., the freshly re-anchored forward op. Its inverse is
    /// re-derived at redo-time, never stale.
    pub fn undo(&mut self, object: &mut Object) -> Result<KernelOpReport, HistoryError> {
        let rec = self.applied.last().ok_or(HistoryError::NothingToUndo)?;

        // Re-resolve the inverse's target from its geometric anchor: redo
        // cycles since `apply` may have reallocated the recorded handle.
        let inverse = re_anchor(object, &rec.entry.inverse, &rec.anchor)
            .map_err(HistoryError::InverseFailed)?;

        // For MergeFaces in the inverse, capture path before dispatch.
        let pre_merge_path: Option<Vec<Point3>> = if let KernelOp::MergeFaces { edge } = &inverse {
            Some(reconstruct_merge_path(object, *edge))
        } else {
            None
        };

        // Run the inverse on a clone, guard-exempt. A dispatch error is a
        // kernel bug surfaced as InverseFailed; a result that fails the
        // recorded-state proof is InverseDiverged. Either way the object is
        // untouched and the entry stays on the stack — commit only after
        // both succeed.
        let mut candidate = object.clone();
        let report =
            dispatch_replay(&mut candidate, &inverse).map_err(HistoryError::InverseFailed)?;
        if !rec.prior.verify_and_align(&mut candidate) {
            return Err(HistoryError::InverseDiverged);
        }

        // Capture the rule-9 proof for the eventual redo — the state this
        // undo is about to leave (the op's own result) — then commit.
        let target = StateProof::of(object);
        *object = candidate;
        self.applied.pop();

        // The op to push onto the redo stack is the RE-ANCHORED forward op:
        // derived from the inverse's report (post-undo handles).  This is the
        // fresh op that redo will replay; its own inverse will be derived at
        // redo-time from the redo's report.
        let redo_op = derive_inverse(&inverse, &report, pre_merge_path);
        let redo_anchor = anchor_of(object, &redo_op);
        self.undone.push(RedoEntry {
            op: redo_op,
            anchor: redo_anchor,
            target,
        });

        Ok(report)
    }

    /// Re-applies the most recently undone op.
    ///
    /// Dispatches in replay mode (rule 9): heuristic guards are exempt, and
    /// the op runs on a clone that commits only if it reproduces the state
    /// it originally produced ([`StateProof`]).
    ///
    /// Pops the redo op, runs it, derives a fresh inverse from the report,
    /// and pushes a complete [`HistoryEntry`] onto the undo stack.
    pub fn redo(&mut self, object: &mut Object) -> Result<KernelOpReport, HistoryError> {
        let rec = self.undone.last().ok_or(HistoryError::NothingToRedo)?;

        // Re-resolve the redo op's target from its geometric anchor: undos
        // that ran after this entry was pushed (unwinding earlier ops) may
        // have destroyed the recorded handle, and the redos that rebuilt the
        // geometry minted fresh ids.
        let redo_op =
            re_anchor(object, &rec.op, &rec.anchor).map_err(HistoryError::InverseFailed)?;

        // For MergeFaces, capture path before dispatch.
        let pre_merge_path: Option<Vec<Point3>> = if let KernelOp::MergeFaces { edge } = &redo_op {
            Some(reconstruct_merge_path(object, *edge))
        } else {
            None
        };

        // Run the redo on a clone, guard-exempt, and hold it to the recorded
        // proof exactly as `undo` does; pop only after both succeed so a
        // failed redo doesn't discard the entry.
        let mut candidate = object.clone();
        let report =
            dispatch_replay(&mut candidate, &redo_op).map_err(HistoryError::InverseFailed)?;
        if !rec.target.verify_and_align(&mut candidate) {
            return Err(HistoryError::InverseDiverged);
        }

        // The state this redo leaves is what its recorded inverse must
        // restore — capture it, then commit.
        let prior = StateProof::of(object);
        *object = candidate;
        self.undone.pop();

        // Derive a fresh inverse from this redo's report and push onto applied.
        let new_inverse = derive_inverse(&redo_op, &report, pre_merge_path);
        let inverse_anchor = anchor_of(object, &new_inverse);
        self.applied.push(AppliedEntry {
            entry: HistoryEntry {
                op: redo_op,
                inverse: new_inverse,
            },
            anchor: inverse_anchor,
            prior,
        });

        Ok(report)
    }
}

// ================================================================= private helpers

/// Geometric fingerprint of an accepted state (DEVELOPMENT.md rule 9 /
/// ARCHITECTURE.md §5.7): every face as its outer ring plus its hole rings,
/// each a position cycle. Captured when a history entry is recorded; the
/// entry's replay must reproduce it before its result is committed.
///
/// Hole rings are fingerprinted PER OWNING FACE, not as free-floating
/// geometry: hole OWNERSHIP is exactly the kind of bookkeeping a buggy
/// replay can scramble while leaving every outer ring — and the structural
/// validator — happy (a hole handed to the wrong coplanar face keeps all
/// loop pointers self-consistent). Covering ownership here is what makes
/// that class InverseDiverged-visible.
///
/// Comparison is the tolerance-aware equivalence the round-trip property
/// tests use — a multiset of faces, rings matched as cyclic rotations within
/// [`tol::POINT_MERGE`] — because floating-point round-trips are not bitwise
/// (`(p + d) − d ≠ p`; intervening baked transforms round-trip with noise
/// too). Matching is greedy pairwise for the same reason [`same_position_set`]
/// is: sort order flips under that noise.
///
/// Memory: every history entry retains one proof — O(total boundary
/// vertices, outer and hole rings alike) per recorded op. Undo correctness
/// buys that; revisit only with a benchmark in hand (DEVELOPMENT.md's
/// strong-guarantee costing rule).
#[derive(Debug, Clone)]
struct StateProof {
    faces: Vec<FaceProof>,
}

/// One face's fingerprint: outer ring and hole rings as position cycles,
/// plus the face's stored plane.
#[derive(Debug, Clone)]
struct FaceProof {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    plane: crate::math::Plane,
}

/// The cyclic shift under which `a[i]` matches `b[(i + shift) % n]` within
/// [`tol::POINT_MERGE`], if any (winding preserved).
fn ring_match_shift(a: &[Point3], b: &[Point3]) -> Option<usize> {
    if a.len() != b.len() {
        return None;
    }
    (0..a.len()).find(|&shift| {
        a.iter()
            .enumerate()
            .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], tol::POINT_MERGE))
    })
}

impl StateProof {
    /// Fingerprints `object`'s current geometry.
    fn of(object: &Object) -> StateProof {
        StateProof {
            faces: object
                .faces()
                .values()
                .map(|face| FaceProof {
                    outer: object.loop_positions(face.outer_loop).collect(),
                    holes: face
                        .inner_loops
                        .iter()
                        .map(|&il| object.loop_positions(il).collect())
                        .collect(),
                    plane: face.plane,
                })
                .collect(),
        }
    }

    /// Verifies that `candidate`'s faces equal this fingerprint up to face
    /// order, ring rotation, hole order, and floating-point round-trip
    /// noise — and, on success, ALIGNS the candidate to the recorded
    /// coordinates: every matched vertex takes the recorded position and
    /// every matched face takes the recorded plane.
    ///
    /// The alignment is what keeps replay exact rather than merely close:
    /// a replayed op recomputes geometry (`fl(fl(x + d) - d) != x`), and
    /// committing the recomputed coordinates would let noise ACCUMULATE
    /// across undo/redo cycles — refit normals amplify coordinate noise by
    /// sweep-distance/face-extent per cycle, eventually flipping a marginal
    /// tolerance decision inside a later replay and refusing an op that the
    /// forward pass accepted. Restoring the recorded bits is not geometry
    /// repair (rule 4 is about masking INVALID results); it is the
    /// definition of undo/redo: the committed state IS the accepted state
    /// the entry recorded, so every subsequent replay re-derives exactly
    /// the computation its forward op ran.
    ///
    /// Returns false — candidate possibly partially aligned, caller must
    /// discard it — if the fingerprint does not match or the match implies
    /// conflicting vertex positions.
    fn verify_and_align(&self, candidate: &mut Object) -> bool {
        struct LiveFace {
            id: crate::ids::FaceId,
            outer_verts: Vec<crate::ids::VertexId>,
            outer: Vec<Point3>,
            holes: Vec<(Vec<crate::ids::VertexId>, Vec<Point3>)>,
        }
        let live: Vec<LiveFace> = candidate
            .faces()
            .iter()
            .map(|(id, face)| {
                let outer_verts: Vec<crate::ids::VertexId> = candidate
                    .loop_half_edges(face.outer_loop)
                    .map(|h| candidate.half_edges()[h].origin)
                    .collect();
                let outer = outer_verts
                    .iter()
                    .map(|&v| candidate.vertices()[v].position)
                    .collect();
                let holes = face
                    .inner_loops
                    .iter()
                    .map(|&il| {
                        let vs: Vec<crate::ids::VertexId> = candidate
                            .loop_half_edges(il)
                            .map(|h| candidate.half_edges()[h].origin)
                            .collect();
                        let ps = vs
                            .iter()
                            .map(|&v| candidate.vertices()[v].position)
                            .collect();
                        (vs, ps)
                    })
                    .collect();
                LiveFace {
                    id,
                    outer_verts,
                    outer,
                    holes,
                }
            })
            .collect();
        if live.len() != self.faces.len() {
            return false;
        }

        // Greedy face matching, extracting the ring alignments as we go.
        let mut taken = vec![false; self.faces.len()];
        let mut vertex_target: slotmap::SecondaryMap<crate::ids::VertexId, Point3> =
            slotmap::SecondaryMap::new();
        let assign = |verts: &[crate::ids::VertexId],
                      rec: &[Point3],
                      shift: usize,
                      vertex_target: &mut slotmap::SecondaryMap<crate::ids::VertexId, Point3>|
         -> bool {
            for (i, &v) in verts.iter().enumerate() {
                let target = rec[(i + shift) % rec.len()];
                match vertex_target.get(v) {
                    Some(prev) if !prev.approx_eq(target, tol::POINT_MERGE) => {
                        return false; // conflicting assignments — bail
                    }
                    Some(_) => {}
                    None => {
                        vertex_target.insert(v, target);
                    }
                }
            }
            true
        };
        let mut face_plane: Vec<(crate::ids::FaceId, crate::math::Plane)> =
            Vec::with_capacity(live.len());
        for lf in &live {
            let mut matched = false;
            for (k, rec) in self.faces.iter().enumerate() {
                if taken[k] || lf.holes.len() != rec.holes.len() {
                    continue;
                }
                let Some(outer_shift) = ring_match_shift(&lf.outer, &rec.outer) else {
                    continue;
                };
                // Match holes as a multiset, remembering each shift.
                let mut hole_taken = vec![false; rec.holes.len()];
                let mut hole_assign: Vec<(usize, usize, usize)> = Vec::new();
                let mut holes_ok = true;
                for (hi, (_, hp)) in lf.holes.iter().enumerate() {
                    let mut found = None;
                    for (hk, rh) in rec.holes.iter().enumerate() {
                        if hole_taken[hk] {
                            continue;
                        }
                        if let Some(shift) = ring_match_shift(hp, rh) {
                            found = Some((hk, shift));
                            break;
                        }
                    }
                    match found {
                        Some((hk, shift)) => {
                            hole_taken[hk] = true;
                            hole_assign.push((hi, hk, shift));
                        }
                        None => {
                            holes_ok = false;
                            break;
                        }
                    }
                }
                if !holes_ok {
                    continue;
                }
                if !assign(&lf.outer_verts, &rec.outer, outer_shift, &mut vertex_target) {
                    return false;
                }
                for (hi, hk, shift) in hole_assign {
                    if !assign(&lf.holes[hi].0, &rec.holes[hk], shift, &mut vertex_target) {
                        return false;
                    }
                }
                face_plane.push((lf.id, rec.plane));
                taken[k] = true;
                matched = true;
                break;
            }
            if !matched {
                return false;
            }
        }

        // Apply the alignment.
        for (v, target) in &vertex_target {
            candidate.vertices[v].position = *target;
        }
        for (f, plane) in face_plane {
            candidate.faces[f].plane = plane;
        }
        // The aligned state is the recorded accepted state; hold it to the
        // full validator anyway (typed refusal beats trusting the alignment).
        candidate.validate().is_ok()
    }
}

/// Geometric fingerprint of a stack entry's target. Captured when the entry's
/// op is derived (its handle is fresh then) and resolved against the live
/// object when the op finally dispatches, because intervening undo/redo
/// mutations reallocate generational ids even though geometry round-trips
/// exactly (see the "Handle-staleness safety" section on [`History`]).
///
/// Positions round-trip to within floating-point noise, far below
/// [`tol::POINT_MERGE`], so fingerprints match positionally at that tolerance.
#[derive(Debug, Clone)]
enum Anchor {
    /// A face: its outer-loop vertex positions (order-independent) and its
    /// plane normal. The normal disambiguates coincident opposite-side faces;
    /// the full vertex set disambiguates coplanar faces sharing a centroid
    /// (e.g. a parent face and a concentric imprinted sub-face).
    Face { verts: Vec<Point3>, normal: Vec3 },
    /// An edge: its two endpoint positions (order-independent).
    Edge { a: Point3, b: Point3 },
}

/// Captures the [`Anchor`] for `op`'s target handle, which must be live in
/// `object` (ops on the stacks are always derived from a fresh report).
fn anchor_of(object: &Object, op: &KernelOp) -> Anchor {
    match op {
        KernelOp::PushPull { face, .. }
        | KernelOp::SplitFace { face, .. }
        | KernelOp::SplitFaceInner { face, .. }
        | KernelOp::MergeInnerFace { sub_face: face }
        | KernelOp::ExtrudeSubFace { sub_face: face, .. }
        | KernelOp::CollapseSubFace { sub_face: face } => {
            let f = &object.faces()[*face];
            Anchor::Face {
                verts: object.loop_positions(f.outer_loop).collect(),
                normal: f.plane.normal(),
            }
        }
        KernelOp::MergeFaces { edge } => {
            let (a, b) = object
                .edge_endpoints(*edge)
                .expect("anchored op holds a fresh handle from its report");
            Anchor::Edge { a, b }
        }
    }
}

/// True if `a` and `b` contain the same positions up to reordering, each
/// matched within [`tol::POINT_MERGE`]. Matching is greedy pairwise rather
/// than sort-and-zip: round-trip noise near a coordinate boundary (a 0.0 that
/// comes back as -4e-15) would flip lexicographic sort order and mispair
/// otherwise-identical sets. Distinct mesh vertices are separated by far more
/// than the tolerance, so greedy matching is unambiguous.
fn same_position_set(a: &[Point3], b: &[Point3]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut unmatched: Vec<Point3> = b.to_vec();
    for p in a {
        let Some(i) = unmatched
            .iter()
            .position(|q| (*p - *q).length() <= tol::POINT_MERGE)
        else {
            return false;
        };
        unmatched.swap_remove(i);
    }
    true
}

/// Finds the live face matching a [`Anchor::Face`] fingerprint.
fn resolve_face(object: &Object, verts: &[Point3], normal: Vec3) -> Option<FaceId> {
    object.faces().iter().find_map(|(fid, f)| {
        if f.plane.normal().dot(normal) < 1.0 - tol::POINT_MERGE {
            return None;
        }
        let fverts: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
        same_position_set(&fverts, verts).then_some(fid)
    })
}

/// Finds the live edge matching a [`Anchor::Edge`] fingerprint.
fn resolve_edge(object: &Object, a: Point3, b: Point3) -> Option<EdgeId> {
    object.edges().keys().find(|&eid| {
        let (p, q) = object.edge_endpoints(eid).expect("iterating live edge ids");
        ((p - a).length() <= tol::POINT_MERGE && (q - b).length() <= tol::POINT_MERGE)
            || ((p - b).length() <= tol::POINT_MERGE && (q - a).length() <= tol::POINT_MERGE)
    })
}

/// Rebuilds `op` with its target handle re-resolved from `anchor` against the
/// live object. Failure to resolve means the geometry the anchor fingerprints
/// no longer exists — a kernel bug by the history contract — reported as the
/// op's own unknown-target error so callers see the familiar typed failure.
fn re_anchor(object: &Object, op: &KernelOp, anchor: &Anchor) -> Result<KernelOp, KernelOpError> {
    let face = |unknown: KernelOpError| -> Result<FaceId, KernelOpError> {
        match anchor {
            Anchor::Face { verts, normal } => resolve_face(object, verts, *normal).ok_or(unknown),
            Anchor::Edge { .. } => Err(unknown),
        }
    };
    let unknown_face_sticky = KernelOpError::Sticky(StickyError::UnknownFace);
    let unknown_face_pp = KernelOpError::PushPull(PushPullError::UnknownFace);
    match op {
        KernelOp::PushPull { distance, .. } => Ok(KernelOp::PushPull {
            face: face(unknown_face_pp)?,
            distance: *distance,
        }),
        KernelOp::SplitFace { path, .. } => Ok(KernelOp::SplitFace {
            face: face(unknown_face_sticky)?,
            path: path.clone(),
        }),
        KernelOp::SplitFaceInner { loop_path, .. } => Ok(KernelOp::SplitFaceInner {
            face: face(unknown_face_sticky)?,
            loop_path: loop_path.clone(),
        }),
        KernelOp::MergeInnerFace { .. } => Ok(KernelOp::MergeInnerFace {
            sub_face: face(unknown_face_sticky)?,
        }),
        KernelOp::ExtrudeSubFace { distance, .. } => Ok(KernelOp::ExtrudeSubFace {
            sub_face: face(unknown_face_pp)?,
            distance: *distance,
        }),
        KernelOp::CollapseSubFace { .. } => Ok(KernelOp::CollapseSubFace {
            sub_face: face(unknown_face_pp)?,
        }),
        KernelOp::MergeFaces { .. } => match anchor {
            Anchor::Edge { a, b } => resolve_edge(object, *a, *b)
                .map(|edge| KernelOp::MergeFaces { edge })
                .ok_or(KernelOpError::Sticky(StickyError::UnknownEdge)),
            Anchor::Face { .. } => Err(KernelOpError::Sticky(StickyError::UnknownEdge)),
        },
    }
}

/// Dispatch a replayed [`KernelOp`] (a recorded inverse or redo) to the
/// appropriate `Object` method in replay mode (rule 9): the ops that carry
/// obstruction heuristics run guard-exempt, because the caller verifies the
/// result against the entry's [`StateProof`]. The remaining ops have no
/// heuristic guards and dispatch identically to [`dispatch`].
fn dispatch_replay(object: &mut Object, op: &KernelOp) -> Result<KernelOpReport, KernelOpError> {
    match op {
        KernelOp::PushPull { face, distance } => object
            .push_pull_replay(*face, *distance)
            .map(KernelOpReport::PushPull)
            .map_err(KernelOpError::PushPull),
        KernelOp::ExtrudeSubFace { sub_face, distance } => object
            .extrude_sub_face_replay(*sub_face, *distance)
            .map(KernelOpReport::ExtrudeSubFace)
            .map_err(KernelOpError::PushPull),
        _ => dispatch(object, op),
    }
}

/// Dispatch a [`KernelOp`] to the appropriate `Object` method.
fn dispatch(object: &mut Object, op: &KernelOp) -> Result<KernelOpReport, KernelOpError> {
    match op {
        KernelOp::PushPull { face, distance } => object
            .push_pull(*face, *distance)
            .map(KernelOpReport::PushPull)
            .map_err(KernelOpError::PushPull),
        KernelOp::SplitFace { face, path } => object
            .split_face(*face, path)
            .map(KernelOpReport::FaceSplit)
            .map_err(KernelOpError::Sticky),
        KernelOp::MergeFaces { edge } => object
            .merge_faces(*edge)
            .map(KernelOpReport::FaceMerge)
            .map_err(KernelOpError::Sticky),
        KernelOp::SplitFaceInner { face, loop_path } => object
            .split_face_inner(*face, loop_path)
            .map(KernelOpReport::FaceSplitInner)
            .map_err(KernelOpError::Sticky),
        KernelOp::MergeInnerFace { sub_face } => object
            .merge_inner_face(*sub_face)
            .map(KernelOpReport::FaceMergeInner)
            .map_err(KernelOpError::Sticky),
        KernelOp::ExtrudeSubFace { sub_face, distance } => object
            .extrude_sub_face(*sub_face, *distance)
            .map(KernelOpReport::ExtrudeSubFace)
            .map_err(KernelOpError::PushPull),
        KernelOp::CollapseSubFace { sub_face } => object
            .collapse_sub_face(*sub_face)
            .map(KernelOpReport::CollapseSubFace)
            .map_err(KernelOpError::PushPull),
    }
}

/// Derive the inverse of `op` from its `report`.
///
/// For `MergeFaces`, `pre_merge_path` must contain the path reconstructed
/// before the merge ran (it is `None` for the other two ops).
fn derive_inverse(
    op: &KernelOp,
    report: &KernelOpReport,
    pre_merge_path: Option<Vec<Point3>>,
) -> KernelOp {
    match (op, report) {
        (KernelOp::PushPull { .. }, KernelOpReport::PushPull(r)) => KernelOp::PushPull {
            face: r.face,
            distance: -match op {
                KernelOp::PushPull { distance, .. } => *distance,
                _ => unreachable!(),
            },
        },
        (KernelOp::SplitFace { .. }, KernelOpReport::FaceSplit(r)) => KernelOp::MergeFaces {
            edge: r.new_edges[0],
        },
        (KernelOp::MergeFaces { .. }, KernelOpReport::FaceMerge(r)) => KernelOp::SplitFace {
            face: r.merged_face,
            path: pre_merge_path
                .expect("pre_merge_path must be provided for MergeFaces inverse derivation"),
        },
        (KernelOp::SplitFaceInner { .. }, KernelOpReport::FaceSplitInner(r)) => {
            KernelOp::MergeInnerFace {
                sub_face: r.sub_face,
            }
        }
        (KernelOp::MergeInnerFace { .. }, KernelOpReport::FaceMergeInner(r)) => {
            // The merge captured the loop positions in its report (post-op state),
            // so re-imprinting them on the restored parent redoes the split.
            KernelOp::SplitFaceInner {
                face: r.parent,
                loop_path: r.loop_path.clone(),
            }
        }
        (KernelOp::ExtrudeSubFace { .. }, KernelOpReport::ExtrudeSubFace(r)) => {
            KernelOp::CollapseSubFace { sub_face: r.face }
        }
        (KernelOp::CollapseSubFace { .. }, KernelOpReport::CollapseSubFace(r)) => {
            KernelOp::ExtrudeSubFace {
                sub_face: r.sub_face,
                distance: r.distance,
            }
        }
        _ => panic!("derive_inverse: op and report type mismatch — kernel bug"),
    }
}

/// Reconstruct the ordered sequence of vertex positions along the shared edge
/// chain that `merge_faces(edge)` will dissolve.
///
/// This must be called BEFORE the merge runs, because:
/// - The half-edges and vertices along the chain die during `do_merge_faces`.
/// - Boundary-healing may also remove the chain-endpoint vertices if they
///   become collinear (scar vertices), so we must capture them here.
///
/// The returned `Vec<Point3>` is the path as `split_face` expects it:
/// `[chain_start_vertex, interior_vertices..., chain_end_vertex]`, ordered
/// along the shared chain from face_a's perspective.
///
/// If `edge` is not valid (stale or boundary), returns an empty vec; the
/// subsequent `merge_faces` call will produce a `StickyError` and the empty
/// path will never reach `derive_inverse`.
fn reconstruct_merge_path(object: &Object, edge: EdgeId) -> Vec<Point3> {
    // Look up the edge.
    let edge_data = match object.edges().get(edge) {
        Some(e) => *e,
        None => return Vec::new(), // stale edge — merge will error; path won't be used
    };

    let he_id = edge_data.half_edge;

    // Determine face_a and face_b from the two half-edges.
    let loop_a = object.half_edges()[he_id].loop_id;
    let face_a = object.loops()[loop_a].face;

    let twin_he_id = match edge_data.twin_half_edge {
        Some(t) => t,
        None => return Vec::new(), // boundary edge — merge will error
    };
    let loop_b = object.half_edges()[twin_he_id].loop_id;
    let face_b = object.loops()[loop_b].face;

    if face_a == face_b {
        return Vec::new(); // SameFaceOnBothSides — merge will error
    }

    // Collect all half-edges on outer_a.
    let outer_a = object.faces()[face_a].outer_loop;
    let outer_b = object.faces()[face_b].outer_loop;

    let hes_a: Vec<HalfEdgeId> = object.loop_half_edges(outer_a).collect();
    let hes_b_set: std::collections::BTreeSet<HalfEdgeId> =
        object.loop_half_edges(outer_b).collect();

    // Find all half-edges on outer_a whose twin is on outer_b.
    let shared_set_a: std::collections::BTreeSet<HalfEdgeId> = hes_a
        .iter()
        .copied()
        .filter(|&h| {
            object.half_edges()[h]
                .twin
                .map(|t| hes_b_set.contains(&t))
                .unwrap_or(false)
        })
        .collect();

    if shared_set_a.is_empty() {
        return Vec::new();
    }

    // Find the start of the chain: a shared half-edge whose prev is NOT shared.
    let chain_start = match shared_set_a.iter().copied().find(|&h| {
        let prev = object.half_edges()[h].prev;
        !shared_set_a.contains(&prev)
    }) {
        Some(h) => h,
        None => return Vec::new(),
    };

    // Walk the chain along outer_a, collecting ordered vertex positions.
    // The path is: origin(chain[0]), origin(chain[1]), ..., origin(chain[n]),
    // then the destination of the last segment = origin(next(chain[n])).
    let mut path: Vec<Point3> = Vec::new();
    let mut cur = chain_start;
    loop {
        let origin_v = object.half_edges()[cur].origin;
        path.push(object.vertices()[origin_v].position);
        let nxt = object.half_edges()[cur].next;
        if shared_set_a.contains(&nxt) {
            cur = nxt;
        } else {
            // cur is the last segment; push the destination vertex too.
            let dest_v = object.half_edges()[nxt].origin;
            path.push(object.vertices()[dest_v].position);
            break;
        }
    }

    // Canonical orientation. This path is reconstructed geometry for a
    // replayed split, not the user's original stroke, and "face_a's
    // perspective" flips with whichever half-edge happens to be the edge's
    // primary — which varies across undo/redo cycles. Orient the path
    // deterministically (lexicographically smaller endpoint first) so that
    // replaying the same log repeatedly assigns vertex slots identically and
    // saves stay byte-identical (DEVELOPMENT.md determinism lane).
    if let (Some(first), Some(last)) = (path.first(), path.last()) {
        let key = |p: &Point3| (p.x, p.y, p.z);
        let (f, l) = (key(first), key(last));
        let reversed =
            l.0.total_cmp(&f.0)
                .then(l.1.total_cmp(&f.1))
                .then(l.2.total_cmp(&f.2))
                == std::cmp::Ordering::Less;
        if reversed {
            path.reverse();
        }
    }

    path
}

// ================================================================= unit tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::topo::WatertightState;

    // ----------------------------------------------------------------- helpers

    fn unit_cube() -> Object {
        Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap()
    }

    fn face_with_normal(obj: &Object, dir: Vec3) -> FaceId {
        obj.faces()
            .iter()
            .find(|(_, f)| {
                f.plane
                    .normal()
                    .approx_eq(dir, crate::tol::NORMAL_DIRECTION)
            })
            .map(|(id, _)| id)
            .expect("face with that normal must exist")
    }

    #[test]
    fn peek_exposes_pending_ops_without_consuming() {
        let mut cube = unit_cube();
        let mut history = History::new();
        assert!(history.peek_undo().is_none());
        assert!(history.peek_redo().is_none());

        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        history
            .apply(
                &mut cube,
                KernelOp::PushPull {
                    face: top,
                    distance: 0.5,
                },
            )
            .unwrap();
        assert!(matches!(
            history.peek_undo(),
            Some(KernelOp::PushPull { distance, .. }) if *distance == -0.5
        ));
        assert!(history.peek_redo().is_none());

        history.undo(&mut cube).unwrap();
        assert!(history.peek_undo().is_none());
        assert!(matches!(
            history.peek_redo(),
            Some(KernelOp::PushPull { distance, .. }) if *distance == 0.5
        ));
    }

    /// Same multiset of faces as cyclically-matching position lists, within POINT_MERGE.
    fn objects_equivalent(x: &Object, y: &Object) -> bool {
        fn polygons_of(obj: &Object) -> Vec<Vec<Point3>> {
            let (points, faces) = obj.to_polygons();
            faces
                .into_iter()
                .map(|poly| poly.into_iter().map(|i| points[i]).collect())
                .collect()
        }
        fn cyclic_match(a: &[Point3], b: &[Point3]) -> bool {
            a.len() == b.len()
                && (0..a.len()).any(|shift| {
                    a.iter().enumerate().all(|(i, p)| {
                        p.approx_eq(b[(i + shift) % b.len()], crate::tol::POINT_MERGE)
                    })
                })
        }
        let xs = polygons_of(x);
        let mut ys = polygons_of(y);
        if xs.len() != ys.len() {
            return false;
        }
        for poly in xs {
            match ys.iter().position(|cand| cyclic_match(&poly, cand)) {
                Some(i) => {
                    ys.swap_remove(i);
                }
                None => return false,
            }
        }
        true
    }

    // ---------------------------------------------------- undo on empty stack

    #[test]
    fn undo_on_empty_stack_returns_nothing_to_undo() {
        let mut cube = unit_cube();
        let mut history = History::new();
        assert_eq!(history.undo(&mut cube), Err(HistoryError::NothingToUndo));
    }

    // ------------------------------------------- redo on empty stack

    #[test]
    fn redo_on_empty_stack_returns_nothing_to_redo() {
        let mut cube = unit_cube();
        let mut history = History::new();
        assert_eq!(history.redo(&mut cube), Err(HistoryError::NothingToRedo));
    }

    // ------------------------------------------- failing apply leaves stacks untouched

    #[test]
    fn failing_apply_leaves_stacks_untouched() {
        let mut cube = unit_cube();
        let mut history = History::new();

        // A PushPull with DistanceTooSmall should fail without recording anything.
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let result = history.apply(
            &mut cube,
            KernelOp::PushPull {
                face: top,
                distance: crate::tol::POINT_MERGE / 2.0,
            },
        );
        assert!(result.is_err(), "should fail with DistanceTooSmall");
        assert!(
            !history.can_undo(),
            "undo stack must be empty after failed apply"
        );
        assert!(
            !history.can_redo(),
            "redo stack must be empty after failed apply"
        );
    }

    // ------------------------------------------- apply clears redo stack

    #[test]
    fn apply_after_undo_clears_redo() {
        let mut cube = unit_cube();
        let mut history = History::new();

        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        history
            .apply(
                &mut cube,
                KernelOp::PushPull {
                    face: top,
                    distance: 0.5,
                },
            )
            .unwrap();

        history.undo(&mut cube).unwrap();
        assert!(history.can_redo(), "redo should be available after undo");

        // Now apply a new op — must discard the redo stack.
        let top2 = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        history
            .apply(
                &mut cube,
                KernelOp::PushPull {
                    face: top2,
                    distance: 0.3,
                },
            )
            .unwrap();
        assert!(
            !history.can_redo(),
            "redo stack must be cleared after a new apply"
        );
    }

    // ------------------------------------------- double undo/redo cycle

    /// apply → undo → redo → undo → redo: object equivalent at each checkpoint.
    #[test]
    fn double_undo_redo_cycle_push_pull() {
        let original = unit_cube();
        let mut cube = original.clone();
        let mut history = History::new();

        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        history
            .apply(
                &mut cube,
                KernelOp::PushPull {
                    face: top,
                    distance: 0.7,
                },
            )
            .unwrap();
        let after_apply = cube.clone();
        assert!(!objects_equivalent(&cube, &original));

        // First undo.
        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &original),
            "after first undo must equal original"
        );

        // First redo.
        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &after_apply),
            "after first redo must equal post-apply state"
        );

        // Second undo.
        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &original),
            "after second undo must equal original"
        );

        // Second redo.
        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &after_apply),
            "after second redo must equal post-apply state"
        );
    }

    /// Double undo/redo cycle for SplitFace → MergeFaces inverse pair.
    #[test]
    fn double_undo_redo_cycle_split_face() {
        let original = unit_cube();
        let mut cube = original.clone();
        let mut history = History::new();

        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let path = vec![Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
        history
            .apply(&mut cube, KernelOp::SplitFace { face: top, path })
            .unwrap();
        let after_split = cube.clone();
        assert_eq!(cube.faces().len(), 7, "split produces 7 faces");

        // First undo (merge).
        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &original),
            "after first undo must equal original"
        );
        assert_eq!(cube.watertight(), WatertightState::Watertight);

        // First redo (split).
        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &after_split),
            "after first redo must equal post-split state"
        );
        assert_eq!(cube.faces().len(), 7);

        // Second undo (merge).
        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &original),
            "after second undo must equal original"
        );

        // Second redo (split).
        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &after_split),
            "after second redo must equal post-split"
        );
        assert_eq!(cube.faces().len(), 7);
    }

    /// Double undo/redo cycle for MergeFaces → SplitFace inverse pair.
    #[test]
    fn double_undo_redo_cycle_merge_faces() {
        let original = unit_cube();
        let mut cube = original.clone();
        let mut history = History::new();

        // First split the top face so we can then merge.
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let path = vec![Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
        let split_report = cube.split_face(top, &path).unwrap();
        let pre_merge = cube.clone();

        // Now apply a merge via history.
        history
            .apply(
                &mut cube,
                KernelOp::MergeFaces {
                    edge: split_report.new_edges[0],
                },
            )
            .unwrap();
        let after_merge = cube.clone();
        // The merged cube should look like the original (6 faces).
        assert_eq!(cube.faces().len(), 6);

        // First undo (split).
        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &pre_merge),
            "after first undo must equal pre-merge state"
        );
        assert_eq!(cube.faces().len(), 7);

        // First redo (merge).
        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &after_merge),
            "after first redo must equal post-merge state"
        );
        assert_eq!(cube.faces().len(), 6);

        // Second undo (split).
        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &pre_merge),
            "after second undo must equal pre-merge state"
        );

        // Second redo (merge).
        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &after_merge),
            "after second redo must equal post-merge"
        );
        assert_eq!(cube.faces().len(), 6);
    }

    /// Imprint a sub-face via History; undo dissolves it, redo restores it.
    #[test]
    fn double_undo_redo_cycle_split_face_inner() {
        let original = unit_cube();
        let mut cube = original.clone();
        let mut history = History::new();

        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let rect = vec![
            Point3::new(0.25, 0.25, 1.0),
            Point3::new(0.75, 0.25, 1.0),
            Point3::new(0.75, 0.75, 1.0),
            Point3::new(0.25, 0.75, 1.0),
        ];
        history
            .apply(
                &mut cube,
                KernelOp::SplitFaceInner {
                    face: top,
                    loop_path: rect,
                },
            )
            .unwrap();
        let after = cube.clone();
        assert_eq!(cube.faces().len(), 7);

        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &original),
            "undo dissolves the sub-face back to the original cube"
        );

        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(objects_equivalent(&cube, &after), "redo re-imprints");

        history.undo(&mut cube).unwrap();
        assert!(objects_equivalent(&cube, &original));
    }

    /// Boss a sub-face via History; undo flattens it, redo re-raises it.
    #[test]
    fn double_undo_redo_cycle_extrude_sub_face() {
        let mut cube = unit_cube();
        let mut history = History::new();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let rect = vec![
            Point3::new(0.25, 0.25, 1.0),
            Point3::new(0.75, 0.25, 1.0),
            Point3::new(0.75, 0.75, 1.0),
            Point3::new(0.25, 0.75, 1.0),
        ];
        let report = history
            .apply(
                &mut cube,
                KernelOp::SplitFaceInner {
                    face: top,
                    loop_path: rect,
                },
            )
            .unwrap();
        let sub = match report {
            KernelOpReport::FaceSplitInner(r) => r.sub_face,
            _ => unreachable!(),
        };
        let imprinted = cube.clone();

        history
            .apply(
                &mut cube,
                KernelOp::ExtrudeSubFace {
                    sub_face: sub,
                    distance: 0.4,
                },
            )
            .unwrap();
        let bossed = cube.clone();

        history.undo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &imprinted),
            "undo flattens the boss"
        );

        history.redo(&mut cube).unwrap();
        cube.validate().unwrap();
        assert!(
            objects_equivalent(&cube, &bossed),
            "redo re-raises the boss"
        );
    }
}
