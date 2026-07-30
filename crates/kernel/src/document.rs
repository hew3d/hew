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
//!   Undo also restores the sketch scaffolding the extrusion deleted
//!   (Model D, the sketch-solid-model design).
//! - **A per-Object op** ([`DocAction::ObjectOp`]) — undo/redo delegate to that
//!   Object's [`History`].
//!
//! Each mutation returns a [`DocChange`] naming the entities it touched, so the
//! shim can reconcile inference candidates and render caches precisely without
//! the kernel knowing those concerns exist.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use slotmap::SlotMap;
use tracing::info;

use crate::camera::CameraState;
use crate::guide::Guide;
use crate::history::{History, HistoryError, KernelOp, KernelOpError, KernelOpReport};
use crate::ids::{
    ComponentId, FaceId, GroupId, GuideId, InstanceId, MaterialId, ObjectId, SketchId,
};
use crate::import::{ImportReport, ImportScene, SkippedMesh};
use crate::material::Material;
use crate::math::{MathError, Plane, Point3, Vec3};
use crate::ops::{BooleanError, BooleanOp, ExtrudeError, FollowMeError, Operand, SliceError};
use crate::serialize::{DocSaveData, LoadError, NodeRefDto, decode_document_raw, encode_document};
use crate::sketch::{
    CurveGeom, Sketch, SketchCurveId, SketchEdgeId, SketchError, SketchIslandId, SketchRegionId,
    SketchVertexId,
};
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

/// One node's tag-list transition inside [`DocAction::TagDeleted`]:
/// `(node, tags before, tags after)`.
type TagListTransition = (NodeId, Vec<Vec<String>>, Vec<Vec<String>>);

/// One document-level step on the undo stack.
///
/// Object creation is undone by hiding (not deleting), so the `ObjectId` never
/// churns — redo just unhides, and a later `ObjectOp` still refers to a live
/// handle.
// `Transform` carries f64s, so this is `PartialEq` but not `Eq`; the `Vec`
// fields (transform targets, grouped membership) make it non-`Copy`.
#[derive(Debug, Clone, PartialEq)]
enum DocAction {
    /// Several ordinary actions committed as one user gesture. Children stay
    /// in their original commit order; undo applies them in reverse and redo
    /// in forward order. Used where a component-edit selection spans object
    /// members and definition-owned sketch islands.
    Compound { actions: Vec<DocAction> },
    /// `extrude_region` created an Object from a sketch region and DELETED
    /// the region's scaffolding from the sketch (Model D,
    /// the sketch-solid-model design: the sketch is the larval form of
    /// the solid — the extruded outline becomes the solid's base face and
    /// ceases to exist as sketch geometry). Undo hides the Object and
    /// RE-INSERTS the deleted scaffolding ([`Sketch::restore_edges`]) in
    /// the same step — solid gone, scaffolding back, atomically — merging
    /// with whatever the sketch holds by then, so edits made after the
    /// extrusion survive its undo (a whole-sketch snapshot would clobber
    /// them). If later geometry crosses where the outline was, the undo
    /// fails typed ([`SketchError::RestoreConflicts`]) and touches
    /// nothing. Restored edges carry fresh handles (a slotmap cannot
    /// re-mint a key); callers re-query, as after any reshaping mutation.
    CreatedObject {
        id: ObjectId,
        /// The sketch the extrusion consumed geometry from.
        sketch: SketchId,
        /// The deleted scaffolding as rows of (endpoint, endpoint,
        /// curve-chain id) — everything undo needs to re-insert it, and
        /// everything redo needs to re-delete it BY GEOMETRY
        /// ([`Sketch::edge_at_positions`]): an interleaved gesture
        /// undo/redo on the same sketch restores snapshots carrying the
        /// outline's original edge ids, so an id set would go stale where
        /// the geometry itself stays exact.
        removed: Vec<(Point3, Point3, Option<SketchCurveId>)>,
        /// The deletion emptied the sketch, so the extrusion also removed
        /// the sketch itself from view (it fully became the solid). Undo
        /// brings it back; redo removes it again.
        emptied: bool,
        /// A merged sweep ([`Document::follow_me_merged`]) consumed the
        /// path's own solid into `id` exactly as a boolean operand. Undo
        /// restores it alongside the scaffolding; redo consumes it again.
        /// `None` for every plain extrusion/sweep.
        merged_base: Option<ObjectId>,
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
    /// `boolean_nodes` combined two tree nodes (each a plain Object or a
    /// whole Group; the group-ops design) into a result of one Object per
    /// connected volume — several arriving inside a fresh result group. Undo
    /// hides the result (objects + container group) and unhides exactly
    /// `hidden_operands`; redo reverses. Pure visibility flipping, all
    /// handles stable (hide-not-delete) — nothing is recomputed on replay.
    BooleanNodes {
        /// The first operand's root node.
        a: NodeId,
        /// The second operand's root node.
        b: NodeId,
        /// Every node hidden by consuming the operands (both subtrees,
        /// pre-order), so undo unhides exactly this set and nothing else.
        hidden_operands: Vec<NodeId>,
        /// The result pieces (one per connected component of the result).
        result_objects: Vec<ObjectId>,
        /// The result container group, present iff there was more than one
        /// piece.
        result_group: Option<GroupId>,
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
    /// A move/rotate baked into ONE island of a sketch (per-island Move).
    /// Undo bakes `inverse`, redo bakes `forward`; the island id is stable
    /// across the transform (its edge set never changes) and therefore
    /// across undo/redo too.
    TransformSketchIsland {
        sketch: SketchId,
        island: SketchIslandId,
        forward: Transform,
        inverse: Transform,
        /// The island's canonical anchor BEFORE the transform (redo's key)
        /// and AFTER it (undo's key). Island ids are NOT stable across a
        /// consume-and-restore cycle (a sweep or extrusion eats the
        /// island's edges; undoing restores them under fresh ids), so
        /// undo/redo re-resolve the island BY GEOMETRY when the stored id
        /// is stale — the `restore_edges` philosophy applied to islands.
        anchor_before: Point3,
        anchor_after: Point3,
    },
    /// An OUT-OF-PLANE island transform detached the island into its own
    /// new sketch (a sketch is planar; an island leaving the plane cannot
    /// stay — see [`Document::transform_sketch_island`]). `removed` holds
    /// the island's edges at their PRE-transform source positions, in the
    /// [`DocAction::CreatedObject`] row shape and for the same reason: undo
    /// re-inserts them into the source by merging with whatever it holds by
    /// then ([`Sketch::restore_edges`]; later drawing in the way is a typed
    /// [`SketchError::RestoreConflicts`] refusal that touches nothing) and
    /// hides the detached sketch; redo re-removes them BY GEOMETRY
    /// ([`Sketch::edge_at_positions`]) and unhides the detached sketch,
    /// whose contents survive hiding bit-exactly (hide-not-delete).
    DetachedSketchIsland {
        source: SketchId,
        detached: SketchId,
        removed: Vec<(Point3, Point3, Option<SketchCurveId>)>,
    },
    /// An OUT-OF-PLANE sketch COPY (Move+Alt off the sketch plane) built a
    /// NEW sketch holding one or more of the source's islands with the
    /// transform baked in, leaving the SOURCE untouched — the detach's twin
    /// without the source removal (see [`Document::copy_sketch_islands`]).
    /// Undo hides the copy; redo unhides it. The copy's contents survive
    /// hiding bit-exactly (hide-not-delete), so the `SketchId` is
    /// handle-stable across undo/redo. Unlike
    /// [`DocAction::DetachedSketchIsland`] there is no source outline to
    /// restore: undo is a pure visibility flip that can never fail.
    CopiedSketchIslands { copy: SketchId },
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
    /// `duplicate_nodes_array` (the Move tool's ×N / /N array copy)
    /// deep-cloned a selection `count` times along a step transform, as **one
    /// action**. Undo hides every created entity and removes each clone root
    /// from its parent's member list; redo unhides and re-appends the roots in
    /// creation order. All handles stay stable (hide-not-delete), mirroring
    /// [`DocAction::Duplicated`] element-wise.
    DuplicatedArray {
        /// Every clone root paired with the parent group it was appended to
        /// (`None` at top level), in creation order.
        roots: Vec<(NodeId, Option<GroupId>)>,
        /// Every world object created across all clones.
        objects: Vec<ObjectId>,
        /// Every group created across all clones.
        groups: Vec<GroupId>,
        /// Every instance created across all clones.
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
    /// objects (`created`) and, per live def-owned sketch the definition
    /// held (component-edit-parity.md phase K1 follow-up), an independent
    /// WORLD sketch (`created_sketches`) — the sketch analog of `created`,
    /// so a not-yet-extruded profile drawn into the component does not
    /// silently disappear from the exploded result. Undo hides both and
    /// unhides the instance; redo reverses. The definition, its own
    /// sketches, and sibling instances are untouched throughout — explode
    /// COPIES shared content into independent geometry, it never moves it.
    Exploded {
        instance: InstanceId,
        created: Vec<ObjectId>,
        created_sketches: Vec<SketchId>,
    },
    /// `make_unique` repointed an instance from its shared definition onto
    /// a fresh private copy. Undo repoints to `prev_def` and hides `new_def`;
    /// redo reverses.
    MadeUnique {
        instance: InstanceId,
        prev_def: ComponentId,
        new_def: ComponentId,
        /// The instance's own name at the moment of the op. A set name is
        /// promoted to the new definition's name and cleared off the
        /// instance, so undo restores it and redo re-clears it (an unnamed
        /// instance round-trips as a no-op either way).
        prev_instance_name: Option<String>,
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
    /// `set_component_name` changed a component definition's display name —
    /// the shared label every instance of that definition shows. Undo
    /// restores `prev_name`; redo re-applies `next_name`. Handle-stable (the
    /// definition is only annotated).
    ComponentRenamed {
        component: ComponentId,
        prev_name: Option<String>,
        next_name: Option<String>,
    },
    /// `delete_tag` removed a tag path (and every registered tag nested under
    /// it) from the document: unregistered from the tag metadata registry and
    /// unassigned from every visible node that carried it. Geometry is never
    /// touched. Undo restores the registry entries — including their
    /// hidden-by-default flags, except for a path the user re-registered
    /// after the delete, whose fresh (non-undoable) flag wins — and every
    /// affected node's previous tag list; redo re-applies the stripped lists
    /// and removes the entries again.
    TagDeleted {
        /// Registry entries removed: `(path, hidden flag)`, for exact restore.
        registry: Vec<(Vec<String>, bool)>,
        /// Per-node tag lists: `(node, tags before, tags after)`.
        nodes: Vec<TagListTransition>,
    },
    /// [`Document::follow_me_face`] swept a solid FACE profile into a new
    /// object (design §3a). The source solid is untouched unless the sweep
    /// MERGED with it — the profile face belonged to the path's own solid
    /// (design §3b) — in which case `merged_base` is that solid, consumed
    /// into the result exactly as a boolean operand. Undo hides the result
    /// and restores the base; redo re-applies both.
    FollowMeFace {
        result: ObjectId,
        merged_base: Option<ObjectId>,
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
    /// `delete_def_member` hid one object from a component definition —
    /// component-edit-parity.md phase K1. Unlike [`DocAction::Deleted`] on a
    /// world object, `ComponentDef.members` is a flat geometry bag, not a
    /// tree structure — the invariant checker requires every listed member's
    /// owner to agree with its definition, but never that it be *live*
    /// (`def_members` already documents returning hidden members too, and
    /// callers already skip them gracefully) — so the member simply stays
    /// listed, hidden, forever, exactly like an object birth that was
    /// undone. Undo/redo therefore only flip `hidden`; `component` is
    /// carried for the [`DocChange`], not for any list surgery.
    DeletedDefMember {
        component: ComponentId,
        object: ObjectId,
    },
    /// [`Document::place_text`]'s final cleanup: after every selected FILL
    /// region has been extruded, any edges still standing in the sketch
    /// belong to un-extruded counter/interior regions — the holes in
    /// glyphs like 'O'/'D'/'e', never anyone's scaffolding, since only the
    /// fill regions the caller selected were extruded
    /// ([`DocAction::CreatedObject`] only ever removes ITS region's own
    /// boundary). `place_text`'s sketch exists solely for that one
    /// placement (docs/design/3d-text.md), so discarding whatever remains
    /// loses nothing legitimate: the whole sketch retires with the
    /// placement instead of lingering as a live, reachable scratch entity.
    /// Mirrors [`DocAction::CreatedObject`]'s own scaffolding
    /// removal/restoration exactly (same row shape, same
    /// [`Sketch::restore_edges`]/[`Sketch::edge_at_positions`] mechanics)
    /// but names no [`ObjectId`] — nothing is extruded, only discarded.
    /// Only ever produced when `place_text` finds the sketch still live
    /// after its region loop; a glyph run with no counters produces none
    /// (the last region's own extrusion already emptied the sketch).
    ConsumedScaffolding {
        sketch: SketchId,
        /// The discarded rows, in [`DocAction::CreatedObject::removed`]'s
        /// exact shape — everything undo needs to re-insert them and redo
        /// needs to re-delete them by geometry.
        removed: Vec<(Point3, Point3, Option<SketchCurveId>)>,
    },
    /// [`Document::place_text`] (3D Text, docs/design/3d-text.md) bundles
    /// several ordinary steps into ONE undo/redo entry: the glyph-injection
    /// gesture that preceded it ([`DocAction::SketchGesture`]), one
    /// [`DocAction::CreatedObject`] per extruded region, an optional
    /// [`DocAction::ConsumedScaffolding`] that discards any counter-glyph
    /// scaffolding the regions left behind, and the
    /// [`DocAction::MadeComponent`] that folds the extruded objects into a
    /// definition plus one instance — in that chronological order. Undo
    /// reverses the list in REVERSE order (last extrusion first, gesture
    /// last); redo replays it forward. Scoped to exactly this shape —
    /// `place_text` is its only producer — rather than the fully general
    /// [`DocAction::Compound`] nested-action mechanism (component-edit
    /// selections spanning object members and def-owned sketch islands):
    /// this variant predates that one and keeps its own dedicated,
    /// pre-validating reversal path (see
    /// `Document::undo_place_text_compound`/`redo_place_text_compound` and
    /// `compound_reversal_feasible`) rather than folding into it.
    PlaceTextCompound(Vec<DocAction>),
}

/// The coarse shape of a pending [`DocAction`], exposed to callers that need
/// to distinguish action KINDS without matching the (private) `DocAction`
/// type itself — see [`Document::peek_undo_action_kind`]. Deliberately not
/// exhaustive of every `DocAction` variant: it exists only to let a guard
/// narrow "which action is this refusal against", so anything not called
/// out by name collapses into `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingActionKind {
    /// [`DocAction::PlaceTextCompound`] — `place_text`'s bundled placement.
    PlaceTextCompound,
    /// [`DocAction::CreatedObject`] — a plain (non-bundled) extrusion.
    CreatedObject,
    /// Any other action kind.
    Other,
}

impl DocAction {
    fn kind(&self) -> PendingActionKind {
        match self {
            DocAction::PlaceTextCompound(_) => PendingActionKind::PlaceTextCompound,
            DocAction::CreatedObject { .. } => PendingActionKind::CreatedObject,
            _ => PendingActionKind::Other,
        }
    }
}

/// A Follow Me path source (the follow-me design §2): either a chain of
/// sketch edges or a solid face's outer boundary loop. Resolved by
/// [`Document::follow_me`]; the path source is never consumed or modified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowMePath {
    /// Edges of one visible sketch forming a single connected chain (open
    /// or closed), in any order.
    SketchEdges {
        /// The sketch owning the edges.
        sketch: SketchId,
        /// The path edges.
        edges: Vec<SketchEdgeId>,
    },
    /// The outer boundary loop of one face of a visible world object —
    /// always a closed path (crown molding around a tabletop). The object
    /// itself is untouched.
    FaceLoop {
        /// The path object.
        object: ObjectId,
        /// The face whose outer boundary is the path.
        face: FaceId,
    },
    /// The outer boundary loop of a face reached THROUGH a component
    /// instance (design §2e): the definition member's loop mapped through
    /// the instance's pose into world space. The instance and its
    /// definition are untouched; a reflected pose (determinant < 0) is
    /// refused typed — the mirrored loop would sweep with inverted
    /// winding.
    InstanceFaceLoop {
        /// The placed instance the face was picked through.
        instance: InstanceId,
        /// The definition-member object owning the face.
        object: ObjectId,
        /// The face whose outer boundary is the path.
        face: FaceId,
    },
}

/// A resolved Follow Me path: the ordered polyline, whether it closes, and
/// each walked segment's analytic [`CurveGeom`] attribution in the same
/// order (the wrap segment last; empty when nothing is attributed).
type ResolvedFollowMePath = (Vec<Point3>, bool, Vec<Option<CurveGeom>>);

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

fn merge_unique<T: Copy + Ord>(into: &mut Vec<T>, from: Vec<T>) {
    for value in from {
        if !into.contains(&value) {
            into.push(value);
        }
    }
}

fn merge_doc_change(into: &mut DocChange, from: DocChange) {
    merge_unique(&mut into.objects_touched, from.objects_touched);
    merge_unique(&mut into.sketches_touched, from.sketches_touched);
    merge_unique(&mut into.groups_touched, from.groups_touched);
    merge_unique(&mut into.instances_touched, from.instances_touched);
    merge_unique(&mut into.components_touched, from.components_touched);
    merge_unique(&mut into.guides_touched, from.guides_touched);
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
    /// `explode_instance` was called on an instance whose definition holds a
    /// live def-owned sketch (component-edit-parity.md phase K1 follow-up),
    /// but the instance's pose is not a similarity (non-uniform scale).
    /// Baking the sketch into an independent world copy through such a pose
    /// would force `Sketch::apply_transform`'s map-or-drop contract to DROP
    /// every curve's analytic identity (a circle would become an
    /// unrepresentable ellipse) — explode refuses typed instead of silently
    /// degrading a profile's fidelity. Unmirror is not the issue here (see
    /// `CannotExplodeReflected` for that); the fix is to even out the
    /// instance's scale first, or extrude/finish the sketch before exploding.
    CannotExplodeNonUniformScale,
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
    /// A [`Document::boolean_nodes`] operand is, or contains, a component
    /// instance. A boolean consumes its operand, and an instance's geometry is
    /// *shared* — consuming it would either mutate every sibling instance or
    /// hide an implicit Make Unique inside another verb (the exact implicit
    /// magic Hew refuses; the group-ops design §2.2). Explode the
    /// instance (or Make Unique, then Explode) first.
    BooleanOperandHasInstance,
    /// A leaf object under a [`Document::boolean_nodes`] operand is not a
    /// watertight solid. Booleans are volume algebra; a leaky leaf anywhere
    /// under an operand refuses the whole op, naming the offending side.
    BooleanOperandNotSolid {
        /// Which operand holds the non-solid leaf.
        which: Operand,
    },
    /// A [`Document::boolean_nodes`] operand contains no solid objects at all
    /// (defensive: the tree normally cannot produce an empty group).
    BooleanOperandEmpty,
    /// A sketch operation (region lookup / profile tracing) failed.
    Sketch(SketchError),
    /// Extruding the region into a solid failed.
    Extrude(ExtrudeError),
    /// A Follow Me sweep failed (path resolution or the sweep itself).
    FollowMe(FollowMeError),
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
    /// A replayed per-Object inverse/redo ran but did not reproduce the
    /// recorded state (rule 9 proof failure) — a kernel bug, surfaced loudly;
    /// the object is untouched.
    InverseDiverged,
    /// A geometry-creating in-instance op (`extrude_region_in_instance`) was
    /// given a typed world-space distance to map through an instance pose
    /// that is not a similarity (rotation × uniform scale, mirror allowed) —
    /// a non-uniformly-scaled instance. A single scalar distance cannot map
    /// unambiguously through a non-uniform scale (component-edit-parity.md,
    /// "Coordinate mapping"): refused typed rather than guessing an axis.
    AmbiguousInstanceScale,
    /// `delete_def_member` was asked to delete a component definition's last
    /// remaining member. SketchUp deletes the now-empty component outright;
    /// v1 refuses instead — deleting the last member means the user wants
    /// the instances gone, so the hint points at deleting those.
    LastDefinitionMember,
    /// `place_text` expects to fold the caller's just-closed glyph-injection
    /// gesture (`begin_sketch_gesture` … `end_sketch_gesture` on the same
    /// sketch, nothing else committed in between) into its own compound
    /// undo step; the top of the undo stack was not that gesture — a
    /// caller-contract violation. The document is untouched.
    UnexpectedGestureState,
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
            DocumentError::CannotExplodeNonUniformScale => write!(
                f,
                "cannot explode an instance with a live in-progress sketch through a \
                 non-uniformly-scaled pose — the sketch's curves cannot map exactly; \
                 even out the instance's scale first, or extrude/finish the sketch before exploding"
            ),
            DocumentError::DuplicateMember => write!(f, "a node was listed twice in a group"),
            DocumentError::MixedParents => {
                write!(f, "only sibling nodes (sharing one parent) can be grouped")
            }
            DocumentError::GroupedOperand => write!(
                f,
                "cannot combine, slice, or push-through an object inside a group — ungroup it first"
            ),
            DocumentError::BooleanOperandHasInstance => write!(
                f,
                "cannot combine a component instance — explode it (or make it unique, then explode) first"
            ),
            DocumentError::BooleanOperandNotSolid { which } => {
                let side = match which {
                    Operand::A => "first",
                    Operand::B => "second",
                };
                write!(
                    f,
                    "an object in the {side} selection is not a watertight solid"
                )
            }
            DocumentError::BooleanOperandEmpty => {
                write!(f, "the selection contains no solids to combine")
            }
            DocumentError::Sketch(e) => write!(f, "{e}"),
            DocumentError::Extrude(e) => write!(f, "{e}"),
            DocumentError::FollowMe(e) => write!(f, "{e}"),
            DocumentError::Boolean(e) => write!(f, "{e}"),
            DocumentError::Slice(e) => write!(f, "{e}"),
            DocumentError::Transform(e) => write!(f, "{e}"),
            DocumentError::Op(e) => write!(f, "{e}"),
            DocumentError::NothingToUndo => write!(f, "nothing to undo"),
            DocumentError::NothingToRedo => write!(f, "nothing to redo"),
            DocumentError::InverseFailed(e) => write!(f, "inverse op failed (kernel bug): {e}"),
            DocumentError::InverseDiverged => {
                write!(
                    f,
                    "replayed op diverged from the recorded state (kernel bug)"
                )
            }
            DocumentError::AmbiguousInstanceScale => write!(
                f,
                "cannot map a typed distance through a non-uniformly-scaled instance — \
                 the world length is ambiguous per axis; drag instead of typing, or unscale the instance"
            ),
            DocumentError::LastDefinitionMember => write!(
                f,
                "cannot delete a component definition's last member — delete its instances instead"
            ),
            DocumentError::UnexpectedGestureState => write!(
                f,
                "expected the just-closed sketch-drawing gesture at the top of the undo stack"
            ),
        }
    }
}

impl std::error::Error for DocumentError {}

/// One side of the document undo/redo log, counting every push it has ever
/// taken. `pushes` is monotonic — pops and clears never decrement it — so the
/// two sides' counts sum to [`Document::history_generation`]: a token that
/// changes on every recorded action, every undo, and every redo, but never on
/// non-undoable view-state edits. The UI uses it as an undo-stack identity
/// check (is the action I committed provably still on top?), which a content
/// hash cannot answer: a net-zero pair of undoable edits restores the content
/// hash while displacing the stack top.
#[derive(Debug, Clone, Default)]
struct ActionStack {
    actions: Vec<DocAction>,
    /// Total pushes ever accepted (monotonic).
    pushes: u64,
}

impl ActionStack {
    fn push(&mut self, action: DocAction) {
        self.actions.push(action);
        self.pushes += 1;
    }
    fn pop(&mut self) -> Option<DocAction> {
        self.actions.pop()
    }
    fn clear(&mut self) {
        self.actions.clear();
    }
    fn last(&self) -> Option<&DocAction> {
        self.actions.last()
    }
    fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

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
    /// Sketches hidden by [`Document::delete_sketch`] (tombstone, not a real
    /// delete — the id stays valid for redo). A document-level visibility
    /// concern, not a field on [`Sketch`] itself, mirroring how object/group/
    /// instance visibility lives on their `*Record` wrappers rather than the
    /// payload type.
    hidden_sketches: BTreeSet<SketchId>,
    /// The `SketchOwner` of every DEFINITION-owned sketch (component-edit-
    /// parity.md phase K1): sketch id → the owning definition. Absence means
    /// `SketchOwner::World` — the common case, and every sketch before this
    /// phase. A side table, not a wrapper record on [`Sketch`] itself,
    /// mirroring how `hidden_sketches`/`fresh_sketches` track exceptional
    /// per-sketch state without touching every `self.sketches` call site.
    /// Populated by [`Document::begin_sketch_on_plane_in_instance`] and
    /// consulted by [`Document::sketch_ids`] (world-only — a def-owned
    /// sketch reaches the scene only through its definition's instances,
    /// exactly like [`ObjectOwner::Definition`] members are excluded from
    /// [`Document::visible_object_ids`]) and [`Document::sketch_owner_component`].
    /// Persisted as the sketch record's `owner` field (manifest v13+).
    ///
    /// `make_unique` deep-copies each LIVE entry mapped to the source
    /// definition into a fresh one owned by the private copy (mirroring how
    /// it copies `members`); `explode_instance` copies each into an
    /// independent WORLD sketch baked through the instance's pose (mirroring
    /// how it bakes `members`), refusing typed under a non-uniform-scale
    /// pose rather than dropping the copy's curve identity. Both leave a
    /// sketch already consumed into a solid (hidden — Model D's larval-form
    /// husk) on the ORIGINAL definition only: nothing in a copy or an
    /// exploded instance can ever reference it.
    def_sketches: BTreeMap<SketchId, ComponentId>,
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
    /// The working camera view at last save (manifest v13+; docs/design/
    /// camera.md §5): `None` for a document that never had one saved (every
    /// pre-v13 file, and a brand-new in-memory `Document` before the app's
    /// first `set_camera_state`), which the app reads as "use today's home
    /// framing". Deliberately NOT undoable — see `camera.rs`'s module doc —
    /// so it sits beside `tag_meta`/`user_hidden_*` rather than going through
    /// `undo`/`redo`.
    camera: Option<CameraState>,
    undo: ActionStack,
    redo: ActionStack,
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
        // `ComponentDef.members` may list a HIDDEN object (component-edit-
        // parity.md phase K1: an undone `extrude_region_in_instance` birth,
        // or a `delete_def_member`) — unlike a `GroupRecord`'s members, whose
        // invariant forbids a hidden entry (debug_validate_tree), a
        // definition's member list tolerates one (def_members' own doc
        // comment; downstream readers already skip hidden ids gracefully).
        // Save persists only LIVE state (undo/redo tombstones are dropped,
        // matching every other entity), so hidden members are filtered out
        // here — otherwise a stale id would reach `encode_document` with no
        // corresponding live geometry buffer to resolve against.
        let components: Vec<(ComponentId, Vec<ObjectId>, Option<String>)> = self
            .components
            .iter()
            .filter(|(_, c)| !c.hidden)
            .map(|(id, c)| {
                let live_members: Vec<ObjectId> = c
                    .members
                    .iter()
                    .copied()
                    .filter(|&o| self.objects.get(o).is_some_and(|r| !r.hidden))
                    .collect();
                (id, live_members, c.name.clone())
            })
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

        // ── Per-sketch `SketchOwner` (manifest v13+), live sketches only ───
        let sketch_owner: BTreeMap<SketchId, ComponentId> = self
            .def_sketches
            .iter()
            .filter(|(id, _)| !self.hidden_sketches.contains(id))
            .map(|(&id, &cid)| (id, cid))
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
            sketch_owner,
            guides,
            roots,
            obj_names,
            obj_tags,
            tag_meta,
            obj_hidden: self.user_hidden_objects.clone(),
            group_hidden: self.user_hidden_groups.clone(),
            instance_hidden: self.user_hidden_instances.clone(),
            camera: self.camera,
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

        // ── 3. Insert sketches → dense→SketchId map (consumed resolution) ──
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

        // ── 4b. `SketchOwner` (manifest v13+): now that `comp_ids` exists,
        // resolve each sketch's owning dense component id to a live
        // ComponentId. `raw.sketch_owner` is `None` for every sketch in a
        // pre-v13 file (world-owned, the only possibility then).
        for (i, owner_dense) in raw.sketch_owner.iter().enumerate() {
            if let Some(cd) = owner_dense {
                let cid =
                    *comp_ids
                        .get(*cd as usize)
                        .ok_or_else(|| LoadError::DanglingReference {
                            what: format!("sketch {i} owner component dense id {cd} out of range"),
                        })?;
                doc.def_sketches.insert(sketch_ids[i], cid);
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

        // ── Pre-v11 consumed index: becoming, retroactively ───────────────
        // Older files persisted extruded outlines as ordinary sketch edges
        // (tombstoned at runtime, not deleted) plus the `consumed` region
        // index. Honor that index ONE final time by applying Model D's
        // consumption to it: delete each consumed region's exclusive
        // scaffolding (an edge shared with a surviving region survives,
        // exactly as at extrusion), remove a sketch the deletion emptied,
        // and then discard the index — consumption is becoming, so nothing
        // about re-extrusion is stored in the file
        // (the sketch-solid-model design §6). Without this, every
        // previously extruded outline would load back as live, drawable
        // geometry. The per-object `footprints` (v9/v10) and `source` (v8)
        // fields ARE ignored entirely: they carried the stored claims a
        // pre-v11 build used and Model D no longer keeps.
        //
        // VERSION-gated, not presence-gated: this runs only for files that
        // declare a pre-retirement format. Decode already rejected a
        // `consumed` list in a v11+ manifest as malformed (reject-not-
        // repair), so the check here is the belt to that suspender —
        // deleting geometry on the say-so of a field the declared version
        // retired would be silent repair of a malformed file. Pairs were
        // range-validated in decode, so resolution here cannot dangle; the
        // deletion is deterministic (BTreeMap order over dense-resolved
        // handles).
        if raw.format_version < crate::serialize::MANIFEST_CLAIMS_RETIRED_VERSION {
            let mut consumed_by_sketch: BTreeMap<SketchId, BTreeSet<SketchRegionId>> =
                BTreeMap::new();
            for [dense_sid, dense_rid] in &raw.consumed {
                let sid = sketch_ids[*dense_sid as usize];
                let rid = doc.sketches[sid]
                    .regions()
                    .keys()
                    .nth(*dense_rid as usize)
                    .expect("consumed pair was range-validated at decode");
                consumed_by_sketch.entry(sid).or_default().insert(rid);
            }
            for (sid, regions) in consumed_by_sketch {
                let scaffolding = doc.sketches[sid].regions_scaffolding(&regions);
                doc.sketches[sid].remove_edges(&scaffolding);
                if doc.sketches[sid].edges().is_empty() {
                    // The sketch wholly became its solids: it ceased to exist.
                    doc.hidden_sketches.insert(sid);
                }
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

        // ── Camera view state (manifest v13; absent pre-v13) ──────────────
        doc.camera = raw.camera;

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

    /// Adds a fresh, empty sketch inside `instance`'s definition — drawing
    /// on a plane while editing a component (component-edit-parity.md phase
    /// K1). `plane` is given in WORLD space (wherever the user clicked/locked
    /// through the instance's rendered pose, exactly like
    /// [`Document::add_sketch`]'s caller resolves a world plane today); the
    /// kernel maps it into definition-local space via the instance's
    /// **pose⁻¹** itself (design: "Coordinate mapping" — one implementation,
    /// exact history/replay, no per-tool drift). The new sketch is
    /// [`SketchOwner`]-marked for `instance`'s definition, so its regions
    /// render/pick and extrude under **every** instance of that definition,
    /// not just this one — a shared-geometry edit exactly like
    /// [`Document::apply_def_op`].
    ///
    /// Like [`Document::add_sketch`], creation is **not** undoable on its
    /// own: an empty sketch draws nothing, and the first gesture recorded
    /// into it folds the creation into that step.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — `instance` is stale/hidden.
    /// - [`DocumentError::Transform`] — the instance's pose fails to invert,
    ///   or the mapped plane comes out degenerate. Unreachable in practice: a
    ///   live instance's pose was validated invertible when it was created or
    ///   last transformed ([`Document::place_instance`] /
    ///   [`Document::transform_instance`] both reject a singular pose), and
    ///   [`Transform::apply_plane`] only fails on a singular transform.
    pub fn begin_sketch_on_plane_in_instance(
        &mut self,
        instance: InstanceId,
        plane: Plane,
    ) -> Result<(SketchId, DocChange), DocumentError> {
        let rec = self
            .instances
            .get(instance)
            .filter(|r| !r.hidden)
            .ok_or(DocumentError::UnknownInstance)?;
        let component = rec.def;
        let pose_inv = rec.pose.inverse().map_err(DocumentError::Transform)?;
        let local_plane = pose_inv
            .apply_plane(&plane)
            .map_err(DocumentError::Transform)?;

        let id = self.sketches.insert(Sketch::on_plane(local_plane));
        self.fresh_sketches.insert(id);
        self.def_sketches.insert(id, component);

        Ok((
            id,
            DocChange {
                components_touched: vec![component],
                ..Default::default()
            },
        ))
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
        // A curve bracket never outlives its gesture: a tool that aborted
        // mid-commit (add_segment error, thrown callback) must not leave
        // `active_curve` armed to silently tag later, unrelated edges.
        if let Some(s) = self.sketches.get_mut(sketch) {
            s.end_curve();
        }

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
        let after = Box::new(self.sketches[sketch].clone());
        self.undo.push(DocAction::SketchGesture {
            sketch,
            before: pending.before,
            after,
            created: pending.created,
        });
        self.redo.clear();
        self.fresh_sketches.remove(&sketch);
        self.debug_validate();
        let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
        Ok(DocChange {
            sketches_touched: vec![sketch],
            components_touched,
            instances_touched,
            ..Default::default()
        })
    }

    /// Drops the open gesture (if any) without recording anything — the
    /// tool-cancel path. Returns whether a gesture was open. Any mutations
    /// made inside the abandoned bracket stay in the sketch but out of the
    /// undo log; cancel-before-mutate is the caller's contract.
    pub fn cancel_sketch_gesture(&mut self) -> bool {
        if let Some(p) = &self.pending_sketch_gesture {
            let sid = p.sketch;
            if let Some(s) = self.sketches.get_mut(sid) {
                s.end_curve();
            }
            self.pending_sketch_gesture = None;
            return true;
        }
        false
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

    /// All **world** sketch handles, in unspecified but stable order.
    /// Excludes sketches hidden by [`Document::delete_sketch`], sketches that
    /// fully became solids (an extrusion that deleted a sketch's last edge
    /// removed the sketch itself — Model D), and DEFINITION-owned sketches
    /// (component-edit-parity.md phase K1): a def-owned sketch lives in
    /// definition-local space and reaches the scene only through its
    /// definition's instances, exactly like [`ObjectOwner::Definition`]
    /// members are excluded from [`Document::visible_object_ids`]. Fetch a
    /// definition's own sketches via [`Document::def_member_sketches`].
    pub fn sketch_ids(&self) -> Vec<SketchId> {
        self.sketches
            .keys()
            .filter(|s| !self.hidden_sketches.contains(s) && !self.def_sketches.contains_key(s))
            .collect()
    }

    /// The [`ComponentId`] that owns sketch `id`'s shared geometry, or `None`
    /// if it is world-owned (or stale) — the `SketchOwner` query
    /// (component-edit-parity.md phase K1), mirroring
    /// [`Document::is_world_object`] for objects.
    pub fn sketch_owner_component(&self, id: SketchId) -> Option<ComponentId> {
        self.def_sketches.get(&id).copied()
    }

    /// The definition-owned sketches of a live component, in unspecified but
    /// stable order, or `None` if the component is stale/hidden — the sketch
    /// analog of [`Document::def_members`]. Each is fetched via
    /// [`Document::sketch`] with a def-owned id from here; def-space
    /// coordinates map to world through every instance's pose exactly like a
    /// member Object's.
    pub fn def_member_sketches(&self, component: ComponentId) -> Option<Vec<SketchId>> {
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return None;
        }
        Some(
            self.def_sketches
                .iter()
                .filter(|&(_, &c)| c == component)
                .map(|(&s, _)| s)
                .collect(),
        )
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
        let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
        Ok(DocChange {
            sketches_touched: vec![sketch],
            components_touched,
            instances_touched,
            ..Default::default()
        })
    }

    /// The extrudable regions of `sketch`: simply its closed regions. Every
    /// closed region extrudes — Hew's solids interpenetrate freely, so
    /// re-extruding occupied ground is allowed like every other overlap
    /// (the sketch-solid-model design). `Err` if the sketch is stale.
    pub fn extrudable_regions(
        &self,
        sketch: SketchId,
    ) -> Result<Vec<SketchRegionId>, DocumentError> {
        let s = self.sketch(sketch).ok_or(DocumentError::UnknownSketch)?;
        Ok(s.regions().keys().collect())
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

    /// The camera's working view at last save (manifest v13+; docs/design/
    /// camera.md §5), or `None` when no view has ever been saved (a pre-v13
    /// file, or a fresh `Document` before the app's first `set_camera_state`)
    /// — the app reads `None` as "use today's home framing".
    pub fn camera_state(&self) -> Option<CameraState> {
        self.camera
    }

    /// Records the camera's current working view.
    ///
    /// View state, deliberately NOT undoable — matching how SketchUp treats
    /// the camera (`camera.rs`'s module doc) and mirroring
    /// [`Document::set_tag_hidden`]'s own non-undoable, still-serialized
    /// posture. Serialized with the document (manifest v13); saved on
    /// document save, applied on load.
    pub fn set_camera_state(&mut self, state: CameraState) {
        self.camera = Some(state);
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

    /// Delete the tag `path` — and every registered tag nested under it —
    /// from the whole document, recording one undoable
    /// [`DocAction::TagDeleted`].
    ///
    /// Deleting a tag only touches metadata; geometry is NEVER deleted or
    /// modified:
    /// - the path, and any registered path nested under it, is removed from
    ///   the tag registry (dropping its hidden-by-default flag — content that
    ///   was hidden solely via this tag becomes visible, since tag-driven
    ///   visibility is resolved from the registry);
    /// - the path and its nested paths are unassigned from every visible
    ///   node's tag list.
    ///
    /// Hidden (undone/deleted) node records keep their tag lists: they can
    /// only resurface through undo, which is LIFO and therefore restores this
    /// tag first.
    ///
    /// No-op (`Ok`, no undo entry) when the path is empty, or is neither
    /// registered nor carried by any visible node — consistent with
    /// [`Document::remove_node_tag`] on an absent path.
    pub fn delete_tag(&mut self, path: &[String]) -> Result<DocChange, DocumentError> {
        if path.is_empty() {
            return Ok(DocChange::default());
        }
        let covers = |tag: &[String]| tag.len() >= path.len() && tag[..path.len()] == *path;

        // Unregister the path and its registered descendants.
        let registry: Vec<(Vec<String>, bool)> = self
            .tag_meta
            .iter()
            .filter(|(p, _)| covers(p))
            .map(|(p, &hidden)| (p.clone(), hidden))
            .collect();
        for (p, _) in &registry {
            self.tag_meta.remove(p);
        }

        // Unassign the path (and nested paths) from every visible node.
        let mut nodes: Vec<TagListTransition> = Vec::new();
        let mut change = DocChange::default();
        for (id, rec) in self.objects.iter_mut() {
            if rec.hidden || !rec.tags.iter().any(|t| covers(t)) {
                continue;
            }
            let prev = rec.tags.clone();
            rec.tags.retain(|t| !covers(t));
            nodes.push((NodeId::Object(id), prev, rec.tags.clone()));
            change.objects_touched.push(id);
        }
        for (id, rec) in self.groups.iter_mut() {
            if rec.hidden || !rec.tags.iter().any(|t| covers(t)) {
                continue;
            }
            let prev = rec.tags.clone();
            rec.tags.retain(|t| !covers(t));
            nodes.push((NodeId::Group(id), prev, rec.tags.clone()));
            change.groups_touched.push(id);
        }
        for (id, rec) in self.instances.iter_mut() {
            if rec.hidden || !rec.tags.iter().any(|t| covers(t)) {
                continue;
            }
            let prev = rec.tags.clone();
            rec.tags.retain(|t| !covers(t));
            nodes.push((NodeId::Instance(id), prev, rec.tags.clone()));
            change.instances_touched.push(id);
        }

        if registry.is_empty() && nodes.is_empty() {
            // Unknown tag — nothing changed, no undo entry.
            return Ok(DocChange::default());
        }
        self.undo.push(DocAction::TagDeleted { registry, nodes });
        self.redo.clear();
        self.debug_validate();
        Ok(change)
    }

    /// Write only the tag list of a node (no guards — undo/redo replay of
    /// [`DocAction::TagDeleted`], which never changes names).
    fn apply_node_tags(&mut self, node: NodeId, tags: Vec<Vec<String>>) {
        match node {
            NodeId::Object(id) => {
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.tags = tags;
                }
            }
            NodeId::Group(id) => {
                if let Some(rec) = self.groups.get_mut(id) {
                    rec.tags = tags;
                }
            }
            NodeId::Instance(id) => {
                if let Some(rec) = self.instances.get_mut(id) {
                    rec.tags = tags;
                }
            }
        }
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

    /// Rename a live component definition, recording an undoable
    /// [`DocAction::ComponentRenamed`]. The definition name is the shared
    /// display label of every instance that places it, so the returned
    /// [`DocChange`] touches the component **and** all of its instances.
    ///
    /// `name = None` clears the name (instances fall back to a positional
    /// label in the UI). Renaming to the current name is a no-op (no undo
    /// entry) — consistent with [`Document::set_node_name`], so a focus-blur
    /// re-commit in the UI never pollutes the undo stack.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — stale or hidden component.
    pub fn set_component_name(
        &mut self,
        component: ComponentId,
        name: Option<String>,
    ) -> Result<DocChange, DocumentError> {
        let prev_name = match self.components.get(component) {
            Some(c) if !c.hidden => c.name.clone(),
            _ => return Err(DocumentError::UnknownComponent),
        };
        if name == prev_name {
            // No change — return a touching change without an undo entry.
            return Ok(self.component_change(component));
        }
        self.components[component].name = name.clone();
        self.undo.push(DocAction::ComponentRenamed {
            component,
            prev_name,
            next_name: name,
        });
        self.redo.clear();
        self.debug_validate();
        Ok(self.component_change(component))
    }

    /// Build a [`DocChange`] for a definition-metadata change: the component
    /// plus every instance of it (each instance displays the definition's
    /// name, so all of them are stale after a rename).
    fn component_change(&self, component: ComponentId) -> DocChange {
        DocChange {
            components_touched: vec![component],
            instances_touched: self.instances_of(component),
            ..Default::default()
        }
    }

    /// The names of all live component definitions, for name generation.
    fn live_component_names(&self) -> Vec<&str> {
        self.components
            .iter()
            .filter(|(_, c)| !c.hidden)
            .filter_map(|(_, c)| c.name.as_deref())
            .collect()
    }

    /// A generated definition name for a component whose selection carried no
    /// name to inherit: `"Component N"` with the lowest `N ≥ 1` no live
    /// definition already uses. Deterministic (a pure function of the live
    /// definition set).
    fn generated_component_name(&self) -> String {
        let taken = self.live_component_names();
        let mut n: u32 = 1;
        loop {
            let candidate = format!("Component {n}");
            if !taken.iter().any(|&t| t == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// The name for a made-unique copy of a definition named `base`:
    /// `"<base> Copy"`, or — when a live definition already holds that name —
    /// `"<base> Copy 2"`, `"<base> Copy 3"`, … (the lowest free number).
    fn copy_component_name(&self, base: &str) -> String {
        let taken = self.live_component_names();
        let first = format!("{base} Copy");
        if !taken.iter().any(|&t| t == first) {
            return first;
        }
        let mut n: u32 = 2;
        loop {
            let candidate = format!("{base} Copy {n}");
            if !taken.iter().any(|&t| t == candidate) {
                return candidate;
            }
            n += 1;
        }
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

    /// `(component, pose)` for a live instance, or [`DocumentError::UnknownInstance`]
    /// — the common first step of every K2 in-instance surface
    /// (component-edit-parity.md phase K2: `follow_me_in_instance` and its
    /// siblings, `slice_def_member`, `transform_def_member`), mirroring the
    /// lookup [`Document::extrude_region_in_instance`] and
    /// [`Document::begin_sketch_on_plane_in_instance`] already open with.
    fn instance_component(
        &self,
        instance: InstanceId,
    ) -> Result<(ComponentId, Transform), DocumentError> {
        let rec = self
            .instances
            .get(instance)
            .filter(|r| !r.hidden)
            .ok_or(DocumentError::UnknownInstance)?;
        Ok((rec.def, rec.pose))
    }

    /// Maps a WORLD-space scalar length through `pose` into definition-local
    /// units — the shared rule [`Document::extrude_region_in_instance`]
    /// documents for its `distance` parameter, factored out so every K2
    /// surface with a typed world-space scalar (a Follow Me partial-sweep
    /// `stop_len`, and any future one) refuses [`DocumentError::AmbiguousInstanceScale`]
    /// identically under a non-uniformly-scaled instance rather than guessing
    /// an axis.
    fn map_world_distance_through_pose(
        &self,
        pose: Transform,
        distance: f64,
    ) -> Result<f64, DocumentError> {
        let scale = pose
            .similarity_scale()
            .ok_or(DocumentError::AmbiguousInstanceScale)?;
        Ok(distance / scale)
    }

    /// Public wrapper of [`Document::instance_component`] +
    /// [`Document::map_world_distance_through_pose`] for wasm-api surfaces
    /// that key a per-instance edit on `(component, object)` rather than
    /// `instance` directly — currently [`Scene::push_pull_in_component`]
    /// (component-edit-parity.md phase A2), whose ghost preview sweeps the
    /// WORLD drag distance exactly like [`Document::extrude_region_in_instance`]'s
    /// birth distance, so the committed distance must map through the same
    /// pose rule rather than land in definition space raw. Resolves
    /// `instance`'s definition and pose, then maps `distance` through the
    /// pose's uniform scale — identical to `extrude_region_in_instance` and
    /// `follow_me_in_instance`'s `stop_len` mapping, including the typed
    /// refusal under a non-uniformly-scaled instance.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — `instance` is stale/hidden.
    /// - [`DocumentError::AmbiguousInstanceScale`] — the instance's pose is
    ///   not a similarity (non-uniform scale); a world length has no single
    ///   local-frame equivalent then.
    pub fn map_instance_world_distance(
        &self,
        instance: InstanceId,
        distance: f64,
    ) -> Result<(ComponentId, f64), DocumentError> {
        let (component, pose) = self.instance_component(instance)?;
        let local = self.map_world_distance_through_pose(pose, distance)?;
        Ok((component, local))
    }

    /// The `(components_touched, instances_touched)` pair for an object id,
    /// empty for a world object or a stale one (component-edit-parity.md
    /// phase K2). Several `DocAction` variants (`Boolean`, `Sliced`,
    /// `Transform`, `FollowMeFace`) now record either a world or a
    /// definition-member result — the toggle-hidden undo/redo mechanics are
    /// identical either way, only the touched-list report differs — so their
    /// undo/redo arms share this rather than re-deriving it inline, mirroring
    /// the pattern `undo_created_object`/`redo_created_object` already use
    /// for a def-owned birth.
    fn def_owner_change(&self, id: ObjectId) -> (Vec<ComponentId>, Vec<InstanceId>) {
        match self.objects.get(id).map(|r| r.owner) {
            Some(ObjectOwner::Definition(component)) => {
                (vec![component], self.instances_of(component))
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// Component/instance touch set for a definition-owned sketch.
    fn def_sketch_owner_change(&self, id: SketchId) -> (Vec<ComponentId>, Vec<InstanceId>) {
        match self.sketch_owner_component(id) {
            Some(component) => (vec![component], self.instances_of(component)),
            None => (Vec::new(), Vec::new()),
        }
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

    /// THE solid-creating act (ARCHITECTURE.md): extrudes a closed sketch
    /// region into a new watertight Object, DELETES the region's
    /// scaffolding from the sketch, and records both on the document undo
    /// log as one step.
    ///
    /// Consumption is becoming (Model D, the sketch-solid-model design):
    /// the drawn profile is now the solid's base face, so the edges only
    /// this region needed leave the sketch — really deleted, not hidden.
    /// Edges shared with a surviving region stay (the neighbor remains
    /// closed), open chains stay, and a partially consumed curve chain
    /// keeps its analytic geometry on the surviving facets
    /// ([`Sketch::region_scaffolding`], [`Sketch::remove_edges`]). If the
    /// deletion empties the sketch, the sketch itself ceases to exist (it
    /// wholly became the solid). Undo restores the exact pre-extrusion
    /// snapshot and hides the solid atomically.
    ///
    /// Never refuses on account of a solid already standing there:
    /// interpenetration on re-extrude is allowed exactly as everywhere else
    /// in Hew (the sketch-solid-model design — the standing-solid gate
    /// was dropped as inconsistent with the freely-interpenetrating-solids
    /// model).
    ///
    /// Returns the new Object's handle and the [`DocChange`] it caused (the
    /// new Object plus the sketch that lost the scaffolding).
    pub fn extrude_region(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        distance: f64,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "extrude_region", distance);
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        // World-op guard (component-edit-parity.md phase K1): a def-owned
        // sketch lives in DEFINITION-local space; birthing a WORLD Object
        // straight from it would place the result in the wrong frame — the
        // sketch-analog of `apply_object_op`'s `is_world()` guard on
        // objects. Use `extrude_region_in_instance` instead.
        if self.sketch_owner_component(sketch).is_some() {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let scaffolding = s
            .region_scaffolding(region)
            .map_err(DocumentError::Sketch)?;
        let object = Object::from_extrusion(&profile, distance).map_err(DocumentError::Extrude)?;

        // Everything that can fail has succeeded; commit.
        Ok(self.commit_region_object(sketch, &scaffolding, object, None, None))
    }

    /// [`Document::extrude_region`] for a sketch owned by `instance`'s
    /// definition (component-edit-parity.md phase K1): extrudes `region` of
    /// `sketch` — which must be a def-owned sketch of that same definition,
    /// e.g. one opened via [`Document::begin_sketch_on_plane_in_instance`] —
    /// by `distance` (a WORLD-space length along the region's normal,
    /// exactly like the plain `extrude_region`'s `distance`), and births the
    /// solid as a member of the definition instead of a world Object. Every
    /// instance of the definition sees the new member at once — a
    /// shared-geometry edit exactly like [`Document::apply_def_op`].
    ///
    /// `distance` is typed in world units, but the sketch (and the extrusion
    /// it produces) lives in DEFINITION-local space, so it is divided by the
    /// instance pose's uniform scale factor before extruding — the same
    /// pose⁻¹ mapping [`Document::begin_sketch_on_plane_in_instance`] applies
    /// to the sketch's plane, applied here to a scalar length instead of a
    /// point/plane (design: "Coordinate mapping"). A pose that is a rotation
    /// and/or a mirror (determinant < 0) with **uniform** scale maps a
    /// distance unambiguously ([`Transform::similarity_scale`] returns
    /// `Some`); a **non-uniformly** scaled instance cannot — which per-axis
    /// scale would the number be? — so this refuses typed rather than
    /// guessing.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — `instance` is stale/hidden.
    /// - [`DocumentError::UnknownSketch`] — `sketch` is stale/hidden, or not
    ///   owned by `instance`'s own definition (extruding a world sketch, or
    ///   one owned by a *different* definition, into this instance is not a
    ///   meaningful op).
    /// - [`DocumentError::AmbiguousInstanceScale`] — the instance's pose is
    ///   not a similarity (non-uniform scale).
    /// - [`DocumentError::Sketch`] — the region handle is stale.
    /// - [`DocumentError::Extrude`] — the profile fails to extrude (see
    ///   [`Object::from_extrusion`]).
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn extrude_region_in_instance(
        &mut self,
        instance: InstanceId,
        sketch: SketchId,
        region: SketchRegionId,
        distance: f64,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "extrude_region_in_instance", distance);
        let rec = self
            .instances
            .get(instance)
            .filter(|r| !r.hidden)
            .ok_or(DocumentError::UnknownInstance)?;
        let component = rec.def;
        let pose = rec.pose;
        if self.sketch_owner_component(sketch) != Some(component) {
            return Err(DocumentError::UnknownSketch);
        }
        let scale = pose
            .similarity_scale()
            .ok_or(DocumentError::AmbiguousInstanceScale)?;
        let local_distance = distance / scale;

        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let scaffolding = s
            .region_scaffolding(region)
            .map_err(DocumentError::Sketch)?;
        let object =
            Object::from_extrusion(&profile, local_distance).map_err(DocumentError::Extrude)?;

        // Everything that can fail has succeeded; commit.
        Ok(self.commit_region_object_in_definition(component, sketch, &scaffolding, object))
    }

    /// Follow Me (the follow-me design): sweeps the closed profile
    /// `region` of `sketch` along `path` into a new watertight world Object
    /// via [`Object::from_follow_me`]. Commits exactly like
    /// [`Document::extrude_region`]: the profile region's exclusive
    /// scaffolding is consumed (Model D — the outline became the solid's
    /// cross-section; the path sketch or solid is never touched) and the
    /// step records [`DocAction::CreatedObject`], so undo hides the solid
    /// and re-inserts the outline atomically, redo re-deletes by geometry.
    ///
    /// # Errors
    /// [`DocumentError::UnknownSketch`] / [`DocumentError::UnknownObject`] /
    /// [`DocumentError::UnknownFace`] for stale or hidden handles;
    /// [`DocumentError::Sketch`] for a stale region;
    /// [`DocumentError::FollowMe`] for everything path chaining and the
    /// sweep itself refuse ([`FollowMeError`]). The document is untouched
    /// on every error.
    pub fn follow_me(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        self.follow_me_impl(sketch, region, path, None, None)
    }

    /// [`Document::follow_me`] stopped after `stop_len` of arc length from
    /// the seam — the partial sweep behind dragging a profile part-way
    /// along its path (see [`Object::from_follow_me_to`] for the stop's
    /// exact semantics). Same errors, same strong exception guarantee.
    pub fn follow_me_to(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
        stop_len: f64,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        self.follow_me_impl(sketch, region, path, Some(stop_len), None)
    }

    /// [`Document::follow_me`] whose result is born INSIDE `group`
    /// (design §2f) — the sweep a user commits while editing that group.
    /// Same behavior otherwise, including the optional partial-sweep stop.
    /// [`DocumentError::UnknownGroup`] for a stale or hidden group.
    pub fn follow_me_grouped(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
        stop_len: Option<f64>,
        group: GroupId,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        self.follow_me_impl(sketch, region, path, stop_len, Some(group))
    }

    fn follow_me_impl(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
        stop_len: Option<f64>,
        parent_group: Option<GroupId>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        if let Some(gid) = parent_group {
            // Birth into a group the user is editing (design §2f): the
            // group must be live and visible.
            if !self.groups.contains_key(gid) || self.groups[gid].hidden {
                return Err(DocumentError::UnknownGroup);
            }
        }
        info!(target: "kernel::op", op = "follow_me");
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        // World-op guard (component-edit-parity.md phase K1): see
        // `extrude_region`'s matching guard.
        if self.sketch_owner_component(sketch).is_some() {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let scaffolding = s
            .region_scaffolding(region)
            .map_err(DocumentError::Sketch)?;

        let (points, closed, curves) = self.resolve_follow_me_path(path)?;
        let object = match stop_len {
            None => Object::from_follow_me(&profile, &points, closed, &curves),
            Some(stop) => Object::from_follow_me_to(&profile, &points, closed, &curves, stop),
        }
        .map_err(DocumentError::FollowMe)?;

        // Everything that can fail has succeeded; commit.
        Ok(self.commit_region_object(sketch, &scaffolding, object, None, parent_group))
    }

    /// [`Document::follow_me`] that MERGES its result with the path's own
    /// solid in one gesture (design §3b): the profile region sweeps around
    /// the face loop, then a sweep whose body overlaps the solid's
    /// interior carves it (Subtract — a chamfer, a dado) and one that only
    /// rides its surface adds to it (Union — a molding), decided by the
    /// boolean engine itself on clones (a nonempty intersection means
    /// overlap). ONE undo step: the region's scaffolding, the consumed
    /// base, and the merged result all restore together. Only a
    /// [`FollowMePath::FaceLoop`] has a solid to merge with; an edge path
    /// refuses [`DocumentError::UnknownObject`].
    pub fn follow_me_merged(
        &mut self,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
        stop_len: Option<f64>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "follow_me_merged");
        let FollowMePath::FaceLoop {
            object: base_id, ..
        } = *path
        else {
            return Err(DocumentError::UnknownObject);
        };
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        // World-op guard (component-edit-parity.md phase K1): see
        // `extrude_region`'s matching guard.
        if self.sketch_owner_component(sketch).is_some() {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let scaffolding = s
            .region_scaffolding(region)
            .map_err(DocumentError::Sketch)?;
        let (points, closed, curves) = self.resolve_follow_me_path(path)?;
        let swept = match stop_len {
            None => Object::from_follow_me(&profile, &points, closed, &curves),
            Some(stop) => Object::from_follow_me_to(&profile, &points, closed, &curves, stop),
        }
        .map_err(DocumentError::FollowMe)?;

        let base_rec = self
            .objects
            .get(base_id)
            .filter(|r| !r.hidden && r.is_world())
            .ok_or(DocumentError::UnknownObject)?;
        if base_rec.group_parent().is_some() {
            return Err(DocumentError::GroupedOperand);
        }
        let base = &base_rec.object;
        let op = match Object::boolean(BooleanOp::Intersect, base, &swept, &Transform::IDENTITY) {
            Ok(_) => BooleanOp::Subtract,
            Err(_) => BooleanOp::Union,
        };
        let mut result = Object::boolean(op, base, &swept, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;
        // Dissolve the merge's coplanar seams, preserving the base's
        // pre-existing imprints (`Document::boolean`'s treatment; the
        // swept tool is fresh and contributes none).
        let preserve = base.coplanar_edge_segments();
        result.merge_coplanar_faces(&preserve);

        // Everything that can fail has succeeded; one compound commit.
        Ok(self.commit_region_object(sketch, &scaffolding, result, Some(base_id), None))
    }

    /// Follow Me with a solid FACE as the profile (design §3a): sweeps the
    /// face's boundary (holes become tunnels) along `path` into a NEW
    /// top-level object, leaving the source solid untouched — unless the
    /// profile face belongs to the path's own solid (design §3b), in which
    /// case the sweep MERGES with it in one gesture and one undo step: a
    /// sweep whose body overlaps the solid's interior carves it (Subtract —
    /// a chamfer, a dado), one that only rides its surface adds to it
    /// (Union — a molding). The overlap question is answered by the boolean
    /// engine itself on clones: a nonempty intersection means overlap.
    /// `stop_len` is the partial-sweep stop, exactly as on
    /// [`Document::follow_me_to`]. Every fallible step runs before the
    /// document is touched (the strong exception guarantee).
    pub fn follow_me_face(
        &mut self,
        object: ObjectId,
        face: FaceId,
        path: &FollowMePath,
        stop_len: Option<f64>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "follow_me_face");
        let rec = self
            .objects
            .get(object)
            .filter(|r| !r.hidden && r.is_world())
            .ok_or(DocumentError::UnknownObject)?;
        let profile = rec
            .object
            .profile_from_face(face)
            .ok_or(DocumentError::UnknownFace)?;
        let (points, closed, curves) = self.resolve_follow_me_path(path)?;
        let swept = match stop_len {
            None => Object::from_follow_me(&profile, &points, closed, &curves),
            Some(stop) => Object::from_follow_me_to(&profile, &points, closed, &curves, stop),
        }
        .map_err(DocumentError::FollowMe)?;

        let merges = matches!(path, FollowMePath::FaceLoop { object: po, .. } if *po == object);
        if !merges {
            let id = self.objects.insert(ObjectRecord {
                object: swept,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::World { parent: None },
                name: None,
                tags: Vec::new(),
            });
            self.undo.push(DocAction::FollowMeFace {
                result: id,
                merged_base: None,
            });
            self.redo.clear();
            self.debug_validate();
            let change = DocChange {
                objects_touched: vec![id],
                sketches_touched: Vec::new(),
                groups_touched: Vec::new(),
                instances_touched: Vec::new(),
                components_touched: Vec::new(),
                guides_touched: Vec::new(),
            };
            return Ok((id, change));
        }

        // Merge with the path's own solid. The base follows the boolean's
        // operand rules; the swept body never enters the document.
        let base_rec = &self.objects[object];
        if base_rec.group_parent().is_some() {
            return Err(DocumentError::GroupedOperand);
        }
        let base = &base_rec.object;
        let op = match Object::boolean(BooleanOp::Intersect, base, &swept, &Transform::IDENTITY) {
            Ok(_) => BooleanOp::Subtract,
            Err(_) => BooleanOp::Union,
        };
        let mut result = Object::boolean(op, base, &swept, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;
        // Dissolve the coplanar seams the merge introduced, preserving the
        // base's pre-existing imprints; the swept tool is fresh and
        // contributes none (exactly `Document::boolean`'s treatment).
        let preserve = base.coplanar_edge_segments();
        result.merge_coplanar_faces(&preserve);

        let id = self.objects.insert(ObjectRecord {
            object: result,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::World { parent: None },
            name: None,
            tags: Vec::new(),
        });
        self.objects[object].hidden = true;
        self.undo.push(DocAction::FollowMeFace {
            result: id,
            merged_base: Some(object),
        });
        self.redo.clear();
        self.debug_validate();
        let change = DocChange {
            objects_touched: vec![object, id],
            sketches_touched: Vec::new(),
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
            guides_touched: Vec::new(),
        };
        Ok((id, change))
    }

    /// [`Document::follow_me`] inside `instance`'s definition
    /// (component-edit-parity.md phase K2): sweeps the closed profile
    /// `region` of a def-owned `sketch` — belonging to `instance`'s OWN
    /// definition — along `path`, entirely in definition-local space. `path`
    /// must resolve within the SAME definition too: a def-owned
    /// `SketchEdges` chain of that definition, or a `FaceLoop` on one of its
    /// OWN members ([`Document::resolve_follow_me_path_in_component`]).
    /// Mixing a world path with this definition profile — or a path from a
    /// *different* definition — refuses typed exactly like
    /// [`Document::extrude_region_in_instance`] refuses a mismatched
    /// sketch. `stop_len`, when given, is the WORLD-space partial-sweep
    /// stop (arc length from the seam, matching [`Document::follow_me_to`]'s
    /// contract), mapped through the instance's pose like
    /// `extrude_region_in_instance`'s `distance`
    /// ([`Document::map_world_distance_through_pose`]).
    ///
    /// The swept solid is born as a NEW member of the definition — every
    /// instance of it sees the result at once.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — `instance` is stale/hidden.
    /// - [`DocumentError::UnknownSketch`] — `sketch` is stale/hidden or not
    ///   owned by `instance`'s own definition.
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownFace`] —
    ///   a `FaceLoop` path object is not a live member of the same
    ///   definition, or has no such face.
    /// - [`DocumentError::AmbiguousInstanceScale`] — `stop_len` is given and
    ///   the instance's pose is not a similarity.
    /// - [`DocumentError::Sketch`] / [`DocumentError::FollowMe`] — as
    ///   [`Document::follow_me`].
    ///
    /// On `Err` the document is untouched.
    pub fn follow_me_in_instance(
        &mut self,
        instance: InstanceId,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
        stop_len: Option<f64>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "follow_me_in_instance");
        let (component, pose) = self.instance_component(instance)?;
        let local_stop = stop_len
            .map(|stop| self.map_world_distance_through_pose(pose, stop))
            .transpose()?;
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        if self.sketch_owner_component(sketch) != Some(component) {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let scaffolding = s
            .region_scaffolding(region)
            .map_err(DocumentError::Sketch)?;
        let (points, closed, curves) = self.resolve_follow_me_path_in_component(component, path)?;
        let object = match local_stop {
            None => Object::from_follow_me(&profile, &points, closed, &curves),
            Some(stop) => Object::from_follow_me_to(&profile, &points, closed, &curves, stop),
        }
        .map_err(DocumentError::FollowMe)?;

        // Everything that can fail has succeeded; commit.
        Ok(self.commit_region_object_in_definition(component, sketch, &scaffolding, object))
    }

    /// [`Document::follow_me_in_instance`] that MERGES its result with the
    /// path's own member solid in one gesture (mirrors
    /// [`Document::follow_me_merged`] — design §3b, applied inside a
    /// definition): only a [`FollowMePath::FaceLoop`] whose object is a
    /// member of the SAME definition has a solid to merge with; anything
    /// else — an edge path, a world object, a different definition's member
    /// — refuses [`DocumentError::UnknownObject`]. ONE undo step: the
    /// profile region's scaffolding, the consumed base member, and the
    /// merged result all restore together.
    pub fn follow_me_merged_in_instance(
        &mut self,
        instance: InstanceId,
        sketch: SketchId,
        region: SketchRegionId,
        path: &FollowMePath,
        stop_len: Option<f64>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "follow_me_merged_in_instance");
        let FollowMePath::FaceLoop {
            object: base_id, ..
        } = *path
        else {
            return Err(DocumentError::UnknownObject);
        };
        let (component, pose) = self.instance_component(instance)?;
        let local_stop = stop_len
            .map(|stop| self.map_world_distance_through_pose(pose, stop))
            .transpose()?;
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        if self.sketch_owner_component(sketch) != Some(component) {
            return Err(DocumentError::UnknownSketch);
        }
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let profile = s.profile(region).map_err(DocumentError::Sketch)?;
        let scaffolding = s
            .region_scaffolding(region)
            .map_err(DocumentError::Sketch)?;
        let (points, closed, curves) = self.resolve_follow_me_path_in_component(component, path)?;
        let swept = match local_stop {
            None => Object::from_follow_me(&profile, &points, closed, &curves),
            Some(stop) => Object::from_follow_me_to(&profile, &points, closed, &curves, stop),
        }
        .map_err(DocumentError::FollowMe)?;

        let base_rec = self
            .objects
            .get(base_id)
            .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
            .ok_or(DocumentError::UnknownObject)?;
        let base = &base_rec.object;
        let op = match Object::boolean(BooleanOp::Intersect, base, &swept, &Transform::IDENTITY) {
            Ok(_) => BooleanOp::Subtract,
            Err(_) => BooleanOp::Union,
        };
        let mut result = Object::boolean(op, base, &swept, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;
        // Dissolve the merge's coplanar seams, preserving the base's
        // pre-existing imprints (`Document::boolean`'s treatment).
        let preserve = base.coplanar_edge_segments();
        result.merge_coplanar_faces(&preserve);

        // Everything that can fail has succeeded; one compound commit.
        Ok(self.commit_region_object_owned(
            sketch,
            &scaffolding,
            result,
            Some(base_id),
            Some(component),
            None,
        ))
    }

    /// [`Document::follow_me_face`] with the profile face on a MEMBER of
    /// `instance`'s definition (component-edit-parity.md phase K2, design
    /// §3a applied inside a definition): sweeps `face`'s boundary along
    /// `path` — which must resolve within the SAME definition
    /// ([`Document::resolve_follow_me_path_in_component`]) — into a new
    /// member, leaving the profile member untouched UNLESS `path` is a
    /// `FaceLoop` on `object` ITSELF, in which case the sweep MERGES with it
    /// in one gesture (design §3b), exactly like the world
    /// [`Document::follow_me_face`].
    ///
    /// # Errors
    /// Mirrors [`Document::follow_me_in_instance`]'s, with `object`/`face`
    /// in place of `sketch`/`region`.
    pub fn follow_me_face_in_instance(
        &mut self,
        instance: InstanceId,
        object: ObjectId,
        face: FaceId,
        path: &FollowMePath,
        stop_len: Option<f64>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "follow_me_face_in_instance");
        let (component, pose) = self.instance_component(instance)?;
        let local_stop = stop_len
            .map(|stop| self.map_world_distance_through_pose(pose, stop))
            .transpose()?;
        let rec = self
            .objects
            .get(object)
            .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
            .ok_or(DocumentError::UnknownObject)?;
        let profile = rec
            .object
            .profile_from_face(face)
            .ok_or(DocumentError::UnknownFace)?;
        let (points, closed, curves) = self.resolve_follow_me_path_in_component(component, path)?;
        let swept = match local_stop {
            None => Object::from_follow_me(&profile, &points, closed, &curves),
            Some(stop) => Object::from_follow_me_to(&profile, &points, closed, &curves, stop),
        }
        .map_err(DocumentError::FollowMe)?;

        let merges = matches!(path, FollowMePath::FaceLoop { object: po, .. } if *po == object);
        if !merges {
            let id = self.objects.insert(ObjectRecord {
                object: swept,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::Definition(component),
                name: None,
                tags: Vec::new(),
            });
            self.components[component].members.push(id);
            self.undo.push(DocAction::FollowMeFace {
                result: id,
                merged_base: None,
            });
            self.redo.clear();
            self.debug_validate();
            let instances_touched = self.instances_of(component);
            let change = DocChange {
                objects_touched: vec![id],
                components_touched: vec![component],
                instances_touched,
                ..Default::default()
            };
            return Ok((id, change));
        }

        // Merge with the path's own member solid. The base follows the
        // boolean's operand rules; the swept body never enters the document.
        let base_rec = &self.objects[object];
        let base = &base_rec.object;
        let op = match Object::boolean(BooleanOp::Intersect, base, &swept, &Transform::IDENTITY) {
            Ok(_) => BooleanOp::Subtract,
            Err(_) => BooleanOp::Union,
        };
        let mut result = Object::boolean(op, base, &swept, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;
        let preserve = base.coplanar_edge_segments();
        result.merge_coplanar_faces(&preserve);

        let id = self.objects.insert(ObjectRecord {
            object: result,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::Definition(component),
            name: None,
            tags: Vec::new(),
        });
        self.objects[object].hidden = true;
        self.components[component].members.push(id);
        self.undo.push(DocAction::FollowMeFace {
            result: id,
            merged_base: Some(object),
        });
        self.redo.clear();
        self.debug_validate();
        let instances_touched = self.instances_of(component);
        let change = DocChange {
            objects_touched: vec![object, id],
            components_touched: vec![component],
            instances_touched,
            ..Default::default()
        };
        Ok((id, change))
    }

    /// Resolves a [`FollowMePath`] into the polyline
    /// [`Object::from_follow_me`] consumes: ordered points, whether the
    /// path closes, and each segment's analytic curve attribution (the
    /// [`CurveGeom`] its sketch edge was drawn from — what lets the sweep
    /// measure perpendicularity against the drawn curve rather than its
    /// facet chords; empty for a face loop, whose rim edges carry no curve
    /// claims). Pure query; the source entities are untouched.
    fn resolve_follow_me_path(
        &self,
        path: &FollowMePath,
    ) -> Result<ResolvedFollowMePath, DocumentError> {
        match path {
            FollowMePath::SketchEdges { sketch, edges } => {
                if self.hidden_sketches.contains(sketch) {
                    return Err(DocumentError::UnknownSketch);
                }
                // World-op guard (component-edit-parity.md phase K1): the
                // PATH sketch, like the profile sketch `follow_me`/
                // `follow_me_merged`/`follow_me_face` already guard, can be
                // definition-owned — its edges live in DEFINITION-local
                // space, so chaining them straight into a WORLD sweep would
                // hand `Object::from_follow_me` raw def-local coordinates
                // (the path sketch is independent of the profile sketch, so
                // this must be checked here too, not just at the call
                // sites). Refuses exactly like `extrude_region`'s guard;
                // `resolve_follow_me_path` is the single chokepoint every
                // `FollowMePath` variant flows through, so this covers
                // every world follow-me surface (plain, merged, face-
                // profile) without duplicating the check at each call site.
                if self.sketch_owner_component(*sketch).is_some() {
                    return Err(DocumentError::UnknownSketch);
                }
                let s = self
                    .sketches
                    .get(*sketch)
                    .ok_or(DocumentError::UnknownSketch)?;
                chain_sketch_edges(s, edges).map_err(DocumentError::FollowMe)
            }
            FollowMePath::FaceLoop { object, face } => {
                let rec = self
                    .objects
                    .get(*object)
                    .filter(|r| !r.hidden && r.is_world())
                    .ok_or(DocumentError::UnknownObject)?;
                let f = rec
                    .object
                    .faces()
                    .get(*face)
                    .ok_or(DocumentError::UnknownFace)?;
                let points: Vec<Point3> = rec.object.loop_positions(f.outer_loop).collect();
                Ok((points, true, Vec::new()))
            }
            FollowMePath::InstanceFaceLoop {
                instance,
                object,
                face,
            } => {
                // The member is definition-owned (NOT world); its loop is
                // definition-local and rides the instance's pose into
                // world space (design §2e). A reflected pose would hand
                // the sweep a mirrored winding — refused typed, matching
                // the explode gate.
                let pose = self
                    .instance_pose(*instance)
                    .ok_or(DocumentError::UnknownInstance)?;
                if pose.determinant() < 0.0 {
                    return Err(DocumentError::Transform(TransformError::Reflection));
                }
                let rec = self
                    .objects
                    .get(*object)
                    .filter(|r| !r.hidden && !r.is_world())
                    .ok_or(DocumentError::UnknownObject)?;
                let f = rec
                    .object
                    .faces()
                    .get(*face)
                    .ok_or(DocumentError::UnknownFace)?;
                let points: Vec<Point3> = rec
                    .object
                    .loop_positions(f.outer_loop)
                    .map(|p| pose.apply_point(p))
                    .collect();
                Ok((points, true, Vec::new()))
            }
        }
    }

    /// Resolves a [`FollowMePath`] the same way [`Document::resolve_follow_me_path`]
    /// does for the WORLD case, but requires every entity the path touches
    /// to belong to `component`'s OWN member set (component-edit-parity.md
    /// phase K2) rather than being a world object: a `SketchEdges` chain
    /// must be a def-owned sketch of `component`, and a `FaceLoop` object
    /// must be a live member of `component`. `InstanceFaceLoop` reaches
    /// through ANOTHER instance's pose into WORLD space by construction
    /// (design §2e) and can never resolve inside a definition — refused
    /// typed ([`DocumentError::UnknownInstance`]) rather than silently
    /// mapping through a pose that would leave the definition's own local
    /// frame. Mixing a world path with a definition profile (or a path from
    /// a *different* definition) is exactly this ownership mismatch,
    /// refused the same way [`Document::extrude_region_in_instance`]
    /// refuses a mismatched sketch — this is the single chokepoint every
    /// `*_in_instance`/`*_in_component` Follow Me surface's path flows
    /// through, mirroring how `resolve_follow_me_path` is the world side's.
    fn resolve_follow_me_path_in_component(
        &self,
        component: ComponentId,
        path: &FollowMePath,
    ) -> Result<ResolvedFollowMePath, DocumentError> {
        match path {
            FollowMePath::SketchEdges { sketch, edges } => {
                if self.hidden_sketches.contains(sketch) {
                    return Err(DocumentError::UnknownSketch);
                }
                if self.sketch_owner_component(*sketch) != Some(component) {
                    return Err(DocumentError::UnknownSketch);
                }
                let s = self
                    .sketches
                    .get(*sketch)
                    .ok_or(DocumentError::UnknownSketch)?;
                chain_sketch_edges(s, edges).map_err(DocumentError::FollowMe)
            }
            FollowMePath::FaceLoop { object, face } => {
                let rec = self
                    .objects
                    .get(*object)
                    .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
                    .ok_or(DocumentError::UnknownObject)?;
                let f = rec
                    .object
                    .faces()
                    .get(*face)
                    .ok_or(DocumentError::UnknownFace)?;
                let points: Vec<Point3> = rec.object.loop_positions(f.outer_loop).collect();
                Ok((points, true, Vec::new()))
            }
            FollowMePath::InstanceFaceLoop { .. } => Err(DocumentError::UnknownInstance),
        }
    }

    /// Shared commit for the region-consuming solid births
    /// ([`Document::extrude_region`], [`Document::follow_me`]): captures the
    /// region's exclusive scaffolding as re-insertable rows (endpoints +
    /// curve chain) so undo can restore it by merging into the sketch's
    /// THEN-current contents rather than clobbering them with a snapshot,
    /// deletes it (Model D), inserts the new object into the WORLD tree
    /// (top-level, or inside `parent_group` — design §2f), and records
    /// [`DocAction::CreatedObject`]. Callers must have finished everything
    /// that can fail before calling. The definition-owning counterpart is
    /// [`Document::commit_region_object_in_definition`], which shares this
    /// scaffolding/undo machinery via [`Document::commit_region_object_owned`].
    fn commit_region_object(
        &mut self,
        sketch: SketchId,
        scaffolding: &BTreeSet<SketchEdgeId>,
        object: Object,
        merged_base: Option<ObjectId>,
        parent_group: Option<GroupId>,
    ) -> (ObjectId, DocChange) {
        self.commit_region_object_owned(
            sketch,
            scaffolding,
            object,
            merged_base,
            None,
            parent_group,
        )
    }

    /// Commits a birthed Object as a member of `component` (component-edit-
    /// parity.md phase K1) — the definition-owning counterpart of
    /// [`Document::commit_region_object`], used by
    /// [`Document::extrude_region_in_instance`]. `component` must already be
    /// validated live by the caller.
    fn commit_region_object_in_definition(
        &mut self,
        component: ComponentId,
        sketch: SketchId,
        scaffolding: &BTreeSet<SketchEdgeId>,
        object: Object,
    ) -> (ObjectId, DocChange) {
        self.commit_region_object_owned(sketch, scaffolding, object, None, Some(component), None)
    }

    /// Consumes a region's sketch scaffolding (Model D) and inserts `object`
    /// as a new Object — either a WORLD object (`def_owner: None`, optionally
    /// inside `parent_group`) or a member of `def_owner`'s definition
    /// (`parent_group` is meaningless there and ignored). Records
    /// [`DocAction::CreatedObject`] either way: undo hides the Object (never
    /// deleting it — its id and, for a definition member, its listing in
    /// `ComponentDef.members`, both stay put, exactly like a hidden world
    /// object stays listed in its parent group's `members`) and restores the
    /// consumed scaffolding atomically; redo re-applies both. Ownership never
    /// changes across undo/redo — only visibility does.
    fn commit_region_object_owned(
        &mut self,
        sketch: SketchId,
        scaffolding: &BTreeSet<SketchEdgeId>,
        object: Object,
        merged_base: Option<ObjectId>,
        def_owner: Option<ComponentId>,
        parent_group: Option<GroupId>,
    ) -> (ObjectId, DocChange) {
        let s = self
            .sketches
            .get(sketch)
            .expect("caller verified the sketch");
        let removed: Vec<(Point3, Point3, Option<SketchCurveId>)> = scaffolding
            .iter()
            .map(|&eid| {
                let e = s.edges()[eid];
                (
                    s.vertices()[e.from].position,
                    s.vertices()[e.to].position,
                    e.curve,
                )
            })
            .collect();
        let sk = self.sketches.get_mut(sketch).expect("sketch was just read");
        sk.remove_edges(scaffolding);
        let emptied = sk.edges().is_empty();
        if emptied {
            // The sketch wholly became the solid: nothing hidden survives —
            // the entity itself leaves the document (larval form, Model D).
            self.hidden_sketches.insert(sketch);
        }

        let owner = match def_owner {
            Some(component) => ObjectOwner::Definition(component),
            None => ObjectOwner::World {
                parent: parent_group,
            },
        };
        let id = self.objects.insert(ObjectRecord {
            object,
            history: History::new(),
            hidden: false,
            owner,
            name: None,
            tags: Vec::new(),
        });
        let mut groups_touched = Vec::new();
        let mut components_touched = Vec::new();
        let mut instances_touched = Vec::new();
        if let Some(component) = def_owner {
            // Shared-geometry birth (component-edit-parity.md phase K1):
            // membership is structural, like a world birth inside a group;
            // every instance of the definition sees the new member.
            self.components[component].members.push(id);
            components_touched.push(component);
            instances_touched = self.instances_of(component);
        } else if let Some(gid) = parent_group {
            // Birth INSIDE the group the user is editing (design §2f):
            // membership is structural; undo's hide-not-delete leaves the
            // hidden member harmlessly listed, like any hidden node.
            self.groups[gid].members.push(NodeId::Object(id));
            groups_touched.push(gid);
        }
        let mut objects_touched = vec![id];
        if let Some(base) = merged_base {
            // The merged sweep consumed the path's own solid (design §3b).
            self.objects[base].hidden = true;
            objects_touched.push(base);
        }

        self.undo.push(DocAction::CreatedObject {
            id,
            sketch,
            removed,
            emptied,
            merged_base,
        });
        self.redo.clear();
        self.debug_validate();

        let change = DocChange {
            objects_touched,
            sketches_touched: vec![sketch],
            groups_touched,
            instances_touched,
            components_touched,
            guides_touched: Vec::new(),
        };
        (id, change)
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

    /// [`Document::boolean`] between two members of the SAME component
    /// definition (component-edit-parity.md phase K2): unions/subtracts/
    /// intersects `a` and `b` — both members of `component` — into a new
    /// member that replaces them; every instance of `component` sees the
    /// result at once. `a`/`b` map with the identity transform (a
    /// definition's members already share one local frame, exactly like two
    /// world objects share the world frame in [`Document::boolean`]) — no
    /// instance/pose is involved, matching this surface's literal signature
    /// (component-edit-parity.md, phase K2).
    ///
    /// Cross-ownership mixes — an operand that is a world object, or a
    /// member of a *different* definition — refuse typed
    /// ([`DocumentError::UnknownObject`]), matching the group-boolean
    /// instance-refusal precedent ([`DocumentError::BooleanOperandHasInstance`]):
    /// never an implicit re-homing, never mixed silently.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — `component` is stale/hidden.
    /// - [`DocumentError::UnknownObject`] — `a`/`b` is stale, hidden, or not
    ///   a member of `component`.
    /// - [`DocumentError::Boolean`] — `a == b` (degenerate contact), or the
    ///   engine itself refused (non-solid operand, empty result, …).
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn boolean_in_component(
        &mut self,
        component: ComponentId,
        a: ObjectId,
        b: ObjectId,
        op: BooleanOp,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "boolean_in_component", boolean_op = ?op);
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return Err(DocumentError::UnknownComponent);
        }
        if a == b {
            // A single object cannot be combined with itself (its faces
            // would be fully coincident — a degenerate contact); reject
            // before mutating.
            return Err(DocumentError::Boolean(BooleanError::DegenerateContact));
        }
        let rec_a = self
            .objects
            .get(a)
            .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
            .ok_or(DocumentError::UnknownObject)?;
        let rec_b = self
            .objects
            .get(b)
            .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
            .ok_or(DocumentError::UnknownObject)?;

        let mut result = Object::boolean(op, &rec_a.object, &rec_b.object, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;

        // Dissolve the coplanar seams the boolean introduced, preserving
        // coplanar edges the operands already had (`Document::boolean`'s
        // treatment).
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
            owner: ObjectOwner::Definition(component),
            name: None,
            tags: Vec::new(),
        });
        self.objects[a].hidden = true;
        self.objects[b].hidden = true;
        // Shared-geometry birth (component-edit-parity.md phase K2):
        // membership is structural; a hidden operand stays listed exactly
        // like `delete_def_member`'s tombstone (see `DocAction::DeletedDefMember`).
        self.components[component].members.push(id);
        self.undo.push(DocAction::Boolean { result: id, a, b });
        self.redo.clear();
        self.debug_validate();

        let instances_touched = self.instances_of(component);
        let change = DocChange {
            objects_touched: vec![a, b, id],
            components_touched: vec![component],
            instances_touched,
            ..Default::default()
        };
        Ok((id, change))
    }

    /// Explicitly combines two **tree nodes** — each a plain solid Object or a
    /// whole Group, mixed freely — consuming both operands
    /// (the group-ops design §2). Subtract is `a − b`.
    ///
    /// Each operand is first *composed*: its leaf solids (in tree order) are
    /// folded with the boolean engine's Union — the user explicitly asked for
    /// volume algebra, so the non-destructive container becomes one composite
    /// volume, disjoint members yielding a multi-shell composite. `op` then
    /// applies between the two composites; coplanar seams the composition and
    /// the op introduced are dissolved (imprints the original leaves already
    /// carried are preserved), and the result is split into connected
    /// components:
    ///
    /// - exactly one component → a single top-level Object (unnamed, base
    ///   material from operand `a`'s first leaf), like [`Document::boolean`];
    /// - several solids → one top-level **result group** named from the
    ///   operands (`"A − B"`), holding one Object per solid. A **cavity** (a
    ///   hollowing subtract leaves its walls as a negative-volume component)
    ///   is not a solid: it stays attached to the positive shell that
    ///   contains it ([`Object::split_solids`]) — never split out as an
    ///   inside-out "solid", and never fusing unrelated solids either.
    ///
    /// Both operand subtrees are hidden (tombstoned, never erased). One undo
    /// step ([`DocAction::BooleanNodes`]) restores them and removes the result,
    /// handle-stably.
    ///
    /// Every fallible step runs on clones before the document is touched, so
    /// any `Err` leaves the entire document untouched (the strong guarantee
    /// across a multi-solid compound op).
    ///
    /// # Errors
    /// - [`DocumentError::UnknownObject`] / `UnknownGroup` / `UnknownInstance`
    ///   — a stale or hidden operand handle.
    /// - [`DocumentError::BooleanOperandHasInstance`] — an operand is, or
    ///   contains, a component instance; explode it first (never an implicit
    ///   Make Unique).
    /// - [`DocumentError::BooleanOperandNotSolid`] — a leaf under an operand
    ///   is not watertight, naming the offending side.
    /// - [`DocumentError::BooleanOperandEmpty`] — an operand flattens to no
    ///   solids (defensive).
    /// - [`DocumentError::GroupedOperand`] — an operand is itself inside a
    ///   group (replacing ops consume operands and emit top-level results).
    /// - [`DocumentError::Boolean`] — the engine refused (degenerate contact
    ///   between members or operands, empty result, …), including
    ///   [`BooleanError::DegenerateContact`] for `a == b`.
    pub fn boolean_nodes(
        &mut self,
        op: BooleanOp,
        a: NodeId,
        b: NodeId,
    ) -> Result<(NodeId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "boolean_nodes", boolean_op = ?op);
        if a == b {
            // A node cannot be combined with itself (fully coincident faces —
            // a degenerate contact); reject before doing any work.
            return Err(DocumentError::Boolean(BooleanError::DegenerateContact));
        }
        let leaves_a = self.boolean_operand_leaves(a, Operand::A)?;
        let leaves_b = self.boolean_operand_leaves(b, Operand::B)?;
        // Replacing op: operands are consumed and the result lands at the top
        // level, so an operand inside some other group would orphan that
        // group's member list. Both operands top-level also means their
        // subtrees are disjoint (neither can contain the other).
        if self.node_parent(a).is_some() || self.node_parent(b).is_some() {
            return Err(DocumentError::GroupedOperand);
        }

        // ── Fallible geometry, entirely on clones (document untouched) ─────
        let composite_a = self.compose_operand_union(&leaves_a)?;
        let composite_b = self.compose_operand_union(&leaves_b)?;
        let mut result = Object::boolean(op, &composite_a, &composite_b, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;

        // Dissolve the coplanar seams the composition unions and the final op
        // introduced, preserving imprints the original leaves already carried
        // (drawn-but-unextruded face imprints are content, not seams) — the
        // same preserve rule as `boolean`, collected across every leaf.
        let preserve: Vec<_> = leaves_a
            .iter()
            .chain(leaves_b.iter())
            .flat_map(|&o| self.objects[o].object.coplanar_edge_segments())
            .collect();
        result.merge_coplanar_faces(&preserve);
        // Split disjoint volumes into discrete solids. A hollowing subtract
        // leaves cavity walls as their own negative-volume component;
        // `split_solids` assigns each cavity to the positive shell that
        // contains it (never minting an inside-out "solid"), while unrelated
        // solids still split out discretely.
        let pieces = result.split_solids();
        debug_assert!(!pieces.is_empty(), "a non-empty boolean result has faces");

        // ── Everything fallible has succeeded; commit (infallible) ─────────
        // The result-group name derives from the operands' display names —
        // resolved before hiding them (name lookups filter hidden records).
        let group_name = if pieces.len() > 1 {
            let sym = match op {
                BooleanOp::Union => "\u{222a}",     // ∪
                BooleanOp::Subtract => "\u{2212}",  // −
                BooleanOp::Intersect => "\u{2229}", // ∩
            };
            Some(format!(
                "{} {} {}",
                self.node_label(a),
                sym,
                self.node_label(b)
            ))
        } else {
            None
        };

        let mut hidden_operands = Vec::new();
        self.collect_subtree(a, &mut hidden_operands);
        self.collect_subtree(b, &mut hidden_operands);
        for &n in &hidden_operands {
            match n {
                NodeId::Object(id) => self.objects[id].hidden = true,
                NodeId::Group(id) => self.groups[id].hidden = true,
                NodeId::Instance(_) => unreachable!("instance operands were refused"),
            }
        }

        let result_group = group_name.map(|name| {
            self.groups.insert(GroupRecord {
                members: Vec::new(),
                parent: None,
                hidden: false,
                name: Some(name),
                tags: Vec::new(),
            })
        });
        let mut result_objects: Vec<ObjectId> = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let id = self.objects.insert(ObjectRecord {
                object: piece,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::World {
                    parent: result_group,
                },
                name: None,
                tags: Vec::new(),
            });
            result_objects.push(id);
        }
        if let Some(g) = result_group {
            self.groups[g].members = result_objects.iter().map(|&o| NodeId::Object(o)).collect();
        }

        let root = match result_group {
            Some(g) => NodeId::Group(g),
            None => NodeId::Object(result_objects[0]),
        };
        let change = boolean_nodes_change(&hidden_operands, &result_objects, result_group);
        self.undo.push(DocAction::BooleanNodes {
            a,
            b,
            hidden_operands,
            result_objects,
            result_group,
        });
        self.redo.clear();
        self.debug_validate();

        Ok((root, change))
    }

    /// Validate one [`Document::boolean_nodes`] operand and flatten it to its
    /// leaf solids: live, no instances anywhere in the subtree, at least one
    /// leaf, every leaf watertight. Read-only — the document is untouched.
    fn boolean_operand_leaves(
        &self,
        node: NodeId,
        which: Operand,
    ) -> Result<Vec<ObjectId>, DocumentError> {
        if !self.node_is_live(node) {
            return Err(match node {
                NodeId::Object(_) => DocumentError::UnknownObject,
                NodeId::Group(_) => DocumentError::UnknownGroup,
                NodeId::Instance(_) => DocumentError::UnknownInstance,
            });
        }
        // Instances are refused, never implicitly made unique
        // (the group-ops design §2.2) — as the operand itself or anywhere
        // in its subtree.
        if matches!(node, NodeId::Instance(_)) || !self.leaf_instances_under(node).is_empty() {
            return Err(DocumentError::BooleanOperandHasInstance);
        }
        let leaves = self.leaf_objects_under(node);
        if leaves.is_empty() {
            return Err(DocumentError::BooleanOperandEmpty);
        }
        for &o in &leaves {
            if self.objects[o].object.watertight() != WatertightState::Watertight {
                return Err(DocumentError::BooleanOperandNotSolid { which });
            }
        }
        Ok(leaves)
    }

    /// Fold one operand's leaves into a single composite volume with the
    /// boolean engine's Union, on clones, in tree order (deterministic).
    /// Group transforms are always baked (a group holds no pose), so members
    /// already share the world frame and map with the identity transform.
    fn compose_operand_union(&self, leaves: &[ObjectId]) -> Result<Object, DocumentError> {
        let mut iter = leaves.iter();
        let &first = iter.next().expect("operand leaves are non-empty");
        let mut acc = self.objects[first].object.clone();
        for &next in iter {
            acc = Object::boolean(
                BooleanOp::Union,
                &acc,
                &self.objects[next].object,
                &Transform::IDENTITY,
            )
            .map_err(DocumentError::Boolean)?;
        }
        Ok(acc)
    }

    /// A node's display label for result-group naming: its name when it has
    /// one, else its kind word. UI positional labels ("Object 2") are a
    /// presentation concern the kernel does not know.
    fn node_label(&self, node: NodeId) -> String {
        match node {
            NodeId::Object(id) => self.object_name(id).unwrap_or("Object").to_string(),
            NodeId::Group(id) => self.group_name(id).unwrap_or("Group").to_string(),
            NodeId::Instance(id) => self
                .instance_name(id)
                .map(str::to_string)
                .or_else(|| {
                    self.instance_def(id)
                        .and_then(|d| self.component_name(d).map(str::to_string))
                })
                .unwrap_or_else(|| "Component".to_string()),
        }
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

    /// [`Document::slice_node`] on a member of `instance`'s definition
    /// (component-edit-parity.md phase K2): cuts `object` — a live member of
    /// `instance`'s own definition — by a WORLD-space `plane`, mapped into
    /// definition-local space through the instance's pose⁻¹ (a plane maps
    /// unambiguously through ANY invertible pose — rotation, mirror, or
    /// non-uniform scale — so unlike a typed scalar this never refuses on
    /// scale; see [`Transform::apply_plane`]). The source member is hidden
    /// (tombstone); the two watertight pieces become new members of the
    /// SAME definition, seen by every instance at once.
    ///
    /// Returns `((positive, negative), DocChange)` — the piece on the
    /// plane's (local-mapped) normal side first, exactly like
    /// [`Document::slice_node`].
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — `instance` is stale/hidden.
    /// - [`DocumentError::Transform`] — the instance's pose fails to invert
    ///   (unreachable for a live instance in practice — see
    ///   [`Document::begin_sketch_on_plane_in_instance`]'s matching note).
    /// - [`DocumentError::UnknownObject`] — `object` is stale, hidden, or
    ///   not a member of `instance`'s own definition.
    /// - [`DocumentError::Slice`] — the cut is degenerate or misses the
    ///   solid (mapped to local space).
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn slice_def_member(
        &mut self,
        instance: InstanceId,
        object: ObjectId,
        plane: &Plane,
    ) -> Result<((ObjectId, ObjectId), DocChange), DocumentError> {
        let n = plane.normal();
        info!(target: "kernel::op", op = "slice_def_member", nx = n.x, ny = n.y, nz = n.z);
        let (component, pose) = self.instance_component(instance)?;
        let pose_inv = pose.inverse().map_err(DocumentError::Transform)?;
        let local_plane = pose_inv
            .apply_plane(plane)
            .map_err(DocumentError::Transform)?;
        let rec = self
            .objects
            .get(object)
            .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
            .ok_or(DocumentError::UnknownObject)?;
        let (positive, negative) = rec
            .object
            .slice(&local_plane)
            .map_err(DocumentError::Slice)?;

        let a = self.objects.insert(ObjectRecord {
            object: positive,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::Definition(component),
            name: None,
            tags: Vec::new(),
        });
        let b = self.objects.insert(ObjectRecord {
            object: negative,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::Definition(component),
            name: None,
            tags: Vec::new(),
        });
        self.objects[object].hidden = true;
        // Shared-geometry birth (component-edit-parity.md phase K2): see
        // `boolean_in_component`'s matching comment.
        self.components[component].members.push(a);
        self.components[component].members.push(b);
        self.undo.push(DocAction::Sliced {
            source: object,
            a,
            b,
        });
        self.redo.clear();
        self.debug_validate();

        let instances_touched = self.instances_of(component);
        let change = DocChange {
            objects_touched: vec![object, a, b],
            components_touched: vec![component],
            instances_touched,
            ..Default::default()
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

    /// [`Document::push_pull_through`] for a member of `component`.
    ///
    /// The source member is replaced by the connected result pieces inside the
    /// same shared definition, so every instance updates together. The
    /// operation records the same visibility-flipping [`DocAction::PushThrough`]
    /// as the world-space sibling; ownership on the source/result records is
    /// what makes undo and redo report the component and all of its instances.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — `component` is stale/hidden.
    /// - [`DocumentError::UnknownObject`] — `object` is stale, hidden, or not a
    ///   member of `component`.
    /// - The same typed push/pull errors as [`Document::push_pull_through`].
    ///
    /// On `Err` the document is untouched.
    pub fn push_pull_through_in_component(
        &mut self,
        component: ComponentId,
        object: ObjectId,
        face: crate::ids::FaceId,
        distance: f64,
    ) -> Result<(Vec<ObjectId>, DocChange), DocumentError> {
        info!(
            target: "kernel::op",
            op = "push_pull_through_in_component",
            distance
        );
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return Err(DocumentError::UnknownComponent);
        }
        let rec = self
            .objects
            .get(object)
            .filter(|r| !r.hidden && r.owner == ObjectOwner::Definition(component))
            .ok_or(DocumentError::UnknownObject)?;
        let result = rec
            .object
            .push_through(face, distance)
            .map_err(|e| DocumentError::Op(KernelOpError::PushPull(e)))?;
        let pieces = result.split_connected_components();

        let mut results = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let id = self.objects.insert(ObjectRecord {
                object: piece,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::Definition(component),
                name: None,
                tags: Vec::new(),
            });
            self.components[component].members.push(id);
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
        Ok((
            results,
            DocChange {
                objects_touched,
                components_touched: vec![component],
                instances_touched: self.instances_of(component),
                ..Default::default()
            },
        ))
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

    /// [`Document::transform_object`] on a member of `instance`'s
    /// definition (component-edit-parity.md phase K2): bakes a WORLD-space
    /// gesture `t` into `object` — a live member of `instance`'s own
    /// definition — by first conjugating it into definition-local space
    /// through the instance's pose: `local_t = pose · t · pose⁻¹` (applied
    /// as `pose.then(t).then(&pose_inv)`, so `local_t` maps a local point
    /// exactly the way `t` maps that point's world image). Every instance of
    /// the definition sees the edit at once.
    ///
    /// **Non-uniform-scale posture** (the design's "Coordinate mapping"
    /// question, decided here): unlike a scalar distance or a `stop_len`
    /// ([`Document::extrude_region_in_instance`],
    /// [`Document::map_world_distance_through_pose`]), a full affine
    /// conjugation is **never ambiguous** — `local_t` is uniquely determined
    /// by `t` and the pose, however the pose scales, mirrors, or shears.
    /// Matrix conjugation also has a sharp consequence: `det(local_t) =
    /// det(pose)·det(t)·det(pose⁻¹) = det(t)`, independent of the pose
    /// entirely — a proper (non-reflecting) world gesture conjugates to a
    /// proper local one through ANY invertible pose, mirrored instances
    /// included, and a reflecting `t` is refused by
    /// [`Object::apply_transform`] exactly as it would be in world space.
    /// So this refuses ONLY where [`Document::transform_object`] would —
    /// singular or orientation-flipping `t` — never on account of the
    /// instance's own scale; there is no typed-scalar rule to trip here (the
    /// design's "only refuse where the typed-scalar rule applies" resolves
    /// to "nowhere" for a full affine).
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — `instance` is stale/hidden.
    /// - [`DocumentError::Transform`] — the instance's pose fails to invert
    ///   (unreachable for a live instance in practice), or `t` (equivalently
    ///   `local_t`) is singular or orientation-flipping.
    /// - [`DocumentError::UnknownObject`] — `object` is stale, hidden, or
    ///   not a member of `instance`'s own definition.
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn transform_def_member(
        &mut self,
        instance: InstanceId,
        object: ObjectId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "transform_def_member");
        let (component, pose) = self.instance_component(instance)?;
        let pose_inv = pose.inverse().map_err(DocumentError::Transform)?;
        // local_t maps a LOCAL point p the way `t` maps its world image:
        // world = pose(p); world' = t(world); p' = pose⁻¹(world').
        let local_t = pose.then(t).then(&pose_inv);
        // Capture the inverse first: it both validates invertibility and is
        // what undo will bake (mirrors `transform_object`).
        let inverse = local_t.inverse().map_err(DocumentError::Transform)?;
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden && rec.owner == ObjectOwner::Definition(component) => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        rec.object
            .apply_transform(&local_t)
            .map_err(DocumentError::Transform)?;
        self.undo.push(DocAction::Transform {
            objects: vec![object],
            forward: local_t,
            inverse,
        });
        self.redo.clear();
        self.debug_validate();

        let instances_touched = self.instances_of(component);
        Ok(DocChange {
            objects_touched: vec![object],
            components_touched: vec![component],
            instances_touched,
            ..Default::default()
        })
    }

    /// Transform a definition-owned sketch through `instance` using the same
    /// world-gesture conjugation as [`Document::transform_def_member`].
    pub fn transform_def_sketch(
        &mut self,
        instance: InstanceId,
        sketch: SketchId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        let (component, pose) = self.instance_component(instance)?;
        if self.sketch_owner_component(sketch) != Some(component) {
            return Err(DocumentError::UnknownSketch);
        }
        let pose_inv = pose.inverse().map_err(DocumentError::Transform)?;
        let local_t = pose.then(t).then(&pose_inv);
        let mut change = self.transform_sketch(sketch, &local_t)?;
        change.components_touched = vec![component];
        change.instances_touched = self.instances_of(component);
        Ok(change)
    }

    /// Transform one island of a definition-owned sketch through `instance`.
    /// Out-of-plane detaches remain owned by the same definition.
    pub fn transform_def_sketch_island(
        &mut self,
        instance: InstanceId,
        sketch: SketchId,
        island: SketchIslandId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        let (component, pose) = self.instance_component(instance)?;
        if self.sketch_owner_component(sketch) != Some(component) {
            return Err(DocumentError::UnknownSketch);
        }
        let pose_inv = pose.inverse().map_err(DocumentError::Transform)?;
        let local_t = pose.then(t).then(&pose_inv);
        let mut change = self.transform_sketch_island(sketch, island, &local_t)?;
        change.components_touched = vec![component];
        change.instances_touched = self.instances_of(component);
        Ok(change)
    }

    /// Transform a mixed selection inside one component definition as one
    /// atomic, undoable gesture. The supplied affine is in world space and is
    /// conjugated through `instance` by the ordinary per-target operations.
    ///
    /// The document snapshot is the transaction boundary: any stale target or
    /// geometric refusal restores the exact pre-call state, including history
    /// stacks and any detached sketch an earlier island transform created.
    pub fn transform_def_selection(
        &mut self,
        instance: InstanceId,
        objects: &[ObjectId],
        sketches: &[SketchId],
        islands: &[(SketchId, SketchIslandId)],
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        if objects.is_empty() && sketches.is_empty() && islands.is_empty() {
            return Err(DocumentError::EmptySelection);
        }
        let checkpoint = self.clone();
        let undo_start = self.undo.actions.len();
        let mut change = DocChange::default();
        let result = (|| {
            for &(sketch, island) in islands {
                merge_doc_change(
                    &mut change,
                    self.transform_def_sketch_island(instance, sketch, island, t)?,
                );
            }
            for &sketch in sketches {
                merge_doc_change(&mut change, self.transform_def_sketch(instance, sketch, t)?);
            }
            for &object in objects {
                merge_doc_change(&mut change, self.transform_def_member(instance, object, t)?);
            }
            Ok::<(), DocumentError>(())
        })();
        if let Err(error) = result {
            *self = checkpoint;
            return Err(error);
        }
        let actions: Vec<DocAction> = self.undo.actions.drain(undo_start..).collect();
        self.undo.push(DocAction::Compound { actions });
        self.redo.clear();
        self.debug_validate();
        Ok(change)
    }

    /// Validation-only sibling of [`Document::transform_def_sketch_island`].
    pub fn validate_transform_def_sketch_island(
        &self,
        instance: InstanceId,
        sketch: SketchId,
        island: SketchIslandId,
        t: &Transform,
    ) -> Result<(), DocumentError> {
        let (component, pose) = self.instance_component(instance)?;
        if self.sketch_owner_component(sketch) != Some(component) {
            return Err(DocumentError::UnknownSketch);
        }
        let pose_inv = pose.inverse().map_err(DocumentError::Transform)?;
        let local_t = pose.then(t).then(&pose_inv);
        self.validate_transform_sketch_island(sketch, island, &local_t)
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

    /// Rigidly move ONE island of a free-standing sketch (the per-shape
    /// Move / Rotate / Scale). Three arms, chosen by where `t` lands the
    /// island:
    ///
    /// - **In-plane** (every vertex stays on the sketch plane): baked in
    ///   place ([`Sketch::apply_transform_island`]). Landings that would
    ///   cross or merge with other islands' geometry are refused with a
    ///   typed error, never welded. Undoable via
    ///   [`DocAction::TransformSketchIsland`]; every handle is stable.
    /// - **Out-of-plane, sole island**: the island IS the sketch, so this
    ///   is a whole-sketch bake — delegates to
    ///   [`Document::transform_sketch`] (plane remaps, `SketchId` and all
    ///   sketch-element handles stay stable).
    /// - **Out-of-plane, shared sketch**: a sketch is planar, so the island
    ///   cannot stay; it DETACHES into a new sketch on the transformed
    ///   plane. The remaining islands are untouched; curve chains keep
    ///   their identity and their analytic [`CurveGeom`] maps under the
    ///   usual map-or-drop contract ([`Sketch::rebuild_island_transformed`]).
    ///   The new sketch's element handles are fresh (a slotmap cannot mint
    ///   keys into another sketch); callers re-query, as after any
    ///   reshaping mutation. Undoable via
    ///   [`DocAction::DetachedSketchIsland`]. The returned
    ///   [`DocChange::sketches_touched`] lists `[source, detached]`.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownSketch`] — stale or hidden sketch.
    /// - [`DocumentError::Sketch`] — stale island
    ///   ([`SketchError::UnknownIsland`]), or an in-plane landing that would
    ///   cross or merge other geometry ([`SketchError::WouldRetopologize`]).
    /// - [`DocumentError::Transform`] — singular or orientation-flipping
    ///   map.
    ///
    /// Every error leaves the document untouched (strong guarantee).
    pub fn transform_sketch_island(
        &mut self,
        sketch: SketchId,
        island: SketchIslandId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        let inverse = t.inverse().map_err(DocumentError::Transform)?;
        if t.determinant() < 0.0 {
            return Err(DocumentError::Transform(TransformError::Reflection));
        }
        if !self.sketches.contains_key(sketch) || self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        let anchor_before = self.sketches[sketch].island_anchor(island);
        match self.sketches[sketch].apply_transform_island(island, t) {
            Ok(()) => {
                let anchor_after = self.sketches[sketch]
                    .island_anchor(island)
                    .expect("island live immediately after its own transform");
                self.undo.push(DocAction::TransformSketchIsland {
                    sketch,
                    island,
                    forward: *t,
                    inverse,
                    anchor_before: anchor_before
                        .expect("island live immediately before its own transform"),
                    anchor_after,
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
            Err(SketchError::PointOffPlane { .. }) => {
                // The transform leaves the sketch plane. (apply_transform_
                // island's strong guarantee: nothing has changed yet.)
                if self.sketches[sketch].islands().len() == 1 {
                    // The island IS the sketch: whole-sketch bake.
                    return self.transform_sketch(sketch, t);
                }
                self.detach_transformed_island(sketch, island, t)
            }
            Err(e) => Err(DocumentError::Sketch(e)),
        }
    }

    /// [`Document::transform_sketch_island`]'s acceptance, without the
    /// commit: `Ok(())` iff the same call would succeed AGAINST THE CURRENT
    /// STATE. Batch movers (the app's multi-island Move) validate EVERY
    /// island first so one refusal aborts the whole gesture before anything
    /// commits.
    ///
    /// The contract is SUCCESS-equivalence, not MECHANISM-equivalence: the
    /// commit's routing between its three arms (in-plane bake / sole-island
    /// whole-sketch bake / detach) is decided against COMMIT-TIME state, so
    /// a validate-all-then-commit-sequentially batch over ALL islands of one
    /// sketch can see a later commit take a different arm than validation
    /// observed — an earlier island's detach can leave a later island the
    /// sketch's sole one, rerouting its out-of-plane commit from "detach"
    /// to a whole-sketch bake. The commit still succeeds, the document
    /// stays sound, and undo restores exactly (pinned by
    /// `batch_commit_over_all_islands_may_reroute_after_earlier_detach` in
    /// `document_specs.rs`). Every real app caller folds full island
    /// coverage into one whole-sketch transform first, so a surviving
    /// sibling island always keeps the observed mechanism in practice.
    pub fn validate_transform_sketch_island(
        &self,
        sketch: SketchId,
        island: SketchIslandId,
        t: &Transform,
    ) -> Result<(), DocumentError> {
        t.inverse().map_err(DocumentError::Transform)?;
        if t.determinant() < 0.0 {
            return Err(DocumentError::Transform(TransformError::Reflection));
        }
        if !self.sketches.contains_key(sketch) || self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        let s = &self.sketches[sketch];
        match s.validate_transform_island(island, t) {
            Ok(()) => Ok(()),
            Err(SketchError::PointOffPlane { .. }) => {
                // Out-of-plane arms: whole-sketch bake needs only the
                // already-validated transform; detach additionally needs the
                // island to replay onto the mapped plane.
                let plane = t
                    .apply_plane(&s.plane())
                    .map_err(DocumentError::Transform)?;
                if s.islands().len() == 1 {
                    return Ok(());
                }
                s.rebuild_island_transformed(island, t, plane)
                    .map(|_| ())
                    .map_err(DocumentError::Sketch)
            }
            Err(e) => Err(DocumentError::Sketch(e)),
        }
    }

    /// The out-of-plane detach arm of [`Document::transform_sketch_island`]:
    /// rebuilds the island with `t` baked in as a NEW sketch on the mapped
    /// plane, removes the island's edges from `source`, and records
    /// [`DocAction::DetachedSketchIsland`]. The caller has already vetted
    /// `t` (invertible, det > 0) and that `source` is live. The detached
    /// sketch is built COMPLETELY before anything mutates, so every typed
    /// failure leaves the document untouched.
    fn detach_transformed_island(
        &mut self,
        source: SketchId,
        island: SketchIslandId,
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        let src = &self.sketches[source];
        let plane = t
            .apply_plane(&src.plane())
            .map_err(DocumentError::Transform)?;
        let fresh = src
            .rebuild_island_transformed(island, t, plane)
            .map_err(DocumentError::Sketch)?;
        // Undo rows: the island's edges at their PRE-transform positions
        // with their source curve ids (curve slots outlive their edges, so
        // a later undo re-links surviving analytic identity) — the
        // extrusion-consumption shape, restored the same way.
        let isl = &src.islands()[island];
        let removed: Vec<(Point3, Point3, Option<SketchCurveId>)> = isl
            .edges
            .iter()
            .map(|&eid| {
                let e = src.edges()[eid];
                (
                    src.vertices()[e.from].position,
                    src.vertices()[e.to].position,
                    e.curve,
                )
            })
            .collect();
        let scaffolding: std::collections::BTreeSet<SketchEdgeId> =
            isl.edges.iter().copied().collect();

        // Nothing left can fail; commit.
        self.sketches[source].remove_edges(&scaffolding);
        let detached = self.sketches.insert(fresh);
        if let Some(component) = self.sketch_owner_component(source) {
            self.def_sketches.insert(detached, component);
        }
        self.undo.push(DocAction::DetachedSketchIsland {
            source,
            detached,
            removed,
        });
        self.redo.clear();
        self.debug_validate();

        let (components_touched, instances_touched) = self.def_sketch_owner_change(source);
        Ok(DocChange {
            objects_touched: Vec::new(),
            sketches_touched: vec![source, detached],
            groups_touched: Vec::new(),
            instances_touched,
            components_touched,
            guides_touched: Vec::new(),
        })
    }

    /// Copy a SET of islands of a free-standing sketch onto ONE NEW sketch
    /// with `t` baked in, leaving the SOURCE untouched. This is the
    /// out-of-plane arm of Move+Alt's sketch copy: an in-plane copy replays
    /// into the source sketch (the UI's gesture-replay path), but a sketch is
    /// planar, so islands copied off its plane land on a sketch of their own —
    /// on the transformed plane. It is [`Document::detach_transformed_island`]
    /// WITHOUT the source removal, generalized to many islands: the same
    /// [`Sketch::rebuild_islands_transformed`] replay, so regions re-derive as
    /// for freshly drawn geometry and curve chains keep their identity and
    /// analytic [`CurveGeom`] (a copied circle is a true circle,
    /// center-snappable).
    ///
    /// All of `islands` land on ONE sketch, which is what preserves a region's
    /// HOLES: a hole boundary is a separate island, so copying a donut's outer
    /// and inner loops must keep them together or the ring would re-derive as
    /// a plain solid. A caller copying a WHOLE sketch passes every island; a
    /// subset copies just those (dropping a hole then is the user's selection,
    /// not silent repair). `t` need not be out-of-plane — the copy is
    /// agnostic; the caller routes in-plane copies elsewhere.
    ///
    /// Recorded as [`DocAction::CopiedSketchIslands`]: undo hides the copy,
    /// redo unhides it (its contents survive hiding bit-exactly, so the
    /// returned `SketchId` stays valid across undo/redo). One call is ONE undo
    /// step regardless of island count. Returns the new sketch id and a
    /// [`DocChange`] whose `sketches_touched` is `[copy]`.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownSketch`] — stale or hidden source.
    /// - [`DocumentError::Sketch`] — any stale island
    ///   ([`SketchError::UnknownIsland`]), or a replay the sticky machinery
    ///   refuses (e.g. a tolerance-collapsing scale).
    /// - [`DocumentError::Transform`] — singular or orientation-flipping map.
    ///
    /// The copy is built completely before anything mutates, so every error
    /// leaves the document untouched (strong guarantee).
    pub fn copy_sketch_islands(
        &mut self,
        source: SketchId,
        islands: &[SketchIslandId],
        t: &Transform,
    ) -> Result<(SketchId, DocChange), DocumentError> {
        t.inverse().map_err(DocumentError::Transform)?;
        if t.determinant() < 0.0 {
            return Err(DocumentError::Transform(TransformError::Reflection));
        }
        if !self.sketches.contains_key(source) || self.hidden_sketches.contains(&source) {
            return Err(DocumentError::UnknownSketch);
        }
        let src = &self.sketches[source];
        let plane = t
            .apply_plane(&src.plane())
            .map_err(DocumentError::Transform)?;
        let fresh = src
            .rebuild_islands_transformed(islands, t, plane)
            .map_err(DocumentError::Sketch)?;

        // Nothing left can fail; commit. The source is not touched.
        let copy = self.sketches.insert(fresh);
        self.undo.push(DocAction::CopiedSketchIslands { copy });
        self.redo.clear();
        self.debug_validate();

        Ok((
            copy,
            DocChange {
                objects_touched: Vec::new(),
                sketches_touched: vec![copy],
                groups_touched: Vec::new(),
                instances_touched: Vec::new(),
                components_touched: Vec::new(),
                guides_touched: Vec::new(),
            },
        ))
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
    /// The new component inherits its display identity from the selection.
    /// A single selected node passes its name on as the **definition name**
    /// (the shared label every instance of this component displays) and its
    /// tags onto the new **instance** (tags attach to placements, never to
    /// definitions — one definition may later be placed under different
    /// tags). The source node keeps its own name and tags, so undo restores
    /// it exactly as it was. A selection with no name to inherit — an
    /// unnamed node, or multiple siblings — gets a generated definition name
    /// (`"Component 1"`, `"Component 2"`, …: the lowest number no live
    /// definition already uses), so every definition always has a name and
    /// all instances of one definition always read identically.
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

        // Inherit display identity from a single-node selection (see the doc
        // comment): its name becomes the definition name, its tags copy onto
        // the new instance. The source keeps both, so undo is exact. With no
        // name to inherit, generate one — a definition always has a name.
        let (inherited_name, inherited_tags) = match members {
            [single] => self.node_meta(*single)?,
            _ => (None, Vec::new()),
        };
        let def_name = inherited_name.unwrap_or_else(|| self.generated_component_name());

        // Build the definition + its single identity-posed instance. No geometry
        // moves: the definition-local frame is the world frame at creation.
        let component = self.components.insert(ComponentDef {
            members: leaves.clone(),
            hidden: false,
            name: Some(def_name),
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
            tags: inherited_tags,
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

    /// The 3D Text placement pipeline's atomic tail (docs/design/3d-text.md).
    /// The app has already injected a text run's glyph outlines as edges
    /// into `sketch` (one shared sketch per placement, every glyph at its
    /// own advance-width offset — letters never interact since real text
    /// doesn't overlap itself) via the ordinary sketch-gesture bracket
    /// (`begin_sketch_gesture` … `end_sketch_gesture`, closed immediately
    /// before this call with nothing else committed in between — the caller
    /// contract this function verifies before touching anything), and has
    /// resolved which of the sketch's closed regions are glyph FILL versus
    /// which are counter-INTERIOR cells: the kernel's region resolver
    /// reports every closed region a nested pair of contours forms — an 'O'
    /// traces as the ring (fill, with the counter as a hole) AND the
    /// counter's own interior standing alone as a second, separate region
    /// (see `Document::extrudable_regions`'s doc and the concentric-rings
    /// test in `sketch.rs`) — so the CALLER selects exactly `regions` (the
    /// fill ones, by the font's own nonzero-winding fill rule) rather than
    /// this call extruding every region blindly. The kernel still does all
    /// the actual geometric heavy lifting: overlapping/self-touching glyph
    /// contours split and close exactly like hand-drawn strokes, and a
    /// selected fill region's counter arrives already closed as a hole by
    /// construction (`Sketch::profile`) — the app only decides WHICH
    /// resolved regions represent material, never how their boundaries
    /// resolve.
    ///
    /// Every region in `regions` extrudes `distance` (exactly
    /// [`Document::extrude_region`]'s own machinery), and the resulting
    /// solids fold into ONE new component definition named `name`, whose
    /// one identity-posed instance is returned — SketchUp's "3D Text"
    /// parity (the maker can Move/Rotate/Scale or explode it like any
    /// instance).
    ///
    /// `group`, when given, births every extruded solid — and so the folded
    /// instance — inside that group, mirroring
    /// [`Document::follow_me_grouped`]; `None` births top-level.
    ///
    /// Recorded as ONE [`DocAction::PlaceTextCompound`] bundling the
    /// gesture's own [`DocAction::SketchGesture`] with each region's
    /// [`DocAction::CreatedObject`] and the fold's [`DocAction::MadeComponent`]
    /// — a single undo removes the whole placement, a single redo restores
    /// it.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownGroup`] — `group` is stale or hidden.
    /// - [`DocumentError::EmptyComponent`] — `regions` is empty (nothing to
    ///   place); the just-closed gesture, if any, is left exactly as the
    ///   caller committed it.
    /// - [`DocumentError::UnexpectedGestureState`] — the top of the undo
    ///   stack was not the `SketchGesture` this call expects to fold in
    ///   (the caller contract above was violated). The document is
    ///   untouched.
    /// - [`DocumentError::Sketch`] / [`DocumentError::Extrude`] — a region's
    ///   profile tracing or extrusion failed (a glyph pathological enough
    ///   that the kernel's own validation refuses it).
    ///
    /// On `Err` the document is untouched: every fallible step (profile
    /// tracing, extrusion) for EVERY region runs to completion before
    /// anything commits, exactly like [`Document::extrude_region`]'s own
    /// strong guarantee, extended across the whole glyph run.
    pub fn place_text(
        &mut self,
        sketch: SketchId,
        regions: &[SketchRegionId],
        distance: f64,
        name: String,
        group: Option<GroupId>,
    ) -> Result<(ComponentId, InstanceId, DocChange), DocumentError> {
        if let Some(gid) = group
            && (!self.groups.contains_key(gid) || self.groups[gid].hidden)
        {
            return Err(DocumentError::UnknownGroup);
        }
        if self.hidden_sketches.contains(&sketch) {
            return Err(DocumentError::UnknownSketch);
        }
        if regions.is_empty() {
            return Err(DocumentError::EmptyComponent);
        }

        // Validate & build every extrusion before committing anything (the
        // strong guarantee) — `extrude_region`'s own single-region body,
        // run once per region ahead of any commit.
        let s = self
            .sketches
            .get(sketch)
            .ok_or(DocumentError::UnknownSketch)?;
        let mut built: Vec<(BTreeSet<SketchEdgeId>, Object)> = Vec::with_capacity(regions.len());
        for &region in regions {
            let profile = s.profile(region).map_err(DocumentError::Sketch)?;
            let scaffolding = s
                .region_scaffolding(region)
                .map_err(DocumentError::Sketch)?;
            let object =
                Object::from_extrusion(&profile, distance).map_err(DocumentError::Extrude)?;
            built.push((scaffolding, object));
        }

        // Fold in the glyph-injection gesture the caller just closed — the
        // caller contract above. Nothing has committed yet, so a mismatch
        // here leaves the document exactly as found.
        let Some(top) = self.undo.pop() else {
            return Err(DocumentError::UnexpectedGestureState);
        };
        if !matches!(&top, DocAction::SketchGesture { sketch: gs, .. } if *gs == sketch) {
            self.undo.push(top);
            return Err(DocumentError::UnexpectedGestureState);
        }
        let mut bundle = vec![top];

        let mut object_ids = Vec::with_capacity(built.len());
        let mut extra_groups_touched = Vec::new();
        for (scaffolding, object) in built {
            let (id, change) = self.commit_region_object(sketch, &scaffolding, object, None, group);
            object_ids.push(id);
            extra_groups_touched.extend(change.groups_touched);
            bundle.push(
                self.undo
                    .pop()
                    .expect("commit_region_object just pushed CreatedObject"),
            );
        }

        // Anything still standing in the sketch belongs to un-extruded
        // counter/interior regions the caller never selected — a glyph's
        // hole (the 'O'/'D'/'e' counters), never scaffolding of an object
        // we just created. This sketch exists solely for this one
        // placement (docs/design/3d-text.md), so nothing legitimate is
        // lost discarding whatever remains: the whole sketch retires with
        // the placement rather than lingering as a live, reachable scratch
        // entity (Finding 2). A glyph run with no counters already emptied
        // (and hid) the sketch via the last region's own extrusion above,
        // so this is a no-op then — no action is recorded.
        if !self.hidden_sketches.contains(&sketch) {
            let sk = self
                .sketches
                .get(sketch)
                .expect("sketch confirmed live above");
            let remaining: BTreeSet<SketchEdgeId> = sk.edges().keys().collect();
            if !remaining.is_empty() {
                let removed: Vec<(Point3, Point3, Option<SketchCurveId>)> = remaining
                    .iter()
                    .map(|&eid| {
                        let e = sk.edges()[eid];
                        (
                            sk.vertices()[e.from].position,
                            sk.vertices()[e.to].position,
                            e.curve,
                        )
                    })
                    .collect();
                let sk = self
                    .sketches
                    .get_mut(sketch)
                    .expect("sketch confirmed live above");
                sk.remove_edges(&remaining);
                debug_assert!(
                    sk.edges().is_empty(),
                    "place_text's scratch sketch holds only its own injected glyph edges"
                );
                self.hidden_sketches.insert(sketch);
                bundle.push(DocAction::ConsumedScaffolding { sketch, removed });
            }
        }

        let members: Vec<NodeId> = object_ids.iter().copied().map(NodeId::Object).collect();
        let (component, instance, mut change) = self
            .make_component(&members)
            .expect("fresh top-level siblings sharing one parent cannot fail make_component");
        bundle.push(
            self.undo
                .pop()
                .expect("make_component just pushed MadeComponent"),
        );
        // The definition's real name: `make_component`'s own naming only
        // ever sees an unnamed multi-member selection here and generates
        // "Component N". `MadeComponent`'s undo/redo never touches
        // `.name`, only `.hidden` — so this direct set is exact across
        // undo and redo alike, with no separate undoable rename step.
        self.components[component].name = Some(name);

        self.undo.push(DocAction::PlaceTextCompound(bundle));
        self.redo.clear();
        self.debug_validate();

        change.sketches_touched.push(sketch);
        change.groups_touched.extend(extra_groups_touched);
        Ok((component, instance, change))
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

    /// Removes one member Object from a component definition — the
    /// definition-member analog of [`Document::delete_node`] (component-edit-
    /// parity.md phase K1). Like every other delete, this is a tombstone:
    /// `object` is hidden, never erased, so its `ObjectId` stays valid for
    /// redo. It stays listed in `ComponentDef.members` (hidden) exactly like
    /// an undone member-birth would — see [`DocAction::DeletedDefMember`] —
    /// so unlike [`Document::delete_node`] there is no list surgery to undo.
    /// The change is seen by every instance of the definition at once,
    /// exactly like [`Document::apply_def_op`].
    ///
    /// Refuses to delete a definition's **last** live member: SketchUp
    /// deletes the now-empty component outright, but v1 refuses instead —
    /// deleting the last member reads as "the user wants the instances
    /// gone," which is [`Document::delete_node`] on each instance, not this.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — `component` is stale/hidden.
    /// - [`DocumentError::UnknownObject`] — `object` is not a live member of
    ///   `component`.
    /// - [`DocumentError::LastDefinitionMember`] — `object` is the
    ///   definition's only live member.
    ///
    /// On `Err` the document is untouched (the strong guarantee).
    pub fn delete_def_member(
        &mut self,
        component: ComponentId,
        object: ObjectId,
    ) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "delete_def_member");
        let comp = match self.components.get(component) {
            Some(c) if !c.hidden => c,
            _ => return Err(DocumentError::UnknownComponent),
        };
        if !comp.members.contains(&object) {
            return Err(DocumentError::UnknownObject);
        }
        if self.objects.get(object).is_none_or(|r| r.hidden) {
            return Err(DocumentError::UnknownObject);
        }
        let live_members = comp
            .members
            .iter()
            .filter(|&&o| self.objects.get(o).is_some_and(|r| !r.hidden))
            .count();
        if live_members <= 1 {
            return Err(DocumentError::LastDefinitionMember);
        }

        self.objects[object].hidden = true;

        self.undo
            .push(DocAction::DeletedDefMember { component, object });
        self.redo.clear();
        self.debug_validate();

        Ok(DocChange {
            objects_touched: vec![object],
            components_touched: vec![component],
            instances_touched: self.instances_of(component),
            ..Default::default()
        })
    }

    /// Detach an instance into independent world geometry — "Explode".
    /// Each definition member is cloned, the instance pose is **baked** into the
    /// clone (reusing [`Object::apply_transform`]), and the clones are inserted
    /// as top-level world objects at the instance's parent; the instance is then
    /// hidden. Each clone keeps its member's own tags, and for a
    /// **single-member** definition the baked name is exactly the identity
    /// the UI displays for the instance: the instance's own name, else the
    /// **live definition name** (which a later [`Document::set_component_name`]
    /// may have changed — the member record keeps only its stale pre-fold
    /// name), else the member's own name. A multi-member definition keeps
    /// each member's own name regardless (stamping one shared name onto
    /// several objects would mint duplicates). The instance record itself is
    /// only hidden, never edited, so undo restores it name and all. The
    /// definition and any sibling instances are untouched. Recorded as
    /// [`DocAction::Exploded`]; handle-stable and reversible.
    ///
    /// Also copies each LIVE def-owned sketch the definition held
    /// (component-edit-parity.md phase K1 follow-up — a not-yet-extruded
    /// profile drawn into the component does not silently disappear from
    /// the exploded result) into an independent WORLD sketch, baking the
    /// same pose the members bake, via [`Sketch::apply_transform`] — the
    /// established sketch transform/detach machinery, which maps curve
    /// analytic identity exactly for a similarity and would otherwise DROP
    /// it (see [`DocumentError::CannotExplodeNonUniformScale`]). The
    /// definition's OWN sketch is untouched — like the members, this
    /// copies shared content, it never moves it, so sibling instances keep
    /// seeing the original.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownInstance`] — the instance is stale/hidden.
    /// - [`DocumentError::CannotExplodeReflected`] — the pose mirrors
    ///   (determinant < 0); baking it would invert winding. Use
    ///   [`Document::make_unique`] then edit instead, or unmirror first.
    /// - [`DocumentError::CannotExplodeNonUniformScale`] — the definition
    ///   holds a live def-owned sketch and the pose is not a similarity
    ///   (non-uniform scale): the sketch's curve identity cannot map
    ///   exactly.
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
        let members: Vec<ObjectId> = match self.components.get(def) {
            Some(c) if !c.hidden => c
                .members
                .iter()
                .copied()
                .filter(|&m| self.objects.get(m).is_some_and(|r| !r.hidden))
                .collect(),
            // The instance held a live def by invariant; treat otherwise as a bug.
            _ => return Err(DocumentError::UnknownComponent),
        };
        // Live def-owned sketches to copy alongside the members (a hidden/
        // consumed husk has nothing left to show and is left with the
        // definition, exactly like `make_unique`'s clone skips it).
        let def_owned_sketches: Vec<SketchId> = self
            .def_sketches
            .iter()
            .filter(|&(sid, &c)| c == def && !self.hidden_sketches.contains(sid))
            .map(|(&sid, _)| sid)
            .collect();
        // A non-similarity pose cannot carry a sketch's curve identity
        // exactly (`Sketch::apply_transform`'s map-or-drop contract would
        // drop it) — refuse before any mutation rather than degrade it
        // silently. Moot when there is nothing to bake.
        if !def_owned_sketches.is_empty() && pose.similarity_scale().is_none() {
            return Err(DocumentError::CannotExplodeNonUniformScale);
        }

        // Clone each member and bake the pose into the copy as an independent
        // world object in the instance's container. `pose` is invertible and
        // orientation-preserving, so `apply_transform` cannot fail. The clone
        // keeps the member's own tags (preserved on the member since before
        // it was folded in); for a single-member definition its name is the
        // identity the UI displays for the instance — instance name, else the
        // LIVE definition name (the member record's name goes stale the
        // moment set_component_name renames the definition), else the
        // member's own pre-fold name (see the doc comment).
        let instance_name = self.instances[instance].name.clone();
        let def_name = self.components[def].name.clone();
        let single_member = members.len() == 1;
        let mut created: Vec<ObjectId> = Vec::with_capacity(members.len());
        for m in members {
            let mut object = self.objects[m].object.clone();
            object
                .apply_transform(&pose)
                .map_err(DocumentError::Transform)?;
            let name = if single_member {
                instance_name
                    .clone()
                    .or_else(|| def_name.clone())
                    .or_else(|| self.objects[m].name.clone())
            } else {
                self.objects[m].name.clone()
            };
            let id = self.objects.insert(ObjectRecord {
                object,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::World { parent },
                name,
                tags: self.objects[m].tags.clone(),
            });
            created.push(id);
        }

        // Copy each live def-owned sketch into an independent WORLD sketch,
        // baking the same pose (guaranteed a similarity by the guard above,
        // and invertible/non-reflected by the checks above, so this cannot
        // fail). NOT registered in `def_sketches` — it is genuinely
        // world-owned from here, exactly like a baked member is genuinely
        // world-owned; the definition's own sketch is left exactly as it
        // was for any sibling instance.
        let mut created_sketches: Vec<SketchId> = Vec::with_capacity(def_owned_sketches.len());
        for sid in def_owned_sketches {
            let mut clone = self.sketches[sid].clone();
            clone
                .apply_transform(&pose)
                .map_err(DocumentError::Transform)?;
            created_sketches.push(self.sketches.insert(clone));
        }

        self.instances[instance].hidden = true;
        if let Some(pg) = parent {
            let nodes: Vec<NodeId> = created.iter().map(|&o| NodeId::Object(o)).collect();
            self.splice_out_parent(pg, NodeId::Instance(instance), &nodes);
        }
        self.undo.push(DocAction::Exploded {
            instance,
            created: created.clone(),
            created_sketches: created_sketches.clone(),
        });
        self.redo.clear();
        self.debug_validate();

        let mut change = DocChange {
            objects_touched: created.clone(),
            sketches_touched: created_sketches,
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
    /// The new definition is a new component, so it gets its own name:
    /// - the instance's **own name, if set, is promoted** to the definition
    ///   name, and the instance name is cleared (the instance now reads as
    ///   the new definition, not as a renamed placement of the old one);
    /// - otherwise the old definition's name with `" Copy"` appended — and if
    ///   a live definition already holds that name, `" Copy 2"`, `" Copy 3"`,
    ///   … (the lowest free number);
    /// - a nameless source definition (possible only via import — native
    ///   definitions are always named) yields a nameless copy.
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
        let members: Vec<ObjectId> = match self.components.get(prev_def) {
            Some(c) if !c.hidden => c
                .members
                .iter()
                .copied()
                .filter(|&m| self.objects.get(m).is_some_and(|r| !r.hidden))
                .collect(),
            _ => return Err(DocumentError::UnknownComponent),
        };

        // Name the new definition per the doc comment: promote the instance
        // name if set, else derive a "<def> Copy" name from the source def.
        let prev_instance_name = self.instances[instance].name.clone();
        let new_name = match (
            &prev_instance_name,
            self.components[prev_def].name.as_deref(),
        ) {
            (Some(n), _) => Some(n.clone()),
            (None, Some(d)) => Some(self.copy_component_name(d)),
            (None, None) => None,
        };
        if prev_instance_name.is_some() {
            self.instances[instance].name = None;
        }

        // Deep-copy each member into a fresh private definition (def-local
        // geometry, fresh per-object history).
        let new_def = self.components.insert(ComponentDef {
            members: Vec::new(),
            hidden: false,
            name: new_name,
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

        // Deep-copy the source definition's LIVE def-owned sketches too
        // (component-edit-parity.md phase K1 follow-up — this used to be a
        // documented scope boundary; the wasm/recording surfaces that make
        // it reachable now ship, so it is closed for real): a private copy
        // must own its own drafting surfaces, not share `prev_def`'s, or
        // drawing further into one instance's copy would silently edit the
        // OTHER instances' shared definition. Def-local coordinates copy
        // verbatim — no pose is involved (unlike `explode_instance`, this
        // does not change coordinate frames). A sketch already consumed
        // into a solid (hidden — Model D's larval-form husk) is skipped:
        // nothing in the new definition can ever reference it (each member
        // clone gets a FRESH empty `History`), and cloning it would leave a
        // phantom hidden entry that `MadeUnique`'s own undo/redo bookkeeping
        // (which hides/unhides by CURRENT visibility, not by recorded ids)
        // would then mishandle on a later undo/redo pair.
        let source_sketches: Vec<SketchId> = self
            .def_sketches
            .iter()
            .filter(|&(sid, &c)| c == prev_def && !self.hidden_sketches.contains(sid))
            .map(|(&sid, _)| sid)
            .collect();
        let mut cloned_sketches: Vec<SketchId> = Vec::with_capacity(source_sketches.len());
        for sid in source_sketches {
            let clone = self.sketches[sid].clone();
            let new_sid = self.sketches.insert(clone);
            self.def_sketches.insert(new_sid, new_def);
            cloned_sketches.push(new_sid);
        }

        self.instances[instance].def = new_def;

        self.undo.push(DocAction::MadeUnique {
            instance,
            prev_def,
            new_def,
            prev_instance_name,
        });
        self.redo.clear();
        self.debug_validate();

        Ok((
            new_def,
            DocChange {
                instances_touched: vec![instance],
                components_touched: vec![prev_def, new_def],
                // The clones need to be registered with inference too — same
                // as `explode_instance`'s `created_sketches` reporting its
                // baked world sketches — or the private copy's drafting
                // surface stays snap-blind until an unrelated op happens to
                // touch it.
                sketches_touched: cloned_sketches,
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

    /// Deep-clone each of `nodes` `count` times along `step` — the kernel half
    /// of the Move tool's **array copy** (the "×N" / "/N" refinement after a
    /// Move+copy commit). Copy `k` (1-based) of each source is placed at `step`
    /// composed `k` times, so a pure-translation step yields evenly spaced
    /// copies continuing along the same vector; the caller expresses an
    /// internal ("/N") array by passing `step = full_offset / N`. Every clone
    /// follows [`Document::duplicate_node`]'s per-kind placement rules — an
    /// Object/Group copy bakes fresh independent geometry, an Instance copy is
    /// another instance of the **same definition** at the composed pose — and
    /// lands under the **same parent** as its source.
    ///
    /// The whole array is recorded as one [`DocAction::DuplicatedArray`]: a
    /// single undo hides every copy, a single redo restores them all.
    ///
    /// Returns the clone roots in creation order (every source's copy 1, then
    /// every source's copy 2, …; each block in `nodes` order) plus what
    /// changed.
    ///
    /// Each listed node is cloned independently, exactly like calling
    /// [`Document::duplicate_node`] per node — listing a node alongside a
    /// group that contains it clones that geometry twice. Listing the same
    /// node twice is rejected instead (the UI's selection is a set).
    ///
    /// # Errors
    /// - [`DocumentError::EmptySelection`] — `nodes` is empty.
    /// - [`DocumentError::DuplicateMember`] — the same node listed twice.
    /// - [`DocumentError::UnknownObject`] / `UnknownGroup` / `UnknownInstance`
    ///   — a source node is stale or hidden.
    /// - [`DocumentError::Transform`] — `step` is singular, or a composed
    ///   placement reflects an Object/Group leaf (baking would invert winding,
    ///   as in [`Document::duplicate_node`]).
    ///
    /// On `Err` the document is untouched (partial clones are rolled back).
    pub fn duplicate_nodes_array(
        &mut self,
        nodes: &[NodeId],
        step: &Transform,
        count: NonZeroU32,
    ) -> Result<(Vec<NodeId>, DocChange), DocumentError> {
        info!(
            target: "kernel::op",
            op = "duplicate_nodes_array",
            nodes = nodes.len(),
            count = count.get(),
        );
        if nodes.is_empty() {
            return Err(DocumentError::EmptySelection);
        }
        // Selections are small; a linear repeat scan stays deterministic and
        // avoids requiring Ord on NodeId.
        for (i, &n) in nodes.iter().enumerate() {
            if nodes[..i].contains(&n) {
                return Err(DocumentError::DuplicateMember);
            }
        }
        for &n in nodes {
            if !self.node_is_live(n) {
                return Err(match n {
                    NodeId::Object(_) => DocumentError::UnknownObject,
                    NodeId::Group(_) => DocumentError::UnknownGroup,
                    NodeId::Instance(_) => DocumentError::UnknownInstance,
                });
            }
        }
        // Validate invertibility up front; a reflecting placement is
        // re-rejected by `apply_transform` per object leaf during the clone.
        step.inverse().map_err(DocumentError::Transform)?;

        let mut created = CreatedClone::default();
        let mut roots: Vec<(NodeId, Option<GroupId>)> = Vec::new();
        let mut placement = *step;
        for k in 0..count.get() {
            if k > 0 {
                placement = placement.then(step);
            }
            for &node in nodes {
                let parent = self.node_parent(node);
                match self.clone_subtree(node, parent, &placement, &mut created) {
                    Ok(root) => roots.push((root, parent)),
                    Err(e) => {
                        // Roll back every record inserted so far so the
                        // document is untouched on error (strong guarantee).
                        // Nothing outside `created` has been mutated yet —
                        // roots are appended to parents only after the loop.
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
                }
            }
        }
        // Append each clone root to its parent's member list in creation order
        // (top-level nodes need no list — they derive from the slotmap,
        // hidden-filtered).
        for &(root, parent) in &roots {
            if let Some(pg) = parent {
                self.groups[pg].members.push(root);
            }
        }

        self.undo.push(DocAction::DuplicatedArray {
            roots: roots.clone(),
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
        // Each distinct parent group is touched once (its member list grew).
        for &(_, parent) in &roots {
            if let Some(pg) = parent
                && !change.groups_touched.contains(&pg)
            {
                change.groups_touched.push(pg);
            }
        }
        Ok((roots.into_iter().map(|(root, _)| root).collect(), change))
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

    /// A monotonic token identifying the current undo-stack state: it changes
    /// on every recorded action, every undo, and every redo (including a
    /// refused undo/redo, which pushes its action back), and on nothing else —
    /// non-undoable view-state edits ([`Document::set_tag_hidden`],
    /// [`Document::set_node_user_hidden`]) leave it untouched.
    ///
    /// An unchanged generation proves the action a caller committed is still
    /// the top of the undo stack, so an [`Document::undo`] will retract
    /// exactly it — the guard the Move tool's array-copy refinement needs. A
    /// content hash ([`Document::state_hash`]) cannot stand in: a net-zero
    /// pair of undoable edits (a tag added, then removed) restores the hash
    /// while burying the caller's action two entries deep, and a view-state
    /// toggle changes the hash without touching the stack at all.
    pub fn history_generation(&self) -> u64 {
        self.undo.pushes + self.redo.pushes
    }

    /// The kernel op the next [`Document::undo`] would reverse, when the
    /// pending document action is a per-object op — on a world object
    /// ([`DocAction::ObjectOp`]) or a definition member
    /// ([`DocAction::DefObjectOp`]) — `None` otherwise or when there is
    /// nothing to undo. Mirrors [`History::peek_undo`].
    pub fn peek_undo_object_op(&self) -> Option<&KernelOp> {
        match self.undo.last()? {
            DocAction::ObjectOp { object } | DocAction::DefObjectOp { object, .. } => {
                self.objects.get(*object)?.history.peek_undo()
            }
            _ => None,
        }
    }

    /// The kernel op the next [`Document::redo`] would replay, when the
    /// pending document action is a per-object op (world or definition
    /// member, as in [`Document::peek_undo_object_op`]). Mirrors
    /// [`History::peek_redo`].
    pub fn peek_redo_object_op(&self) -> Option<&KernelOp> {
        match self.redo.last()? {
            DocAction::ObjectOp { object } | DocAction::DefObjectOp { object, .. } => {
                self.objects.get(*object)?.history.peek_redo()
            }
            _ => None,
        }
    }

    /// The coarse shape of the pending undo/redo [`DocAction`], for guard
    /// harnesses that need to narrow a tolerated refusal to specific
    /// action kinds without matching the (private) `DocAction` type itself
    /// — mirrors [`Document::peek_undo_object_op`]'s "peek without popping"
    /// shape, one level up (the action's own kind rather than its inner
    /// per-object `KernelOp`). `None` when there is nothing pending.
    pub fn peek_undo_action_kind(&self) -> Option<PendingActionKind> {
        self.undo.last().map(DocAction::kind)
    }

    /// [`Document::peek_undo_action_kind`]'s redo-side mirror.
    pub fn peek_redo_action_kind(&self) -> Option<PendingActionKind> {
        self.redo.last().map(DocAction::kind)
    }

    /// Undo one extrusion: hide the solid and RE-INSERT the scaffolding it
    /// deleted, merging with the sketch's current contents
    /// ([`Sketch::restore_edges`]) — edits made after the extrusion
    /// survive. On a re-insertion conflict the action returns to the undo
    /// stack and the document is untouched
    /// ([`SketchError::RestoreConflicts`]).
    ///
    /// Returns the [`DocChange`] alongside the exact [`DocAction`] the
    /// caller should push onto the redo stack — callers push it themselves
    /// rather than this function pushing directly, so
    /// [`Document::undo_place_text_compound`] can bundle several such
    /// redo-actions into one [`DocAction::PlaceTextCompound`] instead of
    /// each landing as its own entry. The ordinary top-level
    /// [`Document::undo`] just pushes it straight through, unchanged from
    /// before this split.
    fn undo_created_object(
        &mut self,
        action: DocAction,
    ) -> Result<(DocChange, DocAction), DocumentError> {
        let DocAction::CreatedObject {
            id,
            sketch,
            removed,
            emptied,
            merged_base,
        } = action
        else {
            unreachable!("dispatched on CreatedObject");
        };
        if let Err(e) = self
            .sketches
            .get_mut(sketch)
            .expect("sketch slots are never removed")
            .restore_edges(&removed)
        {
            // Nothing changed; the step stays undoable after the caller
            // clears the conflicting geometry.
            self.undo.push(DocAction::CreatedObject {
                id,
                sketch,
                removed,
                emptied,
                merged_base,
            });
            return Err(DocumentError::Sketch(e));
        }
        if emptied {
            self.hidden_sketches.remove(&sketch);
        }
        if let Some(rec) = self.objects.get_mut(id) {
            rec.hidden = true;
        }
        let mut groups_touched = Vec::new();
        // A group-born sweep (design §2f) must leave the member list too —
        // the tree invariant forbids hidden members; the owner field keeps
        // the group so redo can relink.
        if let Some(ObjectOwner::World { parent: Some(gid) }) =
            self.objects.get(id).map(|r| r.owner)
            && let Some(grec) = self.groups.get_mut(gid)
        {
            grec.members.retain(|m| *m != NodeId::Object(id));
            groups_touched.push(gid);
        }
        let mut objects_touched = vec![id];
        if let Some(base) = merged_base {
            // The merged sweep consumed the path's solid; undo restores it.
            self.objects[base].hidden = false;
            objects_touched.push(base);
        }
        // A definition-owned birth (component-edit-parity.md phase K1):
        // ownership never changes across undo/redo (only `hidden` does — see
        // `commit_region_object_owned`'s doc comment), so hiding the member
        // back out is exactly the shared-geometry edit `DeletedDefMember`'s
        // own undo already reports — every instance of the definition needs
        // to re-resolve, not just the one drawn through.
        let (components_touched, instances_touched) = match self.objects.get(id).map(|r| r.owner) {
            Some(ObjectOwner::Definition(component)) => {
                (vec![component], self.instances_of(component))
            }
            _ => (Vec::new(), Vec::new()),
        };
        let redo_action = DocAction::CreatedObject {
            id,
            sketch,
            removed,
            emptied,
            merged_base,
        };
        self.debug_validate();
        Ok((
            DocChange {
                objects_touched,
                sketches_touched: vec![sketch],
                groups_touched,
                instances_touched,
                components_touched,
                guides_touched: Vec::new(),
            },
            redo_action,
        ))
    }

    /// Redo one extrusion: re-delete the scaffolding BY GEOMETRY
    /// ([`Sketch::edge_at_positions`] over the stored rows), re-remove an
    /// emptied sketch, and show the solid again. Geometry, not ids: a
    /// gesture undo/redo interleaved on the same sketch restores snapshots
    /// carrying the outline's original edge ids, so ids recorded at the
    /// last undo can be stale while the row positions match exactly.
    /// Cannot conflict — nothing else intervenes between an undo and its
    /// redo (any new op clears the redo stack).
    ///
    /// Returns the [`DocChange`] alongside the exact [`DocAction`] the
    /// caller should push onto the undo stack — see
    /// [`Document::undo_created_object`]'s matching note.
    fn redo_created_object(
        &mut self,
        action: DocAction,
    ) -> Result<(DocChange, DocAction), DocumentError> {
        let DocAction::CreatedObject {
            id,
            sketch,
            removed,
            emptied: _,
            merged_base,
        } = action
        else {
            unreachable!("dispatched on CreatedObject");
        };
        let sk = self
            .sketches
            .get_mut(sketch)
            .expect("sketch slots are never removed");
        let scaffolding: BTreeSet<SketchEdgeId> = removed
            .iter()
            .filter_map(|&(a, b, _)| sk.edge_at_positions(a, b))
            .collect();
        sk.remove_edges(&scaffolding);
        let emptied = sk.edges().is_empty();
        if emptied {
            self.hidden_sketches.insert(sketch);
        }
        if let Some(rec) = self.objects.get_mut(id) {
            rec.hidden = false;
        }
        let mut groups_touched = Vec::new();
        // Relink a group-born sweep's membership (design §2f).
        if let Some(ObjectOwner::World { parent: Some(gid) }) =
            self.objects.get(id).map(|r| r.owner)
            && let Some(grec) = self.groups.get_mut(gid)
            && !grec.members.contains(&NodeId::Object(id))
        {
            grec.members.push(NodeId::Object(id));
            groups_touched.push(gid);
        }
        let mut objects_touched = vec![id];
        if let Some(base) = merged_base {
            // Redo consumes the merged base again.
            self.objects[base].hidden = true;
            objects_touched.push(base);
        }
        // A definition-owned birth: see `undo_created_object`'s matching
        // comment — redo re-shows the member, so every instance needs to
        // re-resolve it too, exactly like `DeletedDefMember`'s redo already
        // reports.
        let (components_touched, instances_touched) = match self.objects.get(id).map(|r| r.owner) {
            Some(ObjectOwner::Definition(component)) => {
                (vec![component], self.instances_of(component))
            }
            _ => (Vec::new(), Vec::new()),
        };
        let undo_action = DocAction::CreatedObject {
            id,
            sketch,
            removed,
            emptied,
            merged_base,
        };
        self.debug_validate();
        Ok((
            DocChange {
                objects_touched,
                sketches_touched: vec![sketch],
                groups_touched,
                instances_touched,
                components_touched,
                guides_touched: Vec::new(),
            },
            undo_action,
        ))
    }

    /// Undo one out-of-plane island detach: re-insert the island's outline
    /// into the source sketch (merging with its current contents, exactly
    /// like extrusion undo — [`Sketch::restore_edges`]) and hide the
    /// detached sketch. On a re-insertion conflict the action returns to
    /// the undo stack and the document is untouched
    /// ([`SketchError::RestoreConflicts`]).
    fn undo_detached_island(&mut self, action: DocAction) -> Result<DocChange, DocumentError> {
        let DocAction::DetachedSketchIsland {
            source,
            detached,
            removed,
        } = action
        else {
            unreachable!("dispatched on DetachedSketchIsland");
        };
        if let Err(e) = self
            .sketches
            .get_mut(source)
            .expect("sketch slots are never removed")
            .restore_edges(&removed)
        {
            // Nothing changed; the step stays undoable after the caller
            // clears the conflicting geometry.
            self.undo.push(DocAction::DetachedSketchIsland {
                source,
                detached,
                removed,
            });
            return Err(DocumentError::Sketch(e));
        }
        self.hidden_sketches.insert(detached);
        self.redo.push(DocAction::DetachedSketchIsland {
            source,
            detached,
            removed,
        });
        self.debug_validate();
        let (components_touched, instances_touched) = self.def_sketch_owner_change(source);
        Ok(DocChange {
            objects_touched: Vec::new(),
            sketches_touched: vec![source, detached],
            groups_touched: Vec::new(),
            instances_touched,
            components_touched,
            guides_touched: Vec::new(),
        })
    }

    /// Redo one out-of-plane island detach: re-remove the outline from the
    /// source BY GEOMETRY ([`Sketch::edge_at_positions`] — undo restored it
    /// with fresh edge ids, so ids would be stale where the row positions
    /// are exact) and unhide the detached sketch, whose contents survived
    /// hiding bit-exactly. Cannot conflict — nothing intervenes between an
    /// undo and its redo (any new op clears the redo stack).
    fn redo_detached_island(&mut self, action: DocAction) -> Result<DocChange, DocumentError> {
        let DocAction::DetachedSketchIsland {
            source,
            detached,
            removed,
        } = action
        else {
            unreachable!("dispatched on DetachedSketchIsland");
        };
        let sk = self
            .sketches
            .get_mut(source)
            .expect("sketch slots are never removed");
        let scaffolding: std::collections::BTreeSet<SketchEdgeId> = removed
            .iter()
            .filter_map(|&(a, b, _)| sk.edge_at_positions(a, b))
            .collect();
        sk.remove_edges(&scaffolding);
        self.hidden_sketches.remove(&detached);
        self.undo.push(DocAction::DetachedSketchIsland {
            source,
            detached,
            removed,
        });
        self.debug_validate();
        let (components_touched, instances_touched) = self.def_sketch_owner_change(source);
        Ok(DocChange {
            objects_touched: Vec::new(),
            sketches_touched: vec![source, detached],
            groups_touched: Vec::new(),
            instances_touched,
            components_touched,
            guides_touched: Vec::new(),
        })
    }

    /// Feasibility pre-check for [`Document::undo_place_text_compound`]:
    /// whether every bundled `CreatedObject` AND
    /// [`DocAction::ConsumedScaffolding`] reversal's
    /// [`Sketch::restore_edges`] call would succeed, WITHOUT mutating
    /// `self` — the only inner reversal that can fail. (`MadeComponent`'s
    /// reversal is pure field mutation; `SketchGesture`'s is a snapshot
    /// restore; neither can err.) Simulates each
    /// `CreatedObject`/`ConsumedScaffolding` entry against a scratch clone
    /// of the sketch it names, replayed in the SAME reverse order
    /// [`Document::undo_place_text_compound`] applies them, so a shared
    /// sketch's cumulative state matches exactly — mirroring the
    /// validate-then-apply
    /// shape `restore_edges` itself already uses internally.
    ///
    /// Historically reachable through a counter glyph (an 'O'): before
    /// [`DocAction::ConsumedScaffolding`] existed, `place_text` left the
    /// counter's own contour standing as live scaffolding, so the sketch
    /// stayed live and reachable for as long as the placement wasn't
    /// undone, and anything drawn against it before that undo could leave
    /// a `restore_edges` row unable to re-insert faithfully. `place_text`
    /// now always retires (hides) its sketch by the time it returns — see
    /// `Document::place_text`'s cleanup step — closing that hole at the
    /// source. This check remains as defense in depth: it validates every
    /// bundled reversal (`CreatedObject` AND `ConsumedScaffolding` alike)
    /// against scratch sketch clones before `undo_place_text_compound`
    /// mutates anything, so a future producer or an edge case this
    /// reasoning missed still fails typed rather than wedging the undo
    /// stack.
    fn compound_reversal_feasible(&self, actions: &[DocAction]) -> Result<(), DocumentError> {
        let mut scratch: Vec<(SketchId, Sketch)> = Vec::new();
        for inner in actions.iter().rev() {
            if let DocAction::CreatedObject {
                sketch, removed, ..
            }
            | DocAction::ConsumedScaffolding { sketch, removed } = inner
            {
                let idx = match scratch.iter().position(|(id, _)| id == sketch) {
                    Some(i) => i,
                    None => {
                        let sk = self
                            .sketches
                            .get(*sketch)
                            .expect("sketch slots are never removed")
                            .clone();
                        scratch.push((*sketch, sk));
                        scratch.len() - 1
                    }
                };
                scratch[idx]
                    .1
                    .restore_edges(removed)
                    .map_err(DocumentError::Sketch)?;
            }
        }
        Ok(())
    }

    /// Undo a [`DocAction::PlaceTextCompound`] (3D Text placement,
    /// [`Document::place_text`]): reverses every bundled action in REVERSE
    /// order — last extrusion first, the glyph-injection gesture last —
    /// exactly reproducing what undoing each step individually would have
    /// done, then re-bundles their own redo-actions (in original order)
    /// into one `PlaceTextCompound` for the redo stack, so one redo restores
    /// the whole placement.
    ///
    /// Scoped to the fixed shape `place_text` builds — zero or more
    /// [`DocAction::CreatedObject`], zero or one
    /// [`DocAction::ConsumedScaffolding`], one [`DocAction::MadeComponent`],
    /// one [`DocAction::SketchGesture`] — rather than the fully general
    /// [`DocAction::Compound`] nested undo mechanism; any other inner
    /// variant is a kernel bug (`place_text` is `PlaceTextCompound`'s only
    /// producer).
    ///
    /// All-or-nothing (DEVELOPMENT.md rule 9): a `CreatedObject` entry's
    /// re-insertion can fail (`SketchError::RestoreConflicts` —
    /// [`Document::compound_reversal_feasible`]'s doc comment gives the
    /// exact repro), and letting that surface mid-unwind would leave the
    /// already-reversed `MadeComponent` step applied (an orphaned solid,
    /// owner World with no live instance) while the rest of the bundle and
    /// the `PlaceTextCompound` action itself are lost — a wedged undo
    /// stack. So [`Document::compound_reversal_feasible`] validates every
    /// bundled reversal against scratch sketch clones BEFORE this function
    /// mutates anything; on refusal the `PlaceTextCompound` goes back onto
    /// the undo stack untouched and the document is byte-identical to
    /// before the call. Once the pre-check passes, every `CreatedObject`
    /// reversal below is guaranteed to succeed (same rows, same order, same
    /// starting sketch state), so the `?` on `undo_created_object` never
    /// actually fires.
    fn undo_place_text_compound(&mut self, action: DocAction) -> Result<DocChange, DocumentError> {
        let DocAction::PlaceTextCompound(actions) = action else {
            unreachable!("dispatched on PlaceTextCompound");
        };
        if let Err(e) = self.compound_reversal_feasible(&actions) {
            self.undo.push(DocAction::PlaceTextCompound(actions));
            return Err(e);
        }
        let mut redo_actions = Vec::with_capacity(actions.len());
        let mut objects_touched = Vec::new();
        let mut sketches_touched = Vec::new();
        let mut groups_touched = Vec::new();
        let mut instances_touched = Vec::new();
        let mut components_touched = Vec::new();
        for inner in actions.into_iter().rev() {
            let (c, redo_action) = match &inner {
                DocAction::CreatedObject { .. } => self.undo_created_object(inner)?,
                DocAction::ConsumedScaffolding { sketch, removed } => {
                    let (sketch, removed) = (*sketch, removed.clone());
                    // `compound_reversal_feasible` already validated this
                    // exact restore against a scratch clone of the same
                    // starting state, so this cannot actually fail — the
                    // `?` mirrors `undo_created_object`'s own guarantee.
                    self.sketches
                        .get_mut(sketch)
                        .expect("sketch slots are never removed")
                        .restore_edges(&removed)
                        .map_err(DocumentError::Sketch)?;
                    self.hidden_sketches.remove(&sketch);
                    let c = DocChange {
                        sketches_touched: vec![sketch],
                        ..Default::default()
                    };
                    (c, inner.clone())
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
                    let leaves: Vec<ObjectId> =
                        member_prior_parents.iter().map(|&(o, _)| o).collect();
                    let c = made_component_change(
                        component,
                        instance,
                        parent,
                        &leaves,
                        consumed_groups,
                    );
                    (c, inner.clone())
                }
                DocAction::SketchGesture {
                    sketch,
                    before,
                    created,
                    ..
                } => {
                    let (sketch, created) = (*sketch, *created);
                    if let Some(s) = self.sketches.get_mut(sketch) {
                        *s = (**before).clone();
                    }
                    if created {
                        self.hidden_sketches.insert(sketch);
                    }
                    let c = DocChange {
                        sketches_touched: vec![sketch],
                        ..Default::default()
                    };
                    (c, inner.clone())
                }
                _ => unreachable!(
                    "PlaceTextCompound only ever bundles SketchGesture/CreatedObject/ConsumedScaffolding/MadeComponent — place_text is its only producer"
                ),
            };
            objects_touched.extend(c.objects_touched);
            sketches_touched.extend(c.sketches_touched);
            groups_touched.extend(c.groups_touched);
            instances_touched.extend(c.instances_touched);
            components_touched.extend(c.components_touched);
            redo_actions.push(redo_action);
        }
        redo_actions.reverse();
        self.redo.push(DocAction::PlaceTextCompound(redo_actions));
        self.debug_validate();
        Ok(DocChange {
            objects_touched,
            sketches_touched,
            groups_touched,
            instances_touched,
            components_touched,
            guides_touched: Vec::new(),
        })
    }

    /// Redo a [`DocAction::PlaceTextCompound`]: replays every bundled action
    /// in FORWARD (original) order — the glyph-injection gesture first,
    /// each extrusion, then the component fold — mirroring
    /// [`Document::undo_place_text_compound`]. See its doc comment for the
    /// scoping and partial-failure notes (both apply here symmetrically);
    /// redo cannot actually conflict in practice (nothing intervenes
    /// between an undo and its matching redo).
    fn redo_place_text_compound(&mut self, action: DocAction) -> Result<DocChange, DocumentError> {
        let DocAction::PlaceTextCompound(actions) = action else {
            unreachable!("dispatched on PlaceTextCompound");
        };
        let mut undo_actions = Vec::with_capacity(actions.len());
        let mut objects_touched = Vec::new();
        let mut sketches_touched = Vec::new();
        let mut groups_touched = Vec::new();
        let mut instances_touched = Vec::new();
        let mut components_touched = Vec::new();
        for inner in actions.into_iter() {
            let (c, undo_action) = match &inner {
                DocAction::CreatedObject { .. } => self.redo_created_object(inner)?,
                DocAction::ConsumedScaffolding { sketch, removed } => {
                    let (sketch, removed) = (*sketch, removed.clone());
                    // Mirrors `redo_created_object`: re-delete BY GEOMETRY
                    // (`Sketch::edge_at_positions`), not by the stored ids
                    // — an interleaved gesture undo/redo on the same
                    // sketch restores snapshots carrying fresh edge ids.
                    let sk = self
                        .sketches
                        .get_mut(sketch)
                        .expect("sketch slots are never removed");
                    let scaffolding: BTreeSet<SketchEdgeId> = removed
                        .iter()
                        .filter_map(|&(a, b, _)| sk.edge_at_positions(a, b))
                        .collect();
                    sk.remove_edges(&scaffolding);
                    self.hidden_sketches.insert(sketch);
                    let c = DocChange {
                        sketches_touched: vec![sketch],
                        ..Default::default()
                    };
                    (c, inner.clone())
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
                    let leaves: Vec<ObjectId> =
                        member_prior_parents.iter().map(|&(o, _)| o).collect();
                    let c = made_component_change(
                        component,
                        instance,
                        parent,
                        &leaves,
                        consumed_groups,
                    );
                    (c, inner.clone())
                }
                DocAction::SketchGesture {
                    sketch,
                    after,
                    created,
                    ..
                } => {
                    let (sketch, created) = (*sketch, *created);
                    if created {
                        self.hidden_sketches.remove(&sketch);
                    }
                    if let Some(s) = self.sketches.get_mut(sketch) {
                        *s = (**after).clone();
                    }
                    let c = DocChange {
                        sketches_touched: vec![sketch],
                        ..Default::default()
                    };
                    (c, inner.clone())
                }
                _ => unreachable!(
                    "PlaceTextCompound only ever bundles SketchGesture/CreatedObject/ConsumedScaffolding/MadeComponent — place_text is its only producer"
                ),
            };
            objects_touched.extend(c.objects_touched);
            sketches_touched.extend(c.sketches_touched);
            groups_touched.extend(c.groups_touched);
            instances_touched.extend(c.instances_touched);
            components_touched.extend(c.components_touched);
            undo_actions.push(undo_action);
        }
        self.undo.push(DocAction::PlaceTextCompound(undo_actions));
        self.debug_validate();
        Ok(DocChange {
            objects_touched,
            sketches_touched,
            groups_touched,
            instances_touched,
            components_touched,
            guides_touched: Vec::new(),
        })
    }

    /// Reverses the most recent document action (LIFO across creations and
    /// per-Object ops alike) and returns what it touched.
    pub fn undo(&mut self) -> Result<DocChange, DocumentError> {
        let action = self.undo.pop().ok_or(DocumentError::NothingToUndo)?;
        if let DocAction::Compound { actions } = &action {
            let checkpoint = self.clone();
            let redo_start = self.redo.actions.len();
            for child in actions {
                self.undo.push(child.clone());
            }
            let mut change = DocChange::default();
            for _ in 0..actions.len() {
                match self.undo() {
                    Ok(child_change) => merge_doc_change(&mut change, child_change),
                    Err(error) => {
                        *self = checkpoint;
                        return Err(error);
                    }
                }
            }
            self.redo.actions.truncate(redo_start);
            self.redo.push(action);
            self.debug_validate();
            return Ok(change);
        }
        // Extrusion undo can FAIL (re-insertion conflicts) and refreshes
        // the action's scaffolding ids on success, so it manages its own
        // stacks in a dedicated helper rather than the shared match below.
        if matches!(action, DocAction::CreatedObject { .. }) {
            let (change, redo_action) = self.undo_created_object(action)?;
            self.redo.push(redo_action);
            return Ok(change);
        }
        // Island-detach undo can fail the same way (it restores the same
        // row shape); same dedicated-helper treatment.
        if matches!(action, DocAction::DetachedSketchIsland { .. }) {
            return self.undo_detached_island(action);
        }
        // A 3D Text placement (docs/design/3d-text.md) bundles several
        // ordinary steps into one entry; its own dedicated helper reverses
        // each in turn and re-bundles their own redo-actions into one
        // `PlaceTextCompound` for the redo stack.
        if matches!(action, DocAction::PlaceTextCompound(_)) {
            return self.undo_place_text_compound(action);
        }
        let change = match &action {
            DocAction::Compound { .. } => unreachable!("Compound is handled before the match"),
            // Dispatched to their dedicated helpers before this match.
            DocAction::CreatedObject { .. } => {
                unreachable!("CreatedObject is handled before the match")
            }
            DocAction::DetachedSketchIsland { .. } => {
                unreachable!("DetachedSketchIsland is handled before the match")
            }
            DocAction::PlaceTextCompound(_) => {
                unreachable!("PlaceTextCompound is handled before the match")
            }
            DocAction::ConsumedScaffolding { .. } => unreachable!(
                "ConsumedScaffolding only ever appears inside a PlaceTextCompound — place_text is its only producer"
            ),
            &DocAction::CopiedSketchIslands { copy } => {
                // Undo a copy: hide the new sketch (its contents survive
                // hiding bit-exactly). The source was never touched.
                self.hidden_sketches.insert(copy);
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![copy],
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
            &DocAction::FollowMeFace {
                result,
                merged_base,
            } => {
                // Undo a face-profile sweep: hide the result; a merged
                // base comes back like a boolean operand. A definition-owned
                // result (component-edit-parity.md phase K2:
                // `follow_me_face_in_instance`) needs every instance to
                // re-resolve, exactly like `undo_created_object`'s matching
                // comment.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = true;
                }
                let mut touched = vec![result];
                if let Some(base) = merged_base {
                    self.objects[base].hidden = false;
                    touched.push(base);
                }
                let (components_touched, instances_touched) = self.def_owner_change(result);
                DocChange {
                    objects_touched: touched,
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::Boolean { result, a, b } => {
                // Undo a combine: hide the result, bring the operands back.
                // A definition-owned result (component-edit-parity.md phase
                // K2: `boolean_in_component`) needs every instance to
                // re-resolve.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = true;
                }
                self.objects[a].hidden = false;
                self.objects[b].hidden = false;
                let (components_touched, instances_touched) = self.def_owner_change(result);
                DocChange {
                    objects_touched: vec![result, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                }
            }
            DocAction::BooleanNodes {
                hidden_operands,
                result_objects,
                result_group,
                ..
            } => {
                // Undo a node combine: hide the result (pieces + container
                // group), unhide exactly the consumed operand subtrees.
                for &o in result_objects {
                    self.objects[o].hidden = true;
                }
                if let Some(g) = *result_group {
                    self.groups[g].hidden = true;
                }
                for &n in hidden_operands {
                    match n {
                        NodeId::Object(id) => self.objects[id].hidden = false,
                        NodeId::Group(id) => self.groups[id].hidden = false,
                        NodeId::Instance(id) => self.instances[id].hidden = false,
                    }
                }
                boolean_nodes_change(hidden_operands, result_objects, *result_group)
            }
            &DocAction::Sliced { source, a, b } => {
                // Undo a slice: hide both pieces, bring the source back. A
                // definition-owned source (component-edit-parity.md phase
                // K2: `slice_def_member`) needs every instance to
                // re-resolve.
                if let Some(rec) = self.objects.get_mut(a) {
                    rec.hidden = true;
                }
                if let Some(rec) = self.objects.get_mut(b) {
                    rec.hidden = true;
                }
                self.objects[source].hidden = false;
                let (components_touched, instances_touched) = self.def_owner_change(source);
                DocChange {
                    objects_touched: vec![source, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                let (components_touched, instances_touched) = self.def_owner_change(source);
                DocChange {
                    objects_touched,
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                // A definition-owned target (component-edit-parity.md phase
                // K2: `transform_def_member`) needs every instance of its
                // definition to re-resolve; `transform_object`/
                // `transform_group` only ever bake world objects, for which
                // this is always empty.
                let mut components_touched = Vec::new();
                let mut instances_touched = Vec::new();
                for &obj in objects {
                    let (c, i) = self.def_owner_change(obj);
                    for cc in c {
                        if !components_touched.contains(&cc) {
                            components_touched.push(cc);
                        }
                    }
                    for ii in i {
                        if !instances_touched.contains(&ii) {
                            instances_touched.push(ii);
                        }
                    }
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::TransformSketchIsland {
                sketch,
                island,
                forward,
                inverse,
                anchor_before,
                anchor_after,
            } => {
                // Undo an island transform by baking its exact inverse.
                // The stored id can be STALE — a later sweep/extrusion
                // consumed the island and its own undo restored the edges
                // under fresh ids — so re-resolve by the recorded anchor
                // when needed; a typed refusal (action re-pushed, document
                // untouched) if the geometry itself is gone.
                let resolved = if self.sketches[sketch].islands().contains_key(island) {
                    Some(island)
                } else {
                    self.sketches[sketch].island_at_anchor(anchor_after)
                };
                let action = DocAction::TransformSketchIsland {
                    sketch,
                    island: resolved.unwrap_or(island),
                    forward,
                    inverse,
                    anchor_before,
                    anchor_after,
                };
                let Some(resolved) = resolved else {
                    self.undo.push(action);
                    return Err(DocumentError::Sketch(SketchError::UnknownIsland));
                };
                if let Err(e) = self.sketches[sketch].apply_transform_island(resolved, &inverse) {
                    self.undo.push(action);
                    return Err(DocumentError::Sketch(e));
                }
                self.redo.push(action);
                self.debug_validate();
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                return Ok(DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                });
            }
            &DocAction::TransformSketch {
                sketch, inverse, ..
            } => {
                // Undo a sketch transform by baking its exact inverse.
                self.sketches[sketch]
                    .apply_transform(&inverse)
                    .expect("inverse of a validated transform must re-apply");
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                // snapshot (keys preserved — every prior handle stays
                // valid). A gesture that created the sketch also hides it,
                // so no empty ghost lingers.
                let (sketch, created) = (*sketch, *created);
                if let Some(s) = self.sketches.get_mut(sketch) {
                    *s = (**before).clone();
                }
                if created {
                    self.hidden_sketches.insert(sketch);
                }
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                DocChange {
                    sketches_touched: vec![sketch],
                    components_touched,
                    instances_touched,
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
            &DocAction::DeletedDefMember { component, object } => {
                // Undo delete_def_member: unhide the member. It never left
                // `ComponentDef.members` (see the action's doc comment), so
                // there is nothing else to restore.
                self.objects[object].hidden = false;
                DocChange {
                    objects_touched: vec![object],
                    components_touched: vec![component],
                    instances_touched: self.instances_of(component),
                    ..Default::default()
                }
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
                // Any sketch drawn INTO this definition after the fold
                // (component-edit-parity.md phase K1 — `member_prior_parents`
                // predates that capability, so it only ever lists the
                // original fold's objects) has no world home to return to:
                // hide it too, exactly like a birthed member the fold itself
                // never touches. Recomputed by ownership rather than stored,
                // since nothing else can retarget `def_sketches` for a
                // now-about-to-be-hidden component between this undo and its
                // matching redo (LIFO replay).
                let sketches_touched: Vec<SketchId> = self
                    .def_sketches
                    .iter()
                    .filter(|&(sid, &c)| c == component && !self.hidden_sketches.contains(sid))
                    .map(|(&sid, _)| sid)
                    .collect();
                for &sid in &sketches_touched {
                    self.hidden_sketches.insert(sid);
                }
                self.instances[instance].hidden = true;
                self.components[component].hidden = true;
                let leaves: Vec<ObjectId> = member_prior_parents.iter().map(|&(o, _)| o).collect();
                let mut change =
                    made_component_change(component, instance, parent, &leaves, consumed_groups);
                change.sketches_touched = sketches_touched;
                change
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
            DocAction::DuplicatedArray {
                roots,
                objects,
                groups,
                instances,
            } => {
                // Hide every clone and unlink each root from its parent —
                // [`DocAction::Duplicated`]'s undo, element-wise.
                for &o in objects {
                    self.objects[o].hidden = true;
                }
                for &g in groups {
                    self.groups[g].hidden = true;
                }
                for &i in instances {
                    self.instances[i].hidden = true;
                }
                for &(root, parent) in roots {
                    if let Some(pg) = parent {
                        self.groups[pg].members.retain(|&n| n != root);
                    }
                }
                let mut change = DocChange {
                    objects_touched: objects.clone(),
                    groups_touched: groups.clone(),
                    instances_touched: instances.clone(),
                    ..Default::default()
                };
                for &(_, parent) in roots {
                    if let Some(pg) = parent
                        && !change.groups_touched.contains(&pg)
                    {
                        change.groups_touched.push(pg);
                    }
                }
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
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                DocChange {
                    sketches_touched: vec![sketch],
                    components_touched,
                    instances_touched,
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
                // Mirror the ObjectOp arm: the member object's History keeps
                // its entry when a dispatch is refused, so push the document
                // action back too — a bare `?` here dropped it from BOTH
                // stacks, desyncing the two logs (the next undo panicked),
                // breaking the strong exception guarantee, and mutating the
                // stack top without moving `history_generation` (a pop is
                // only counted through the push that reverses it).
                if let Err(e) = rec.history.undo(&mut rec.object) {
                    self.undo.push(action);
                    return Err(map_history_err(e));
                }
                let instances_touched = self.instances_of(component);
                DocChange {
                    objects_touched: vec![object],
                    components_touched: vec![component],
                    instances_touched,
                    ..Default::default()
                }
            }
            DocAction::Exploded {
                instance,
                created,
                created_sketches,
            } => {
                // Hide the baked world objects and sketches, bring the
                // instance back, and re-splice it into its parent in their
                // place.
                let instance = *instance;
                for &o in created {
                    self.objects[o].hidden = true;
                }
                for &sid in created_sketches {
                    self.hidden_sketches.insert(sid);
                }
                self.instances[instance].hidden = false;
                let parent = self.instances[instance].parent;
                if let Some(pg) = parent {
                    let nodes: Vec<NodeId> = created.iter().map(|&o| NodeId::Object(o)).collect();
                    self.splice_in_parent(pg, &nodes, NodeId::Instance(instance));
                }
                let mut change = DocChange {
                    objects_touched: created.clone(),
                    sketches_touched: created_sketches.clone(),
                    instances_touched: vec![instance],
                    ..Default::default()
                };
                change.groups_touched.extend(parent);
                change
            }
            DocAction::MadeUnique {
                instance,
                prev_def,
                new_def,
                prev_instance_name,
            } => {
                let (instance, prev_def, new_def) = (*instance, *prev_def, *new_def);
                self.instances[instance].def = prev_def;
                // Restore the instance name a promotion cleared (a no-op for
                // an instance that was unnamed at the op).
                self.instances[instance].name = prev_instance_name.clone();
                let new_members = self.components[new_def].members.clone();
                for o in new_members {
                    self.objects[o].hidden = true;
                }
                // Any sketch drawn INTO the private copy (component-edit-
                // parity.md phase K1) has no home once `new_def` is hidden —
                // hide it too, exactly like `MadeComponent`'s undo does for
                // its dissolved definition.
                let sketches_touched: Vec<SketchId> = self
                    .def_sketches
                    .iter()
                    .filter(|&(sid, &c)| c == new_def && !self.hidden_sketches.contains(sid))
                    .map(|(&sid, _)| sid)
                    .collect();
                for &sid in &sketches_touched {
                    self.hidden_sketches.insert(sid);
                }
                self.components[new_def].hidden = true;
                DocChange {
                    instances_touched: vec![instance],
                    components_touched: vec![prev_def, new_def],
                    sketches_touched,
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
            DocAction::ComponentRenamed {
                component,
                prev_name,
                ..
            } => {
                // Undo: restore the definition's previous name.
                let component = *component;
                self.components[component].name = prev_name.clone();
                self.component_change(component)
            }
            DocAction::TagDeleted { registry, nodes } => {
                // Undo: re-register the removed entries (restoring their
                // hidden flags) and restore every affected node's tag list.
                // A path the user RE-registered after the delete keeps its
                // current flag: `set_tag_hidden` is deliberately non-undoable
                // view state, so a fresh post-delete registration must not
                // be clobbered by the captured pre-delete value.
                for (path, hidden) in registry.iter() {
                    if !self.tag_meta.contains_key(path) {
                        self.tag_meta.insert(path.clone(), *hidden);
                    }
                }
                let mut change = DocChange::default();
                for (node, prev_tags, _) in nodes.clone() {
                    self.apply_node_tags(node, prev_tags);
                    match node {
                        NodeId::Object(id) => change.objects_touched.push(id),
                        NodeId::Group(id) => change.groups_touched.push(id),
                        NodeId::Instance(id) => change.instances_touched.push(id),
                    }
                }
                change
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
        if let DocAction::Compound { actions } = &action {
            let checkpoint = self.clone();
            let undo_start = self.undo.actions.len();
            for child in actions.iter().rev() {
                self.redo.push(child.clone());
            }
            let mut change = DocChange::default();
            for _ in 0..actions.len() {
                match self.redo() {
                    Ok(child_change) => merge_doc_change(&mut change, child_change),
                    Err(error) => {
                        *self = checkpoint;
                        return Err(error);
                    }
                }
            }
            self.undo.actions.truncate(undo_start);
            self.undo.push(action);
            self.debug_validate();
            return Ok(change);
        }
        if matches!(action, DocAction::CreatedObject { .. }) {
            let (change, undo_action) = self.redo_created_object(action)?;
            self.undo.push(undo_action);
            return Ok(change);
        }
        if matches!(action, DocAction::DetachedSketchIsland { .. }) {
            return self.redo_detached_island(action);
        }
        if matches!(action, DocAction::PlaceTextCompound(_)) {
            return self.redo_place_text_compound(action);
        }
        let change = match &action {
            DocAction::Compound { .. } => unreachable!("Compound is handled before the match"),
            // Dispatched to their dedicated helpers before this match.
            DocAction::CreatedObject { .. } => {
                unreachable!("CreatedObject is handled before the match")
            }
            DocAction::DetachedSketchIsland { .. } => {
                unreachable!("DetachedSketchIsland is handled before the match")
            }
            DocAction::PlaceTextCompound(_) => {
                unreachable!("PlaceTextCompound is handled before the match")
            }
            DocAction::ConsumedScaffolding { .. } => unreachable!(
                "ConsumedScaffolding only ever appears inside a PlaceTextCompound — place_text is its only producer"
            ),
            &DocAction::CopiedSketchIslands { copy } => {
                // Redo a copy: unhide the new sketch again (its contents
                // survived hiding bit-exactly). The source stays untouched.
                self.hidden_sketches.remove(&copy);
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![copy],
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
            &DocAction::FollowMeFace {
                result,
                merged_base,
            } => {
                // Redo a face-profile sweep: show the result; a merged
                // base is consumed again. See the matching undo comment for
                // the def-owned-result touched-list rationale.
                if let Some(rec) = self.objects.get_mut(result) {
                    rec.hidden = false;
                }
                let mut touched = vec![result];
                if let Some(base) = merged_base {
                    self.objects[base].hidden = true;
                    touched.push(base);
                }
                let (components_touched, instances_touched) = self.def_owner_change(result);
                DocChange {
                    objects_touched: touched,
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                let (components_touched, instances_touched) = self.def_owner_change(result);
                DocChange {
                    objects_touched: vec![result, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                }
            }
            DocAction::BooleanNodes {
                hidden_operands,
                result_objects,
                result_group,
                ..
            } => {
                // Redo a node combine: re-hide the operand subtrees, show the
                // result again.
                for &n in hidden_operands {
                    match n {
                        NodeId::Object(id) => self.objects[id].hidden = true,
                        NodeId::Group(id) => self.groups[id].hidden = true,
                        NodeId::Instance(id) => self.instances[id].hidden = true,
                    }
                }
                if let Some(g) = *result_group {
                    self.groups[g].hidden = false;
                }
                for &o in result_objects {
                    self.objects[o].hidden = false;
                }
                boolean_nodes_change(hidden_operands, result_objects, *result_group)
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
                let (components_touched, instances_touched) = self.def_owner_change(source);
                DocChange {
                    objects_touched: vec![source, a, b],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                let (components_touched, instances_touched) = self.def_owner_change(source);
                DocChange {
                    objects_touched,
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                // See the matching undo comment for the def-owned-target
                // touched-list rationale.
                let mut components_touched = Vec::new();
                let mut instances_touched = Vec::new();
                for &obj in objects {
                    let (c, i) = self.def_owner_change(obj);
                    for cc in c {
                        if !components_touched.contains(&cc) {
                            components_touched.push(cc);
                        }
                    }
                    for ii in i {
                        if !instances_touched.contains(&ii) {
                            instances_touched.push(ii);
                        }
                    }
                }
                DocChange {
                    objects_touched: objects.clone(),
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                }
            }
            &DocAction::TransformSketchIsland {
                sketch,
                island,
                forward,
                inverse,
                anchor_before,
                anchor_after,
            } => {
                // Redo an island transform by re-baking the forward (the
                // destination was validated when first applied). The id can
                // be stale for the same reason undo's can — re-resolve by
                // the pre-transform anchor; typed refusal if the geometry
                // is gone (action re-pushed, document untouched).
                let resolved = if self.sketches[sketch].islands().contains_key(island) {
                    Some(island)
                } else {
                    self.sketches[sketch].island_at_anchor(anchor_before)
                };
                let action = DocAction::TransformSketchIsland {
                    sketch,
                    island: resolved.unwrap_or(island),
                    forward,
                    inverse,
                    anchor_before,
                    anchor_after,
                };
                let Some(resolved) = resolved else {
                    self.redo.push(action);
                    return Err(DocumentError::Sketch(SketchError::UnknownIsland));
                };
                if let Err(e) = self.sketches[sketch].apply_transform_island(resolved, &forward) {
                    self.redo.push(action);
                    return Err(DocumentError::Sketch(e));
                }
                self.undo.push(action);
                self.debug_validate();
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                return Ok(DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
                    guides_touched: Vec::new(),
                });
            }
            &DocAction::TransformSketch {
                sketch, forward, ..
            } => {
                // Redo a sketch transform by re-baking the forward.
                self.sketches[sketch]
                    .apply_transform(&forward)
                    .expect("forward of a validated transform must re-apply");
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                DocChange {
                    objects_touched: Vec::new(),
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched,
                    components_touched,
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
                // created the sketch), then restore the post-gesture
                // snapshot.
                let (sketch, created) = (*sketch, *created);
                if created {
                    self.hidden_sketches.remove(&sketch);
                }
                if let Some(s) = self.sketches.get_mut(sketch) {
                    *s = (**after).clone();
                }
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                DocChange {
                    sketches_touched: vec![sketch],
                    components_touched,
                    instances_touched,
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
                // Redo delete: re-hide the subtree and splice `node` out
                // again.
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
            &DocAction::DeletedDefMember { component, object } => {
                // Redo delete_def_member: re-hide the member.
                self.objects[object].hidden = true;
                DocChange {
                    objects_touched: vec![object],
                    components_touched: vec![component],
                    instances_touched: self.instances_of(component),
                    ..Default::default()
                }
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
                // Un-hide any sketch this definition owned when it was
                // dissolved (the matching undo's counterpart above).
                let sketches_touched: Vec<SketchId> = self
                    .def_sketches
                    .iter()
                    .filter(|&(sid, &c)| c == component && self.hidden_sketches.contains(sid))
                    .map(|(&sid, _)| sid)
                    .collect();
                for &sid in &sketches_touched {
                    self.hidden_sketches.remove(&sid);
                }
                self.components[component].hidden = false;
                self.instances[instance].hidden = false;
                if let Some(pg) = parent {
                    self.splice_in_parent(pg, selected, NodeId::Instance(instance));
                }
                let leaves: Vec<ObjectId> = member_prior_parents.iter().map(|&(o, _)| o).collect();
                let mut change =
                    made_component_change(component, instance, parent, &leaves, consumed_groups);
                change.sketches_touched = sketches_touched;
                change
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
            DocAction::DuplicatedArray {
                roots,
                objects,
                groups,
                instances,
            } => {
                // Unhide every clone and re-append each root to its parent in
                // creation order — [`DocAction::Duplicated`]'s redo,
                // element-wise.
                for &o in objects {
                    self.objects[o].hidden = false;
                }
                for &g in groups {
                    self.groups[g].hidden = false;
                }
                for &i in instances {
                    self.instances[i].hidden = false;
                }
                for &(root, parent) in roots {
                    if let Some(pg) = parent {
                        self.groups[pg].members.push(root);
                    }
                }
                let mut change = DocChange {
                    objects_touched: objects.clone(),
                    groups_touched: groups.clone(),
                    instances_touched: instances.clone(),
                    ..Default::default()
                };
                for &(_, parent) in roots {
                    if let Some(pg) = parent
                        && !change.groups_touched.contains(&pg)
                    {
                        change.groups_touched.push(pg);
                    }
                }
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
                let (components_touched, instances_touched) = self.def_sketch_owner_change(sketch);
                DocChange {
                    sketches_touched: vec![sketch],
                    components_touched,
                    instances_touched,
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
                // Mirror undo (and the ObjectOp arm): keep the two logs
                // aligned on a refused replay by pushing the action back.
                if let Err(e) = rec.history.redo(&mut rec.object) {
                    self.redo.push(action);
                    return Err(map_history_err(e));
                }
                let instances_touched = self.instances_of(component);
                DocChange {
                    objects_touched: vec![object],
                    components_touched: vec![component],
                    instances_touched,
                    ..Default::default()
                }
            }
            DocAction::Exploded {
                instance,
                created,
                created_sketches,
            } => {
                let instance = *instance;
                self.instances[instance].hidden = true;
                for &o in created {
                    self.objects[o].hidden = false;
                }
                for &sid in created_sketches {
                    self.hidden_sketches.remove(&sid);
                }
                let parent = self.instances[instance].parent;
                if let Some(pg) = parent {
                    let nodes: Vec<NodeId> = created.iter().map(|&o| NodeId::Object(o)).collect();
                    self.splice_out_parent(pg, NodeId::Instance(instance), &nodes);
                }
                let mut change = DocChange {
                    objects_touched: created.clone(),
                    sketches_touched: created_sketches.clone(),
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
                ..
            } => {
                self.components[new_def].hidden = false;
                let new_members = self.components[new_def].members.clone();
                for o in new_members {
                    self.objects[o].hidden = false;
                }
                // Un-hide any sketch `new_def` owned when it was dissolved
                // (the matching undo's counterpart above).
                let sketches_touched: Vec<SketchId> = self
                    .def_sketches
                    .iter()
                    .filter(|&(sid, &c)| c == new_def && self.hidden_sketches.contains(sid))
                    .map(|(&sid, _)| sid)
                    .collect();
                for &sid in &sketches_touched {
                    self.hidden_sketches.remove(&sid);
                }
                self.instances[instance].def = new_def;
                // Re-clear the instance name a promotion moved onto the new
                // definition (exact by LIFO: at redo time the name is the
                // recorded `prev_instance_name` again, and the op left it
                // `None` — unconditionally for an unnamed instance too).
                self.instances[instance].name = None;
                DocChange {
                    instances_touched: vec![instance],
                    components_touched: vec![prev_def, new_def],
                    sketches_touched,
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
            DocAction::ComponentRenamed {
                component,
                next_name,
                ..
            } => {
                // Redo: re-apply the definition's next name.
                let component = *component;
                self.components[component].name = next_name.clone();
                self.component_change(component)
            }
            DocAction::TagDeleted { registry, nodes } => {
                // Redo: unregister the entries and strip the tag lists again.
                for (path, _) in registry.iter() {
                    self.tag_meta.remove(path);
                }
                let mut change = DocChange::default();
                for (node, _, next_tags) in nodes.clone() {
                    self.apply_node_tags(node, next_tags);
                    match node {
                        NodeId::Object(id) => change.objects_touched.push(id),
                        NodeId::Group(id) => change.groups_touched.push(id),
                        NodeId::Instance(id) => change.instances_touched.push(id),
                    }
                }
                change
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
    /// visible Object passes the topology validator.
    #[inline]
    fn debug_validate(&self) {
        // Torture mode (docs/DEVELOPMENT.md): run the topology validator on every
        // visible object after every op, **always-on** (release included), so a
        // corruption that slips past an op's own backstop surfaces here at the
        // exact op. The fuller debug-only invariant battery (tree) follows
        // below in debug builds.
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

/// The [`DocChange`] for `boolean_nodes`/its undo/redo: every node in the
/// consumed operand subtrees changed visibility, and every result piece (plus
/// the result container group, when present) appeared or disappeared.
fn boolean_nodes_change(
    hidden_operands: &[NodeId],
    result_objects: &[ObjectId],
    result_group: Option<GroupId>,
) -> DocChange {
    let mut objects_touched: Vec<ObjectId> = Vec::new();
    let mut groups_touched: Vec<GroupId> = Vec::new();
    let mut instances_touched: Vec<InstanceId> = Vec::new();
    for &n in hidden_operands {
        match n {
            NodeId::Object(o) => objects_touched.push(o),
            NodeId::Group(g) => groups_touched.push(g),
            NodeId::Instance(i) => instances_touched.push(i),
        }
    }
    objects_touched.extend_from_slice(result_objects);
    groups_touched.extend(result_group);
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

/// Orders a set of sketch edges into one connected chain of positions — the
/// path half of Follow Me's eligibility (the follow-me design §2) —
/// plus each walked segment's analytic [`CurveGeom`] attribution, in the
/// same order (the wrap segment last for a closed chain). Duplicate ids are
/// collapsed. Deterministic: an open chain starts at the smaller of its two
/// end vertex ids; a closed chain starts at its smallest member vertex id
/// and steps first onto that vertex's smaller-id incident edge.
fn chain_sketch_edges(
    sketch: &Sketch,
    edges: &[SketchEdgeId],
) -> Result<ResolvedFollowMePath, FollowMeError> {
    let unique: BTreeSet<SketchEdgeId> = edges.iter().copied().collect();
    if unique.is_empty() {
        return Err(FollowMeError::EmptyPath);
    }
    // Vertex -> incident selected edges, both in id order.
    let mut incident: BTreeMap<SketchVertexId, Vec<SketchEdgeId>> = BTreeMap::new();
    for &eid in &unique {
        let e = sketch
            .edges()
            .get(eid)
            .ok_or(FollowMeError::UnknownPathEdge)?;
        for v in [e.from, e.to] {
            let list = incident.entry(v).or_default();
            list.push(eid);
            if list.len() > 2 {
                return Err(FollowMeError::PathBranches);
            }
        }
    }
    let ends: Vec<SketchVertexId> = incident
        .iter()
        .filter(|(_, l)| l.len() == 1)
        .map(|(&v, _)| v)
        .collect();
    let closed = match ends.len() {
        0 => true,
        2 => false,
        // With every degree at most 2, one chain has exactly 0 or 2
        // degree-1 vertices; any other count means several open chains.
        _ => return Err(FollowMeError::PathDisconnected),
    };
    // Walk from the deterministic start; visiting every selected edge
    // exactly once proves connectedness.
    let start = if closed {
        *incident.keys().next().expect("nonempty incident map")
    } else {
        ends[0]
    };
    let mut points: Vec<Point3> = vec![sketch.vertices()[start].position];
    let mut curves: Vec<Option<CurveGeom>> = Vec::new();
    let mut visited: BTreeSet<SketchEdgeId> = BTreeSet::new();
    let mut at = start;
    while let Some(&next_edge) = incident[&at].iter().find(|e| !visited.contains(e)) {
        visited.insert(next_edge);
        let e = sketch.edges()[next_edge];
        curves.push(e.curve.and_then(|cid| sketch.curve_geom(cid)));
        at = if e.from == at { e.to } else { e.from };
        if closed && at == start {
            break;
        }
        points.push(sketch.vertices()[at].position);
    }
    if visited.len() != unique.len() {
        return Err(FollowMeError::PathDisconnected);
    }
    Ok((points, closed, curves))
}

/// Map a per-Object [`HistoryError`] onto a [`DocumentError`]. Empty-stack cases
/// cannot occur here (the document log guarantees the op exists), so they map to
/// `InverseFailed`-adjacent loud failures rather than being silently ignored.
fn map_history_err(e: HistoryError) -> DocumentError {
    match e {
        HistoryError::InverseFailed(op) => DocumentError::InverseFailed(op),
        HistoryError::InverseDiverged => DocumentError::InverseDiverged,
        // The document log only records ObjectOp steps that were applied, so the
        // delegated History always has the matching entry to undo/redo. Reaching
        // these is a kernel bug; surface it loudly rather than swallow it.
        HistoryError::NothingToUndo | HistoryError::NothingToRedo => {
            panic!("document/object history desync — kernel bug: {e}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tol;

    /// Extrude the unit box from a ground-sketch rectangle.
    fn extrude_unit_box(doc: &mut Document) -> ObjectId {
        let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("ground plane");
        let s = doc.add_sketch(plane);
        {
            let sk = doc.sketch_mut(s).expect("sketch is live");
            let p = [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ];
            for k in 0..4 {
                sk.add_segment(p[k], p[(k + 1) % 4]).expect("segment adds");
            }
        }
        let region = doc
            .sketch(s)
            .expect("sketch is live")
            .regions()
            .keys()
            .next()
            .expect("rectangle closes one region");
        doc.extrude_region(s, region, 1.0).expect("box extrudes").0
    }

    /// The unique face of `obj` whose plane normal matches `dir`, if any.
    fn face_with_normal(obj: &Object, dir: Vec3) -> Option<FaceId> {
        obj.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().approx_eq(dir, tol::NORMAL_DIRECTION))
            .map(|(id, _)| id)
    }

    /// Component-member wedge whose slanted cut face was pulled through
    /// `apply_def_op` (recording a [`DocAction::DefObjectOp`] whose undo is
    /// the `UnbuildPushPull` inverse). Returns `(component, member, wall)` —
    /// the first wall the pull built.
    fn def_wedge_with_pulled_cut_face(doc: &mut Document) -> (ComponentId, ObjectId, FaceId) {
        let cube = extrude_unit_box(doc);
        let plane = Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0))
            .expect("slice plane");
        let ((a, b), _) = doc.slice_node(cube, &plane).expect("slice the cube");
        let n = Vec3::new(1.0, 0.0, 1.0).normalized().expect("unit normal");
        // The wedge is the piece whose CUT face faces +n outward.
        let wedge = [a, b]
            .into_iter()
            .find(|&id| face_with_normal(doc.object(id).expect("live piece"), n).is_some())
            .expect("one piece owns the +n cut face");
        let (comp, _inst, _) = doc
            .make_component(&[NodeId::Object(wedge)])
            .expect("fold the wedge into a definition");
        let member = doc.def_members(comp).expect("live component")[0];
        let cut = face_with_normal(doc.object(member).expect("member is live"), n)
            .expect("member keeps the cut face");

        let (report, _) = doc
            .apply_def_op(
                comp,
                member,
                KernelOp::PushPull {
                    face: cut,
                    distance: 0.3,
                },
            )
            .expect("wedge cut-face pull builds walls");
        let wall = match report {
            KernelOpReport::PushPull(r) => r.created_faces[0],
            other => panic!("expected a PushPull report, got {other:?}"),
        };
        (comp, member, wall)
    }

    /// Imprint an inset rectangle strictly inside `face` of `member`,
    /// BYPASSING the per-object history — the op_specs unbuild-refusal
    /// technique, lifted to a definition member. Models any path that
    /// mutates geometry without a history record.
    fn bypass_imprint(doc: &mut Document, member: ObjectId, face: FaceId) {
        let rec = &mut doc.objects[member];
        let corners: Vec<Point3> = rec
            .object
            .loop_positions(rec.object.faces()[face].outer_loop)
            .collect();
        let inv = 1.0 / corners.len() as f64;
        let centroid = corners
            .iter()
            .fold(Point3::ORIGIN, |acc, &p| acc + p.to_vec() * inv);
        let rect: Vec<Point3> = corners
            .iter()
            .map(|&p| centroid + (p - centroid) * 0.4)
            .collect();
        rec.object
            .split_face_inner(face, &rect)
            .expect("imprint a hole bypassing history");
    }

    /// A REFUSED DefObjectOp undo must push the popped action back — exactly
    /// like the structurally-parallel ObjectOp arm — or the document log
    /// desyncs from the member's History (the next undo panics), the strong
    /// exception guarantee breaks (Err but the log mutated), and
    /// `history_generation` misses a stack-top change. Red-checked against
    /// the bare-`?` arm this replaces.
    #[test]
    fn refused_def_object_op_undo_pushes_the_action_back() {
        let mut doc = Document::new();
        let (_comp, member, wall) = def_wedge_with_pulled_cut_face(&mut doc);

        // The recorded wall gains a hole with no history record, so the
        // pending UnbuildPushPull inverse must refuse.
        bypass_imprint(&mut doc, member, wall);
        let holed = doc.object(member).expect("member is live").to_polygons();

        let gen_before = doc.history_generation();
        let err = doc.undo().expect_err("undo must refuse: wall is holed");
        assert!(
            matches!(err, DocumentError::InverseFailed(_)),
            "typed refusal, got {err:?}"
        );
        // The action is STILL the pending step (pushed back, logs aligned).
        assert!(doc.can_undo(), "the refused action stays on the stack");
        assert!(
            matches!(
                doc.peek_undo_object_op(),
                Some(KernelOp::UnbuildPushPull { .. })
            ),
            "the stack top is still the def op's recorded inverse"
        );
        assert!(
            doc.history_generation() > gen_before,
            "the push-back moves the history generation (stack-top identity)"
        );
        assert_eq!(
            doc.object(member).expect("member is live").to_polygons(),
            holed,
            "a refused undo leaves the member byte-identical"
        );

        // A retry fails identically — never a desync panic.
        let err2 = doc.undo().expect_err("still refused");
        assert!(matches!(err2, DocumentError::InverseFailed(_)));
    }

    /// The mirrored redo arm: undo the def-op pull (walls unbuild cleanly),
    /// bypass-imprint the CUT face, then redo — the proof-carrying replay
    /// (DEVELOPMENT.md rule 9) refuses on the changed geometry, and the
    /// refused action must return to the redo stack.
    #[test]
    fn refused_def_object_op_redo_pushes_the_action_back() {
        let mut doc = Document::new();
        let (_comp, member, _wall) = def_wedge_with_pulled_cut_face(&mut doc);

        doc.undo().expect("clean unbuild of the pulled walls");
        let n = Vec3::new(1.0, 0.0, 1.0).normalized().expect("unit normal");
        let cut = face_with_normal(doc.object(member).expect("member is live"), n)
            .expect("the cut face is back");
        bypass_imprint(&mut doc, member, cut);
        let holed = doc.object(member).expect("member is live").to_polygons();

        let gen_before = doc.history_generation();
        let err = doc
            .redo()
            .expect_err("redo must refuse: the recorded state diverged");
        assert!(doc.can_redo(), "the refused action stays on the redo stack");
        assert!(
            doc.history_generation() > gen_before,
            "the push-back moves the history generation"
        );
        assert_eq!(
            doc.object(member).expect("member is live").to_polygons(),
            holed,
            "a refused redo leaves the member byte-identical, got {err:?}"
        );

        let err2 = doc.redo().expect_err("still refused");
        assert!(
            std::mem::discriminant(&err2) == std::mem::discriminant(&err),
            "a retry fails identically ({err:?} vs {err2:?}) — never a desync panic"
        );
    }

    // ─────────────────────────────────────────────── 3D Text (place_text)

    /// Build a glyph-like 'O' sketch — an outer square loop plus a smaller
    /// concentric "counter" square loop — injected as ONE sketch-drawing
    /// gesture, mirroring how the 3D Text tool injects a glyph outline
    /// (`docs/design/3d-text.md`). The kernel's own region resolver
    /// produces exactly the two regions the concentric-rings test in
    /// `sketch.rs` documents for any two nested loops: the `ring` (fill —
    /// outer boundary with the counter as a hole) and the `disk` (the
    /// counter's own interior, no holes) — `place_text`'s caller must
    /// select only `ring`, never `disk`, or the letter's hole would fill
    /// in solid.
    fn glyph_o_sketch(doc: &mut Document) -> (SketchId, SketchRegionId, SketchRegionId) {
        let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("ground plane");
        let sketch = doc.add_sketch(plane);
        doc.begin_sketch_gesture(sketch).expect("gesture opens");
        {
            let sk = doc.sketch_mut(sketch).expect("sketch is live");
            let outer = [
                Point3::new(-1.0, -1.0, 0.0),
                Point3::new(1.0, -1.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(-1.0, 1.0, 0.0),
            ];
            for k in 0..4 {
                sk.add_segment(outer[k], outer[(k + 1) % 4])
                    .expect("outer segment adds");
            }
            let inner = [
                Point3::new(-0.4, -0.4, 0.0),
                Point3::new(0.4, -0.4, 0.0),
                Point3::new(0.4, 0.4, 0.0),
                Point3::new(-0.4, 0.4, 0.0),
            ];
            for k in 0..4 {
                sk.add_segment(inner[k], inner[(k + 1) % 4])
                    .expect("inner segment adds");
            }
        }
        doc.end_sketch_gesture(sketch).expect("gesture closes");

        let regions: Vec<SketchRegionId> =
            doc.sketch(sketch).expect("live").regions().keys().collect();
        assert_eq!(regions.len(), 2, "two nested loops trace as ring + disk");
        let ring = regions
            .iter()
            .copied()
            .find(|&r| !doc.sketch(sketch).unwrap().regions()[r].holes.is_empty())
            .expect("one region has the counter as a hole");
        assert_eq!(
            doc.sketch(sketch).unwrap().regions()[ring].holes.len(),
            1,
            "the ring has exactly one hole — the counter"
        );
        let disk = regions
            .into_iter()
            .find(|&r| r != ring)
            .expect("the other region is the disk");
        assert!(
            doc.sketch(sketch).unwrap().regions()[disk].holes.is_empty(),
            "the disk region (counter's own interior) has no holes"
        );
        (sketch, ring, disk)
    }

    #[test]
    fn place_text_extrudes_only_the_selected_fill_region_and_is_watertight() {
        let mut doc = Document::new();
        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);

        let (component, instance, _change) = doc
            .place_text(sketch, &[ring], 0.5, "3D Text \"O\"".to_string(), None)
            .expect("the ring alone places cleanly");

        let members = doc.def_members(component).expect("live definition");
        assert_eq!(members.len(), 1, "one fill region → one member object");
        let obj = doc.object(members[0]).expect("member is live");
        assert_eq!(
            obj.watertight(),
            WatertightState::Watertight,
            "an extruded ring-with-hole must be watertight (a real tunnel, not a leak)"
        );
        assert_eq!(
            doc.components[component].name.as_deref(),
            Some("3D Text \"O\"")
        );
        assert!(
            doc.instances[instance].pose == Transform::IDENTITY,
            "the fold's instance sits exactly where the extrusion happened"
        );
    }

    #[test]
    fn place_text_is_one_undo_step_and_redo_restores_it() {
        let mut doc = Document::new();
        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);
        let gen_before_place = doc.history_generation();

        let (component, instance, _change) = doc
            .place_text(sketch, &[ring], 0.5, "3D Text \"O\"".to_string(), None)
            .expect("places cleanly");
        assert!(
            doc.history_generation() > gen_before_place,
            "place_text pushed exactly one generation-advancing step"
        );

        doc.undo()
            .expect("a single undo reverses the whole placement");
        assert!(
            doc.components[component].hidden,
            "one undo hides the definition"
        );
        assert!(
            doc.instances[instance].hidden,
            "one undo hides the instance"
        );
        // The gesture that drew the glyph outline is folded into this SAME
        // compound step (it created the sketch fresh), so undoing the
        // whole placement removes the sketch too — "one undo removes the
        // text" (docs/design/3d-text.md) means the glyph scaffolding as
        // well, not just the solid.
        assert!(
            doc.sketch(sketch).is_none(),
            "the whole compound undo removes the sketch the gesture created, not just the solid"
        );
        assert!(!doc.can_undo(), "the whole compound was one single step");

        doc.redo()
            .expect("a single redo restores the whole placement");
        assert!(
            !doc.components[component].hidden,
            "one redo unhides the definition"
        );
        assert!(
            !doc.instances[instance].hidden,
            "one redo unhides the instance"
        );
        let members = doc.def_members(component).expect("live definition");
        let obj = doc.object(members[0]).expect("member is live");
        assert_eq!(
            obj.watertight(),
            WatertightState::Watertight,
            "redo reproduces the identical watertight solid"
        );
        assert!(!doc.can_redo(), "the whole compound replayed as one step");
    }

    #[test]
    fn place_text_births_into_the_given_group() {
        let mut doc = Document::new();
        // `group_nodes` needs at least one live member; seed the group with
        // a throwaway solid, matching the group-context contract every other
        // grouped-birth op (`follow_me_grouped`) is tested against.
        let seed = extrude_unit_box(&mut doc);
        let (group, _change) = doc
            .group_nodes(&[NodeId::Object(seed)])
            .expect("group forms from the seed");

        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);
        let (component, instance, change) = doc
            .place_text(
                sketch,
                &[ring],
                0.5,
                "3D Text \"O\"".to_string(),
                Some(group),
            )
            .expect("places inside the group");

        assert_eq!(
            doc.node_parent(NodeId::Instance(instance)),
            Some(group),
            "the folded instance lands inside the given group"
        );
        // `make_component` re-owns every member as `ObjectOwner::Definition`
        // (never a world group) — `node_parent(members[0])`, read BEFORE
        // that re-owning happens, is what `place_text` threads `group`
        // through to land the fold's instance there; the member itself no
        // longer has a world group parent at all once folded.
        let members = doc.def_members(component).expect("live definition");
        assert_eq!(
            doc.objects[members[0]].group_parent(),
            None,
            "a definition member is never a world object, so it has no group parent of its own"
        );
        assert!(change.groups_touched.contains(&group));
    }

    /// RED-CHECK (playtest finding 2): before `DocAction::ConsumedScaffolding`
    /// existed, a counter glyph (the 'O's disk) kept `place_text`'s sketch
    /// live after placement — only the `ring` is selected/extruded (see
    /// `glyph_o_sketch`'s doc comment) — so the counter loop lingered as
    /// live, visible scratch geometry in the document for as long as the
    /// placement wasn't undone. On that pre-fix code this test's first
    /// assertion (`doc.sketch(sketch).is_none()`) fails: the sketch was
    /// still `Some`. The fix makes `place_text` discard whatever the fill
    /// regions didn't consume and retire the whole scratch sketch in the
    /// same compound, so nothing outlives the placement.
    #[test]
    fn place_text_consumes_the_counter_glyphs_scratch_sketch() {
        let mut doc = Document::new();
        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);
        let (component, _instance, _change) = doc
            .place_text(sketch, &[ring], 0.5, "3D Text \"O\"".to_string(), None)
            .expect("the ring alone places cleanly");

        assert!(
            doc.sketch(sketch).is_none(),
            "the counter loop's leftover scaffolding must not keep the sketch live"
        );
        assert!(
            !doc.sketch_ids().contains(&sketch),
            "no leftover world sketch survives placement"
        );
        let members = doc.def_members(component).expect("live definition");
        assert_eq!(members.len(), 1, "one fill region → one member object");
        let obj = doc.object(members[0]).expect("member is live");
        assert_eq!(
            obj.watertight(),
            WatertightState::Watertight,
            "the ring's solid is unaffected by discarding the counter's own scaffolding"
        );
    }

    /// Companion to the RED-CHECK above: undo must restore the placement
    /// exactly — including the discarded counter scaffolding — and redo
    /// must re-discard it, twice over (rule 9: undo restores everything,
    /// redo re-removes, with no drift across repeated cycles).
    #[test]
    fn place_text_undo_redo_round_trips_exactly_with_a_counter_glyph() {
        let mut doc = Document::new();
        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);
        let (component, instance, _change) = doc
            .place_text(sketch, &[ring], 0.5, "3D Text \"O\"".to_string(), None)
            .expect("the ring alone places cleanly");
        let save_after_place = doc.save();

        for cycle in 0..2 {
            doc.undo().unwrap_or_else(|e| {
                panic!("cycle {cycle}: single undo reverses the whole placement: {e:?}")
            });
            assert!(
                doc.components[component].hidden,
                "cycle {cycle}: undo hides the definition"
            );
            assert!(
                doc.instances[instance].hidden,
                "cycle {cycle}: undo hides the instance"
            );
            assert!(
                doc.sketch(sketch).is_none(),
                "cycle {cycle}: undo removes the whole gesture-created sketch, counter \
                 scaffolding and all — not just the extruded ring"
            );
            assert!(
                !doc.can_undo(),
                "cycle {cycle}: the whole compound was one single step"
            );

            doc.redo().unwrap_or_else(|e| {
                panic!("cycle {cycle}: single redo restores the whole placement: {e:?}")
            });
            assert!(
                !doc.components[component].hidden,
                "cycle {cycle}: redo unhides the definition"
            );
            assert!(
                !doc.instances[instance].hidden,
                "cycle {cycle}: redo unhides the instance"
            );
            assert!(
                doc.sketch(sketch).is_none(),
                "cycle {cycle}: redo re-consumes the counter's scaffolding — the sketch stays \
                 retired exactly as it was right after the original placement"
            );
            assert!(
                !doc.can_redo(),
                "cycle {cycle}: the whole compound replayed as one step"
            );
            let members = doc.def_members(component).expect("live definition");
            let obj = doc.object(members[0]).expect("member is live");
            assert_eq!(
                obj.watertight(),
                WatertightState::Watertight,
                "cycle {cycle}: redo reproduces the identical watertight solid"
            );
            assert_eq!(
                doc.save(),
                save_after_place,
                "cycle {cycle}: redo lands byte-identical to the original placement — no drift \
                 across repeated undo/redo cycles"
            );
        }
    }

    /// Defense-in-depth pin for `compound_reversal_feasible`'s refusal
    /// branch (DEVELOPMENT.md rule 9: a guard with no live repro still gets
    /// a red-checked spec, not a deleted one). Before
    /// `DocAction::ConsumedScaffolding` existed, a counter glyph left
    /// `place_text`'s sketch live after placement, and the fuzz harness's
    /// `DocOp::TouchGlyphSketch` could draw an untracked segment onto it —
    /// exactly the residue that made this branch refuse typed (pinned by
    /// the now-deleted
    /// `undo_compound_refuses_atomically_when_a_counter_glyph_kept_the_sketch_live`
    /// spec). `place_text` now always retires (hides) its sketch before
    /// returning, so `Document::sketch_mut` refuses every live API path
    /// onto it for as long as the `PlaceTextCompound` sits on the undo stack —
    /// closing that hole at the source. A search through every other
    /// document op turned up no surviving way to leave live, reachable
    /// residue on a `PlaceTextCompound`'s scaffolding sketch either: nothing besides
    /// `place_text`/`undo`/`redo` ever touches one of its sketches, undoing
    /// the `PlaceTextCompound` is all-or-nothing (it either fully restores the
    /// original two-loop sketch or is refused), and the instant it restores
    /// the original sketch, that exact restore has already succeeded — so
    /// there is no real op sequence left that lands the sketch in a
    /// half-restored, conflicting state for a subsequent op to build on.
    /// This spec reconstructs that now-unreachable-by-real-ops state
    /// directly (via the private `sketches` field, standing in for "a
    /// future producer or bypass this reasoning missed"), the same
    /// technique `compound_reversal_feasible`'s own doc comment describes
    /// as its reason for existing.
    #[test]
    fn compound_reversal_feasible_traps_a_residue_left_on_a_retired_scratch_sketch() {
        let mut doc = Document::new();
        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);
        let (component, instance, _change) = doc
            .place_text(sketch, &[ring], 0.5, "3D Text \"O\"".to_string(), None)
            .expect("the ring alone places cleanly");
        assert!(
            doc.hidden_sketches.contains(&sketch),
            "place_text always retires its scratch sketch now — no live API reaches it while \
             the PlaceTextCompound is pending, which is exactly why this defense-in-depth check can no \
             longer be triggered through real ops"
        );

        // Reach around the now-closed hole directly: land untracked geometry
        // exactly where the ring's own removed baseline edge needs to
        // re-insert, on the sketch slot itself (never removed, only
        // hidden — `Document::sketch_mut`'s own doc comment).
        doc.sketches
            .get_mut(sketch)
            .expect("sketch slots are never removed, only hidden")
            .add_segment(Point3::new(-1.0, -1.0, 0.0), Point3::new(1.0, -1.0, 0.0))
            .expect("lands cleanly on the emptied, hidden sketch");

        let save_before = doc.save();
        let err = doc
            .undo()
            .expect_err("the ring's removed baseline collides with the residue");
        assert!(
            matches!(err, DocumentError::Sketch(SketchError::RestoreConflicts)),
            "typed refusal, got {err:?}"
        );

        // Nothing orphaned: the fold is still fully intact, not mid-unwind.
        assert!(
            !doc.components[component].hidden,
            "the component must still be live — not wedged mid-unwind"
        );
        assert!(
            !doc.instances[instance].hidden,
            "the instance must still be live — not wedged mid-unwind"
        );
        assert_eq!(
            doc.save(),
            save_before,
            "a refused compound undo leaves the document's live, visible state byte-identical"
        );
        assert!(
            doc.can_undo(),
            "the refused PlaceTextCompound stays on the undo stack, not lost"
        );
        let err2 = doc.undo().expect_err("a retry fails identically");
        assert!(
            std::mem::discriminant(&err2) == std::mem::discriminant(&err),
            "never a desync panic on retry"
        );
    }

    #[test]
    fn place_text_refuses_when_the_top_of_the_undo_stack_is_not_its_gesture() {
        let mut doc = Document::new();
        // A sketch drawn WITHOUT the gesture bracket — `place_text`'s caller
        // contract (immediately after `end_sketch_gesture` on this same
        // sketch) is violated, so it must refuse rather than fold in some
        // unrelated action.
        let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("ground plane");
        let sketch = doc.add_sketch(plane);
        let region = {
            let sk = doc.sketch_mut(sketch).expect("sketch is live");
            let p = [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ];
            for k in 0..4 {
                sk.add_segment(p[k], p[(k + 1) % 4]).expect("segment adds");
            }
            sk.regions().keys().next().expect("square closes a region")
        };

        let gen_before = doc.history_generation();
        let err = doc
            .place_text(sketch, &[region], 0.5, "3D Text \"I\"".to_string(), None)
            .expect_err("no gesture was ever closed on this sketch");
        assert!(matches!(err, DocumentError::UnexpectedGestureState));
        assert_eq!(
            doc.history_generation(),
            gen_before,
            "a refused place_text touches neither stack"
        );
        assert!(
            doc.sketch(sketch).expect("untouched").regions().len() == 1,
            "the document is completely untouched"
        );
    }

    #[test]
    fn place_text_refuses_on_empty_regions_document_untouched() {
        let mut doc = Document::new();
        let (sketch, _ring, _disk) = glyph_o_sketch(&mut doc);

        let gen_before = doc.history_generation();
        let err = doc
            .place_text(sketch, &[], 0.5, "Nothing".to_string(), None)
            .expect_err("nothing to place");
        assert!(matches!(err, DocumentError::EmptyComponent));
        assert_eq!(
            doc.history_generation(),
            gen_before,
            "a refused place_text touches neither stack"
        );
        assert_eq!(
            doc.sketch(sketch).expect("untouched").regions().len(),
            2,
            "the just-closed gesture is left exactly as the caller committed it"
        );
    }

    #[test]
    fn place_text_refuses_on_an_unknown_region_id_document_untouched() {
        let mut doc = Document::new();
        let (sketch, ring, _disk) = glyph_o_sketch(&mut doc);
        // Consume the ring directly (not through `place_text`), invalidating
        // its region id while the disk survives and keeps the sketch live —
        // `region_scaffolding`'s exclusive-edges rule (only the ring's own
        // outer boundary is exclusive to it) means this does not touch the
        // disk at all.
        doc.extrude_region(sketch, ring, 0.5)
            .expect("the ring extrudes independently, consuming its own boundary");

        let gen_before = doc.history_generation();
        let err = doc
            .place_text(sketch, &[ring], 0.5, "Stale".to_string(), None)
            .expect_err("the pre-extrusion `ring` handle is now stale");
        assert!(
            matches!(err, DocumentError::Sketch(SketchError::UnknownRegion)),
            "typed refusal, got {err:?}"
        );
        assert_eq!(
            doc.history_generation(),
            gen_before,
            "a refused place_text touches neither stack — profile/scaffolding validation \
             for every region runs before anything commits, including before the gesture \
             is even looked at"
        );
    }
}
