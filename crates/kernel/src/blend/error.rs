//! The blend engine's typed refusals.

use crate::math::Point3;
use serde::Serialize;

/// What stopped a blend from growing: names the feature for the status bar
/// ("Radius limited to 14 mm by the adjacent face").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LimitFeature {
    /// "by the adjacent face"
    AdjacentFace,
    /// "by the neighboring edge"
    NeighborEdge,
    /// "by the hole in this face"
    Hole,
    /// "by the shape drawn on this face"
    Imprint,
    /// "by the blend across the face"
    OppositeBlend,
    /// "by the length of this edge"
    ShortEdge,
    /// "by the rim's radius"
    RimRadius,
    /// "by the facet width of the curved wall"
    FacetWidth,
    /// "by other geometry" (the obstruction guard)
    OtherGeometry,
    /// never shown alone: "by the shape of the model"
    Unknown,
}

impl LimitFeature {
    /// The snake_case name the wasm and Hew API boundaries report.
    pub fn name(self) -> &'static str {
        match self {
            LimitFeature::AdjacentFace => "adjacent_face",
            LimitFeature::NeighborEdge => "neighbor_edge",
            LimitFeature::Hole => "hole",
            LimitFeature::Imprint => "imprint",
            LimitFeature::OppositeBlend => "opposite_blend",
            LimitFeature::ShortEdge => "short_edge",
            LimitFeature::RimRadius => "rim_radius",
            LimitFeature::FacetWidth => "facet_width",
            LimitFeature::OtherGeometry => "other_geometry",
            LimitFeature::Unknown => "unknown",
        }
    }
}

/// Why a blend refused. Every refusal leaves the object unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlendError {
    /// A spec edge is not an edge of the object.
    UnknownEdge,
    /// The spec names no edges.
    EmptySpec,
    /// The object is not a watertight solid.
    BlendNeedsSolid,
    /// A spec edge is already smooth (a seam, a tangent joint, coplanar).
    EdgeIsSmooth,
    /// The edge's neighborhood is outside the blend catalogue and the swept
    /// blend cannot take the spec there (it rolls rounds only, at one size,
    /// without stops), or the profile cannot go on this kind of edge.
    UnsupportedEdge,
    /// A vertex configuration the engine cannot blend.
    UnsupportedVertex {
        /// The vertex.
        at: Point3,
    },
    /// The edge's two faces meet tangentially at `at` (two pipes of one
    /// radius in a tee: their walls run flush past the branch's sides), so
    /// no ball fits between them there at any size (D-234).
    BlendFacesFlush {
        /// Where the faces touch flush.
        at: Point3,
    },
    /// The size is larger than fits; `max` is the largest that does (meters).
    RadiusTooLarge {
        /// The largest size that fits.
        max: f64,
        /// What limits it.
        limited_by: LimitFeature,
    },
    /// A profile needs more face than there is.
    ProfileTooWide {
        /// Width the profile needs (meters).
        needed: f64,
        /// Width available (meters).
        available: f64,
    },
    /// A cove needs a convex edge.
    ProfileNeedsConvexEdge,
    /// A bullnose needs two parallel edges bounding one face.
    BullnoseNeedsParallelSides,
    /// The profile shape is not valid.
    InvalidProfile,
    /// A stop or radius handle lies outside its chain.
    StopOutOfRange,
    /// The blend would cut into other geometry.
    Obstructed {
        /// A point on the obstructed blend face.
        at: Point3,
    },
    /// Facet count outside 2..=96.
    SegmentsOutOfRange {
        /// The requested count.
        segments: u32,
    },
    /// An instance whose pose is not a similarity.
    NonUniformInstance,
    /// The blend was changed since it was made and is ordinary geometry now.
    BlendModified,
    /// A later blend that runs into this one cannot be put back after the
    /// edit (its edges are gone, or its size no longer fits there).
    DependentBlend,
    /// The face is not part of a blend.
    UnknownBlend,
    /// The plan produced invalid topology at a size the checks accepted (a
    /// kernel bug channel).
    BlendDegenerate,
}

impl std::fmt::Display for BlendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlendError::UnknownEdge => write!(f, "unknown edge"),
            BlendError::EmptySpec => write!(f, "no edges to blend"),
            BlendError::BlendNeedsSolid => write!(f, "the object is not a watertight solid"),
            BlendError::EdgeIsSmooth => write!(f, "that edge is already smooth"),
            BlendError::UnsupportedEdge => write!(f, "this edge cannot be blended"),
            BlendError::UnsupportedVertex { at } => write!(f, "cannot blend the corner at {at:?}"),
            BlendError::BlendFacesFlush { at } => {
                write!(
                    f,
                    "the faces meet flush at {at:?}, leaving no room for a blend"
                )
            }
            BlendError::RadiusTooLarge { max, limited_by } => {
                write!(f, "radius limited to {max} by {}", limited_by.name())
            }
            BlendError::ProfileTooWide { needed, available } => {
                write!(f, "the profile needs {needed}; {available} is available")
            }
            BlendError::ProfileNeedsConvexEdge => write!(f, "a cove needs a convex edge"),
            BlendError::BullnoseNeedsParallelSides => {
                write!(f, "a bullnose needs two parallel edges of one face")
            }
            BlendError::InvalidProfile => write!(f, "invalid profile"),
            BlendError::StopOutOfRange => write!(f, "a stop is outside the edge"),
            BlendError::Obstructed { at } => {
                write!(f, "the blend would cut into geometry at {at:?}")
            }
            BlendError::SegmentsOutOfRange { segments } => {
                write!(f, "{segments} segments is outside 2..=96")
            }
            BlendError::NonUniformInstance => write!(f, "the instance is not uniformly scaled"),
            BlendError::BlendModified => write!(f, "the blend has been modified"),
            BlendError::DependentBlend => {
                write!(
                    f,
                    "a blend that runs into this one cannot be rebuilt after the edit"
                )
            }
            BlendError::UnknownBlend => write!(f, "not a blend"),
            BlendError::BlendDegenerate => write!(f, "the blend produced invalid topology"),
        }
    }
}

impl std::error::Error for BlendError {}

// `DocumentError` is `Eq`; every f64 payload is finite by construction.
impl Eq for BlendError {}
