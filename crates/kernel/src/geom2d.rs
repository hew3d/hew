//! Crate-internal 2-D geometry helpers (point-in-polygon, plane axes).
//!
//! These are used by both `sketch` (profile validation) and `ops` (hole
//! reassignment during `split_face`).  Nothing here is public API.
//!
//! Segment contact is decided by exact orientation signs for crossings and a
//! metric `tol::POINT_MERGE` distance for touches (DEVELOPMENT.md rule 6);
//! no test compares a squared quantity against the metre tolerance.

use crate::math::{Point3, Vec3};
use crate::tol;

/// Returns two orthonormal vectors (u, v) that span the plane with the given
/// normal.  The result is stable for any non-zero normal.
pub(crate) fn plane_axes(normal: Vec3) -> (Vec3, Vec3) {
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

/// Signed area of a polygon against the given normal, using the 2-D cross
/// product projected onto the plane.  Positive = CCW seen from the normal
/// side.
pub(crate) fn signed_area_on_plane(pts: &[Point3], normal: Vec3) -> f64 {
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

/// Point-in-polygon test using the ray-casting method projected onto the
/// plane spanned by `normal`. Returns `true` for strictly interior points
/// and `false` for strictly exterior ones. A point ON the boundary gets
/// either answer: the ray's half-open edge convention reads some boundary
/// edges and corners as inside and others as outside, and which ones depends
/// on the axes [`plane_axes`] picks for `normal`. A caller that can meet a
/// boundary point must exclude it first.
pub(crate) fn point_inside_polygon(pt: Point3, poly: &[Point3], normal: Vec3) -> bool {
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

/// Whether two segments cross at a single interior point of both — a
/// PROPER crossing: touching at an endpoint, sharing a point of one with the
/// interior of the other, or lying along each other all answer false.
///
/// Decided in two parts, neither scaling with segment length: the crossing
/// itself is exact (the orientation signs of [`segments_cross_exactly`]),
/// and any contact within [`tol::POINT_MERGE`] metres of an end
/// ([`segments_touch`]) disqualifies it. (An earlier form compared
/// orientation cross products — square metres — against the metre
/// tolerance, so its dead band grew and shrank with the segments.) The
/// tolerance is the kernel's one length tolerance (DEVELOPMENT.md rule 6).
pub(crate) fn segments_cross_properly(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
    segments_cross_exactly(p, q, r, s) && !segments_touch(p, q, r, s)
}

/// Whether an end of either segment lies within [`tol::POINT_MERGE`] of the
/// other segment (an end touching, or the two running along each other).
fn segments_touch(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
    let eps = tol::POINT_MERGE;
    point_near_segment(p, r, s, eps)
        || point_near_segment(q, r, s, eps)
        || point_near_segment(r, p, q, eps)
        || point_near_segment(s, p, q, eps)
}

/// Whether segments `p–q` and `r–s` (coplanar) share a point: they cross,
/// or an end of one lies within [`tol::POINT_MERGE`] of the other (an end
/// touching, or the two running along each other).
///
/// A crossing is decided EXACTLY, with no tolerance: the four orientation
/// signs (each end against the other segment's line) are computed by an
/// adaptive exact predicate, so a proper crossing is never missed however
/// close to an end it falls — at kilometre coordinates, an acute crossing
/// a nanometre from an end of each segment once read as no contact, every
/// end being just over the tolerance from the other segment's body. Every
/// other contact is METRIC — a distance in metres against `POINT_MERGE`,
/// the tolerance every other contact gate in the kernel uses — so nothing
/// is reported that is neither a crossing nor a touch.
pub(crate) fn segments_intersect(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
    segments_touch(p, q, r, s) || segments_cross_exactly(p, q, r, s)
}

/// Whether coplanar segments `p–q` and `r–s` cross at a point interior to
/// both, decided by exact orientation signs in the plane's projection onto
/// the two world axes its normal is least aligned with. Four points lying
/// along one line to rounding have no plane to project into: no crossing
/// (any contact between such segments is an end within rounding of the
/// other's line, which the distance tests see).
fn segments_cross_exactly(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
    let (pq, rs) = (q - p, s - r);
    let normal = [pq.cross(rs), pq.cross(r - p), rs.cross(p - r)]
        .into_iter()
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .expect("three candidates");
    let reach = pq.length() + rs.length() + (r - p).length();
    if normal.length() <= COLLINEAR_FLOOR * reach * reach {
        return false;
    }
    let (ax, ay) = match (normal.x.abs(), normal.y.abs(), normal.z.abs()) {
        (x, y, z) if z >= x && z >= y => (0, 1),
        (x, y, _) if y >= x => (2, 0),
        _ => (1, 2),
    };
    let at = |pt: Point3, k: usize| match k {
        0 => pt.x,
        1 => pt.y,
        _ => pt.z,
    };
    let o = |a: Point3, b: Point3, c: Point3| {
        orient2d(
            (at(a, ax), at(a, ay)),
            (at(b, ax), at(b, ay)),
            (at(c, ax), at(c, ay)),
        )
    };
    o(r, s, p) * o(r, s, q) < 0 && o(p, q, r) * o(p, q, s) < 0
}

/// Below this many units of rounding (relative to the squared extent of
/// the points), a normal estimate is rounding noise: the points lie along
/// one line. An arithmetic bound, not a geometric tolerance.
const COLLINEAR_FLOOR: f64 = 64.0 * f64::EPSILON;

/// Shewchuk's first error bound for the 2-D orientation determinant:
/// `(3 + 16ε)ε` with `ε = 2⁻⁵³`. A float determinant larger than this
/// times its two products' magnitudes has the exact sign.
const ORIENT_ERR_BOUND: f64 = (3.0 + 8.0 * f64::EPSILON) * (f64::EPSILON / 2.0);

/// The exact sign (+1 counter-clockwise, −1 clockwise, 0 collinear) of
/// `(b − a) × (c − a)`: a floating-point evaluation when its error bound
/// proves the sign, else an exact expansion (every difference and product
/// kept as an unevaluated sum of doubles).
pub(crate) fn orient2d(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> i32 {
    let left = (b.0 - a.0) * (c.1 - a.1);
    let right = (b.1 - a.1) * (c.0 - a.0);
    let det = left - right;
    if det.abs() > ORIENT_ERR_BOUND * (left.abs() + right.abs()) {
        return if det > 0.0 { 1 } else { -1 };
    }
    let (abx, abx_e) = two_sum(b.0, -a.0);
    let (aby, aby_e) = two_sum(b.1, -a.1);
    let (acx, acx_e) = two_sum(c.0, -a.0);
    let (acy, acy_e) = two_sum(c.1, -a.1);
    let mut e: Vec<f64> = Vec::with_capacity(16);
    for (u, v, sign) in [
        (abx, acy, 1.0),
        (abx, acy_e, 1.0),
        (abx_e, acy, 1.0),
        (abx_e, acy_e, 1.0),
        (aby, acx, -1.0),
        (aby, acx_e, -1.0),
        (aby_e, acx, -1.0),
        (aby_e, acx_e, -1.0),
    ] {
        let (prod, err) = two_product(u, v);
        grow_expansion(&mut e, sign * err);
        grow_expansion(&mut e, sign * prod);
    }
    match e.iter().rev().find(|&&x| x != 0.0) {
        Some(&x) if x > 0.0 => 1,
        Some(_) => -1,
        None => 0,
    }
}

/// `a + b` as an exact two-term sum `(rounded, error)`.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bv = s - a;
    let av = s - bv;
    (s, (a - av) + (b - bv))
}

/// `a · b` as an exact two-term product `(rounded, error)`.
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    (p, a.mul_add(b, -p))
}

/// Adds `b` to the nonoverlapping expansion `e` exactly (components in
/// increasing magnitude; the last nonzero one carries the sign).
fn grow_expansion(e: &mut Vec<f64>, b: f64) {
    let mut q = b;
    for x in e.iter_mut() {
        let (sum, err) = two_sum(q, *x);
        *x = err;
        q = sum;
    }
    e.push(q);
}

/// True if `p` lies within [`tol::POINT_MERGE`] of some edge of the closed
/// ring `ring`: a boundary point, which [`point_inside_polygon`] answers
/// either way.
pub(crate) fn point_on_ring(p: Point3, ring: &[Point3]) -> bool {
    let n = ring.len();
    (0..n).any(|i| point_near_segment(p, ring[i], ring[(i + 1) % n], tol::POINT_MERGE))
}

/// True if `p` lies on the face region bounded by `outer` and `holes`
/// (all on one plane with normal `normal`, and `p` on that plane): inside
/// `outer` and outside every hole, or on one of the loops. A point on a loop lies on the face;
/// the inside test alone would answer it by which edge it sits on.
pub(crate) fn point_on_region(
    p: Point3,
    outer: &[Point3],
    holes: &[Vec<Point3>],
    normal: Vec3,
) -> bool {
    point_on_ring(p, outer)
        || holes.iter().any(|h| point_on_ring(p, h))
        || (point_inside_polygon(p, outer, normal)
            && !holes.iter().any(|h| point_inside_polygon(p, h, normal)))
}

/// True if `p` lies within `tol` of the closed segment `[a, b]`.
pub(crate) fn point_near_segment(p: Point3, a: Point3, b: Point3, tol: f64) -> bool {
    // A point farther than `tol` from the segment's bounding box is farther
    // than `tol` from the segment. The box is grown by twice `tol`, so a
    // nearest point that rounding puts a hair outside the box never turns
    // an answer: this only skips the arithmetic below for far points, which
    // the scans over every edge of a large plane are made of.
    let m = 2.0 * tol;
    if p.x < a.x.min(b.x) - m
        || p.x > a.x.max(b.x) + m
        || p.y < a.y.min(b.y) - m
        || p.y > a.y.max(b.y) + m
        || p.z < a.z.min(b.z) - m
        || p.z > a.z.max(b.z) + m
    {
        return false;
    }
    let ab = b - a;
    let len_sq = ab.length_squared();
    let t = if len_sq <= tol * tol {
        0.0
    } else {
        ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length_squared() <= tol * tol
}

/// A point strictly inside the material of a polygon-with-holes (inside
/// `outer`, outside every hole), or `None` for degenerate loops.
///
/// Scanline method: pick the plane-axis-horizontal line whose ordinate is the
/// midpoint of the widest gap between distinct vertex ordinates (maximally
/// far from every vertex, so no edge is grazed), intersect it with every loop
/// edge, and take the midpoint of the widest inside interval under the
/// even-odd rule. Deterministic, and works for concave outers and any hole
/// arrangement — unlike a centroid, which can land outside or in a hole.
pub(crate) fn interior_point_of_loops(
    outer: &[Point3],
    holes: &[Vec<Point3>],
    normal: Vec3,
) -> Option<Point3> {
    if outer.len() < 3 {
        return None;
    }
    let (u, v) = plane_axes(normal);
    let px = |p: Point3| p.to_vec().dot(u);
    let py = |p: Point3| p.to_vec().dot(v);
    // {u, v, normal} is orthonormal, so a point on the plane reconstructs
    // exactly from its three projections; the normal component is shared by
    // every loop point.
    let w = outer[0].to_vec().dot(normal);

    let mut ys: Vec<f64> = outer.iter().map(|&p| py(p)).collect();
    let outer_min = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let outer_max = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for h in holes {
        ys.extend(h.iter().map(|&p| py(p)));
    }
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() <= tol::POINT_MERGE);

    // Vertex-free ordinate bands, clamped to the outer loop's range so the
    // scanline is guaranteed to cross it, widest first: the widest band can
    // lie wholly inside a hole that spans the loop (an enclosed imprint
    // flush with two of its sides), so the next is tried until a scanline
    // finds material.
    let mut bands: Vec<(f64, f64)> = Vec::new(); // (gap, midpoint)
    for pair in ys.windows(2) {
        let (lo, hi) = (pair[0].max(outer_min), pair[1].min(outer_max));
        let gap = hi - lo;
        if gap > tol::POINT_MERGE {
            bands.push((gap, (lo + hi) * 0.5));
        }
    }
    // Widest first; equal gaps keep their ordinate order (stable sort).
    bands.sort_by(|a, b| b.0.total_cmp(&a.0));
    bands.into_iter().find_map(|(_, y_star)| {
        // Every crossing of the scanline with every loop edge, in plane-x.
        let mut xs: Vec<f64> = Vec::new();
        let mut collect = |lp: &[Point3]| {
            let n = lp.len();
            for i in 0..n {
                let (a, b) = (lp[i], lp[(i + 1) % n]);
                let (ax, ay) = (px(a), py(a));
                let (bx, by) = (px(b), py(b));
                if (ay > y_star) != (by > y_star) {
                    xs.push(ax + (bx - ax) * (y_star - ay) / (by - ay));
                }
            }
        };
        collect(outer);
        for h in holes {
            collect(h);
        }
        if xs.len() < 2 {
            return None;
        }
        xs.sort_by(f64::total_cmp);

        // Even-odd: intervals (xs[0], xs[1]), (xs[2], xs[3]), … are material.
        let mut best: Option<(f64, f64)> = None; // (width, midpoint)
        for pair in xs.chunks_exact(2) {
            let width = pair[1] - pair[0];
            if width > tol::POINT_MERGE && best.is_none_or(|(g, _)| width > g) {
                best = Some((width, (pair[0] + pair[1]) * 0.5));
            }
        }
        let (_, x_star) = best?;
        Some(Point3::ORIGIN + u * x_star + v * y_star + normal * w)
    })
}

/// Check whether two separate polygons share any point, cross, or touch.
pub(crate) fn boundaries_contact(a: &[Point3], b: &[Point3]) -> bool {
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

/// Whether two separate polygons share any point other than vertices both
/// of them hold (the same exact position in each): a hole touching its
/// face's outer loop, or another hole, at one shared vertex is a point
/// contact the face is allowed (every loop simple, loops sharing isolated
/// vertices, never an edge), while any other touch, a crossing, or two
/// edges running along each other is contact.
pub(crate) fn boundaries_contact_away_from_shared(a: &[Point3], b: &[Point3]) -> bool {
    let bits = |p: Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let shared: Vec<[u64; 3]> = a
        .iter()
        .map(|&p| bits(p))
        .filter(|k| b.iter().any(|&q| bits(q) == *k))
        .collect();
    let na = a.len();
    let nb = b.len();
    for i in 0..na {
        let (p, q) = (a[i], a[(i + 1) % na]);
        for j in 0..nb {
            let (r, s) = (b[j], b[(j + 1) % nb]);
            if !segments_intersect(p, q, r, s) {
                continue;
            }
            // Two segments meeting at a vertex both rings share touch only
            // there unless one runs along the other.
            let common = [p, q]
                .into_iter()
                .find(|&x| shared.contains(&bits(x)) && (bits(x) == bits(r) || bits(x) == bits(s)));
            match common {
                Some(x) => {
                    let far_a = if bits(p) == bits(x) { q } else { p };
                    let far_b = if bits(r) == bits(x) { s } else { r };
                    if point_near_segment(far_a, r, s, tol::POINT_MERGE)
                        || point_near_segment(far_b, p, q, tol::POINT_MERGE)
                    {
                        return true;
                    }
                }
                None => return true,
            }
        }
    }
    false
}

/// Whether hole ring `hole` leaves the region of outer ring `outer` (both
/// on one plane with unit normal `normal`): some stretch of a hole edge
/// lies outside the outer ring by more than `graze`.
///
/// Each hole edge is cut at every outer vertex lying on it (within
/// [`tol::POINT_MERGE`]) and, when `graze` exceeds `POINT_MERGE`, at every
/// point where it properly crosses an outer edge. Every piece's midpoint must
/// be inside the outer ring, or on it: running along an outer edge (allowed
/// only when `allow_overlap`: an imported hole lying on its face's
/// boundary), or within `graze` of it. A piece outside is a GRAZE, not an
/// escape, when no point of it lies farther than `graze` from the ring: an
/// imported ring, flat only to its planarity tolerance, may clip a corner
/// or an edge of its outer loop by that much. That is decided by geometry,
/// not sampling: for each outer edge, the stretch of the piece within
/// `graze` of it is one interval (a line meets the edge's capsule in a
/// convex set), and the intervals together must cover the piece (exact up
/// to the root-finding's conditioning; see [`capsule_interval`]). With `graze` at `POINT_MERGE`
/// (native geometry) the rule is exact: any proper crossing leaves, and a
/// piece is on the ring by [`point_near_segment`].
///
/// Points of the two rings may touch: a hole vertex shared with the outer
/// ring, or on an outer edge, and an outer vertex on a hole edge, are point
/// contacts, not escapes. Cutting at outer vertices is what catches a hole
/// edge running out between two of them through a notch in the outer ring,
/// with both its ends and its midpoint in material.
pub(crate) fn hole_leaves_outer(
    outer: &[Point3],
    hole: &[Point3],
    normal: Vec3,
    graze: f64,
    allow_overlap: bool,
) -> bool {
    let (no, nh) = (outer.len(), hole.len());
    let on_ring = |p: Point3| {
        (0..no).any(|k| point_near_segment(p, outer[k], outer[(k + 1) % no], tol::POINT_MERGE))
    };
    let grazing = graze > tol::POINT_MERGE;
    for i in 0..nh {
        let (a, b) = (hole[i], hole[(i + 1) % nh]);
        let d = b - a;
        let len2 = d.length_squared();
        let mut cuts: Vec<f64> = vec![0.0, 1.0];
        for k in 0..no {
            let (c, e) = (outer[k], outer[(k + 1) % no]);
            if !segments_cross_properly(a, b, c, e) {
                continue;
            }
            if !grazing {
                return true;
            }
            // The crossing's parameter along the hole edge.
            let f = e - c;
            let den = d.cross(f).dot(normal);
            if den.abs() > 0.0 {
                cuts.push(((c - a).cross(f).dot(normal) / den).clamp(0.0, 1.0));
            }
        }
        if len2 <= tol::POINT_MERGE * tol::POINT_MERGE {
            continue;
        }
        for &v in outer {
            if point_near_segment(v, a, b, tol::POINT_MERGE) {
                cuts.push(((v - a).dot(d) / len2).clamp(0.0, 1.0));
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() * len2.sqrt() <= tol::POINT_MERGE);
        for w in cuts.windows(2) {
            let m = a + d * (0.5 * (w[0] + w[1]));
            if on_ring(m) {
                // Running along an outer edge.
                if !allow_overlap {
                    return true;
                }
                continue;
            }
            if point_inside_polygon(m, outer, normal) {
                continue;
            }
            // Outside: a graze only if every point of the piece is within
            // `graze` of the ring.
            if !grazing || !within_of_ring(a, d, w[0], w[1], outer, graze) {
                return true;
            }
        }
    }
    false
}

/// Whether every point `a + d·t`, `t` in `[t0, t1]`, lies within `g` of
/// some edge of the closed ring `ring`. Exact: the points within `g` of one
/// edge form a capsule (convex), so the line meets it in one interval of
/// `t`; the union of those intervals must cover `[t0, t1]`, joins allowed a
/// [`tol::POINT_MERGE`] gap along the line.
fn within_of_ring(a: Point3, d: Vec3, t0: f64, t1: f64, ring: &[Point3], g: f64) -> bool {
    let len = d.length();
    if len == 0.0 {
        return true;
    }
    let n = ring.len();
    let mut spans: Vec<(f64, f64)> = (0..n)
        .filter_map(|k| capsule_interval(a, d, ring[k], ring[(k + 1) % n], g))
        .collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let slop = tol::POINT_MERGE / len;
    let mut reach = t0;
    for (lo, hi) in spans {
        if lo > reach + slop {
            break;
        }
        reach = reach.max(hi);
        if reach >= t1 - slop {
            return true;
        }
    }
    reach >= t1 - slop
}

/// The interval of `t` for which `a + d·t` lies within `g` of the segment
/// `c`–`e` (its capsule), or `None`. The capsule is the union of the disks
/// (balls) about the two ends and the slab between them; each meets the
/// line in an interval, and the capsule being convex, their hull is the
/// answer.
///
/// Conditioning: each interval is found about the line's closest approach
/// to a ball centre or to the edge's line, from the closest-approach vector
/// itself rather than a quadratic's discriminant, so its ends are good to
/// rounding in the coordinates (about `f64::EPSILON` times the distances
/// involved), not the `sqrt(f64::EPSILON)` times the line's length a
/// discriminant would cost. Near tangency (the band's half-width under that
/// rounding) the ends are only as good as the rounding allows. The only
/// caller with `g` above `POINT_MERGE` is the import graze (`g` = 1 mm);
/// native containment never reaches this.
fn capsule_interval(a: Point3, d: Vec3, c: Point3, e: Point3, g: f64) -> Option<(f64, f64)> {
    // The `t` for which `w0 + w1·t` has length at most `g`: about the
    // closest approach, half-width sqrt((g² − closest²) / |w1|²). Taken
    // from the closest-approach vector itself (not a quadratic's
    // discriminant), so the ends carry no cancellation.
    let band = |w0: Vec3, w1: Vec3| -> Option<(f64, f64)> {
        let aa = w1.dot(w1);
        if aa == 0.0 {
            return (w0.dot(w0) <= g * g).then_some((f64::NEG_INFINITY, f64::INFINITY));
        }
        let t_star = -w0.dot(w1) / aa;
        let closest = w0 + w1 * t_star;
        let room = g * g - closest.dot(closest);
        if room < 0.0 {
            return None;
        }
        let half = (room / aa).sqrt();
        Some((t_star - half, t_star + half))
    };
    let mut pieces: Vec<(f64, f64)> = Vec::new();
    pieces.extend(band(a - c, d));
    pieces.extend(band(a - e, d));
    let f = e - c;
    let ff = f.dot(f);
    if ff > 0.0 {
        // Projection parameter s(t) = ((a - c) + d t)·f / ff in [0, 1].
        let (s0, s1) = ((a - c).dot(f) / ff, d.dot(f) / ff);
        let range = if s1 == 0.0 {
            (0.0..=1.0)
                .contains(&s0)
                .then_some((f64::NEG_INFINITY, f64::INFINITY))
        } else {
            let (r1, r2) = (-s0 / s1, (1.0 - s0) / s1);
            Some((r1.min(r2), r1.max(r2)))
        };
        // Perpendicular part: (a - c) + d t minus its projection on f.
        let near = band((a - c) - f * s0, d - f * s1);
        if let (Some(r), Some(q)) = (range, near) {
            let (lo, hi) = (r.0.max(q.0), r.1.min(q.1));
            if lo <= hi {
                pieces.push((lo, hi));
            }
        }
    }
    let lo = pieces.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let hi = pieces.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    (lo <= hi).then_some((lo, hi))
}

/// Check that a polygon does not cross or touch itself.  Tests every pair of
/// non-adjacent edges for intersection.
pub(crate) fn polygon_is_simple(pts: &[Point3]) -> bool {
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
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::f64::consts::PI;

    /// A point on an edge or at a corner of either loop is on the region,
    /// whichever edge it is: the inside test alone answers those by edge.
    #[test]
    fn point_on_region_counts_every_loop_edge_and_corner() {
        let outer = xy(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let hole = xy(&[(1.0, 1.0), (1.0, 3.0), (3.0, 3.0), (3.0, 1.0)]);
        let holes = vec![hole];
        let on = |x: f64, y: f64| point_on_region(Point3::new(x, y, 0.0), &outer, &holes, UP);
        for (x, y) in [
            (2.0, 0.0),
            (4.0, 2.0),
            (2.0, 4.0),
            (0.0, 2.0),
            (0.0, 0.0),
            (4.0, 4.0),
            (2.0, 1.0),
            (3.0, 2.0),
            (2.0, 3.0),
            (1.0, 2.0),
            (1.0, 1.0),
            (3.0, 3.0),
        ] {
            assert!(on(x, y), "({x}, {y}) is on a loop");
            assert!(
                point_on_ring(Point3::new(x, y, 0.0), &outer)
                    || point_on_ring(Point3::new(x, y, 0.0), &holes[0])
            );
        }
        assert!(on(0.5, 0.5), "inside the outer, outside the hole");
        assert!(!on(2.0, 2.0), "inside the hole");
        assert!(!on(5.0, 2.0), "outside the outer");
        assert!(!point_on_ring(Point3::new(0.5, 0.5, 0.0), &outer));
    }

    fn xy(pts: &[(f64, f64)]) -> Vec<Point3> {
        pts.iter().map(|&(x, y)| Point3::new(x, y, 0.0)).collect()
    }

    const UP: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    /// A 6 x 6 square with two notches cut down from its top edge to
    /// y = 2, at x in [1, 2] and [4, 5]; each notch side carries an extra
    /// ring vertex at y = 3.
    fn comb() -> Vec<Point3> {
        xy(&[
            (0.0, 0.0),
            (6.0, 0.0),
            (6.0, 6.0),
            (5.0, 6.0),
            (5.0, 3.0),
            (5.0, 2.0),
            (4.0, 2.0),
            (4.0, 3.0),
            (4.0, 6.0),
            (2.0, 6.0),
            (2.0, 3.0),
            (2.0, 2.0),
            (1.0, 2.0),
            (1.0, 3.0),
            (1.0, 6.0),
            (0.0, 6.0),
        ])
    }

    #[test]
    fn a_hole_edge_along_several_collinear_outer_vertices_overlaps_only_where_allowed() {
        // The hole edge y = 2 from x = 0.5 to 5.5 passes through the four
        // notch-bottom corners: inside material, along notch A's bottom,
        // inside, along notch B's bottom, inside. Running along an outer
        // edge leaves only when overlap is not allowed.
        let hole = xy(&[(0.5, 2.0), (3.0, 0.5), (5.5, 2.0)]);
        assert!(!hole_leaves_outer(
            &comb(),
            &hole,
            UP,
            tol::POINT_MERGE,
            true
        ));
        assert!(hole_leaves_outer(
            &comb(),
            &hole,
            UP,
            tol::POINT_MERGE,
            false
        ));
    }

    #[test]
    fn a_hole_edge_through_several_outer_vertices_with_one_stretch_outside_leaves() {
        // The hole edge y = 3 from x = 0.5 to 3 passes through notch A's
        // side vertices (1, 3) and (2, 3); its ends and the stretches beside
        // the notch are in material, the stretch between them is in the
        // notch, outside the face, even with overlap allowed and a graze.
        let hole = xy(&[(0.5, 3.0), (3.0, 3.0), (2.0, 0.5)]);
        assert!(hole_leaves_outer(
            &comb(),
            &hole,
            UP,
            tol::POINT_MERGE,
            true
        ));
        assert!(hole_leaves_outer(&comb(), &hole, UP, 1e-3, true));
    }

    /// The L (0,0) (2,0) (2,1) (1,1) (1,2) (0,2), and a hole whose edge
    /// runs parallel to x + y = 2 at `beyond` past the reflex corner (1, 1)
    /// (positive: clipping the corner, outside the face).
    fn l_and_a_hole_clipping_its_corner(beyond: f64) -> (Vec<Point3>, Vec<Point3>) {
        let l = xy(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ]);
        let k = beyond * std::f64::consts::SQRT_2;
        let hole = xy(&[(1.5 + k, 0.5 + 0.0), (0.4, 0.4), (0.5, 1.5 + k)]);
        (l, hole)
    }

    #[test]
    fn an_outer_vertex_within_the_merge_tolerance_of_a_hole_edge_is_on_it() {
        // Half the merge tolerance past the corner: the corner is ON the
        // edge, a point contact; twice it: a crossing, which a native
        // (exact) ring refuses and an imported one grazes.
        let (l, hole) = l_and_a_hole_clipping_its_corner(0.5 * tol::POINT_MERGE);
        assert!(!hole_leaves_outer(&l, &hole, UP, tol::POINT_MERGE, false));
        let (l, hole) = l_and_a_hole_clipping_its_corner(2.0 * tol::POINT_MERGE);
        assert!(hole_leaves_outer(&l, &hole, UP, tol::POINT_MERGE, false));
        assert!(!hole_leaves_outer(&l, &hole, UP, 1e-3, false));
        // Just inside the corner: clear of it either way.
        let (l, hole) = l_and_a_hole_clipping_its_corner(-2.0 * tol::POINT_MERGE);
        assert!(!hole_leaves_outer(&l, &hole, UP, tol::POINT_MERGE, false));
    }

    #[test]
    fn a_hole_edge_over_a_slot_far_wider_than_the_graze_leaves() {
        // A 10 x 10 square with a slot 0.31 wide and 2 deep cut down from
        // its top edge at x in [5, 5.31]; the hole edge y = 9 from x = 1 to
        // x = 9 runs across the slot, 0.155 from its walls at the middle.
        // Within the import tolerance of a wall near each crossing, not
        // across the slot: it leaves, however the stretch is sampled.
        let outer = xy(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (5.31, 10.0),
            (5.31, 8.0),
            (5.0, 8.0),
            (5.0, 10.0),
            (0.0, 10.0),
        ]);
        let hole = xy(&[(1.0, 9.0), (5.0, 1.0), (9.0, 9.0)]);
        assert!(hole_leaves_outer(&outer, &hole, UP, 1e-3, false));
        // The same edge clear of the slot loads.
        let low = xy(&[(1.0, 7.0), (5.0, 1.0), (9.0, 7.0)]);
        assert!(!hole_leaves_outer(&outer, &low, UP, 1e-3, false));
    }

    #[test]
    fn a_tent_just_past_the_graze_leaves_and_just_inside_it_grazes() {
        // The ring dents in to a tent peak at (4 - peak, 2) between ring
        // vertices (4, 1) and (4, 3); the hole edge from (4, 1) to (4, 3)
        // runs straight past it, `peak` outside the face at its middle (to
        // within the tent's slope).
        let g = 1e-3;
        let ring = |peak: f64| {
            xy(&[
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 1.0),
                (4.0 - peak, 2.0),
                (4.0, 3.0),
                (4.0, 4.0),
                (0.0, 4.0),
            ])
        };
        let hole = xy(&[(4.0, 1.0), (2.0, 2.0), (4.0, 3.0)]);
        assert!(hole_leaves_outer(&ring(1.02 * g), &hole, UP, g, false));
        assert!(!hole_leaves_outer(&ring(0.98 * g), &hole, UP, g, false));
    }

    #[test]
    fn a_degenerate_outer_edge_does_not_disturb_the_on_ring_judgement() {
        // The square's right edge split at (4, 2) and again a third of the
        // merge tolerance above it: an outer edge shorter than the merge
        // tolerance. The hole's edge along x = 4 runs on the ring through it:
        // an overlap, allowed or not as asked, and a hole clear of the ring
        // stays inside.
        //
        // What this does NOT test: a piece whose midpoint is decided by the
        // short edge alone (within POINT_MERGE of it, farther from every
        // other edge). Such a point exists, beside the short edge's middle,
        // but no piece's midpoint can land there: any hole edge passing
        // within POINT_MERGE of that point passes within POINT_MERGE of the
        // short edge's two ends, which become cuts on it (merged into one,
        // being closer than POINT_MERGE), so the piece midpoints fall
        // elsewhere. The degenerate branch of `point_near_segment` (taking
        // such an edge as its first vertex) is therefore never decisive
        // here; the native on-ring test uses it only for parity with the
        // predicate the containment check always used.
        let outer = xy(&[
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 2.0),
            (4.0, 2.0 + tol::POINT_MERGE / 3.0),
            (4.0, 4.0),
            (0.0, 4.0),
        ]);
        let hole = xy(&[(4.0, 1.0), (2.0, 2.0), (4.0, 3.0)]);
        assert!(!hole_leaves_outer(
            &outer,
            &hole,
            UP,
            tol::POINT_MERGE,
            true
        ));
        assert!(hole_leaves_outer(
            &outer,
            &hole,
            UP,
            tol::POINT_MERGE,
            false
        ));
        // A hole clear of the ring is inside, the short edge notwithstanding.
        let inner = xy(&[(3.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
        assert!(!hole_leaves_outer(
            &outer,
            &inner,
            UP,
            tol::POINT_MERGE,
            false
        ));
    }

    /// The sampled graze test this replaced, kept here as a reference: the
    /// piece is graze when 33 evenly spaced points and the feet of the
    /// outer vertices on it are all within `g` of the ring.
    fn sampled_reference(a: Point3, d: Vec3, ring: &[Point3], g: f64) -> bool {
        let n = ring.len();
        let dist = |p: Point3| {
            (0..n)
                .map(|k| {
                    let (c, e) = (ring[k], ring[(k + 1) % n]);
                    let f = e - c;
                    let t = ((p - c).dot(f) / f.dot(f)).clamp(0.0, 1.0);
                    (p - (c + f * t)).length()
                })
                .fold(f64::INFINITY, f64::min)
        };
        let len2 = d.dot(d);
        let mut ts: Vec<f64> = (0..=32).map(|j| f64::from(j) / 32.0).collect();
        for &v in ring {
            let t = (v - a).dot(d) / len2;
            if t > 0.0 && t < 1.0 {
                ts.push(t);
            }
        }
        ts.into_iter().all(|t| dist(a + d * t) <= g)
    }

    /// A ring whose bottom edge y = 0 runs from x = -1 to 11 but for a
    /// feature between `x0` and `x1` (its vertices in `feature`), and a hole
    /// whose bottom edge runs 10 m from (10, -g/4) to (0, -g/4), a quarter
    /// of the graze below the ring: within the graze of it everywhere but
    /// over the feature.
    fn ring_with_a_feature(feature: &[(f64, f64)]) -> (Vec<Point3>, Vec<Point3>) {
        let g = 1e-3;
        let mut pts = vec![(-1.0, 0.0)];
        pts.extend_from_slice(feature);
        pts.extend_from_slice(&[(11.0, 0.0), (11.0, 4.0), (-1.0, 4.0)]);
        let hole = xy(&[(0.0, -0.25 * g), (1.0, 3.0), (10.0, -0.25 * g)]);
        (xy(&pts), hole)
    }

    #[test]
    fn a_slot_narrower_than_the_old_sample_spacing_leaves() {
        // A slot 0.2 m wide and 2 m deep at x in [3.15, 3.35]: off every
        // multiple of the 10 m piece's 1/32 (3.125, 3.4375) and, its walls
        // being within the graze of the piece at their feet, invisible to
        // the old samples. The piece crosses its mouth 0.1 m from its walls:
        // it leaves. The sampled reference would have grazed it.
        let g = 1e-3;
        let (outer, hole) =
            ring_with_a_feature(&[(3.15, 0.0), (3.15, 2.0), (3.35, 2.0), (3.35, 0.0)]);
        assert!(hole_leaves_outer(&outer, &hole, UP, g, false));
        let (a, d) = (hole[2], hole[0] - hole[2]);
        assert!(!within_of_ring(a, d, 0.0, 1.0, &outer, g));
        assert!(
            sampled_reference(a, d, &outer, g),
            "the old sampled test misses this slot"
        );
    }

    #[test]
    fn a_narrow_off_centre_tent_spike_leaves_past_the_graze_and_grazes_inside_it() {
        // A tent spike 0.1 m wide (narrower than the piece's 1/32) at x in
        // [3.2, 3.3], its peak at 3.25, rising `rise` into the face: the
        // piece passes under it `rise + g/4` from the ring. 1.5 x graze
        // leaves; 0.5 x graze grazes. (The old samples happened to see a
        // tent: its peak is a vertex, whose foot they sampled.)
        let g = 1e-3;
        for (excursion, leaves) in [(1.5 * g, true), (0.5 * g, false)] {
            let rise = excursion - 0.25 * g;
            let (outer, hole) = ring_with_a_feature(&[(3.2, 0.0), (3.25, rise), (3.3, 0.0)]);
            assert_eq!(
                hole_leaves_outer(&outer, &hole, UP, g, false),
                leaves,
                "excursion {excursion}"
            );
        }
    }

    // ---- capsule_interval and within_of_ring

    fn p(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    fn v(x: f64, y: f64) -> Vec3 {
        Vec3::new(x, y, 0.0)
    }

    #[test]
    fn a_line_inside_an_edges_band_meets_its_capsule_over_the_edge_and_its_caps() {
        // The edge (0,0)-(10,0), g = 1; the line y = 0.5 along +x from x = -5.
        // Inside the band over the edge, plus the end caps' reach: x from
        // -sqrt(0.75) to 10 + sqrt(0.75), so t (per metre from x = -5) from
        // 5 - 0.866 to 15 + 0.866.
        let (lo, hi) = capsule_interval(p(-5.0, 0.5), v(1.0, 0.0), p(0.0, 0.0), p(10.0, 0.0), 1.0)
            .expect("meets");
        let r = 0.75f64.sqrt();
        assert!((lo - (5.0 - r)).abs() < 1e-12 && (hi - (15.0 + r)).abs() < 1e-12);
    }

    #[test]
    fn a_line_clipping_only_an_end_ball_meets_it_in_a_chord() {
        // The edge (0,0)-(10,0), g = 1; the vertical line x = -0.6 misses the
        // slab but cuts the ball about (0, 0): y in [-0.8, 0.8].
        let (lo, hi) = capsule_interval(p(-0.6, -5.0), v(0.0, 1.0), p(0.0, 0.0), p(10.0, 0.0), 1.0)
            .expect("meets the end ball");
        assert!((lo - 4.2).abs() < 1e-12 && (hi - 5.8).abs() < 1e-12);
        // Past the ball: nothing.
        assert_eq!(
            capsule_interval(p(-1.5, -5.0), v(0.0, 1.0), p(0.0, 0.0), p(10.0, 0.0), 1.0),
            None
        );
    }

    #[test]
    fn a_long_line_and_a_tiny_band_keep_their_ends() {
        // A 100 m line passing half a micron from a ball of radius 1 um:
        // half-width sqrt(1 - 0.25) um, at t = 0.5 +- 0.866e-8. Found from
        // the closest approach, the ends hold to a thousandth of the band.
        let g = 1e-6;
        let (lo, hi) =
            capsule_interval(p(-50.0, 0.5e-6), v(100.0, 0.0), p(0.0, 0.0), p(0.0, 0.0), g)
                .expect("meets");
        let half = 0.75f64.sqrt() * g / 100.0;
        assert!(((hi - lo) / 2.0 - half).abs() < 1e-3 * half, "{lo} {hi}");
        assert!(((lo + hi) / 2.0 - 0.5).abs() < 1e-3 * half);
    }

    #[test]
    fn a_line_tangent_to_an_end_ball_meets_it_in_one_point() {
        // x = -1 touches the ball about (0, 0) at (-1, 0): a zero
        // discriminant, one point.
        let (lo, hi) = capsule_interval(p(-1.0, -5.0), v(0.0, 1.0), p(0.0, 0.0), p(10.0, 0.0), 1.0)
            .expect("tangent");
        assert!((lo - 5.0).abs() < 1e-9 && (hi - 5.0).abs() < 1e-9);
    }

    #[test]
    fn within_of_ring_guards_a_zero_length_direction_and_covers_to_the_ends() {
        let square = xy(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        // A zero-length piece is trivially covered.
        assert!(within_of_ring(
            p(9.0, 9.0),
            v(0.0, 0.0),
            0.0,
            1.0,
            &square,
            1e-3
        ));
        // The piece y = -g along the bottom edge, x from 0 to 4: covered
        // exactly from t = 0 to t = 1 (the bottom edge's band, the corner
        // balls at both ends).
        let g = 1e-3;
        assert!(within_of_ring(
            p(0.0, -g),
            v(4.0, 0.0),
            0.0,
            1.0,
            &square,
            g
        ));
        // Reaching a hair past the corner ball at either end is not.
        assert!(!within_of_ring(
            p(-0.01, -g),
            v(4.02, 0.0),
            0.0,
            1.0,
            &square,
            g
        ));
    }

    #[test]
    fn a_gap_between_bands_is_bridged_only_within_the_merge_slop() {
        // Two collinear outer edges along y = 0 with a gap between x = 1 and
        // x = 1 + gap; the piece y = 0 from x = 0 to 2 (in both bands, but
        // for the gap, where only the end balls of radius g = POINT_MERGE / 4
        // reach). Coverage holds when the uncovered stretch is under the
        // POINT_MERGE join slop and fails above it.
        let g = tol::POINT_MERGE / 4.0;
        let ring = |gap: f64| {
            xy(&[
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, -1.0),
                (1.0 + gap, -1.0),
                (1.0 + gap, 0.0),
                (2.0, 0.0),
                (2.0, 1.0),
                (0.0, 1.0),
            ])
        };
        // Uncovered stretch = gap - 2g.
        let under = 2.0 * g + 0.9 * tol::POINT_MERGE;
        let over = 2.0 * g + 1.1 * tol::POINT_MERGE;
        assert!(within_of_ring(
            p(0.0, 0.0),
            v(2.0, 0.0),
            0.0,
            1.0,
            &ring(under),
            g
        ));
        assert!(!within_of_ring(
            p(0.0, 0.0),
            v(2.0, 0.0),
            0.0,
            1.0,
            &ring(over),
            g
        ));
    }

    #[test]
    fn a_crossing_within_graze_grazes_and_beyond_it_leaves() {
        // The square [0, 4]^2 and a hole whose vertex pokes past its right
        // edge by `out`.
        let square = xy(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let hole = |out: f64| xy(&[(2.0, 1.0), (2.0, 3.0), (4.0 + out, 2.0)]);
        assert!(!hole_leaves_outer(&square, &hole(1e-6), UP, 1e-3, false));
        assert!(hole_leaves_outer(
            &square,
            &hole(1e-6),
            UP,
            tol::POINT_MERGE,
            false
        ));
        assert!(hole_leaves_outer(&square, &hole(1e-2), UP, 1e-3, false));
    }

    /// `a + b` as an exact two-term sum.
    fn two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        let bv = s - a;
        let av = s - bv;
        (s, (a - av) + (b - bv))
    }

    /// `a * b` as an exact two-term product.
    fn two_product(a: f64, b: f64) -> (f64, f64) {
        let p = a * b;
        (p, a.mul_add(b, -p))
    }

    /// Adds `b` to the nonoverlapping expansion `e` exactly (Shewchuk's
    /// grow-expansion).
    fn grow(e: &[f64], b: f64) -> Vec<f64> {
        let mut q = b;
        let mut out = Vec::with_capacity(e.len() + 1);
        for &x in e {
            let (s, err) = two_sum(q, x);
            out.push(err);
            q = s;
        }
        out.push(q);
        out
    }

    /// The exact sign of the 2-D orientation `(b − a) × (c − a)` (z = 0
    /// inputs): every difference and product is kept as an exact expansion.
    fn exact_orient(a: Point3, b: Point3, c: Point3) -> i32 {
        let diff = |x: f64, y: f64| two_sum(x, -y);
        let (abx, abx_e) = diff(b.x, a.x);
        let (aby, aby_e) = diff(b.y, a.y);
        let (acx, acx_e) = diff(c.x, a.x);
        let (acy, acy_e) = diff(c.y, a.y);
        let mut e: Vec<f64> = Vec::new();
        for (u, v, sign) in [
            (abx, acy, 1.0),
            (abx, acy_e, 1.0),
            (abx_e, acy, 1.0),
            (abx_e, acy_e, 1.0),
            (aby, acx, -1.0),
            (aby, acx_e, -1.0),
            (aby_e, acx, -1.0),
            (aby_e, acx_e, -1.0),
        ] {
            let (p, perr) = two_product(u, v);
            e = grow(&e, sign * perr);
            e = grow(&e, sign * p);
        }
        match e.iter().rev().find(|&&x| x != 0.0) {
            Some(&x) if x > 0.0 => 1,
            Some(_) => -1,
            None => 0,
        }
    }

    /// Exact reference: a proper crossing by exact orientation signs, or an
    /// endpoint within `POINT_MERGE` of the other segment.
    fn reference(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
        let proper = exact_orient(r, s, p) * exact_orient(r, s, q) < 0
            && exact_orient(p, q, r) * exact_orient(p, q, s) < 0;
        let m = tol::POINT_MERGE;
        proper
            || point_near_segment(p, r, s, m)
            || point_near_segment(q, r, s, m)
            || point_near_segment(r, p, q, m)
            || point_near_segment(s, p, q, m)
    }

    /// Proper crossings at 2–4 km within nanometres of an end of each
    /// segment, every end more than `POINT_MERGE` from the other segment:
    /// once read as no contact at all (a fuzz of 3 million such pairs).
    #[test]
    fn crossings_near_both_ends_far_from_the_origin_are_contacts() {
        let pt = |x: f64, y: f64| Point3::new(x, y, 0.0);
        let cases = [
            [
                pt(3526.0952849980645, 2468.43609094126),
                pt(3526.1192999042596, 2468.2835099329814),
                pt(3526.0856694564486, 2468.356381010453),
                pt(3526.1192999057585, 2468.28350993212),
            ],
            [
                pt(3705.319407149622, 2492.0640208240848),
                pt(3732.8821521849077, 2613.017506529435),
                pt(3790.8912924388733, 2466.1224924391518),
                pt(3732.88215218381, 2613.0175065291187),
            ],
            [
                pt(3927.5478617999415, 2742.9636127014583),
                pt(3917.233113590398, 2742.063179513278),
                pt(3919.774483388066, 2743.937489396721),
                pt(3917.2331135919044, 2742.063179512406),
            ],
            [
                pt(3358.0863456321754, 2350.518000812669),
                pt(3357.9526526662385, 2350.566856866367),
                pt(3358.0426655272045, 2350.5918940701404),
                pt(3357.9526526649247, 2350.5668568649635),
            ],
        ];
        for [p, q, r, s] in cases {
            assert!(reference(p, q, r, s), "the reference sees the crossing");
            assert!(segments_intersect(p, q, r, s), "{p:?} {q:?} / {r:?} {s:?}");
            assert!(segments_intersect(r, s, p, q), "symmetric");
        }
    }

    fn pt2(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    /// Every argument order and pair swap must answer alike.
    fn proper_all_orders(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
        let v = segments_cross_properly(p, q, r, s);
        assert_eq!(v, segments_cross_properly(q, p, r, s), "reverse first");
        assert_eq!(v, segments_cross_properly(p, q, s, r), "reverse second");
        assert_eq!(v, segments_cross_properly(r, s, p, q), "swap pair");
        v
    }

    /// 1 mm segments crossing with one end 10 nm past the other: a real
    /// crossing, beyond `POINT_MERGE`. (The product-against-length form called it
    /// a touch: its dead band was ~1 um at this scale.)
    #[test]
    fn millimetre_crossing_ten_nanometres_from_an_end_is_proper() {
        let (p, q) = (pt2(0.0, 0.0), pt2(1e-3, 0.0));
        // r sits 10 nm past the first segment, s a millimetre the other way.
        let (r, s) = (pt2(5e-4, -1e-8), pt2(5e-4, 1e-3));
        assert!(proper_all_orders(p, q, r, s));
        assert!(segments_intersect(p, q, r, s));
    }

    /// 100 m segments, an end 0.5 nm from the other segment while the body
    /// crosses: a touch (within `POINT_MERGE`), not a proper crossing.
    #[test]
    fn long_segment_end_half_a_nanometre_away_is_a_touch() {
        let (p, q) = (pt2(0.0, 0.0), pt2(100.0, 0.0));
        let (r, s) = (pt2(50.0, -0.5e-9), pt2(50.0, 50.0));
        assert!(!proper_all_orders(p, q, r, s));
        assert!(segments_intersect(p, q, r, s));
        assert!(segments_intersect(r, s, p, q));
    }

    #[test]
    fn contacts_are_not_proper_crossings() {
        let (p, q) = (pt2(0.0, 0.0), pt2(2.0, 0.0));
        // End on end.
        assert!(!proper_all_orders(p, q, pt2(2.0, 0.0), pt2(3.0, 1.0)));
        // T-touch: an end exactly on the other's body.
        assert!(!proper_all_orders(p, q, pt2(1.0, 0.0), pt2(1.0, 1.0)));
        // Collinear overlap.
        assert!(!proper_all_orders(p, q, pt2(1.0, 0.0), pt2(3.0, 0.0)));
        // Shared endpoint.
        assert!(!proper_all_orders(p, q, pt2(0.0, 0.0), pt2(1.0, 1.0)));
        // A plain X still crosses.
        assert!(proper_all_orders(p, q, pt2(1.0, -1.0), pt2(1.0, 1.0)));
    }

    /// Hand-checked pairs `(p, q, r, s, proper, touch)`, expectations written
    /// as literals so the touch half is not checked only against itself.
    #[test]
    fn hand_checked_contact_table() {
        let cases = [
            // Plain X.
            ([0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0], true, false),
            // T: end on the other's body.
            ([0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [1.0, 1.0], false, true),
            // Shared endpoint.
            ([0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], false, true),
            // Collinear overlap.
            ([0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [3.0, 0.0], false, true),
            // Disjoint.
            ([0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], false, false),
            // End 1 um short of the other's line: neither crossing nor touch.
            (
                [0.0, 0.0],
                [2.0, 0.0],
                [1.0, 1e-6],
                [1.0, 1.0],
                false,
                false,
            ),
            // End past the other's far end, on its line: the nearest point
            // of a segment is its end, not the foot on its line.
            ([0.0, 0.0], [1.0, 0.0], [1.5, 0.0], [1.5, 1.0], false, false),
            // End half a nanometre off the other's body: a touch.
            (
                [0.0, 0.0],
                [2.0, 0.0],
                [1.0, 0.5e-9],
                [1.0, 1.0],
                false,
                true,
            ),
            // End three nanometres off: not a touch.
            (
                [0.0, 0.0],
                [2.0, 0.0],
                [1.0, 3e-9],
                [1.0, 1.0],
                false,
                false,
            ),
        ];
        for (p, q, r, s, proper, touch) in cases {
            let [p, q, r, s] = [p, q, r, s].map(|c| pt2(c[0], c[1]));
            assert_eq!(
                segments_cross_properly(p, q, r, s),
                proper,
                "{p:?} {q:?} {r:?} {s:?}"
            );
            assert_eq!(segments_touch(p, q, r, s), touch, "{p:?} {q:?} {r:?} {s:?}");
            assert_eq!(segments_intersect(p, q, r, s), proper || touch);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20000))]

        /// `segments_cross_properly` is "exact proper crossing and no touch"
        /// and `segments_intersect` is "proper crossing or touch", at every
        /// scale from a micron to ten kilometres.
        #[test]
        fn proper_crossing_and_contact_decompose_exactly(
            exp in -6i32..5,
            far in 0.0f64..4000.0,
            ox in -1.0f64..1.0,
            oy in -1.0f64..1.0,
            ang in 0.0f64..PI,
            tilt in -1e-3f64..1e-3,
            t in -0.2f64..1.2,
            nudge in -3.0f64..3.0,
            shape in 0u8..3,
            turn in 0.01f64..3.1,
        ) {
            let scale = 10f64.powi(exp);
            let base = Point3::new(far + ox * scale, far * 0.5 + oy * scale, 0.0);
            let dir = Vec3::new(ang.cos(), ang.sin(), 0.0);
            let nrm = Vec3::new(-dir.y, dir.x, 0.0);
            let p = base;
            let q = base + dir * scale;
            let (r, s) = match shape {
                0 => {
                    let d2 = (dir + nrm * tilt).normalized().expect("direction");
                    let r = p + nrm * (nudge * 1e-9) + dir * (t * scale);
                    (r, r + d2 * scale)
                }
                1 => {
                    let foot = p + dir * (t * scale) + nrm * (nudge * 1e-9);
                    (foot, foot + nrm * scale)
                }
                _ => {
                    let d2 = Vec3::new((ang + turn).cos(), (ang + turn).sin(), 0.0);
                    let x = p + dir * (t.clamp(0.05, 0.95) * scale);
                    (x - d2 * (scale * 0.5), x + d2 * (scale * 0.5 + nudge * 1e-9))
                }
            };
            let proper_exact = exact_orient(r, s, p) * exact_orient(r, s, q) < 0
                && exact_orient(p, q, r) * exact_orient(p, q, s) < 0;
            let m = tol::POINT_MERGE;
            let touch = point_near_segment(p, r, s, m)
                || point_near_segment(q, r, s, m)
                || point_near_segment(r, p, q, m)
                || point_near_segment(s, p, q, m);
            let proper = segments_cross_properly(p, q, r, s);
            prop_assert_eq!(proper, proper_exact && !touch, "{:?} {:?} / {:?} {:?}", p, q, r, s);
            prop_assert_eq!(
                segments_intersect(p, q, r, s),
                proper || touch,
                "{:?} {:?} / {:?} {:?}", p, q, r, s
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20000))]

        /// `segments_intersect` agrees with the exact reference on
        /// near-parallel, T, and acute near-endpoint pairs from a micron to
        /// ten kilometres, offset by about a nanometre — no proper crossing
        /// missed, no contact invented.
        #[test]
        fn segment_contact_matches_an_exact_reference(
            exp in -6i32..5,
            far in 0.0f64..4000.0,
            ox in -1.0f64..1.0,
            oy in -1.0f64..1.0,
            ang in 0.0f64..PI,
            tilt in -1e-3f64..1e-3,
            t in -0.2f64..1.2,
            nudge in -3.0f64..3.0,
            shape in 0u8..4,
            turn in 0.01f64..3.1,
            a in 0.0f64..5e-9,
            b in 0.0f64..5e-9,
        ) {
            let scale = 10f64.powi(exp);
            let base = Point3::new(far + ox * scale, far * 0.5 + oy * scale, 0.0);
            let dir = Vec3::new(ang.cos(), ang.sin(), 0.0);
            let nrm = Vec3::new(-dir.y, dir.x, 0.0);
            let p = base;
            let q = base + dir * scale;
            let (r, s) = match shape {
                // Near-parallel, ~nudge nm off.
                0 => {
                    let d2 = (dir + nrm * tilt).normalized().expect("direction");
                    let r = p + nrm * (nudge * 1e-9) + dir * (t * scale);
                    (r, r + d2 * scale)
                }
                // T: an end ~nudge nm off the first segment's body.
                1 => {
                    let foot = p + dir * (t * scale) + nrm * (nudge * 1e-9);
                    (foot, foot + nrm * scale)
                }
                // Crossing within nanometres of the ENDS q and s, at any
                // angle, the segments micrometres to kilometres long.
                3 => {
                    let d2 = Vec3::new((ang + turn).cos(), (ang + turn).sin(), 0.0);
                    let x = q - dir * a;
                    (x - d2 * (scale * (0.3 + t.abs())), x + d2 * b)
                }
                // Acute, crossing within nanometres of an end of each.
                _ => {
                    let d2 = (dir + nrm * (tilt * 50.0 + 1e-4))
                        .normalized()
                        .expect("direction");
                    let x = q - dir * (nudge.abs() * 1e-9);
                    (x - d2 * (scale * 0.5), x + d2 * ((nudge + 3.0) * 1e-9))
                }
            };
            prop_assert_eq!(
                segments_intersect(p, q, r, s),
                reference(p, q, r, s),
                "{:?} {:?} / {:?} {:?}", p, q, r, s
            );
        }
    }
}
