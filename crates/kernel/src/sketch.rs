//! The draft context: 2D sticky geometry on a plane, before any solid exists.
//!
//! A [`Sketch`] is where SketchUp-style drawing happens *outside* an Object
//! (ARCHITECTURE.md): line/rectangle tools feed it segments, sticky rules apply
//! (endpoints merge, crossings split), and closed circuits become
//! [`SketchRegion`]s. A region converts to a [`Profile`] — a validated closed
//! planar polygon with holes — which is the input to
//! `Object::from_extrusion`: extruding a Profile is THE way solids are born
//! (solids-by-default, docs/DEVELOPMENT.md).
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
//!
//! M0 status: storage and types are real; algorithms are `todo!()` stubs.
//! The executable contract lives in `crates/kernel/tests/op_specs.rs`.

use slotmap::{SlotMap, new_key_type};

use crate::math::{Plane, Point3};

new_key_type! {
    /// Handle to a [`SketchVertex`].
    pub struct SketchVertexId;
    /// Handle to a [`SketchEdge`].
    pub struct SketchEdgeId;
    /// Handle to a [`SketchRegion`].
    pub struct SketchRegionId;
}

/// A point in a sketch. Always on the sketch plane
/// (within [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)).
#[derive(Debug, Clone, Copy)]
pub struct SketchVertex {
    /// Position in f64 meters (world frame).
    pub position: Point3,
}

/// A straight segment between two sketch vertices.
///
/// Edges never overlap or cross other edges of the same sketch — the sticky
/// rules split/merge eagerly at insertion time, so this is an invariant, not
/// a hope.
#[derive(Debug, Clone, Copy)]
pub struct SketchEdge {
    /// Start vertex.
    pub from: SketchVertexId,
    /// End vertex.
    pub to: SketchVertexId,
}

/// A closed region bounded by sketch edges: what the user perceives as "a
/// face appeared" while drawing.
#[derive(Debug, Clone)]
pub struct SketchRegion {
    /// Outer boundary vertices in cycle order, counter-clockwise seen from
    /// the plane normal side.
    pub outer: Vec<SketchVertexId>,
    /// Hole boundaries, each clockwise seen from the plane normal side.
    pub holes: Vec<Vec<SketchVertexId>>,
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
    /// The given region handle is not in this sketch (or already invalid).
    UnknownRegion,
}

impl std::fmt::Display for SketchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SketchError::PointOffPlane { which } => {
                write!(f, "input point {which} is off the sketch plane")
            }
            SketchError::DegenerateSegment => write!(f, "segment endpoints coincide"),
            SketchError::UnknownEdge => write!(f, "no such edge in this sketch"),
            SketchError::UnknownRegion => write!(f, "no such region in this sketch"),
        }
    }
}

impl std::error::Error for SketchError {}

/// A planar drawing surface with sticky-geometry semantics.
#[derive(Debug, Clone)]
pub struct Sketch {
    plane: Plane,
    vertices: SlotMap<SketchVertexId, SketchVertex>,
    edges: SlotMap<SketchEdgeId, SketchEdge>,
    regions: SlotMap<SketchRegionId, SketchRegion>,
}

impl Sketch {
    /// An empty sketch on `plane`.
    pub fn on_plane(plane: Plane) -> Sketch {
        Sketch {
            plane,
            vertices: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            regions: SlotMap::with_key(),
        }
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

    /// Current closed regions (read-only). Kept up to date by every mutation;
    /// never recomputed lazily.
    pub fn regions(&self) -> &SlotMap<SketchRegionId, SketchRegion> {
        &self.regions
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
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn add_segment(&mut self, from: Point3, to: Point3) -> Result<SegmentAdded, SketchError> {
        todo!("M1: sticky segment insertion (see module docs and tests/op_specs.rs)")
    }

    /// Removes one edge (the eraser tool), deleting vertices that become
    /// unused and dissolving/merging the regions it bounded.
    ///
    /// # Errors
    /// [`SketchError::UnknownEdge`] if the handle is stale. On error the
    /// sketch is unchanged.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn remove_edge(&mut self, edge: SketchEdgeId) -> Result<EdgeRemoved, SketchError> {
        todo!("M1: edge removal with region dissolution")
    }

    /// Extracts a region as a validated [`Profile`] ready for extrusion.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale (regions die
    /// whenever a mutation reshapes them — always re-query after mutating).
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn profile(&self, region: SketchRegionId) -> Result<Profile, SketchError> {
        todo!("M1: region -> Profile extraction")
    }
}

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
        })
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

/// Signed area of a polygon against the given normal, using the 2-D cross
/// product projected onto the plane.  Positive = CCW seen from the normal
/// side.
fn signed_area_on_plane(pts: &[Point3], normal: crate::math::Vec3) -> f64 {
    // Pick two axes perpendicular to the normal via a stable cross product.
    let (u, v) = plane_axes(normal);
    let n = pts.len();
    let mut area = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let ax = a.to_vec().dot(u);
        let ay = a.to_vec().dot(v);
        let bx = b.to_vec().dot(u);
        let by = b.to_vec().dot(v);
        area += ax * by - bx * ay;
    }
    area * 0.5
}

/// Returns two orthonormal vectors (u, v) that span the plane with the given
/// normal.  The result is stable for any non-zero normal.
fn plane_axes(normal: crate::math::Vec3) -> (crate::math::Vec3, crate::math::Vec3) {
    use crate::math::Vec3;
    // Choose a reference vector not parallel to normal.
    let reference = if normal.x.abs() <= normal.y.abs() && normal.x.abs() <= normal.z.abs() {
        Vec3::new(1.0, 0.0, 0.0)
    } else if normal.y.abs() <= normal.z.abs() {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let u = normal
        .cross(reference)
        .normalized()
        .expect("plane_axes: normal is non-zero");
    let v = normal
        .cross(u)
        .normalized()
        .expect("plane_axes: u is non-zero");
    (u, v)
}

/// Check that a polygon does not cross or touch itself.  Tests every pair of
/// non-adjacent edges for intersection.
fn profile_check_simple(pts: &[Point3]) -> Result<(), ProfileError> {
    let n = pts.len();
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        // Skip edges that share a vertex with edge i.
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue; // edges 0 and n-1 share vertex 0
            }
            let c = pts[j];
            let d = pts[(j + 1) % n];
            if segments_intersect(a, b, c, d) {
                return Err(ProfileError::SelfIntersecting);
            }
        }
    }
    Ok(())
}

/// Check whether two separate polygons share any point, cross, or touch.
fn boundaries_contact(a: &[Point3], b: &[Point3]) -> bool {
    let na = a.len();
    let nb = b.len();
    for i in 0..na {
        let p = a[i];
        let q = a[(i + 1) % na];
        for j in 0..nb {
            let r = b[j];
            let s = b[(j + 1) % nb];
            if segments_intersect(p, q, r, s) {
                return true;
            }
        }
    }
    false
}

/// 2-D segment intersection test (using the plane projection implicitly via
/// the signed-area / cross-product test in 3-D).  Returns true if the open
/// segments (p,q) and (r,s) properly cross, or if any endpoint touches the
/// other segment (closed-interval test).
fn segments_intersect(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
    let d1 = cross_z(r, s, p);
    let d2 = cross_z(r, s, q);
    let d3 = cross_z(p, q, r);
    let d4 = cross_z(p, q, s);

    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    // Collinear / endpoint cases.
    let eps = crate::tol::POINT_MERGE;
    if d1.abs() <= eps && on_segment(r, s, p) {
        return true;
    }
    if d2.abs() <= eps && on_segment(r, s, q) {
        return true;
    }
    if d3.abs() <= eps && on_segment(p, q, r) {
        return true;
    }
    if d4.abs() <= eps && on_segment(p, q, s) {
        return true;
    }
    false
}

/// Signed 2-D cross product of (q-p) × (r-p) using the 3-D cross product
/// and extracting the dominant axis (which is the normal axis for coplanar
/// points).
fn cross_z(p: Point3, q: Point3, r: Point3) -> f64 {
    let pq = q - p;
    let pr = r - p;
    let c = pq.cross(pr);
    // Return the component along the dominant axis of the cross product.
    if c.z.abs() >= c.x.abs() && c.z.abs() >= c.y.abs() {
        c.z
    } else if c.y.abs() >= c.x.abs() {
        c.y
    } else {
        c.x
    }
}

/// True if point `t` lies on the line segment `[p, q]` (collinearity assumed).
fn on_segment(p: Point3, q: Point3, t: Point3) -> bool {
    let min_x = p.x.min(q.x);
    let max_x = p.x.max(q.x);
    let min_y = p.y.min(q.y);
    let max_y = p.y.max(q.y);
    let min_z = p.z.min(q.z);
    let max_z = p.z.max(q.z);
    t.x >= min_x - crate::tol::POINT_MERGE
        && t.x <= max_x + crate::tol::POINT_MERGE
        && t.y >= min_y - crate::tol::POINT_MERGE
        && t.y <= max_y + crate::tol::POINT_MERGE
        && t.z >= min_z - crate::tol::POINT_MERGE
        && t.z <= max_z + crate::tol::POINT_MERGE
}

/// Point-in-polygon test using the ray-casting method projected onto the
/// plane spanned by `normal`. Returns `true` only for strictly interior
/// points; boundary points return `false` (strict interior required for
/// `HoleOutsideOuter`).
fn point_inside_polygon(pt: Point3, poly: &[Point3], normal: crate::math::Vec3) -> bool {
    let (u, v) = plane_axes(normal);
    let px = pt.to_vec().dot(u);
    let py = pt.to_vec().dot(v);

    let n = poly.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let ai_x = poly[i].to_vec().dot(u);
        let ai_y = poly[i].to_vec().dot(v);
        let aj_x = poly[j].to_vec().dot(u);
        let aj_y = poly[j].to_vec().dot(v);

        if ((ai_y > py) != (aj_y > py)) && (px < (aj_x - ai_x) * (py - ai_y) / (aj_y - ai_y) + ai_x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}
