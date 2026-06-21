//! Import recipe IR and `Document::ingest`.
//!
//! `dae-import` (and future importers) parse external formats and emit an
//! `ImportScene` — a *recipe* of positions + face-index lists + dense material
//! indices — then call `Document::ingest` to materialise it into the document.
//!
//! Keeping construction in the kernel means:
//! - `FaceMaterial = Option<MaterialId>` is resolved after palette insertion.
//! - `from_polygons_with_materials` rejection/watertight tallying is automatic.
//! - The `DocAction::Imported` undo step wraps the whole import atomically.
//!
//! The recipe types carry no `serde` derives — they are in-memory only and do
//! **not** touch the file format (DEVELOPMENT.md rule).

use crate::ids::{ComponentId, GroupId, InstanceId, MaterialId, ObjectId};
use crate::material::{Material, UvFrame};
use crate::math::Point3;
use crate::transform::Transform;

// ── Recipe IR ────────────────────────────────────────────────────────────────

/// One mesh recipe: deduplicated/welded positions + CCW polygon index lists,
/// plus per-face and base material as DENSE indices into `ImportScene::materials`
/// (`NO_MATERIAL` sentinel = none — reuse `crate::serialize::NO_MATERIAL`).
pub struct MeshRecipe {
    /// Source node/geometry id, used in diagnostics and `ImportReport`.
    pub name: String,
    /// Unique vertex positions (already in Hew world space: meters, Z-up).
    pub positions: Vec<Point3>,
    /// CCW-wound polygon index lists (from outside), indices into `positions`.
    pub faces: Vec<Vec<usize>>,
    /// Per-face dense material index, parallel to `faces`.
    /// `crate::serialize::NO_MATERIAL` means no material assigned.
    pub face_materials: Vec<u32>,
    /// Per-face affine UV frame, parallel to `faces` ( extension). `None`
    /// entries mean no frame was fitted for that face ( `world_size` fallback
    /// in the tessellator). Set by `dae-import` when TEXCOORD inputs are present.
    pub face_uv_frames: Vec<Option<UvFrame>>,
    /// Per-face inner-loop index lists (holes), parallel to `faces`. For face `i`,
    /// `face_holes[i]` is a list of hole loops (each a `Vec<usize>` of position
    /// indices). An empty entry means face `i` has no holes, which is the common
    /// case and preserves byte-identical behaviour with the no-holes path. Set by
    /// `dae-import` when COLLADA `<ph>`/`<h>` elements are present.
    pub face_holes: Vec<Vec<Vec<usize>>>,
    /// Dense material index for the object's base material, or `NO_MATERIAL`.
    pub base_material: u32,
    /// Tag paths decoded from `__HEWMETA__` (or `__HEWTAG__`) in the source
    /// node name. Root-first segment lists, e.g. `[["Structure","Roof"]]`.
    /// Set by `dae-import` (WS2); default empty until that workstream lands.
    pub tags: Vec<Vec<String>>,
}

/// A shared definition recipe: a flat set of meshes in definition-local coords.
pub struct DefRecipe {
    /// Source name for the definition (e.g. a SketchUp component name), used as
    /// the display name for its instances. `None` falls back to a positional
    /// label in the UI.
    pub name: Option<String>,
    /// The meshes that make up this component definition.
    pub meshes: Vec<MeshRecipe>,
}

/// A node in the imported scene tree.
pub enum ImportNode {
    /// A mesh that becomes a world-space `Object`.
    Mesh(MeshRecipe),
    /// A non-destructive container (becomes a `Group`).
    Group {
        /// Display name for the group.
        name: String,
        /// Child nodes of this group.
        children: Vec<ImportNode>,
        /// Tag paths for this group node (from `__HEWMETA__` decode, WS2).
        tags: Vec<Vec<String>>,
    },
    /// An instance of a shared definition (index into `ImportScene::defs`).
    Instance {
        /// Dense index into `ImportScene::defs`.
        def: usize,
        /// Pose (definition-local → world) for this placement.
        pose: Transform,
        /// Tag paths for this instance node (from `__HEWMETA__` decode, WS2).
        tags: Vec<Vec<String>>,
    },
}

/// The complete import recipe: produced by `dae-import`, consumed by
/// `Document::ingest`.
pub struct ImportScene {
    /// Materials to add to the palette; face dense indices refer here.
    pub materials: Vec<Material>,
    /// Component definitions (shared geometry library entries).
    pub defs: Vec<DefRecipe>,
    /// Top-level world tree nodes to splice in.
    pub roots: Vec<ImportNode>,
}

// ── Report ───────────────────────────────────────────────────────────────────

/// A mesh that `from_polygons_with_materials` rejected.
pub struct SkippedMesh {
    /// Mesh name (from the recipe).
    pub name: String,
    /// `TopologyError::to_string()` explaining the rejection.
    pub reason: String,
}

/// Summary returned by `Document::ingest`.
pub struct ImportReport {
    /// Total `Object`s created (world objects + definition members).
    pub objects_created: usize,
    /// How many of the created objects are watertight.
    pub watertight: usize,
    /// How many of the created objects have open (non-watertight) shells.
    pub leaky: usize,
    /// Meshes that `from_polygons_with_materials` rejected — reported but not
    /// repaired (DEVELOPMENT.md rule 4).
    pub skipped: Vec<SkippedMesh>,
    /// Image URIs that the importer could not resolve from the `ImageMap`
    /// (populated by `dae-import`, passed through `ingest` unchanged).
    pub textures_missing: Vec<String>,
}

// ── DocAction::Imported + ingest ──────────────────────────────────────────────
// (These are defined on Document in document.rs; the types live here so callers
//  can depend on them without pulling in all of document.rs.)

/// The stable handles created by one `Document::ingest` call, stored in
/// `DocAction::Imported` for undo/redo.
pub struct ImportedHandles {
    /// Top-level created node ids (for ordering/cleanup).
    pub roots: Vec<crate::document::NodeId>,
    /// ALL created `ObjectId`s — world objects and definition members.
    pub objects: Vec<ObjectId>,
    /// Created `ComponentId`s (shared definitions).
    pub components: Vec<ComponentId>,
    /// Created `InstanceId`s.
    pub instances: Vec<InstanceId>,
    /// Created `GroupId`s.
    pub groups: Vec<GroupId>,
    /// Material ids added to the palette (used to undo material palette entries
    /// if we ever need it; currently kept for completeness).
    pub materials: Vec<MaterialId>,
}
