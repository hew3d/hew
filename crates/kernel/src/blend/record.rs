//! Stored blend restoration (true-curves.md §3.2, decision D-111): a blend
//! records what it replaced when it is made, so Remove, Resize and the
//! corner re-solve replay stored geometry instead of re-deriving the sharp
//! corner from the planes around the blend. The derivation
//! ([`super::restore`]) stays as the fallback for blends saved before
//! records existed.
//!
//! A record holds, for every face the blend trimmed into one piece, the
//! loop it left and the loop it replaced; every other face it replaced
//! (consumed whole, or cut into several pieces as a swept blend cuts the
//! facets it rolls across) as it was, with the untagged pieces it left of
//! them; the marks of every edge it removed; and the blend's own spec with
//! its edges as segments (decision D-228). Replaying it splices the
//! replaced loops back into the faces beside the blend's own, so an edit
//! elsewhere on such a face (a pocket pushed into it later) survives, and
//! swaps the pieces for the faces they were cut from. A later blend that
//! ran into this one is taken off with it and put back on the restored geometry, so a fillet under a later
//! fillet stays editable. A record that no longer fits the object (a
//! Push/Pull or a boolean through the blend's region) is not applied: the
//! edit falls back to the derivation, and refuses when that fails too. It
//! never edits wrongly, because the patch it builds is validated like any
//! other.

use std::collections::{BTreeMap, BTreeSet};

use super::edit::{CentroidGrid, EdgeCells};
use super::{BlendError, BlendProfile, BlendSpec, BlendStops, CornerStyle, RadiusHandle};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Plane, Point3};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::tol;
use crate::topo::{BlendId, FaceAttrs, Object, WatertightState};
use crate::transform::Transform;

const PM: f64 = tol::POINT_MERGE;

/// The blend as it was asked for, with its edges as segments so they can be
/// found again on restored geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSpec {
    /// Each picked edge's two ends.
    pub edges: Vec<(Point3, Point3)>,
    /// The cross-section.
    pub profile: BlendProfile,
    /// The plane of the asymmetric chamfer's primary face.
    pub primary: Option<Plane>,
    /// How three-way corners are built.
    pub corners: CornerStyle,
    /// Stopped blends: distances from each chain end.
    pub stops: Option<BlendStops>,
    /// Variable radius handles.
    pub handles: Vec<RadiusHandle>,
}

/// One loop of a face the blend trimmed: the ring it left and the ring it
/// replaced.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopEdit {
    /// The loop as the blend left it.
    pub after: Vec<Point3>,
    /// The loop as it was.
    pub before: Vec<Point3>,
}

/// A face the blend consumed whole.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFace {
    /// The outer loop.
    pub outer: Vec<Point3>,
    /// The hole loops.
    pub holes: Vec<Vec<Point3>>,
    /// Material, back material, UV frame, surface claim, blend tag. The
    /// attribute slot is always [`crate::topo::AttrSlot::NONE`]: a slot
    /// names a dictionary in one Object's table only, so the dictionary
    /// itself is `dict`.
    pub attrs: FaceAttrs,
    /// The face's attribute dictionary (empty = none), by value, so the
    /// record means the same in any Object and through a file.
    pub dict: crate::attr::AttrDict,
}

/// The marks of an edge the blend removed.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMark {
    /// One end.
    pub a: Point3,
    /// The other end.
    pub b: Point3,
    /// Its soft and crease flags and its circle and polygon claims.
    pub meta: EdgeMeta,
}

/// What one blend replaced on its object.
#[derive(Debug, Clone, PartialEq)]
pub struct BlendRecord {
    /// The order the object's blends were made in: a later blend has a
    /// larger `seq` than every blend it could have run into.
    pub seq: u32,
    /// The blend's spec.
    pub spec: StoredSpec,
    /// The loops the blend trimmed.
    pub trimmed: Vec<LoopEdit>,
    /// The faces the blend replaced other than through `trimmed`: consumed
    /// whole, or cut into the `pieces`.
    pub consumed: Vec<StoredFace>,
    /// The untagged faces the blend left that replay takes away whole: the
    /// pieces of a face it cut into several (or whose loops it changed in
    /// number), each as its rings, outer first.
    pub pieces: Vec<Vec<Vec<Point3>>>,
    /// The marks of the edges the blend removed.
    pub marks: Vec<StoredMark>,
}

/// A later blend taken off ahead of the one being restored, to be put back
/// on the restored geometry.
#[derive(Debug, Clone)]
pub(crate) struct Dependent {
    pub id: BlendId,
    pub seq: u32,
    pub spec: StoredSpec,
}

/// A blend taken off by its record.
pub(crate) struct Replayed {
    /// The object without the blend.
    pub object: Object,
    /// The blend's spec, its edges located in `object`.
    pub spec: BlendSpec,
    /// The faces the blend had trimmed → their restored ids.
    pub face_map: BTreeMap<FaceId, FaceId>,
}

/// A blend taken off by its record, with every later blend that stood in
/// the way taken off too.
pub(crate) struct Unwound {
    /// The object without the blend and without `dependents`.
    pub object: Object,
    /// The blend's spec, its edges located in `object`.
    pub spec: BlendSpec,
    /// The later blends taken off, earliest first.
    pub dependents: Vec<Dependent>,
    /// Faces of the original object re-added along the way → their ids in
    /// `object`.
    pub face_map: BTreeMap<FaceId, FaceId>,
}

/// `first` then `second`, as one map from the earliest ids to the latest.
fn compose(
    first: &BTreeMap<FaceId, FaceId>,
    second: &BTreeMap<FaceId, FaceId>,
) -> BTreeMap<FaceId, FaceId> {
    let mut out: BTreeMap<FaceId, FaceId> = first
        .iter()
        .map(|(&o, &m)| (o, second.get(&m).copied().unwrap_or(m)))
        .collect();
    let mids: BTreeSet<FaceId> = first.values().copied().collect();
    for (&m, &n) in second {
        if !mids.contains(&m) {
            out.entry(m).or_insert(n);
        }
    }
    out
}

fn same_plane(a: &Plane, b: &Plane) -> bool {
    a.normal().dot(b.normal()) >= 1.0 - tol::NORMAL_DIRECTION
        && a.signed_distance(b.point()).abs() <= tol::BLEND_VERIFY
}

fn centroid(ring: &[Point3]) -> Point3 {
    let s = ring
        .iter()
        .fold(crate::math::Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
        / ring.len().max(1) as f64;
    Point3::new(s.x, s.y, s.z)
}

/// Whether two rings are the same cycle (winding kept) within `PM`.
fn ring_eq(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len()
        && (0..b.len()).any(|s| {
            a.iter()
                .enumerate()
                .all(|(i, p)| p.approx_eq(b[(i + s) % b.len()], PM))
        })
}

fn same_seq(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.approx_eq(*q, PM))
}

fn contains(ring: &[Point3], p: Point3) -> bool {
    ring.iter().any(|q| q.approx_eq(p, PM))
}

/// Positions two sets of rings have in common.
fn shared_count(a: &[Vec<Point3>], b: &[Vec<Point3>]) -> usize {
    a.iter()
        .flatten()
        .filter(|p| b.iter().any(|r| contains(r, **p)))
        .count()
}

/// One stretch of a loop the blend changed: the vertices between the kept
/// vertices `a` and `b` as the blend left them and as they were.
struct Run {
    a: Point3,
    after: Vec<Point3>,
    b: Point3,
    before: Vec<Point3>,
}

/// The vertices of `ring` strictly between `a` and `b` walking forward,
/// none of which may be a kept vertex (the kept vertices keep their cyclic
/// order through a blend).
fn between(ring: &[Point3], a: Point3, b: Point3, kept: &[Point3]) -> Option<Vec<Point3>> {
    let n = ring.len();
    let ia = ring.iter().position(|p| p.approx_eq(a, PM))?;
    let mut out = Vec::new();
    let mut k = (ia + 1) % n;
    loop {
        if ring[k].approx_eq(b, PM) {
            return Some(out);
        }
        if k == ia || contains(kept, ring[k]) {
            return None;
        }
        out.push(ring[k]);
        k = (k + 1) % n;
    }
}

/// The stretches of `after` that differ from `before`, between the vertices
/// the two rings share; `None` when they share none or disagree on the
/// order of the ones they share.
fn runs_of(after: &[Point3], before: &[Point3]) -> Option<Vec<Run>> {
    let n = after.len();
    let kept_flag: Vec<bool> = after.iter().map(|p| contains(before, *p)).collect();
    let kept: Vec<Point3> = after
        .iter()
        .zip(&kept_flag)
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect();
    let start = kept_flag.iter().position(|&k| k)?;
    let mut runs = Vec::new();
    let mut i = start;
    loop {
        let mut run = Vec::new();
        let mut j = (i + 1) % n;
        while !kept_flag[j] {
            run.push(after[j]);
            j = (j + 1) % n;
        }
        let (a, b) = (after[i], after[j]);
        let brun = between(before, a, b, &kept)?;
        if !same_seq(&run, &brun) {
            runs.push(Run {
                a,
                after: run,
                b,
                before: brun,
            });
        }
        i = j;
        if i == start {
            break;
        }
    }
    Some(runs)
}

/// `ring` with the stretch `a, run.after.., b` replaced by `a, run.before.., b`.
fn replace_span(ring: &[Point3], run: &Run) -> Option<Vec<Point3>> {
    let n = ring.len();
    let m = run.after.len();
    (0..n).find_map(|ia| {
        if !ring[ia].approx_eq(run.a, PM)
            || !(0..m).all(|k| ring[(ia + 1 + k) % n].approx_eq(run.after[k], PM))
            || !ring[(ia + 1 + m) % n].approx_eq(run.b, PM)
        {
            return None;
        }
        let mut out = vec![ring[ia]];
        out.extend(run.before.iter().copied());
        let mut k = (ia + 1 + m) % n;
        while k != ia {
            out.push(ring[k]);
            k = (k + 1) % n;
        }
        Some(out)
    })
}

/// `cur` with the loop edit (`after` → `before`) applied: the whole ring
/// when `cur` is `after`, else each changed stretch spliced in and the rest
/// of `cur` kept as it is now. `None` when the edit's stretches are not in
/// `cur`.
fn splice(cur: &[Point3], after: &[Point3], before: &[Point3]) -> Option<Vec<Point3>> {
    if ring_eq(cur, after) {
        return Some(before.to_vec());
    }
    let runs = runs_of(after, before)?;
    if runs.is_empty() {
        return None;
    }
    let mut out = cur.to_vec();
    for r in &runs {
        out = replace_span(&out, r)?;
    }
    Some(out)
}

/// A vertex of the loop the edit leaves that must be on the current loop
/// for the edit to apply: one of a changed stretch, else its anchor.
fn probe_of(after: &[Point3], before: &[Point3]) -> Point3 {
    match runs_of(after, before) {
        Some(runs) if !runs.is_empty() => runs[0].after.first().copied().unwrap_or(runs[0].a),
        _ => after[0],
    }
}

/// Whether two lists of hole rings are the same rings one to one (each
/// matched once, so a list with a ring twice never matches one without).
fn holes_match(cur: &[Vec<Point3>], want: &[Vec<Point3>]) -> bool {
    if cur.len() != want.len() {
        return false;
    }
    let mut used = vec![false; cur.len()];
    want.iter().all(|h| {
        let Some(i) = (0..cur.len()).find(|&i| !used[i] && ring_eq(&cur[i], h)) else {
            return false;
        };
        used[i] = true;
        true
    })
}

/// The plane a face cut into pieces comes back on: the first of the
/// pieces' planes that holds its outer ring within [`tol::POINT_MERGE`]
/// and faces the way its winding does (a coplanar piece facing the other
/// way, the underside of a step, is not its piece; review F1); `None`
/// takes the ring's own plane.
fn consumed_plane(outer: &[Point3], planes: impl Iterator<Item = Plane>) -> Option<Plane> {
    let mut n = crate::math::Vec3::new(0.0, 0.0, 0.0);
    for i in 0..outer.len() {
        let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
        n = n + crate::math::Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    let mut planes = planes;
    planes.find(|pl| {
        pl.normal().dot(n) > 0.0
            && outer
                .iter()
                .all(|&p| pl.signed_distance(p).abs() <= tol::POINT_MERGE)
    })
}

impl StoredSpec {
    fn of(obj: &Object, spec: &BlendSpec) -> Option<StoredSpec> {
        let edges = spec
            .edges
            .iter()
            .map(|&e| obj.edge_endpoints(e))
            .collect::<Option<Vec<_>>>()?;
        // An asymmetric chamfer with no primary face takes side A from the
        // first edge's half-edge, which a restored edge may carry the other
        // way: the record names the face, so the setbacks stay put.
        let asymmetric = matches!(
            spec.profile,
            BlendProfile::Chamfer { setback_a, setback_b } if (setback_a - setback_b).abs() > PM
        );
        let side_a = |e: EdgeId| -> Option<Plane> {
            let edge = obj.edges().get(e)?;
            let face = obj.loops()[obj.half_edges()[edge.half_edge].loop_id].face;
            Some(obj.faces()[face].plane)
        };
        let primary = match spec.primary {
            Some(f) => obj.faces().get(f).map(|face| face.plane),
            None if asymmetric => spec.edges.first().and_then(|&e| side_a(e)),
            None => None,
        };
        Some(StoredSpec {
            edges,
            profile: spec.profile.clone(),
            primary,
            corners: spec.corners,
            stops: spec.stops,
            handles: spec.handles.clone(),
        })
    }

    /// The spec on `obj`: the edges along each stored segment (a segment an
    /// edge grew past finds the longer edge; one no edge lies along, the
    /// chord of a blend since removed, finds none).
    ///
    /// # Errors
    /// `BlendModified` when no edge lies along any segment.
    pub(crate) fn locate(&self, obj: &Object) -> Result<BlendSpec, BlendError> {
        // One grid of the object's edges for every segment, and a set
        // beside the list for membership: the list keeps the order found.
        let cells = EdgeCells::new(obj, &self.edges);
        let mut edges: Vec<EdgeId> = Vec::new();
        let mut found: BTreeSet<EdgeId> = BTreeSet::new();
        let mut seed_segment: Option<(Point3, Point3)> = None;
        for &seg in &self.edges {
            for e in cells.edges_on(seg) {
                if found.insert(e) {
                    if edges.is_empty() {
                        seed_segment = Some(seg);
                    }
                    edges.push(e);
                }
            }
        }
        if edges.is_empty() {
            return Err(BlendError::BlendModified);
        }
        let primary = self.primary.and_then(|plane| {
            edges
                .iter()
                .filter_map(|&e| obj.edge_faces(e))
                .flat_map(|(a, b)| [a, b])
                .find(|&f| same_plane(&obj.faces()[f].plane, &plane))
        });
        // The chain runs from the first edge's side-A origin, and a
        // restored edge may carry its half-edge the other way: the stops
        // and handles, measured from the chain's ends, swap ends with it.
        let mut stops = self.stops;
        let mut handles = self.handles.clone();
        if stops.is_some() || !handles.is_empty() {
            let reversed = match (seed_segment, obj.edge_endpoints(edges[0])) {
                (Some((a, b)), Some((p, q))) => (q - p).dot(b - a) < 0.0,
                _ => false,
            };
            if reversed {
                let len: f64 = edges
                    .iter()
                    .filter_map(|&e| obj.edge_endpoints(e))
                    .map(|(x, y)| (y - x).length())
                    .sum();
                stops = stops.map(|s| BlendStops {
                    start: s.end,
                    end: s.start,
                });
                for h in &mut handles {
                    h.at = (len - h.at).max(0.0);
                }
            }
        }
        Ok(BlendSpec {
            edges,
            profile: self.profile.clone(),
            primary,
            corners: self.corners,
            stops,
            handles,
        })
    }

    fn transformed(&self, t: &Transform, k: f64) -> StoredSpec {
        StoredSpec {
            edges: self
                .edges
                .iter()
                .map(|&(a, b)| (t.apply_point(a), t.apply_point(b)))
                .collect(),
            profile: self.profile.scaled(k),
            primary: self.primary.and_then(|p| transform_plane(&p, t)),
            corners: self.corners,
            stops: self.stops.map(|s| BlendStops {
                start: s.start * k,
                end: s.end * k,
            }),
            handles: self
                .handles
                .iter()
                .map(|h| RadiusHandle {
                    at: h.at * k,
                    radius: h.radius * k,
                })
                .collect(),
        }
    }
}

fn transform_plane(p: &Plane, t: &Transform) -> Option<Plane> {
    Plane::from_point_normal(t.apply_point(p.point()), t.apply_normal(p.normal())).ok()
}

impl BlendRecord {
    /// The record under a similarity of scale `k` (the only maps a blend
    /// tag survives, see [`Object::apply_transform`]).
    pub(crate) fn transformed(&self, t: &Transform, k: f64) -> BlendRecord {
        let scale = Some(k);
        let ring = |r: &[Point3]| -> Vec<Point3> { r.iter().map(|&p| t.apply_point(p)).collect() };
        BlendRecord {
            seq: self.seq,
            spec: self.spec.transformed(t, k),
            trimmed: self
                .trimmed
                .iter()
                .map(|e| LoopEdit {
                    after: ring(&e.after),
                    before: ring(&e.before),
                })
                .collect(),
            consumed: self
                .consumed
                .iter()
                .map(|f| StoredFace {
                    outer: ring(&f.outer),
                    holes: f.holes.iter().map(|h| ring(h)).collect(),
                    attrs: FaceAttrs {
                        surface: crate::ops::map_surface(f.attrs.surface, t, scale),
                        ..f.attrs
                    },
                    dict: f.dict.clone(),
                })
                .collect(),
            pieces: self
                .pieces
                .iter()
                .map(|f| f.iter().map(|r| ring(r)).collect())
                .collect(),
            marks: self
                .marks
                .iter()
                .map(|m| StoredMark {
                    a: t.apply_point(m.a),
                    b: t.apply_point(m.b),
                    meta: EdgeMeta {
                        curve: crate::ops::map_curve(m.meta.curve, t, scale),
                        polygon: crate::ops::map_curve(m.meta.polygon, t, scale),
                        ..m.meta
                    },
                })
                .collect(),
        }
    }

    /// The record with every blend id in it moved up by `offset` (a boolean
    /// operand's blends behind the other operand's).
    pub(crate) fn with_ids_offset(&self, offset: u32, seq_offset: u32) -> BlendRecord {
        let mut out = self.clone();
        out.seq = out.seq.saturating_add(seq_offset);
        for f in &mut out.consumed {
            if let Some(t) = f.attrs.blend.as_mut() {
                t.id = BlendId(t.id.0.saturating_add(offset));
            }
        }
        out
    }
}

/// What `after`, the result of blending `before`, replaced on it, as a
/// record; `None` when nothing was replaced or the spec's edges are gone.
///
/// The record is exact by construction: `after` is `before` less the
/// replaced faces plus the blend's own faces and the untagged faces it
/// left, so taking those away and putting the replaced faces back gives
/// `before`. Each untagged face is matched to the replaced face it came
/// from (same plane, most positions in common, then nearest); a replaced
/// face left as exactly one piece with as many loops is stored as loop
/// edits, so later work elsewhere on it survives a replay, and every other
/// replaced face is stored whole, its pieces with it (D-228).
fn capture(before: &Object, after: &Object, id: BlendId, spec: &BlendSpec) -> Option<BlendRecord> {
    let replaced: Vec<FaceId> = before
        .faces()
        .keys()
        .filter(|&f| !after.faces().contains_key(f))
        .collect();
    if replaced.is_empty() {
        return None;
    }
    let replaced_set: BTreeSet<FaceId> = replaced.iter().copied().collect();
    let rings = |o: &Object, f: FaceId| -> Vec<Vec<Point3>> {
        o.face_loops(f)
            .into_iter()
            .map(|l| o.loop_positions(l).collect())
            .collect()
    };
    let before_rings: BTreeMap<FaceId, Vec<Vec<Point3>>> =
        replaced.iter().map(|&r| (r, rings(before, r))).collect();
    let fresh: Vec<(FaceId, Vec<Vec<Point3>>)> = after
        .faces()
        .iter()
        .filter(|(c, cf)| !before.faces().contains_key(*c) && !cf.blend.is_some_and(|t| t.id == id))
        .map(|(c, _)| (c, rings(after, c)))
        .collect();
    // Every (fresh, replaced) pair on one plane, best first: most positions
    // in common, then nearest, then by id for a deterministic order.
    let mut pairs: Vec<(usize, f64, usize, FaceId)> = Vec::new();
    for (ci, (c, c_rings)) in fresh.iter().enumerate() {
        let cplane = after.faces()[*c].plane;
        for &r in &replaced {
            if !same_plane(&before.faces()[r].plane, &cplane) {
                continue;
            }
            let rr = &before_rings[&r];
            let shared = shared_count(rr, c_rings);
            let d = (centroid(&rr[0]) - centroid(&c_rings[0])).length();
            pairs.push((shared, d, ci, r));
        }
    }
    pairs.sort_by(|x, y| {
        y.0.cmp(&x.0)
            .then(x.1.total_cmp(&y.1))
            .then(x.2.cmp(&y.2))
            .then(x.3.cmp(&y.3))
    });
    // One to one first, best pair first; a fresh face left over (a second
    // piece of a cut face) goes to its best replaced face; one on no
    // replaced face's plane is a piece of no face in particular.
    let mut origin: Vec<Option<FaceId>> = vec![None; fresh.len()];
    let mut has_child: BTreeSet<FaceId> = BTreeSet::new();
    for &(_, _, ci, r) in &pairs {
        if origin[ci].is_none() && !has_child.contains(&r) {
            origin[ci] = Some(r);
            has_child.insert(r);
        }
    }
    for &(_, _, ci, r) in &pairs {
        if origin[ci].is_none() {
            origin[ci] = Some(r);
        }
    }
    let mut children: BTreeMap<FaceId, Vec<usize>> = BTreeMap::new();
    let mut orphans: Vec<usize> = Vec::new();
    for (ci, o) in origin.iter().enumerate() {
        match o {
            Some(r) => children.entry(*r).or_default().push(ci),
            None => orphans.push(ci),
        }
    }
    let mut trimmed: Vec<LoopEdit> = Vec::new();
    let mut whole: Vec<FaceId> = Vec::new();
    let mut pieces: Vec<Vec<Vec<Point3>>> = Vec::new();
    for &r in &replaced {
        let kids = children.get(&r).map(Vec::as_slice).unwrap_or(&[]);
        let r_rings = &before_rings[&r];
        // One piece: loop edits, unless the blend rebuilt the face with
        // its rings as they were but other attributes (paint, claim, tag),
        // which only storing it whole puts back (review F6).
        let edits = match kids {
            [ci] => loop_edits(r_rings, &fresh[*ci].1).filter(|e| {
                !e.is_empty() || before.faces()[r].attrs() == after.faces()[fresh[*ci].0].attrs()
            }),
            _ => None,
        };
        match edits {
            Some(e) => trimmed.extend(e),
            None => {
                whole.push(r);
                pieces.extend(kids.iter().map(|&ci| fresh[ci].1.clone()));
            }
        }
    }
    pieces.extend(orphans.iter().map(|&ci| fresh[ci].1.clone()));
    let consumed: Vec<StoredFace> = whole
        .iter()
        .map(|&r| {
            let rr = &before_rings[&r];
            StoredFace {
                outer: rr[0].clone(),
                holes: rr[1..].to_vec(),
                attrs: crate::topo::FaceAttrs {
                    attrs: crate::topo::AttrSlot::NONE,
                    ..before.faces()[r].attrs()
                },
                dict: before.face_attrs(r).cloned().unwrap_or_default(),
            }
        })
        .collect();
    // The marks of every edge of a replaced face that is gone.
    let mut marks: Vec<StoredMark> = Vec::new();
    for (e, edge) in before.edges().iter() {
        if after.edges().contains_key(e) {
            continue;
        }
        let Some((fa, fb)) = before.edge_faces(e) else {
            continue;
        };
        if !(replaced_set.contains(&fa) || replaced_set.contains(&fb)) {
            continue;
        }
        let (a, b) = before.edge_endpoints(e)?;
        marks.push(StoredMark {
            a,
            b,
            meta: EdgeMeta {
                soft: edge.soft,
                crease: edge.crease,
                curve: edge.curve,
                polygon: edge.polygon,
            },
        });
    }
    // The order is the order of application: a blend put back after a
    // restoration goes after every blend present then, as it is applied
    // on top of them.
    let seq = after
        .blend_records
        .values()
        .map(|r| r.seq.saturating_add(1))
        .max()
        .unwrap_or(0);
    Some(BlendRecord {
        seq,
        spec: StoredSpec::of(before, spec)?,
        trimmed,
        consumed,
        pieces,
        marks,
    })
}

/// The loop edits that turn the one piece `c_rings` back into the face
/// `r_rings` it was trimmed from: outer with outer, holes by positions in
/// common, then by distance; `None` when the loop counts differ.
fn loop_edits(r_rings: &[Vec<Point3>], c_rings: &[Vec<Point3>]) -> Option<Vec<LoopEdit>> {
    if r_rings.len() != c_rings.len() {
        return None;
    }
    let mut out = Vec::new();
    let mut used = vec![false; r_rings.len()];
    for (ci, cr) in c_rings.iter().enumerate() {
        let ri = if ci == 0 {
            0
        } else {
            let mut best: Option<(usize, f64, usize)> = None;
            for (ri, rr) in r_rings.iter().enumerate().skip(1) {
                if used[ri] {
                    continue;
                }
                let shared = shared_count(std::slice::from_ref(rr), std::slice::from_ref(cr));
                let d = (centroid(rr) - centroid(cr)).length();
                if best.is_none_or(|(s, bd, _)| shared > s || (shared == s && d < bd)) {
                    best = Some((shared, d, ri));
                }
            }
            best?.2
        };
        used[ri] = true;
        if !ring_eq(&r_rings[ri], cr) {
            out.push(LoopEdit {
                after: cr.clone(),
                before: r_rings[ri].clone(),
            });
        }
    }
    Some(out)
}

/// Patch vertices by position: an existing vertex where one sits at the
/// position, a new one otherwise.
struct VertIndex<'a> {
    obj: &'a Object,
    vids: Vec<VertexId>,
    grid: CentroidGrid,
    existing: BTreeMap<VertexId, usize>,
    fresh: Vec<(Point3, usize)>,
}

impl<'a> VertIndex<'a> {
    fn new(obj: &'a Object) -> Self {
        let vids: Vec<VertexId> = obj.vertices().keys().collect();
        let grid = CentroidGrid::new(vids.iter().map(|&v| obj.vertices()[v].position), PM);
        VertIndex {
            obj,
            vids,
            grid,
            existing: BTreeMap::new(),
            fresh: Vec::new(),
        }
    }

    fn vertex_at(&self, p: Point3) -> Option<VertexId> {
        self.grid
            .near(p)
            .map(|i| self.vids[i])
            .find(|&v| self.obj.vertices()[v].position.approx_eq(p, PM))
    }

    fn index(&mut self, p: Point3, patch: &mut PolyPatch) -> usize {
        if let Some(v) = self.vertex_at(p) {
            return *self.existing.entry(v).or_insert_with(|| {
                patch.verts.push(PVert::Existing(v));
                patch.verts.len() - 1
            });
        }
        if let Some(&(_, i)) = self.fresh.iter().find(|(q, _)| q.approx_eq(p, PM)) {
            return i;
        }
        patch.verts.push(PVert::New(p));
        let i = patch.verts.len() - 1;
        self.fresh.push((p, i));
        i
    }
}

impl Object {
    /// Whether blend `id` carries a record (every blend made since records
    /// exist does; one from an older file does not and restores by
    /// derivation).
    pub fn has_blend_record(&self, id: BlendId) -> bool {
        self.blend_records.contains_key(&id)
    }

    /// Test support only: forgets every record, so a spec can exercise the
    /// derivation a blend from an older file falls back to.
    #[doc(hidden)]
    pub fn drop_blend_records_for_test(&mut self) {
        self.blend_records.clear();
    }

    /// What blend `id`'s record asks for, in words that do not depend on
    /// where the object sits: its profile, corner style, stops and radius
    /// handles as canonical JSON; `None` when the blend has no record.
    /// Shape identity ([`Document::join_linked`](crate::Document::join_linked),
    /// the API's signature) tells two look-alikes apart by it.
    pub fn blend_record_digest(&self, id: BlendId) -> Option<String> {
        let r = self.blend_records.get(&id)?;
        serde_json::to_string(&(
            &r.spec.profile,
            &r.spec.corners,
            &r.spec.stops,
            &r.spec.handles,
        ))
        .ok()
    }

    /// The stored records, by blend id.
    pub(crate) fn blend_records(&self) -> &BTreeMap<BlendId, BlendRecord> {
        &self.blend_records
    }

    /// Replaces the stored records.
    pub(crate) fn set_blend_records(&mut self, records: BTreeMap<BlendId, BlendRecord>) {
        self.blend_records = crate::shared::Shared::new(records);
        self.prune_blend_records();
    }

    /// Drops the records of blends no face carries any more.
    pub(crate) fn prune_blend_records(&mut self) {
        let present: BTreeSet<BlendId> = self
            .faces
            .values()
            .filter_map(|f| f.blend.map(|t| t.id))
            .collect();
        // The table is copy-on-write: write it only when a record goes, so a
        // prune that keeps everything does not copy a shared table.
        if self.blend_records.keys().any(|id| !present.contains(id)) {
            self.blend_records.retain(|id, _| present.contains(id));
        }
    }

    /// Records on `after`, the result of blending `self` with `spec`, what
    /// blend `id` replaced. Nothing is recorded for a change no record can
    /// hold; such a blend restores by derivation.
    pub(crate) fn record_blend(&self, after: &mut Object, id: BlendId, spec: &BlendSpec) {
        // A record left by an earlier blend under this id (its tags dropped
        // by some later op) must not stand in for this one.
        if after.blend_records.contains_key(&id) {
            after.blend_records.remove(&id);
        }
        if let Some(rec) = capture(self, after, id, spec) {
            after.blend_records.insert(id, rec);
        }
    }

    /// Maps the records under a transform: kept and mapped under a
    /// similarity (`scale`), dropped under anything else, like the tags.
    pub(crate) fn transform_blend_records(&mut self, t: &Transform, scale: Option<f64>) {
        match scale {
            // The table is copy-on-write: an empty one is left alone, and
            // a shared one is replaced rather than copied and then rewritten.
            Some(_) | None if self.blend_records.is_empty() => {}
            Some(k) => {
                let mapped: BTreeMap<BlendId, BlendRecord> = self
                    .blend_records
                    .iter()
                    .map(|(&id, rec)| (id, rec.transformed(t, k)))
                    .collect();
                self.blend_records = crate::shared::Shared::new(mapped);
            }
            None => self.blend_records = crate::shared::Shared::new(BTreeMap::new()),
        }
    }

    /// The faces beside blend `id`'s own (sharing an edge with one), and
    /// those beside them when `two_rings`.
    fn faces_around_blend(&self, tagged: &BTreeSet<FaceId>, two_rings: bool) -> BTreeSet<FaceId> {
        let neighbours = |seed: &BTreeSet<FaceId>| -> BTreeSet<FaceId> {
            let mut out = BTreeSet::new();
            for &f in seed {
                for l in self.face_loops(f) {
                    for h in self.loop_half_edges(l) {
                        if let Some(t) = self.half_edges()[h].twin {
                            let g = self.loops()[self.half_edges()[t].loop_id].face;
                            if !tagged.contains(&g) {
                                out.insert(g);
                            }
                        }
                    }
                }
            }
            out
        };
        let mut ring = neighbours(tagged);
        if two_rings {
            let more = neighbours(&ring);
            ring.extend(more);
        }
        ring
    }

    /// Takes blend `id` off by its record alone: its faces and the faces
    /// it trimmed go, the trimmed faces come back with their replaced loops
    /// spliced in (whatever else changed on them since stays), the faces it
    /// consumed come back whole, and the edges it removed get their marks
    /// back (an edge a shortened edge grew back into takes that edge's soft
    /// and crease, as the derivation did).
    ///
    /// # Errors
    /// `UnknownBlend` without a record or a tagged face; `BlendModified`
    /// when a replaced loop no longer fits a face beside the blend or the
    /// patch does not close.
    pub(crate) fn replay_blend_record(&self, id: BlendId) -> Result<Replayed, BlendError> {
        let modified = BlendError::BlendModified;
        let rec = self
            .blend_records
            .get(&id)
            .ok_or(BlendError::UnknownBlend)?;
        let tagged: BTreeSet<FaceId> = self
            .faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| t.id == id))
            .map(|(f, _)| f)
            .collect();
        if tagged.is_empty() {
            return Err(BlendError::UnknownBlend);
        }
        let ve = self.vertex_edges();
        // The faces at a vertex, other than the blend's own.
        let faces_at = |v: VertexId| -> Vec<FaceId> {
            let mut out: Vec<FaceId> = ve
                .get(&v)
                .into_iter()
                .flatten()
                .filter_map(|&e| self.edge_faces(e))
                .flat_map(|(a, b)| [a, b])
                .filter(|f| !tagged.contains(f))
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        };
        let mut rings: BTreeMap<FaceId, Vec<Vec<Point3>>> = BTreeMap::new();
        let mut rings_of = |g: FaceId| -> Vec<Vec<Point3>> {
            rings
                .entry(g)
                .or_insert_with(|| {
                    self.face_loops(g)
                        .into_iter()
                        .map(|l| self.loop_positions(l).collect())
                        .collect()
                })
                .clone()
        };
        let mut verts = VertIndex::new(self);
        // Each piece the blend left is a face as it was left; they go whole.
        let mut piece_faces: BTreeSet<FaceId> = BTreeSet::new();
        let mut piece_order: Vec<FaceId> = Vec::new();
        for piece in &rec.pieces {
            let first = piece.first().and_then(|r| r.first()).ok_or(modified)?;
            let v = verts.vertex_at(*first).ok_or(modified)?;
            let mut hit: Option<FaceId> = None;
            for g in faces_at(v) {
                let cur = rings_of(g);
                let same = cur.len() == piece.len()
                    && ring_eq(&cur[0], &piece[0])
                    && holes_match(&cur[1..], &piece[1..]);
                if same {
                    if hit.is_some() {
                        return Err(modified);
                    }
                    hit = Some(g);
                }
            }
            let g = hit.ok_or(modified)?;
            if !piece_faces.insert(g) {
                return Err(modified);
            }
            piece_order.push(g);
        }
        // Each trimmed loop splices into exactly one loop of a face at it.
        let mut edits: BTreeMap<FaceId, Vec<Option<Vec<Point3>>>> = BTreeMap::new();
        for edit in &rec.trimmed {
            let v = verts
                .vertex_at(probe_of(&edit.after, &edit.before))
                .ok_or(modified)?;
            let mut hit: Option<(FaceId, usize, Vec<Point3>)> = None;
            for g in faces_at(v) {
                if piece_faces.contains(&g) {
                    continue;
                }
                for (li, ring) in rings_of(g).iter().enumerate() {
                    if let Some(new) = splice(ring, &edit.after, &edit.before) {
                        if hit.is_some() {
                            return Err(modified);
                        }
                        hit = Some((g, li, new));
                    }
                }
            }
            let (g, li, new) = hit.ok_or(modified)?;
            let n_loops = rings_of(g).len();
            let slots = edits.entry(g).or_insert_with(|| vec![None; n_loops]);
            if slots[li].is_some() {
                return Err(modified);
            }
            slots[li] = Some(new);
        }
        let mut patch = PolyPatch {
            expect_watertight: self.watertight == WatertightState::Watertight,
            ..Default::default()
        };
        patch.remove.extend(tagged.iter().copied());
        patch.remove.extend(piece_order.iter().copied());
        let mut edited_order: Vec<FaceId> = Vec::with_capacity(edits.len());
        for (&g, slots) in &edits {
            patch.remove.push(g);
            edited_order.push(g);
            let loops: Vec<Vec<usize>> = slots
                .iter()
                .enumerate()
                .map(|(li, s)| {
                    s.clone()
                        .unwrap_or_else(|| rings_of(g)[li].clone())
                        .iter()
                        .map(|&p| verts.index(p, &mut patch))
                        .collect()
                })
                .collect();
            patch.add.push(PatchFace {
                outer: loops[0].clone(),
                holes: loops[1..].to_vec(),
                attrs: self.faces()[g].attrs(),
                plane: Some(self.faces()[g].plane),
            });
        }
        for cf in &rec.consumed {
            // A face cut into pieces comes back on its pieces' plane, the
            // plane it had (one consumed whole takes its ring's).
            let plane = consumed_plane(
                &cf.outer,
                piece_order.iter().map(|&g| self.faces()[g].plane),
            );
            let outer = cf
                .outer
                .iter()
                .map(|&p| verts.index(p, &mut patch))
                .collect();
            let holes = cf
                .holes
                .iter()
                .map(|h| h.iter().map(|&p| verts.index(p, &mut patch)).collect())
                .collect();
            patch.add.push(PatchFace {
                outer,
                holes,
                attrs: cf.attrs,
                plane,
            });
        }
        // Edge marks: an edge that exists keeps its own; one the restoration
        // makes takes its recorded marks, and the soft and crease of an
        // edge it grows back out of.
        let pos = |i: usize| -> Point3 {
            match patch.verts[i] {
                PVert::Existing(v) => self.vertices()[v].position,
                PVert::New(p) => p,
            }
        };
        let vid = |i: usize| -> Option<VertexId> {
            match patch.verts[i] {
                PVert::Existing(v) => Some(v),
                PVert::New(_) => None,
            }
        };
        let existing_edge = |a: VertexId, b: VertexId| -> bool {
            ve.get(&a).is_some_and(|es| {
                es.iter().any(|&e| {
                    self.edge_vertex_ids(e)
                        .is_some_and(|(x, y)| (x == a && y == b) || (x == b && y == a))
                })
            })
        };
        let grown_from = |from: Option<VertexId>, pa: Point3, pb: Point3| -> Option<EdgeId> {
            let v = from?;
            let d = (pb - pa).normalized().ok()?;
            ve.get(&v)?.iter().copied().find(|&e| {
                self.edge_endpoints(e).is_some_and(|(x, y)| {
                    let far = if x.approx_eq(pa, PM) { y } else { x };
                    (far - pa)
                        .normalized()
                        .is_ok_and(|u| u.dot(d) >= 1.0 - tol::BLEND_RAIL_PARALLEL_DEFECT)
                })
            })
        };
        let mark_of = |a: Point3, b: Point3| -> Option<EdgeMeta> {
            rec.marks
                .iter()
                .find(|m| {
                    (m.a.approx_eq(a, PM) && m.b.approx_eq(b, PM))
                        || (m.a.approx_eq(b, PM) && m.b.approx_eq(a, PM))
                })
                .map(|m| m.meta)
        };
        let mut edge_meta: BTreeMap<(usize, usize), EdgeMeta> = BTreeMap::new();
        for face in &patch.add {
            for lp in std::iter::once(&face.outer).chain(face.holes.iter()) {
                for k in 0..lp.len() {
                    let (i, j) = (lp[k], lp[(k + 1) % lp.len()]);
                    let key = (i.min(j), i.max(j));
                    if edge_meta.contains_key(&key) {
                        continue;
                    }
                    let (pi, pj) = (pos(i), pos(j));
                    let (vi, vj) = (vid(i), vid(j));
                    if let (Some(a), Some(b)) = (vi, vj)
                        && existing_edge(a, b)
                    {
                        continue;
                    }
                    let mut meta = mark_of(pi, pj).unwrap_or_default();
                    if let Some(e) = grown_from(vi, pi, pj).or_else(|| grown_from(vj, pj, pi)) {
                        meta.soft = self.edges()[e].soft;
                        meta.crease = self.edges()[e].crease;
                    }
                    edge_meta.insert(key, meta);
                }
            }
        }
        patch.edge_meta = edge_meta;
        let out = self.apply_patch(&patch).map_err(|_| modified)?;
        let face_map: BTreeMap<FaceId, FaceId> = edited_order
            .iter()
            .copied()
            .zip(out.faces.iter().copied())
            .collect();
        let mut object = out.object;
        // The consumed faces come after the edited ones in `patch.add`;
        // each takes back its dictionary, interned in this Object's table.
        for (cf, &nf) in rec.consumed.iter().zip(&out.faces[edited_order.len()..]) {
            if !cf.dict.is_empty() {
                object.set_face_attrs(nf, cf.dict.clone());
            }
        }
        object.blend_records.remove(&id);
        let spec = rec.spec.locate(&object)?;
        Ok(Replayed {
            object,
            spec,
            face_map,
        })
    }

    /// Takes blend `id` off by its record, taking off first every later
    /// blend that stands in the way (one that trimmed this blend's faces or
    /// a face beside them), latest first, each by the same rule.
    ///
    /// # Errors
    /// `UnknownBlend`; `BlendModified` when the record does not fit even
    /// with every later blend around it off.
    pub(crate) fn unwind_blend(&self, id: BlendId) -> Result<Unwound, BlendError> {
        let rec = self
            .blend_records
            .get(&id)
            .ok_or(BlendError::UnknownBlend)?;
        if let Ok(r) = self.replay_blend_record(id) {
            return Ok(Unwound {
                object: r.object,
                spec: r.spec,
                dependents: Vec::new(),
                face_map: r.face_map,
            });
        }
        let tagged: BTreeSet<FaceId> = self
            .faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| t.id == id))
            .map(|(f, _)| f)
            .collect();
        if tagged.is_empty() {
            return Err(BlendError::UnknownBlend);
        }
        let mut later: Vec<(u32, BlendId)> = self
            .faces_around_blend(&tagged, true)
            .iter()
            .filter_map(|&f| self.faces()[f].blend.map(|t| t.id))
            .filter(|&j| j != id)
            .filter_map(|j| self.blend_records.get(&j).map(|r| (r.seq, j)))
            .filter(|&(seq, _)| seq > rec.seq)
            .collect();
        later.sort_unstable_by(|x, y| y.cmp(x));
        later.dedup();
        if later.is_empty() {
            return Err(BlendError::BlendModified);
        }
        let mut cur = self.clone();
        let mut dependents: Vec<Dependent> = Vec::new();
        let mut face_map: BTreeMap<FaceId, FaceId> = BTreeMap::new();
        for (_, j) in later {
            let Some(jr) = cur.blend_records.get(&j) else {
                continue;
            };
            if !cur
                .faces()
                .values()
                .any(|f| f.blend.is_some_and(|t| t.id == j))
            {
                continue;
            }
            let dep = Dependent {
                id: j,
                seq: jr.seq,
                spec: jr.spec.clone(),
            };
            let u = cur.unwind_blend(j)?;
            cur = u.object;
            face_map = compose(&face_map, &u.face_map);
            dependents.push(dep);
            dependents.extend(u.dependents);
        }
        let r = cur.replay_blend_record(id)?;
        face_map = compose(&face_map, &r.face_map);
        dependents.sort_by_key(|d| d.seq);
        dependents.dedup_by_key(|d| d.id);
        Ok(Unwound {
            object: r.object,
            spec: r.spec,
            dependents,
            face_map,
        })
    }

    /// Puts `dependents` back, earliest first, each under its own id
    /// (unless the corner re-solve merges it with an equal neighbour it
    /// runs into, into one blend under a fresh id as any merged blend) and
    /// with its edges found again on the geometry as it is now. Each takes
    /// a fresh place in the order, after every blend present: the order is
    /// the order of application.
    ///
    /// # Errors
    /// `DependentBlend` when one of them cannot be put back.
    pub(crate) fn rewind_blends(&self, dependents: &[Dependent]) -> Result<Object, BlendError> {
        let _reserved = super::ReservedBlendIds::new(dependents.iter().map(|d| d.id));
        let mut cur = self.clone();
        for d in dependents {
            let spec = d
                .spec
                .locate(&cur)
                .map_err(|_| BlendError::DependentBlend)?;
            let (next, _) = super::with_forced_blend_id(Some(d.id), || cur.blend_edges(&spec))
                .map_err(|_| BlendError::DependentBlend)?;
            cur = next;
        }
        Ok(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(z: f64, up: bool) -> Vec<Point3> {
        let mut r = vec![
            Point3::new(0.0, 0.0, z),
            Point3::new(1.0, 0.0, z),
            Point3::new(1.0, 1.0, z),
            Point3::new(0.0, 1.0, z),
        ];
        if !up {
            r.reverse();
        }
        r
    }

    /// A coplanar piece facing the other way does not lend its plane
    /// (review F1): an up-facing face takes the up-facing plane only.
    #[test]
    fn a_consumed_face_takes_a_plane_facing_its_way() {
        let up = Plane::from_polygon(&square(1.0, true)).unwrap();
        let down = Plane::from_polygon(&square(1.0, false)).unwrap();
        assert!(up.normal().z > 0.0 && down.normal().z < 0.0);
        let got = consumed_plane(&square(1.0, true), [down, up].into_iter()).unwrap();
        assert!(got.normal().z > 0.0);
        assert!(consumed_plane(&square(1.0, true), [down].into_iter()).is_none());
        assert!(
            consumed_plane(&square(1.0, false), [up, down].into_iter())
                .unwrap()
                .normal()
                .z
                < 0.0
        );
    }

    /// A face the blend rebuilt with its rings as they were but other
    /// attributes is stored whole with its piece, so a replay puts its
    /// attributes back (review F6).
    #[test]
    fn a_face_rebuilt_with_new_attributes_is_stored_whole() {
        let pts = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 1.0, 1.0),
        ];
        let quads = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let before = Object::from_polygons(&pts, &quads).unwrap();
        let top = before
            .faces()
            .keys()
            .find(|&f| before.faces()[f].plane.normal().z > 0.5)
            .unwrap();
        let ring: Vec<VertexId> = before
            .loop_half_edges(before.faces()[top].outer_loop)
            .map(|h| before.half_edges()[h].origin)
            .collect();
        let mut attrs = before.faces()[top].attrs();
        attrs.uv_frame = Some(crate::material::UvFrame {
            s: crate::math::Vec3::new(1.0, 0.0, 0.0),
            t: crate::math::Vec3::new(0.0, 1.0, 0.0),
            u0: 0.25,
            v0: 0.0,
        });
        let patch = PolyPatch {
            verts: ring.iter().map(|&v| PVert::Existing(v)).collect(),
            remove: vec![top],
            add: vec![PatchFace {
                outer: (0..ring.len()).collect(),
                holes: Vec::new(),
                attrs,
                plane: Some(before.faces()[top].plane),
            }],
            edge_meta: BTreeMap::new(),
            expect_watertight: true,
        };
        let after = before.apply_patch(&patch).unwrap().object;
        let edge = before.edges().keys().next().unwrap();
        let spec = BlendSpec {
            edges: vec![edge],
            profile: BlendProfile::Round {
                radius: 0.1,
                segments: 4,
            },
            primary: None,
            corners: CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        };
        let rec = capture(&before, &after, BlendId(1), &spec).unwrap();
        assert!(rec.trimmed.is_empty());
        assert_eq!(rec.consumed.len(), 1);
        assert_eq!(rec.consumed[0].attrs, before.faces()[top].attrs());
        assert_eq!(rec.pieces.len(), 1);
    }

    /// Holes match one to one (review F19): a piece listing one hole twice
    /// never matches a face with two different holes.
    #[test]
    fn holes_match_one_to_one() {
        let h = square(0.0, false);
        let k: Vec<Point3> = h.iter().map(|p| Point3::new(p.x + 5.0, p.y, p.z)).collect();
        assert!(holes_match(
            &[h.clone(), k.clone()],
            &[k.clone(), h.clone()]
        ));
        assert!(!holes_match(
            &[h.clone(), k.clone()],
            &[h.clone(), h.clone()]
        ));
        assert!(!holes_match(
            std::slice::from_ref(&h),
            &[h.clone(), h.clone()]
        ));
    }
}
