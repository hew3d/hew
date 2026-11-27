//! Loft (true-curves.md §5.4): a solid skinned through closed profiles in
//! order, optionally ending in an apex and guided by up to two rails.
//! Every profile keeps every one of its own vertices (decision I-23).
//! Profiles with the same number of corners correspond corner to corner,
//! by arc length within each side; otherwise by arc length round the whole
//! perimeter (D-126). The skin builder (WP13) assembles it.

use crate::math::{Plane, Point3, Vec3};
use crate::shape_error::ShapeError;
use crate::sketch::CurveGeom;
use crate::station::{ColumnFade, Fade, Side, SkinSpec, area_centroid, build_skin, rotate_about};
use crate::tol;
use crate::topo::{Object, SurfaceRef};
use crate::trig;

/// Two perimeter parameters closer than this are the same sample. A
/// parameter is a dimensionless fraction of a perimeter, so this is not a
/// length tolerance: it identifies the same fraction reached in two
/// profiles.
pub const LOFT_PARAM_MERGE: f64 = 1e-12;

/// Most samples a loft ring may carry.
const LOFT_MAX_SAMPLES: usize = 4096;

/// One loft profile: a closed planar loop without holes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoftProfile {
    /// The loop, in order.
    pub points: Vec<Point3>,
    /// Per-edge circle attribution (edge `i` = `points[i] → points[i+1]`);
    /// empty means none. A lone arc edge (its neighbours on no other edge
    /// of its circle) turns the short way unless a neighbour tangent to it
    /// decides: split an arc over a half turn into two or more edges.
    #[serde(default)]
    pub curves: Vec<Option<CurveGeom>>,
    /// Per-edge smooth-chain key (WP03 semantics); empty means none.
    #[serde(default)]
    pub smooth: Vec<Option<u32>>,
    /// Per-edge point on an arc edge strictly between `points[i]` and
    /// `points[i+1]`, where the caller knows which way the arc runs: it
    /// decides the edge's direction, and the run holding it (D-130). Empty
    /// or `None` means inferred (D-129 B: a run of two or more edges turns
    /// as its angles say, a lone edge as a tangent neighbor continues it,
    /// else the short way). The point must lie on the edge's circle. Sketch
    /// and face profiles need none: their arcs are chords of at most 15° or
    /// corner rounds tangent to their sides, which the inference reads
    /// exactly.
    #[serde(default)]
    pub through: Vec<Option<Point3>>,
    /// Moves the correspondence start by this many start candidates from
    /// the automatic start: corners where every profile has the same
    /// number of corners, else original vertices (0 = automatic; D-126).
    #[serde(default)]
    pub start_offset: i32,
}

/// A Loft.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoftSpec {
    /// The profiles, in order.
    pub profiles: Vec<LoftProfile>,
    /// The point the loft ends in after the last profile.
    #[serde(default)]
    pub apex: Option<Point3>,
    /// Up to two rails: polylines touching every profile (and the apex).
    #[serde(default)]
    pub rails: Vec<Vec<Point3>>,
    /// Stations between consecutive profiles (≥ 1; 1 = ruled). `None`: 1
    /// for two rows (two profiles, or one and an apex) without rails, else 8.
    #[serde(default)]
    pub stations: Option<u32>,
}

/// One oriented, start-aligned profile with its perimeter parameters.
struct Prepared {
    points: Vec<Point3>,
    curves: Vec<Option<CurveGeom>>,
    smooth: Vec<Option<u32>>,
    params: Vec<f64>,
    normal: Vec3,
    /// The automatic start (before `start_offset`).
    auto_point: Point3,
    /// The start candidates in loft order from the automatic start (D-126
    /// R2): where the tool draws its start markers.
    candidates: Vec<Point3>,
    /// Per edge: the signed angle an attributed arc edge turns through
    /// about its circle's center (about `normal`, counterclockwise
    /// positive); `None` for a straight edge.
    sweeps: Vec<Option<f64>>,
}

fn same_geom(a: Option<CurveGeom>, b: Option<CurveGeom>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            x.center.approx_eq(y.center, tol::POINT_MERGE)
                && (x.radius - y.radius).abs() <= tol::POINT_MERGE
        }
        _ => false,
    }
}

impl Prepared {
    fn n(&self) -> usize {
        self.points.len()
    }

    /// Original vertex `v` is a corner: its two edges share no curve or
    /// smooth key and the boundary turns there.
    fn is_corner(&self, v: usize) -> bool {
        corner_at(
            &self.points,
            &self.curves,
            &self.smooth,
            &self.sweeps,
            self.normal,
            v,
        )
    }

    /// The sample at perimeter parameter `u`: the vertex itself, a point on
    /// the edge's arc (turning the way the arc runs), or a point along the
    /// chord.
    fn sample(&self, u: f64) -> Point3 {
        let n = self.n();
        for v in 0..n {
            if (self.params[v] - u).abs() <= LOFT_PARAM_MERGE {
                return self.points[v];
            }
        }
        // The edge holding `u`; before the first vertex's parameter (a
        // start between vertices, see `align_coaxial`), the closing edge.
        let (i, start, end) = match (0..n).rev().find(|&v| self.params[v] < u) {
            Some(i) if i + 1 < n => (i, self.params[i], self.params[i + 1]),
            Some(i) => (i, self.params[i], 1.0 + self.params[0]),
            None => (n - 1, self.params[n - 1] - 1.0, self.params[0]),
        };
        let t = (u - start) / (end - start);
        let (a, b) = (self.points[i], self.points[(i + 1) % n]);
        if let (Some(g), Some(sweep)) = (self.curves[i], self.sweeps[i])
            && let Ok(e1) = (a - g.center).normalized()
        {
            let e2 = self.normal.cross(e1);
            let th = sweep * t;
            return g.center + (e1 * trig::cos(th) + e2 * trig::sin(th)) * g.radius;
        }
        a + (b - a) * t
    }
}

/// Vertex `v` of a loop is a corner: its two edges share no curve or smooth
/// key and the boundary turns there, read from the edges' tangents at `v`
/// (an arc's from its circle and signed sweep, `arc_sweeps`), so a line
/// meeting an arc tangentially is no corner.
fn corner_at(
    points: &[Point3],
    curves: &[Option<CurveGeom>],
    smooth: &[Option<u32>],
    sweeps: &[Option<f64>],
    normal: Vec3,
    v: usize,
) -> bool {
    let n = points.len();
    let prev = (v + n - 1) % n;
    let joined = same_geom(curves[prev], curves[v])
        || matches!((smooth[prev], smooth[v]), (Some(a), Some(b)) if a == b);
    if joined {
        return false;
    }
    // Edge `e`'s unit tangent at its point `at` (its start or its end).
    let tangent = |e: usize, at: Point3| -> Option<Vec3> {
        match (curves[e], sweeps.get(e).copied().flatten()) {
            (Some(g), Some(sw)) => (normal.cross(at - g.center) * sw.signum())
                .normalized()
                .ok(),
            _ => (points[(e + 1) % n] - points[e]).normalized().ok(),
        }
    };
    let (Some(a), Some(b)) = (tangent(prev, points[v]), tangent(v, points[v])) else {
        return true;
    };
    // A smooth-keyed run with no analytic curve (a spline or a Bézier) has
    // only its chords: where it meets another edge, the junction is a
    // corner only where it turns more than the run's own end facet does
    // there (the angle between its last two chords, or its first two; a
    // one-edge run turns 0), so a line meeting it tangentially is no corner
    // (D-129 C).
    let keyed = |e: usize| smooth[e].is_some() && curves[e].is_none();
    if keyed(prev) || keyed(v) {
        let chord = |e: usize| (points[(e + 1) % n] - points[e]).normalized().ok();
        let turn = |x: Option<Vec3>, y: Option<Vec3>| match (x, y) {
            (Some(x), Some(y)) => trig::acos(x.dot(y).clamp(-1.0, 1.0)),
            _ => 0.0,
        };
        let mut allowance = 0.0_f64;
        // The run ending at v: its last two chords.
        let before = (prev + n - 1) % n;
        if keyed(prev) && smooth[before] == smooth[prev] && before != v {
            allowance = allowance.max(turn(chord(before), chord(prev)));
        }
        // The run starting at v: its first two chords.
        let after = (v + 1) % n;
        if keyed(v) && smooth[after] == smooth[v] && after != prev {
            allowance = allowance.max(turn(chord(v), chord(after)));
        }
        // In the dot space the plain test uses (so collinear chords in
        // any direction read alike), the allowance as an angle.
        return a.dot(b) < trig::cos(allowance) - tol::NORMAL_DIRECTION;
    }
    a.dot(b) < 1.0 - tol::NORMAL_DIRECTION
}

/// The angle the boundary turns through at vertex `v` as the facets have it
/// (D-128): between two edges of one curve, the angle between their chords
/// (a 48-gon circle turns 7.5° at each vertex); elsewhere between the
/// edges' tangents, an arc's from its circle and sweep and any other
/// edge's (straight or smooth-keyed) its chord (a line meeting an arc
/// tangentially turns 0). The raw turn: [`corner_at`]'s allowance for a
/// smooth-keyed run's end facet does not apply.
fn facet_turn(
    points: &[Point3],
    curves: &[Option<CurveGeom>],
    sweeps: &[Option<f64>],
    normal: Vec3,
    v: usize,
) -> f64 {
    let n = points.len();
    let prev = (v + n - 1) % n;
    let chord = |e: usize| (points[(e + 1) % n] - points[e]).normalized().ok();
    let tangent = |e: usize, at: Point3| -> Option<Vec3> {
        match (curves[e], sweeps.get(e).copied().flatten()) {
            (Some(g), Some(sw)) => (normal.cross(at - g.center) * sw.signum())
                .normalized()
                .ok(),
            _ => chord(e),
        }
    };
    let (a, b) = if same_geom(curves[prev], curves[v]) {
        (chord(prev), chord(v))
    } else {
        (tangent(prev, points[v]), tangent(v, points[v]))
    };
    match (a, b) {
        (Some(a), Some(b)) => trig::acos(a.dot(b).clamp(-1.0, 1.0)),
        _ => 0.0,
    }
}

/// `v` turned by the least rotation that takes unit `from` onto unit `to`.
fn carry(v: Vec3, from: Vec3, to: Vec3) -> Vec3 {
    let axis = from.cross(to);
    let (sin, cos) = (axis.length(), from.dot(to));
    match axis.normalized() {
        Ok(k) if sin > tol::NORMALIZE_MIN_LENGTH => {
            // Rodrigues.
            v * cos + k.cross(v) * sin + k * (k.dot(v) * (1.0 - cos))
        }
        // Parallel normals carry nothing; opposite ones do not occur
        // (every profile faces the loft direction).
        _ => v,
    }
}

/// Newell plane, planarity, and a 2D simple-polygon test.
fn check_profile(pts: &[Point3]) -> Result<Plane, ShapeError> {
    let bad = ShapeError::ShapeDegenerate;
    if pts.len() < 3 {
        return Err(bad);
    }
    let plane = Plane::from_polygon(pts).map_err(|_| bad)?;
    if pts
        .iter()
        .any(|&p| plane.signed_distance(p).abs() > tol::PLANE_DIST)
    {
        return Err(bad);
    }
    let (u, v) = crate::geom2d::plane_axes(plane.normal());
    let q: Vec<(f64, f64)> = pts
        .iter()
        .map(|p| (p.to_vec().dot(u), p.to_vec().dot(v)))
        .collect();
    let n = q.len();
    for i in 0..n {
        if (pts[(i + 1) % n] - pts[i]).length() <= tol::POINT_MERGE {
            return Err(bad);
        }
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let (a, b, c, d) = (q[i], q[(i + 1) % n], q[j], q[(j + 1) % n]);
            let (d1, d2) = (cross(a, b, c), cross(a, b, d));
            let (d3, d4) = (cross(c, d, a), cross(c, d, b));
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                return Err(bad);
            }
        }
    }
    Ok(plane)
}

/// Distance from `p` to the polyline, and the arc length of the nearest
/// point along it.
fn nearest_on_polyline(line: &[Point3], p: Point3) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0);
    let mut acc = 0.0;
    for w in line.windows(2) {
        let ab = w[1] - w[0];
        let len = ab.length();
        let t = if len > 0.0 {
            ((p - w[0]).dot(ab) / (len * len)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = (p - (w[0] + ab * t)).length();
        if d < best.0 {
            best = (d, acc + t * len);
        }
        acc += len;
    }
    best
}

fn point_at(line: &[Point3], s: f64) -> Point3 {
    let mut acc = 0.0;
    for w in line.windows(2) {
        let len = (w[1] - w[0]).length();
        if acc + len >= s && len > 0.0 {
            return w[0] + (w[1] - w[0]) * ((s - acc) / len).clamp(0.0, 1.0);
        }
        acc += len;
    }
    *line.last().expect("a rail has points")
}

/// The frame a station path's tangents are limited in (see
/// [`limited_tangents`]).
#[derive(Debug, Clone, Copy)]
enum StationFrame {
    /// The world axes: each coordinate stays between its rows', so a
    /// station stays in the box of the two rows either side.
    World,
    /// About the coaxial circles' axis (a point on it, its direction): along
    /// it, away from it and around it, so a station's distance from the
    /// axis stays between its rows'.
    Meridian(Point3, Vec3),
    /// Profiles on parallel planes (their common normal): along the normal
    /// and two directions in the planes (the world's x and y where the
    /// planes are level), so a station stays in the box of the two rows
    /// either side in that frame and, with knots shared by every column, is
    /// a plane slice.
    Plane(Vec3),
}

/// The centripetal knots of the path through `rows` (square root of chord
/// length, floored so coincident rows do not repeat a knot).
fn cr_knots(rows: &[Point3]) -> Vec<f64> {
    let mut knots = vec![0.0_f64; rows.len()];
    for i in 1..rows.len() {
        knots[i] = knots[i - 1]
            + (rows[i] - rows[i - 1])
                .length()
                .sqrt()
                .max(tol::LOFT_KNOT_MIN);
    }
    knots
}

/// Hermite tangents (per unit knot) of the non-uniform Catmull–Rom; the
/// duplicated end control gives the chord slope at each end.
fn cr_tangents(rows: &[Point3], knots: &[f64]) -> Vec<Vec3> {
    let k = rows.len();
    let slope = |i: usize, j: usize| (rows[j] - rows[i]) * (1.0 / (knots[j] - knots[i]));
    (0..k)
        .map(|i| {
            if i == 0 {
                slope(0, 1)
            } else if i == k - 1 {
                slope(k - 2, k - 1)
            } else {
                slope(i - 1, i) - slope(i - 1, i + 1) + slope(i, i + 1)
            }
        })
        .collect()
}

/// Centripetal Catmull–Rom through `rows` (duplicated end controls), with
/// `per` stations per span; includes every row. The tangents are limited
/// (see [`limited_tangents`]) so that between two rows each component of
/// the path in `frame` stays between its two values: no station bulges
/// past the rows either side of it, and a path whose spans all stay
/// between their rows as they are keeps its Catmull–Rom tangents exactly.
fn catmull_rom(
    rows: &[Point3],
    per: usize,
    frame: StationFrame,
    shared_knots: Option<&[f64]>,
) -> Vec<Point3> {
    let k = rows.len();
    let knots = shared_knots.map_or_else(|| cr_knots(rows), <[f64]>::to_vec);
    let tangents = limited_tangents(rows, &knots, cr_tangents(rows, &knots), frame);
    let mut out = Vec::with_capacity((k - 1) * per + 1);
    for i in 0..k - 1 {
        let dt = knots[i + 1] - knots[i];
        let (m0, m1) = (tangents[i] * dt, tangents[i + 1] * dt);
        for s in 0..per {
            let t = s as f64 / per as f64;
            let (t2, t3) = (t * t, t * t * t);
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h10 = t3 - 2.0 * t2 + t;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let h11 = t3 - t2;
            out.push(if s == 0 {
                rows[i]
            } else {
                Point3::ORIGIN
                    + rows[i].to_vec() * h00
                    + m0 * h10
                    + rows[i + 1].to_vec() * h01
                    + m1 * h11
            });
        }
    }
    out.push(rows[k - 1]);
    out
}

/// A column's frame about the axis `(origin, a)`: `a`, the direction away
/// from the axis toward the column, and the direction around it. A column
/// on the axis takes any direction square to it.
fn column_frame(rows: &[Point3], (origin, a): (Point3, Vec3)) -> [Vec3; 3] {
    let away = rows
        .iter()
        .find_map(|&p| {
            let d = p - origin;
            (d - a * d.dot(a)).normalized().ok()
        })
        .or_else(|| {
            [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)]
                .into_iter()
                .find_map(|x| a.cross(x).normalized().ok())
        })
        .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
    [a, away, a.cross(away)]
}

/// Whether the cubic Hermite span with end slopes `al` and `be` (as
/// multiples of the span's own slope, both ≥ 0) is monotone: Fritsch and
/// Carlson's exact region.
fn monotone_span(al: f64, be: f64) -> bool {
    let s = al + be - 2.0;
    let (p, q) = (2.0 * al + be - 3.0, al + 2.0 * be - 3.0);
    s <= 0.0 || p <= 0.0 || q <= 0.0 || al - p * p / (3.0 * s) >= 0.0
}

/// Fritsch–Carlson limiting of the Hermite `tangents` (per unit knot) of
/// the path through `rows`, component by component in `frame`. A component
/// changes only where a span through its row would leave its two rows'
/// values: its tangent is zero at a row where it turns back (or holds), is
/// zero where it runs against its span, and where a span's cubic is not
/// monotone (Fritsch and Carlson's exact test) both end tangents scale into
/// the disc of radius 3, which lies inside the monotone region, sweeping
/// until no span is left outside (shrinking a shared tangent can push the
/// span on its other side out). A span that stays between its rows keeps
/// its tangents bit for bit unless a neighbor shrinks a shared one.
fn limited_tangents(
    rows: &[Point3],
    knots: &[f64],
    tangents: Vec<Vec3>,
    frame: StationFrame,
) -> Vec<Vec3> {
    let k = rows.len();
    let frame = match frame {
        StationFrame::World => [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
        StationFrame::Meridian(origin, a) => column_frame(rows, (origin, a)),
        StationFrame::Plane(n) => {
            let x = Vec3::new(1.0, 0.0, 0.0);
            let e1 = (x - n * x.dot(n))
                .normalized()
                .unwrap_or(Vec3::new(0.0, 1.0, 0.0));
            [e1, n.cross(e1), n]
        }
    };
    let mut parts: Vec<[f64; 3]> = tangents
        .iter()
        .map(|t| [t.dot(frame[0]), t.dot(frame[1]), t.dot(frame[2])])
        .collect();
    let mut changed = false;
    for (c, dir) in frame.iter().enumerate() {
        let delta: Vec<f64> = (0..k - 1)
            .map(|i| (rows[i + 1] - rows[i]).dot(*dir) / (knots[i + 1] - knots[i]))
            .collect();
        let mut set = |part: &mut f64, v: f64| {
            if *part != v {
                *part = v;
                changed = true;
            }
        };
        for i in 1..k - 1 {
            if delta[i - 1] * delta[i] <= 0.0 {
                set(&mut parts[i][c], 0.0);
            }
        }
        for (i, &d) in delta.iter().enumerate() {
            for j in [i, i + 1] {
                if parts[j][c] * d < 0.0 || d == 0.0 {
                    set(&mut parts[j][c], 0.0);
                }
            }
        }
        // A row's tangent is shared by the spans either side, and the
        // monotone region is not closed under shrinking one end (a span at
        // (4, 1) is on its edge, at (4, 0.5) outside): sweep until every
        // span is inside. A span scaled into the disc of radius 3 stays
        // inside whatever shrinks its ends later, so each sweep settles at
        // least one more span and `k` sweeps suffice.
        for _ in 0..k {
            let mut settled = true;
            for (i, &d) in delta.iter().enumerate() {
                if d == 0.0 {
                    continue;
                }
                let (al, be) = (parts[i][c] / d, parts[i + 1][c] / d);
                if !monotone_span(al, be) {
                    let tau = 3.0 / trig::hypot(al, be);
                    let (p0, p1) = (parts[i][c] * tau, parts[i + 1][c] * tau);
                    set(&mut parts[i][c], p0);
                    set(&mut parts[i + 1][c], p1);
                    settled = false;
                }
            }
            if settled {
                break;
            }
        }
    }
    if !changed {
        return tangents;
    }
    parts
        .iter()
        .map(|p| frame[0] * p[0] + frame[1] * p[1] + frame[2] * p[2])
        .collect()
}

/// The signed sweep of each attributed arc edge of an oriented loop
/// (`normal` its counterclockwise side): the turn from the edge's start to
/// its end about the circle's center.
///
/// The rule: consecutive edges on one circle form a run, traversed one way
/// about the center. Each edge's counterclockwise angle `θ` (in (0, 2π))
/// is taken; a run of two or more edges turns counterclockwise, with the
/// loop, by those angles when they add to less than a full turn, and
/// clockwise (each edge by `θ − 2π`) otherwise, which a run of `n` edges
/// can only do when they add to more than `n − 1` turns: arcs of 250° then
/// 50° turn 300° together. A whole circle runs counterclockwise with the
/// loop. A lone arc edge (D-129 B) turns the way a neighbour tangent to
/// its circle continues it (a straight edge, or a run already decided),
/// else the short way (a lone 250° edge turns 110° the other way), and an
/// exact half turn with no such neighbour counterclockwise with the loop.
/// A lone arc over a half turn should be drawn as two or more edges.
///
/// # Errors
/// `ShapeDegenerate` where the profile does not decide a direction: a run
/// whose angles fit neither way, or a lone arc whose two tangent
/// neighbours ask opposite ways.
fn arc_sweeps(
    points: &[Point3],
    curves: &[Option<CurveGeom>],
    through: &[Option<Point3>],
    normal: Vec3,
) -> Result<Vec<Option<f64>>, ShapeError> {
    use std::f64::consts::{PI, TAU};
    let bad = ShapeError::ShapeDegenerate;
    let n = points.len();
    // Each attributed edge's counterclockwise angle about its center.
    let mut ccw: Vec<Option<f64>> = Vec::with_capacity(n);
    for e in 0..n {
        let Some(g) = curves[e] else {
            ccw.push(None);
            continue;
        };
        let (ra, rb) = (points[e] - g.center, points[(e + 1) % n] - g.center);
        let e1 = ra.normalized().map_err(|_| bad)?;
        let e2 = normal.cross(e1);
        ccw.push(Some(trig::atan2(rb.dot(e2), rb.dot(e1)).rem_euclid(TAU)));
    }
    let same = |a: usize, b: usize| same_geom(curves[a], curves[b]) && curves[a].is_some();
    // Points the caller gave on arcs (D-130): each on its edge's circle
    // and off its endpoints; the arc runs counterclockwise exactly where
    // the point comes before the end, counterclockwise from the start.
    let mut given: Vec<Option<bool>> = vec![None; n];
    for e in 0..n {
        let (Some(t), Some(g), Some(a)) = (through.get(e).copied().flatten(), curves[e], ccw[e])
        else {
            continue;
        };
        if ((t - g.center).length() - g.radius).abs() > tol::PLANE_DIST {
            return Err(bad);
        }
        let e1 = (points[e] - g.center).normalized().map_err(|_| bad)?;
        let e2 = normal.cross(e1);
        let rt = t - g.center;
        let at = trig::atan2(rt.dot(e2), rt.dot(e1)).rem_euclid(TAU);
        // Within tolerance of either endpoint, on either side of it (a
        // point just short of the start reads as nearly a full turn).
        let near = |x: f64| x.min(TAU - x) <= tol::NORMAL_DIRECTION;
        if near(at) || near((at - a).abs()) {
            return Err(bad);
        }
        given[e] = Some(at < a);
    }
    // The direction a set of edges' given points ask (`None`: none given),
    // refusing two that disagree.
    let asked = |edges: &[usize]| -> Result<Option<bool>, ShapeError> {
        let mut dir = None;
        for &e in edges {
            if let Some(d) = given[e] {
                if dir.is_some_and(|x| x != d) {
                    return Err(bad);
                }
                dir = Some(d);
            }
        }
        Ok(dir)
    };
    let turned = |a: f64, counterclockwise: bool| if counterclockwise { a } else { a - TAU };
    if (0..n).all(|e| same(e, 0)) {
        let all: Vec<usize> = (0..n).collect();
        return Ok(match asked(&all)? {
            Some(dir) => ccw.iter().map(|a| a.map(|a| turned(a, dir))).collect(),
            None => ccw,
        });
    }
    let mut out: Vec<Option<f64>> = vec![None; n];
    // Runs start at an attributed edge whose predecessor is on another
    // circle (or straight); the loop has at least one such start here.
    let mut lone: Vec<usize> = Vec::new();
    for start in (0..n).filter(|&e| curves[e].is_some() && !same((e + n - 1) % n, e)) {
        let run: Vec<usize> = (0..n)
            .map(|k| (start + k) % n)
            .take_while(|&e| same(e, start))
            .collect();
        if let Some(dir) = asked(&run)? {
            // A run that way must turn less than a full turn (a run of two
            // or more edges fits one way only: a point says which, or
            // contradicts it and refuses).
            let total: f64 = run
                .iter()
                .map(|&e| turned(ccw[e].expect("attributed"), dir))
                .sum();
            if total.abs() >= TAU {
                return Err(bad);
            }
            for &e in &run {
                out[e] = Some(turned(ccw[e].expect("attributed"), dir));
            }
            continue;
        }
        if run.len() == 1 {
            lone.push(start);
            continue;
        }
        let angles: Vec<f64> = run.iter().map(|&e| ccw[e].expect("attributed")).collect();
        let total: f64 = angles.iter().sum();
        let edges = run.len() as f64;
        let counterclockwise = if total < TAU {
            true
        } else if total > TAU * (edges - 1.0) {
            false
        } else {
            return Err(bad);
        };
        for (&e, &a) in run.iter().zip(&angles) {
            out[e] = Some(if counterclockwise { a } else { a - TAU });
        }
    }
    // A lone arc edge (D-129 B) turns the way a neighbour tangent to its
    // circle at their shared vertex continues it: a straight edge, or an
    // arc already decided (an undecided lone arc does not vote); two
    // disagreeing votes refuse. With no vote it turns the short way, and
    // an exact half turn counterclockwise with the loop (convex).
    // The direction of travel along edge `e` at its end `at` (its start or
    // end point), or `None` for an undecided arc.
    let travel = |out: &[Option<f64>], e: usize, at: Point3| -> Option<Vec3> {
        match (curves[e], out[e]) {
            (None, _) => (points[(e + 1) % n] - points[e]).normalized().ok(),
            (Some(g), Some(sw)) => (normal.cross(at - g.center) * sw.signum())
                .normalized()
                .ok(),
            (Some(_), None) => None,
        }
    };
    let vote = |out: &[Option<f64>], e: usize| -> Result<Option<bool>, ShapeError> {
        let g = curves[e].expect("attributed");
        let (a, b) = (points[e], points[(e + 1) % n]);
        let tangent_at = |p: Point3| normal.cross(p - g.center).normalized().ok();
        let mut votes: Vec<bool> = Vec::new();
        // Arriving at the arc's start, leaving from its end.
        for (nb, at) in [((e + n - 1) % n, a), ((e + 1) % n, b)] {
            if let (Some(t), Some(tau)) = (travel(out, nb, at), tangent_at(at)) {
                let d = t.dot(tau);
                if d.abs() >= 1.0 - tol::NORMAL_DIRECTION {
                    votes.push(d > 0.0);
                }
            }
        }
        match votes.as_slice() {
            [] => Ok(None),
            [v] => Ok(Some(*v)),
            [x, y] if x == y => Ok(Some(*x)),
            _ => Err(bad),
        }
    };
    let mut undecided = lone;
    loop {
        let mut progress = false;
        let mut rest = Vec::new();
        for e in undecided {
            match vote(&out, e)? {
                Some(counterclockwise) => {
                    let a = ccw[e].expect("attributed");
                    out[e] = Some(if counterclockwise { a } else { a - TAU });
                    progress = true;
                }
                None => rest.push(e),
            }
        }
        undecided = rest;
        if !progress || undecided.is_empty() {
            break;
        }
    }
    for e in undecided {
        let a = ccw[e].expect("attributed");
        out[e] = Some(if a <= PI + tol::NORMAL_DIRECTION {
            a
        } else {
            a - TAU
        });
    }
    Ok(out)
}

/// Validates and orients the profiles, and aligns their starts
/// (§1.1, §1.2, §1.4).
/// `n0` (a loop's chord-polygon normal), or its opposite where the loop's
/// arcs, run as they are drawn, wind it the other way: each arc edge's
/// middle (by its decided sweep about `n0`) joins the polygon.
fn arc_aware_normal(p: &LoftProfile, n0: Vec3) -> Result<Vec3, ShapeError> {
    let n = p.points.len();
    let curves = if p.curves.is_empty() {
        vec![None; n]
    } else {
        p.curves.clone()
    };
    if curves.iter().all(Option::is_none) {
        return Ok(n0);
    }
    let through = if p.through.is_empty() {
        vec![None; n]
    } else {
        p.through.clone()
    };
    let sweeps = arc_sweeps(&p.points, &curves, &through, n0)?;
    let mut ring: Vec<Point3> = Vec::with_capacity(2 * n);
    for e in 0..n {
        ring.push(p.points[e]);
        if let (Some(g), Some(sw)) = (curves[e], sweeps[e]) {
            ring.push(rotate_about(p.points[e], g.center, n0, sw / 2.0));
        }
    }
    let area = Plane::from_polygon(&ring).map_err(|_| ShapeError::ShapeDegenerate)?;
    Ok(if area.normal().dot(n0) < 0.0 { -n0 } else { n0 })
}

/// One profile oriented about the loft direction, before its start.
struct Oriented {
    /// Points the caller gave on arcs, per edge (D-130).
    given: Vec<Option<Point3>>,
    points: Vec<Point3>,
    curves: Vec<Option<CurveGeom>>,
    smooth: Vec<Option<u32>>,
    normal: Vec3,
    centroid: Point3,
    /// Where the input's vertex 0 sits in the oriented loop.
    zero: usize,
    /// The corners, ascending.
    corners: Vec<usize>,
    /// The circle the whole loop is, if it is one.
    circle: Option<CurveGeom>,
}

/// A profile's automatic start: its candidate, the start direction (unit,
/// in its plane, from its area centroid) its neighbors align to, and for a
/// whole circle the turn (in turns) by which its parameters move so the
/// start falls on that direction between vertices.
#[derive(Clone, Copy)]
struct Start {
    candidate: usize,
    dir: Vec3,
    ahead: f64,
}

fn prepare(spec: &LoftSpec) -> Result<Vec<Prepared>, ShapeError> {
    let bad = ShapeError::ShapeDegenerate;
    // Planes and centroids.
    let mut planes = Vec::with_capacity(spec.profiles.len());
    for p in &spec.profiles {
        let n = p.points.len();
        if (!p.curves.is_empty() && p.curves.len() != n)
            || (!p.smooth.is_empty() && p.smooth.len() != n)
            || (!p.through.is_empty() && p.through.len() != n)
        {
            return Err(bad);
        }
        planes.push(check_profile(&p.points)?);
    }
    if let Some(apex) = spec.apex {
        let last = planes.last().expect("one profile at least");
        if last.signed_distance(apex).abs() <= tol::PLANE_DIST {
            return Err(bad);
        }
    }
    // Each loop's winding with its arcs as they run (a chord's middle
    // replaced by the arc's): a lone arc over a half turn reverses the
    // chords' winding, and the loop's normal follows the arcs (D-130).
    let normals: Vec<Vec3> = spec
        .profiles
        .iter()
        .zip(&planes)
        .map(|(p, pl)| arc_aware_normal(p, pl.normal()))
        .collect::<Result<_, _>>()?;
    let centroids: Vec<Point3> = spec
        .profiles
        .iter()
        .zip(&normals)
        .map(|(p, &n)| area_centroid(&p.points, n).unwrap_or(p.points[0]))
        .collect();

    // Orientation: every profile CCW about the loft direction.
    let k = spec.profiles.len();
    let dir = |i: usize| -> Vec3 {
        let next = if i + 1 < k {
            Some(centroids[i + 1])
        } else {
            spec.apex
        };
        match next {
            Some(q) => q - centroids[i],
            None => centroids[i] - centroids[i - 1],
        }
    };
    let mut oriented: Vec<Oriented> = Vec::with_capacity(k);
    for (i, p) in spec.profiles.iter().enumerate() {
        let n = p.points.len();
        let mut points = p.points.clone();
        let mut curves = if p.curves.is_empty() {
            vec![None; n]
        } else {
            p.curves.clone()
        };
        let mut smooth = if p.smooth.is_empty() {
            vec![None; n]
        } else {
            p.smooth.clone()
        };
        let mut given = if p.through.is_empty() {
            vec![None; n]
        } else {
            p.through.clone()
        };
        let mut normal = normals[i];
        let mut zero = 0;
        if normal.dot(dir(i)) < 0.0 {
            points.reverse();
            // Edge i of the reversed loop is old edge n−2−i, reversed.
            curves = (0..n).map(|e| curves[(2 * n - 2 - e) % n]).collect();
            smooth = (0..n).map(|e| smooth[(2 * n - 2 - e) % n]).collect();
            given = (0..n).map(|e| given[(2 * n - 2 - e) % n]).collect();
            normal = -normal;
            zero = n - 1;
        }
        let sweeps = arc_sweeps(&points, &curves, &given, normal)?;
        let corners = (0..n)
            .filter(|&v| corner_at(&points, &curves, &smooth, &sweeps, normal, v))
            .collect();
        let circle = curves[0].filter(|&g| curves.iter().all(|&c| same_geom(c, Some(g))));
        oriented.push(Oriented {
            given,
            points,
            curves,
            smooth,
            normal,
            centroid: centroids[i],
            zero,
            corners,
            circle,
        });
    }

    // Corner mode where every profile has the same number of corners
    // (R1); the start candidates are then the corners, else every vertex
    // (R2).
    let c = oriented[0].corners.len();
    let corner_mode = c >= 1 && oriented.iter().all(|o| o.corners.len() == c);
    let cands: Vec<Vec<usize>> = oriented
        .iter()
        .map(|o| {
            if corner_mode {
                o.corners.clone()
            } else {
                (0..o.points.len()).collect()
            }
        })
        .collect();

    // Automatic starts by least twist (R3): from the first profile with a
    // corner (profile 0 where none has one), whose start is its first
    // candidate at or after the input's vertex 0, out to either end, each
    // aligned to its placed neighbor. Twist is read from vectors off each
    // profile's area centroid, the neighbor's carried onto this profile's
    // plane by the least rotation between their normals.
    let unit = |v: Vec3| v.normalized().unwrap_or(v);
    // A start direction off profile `o`'s area centroid toward its point
    // `p`; where `p` sits on the centroid (a concave outline can put a
    // vertex there), off the mean of its points, else along its next edge:
    // never a zero vector, which would align by index alone.
    let start_dir = |o: &Oriented, v: usize| -> Vec3 {
        let p = o.points[v];
        let n = o.points.len();
        let mean = o
            .points
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |a, q| a + q.to_vec())
            / n as f64;
        let mean = Point3::new(mean.x, mean.y, mean.z);
        (p - o.centroid)
            .normalized()
            .or_else(|_| (p - mean).normalized())
            .or_else(|_| (o.points[(v + 1) % n] - p).normalized())
            .unwrap_or(o.normal)
    };
    let anchor = (0..k)
        .find(|&r| !oriented[r].corners.is_empty())
        .unwrap_or(0);
    let mut starts: Vec<Option<Start>> = vec![None; k];
    {
        let o = &oriented[anchor];
        let n = o.points.len();
        let cand = &cands[anchor];
        let i0 = (0..cand.len())
            .min_by_key(|&i| (cand[i] + n - o.zero) % n)
            .expect("three points at least");
        starts[anchor] = Some(Start {
            candidate: i0,
            dir: start_dir(o, cand[i0]),
            ahead: 0.0,
        });
    }
    let align = |r: usize, nb: usize, placed: &Start| -> Start {
        let (o, onb) = (&oriented[r], &oriented[nb]);
        let moved = |v: Vec3| carry(v, onb.normal, o.normal);
        let cand = &cands[r];
        if corner_mode {
            // The corner pairing with the least sum of squared differences
            // (ties: the lowest candidate).
            let from: Vec<Vec3> = (0..c)
                .map(|q| moved(onb.points[cands[nb][(placed.candidate + q) % c]] - onb.centroid))
                .collect();
            let cost = |i: usize| -> f64 {
                (0..c)
                    .map(|q| {
                        let d = (o.points[cand[(i + q) % c]] - o.centroid) - from[q];
                        d.dot(d)
                    })
                    .sum()
            };
            let mut best = 0;
            for i in 1..c {
                if cost(i) < cost(best) {
                    best = i;
                }
            }
            return Start {
                candidate: best,
                dir: start_dir(o, cand[best]),
                ahead: 0.0,
            };
        }
        // Perimeter mode: the neighbor's start direction.
        let d = unit(moved(placed.dir));
        let d = unit(d - o.normal * d.dot(o.normal));
        let center = o.circle.map_or(o.centroid, |g| g.center);
        let mut best = 0;
        let mut best_cos = f64::NEG_INFINITY;
        for (i, &v) in cand.iter().enumerate() {
            let cos = unit(o.points[v] - center).dot(d);
            if cos > best_cos {
                best_cos = cos;
                best = i;
            }
        }
        match o.circle {
            // A whole circle starts on that direction, between vertices if
            // need be (as coaxial circles do): its parameters move on by
            // how far its nearest vertex sits past it.
            Some(_) => {
                let e1 = d;
                let e2 = o.normal.cross(e1);
                let w = o.points[cand[best]] - center;
                let ahead =
                    (trig::atan2(w.dot(e2), w.dot(e1)) / std::f64::consts::TAU).rem_euclid(1.0);
                Start {
                    candidate: best,
                    dir: d,
                    ahead,
                }
            }
            None => Start {
                candidate: best,
                dir: start_dir(o, cand[best]),
                ahead: 0.0,
            },
        }
    };
    for r in anchor + 1..k {
        let placed = starts[r - 1].expect("placed");
        starts[r] = Some(align(r, r - 1, &placed));
    }
    for r in (0..anchor).rev() {
        let placed = starts[r + 1].expect("placed");
        starts[r] = Some(align(r, r + 1, &placed));
    }

    let mut prepared: Vec<Prepared> = Vec::with_capacity(k);
    for (r, (o, p)) in oriented.into_iter().zip(&spec.profiles).enumerate() {
        let st = starts[r].expect("every profile placed");
        let cand = &cands[r];
        let len_c = cand.len();
        let n = o.points.len();
        let auto = cand[st.candidate];
        // A whole circle started between vertices turns its candidates by
        // the same amount, so each marker sits where its offset puts column
        // 0 (the start direction turned by that many vertices).
        let turned = |p: Point3| match o.circle {
            Some(g) if st.ahead != 0.0 => {
                rotate_about(p, g.center, o.normal, -st.ahead * std::f64::consts::TAU)
            }
            _ => p,
        };
        let candidates: Vec<Point3> = (0..len_c)
            .map(|q| turned(o.points[cand[(st.candidate + q) % len_c]]))
            .collect();
        let auto_point = match o.circle {
            Some(g) if st.ahead != 0.0 => g.center + st.dir * g.radius,
            _ => o.points[auto],
        };
        // The offset counts candidates from the automatic start (R2).
        let start = cand
            [(st.candidate as i64 + i64::from(p.start_offset)).rem_euclid(len_c as i64) as usize];
        let (mut points, mut curves, mut smooth, mut given) =
            (o.points, o.curves, o.smooth, o.given);
        points.rotate_left(start);
        curves.rotate_left(start);
        smooth.rotate_left(start);
        given.rotate_left(start);
        let normal = o.normal;
        let sweeps = arc_sweeps(&points, &curves, &given, normal)?;
        // Lengths along the boundary: an arc edge by its arc (so a circle
        // is parameterized by angle, as it is sampled), a straight edge by
        // its chord.
        let len = |e: usize| match (curves[e], sweeps[e]) {
            (Some(g), Some(sw)) => g.radius * sw.abs(),
            _ => (points[(e + 1) % n] - points[e]).length(),
        };
        let mut params = vec![0.0; n];
        if corner_mode {
            // Corner q at exactly q / C in every profile, a vertex inside
            // side q at (q + l / L_q) / C by arc length along that side.
            let mut at: Vec<usize> = o.corners.iter().map(|&v| (v + n - start) % n).collect();
            at.sort_unstable();
            for q in 0..c {
                let (a, b) = (at[q], if q + 1 < c { at[q + 1] } else { n });
                let side: f64 = (a..b).map(len).sum();
                let mut l = 0.0;
                for (v, x) in params.iter_mut().enumerate().take(b).skip(a) {
                    *x = (q as f64 + l / side) / c as f64;
                    l += len(v);
                }
            }
        } else {
            for v in 1..n {
                params[v] = params[v - 1] + len(v - 1);
            }
            let perimeter = params[n - 1] + len(n - 1);
            for x in &mut params {
                *x /= perimeter;
            }
        }
        let mut prep = Prepared {
            points,
            curves,
            smooth,
            params,
            normal,
            auto_point,
            candidates,
            sweeps,
        };
        if st.ahead > LOFT_PARAM_MERGE && 1.0 - st.ahead > LOFT_PARAM_MERGE {
            turn_params(&mut prep, st.ahead);
        }
        prepared.push(prep);
    }
    Ok(prepared)
}

/// Moves every parameter of `p` on by `ahead` turns (wrapping), the loop
/// rotated so they ascend from the smallest: a start between vertices.
fn turn_params(p: &mut Prepared, ahead: f64) {
    let n = p.points.len();
    let shifted: Vec<f64> = p
        .params
        .iter()
        .map(|&x| (x + ahead).rem_euclid(1.0))
        .collect();
    let first = (0..n)
        .min_by(|&a, &b| shifted[a].total_cmp(&shifted[b]))
        .expect("three points at least");
    // Ascending from the smallest: a rotation of the loop.
    let order: Vec<usize> = (0..n).map(|v| (first + v) % n).collect();
    p.points = order.iter().map(|&v| p.points[v]).collect();
    p.curves = order.iter().map(|&v| p.curves[v]).collect();
    p.smooth = order.iter().map(|&v| p.smooth[v]).collect();
    p.params = order.iter().map(|&v| shifted[v]).collect();
    p.sweeps = order.iter().map(|&v| p.sweeps[v]).collect();
}

/// The profiles' circles when every profile is entirely one circle chain,
/// the planes are parallel and the centers lie on one normal line.
fn coaxial_circles(prepared: &[Prepared]) -> Option<Vec<CurveGeom>> {
    let circle = |p: &Prepared| -> Option<CurveGeom> {
        let g = p.curves[0]?;
        p.curves.iter().all(|&c| same_geom(c, Some(g))).then_some(g)
    };
    let gs: Vec<CurveGeom> = prepared.iter().map(circle).collect::<Option<_>>()?;
    let n0 = prepared[0].normal;
    prepared
        .iter()
        .zip(&gs)
        .all(|(p, g)| {
            n0.cross(p.normal).length() <= tol::NORMAL_DIRECTION
                && (g.center - gs[0].center).cross(n0).length() <= tol::POINT_MERGE
        })
        .then_some(gs)
}

/// Coaxial circles correspond by angle about their common axis rather than
/// from each profile's own start vertex: each profile's parameters turn so
/// its automatic start sits at the first profile's automatic start angle
/// (a start offset still turns its own profile by its vertices). The
/// columns then run on meridians, every station is a circle and every band
/// a planar strip of a cone, whatever the side counts and the angles the
/// circles were drawn from. A start may then fall between two vertices;
/// every vertex keeps its own parameter (I-23). Returns the axis (a point
/// on it and its direction) for coaxial circles.
fn align_coaxial(prepared: &mut [Prepared]) -> Option<(Point3, Vec3)> {
    let gs = coaxial_circles(prepared)?;
    let n0 = prepared[0].normal;
    let e1 = (prepared[0].auto_point - gs[0].center).normalized().ok()?;
    let e2 = n0.cross(e1);
    let turns = |p: Point3, c: Point3| {
        let d = p - c;
        trig::atan2(d.dot(e2), d.dot(e1)) / std::f64::consts::TAU
    };
    for (p, g) in prepared.iter_mut().zip(&gs) {
        // How far this profile's automatic start sits past the first
        // profile's, in turns: every parameter moves on by that much (a
        // start offset still turns the profile on top of it).
        let ahead = turns(p.auto_point, g.center).rem_euclid(1.0);
        if ahead <= LOFT_PARAM_MERGE || 1.0 - ahead <= LOFT_PARAM_MERGE {
            continue;
        }
        turn_params(p, ahead);
    }
    Some((gs[0].center, n0))
}

/// The profiles prepared and, for coaxial circles, turned onto one another
/// (`align_coaxial`), with the axis.
type PreparedLoft = (Vec<Prepared>, Option<(Point3, Vec3)>);

fn prepare_aligned(spec: &LoftSpec) -> Result<PreparedLoft, ShapeError> {
    let mut prepared = prepare(spec)?;
    let coaxial = align_coaxial(&mut prepared);
    Ok((prepared, coaxial))
}

/// The perimeter parameter where `rail` touches `p`: its nearest vertex, or
/// its start (a whole circle started between vertices), within
/// `tol::LOFT_RAIL_TOUCH`; `None` where it touches neither.
fn rail_touch(p: &Prepared, rail: &[Point3]) -> Option<f64> {
    let start = (p.sample(0.0), 0.0);
    let mut best: Option<(f64, f64)> = None;
    for (q, u) in p
        .points
        .iter()
        .copied()
        .zip(p.params.iter().copied())
        .chain(std::iter::once(start))
    {
        let (d, _) = nearest_on_polyline(rail, q);
        if d <= tol::LOFT_RAIL_TOUCH && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, u));
        }
    }
    best.map(|(_, u)| u)
}

/// Two perimeter parameters name one column (they wrap at 1).
fn same_param(a: f64, b: f64) -> bool {
    let d = (a - b).rem_euclid(1.0);
    d <= LOFT_PARAM_MERGE || 1.0 - d <= LOFT_PARAM_MERGE
}

/// The profiles prepared with the first rail deciding the correspondence
/// (D-232): a rail joins the points that must pair, so where it touches
/// the profiles at different columns under the starts asked for, each
/// profile's start moves round its candidates until the rail touches it at
/// the first profile's column; failing that (shapes matched by perimeter,
/// whose columns only meet at the start), every profile starts where the
/// rail touches it. A whole circle no start offset brings there (coaxial
/// circles pair by angle, so one drawn from another phase has no vertex on
/// the other's columns) turns until the rail touches it on the column,
/// its start between vertices (D-251). A rail that misses a profile, or
/// that no choice of starts brings onto one column, is left to
/// `from_loft`'s column check, which refuses `RailMissesProfile`. Without rails, or where the rail
/// already runs on one column, the starts are those asked for.
fn prepare_with_rails(spec: &LoftSpec) -> Result<PreparedLoft, ShapeError> {
    let first = prepare_aligned(spec)?;
    let Some(rail) = spec.rails.first().filter(|r| r.len() >= 2) else {
        return Ok(first);
    };
    let Some(touch) = first
        .0
        .iter()
        .map(|p| rail_touch(p, rail))
        .collect::<Option<Vec<f64>>>()
    else {
        return Ok(first);
    };
    if touch.iter().all(|&t| same_param(t, touch[0])) {
        return Ok(first);
    }
    // One profile's start moved until the rail touches it at `target`:
    // the other profiles' starts do not depend on it (each aligns to its
    // neighbour's automatic start, and coaxial circles to the first's).
    // Only the start offsets that can bring it there are tried (review
    // F1: trying every one prepared the whole loft once per vertex, 3.8 s
    // for two 2000-gons). Away from a whole circle a profile is
    // parameterized by arc length from its start, so a start at candidate
    // `c` moves every parameter back by `c`'s: the rail's vertex lands on
    // `target` only from a candidate at its parameter less `target`. A
    // whole circle the prediction misses is turned by `settle`.
    let place = |s: &mut LoftSpec, r: usize, target: f64| -> Result<(), ShapeError> {
        let p = &first.0[r];
        let mut at: Vec<([u64; 3], f64)> = p
            .points
            .iter()
            .zip(&p.params)
            .map(|(q, &u)| ([q.x.to_bits(), q.y.to_bits(), q.z.to_bits()], u))
            .collect();
        at.sort_by_key(|e| e.0);
        let param_of = |q: Point3| {
            let key = [q.x.to_bits(), q.y.to_bits(), q.z.to_bits()];
            at.binary_search_by(|e| e.0.cmp(&key)).ok().map(|i| at[i].1)
        };
        for (c, &q) in p.candidates.iter().enumerate() {
            if !param_of(q).is_some_and(|u| same_param(touch[r] - u, target)) {
                continue;
            }
            let Ok(offset) = i32::try_from(c) else {
                continue;
            };
            s.profiles[r].start_offset = offset;
            let (prepared, _) = prepare_aligned(s)?;
            if rail_touch(&prepared[r], rail).is_some_and(|t| same_param(t, target)) {
                return Ok(());
            }
        }
        s.profiles[r].start_offset = spec.profiles[r].start_offset;
        Ok(())
    };
    // The profiles prepared under `s`, each whole circle the rail still
    // touches off its target turned until it touches there (its start
    // then falls between vertices, as `align_coaxial` starts one): coaxial
    // circles pair by angle, so one drawn from another phase has no
    // vertex on the other's columns and no start offset brings the rail
    // onto one (review F4). `None` if a profile other than a whole circle
    // stays off its target.
    let settle = |s: &LoftSpec, target: &dyn Fn(usize) -> Option<f64>| {
        let (mut prepared, coaxial) = prepare_aligned(s)?;
        for (r, p) in prepared.iter_mut().enumerate() {
            let Some(target) = target(r) else { continue };
            let Some(t) = rail_touch(p, rail) else {
                return Ok(None);
            };
            if same_param(t, target) {
                continue;
            }
            if whole_circle(p).is_none() {
                return Ok(None);
            }
            turn_params(p, (target - t).rem_euclid(1.0));
        }
        Ok::<_, ShapeError>(Some((prepared, coaxial)))
    };
    // The first profile keeps its start; the others come round to it.
    let mut s = spec.clone();
    for r in 1..s.profiles.len() {
        place(&mut s, r, touch[0])?;
    }
    if let Some(done) = settle(&s, &|r| (r > 0).then_some(touch[0]))? {
        return Ok(done);
    }
    // Every profile starts where the rail touches it.
    let mut s = spec.clone();
    for r in 0..s.profiles.len() {
        place(&mut s, r, 0.0)?;
    }
    Ok(settle(&s, &|_| Some(0.0))?.unwrap_or(first))
}

/// The circle `p` is entirely an arc chain of, if it is one.
fn whole_circle(p: &Prepared) -> Option<CurveGeom> {
    let g = p.curves[0]?;
    p.curves.iter().all(|&c| same_geom(c, Some(g))).then_some(g)
}

/// Each profile's start candidates in loft order from its automatic start
/// (D-126 R2: its corners where every profile has the same number of
/// corners, else its original vertices): where the tool draws its start
/// markers; `start_offset` counts along this list.
///
/// # Errors
/// [`ShapeError::LoftNeedsTwoProfiles`] for fewer than two rows (one
/// profile without an apex, or none), and the profile refusals of
/// [`Object::from_loft`].
pub fn start_points(spec: &LoftSpec) -> Result<Vec<Vec<Point3>>, ShapeError> {
    if spec.profiles.len() + usize::from(spec.apex.is_some()) < 2 {
        return Err(ShapeError::LoftNeedsTwoProfiles);
    }
    Ok(prepare(spec)?.into_iter().map(|p| p.candidates).collect())
}

impl Object {
    /// Loft (true-curves.md §5.4), pure: a new solid skinned through the
    /// profiles in order (and the apex), guided by the rails.
    ///
    /// # Errors
    /// [`ShapeError::LoftNeedsTwoProfiles`], [`ShapeError::LoftTooManyRails`],
    /// [`ShapeError::RailMissesProfile`] (a rail not within
    /// `tol::LOFT_RAIL_TOUCH` of a column point of every profile, a vertex
    /// or a start marker, or touching them at different columns), [`ShapeError::AmountOutOfRange`] (zero
    /// stations, more than 4096 samples or 4096 stations), [`ShapeError::ShapeDegenerate`]
    /// (a profile under three points, non-planar, self-crossing, or an apex
    /// on the last profile's plane), [`ShapeError::SelfIntersecting`].
    pub fn from_loft(spec: &LoftSpec) -> Result<Object, ShapeError> {
        let bad = ShapeError::ShapeDegenerate;
        let rows = spec.profiles.len() + usize::from(spec.apex.is_some());
        if spec.profiles.is_empty() || rows < 2 {
            return Err(ShapeError::LoftNeedsTwoProfiles);
        }
        if spec.rails.len() > 2 {
            return Err(ShapeError::LoftTooManyRails);
        }
        if spec.stations == Some(0)
            || spec.stations.is_some_and(|n| {
                (n as usize).saturating_mul(rows - 1) > crate::deform::MAX_STATIONS
            })
        {
            return Err(ShapeError::AmountOutOfRange);
        }

        let (prepared, coaxial) = prepare_with_rails(spec)?;
        let k = prepared.len();

        // The common sampling.
        let mut all: Vec<f64> = prepared.iter().flat_map(|p| p.params.clone()).collect();
        all.sort_by(f64::total_cmp);
        let mut u: Vec<f64> = Vec::with_capacity(all.len());
        for x in all {
            if u.last().is_none_or(|&l| x - l > LOFT_PARAM_MERGE) {
                u.push(x);
            }
        }
        if u.len() > LOFT_MAX_SAMPLES {
            return Err(ShapeError::AmountOutOfRange);
        }
        let m = u.len();
        let rings: Vec<Vec<Point3>> = prepared
            .iter()
            .map(|p| u.iter().map(|&x| p.sample(x)).collect())
            .collect();
        // `corner[r][j]`: column `j` is a corner of profile `r`; a column
        // is hard if it is a corner of any profile (I-23), and fades
        // between them where it no longer folds (D-S23).
        let corner: Vec<Vec<bool>> = prepared
            .iter()
            .map(|p| {
                u.iter()
                    .map(|&x| {
                        (0..p.n())
                            .any(|v| (p.params[v] - x).abs() <= LOFT_PARAM_MERGE && p.is_corner(v))
                    })
                    .collect()
            })
            .collect();
        let hard: Vec<bool> = (0..m).map(|j| corner.iter().any(|c| c[j])).collect();
        // Each profile's facet floor (D-128): the largest turn at a vertex
        // that is no corner (0 where every vertex is one), capped at the
        // coarsest circle's facet (TAU / MIN_CIRCLE_SEGMENTS, 15°; D-129 A).
        let cap = std::f64::consts::TAU / crate::sketch::MIN_CIRCLE_SEGMENTS as f64;
        let floor_of: Vec<f64> = prepared
            .iter()
            .map(|p| {
                (0..p.n())
                    .filter(|&v| !p.is_corner(v))
                    .map(|v| facet_turn(&p.points, &p.curves, &p.sweeps, p.normal, v))
                    .fold(0.0_f64, f64::max)
                    .min(cap)
            })
            .collect();
        // Each column's turn at a profile it is a corner of (D-129 A): the
        // corner's own turn there, 0 where it is no corner.
        let corner_turn: Vec<Vec<f64>> = prepared
            .iter()
            .map(|p| {
                u.iter()
                    .map(|&x| {
                        (0..p.n())
                            .find(|&v| {
                                (p.params[v] - x).abs() <= LOFT_PARAM_MERGE && p.is_corner(v)
                            })
                            .map_or(0.0, |v| {
                                facet_turn(&p.points, &p.curves, &p.sweeps, p.normal, v)
                            })
                    })
                    .collect()
            })
            .collect();

        // Rails: the column each touches and its arc length at each row.
        let mut rail_cols: Vec<(usize, Vec<f64>)> = Vec::new();
        for rail in &spec.rails {
            if rail.len() < 2 {
                return Err(ShapeError::RailMissesProfile);
            }
            let mut col = None;
            let mut at = Vec::with_capacity(rows);
            for ring in &rings {
                // The column whose point on this profile the rail passes
                // nearest: a vertex, or a start a whole circle took between
                // vertices (its start markers).
                let hit = (0..m)
                    .map(|j| (j, nearest_on_polyline(rail, ring[j])))
                    .filter(|(_, (d, _))| *d <= tol::LOFT_RAIL_TOUCH)
                    .min_by(|a, b| a.1.0.total_cmp(&b.1.0));
                let Some((c, (_, s))) = hit else {
                    return Err(ShapeError::RailMissesProfile);
                };
                if col.is_some_and(|cc| cc != c) {
                    return Err(ShapeError::RailMissesProfile);
                }
                col = Some(c);
                at.push(s);
            }
            if let Some(apex) = spec.apex {
                let (d, s) = nearest_on_polyline(rail, apex);
                if d > tol::LOFT_RAIL_TOUCH {
                    return Err(ShapeError::RailMissesProfile);
                }
                at.push(s);
            }
            rail_cols.push((col.expect("a profile at least"), at));
        }

        // Stations.
        let per = spec
            .stations
            .unwrap_or(if rows == 2 && spec.rails.is_empty() {
                1
            } else {
                8
            }) as usize;
        if m.saturating_mul((rows - 1).saturating_mul(per)) > crate::deform::MAX_SKIN_QUADS {
            return Err(ShapeError::AmountOutOfRange);
        }
        let row_point = |r: usize, j: usize| -> Point3 {
            if r < k {
                rings[r][j]
            } else {
                spec.apex.expect("the apex row")
            }
        };
        // The station path's frame: about the axis of coaxial circles, else
        // the planes' frame where the profiles are parallel, else the
        // world's axes. Between parallel planes every column takes the
        // knots of the centroids' path, so each station is a plane slice
        // and a band whose sides are parallel stays one planar quad.
        let n0 = prepared[0].normal;
        let parallel = prepared
            .iter()
            .all(|p| n0.cross(p.normal).length() <= tol::NORMAL_DIRECTION);
        let frame = match coaxial {
            Some((origin, axis)) => StationFrame::Meridian(origin, axis),
            None if parallel => StationFrame::Plane(n0),
            None => StationFrame::World,
        };
        let shared_knots = parallel.then(|| {
            let centroid = |p: &Prepared| area_centroid(&p.points, p.normal).unwrap_or(p.points[0]);
            let mut path: Vec<Point3> = prepared.iter().map(centroid).collect();
            path.extend(spec.apex);
            cr_knots(&path)
        });
        let mut columns: Vec<Vec<Point3>> = Vec::with_capacity(m);
        for j in 0..m {
            let ctrl: Vec<Point3> = (0..rows).map(|r| row_point(r, j)).collect();
            columns.push(if rows == 2 {
                (0..=per)
                    .map(|s| ctrl[0] + (ctrl[1] - ctrl[0]) * (s as f64 / per as f64))
                    .collect()
            } else {
                catmull_rom(&ctrl, per, frame, shared_knots.as_deref())
            });
        }
        let n_st = (rows - 1) * per + 1;
        let mut station_rings: Vec<Vec<Point3>> = (0..n_st)
            .map(|s| (0..m).map(|j| columns[j][s]).collect())
            .collect();
        // The profiles stay exactly as given.
        for (r, ring) in rings.iter().enumerate() {
            station_rings[r * per] = ring.clone();
        }
        if !rail_cols.is_empty() {
            for (s, ring) in station_rings.iter_mut().enumerate() {
                if s % per == 0 || (spec.apex.is_some() && s == n_st - 1) {
                    continue;
                }
                let (span, f) = (s / per, (s % per) as f64 / per as f64);
                let target = |(_, at): &(usize, Vec<f64>), rail: &[Point3]| {
                    point_at(rail, at[span] + (at[span + 1] - at[span]) * f)
                };
                match rail_cols.len() {
                    1 => {
                        let x = target(&rail_cols[0], &spec.rails[0]);
                        let shift = x - ring[rail_cols[0].0];
                        for p in ring.iter_mut() {
                            *p = *p + shift;
                        }
                    }
                    _ => {
                        let (c1, c2) = (rail_cols[0].0, rail_cols[1].0);
                        let (x1, x2) = (
                            target(&rail_cols[0], &spec.rails[0]),
                            target(&rail_cols[1], &spec.rails[1]),
                        );
                        let normal = Plane::from_polygon(ring).map_err(|_| bad)?.normal();
                        let flat = |v: Vec3| v - normal * v.dot(normal);
                        let (a, b) = (flat(ring[c2] - ring[c1]), flat(x2 - x1));
                        let (la, lb) = (a.length(), b.length());
                        if la <= tol::POINT_MERGE || lb <= tol::POINT_MERGE {
                            return Err(bad);
                        }
                        let scale = lb / la;
                        let angle = trig::atan2(a.cross(b).dot(normal), a.dot(b));
                        let pivot = ring[c1];
                        for p in ring.iter_mut() {
                            let r = rotate_about(*p, pivot, normal, angle);
                            *p = x1 + (r - pivot) * scale;
                        }
                    }
                }
            }
        }

        // Claims for ruled spans between coaxial circles.
        let ruled = per == 1 || rows == 2;
        let mut claims: Vec<Vec<Vec<Option<SurfaceRef>>>> =
            vec![vec![vec![None; m]]; n_st.saturating_sub(1)];
        if ruled {
            let circle = |p: &Prepared| -> Option<CurveGeom> {
                let g = p.curves[0]?;
                p.curves.iter().all(|&c| same_geom(c, Some(g))).then_some(g)
            };
            for r in 0..k.saturating_sub(1) {
                let (Some(g0), Some(g1)) = (circle(&prepared[r]), circle(&prepared[r + 1])) else {
                    continue;
                };
                let n0 = prepared[r].normal;
                if n0.cross(prepared[r + 1].normal).length() > tol::NORMAL_DIRECTION {
                    continue;
                }
                let d = g1.center - g0.center;
                if d.cross(n0).length() > tol::POINT_MERGE {
                    continue;
                }
                let h = d.dot(n0);
                if h.abs() <= tol::POINT_MERGE {
                    continue;
                }
                let claim = if (g0.radius - g1.radius).abs() <= tol::POINT_MERGE {
                    SurfaceRef::Cylinder {
                        axis_point: g0.center,
                        axis: n0,
                        radius: g0.radius,
                    }
                } else {
                    let apex = g0.center + d * (g0.radius / (g0.radius - g1.radius));
                    let toward = (g0.center - apex).normalized().map_err(|_| bad)?;
                    SurfaceRef::Cone {
                        apex,
                        axis: toward,
                        half_angle: trig::atan((g0.radius - g1.radius).abs() / h.abs()),
                    }
                };
                for band in &mut claims[r * per..(r + 1) * per] {
                    band[0] = vec![Some(claim); m];
                }
            }
        }

        let mut stations: Vec<Vec<Vec<Point3>>> =
            station_rings.into_iter().map(|r| vec![r]).collect();
        if let Some(apex) = spec.apex {
            *stations.last_mut().expect("rows ≥ 2") = vec![vec![apex]];
        }
        let n_stations = stations.len();
        let mut obj = build_skin(&SkinSpec {
            stations,
            column_smooth: vec![hard.iter().map(|&h| !h).collect()],
            station_smooth: vec![true; n_stations],
            cap_start: true,
            cap_end: spec.apex.is_none(),
            claims: Some(claims),
            material: None,
            // Per column, on every band of the span between profiles r and
            // r + 1 (the apex has no corners): a corner in both is a crease,
            // in neither smooth, in exactly one a crease only where it
            // stands out from the columns beside it that are a corner in
            // neither (D-126 R4; a circle meeting a square fans the corner
            // out smoothly, and so does a large square to a small one).
            column_fade: Some(ColumnFade {
                fold: tol::LOFT_CREASE_FOLD,
                rule: (0..n_stations.saturating_sub(1))
                    .map(|s| {
                        let span = s / per;
                        let corner_at = |r: usize, j: usize| r < k && corner[r][j];
                        (0..m)
                            .map(|j| match (corner_at(span, j), corner_at(span + 1, j)) {
                                (true, true) => Fade::Crease,
                                (false, false) => Fade::Soft,
                                (true, false) => Fade::Stand(Side::Start),
                                (false, true) => Fade::Stand(Side::End),
                            })
                            .collect()
                    })
                    .collect(),
                // Per band, each column's corner turn in the span's two
                // profiles (one of them is 0 for a one-sided column).
                turn: (0..n_stations.saturating_sub(1))
                    .map(|s| {
                        let span = s / per;
                        let at = |r: usize, j: usize| if r < k { corner_turn[r][j] } else { 0.0 };
                        (0..m).map(|j| at(span, j).max(at(span + 1, j))).collect()
                    })
                    .collect(),
                // A span's floor is the larger of its two profiles' (the
                // apex has none), as a fold.
                floor: (0..n_stations.saturating_sub(1))
                    .map(|s| {
                        let span = s / per;
                        let at = |r: usize| if r < k { floor_of[r] } else { 0.0 };
                        2.0 * trig::sin(at(span).max(at(span + 1)) / 2.0)
                    })
                    .collect(),
            }),
        })?;
        // A cap that is a whole circle keeps it on its rim edges (each a
        // chord of that circle), as an extrusion's cap does: the rim's
        // identity for a later Push/Pull or a fillet (D-118).
        let ends = [Some(0), (spec.apex.is_none()).then(|| k - 1)];
        for r in ends.into_iter().flatten() {
            let p = &prepared[r];
            let Some(g) = p.curves[0].filter(|&g| p.curves.iter().all(|&c| same_geom(c, Some(g))))
            else {
                continue;
            };
            let plane = Plane::from_point_normal(g.center, p.normal).map_err(|_| bad)?;
            let on = |q: Point3| {
                plane.signed_distance(q).abs() <= tol::PLANE_DIST
                    && ((q - g.center).length() - g.radius).abs() <= tol::POINT_MERGE
            };
            let rim: Vec<crate::ids::EdgeId> = obj
                .edges()
                .keys()
                .filter(|&e| obj.edge_endpoints(e).is_some_and(|(a, b)| on(a) && on(b)))
                .collect();
            for e in rim {
                obj.edges[e].curve = Some(g);
            }
        }
        obj.validate().map_err(|_| bad)?;
        Ok(obj)
    }
}

#[cfg(test)]
mod tests {
    use super::{StationFrame, cr_knots, cr_tangents, limited_tangents};
    use crate::math::Point3;

    /// Rows a uniform knot apart (equal chords): `y` as given, `z` making
    /// up each chord to a length of 4.
    fn rows(ys: &[f64]) -> Vec<Point3> {
        let mut z = 0.0;
        let mut out = vec![Point3::new(0.0, ys[0], 0.0)];
        for w in ys.windows(2) {
            let dy: f64 = w[1] - w[0];
            z += (16.0 - dy * dy).sqrt();
            out.push(Point3::new(0.0, w[1], z));
        }
        out
    }

    #[test]
    fn a_span_that_stays_between_its_rows_keeps_its_catmull_rom_tangents() {
        // y = −3.4, 0, 1, 4.4 with uniform knots: every span of the
        // Catmull–Rom path is monotone in y although its end slopes reach
        // 2.2 times the middle span's, outside the circle of radius 3 the
        // simple Fritsch–Carlson scaling uses; the exact test leaves it.
        let r = rows(&[-3.4, 0.0, 1.0, 4.4]);
        let knots = cr_knots(&r);
        let t = cr_tangents(&r, &knots);
        let limited = limited_tangents(&r, &knots, t.clone(), StationFrame::World);
        assert_eq!(limited, t, "bit for bit");
    }

    #[test]
    fn every_span_ends_monotone_after_limiting() {
        // Scaling one span's end tangent can push the span on its other
        // side out of the monotone region (from (4, 1) on its edge to
        // (4, 0.5) outside): every span must end inside, in every
        // component, whatever the rows.
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 20_001) as f64 / 10_000.0 - 1.0
        };
        for case in 0..4000 {
            let n = 4 + case % 3;
            let r: Vec<Point3> = (0..n)
                .map(|i| Point3::new(next(), next() * 3.0, i as f64 * 0.5 + next() * 0.2))
                .collect();
            let knots = cr_knots(&r);
            let t = limited_tangents(&r, &knots, cr_tangents(&r, &knots), StationFrame::World);
            for i in 0..n - 1 {
                let dt = knots[i + 1] - knots[i];
                for c in 0..3 {
                    let get = |v: Point3| [v.x, v.y, v.z][c];
                    let gt = |v: crate::math::Vec3| [v.x, v.y, v.z][c];
                    let d = (get(r[i + 1]) - get(r[i])) / dt;
                    if d == 0.0 {
                        continue;
                    }
                    // Rounding may leave a scaled pair a few ulps past the
                    // region's edge; that moves a station by as little.
                    let shrink = 1.0 - 1e-12;
                    let (al, be) = (gt(t[i]) / d, gt(t[i + 1]) / d);
                    assert!(
                        al >= 0.0 && be >= 0.0 && super::monotone_span(al * shrink, be * shrink),
                        "case {case}, span {i}, component {c}: ({al}, {be})"
                    );
                }
            }
        }
    }

    #[test]
    fn a_span_that_would_overshoot_is_limited_to_its_rows() {
        // A steep rise then a near-flat step: Catmull–Rom overshoots the last
        // row in y; the limited path stays between the rows of each span.
        let r = rows(&[0.0, 3.0, 3.1]);
        let path = super::catmull_rom(&r, 16, StationFrame::World, None);
        for (i, w) in r.windows(2).enumerate() {
            let (lo, hi) = (w[0].y.min(w[1].y), w[0].y.max(w[1].y));
            for p in &path[i * 16..=(i + 1) * 16] {
                assert!(
                    p.y >= lo - 1e-12 && p.y <= hi + 1e-12,
                    "{p:?} outside {lo}..{hi}"
                );
            }
        }
    }
}
