//! The draft context: 2D sticky geometry on a plane, before any solid exists.
//!
//! A [`Sketch`] is where SketchUp-style drawing happens *outside* an Object
//! (ARCHITECTURE.md): line/rectangle tools feed it segments, sticky rules apply
//! (endpoints merge, crossings split), and closed circuits become
//! [`SketchRegion`]s. A region converts to a [`Profile`] — a validated closed
//! planar polygon with holes — which is the input to
//! `Object::from_extrusion`: extruding a Profile is THE way solids are born
//! (solids-by-default, docs/dev/DEVELOPMENT.md).
//!
//! Sketches are deliberately not Objects: an Object models a solid with a
//! watertightness contract; a sketch is scaffolding. Drawing on an existing
//! Object's face goes through `Object::split_face` instead (see `ops`).
//!
//! # Sticky rules (the contract `add_segment` implements)
//!
//! Within one sketch — never across sketches or Objects:
//! 1. A new endpoint within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of an existing sketch vertex
//!    merges with it.
//! 2. A new endpoint within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of an existing edge's
//!    interior splits that edge (T-junction).
//! 3. A proper crossing between the new segment and an existing edge splits
//!    both at the intersection point.
//! 4. Collinear overlap between the new segment and an existing edge merges
//!    into non-overlapping edges (no duplicate or partially-stacked edges
//!    survive).
//! 5. After insertion, the set of closed regions is recomputed: every minimal
//!    closed circuit of edges bounds exactly one region; a circuit strictly
//!    inside another region's boundary makes a hole in it.
//! 6. A region that a drawn segment brings about has a face. A region can
//!    also be a loop of edges with no face ([`SketchRegion::faceless`]):
//!    what SketchUp has where a face was erased from its edges, or where
//!    edges alone were drawn — a wire frame, a light rig. Such a loop is
//!    not filled, picked or extruded. Drawing over one of its edges gives
//!    it its face, as in SketchUp; a region unchanged by an insertion
//!    keeps its state, and one the insertion reshapes (split by a new
//!    edge) is new, with a face.
//!
//! The executable contract lives in `crates/kernel/tests/op_specs.rs`.

use slotmap::{SlotMap, new_key_type};

use crate::geom2d::{
    boundaries_contact, interior_point_of_loops, plane_axes, point_inside_polygon,
    polygon_is_simple, signed_area_on_plane,
};
use crate::math::{Plane, Point3, Vec3};
use crate::tol;

new_key_type! {
    /// Handle to a [`SketchVertex`].
    pub struct SketchVertexId;
    /// Handle to a [`SketchEdge`].
    pub struct SketchEdgeId;
    /// Handle to a [`SketchRegion`].
    pub struct SketchRegionId;
    /// Handle to a curve chain: the edges committed as ONE drawn curve (an
    /// arc or circle's facets), selected and deleted as a unit.
    pub struct SketchCurveId;
    /// Handle to an [`SketchIsland`].
    pub struct SketchIslandId;
}

/// The fewest facets a closed chord ring may have and still count as a
/// "circle" for analytic stamping — the density floor below which a ring is
/// a coarse polygon of secants, not a curve. Coupled to the draw tools'
/// segments-per-turn floor (24, the true-curves design §6: "24 becomes
/// the floor, adaptive by radius up to 96"), so every genuine tool-produced
/// circle clears it while a triangle/hexagon/skip-connected coarse ring does
/// not. The kernel legitimately owns "what density counts as a circle": an
/// analytic claim on a coarser ring would sweep a secant into a cylinder
/// wall, and stamp-wrong is worse than don't-stamp (map-or-drop).
pub const MIN_CIRCLE_SEGMENTS: usize = 24;

/// The most facets [`Sketch::refacet_curve`] will build a circle from.
///
/// Not a geometric tolerance — a policy bound on a user-typed count. Past a
/// few hundred facets the chords are far below any display or manufacturing
/// resolution while the edge, vertex and region-tracing cost keeps growing,
/// so an accidental extra digit would hang the app rather than draw anything
/// a user could see. Refused typed ([`SketchError::SegmentsAboveCap`])
/// rather than clamped, because silently building a different circle than the
/// one asked for is the kind of quiet substitution rule 4 exists to prevent.
pub const MAX_CIRCLE_SEGMENTS: usize = 1024;

/// The analytic definition a curve chain was drawn from: the exact circle
/// (or circular arc) whose facets the chain's edges are. The circle lies in
/// the sketch plane; arc extent is derived from the chain's member edges,
/// never stored. This is the durable form of what the drawing tool computed
/// and, before this existed, immediately discarded — the foundation the
/// true-curves plan builds on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveGeom {
    /// Circle center, on the sketch plane (within
    /// [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)).
    pub center: Point3,
    /// Circle radius in meters, > [`tol::POINT_MERGE`](crate::tol::POINT_MERGE).
    pub radius: f64,
}

impl CurveGeom {
    /// The side count `n` of the regular polygon this circle circumscribes
    /// whose side lies along `a`→`b`, or `None` if no regular polygon's side
    /// does: the segment must lie inside the circle on a line at the
    /// polygon's apothem `radius·cos(π/n)` from the center, for a whole
    /// `n ≥ 3`. A piece of a side (split where something crossed it) passes;
    /// a side whose corner was dragged off the circle does not.
    pub fn polygon_side_count(&self, a: Point3, b: Point3) -> Option<u32> {
        let r = self.radius;
        if !(r.is_finite() && r > crate::tol::POINT_MERGE) {
            return None;
        }
        let tol = crate::tol::PLANE_DIST.max(r * 1e-9);
        if (a - self.center).length() > r + tol || (b - self.center).length() > r + tol {
            return None;
        }
        let d = (b - a).normalized().ok()?;
        let f = self.center - a;
        let apothem = (f - d * f.dot(d)).length();
        let ratio = (apothem / r).clamp(-1.0, 1.0);
        let n = (std::f64::consts::PI / ratio.acos()).round();
        if !(3.0..=100_000.0).contains(&n) {
            return None;
        }
        ((apothem - r * (std::f64::consts::PI / n).cos()).abs() <= tol).then_some(n as u32)
    }
}

/// What a chain's [`CurveGeom`] actually *claims* — the difference between a
/// circle a chain approximates and a polygon a chain simply is.
///
/// A faceted circle's chords are an approximation: the analytic circle is the
/// truth and the facets are an artifact, so the circle's quadrants, tangents
/// and smooth-shaded sweeps are all legitimate. A regular polygon's sides are
/// the geometry itself; the only thing its circumcircle legitimately supplies
/// is the **center** the user placed. Keeping the two apart is what lets a
/// polygon have a center without acquiring quadrant points that lie on no
/// edge, concentric-arc offsets, or a cylindrical wall on extrusion — the
/// same "stamp-wrong is worse than don't-stamp" judgment
/// [`MIN_CIRCLE_SEGMENTS`] makes for coarse rings, made once, explicitly,
/// where the drawing tool already knows the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchCurveKind {
    /// The chain's edges are chord facets approximating the exact circle.
    /// Every analytic consumer applies: rims (center, quadrants, tangents),
    /// concentric offset, and the analytic surface an extrusion sweeps.
    Circle,
    /// The chain's edges ARE the geometry — a regular polygon, drawn from a
    /// center and a circumradius. Only the center is offered analytically.
    Polygon,
}

/// A curve chain's analytic record: the circle it was drawn from, plus what
/// that circle claims about the chain ([`SketchCurveKind`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveAnalytic {
    /// The exact circle: for [`SketchCurveKind::Circle`] the curve the facets
    /// approximate, for [`SketchCurveKind::Polygon`] its circumcircle.
    pub geom: CurveGeom,
    /// What the chain is.
    pub kind: SketchCurveKind,
}

/// The exact circle of one drawn curve chain with the angular range its
/// surviving edges cover — the sketch-level analogue of
/// [`crate::topo::AnalyticRim`], produced by [`Sketch::curve_rims`] so the
/// inference engine can offer a drawn (unextruded) circle or arc's true
/// center, quadrant points, and tangents as snap candidates. World space
/// (sketch geometry is authored in world coordinates); the circle lies in
/// the sketch plane, whose normal is `axis`.
#[derive(Debug, Clone, PartialEq)]
pub struct SketchCurveRim {
    /// The curve chain this rim describes.
    pub curve: SketchCurveId,
    /// The exact drawn center, on the sketch plane.
    pub center: Point3,
    /// Unit circle-plane normal (the sketch plane normal).
    pub axis: Vec3,
    /// The exact drawn radius (> [`tol::POINT_MERGE`](crate::tol::POINT_MERGE)).
    pub radius: f64,
    /// First basis vector of the angular frame (unit, perpendicular to
    /// `axis`); angle 0 lies along it.
    pub basis_u: Vec3,
    /// Second basis vector (unit, completing the frame via
    /// [`crate::geom2d::plane_axes`] handedness).
    pub basis_v: Vec3,
    /// Merged angular coverage in the (basis_u, basis_v) frame — same
    /// representation as [`crate::topo::AnalyticRim::coverage`]: `None` =
    /// the full circle; otherwise disjoint `[start, end]` intervals with
    /// `start` normalized into `[-pi, pi)` and `end > start`.
    pub coverage: Option<Vec<[f64; 2]>>,
}

impl SketchCurveRim {
    /// Whether `angle` (radians, in the rim's basis frame) falls inside the
    /// covered range, within an angular tolerance derived from
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) at this radius — the
    /// same test as [`crate::topo::AnalyticRim::covers`].
    pub fn covers(&self, angle: f64) -> bool {
        crate::topo::coverage_covers(&self.coverage, self.radius, angle)
    }

    /// The rim's quadrant points — the covered subset of the four points at
    /// angles 0, pi/2, pi, 3pi/2 in the basis frame, on the exact circle.
    pub fn quadrant_points(&self) -> Vec<Point3> {
        [0.0f64, 0.5, 1.0, 1.5]
            .into_iter()
            .map(|q| q * std::f64::consts::PI)
            .filter(|&q| self.covers(q))
            .map(|q| {
                self.center
                    + self.basis_u * (self.radius * q.cos())
                    + self.basis_v * (self.radius * q.sin())
            })
            .collect()
    }
}

/// A connected component of a sketch's edges: what the user perceives as one
/// independent drawn shape. Derived (never serialized) and recomputed on
/// every edge-set mutation with identity reuse, exactly like regions — the
/// selection/label/delete/move unit for free-standing sketch geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SketchIsland {
    /// The island's edges, ascending by id.
    pub edges: Vec<SketchEdgeId>,
}

/// A point in a sketch. Always on the sketch plane
/// (within [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchVertex {
    /// Position in f64 meters (world frame).
    pub position: Point3,
}

/// A straight segment between two sketch vertices.
///
/// Edges never overlap or cross other edges of the same sketch — the sticky
/// rules split/merge eagerly at insertion time, so this is an invariant, not
/// a hope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SketchEdge {
    /// Start vertex.
    pub from: SketchVertexId,
    /// End vertex.
    pub to: SketchVertexId,
    /// The curve chain this edge belongs to, when it was committed as part
    /// of one drawn curve (an arc's or circle's facets). `None` for plain
    /// lines and rectangle sides. Fragments of a split curve edge inherit
    /// the id, so a curve stays one selectable unit across sticky splits.
    pub curve: Option<SketchCurveId>,
}

/// A closed region bounded by sketch edges: what the user perceives as "a
/// face appeared" while drawing — unless it is `faceless`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SketchRegion {
    /// Outer boundary vertices in cycle order, counter-clockwise seen from
    /// the plane normal side.
    pub outer: Vec<SketchVertexId>,
    /// Hole boundaries, each clockwise seen from the plane normal side.
    pub holes: Vec<Vec<SketchVertexId>>,
    /// A loop of edges with no face (module docs, rule 6): not filled,
    /// not picked as a face and not extruded ([`Sketch::profile`] refuses
    /// it). Set on the loops an import brings in as bare edges; cleared
    /// when a segment is drawn over one of the loop's edges. A region a
    /// drawing step brings about has a face.
    pub faceless: bool,
}

/// A region as it was before a mutation: its id and value, and its rings
/// as positions (its vertices may be gone once the mutation has run).
#[derive(Debug, Clone)]
struct RegionSnapshot {
    id: SketchRegionId,
    region: SketchRegion,
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
}

/// What [`Sketch::add_segment`] did, so tools and undo can react precisely.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentAdded {
    /// Vertices created (merged endpoints and crossings reuse existing ones,
    /// which are not listed).
    pub new_vertices: Vec<SketchVertexId>,
    /// Edges created, including the fragments of the inserted segment.
    pub new_edges: Vec<SketchEdgeId>,
    /// Pre-existing edges that were split: the now-dead handle and its
    /// replacement fragments, in order from the old edge's `from` end.
    pub split_edges: Vec<(SketchEdgeId, Vec<SketchEdgeId>)>,
    /// Regions that came into existence because circuits closed.
    pub regions_created: Vec<SketchRegionId>,
    /// Regions invalidated by the insertion (e.g., a region that gained a
    /// crossing edge is replaced by its two halves, listed in
    /// `regions_created`).
    pub regions_removed: Vec<SketchRegionId>,
}

/// What [`Sketch::offset_region`] did, so tools and undo can react
/// precisely. Region ids are diffed once over the whole insertion (not per
/// segment), so a region that appeared and vanished mid-insertion is listed
/// in neither field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RegionOffsetAdded {
    /// Edges created for the offset loops (fragments included, in insertion
    /// order).
    pub new_edges: Vec<SketchEdgeId>,
    /// Curve chains minted for the offset loops' analytic arc runs.
    pub new_curves: Vec<SketchCurveId>,
    /// Regions that exist now but did not before the offset.
    pub regions_created: Vec<SketchRegionId>,
    /// Regions that existed before the offset but do not now.
    pub regions_removed: Vec<SketchRegionId>,
}

/// What [`Sketch::refacet_curve`] did. The chain's own id and its analytic
/// [`CurveGeom`] are deliberately absent: they are what re-faceting
/// *preserves*, so there is nothing to report about them.
///
/// **Island ids are NOT preserved and are not reported.** Rebuilding replaces
/// every edge of the chain, and [`SketchIsland`] identity is anchored on an
/// island's edges, so the re-faceted circle's island is a new one afterwards
/// even when nothing else about the drawing changed. That is deliberately not
/// listed here, unlike the region churn: regions are handed to callers as
/// durable references (a region is what an extrusion consumes), whereas an
/// island is a derived selection/labelling unit that every caller re-queries
/// from the sketch after a mutation rather than holding across one. A caller
/// that ever does hold an island id across a re-facet must re-query it —
/// there is no report field that will tell it to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CurveRefaceted {
    /// The chain's old facet edges, all now dead.
    pub removed_edges: Vec<SketchEdgeId>,
    /// Vertices deleted with them (the whole old ring, since a re-facetable
    /// chain by definition shares no vertex with anything else).
    pub removed_vertices: Vec<SketchVertexId>,
    /// The chain's new facet edges, in ring order from the preserved start
    /// angle.
    pub new_edges: Vec<SketchEdgeId>,
    /// Vertices created for them, ascending by id.
    pub new_vertices: Vec<SketchVertexId>,
    /// Regions that exist now but did not before (a region whose boundary
    /// cycle is unchanged keeps its id and appears in neither list — but a
    /// re-facet always reshapes the cycle, so the circle's own region is
    /// always replaced).
    pub regions_created: Vec<SketchRegionId>,
    /// Regions that existed before but do not now.
    pub regions_removed: Vec<SketchRegionId>,
}

/// What [`Sketch::remove_edge`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdgeRemoved {
    /// Vertices deleted because no edge uses them anymore.
    pub removed_vertices: Vec<SketchVertexId>,
    /// Regions that no longer close without this edge.
    pub regions_removed: Vec<SketchRegionId>,
    /// Regions created by the removal (two regions separated only by this
    /// edge merge into one).
    pub regions_created: Vec<SketchRegionId>,
}

/// What [`Sketch::move_vertex_arranging`] did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VertexMoved {
    /// The vertex's position before the drag.
    pub old_pos: Point3,
    /// `false`: the vertex moved in place and every handle is valid.
    /// `true`: the drag touched other geometry and its incident edges were
    /// replayed through the sticky rules; handles of that geometry are fresh.
    pub rearranged: bool,
}

/// Typed failures of sketch operations. Nothing is repaired silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchError {
    /// An input point is farther than [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the sketch
    /// plane. The kernel does not project; the caller decides.
    PointOffPlane {
        /// Index of the offending point in the call's arguments (0-based).
        which: usize,
    },
    /// Segment endpoints are within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of each other.
    DegenerateSegment,
    /// The given edge handle is not in this sketch (or already removed).
    UnknownEdge,
    /// The given vertex handle is not in this sketch (or already removed).
    UnknownVertex,
    /// The given region handle is not in this sketch (or already invalid).
    UnknownRegion,
    /// The region is a loop of edges with no face ([`SketchRegion::faceless`]),
    /// so there is no profile to extrude. Drawing over one of its edges gives
    /// it a face. No silent repair (rule 4) — the caller decides.
    RegionHasNoFace,
    /// A topology-preserving [`Sketch::move_vertex`] was refused because it
    /// would require re-stitching the sketch: an incident edge would cross or
    /// overlap another edge, or the moved vertex would land on another vertex
    /// (a merge). No silent repair (rule 4) — the caller decides.
    WouldRetopologize,
    /// The island handle is stale (islands die whenever an edge-set mutation
    /// reshapes them — always re-query after mutating).
    UnknownIsland,
    /// The region's traced boundary does not form a valid [`Profile`] (a
    /// kernel bug in region tracing, surfaced as a typed error rather than a
    /// panic so it cannot brick the caller).
    MalformedRegion,
    /// The region's outer ring touches one of its holes (or two holes touch)
    /// at a single point: a circle drawn inside a rectangle with its rim on
    /// one side. The region is a face, but it cannot be extruded as one
    /// piece: the swept walls would meet four to an edge at that point.
    /// The region inside the hole extrudes on its own.
    RegionTouchesHole,
    /// A [`CurveGeom`] is degenerate: its radius is not finite or not larger
    /// than [`tol::POINT_MERGE`](crate::tol::POINT_MERGE).
    DegenerateCurve,
    /// The given curve-chain handle is not in this sketch.
    UnknownCurve,
    /// The chain carries no analytic definition, so there is no circle to
    /// re-facet from: it was committed through the plain
    /// [`Sketch::begin_curve`], loaded from a pre-v10 file, or had its
    /// geometry dropped by a deforming edit ([`Sketch::move_vertex`], a
    /// non-similarity [`Sketch::apply_transform`]).
    CurveNotAnalytic,
    /// The chain is not a re-facetable circle. Re-faceting replaces every
    /// facet, so it is only defined on a chain that is a WHOLE, untouched
    /// circle: a [`SketchCurveKind::Circle`] whose live edges form one closed
    /// ring covering the full turn, none of whose vertices carries anything
    /// but that chain's own edges. A partially erased circle, an arc, a
    /// polygon, or a circle glued to other geometry (a T-junction, a rounded
    /// corner, a boundary shared with another region) is refused — rebuilding
    /// it would silently destroy those relationships (rule 4).
    CurveNotRefacetable,
    /// The requested facet count is below
    /// [`MIN_CIRCLE_SEGMENTS`] — the density floor at which a chord ring
    /// stops being a circle at all.
    SegmentsBelowFloor,
    /// The requested facet count is above [`MAX_CIRCLE_SEGMENTS`].
    SegmentsAboveCap,
    /// Undoing an extrusion could not re-insert the scaffolding it had
    /// deleted: geometry drawn since then crosses or overlaps where the
    /// outline was ([`Sketch::restore_edges`]). The sketch is untouched —
    /// erase the conflicting geometry and undo again.
    RestoreConflicts,
    /// An [`Sketch::offset_region`] distance whose magnitude is not larger
    /// than [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) (or not finite) —
    /// the offset boundary would coincide with the original.
    OffsetTooSmall,
    /// The [`Sketch::offset_region`] distance collapses the region's
    /// boundary: it exceeds what the shape can absorb (an inward offset past
    /// the inradius, an arc radius driven to zero, a hole shrunk away) or
    /// the offset loops cross themselves or each other. The sketch is
    /// untouched — no silent repair (rule 4).
    OffsetCollapsed,
}

impl std::fmt::Display for SketchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SketchError::PointOffPlane { which } => {
                write!(f, "input point {which} is off the sketch plane")
            }
            SketchError::DegenerateSegment => write!(f, "segment endpoints coincide"),
            SketchError::UnknownEdge => write!(f, "no such edge in this sketch"),
            SketchError::UnknownVertex => write!(f, "no such vertex in this sketch"),
            SketchError::UnknownRegion => write!(f, "no such region in this sketch"),
            SketchError::RegionHasNoFace => {
                write!(
                    f,
                    "the loop has no face to extrude; draw over one of its edges to give it one"
                )
            }
            SketchError::UnknownIsland => {
                write!(f, "no such island in this sketch")
            }
            SketchError::WouldRetopologize => {
                write!(f, "the move would cross or merge sketch geometry")
            }
            SketchError::MalformedRegion => {
                write!(f, "region boundary does not form a valid profile")
            }
            SketchError::RegionTouchesHole => {
                write!(
                    f,
                    "region touches one of its holes at a point and cannot be extruded as one piece"
                )
            }
            SketchError::DegenerateCurve => {
                write!(f, "curve radius is degenerate")
            }
            SketchError::UnknownCurve => write!(f, "no such curve chain in this sketch"),
            SketchError::CurveNotAnalytic => {
                write!(f, "the curve chain carries no analytic circle")
            }
            SketchError::CurveNotRefacetable => {
                write!(f, "only a whole, untouched circle can be re-faceted")
            }
            SketchError::SegmentsBelowFloor => {
                write!(f, "a circle needs at least {MIN_CIRCLE_SEGMENTS} segments")
            }
            SketchError::SegmentsAboveCap => {
                write!(f, "a circle allows at most {MAX_CIRCLE_SEGMENTS} segments")
            }
            SketchError::RestoreConflicts => {
                write!(
                    f,
                    "the restored outline would cross geometry drawn since the extrusion"
                )
            }
            SketchError::OffsetTooSmall => {
                write!(f, "offset distance is too small to produce a boundary")
            }
            SketchError::OffsetCollapsed => {
                write!(f, "the offset distance collapses the boundary")
            }
        }
    }
}

impl std::error::Error for SketchError {}

/// A planar drawing surface with sticky-geometry semantics.
#[derive(Debug, Clone)]
pub struct Sketch {
    plane: Plane,
    /// A **detached shape** (persisted, part of equality): drawing that
    /// takes part in no plane arrangement. A drafting lift of its plane
    /// leaves it out, so later drawing there neither splits nor merges with
    /// it and it never becomes face topology; it is edited only through its
    /// own sketch-only draft ([`crate::Document::draft_detached`]), and
    /// extruding one of its regions always makes a new Object. Never set on
    /// an object-owned sketch (invariant I9). Every rebuild of an existing
    /// sketch starts from [`Sketch::blank_like`], which carries it.
    detached: bool,
    vertices: SlotMap<SketchVertexId, SketchVertex>,
    edges: SlotMap<SketchEdgeId, SketchEdge>,
    regions: SlotMap<SketchRegionId, SketchRegion>,
    /// Live curve chains: membership lives on each edge's `curve` field; the
    /// value is the chain's analytic definition, when the drawing tool
    /// supplied one ([`Sketch::begin_curve_with`]). `None` for chains
    /// committed before geometry capture existed (pre-v10 files) or through
    /// the plain [`Sketch::begin_curve`].
    curves: SlotMap<SketchCurveId, Option<CurveAnalytic>>,
    /// Current connected components (derived; see [`SketchIsland`]).
    islands: SlotMap<SketchIslandId, SketchIsland>,
    /// Curve id applied to edges inserted by `add_segment` while a
    /// `begin_curve`/`end_curve` bracket is open. Transient tool state — a
    /// gesture snapshot restores it with the rest of the sketch.
    active_curve: Option<SketchCurveId>,
    /// Transient bulk-insertion state ([`Sketch::add_segments`]; never
    /// saved, ignored by equality, `None` outside a bulk call): every
    /// vertex by the cell of [`vertex_cell`] it falls in, so the merge
    /// lookup of a new end probes a few cells instead of every vertex.
    bulk_vertex_cells: Option<std::collections::BTreeMap<[i64; 3], Vec<SketchVertexId>>>,
    /// Transient drafting state (never saved, ignored by equality): while
    /// set, [`Sketch::add_segment`] first moves each endpoint within
    /// [`tol::POINT_MERGE`] of existing geometry onto it — a vertex, else the
    /// nearest edge's line (its end when the foot lands within the
    /// tolerance of it) — so drawing that touches an edge splits it
    /// straight. A drafting working sketch sets it; plain sketches keep the
    /// contact-at-the-point behaviour.
    snap_to_existing: bool,
    /// Transient drafting state (never saved, ignored by equality): the
    /// points where drawing was split by a solid's own edge when the draft
    /// opened, carried along when the drawing at them is moved or copied
    /// ([`Sketch::apply_transform_edges`], [`Sketch::copy_edges_transformed`]),
    /// so the drafting engine can heal a seam moved off the boundary.
    seams: Vec<Point3>,
    /// Transient drafting state (never saved, ignored by equality): lines a
    /// drafting lift holds from elsewhere (another plane's or sketch's
    /// drawing lying in this plane) that new drawing must not bend. A
    /// facet of a curve chain lying along one is split only on its own
    /// line, never at a point of its exact circle off that line
    /// ([`Sketch::split_circle_facet_at`]): the line stays drawn end to end
    /// as its home holds it.
    rigid: Vec<(Point3, Point3)>,
}

/// Everything an undo needs to re-insert consumed scaffolding
/// slot-exactly: the removed edges as geometry rows (ascending
/// original-edge-slot order, [`Sketch::scaffolding_removal`]'s `BTreeSet`
/// capture order) plus the positions of the vertices the removal doomed
/// (ascending original-vertex-slot order). The vertex list is recorded —
/// not re-derived from the rows — because a row order sorted by edge id
/// does NOT determine the vertices' first-encounter order: an edge living
/// in a reused (low) slot sorts before older edges while referencing
/// newer (high-slot) vertices, so any single replay order can weld
/// vertices back into permuted slots. Recording both orders independently
/// is what lets [`Sketch::restore_edges`] restore each side into its
/// exact original slot.
#[derive(Debug, Clone)]
pub(crate) struct RemovedScaffolding {
    /// One row per removed edge: (endpoint, endpoint, curve-chain id), in
    /// ascending original-edge-slot order.
    pub rows: Vec<(Point3, Point3, Option<SketchCurveId>)>,
    /// The doomed vertices' positions in ascending original-vertex-slot
    /// order — exactly the vertices [`Sketch::remove_edges`] will delete
    /// for this edge set (every incident edge is in the set).
    pub vertices: Vec<Point3>,
}

impl Sketch {
    /// An empty sketch on `plane` (not detached).
    pub fn on_plane(plane: Plane) -> Sketch {
        Self::blank(plane, false)
    }

    /// An empty **detached** sketch on `plane` (see the field `detached`).
    pub fn detached_on_plane(plane: Plane) -> Sketch {
        Self::blank(plane, true)
    }

    /// Whether this sketch is a detached shape: drawing that takes part in
    /// no plane arrangement (see the field `detached`).
    pub fn detached(&self) -> bool {
        self.detached
    }

    /// An empty sketch on this sketch's plane with its `detached` flag:
    /// what every rebuild of an existing sketch starts from, so the flag is
    /// carried structurally. Transient drafting state (curve bracket, bulk
    /// insertion cells, snapping, seams, rigid lines) is NOT copied.
    pub fn blank_like(&self) -> Sketch {
        Self::blank(self.plane, self.detached)
    }

    /// [`Sketch::blank_like`] on another plane: an empty sketch on `plane`
    /// carrying this sketch's `detached` flag — the start of a rebuild that
    /// lands this sketch's drawing on a transformed plane.
    pub(crate) fn blank_like_on(&self, plane: Plane) -> Sketch {
        Self::blank(plane, self.detached)
    }

    /// The one empty-sketch constructor every other one goes through.
    fn blank(plane: Plane, detached: bool) -> Sketch {
        Sketch {
            plane,
            detached,
            vertices: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            regions: SlotMap::with_key(),
            curves: SlotMap::with_key(),
            active_curve: None,
            islands: SlotMap::with_key(),
            bulk_vertex_cells: None,
            snap_to_existing: false,
            seams: Vec::new(),
            rigid: Vec::new(),
        }
    }

    /// Turns endpoint snapping onto existing geometry on or off (see the
    /// field `snap_to_existing`). Drafting turns it on for its working
    /// sketch.
    pub(crate) fn set_snap_to_existing(&mut self, on: bool) {
        self.snap_to_existing = on;
    }

    /// Sets the seam points (see the field `seams`).
    pub(crate) fn set_seams(&mut self, seams: Vec<Point3>) {
        self.seams = seams;
    }

    /// The seam points, carried through moves and copies.
    pub(crate) fn seams(&self) -> &[Point3] {
        &self.seams
    }

    /// Sets the rigid lines (see the field `rigid`).
    pub(crate) fn set_rigid(&mut self, rigid: Vec<(Point3, Point3)>) {
        self.rigid = rigid;
    }

    /// `t` of every seam that is an endpoint of one of `edges` (the drawing
    /// about to move or be copied by `t`), read from this sketch BEFORE the
    /// edit: a copy can split or merge those very edges in its target.
    fn carried_seams(&self, edges: &[SketchEdgeId], t: &crate::Transform) -> Vec<Point3> {
        if self.seams.is_empty() {
            return Vec::new();
        }
        let ends: Vec<Point3> = edges
            .iter()
            .flat_map(|&e| {
                let ed = self.edges[e];
                [
                    self.vertices[ed.from].position,
                    self.vertices[ed.to].position,
                ]
            })
            .collect();
        self.seams
            .iter()
            .filter(|&&s| ends.iter().any(|q| q.approx_eq(s, tol::POINT_MERGE)))
            .map(|&s| t.apply_point(s))
            .collect()
    }

    /// Adds the seam positions `pts` not already present.
    fn add_seams(&mut self, pts: Vec<Point3>) {
        for q in pts {
            if !self.seams.iter().any(|s| s.approx_eq(q, tol::POINT_MERGE)) {
                self.seams.push(q);
            }
        }
    }

    /// `p` moved onto existing geometry within [`tol::POINT_MERGE`]: an
    /// existing vertex, else the foot on the nearest edge (that edge's end
    /// when the foot lies within the tolerance of it); `p` itself when
    /// nothing is that close.
    fn snapped_endpoint(&self, p: Point3) -> Point3 {
        let m = tol::POINT_MERGE;
        if let Some(v) = self.vertices.values().find(|v| v.position.approx_eq(p, m)) {
            return v.position;
        }
        let mut best: Option<(f64, Point3)> = None;
        for e in self.edges.values() {
            let (a, b) = (self.vertices[e.from].position, self.vertices[e.to].position);
            let ab = b - a;
            let len2 = ab.length_squared();
            if len2 <= 0.0 {
                continue;
            }
            let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
            let q = a + ab * t;
            let d = (q - p).length();
            if d <= m && best.is_none_or(|(bd, _)| d < bd) {
                let q = if q.approx_eq(a, m) {
                    a
                } else if q.approx_eq(b, m) {
                    b
                } else {
                    q
                };
                best = Some((d, q));
            }
        }
        best.map_or(p, |(_, q)| q)
    }

    /// The sketch plane.
    pub fn plane(&self) -> Plane {
        self.plane
    }

    /// Vertex storage (read-only).
    pub fn vertices(&self) -> &SlotMap<SketchVertexId, SketchVertex> {
        &self.vertices
    }

    /// Edge storage (read-only).
    pub fn edges(&self) -> &SlotMap<SketchEdgeId, SketchEdge> {
        &self.edges
    }

    /// Opens a curve bracket: edges added by `add_segment` until
    /// [`Sketch::end_curve`] are tagged as ONE curve chain (an arc's or
    /// circle's facets), so the UI can select and delete them as a unit.
    /// Returns the minted curve id. Nesting is not supported — a second
    /// `begin_curve` simply replaces the active id.
    pub fn begin_curve(&mut self) -> SketchCurveId {
        let id = self.curves.insert(None);
        self.active_curve = Some(id);
        id
    }

    /// [`Sketch::begin_curve`] with the chain's analytic definition: the
    /// exact circle the facets approximate, as the drawing tool computed it.
    /// The geometry is durable — it survives edge splits (fragments keep the
    /// chain id), persists in the file format, and is what extrusion carries
    /// onto the solid (the true-curves design).
    ///
    /// # Errors
    /// [`SketchError::PointOffPlane`] (`which: 0`) if `geom.center` is
    /// farther than [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the
    /// sketch plane; [`SketchError::DegenerateCurve`] if `geom.radius` is
    /// not finite or not larger than
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE). On error no curve is
    /// minted and no bracket opens (strong guarantee).
    pub fn begin_curve_with(&mut self, geom: CurveGeom) -> Result<SketchCurveId, SketchError> {
        self.begin_curve_with_kind(geom, SketchCurveKind::Circle)
    }

    /// [`Sketch::begin_curve_with`] naming what the chain's circle *claims*
    /// (see [`SketchCurveKind`]): `Circle` for a faceted circle or arc,
    /// `Polygon` for a regular polygon whose sides are its real geometry and
    /// whose circumcircle contributes only a center.
    ///
    /// # Errors
    /// Same as [`Sketch::begin_curve_with`]: [`SketchError::PointOffPlane`]
    /// (`which: 0`) for an off-plane center, [`SketchError::DegenerateCurve`]
    /// for a non-finite or too-small radius. On error no curve is minted and
    /// no bracket opens (strong guarantee).
    pub fn begin_curve_with_kind(
        &mut self,
        geom: CurveGeom,
        kind: SketchCurveKind,
    ) -> Result<SketchCurveId, SketchError> {
        if self.plane.signed_distance(geom.center).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 0 });
        }
        if !geom.radius.is_finite() || geom.radius <= tol::POINT_MERGE {
            return Err(SketchError::DegenerateCurve);
        }
        let id = self.curves.insert(Some(CurveAnalytic { geom, kind }));
        self.active_curve = Some(id);
        Ok(id)
    }

    /// The analytic circle of `curve` **when the chain is a circle** — `None`
    /// for a chain carrying no geometry (plain [`Sketch::begin_curve`],
    /// pre-v10 file, geometry dropped by a deforming edit, or a stale handle)
    /// AND for a [`SketchCurveKind::Polygon`] chain.
    ///
    /// Polygons are excluded deliberately: every caller of this method feeds
    /// an analytic *circle* consumer — profile curve attribution and the
    /// cylindrical walls extrusion sweeps from it, concentric offset, the
    /// Follow Me path's analytic tangent, the serializer's circle record — and
    /// a polygon's circumcircle describes none of them. Use
    /// [`Sketch::curve_analytic`] to see a polygon's record, or
    /// [`Sketch::curve_center`] for the one thing it does supply.
    pub fn curve_geom(&self, curve: SketchCurveId) -> Option<CurveGeom> {
        self.curve_analytic(curve)
            .filter(|a| a.kind == SketchCurveKind::Circle)
            .map(|a| a.geom)
    }

    /// The full analytic record of `curve` — circle *and* what it claims —
    /// regardless of kind. `None` when the chain carries none, or the handle
    /// is stale.
    pub fn curve_analytic(&self, curve: SketchCurveId) -> Option<CurveAnalytic> {
        self.curves.get(curve).copied().flatten()
    }

    /// The exact drawn center of `curve`, whatever kind it is — the one
    /// analytic fact a polygon chain contributes, and the same point a circle
    /// chain's rim reports. `None` when the chain carries no geometry or the
    /// handle is stale.
    pub fn curve_center(&self, curve: SketchCurveId) -> Option<Point3> {
        self.curve_analytic(curve).map(|a| a.geom.center)
    }

    /// Closes the open curve bracket (no-op when none is open).
    pub fn end_curve(&mut self) {
        self.active_curve = None;
    }

    /// The curve chain `edge` belongs to, or `None` for a plain line (or a
    /// stale handle).
    pub fn edge_curve(&self, edge: SketchEdgeId) -> Option<SketchCurveId> {
        self.edges.get(edge).and_then(|e| e.curve)
    }

    /// The maximal run of `edge`'s curve reachable through vertices used by
    /// NOTHING but that curve — the selection unit for a drawn arc/circle
    /// once other geometry touches it. A junction vertex (crossing or touch:
    /// three or more incident edges, or two from different owners) stops the
    /// walk, so the piece of a circle inside an outline selects separately
    /// from the piece that became the outline's rounded corner. For an
    /// untouched curve this is the whole curve; for a plain line it is just
    /// the edge itself. Ascending by id; empty for a stale handle.
    pub fn curve_chain_at(&self, edge: SketchEdgeId) -> Vec<SketchEdgeId> {
        let Some(e) = self.edges.get(edge) else {
            return Vec::new();
        };
        let Some(curve) = e.curve else {
            return vec![edge];
        };

        let mut vertex_edges: std::collections::BTreeMap<SketchVertexId, Vec<SketchEdgeId>> =
            std::collections::BTreeMap::new();
        for (eid, ed) in &self.edges {
            vertex_edges.entry(ed.from).or_default().push(eid);
            vertex_edges.entry(ed.to).or_default().push(eid);
        }

        let mut chain: std::collections::BTreeSet<SketchEdgeId> = std::collections::BTreeSet::new();
        chain.insert(edge);
        let mut frontier = vec![edge];
        while let Some(cur) = frontier.pop() {
            let ed = self.edges[cur];
            for v in [ed.from, ed.to] {
                let incident = &vertex_edges[&v];
                // Interior curve vertex: exactly two edges, both this curve.
                if incident.len() != 2 {
                    continue;
                }
                if !incident.iter().all(|&i| self.edges[i].curve == Some(curve)) {
                    continue;
                }
                for &n in incident {
                    if chain.insert(n) {
                        frontier.push(n);
                    }
                }
            }
        }
        chain.into_iter().collect()
    }

    /// Every edge of `curve`, in slotmap order. Empty for a stale id.
    pub fn curve_edges(&self, curve: SketchCurveId) -> Vec<SketchEdgeId> {
        self.edges
            .iter()
            .filter(|(_, e)| e.curve == Some(curve))
            .map(|(id, _)| id)
            .collect()
    }

    /// Registers a curve id (with its optional analytic definition) during
    /// structural (file-load) reconstruction.
    pub(crate) fn insert_curve_raw(&mut self, analytic: Option<CurveAnalytic>) -> SketchCurveId {
        self.curves.insert(analytic)
    }

    /// The exact circles of this sketch's drawn curves, one rim per curve
    /// chain that carries an analytic definition ([`CurveGeom`]) and still
    /// has at least one live edge — the sketch-level analogue of
    /// [`Object::analytic_rims`](crate::topo::Object::analytic_rims), so a
    /// drawn (unextruded) circle or arc offers its true center and quadrant
    /// points to inference before any extrusion exists.
    ///
    /// Coverage comes from the surviving member edges: each edge's chord
    /// spans the short way between its endpoint angles (a facet subtends
    /// less than a half turn by construction), merged exactly like a solid
    /// rim's boundary edges — so deleting part of a circle trims the covered
    /// arc, and a curve whose surviving edges yield no coverage (all
    /// endpoints angle-degenerate) is omitted entirely, matching the
    /// vacant-rim gate ([`crate::topo::AnalyticRim::has_coverage`]).
    ///
    /// Deterministic: curves in slotmap order, basis via
    /// [`plane_axes`] on the sketch plane normal.
    pub fn curve_rims(&self) -> Vec<SketchCurveRim> {
        let axis = self.plane.normal();
        let (u, v) = plane_axes(axis);
        let mut out = Vec::new();
        for (cid, analytic) in &self.curves {
            // Circles only. A polygon's sides are its geometry, so its
            // circumcircle has no rim to offer — no quadrant points (they lie
            // on no edge) and no tangents. Its center is published separately
            // by `curve_centers`.
            let Some(CurveAnalytic {
                geom,
                kind: SketchCurveKind::Circle,
            }) = *analytic
            else {
                continue;
            };
            let mut intervals: Vec<[f64; 2]> = Vec::new();
            let mut live = false;
            for (_, e) in self.edges.iter().filter(|(_, e)| e.curve == Some(cid)) {
                live = true;
                // Endpoint angle in the (u, v) frame; `None` for a point
                // radially degenerate at the center (cannot happen for a
                // genuine facet of a curve with radius > POINT_MERGE, but a
                // guard beats a garbage atan2).
                let angle_of = |vid: SketchVertexId| -> Option<f64> {
                    let d = self.vertices[vid].position - geom.center;
                    let radial = d - axis * d.dot(axis);
                    if radial.length() <= tol::NORMALIZE_MIN_LENGTH {
                        None
                    } else {
                        Some(radial.dot(v).atan2(radial.dot(u)))
                    }
                };
                if let (Some(a), Some(b)) = (angle_of(e.from), angle_of(e.to)) {
                    let mut offset = b - a;
                    while offset > std::f64::consts::PI {
                        offset -= 2.0 * std::f64::consts::PI;
                    }
                    while offset <= -std::f64::consts::PI {
                        offset += 2.0 * std::f64::consts::PI;
                    }
                    intervals.push([a.min(a + offset), a.max(a + offset)]);
                }
            }
            if !live {
                continue;
            }
            let coverage = crate::topo::merge_angular_intervals(&intervals, geom.radius);
            if matches!(&coverage, Some(iv) if iv.is_empty()) {
                continue; // vacant rim: no surviving arc, no candidates
            }
            out.push(SketchCurveRim {
                curve: cid,
                center: geom.center,
                axis,
                radius: geom.radius,
                basis_u: u,
                basis_v: v,
                coverage,
            });
        }
        out
    }

    /// The exact drawn centers of this sketch's [`SketchCurveKind::Polygon`]
    /// chains that still have at least one live edge — the polygon analogue
    /// of the center [`Sketch::curve_rims`] publishes for a circle, and the
    /// ONLY analytic point a polygon offers (its vertices are already
    /// endpoints and its side midpoints already midpoints; cardinal points of
    /// the circumcircle would lie on no edge at all).
    ///
    /// Circles are not listed here — their center arrives with the rest of
    /// their rim through [`Sketch::curve_rims`], so a caller registering both
    /// gets each center exactly once.
    ///
    /// Deterministic: curves in slotmap order.
    pub fn polygon_centers(&self) -> Vec<(SketchCurveId, Point3)> {
        self.curves
            .iter()
            .filter_map(|(cid, analytic)| {
                let CurveAnalytic {
                    geom,
                    kind: SketchCurveKind::Polygon,
                } = (*analytic)?
                else {
                    return None;
                };
                // A chain whose every edge has been erased is gone as far as
                // the user is concerned — matching `curve_rims`' `live` gate.
                self.edges
                    .values()
                    .any(|e| e.curve == Some(cid))
                    .then_some((cid, geom.center))
            })
            .collect()
    }

    /// Current islands — connected components of the edge graph (read-only).
    /// Kept up to date by every edge-set mutation; ids are stable while an
    /// island keeps its anchor edge (see [`Sketch::recompute_islands`]).
    pub fn islands(&self) -> &SlotMap<SketchIslandId, SketchIsland> {
        &self.islands
    }
    pub fn island_of_edge(&self, edge: SketchEdgeId) -> Option<SketchIslandId> {
        self.islands
            .iter()
            .find(|(_, isl)| isl.edges.binary_search(&edge).is_ok())
            .map(|(id, _)| id)
    }

    /// Rebuild the island set from the current edge graph, preserving ids:
    /// a new component reuses an old island's id when it contains that
    /// island's smallest surviving edge (each old id claimed once, smallest
    /// anchor first — deterministic for a deterministically built sketch).
    /// Called at the end of every mutation that changes the edge set.
    pub(crate) fn recompute_islands(&mut self) {
        // Anchor edge (smallest surviving) per old island.
        let old_anchors: Vec<(SketchIslandId, SketchEdgeId)> = self
            .islands
            .iter()
            .filter_map(|(id, isl)| {
                isl.edges
                    .iter()
                    .copied()
                    .filter(|e| self.edges.contains_key(*e))
                    .min()
                    .map(|m| (id, m))
            })
            .collect();

        // Connected components by shared vertices, discovered in edge-id
        // order (each component's edge list ends up ascending).
        let mut vertex_edges: std::collections::BTreeMap<SketchVertexId, Vec<SketchEdgeId>> =
            std::collections::BTreeMap::new();
        for (eid, e) in &self.edges {
            vertex_edges.entry(e.from).or_default().push(eid);
            vertex_edges.entry(e.to).or_default().push(eid);
        }
        let mut seen: std::collections::BTreeSet<SketchEdgeId> = std::collections::BTreeSet::new();
        let mut components: Vec<Vec<SketchEdgeId>> = Vec::new();
        for start in self.edges.keys() {
            if seen.contains(&start) {
                continue;
            }
            let mut comp = Vec::new();
            let mut queue = vec![start];
            seen.insert(start);
            while let Some(eid) = queue.pop() {
                comp.push(eid);
                let e = self.edges[eid];
                for v in [e.from, e.to] {
                    for &n in &vertex_edges[&v] {
                        if seen.insert(n) {
                            queue.push(n);
                        }
                    }
                }
            }
            comp.sort();
            components.push(comp);
        }

        // Assign ids: reuse where the anchor edge landed; fresh otherwise.
        // Each component takes, of the unused old islands anchored on one
        // of its edges, the one with the smallest (anchor, id) — found by
        // walking its ascending edges through an anchor index, so a sketch
        // of thousands of islands is not quadratic in them.
        let mut by_anchor: std::collections::BTreeMap<SketchEdgeId, Vec<SketchIslandId>> =
            std::collections::BTreeMap::new();
        for &(id, anchor) in &old_anchors {
            by_anchor.entry(anchor).or_default().push(id);
        }
        for ids in by_anchor.values_mut() {
            ids.sort();
        }
        let mut used: std::collections::BTreeSet<SketchIslandId> =
            std::collections::BTreeSet::new();
        let mut assignments: Vec<(Option<SketchIslandId>, Vec<SketchEdgeId>)> = Vec::new();
        for comp in components {
            let reuse = comp.iter().find_map(|e| {
                by_anchor
                    .get(e)?
                    .iter()
                    .copied()
                    .find(|id| !used.contains(id))
            });
            if let Some(id) = reuse {
                used.insert(id);
            }
            assignments.push((reuse, comp));
        }

        let dead: Vec<SketchIslandId> = self
            .islands
            .keys()
            .filter(|id| !used.contains(id))
            .collect();
        for id in dead {
            self.islands.remove(id);
        }
        for (reuse, edges) in assignments {
            match reuse {
                Some(id) => self.islands[id].edges = edges,
                None => {
                    self.islands.insert(SketchIsland { edges });
                }
            }
        }
    }

    /// Rigidly move ONE island by an in-plane transform. The single-island
    /// case of [`Sketch::apply_transform_islands`].
    ///
    /// [`Sketch::apply_transform_island`]'s acceptance without the commit:
    /// `Ok(())` iff the move would be accepted. Callers batching several
    /// island moves validate ALL of them first so a refusal aborts the whole
    /// batch instead of leaving earlier islands moved.
    pub fn validate_transform_island(
        &self,
        island: SketchIslandId,
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        let mut probe = self.clone();
        probe.apply_transform_island(island, t)
    }

    /// Rigidly move ONE island by an in-plane transform; see
    /// [`Sketch::apply_transform_islands`].
    pub fn apply_transform_island(
        &mut self,
        island: SketchIslandId,
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        self.apply_transform_islands(&[island], t)
    }

    /// Adds a transformed COPY of `edges` to this sketch, landing like the
    /// same lines drawn there (the copy arranges with whatever it meets; the
    /// originals stay). One curve bracket per source chain.
    ///
    /// # Errors
    /// [`SketchError::UnknownEdge`]; [`SketchError::PointOffPlane`] for an
    /// off-plane landing; any typed refusal of the replay. Strong guarantee.
    pub fn copy_edges_transformed(
        &mut self,
        edges: &[SketchEdgeId],
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        if edges.iter().any(|e| !self.edges.contains_key(*e)) {
            return Err(SketchError::UnknownEdge);
        }
        let source = self.clone();
        let mut target = self.clone();
        source.replay_edges_into(edges, t, &mut target)?;
        target.add_seams(source.carried_seams(edges, t));
        *self = target;
        Ok(())
    }

    /// Moves a set of EDGES (not whole islands) by one in-plane transform,
    /// landing them exactly as the same lines drawn there would: they are
    /// removed and replayed through [`Sketch::add_segment`] at their
    /// transformed positions (one curve bracket per chain, map-or-drop curve
    /// geometry). The drafting engine's move: a drawing unit there is an
    /// island of drawing edges, which can share vertices with a solid's
    /// fixed boundary that must not move with it. Handles of the moved
    /// geometry are fresh.
    ///
    /// # Errors
    /// [`SketchError::UnknownEdge`] for a stale edge;
    /// [`SketchError::PointOffPlane`] for an off-plane landing; any typed
    /// refusal of the replay. Strong guarantee: untouched on `Err`.
    pub fn apply_transform_edges(
        &mut self,
        edges: &[SketchEdgeId],
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        if edges.iter().any(|e| !self.edges.contains_key(*e)) {
            return Err(SketchError::UnknownEdge);
        }
        for &e in edges {
            let ed = self.edges[e];
            for v in [ed.from, ed.to] {
                let p = t.apply_point(self.vertices[v].position);
                if self.plane.signed_distance(p).abs() > tol::PLANE_DIST {
                    return Err(SketchError::PointOffPlane { which: 0 });
                }
            }
        }
        let set: std::collections::BTreeSet<SketchEdgeId> = edges.iter().copied().collect();
        let mut moved = self.clone();
        moved.add_seams(self.carried_seams(edges, t));
        moved.remove_edges(&set);
        self.replay_edges_into(edges, t, &mut moved)?;
        *self = moved;
        Ok(())
    }

    /// Move a SET of islands together by one in-plane transform. The
    /// transform must keep every vertex on the sketch plane
    /// ([`SketchError::PointOffPlane`] otherwise); the caller (the document
    /// layer) has already vetted invertibility; an in-plane mirror reshapes
    /// its islands (their regions are re-traced with new ids).
    ///
    /// A moved shape lands exactly as the same shape DRAWN there would
    /// (unified-drafting decision 4): where it touches, crosses, or overlaps
    /// geometry outside the set, the sticky rules split and merge with it.
    /// Two mechanisms, one result:
    ///
    /// - **Clear landing** (no moved edge comes within
    ///   [`tol::POINT_MERGE`] of any other edge): the vertices move in place.
    ///   Every edge, vertex, island, and curve handle stays valid, and a
    ///   region keeps its id while its outer cycle is unchanged. Curve
    ///   geometry rides along iff the whole chain moved under a similarity
    ///   (map-or-drop): a chain straddling the set's boundary can no longer
    ///   be described by one circle, so its geometry drops.
    /// - **Touching landing**: the set's edges are removed and replayed
    ///   through [`Sketch::add_segment`] at their transformed positions
    ///   ([`Sketch::rebuild_islands_transformed`]'s replay, onto this
    ///   sketch). Handles of the moved geometry are fresh; callers re-query,
    ///   as after any reshaping mutation. Each moved curve chain becomes one
    ///   new chain carrying the mapped circle; members left behind keep the
    ///   source chain.
    ///
    /// Regions and islands re-derive in both cases, so a shape moved inside
    /// a closed loop becomes its hole, as a drawn one would. All islands move
    /// together: removing the whole set before replaying any of it keeps one
    /// moved island from arranging against another's old position.
    ///
    /// # Errors
    /// [`SketchError::UnknownIsland`] for any stale island id;
    /// [`SketchError::PointOffPlane`] for an off-plane landing; any typed
    /// refusal of the replay (a tolerance-collapsing transform merging a
    /// moved edge's ends) propagates. Strong guarantee: untouched on `Err`.
    pub fn apply_transform_islands(
        &mut self,
        islands: &[SketchIslandId],
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        let mut island_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        for &island in islands {
            let isl = self.islands.get(island).ok_or(SketchError::UnknownIsland)?;
            island_edges.extend(isl.edges.iter().copied());
        }
        let mut island_verts: std::collections::BTreeSet<SketchVertexId> =
            std::collections::BTreeSet::new();
        for &eid in &island_edges {
            let e = self.edges[eid];
            island_verts.insert(e.from);
            island_verts.insert(e.to);
        }

        // Validate on a clone; swap in only once every check passes.
        let mut s = self.clone();
        for &v in &island_verts {
            let p = t.apply_point(s.vertices[v].position);
            if s.plane.signed_distance(p).abs() > tol::PLANE_DIST {
                return Err(SketchError::PointOffPlane { which: 0 });
            }
            s.vertices[v].position = p;
        }

        // Does the landing touch anything OUTSIDE the set? Crossings, vertex
        // merges, and within-tolerance grazes all count.
        let moved_segs: Vec<(Point3, Point3)> = island_edges
            .iter()
            .map(|&eid| {
                let e = s.edges[eid];
                (s.vertices[e.from].position, s.vertices[e.to].position)
            })
            .collect();
        let touches = s.edges.iter().any(|(oid, oe)| {
            if island_edges.contains(&oid) {
                return false;
            }
            let (r, w) = (s.vertices[oe.from].position, s.vertices[oe.to].position);
            moved_segs
                .iter()
                .any(|&(p, q)| crate::geom2d::segments_intersect(p, q, r, w))
        });

        if touches {
            // Arrange: remove the whole set, then replay it transformed
            // through the sticky machinery, exactly as a draw would land.
            let mut arranged = self.clone();
            arranged.remove_edges(&island_edges);
            self.replay_islands_into(islands, t, &mut arranged)?;
            *self = arranged;
            return Ok(());
        }

        // Clear landing: the vertices moved in place above. Curve geometry
        // rides along iff the whole chain moved (map-or-drop, see
        // apply_transform); a chain straddling the set's boundary — possible
        // after partial deletion split its edges across components — can no
        // longer be described by one circle, so its geometry drops.
        let scale = in_plane_similarity_scale(t, s.plane.normal());
        let curve_ids: Vec<SketchCurveId> = s.curves.keys().collect();
        for cid in curve_ids {
            if s.curves[cid].is_none() {
                continue;
            }
            let members: Vec<SketchEdgeId> = s
                .edges
                .iter()
                .filter(|(_, e)| e.curve == Some(cid))
                .map(|(id, _)| id)
                .collect();
            let inside = members.iter().filter(|e| island_edges.contains(e)).count();
            if inside == 0 {
                continue; // untouched chain
            }
            s.curves[cid] = match (s.curves[cid], scale, inside == members.len()) {
                (Some(a), Some(sc), true) => Some(CurveAnalytic {
                    geom: CurveGeom {
                        center: t.apply_point(a.geom.center),
                        radius: a.geom.radius * sc,
                    },
                    kind: a.kind,
                }),
                _ => None,
            };
        }

        // Containment may have changed even though nothing touched (a shape
        // moved into a closed loop becomes its hole; out of one, no longer):
        // re-derive regions — ids survive while outer cycles are unchanged —
        // and islands.
        let old_regions = s.region_snapshot();
        s.recompute_regions_with_diff(&old_regions);
        s.recompute_islands();

        *self = s;
        Ok(())
    }

    /// A NEW sketch holding this sketch's `islands` with `t` baked in, on
    /// `plane` (the source plane mapped through `t`; the caller computes it
    /// because plane mapping errors are transform-typed, not sketch-typed).
    /// This is the builder for the document layer's out-of-plane sketch COPY
    /// ([`Document::copy_sketch_islands`]) and the island DETACH
    /// ([`Document::transform_sketch_islands`]). Pure — `self` is untouched.
    ///
    /// Every listed island's edges are replayed through the ordinary sticky
    /// machinery ONTO ONE fresh sketch, so regions and islands re-derive
    /// exactly as they would for freshly drawn geometry (no hand-assembled
    /// topology). Keeping the whole set together is what preserves a region's
    /// HOLES: a hole loop is a separate connected component (its own island),
    /// so a donut's outer and inner boundaries must land on the SAME sketch or
    /// the ring would re-derive as a plain solid and the hole would be lost.
    /// An affine map preserves incidence and non-crossing, so a valid island
    /// set replays cleanly; anything the machinery still refuses (e.g. a
    /// tolerance-collapsing scale merging vertices) surfaces as that
    /// machinery's typed error, never a silent weld.
    ///
    /// Curve chains keep their identity: each source chain becomes one chain
    /// in the new sketch. Analytic [`CurveGeom`] maps (center through the
    /// point map, radius by the uniform factor) when `t` restricted to the
    /// SOURCE plane is a similarity, and is dropped otherwise — the
    /// map-or-drop contract of [`Sketch::apply_transform`]. A chain split
    /// across islands by earlier partial deletion forks: the members inside
    /// each island carry the mapped geometry into the new sketch (their
    /// facets still lie on the mapped circle), and the members left behind
    /// keep the source chain untouched.
    ///
    /// # Errors
    /// [`SketchError::UnknownIsland`] for any stale island id; any typed
    /// refusal of the replay ([`Sketch::add_segment`] /
    /// [`Sketch::begin_curve_with`]) propagates with nothing built.
    ///
    /// [`Document::transform_sketch_islands`]: crate::Document::transform_sketch_islands
    /// [`Document::copy_sketch_islands`]: crate::Document::copy_sketch_islands
    pub(crate) fn rebuild_islands_transformed(
        &self,
        islands: &[SketchIslandId],
        t: &crate::Transform,
        plane: Plane,
    ) -> Result<Sketch, SketchError> {
        // The new sketch is this one's kind: a detached shape's islands
        // detach or copy into a detached sketch.
        let mut fresh = self.blank_like_on(plane);
        self.replay_islands_into(islands, t, &mut fresh)?;
        Ok(fresh)
    }

    /// Replays this sketch's `islands`, with `t` baked in, into `target`
    /// through the ordinary sticky machinery ([`Sketch::add_segment`]), one
    /// curve bracket per source chain. The shared engine of
    /// [`Sketch::rebuild_islands_transformed`] (onto a fresh sketch) and the
    /// arranging arm of [`Sketch::apply_transform_islands`] (back onto this
    /// sketch, after removing the originals). `target` is left partially
    /// written on `Err`; callers pass a clone.
    fn replay_islands_into(
        &self,
        islands: &[SketchIslandId],
        t: &crate::Transform,
        target: &mut Sketch,
    ) -> Result<(), SketchError> {
        let mut edges: Vec<SketchEdgeId> = Vec::new();
        for &island in islands {
            let isl = self.islands.get(island).ok_or(SketchError::UnknownIsland)?;
            for &e in &isl.edges {
                if !edges.contains(&e) {
                    edges.push(e);
                }
            }
        }
        self.replay_edges_into(&edges, t, target)
    }

    /// A NEW sketch on `plane` holding exactly `edges` of this sketch with
    /// `t` baked in, replayed through the sticky machinery (one curve
    /// bracket per source chain, map-or-drop curve geometry). Pure. The
    /// builder for moving PART of an object-owned sketch with a pushed face.
    ///
    /// # Errors
    /// [`SketchError::UnknownEdge`] for a stale edge; any typed refusal of
    /// the replay propagates with nothing built.
    pub(crate) fn rebuild_edges_transformed(
        &self,
        edges: &[SketchEdgeId],
        t: &crate::Transform,
        plane: Plane,
    ) -> Result<Sketch, SketchError> {
        let mut fresh = self.blank_like_on(plane);
        self.replay_edges_into(edges, t, &mut fresh)?;
        Ok(fresh)
    }

    /// Replays `edges` of this sketch, with `t` baked in, into `target`
    /// through [`Sketch::add_segment`], one curve bracket per source chain
    /// (its members in `edges` order). Curve geometry maps when `t` restricted
    /// to this sketch's plane is a similarity and drops otherwise. `target` is
    /// left partially written on `Err`; callers pass a clone or a fresh sketch.
    fn replay_edges_into(
        &self,
        edges: &[SketchEdgeId],
        t: &crate::Transform,
        target: &mut Sketch,
    ) -> Result<(), SketchError> {
        if edges.iter().any(|e| !self.edges.contains_key(*e)) {
            return Err(SketchError::UnknownEdge);
        }
        let scale = in_plane_similarity_scale(t, self.plane.normal());
        let mut emitted: std::collections::BTreeSet<SketchCurveId> =
            std::collections::BTreeSet::new();
        let endpoints = |eid: SketchEdgeId| {
            let e = self.edges[eid];
            (
                t.apply_point(self.vertices[e.from].position),
                t.apply_point(self.vertices[e.to].position),
            )
        };
        let keep_faced = target.faced_rings();
        for &eid in edges {
            match self.edges[eid].curve {
                None => {
                    let (a, b) = endpoints(eid);
                    target.add_segment(a, b)?;
                }
                Some(cid) => {
                    if !emitted.insert(cid) {
                        continue; // this chain already replayed with its first edge
                    }
                    // One bracket per source chain, so the fresh edges are
                    // one selectable unit again.
                    match (self.curves.get(cid).copied().flatten(), scale) {
                        (Some(a), Some(sc)) => {
                            target.begin_curve_with_kind(
                                CurveGeom {
                                    center: t.apply_point(a.geom.center),
                                    radius: a.geom.radius * sc,
                                },
                                a.kind,
                            )?;
                        }
                        _ => {
                            target.begin_curve();
                        }
                    }
                    for &member in edges.iter().filter(|&&m| self.edges[m].curve == Some(cid)) {
                        let (a, b) = endpoints(member);
                        if let Err(e) = target.add_segment(a, b) {
                            target.end_curve();
                            return Err(e);
                        }
                    }
                    target.end_curve();
                }
            }
        }
        // A faceless loop replayed stays faceless (module docs, rule 6):
        // the region in `target` with its ring, carried through `t`, is the
        // same loop moved or copied, not one drawn. A loop `target` had
        // with a face before the replay keeps it.
        target.carry_faceless_from(self, &|p| t.apply_point(p), &keep_faced);
        Ok(())
    }

    /// Current closed regions (read-only). Kept up to date by every mutation;
    /// never recomputed lazily.
    pub fn regions(&self) -> &SlotMap<SketchRegionId, SketchRegion> {
        &self.regions
    }

    /// Make every closed region a loop with no face (module docs, rule 6):
    /// what an import brings in as bare edges, which bound no face in the
    /// source. Drawing over an edge of a loop gives it its face again.
    pub fn make_regions_faceless(&mut self) {
        for r in self.regions.values_mut() {
            r.faceless = true;
        }
    }

    /// Set whether one region is a loop with no face (module docs, rule 6).
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] for a stale handle.
    pub fn set_region_faceless(
        &mut self,
        region: SketchRegionId,
        faceless: bool,
    ) -> Result<(), SketchError> {
        let r = self
            .regions
            .get_mut(region)
            .ok_or(SketchError::UnknownRegion)?;
        r.faceless = faceless;
        Ok(())
    }

    /// Adds many segments at once, each through the same sticky rules as
    /// [`Sketch::add_segment`] and in the same order, so the result is
    /// exactly what adding them one by one builds. What differs is the
    /// cost: a segment is tested only against the edges in the cells of a
    /// grid it crosses (a scan of every edge per segment was quadratic,
    /// and a floor of thousands of coplanar faces lifted in minutes), a
    /// new end finds the vertex it merges with through cells too, no
    /// copy of the sketch is taken per segment, and the regions and
    /// islands are recomputed once at the end. The faces of a solid are
    /// the usual input: lifting a plane feeds every face edge on it here.
    ///
    /// # Errors
    /// The first segment the sticky rules refuse, as [`Sketch::add_segment`]
    /// refuses it; the sketch then holds the segments before it, the
    /// regions recomputed, and the caller decides (a lift discards it).
    pub fn add_segments(&mut self, segs: &[(Point3, Point3)]) -> Result<(), SketchError> {
        let curve = self.active_curve;
        let segs: Vec<(Point3, Point3, Option<SketchCurveId>)> =
            segs.iter().map(|&(a, b)| (a, b, curve)).collect();
        self.add_segments_on_curves(&segs)
    }

    /// [`Sketch::add_segments`] with each segment's curve given: what
    /// adding them one by one builds, each inside a bracket of its curve
    /// ([`Sketch::begin_curve`] … [`Sketch::end_curve`]) or outside any
    /// when it has none. The curves must exist; the open bracket, if any,
    /// is the same after as before.
    pub(crate) fn add_segments_on_curves(
        &mut self,
        segs: &[(Point3, Point3, Option<SketchCurveId>)],
    ) -> Result<(), SketchError> {
        let bracket = self.active_curve;
        let outcome = self.add_segments_on_curves_inner(segs);
        self.active_curve = bracket;
        outcome
    }

    fn add_segments_on_curves_inner(
        &mut self,
        segs: &[(Point3, Point3, Option<SketchCurveId>)],
    ) -> Result<(), SketchError> {
        // A coordinate that is not a number would be refused by the
        // insertion below, but the grid is built first, and a coordinate
        // of infinity would give it a span of every cell there is. Refuse
        // such a point before any of that, as the one-by-one path does.
        let finite = |p: Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
        for &(a, b, _) in segs {
            if !finite(a) {
                return Err(SketchError::PointOffPlane { which: 0 });
            }
            if !finite(b) {
                return Err(SketchError::PointOffPlane { which: 1 });
            }
        }
        let old_regions = self.region_snapshot();
        let normal = self.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);
        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };
        // A grid over the plane with about one segment per cell: cells of
        // the extent the input spans, divided by the root of the count.
        let mut lo = (f64::INFINITY, f64::INFINITY);
        let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut take = |p: Point3| {
            let q = proj(p);
            lo = (lo.0.min(q.0), lo.1.min(q.1));
            hi = (hi.0.max(q.0), hi.1.max(q.1));
        };
        for &(a, b, _) in segs {
            take(a);
            take(b);
        }
        for v in self.vertices.values() {
            take(v.position);
        }
        let n = (segs.len() + self.edges.len()).max(1) as f64;
        let extent = (hi.0 - lo.0).max(hi.1 - lo.1);
        // (An extent that overflows to infinity, from finite coordinates
        // of opposite signs near the largest number, gets a cell so wide
        // that every coordinate lands within a few cells of the origin:
        // the scan then covers nearly everything, as the one-by-one path
        // does, instead of spanning more cells than can be visited.)
        let cell = if extent.is_finite() && extent > 0.0 {
            (extent / n.sqrt()).max(tol::POINT_MERGE * 1e3)
        } else if extent > 0.0 {
            f64::MAX / 8.0
        } else {
            1.0
        };
        let cells_of = |a: (f64, f64), b: (f64, f64)| -> (i64, i64, i64, i64) {
            let eps = tol::POINT_MERGE;
            (
                ((a.0.min(b.0) - eps) / cell).floor() as i64,
                ((a.0.max(b.0) + eps) / cell).floor() as i64,
                ((a.1.min(b.1) - eps) / cell).floor() as i64,
                ((a.1.max(b.1) + eps) / cell).floor() as i64,
            )
        };
        let mut grid: std::collections::BTreeMap<(i64, i64), Vec<SketchEdgeId>> =
            std::collections::BTreeMap::new();
        let mut placed: std::collections::BTreeMap<SketchEdgeId, (i64, i64, i64, i64)> =
            std::collections::BTreeMap::new();
        let edge_span = |s: &Sketch, e: SketchEdgeId| -> (i64, i64, i64, i64) {
            let ed = s.edges[e];
            cells_of(
                proj(s.vertices[ed.from].position),
                proj(s.vertices[ed.to].position),
            )
        };
        let put = |grid: &mut std::collections::BTreeMap<(i64, i64), Vec<SketchEdgeId>>,
                   placed: &mut std::collections::BTreeMap<SketchEdgeId, (i64, i64, i64, i64)>,
                   e: SketchEdgeId,
                   span: (i64, i64, i64, i64)| {
            if placed.contains_key(&e) {
                return;
            }
            for x in span.0..=span.1 {
                for y in span.2..=span.3 {
                    grid.entry((x, y)).or_default().push(e);
                }
            }
            placed.insert(e, span);
        };
        let drop = |grid: &mut std::collections::BTreeMap<(i64, i64), Vec<SketchEdgeId>>,
                    placed: &mut std::collections::BTreeMap<SketchEdgeId, (i64, i64, i64, i64)>,
                    e: SketchEdgeId| {
            let Some(span) = placed.remove(&e) else {
                return;
            };
            for x in span.0..=span.1 {
                for y in span.2..=span.3 {
                    if let Some(list) = grid.get_mut(&(x, y)) {
                        list.retain(|&id| id != e);
                    }
                }
            }
        };
        let existing: Vec<SketchEdgeId> = self.edges.keys().collect();
        for e in existing {
            let span = edge_span(self, e);
            put(&mut grid, &mut placed, e, span);
        }
        // The vertices by cell, for the merge lookup.
        let mut vcells: std::collections::BTreeMap<[i64; 3], Vec<SketchVertexId>> =
            std::collections::BTreeMap::new();
        for (id, v) in &self.vertices {
            vcells.entry(vertex_cell(v.position)).or_default().push(id);
        }
        self.bulk_vertex_cells = Some(vcells);
        let mut outcome = Ok(());
        for &(a, b, curve) in segs {
            self.active_curve = curve;
            let span = cells_of(proj(a), proj(b));
            let mut scan: Vec<SketchEdgeId> = Vec::new();
            for x in span.0..=span.1 {
                for y in span.2..=span.3 {
                    if let Some(list) = grid.get(&(x, y)) {
                        scan.extend(list.iter().copied());
                    }
                }
            }
            scan.sort_unstable();
            scan.dedup();
            let report = match self.add_segment_inner_with(a, b, Some(&scan), true) {
                Ok(r) => r,
                Err(e) => {
                    outcome = Err(e);
                    break;
                }
            };
            for (old, fragments) in &report.split_edges {
                drop(&mut grid, &mut placed, *old);
                for &f in fragments {
                    let span = edge_span(self, f);
                    put(&mut grid, &mut placed, f, span);
                }
            }
            for &e in &report.new_edges {
                let span = edge_span(self, e);
                put(&mut grid, &mut placed, e, span);
            }
        }
        self.bulk_vertex_cells = None;
        self.recompute_regions_with_diff(&old_regions);
        self.recompute_islands();
        outcome
    }

    /// [`Sketch::add_segments`] for the executable specs.
    #[doc(hidden)]
    pub fn add_segments_for_test(&mut self, segs: &[(Point3, Point3)]) -> Result<(), SketchError> {
        self.add_segments(segs)
    }

    /// [`Sketch::add_segments_on_curves`] for the executable specs.
    #[doc(hidden)]
    pub fn add_segments_on_curves_for_test(
        &mut self,
        segs: &[(Point3, Point3, Option<SketchCurveId>)],
    ) -> Result<(), SketchError> {
        self.add_segments_on_curves(segs)
    }

    /// The outer rings, as positions, of every region that has a face.
    pub(crate) fn faced_rings(&self) -> Vec<Vec<Point3>> {
        self.regions
            .values()
            .filter(|r| !r.faceless)
            .map(|r| r.outer.iter().map(|&v| self.vertices[v].position).collect())
            .collect()
    }

    /// Make faceless every region whose outer ring is a faceless loop of
    /// `source`, each source position passed through `map` (module docs,
    /// rule 6: a loop replayed into this sketch, moved, copied or rebuilt,
    /// is the same loop, not one drawn). A ring in `keep_faced` — the loops
    /// this sketch had with a face before the replay — keeps its face: a
    /// wire moved exactly onto a face does not take the face away.
    pub(crate) fn carry_faceless_from(
        &mut self,
        source: &Sketch,
        map: &dyn Fn(Point3) -> Point3,
        keep_faced: &[Vec<Point3>],
    ) {
        let faceless: Vec<Vec<Point3>> = source
            .regions
            .values()
            .filter(|r| r.faceless)
            .map(|r| {
                r.outer
                    .iter()
                    .map(|&v| map(source.vertices[v].position))
                    .collect()
            })
            .collect();
        if faceless.is_empty() {
            return;
        }
        let vertices = &self.vertices;
        for r in self.regions.values_mut().filter(|r| !r.faceless) {
            let ring: Vec<Point3> = r.outer.iter().map(|&v| vertices[v].position).collect();
            if keep_faced.iter().any(|k| rings_coincide(k, &ring)) {
                continue;
            }
            if faceless.iter().any(|f| rings_coincide(f, &ring)) {
                r.faceless = true;
            }
        }
    }

    /// Every region with its rings as positions, taken before a mutation so
    /// [`Sketch::recompute_regions_with_diff`] can tell what a region that
    /// came out of it was before (its vertices may be gone by then).
    fn region_snapshot(&self) -> Vec<RegionSnapshot> {
        self.regions
            .iter()
            .map(|(id, r)| {
                // (A ring with a vertex already gone — a region left stale
                // between the two halves of one operation — has no shape
                // to compare, and reads as empty.)
                let ring = |vs: &[SketchVertexId]| -> Vec<Point3> {
                    vs.iter()
                        .map(|v| self.vertices.get(*v).map(|x| x.position))
                        .collect::<Option<Vec<_>>>()
                        .unwrap_or_default()
                };
                RegionSnapshot {
                    id,
                    region: r.clone(),
                    outer: ring(&r.outer),
                    holes: r.holes.iter().map(|h| ring(h)).collect(),
                }
            })
            .collect()
    }

    /// Bakes an affine `transform` into this sketch: every vertex position is
    /// moved by `transform` and the sketch `plane` is remapped (the
    /// inverse-transpose rule) so vertices stay coplanar. The 2D topology —
    /// vertices, edges, regions — is untouched, so all handles stay valid and
    /// no region is gained or lost. This is what move/rotate/scale on a
    /// free-standing sketch commit (Phase D), mirroring [`Object::apply_transform`].
    ///
    /// A reflection (determinant < 0) gives the plane the mapped normal
    /// ([`Transform::apply_plane_keeping_side`]) and reverses every
    /// region's stored cycles, because the map reverses their winding seen
    /// from that normal; region ids are kept.
    ///
    /// Validated up front so the mutation is transactional: a singular linear
    /// part is [`TransformError::Singular`], refused
    /// before any vertex moves, so the sketch is never left half-transformed.
    ///
    /// [`Object::apply_transform`]: crate::Object::apply_transform
    pub fn apply_transform(
        &mut self,
        transform: &crate::Transform,
    ) -> Result<(), crate::TransformError> {
        // Reject before mutating. `inverse()` fails iff the linear part is
        // singular.
        transform.inverse()?;
        // The plane first: an invertible but extremely anisotropic map can
        // still squash it to no direction, refused before anything moves.
        // The sketch's normal is its up (the side it extrudes toward and
        // the side a riding sketch shares with its face), so the plane maps
        // keeping its side: under a reflection the winding-following refit
        // would turn it over.
        let plane = transform.apply_plane_keeping_side(&self.plane)?;
        // Curve geometry maps when the transform is an in-plane similarity
        // (center through the point map, radius by the uniform factor) and is
        // DROPPED otherwise — a non-uniform in-plane scale turns the circle
        // into an ellipse the metadata cannot describe. Dropping metadata is
        // not geometry repair: the facets are untouched; the chain merely
        // stops claiming an analytic ancestry it no longer has
        // (the true-curves design). Chain identity always survives.
        let scale = in_plane_similarity_scale(transform, self.plane.normal());
        for analytic in self.curves.values_mut() {
            *analytic = match (*analytic, scale) {
                (Some(a), Some(s)) => Some(CurveAnalytic {
                    geom: CurveGeom {
                        center: transform.apply_point(a.geom.center),
                        radius: a.geom.radius * s,
                    },
                    kind: a.kind,
                }),
                _ => None,
            };
        }
        // Remap the plane, then move every vertex onto it. The cycles,
        // whose winding a reflection reverses about the kept normal, are
        // reversed below.
        let reflects = transform.determinant() < 0.0;
        self.plane = plane;
        for v in self.vertices.values_mut() {
            v.position = transform.apply_point(v.position);
        }
        if reflects {
            self.reverse_region_cycles();
            debug_assert!(
                self.regions_match_a_fresh_trace(),
                "a reflected sketch's stored regions must equal a fresh trace"
            );
        }
        Ok(())
    }

    /// Reverses every region's outer and hole cycles in place, keeping the
    /// region ids (the winding flip a reflection causes).
    fn reverse_region_cycles(&mut self) {
        for r in self.regions.values_mut() {
            r.outer.reverse();
            for h in &mut r.holes {
                h.reverse();
            }
        }
    }

    /// Whether the stored regions are exactly a fresh trace's (same outer
    /// cycles up to rotation, same orientation).
    fn regions_match_a_fresh_trace(&self) -> bool {
        let fresh = self.compute_all_regions();
        fresh.len() == self.regions.len()
            && fresh.iter().all(|f| {
                self.regions
                    .values()
                    .any(|r| cycles_equal(&r.outer, &f.outer))
            })
    }

    /// Inserts the segment `from -> to`, applying the sticky rules in the
    /// module docs, and reports exactly what changed.
    ///
    /// Line and rectangle tools are UI concerns that decompose into calls of
    /// this one primitive.
    ///
    /// # Errors
    /// [`SketchError::PointOffPlane`] if an endpoint is farther than
    /// [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the plane (`which` = 0 for `from`, 1 for
    /// `to`); [`SketchError::DegenerateSegment`] if the endpoints are within
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of each other *after* vertex merging.
    ///
    /// On error the sketch is unchanged (strong guarantee).
    pub fn add_segment(&mut self, from: Point3, to: Point3) -> Result<SegmentAdded, SketchError> {
        // Strong guarantee: work on a clone, validate everything that can fail,
        // then swap.
        let (from, to) = if self.snap_to_existing {
            (self.snapped_endpoint(from), self.snapped_endpoint(to))
        } else {
            (from, to)
        };
        let mut s = self.clone();
        if s.snap_to_existing {
            for p in [from, to] {
                s.split_circle_facet_at(p)?;
            }
        }
        let report = s.add_segment_inner(from, to)?;
        *self = s;
        Ok(report)
    }

    /// When `p` lies on a drawn circle or arc (on its exact circle) inside
    /// one facet's span but on none of its points, splits that facet at `p`
    /// into two chords of the same circle in the same chain, so drawing that
    /// ends on the curve (a quadrant, any point the curve's own snaps offer)
    /// meets it there rather than a sagitta outside it. A facet lying along
    /// a rigid line (see the field `rigid`) is never split so. Returns
    /// whether it split.
    fn split_circle_facet_at(&mut self, p: Point3) -> Result<bool, SketchError> {
        let m = tol::POINT_MERGE;
        let found = self.edges.iter().find_map(|(id, e)| {
            let c = e.curve?;
            let an = self.curve_analytic(c)?;
            if an.kind != SketchCurveKind::Circle {
                return None;
            }
            let g = an.geom;
            if ((p - g.center).length() - g.radius).abs() > tol::PLANE_DIST.max(g.radius * 1e-9) {
                return None;
            }
            let (a, b) = (self.vertices[e.from].position, self.vertices[e.to].position);
            if p.approx_eq(a, m) || p.approx_eq(b, m) {
                return None;
            }
            if self.rigid.iter().any(|&(r0, r1)| {
                crate::geom2d::point_near_segment(a, r0, r1, m)
                    && crate::geom2d::point_near_segment(b, r0, r1, m)
            }) {
                return None;
            }
            let ab = b - a;
            let len2 = ab.length_squared();
            if len2 <= 0.0 {
                return None;
            }
            // Inside the span the facet subtends: projects into the chord, on
            // the side of the center the chord's arc bulges to.
            let t = (p - a).dot(ab) / len2;
            let mid = a + ab * 0.5;
            (t > 0.0 && t < 1.0 && (mid - g.center).dot(p - g.center) > 0.0)
                .then_some((id, a, b, c))
        });
        let Some((e, a, b, c)) = found else {
            return Ok(false);
        };
        self.remove_edge_inner(e);
        let saved = self.active_curve;
        self.active_curve = Some(c);
        let added = self
            .add_segment_inner(a, p)
            .and_then(|_| self.add_segment_inner(p, b));
        self.active_curve = saved;
        added?;
        Ok(true)
    }

    /// Merges the two edges meeting at `v` into one when `v` is a straight
    /// pass-through: exactly two edges use it, they run on one line (within
    /// [`tol::POINT_MERGE`]) in one direction, and they belong to the same
    /// curve chain or to none. The merged edge keeps that chain. `Ok(false)`
    /// (unchanged) when `v` is not such a point. Strong guarantee.
    ///
    /// # Errors
    /// [`SketchError::UnknownVertex`] for a stale handle; any error the
    /// re-insertion raises (never for a true pass-through).
    pub(crate) fn dissolve_straight_vertex(
        &mut self,
        v: SketchVertexId,
    ) -> Result<bool, SketchError> {
        let p = self
            .vertices
            .get(v)
            .ok_or(SketchError::UnknownVertex)?
            .position;
        let incident: Vec<SketchEdgeId> = self
            .edges
            .iter()
            .filter(|(_, e)| e.from == v || e.to == v)
            .map(|(id, _)| id)
            .collect();
        let [e1, e2] = incident[..] else {
            return Ok(false);
        };
        let (d1, d2) = (self.edges[e1], self.edges[e2]);
        if d1.curve != d2.curve {
            return Ok(false);
        }
        let other = |e: SketchEdge| if e.from == v { e.to } else { e.from };
        let (a, c) = (
            self.vertices[other(d1)].position,
            self.vertices[other(d2)].position,
        );
        let ac = c - a;
        let len = ac.length();
        if len <= tol::POINT_MERGE
            || ac.cross(p - a).length() / len > tol::POINT_MERGE
            || (p - a).dot(c - p) <= 0.0
        {
            return Ok(false);
        }
        let mut s = self.clone();
        s.remove_edge_inner(e1);
        s.remove_edge_inner(e2);
        let saved = s.active_curve;
        s.active_curve = d1.curve;
        let added = s.add_segment_inner(a, c);
        s.active_curve = saved;
        added?;
        *self = s;
        Ok(true)
    }

    /// Removes one edge (the eraser tool), deleting vertices that become
    /// unused and dissolving/merging the regions it bounded.
    ///
    /// # Errors
    /// [`SketchError::UnknownEdge`] if the handle is stale. On error the
    /// sketch is unchanged.
    pub fn remove_edge(&mut self, edge: SketchEdgeId) -> Result<EdgeRemoved, SketchError> {
        if !self.edges.contains_key(edge) {
            return Err(SketchError::UnknownEdge);
        }
        let mut s = self.clone();
        let report = s.remove_edge_inner(edge);
        *self = s;
        Ok(report)
    }

    /// Rebuilds a drawn circle's facets at a new density, in place.
    ///
    /// The chain's identity and its analytic [`CurveGeom`] are the fixed
    /// points: `curve` still names the same chain afterwards and still
    /// describes the same exact circle. Only the facets change — the chords
    /// approximating that circle are discarded and `segments` fresh ones are
    /// built in their place, evenly spaced and starting at the angle the old
    /// ring started at, so a re-faceted circle does not visibly rotate under
    /// the user. Regions and islands re-derive through the ordinary sticky
    /// machinery, exactly as they would for freshly drawn geometry.
    ///
    /// This is the whole of "editing the segment count of an existing
    /// circle": it is a rebuild, not a setting for the next circle drawn.
    ///
    /// **Only a whole, untouched circle qualifies.** Re-faceting replaces
    /// every facet, so anything holding onto a particular facet would be
    /// silently broken by it. The chain must therefore be a
    /// [`SketchCurveKind::Circle`] whose live edges form ONE closed ring
    /// covering the full turn, every vertex of which is incident to exactly
    /// two edges and both of them members. A partially erased circle, an arc,
    /// a polygon, or a circle glued to other geometry (a T-junction, a
    /// rounded corner, a boundary shared with a neighbouring region) is
    /// refused rather than rebuilt (rule 4 — no silent repair).
    ///
    /// Note that a circle already consumed by an extrusion cannot reach this
    /// path at all: extrusion deletes the region's exclusive scaffolding
    /// ([`Sketch::region_scaffolding`]), so the chain has no live edges left
    /// and refuses here as [`SketchError::CurveNotRefacetable`]. Re-faceting
    /// a *solid's* cylindrical wall is a different operation entirely and is
    /// not this one.
    ///
    /// # Errors
    /// - [`SketchError::UnknownCurve`] — stale chain handle.
    /// - [`SketchError::CurveNotAnalytic`] — the chain carries no circle.
    /// - [`SketchError::CurveNotRefacetable`] — the chain is not a whole,
    ///   untouched circle (see above), or the rebuilt ring would interact
    ///   with geometry drawn since (a denser ring sits closer to the exact
    ///   circle, so it can reach a neighbour the old one did not): any split,
    ///   crossing or collinear overlap refuses the whole operation.
    /// - [`SketchError::SegmentsBelowFloor`] / [`SketchError::SegmentsAboveCap`]
    ///   — `segments` outside `[MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS]`.
    ///
    /// On any error the sketch is unchanged (strong guarantee).
    pub fn refacet_curve(
        &mut self,
        curve: SketchCurveId,
        segments: usize,
    ) -> Result<CurveRefaceted, SketchError> {
        let analytic = self
            .curves
            .get(curve)
            .copied()
            .ok_or(SketchError::UnknownCurve)?
            .ok_or(SketchError::CurveNotAnalytic)?;
        if analytic.kind != SketchCurveKind::Circle {
            return Err(SketchError::CurveNotRefacetable);
        }
        if segments < MIN_CIRCLE_SEGMENTS {
            return Err(SketchError::SegmentsBelowFloor);
        }
        if segments > MAX_CIRCLE_SEGMENTS {
            return Err(SketchError::SegmentsAboveCap);
        }
        let geom = analytic.geom;

        let members: Vec<SketchEdgeId> = self.curve_edges(curve);
        if members.is_empty() {
            return Err(SketchError::CurveNotRefacetable);
        }
        let member_set: std::collections::BTreeSet<SketchEdgeId> =
            members.iter().copied().collect();

        // --- Isolation: every vertex the chain touches must carry exactly
        //     this chain's two edges. That single test rules out both a
        //     junction with foreign geometry (degree > 2, or a neighbour of
        //     another owner) and an open end (degree 1) in one pass.
        let mut incidence: std::collections::BTreeMap<SketchVertexId, usize> =
            std::collections::BTreeMap::new();
        let mut foreign: std::collections::BTreeSet<SketchVertexId> =
            std::collections::BTreeSet::new();
        for (eid, e) in &self.edges {
            for v in [e.from, e.to] {
                if member_set.contains(&eid) {
                    *incidence.entry(v).or_insert(0) += 1;
                } else {
                    foreign.insert(v);
                }
            }
        }
        if incidence.len() != members.len() {
            // A closed ring has as many vertices as edges; anything else is
            // an open chain or several pieces.
            return Err(SketchError::CurveNotRefacetable);
        }
        for (&v, &deg) in &incidence {
            if deg != 2 || foreign.contains(&v) {
                return Err(SketchError::CurveNotRefacetable);
            }
        }

        // --- ONE ring, not several.
        //
        // Degree-2-everywhere only makes the member set a disjoint UNION of
        // cycles, and neither the on-circle test nor the full-turn test below
        // closes that gap. A ring whose edges are not all short chords can
        // cover the whole turn by itself: one long secant spanning a wide
        // gap contributes that gap's whole arc to the coverage, while the
        // circular segment BEHIND it stays empty. A second, wholly separate
        // ring inscribed inside that segment then shares no vertex with the
        // first, crosses none of its chords, adds only arcs the first already
        // claimed — and passes every other test here. Rebuilding would delete
        // it without a word, which is exactly the silent repair rule 4
        // forbids. So walk the member graph and require one component.
        {
            let mut adjacency: std::collections::BTreeMap<SketchVertexId, Vec<SketchEdgeId>> =
                std::collections::BTreeMap::new();
            for &eid in &members {
                let e = self.edges[eid];
                adjacency.entry(e.from).or_default().push(eid);
                adjacency.entry(e.to).or_default().push(eid);
            }
            let mut seen: std::collections::BTreeSet<SketchEdgeId> =
                std::collections::BTreeSet::new();
            seen.insert(members[0]);
            let mut frontier = vec![members[0]];
            while let Some(cur) = frontier.pop() {
                let e = self.edges[cur];
                for v in [e.from, e.to] {
                    for &n in &adjacency[&v] {
                        if seen.insert(n) {
                            frontier.push(n);
                        }
                    }
                }
            }
            if seen.len() != members.len() {
                return Err(SketchError::CurveNotRefacetable);
            }
        }

        // --- The ring must be the WHOLE circle, and actually lie on it.
        let axis = self.plane.normal();
        let (u, v) = plane_axes(axis);
        let angle_of = |vid: SketchVertexId| -> Option<f64> {
            let d = self.vertices[vid].position - geom.center;
            let radial = d - axis * d.dot(axis);
            if (radial.length() - geom.radius).abs() > tol::POINT_MERGE {
                return None; // not on the exact circle
            }
            if radial.length() <= tol::NORMALIZE_MIN_LENGTH {
                return None;
            }
            Some(radial.dot(v).atan2(radial.dot(u)))
        };
        let mut intervals: Vec<[f64; 2]> = Vec::with_capacity(members.len());
        for &eid in &members {
            let e = self.edges[eid];
            let (Some(a), Some(b)) = (angle_of(e.from), angle_of(e.to)) else {
                return Err(SketchError::CurveNotRefacetable);
            };
            let mut offset = b - a;
            while offset > std::f64::consts::PI {
                offset -= 2.0 * std::f64::consts::PI;
            }
            while offset <= -std::f64::consts::PI {
                offset += 2.0 * std::f64::consts::PI;
            }
            intervals.push([a.min(a + offset), a.max(a + offset)]);
        }
        // `None` from the shared merger IS "the full circle" — the same
        // notion `curve_rims` publishes, so the two agree by construction.
        if crate::topo::merge_angular_intervals(&intervals, geom.radius).is_some() {
            return Err(SketchError::CurveNotRefacetable);
        }

        // --- Start angle: the ring's SMALLEST vertex angle in the basis
        //     frame. The new ring is built from it, so it survives the
        //     rebuild and the circle does not appear to spin when the count
        //     changes.
        //
        //     Chosen geometrically rather than from the lowest-id member,
        //     which would be equally deterministic but not equally STABLE:
        //     edge ids depend on insertion history and on slotmap slot reuse,
        //     so the lowest-id edge of a rebuilt ring is not the one the ring
        //     starts at, and a circle re-faceted twice — or one whose ids
        //     were re-minted by a save/load — would drift a fraction of a
        //     facet each time. An angle read off the geometry drifts for
        //     nobody.
        let start = members
            .iter()
            .flat_map(|&eid| {
                let e = self.edges[eid];
                [angle_of(e.from), angle_of(e.to)]
            })
            .flatten()
            .fold(f64::INFINITY, f64::min);
        if !start.is_finite() {
            return Err(SketchError::CurveNotRefacetable);
        }

        let ring: Vec<Point3> = (0..segments)
            .map(|k| {
                let a = start + std::f64::consts::TAU * (k as f64) / (segments as f64);
                geom.center + u * (geom.radius * a.cos()) + v * (geom.radius * a.sin())
            })
            .collect();

        // --- Commit on a clone (strong guarantee).
        let mut s = self.clone();
        let regions_before: std::collections::BTreeSet<SketchRegionId> = s.regions.keys().collect();

        let mut removed_vertices: Vec<SketchVertexId> = Vec::new();
        for &eid in &members {
            s.edges.remove(eid);
        }
        for &vid in incidence.keys() {
            if !s.vertex_has_edges(vid) {
                s.vertices.remove(vid);
                removed_vertices.push(vid);
            }
        }
        // Bring regions/islands back in step with the now-smaller edge set
        // before replaying, so the sketch is never observed holding a region
        // whose boundary vertices are gone. (`add_segment_inner` recomputes
        // both itself, so this is about the state *between* the two halves.)
        let stale_regions = s.region_snapshot();
        s.recompute_regions_with_diff(&stale_regions);
        s.recompute_islands();

        // Replay the new facets through the ordinary sticky machinery under
        // this chain's bracket, so the chain keeps its id and any interaction
        // with geometry drawn since is SEEN rather than assumed away. A clean
        // insert is exactly one new edge and no split; anything else means the
        // ring met something, and the whole operation refuses (`restore_edges`
        // holds itself to the same bar).
        let saved_bracket = s.active_curve;
        s.active_curve = Some(curve);
        let mut new_edges: Vec<SketchEdgeId> = Vec::with_capacity(segments);
        for k in 0..segments {
            let report = s
                .add_segment_inner(ring[k], ring[(k + 1) % segments])
                .map_err(|_| SketchError::CurveNotRefacetable)?;
            if !report.split_edges.is_empty() || report.new_edges.len() != 1 {
                return Err(SketchError::CurveNotRefacetable);
            }
            new_edges.push(report.new_edges[0]);
        }
        s.active_curve = saved_bracket;
        if s.curve_edges(curve).len() != segments {
            return Err(SketchError::CurveNotRefacetable);
        }

        let new_vertices: Vec<SketchVertexId> = {
            let mut vs: std::collections::BTreeSet<SketchVertexId> =
                std::collections::BTreeSet::new();
            for &eid in &new_edges {
                let e = s.edges[eid];
                vs.insert(e.from);
                vs.insert(e.to);
            }
            vs.into_iter().collect()
        };

        // Region diff over the WHOLE operation: `add_segment_inner` keeps
        // regions current step by step, so the only honest report is the
        // before/after id difference (a region that appeared and vanished
        // mid-replay is in neither list, matching `RegionOffsetAdded`).
        let regions_after: std::collections::BTreeSet<SketchRegionId> = s.regions.keys().collect();
        let regions_created: Vec<SketchRegionId> =
            regions_after.difference(&regions_before).copied().collect();
        let regions_removed: Vec<SketchRegionId> =
            regions_before.difference(&regions_after).copied().collect();

        *self = s;
        Ok(CurveRefaceted {
            removed_edges: members,
            removed_vertices,
            new_edges,
            new_vertices,
            regions_created,
            regions_removed,
        })
    }

    /// Repositions vertex `v` to `new_pos`, dragging its incident edges with
    /// it while **preserving the 2D topology**: the same vertices, edges, and
    /// regions exist (and keep their handles) before and after — nothing
    /// splits, merges, or re-forms. This is Phase D's per-vertex edit (the user
    /// drags one sketch corner), the single-vertex analogue of
    /// [`Sketch::apply_transform`].
    ///
    /// Unlike [`Sketch::add_segment`], it never re-stitches geometry. A move
    /// that *would* require re-topologizing is refused (rule 4 — no silent
    /// repair); the caller decides what to do.
    ///
    /// # Errors
    /// - [`SketchError::UnknownVertex`] if `v` is stale.
    /// - [`SketchError::PointOffPlane`] (`which: 0`) if `new_pos` is farther
    ///   than [`tol::PLANE_DIST`] from the sketch plane (we don't project).
    /// - [`SketchError::DegenerateSegment`] if the move would collapse an
    ///   incident edge below [`tol::POINT_MERGE`].
    /// - [`SketchError::WouldRetopologize`] if after the move an incident edge
    ///   would cross or collinearly overlap another edge, or `v` would land on
    ///   another vertex (a merge).
    ///
    /// On any error the sketch is unchanged (strong guarantee). On success
    /// returns the vertex's **old** position, so the document layer can record
    /// an exact inverse for undo.
    pub fn move_vertex(
        &mut self,
        v: SketchVertexId,
        new_pos: Point3,
    ) -> Result<Point3, SketchError> {
        let old_pos = self
            .vertices
            .get(v)
            .ok_or(SketchError::UnknownVertex)?
            .position;

        if self.plane.signed_distance(new_pos).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 0 });
        }

        // Validate on a clone; swap in only once every check passes.
        let mut s = self.clone();
        s.vertices[v].position = new_pos;

        // The incident edges are the only geometry that changed.
        let incident: Vec<SketchEdgeId> = s
            .edges
            .iter()
            .filter(|(_, e)| e.from == v || e.to == v)
            .map(|(id, _)| id)
            .collect();

        // (a) No incident edge may collapse to (near) zero length. Checked
        //     before the merge guard so dropping a corner onto an *adjacent*
        //     corner reads as a degenerate segment, not a merge.
        for &eid in &incident {
            let e = s.edges[eid];
            if s.vertices[e.from]
                .position
                .approx_eq(s.vertices[e.to].position, tol::POINT_MERGE)
            {
                return Err(SketchError::DegenerateSegment);
            }
        }

        // (b) The moved vertex may not land on top of another vertex (a merge).
        if s.vertices
            .iter()
            .any(|(vid, vert)| vid != v && vert.position.approx_eq(new_pos, tol::POINT_MERGE))
        {
            return Err(SketchError::WouldRetopologize);
        }

        // (c) No incident edge may cross or collinearly overlap any other edge
        //     (that would demand a split). Reuse the same 2D arrangement math
        //     `add_segment` uses, treating each incident edge as the "new"
        //     segment. Edges that merely share an endpoint produce no event.
        let normal = s.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);
        let anchor = Point3::ORIGIN + normal * (-s.plane.signed_distance(Point3::ORIGIN));
        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };

        for &inc_id in &incident {
            let inc = s.edges[inc_id];
            let nf = proj(s.vertices[inc.from].position);
            let nt = proj(s.vertices[inc.to].position);
            for (oth_id, oth) in &s.edges {
                if oth_id == inc_id {
                    continue;
                }
                let ef = proj(s.vertices[oth.from].position);
                let et = proj(s.vertices[oth.to].position);
                let events =
                    seg_seg_intersections_2d(nf, nt, ef, et, &s.vertices, *oth, u_ax, v_ax, anchor);
                for ev in events {
                    let bad = match ev {
                        Intersection2D::Proper { .. }
                        | Intersection2D::NewEndpointOnExisting { .. }
                        | Intersection2D::ExistingEndpointOnNew { .. } => true,
                        Intersection2D::Collinear {
                            t_params_on_new,
                            s_params_on_existing,
                            ..
                        } => !t_params_on_new.is_empty() || !s_params_on_existing.is_empty(),
                    };
                    if bad {
                        return Err(SketchError::WouldRetopologize);
                    }
                }
            }
        }

        // (d) Topology is unchanged, so the region cycles still reference the
        //     same vertex ids and now read the moved geometry automatically.
        //     Belt-and-suspenders: every region must still be a valid profile
        //     (a reflex drag could in principle pinch a boundary). Surfaced as
        //     a typed error rather than silently accepted. A region whose
        //     rings touch at a shared vertex is a region the tracer makes on
        //     purpose (it is not extrudable as one piece, but the drag did not
        //     make it so: topology is unchanged), so it is no refusal here.
        for region in s.regions.keys().collect::<Vec<_>>() {
            match s.profile(region) {
                Ok(_) | Err(SketchError::RegionTouchesHole) => {}
                Err(e) => return Err(e),
            }
        }

        // Dragging a vertex of a curve chain deforms the chain away from its
        // drawn circle: the chains touching the moved vertex drop their
        // analytic geometry (map-or-drop contract; identity survives so the
        // chain still selects as a unit).
        for &eid in &incident {
            if let Some(cid) = s.edges[eid].curve {
                s.curves[cid] = None;
            }
        }

        *self = s;
        Ok(old_pos)
    }

    /// Drags vertex `v` to `new_pos` the way the same lines DRAWN there would
    /// land (unified-drafting decision 4). When the drag touches nothing
    /// ([`Sketch::move_vertex`] accepts it), the vertex moves in place and
    /// every handle stays valid. When it would land on another vertex, cross
    /// or touch another edge, or run along one, the incident edges are
    /// removed and replayed through [`Sketch::add_segment`] from `new_pos`:
    /// they weld, split, and merge exactly as freshly drawn lines, regions
    /// re-derive, and handles of the replayed geometry are fresh. Either
    /// way, each curve chain through `v` keeps its identity (its members stay
    /// one selectable unit) and drops its analytic circle, which a dragged
    /// facet no longer lies on (map-or-drop).
    ///
    /// # Errors
    /// [`SketchError::UnknownVertex`], [`SketchError::PointOffPlane`], and
    /// [`SketchError::DegenerateSegment`] (an incident edge collapsing onto
    /// its other end) exactly as [`Sketch::move_vertex`]; a pinching drag the
    /// in-place path refuses as an invalid profile is likewise refused.
    /// Strong guarantee: untouched on `Err`.
    pub fn move_vertex_arranging(
        &mut self,
        v: SketchVertexId,
        new_pos: Point3,
    ) -> Result<VertexMoved, SketchError> {
        match self.move_vertex(v, new_pos) {
            Ok(old_pos) => {
                return Ok(VertexMoved {
                    old_pos,
                    rearranged: false,
                });
            }
            Err(SketchError::WouldRetopologize) => {}
            Err(e) => return Err(e),
        }
        let old_pos = self.vertices[v].position;
        let incident: Vec<(SketchEdgeId, SketchEdge)> = self
            .edges
            .iter()
            .filter(|(_, e)| e.from == v || e.to == v)
            .map(|(id, e)| (id, *e))
            .collect();

        let mut s = self.clone();
        let doomed: std::collections::BTreeSet<SketchEdgeId> =
            incident.iter().map(|&(id, _)| id).collect();
        s.remove_edges(&doomed);
        let prior_active = s.active_curve;
        for &(_, e) in &incident {
            let (a, b) = if e.from == v {
                (new_pos, self.vertices[e.to].position)
            } else {
                (self.vertices[e.from].position, new_pos)
            };
            // Re-attach the replayed edge to its own chain, so the chain stays
            // one unit; its circle drops below.
            s.active_curve = e.curve;
            let added = s.add_segment_inner(a, b);
            s.active_curve = prior_active;
            added?;
        }
        for &(_, e) in &incident {
            if let Some(cid) = e.curve {
                s.curves[cid] = None;
            }
        }
        *self = s;
        Ok(VertexMoved {
            old_pos,
            rearranged: true,
        })
    }

    /// The scaffolding edges only `region` needs: the edges on its boundary
    /// (outer or hole loops) that lie on NO other region's boundary.
    /// Extrusion deletes exactly these (Model D,
    /// the sketch-solid-model design §4D): the region became the
    /// solid's base face, so its exclusive boundary leaves the sketch with
    /// it — while an edge shared with a surviving region stays (the
    /// neighbor must remain closed) and open chains are untouched. Pure
    /// query — no mutation.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale.
    pub fn region_scaffolding(
        &self,
        region: SketchRegionId,
    ) -> Result<std::collections::BTreeSet<SketchEdgeId>, SketchError> {
        if !self.regions.contains_key(region) {
            return Err(SketchError::UnknownRegion);
        }
        let mut own_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        let mut other_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        for (rid, r) in &self.regions {
            let loops = std::iter::once(&r.outer).chain(r.holes.iter());
            for lp in loops {
                for i in 0..lp.len() {
                    let a = lp[i];
                    let b = lp[(i + 1) % lp.len()];
                    if let Some(eid) = self.edge_between(a, b) {
                        if rid == region {
                            own_edges.insert(eid);
                        } else {
                            other_edges.insert(eid);
                        }
                    }
                }
            }
        }
        Ok(own_edges.difference(&other_edges).copied().collect())
    }

    /// The scaffolding edges only the `consumed` regions need — the set
    /// variant of [`Sketch::region_scaffolding`]: an edge dies iff it lies
    /// on a consumed region's boundary and on NO surviving region's
    /// boundary, so an edge shared by two consumed regions goes while an
    /// edge shared with a survivor stays. The load path uses this to honor
    /// a pre-v11 file's stored consumed index one final time
    /// (the sketch-solid-model design §6); ids not in this sketch are
    /// skipped. Pure query — no mutation.
    pub(crate) fn regions_scaffolding(
        &self,
        consumed: &std::collections::BTreeSet<SketchRegionId>,
    ) -> std::collections::BTreeSet<SketchEdgeId> {
        let mut consumed_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        let mut live_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        for (rid, r) in &self.regions {
            let loops = std::iter::once(&r.outer).chain(r.holes.iter());
            for lp in loops {
                for i in 0..lp.len() {
                    let a = lp[i];
                    let b = lp[(i + 1) % lp.len()];
                    if let Some(eid) = self.edge_between(a, b) {
                        if consumed.contains(&rid) {
                            consumed_edges.insert(eid);
                        } else {
                            live_edges.insert(eid);
                        }
                    }
                }
            }
        }
        consumed_edges.difference(&live_edges).copied().collect()
    }

    /// Captures the exact removal record for `edges` — the rows and doomed
    /// vertices [`Sketch::restore_edges`] needs to undo the removal
    /// slot-exactly. Pure query; call BEFORE [`Sketch::remove_edges`].
    pub(crate) fn scaffolding_removal(
        &self,
        edges: &std::collections::BTreeSet<SketchEdgeId>,
    ) -> RemovedScaffolding {
        let rows: Vec<(Point3, Point3, Option<SketchCurveId>)> = edges
            .iter()
            .filter_map(|&eid| self.edges.get(eid))
            .map(|e| {
                (
                    self.vertices[e.from].position,
                    self.vertices[e.to].position,
                    e.curve,
                )
            })
            .collect();
        // A vertex dies iff it has at least one incident edge and every
        // incident edge is being removed — the same criterion
        // `remove_edges`'s `vertex_has_edges` filter applies after the
        // fact. Slotmap iteration is ascending slot order.
        let mut incident: std::collections::BTreeMap<SketchVertexId, (usize, usize)> =
            std::collections::BTreeMap::new();
        for (eid, e) in &self.edges {
            let doomed = edges.contains(&eid);
            for vid in [e.from, e.to] {
                let entry = incident.entry(vid).or_insert((0, 0));
                entry.0 += 1;
                if doomed {
                    entry.1 += 1;
                }
            }
        }
        let vertices: Vec<Point3> = self
            .vertices
            .iter()
            .filter(|(vid, _)| matches!(incident.get(vid), Some(&(total, doomed)) if total > 0 && total == doomed))
            .map(|(_, v)| v.position)
            .collect();
        RemovedScaffolding { rows, vertices }
    }

    /// Re-inserts scaffolding an extrusion deleted — the undo half of
    /// [`Sketch::remove_edges`]. Each row is an edge as endpoint positions
    /// plus its curve-chain id; endpoints weld to existing vertices within
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) (the shared-wall case)
    /// and re-form regions/islands through the ordinary sticky machinery,
    /// merging with whatever the sketch holds NOW — never a whole-sketch
    /// snapshot, so edits made after the extrusion survive its undo.
    ///
    /// Slot exactness: the doomed vertices are re-created FIRST, in
    /// `removed.vertices`' recorded (ascending original-slot) order — each
    /// pops its exact original slot back off the free list, thanks to
    /// [`Sketch::remove_edges`]' descending frees. Only then are the edge
    /// rows replayed, in their own recorded (ascending original-edge-slot)
    /// order; every endpoint welds to an existing vertex (pre-created or
    /// survivor), so each row consumes exactly one edge slot — its own.
    /// The two orders are independent on purpose: neither can be derived
    /// from the other once any slot has ever been freed and reused
    /// (eraser deletions, earlier consumptions), which is what broke the
    /// previous rows-only replay.
    ///
    /// A pre-created vertex is skipped when the sketch ALREADY has a
    /// vertex (or an edge interior) within
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of its position —
    /// that only happens when something drew there after the extrusion,
    /// the merge case where slot exactness is out of scope anyway; the
    /// row replay then welds or refuses exactly as before.
    ///
    /// A row that cannot re-insert faithfully — it would split, cross, or
    /// collinearly overlap geometry drawn since (any sticky event beyond a
    /// clean single-edge insert with endpoint welds) — is
    /// [`SketchError::RestoreConflicts`]: the whole restore is refused and
    /// the sketch left untouched (strong guarantee). Curve-chain ids are
    /// re-applied when the chain still exists (chain entries outlive their
    /// edges), reconnecting the surviving analytic [`CurveGeom`].
    ///
    pub(crate) fn restore_edges(
        &mut self,
        removed: &RemovedScaffolding,
    ) -> Result<(), SketchError> {
        let mut s = self.clone();
        for &p in &removed.vertices {
            let occupied = s
                .vertices
                .iter()
                .any(|(_, v)| v.position.approx_eq(p, tol::POINT_MERGE))
                || s.edges.iter().any(|(_, e)| {
                    crate::geom2d::point_near_segment(
                        p,
                        s.vertices[e.from].position,
                        s.vertices[e.to].position,
                        tol::POINT_MERGE,
                    )
                });
            if !occupied {
                s.vertices.insert(SketchVertex { position: p });
            }
        }
        for &(a, b, curve) in &removed.rows {
            let report = s
                .add_segment_inner(a, b)
                .map_err(|_| SketchError::RestoreConflicts)?;
            if !report.split_edges.is_empty() || report.new_edges.len() != 1 {
                return Err(SketchError::RestoreConflicts);
            }
            // The slot correspondence rests on every DOOMED endpoint
            // welding to its pre-created vertex. A row that instead
            // CREATES a vertex at a recorded doomed position means the
            // pre-creation above was skipped (its occupancy guard is
            // deliberately conservative — metric distance, while the
            // sticky split filters are parametric, so the two can
            // disagree in the sub-tolerance band of a long surviving
            // edge): the pops are already desynchronized, and committing
            // would silently permute slots. Refuse typed instead — the
            // documented outcome for any restore that cannot be faithful.
            // A created vertex at a NON-doomed position is the ordinary
            // merge case (a surviving endpoint erased since the
            // extrusion) and stays allowed: pre-creation completed before
            // any row ran, so the doomed set's slots are already exact.
            if report
                .new_vertices
                .iter()
                .any(|&vid| removed.vertices.contains(&s.vertices[vid].position))
            {
                return Err(SketchError::RestoreConflicts);
            }
            let eid = report.new_edges[0];
            if let Some(cid) = curve
                && s.curves.contains_key(cid)
            {
                s.edges[eid].curve = Some(cid);
            }
        }
        *self = s;
        Ok(())
    }

    /// The edge whose endpoints coincide with `a` and `b` (either
    /// orientation, within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE)),
    /// or `None`. Sticky rules forbid coincident duplicate edges, so a
    /// match is unique. The extrusion REDO path uses this to re-delete
    /// scaffolding by geometry: an interleaved gesture undo/redo restores
    /// snapshots carrying the outline's ORIGINAL edge ids, so a stored id
    /// set can go stale while the geometry itself is exact.
    ///
    /// Public (not `pub(crate)`) because `crates/api`'s sketch-edge
    /// locator (docs/agents/HEW_API.md §5.2) reuses this exact geometry match for
    /// its by-two-endpoints form, rather than duplicating the tolerant
    /// endpoint comparison at the API boundary — a read-only accessor, no
    /// new kernel behavior.
    pub fn edge_at_positions(&self, a: Point3, b: Point3) -> Option<SketchEdgeId> {
        self.edges
            .iter()
            .find(|(_, e)| {
                let f = self.vertices[e.from].position;
                let t = self.vertices[e.to].position;
                (f.approx_eq(a, tol::POINT_MERGE) && t.approx_eq(b, tol::POINT_MERGE))
                    || (f.approx_eq(b, tol::POINT_MERGE) && t.approx_eq(a, tol::POINT_MERGE))
            })
            .map(|(id, _)| id)
    }

    /// Removes a set of edges in one pass — the extrusion-consumption path
    /// (Model D): the batch analogue of [`Sketch::remove_edge`]. Vertices
    /// left without any incident edge are deleted; regions and islands
    /// recompute once at the end. Stale ids are skipped. Curve-chain
    /// identity survives on any remaining member edges, and a chain's
    /// analytic [`CurveGeom`] stays valid — deletion removes facets, it
    /// never deforms the ones that remain.
    ///
    /// ## Freed slots go back in DESCENDING id order
    ///
    /// `edges` and the derived `touched` vertex set are both freed
    /// highest-id-first, not in their natural ascending iteration order.
    /// This is load-bearing for undo byte-identity, not cosmetic:
    ///
    /// `slotmap::SlotMap::remove` pushes each freed slot onto an internal
    /// free list that the next `insert()` pops LIFO (most-recently-freed
    /// slot first). Freeing the doomed subset in DESCENDING slot order
    /// therefore makes the next insertions pop exactly that subset back in
    /// ASCENDING slot order — the n most recently pushed slots pop first,
    /// smallest first, before any older free-list residue from earlier
    /// deletions. [`Sketch::restore_edges`] leans on precisely that: it
    /// re-creates the doomed vertices in recorded ascending-original-slot
    /// order (each pops its own slot back), then replays the edge rows in
    /// recorded ascending-original-edge-slot order (each welds both
    /// endpoints and so consumes exactly one edge slot — its own).
    ///
    /// The two recorded orders are deliberately independent. Ascending
    /// slot-id order is NOT creation order once any slot has ever been
    /// freed and reused (an eraser deletion or an earlier consumption
    /// followed by new drawing puts a newer edge in an older slot), so no
    /// single row-replay order can satisfy both the edge and the vertex
    /// correspondence at once; [`RemovedScaffolding`] records each side in
    /// its own slot order instead ([`Sketch::scaffolding_removal`]).
    ///
    /// This holds for PARTIAL consumption too: the doomed ids are a
    /// non-contiguous subset, but the LIFO argument above is about the
    /// freed subset alone, regardless of what surviving (never-freed) ids
    /// are interleaved between them. A restored row's endpoint that welds
    /// to a SURVIVING vertex consumes no free slot, but it also was never
    /// freed — it is simply unaffected, on either side. (A restore that
    /// instead welds to a vertex created by DIFFERENT geometry drawn after
    /// the extrusion — not a survivor of this same consumption — is the
    /// pre-existing merge behavior of `restore_edges`; slot exactness is
    /// out of scope there, since the sketch's contents already differ.)
    pub(crate) fn remove_edges(&mut self, edges: &std::collections::BTreeSet<SketchEdgeId>) {
        let old_regions = self.region_snapshot();
        let mut touched: std::collections::BTreeSet<SketchVertexId> =
            std::collections::BTreeSet::new();
        for &eid in edges.iter().rev() {
            if let Some(e) = self.edges.remove(eid) {
                touched.insert(e.from);
                touched.insert(e.to);
            }
        }
        for vid in touched.into_iter().rev() {
            if !self.vertex_has_edges(vid) {
                self.vertices.remove(vid);
            }
        }
        self.recompute_regions_with_diff(&old_regions);
        self.recompute_islands();
    }

    /// Extracts a region as a validated [`Profile`] ready for extrusion.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale (regions die
    /// whenever a mutation reshapes them — always re-query after mutating);
    /// [`SketchError::RegionHasNoFace`] for a loop with no face;
    /// [`SketchError::RegionTouchesHole`] if two of its rings share a vertex
    /// (a hole touching the outer ring or another hole at a point);
    /// [`SketchError::MalformedRegion`] if the rings form no valid profile.
    pub fn profile(&self, region: SketchRegionId) -> Result<Profile, SketchError> {
        let r = self.regions.get(region).ok_or(SketchError::UnknownRegion)?;
        if r.faceless {
            return Err(SketchError::RegionHasNoFace);
        }

        // Outer boundary: CCW as stored — map vertex ids to positions.
        let outer: Vec<Point3> = r
            .outer
            .iter()
            .map(|&vid| self.vertices[vid].position)
            .collect();

        // Holes: CW as stored.
        let holes: Vec<Vec<Point3>> = r
            .holes
            .iter()
            .map(|h| h.iter().map(|&vid| self.vertices[vid].position).collect())
            .collect();

        // Rings sharing a vertex (a hole touching the outer ring or another
        // hole at a point) make a face but no single extrudable profile:
        // refused for what it is, not as a tracing failure.
        let rings: Vec<&Vec<SketchVertexId>> =
            std::iter::once(&r.outer).chain(r.holes.iter()).collect();
        if (0..rings.len())
            .any(|i| (i + 1..rings.len()).any(|j| rings[i].iter().any(|v| rings[j].contains(v))))
        {
            return Err(SketchError::RegionTouchesHole);
        }

        // A region produced by the pipeline should always yield a valid
        // Profile; if it does not, that is a kernel bug — but surface it as a
        // typed error (rule 4) rather than panicking, since a panic at the
        // WASM boundary leaves the Scene unusable.
        let mut profile =
            Profile::new(self.plane, outer, holes).map_err(|_| SketchError::MalformedRegion)?;

        // Analytic attribution: for each boundary edge, the circle its curve
        // chain carries (when it carries one). This is what extrusion stamps
        // onto side-wall faces (the true-curves design).
        let edge_geom = |a: SketchVertexId, b: SketchVertexId| -> Option<CurveGeom> {
            self.edge_between(a, b)
                .and_then(|eid| self.edges[eid].curve)
                .and_then(|cid| self.curve_geom(cid))
        };
        let loop_geoms = |cycle: &[SketchVertexId]| -> Vec<Option<CurveGeom>> {
            (0..cycle.len())
                .map(|k| edge_geom(cycle[k], cycle[(k + 1) % cycle.len()]))
                .collect()
        };
        let outer_curves = loop_geoms(&r.outer);
        let hole_curves: Vec<Vec<Option<CurveGeom>>> =
            r.holes.iter().map(|h| loop_geoms(h)).collect();
        profile.set_curve_attribution(outer_curves, hole_curves);
        Ok(profile)
    }

    /// The unsigned area of `region`'s outer boundary, in m². Hole areas are
    /// NOT subtracted — pickers use this to rank nested containing regions
    /// (smallest outer wins), where the raw outer extent is the right measure.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale.
    pub fn region_area(&self, region: SketchRegionId) -> Result<f64, SketchError> {
        let r = self.regions.get(region).ok_or(SketchError::UnknownRegion)?;
        let pts: Vec<Point3> = r
            .outer
            .iter()
            .map(|&vid| self.vertices[vid].position)
            .collect();
        Ok(signed_area_on_plane(&pts, self.plane.normal()).abs())
    }

    /// Whether `p` (a point on the sketch plane) lies in `region`'s material:
    /// inside its outer boundary and outside every hole. Boundary-exact hits
    /// follow [`point_inside_polygon`]'s convention. Pure query.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale.
    pub fn region_contains_point(
        &self,
        region: SketchRegionId,
        p: Point3,
    ) -> Result<bool, SketchError> {
        let r = self.regions.get(region).ok_or(SketchError::UnknownRegion)?;
        let normal = self.plane.normal();
        let outer: Vec<Point3> = r
            .outer
            .iter()
            .map(|&vid| self.vertices[vid].position)
            .collect();
        if !point_inside_polygon(p, &outer, normal) {
            return Ok(false);
        }
        for hole in &r.holes {
            let pts: Vec<Point3> = hole
                .iter()
                .map(|&vid| self.vertices[vid].position)
                .collect();
            if point_inside_polygon(p, &pts, normal) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The Offset tool's sketch commit: computes the uniform offset of
    /// `region`'s entire boundary (outer loop and every hole) by `distance`
    /// — positive grows the material, negative shrinks it (see
    /// [`crate::offset::offset_profile`] for the geometry contract) — and
    /// inserts the offset loops into this sketch through the ordinary sticky
    /// machinery, so they weld, split, and close regions exactly as drawn
    /// geometry does. Both the original and the offset regions remain
    /// extrudable.
    ///
    /// Analytic arc runs of the boundary come back as true curves: each
    /// maximal run of offset edges claiming one circle is committed inside a
    /// [`Sketch::begin_curve_with`] bracket carrying the concentric offset
    /// circle, so the result selects as one curve and extrudes to a stamped
    /// cylinder wall, exactly like a freshly drawn arc.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] for a stale handle;
    /// [`SketchError::MalformedRegion`] if the region cannot produce a
    /// profile (a kernel bug surfaced typed); [`SketchError::OffsetTooSmall`]
    /// and [`SketchError::OffsetCollapsed`] per
    /// [`crate::offset::OffsetError`]. An offset loop that cannot insert
    /// cleanly is also [`SketchError::OffsetCollapsed`]. On any error the
    /// sketch is unchanged (strong guarantee).
    pub fn offset_region(
        &mut self,
        region: SketchRegionId,
        distance: f64,
    ) -> Result<RegionOffsetAdded, SketchError> {
        let profile = self.profile(region)?;
        let off = crate::offset::offset_profile(&profile, distance).map_err(|e| match e {
            crate::offset::OffsetError::OffsetTooSmall => SketchError::OffsetTooSmall,
            crate::offset::OffsetError::OffsetCollapsed => SketchError::OffsetCollapsed,
        })?;

        // Insert on a clone; swap only when every loop lands (strong
        // guarantee). Region identity is diffed once over the whole batch so
        // transient mid-insertion regions appear in neither report list.
        let mut s = self.clone();
        let before: std::collections::BTreeSet<SketchRegionId> = s.regions.keys().collect();
        let mut report = RegionOffsetAdded::default();
        for lp in std::iter::once(&off.outer).chain(off.holes.iter()) {
            s.insert_offset_loop(lp, &mut report)?;
        }
        let after: std::collections::BTreeSet<SketchRegionId> = s.regions.keys().collect();
        report.regions_created = after.difference(&before).copied().collect();
        report.regions_removed = before.difference(&after).copied().collect();

        *self = s;
        Ok(report)
    }

    /// Inserts one offset loop's edges, bracketing each maximal analytic arc
    /// run in a curve chain carrying its offset circle. Called on the
    /// offset's working clone; any sticky failure aborts the whole offset as
    /// [`SketchError::OffsetCollapsed`].
    fn insert_offset_loop(
        &mut self,
        lp: &crate::offset::OffsetLoop,
        report: &mut RegionOffsetAdded,
    ) -> Result<(), SketchError> {
        let n = lp.points.len();
        // Start at a run boundary so a curve bracket never wraps the seam;
        // a uniform loop (a full circle, or all straight) has none and any
        // start works.
        let start = (0..n)
            .find(|&k| lp.curves[(k + n - 1) % n] != lp.curves[k])
            .unwrap_or(0);

        let mut k = 0;
        while k < n {
            let geom = lp.curves[(start + k) % n];
            let mut len = 1;
            while k + len < n && lp.curves[(start + k + len) % n] == geom {
                len += 1;
            }
            if let Some(g) = geom {
                let cid = self
                    .begin_curve_with(g)
                    .map_err(|_| SketchError::OffsetCollapsed)?;
                report.new_curves.push(cid);
            }
            for j in 0..len {
                let i = (start + k + j) % n;
                let seg = self
                    .add_segment_inner(lp.points[i], lp.points[(i + 1) % n])
                    .map_err(|_| SketchError::OffsetCollapsed)?;
                report.new_edges.extend(seg.new_edges);
            }
            self.end_curve();
            k += len;
        }
        Ok(())
    }
}

/// Content equality: same plane, same `detached` flag, and identical
/// vertex/edge/region storage, **including handle identity** (same `SlotMap`
/// keys). Gesture undo snapshots (see `Document::end_sketch_gesture`) rely
/// on the handle-identity part — restoring a snapshot must keep every
/// previously-issued id valid — so plain positional equality would be too
/// weak here.
impl PartialEq for Sketch {
    fn eq(&self, other: &Sketch) -> bool {
        fn slotmaps_eq<K: slotmap::Key, V: PartialEq>(
            a: &SlotMap<K, V>,
            b: &SlotMap<K, V>,
        ) -> bool {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
        }
        self.plane == other.plane
            && self.detached == other.detached
            && slotmaps_eq(&self.vertices, &other.vertices)
            && slotmaps_eq(&self.edges, &other.edges)
            && slotmaps_eq(&self.regions, &other.regions)
            && slotmaps_eq(&self.curves, &other.curves)
    }
}

/// The uniform scale factor of `transform` restricted to the plane with unit
/// `normal`, or `None` if the restriction is not a similarity (it maps the
/// plane's circles to ellipses). Both in-plane basis directions must map to
/// equal lengths and stay orthogonal, within [`tol::NORMAL_DIRECTION`]
/// (dimensionless, applied relatively).
fn in_plane_similarity_scale(transform: &crate::Transform, normal: Vec3) -> Option<f64> {
    let (u, v) = plane_axes(normal);
    let lu = transform.apply_vector(u);
    let lv = transform.apply_vector(v);
    let (a, b) = (lu.length(), lv.length());
    let max = a.max(b);
    if max < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    if (a - b).abs() > tol::NORMAL_DIRECTION * max {
        return None;
    }
    if lu.dot(lv).abs() > tol::NORMAL_DIRECTION * a * b {
        return None;
    }
    Some(a)
}

// ═══════════════════════════════════════════════════════════════════ internals

impl Sketch {
    /// Core implementation of `add_segment`. Called on a clone so the caller
    /// can enforce the strong guarantee.
    fn add_segment_inner(&mut self, from: Point3, to: Point3) -> Result<SegmentAdded, SketchError> {
        self.add_segment_inner_with(from, to, None, false)
    }

    /// [`Sketch::add_segment_inner`] with its scans narrowed by a bulk
    /// caller ([`Sketch::add_segments`]): `scan` is the existing edges the
    /// segment can touch, in id order (every edge otherwise), the merge
    /// vertex lookup goes through `self.bulk_vertex_cells` when set, and
    /// `defer_regions` skips the region and island recompute (the caller
    /// does it once at the end; the report then lists no regions).
    fn add_segment_inner_with(
        &mut self,
        from: Point3,
        to: Point3,
        scan: Option<&[SketchEdgeId]>,
        defer_regions: bool,
    ) -> Result<SegmentAdded, SketchError> {
        // (A coordinate that is not a number is off every plane; the
        // distance test below would let it through, as nothing compares
        // greater than a tolerance when one side is not a number.)
        let finite = |p: Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
        if !finite(from) {
            return Err(SketchError::PointOffPlane { which: 0 });
        }
        if !finite(to) {
            return Err(SketchError::PointOffPlane { which: 1 });
        }
        // ── Step 1: validate planarity ────────────────────────────────────────
        if self.plane.signed_distance(from).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 0 });
        }
        if self.plane.signed_distance(to).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 1 });
        }

        // ── Step 2: snap endpoints to existing vertices (rule 1) ─────────────
        // Returns the snapped position and whether a new vertex must be created.
        let snap_from = self.find_or_plan_vertex(from);
        let snap_to = self.find_or_plan_vertex(to);

        // After snapping, check for degeneracy.
        let from_pos = snap_from.map_or(from, |id| self.vertices[id].position);
        let to_pos = snap_to.map_or(to, |id| self.vertices[id].position);

        if from_pos.approx_eq(to_pos, tol::POINT_MERGE) {
            return Err(SketchError::DegenerateSegment);
        }

        // ── Step 3: gather intersections ──────────────────────────────────────
        // Collect all split events for existing edges and for the new segment.
        // We work in 2D (plane-projected) coordinates throughout.
        let normal = self.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);
        // A point on the plane: 2D projections drop the out-of-plane component,
        // so reconstructing a 3D position must add it back via this anchor (the
        // plane need not pass through the origin).
        let anchor = Point3::ORIGIN + normal * (-self.plane.signed_distance(Point3::ORIGIN));

        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };

        let fp = proj(from_pos);
        let tp = proj(to_pos);

        // For each existing edge, find intersection parameters t (along new
        // segment, 0..=1) and s (along existing edge, 0..=1).
        // We'll collect:
        //   - for existing edges: the set of interior split parameters s
        //   - for the new segment: the set of interior split parameters t

        // Represent intersections as t-parameters along the new segment plus
        // the associated position and vertex id (to be created or reused).
        // Also track which existing edges need to be split at which s-params.
        struct ExistingEdgeSplit {
            edge_id: SketchEdgeId,
            // (s_param, position) for each interior split point on this edge.
            splits: Vec<(f64, Point3)>,
            // The edge's length, to read its parameters metrically.
            len: f64,
        }

        // Parameters are read METRICALLY: `t` on a segment of length `len` is
        // strictly interior iff it lies more than `POINT_MERGE` metres from
        // both ends along it, and two parameters coincide iff they are less
        // than `POINT_MERGE` metres apart.
        let interior =
            |t: f64, len: f64| t * len > tol::POINT_MERGE && (1.0 - t) * len > tol::POINT_MERGE;
        let new_len = ((tp.0 - fp.0).powi(2) + (tp.1 - fp.1).powi(2)).sqrt();

        let mut existing_splits: Vec<ExistingEdgeSplit> = Vec::new();
        // t-params along the new segment where we need to insert a vertex
        // (includes 0.0 and 1.0 from snap above, and interior crossings).
        let mut new_seg_t_params: Vec<f64> = vec![0.0, 1.0];

        let existing_edge_ids: Vec<SketchEdgeId> = match scan {
            Some(ids) => ids.to_vec(),
            None => self.edges.keys().collect(),
        };

        for eid in existing_edge_ids {
            let edge = self.edges[eid];
            let ep = proj(self.vertices[edge.from].position);
            let eq_ = proj(self.vertices[edge.to].position);

            let mut edge_splits: Vec<(f64, Point3)> = Vec::new();
            let ext_len = ((eq_.0 - ep.0).powi(2) + (eq_.1 - ep.1).powi(2)).sqrt();

            // Find all intersections between the new segment [fp,tp] and this
            // existing edge [ep,eq_].
            let intersections =
                seg_seg_intersections_2d(fp, tp, ep, eq_, &self.vertices, edge, u_ax, v_ax, anchor);

            for isect in intersections {
                match isect {
                    Intersection2D::Proper {
                        t_new,
                        s_existing,
                        point,
                    } => {
                        // Interior crossing: splits both segments.
                        if interior(t_new, new_len) {
                            new_seg_t_params.push(t_new);
                        }
                        if interior(s_existing, ext_len) {
                            edge_splits.push((s_existing, point));
                        }
                    }
                    Intersection2D::NewEndpointOnExisting { s_existing, point } => {
                        // New segment endpoint lands on interior of existing edge.
                        if interior(s_existing, ext_len) {
                            edge_splits.push((s_existing, point));
                        }
                    }
                    Intersection2D::ExistingEndpointOnNew { t_new, .. } => {
                        // Existing endpoint lands on interior of new segment.
                        if interior(t_new, new_len) {
                            new_seg_t_params.push(t_new);
                        }
                    }
                    Intersection2D::Collinear {
                        t_params_on_new,
                        s_params_on_existing,
                        points_on_new,
                        points_on_existing,
                    } => {
                        // Rule 4: collinear overlap.
                        for t in t_params_on_new {
                            if interior(t, new_len) {
                                new_seg_t_params.push(t);
                            }
                        }
                        let _ = points_on_new; // used above via t_params_on_new
                        for (s, p) in s_params_on_existing.into_iter().zip(points_on_existing) {
                            if interior(s, ext_len) {
                                edge_splits.push((s, p));
                            }
                        }
                    }
                }
            }

            if !edge_splits.is_empty() {
                existing_splits.push(ExistingEdgeSplit {
                    edge_id: eid,
                    splits: edge_splits,
                    len: ext_len,
                });
            }
        }

        // ── Step 4: snapshot old regions for identity diffing ─────────────────
        let old_regions = if defer_regions {
            Vec::new()
        } else {
            self.region_snapshot()
        };

        // ── Step 5: materialise vertices for the new segment endpoints ─────────
        let mut report = SegmentAdded::default();

        // from vertex
        let from_vid = match snap_from {
            Some(id) => id,
            None => {
                // Check one more time for snapping to an edge interior
                // (the snapping check at step 1 only checked existing vertices).
                // If still not found, create a fresh vertex.
                let vid = self.vertices.insert(SketchVertex { position: from_pos });
                report.new_vertices.push(vid);
                self.note_bulk_vertex(vid);
                vid
            }
        };

        // to vertex
        let to_vid = match snap_to {
            Some(id) => id,
            None => {
                let vid = self.vertices.insert(SketchVertex { position: to_pos });
                report.new_vertices.push(vid);
                self.note_bulk_vertex(vid);
                vid
            }
        };

        // ── Step 6: split existing edges at their interior intersection points ──
        for esplit in existing_splits {
            // Deduplicate and sort split points by s param.
            let mut splits = esplit.splits;
            let len = esplit.len;
            splits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            splits.dedup_by(|a, b| (a.0 - b.0).abs() * len < tol::POINT_MERGE);

            // Remove zero-length fragments: skip if s is at 0 or 1 (already
            // covered by snapping).
            let splits: Vec<(f64, Point3)> = splits
                .into_iter()
                .filter(|(s, _)| interior(*s, len))
                .collect();

            if splits.is_empty() {
                continue;
            }

            let old_edge = self.edges[esplit.edge_id];
            let old_from_vid = old_edge.from;
            let old_to_vid = old_edge.to;

            // Create or find vertices at each split point.
            let mut split_vids: Vec<SketchVertexId> = Vec::new();
            for (_, pos) in &splits {
                let vid = self.find_or_create_vertex(*pos, &mut report.new_vertices);
                split_vids.push(vid);
            }

            // Build the chain of fragment vertices: old_from -> split[0] -> ... -> old_to
            let mut chain: Vec<SketchVertexId> = vec![old_from_vid];
            chain.extend_from_slice(&split_vids);
            chain.push(old_to_vid);

            // Remove the old edge — its fragments inherit its curve chain,
            // so a split arc facet stays part of its arc.
            let split_curve = self.edges[esplit.edge_id].curve;
            self.edges.remove(esplit.edge_id);

            // Insert fragment edges.  Fragments of split *existing* edges go into
            // split_edges only, not new_edges (new_edges is for the inserted segment
            // fragments).
            let mut fragments: Vec<SketchEdgeId> = Vec::new();
            for w in chain.windows(2) {
                let frag_id = self.edges.insert(SketchEdge {
                    from: w[0],
                    to: w[1],
                    curve: split_curve,
                });
                fragments.push(frag_id);
            }
            report.split_edges.push((esplit.edge_id, fragments));
        }

        // ── Step 7: collect t-params along the new segment, materialise verts ──
        // Deduplicate and sort.
        new_seg_t_params.sort_by(|a, b| a.partial_cmp(b).unwrap());
        new_seg_t_params.dedup_by(|a, b| (*a - *b).abs() * new_len < tol::POINT_MERGE);

        // Build the sequence of vertex ids along the new segment.
        let seg_dir = to_pos - from_pos;
        let mut seg_vids: Vec<SketchVertexId> = Vec::new();
        for &t in &new_seg_t_params {
            let pos = from_pos + seg_dir * t;
            // t == 0.0 -> from_vid, t == 1.0 -> to_vid
            let vid = if t * new_len < tol::POINT_MERGE {
                from_vid
            } else if (1.0 - t) * new_len < tol::POINT_MERGE {
                to_vid
            } else {
                self.find_or_create_vertex(pos, &mut report.new_vertices)
            };
            seg_vids.push(vid);
        }

        // ── Step 8: insert fragment edges for the new segment ─────────────────
        // Skip fragments that already have an existing edge (collinear merge).
        // Such a fragment is drawn over an edge already there: the loops
        // that edge bounds get their face below (rule 6).
        // With a scan list, an edge already between two of the segment's
        // vertices lies along the segment, so it is in the list, or it is
        // a fragment this insertion just cut from a collinear edge.
        let edge_between = |s: &Sketch,
                            report: &SegmentAdded,
                            a: SketchVertexId,
                            b: SketchVertexId|
         -> Option<SketchEdgeId> {
            let joins = |e: &SketchEdge| (e.from == a && e.to == b) || (e.from == b && e.to == a);
            match scan {
                Some(list) => list
                    .iter()
                    .copied()
                    .chain(
                        report
                            .split_edges
                            .iter()
                            .flat_map(|(_, f)| f.iter().copied()),
                    )
                    .chain(report.new_edges.iter().copied())
                    .find(|&id| s.edges.get(id).is_some_and(joins)),
                None => s.edges.iter().find(|(_, e)| joins(e)).map(|(id, _)| id),
            }
        };
        let mut retraced: Vec<(SketchVertexId, SketchVertexId)> = Vec::new();
        for w in seg_vids.windows(2) {
            let a = w[0];
            let b = w[1];
            if let Some(eid) = edge_between(self, &report, a, b) {
                retraced.push((a, b));
                // Collinear merge: this sub-segment already exists. If a
                // curve bracket is open and the existing edge is a plain
                // line, ADOPT it into the curve — otherwise the drawn curve
                // would have an untagged hole where it overlapped existing
                // geometry, splitting the selectable unit. An edge already
                // in another curve keeps its allegiance.
                if let Some(curve) = self.active_curve
                    && self.edges[eid].curve.is_none()
                {
                    self.edges[eid].curve = Some(curve);
                }
                continue;
            }
            // Skip zero-length fragments.
            if self.vertices[a]
                .position
                .approx_eq(self.vertices[b].position, tol::POINT_MERGE)
            {
                continue;
            }
            let eid = self.edges.insert(SketchEdge {
                from: a,
                to: b,
                curve: self.active_curve,
            });
            report.new_edges.push(eid);
        }

        // ── Step 9: recompute regions, islands, and diff ──────────────────────
        if defer_regions {
            return Ok(report);
        }
        let (regions_created, regions_removed) = self.recompute_regions_with_diff(&old_regions);
        report.regions_created = regions_created;
        report.regions_removed = regions_removed;
        // A loop drawn over gets its face (rule 6): every faceless region
        // whose outer ring one of the retraced edges is on. (A region the
        // edge bounds as a hole is left alone: tracing a loop inside a
        // larger one faces that loop, not the ring around it.)
        if !retraced.is_empty() {
            for r in self.regions.values_mut().filter(|r| r.faceless) {
                let bounded = r
                    .outer
                    .iter()
                    .zip(r.outer.iter().cycle().skip(1))
                    .any(|(&p, &q)| {
                        retraced
                            .iter()
                            .any(|&(a, b)| (a == p && b == q) || (a == q && b == p))
                    });
                if bounded {
                    r.faceless = false;
                }
            }
        }
        self.recompute_islands();

        Ok(report)
    }

    /// Core implementation of `remove_edge`. Called on a clone so the strong
    /// guarantee holds at the public method.
    fn remove_edge_inner(&mut self, edge: SketchEdgeId) -> EdgeRemoved {
        let old_regions = self.region_snapshot();

        let e = self.edges[edge];
        self.edges.remove(edge);

        // Delete vertices with no remaining incident edges.
        let mut removed_vertices = Vec::new();
        for vid in [e.from, e.to] {
            if !self.vertex_has_edges(vid) {
                self.vertices.remove(vid);
                removed_vertices.push(vid);
            }
        }

        let (regions_created, regions_removed) = self.recompute_regions_with_diff(&old_regions);
        self.recompute_islands();

        EdgeRemoved {
            removed_vertices,
            regions_removed,
            regions_created,
        }
    }

    // ── Vertex helpers ────────────────────────────────────────────────────────

    /// Returns the id of an existing vertex within `tol::POINT_MERGE` of `pos`,
    /// or `None` if there is none.
    fn find_or_plan_vertex(&self, pos: Point3) -> Option<SketchVertexId> {
        if let Some(cells) = &self.bulk_vertex_cells {
            // The same answer as the scan below — the first vertex in id
            // order within the merge tolerance — from the cell the point
            // falls in and its neighbors.
            let c = vertex_cell(pos);
            let mut best: Option<SketchVertexId> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                            for &id in list {
                                if self.vertices[id].position.approx_eq(pos, tol::POINT_MERGE)
                                    && best.is_none_or(|b| id < b)
                                {
                                    best = Some(id);
                                }
                            }
                        }
                    }
                }
            }
            return best;
        }
        for (id, v) in &self.vertices {
            if v.position.approx_eq(pos, tol::POINT_MERGE) {
                return Some(id);
            }
        }
        None
    }

    /// Returns an existing vertex within `tol::POINT_MERGE` of `pos`, or
    /// creates a new one and records it in `new_vertices`.
    fn find_or_create_vertex(
        &mut self,
        pos: Point3,
        new_vertices: &mut Vec<SketchVertexId>,
    ) -> SketchVertexId {
        if let Some(id) = self.find_or_plan_vertex(pos) {
            return id;
        }
        let id = self.vertices.insert(SketchVertex { position: pos });
        new_vertices.push(id);
        self.note_bulk_vertex(id);
        id
    }

    /// Puts a vertex just inserted into the bulk-insertion cells, when a
    /// bulk add is in progress, so a lookup later in the same insertion
    /// finds it as the scan would.
    fn note_bulk_vertex(&mut self, id: SketchVertexId) {
        if let Some(cells) = &mut self.bulk_vertex_cells {
            cells
                .entry(vertex_cell(self.vertices[id].position))
                .or_default()
                .push(id);
        }
    }

    /// Returns the id of the edge connecting `a` and `b` (in either direction),
    /// or `None` if no such edge exists.
    fn edge_between(&self, a: SketchVertexId, b: SketchVertexId) -> Option<SketchEdgeId> {
        self.edges
            .iter()
            .find(|(_, e)| (e.from == a && e.to == b) || (e.from == b && e.to == a))
            .map(|(id, _)| id)
    }

    /// True if the vertex has at least one incident edge (in either direction).
    fn vertex_has_edges(&self, vid: SketchVertexId) -> bool {
        self.edges.values().any(|e| e.from == vid || e.to == vid)
    }

    // ── Region computation ────────────────────────────────────────────────────

    /// Recompute all regions from scratch, then diff against the previous set.
    /// Returns `(created, removed)`.
    fn recompute_regions_with_diff(
        &mut self,
        old_regions: &[RegionSnapshot],
    ) -> (Vec<SketchRegionId>, Vec<SketchRegionId>) {
        let new_region_data = self.compute_all_regions();

        // Identity rule: a region whose outer vertex-id cycle is unchanged
        // (cyclic equality, same orientation) KEEPS its SketchRegionId — its
        // stored value is refreshed in place (hole sets may differ) and it
        // appears in neither report list, so handles held by tools stay
        // valid until a mutation actually reshapes the region. It keeps
        // whether it has a face, too (module docs, rule 6). Everything
        // else is a removal + creation; whether a created region has a
        // face is `inherits_no_face`'s call.
        let mut regions_created: Vec<SketchRegionId> = Vec::new();
        let mut reused: std::collections::BTreeSet<SketchRegionId> =
            std::collections::BTreeSet::new();

        for mut new_r in new_region_data {
            let matched = old_regions.iter().find(|old| {
                !reused.contains(&old.id) && cycles_equal(&old.region.outer, &new_r.outer)
            });
            match matched {
                Some(old) => {
                    reused.insert(old.id);
                    new_r.faceless = old.region.faceless;
                    self.regions[old.id] = new_r;
                }
                None => {
                    new_r.faceless = self.inherits_no_face(&new_r, old_regions);
                    regions_created.push(self.regions.insert(new_r));
                }
            }
        }

        let mut regions_removed: Vec<SketchRegionId> = Vec::new();
        for old in old_regions {
            if !reused.contains(&old.id) {
                self.regions.remove(old.id);
                regions_removed.push(old.id);
            }
        }

        (regions_created, regions_removed)
    }

    /// Whether a region a mutation brought about, with no old region of
    /// the same vertex cycle, is a loop with no face (module docs, rule 6).
    /// It is when it is an old loop with no face in a new guise: the same
    /// shape with vertices added or dropped along its straight edges (a
    /// line ending on one of its edges splits that edge; erasing such a
    /// line heals it), or a larger loop that took in a faceless loop's
    /// interior because an edge between them was erased (erasing an edge
    /// never gives a face; a face next to a loop with none loses its
    /// boundary with it). Otherwise it is a loop a drawing step closed,
    /// and it has a face.
    fn inherits_no_face(&self, new_r: &SketchRegion, old_regions: &[RegionSnapshot]) -> bool {
        let normal = self.plane.normal();
        let ring: Vec<Point3> = new_r
            .outer
            .iter()
            .map(|&v| self.vertices[v].position)
            .collect();
        let shape = simplified_ring(&ring);
        if let Some(old) = old_regions
            .iter()
            .find(|old| rings_coincide(&simplified_ring(&old.outer), &shape))
        {
            return old.region.faceless;
        }
        let holes: Vec<Vec<Point3>> = new_r
            .holes
            .iter()
            .map(|h| h.iter().map(|&v| self.vertices[v].position).collect())
            .collect();
        let area = signed_area_on_plane(&ring, normal).abs();
        old_regions
            .iter()
            .filter(|old| old.region.faceless)
            .any(|old| {
                signed_area_on_plane(&old.outer, normal).abs() <= area + tol::POINT_MERGE
                    && interior_point_of_loops(&old.outer, &old.holes, normal).is_some_and(|p| {
                        point_inside_polygon(p, &ring, normal)
                            && !holes.iter().any(|h| point_inside_polygon(p, h, normal))
                    })
            })
    }

    /// Compute all regions (CCW outer cycles + their CW hole cycles) from the
    /// current edge graph, using planar half-edge face tracing.
    fn compute_all_regions(&self) -> Vec<SketchRegion> {
        if self.edges.is_empty() {
            return Vec::new();
        }

        let normal = self.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);

        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };

        // Build adjacency: for each vertex, sorted list of outgoing directed
        // half-edges by angle.
        //
        // A sketch edge (a, b) gives two directed half-edges: a->b and b->a.
        // We store them as (from_vid, to_vid).

        // Collect all directed half-edges.
        let mut half_edges: Vec<(SketchVertexId, SketchVertexId)> = Vec::new();
        for e in self.edges.values() {
            half_edges.push((e.from, e.to));
            half_edges.push((e.to, e.from));
        }

        // For each vertex, build sorted adjacency list by angle.
        let mut adj: std::collections::BTreeMap<SketchVertexId, Vec<SketchVertexId>> =
            std::collections::BTreeMap::new();
        for &(from, to) in &half_edges {
            adj.entry(from).or_default().push(to);
        }
        // Sort each adjacency list by angle of the outgoing direction.
        for (from_vid, neighbors) in adj.iter_mut() {
            let from_pos = self.vertices[*from_vid].position;
            let fp = proj(from_pos);
            neighbors.sort_by(|&a, &b| {
                let pa = proj(self.vertices[a].position);
                let pb = proj(self.vertices[b].position);
                let angle_a = (pa.1 - fp.1).atan2(pa.0 - fp.0);
                let angle_b = (pb.1 - fp.1).atan2(pb.0 - fp.0);
                angle_a.partial_cmp(&angle_b).unwrap()
            });
        }

        // Trace all face cycles: for each directed half-edge (a->b), the next
        // half-edge in the face cycle is found by:
        //   1. At vertex b, find the incoming direction = reverse of a->b = b->a.
        //   2. Find b->a in b's sorted adjacency list.
        //   3. Take the NEXT entry (wrapping) — the rotational successor.
        //   4. That gives the next outgoing half-edge b->c.
        let mut visited: std::collections::BTreeSet<(SketchVertexId, SketchVertexId)> =
            std::collections::BTreeSet::new();
        let mut cycles: Vec<Vec<SketchVertexId>> = Vec::new();

        for &start_he in &half_edges {
            if visited.contains(&start_he) {
                continue;
            }
            let mut cycle: Vec<SketchVertexId> = Vec::new();
            let mut current = start_he;
            loop {
                if visited.contains(&current) {
                    break;
                }
                visited.insert(current);
                cycle.push(current.0); // push the 'from' vertex

                // Find successor: at vertex current.1, find the next outgoing
                // direction after the reverse of current.
                let arrive_at = current.1;
                let reverse_dir = current.0; // the edge that brought us to arrive_at

                let neighbors = match adj.get(&arrive_at) {
                    Some(n) => n,
                    None => break,
                };

                // Find index of reverse_dir in neighbors.
                let idx = match neighbors.iter().position(|&n| n == reverse_dir) {
                    Some(i) => i,
                    None => break,
                };

                // Rotational successor: previous index (wrapping), because we
                // want "next CCW from the reversed incoming direction".
                // In standard planar tracing: sort CCW, successor of reverse is
                // the one just BEFORE it in CCW order (i.e., rotate CW to find
                // the face to the left of the current directed edge).
                let next_idx = if idx == 0 {
                    neighbors.len() - 1
                } else {
                    idx - 1
                };
                let next_to = neighbors[next_idx];
                current = (arrive_at, next_to);
            }

            // Prune spurs: a dangling chain attached to the cycle is walked
            // out-and-back by planar face tracing, leaving a pinched
            // `… a, tip, a …` pattern with repeated vertices. Such a cycle
            // renders a fill (the spur has zero area) but is not a simple
            // polygon, so profiles built from it are refused — the spur is
            // decoration, not boundary. Collapse every backtrack until none
            // remain; chains fold up tip-first over the iterations.
            loop {
                let n = cycle.len();
                if n < 3 {
                    break;
                }
                let mut pruned = false;
                for i in 0..n {
                    let prev = cycle[(i + n - 1) % n];
                    let next = cycle[(i + 1) % n];
                    if prev == next {
                        // cycle[i] is a spur tip: drop it and one duplicate
                        // of its base (the entry at i+1).
                        let (a, b) = (i, (i + 1) % n);
                        let (first, second) = if a > b { (a, b) } else { (b, a) };
                        cycle.remove(first);
                        cycle.remove(second);
                        pruned = true;
                        break;
                    }
                }
                if !pruned {
                    break;
                }
            }

            // A walk visiting a vertex twice (a loop touching another, or
            // itself, at one point) is split there into simple rings, as a
            // solid's face loops are: a circle tangent to a rectangle's side
            // is a hole sharing that vertex with the rectangle's ring, and
            // two circles touching are two holes sharing one vertex.
            for ring in split_ring_at_repeats(cycle) {
                if ring.len() >= 3 {
                    cycles.push(ring);
                }
            }
        }

        if cycles.is_empty() {
            return Vec::new();
        }

        // Compute signed area for each cycle.  Positive = CCW (candidate region
        // outer boundary); negative = CW (hole boundary or outer face).
        let cycle_areas: Vec<f64> = cycles
            .iter()
            .map(|cycle| {
                let pts: Vec<Point3> = cycle
                    .iter()
                    .map(|&vid| self.vertices[vid].position)
                    .collect();
                signed_area_on_plane(&pts, normal)
            })
            .collect();

        // Identify the unbounded outer face: among negative-area cycles, the one
        // whose absolute area is largest (it encircles everything).
        // Actually for a planar graph on an infinite plane the unbounded face is
        // the cycle with the largest *absolute* area (positive or negative).
        // We discard the single cycle with the maximum absolute area if it has
        // negative (CW) signed area — that's the outer unbounded face.
        //
        // However, some graph configurations don't produce an unbounded face
        // cycle (e.g., a single isolated edge); in that case we just skip.

        let max_abs_area = cycle_areas
            .iter()
            .map(|a| a.abs())
            .fold(f64::NEG_INFINITY, f64::max);
        let unbounded_idx = cycle_areas
            .iter()
            .enumerate()
            .filter(|&(_, a)| *a < 0.0 && (a.abs() - max_abs_area).abs() < tol::POINT_MERGE * 1e6)
            // The most negative of them: a cycle split off a pinched walk
            // (a hole touching the outer ring) may come first and lie
            // within that window of the true outside.
            .min_by(|x, y| x.1.total_cmp(y.1))
            .map(|(i, _)| i);

        // Separate CCW outer candidates from CW hole candidates (excluding the
        // unbounded outer face).
        let mut outer_candidates: Vec<(usize, f64)> = Vec::new(); // (cycle_idx, area)
        let mut hole_candidates: Vec<(usize, f64)> = Vec::new(); // (cycle_idx, abs_area)

        for (i, &area) in cycle_areas.iter().enumerate() {
            if Some(i) == unbounded_idx {
                continue;
            }
            if area > 0.0 {
                outer_candidates.push((i, area));
            } else if area < 0.0 {
                hole_candidates.push((i, area.abs()));
            }
        }

        // Assign each hole cycle to the smallest-area enclosing outer cycle
        // (point-in-polygon on one representative point of the hole).
        // If no enclosing outer cycle is found, the hole is an island that
        // failed to find its encloser — treat as outer with no holes (shouldn't
        // happen with valid planar graphs).

        // Build region data: one SketchRegion per outer candidate, then assign
        // holes.
        let mut region_outers: Vec<Vec<SketchVertexId>> = outer_candidates
            .iter()
            .map(|&(ci, _)| cycles[ci].clone())
            .collect();
        let mut region_holes: Vec<Vec<Vec<SketchVertexId>>> =
            vec![Vec::new(); outer_candidates.len()];

        for &(hole_ci, hole_area) in &hole_candidates {
            let hole_cycle = &cycles[hole_ci];
            // Representative point: a point strictly inside the hole loop
            // ([`interior_point_of_loops`], a scanline clear of every vertex).
            // A boundary vertex is unreliable for point-in-polygon and, being
            // shared with neighbouring cycles, could land exactly on another
            // cycle's edge; the vertex centroid is no better for a concave
            // hole — a C-shaped loop's centroid lies outside it, inside
            // whatever the C wraps around, which then wrongly took the loop
            // as its hole (found by the world-sketch fuzzer: an inward offset
            // of a notched ring). The centroid stays only as the fallback for
            // a loop too degenerate to scan.
            let hole_pts: Vec<Point3> = hole_cycle
                .iter()
                .map(|&vid| self.vertices[vid].position)
                .collect();
            let rep_pos = interior_point_of_loops(&hole_pts, &[], normal).unwrap_or_else(|| {
                let n = hole_pts.len() as f64;
                let sum = hole_pts.iter().fold(Vec3::ZERO, |acc, p| acc + p.to_vec());
                Point3::ORIGIN + sum / n
            });

            // Assign the hole to the *smallest* enclosing outer cycle, while
            // excluding two things: (1) the hole's own reverse-wound twin — the
            // same closed loop bounds the region on one side (CCW outer
            // candidate) and its neighbour on the other (CW hole), so a hole
            // must never take its own boundary as its encloser (without this, a
            // ring took its own twin as a hole and Profile::new rejected it as
            // self-intersecting); and (2) any cycle nested *inside* the hole, so
            // nested rings attach holes to their immediate encloser. The twin is
            // identified by exact vertex-set identity — NOT by an "equal area"
            // test, which is FP-fragile (at some facet counts the twin's area
            // drifts an epsilon past a strict `<=`, letting the hole attach to
            // itself — the faceted-through-hole NonManifoldResult bug).
            let mut best: Option<(usize, f64)> = None; // (region_idx, area)
            for (ri, &(outer_ci, outer_area)) in outer_candidates.iter().enumerate() {
                let outer_cycle = &cycles[outer_ci];
                // (1) Skip the hole's reverse-wound twin (identical vertex set).
                if outer_cycle.len() == hole_cycle.len()
                    && hole_cycle.iter().all(|v| outer_cycle.contains(v))
                {
                    continue;
                }
                // (2) Skip cycles nested inside (or coincident with) the hole.
                if outer_area <= hole_area {
                    continue;
                }
                let outer_pts: Vec<Point3> = cycles[outer_ci]
                    .iter()
                    .map(|&vid| self.vertices[vid].position)
                    .collect();
                if point_inside_polygon(rep_pos, &outer_pts, normal) {
                    match best {
                        None => best = Some((ri, outer_area)),
                        Some((_, best_area)) if outer_area < best_area => {
                            best = Some((ri, outer_area));
                        }
                        _ => {}
                    }
                }
            }

            if let Some((ri, _)) = best {
                region_holes[ri].push(hole_cycle.clone());
            }
            // If no enclosing outer found, the hole cycle is discarded (edge
            // case of disconnected geometry — not reachable in a well-formed sketch).
        }

        // Build SketchRegion objects.
        let mut regions: Vec<SketchRegion> = Vec::new();
        for (i, outer) in region_outers.drain(..).enumerate() {
            regions.push(SketchRegion {
                outer,
                holes: region_holes[i].clone(),
                faceless: false,
            });
        }

        regions
    }
}

/// Splits a closed vertex walk into simple rings at every vertex it visits
/// more than once: each stretch between two visits of one vertex closes on
/// its own, in the walk's order. A simple walk comes back whole.
fn split_ring_at_repeats(walk: Vec<SketchVertexId>) -> Vec<Vec<SketchVertexId>> {
    let mut rings: Vec<Vec<SketchVertexId>> = Vec::new();
    let mut stack: Vec<SketchVertexId> = Vec::with_capacity(walk.len());
    let mut at: std::collections::BTreeMap<SketchVertexId, usize> =
        std::collections::BTreeMap::new();
    for v in walk {
        if let Some(&i) = at.get(&v) {
            let ring = stack.split_off(i);
            for x in &ring {
                at.remove(x);
            }
            rings.push(ring);
        }
        at.insert(v, stack.len());
        stack.push(v);
    }
    rings.push(stack);
    rings
}

// ═════════════════════════════════════════════ structural reconstruction

impl Sketch {
    /// Create an empty sketch for structural reconstruction, `detached` as
    /// the file says. Unlike `on_plane`, this is `pub(crate)` so the
    /// serializer can build a Sketch by directly inserting pre-validated
    /// elements (bypassing the sticky-geometry pipeline).
    pub(crate) fn reconstruct(plane: Plane, detached: bool) -> Sketch {
        Self::blank(plane, detached)
    }

    /// Insert a vertex directly into the slotmap (no planarity check, no merging).
    /// For structural reconstruction only.
    pub(crate) fn insert_vertex_raw(&mut self, v: SketchVertex) -> SketchVertexId {
        self.vertices.insert(v)
    }

    /// Insert an edge directly into the slotmap (no intersection checks).
    /// For structural reconstruction only.
    pub(crate) fn insert_edge_raw(&mut self, e: SketchEdge) -> SketchEdgeId {
        self.edges.insert(e)
    }

    /// Insert a region directly into the slotmap (no geometric check).
    /// For structural reconstruction only.
    pub(crate) fn insert_region_raw(&mut self, r: SketchRegion) -> SketchRegionId {
        self.regions.insert(r)
    }
}

// ═══════════════════════════════════════════════════════════ intersection math

/// Result of a 2D segment intersection query.
#[derive(Debug)]
enum Intersection2D {
    /// Proper interior crossing.
    Proper {
        t_new: f64,
        s_existing: f64,
        point: Point3,
    },
    /// An endpoint of the *new* segment lands on the interior of the existing
    /// segment.
    NewEndpointOnExisting { s_existing: f64, point: Point3 },
    /// An endpoint of the *existing* segment lands on the interior of the new
    /// segment.
    ExistingEndpointOnNew { t_new: f64, _point: Point3 },
    /// The two segments are collinear and overlap (or one contains the other).
    Collinear {
        /// t-parameters on the new segment for the existing endpoints that
        /// land inside it.
        t_params_on_new: Vec<f64>,
        /// s-parameters on the existing segment for the new endpoints that
        /// land inside it.
        s_params_on_existing: Vec<f64>,
        points_on_new: Vec<Point3>,
        points_on_existing: Vec<Point3>,
    },
}

/// Compute all intersection events between the new segment [new_from, new_to]
/// (given as 2D projections `nf`, `nt`) and the existing edge `e` (2D
/// projections `ef`, `et`).
///
/// Every contact is classified METRICALLY, against [`tol::POINT_MERGE`] in
/// metres — the tolerance every face-side gate uses (the drafting engine,
/// `split_face`), so a sketch and a solid's face agree on what touches what.
/// Distance tests run first: an end of one segment within `POINT_MERGE` of
/// the other's interior is a T-contact; both ends of one within
/// `POINT_MERGE` of the other's line is collinear overlap. Only when no end
/// touches is a PROPER crossing decided, exactly, by orientation signs
/// ([`crate::geom2d::orient2d`]), so a crossing is never missed however
/// acute and never invented by rounding. Only events in the open interior
/// of the relevant segment (more than `POINT_MERGE` from both its ends,
/// measured along it) are returned; end-to-end coincidences are the
/// caller's vertex snapping.
#[allow(clippy::too_many_arguments)]
fn seg_seg_intersections_2d(
    nf: (f64, f64), // new segment from
    nt: (f64, f64), // new segment to
    ef: (f64, f64), // existing edge from
    et: (f64, f64), // existing edge to
    vertices: &SlotMap<SketchVertexId, SketchVertex>,
    edge: SketchEdge,
    u_ax: Vec3,
    v_ax: Vec3,
    anchor: Point3,
) -> Vec<Intersection2D> {
    // Reconstruct 3D position from 2D plane coords. `anchor` carries the
    // out-of-plane component (the plane need not pass through the origin); the
    // 2D coords supply the in-plane displacement.
    let pos_from_2d = |p: (f64, f64)| -> Point3 { anchor + u_ax * p.0 + v_ax * p.1 };
    let eps = tol::POINT_MERGE;

    let sub = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0, a.1 - b.1);
    let dot = |a: (f64, f64), b: (f64, f64)| a.0 * b.0 + a.1 * b.1;
    let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;

    let d_new = sub(nt, nf);
    let d_ext = sub(et, ef);
    let new_len2 = dot(d_new, d_new);
    let ext_len2 = dot(d_ext, d_ext);
    let new_len = new_len2.sqrt();
    let ext_len = ext_len2.sqrt();
    if new_len <= eps || ext_len <= eps {
        return Vec::new(); // degenerate segment: nothing to classify
    }

    // Parameter of `p`'s projection onto the line `a + d·t` (unclamped).
    let param = |p: (f64, f64), a: (f64, f64), d: (f64, f64), len2: f64| dot(sub(p, a), d) / len2;
    // Perpendicular distance from `p` to the infinite line `a + d·t`.
    let line_dist =
        |p: (f64, f64), a: (f64, f64), d: (f64, f64), len: f64| cross(d, sub(p, a)).abs() / len;
    // Strictly inside a segment of length `len`: more than `eps` from both
    // ends, measured along it.
    let interior = |t: f64, len: f64| t * len > eps && (1.0 - t) * len > eps;

    let ef_on_new_line = line_dist(ef, nf, d_new, new_len) <= eps;
    let et_on_new_line = line_dist(et, nf, d_new, new_len) <= eps;
    let nf_on_ext_line = line_dist(nf, ef, d_ext, ext_len) <= eps;
    let nt_on_ext_line = line_dist(nt, ef, d_ext, ext_len) <= eps;

    if (ef_on_new_line && et_on_new_line) || (nf_on_ext_line && nt_on_ext_line) {
        // Collinear (to within tolerance): rule 4, overlap. Each end of one
        // segment lying strictly inside the other splits it there.
        let mut t_params_on_new: Vec<f64> = Vec::new();
        let mut points_on_new: Vec<Point3> = Vec::new();
        for (ep2d, vid) in [(ef, edge.from), (et, edge.to)] {
            let t = param(ep2d, nf, d_new, new_len2);
            if interior(t, new_len) {
                t_params_on_new.push(t);
                // The existing vertex's own bits, not a reconstruction.
                points_on_new.push(vertices[vid].position);
            }
        }
        let mut s_params_on_existing: Vec<f64> = Vec::new();
        let mut points_on_existing: Vec<Point3> = Vec::new();
        for np2d in [nf, nt] {
            let s = param(np2d, ef, d_ext, ext_len2);
            if interior(s, ext_len) {
                s_params_on_existing.push(s);
                points_on_existing.push(pos_from_2d(np2d));
            }
        }
        if t_params_on_new.is_empty() && s_params_on_existing.is_empty() {
            return Vec::new(); // disjoint along the line, or touching at ends
        }
        return vec![Intersection2D::Collinear {
            t_params_on_new,
            s_params_on_existing,
            points_on_new,
            points_on_existing,
        }];
    }

    // T-contacts: an end of one segment within `eps` of the other's interior.
    let mut events: Vec<Intersection2D> = Vec::new();
    for (np2d, on_line) in [(nf, nf_on_ext_line), (nt, nt_on_ext_line)] {
        let s = param(np2d, ef, d_ext, ext_len2);
        if on_line && interior(s, ext_len) {
            events.push(Intersection2D::NewEndpointOnExisting {
                s_existing: s,
                point: pos_from_2d(np2d),
            });
        }
    }
    for (ep2d, vid, on_line) in [
        (ef, edge.from, ef_on_new_line),
        (et, edge.to, et_on_new_line),
    ] {
        let t = param(ep2d, nf, d_new, new_len2);
        if on_line && interior(t, new_len) {
            events.push(Intersection2D::ExistingEndpointOnNew {
                t_new: t,
                _point: vertices[vid].position,
            });
        }
    }
    if !events.is_empty() {
        // Two non-collinear lines meet once: a touching end IS the contact.
        return events;
    }

    // Proper crossing, decided exactly: each segment's ends strictly on
    // opposite sides of the other's line. No end is within `eps` of the other
    // segment here, so the crossing lies strictly inside both.
    use crate::geom2d::orient2d;
    if orient2d(ef, et, nf) * orient2d(ef, et, nt) < 0
        && orient2d(nf, nt, ef) * orient2d(nf, nt, et) < 0
    {
        let denom = cross(d_new, d_ext);
        let w = sub(ef, nf);
        let t = (cross(w, d_ext) / denom).clamp(0.0, 1.0);
        let s = (cross(w, d_new) / denom).clamp(0.0, 1.0);
        let point = pos_from_2d((ef.0 + d_ext.0 * s, ef.1 + d_ext.1 * s));
        return vec![Intersection2D::Proper {
            t_new: t,
            s_existing: s,
            point,
        }];
    }
    Vec::new()
}

// ══════════════════════════════════════════════════════════ cycle comparison

/// The cell a vertex falls in for [`Sketch::add_segments`]' merge lookup:
/// its position at the grain of [`tol::POINT_MERGE`], so two points within
/// the tolerance are in one cell or neighboring ones.
fn vertex_cell(p: Point3) -> [i64; 3] {
    // Clamped short of the integer range, so a neighbor of the cell a
    // far-off coordinate falls in can still be named.
    let cell = |x: f64| (x / tol::POINT_MERGE).round().clamp(-4.0e18, 4.0e18) as i64;
    [cell(p.x), cell(p.y), cell(p.z)]
}

/// A ring without the vertices that lie on the straight line between their
/// neighbors: the loop's shape alone, whatever a T-junction or a healed
/// split has added to or dropped from its vertex list.
fn simplified_ring(ring: &[Point3]) -> Vec<Point3> {
    let n = ring.len();
    if n < 3 {
        return ring.to_vec();
    }
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let ab = b - a;
            let len = ab.length();
            // `b` is kept unless it sits on the line through `a` and `c`
            // (`c`'s distance from the line `ab` within the merge tolerance).
            len <= tol::POINT_MERGE || ab.cross(c - b).length() / len > tol::POINT_MERGE
        })
        .map(|i| ring[i])
        .collect()
}

/// True if two rings of points are the same loop: equal under cyclic
/// rotation, in either direction, within [`tol::POINT_MERGE`] per point.
/// (A loop replayed through a transform may come back traced the other way
/// round when the plane is turned over, so direction is not fixed here.)
fn rings_coincide(a: &[Point3], b: &[Point3]) -> bool {
    let n = a.len();
    if n != b.len() || n == 0 {
        return false;
    }
    (0..n).any(|start| {
        (0..n).all(|i| a[(start + i) % n].approx_eq(b[i], tol::POINT_MERGE))
            || (0..n).all(|i| a[(start + n - i) % n].approx_eq(b[i], tol::POINT_MERGE))
    })
}

/// True if two vertex-id cycles are equal under cyclic rotation and same
/// orientation (we do NOT check reversed orientation here — outer cycles are
/// always CCW and holes always CW, so direction is fixed).
fn cycles_equal(a: &[SketchVertexId], b: &[SketchVertexId]) -> bool {
    let n = a.len();
    if n != b.len() {
        return false;
    }
    // Try all rotations of a against b.
    'outer: for start in 0..n {
        for i in 0..n {
            if a[(start + i) % n] != b[i] {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

// ══════════════════════════════════════════════════════════════════ Profile

/// A validated closed planar polygon with holes: the only currency accepted
/// by `Object::from_extrusion`.
///
/// Invariants established at construction and never re-checked downstream:
/// every boundary point on `plane` within [`tol::PLANE_DIST`](crate::tol::PLANE_DIST); outer boundary
/// counter-clockwise seen from the plane normal, holes clockwise; no
/// self-intersection, no boundary-boundary contact; every hole strictly
/// inside the outer boundary and outside every other hole.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    plane: Plane,
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    /// Analytic attribution per outer boundary edge: `outer_curves[k]` is
    /// the circle edge `outer[k] → outer[k+1]` is a facet of, when the edge
    /// came from a curve chain with geometry. Parallel to `outer`; empty ⇒
    /// no attribution anywhere (the [`Profile::new`] default).
    outer_curves: Vec<Option<CurveGeom>>,
    /// Same, per hole boundary edge. Parallel to `holes` when non-empty.
    hole_curves: Vec<Vec<Option<CurveGeom>>>,
}

/// Typed reasons a would-be [`Profile`] is rejected. The kernel never fixes
/// winding, drops points, or "heals" intersections (DEVELOPMENT.md rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// A boundary has fewer than 3 points.
    TooFewPoints,
    /// A boundary point is farther than [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the plane.
    PointOffPlane,
    /// Two consecutive boundary points are within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE).
    DegenerateEdge,
    /// The outer boundary is not counter-clockwise, or a hole is not
    /// clockwise, seen from the plane normal side.
    WrongWinding,
    /// A boundary crosses or touches itself or another boundary.
    SelfIntersecting,
    /// A hole is not strictly inside the outer boundary, or overlaps another
    /// hole.
    HoleOutsideOuter,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            ProfileError::TooFewPoints => "boundary has fewer than 3 points",
            ProfileError::PointOffPlane => "boundary point is off the profile plane",
            ProfileError::DegenerateEdge => "consecutive boundary points coincide",
            ProfileError::WrongWinding => {
                "outer boundary must be CCW and holes CW seen from the plane normal"
            }
            ProfileError::SelfIntersecting => "boundaries cross or touch",
            ProfileError::HoleOutsideOuter => "hole is not strictly inside the outer boundary",
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for ProfileError {}

impl Profile {
    /// Validates and constructs a profile. See the type docs for the
    /// invariants checked; each failure maps to one [`ProfileError`] variant.
    pub fn new(
        plane: Plane,
        outer: Vec<Point3>,
        holes: Vec<Vec<Point3>>,
    ) -> Result<Profile, ProfileError> {
        // 1. Minimum point counts.
        if outer.len() < 3 {
            return Err(ProfileError::TooFewPoints);
        }
        for hole in &holes {
            if hole.len() < 3 {
                return Err(ProfileError::TooFewPoints);
            }
        }

        // 2. Planarity of all boundaries.
        for &p in &outer {
            if plane.signed_distance(p).abs() > crate::tol::PLANE_DIST {
                return Err(ProfileError::PointOffPlane);
            }
        }
        for hole in &holes {
            for &p in hole {
                if plane.signed_distance(p).abs() > crate::tol::PLANE_DIST {
                    return Err(ProfileError::PointOffPlane);
                }
            }
        }

        // 3. Consecutive-point separation (degenerate edges).
        profile_check_degenerate(&outer)?;
        for hole in &holes {
            profile_check_degenerate(hole)?;
        }

        // 4. Self-intersection and boundary-boundary contact.
        //    Must be checked before winding because a bowtie's signed area is
        //    ambiguous (often ~0) and we want SelfIntersecting, not WrongWinding.
        profile_check_simple(&outer)?;
        for hole in &holes {
            profile_check_simple(hole)?;
        }
        // Check outer against each hole, and holes against each other.
        let all_boundaries: Vec<&Vec<Point3>> =
            std::iter::once(&outer).chain(holes.iter()).collect();
        for i in 0..all_boundaries.len() {
            for j in (i + 1)..all_boundaries.len() {
                if boundaries_contact(all_boundaries[i], all_boundaries[j]) {
                    return Err(ProfileError::SelfIntersecting);
                }
            }
        }

        // 5. Winding check via signed area against the plane normal.
        //    Outer must be CCW (positive signed area), holes CW (negative).
        let outer_area = signed_area_on_plane(&outer, plane.normal());
        if outer_area <= 0.0 {
            return Err(ProfileError::WrongWinding);
        }
        for hole in &holes {
            let hole_area = signed_area_on_plane(hole, plane.normal());
            if hole_area >= 0.0 {
                return Err(ProfileError::WrongWinding);
            }
        }

        // 6. Holes must be strictly inside the outer boundary and disjoint.
        for hole in &holes {
            // Every hole vertex must be inside the outer boundary.
            for &p in hole {
                if !point_inside_polygon(p, &outer, plane.normal()) {
                    return Err(ProfileError::HoleOutsideOuter);
                }
            }
            // Every outer vertex must be outside each hole.
            for &p in &outer {
                if point_inside_polygon(p, hole, plane.normal()) {
                    return Err(ProfileError::HoleOutsideOuter);
                }
            }
        }
        // Holes must be disjoint from each other.
        for i in 0..holes.len() {
            for j in (i + 1)..holes.len() {
                for &p in &holes[j] {
                    if point_inside_polygon(p, &holes[i], plane.normal()) {
                        return Err(ProfileError::HoleOutsideOuter);
                    }
                }
                for &p in &holes[i] {
                    if point_inside_polygon(p, &holes[j], plane.normal()) {
                        return Err(ProfileError::HoleOutsideOuter);
                    }
                }
            }
        }

        Ok(Profile {
            plane,
            outer,
            holes,
            outer_curves: Vec::new(),
            hole_curves: Vec::new(),
        })
    }

    /// Attaches per-edge analytic curve attribution (see the field docs).
    /// Crate-internal: only [`Sketch::profile`] produces attribution, from
    /// the region's own edges. Lengths must be parallel to the boundaries.
    pub(crate) fn set_curve_attribution(
        &mut self,
        outer_curves: Vec<Option<CurveGeom>>,
        hole_curves: Vec<Vec<Option<CurveGeom>>>,
    ) {
        debug_assert_eq!(outer_curves.len(), self.outer.len());
        debug_assert_eq!(hole_curves.len(), self.holes.len());
        debug_assert!(
            hole_curves
                .iter()
                .zip(&self.holes)
                .all(|(c, h)| c.len() == h.len())
        );
        self.outer_curves = outer_curves;
        self.hole_curves = hole_curves;
    }

    /// The supporting plane.
    pub fn plane(&self) -> Plane {
        self.plane
    }

    /// Outer boundary, CCW seen from the plane normal side.
    pub fn outer(&self) -> &[Point3] {
        &self.outer
    }

    /// Hole boundaries, each CW seen from the plane normal side.
    pub fn holes(&self) -> &[Vec<Point3>] {
        &self.holes
    }

    /// The analytic circle outer edge `k` (`outer[k] → outer[k+1]`) is a
    /// facet of, or `None` (plain line, or no attribution attached).
    pub fn outer_curve(&self, k: usize) -> Option<CurveGeom> {
        self.outer_curves.get(k).copied().flatten()
    }

    /// The analytic circle hole `i`'s edge `k` is a facet of, or `None`.
    pub fn hole_curve(&self, i: usize, k: usize) -> Option<CurveGeom> {
        self.hole_curves
            .get(i)
            .and_then(|h| h.get(k))
            .copied()
            .flatten()
    }
}

// ----------------------------------------------------------------- helpers

/// Check that no two consecutive points (including the wrap-around pair) are
/// within `tol::POINT_MERGE` of each other.
fn profile_check_degenerate(pts: &[Point3]) -> Result<(), ProfileError> {
    let n = pts.len();
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if a.approx_eq(b, crate::tol::POINT_MERGE) {
            return Err(ProfileError::DegenerateEdge);
        }
    }
    Ok(())
}

/// Delegate to the shared geom2d helper and map the error.
fn profile_check_simple(pts: &[Point3]) -> Result<(), ProfileError> {
    if polygon_is_simple(pts) {
        Ok(())
    } else {
        Err(ProfileError::SelfIntersecting)
    }
}

// ═════════════════════════════════════════════════════════════════════ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Plane;

    fn xy_plane() -> Plane {
        Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
    }

    fn pt(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    /// Build a rectangle sketch and return the single region.
    fn make_rect_sketch(x0: f64, y0: f64, x1: f64, y1: f64) -> Sketch {
        let mut s = Sketch::on_plane(xy_plane());
        let segs = [
            (pt(x0, y0), pt(x1, y0)),
            (pt(x1, y0), pt(x1, y1)),
            (pt(x1, y1), pt(x0, y1)),
            (pt(x0, y1), pt(x0, y0)),
        ];
        for (a, b) in &segs {
            s.add_segment(*a, *b).unwrap();
        }
        s
    }

    /// A copy that lands across its own source splits the source's edges in
    /// the target; the seams it carries are read from the source, so a seam
    /// at a split edge's old end does not name a stale edge (a rotate-copy
    /// array of a shape touching a face's edge once panicked here).
    #[test]
    fn a_copy_across_its_source_carries_seams_without_stale_edges() {
        let mut s = make_rect_sketch(0.0, 0.0, 2.0, 1.0);
        s.set_seams(vec![pt(0.0, 0.0), pt(2.0, 0.0)]);
        let edges: Vec<SketchEdgeId> = s.edges().keys().collect();
        let about = |p: Point3| {
            crate::Transform::translation(Vec3::new(-p.x, -p.y, 0.0))
                .then(
                    &crate::Transform::rotation(
                        Vec3::new(0.0, 0.0, 1.0),
                        std::f64::consts::FRAC_PI_4,
                    )
                    .unwrap(),
                )
                .then(&crate::Transform::translation(Vec3::new(p.x, p.y, 0.0)))
        };
        s.copy_edges_transformed(&edges, &about(pt(1.0, 0.5)))
            .unwrap();
        assert!(s.edges().len() > 8, "the copy crossed and split its source");
        assert!(
            s.seams().len() >= 3,
            "the corners' seams travel with the copy"
        );
    }

    // ── Angular sorting around a 4-way crossing vertex ────────────────────────

    /// After inserting two crossing segments (+), the crossing vertex must have
    /// 4 outgoing directed half-edges sorted in angular order.  We verify by
    /// checking the region structure: a 4-way crossing of equal-length arms
    /// produces 4 triangular regions (none actually, since the arms extend to
    /// different lengths — we use the simpler rectangle-then-chord case).
    #[test]
    fn angular_sort_4way_crossing() {
        // Draw a cross (+) with all four arms meeting at the origin.
        // Arm segments: (-1,0)-(1,0) and (0,-1)-(0,1).
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(-1.0, 0.0), pt(1.0, 0.0)).unwrap();
        s.add_segment(pt(0.0, -1.0), pt(0.0, 1.0)).unwrap();
        // 4 arms meeting at origin — no closed region since arms are open.
        assert_eq!(s.regions().len(), 0);

        // Now close them into a square.
        s.add_segment(pt(-1.0, -1.0), pt(1.0, -1.0)).unwrap();
        s.add_segment(pt(1.0, -1.0), pt(1.0, 1.0)).unwrap();
        s.add_segment(pt(1.0, 1.0), pt(-1.0, 1.0)).unwrap();
        s.add_segment(pt(-1.0, 1.0), pt(-1.0, -1.0)).unwrap();
        // The cross divides the square into 4 equal quadrant regions.
        assert_eq!(s.regions().len(), 4);
    }

    /// Regression: reconstructing a 3D position from 2D plane coordinates must
    /// restore the plane's offset, not assume the plane passes through the
    /// origin. Before the fix, the interior crossing vertex landed at z=0.
    #[test]
    fn intersection_vertices_lie_on_an_offset_plane() {
        let offset = 5.0;
        let plane = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, offset),
            Point3::new(1.0, 0.0, offset),
            Point3::new(0.0, 1.0, offset),
        ])
        .unwrap();
        let mut s = Sketch::on_plane(plane);
        s.add_segment(
            Point3::new(-1.0, 0.0, offset),
            Point3::new(1.0, 0.0, offset),
        )
        .unwrap();
        // This crosses the first segment, creating an interior vertex.
        s.add_segment(
            Point3::new(0.0, -1.0, offset),
            Point3::new(0.0, 1.0, offset),
        )
        .unwrap();

        for v in s.vertices().values() {
            assert!(
                (v.position.z - offset).abs() <= crate::tol::PLANE_DIST,
                "vertex left the plane: {:?}",
                v.position
            );
        }
        assert!(
            s.vertices().values().any(|v| v
                .position
                .approx_eq(Point3::new(0.0, 0.0, offset), crate::tol::POINT_MERGE)),
            "crossing vertex not found on the offset plane"
        );
    }

    // ── Triangle region (non-axis-aligned edges) ──────────────────────────────

    #[test]
    fn triangle_region_non_axis_aligned() {
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(0.0, 0.0), pt(3.0, 0.0)).unwrap();
        s.add_segment(pt(3.0, 0.0), pt(1.5, 2.0)).unwrap();
        s.add_segment(pt(1.5, 2.0), pt(0.0, 0.0)).unwrap();
        assert_eq!(s.regions().len(), 1);
        let region = s.regions().values().next().unwrap();
        assert_eq!(region.outer.len(), 3);
        assert!(region.holes.is_empty());
        // Verify CCW winding.
        let pts: Vec<Point3> = region
            .outer
            .iter()
            .map(|&vid| s.vertices()[vid].position)
            .collect();
        let area = signed_area_on_plane(&pts, xy_plane().normal());
        assert!(area > 0.0, "outer boundary should be CCW");
    }

    // ── Nested concentric rings: outer-with-hole, not self-as-hole ────────────

    /// Two concentric regular n-gons (radii 1.0 and 0.5) must trace as exactly
    /// two regions: the outer ring (outer r=1 with the r=0.5 loop as a hole) and
    /// the inner disk (outer r=0.5, no holes). Regression for the facet-count
    /// sensitive nesting bug: the inner loop's CW hole-cycle used to attach to
    /// its own reverse-wound CCW twin (whose area equals the hole's only up to
    /// floating point) instead of the true r=1 encloser — leaving the outer ring
    /// hole-less and the inner disk holed by itself. That surfaced downstream as
    /// a boolean `DegenerateContact` / push-through `NonManifoldResult` on
    /// faceted solids (n = 5, 6, 16, 24 failed; 3, 4, 8, 12 happened to pass).
    #[test]
    fn concentric_rings_nest_outer_over_inner_for_all_facet_counts() {
        for n in [3usize, 4, 5, 6, 8, 12, 16, 24, 32, 48, 64] {
            let ngon = |r: f64| -> Vec<Point3> {
                (0..n)
                    .map(|i| {
                        let a = std::f64::consts::TAU * (i as f64) / (n as f64);
                        Point3::new(r * a.cos(), r * a.sin(), 0.0)
                    })
                    .collect()
            };
            let mut s = Sketch::on_plane(xy_plane());
            let add_loop = |s: &mut Sketch, pts: &[Point3]| {
                for i in 0..pts.len() {
                    s.add_segment(pts[i], pts[(i + 1) % pts.len()]).unwrap();
                }
            };
            add_loop(&mut s, &ngon(1.0));
            add_loop(&mut s, &ngon(0.5));

            let regs: Vec<&SketchRegion> = s.regions().values().collect();
            assert_eq!(regs.len(), 2, "n={n}: expected ring + inner disk");

            let radius = |vids: &[SketchVertexId]| {
                let p = s.vertices()[vids[0]].position;
                (p.x * p.x + p.y * p.y).sqrt()
            };
            let ring = regs
                .iter()
                .find(|r| radius(&r.outer) > 0.75)
                .unwrap_or_else(|| panic!("n={n}: no r=1 outer region"));
            let disk = regs
                .iter()
                .find(|r| radius(&r.outer) < 0.75)
                .unwrap_or_else(|| panic!("n={n}: no r=0.5 outer region"));

            assert_eq!(ring.holes.len(), 1, "n={n}: outer ring must have one hole");
            assert!(
                (radius(&ring.holes[0]) - 0.5).abs() < 1e-9,
                "n={n}: the ring's hole must be the r=0.5 loop"
            );
            assert!(
                disk.holes.is_empty(),
                "n={n}: the inner disk must not hole itself"
            );
        }
    }

    // ── Collinear merge: new segment entirely inside an existing edge ─────────

    #[test]
    fn collinear_new_inside_existing() {
        let mut s = Sketch::on_plane(xy_plane());
        // Existing edge spans [0,4].
        s.add_segment(pt(0.0, 0.0), pt(4.0, 0.0)).unwrap();
        // New segment is [1,3], entirely inside the existing edge.
        s.add_segment(pt(1.0, 0.0), pt(3.0, 0.0)).unwrap();
        // Result: three abutting edges [0,1], [1,3], [3,4].
        // Vertices: 0, 1, 3, 4 = 4 vertices; edges: 3.
        assert_eq!(s.vertices().len(), 4);
        assert_eq!(s.edges().len(), 3);
        assert_eq!(s.regions().len(), 0);
    }

    // ── Region identity: unchanged region keeps its id ────────────────────────

    #[test]
    fn region_identity_stable_across_disjoint_add() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        let id_before: Vec<SketchRegionId> = s.regions().keys().collect();
        assert_eq!(id_before.len(), 1);

        // Add a segment far away that doesn't touch the rectangle.
        s.add_segment(pt(10.0, 10.0), pt(11.0, 10.0)).unwrap();

        // The rectangle region should still be present (same outer cycle).
        // It will have a fresh id (our implementation re-inserts), but it
        // should NOT appear in regions_created (since cycles_equal matched it).
        // We verify by checking the total region count stays 1.
        assert_eq!(s.regions().len(), 1);
    }

    // ── Chord across region produces correct diff counts ─────────────────────

    #[test]
    fn chord_diff_counts() {
        let mut s = make_rect_sketch(0.0, 0.0, 2.0, 2.0);
        let report = s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();
        assert_eq!(report.regions_removed.len(), 1);
        assert_eq!(report.regions_created.len(), 2);
        assert_eq!(s.regions().len(), 2);
    }

    /// Drawing a curve over an existing plain collinear line adopts that
    /// line into the curve — no untagged hole in the selectable chain. An
    /// edge already claimed by another curve keeps its allegiance.
    #[test]
    fn curve_adopts_overlapped_plain_edges_but_not_other_curves() {
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(1.0, 0.0), pt(2.0, 0.0)).unwrap(); // plain line
        let c1 = s.begin_curve();
        // A longer collinear segment spanning the plain line.
        s.add_segment(pt(0.0, 0.0), pt(3.0, 0.0)).unwrap();
        s.end_curve();
        assert_eq!(
            s.curve_edges(c1).len(),
            3,
            "all three pieces (including the adopted one) are the curve"
        );

        // A second curve over c1's territory does NOT steal its edges.
        let c2 = s.begin_curve();
        s.add_segment(pt(0.0, 0.0), pt(2.0, 0.0)).unwrap();
        s.end_curve();
        assert_eq!(s.curve_edges(c1).len(), 3, "c1 keeps its edges");
        assert_eq!(s.curve_edges(c2).len(), 0, "fully-overlapped c2 owns none");
    }

    /// A closed curve touching other geometry at two vertices splits into
    /// two selectable runs at those junctions; an untouched curve is one
    /// run; a plain line is just itself.
    #[test]
    fn curve_chain_stops_at_junctions() {
        let mut s = Sketch::on_plane(xy_plane());
        // A diamond "circle" of four facets…
        let c = s.begin_curve();
        for (a, b) in [
            (pt(1.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 2.0)),
            (pt(1.0, 2.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(1.0, 0.0)),
        ] {
            s.add_segment(a, b).unwrap();
        }
        s.end_curve();
        let any_edge = s.curve_edges(c)[0];
        assert_eq!(
            s.curve_chain_at(any_edge).len(),
            4,
            "untouched curve selects whole"
        );

        // …then a line through two opposite vertices makes them junctions.
        s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();
        for &e in &s.curve_edges(c) {
            assert_eq!(
                s.curve_chain_at(e).len(),
                2,
                "each side of the crossing selects separately"
            );
        }

        // Plain lines select alone.
        let plain = s
            .edges()
            .iter()
            .find(|(_, e)| e.curve.is_none())
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(s.curve_chain_at(plain), vec![plain]);
    }

    // ── islands ───────────────────────────────────────────────────────────────

    fn two_rect_sketch() -> Sketch {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        for (a, b) in [
            (pt(3.0, 0.0), pt(4.0, 0.0)),
            (pt(4.0, 0.0), pt(4.0, 1.0)),
            (pt(4.0, 1.0), pt(3.0, 1.0)),
            (pt(3.0, 1.0), pt(3.0, 0.0)),
        ] {
            s.add_segment(a, b).unwrap();
        }
        s
    }

    /// Two disjoint rectangles are two islands; ids survive an unrelated
    /// third shape appearing, and the survivor keeps its id when the other
    /// island loses an edge.
    #[test]
    fn disjoint_shapes_are_separate_islands_with_stable_ids() {
        let mut s = two_rect_sketch();
        assert_eq!(s.islands().len(), 2);
        let before: Vec<SketchIslandId> = s.islands().keys().collect();

        s.add_segment(pt(6.0, 0.0), pt(7.0, 0.0)).unwrap();
        assert_eq!(s.islands().len(), 3);
        for id in &before {
            assert!(s.islands().contains_key(*id), "existing island ids survive");
        }

        // Remove one edge of the first rectangle: both islands keep ids.
        let some_edge = s
            .edges()
            .iter()
            .find(|(_, e)| s.vertices()[e.from].position.x < 2.0)
            .map(|(id, _)| id)
            .unwrap();
        s.remove_edge(some_edge).unwrap();
        for id in &before {
            assert!(s.islands().contains_key(*id), "ids survive an edge removal");
        }
    }

    /// A segment bridging two islands merges them into one (the id with the
    /// smaller anchor edge survives); removing the bridge splits them again
    /// and the anchor-holding component keeps the id.
    #[test]
    fn bridging_merges_islands_and_unbridging_splits_them() {
        let mut s = two_rect_sketch();
        let ids: Vec<SketchIslandId> = s.islands().keys().collect();
        let survivor = *s
            .islands()
            .iter()
            .min_by_key(|(_, isl)| isl.edges[0])
            .map(|(id, _)| id)
            .iter()
            .next()
            .unwrap();

        let report = s.add_segment(pt(1.0, 0.5), pt(3.0, 0.5)).unwrap();
        let _ = report;
        assert_eq!(s.islands().len(), 1, "bridged into one island");
        assert!(s.islands().contains_key(survivor));

        // Remove the bridge (the edge whose midpoint sits between the rects).
        let bridge = s
            .edges()
            .iter()
            .find(|(_, e)| {
                let m = (s.vertices()[e.from].position.x + s.vertices()[e.to].position.x) / 2.0;
                (1.0..3.0).contains(&m) && m > 1.01 && m < 2.99
            })
            .map(|(id, _)| id)
            .unwrap();
        s.remove_edge(bridge).unwrap();
        assert_eq!(s.islands().len(), 2, "split back into two");
        assert!(s.islands().contains_key(survivor), "anchor holder keeps id");
        // One of the two is the survivor; the other is fresh or the old
        // second id — either way there are exactly two distinct live ids.
        let _ = ids;
    }

    /// Moving one island rigidly succeeds when it lands clear, refuses when
    /// it would cross the other island, and refuses off-plane transforms —
    /// untouched on every refusal.
    #[test]
    fn island_transform_moves_one_shape_and_arranges_on_contact() {
        let left_of = |s: &Sketch| {
            s.islands()
                .iter()
                .find(|(_, isl)| {
                    let e = s.edges()[isl.edges[0]];
                    s.vertices()[e.from].position.x < 2.0
                })
                .map(|(id, _)| id)
                .unwrap()
        };
        let mut s = two_rect_sketch();
        let left = left_of(&s);

        // Clear landing: shift left rect up by 5. Moves in place, ids stable.
        let up = crate::Transform::translation(crate::math::Vec3::new(0.0, 5.0, 0.0));
        s.apply_transform_island(left, &up).unwrap();
        assert_eq!(s.islands().len(), 2, "still two islands");
        let e = s.edges()[s.islands()[left].edges[0]];
        assert!(s.vertices()[e.from].position.y >= 5.0 - 1e-9);

        // Overlapping landing: drop it half onto the right rectangle. It
        // arranges exactly as the same rectangle DRAWN there would.
        let onto = crate::Transform::translation(crate::math::Vec3::new(3.5, -5.0, 0.0));
        s.apply_transform_island(left, &onto).unwrap();
        let mut drawn = make_rect_sketch(3.5, 0.0, 4.5, 1.0);
        for (a, b) in [
            (pt(3.0, 0.0), pt(4.0, 0.0)),
            (pt(4.0, 0.0), pt(4.0, 1.0)),
            (pt(4.0, 1.0), pt(3.0, 1.0)),
            (pt(3.0, 1.0), pt(3.0, 0.0)),
        ] {
            drawn.add_segment(a, b).unwrap();
        }
        assert_eq!(s.islands().len(), 1, "the landing merged the two shapes");
        assert_eq!(
            s.regions().len(),
            drawn.regions().len(),
            "move == draw: regions"
        );
        assert_eq!(
            s.regions().len(),
            3,
            "two crescents' worth: left, overlap, right"
        );
        assert_eq!(s.edges().len(), drawn.edges().len(), "move == draw: edges");

        // Off-plane landing.
        let lift = crate::Transform::translation(crate::math::Vec3::new(0.0, 0.0, 1.0));
        let any = s.islands().keys().next().unwrap();
        assert!(matches!(
            s.apply_transform_island(any, &lift).unwrap_err(),
            SketchError::PointOffPlane { .. }
        ));
    }

    // ── curve chains ──────────────────────────────────────────────────────────

    /// Edges committed inside a begin_curve/end_curve bracket share one
    /// curve id; edges outside carry none.
    #[test]
    fn curve_bracket_tags_only_its_edges() {
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap(); // plain line
        let curve = s.begin_curve();
        s.add_segment(pt(0.0, 1.0), pt(0.5, 1.2)).unwrap();
        s.add_segment(pt(0.5, 1.2), pt(1.0, 1.0)).unwrap();
        s.end_curve();
        s.add_segment(pt(0.0, 2.0), pt(1.0, 2.0)).unwrap(); // plain line

        assert_eq!(s.curve_edges(curve).len(), 2);
        let tagged: Vec<_> = s
            .edges()
            .iter()
            .filter(|(_, e)| e.curve.is_some())
            .collect();
        assert_eq!(tagged.len(), 2);
        for (eid, _) in tagged {
            assert_eq!(s.edge_curve(eid), Some(curve));
        }
    }

    /// Splitting a curve edge (a line drawn across an arc facet) leaves the
    /// fragments in the curve — the arc stays one selectable unit.
    #[test]
    fn split_curve_edge_fragments_inherit_the_curve() {
        let mut s = Sketch::on_plane(xy_plane());
        let curve = s.begin_curve();
        s.add_segment(pt(0.0, 1.0), pt(2.0, 1.0)).unwrap();
        s.end_curve();
        // Cross it with a plain line — the curve edge splits in two.
        s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();

        assert_eq!(
            s.curve_edges(curve).len(),
            2,
            "both fragments stay in the curve"
        );
        let plain = s.edges().values().filter(|e| e.curve.is_none()).count();
        assert_eq!(plain, 2, "the crossing line's fragments stay plain");
    }

    // ── spur pruning ──────────────────────────────────────────────────────────

    /// A dangling chain hanging into a region's interior is decoration, not
    /// boundary: the region's outer cycle is the clean silhouette (no
    /// repeated vertices) and converts to a valid profile. This is the
    /// leftover-arc-facet case: a stray interior line must not make the
    /// region unextrudable.
    #[test]
    fn interior_spur_is_pruned_from_the_region_boundary() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // Whisker from the (1,1) corner into the interior.
        s.add_segment(pt(1.0, 1.0), pt(0.5, 0.5)).unwrap();

        assert_eq!(s.regions().len(), 1, "the square still reads as one region");
        let (rid, r) = s.regions().iter().next().unwrap();
        assert_eq!(r.outer.len(), 4, "the spur is not part of the boundary");
        let mut seen = std::collections::BTreeSet::new();
        assert!(
            r.outer.iter().all(|v| seen.insert(*v)),
            "no repeated boundary vertices"
        );
        assert!(s.profile(rid).is_ok(), "the region converts to a profile");
    }

    /// Same, with the spur hanging off a mid-edge vertex and two segments
    /// long — the chain folds up tip-first and the split boundary vertex
    /// stays.
    #[test]
    fn two_segment_spur_off_a_split_edge_is_pruned() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        s.add_segment(pt(0.5, 1.0), pt(0.5, 0.6)).unwrap();
        s.add_segment(pt(0.5, 0.6), pt(0.4, 0.4)).unwrap();

        assert_eq!(s.regions().len(), 1);
        let (rid, r) = s.regions().iter().next().unwrap();
        // 4 corners + the split vertex at (0.5, 1).
        assert_eq!(r.outer.len(), 5);
        assert!(s.profile(rid).is_ok());
    }

    // ── region_scaffolding / remove_edges ────────────────────────────────────

    /// A lone rectangle's scaffolding is all 4 edges; deleting them leaves
    /// an empty sketch (vertices die with their last edge; regions and
    /// islands recompute to nothing).
    #[test]
    fn scaffolding_of_a_lone_rect_is_all_its_edges() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        let region = s.regions().keys().next().expect("one region");
        let edges = s.region_scaffolding(region).expect("live region");
        assert_eq!(edges.len(), 4, "all 4 edges are exclusive scaffolding");

        s.remove_edges(&edges);
        assert!(s.edges().is_empty(), "edges deleted");
        assert!(s.vertices().is_empty(), "orphaned vertices deleted");
        assert!(s.regions().is_empty(), "no region survives");
        assert!(s.islands().is_empty(), "no island survives");
    }

    /// Two rectangles sharing the x=1 wall: the left region's scaffolding
    /// has 3 edges — the shared wall still bounds the right region and must
    /// survive, keeping the neighbor closed.
    #[test]
    fn scaffolding_excludes_an_edge_shared_with_a_surviving_region() {
        let mut s = Sketch::on_plane(xy_plane());
        // Left rect [0,0]-[1,1].
        for (a, b) in &[
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(1.0, 0.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(0.0, 0.0)),
        ] {
            s.add_segment(*a, *b).unwrap();
        }
        // Right rect [1,0]-[2,1] — shares the x=1 wall.
        for (a, b) in &[
            (pt(1.0, 0.0), pt(2.0, 0.0)),
            (pt(2.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(1.0, 0.0)),
        ] {
            s.add_segment(*a, *b).unwrap();
        }
        assert_eq!(s.regions().len(), 2);

        // The left region: all outer vertices are at x <= 1.
        let left_region = s
            .regions()
            .iter()
            .find(|(_, r)| {
                r.outer
                    .iter()
                    .all(|&vid| s.vertices()[vid].position.x < 1.5)
            })
            .map(|(id, _)| id)
            .expect("left region");
        let right_region = s
            .regions()
            .keys()
            .find(|&id| id != left_region)
            .expect("right region");

        let edges = s.region_scaffolding(left_region).expect("live region");
        assert_eq!(edges.len(), 3, "3 exclusive edges (not the shared wall)");

        s.remove_edges(&edges);
        // The right region survives WITH ITS ID (its outer cycle is
        // untouched), the shared wall still bounds it, and only the two
        // x=0 corners died.
        assert!(s.regions().contains_key(right_region), "neighbor stays");
        assert_eq!(s.regions().len(), 1);
        assert_eq!(s.edges().len(), 4, "the right rect keeps all 4 walls");
        assert!(
            s.vertices().values().all(|v| v.position.x > 0.5),
            "only the x=0 corners died"
        );
    }

    /// Consuming the left region then the right deletes everything: the
    /// shared wall dies exactly when the LAST region needing it goes.
    #[test]
    fn shared_wall_dies_with_the_last_region_needing_it() {
        let mut s = Sketch::on_plane(xy_plane());
        for (a, b) in &[
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(1.0, 0.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(0.0, 0.0)),
            (pt(1.0, 0.0), pt(2.0, 0.0)),
            (pt(2.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 1.0)),
        ] {
            s.add_segment(*a, *b).unwrap();
        }
        assert_eq!(s.regions().len(), 2);

        let first = s.regions().keys().next().expect("first region");
        let edges = s.region_scaffolding(first).expect("live");
        s.remove_edges(&edges);
        assert_eq!(s.regions().len(), 1, "one region left");

        let second = s.regions().keys().next().expect("second region");
        let edges = s.region_scaffolding(second).expect("live");
        assert_eq!(edges.len(), s.edges().len(), "now ALL edges are exclusive");
        s.remove_edges(&edges);
        assert!(s.edges().is_empty());
        assert!(s.vertices().is_empty());
    }

    /// An open chain hanging off a consumed region's corner survives the
    /// deletion, and so does the corner vertex it needs.
    #[test]
    fn open_chains_survive_region_scaffolding_deletion() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // Open whisker off the (1,1) corner — bounds no region.
        s.add_segment(pt(1.0, 1.0), pt(2.0, 2.0)).unwrap();
        assert_eq!(s.regions().len(), 1);

        let region = s.regions().keys().next().expect("one region");
        let edges = s.region_scaffolding(region).expect("live");
        assert_eq!(edges.len(), 4, "the whisker edge is not scaffolding");

        s.remove_edges(&edges);
        assert_eq!(s.edges().len(), 1, "the whisker survives");
        assert_eq!(s.vertices().len(), 2, "the whisker keeps its corner vertex");
        assert_eq!(s.islands().len(), 1, "the whisker is one island");
    }

    /// A stale region handle is a typed error, and removing an empty edge
    /// set is a no-op.
    #[test]
    fn scaffolding_of_a_stale_region_errors() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // The null key (default) is never inserted by slotmap.
        assert_eq!(
            s.region_scaffolding(SketchRegionId::default()),
            Err(SketchError::UnknownRegion)
        );
        let before_edges = s.edges().len();
        s.remove_edges(&std::collections::BTreeSet::new());
        assert_eq!(s.edges().len(), before_edges, "empty removal is a no-op");
    }

    /// A curve chain partially deleted by consumption keeps its analytic
    /// geometry on the surviving edges: deletion removes facets but never
    /// deforms the remaining ones, so the circle stays a true description.
    #[test]
    fn partial_curve_chain_keeps_valid_geometry_after_removal() {
        let mut s = Sketch::on_plane(xy_plane());
        let geom = CurveGeom {
            center: pt(0.0, 0.0),
            radius: 1.0,
        };
        let curve = s.begin_curve_with(geom).expect("curve opens");
        // Three facets of the unit circle.
        let a = pt(1.0, 0.0);
        let b = pt(0.0, 1.0);
        let c = pt(-1.0, 0.0);
        let d = pt(0.0, -1.0);
        s.add_segment(a, b).unwrap();
        s.add_segment(b, c).unwrap();
        s.add_segment(c, d).unwrap();
        s.end_curve();

        // Delete the middle facet.
        let middle = s
            .edges()
            .iter()
            .find(|(_, e)| {
                let f = s.vertices()[e.from].position;
                let t = s.vertices()[e.to].position;
                // The b→c facet: midpoint at (-0.5, 0.5).
                (f.x + t.x) * 0.5 < -0.4 && (f.y + t.y) * 0.5 > 0.4
            })
            .map(|(id, _)| id)
            .expect("middle facet");
        s.remove_edges(&std::collections::BTreeSet::from([middle]));

        assert_eq!(s.edges().len(), 2, "two facets survive");
        assert_eq!(
            s.curve_geom(curve),
            Some(geom),
            "the surviving facets still lie on the drawn circle"
        );
        for e in s.edges().values() {
            assert_eq!(e.curve, Some(curve), "chain identity survives");
        }
    }
}
