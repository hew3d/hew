//! Deform (true-curves.md §5.3): Twist, Bend, Taper and Bend to Path — a
//! smooth map of the whole object, applied by clone-validate-swap (§3.4).
//! Faces are triangulated, cut by station planes across the deform axis
//! so the map has somewhere to bend, every point is mapped, and the soup is
//! regrouped into planar faces ([`planarize`]). Surface claims and blend
//! tags go (a non-similarity map); the edges that shaded smooth are
//! softened first, so the result still reads as the curve it was.

use crate::trig;
use std::collections::BTreeMap;

use crate::geom2d::plane_axes;
use crate::ids::VertexId;
use crate::math::{Plane, Point3, Vec3};
use crate::patch::{EdgeMeta, TriSoup, planarize, self_intersection_charged};
use crate::rederive::soften_before_claim_drop;
use crate::shape_error::ShapeError;
use crate::tol;
use crate::topo::{FaceAttrs, Object, WatertightState};
use crate::triangulate::triangulate_face;

/// The deform box of Twist, Bend and Taper.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeformFrame {
    /// Center of the FIXED end face of the deform box.
    pub origin: Point3,
    /// Unit direction from the fixed end to the moving end.
    pub axis: Vec3,
    /// Unit cross axis, perpendicular to `axis`: Bend bends toward +u.
    pub u: Vec3,
    /// Box extent along `axis` (> POINT_MERGE).
    pub length: f64,
}

/// One Deform.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DeformSpec {
    /// Rotate cross-sections about the axis by `angle·w/L` (radians).
    Twist {
        /// The deform box.
        frame: DeformFrame,
        /// Total turn at the moving end, radians, `|angle| ≤ 4π`.
        angle: f64,
        /// Bands along the axis; `None` = one per 15°.
        #[serde(default)]
        stations: Option<u32>,
    },
    /// Bend the axis into a circular arc of total turn `angle` toward +u.
    Bend {
        /// The deform box.
        frame: DeformFrame,
        /// Total turn, radians, `|angle| ≤ 2π`.
        angle: f64,
        /// Bands along the axis; `None` = one per 15°.
        #[serde(default)]
        stations: Option<u32>,
    },
    /// Scale cross-sections about the axis by `1 + (ratio − 1)·w/L`.
    Taper {
        /// The deform box.
        frame: DeformFrame,
        /// Scale at the moving end, `> 0`.
        ratio: f64,
        /// Bands along the axis; `None` = one (planar sides stay planar).
        #[serde(default)]
        stations: Option<u32>,
    },
    /// Remap from the straight base line onto `path`.
    BendToPath {
        /// Start of the base line (maps to the path's start).
        base_from: Point3,
        /// End of the base line (maps to the path's end).
        base_to: Point3,
        /// The path polyline.
        path: Vec<Point3>,
        /// The path closes back to its first point.
        #[serde(default)]
        closed: bool,
        /// Stations inside each path segment, plus one; `None` = 4.
        #[serde(default)]
        stations_per_segment: Option<u32>,
    },
}

const STEP: f64 = std::f64::consts::PI / 12.0;

/// Most stations a Deform or Loft may cut: per span for a typed count, and
/// in all. A larger count refuses `AmountOutOfRange` instead of building
/// an unbounded station list.
pub(crate) const MAX_STATIONS: usize = 4096;

/// Most quads a generated skin may carry (stations × profile points), so
/// a large count and a dense profile cannot multiply into an allocation
/// the WebAssembly heap cannot hold.
pub(crate) const MAX_SKIN_QUADS: usize = 1 << 20;

/// The most work one Deform preview may do, in work units (a piece visited
/// by a station cut, a piece mapped, 8 per rebuilt triangle, a face pair
/// tested for self-intersection; see [`Work`]). A preview that would pass
/// it refuses [`ShapeError::PreviewTooLarge`] instead of holding the
/// pointer; the commit has no cap. Natively (release) the dearest builds
/// spend about two million units a second (most spend three or four), so
/// a preview at the cap gives up within about a tenth of a second there
/// and within about a quarter of a second in WebAssembly: a drag keeps
/// following the pointer. A count, never a clock: the same object and
/// spec always preview or skip alike.
pub const PREVIEW_WORK_CAP: u64 = 300_000;

/// The most work the limit search behind a preview's clamp may do in all
/// ([`Object::deform_preview_limit`]): one preview's worth, so the pointer
/// move that first passes the limit is no slower than any other. A search
/// that needs more (most solids' do: it builds the shape a few dozen times)
/// gives up and the drag skips the preview; the commit still names the
/// limit when it refuses.
pub const PREVIEW_LIMIT_WORK_CAP: u64 = PREVIEW_WORK_CAP;

/// The scale of a limit search's own budgets ([`LIMIT_RETRY_WORK`] and
/// `LIMIT_FIRST_WORK`), which set where the search stops and so which
/// limit a refusal names. Independent of the preview caps: a cap only
/// decides whether a preview answers, never what the commit names.
pub const LIMIT_WORK_SCALE: u64 = 1_500_000;

/// The most work the self-intersection retries of one limit search may do
/// in all ([`Object::deform_limit`]), commit or preview. A genuine fold
/// refuses at every amount past it, and each refusal ran the retry (a
/// heavy rebuild with thin-facet stations): a 1.0 × 0.6 m filleted bar,
/// twisted 30° and bent 300°, searched for 207 s. Past this budget the
/// search's steps build without the retry, so they can only answer lower;
/// every amount the search keeps was built, by the same steps the commit
/// takes, so the limit it names always commits.
pub const LIMIT_RETRY_WORK: u64 = 2 * LIMIT_WORK_SCALE;

/// The most work the first-build halving of a limit search may do before
/// it settles for the precision it has (a plain solid's builds are cheap
/// enough for all 30 steps; a heavy one stops nearer a tenth of that).
const LIMIT_FIRST_WORK: u64 = LIMIT_WORK_SCALE / 2;

/// How far past the last amount that built the retry steps of a limit
/// search reach: half again.
const LIMIT_CLIMB: f64 = 1.5;

/// Steps of each stage of a limit search.
const LIMIT_STEPS: usize = 40;

/// The smallest fraction of the asked amount a limit search seeds at
/// before it settles for none (the identity).
const LIMIT_SMALLEST: f64 = 1e-6;

/// The gap, as a fraction of the proven amount, the first-build stage of a
/// limit search closes before its work budget may stop it.
const LIMIT_CLOSE: f64 = 0.125;

/// Pieces a station cut visits per work unit: most pieces lie wholly on one
/// side and cost one distance test per corner.
const CUT_VISITS_PER_UNIT: u64 = 16;

/// Work units charged per triangle of the rebuilt soup: planarizing and
/// rebuilding cost several piece visits each.
const WORK_PER_TRIANGLE: u64 = 8;

/// Boundary vertices of a face pair tested for self-intersection per work
/// unit: the check reports the vertices of every pair it tests
/// ([`crate::patch::improper_contact_charged`]), and testing eight of them
/// costs about as long as visiting a piece. Long thin faces a twist leaves
/// have many partners each, so the check is often most of a preview's time.
const PAIR_VERTICES_PER_UNIT: u64 = 8;

/// A face pair the self-intersection check passes over because one face
/// lies clear of the other's plane is charged to [`Work::time`] as this
/// many boundary vertices, against the four to eight of a pair it tests in
/// full: measured, the plane check takes about a twentieth of the test.
const APART_PAIR_VERTICES: u64 = 1;

/// Two counts of the work a build does. `used` is the count a limit
/// search's budgets ([`LIMIT_RETRY_WORK`], `LIMIT_FIRST_WORK`) are kept in,
/// so it decides where a search stops and which limit it names. `time` is
/// the same count with each face pair the self-intersection check rules
/// out from the faces' planes alone charged as the little it costs
/// ([`APART_PAIR_VERTICES`]; a dense twist has hundreds of thousands of
/// them): it follows the clock closely enough for a preview's cap
/// ([`PREVIEW_WORK_CAP`]), and never exceeds `used`. `cap` bounds `used`
/// (a search step's retry budget), `time_cap` bounds `time` (a preview);
/// the commit runs with neither.
#[derive(Debug, Clone, Copy)]
struct Work {
    cap: Option<u64>,
    used: u64,
    time_cap: Option<u64>,
    time: u64,
    /// Pair vertices charged to `time` but not yet a whole unit.
    pair_rest: u64,
}

impl Work {
    fn uncapped() -> Work {
        Work {
            cap: None,
            used: 0,
            time_cap: None,
            time: 0,
            pair_rest: 0,
        }
    }

    /// A preview's counter: `time` capped at `cap`.
    fn preview(cap: u64) -> Work {
        Work {
            time_cap: Some(cap),
            ..Work::uncapped()
        }
    }

    /// Charges `n` units to both counts; past either cap the build gives
    /// up.
    fn spend(&mut self, n: u64) -> Result<(), ShapeError> {
        self.spend_split(n, n)
    }

    /// Charges `n` units to `used` and `t` to `time`.
    fn spend_split(&mut self, n: u64, t: u64) -> Result<(), ShapeError> {
        self.used = self.used.saturating_add(n);
        self.time = self.time.saturating_add(t);
        if self.cap.is_some_and(|c| self.used > c) || self.over_time() {
            return Err(ShapeError::PreviewTooLarge);
        }
        Ok(())
    }

    fn over_time(&self) -> bool {
        self.time_cap.is_some_and(|c| self.time > c)
    }

    /// Charges the self-intersection check's report: `n` boundary vertices
    /// to `used` (rounded up per report, as the search budgets were
    /// measured), `t` to `time` (carried until they make a whole unit).
    fn spend_pairs(&mut self, n: u64, t: u64) -> Result<(), ShapeError> {
        let rest = self.pair_rest.saturating_add(t);
        self.pair_rest = rest % PAIR_VERTICES_PER_UNIT;
        self.spend_split(
            n.div_ceil(PAIR_VERTICES_PER_UNIT),
            rest / PAIR_VERTICES_PER_UNIT,
        )
    }
}

/// Minimal rotation taking unit `a` to unit `b`, applied to `v`.
fn min_rotate(a: Vec3, b: Vec3, v: Vec3) -> Vec3 {
    let c = a.dot(b);
    let k = a.cross(b);
    let s = k.length();
    if s <= tol::ROTATION_PARALLEL_SIN {
        if c > 0.0 {
            return v;
        }
        // Antiparallel: a half turn about any axis perpendicular to `a`.
        let (p, _) = plane_axes(a);
        return p * (2.0 * p.dot(v)) - v;
    }
    let k = k * (1.0 / s);
    v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c))
}

/// The point map of one spec, in world coordinates.
enum Map {
    Twist {
        o: Point3,
        a: Vec3,
        u: Vec3,
        v: Vec3,
        l: f64,
        angle: f64,
    },
    Taper {
        o: Point3,
        a: Vec3,
        u: Vec3,
        v: Vec3,
        l: f64,
        ratio: f64,
    },
    Bend {
        o: Point3,
        a: Vec3,
        u: Vec3,
        v: Vec3,
        l: f64,
        angle: f64,
    },
    Path(PathMap),
}

impl Map {
    /// The deform box's length along its axis (the base line's for Bend to
    /// Path): stations lie strictly inside `0..length`.
    fn length(&self) -> f64 {
        match self {
            Map::Twist { l, .. } | Map::Taper { l, .. } | Map::Bend { l, .. } => *l,
            Map::Path(pm) => pm.l,
        }
    }
}

/// Bend to Path's frame along the polyline: per vertex its arc length,
/// tangent (the bisector at a joint) and normal (transported by minimal
/// rotations), interpolated within each segment.
struct PathMap {
    o: Point3,
    a: Vec3,
    u: Vec3,
    v: Vec3,
    l: f64,
    pts: Vec<Point3>,
    cum: Vec<f64>,
    tan: Vec<Vec3>,
    nrm: Vec<Vec3>,
}

impl PathMap {
    fn new(
        base_from: Point3,
        base_to: Point3,
        path: &[Point3],
        closed: bool,
    ) -> Result<PathMap, ShapeError> {
        let bad = ShapeError::AmountOutOfRange;
        let a = (base_to - base_from).normalized().map_err(|_| bad)?;
        let (u, v) = plane_axes(a);
        let l = (base_to - base_from).length();
        let mut pts: Vec<Point3> = path.to_vec();
        if closed {
            pts.push(path[0]);
        }
        pts.dedup_by(|x, y| x.approx_eq(*y, tol::POINT_MERGE));
        if pts.len() < 2 {
            return Err(bad);
        }
        let segs: Vec<Vec3> = pts
            .windows(2)
            .map(|w| (w[1] - w[0]).normalized().map_err(|_| bad))
            .collect::<Result<_, _>>()?;
        let n = pts.len();
        let mut cum = vec![0.0; n];
        for j in 1..n {
            cum[j] = cum[j - 1] + (pts[j] - pts[j - 1]).length();
        }
        let mut tan = Vec::with_capacity(n);
        tan.push(segs[0]);
        for j in 1..n - 1 {
            tan.push((segs[j - 1] + segs[j]).normalized().map_err(|_| bad)?);
        }
        tan.push(segs[n - 2]);
        let mut nrm = Vec::with_capacity(n);
        nrm.push(min_rotate(a, tan[0], u));
        for j in 1..n {
            let t = min_rotate(tan[j - 1], tan[j], nrm[j - 1]);
            let t = t - tan[j] * t.dot(tan[j]);
            nrm.push(t.normalized().map_err(|_| bad)?);
        }
        Ok(PathMap {
            o: base_from,
            a,
            u,
            v,
            l,
            pts,
            cum,
            tan,
            nrm,
        })
    }

    fn map(&self, p: Point3) -> Point3 {
        let d = p - self.o;
        let (w, x, y) = (d.dot(self.a), d.dot(self.u), d.dot(self.v));
        let total = self.cum[self.cum.len() - 1];
        let s = w * total / self.l;
        let n = self.pts.len();
        // Segment containing s; beyond either end the end frame extends
        // straight along the end tangent.
        let (base, t, nn) = if s <= 0.0 {
            (self.pts[0] + self.tan[0] * s, self.tan[0], self.nrm[0])
        } else if s >= total {
            (
                self.pts[n - 1] + self.tan[n - 1] * (s - total),
                self.tan[n - 1],
                self.nrm[n - 1],
            )
        } else {
            let j = match self.cum.binary_search_by(|c| c.total_cmp(&s)) {
                Ok(j) => j.min(n - 2),
                Err(j) => j - 1,
            };
            let len = self.cum[j + 1] - self.cum[j];
            let f = (s - self.cum[j]) / len;
            let base = self.pts[j] + (self.pts[j + 1] - self.pts[j]) * f;
            let t = (self.tan[j] * (1.0 - f) + self.tan[j + 1] * f)
                .normalized()
                .unwrap_or(self.tan[j]);
            let nn = self.nrm[j] * (1.0 - f) + self.nrm[j + 1] * f;
            (base, t, nn)
        };
        let nn = (nn - t * nn.dot(t)).normalized().unwrap_or(self.nrm[0]);
        let b = t.cross(nn);
        base + nn * x + b * y
    }
}

fn frame_axes(f: &DeformFrame) -> Result<(Vec3, Vec3, Vec3), ShapeError> {
    let bad = ShapeError::AmountOutOfRange;
    let unit = |x: Vec3| (x.length() - 1.0).abs() <= tol::NORMAL_DIRECTION;
    if !unit(f.axis)
        || !unit(f.u)
        || f.axis.dot(f.u).abs() > tol::NORMAL_DIRECTION
        || f.length <= tol::POINT_MERGE
        || !f.length.is_finite()
        || ![f.origin.x, f.origin.y, f.origin.z]
            .iter()
            .all(|c| c.is_finite())
    {
        return Err(bad);
    }
    Ok((f.axis, f.u, f.axis.cross(f.u)))
}

impl Map {
    fn map(&self, p: Point3) -> Point3 {
        match self {
            Map::Twist {
                o,
                a,
                u,
                v,
                l,
                angle,
            } => {
                let d = p - *o;
                let (w, x, y) = (d.dot(*a), d.dot(*u), d.dot(*v));
                let (s, c) = trig::sin_cos(angle * w / l);
                *o + *a * w + *u * (x * c - y * s) + *v * (x * s + y * c)
            }
            Map::Taper {
                o,
                a,
                u,
                v,
                l,
                ratio,
            } => {
                let d = p - *o;
                let (w, x, y) = (d.dot(*a), d.dot(*u), d.dot(*v));
                let k = 1.0 + (ratio - 1.0) * w / l;
                *o + *a * w + *u * (k * x) + *v * (k * y)
            }
            Map::Bend {
                o,
                a,
                u,
                v,
                l,
                angle,
            } => {
                let d = p - *o;
                let (w, x, y) = (d.dot(*a), d.dot(*u), d.dot(*v));
                if *angle == 0.0 {
                    return p;
                }
                let r = l / angle;
                let phi = w / r;
                let (s, c) = trig::sin_cos(phi);
                *o + *a * ((r - x) * s) + *u * (r - (r - x) * c) + *v * y
            }
            Map::Path(pm) => pm.map(p),
        }
    }
}

/// The spec's single amount (angle or ratio) and its identity value.
fn amount(spec: &DeformSpec) -> Option<(f64, f64)> {
    match spec {
        DeformSpec::Twist { angle, .. } | DeformSpec::Bend { angle, .. } => Some((*angle, 0.0)),
        DeformSpec::Taper { ratio, .. } => Some((*ratio, 1.0)),
        DeformSpec::BendToPath { .. } => None,
    }
}

fn with_amount(spec: &DeformSpec, value: f64) -> DeformSpec {
    let mut s = spec.clone();
    match &mut s {
        DeformSpec::Twist { angle, .. } | DeformSpec::Bend { angle, .. } => *angle = value,
        DeformSpec::Taper { ratio, .. } => *ratio = value,
        DeformSpec::BendToPath { .. } => {}
    }
    s
}

/// Validates `spec`, returning the point map, the station planes (axis
/// direction, origin, and the sorted interior stations `w`).
#[allow(clippy::type_complexity)]
fn prepare(spec: &DeformSpec) -> Result<(Map, Point3, Vec3, Vec<f64>), ShapeError> {
    let bad = ShapeError::AmountOutOfRange;
    let interior =
        |l: f64, k: u32| -> Vec<f64> { (1..k).map(|i| l * f64::from(i) / f64::from(k)).collect() };
    let count = |angle: f64, stations: Option<u32>| -> Result<u32, ShapeError> {
        match stations {
            Some(0) => Err(bad),
            Some(k) if k as usize > MAX_STATIONS => Err(bad),
            Some(k) => Ok(k),
            None => Ok(((angle.abs() / STEP).ceil() as u32).max(MIN_BANDS)),
        }
    };
    Ok(match spec {
        DeformSpec::Twist {
            frame,
            angle,
            stations,
        } => {
            let (a, u, v) = frame_axes(frame)?;
            if !angle.is_finite() || angle.abs() > 4.0 * std::f64::consts::PI {
                return Err(bad);
            }
            let k = count(*angle, *stations)?;
            (
                Map::Twist {
                    o: frame.origin,
                    a,
                    u,
                    v,
                    l: frame.length,
                    angle: *angle,
                },
                frame.origin,
                a,
                interior(frame.length, k),
            )
        }
        DeformSpec::Bend {
            frame,
            angle,
            stations,
        } => {
            let (a, u, v) = frame_axes(frame)?;
            if !angle.is_finite() || angle.abs() > std::f64::consts::TAU {
                return Err(bad);
            }
            let k = count(*angle, *stations)?;
            (
                Map::Bend {
                    o: frame.origin,
                    a,
                    u,
                    v,
                    l: frame.length,
                    angle: *angle,
                },
                frame.origin,
                a,
                interior(frame.length, k),
            )
        }
        DeformSpec::Taper {
            frame,
            ratio,
            stations,
        } => {
            let (a, u, v) = frame_axes(frame)?;
            if !ratio.is_finite() || *ratio <= 0.0 {
                return Err(bad);
            }
            let k = match stations {
                Some(0) => return Err(bad),
                Some(k) if *k as usize > MAX_STATIONS => return Err(bad),
                Some(k) => *k,
                None => 1,
            };
            (
                Map::Taper {
                    o: frame.origin,
                    a,
                    u,
                    v,
                    l: frame.length,
                    ratio: *ratio,
                },
                frame.origin,
                a,
                interior(frame.length, k),
            )
        }
        DeformSpec::BendToPath {
            base_from,
            base_to,
            path,
            closed,
            stations_per_segment,
        } => {
            if (*base_to - *base_from).length() <= tol::POINT_MERGE
                || path.len() < 2
                || *stations_per_segment == Some(0)
                || stations_per_segment.is_some_and(|k| k as usize > MAX_STATIONS)
                || path
                    .iter()
                    .chain([base_from, base_to])
                    .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()))
            {
                return Err(bad);
            }
            let pm = PathMap::new(*base_from, *base_to, path, *closed)?;
            let per = stations_per_segment.unwrap_or(4);
            let total = pm.cum[pm.cum.len() - 1];
            let mut ws = Vec::new();
            if (pm.cum.len() - 1).saturating_mul(per as usize) > MAX_STATIONS {
                return Err(bad);
            }
            for j in 0..pm.cum.len() - 1 {
                let (s0, s1) = (pm.cum[j], pm.cum[j + 1]);
                if j > 0 {
                    ws.push(s0 * pm.l / total);
                }
                for i in 1..per {
                    ws.push((s0 + (s1 - s0) * f64::from(i) / f64::from(per)) * pm.l / total);
                }
            }
            let (o, a) = (pm.o, pm.a);
            (Map::Path(pm), o, a, ws)
        }
    })
}

/// Station spacing below which [`conforming_stations`] adds no cut, as a
/// fraction of the deform box's length: a station-placement heuristic, not
/// a comparison tolerance. A vertex that close to an end of its piece's
/// span bends with that end, and a cut there would only make slivers.
const CONFORM_FRACTION: f64 = 1e-3;

/// The fewest bands a twist or bend takes by default, whatever its angle:
/// a long face mapped as two chords loses noticeably more volume than one
/// mapped as four (an 18° twist of a blade: 0.2% against under 0.1%).
const MIN_BANDS: u32 = 4;

/// The fewest bands the self-intersection retry of [`Object::deformed`]
/// cuts the box into.
const RETRY_BANDS: usize = 16;

/// Extra stations through the vertices that lie well inside their own
/// piece's span along the axis (a fan center a twist left in a long
/// facet): every piece vertex more than `CONFORM_FRACTION·length` from both
/// ends of its piece's span, merged within that spacing and kept clear of
/// the existing `stations`. Sorted, deterministic.
fn conforming_stations(
    soup: &TriSoup,
    pieces: &[Piece],
    o: Point3,
    a: Vec3,
    length: f64,
    stations: &[f64],
) -> Vec<f64> {
    let gap = (length * CONFORM_FRACTION).max(tol::PLANE_DIST);
    let w_of = |i: u32| (soup.points[i as usize] - o).dot(a);
    let mut candidates: Vec<f64> = Vec::new();
    for (poly, ..) in pieces {
        let ws: Vec<f64> = poly.iter().map(|&i| w_of(i)).collect();
        let lo = ws.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = ws.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        candidates.extend(
            ws.into_iter()
                .filter(|&w| w - lo > gap && hi - w > gap && w > gap && w < length - gap),
        );
    }
    candidates.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for w in candidates {
        let clear = stations.iter().all(|&s| (s - w).abs() > gap)
            && out.last().is_none_or(|&l| w - l > gap);
        if clear {
            out.push(w);
        }
    }
    out
}

/// The largest sag of a chord under the map, as a fraction of the
/// thickness of the piece it crosses, that [`sliver_stations`] allows: a
/// station-placement heuristic, not a comparison tolerance. A thin piece
/// (a strip a twist fanned along a fillet) whose chord sags by about its own
/// thickness maps folded against its neighbors.
const SAG_FRACTION: f64 = 0.25;

/// How fast the map bends a straight line along its axis: a chord of axial
/// length `h` sags by about `h²·rate/8` from the image of the line. Zero
/// for Taper (straight along the axis at every point) and Bend to Path
/// (its own stations sit at the path's joints).
fn axial_bend_rate(map: &Map, points: &[Point3]) -> f64 {
    match map {
        Map::Bend { o, u, l, angle, .. } if *angle != 0.0 => {
            let r = (l / angle).abs();
            let x = points
                .iter()
                .map(|&p| (p - *o).dot(*u).abs())
                .fold(0.0, f64::max);
            (r + x) / (r * r)
        }
        Map::Twist { o, a, l, angle, .. } => {
            let rho = points
                .iter()
                .map(|&p| {
                    let d = p - *o;
                    (d - *a * d.dot(*a)).length()
                })
                .fold(0.0, f64::max);
            let tau = angle / l;
            tau * tau * rho
        }
        _ => 0.0,
    }
}

/// Extra stations for the self-intersection retry where thin pieces run
/// along the axis: over the axial span of every piece whose chords would
/// sag by more than `SAG_FRACTION` of its thickness (twice its area over
/// its longest edge) at the spacing it has, stations close enough that they
/// do not, spread evenly over that piece's span. The cuts are gathered in
/// bins `CONFORM_FRACTION·length` long, each keeping the lowest cut that
/// lands in it, then kept in order more than a bin apart: memory and output
/// stay bounded by the bin count (never more than about
/// `1/CONFORM_FRACTION` stations), and every piece, cut and bin is charged
/// to `work`. Spacing never drops under twice the bin length; stations keep
/// clear of `stations` (sorted) by a bin. Deterministic; empty for a map
/// that keeps axial lines straight.
///
/// # Errors
/// [`ShapeError::PreviewTooLarge`] past `work`'s cap;
/// [`ShapeError::SelfIntersecting`] when the stations would pass
/// [`MAX_STATIONS`].
fn sliver_stations(
    soup: &TriSoup,
    pieces: &[Piece],
    map: &Map,
    o: Point3,
    a: Vec3,
    stations: &[f64],
    work: &mut Work,
) -> Result<Vec<f64>, ShapeError> {
    let length = map.length();
    work.spend(soup.points.len() as u64)?;
    let rate = axial_bend_rate(map, &soup.points);
    if !rate.is_finite() || rate <= 0.0 {
        return Ok(Vec::new());
    }
    let gap = (length * CONFORM_FRACTION).max(tol::PLANE_DIST);
    let bins = ((length / gap).ceil() as usize).max(1);
    let bin_of = |w: f64| ((w / gap).floor().max(0.0) as usize).min(bins - 1);
    // The lowest cut asked for in each bin.
    let mut first: Vec<Option<f64>> = vec![None; bins];
    for (poly, ..) in pieces {
        work.spend(poly.len() as u64)?;
        let pts: Vec<Point3> = poly.iter().map(|&i| soup.points[i as usize]).collect();
        let ws: Vec<f64> = pts.iter().map(|&p| (p - o).dot(a)).collect();
        let lo = ws.iter().copied().fold(f64::INFINITY, f64::min).max(0.0);
        let hi = ws
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            .min(length);
        let span = hi - lo;
        if span <= 2.0 * gap {
            continue;
        }
        let n = pts.len();
        let mut twice_area = Vec3::ZERO;
        let mut longest = 0.0_f64;
        for k in 0..n {
            let (p, q) = (pts[k], pts[(k + 1) % n]);
            twice_area = twice_area + (p - pts[0]).cross(q - pts[0]);
            longest = longest.max((q - p).length());
        }
        if longest <= 0.0 {
            continue;
        }
        let thickness = twice_area.length() / longest;
        let spacing = (8.0 * SAG_FRACTION * thickness / rate)
            .sqrt()
            .max(2.0 * gap);
        if span <= spacing {
            continue;
        }
        // Cuts spread evenly over the piece's own span, as many as its
        // spacing needs; each bin keeps the lowest cut that lands in it.
        let cuts = (span / spacing).ceil() as usize;
        work.spend(cuts as u64)?;
        for i in 1..cuts {
            let w = lo + span * i as f64 / cuts as f64;
            let slot = &mut first[bin_of(w)];
            if slot.is_none_or(|f| w < f) {
                *slot = Some(w);
            }
        }
    }
    work.spend(bins as u64)?;
    let clear = |w: f64| {
        let k = stations.partition_point(|&s| s < w);
        let near = |j: usize| stations.get(j).is_some_and(|&s| (s - w).abs() <= gap);
        !(near(k) || (k > 0 && near(k - 1)))
    };
    let mut out: Vec<f64> = Vec::new();
    for w in first.into_iter().flatten() {
        if w > gap && w < length - gap && clear(w) && out.last().is_none_or(|&l| w - l > gap) {
            out.push(w);
            if stations.len() + out.len() > MAX_STATIONS {
                return Err(ShapeError::SelfIntersecting { limit: None });
            }
        }
    }
    Ok(out)
}

/// One convex planar piece of a face being cut: point indices CCW about
/// the source face's normal, the source face index (its attributes are
/// that face's entry of the list [`pieces_of`] returns beside the pieces,
/// so a cut moves small pieces) and the face's normal.
type Piece = (Vec<u32>, u32, Vec3);

/// The object as convex pieces ready for cutting: every vertex once (slot
/// order); a convex face without holes is one piece, any other face is
/// triangulated into pieces; the original edges' metadata (curve claims
/// dropped: the map is no similarity).
#[allow(clippy::type_complexity)]
fn pieces_of(obj: &Object) -> Result<(TriSoup, Vec<Piece>, Vec<FaceAttrs>), ShapeError> {
    let mut index: BTreeMap<VertexId, u32> = BTreeMap::new();
    let mut soup = TriSoup::default();
    for (vid, v) in obj.vertices() {
        index.insert(vid, soup.points.len() as u32);
        soup.points.push(v.position);
    }
    let loop_ids = |lid| -> Vec<u32> {
        obj.loop_half_edges(lid)
            .map(|h| index[&obj.half_edges()[h].origin])
            .collect()
    };
    let mut pieces = Vec::new();
    let mut face_attrs = Vec::with_capacity(obj.faces().len());
    for (fi, (_, f)) in obj.faces().iter().enumerate() {
        let outer = loop_ids(f.outer_loop);
        let holes: Vec<Vec<u32>> = f.inner_loops.iter().map(|&l| loop_ids(l)).collect();
        let normal = f.plane.normal();
        face_attrs.push(f.attrs());
        let pos =
            |ids: &[u32]| -> Vec<Point3> { ids.iter().map(|&i| soup.points[i as usize]).collect() };
        if holes.is_empty() && convex(&pos(&outer), normal) {
            pieces.push((outer, fi as u32, normal));
            continue;
        }
        let all: Vec<u32> = outer
            .iter()
            .chain(holes.iter().flatten())
            .copied()
            .collect();
        let hole_pts: Vec<Vec<Point3>> = holes.iter().map(|h| pos(h)).collect();
        let tris = triangulate_face(&pos(&outer), &hole_pts, normal)
            .map_err(|_| ShapeError::ShapeDegenerate)?;
        for t in tris {
            pieces.push((vec![all[t[0]], all[t[1]], all[t[2]]], fi as u32, normal));
        }
    }
    for (eid, e) in obj.edges() {
        let Some((a, b)) = obj.edge_vertex_ids(eid) else {
            continue;
        };
        let (a, b) = (index[&a], index[&b]);
        soup.original_edges.insert(
            (a.min(b), a.max(b)),
            EdgeMeta {
                soft: e.soft,
                crease: e.crease,
                curve: None,
                polygon: None,
            },
        );
    }
    Ok((soup, pieces, face_attrs))
}

/// No reflex corner about `normal` (collinear runs allowed).
fn convex(ring: &[Point3], normal: Vec3) -> bool {
    let n = ring.len();
    (0..n).all(|k| {
        let (a, b, c) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
        (b - a).cross(c - b).dot(normal) >= -tol::NORMALIZE_MIN_LENGTH
    })
}

/// Crossing points of one cut closer than this, as a fraction of the
/// deform box's length, are one point: a construction tolerance for the
/// points a cut makes, not a comparison tolerance. Two piece edges fanned
/// nearly parallel from one corner cross a plane nanometers apart (a
/// station through the tilted end cap of a bent, filleted bar, twisted:
/// 16 to 80 nm), and the next deform's cuts turned those into faces under
/// the plane-fit area cutoff.
const CROSSING_MERGE_FRACTION: f64 = 1e-6;

/// The shared-crossing radius never exceeds this fraction of the solid's
/// section (twice its volume over its area: the thickness of a plate, about
/// half that of a square bar), so a merge cannot reach across it.
const CROSSING_MERGE_OF_SECTION: f64 = 1e-3;

/// The most surface the slivers dropped by shared crossings may hold, as a
/// fraction of the solid's area, before a deform refuses
/// [`ShapeError::TooThinToDeform`] rather than return less than was asked.
const DROPPED_AREA_FRACTION: f64 = 1e-6;

/// The crossing points one cut has made, bucketed by position so a new
/// crossing finds a near one in constant time.
struct Crossings {
    cell: f64,
    buckets: BTreeMap<[i64; 3], Vec<u32>>,
}

impl Crossings {
    fn new(cell: f64) -> Crossings {
        Crossings {
            cell,
            buckets: BTreeMap::new(),
        }
    }

    fn key(&self, p: Point3) -> [i64; 3] {
        [
            (p.x / self.cell).floor() as i64,
            (p.y / self.cell).floor() as i64,
            (p.z / self.cell).floor() as i64,
        ]
    }

    /// The first crossing (lowest index) within `cell` of `p`.
    fn near(&self, points: &[Point3], p: Point3) -> Option<u32> {
        let [x, y, z] = self.key(p);
        let mut best: Option<u32> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in self
                        .buckets
                        .get(&[x + dx, y + dy, z + dz])
                        .into_iter()
                        .flatten()
                    {
                        if (points[i as usize] - p).length() <= self.cell
                            && best.is_none_or(|b| i < b)
                        {
                            best = Some(i);
                        }
                    }
                }
            }
        }
        best
    }

    fn add(&mut self, i: u32, p: Point3) {
        let k = self.key(p);
        self.buckets.entry(k).or_default().push(i);
    }
}

/// Cuts every piece by the plane `(p − o)·a = w`, keeping windings; an
/// intersection point is made once per (edge, plane), and a crossing within
/// `merge` of one this cut already made is that point (so both pieces of
/// every edge agree on it); an original edge split by the plane passes its
/// metadata to both halves. A piece side that the shared point collapses
/// below three corners (the sliver between two such edges) is dropped: its
/// neighbors across both edges now meet on the shared point. Convex pieces
/// stay convex. `along` holds `(p − o)·a` for every point of the soup, in
/// order (a crossing's is appended as it is made), so a piece wholly on one
/// side is passed over without measuring its corners again.
#[allow(clippy::too_many_arguments)]
fn cut(
    soup: &mut TriSoup,
    along: &mut Vec<f64>,
    pieces: &mut Vec<Piece>,
    o: Point3,
    a: Vec3,
    w: f64,
    plane: usize,
    made: &mut BTreeMap<(u32, u32, usize), u32>,
    on: f64,
    merge: f64,
    dropped: &mut f64,
) {
    let side = |d: f64| {
        if d > on {
            1
        } else if d < -on {
            -1
        } else {
            0
        }
    };
    debug_assert_eq!(along.len(), soup.points.len());
    let old = std::mem::take(pieces);
    pieces.reserve(old.len());
    let mut crossings = Crossings::new(merge);
    // Every piece in order: one the plane misses as it was, one it crosses
    // as its two sides. (Making a crossing adds a point and never moves
    // one, so each piece's sides read the same whether measured before any
    // piece is cut or just before its own cut.)
    for (poly, face, normal) in old {
        // Rounding keeps order, so the corners' extremes less `w` are the
        // extremes of their distances: a piece crosses exactly when its
        // highest corner is past `on` above the plane and its lowest past
        // `on` below.
        let (lo, hi) = poly
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &i| {
                let x = along[i as usize];
                (l.min(x), h.max(x))
            });
        if !(side(hi - w) == 1 && side(lo - w) == -1) {
            pieces.push((poly, face, normal));
            continue;
        }
        let d: Vec<f64> = poly.iter().map(|&i| along[i as usize] - w).collect();
        let s: Vec<i32> = d.iter().map(|&x| side(x)).collect();
        let n = poly.len();
        // Each corner with where this piece's own edge puts it: a shared
        // crossing's point may sit up to `merge` off this edge.
        let mut pos_side: Vec<(u32, Point3)> = Vec::with_capacity(n + 1);
        let mut neg_side: Vec<(u32, Point3)> = Vec::with_capacity(n + 1);
        for k in 0..n {
            let (i, j) = (poly[k], poly[(k + 1) % n]);
            let (si, sj) = (s[k], s[(k + 1) % n]);
            let pi = soup.points[i as usize];
            if si >= 0 {
                pos_side.push((i, pi));
            }
            if si <= 0 {
                neg_side.push((i, pi));
            }
            if si * sj < 0 {
                let key = (i.min(j), i.max(j), plane);
                let pj = soup.points[j as usize];
                let (di, dj) = (d[k], d[(k + 1) % n]);
                let q = pi + (pj - pi) * (di / (di - dj));
                let x = match made.get(&key) {
                    Some(&x) => x,
                    None => {
                        let x = match crossings.near(&soup.points, q) {
                            Some(x) => x,
                            None => {
                                soup.points.push(q);
                                along.push((q - o).dot(a));
                                let x = (soup.points.len() - 1) as u32;
                                crossings.add(x, q);
                                x
                            }
                        };
                        made.insert(key, x);
                        if let Some(meta) = soup.original_edges.get(&(key.0, key.1)).copied() {
                            for end in [key.0, key.1] {
                                merge_edge_meta(
                                    &mut soup.original_edges,
                                    (end.min(x), end.max(x)),
                                    meta,
                                );
                            }
                        }
                        x
                    }
                };
                pos_side.push((x, q));
                neg_side.push((x, q));
            }
        }
        for side in [pos_side, neg_side] {
            let mut p: Vec<u32> = side.iter().map(|&(i, _)| i).collect();
            p.dedup();
            while p.len() > 1 && p.first() == p.last() {
                p.pop();
            }
            if p.len() >= 3 {
                pieces.push((p, face, normal));
            } else {
                let pts: Vec<Point3> = side.iter().map(|&(_, q)| q).collect();
                *dropped += polygon_area(&pts);
            }
        }
    }
}

/// Records `meta` for the segment `pair` a cut made. Two edges whose
/// crossings share one point can both name the segment from their common
/// corner to it; the record is then the same whichever edge comes first:
/// soft only when both are, a crease when either is.
fn merge_edge_meta(edges: &mut BTreeMap<(u32, u32), EdgeMeta>, pair: (u32, u32), meta: EdgeMeta) {
    edges
        .entry(pair)
        .and_modify(|m| {
            m.soft &= meta.soft;
            m.crease |= meta.crease;
            if m.curve != meta.curve {
                m.curve = None;
            }
            if m.polygon != meta.polygon {
                m.polygon = None;
            }
        })
        .or_insert(meta);
}

/// The area of a planar polygon (Newell).
fn polygon_area(pts: &[Point3]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut twice = Vec3::ZERO;
    for k in 1..n - 1 {
        twice = twice + (pts[k] - pts[0]).cross(pts[k + 1] - pts[0]);
    }
    0.5 * twice.length()
}

impl Object {
    /// Deform (true-curves.md §5.3), pure: the deformed object, or a typed
    /// refusal. The object must be watertight.
    ///
    /// # Errors
    /// [`ShapeError::ShapeNeedsSolid`] for an open object;
    /// [`ShapeError::AmountOutOfRange`] for a non-finite amount, a twist
    /// beyond ±4π or a bend beyond ±2π, a ratio ≤ 0, a box length ≤
    /// `POINT_MERGE`, a non-unit or non-perpendicular frame, a base line
    /// shorter than `POINT_MERGE`, a path under two points, or zero
    /// stations or more than 4096; [`ShapeError::SelfIntersecting`] with the largest amount
    /// that works (`limit`, from [`Object::deform_limit`]) when the result
    /// passes through itself; [`ShapeError::ShapeDegenerate`] for invalid
    /// topology.
    pub fn deformed(&self, spec: &DeformSpec) -> Result<Object, ShapeError> {
        match self.deformed_inner(spec, &mut Work::uncapped()) {
            // The amount asked was just refused: the search starts below it.
            Err(ShapeError::SelfIntersecting { .. }) => Err(ShapeError::SelfIntersecting {
                limit: self
                    .deform_limit_with(spec, &mut Work::uncapped(), true)
                    .ok(),
            }),
            r => r,
        }
    }

    /// What [`Object::deformed`] makes of `spec`, for a live preview: the
    /// same object when it succeeds, but capped at [`PREVIEW_WORK_CAP`]
    /// work units and with no search for the limit when the result passes
    /// through itself (ask [`Object::deform_preview_limit`] once instead of
    /// on every pointer move).
    ///
    /// # Errors
    /// [`Object::deformed`]'s, except that [`ShapeError::SelfIntersecting`]
    /// carries no limit, plus [`ShapeError::PreviewTooLarge`] when the
    /// preview would pass the cap (the commit is still possible).
    pub fn deform_preview(&self, spec: &DeformSpec) -> Result<Object, ShapeError> {
        self.deformed_inner(spec, &mut Work::preview(PREVIEW_WORK_CAP))
    }

    /// The largest |amount| (same sign as the spec's) that
    /// [`Object::deformed`] is shown to build: the amount itself when it
    /// builds (its retry on [`LIMIT_RETRY_WORK`] of its own); otherwise a
    /// search from half the amount down, then up, with first builds, and a
    /// climb past them with retries sharing [`LIMIT_RETRY_WORK`]. Every
    /// amount kept was built, so [`Object::deformed`] builds the answer, and
    /// it is never less than an amount the search saw build; the identity
    /// only when nothing down to a millionth of the amount builds.
    ///
    /// # Errors
    /// [`ShapeError::AmountOutOfRange`] for Bend to Path (no single amount)
    /// and whatever [`Object::deformed`] raises before building.
    pub fn deform_limit(&self, spec: &DeformSpec) -> Result<f64, ShapeError> {
        self.deform_limit_with(spec, &mut Work::uncapped(), false)
    }

    /// [`Object::deform_limit`] for an amount a live preview has just
    /// refused for passing through itself (the drag's clamp asks it then),
    /// under [`PREVIEW_LIMIT_WORK_CAP`] work units in all: the amount itself
    /// is not built again. When it answers, it answers exactly what
    /// [`Object::deform_limit`] and the commit's refusal name for that
    /// amount.
    ///
    /// # Errors
    /// [`Object::deform_limit`]'s, plus [`ShapeError::PreviewTooLarge`]
    /// when the search would pass the cap.
    pub fn deform_preview_limit(&self, spec: &DeformSpec) -> Result<f64, ShapeError> {
        self.deform_limit_with(spec, &mut Work::preview(PREVIEW_LIMIT_WORK_CAP), true)
    }

    /// The limit search. `refused`: the spec's own amount was just refused
    /// (by [`Object::deformed`]), so it is not built again.
    fn deform_limit_with(
        &self,
        spec: &DeformSpec,
        work: &mut Work,
        refused: bool,
    ) -> Result<f64, ShapeError> {
        let Some((target, identity)) = amount(spec) else {
            return Err(ShapeError::AmountOutOfRange);
        };
        prepare(spec)?;
        // Only "passes through itself" narrows the search; any other
        // refusal (over the work cap included) is the answer. `retry`: the
        // budget a step's self-intersection retry may draw on (0: the first
        // build only).
        let fits = |s: &DeformSpec, work: &mut Work, retry: &mut u64| -> Result<bool, ShapeError> {
            let rr = self.deformed_inner_budgeted(s, work, retry);
            match rr {
                Ok(_) => Ok(true),
                Err(ShapeError::SelfIntersecting { .. }) => Ok(false),
                Err(ShapeError::PreviewTooLarge) => Err(ShapeError::PreviewTooLarge),
                // A refusal that is not about the amount's size (the solid
                // check, a degenerate rebuild) also stops the amount here.
                Err(_) => Ok(false),
            }
        };
        // The amount itself, as the commit builds it (its retry on a budget
        // of its own, so the search below is the same whether or not this
        // ran): named when it builds.
        if !refused {
            let mut own = LIMIT_RETRY_WORK;
            if fits(spec, work, &mut own)? {
                return Ok(target);
            }
        }
        let mut retry_left = LIMIT_RETRY_WORK;
        let at = |t: f64| identity + (target - identity) * t;
        // `lo`: the largest fraction of the amount PROVEN to build (0: none
        // yet; the answer never drops below it). `hi`: the smallest seen to
        // refuse.
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        let mut steps = 0;
        // 1. First builds only (cheap, and all a plain solid needs). Seed at
        //    half the amount and a quarter at a time down from there, so
        //    something is proven before anything dear is tried; gallop up
        //    by doubling; then halve the gap. Past `LIMIT_FIRST_WORK` it stops once the
        //    gap is within an eighth of what is proven, and at six times
        //    that whatever the gap.
        let start = work.used;
        let mut t = 0.5_f64;
        while steps < LIMIT_STEPS && t > LIMIT_SMALLEST {
            steps += 1;
            if fits(&with_amount(spec, at(t)), work, &mut 0)? {
                lo = t;
                break;
            }
            hi = t;
            t *= 0.25;
        }
        while lo > 0.0 && steps < LIMIT_STEPS && hi - lo > f64::EPSILON {
            let spent = work.used - start;
            let close = hi - lo <= LIMIT_CLOSE * lo;
            if spent > 6 * LIMIT_FIRST_WORK || (close && spent > LIMIT_FIRST_WORK) {
                break;
            }
            steps += 1;
            let c = (2.0 * lo).min(0.5 * (lo + hi));
            if fits(&with_amount(spec, at(c)), work, &mut 0)? {
                lo = c;
            } else {
                hi = c;
            }
        }
        // 2. Past what first builds reach, a retry may still build (thin
        //    facets a twist left): climb from the proven amount by half again
        //    each time it builds, halving back when it does not, while the
        //    retry budget lasts; with nothing proven, seed low again with
        //    the retry. A step the budget cuts short proves nothing either
        //    way and ends the search.
        let mut hi = 1.0_f64;
        let mut climbing = true;
        let mut seed = 0.125_f64;
        for _ in 0..LIMIT_STEPS {
            if retry_left == 0 || hi - lo <= f64::EPSILON {
                break;
            }
            let c = if lo == 0.0 {
                if seed <= LIMIT_SMALLEST {
                    break;
                }
                seed
            } else if climbing {
                (lo * LIMIT_CLIMB).min(0.5 * (lo + hi))
            } else {
                0.5 * (lo + hi)
            };
            let fit = fits(&with_amount(spec, at(c)), work, &mut retry_left)?;
            if fit {
                lo = c;
            } else if retry_left > 0 {
                if lo == 0.0 {
                    seed *= 0.25;
                } else {
                    hi = c;
                    climbing = false;
                }
            }
        }
        Ok(at(lo))
    }

    /// [`Object::deformed`] as it ran before cuts through every piece vertex
    /// became the rule: the spec's bands only. Test support only, for the
    /// regression specs whose geometry the coarse mapping produced.
    #[doc(hidden)]
    pub fn deformed_coarse_for_test(&self, spec: &DeformSpec) -> Result<Object, ShapeError> {
        self.deformed_at_stations_impl(spec, false, false, &mut Work::uncapped(), None)
    }

    /// [`Object::deform_preview`] under `cap` work units instead of
    /// [`PREVIEW_WORK_CAP`], and the units the build spent as the preview
    /// counts them (`Work::time`; the cap's value when it gave up). Test
    /// support only: the specs exercise the cap on small objects.
    #[doc(hidden)]
    pub fn deform_preview_with_cap_for_test(
        &self,
        spec: &DeformSpec,
        cap: u64,
    ) -> (Result<Object, ShapeError>, u64) {
        let mut work = Work::preview(cap);
        let r = self.deformed_inner(spec, &mut work);
        (r, work.time.min(cap))
    }

    /// One [`cut`] by the plane `x = w` of `pieces` over `points`, the
    /// original edges given as `(pair, soft, crease)`, with crossings shared
    /// within `merge`; the pieces after, every recorded edge as
    /// `(pair, soft, crease)`, and the area the shared crossings dropped.
    /// `self` is not read. Test support only.
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn deform_cut_for_test(
        &self,
        points: Vec<Point3>,
        pieces: Vec<Vec<u32>>,
        edges: Vec<((u32, u32), bool, bool)>,
        w: f64,
        merge: f64,
    ) -> (Vec<Vec<u32>>, Vec<((u32, u32), bool, bool)>, f64) {
        let mut soup = TriSoup {
            points,
            ..TriSoup::default()
        };
        for (pair, soft, crease) in edges {
            soup.original_edges.insert(
                pair,
                EdgeMeta {
                    soft,
                    crease,
                    curve: None,
                    polygon: None,
                },
            );
        }
        let mut ps: Vec<Piece> = pieces
            .into_iter()
            .map(|p| (p, 0, Vec3::new(0.0, 0.0, 1.0)))
            .collect();
        let mut made = BTreeMap::new();
        let mut dropped = 0.0;
        let x = Vec3::new(1.0, 0.0, 0.0);
        let mut along: Vec<f64> = soup
            .points
            .iter()
            .map(|&p| (p - Point3::ORIGIN).dot(x))
            .collect();
        cut(
            &mut soup,
            &mut along,
            &mut ps,
            Point3::ORIGIN,
            x,
            w,
            0,
            &mut made,
            1e-12,
            merge,
            &mut dropped,
        );
        (
            ps.into_iter().map(|p| p.0).collect(),
            soup.original_edges
                .iter()
                .map(|(&k, m)| (k, m.soft, m.crease))
                .collect(),
            dropped,
        )
    }

    /// [`Object::deformed`]'s first build with the shared-crossing radius
    /// forced to `merge`. Test support only: the specs show the dropped-area
    /// guard refusing a radius that would reach across a thin section.
    #[doc(hidden)]
    pub fn deformed_with_merge_for_test(
        &self,
        spec: &DeformSpec,
        merge: f64,
    ) -> Result<Object, ShapeError> {
        self.deformed_at_stations_impl(spec, false, true, &mut Work::uncapped(), Some(merge))
    }

    /// The thin-facet stations the self-intersection retry of `spec` adds
    /// ([`sliver_stations`]) under `cap` work units, and the units it
    /// spent. Test support only.
    #[doc(hidden)]
    pub fn deform_sliver_stations_for_test(
        &self,
        spec: &DeformSpec,
        cap: u64,
    ) -> (Result<Vec<f64>, ShapeError>, u64) {
        let mut work = Work::preview(cap);
        let r = (|| {
            let (map, o, a, stations) = prepare(spec)?;
            let mut clone = self.clone();
            soften_before_claim_drop(&mut clone);
            let (soup, pieces, _) = pieces_of(&clone)?;
            sliver_stations(&soup, &pieces, &map, o, a, &stations, &mut work)
        })();
        (r, work.time.min(cap))
    }

    /// [`Object::deform_preview_limit`] (the amount taken as refused) under
    /// `cap` work units in all. Test support only.
    #[doc(hidden)]
    pub fn deform_preview_limit_with_cap_for_test(
        &self,
        spec: &DeformSpec,
        cap: u64,
    ) -> Result<f64, ShapeError> {
        self.deform_limit_with(spec, &mut Work::preview(cap), true)
    }

    /// [`Object::deformed_inner`] with the retry drawn from `retry_left`
    /// (charged to `work` as well): once it is spent the retry is skipped
    /// and the first refusal stands. A retry that runs out of `work`'s own
    /// time cap is [`ShapeError::PreviewTooLarge`].
    fn deformed_inner_budgeted(
        &self,
        spec: &DeformSpec,
        work: &mut Work,
        retry_left: &mut u64,
    ) -> Result<Object, ShapeError> {
        match self.deformed_at_stations(spec, false, work) {
            Err(e @ ShapeError::SelfIntersecting { .. }) => {
                if *retry_left == 0 {
                    return Err(e);
                }
                let mut sub = Work {
                    cap: Some(*retry_left),
                    time_cap: work.time_cap.map(|c| c.saturating_sub(work.time)),
                    ..Work::uncapped()
                };
                let r = self.deformed_at_stations(spec, true, &mut sub);
                let spent = sub.used.min(*retry_left);
                work.used = work.used.saturating_add(spent);
                work.time = work
                    .time
                    .saturating_add(sub.time.min(sub.time_cap.unwrap_or(u64::MAX)));
                *retry_left = retry_left.saturating_sub(spent);
                match r {
                    Err(ShapeError::PreviewTooLarge) if sub.over_time() => {
                        Err(ShapeError::PreviewTooLarge)
                    }
                    Err(ShapeError::PreviewTooLarge) => {
                        *retry_left = 0;
                        Err(e)
                    }
                    r => r.map_err(|_| e),
                }
            }
            other => other,
        }
    }

    fn deformed_inner(&self, spec: &DeformSpec, work: &mut Work) -> Result<Object, ShapeError> {
        match self.deformed_at_stations(spec, false, work) {
            // Where the facets are narrower than a band's chord sagitta (a
            // filleted rim twisted, then bent) neighbors fold through each
            // other: band the box finely before calling it self-intersecting.
            // Only a success replaces the first refusal, which keeps its
            // limit.
            // Over the work cap the retry stops as well.
            Err(e @ ShapeError::SelfIntersecting { .. }) => {
                match self.deformed_at_stations(spec, true, work) {
                    Err(ShapeError::PreviewTooLarge) => Err(ShapeError::PreviewTooLarge),
                    r => r.map_err(|_| e),
                }
            }
            other => other,
        }
    }

    /// Cuts the object at the spec's stations and through every piece
    /// vertex well inside its span (a piece edge spanning a vertex of its
    /// own piece would map as one chord while the vertex lands on the
    /// deformed surface: a blade's arc tip, all inside one band, lost a
    /// sixth of its volume to that before the vertex cuts were the rule),
    /// then maps and rebuilds it. `fine` is the self-intersection retry:
    /// it also bands the box finely.
    fn deformed_at_stations(
        &self,
        spec: &DeformSpec,
        fine: bool,
        work: &mut Work,
    ) -> Result<Object, ShapeError> {
        self.deformed_at_stations_impl(spec, fine, true, work, None)
    }

    /// `work` is charged as the build goes (see [`Work`]); past its cap the
    /// build stops with [`ShapeError::PreviewTooLarge`].
    fn deformed_at_stations_impl(
        &self,
        spec: &DeformSpec,
        fine: bool,
        conforming: bool,
        work: &mut Work,
        merge_override: Option<f64>,
    ) -> Result<Object, ShapeError> {
        let (map, o, a, mut stations) = prepare(spec)?;
        if fine {
            // The retry bands the box finely, so no piece spans a long
            // stretch of the curved map as one chord.
            let l = map.length();
            let k = (4 * (stations.len() + 1)).max(RETRY_BANDS);
            let mut ws: Vec<f64> = stations.clone();
            ws.extend((1..k).map(|i| l * i as f64 / k as f64));
            ws.sort_by(f64::total_cmp);
            ws.dedup_by(|x, y| (*x - *y).abs() <= l * CONFORM_FRACTION);
            // Past the cap the retry could not finish anyway: give up before
            // cutting anything.
            if ws.len() > MAX_STATIONS {
                return Err(ShapeError::SelfIntersecting { limit: None });
            }
            stations = ws;
        }
        if self.watertight() != WatertightState::Watertight {
            return Err(ShapeError::ShapeNeedsSolid);
        }
        if let Map::Bend { o, u, l, angle, .. } = &map
            && *angle != 0.0
        {
            // Material at or past the bend center folds through itself.
            let r = l / angle;
            let folds = self.vertices().values().any(|vx| {
                let x = (vx.position - *o).dot(*u);
                if r > 0.0 { x >= r } else { x <= r }
            });
            if folds {
                return Err(ShapeError::SelfIntersecting { limit: None });
            }
        }
        let mut clone = self.clone();
        soften_before_claim_drop(&mut clone);
        let (mut soup, mut pieces, face_attrs) = pieces_of(&clone)?;
        work.spend(pieces.len() as u64)?;
        if fine {
            // Thin pieces along the axis also get stations close enough that
            // their chords do not sag by their own thickness.
            let extra = sliver_stations(&soup, &pieces, &map, o, a, &stations, work)?;
            stations.extend(extra);
            stations.sort_by(f64::total_cmp);
        }
        let mut made = BTreeMap::new();
        // A vertex this near a cut counts as on it: the vertices a cut
        // passes through (fan centers, band ends) sit a few ulps apart,
        // and splitting edges beside them would leave slivers.
        let on = if conforming {
            (map.length() * CONFORM_FRACTION * 0.1).max(tol::PLANE_DIST)
        } else {
            tol::PLANE_DIST
        };
        // The solid's area and volume, from its pieces (planar, convex,
        // outward): twice the volume over the area is its section, a
        // thickness no single tiny edge (a chamfer, a micrometer artifact
        // of an earlier deform) can shrink.
        let (area, volume) = pieces
            .iter()
            .fold((0.0, 0.0), |(s_acc, v_acc), (poly, ..)| {
                let pts: Vec<Point3> = poly.iter().map(|&i| soup.points[i as usize]).collect();
                let mut v = 0.0;
                for k in 1..pts.len().saturating_sub(1) {
                    v += pts[0]
                        .to_vec()
                        .dot(pts[k].to_vec().cross(pts[k + 1].to_vec()));
                }
                (s_acc + polygon_area(&pts), v_acc + v / 6.0)
            });
        let section = if area > 0.0 {
            2.0 * volume.abs() / area
        } else {
            0.0
        };
        // The shared-crossing radius: at most a millionth of the box and a
        // thousandth of the solid's section, so it cannot reach across the
        // section (a 0.2 mm bar 1000 m long merged across its own section
        // at a millionth of its length and lost most of its volume).
        let merge = match merge_override {
            Some(m) => m,
            None if conforming => (map.length() * CROSSING_MERGE_FRACTION)
                .min(section * CROSSING_MERGE_OF_SECTION)
                .max(tol::POINT_MERGE),
            None => tol::POINT_MERGE,
        };
        let mut dropped = 0.0;
        let mut along: Vec<f64> = soup.points.iter().map(|&p| (p - o).dot(a)).collect();
        for (i, &w) in stations.iter().enumerate() {
            work.spend(pieces.len() as u64)?;
            cut(
                &mut soup,
                &mut along,
                &mut pieces,
                o,
                a,
                w,
                i,
                &mut made,
                on,
                merge,
                &mut dropped,
            );
        }
        if conforming {
            work.spend(pieces.len() as u64)?;
            let extra = conforming_stations(&soup, &pieces, o, a, map.length(), &stations);
            if stations.len() + extra.len() > MAX_STATIONS {
                return Err(ShapeError::SelfIntersecting { limit: None });
            }
            for (k, &w) in extra.iter().enumerate() {
                work.spend((pieces.len() as u64).div_ceil(CUT_VISITS_PER_UNIT))?;
                cut(
                    &mut soup,
                    &mut along,
                    &mut pieces,
                    o,
                    a,
                    w,
                    stations.len() + k,
                    &mut made,
                    on,
                    merge,
                    &mut dropped,
                );
            }
        }
        // The slivers a shared crossing dropped are a construction detail;
        // were they ever a visible part of the surface, the result would be
        // a smaller solid than asked, so refuse instead.
        if dropped > area * DROPPED_AREA_FRACTION {
            return Err(ShapeError::TooThinToDeform);
        }
        // Map, then triangulate the convex pieces. A four-cornered piece
        // the map bent fans to the image of its own center, a point on the
        // smooth deformed surface: either diagonal alone folds the band in
        // or out by much more than the stations' sagitta. Any other piece
        // triangulates as it lay.
        work.spend(pieces.len() as u64)?;
        let mut mapped: Vec<Point3> = soup.points.iter().map(|&p| map.map(p)).collect();
        for (poly, face, normal) in pieces {
            let attrs = face_attrs[face as usize];
            if poly.len() == 4 {
                let q: Vec<Point3> = poly.iter().map(|&i| mapped[i as usize]).collect();
                let flat = Plane::from_polygon(&q).is_ok_and(|pl| {
                    q.iter()
                        .all(|&p| pl.signed_distance(p).abs() <= tol::PLANE_DIST)
                });
                if !flat {
                    let c = poly
                        .iter()
                        .fold(Vec3::ZERO, |acc, &i| acc + soup.points[i as usize].to_vec())
                        * 0.25;
                    mapped.push(map.map(Point3::ORIGIN + c));
                    let ci = (mapped.len() - 1) as u32;
                    for k in 0..4 {
                        soup.tris
                            .push(([poly[k], poly[(k + 1) % 4], ci], face, attrs));
                    }
                    continue;
                }
            }
            let ring: Vec<Point3> = poly.iter().map(|&i| soup.points[i as usize]).collect();
            let tris =
                triangulate_face(&ring, &[], normal).map_err(|_| ShapeError::ShapeDegenerate)?;
            for t in tris {
                soup.tris
                    .push(([poly[t[0]], poly[t[1]], poly[t[2]]], face, attrs));
            }
        }
        soup.points = mapped;
        work.spend((soup.tris.len() as u64).saturating_mul(WORK_PER_TRIANGLE))?;
        let mut patch = planarize(&clone, &soup);
        patch.expect_watertight = true;
        let result = clone
            .apply_patch(&patch)
            .map_err(|_| ShapeError::ShapeDegenerate)?;
        let obj = result.object;
        // The check is often most of a preview's time (long thin faces have
        // many partners), so it is charged as it goes.
        let crossing = self_intersection_charged(&obj, APART_PAIR_VERTICES, &mut |n, t| {
            work.spend_pairs(n, t).is_ok()
        })
        .map_err(|()| ShapeError::PreviewTooLarge)?;
        if crossing.is_some() {
            return Err(ShapeError::SelfIntersecting { limit: None });
        }
        Ok(obj)
    }
}
