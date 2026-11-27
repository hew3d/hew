//! Project (true-curves.md §5.5): drapes a closed sketch shape onto the
//! faces of an object along a direction (the imprint), then engraves or
//! embosses the imprinted region, straight along the direction (a boolean
//! with the swept prism) or following the surface (the offset engine).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::document::DocumentError;
use crate::geom2d::point_inside_polygon;
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::offset3d::{offset_claim, offset_vertices, push_face};
use crate::ops::{BooleanError, BooleanOp};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::shape_error::ShapeError;
use crate::sketch::{CurveGeom, Sketch};
use crate::tol;
use crate::topo::{FaceAttrs, Object, SurfaceRef, WatertightState};
use crate::transform::Transform;

/// Face ids cross the boundary as their `u64` handles.
mod serde_face_ids {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use slotmap::{Key, KeyData};

    use crate::ids::FaceId;

    pub fn serialize<S: Serializer>(ids: &[FaceId], s: S) -> Result<S::Ok, S::Error> {
        let raw: Vec<u64> = ids.iter().map(|f| f.data().as_ffi()).collect();
        raw.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<FaceId>, D::Error> {
        let raw = Vec::<u64>::deserialize(d)?;
        Ok(raw
            .into_iter()
            .map(|h| FaceId::from(KeyData::from_ffi(h)))
            .collect())
    }
}

/// One closed region of the shape, in its own (sketch) plane.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedRegion {
    /// The outer loop.
    pub outer: Vec<Point3>,
    /// Hole loops.
    #[serde(default)]
    pub holes: Vec<Vec<Point3>>,
    /// The circle each outer edge `outer[k] → outer[k+1]` is a facet of, as
    /// the sketch's profile carries it. Parallel to `outer`; empty: none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outer_curves: Vec<Option<CurveGeom>>,
    /// Same, per hole edge. Parallel to `holes` when non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hole_curves: Vec<Vec<Option<CurveGeom>>>,
    /// The smooth-chain key of each outer edge (a curve with no circle
    /// record, such as a stretched circle). Parallel to `outer`; empty: none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outer_smooth: Vec<Option<u32>>,
    /// Same, per hole edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hole_smooth: Vec<Vec<Option<u32>>>,
}

impl ProjectedRegion {
    /// The circle and smooth-chain key of edge `k` of ring `ring` (0 is the
    /// outer loop, `i + 1` hole `i`).
    fn edge_curve(&self, ring: usize, k: usize) -> (Option<CurveGeom>, Option<u32>) {
        if ring == 0 {
            (
                self.outer_curves.get(k).copied().flatten(),
                self.outer_smooth.get(k).copied().flatten(),
            )
        } else {
            (
                self.hole_curves
                    .get(ring - 1)
                    .and_then(|h| h.get(k))
                    .copied()
                    .flatten(),
                self.hole_smooth
                    .get(ring - 1)
                    .and_then(|h| h.get(k))
                    .copied()
                    .flatten(),
            )
        }
    }
}

/// Exactly the inputs of one imprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImprintSpec {
    /// Unit projection direction (from the sketch toward the target).
    pub dir: Vec3,
    /// The shape, in its own plane (the sketch plane).
    pub regions: Vec<ProjectedRegion>,
}

/// How a depth follows the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthMode {
    /// Straight along the projection direction: a boolean with the swept
    /// region (can cut through a wall).
    Straight,
    /// Along the surface normals: the region's surface offset.
    Follow,
}

/// Exactly the inputs of one depth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepthSpec {
    /// The imprinted region (as the imprint returned it).
    #[serde(with = "serde_face_ids")]
    pub faces: Vec<FaceId>,
    /// Unit projection direction.
    pub dir: Vec3,
    /// `> 0` engraves (into the material), `< 0` embosses.
    pub depth: f64,
    /// Straight or follow.
    pub mode: DepthMode,
    /// The projected shape, so a straight depth's walls take the shape's
    /// curves: a wall along an edge drawn from a circle carries that
    /// circle's cylinder, and walls along one smooth chain meet at soft
    /// seams. Empty: plain walls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<ProjectedRegion>,
}

const DEGENERATE: ShapeError = ShapeError::ShapeDegenerate;

/// The sketch plane of a spec: through the first region's first point,
/// normal `dir`.
fn sketch_plane(spec: &ImprintSpec) -> Option<Plane> {
    let p = spec.regions.first()?.outer.first()?;
    Plane::from_point_normal(*p, spec.dir).ok()
}

/// `p` moved along `dir` onto `plane`.
fn along_onto(p: Point3, dir: Vec3, plane: &Plane) -> Option<Point3> {
    let denom = plane.normal().dot(dir);
    if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    Some(p + dir * (-plane.signed_distance(p) / denom))
}

/// Whether `p` (in the sketch plane) is inside a region of the shape.
fn in_shape(spec: &ImprintSpec, p: Point3) -> bool {
    spec.regions.iter().any(|r| {
        point_inside_polygon(p, &r.outer, spec.dir)
            && !r.holes.iter().any(|h| point_inside_polygon(p, h, spec.dir))
    })
}

/// A strictly interior point of a polygon with holes: the centroid of its
/// largest triangle.
fn interior_point(outer: &[Point3], holes: &[Vec<Point3>], normal: Vec3) -> Option<Point3> {
    let all: Vec<Point3> = outer
        .iter()
        .chain(holes.iter().flatten())
        .copied()
        .collect();
    let tris = crate::triangulate::triangulate_face(outer, holes, normal).ok()?;
    tris.iter()
        .map(|&[a, b, c]| {
            let (pa, pb, pc) = (all[a], all[b], all[c]);
            let area = (pb - pa).cross(pc - pa).length();
            let g = Point3::new(
                (pa.x + pb.x + pc.x) / 3.0,
                (pa.y + pb.y + pc.y) / 3.0,
                (pa.z + pb.z + pc.z) / 3.0,
            );
            (area, g)
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, g)| g)
}

fn face_rings(obj: &Object, f: FaceId) -> (Vec<Point3>, Vec<Vec<Point3>>) {
    let face = &obj.faces()[f];
    (
        obj.loop_positions(face.outer_loop).collect(),
        face.inner_loops
            .iter()
            .map(|&l| obj.loop_positions(l).collect())
            .collect(),
    )
}

fn inside_face(obj: &Object, f: FaceId, p: Point3) -> bool {
    let (outer, holes) = face_rings(obj, f);
    let n = obj.faces()[f].plane.normal();
    point_inside_polygon(p, &outer, n) && !holes.iter().any(|h| point_inside_polygon(p, h, n))
}

fn point_segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn edge_meta(obj: &Object, e: EdgeId) -> EdgeMeta {
    let e = &obj.edges()[e];
    EdgeMeta {
        soft: e.soft,
        crease: e.crease,
        curve: e.curve,
        polygon: e.polygon,
    }
}

/// The loops of `f` as `(vertex, edge leaving it)` lists, outer first.
fn face_loop_edges(obj: &Object, f: FaceId) -> Vec<Vec<(VertexId, EdgeId)>> {
    let face = &obj.faces()[f];
    std::iter::once(face.outer_loop)
        .chain(face.inner_loops.iter().copied())
        .map(|l| {
            obj.loop_half_edges(l)
                .map(|h| (obj.half_edges()[h].origin, obj.half_edges()[h].edge))
                .collect()
        })
        .collect()
}

/// The faces on either side of `e`.
fn edge_faces(obj: &Object, e: EdgeId) -> Vec<FaceId> {
    let edge = &obj.edges()[e];
    let face_of = |h| obj.loops()[obj.half_edges()[h].loop_id].face;
    std::iter::once(edge.half_edge)
        .chain(edge.twin_half_edge)
        .map(face_of)
        .collect()
}

impl Object {
    /// Imprints the projected shape on every front-facing, visible face it
    /// lands on, and returns the object with the region's faces. Pure.
    ///
    /// # Errors
    /// - `ProjectNeedsClosedShape`: no regions, or a loop under 3 points.
    /// - `AmountOutOfRange`: `dir` is not a unit vector.
    /// - `ShapeDegenerate`: the shape lines up exactly with an edge or a
    ///   vertex of the object (move it slightly), or crosses itself.
    /// - `ProjectMisses`: no visible front-facing face is inside the shape.
    pub fn project_imprint(&self, spec: &ImprintSpec) -> Result<(Object, Vec<FaceId>), ShapeError> {
        if spec.regions.is_empty()
            || spec
                .regions
                .iter()
                .any(|r| r.outer.len() < 3 || r.holes.iter().any(|h| h.len() < 3))
        {
            return Err(ShapeError::ProjectNeedsClosedShape);
        }
        if !spec.dir.length().is_finite() || (spec.dir.length() - 1.0).abs() > tol::PROJECT_DIR_UNIT
        {
            return Err(ShapeError::AmountOutOfRange);
        }
        let dir = spec.dir;
        let splane = sketch_plane(spec).ok_or(ShapeError::ProjectNeedsClosedShape)?;
        let candidates: BTreeSet<FaceId> = self
            .faces()
            .iter()
            .filter(|(_, f)| f.plane.normal().dot(dir) < -tol::NORMAL_DIRECTION)
            .map(|(id, _)| id)
            .collect();

        // Shape segments, in the sketch plane.
        let segs: Vec<(Point3, Point3)> = spec
            .regions
            .iter()
            .flat_map(|r| std::iter::once(&r.outer).chain(r.holes.iter()))
            .flat_map(|ring| (0..ring.len()).map(move |k| (ring[k], ring[(k + 1) % ring.len()])))
            .collect();

        // Crossings of each shape wall with each object edge next to a
        // candidate: computed once per (edge, segment), shared by both
        // faces of the edge.
        let mut crossings: BTreeMap<EdgeId, Vec<(usize, f64, Point3)>> = BTreeMap::new();
        for (e, _) in self.edges() {
            let faces = edge_faces(self, e);
            if !faces.iter().any(|f| candidates.contains(f)) {
                continue;
            }
            let (a, b) = self.edge_endpoints(e).ok_or(DEGENERATE)?;
            for (k, &(s0, s1)) in segs.iter().enumerate() {
                let Ok(wn) = (s1 - s0).cross(dir).normalized() else {
                    return Err(ShapeError::ProjectNeedsClosedShape);
                };
                let (da, db) = (wn.dot(a - s0), wn.dot(b - s0));
                // Where the edge's ends project inside the segment's span,
                // a vertex on the wall is a measure-zero alignment.
                let span = |p: Point3| {
                    along_onto(p, dir, &splane).map(|q| {
                        let ab = s1 - s0;
                        (q - s0).dot(ab) / ab.length_squared()
                    })
                };
                for (d, p) in [(da, a), (db, b)] {
                    if d.abs() <= tol::PLANE_DIST
                        && span(p).is_some_and(|u| {
                            (-tol::SPAN_PARAM_SLACK..=1.0 + tol::SPAN_PARAM_SLACK).contains(&u)
                        })
                    {
                        return Err(DEGENERATE);
                    }
                }
                if (da > tol::PLANE_DIST && db < -tol::PLANE_DIST)
                    || (da < -tol::PLANE_DIST && db > tol::PLANE_DIST)
                {
                    let x = a + (b - a) * (da / (da - db));
                    let u = span(x).ok_or(DEGENERATE)?;
                    let seg_len = (s1 - s0).length();
                    let slack = tol::PLANE_DIST / seg_len;
                    if u.abs() <= slack || (u - 1.0).abs() <= slack {
                        // A shape vertex projects onto the edge.
                        return Err(DEGENERATE);
                    }
                    if u > 0.0 && u < 1.0 {
                        crossings.entry(e).or_default().push((k, u, x));
                    }
                }
            }
        }

        // Per candidate face: the pieces of the projected boundary inside
        // it, from crossing to crossing (or vertex).
        let mut pieces: BTreeMap<FaceId, Vec<(Point3, Point3)>> = BTreeMap::new();
        let mut touched: BTreeSet<FaceId> = BTreeSet::new();
        for &f in &candidates {
            let plane = self.faces()[f].plane;
            let loops = face_loop_edges(self, f);
            let mut out = Vec::new();
            for (k, &(s0, s1)) in segs.iter().enumerate() {
                let mut pts: Vec<(f64, Point3)> = Vec::new();
                for ring in &loops {
                    for &(_, e) in ring {
                        if let Some(list) = crossings.get(&e) {
                            for &(kk, u, x) in list {
                                if kk == k {
                                    pts.push((u, x));
                                }
                            }
                        }
                    }
                }
                for (u, s) in [(0.0, s0), (1.0, s1)] {
                    let p = along_onto(s, dir, &plane).ok_or(DEGENERATE)?;
                    if inside_face(self, f, p) {
                        // A vertex projecting onto the face's boundary is
                        // the edge alignment refused above; here it is
                        // strictly inside.
                        for ring in &loops {
                            for &(_, e) in ring {
                                let (a, b) = self.edge_endpoints(e).ok_or(DEGENERATE)?;
                                if point_segment_distance(p, a, b) <= tol::PLANE_DIST {
                                    return Err(DEGENERATE);
                                }
                            }
                        }
                        pts.push((u, p));
                    }
                }
                pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                pts.dedup_by(|a, b| a.1.approx_eq(b.1, tol::POINT_MERGE));
                for w in pts.windows(2) {
                    let (p, q) = (w[0].1, w[1].1);
                    let mid = Point3::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0, (p.z + q.z) / 2.0);
                    if inside_face(self, f, mid) {
                        out.push((p, q));
                    }
                }
            }
            let (outer, holes) = face_rings(self, f);
            let whole = out.is_empty()
                && interior_point(&outer, &holes, plane.normal())
                    .and_then(|c| along_onto(c, dir, &splane))
                    .is_some_and(|c| in_shape(spec, c));
            if !out.is_empty() {
                pieces.insert(f, out);
                touched.insert(f);
            } else if whole {
                pieces.insert(f, Vec::new());
                touched.insert(f);
            }
        }

        // Every face whose boundary carries a crossing must be rebuilt with
        // the crossing inserted, split or not (no T-junctions).
        for &e in crossings.keys() {
            for f in edge_faces(self, e) {
                touched.insert(f);
            }
        }
        if !pieces.keys().any(|_| true) {
            return Err(ShapeError::ProjectMisses);
        }

        let mut patch = PolyPatch {
            expect_watertight: self.watertight() == WatertightState::Watertight,
            ..PolyPatch::default()
        };
        let mut existing: BTreeMap<VertexId, usize> = BTreeMap::new();
        let mut new_at: BTreeMap<[u64; 3], usize> = BTreeMap::new();
        let bits = |p: Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
        let mut slot_existing = |patch: &mut PolyPatch, v: VertexId| {
            *existing.entry(v).or_insert_with(|| {
                patch.verts.push(PVert::Existing(v));
                patch.verts.len() - 1
            })
        };
        let mut slot_new = |patch: &mut PolyPatch, p: Point3| {
            *new_at.entry(bits(p)).or_insert_with(|| {
                patch.verts.push(PVert::New(p));
                patch.verts.len() - 1
            })
        };
        // Region membership of each added face.
        let mut in_region: Vec<bool> = Vec::new();

        for &f in &touched {
            let face = &self.faces()[f];
            let plane = face.plane;
            let loops = face_loop_edges(self, f);
            // The face's loops with the crossings on each edge inserted in
            // order along the edge, as positions and patch indices.
            let mut rings: Vec<Vec<(Point3, usize)>> = Vec::new();
            for ring in &loops {
                let mut out = Vec::new();
                for (k, &(v, e)) in ring.iter().enumerate() {
                    let nv = ring[(k + 1) % ring.len()].0;
                    let p = self.vertices()[v].position;
                    out.push((p, slot_existing(&mut patch, v)));
                    if let Some(list) = crossings.get(&e) {
                        let next = self.vertices()[nv].position;
                        let mut on: Vec<(f64, Point3)> = list
                            .iter()
                            .map(|&(_, _, x)| ((x - p).dot(next - p), x))
                            .collect();
                        on.sort_by(|a, b| a.0.total_cmp(&b.0));
                        on.dedup_by(|a, b| a.1.approx_eq(b.1, tol::POINT_MERGE));
                        let meta = edge_meta(self, e);
                        let mut prev = *out.last().map(|(_, i)| i).unwrap();
                        for (_, x) in on {
                            let i = slot_new(&mut patch, x);
                            patch.edge_meta.insert(pair(prev, i), meta);
                            out.push((x, i));
                            prev = i;
                        }
                        // The last piece runs to the next loop vertex.
                        let ni = slot_existing(&mut patch, nv);
                        patch.edge_meta.insert(pair(prev, ni), meta);
                    }
                }
                rings.push(out);
            }
            patch.remove.push(f);
            let Some(cuts) = pieces.get(&f) else {
                // Not split: the face again, with the crossings inserted.
                let outer = rings[0].iter().map(|&(_, i)| i).collect();
                let holes = rings[1..]
                    .iter()
                    .map(|r| r.iter().map(|&(_, i)| i).collect())
                    .collect();
                patch.add.push(PatchFace {
                    outer,
                    holes,
                    attrs: face.attrs(),
                    plane: Some(plane),
                });
                in_region.push(false);
                continue;
            };
            // Arrange the face against the pieces (pre-split: no new points).
            let mut sk = Sketch::on_plane(plane);
            let mut index_of: BTreeMap<[u64; 3], usize> = BTreeMap::new();
            for ring in &rings {
                for k in 0..ring.len() {
                    let ((p, i), (q, _)) = (ring[k], ring[(k + 1) % ring.len()]);
                    index_of.insert(bits(p), i);
                    sk.add_segment(p, q).map_err(|_| DEGENERATE)?;
                }
            }
            for &(p, q) in cuts {
                for x in [p, q] {
                    index_of
                        .entry(bits(x))
                        .or_insert_with(|| slot_new(&mut patch, x));
                }
                sk.add_segment(p, q).map_err(|_| DEGENERATE)?;
            }
            let regions: Vec<_> = sk.regions().values().cloned().collect();
            for r in regions {
                let pos = |v| sk.vertices()[v].position;
                let idx = |v| index_of.get(&bits(pos(v))).copied();
                let outer: Vec<usize> = r
                    .outer
                    .iter()
                    .map(|&v| idx(v))
                    .collect::<Option<_>>()
                    .ok_or(DEGENERATE)?;
                let holes: Vec<Vec<usize>> = r
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|&v| idx(v)).collect::<Option<Vec<_>>>())
                    .collect::<Option<_>>()
                    .ok_or(DEGENERATE)?;
                let outer_pts: Vec<Point3> = r.outer.iter().map(|&v| pos(v)).collect();
                let hole_pts: Vec<Vec<Point3>> = r
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|&v| pos(v)).collect())
                    .collect();
                let c = interior_point(&outer_pts, &hole_pts, plane.normal()).ok_or(DEGENERATE)?;
                // Sub-faces outside the original face (arrangement of a
                // hole's loop) are not part of it.
                if !inside_face(self, f, c) {
                    continue;
                }
                let inside = along_onto(c, dir, &splane).is_some_and(|q| in_shape(spec, q))
                    && !self.occluded(f, c, dir, &splane);
                patch.add.push(PatchFace {
                    outer,
                    holes,
                    attrs: face.attrs(),
                    plane: Some(plane),
                });
                in_region.push(inside);
            }
        }
        if !in_region.iter().any(|&b| b) {
            return Err(ShapeError::ProjectMisses);
        }
        let result = self.apply_patch(&patch).map_err(|_| DEGENERATE)?;
        let region: Vec<FaceId> = result
            .faces
            .iter()
            .zip(&in_region)
            .filter(|(_, b)| **b)
            .map(|(&f, _)| f)
            .collect();
        Ok((result.object, region))
    }

    /// Whether another face of this object lies between `c` (on face `f`)
    /// and the sketch plane, seen back along `dir`.
    fn occluded(&self, f: FaceId, c: Point3, dir: Vec3, splane: &Plane) -> bool {
        let back = dir * -1.0;
        let reach = match along_onto(c, dir, splane) {
            Some(q) => (q - c).dot(back),
            None => return false,
        };
        self.faces().iter().any(|(g, face)| {
            if g == f {
                return false;
            }
            let denom = face.plane.normal().dot(back);
            if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
                return false;
            }
            let t = -face.plane.signed_distance(c) / denom;
            t > tol::POINT_MERGE && t < reach && inside_face(self, g, c + back * t)
        })
    }

    /// Follow-mode depth (surgery on the offset engine): the region's faces
    /// move `spec.depth` along the solid's normals (inward to engrave),
    /// walled to the surrounding surface. Pure.
    ///
    /// # Errors
    /// `ShapeNeedsSolid`, `AmountOutOfRange` (depth not finite or ~0),
    /// `ShapeDegenerate` (a region face is not a face of this object),
    /// `UnsupportedShellCorner`, `WallTooThick` (an offset claim radius runs
    /// out), `SelfIntersecting` (the offset pierces or hits the object).
    pub fn project_depth_follow(&self, spec: &DepthSpec) -> Result<Object, ShapeError> {
        let region = check_depth(self, spec)?;
        let moved = offset_vertices(self, &region, spec.depth)?;
        let mut patch = PolyPatch {
            expect_watertight: true,
            ..PolyPatch::default()
        };
        let mut existing: BTreeMap<VertexId, usize> = BTreeMap::new();
        let mut offset: BTreeMap<VertexId, usize> = BTreeMap::new();
        let mut walls_at: BTreeMap<VertexId, Vec<Option<WallSource>>> = BTreeMap::new();
        // Rim vertices where a hard edge of the surface runs into the
        // region: the wall seam there continues that crease.
        let mut creased: BTreeSet<VertexId> = BTreeSet::new();
        for (&v, &p) in &moved {
            patch.verts.push(PVert::New(p));
            offset.insert(v, patch.verts.len() - 1);
        }
        for &f in &region {
            let face = &self.faces()[f];
            if face.surface.is_some() && offset_claim(self, f, spec.depth).is_none() {
                return Err(ShapeError::WallTooThick {
                    at: self.vertices()
                        [self.half_edges()[self.loops()[face.outer_loop].first_half_edge].origin]
                        .position,
                });
            }
            let loops = face_loop_edges(self, f);
            let rings: Vec<Vec<usize>> = loops
                .iter()
                .map(|l| l.iter().map(|(v, _)| offset[v]).collect())
                .collect();
            // Offset edges keep soft/crease.
            for (l, ring) in loops.iter().zip(&rings) {
                for k in 0..ring.len() {
                    let meta = edge_meta(self, l[k].1);
                    patch.edge_meta.insert(
                        pair(ring[k], ring[(k + 1) % ring.len()]),
                        EdgeMeta {
                            soft: meta.soft,
                            crease: meta.crease,
                            ..EdgeMeta::default()
                        },
                    );
                }
            }
            let attrs = FaceAttrs {
                surface: offset_claim(self, f, spec.depth),
                blend: None,
                ..face.attrs()
            };
            add_maybe_triangulated(self, &mut patch, rings, attrs)?;
            patch.remove.push(f);
            // Walls along the region's boundary.
            for l in &loops {
                for k in 0..l.len() {
                    let (a, e) = l[k];
                    let b = l[(k + 1) % l.len()].0;
                    if edge_faces(self, e).iter().all(|g| region.contains(g)) {
                        if self.edge_smoothness(e) == crate::surface::EdgeSmoothness::Hard {
                            creased.extend([a, b]);
                        }
                        continue;
                    }
                    let ia = *existing.entry(a).or_insert_with(|| {
                        patch.verts.push(PVert::Existing(a));
                        patch.verts.len() - 1
                    });
                    let ib = *existing.entry(b).or_insert_with(|| {
                        patch.verts.push(PVert::Existing(b));
                        patch.verts.len() - 1
                    });
                    let pos = |v: VertexId| self.vertices()[v].position;
                    let src = wall_source(spec, pos(a), pos(b));
                    walls_at.entry(a).or_default().push(src);
                    walls_at.entry(b).or_default().push(src);
                    let wall = vec![ia, ib, offset[&b], offset[&a]];
                    add_maybe_triangulated(
                        self,
                        &mut patch,
                        vec![wall],
                        FaceAttrs {
                            surface: None,
                            blend: None,
                            ..face.attrs()
                        },
                    )?;
                }
            }
        }
        for v in soft_wall_seams(&walls_at) {
            if !creased.contains(&v) {
                patch
                    .edge_meta
                    .insert(pair(existing[&v], offset[&v]), SOFT_SEAM);
            }
        }
        let result = self.apply_patch(&patch).map_err(|_| DEGENERATE)?;
        let added: Vec<FaceId> = result.faces.clone();
        if crate::patch::improper_contact(&result.object, &added).is_some() {
            return Err(ShapeError::SelfIntersecting { limit: None });
        }
        Ok(result.object)
    }
}

/// How far (meters) an imprint edge, moved back onto the sketch plane, may
/// lie from a source edge and still count as running along it.
const ON_SOURCE_EDGE: f64 = 1e-7;

/// The source edge of the projected shape a straight-depth wall stands on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WallSource {
    /// Region, ring (0 outer, `i + 1` hole `i`) and edge index.
    edge: (usize, usize, usize),
    curve: Option<CurveGeom>,
    /// Region, ring and smooth-chain key.
    chain: Option<(usize, usize, u32)>,
}

/// The source edge the imprint edge `a → b` runs along: both ends, moved
/// along the projection onto the sketch plane, lie on it.
fn wall_source(spec: &DepthSpec, a: Point3, b: Point3) -> Option<WallSource> {
    let first = spec.regions.first()?.outer.first()?;
    let plane = Plane::from_point_normal(*first, spec.dir).ok()?;
    let a = along_onto(a, spec.dir, &plane)?;
    let b = along_onto(b, spec.dir, &plane)?;
    for (ri, r) in spec.regions.iter().enumerate() {
        for (ring, pts) in std::iter::once(&r.outer).chain(&r.holes).enumerate() {
            let n = pts.len();
            for k in 0..n {
                let (s, t) = (pts[k], pts[(k + 1) % n]);
                if point_segment_distance(a, s, t) <= ON_SOURCE_EDGE
                    && point_segment_distance(b, s, t) <= ON_SOURCE_EDGE
                {
                    let (curve, key) = r.edge_curve(ring, k);
                    return Some(WallSource {
                        edge: (ri, ring, k),
                        curve,
                        chain: key.map(|c| (ri, ring, c)),
                    });
                }
            }
        }
    }
    None
}

/// Whether walls on source edges `p` and `c` meet at a soft seam: two on
/// one source edge are coplanar, and two on one circle or smooth chain
/// shade as that curve.
fn soft_between(p: WallSource, c: WallSource) -> bool {
    p.edge == c.edge
        || (p.curve.is_some() && p.curve == c.curve && p.edge.0 == c.edge.0 && p.edge.1 == c.edge.1)
        || (p.chain.is_some() && p.chain == c.chain)
}

/// The vertices whose wall seam is soft, from the source edges of the walls
/// meeting at each vertex of the region's rim (exactly two walls meet at a
/// rim vertex; anything else stays hard).
fn soft_wall_seams(walls_at: &BTreeMap<VertexId, Vec<Option<WallSource>>>) -> Vec<VertexId> {
    walls_at
        .iter()
        .filter_map(|(&v, walls)| match walls.as_slice() {
            [Some(p), Some(c)] if soft_between(*p, *c) => Some(v),
            _ => None,
        })
        .collect()
}

/// Edge metadata for a soft seam between two walls.
const SOFT_SEAM: EdgeMeta = EdgeMeta {
    soft: true,
    crease: false,
    curve: None,
    polygon: None,
};

fn pair(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

/// The region of a depth spec, checked.
fn check_depth(obj: &Object, spec: &DepthSpec) -> Result<BTreeSet<FaceId>, ShapeError> {
    if obj.watertight() != WatertightState::Watertight {
        return Err(ShapeError::ShapeNeedsSolid);
    }
    if !spec.depth.is_finite() || spec.depth.abs() <= tol::POINT_MERGE {
        return Err(ShapeError::AmountOutOfRange);
    }
    let region: BTreeSet<FaceId> = spec.faces.iter().copied().collect();
    if region.is_empty()
        || region.iter().any(|&f| {
            obj.faces()
                .get(f)
                .is_none_or(|face| face.plane.normal().dot(spec.dir) >= -tol::NORMAL_DIRECTION)
        })
    {
        return Err(DEGENERATE);
    }
    Ok(region)
}

fn plain(material: crate::material::FaceMaterial) -> FaceAttrs {
    FaceAttrs {
        material,
        back_material: None,
        uv_frame: None,
        surface: None,
        blend: None,
        attrs: crate::topo::AttrSlot::NONE,
    }
}

/// Adds a follow-mode face via [`push_face`].
fn add_maybe_triangulated(
    obj: &Object,
    patch: &mut PolyPatch,
    mut rings: Vec<Vec<usize>>,
    attrs: FaceAttrs,
) -> Result<(), ShapeError> {
    let outer = rings.remove(0);
    let claim_tol = obj.planarity_tol.max(tol::PLANE_DIST);
    push_face(obj, patch, outer, rings, attrs, None, claim_tol)
        .map(|_| ())
        .ok_or(ShapeError::WallTooThick {
            at: Point3::new(0.0, 0.0, 0.0),
        })
}

/// Straight mode: builds the prism the region sweeps along `dir` by the
/// depth, booleans it (subtract to engrave, union to emboss), and merges
/// the coplanar seams. The boolean's own refusals pass through unchanged.
pub(crate) fn project_depth_straight(
    obj: &Object,
    spec: &DepthSpec,
) -> Result<Object, DocumentError> {
    let region = check_depth(obj, spec).map_err(DocumentError::Shape)?;
    let delta = spec.dir * spec.depth;
    let mut patch = PolyPatch {
        expect_watertight: true,
        ..PolyPatch::default()
    };
    let mut near: BTreeMap<VertexId, usize> = BTreeMap::new();
    let mut far: BTreeMap<VertexId, usize> = BTreeMap::new();
    let mut walls_at: BTreeMap<VertexId, Vec<Option<WallSource>>> = BTreeMap::new();
    let slot =
        |patch: &mut PolyPatch, map: &mut BTreeMap<VertexId, usize>, v: VertexId, p: Point3| {
            *map.entry(v).or_insert_with(|| {
                patch.verts.push(PVert::New(p));
                patch.verts.len() - 1
            })
        };
    for &f in &region {
        let face = &obj.faces()[f];
        let loops = face_loop_edges(obj, f);
        let mut near_rings = Vec::new();
        let mut far_rings = Vec::new();
        for l in &loops {
            let mut nr = Vec::new();
            let mut fr = Vec::new();
            for &(v, _) in l {
                let p = obj.vertices()[v].position;
                nr.push(slot(&mut patch, &mut near, v, p));
                fr.push(slot(&mut patch, &mut far, v, p + delta));
            }
            fr.reverse();
            near_rings.push(nr);
            far_rings.push(fr);
        }
        let mut push = |rings: Vec<Vec<usize>>, material| {
            let mut rings = rings;
            let outer = rings.remove(0);
            patch.add.push(PatchFace {
                outer,
                holes: rings,
                attrs: plain(material),
                plane: None,
            });
        };
        push(near_rings, None);
        push(far_rings, face.material);
        for l in &loops {
            for k in 0..l.len() {
                let (a, e) = l[k];
                let b = l[(k + 1) % l.len()].0;
                if edge_faces(obj, e).iter().all(|g| region.contains(g)) {
                    continue;
                }
                let pos = |v: VertexId| obj.vertices()[v].position;
                let src = wall_source(spec, pos(a), pos(b));
                walls_at.entry(a).or_default().push(src);
                walls_at.entry(b).or_default().push(src);
                let mut attrs = plain(None);
                attrs.surface = src.and_then(|s| s.curve).map(|g| SurfaceRef::Cylinder {
                    axis_point: g.center,
                    axis: spec.dir,
                    radius: g.radius,
                });
                let wall = vec![near[&b], near[&a], far[&a], far[&b]];
                patch.add.push(PatchFace {
                    outer: wall,
                    holes: Vec::new(),
                    attrs,
                    plane: None,
                });
            }
        }
    }
    for v in soft_wall_seams(&walls_at) {
        patch.edge_meta.insert(pair(near[&v], far[&v]), SOFT_SEAM);
    }
    let shape = DocumentError::Shape;
    let mut tool = Object::empty()
        .apply_patch(&patch)
        .map_err(|_| shape(DEGENERATE))?
        .object;
    if tool.signed_volume() < 0.0 {
        let flipped = PolyPatch {
            add: patch
                .add
                .iter()
                .map(|f| PatchFace {
                    outer: f.outer.iter().rev().copied().collect(),
                    holes: f
                        .holes
                        .iter()
                        .map(|h| h.iter().rev().copied().collect())
                        .collect(),
                    attrs: f.attrs,
                    plane: None,
                })
                .collect(),
            ..patch.clone()
        };
        tool = Object::empty()
            .apply_patch(&flipped)
            .map_err(|_| shape(DEGENERATE))?
            .object;
    }
    tool.validate().map_err(|_| shape(DEGENERATE))?;
    let op = if spec.depth > 0.0 {
        BooleanOp::Subtract
    } else {
        BooleanOp::Union
    };
    let mut result =
        Object::boolean(op, obj, &tool, &Transform::IDENTITY).map_err(|e| match e {
            BooleanError::EmptyResult => shape(ShapeError::AmountOutOfRange),
            e => DocumentError::Boolean(e),
        })?;
    result.merge_coplanar_faces(&obj.coplanar_edge_segments());
    result.check_invariants();
    result.validate().map_err(|_| shape(DEGENERATE))?;
    if result.face_components().len() > obj.face_components().len() {
        return Err(shape(ShapeError::WouldSever));
    }
    Ok(result)
}

/// [`project_depth_straight`] for the executable specs. Test support only.
#[doc(hidden)]
pub fn project_depth_straight_for_test(
    obj: &Object,
    spec: &DepthSpec,
) -> Result<Object, DocumentError> {
    project_depth_straight(obj, spec)
}
