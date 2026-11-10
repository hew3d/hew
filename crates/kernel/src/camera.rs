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

/// The camera's working view, in world space. Both projections' own
/// parameters are always present: `fov_deg` applies under
/// [`CameraProjection::Perspective`] and `parallel_height` under
/// [`CameraProjection::Parallel`], and each is kept under the other so a
/// projection toggle lands where the view was (mirrors the app's
/// `CameraRig`, which keeps both cameras posed) — simpler than `Option`s
/// that would each need an absent-under-the-other story.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub projection: CameraProjection,
    /// Perspective vertical field of view, degrees. Meaningless (but still
    /// stored) under `Parallel`.
    pub fov_deg: f64,
    /// The height the view spans at `target`, in meters: the parallel
    /// frustum's height under `Parallel`, independent of how far `eye`
    /// sits from `target` (a SketchUp parallel view keeps exactly this).
    /// Under `Perspective` it is what the view spans at `target` with
    /// `fov_deg` from `eye` (see [`CameraState::height_at_target`]), so a
    /// toggle to parallel keeps the framing. Finite and positive.
    pub parallel_height: f64,
    pub eye: Point3,
    pub target: Point3,
    pub up: Vec3,
    /// The width of the view in meters that a camera set to an image's
    /// width holds (a `.skp` Match Photo camera's); `None` when unset.
    /// Carried and saved only: no navigation reads it, and a camera the
    /// user moves replaces it with `None` ([`Document::set_camera_state`]).
    pub image_width: Option<f64>,
    /// The width-to-height ratio the view is held to (a `.skp` Match Photo
    /// camera's image ratio); `None` when the view follows the window.
    /// Carried like [`CameraState::image_width`].
    pub aspect_ratio: Option<f64>,
}

impl CameraState {
    /// The height a perspective view of `fov_deg` spans at `target` from
    /// `eye`: `2 · |eye − target| · tan(fov / 2)`. What `parallel_height`
    /// takes when nothing else fixes it (a camera from before the field
    /// existed, or one given by eye, target and fov alone).
    pub fn height_at_target(eye: Point3, target: Point3, fov_deg: f64) -> f64 {
        let dist = (eye - target).length();
        CameraState::usable_height(2.0 * dist * (fov_deg.to_radians() / 2.0).tan())
    }

    /// `height` when it is a height a view can span (finite and positive),
    /// else [`CameraState::DEFAULT_PARALLEL_HEIGHT`]: an eye on its target
    /// or a field of view of 180° or more spans nothing, and a camera that
    /// carries that could be saved but never loaded (`serialize.rs` rejects
    /// a non-positive height).
    pub fn usable_height(height: f64) -> f64 {
        if height.is_finite() && height > 0.0 {
            height
        } else {
            CameraState::DEFAULT_PARALLEL_HEIGHT
        }
    }

    /// The parallel height a degenerate camera takes, in meters: what a
    /// new SketchUp model's parallel view spans (133 inches).
    pub const DEFAULT_PARALLEL_HEIGHT: f64 = 3.3782;

    /// This camera with a parallel height a view can span (see
    /// [`CameraState::usable_height`]); every path that stores a camera
    /// goes through it, so a saved document always loads.
    pub fn usable(mut self) -> CameraState {
        self.parallel_height = CameraState::usable_height(self.parallel_height);
        self
    }

    /// A camera from eye, target, up, projection and fov, with the parallel
    /// height the perspective view spans at the target.
    pub fn framed(
        projection: CameraProjection,
        fov_deg: f64,
        eye: Point3,
        target: Point3,
        up: Vec3,
    ) -> CameraState {
        CameraState {
            projection,
            fov_deg,
            parallel_height: CameraState::height_at_target(eye, target, fov_deg),
            eye,
            target,
            up,
            image_width: None,
            aspect_ratio: None,
        }
    }

    /// Whether `other` looks the same way: the same projection, field of
    /// view, framing, eye, target and up, within the merge tolerance
    /// (`image_width` and `aspect_ratio` aside).
    pub fn same_view(&self, other: &CameraState) -> bool {
        let tol = crate::tol::POINT_MERGE;
        let near = |a: f64, b: f64| (a - b).abs() <= tol;
        self.projection == other.projection
            && near(self.fov_deg, other.fov_deg)
            && near(self.parallel_height, other.parallel_height)
            && (self.eye - other.eye).length() <= tol
            && (self.target - other.target).length() <= tol
            && (self.up - other.up).length() <= tol
    }
}
