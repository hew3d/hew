//! Mend (the mesh-repair design): merge nearby points, separate pinched
//! parts and fill small gaps on one open object, as one explicit repair
//! (DEVELOPMENT.md rule 4: nothing here runs unless a plan names it).
//!
//! Every step works on a polygon soup taken from the object — positions,
//! face rings, each face's source — and the soup is rebuilt once at the
//! end through the builder at the object's own planarity tolerance, so a
//! Mend never commits an intermediate state and refuses with the object
//! untouched (the strong guarantee every repair gives).
//!
//! - **Weld**: points within the distance of each other become one; the
//!   lowest point (by position, so a reordered input gives the same
//!   result) keeps its place, and no point moves farther than the
//!   distance. Faces the weld collapses are dropped and counted.
//! - **Folds**: faces no wider than the weld distance at an open or
//!   crowded edge (zero-width folds) are removed and counted; the seams
//!   they leave are stitched by the fill.
//! - **Split**: around every point, the corners of the faces meeting
//!   there are grouped into fans joined through shared edges; each fan
//!   past the first takes a copy of the point. An edge used by more than
//!   two faces (a weld can make one) is paired up by the faces' turn about
//!   it, so two solids touching along an edge come apart whole. Islands
//!   become objects of their own; the largest stays.
//! - **Fill**: each closed hole is closed by the cheapest patch that fits:
//!   a seam no wider than the weld distance is stitched shut (each side's
//!   points let into the other side's edges, no new face); a flat hole
//!   takes one face, as Close an opening would; a bent one takes a
//!   minimum-weight triangulation, or past [`FILL_MWT_MAX_EDGES`] a fan
//!   from its center. A patch that would cross the object is left out and
//!   reported, never committed.

use std::collections::{BTreeMap, BTreeSet};

use slotmap::SecondaryMap;

use super::RepairError;
use crate::error::TopologyError;
use crate::health::{self, RimRole};
use crate::ids::{FaceId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::tol;
use crate::topo::{Object, WatertightState};

/// The largest hole (in edges) filled by the minimum-weight triangulation;
/// larger holes take a fan from their center. A work bound (the dynamic
/// program is cubic in the edges), not a tolerance.
pub const FILL_MWT_MAX_EDGES: usize = 300;

/// The minimum-weight triangulations' whole work in one Mend (about n³/6
/// steps per rim of n edges, summed over rims): past it the rest of the
/// holes take fans, so many large holes never stall the Mend. A work
/// bound, not a tolerance: about two seconds of wasm.
pub const FILL_MWT_TOTAL_WORK: f64 = 2e8;

/// How much a fill triangle's bend against its neighbors weighs against
/// its area in the minimum-weight triangulation (dimensionless; the bend
/// term is scaled by the hole's extent squared, so it is an area too).
pub const FILL_DIHEDRAL_WEIGHT: f64 = 0.1;

/// A Mend that welded a solid refuses ([`RepairError::WeldFlattensObject`])
/// when its results keep less than this fraction of the solid's volume
/// (dimensionless): a weld moves no point farther than the merge
/// distance, so on a real solid the volume barely changes, and losing
/// half of it means opposite sides were pressed together.
pub const FLAT_VOLUME_FRACTION: f64 = 0.5;

/// A closed result of a weld whose volume is at most this fraction of its
/// area times the merge distance (dimensionless; an average thickness of
/// a thousandth of that distance) encloses nothing: a shell of faces
/// pressed onto each other, refused as
/// [`RepairError::WeldFlattensObject`].
pub const FLAT_THICKNESS: f64 = 1e-3;

/// The steps a Mend runs. Every field is explicit: nothing runs that the
/// caller did not ask for (rule 4).
#[derive(Debug, Clone, PartialEq)]
pub struct MendPlan {
    /// Merge points closer than this (meters). `None`: no weld.
    pub weld: Option<f64>,
    /// Remove folds: faces that repeat another face exactly, and faces of
    /// zero width — every point within the weld distance (the plan's, but
    /// never past the default one) of the line through the face's longest
    /// edge — that touch an open or crowded edge. The seams they leave are
    /// stitched by the fill.
    pub folds: bool,
    /// Split branching boundary points and edges a weld made shared by
    /// more than two faces; separate islands into objects of their own.
    pub split: bool,
    /// Fill closed holes. `None`: no fill.
    pub fill: Option<FillScope>,
    /// Refuse [`RepairError::StillOpen`] instead of committing a result
    /// that is valid but not watertight (scripts that want a solid or
    /// nothing).
    pub require_solid: bool,
}

impl MendPlan {
    /// The one-click Mend: weld at `weld`, split, fill every hole.
    pub fn all(weld: f64) -> MendPlan {
        MendPlan {
            weld: Some(weld),
            folds: true,
            split: true,
            fill: Some(FillScope::Holes),
            require_solid: false,
        }
    }
}

/// Which open rims a Mend fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillScope {
    /// Every closed rim that is a hole ([`RimRole::Hole`]); an outer rim
    /// (a bent sheet's edge) stays open.
    Holes,
    /// The rims with these indices into [`health::diagnose`]'s rims of the
    /// object as it is before the Mend, whatever their role. A rim the weld
    /// or split changes is followed through them.
    Rims(Vec<usize>),
}

/// A rim a Mend left open, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedRim {
    /// 0 for the mended object, `i + 1` for piece `i`.
    pub object: usize,
    /// The rim's index in [`health::diagnose`] of that object after the
    /// Mend: the rim through the skipped rim's first edge, else the first
    /// through its first point; `None` when no open rim passes there (the
    /// rim is still counted, never dropped from the report).
    pub rim: Option<usize>,
    pub reason: RepairError,
}

/// What a Mend did.
#[derive(Debug, Clone, PartialEq)]
pub struct MendReport {
    /// Points merged away.
    pub welded_vertices: usize,
    /// The farthest a merged point moved (meters).
    pub weld_max_move: f64,
    /// Faces (or a face's holes) dropped because the weld collapsed them.
    pub collapsed_faces: usize,
    /// Folds the fold step removed: exact repeats of a face, zero-width
    /// faces at open or crowded edges.
    pub folds_removed: usize,
    /// Zero-width doublings-back a stitch cut out of a face's ring.
    pub seam_folds: usize,
    /// Edges the Mend left between two faces on one plane (a sliver gone
    /// from between them, a flat fill beside its plane), made soft so the
    /// Mend adds no drawing: no line shows there and the plane lists no
    /// new shape. Seams the object drew before stay as they were.
    pub soft_seams: usize,
    /// Points split because the faces around them fell into several fans.
    pub split_vertices: usize,
    /// Edges used by more than two faces (or twice the same way) that were
    /// cut apart.
    pub split_edges: usize,
    /// Islands returned as objects of their own (not counting the kept one).
    pub pieces: usize,
    /// Rims closed, stitched ones included.
    pub filled_rims: usize,
    /// Of `filled_rims`, the seams no wider than the weld distance closed
    /// by stitching (no new face).
    pub stitched_rims: usize,
    /// Triangles the bent-hole fills added.
    pub fill_triangles: usize,
    /// Rims left open and why.
    pub skipped_rims: Vec<SkippedRim>,
    /// An island was turned right side out.
    pub reversed: bool,
    /// The kept object is watertight.
    pub closed: bool,
    /// How many pieces are watertight.
    pub pieces_closed: usize,
}

/// What a Mend would do: the preview, counted by running the Mend on a
/// copy, so each count is the commit's ([`Object::mend_census`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MendCensus {
    /// Pairs of points within the weld distance.
    pub weld_pairs: usize,
    /// Points the weld would merge away.
    pub weld_vertices: usize,
    /// Faces the fold step removes ([`MendReport::folds_removed`]).
    pub folds: usize,
    /// Faces the weld collapses ([`MendReport::collapsed_faces`]).
    pub collapsed_faces: usize,
    /// Crowded edges the split cuts ([`MendReport::split_edges`]).
    pub split_edges: usize,
    /// The widest gap among those pairs (meters).
    pub weld_max_gap: f64,
    /// Points the split splits ([`MendReport::split_vertices`]: fans that
    /// meet only there, on a rim or inside a closed surface).
    pub branching_vertices: usize,
    /// Islands of faces after the split ([`MendReport::pieces`] + 1), or
    /// as the object is when the plan does not split.
    pub islands: usize,
    /// New objects the Mend makes ([`MendReport::pieces`]): the islands
    /// past the kept one, whether a pinch's split or a loose part made
    /// them.
    pub pieces: usize,
    /// The object's open rims as they are, by role (not what the fill
    /// closes: see `filled_rims`).
    pub hole_rims: usize,
    pub hole_rims_planar: usize,
    pub hole_rims_warped: usize,
    pub outer_rims: usize,
    /// Points that would merge, to highlight.
    pub weld_points: Vec<Point3>,
    /// Points that would split, to highlight.
    pub split_points: Vec<Point3>,
    /// Rim indices ([`health::diagnose`] order, before the Mend) the fill
    /// closes: those with an edge in a filled rim. A weld that closes a
    /// rim leaves it out; rims the weld joins are each listed.
    pub fill_rims: Vec<usize>,
    /// Rims the fill closes ([`MendReport::filled_rims`]), seams the weld
    /// or the folds left included.
    pub filled_rims: usize,
    /// Of `filled_rims`, those closed with triangles (a bent hole); the
    /// rest take one flat face or are stitched.
    pub fill_curved: usize,
    /// Rims the fill leaves open ([`MendReport::skipped_rims`]).
    pub skipped_rims: usize,
}

/// The weld distances Mend accepts for `obj`: from [`tol::WELD_MIN`] (or
/// the object's planarity tolerance, when tighter) to that tolerance — a
/// weld moves points, and a moved point must stay on its faces' planes.
pub fn weld_range(obj: &Object) -> (f64, f64) {
    let hi = obj.planarity_tol;
    (tol::WELD_MIN.min(hi), hi)
}

/// The default weld distance: [`tol::WELD_DEFAULT_REL`] of the object's
/// diameter, within [`weld_range`].
pub fn default_weld_distance(obj: &Object) -> f64 {
    let pts: Vec<Point3> = obj.vertices.values().map(|v| v.position).collect();
    let (lo, hi) = weld_range(obj);
    (tol::WELD_DEFAULT_REL * health::diameter(&pts)).clamp(lo, hi)
}

// ───────────────────────────────────────────────────────────── the soup

/// One face of the soup: its rings (outer first), the face it came from
/// and that face's plane while its rings are as they were.
#[derive(Debug, Clone)]
struct SFace {
    rings: Vec<Vec<usize>>,
    src: Option<FaceId>,
    /// The source face's plane, kept while the rings are unchanged.
    plane: Option<Plane>,
    /// The fill (rim tag) that made or stitched this face.
    fill: Option<usize>,
}

#[derive(Debug, Clone)]
struct Soup {
    pos: Vec<Point3>,
    /// Each point's source point (its own index for an original, what it
    /// copied for a split's copy, `usize::MAX` for a fan's center).
    origin: Vec<usize>,
    faces: Vec<SFace>,
}

impl Soup {
    /// The object's faces as rings over dense vertex indices (vertex
    /// storage order), and that index.
    fn of(obj: &Object) -> (Soup, SecondaryMap<VertexId, usize>) {
        let mut index = SecondaryMap::new();
        let mut pos = Vec::with_capacity(obj.vertices.len());
        for (v, vx) in obj.vertices.iter() {
            index.insert(v, pos.len());
            pos.push(vx.position);
        }
        let faces = obj
            .faces
            .iter()
            .map(|(fid, f)| SFace {
                rings: std::iter::once(f.outer_loop)
                    .chain(f.inner_loops.iter().copied())
                    .map(|l| {
                        obj.loop_half_edges(l)
                            .map(|h| index[obj.half_edges[h].origin])
                            .collect()
                    })
                    .collect(),
                src: Some(fid),
                plane: Some(f.plane),
                fill: None,
            })
            .collect();
        let origin = (0..pos.len()).collect();
        (Soup { pos, origin, faces }, index)
    }

    /// The unit normal of face `f`: its source plane's while unchanged,
    /// else its outer ring's Newell normal (zero when it has none).
    fn normal(&self, f: usize) -> Vec3 {
        let face = &self.faces[f];
        if let Some(p) = face.plane {
            return p.normal();
        }
        newell(&self.ring_points(&face.rings[0]))
            .normalized()
            .unwrap_or(Vec3::ZERO)
    }

    fn ring_points(&self, ring: &[usize]) -> Vec<Point3> {
        ring.iter().map(|&i| self.pos[i]).collect()
    }
}

fn newell(points: &[Point3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        n = n + Vec3::new(
            (p.y - q.y) * (p.z + q.z),
            (p.z - q.z) * (p.x + q.x),
            (p.x - q.x) * (p.y + q.y),
        );
    }
    n
}

fn cmp_point(a: Point3, b: Point3) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x)
        .then(a.y.total_cmp(&b.y))
        .then(a.z.total_cmp(&b.z))
}

/// Each point's place in position order (ties by index): the order every
/// choice between points follows, so a reordered input decides the same.
fn ranks(pos: &[Point3]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..pos.len()).collect();
    idx.sort_by(|&a, &b| cmp_point(pos[a], pos[b]).then(a.cmp(&b)));
    let mut r = vec![0; pos.len()];
    for (k, &i) in idx.iter().enumerate() {
        r[i] = k;
    }
    r
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

// ─────────────────────────────────────────────────────────────── corners

/// One corner of a face ring, which is also the use of the edge leaving
/// it: `rings[f][r][k] → rings[f][r][k + 1]`.
#[derive(Debug, Clone, Copy)]
struct Corner {
    f: usize,
    r: usize,
    k: usize,
    /// The flat index of the ring's first corner, and the ring's length.
    start: usize,
    len: usize,
}

struct Uses {
    corners: Vec<Corner>,
    /// `(lo, hi, forward, corner)` per edge use, sorted: `forward` when the
    /// use runs lo → hi.
    edges: Vec<(usize, usize, bool, usize)>,
}

impl Uses {
    fn of(soup: &Soup, faces: &[usize]) -> Uses {
        let mut corners = Vec::new();
        for &f in faces {
            for (r, ring) in soup.faces[f].rings.iter().enumerate() {
                let start = corners.len();
                for k in 0..ring.len() {
                    corners.push(Corner {
                        f,
                        r,
                        k,
                        start,
                        len: ring.len(),
                    });
                }
            }
        }
        let mut edges: Vec<(usize, usize, bool, usize)> = corners
            .iter()
            .enumerate()
            .map(|(c, _)| {
                let (a, b) = Self::ends_of(soup, &corners, c);
                (a.min(b), a.max(b), a < b, c)
            })
            .collect();
        edges.sort_unstable();
        Uses { corners, edges }
    }

    fn ends_of(soup: &Soup, corners: &[Corner], c: usize) -> (usize, usize) {
        let k = corners[c];
        let ring = &soup.faces[k.f].rings[k.r];
        (ring[k.k], ring[(k.k + 1) % k.len])
    }

    fn ends(&self, soup: &Soup, c: usize) -> (usize, usize) {
        Self::ends_of(soup, &self.corners, c)
    }

    fn next(&self, c: usize) -> usize {
        let k = self.corners[c];
        k.start + (k.k + 1) % k.len
    }

    fn vertex(&self, soup: &Soup, c: usize) -> usize {
        let k = self.corners[c];
        soup.faces[k.f].rings[k.r][k.k]
    }

    /// The uses grouped by undirected edge.
    fn groups(&self) -> impl Iterator<Item = &[(usize, usize, bool, usize)]> {
        self.edges
            .chunk_by(|a, b| a.0 == b.0 && a.1 == b.1)
            .filter(|g| !g.is_empty())
    }
}

/// Whether a group of uses of one edge is clean: two uses running
/// opposite ways.
fn clean(g: &[(usize, usize, bool, usize)]) -> bool {
    g.len() == 2 && g[0].2 != g[1].2
}

/// Whether the soup has an edge used by more than two faces or twice the
/// same way, or a ring through a point twice: what the builder refuses.
fn folds(soup: &Soup, faces: &[usize]) -> bool {
    let ring_repeats = faces.iter().any(|&f| {
        soup.faces[f].rings.iter().any(|ring| {
            let mut s = ring.clone();
            s.sort_unstable();
            s.windows(2).any(|w| w[0] == w[1])
        })
    });
    ring_repeats
        || Uses::of(soup, faces)
            .groups()
            .any(|g| g.len() > 1 && !clean(g))
}

// ────────────────────────────────────────────────────────────────── weld

/// The largest cell number the weld's grid uses on an axis (2⁵⁰): far
/// inside `i64`, so a neighbor's key never overflows.
const GRID_MAX_CELL: f64 = 1_125_899_906_842_624.0;

struct WeldPlan {
    /// Each point's cluster root (itself when it stays).
    root: Vec<usize>,
    merged: usize,
    max_move: f64,
    pairs: usize,
    max_gap: f64,
}

/// The weld at distance `d`: pairs within `d` joined (a grid of cells
/// `d` across, the 27 around each point), each cluster rooted at its
/// lowest point; a cluster with a point farther than `d` from its root
/// (a chain of near points along a thin feature) is re-split greedily in
/// position order, so no point moves farther than `d`.
fn weld_clusters(pos: &[Point3], d: f64) -> WeldPlan {
    let n = pos.len();
    let rank = ranks(pos);
    // Cells at least `d` across, and never so small that a coordinate's
    // cell number leaves ±2⁵⁰ (an object far from the origin, or a tiny
    // distance on a huge object): the key and its neighbors stay exact.
    let reach = pos
        .iter()
        .flat_map(|p| [p.x.abs(), p.y.abs(), p.z.abs()])
        .filter(|c| c.is_finite())
        .fold(0.0f64, f64::max);
    let cell = d.max(reach / GRID_MAX_CELL);
    let key = |p: Point3| {
        let k = |c: f64| (c / cell).floor().clamp(-GRID_MAX_CELL, GRID_MAX_CELL) as i64;
        (k(p.x), k(p.y), k(p.z))
    };
    let mut cells: Vec<((i64, i64, i64), usize)> =
        pos.iter().enumerate().map(|(i, &p)| (key(p), i)).collect();
    cells.sort_unstable();
    let mut parent: Vec<usize> = (0..n).collect();
    let (mut pairs, mut max_gap) = (0usize, 0.0f64);
    for i in 0..n {
        let (x, y, z) = key(pos[i]);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let c = (x + dx, y + dy, z + dz);
                    let lo = cells.partition_point(|e| e.0 < c);
                    for &(_, j) in cells[lo..].iter().take_while(|e| e.0 == c) {
                        if j <= i {
                            continue;
                        }
                        let g = (pos[j] - pos[i]).length();
                        if g <= d {
                            pairs += 1;
                            max_gap = max_gap.max(g);
                            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                            if ri != rj {
                                if rank[ri] < rank[rj] {
                                    parent[rj] = ri;
                                } else {
                                    parent[ri] = rj;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        if r != i {
            clusters.entry(r).or_insert_with(|| vec![r]).push(i);
        }
    }
    let mut root: Vec<usize> = (0..n).collect();
    let mut max_move = 0.0f64;
    for (_, mut members) in clusters {
        members.sort_by_key(|&m| rank[m]);
        let head = members[0];
        let whole = members.iter().all(|&m| (pos[m] - pos[head]).length() <= d);
        let mut heads: Vec<usize> = Vec::new();
        for &m in &members {
            let to = if whole {
                Some(head)
            } else {
                heads
                    .iter()
                    .copied()
                    .find(|&h| (pos[m] - pos[h]).length() <= d)
            };
            match to {
                Some(h) => {
                    root[m] = h;
                    max_move = max_move.max((pos[m] - pos[h]).length());
                }
                None => heads.push(m),
            }
        }
    }
    let merged = (0..n).filter(|&i| root[i] != i).count();
    WeldPlan {
        root,
        merged,
        max_move,
        pairs,
        max_gap,
    }
}

/// What [`apply_weld`] dropped.
struct Collapsed {
    /// Faces (or a face's holes) dropped.
    count: usize,
    /// The edges of the faces dropped while still on three or more points
    /// (degenerate or too thin to build): the cracks they leave are the
    /// fill's to stitch, so the open-edge check allows for them.
    open_edges: usize,
}

/// Rewrite every ring through the weld's roots; drop what collapses (a
/// face under three points, or, where the weld changed it, one the
/// builder could not give a plane — degenerate or too thin; a hole under
/// three points). Refuses [`RepairError::WeldFlipsFaces`] when a face the
/// weld keeps turns over (a point merged across the face's opposite edge:
/// its normal now points against the one it had).
fn apply_weld(soup: &mut Soup, root: &[usize]) -> Result<Collapsed, RepairError> {
    let mut collapsed = Collapsed {
        count: 0,
        open_edges: 0,
    };
    let mut keep = Vec::with_capacity(soup.faces.len());
    for mut face in std::mem::take(&mut soup.faces) {
        let before = newell(&soup.ring_points(&face.rings[0]));
        let mut changed = false;
        for ring in face.rings.iter_mut() {
            let before = ring.len();
            let mut out: Vec<usize> = Vec::with_capacity(before);
            for &v in ring.iter() {
                let r = root[v];
                changed |= r != v;
                if out.last() != Some(&r) {
                    out.push(r);
                }
            }
            while out.len() > 1 && out.first() == out.last() {
                out.pop();
            }
            *ring = out;
        }
        if !changed {
            keep.push(face);
            continue;
        }
        face.plane = None;
        if face.rings[0].len() < 3 {
            collapsed.count += 1;
            continue;
        }
        let after = soup.ring_points(&face.rings[0]);
        if !buildable(&after) {
            collapsed.count += 1;
            collapsed.open_edges += face.rings.iter().map(Vec::len).sum::<usize>();
            continue;
        }
        if newell(&after).dot(before) < 0.0 {
            return Err(RepairError::WeldFlipsFaces);
        }
        let holes = face.rings.len();
        let outer = face.rings.remove(0);
        face.rings.retain(|h| h.len() >= 3);
        collapsed.count += holes - 1 - face.rings.len();
        face.rings.insert(0, outer);
        keep.push(face);
    }
    soup.faces = keep;
    Ok(collapsed)
}

/// How thin a face must be to count as a fold: the plan's weld distance,
/// but never more than the object's default one (micron-scale on a print),
/// so a raised weld distance never takes a thin wall that is really there.
fn fold_width(obj: &Object, plan: &MendPlan) -> f64 {
    let default = default_weld_distance(obj);
    plan.weld.map_or(default, |w| w.min(default))
}

/// Whether every point of `ring` lies within `d` of the line through its
/// longest edge: a face of zero width at that distance, whatever its
/// length.
fn thinner_than(ring: &[Point3], d: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return true;
    }
    let longest = (0..n)
        .max_by(|&i, &j| {
            let li = (ring[(i + 1) % n] - ring[i]).length_squared();
            let lj = (ring[(j + 1) % n] - ring[j]).length_squared();
            li.total_cmp(&lj)
        })
        .expect("a ring of three or more");
    let (a, b) = (ring[longest], ring[(longest + 1) % n]);
    let dir = b - a;
    let len2 = dir.length_squared();
    if len2 == 0.0 {
        return true;
    }
    ring.iter().all(|&p| {
        let t = (p - a).dot(dir) / len2;
        (p - (a + dir * t)).length() <= d
    })
}

/// The fold step: drop every face that repeats another face exactly (the
/// same rings, wound the same way: a doubled surface), then the faces of
/// one ring no wider than `d` that touch the object's defects (a point on
/// an open or crowded edge). Each removal opens its edges, so the faces
/// at its points are looked at in turn, in one pass over a work list.
/// How many went, and how many edges they had.
fn remove_folds(soup: &mut Soup, d: f64) -> (usize, usize) {
    let n = soup.faces.len();
    let mut gone = vec![false; n];
    // Exact repeats: each ring rotated to its lowest point, holes sorted.
    let mut seen: BTreeMap<Vec<Vec<usize>>, usize> = BTreeMap::new();
    for (f, face) in soup.faces.iter().enumerate() {
        let mut key: Vec<Vec<usize>> = face
            .rings
            .iter()
            .map(|r| {
                let k = (0..r.len()).min_by_key(|&i| r[i]).unwrap_or(0);
                let mut r = r.clone();
                r.rotate_left(k);
                r
            })
            .collect();
        key[1..].sort();
        if seen.insert(key, f).is_some() {
            gone[f] = true;
        }
    }
    let thin: Vec<bool> = soup
        .faces
        .iter()
        .map(|f| f.rings.len() == 1 && thinner_than(&soup.ring_points(&f.rings[0]), d))
        .collect();
    let all: Vec<usize> = (0..n).filter(|&f| !gone[f]).collect();
    let uses = Uses::of(soup, &all);
    let mut defect = vec![false; soup.pos.len()];
    for g in uses.groups() {
        if !clean(g) {
            defect[g[0].0] = true;
            defect[g[0].1] = true;
        }
    }
    // (A repeat's points are no defect: the copy kept still holds every
    // edge the repeat ran along, clean once the repeat is gone.)
    let mut at: Vec<Vec<usize>> = vec![Vec::new(); soup.pos.len()];
    for (f, face) in soup.faces.iter().enumerate() {
        if thin[f] && !gone[f] {
            for &v in &face.rings[0] {
                at[v].push(f);
            }
        }
    }
    let mut work: Vec<usize> = (0..n)
        .filter(|&f| thin[f] && !gone[f] && soup.faces[f].rings[0].iter().any(|&v| defect[v]))
        .collect();
    while let Some(f) = work.pop() {
        if gone[f] {
            continue;
        }
        gone[f] = true;
        for &v in &soup.faces[f].rings[0] {
            if !defect[v] {
                defect[v] = true;
                work.extend(at[v].iter().copied().filter(|&g| !gone[g]));
            }
        }
    }
    let removed = gone.iter().filter(|&&g| g).count();
    let edges = (0..n)
        .filter(|&f| gone[f])
        .map(|f| soup.faces[f].rings.iter().map(Vec::len).sum::<usize>())
        .sum();
    let mut k = 0;
    soup.faces.retain(|_| {
        k += 1;
        !gone[k - 1]
    });
    (removed, edges)
}

// ───────────────────────────────────────────────────────────────── split

/// Pair the uses of one edge used by more than two faces (or twice the
/// same way) by their faces' turn about it: going round the edge, a face
/// whose use runs against the edge opens a wedge of material that the
/// next face running with it closes (two faces at one angle — solids
/// touching — close before they open). Unmatched uses stay boundary.
fn pair_around(
    soup: &Soup,
    uses: &Uses,
    group: &[(usize, usize, bool, usize)],
    skip: &BTreeSet<usize>,
) -> Vec<(usize, usize)> {
    let (lo, hi) = (group[0].0, group[0].1);
    let Ok(t) = (soup.pos[hi] - soup.pos[lo]).normalized() else {
        return Vec::new();
    };
    let helper = if t.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let Ok(e1) = t.cross(helper).normalized() else {
        return Vec::new();
    };
    let e2 = t.cross(e1);
    let mut around: Vec<(f64, bool, usize)> = group
        .iter()
        .filter(|u| !skip.contains(&u.3))
        .map(|&(_, _, fwd, c)| {
            let dir = if fwd { t } else { -t };
            let w = soup.normal(uses.corners[c].f).cross(dir);
            (w.dot(e2).atan2(w.dot(e1)), fwd, c)
        })
        .collect();
    // `true` (closing) sorts after `false`; closing goes first at a tie.
    around.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    let n = around.len();
    if n < 2 {
        return Vec::new();
    }
    // Start after the lowest running depth, so a balanced circle matches
    // in one pass.
    let (mut depth, mut low, mut at) = (0i64, 0i64, 0usize);
    for (i, a) in around.iter().enumerate() {
        depth += if a.1 { -1 } else { 1 };
        if depth < low {
            low = depth;
            at = i + 1;
        }
    }
    let mut open: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for k in 0..n {
        let (_, closing, c) = around[(at + k) % n];
        if closing {
            if let Some(o) = open.pop() {
                out.push((o, c));
            }
        } else {
            open.push(c);
        }
    }
    out
}

struct SplitStats {
    vertices: usize,
    edges: usize,
    /// The points that split, by their index before the split.
    points: Vec<usize>,
}

/// The split (see the module docs): every point whose corners fall into
/// several fans takes a copy per fan past the first. Corners are joined
/// through paired uses; edges with more than two uses are paired by
/// [`pair_around`]. A pinch the pairing cannot separate (both pairs of an
/// edge in one fan at both ends, or a ring through one point twice) has
/// the offending corner cut loose and the split rerun.
fn split(soup: &mut Soup) -> SplitStats {
    let all: Vec<usize> = (0..soup.faces.len()).collect();
    let base: Vec<SFace> = soup.faces.clone();
    let n_before = soup.pos.len();
    let mut isolated: BTreeSet<usize> = BTreeSet::new();
    let mut bad_edges: Option<usize> = None;
    loop {
        soup.faces = base.clone();
        let uses = Uses::of(soup, &all);
        let nc = uses.corners.len();
        let mut parent: Vec<usize> = (0..nc).collect();
        let mut unclean = 0;
        let join = |u: usize, w: usize, parent: &mut Vec<usize>| {
            for (a, b) in [(u, uses.next(w)), (uses.next(u), w)] {
                if isolated.contains(&a) || isolated.contains(&b) {
                    continue;
                }
                let (ra, rb) = (find(parent, a), find(parent, b));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        };
        for g in uses.groups() {
            if g.len() < 2 {
                continue;
            }
            if clean(g) {
                join(g[0].3, g[1].3, &mut parent);
                continue;
            }
            unclean += 1;
            for (u, w) in pair_around(soup, &uses, g, &isolated) {
                join(u, w, &mut parent);
            }
        }
        bad_edges.get_or_insert(unclean);
        // An isolated corner joined nothing, so it is a fan of its own.
        let mut by_vertex: Vec<(usize, usize)> =
            (0..nc).map(|c| (uses.vertex(soup, c), c)).collect();
        by_vertex.sort_unstable();
        let mut split_vertices = 0;
        let mut split_points: Vec<usize> = Vec::new();
        let mut new_of: Vec<usize> = vec![usize::MAX; nc];
        for g in by_vertex.chunk_by(|a, b| a.0 == b.0) {
            let v = g[0].0;
            let mut sector_vertex: BTreeMap<usize, usize> = BTreeMap::new();
            for &(_, c) in g {
                let r = if isolated.contains(&c) {
                    usize::MAX - c
                } else {
                    find(&mut parent, c)
                };
                let next = if sector_vertex.is_empty() {
                    v
                } else {
                    usize::MAX
                };
                let e = sector_vertex.entry(r).or_insert(next);
                if *e == usize::MAX {
                    soup.pos.push(soup.pos[v]);
                    soup.origin.push(soup.origin[v]);
                    *e = soup.pos.len() - 1;
                }
                new_of[c] = *e;
            }
            if sector_vertex.len() > 1 {
                split_vertices += 1;
                split_points.push(v);
            }
        }
        for (c, k) in uses.corners.iter().enumerate() {
            soup.faces[k.f].rings[k.r][k.k] = new_of[c];
        }
        // What still folds: cut its later corners loose and go again.
        let after = Uses::of(soup, &all);
        let mut cut: Vec<usize> = Vec::new();
        for g in after.groups() {
            if g.len() > 1 && !clean(g) {
                // Keep one clean pair if there is one; cut the rest.
                let keep: Option<(usize, usize)> = g.iter().find_map(|a| {
                    g.iter()
                        .find(|b| b.2 != a.2 && b.3 != a.3)
                        .map(|b| (a.3.min(b.3), a.3.max(b.3)))
                });
                cut.extend(
                    g.iter()
                        .map(|u| u.3)
                        .filter(|&c| keep.is_none_or(|(a, b)| c != a && c != b)),
                );
            }
        }
        // Each ring's first corner, so a ring position's corner is a sum.
        let mut start: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for q in &after.corners {
            start.entry((q.f, q.r)).or_insert(q.start);
        }
        for (f, face) in soup.faces.iter().enumerate() {
            for (r, ring) in face.rings.iter().enumerate() {
                let mut seen: BTreeSet<usize> = BTreeSet::new();
                for (k, &v) in ring.iter().enumerate() {
                    if !seen.insert(v) {
                        cut.push(start[&(f, r)] + k);
                    }
                }
            }
        }
        // A corner cut loose already that still folds takes the next one
        // round its ring with it.
        let fresh: Vec<usize> = cut
            .into_iter()
            .map(|c| {
                if isolated.contains(&c) {
                    after.next(c)
                } else {
                    c
                }
            })
            .filter(|c| !isolated.contains(c))
            .collect();
        if fresh.is_empty() {
            // Positions the copies left unused are dropped at the rebuild.
            return SplitStats {
                vertices: split_vertices,
                edges: bad_edges.unwrap_or(0),
                points: split_points,
            };
        }
        isolated.extend(fresh);
        soup.pos.truncate(n_before);
        soup.origin.truncate(n_before);
    }
}

/// The soup's islands: faces joined through clean edges, each island's
/// faces in soup order, islands in order of their first face.
fn islands(soup: &Soup) -> Vec<Vec<usize>> {
    let all: Vec<usize> = (0..soup.faces.len()).collect();
    let uses = Uses::of(soup, &all);
    let mut parent: Vec<usize> = (0..soup.faces.len()).collect();
    for g in uses.groups() {
        if g.len() == 2 && clean(g) {
            let (a, b) = (uses.corners[g[0].3].f, uses.corners[g[1].3].f);
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut label: BTreeMap<usize, usize> = BTreeMap::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    for f in 0..soup.faces.len() {
        let r = find(&mut parent, f);
        let i = *label.entry(r).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[i].push(f);
    }
    out
}

// ────────────────────────────────────────────────────────────────── fill

#[derive(Clone)]
/// One open rim of an island: its corners (each the boundary use leaving
/// the rim's next point) in boundary order, starting at its lowest point.
struct SoupRim {
    corners: Vec<usize>,
    points: Vec<usize>,
    /// The rim runs through a point where the boundary branches.
    branching: bool,
    closed: bool,
}

fn rims(soup: &Soup, uses: &Uses, rank: &[usize]) -> Vec<SoupRim> {
    let mut boundary: Vec<usize> = uses
        .groups()
        .filter(|g| g.len() == 1)
        .map(|g| g[0].3)
        .collect();
    let mut leaving: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &c in &boundary {
        leaving.entry(uses.ends(soup, c).0).or_default().push(c);
    }
    boundary.sort_by_key(|&c| (rank[uses.ends(soup, c).0], c));
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut out = Vec::new();
    for &start in &boundary {
        if used.contains(&start) {
            continue;
        }
        used.insert(start);
        let first = uses.ends(soup, start).0;
        let mut chain = vec![start];
        let mut branching = leaving[&first].len() > 1;
        let mut cur = start;
        let closed = loop {
            let at = uses.ends(soup, cur).1;
            if at == first {
                break true;
            }
            let out_here = leaving.get(&at).map(Vec::as_slice).unwrap_or_default();
            branching |= out_here.len() > 1;
            let Some(&next) = out_here.iter().find(|c| !used.contains(c)) else {
                break false;
            };
            used.insert(next);
            chain.push(next);
            cur = next;
        };
        // Start at the rim's lowest point.
        let k = (0..chain.len())
            .min_by_key(|&i| rank[uses.ends(soup, chain[i]).0])
            .unwrap_or(0);
        if closed {
            chain.rotate_left(k);
        }
        let points = chain.iter().map(|&c| uses.ends(soup, c).0).collect();
        out.push(SoupRim {
            corners: chain,
            points,
            branching,
            closed,
        });
    }
    out
}

/// One rim's patch: points of a split that become one again, points let
/// into existing rings along a seam, and new faces.
#[derive(Debug, Clone, Default)]
struct Patch {
    /// `(gone, kept)`: `gone` becomes `kept` in every ring, first.
    rejoin: Vec<(usize, usize)>,
    /// Into face `f`'s ring `r`, between consecutive points `a` and `b`,
    /// the points `between` in order.
    cuts: Vec<(usize, usize, usize, usize, Vec<usize>)>,
    /// New faces (rings over soup points).
    rings: Vec<Vec<usize>>,
    /// How many of `rings` are fill triangles.
    triangles: usize,
}

impl Patch {
    fn faces(rings: Vec<Vec<usize>>, triangles: usize) -> Patch {
        Patch {
            rings,
            triangles,
            ..Patch::default()
        }
    }

    /// A seam closed by stitching alone (no new face).
    fn stitched(&self) -> bool {
        self.rings.is_empty()
    }
}

/// Whether a ring would build as a face: a plane the builder and a saved
/// file both find, and not degenerate. The builder asks an area plane of
/// every ring, imported or not.
fn buildable(points: &[Point3]) -> bool {
    !ring_is_degenerate(points)
        && crate::serialize::outer_ring_plane(points).is_ok()
        && Plane::from_polygon(points).is_ok()
}

/// Whether a ring is degenerate: an edge shorter than `POINT_MERGE`, or a
/// sliver whose every vertex lies within `POINT_MERGE` of the line through
/// its longest edge (zero width, whatever its length).
fn ring_is_degenerate(ring: &[Point3]) -> bool {
    let n = ring.len();
    if n < 3 {
        return true;
    }
    let edge = |k: usize| (ring[k], ring[(k + 1) % n]);
    if (0..n).any(|k| {
        let (a, b) = edge(k);
        a.approx_eq(b, tol::POINT_MERGE)
    }) {
        return true;
    }
    let longest = (0..n)
        .max_by(|&i, &j| {
            let (a, b) = edge(i);
            let (c, d) = edge(j);
            (b - a).length().total_cmp(&(d - c).length())
        })
        .expect("a ring of three or more");
    let (a, b) = edge(longest);
    let dir = b - a;
    ring.iter().all(|&p| {
        let t = (p - a).dot(dir) / dir.length_squared();
        (p - (a + dir * t)).length() <= tol::POINT_MERGE
    })
}

/// How wide a seam may be against its length (dimensionless): a rim
/// whose points all lie within this fraction of its length (or within the
/// weld distance, whichever is more) of the line through its two
/// farthest points is a seam, stitched rather than filled. A face along a
/// stitched seam takes points at most that far off its edge.
pub const SEAM_ASPECT: f64 = 0.02;

/// The stitch for a seam: a rim whose points all lie within `width` (or
/// [`SEAM_ASPECT`] of its length, never past the object's planarity
/// tolerance `cap`) of the line through its two farthest points, each
/// side running steadily along it. Each side's points are let into the
/// other side's edges; a point of one side at the very place of a point
/// of the other (a copy the split made) becomes that point again.
fn stitch(soup: &Soup, uses: &Uses, rim: &SoupRim, width: f64, cap: f64) -> Option<Patch> {
    let n = rim.points.len();
    if n < 3 || !rim.closed || rim.branching {
        return None;
    }
    let p = |i: usize| soup.pos[rim.points[i]];
    let far = |from: usize| {
        (0..n)
            .max_by(|&a, &b| {
                (p(a) - p(from))
                    .length()
                    .total_cmp(&(p(b) - p(from)).length())
                    .then(b.cmp(&a))
            })
            .unwrap_or(from)
    };
    let ia = far(0);
    let ib = far(ia);
    if ia == ib {
        return None;
    }
    let (a, dir) = (p(ia), p(ib) - p(ia));
    let len2 = dir.length_squared();
    let len = len2.sqrt();
    let width = width.max(SEAM_ASPECT * len).min(cap);
    let t = |i: usize| (p(i) - a).dot(dir) / len2;
    for i in 0..n {
        let q = p(i);
        if (q - (a + dir * t(i))).length() > width {
            return None;
        }
    }
    // Side one runs ia → ib, side two ib → ia (indices along the rim).
    let walk = |from: usize, to: usize| {
        let mut v = vec![from];
        let mut i = from;
        while i != to {
            i = (i + 1) % n;
            v.push(i);
        }
        v
    };
    let one = walk(ia, ib);
    let two = walk(ib, ia);
    let rising = one.windows(2).all(|w| t(w[1]) > t(w[0]));
    let falling = two.windows(2).all(|w| t(w[1]) < t(w[0]));
    if !rising || !falling {
        return None;
    }
    // Every point by its place along the seam; points at one place must be
    // one point (a split's copies), one from each side or an end.
    let close = |x: usize, y: usize| (p(x) - p(y)).length() <= tol::POINT_MERGE;
    let mut placed: Vec<(f64, u8, usize)> = vec![(t(ia), 0, ia), (t(ib), 0, ib)];
    placed.extend(one[1..one.len() - 1].iter().map(|&i| (t(i), 1, i)));
    placed.extend(two[1..two.len() - 1].iter().map(|&i| (t(i), 2, i)));
    placed.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut rejoin: Vec<(usize, usize)> = Vec::new();
    let mut merged: BTreeSet<usize> = BTreeSet::new();
    for w in placed.windows(2) {
        let ((t0, s0, i0), (t1, s1, i1)) = (w[0], w[1]);
        if (t1 - t0) * len > tol::POINT_MERGE {
            continue;
        }
        if !close(i0, i1) || (s0 == s1 && s0 != 0) || merged.contains(&i0) {
            return None;
        }
        // The later one becomes the earlier (an end, or side one's).
        let (keep, gone) = if s1 == 0 { (i1, i0) } else { (i0, i1) };
        rejoin.push((rim.points[gone], rim.points[keep]));
        merged.insert(gone);
    }
    let mut out = Vec::new();
    for (side, other_side, up) in [(&one, &two, true), (&two, &one, false)] {
        let other: Vec<usize> = other_side[1..other_side.len() - 1]
            .iter()
            .copied()
            .filter(|i| !merged.contains(i))
            .collect();
        for w in side.windows(2) {
            let (t0, t1) = (t(w[0]), t(w[1]));
            let mut between: Vec<usize> = other
                .iter()
                .copied()
                .filter(|&i| {
                    let ti = t(i);
                    let inside = if up {
                        ti > t0 && ti < t1
                    } else {
                        ti < t0 && ti > t1
                    };
                    // Not at the place of this edge's own ends.
                    inside && !close(i, w[0]) && !close(i, w[1])
                })
                .collect();
            if between.is_empty() {
                continue;
            }
            between.sort_by(|&x, &y| {
                let o = t(x).total_cmp(&t(y));
                if up { o } else { o.reverse() }
            });
            let c = rim.corners[w[0]];
            let k = uses.corners[c];
            out.push((
                k.f,
                k.r,
                rim.points[w[0]],
                rim.points[w[1]],
                between.into_iter().map(|i| rim.points[i]).collect(),
            ));
        }
    }
    Some(Patch {
        rejoin,
        cuts: out,
        ..Patch::default()
    })
}

/// The minimum-weight triangulation of the fill polygon `q` (the rim
/// reversed): each triangle weighs its area plus, against each neighbor
/// (a fill triangle, or across a polygon edge the face beside the rim),
/// [`FILL_DIHEDRAL_WEIGHT`] × extent² × (1 − cos of the bend). Ties go to
/// the lower split point. `beside[j]` is the normal of the face across
/// polygon edge `q[j] → q[j + 1]`. A chord `taken(i, k)` says is an edge
/// already (the two points are joined elsewhere on the object) is never
/// used: it would be an edge of three faces. `None` when every
/// triangulation needs such a chord.
fn min_weight_triangulation(
    q: &[Point3],
    beside: &[Vec3],
    extent: f64,
    taken: &dyn Fn(usize, usize) -> bool,
) -> Option<Vec<[usize; 3]>> {
    let n = q.len();
    let lambda = FILL_DIHEDRAL_WEIGHT * extent * extent;
    let tri_normal = |i: usize, m: usize, k: usize| {
        (q[m] - q[i])
            .cross(q[k] - q[i])
            .normalized()
            .unwrap_or(Vec3::ZERO)
    };
    let area = |i: usize, m: usize, k: usize| 0.5 * (q[m] - q[i]).cross(q[k] - q[i]).length();
    let bend = |a: Vec3, b: Vec3| {
        if a == Vec3::ZERO || b == Vec3::ZERO {
            2.0
        } else {
            1.0 - a.dot(b)
        }
    };
    let mut w = vec![0.0f64; n * n];
    let mut apex = vec![usize::MAX; n * n];
    let at = |i: usize, k: usize| i * n + k;
    // The normal of whatever lies across chord (i, k) inside sub-polygon
    // i..=k: the polygon edge's neighbor for k = i + 1, else the chosen
    // triangle.
    for span in 2..n {
        for i in 0..n - span {
            let k = i + span;
            let mut best = (f64::INFINITY, usize::MAX);
            if !(i == 0 && k == n - 1) && taken(i, k) {
                w[at(i, k)] = best.0;
                continue;
            }
            for m in i + 1..k {
                let nt = tri_normal(i, m, k);
                let across = |a: usize, b: usize, apex: &[usize]| {
                    if b == a + 1 {
                        beside[a]
                    } else {
                        match apex[at(a, b)] {
                            usize::MAX => Vec3::ZERO,
                            o => tri_normal(a, o, b),
                        }
                    }
                };
                // The last triangle also meets the face across q[n-1] → q[0].
                let closing = if i == 0 && k == n - 1 {
                    bend(nt, beside[n - 1])
                } else {
                    0.0
                };
                let cost = w[at(i, m)]
                    + w[at(m, k)]
                    + area(i, m, k)
                    + lambda
                        * (bend(nt, across(i, m, &apex)) + bend(nt, across(m, k, &apex)) + closing);
                if cost < best.0 {
                    best = (cost, m);
                }
            }
            w[at(i, k)] = best.0;
            apex[at(i, k)] = best.1;
        }
    }
    if !w[at(0, n - 1)].is_finite() {
        return None;
    }
    let mut out = Vec::with_capacity(n - 2);
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, k)) = stack.pop() {
        if k < i + 2 {
            continue;
        }
        let m = apex[at(i, k)];
        out.push([i, m, k]);
        stack.push((i, m));
        stack.push((m, k));
    }
    Some(out)
}

/// What a rim is, judged as [`health::diagnose`] judges rims: its extent,
/// its plane (within the planarity tolerance), whether a flat face beside
/// it lies on that plane facing its way (the edge of a flat part).
struct RimShape {
    extent: f64,
    plane: Option<Plane>,
    sheet_edge: bool,
}

fn rim_shape(soup: &Soup, uses: &Uses, rim: &SoupRim, tol: f64) -> RimShape {
    let pts: Vec<Point3> = rim.points.iter().map(|&i| soup.pos[i]).collect();
    let plane = health::planar_fit(&pts, tol);
    let sheet_edge = plane.is_some_and(|pl| {
        rim.corners.iter().any(|&c| {
            let f = uses.corners[c].f;
            soup.normal(f).dot(pl.normal()) > 0.0
                && soup.faces[f].rings[0]
                    .iter()
                    .all(|&i| pl.signed_distance(soup.pos[i]).abs() <= tol)
        })
    });
    RimShape {
        extent: health::diameter(&pts),
        plane,
        sheet_edge,
    }
}

/// The patch for one rim, or why it gets none.
#[allow(clippy::too_many_arguments)]
fn patch_for(
    soup: &mut Soup,
    uses: &Uses,
    rim: &SoupRim,
    shape: &RimShape,
    stitch_width: f64,
    cap: f64,
    budget: &mut f64,
) -> Result<Patch, RepairError> {
    if !rim.closed || rim.branching {
        return Err(RepairError::NonManifoldRim);
    }
    let n = rim.points.len();
    if n < 3 {
        return Err(RepairError::FillRimTooSmall);
    }
    if let Some(p) = stitch(soup, uses, rim, stitch_width, cap) {
        return Ok(p);
    }
    // The fill polygon runs against the rim.
    let ring: Vec<usize> = std::iter::once(rim.points[0])
        .chain(rim.points[1..].iter().rev().copied())
        .collect();
    let q: Vec<Point3> = ring.iter().map(|&i| soup.pos[i]).collect();
    if shape.plane.is_some()
        && !shape.sheet_edge
        && crate::geom2d::polygon_is_simple(&q)
        && buildable(&q)
    {
        return Ok(Patch::faces(vec![ring], 0));
    }
    // Polygon edge j runs q[j] → q[j+1]: against rim edge from
    // points[n-j-1]... the face beside it is that rim corner's.
    let beside: Vec<Vec3> = (0..n)
        .map(|j| {
            // q[j] = points[(n - j) % n]; edge q[j] → q[j+1] twins rim edge
            // points[(n - j - 1) % n] → points[(n - j) % n].
            let c = rim.corners[(n - j - 1) % n];
            soup.normal(uses.corners[c].f)
        })
        .collect();
    let fill_normal = newell(&q).normalized().unwrap_or(Vec3::ZERO);
    // The triangulation's work (about n³/6 steps) comes out of the Mend's
    // whole budget; past it, a fan (linear) does instead.
    let work = (n as f64).powi(3) / 6.0;
    let fan = !(n <= FILL_MWT_MAX_EDGES && work <= *budget);
    let tris: Vec<Vec<usize>> = if !fan {
        *budget -= work;
        let taken = |i: usize, k: usize| {
            let (a, b) = (ring[i].min(ring[k]), ring[i].max(ring[k]));
            let at = uses.edges.partition_point(|e| (e.0, e.1) < (a, b));
            uses.edges.get(at).is_some_and(|e| (e.0, e.1) == (a, b))
        };
        min_weight_triangulation(&q, &beside, shape.extent, &taken)
            .ok_or(RepairError::FillCrossesFaces)?
            .into_iter()
            .map(|[i, m, k]| vec![ring[i], ring[m], ring[k]])
            .collect()
    } else {
        let c = q.iter().fold(Vec3::ZERO, |s, p| s + p.to_vec()) * (1.0 / n as f64);
        soup.pos.push(Point3::ORIGIN + c);
        soup.origin.push(usize::MAX);
        let center = soup.pos.len() - 1;
        (0..n)
            .map(|j| vec![center, ring[j], ring[(j + 1) % n]])
            .collect()
    };
    for t in &tris {
        let pts: Vec<Point3> = t.iter().map(|&i| soup.pos[i]).collect();
        if !buildable(&pts) {
            return Err(RepairError::FillRimTooSmall);
        }
        // A fan triangle turned against the patch as a whole is a fold. A
        // triangulation's may face away from the mean of a strongly bent
        // rim and still be right (a tent over a folded card); the contact
        // guard is what catches one that crosses.
        if fan && newell(&pts).dot(fill_normal) <= 0.0 {
            return Err(RepairError::FillCrossesFaces);
        }
    }
    let n = tris.len();
    Ok(Patch::faces(tris, n))
}

/// A rim's loops and the `(gone, kept)` rejoins that make them.
type Decomposed = (Vec<SoupRim>, Vec<(usize, usize)>);

/// The most places a rim may come back to before Mend gives up on it
/// (refused as branching): each costs a pass over the rest of the rim.
const DECOMPOSE_MAX_PARTS: usize = 64;

/// [`decompose`]'s work bound, points visited summed over its passes: a
/// short rim may come back to itself as often as it likes within it.
const DECOMPOSE_MAX_WORK: usize = 1 << 20;

/// A rim that passes one place twice (two copies the split made of a
/// point where its fans met) as the loops it makes once those copies are
/// one point again, with the `(gone, kept)` pairs that make them one.
/// `None` past [`DECOMPOSE_MAX_PARTS`] loops.
fn decompose(soup: &Soup, rim: &SoupRim) -> Option<Decomposed> {
    let key = |v: usize| {
        let p = soup.pos[v];
        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
    };
    let mut parts = Vec::new();
    let mut rejoin = Vec::new();
    let mut rest = rim.clone();
    loop {
        let n = rest.points.len();
        if !rest.closed || rest.branching {
            parts.push(rest);
            return Some((parts, rejoin));
        }
        // The first place the loop comes back to, in one pass by exact
        // position (a split's copies sit at one place to the bit). The
        // loop up to it repeats nothing; the rest may.
        let mut first_at: BTreeMap<[u64; 3], usize> = BTreeMap::new();
        let mut found = None;
        for j in 0..n {
            if let Some(&i) = first_at.get(&key(rest.points[j])) {
                found = Some((i, j));
                break;
            }
            first_at.insert(key(rest.points[j]), j);
        }
        let Some((i, j)) = found else {
            parts.push(rest);
            return Some((parts, rejoin));
        };
        if parts.len() >= DECOMPOSE_MAX_PARTS.max(DECOMPOSE_MAX_WORK / n.max(1)) {
            return None;
        }
        let (a, b) = (rest.points[i], rest.points[j]);
        let mut points = vec![a];
        points.extend(rest.points[i + 1..j].iter().copied());
        parts.push(SoupRim {
            corners: rest.corners[i..j].to_vec(),
            points,
            branching: false,
            closed: true,
        });
        // j..i round the end, starting at b, which becomes a.
        let mut points = vec![a];
        let mut corners = Vec::new();
        for k in j..n + i {
            if k > j {
                points.push(rest.points[k % n]);
            }
            corners.push(rest.corners[k % n]);
        }
        rejoin.push((b, a));
        rest = SoupRim {
            corners,
            points,
            branching: false,
            closed: true,
        };
    }
}

/// One simple rim's patch: a stitch for a seam, else (the edge of a flat
/// part refused under the one-click scope) a face or triangles.
#[allow(clippy::too_many_arguments)]
fn patch_one(
    soup: &mut Soup,
    uses: &Uses,
    rim: &SoupRim,
    stitch_width: f64,
    cap: f64,
    holes_scope: bool,
    budget: &mut f64,
) -> Result<Patch, RepairError> {
    if let Some(p) = stitch(soup, uses, rim, stitch_width, cap) {
        return Ok(p);
    }
    let shape = rim_shape(soup, uses, rim, cap);
    // The edge of a flat part takes no face under the one-click scope (a
    // face there would lie on that part's back).
    if shape.sheet_edge && holes_scope {
        return Err(RepairError::CapWouldBeCoplanar);
    }
    patch_for(soup, uses, rim, &shape, stitch_width, cap, budget)
}

/// Apply patches `(tag, patch)` to a copy of `faces` (an island's faces),
/// returning the island's faces with the fill faces appended, and how
/// many zero-width folds a stitch cut out of a ring. A stitch that no
/// longer finds its edge fails with its tag; so does one that would put a
/// point into a ring twice, unless the ring's way out from that point and
/// back is a fold of no width (no wider than `width`, or
/// [`SEAM_ASPECT`] of its length), which is cut out of the ring.
fn apply_patches(
    soup: &Soup,
    faces: &[usize],
    patches: &[(usize, Patch)],
    width: f64,
    cap: f64,
    folds_on: bool,
) -> Result<(Vec<SFace>, usize), usize> {
    let mut folds = 0;
    let mut out: Vec<SFace> = faces.iter().map(|&f| soup.faces[f].clone()).collect();
    let local: BTreeMap<usize, usize> = faces.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    // Every rejoin so far, across patches: a later patch's points may name
    // a copy an earlier one made one again.
    let mut map: BTreeMap<usize, usize> = BTreeMap::new();
    for (tag, patch) in patches {
        for &(gone, kept) in &patch.rejoin {
            if gone != kept {
                map.insert(gone, kept);
            }
        }
        let to = |mut v: usize| {
            // Follow the chain (each step to a point rejoined earlier).
            for _ in 0..=map.len() {
                match map.get(&v) {
                    Some(&k) => v = k,
                    None => break,
                }
            }
            v
        };
        if !patch.rejoin.is_empty() {
            for face in out.iter_mut() {
                let mut hit = false;
                for ring in face.rings.iter_mut() {
                    for v in ring.iter_mut() {
                        let k = to(*v);
                        if k != *v {
                            *v = k;
                            hit = true;
                        }
                    }
                }
                if hit {
                    face.plane = None;
                    face.fill = Some(*tag);
                }
            }
        }
        for (f, r, a, b, between) in &patch.cuts {
            let (a, b) = (to(*a), to(*b));
            let between: Vec<usize> = between.iter().map(|&v| to(v)).collect();
            let face = &mut out[local[f]];
            if between.iter().any(|v| {
                face.rings
                    .iter()
                    .enumerate()
                    .any(|(k, ring)| k != *r && ring.contains(v))
            }) {
                return Err(*tag);
            }
            let ring = &mut face.rings[*r];
            let len = ring.len();
            let Some(j) = (0..len).find(|&j| ring[j] == a && ring[(j + 1) % len] == b) else {
                return Err(*tag);
            };
            for (o, &v) in between.iter().enumerate() {
                ring.insert(j + 1 + o, v);
            }
            if folds_on {
                folds += cut_folds(&soup.pos, ring, width, cap).ok_or(*tag)?;
            }
            if ring.len() < 3 {
                return Err(*tag);
            }
            face.plane = None;
            face.fill = Some(*tag);
        }
        for r in &patch.rings {
            out.push(SFace {
                rings: vec![r.iter().map(|&v| to(v)).collect()],
                src: None,
                plane: None,
                fill: Some(*tag),
            });
        }
    }
    Ok((out, folds))
}

/// Cut out of `ring` every way out from a point and back to it that has
/// no width (all its points within `width`, or [`SEAM_ASPECT`] of its
/// length, of one line): a fold. How many went; `None` when a point
/// comes round twice any other way.
fn cut_folds(pos: &[Point3], ring: &mut Vec<usize>, width: f64, cap: f64) -> Option<usize> {
    // As wide as a seam may be: the weld distance or SEAM_ASPECT of the
    // fold's length, never past the planarity tolerance (a real tongue of
    // surface is never cut as a fold).
    let thin =
        |pts: &[Point3]| thinner_than(pts, width.max(SEAM_ASPECT * health::diameter(pts)).min(cap));
    let mut cut = 0;
    loop {
        // The first point the ring comes back to (one pass).
        let mut first_at: BTreeMap<usize, usize> = BTreeMap::new();
        let mut found = None;
        for (j, &v) in ring.iter().enumerate() {
            if let Some(&i) = first_at.get(&v) {
                found = Some((i, j));
                break;
            }
            first_at.insert(v, j);
        }
        let Some((i, j)) = found else {
            return Some(cut);
        };
        let inner: Vec<Point3> = ring[i..j].iter().map(|&v| pos[v]).collect();
        let outer: Vec<Point3> = ring[j..]
            .iter()
            .chain(&ring[..i])
            .map(|&v| pos[v])
            .collect();
        if thin(&inner) {
            // Out from ring[i] and back at ring[j]: drop i+1..=j.
            ring.drain(i + 1..=j);
        } else if thin(&outer) {
            // The rest of the ring is the fold: keep i..j.
            *ring = ring[i..j].to_vec();
        } else {
            return None;
        }
        cut += 1;
    }
}

// ─────────────────────────────────────────────────────────────── rebuild

/// Build `faces` (soup faces) as an object: points compacted in index
/// order, each face's attributes from its source, an unchanged face's
/// plane kept exactly, `claims` reapplied, `source`'s object-level state
/// carried. With the face handles in `faces` order.
fn build(
    pos: &[Point3],
    faces: &[SFace],
    source: &Object,
    claims: &EdgeClaims,
) -> Result<(Object, Vec<FaceId>, Vec<usize>), TopologyError> {
    let mut used: Vec<usize> = faces
        .iter()
        .flat_map(|f| f.rings.iter().flatten())
        .copied()
        .collect();
    used.sort_unstable();
    used.dedup();
    let mut local = vec![usize::MAX; pos.len()];
    for (i, &v) in used.iter().enumerate() {
        local[v] = i;
    }
    let positions: Vec<Point3> = used.iter().map(|&v| pos[v]).collect();
    let remap = |ring: &Vec<usize>| ring.iter().map(|&v| local[v]).collect::<Vec<_>>();
    let outers: Vec<Vec<usize>> = faces.iter().map(|f| remap(&f.rings[0])).collect();
    let holes: Vec<Vec<Vec<usize>>> = faces
        .iter()
        .map(|f| f.rings[1..].iter().map(remap).collect())
        .collect();
    let attr = |f: &SFace| f.src.map(|s| &source.faces[s]);
    let materials: Vec<_> = faces
        .iter()
        .map(|f| attr(f).and_then(|s| s.material))
        .collect();
    let uvs: Vec<_> = faces
        .iter()
        .map(|f| attr(f).and_then(|s| s.uv_frame))
        .collect();
    let surfaces: Vec<_> = faces
        .iter()
        .map(|f| attr(f).and_then(|s| s.surface))
        .collect();
    let mut obj = Object::from_polygons_full(
        &positions,
        &outers,
        &holes,
        &materials,
        &uvs,
        &surfaces,
        source.planarity_tol,
    )?;
    let backs: Vec<_> = faces
        .iter()
        .map(|f| attr(f).and_then(|s| s.back_material))
        .collect();
    obj.set_built_back_materials(&backs);
    let ids: Vec<FaceId> = obj.faces.keys().collect();
    for (id, f) in ids.iter().zip(faces) {
        if let Some(plane) = f.plane {
            obj.faces[*id].plane = plane;
        }
        // A face keeps the attributes of the face it came from.
        if let Some(dict) = f.src.and_then(|s| source.face_attrs(s)).cloned() {
            obj.set_face_attrs(*id, dict);
        }
    }
    apply_edge_claims(&mut obj, claims);
    obj.default_material = source.default_material;
    Ok((obj, ids, used))
}

/// One island filled and built: the object, the rims filled, stitched and
/// skipped (with their first point and reason), the triangles added.
struct Built {
    obj: Object,
    /// Each of `obj`'s points (in storage order) as its soup point.
    used: Vec<usize>,
    /// Zero-width folds cut out of rings while stitching.
    folds: usize,
    filled: usize,
    stitched: usize,
    /// Of `filled`, the rims closed with fill triangles (a bent hole).
    curved: usize,
    /// Each filled rim's points (soup indices) in boundary order.
    filled_points: Vec<Vec<usize>>,
    triangles: usize,
    skipped: Vec<((usize, usize), RepairError)>,
}

#[allow(clippy::too_many_arguments)]
fn fill_and_build(
    soup: &mut Soup,
    faces: &[usize],
    source: &Object,
    claims: &EdgeClaims,
    plan: &MendPlan,
    named: &BTreeSet<(usize, usize)>,
    rank: &[usize],
    stitch_width: f64,
    budget: &mut f64,
) -> Result<Built, RepairError> {
    let mut patches: Vec<(usize, Patch)> = Vec::new();
    let mut skipped: Vec<((usize, usize), RepairError)> = Vec::new();
    let mut rim_edge: Vec<(usize, usize)> = Vec::new();
    let mut rim_points: Vec<Vec<usize>> = Vec::new();
    if let Some(scope) = &plan.fill {
        let uses = Uses::of(soup, faces);
        let mut found = rims(soup, &uses, rank);
        // Smallest first, then by where the rim lies point by point: the
        // order the triangulation budget is spent in follows the geometry
        // alone, never the order the faces came in.
        found.sort_by(|a, b| {
            a.points.len().cmp(&b.points.len()).then_with(|| {
                a.points
                    .iter()
                    .zip(&b.points)
                    .map(|(&x, &y)| cmp_point(soup.pos[x], soup.pos[y]))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let island_pts: Vec<Point3> = {
            let mut vs: Vec<usize> = faces
                .iter()
                .flat_map(|&f| soup.faces[f].rings.iter().flatten())
                .copied()
                .collect();
            vs.sort_unstable();
            vs.dedup();
            vs.into_iter().map(|v| soup.pos[v]).collect()
        };
        let island_extent = health::diameter(&island_pts);
        for rim in &found {
            let edge = (
                rim.points[0],
                rim.points.get(1).copied().unwrap_or(rim.points[0]),
            );
            let shape = rim_shape(soup, &uses, rim, source.planarity_tol);
            let role = if shape.extent >= health::OUTER_RIM_FRACTION * island_extent {
                RimRole::Outer
            } else {
                RimRole::Hole
            };
            // A seam (no wider than the stitch width) has no area to be a
            // surface's edge: the one-click scope stitches it whatever its
            // size.
            let seam = stitch(soup, &uses, rim, stitch_width, source.planarity_tol).is_some();
            let wanted = match scope {
                FillScope::Holes => {
                    // An outer edge is no gap: left as it is, unreported.
                    if role == RimRole::Outer && !seam {
                        continue;
                    }
                    true
                }
                FillScope::Rims(_) => rim.corners.iter().any(|&c| {
                    let (a, b) = uses.ends(soup, c);
                    named.contains(&(soup.origin[a], soup.origin[b]))
                }),
            };
            if !wanted {
                continue;
            }
            let holes_scope = matches!(scope, FillScope::Holes);
            let Some((parts, rejoin)) = decompose(soup, rim) else {
                skipped.push((edge, RepairError::NonManifoldRim));
                continue;
            };
            let mut patch = Patch {
                rejoin,
                ..Patch::default()
            };
            let mut refused = None;
            for part in &parts {
                // A loop of two edges closes by its points becoming one.
                if part.points.len() == 2 {
                    continue;
                }
                match patch_one(
                    soup,
                    &uses,
                    part,
                    stitch_width,
                    source.planarity_tol,
                    holes_scope,
                    budget,
                ) {
                    Ok(p) => {
                        patch.rejoin.extend(p.rejoin);
                        patch.cuts.extend(p.cuts);
                        patch.triangles += p.triangles;
                        patch.rings.extend(p.rings);
                    }
                    Err(e) => {
                        refused = Some(e);
                        break;
                    }
                }
            }
            match refused {
                None => {
                    patches.push((rim_edge.len(), patch));
                    rim_edge.push(edge);
                    rim_points.push(rim.points.clone());
                }
                Some(e) => skipped.push((edge, e)),
            }
        }
    }
    // Build; a patch the builder or the contact guard refuses is left out
    // and the island built again.
    let mut cut_folds_total;
    loop {
        let island = match apply_patches(
            soup,
            faces,
            &patches,
            stitch_width,
            source.planarity_tol,
            plan.folds,
        ) {
            Ok((f, folds)) => {
                cut_folds_total = folds;
                f
            }
            Err(tag) => {
                drop_patch(
                    &mut patches,
                    &mut skipped,
                    &rim_edge,
                    tag,
                    RepairError::FillCrossesFaces,
                );
                continue;
            }
        };
        let tag_of_face = |i: usize| island.get(i).and_then(|f| f.fill);
        let (obj, ids, used) = match build(&soup.pos, &island, source, claims) {
            Ok(b) => b,
            Err(e) => {
                let face = match e {
                    TopologyError::DegenerateFace { face }
                    | TopologyError::NonPlanarFace { face }
                    | TopologyError::InvalidVertexIndex { face, .. } => Some(face),
                    _ => None,
                };
                match face.and_then(tag_of_face) {
                    Some(tag) => {
                        let reason = if matches!(e, TopologyError::NonPlanarFace { .. }) {
                            RepairError::RimNotPlanar
                        } else {
                            RepairError::FillRimTooSmall
                        };
                        drop_patch(&mut patches, &mut skipped, &rim_edge, tag, reason);
                        continue;
                    }
                    None if !patches.is_empty() => {
                        // An edge the fills made crowded names its points,
                        // not a face: leave out the patches whose faces
                        // run along it. Anything else names nothing, so
                        // every patch goes (one rebuild, never a guess).
                        let mut tags: BTreeSet<usize> = BTreeSet::new();
                        if let TopologyError::NonManifoldEdge { from, to } = e {
                            let mut used: Vec<usize> = island
                                .iter()
                                .flat_map(|f| f.rings.iter().flatten())
                                .copied()
                                .collect();
                            used.sort_unstable();
                            used.dedup();
                            if let (Some(&a), Some(&b)) = (used.get(from), used.get(to)) {
                                for f in &island {
                                    let Some(tag) = f.fill else { continue };
                                    let along = f.rings.iter().any(|r| {
                                        let n = r.len();
                                        (0..n).any(|k| {
                                            let (x, y) = (r[k], r[(k + 1) % n]);
                                            (x, y) == (a, b) || (x, y) == (b, a)
                                        })
                                    });
                                    if along {
                                        tags.insert(tag);
                                    }
                                }
                            }
                        }
                        if tags.is_empty() {
                            tags = patches.iter().map(|(t, _)| *t).collect();
                        }
                        for tag in tags {
                            drop_patch(
                                &mut patches,
                                &mut skipped,
                                &rim_edge,
                                tag,
                                RepairError::FillCrossesFaces,
                            );
                        }
                        continue;
                    }
                    None => {
                        tracing::warn!(target: "kernel::op", error = %e, "mend: the soup did not rebuild");
                        return Err(RepairError::Invalid);
                    }
                }
            }
        };
        // New fill faces only: a stitch moves no point and changes a face
        // by a sliver no wider than its seam, so whatever such a face
        // touches it touched before (a fold beside it included).
        let candidates: Vec<FaceId> = ids
            .iter()
            .zip(&island)
            .filter(|(_, f)| f.fill.is_some() && f.src.is_none())
            .map(|(&id, _)| id)
            .collect();
        let contacts = improper_contacts(&obj, &candidates);
        if !contacts.is_empty() {
            let mut bad: BTreeSet<usize> = BTreeSet::new();
            for (f, g) in contacts {
                let tag = [f, g]
                    .iter()
                    .find_map(|x| ids.iter().position(|y| y == x).and_then(tag_of_face))
                    .expect("a contact involves a candidate");
                bad.insert(tag);
            }
            for tag in bad {
                drop_patch(
                    &mut patches,
                    &mut skipped,
                    &rim_edge,
                    tag,
                    RepairError::FillCrossesFaces,
                );
            }
            continue;
        }
        let (mut filled, mut stitched, mut curved, mut triangles) = (0, 0, 0, 0);
        for (_, p) in &patches {
            filled += 1;
            if p.stitched() {
                stitched += 1;
            }
            if p.triangles > 0 {
                curved += 1;
            }
            triangles += p.triangles;
        }
        return Ok(Built {
            used,
            folds: cut_folds_total,
            obj,
            filled,
            stitched,
            curved,
            filled_points: patches
                .iter()
                .map(|(t, _)| rim_points[*t].clone())
                .collect(),
            triangles,
            skipped,
        });
    }
}

fn drop_patch(
    patches: &mut Vec<(usize, Patch)>,
    skipped: &mut Vec<((usize, usize), RepairError)>,
    rim_edge: &[(usize, usize)],
    tag: usize,
    reason: RepairError,
) {
    let before = patches.len();
    patches.retain(|(t, _)| *t != tag);
    if patches.len() < before {
        skipped.push((rim_edge[tag], reason));
    }
}

/// What [`Object::mend_front`] leaves for the fill.
struct Front {
    soup: Soup,
    weld: Option<WeldPlan>,
    collapsed: usize,
    folds: usize,
    /// Edges the weld's dropped slivers and the fold step's faces leave
    /// open (seams for the fill): what the open-edge check allows for.
    opened_edges: usize,
    split: Option<SplitStats>,
}

/// The soup's points (by exact position) where the boundary branches:
/// more than one open edge leaves them.
fn branch_points(soup: &Soup) -> BTreeSet<[u64; 3]> {
    let all: Vec<usize> = (0..soup.faces.len()).collect();
    let uses = Uses::of(soup, &all);
    let mut leaving: BTreeMap<usize, usize> = BTreeMap::new();
    for g in uses.groups() {
        if g.len() == 1 {
            *leaving.entry(uses.ends(soup, g[0].3).0).or_default() += 1;
        }
    }
    leaving
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(v, _)| {
            let p = soup.pos[v];
            [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
        })
        .collect()
}

/// An edge's endpoints as exact position bits, order-free.
type ClaimKey = ([u64; 3], [u64; 3]);

/// Marked edges by endpoints: curve claim, polygon claim, soft, and the
/// edge's attribute dictionary.
type EdgeClaims = BTreeMap<
    ClaimKey,
    (
        Option<crate::sketch::CurveGeom>,
        Option<crate::sketch::CurveGeom>,
        bool,
        Option<crate::attr::AttrDict>,
    ),
>;

/// Whether an edge carries anything a rebuild has to give back.
fn edge_is_marked(obj: &Object, id: crate::ids::EdgeId) -> bool {
    let e = &obj.edges[id];
    e.curve.is_some() || e.polygon.is_some() || e.soft || obj.edge_attrs(id).is_some()
}

fn claim_key(a: Point3, b: Point3) -> ClaimKey {
    let k = |p: Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let (ka, kb) = (k(a), k(b));
    if ka <= kb { (ka, kb) } else { (kb, ka) }
}

/// `obj`'s marked edges keyed by their exact endpoint positions: what
/// [`apply_edge_claims`] gives back to the rebuilt object.
fn edge_claims(obj: &Object) -> EdgeClaims {
    obj.edges
        .keys()
        .filter_map(|e| {
            let (a, b) = obj.edge_endpoints(e)?;
            let edge = &obj.edges[e];
            edge_is_marked(obj, e).then(|| {
                (
                    claim_key(a, b),
                    (
                        edge.curve,
                        edge.polygon,
                        edge.soft,
                        obj.edge_attrs(e).cloned(),
                    ),
                )
            })
        })
        .collect()
}

/// Mark each edge of `obj` whose endpoints match a claim in `claims`. A
/// soft flag needs an interior edge, so it only carries onto one.
fn apply_edge_claims(obj: &mut Object, claims: &EdgeClaims) {
    if claims.is_empty() {
        return;
    }
    let ids: Vec<crate::ids::EdgeId> = obj.edges.keys().collect();
    for e in ids {
        let Some((a, b)) = obj.edge_endpoints(e) else {
            continue;
        };
        if let Some((curve, polygon, soft, attrs)) = claims.get(&claim_key(a, b)) {
            let edge = &mut obj.edges[e];
            edge.curve = *curve;
            edge.polygon = *polygon;
            edge.soft = *soft && edge.twin_half_edge.is_some();
            if let Some(dict) = attrs {
                obj.set_edge_attrs(e, dict.clone());
            }
        }
    }
}

/// This object's edge marks keyed by where their points stand after the
/// weld (each point at its root's place), so an edge through a merged
/// point keeps its curve, polygon and soft marks.
fn remapped_claims(
    obj: &Object,
    index: &SecondaryMap<VertexId, usize>,
    root: Option<&[usize]>,
    pos: &[Point3],
) -> EdgeClaims {
    let Some(root) = root else {
        return edge_claims(obj);
    };
    let at = |v: VertexId| pos[root[index[v]]];
    obj.edges
        .iter()
        .filter(|&(id, _)| edge_is_marked(obj, id))
        .filter_map(|(id, e)| {
            let h = &obj.half_edges[e.half_edge];
            let (a, b) = (h.origin, obj.half_edges[h.next].origin);
            let (pa, pb) = (at(a), at(b));
            (pa != pb).then(|| {
                (
                    claim_key(pa, pb),
                    (e.curve, e.polygon, e.soft, obj.edge_attrs(id).cloned()),
                )
            })
        })
        .collect()
}

/// Whether faces `f` and `g` of `obj` lie on one plane facing one way, in
/// one paint: the test that makes the edge between them drawing
/// (`document::draft`'s `face_on_plane`, with materials).
fn coplanar_pair(obj: &Object, f: FaceId, g: FaceId) -> bool {
    let (a, b) = (&obj.faces[f], &obj.faces[g]);
    (a.plane.normal() - b.plane.normal()).length() < tol::NORMAL_DIRECTION
        && obj
            .loop_positions(b.outer_loop)
            .all(|p| a.plane.signed_distance(p).abs() <= tol::PLANE_DIST)
        && a.material == b.material
        && a.back_material == b.back_material
}

/// The hard edges of `obj` between two faces of [`coplanar_pair`], as
/// `((a, b), edge)` with their points named through `name`.
fn seam_edges(
    obj: &Object,
    name: impl Fn(VertexId) -> usize,
) -> Vec<((usize, usize), crate::ids::EdgeId)> {
    obj.edges
        .iter()
        .filter_map(|(e, edge)| {
            let t = edge.twin_half_edge?;
            if edge.soft {
                return None;
            }
            let h = edge.half_edge;
            let f = obj.loops[obj.half_edges[h].loop_id].face;
            let g = obj.loops[obj.half_edges[t].loop_id].face;
            if f == g || !coplanar_pair(obj, f, g) {
                return None;
            }
            let a = name(obj.half_edges[h].origin);
            let b = name(obj.half_edges[t].origin);
            Some(((a, b), e))
        })
        .collect()
}

/// [`seam_edges`]'s points, order-free.
fn drawn_seams(obj: &Object, name: impl Fn(VertexId) -> usize) -> BTreeSet<(usize, usize)> {
    seam_edges(obj, name)
        .into_iter()
        .map(|((a, b), _)| (a.min(b), a.max(b)))
        .collect()
}

/// The index of the rim of `obj` ([`health::diagnose`] order) through
/// the directed edge `a → b`, given the positions `a` and `b` stood at.
fn rim_index(d: &health::Diagnosis, obj: &Object, a: Point3, b: Point3) -> Option<usize> {
    d.rims.iter().position(|r| {
        r.half_edges.iter().any(|&h| {
            let he = &obj.half_edges[h];
            obj.vertices[he.origin].position == a
                && obj.vertices[obj.half_edges[he.next].origin].position == b
        })
    })
}

/// A Mend worked through on a copy, nothing committed: what
/// [`Object::mend`] installs and [`Object::mend_census`] counts.
struct MendRun {
    report: MendReport,
    /// The kept object first, then the pieces.
    objects: Vec<Object>,
    weld: Option<WeldPlan>,
    /// The points the split splits, where they stand.
    split_points: Vec<Point3>,
    /// Each filled rim's edges, by the points they started from (a weld
    /// root for a merged point), order-free.
    filled_edges: BTreeSet<(usize, usize)>,
    /// Of the filled rims, the ones closed with fill triangles.
    fill_curved: usize,
}

/// The total area of `obj`'s faces (holes subtracted).
fn surface_area(obj: &Object) -> f64 {
    obj.faces
        .values()
        .map(|f| {
            let area = |l| newell(&obj.loop_positions(l).collect::<Vec<_>>()).length() / 2.0;
            area(f.outer_loop) - f.inner_loops.iter().map(|&l| area(l)).sum::<f64>()
        })
        .sum()
}

/// Open edges (an edge with one face) of `obj`.
fn open_edges(obj: &Object) -> usize {
    obj.edges
        .values()
        .filter(|e| e.twin_half_edge.is_none())
        .count()
}

impl Object {
    /// What [`Object::mend`] would do under `plan`, without doing it: the
    /// Mend itself run on a copy, so every count — points merged, folds,
    /// points and edges split, pieces, rims filled and left open — is the
    /// commit's, and every refusal the commit meets is met here too. Its
    /// cost is the Mend's. Pure.
    pub fn mend_census(&self, plan: &MendPlan) -> Result<MendCensus, RepairError> {
        self.check_mend_plan(plan)?;
        let run = self.mend_run(plan)?;
        let d = health::diagnose(self);
        let pos: Vec<Point3> = self.vertices.values().map(|v| v.position).collect();
        let (mut weld_pairs, mut weld_vertices, mut weld_max_gap) = (0, 0, 0.0);
        let mut weld_points = Vec::new();
        if let Some(w) = &run.weld {
            weld_pairs = w.pairs;
            weld_vertices = w.merged;
            weld_max_gap = w.max_gap;
            // Every point of a merging cluster, its root included.
            let mut merging: BTreeSet<usize> = BTreeSet::new();
            for (i, &r) in w.root.iter().enumerate() {
                if r != i {
                    merging.insert(i);
                    merging.insert(r);
                }
            }
            weld_points = merging.into_iter().map(|i| pos[i]).collect();
            weld_points.sort_by(|a, b| cmp_point(*a, *b));
        }
        let r = &run.report;
        let islands = if plan.split {
            r.pieces + 1
        } else {
            d.findings
                .iter()
                .find_map(|f| match f {
                    health::Finding::DisconnectedIslands { count } => Some(*count),
                    _ => None,
                })
                .unwrap_or(1)
        };
        // The rims (this object's, before the Mend) the fill closes: those
        // with an edge, its points taken to where the weld puts them, among
        // the filled rims' edges.
        let index: SecondaryMap<VertexId, usize> = self
            .vertices
            .keys()
            .enumerate()
            .map(|(i, v)| (v, i))
            .collect();
        let at = |v: VertexId| run.weld.as_ref().map_or(index[v], |w| w.root[index[v]]);
        let fill_rims: Vec<usize> = d
            .rims
            .iter()
            .enumerate()
            .filter(|(_, rim)| {
                rim.half_edges.iter().any(|&h| {
                    let he = &self.half_edges[h];
                    let (a, b) = (at(he.origin), at(self.half_edges[he.next].origin));
                    run.filled_edges.contains(&(a.min(b), a.max(b)))
                })
            })
            .map(|(i, _)| i)
            .collect();
        let mut c = MendCensus {
            weld_pairs,
            weld_vertices,
            folds: r.folds_removed,
            collapsed_faces: r.collapsed_faces,
            split_edges: r.split_edges,
            weld_max_gap,
            branching_vertices: r.split_vertices,
            islands,
            pieces: r.pieces,
            hole_rims: 0,
            hole_rims_planar: 0,
            hole_rims_warped: 0,
            outer_rims: 0,
            weld_points,
            split_points: run.split_points,
            fill_rims,
            filled_rims: r.filled_rims,
            fill_curved: run.fill_curved,
            skipped_rims: r.skipped_rims.len(),
        };
        for rim in &d.rims {
            match rim.role {
                RimRole::Outer => c.outer_rims += 1,
                RimRole::Hole => {
                    c.hole_rims += 1;
                    if rim.plane.is_some() && !rim.sheet_edge {
                        c.hole_rims_planar += 1;
                    } else {
                        c.hole_rims_warped += 1;
                    }
                }
            }
        }
        Ok(c)
    }

    /// The steps before the fill, on `soup` (this object's), with every
    /// refusal they meet: the weld (`WeldCollapsesObject` when nothing is
    /// left, `WeldFlipsFaces` when a face it keeps turns over), the fold
    /// step, then, with the split off, `FoldRemovalPinches` (a point the
    /// folds left where the boundary branches, judged after the weld so a
    /// pinch the weld only moved is not new) and `WeldFoldsFaces` (an edge
    /// still crowded once the folds are gone); then the split.
    fn mend_front(&self, plan: &MendPlan, mut soup: Soup) -> Result<Front, RepairError> {
        let mut weld = None;
        let mut collapsed = 0;
        let mut opened_edges = 0;
        if let Some(d) = plan.weld {
            let w = weld_clusters(&soup.pos, d);
            let c = apply_weld(&mut soup, &w.root)?;
            collapsed = c.count;
            opened_edges += c.open_edges;
            if soup.faces.is_empty() {
                return Err(RepairError::WeldCollapsesObject);
            }
            weld = Some(w);
        }
        let mut folds_removed = 0;
        if plan.folds {
            let before = (!plan.split).then(|| branch_points(&soup));
            let (removed, edges) = remove_folds(&mut soup, fold_width(self, plan));
            folds_removed = removed;
            opened_edges += edges;
            if soup.faces.is_empty() {
                return Err(RepairError::WeldCollapsesObject);
            }
            if let Some(before) = before
                && folds_removed > 0
                && !branch_points(&soup).is_subset(&before)
            {
                return Err(RepairError::FoldRemovalPinches);
            }
        }
        if !plan.split && weld.is_some() {
            let all: Vec<usize> = (0..soup.faces.len()).collect();
            if folds(&soup, &all) {
                return Err(RepairError::WeldFoldsFaces);
            }
        }
        let split = plan.split.then(|| split(&mut soup));
        Ok(Front {
            soup,
            weld,
            collapsed,
            folds: folds_removed,
            opened_edges,
            split,
        })
    }

    /// The refusals a plan meets before any work, shared by the census and
    /// the Mend so a preview never promises what the commit refuses: a
    /// weld distance outside [`weld_range`], a named rim past the object's.
    fn check_mend_plan(&self, plan: &MendPlan) -> Result<(), RepairError> {
        if let Some(w) = plan.weld {
            let (lo, hi) = weld_range(self);
            if !w.is_finite() || w < lo || w > hi {
                return Err(RepairError::WeldDistanceOutOfRange);
            }
        }
        if let Some(FillScope::Rims(ids)) = &plan.fill {
            let n = health::diagnose(self).rims.len();
            if ids.is_empty() || ids.iter().any(|&i| i >= n) {
                return Err(RepairError::UnknownRim);
            }
        }
        Ok(())
    }

    /// Mend this object under `plan` (see the module docs): weld, split,
    /// fill, then one rebuild per island. Returns what happened and the
    /// islands the split cut loose, as objects of their own (the object
    /// keeps the largest).
    ///
    /// # Errors
    /// The Mend as a whole refuses only on
    /// [`RepairError::WeldDistanceOutOfRange`],
    /// [`RepairError::WeldFoldsFaces`] (split off),
    /// [`RepairError::WeldFlipsFaces`],
    /// [`RepairError::WeldCollapsesObject`],
    /// [`RepairError::WeldFlattensObject`],
    /// [`RepairError::MendOpensObject`], [`RepairError::UnknownRim`],
    /// [`RepairError::NothingToMend`], [`RepairError::StillOpen`] (with
    /// `require_solid`) and [`RepairError::Invalid`]; a rim it could not
    /// fill is reported in [`MendReport::skipped_rims`]. When nothing at
    /// all changed but a rim was refused, that rim's reason. On any error
    /// the object is untouched.
    pub fn mend(&mut self, plan: &MendPlan) -> Result<(MendReport, Vec<Object>), RepairError> {
        self.check_mend_plan(plan)?;
        let MendRun {
            report, objects, ..
        } = self.mend_run(plan)?;
        let changed = report.welded_vertices > 0
            || report.collapsed_faces > 0
            || report.folds_removed > 0
            || report.seam_folds > 0
            || report.split_vertices > 0
            || report.split_edges > 0
            || report.pieces > 0
            || report.filled_rims > 0;
        if !changed {
            return Err(report
                .skipped_rims
                .iter()
                .map(|s| s.reason)
                .next()
                .unwrap_or(RepairError::NothingToMend));
        }
        if plan.require_solid && !(report.closed && report.pieces_closed == report.pieces) {
            return Err(RepairError::StillOpen);
        }
        let mut objects = objects.into_iter();
        *self = objects.next().expect("the kept island");
        Ok((report, objects.collect()))
    }

    /// The whole Mend on a copy: the steps, the rebuild, and the checks
    /// that a result is no worse than the object it replaces. A weld that
    /// pinches a narrow part shut can join two points of one face ring and
    /// leave the split to pair what was never one edge: if the object was
    /// a solid and a result is not, or the results have more open edges
    /// than the object had (beyond the seams the dropped slivers and folds
    /// leave for the fill), that is [`RepairError::MendOpensObject`]. A
    /// weld that presses opposite sides of a thin part together leaves a
    /// closed shell around nothing: when a solid's results keep less than
    /// [`FLAT_VOLUME_FRACTION`] of its volume, or a closed result is
    /// thinner on average than [`FLAT_THICKNESS`] of the merge distance,
    /// that is [`RepairError::WeldFlattensObject`].
    fn mend_run(&self, plan: &MendPlan) -> Result<MendRun, RepairError> {
        let (soup, index) = Soup::of(self);
        // The named rims' edges, by the points they started at.
        let named: BTreeSet<(usize, usize)> = match &plan.fill {
            Some(FillScope::Rims(ids)) => {
                let d = health::diagnose(self);
                if ids.is_empty() || ids.iter().any(|&i| i >= d.rims.len()) {
                    return Err(RepairError::UnknownRim);
                }
                ids.iter()
                    .flat_map(|&i| d.rims[i].half_edges.iter())
                    .map(|&h| {
                        let he = &self.half_edges[h];
                        (index[he.origin], index[self.half_edges[he.next].origin])
                    })
                    .collect()
            }
            _ => BTreeSet::new(),
        };
        let mut report = MendReport {
            welded_vertices: 0,
            weld_max_move: 0.0,
            collapsed_faces: 0,
            folds_removed: 0,
            seam_folds: 0,
            soft_seams: 0,
            split_vertices: 0,
            split_edges: 0,
            pieces: 0,
            filled_rims: 0,
            stitched_rims: 0,
            fill_triangles: 0,
            skipped_rims: Vec::new(),
            reversed: false,
            closed: false,
            pieces_closed: 0,
        };
        let mut named = named;
        // The seams the object already draws (two faces on one plane, one
        // paint, joined by a hard edge), by their points: a seam the Mend
        // makes (a sliver gone from between two coplanar faces, a flat
        // fill beside its plane) is made soft, so the Mend adds no drawing.
        let mut drawn = drawn_seams(self, |v| index[v]);
        let front = self.mend_front(plan, soup)?;
        let mut soup = front.soup;
        if let Some(w) = &front.weld {
            drawn = drawn
                .into_iter()
                .map(|(a, b)| (w.root[a].min(w.root[b]), w.root[a].max(w.root[b])))
                .collect();
            named = named
                .into_iter()
                .map(|(a, b)| (w.root[a], w.root[b]))
                .collect();
            report.welded_vertices = w.merged;
            report.weld_max_move = w.max_move;
        }
        report.collapsed_faces = front.collapsed;
        report.folds_removed = front.folds;
        let mut split_points = Vec::new();
        let islands: Vec<Vec<usize>> = if let Some(s) = front.split {
            report.split_vertices = s.vertices;
            report.split_edges = s.edges;
            split_points = s.points.iter().map(|&v| soup.pos[v]).collect();
            let mut isl = islands(&soup);
            if isl.len() > 1 {
                // The largest island stays; ties go to the one with the
                // lowest point, so a reordered input keeps the same one.
                let rank = ranks(&soup.pos);
                let low = |faces: &Vec<usize>| {
                    faces
                        .iter()
                        .flat_map(|&f| soup.faces[f].rings.iter().flatten())
                        .map(|&v| rank[v])
                        .min()
                        .unwrap_or(usize::MAX)
                };
                let keep = (0..isl.len())
                    .max_by(|&a, &b| {
                        isl[a]
                            .len()
                            .cmp(&isl[b].len())
                            .then(low(&isl[b]).cmp(&low(&isl[a])))
                    })
                    .expect("several islands");
                let kept = isl.remove(keep);
                isl.sort_by_key(|faces| low(faces));
                isl.insert(0, kept);
            }
            isl
        } else {
            vec![(0..soup.faces.len()).collect()]
        };
        let rank = ranks(&soup.pos);
        let stitch_width = plan.weld.unwrap_or_else(|| default_weld_distance(self));
        let claims = remapped_claims(
            self,
            &index,
            front.weld.as_ref().map(|w| &w.root[..]),
            &soup.pos,
        );
        let mut built: Vec<Built> = Vec::with_capacity(islands.len());
        let mut budget = FILL_MWT_TOTAL_WORK;
        for faces in &islands {
            built.push(fill_and_build(
                &mut soup,
                faces,
                self,
                &claims,
                plan,
                &named,
                &rank,
                stitch_width,
                &mut budget,
            )?);
        }
        report.pieces = built.len() - 1;
        let mut filled_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut fill_curved = 0;
        let mut objects: Vec<Object> = Vec::with_capacity(built.len());
        for (i, b) in built.into_iter().enumerate() {
            for points in &b.filled_points {
                let n = points.len();
                for k in 0..n {
                    let (x, y) = (soup.origin[points[k]], soup.origin[points[(k + 1) % n]]);
                    filled_edges.insert((x.min(y), x.max(y)));
                }
            }
            fill_curved += b.curved;
            let mut obj = b.obj;
            let ids: Vec<VertexId> = obj.vertices.keys().collect();
            let back: SecondaryMap<VertexId, usize> = ids
                .iter()
                .zip(&b.used)
                .map(|(&v, &u)| (v, soup.origin[u]))
                .collect();
            for (a, c) in seam_edges(&obj, |v| back[v]) {
                if !drawn.contains(&(a.0.min(a.1), a.0.max(a.1))) {
                    obj.edges[c].soft = true;
                    report.soft_seams += 1;
                }
            }
            report.reversed |= obj.orient_closed_islands();
            obj.validate().map_err(super::invalid)?;
            debug_assert!(
                !plan.split
                    || !health::diagnose(&obj)
                        .findings
                        .iter()
                        .any(|f| matches!(f, health::Finding::BranchingBoundaryVertex { .. })),
                "a split leaves no branching boundary point"
            );
            report.filled_rims += b.filled;
            report.seam_folds += b.folds;
            report.stitched_rims += b.stitched;
            report.fill_triangles += b.triangles;
            let closed = obj.watertight == WatertightState::Watertight;
            if i == 0 {
                report.closed = closed;
            } else if closed {
                report.pieces_closed += 1;
            }
            if !b.skipped.is_empty() {
                let d = health::diagnose(&obj);
                for ((a, bb), reason) in b.skipped {
                    let (pa, pb) = (soup.pos[a], soup.pos[bb]);
                    let rim = rim_index(&d, &obj, pa, pb).or_else(|| {
                        d.rims.iter().position(|r| {
                            r.half_edges
                                .iter()
                                .any(|&h| obj.vertices[obj.half_edges[h].origin].position == pa)
                        })
                    });
                    report.skipped_rims.push(SkippedRim {
                        object: i,
                        rim,
                        reason,
                    });
                }
            }
            objects.push(obj);
        }
        // No worse than before: no new opening, no solid pressed flat.
        let was_solid = self.watertight == WatertightState::Watertight;
        let all_closed = objects
            .iter()
            .all(|o| o.watertight == WatertightState::Watertight);
        let open_after: usize = objects.iter().map(open_edges).sum();
        if (was_solid && !all_closed) || open_after > open_edges(self) + front.opened_edges {
            return Err(RepairError::MendOpensObject);
        }
        if let Some(d) = plan.weld {
            let volume_after: f64 = objects
                .iter()
                .filter(|o| o.watertight == WatertightState::Watertight)
                .map(|o| o.signed_volume().abs())
                .sum();
            let pressed =
                was_solid && volume_after < FLAT_VOLUME_FRACTION * self.signed_volume().abs();
            let shell = objects.iter().any(|o| {
                o.watertight == WatertightState::Watertight
                    && o.signed_volume().abs() <= FLAT_THICKNESS * d * surface_area(o)
            });
            if pressed || shell {
                return Err(RepairError::WeldFlattensObject);
            }
        }
        Ok(MendRun {
            report,
            objects,
            weld: front.weld,
            split_points,
            filled_edges,
            fill_curved,
        })
    }
}

/// Axis-aligned bounds of a face's outer loop, expanded by `POINT_MERGE`.
fn face_bounds(obj: &Object, f: FaceId) -> [f64; 6] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in obj.loop_positions(obj.faces()[f].outer_loop) {
        for (i, c) in [p.x, p.y, p.z].into_iter().enumerate() {
            b[i] = b[i].min(c - tol::POINT_MERGE);
            b[i + 3] = b[i + 3].max(c + tol::POINT_MERGE);
        }
    }
    b
}

/// A box over more grid cells than this is held against every candidate
/// instead of being entered cell by cell (a big face on a fine grid).
const GRID_MAX_CELLS: i128 = 512;

/// Every pair of faces of `obj`, one of them in `candidates`, whose bounds
/// overlap (expanded by `POINT_MERGE`) and that contact improperly
/// (`ops::faces_improperly_contact`: contact away from their shared
/// vertices and edges). Candidates in their given order, partners in
/// ascending `FaceId` order; a pair of two candidates is tested once. The
/// fill guard drops every offender at once from this one pass.
///
/// Each candidate's partners are found through a grid over the candidates
/// (cells the size of a typical candidate), so a big object is not scanned
/// once per candidate.
fn improper_contacts(obj: &Object, candidates: &[FaceId]) -> Vec<(FaceId, FaceId)> {
    let cand_set: BTreeSet<FaceId> = candidates.iter().copied().collect();
    let cands: Vec<(FaceId, [f64; 6])> = candidates
        .iter()
        .copied()
        .filter(|&c| obj.faces().contains_key(c))
        .map(|c| (c, face_bounds(obj, c)))
        .collect();
    if cands.is_empty() {
        return Vec::new();
    }
    let mut extents: Vec<f64> = cands
        .iter()
        .map(|(_, b)| (b[3] - b[0]).max(b[4] - b[1]).max(b[5] - b[2]))
        .collect();
    extents.sort_by(f64::total_cmp);
    let h = extents[extents.len() / 2].max(tol::POINT_MERGE * 1e3);
    let span = |b: &[f64; 6]| -> Option<[std::ops::RangeInclusive<i64>; 3]> {
        let r = |k: usize| (b[k] / h).floor() as i64..=(b[k + 3] / h).floor() as i64;
        let rs = [r(0), r(1), r(2)];
        let n: i128 = rs
            .iter()
            .map(|r| (r.end() - r.start() + 1) as i128)
            .product();
        (n <= GRID_MAX_CELLS).then_some(rs)
    };
    let mut grid: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
    let mut big: Vec<usize> = Vec::new();
    for (i, (_, b)) in cands.iter().enumerate() {
        match span(b) {
            Some([xs, ys, zs]) => {
                for x in xs {
                    for y in ys.clone() {
                        for z in zs.clone() {
                            grid.entry([x, y, z]).or_default().push(i);
                        }
                    }
                }
            }
            None => big.push(i),
        }
    }
    let overlaps =
        |b: &[f64; 6], cb: &[f64; 6]| (0..3).all(|k| b[k] <= cb[k + 3] && b[k + 3] >= cb[k]);
    let mut partners: Vec<Vec<FaceId>> = vec![Vec::new(); cands.len()];
    let mut stamp: Vec<usize> = vec![usize::MAX; cands.len()];
    for (n, f) in obj.faces().keys().enumerate() {
        let b = face_bounds(obj, f);
        let f_is_cand = cand_set.contains(&f);
        let mut consider = |i: usize, partners: &mut Vec<Vec<FaceId>>| {
            if stamp[i] == n {
                return;
            }
            stamp[i] = n;
            let (c, cb) = &cands[i];
            // A pair of two candidates is tested once, from the larger id's
            // side.
            if f != *c && overlaps(&b, cb) && !(f_is_cand && f < *c) {
                partners[i].push(f);
            }
        };
        match span(&b) {
            Some([xs, ys, zs]) => {
                for x in xs {
                    for y in ys.clone() {
                        for z in zs.clone() {
                            for &i in grid.get(&[x, y, z]).into_iter().flatten() {
                                consider(i, &mut partners);
                            }
                        }
                    }
                }
            }
            None => {
                for i in 0..cands.len() {
                    consider(i, &mut partners);
                }
            }
        }
        for &i in &big {
            consider(i, &mut partners);
        }
    }
    let mut out = Vec::new();
    for (i, (c, _)) in cands.iter().enumerate() {
        let mut ps = std::mem::take(&mut partners[i]);
        ps.sort();
        for p in ps {
            if crate::ops::faces_improperly_contact(obj, *c, p) {
                out.push((*c, p));
            }
        }
    }
    out
}

/// [`improper_contacts`] for the executable specs: every improper contact
/// between `candidates` and the rest of `obj`. Test support only.
#[doc(hidden)]
pub fn improper_contacts_for_test(obj: &Object, candidates: &[FaceId]) -> Vec<(FaceId, FaceId)> {
    improper_contacts(obj, candidates)
}
