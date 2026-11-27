//! The one geometry-surgery primitive the true-curves operations share
//! (decision I-7): [`Object::apply_patch`] removes faces, adds polygon faces,
//! twins them by vertex pairs, and carries edge metadata — on a clone,
//! validated before it is returned. Blends, removal, Deform, Bake, Shell and
//! Project all build a [`PolyPatch`] and apply it, so the half-edge surgery
//! lives (and is validated) in one place.
//!
//! Also here: [`planarize`] (regrouping a moved triangle soup into planar
//! polygon faces), [`resplit_shells`] (shells from twin connectivity), and
//! the self-intersection / obstruction guard ([`improper_contact`],
//! [`self_intersection`]).

use std::collections::{BTreeMap, BTreeSet};

use slotmap::SecondaryMap;

use crate::error::TopologyError;
use crate::ids::{EdgeId, FaceId, HalfEdgeId, LoopId, VertexId};
use crate::math::{Plane, Point3};
use crate::sketch::CurveGeom;
use crate::tol;
use crate::topo::{
    Edge, Face, FaceAttrs, HalfEdge, Loop, LoopKind, Object, Shell, Vertex, WatertightState,
};

/// A vertex of a patch: an existing vertex (kept, possibly shared with faces
/// the patch does not touch) or a new position.
#[derive(Debug, Clone, Copy)]
pub enum PVert {
    /// A vertex of the object being patched; it keeps its id and position.
    Existing(VertexId),
    /// A vertex the patch creates at this position.
    New(Point3),
}

/// Edge metadata as a value (the fields of [`Edge`] beyond its topology).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeMeta {
    pub soft: bool,
    pub crease: bool,
    pub curve: Option<CurveGeom>,
    pub polygon: Option<CurveGeom>,
}

impl EdgeMeta {
    fn of(e: &Edge) -> EdgeMeta {
        EdgeMeta {
            soft: e.soft,
            crease: e.crease,
            curve: e.curve,
            polygon: e.polygon,
        }
    }

    fn write(self, e: &mut Edge) {
        e.soft = self.soft;
        e.crease = self.crease;
        e.curve = self.curve;
        e.polygon = self.polygon;
    }
}

/// One face a patch adds.
#[derive(Debug, Clone)]
pub struct PatchFace {
    /// Indices into [`PolyPatch::verts`], outer loop CCW seen from the
    /// outward normal.
    pub outer: Vec<usize>,
    /// Hole loops (opposite winding).
    pub holes: Vec<Vec<usize>>,
    /// Material, back material, UV frame, surface claim, blend tag.
    pub attrs: FaceAttrs,
    /// The plane, if the caller computed it exactly; `None` → Newell refit.
    pub plane: Option<Plane>,
}

/// A set of faces to remove and polygon faces to add.
#[derive(Debug, Clone, Default)]
pub struct PolyPatch {
    pub verts: Vec<PVert>,
    /// Faces deleted. A face being *modified* is listed here and re-added.
    pub remove: Vec<FaceId>,
    pub add: Vec<PatchFace>,
    /// Explicit metadata for edges between two patch vertices (unordered
    /// pair, smaller index first). Overrides inheritance.
    pub edge_meta: BTreeMap<(usize, usize), EdgeMeta>,
    /// Refuse unless the result is watertight.
    pub expect_watertight: bool,
}

/// What [`Object::apply_patch`] produced.
#[derive(Debug, Clone)]
pub struct PatchResult {
    pub object: Object,
    /// `add[i]` became `faces[i]`.
    pub faces: Vec<FaceId>,
    /// `verts[i]` became `vertices[i]`.
    pub vertices: Vec<VertexId>,
}

fn pair(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a < b { (a, b) } else { (b, a) }
}

impl Object {
    /// Applies `patch` to a clone of `self` and returns the validated result;
    /// `self` is never modified (strong guarantee). Faces not in
    /// `patch.remove` keep their `FaceId`s and attributes. An edge between two
    /// vertices that survive keeps its metadata unless `patch.edge_meta`
    /// overrides it; a new edge between two existing vertices that an edge
    /// of a removed face also joined inherits that edge's metadata.
    ///
    /// Steps, in order: input check; clone; snapshot the metadata of edges on
    /// removed faces; remove faces (their half-edges open the neighbours'
    /// twins); create the new vertices, faces and loops; twin every open
    /// half-edge with its reverse (exactly one candidate, else
    /// `NonManifoldEdge`; none leaves the edge open); edge metadata; vertex
    /// `outgoing` and orphan vertices; planes (Newell where not given) with
    /// every loop vertex within the object's planarity tolerance; shells from
    /// twin connectivity; the watertight flag; stale circle claims dropped;
    /// `validate()`.
    ///
    /// # Errors
    /// - `InvalidVertexIndex { face, index }`: an index out of range, or an
    ///   `Existing` vertex that is not live (`face` is the `add` index).
    /// - `DegenerateFace { face }`: a loop under 3 entries, two consecutive
    ///   entries resolving to one vertex or coinciding within `POINT_MERGE`,
    ///   or a plane that cannot be fitted.
    /// - `DanglingHandle`: a `remove` id that is not live, or listed twice.
    /// - `NonManifoldEdge { from, to }` (patch vertex indices): a directed
    ///   edge traversed twice, or more than one reverse candidate.
    /// - `NonPlanarFace { face }`: an added face with a vertex off its plane.
    /// - `WatertightFlagMismatch`: `expect_watertight` and the result is open.
    /// - anything `validate()` reports.
    pub(crate) fn apply_patch(&self, patch: &PolyPatch) -> Result<PatchResult, TopologyError> {
        // 1. Check the input.
        for (fi, f) in patch.add.iter().enumerate() {
            for lp in std::iter::once(&f.outer).chain(f.holes.iter()) {
                if lp.len() < 3 {
                    return Err(TopologyError::DegenerateFace { face: fi });
                }
                for &i in lp {
                    let ok = match patch.verts.get(i) {
                        None => false,
                        Some(PVert::Existing(v)) => self.vertices.contains_key(*v),
                        Some(PVert::New(p)) => {
                            p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
                        }
                    };
                    if !ok {
                        return Err(TopologyError::InvalidVertexIndex { face: fi, index: i });
                    }
                }
                let n = lp.len();
                for k in 0..n {
                    let (i, j) = (lp[k], lp[(k + 1) % n]);
                    let same = i == j
                        || match (patch.verts[i], patch.verts[j]) {
                            (PVert::Existing(a), PVert::Existing(b)) => a == b,
                            _ => false,
                        };
                    let pos = |v: PVert| match v {
                        PVert::Existing(id) => self.vertices[id].position,
                        PVert::New(p) => p,
                    };
                    if same || pos(patch.verts[i]).approx_eq(pos(patch.verts[j]), tol::POINT_MERGE)
                    {
                        return Err(TopologyError::DegenerateFace { face: fi });
                    }
                }
            }
        }
        let removing: BTreeSet<FaceId> = patch.remove.iter().copied().collect();
        for &f in &patch.remove {
            if !self.faces.contains_key(f) {
                return Err(TopologyError::DanglingHandle {
                    context: "patch removes a face that is not live",
                });
            }
        }
        if removing.len() != patch.remove.len() {
            return Err(TopologyError::DanglingHandle {
                context: "patch removes a face twice",
            });
        }

        // 2. Clone.
        let mut o = self.clone();

        // 3. Snapshot the metadata of every edge on a removed face.
        let mut inherited: BTreeMap<(VertexId, VertexId), EdgeMeta> = BTreeMap::new();
        for &f in &patch.remove {
            for lid in o.face_loops(f) {
                for h in o.loop_half_edges(lid).collect::<Vec<_>>() {
                    let he = o.half_edges[h];
                    let to = o.half_edges[he.next].origin;
                    inherited.insert(pair(he.origin, to), EdgeMeta::of(&o.edges[he.edge]));
                }
            }
        }

        // 4. Remove faces.
        for &f in &patch.remove {
            let loops = o.face_loops(f);
            for lid in &loops {
                let hs: Vec<HalfEdgeId> = o.loop_half_edges(*lid).collect();
                for h in hs {
                    let he = o.half_edges[h];
                    let live_twin = he.twin.filter(|&t| {
                        o.half_edges
                            .get(t)
                            .is_some_and(|th| !removing.contains(&o.loops[th.loop_id].face))
                    });
                    match live_twin {
                        Some(t) => {
                            o.half_edges[t].twin = None;
                            if let Some(e) = o.edges.get_mut(he.edge) {
                                e.half_edge = t;
                                e.twin_half_edge = None;
                            }
                        }
                        None => {
                            o.edges.remove(he.edge);
                        }
                    }
                }
                for h in o.loop_half_edges(*lid).collect::<Vec<_>>() {
                    o.half_edges.remove(h);
                }
            }
            for lid in loops {
                o.loops.remove(lid);
            }
            o.faces.remove(f);
        }
        // The shells are not edited here: step 11 rebuilds them all from the
        // result's twins (editing them per removed face cost the removed
        // count times the object's faces, for nothing).

        // 5. Create vertices.
        let mut vertex_of: Vec<VertexId> = Vec::with_capacity(patch.verts.len());
        let mut touched: BTreeSet<VertexId> = BTreeSet::new();
        for v in &patch.verts {
            let id = match *v {
                PVert::Existing(id) => id,
                PVert::New(p) => o.vertices.insert(Vertex {
                    position: p,
                    outgoing: HalfEdgeId::default(),
                }),
            };
            touched.insert(id);
            vertex_of.push(id);
        }
        // First patch index of each patch vertex, for `edge_meta` lookups.
        let mut index_of: BTreeMap<VertexId, usize> = BTreeMap::new();
        for (i, &v) in vertex_of.iter().enumerate() {
            index_of.entry(v).or_insert(i);
        }

        // 6. Create faces, loops and half-edges.
        let mut new_faces: Vec<FaceId> = Vec::with_capacity(patch.add.len());
        let mut new_half_edges: Vec<HalfEdgeId> = Vec::new();
        for pf in &patch.add {
            let face = o.faces.insert(Face {
                outer_loop: LoopId::default(),
                inner_loops: Vec::new(),
                plane: pf.plane.unwrap_or_else(|| {
                    Plane::from_point_normal(
                        Point3::new(0.0, 0.0, 0.0),
                        crate::Vec3::new(0.0, 0.0, 1.0),
                    )
                    .expect("unit +Z is a valid normal")
                }),
                material: pf.attrs.material,
                back_material: pf.attrs.back_material,
                uv_frame: pf.attrs.uv_frame,
                surface: pf.attrs.surface,
                blend: pf.attrs.blend,
                attrs: pf.attrs.attrs,
            });
            for (li, lp) in std::iter::once(&pf.outer)
                .chain(pf.holes.iter())
                .enumerate()
            {
                let kind = if li == 0 {
                    LoopKind::Outer
                } else {
                    LoopKind::Inner
                };
                let lid = o.loops.insert(Loop {
                    face,
                    first_half_edge: HalfEdgeId::default(),
                    kind,
                });
                let hs: Vec<HalfEdgeId> = lp
                    .iter()
                    .map(|&i| {
                        o.half_edges.insert(HalfEdge {
                            origin: vertex_of[i],
                            twin: None,
                            next: HalfEdgeId::default(),
                            prev: HalfEdgeId::default(),
                            edge: EdgeId::default(),
                            loop_id: lid,
                        })
                    })
                    .collect();
                let n = hs.len();
                for k in 0..n {
                    o.half_edges[hs[k]].next = hs[(k + 1) % n];
                    o.half_edges[hs[k]].prev = hs[(k + n - 1) % n];
                }
                o.loops[lid].first_half_edge = hs[0];
                if li == 0 {
                    o.faces[face].outer_loop = lid;
                } else {
                    o.faces[face].inner_loops.push(lid);
                }
                new_half_edges.extend(hs);
            }
            new_faces.push(face);
        }
        let new_set: BTreeSet<HalfEdgeId> = new_half_edges.iter().copied().collect();

        // 7. Twin. Every directed edge may be traversed once; each new
        // half-edge pairs with the one open half-edge running the other way.
        let dest = |o: &Object, h: HalfEdgeId| o.half_edges[o.half_edges[h].next].origin;
        let mut directed: BTreeMap<(VertexId, VertexId), Vec<HalfEdgeId>> = BTreeMap::new();
        for (h, he) in o.half_edges.iter() {
            directed
                .entry((he.origin, dest(&o, h)))
                .or_default()
                .push(h);
        }
        let patch_index = |v: VertexId| index_of.get(&v).copied().unwrap_or(usize::MAX);
        for (&(a, b), hs) in &directed {
            if hs.len() > 1 && hs.iter().any(|h| new_set.contains(h)) {
                return Err(TopologyError::NonManifoldEdge {
                    from: patch_index(a),
                    to: patch_index(b),
                });
            }
        }
        let mut created_edges: Vec<EdgeId> = Vec::new();
        for &h in &new_half_edges {
            if o.half_edges[h].twin.is_some() {
                continue; // twinned as some earlier half-edge's partner
            }
            let (a, b) = (o.half_edges[h].origin, dest(&o, h));
            let candidates: Vec<HalfEdgeId> = directed
                .get(&(b, a))
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|&c| o.half_edges[c].twin.is_none())
                        .collect()
                })
                .unwrap_or_default();
            match candidates.as_slice() {
                [] => {
                    let e = o.edges.insert(Edge {
                        half_edge: h,
                        twin_half_edge: None,
                        curve: None,
                        soft: false,
                        crease: false,
                        polygon: None,
                        attrs: crate::topo::AttrSlot::NONE,
                    });
                    o.half_edges[h].edge = e;
                    created_edges.push(e);
                }
                [c] => {
                    let c = *c;
                    o.half_edges[h].twin = Some(c);
                    o.half_edges[c].twin = Some(h);
                    if new_set.contains(&c) {
                        let e = o.edges.insert(Edge {
                            half_edge: h,
                            twin_half_edge: Some(c),
                            curve: None,
                            soft: false,
                            crease: false,
                            polygon: None,
                            attrs: crate::topo::AttrSlot::NONE,
                        });
                        o.half_edges[h].edge = e;
                        o.half_edges[c].edge = e;
                        created_edges.push(e);
                    } else {
                        // Reuse the opened existing edge: it keeps its
                        // metadata (unless `edge_meta` overrides it below).
                        let e = o.half_edges[c].edge;
                        o.edges[e].half_edge = c;
                        o.edges[e].twin_half_edge = Some(h);
                        o.half_edges[h].edge = e;
                        if let (Some(&ia), Some(&ib)) = (index_of.get(&a), index_of.get(&b))
                            && let Some(m) = patch.edge_meta.get(&(ia.min(ib), ia.max(ib)))
                        {
                            m.write(&mut o.edges[e]);
                        }
                    }
                }
                _ => {
                    return Err(TopologyError::NonManifoldEdge {
                        from: patch_index(a),
                        to: patch_index(b),
                    });
                }
            }
        }

        // 8. Edge metadata of the created edges.
        for &e in &created_edges {
            let h = o.edges[e].half_edge;
            let (a, b) = (o.half_edges[h].origin, dest(&o, h));
            let explicit = match (index_of.get(&a), index_of.get(&b)) {
                (Some(&ia), Some(&ib)) => patch.edge_meta.get(&(ia.min(ib), ia.max(ib))).copied(),
                _ => None,
            };
            let both_existing = |v: VertexId| {
                index_of
                    .get(&v)
                    .is_some_and(|&i| matches!(patch.verts[i], PVert::Existing(_)))
            };
            let meta = explicit
                .or_else(|| {
                    (both_existing(a) && both_existing(b))
                        .then(|| inherited.get(&pair(a, b)).copied())
                        .flatten()
                })
                .unwrap_or_default();
            meta.write(&mut o.edges[e]);
        }
        for e in o.edges.values_mut() {
            if e.twin_half_edge.is_none() {
                e.soft = false;
                e.crease = false;
            }
        }

        // 9. Vertices: `outgoing` for every created or touched vertex (and
        // any whose half-edge died), then drop vertices nothing references.
        let mut first_out: SecondaryMap<VertexId, HalfEdgeId> = SecondaryMap::new();
        for (h, he) in o.half_edges.iter() {
            if !first_out.contains_key(he.origin) {
                first_out.insert(he.origin, h);
            }
        }
        let vids: Vec<VertexId> = o.vertices.keys().collect();
        for v in vids {
            match first_out.get(v) {
                None => {
                    o.vertices.remove(v);
                }
                Some(&h) => {
                    let current = o.vertices[v].outgoing;
                    let stale = o.half_edges.get(current).is_none_or(|he| he.origin != v);
                    if touched.contains(&v) || stale {
                        o.vertices[v].outgoing = h;
                    }
                }
            }
        }

        // 10. Planes.
        for (fi, (&face, pf)) in new_faces.iter().zip(&patch.add).enumerate() {
            let plane = match pf.plane {
                Some(p) => p,
                None => {
                    let ring: Vec<Point3> = o.loop_positions(o.faces[face].outer_loop).collect();
                    Plane::from_polygon(&ring)
                        .map_err(|_| TopologyError::DegenerateFace { face: fi })?
                }
            };
            o.faces[face].plane = plane;
            for lid in o.face_loops(face) {
                if o.loop_positions(lid)
                    .any(|p| plane.signed_distance(p).abs() > o.planarity_tol)
                {
                    return Err(TopologyError::NonPlanarFace { face: fi });
                }
            }
        }

        // 11. Shells.
        resplit_shells(&mut o);

        // 12. Watertightness.
        o.watertight = if o.edges.values().all(|e| e.twin_half_edge.is_some()) {
            WatertightState::Watertight
        } else {
            WatertightState::Open
        };
        if patch.expect_watertight && o.watertight != WatertightState::Watertight {
            return Err(TopologyError::WatertightFlagMismatch {
                expected: WatertightState::Watertight,
            });
        }

        // 13. Claims that no longer describe their edge drop.
        o.drop_stale_edge_curves();

        // 14. Validate. A patch that builds an invalid solid is refused with
        // the validator's own typed error, in every build: this is the
        // contract's "anything `validate()` reports", not an assertion about
        // the caller's patch.
        o.validate()?;
        Ok(PatchResult {
            object: o,
            faces: new_faces,
            vertices: vertex_of,
        })
    }

    /// [`Object::apply_patch`] for a closed object whose edit is small
    /// against its size (a boolean that touches a corner of a dense scan):
    /// removes `remove`, adds `add` (every plane given), twins the new
    /// half-edges with each other and with the half-edges the removal
    /// opened, and returns the result with the new faces. Untouched faces,
    /// edges and vertices keep their ids, attributes and metadata, and the
    /// work outside the edit is linear scans of plain tables, never a
    /// rebuild: no directed-edge map of the whole object, and the validator
    /// runs on the edited faces and their seams ([`Object::validate_local`]).
    /// New edges carry no metadata; an opened edge keeps its own.
    ///
    /// The result must be closed again: a new half-edge without a partner,
    /// or an opened one left without one, refuses `WatertightFlagMismatch`.
    /// The planarity gate is the wider of the object's and `plane_tol`.
    ///
    /// # Errors
    /// As [`Object::apply_patch`] (`NonManifoldEdge` names patch vertex
    /// indices, `NonPlanarFace` and `DegenerateFace` the `add` index), and
    /// `WatertightFlagMismatch` as above.
    pub(crate) fn splice_closed(
        &self,
        remove: &[FaceId],
        verts: &[PVert],
        add: &[PatchFace],
        plane_tol: f64,
    ) -> Result<(Object, Vec<FaceId>), TopologyError> {
        let open_err = || TopologyError::WatertightFlagMismatch {
            expected: WatertightState::Watertight,
        };
        // 1. Check the input (as `apply_patch`).
        let pos = |v: PVert| match v {
            PVert::Existing(id) => self.vertices[id].position,
            PVert::New(p) => p,
        };
        for (fi, f) in add.iter().enumerate() {
            if f.plane.is_none() {
                return Err(TopologyError::DegenerateFace { face: fi });
            }
            for lp in std::iter::once(&f.outer).chain(f.holes.iter()) {
                if lp.len() < 3 {
                    return Err(TopologyError::DegenerateFace { face: fi });
                }
                for &i in lp {
                    let ok = match verts.get(i) {
                        None => false,
                        Some(PVert::Existing(v)) => self.vertices.contains_key(*v),
                        Some(PVert::New(p)) => {
                            p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
                        }
                    };
                    if !ok {
                        return Err(TopologyError::InvalidVertexIndex { face: fi, index: i });
                    }
                }
                let n = lp.len();
                for k in 0..n {
                    let (i, j) = (lp[k], lp[(k + 1) % n]);
                    let same = i == j
                        || matches!(
                            (verts[i], verts[j]),
                            (PVert::Existing(a), PVert::Existing(b)) if a == b
                        );
                    if same || pos(verts[i]).approx_eq(pos(verts[j]), tol::POINT_MERGE) {
                        return Err(TopologyError::DegenerateFace { face: fi });
                    }
                }
            }
        }
        if self.watertight != WatertightState::Watertight {
            return Err(open_err());
        }
        let mut removing: SecondaryMap<FaceId, ()> = SecondaryMap::new();
        for &f in remove {
            if !self.faces.contains_key(f) {
                return Err(TopologyError::DanglingHandle {
                    context: "patch removes a face that is not live",
                });
            }
            removing.insert(f, ());
        }
        if removing.len() != remove.len() {
            return Err(TopologyError::DanglingHandle {
                context: "patch removes a face twice",
            });
        }

        // 2. Clone (tables are shared until written).
        let mut o = self.clone();
        o.planarity_tol = o.planarity_tol.max(plane_tol);

        // 3. Remove faces; their live neighbours' half-edges open.
        let mut opened: Vec<HalfEdgeId> = Vec::new();
        let mut freed: Vec<VertexId> = Vec::new();
        for &f in remove {
            let loops = o.face_loops(f);
            for lid in &loops {
                let hs: Vec<HalfEdgeId> = o.loop_half_edges(*lid).collect();
                for &h in &hs {
                    let he = o.half_edges[h];
                    freed.push(he.origin);
                    let live_twin = he.twin.filter(|&t| {
                        o.half_edges
                            .get(t)
                            .is_some_and(|th| !removing.contains_key(o.loops[th.loop_id].face))
                    });
                    match live_twin {
                        Some(t) => {
                            o.half_edges[t].twin = None;
                            if let Some(e) = o.edges.get_mut(he.edge) {
                                e.half_edge = t;
                                e.twin_half_edge = None;
                            }
                            opened.push(t);
                        }
                        None => {
                            o.edges.remove(he.edge);
                        }
                    }
                }
                for h in hs {
                    o.half_edges.remove(h);
                }
            }
            for lid in loops {
                o.loops.remove(lid);
            }
            o.faces.remove(f);
        }
        // The shells are not edited here: step 9 rebuilds them all from the
        // result's twins.

        // 4. Vertices.
        let mut vertex_of: Vec<VertexId> = Vec::with_capacity(verts.len());
        for v in verts {
            vertex_of.push(match *v {
                PVert::Existing(id) => id,
                PVert::New(p) => o.vertices.insert(Vertex {
                    position: p,
                    outgoing: HalfEdgeId::default(),
                }),
            });
        }
        let mut index_of: BTreeMap<VertexId, usize> = BTreeMap::new();
        for (i, &v) in vertex_of.iter().enumerate() {
            index_of.entry(v).or_insert(i);
        }
        let patch_index = |v: VertexId| index_of.get(&v).copied().unwrap_or(usize::MAX);

        // 5. Faces, loops, half-edges.
        let mut new_faces: Vec<FaceId> = Vec::with_capacity(add.len());
        let mut new_half_edges: Vec<HalfEdgeId> = Vec::new();
        for pf in add {
            let plane = pf.plane.expect("checked above");
            let face = o.faces.insert(Face {
                outer_loop: LoopId::default(),
                inner_loops: Vec::new(),
                plane,
                material: pf.attrs.material,
                back_material: pf.attrs.back_material,
                uv_frame: pf.attrs.uv_frame,
                surface: pf.attrs.surface,
                blend: pf.attrs.blend,
                attrs: pf.attrs.attrs,
            });
            for (li, lp) in std::iter::once(&pf.outer)
                .chain(pf.holes.iter())
                .enumerate()
            {
                let kind = if li == 0 {
                    LoopKind::Outer
                } else {
                    LoopKind::Inner
                };
                let lid = o.loops.insert(Loop {
                    face,
                    first_half_edge: HalfEdgeId::default(),
                    kind,
                });
                let hs: Vec<HalfEdgeId> = lp
                    .iter()
                    .map(|&i| {
                        o.half_edges.insert(HalfEdge {
                            origin: vertex_of[i],
                            twin: None,
                            next: HalfEdgeId::default(),
                            prev: HalfEdgeId::default(),
                            edge: EdgeId::default(),
                            loop_id: lid,
                        })
                    })
                    .collect();
                let n = hs.len();
                for k in 0..n {
                    o.half_edges[hs[k]].next = hs[(k + 1) % n];
                    o.half_edges[hs[k]].prev = hs[(k + n - 1) % n];
                }
                o.loops[lid].first_half_edge = hs[0];
                if li == 0 {
                    o.faces[face].outer_loop = lid;
                } else {
                    o.faces[face].inner_loops.push(lid);
                }
                new_half_edges.extend(hs);
            }
            new_faces.push(face);
        }
        let mut is_new: SecondaryMap<HalfEdgeId, ()> = SecondaryMap::new();
        for &h in &new_half_edges {
            is_new.insert(h, ());
        }

        // 6. Twin, among the open half-edges only: the opened ones and the
        // new ones. Every directed edge may be traversed once.
        let dest = |o: &Object, h: HalfEdgeId| o.half_edges[o.half_edges[h].next].origin;
        let mut directed: BTreeMap<(VertexId, VertexId), Vec<HalfEdgeId>> = BTreeMap::new();
        for &h in opened.iter().chain(&new_half_edges) {
            directed
                .entry((o.half_edges[h].origin, dest(&o, h)))
                .or_default()
                .push(h);
        }
        for (&(a, b), hs) in &directed {
            if hs.len() > 1 && hs.iter().any(|h| is_new.contains_key(*h)) {
                return Err(TopologyError::NonManifoldEdge {
                    from: patch_index(a),
                    to: patch_index(b),
                });
            }
        }
        // A new half-edge must not repeat a closed one the edit left alone,
        // and every vertex the edit touched needs an outgoing half-edge:
        // one pass over the half-edges, reading only those that start at a
        // touched vertex.
        let mut touched: SecondaryMap<VertexId, bool> = SecondaryMap::new();
        for &v in &freed {
            touched.insert(v, false);
        }
        for &v in &vertex_of {
            touched.insert(v, true);
        }
        let mut first_out: SecondaryMap<VertexId, HalfEdgeId> = SecondaryMap::new();
        for (h, he) in o.half_edges.iter() {
            if !touched.contains_key(he.origin) {
                continue;
            }
            if !first_out.contains_key(he.origin) {
                first_out.insert(he.origin, h);
            }
            if he.twin.is_some() && !is_new.contains_key(h) {
                let key = (he.origin, o.half_edges[he.next].origin);
                if let Some(hs) = directed.get(&key)
                    && hs.iter().any(|x| is_new.contains_key(*x))
                {
                    return Err(TopologyError::NonManifoldEdge {
                        from: patch_index(key.0),
                        to: patch_index(key.1),
                    });
                }
            }
        }
        for &h in &new_half_edges {
            if o.half_edges[h].twin.is_some() {
                continue; // twinned as some earlier half-edge's partner
            }
            let (a, b) = (o.half_edges[h].origin, dest(&o, h));
            let candidates: Vec<HalfEdgeId> = directed
                .get(&(b, a))
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|&c| o.half_edges[c].twin.is_none())
                        .collect()
                })
                .unwrap_or_default();
            let c = match candidates.as_slice() {
                [] => return Err(open_err()),
                [c] => *c,
                _ => {
                    return Err(TopologyError::NonManifoldEdge {
                        from: patch_index(a),
                        to: patch_index(b),
                    });
                }
            };
            o.half_edges[h].twin = Some(c);
            o.half_edges[c].twin = Some(h);
            if is_new.contains_key(c) {
                let e = o.edges.insert(Edge {
                    half_edge: h,
                    twin_half_edge: Some(c),
                    curve: None,
                    soft: false,
                    crease: false,
                    polygon: None,
                    attrs: crate::topo::AttrSlot::NONE,
                });
                o.half_edges[h].edge = e;
                o.half_edges[c].edge = e;
            } else {
                // Reuse the opened edge: it keeps its metadata.
                let e = o.half_edges[c].edge;
                o.edges[e].half_edge = c;
                o.edges[e].twin_half_edge = Some(h);
                o.half_edges[h].edge = e;
            }
        }
        if opened.iter().any(|&h| o.half_edges[h].twin.is_none()) {
            return Err(open_err());
        }

        // 7. Vertices: `outgoing` for every vertex the patch names (and any
        // whose half-edge died); a removed face's vertex nothing uses goes.
        for (v, named) in touched.iter() {
            match first_out.get(v) {
                None => {
                    o.vertices.remove(v);
                }
                Some(&h) => {
                    let current = o.vertices[v].outgoing;
                    let stale = o.half_edges.get(current).is_none_or(|he| he.origin != v);
                    if *named || stale {
                        o.vertices[v].outgoing = h;
                    }
                }
            }
        }

        // 8. Planes hold every loop vertex within the planarity gate.
        for (fi, &face) in new_faces.iter().enumerate() {
            let plane = o.faces[face].plane;
            for lid in o.face_loops(face) {
                if o.loop_positions(lid)
                    .any(|p| plane.signed_distance(p).abs() > o.planarity_tol)
                {
                    return Err(TopologyError::NonPlanarFace { face: fi });
                }
            }
        }

        // 9. Shells from twin connectivity; the object is closed.
        resplit_shells(&mut o);
        o.watertight = WatertightState::Watertight;

        // 10. Validate what changed.
        o.validate_local(&new_faces)?;
        Ok((o, new_faces))
    }

    /// The object as a patch that removes every face and re-adds it unchanged,
    /// with `PVert::Existing` for every vertex (slot order) and edge metadata
    /// filled for every edge. Callers edit positions (turning `Existing` into
    /// `New`) and faces, then apply. `apply_patch(&o.to_patch())` reproduces
    /// an object equal to `o` up to face and edge ids (geometry, attributes
    /// and metadata identical).
    pub(crate) fn to_patch(&self) -> PolyPatch {
        let index: BTreeMap<VertexId, usize> = self
            .vertices
            .keys()
            .enumerate()
            .map(|(i, v)| (v, i))
            .collect();
        let verts: Vec<PVert> = self.vertices.keys().map(PVert::Existing).collect();
        let mut add = Vec::with_capacity(self.faces.len());
        for (_, f) in self.faces.iter() {
            let ring = |lid: LoopId| -> Vec<usize> {
                self.loop_half_edges(lid)
                    .map(|h| index[&self.half_edges[h].origin])
                    .collect()
            };
            add.push(PatchFace {
                outer: ring(f.outer_loop),
                holes: f.inner_loops.iter().map(|&l| ring(l)).collect(),
                attrs: f.attrs(),
                plane: Some(f.plane),
            });
        }
        let mut edge_meta = BTreeMap::new();
        for (eid, e) in self.edges.iter() {
            if let Some((a, b)) = self.edge_vertex_ids(eid) {
                let (ia, ib) = (index[&a], index[&b]);
                edge_meta.insert((ia.min(ib), ia.max(ib)), EdgeMeta::of(e));
            }
        }
        PolyPatch {
            verts,
            remove: self.faces.keys().collect(),
            add,
            edge_meta,
            expect_watertight: self.watertight == WatertightState::Watertight,
        }
    }

    /// Every loop of `face`, outer first.
    pub(crate) fn face_loops(&self, face: FaceId) -> Vec<LoopId> {
        let f = &self.faces[face];
        std::iter::once(f.outer_loop)
            .chain(f.inner_loops.iter().copied())
            .collect()
    }

    /// An edge's two endpoint vertex ids (origin of its half-edge first).
    pub(crate) fn edge_vertex_ids(&self, edge: EdgeId) -> Option<(VertexId, VertexId)> {
        let e = self.edges.get(edge)?;
        let he = self.half_edges.get(e.half_edge)?;
        Some((he.origin, self.half_edges[he.next].origin))
    }
}

/// Replaces the object's shells with one per connected component of the
/// face-adjacency graph (faces linked by a shared, twinned edge). The builders
/// emit a single shell; a boolean or a patch result can be disconnected (a
/// subtract that cuts a solid in two, a disjoint union, a shell's cavity).
pub(crate) fn resplit_shells(obj: &mut Object) {
    let mut adjacency: SecondaryMap<FaceId, Vec<FaceId>> = SecondaryMap::new();
    for f in obj.faces.keys() {
        adjacency.insert(f, Vec::new());
    }
    for edge in obj.edges.values() {
        if let Some(twin) = edge.twin_half_edge {
            let fa = obj.loops[obj.half_edges[edge.half_edge].loop_id].face;
            let fb = obj.loops[obj.half_edges[twin].loop_id].face;
            adjacency[fa].push(fb);
            adjacency[fb].push(fa);
        }
    }

    let mut component: SecondaryMap<FaceId, ()> = SecondaryMap::new();
    let mut shells: Vec<Vec<FaceId>> = Vec::new();
    for start in obj.faces.keys() {
        if component.contains_key(start) {
            continue;
        }
        let mut group = Vec::new();
        let mut stack = vec![start];
        component.insert(start, ());
        while let Some(f) = stack.pop() {
            group.push(f);
            for &nb in &adjacency[f] {
                if component.insert(nb, ()).is_none() {
                    stack.push(nb);
                }
            }
        }
        shells.push(group);
    }

    obj.shells.clear();
    for faces in shells {
        obj.shells.insert(Shell { faces });
    }
}

// ═══════════════════════════════════════════════════════ planarize

/// A triangle soup with provenance: the input of [`planarize`].
#[derive(Debug, Clone, Default)]
pub struct TriSoup {
    pub points: Vec<Point3>,
    /// Each triangle: three point indices (CCW outward), its source face
    /// index, and the source face's attrs.
    pub tris: Vec<([u32; 3], u32, FaceAttrs)>,
    /// Metadata of ORIGINAL edges (edges of the pre-triangulation faces), by
    /// point pair (smaller index first).
    pub original_edges: BTreeMap<(u32, u32), EdgeMeta>,
}

fn upair(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

/// Groups triangles into planar polygon faces and returns a whole-object
/// [`PolyPatch`] (`remove` = every face of `base`). Regions never span two
/// source faces and never cross an original edge. A new edge between two
/// regions of the same source face is soft (a seam inside a formerly flat or
/// smooth face); an original edge keeps its metadata; a triangle diagonal
/// interior to a region disappears. Every point becomes `PVert::New` (the
/// caller has moved them). Claims and blend tags are dropped (planarize runs
/// only after non-similarity moves); material, back material and UV frame
/// are kept. Deterministic: regions grow by BFS in ascending triangle index.
pub(crate) fn planarize(base: &Object, soup: &TriSoup) -> PolyPatch {
    // 1. Adjacency through shared undirected point pairs.
    let mut by_edge: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (ti, (t, _, _)) in soup.tris.iter().enumerate() {
        for k in 0..3 {
            by_edge
                .entry(upair(t[k], t[(k + 1) % 3]))
                .or_default()
                .push(ti);
        }
    }
    let pts =
        |idx: &[u32]| -> Vec<Point3> { idx.iter().map(|&i| soup.points[i as usize]).collect() };

    // 2. Region growing.
    let n = soup.tris.len();
    let mut region_of: Vec<Option<usize>> = vec![None; n];
    let mut regions: Vec<Vec<usize>> = Vec::new();
    for seed in 0..n {
        if region_of[seed].is_some() {
            continue;
        }
        let rid = regions.len();
        let Ok(mut plane) = Plane::from_polygon(&pts(&soup.tris[seed].0)) else {
            region_of[seed] = Some(rid);
            regions.push(vec![seed]);
            continue;
        };
        region_of[seed] = Some(rid);
        let mut members = vec![seed];
        let mut queue = std::collections::VecDeque::from([seed]);
        while let Some(t) = queue.pop_front() {
            let tri = soup.tris[t].0;
            let mut nbrs: Vec<(usize, (u32, u32))> = Vec::new();
            for k in 0..3 {
                let key = upair(tri[k], tri[(k + 1) % 3]);
                for &u in &by_edge[&key] {
                    if u != t {
                        nbrs.push((u, key));
                    }
                }
            }
            nbrs.sort_by_key(|&(u, _)| u);
            for (u, shared) in nbrs {
                if region_of[u].is_some()
                    || soup.tris[u].1 != soup.tris[seed].1
                    || soup.original_edges.contains_key(&shared)
                {
                    continue;
                }
                let up = pts(&soup.tris[u].0);
                if up
                    .iter()
                    .any(|&p| plane.signed_distance(p).abs() > tol::PLANARIZE_COPLANAR)
                {
                    continue;
                }
                // Refit over the grown region and re-check every point.
                let mut all: Vec<u32> = members.iter().flat_map(|&m| soup.tris[m].0).collect();
                all.extend(soup.tris[u].0);
                all.sort_unstable();
                all.dedup();
                let candidate_pts = pts(&all);
                let refit = newell_plane(&candidate_pts, plane);
                if candidate_pts
                    .iter()
                    .all(|&p| refit.signed_distance(p).abs() <= tol::PLANARIZE_COPLANAR)
                {
                    plane = refit;
                    region_of[u] = Some(rid);
                    members.push(u);
                    queue.push_back(u);
                }
            }
        }
        regions.push(members);
    }

    // 3. Boundary tracing per region (pinches and odd loop shapes dissolve
    // the region into its triangles).
    let mut faces: Vec<(Vec<Vec<u32>>, usize)> = Vec::new(); // loops (outer first), source tri
    for members in &regions {
        match trace_region(soup, members) {
            Some(loops) => faces.push((loops, members[0])),
            None => {
                for &t in members {
                    let tri = soup.tris[t].0;
                    faces.push((vec![tri.to_vec()], t));
                }
            }
        }
    }

    // 4–6. Emit the patch.
    let mut patch = PolyPatch {
        verts: soup.points.iter().map(|&p| PVert::New(p)).collect(),
        remove: base.faces().keys().collect(),
        add: Vec::with_capacity(faces.len()),
        edge_meta: BTreeMap::new(),
        expect_watertight: base.watertight() == WatertightState::Watertight,
    };
    let mut region_edges: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new(); // pair → source of each face using it
    for (loops, t) in &faces {
        let (_, source, attrs) = soup.tris[*t];
        let mut a = attrs;
        a.surface = None;
        a.blend = None;
        let as_usize = |l: &Vec<u32>| l.iter().map(|&i| i as usize).collect::<Vec<usize>>();
        patch.add.push(PatchFace {
            outer: as_usize(&loops[0]),
            holes: loops[1..].iter().map(as_usize).collect(),
            attrs: a,
            plane: None,
        });
        for lp in loops {
            for k in 0..lp.len() {
                region_edges
                    .entry(upair(lp[k], lp[(k + 1) % lp.len()]))
                    .or_default()
                    .push(source);
            }
        }
    }
    for (&(a, b), sources) in &region_edges {
        let meta = if let Some(m) = soup.original_edges.get(&(a, b)) {
            *m
        } else if sources.len() == 2 && sources[0] == sources[1] {
            EdgeMeta {
                soft: true,
                ..EdgeMeta::default()
            }
        } else {
            EdgeMeta::default()
        };
        patch.edge_meta.insert((a as usize, b as usize), meta);
    }
    patch
}

/// Newell plane of `points`, oriented like `like`; `like` if degenerate.
fn newell_plane(points: &[Point3], like: Plane) -> Plane {
    match Plane::from_polygon(points) {
        Ok(p) if p.normal().dot(like.normal()) >= 0.0 => p,
        Ok(p) => p.flipped(),
        Err(_) => like,
    }
}

/// The boundary loops of a region of triangles (outer first, CCW), or
/// `None` when the region is pinched or does not have exactly one outer
/// loop.
fn trace_region(soup: &TriSoup, members: &[usize]) -> Option<Vec<Vec<u32>>> {
    let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
    for &t in members {
        let tri = soup.tris[t].0;
        for k in 0..3 {
            directed.insert((tri[k], tri[(k + 1) % 3]));
        }
    }
    let boundary: Vec<(u32, u32)> = directed
        .iter()
        .copied()
        .filter(|&(a, b)| !directed.contains(&(b, a)))
        .collect();
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    for &(a, b) in &boundary {
        if next.insert(a, b).is_some() {
            return None; // pinch: two boundary edges leave one point
        }
    }
    let mut loops: Vec<Vec<u32>> = Vec::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for &(start, _) in &boundary {
        if seen.contains(&start) {
            continue;
        }
        let mut lp = vec![start];
        seen.insert(start);
        let mut cur = next[&start];
        while cur != start {
            if !seen.insert(cur) {
                return None;
            }
            lp.push(cur);
            cur = *next.get(&cur)?;
        }
        loops.push(lp);
    }
    // Classify by signed area about the region's normal.
    let first = soup.tris[members[0]].0;
    let normal = Plane::from_polygon(&[
        soup.points[first[0] as usize],
        soup.points[first[1] as usize],
        soup.points[first[2] as usize],
    ])
    .ok()?
    .normal();
    let area = |lp: &[u32]| -> f64 {
        let ring: Vec<Point3> = lp.iter().map(|&i| soup.points[i as usize]).collect();
        crate::geom2d::signed_area_on_plane(&ring, normal)
    };
    let (outer, holes): (Vec<_>, Vec<_>) = loops.into_iter().partition(|lp| area(lp) > 0.0);
    if outer.len() != 1 {
        return None;
    }
    let mut out = outer;
    out.extend(holes);
    Some(out)
}

// ════════════════════════════════════════ self-intersection guard

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

/// The first pair (in deterministic order) of faces of `obj`, one of them in
/// `candidates`, that contact improperly (`faces_improperly_contact`: contact
/// away from their shared vertices and edges) and whose bounds overlap
/// (expanded by `POINT_MERGE`). `None` = clean. Candidates in their given
/// order, partners in ascending `FaceId` order. A pair of two candidates is
/// tested once.
pub(crate) fn improper_contact(obj: &Object, candidates: &[FaceId]) -> Option<(FaceId, FaceId)> {
    improper_contact_charged(obj, candidates, 0, &mut |_, _| true).unwrap_or(None)
}

/// [`improper_contact`] that reports its work as it goes, as `charge(n, t)`:
/// `n` the cells each candidate is registered in; for every face matched
/// against the grid, one unit plus one per cell it visits and per
/// candidate it is held against; and for every pair tested, the boundary
/// vertices of the two faces. `t` is `n` again, except for a pair one face
/// of which lies clear of the other's plane ([`Flats::apart`]): that pair
/// is passed over for a fraction of the cost, and `t` is the caller's
/// `apart` weight.
/// `Err(())` as soon as `charge` returns `false`: the caller's budget ran
/// out (a live preview gives up rather than test hundreds of thousands of
/// pairs). The answer, when there is one, is [`improper_contact`]'s.
pub(crate) fn improper_contact_charged(
    obj: &Object,
    candidates: &[FaceId],
    apart: u64,
    charge: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<Option<(FaceId, FaceId)>, ()> {
    improper_contact_scan(obj, candidates, apart, charge, &mut |_, _| true)
}

/// [`improper_contact_charged`]'s scan; `hit(c, p)` hears each improper
/// pair and says whether to stop there (that pair is then the answer).
fn improper_contact_scan(
    obj: &Object,
    candidates: &[FaceId],
    apart: u64,
    charge: &mut dyn FnMut(u64, u64) -> bool,
    hit: &mut dyn FnMut(FaceId, FaceId) -> bool,
) -> Result<Option<(FaceId, FaceId)>, ()> {
    let cand_set: BTreeSet<FaceId> = candidates.iter().copied().collect();
    let cands: Vec<(FaceId, [f64; 6])> = candidates
        .iter()
        .copied()
        .filter(|&c| obj.faces().contains_key(c))
        .map(|c| (c, face_bounds(obj, c)))
        .collect();
    if cands.is_empty() {
        return Ok(None);
    }
    // Each candidate's partners: the faces whose bounds overlap its own,
    // found through a grid over the candidates (cells the size of a typical
    // candidate) so a big object is not scanned once per candidate. A box
    // over too many cells (a big face) is matched against everything.
    let mut extents: Vec<f64> = cands
        .iter()
        .map(|(_, b)| (b[3] - b[0]).max(b[4] - b[1]).max(b[5] - b[2]))
        .collect();
    extents.sort_by(f64::total_cmp);
    let h = extents[extents.len() / 2].max(tol::PATCH_MIN_EXTENT);
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
    let cells = |rs: &[std::ops::RangeInclusive<i64>; 3]| -> u64 {
        rs.iter()
            .map(|r| (r.end() - r.start() + 1) as u64)
            .product()
    };
    for (i, (_, b)) in cands.iter().enumerate() {
        match span(b) {
            Some([xs, ys, zs]) => {
                let n =
                    cells(&[xs.clone(), ys.clone(), zs.clone()]).div_ceil(GRID_STEPS_PER_VERTEX);
                if !charge(n, n) {
                    return Err(());
                }
                for x in xs {
                    for y in ys.clone() {
                        for z in zs.clone() {
                            grid.entry([x, y, z]).or_default().push(i);
                        }
                    }
                }
            }
            None => {
                if !charge(1, 1) {
                    return Err(());
                }
                big.push(i);
            }
        }
    }
    let overlaps =
        |b: &[f64; 6], cb: &[f64; 6]| (0..3).all(|k| b[k] <= cb[k + 3] && b[k + 3] >= cb[k]);
    let mut partners: Vec<Vec<FaceId>> = vec![Vec::new(); cands.len()];
    let mut stamp: Vec<usize> = vec![usize::MAX; cands.len()];
    for (n, f) in obj.faces().keys().enumerate() {
        let b = face_bounds(obj, f);
        // One unit for the face, one per cell it visits and one per
        // candidate it is held against.
        let mut cost: u64 = 1 + big.len() as u64;
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
                            let held = grid.get(&[x, y, z]);
                            cost += 1 + held.map_or(0, |v| v.len() as u64);
                            for &i in held.into_iter().flatten() {
                                consider(i, &mut partners);
                            }
                        }
                    }
                }
            }
            None => {
                cost += cands.len() as u64;
                for i in 0..cands.len() {
                    consider(i, &mut partners);
                }
            }
        }
        for &i in &big {
            consider(i, &mut partners);
        }
        let n = cost.div_ceil(GRID_STEPS_PER_VERTEX);
        if !charge(n, n) {
            return Err(());
        }
    }
    let ring = |f: FaceId| -> u64 {
        std::iter::once(obj.faces()[f].outer_loop)
            .chain(obj.faces()[f].inner_loops.iter().copied())
            .map(|l| obj.loop_half_edges(l).count() as u64)
            .sum()
    };
    let mut flats = Flats::default();
    for (i, (c, _)) in cands.iter().enumerate() {
        let mut ps = std::mem::take(&mut partners[i]);
        ps.sort();
        let rc = ring(*c);
        for p in ps {
            let skip = flats.apart(obj, *c, p);
            let n = rc + ring(p);
            if !charge(n, if skip { apart } else { n }) {
                return Err(());
            }
            if skip {
                continue;
            }
            if crate::ops::faces_improperly_contact(obj, *c, p) && hit(*c, p) {
                return Ok(Some((*c, p)));
            }
        }
    }
    Ok(None)
}

/// Per face, for [`Flats::apart`]: its boundary points (every loop), how
/// far they stray from its own plane, and the diagonal of their box,
/// gathered once per face.
#[derive(Default)]
struct Flats {
    seen: SecondaryMap<FaceId, Flat>,
    points: Vec<Point3>,
}

#[derive(Clone, Copy)]
struct Flat {
    start: usize,
    end: usize,
    stray: f64,
    reach: f64,
}

impl Flats {
    fn of(&mut self, obj: &Object, f: FaceId) -> Flat {
        if let Some(&r) = self.seen.get(f) {
            return r;
        }
        let face = &obj.faces()[f];
        let start = self.points.len();
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            self.points.extend(obj.loop_positions(l));
        }
        let pts = &self.points[start..];
        let stray = pts
            .iter()
            .map(|&q| face.plane.signed_distance(q).abs())
            .fold(0.0, f64::max);
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in pts {
            for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
        let reach = if pts.is_empty() {
            0.0
        } else {
            (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt()
        };
        let r = Flat {
            start,
            end: self.points.len(),
            stray,
            reach,
        };
        self.seen.insert(f, r);
        r
    }

    /// Whether one of faces `a` and `b` lies wholly on one side of the
    /// other's plane, clear of it by more than both faces stray from their
    /// own planes plus every tolerance [`crate::ops::faces_improperly_contact`]
    /// works to. That test then answers false: no boundary segment of the
    /// clear face reaches the other's plane, and any contact it could find
    /// for the other face's boundary would have to lie, within those
    /// tolerances, on the clear face as well, which is farther off. A
    /// shortcut for the many pairs of a dense solid whose boxes overlap but
    /// whose faces come nowhere near each other; it never passes a pair
    /// that test would call improper.
    fn apart(&mut self, obj: &Object, a: FaceId, b: FaceId) -> bool {
        let fa = self.of(obj, a);
        let fb = self.of(obj, b);
        // The near-parallel branch of the segment test may place a contact
        // up to a segment's length times `NORMAL_DIRECTION` off its line.
        let margin = fa.stray
            + fb.stray
            + 4.0 * (tol::PLANE_DIST + tol::POINT_MERGE)
            + (fa.reach + fb.reach) * tol::NORMAL_DIRECTION;
        let clear = |pts: &[Point3], plane: Plane| -> bool {
            let Some(&first) = pts.first() else {
                return false;
            };
            let s = if plane.signed_distance(first) > 0.0 {
                1.0
            } else {
                -1.0
            };
            pts.iter().all(|&q| s * plane.signed_distance(q) > margin)
        };
        clear(&self.points[fa.start..fa.end], obj.faces()[b].plane)
            || clear(&self.points[fb.start..fb.end], obj.faces()[a].plane)
    }
}

/// Grid steps (a cell visited, a candidate held against a face, a cell
/// registered) that [`improper_contact_charged`] reports as one boundary
/// vertex: a step is a lookup and a stamp, a pair test's vertex a polygon
/// probe, measured some sixteen times dearer.
const GRID_STEPS_PER_VERTEX: u64 = 16;

/// The most grid cells a face's box may cover in [`improper_contact`]
/// before it is matched against every candidate instead.
const GRID_MAX_CELLS: i128 = 512;

/// [`improper_contact`] over every face of `obj`.
pub(crate) fn self_intersection(obj: &Object) -> Option<(FaceId, FaceId)> {
    let all: Vec<FaceId> = obj.faces().keys().collect();
    improper_contact(obj, &all)
}

/// [`self_intersection`] reporting its work to `charge`
/// ([`improper_contact_charged`]); `Err(())` when the budget ran out.
pub(crate) fn self_intersection_charged(
    obj: &Object,
    apart: u64,
    charge: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<Option<(FaceId, FaceId)>, ()> {
    let all: Vec<FaceId> = obj.faces().keys().collect();
    improper_contact_charged(obj, &all, apart, charge)
}

// ═══════════════════════════════════════════════════ test support

impl Object {
    /// [`Object::apply_patch`] for the executable specs. Test support only;
    /// never call from product code.
    #[doc(hidden)]
    pub fn apply_patch_for_test(&self, patch: &PolyPatch) -> Result<PatchResult, TopologyError> {
        self.apply_patch(patch)
    }

    /// [`Object::to_patch`] for the executable specs.
    #[doc(hidden)]
    pub fn to_patch_for_test(&self) -> PolyPatch {
        self.to_patch()
    }
}

/// [`planarize`] for the executable specs.
#[doc(hidden)]
pub fn planarize_for_test(base: &Object, soup: &TriSoup) -> PolyPatch {
    planarize(base, soup)
}

/// [`improper_contact`] for the executable specs.
#[doc(hidden)]
pub fn improper_contact_for_test(obj: &Object, candidates: &[FaceId]) -> Option<(FaceId, FaceId)> {
    improper_contact(obj, candidates)
}

/// What [`self_intersection_charged`] charges for `obj` in all, and its
/// answer under `budget` units (`Err(())` when it ran out). Test support
/// only.
#[doc(hidden)]
pub fn self_intersection_cost_for_test(obj: &Object, budget: u64) -> (u64, Result<bool, ()>) {
    let mut total = 0u64;
    let r = self_intersection_charged(obj, 0, &mut |n, _| {
        total = total.saturating_add(n);
        total <= budget
    });
    (total, r.map(|x| x.is_some()))
}

/// Every pair of faces of `obj` whose bounds overlap, against the plane
/// shortcut of [`improper_contact_charged`]: how many pairs it passes over,
/// and how many of those the full test calls improper (the shortcut is
/// sound when that is none). Test support only.
#[doc(hidden)]
pub fn plane_shortcut_census_for_test(obj: &Object) -> (usize, usize) {
    let faces: Vec<(FaceId, [f64; 6])> = obj
        .faces()
        .keys()
        .map(|f| (f, face_bounds(obj, f)))
        .collect();
    let overlaps =
        |b: &[f64; 6], cb: &[f64; 6]| (0..3).all(|k| b[k] <= cb[k + 3] && b[k + 3] >= cb[k]);
    let mut flats = Flats::default();
    let (mut passed, mut wrong) = (0, 0);
    for (i, (a, ba)) in faces.iter().enumerate() {
        for (b, bb) in &faces[i + 1..] {
            if overlaps(ba, bb) && flats.apart(obj, *a, *b) {
                passed += 1;
                if crate::ops::faces_improperly_contact(obj, *a, *b) {
                    wrong += 1;
                }
            }
        }
    }
    (passed, wrong)
}

/// [`self_intersection`] for the executable specs.
#[doc(hidden)]
pub fn self_intersection_for_test(obj: &Object) -> Option<(FaceId, FaceId)> {
    self_intersection(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`Object::splice_closed`] refuses a face listed twice in `remove`,
    /// typed, as [`Object::apply_patch`] does, and splices it once.
    #[test]
    fn splice_closed_refuses_a_face_removed_twice() {
        let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        let pts = [
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ];
        let quads = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let o = Object::from_polygons(&pts, &quads).unwrap();
        let top = o
            .faces()
            .keys()
            .find(|&f| o.faces()[f].plane.normal().z > 0.5)
            .unwrap();
        let ring: Vec<VertexId> = o
            .loop_half_edges(o.faces()[top].outer_loop)
            .map(|h| o.half_edges()[h].origin)
            .collect();
        let verts: Vec<PVert> = ring.iter().map(|&v| PVert::Existing(v)).collect();
        let add = vec![PatchFace {
            outer: (0..ring.len()).collect(),
            holes: Vec::new(),
            attrs: o.faces()[top].attrs(),
            plane: Some(o.faces()[top].plane),
        }];
        assert!(matches!(
            o.splice_closed(&[top, top], &verts, &add, crate::tol::PLANE_DIST),
            Err(TopologyError::DanglingHandle { .. })
        ));
        assert!(
            o.splice_closed(&[top], &verts, &add, crate::tol::PLANE_DIST)
                .is_ok()
        );
    }
}
