//! Chains and pass-through vertices (WP05 §6, WP06 §4): picked edges that
//! continue through a vertex with the same two sides, and only side-internal
//! edges besides, form one chain whose stations share their section points.
//! A chain may close (a rim all the way round a circle).

use std::collections::BTreeMap;

use super::BlendError;
use super::BlendProfile;
use super::classify::{EdgeInfo, EdgeKind, info_for};
use super::plan::PlanFail;
use super::station::{same_side, station_section};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::Vec3;
use crate::tol;
use crate::topo::Object;

/// A maximal run of picked edges through pass-through vertices.
#[derive(Debug, Clone)]
pub(crate) struct Chain {
    /// The edges in chain order, each oriented so its side A continues the
    /// previous edge's side A.
    pub edges: Vec<EdgeInfo>,
    /// The chain's vertices, `edges.len() + 1` of them; a closed chain
    /// repeats its first vertex last.
    pub verts: Vec<VertexId>,
    /// Whether the chain closes on itself.
    pub closed: bool,
}

impl Chain {
    /// Unit direction of segment `i`.
    pub(crate) fn seg_dir(&self, obj: &Object, i: usize) -> Vec3 {
        let (a, b) = (
            obj.vertices()[self.verts[i]].position,
            obj.vertices()[self.verts[i + 1]].position,
        );
        (b - a).normalized().unwrap_or(Vec3::new(0.0, 0.0, 0.0))
    }
}

/// Whether faces `f` and `g` lie in one plane (normals within
/// `NORMAL_DIRECTION`, `g`'s plane point within `PLANE_DIST` of `f`'s plane).
pub(crate) fn coplanar(obj: &Object, f: FaceId, g: FaceId) -> bool {
    let (pf, pg) = (obj.faces()[f].plane, obj.faces()[g].plane);
    pf.normal().dot(pg.normal()) >= 1.0 - tol::NORMAL_DIRECTION
        && pf.signed_distance(pg.point()).abs() <= tol::PLANE_DIST
}

fn ends(obj: &Object, e: EdgeId) -> (VertexId, VertexId) {
    let he = obj.half_edges()[obj.edges()[e].half_edge];
    (he.origin, obj.half_edges()[he.next].origin)
}

/// `e2` oriented so its sides continue `e1`'s at the vertex whose edges are
/// `incident`, or `None`.
fn continue_sides(
    obj: &Object,
    incident: &[EdgeId],
    e1: &EdgeInfo,
    e2: &EdgeInfo,
) -> Result<Option<EdgeInfo>, PlanFail> {
    let at = Some(incident);
    let ok = |a: FaceId, b: FaceId| same_side(obj, e1.a, a, at) && same_side(obj, e1.b, b, at);
    if ok(e2.a, e2.b) {
        Ok(Some(e2.clone()))
    } else if ok(e2.b, e2.a) {
        Ok(Some(info_for(obj, e2.edge, e2.b)?))
    } else {
        Ok(None)
    }
}

/// A swept chain through `v`: the two picked edges split the faces around
/// `v` into two fans, one per side (faceted sides need not be one surface);
/// `e2` is oriented so its side A lies in `e1`'s side-A fan.
fn swept_continue(
    obj: &Object,
    e1: &EdgeInfo,
    e2: &EdgeInfo,
    incident: &[EdgeId],
) -> Result<Option<EdgeInfo>, PlanFail> {
    // The faces reached from `e1.a` around `v` without crossing either
    // picked edge.
    let mut fan = vec![e1.a];
    let mut i = 0;
    while i < fan.len() {
        let f = fan[i];
        for &o in incident {
            if o == e1.edge || o == e2.edge {
                continue;
            }
            if let Some((x, y)) = obj.edge_faces(o) {
                let g = if x == f {
                    y
                } else if y == f {
                    x
                } else {
                    continue;
                };
                if !fan.contains(&g) {
                    fan.push(g);
                }
            }
        }
        i += 1;
    }
    if fan.contains(&e1.b) {
        return Ok(None);
    }
    match (fan.contains(&e2.a), fan.contains(&e2.b)) {
        (true, false) => Ok(Some(e2.clone())),
        (false, true) => Ok(Some(info_for(obj, e2.edge, e2.b)?)),
        _ => Ok(None),
    }
}

/// Whether `v`, shared by picked edges `e1` and `e2` (and no other picked
/// edge), is a pass-through vertex; returns `e2` re-oriented to continue
/// `e1`.
fn pass_through(
    obj: &Object,
    vertex_edges: &BTreeMap<VertexId, Vec<EdgeId>>,
    v: VertexId,
    e1: &EdgeInfo,
    e2: &EdgeInfo,
    incident: &[EdgeId],
    profile: &BlendProfile,
) -> Result<Option<EdgeInfo>, PlanFail> {
    if e1.kind == EdgeKind::Swept && e2.kind == EdgeKind::Swept {
        return swept_continue(obj, e1, e2, incident);
    }
    let Some(e2) = continue_sides(obj, incident, e1, e2)? else {
        return Ok(None);
    };
    // Every other edge at v lies inside one side.
    let inside = incident
        .iter()
        .filter(|&&o| o != e1.edge && o != e2.edge)
        .all(|&o| {
            obj.edge_faces(o).is_some_and(|(x, y)| {
                let within = |s: FaceId| {
                    same_side(obj, s, x, Some(incident)) && same_side(obj, s, y, Some(incident))
                };
                within(e1.a) || within(e1.b)
            })
        });
    if !inside {
        return Ok(None);
    }
    if e1.kind == EdgeKind::Swept || e2.kind == EdgeKind::Swept {
        // A swept chain continues through any vertex where its sides do;
        // mixing it with a catalogue kind is not supported.
        return Ok((e1.kind == e2.kind).then_some(e2));
    }
    if e1.kind == EdgeKind::PlanePlane && e2.kind == EdgeKind::PlanePlane {
        // Straight runs pass through only when collinear.
        return Ok((e1.d_a.dot(e2.d_a).abs() >= 1.0 - tol::NORMAL_DIRECTION).then_some(e2));
    }
    // Otherwise the two kinds' sections at v must agree point for point.
    let (Ok(s1), Ok(s2)) = (
        station_section(obj, vertex_edges, e1, v, profile),
        station_section(obj, vertex_edges, &e2, v, profile),
    ) else {
        return Ok(None);
    };
    let agree = s1.offsets.len() == s2.offsets.len()
        && s1
            .offsets
            .iter()
            .zip(&s2.offsets)
            .all(|(a, b)| (*a - *b).length() <= tol::POINT_MERGE);
    Ok(agree.then_some(e2))
}

/// Groups the classified edges into chains (deterministic: seeded in
/// ascending edge id).
pub(crate) fn build_chains(
    obj: &Object,
    infos: &[EdgeInfo],
    vertex_edges: &BTreeMap<VertexId, Vec<EdgeId>>,
    profile: &BlendProfile,
) -> Result<Vec<Chain>, PlanFail> {
    let by_edge: BTreeMap<EdgeId, &EdgeInfo> = infos.iter().map(|i| (i.edge, i)).collect();
    let mut picked_at: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
    for i in infos {
        let (a, b) = ends(obj, i.edge);
        picked_at.entry(a).or_default().push(i.edge);
        picked_at.entry(b).or_default().push(i.edge);
    }
    let mut used: BTreeMap<EdgeId, bool> = infos.iter().map(|i| (i.edge, false)).collect();
    let mut chains = Vec::new();
    for seed in infos.iter().map(|i| i.edge) {
        if used[&seed] {
            continue;
        }
        let first = by_edge[&seed].clone();
        // Walks from `start` through `through`: (edge, its far vertex) in
        // order away from the seed, and whether the walk came back round.
        let walk = |start: &EdgeInfo,
                    through: VertexId|
         -> Result<(Vec<(EdgeInfo, VertexId)>, bool), PlanFail> {
            let mut out: Vec<(EdgeInfo, VertexId)> = Vec::new();
            let (mut cur, mut v) = (start.clone(), through);
            for _ in 0..infos.len() {
                let Some(p) = picked_at.get(&v) else { break };
                if p.len() != 2 {
                    break;
                }
                let next_id = if p[0] == cur.edge { p[1] } else { p[0] };
                let incident = vertex_edges.get(&v).map(|v| v.as_slice()).unwrap_or(&[]);
                let Some(next) = pass_through(
                    obj,
                    vertex_edges,
                    v,
                    &cur,
                    by_edge[&next_id],
                    incident,
                    profile,
                )?
                else {
                    break;
                };
                if next_id == seed {
                    return Ok((out, true));
                }
                let (a, b) = ends(obj, next_id);
                let far = if a == v { b } else { a };
                out.push((next.clone(), far));
                cur = next;
                v = far;
            }
            Ok((out, false))
        };
        // The seed runs start_v → end_v along its side-A half-edge.
        let (a, b) = ends(obj, seed);
        let (start_v, end_v) = {
            let pa = obj.vertices()[a].position;
            if (obj.vertices()[b].position - pa).dot(first.d_a) >= 0.0 {
                (a, b)
            } else {
                (b, a)
            }
        };
        let (fwd, closed) = walk(&first, end_v)?;
        let back = if closed {
            Vec::new()
        } else {
            walk(&first, start_v)?.0
        };
        // Chain order: back reversed, the seed, then forward.
        let mut edges: Vec<EdgeInfo> = Vec::new();
        let mut verts: Vec<VertexId> = Vec::new();
        for (info, far) in back.iter().rev() {
            if verts.is_empty() {
                verts.push(*far);
            }
            edges.push(info.clone());
            let (x, y) = ends(obj, info.edge);
            verts.push(if x == *far { y } else { x });
        }
        if verts.is_empty() {
            verts.push(start_v);
        }
        edges.push(first.clone());
        verts.push(end_v);
        for (info, far) in &fwd {
            edges.push(info.clone());
            verts.push(*far);
        }
        for e in &edges {
            if used[&e.edge] {
                return Err(PlanFail::Hard(BlendError::UnsupportedVertex {
                    at: obj.vertices()[start_v].position,
                }));
            }
            used.insert(e.edge, true);
        }
        chains.push(Chain {
            edges,
            verts,
            closed,
        });
    }
    Ok(chains)
}

thread_local! {
    /// Which of a swept chain's canonical starts and directions this
    /// attempt takes ([`with_swept_attempt`]).
    static SWEPT_ATTEMPT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many attempts the swept chains planned since the flag was last
    /// cleared offer (0 when none was planned).
    static SWEPT_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The attempts a closed swept chain offers: its first two starts, each
/// both ways round. An open chain offers its two directions.
const CLOSED_ATTEMPTS: usize = 4;
const OPEN_ATTEMPTS: usize = 2;

/// Runs `f` with every swept chain started and run as attempt `k` of
/// [`canonical_swept`], and says how many attempts the swept chains `f`
/// planned offer (0 when it planned none).
pub(crate) fn with_swept_attempt<T>(k: usize, f: impl FnOnce() -> T) -> (T, usize) {
    /// Puts the attempt state back however `f` ends (a caught panic
    /// included), so a later plan never runs as a stale attempt.
    struct Restore(usize, usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            SWEPT_ATTEMPT.with(|c| c.set(self.0));
            SWEPT_ATTEMPTS.with(|c| c.set(self.1.max(c.get())));
        }
    }
    let guard = Restore(
        SWEPT_ATTEMPT.with(|c| c.replace(k)),
        SWEPT_ATTEMPTS.with(|c| c.replace(0)),
    );
    let out = f();
    let n = SWEPT_ATTEMPTS.with(std::cell::Cell::get);
    drop(guard);
    (out, n)
}

/// The grid ([`tol::POINT_MERGE`]) positions and turns are compared on, so
/// a start does not hang on rounding noise: a regular polygon's equal
/// turns differ by about 1e-14 from vertex to vertex, and differently
/// wherever the polygon stands, and a mirrored solid's zeros may be −0.
const START_GRID: f64 = tol::POINT_MERGE;

/// `p`'s place in x, then y, then z order, on the [`START_GRID`] (so −0
/// and +0, and coordinates within a grid step of a grid point, tie).
pub(crate) fn lex_key(p: crate::math::Point3) -> [i64; 3] {
    let q = |x: f64| (x / START_GRID).round() as i64;
    [q(p.x), q(p.y), q(p.z)]
}

/// `p` before `q` in x, then y, then z, on the [`START_GRID`].
fn lex_cmp(p: crate::math::Point3, q: crate::math::Point3) -> std::cmp::Ordering {
    lex_key(p).cmp(&lex_key(q))
}

/// A swept chain started and run by its geometry alone (D-228), so the
/// same solid gives the same blend whatever its ids (an object put back by
/// a blend's record, or the same solid built in another order) and a
/// resize matches a fresh blend. The swept blend's stations follow the
/// chain (on a generated wall or a scan they are spaced evenly from its
/// start), and which start and direction the ball rolls from decides
/// whether some sizes build; so a chain offers a few attempts, tried in
/// turn until one builds ([`Object::blend_single`]). A closed chain starts
/// at its straightest vertex (the smallest turn between its two edges,
/// turns and positions compared on a [`START_GRID`]; ties first in x, then
/// y, then z), then at the next straightest, each
/// run toward whichever of the start's two neighbors comes first in that
/// order, then the other way round. An open chain runs from its end first
/// in that order, then from its other end. Reversing a chain swaps every
/// edge's sides, since side A's half-edge runs along the chain.
pub(crate) fn canonical_swept(obj: &Object, chain: Chain) -> Result<Chain, PlanFail> {
    let pos = |v: VertexId| obj.vertices()[v].position;
    let n = chain.edges.len();
    if n == 0 {
        return Ok(chain);
    }
    let attempt = SWEPT_ATTEMPT.with(std::cell::Cell::get);
    let offered = if chain.closed {
        CLOSED_ATTEMPTS
    } else {
        OPEN_ATTEMPTS
    };
    SWEPT_ATTEMPTS.with(|c| c.set(c.get().max(offered)));
    let flip = attempt % 2 == 1;
    let reversed = |c: &Chain| -> Result<Chain, PlanFail> {
        let mut edges = Vec::with_capacity(c.edges.len());
        for e in c.edges.iter().rev() {
            edges.push(info_for(obj, e.edge, e.b)?);
        }
        let mut verts = c.verts.clone();
        verts.reverse();
        Ok(Chain {
            edges,
            verts,
            closed: c.closed,
        })
    };
    if !chain.closed {
        let backward = lex_cmp(pos(chain.verts[n]), pos(chain.verts[0])).is_lt();
        return if backward != flip {
            reversed(&chain)
        } else {
            Ok(chain)
        };
    }
    // The turn at each vertex: 1 − cos of the angle between its edges.
    let dir = |i: usize| {
        (pos(chain.verts[(i + 1) % n]) - pos(chain.verts[i % n]))
            .normalized()
            .unwrap_or(Vec3::new(0.0, 0.0, 0.0))
    };
    let turns: Vec<f64> = (0..n)
        .map(|i| 1.0 - dir((i + n - 1) % n).dot(dir(i)))
        .collect();
    let keys: Vec<[i64; 3]> = (0..n).map(|i| lex_key(pos(chain.verts[i]))).collect();
    let order = start_order(&turns, &keys);
    let k = order[(attempt / 2).min(n - 1)];
    let mut edges: Vec<EdgeInfo> = Vec::with_capacity(n);
    let mut verts: Vec<VertexId> = Vec::with_capacity(n + 1);
    for i in 0..n {
        edges.push(chain.edges[(k + i) % n].clone());
        verts.push(chain.verts[(k + i) % n]);
    }
    verts.push(verts[0]);
    let rotated = Chain {
        edges,
        verts,
        closed: true,
    };
    let backward = lex_cmp(pos(rotated.verts[n - 1]), pos(rotated.verts[1])).is_lt();
    if backward != flip {
        // Reversed, a closed chain still starts at the same vertex.
        reversed(&rotated)
    } else {
        Ok(rotated)
    }
}

/// The order a closed chain's vertices are tried as its start: by turn,
/// turns within a [`START_GRID`] of the smallest of their tier tying (tiers
/// are cut once, so the order is a total one), then by position.
fn start_order(turns: &[f64], keys: &[[i64; 3]]) -> Vec<usize> {
    let n = turns.len();
    let mut by_turn: Vec<usize> = (0..n).collect();
    by_turn.sort_by(|&i, &j| turns[i].total_cmp(&turns[j]).then(i.cmp(&j)));
    let mut tier = vec![0usize; n];
    if let Some(&first) = by_turn.first() {
        let (mut t, mut base) = (0usize, turns[first]);
        for &i in &by_turn {
            if turns[i] > base + START_GRID {
                t += 1;
                base = turns[i];
            }
            tier[i] = t;
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        tier[i]
            .cmp(&tier[j])
            .then(keys[i].cmp(&keys[j]))
            .then(i.cmp(&j))
    });
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Point3;
    use crate::trig;

    /// The start index on a regular `n`-gon of radius `r` centered at `c`,
    /// with turns computed as [`canonical_swept`] computes them.
    fn polygon_start(n: usize, r: f64, c: Point3) -> usize {
        let p: Vec<Point3> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                Point3::new(c.x + r * trig::cos(a), c.y + r * trig::sin(a), c.z)
            })
            .collect();
        let dir = |i: usize| (p[(i + 1) % n] - p[i % n]).normalized().unwrap();
        let turns: Vec<f64> = (0..n)
            .map(|i| 1.0 - dir((i + n - 1) % n).dot(dir(i)))
            .collect();
        let keys: Vec<[i64; 3]> = p.iter().map(|&q| lex_key(q)).collect();
        start_order(&turns, &keys)[0]
    }

    /// A regular polygon's turns are equal but for rounding noise that
    /// changes with where it stands: the start is the same vertex wherever
    /// it stands (review F2; raw turns started a 48-gon at the origin at
    /// vertex 30 and at (100.3, 37.1) at vertex 27).
    #[test]
    fn a_moved_regular_polygon_starts_at_the_same_vertex() {
        for n in [24, 48, 64] {
            let here = polygon_start(n, 0.5, Point3::new(0.0, 0.0, 0.0));
            for c in [(100.3, 37.1, 0.0), (-3.7, 12.25, 4.0), (0.001, -0.002, 0.0)] {
                let there = polygon_start(n, 0.5, Point3::new(c.0, c.1, c.2));
                assert_eq!(here, there, "{n} at {c:?}");
            }
        }
    }

    /// −0 and +0 tie, so x falls through to y (review F14).
    #[test]
    fn a_signed_zero_ties_with_zero() {
        let a = Point3::new(-0.0, 5.0, 0.0);
        let b = Point3::new(0.0, 0.0, 0.0);
        assert_eq!(lex_cmp(a, b), std::cmp::Ordering::Greater);
        assert_eq!(lex_key(Point3::new(-0.0, -0.0, -0.0)), [0, 0, 0]);
    }
}
