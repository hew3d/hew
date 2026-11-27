//! The typed refusals shared by the shape operations of the true-curves
//! effort: Deform, Loft, Shell, Project, Revolve, the path array, Soften,
//! Bake, creases and the import repairs.

use crate::math::Point3;

/// Why a shape operation refused. Every refusal leaves the document
/// unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeError {
    /// The result would pass through itself. `limit`: the largest amount that
    /// works, in the operation's own unit (radians for angles, a ratio for
    /// scales, meters for lengths), when known.
    SelfIntersecting {
        /// The largest amount that works, when known.
        limit: Option<f64>,
    },
    /// Shell: a wall this thick leaves no room. `at`: a point on the face.
    WallTooThick {
        /// A point on the face where the wall runs out of room.
        at: Point3,
    },
    /// Loft: a profile is not closed.
    OpenProfile,
    /// Loft: a profile has holes.
    LoftProfileHasHole,
    /// Loft: fewer than two profiles (an apex counts as one).
    LoftNeedsTwoProfiles,
    /// Loft: more than two rails.
    LoftTooManyRails,
    /// Loft: a rail does not touch every profile at a corner or point of
    /// it, or the points it touches cannot pair (a second rail lining the
    /// profiles up otherwise than the first).
    RailMissesProfile,
    /// Revolve: the axis does not lie in the profile's plane.
    AxisNotInProfilePlane,
    /// Project: the shape lands on no face of the target.
    ProjectMisses,
    /// Project: the source is not closed regions.
    ProjectNeedsClosedShape,
    /// Project: a straight depth would cut the object into pieces.
    WouldSever,
    /// Shell: a vertex where the offset planes do not meet in one point.
    UnsupportedShellCorner {
        /// The vertex.
        at: Point3,
    },
    /// The object must be a watertight solid.
    ShapeNeedsSolid,
    /// A typed amount is outside its allowed range.
    AmountOutOfRange,
    /// Smooth: the cage would subdivide to more faces than the surface may
    /// hold at that level.
    SmoothTooDense {
        /// Faces the surface would have at that level.
        faces: usize,
        /// The most it may have.
        cap: usize,
    },
    /// Path array: the path is too short for the count.
    PathTooShortForCount,
    /// Crease: the edge is a soft seam (soft seams never crease).
    CreaseOnSoftEdge,
    /// Crease: the edge is a boundary edge of an open object.
    CreaseOnBoundary,
    /// A live preview gave up: building it would take more work than a
    /// preview may (Deform's `PREVIEW_WORK_CAP`). Only previews raise it;
    /// the operation itself has no cap and can still be applied.
    PreviewTooLarge,
    /// Deform: a part of the solid is too thin for the map at this size
    /// (the cuts would drop a visible part of its surface).
    TooThinToDeform,
    /// The operation produced invalid topology (a kernel bug channel).
    ShapeDegenerate,
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeError::SelfIntersecting { limit: Some(l) } => {
                write!(f, "the result would pass through itself (limit {l})")
            }
            ShapeError::SelfIntersecting { limit: None } => {
                write!(f, "the result would pass through itself")
            }
            ShapeError::WallTooThick { at } => write!(f, "the wall is too thick at {at:?}"),
            ShapeError::OpenProfile => write!(f, "a profile is not closed"),
            ShapeError::LoftProfileHasHole => write!(f, "a loft profile has a hole"),
            ShapeError::LoftNeedsTwoProfiles => write!(f, "a loft needs two profiles"),
            ShapeError::LoftTooManyRails => write!(f, "a loft takes at most two rails"),
            ShapeError::RailMissesProfile => {
                write!(f, "a rail does not join matching points of every profile")
            }
            ShapeError::AxisNotInProfilePlane => {
                write!(f, "the axis does not lie in the profile's plane")
            }
            ShapeError::ProjectMisses => write!(f, "the shape lands on no face"),
            ShapeError::ProjectNeedsClosedShape => write!(f, "project needs a closed shape"),
            ShapeError::WouldSever => write!(f, "the depth would cut the object apart"),
            ShapeError::UnsupportedShellCorner { at } => {
                write!(f, "the offset planes do not meet in one point at {at:?}")
            }
            ShapeError::ShapeNeedsSolid => write!(f, "the object is not a watertight solid"),
            ShapeError::AmountOutOfRange => write!(f, "the amount is out of range"),
            ShapeError::SmoothTooDense { faces, cap } => {
                write!(
                    f,
                    "the cage is too dense for that level ({faces} faces, at most {cap})"
                )
            }
            ShapeError::PathTooShortForCount => write!(f, "the path is too short"),
            ShapeError::CreaseOnSoftEdge => write!(f, "a soft seam cannot be creased"),
            ShapeError::CreaseOnBoundary => write!(f, "a boundary edge cannot be creased"),
            ShapeError::PreviewTooLarge => write!(f, "the object is too large to preview"),
            ShapeError::TooThinToDeform => {
                write!(f, "a part of the solid is too thin to deform at this size")
            }
            ShapeError::ShapeDegenerate => write!(f, "the operation produced invalid topology"),
        }
    }
}

impl std::error::Error for ShapeError {}

// `DocumentError` is `Eq`. The f64 payloads (limits, positions) are finite
// by construction — every refusal computes them from validated geometry —
// so equality is reflexive on every value a refusal carries.
impl Eq for ShapeError {}
