//! Hew geometry kernel.
//!
//! Half-edge mesh `Object`s with tracked watertightness, built strictly from
//! validated input — operations that would produce invalid topology fail with
//! a typed error; nothing is repaired silently.
//!
//! This crate is UI-free and I/O-free by rule (see docs/DEVELOPMENT.md): no rendering,
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

pub mod document;
pub mod error;
pub mod history;
pub mod ids;
pub mod math;
pub mod ops;
pub mod serialize;
pub mod sketch;
pub mod tol;
pub mod topo;
pub mod transform;

pub use document::{DocChange, Document, DocumentError};
pub use error::TopologyError;
pub use history::{History, HistoryEntry, HistoryError, KernelOp, KernelOpError, KernelOpReport};
pub use ids::{EdgeId, FaceId, HalfEdgeId, LoopId, ObjectId, ShellId, SketchId, VertexId};
pub use math::{MathError, Plane, Point3, Vec3};
pub use ops::{
    BooleanError, BooleanOp, ExtrudeError, FaceMergeReport, FaceSplitReport, Operand,
    PushPullError, PushPullReport, StickyError,
};
pub use serialize::{DecodeError, GEOMETRY_FORMAT_VERSION};
pub use sketch::{
    EdgeRemoved, Profile, ProfileError, SegmentAdded, Sketch, SketchEdge, SketchEdgeId,
    SketchError, SketchRegion, SketchRegionId, SketchVertex, SketchVertexId,
};
pub use topo::{Object, WatertightState};
pub use transform::{Transform, TransformError};
