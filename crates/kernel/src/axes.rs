//! Document-level movable drawing axes (tool-parity design §4).
//!
//! SketchUp's Axes tool reorients drawing and inference away from the world
//! frame without moving any geometry: an [`AxesFrame`] is a pure coordinate
//! system, stored on the [`crate::document::Document`] and consumed wherever
//! "the model axes" used to mean the literal world X/Y/Z (axes rendering,
//! Move/Rotate's arrow-key locks, the active draw plane, inference's axis
//! snaps). The ground grid and Scale/Section/standard views stay
//! world-aligned (the design's explicit v1 scope).
//!
//! `x` and `y` are stored explicitly; the blue axis is always their cross
//! product ([`AxesFrame::z`]), never stored, so a live or persisted frame can
//! never itself disagree with its own blue axis — there is nothing to drift
//! out of sync.

use crate::math::{Point3, Vec3};
use crate::tol;

/// An orthonormal, right-handed coordinate frame that reorients drawing and
/// inference (tool-parity design §4). Default is world identity: origin at
/// [`Point3::ORIGIN`], `x` = +X, `y` = +Y (so `z()` = +Z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxesFrame {
    /// The frame's origin in world space.
    pub origin: Point3,
    /// Unit "red" axis direction, in world space.
    pub x: Vec3,
    /// Unit "green" axis direction, in world space. Perpendicular to `x`.
    pub y: Vec3,
}

/// Why a candidate [`AxesFrame`] was refused (DEVELOPMENT.md rule 4: no
/// silent repair — a bad candidate is rejected, never renormalized,
/// reoriented, or otherwise silently fixed up on the caller's behalf).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxesFrameError {
    /// A component of `origin`, `x`, or `y` (or, via
    /// [`AxesFrame::from_basis`], the explicit `z`) is NaN or infinite.
    NonFinite,
    /// The candidate basis vectors are not each unit length, or not
    /// mutually perpendicular.
    NonOrthonormal,
    /// The candidate basis is orthonormal but left-handed. Unreachable
    /// through the common two-vector [`AxesFrame::new`] constructor — its
    /// `z` is derived as `x × y`, which is right-handed by construction —
    /// so this only fires through [`AxesFrame::from_basis`], the defensive
    /// three-vector constructor used to validate an externally supplied
    /// full basis (a hand-edited or corrupted file, say) without trusting
    /// its author's cross product.
    LeftHanded,
}

impl std::fmt::Display for AxesFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AxesFrameError::NonFinite => {
                write!(f, "axes frame has a non-finite coordinate or direction")
            }
            AxesFrameError::NonOrthonormal => write!(
                f,
                "axes frame's x/y directions are not unit length and mutually perpendicular"
            ),
            AxesFrameError::LeftHanded => {
                write!(f, "axes frame basis is orthonormal but left-handed")
            }
        }
    }
}

impl std::error::Error for AxesFrameError {}

impl AxesFrame {
    /// World identity: origin at [`Point3::ORIGIN`], `x` = +X, `y` = +Y.
    pub const IDENTITY: AxesFrame = AxesFrame {
        origin: Point3::ORIGIN,
        x: Vec3::new(1.0, 0.0, 0.0),
        y: Vec3::new(0.0, 1.0, 0.0),
    };

    /// Builds a frame from an origin and two directions, deriving the third
    /// (`z = x × y`) rather than accepting it from the caller. Because the
    /// derived `z` is always right-handed relative to `x`/`y`, this
    /// constructor can only ever fail [`AxesFrameError::NonFinite`] or
    /// [`AxesFrameError::NonOrthonormal`] — never `LeftHanded` (see
    /// [`AxesFrame::from_basis`], which can).
    ///
    /// This is the shape the manifest persists (`origin`, `x`, `y` —
    /// HEW_FILE_FORMAT.md) and the shape the Axes tool's two-direction click
    /// gesture naturally produces.
    pub fn new(origin: Point3, x: Vec3, y: Vec3) -> Result<AxesFrame, AxesFrameError> {
        let z = x.cross(y);
        Self::from_basis(origin, x, y, z)?;
        Ok(AxesFrame { origin, x, y })
    }

    /// Builds a frame from an origin and an EXPLICIT full basis, validating
    /// `x`, `y`, and `z` independently rather than trusting the caller's own
    /// cross product — the defensive constructor for a boundary that hands
    /// over all three directions (a file's hand-edited or corrupted basis,
    /// or a future caller that computes `z` itself).
    ///
    /// # Errors
    /// - [`AxesFrameError::NonFinite`] — any coordinate/component is NaN or
    ///   infinite.
    /// - [`AxesFrameError::NonOrthonormal`] — `x`, `y`, `z` are not each unit
    ///   length ([`tol::AXES_ORTHONORMAL`]), or not mutually perpendicular.
    /// - [`AxesFrameError::LeftHanded`] — the basis IS orthonormal but
    ///   `z ≈ -(x × y)` (determinant −1) instead of `z ≈ x × y`
    ///   (determinant +1).
    pub fn from_basis(
        origin: Point3,
        x: Vec3,
        y: Vec3,
        z: Vec3,
    ) -> Result<AxesFrame, AxesFrameError> {
        let components = [
            origin.x, origin.y, origin.z, x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z,
        ];
        if !components.into_iter().all(f64::is_finite) {
            return Err(AxesFrameError::NonFinite);
        }

        let is_unit = |v: Vec3| (v.length() - 1.0).abs() <= tol::AXES_ORTHONORMAL;
        if !is_unit(x) || !is_unit(y) || !is_unit(z) {
            return Err(AxesFrameError::NonOrthonormal);
        }
        if x.dot(y).abs() > tol::AXES_ORTHONORMAL
            || x.dot(z).abs() > tol::AXES_ORTHONORMAL
            || y.dot(z).abs() > tol::AXES_ORTHONORMAL
        {
            return Err(AxesFrameError::NonOrthonormal);
        }

        // Orthonormal triads have exactly two possible third vectors given
        // the first two: ±(x × y). The determinant picks the sign; +1 is
        // right-handed, -1 is left-handed.
        let det = x.dot(y.cross(z));
        if det < 0.0 {
            return Err(AxesFrameError::LeftHanded);
        }

        Ok(AxesFrame { origin, x, y })
    }

    /// The derived "blue" axis: `x × y`, always unit length and
    /// perpendicular to both when the frame was built through [`AxesFrame::new`]
    /// or [`AxesFrame::from_basis`] (the only ways to construct one).
    pub fn z(&self) -> Vec3 {
        self.x.cross(self.y)
    }
}

impl Default for AxesFrame {
    fn default() -> AxesFrame {
        AxesFrame::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    #[test]
    fn identity_is_valid_and_matches_world() {
        let f = AxesFrame::IDENTITY;
        assert_eq!(f.origin, Point3::ORIGIN);
        assert_eq!(f.x, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(f.y, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(f.z(), Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn new_accepts_an_arbitrary_orthonormal_pair() {
        // A frame rotated 90° about Z: red along +Y, green along -X.
        let x = Vec3::new(0.0, 1.0, 0.0);
        let y = Vec3::new(-1.0, 0.0, 0.0);
        let f = AxesFrame::new(Point3::new(1.0, 2.0, 3.0), x, y).expect("orthonormal pair");
        assert!((f.z() - Vec3::new(0.0, 0.0, 1.0)).length() < EPS);
    }

    #[test]
    fn new_refuses_non_finite() {
        let x = Vec3::new(f64::NAN, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(
            AxesFrame::new(Point3::ORIGIN, x, y),
            Err(AxesFrameError::NonFinite)
        );
        let bad_origin = Point3::new(f64::INFINITY, 0.0, 0.0);
        assert_eq!(
            AxesFrame::new(
                bad_origin,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0)
            ),
            Err(AxesFrameError::NonFinite)
        );
    }

    #[test]
    fn new_refuses_non_unit_length() {
        let x = Vec3::new(2.0, 0.0, 0.0); // not unit
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(
            AxesFrame::new(Point3::ORIGIN, x, y),
            Err(AxesFrameError::NonOrthonormal)
        );
    }

    #[test]
    fn new_refuses_non_perpendicular() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.6, 0.8, 0.0); // unit length, but not perpendicular to X
        assert_eq!(
            AxesFrame::new(Point3::ORIGIN, x, y),
            Err(AxesFrameError::NonOrthonormal)
        );
    }

    #[test]
    fn new_never_produces_left_handed() {
        // Sweep several orthonormal (x, y) pairs — z = x × y is always
        // right-handed by construction, so `from_basis`'s handedness check
        // (exercised directly below) can never fire through `new`.
        let pairs = [
            (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
            (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
            (Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)),
        ];
        for (x, y) in pairs {
            assert!(AxesFrame::new(Point3::ORIGIN, x, y).is_ok());
        }
    }

    #[test]
    fn from_basis_refuses_left_handed() {
        // x, y, z each unit and mutually perpendicular, but z is the NEGATION
        // of x × y = +Z: a valid orthonormal triad, wrong-handed.
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        let z = Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(
            AxesFrame::from_basis(Point3::ORIGIN, x, y, z),
            Err(AxesFrameError::LeftHanded)
        );
    }

    #[test]
    fn from_basis_accepts_the_matching_right_handed_z() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        let f = AxesFrame::from_basis(Point3::ORIGIN, x, y, z).expect("right-handed");
        assert_eq!(f.x, x);
        assert_eq!(f.y, y);
    }

    #[test]
    fn default_is_identity() {
        assert_eq!(AxesFrame::default(), AxesFrame::IDENTITY);
    }
}
