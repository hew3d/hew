//! Shell (true-curves.md §5.6): hollows a solid to a wall of constant
//! thickness, leaving chosen faces open. Built on the offset engine
//! ([`crate::offset3d`]) and the patch primitive.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ids::{FaceId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::offset3d::{offset_claim, offset_vertex_in_plane, offset_vertices, push_face};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::shape_error::ShapeError;
use crate::tol;
use crate::topo::{FaceAttrs, Object, WatertightState};

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

/// Exactly the inputs of one Shell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellSpec {
    /// The faces left open (removed, with a rim where each was).
    #[serde(with = "serde_face_ids")]
    pub open: Vec<FaceId>,
    /// The wall thickness in meters. `> 0`: the wall grows inward (the
    /// outer size is kept). `< 0`: it grows outward (the inner size is
    /// kept).
    pub thickness: f64,
}

/// The patch under construction: vertex slots for existing and offset
/// points, the faces added, and each added face's source face (for
/// refusals that name a place).
struct Builder<'a> {
    obj: &'a Object,
    patch: PolyPatch,
    existing: BTreeMap<VertexId, usize>,
    offset: BTreeMap<VertexId, usize>,
    source: Vec<FaceId>,
}

impl<'a> Builder<'a> {
    fn existing(&mut self, v: VertexId) -> usize {
        *self.existing.entry(v).or_insert_with(|| {
            self.patch.verts.push(PVert::Existing(v));
            self.patch.verts.len() - 1
        })
    }

    fn pos(&self, i: usize) -> Point3 {
        match self.patch.verts[i] {
            PVert::Existing(v) => self.obj.vertices()[v].position,
            PVert::New(p) => p,
        }
    }

    /// A face's loops as vertex ids: outer first, then holes.
    fn loops(&self, f: FaceId) -> Vec<Vec<VertexId>> {
        let face = &self.obj.faces()[f];
        std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .map(|l| {
                self.obj
                    .loop_half_edges(l)
                    .map(|h| self.obj.half_edges()[h].origin)
                    .collect()
            })
            .collect()
    }

    /// Adds a face via [`push_face`], recording `src` for each face added.
    fn add_face(
        &mut self,
        outer: Vec<usize>,
        holes: Vec<Vec<usize>>,
        attrs: FaceAttrs,
        plane: Option<Plane>,
        src: FaceId,
        claim_tol: f64,
    ) -> Result<(), ShapeError> {
        let n = push_face(
            self.obj,
            &mut self.patch,
            outer,
            holes,
            attrs,
            plane,
            claim_tol,
        )
        .ok_or_else(|| self.too_thick(src))?;
        self.source.extend(std::iter::repeat_n(src, n));
        Ok(())
    }

    fn too_thick(&self, f: FaceId) -> ShapeError {
        ShapeError::WallTooThick {
            at: face_centroid(self.obj, f),
        }
    }
}

fn key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn face_centroid(obj: &Object, f: FaceId) -> Point3 {
    let pts: Vec<Point3> = obj.loop_positions(obj.faces()[f].outer_loop).collect();
    let n = pts.len().max(1) as f64;
    let s = pts
        .iter()
        .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| acc + p.to_vec());
    Point3::new(s.x / n, s.y / n, s.z / n)
}

/// The Newell normal (unnormalized) of a ring.
fn newell(pts: &[Point3]) -> Vec3 {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        n = n + Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    n
}

/// Whether `hole` lies inside `outer` (both rings in one plane with normal
/// `n`) without touching it: some hole vertex strictly inside and no edge
/// of one meeting an edge of the other.
fn ring_nests(outer: &[Point3], hole: &[Point3], n: Vec3) -> bool {
    hole.iter()
        .any(|&p| crate::geom2d::point_inside_polygon(p, outer, n))
        && !rings_meet(outer, hole)
}

/// Whether some edge of `a` meets some edge of `b`: a crossing (decided
/// exactly) or a touch within `POINT_MERGE` (a distance in meters).
fn rings_meet(a: &[Point3], b: &[Point3]) -> bool {
    (0..a.len()).any(|i| {
        let (p, q) = (a[i], a[(i + 1) % a.len()]);
        (0..b.len()).any(|k| crate::geom2d::segments_intersect(p, q, b[k], b[(k + 1) % b.len()]))
    })
}

/// An opened pocket floor: the floor's loop, the walls standing on it and
/// the top they rise to. Under a wall thinner than the pocket is deep, the
/// walls' offsets stop at the floor's plane, where the rim rings the
/// opening. Under one at least as thick, they would have no height (or turn
/// over), so the walls run on down through the top's wall instead and the
/// opening is a hole through it: the top's inner face takes the pocket's
/// outline, carried down the walls, as its hole.
struct Pocket {
    /// The walls, in the floor loop's order (wall `k` stands on the floor
    /// edge from `floor[k]` to `floor[k + 1]`).
    walls: Vec<FaceId>,
    /// The floor's loop.
    floor: Vec<VertexId>,
    /// Each floor vertex's mouth vertex, the far end of its side edge on
    /// the top face.
    mouth: Vec<VertexId>,
    /// The face the walls rise to; the mouth is one of its holes.
    top: FaceId,
}

impl Object {
    /// Hollows this solid to a wall `spec.thickness` thick, removing the
    /// `spec.open` faces and closing each with a rim; with nothing open the
    /// result is a closed hollow (two shells in one object). Pure.
    ///
    /// An open pocket floor has its rim in the floor's plane, ringing the
    /// opening and facing the cavity, while the pocket is deeper than the
    /// wall. Under a wall at least as thick as the pocket is deep, the
    /// pocket's walls run on down through the top's wall instead, and the
    /// opening is a hole through it with the pocket's outline (see
    /// [`Pocket`]).
    ///
    /// # Errors
    /// - `ShapeNeedsSolid`: the object is not watertight.
    /// - `AmountOutOfRange`: the thickness is not finite or within
    ///   `POINT_MERGE` of zero, or every face is open.
    /// - `ShapeDegenerate`: an open face id is not a face of this object.
    /// - `UnsupportedShellCorner { at }`: the offset surfaces at a vertex do
    ///   not meet in one point.
    /// - `WallTooThick { at }`: an offset face turns over, an offset edge or
    ///   claim radius collapses (`at`: the source face's centroid), or a
    ///   hole's offset runs into the face's offset outline or another
    ///   hole's, as where a pocket's walls would meet the side walls (`at`:
    ///   the hole's centroid).
    /// - `SelfIntersecting { limit: None }`: the walls pass through each
    ///   other.
    pub fn shelled(&self, spec: &ShellSpec) -> Result<Object, ShapeError> {
        if self.watertight() != WatertightState::Watertight {
            return Err(ShapeError::ShapeNeedsSolid);
        }
        if !spec.thickness.is_finite() || spec.thickness.abs() <= tol::POINT_MERGE {
            return Err(ShapeError::AmountOutOfRange);
        }
        let open: BTreeSet<FaceId> = spec.open.iter().copied().collect();
        if open.iter().any(|&f| !self.faces().contains_key(f)) {
            return Err(ShapeError::ShapeDegenerate);
        }
        let kept: BTreeSet<FaceId> = self.faces().keys().filter(|f| !open.contains(f)).collect();
        if kept.is_empty() {
            return Err(ShapeError::AmountOutOfRange);
        }
        let inward = spec.thickness > 0.0;
        let d = spec.thickness.abs();
        // A claimed surface whose offset radius runs out has no room for the
        // wall; say so before the vertex solve fails to converge on it.
        for &f in &kept {
            if self.faces()[f].surface.is_some()
                && offset_claim(self, f, if inward { d } else { -d }).is_none()
            {
                return Err(ShapeError::WallTooThick {
                    at: face_centroid(self, f),
                });
            }
        }
        let mut moved = offset_vertices(self, &kept, if inward { d } else { -d })?;
        let claim_tol = self.planarity_tol.max(tol::PLANE_DIST);

        // Opened pocket floors (inward). Under a wall at least as thick as
        // the pocket is deep, the walls run on down to the top's inner face,
        // which takes the pocket's outline as its hole in place of the
        // walls' offset ring. Otherwise each floor vertex's offset point
        // slides along its side edge into the floor's plane, so the rim
        // lies on the floor however the walls lean (the offset constraints
        // hold along the side edge: it runs along both walls' offsets, or
        // along a curved wall's ruling), and the walls shrink to no height
        // exactly where the hole through the top takes over.
        let mut tunnels: BTreeMap<FaceId, (Pocket, Vec<Option<Point3>>)> = BTreeMap::new();
        if inward {
            for &o in &open {
                let Some(p) = self.pocket(o, &open) else {
                    continue;
                };
                if let Some(exit) = self.tunnel_exits(&p, d) {
                    tunnels.insert(o, (p, exit));
                    continue;
                }
                let floor_plane = self.faces()[o].plane;
                let n_floor = floor_plane.normal();
                for (&v, &u) in p.floor.iter().zip(&p.mouth) {
                    let pv = self.vertices()[v].position;
                    let e = pv - self.vertices()[u].position;
                    let Some(x) = moved.get_mut(&v) else { continue };
                    // Along the side edge first: exact for flat walls and
                    // for a cylinder's or cone's ruling. A wall on a sphere
                    // or torus has a chord for a side edge, so the point is
                    // then settled onto the offset surfaces and the floor's
                    // plane together.
                    let mu = -n_floor.dot(*x - pv) / n_floor.dot(e);
                    *x = offset_vertex_in_plane(self, &kept, v, d, *x + e * mu, &floor_plane)
                        .ok_or(ShapeError::UnsupportedShellCorner { at: pv })?;
                }
            }
        }

        let mut b = Builder {
            obj: self,
            patch: PolyPatch {
                expect_watertight: true,
                ..PolyPatch::default()
            },
            existing: BTreeMap::new(),
            offset: BTreeMap::new(),
            source: Vec::new(),
        };
        for (&v, &p) in &moved {
            b.patch.verts.push(PVert::New(p));
            b.offset.insert(v, b.patch.verts.len() - 1);
        }
        b.patch.remove = open.iter().copied().collect();
        if !inward {
            b.patch.remove.extend(kept.iter().copied());
        }

        let mut carried: BTreeSet<FaceId> = BTreeSet::new();
        for (t, exit) in tunnels.values() {
            for (k, &u) in t.mouth.iter().enumerate() {
                let end = match exit[k] {
                    Some(p) => {
                        b.patch.verts.push(PVert::New(p));
                        b.patch.verts.len() - 1
                    }
                    None => b.existing(t.floor[k]),
                };
                // The mouth's offset point is the exit now; nothing else
                // offsets the mouth (it has just the walls and the top).
                b.offset.insert(u, end);
            }
            carried.extend(t.walls.iter().copied());
        }
        b.patch.remove.extend(carried.iter().copied());

        // Offset faces, plus (outward) the originals re-added reversed.
        for &f in kept.iter().filter(|f| !carried.contains(f)) {
            let face = &self.faces()[f];
            let loops = b.loops(f);
            let off: Vec<Vec<usize>> = loops
                .iter()
                .map(|l| l.iter().map(|v| b.offset[v]).collect())
                .collect();
            // Wall too thick: an offset edge collapses, the offset ring
            // turns over, or a claimed radius runs out.
            // An offset edge that runs the other way from its source edge
            // has passed through a corner: a box offset past its middle in
            // two directions keeps its winding but not its edges.
            for (ring, src) in off.iter().zip(&loops) {
                for k in 0..ring.len() {
                    let (p, q) = (b.pos(ring[k]), b.pos(ring[(k + 1) % ring.len()]));
                    let (sp, sq) = (
                        self.vertices()[src[k]].position,
                        self.vertices()[src[(k + 1) % src.len()]].position,
                    );
                    if p.approx_eq(q, tol::POINT_MERGE) || (q - p).dot(sq - sp) <= 0.0 {
                        return Err(b.too_thick(f));
                    }
                }
            }
            let off_outer: Vec<Point3> = off[0].iter().map(|&i| b.pos(i)).collect();
            if newell(&off_outer).dot(face.plane.normal()) <= 0.0 {
                return Err(b.too_thick(f));
            }
            // A hole whose offset ring runs out past the face's offset
            // outline, or into another hole's, or touches either, is walls
            // meeting: a pocket closer to the side, or to another pocket,
            // than the wall leaves room for. (Rings that already touched
            // before the offset, as imported holes on their outline can,
            // are not walls meeting.)
            let ring_pts = |r: &[usize]| r.iter().map(|&i| b.pos(i)).collect::<Vec<Point3>>();
            let src_pts = |l: &[VertexId]| {
                l.iter()
                    .map(|&v| self.vertices()[v].position)
                    .collect::<Vec<Point3>>()
            };
            let off_holes: Vec<Vec<Point3>> = off[1..].iter().map(|r| ring_pts(r)).collect();
            let src_outer = src_pts(&loops[0]);
            let src_holes: Vec<Vec<Point3>> = loops[1..].iter().map(|l| src_pts(l)).collect();
            let n = face.plane.normal();
            let inside =
                |p: Point3, ring: &[Point3]| crate::geom2d::point_inside_polygon(p, ring, n);
            for (i, h) in off_holes.iter().enumerate() {
                let out = !h.iter().any(|&p| inside(p, &off_outer))
                    || (rings_meet(&off_outer, h) && !rings_meet(&src_outer, &src_holes[i]));
                let into = (i + 1..off_holes.len()).any(|j| {
                    let g = &off_holes[j];
                    inside(g[0], h)
                        || inside(h[0], g)
                        || (rings_meet(h, g) && !rings_meet(&src_holes[i], &src_holes[j]))
                });
                if out || into {
                    // Named at the hole (the pocket), not the face's middle.
                    let src: Vec<Point3> = loops[i + 1]
                        .iter()
                        .map(|&v| self.vertices()[v].position)
                        .collect();
                    let c = src
                        .iter()
                        .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| acc + p.to_vec())
                        / src.len() as f64;
                    return Err(ShapeError::WallTooThick {
                        at: Point3::new(c.x, c.y, c.z),
                    });
                }
            }
            let claim = offset_claim(self, f, if inward { d } else { -d });
            let offset_attrs = FaceAttrs {
                surface: claim,
                blend: None,
                ..face.attrs()
            };
            // Offset edges copy their source edge's soft and crease flags;
            // circle claims do not carry (the offset circle differs).
            for (l, ring) in loops.iter().zip(&off) {
                for k in 0..ring.len() {
                    let (a, c) = (l[k], l[(k + 1) % l.len()]);
                    let Some(meta) = self.edge_meta_between(a, c) else {
                        continue;
                    };
                    b.patch.edge_meta.insert(
                        key(ring[k], ring[(k + 1) % ring.len()]),
                        EdgeMeta {
                            soft: meta.soft,
                            crease: meta.crease,
                            ..EdgeMeta::default()
                        },
                    );
                }
            }
            if inward {
                // The inner face faces the cavity: F's loops at the offset
                // points, reversed.
                let mut rings: Vec<Vec<usize>> = off
                    .into_iter()
                    .map(|mut r| {
                        r.reverse();
                        r
                    })
                    .collect();
                let outer = rings.remove(0);
                b.add_face(outer, rings, offset_attrs, None, f, claim_tol)?;
            } else {
                let mut rings = off;
                let outer = rings.remove(0);
                b.add_face(outer, rings, offset_attrs, None, f, claim_tol)?;
                // The original, reversed: the same surface, facing the
                // cavity.
                let mut rings: Vec<Vec<usize>> = loops
                    .iter()
                    .map(|l| l.iter().rev().map(|&v| b.existing(v)).collect())
                    .collect();
                let outer = rings.remove(0);
                b.patch.add.push(PatchFace {
                    outer,
                    holes: rings,
                    attrs: face.attrs(),
                    plane: Some(face.plane.flipped()),
                });
                b.source.push(f);
            }
        }

        // The pocket walls carried down to the top's inner face: each wall
        // with its floor edge moved to the exit points (the same face where
        // the wall is exactly as thick as the pocket is deep).
        for (t, _) in tunnels.values() {
            let n = t.floor.len();
            for (k, &w) in t.walls.iter().enumerate() {
                let j = (k + 1) % n;
                let (ua, ub) = (b.existing(t.mouth[k]), b.existing(t.mouth[j]));
                let (ea, eb) = (b.offset[&t.mouth[k]], b.offset[&t.mouth[j]]);
                let wf = &self.faces()[w];
                // A claim the carried-down wall no longer fits (a sphere's
                // or torus's facet run on past its chord) drops: map or
                // drop, as every offset face's claim does.
                let outer = vec![eb, ea, ua, ub];
                let pts: Vec<Point3> = outer.iter().map(|&i| b.pos(i)).collect();
                let attrs = FaceAttrs {
                    surface: wf
                        .surface
                        .filter(|s| s.holds_on(&pts, &wf.plane, claim_tol)),
                    ..wf.attrs()
                };
                b.patch.add.push(PatchFace {
                    outer,
                    holes: Vec::new(),
                    attrs,
                    plane: Some(wf.plane),
                });
                b.source.push(w);
                // The side edge keeps its soft and crease flags along its
                // new length.
                if let Some(meta) = self.edge_meta_between(t.floor[k], t.mouth[k]) {
                    b.patch.edge_meta.insert(
                        key(ua, ea),
                        EdgeMeta {
                            soft: meta.soft,
                            crease: meta.crease,
                            ..EdgeMeta::default()
                        },
                    );
                }
            }
        }

        // Rims where the open faces were.
        for &o in open.iter().filter(|o| !tunnels.contains_key(o)) {
            self.add_rims(&mut b, o, &open, inward, claim_tol)?;
        }

        let source = b.source;
        let result = self.apply_patch(&b.patch).map_err(|e| match e {
            crate::error::TopologyError::DegenerateFace { face } => {
                source.get(face).map_or(ShapeError::ShapeDegenerate, |&f| {
                    ShapeError::WallTooThick {
                        at: face_centroid(self, f),
                    }
                })
            }
            _ => ShapeError::ShapeDegenerate,
        })?;
        if crate::patch::self_intersection(&result.object).is_some() {
            return Err(ShapeError::SelfIntersecting { limit: None });
        }
        Ok(result.object)
    }

    /// The face across the edge `a`→`b` of face `f`'s loops, if any.
    fn face_across(&self, f: FaceId, a: VertexId, b: VertexId) -> Option<FaceId> {
        let hes = self.half_edges();
        let start = self.vertices()[a].outgoing;
        let mut h = start;
        for _ in 0..hes.len() {
            if hes[hes[h].next].origin == b && self.loops()[hes[h].loop_id].face == f {
                return hes[h].twin.map(|t| self.loops()[hes[t].loop_id].face);
            }
            h = hes[hes[h].prev].twin?;
            if h == start {
                break;
            }
        }
        None
    }

    /// The faces around vertex `v`, once each, in cyclic order; `None` when
    /// the fan does not close.
    fn faces_around(&self, v: VertexId) -> Option<Vec<FaceId>> {
        let hes = self.half_edges();
        let start = self.vertices()[v].outgoing;
        let mut out = Vec::new();
        let mut h = start;
        loop {
            out.push(self.loops()[hes[h].loop_id].face);
            h = hes[hes[h].prev].twin?;
            if h == start {
                return Some(out);
            }
            if out.len() > hes.len() {
                return None;
            }
        }
    }

    /// Open face `o` as a pocket's floor: `o` has no holes and every edge
    /// of it meets a kept, unblended four-sided wall; each floor vertex has
    /// just its two walls and the floor around it; its side edge runs up to
    /// a mouth vertex with just those walls and one kept, unclaimed top face
    /// around it, the same top for every wall; the mouth is one of the
    /// top's holes (so a box opened on one face, its far face ringing the
    /// sides' other ends, is no pocket); and every side edge runs from the
    /// top down into the material. `None` otherwise.
    fn pocket(&self, o: FaceId, open: &BTreeSet<FaceId>) -> Option<Pocket> {
        let face = &self.faces()[o];
        if !face.inner_loops.is_empty() {
            return None;
        }
        let floor: Vec<VertexId> = self
            .loop_half_edges(face.outer_loop)
            .map(|h| self.half_edges()[h].origin)
            .collect();
        let n = floor.len();
        let mut walls = Vec::with_capacity(n);
        let mut mouth = Vec::with_capacity(n);
        let mut top: Option<FaceId> = None;
        for k in 0..n {
            let (a, b) = (floor[k], floor[(k + 1) % n]);
            let w = self.face_across(o, a, b)?;
            let wf = &self.faces()[w];
            if open.contains(&w) || wf.blend.is_some() || !wf.inner_loops.is_empty() {
                return None;
            }
            // The wall's loop runs b, a, then up the side edge from a to
            // its mouth vertex, across the mouth, and down to b.
            let ring: Vec<VertexId> = self
                .loop_half_edges(wf.outer_loop)
                .map(|h| self.half_edges()[h].origin)
                .collect();
            if ring.len() != 4 {
                return None;
            }
            let i = ring.iter().position(|&x| x == b)?;
            let at = |j: usize| ring[(i + j) % 4];
            if at(1) != a {
                return None;
            }
            let (ua, ub) = (at(2), at(3));
            let t = self.face_across(w, ua, ub)?;
            if top.is_some_and(|x| x != t) {
                return None;
            }
            top = Some(t);
            walls.push(w);
            mouth.push((ua, ub));
        }
        let top = top?;
        let tf = &self.faces()[top];
        if open.contains(&top) || tf.surface.is_some() || walls.contains(&top) {
            return None;
        }
        // Neighboring walls agree on the side edge between them.
        let mut up = Vec::with_capacity(n);
        for k in 0..n {
            let (ua, _) = mouth[k];
            let (_, ub_prev) = mouth[(k + n - 1) % n];
            if ua != ub_prev {
                return None;
            }
            up.push(ua);
        }
        // The mouth is a hole of the top, not its outline.
        let want: BTreeSet<VertexId> = up.iter().copied().collect();
        let is_hole = tf.inner_loops.iter().any(|&l| {
            let ring: BTreeSet<VertexId> = self
                .loop_half_edges(l)
                .map(|h| self.half_edges()[h].origin)
                .collect();
            ring == want && ring.len() == n
        });
        if !is_hole {
            return None;
        }
        let n_top = tf.plane.normal();
        let n_floor = face.plane.normal();
        for k in 0..n {
            let (v, u) = (floor[k], up[k]);
            let (prev, next) = (walls[(k + n - 1) % n], walls[k]);
            let only = |fs: Option<Vec<FaceId>>, want: [FaceId; 3]| {
                fs.is_some_and(|fs| fs.len() == 3 && want.iter().all(|w| fs.contains(w)))
            };
            if !only(self.faces_around(v), [o, prev, next])
                || !only(self.faces_around(u), [top, prev, next])
            {
                return None;
            }
            let e = self.vertices()[v].position - self.vertices()[u].position;
            let len = e.length();
            // Down from the top into the material, and into the floor from
            // above (the floor faces up the pocket).
            if len <= tol::POINT_MERGE
                || -n_top.dot(e) <= tol::NORMAL_DIRECTION * len
                || -n_floor.dot(e) <= tol::NORMAL_DIRECTION * len
            {
                return None;
            }
        }
        Some(Pocket {
            walls,
            floor,
            mouth: up,
            top,
        })
    }

    /// Where pocket `p`'s side edges, carried on past the floor, meet the
    /// top's inner face `d` under the top (`None` for one that meets it at
    /// its floor vertex), when every one meets it at or beyond its floor
    /// vertex and the floor edges carried down keep their length and
    /// direction. `None` when the pocket is deeper than the wall (the rim
    /// on the floor), or when walls leaning in toward each other (a
    /// countersink) would cross before they reach the inner face.
    fn tunnel_exits(&self, p: &Pocket, d: f64) -> Option<Vec<Option<Point3>>> {
        let n_top = self.faces()[p.top].plane.normal();
        let n = p.floor.len();
        let pos = |v: VertexId| self.vertices()[v].position;
        let mut exit = Vec::with_capacity(n);
        for k in 0..n {
            let (pv, pu) = (pos(p.floor[k]), pos(p.mouth[k]));
            let e = pv - pu;
            let len = e.length();
            // The top's inner face lies `d` under the top: the side edge
            // from the mouth reaches it at `d / down` of its length.
            let lambda = d / -n_top.dot(e);
            if (lambda - 1.0) * len < -tol::POINT_MERGE {
                return None;
            }
            let w = pu + e * lambda;
            exit.push((!w.approx_eq(pv, tol::POINT_MERGE)).then_some(w));
        }
        let at = |k: usize| exit[k % n].unwrap_or_else(|| pos(p.floor[k % n]));
        for k in 0..n {
            let (pa, pb) = (pos(p.floor[k]), pos(p.floor[(k + 1) % n]));
            let (ea, eb) = (at(k), at(k + 1));
            if ea.approx_eq(eb, tol::POINT_MERGE) || (eb - ea).dot(pb - pa) <= 0.0 {
                return None;
            }
        }
        Some(exit)
    }

    /// The soft/crease/curve metadata of the edge joining `a` and `b`.
    fn edge_meta_between(&self, a: VertexId, b: VertexId) -> Option<EdgeMeta> {
        let start = self.vertices()[a].outgoing;
        let hes = self.half_edges();
        let mut h = start;
        for _ in 0..hes.len() {
            if hes[hes[h].next].origin == b {
                let e = &self.edges()[hes[h].edge];
                return Some(EdgeMeta {
                    soft: e.soft,
                    crease: e.crease,
                    curve: e.curve,
                    polygon: e.polygon,
                });
            }
            h = hes[hes[h].prev].twin?;
            if h == start {
                break;
            }
        }
        None
    }

    /// The rim faces closing open face `o` between its loops and their
    /// offsets. A rim lying in `o`'s plane is one face per loop (an annulus
    /// around the opening); otherwise one quad per loop edge, split in two
    /// triangles when it is not planar.
    fn add_rims(
        &self,
        b: &mut Builder<'_>,
        o: FaceId,
        open: &BTreeSet<FaceId>,
        inward: bool,
        claim_tol: f64,
    ) -> Result<(), ShapeError> {
        let face = &self.faces()[o];
        let attrs = FaceAttrs {
            surface: None,
            blend: None,
            ..face.attrs()
        };
        let loops = b.loops(o);
        for (li, l) in loops.iter().enumerate() {
            // A wall stands only where `o` meets a kept face: an edge
            // between two opened faces has no material on either side, and
            // a corner where every face is open has no offset point.
            let walled: Vec<bool> = (0..l.len())
                .map(|k| {
                    self.face_across(o, l[k], l[(k + 1) % l.len()])
                        .is_some_and(|f| !open.contains(&f))
                })
                .collect();
            if !walled.iter().any(|&w| w) {
                continue;
            }
            let a: Vec<usize> = l.iter().map(|&v| b.existing(v)).collect();
            let s: Vec<Option<usize>> = l.iter().map(|v| b.offset.get(v).copied()).collect();
            let whole = walled.iter().all(|&w| w) && s.iter().all(Option::is_some);
            let in_plane = whole
                && a.iter()
                    .chain(s.iter().flatten())
                    .all(|&i| face.plane.signed_distance(b.pos(i)).abs() <= tol::PLANE_DIST);
            let rev = |r: &[usize]| r.iter().rev().copied().collect::<Vec<usize>>();
            if in_plane {
                let s: Vec<usize> = s.iter().flatten().copied().collect();
                let is_outer = li == 0;
                let (outer, hole) = match (inward, is_outer) {
                    (true, true) => (a.clone(), rev(&s)),
                    (true, false) => (rev(&s), a.clone()),
                    (false, true) => (s.clone(), rev(&a)),
                    (false, false) => (rev(&a), s.clone()),
                };
                // Past a concave opening (a pocket's floor or wall) the
                // offset ring grows outside the opening instead of inside
                // it, and the rim caps the walls from the other side: the
                // rings trade places and the rim faces the other way.
                let pts = |r: &[usize]| r.iter().map(|&i| b.pos(i)).collect::<Vec<Point3>>();
                let n = face.plane.normal();
                let (po, ph) = (pts(&outer), pts(&hole));
                let (outer, hole, plane) = if ring_nests(&po, &ph, n) {
                    (outer, hole, face.plane)
                } else if ring_nests(&ph, &po, n) {
                    (hole, outer, face.plane.flipped())
                } else {
                    // The offset ring crosses the opening (a pocket's wall:
                    // wider along the floor, lower under the top): no
                    // annulus, and no single rim face to build.
                    return Err(ShapeError::UnsupportedShellCorner { at: po[0] });
                };
                b.add_face(outer, vec![hole], attrs, Some(plane), o, claim_tol)?;
                continue;
            }
            for k in 0..a.len() {
                if !walled[k] {
                    continue;
                }
                let j = (k + 1) % a.len();
                let (Some(sk), Some(sj)) = (s[k], s[j]) else {
                    return Err(ShapeError::UnsupportedShellCorner { at: b.pos(a[k]) });
                };
                let quad = if inward {
                    vec![a[k], a[j], sj, sk]
                } else {
                    vec![a[j], a[k], sk, sj]
                };
                // The winding is forced by the twins. Where `o` is tangent to
                // its neighbors the rim stands perpendicular to it, so only a
                // collapsed quad is refused here; a rim that turns over
                // crosses the walls and is caught by the self-intersection
                // check.
                let pts: Vec<Point3> = quad.iter().map(|&i| b.pos(i)).collect();
                if newell(&pts).length() <= tol::NORMALIZE_MIN_LENGTH {
                    return Err(b.too_thick(o));
                }
                b.add_face(quad, Vec::new(), attrs, None, o, claim_tol)?;
            }
        }
        Ok(())
    }
}
