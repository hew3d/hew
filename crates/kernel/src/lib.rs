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
//! Module map: implemented — `math`, `tol`, `ids`, `topo`, `error`,
//! `transform`, plus construction (`build`) and the validator (`validate`).
//! Contract stubs with `todo!()` bodies pending M1–M3 — `sketch`, `ops`,
//! `history`, `serialize`; their executable specs live in
//! `tests/op_specs.rs`.

mod boolean;
mod build;
mod geom2d;
mod validate;

pub mod annotation;
pub mod attr;
pub mod axes;
pub mod camera;
pub mod document;
pub mod error;
pub mod guide;
pub mod history;
pub mod ids;
pub mod import;
pub mod material;
pub mod math;
pub mod offset;
pub mod ops;
pub mod scenes;
pub mod serialize;
pub mod sketch;
pub mod tol;
pub mod topo;
pub mod transform;

pub use annotation::{Anchor, Annotation, CapturedCurve, RadialKind};
pub use attr::{AttrDict, AttrValue, AttrValueError};
pub use axes::{AxesFrame, AxesFrameError};
pub use camera::{CameraProjection, CameraState};
pub use document::{
    AttrTarget, CompoundMeta, DocChange, DocTransaction, Document, DocumentError, EntityRef,
    FollowMePath, HistoryOrigin, InsertOptions, InsertReport, LibraryProvenance,
    MAX_COMPONENT_DEPTH, MaterialScope, NodeId, PendingActionKind,
};
pub use error::TopologyError;
pub use guide::Guide;
pub use history::{History, HistoryEntry, HistoryError, KernelOp, KernelOpError, KernelOpReport};
pub use ids::{
    AnnotationId, ComponentId, EdgeId, FaceId, GroupId, GuideId, HalfEdgeId, InstanceId, LoopId,
    MaterialId, ObjectId, ShellId, SketchId, VertexId,
};
pub use import::{
    DefRecipe, ImportGuide, ImportNode, ImportReport, ImportScene, ImportTag, MeshRecipe,
    SkippedMesh,
};
pub use material::{
    FaceMaterial, ImageFormat, Material, MaterialPalette, Rgba8, Texture, UvFrame,
    material_content_hash,
};
pub use math::{MathError, Plane, Point3, Vec3};
pub use offset::{
    FaceOffsetError, OffsetError, OffsetLoop, ProfileOffset, offset_face_boundary, offset_profile,
};
pub use ops::{
    BooleanError, BooleanOp, CollapseSubFaceReport, ExtrudeError, FaceAttrsAt,
    FaceMergeInnerReport, FaceMergeReport, FaceSplitInnerReport, FaceSplitReport, FollowMeError,
    Operand, PushPullError, PushPullReport, SliceError, StickyError,
};
pub use scenes::{DisplayState, ResolvedScene, Scene, SceneDrift, SceneProps, SectionPlaneState};
pub use serialize::{
    DecodeError, GEOMETRY_FORMAT_VERSION, ItemSummary, LoadError, MANIFEST_FORMAT_VERSION,
    MaterialSummary, NO_MATERIAL, read_item_asset, read_item_summary,
};
pub use sketch::{
    CurveAnalytic, CurveGeom, CurveRefaceted, EdgeRemoved, MAX_CIRCLE_SEGMENTS,
    MIN_CIRCLE_SEGMENTS, Profile, ProfileError, RegionOffsetAdded, SegmentAdded, Sketch,
    SketchCurveId, SketchCurveKind, SketchCurveRim, SketchEdge, SketchEdgeId, SketchError,
    SketchIsland, SketchIslandId, SketchRegion, SketchRegionId, SketchVertex, SketchVertexId,
};
pub use topo::{AnalyticRim, FaceAttrs, Object, SurfaceRef, WatertightState};
pub use transform::{Transform, TransformError};
