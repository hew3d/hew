//! Camera view state (docs/design/camera.md §5): the working view — which
//! projection is active, its field of view, and where the eye/target/up sit
//! — persisted with the document (manifest v13+) and restored on load.
//!
//! Deliberately NOT document history: changing your viewpoint is not a
//! model edit, so [`Document::set_camera_state`] never touches
//! `Document::undo`/`redo` — matching how SketchUp itself treats the
//! camera, and mirroring this crate's other non-undoable, still-persisted
//! view state (`Document`'s tag-visibility registry and user-hidden-node
//! sets; see `document.rs`).

use crate::math::{Point3, Vec3};

/// Which projection [`CameraState`] describes. Mirrors the app's
/// `CameraRig.projection` (`'perspective' | 'parallel'`) one-to-one; kept as
/// its own kernel-side enum rather than a raw string so a malformed
/// manifest value is a typed decode error (DEVELOPMENT.md rule 4), not a
/// silently-accepted default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraProjection {
    Perspective,
    Parallel,
}

/// The camera's working view, in world space. `fov_deg` only ever applies
/// under [`CameraProjection::Perspective`] (mirrors `CameraRig.perspective.fov`
/// persisting across projection toggles in the app) but is always present —
/// simpler than an `Option` that would need its own absent-under-Parallel
/// story, and harmless to carry a value the parallel path ignores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub projection: CameraProjection,
    /// Perspective vertical field of view, degrees. Meaningless (but still
    /// stored) under `Parallel`.
    pub fov_deg: f64,
    pub eye: Point3,
    pub target: Point3,
    pub up: Vec3,
}
