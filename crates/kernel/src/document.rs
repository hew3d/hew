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
use crate::ids::{ComponentId, FaceId, GroupId, InstanceId, MaterialId, ObjectId, SketchId};
use crate::material::Material;
use crate::math::Plane;
use crate::ops::{BooleanError, BooleanOp, ExtrudeError};
use crate::serialize::{DocSaveData, LoadError, NodeRefDto, decode_document_raw, encode_document};
use crate::sketch::{Sketch, SketchError, SketchRegionId};
use crate::topo::Object;
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
    /// A move/rotate/scale baked into one or more objects' geometry. A single
    /// object carries one target; a group transform carries every leaf object
    /// beneath it. Undo bakes `inverse` into each, redo bakes `forward`;
    /// the transform is handle-stable so the `ObjectId`s never change.
    Transform {
        objects: Vec<ObjectId>,
        forward: Transform,
        inverse: Transform,
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
    /// `group_nodes` was called with no members.
    EmptyGroup,
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
            DocumentError::UnknownFace => write!(f, "no such face in the target object"),
            DocumentError::UnknownMaterial => write!(f, "no such material in this document"),
            DocumentError::UnknownGroup => write!(f, "no such group in this document"),
            DocumentError::UnknownComponent => {
                write!(f, "no such component definition in this document")
            }
            DocumentError::UnknownInstance => write!(f, "no such instance in this document"),
            DocumentError::EmptyGroup => write!(f, "cannot group an empty selection"),
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

        // ── Collect live groups (in slotmap key order) ─────────────────────
        let groups: Vec<(GroupId, Vec<NodeId>)> = self
            .groups
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.members.clone()))
            .collect();

        // ── Collect live components (in slotmap key order) ─────────────────
        let components: Vec<(ComponentId, Vec<ObjectId>)> = self
            .components
            .iter()
            .filter(|(_, c)| !c.hidden)
            .map(|(id, c)| (id, c.members.clone()))
            .collect();

        // ── Collect live instances (in slotmap key order) ──────────────────
        let instances: Vec<(InstanceId, ComponentId, Transform)> = self
            .instances
            .iter()
            .filter(|(_, rec)| !rec.hidden)
            .map(|(id, rec)| (id, rec.def, rec.pose))
            .collect();

        // ── Collect live sketches (in slotmap key order) ───────────────────
        let sketches: Vec<(SketchId, Sketch)> = self
            .sketches
            .iter()
            .map(|(id, sk)| (id, sk.clone()))
            .collect();

        // ── Collect root nodes: top-level visible world nodes ─────────────
        // Roots = all live objects/groups/instances whose parent is None.
        // We emit objects first, then groups, then instances (same order as
        // `top_level_nodes`) to be deterministic.
        let roots: Vec<NodeId> = self.top_level_nodes();

        // ── Collect consumed (SketchId, SketchRegionId) pairs ─────────────
        // Filter to only those where both the sketch and region are live.
        let mut consumed: Vec<(SketchId, SketchRegionId)> = self
            .consumed
            .iter()
            .filter(|(sid, _)| self.sketches.contains_key(*sid))
            .copied()
            .collect();
        // Sort for determinism (never emit a HashSet directly — rule from plan).
        // The slotmap keys are not directly comparable, but their iteration
        // order from the slotmaps IS stable. We derive dense indices for the
        // sort key by looking them up in the sketch slotmap iteration order.
        let sketch_dense: std::collections::HashMap<SketchId, usize> = sketches
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

        encode_document(DocSaveData {
            materials,
            world_objects,
            def_objects,
            groups,
            components,
            instances,
            sketches,
            roots,
            consumed,
        })
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
            });
            dense_obj_ids.push(oid);
        }

        // ── 3. Insert sketches → build dense→SketchId map ─────────────────
        let sketch_ids: Vec<SketchId> = raw
            .sketches
            .into_iter()
            .map(|sk| doc.sketches.insert(sk))
            .collect();

        // ── 4. Insert components → build dense→ComponentId map ────────────
        // Each component's members are dense object ids → now live ObjectIds.
        let mut comp_ids: Vec<ComponentId> = Vec::with_capacity(raw.components.len());
        for member_dense_ids in &raw.components {
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
            });
            comp_ids.push(cid);
            // Re-assign ownership for these objects.
            for oid in members {
                doc.objects[oid].owner = ObjectOwner::Definition(cid);
            }
        }

        // ── 5. Insert instances → build dense→InstanceId map ─────────────
        let mut inst_ids: Vec<InstanceId> = Vec::with_capacity(raw.instances.len());
        for (comp_dense, pose) in raw.instances {
            let cid =
                *comp_ids
                    .get(comp_dense as usize)
                    .ok_or_else(|| LoadError::DanglingReference {
                        what: format!("instance def component dense id {comp_dense} out of range"),
                    })?;
            let iid = doc.instances.insert(InstanceRecord {
                def: cid,
                pose,
                parent: None,
                hidden: false,
            });
            inst_ids.push(iid);
        }

        // ── 6. Insert groups → build dense→GroupId map ────────────────────
        // Groups may reference other groups (nesting), so insert all first,
        // then patch members.
        let mut grp_ids: Vec<GroupId> = Vec::with_capacity(raw.groups.len());
        for _ in &raw.groups {
            let gid = doc.groups.insert(GroupRecord {
                members: Vec::new(),
                parent: None,
                hidden: false,
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

        // Undo/redo stacks are empty by construction (Document::new() gives empty).
        Ok(doc)
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

    // -------------------------------------------------------------- materials

    /// Add `material` to the palette and return its handle. Additive and
    /// **not** undoable on its own — only face assignment ([`paint_face`]) is.
    ///
    /// [`paint_face`]: Document::paint_face
    pub fn add_material(&mut self, material: Material) -> MaterialId {
        self.materials.insert(material)
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
            groups_touched: Vec::new(),
            instances_touched: Vec::new(),
            components_touched: Vec::new(),
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
            Some(rec) if !rec.hidden && rec.is_world() => &rec.object,
            _ => return Err(DocumentError::UnknownObject),
        };
        let obj_b = match self.objects.get(b) {
            Some(rec) if !rec.hidden && rec.is_world() => &rec.object,
            _ => return Err(DocumentError::UnknownObject),
        };

        let result = Object::boolean(op, obj_a, obj_b, &Transform::IDENTITY)
            .map_err(DocumentError::Boolean)?;

        let id = self.objects.insert(ObjectRecord {
            object: result,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::World { parent: None },
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
        })
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
            // Nesting a component inside a definition is deferred.
            if matches!(m, NodeId::Instance(_)) {
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
        // geometry, fresh per-object history).
        let new_def = self.components.insert(ComponentDef {
            members: Vec::new(),
            hidden: false,
        });
        let mut new_members: Vec<ObjectId> = Vec::with_capacity(members.len());
        for m in members {
            let object = self.objects[m].object.clone();
            let id = self.objects.insert(ObjectRecord {
                object,
                history: History::new(),
                hidden: false,
                owner: ObjectOwner::Definition(new_def),
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
    pub fn undo(&mut self) -> Result<DocChange, DocumentError> {
        let action = self.undo.pop().ok_or(DocumentError::NothingToUndo)?;
        let change = match &action {
            &DocAction::CreatedObject { id, sketch, region } => {
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.hidden = true;
                }
                self.consumed.remove(&(sketch, region));
                DocChange {
                    objects_touched: vec![id],
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                }
            }
            &DocAction::ObjectOp { object } => {
                let rec = &mut self.objects[object];
                rec.history.undo(&mut rec.object).map_err(map_history_err)?;
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
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
            &DocAction::CreatedObject { id, sketch, region } => {
                if let Some(rec) = self.objects.get_mut(id) {
                    rec.hidden = false;
                }
                self.consumed.insert((sketch, region));
                DocChange {
                    objects_touched: vec![id],
                    sketches_touched: vec![sketch],
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
                }
            }
            &DocAction::ObjectOp { object } => {
                let rec = &mut self.objects[object];
                rec.history.redo(&mut rec.object).map_err(map_history_err)?;
                DocChange {
                    objects_touched: vec![object],
                    sketches_touched: Vec::new(),
                    groups_touched: Vec::new(),
                    instances_touched: Vec::new(),
                    components_touched: Vec::new(),
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
