//! The document model: the authoritative entity tree (ARCHITECTURE.md).
//!
//! A [`Document`] owns the first-class [`Sketch`] and solid [`Object`] entities
//! plus the document-level undo/redo log. It is the kernel's model authority.
//! The wasm-api `Scene` is a thin FFI shim over it: inference and tessellation
//! live there, since the kernel may not depend on those sibling crates
//! (DEVELOPMENT.md rule 1).
//!
//! # Why a Document and not a bag of Objects
//!
//! 2D geometry is a first-class, persistent Sketch — movable/copyable/deletable
//! before extrusion — and distinct from a Solid. The Document holds many
//! Sketches and many Objects, so independent coplanar shapes are expressible
//! (the single ephemeral sketch could not represent them).
//!
//! # Undo model
//!
//! [`Document`] keeps a document-level command log layered over each Object's
//! per-Object [`History`]. Two kinds of step exist:
//!
//! - **Object creation** ([`DocAction::CreatedObject`]) — undone by *hiding*
//!   the Object, never deleting it, so its [`ObjectId`] stays stable across
//!   undo/redo and any later per-Object op keeps referring to a live handle.
//!   Undo also restores the consumed sketch region's extrudability.
//! - **A per-Object op** ([`DocAction::ObjectOp`]) — undo/redo delegate to that
//!   Object's [`History`].
//!
//! Each mutation returns a [`DocChange`] naming the entities it touched, so the
//! shim can reconcile inference candidates and render caches precisely without
//! the kernel knowing those concerns exist.

use std::collections::HashSet;

use slotmap::SlotMap;

use crate::history::{History, HistoryError, KernelOp, KernelOpError, KernelOpReport};
use crate::ids::{ObjectId, SketchId};
use crate::math::Plane;
use crate::ops::{BooleanError, BooleanOp, ExtrudeError};
use crate::sketch::{Sketch, SketchError, SketchRegionId};
use crate::topo::Object;
use crate::transform::{Transform, TransformError};

/// A solid Object plus its undo history and visibility.
///
/// `hidden` marks an undone creation: kept in the slotmap (so the [`ObjectId`]
/// stays valid for redo and for any later op in its [`History`]) but excluded
/// from [`Document::visible_object_ids`].
#[derive(Debug, Clone)]
struct ObjectRecord {
    object: Object,
    history: History,
    hidden: bool,
}

/// One document-level step on the undo stack.
///
/// Object creation is undone by hiding (not deleting), so the `ObjectId` never
/// churns — redo just unhides, and a later `ObjectOp` still refers to a live
/// handle.
// `Transform` carries f64s, so this is `PartialEq` but not `Eq`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DocAction {
    /// `extrude_region` created an Object from a sketch region. Undo hides the
    /// Object and restores the region's extrudability; redo reverses both.
    CreatedObject {
        id: ObjectId,
        sketch: SketchId,
        region: SketchRegionId,
    },
    /// A per-Object op (push/pull, split, merge) ran; undo/redo delegate to that
    /// Object's [`History`].
    ObjectOp { object: ObjectId },
    /// A boolean combined two objects into one. Undo hides `result` and unhides
    /// the operands; redo reverses. Like `CreatedObject`, all three handles stay
    /// stable (hide-not-delete), so later ops keep referring to live handles.
    Boolean {
        result: ObjectId,
        a: ObjectId,
        b: ObjectId,
    },
    /// A move/rotate/scale baked into one object's geometry. Undo bakes
    /// `inverse`, redo bakes `forward`; the transform is handle-stable so the
    /// `ObjectId` never changes.
    Transform {
        object: ObjectId,
        forward: Transform,
        inverse: Transform,
    },
}

/// The entities a mutation touched, so the caller (the shim) can reconcile its
/// own derived state (inference candidates, render caches) precisely.
///
/// "Touched" means *may have changed* — an Object whose visibility flipped, an
/// Object whose geometry changed, or a Sketch whose extrudable regions changed.
/// The caller queries current [`Document`] state for the details.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocChange {
    /// Objects whose geometry or visibility may have changed.
    pub objects_touched: Vec<ObjectId>,
    /// Sketches whose contents or extrudable regions may have changed.
    pub sketches_touched: Vec<SketchId>,
}

/// Typed failures of document operations. Nothing is repaired silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentError {
    /// The sketch handle is stale or from another Document.
    UnknownSketch,
    /// The object handle is stale, hidden, or from another Document.
    UnknownObject,
    /// A sketch operation (region lookup / profile tracing) failed.
    Sketch(SketchError),
    /// Extruding the region into a solid failed.
    Extrude(ExtrudeError),
    /// A boolean combine failed (non-solid operand, empty result, degenerate
    /// contact, …).
    Boolean(BooleanError),
    /// A move/rotate/scale failed (singular or orientation-flipping transform).
    Transform(TransformError),
    /// A per-Object op failed to apply.
    Op(KernelOpError),
    /// Undo with an empty document undo stack.
    NothingToUndo,
    /// Redo with an empty document redo stack.
    NothingToRedo,
    /// Replaying a per-Object inverse failed — a kernel bug, surfaced loudly.
    InverseFailed(KernelOpError),
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentError::UnknownSketch => write!(f, "no such sketch in this document"),
            DocumentError::UnknownObject => write!(f, "no such object in this document"),
            DocumentError::Sketch(e) => write!(f, "{e}"),
            DocumentError::Extrude(e) => write!(f, "{e}"),
            DocumentError::Boolean(e) => write!(f, "{e}"),
            DocumentError::Transform(e) => write!(f, "{e}"),
            DocumentError::Op(e) => write!(f, "{e}"),
            DocumentError::NothingToUndo => write!(f, "nothing to undo"),
            DocumentError::NothingToRedo => write!(f, "nothing to redo"),
            DocumentError::InverseFailed(e) => write!(f, "inverse op failed (kernel bug): {e}"),
        }
    }
}

impl std::error::Error for DocumentError {}

/// The authoritative model: the tree of Sketches and solid Objects plus the
/// document-level undo/redo log. UI-free and I/O-free (DEVELOPMENT.md rule 1).
#[derive(Debug, Clone, Default)]
pub struct Document {
    sketches: SlotMap<SketchId, Sketch>,
    objects: SlotMap<ObjectId, ObjectRecord>,
    /// `(sketch, region)` pairs already extruded into a solid: such a region is
    /// the bottom of its box and is no longer offered for extrusion. Keyed by
    /// sketch too because a different sketch's slotmap reuses region keys.
    consumed: HashSet<(SketchId, SketchRegionId)>,
    undo: Vec<DocAction>,
    redo: Vec<DocAction>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Document {
        Document::default()
    }

    // --------------------------------------------------------------- sketches

    /// Adds a fresh, empty sketch on `plane` and returns its handle. **Additive**
    /// — existing sketches are untouched, so independent coplanar shapes can
    /// coexist. Plane choice (ground or a face) is the caller's concern.
    pub fn add_sketch(&mut self, plane: Plane) -> SketchId {
        self.sketches.insert(Sketch::on_plane(plane))
    }

    /// A sketch by handle, or `None` if stale.
    pub fn sketch(&self, id: SketchId) -> Option<&Sketch> {
        self.sketches.get(id)
    }

    /// A mutable sketch by handle, or `None` if stale.
    ///
    /// Sketch edits do not flow through the document undo log (sketch-level undo
    /// is a later milestone); they are surfaced to the caller via the returned
    /// handle and reconciled through [`Document::sketch`] reads.
    pub fn sketch_mut(&mut self, id: SketchId) -> Option<&mut Sketch> {
        self.sketches.get_mut(id)
    }

    /// All sketch handles, in unspecified but stable order.
    pub fn sketch_ids(&self) -> Vec<SketchId> {
        self.sketches.keys().collect()
    }

    /// Whether `region` of `sketch` has already been extruded into a solid (and
    /// is therefore no longer extrudable).
    pub fn is_region_consumed(&self, sketch: SketchId, region: SketchRegionId) -> bool {
        self.consumed.contains(&(sketch, region))
    }

    /// The still-extrudable regions of `sketch` (its closed regions minus any
    /// already consumed by an extrusion). `Err` if the sketch is stale.
    pub fn extrudable_regions(
        &self,
        sketch: SketchId,
    ) -> Result<Vec<SketchRegionId>, DocumentError> {
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        Ok(s.regions()
            .keys()
            .filter(|&r| !self.consumed.contains(&(sketch, r)))
            .collect())
    }

    // ---------------------------------------------------------------- objects

    /// A visible Object by handle, or `None` if stale or hidden.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        match self.objects.get(id) {
            Some(rec) if !rec.hidden => Some(&rec.object),
            _ => None,
        }
    }

    /// Handles of all currently visible Objects (undone creations are hidden,
    /// not listed), in unspecified but stable order.
    pub fn visible_object_ids(&self) -> Vec<ObjectId> {
        self.objects
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, _)| id)
            .collect()
    }

    // -------------------------------------------------------------- mutations

    /// THE solid-creating act (ARCHITECTURE.md): extrudes a closed sketch region into
    /// a new watertight Object, consumes the region, and records the creation on
    /// the document undo log.
    ///
    /// Returns the new Object's handle and the [`DocChange`] it caused (the new
    /// Object plus the sketch whose extrudable set shrank).
    pub fn extrude_region(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        distance: f64,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let object = Object::from_extrusion(&profile, distance).map_err(DocumentError::Extrude)?;

        let id = self.objects.insert(ObjectRecord {
            object,
            history: History::new(),
            hidden: false,
        });
        // The region is now the bottom of a solid: consume it so it neither
        // re-extrudes nor leaves a stray fill.
        self.consumed.insert((sketch, region));
        self.undo
            .push(DocAction::CreatedObject { id, sketch, region });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: vec![id],
            sketches_touched: vec![sketch],
        };
        Ok((id, change))
    }

    /// Applies a per-Object op (push/pull, split, merge) through that Object's
    /// undo [`History`] and records a document-level step delegating to it.
    ///
    /// On `Err` the Object is untouched (the op's strong guarantee) and nothing
    /// is recorded.
    pub fn apply_object_op(
        &mut self,
        object: ObjectId,
        op: KernelOp,
    ) -> Result<(KernelOpReport, DocChange), DocumentError> {
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        let report = rec
            .history
            .apply(&mut rec.object, op)
            .map_err(DocumentError::Op)?;
        self.undo.push(DocAction::ObjectOp { object });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: vec![object],
            sketches_touched: Vec::new(),
        };
        Ok((report, change))
    }

    /// Explicitly combines two visible objects (union / subtract / intersect)
    /// into a new object, consuming the operands. Subtract is `a - b`.
    ///
    /// Document objects share the world frame, so the operands map with the
    /// identity transform. On success the operands are hidden and the result is
    /// the only visible product; on `Err` nothing changes (the op's strong
    /// guarantee) and the operands stay visible.
    pub fn boolean(
        &mut self,
        op: BooleanOp,
        a: ObjectId,
        b: ObjectId,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        if a == b {
            // A single object cannot be combined with itself (its faces would be
            // fully coincident — a degenerate contact); reject before mutating.
            return Err(DocumentError::Boolean(BooleanError::DegenerateContact));
        }
        let obj_a = match self.objects.get(a) {
            Some(rec) if !rec.hidden => &rec.object,
            _ => return Err(DocumentError::UnknownObject),
        };
        let obj_b = match self.objects.get(b) {
            Some(rec) if !rec.hidden => &rec.object,
            _ => return Err(DocumentError::UnknownObject),
        };

        let result = Object::boolean(op, obj_a, obj_b, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;

        let id = self.objects.insert(ObjectRecord {
            object: result,
            history: History::new(),
            hidden: false,
        });
        self.objects[a].hidden = true;
        self.objects[b].hidden = true;
        self.undo.push(DocAction::Boolean { result: id, a, b });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: vec![a, b, id],
            sketches_touched: Vec::new(),
        };
        Ok((id, change))
    }

    /// Move / rotate / scale a visible object by baking `t` into its geometry.
    /// Undoable via the exact inverse; the object keeps its handle. `Err` if the
    /// object is unknown/hidden or `t` is singular or orientation-flipping —
    /// nothing changes in that case (the op's strong guarantee).
    pub fn transform_object(
        &mut self,
        object: ObjectId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        // Capture the inverse first: it both validates invertibility and is what
        // undo will bake. (`apply_transform` re-checks and also rejects det<0.)
        let inverse = t.inverse().map_err(DocumentError::Transform)?;
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        rec.object
            .apply_transform(t)
            .map_err(DocumentError::Transform)?;
        self.undo.push(DocAction::Transform {
            object,
            forward: *t,
            inverse,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: vec![object],
            sketches_touched: Vec::new(),
        })
    }

    // ------------------------------------------------------------ undo / redo

    /// True if there is a document-level action to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True if there is a document-level action to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverses the most recent document action (LIFO across creations and
    /// per-Object ops alike) and returns what it touched.
    pub fn undo(&mut self) -> Result<DocChange, DocumentError> {
        let action = self.undo.pop().ok_or(DocumentError::NothingToUndo)?;
        let change = match action {
            DocAction::CreatedObject { id, sketch, region } => {
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.hidden = true;
                }
                self.consumed.remove(&(sketch, region));
                DocChange {
                    objects_touched: vec![id],
                    sketches_touched: vec![sketch],
                }
            }
            DocAction::ObjectOp { object } => {
                let rec = &mut self.objects[object];
                rec.history.undo(&mut rec.object).map_err(map_history_err)?;
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                }
            }
            DocAction::Boolean { result, a, b } => {
                // Undo a combine: hide the result, bring the operands back.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = true;
                }
                self.objects[a].hidden = false;
                self.objects[b].hidden = false;
                DocChange {
                    objects_touched: vec![result, a, b],
                    sketches_touched: Vec::new(),
                }
            }
            DocAction::Transform {
                object, inverse, ..
            } => {
                // Undo a transform by baking its exact inverse.
                self.objects[object]
                    .object
                    .apply_transform(&inverse)
                    .expect("inverse of a validated transform must re-apply");
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                }
            }
        };
        self.redo.push(action);
        self.debug_validate();
        Ok(change)
    }

    /// Re-applies the most recently undone document action. Object handles are
    /// stable across undo/redo (undone creations are hidden, not deleted), so
    /// redo never has to remap ids.
    pub fn redo(&mut self) -> Result<DocChange, DocumentError> {
        let action = self.redo.pop().ok_or(DocumentError::NothingToRedo)?;
        let change = match action {
            DocAction::CreatedObject { id, sketch, region } => {
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.hidden = false;
                }
                self.consumed.insert((sketch, region));
                DocChange {
                    objects_touched: vec![id],
                    sketches_touched: vec![sketch],
                }
            }
            DocAction::ObjectOp { object } => {
                let rec = &mut self.objects[object];
                rec.history.redo(&mut rec.object).map_err(map_history_err)?;
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                }
            }
            DocAction::Boolean { result, a, b } => {
                // Redo a combine: hide the operands again, show the result.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = false;
                }
                self.objects[a].hidden = true;
                self.objects[b].hidden = true;
                DocChange {
                    objects_touched: vec![result, a, b],
                    sketches_touched: Vec::new(),
                }
            }
            DocAction::Transform {
                object, forward, ..
            } => {
                // Redo a transform by re-baking the forward transform.
                self.objects[object]
                    .object
                    .apply_transform(&forward)
                    .expect("forward of a validated transform must re-apply");
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                }
            }
        };
        self.undo.push(action);
        self.debug_validate();
        Ok(change)
    }

    // ----------------------------------------------------------------- checks

    /// Document-level invariants, debug builds only (DEVELOPMENT.md rule 2): every
    /// visible Object passes the topology validator, and no consumed region is
    /// still listed extrudable.
    #[inline]
    fn debug_validate(&self) {
        if cfg!(debug_assertions) {
            for (_, rec) in self.objects.iter().filter(|(_, r)| !r.hidden) {
                rec.object
                    .validate()
                    .expect("document holds an invalid visible object — kernel bug");
            }
            for &(sketch, region) in &self.consumed {
                if let Ok(extrudable) = self.extrudable_regions(sketch) {
                    debug_assert!(
                        !extrudable.contains(&region),
                        "a consumed region is still offered as extrudable — kernel bug"
                    );
                }
            }
        }
    }
}

/// Map a per-Object [`HistoryError`] onto a [`DocumentError`]. Empty-stack cases
/// cannot occur here (the document log guarantees the op exists), so they map to
/// `InverseFailed`-adjacent loud failures rather than being silently ignored.
fn map_history_err(e: HistoryError) -> DocumentError {
    match e {
        HistoryError::InverseFailed(op) => DocumentError::InverseFailed(op),
        // The document log only records ObjectOp steps that were applied, so the
        // delegated History always has the matching entry to undo/redo. Reaching
        // these is a kernel bug; surface it loudly rather than swallow it.
        HistoryError::NothingToUndo | HistoryError::NothingToRedo => {
            panic!("document/object history desync — kernel bug: {e}")
        }
    }
}
