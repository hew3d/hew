//! Restoring the sharp geometry under a blend (WP08 §2): re-derive each
//! neighbour's corner from the planes around the blend, re-create the faces
//! the blend consumed, and accept the result only when re-blending the
//! restored edges reproduces the blend.

use crate::trig;
use std::collections::{BTreeMap, BTreeSet};

use super::BlendError;
use crate::ids::{FaceId, HalfEdgeId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::tol;
use crate::topo::{FaceAttrs, Object, SurfaceRef};

/// A plane taking part in the restoration: a face of the object, or a face
/// the restoration re-creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Face(FaceId),
    Virt(usize),
}

/// A face the blend consumed, re-created: its plane, the cyclic ring of the
/// planes meeting it (corner `i` lies on it, `ring[i]` and `ring[i+1]`), its
/// attributes and outward normal.
pub(crate) struct Virtual {
    pub plane: Plane,
    pub ring: Vec<Key>,
    pub attrs: FaceAttrs,
    pub outward: Vec3,
}

/// What the consumed faces were, for the measured profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consumed {
    None,
    Cap,
    Bullnose,
    Facets,
}

/// The patch restoring a blend, before it is applied.
pub(crate) struct Restoration {
    pub patch: PolyPatch,
    /// The re-added neighbours, in patch order (then the virtual faces).
    pub neighbours: Vec<FaceId>,
    /// Patch point indices that are restored corners.
    pub restored_pts: BTreeSet<usize>,
    pub consumed: Consumed,
    /// A bullnose's two side faces (its restored edges lie against them).
    pub sides: Option<(FaceId, FaceId)>,
    /// The re-created face (patch index past the neighbours) or flat face
    /// every restored edge must touch when the blend consumed faces: the
    /// cap of a hemisphere, the flat rail of a D-post crossing.
    pub touching: Option<Key>,
    /// Face pairs (original ids) whose restored sharp edge a stopped
    /// blend's run continued.
    pub continued: BTreeSet<(FaceId, FaceId)>,
}

/// The exact intersection of three planes.
pub(crate) fn three_planes(a: &Plane, b: &Plane, c: &Plane) -> Option<Point3> {
    let (na, nb, nc) = (a.normal(), b.normal(), c.normal());
    let det = na.dot(nb.cross(nc));
    if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let (da, db, dc) = (
        na.dot(a.point().to_vec()),
        nb.dot(b.point().to_vec()),
        nc.dot(c.point().to_vec()),
    );
    let v = (nb.cross(nc) * da + nc.cross(na) * db + na.cross(nb) * dc) / det;
    Some(Point3::new(v.x, v.y, v.z))
}

struct Restorer<'a> {
    obj: &'a Object,
    region: &'a BTreeSet<FaceId>,
    neighbours: BTreeSet<FaceId>,
    virtuals: Vec<Virtual>,
    verts: Vec<PVert>,
    existing: BTreeMap<VertexId, usize>,
    triples: BTreeMap<[Key; 3], usize>,
    restored: BTreeSet<usize>,
    /// Face pairs whose sharp edge a run merely continued.
    continued: BTreeSet<(FaceId, FaceId)>,
}

impl Restorer<'_> {
    fn plane(&self, k: Key) -> Plane {
        match k {
            Key::Face(f) => self.obj.faces()[f].plane,
            Key::Virt(i) => self.virtuals[i].plane,
        }
    }

    fn existing(&mut self, v: VertexId) -> usize {
        if let Some(&i) = self.existing.get(&v) {
            return i;
        }
        self.verts.push(PVert::Existing(v));
        let i = self.verts.len() - 1;
        self.existing.insert(v, i);
        i
    }

    /// The restored corner of three planes, computed once.
    fn point(&mut self, a: Key, b: Key, c: Key) -> Result<usize, BlendError> {
        let mut key = [a, b, c];
        key.sort();
        if key[0] == key[1] || key[1] == key[2] {
            return Err(BlendError::BlendModified);
        }
        if let Some(&i) = self.triples.get(&key) {
            return Ok(i);
        }
        let p = three_planes(&self.plane(a), &self.plane(b), &self.plane(c))
            .ok_or(BlendError::BlendModified)?;
        self.verts.push(PVert::New(p));
        let i = self.verts.len() - 1;
        self.triples.insert(key, i);
        self.restored.insert(i);
        Ok(i)
    }

    /// Whether every face at `v` outside the region is `a` or `b`.
    fn only_between(&self, v: VertexId, a: FaceId, b: FaceId) -> bool {
        let es = self.obj.edges_at_vertex(v);
        !es.is_empty()
            && es
                .into_iter()
                .filter_map(|e| self.obj.edge_faces(e))
                .flat_map(|(x, y)| [x, y])
                .filter(|f| !self.region.contains(f))
                .all(|f| f == a || f == b)
    }

    fn pos(&self, i: usize) -> Point3 {
        match self.verts[i] {
            PVert::New(p) => p,
            PVert::Existing(v) => self.obj.vertices()[v].position,
        }
    }

    fn twin_face(&self, h: HalfEdgeId) -> Option<FaceId> {
        let t = self.obj.half_edges()[h].twin?;
        Some(self.obj.loops()[self.obj.half_edges()[t].loop_id].face)
    }

    /// WP08 §2.2 step 2: for each run edge of face `f`, the plane the ray
    /// from its midpoint away from `f`'s interior meets first (behind, over
    /// a concave blend); consecutive duplicates collapsed, `p`/`q` dropped.
    fn across(
        &self,
        f: FaceId,
        run: &[HalfEdgeId],
        p: Option<Key>,
        q: Option<Key>,
    ) -> Result<Vec<Key>, BlendError> {
        let obj = self.obj;
        let n_f = obj.faces()[f].plane.normal();
        let cands: Vec<Key> = self
            .neighbours
            .iter()
            .map(|&g| Key::Face(g))
            .chain((0..self.virtuals.len()).map(Key::Virt))
            .filter(|&k| k != Key::Face(f))
            .collect();
        let mut seq: Vec<Key> = Vec::new();
        for &h in run {
            let he = obj.half_edges()[h];
            let a = obj.vertices()[he.origin].position;
            let b = obj.vertices()[obj.half_edges()[he.next].origin].position;
            let d = (b - a)
                .normalized()
                .map_err(|_| BlendError::BlendModified)?;
            let s = d.cross(n_f);
            let m = a + (b - a) * 0.5;
            let mut fwd: Option<(f64, Key)> = None;
            let mut back: Option<(f64, Key)> = None;
            for &k in &cands {
                let pk = self.plane(k);
                let nk = pk.normal();
                if nk.dot(n_f).abs() >= 1.0 - tol::NORMAL_DIRECTION {
                    continue;
                }
                let den = nk.dot(s);
                if den.abs() <= tol::NORMALIZE_MIN_LENGTH {
                    continue;
                }
                let lam = -pk.signed_distance(m) / den;
                if lam > tol::POINT_MERGE && fwd.is_none_or(|(bl, _)| lam < bl) {
                    fwd = Some((lam, k));
                }
                if lam < -tol::POINT_MERGE && back.is_none_or(|(bl, _)| lam > bl) {
                    back = Some((lam, k));
                }
            }
            let (_, k) = fwd.or(back).ok_or(BlendError::BlendModified)?;
            if seq.last() != Some(&k) {
                seq.push(k);
            }
        }
        seq.retain(|k| Some(*k) != p && Some(*k) != q);
        // Across re-created faces, follow their own order: the shortest
        // chain from P to Q through them.
        if let (Some(p), Some(q)) = (p, q)
            && seq.iter().any(|k| matches!(k, Key::Virt(_)))
            && let Some(path) = self.virtual_chain(p, q)
        {
            return Ok(path);
        }
        Ok(seq)
    }

    /// The re-created faces between `p` and `q` (exclusive), in order: a
    /// breadth-first chain whose inner nodes are all re-created faces.
    fn virtual_chain(&self, p: Key, q: Key) -> Option<Vec<Key>> {
        let adjacent = |a: Key, b: Key| {
            self.virtuals.iter().enumerate().any(|(i, v)| {
                // A re-created facet lies between ring[0] and ring[2].
                let me = Key::Virt(i);
                let (x, y) = (v.ring[0], v.ring[v.ring.len().min(3) - 1]);
                let y = if v.ring.len() >= 3 { v.ring[2] } else { y };
                (me == a && (x == b || y == b)) || (me == b && (x == a || y == a))
            })
        };
        let virts: Vec<Key> = (0..self.virtuals.len()).map(Key::Virt).collect();
        let mut prev: BTreeMap<Key, Key> = BTreeMap::new();
        let mut queue = std::collections::VecDeque::from([p]);
        let mut seen = BTreeSet::from([p]);
        while let Some(k) = queue.pop_front() {
            if k != p && adjacent(k, q) {
                let mut path = vec![k];
                let mut c = k;
                while let Some(&b) = prev.get(&c) {
                    if b == p {
                        break;
                    }
                    path.push(b);
                    c = b;
                }
                path.reverse();
                return Some(path);
            }
            for &v in &virts {
                if !seen.contains(&v) && adjacent(k, v) {
                    seen.insert(v);
                    prev.insert(v, k);
                    queue.push_back(v);
                }
            }
        }
        None
    }

    /// The restored copy of neighbour `f`.
    fn restore_neighbour(&mut self, f: FaceId) -> Result<PatchFace, BlendError> {
        let modified = BlendError::BlendModified;
        let obj = self.obj;
        let face = &obj.faces()[f];
        let mut rings: Vec<Vec<usize>> = Vec::new();
        for l in obj.face_loops(f) {
            let hs: Vec<HalfEdgeId> = obj.loop_half_edges(l).collect();
            let k = hs.len();
            let in_region = |s: &Self, i: usize| {
                s.twin_face(hs[i % k])
                    .is_some_and(|g| s.region.contains(&g))
            };
            let mut ring: Vec<usize> = Vec::new();
            if (0..k).all(|i| in_region(self, i)) {
                let across = self.across(f, &hs, None, None)?;
                if across.len() < 2 {
                    return Err(modified);
                }
                for i in 0..across.len() {
                    ring.push(self.point(
                        Key::Face(f),
                        across[i],
                        across[(i + 1) % across.len()],
                    )?);
                }
                rings.push(ring);
                continue;
            }
            let start = (0..k)
                .find(|&i| !in_region(self, i))
                .expect("some edge is outside the region");
            let mut i = 0;
            while i < k {
                let idx = (start + i) % k;
                if !in_region(self, idx) {
                    // A run's last vertex is replaced by its path.
                    if !in_region(self, idx + k - 1) {
                        let v = obj.half_edges()[hs[idx]].origin;
                        ring.push(self.existing(v));
                    }
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < k && in_region(self, (start + j) % k) {
                    j += 1;
                }
                let run: Vec<HalfEdgeId> = (i..j).map(|t| hs[(start + t) % k]).collect();
                let p = Key::Face(
                    self.twin_face(hs[(start + i + k - 1) % k])
                        .ok_or(modified)?,
                );
                let q = Key::Face(self.twin_face(hs[(start + j) % k]).ok_or(modified)?);
                let across = self.across(f, &run, Some(p), Some(q))?;
                // A run that starts and ends against the same face (a
                // stopped blend's fan, from R to R'): the sharp edge just
                // continues; its end vertices go when nothing else holds them.
                if p == q && across.is_empty() {
                    let (w0, wm) = (
                        obj.half_edges()[run[0]].origin,
                        obj.half_edges()[hs[(start + j) % k]].origin,
                    );
                    let Key::Face(pf) = p else {
                        return Err(modified);
                    };
                    for w in [w0, wm] {
                        if !self.only_between(w, f, pf) {
                            ring.push(self.existing(w));
                        }
                    }
                    self.continued.insert((f.min(pf), f.max(pf)));
                    i = j;
                    continue;
                }
                let mut chain = vec![p];
                chain.extend(across);
                chain.push(q);
                for w in chain.windows(2) {
                    ring.push(self.point(Key::Face(f), w[0], w[1])?);
                }
                i = j;
            }
            ring.dedup();
            while ring.len() > 1 && ring.first() == ring.last() {
                ring.pop();
            }
            rings.push(ring);
        }
        let mut it = rings.into_iter();
        let outer = it.next().ok_or(modified)?;
        let mut attrs = face.attrs();
        if let Some(claim) = attrs.surface {
            let ps: Vec<Point3> = outer.iter().map(|&i| self.pos(i)).collect();
            if !claim.holds_on(&ps, &face.plane, tol::PLANE_DIST) {
                attrs.surface = None;
            }
        }
        Ok(PatchFace {
            outer,
            holes: it.collect(),
            attrs,
            plane: Some(face.plane),
        })
    }

    /// A re-created face: its corners, oriented outward.
    fn restore_virtual(&mut self, i: usize) -> Result<PatchFace, BlendError> {
        let ring = self.virtuals[i].ring.clone();
        let n = ring.len();
        let mut out: Vec<usize> = Vec::with_capacity(n);
        for j in 0..n {
            out.push(self.point(Key::Virt(i), ring[j], ring[(j + 1) % n])?);
        }
        out.dedup();
        let ps: Vec<Point3> = out.iter().map(|&k| self.pos(k)).collect();
        let pl = Plane::from_polygon(&ps).map_err(|_| BlendError::BlendModified)?;
        if pl.normal().dot(self.virtuals[i].outward) < 0.0 {
            out.reverse();
        }
        let v = &self.virtuals[i];
        let mut attrs = v.attrs;
        if let Some(claim) = attrs.surface {
            let ps: Vec<Point3> = out.iter().map(|&k| self.pos(k)).collect();
            if !claim.holds_on(&ps, &v.plane, tol::PLANE_DIST) {
                attrs.surface = None;
            }
        }
        Ok(PatchFace {
            outer: out,
            holes: Vec::new(),
            attrs,
            plane: Some(v.plane),
        })
    }
}

fn plain(attrs: FaceAttrs) -> FaceAttrs {
    FaceAttrs {
        surface: None,
        blend: None,
        uv_frame: None,
        ..attrs
    }
}

/// The faces a blend consumed (WP08 §2.3), as virtual faces over the
/// neighbours' planes.
fn consumed_faces(
    obj: &Object,
    region: &BTreeSet<FaceId>,
    neighbours: &BTreeSet<FaceId>,
) -> (Vec<Virtual>, Consumed) {
    let claims: Vec<SurfaceRef> = region
        .iter()
        .filter_map(|&f| obj.faces()[f].surface)
        .collect();
    let centroid = |f: FaceId| {
        let ps: Vec<Point3> = obj.loop_positions(obj.faces()[f].outer_loop).collect();
        let c = ps
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
            / ps.len() as f64;
        Point3::new(c.x, c.y, c.z)
    };
    // A hemisphere: a pole every face around which is in the region.
    let ve = obj.vertex_edges();
    for (&v, es) in &ve {
        let around: BTreeSet<FaceId> = es
            .iter()
            .filter_map(|&e| obj.edge_faces(e))
            .flat_map(|(a, b)| [a, b])
            .collect();
        // A hemisphere's pole sits among strip faces (a rolled corner's
        // fan is tagged as a corner).
        if around.is_empty()
            || !around.iter().all(|f| {
                region.contains(f)
                    && obj.faces()[*f]
                        .blend
                        .is_some_and(|t| t.role == crate::topo::BlendRole::Strip)
            })
        {
            continue;
        }
        let Some(SurfaceRef::Sphere { center, .. }) =
            around.iter().find_map(|&f| obj.faces()[f].surface)
        else {
            continue;
        };
        let pole = obj.vertices()[v].position;
        let Ok(n) = (pole - center).normalized() else {
            continue;
        };
        let Ok(plane) = Plane::from_point_normal(pole, n) else {
            continue;
        };
        // The walls in order about the axis.
        let (u, w) = crate::triangulate::plane_basis(n);
        let mut walls: Vec<(f64, FaceId)> = neighbours
            .iter()
            .map(|&f| {
                let c = centroid(f) - center;
                (trig::atan2(c.dot(w), c.dot(u)), f)
            })
            .collect();
        walls.sort_by(|a, b| a.0.total_cmp(&b.0));
        let attrs = plain(obj.faces()[walls[0].1].attrs());
        return (
            vec![Virtual {
                plane,
                ring: walls.into_iter().map(|(_, f)| Key::Face(f)).collect(),
                attrs,
                outward: n,
            }],
            Consumed::Cap,
        );
    }
    // A bullnose: two antiparallel rail neighbours square to the strip
    // axis, 2r apart, the axis midway.
    if let Some(SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    }) = claims.first().copied()
    {
        let flat: Vec<FaceId> = neighbours
            .iter()
            .copied()
            .filter(|&f| obj.faces()[f].surface.is_none())
            .collect();
        for &s1 in &flat {
            for &s2 in &flat {
                let (p1, p2) = (obj.faces()[s1].plane, obj.faces()[s2].plane);
                if s1 >= s2
                    || p1.normal().dot(p2.normal()) > -1.0 + tol::NORMAL_DIRECTION
                    || p1.normal().dot(axis).abs() > tol::NORMAL_DIRECTION
                    || (p1.signed_distance(axis_point) + radius).abs() > tol::BLEND_VERIFY
                    || (p2.signed_distance(axis_point) + radius).abs() > tol::BLEND_VERIFY
                {
                    continue;
                }
                let ends: Vec<FaceId> = neighbours
                    .iter()
                    .copied()
                    .filter(|&f| f != s1 && f != s2)
                    .collect();
                let [e1, e2] = ends.as_slice() else {
                    continue;
                };
                // Out of the material, toward the strip.
                let mut n = axis.cross(p1.normal());
                let strip_c = centroid(*region.iter().next().expect("non-empty"));
                if n.dot(strip_c - axis_point) < 0.0 {
                    n = -n;
                }
                let Ok(plane) = Plane::from_point_normal(axis_point + n * radius, n) else {
                    continue;
                };
                return (
                    vec![Virtual {
                        plane,
                        ring: vec![Key::Face(s1), Key::Face(*e1), Key::Face(s2), Key::Face(*e2)],
                        attrs: plain(obj.faces()[s1].attrs()),
                        outward: n,
                    }],
                    Consumed::Bullnose,
                );
            }
        }
    }
    // A D-post crossing: a rail neighbour that is a cylinder facet along the
    // strip, with seams to extend back toward the other rail.
    if let Some(v) = dpost_facets(obj, region, neighbours, &claims)
        && !v.is_empty()
    {
        return (v, Consumed::Facets);
    }
    (Vec::new(), Consumed::None)
}

/// WP08 §2.3 D-post: re-create the facets a seam-crossing blend consumed by
/// extending the band's equal seam spacing back from the trimmed facet
/// until a seam would cross the flat rail face.
fn dpost_facets(
    obj: &Object,
    region: &BTreeSet<FaceId>,
    neighbours: &BTreeSet<FaceId>,
    claims: &[SurfaceRef],
) -> Option<Vec<Virtual>> {
    let Some(SurfaceRef::Cylinder {
        axis: strip_axis, ..
    }) = claims.first().copied()
    else {
        return None;
    };
    // The trimmed facet: a neighbour on a cylinder along the strip.
    let fm = neighbours.iter().copied().find(|&f| {
        matches!(obj.faces()[f].surface, Some(SurfaceRef::Cylinder { axis, .. })
            if axis.dot(strip_axis).abs() >= 1.0 - tol::NORMAL_DIRECTION)
    })?;
    let Some(SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    }) = obj.faces()[fm].surface
    else {
        return None;
    };
    let caps: Vec<FaceId> = neighbours
        .iter()
        .copied()
        .filter(|&f| obj.faces()[f].plane.normal().dot(axis).abs() >= 1.0 - tol::NORMAL_DIRECTION)
        .collect();
    let [c0, c1] = caps.as_slice() else {
        return None;
    };
    let flat = neighbours
        .iter()
        .copied()
        .find(|&f| f != fm && !caps.contains(&f) && obj.faces()[f].surface.is_none())?;
    let a_plane = obj.faces()[flat].plane;
    let (u, w) = crate::triangulate::plane_basis(axis);
    let angle = |p: Point3| {
        let r = (p - axis_point) - axis * (p - axis_point).dot(axis);
        trig::atan2(r.dot(w), r.dot(u))
    };
    // The trimmed facet's far seam (not against the region), and the next
    // facet across it, whose other seam gives the spacing.
    let mut far: Option<(f64, FaceId)> = None;
    for l in obj.face_loops(fm) {
        for h in obj.loop_half_edges(l) {
            let he = obj.half_edges()[h];
            let a = obj.vertices()[he.origin].position;
            let b = obj.vertices()[obj.half_edges()[he.next].origin].position;
            let Ok(dir) = (b - a).normalized() else {
                continue;
            };
            if dir.dot(axis).abs() < 1.0 - tol::NORMAL_DIRECTION {
                continue;
            }
            let t = he
                .twin
                .map(|t| obj.loops()[obj.half_edges()[t].loop_id].face)?;
            if !region.contains(&t) {
                far = Some((angle(a), t));
            }
        }
    }
    let (phi_far, next) = far?;
    let next_seams: Vec<f64> = obj
        .loop_positions(obj.faces()[next].outer_loop)
        .map(angle)
        .collect();
    let wrap = |x: f64| {
        (x + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    };
    let step = next_seams
        .iter()
        .map(|&a| wrap(a - phi_far))
        .filter(|d| d.abs() > tol::BLEND_SEAM_ANGLE)
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))?;
    // Seams back from the far seam, away from `next`.
    let seam = |phi: f64| axis_point + (u * trig::cos(phi) + w * trig::sin(phi)) * radius;
    let mut seams: Vec<f64> = vec![phi_far, phi_far - step];
    for _ in 0..1024 {
        let last = *seams.last().expect("seams");
        if a_plane.signed_distance(seam(last)) > -tol::POINT_MERGE {
            break;
        }
        seams.push(last - step);
    }
    // seams[0..2] bound the trimmed facet; each further pair is a consumed
    // facet, the last one crossing the flat face.
    let n_virt = seams.len() - 2;
    if n_virt == 0 {
        return Some(Vec::new());
    }
    let chord_plane = |a: f64, b: f64| -> Option<Plane> {
        let (pa, pb) = (seam(a), seam(b));
        let n = (pb - pa).cross(axis).normalized().ok()?;
        let mid = pa + (pb - pa) * 0.5;
        let n = if n.dot(mid - axis_point) < 0.0 { -n } else { n };
        Plane::from_point_normal(pa, n).ok()
    };
    let mut virtuals = Vec::with_capacity(n_virt);
    for j in 0..n_virt {
        let plane = chord_plane(seams[j + 1], seams[j + 2])?;
        let prev = if j == 0 {
            Key::Face(fm)
        } else {
            Key::Virt(j - 1)
        };
        let next_k = if j + 1 == n_virt {
            Key::Face(flat)
        } else {
            Key::Virt(j + 1)
        };
        virtuals.push(Virtual {
            plane,
            ring: vec![prev, Key::Face(*c0), next_k, Key::Face(*c1)],
            attrs: FaceAttrs {
                blend: None,
                ..obj.faces()[fm].attrs()
            },
            outward: plane.normal(),
        });
    }
    Some(virtuals)
}

/// Builds the restoration patch for the region of one blend.
pub(crate) fn restoration(
    obj: &Object,
    region: &BTreeSet<FaceId>,
) -> Result<Restoration, BlendError> {
    let mut neighbours: BTreeSet<FaceId> = BTreeSet::new();
    for &f in region {
        for l in obj.face_loops(f) {
            for h in obj.loop_half_edges(l) {
                if let Some(t) = obj.half_edges()[h].twin {
                    let g = obj.loops()[obj.half_edges()[t].loop_id].face;
                    if !region.contains(&g) {
                        neighbours.insert(g);
                    }
                }
            }
        }
    }
    let (virtuals, consumed) = consumed_faces(obj, region, &neighbours);
    let touching = match consumed {
        Consumed::Cap => Some(Key::Virt(0)),
        Consumed::Facets => virtuals.last().and_then(|v| match v.ring[2] {
            k @ Key::Face(_) => Some(k),
            Key::Virt(_) => None,
        }),
        _ => None,
    };
    let sides = match (consumed, virtuals.first()) {
        (Consumed::Bullnose, Some(v)) => match (v.ring[0], v.ring[2]) {
            (Key::Face(a), Key::Face(b)) => Some((a, b)),
            _ => None,
        },
        _ => None,
    };
    let mut r = Restorer {
        obj,
        region,
        neighbours: neighbours.clone(),
        virtuals,
        verts: Vec::new(),
        existing: BTreeMap::new(),
        triples: BTreeMap::new(),
        restored: BTreeSet::new(),
        continued: BTreeSet::new(),
    };
    let mut add: Vec<PatchFace> = Vec::new();
    for &f in &neighbours {
        add.push(r.restore_neighbour(f)?);
    }
    for i in 0..r.virtuals.len() {
        add.push(r.restore_virtual(i)?);
    }
    let mut remove: Vec<FaceId> = region.iter().copied().collect();
    remove.extend(neighbours.iter().copied());
    // An edge the blend shortened comes back at full length from its kept
    // end to the restored corner: it keeps the marks it carried (a dome
    // base's soft spokes, a crease), as the blend kept them (D-69).
    let mut edge_meta = BTreeMap::new();
    for face in &add {
        for ring in std::iter::once(&face.outer).chain(&face.holes) {
            for k in 0..ring.len() {
                let (i, j) = (ring[k], ring[(k + 1) % ring.len()]);
                let (v, p) = match (&r.verts[i], &r.verts[j]) {
                    (PVert::Existing(v), PVert::New(p)) | (PVert::New(p), PVert::Existing(v)) => {
                        (*v, *p)
                    }
                    _ => continue,
                };
                if let Some(meta) = lengthened_meta(obj, v, p) {
                    edge_meta.insert((i.min(j), i.max(j)), meta);
                }
            }
        }
    }
    Ok(Restoration {
        patch: PolyPatch {
            verts: r.verts,
            remove,
            add,
            edge_meta,
            expect_watertight: true,
        },
        neighbours: neighbours.into_iter().collect(),
        restored_pts: r.restored,
        consumed,
        sides,
        touching,
        continued: r.continued,
    })
}

/// The marks of the edge at `v` that runs toward `p` and stops short of it
/// (an edge the blend shortened), for the edge from `v` to `p` that
/// replaces it. A circle claim carries only while `p` stays on its circle.
fn lengthened_meta(obj: &Object, v: VertexId, p: Point3) -> Option<EdgeMeta> {
    let hes = obj.half_edges();
    let start = obj.vertices()[v].outgoing;
    let at = obj.vertices()[v].position;
    let mut h = start;
    for _ in 0..hes.len() {
        let w = obj.vertices()[hes[hes[h].next].origin].position;
        let (along, toward) = (w - at, p - at);
        let len = along.length();
        if len > tol::POINT_MERGE && toward.length() + tol::BLEND_VERIFY >= len {
            let t = toward.dot(along) / len;
            let off = (toward - along * (t / len)).length();
            if t > 0.0 && off <= tol::BLEND_VERIFY {
                let e = &obj.edges()[hes[h].edge];
                let on_circle = |g: &crate::sketch::CurveGeom| {
                    ((p - g.center).length() - g.radius).abs()
                        <= obj.planarity_tol.max(tol::PLANE_DIST)
                };
                let meta = EdgeMeta {
                    soft: e.soft,
                    crease: e.crease,
                    curve: e.curve.filter(on_circle),
                    polygon: e.polygon,
                };
                return (meta != EdgeMeta::default()).then_some(meta);
            }
        }
        h = hes[hes[h].prev].twin?;
        if h == start {
            break;
        }
    }
    None
}
