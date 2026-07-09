//! Hand-rolled spatial index over the scene's snap candidates: an [`Aabb`]
//! plus a binary BVH ([`Bvh`]) per candidate set (points, segments, faces).
//!
//! DEVELOPMENT.md rule 1 keeps this crate free of outside dependencies, so
//! both structures are hand-rolled here rather than pulled from a crate. A
//! BVH (median split over box centroids) fits the query mix better than the
//! mesh-heal-style uniform hash grid: the hot queries are rays and pick
//! cones over faces of wildly varying size, which a hierarchy prunes by
//! region regardless of primitive scale, while a grid needs a well-chosen
//! cell size to avoid degenerating back into a linear walk.
//!
//! Every node test in this module is **conservative by construction**: it
//! may admit a box the true cone/ray misses, but it never rejects a box
//! containing a primitive that could pass the exact per-primitive test.
//! Callers re-run the exact geometry test on every admitted primitive, so
//! the indexed paths return byte-for-byte the same results as a linear
//! scan — the index only prunes, it never decides.

use kernel::{Point3, Vec3, tol};

use crate::{SceneFace, ScenePoint, SceneSegment};

/// Maximum primitives per BVH leaf: small enough that a leaf visit stays
/// cheap, large enough to keep the tree shallow. A structural knob, not a
/// geometric tolerance.
const LEAF_SIZE: usize = 8;

/// An axis-aligned bounding box (f64 meters, DEVELOPMENT.md rule 6).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Aabb {
    min: Point3,
    max: Point3,
}

impl Aabb {
    /// The empty box (union identity): min > max on every axis, so every
    /// query test rejects it and the first `include` overwrites it.
    const EMPTY: Aabb = Aabb {
        min: Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        max: Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
    };

    /// The degenerate box containing exactly `p`.
    fn of_point(p: Point3) -> Aabb {
        Aabb { min: p, max: p }
    }

    /// Grows the box to contain `p`.
    fn include(&mut self, p: Point3) {
        self.min = Point3::new(
            self.min.x.min(p.x),
            self.min.y.min(p.y),
            self.min.z.min(p.z),
        );
        self.max = Point3::new(
            self.max.x.max(p.x),
            self.max.y.max(p.y),
            self.max.z.max(p.z),
        );
    }

    /// The smallest box containing both inputs.
    fn union(a: Aabb, b: Aabb) -> Aabb {
        let mut out = a;
        out.include(b.min);
        out.include(b.max);
        out
    }

    /// Grown by [`tol::POINT_MERGE`] on every side. Geometry closer than the
    /// coincidence tolerance is the same point (rule 6), so the inflation can
    /// only admit extra primitives for the exact tests to reject — it can
    /// never lose one to floating-point rounding at a box boundary.
    fn padded(self) -> Aabb {
        let pad = Vec3::new(tol::POINT_MERGE, tol::POINT_MERGE, tol::POINT_MERGE);
        Aabb {
            min: self.min + pad * -1.0,
            max: self.max + pad,
        }
    }

    /// Box center (NaN for [`Aabb::EMPTY`], which every test then rejects).
    fn center(self) -> Point3 {
        Point3::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    /// Half the diagonal length: the radius of the box's bounding sphere.
    fn half_diagonal(self) -> f64 {
        (self.max - self.min).length() * 0.5
    }

    /// Slab test: the ray parameter at which `origin + t·dir` (t ≥ 0, `dir`
    /// normalized) first enters the box, or `None` if the ray misses it.
    ///
    /// Zero direction components produce ±∞ slab bounds, which the
    /// NaN-dropping `f64::min`/`max` fold handles; the one true 0·∞ = NaN
    /// case (origin exactly on a zero-direction slab bound) conservatively
    /// treats that axis as unconstrained rather than corrupting the range.
    fn ray_entry(self, origin: Point3, dir: Vec3) -> Option<f64> {
        let o = [origin.x, origin.y, origin.z];
        let lo = [self.min.x, self.min.y, self.min.z];
        let hi = [self.max.x, self.max.y, self.max.z];
        let d = [dir.x, dir.y, dir.z];
        let mut tmin = 0.0_f64;
        let mut tmax = f64::INFINITY;
        for axis in 0..3 {
            let inv = 1.0 / d[axis];
            let t0 = (lo[axis] - o[axis]) * inv;
            let t1 = (hi[axis] - o[axis]) * inv;
            if t0.is_nan() || t1.is_nan() {
                continue; // 0·∞: axis unconstrained (conservative)
            }
            tmin = tmin.max(t0.min(t1));
            tmax = tmax.min(t0.max(t1));
        }
        (tmin <= tmax).then_some(tmin)
    }

    /// Conservative cone-vs-box test: `false` only when NO point of the box
    /// can pass [`crate::cone_test`] for a cone of half-angle `atan(tan_aperture)`
    /// around the (normalized) ray. `tan_aperture == None` means a half-angle
    /// within [`tol::CONE_SLACK`] of 90° or more — the cone is (conservatively)
    /// the whole front half-space, so only the behind-the-origin rejection
    /// applies.
    ///
    /// Uses the box's bounding sphere: a candidate at depth `t` and angle
    /// `θ ≤ aperture` sits within `t·tan(aperture)` of the ray line, so the
    /// sphere-center distance to the (origin-clamped) ray is at most that
    /// plus the sphere radius, and `t` is at most the center depth plus the
    /// radius. Testing against those bounds admits a superset.
    fn maybe_in_cone(self, origin: Point3, dir: Vec3, tan_aperture: Option<f64>) -> bool {
        let c = self.center();
        let r = self.half_diagonal();
        let t_center = (c - origin).dot(dir);
        let t_far = t_center + r;
        if t_far <= 0.0 {
            // Everything in the box is at depth ≤ 0: `cone_test` admits only
            // candidates strictly in front of the origin.
            return false;
        }
        let Some(tan_aperture) = tan_aperture else {
            return true;
        };
        // Guard band ([`tol::CONE_SLACK`]): `cone_test`'s `acos(depth/dist)`
        // is √-amplified rounding — an ulp-scale error in the cosine admits
        // candidates up to ~4e-8 rad OUTSIDE the exact cone (at the extreme,
        // the quotient rounds to 1.0 and the computed angle saturates to 0,
        // passing any aperture). A cut at the mathematically exact cone
        // boundary would prune boxes holding such accepted candidates; the
        // POINT_MERGE box padding (1e-9 m, absolute) does not cover a slack
        // that grows linearly with depth. The additive `CONE_SLACK · t_far`
        // term dominates the saturation band, and the `1 + CONE_SLACK`
        // factor dominates the same cosine error at large apertures (cone
        // pruning is disabled within CONE_SLACK of π/2 — see `resolve_impl`).
        //
        // The ray-only paths need no such band, deliberately: the exact
        // predicates behind `ray_candidates`/`any_hit_before` (`pick_face`,
        // `is_occluded`) are pure algebraic sign and ordering comparisons
        // with no inverse trig, so their rounding stays linear in the ulp
        // (~1e-16 · coordinate scale) and the POINT_MERGE padding already
        // dominates it. Likewise the `t_far <= 0.0` cut above: `cone_test`
        // rejects on the plain sign of a dot product (linear error, covered
        // by the padding), and with cone pruning active every accepted
        // candidate has cos θ ≥ cos(aperture) − e ≥ sin(CONE_SLACK) − e > 0,
        // i.e. strictly positive depth.
        let closest = origin + dir * t_center.max(0.0);
        let slack = tol::CONE_SLACK;
        (c - closest).length() - r <= (tan_aperture * (1.0 + slack) + slack) * t_far
    }
}

/// A binary bounding-volume hierarchy over pre-built primitive boxes.
/// Leaves carry indices into the candidate `Vec` the boxes were built from.
#[derive(Debug, Clone)]
pub(crate) struct Bvh {
    /// Node 0 is the root (empty when there are no primitives).
    nodes: Vec<Node>,
    /// Primitive indices, contiguous per leaf.
    items: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    aabb: Aabb,
    kind: NodeKind,
}

#[derive(Debug, Clone, Copy)]
enum NodeKind {
    /// Children are indices into `Bvh::nodes`.
    Internal { left: u32, right: u32 },
    /// Primitives are `items[start..start + len]`.
    Leaf { start: u32, len: u32 },
}

impl Bvh {
    /// Builds the hierarchy over `boxes[i]` for primitive index `i`.
    /// Deterministic (DEVELOPMENT.md §7): the split ordering ties break on
    /// the primitive index, never on ambient state.
    fn build(boxes: &[Aabb]) -> Bvh {
        let mut items: Vec<u32> = (0..boxes.len() as u32).collect();
        let mut nodes = Vec::new();
        if !items.is_empty() {
            build_node(boxes, &mut items, 0, boxes.len(), &mut nodes);
        }
        Bvh { nodes, items }
    }

    /// Ascending indices of primitives whose box may intersect the pick
    /// cone — a conservative superset (see [`Aabb::maybe_in_cone`]); the
    /// caller re-runs the exact test. Ascending order keeps candidate
    /// emission identical to the linear scan, so ranking ties break the
    /// same way on both paths.
    pub(crate) fn cone_candidates(
        &self,
        origin: Point3,
        dir: Vec3,
        tan_aperture: Option<f64>,
    ) -> Vec<usize> {
        self.collect(|aabb| aabb.maybe_in_cone(origin, dir, tan_aperture))
    }

    /// Ascending indices of primitives whose box the ray `origin + t·dir`
    /// (t ≥ 0) crosses — a conservative superset for exact ray-primitive
    /// tests, in linear-scan order like [`Bvh::cone_candidates`].
    pub(crate) fn ray_candidates(&self, origin: Point3, dir: Vec3) -> Vec<usize> {
        self.collect(|aabb| aabb.ray_entry(origin, dir).is_some())
    }

    /// Depth-first collection of leaf primitives under nodes admitted by
    /// `admit`, returned ascending.
    fn collect(&self, admit: impl Fn(Aabb) -> bool) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<u32> = if self.nodes.is_empty() {
            vec![]
        } else {
            vec![0]
        };
        while let Some(n) = stack.pop() {
            let node = self.nodes[n as usize];
            if !admit(node.aabb) {
                continue;
            }
            match node.kind {
                NodeKind::Internal { left, right } => {
                    stack.push(left);
                    stack.push(right);
                }
                NodeKind::Leaf { start, len } => out.extend(
                    self.items[start as usize..(start + len) as usize]
                        .iter()
                        .map(|&i| i as usize),
                ),
            }
        }
        out.sort_unstable();
        out
    }

    /// Early-out any-hit ray walk: returns `true` as soon as `hit` accepts a
    /// primitive from a subtree whose box the ray enters at t < `t_before`.
    ///
    /// Sound pruning: a primitive's exact hit point lies inside its own box,
    /// so its hit-t is never smaller than the box's entry-t — a subtree
    /// entered at or beyond `t_before` cannot contain a qualifying hit. The
    /// nearer child is visited first so a real occluder ends the walk early;
    /// visit order cannot change the boolean result.
    pub(crate) fn any_hit_before(
        &self,
        origin: Point3,
        dir: Vec3,
        t_before: f64,
        mut hit: impl FnMut(usize) -> bool,
    ) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let entry = |n: u32| {
            self.nodes[n as usize]
                .aabb
                .ray_entry(origin, dir)
                .filter(|&t| t < t_before)
        };
        let mut stack: Vec<u32> = Vec::new();
        if entry(0).is_some() {
            stack.push(0);
        }
        while let Some(n) = stack.pop() {
            match self.nodes[n as usize].kind {
                NodeKind::Internal { left, right } => match (entry(left), entry(right)) {
                    (Some(lt), Some(rt)) => {
                        // Push the farther child first: the stack pops the
                        // nearer one next.
                        if lt <= rt {
                            stack.push(right);
                            stack.push(left);
                        } else {
                            stack.push(left);
                            stack.push(right);
                        }
                    }
                    (Some(_), None) => stack.push(left),
                    (None, Some(_)) => stack.push(right),
                    (None, None) => {}
                },
                NodeKind::Leaf { start, len } => {
                    for &i in &self.items[start as usize..(start + len) as usize] {
                        if hit(i as usize) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Recursive median-split build over `items[start..end]`; returns the index
/// of the created node in `nodes`.
fn build_node(
    boxes: &[Aabb],
    items: &mut Vec<u32>,
    start: usize,
    end: usize,
    nodes: &mut Vec<Node>,
) -> u32 {
    let mut aabb = Aabb::EMPTY;
    let mut centroid_bounds = Aabb::EMPTY;
    for &i in &items[start..end] {
        aabb = Aabb::union(aabb, boxes[i as usize]);
        centroid_bounds.include(boxes[i as usize].center());
    }
    let this = nodes.len() as u32;
    // Provisional leaf; patched to Internal below if the range splits.
    nodes.push(Node {
        aabb,
        kind: NodeKind::Leaf {
            start: start as u32,
            len: (end - start) as u32,
        },
    });

    let extent = centroid_bounds.max - centroid_bounds.min;
    let (axis_extent, axis) = [(extent.x, 0usize), (extent.y, 1), (extent.z, 2)]
        .into_iter()
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .expect("three axes");
    // Split only when the range is big enough AND there is something to
    // split by: coincident centroids — and non-finite degenerate boxes,
    // whose extent is NaN — keep the leaf.
    let can_split = end - start > LEAF_SIZE && axis_extent > 0.0;
    if !can_split {
        return this;
    }

    // Median split ordered by (centroid coordinate, primitive index): the
    // index tiebreak keeps the build fully deterministic.
    let coord = |i: u32| -> f64 {
        let c = boxes[i as usize].center();
        [c.x, c.y, c.z][axis]
    };
    items[start..end].sort_unstable_by(|&a, &b| {
        coord(a)
            .partial_cmp(&coord(b))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    let mid = start + (end - start) / 2;
    let left = build_node(boxes, items, start, mid, nodes);
    let right = build_node(boxes, items, mid, end, nodes);
    nodes[this as usize].kind = NodeKind::Internal { left, right };
    this
}

/// The scene-wide spatial index: one BVH per indexed candidate set. Guides,
/// world axes/origin (constant count) and sketch/transient segments
/// (gesture-scoped, small) deliberately stay on the linear path in
/// `lib.rs` — indexing them buys nothing.
#[derive(Debug, Clone)]
pub(crate) struct SceneIndex {
    points: Bvh,
    segments: Bvh,
    faces: Bvh,
}

impl SceneIndex {
    /// Builds the index from the scene's current candidate Vecs. Leaves
    /// carry indices into those Vecs, so any mutation that reorders or
    /// resizes them must invalidate the whole index.
    pub(crate) fn build(
        points: &[ScenePoint],
        segments: &[SceneSegment],
        faces: &[SceneFace],
    ) -> SceneIndex {
        let point_boxes: Vec<Aabb> = points
            .iter()
            .map(|p| Aabb::of_point(p.position).padded())
            .collect();
        let segment_boxes: Vec<Aabb> = segments
            .iter()
            .map(|s| {
                let mut b = Aabb::of_point(s.a);
                b.include(s.b);
                b.padded()
            })
            .collect();
        let face_boxes: Vec<Aabb> = faces.iter().map(face_aabb).collect();
        SceneIndex {
            points: Bvh::build(&point_boxes),
            segments: Bvh::build(&segment_boxes),
            faces: Bvh::build(&face_boxes),
        }
    }

    /// Point candidates for `resolve`'s Endpoint tests (superset, ascending).
    pub(crate) fn points_in_cone(
        &self,
        origin: Point3,
        dir: Vec3,
        tan_aperture: Option<f64>,
    ) -> Vec<usize> {
        self.points.cone_candidates(origin, dir, tan_aperture)
    }

    /// Segment candidates for `resolve`'s Midpoint/OnEdge tests (superset,
    /// ascending). Both exact candidate positions — the midpoint and the
    /// closest point on the segment — lie inside the segment's box, so the
    /// cone-vs-box test covers them.
    pub(crate) fn segments_in_cone(
        &self,
        origin: Point3,
        dir: Vec3,
        tan_aperture: Option<f64>,
    ) -> Vec<usize> {
        self.segments.cone_candidates(origin, dir, tan_aperture)
    }

    /// Face candidates for ray-hit tests (`resolve`'s OnFace and
    /// `pick_face`), superset, ascending. `face_cone_hit` ignores the
    /// aperture for faces — a face hit is pure ray-polygon containment — so
    /// this is a ray query, not a cone query.
    pub(crate) fn faces_crossing_ray(&self, origin: Point3, dir: Vec3) -> Vec<usize> {
        self.faces.ray_candidates(origin, dir)
    }

    /// Occlusion early-out over faces (see [`Bvh::any_hit_before`]).
    pub(crate) fn any_face_hit_before(
        &self,
        origin: Point3,
        dir: Vec3,
        t_before: f64,
        hit: impl FnMut(usize) -> bool,
    ) -> bool {
        self.faces.any_hit_before(origin, dir, t_before, hit)
    }
}

/// The box of a face's snappable surface. `face_cone_hit` intersects the ray
/// with the face's *fitted plane* and containment-tests the projected outer
/// boundary, so the exact hit region is the boundary's shadow ON the plane —
/// and imported faces may sit up to [`tol::IMPORT_PLANE_DIST`] off their
/// fitted plane. Covering each boundary vertex AND its projection onto the
/// plane bounds that region exactly (it is contained in the convex hull of
/// the projections), with no scale-dependent guess.
fn face_aabb(face: &SceneFace) -> Aabb {
    let n = face.plane.normal();
    let mut b = Aabb::EMPTY;
    for &p in &face.boundary {
        b.include(p);
        b.include(p + n * -face.plane.signed_distance(p));
    }
    b.padded()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes_on_a_line(n: usize) -> Vec<Aabb> {
        (0..n)
            .map(|i| {
                let mut b = Aabb::of_point(Point3::new(i as f64, 0.0, 0.0));
                b.include(Point3::new(i as f64 + 0.5, 1.0, 1.0));
                b.padded()
            })
            .collect()
    }

    #[test]
    fn ray_entry_hits_and_misses() {
        let mut b = Aabb::of_point(Point3::new(1.0, 1.0, 1.0));
        b.include(Point3::new(2.0, 2.0, 2.0));
        // Straight into the box.
        let t = b
            .ray_entry(Point3::new(1.5, 1.5, -1.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("ray enters the box");
        assert!((t - 2.0).abs() <= tol::POINT_MERGE);
        // Pointing away: behind-origin boxes are misses.
        assert!(
            b.ray_entry(Point3::new(1.5, 1.5, -1.0), Vec3::new(0.0, 0.0, -1.0))
                .is_none()
        );
        // Zero direction component with the origin inside the slab: still a
        // hit (axis unconstrained), no NaN corruption.
        let t = b
            .ray_entry(Point3::new(1.5, 1.5, 0.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("ray enters the box from below");
        assert!(t > 0.0);
        // Zero direction component with the origin outside the slab: a miss.
        assert!(
            b.ray_entry(Point3::new(5.0, 1.5, 0.0), Vec3::new(0.0, 0.0, 1.0))
                .is_none()
        );
        // Origin inside the box: entry clamps to 0.
        let t = b
            .ray_entry(Point3::new(1.5, 1.5, 1.5), Vec3::new(1.0, 0.0, 0.0))
            .expect("origin is inside");
        assert_eq!(t, 0.0);
    }

    #[test]
    fn cone_test_rejects_behind_and_far_off_axis() {
        let b = Aabb::of_point(Point3::new(0.0, 0.0, 10.0)).padded();
        let origin = Point3::ORIGIN;
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let tan = Some(0.1_f64.tan());
        assert!(b.maybe_in_cone(origin, dir, tan), "on-axis box is admitted");
        let behind = Aabb::of_point(Point3::new(0.0, 0.0, -10.0)).padded();
        assert!(!behind.maybe_in_cone(origin, dir, tan));
        let off_axis = Aabb::of_point(Point3::new(50.0, 0.0, 10.0)).padded();
        assert!(!off_axis.maybe_in_cone(origin, dir, tan));
        // A wide-open cone (None) still rejects behind-the-origin boxes.
        assert!(!behind.maybe_in_cone(origin, dir, None));
        assert!(off_axis.maybe_in_cone(origin, dir, None));
    }

    #[test]
    fn cone_test_saturation_band_is_admitted() {
        // The box of a 2 × 1e-4 × 2 plate whose bounding-sphere silhouette
        // grazes the ray: the corner at the origin is ~1.15e-8 rad off a
        // 1e-8 rad cone — outside the exact cone, but inside `cone_test`'s
        // acos-saturation window, so the exact test accepts it. The guard
        // band must admit the box (the regression spec in inference_specs
        // pins the end-to-end resolve == resolve_linear consequence).
        let mut b = Aabb::of_point(Point3::ORIGIN);
        b.include(Point3::new(2.0, 1e-4, 2.0));
        let b = b.padded();
        let origin = Point3::new(-1.0606601717798211e-7, -13.0, -1.0606601717798211e-7);
        let dir = Vec3::new(0.0, 1.0, 0.0);
        assert!(b.maybe_in_cone(origin, dir, Some(1e-8_f64.tan())));
    }

    #[test]
    fn build_is_deterministic_and_covers_all_items() {
        let boxes = boxes_on_a_line(37);
        let a = Bvh::build(&boxes);
        let b = Bvh::build(&boxes);
        assert_eq!(a.items, b.items, "identical input, identical build");
        // A whole-space query returns every primitive exactly once, sorted.
        let all = a.ray_candidates(Point3::new(-1.0, 0.5, 0.5), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(all, (0..37).collect::<Vec<_>>());
    }

    #[test]
    fn empty_bvh_answers_queries_without_panicking() {
        let bvh = Bvh::build(&[]);
        let origin = Point3::ORIGIN;
        let dir = Vec3::new(0.0, 0.0, 1.0);
        assert!(bvh.cone_candidates(origin, dir, Some(1.0)).is_empty());
        assert!(bvh.ray_candidates(origin, dir).is_empty());
        assert!(!bvh.any_hit_before(origin, dir, f64::INFINITY, |_| true));
    }
}
