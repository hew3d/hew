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

use crate::ids::{EdgeId, FaceId, HalfEdgeId};
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
///
/// # Stack invariants
///
/// `applied` holds full [`HistoryEntry`] records (op + derived inverse).
/// `undone` holds only the operations to re-apply on redo; the inverse is
/// re-derived at redo-time from the fresh report, so stale handles never
/// appear in the wrong slot. This is the key to handle-staleness safety:
/// every inverse is derived from the *post-op* report at the moment the op
/// runs, never stored across a mutation that would invalidate its handles.
#[derive(Debug, Clone, Default)]
pub struct History {
    applied: Vec<HistoryEntry>,
    /// Ops to re-apply on redo. Inverses are re-derived at redo-time.
    undone: Vec<KernelOp>,
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

        // Dispatch the op. On error the object is untouched (strong guarantee).
        let report = dispatch(object, &op)?;

        // Derive the inverse from the report (post-apply handles).
        let inverse = derive_inverse(&op, &report, pre_merge_path);

        // Push the entry and clear the redo stack (branch-discard).
        self.applied.push(HistoryEntry { op, inverse });
        self.undone.clear();

        Ok(report)
    }

    /// Reverses the most recent applied op and moves it to the redo stack.
    ///
    /// The redo entry pushed to `undone` is the op derived from the inverse's
    /// report — i.e., the freshly re-anchored forward op. Its inverse is
    /// re-derived at redo-time, never stale.
    pub fn undo(&mut self, object: &mut Object) -> Result<KernelOpReport, HistoryError> {
        let entry = self.applied.pop().ok_or(HistoryError::NothingToUndo)?;

        // For MergeFaces in the inverse, capture path before dispatch.
        let pre_merge_path: Option<Vec<Point3>> =
            if let KernelOp::MergeFaces { edge } = &entry.inverse {
                Some(reconstruct_merge_path(object, *edge))
            } else {
                None
            };

        // Run the inverse. On kernel bug this surfaces as InverseFailed.
        let report = dispatch(object, &entry.inverse).map_err(HistoryError::InverseFailed)?;

        // The op to push onto the redo stack is the RE-ANCHORED forward op:
        // derived from the inverse's report (post-undo handles).  This is the
        // fresh op that redo will replay; its own inverse will be derived at
        // redo-time from the redo's report.
        let redo_op = derive_inverse(&entry.inverse, &report, pre_merge_path);
        self.undone.push(redo_op);

        Ok(report)
    }

    /// Re-applies the most recently undone op.
    ///
    /// Pops the redo op, runs it, derives a fresh inverse from the report,
    /// and pushes a complete [`HistoryEntry`] onto the undo stack.
    pub fn redo(&mut self, object: &mut Object) -> Result<KernelOpReport, HistoryError> {
        let redo_op = self.undone.pop().ok_or(HistoryError::NothingToRedo)?;

        // For MergeFaces, capture path before dispatch.
        let pre_merge_path: Option<Vec<Point3>> = if let KernelOp::MergeFaces { edge } = &redo_op {
            Some(reconstruct_merge_path(object, *edge))
        } else {
            None
        };

        // Run the redo op. Failure here is a kernel bug.
        let report = dispatch(object, &redo_op).map_err(HistoryError::InverseFailed)?;

        // Derive a fresh inverse from this redo's report and push onto applied.
        let new_inverse = derive_inverse(&redo_op, &report, pre_merge_path);
        self.applied.push(HistoryEntry {
            op: redo_op,
            inverse: new_inverse,
        });

        Ok(report)
    }
}

// ================================================================= private helpers

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
    let hes_b_set: std::collections::HashSet<HalfEdgeId> =
        object.loop_half_edges(outer_b).collect();

    // Find all half-edges on outer_a whose twin is on outer_b.
    let shared_set_a: std::collections::HashSet<HalfEdgeId> = hes_a
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
}
