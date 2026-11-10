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

use std::collections::BTreeMap;

use crate::attr::AttrDict;
use crate::camera::CameraState;
use crate::document::NodeId;
use crate::ids::{ComponentId, GroupId, InstanceId, MaterialId, ObjectId};
use crate::material::{Material, UvFrame};
use crate::math::{Plane, Point3};
use crate::scenes::{DisplayState, SectionPlaneState};
use crate::sketch::{CurveAnalytic, CurveGeom};
use crate::topo::SurfaceRef;
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
    /// What the source says about the mesh beyond its faces (a `.skp`'s
    /// soft edges, welded arcs and curved surfaces) and the importer's key
    /// for it. Empty by default.
    pub extra: MeshExtra,
}

impl MeshRecipe {
    /// This recipe's Object, built the way [`crate::Document::ingest`]
    /// builds it: the holes-aware import path at
    /// [`crate::tol::IMPORT_PLANE_DIST`], each face given the material at
    /// its index in `materials` (none past its end) and its UV frame from
    /// `face_uv_frames` (none past its end).
    pub(crate) fn build_object(
        &self,
        materials: &[crate::material::FaceMaterial],
    ) -> Result<crate::topo::Object, crate::error::TopologyError> {
        let frames: Vec<Option<UvFrame>> = (0..self.faces.len())
            .map(|i| self.face_uv_frames.get(i).copied().flatten())
            .collect();
        crate::topo::Object::from_polygons_with_holes_import(
            &self.positions,
            &self.faces,
            &self.face_holes,
            materials,
            &frames,
        )
    }

    /// Whether [`crate::Document::ingest`] would build this mesh, answered
    /// without a document: `Ok` when it would, else the typed error it
    /// would skip the mesh with (an [`ImportReport::skipped`] entry's
    /// reason is that error's text). The check builds the mesh through the
    /// very path `ingest` takes; materials take no part in whether a mesh
    /// builds, so none are resolved. A host that must place a recipe whole
    /// or not at all checks it first, so a refusal changes nothing.
    pub fn check_builds(&self) -> Result<(), crate::error::TopologyError> {
        self.build_object(&[]).map(|_| ())
    }
}

/// Source claims about a mesh's edges and faces, applied by `ingest` to the
/// built Object wherever they hold — claims are metadata, never geometry,
/// so one the built Object contradicts is dropped (the map-or-drop
/// contract of [`SurfaceRef`] and [`crate::topo::Edge::curve`]), never
/// forced. Positions name edges, since healing may renumber vertices; an
/// endpoint matches the built vertex within the Object's planarity
/// tolerance.
#[derive(Debug, Clone, Default)]
pub struct MeshExtra {
    /// The importer's identity for this mesh's Object (see
    /// [`ImportNode::Group::key`]); reported back in [`ImportReport::keys`].
    pub key: Option<u64>,
    /// Edges drawn soft in the source: the seams of a curved surface.
    /// Applied to interior edges only (a soft edge has two faces).
    pub soft_edges: Vec<[Point3; 2]>,
    /// Edges that are chords of a circle (a welded arc in the source).
    pub edge_curves: Vec<([Point3; 2], CurveGeom)>,
    /// Curved surfaces the mesh's faces may be facets of: each face whose
    /// plane and vertices fit one becomes its chord facet.
    pub surfaces: Vec<SurfaceRef>,
    /// USER-hidden in the source (a hidden `.skp` group standing for one
    /// solid): imported, registered hidden-by-default, never dropped.
    pub hidden: bool,
    /// Per-face dense back paint, parallel to [`MeshRecipe::faces`]
    /// (`NO_MATERIAL` for none; empty for a mesh with none): a `.skp`
    /// face painted on its back (see [`crate::topo::Face::back_material`]).
    pub face_back_materials: Vec<u32>,
}

/// A drawn, unextruded sketch carried by an import: one planar set of
/// segments, with its welded curves.
#[derive(Debug, Clone)]
pub struct ImportSketch {
    /// The sketch plane (world frame, or the owning definition's frame).
    pub plane: Plane,
    /// Dense index into [`ImportScene::defs`] of the owning definition;
    /// `None` for a world sketch.
    pub def: Option<usize>,
    /// The key ([`ImportNode::Group::key`]) of the group the sketch is
    /// drawn in (a `.skp` group of edges), `None` for one in no group. The
    /// group must lie in the sketch's frame (`def`); one that does not is
    /// ignored and the sketch stays free.
    pub group: Option<u64>,
    /// Segment endpoints, in drawing order. Each must lie on `plane`.
    pub segments: Vec<[Point3; 2]>,
    /// Welded curves: the segment indices of each chain, and the circle it
    /// was drawn from when it has one.
    pub curves: Vec<(Vec<usize>, Option<CurveAnalytic>)>,
    /// Tag paths the sketch carries (a `.skp` edge's layer), root-first:
    /// it is not shown while one of them, or a tag above one, is hidden.
    /// Empty for none. An import declares the tags it uses
    /// ([`ImportScene::tags`]).
    pub tags: Vec<Vec<String>>,
}

/// What an imported attribute dictionary hangs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAttrTarget {
    /// The document itself.
    Document,
    /// Every node built from the import node with this key.
    Node(u64),
    /// A definition, by dense index into [`ImportScene::defs`].
    Def(usize),
    /// A palette material, by dense index into [`ImportScene::materials`].
    Material(usize),
    /// A declared tag, by path.
    Tag(Vec<String>),
}

/// The parts of an import beyond the node tree: sketches and attribute
/// dictionaries. Empty by default.
#[derive(Debug, Clone, Default)]
pub struct ImportSceneExtra {
    /// Drawn sketches, created inside the import's undo step.
    pub sketches: Vec<ImportSketch>,
    /// Attribute dictionaries to set, in order (a later one for the same
    /// target replaces an earlier one's namespaces of the same name).
    pub attrs: Vec<(ImportAttrTarget, AttrDict)>,
    /// Dimensions and leader texts, with free anchors (an import cannot
    /// name nodes it is still building). Created inside the import's undo
    /// step.
    pub annotations: Vec<crate::annotation::Annotation>,
    /// Each annotation's tag paths, parallel to `annotations` (empty, or
    /// short, for none).
    pub annotation_tags: Vec<Vec<Vec<String>>>,
}

/// A shared definition recipe: a subtree in definition-local coords.
pub struct DefRecipe {
    /// Source name for the definition (e.g. a SketchUp component name), used as
    /// the display name for its instances. `None` falls back to a positional
    /// label in the UI.
    pub name: Option<String>,
    /// The definition's member subtree. Meshes and groups build
    /// definition-owned content; an [`ImportNode::Instance`] member nests
    /// another definition (its `def` index must not reach this definition
    /// back through the recipe graph — `ingest` refuses cyclic scenes with a
    /// typed error, never repairs them).
    pub children: Vec<ImportNode>,
}

impl DefRecipe {
    /// A flat, meshes-only definition — the shape every pre-nesting importer
    /// emits.
    pub fn from_meshes(name: Option<String>, meshes: Vec<MeshRecipe>) -> Self {
        DefRecipe {
            name,
            children: meshes.into_iter().map(ImportNode::Mesh).collect(),
        }
    }

    /// Every mesh recipe in this definition's own subtree, depth-first —
    /// groups descended, nested `Instance` members NOT followed (their
    /// meshes belong to the referenced definition). Diagnostics and tests
    /// inspect definitions through this.
    pub fn mesh_recipes(&self) -> impl Iterator<Item = &MeshRecipe> {
        fn walk<'a>(nodes: &'a [ImportNode], out: &mut Vec<&'a MeshRecipe>) {
            for n in nodes {
                match n {
                    ImportNode::Mesh(m) => out.push(m),
                    ImportNode::Group { children, .. } => walk(children, out),
                    ImportNode::Instance { .. } => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.children, &mut out);
        out.into_iter()
    }
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
        /// USER-hidden in the source (a `.skp` hidden group): imported,
        /// registered hidden-by-default (manifest v6), never dropped.
        hidden: bool,
        /// The importer's own identity for this node (a `.skp` archive
        /// slot), so an [`ImportSceneRecipe`]'s hidden-node list can name
        /// it before the node exists. `None` when no scene refers to it.
        key: Option<u64>,
    },
    /// An instance of a shared definition (index into `ImportScene::defs`).
    Instance {
        /// Dense index into `ImportScene::defs`.
        def: usize,
        /// Pose (definition-local → world) for this placement.
        pose: Transform,
        /// The placement's OWN display name (`.skp` carries instance
        /// names natively — "Front Wall" on one placement of "Wall"). `None`
        /// falls back to the definition's name, as before.
        name: Option<String>,
        /// Tag paths for this instance node (from `__HEWMETA__` decode, WS2).
        tags: Vec<Vec<String>>,
        /// USER-hidden in the source (a `.skp` hidden component instance).
        hidden: bool,
        /// The importer's own identity for this node (see
        /// [`ImportNode::Group::key`]).
        key: Option<u64>,
    },
}

/// A saved view (a `.skp` scene) carried by an import: created by `ingest`
/// after the geometry, so its hidden sets can name the nodes and tags the
/// same import creates. Each `Option` is a captured property (`None` = the
/// scene does not capture it); a captured hidden set is a FULL set, as
/// `Scene` stores it.
pub struct ImportSceneRecipe {
    /// Display name; made unique against the document's existing scenes.
    pub name: String,
    pub description: String,
    pub camera: Option<CameraState>,
    /// Keys (see [`ImportNode::Group::key`]) of the nodes hidden in this
    /// scene. Keys that no imported node carries (source entities Hew has
    /// no node for, such as individual faces) are ignored.
    pub hidden_keys: Option<Vec<u64>>,
    /// Tag paths hidden in this scene (`.skp` layers), each declared in
    /// [`ImportScene::tags`].
    pub hidden_tags: Option<Vec<Vec<String>>>,
    /// The section plane active in this scene (world frame), when the
    /// scene captures one; `Some(None)` captures "no section cut".
    pub section: Option<Option<SectionPlaneState>>,
    /// The editor display toggles the scene captures, when it captures
    /// them (a `.skp` Hew wrote carries them).
    pub display: Option<DisplayState>,
    /// The look the scene captures (a `.skp` scene's saved style:
    /// backdrop and axes), when it captures one.
    pub view_style: Option<crate::style::ViewStyle>,
    /// The scene's own transition time in seconds; `None` is the
    /// document's own.
    pub transition_s: Option<f64>,
    /// The scene's own delay in seconds; `None` is the document's own.
    pub delay_s: Option<f64>,
}

/// A construction guide carried by an import (`.skp` guides).
/// Ingested inside the same `DocAction::Imported` step as the geometry, so
/// one undo removes the whole import, guides included.
pub enum ImportGuide {
    /// A construction line: a point on it + a direction, over a stretch.
    Line {
        /// A point the line passes through (meters, world space).
        origin: Point3,
        /// Line direction; normalized by `ingest` (degenerate → skipped, reported).
        direction: crate::math::Vec3,
        /// The stretch that exists, in meters along the NORMALIZED
        /// direction from `origin` ([`crate::GuideSpan::INFINITE`] for the
        /// whole line; one of no length → skipped, reported).
        span: crate::guide::GuideSpan,
        /// How the line is drawn (a `.skp` guide's stipple mask).
        dash: crate::guide::GuideDash,
        /// Tag paths, root-first (a `.skp` guide's layer, then each
        /// container's above it); the guide hides with them.
        tags: Vec<Vec<String>>,
    },
    /// A construction point.
    Point {
        /// Position (meters, world space).
        position: Point3,
        /// Tag paths, as for a line.
        tags: Vec<Vec<String>>,
    },
}

/// A tag the source document DECLARES (a `.skp` layer), independent of
/// whether any imported node carries it. Registered in the document's tag
/// metadata by `ingest` so the full source layer list survives import —
/// including empty layers — with its visibility. Content tagged with a
/// `hidden` tag imports normally and is hidden by default in the UI.
pub struct ImportTag {
    /// Root-first tag path (a `.skp` layer maps to a single segment).
    pub path: Vec<String>,
    /// Hidden in the source document → hidden by default in Hew.
    pub hidden: bool,
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
    /// Construction guides to add. Undone with the import.
    pub guides: Vec<ImportGuide>,
    /// The source document's declared tag list (`.skp` layers) with their
    /// visibility. Registered by `ingest`; unregistered on undo.
    pub tags: Vec<ImportTag>,
    /// The source document's saved view. Adopted as the working camera
    /// only when the import lands in an EMPTY document (a fresh open of a
    /// `.skp`); an import into existing work keeps the user's view.
    pub camera: Option<CameraState>,
    /// The source document's saved views (`.skp` scenes), created after
    /// the geometry in source order. Like palette materials, scenes are
    /// not removed by undoing the import.
    pub scenes: Vec<ImportSceneRecipe>,
    /// The section cut active in the source's saved view, adopted under
    /// the same empty-document rule as [`ImportScene::camera`].
    pub section: Option<SectionPlaneState>,
    /// How the source document is shown (`style.rs`), adopted under the
    /// same empty-document rule as [`ImportScene::camera`].
    pub view_style: Option<crate::style::ViewStyle>,
    /// The source document's coordinate reference system, adopted under
    /// the same empty-document rule as [`ImportScene::camera`]. A record
    /// that fails [`crate::GeoReference::validate`] is not adopted.
    pub geo: Option<crate::GeoReference>,
    /// Sketches and attribute dictionaries.
    pub extra: ImportSceneExtra,
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
    /// Drawing segments built: the edges of every sketch the import made
    /// (lines that bound no face in the source), so an open that brought
    /// in lines alone can say so.
    pub lines_added: usize,
    /// How many of the created objects are watertight.
    pub watertight: usize,
    /// How many of the created objects have open (non-watertight) shells —
    /// surfaces included ([`ImportReport::health`] tells them apart).
    pub leaky: usize,
    /// The created objects by what they are (`crate::health`): solids,
    /// surfaces, and the open ones that need attention, plus the seam
    /// families the importer's splits left. A definition member counts
    /// once, however many times it is placed.
    pub health: crate::health::HealthCounts,
    /// Meshes that `from_polygons_with_materials` rejected — reported but not
    /// repaired (DEVELOPMENT.md rule 4).
    pub skipped: Vec<SkippedMesh>,
    /// Image URIs that the importer could not resolve from the `ImageMap`
    /// (populated by `dae-import`, passed through `ingest` unchanged).
    pub textures_missing: Vec<String>,
    /// Importer keys -> the nodes built from them. One key can name several
    /// nodes: a shared definition's members are rebuilt per material
    /// variant of the definition.
    pub keys: BTreeMap<u64, Vec<NodeId>>,
    /// Each recipe definition's component, by dense index (`None` when
    /// every member was rejected).
    pub defs: Vec<Option<ComponentId>>,
    /// The sketches created, in [`ImportSceneExtra::sketches`] order
    /// (`None` for one that could not be built — reported in `skipped`).
    pub sketches: Vec<Option<crate::ids::SketchId>>,
    /// The annotations created, in [`ImportSceneExtra::annotations`] order
    /// (`None` for one that was skipped — reported in `skipped`).
    pub annotations: Vec<Option<crate::ids::AnnotationId>>,
    /// The stable id of each scene created, in [`ImportScene::scenes`]
    /// order (`None` for one that could not be added).
    pub scenes: Vec<Option<u64>>,
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
