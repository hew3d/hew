//! The drafting engine (unified drafting §4): one way to edit the drawing on
//! a plane, whether it lies on the ground, on a solid's face, or across both.
//!
//! **Lift.** Everything drawn on a plane — the face topology of every
//! participating object's faces on it, those objects' owned sketches, and the
//! free sketches on the plane — is replayed into ONE working [`Sketch`]. A
//! solid's own edges on the plane (where its faces meet non-coplanar ones)
//! are FIXED: drafting splits them where lines meet them, but never moves or
//! removes them.
//!
//! **Edit.** Any [`Sketch`] method runs on the working sketch: drawing,
//! erasing, moving islands, dragging points, offsetting, re-faceting. The
//! sticky rules arrange the edit exactly as they do on the ground.
//!
//! **Lower.** The edit is diffed against the lift by exact positions. Every
//! island it touched (and every island bounding a region it touched) is
//! re-partitioned by where its edges lie: an edge over an object's face that
//! separates two regions is that object's face TOPOLOGY; a loose edge over a
//! face (a free end, a bridge) is the object's OWNED sketch; an edge over no
//! face is FREE. Every other edge keeps its store. Topology changes by
//! minimal delta — only edges no longer drawn are removed, only new edges
//! inserted — through [`Object::retrace_face`], rehearsed on a copy; an
//! island the face cannot hold (a sliver too small for a saved file to
//! reopen) is kept as owned-sketch drawing instead. A loop touching the
//! face's boundary or another loop at one point is topology: the re-trace
//! splits its walk there into loops sharing that vertex. Sketch stores are rewritten only when their content
//! changed. The whole gesture is one labeled undo step.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::geom2d::{interior_point_of_loops, point_near_segment, signed_area_on_plane};
use crate::history::KernelOp;
use crate::retrace::RetracePath;
use crate::sketch::{CurveGeom, SketchCurveKind, SketchEdgeId, SketchVertexId};
use crate::tol;

/// Exact-bits key of an undirected segment, smaller endpoint first.
type SegKey = ([u64; 3], [u64; 3]);

fn bits(p: Point3) -> [u64; 3] {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

fn seg_key(a: Point3, b: Point3) -> SegKey {
    let (x, y) = (bits(a), bits(b));
    if x <= y { (x, y) } else { (y, x) }
}

/// Where a lifted edge is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Store {
    /// Face topology of an object.
    Topo(ObjectId),
    /// A sketch (owned by an object, or free).
    Sketch(SketchId),
}

/// Where an edge belongs after the edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Topo(ObjectId),
    Owned(ObjectId),
    Free,
}

/// One stored edge as lifted.
#[derive(Debug, Clone, Copy)]
struct StoredEdge {
    store: Store,
    a: Point3,
    b: Point3,
}

/// A **foreign edge** (wireframe faces §4.1): a straight edge lying in the
/// lifted plane that no sketch on the plane and no face on it holds — an
/// edge of a free sketch on another plane, or an edge of a visible object
/// that bounds no face on the plane (for a detached lift, one whose two ends
/// are both vertices of the detached shape, §16 R8). Lifted FIXED, like a
/// solid's own edge: drawing splits where it meets one, nothing moves or
/// removes it.
#[derive(Debug, Clone, Copy)]
struct Foreign {
    a: Point3,
    b: Point3,
    /// The free sketch holding it, or `None` for an object's edge.
    home: Option<SketchId>,
    /// For a detached gather: whether it is an edge of the detached shape
    /// being closed — the connected set of §16 R23 (an object's edge is,
    /// by the time it is gathered there). Every region the set's edges
    /// bound is a face of the gesture (§16 R24). `false` for a plane
    /// gather.
    linked: bool,
    /// The curve chain it is a piece of at home (an arc's or circle's
    /// facet): the home sketch and chain, and the chain's analytic record.
    /// The lift adds a chain's pieces in one curve bracket, so a face that
    /// takes them keeps its curve (§16 R30).
    curve: Option<ForeignCurve>,
}

/// A foreign edge's home curve chain ([`Foreign::curve`]).
#[derive(Debug, Clone, Copy)]
struct ForeignCurve {
    home: SketchId,
    chain: crate::sketch::SketchCurveId,
    analytic: Option<crate::sketch::CurveAnalytic>,
}

/// Adds `found` to `work` as a lift's foreign edges: the plain ones at once
/// through the grid, each home curve chain's pieces in one curve bracket
/// (§16 R30). A part the sticky rules refuse (it would collapse onto a
/// vertex) is not offered; nothing of the document changes, the plane only
/// does not see it. Returns the edges added.
fn add_foreign(work: &mut Sketch, found: Vec<Foreign>) -> Vec<Foreign> {
    let (plain, curved): (Vec<Foreign>, Vec<Foreign>) =
        found.into_iter().partition(|f| f.curve.is_none());
    let mut added: Vec<Foreign> = Vec::new();
    if !plain.is_empty() {
        // All at once, through the grid (a ground under thousands of
        // walls' feet would otherwise copy the sketch per foot).
        let segs: Vec<(Point3, Point3)> = plain.iter().map(|f| (f.a, f.b)).collect();
        let mut bulk = work.clone();
        if bulk.add_segments(&segs).is_ok() {
            *work = bulk;
            added = plain;
        } else {
            for f in plain {
                if work.add_segment(f.a, f.b).is_ok() {
                    added.push(f);
                }
            }
        }
    }
    let mut chains: BTreeMap<(SketchId, crate::sketch::SketchCurveId), Vec<Foreign>> =
        BTreeMap::new();
    for f in curved {
        let c = f.curve.expect("partitioned");
        chains.entry((c.home, c.chain)).or_default().push(f);
    }
    for (_, pieces) in chains {
        let c = pieces[0].curve.expect("partitioned");
        let mut trial = work.clone();
        let opened = match c.analytic {
            Some(a) => trial.begin_curve_with_kind(a.geom, a.kind).is_ok(),
            None => {
                trial.begin_curve();
                true
            }
        };
        if !opened {
            continue;
        }
        let mut kept = Vec::new();
        for f in pieces {
            let mut step = trial.clone();
            if step.add_segment(f.a, f.b).is_ok() {
                trial = step;
                kept.push(f);
            }
        }
        trial.end_curve();
        *work = trial;
        added.extend(kept);
    }
    added
}

/// Which foreign edges a lift gathers (wireframe faces §4.1, §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gather {
    /// None: a read-only listing ([`Document::free_plane_units`] and the
    /// other unit listings), which sees materialized faces as ordinary
    /// regions.
    No,
    /// A plane draft: edges of the frame's ordinary free sketches on other
    /// planes, and objects' edges bounding no face on the plane (§16 R2).
    Plane,
    /// A detached shape's own context: edges of the frame's other detached
    /// sketches (off its plane), and objects' edges ending, both ends, on
    /// vertices of the shape (§16 R8).
    Detached,
    /// [`Gather::Detached`], less the other detached sketches' edges the
    /// drawn shape does not reach through shared ends (§16 R29): the lift a
    /// detached draft is lowered from once its drawing is known
    /// ([`Document::detached_shape_completed`]).
    DetachedReached,
    /// The close pass on a candidate plane for a detached seed (wireframe
    /// faces §16 R6): edges of every other detached sketch of the frame
    /// lying in the plane, those on the plane itself (either facing)
    /// included, that the seeds reach through shared ends (§16 R23, R29),
    /// and objects' edges ending, both ends, on vertices of that detached
    /// drawing (§16 R8) — so a detached line closes a face whichever plane
    /// the tool homed it on and in whichever order it was drawn, and a line
    /// that merely crosses it neither splits nor blocks one.
    DetachedClose,
}

/// A plane's drawing lifted into one working sketch.
#[derive(Debug, Clone)]
pub(crate) struct Lifted {
    plane: Plane,
    frame: Option<ComponentId>,
    objects: Vec<ObjectId>,
    /// Each participating object's owned sketches on the plane.
    owned: BTreeMap<ObjectId, Vec<SketchId>>,
    /// The free sketches on the plane, in key order.
    free: Vec<SketchId>,
    /// The objects' own edges on the plane, then the foreign edges (fixed).
    fixed: Vec<(Point3, Point3)>,
    /// The foreign edges among `fixed`, with where each comes from.
    foreign: Vec<Foreign>,
    stored: Vec<StoredEdge>,
    /// The working sketch as lifted, before any edit.
    before: Sketch,
    /// A detached shape's own drafting context ([`Document::lift_detached`],
    /// [`Document::lift_detached_new`]): no objects, no other sketches, and
    /// everything lowers into the one detached sketch (or a new one).
    detached: bool,
    /// The foreign edges the lift gathered.
    gather: Gather,
}

/// An open drafting gesture ([`Document::begin_draft`]).
#[derive(Debug, Clone)]
pub(crate) struct PendingDraft {
    lifted: Lifted,
    work: Sketch,
    label: String,
    /// [`Document::history_generation`] when the draft opened. The lift is
    /// a picture of the document at that moment; any recorded change, undo,
    /// or redo since makes it stale, and the draft closes unlowered.
    generation: u64,
}

/// What a committed draft changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DraftReport {
    /// Objects whose face topology changed.
    pub objects: Vec<ObjectId>,
    /// Sketches written (created, rewritten, or emptied).
    pub sketches: Vec<SketchId>,
    /// Islands kept as owned-sketch drawing because the face could not hold
    /// them (a sliver), as one edge's endpoints each.
    pub kept_as_sketch: Vec<(Point3, Point3)>,
    /// The free sketches on OTHER planes that the edit closed a face in
    /// (wireframe faces §4.2): a loop on one plane is a face, whichever
    /// sketches hold its lines, so a line drawn on one plane can close a
    /// face on another. Each sketch once, in the order written. Empty when
    /// the edit closed none.
    pub faces_closed: Vec<SketchId>,
    /// The solids the edit made from closed faces, and the faces that
    /// closed a cell but made none (wireframe faces §5): what
    /// [`Document::close_cells`] reports, less the faces that closed
    /// nothing at all. Empty when the edit closed no cell.
    pub cells: Vec<super::cells::CellOutcome>,
}

/// A free edge an edit added or moved, with the sketch it now lives in: a
/// seed of [`Document::close_faces_through`].
pub type ClosingEdge = (SketchId, Point3, Point3);

/// Whether `q` is the same plane as `p` (same-facing, coincident).
fn planes_coincide(p: &Plane, q: &Plane) -> bool {
    let anchor = Point3::ORIGIN + p.normal() * (-p.signed_distance(Point3::ORIGIN));
    (p.normal() - q.normal()).length() < tol::NORMAL_DIRECTION
        && q.signed_distance(anchor).abs() <= tol::PLANE_DIST
}

/// Whether `q` is the same geometric plane as `p`, facing either way.
fn planes_coincide_either(p: &Plane, q: &Plane) -> bool {
    planes_coincide(p, q) || planes_coincide(p, &q.flipped())
}

/// Whether face `f` of `obj` lies on the geometric plane of `plane`,
/// facing either way.
pub(super) fn face_on_plane_either(obj: &Object, f: FaceId, plane: &Plane) -> bool {
    face_on_plane(obj, f, plane) || face_on_plane(obj, f, &plane.flipped())
}

/// The parts of segment `a–b` that none of the segments in `cover` lying
/// along its line (both ends within [`tol::POINT_MERGE`] of it) covers, in
/// `a → b` order. A part's ends are `a`, `b`, or an end of a covering
/// segment, never a computed point, so they meet the cover exactly; a
/// part no longer than [`tol::POINT_MERGE`] is no part.
pub(super) fn uncovered_parts(
    a: Point3,
    b: Point3,
    cover: impl IntoIterator<Item = (Point3, Point3)>,
) -> Vec<(Point3, Point3)> {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 0.0 {
        return Vec::new();
    }
    let len = len2.sqrt();
    let eps = tol::POINT_MERGE / len;
    let on_line = |p: Point3| ab.cross(p - a).length() / len <= tol::POINT_MERGE;
    let mut spans: Vec<(f64, Point3, f64, Point3)> = Vec::new();
    for (c, d) in cover {
        if !(on_line(c) && on_line(d)) {
            continue;
        }
        let (tc, td) = ((c - a).dot(ab) / len2, (d - a).dot(ab) / len2);
        let span = if tc <= td {
            (tc, c, td, d)
        } else {
            (td, d, tc, c)
        };
        // Touching at an end only covers nothing.
        if span.2 <= eps || span.0 >= 1.0 - eps {
            continue;
        }
        spans.push(span);
    }
    if spans.is_empty() {
        return vec![(a, b)];
    }
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out = Vec::new();
    let mut at = (0.0, a);
    for (t0, p0, t1, p1) in spans {
        if t0 > at.0 + eps && (p0 - at.1).length() > tol::POINT_MERGE {
            out.push((at.1, p0));
        }
        if t1 > at.0 {
            at = (t1, p1);
        }
    }
    if at.0 < 1.0 - eps && (b - at.1).length() > tol::POINT_MERGE {
        out.push((at.1, b));
    }
    out
}

/// The parts of segment `a–b` (on `plane`) over no face of `objects` on
/// the plane: the segment split where it crosses one of the faces'
/// boundary edges (`boundary`), each part kept when a point inside it lies
/// in no face, and kept parts that meet end to end joined again, so a
/// split that kept both sides leaves no vertex behind.
fn parts_off_faces(
    doc: &Document,
    plane: &Plane,
    objects: &[ObjectId],
    boundary: &[(Point3, Point3)],
    a: Point3,
    b: Point3,
) -> Vec<(Point3, Point3)> {
    let r = b - a;
    let rl = r.length();
    if rl <= tol::POINT_MERGE {
        return Vec::new();
    }
    let n = plane.normal();
    let mut cuts: Vec<f64> = Vec::new();
    for &(c, d) in boundary {
        let s = d - c;
        let sl = s.length();
        let denom = r.cross(s).dot(n);
        if denom == 0.0 || sl <= 0.0 {
            continue;
        }
        let t = (c - a).cross(s).dot(n) / denom;
        let u = (c - a).cross(r).dot(n) / denom;
        let (et, eu) = (tol::POINT_MERGE / rl, tol::POINT_MERGE / sl);
        if t > et && t < 1.0 - et && u >= -eu && u <= 1.0 + eu {
            cuts.push(t);
        }
    }
    cuts.sort_by(f64::total_cmp);
    let mut stops: Vec<(f64, Point3)> = vec![(0.0, a)];
    for t in cuts {
        let q = a + r * t;
        if (q - stops[stops.len() - 1].1).length() > tol::POINT_MERGE {
            stops.push((t, q));
        }
    }
    if (b - stops[stops.len() - 1].1).length() <= tol::POINT_MERGE {
        stops.pop();
    }
    stops.push((1.0, b));
    let covered = |p: Point3| {
        objects
            .iter()
            .any(|&o| face_holding(&doc.objects[o].object, plane, p).is_some())
    };
    let mut out: Vec<(Point3, Point3)> = Vec::new();
    let mut run: Option<Point3> = None;
    for w in stops.windows(2) {
        let (p, q) = (w[0].1, w[1].1);
        if covered(midpoint(p, q)) {
            if let Some(start) = run.take() {
                out.push((start, p));
            }
        } else if run.is_none() {
            run = Some(p);
        }
    }
    if let Some(start) = run {
        out.push((start, b));
    }
    out
}

/// `q` facing the canonical way (wireframe faces §4.2): its normal's first
/// component that is not zero, taken in the order z, y, x, positive — a
/// horizontal plane faces up like the ground.
fn canonical_facing(q: Plane) -> Plane {
    let n = q.normal();
    for c in [n.z, n.y, n.x] {
        if c > tol::NORMAL_DIRECTION {
            return q;
        }
        if c < -tol::NORMAL_DIRECTION {
            return q.flipped();
        }
    }
    q
}

/// Whether face `f` of `obj` lies on `plane` (same-facing, every outer
/// point within [`tol::PLANE_DIST`]).
fn face_on_plane(obj: &Object, f: FaceId, plane: &Plane) -> bool {
    let face = &obj.faces()[f];
    (face.plane.normal() - plane.normal()).length() < tol::NORMAL_DIRECTION
        && obj
            .loop_positions(face.outer_loop)
            .all(|p| plane.signed_distance(p).abs() <= tol::PLANE_DIST)
}

/// The face of `obj` on `plane` holding `p` strictly inside its region.
fn face_holding(obj: &Object, plane: &Plane, p: Point3) -> Option<FaceId> {
    obj.faces()
        .keys()
        .find(|&f| face_on_plane(obj, f, plane) && obj.face_contains_point(f, p))
}

fn midpoint(a: Point3, b: Point3) -> Point3 {
    Point3::ORIGIN + (a.to_vec() + b.to_vec()) * 0.5
}

/// Whether segment `c–d` lies along segment `a–b` (both ends within
/// [`tol::POINT_MERGE`] of it).
pub(super) fn along(c: Point3, d: Point3, a: Point3, b: Point3) -> bool {
    point_near_segment(c, a, b, tol::POINT_MERGE) && point_near_segment(d, a, b, tol::POINT_MERGE)
}

/// A drawn edge's marks for the solid: its circle claim (when both ends lie
/// on the circle) and its regular-polygon claim.
type Claim = (Option<CurveGeom>, Option<CurveGeom>);

/// The circle claim an edge `a–b` may carry: `geom` only when both ends lie
/// on it within `tol_on` (a facet a crossing cut short keeps none —
/// map-or-drop).
fn claim_if_on(geom: Option<CurveGeom>, a: Point3, b: Point3, tol_on: f64) -> Option<CurveGeom> {
    geom.filter(|g| {
        [a, b]
            .iter()
            .all(|&q| ((q - g.center).length() - g.radius).abs() <= tol_on)
    })
}

/// `ring` without its straight-through vertices: a vertex whose neighbours
/// it lies between on one line (within [`tol::POINT_MERGE`]) is no corner.
fn corners(ring: &[Point3]) -> Vec<Point3> {
    let mut pts = ring.to_vec();
    loop {
        let n = pts.len();
        if n <= 3 {
            return pts;
        }
        let straight = (0..n).find(|&i| {
            let (a, v, b) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let ab = b - a;
            let len = ab.length();
            len > tol::POINT_MERGE
                && ab.cross(v - a).length() / len <= tol::POINT_MERGE
                && (v - a).dot(b - v) > 0.0
        });
        match straight {
            Some(i) => {
                pts.remove(i);
            }
            None => return pts,
        }
    }
}

/// A detached working sketch's own drawing and vertices (§16 R23): its
/// edges that lie along none of the lift's `fixed` edges (the foreign
/// edges lifted with it are not its own), and their ends, less a point
/// where a drawn line runs straight through only because a gathered edge
/// ends on it (the working sketch split it there; the shape has no vertex
/// there, so that edge does not link to it).
fn drawn_shape(work: &Sketch, fixed: &[(Point3, Point3)]) -> (Vec<(Point3, Point3)>, Vec<Point3>) {
    let pos = |v: SketchVertexId| work.vertices()[v].position;
    let fixed_at = |p: Point3| {
        fixed
            .iter()
            .any(|&(c, d)| point_near_segment(p, c, d, tol::POINT_MERGE))
    };
    let drawn: Vec<(Point3, Point3)> = work
        .edges()
        .values()
        .map(|e| (pos(e.from), pos(e.to)))
        .filter(|&(a, b)| !fixed.iter().any(|&(c, d)| along(a, b, c, d)))
        .collect();
    let mut shape: Vec<Point3> = Vec::new();
    for &(a, b) in &drawn {
        for p in [a, b] {
            if shape.iter().any(|&q| q.approx_eq(p, tol::POINT_MERGE)) {
                continue;
            }
            let dirs: Vec<Vec3> = drawn
                .iter()
                .filter_map(|&(c, d)| {
                    if c.approx_eq(p, tol::POINT_MERGE) {
                        (d - c).normalized().ok()
                    } else if d.approx_eq(p, tol::POINT_MERGE) {
                        (c - d).normalized().ok()
                    } else {
                        None
                    }
                })
                .collect();
            let straight = dirs.len() == 2
                && dirs[0].cross(dirs[1]).length() <= tol::NORMAL_DIRECTION
                && dirs[0].dot(dirs[1]) < 0.0;
            if !(straight && fixed_at(p)) {
                shape.push(p);
            }
        }
    }
    (drawn, shape)
}

/// The segments of `sk`'s region `r`'s rings, outer then holes, as
/// position pairs.
fn region_segments(sk: &Sketch, r: &crate::sketch::SketchRegion) -> Vec<(Point3, Point3)> {
    let pos = |v: SketchVertexId| sk.vertices()[v].position;
    std::iter::once(&r.outer)
        .chain(r.holes.iter())
        .flat_map(|ring| {
            (0..ring.len())
                .map(|i| (pos(ring[i]), pos(ring[(i + 1) % ring.len()])))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A blank copy of `cur` holding `segs`, each with the curve chain of `cur`
/// it belongs to (`None`: a plain line): the plain ones as segments, each
/// chain's pieces in one curve bracket per contiguous run, with the chain's
/// analytic record (§16 R30: a chain cut into separate runs is a chain per
/// run, never plain segments); `cur`'s faceless loops stay faceless.
fn rebuild_by_runs(
    cur: &Sketch,
    segs: &[(Point3, Point3, Option<crate::sketch::SketchCurveId>)],
) -> Result<Sketch, SketchError> {
    let mut next = cur.blank_like();
    let mut chains: BTreeMap<crate::sketch::SketchCurveId, Vec<(Point3, Point3)>> = BTreeMap::new();
    for &(a, b, c) in segs {
        match c {
            None => {
                next.add_segment(a, b)?;
            }
            Some(c) => chains.entry(c).or_default().push((a, b)),
        }
    }
    for (c, pieces) in chains {
        // Contiguous runs: pieces joined end to end.
        let mut run_of: Vec<usize> = (0..pieces.len()).collect();
        fn root(r: &mut [usize], mut i: usize) -> usize {
            while r[i] != i {
                r[i] = r[r[i]];
                i = r[i];
            }
            i
        }
        for i in 0..pieces.len() {
            for j in i + 1..pieces.len() {
                let (p, q) = (pieces[i], pieces[j]);
                let touch = [p.0, p.1]
                    .iter()
                    .any(|&x| [q.0, q.1].iter().any(|&y| x.approx_eq(y, tol::POINT_MERGE)));
                if touch {
                    let (ri, rj) = (root(&mut run_of, i), root(&mut run_of, j));
                    if ri != rj {
                        run_of[ri] = rj;
                    }
                }
            }
        }
        let mut runs: BTreeMap<usize, Vec<(Point3, Point3)>> = BTreeMap::new();
        for (i, &piece) in pieces.iter().enumerate() {
            let r = root(&mut run_of, i);
            runs.entry(r).or_default().push(piece);
        }
        for run in runs.into_values() {
            match cur.curve_analytic(c) {
                Some(an) => {
                    next.begin_curve_with_kind(an.geom, an.kind)?;
                }
                None => {
                    next.begin_curve();
                }
            }
            let added = run
                .iter()
                .try_for_each(|&(a, b)| next.add_segment(a, b).map(|_| ()));
            next.end_curve();
            added?;
        }
    }
    next.carry_faceless_from(cur, &|p| p, &[]);
    Ok(next)
}

/// Whether two closed rings trace the same polygon: the same corners, as a
/// set (within [`tol::POINT_MERGE`]).
fn same_ring(a: &[Point3], b: &[Point3]) -> bool {
    let (ca, cb) = (corners(a), corners(b));
    ca.len() == cb.len()
        && ca
            .iter()
            .all(|&q| cb.iter().any(|&p| p.approx_eq(q, tol::POINT_MERGE)))
}

/// The region of `w` whose key ([`crate::unit_key::region_key`] of its outer
/// ring) is `key`.
fn region_by_key(w: &Sketch, key: u64) -> Option<crate::sketch::SketchRegionId> {
    let plane = w.plane();
    w.regions().iter().find_map(|(rid, r)| {
        let n = r.outer.len();
        let k = crate::unit_key::region_key((0..n).map(|i| {
            crate::unit_key::edge_key(
                &plane,
                w.vertices()[r.outer[i]].position,
                w.vertices()[r.outer[(i + 1) % n]].position,
            )
        }));
        (n > 0 && k == key).then_some(rid)
    })
}

/// `work` with every NEW vertex (one `before` does not have) that lies
/// within [`tol::POINT_MERGE`] of one of `carriers`, but not on its line,
/// moved onto that line (the nearest point of the segment); `None` when no
/// vertex needs it. Corners and points already on a line stay put.
fn snap_onto_fixed(
    work: &Sketch,
    before: &Sketch,
    carriers: &[(Point3, Point3)],
) -> Option<Sketch> {
    let mut moves: Vec<(Point3, Point3)> = Vec::new();
    for vert in work.vertices().values() {
        let p = vert.position;
        if before.vertices().values().any(|b| b.position == p) {
            continue;
        }
        for &(c, d) in carriers {
            let cd = d - c;
            let len2 = cd.length_squared();
            if len2 <= 0.0 {
                continue;
            }
            let t = ((p - c).dot(cd) / len2).clamp(0.0, 1.0);
            let q = c + cd * t;
            let off = (q - p).length();
            if off == 0.0 || off > tol::POINT_MERGE {
                continue;
            }
            if p.approx_eq(c, tol::POINT_MERGE) || p.approx_eq(d, tol::POINT_MERGE) {
                break; // a corner: the sketch merged it already
            }
            // On the line within the merge tolerance of the segment's end, it
            // IS that end: it merges into the corner, so no sub-tolerance
            // sliver of the edge is left.
            let to = [c, d]
                .into_iter()
                .find(|&k| q.approx_eq(k, tol::POINT_MERGE))
                .unwrap_or(q);
            moves.push((p, to));
            break;
        }
    }
    if moves.is_empty() {
        return None;
    }
    // Rebuild the drawing with the snapped positions: the replay's own
    // arrangement merges a point into a corner and splits an edge at a
    // point on it exactly as drawing there would. If the rebuild is refused,
    // the drawing stays as drawn (the lowering then refuses or demotes as
    // for any other drawing).
    let map = |p: Point3| {
        moves
            .iter()
            .find(|&&(from, _)| from == p)
            .map_or(p, |&(_, to)| to)
    };
    let mut fresh = work.blank_like();
    let edges: Vec<SketchEdgeId> = work.edges().keys().collect();
    replay_edges_mapped(work, &edges, &mut fresh, &map).ok()?;
    fresh.carry_faceless_from(work, &map, &[]);
    Some(fresh)
}

/// The seams of a lifted drawing: points on a solid's own edge where that
/// edge runs straight through (inside one fixed edge, or where two fixed
/// pieces of one line meet — a solid's corner is no seam) and a drawing
/// edge ends.
fn seams_of(work: &Sketch, fixed: &[(Point3, Point3)]) -> Vec<Point3> {
    let pos = |v: SketchVertexId| work.vertices()[v].position;
    let straight_through = |p: Point3| {
        let m = tol::POINT_MERGE;
        // Only the fixed edges whose box (grown by twice the tolerance)
        // holds `p` can pass either test below: a cheap cut first, since a
        // plane can hold thousands of fixed edges.
        let g = 2.0 * m;
        let near: Vec<(Point3, Point3)> = fixed
            .iter()
            .copied()
            .filter(|&(c, d)| {
                p.x >= c.x.min(d.x) - g
                    && p.x <= c.x.max(d.x) + g
                    && p.y >= c.y.min(d.y) - g
                    && p.y <= c.y.max(d.y) + g
                    && p.z >= c.z.min(d.z) - g
                    && p.z <= c.z.max(d.z) + g
            })
            .collect();
        let fixed = &near;
        if fixed.iter().any(|&(c, d)| {
            point_near_segment(p, c, d, m) && !p.approx_eq(c, m) && !p.approx_eq(d, m)
        }) {
            return true;
        }
        let dirs: Vec<Vec3> = fixed
            .iter()
            .filter_map(|&(c, d)| {
                if p.approx_eq(c, m) {
                    (d - c).normalized().ok()
                } else if p.approx_eq(d, m) {
                    (c - d).normalized().ok()
                } else {
                    None
                }
            })
            .collect();
        dirs.len() == 2 && dirs[0].dot(dirs[1]) < -1.0 + 1e-9
    };
    let is_fixed = |a: Point3, b: Point3| fixed.iter().any(|&(c, d)| along(a, b, c, d));
    work.vertices()
        .keys()
        .filter(|&v| straight_through(pos(v)))
        .filter(|&v| {
            work.edges()
                .values()
                .any(|e| (e.from == v || e.to == v) && !is_fixed(pos(e.from), pos(e.to)))
        })
        .map(pos)
        .collect()
}

/// `work` with every seam the edit moved off every fixed edge dissolved
/// where it joins two pieces of one straight line ([`Sketch::dissolve_straight_vertex`])
/// — a shape moved across a solid's edge and back is whole again, as on
/// the ground; `None` when nothing heals.
fn heal_seams(work: &Sketch, fixed: &[(Point3, Point3)]) -> Option<Sketch> {
    let mut out: Option<Sketch> = None;
    for &p in work.seams() {
        let cur = out.as_ref().unwrap_or(work);
        let Some(v) = cur
            .vertices()
            .iter()
            .find(|(_, x)| x.position.approx_eq(p, tol::POINT_MERGE))
            .map(|(id, _)| id)
        else {
            continue;
        };
        if fixed
            .iter()
            .any(|&(c, d)| point_near_segment(p, c, d, tol::POINT_MERGE))
        {
            continue; // still on the boundary: a real seam
        }
        let mut next = cur.clone();
        if next.dissolve_straight_vertex(v).unwrap_or(false) {
            out = Some(next);
        }
    }
    out
}

/// Why a region over a solid is not one of its faces: a [`DraftRegionWhy::
/// Sliver`] by area, else [`DraftRegionWhy::Other`]. (A region touching
/// itself, another shape, or the face's edge at a single point is a face:
/// its face holds loops sharing that vertex.)
fn region_why(outer: &[Point3], holes: &[Vec<Point3>], normal: Vec3) -> DraftRegionWhy {
    let area = signed_area_on_plane(outer, normal).abs()
        - holes
            .iter()
            .map(|h| signed_area_on_plane(h, normal).abs())
            .sum::<f64>();
    if area < 1e-12 {
        return DraftRegionWhy::Sliver;
    }
    DraftRegionWhy::Other
}

/// Groups polygon-marked edges into one chain per polygon. The mark holds
/// only the circumcircle, so two polygons inscribed in one circle (a star's
/// two triangles) share it; they are told apart by how their pieces meet:
/// end to end along one line (the same side, split where something crossed
/// it) or at a corner on the circle (the next side of the same count). A
/// crossing of two polygons' sides joins neither.
fn polygon_chains(
    edges: &[(CurveGeom, Point3, Point3)],
) -> Vec<(CurveGeom, Vec<(Point3, Point3)>)> {
    fn find(root: &mut [usize], mut i: usize) -> usize {
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    let mut root: Vec<usize> = (0..edges.len()).collect();
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            let ((g, a, b), (h, c, d)) = (edges[i], edges[j]);
            if g != h {
                continue;
            }
            let Some(v) = [a, b]
                .into_iter()
                .find(|p| p.approx_eq(c, tol::POINT_MERGE) || p.approx_eq(d, tol::POINT_MERGE))
            else {
                continue;
            };
            let corner =
                ((v - g.center).length() - g.radius).abs() <= tol::PLANE_DIST.max(g.radius * 1e-7);
            let along = match ((b - a).normalized(), (d - c).normalized()) {
                (Ok(u), Ok(w)) => u.cross(w).length() <= 1e-7,
                _ => false,
            };
            // Polygons sharing a circle and a corner (a triangle and a
            // square started at one point) differ in their sides' count.
            let same_kind = g.polygon_side_count(a, b) == g.polygon_side_count(c, d);
            if (corner && same_kind) || along {
                let (x, y) = (find(&mut root, i), find(&mut root, j));
                root[x] = y;
            }
        }
    }
    let mut roots: Vec<usize> = Vec::new();
    let mut out: Vec<(CurveGeom, Vec<(Point3, Point3)>)> = Vec::new();
    for (i, &(g, a, b)) in edges.iter().enumerate() {
        let r = find(&mut root, i);
        match roots.iter().position(|&k| k == r) {
            Some(k) => out[k].1.push((a, b)),
            None => {
                roots.push(r);
                out.push((g, vec![(a, b)]));
            }
        }
    }
    out
}

impl Document {
    /// Lifts the drawing on `plane` in `frame` (the world, or one
    /// definition's local space) into one working sketch: the face topology
    /// of every face of `objects` lying on the plane (their own boundary
    /// edges fixed), those objects' owned sketches on the plane, and the free
    /// sketches on it.
    ///
    /// # Errors
    /// [`DocumentError::UnknownObject`] for a stale, hidden, or other-frame
    /// object; [`DocumentError::Sketch`] if the lifted drawing cannot be
    /// replayed (never for a valid document).
    pub(crate) fn lift_plane(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
    ) -> Result<Lifted, DocumentError> {
        self.lift_plane_with(plane, frame, objects, None, Gather::No)
    }

    /// [`Document::lift_plane`] for a draft: the foreign edges lying in the
    /// plane are lifted too, fixed (wireframe faces §4.1), so a loop the
    /// edit closes with lines held by sketches on other planes, or with an
    /// object's edge, is a region of the plane.
    ///
    /// # Errors
    /// As [`Document::lift_plane`].
    fn lift_plane_for_draft(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
    ) -> Result<Lifted, DocumentError> {
        self.lift_plane_with(plane, frame, objects, None, Gather::Plane)
    }

    /// The close pass's lift of an ordinary candidate plane (wireframe
    /// faces §4.2, §16 R11): [`Document::lift_plane_for_draft`] with only
    /// the drawing and objects that take part — the plane's free sketches
    /// that [`Document::sketch_takes_part`] accepts, and the objects with a
    /// face on it that [`Document::object_takes_part`] accepts — so a
    /// tag-hidden or closed group's line, or a view-hidden solid, closes
    /// nothing.
    ///
    /// # Errors
    /// As [`Document::lift_plane`].
    fn lift_plane_for_close(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
    ) -> Result<Lifted, DocumentError> {
        let objects: Vec<ObjectId> = self
            .drawable_objects_on_plane(plane, frame)
            .into_iter()
            .filter(|&o| self.object_takes_part(o, frame))
            .collect();
        let free: Vec<SketchId> = self
            .sketches
            .iter()
            .filter(|&(s, sk)| {
                self.sketch_takes_part(s, frame)
                    && !sk.detached()
                    && planes_coincide(&plane, &sk.plane())
            })
            .map(|(s, _)| s)
            .collect();
        self.lift_plane_with(plane, frame, &objects, Some(free), Gather::Plane)
    }

    /// [`Document::lift_plane`] with the plane's free sketches given
    /// (`free`), when the caller has already found them, instead of found
    /// by walking every sketch of the frame — what
    /// [`Document::free_plane_units`] does once for all planes — and with
    /// the foreign edges `gather` names lifted fixed after everything else.
    fn lift_plane_with(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
        free: Option<Vec<SketchId>>,
        gather: Gather,
    ) -> Result<Lifted, DocumentError> {
        self.lift_plane_with_own(plane, frame, objects, free, gather, None)
    }

    /// [`Document::lift_plane_with`], naming the detached shape's own
    /// vertices for a detached gather (`own`, [`Document::gather_foreign`]);
    /// `None`: the lifted sketches'.
    fn lift_plane_with_own(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
        free: Option<Vec<SketchId>>,
        gather: Gather,
        own: Option<&[Point3]>,
    ) -> Result<Lifted, DocumentError> {
        let mut objs: Vec<ObjectId> = Vec::new();
        for &o in objects {
            if self.objects.get(o).is_none_or(|r| r.hidden)
                || self.object_owner_component(o) != frame
            {
                return Err(DocumentError::UnknownObject);
            }
            if !objs.contains(&o) {
                objs.push(o);
            }
        }
        let mut work = Sketch::on_plane(plane);
        let mut fixed: Vec<(Point3, Point3)> = Vec::new();
        let mut stored: Vec<StoredEdge> = Vec::new();
        let mut claimed: Vec<(Point3, Point3, CurveGeom)> = Vec::new();
        let mut poly_claimed: Vec<(Point3, Point3, CurveGeom)> = Vec::new();

        // Face topology: every edge of every face on the plane, once.
        for &o in &objs {
            let obj = &self.objects[o].object;
            let on: BTreeSet<FaceId> = obj
                .faces()
                .keys()
                .filter(|&f| face_on_plane(obj, f, &plane))
                .collect();
            let mut seen: BTreeSet<EdgeId> = BTreeSet::new();
            for &f in &on {
                let face = &obj.faces()[f];
                for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                    for h in obj.loop_half_edges(l) {
                        let he = &obj.half_edges()[h];
                        if !seen.insert(he.edge) {
                            continue;
                        }
                        let Some((a, b)) = obj.edge_endpoints(he.edge) else {
                            continue;
                        };
                        let other = he
                            .twin
                            .map(|t| obj.loops()[obj.half_edges()[t].loop_id].face);
                        // An edge between two faces on the plane is drawing,
                        // unless they are painted differently or the edge is
                        // soft: that split is the solid's content (the paint
                        // boundary, a source's smoothed seam — a terrain's
                        // flat triangles), kept like its own edges.
                        if !obj.edges()[he.edge].soft
                            && other.is_some_and(|g| {
                                on.contains(&g)
                                    && obj.faces()[g].material == face.material
                                    && obj.faces()[g].back_material == face.back_material
                            })
                        {
                            stored.push(StoredEdge {
                                store: Store::Topo(o),
                                a,
                                b,
                            });
                            if let Some(g) = obj.edges()[he.edge].curve {
                                claimed.push((a, b, g));
                            } else if let Some(g) = obj.edges()[he.edge]
                                .polygon
                                .filter(|g| g.polygon_side_count(a, b).is_some())
                            {
                                poly_claimed.push((a, b, g));
                            }
                        } else {
                            fixed.push((a, b));
                        }
                    }
                }
            }
        }
        // All the face edges at once: the same sticky rules, found
        // through a grid rather than a scan of every edge per edge.
        work.add_segments(&fixed).map_err(DocumentError::Sketch)?;
        // Claimed facets go in one chain per circle, as a drawn circle's do,
        // so chain-granular edits (select or erase the circle, offset,
        // refacet, rim inference) see the whole circle, not one facet.
        let mut circles: Vec<(CurveGeom, Vec<(Point3, Point3)>)> = Vec::new();
        let mut poly_edges: Vec<(CurveGeom, Point3, Point3)> = Vec::new();
        let mut plain: Vec<(Point3, Point3)> = Vec::new();
        for e in &stored {
            let key = seg_key(e.a, e.b);
            if let Some(&(_, _, g)) = claimed.iter().find(|c| seg_key(c.0, c.1) == key) {
                match circles.iter_mut().find(|(h, _)| *h == g) {
                    Some((_, segs)) => segs.push((e.a, e.b)),
                    None => circles.push((g, vec![(e.a, e.b)])),
                }
            } else if let Some(&(_, _, g)) = poly_claimed.iter().find(|c| seg_key(c.0, c.1) == key)
            {
                // A regular polygon's sides: one polygon chain, as drawn
                // (grouped below).
                poly_edges.push((g, e.a, e.b));
            } else {
                plain.push((e.a, e.b));
            }
        }
        let polygons = polygon_chains(&poly_edges);
        // A facet a crossing cut mid-way keeps no claim in the solid (a claim
        // needs both ends on the circle), yet its pieces are still the
        // circle's: each lies on the chord between two of the circle's
        // vertices no farther apart than its longest facet. They rejoin the
        // circle's chain, so the circle's runs close as drawn.
        let chords: Vec<(f64, Vec<(Point3, Point3)>)> = circles
            .iter()
            .map(|(g, segs)| {
                let on = |q: Point3| ((q - g.center).length() - g.radius).abs() <= tol::PLANE_DIST;
                let longest = segs
                    .iter()
                    .map(|&(p, q)| (q - p).length())
                    .fold(0.0, f64::max);
                let mut verts: Vec<Point3> = Vec::new();
                for q in segs
                    .iter()
                    .flat_map(|&(p, q)| [p, q])
                    .chain(plain.iter().flat_map(|&(p, q)| [p, q]).filter(|&q| on(q)))
                {
                    if !verts.iter().any(|v| v.approx_eq(q, tol::POINT_MERGE)) {
                        verts.push(q);
                    }
                }
                let mut pairs = Vec::new();
                for (i, &p) in verts.iter().enumerate() {
                    for &q in &verts[i + 1..] {
                        let d = (q - p).length();
                        if d > tol::POINT_MERGE && d <= longest + tol::PLANE_DIST {
                            pairs.push((p, q));
                        }
                    }
                }
                (longest, pairs)
            })
            .collect();
        for &(a, b) in &plain {
            let home = circles
                .iter()
                .zip(&chords)
                .position(|((g, _), (longest, pairs))| {
                    // A chord lies between the circle and its facets' sagitta.
                    let near = |q: Point3| {
                        let d = (q - g.center).length();
                        d <= g.radius + tol::PLANE_DIST && d >= g.radius - longest
                    };
                    near(a)
                        && near(b)
                        && pairs.iter().any(|&(p, q)| {
                            point_near_segment(a, p, q, tol::POINT_MERGE)
                                && point_near_segment(b, p, q, tol::POINT_MERGE)
                        })
                });
            match home {
                Some(i) => circles[i].1.push((a, b)),
                None => {
                    work.add_segment(a, b).map_err(DocumentError::Sketch)?;
                }
            }
        }
        for (g, segs) in circles {
            work.begin_curve_with(g).map_err(DocumentError::Sketch)?;
            let r = segs
                .iter()
                .try_for_each(|&(a, b)| work.add_segment(a, b).map(|_| ()));
            work.end_curve();
            r.map_err(DocumentError::Sketch)?;
        }
        for (g, segs) in polygons {
            work.begin_curve_with_kind(g, SketchCurveKind::Polygon)
                .map_err(DocumentError::Sketch)?;
            let r = segs
                .iter()
                .try_for_each(|&(a, b)| work.add_segment(a, b).map(|_| ()));
            work.end_curve();
            r.map_err(DocumentError::Sketch)?;
        }

        // Sketches: the objects' owned ones, then the free ones.
        let mut owned: BTreeMap<ObjectId, Vec<SketchId>> = BTreeMap::new();
        for &o in &objs {
            for s in self.object_owned_sketches(o) {
                if planes_coincide(&plane, &self.sketches[s].plane()) {
                    owned.entry(o).or_default().push(s);
                }
            }
        }
        let free: Vec<SketchId> = match free {
            Some(given) => given,
            None => self
                .sketches
                .iter()
                .filter(|&(s, sk)| {
                    !self.sketch_tombstoned(&s)
                        && !self.object_sketches.contains_key(&s)
                        && self.def_sketches.get(&s).copied() == frame
                        // A detached shape takes part in no plane's
                        // arrangement: it is its own drafting context.
                        && !sk.detached()
                        && planes_coincide(&plane, &sk.plane())
                })
                .map(|(s, _)| s)
                .collect(),
        };
        let sketch_ids: Vec<SketchId> = owned
            .values()
            .flatten()
            .copied()
            .chain(free.iter().copied())
            .collect();
        for &s in &sketch_ids {
            let sk = &self.sketches[s];
            // (A loop with no face stays one on the working sketch; a face
            // already there, a solid's, keeps its face under a wire.)
            let faced = work.faced_rings();
            replay_sketch_into(sk, &mut work).map_err(DocumentError::Sketch)?;
            work.carry_faceless_from(sk, &|p| p, &faced);
            for e in sk.edges().values() {
                stored.push(StoredEdge {
                    store: Store::Sketch(s),
                    a: sk.vertices()[e.from].position,
                    b: sk.vertices()[e.to].position,
                });
            }
        }

        // Foreign edges last, and only when there are any, so a plane that
        // shares no edge with another lifts exactly as it always has.
        let mut foreign: Vec<Foreign> = Vec::new();
        if gather != Gather::No {
            let held: Vec<(Point3, Point3)> = fixed
                .iter()
                .copied()
                .chain(stored.iter().map(|s| (s.a, s.b)))
                .collect();
            let boundary: Vec<(Point3, Point3)> = fixed
                .iter()
                .copied()
                .chain(
                    stored
                        .iter()
                        .filter(|s| matches!(s.store, Store::Topo(_)))
                        .map(|s| (s.a, s.b)),
                )
                .collect();
            let found = self.gather_foreign(
                &plane,
                frame,
                gather,
                &sketch_ids,
                &objs,
                &held,
                &boundary,
                own,
            );
            if !found.is_empty() {
                foreign = add_foreign(&mut work, found);
                fixed.extend(foreign.iter().map(|f| (f.a, f.b)));
            }
        }

        // From here on, new drawing that touches existing geometry snaps
        // onto it (an edge splits straight, a corner merges), and the points
        // where drawing meets a solid's edge are tracked as seams.
        work.set_snap_to_existing(true);
        let seams = seams_of(&work, &fixed);
        work.set_seams(seams);
        // A foreign facet keeps its curve (§16 R30) but is another sketch's
        // or an object's line: drawing ending on its exact circle does not
        // bend it there, or it would no longer be drawn end to end.
        work.set_rigid(foreign.iter().map(|f| (f.a, f.b)).collect());
        Ok(Lifted {
            plane,
            frame,
            objects: objs,
            owned,
            free,
            fixed,
            foreign,
            stored,
            before: work,
            detached: false,
            gather,
        })
    }

    /// The foreign edges of `plane` in `frame` (wireframe faces §4.1), as
    /// parts no edge the lift already holds (`held`) covers and, when
    /// objects take part, lying over none of their faces on the plane
    /// (`boundary` is those faces' edges): a line from another plane that
    /// runs across a solid's face is not drawing on that face. Sources, by
    /// `gather`:
    ///
    /// - [`Gather::Plane`]: every edge of a live, ordinary free sketch of
    ///   the frame not on the plane, lying in the plane. A sketch on the
    ///   same geometric plane facing the other way counts as another plane
    ///   (§4.1): it is not lifted with this one, so its edges ARE gathered
    ///   here, as foreign;
    /// - [`Gather::Detached`]: every edge of another live detached sketch of
    ///   the frame not on the plane's geometric plane, lying in it;
    /// - [`Gather::DetachedClose`]: every edge of a live detached sketch of
    ///   the frame lying in the plane, those on the plane itself included
    ///   (the close pass's lift for a detached seed, §16 R6), that the
    ///   shape reaches through shared ends (§16 R29);
    /// - [`Gather::DetachedReached`]: as [`Gather::Detached`], those the
    ///   shape reaches only (§16 R29);
    /// - [`Gather::Plane`]: every edge of a visible object of the frame
    ///   (top-level in the world, or the definition's member) lying in the
    ///   plane that bounds no face on the plane's geometric plane, facing
    ///   either way — a box standing on the ground lends the ground nothing
    ///   (§16 R2);
    /// - [`Gather::Detached`] and [`Gather::DetachedClose`]: every edge of a
    ///   visible object of the frame lying in the plane whose two ends are
    ///   both vertices of the detached shape being closed (§16 R8, Option Z
    ///   narrowed; §16 R23). That shape is its own vertices — `own` when
    ///   given (the drawing after an edit, or the close pass's seed edges'
    ///   ends), else the lifted detached sketch's — and every gathered
    ///   detached edge reachable from them through chains of detached edges
    ///   sharing an END (within [`tol::POINT_MERGE`]); a vertex lying on
    ///   another detached edge does not link (detached sketches on one plane
    ///   never arrange together). Whether the Object edge bounds a face on
    ///   the plane does not matter: a gable's rafters, sharing the apex and
    ///   ending on a box's top corners, close against the box's top edge,
    ///   while a rectangle drawn around the box's footprint, or a detached
    ///   line that merely ends on its edge, shares no vertex with it.
    ///
    /// One pass over the frame's sketches and objects: a sketch parallel to
    /// the plane and off it, and an object whose bounding box the plane
    /// misses, are skipped whole; every other edge costs two dot products.
    #[allow(clippy::too_many_arguments)]
    fn gather_foreign(
        &self,
        plane: &Plane,
        frame: Option<ComponentId>,
        gather: Gather,
        lifted: &[SketchId],
        objects: &[ObjectId],
        held: &[(Point3, Point3)],
        boundary: &[(Point3, Point3)],
        own: Option<&[Point3]>,
    ) -> Vec<Foreign> {
        let on = |p: Point3| plane.signed_distance(p).abs() <= tol::PLANE_DIST;
        let mut candidates: Vec<(Point3, Point3, Option<SketchId>)> = Vec::new();
        let mut cand_curve: Vec<Option<ForeignCurve>> = Vec::new();
        for (s, sk) in self.sketches.iter() {
            // Only drawing that takes part lends edges (§16 R11): not a
            // tag-hidden sketch, nor a closed group's.
            if lifted.contains(&s) || !self.sketch_takes_part(s, frame) {
                continue;
            }
            let sp = sk.plane();
            let source = match gather {
                Gather::No => false,
                Gather::Plane => !sk.detached() && !planes_coincide(plane, &sp),
                Gather::Detached | Gather::DetachedReached => {
                    sk.detached() && !planes_coincide_either(plane, &sp)
                }
                Gather::DetachedClose => sk.detached(),
            };
            if !source {
                continue;
            }
            if sp.normal().cross(plane.normal()).length() <= tol::NORMAL_DIRECTION
                && !on(sp.point())
            {
                continue;
            }
            for e in sk.edges().values() {
                let (a, b) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
                if on(a) && on(b) {
                    candidates.push((a, b, Some(s)));
                    cand_curve.push(e.curve.map(|c| ForeignCurve {
                        home: s,
                        chain: c,
                        analytic: sk.curve_analytic(c),
                    }));
                }
            }
        }
        // A detached lift's shape (§16 R8): the vertices an object's edge
        // must end on, both ends, to be foreign there.
        let detached = matches!(
            gather,
            Gather::Detached | Gather::DetachedReached | Gather::DetachedClose
        );
        let mut joined = vec![false; candidates.len()];
        let shape: Vec<Point3> = if detached {
            let mut shape: Vec<Point3> = match own {
                Some(pts) => pts.iter().copied().filter(|&p| on(p)).collect(),
                None => lifted
                    .iter()
                    .flat_map(|&s| self.sketches[s].vertices().values().map(|v| v.position))
                    .filter(|&p| on(p))
                    .collect(),
            };
            // The detached edges connected to it through shared ends (§16
            // R23), until no more join.
            loop {
                let mut grew = false;
                for (k, &(a, b, home)) in candidates.iter().enumerate() {
                    if joined[k] || home.is_none() {
                        continue;
                    }
                    let at = |shape: &[Point3], p: Point3| {
                        shape.iter().any(|&q| q.approx_eq(p, tol::POINT_MERGE))
                    };
                    if at(&shape, a) || at(&shape, b) {
                        joined[k] = true;
                        grew = true;
                        for p in [a, b] {
                            if !at(&shape, p) {
                                shape.push(p);
                            }
                        }
                    }
                }
                if !grew {
                    break shape;
                }
            }
        } else {
            Vec::new()
        };
        let of_shape = |p: Point3| shape.iter().any(|&q| q.approx_eq(p, tol::POINT_MERGE));
        for (id, rec) in self.objects.iter() {
            if (detached && shape.is_empty()) || !self.object_takes_part(id, frame) {
                continue;
            }
            let obj = &rec.object;
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for v in obj.vertices().values() {
                let p = v.position;
                for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
            if lo[0] > hi[0] {
                continue;
            }
            let mut above = false;
            let mut below = false;
            for i in 0..8 {
                let pick = |k: usize| if i >> k & 1 == 0 { lo[k] } else { hi[k] };
                let d = plane.signed_distance(Point3::new(pick(0), pick(1), pick(2)));
                above |= d >= -tol::PLANE_DIST;
                below |= d <= tol::PLANE_DIST;
            }
            if !(above && below) {
                continue;
            }
            for (eid, edge) in obj.edges().iter() {
                let Some((a, b)) = obj.edge_endpoints(eid) else {
                    continue;
                };
                if !(on(a) && on(b)) {
                    continue;
                }
                if detached {
                    if of_shape(a) && of_shape(b) {
                        candidates.push((a, b, None));
                        joined.push(true);
                    }
                    continue;
                }
                let bounds_face_here = std::iter::once(edge.half_edge)
                    .chain(edge.twin_half_edge)
                    .any(|h| {
                        let f = obj.loops()[obj.half_edges()[h].loop_id].face;
                        face_on_plane_either(obj, f, plane)
                    });
                if !bounds_face_here {
                    candidates.push((a, b, None));
                }
            }
        }
        let mut out: Vec<Foreign> = Vec::new();
        // The close pass and a drawn detached shape arrange their connected
        // set only (§16 R29): a detached line the set does not reach through
        // shared ends neither splits nor blocks its faces.
        let reached_only = matches!(gather, Gather::DetachedReached | Gather::DetachedClose);
        for (k, (a, b, home)) in candidates.into_iter().enumerate() {
            let linked = joined.get(k).copied().unwrap_or(false);
            if reached_only && home.is_some() && !linked {
                continue;
            }
            let curve = cand_curve.get(k).copied().flatten();
            let (lo, hi) = (
                Point3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
                Point3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
            );
            let m = tol::POINT_MERGE;
            let near = |&&(c, d): &&(Point3, Point3)| {
                c.x.min(d.x) <= hi.x + m
                    && c.x.max(d.x) >= lo.x - m
                    && c.y.min(d.y) <= hi.y + m
                    && c.y.max(d.y) >= lo.y - m
                    && c.z.min(d.z) <= hi.z + m
                    && c.z.max(d.z) >= lo.z - m
            };
            for (c, d) in uncovered_parts(a, b, held.iter().filter(near).copied()) {
                let parts = if objects.is_empty() {
                    vec![(c, d)]
                } else {
                    parts_off_faces(self, plane, objects, boundary, c, d)
                };
                for (p, q) in parts {
                    out.push(Foreign {
                        a: p,
                        b: q,
                        home,
                        linked,
                        curve,
                    });
                }
            }
        }
        out
    }

    /// The sketch-only lift of detached sketch `sketch` (its own drafting
    /// context): no objects and no other sketch take part, so an edit
    /// arranges only with the sketch's own drawing and lowers back into it.
    ///
    /// # Errors
    /// [`DocumentError::UnknownSketch`] for a stale, deleted, object-owned,
    /// or non-detached sketch; [`DocumentError::Sketch`] if its drawing
    /// cannot be replayed (never for a valid document).
    pub(crate) fn lift_detached(&self, sketch: SketchId) -> Result<Lifted, DocumentError> {
        self.lift_detached_with(sketch, Gather::No)
    }

    /// [`Document::lift_detached`], with the foreign edges `gather` names:
    /// a draft on a detached sketch sees the other detached sketches' lines
    /// and objects' edges lying in its plane (wireframe faces §4.3; an
    /// object's edge only when both its ends are vertices of the shape,
    /// §16 R8), and nothing else.
    fn lift_detached_with(
        &self,
        sketch: SketchId,
        gather: Gather,
    ) -> Result<Lifted, DocumentError> {
        let sk = self
            .sketches
            .get(sketch)
            .filter(|sk| {
                sk.detached()
                    && !self.sketch_tombstoned(&sketch)
                    && !self.object_sketches.contains_key(&sketch)
            })
            .ok_or(DocumentError::UnknownSketch)?;
        let frame = self.def_sketches.get(&sketch).copied();
        let mut lifted =
            self.lift_plane_with(sk.plane(), frame, &[], Some(vec![sketch]), gather)?;
        lifted.detached = true;
        Ok(lifted)
    }

    /// The lift that CREATES a detached sketch on `plane` in `frame`:
    /// nothing is lifted but the foreign edges a detached shape sees (the
    /// other detached sketches' lines and objects' edges lying in the
    /// plane, wireframe faces §4.3; an object's edge once both its ends are
    /// vertices of the shape, which for a new shape is only after the edit,
    /// §16 R8), and everything the edit draws lowers into ONE new detached
    /// sketch.
    ///
    /// # Errors
    /// [`DocumentError::UnknownComponent`] for a stale or hidden `frame`.
    pub(crate) fn lift_detached_new(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
    ) -> Result<Lifted, DocumentError> {
        if let Some(c) = frame
            && self.components.get(c).is_none_or(|r| r.hidden)
        {
            return Err(DocumentError::UnknownComponent);
        }
        self.lift_detached_new_with(plane, frame, Gather::Detached, None)
    }

    /// [`Document::lift_detached_new`] gathering `gather`'s foreign edges:
    /// [`Gather::Detached`] for a detached draft, which never arranges with
    /// another detached shape on its own plane (separate shapes D3), and
    /// [`Gather::DetachedClose`] for the close pass, which sees them too
    /// (wireframe faces §16 R6).
    fn lift_detached_new_with(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        gather: Gather,
        own: Option<&[Point3]>,
    ) -> Result<Lifted, DocumentError> {
        if let Some(c) = frame
            && self.components.get(c).is_none_or(|r| r.hidden)
        {
            return Err(DocumentError::UnknownComponent);
        }
        let mut lifted =
            self.lift_plane_with_own(plane, frame, &[], Some(Vec::new()), gather, own)?;
        lifted.detached = true;
        Ok(lifted)
    }

    /// Opens a drafting gesture on `plane` in `frame` over `objects`
    /// (unified drafting §4.3): lifts the plane's drawing into a working
    /// sketch that [`Document::draft_sketch_mut`] edits with any [`Sketch`]
    /// method; [`Document::end_draft`] lowers the edit back as one undo step
    /// labeled `label`. Exactly one draft may be open.
    ///
    /// # Errors
    /// [`DocumentError::DraftAlreadyOpen`]; anything [`Document::lift_plane`]
    /// refuses.
    pub fn begin_draft(
        &mut self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
        label: &str,
    ) -> Result<(), DocumentError> {
        // A stale draft (the document changed under it) is simply replaced.
        if self.current_draft().is_some() {
            return Err(DocumentError::DraftAlreadyOpen);
        }
        let lifted = self.lift_plane_for_draft(plane, frame, objects)?;
        self.open_draft(lifted, label);
        Ok(())
    }

    /// Opens a drafting gesture on detached sketch `sketch` in its own
    /// context (detached shapes D5): only that sketch is lifted — no
    /// object, no other drawing on its plane — so the edit (continuing a
    /// line, erasing, moving, offsetting, re-faceting) arranges with the
    /// sketch's own drawing alone, and [`Document::end_draft`] lowers it
    /// all back into the sketch (an emptied sketch is deleted) as one undo
    /// step labeled `label`.
    ///
    /// # Errors
    /// [`DocumentError::DraftAlreadyOpen`]; [`DocumentError::UnknownSketch`]
    /// for a stale, deleted, object-owned, or non-detached sketch.
    pub fn begin_draft_detached(
        &mut self,
        sketch: SketchId,
        label: &str,
    ) -> Result<(), DocumentError> {
        if self.current_draft().is_some() {
            return Err(DocumentError::DraftAlreadyOpen);
        }
        let lifted = self.lift_detached_with(sketch, Gather::Detached)?;
        self.open_draft(lifted, label);
        Ok(())
    }

    /// Opens a drafting gesture that draws a NEW detached shape on `plane`
    /// in `frame` (detached shapes D3): the working sketch starts empty, and
    /// everything the edit draws lowers into ONE new detached sketch, which
    /// [`DraftReport::sketches`] names (an edit that draws nothing records
    /// nothing). Nothing else on the plane takes part.
    ///
    /// # Errors
    /// [`DocumentError::DraftAlreadyOpen`]; [`DocumentError::UnknownComponent`]
    /// for a stale or hidden `frame`.
    pub fn begin_draft_detached_new(
        &mut self,
        plane: Plane,
        frame: Option<ComponentId>,
        label: &str,
    ) -> Result<(), DocumentError> {
        if self.current_draft().is_some() {
            return Err(DocumentError::DraftAlreadyOpen);
        }
        let lifted = self.lift_detached_new(plane, frame)?;
        self.open_draft(lifted, label);
        Ok(())
    }

    /// **A vertex drag** in the open draft (the app's Move vertex): the
    /// draft vertex at `at` moves to `to`, arranging
    /// ([`Sketch::move_vertex_arranging`]). A drag moves its plane's drawing
    /// and never another plane's line (wireframe faces §4.1, §13), so while
    /// nothing is drawn in the draft yet it is first lifted again without
    /// foreign edges: a corner that a line of another plane ends on, or
    /// passes through, moves alone, as it did before foreign edges (with
    /// them lifted, the line would follow the corner and the draft would
    /// refuse with [`DocumentError::DraftBoundaryEdgeTouched`]). The close
    /// pass still runs when the drag lowers.
    ///
    /// Returns `None` when no open draft has a vertex at `at`, and the
    /// move's refusal as `Some(Err)`; either way the draft is left exactly
    /// as it was, its lift included.
    pub fn draft_drag_vertex(
        &mut self,
        at: Point3,
        to: Point3,
    ) -> Option<Result<crate::sketch::VertexMoved, SketchError>> {
        self.draft_vertex_at(at)?;
        let saved = self.pending_draft.clone();
        self.draft_drop_foreign_edges();
        let moved = self.draft_vertex_at(at).and_then(|v| {
            let work = self.draft_sketch_mut()?;
            Some(work.move_vertex_arranging(v, to))
        });
        if matches!(moved, Some(Ok(_))) {
            // A drag never closes against a line it does not move, on its
            // own plane (§13): a detached draft is not completed by the
            // object edges its moved corner now ends on either.
            if let Some(d) = self.pending_draft.as_deref_mut() {
                d.lifted.gather = Gather::No;
            }
        } else {
            self.pending_draft = saved;
        }
        moved
    }

    /// Lifts the open draft again without foreign edges, while nothing is
    /// drawn in it yet ([`Document::draft_drag_vertex`]). Returns whether
    /// the lift changed: `false` with no open draft, no foreign edge in its
    /// lift, or an edit already made.
    fn draft_drop_foreign_edges(&mut self) -> bool {
        let Some(d) = self.current_draft() else {
            return false;
        };
        let l = &d.lifted;
        if l.foreign.is_empty() || !same_drawing(&d.work, &l.before) {
            return false;
        }
        let free = l.detached.then(|| l.free.clone());
        let Ok(mut lifted) = self.lift_plane_with(l.plane, l.frame, &l.objects, free, Gather::No)
        else {
            return false;
        };
        lifted.detached = l.detached;
        let Some(d) = self.pending_draft.as_deref_mut() else {
            return false;
        };
        d.work = lifted.before.clone();
        d.lifted = lifted;
        true
    }

    /// Makes `lifted` the open draft, its working sketch a copy of the lift.
    fn open_draft(&mut self, lifted: Lifted, label: &str) {
        let work = lifted.before.clone();
        let generation = self.history_generation();
        self.pending_draft = Some(Box::new(PendingDraft {
            lifted,
            work,
            label: label.to_string(),
            generation,
        }));
    }

    /// The open draft, unless the document changed under it (a recorded
    /// change, an undo, or a redo since it opened).
    fn current_draft(&self) -> Option<&PendingDraft> {
        self.pending_draft
            .as_deref()
            .filter(|d| d.generation == self.history_generation())
    }

    /// The open draft's working sketch, or `None` when no draft is open or
    /// the document changed since it opened.
    pub fn draft_sketch_mut(&mut self) -> Option<&mut Sketch> {
        let generation = self.history_generation();
        self.pending_draft
            .as_deref_mut()
            .filter(|d| d.generation == generation)
            .map(|d| &mut d.work)
    }

    /// The open draft's working sketch (read-only); `None` as for
    /// [`Document::draft_sketch_mut`].
    pub fn draft_sketch(&self) -> Option<&Sketch> {
        self.current_draft().map(|d| &d.work)
    }

    /// Abandons the open draft; nothing changes. `false` when none was open.
    pub fn cancel_draft(&mut self) -> bool {
        self.pending_draft.take().is_some()
    }

    /// Lowers the open draft (see the module docs) as ONE undo step and
    /// closes it. An unchanged draft records nothing.
    ///
    /// # Errors
    /// All leave the document untouched and the draft open, except a stale
    /// draft, which closes:
    /// - [`DocumentError::DraftNotOpen`] — none is open, or the document
    ///   changed since it opened (its lift no longer describes the plane).
    /// - [`DocumentError::DraftBoundaryEdgeTouched`] — the edit moved or
    ///   removed a solid's own edge (topology editing).
    /// - [`DocumentError::Op`] / [`DocumentError::Sketch`] — a store could
    ///   not take its part (a kernel refusal no demotion could avoid).
    pub fn end_draft(&mut self) -> Result<(DraftReport, DocChange), DocumentError> {
        let Some(pending) = self.pending_draft.take() else {
            return Err(DocumentError::DraftNotOpen);
        };
        if pending.generation != self.history_generation() {
            return Err(DocumentError::DraftNotOpen);
        }
        match self.lower_draft(&pending) {
            Ok(done) => Ok(done),
            Err(e) => {
                self.pending_draft = Some(pending);
                Err(e)
            }
        }
    }

    /// Runs one drafting gesture: [`Document::begin_draft`], `edit` on the
    /// working sketch, [`Document::end_draft`]. A refusal from `edit` or the
    /// lowering cancels the draft and leaves the document untouched.
    pub fn draft<F>(
        &mut self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
        label: &str,
        edit: F,
    ) -> Result<(DraftReport, DocChange), DocumentError>
    where
        F: FnOnce(&mut Sketch) -> Result<(), SketchError>,
    {
        self.begin_draft(plane, frame, objects, label)?;
        self.run_open_draft(edit)
    }

    /// Runs one drafting gesture on detached sketch `sketch` in its own
    /// context: [`Document::begin_draft_detached`], `edit` on the working
    /// sketch, [`Document::end_draft`]. A refusal from `edit` or the
    /// lowering cancels the draft and leaves the document untouched.
    pub fn draft_detached<F>(
        &mut self,
        sketch: SketchId,
        label: &str,
        edit: F,
    ) -> Result<(DraftReport, DocChange), DocumentError>
    where
        F: FnOnce(&mut Sketch) -> Result<(), SketchError>,
    {
        self.begin_draft_detached(sketch, label)?;
        self.run_open_draft(edit)
    }

    /// Runs one drafting gesture that draws a new detached shape on `plane`
    /// in `frame`: [`Document::begin_draft_detached_new`], `edit`,
    /// [`Document::end_draft`]. The report's `sketches` names the created
    /// sketch. A refusal cancels the draft and leaves the document
    /// untouched.
    pub fn draft_detached_new<F>(
        &mut self,
        plane: Plane,
        frame: Option<ComponentId>,
        label: &str,
        edit: F,
    ) -> Result<(DraftReport, DocChange), DocumentError>
    where
        F: FnOnce(&mut Sketch) -> Result<(), SketchError>,
    {
        self.begin_draft_detached_new(plane, frame, label)?;
        self.run_open_draft(edit)
    }

    /// `edit` on the draft just opened, then [`Document::end_draft`]; any
    /// refusal cancels the draft.
    fn run_open_draft<F>(&mut self, edit: F) -> Result<(DraftReport, DocChange), DocumentError>
    where
        F: FnOnce(&mut Sketch) -> Result<(), SketchError>,
    {
        let edited = edit(self.draft_sketch_mut().expect("just opened"));
        if let Err(e) = edited {
            self.cancel_draft();
            return Err(DocumentError::Sketch(e));
        }
        let lowered = self.end_draft();
        if lowered.is_err() {
            self.cancel_draft();
        }
        lowered
    }

    /// Lowers the open draft as ONE undo step under its label, then runs
    /// the close pass ([`Document::close_faces_through`]) over the free
    /// edges the edit added, inside the same step.
    fn lower_draft(
        &mut self,
        pending: &PendingDraft,
    ) -> Result<(DraftReport, DocChange), DocumentError> {
        let completed = self.detached_shape_completed(&pending.lifted, &pending.work);
        let (lifted, work) = match &completed {
            Some((l, w)) => (l, w),
            None => (&pending.lifted, &pending.work),
        };
        let Lowered {
            mut report,
            actions,
            seeds,
            ..
        } = self.lower_lifted(lifted, work, &BTreeSet::new())?;
        if actions.is_empty() {
            return Ok((report, DocChange::default()));
        }
        let frame = pending.lifted.frame;
        self.undo.push(DocAction::Compound {
            actions,
            meta: Some(CompoundMeta {
                label: pending.label.clone(),
                origin: HistoryOrigin::User,
            }),
        });
        self.commit_new_action();
        // A detached draft that drew (not a vertex drag, §13) closes on its
        // own plane too (§16 R24).
        let detached_home = lifted.detached && lifted.gather == Gather::Detached;
        let pass = self.close_faces_followups_with(&seeds, frame, false, detached_home);
        self.debug_validate();

        let mut sketches_touched = report.sketches.clone();
        for s in pass.sketches_touched() {
            if !sketches_touched.contains(&s) {
                sketches_touched.push(s);
            }
        }
        let mut objects_touched = report.objects.clone();
        for &o in pass.objects_touched() {
            if !objects_touched.contains(&o) {
                objects_touched.push(o);
            }
        }
        report.faces_closed = pass.closed;
        report.cells = pass.cells.outcomes;
        let (components_touched, instances_touched) = match frame {
            Some(c) => (vec![c], self.placing_instances(c)),
            None => (Vec::new(), Vec::new()),
        };
        let change = DocChange {
            objects_touched,
            sketches_touched,
            components_touched,
            instances_touched,
            ..Default::default()
        };
        Ok((report, change))
    }

    /// A detached draft's lift completed by its own edit (wireframe faces
    /// §16 R8, R23, R29): the detached shape is only known once the edit
    /// has drawn it.
    ///
    /// - Another detached sketch's line the lift gathered (one lying in the
    ///   plane from another plane) that the drawn shape does not reach
    ///   through shared ends is not part of it: the lift is taken again
    ///   without such lines, and the drawing replayed onto it, so a line
    ///   that merely crosses the shape neither splits nor joins it; a
    ///   drawn line it had split is whole again (§16 R29).
    /// - Every object edge lying in the plane whose two ends are now
    ///   vertices of the shape, and which `work` does not already hold, is
    ///   added to the lift as foreign and to `work` as drawn there, exactly
    ///   as if the lift had gathered it — a gable's rafters drawn in one
    ///   gesture end on the box's top corners only after the gesture; a part
    ///   the sticky rules refuse is not offered (§16 R8).
    ///
    /// Every foreign edge of the lift returned is marked `linked`, the one
    /// set take may move lines from (§16 R32).
    ///
    /// `None` for an ordinary draft, a detached one lifted without foreign
    /// edges (a vertex drag, [`Document::draft_drag_vertex`]), or one that
    /// draws nothing.
    fn detached_shape_completed(&self, lifted: &Lifted, work: &Sketch) -> Option<(Lifted, Sketch)> {
        if !lifted.detached
            || lifted.gather != Gather::Detached
            || same_drawing(work, &lifted.before)
        {
            return None;
        }
        let (_, mut shape) = drawn_shape(work, &lifted.fixed);
        let at =
            |shape: &[Point3], p: Point3| shape.iter().any(|&q| q.approx_eq(p, tol::POINT_MERGE));
        let mut reached = vec![false; lifted.foreign.len()];
        loop {
            let mut grew = false;
            for (k, f) in lifted.foreign.iter().enumerate() {
                if reached[k] || f.home.is_none() || !(at(&shape, f.a) || at(&shape, f.b)) {
                    continue;
                }
                reached[k] = true;
                grew = true;
                for p in [f.a, f.b] {
                    if !at(&shape, p) {
                        shape.push(p);
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let mut lifted = lifted.clone();
        let mut work = work.clone();
        let stray: Vec<(Point3, Point3)> = lifted
            .foreign
            .iter()
            .zip(&reached)
            .filter(|&(f, &r)| f.home.is_some() && !r)
            .map(|(f, _)| (f.a, f.b))
            .collect();
        if !stray.is_empty()
            && let Some((l, w)) = self.relift_reached(&lifted, &work, &shape, &stray)
        {
            // Every detached line left in the lift is reached.
            lifted = l;
            work = w;
            reached = vec![true; lifted.foreign.len()];
        }
        for (f, &r) in lifted.foreign.iter_mut().zip(&reached) {
            f.linked = if f.home.is_some() {
                r
            } else {
                at(&shape, f.a) && at(&shape, f.b)
            };
        }
        let pos = |v: SketchVertexId| work.vertices()[v].position;
        let held: Vec<(Point3, Point3)> = work
            .edges()
            .values()
            .map(|e| (pos(e.from), pos(e.to)))
            .collect();
        let (_, own) = drawn_shape(&work, &lifted.fixed);
        let found: Vec<Foreign> = self
            .gather_foreign(
                &lifted.plane,
                lifted.frame,
                Gather::DetachedReached,
                &lifted.free,
                &[],
                &held,
                &[],
                Some(&own),
            )
            .into_iter()
            .filter(|f| f.home.is_none())
            .collect();
        for mut f in found {
            let mut trial = work.clone();
            if trial.add_segment(f.a, f.b).is_ok() {
                work = trial;
                f.linked = true;
                lifted.fixed.push((f.a, f.b));
                lifted.foreign.push(f);
            }
        }
        Some((lifted, work))
    }

    /// [`Document::detached_shape_completed`]'s re-lift without the gathered
    /// detached lines the drawn shape does not reach (`stray`, §16 R29): the
    /// lift taken again gathering only what `shape` reaches, and the edit's
    /// drawing (the working sketch's edges that lie along none of the old
    /// lift's fixed edges, curves kept) replayed onto a fresh working sketch
    /// with the kept foreign edges, a drawn line a stray line had split
    /// joined again. `None` when the lift or the replay refuses: the draft
    /// then lowers as lifted.
    fn relift_reached(
        &self,
        lifted: &Lifted,
        work: &Sketch,
        shape: &[Point3],
        stray: &[(Point3, Point3)],
    ) -> Option<(Lifted, Sketch)> {
        let mut relift = self
            .lift_plane_with_own(
                lifted.plane,
                lifted.frame,
                &[],
                Some(lifted.free.clone()),
                Gather::DetachedReached,
                Some(shape),
            )
            .ok()?;
        relift.detached = true;
        relift.gather = Gather::Detached;
        let pos = |v: SketchVertexId| work.vertices()[v].position;
        let m = tol::POINT_MERGE;
        let is_stray = |(c, d): (Point3, Point3)| {
            stray.iter().any(|&(x, y)| {
                (c.approx_eq(x, m) && d.approx_eq(y, m)) || (c.approx_eq(y, m) && d.approx_eq(x, m))
            })
        };
        let on_fixed = |a: Point3, b: Point3| lifted.fixed.iter().any(|&(c, d)| along(a, b, c, d));
        let on_kept = |a: Point3, b: Point3| {
            lifted
                .fixed
                .iter()
                .any(|&(c, d)| !is_stray((c, d)) && along(a, b, c, d))
        };
        let on_stray = |a: Point3, b: Point3| stray.iter().any(|&(c, d)| along(a, b, c, d));
        // Where the drawing runs along a stray line, the working sketch
        // holds the line's pieces only: a piece is the drawing's when both
        // its ends are the drawing's own points — a point the edit made or
        // one a drawn edge meets — and neither is a mere crossing (a drawn
        // line running straight through the stray line).
        let before_pts: Vec<Point3> = lifted
            .before
            .vertices()
            .values()
            .map(|v| v.position)
            .collect();
        let drawing_point = |v: SketchVertexId| {
            let p = pos(v);
            let drawn_dirs: Vec<Vec3> = work
                .edges()
                .values()
                .filter(|e| e.from == v || e.to == v)
                .filter(|e| !on_fixed(pos(e.from), pos(e.to)))
                .filter_map(|e| {
                    let q = if e.from == v { pos(e.to) } else { pos(e.from) };
                    (q - p).normalized().ok()
                })
                .collect();
            let crossing = drawn_dirs.len() == 2
                && drawn_dirs[0].cross(drawn_dirs[1]).length() <= tol::NORMAL_DIRECTION
                && drawn_dirs[0].dot(drawn_dirs[1]) < 0.0;
            let made = !before_pts.iter().any(|&q| q.approx_eq(p, m));
            !crossing && (made || !drawn_dirs.is_empty())
        };
        let drawn: Vec<SketchEdgeId> = work
            .edges()
            .iter()
            .filter(|(_, e)| {
                let (a, b) = (pos(e.from), pos(e.to));
                if on_kept(a, b) {
                    false
                } else if on_stray(a, b) {
                    drawing_point(e.from) && drawing_point(e.to)
                } else {
                    !on_fixed(a, b)
                }
            })
            .map(|(id, _)| id)
            .collect();
        let mut fresh = relift.before.blank_like();
        replay_edges(work, &drawn, &mut fresh).ok()?;
        fresh.carry_faceless_from(work, &|p| p, &[]);
        let _ = add_foreign(&mut fresh, relift.foreign.clone());
        // A drawn line a stray line crossed was split there: whole again.
        loop {
            let joint = fresh.vertices().iter().find_map(|(v, x)| {
                let p = x.position;
                let on_stray = stray
                    .iter()
                    .any(|&(c, d)| point_near_segment(p, c, d, tol::POINT_MERGE));
                if !on_stray
                    || relift
                        .fixed
                        .iter()
                        .any(|&(c, d)| point_near_segment(p, c, d, tol::POINT_MERGE))
                {
                    return None;
                }
                let mut probe = fresh.clone();
                probe
                    .dissolve_straight_vertex(v)
                    .unwrap_or(false)
                    .then_some(probe)
            });
            match joint {
                Some(next) => fresh = next,
                None => break,
            }
        }
        fresh.set_snap_to_existing(true);
        let seams = seams_of(&fresh, &relift.fixed);
        fresh.set_seams(seams);
        fresh.set_rigid(relift.foreign.iter().map(|f| (f.a, f.b)).collect());
        Some((relift, fresh))
    }

    /// Lowers `work`, an edit of `lifted`'s drawing (see the module docs),
    /// into the document, and returns what it recorded without recording
    /// it: the caller pushes the actions as one undo step, or appends them
    /// to one. `force` names regions of `work` — which must then be the lift
    /// itself, unedited — whose fixed ring edges are materialized whatever
    /// they lie over and whatever their rings hold: how the close pass
    /// writes a face nobody drew on its plane (wireframe faces §4.2).
    ///
    /// A materialized foreign edge from a free sketch in which it bounds no
    /// region is TAKEN: removed there in the same step, so a loose line that
    /// becomes a face's edge moves to the face's sketch (wireframe faces
    /// §4.1). An edge bounding a region at home is copied, never taken.
    ///
    /// # Errors
    /// As [`Document::end_draft`]; all leave the document untouched.
    fn lower_lifted(
        &mut self,
        lifted: &Lifted,
        work: &Sketch,
        force: &BTreeSet<SketchRegionId>,
    ) -> Result<Lowered, DocumentError> {
        let Lifted {
            plane,
            frame,
            objects,
            owned,
            free,
            fixed,
            foreign,
            stored,
            before,
            detached,
            gather,
        } = lifted;
        // I11: a detached shape's own context lifts no object, no face's
        // edge and no sketch on its plane but its own: every fixed edge is
        // foreign (another detached sketch's line, or an object's edge
        // ending on the shape's vertices, §16 R8), so the topology plan is
        // a no-op and every edge lowers as free drawing into its one sketch.
        debug_assert!(
            !*detached || (objects.is_empty() && owned.is_empty() && fixed.len() == foreign.len()),
            "a detached lower lifted objects or solid edges (I11) — kernel bug"
        );
        // A new point within the merge tolerance of an edge that was there
        // before (a solid's own edge, or drawing already on the plane) is ON
        // that edge: it moves onto the edge's line, so the edge splits
        // straight, the face's area is conserved, and undo restores the edge
        // exactly (the sketch merged it as a contact at the point itself, a
        // hair off the line).
        let carriers: Vec<(Point3, Point3)> = fixed
            .iter()
            .copied()
            .chain(before.edges().values().map(|e| {
                (
                    before.vertices()[e.from].position,
                    before.vertices()[e.to].position,
                )
            }))
            .collect();
        let healed = heal_seams(work, fixed);
        let base = healed.as_ref().unwrap_or(work);
        let snapped = snap_onto_fixed(base, before, &carriers);
        let after = snapped.as_ref().unwrap_or(base);
        let pos = |v: SketchVertexId| after.vertices()[v].position;

        // ---- gate: every fixed edge still drawn, end to end ----
        for &(a, b) in fixed {
            if !covered_by(after, a, b) {
                return Err(DocumentError::DraftBoundaryEdgeTouched);
            }
        }

        // ---- the diff ----
        let key_of = |sk: &Sketch, e: SketchEdgeId| {
            let ed = sk.edges()[e];
            seg_key(
                sk.vertices()[ed.from].position,
                sk.vertices()[ed.to].position,
            )
        };
        let before_keys: BTreeSet<SegKey> =
            before.edges().keys().map(|e| key_of(before, e)).collect();
        let after_keys: BTreeMap<SegKey, SketchEdgeId> = after
            .edges()
            .keys()
            .map(|e| (key_of(after, e), e))
            .collect();
        // (Nothing drawn or erased, and no loop given or robbed of its face
        // by drawing over an edge: nothing to lower.)
        if force.is_empty()
            && before_keys.len() == after_keys.len()
            && after_keys.keys().all(|k| before_keys.contains(k))
            && !face_state_changed(before, after)
        {
            return Ok(Lowered::default());
        }
        let removed: Vec<(Point3, Point3)> = before
            .edges()
            .keys()
            .filter(|&e| !after_keys.contains_key(&key_of(before, e)))
            .map(|e| {
                let ed = before.edges()[e];
                (
                    before.vertices()[ed.from].position,
                    before.vertices()[ed.to].position,
                )
            })
            .collect();
        let added: BTreeSet<SketchEdgeId> = after_keys
            .iter()
            .filter(|(k, _)| !before_keys.contains(*k))
            .map(|(_, &e)| e)
            .collect();

        // ---- islands of the drawing (fixed edges excluded) ----
        let is_fixed = |e: SketchEdgeId| {
            let ed = after.edges()[e];
            let (a, b) = (pos(ed.from), pos(ed.to));
            fixed.iter().any(|&(c, d)| along(a, b, c, d))
        };
        let drawing: Vec<SketchEdgeId> = after.edges().keys().filter(|&e| !is_fixed(e)).collect();
        let comp_of = components(after, &drawing);
        // A detached draft's other faces (§16 R24): the regions its
        // connected set now bounds that none of its drawn edges does.
        let completes: BTreeSet<SketchRegionId> = if *detached && *gather == Gather::Detached {
            self.detached_regions_completed(lifted, after)
        } else {
            BTreeSet::new()
        };

        // Regions on each side of every edge, from the traced rings.
        let mut sides: BTreeMap<SketchEdgeId, Vec<SketchRegionId>> = BTreeMap::new();
        for (rid, r) in after.regions().iter() {
            for ring in std::iter::once(&r.outer).chain(r.holes.iter()) {
                for i in 0..ring.len() {
                    let (u, v) = (ring[i], ring[(i + 1) % ring.len()]);
                    if let Some(e) = edge_between(after, u, v) {
                        sides.entry(e).or_default().push(rid);
                    }
                }
            }
        }

        // ---- affected islands ----
        let mut touched_regions: BTreeSet<SketchRegionId> = BTreeSet::new();
        for &e in &added {
            touched_regions.extend(sides.get(&e).into_iter().flatten().copied());
        }
        for &(a, b) in &removed {
            let m = midpoint(a, b);
            for (rid, _) in after.regions().iter() {
                if after.region_contains_point(rid, m).unwrap_or(false) {
                    touched_regions.insert(rid);
                }
            }
        }
        let touched_points: Vec<Point3> = removed.iter().flat_map(|&(a, b)| [a, b]).collect();
        let mut affected: BTreeSet<usize> = BTreeSet::new();
        for &e in &drawing {
            let ed = after.edges()[e];
            let hit = added.contains(&e)
                || sides
                    .get(&e)
                    .is_some_and(|rs| rs.iter().any(|r| touched_regions.contains(r)))
                || touched_points.iter().any(|&p| {
                    p.approx_eq(pos(ed.from), tol::POINT_MERGE)
                        || p.approx_eq(pos(ed.to), tol::POINT_MERGE)
                });
            if hit {
                affected.insert(comp_of[&e]);
            }
        }

        // ---- each drawing edge's target ----
        let current_store = |a: Point3, b: Point3| {
            stored
                .iter()
                .find(|s| along(a, b, s.a, s.b))
                .map(|s| s.store)
        };
        let cover = |m: Point3| {
            objects
                .iter()
                .copied()
                .find(|&o| face_holding(&self.objects[o].object, plane, m).is_some())
        };
        // What lies under each region, from a point strictly inside it — an
        // edge's own midpoint is no test: a drawing edge already in a face
        // lies exactly ON the boundary between two faces, where containment
        // is ambiguous. An edge's cover is its regions' (both sides of a
        // drawing edge lie over the same face or the same void).
        // Every object holding a face there, in participant order: two
        // coincident objects both cover a region.
        let covers = |m: Point3| -> Vec<ObjectId> {
            objects
                .iter()
                .copied()
                .filter(|&o| face_holding(&self.objects[o].object, plane, m).is_some())
                .collect()
        };
        let region_cover: BTreeMap<SketchRegionId, Vec<ObjectId>> = after
            .regions()
            .iter()
            .map(|(rid, r)| {
                let outer: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
                let holes: Vec<Vec<Point3>> = r
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|&v| pos(v)).collect())
                    .collect();
                let c = interior_point_of_loops(&outer, &holes, plane.normal())
                    .map(covers)
                    .unwrap_or_default();
                (rid, c)
            })
            .collect();
        // An edge already kept by one of the objects covering it stays with
        // that object; otherwise it goes to the first one covering it.
        let edge_cover = |e: SketchEdgeId, a: Point3, b: Point3, keeper: Option<ObjectId>| {
            let all: Vec<ObjectId> = match sides.get(&e) {
                Some(rs) if !rs.is_empty() => rs
                    .iter()
                    .find(|r| !region_cover[*r].is_empty())
                    .map(|r| region_cover[r].clone())
                    .unwrap_or_default(),
                _ => covers(midpoint(a, b)),
            };
            keeper
                .filter(|k| all.contains(k))
                .or_else(|| all.first().copied())
        };
        // The object already keeping each drawing island (its first edge kept
        // by an object): new drawing that touches an object's drawing joins
        // that object when it covers the new edge too (coincident solids).
        let keeper_of = |st: Store| match st {
            Store::Topo(o) => Some(o),
            Store::Sketch(s) => self.object_sketches.get(&s).copied(),
        };
        let mut island_keeper: BTreeMap<usize, ObjectId> = BTreeMap::new();
        for &e in &drawing {
            let ed = after.edges()[e];
            if let Some(o) = current_store(pos(ed.from), pos(ed.to)).and_then(keeper_of) {
                island_keeper.entry(comp_of[&e]).or_insert(o);
            }
        }
        let mut target: BTreeMap<SketchEdgeId, Target> = BTreeMap::new();
        for &e in &drawing {
            let ed = after.edges()[e];
            let (a, b) = (pos(ed.from), pos(ed.to));
            let current = current_store(a, b);
            let t = if !affected.contains(&comp_of[&e])
                && let Some(store) = current
            {
                match store {
                    Store::Topo(o) => Target::Topo(o),
                    Store::Sketch(s) => match self.object_sketches.get(&s) {
                        Some(&o) => Target::Owned(o),
                        None => Target::Free,
                    },
                }
            } else {
                let proper = sides
                    .get(&e)
                    .is_some_and(|rs| rs.len() == 2 && rs[0] != rs[1]);
                let keeper = current
                    .and_then(keeper_of)
                    .or_else(|| island_keeper.get(&comp_of[&e]).copied());
                match edge_cover(e, a, b, keeper) {
                    Some(o) if proper => Target::Topo(o),
                    Some(o) => Target::Owned(o),
                    None => Target::Free,
                }
            };
            target.insert(e, t);
        }

        // ---- topology, per object: minimal delta, rehearsed ----
        let mut report = DraftReport::default();
        let mut object_ops: Vec<(ObjectId, Vec<KernelOp>)> = Vec::new();
        for &o in objects {
            let topo_after: Vec<SketchEdgeId> = drawing
                .iter()
                .copied()
                .filter(|e| target[e] == Target::Topo(o))
                .collect();
            let stored_topo: Vec<(Point3, Point3)> = stored
                .iter()
                .filter(|s| s.store == Store::Topo(o))
                .map(|s| (s.a, s.b))
                .collect();
            // What the face already holds stays put unless it must go: the
            // edges along a stored topology edge are EXISTING, the rest NEW.
            let is_existing = |e: SketchEdgeId| {
                let ed = after.edges()[e];
                let (a, b) = (pos(ed.from), pos(ed.to));
                stored_topo.iter().any(|&(c, d)| along(a, b, c, d))
            };
            let existing: Vec<SketchEdgeId> = topo_after
                .iter()
                .copied()
                .filter(|&e| is_existing(e))
                .collect();
            let fresh: Vec<SketchEdgeId> = topo_after
                .iter()
                .copied()
                .filter(|&e| !is_existing(e))
                .collect();
            let obj = &self.objects[o].object;
            let planarity = obj.planarity_tol;
            let plan_with = |set: &[SketchEdgeId]| -> Result<Vec<KernelOp>, StickyError> {
                let wanted: Vec<(Point3, Point3, Claim)> = set
                    .iter()
                    .map(|&e| {
                        let ed = after.edges()[e];
                        let (a, b) = (pos(ed.from), pos(ed.to));
                        let g = ed.curve.and_then(|c| after.curve_geom(c));
                        // A regular polygon's sides carry its circumcircle
                        // as a polygon mark (pieces of a side included).
                        let poly = ed
                            .curve
                            .and_then(|c| after.curve_analytic(c))
                            .filter(|an| an.kind == SketchCurveKind::Polygon)
                            .map(|an| an.geom)
                            .filter(|g| g.polygon_side_count(a, b).is_some());
                        (a, b, (claim_if_on(g, a, b, planarity), poly))
                    })
                    .collect();
                plan_topology(obj, plane, &stored_topo, &wanted)
            };
            // Islands in a deterministic order (by their smallest edge key).
            let islands_of = |set: &[SketchEdgeId]| -> Vec<Vec<SketchEdgeId>> {
                let comp = components(after, set);
                let mut by: BTreeMap<usize, Vec<SketchEdgeId>> = BTreeMap::new();
                for &e in set {
                    by.entry(comp[&e]).or_default().push(e);
                }
                let mut v: Vec<Vec<SketchEdgeId>> = by.into_values().collect();
                v.sort_by_key(|isl| isl.iter().map(|&e| key_of(after, e)).min());
                v
            };
            // Greedy: keep each island the face can still hold.
            let greedy = |base: Vec<SketchEdgeId>,
                          candidates: Vec<Vec<SketchEdgeId>>,
                          kept: &mut Vec<(Point3, Point3)>|
             -> Result<(Vec<SketchEdgeId>, Vec<KernelOp>), StickyError> {
                let mut accepted = base;
                let mut ops = plan_with(&accepted)?;
                for island in candidates {
                    let mut trial = accepted.clone();
                    trial.extend(island.iter().copied());
                    match plan_with(&trial) {
                        Ok(found) => {
                            accepted = trial;
                            ops = found;
                        }
                        Err(_) => {
                            let ed = after.edges()[island[0]];
                            kept.push((pos(ed.from), pos(ed.to)));
                        }
                    }
                }
                Ok((accepted, ops))
            };
            let (accepted, ops) = match plan_with(&topo_after) {
                Ok(ops) => (topo_after.clone(), ops),
                Err(_) => {
                    // The face cannot hold all of it. Keep what it holds now
                    // and add the new islands it can; only when the existing
                    // drawing itself no longer fits (a deletion left a
                    // sliver) does an existing island leave the face.
                    match greedy(
                        existing.clone(),
                        islands_of(&fresh),
                        &mut report.kept_as_sketch,
                    ) {
                        Ok(done) => done,
                        Err(_) => greedy(
                            Vec::new(),
                            islands_of(&topo_after),
                            &mut report.kept_as_sketch,
                        )
                        .map_err(|e| DocumentError::Op(KernelOpError::Sticky(e)))?,
                    }
                }
            };
            for e in topo_after {
                if !accepted.contains(&e) {
                    target.insert(e, Target::Owned(o));
                }
            }
            if !ops.is_empty() {
                object_ops.push((o, ops));
            }
        }

        // ---- sketch stores ----
        // Each owned/free edge goes to the sketch it already lives in when
        // that sketch is a store of the same kind; otherwise to the target's
        // primary sketch (its first, or a new one).
        let mut desired: BTreeMap<Store, Vec<SketchEdgeId>> = BTreeMap::new();
        let mut materialized: Vec<SketchEdgeId> = Vec::new();
        let mut new_owned: BTreeMap<ObjectId, Vec<SketchEdgeId>> = BTreeMap::new();
        let mut new_free: Vec<SketchEdgeId> = Vec::new();
        let mut free_edges: Vec<SketchEdgeId> = Vec::new();
        for &e in &drawing {
            let ed = after.edges()[e];
            let (a, b) = (pos(ed.from), pos(ed.to));
            let here = current_store(a, b);
            match target[&e] {
                Target::Topo(_) => {}
                Target::Owned(o) => {
                    let keep = here.filter(|s| {
                        matches!(s, Store::Sketch(sid) if self.object_sketches.get(sid) == Some(&o))
                    });
                    match keep.or_else(|| {
                        owned
                            .get(&o)
                            .and_then(|v| v.first())
                            .map(|&s| Store::Sketch(s))
                    }) {
                        Some(s) => desired.entry(s).or_default().push(e),
                        None => new_owned.entry(o).or_default().push(e),
                    }
                }
                Target::Free => free_edges.push(e),
            }
        }
        // A drawn region over the void that a solid's edge closes keeps that
        // stretch of edge in its free sketch too: the region is then closed
        // in the sketch itself — it fills, extrudes into a new object flush
        // against the solid, and stays whole when the solid later moves.
        for (rid, r) in after.regions().iter() {
            let ring_edges: Vec<SketchEdgeId> = std::iter::once(&r.outer)
                .chain(r.holes.iter())
                .flat_map(|ring| {
                    (0..ring.len())
                        .filter_map(|i| edge_between(after, ring[i], ring[(i + 1) % ring.len()]))
                        .collect::<Vec<_>>()
                })
                .collect();
            // (A forced region is one the close pass already found over the
            // void: it is written whatever its ring holds; so is one a
            // detached draft completes, §16 R24.)
            let forced = force.contains(&rid) || completes.contains(&rid);
            let drawn_free = ring_edges
                .iter()
                .any(|e| target.get(e) == Some(&Target::Free));
            if !drawn_free && !forced {
                continue;
            }
            let outer: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
            let holes: Vec<Vec<Point3>> = r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| pos(v)).collect())
                .collect();
            let Some(sample) = interior_point_of_loops(&outer, &holes, plane.normal()) else {
                continue;
            };
            if !forced && cover(sample).is_some() {
                continue;
            }
            for e in ring_edges {
                if is_fixed(e) && !materialized.contains(&e) {
                    materialized.push(e);
                }
            }
        }
        // Free drawing that touches is kept in ONE free sketch, so every
        // region it closes is a region of that sketch: each connected group
        // (through shared points, materialized edges included) goes to the
        // free sketch already holding most of it, merging the others into
        // it; a group no free sketch holds yet goes to the plane's first.
        free_edges.extend(materialized.iter().copied());
        let mut root: BTreeMap<SketchVertexId, SketchVertexId> = BTreeMap::new();
        fn find(
            root: &mut BTreeMap<SketchVertexId, SketchVertexId>,
            v: SketchVertexId,
        ) -> SketchVertexId {
            let mut r = v;
            while let Some(&up) = root.get(&r) {
                if up == r {
                    break;
                }
                r = up;
            }
            root.insert(v, r);
            r
        }
        for &e in &free_edges {
            let ed = after.edges()[e];
            let (ra, rb) = (find(&mut root, ed.from), find(&mut root, ed.to));
            if ra != rb {
                root.insert(ra, rb);
            }
        }
        let mut groups: BTreeMap<SketchVertexId, Vec<SketchEdgeId>> = BTreeMap::new();
        for &e in &free_edges {
            let r = find(&mut root, after.edges()[e].from);
            groups.entry(r).or_default().push(e);
        }
        for edges in groups.into_values() {
            let mut tally: BTreeMap<usize, usize> = BTreeMap::new();
            for &e in &edges {
                let ed = after.edges()[e];
                if let Some(Store::Sketch(sid)) = current_store(pos(ed.from), pos(ed.to))
                    && let Some(i) = free.iter().position(|&f| f == sid)
                {
                    *tally.entry(i).or_default() += 1;
                }
            }
            // Most edges wins; the earlier sketch breaks a tie.
            let chosen = tally
                .iter()
                .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
                .map(|(&i, _)| free[i])
                .or_else(|| free.first().copied());
            match chosen {
                Some(sid) => desired.entry(Store::Sketch(sid)).or_default().extend(edges),
                None => new_free.extend(edges),
            }
        }
        // The close pass's seeds: the free edges this edit added, by the
        // free sketch they go to (`None`: the new one).
        let seg = |e: SketchEdgeId| {
            let ed = after.edges()[e];
            (pos(ed.from), pos(ed.to))
        };
        let added_free =
            |e: &&SketchEdgeId| added.contains(*e) && target.get(*e) == Some(&Target::Free);
        let mut seeds_at: Vec<(Option<SketchId>, Point3, Point3)> = Vec::new();
        for &s in free {
            for &e in desired
                .get(&Store::Sketch(s))
                .into_iter()
                .flatten()
                .filter(added_free)
            {
                let (a, b) = seg(e);
                seeds_at.push((Some(s), a, b));
            }
        }
        for &e in new_free.iter().filter(added_free) {
            let (a, b) = seg(e);
            seeds_at.push((None, a, b));
        }
        // ---- take (wireframe faces §4.1) ----
        // A materialized foreign edge whose free sketch holds it as a loose
        // line (it bounds no region there) leaves that sketch: the line
        // moves to the face's sketch. Only the parts materialized go; an
        // edge bounding a region at home is copied and stays.
        let mut taken: BTreeMap<SketchId, Vec<(Point3, Point3)>> = BTreeMap::new();
        for &e in &materialized {
            let (c, d) = seg(e);
            for f in foreign {
                // A detached lower takes only lines of its connected set
                // (§16 R32); a plane lower's take is between planes (§4.1).
                if let Some(h) = f.home
                    && (!*detached || f.linked)
                    && along(c, d, f.a, f.b)
                {
                    let parts = taken.entry(h).or_default();
                    if !parts.contains(&(c, d)) {
                        parts.push((c, d));
                    }
                }
            }
        }
        let mut take_rewrites: Vec<(SketchId, Sketch)> = Vec::new();
        for (h, parts) in taken {
            let cur = &self.sketches[h];
            // I10, with take: a plane lower takes only from ordinary free
            // sketches on other planes, a detached lower only from other
            // detached sketches — never from a sketch it lifted.
            debug_assert!(
                cur.detached() == *detached
                    && !free.contains(&h)
                    && !owned.values().flatten().any(|&s| s == h),
                "a {} lower would take from sketch {h:?} (detached: {}) (I10) — kernel bug",
                if *detached { "detached" } else { "plane" },
                cur.detached()
            );
            let vpos = |v: SketchVertexId| cur.vertices()[v].position;
            let bounding: BTreeSet<SegKey> = cur
                .regions()
                .values()
                .flat_map(|r| std::iter::once(&r.outer).chain(r.holes.iter()))
                .flat_map(|ring| {
                    (0..ring.len())
                        .map(|i| seg_key(vpos(ring[i]), vpos(ring[(i + 1) % ring.len()])))
                        .collect::<Vec<_>>()
                })
                .collect();
            let mut gone: BTreeSet<SketchEdgeId> = BTreeSet::new();
            let mut rest: Vec<(Point3, Point3, Option<crate::sketch::SketchCurveId>)> = Vec::new();
            for (eid, ed) in cur.edges().iter() {
                let (a, b) = (vpos(ed.from), vpos(ed.to));
                if bounding.contains(&seg_key(a, b)) {
                    continue;
                }
                let on: Vec<(Point3, Point3)> = parts
                    .iter()
                    .copied()
                    .filter(|&(c, d)| along(c, d, a, b))
                    .collect();
                if on.is_empty() {
                    continue;
                }
                gone.insert(eid);
                rest.extend(
                    uncovered_parts(a, b, on)
                        .into_iter()
                        .map(|(p, q)| (p, q, ed.curve)),
                );
            }
            if gone.is_empty() {
                continue;
            }
            // The home keeps what was not taken, curves and all (§16 R30).
            let mut left: Vec<(Point3, Point3, Option<crate::sketch::SketchCurveId>)> = cur
                .edges()
                .iter()
                .filter(|(eid, _)| !gone.contains(eid))
                .map(|(_, ed)| (vpos(ed.from), vpos(ed.to), ed.curve))
                .collect();
            left.extend(rest);
            let next = rebuild_by_runs(cur, &left).map_err(DocumentError::Sketch)?;
            take_rewrites.push((h, next));
        }

        let every_sketch: Vec<SketchId> = owned
            .values()
            .flatten()
            .copied()
            .chain(free.iter().copied())
            .collect();
        let mut rewrites: Vec<(SketchId, Sketch)> = Vec::new();
        for s in every_sketch {
            let edges = desired.remove(&Store::Sketch(s)).unwrap_or_default();
            let cur = &self.sketches[s];
            // I10: a plane lower never writes a detached sketch (the lift
            // filter keeps them out), and a detached lower writes only its
            // own.
            debug_assert_eq!(
                cur.detached(),
                *detached,
                "lower of a {} draft would write sketch {s:?} (detached: {}) (I10) — kernel bug",
                if *detached { "detached" } else { "plane" },
                cur.detached()
            );
            let cur_keys: BTreeSet<SegKey> = cur.edges().keys().map(|e| key_of(cur, e)).collect();
            let new_keys: BTreeSet<SegKey> = edges.iter().map(|&e| key_of(after, e)).collect();
            // (A loop given its face by drawing over an edge changes no edge,
            // but the sketch all the same.)
            if cur_keys != new_keys || face_state_changed(cur, after) {
                // The rebuild keeps the store's kind (its `detached` flag).
                let mut fresh = cur.blank_like();
                replay_edges(after, &edges, &mut fresh).map_err(DocumentError::Sketch)?;
                // A loop with no face stays one through the rebuild (sketch rule 6).
                fresh.carry_faceless_from(after, &|p| p, &[]);
                rewrites.push((s, fresh));
            }
        }
        let mut creations: Vec<(Option<ObjectId>, Sketch)> = Vec::new();
        for (o, edges) in new_owned {
            let mut fresh = Sketch::on_plane(*plane);
            replay_edges(after, &edges, &mut fresh).map_err(DocumentError::Sketch)?;
            // A loop with no face stays one through the rebuild (sketch rule 6).
            fresh.carry_faceless_from(after, &|p| p, &[]);
            creations.push((Some(o), fresh));
        }
        if !new_free.is_empty() {
            // One new free sketch per draft; in a detached shape's context
            // it is a new detached sketch.
            let mut fresh = if *detached {
                Sketch::detached_on_plane(*plane)
            } else {
                Sketch::on_plane(*plane)
            };
            replay_edges(after, &new_free, &mut fresh).map_err(DocumentError::Sketch)?;
            // A loop with no face stays one through the rebuild (sketch rule 6).
            fresh.carry_faceless_from(after, &|p| p, &[]);
            creations.push((None, fresh));
        }

        // ---- commit (nothing below can fail: every op was rehearsed) ----
        let mut actions: Vec<DocAction> = Vec::new();
        let mut on_plane: Vec<SketchId> = Vec::new();
        for (o, ops) in object_ops {
            for op in ops {
                let rec = &mut self.objects[o];
                rec.history
                    .apply(&mut rec.object, op)
                    .expect("a rehearsed re-trace replays identically");
                actions.push(match frame {
                    Some(component) => DocAction::DefObjectOp {
                        component: *component,
                        object: o,
                    },
                    None => DocAction::ObjectOp { object: o },
                });
            }
            report.objects.push(o);
        }
        for (s, fresh) in rewrites {
            let before_sk = std::mem::replace(&mut self.sketches[s], fresh);
            let emptied = self.sketches[s].edges().is_empty();
            actions.push(DocAction::SketchGesture {
                sketch: s,
                before: Box::new(before_sk),
                after: Box::new(self.sketches[s].clone()),
                created: false,
            });
            if emptied {
                self.hidden_sketches.insert(s);
                actions.push(DocAction::DeletedSketch { sketch: s });
            }
            report.sketches.push(s);
            on_plane.push(s);
        }
        let mut created_free: Option<SketchId> = None;
        for (owner, fresh) in creations {
            debug_assert!(
                !(fresh.detached() && owner.is_some()),
                "a detached sketch would be created object-owned (I9) — kernel bug"
            );
            let s = self.insert_sketch_record(fresh.blank_like());
            if let Some(c) = frame {
                self.def_sketches.insert(s, *c);
            }
            if let Some(o) = owner {
                self.object_sketches.insert(s, o);
            } else {
                created_free = Some(s);
            }
            actions.push(DocAction::SketchGesture {
                sketch: s,
                before: Box::new(self.sketches[s].clone()),
                after: Box::new(fresh.clone()),
                created: true,
            });
            self.sketches[s] = fresh;
            report.sketches.push(s);
            on_plane.push(s);
        }
        for (h, fresh) in take_rewrites {
            let before_sk = std::mem::replace(&mut self.sketches[h], fresh);
            let emptied = self.sketches[h].edges().is_empty();
            actions.push(DocAction::SketchGesture {
                sketch: h,
                before: Box::new(before_sk),
                after: Box::new(self.sketches[h].clone()),
                created: false,
            });
            if emptied {
                self.hidden_sketches.insert(h);
                actions.push(DocAction::DeletedSketch { sketch: h });
            }
            report.sketches.push(h);
        }
        let seeds = seeds_at
            .into_iter()
            .filter_map(|(s, a, b)| s.or(created_free).map(|s| (s, a, b)))
            .collect();
        Ok(Lowered {
            report,
            actions,
            seeds,
            on_plane,
        })
    }
}

/// What [`Document::lower_lifted`] wrote, not yet recorded.
#[derive(Debug, Default)]
struct Lowered {
    report: DraftReport,
    actions: Vec<DocAction>,
    /// The free edges the edit added, each with the free sketch it now
    /// lives in: the close pass's seeds.
    seeds: Vec<ClosingEdge>,
    /// The sketches on the lifted plane it wrote (the take targets, on
    /// other planes, are not among them).
    on_plane: Vec<SketchId>,
}

/// What the close pass ([`Document::close_faces_through`]) wrote.
#[derive(Debug, Default)]
pub(crate) struct ClosePass {
    /// Its undo rows, not yet recorded.
    pub(super) actions: Vec<DocAction>,
    /// The free sketches it closed a face in, each once.
    pub(crate) closed: Vec<SketchId>,
    /// Every sketch it wrote: those, and those it took a line from.
    pub(crate) touched: Vec<SketchId>,
    /// The cells search run after it ([`Document::close_cells`]), when
    /// the pass ran as an edit's followup.
    pub(super) cells: super::cells::CellsPass,
}

impl ClosePass {
    /// Every sketch the pass and the cells search wrote.
    pub(crate) fn sketches_touched(&self) -> Vec<SketchId> {
        let mut out = self.touched.clone();
        for &s in &self.cells.sketches {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// Every Object the cells search made, grew, or hid.
    pub(crate) fn objects_touched(&self) -> &[ObjectId] {
        &self.cells.objects
    }
}

/// One seed of the close pass, resolved.
struct Seed {
    a: Point3,
    b: Point3,
    /// The plane of the sketch holding it (H, §4.2): the edit's own plane,
    /// which the pass never looks at.
    home: Plane,
    detached: bool,
    /// The sketch holding it.
    sketch: SketchId,
}

/// A plane the close pass looks at, with the seeds lying in it.
struct Candidate {
    plane: Plane,
    detached: bool,
    seeds: Vec<usize>,
}

impl Document {
    /// **The close pass** (wireframe faces §4.2, rule 3: a loop on one
    /// plane is a face, whichever sketches hold its lines). For each free
    /// edge `e` of `edges` (an edge an edit added or moved, with the free
    /// sketch holding it, on plane H), in the world or definition `frame`:
    ///
    /// 1. **Candidate planes**: for each edge `f` of the frame — an edge of
    ///    a live free sketch of `e`'s kind (ordinary or detached), or of a
    ///    visible object — that meets an end of `e` and is not collinear
    ///    with it, the plane through `e` and `f`; de-duplicated ignoring
    ///    facing; H dropped.
    /// 2. Each candidate Q is lifted as a draft lifts it, foreign edges
    ///    included (`e` is one there), and its regions traced.
    /// 3. Every region whose ring runs along `e` and holds a foreign edge,
    ///    whose interior no object's face on Q (either facing) covers, and
    ///    which no free sketch of the kind on Q already holds as a faced
    ///    region, is written into Q's free sketch by the materialization
    ///    path, with take ([`DraftReport::faces_closed`]). For a detached
    ///    seed (§16 R24) its own plane H is a candidate as well, and every
    ///    region the seeds' connected set bounds is written, whether or not
    ///    `e` runs along it, unless a detached loop on Q already holds it
    ///    ([`Document::detached_regions_to_close`]).
    ///
    /// Q faces as a free sketch of the kind already on it does; failing
    /// that, for ordinary drawing, as an object's face on it does (so the
    /// object takes part, as when drawing there); failing that, the
    /// canonical way (`canonical_facing`). A detached edge closes faces
    /// only with other detached sketches' lines and objects' edges whose
    /// two ends are vertices of that detached drawing (§16 R8), and the
    /// face is a new detached sketch (§4.3); an ordinary edge never with a
    /// detached line. Only drawing and objects that take part are seeds,
    /// partners, facing or lifted (§16 R11: [`Document::sketch_takes_part`],
    /// [`Document::object_takes_part`]): a tag-hidden or closed group's
    /// line, or a view-hidden solid, closes nothing.
    ///
    /// Recorded as ONE undo step of its own; the drafting engine and the
    /// sketch moves and copies run the same pass inside their own step.
    /// Load, import, library insert, undo and redo never run it: a file
    /// reopens as it was saved. A plane whose lift or lowering refuses is
    /// skipped whole (no face there), never repaired. Returns the free
    /// sketches it closed a face in, each once, in the order written.
    pub fn close_faces_through(
        &mut self,
        edges: &[ClosingEdge],
        frame: Option<ComponentId>,
    ) -> Vec<SketchId> {
        let pass = self.close_faces_pass(edges, frame, false, false);
        if pass.actions.is_empty() {
            return Vec::new();
        }
        self.undo.push(DocAction::Compound {
            actions: pass.actions,
            meta: None,
        });
        self.commit_new_action();
        self.faces_closed_last = pass.closed.clone();
        self.debug_validate();
        pass.closed
    }

    /// The faces the close pass made in the most recent recorded edit that
    /// ran it (a draft, a sketch move or copy, a vertex drag,
    /// [`Document::close_faces_through`]), as [`DraftReport::faces_closed`]
    /// lists them; empty when that edit closed none, and after any other
    /// recorded edit. Undo and redo leave it as it was.
    pub fn faces_closed_by_last_edit(&self) -> &[SketchId] {
        &self.faces_closed_last
    }

    /// The report of the sketch edit just recorded — a move, rotate or
    /// scale of free drawing ([`Document::transform_sketch_islands`] and its
    /// arms, [`Document::transform_sketch`]), a copy
    /// ([`Document::copy_sketch_islands`]) or a vertex drag
    /// ([`Document::move_sketch_vertex`]) — in the shape a draft reports
    /// (wireframe faces §16 R20), from the [`DocChange`] that edit returned:
    ///
    /// - `objects`: none (a sketch edit changes no face topology; a solid it
    ///   made is in `cells`);
    /// - `sketches`: every sketch the step wrote, `change.sketches_touched`
    ///   in its order — the edited sketch first (for a detaching move, the
    ///   source then the new sketch; for a copy, the copy);
    /// - `kept_as_sketch`: none;
    /// - `faces_closed` and `cells`: what the close pass and the cells
    ///   search made in the step ([`Document::faces_closed_by_last_edit`],
    ///   [`Document::cells_by_last_edit`]).
    ///
    /// Read-only. Called after any other edit, `faces_closed` and `cells`
    /// are empty.
    pub fn sketch_edit_report(&self, change: &DocChange) -> DraftReport {
        DraftReport {
            objects: Vec::new(),
            sketches: change.sketches_touched.clone(),
            kept_as_sketch: Vec::new(),
            faces_closed: self.faces_closed_last.clone(),
            cells: self.cells_last.clone(),
        }
    }

    /// Runs the close pass over `seeds` after an edit was recorded, then
    /// the cells search (wireframe faces §5) seeded by the faces along
    /// those edges — the edit's own and the pass's — all its writes
    /// appended to that edit's undo step. `keep_home`: the seeds' own
    /// planes are candidates too ([`Document::close_faces_pass`]).
    pub(crate) fn close_faces_followups(
        &mut self,
        seeds: &[ClosingEdge],
        frame: Option<ComponentId>,
        keep_home: bool,
    ) -> ClosePass {
        self.close_faces_followups_with(seeds, frame, keep_home, false)
    }

    /// [`Document::close_faces_followups`], with `detached_home`: a
    /// detached seed's own plane is a candidate too
    /// ([`Document::close_faces_pass`]).
    fn close_faces_followups_with(
        &mut self,
        seeds: &[ClosingEdge],
        frame: Option<ComponentId>,
        keep_home: bool,
        detached_home: bool,
    ) -> ClosePass {
        let mut pass = self.close_faces_pass(seeds, frame, keep_home, detached_home);
        let rows = std::mem::take(&mut pass.actions);
        self.append_followups(rows);
        self.faces_closed_last = pass.closed.clone();
        let cell_seeds: Vec<super::cells::CellSeed> = seeds
            .iter()
            .map(|&(_, a, b)| super::cells::CellSeed::Edge(a, b))
            .collect();
        let scope = self.auto_cell_scope(frame);
        let mut cells = self.cells_pass(&cell_seeds, &scope, false);
        if !cells.actions.is_empty() {
            let rows = std::mem::take(&mut cells.actions);
            self.append_followups(rows);
            // The grown Objects' drawing, as after any recorded action.
            self.reconcile_owned_drawing();
        }
        self.cells_last = cells.outcomes.clone();
        pass.cells = cells;
        pass
    }

    /// Appends `rows` to the undo step on top (an edit just recorded), so
    /// they undo and redo with it.
    fn append_followups(&mut self, rows: Vec<DocAction>) {
        if !rows.is_empty() {
            let top = self
                .undo
                .actions
                .pop()
                .expect("followups run after an edit was recorded");
            let wrapped = match top {
                DocAction::Compound { mut actions, meta } => {
                    actions.extend(rows);
                    DocAction::Compound { actions, meta }
                }
                other => {
                    let mut actions = Vec::with_capacity(rows.len() + 1);
                    actions.push(other);
                    actions.extend(rows);
                    DocAction::Compound {
                        actions,
                        meta: None,
                    }
                }
            };
            // Straight onto the vector: the step was already counted.
            self.undo.actions.push(wrapped);
        }
    }

    /// The close-pass seeds of an edit of free sketch `sketch`: its edges
    /// that `prior` (its drawing before the edit) does not have, by exact
    /// position; every edge when `prior` is `None`.
    pub(crate) fn sketch_edges_new_since(
        &self,
        sketch: SketchId,
        prior: Option<&Sketch>,
    ) -> Vec<ClosingEdge> {
        let Some(sk) = self.sketches.get(sketch) else {
            return Vec::new();
        };
        let had: BTreeSet<SegKey> = prior
            .map(|p| {
                p.edges()
                    .values()
                    .map(|e| seg_key(p.vertices()[e.from].position, p.vertices()[e.to].position))
                    .collect()
            })
            .unwrap_or_default();
        sk.edges()
            .values()
            .map(|e| (sk.vertices()[e.from].position, sk.vertices()[e.to].position))
            .filter(|&(a, b)| !had.contains(&seg_key(a, b)))
            .map(|(a, b)| (sketch, a, b))
            .collect()
    }

    /// The close pass for a sketch edit just recorded (a move, rotate,
    /// scale, copy, or vertex drag of free sketch drawing): `seeds` in the
    /// sketch's frame, the writes appended to the edit's undo step, the
    /// cells search after it. Returns what the pass and the search wrote,
    /// for the edit's [`DocChange`].
    pub(crate) fn close_faces_after_sketch_edit(&mut self, seeds: &[ClosingEdge]) -> ClosePass {
        let Some(&(s, _, _)) = seeds.first() else {
            self.faces_closed_last.clear();
            self.cells_last.clear();
            return ClosePass::default();
        };
        let frame = self.def_sketches.get(&s).copied();
        self.close_faces_followups(seeds, frame, false)
    }

    /// The pass itself ([`Document::close_faces_through`]): writes, and
    /// returns the undo rows unrecorded.
    ///
    /// `keep_home`: the plane H of the sketch holding a seed is a
    /// candidate like any other, and edges lying in it are partners. A
    /// draft drops H because its own lift already arranged H with its
    /// foreign edges; an edit that lifted nothing (a raw sketch gesture,
    /// [`Document::end_sketch_gesture_closing_faces`]) keeps it, so a line
    /// drawn on the very plane of the face it closes still closes it.
    ///
    /// `detached_home`: a detached seed's H is a candidate too, whatever
    /// `keep_home` says — a detached draft that drew left the other
    /// detached shapes on H out of its lift (§16 R6), so lines of theirs
    /// that share its ends close faces only here (§16 R24). A vertex drag
    /// and a sketch move or copy pass `false`: a drag closes nothing on its
    /// own plane against a line it does not move (§13).
    pub(super) fn close_faces_pass(
        &mut self,
        seeds: &[ClosingEdge],
        frame: Option<ComponentId>,
        keep_home: bool,
        detached_home: bool,
    ) -> ClosePass {
        let mut out = ClosePass::default();
        let seeds: Vec<Seed> = seeds
            .iter()
            .filter_map(|&(s, a, b)| {
                let sk = self.sketches.get(s)?;
                let live = self.sketch_takes_part(s, frame);
                (live && !a.approx_eq(b, tol::POINT_MERGE)).then(|| Seed {
                    a,
                    b,
                    home: sk.plane(),
                    detached: sk.detached(),
                    sketch: s,
                })
            })
            .collect();
        if seeds.is_empty() {
            return out;
        }
        // ---- 1. candidate planes ----
        let mut linked_home: BTreeMap<SketchId, bool> = BTreeMap::new();
        let mut partners: [Option<Vec<(Point3, Point3)>>; 2] = [None, None];
        let mut cands: Vec<Candidate> = Vec::new();
        let m = tol::POINT_MERGE;
        for (i, sd) in seeds.iter().enumerate() {
            let set = partners[usize::from(sd.detached)]
                .get_or_insert_with(|| self.frame_edges(frame, sd.detached));
            let ab = sd.b - sd.a;
            let len = ab.length();
            let off_line = |q: Point3| ab.cross(q - sd.a).length() / len;
            // A detached draft's own plane is a candidate too (§16 R24),
            // when another detached shape there shares a corner with the
            // seed's: the draft's own lift arranged everything else on it,
            // so without one the plane adds nothing, and is not lifted
            // again (a plane of many separate shapes stays cheap to draw on).
            let home_too = keep_home
                || (detached_home
                    && sd.detached
                    && *linked_home
                        .entry(sd.sketch)
                        .or_insert_with(|| self.detached_shares_a_corner(sd.sketch, frame)));
            let in_home =
                |q: Point3| !home_too && sd.home.signed_distance(q).abs() <= tol::PLANE_DIST;
            for p in [sd.a, sd.b] {
                for &(c, d) in set.iter() {
                    if p.x < c.x.min(d.x) - m
                        || p.x > c.x.max(d.x) + m
                        || p.y < c.y.min(d.y) - m
                        || p.y > c.y.max(d.y) + m
                        || p.z < c.z.min(d.z) - m
                        || p.z > c.z.max(d.z) + m
                        || !point_near_segment(p, c, d, m)
                        || (in_home(c) && in_home(d))
                    {
                        continue;
                    }
                    let far = if off_line(c) >= off_line(d) { c } else { d };
                    if off_line(far) <= m {
                        continue; // collinear with `e`
                    }
                    let Ok(q) = Plane::from_point_normal(sd.a, ab.cross(far - sd.a)) else {
                        continue;
                    };
                    if !home_too && planes_coincide_either(&q, &sd.home) {
                        continue;
                    }
                    match cands
                        .iter_mut()
                        .find(|k| k.detached == sd.detached && planes_coincide_either(&k.plane, &q))
                    {
                        Some(k) => {
                            if !k.seeds.contains(&i) {
                                k.seeds.push(i);
                            }
                        }
                        None => cands.push(Candidate {
                            plane: q,
                            detached: sd.detached,
                            seeds: vec![i],
                        }),
                    }
                }
            }
        }
        // ---- 2 and 3, plane by plane: lift, find, write ----
        for cand in cands {
            let q = self.closing_plane(cand.plane, frame, cand.detached);
            let on = |p: Point3| q.signed_distance(p).abs() <= tol::PLANE_DIST;
            let through: Vec<(Point3, Point3)> = cand
                .seeds
                .iter()
                .map(|&i| (seeds[i].a, seeds[i].b))
                .filter(|&(a, b)| on(a) && on(b))
                .collect();
            if through.is_empty() {
                continue;
            }
            let lift = if cand.detached {
                // Every detached sketch lying in Q is gathered, those on Q
                // itself and the seeds' own included (§16 R6, R24). The
                // detached shape is the seeds' and what connects to them
                // through shared ends (§16 R23); only the regions it bounds
                // are written, and none a detached loop already holds, so a
                // separate shape on the plane is never arranged with.
                let own: Vec<Point3> = through.iter().flat_map(|&(a, b)| [a, b]).collect();
                self.lift_detached_new_with(q, frame, Gather::DetachedClose, Some(&own))
            } else {
                self.lift_plane_for_close(q, frame)
            };
            let Ok(lift) = lift else {
                continue;
            };
            let forced = if cand.detached {
                // The rule before §16 R24 (a ring along a seed, holding a
                // foreign edge) still holds off the seeds' own planes.
                let off_home: Vec<(Point3, Point3)> = cand
                    .seeds
                    .iter()
                    .filter(|&&i| !planes_coincide_either(&q, &seeds[i].home))
                    .map(|&i| (seeds[i].a, seeds[i].b))
                    .filter(|&(a, b)| on(a) && on(b))
                    .collect();
                self.detached_regions_to_close(&lift, &off_home)
            } else {
                self.regions_to_close(&lift, &through)
            };
            if forced.is_empty() {
                continue;
            }
            let Ok(lowered) = self.lower_lifted(&lift, &lift.before, &forced) else {
                continue;
            };
            out.actions.extend(lowered.actions);
            for s in lowered.on_plane {
                if !self.object_sketches.contains_key(&s) && !out.closed.contains(&s) {
                    out.closed.push(s);
                }
            }
            for s in lowered.report.sketches {
                if !out.touched.contains(&s) {
                    out.touched.push(s);
                }
            }
        }
        out
    }

    /// Every edge of `frame` a closing edge can share a plane with: the
    /// edges of its free sketches of the kind (`detached`) that take part,
    /// then of its objects that take part (§16 R11).
    fn frame_edges(&self, frame: Option<ComponentId>, detached: bool) -> Vec<(Point3, Point3)> {
        let mut out = Vec::new();
        for (s, sk) in self.sketches.iter() {
            if !self.sketch_takes_part(s, frame) || sk.detached() != detached {
                continue;
            }
            for e in sk.edges().values() {
                out.push((sk.vertices()[e.from].position, sk.vertices()[e.to].position));
            }
        }
        for (id, rec) in self.objects.iter() {
            if !self.object_takes_part(id, frame) {
                continue;
            }
            for eid in rec.object.edges().keys() {
                if let Some(ab) = rec.object.edge_endpoints(eid) {
                    out.push(ab);
                }
            }
        }
        out
    }

    /// `q` facing the way a face closed on it faces (see
    /// [`Document::close_faces_through`]).
    fn closing_plane(&self, q: Plane, frame: Option<ComponentId>, detached: bool) -> Plane {
        for (s, sk) in self.sketches.iter() {
            if self.sketch_takes_part(s, frame)
                && sk.detached() == detached
                && planes_coincide_either(&q, &sk.plane())
            {
                return sk.plane();
            }
        }
        if !detached {
            for o in self
                .drawable_objects_on_plane(q, frame)
                .into_iter()
                .chain(self.drawable_objects_on_plane(q.flipped(), frame))
                .filter(|&o| self.object_takes_part(o, frame))
            {
                let obj = &self.objects[o].object;
                if let Some(f) = obj
                    .faces()
                    .keys()
                    .find(|&f| face_on_plane_either(obj, f, &q))
                {
                    return obj.faces()[f].plane;
                }
            }
        }
        canonical_facing(q)
    }

    /// The regions of an ordinary candidate plane's `lift` the close pass
    /// writes (step 3 of [`Document::close_faces_through`]): a ring running
    /// along one of `through` and holding a foreign edge, over no object's
    /// face on the plane (either facing), and not already a faced region of
    /// an ordinary free sketch on the plane (either facing). A detached
    /// plane's are [`Document::detached_regions_to_close`].
    fn regions_to_close(
        &self,
        lift: &Lifted,
        through: &[(Point3, Point3)],
    ) -> BTreeSet<SketchRegionId> {
        let plane = lift.plane;
        let frame = lift.frame;
        let w = &lift.before;
        let pos = |v: SketchVertexId| w.vertices()[v].position;
        let solids: Vec<ObjectId> = self
            .drawable_objects_on_plane(plane, frame)
            .into_iter()
            .chain(self.drawable_objects_on_plane(plane.flipped(), frame))
            .filter(|&o| self.object_takes_part(o, frame))
            .collect();
        let mut faced: Vec<(Vec<Point3>, Vec<Vec<Point3>>)> = Vec::new();
        for (s, sk) in self.sketches.iter() {
            if !self.sketch_takes_part(s, frame)
                || sk.detached()
                || !planes_coincide_either(&plane, &sk.plane())
            {
                continue;
            }
            let at = |v: SketchVertexId| sk.vertices()[v].position;
            for r in sk.regions().values().filter(|r| !r.faceless) {
                faced.push((
                    r.outer.iter().map(|&v| at(v)).collect(),
                    r.holes
                        .iter()
                        .map(|h| h.iter().map(|&v| at(v)).collect())
                        .collect(),
                ));
            }
        }
        let mut out = BTreeSet::new();
        for (rid, r) in w.regions().iter() {
            if r.faceless {
                continue;
            }
            let outer: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
            let holes: Vec<Vec<Point3>> = r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| pos(v)).collect())
                .collect();
            let segs: Vec<(Point3, Point3)> = std::iter::once(&outer)
                .chain(holes.iter())
                .flat_map(|ring| {
                    (0..ring.len())
                        .map(|i| (ring[i], ring[(i + 1) % ring.len()]))
                        .collect::<Vec<_>>()
                })
                .collect();
            let runs_along_e = segs
                .iter()
                .any(|&(c, d)| through.iter().any(|&(a, b)| along(c, d, a, b)));
            let holds_foreign = segs
                .iter()
                .any(|&(c, d)| lift.foreign.iter().any(|f| along(c, d, f.a, f.b)));
            if !runs_along_e || !holds_foreign {
                continue;
            }
            let Some(sample) = interior_point_of_loops(&outer, &holes, plane.normal()) else {
                continue;
            };
            let covered = solids.iter().any(|&o| {
                let obj = &self.objects[o].object;
                face_holding(obj, &plane, sample).is_some()
                    || face_holding(obj, &plane.flipped(), sample).is_some()
            });
            if covered {
                continue;
            }
            let already = faced.iter().any(|(fo, fh)| {
                same_ring(fo, &outer)
                    && fh.len() == holes.len()
                    && fh.iter().all(|h| holes.iter().any(|x| same_ring(h, x)))
            });
            if !already {
                out.insert(rid);
            }
        }
        out
    }

    /// The close pass's regions of a detached candidate plane: every region
    /// of `lift` whose ring runs wholly along edges of the detached shape
    /// being closed (the seeds' connected set, §16 R23, the object edges
    /// ending on it included: the lift's linked foreign edges), whether or
    /// not a seed bounds it (§16 R24), and, as before R24, one whose ring
    /// runs along one of `through` (the seeds lying in the plane that are
    /// not on it: a seed's own plane takes only the connected set, §16 R6)
    /// and holds a foreign edge — in either case one that no loop of the
    /// set already holds ([`Document::detached_loop_holds`]). The lift holds
    /// the connected set only (§16 R29), and no Object face covers a
    /// detached shape (§16 R31).
    fn detached_regions_to_close(
        &self,
        lift: &Lifted,
        through: &[(Point3, Point3)],
    ) -> BTreeSet<SketchRegionId> {
        let plane = lift.plane;
        let frame = lift.frame;
        let w = &lift.before;
        let linked: Vec<(Point3, Point3)> = lift
            .foreign
            .iter()
            .filter(|f| f.linked)
            .map(|f| (f.a, f.b))
            .collect();
        // The set's vertices (§16 R28): the seeds' ends and every linked
        // edge's.
        let mut set: Vec<Point3> = Vec::new();
        for &(a, b) in through.iter().chain(linked.iter()) {
            for p in [a, b] {
                if !set.iter().any(|&q| q.approx_eq(p, tol::POINT_MERGE)) {
                    set.push(p);
                }
            }
        }
        let mut out = BTreeSet::new();
        for (rid, r) in w.regions().iter() {
            if r.faceless {
                continue;
            }
            let segs = region_segments(w, r);
            let of_set = segs
                .iter()
                .all(|&(c, d)| linked.iter().any(|&(a, b)| along(c, d, a, b)));
            let along_seed = segs
                .iter()
                .any(|&(c, d)| through.iter().any(|&(a, b)| along(c, d, a, b)))
                && segs
                    .iter()
                    .any(|&(c, d)| lift.foreign.iter().any(|f| along(c, d, f.a, f.b)));
            if !(of_set || along_seed) {
                continue;
            }
            let pos = |v: SketchVertexId| w.vertices()[v].position;
            let outer: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
            let holes: Vec<Vec<Point3>> = r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| pos(v)).collect())
                .collect();
            let Some(sample) = interior_point_of_loops(&outer, &holes, plane.normal()) else {
                continue;
            };
            // No Object face covers a detached shape: it is its own context
            // (§16 R31); a face over one is a loose face lying on it.
            if !self.detached_loop_holds(&plane, frame, sample, &set) {
                out.insert(rid);
            }
        }
        out
    }

    /// The regions of a detached draft's lowered drawing `after` that its
    /// connected set now bounds and none of its drawn edges does (§16 R24):
    /// each ring runs wholly along the lift's foreign edges that the drawing
    /// reaches through shared ends (another detached sketch's line, §16
    /// R23) or whose two ends are both vertices of that set (an object's
    /// edge, §16 R8), and no loop of a detached sketch on the plane already
    /// holds it ([`Document::detached_loop_holds`]). The draft writes them
    /// into its own sketch, as it writes the regions its own edges bound.
    fn detached_regions_completed(
        &self,
        lifted: &Lifted,
        after: &Sketch,
    ) -> BTreeSet<SketchRegionId> {
        let mut out = BTreeSet::new();
        if lifted.foreign.is_empty() {
            return out;
        }
        let (drawn, mut shape) = drawn_shape(after, &lifted.fixed);
        if drawn.is_empty() {
            return out;
        }
        let at =
            |shape: &[Point3], p: Point3| shape.iter().any(|&q| q.approx_eq(p, tol::POINT_MERGE));
        let mut linked = vec![false; lifted.foreign.len()];
        loop {
            let mut grew = false;
            for (k, f) in lifted.foreign.iter().enumerate() {
                if linked[k] || f.home.is_none() || !(at(&shape, f.a) || at(&shape, f.b)) {
                    continue;
                }
                linked[k] = true;
                grew = true;
                for p in [f.a, f.b] {
                    if !at(&shape, p) {
                        shape.push(p);
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let members: Vec<(Point3, Point3)> = lifted
            .foreign
            .iter()
            .zip(&linked)
            .filter(|&(f, &l)| l || (f.home.is_none() && at(&shape, f.a) && at(&shape, f.b)))
            .map(|(f, _)| (f.a, f.b))
            .collect();
        if members.is_empty() {
            return out;
        }
        let pos = |v: SketchVertexId| after.vertices()[v].position;
        for (rid, r) in after.regions().iter() {
            if r.faceless {
                continue;
            }
            // Only rings of foreign edges alone: a ring a drawn edge bounds
            // is the draft's already.
            let segs = region_segments(after, r);
            if !segs
                .iter()
                .all(|&(c, d)| members.iter().any(|&(a, b)| along(c, d, a, b)))
            {
                continue;
            }
            let outer: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
            let holes: Vec<Vec<Point3>> = r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| pos(v)).collect())
                .collect();
            let Some(sample) = interior_point_of_loops(&outer, &holes, lifted.plane.normal())
            else {
                continue;
            };
            if !self.detached_loop_holds(&lifted.plane, lifted.frame, sample, &shape) {
                out.insert(rid);
            }
        }
        out
    }

    /// Whether another detached sketch of `frame` that takes part, on the
    /// geometric plane of detached sketch `s` (either facing), has a vertex
    /// at a vertex of `s` (within [`tol::POINT_MERGE`]), or an object's
    /// edge lying in that plane ends, both ends, on vertices of `s`: whether
    /// the connected set of `s`'s drawing on its plane (§16 R23) reaches
    /// beyond `s` itself.
    fn detached_shares_a_corner(&self, s: SketchId, frame: Option<ComponentId>) -> bool {
        let Some(own) = self.sketches.get(s) else {
            return false;
        };
        let plane = own.plane();
        let pts: Vec<Point3> = own.vertices().values().map(|v| v.position).collect();
        let Some(first) = pts.first() else {
            return false;
        };
        let m = tol::POINT_MERGE;
        let (mut lo, mut hi) = (*first, *first);
        for q in &pts {
            lo = Point3::new(lo.x.min(q.x), lo.y.min(q.y), lo.z.min(q.z));
            hi = Point3::new(hi.x.max(q.x), hi.y.max(q.y), hi.z.max(q.z));
        }
        let near = |q: Point3| {
            q.x >= lo.x - m
                && q.x <= hi.x + m
                && q.y >= lo.y - m
                && q.y <= hi.y + m
                && q.z >= lo.z - m
                && q.z <= hi.z + m
        };
        let at = |q: Point3| near(q) && pts.iter().any(|&p| p.approx_eq(q, m));
        let shape = self.sketches.iter().any(|(o, sk)| {
            o != s
                && sk.detached()
                && planes_coincide_either(&plane, &sk.plane())
                && self.sketch_takes_part(o, frame)
                && sk.vertices().values().any(|v| at(v.position))
        });
        // An object's edge with both ends on `s` (§16 R8) closes against it
        // too: a file from before its draft completed such shapes.
        shape
            || self.objects.iter().any(|(id, rec)| {
                self.object_takes_part(id, frame)
                    && rec.object.edges().keys().any(|e| {
                        rec.object.edge_endpoints(e).is_some_and(|(a, b)| {
                            plane.signed_distance(a).abs() <= tol::PLANE_DIST
                                && plane.signed_distance(b).abs() <= tol::PLANE_DIST
                                && at(a)
                                && at(b)
                        })
                    })
            })
    }

    /// Whether a loop of a detached sketch of the connected set being
    /// closed holds point `p` — faced or not: a region such a loop already
    /// holds is not made again, and one whose face was erased is not given
    /// one back (§16 R24). The set's sketches are those of `frame` that take
    /// part, lie on `plane`'s geometric plane (either facing) and hold a
    /// vertex at one of the set's vertices `set` (§16 R28): two shapes that
    /// share a corner and overlap stay as drawn, while a loop the set does
    /// not reach is a separate shape and never blocks one (a small square
    /// inside a big ring, each its own lines, both fill, in either order).
    fn detached_loop_holds(
        &self,
        plane: &Plane,
        frame: Option<ComponentId>,
        p: Point3,
        set: &[Point3],
    ) -> bool {
        let of_set = |sk: &Sketch| {
            sk.vertices().values().any(|v| {
                set.iter()
                    .any(|&q| q.approx_eq(v.position, tol::POINT_MERGE))
            })
        };
        self.sketches.iter().any(|(s, sk)| {
            sk.detached()
                && planes_coincide_either(plane, &sk.plane())
                && self.sketch_takes_part(s, frame)
                && sk
                    .regions()
                    .keys()
                    .any(|r| sk.region_contains_point(r, p).unwrap_or(false))
                && of_set(sk)
        })
    }
}

/// Whether a loop of `cur` has a face in `after` and none in `cur`, or the
/// other way round: the working sketch carries `cur`'s loops by position
/// (sketch rule 6), so the same ring with another state is a change to
/// write back.
fn face_state_changed(cur: &Sketch, after: &Sketch) -> bool {
    let ring = |sk: &Sketch, r: &crate::sketch::SketchRegion| -> Vec<Point3> {
        r.outer.iter().map(|&v| sk.vertices()[v].position).collect()
    };
    cur.regions().values().any(|r| {
        let cur_ring = ring(cur, r);
        after
            .regions()
            .values()
            .any(|ar| ar.faceless != r.faceless && same_ring(&cur_ring, &ring(after, ar)))
    })
}

/// Whether working sketch `a` still holds exactly the drawing of `b` (the
/// lift it started from): the same edges, end to end, and the same face
/// states.
fn same_drawing(a: &Sketch, b: &Sketch) -> bool {
    let keys = |sk: &Sketch| -> BTreeSet<SegKey> {
        sk.edges()
            .values()
            .map(|e| seg_key(sk.vertices()[e.from].position, sk.vertices()[e.to].position))
            .collect()
    };
    a.vertices().len() == b.vertices().len()
        && a.edges().len() == b.edges().len()
        && keys(a) == keys(b)
        && !face_state_changed(b, a)
}

/// Replays every edge of `src` into `dst` through the sticky machinery, one
/// curve bracket per analytic chain.
fn replay_sketch_into(src: &Sketch, dst: &mut Sketch) -> Result<(), SketchError> {
    let edges: Vec<SketchEdgeId> = src.edges().keys().collect();
    replay_edges(src, &edges, dst)
}

/// Replays `edges` of `src` into `dst`, one curve bracket per source chain
/// (its analytic circle when it has one).
fn replay_edges(src: &Sketch, edges: &[SketchEdgeId], dst: &mut Sketch) -> Result<(), SketchError> {
    replay_edges_mapped(src, edges, dst, &|p| p)
}

/// [`replay_edges`] with every position passed through `map`; an edge whose
/// mapped ends coincide (within [`tol::POINT_MERGE`]) is dropped.
fn replay_edges_mapped(
    src: &Sketch,
    edges: &[SketchEdgeId],
    dst: &mut Sketch,
    map: &dyn Fn(Point3) -> Point3,
) -> Result<(), SketchError> {
    let pos = |v: SketchVertexId| map(src.vertices()[v].position);
    let collapsed = |a: Point3, b: Point3| a.approx_eq(b, tol::POINT_MERGE);
    let mut emitted: BTreeSet<crate::sketch::SketchCurveId> = BTreeSet::new();
    for &e in edges {
        let ed = src.edges()[e];
        match ed.curve {
            None => {
                if !collapsed(pos(ed.from), pos(ed.to)) {
                    dst.add_segment(pos(ed.from), pos(ed.to))?;
                }
            }
            Some(c) => {
                if !emitted.insert(c) {
                    continue;
                }
                match src.curve_analytic(c) {
                    Some(a) => {
                        dst.begin_curve_with_kind(a.geom, a.kind)?;
                    }
                    None => {
                        dst.begin_curve();
                    }
                }
                for &m in edges.iter().filter(|&&m| src.edges()[m].curve == Some(c)) {
                    let em = src.edges()[m];
                    if collapsed(pos(em.from), pos(em.to)) {
                        continue;
                    }
                    if let Err(err) = dst.add_segment(pos(em.from), pos(em.to)) {
                        dst.end_curve();
                        return Err(err);
                    }
                }
                dst.end_curve();
            }
        }
    }
    let _ = SketchCurveKind::Circle; // the kind travels inside `CurveAnalytic`
    Ok(())
}

/// The edge of `sk` between vertices `u` and `v`, either direction.
fn edge_between(sk: &Sketch, u: SketchVertexId, v: SketchVertexId) -> Option<SketchEdgeId> {
    sk.edges()
        .iter()
        .find(|(_, e)| (e.from == u && e.to == v) || (e.from == v && e.to == u))
        .map(|(id, _)| id)
}

/// Whether segment `a–b` is drawn end to end in `sk` (as one edge or a run
/// of collinear pieces along it).
fn covered_by(sk: &Sketch, a: Point3, b: Point3) -> bool {
    let d = b - a;
    let len2 = d.length_squared();
    let mut stops: Vec<(f64, SketchVertexId)> = sk
        .vertices()
        .iter()
        .filter(|(_, v)| point_near_segment(v.position, a, b, tol::POINT_MERGE))
        .map(|(id, v)| ((v.position - a).dot(d) / len2, id))
        .collect();
    stops.sort_by(|x, y| x.0.total_cmp(&y.0));
    if stops.len() < 2
        || !sk.vertices()[stops[0].1]
            .position
            .approx_eq(a, tol::POINT_MERGE)
        || !sk.vertices()[stops[stops.len() - 1].1]
            .position
            .approx_eq(b, tol::POINT_MERGE)
    {
        return false;
    }
    stops
        .windows(2)
        .all(|w| edge_between(sk, w[0].1, w[1].1).is_some())
}

/// Connected components of `edges` (by shared vertices): edge → component.
fn components(sk: &Sketch, edges: &[SketchEdgeId]) -> BTreeMap<SketchEdgeId, usize> {
    let mut parent: BTreeMap<SketchVertexId, SketchVertexId> = BTreeMap::new();
    fn find(p: &mut BTreeMap<SketchVertexId, SketchVertexId>, x: SketchVertexId) -> SketchVertexId {
        let mut r = x;
        while let Some(&q) = p.get(&r) {
            if q == r {
                break;
            }
            r = q;
        }
        p.insert(x, r);
        r
    }
    for &e in edges {
        let ed = sk.edges()[e];
        parent.entry(ed.from).or_insert(ed.from);
        parent.entry(ed.to).or_insert(ed.to);
        let (x, y) = (find(&mut parent, ed.from), find(&mut parent, ed.to));
        if x != y {
            parent.insert(x, y);
        }
    }
    let mut ids: BTreeMap<SketchVertexId, usize> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for &e in edges {
        let root = find(&mut parent, sk.edges()[e].from);
        let n = ids.len();
        let id = *ids.entry(root).or_insert(n);
        out.insert(e, id);
    }
    out
}

/// The re-trace ops that take `obj`'s face topology on `plane` from
/// `stored` (its current drawing edges there) to `wanted` (the edges it
/// should hold), by minimal delta, rehearsed on a copy: a stored edge no
/// wanted edge covers is removed; a wanted edge no stored or boundary edge
/// already holds is inserted. Inserted edges become paths between nodes
/// (anchors on the kept drawing or boundary, junctions where three or more
/// meet); a loop touching nothing is given as two paths. One re-trace per
/// cluster of faces joined by removed edges, seeded by a point inside it.
fn plan_topology(
    obj: &Object,
    plane: &Plane,
    stored: &[(Point3, Point3)],
    wanted: &[(Point3, Point3, Claim)],
) -> Result<Vec<KernelOp>, StickyError> {
    let removed: Vec<(Point3, Point3)> = stored
        .iter()
        .copied()
        .filter(|&(a, b)| !covered_by_set(wanted, a, b))
        .collect();
    let kept: Vec<(Point3, Point3)> = stored
        .iter()
        .copied()
        .filter(|s| !removed.contains(s))
        .collect();
    // Boundary + kept drawing, for anchoring; inserted = wanted edges not
    // already held as kept drawing or boundary.
    let mut boundary: Vec<(Point3, Point3)> = Vec::new();
    for f in obj.faces().keys().filter(|&f| face_on_plane(obj, f, plane)) {
        let face = &obj.faces()[f];
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            let pts: Vec<Point3> = obj.loop_positions(l).collect();
            for i in 0..pts.len() {
                boundary.push((pts[i], pts[(i + 1) % pts.len()]));
            }
        }
    }
    let inserted: Vec<(Point3, Point3, Claim)> = wanted
        .iter()
        .copied()
        .filter(|&(a, b, _)| {
            !boundary.iter().any(|&(c, d)| {
                along(a, b, c, d) && !removed.iter().any(|&(r, s)| along(a, b, r, s))
            })
        })
        .collect();
    if removed.is_empty() && inserted.is_empty() {
        return Ok(Vec::new());
    }
    let anchored = |p: Point3| {
        kept.iter()
            .chain(
                boundary
                    .iter()
                    .filter(|&&(c, d)| !removed.iter().any(|&(r, s)| along(c, d, r, s))),
            )
            .any(|&(c, d)| point_near_segment(p, c, d, tol::POINT_MERGE))
    };

    // Inserted edges as a graph over exact-bits vertices.
    let mut adj: BTreeMap<[u64; 3], Vec<usize>> = BTreeMap::new();
    for (i, &(a, b, _)) in inserted.iter().enumerate() {
        adj.entry(bits(a)).or_default().push(i);
        adj.entry(bits(b)).or_default().push(i);
    }
    let point_of: BTreeMap<[u64; 3], Point3> = inserted
        .iter()
        .flat_map(|&(a, b, _)| [(bits(a), a), (bits(b), b)])
        .collect();
    let is_node = |k: &[u64; 3]| adj[k].len() != 2 || anchored(point_of[k]);
    let mut used = vec![false; inserted.len()];
    let mut paths: Vec<RetracePath> = Vec::new();
    let walk = |start: [u64; 3], first: usize, used: &mut Vec<bool>| -> RetracePath {
        let mut path = vec![point_of[&start]];
        let mut curves = Vec::new();
        let mut polygons = Vec::new();
        let (mut at, mut e) = (start, first);
        loop {
            used[e] = true;
            let (a, b, g) = inserted[e];
            let next = if bits(a) == at { bits(b) } else { bits(a) };
            path.push(point_of[&next]);
            curves.push(g.0);
            polygons.push(g.1);
            at = next;
            if is_node(&at) {
                break;
            }
            match adj[&at].iter().copied().find(|&x| !used[x]) {
                Some(x) => e = x,
                None => break,
            }
        }
        RetracePath {
            path,
            curves,
            polygons,
        }
    };
    // A path closing on its own start is a loop touching the rest at that
    // one point (a circle tangent to an edge, two circles touching): it is
    // inserted as two paths meeting at a point midway round, so every path
    // runs between two distinct points.
    let push_path = |p: RetracePath, paths: &mut Vec<RetracePath>| {
        let n = p.path.len();
        if n >= 4 && bits(p.path[0]) == bits(p.path[n - 1]) {
            let mid = n / 2;
            paths.push(RetracePath {
                path: p.path[..=mid].to_vec(),
                curves: p.curves[..mid].to_vec(),
                polygons: p.polygons[..mid].to_vec(),
            });
            paths.push(RetracePath {
                path: p.path[mid..].to_vec(),
                curves: p.curves[mid..].to_vec(),
                polygons: p.polygons[mid..].to_vec(),
            });
        } else {
            paths.push(p);
        }
    };
    let nodes: Vec<[u64; 3]> = adj.keys().copied().filter(|k| is_node(k)).collect();
    for n in &nodes {
        for &e in &adj[n].clone() {
            if !used[e] {
                let p = walk(*n, e, &mut used);
                push_path(p, &mut paths);
            }
        }
    }
    // Loops touching nothing: two paths each, split at two of its points.
    while let Some(e0) = (0..inserted.len()).find(|&i| !used[i]) {
        let start = bits(inserted[e0].0);
        let p = walk(start, e0, &mut used);
        let n = p.path.len();
        if n < 4 {
            return Err(StickyError::PathNotSimple);
        }
        let mid = n / 2;
        paths.push(RetracePath {
            path: p.path[..=mid].to_vec(),
            curves: p.curves[..mid].to_vec(),
            polygons: p.polygons[..mid].to_vec(),
        });
        paths.push(RetracePath {
            path: p.path[mid..].to_vec(),
            curves: p.curves[mid..].to_vec(),
            polygons: p.polygons[mid..].to_vec(),
        });
    }

    // Clusters of faces joined by removed edges; each path's face.
    let mut work = obj.clone();
    let faces_on: Vec<FaceId> = work
        .faces()
        .keys()
        .filter(|&f| face_on_plane(&work, f, plane))
        .collect();
    let mut cluster: BTreeMap<FaceId, FaceId> = faces_on.iter().map(|&f| (f, f)).collect();
    fn root(c: &mut BTreeMap<FaceId, FaceId>, f: FaceId) -> FaceId {
        let mut r = f;
        while c[&r] != r {
            r = c[&r];
        }
        c.insert(f, r);
        r
    }
    for &(a, b) in &removed {
        let (e, _, _) = work.edge_matching(a, b).ok_or(StickyError::UnknownEdge)?;
        let ed = work.edges()[e];
        let fa = work.loops()[work.half_edges()[ed.half_edge].loop_id].face;
        let fb = ed
            .twin_half_edge
            .map(|t| work.loops()[work.half_edges()[t].loop_id].face)
            .ok_or(StickyError::BoundaryEdge)?;
        let (ra, rb) = (root(&mut cluster, fa), root(&mut cluster, fb));
        if ra != rb {
            cluster.insert(ra, rb);
        }
    }
    let mut by_cluster: BTreeMap<FaceId, (Vec<RetracePath>, Vec<Vec<Point3>>)> = BTreeMap::new();
    for p in paths {
        let s = midpoint(p.path[0], p.path[1]);
        let f = face_holding(&work, plane, s).ok_or(StickyError::PointNotOnFace { index: 0 })?;
        let r = root(&mut cluster, f);
        by_cluster.entry(r).or_default().0.push(p);
    }
    for &(a, b) in &removed {
        let (e, _, _) = work.edge_matching(a, b).ok_or(StickyError::UnknownEdge)?;
        let fa = work.loops()[work.half_edges()[work.edges()[e].half_edge].loop_id].face;
        let r = root(&mut cluster, fa);
        by_cluster.entry(r).or_default().1.push(vec![a, b]);
    }
    // A point inside each cluster's seed face, to find it again after other
    // clusters' re-traces re-mint faces.
    let seeds: Vec<(Point3, Vec<RetracePath>, Vec<Vec<Point3>>)> = by_cluster
        .into_iter()
        .map(|(f, (ins, rem))| {
            let face = &work.faces()[f];
            let outer: Vec<Point3> = work.loop_positions(face.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = face
                .inner_loops
                .iter()
                .map(|&l| work.loop_positions(l).collect())
                .collect();
            let p = interior_point_of_loops(&outer, &holes, plane.normal()).unwrap_or(outer[0]);
            (p, ins, rem)
        })
        .collect();
    let mut ops = Vec::new();
    for (p, insert, remove) in seeds {
        let face = face_holding(&work, plane, p).ok_or(StickyError::UnknownFace)?;
        work.retrace_face(face, &insert, &remove)?;
        ops.push(KernelOp::RetraceFace {
            face,
            insert,
            remove,
            restore: Vec::new(),
        });
    }
    Ok(ops)
}

/// Whether segment `a–b` is covered end to end by `set`'s segments lying
/// along it.
fn covered_by_set(set: &[(Point3, Point3, Claim)], a: Point3, b: Point3) -> bool {
    let d = b - a;
    let len2 = d.length_squared();
    let mut spans: Vec<(f64, f64)> = set
        .iter()
        .filter(|&&(c, e, _)| along(c, e, a, b))
        .map(|&(c, e, _)| {
            let (t0, t1) = ((c - a).dot(d) / len2, (e - a).dot(d) / len2);
            (t0.min(t1), t0.max(t1))
        })
        .collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let eps = tol::POINT_MERGE / len2.sqrt();
    let mut reach = 0.0;
    for (s, e) in spans {
        if s > reach + eps {
            return false;
        }
        reach = f64::max(reach, e);
    }
    reach >= 1.0 - eps
}

/// Where one drawing edge on a plane is stored ([`Document::plane_units`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStore {
    /// Face topology of an object.
    Topology(ObjectId),
    /// An object's owned sketch.
    Owned(ObjectId, SketchId),
    /// A free sketch.
    Free(SketchId),
}

/// One drawing edge on a plane.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftEdge {
    /// Geometry key ([`crate::unit_key::edge_key`]).
    pub key: u64,
    pub a: Point3,
    pub b: Point3,
    pub store: DraftStore,
    /// The circle this edge is a facet of, when it carries one.
    pub curve: Option<CurveGeom>,
    /// Its curve's key ([`crate::unit_key::chain_key`]), with `curve`.
    pub curve_key: Option<u64>,
}

/// One drawing unit: an island of drawing edges (a solid's own edges
/// excluded — they bound regions but belong to no unit).
#[derive(Debug, Clone, PartialEq)]
pub struct DraftUnit {
    /// The smallest edge key in the island.
    pub key: u64,
    pub edges: Vec<DraftEdge>,
    /// The island's curve runs, in key order.
    pub curves: Vec<DraftCurveRun>,
}

/// One run of a drawn circle, arc, or regular polygon: the facets of one
/// curve reachable from each other through points nothing else uses
/// ([`Sketch::curve_chain_at`]) — the piece a click selects. A circle
/// another line crosses is several runs; an uncrossed one is one run.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftCurveRun {
    /// The smallest edge key in the run.
    pub key: u64,
    /// The curve's key ([`crate::unit_key::curve_key`] of its circle).
    pub curve_key: u64,
    /// Whether the facets approximate the circle or are a polygon's sides.
    pub kind: SketchCurveKind,
    /// The circle (a polygon's circumcircle).
    pub geom: CurveGeom,
    /// The run's edge keys, ascending.
    pub edges: Vec<u64>,
}

/// Why a region over a solid is not one of its faces (so it does not push
/// or pull), for the message that says what to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftRegionWhy {
    /// Its area is too small for a face (under about a square micron).
    Sliver,
    /// Anything else the face cannot hold as drawn (part of its outline is
    /// drawing the solid does not keep as face structure).
    Other,
}

/// One region of a plane's drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftRegion {
    /// [`crate::unit_key::region_key`] of its outer ring (unique per plane).
    pub key: u64,
    pub outer: Vec<Point3>,
    pub holes: Vec<Vec<Point3>>,
    /// The object and face under it: a face region pushes and pulls as that
    /// face (`face` is `None` when the face cannot hold it — a sliver kept
    /// as owned-sketch drawing, which does not push).
    pub over: Option<(ObjectId, Option<FaceId>)>,
    /// With `over` naming no face: why the face cannot hold it.
    pub why: Option<DraftRegionWhy>,
    /// Over the void: the free sketch region it extrudes as.
    pub extrude: Option<(SketchId, crate::sketch::SketchRegionId)>,
}

/// A plane's drawing as units and regions ([`Document::plane_units`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaneUnits {
    pub units: Vec<DraftUnit>,
    pub regions: Vec<DraftRegion>,
}

/// What one entry of [`Document::free_plane_units`] lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaneListing {
    /// A plane's arrangement: its free drawing with the objects taking part.
    Plane(Plane),
    /// One detached sketch, in its own context
    /// ([`Document::detached_sketch_units`]).
    Detached { sketch: SketchId, plane: Plane },
}

impl PlaneListing {
    /// The plane the entry's drawing lies on.
    pub fn plane(&self) -> Plane {
        match *self {
            PlaneListing::Plane(plane) | PlaneListing::Detached { plane, .. } => plane,
        }
    }
}

impl Document {
    /// The drawing on `plane` in `frame` over `objects`, as the units a user
    /// selects (islands of drawing edges, each edge with its store and key)
    /// and the regions it encloses (with what each pushes or extrudes as).
    /// Read-only; computed from a lift. Keys are geometry-derived
    /// ([`crate::unit_key`]): stable until the geometry they hash changes.
    ///
    /// # Errors
    /// As [`Document::lift_plane`].
    pub fn plane_units(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
    ) -> Result<PlaneUnits, DocumentError> {
        let lifted = self.lift_plane(plane, frame, objects)?;
        self.plane_units_of(&lifted)
    }

    /// The drawing of a frame, plane by plane, in one pass: every plane a
    /// free sketch of the frame lies on (planes within tolerance as one),
    /// each listed as [`Document::plane_units`] lists it
    /// ([`PlaneListing::Plane`]), with the objects of the frame that have a
    /// face on the plane taking part, except the `excluded` ones (hidden on
    /// screen). Each plane is lifted with its own sketches, found by one
    /// grouping of all of them; asking plane by plane walks every sketch and
    /// every face of the frame for each plane, which on a document of tens
    /// of thousands of small sketches took minutes. The order is by plane,
    /// so the listing is deterministic. After the planes, every detached
    /// sketch of the frame is listed on its own
    /// ([`PlaneListing::Detached`], [`Document::detached_sketch_units`]), in
    /// [`SketchId`] order: it belongs to no plane's arrangement.
    pub fn free_plane_units(
        &self,
        frame: Option<ComponentId>,
        excluded: &BTreeSet<ObjectId>,
    ) -> Vec<(PlaneListing, PlaneUnits)> {
        // A plane's bucket: its normal and offset at a grain of a
        // millionth, a thousand times coarser than the tolerances two
        // coincident planes are allowed to differ by. Such planes land in
        // one bucket or in two adjacent ones, never further apart, so a
        // lookup probes the bucket and its neighbors in every direction
        // and `planes_coincide` decides: no plane is listed twice, and no
        // object with a face on a plane is missed at a bucket's edge.
        let key_of = |pl: &Plane| -> [i64; 4] {
            let n = pl.normal();
            let d = -pl.signed_distance(Point3::ORIGIN);
            let q = |x: f64| (x * 1e6).round() as i64;
            [q(n.x), q(n.y), q(n.z), q(d)]
        };
        let around = |k: [i64; 4]| -> Vec<[i64; 4]> {
            let mut out = Vec::with_capacity(81);
            for a in -1..=1 {
                for b in -1..=1 {
                    for c in -1..=1 {
                        for e in -1..=1 {
                            out.push([k[0] + a, k[1] + b, k[2] + c, k[3] + e]);
                        }
                    }
                }
            }
            out
        };
        // Free sketches by plane, in sketch order; detached ones apart.
        let mut groups: Vec<(Plane, Vec<SketchId>)> = Vec::new();
        let mut group_of: BTreeMap<[i64; 4], Vec<usize>> = BTreeMap::new();
        let mut detached: Vec<SketchId> = Vec::new();
        for (s, sk) in self.sketches.iter() {
            if self.sketch_tombstoned(&s)
                || self.object_sketches.contains_key(&s)
                || self.def_sketches.get(&s).copied() != frame
            {
                continue;
            }
            if sk.detached() {
                detached.push(s);
                continue;
            }
            let plane = sk.plane();
            let key = key_of(&plane);
            let found = around(key).into_iter().find_map(|k| {
                group_of
                    .get(&k)?
                    .iter()
                    .copied()
                    .find(|&g| planes_coincide(&groups[g].0, &plane))
            });
            match found {
                Some(g) => groups[g].1.push(s),
                None => {
                    group_of.entry(key).or_default().push(groups.len());
                    groups.push((plane, vec![s]));
                }
            }
        }
        // The frame's objects by the planes of their faces.
        let mut objects_by_plane: BTreeMap<[i64; 4], Vec<ObjectId>> = BTreeMap::new();
        for (id, rec) in self.objects.iter() {
            let drawable = !rec.hidden
                && self.object_owner_component(id) == frame
                && match rec.owner {
                    ObjectOwner::World { parent } => parent.is_none(),
                    ObjectOwner::Definition { .. } => true,
                }
                && !excluded.contains(&id);
            if !drawable {
                continue;
            }
            for face in rec.object.faces().values() {
                let on = objects_by_plane.entry(key_of(&face.plane)).or_default();
                if !on.contains(&id) {
                    on.push(id);
                }
            }
        }
        let mut out = Vec::with_capacity(groups.len());
        for (plane, sketches) in groups {
            let mut objects: Vec<ObjectId> = Vec::new();
            for k in around(key_of(&plane)) {
                for &o in objects_by_plane.get(&k).into_iter().flatten() {
                    if objects.contains(&o) {
                        continue;
                    }
                    let obj = &self.objects[o].object;
                    if obj.faces().keys().any(|f| face_on_plane(obj, f, &plane)) {
                        objects.push(o);
                    }
                }
            }
            objects.sort_unstable();
            let Ok(lifted) =
                self.lift_plane_with(plane, frame, &objects, Some(sketches), Gather::No)
            else {
                continue;
            };
            let Ok(units) = self.plane_units_of(&lifted) else {
                continue;
            };
            out.push((PlaneListing::Plane(plane), units));
        }
        detached.sort_unstable();
        for s in detached {
            let Ok(units) = self.detached_sketch_units(s) else {
                continue;
            };
            out.push((
                PlaneListing::Detached {
                    sketch: s,
                    plane: self.sketches[s].plane(),
                },
                units,
            ));
        }
        out
    }

    /// The drawing of detached sketch `sketch` in its own context, as
    /// [`Document::plane_units`] lists a plane's: its units, and its regions
    /// — every one over no face, extruding as the sketch's own region
    /// ([`DraftRegion::extrude`]), since no object takes part.
    ///
    /// # Errors
    /// As [`Document::begin_draft_detached`]: [`DocumentError::UnknownSketch`]
    /// for a stale, deleted, object-owned, or non-detached sketch.
    pub fn detached_sketch_units(&self, sketch: SketchId) -> Result<PlaneUnits, DocumentError> {
        self.plane_units_of(&self.lift_detached(sketch)?)
    }

    /// The live detached sketches on `plane` (within tolerance, same
    /// facing) in `frame`, in [`SketchId`] order — what a pick on a plane
    /// probes besides the plane's own arrangement, since a detached sketch
    /// is in none.
    pub fn detached_sketches_on(&self, plane: Plane, frame: Option<ComponentId>) -> Vec<SketchId> {
        let mut out: Vec<SketchId> = self
            .sketches
            .iter()
            .filter(|&(s, sk)| {
                sk.detached()
                    && !self.sketch_tombstoned(&s)
                    && !self.object_sketches.contains_key(&s)
                    && self.def_sketches.get(&s).copied() == frame
                    && planes_coincide(&plane, &sk.plane())
            })
            .map(|(s, _)| s)
            .collect();
        out.sort_unstable();
        out
    }

    /// [`Document::plane_units`] of a plane already lifted.
    fn plane_units_of(&self, lifted: &Lifted) -> Result<PlaneUnits, DocumentError> {
        let plane = lifted.plane;
        let w = &lifted.before;
        let pos = |v: SketchVertexId| w.vertices()[v].position;
        let fixed = |a: Point3, b: Point3| lifted.fixed.iter().any(|&(c, d)| along(a, b, c, d));
        let drawing: Vec<SketchEdgeId> = w
            .edges()
            .iter()
            .filter(|(_, e)| !fixed(pos(e.from), pos(e.to)))
            .map(|(id, _)| id)
            .collect();
        let comp = components(w, &drawing);
        let store_of = |a: Point3, b: Point3| {
            lifted
                .stored
                .iter()
                .find(|s| along(a, b, s.a, s.b))
                .map(|s| match s.store {
                    Store::Topo(o) => DraftStore::Topology(o),
                    Store::Sketch(sk) => match self.object_sketches.get(&sk) {
                        Some(&o) => DraftStore::Owned(o, sk),
                        None => DraftStore::Free(sk),
                    },
                })
        };
        let mut by_comp: BTreeMap<usize, Vec<DraftEdge>> = BTreeMap::new();
        for &e in &drawing {
            let ed = w.edges()[e];
            let (a, b) = (pos(ed.from), pos(ed.to));
            let Some(store) = store_of(a, b) else {
                continue;
            };
            by_comp.entry(comp[&e]).or_default().push(DraftEdge {
                key: crate::unit_key::edge_key(&plane, a, b),
                a,
                b,
                store,
                curve: ed.curve.and_then(|c| w.curve_geom(c)),
                curve_key: ed.curve.and_then(|c| crate::unit_key::chain_key(w, c)),
            });
        }
        // Curve runs, per island.
        let key_of = |e: SketchEdgeId| {
            let ed = w.edges()[e];
            crate::unit_key::edge_key(&plane, pos(ed.from), pos(ed.to))
        };
        let mut runs_by_comp: BTreeMap<usize, Vec<DraftCurveRun>> = BTreeMap::new();
        let mut in_run: BTreeSet<SketchEdgeId> = BTreeSet::new();
        for &e in &drawing {
            if in_run.contains(&e) {
                continue;
            }
            let Some(analytic) = w.edges()[e].curve.and_then(|c| w.curve_analytic(c)) else {
                continue;
            };
            let chain = w.curve_chain_at(e);
            in_run.extend(chain.iter().copied());
            let mut edges: Vec<u64> = chain.iter().map(|&c| key_of(c)).collect();
            edges.sort_unstable();
            runs_by_comp
                .entry(comp[&e])
                .or_default()
                .push(DraftCurveRun {
                    key: edges[0],
                    curve_key: w.edges()[e]
                        .curve
                        .and_then(|c| crate::unit_key::chain_key(w, c))
                        .unwrap_or_default(),
                    kind: analytic.kind,
                    geom: analytic.geom,
                    edges,
                });
        }
        let mut units: Vec<DraftUnit> = by_comp
            .into_iter()
            .map(|(c, mut edges)| {
                edges.sort_by_key(|e| e.key);
                let mut curves = runs_by_comp.remove(&c).unwrap_or_default();
                curves.sort_by_key(|r| r.key);
                DraftUnit {
                    key: edges.first().map_or(0, |e| e.key),
                    edges,
                    curves,
                }
            })
            .collect();
        units.sort_by_key(|u| u.key);

        let mut regions = Vec::new();
        for (_, r) in w.regions().iter() {
            let outer: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
            let holes: Vec<Vec<Point3>> = r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| pos(v)).collect())
                .collect();
            let key = crate::unit_key::region_key((0..outer.len()).map(|i| {
                crate::unit_key::edge_key(&plane, outer[i], outer[(i + 1) % outer.len()])
            }));
            let Some(sample) = interior_point_of_loops(&outer, &holes, plane.normal()) else {
                continue;
            };
            let holder = lifted.objects.iter().copied().find_map(|o| {
                face_holding(&self.objects[o].object, &plane, sample).map(|f| (o, f))
            });
            let (over, extrude) = match holder {
                Some((o, f)) => {
                    // A face IS this region when their rings trace the same
                    // shapes. A drafted vertex on a face edge (where a loose
                    // line ends on it) is a straight-through point that the
                    // face does not have, so rings compare by their corners.
                    let obj = &self.objects[o].object;
                    let face = &obj.faces()[f];
                    let ring: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
                    let face_holes: Vec<Vec<Point3>> = face
                        .inner_loops
                        .iter()
                        .map(|&l| obj.loop_positions(l).collect())
                        .collect();
                    let same = same_ring(&ring, &outer)
                        && face_holes.len() == holes.len()
                        && face_holes
                            .iter()
                            .all(|fh| holes.iter().any(|h| same_ring(fh, h)));
                    (Some((o, same.then_some(f))), None)
                }
                None => {
                    let found = lifted.free.iter().copied().find_map(|s| {
                        let sk = &self.sketches[s];
                        sk.regions()
                            .keys()
                            .find(|&rid| {
                                !sk.regions()[rid].faceless
                                    && sk.region_contains_point(rid, sample).unwrap_or(false)
                            })
                            .map(|rid| (s, rid))
                    });
                    (None, found)
                }
            };
            let why =
                matches!(over, Some((_, None))).then(|| region_why(&outer, &holes, plane.normal()));
            regions.push(DraftRegion {
                key,
                outer,
                holes,
                over,
                why,
                extrude,
            });
        }
        regions.sort_by_key(|r| r.key);
        Ok(PlaneUnits { units, regions })
    }

    /// The open draft's working-sketch edges whose geometry keys are in
    /// `keys` ([`DraftEdge::key`]), in edge order — how a caller holding
    /// keys from [`Document::plane_units`] names what to erase, move, or
    /// copy inside a draft. A solid's own boundary edges never match. See
    /// [`Document::draft_edges_for_keys`] for the keys that name nothing.
    pub fn draft_edges_by_key(&self, keys: &[u64]) -> Vec<SketchEdgeId> {
        self.draft_edges_for_keys(keys).0
    }

    /// [`Document::draft_edges_by_key`], with the keys that name no edge
    /// of the open draft (in `keys` order; every key when no draft is
    /// open).
    ///
    /// A key names a stored edge of the drawing as the listing shows it.
    /// The draft's lift also arranges the plane's drawing with the foreign
    /// edges lying in it (wireframe faces §4.1), which split a stored edge
    /// where another plane's line crosses or ends on it, while the listing
    /// ([`Document::plane_units`], [`Document::free_plane_units`]) does not
    /// gather them. Such a key names every piece of the working sketch
    /// lying along that stored edge when the pieces cover it end to end, so
    /// an edge crossed by another plane's line is still one unit to edit.
    pub fn draft_edges_for_keys(&self, keys: &[u64]) -> (Vec<SketchEdgeId>, Vec<u64>) {
        let Some(d) = self.current_draft() else {
            return (Vec::new(), keys.to_vec());
        };
        let w = &d.work;
        let plane = w.plane();
        let wanted: BTreeSet<u64> = keys.iter().copied().collect();
        let fixed = |a: Point3, b: Point3| d.lifted.fixed.iter().any(|&(c, f)| along(a, b, c, f));
        let drawing: Vec<(SketchEdgeId, Point3, Point3)> = w
            .edges()
            .iter()
            .map(|(id, e)| {
                (
                    id,
                    w.vertices()[e.from].position,
                    w.vertices()[e.to].position,
                )
            })
            // A solid's own edges are not drawing: they never name.
            .filter(|&(_, a, b)| !fixed(a, b))
            .collect();
        let mut picked: BTreeSet<SketchEdgeId> = BTreeSet::new();
        let mut found: BTreeSet<u64> = BTreeSet::new();
        for &(id, a, b) in &drawing {
            let k = crate::unit_key::edge_key(&plane, a, b);
            if wanted.contains(&k) {
                picked.insert(id);
                found.insert(k);
            }
        }
        for st in d
            .lifted
            .stored
            .iter()
            .filter(|s| matches!(s.store, Store::Sketch(_)))
        {
            let k = crate::unit_key::edge_key(&plane, st.a, st.b);
            if !wanted.contains(&k) || found.contains(&k) {
                continue;
            }
            let pieces: Vec<(SketchEdgeId, Point3, Point3)> = drawing
                .iter()
                .copied()
                .filter(|&(_, a, b)| along(a, b, st.a, st.b))
                .collect();
            let covers = !pieces.is_empty()
                && uncovered_parts(st.a, st.b, pieces.iter().map(|&(_, a, b)| (a, b))).is_empty();
            if covers {
                picked.extend(pieces.iter().map(|&(id, _, _)| id));
                found.insert(k);
            }
        }
        let missing = keys
            .iter()
            .copied()
            .filter(|k| !found.contains(k))
            .collect();
        let edges = drawing
            .iter()
            .map(|&(id, _, _)| id)
            .filter(|id| picked.contains(id))
            .collect();
        (edges, missing)
    }
}

impl Document {
    /// The planes `object` carries drawing on, in its own frame: each plane
    /// where two of its coplanar, same-facing faces meet along an edge that
    /// is not soft (a shape drawn there split the face), and each plane of a
    /// sketch it owns. One entry per plane, in face-key order then sketch order. Empty
    /// for a stale or hidden object. Renderers and the outliner list an
    /// object's drawing plane by plane through [`Document::plane_units`].
    pub fn object_drawing_planes(&self, object: ObjectId) -> Vec<Plane> {
        let Some(obj) = self.object(object) else {
            return Vec::new();
        };
        let mut planes: Vec<Plane> = Vec::new();
        let mut push = |p: Plane| {
            if !planes.iter().any(|q| planes_coincide(q, &p)) {
                planes.push(p);
            }
        };
        for (f, face) in obj.faces().iter() {
            let plane = face.plane;
            let split = std::iter::once(face.outer_loop)
                .chain(face.inner_loops.iter().copied())
                .flat_map(|l| obj.loop_half_edges(l))
                .any(|h| {
                    let he = &obj.half_edges()[h];
                    !obj.edges()[he.edge].soft
                        && he
                            .twin
                            .map(|t| obj.loops()[obj.half_edges()[t].loop_id].face)
                            .is_some_and(|g| {
                                g != f
                                    && face_on_plane(obj, g, &plane)
                                    && obj.faces()[g].material == face.material
                                    && obj.faces()[g].back_material == face.back_material
                            })
                });
            if split {
                push(plane);
            }
        }
        for s in self.object_owned_sketches(object) {
            push(self.sketches[s].plane());
        }
        planes
    }

    /// The objects a drawing on `plane` in `frame` lands on: every live
    /// object of the frame (top-level in the world, or the definition's
    /// members) with a face on the plane. Group members join only while
    /// their group is entered (its frame surfaces them to the top level).
    pub fn drawable_objects_on_plane(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
    ) -> Vec<ObjectId> {
        self.objects
            .iter()
            .filter(|(id, rec)| {
                !rec.hidden
                    && self.object_owner_component(*id) == frame
                    && match rec.owner {
                        ObjectOwner::World { parent } => parent.is_none(),
                        ObjectOwner::Definition { .. } => true,
                    }
                    && rec
                        .object
                        .faces()
                        .keys()
                        .any(|f| face_on_plane(&rec.object, f, &plane))
            })
            .map(|(id, _)| id)
            .collect()
    }

    /// The open draft's vertex at `at` (within [`tol::POINT_MERGE`]).
    pub fn draft_vertex_at(&self, at: Point3) -> Option<SketchVertexId> {
        let w = &self.current_draft()?.work;
        w.vertices()
            .iter()
            .find(|(_, v)| v.position.approx_eq(at, tol::POINT_MERGE))
            .map(|(id, _)| id)
    }

    /// The open draft's region whose key ([`DraftRegion::key`]) is `key`.
    pub fn draft_region_by_key(&self, key: u64) -> Option<crate::sketch::SketchRegionId> {
        region_by_key(&self.current_draft()?.work, key)
    }

    /// The profile of the region with key `key` ([`DraftRegion::key`]) in the
    /// drawing on `plane` — what an offset or an extrusion of that region
    /// would start from, read without opening a draft (an Offset tool's live
    /// preview offsets it with [`crate::offset::offset_profile`]).
    ///
    /// # Errors
    /// Anything [`Document::lift_plane`] refuses;
    /// [`DocumentError::Sketch`] with [`crate::sketch::SketchError::UnknownRegion`]
    /// when no region has that key.
    pub fn plane_region_profile(
        &self,
        plane: Plane,
        frame: Option<ComponentId>,
        objects: &[ObjectId],
        key: u64,
    ) -> Result<crate::sketch::Profile, DocumentError> {
        Self::region_profile_of(&self.lift_plane(plane, frame, objects)?, key)
    }

    /// [`Document::plane_region_profile`] for detached sketch `sketch`, in
    /// its own context ([`Document::detached_sketch_units`] lists its
    /// regions): what an offset of that region would start from.
    ///
    /// # Errors
    /// As [`Document::begin_draft_detached`] for the sketch;
    /// [`DocumentError::Sketch`] with
    /// [`crate::sketch::SketchError::UnknownRegion`] when no region has that
    /// key.
    pub fn detached_region_profile(
        &self,
        sketch: SketchId,
        key: u64,
    ) -> Result<crate::sketch::Profile, DocumentError> {
        Self::region_profile_of(&self.lift_detached(sketch)?, key)
    }

    /// The profile of the region with key `key` in a lift's drawing.
    fn region_profile_of(
        lifted: &Lifted,
        key: u64,
    ) -> Result<crate::sketch::Profile, DocumentError> {
        let rid = region_by_key(&lifted.before, key).ok_or(DocumentError::Sketch(
            crate::sketch::SketchError::UnknownRegion,
        ))?;
        lifted.before.profile(rid).map_err(DocumentError::Sketch)
    }

    /// The open draft's curve chains claiming the circle whose key
    /// ([`crate::unit_key::curve_key`]) is `key`.
    pub fn draft_curves_by_key(&self, key: u64) -> Vec<crate::sketch::SketchCurveId> {
        let Some(d) = self.current_draft() else {
            return Vec::new();
        };
        let w = &d.work;
        let chains: BTreeSet<crate::sketch::SketchCurveId> =
            w.edges().values().filter_map(|e| e.curve).collect();
        chains
            .into_iter()
            .filter(|&c| crate::unit_key::chain_key(w, c) == Some(key))
            .collect()
    }
}
