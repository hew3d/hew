//! Hew geometry kernel.
//!
//! Half-edge mesh `Object`s with tracked watertightness, built strictly from
//! validated input — operations that would produce invalid topology fail with
//! a typed error; nothing is repaired silently.
//!
//! This crate is UI-free and I/O-free by rule (see docs/dev/DEVELOPMENT.md): no rendering,
//! filesystem, or network dependencies, ever. The WASM boundary lives in
//! `crates/wasm-api`.
//!
//! Module map: `math`, `tol`, `ids`, `topo`, `error`, `transform`,
//! construction (`build`), the validator (`validate`), and the fully
//! implemented `sketch`, `ops`, `history`, and `serialize`. Executable
//! specs live in `tests/op_specs.rs`.

mod boolean;
mod build;
mod geom2d;
mod retrace;
mod validate;

pub mod annotation;
pub mod attr;
pub mod axes;
pub mod camera;
pub mod changes;
pub mod document;
pub mod error;
pub mod geo;
pub mod guide;
pub mod health;
pub mod history;
pub mod ids;
pub mod import;
pub mod material;
pub mod math;
pub mod offset;
pub mod ops;
pub mod repair;
pub mod scenes;
pub mod serialize;
pub mod sketch;
pub mod style;
pub mod tol;
pub mod topo;
pub mod transform;
pub mod unit_key;

pub use annotation::{Anchor, Annotation, CapturedCurve, RadialKind};
pub use attr::{AttrDict, AttrValue, AttrValueError};
pub use axes::{AxesFrame, AxesFrameError};
pub use camera::{CameraProjection, CameraState};
pub use changes::{
    AttrChange, AttrScope, CHANGE_RING_CAP, ChangeKind, ChangeRecord, ChangeRef, ChangeSet,
};
pub use document::{
    AttrTarget, Axis, CellDecline, CellDeclineReason, CellOutcome, CellScope, CellSeed,
    ClosingEdge, CompoundMeta, CompoundOutcome, CompoundToken, DocChange, DocTransaction, Document,
    DocumentError, DraftCurveRun, DraftEdge, DraftRegion, DraftRegionWhy, DraftReport, DraftStore,
    DraftUnit, EntityRef, FollowMePath, HistoryArg, HistoryEntries, HistoryEntryInfo, HistoryKind,
    HistoryOrigin, IngestJob, IngestProgress, InsertOptions, InsertReport, LibraryProvenance,
    MAX_COMPONENT_DEPTH, MAX_OCCURRENCES, MaterialScope, NodeId, Occurrence, OpenEdges,
    PendingActionKind, PlaneListing, PlaneUnits, PurgeReport, Remap, RenderRoot, WRAPPED_DEF_SID,
};
pub use error::TopologyError;
pub use geo::{GeoReference, GeoReferenceError};
pub use guide::{Guide, GuideDash, GuideSpan};
pub use history::{History, HistoryEntry, HistoryError, KernelOp, KernelOpError, KernelOpReport};
pub use ids::{
    AnnotationId, ComponentId, EdgeId, FaceId, GroupId, GuideId, HalfEdgeId, InstanceId, LoopId,
    MaterialId, ObjectId, ShellId, SketchId, VertexId,
};
pub use import::{
    DefRecipe, ImportAttrTarget, ImportGuide, ImportNode, ImportReport, ImportScene,
    ImportSceneExtra, ImportSceneRecipe, ImportSketch, ImportTag, MeshExtra, MeshRecipe,
    SkippedMesh,
};
pub use material::{
    FaceMaterial, ImageFormat, Material, MaterialPalette, Pbr, PbrMap, Rgba8, Texture, UvFrame,
    material_content_hash,
};
pub use math::{MathError, Plane, Point3, Vec3};
pub use offset::{
    FaceOffsetError, OffsetError, OffsetLoop, ProfileOffset, offset_face_boundary, offset_profile,
};
pub use ops::{
    BooleanError, BooleanOp, CollapseSubFaceReport, ExtrudeError, FaceAttrsAt,
    FaceMergeInnerReport, FaceMergeReport, FaceSplitInnerReport, FaceSplitReport, FollowMeError,
    LoopImprintPlan, Operand, PushPullError, PushPullReport, SliceError, StickyError,
    TransformSubFaceReport,
};
pub use retrace::{FaceRetraceReport, RetracePath};
pub use scenes::{DisplayState, ResolvedScene, Scene, SceneDrift, SceneProps, SectionPlaneState};
pub use serialize::{
    DecodeError, GEOMETRY_FORMAT_VERSION, ItemSummary, LoadError, MANIFEST_FORMAT_VERSION,
    MaterialSummary, NO_MATERIAL, read_item_asset, read_item_summary,
    read_item_summary_from_reader,
};
pub use sketch::{
    CurveAnalytic, CurveGeom, CurveRefaceted, EdgeRemoved, MAX_CIRCLE_SEGMENTS,
    MIN_CIRCLE_SEGMENTS, Profile, ProfileError, RegionOffsetAdded, SegmentAdded, Sketch,
    SketchCurveId, SketchCurveKind, SketchCurveRim, SketchEdge, SketchEdgeId, SketchError,
    SketchIsland, SketchIslandId, SketchRegion, SketchRegionId, SketchVertex, SketchVertexId,
    VertexMoved,
};
pub use style::{SectionLook, ViewStyle};
pub use topo::{AnalyticRim, AttrSlot, FaceAttrs, Object, SurfaceRef, WatertightState};
pub use transform::{Transform, TransformError};
pub use unit_key::{SketchUnitKeys, chain_key, curve_key, edge_key, region_key};
