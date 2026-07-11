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

use std::collections::{BTreeMap, BTreeSet};

use slotmap::SlotMap;
use tracing::info;

use crate::guide::Guide;
use crate::history::{History, HistoryError, KernelOp, KernelOpError, KernelOpReport};
use crate::ids::{
    ComponentId, FaceId, GroupId, GuideId, InstanceId, MaterialId, ObjectId, SketchId,
};
use crate::import::{ImportReport, ImportScene, SkippedMesh};
use crate::material::Material;
use crate::math::{MathError, Plane, Point3, Vec3};
use crate::ops::{BooleanError, BooleanOp, ExtrudeError, SliceError};
use crate::serialize::{DocSaveData, LoadError, NodeRefDto, decode_document_raw, encode_document};
use crate::sketch::{Sketch, SketchEdgeId, SketchError, SketchRegionId, SketchVertexId};
use crate::topo::{Object, WatertightState};
use crate::transform::{Transform, TransformError};

/// A node in the document tree (ARCHITECTURE.md): either a solid Object or a
/// merge [`Group`](GroupRecord). This is the unit of selection, picking, and
/// transform — *not* of rendering, which stays flat over leaf objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    /// A solid object leaf.
    Object(ObjectId),
    /// A non-destructive container of other nodes.
    Group(GroupId),
    /// A component instance: a tree node placing a shared
    /// [`ComponentDef`] at a per-instance pose (ARCHITECTURE.md).
    Instance(InstanceId),
}

/// Who owns an [`Object`] (ARCHITECTURE.md). A `World` object is a top-level or
/// grouped solid rendered directly in world space (baked). A `Definition`
/// object is a member of a [`ComponentDef`] — geometry in definition-local
/// coordinates, rendered only through that component's instances and never
/// directly, so it is excluded from [`Document::visible_object_ids`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectOwner {
    /// A world-space solid; `parent` is its containing merge group, or `None`
    /// at the top level.
    World { parent: Option<GroupId> },
    /// A member of a component definition (definition-local coordinates).
    Definition(ComponentId),
}

/// A solid Object plus its undo history, visibility, and owner.
///
/// `hidden` marks an undone creation: kept in the slotmap (so the [`ObjectId`]
/// stays valid for redo and for any later op in its [`History`]) but excluded
/// from [`Document::visible_object_ids`]. `owner` is its place in the model
/// (world tree vs. component definition).
#[derive(Debug, Clone)]
struct ObjectRecord {
    object: Object,
    history: History,
    hidden: bool,
    owner: ObjectOwner,
    /// Optional display name (e.g. carried in from an import). `None` falls back
    /// to a positional label in the UI.
    name: Option<String>,
    /// Per-node tag paths (root-first segment lists, e.g. `["Structure","Roof"]`).
    /// Empty by default; set by import decode or user ops. No global tag registry.
    tags: Vec<Vec<String>>,
}

impl ObjectRecord {
    /// The containing merge group, or `None` (top level, or a definition member
    /// — which has no tree parent).
    fn group_parent(&self) -> Option<GroupId> {
        match self.owner {
            ObjectOwner::World { parent } => parent,
            ObjectOwner::Definition(_) => None,
        }
    }

    /// Whether this is a world-space (directly rendered) solid, as opposed to a
    /// component-definition member.
    fn is_world(&self) -> bool {
        matches!(self.owner, ObjectOwner::World { .. })
    }
}

/// A merge group: a non-destructive container recording membership only — no
/// geometry, no pose. Transforming a group bakes the transform into every
/// leaf object beneath it, so a group stays purely structural.
///
/// `members` is ordered (stable across edits); `parent` is the containing group
/// or `None` at the top level. Like objects, an ungrouped group is `hidden`,
/// not deleted, so its [`GroupId`] stays valid for redo.
#[derive(Debug, Clone)]
struct GroupRecord {
    members: Vec<NodeId>,
    parent: Option<GroupId>,
    hidden: bool,
    /// Optional display name (e.g. carried in from an import). `None` falls back
    /// to a positional label in the UI.
    name: Option<String>,
    /// Per-node tag paths (root-first segment lists). Empty by default.
    tags: Vec<Vec<String>>,
}

/// A component definition (ARCHITECTURE.md): shared geometry as a flat set of leaf
/// [`Object`]s in definition-local coordinates. A *library entry*, not a tree
/// node — it has no pose and no place in space; its geometry reaches the scene
/// only through [`InstanceRecord`]s. Editing a member ([`Document::apply_def_op`])
/// changes every instance of this definition at once.
///
/// `members` is ordered and stable. `hidden` marks an undone creation
/// (`make_component` / `make_unique`), kept so the [`ComponentId`] stays valid
/// for redo. The member objects' [`ObjectRecord`]s carry
/// [`ObjectOwner::Definition`] pointing back here.
#[derive(Debug, Clone)]
struct ComponentDef {
    members: Vec<ObjectId>,
    hidden: bool,
    /// Optional definition name (e.g. a SketchUp component name), used as the
    /// display name for this definition's instances. `None` falls back to a
    /// positional label in the UI.
    name: Option<String>,
}

/// A component instance (ARCHITECTURE.md): a tree node placing a
/// [`ComponentDef`] at an invertible per-instance `pose` (definition-local →
/// world). Unlike a baked object transform, the pose may mirror
/// (determinant < 0) and scale non-uniformly — it is applied at
/// tessellation/render/inference time, never baked, so winding is handled at
/// draw time rather than refused.
///
/// `parent` is the containing merge group, or `None` at the top level; `hidden`
/// marks an undone placement/explode, kept so the [`InstanceId`] stays valid for
/// redo.
#[derive(Debug, Clone)]
struct InstanceRecord {
    def: ComponentId,
    pose: Transform,
    parent: Option<GroupId>,
    hidden: bool,
    /// Optional per-instance display name. `None` falls back to the def's name,
    /// then to a positional label, in the UI.
    name: Option<String>,
    /// Per-node tag paths (root-first segment lists). Empty by default.
    tags: Vec<Vec<String>>,
}

/// A construction guide plus its visibility. `hidden` marks an undone
/// creation or a delete, kept so the [`GuideId`] stays valid for redo —
/// exactly the tombstone pattern used for objects/groups/instances.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GuideRecord {
    guide: Guide,
    hidden: bool,
}

/// Every entity created while deep-cloning a subtree in
/// [`Document::duplicate_node`], accumulated so the clone is one atomic,
/// reversible action — and so a partial clone can be rolled back on error.
#[derive(Debug, Clone, Default, PartialEq)]
struct CreatedClone {
    objects: Vec<ObjectId>,
    groups: Vec<GroupId>,
    instances: Vec<InstanceId>,
}

/// An open sketch-drawing gesture: the snapshot taken at
/// [`Document::begin_sketch_gesture`], waiting for its `end_` to decide
/// whether anything changed and push a [`DocAction::SketchGesture`].
/// Session-only bookkeeping — never serialized (like the undo log).
#[derive(Debug, Clone)]
struct PendingSketchGesture {
    sketch: SketchId,
    before: Box<Sketch>,
    /// `sketch` was freshly added and still empty at gesture begin.
    created: bool,
}

/// One document-level step on the undo stack.
///
/// Object creation is undone by hiding (not deleting), so the `ObjectId` never
/// churns — redo just unhides, and a later `ObjectOp` still refers to a live
/// handle.
// `Transform` carries f64s, so this is `PartialEq` but not `Eq`; the `Vec`
// fields (transform targets, grouped membership) make it non-`Copy`.
#[derive(Debug, Clone, PartialEq)]
enum DocAction {
    /// `extrude_region` created an Object from a sketch region. Undo hides the
    /// Object and restores the region's extrudability; redo reverses both.
    CreatedObject {
        id: ObjectId,
        sketch: SketchId,
        region: SketchRegionId,
        /// Sketch edges exclusively bounding this region (not shared by any
        /// surviving region); hidden from rendering after extrusion, restored
        /// on undo. Derived index — not serialized.
        consumed_edges: Vec<SketchEdgeId>,
        /// Sketch vertices left isolated once `consumed_edges` are hidden;
        /// treated as hidden from rendering after extrusion, restored on undo.
        /// Derived index — not serialized.
        consumed_verts: Vec<SketchVertexId>,
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
    /// A slice cut one solid into two. Undo hides both pieces and unhides
    /// the source; redo reverses. Like `Boolean`, all three handles stay stable
    /// (hide-not-delete).
    Sliced {
        source: ObjectId,
        a: ObjectId,
        b: ObjectId,
    },
    /// A push-through subtract removed material from one solid, replacing
    /// it with one or more result shells (`results` — more than one when the cut
    /// severed the solid). Undo hides the results and unhides the source; redo
    /// reverses. All handles stay stable (hide-not-delete).
    PushThrough {
        source: ObjectId,
        results: Vec<ObjectId>,
    },
    /// A move/rotate/scale baked into one or more objects' geometry. A single
    /// object carries one target; a group transform carries every leaf object
    /// beneath it. Undo bakes `inverse` into each, redo bakes `forward`;
    /// the transform is handle-stable so the `ObjectId`s never change.
    Transform {
        objects: Vec<ObjectId>,
        forward: Transform,
        inverse: Transform,
    },
    /// A move/rotate/scale baked into a free-standing sketch's geometry (Phase
    /// D). The sketch analogue of [`DocAction::Transform`]: undo bakes
    /// `inverse`, redo bakes `forward`; the `SketchId` is handle-stable.
    TransformSketch {
        sketch: SketchId,
        forward: Transform,
        inverse: Transform,
    },
    /// A move/rotate/scale applied to a whole mixed selection in one step
    /// (`transform_selection`: select-all → Move). Baked into every world
    /// leaf object and listed sketch; composed into every leaf instance's
    /// pose. Undo bakes `inverse` into the baked targets and restores each
    /// instance's exact prior pose; redo bakes `forward` and re-composes
    /// each prior pose with it (bit-identical to the original application).
    /// All handles are stable across undo/redo.
    TransformSelection {
        objects: Vec<ObjectId>,
        sketches: Vec<SketchId>,
        /// `(instance, prior pose)` pairs, in flattening order.
        instances: Vec<(InstanceId, Transform)>,
        forward: Transform,
        inverse: Transform,
    },
    /// A single sketch vertex dragged to a new position (Phase D per-vertex
    /// edit). Topology-preserving, so the inverse is just the old position:
    /// undo restores `old_pos`, redo re-applies `new_pos`; both the `SketchId`
    /// and the `SketchVertexId` are handle-stable.
    MovedSketchVertex {
        sketch: SketchId,
        vertex: SketchVertexId,
        old_pos: Point3,
        new_pos: Point3,
    },
    /// One sketch-drawing gesture (`begin_sketch_gesture` … `end_sketch_gesture`):
    /// a whole rectangle/circle/arc — or one committed Line segment — as a
    /// single undo step. Snapshot-based rather than delta-based: post-gesture
    /// topology is the product of sticky-rule cascades (splits, merges, region
    /// recomputes), so an exact before/after image is the only inverse that
    /// cannot drift. `SlotMap` clones preserve keys, so every handle issued
    /// before the gesture stays valid across undo/redo (the hide-not-delete
    /// convention's snapshot analogue). Undo restores `before` — and when the
    /// gesture `created` the sketch, also hides it so no empty ghost lingers;
    /// redo restores `after` (unhiding first).
    SketchGesture {
        sketch: SketchId,
        /// Sketch contents at gesture begin. Boxed — a `Sketch` is three
        /// `SlotMap`s and would dominate the enum's inline size.
        before: Box<Sketch>,
        /// Sketch contents at gesture end.
        after: Box<Sketch>,
        /// The gesture drew the first geometry into a freshly-added sketch,
        /// folding "the sketch appeared" into this one undo step.
        created: bool,
    },
    /// `group_nodes` formed a group. Undo dissolves it (reparenting members to
    /// `parent` and restoring the parent's member order), redo re-forms it. The
    /// `GroupId` stays stable (hide-not-delete), as do all member handles.
    Grouped {
        group: GroupId,
        parent: Option<GroupId>,
        /// The parent group's member list immediately before grouping, for an
        /// exact undo. `None` at the top level, whose order derives from the
        /// slotmap and is unaffected by reparenting.
        prev_parent_members: Option<Vec<NodeId>>,
    },
    /// `ungroup` dissolved a group. The exact inverse of [`DocAction::Grouped`]:
    /// undo re-forms the group, redo dissolves it again.
    Ungrouped {
        group: GroupId,
        parent: Option<GroupId>,
        prev_parent_members: Option<Vec<NodeId>>,
    },
    /// `delete_node` hid a whole tree node — an Object, Group, or
    /// Instance — and its entire subtree in one undoable step (tombstone, not a
    /// real delete: every id stays valid for redo). Unlike [`DocAction::Ungrouped`],
    /// a deleted Group's members are hidden along with it rather than reparented
    /// up — the whole subtree disappears. Deleting an Instance never touches its
    /// shared [`ComponentDef`] or sibling instances. Undo unhides exactly
    /// `hidden_subtree` and re-splices `node` back into `parent` at its original
    /// position; redo re-hides the subtree and splices it out again.
    Deleted {
        node: NodeId,
        parent: Option<GroupId>,
        /// The parent group's member list immediately before the delete, for an
        /// exact undo (mirrors [`DocAction::Ungrouped::prev_parent_members`]);
        /// `None` at the top level.
        prev_parent_members: Option<Vec<NodeId>>,
        /// Every node hidden by this delete — `node` itself plus every live
        /// descendant (groups, objects, instances) beneath it — captured so
        /// undo unhides exactly this set and nothing else.
        hidden_subtree: Vec<NodeId>,
    },
    /// `make_component` folded a selection into a new definition plus
    /// one identity-posed instance. Undo dissolves it: each def member returns
    /// to the world parent it had before (`member_prior_parents`), the consumed
    /// container nodes reappear, the shared parent's order is restored, and the
    /// def + instance are hidden. Redo re-forms it. All handles stay stable.
    MadeComponent {
        component: ComponentId,
        instance: InstanceId,
        /// The selected sibling nodes folded in, in order — replayed by redo to
        /// re-splice the parent's member list.
        selected: Vec<NodeId>,
        /// The merge group the new instance was inserted into, or `None` at the
        /// top level.
        parent: Option<GroupId>,
        /// Each def-member object paired with the world parent it had before
        /// being folded in, so undo can return it to the world tree.
        member_prior_parents: Vec<(ObjectId, Option<GroupId>)>,
        /// Groups consumed (hidden) by the fold — every group node in the
        /// selected subtrees — to reappear on undo.
        consumed_groups: Vec<GroupId>,
        /// The shared parent's member list immediately before, for exact undo
        /// (mirrors [`DocAction::Grouped::prev_parent_members`]); `None` at the
        /// top level.
        prev_parent_members: Option<Vec<NodeId>>,
    },
    /// `place_instance` stamped another instance of an existing
    /// definition. Undo hides it; redo unhides. The `InstanceId` stays stable.
    PlacedInstance { instance: InstanceId },
    /// `duplicate_node` (Move+Option "copy") deep-cloned a node under the
    /// same parent. Undo hides every created entity and removes the clone root
    /// from its parent's member list; redo unhides and re-appends. All handles
    /// stay stable (hide-not-delete).
    Duplicated {
        /// The clone's root node (same kind as the source).
        root: NodeId,
        /// The parent group the clone was appended to, or `None` at top level.
        parent: Option<GroupId>,
        /// Every world object created by the clone (the root if it is an Object,
        /// plus every cloned leaf beneath a cloned Group).
        objects: Vec<ObjectId>,
        /// Every group created by the clone.
        groups: Vec<GroupId>,
        /// Every instance created by the clone.
        instances: Vec<InstanceId>,
    },
    /// `add_guide_line`/`add_guide_point` created a construction guide.
    /// Undo hides it; redo unhides. The `GuideId` stays stable.
    CreatedGuide { guide: GuideId },
    /// `delete_guide` hid one construction guide (tombstone, not a real
    /// delete). Undo unhides it; redo re-hides it. The `GuideId` stays stable.
    DeletedGuide { guide: GuideId },
    /// `delete_all_guides` (Edit ▸ Delete Guide Lines) hid every
    /// then-visible guide in one step. Undo unhides exactly these; redo
    /// re-hides them.
    DeletedGuides { guides: Vec<GuideId> },
    /// `delete_sketch` hid a free-standing sketch (tombstone, not a real
    /// delete — the `SketchId` stays valid for redo). Undo un-hides it; redo
    /// re-hides it. Mirrors [`DocAction::DeletedGuide`].
    DeletedSketch { sketch: SketchId },
    /// `transform_instance` changed an instance's pose. Undo restores
    /// `prev` exactly; redo re-applies `next`. No bake — the pose is mutable
    /// instance state, so this is exact rather than an inverse-transform.
    TransformInstance {
        instance: InstanceId,
        prev: Transform,
        next: Transform,
    },
    /// A per-Object op ran on a definition member (editing shared geometry,
    ///). Undo/redo delegate to that member object's [`History`]; the change
    /// is reflected in every instance of `component`.
    DefObjectOp {
        component: ComponentId,
        object: ObjectId,
    },
    /// `explode_instance` baked an instance's pose into independent world
    /// objects (`created`). Undo hides those and unhides the instance; redo
    /// reverses. The definition and sibling instances are untouched throughout.
    Exploded {
        instance: InstanceId,
        created: Vec<ObjectId>,
    },
    /// `make_unique` repointed an instance from its shared definition onto
    /// a fresh private copy. Undo repoints to `prev_def` and hides `new_def`;
    /// redo reverses.
    MadeUnique {
        instance: InstanceId,
        prev_def: ComponentId,
        new_def: ComponentId,
    },
    /// `paint_face` reassigned a face's material. Non-topological, so it
    /// touches no [`History`]; undo restores `prev` exactly, redo re-applies
    /// `next`. Handle-stable (the `ObjectId`/`FaceId` are untouched).
    PaintFace {
        object: ObjectId,
        face: FaceId,
        prev: Option<MaterialId>,
        next: Option<MaterialId>,
    },
    /// `set_object_material` ( follow-up) reassigned an object's base
    /// material. Like [`DocAction::PaintFace`] but on the object default; undo
    /// restores `prev`, redo re-applies `next`.
    SetObjectMaterial {
        object: ObjectId,
        prev: Option<MaterialId>,
        next: Option<MaterialId>,
    },
    /// `set_material_alpha` changed a palette material's opacity. Unlike
    /// [`DocAction::PaintFace`]/[`DocAction::SetObjectMaterial`], this mutates
    /// the palette entry itself (shared by every face/object referencing it),
    /// not an assignment; undo restores `prev`, redo re-applies `next`.
    SetMaterialAlpha {
        material: MaterialId,
        prev: u8,
        next: u8,
    },
    /// `set_node_name` / `add_node_tag` / `remove_node_tag` changed a tree
    /// node's display name or tag list (or both). Undo restores `prev_name` /
    /// `prev_tags`; redo re-applies `next_name` / `next_tags`. All three ops
    /// share one variant so rename-plus-retag in a single edit composes
    /// cleanly. The node handle is stable (no hide-not-delete needed here —
    /// the node is not created/destroyed, just annotated).
    NodeMetaChanged {
        node: NodeId,
        prev_name: Option<String>,
        next_name: Option<String>,
        prev_tags: Vec<Vec<String>>,
        next_tags: Vec<Vec<String>>,
    },
    /// `Document::ingest` merged an imported scene into this document.
    /// Undo hides every created node/object/group/instance/component (ids
    /// stay stable — hide-not-delete); redo unhides them. Materials added to
    /// the palette are not individually undone (matches `add_material`).
    Imported {
        /// Top-level created node ids (ordering / tree-root list).
        roots: Vec<NodeId>,
        /// ALL created `ObjectId`s — world objects and definition members alike.
        objects: Vec<ObjectId>,
        /// Created `ComponentId`s (shared definitions).
        components: Vec<ComponentId>,
        /// Created `InstanceId`s.
        instances: Vec<InstanceId>,
        /// Created `GroupId`s.
        groups: Vec<GroupId>,
        /// Created `GuideId`s (imported construction guides).
        guides: Vec<GuideId>,
        /// Tag paths this import NEWLY registered in the tag metadata
        /// (with their hidden flags). Undo unregisters exactly these; tags
        /// that already existed before the import are untouched.
        tags: Vec<(Vec<String>, bool)>,
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
    /// Groups whose membership or visibility may have changed.
    pub groups_touched: Vec<GroupId>,
    /// Instances whose pose, definition, or visibility may have changed.
    pub instances_touched: Vec<InstanceId>,
    /// Component definitions whose membership, geometry, or visibility may have
    /// changed. A geometry edit to a definition touches *every* instance of it
    /// too (shared geometry) — those instances appear in
    /// `instances_touched`.
    pub components_touched: Vec<ComponentId>,
    /// Guides whose geometry or visibility may have changed.
    pub guides_touched: Vec<GuideId>,
}

/// Typed failures of document operations. Nothing is repaired silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentError {
    /// The sketch handle is stale or from another Document.
    UnknownSketch,
    /// The object handle is stale, hidden, or from another Document.
    UnknownObject,
    /// The face handle is not present in the target object ( paint).
    UnknownFace,
    /// The material handle is stale or from another Document's palette.
    UnknownMaterial,
    /// The group handle is stale, hidden, or from another Document.
    UnknownGroup,
    /// The component-definition handle is stale, hidden, or from another
    /// Document.
    UnknownComponent,
    /// The instance handle is stale, hidden, or from another Document.
    UnknownInstance,
    /// The guide handle is stale, hidden, or from another Document.
    UnknownGuide,
    /// `begin_sketch_gesture` while another gesture is already open. Gestures
    /// never nest or interleave — a tool brackets exactly one commit batch.
    SketchGestureAlreadyOpen,
    /// `end_sketch_gesture` with no gesture open, or for a different sketch
    /// than the open one.
    SketchGestureNotOpen,
    /// Guide geometry is degenerate: a zero-length/non-finite direction, or a
    /// non-finite coordinate. Nothing is silently repaired or guessed.
    DegenerateGuide,
    /// `group_nodes` was called with no members.
    EmptyGroup,
    /// `transform_selection` was called with nothing to transform (no nodes
    /// and no sketches, or every listed node flattened to nothing visible).
    EmptySelection,
    /// `make_component` was called with no nodes selected.
    EmptyComponent,
    /// `make_component` was given a selection containing a component instance.
    /// Nesting a component inside a definition is deferred; the v1
    /// definition is a flat set of world objects.
    NestedComponentUnsupported,
    /// `explode_instance` was called on an instance whose pose mirrors
    /// (determinant < 0): baking a reflection into a solid would invert its
    /// winding, which `Object::apply_transform` refuses. Use
    /// `make_unique` instead, or unmirror the instance first.
    CannotExplodeReflected,
    /// `group_nodes` was given the same node twice.
    DuplicateMember,
    /// `group_nodes` members do not share a common parent — only siblings (all
    /// top-level, or all direct children of one group) can be grouped.
    MixedParents,
    /// A replacing world-context op (boolean / slice / push-through subtract)
    /// was targeted at an object that is **inside a group**. These ops consume
    /// their operand(s) and emit fresh top-level world solids; applying one to a
    /// group member would leave the parent group listing a consumed id (a
    /// tree-consistency violation). Refused loudly (DEVELOPMENT.md rule 4) rather than
    /// silently re-homed — ungroup, or enter no group context, first.
    GroupedOperand,
    /// A sketch operation (region lookup / profile tracing) failed.
    Sketch(SketchError),
    /// Extruding the region into a solid failed.
    Extrude(ExtrudeError),
    /// A boolean combine failed (non-solid operand, empty result, degenerate
    /// contact, …).
    Boolean(BooleanError),
    /// A slice failed (non-solid source, plane missing the solid, degenerate
    /// or tangent cut) —.
    Slice(SliceError),
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
            DocumentError::UnknownFace => write!(f, "no such face in the target object"),
            DocumentError::UnknownMaterial => write!(f, "no such material in this document"),
            DocumentError::UnknownGroup => write!(f, "no such group in this document"),
            DocumentError::UnknownComponent => {
                write!(f, "no such component definition in this document")
            }
            DocumentError::UnknownInstance => write!(f, "no such instance in this document"),
            DocumentError::UnknownGuide => write!(f, "no such guide in this document"),
            DocumentError::SketchGestureAlreadyOpen => {
                write!(f, "a sketch gesture is already open")
            }
            DocumentError::SketchGestureNotOpen => {
                write!(f, "no open sketch gesture for this sketch")
            }
            DocumentError::DegenerateGuide => write!(
                f,
                "guide geometry is degenerate (zero-length direction or non-finite coordinate)"
            ),
            DocumentError::EmptyGroup => write!(f, "cannot group an empty selection"),
            DocumentError::EmptySelection => {
                write!(f, "cannot transform an empty selection")
            }
            DocumentError::EmptyComponent => {
                write!(f, "cannot make a component from an empty selection")
            }
            DocumentError::NestedComponentUnsupported => {
                write!(
                    f,
                    "cannot nest a component instance inside a new definition"
                )
            }
            DocumentError::CannotExplodeReflected => {
                write!(
                    f,
                    "cannot explode a mirrored instance (would invert winding)"
                )
            }
            DocumentError::DuplicateMember => write!(f, "a node was listed twice in a group"),
            DocumentError::MixedParents => {
                write!(f, "only sibling nodes (sharing one parent) can be grouped")
            }
            DocumentError::GroupedOperand => write!(
                f,
                "cannot combine, slice, or push-through an object inside a group — ungroup it first"
            ),
            DocumentError::Sketch(e) => write!(f, "{e}"),
            DocumentError::Extrude(e) => write!(f, "{e}"),
            DocumentError::Boolean(e) => write!(f, "{e}"),
            DocumentError::Slice(e) => write!(f, "{e}"),
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
    groups: SlotMap<GroupId, GroupRecord>,
    /// Component definitions (shared geometry library entries).
    components: SlotMap<ComponentId, ComponentDef>,
    /// Component instances (tree nodes placing a definition at a pose).
    instances: SlotMap<InstanceId, InstanceRecord>,
    /// The material palette: named color/texture entries that faces
    /// reference by [`MaterialId`]. Palette additions are not individually
    /// undoable (an unreferenced material is harmless); face *assignment* is
    /// (see [`Document::paint_face`]).
    materials: SlotMap<MaterialId, Material>,
    /// Construction guides: non-solid alignment helpers (lines + points).
    guides: SlotMap<GuideId, GuideRecord>,
    /// `(sketch, region)` pairs already extruded into a solid: such a region is
    /// the bottom of its box and is no longer offered for extrusion. Keyed by
    /// sketch too because a different sketch's slotmap reuses region keys.
    consumed: BTreeSet<(SketchId, SketchRegionId)>,
    /// Sketch edges hidden because their region was extruded and they are not
    /// shared by any surviving region. Derived index — rebuilt on load.
    consumed_sketch_edges: BTreeSet<(SketchId, SketchEdgeId)>,
    /// Sketch vertices hidden because all their incident edges are in
    /// `consumed_sketch_edges`. Derived index — rebuilt on load.
    consumed_sketch_verts: BTreeSet<(SketchId, SketchVertexId)>,
    /// Sketches hidden by [`Document::delete_sketch`] (tombstone, not a real
    /// delete — the id stays valid for redo). A document-level visibility
    /// concern, not a field on [`Sketch`] itself, mirroring how object/group/
    /// instance visibility lives on their `*Record` wrappers rather than the
    /// payload type.
    hidden_sketches: BTreeSet<SketchId>,
    /// Sketches added by [`Document::add_sketch`] that no gesture has recorded
    /// into yet: the first gesture on one of these folds the sketch's creation
    /// into its undo step ([`DocAction::SketchGesture::created`]). Session-only
    /// bookkeeping — never serialized (like the undo log).
    fresh_sketches: BTreeSet<SketchId>,
    /// The open sketch gesture, if any ([`Document::begin_sketch_gesture`] …
    /// [`Document::end_sketch_gesture`]). Session-only, never serialized.
    pending_sketch_gesture: Option<PendingSketchGesture>,
    /// Tag metadata: every KNOWN tag path → hidden-by-default flag. Tags
    /// still exist implicitly by appearing on a node; this registry adds
    /// (a) tags with no content yet (an imported `.skp` layer list survives
    /// even for empty layers) and (b) the persistent hidden flag the UI
    /// seeds its visibility state from. Serialized (manifest v5). Toggling
    /// visibility is view state and NOT undoable (matches palette
    /// additions' spirit); import-time registration is undone with the
    /// import's `DocAction::Imported` step.
    tag_meta: BTreeMap<Vec<String>, bool>,
    /// USER-hidden nodes (SketchUp "Hide"): view state the user (or an
    /// import — a `.skp` hidden group/component) toggles per node,
    /// persisted at manifest v6. DISTINCT from the records' `hidden`
    /// tombstone (undone creations excluded from save). Not undoable,
    /// matching the tag-visibility registry.
    user_hidden_objects: BTreeSet<ObjectId>,
    user_hidden_groups: BTreeSet<GroupId>,
    user_hidden_instances: BTreeSet<InstanceId>,
    undo: Vec<DocAction>,
    redo: Vec<DocAction>,
    /// Torture/"paranoid" mode (docs/DEVELOPMENT.md): when on, the topology
    /// validator runs after **every** op even in release builds (where
    /// `check_invariants` / `debug_assert!` are compiled out), so a flaky op
    /// surfaces at the exact op instead of as a downstream glitch. Session-only
    /// debug state — never serialized (like the undo log), defaults off.
    torture: bool,
}

impl Document {
    /// An empty document.
    pub fn new() -> Document {
        Document::default()
    }

    /// Enables/disables torture ("paranoid") mode (docs/DEVELOPMENT.md): the
    /// always-on topology validator after every op, even in release. A debug aid
    /// — on a violation it panics at the offending op rather than committing.
    /// (The companion re-tessellation self-check lives above the kernel, in the
    /// wasm Debug-Mode wiring,  — `tessellate` may not be a kernel dep.)
    pub fn set_torture_mode(&mut self, on: bool) {
        self.torture = on;
    }

    /// Whether torture mode is enabled (see [`Document::set_torture_mode`]).
    pub fn torture_mode(&self) -> bool {
        self.torture
    }

    // ---------------------------------------------------------- persistence

    /// Serializes the whole document to a `.hew` container (HEW_FILE_FORMAT.md):
    /// a zip of `manifest.json` + per-object geometry buffers + texture assets.
    ///
    /// Pure (no I/O — DEVELOPMENT.md rule 1): bytes out, the shell writes the file.
    /// **Deterministic** — saving the same document twice yields identical bytes
    /// (golden-file contract). Persists only the live, visible state: undo/redo
    /// logs and `hidden` (undone-creation) records are dropped.
    pub fn save(&self) -> Vec<u8> {
        // ── Collect live, visible materials (in slotmap key order) ─────────
        let materials: Vec<(MaterialId, Material)> = self
            .materials
            .iter()
            .map(|(id, m)| (id, m.clone()))
            .collect();

        // ── Collect live world objects (in slotmap key order) ──────────────
        let world_objects: Vec<(ObjectId, Object)> = self
            .objects
            .iter()
            .filter(|(_, rec)| !rec.hidden && rec.is_world())
            .map(|(id, rec)| (id, rec.object.clone()))
            .collect();

        // ── Collect live definition objects (in slotmap key order) ─────────
        let def_objects: Vec<(ObjectId, Object, ComponentId)> = self
            .objects
            .iter()
            .filter(|(_, rec)| !rec.hidden && !rec.is_world())
            .filter_map(|(id, rec)| {
                if let ObjectOwner::Definition(cid) = rec.owner {
                    Some((id, rec.object.clone(), cid))
                } else {
                    None
                }
            })
            .collect();

        // ── Per-object names (world + def members), keyed by id ────────────
        let obj_names: std::collections::BTreeMap<ObjectId, Option<String>> = self
            .objects
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.name.clone()))
            .collect();

        // ── Per-object tags (world + def members), keyed by id ─────────────
        let obj_tags: std::collections::BTreeMap<ObjectId, Vec<Vec<String>>> = self
            .objects
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.tags.clone()))
            .collect();

        // ── Collect live groups (in slotmap key order) ─────────────────────
        let groups: Vec<crate::serialize::GroupSaveRow> = self
            .groups
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.members.clone(), rec.name.clone(), rec.tags.clone()))
            .collect();

        // ── Collect live components (in slotmap key order) ─────────────────
        let components: Vec<(ComponentId, Vec<ObjectId>, Option<String>)> = self
            .components
            .iter()
            .filter(|(_, c)| !c.hidden)
            .map(|(id, c)| (id, c.members.clone(), c.name.clone()))
            .collect();

        // ── Collect live instances (in slotmap key order) ──────────────────
        let instances: Vec<crate::serialize::InstanceSaveRow> = self
            .instances
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.def, rec.pose, rec.name.clone(), rec.tags.clone()))
            .collect();

        // ── Collect live sketches (in slotmap key order) ───────────────────
        let sketches: Vec<(SketchId, Sketch)> = self
            .sketches
            .iter()
            .filter(|(id, _)| !self.hidden_sketches.contains(id))
            .map(|(id, sk)| (id, sk.clone()))
            .collect();

        // ── Collect live guides (in slotmap key order) ─────────────────
        let guides: Vec<(GuideId, Guide)> = self
            .guides
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.guide))
            .collect();

        // ── Collect root nodes: top-level visible world nodes ─────────────
        // Roots = all live objects/groups/instances whose parent is None.
        // We emit objects first, then groups, then instances (same order as
        // `top_level_nodes`) to be deterministic.
        let roots: Vec<NodeId> = self.top_level_nodes();

        // ── Collect consumed (SketchId, SketchRegionId) pairs ─────────────
        // Filter to only those where both the sketch and region are live
        // (present and not hidden — a hidden sketch is dropped from `sketches`
        // above, so its consumed entries must not dangle either).
        let mut consumed: Vec<(SketchId, SketchRegionId)> = self
            .consumed
            .iter()
            .filter(|(sid, _)| {
                self.sketches.contains_key(*sid) && !self.hidden_sketches.contains(sid)
            })
            .copied()
            .collect();
        // Sort for canonical output by dense id (a BTreeSet is keyed by raw
        // slotmap id, not the dense remap — so we still sort explicitly here).
        // The slotmap keys are not directly comparable, but their iteration
        // order from the slotmaps IS stable. We derive dense indices for the
        // sort key by looking them up in the sketch slotmap iteration order.
        let sketch_dense: std::collections::BTreeMap<SketchId, usize> = sketches
            .iter()
            .enumerate()
            .map(|(i, (sid, _))| (*sid, i))
            .collect();
        consumed.sort_by_key(|(sid, rid)| {
            let sdense = sketch_dense.get(sid).copied().unwrap_or(usize::MAX);
            // dense region id = iteration order index in the sketch's region slotmap
            let rdense = sketches
                .iter()
                .find(|(s, _)| s == sid)
                .map(|(_, sk)| {
                    sk.regions()
                        .keys()
                        .enumerate()
                        .find(|(_, r)| r == rid)
                        .map(|(i, _)| i)
                        .unwrap_or(usize::MAX)
                })
                .unwrap_or(usize::MAX);
            (sdense, rdense)
        });

        // ── Tag metadata registry (manifest v5) ────────────────────────────
        let tag_meta: Vec<(Vec<String>, bool)> =
            self.tag_meta.iter().map(|(p, &h)| (p.clone(), h)).collect();

        encode_document(DocSaveData {
            materials,
            world_objects,
            def_objects,
            groups,
            components,
            instances,
            sketches,
            guides,
            roots,
            consumed,
            obj_names,
            obj_tags,
            tag_meta,
            obj_hidden: self.user_hidden_objects.clone(),
            group_hidden: self.user_hidden_groups.clone(),
            instance_hidden: self.user_hidden_instances.clone(),
        })
    }

    /// A canonical, deterministic digest of the document's live state ( /
    /// docs/DEVELOPMENT.md). The single oracle for the Road-to-Reliable phase:
    /// record/replay asserts against it, the diagnostic log stamps every op with
    /// it, and the determinism guard compares it.
    ///
    /// Defined as a hash of the canonical [`save`] bytes, which are themselves
    /// byte-for-byte deterministic, so two documents share a `state_hash` iff
    /// they serialize identically — i.e. iff their live, visible state matches
    /// (undo/redo history and undone-creation records are excluded, exactly as
    /// `save` excludes them).
    ///
    /// The digest is [FNV-1a/64] — fixed, zero-dependency, and stable across
    /// Rust toolchain versions (unlike `DefaultHasher`/SipHash), so a hash frozen
    /// into a committed replay fixture stays valid forever. 64 bits is ample for
    /// an equality oracle; this is never a security or anti-collision primitive.
    ///
    /// [`save`]: Document::save
    /// [FNV-1a/64]: https://en.wikipedia.org/wiki/Fowler%E2%80%93Noll%E2%80%93Vo_hash_function
    pub fn state_hash(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = FNV_OFFSET;
        for byte in self.save() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }

    /// Reconstructs a document from a `.hew` container produced by [`save`].
    /// Validates every rebuilt object (rule 4: reject, never repair); a corrupt
    /// or tampered file yields a typed [`LoadError`]. The returned document has
    /// an empty undo stack.
    ///
    /// [`save`]: Document::save
    pub fn load(bytes: &[u8]) -> Result<Document, LoadError> {
        let raw = decode_document_raw(bytes)?;

        let mut doc = Document::new();

        // ── 1. Insert materials → build dense→MaterialId map ──────────────
        let mat_ids: Vec<MaterialId> = raw
            .materials
            .into_iter()
            .map(|m| doc.materials.insert(m))
            .collect();
        let dense_to_mat = |dense: u32| -> Option<MaterialId> {
            if dense == crate::serialize::NO_MATERIAL {
                None
            } else {
                mat_ids.get(dense as usize).copied()
            }
        };

        // ── 2. Decode objects (with live material closure) ─────────────────
        // Dense object ids are used in the manifest. We decode all objects
        // in dense-id order, then insert them into the document.
        let obj_count = raw.geom_buffers.len();
        let mut dense_obj_ids: Vec<ObjectId> = Vec::with_capacity(obj_count);

        for (i, buf) in raw.geom_buffers.iter().enumerate() {
            let mut obj = Object::decode(buf, &dense_to_mat).map_err(LoadError::Geometry)?;

            // Patch base material from manifest (the geometry buffer also carries
            // it, but we restore it from the manifest to match exactly).
            obj.default_material = raw
                .obj_base_materials
                .get(i)
                .copied()
                .flatten()
                .and_then(dense_to_mat);

            // Determine ownership: is this a definition member?
            let owner = if let Some(comp_dense) = raw.def_membership.get(i).copied().flatten() {
                // We don't have the ComponentId yet — we'll patch it after
                // inserting components. Use a placeholder World owner for now.
                // We'll re-assign below.
                let _ = comp_dense;
                ObjectOwner::World { parent: None }
            } else {
                ObjectOwner::World { parent: None }
            };

            let oid = doc.objects.insert(ObjectRecord {
                object: obj,
                history: History::new(),
                hidden: false,
                owner,
                name: raw.obj_names.get(i).cloned().flatten(),
                tags: raw.obj_tags.get(i).cloned().unwrap_or_default(),
            });
            dense_obj_ids.push(oid);
        }

        // ── 3. Insert sketches → build dense→SketchId map ─────────────────
        let sketch_ids: Vec<SketchId> = raw
            .sketches
            .into_iter()
            .map(|sk| doc.sketches.insert(sk))
            .collect();

        // ── 3b. Insert guides — order among independent collections
        // doesn't matter; after sketches is fine, and is deterministic since
        // `raw.guides` is already in dense (save-time) order.
        for guide in raw.guides {
            doc.guides.insert(GuideRecord {
                guide,
                hidden: false,
            });
        }

        // ── 4. Insert components → build dense→ComponentId map ────────────
        // Each component's members are dense object ids → now live ObjectIds.
        let mut comp_ids: Vec<ComponentId> = Vec::with_capacity(raw.components.len());
        for (ci, member_dense_ids) in raw.components.iter().enumerate() {
            let members: Vec<ObjectId> = member_dense_ids
                .iter()
                .map(|&di| {
                    dense_obj_ids.get(di as usize).copied().ok_or_else(|| {
                        LoadError::DanglingReference {
                            what: format!("component member object dense id {di} out of range"),
                        }
                    })
                })
                .collect::<Result<_, _>>()?;
            let cid = doc.components.insert(ComponentDef {
                members: members.clone(),
                hidden: false,
                name: raw.component_names.get(ci).cloned().flatten(),
            });
            comp_ids.push(cid);
            // Re-assign ownership for these objects.
            for oid in members {
                doc.objects[oid].owner = ObjectOwner::Definition(cid);
            }
        }

        // ── 5. Insert instances → build dense→InstanceId map ─────────────
        let mut inst_ids: Vec<InstanceId> = Vec::with_capacity(raw.instances.len());
        for (ii, (comp_dense, pose)) in raw.instances.iter().enumerate() {
            let cid = *comp_ids.get(*comp_dense as usize).ok_or_else(|| {
                LoadError::DanglingReference {
                    what: format!("instance def component dense id {comp_dense} out of range"),
                }
            })?;
            let iid = doc.instances.insert(InstanceRecord {
                def: cid,
                pose: *pose,
                parent: None,
                hidden: false,
                name: raw.instance_names.get(ii).cloned().flatten(),
                tags: raw.instance_tags.get(ii).cloned().unwrap_or_default(),
            });
            inst_ids.push(iid);
        }

        // ── 6. Insert groups → build dense→GroupId map ────────────────────
        // Groups may reference other groups (nesting), so insert all first,
        // then patch members.
        let mut grp_ids: Vec<GroupId> = Vec::with_capacity(raw.groups.len());
        for gi in 0..raw.groups.len() {
            let gid = doc.groups.insert(GroupRecord {
                members: Vec::new(),
                parent: None,
                hidden: false,
                name: raw.group_names.get(gi).cloned().flatten(),
                tags: raw.group_tags.get(gi).cloned().unwrap_or_default(),
            });
            grp_ids.push(gid);
        }

        // Helper to resolve a NodeRefDto to a NodeId.
        let resolve_node = |dto: &NodeRefDto| -> Result<NodeId, LoadError> {
            match dto.kind.as_str() {
                "object" => {
                    let oid = dense_obj_ids.get(dto.id as usize).copied().ok_or_else(|| {
                        LoadError::DanglingReference {
                            what: format!("node ref object id {} out of range", dto.id),
                        }
                    })?;
                    Ok(NodeId::Object(oid))
                }
                "group" => {
                    let gid = grp_ids.get(dto.id as usize).copied().ok_or_else(|| {
                        LoadError::DanglingReference {
                            what: format!("node ref group id {} out of range", dto.id),
                        }
                    })?;
                    Ok(NodeId::Group(gid))
                }
                "instance" => {
                    let iid = inst_ids.get(dto.id as usize).copied().ok_or_else(|| {
                        LoadError::DanglingReference {
                            what: format!("node ref instance id {} out of range", dto.id),
                        }
                    })?;
                    Ok(NodeId::Instance(iid))
                }
                _ => Err(LoadError::MalformedManifest {
                    what: format!("unknown node kind '{}'", dto.kind),
                }),
            }
        };

        // Patch group members and set up parent pointers.
        for (i, member_dtos) in raw.groups.iter().enumerate() {
            let gid = grp_ids[i];
            let members: Vec<NodeId> = member_dtos
                .iter()
                .map(resolve_node)
                .collect::<Result<_, _>>()?;
            doc.groups[gid].members = members.clone();
            // Set child → parent pointers.
            for m in &members {
                match m {
                    NodeId::Object(oid) => {
                        doc.objects[*oid].owner = match doc.objects[*oid].owner {
                            ObjectOwner::World { .. } => ObjectOwner::World { parent: Some(gid) },
                            def @ ObjectOwner::Definition(_) => def,
                        };
                    }
                    NodeId::Group(child_gid) => {
                        doc.groups[*child_gid].parent = Some(gid);
                    }
                    NodeId::Instance(iid) => {
                        doc.instances[*iid].parent = Some(gid);
                    }
                }
            }
        }

        // ── 7. Restore consumed set ────────────────────────────────────────
        for [dense_sid, dense_rid] in &raw.consumed {
            let sid = *sketch_ids.get(*dense_sid as usize).ok_or_else(|| {
                LoadError::DanglingReference {
                    what: format!("consumed sketch dense id {dense_sid} out of range"),
                }
            })?;
            // dense_rid is the slotmap-iteration-order index of the region
            // within the sketch. We need to map it to a live SketchRegionId.
            let sk = &doc.sketches[sid];
            let rid = sk
                .regions()
                .keys()
                .nth(*dense_rid as usize)
                .ok_or_else(|| LoadError::DanglingReference {
                    what: format!(
                        "consumed region dense id {dense_rid} in sketch {dense_sid} out of range"
                    ),
                })?;
            doc.consumed.insert((sid, rid));
        }

        // Rebuild the derived tombstone index for consumed sketch edges/verts
        // from each sketch's FULL consumed set (the rule is order-free — see
        // `Sketch::consumed_tombstones` — so one evaluation per sketch lands
        // on the same answer the original extrude sequence produced). Group
        // first to avoid a simultaneous borrow of `doc.consumed` and
        // `doc.sketches`.
        let mut consumed_by_sketch: std::collections::BTreeMap<
            SketchId,
            std::collections::BTreeSet<SketchRegionId>,
        > = std::collections::BTreeMap::new();
        for &(sid, rid) in &doc.consumed {
            consumed_by_sketch.entry(sid).or_default().insert(rid);
        }
        for (sid, regions) in consumed_by_sketch {
            let (edges, verts) = doc.sketches[sid].consumed_tombstones(&regions);
            for e in edges {
                doc.consumed_sketch_edges.insert((sid, e));
            }
            for v in verts {
                doc.consumed_sketch_verts.insert((sid, v));
            }
        }

        // ── Tag metadata registry (manifest v5; empty in v1–v4 files) ─────
        for (path, hidden) in raw.tag_meta {
            if !path.is_empty() {
                doc.tag_meta.insert(path, hidden);
            }
        }

        // ── USER-hidden view state (manifest v6; empty pre-v6) ────────────
        for (i, &h) in raw.obj_hidden.iter().enumerate() {
            if h && let Some(&oid) = dense_obj_ids.get(i) {
                doc.user_hidden_objects.insert(oid);
            }
        }
        for (i, &h) in raw.group_hidden.iter().enumerate() {
            if h && let Some(&gid) = grp_ids.get(i) {
                doc.user_hidden_groups.insert(gid);
            }
        }
        for (i, &h) in raw.instance_hidden.iter().enumerate() {
            if h && let Some(&iid) = inst_ids.get(i) {
                doc.user_hidden_instances.insert(iid);
            }
        }

        // Undo/redo stacks are empty by construction (Document::new() gives empty).
        Ok(doc)
    }

    // -------------------------------------------------------------- ingest

    /// Merge an imported scene (COLLADA, etc.) into this document as new
    /// world-tree nodes. Mirrors `load`'s insertion cascade but is ADDITIVE —
    /// existing entities are untouched.
    ///
    /// The entire import is atomic and undoable as ONE step
    /// (`DocAction::Imported`): undo hides every created node/object, redo
    /// unhides (hide-not-delete; ids stable). Added palette materials are not
    /// individually undone (matches `add_material`).
    ///
    /// Per-mesh `from_polygons_with_materials` failures are recorded in the
    /// returned `ImportReport.skipped` and the mesh is dropped — never
    /// repaired (DEVELOPMENT.md rule 4). A scene that produces zero objects still
    /// returns `Ok` with an empty report.
    pub fn ingest(
        &mut self,
        scene: ImportScene,
        textures_missing: Vec<String>,
    ) -> Result<(ImportReport, DocChange), DocumentError> {
        use crate::serialize::NO_MATERIAL;

        // ── 1. Insert materials → build dense→MaterialId map ──────────────
        let mat_ids: Vec<MaterialId> = scene
            .materials
            .into_iter()
            .map(|m| self.materials.insert(m))
            .collect();
        let dense_to_mat = |dense: u32| -> Option<MaterialId> {
            if dense == NO_MATERIAL {
                None
            } else {
                mat_ids.get(dense as usize).copied()
            }
        };

        // Tracking collections for the DocAction + DocChange.
        let mut all_objects: Vec<ObjectId> = Vec::new();
        let mut all_components: Vec<ComponentId> = Vec::new();
        let mut all_instances: Vec<InstanceId> = Vec::new();
        let mut all_groups: Vec<GroupId> = Vec::new();
        let mut top_roots: Vec<NodeId> = Vec::new();

        let mut watertight_count = 0usize;
        let mut leaky_count = 0usize;
        let mut skipped: Vec<SkippedMesh> = Vec::new();

        // ── 2. Build component definitions ────────────────────────────────
        // Map dae-import def index → ComponentId (or None if all meshes failed)
        let mut def_cid: Vec<Option<ComponentId>> = Vec::with_capacity(scene.defs.len());
        for def_recipe in scene.defs {
            // Pre-allocate the component so members can reference it.
            let cid = self.components.insert(ComponentDef {
                members: Vec::new(),
                hidden: false,
                name: def_recipe.name,
            });
            let mut def_members: Vec<ObjectId> = Vec::new();
            for mesh in def_recipe.meshes {
                if let Some(oid) = ingest_build_mesh(
                    self,
                    mesh,
                    ObjectOwner::Definition(cid),
                    &mut all_objects,
                    &mut watertight_count,
                    &mut leaky_count,
                    &mut skipped,
                    &dense_to_mat,
                ) {
                    def_members.push(oid);
                }
            }
            if def_members.is_empty() {
                // All meshes rejected → remove the placeholder def.
                self.components.remove(cid);
                def_cid.push(None);
            } else {
                self.components[cid].members = def_members;
                all_components.push(cid);
                def_cid.push(Some(cid));
            }
        }

        // ── 3. Recursively build the scene tree ───────────────────────────
        for root_node in scene.roots {
            if let Some(nid) = ingest_build_node(
                self,
                root_node,
                None,
                &def_cid,
                &mut all_objects,
                &mut all_instances,
                &mut all_groups,
                &mut watertight_count,
                &mut leaky_count,
                &mut skipped,
                &dense_to_mat,
            ) {
                top_roots.push(nid);
            }
        }

        let objects_created = all_objects.len();

        // ── 4. Construction guides ─────────────────────────────────
        // Inserted directly (not via add_guide_line/point, which would push
        // their own undo steps): the guides belong to the single Imported
        // action below. Degenerate inputs are skipped and reported, never
        // fixed up (DEVELOPMENT.md rule 4).
        let mut all_guides: Vec<GuideId> = Vec::new();
        for g in scene.guides {
            match g {
                crate::import::ImportGuide::Line { origin, direction } => {
                    let dir = direction.normalized().ok();
                    match dir {
                        Some(d) if point_is_finite(origin) && vec_is_finite(d) => {
                            all_guides.push(self.guides.insert(GuideRecord {
                                guide: Guide::Line {
                                    origin,
                                    direction: d,
                                },
                                hidden: false,
                            }));
                        }
                        _ => skipped.push(SkippedMesh {
                            name: "construction guide".into(),
                            reason: "degenerate guide line (non-finite origin or zero direction)"
                                .into(),
                        }),
                    }
                }
                crate::import::ImportGuide::Point { position } => {
                    if point_is_finite(position) {
                        all_guides.push(self.guides.insert(GuideRecord {
                            guide: Guide::Point { position },
                            hidden: false,
                        }));
                    } else {
                        skipped.push(SkippedMesh {
                            name: "construction guide".into(),
                            reason: "degenerate guide point (non-finite position)".into(),
                        });
                    }
                }
            }
        }

        // ── 4b. Register the source document's declared tag list ──────────
        // Only NEWLY registered paths are recorded (and undone): a tag that
        // already exists keeps its current hidden flag — an import must not
        // flip visibility the user already chose.
        let mut tags_added: Vec<(Vec<String>, bool)> = Vec::new();
        for t in scene.tags {
            if !t.path.is_empty() && !self.tag_meta.contains_key(&t.path) {
                self.tag_meta.insert(t.path.clone(), t.hidden);
                tags_added.push((t.path, t.hidden));
            }
        }

        // ── 5. Push action + clear redo ───────────────────────────────────
        self.undo.push(DocAction::Imported {
            roots: top_roots.clone(),
            objects: all_objects.clone(),
            components: all_components.clone(),
            instances: all_instances.clone(),
            groups: all_groups.clone(),
            guides: all_guides.clone(),
            tags: tags_added,
        });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: all_objects,
            sketches_touched: Vec::new(),
            groups_touched: all_groups,
            instances_touched: all_instances,
            components_touched: all_components,
            guides_touched: all_guides,
        };

        let report = ImportReport {
            objects_created,
            watertight: watertight_count,
            leaky: leaky_count,
            skipped,
            textures_missing,
        };

        Ok((report, change))
    }

    // --------------------------------------------------------------- sketches

    /// Adds a fresh, empty sketch on `plane` and returns its handle. **Additive**
    /// — existing sketches are untouched, so independent coplanar shapes can
    /// coexist. Plane choice (ground or a face) is the caller's concern.
    pub fn add_sketch(&mut self, plane: Plane) -> SketchId {
        let id = self.sketches.insert(Sketch::on_plane(plane));
        // Not undoable on its own: an empty sketch draws nothing. The first
        // gesture recorded into it folds the creation into its undo step.
        self.fresh_sketches.insert(id);
        id
    }

    /// Opens a sketch-drawing gesture on `sketch`: snapshots its contents so
    /// [`Document::end_sketch_gesture`] can record the whole edit batch — a
    /// full rectangle/circle/arc, not each sticky segment — as ONE undo step.
    /// The first gesture on a freshly-added sketch also folds the sketch's
    /// creation into that step, so undoing it removes the sketch from view.
    ///
    /// Between begin and end, callers mutate via [`Document::sketch_mut`] as
    /// usual, and must not run other document ops (extrude, delete, undo): a
    /// gesture is a tight bracket around one tool commit.
    ///
    /// # Errors
    /// - [`DocumentError::SketchGestureAlreadyOpen`] — gestures never nest or
    ///   interleave.
    /// - [`DocumentError::UnknownSketch`] — stale or hidden handle.
    ///
    /// On `Err` the document is untouched.
    pub fn begin_sketch_gesture(&mut self, sketch: SketchId) -> Result<(), DocumentError> {
        if self.pending_sketch_gesture.is_some() {
            return Err(DocumentError::SketchGestureAlreadyOpen);
        }
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let created = self.fresh_sketches.contains(&sketch) && s.edges().is_empty();
        self.pending_sketch_gesture = Some(PendingSketchGesture {
            sketch,
            before: Box::new(s.clone()),
            created,
        });
        Ok(())
    }

    /// Closes the open gesture on `sketch`. If the sketch changed since
    /// [`Document::begin_sketch_gesture`], pushes one
    /// [`DocAction::SketchGesture`] and clears redo; an unchanged gesture
    /// records nothing (and stays undo-invisible). Either way the gesture is
    /// closed on return — including the `Err` paths.
    ///
    /// # Errors
    /// - [`DocumentError::SketchGestureNotOpen`] — no gesture is open, or the
    ///   open one is for a different sketch.
    /// - [`DocumentError::UnknownSketch`] — the sketch vanished mid-gesture.
    pub fn end_sketch_gesture(&mut self, sketch: SketchId) -> Result<DocChange, DocumentError> {
        match &self.pending_sketch_gesture {
            Some(p) if p.sketch == sketch => {}
            _ => return Err(DocumentError::SketchGestureNotOpen),
        }
        let pending = self
            .pending_sketch_gesture
            .take()
            .expect("matched Some above");
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        let Some(s) = self.sketches.get(sketch) else {
            return Err(DocumentError::UnknownSketch);
        };
        if *s == *pending.before {
            return Ok(DocChange::default());
        }
        self.undo.push(DocAction::SketchGesture {
            sketch,
            before: pending.before,
            after: Box::new(s.clone()),
            created: pending.created,
        });
        self.redo.clear();
        self.fresh_sketches.remove(&sketch);
        self.debug_validate();
        Ok(DocChange {
            sketches_touched: vec![sketch],
            ..Default::default()
        })
    }

    /// Drops the open gesture (if any) without recording anything — the
    /// tool-cancel path. Returns whether a gesture was open. Any mutations
    /// made inside the abandoned bracket stay in the sketch but out of the
    /// undo log; cancel-before-mutate is the caller's contract.
    pub fn cancel_sketch_gesture(&mut self) -> bool {
        self.pending_sketch_gesture.take().is_some()
    }

    /// A sketch by handle, or `None` if stale or hidden (deleted).
    pub fn sketch(&self, id: SketchId) -> Option<&Sketch> {
        if self.hidden_sketches.contains(&id) {
            return None;
        }
        self.sketches.get(id)
    }

    /// A mutable sketch by handle, or `None` if stale or hidden (deleted).
    ///
    /// Sketch edits do not flow through the document undo log (sketch-level undo
    /// is a later milestone); they are surfaced to the caller via the returned
    /// handle and reconciled through [`Document::sketch`] reads.
    pub fn sketch_mut(&mut self, id: SketchId) -> Option<&mut Sketch> {
        if self.hidden_sketches.contains(&id) {
            return None;
        }
        self.sketches.get_mut(id)
    }

    /// All sketch handles, in unspecified but stable order. Excludes sketches
    /// hidden by [`Document::delete_sketch`] (D-pending) as well as sketches
    /// fully consumed by extrusion.
    pub fn sketch_ids(&self) -> Vec<SketchId> {
        self.sketches
            .keys()
            .filter(|&s| !self.hidden_sketches.contains(&s) && !self.is_sketch_fully_consumed(s))
            .collect()
    }

    /// Delete one free-standing sketch (hide-not-delete; the id stays valid
    /// for redo) — whole-sketch granularity: every edge/vertex in it goes with
    /// it. Stale or already-hidden id → [`DocumentError::UnknownSketch`].
    /// Undoable ([`DocAction::DeletedSketch`]). Mirrors [`Document::delete_guide`].
    ///
    /// # Errors
    /// - [`DocumentError::UnknownSketch`] — stale, hidden, or from another
    ///   Document.
    ///
    /// On `Err` the document is untouched.
    pub fn delete_sketch(&mut self, sketch: SketchId) -> Result<DocChange, DocumentError> {
        if !self.sketches.contains_key(sketch) || self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        self.hidden_sketches.insert(sketch);
        self.undo.push(DocAction::DeletedSketch { sketch });
        self.redo.clear();
        self.debug_validate();
        Ok(DocChange {
            sketches_touched: vec![sketch],
            ..Default::default()
        })
    }

    /// Whether every edge of `sketch` has been consumed by an extrusion — i.e.
    /// the sketch has been wholly subsumed into one or more solids and no longer
    /// exists as an actionable sketch (it draws nothing and offers no snap or
    /// extrudable region). Such a sketch is dropped from [`Document::sketch_ids`]
    /// so it vanishes from the UI the moment its last region is extruded; undo
    /// un-consumes the edges (the consumed set is undo-aware) and it reappears.
    /// An edge-less (freshly created, still-empty) sketch is NOT consumed.
    fn is_sketch_fully_consumed(&self, sketch: SketchId) -> bool {
        let Some(sk) = self.sketches.get(sketch) else {
            return false;
        };
        let edges = sk.edges();
        !edges.is_empty()
            && edges
                .keys()
                .all(|e| self.consumed_sketch_edges.contains(&(sketch, e)))
    }

    /// Whether `region` of `sketch` has already been extruded into a solid (and
    /// is therefore no longer extrudable).
    pub fn is_region_consumed(&self, sketch: SketchId, region: SketchRegionId) -> bool {
        self.consumed.contains(&(sketch, region))
    }

    /// True if this sketch edge has been consumed by an extrusion (hidden from
    /// rendering — it became part of a solid's base).
    pub fn is_sketch_edge_consumed(&self, sketch: SketchId, edge: SketchEdgeId) -> bool {
        self.consumed_sketch_edges.contains(&(sketch, edge))
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

    // -------------------------------------------------------------- materials

    /// Add `material` to the palette and return its handle. Additive and
    /// **not** undoable on its own — only face assignment ([`paint_face`]) is.
    ///
    /// [`paint_face`]: Document::paint_face
    pub fn add_material(&mut self, material: Material) -> MaterialId {
        self.materials.insert(material)
    }

    /// Set an existing palette material's opacity (alpha channel of its
    /// color; 0–255, 255 = opaque) — applies uniformly whether the material
    /// is a flat color or textured, since `color` also modulates a texture.
    /// Undoable, recording [`DocAction::SetMaterialAlpha`]: unlike
    /// [`add_material`], this mutates a palette entry that may already be in
    /// use, so it's a visible change like any other.
    ///
    /// Returns an empty [`DocChange`]: alpha is resolved live from the
    /// palette at render time (see `MaterialJs::a` / the wasm-api), not baked
    /// into tessellated geometry the way a face's material *assignment* is,
    /// so no object/instance needs its render or inference cache invalidated.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownMaterial`] — `id` is not in the palette.
    ///
    /// [`add_material`]: Document::add_material
    pub fn set_material_alpha(
        &mut self,
        id: MaterialId,
        alpha: u8,
    ) -> Result<DocChange, DocumentError> {
        let mat = self
            .materials
            .get_mut(id)
            .ok_or(DocumentError::UnknownMaterial)?;
        let prev = mat.color.a;
        if prev == alpha {
            return Ok(DocChange::default());
        }
        mat.color.a = alpha;
        self.undo.push(DocAction::SetMaterialAlpha {
            material: id,
            prev,
            next: alpha,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(DocChange::default())
    }

    /// A palette material by handle, or `None` if stale.
    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id)
    }

    /// All palette material handles, in unspecified but stable order.
    pub fn material_ids(&self) -> Vec<MaterialId> {
        self.materials.keys().collect()
    }

    /// The whole material palette, for the tessellator to resolve face
    /// colors/textures into render buffers.
    pub fn materials(&self) -> &crate::material::MaterialPalette {
        &self.materials
    }

    /// The material currently on `face` of `object` (`None` = default), or
    /// `None` if the object/face is unknown. Read path for the renderer/shim.
    pub fn face_material(&self, object: ObjectId, face: FaceId) -> Option<MaterialId> {
        self.objects
            .get(object)
            .filter(|r| !r.hidden)
            .and_then(|r| r.object.faces().get(face))
            .and_then(|f| f.material)
    }

    /// Paint `face` of `object` with `material` (`None` resets it to the default
    /// material), recording an undoable [`DocAction::PaintFace`].
    ///
    /// Works on world objects **and** component-definition members alike;
    /// painting a definition member repaints the face in every instance of that
    /// definition (shared geometry). Assignment is non-topological — it
    /// bypasses the per-Object [`History`] and never affects watertightness.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] — stale or hidden object.
    /// - [`DocumentError::UnknownFace`] — `face` is not in the object.
    /// - [`DocumentError::UnknownMaterial`] — `Some(id)` is not in the palette.
    ///
    /// On `Err` the document is untouched.
    pub fn paint_face(
        &mut self,
        object: ObjectId,
        face: FaceId,
        material: Option<MaterialId>,
    ) -> Result<DocChange, DocumentError> {
        if let Some(id) = material
            && !self.materials.contains_key(id)
        {
            return Err(DocumentError::UnknownMaterial);
        }
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        let f = match rec.object.faces.get_mut(face) {
            Some(f) => f,
            None => return Err(DocumentError::UnknownFace),
        };
        let prev = f.material;
        f.material = material;
        self.undo.push(DocAction::PaintFace {
            object,
            face,
            prev,
            next: material,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(self.paint_change(object))
    }

    /// Set `object`'s **base material** (`None` clears it to the renderer's
    /// default), recording an undoable [`DocAction::SetObjectMaterial`] (
    /// follow-up). A face with no own material resolves to the base, so the
    /// solid — and any faces grown from it by extrude/boolean — render
    /// consistently. Explicitly painted faces still override the base.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] — stale or hidden object.
    /// - [`DocumentError::UnknownMaterial`] — `Some(id)` is not in the palette.
    ///
    /// On `Err` the document is untouched.
    pub fn set_object_material(
        &mut self,
        object: ObjectId,
        material: Option<MaterialId>,
    ) -> Result<DocChange, DocumentError> {
        if let Some(id) = material
            && !self.materials.contains_key(id)
        {
            return Err(DocumentError::UnknownMaterial);
        }
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        let prev = rec.object.default_material;
        rec.object.default_material = material;
        self.undo.push(DocAction::SetObjectMaterial {
            object,
            prev,
            next: material,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(self.paint_change(object))
    }

    /// The [`DocChange`] for a paint of `object`: the object itself, plus — if it
    /// is a definition member — its component and every instance of it, since the
    /// repaint is seen through all of them (shared geometry).
    fn paint_change(&self, object: ObjectId) -> DocChange {
        match self.objects.get(object).map(|r| r.owner) {
            Some(ObjectOwner::Definition(component)) => DocChange {
                objects_touched: vec![object],
                components_touched: vec![component],
                instances_touched: self.instances_of(component),
                ..Default::default()
            },
            _ => DocChange {
                objects_touched: vec![object],
                ..Default::default()
            },
        }
    }

    // ------------------------------------------------------------------ guides

    /// Add an infinite construction guide line. `direction` is normalized; a
    /// zero-length/non-finite direction or non-finite `origin` →
    /// [`DocumentError::DegenerateGuide`]. Undoable ([`DocAction::CreatedGuide`]);
    /// the returned [`GuideId`] is stable across undo/redo. Clears the redo
    /// stack.
    ///
    /// # Errors
    /// - [`DocumentError::DegenerateGuide`] — non-finite `origin`, or `direction`
    ///   that fails to normalize (zero-length/non-finite).
    ///
    /// On `Err` the document is untouched.
    pub fn add_guide_line(
        &mut self,
        origin: Point3,
        direction: Vec3,
    ) -> Result<GuideId, DocumentError> {
        if !point_is_finite(origin) || !vec_is_finite(direction) {
            return Err(DocumentError::DegenerateGuide);
        }
        let direction = direction
            .normalized()
            .map_err(|_: MathError| DocumentError::DegenerateGuide)?;
        let guide = Guide::Line { origin, direction };
        let id = self.guides.insert(GuideRecord {
            guide,
            hidden: false,
        });
        self.undo.push(DocAction::CreatedGuide { guide: id });
        self.redo.clear();
        self.debug_validate();
        Ok(id)
    }

    /// Add a construction guide point. Non-finite coordinate →
    /// [`DocumentError::DegenerateGuide`]. Undoable
    /// ([`DocAction::CreatedGuide`]).
    ///
    /// # Errors
    /// - [`DocumentError::DegenerateGuide`] — non-finite `position`.
    ///
    /// On `Err` the document is untouched.
    pub fn add_guide_point(&mut self, position: Point3) -> Result<GuideId, DocumentError> {
        if !point_is_finite(position) {
            return Err(DocumentError::DegenerateGuide);
        }
        let guide = Guide::Point { position };
        let id = self.guides.insert(GuideRecord {
            guide,
            hidden: false,
        });
        self.undo.push(DocAction::CreatedGuide { guide: id });
        self.redo.clear();
        self.debug_validate();
        Ok(id)
    }

    /// Delete one guide (hide-not-delete; the id stays valid for redo). Stale
    /// or already-hidden id → [`DocumentError::UnknownGuide`]. Undoable
    /// ([`DocAction::DeletedGuide`]).
    ///
    /// # Errors
    /// - [`DocumentError::UnknownGuide`] — stale, hidden, or from another
    ///   Document.
    ///
    /// On `Err` the document is untouched.
    pub fn delete_guide(&mut self, guide: GuideId) -> Result<DocChange, DocumentError> {
        match self.guides.get_mut(guide) {
            Some(rec) if !rec.hidden => rec.hidden = true,
            _ => return Err(DocumentError::UnknownGuide),
        }
        self.undo.push(DocAction::DeletedGuide { guide });
        self.redo.clear();
        self.debug_validate();
        Ok(DocChange {
            guides_touched: vec![guide],
            ..Default::default()
        })
    }

    /// Delete every currently-visible guide in one undoable step (Edit ▸
    /// Delete Guide Lines). No visible guides → `Ok` with an empty [`DocChange`]
    /// and NO undo entry pushed. Otherwise pushes one
    /// [`DocAction::DeletedGuides`].
    pub fn delete_all_guides(&mut self) -> Result<DocChange, DocumentError> {
        let live: Vec<GuideId> = self.guide_ids();
        if live.is_empty() {
            return Ok(DocChange::default());
        }
        for &id in &live {
            self.guides[id].hidden = true;
        }
        self.undo.push(DocAction::DeletedGuides {
            guides: live.clone(),
        });
        self.redo.clear();
        self.debug_validate();
        Ok(DocChange {
            guides_touched: live,
            ..Default::default()
        })
    }

    /// Live (visible) guide ids in slotmap key order (deterministic).
    pub fn guide_ids(&self) -> Vec<GuideId> {
        self.guides
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, _)| id)
            .collect()
    }

    /// The guide behind a live (visible) id, else `None`.
    pub fn guide(&self, id: GuideId) -> Option<&Guide> {
        self.guides
            .get(id)
            .filter(|rec| !rec.hidden)
            .map(|rec| &rec.guide)
    }

    // -------------------------------------------------- node metadata ops / getters

    /// Returns the tag paths of a visible tree node (`&[]` if stale or hidden).
    ///
    /// Each path is a root-first list of segments (e.g. `["Structure","Roof"]`).
    pub fn node_tags(&self, node: NodeId) -> &[Vec<String>] {
        match node {
            NodeId::Object(id) => self
                .objects
                .get(id)
                .filter(|r| !r.hidden)
                .map_or(&[], |r| r.tags.as_slice()),
            NodeId::Group(id) => self
                .groups
                .get(id)
                .filter(|r| !r.hidden)
                .map_or(&[], |r| r.tags.as_slice()),
            NodeId::Instance(id) => self
                .instances
                .get(id)
                .filter(|r| !r.hidden)
                .map_or(&[], |r| r.tags.as_slice()),
        }
    }

    /// The tag metadata registry: every KNOWN tag path with its
    /// hidden-by-default flag, sorted by path. Tags carried only by nodes
    /// (never registered) are NOT listed here — the UI unions this registry
    /// with the per-node tags it already collects.
    pub fn tag_meta(&self) -> impl Iterator<Item = (&[String], bool)> {
        self.tag_meta
            .iter()
            .map(|(path, &hidden)| (path.as_slice(), hidden))
    }

    /// The hidden-by-default flag for `path` (`false` for unregistered tags).
    pub fn tag_hidden(&self, path: &[String]) -> bool {
        self.tag_meta.get(path).copied().unwrap_or(false)
    }

    /// Sets (registering if unknown) a tag's hidden-by-default flag.
    ///
    /// View state, deliberately NOT undoable — matching how palette
    /// additions escape the undo log. Serialized with the document
    /// (manifest v5) so a hidden `.skp` layer stays hidden across
    /// save/load.
    pub fn set_tag_hidden(&mut self, path: Vec<String>, hidden: bool) {
        if path.is_empty() {
            return;
        }
        self.tag_meta.insert(path, hidden);
    }

    /// Whether a node is USER-hidden (view state; persisted, manifest v6).
    pub fn node_user_hidden(&self, node: NodeId) -> bool {
        match node {
            NodeId::Object(id) => self.user_hidden_objects.contains(&id),
            NodeId::Group(id) => self.user_hidden_groups.contains(&id),
            NodeId::Instance(id) => self.user_hidden_instances.contains(&id),
        }
    }

    /// Sets a node's USER-hidden flag (view state, deliberately NOT
    /// undoable — matching [`Document::set_tag_hidden`]). Stale ids are
    /// ignored.
    pub fn set_node_user_hidden(&mut self, node: NodeId, hidden: bool) {
        match node {
            NodeId::Object(id) => {
                if hidden {
                    self.user_hidden_objects.insert(id);
                } else {
                    self.user_hidden_objects.remove(&id);
                }
            }
            NodeId::Group(id) => {
                if hidden {
                    self.user_hidden_groups.insert(id);
                } else {
                    self.user_hidden_groups.remove(&id);
                }
            }
            NodeId::Instance(id) => {
                if hidden {
                    self.user_hidden_instances.insert(id);
                } else {
                    self.user_hidden_instances.remove(&id);
                }
            }
        }
    }

    /// Every USER-hidden node, for seeding the UI's visibility state.
    pub fn user_hidden_nodes(&self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = Vec::new();
        out.extend(self.user_hidden_objects.iter().map(|&i| NodeId::Object(i)));
        out.extend(self.user_hidden_groups.iter().map(|&i| NodeId::Group(i)));
        out.extend(
            self.user_hidden_instances
                .iter()
                .map(|&i| NodeId::Instance(i)),
        );
        out
    }

    /// Returns `true` when `object` is a live, visible, watertight (solid) object.
    ///
    /// Returns `false` if the id is stale, hidden, or the object is leaky/open.
    pub fn object_solid(&self, object: ObjectId) -> bool {
        self.objects
            .get(object)
            .filter(|r| !r.hidden)
            .is_some_and(|r| r.object.watertight() == crate::topo::WatertightState::Watertight)
    }

    /// Rename a visible tree node, recording an undoable [`DocAction::NodeMetaChanged`].
    ///
    /// `name = None` clears the name (falls back to positional label in the UI).
    /// Renaming to the current name is a no-op (no undo entry) — consistent with
    /// [`Document::add_node_tag`] / [`Document::remove_node_tag`], so re-committing
    /// an unchanged name (e.g. a focus blur in the UI) never pollutes the undo
    /// stack.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] — stale or hidden node.
    pub fn set_node_name(
        &mut self,
        node: NodeId,
        name: Option<String>,
    ) -> Result<DocChange, DocumentError> {
        let (prev_name, prev_tags) = self.node_meta(node)?;
        if name == prev_name {
            // No change — return a touching change without an undo entry.
            return Ok(self.node_change(node));
        }
        let next_tags = prev_tags.clone();
        self.apply_node_meta(node, name.clone(), next_tags.clone());
        self.undo.push(DocAction::NodeMetaChanged {
            node,
            prev_name,
            next_name: name,
            prev_tags,
            next_tags,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(self.node_change(node))
    }

    /// Append `path` to `node`'s tag list if not already present, recording an
    /// undoable [`DocAction::NodeMetaChanged`]. Returns the change (touching the
    /// node) whether or not the tag was new; only pushes an undo entry when the
    /// tag list actually changed.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] — stale or hidden node.
    pub fn add_node_tag(
        &mut self,
        node: NodeId,
        path: Vec<String>,
    ) -> Result<DocChange, DocumentError> {
        let (prev_name, prev_tags) = self.node_meta(node)?;
        // Only add if not already present.
        if prev_tags.contains(&path) {
            // No change — return a touching change without an undo entry.
            return Ok(self.node_change(node));
        }
        let mut next_tags = prev_tags.clone();
        next_tags.push(path);
        let next_name = prev_name.clone();
        self.apply_node_meta(node, next_name.clone(), next_tags.clone());
        self.undo.push(DocAction::NodeMetaChanged {
            node,
            prev_name: next_name.clone(),
            next_name,
            prev_tags,
            next_tags,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(self.node_change(node))
    }

    /// Remove the first occurrence of `path` from `node`'s tag list, recording
    /// an undoable [`DocAction::NodeMetaChanged`]. No-op (no undo entry) if the
    /// path is not present.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] — stale or hidden node.
    pub fn remove_node_tag(
        &mut self,
        node: NodeId,
        path: &[String],
    ) -> Result<DocChange, DocumentError> {
        let (prev_name, prev_tags) = self.node_meta(node)?;
        let Some(pos) = prev_tags.iter().position(|t| t.as_slice() == path) else {
            // Not present — no undo entry.
            return Ok(self.node_change(node));
        };
        let mut next_tags = prev_tags.clone();
        next_tags.remove(pos);
        let next_name = prev_name.clone();
        self.apply_node_meta(node, next_name.clone(), next_tags.clone());
        self.undo.push(DocAction::NodeMetaChanged {
            node,
            prev_name: next_name.clone(),
            next_name,
            prev_tags,
            next_tags,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(self.node_change(node))
    }

    /// Read name + tags of a live, visible node. Returns `Err` for stale/hidden.
    fn node_meta(&self, node: NodeId) -> Result<(Option<String>, Vec<Vec<String>>), DocumentError> {
        match node {
            NodeId::Object(id) => {
                let rec = self
                    .objects
                    .get(id)
                    .filter(|r| !r.hidden)
                    .ok_or(DocumentError::UnknownObject)?;
                Ok((rec.name.clone(), rec.tags.clone()))
            }
            NodeId::Group(id) => {
                let rec = self
                    .groups
                    .get(id)
                    .filter(|r| !r.hidden)
                    .ok_or(DocumentError::UnknownGroup)?;
                Ok((rec.name.clone(), rec.tags.clone()))
            }
            NodeId::Instance(id) => {
                let rec = self
                    .instances
                    .get(id)
                    .filter(|r| !r.hidden)
                    .ok_or(DocumentError::UnknownInstance)?;
                Ok((rec.name.clone(), rec.tags.clone()))
            }
        }
    }

    /// Write name + tags to a node (no guards — caller has already validated).
    fn apply_node_meta(&mut self, node: NodeId, name: Option<String>, tags: Vec<Vec<String>>) {
        match node {
            NodeId::Object(id) => {
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.name = name;
                    rec.tags = tags;
                }
            }
            NodeId::Group(id) => {
                if let Some(rec) = self.groups.get_mut(id) {
                    rec.name = name;
                    rec.tags = tags;
                }
            }
            NodeId::Instance(id) => {
                if let Some(rec) = self.instances.get_mut(id) {
                    rec.name = name;
                    rec.tags = tags;
                }
            }
        }
    }

    /// Build a [`DocChange`] that marks `node` as touched (in its respective
    /// touched vec — `objects_touched` / `groups_touched` / `instances_touched`).
    fn node_change(&self, node: NodeId) -> DocChange {
        match node {
            NodeId::Object(id) => DocChange {
                objects_touched: vec![id],
                ..Default::default()
            },
            NodeId::Group(id) => DocChange {
                groups_touched: vec![id],
                ..Default::default()
            },
            NodeId::Instance(id) => DocChange {
                instances_touched: vec![id],
                ..Default::default()
            },
        }
    }

    // ---------------------------------------------------------------- objects

    /// A visible Object by handle, or `None` if stale or hidden.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        match self.objects.get(id) {
            Some(rec) if !rec.hidden => Some(&rec.object),
            _ => None,
        }
    }

    /// Whether `id` is a currently visible **world** object (top-level or
    /// grouped), as opposed to a definition member or a hidden/stale object.
    /// The shim uses this to decide whether a touched object is a direct
    /// render/inference candidate, since [`Document::object`] also returns
    /// definition members.
    pub fn is_world_object(&self, id: ObjectId) -> bool {
        self.objects
            .get(id)
            .is_some_and(|r| !r.hidden && r.is_world())
    }

    /// Handles of all currently visible **world** Objects (undone creations are
    /// hidden, not listed), in unspecified but stable order. **Flat** — every
    /// leaf object regardless of group membership, since rendering is flat over
    /// leaves and grouping affects only selection/picking/transform.
    ///
    /// Component-definition members are **excluded**: they live in
    /// definition-local space and reach the scene only through instances (see
    /// [`Document::instance_ids`]), so rendering them here would draw them in
    /// the wrong place and double-count them. Fetch a definition member's
    /// geometry for tessellation via [`Document::object`] with a member id from
    /// [`Document::def_members`].
    pub fn visible_object_ids(&self) -> Vec<ObjectId> {
        self.objects
            .iter()
            .filter(|(_, rec)| !rec.hidden && rec.is_world())
            .map(|(id, _)| id)
            .collect()
    }

    // ----------------------------------------------------------------- groups

    /// Handles of all currently visible Groups (ungrouped groups are hidden,
    /// not listed), in unspecified but stable order.
    pub fn group_ids(&self) -> Vec<GroupId> {
        self.groups
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, _)| id)
            .collect()
    }

    /// Direct members of a visible group, in order, or `None` if the group is
    /// stale or hidden.
    pub fn group_members(&self, group: GroupId) -> Option<Vec<NodeId>> {
        match self.groups.get(group) {
            Some(rec) if !rec.hidden => Some(rec.members.clone()),
            _ => None,
        }
    }

    /// The containing group of a node, or `None` if it is top-level (or the
    /// node handle is stale/hidden).
    pub fn node_parent(&self, node: NodeId) -> Option<GroupId> {
        match node {
            NodeId::Object(id) => self.objects.get(id).filter(|r| !r.hidden)?.group_parent(),
            NodeId::Group(id) => self.groups.get(id).filter(|r| !r.hidden)?.parent,
            NodeId::Instance(id) => self.instances.get(id).filter(|r| !r.hidden)?.parent,
        }
    }

    /// The visible top-level nodes (parent `None`): world objects first, then
    /// groups, then instances, each in slotmap order. The unit of top-level
    /// selection/picking. Definition members are not nodes and never
    /// appear here.
    pub fn top_level_nodes(&self) -> Vec<NodeId> {
        let objects = self
            .objects
            .iter()
            .filter(|(_, r)| !r.hidden && r.is_world() && r.group_parent().is_none())
            .map(|(id, _)| NodeId::Object(id));
        let groups = self
            .groups
            .iter()
            .filter(|(_, r)| !r.hidden && r.parent.is_none())
            .map(|(id, _)| NodeId::Group(id));
        let instances = self
            .instances
            .iter()
            .filter(|(_, r)| !r.hidden && r.parent.is_none())
            .map(|(id, _)| NodeId::Instance(id));
        objects.chain(groups).chain(instances).collect()
    }

    /// Every visible **world** leaf Object beneath `node` (the node itself if it
    /// is a world object), recursively. Drives baked group transforms (which
    /// world objects to bake) and the non-instanced part of the UI isolation
    /// set. **Stops at instances**: an instance's geometry is its definition's
    /// *shared* members, which are never baked — use
    /// [`Document::leaf_instances_under`] for the instances beneath a node.
    /// Empty if `node` is stale.
    pub fn leaf_objects_under(&self, node: NodeId) -> Vec<ObjectId> {
        let mut out = Vec::new();
        self.collect_leaves(node, &mut out);
        out
    }

    fn collect_leaves(&self, node: NodeId, out: &mut Vec<ObjectId>) {
        match node {
            NodeId::Object(id) => {
                if self
                    .objects
                    .get(id)
                    .is_some_and(|r| !r.hidden && r.is_world())
                {
                    out.push(id);
                }
            }
            NodeId::Group(id) => {
                if let Some(rec) = self.groups.get(id).filter(|r| !r.hidden) {
                    for &m in &rec.members {
                        self.collect_leaves(m, out);
                    }
                }
            }
            // An instance is a renderable leaf, but its geometry is the
            // definition's *shared* members (never baked) — counted by
            // `leaf_instances_under`, not here.
            NodeId::Instance(_) => {}
        }
    }

    /// Every visible instance beneath `node` (the node itself if it is an
    /// instance), recursively. Complements [`Document::leaf_objects_under`]: a
    /// node's renderable leaves are its world objects **plus** its instances.
    /// Drives the instance side of a group transform (compose each instance's
    /// pose rather than bake) and the instanced part of the isolation set.
    pub fn leaf_instances_under(&self, node: NodeId) -> Vec<InstanceId> {
        let mut out = Vec::new();
        self.collect_instances(node, &mut out);
        out
    }

    fn collect_instances(&self, node: NodeId, out: &mut Vec<InstanceId>) {
        match node {
            NodeId::Object(_) => {}
            NodeId::Instance(id) => {
                if self.instances.get(id).is_some_and(|r| !r.hidden) {
                    out.push(id);
                }
            }
            NodeId::Group(id) => {
                if let Some(rec) = self.groups.get(id).filter(|r| !r.hidden) {
                    for &m in &rec.members {
                        self.collect_instances(m, out);
                    }
                }
            }
        }
    }

    /// Every visible group at or beneath `node` (the node itself if it is a
    /// group), recursively. Used by `make_component` to hide every group in the
    /// folded subtrees (their leaves move into the definition).
    fn collect_groups(&self, node: NodeId, out: &mut Vec<GroupId>) {
        if let NodeId::Group(id) = node
            && let Some(rec) = self.groups.get(id).filter(|r| !r.hidden)
        {
            out.push(id);
            for &m in &rec.members {
                self.collect_groups(m, out);
            }
        }
    }

    /// Every live node at or beneath `node` (the node itself, plus every live
    /// descendant), recursively, in pre-order. Used by `delete_node` to
    /// capture the exact set of node ids a whole-subtree delete hides — unlike
    /// [`Document::collect_groups`] (groups only) or [`Document::leaf_objects_under`]
    /// (leaf objects only), this names every kind of node in the subtree so undo
    /// can unhide precisely what was hidden.
    fn collect_subtree(&self, node: NodeId, out: &mut Vec<NodeId>) {
        if !self.node_is_live(node) {
            return;
        }
        out.push(node);
        if let NodeId::Group(id) = node {
            let members = self.groups[id].members.clone();
            for m in members {
                self.collect_subtree(m, out);
            }
        }
    }

    // ----------------------------------------------- components & instances

    /// Handles of all currently visible component instances (undone
    /// placements/explodes are hidden, not listed), in stable order.
    pub fn instance_ids(&self) -> Vec<InstanceId> {
        self.instances
            .iter()
            .filter(|(_, r)| !r.hidden)
            .map(|(id, _)| id)
            .collect()
    }

    /// Handles of all currently live component definitions (undone
    /// creations are hidden, not listed), in stable order.
    pub fn component_ids(&self) -> Vec<ComponentId> {
        self.components
            .iter()
            .filter(|(_, c)| !c.hidden)
            .map(|(id, _)| id)
            .collect()
    }

    /// The definition a visible instance places, or `None` if the instance is
    /// stale or hidden.
    pub fn instance_def(&self, instance: InstanceId) -> Option<ComponentId> {
        self.instances
            .get(instance)
            .filter(|r| !r.hidden)
            .map(|r| r.def)
    }

    /// A visible instance's pose (definition-local → world), or `None` if
    /// the instance is stale or hidden.
    pub fn instance_pose(&self, instance: InstanceId) -> Option<Transform> {
        self.instances
            .get(instance)
            .filter(|r| !r.hidden)
            .map(|r| r.pose)
    }

    /// A visible object's display name, or `None` if it is stale/hidden or
    /// unnamed. Callers fall back to a positional label when `None`.
    pub fn object_name(&self, id: ObjectId) -> Option<&str> {
        self.objects
            .get(id)
            .filter(|r| !r.hidden)
            .and_then(|r| r.name.as_deref())
    }

    /// A visible group's display name, or `None` if stale/hidden or unnamed.
    pub fn group_name(&self, id: GroupId) -> Option<&str> {
        self.groups
            .get(id)
            .filter(|r| !r.hidden)
            .and_then(|r| r.name.as_deref())
    }

    /// A visible instance's own display name, or `None` if stale/hidden or
    /// unnamed. An unnamed instance usually displays its def's name — see
    /// [`Document::component_name`] with [`Document::instance_def`].
    pub fn instance_name(&self, id: InstanceId) -> Option<&str> {
        self.instances
            .get(id)
            .filter(|r| !r.hidden)
            .and_then(|r| r.name.as_deref())
    }

    /// A component definition's display name, or `None` if stale/hidden or
    /// unnamed. Used as the fallback label for the definition's instances.
    pub fn component_name(&self, id: ComponentId) -> Option<&str> {
        self.components
            .get(id)
            .filter(|c| !c.hidden)
            .and_then(|c| c.name.as_deref())
    }

    /// The visible instances that place `component`, in stable order. Empty if
    /// the component is stale/hidden or unplaced. Drives shared-geometry
    /// propagation: a `apply_def_op` edit touches exactly these.
    pub fn instances_of(&self, component: ComponentId) -> Vec<InstanceId> {
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return Vec::new();
        }
        self.instances
            .iter()
            .filter(|(_, r)| !r.hidden && r.def == component)
            .map(|(id, _)| id)
            .collect()
    }

    /// The member objects of a live definition, in definition order, or `None`
    /// if the component is stale or hidden. Each is fetched for tessellation via
    /// [`Document::object`]; they are in definition-local coordinates.
    pub fn def_members(&self, component: ComponentId) -> Option<Vec<ObjectId>> {
        self.components
            .get(component)
            .filter(|c| !c.hidden)
            .map(|c| c.members.clone())
    }

    /// Whether a node handle is live and visible (not stale, not hidden). A
    /// definition member is *not* a tree node, so an `Object` handle pointing at
    /// one is not a live node (it fails `is_world`).
    fn node_is_live(&self, node: NodeId) -> bool {
        match node {
            NodeId::Object(id) => self
                .objects
                .get(id)
                .is_some_and(|r| !r.hidden && r.is_world()),
            NodeId::Group(id) => self.group_is_live(id),
            NodeId::Instance(id) => self.instances.get(id).is_some_and(|r| !r.hidden),
        }
    }

    /// Whether a group handle is live and visible (not stale, not hidden).
    fn group_is_live(&self, group: GroupId) -> bool {
        self.groups.get(group).is_some_and(|r| !r.hidden)
    }

    /// Set a node's parent pointer (the half of the parent/members relation
    /// stored on the child). The caller maintains the group's `members` list.
    fn set_node_parent(&mut self, node: NodeId, parent: Option<GroupId>) {
        match node {
            // Only ever called on world nodes (grouping/ungrouping operates on
            // the world tree), so re-homing an object keeps it a world object.
            NodeId::Object(id) => self.objects[id].owner = ObjectOwner::World { parent },
            NodeId::Group(id) => self.groups[id].parent = parent,
            NodeId::Instance(id) => self.instances[id].parent = parent,
        }
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
        info!(target: "kernel::op", op = "extrude_region", distance);
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
            owner: ObjectOwner::World { parent: None },
            name: None,
            tags: Vec::new(),
        });
        // The region is now the bottom of a solid: consume it so it neither
        // re-extrudes nor leaves a stray fill.
        self.consumed.insert((sketch, region));

        // Tombstone the sketch edges/vertices that no longer bound any LIVE
        // region: evaluate the full derived rule for this sketch's consumed
        // set, then keep only what is newly tombstoned so the undo record
        // carries exactly this extrude's increment. An edge shared with a
        // previously consumed region dies here, with the last region that
        // needed it. (Re-borrow the sketch — the earlier `let s = ...` borrow
        // ended after `profile(region)`.)
        let consumed_regions: std::collections::BTreeSet<SketchRegionId> = self
            .consumed
            .iter()
            .filter(|&&(sid, _)| sid == sketch)
            .map(|&(_, rid)| rid)
            .collect();
        let (tomb_edges, tomb_verts) = self.sketches[sketch].consumed_tombstones(&consumed_regions);
        let cons_edges: Vec<SketchEdgeId> = tomb_edges
            .into_iter()
            .filter(|&e| self.consumed_sketch_edges.insert((sketch, e)))
            .collect();
        let cons_verts: Vec<SketchVertexId> = tomb_verts
            .into_iter()
            .filter(|&v| self.consumed_sketch_verts.insert((sketch, v)))
            .collect();

        self.undo.push(DocAction::CreatedObject {
            id,
            sketch,
            region,
            consumed_edges: cons_edges,
            consumed_verts: cons_verts,
        });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: vec![id],
            sketches_touched: vec![sketch],
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
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
            Some(rec) if !rec.hidden && rec.is_world() => rec,
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
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
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
        info!(target: "kernel::op", op = "boolean", boolean_op = ?op);
        if a == b {
            // A single object cannot be combined with itself (its faces would be
            // fully coincident — a degenerate contact); reject before mutating.
            return Err(DocumentError::Boolean(BooleanError::DegenerateContact));
        }
        let rec_a = self
            .objects
            .get(a)
            .filter(|r| !r.hidden && r.is_world())
            .ok_or(DocumentError::UnknownObject)?;
        let rec_b = self
            .objects
            .get(b)
            .filter(|r| !r.hidden && r.is_world())
            .ok_or(DocumentError::UnknownObject)?;
        // A replacing op consumes its operands and emits fresh top-level solids;
        // a grouped operand would orphan the parent group's member list.
        if rec_a.group_parent().is_some() || rec_b.group_parent().is_some() {
            return Err(DocumentError::GroupedOperand);
        }

        let mut result = Object::boolean(op, &rec_a.object, &rec_b.object, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;

        // Dissolve the coplanar seams the boolean introduced (two coplanar
        // top faces joined by a union must read as ONE face), but preserve
        // coplanar edges the operands already had — those are face imprints
        // drawn but not yet extruded, not seams. Runs BEFORE the result is
        // inserted, so undo/redo of the boolean is untouched.
        let preserve: Vec<_> = rec_a
            .object
            .coplanar_edge_segments()
            .into_iter()
            .chain(rec_b.object.coplanar_edge_segments())
            .collect();
        result.merge_coplanar_faces(&preserve);

        let id = self.objects.insert(ObjectRecord {
            object: result,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::World { parent: None },
            name: None,
            tags: Vec::new(),
        });
        self.objects[a].hidden = true;
        self.objects[b].hidden = true;
        self.undo.push(DocAction::Boolean { result: id, a, b });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: vec![a, b, id],
            sketches_touched: Vec::new(),
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        };
        Ok((id, change))
    }

    /// Slice a visible world solid by `plane` into two independent watertight
    /// Objects. The source is hidden (tombstone) and the two pieces are
    /// inserted as top-level world objects; re-joining is an explicit Union.
    /// Undoable; all three handles stay stable. `Err` (document untouched) if
    /// the object is unknown/hidden, not a solid, or the cut is degenerate /
    /// misses the solid — see [`SliceError`].
    ///
    /// Returns `((positive, negative), DocChange)` — the piece on the plane's
    /// normal side first.
    pub fn slice_node(
        &mut self,
        object: ObjectId,
        plane: &Plane,
    ) -> Result<((ObjectId, ObjectId), DocChange), DocumentError> {
        let n = plane.normal();
        info!(target: "kernel::op", op = "slice_node", nx = n.x, ny = n.y, nz = n.z);
        let rec = self
            .objects
            .get(object)
            .filter(|r| !r.hidden && r.is_world())
            .ok_or(DocumentError::UnknownObject)?;
        // Replacing op: a grouped source would orphan its parent group.
        if rec.group_parent().is_some() {
            return Err(DocumentError::GroupedOperand);
        }
        let (positive, negative) = rec.object.slice(plane).map_err(DocumentError::Slice)?;

        let a = self.objects.insert(ObjectRecord {
            object: positive,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::World { parent: None },
            name: None,
            tags: Vec::new(),
        });
        let b = self.objects.insert(ObjectRecord {
            object: negative,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::World { parent: None },
            name: None,
            tags: Vec::new(),
        });
        self.objects[object].hidden = true;
        self.undo.push(DocAction::Sliced {
            source: object,
            a,
            b,
        });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched: vec![object, a, b],
            sketches_touched: Vec::new(),
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        };
        Ok(((a, b), change))
    }

    /// Push `face` of a visible world solid inward by `distance` *past* opposing
    /// material, as a subtract: material the swept face passes through is
    /// removed — a recess that breaks the far wall becomes a through-hole, and a
    /// cut that fully severs the solid yields two (or more) independent Objects.
    /// The source is hidden (tombstone); the result pieces become top-level
    /// world objects. Undoable; handles stable. Routed to by the push/pull entry
    /// when [`Object::push_pull_overshoots`] reports the through case.
    ///
    /// Returns `(result_ids, DocChange)`. `Err` (document untouched) if the
    /// object is unknown/hidden or the subtract is degenerate / removes all
    /// material — see [`PushPullError`](crate::ops::PushPullError).
    pub fn push_pull_through(
        &mut self,
        object: ObjectId,
        face: crate::ids::FaceId,
        distance: f64,
    ) -> Result<(Vec<ObjectId>, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "push_pull_through", distance);
        let rec = self
            .objects
            .get(object)
            .filter(|r| !r.hidden && r.is_world())
            .ok_or(DocumentError::UnknownObject)?;
        // Replacing op: a grouped source would orphan its parent group.
        if rec.group_parent().is_some() {
            return Err(DocumentError::GroupedOperand);
        }
        let result = rec
            .object
            .push_through(face, distance)
            .map_err(|e| DocumentError::Op(KernelOpError::PushPull(e)))?;
        let pieces = result.split_connected_components();

        let mut results: Vec<ObjectId> = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let id = self.objects.insert(ObjectRecord {
                object: piece,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::World { parent: None },
                name: None,
                tags: Vec::new(),
            });
            results.push(id);
        }
        self.objects[object].hidden = true;
        self.undo.push(DocAction::PushThrough {
            source: object,
            results: results.clone(),
        });
        self.redo.clear();
        self.debug_validate();

        let mut objects_touched = results.clone();
        objects_touched.push(object);
        let change = DocChange {
            objects_touched,
            sketches_touched: Vec::new(),
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        };
        Ok((results, change))
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
        info!(target: "kernel::op", op = "transform_object");
        // Capture the inverse first: it both validates invertibility and is what
        // undo will bake. (`apply_transform` re-checks and also rejects det<0.)
        let inverse = t.inverse().map_err(DocumentError::Transform)?;
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden && rec.is_world() => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        rec.object
            .apply_transform(t)
            .map_err(DocumentError::Transform)?;
        self.undo.push(DocAction::Transform {
            objects: vec![object],
            forward: *t,
            inverse,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: vec![object],
            sketches_touched: Vec::new(),
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        })
    }

    /// Bakes an affine into a free-standing sketch's geometry (Phase D move/
    /// rotate/scale). The sketch analogue of [`Document::transform_object`]:
    /// every vertex moves and the sketch plane is remapped, the `SketchId`
    /// stays stable, and the change is undoable via [`DocAction::TransformSketch`].
    ///
    /// # Errors
    /// - [`DocumentError::UnknownSketch`] — stale or hidden (deleted) sketch.
    /// - [`DocumentError::Transform`] — singular or orientation-flipping map;
    ///   the sketch is left untouched (transactional).
    pub fn transform_sketch(
        &mut self,
        sketch: SketchId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        // Capture the inverse first: it both validates invertibility and is what
        // undo will bake. (`apply_transform` re-checks and also rejects det<0.)
        let inverse = t.inverse().map_err(DocumentError::Transform)?;
        if !self.sketches.contains_key(sketch) || self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        self.sketches[sketch]
            .apply_transform(t)
            .map_err(DocumentError::Transform)?;
        self.undo.push(DocAction::TransformSketch {
            sketch,
            forward: *t,
            inverse,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: Vec::new(),
            sketches_touched: vec![sketch],
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        })
    }

    /// Drags one vertex of a free-standing sketch to `new_pos` (Phase D
    /// per-vertex edit). Topology-preserving — see [`Sketch::move_vertex`]:
    /// the vertex moves and its incident edges stretch, but nothing splits,
    /// merges, or re-forms. Undoable via [`DocAction::MovedSketchVertex`].
    ///
    /// # Errors
    /// - [`DocumentError::UnknownSketch`] — stale or hidden (deleted) sketch.
    /// - [`DocumentError::Sketch`] — the move was refused (off-plane, would
    ///   collapse an incident edge, or would cross/merge geometry); the sketch
    ///   is left untouched (the [`Sketch::move_vertex`] strong guarantee).
    pub fn move_sketch_vertex(
        &mut self,
        sketch: SketchId,
        vertex: SketchVertexId,
        new_pos: Point3,
    ) -> Result<DocChange, DocumentError> {
        if !self.sketches.contains_key(sketch) || self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        let old_pos = self.sketches[sketch]
            .move_vertex(vertex, new_pos)
            .map_err(DocumentError::Sketch)?;
        self.undo.push(DocAction::MovedSketchVertex {
            sketch,
            vertex,
            old_pos,
            new_pos,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: Vec::new(),
            sketches_touched: vec![sketch],
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        })
    }

    /// Non-destructively groups sibling nodes into a new [`Group`](GroupRecord)
    /// (ARCHITECTURE.md). Unlike a boolean union, no geometry is welded and no
    /// member is consumed — the members keep their identity, geometry, and
    /// watertightness; the group is a container for selection and transform.
    ///
    /// The members must be live, visible, distinct, and **siblings** (all
    /// top-level, or all direct children of one group); the new group takes
    /// their shared parent and is inserted at the first member's position.
    /// `Err` (leaving the document untouched) on an empty/duplicate selection,
    /// a stale/hidden member, or mixed parents.
    pub fn group_nodes(
        &mut self,
        members: &[NodeId],
    ) -> Result<(GroupId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "group_nodes", members = members.len());
        if members.is_empty() {
            return Err(DocumentError::EmptyGroup);
        }
        for (i, m) in members.iter().enumerate() {
            if members[i + 1..].contains(m) {
                return Err(DocumentError::DuplicateMember);
            }
        }
        for &m in members {
            if !self.node_is_live(m) {
                return Err(match m {
                    NodeId::Object(_) => DocumentError::UnknownObject,
                    NodeId::Group(_) => DocumentError::UnknownGroup,
                    NodeId::Instance(_) => DocumentError::UnknownInstance,
                });
            }
        }
        let parent = self.node_parent(members[0]);
        if members[1..].iter().any(|&m| self.node_parent(m) != parent) {
            return Err(DocumentError::MixedParents);
        }

        let prev_parent_members = parent.map(|pg| self.groups[pg].members.clone());
        let group = self.groups.insert(GroupRecord {
            members: members.to_vec(),
            parent,
            hidden: false,
            name: None,
            tags: Vec::new(),
        });
        for &m in members {
            self.set_node_parent(m, Some(group));
        }
        if let Some(pg) = parent {
            self.splice_in_parent(pg, members, NodeId::Group(group));
        }

        self.undo.push(DocAction::Grouped {
            group,
            parent,
            prev_parent_members,
        });
        self.redo.clear();
        self.debug_validate();

        Ok((group, group_change(group, parent, members)))
    }

    /// Dissolves a group, returning its members to the group's own parent (the
    /// members keep their subtrees). The exact inverse of [`group_nodes`]. The
    /// `GroupId` is retained but hidden, so redo can re-form it. `Err` (document
    /// untouched) if the group handle is stale or already hidden.
    pub fn ungroup(&mut self, group: GroupId) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "ungroup");
        if !self.group_is_live(group) {
            return Err(DocumentError::UnknownGroup);
        }
        let parent = self.groups[group].parent;
        let members = self.groups[group].members.clone();
        let prev_parent_members = parent.map(|pg| self.groups[pg].members.clone());

        for &m in &members {
            self.set_node_parent(m, parent);
        }
        if let Some(pg) = parent {
            self.splice_out_parent(pg, NodeId::Group(group), &members);
        }
        self.groups[group].hidden = true;

        self.undo.push(DocAction::Ungrouped {
            group,
            parent,
            prev_parent_members,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(group_change(group, parent, &members))
    }

    /// Removes a whole tree node — an Object, Group, or Instance — from the
    /// document. Whole-node delete only: single-face/edge delete is
    /// out of scope (it would open a watertight solid) and guide selections are
    /// routed elsewhere ('s `delete_guide`/`delete_all_guides`).
    ///
    /// Like every other document mutation, this is a tombstone, not a real
    /// delete: `node` and its entire live subtree are hidden (never erased), so
    /// every id stays valid for redo. Deleting a Group hides the group shell
    /// *and* its whole subtree in one step — unlike [`Document::ungroup`], which
    /// reparents members up, a delete makes the whole subtree disappear.
    /// Deleting an Instance hides only that instance node; its shared
    /// [`ComponentDef`] and sibling instances are untouched. `node` is spliced
    /// out of its parent's member list (or the top-level order, which needs no
    /// bookkeeping); the exact position is captured for undo.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] — `node` is stale or already hidden.
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn delete_node(&mut self, node: NodeId) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "delete_node", node = ?node);
        if !self.node_is_live(node) {
            return Err(match node {
                NodeId::Object(_) => DocumentError::UnknownObject,
                NodeId::Group(_) => DocumentError::UnknownGroup,
                NodeId::Instance(_) => DocumentError::UnknownInstance,
            });
        }
        let parent = self.node_parent(node);
        let prev_parent_members = parent.map(|pg| self.groups[pg].members.clone());

        let mut hidden_subtree = Vec::new();
        self.collect_subtree(node, &mut hidden_subtree);
        for &n in &hidden_subtree {
            match n {
                NodeId::Object(id) => self.objects[id].hidden = true,
                NodeId::Group(id) => self.groups[id].hidden = true,
                NodeId::Instance(id) => self.instances[id].hidden = true,
            }
        }
        if let Some(pg) = parent {
            self.splice_out_parent(pg, node, &[]);
        }

        self.undo.push(DocAction::Deleted {
            node,
            parent,
            prev_parent_members,
            hidden_subtree: hidden_subtree.clone(),
        });
        self.redo.clear();
        self.debug_validate();

        Ok(delete_change(node, parent, &hidden_subtree))
    }

    /// Move / rotate / scale a group: **bake** `t` into every world leaf object
    /// beneath it and **compose** it into the pose of every instance
    /// beneath it — the group itself holds no pose. Undoable via the exact
    /// inverse; all handles stay stable. `Err` (document untouched) if the group
    /// is unknown/hidden or `t` is singular or orientation-flipping.
    pub fn transform_group(
        &mut self,
        group: GroupId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        // Pre-validate invertibility before mutating; this is also undo's bake.
        let inverse = t.inverse().map_err(DocumentError::Transform)?;
        if !self.group_is_live(group) {
            return Err(DocumentError::UnknownGroup);
        }
        let leaves = self.leaf_objects_under(NodeId::Group(group));
        // TODO(components): also compose `t` into `leaf_instances_under`
        // (instance poses, never baked) and extend `DocAction::Transform` to
        // carry their prior poses for an exact undo. Unreachable until the
        // instance ops below land (no instance can be a group member yet), so
        // baking world leaves here stays correct for the current model.

        // `t` is invertible and non-reflecting, so per-leaf apply cannot fail
        // for geometric reasons. Should one somehow err, roll back the leaves
        // already baked to preserve the strong guarantee.
        let mut done: Vec<ObjectId> = Vec::new();
        for &obj in &leaves {
            match self.objects[obj].object.apply_transform(t) {
                Ok(()) => done.push(obj),
                Err(e) => {
                    for &d in &done {
                        self.objects[d]
                            .object
                            .apply_transform(&inverse)
                            .expect("inverse of a validated transform must re-apply");
                    }
                    return Err(DocumentError::Transform(e));
                }
            }
        }

        self.undo.push(DocAction::Transform {
            objects: leaves.clone(),
            forward: *t,
            inverse,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: leaves,
            sketches_touched: Vec::new(),
            groups_touched: vec![group],
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        })
    }

    /// Move/rotate/scale a whole mixed selection — world objects, groups,
    /// component instances, and free-standing sketches — as **one undoable
    /// step** ([`DocAction::TransformSelection`]; select-all → Move).
    ///
    /// Each listed node is flattened the way [`Document::transform_group`]
    /// flattens a group: `t` is **baked** into every visible world leaf
    /// object beneath it, and **composed** into the pose of every instance
    /// beneath it (exactly like [`Document::transform_instance`], geometry
    /// shared, never baked). Each listed sketch is baked like
    /// [`Document::transform_sketch`]. Flattened targets are deduplicated in
    /// first-listing order, so listing a node alongside its ancestor group
    /// transforms it once — never twice. All handles stay stable.
    ///
    /// # Errors
    /// - [`DocumentError::EmptySelection`] — nothing listed, or every listed
    ///   node flattened to nothing visible.
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] / [`DocumentError::UnknownSketch`]
    ///   — a listed handle is stale or hidden.
    /// - [`DocumentError::Transform`] — `t` is singular, or it reflects and
    ///   the selection contains a baked target (an object or sketch), which
    ///   [`Object::apply_transform`] refuses.
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn transform_selection(
        &mut self,
        nodes: &[NodeId],
        sketches: &[SketchId],
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "transform_selection");
        // Pre-validate invertibility before mutating; this is also undo's bake.
        let inverse = t.inverse().map_err(DocumentError::Transform)?;

        // Validate every listed handle up front — nothing mutates until the
        // whole selection is known live.
        for &node in nodes {
            match node {
                NodeId::Object(id) => {
                    if !self
                        .objects
                        .get(id)
                        .is_some_and(|r| !r.hidden && r.is_world())
                    {
                        return Err(DocumentError::UnknownObject);
                    }
                }
                NodeId::Group(id) => {
                    if !self.group_is_live(id) {
                        return Err(DocumentError::UnknownGroup);
                    }
                }
                NodeId::Instance(id) => {
                    if self.instances.get(id).is_none_or(|r| r.hidden) {
                        return Err(DocumentError::UnknownInstance);
                    }
                }
            }
        }
        for &s in sketches {
            if !self.sketches.contains_key(s) || self.hidden_sketches.contains(&s) {
                return Err(DocumentError::UnknownSketch);
            }
        }

        // Flatten to unique leaf targets in first-listing order. Select-all →
        // Move visits every leaf in the model, so membership checks use
        // BTreeSets (deterministic, per clippy.toml) beside the order-keeping
        // Vecs rather than an O(n²) Vec::contains scan.
        let mut objects: Vec<ObjectId> = Vec::new();
        let mut object_set: BTreeSet<ObjectId> = BTreeSet::new();
        let mut instances: Vec<InstanceId> = Vec::new();
        let mut instance_set: BTreeSet<InstanceId> = BTreeSet::new();
        for &node in nodes {
            for obj in self.leaf_objects_under(node) {
                if object_set.insert(obj) {
                    objects.push(obj);
                }
            }
            for inst in self.leaf_instances_under(node) {
                if instance_set.insert(inst) {
                    instances.push(inst);
                }
            }
        }
        let mut sketch_targets: Vec<SketchId> = Vec::new();
        let mut sketch_set: BTreeSet<SketchId> = BTreeSet::new();
        for &s in sketches {
            if sketch_set.insert(s) {
                sketch_targets.push(s);
            }
        }

        if objects.is_empty() && instances.is_empty() && sketch_targets.is_empty() {
            return Err(DocumentError::EmptySelection);
        }

        // Bake into objects, then sketches. `t` is invertible, so a per-target
        // failure (a reflecting `t` hitting a baked target) can only happen on
        // the first bake — but roll back whatever was already baked either
        // way, to preserve the strong guarantee.
        let mut baked_objects: Vec<ObjectId> = Vec::new();
        let mut baked_sketches: Vec<SketchId> = Vec::new();
        for &obj in &objects {
            if let Err(e) = self.objects[obj].object.apply_transform(t) {
                self.rollback_selection_bakes(&baked_objects, &baked_sketches, &inverse);
                return Err(DocumentError::Transform(e));
            }
            baked_objects.push(obj);
        }
        for &s in &sketch_targets {
            if let Err(e) = self.sketches[s].apply_transform(t) {
                self.rollback_selection_bakes(&baked_objects, &baked_sketches, &inverse);
                return Err(DocumentError::Transform(e));
            }
            baked_sketches.push(s);
        }

        // Compose into instance poses last — cannot fail once `t` is known
        // invertible, so no rollback is reachable past this point.
        let mut instance_prevs: Vec<(InstanceId, Transform)> = Vec::with_capacity(instances.len());
        for &inst in &instances {
            let rec = &mut self.instances[inst];
            let prev = rec.pose;
            rec.pose = prev.then(t);
            instance_prevs.push((inst, prev));
        }

        let groups_touched: Vec<GroupId> = nodes
            .iter()
            .filter_map(|&n| match n {
                NodeId::Group(g) => Some(g),
                _ => None,
            })
            .collect();

        self.undo.push(DocAction::TransformSelection {
            objects: objects.clone(),
            sketches: sketch_targets.clone(),
            instances: instance_prevs,
            forward: *t,
            inverse,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: objects,
            sketches_touched: sketch_targets,
            groups_touched,
            instances_touched: instances,
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        })
    }

    /// Bake `inverse` back into targets a failed `transform_selection` had
    /// already transformed — the strong-guarantee rollback shared by its bake
    /// arms. The inverse of a validated transform cannot fail to re-apply.
    fn rollback_selection_bakes(
        &mut self,
        baked_objects: &[ObjectId],
        baked_sketches: &[SketchId],
        inverse: &Transform,
    ) {
        for &s in baked_sketches {
            self.sketches[s]
                .apply_transform(inverse)
                .expect("inverse of a validated transform must re-apply");
        }
        for &d in baked_objects {
            self.objects[d]
                .object
                .apply_transform(inverse)
                .expect("inverse of a validated transform must re-apply");
        }
    }

    // ------------------------------------------------- component mutations

    /// Folds a selection of sibling nodes into a new component definition plus
    /// one identity-posed instance in their place (ARCHITECTURE.md) — the
    /// "Make Component" act.
    ///
    /// The selection is flattened to its leaf world objects
    /// ([`Document::leaf_objects_under`] over each member); those objects are
    /// re-owned as [`ObjectOwner::Definition`] members of the new
    /// [`ComponentDef`] **without moving any geometry** (the definition-local
    /// frame *is* the world frame at creation), and a single instance with
    /// `pose == Transform::IDENTITY` is created at the selection's shared parent
    /// — so creation is a visual no-op. Any selected groups/instances are
    /// consumed (hidden); their internal structure is flattened away (nested
    /// definitions are deferred). The instance becomes the unit of
    /// selection/transform; editing the shared geometry later goes through
    /// [`Document::apply_def_op`].
    ///
    /// Returns the new definition, its first instance, and the [`DocChange`].
    /// The whole act is one undoable step ([`DocAction::MadeComponent`]), exactly
    /// reversible and handle-stable (hide-not-delete).
    ///
    /// # Errors
    /// - [`DocumentError::EmptyComponent`] — no nodes selected.
    /// - [`DocumentError::DuplicateMember`] — a node listed twice.
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] — a stale/hidden member.
    /// - [`DocumentError::MixedParents`] — members are not siblings (they must
    ///   all be top-level or all direct children of one group, as with
    ///   [`Document::group_nodes`]).
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn make_component(
        &mut self,
        members: &[NodeId],
    ) -> Result<(ComponentId, InstanceId, DocChange), DocumentError> {
        if members.is_empty() {
            return Err(DocumentError::EmptyComponent);
        }
        for (i, m) in members.iter().enumerate() {
            if members[i + 1..].contains(m) {
                return Err(DocumentError::DuplicateMember);
            }
        }
        for &m in members {
            if !self.node_is_live(m) {
                return Err(match m {
                    NodeId::Object(_) => DocumentError::UnknownObject,
                    NodeId::Group(_) => DocumentError::UnknownGroup,
                    NodeId::Instance(_) => DocumentError::UnknownInstance,
                });
            }
            // Nesting a component inside a definition is deferred — and that
            // covers instances anywhere in a member's subtree, not just
            // direct ones: consuming a group while an instance inside it
            // still names the group as its parent would strand the
            // instance's parent link.
            if matches!(m, NodeId::Instance(_)) || !self.leaf_instances_under(m).is_empty() {
                return Err(DocumentError::NestedComponentUnsupported);
            }
        }
        let parent = self.node_parent(members[0]);
        if members[1..].iter().any(|&m| self.node_parent(m) != parent) {
            return Err(DocumentError::MixedParents);
        }

        // Flatten the selection to its leaf world objects. Instances were
        // refused above, so every world solid in the selection is covered.
        let mut leaves: Vec<ObjectId> = Vec::new();
        for &m in members {
            for o in self.leaf_objects_under(m) {
                if !leaves.contains(&o) {
                    leaves.push(o);
                }
            }
        }
        if leaves.is_empty() {
            return Err(DocumentError::EmptyComponent);
        }

        // Capture exact-undo state before mutating.
        let member_prior_parents: Vec<(ObjectId, Option<GroupId>)> = leaves
            .iter()
            .map(|&o| (o, self.objects[o].group_parent()))
            .collect();
        let mut consumed_groups: Vec<GroupId> = Vec::new();
        for &m in members {
            self.collect_groups(m, &mut consumed_groups);
        }
        let prev_parent_members = parent.map(|pg| self.groups[pg].members.clone());

        // Build the definition + its single identity-posed instance. No geometry
        // moves: the definition-local frame is the world frame at creation.
        let component = self.components.insert(ComponentDef {
            members: leaves.clone(),
            hidden: false,
            name: None,
        });
        for &o in &leaves {
            self.objects[o].owner = ObjectOwner::Definition(component);
        }
        for &g in &consumed_groups {
            self.groups[g].hidden = true;
        }
        let instance = self.instances.insert(InstanceRecord {
            def: component,
            pose: Transform::IDENTITY,
            parent,
            hidden: false,
            name: None,
            tags: Vec::new(),
        });
        if let Some(pg) = parent {
            self.splice_in_parent(pg, members, NodeId::Instance(instance));
        }

        self.undo.push(DocAction::MadeComponent {
            component,
            instance,
            selected: members.to_vec(),
            parent,
            member_prior_parents,
            consumed_groups: consumed_groups.clone(),
            prev_parent_members,
        });
        self.redo.clear();
        self.debug_validate();

        let change = made_component_change(component, instance, parent, &leaves, &consumed_groups);
        Ok((component, instance, change))
    }

    /// Stamps another instance of an existing definition at `pose` — the
    /// shared-geometry payoff: no geometry is copied, only a new posed reference.
    /// The instance lands at the top level. Recorded as
    /// [`DocAction::PlacedInstance`].
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — the definition is stale/hidden.
    /// - [`DocumentError::Transform`] — `pose` is singular (non-invertible).
    ///   Reflection and non-uniform scale are **allowed**.
    ///
    /// On `Err` the document is untouched.
    pub fn place_instance(
        &mut self,
        component: ComponentId,
        pose: Transform,
    ) -> Result<(InstanceId, DocChange), DocumentError> {
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return Err(DocumentError::UnknownComponent);
        }
        // Reject a singular pose; reflection and non-uniform scale are fine.
        pose.inverse().map_err(DocumentError::Transform)?;

        let instance = self.instances.insert(InstanceRecord {
            def: component,
            pose,
            parent: None,
            hidden: false,
            name: None,
            tags: Vec::new(),
        });
        self.undo.push(DocAction::PlacedInstance { instance });
        self.redo.clear();
        self.debug_validate();

        Ok((
            instance,
            DocChange {
                instances_touched: vec![instance],
                components_touched: vec![component],
                ..Default::default()
            },
        ))
    }

    /// Move/rotate/scale a visible instance by **composing** `t` into its pose
    /// (`pose' = pose.then(t)`) — *not* baked: the geometry is shared, so
    /// only this instance's pose changes. The pose may end up mirrored or
    /// non-uniformly scaled; only a singular `t` is refused. Undo restores the
    /// exact prior pose ([`DocAction::TransformInstance`]).
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — the instance is stale/hidden.
    /// - [`DocumentError::Transform`] — `t` is singular.
    ///
    /// On `Err` the document is untouched.
    pub fn transform_instance(
        &mut self,
        instance: InstanceId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        // Reject a singular `t`; reflection and non-uniform scale are fine.
        t.inverse().map_err(DocumentError::Transform)?;
        let rec = match self.instances.get_mut(instance) {
            Some(rec) if !rec.hidden => rec,
            _ => return Err(DocumentError::UnknownInstance),
        };
        let prev = rec.pose;
        let next = prev.then(t);
        rec.pose = next;
        self.undo.push(DocAction::TransformInstance {
            instance,
            prev,
            next,
        });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            instances_touched: vec![instance],
            ..Default::default()
        })
    }

    /// Edit the shared geometry of a definition by applying a per-Object op to
    /// one of its member objects — drawing/push-pull *inside a component*.
    /// The change is seen by **every** instance of `component` at once. Routed
    /// through that member's [`History`] and recorded as
    /// [`DocAction::DefObjectOp`]; the returned [`DocChange`] names the component
    /// and all its instances (so the shim re-tessellates the shared mesh and
    /// refreshes every placement).
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — the definition is stale/hidden.
    /// - [`DocumentError::UnknownObject`] — `object` is not a member of it.
    /// - [`DocumentError::Op`] — the op failed (the member is untouched — the
    ///   op's strong guarantee).
    pub fn apply_def_op(
        &mut self,
        component: ComponentId,
        object: ObjectId,
        op: KernelOp,
    ) -> Result<(KernelOpReport, DocChange), DocumentError> {
        match self.components.get(component) {
            Some(c) if !c.hidden => {
                if !c.members.contains(&object) {
                    return Err(DocumentError::UnknownObject);
                }
            }
            _ => return Err(DocumentError::UnknownComponent),
        }
        let rec = self
            .objects
            .get_mut(object)
            .ok_or(DocumentError::UnknownObject)?;
        let report = rec
            .history
            .apply(&mut rec.object, op)
            .map_err(DocumentError::Op)?;
        self.undo.push(DocAction::DefObjectOp { component, object });
        self.redo.clear();
        self.debug_validate();

        // A shared-geometry edit is seen by every instance of the definition.
        let instances_touched = self.instances_of(component);
        Ok((
            report,
            DocChange {
                objects_touched: vec![object],
                components_touched: vec![component],
                instances_touched,
                ..Default::default()
            },
        ))
    }

    /// Detach an instance into independent world geometry — "Explode".
    /// Each definition member is cloned, the instance pose is **baked** into the
    /// clone (reusing [`Object::apply_transform`]), and the clones are inserted
    /// as top-level world objects at the instance's parent; the instance is then
    /// hidden. The definition and any sibling instances are untouched. Recorded
    /// as [`DocAction::Exploded`]; handle-stable and reversible.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — the instance is stale/hidden.
    /// - [`DocumentError::CannotExplodeReflected`] — the pose mirrors
    ///   (determinant < 0); baking it would invert winding. Use
    ///   [`Document::make_unique`] then edit instead, or unmirror first.
    ///
    /// On `Err` the document is untouched.
    pub fn explode_instance(
        &mut self,
        instance: InstanceId,
    ) -> Result<(Vec<ObjectId>, DocChange), DocumentError> {
        let (def, pose, parent) = match self.instances.get(instance) {
            Some(rec) if !rec.hidden => (rec.def, rec.pose, rec.parent),
            _ => return Err(DocumentError::UnknownInstance),
        };
        // Baking a reflection would invert winding: refuse before mutating.
        if pose.determinant() < 0.0 {
            return Err(DocumentError::CannotExplodeReflected);
        }
        let members = match self.components.get(def) {
            Some(c) if !c.hidden => c.members.clone(),
            // The instance held a live def by invariant; treat otherwise as a bug.
            _ => return Err(DocumentError::UnknownComponent),
        };

        // Clone each member and bake the pose into the copy as an independent
        // world object in the instance's container. `pose` is invertible and
        // orientation-preserving, so `apply_transform` cannot fail.
        let mut created: Vec<ObjectId> = Vec::with_capacity(members.len());
        for m in members {
            let mut object = self.objects[m].object.clone();
            object
                .apply_transform(&pose)
                .map_err(DocumentError::Transform)?;
            let id = self.objects.insert(ObjectRecord {
                object,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::World { parent },
                name: None,
                tags: Vec::new(),
            });
            created.push(id);
        }

        self.instances[instance].hidden = true;
        if let Some(pg) = parent {
            let nodes: Vec<NodeId> = created.iter().map(|&o| NodeId::Object(o)).collect();
            self.splice_out_parent(pg, NodeId::Instance(instance), &nodes);
        }
        self.undo.push(DocAction::Exploded {
            instance,
            created: created.clone(),
        });
        self.redo.clear();
        self.debug_validate();

        let mut change = DocChange {
            objects_touched: created.clone(),
            instances_touched: vec![instance],
            ..Default::default()
        };
        change.groups_touched.extend(parent);
        Ok((created, change))
    }

    /// Detach one instance onto its **own private copy** of the definition
    /// — "Make Unique". The definition's members are deep-copied into a
    /// fresh [`ComponentDef`] and the instance is repointed to it (pose
    /// unchanged), so later [`Document::apply_def_op`] edits to this instance no
    /// longer affect its former siblings. Recorded as [`DocAction::MadeUnique`].
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — the instance is stale/hidden.
    ///
    /// On `Err` the document is untouched.
    pub fn make_unique(
        &mut self,
        instance: InstanceId,
    ) -> Result<(ComponentId, DocChange), DocumentError> {
        let prev_def = match self.instances.get(instance) {
            Some(rec) if !rec.hidden => rec.def,
            _ => return Err(DocumentError::UnknownInstance),
        };
        let members = match self.components.get(prev_def) {
            Some(c) if !c.hidden => c.members.clone(),
            _ => return Err(DocumentError::UnknownComponent),
        };

        // Deep-copy each member into a fresh private definition (def-local
        // geometry, fresh per-object history). Inherit the source def's name so
        // a made-unique instance keeps its label.
        let prev_name = self.components[prev_def].name.clone();
        let new_def = self.components.insert(ComponentDef {
            members: Vec::new(),
            hidden: false,
            name: prev_name,
        });
        let mut new_members: Vec<ObjectId> = Vec::with_capacity(members.len());
        for m in members {
            let object = self.objects[m].object.clone();
            let id = self.objects.insert(ObjectRecord {
                object,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::Definition(new_def),
                name: None,
                tags: Vec::new(),
            });
            new_members.push(id);
        }
        self.components[new_def].members = new_members;
        self.instances[instance].def = new_def;

        self.undo.push(DocAction::MadeUnique {
            instance,
            prev_def,
            new_def,
        });
        self.redo.clear();
        self.debug_validate();

        Ok((
            new_def,
            DocChange {
                instances_touched: vec![instance],
                components_touched: vec![prev_def, new_def],
                ..Default::default()
            },
        ))
    }

    /// Deep-clone a node (Object / Group / Instance) and place the copy under the
    /// **same parent** as the source, offset by `placement` — the kernel
    /// half of Move+Option "copy". Returns the new root node (always the **same
    /// kind** as the source) and what it touched.
    ///
    /// This is deliberately distinct from its two neighbours:
    /// - unlike [`Document::make_unique`] (detach an instance from its shared
    ///   definition), a duplicated **Object** is a genuinely independent baked
    ///   solid with fresh geometry and its own empty history;
    /// - unlike [`Document::place_instance`] (share geometry under a new pose),
    ///   it copies whatever the source *is* — an Object copy is new geometry, a
    ///   Group copy is a new subtree, an Instance copy is another instance of the
    ///   same definition.
    ///
    /// `placement` is composed per kind exactly as the matching transform op:
    /// baked into a cloned Object's geometry ([`Object::apply_transform`]); baked
    /// into every cloned leaf of a Group (like [`Document::transform_group`]);
    /// and composed into a cloned Instance's pose (like
    /// [`Document::transform_instance`]), keeping its geometry shared.
    ///
    /// Recorded as [`DocAction::Duplicated`]; undo/redo are handle-stable.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] / `UnknownGroup` / `UnknownInstance` —
    ///   the source node is stale or hidden.
    /// - [`DocumentError::Transform`] — `placement` is singular, or it reflects an
    ///   Object/Group leaf (baking would invert winding, as in
    ///   [`Document::transform_object`]).
    ///
    /// On `Err` the document is untouched (a partial clone is rolled back).
    pub fn duplicate_node(
        &mut self,
        node: NodeId,
        placement: &Transform,
    ) -> Result<(NodeId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "duplicate_node", node = ?node);
        if !self.node_is_live(node) {
            return Err(match node {
                NodeId::Object(_) => DocumentError::UnknownObject,
                NodeId::Group(_) => DocumentError::UnknownGroup,
                NodeId::Instance(_) => DocumentError::UnknownInstance,
            });
        }
        // Validate invertibility up front; a reflecting `placement` is re-rejected
        // by `apply_transform` per object leaf during the clone.
        placement.inverse().map_err(DocumentError::Transform)?;

        let parent = self.node_parent(node);
        let mut created = CreatedClone::default();
        let root = match self.clone_subtree(node, parent, placement, &mut created) {
            Ok(root) => root,
            Err(e) => {
                // Roll back any records inserted before the failure so the
                // document is untouched on error (strong guarantee). Nothing
                // outside `created` has been mutated yet.
                for o in created.objects {
                    self.objects.remove(o);
                }
                for g in created.groups {
                    self.groups.remove(g);
                }
                for i in created.instances {
                    self.instances.remove(i);
                }
                return Err(e);
            }
        };
        // Append the clone root to its parent's member list (top-level nodes need
        // no list — they derive from the slotmap, hidden-filtered).
        if let Some(pg) = parent {
            self.groups[pg].members.push(root);
        }

        self.undo.push(DocAction::Duplicated {
            root,
            parent,
            objects: created.objects.clone(),
            groups: created.groups.clone(),
            instances: created.instances.clone(),
        });
        self.redo.clear();
        self.debug_validate();

        let mut change = DocChange {
            objects_touched: created.objects,
            groups_touched: created.groups,
            instances_touched: created.instances,
            ..Default::default()
        };
        change.groups_touched.extend(parent);
        Ok((root, change))
    }

    /// Recursively deep-clone `node` under `new_parent`, baking/composing
    /// `placement` per kind (see [`Document::duplicate_node`]). Newly created ids
    /// are pushed onto `created` as they are inserted, so the caller can roll back
    /// on error and record one atomic action. Returns the cloned node's id.
    fn clone_subtree(
        &mut self,
        node: NodeId,
        new_parent: Option<GroupId>,
        placement: &Transform,
        created: &mut CreatedClone,
    ) -> Result<NodeId, DocumentError> {
        match node {
            NodeId::Object(id) => {
                let src = &self.objects[id];
                let mut object = src.object.clone();
                let name = src.name.clone();
                let tags = src.tags.clone();
                object
                    .apply_transform(placement)
                    .map_err(DocumentError::Transform)?;
                let new_id = self.objects.insert(ObjectRecord {
                    object,
                    history: History::new(),
                    hidden: false,
                    owner: ObjectOwner::World { parent: new_parent },
                    name,
                    tags,
                });
                created.objects.push(new_id);
                Ok(NodeId::Object(new_id))
            }
            NodeId::Instance(id) => {
                let src = &self.instances[id];
                // Compose like `transform_instance`: an invertible `placement`
                // into an invertible pose stays invertible, so no extra check.
                let pose = src.pose.then(placement);
                let def = src.def;
                let name = src.name.clone();
                let tags = src.tags.clone();
                let new_id = self.instances.insert(InstanceRecord {
                    def,
                    pose,
                    parent: new_parent,
                    hidden: false,
                    name,
                    tags,
                });
                created.instances.push(new_id);
                Ok(NodeId::Instance(new_id))
            }
            NodeId::Group(id) => {
                let members = self.groups[id].members.clone();
                let name = self.groups[id].name.clone();
                let tags = self.groups[id].tags.clone();
                let new_gid = self.groups.insert(GroupRecord {
                    members: Vec::new(),
                    parent: new_parent,
                    hidden: false,
                    name,
                    tags,
                });
                created.groups.push(new_gid);
                let mut new_members = Vec::with_capacity(members.len());
                for m in members {
                    let child = self.clone_subtree(m, Some(new_gid), placement, created)?;
                    new_members.push(child);
                }
                self.groups[new_gid].members = new_members;
                Ok(NodeId::Group(new_gid))
            }
        }
    }

    /// Replace the span of `members` in group `pg`'s member list with the single
    /// node `replacement` at the position of the first member (group/instance
    /// fold-in). Inverse of [`Document::splice_out_parent`].
    fn splice_in_parent(&mut self, pg: GroupId, members: &[NodeId], replacement: NodeId) {
        let old = std::mem::take(&mut self.groups[pg].members);
        let mut new = Vec::with_capacity(old.len());
        let mut inserted = false;
        for n in old {
            if members.contains(&n) {
                if !inserted {
                    new.push(replacement);
                    inserted = true;
                }
            } else {
                new.push(n);
            }
        }
        self.groups[pg].members = new;
    }

    /// Replace the single node `node` in group `pg`'s member list with `members`
    /// (in order) — the inverse of [`Document::splice_in_parent`].
    fn splice_out_parent(&mut self, pg: GroupId, node: NodeId, members: &[NodeId]) {
        let old = std::mem::take(&mut self.groups[pg].members);
        let mut new = Vec::with_capacity(old.len() + members.len());
        for n in old {
            if n == node {
                new.extend_from_slice(members);
            } else {
                new.push(n);
            }
        }
        self.groups[pg].members = new;
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
    /// The kernel op the next [`Document::undo`] would reverse, when the
    /// pending document action is a per-object op (`None` otherwise or when
    /// there is nothing to undo). Mirrors [`History::peek_undo`].
    pub fn peek_undo_object_op(&self) -> Option<&KernelOp> {
        match self.undo.last()? {
            DocAction::ObjectOp { object } => self.objects.get(*object)?.history.peek_undo(),
            _ => None,
        }
    }

    /// The kernel op the next [`Document::redo`] would replay, when the
    /// pending document action is a per-object op. Mirrors
    /// [`History::peek_redo`].
    pub fn peek_redo_object_op(&self) -> Option<&KernelOp> {
        match self.redo.last()? {
            DocAction::ObjectOp { object } => self.objects.get(*object)?.history.peek_redo(),
            _ => None,
        }
    }

    pub fn undo(&mut self) -> Result<DocChange, DocumentError> {
        let action = self.undo.pop().ok_or(DocumentError::NothingToUndo)?;
        let change = match &action {
            DocAction::CreatedObject {
                id,
                sketch,
                region,
                consumed_edges,
                consumed_verts,
            } => {
                let (id, sketch, region) = (*id, *sketch, *region);
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.hidden = true;
                }
                self.consumed.remove(&(sketch, region));
                for &e in consumed_edges {
                    self.consumed_sketch_edges.remove(&(sketch, e));
                }
                for &v in consumed_verts {
                    self.consumed_sketch_verts.remove(&(sketch, v));
                }
                DocChange {
                    objects_touched: vec![id],
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::ObjectOp { object } => {
                let rec = &mut self.objects[object];
                // The object-level History keeps its entry when a dispatch is
                // refused; push the document action back too, or the two logs
                // desync and the next undo panics.
                if let Err(e) = rec.history.undo(&mut rec.object) {
                    self.undo.push(action);
                    return Err(map_history_err(e));
                }
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::Boolean { result, a, b } => {
                // Undo a combine: hide the result, bring the operands back.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = true;
                }
                self.objects[a].hidden = false;
                self.objects[b].hidden = false;
                DocChange {
                    objects_touched: vec![result, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::Sliced { source, a, b } => {
                // Undo a slice: hide both pieces, bring the source back.
                if let Some(rec) = self.objects.get_mut(a) {
                    rec.hidden = true;
                }
                if let Some(rec) = self.objects.get_mut(b) {
                    rec.hidden = true;
                }
                self.objects[source].hidden = false;
                DocChange {
                    objects_touched: vec![source, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::PushThrough { source, results } => {
                // Undo a push-through: hide the result pieces, bring the source back.
                let source = *source;
                let results = results.clone();
                for &r in &results {
                    if let Some(rec) = self.objects.get_mut(r) {
                        rec.hidden = true;
                    }
                }
                self.objects[source].hidden = false;
                let mut objects_touched = results;
                objects_touched.push(source);
                DocChange {
                    objects_touched,
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::Transform {
                objects, inverse, ..
            } => {
                // Undo a transform by baking its exact inverse into every target.
                for &obj in objects {
                    self.objects[obj]
                        .object
                        .apply_transform(inverse)
                        .expect("inverse of a validated transform must re-apply");
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::TransformSketch {
                sketch, inverse, ..
            } => {
                // Undo a sketch transform by baking its exact inverse.
                self.sketches[sketch]
                    .apply_transform(&inverse)
                    .expect("inverse of a validated transform must re-apply");
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::TransformSelection {
                objects,
                sketches,
                instances,
                inverse,
                ..
            } => {
                // Undo by baking the exact inverse into every baked target and
                // restoring every instance's exact prior pose.
                for &obj in objects {
                    self.objects[obj]
                        .object
                        .apply_transform(inverse)
                        .expect("inverse of a validated transform must re-apply");
                }
                for &s in sketches {
                    self.sketches[s]
                        .apply_transform(inverse)
                        .expect("inverse of a validated transform must re-apply");
                }
                for &(inst, prev) in instances {
                    self.instances[inst].pose = prev;
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: sketches.clone(),
                    groups_touched: Vec::new(),
                    instances_touched: instances.iter().map(|&(i, _)| i).collect(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::MovedSketchVertex {
                sketch,
                vertex,
                old_pos,
                ..
            } => {
                // Undo a vertex drag by moving it back. The reverse move is
                // topology-preserving by construction, so it cannot be refused.
                self.sketches[sketch]
                    .move_vertex(vertex, old_pos)
                    .expect("reverse of a validated vertex move must re-apply");
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::SketchGesture {
                sketch,
                before,
                created,
                ..
            } => {
                // Undo a drawing gesture: restore the exact pre-gesture
                // snapshot (keys preserved — every prior handle stays valid).
                // A gesture that created the sketch also hides it, so no
                // empty ghost lingers in the sketch list.
                let (sketch, created) = (*sketch, *created);
                if let Some(s) = self.sketches.get_mut(sketch) {
                    *s = (**before).clone();
                }
                if created {
                    self.hidden_sketches.insert(sketch);
                }
                DocChange {
                    sketches_touched: vec![sketch],
                    ..Default::default()
                }
            }
            DocAction::Grouped {
                group,
                parent,
                prev_parent_members,
            } => {
                // Undo grouping = dissolve: reparent members to the group's own
                // parent, restore that parent's order, hide the group.
                let (group, parent) = (*group, *parent);
                let members = self.groups[group].members.clone();
                for &m in &members {
                    self.set_node_parent(m, parent);
                }
                if let (Some(pg), Some(prev)) = (parent, prev_parent_members) {
                    self.groups[pg].members = prev.clone();
                }
                self.groups[group].hidden = true;
                group_change(group, parent, &members)
            }
            DocAction::Ungrouped {
                group,
                parent,
                prev_parent_members,
            } => {
                // Undo ungroup = re-form: reparent members back into the group,
                // restore the parent's order, unhide the group.
                let (group, parent) = (*group, *parent);
                self.groups[group].hidden = false;
                let members = self.groups[group].members.clone();
                for &m in &members {
                    self.set_node_parent(m, Some(group));
                }
                if let (Some(pg), Some(prev)) = (parent, prev_parent_members) {
                    self.groups[pg].members = prev.clone();
                }
                group_change(group, parent, &members)
            }
            DocAction::Deleted {
                node,
                parent,
                prev_parent_members,
                hidden_subtree,
            } => {
                // Undo delete = unhide exactly the hidden subtree and re-splice
                // `node` back into its parent at the original position.
                let (node, parent) = (*node, *parent);
                for &n in hidden_subtree {
                    match n {
                        NodeId::Object(id) => self.objects[id].hidden = false,
                        NodeId::Group(id) => self.groups[id].hidden = false,
                        NodeId::Instance(id) => self.instances[id].hidden = false,
                    }
                }
                if let (Some(pg), Some(prev)) = (parent, prev_parent_members) {
                    self.groups[pg].members = prev.clone();
                }
                delete_change(node, parent, hidden_subtree)
            }
            DocAction::MadeComponent {
                component,
                instance,
                parent,
                member_prior_parents,
                consumed_groups,
                prev_parent_members,
                ..
            } => {
                // Dissolve: return each def member to its prior world parent,
                // reveal the consumed groups, restore the parent's order, and
                // hide the now-empty definition + its instance.
                let (component, instance, parent) = (*component, *instance, *parent);
                for &(o, prior) in member_prior_parents {
                    self.objects[o].owner = ObjectOwner::World { parent: prior };
                }
                for &g in consumed_groups {
                    self.groups[g].hidden = false;
                }
                if let (Some(pg), Some(prev)) = (parent, prev_parent_members) {
                    self.groups[pg].members = prev.clone();
                }
                self.instances[instance].hidden = true;
                self.components[component].hidden = true;
                let leaves: Vec<ObjectId> = member_prior_parents.iter().map(|&(o, _)| o).collect();
                made_component_change(component, instance, parent, &leaves, consumed_groups)
            }
            &DocAction::PlacedInstance { instance } => {
                self.instances[instance].hidden = true;
                let def = self.instances[instance].def;
                DocChange {
                    instances_touched: vec![instance],
                    components_touched: vec![def],
                    ..Default::default()
                }
            }
            DocAction::Duplicated {
                root,
                parent,
                objects,
                groups,
                instances,
            } => {
                // Hide the whole clone and unlink its root from its parent.
                let (root, parent) = (*root, *parent);
                for &o in objects {
                    self.objects[o].hidden = true;
                }
                for &g in groups {
                    self.groups[g].hidden = true;
                }
                for &i in instances {
                    self.instances[i].hidden = true;
                }
                if let Some(pg) = parent {
                    self.groups[pg].members.retain(|&n| n != root);
                }
                let mut change = DocChange {
                    objects_touched: objects.clone(),
                    groups_touched: groups.clone(),
                    instances_touched: instances.clone(),
                    ..Default::default()
                };
                change.groups_touched.extend(parent);
                change
            }
            &DocAction::CreatedGuide { guide } => {
                if let Some(rec) = self.guides.get_mut(guide) {
                    rec.hidden = true;
                }
                DocChange {
                    guides_touched: vec![guide],
                    ..Default::default()
                }
            }
            &DocAction::DeletedGuide { guide } => {
                if let Some(rec) = self.guides.get_mut(guide) {
                    rec.hidden = false;
                }
                DocChange {
                    guides_touched: vec![guide],
                    ..Default::default()
                }
            }
            DocAction::DeletedGuides { guides } => {
                for &id in guides {
                    if let Some(rec) = self.guides.get_mut(id) {
                        rec.hidden = false;
                    }
                }
                DocChange {
                    guides_touched: guides.clone(),
                    ..Default::default()
                }
            }
            &DocAction::DeletedSketch { sketch } => {
                self.hidden_sketches.remove(&sketch);
                DocChange {
                    sketches_touched: vec![sketch],
                    ..Default::default()
                }
            }
            &DocAction::TransformInstance { instance, prev, .. } => {
                self.instances[instance].pose = prev;
                DocChange {
                    instances_touched: vec![instance],
                    ..Default::default()
                }
            }
            &DocAction::DefObjectOp { component, object } => {
                let rec = &mut self.objects[object];
                rec.history.undo(&mut rec.object).map_err(map_history_err)?;
                let instances_touched = self.instances_of(component);
                DocChange {
                    objects_touched: vec![object],
                    components_touched: vec![component],
                    instances_touched,
                    ..Default::default()
                }
            }
            DocAction::Exploded { instance, created } => {
                // Hide the baked world objects, bring the instance back, and
                // re-splice it into its parent in their place.
                let instance = *instance;
                for &o in created {
                    self.objects[o].hidden = true;
                }
                self.instances[instance].hidden = false;
                let parent = self.instances[instance].parent;
                if let Some(pg) = parent {
                    let nodes: Vec<NodeId> = created.iter().map(|&o| NodeId::Object(o)).collect();
                    self.splice_in_parent(pg, &nodes, NodeId::Instance(instance));
                }
                let mut change = DocChange {
                    objects_touched: created.clone(),
                    instances_touched: vec![instance],
                    ..Default::default()
                };
                change.groups_touched.extend(parent);
                change
            }
            &DocAction::MadeUnique {
                instance,
                prev_def,
                new_def,
            } => {
                self.instances[instance].def = prev_def;
                let new_members = self.components[new_def].members.clone();
                for o in new_members {
                    self.objects[o].hidden = true;
                }
                self.components[new_def].hidden = true;
                DocChange {
                    instances_touched: vec![instance],
                    components_touched: vec![prev_def, new_def],
                    ..Default::default()
                }
            }
            &DocAction::PaintFace {
                object, face, prev, ..
            } => {
                if let Some(f) = self
                    .objects
                    .get_mut(object)
                    .and_then(|r| r.object.faces.get_mut(face))
                {
                    f.material = prev;
                }
                self.paint_change(object)
            }
            &DocAction::SetObjectMaterial { object, prev, .. } => {
                if let Some(rec) = self.objects.get_mut(object) {
                    rec.object.default_material = prev;
                }
                self.paint_change(object)
            }
            &DocAction::SetMaterialAlpha { material, prev, .. } => {
                if let Some(mat) = self.materials.get_mut(material) {
                    mat.color.a = prev;
                }
                DocChange::default()
            }
            DocAction::NodeMetaChanged {
                node,
                prev_name,
                prev_tags,
                ..
            } => {
                // Undo: restore previous name and tags.
                let node = *node;
                self.apply_node_meta(node, prev_name.clone(), prev_tags.clone());
                self.node_change(node)
            }
            DocAction::Imported {
                objects,
                components,
                instances,
                groups,
                guides,
                tags,
                ..
            } => {
                // Undo import: hide every created entity (ids stay stable).
                // Materials added to the palette are not hidden. Tags this
                // import registered are unregistered.
                for (path, _) in tags.iter() {
                    self.tag_meta.remove(path);
                }
                for &oid in objects.iter() {
                    if let Some(rec) = self.objects.get_mut(oid) {
                        rec.hidden = true;
                    }
                }
                for &cid in components.iter() {
                    if let Some(c) = self.components.get_mut(cid) {
                        c.hidden = true;
                    }
                }
                for &iid in instances.iter() {
                    if let Some(rec) = self.instances.get_mut(iid) {
                        rec.hidden = true;
                    }
                }
                for &gid in groups.iter() {
                    if let Some(rec) = self.groups.get_mut(gid) {
                        rec.hidden = true;
                    }
                }
                for &guide in guides.iter() {
                    if let Some(rec) = self.guides.get_mut(guide) {
                        rec.hidden = true;
                    }
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: Vec::new(),
                    groups_touched: groups.clone(),
                    instances_touched: instances.clone(),
                    components_touched: components.clone(),
                    guides_touched: guides.clone(),
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
        let change = match &action {
            DocAction::CreatedObject {
                id,
                sketch,
                region,
                consumed_edges,
                consumed_verts,
            } => {
                let (id, sketch, region) = (*id, *sketch, *region);
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.hidden = false;
                }
                self.consumed.insert((sketch, region));
                for &e in consumed_edges {
                    self.consumed_sketch_edges.insert((sketch, e));
                }
                for &v in consumed_verts {
                    self.consumed_sketch_verts.insert((sketch, v));
                }
                DocChange {
                    objects_touched: vec![id],
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::ObjectOp { object } => {
                let rec = &mut self.objects[object];
                // Mirror undo: keep the two logs aligned on a refused replay.
                if let Err(e) = rec.history.redo(&mut rec.object) {
                    self.redo.push(action);
                    return Err(map_history_err(e));
                }
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::Boolean { result, a, b } => {
                // Redo a combine: hide the operands again, show the result.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = false;
                }
                self.objects[a].hidden = true;
                self.objects[b].hidden = true;
                DocChange {
                    objects_touched: vec![result, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::Sliced { source, a, b } => {
                // Redo a slice: hide the source again, show both pieces.
                self.objects[source].hidden = true;
                if let Some(rec) = self.objects.get_mut(a) {
                    rec.hidden = false;
                }
                if let Some(rec) = self.objects.get_mut(b) {
                    rec.hidden = false;
                }
                DocChange {
                    objects_touched: vec![source, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::PushThrough { source, results } => {
                // Redo a push-through: hide the source again, show the pieces.
                let source = *source;
                let results = results.clone();
                self.objects[source].hidden = true;
                for &r in &results {
                    if let Some(rec) = self.objects.get_mut(r) {
                        rec.hidden = false;
                    }
                }
                let mut objects_touched = results;
                objects_touched.push(source);
                DocChange {
                    objects_touched,
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::Transform {
                objects, forward, ..
            } => {
                // Redo a transform by re-baking the forward into every target.
                for &obj in objects {
                    self.objects[obj]
                        .object
                        .apply_transform(forward)
                        .expect("forward of a validated transform must re-apply");
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::TransformSketch {
                sketch, forward, ..
            } => {
                // Redo a sketch transform by re-baking the forward.
                self.sketches[sketch]
                    .apply_transform(&forward)
                    .expect("forward of a validated transform must re-apply");
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::TransformSelection {
                objects,
                sketches,
                instances,
                forward,
                ..
            } => {
                // Redo by re-baking the forward and re-composing each prior
                // pose with it — the same computation as the original
                // application, so the result is bit-identical.
                for &obj in objects {
                    self.objects[obj]
                        .object
                        .apply_transform(forward)
                        .expect("forward of a validated transform must re-apply");
                }
                for &s in sketches {
                    self.sketches[s]
                        .apply_transform(forward)
                        .expect("forward of a validated transform must re-apply");
                }
                for &(inst, prev) in instances.iter() {
                    self.instances[inst].pose = prev.then(forward);
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: sketches.clone(),
                    groups_touched: Vec::new(),
                    instances_touched: instances.iter().map(|&(i, _)| i).collect(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::MovedSketchVertex {
                sketch,
                vertex,
                new_pos,
                ..
            } => {
                // Redo a vertex drag by re-applying the new position.
                self.sketches[sketch]
                    .move_vertex(vertex, new_pos)
                    .expect("forward of a validated vertex move must re-apply");
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                    guides_touched: Vec::new(),
                }
            }
            DocAction::SketchGesture {
                sketch,
                after,
                created,
                ..
            } => {
                // Redo a drawing gesture: unhide first (when the gesture
                // created the sketch), then restore the post-gesture snapshot.
                let (sketch, created) = (*sketch, *created);
                if created {
                    self.hidden_sketches.remove(&sketch);
                }
                if let Some(s) = self.sketches.get_mut(sketch) {
                    *s = (**after).clone();
                }
                DocChange {
                    sketches_touched: vec![sketch],
                    ..Default::default()
                }
            }
            &DocAction::Grouped { group, parent, .. } => {
                // Redo grouping: re-form the group from its retained members.
                self.groups[group].hidden = false;
                let members = self.groups[group].members.clone();
                for &m in &members {
                    self.set_node_parent(m, Some(group));
                }
                if let Some(pg) = parent {
                    self.splice_in_parent(pg, &members, NodeId::Group(group));
                }
                group_change(group, parent, &members)
            }
            &DocAction::Ungrouped { group, parent, .. } => {
                // Redo ungroup: dissolve the group again.
                let members = self.groups[group].members.clone();
                for &m in &members {
                    self.set_node_parent(m, parent);
                }
                if let Some(pg) = parent {
                    self.splice_out_parent(pg, NodeId::Group(group), &members);
                }
                self.groups[group].hidden = true;
                group_change(group, parent, &members)
            }
            DocAction::Deleted {
                node,
                parent,
                hidden_subtree,
                ..
            } => {
                // Redo delete: re-hide the subtree and splice `node` out again.
                let (node, parent) = (*node, *parent);
                for &n in hidden_subtree {
                    match n {
                        NodeId::Object(id) => self.objects[id].hidden = true,
                        NodeId::Group(id) => self.groups[id].hidden = true,
                        NodeId::Instance(id) => self.instances[id].hidden = true,
                    }
                }
                if let Some(pg) = parent {
                    self.splice_out_parent(pg, node, &[]);
                }
                delete_change(node, parent, hidden_subtree)
            }
            DocAction::MadeComponent {
                component,
                instance,
                selected,
                parent,
                member_prior_parents,
                consumed_groups,
                ..
            } => {
                // Re-fold: re-own members as definition members, re-hide the
                // consumed groups, reveal the def + instance, and re-splice the
                // instance into the parent in the selection's place.
                let (component, instance, parent) = (*component, *instance, *parent);
                for &(o, _) in member_prior_parents {
                    self.objects[o].owner = ObjectOwner::Definition(component);
                }
                for &g in consumed_groups {
                    self.groups[g].hidden = true;
                }
                self.components[component].hidden = false;
                self.instances[instance].hidden = false;
                if let Some(pg) = parent {
                    self.splice_in_parent(pg, selected, NodeId::Instance(instance));
                }
                let leaves: Vec<ObjectId> = member_prior_parents.iter().map(|&(o, _)| o).collect();
                made_component_change(component, instance, parent, &leaves, consumed_groups)
            }
            &DocAction::PlacedInstance { instance } => {
                self.instances[instance].hidden = false;
                let def = self.instances[instance].def;
                DocChange {
                    instances_touched: vec![instance],
                    components_touched: vec![def],
                    ..Default::default()
                }
            }
            DocAction::Duplicated {
                root,
                parent,
                objects,
                groups,
                instances,
            } => {
                // Unhide the whole clone and re-append its root to its parent.
                let (root, parent) = (*root, *parent);
                for &o in objects {
                    self.objects[o].hidden = false;
                }
                for &g in groups {
                    self.groups[g].hidden = false;
                }
                for &i in instances {
                    self.instances[i].hidden = false;
                }
                if let Some(pg) = parent {
                    self.groups[pg].members.push(root);
                }
                let mut change = DocChange {
                    objects_touched: objects.clone(),
                    groups_touched: groups.clone(),
                    instances_touched: instances.clone(),
                    ..Default::default()
                };
                change.groups_touched.extend(parent);
                change
            }
            &DocAction::CreatedGuide { guide } => {
                if let Some(rec) = self.guides.get_mut(guide) {
                    rec.hidden = false;
                }
                DocChange {
                    guides_touched: vec![guide],
                    ..Default::default()
                }
            }
            &DocAction::DeletedGuide { guide } => {
                if let Some(rec) = self.guides.get_mut(guide) {
                    rec.hidden = true;
                }
                DocChange {
                    guides_touched: vec![guide],
                    ..Default::default()
                }
            }
            DocAction::DeletedGuides { guides } => {
                for &id in guides {
                    if let Some(rec) = self.guides.get_mut(id) {
                        rec.hidden = true;
                    }
                }
                DocChange {
                    guides_touched: guides.clone(),
                    ..Default::default()
                }
            }
            &DocAction::DeletedSketch { sketch } => {
                self.hidden_sketches.insert(sketch);
                DocChange {
                    sketches_touched: vec![sketch],
                    ..Default::default()
                }
            }
            &DocAction::TransformInstance { instance, next, .. } => {
                self.instances[instance].pose = next;
                DocChange {
                    instances_touched: vec![instance],
                    ..Default::default()
                }
            }
            &DocAction::DefObjectOp { component, object } => {
                let rec = &mut self.objects[object];
                rec.history.redo(&mut rec.object).map_err(map_history_err)?;
                let instances_touched = self.instances_of(component);
                DocChange {
                    objects_touched: vec![object],
                    components_touched: vec![component],
                    instances_touched,
                    ..Default::default()
                }
            }
            DocAction::Exploded { instance, created } => {
                let instance = *instance;
                self.instances[instance].hidden = true;
                for &o in created {
                    self.objects[o].hidden = false;
                }
                let parent = self.instances[instance].parent;
                if let Some(pg) = parent {
                    let nodes: Vec<NodeId> = created.iter().map(|&o| NodeId::Object(o)).collect();
                    self.splice_out_parent(pg, NodeId::Instance(instance), &nodes);
                }
                let mut change = DocChange {
                    objects_touched: created.clone(),
                    instances_touched: vec![instance],
                    ..Default::default()
                };
                change.groups_touched.extend(parent);
                change
            }
            &DocAction::MadeUnique {
                instance,
                prev_def,
                new_def,
            } => {
                self.components[new_def].hidden = false;
                let new_members = self.components[new_def].members.clone();
                for o in new_members {
                    self.objects[o].hidden = false;
                }
                self.instances[instance].def = new_def;
                DocChange {
                    instances_touched: vec![instance],
                    components_touched: vec![prev_def, new_def],
                    ..Default::default()
                }
            }
            &DocAction::PaintFace {
                object, face, next, ..
            } => {
                if let Some(f) = self
                    .objects
                    .get_mut(object)
                    .and_then(|r| r.object.faces.get_mut(face))
                {
                    f.material = next;
                }
                self.paint_change(object)
            }
            &DocAction::SetObjectMaterial { object, next, .. } => {
                if let Some(rec) = self.objects.get_mut(object) {
                    rec.object.default_material = next;
                }
                self.paint_change(object)
            }
            &DocAction::SetMaterialAlpha { material, next, .. } => {
                if let Some(mat) = self.materials.get_mut(material) {
                    mat.color.a = next;
                }
                DocChange::default()
            }
            DocAction::NodeMetaChanged {
                node,
                next_name,
                next_tags,
                ..
            } => {
                // Redo: re-apply next name and tags.
                let node = *node;
                self.apply_node_meta(node, next_name.clone(), next_tags.clone());
                self.node_change(node)
            }
            DocAction::Imported {
                objects,
                components,
                instances,
                groups,
                guides,
                tags,
                ..
            } => {
                // Redo import: unhide every created entity; re-register the
                // import's tags with their original hidden flags.
                for (path, hidden) in tags.iter() {
                    self.tag_meta.insert(path.clone(), *hidden);
                }
                for &oid in objects.iter() {
                    if let Some(rec) = self.objects.get_mut(oid) {
                        rec.hidden = false;
                    }
                }
                for &cid in components.iter() {
                    if let Some(c) = self.components.get_mut(cid) {
                        c.hidden = false;
                    }
                }
                for &iid in instances.iter() {
                    if let Some(rec) = self.instances.get_mut(iid) {
                        rec.hidden = false;
                    }
                }
                for &gid in groups.iter() {
                    if let Some(rec) = self.groups.get_mut(gid) {
                        rec.hidden = false;
                    }
                }
                for &guide in guides.iter() {
                    if let Some(rec) = self.guides.get_mut(guide) {
                        rec.hidden = false;
                    }
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: Vec::new(),
                    groups_touched: groups.clone(),
                    instances_touched: instances.clone(),
                    components_touched: components.clone(),
                    guides_touched: guides.clone(),
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
        // Torture mode (docs/DEVELOPMENT.md): run the topology validator on every
        // visible object after every op, **always-on** (release included), so a
        // corruption that slips past an op's own backstop surfaces here at the
        // exact op. The fuller debug-only invariant battery (tree/consumed)
        // follows below in debug builds.
        if self.torture {
            for (_, rec) in self.objects.iter().filter(|(_, r)| !r.hidden) {
                rec.object
                    .validate()
                    .expect("torture mode: document holds an invalid visible object");
            }
        }
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
            self.debug_validate_tree();
        }
    }

    /// Group-tree invariants: a consistent, acyclic forest. Each visible
    /// group lists distinct, live members that point back to it; each visible
    /// node's parent is a visible group that lists it; parent chains terminate.
    fn debug_validate_tree(&self) {
        for (gid, grec) in self.groups.iter().filter(|(_, r)| !r.hidden) {
            for (i, &m) in grec.members.iter().enumerate() {
                debug_assert!(
                    !grec.members[i + 1..].contains(&m),
                    "a group lists a member twice — kernel bug"
                );
                debug_assert!(
                    self.node_is_live(m),
                    "a group lists a stale/hidden member — kernel bug"
                );
                debug_assert_eq!(
                    self.node_parent(m),
                    Some(gid),
                    "a group member's parent disagrees with its container — kernel bug"
                );
            }
        }
        // Reverse direction: a parent pointer must lead to a visible group that
        // actually lists the child.
        let lists_child = |pg: GroupId, child: NodeId| {
            self.groups
                .get(pg)
                .is_some_and(|r| !r.hidden && r.members.contains(&child))
        };
        for (id, rec) in self.objects.iter().filter(|(_, r)| !r.hidden) {
            if let Some(pg) = rec.group_parent() {
                debug_assert!(
                    lists_child(pg, NodeId::Object(id)),
                    "an object's parent group does not list it — kernel bug"
                );
            }
        }
        // Instances: each visible instance's parent group (if any) must list it,
        // and its definition must be a live (non-hidden) component.
        for (id, rec) in self.instances.iter().filter(|(_, r)| !r.hidden) {
            if let Some(pg) = rec.parent {
                debug_assert!(
                    lists_child(pg, NodeId::Instance(id)),
                    "an instance's parent group does not list it — kernel bug"
                );
            }
            debug_assert!(
                self.components.get(rec.def).is_some_and(|c| !c.hidden),
                "an instance references a stale or hidden definition — kernel bug"
            );
        }
        // Definitions: each visible component's members are live objects owned
        // by exactly this definition.
        for (cid, def) in self.components.iter().filter(|(_, c)| !c.hidden) {
            for (i, &m) in def.members.iter().enumerate() {
                debug_assert!(
                    !def.members[i + 1..].contains(&m),
                    "a definition lists a member twice — kernel bug"
                );
                debug_assert_eq!(
                    self.objects.get(m).map(|r| r.owner),
                    Some(ObjectOwner::Definition(cid)),
                    "a definition member's owner disagrees with its definition — kernel bug"
                );
            }
        }
        for (id, rec) in self.groups.iter().filter(|(_, r)| !r.hidden) {
            if let Some(pg) = rec.parent {
                debug_assert!(
                    lists_child(pg, NodeId::Group(id)),
                    "a group's parent group does not list it — kernel bug"
                );
            }
            // Parent chain terminates (no cycle): bounded by the group count.
            let mut steps = 0;
            let mut cursor = rec.parent;
            while let Some(g) = cursor {
                steps += 1;
                debug_assert!(
                    steps <= self.groups.len(),
                    "a group parent cycle — kernel bug"
                );
                cursor = self.groups.get(g).and_then(|r| r.parent);
            }
        }
    }
}

// ─────────────────────────────────────── ingest helpers (module-level) ──────

/// Build one `MeshRecipe` into an `Object`, insert it, and tally stats.
/// Returns the `ObjectId` on success, or `None` + pushes `SkippedMesh` on
/// `TopologyError` (no silent repair — DEVELOPMENT.md rule 4).
#[allow(clippy::too_many_arguments)]
fn ingest_build_mesh(
    doc: &mut Document,
    recipe: crate::import::MeshRecipe,
    owner: ObjectOwner,
    all_objects: &mut Vec<ObjectId>,
    watertight_count: &mut usize,
    leaky_count: &mut usize,
    skipped: &mut Vec<crate::import::SkippedMesh>,
    dense_to_mat: &dyn Fn(u32) -> Option<MaterialId>,
) -> Option<ObjectId> {
    let face_mats: Vec<crate::material::FaceMaterial> = recipe
        .face_materials
        .iter()
        .map(|&d| dense_to_mat(d))
        .collect();
    // Propagate per-face UV frames from the recipe ( extension). If the
    // recipe's face_uv_frames is empty or short, pad with None.
    let face_uv_frames: Vec<Option<crate::material::UvFrame>> = (0..recipe.faces.len())
        .map(|i| recipe.face_uv_frames.get(i).copied().flatten())
        .collect();
    // Use the holes-aware import path. For non-holed meshes face_holes is all
    // empty vecs (byte-identical behaviour to the no-holes path).
    match Object::from_polygons_with_holes_import(
        &recipe.positions,
        &recipe.faces,
        &recipe.face_holes,
        &face_mats,
        &face_uv_frames,
    ) {
        Err(e) => {
            skipped.push(crate::import::SkippedMesh {
                name: recipe.name,
                reason: e.to_string(),
            });
            None
        }
        Ok(mut obj) => {
            obj.default_material = dense_to_mat(recipe.base_material);
            match obj.watertight() {
                WatertightState::Watertight => *watertight_count += 1,
                WatertightState::Open => *leaky_count += 1,
            }
            let oid = doc.objects.insert(ObjectRecord {
                object: obj,
                history: History::new(),
                hidden: false,
                owner,
                name: Some(recipe.name),
                tags: recipe.tags,
            });
            all_objects.push(oid);
            Some(oid)
        }
    }
}

/// Recursively build one `ImportNode` into the document tree, inserting objects,
/// groups, and instances into their respective slotmaps. Returns the created
/// `NodeId`, or `None` if the node was entirely skipped (all meshes failed, or
/// an `Instance` referencing a failed def).
#[allow(clippy::too_many_arguments)]
fn ingest_build_node(
    doc: &mut Document,
    node: crate::import::ImportNode,
    parent: Option<GroupId>,
    def_cid: &[Option<ComponentId>],
    all_objects: &mut Vec<ObjectId>,
    all_instances: &mut Vec<InstanceId>,
    all_groups: &mut Vec<GroupId>,
    watertight_count: &mut usize,
    leaky_count: &mut usize,
    skipped: &mut Vec<crate::import::SkippedMesh>,
    dense_to_mat: &dyn Fn(u32) -> Option<MaterialId>,
) -> Option<NodeId> {
    match node {
        crate::import::ImportNode::Mesh(recipe) => {
            let owner = ObjectOwner::World { parent };
            let oid = ingest_build_mesh(
                doc,
                recipe,
                owner,
                all_objects,
                watertight_count,
                leaky_count,
                skipped,
                dense_to_mat,
            )?;
            Some(NodeId::Object(oid))
        }
        crate::import::ImportNode::Instance {
            def,
            pose,
            name,
            tags,
            hidden,
        } => {
            let cid = def_cid.get(def).copied().flatten()?;
            let iid = doc.instances.insert(InstanceRecord {
                def: cid,
                pose,
                parent,
                hidden: false,
                // The placement's own name when the source carries one;
                // None resolves to the def's name (set on ComponentDef).
                name,
                tags,
            });
            if hidden {
                doc.user_hidden_instances.insert(iid);
            }
            all_instances.push(iid);
            Some(NodeId::Instance(iid))
        }
        crate::import::ImportNode::Group {
            name,
            children,
            tags,
            hidden,
        } => {
            let gid = doc.groups.insert(GroupRecord {
                members: Vec::new(),
                parent,
                hidden: false,
                name: if name.is_empty() { None } else { Some(name) },
                tags,
            });
            if hidden {
                doc.user_hidden_groups.insert(gid);
            }
            all_groups.push(gid);
            let mut members: Vec<NodeId> = Vec::new();
            for child in children {
                if let Some(nid) = ingest_build_node(
                    doc,
                    child,
                    Some(gid),
                    def_cid,
                    all_objects,
                    all_instances,
                    all_groups,
                    watertight_count,
                    leaky_count,
                    skipped,
                    dense_to_mat,
                ) {
                    members.push(nid);
                }
            }
            doc.groups[gid].members = members;
            Some(NodeId::Group(gid))
        }
    }
}

/// True if every coordinate of `p` is finite (no NaN/∞) — the no-silent-repair
/// guard for guide geometry: a non-finite input is rejected, never
/// clamped or guessed.
fn point_is_finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// True if every component of `v` is finite (no NaN/∞) — see
/// [`point_is_finite`]; `Vec3::normalized` alone does not reject a non-finite
/// input (a NaN length compares false against the minimum-length tolerance).
fn vec_is_finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// The [`DocChange`] for a group/ungroup: the group, its parent, and any member
/// groups changed structurally; member objects changed their top-level
/// container. The shim re-derives the rest from current [`Document`] state.
fn group_change(group: GroupId, parent: Option<GroupId>, members: &[NodeId]) -> DocChange {
    let mut groups_touched = vec![group];
    groups_touched.extend(parent);
    let mut objects_touched = Vec::new();
    let mut instances_touched = Vec::new();
    for &m in members {
        match m {
            NodeId::Object(o) => objects_touched.push(o),
            NodeId::Group(g) => groups_touched.push(g),
            NodeId::Instance(i) => instances_touched.push(i),
        }
    }
    DocChange {
        objects_touched,
        sketches_touched: Vec::new(),
        groups_touched,
        instances_touched,
        components_touched: Vec::new(),
        guides_touched: Vec::new(),
    }
}

/// The [`DocChange`] for `delete_node`/its undo/redo: every node in the hidden
/// (or re-hidden/unhidden) subtree changed visibility, plus the shared parent's
/// membership changed. `subtree` already includes `node` itself (it is the
/// first element collected by [`Document::collect_subtree`]).
fn delete_change(node: NodeId, parent: Option<GroupId>, subtree: &[NodeId]) -> DocChange {
    let mut groups_touched = Vec::new();
    groups_touched.extend(parent);
    let mut objects_touched = Vec::new();
    let mut instances_touched = Vec::new();
    for &n in subtree {
        match n {
            NodeId::Object(o) => objects_touched.push(o),
            NodeId::Group(g) => groups_touched.push(g),
            NodeId::Instance(i) => instances_touched.push(i),
        }
    }
    // `node` itself is always in `subtree` (collect_subtree's first push), but
    // guard the invariant explicitly in case that ever changes.
    debug_assert!(
        subtree.contains(&node),
        "delete_change: node not in its own subtree"
    );
    DocChange {
        objects_touched,
        sketches_touched: Vec::new(),
        groups_touched,
        instances_touched,
        components_touched: Vec::new(),
        guides_touched: Vec::new(),
    }
}

/// The [`DocChange`] for `make_component`/its undo: the folded leaf objects
/// changed owner (they leave / rejoin the world-object set), the consumed groups
/// and the shared parent changed visibility/membership, and the new definition +
/// instance appeared/disappeared. The shim re-derives the details from current
/// [`Document`] state.
fn made_component_change(
    component: ComponentId,
    instance: InstanceId,
    parent: Option<GroupId>,
    leaves: &[ObjectId],
    consumed_groups: &[GroupId],
) -> DocChange {
    let mut groups_touched = consumed_groups.to_vec();
    groups_touched.extend(parent);
    DocChange {
        objects_touched: leaves.to_vec(),
        sketches_touched: Vec::new(),
        groups_touched,
        instances_touched: vec![instance],
        components_touched: vec![component],
        guides_touched: Vec::new(),
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
