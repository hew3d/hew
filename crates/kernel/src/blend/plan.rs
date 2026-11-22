//! Planning a blend (WP05 §8): classify, chain, section, build the vertex
//! constructions, assemble one patch, and check it. Pure; nothing is
//! applied here.

use crate::trig;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::chain::{Chain, build_chains, canonical_swept, coplanar};
use super::classify::{EdgeKind, classify};
use super::section::Section;
use super::station::{axial, facet_faces, station_section};
use super::{BlendError, BlendProfile, BlendSpec, Convexity, LimitFeature};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::sketch::CurveGeom;
use crate::tol;
use crate::topo::{BlendId, BlendKind, BlendRole, BlendTag, Object, SurfaceRef};

/// Why a plan failed: a size limit (the feature that stopped it), a hard
/// refusal, or (from the apply step) an obstruction at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PlanFail {
    Size(LimitFeature),
    Hard(BlendError),
    Obstructed(Point3),
}

impl From<BlendError> for PlanFail {
    fn from(e: BlendError) -> PlanFail {
        PlanFail::Hard(e)
    }
}

/// A planned blend.
pub(crate) struct BlendPlan {
    pub patch: PolyPatch,
    /// Faces the patch removes (original + modified).
    pub replaced: Vec<FaceId>,
    /// Indices into `patch.add` that are blend faces.
    pub new_face_range: Range<usize>,
    pub blend: BlendId,
    /// Where a swept chain took the solve against the side surfaces (the
    /// walk refused it), the walk's refusal: it stands if the result then
    /// fails its checks, so the fallback only ever turns a refusal into a
    /// blend.
    pub fallback: Option<PlanFail>,
}

/// The patch's point table: existing vertices by id, new points once each.
pub(crate) struct Pts {
    pub verts: Vec<PVert>,
    existing: BTreeMap<VertexId, usize>,
    positions: Vec<Point3>,
}

impl Pts {
    fn new() -> Pts {
        Pts {
            verts: Vec::new(),
            existing: BTreeMap::new(),
            positions: Vec::new(),
        }
    }
    pub(crate) fn existing(&mut self, obj: &Object, v: VertexId) -> usize {
        if let Some(&i) = self.existing.get(&v) {
            return i;
        }
        let i = self.verts.len();
        self.verts.push(PVert::Existing(v));
        self.positions.push(obj.vertices()[v].position);
        self.existing.insert(v, i);
        i
    }
    pub(crate) fn add(&mut self, p: Point3) -> usize {
        let i = self.verts.len();
        self.verts.push(PVert::New(p));
        self.positions.push(p);
        i
    }
    pub(crate) fn pos(&self, i: usize) -> Point3 {
        self.positions[i]
    }
    /// The point of existing vertex `v`, if the plan uses it.
    fn existing_index(&self, v: VertexId) -> Option<usize> {
        self.existing.get(&v).copied()
    }
    /// Moves a point this plan creates to `p` (an existing vertex stays put).
    pub(crate) fn snap(&mut self, i: usize, p: Point3) {
        if let PVert::New(q) = &mut self.verts[i] {
            *q = p;
            self.positions[i] = p;
        }
    }
}

/// The cuts of one strip end at a vertex construction: the point on each
/// ruling `0..=N` where the strip's last segment ends, and per facet the
/// extra points (ruling k → ruling k+1 order) the construction adds there.
#[derive(Debug, Clone, Default)]
pub(crate) struct StripEnd {
    pub ruling_cut: Vec<Option<usize>>,
    pub facet_extra: Vec<Vec<usize>>,
}

/// One chain's strip.
pub(crate) struct Strip {
    pub chain: Chain,
    /// The section at each chain vertex (a closed chain repeats the first).
    pub secs: Vec<Section>,
    /// The station point of each chain vertex: the vertex itself, except
    /// where a construction ends the strip elsewhere (a rolled corner).
    pub qs: Vec<Point3>,
    /// The one patch point every station shares on a ruling that collapses
    /// onto the axis (a hemisphere's pole), with that ruling's index.
    pub pole: Option<(usize, usize)>,
    /// The facet the side-B rail trims (seam crossing).
    pub trimmed_b: Option<FaceId>,
    /// Mid-edge stations per segment (variable radius), in chain order.
    pub mids: Vec<Vec<Mid>>,
    /// The sharp-edge end point `R` of a stopped end (start, end).
    pub stop: [Option<usize>; 2],
    /// How the radius varies along the chain.
    pub variable: Variable,
    /// A linear taper's cone apex: its rulings are generators through it,
    /// not lines along the chain.
    pub apex: Option<Point3>,
    /// Facet count (N): `segments` of a round, 1 for a chamfer.
    pub n: usize,
    pub convexity: Convexity,
    /// Station points at interior (pass-through) chain vertices, by chain
    /// vertex index (`None` at the two ends).
    pub stations: Vec<Option<Vec<usize>>>,
    /// The constructions' cuts at the start (`ends[0]`) and end.
    pub ends: [StripEnd; 2],
    /// Extra points on rulings 0 and N inside each segment, with their
    /// distance along the chain direction from the segment's start.
    pub inserts: Vec<[Vec<(f64, usize)>; 2]>,
}

impl Strip {
    /// The edge info at end `which` (0 = start, 1 = end).
    pub(crate) fn end_info(&self, which: usize) -> &super::classify::EdgeInfo {
        if which == 0 {
            &self.chain.edges[0]
        } else {
            self.chain.edges.last().expect("a chain has edges")
        }
    }
    /// The end vertex `which`.
    pub(crate) fn end_vertex(&self, which: usize) -> VertexId {
        if which == 0 {
            self.chain.verts[0]
        } else {
            *self.chain.verts.last().expect("a chain has vertices")
        }
    }
    /// The chain vertex index of end `which`.
    pub(crate) fn end_index(&self, which: usize) -> usize {
        if which == 0 {
            0
        } else {
            self.chain.verts.len() - 1
        }
    }
    /// The section at end `which`.
    pub(crate) fn end_sec(&self, which: usize) -> &Section {
        &self.secs[self.end_index(which)]
    }
    /// The ruling direction at end `which`, pointing out of the strip toward
    /// the end vertex.
    pub(crate) fn toward_end(&self, obj: &Object, which: usize) -> Vec3 {
        if which == 0 {
            -self.chain.seg_dir(obj, 0)
        } else {
            self.chain.seg_dir(obj, self.chain.verts.len() - 2)
        }
    }
}

/// A station between two chain vertices.
#[derive(Debug, Clone)]
pub(crate) struct Mid {
    pub q: Point3,
    pub sec: Section,
    pub pts: Vec<usize>,
}

/// How a round's radius varies along its chain (WP07 §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variable {
    Constant,
    /// Linear in arc length on one straight chain: cone strips.
    Linear,
    /// Monotone cubic in arc length: generated, unclaimed.
    General,
}

/// Everything the vertex constructions write.
#[derive(Default, Clone)]
pub(crate) struct Out {
    /// For each (face, replaced vertex): the run of points replacing it, in
    /// the face's loop order.
    pub face_runs: BTreeMap<(FaceId, VertexId), Vec<usize>>,
    /// For each unpicked edge shortened at a vertex: (that vertex, new point).
    pub shortened: BTreeMap<EdgeId, Vec<(VertexId, usize)>>,
    /// Explicit edge metadata by patch point pair.
    pub edge_meta: BTreeMap<(usize, usize), EdgeMeta>,
    /// Interior run points of concave constructions: they lie in the face's
    /// plane outside its original outline (the fill grows the face), so C2
    /// does not apply to them; C4 still checks their loop is simple.
    pub fill_points: BTreeSet<usize>,
    /// Faces removed entirely (consumed facets, a hemisphere's cap).
    pub removed: BTreeSet<FaceId>,
    /// Faces made at vertex constructions (rolled corners, flat corners,
    /// stop fans).
    pub corner_faces: Vec<CornerFace>,
    /// Points inserted into a face's loop right before / after a vertex
    /// (stop fans, mid-edge rail points), in loop order.
    pub before: BTreeMap<(FaceId, VertexId), Vec<usize>>,
    pub after: BTreeMap<(FaceId, VertexId), Vec<usize>>,
    /// Faces a swept chain has already replaced, with the places in the
    /// kept list of the pieces that stand for them: a later chain on the
    /// same face (two junctions on one wall, two open chains ending on one
    /// cut face) cuts those pieces, not the face again.
    pub swept_faces: BTreeMap<FaceId, Vec<usize>>,
    /// Where imprints leave the cut on a strip's side ruling, placed once
    /// every construction has cut the rulings ([`place_crossings`]).
    pub crossings: Vec<Crossing>,
}

/// A point where an imprint line leaves the cut across a strip's side
/// ruling inside one segment. A ruling runs from the cut at one end of the
/// segment to the cut at the other, askew to the edge where the radius
/// ramps and set by constructions that may come later, so the point is
/// placed on it once they have all run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Crossing {
    /// The patch point (a new point, snapped into place).
    pub point: usize,
    pub strip: usize,
    pub seg: usize,
    /// 0: ruling 0 (side A); 1: ruling N (side B).
    pub side: usize,
    /// The imprint line: from the vertex, its unit direction, and its length.
    pub from: Point3,
    pub dir: Vec3,
    pub reach: f64,
    /// Whether the vertex the imprint leaves is at the segment's start.
    pub at_start: bool,
}

/// A face a vertex construction makes: its ring, claim (verified before
/// stamping), kind when it differs from the blend's, and the vertex and
/// convexity that orient it.
#[derive(Clone)]
pub(crate) struct CornerFace {
    pub ring: Vec<usize>,
    pub claim: Option<SurfaceRef>,
    pub kind: Option<BlendKind>,
    pub role: BlendRole,
    pub at: Point3,
    pub convex: bool,
    /// The ring edges (point pairs) that are soft seams.
    pub soft: Vec<(usize, usize)>,
}

impl Out {
    pub(crate) fn meta(&mut self, a: usize, b: usize, m: EdgeMeta) {
        self.edge_meta.insert((a.min(b), a.max(b)), m);
    }
}

/// The shared inputs of a vertex construction.
pub(crate) struct Ctx<'a> {
    pub obj: &'a Object,
    pub vertex_edges: &'a BTreeMap<VertexId, Vec<EdgeId>>,
    pub picked: &'a BTreeSet<EdgeId>,
    pub profile: &'a BlendProfile,
    pub corners: super::CornerStyle,
}

impl Ctx<'_> {
    /// The faces incident to `v`, ascending.
    pub(crate) fn faces_at(&self, v: VertexId) -> Vec<FaceId> {
        let mut out: BTreeSet<FaceId> = BTreeSet::new();
        for &e in self
            .vertex_edges
            .get(&v)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
        {
            if let Some((a, b)) = self.obj.edge_faces(e) {
                out.insert(a);
                out.insert(b);
            }
        }
        out.into_iter().collect()
    }
    pub(crate) fn edges_at(&self, v: VertexId) -> Vec<EdgeId> {
        self.vertex_edges.get(&v).cloned().unwrap_or_default()
    }
    /// The edge between faces `f` and `g` at vertex `v`, if any.
    pub(crate) fn edge_between_faces(&self, v: VertexId, f: FaceId, g: FaceId) -> Option<EdgeId> {
        self.edges_at(v).into_iter().find(|&e| {
            self.obj
                .edge_faces(e)
                .is_some_and(|(x, y)| (x == f && y == g) || (x == g && y == f))
        })
    }
    /// The vertex after `v` in `f`'s loops.
    pub(crate) fn next_in_face(&self, f: FaceId, v: VertexId) -> Option<VertexId> {
        for lid in self.obj.face_loops(f) {
            for h in self.obj.loop_half_edges(lid) {
                let he = self.obj.half_edges()[h];
                if he.origin == v {
                    return Some(self.obj.half_edges()[he.next].origin);
                }
            }
        }
        None
    }
    /// The vertex before `v` in `f`'s loops (the origin of the half-edge
    /// arriving at `v`).
    pub(crate) fn prev_in_face(&self, f: FaceId, v: VertexId) -> Option<VertexId> {
        for lid in self.obj.face_loops(f) {
            for h in self.obj.loop_half_edges(lid) {
                let he = self.obj.half_edges()[h];
                if self.obj.half_edges()[he.next].origin == v {
                    return Some(he.origin);
                }
            }
        }
        None
    }
}

impl Object {
    /// Pure: classify, chain, section, vertex constructions, assemble,
    /// checks C1–C5. The patch is not applied (C6 is the caller's).
    pub(crate) fn plan_blend(&self, spec: &BlendSpec) -> Result<BlendPlan, PlanFail> {
        let infos = classify(self, spec)?;
        if let BlendProfile::Bullnose { segments } = spec.profile {
            return plan_bullnose(self, &infos, segments as usize);
        }
        let vertex_edges = self.vertex_edges();
        let chains = build_chains(self, &infos, &vertex_edges, &spec.profile)?;
        let picked: BTreeSet<EdgeId> = infos.iter().map(|i| i.edge).collect();
        let ctx = Ctx {
            obj: self,
            vertex_edges: &vertex_edges,
            picked: &picked,
            profile: &spec.profile,
            corners: spec.corners,
        };
        let mut pts = Pts::new();
        let mut out = Out::default();

        // Strips and their pass-through stations; swept chains apart.
        let mut strips: Vec<Strip> = Vec::with_capacity(chains.len());
        let mut swept: Vec<Chain> = Vec::new();
        for chain in chains {
            if chain.edges.iter().any(|e| e.kind == EdgeKind::Swept) {
                swept.push(canonical_swept(self, chain)?);
                continue;
            }
            let m = chain.verts.len();
            // Each station sections by the rule of the edge arriving at it
            // (the first edge at the start).
            // Arc length of each chain vertex from the chain start.
            let mut arc: Vec<f64> = vec![0.0];
            for i in 1..m {
                let (a, b) = (chain.verts[i - 1], chain.verts[i]);
                let d = (self.vertices()[b].position - self.vertices()[a].position).length();
                arc.push(arc[i - 1] + d);
            }
            let radius = radius_law(spec, &chain, arc[m - 1])?;
            let variable = match &radius {
                None => Variable::Constant,
                Some(law) if law.linear => Variable::Linear,
                Some(_) => Variable::General,
            };
            let profile_at = |len: f64| match (&radius, &spec.profile) {
                (Some(law), BlendProfile::Round { segments, .. }) => BlendProfile::Round {
                    radius: law.at(len),
                    segments: *segments,
                },
                _ => spec.profile.clone(),
            };
            let mut secs = Vec::with_capacity(m);
            for (i, &len) in arc.iter().enumerate() {
                let info = &chain.edges[i.saturating_sub(1)];
                secs.push(station_section(
                    self,
                    &vertex_edges,
                    info,
                    chain.verts[i],
                    &profile_at(len),
                )?);
            }
            if chain.closed {
                secs[m - 1] = secs[0].clone();
            }
            let apex = if variable == Variable::Linear {
                taper_onto_cone(self, &chain, &mut secs)?
            } else {
                None
            };
            let n = secs[0].offsets.len() - 1;
            if secs.iter().any(|s| s.offsets.len() != n + 1) {
                return Err(PlanFail::Hard(BlendError::BlendDegenerate));
            }
            let qs: Vec<Point3> = chain
                .verts
                .iter()
                .map(|&v| self.vertices()[v].position)
                .collect();
            let mut strip = Strip {
                convexity: chain.edges[0].convexity,
                secs,
                qs,
                pole: None,
                trimmed_b: None,
                mids: vec![Vec::new(); m - 1],
                stop: [None, None],
                variable,
                apex,
                n,
                stations: vec![None; m],
                ends: [StripEnd::default(), StripEnd::default()],
                inserts: vec![[Vec::new(), Vec::new()]; m - 1],
                chain,
            };
            rim_checks(self, &mut strip, &mut pts, &mut out)?;
            seam_crossing(self, &mut strip, &mut out)?;
            if let Some(law) = &radius
                && !law.linear
            {
                mid_stations(
                    self,
                    &vertex_edges,
                    &mut strip,
                    &arc,
                    law,
                    &profile_at,
                    &mut pts,
                    &mut out,
                )?;
            }
            if let Some(stops) = spec.stops {
                stop_ends(self, &mut strip, stops, &mut pts, &mut out)?;
            }
            let interior: Vec<usize> = if strip.chain.closed {
                (0..m - 1).collect()
            } else {
                (1..m - 1).collect()
            };
            for i in interior {
                pass_through(&ctx, strips.len(), &mut strip, i, &mut pts, &mut out)?;
            }
            if strip.chain.closed {
                strip.stations[m - 1] = strip.stations[0].clone();
            }
            strips.push(strip);
        }

        // One construction per chain-end vertex.
        let mut at_vertex: BTreeMap<VertexId, Vec<(usize, usize)>> = BTreeMap::new();
        for (si, s) in strips.iter().enumerate() {
            if s.chain.closed {
                continue;
            }
            for w in [0, 1] {
                if s.stop[w].is_none() {
                    at_vertex.entry(s.end_vertex(w)).or_default().push((si, w));
                }
            }
        }
        for (&v, ends) in &at_vertex {
            super::vertex::construct(&ctx, v, ends, &mut strips, &mut pts, &mut out)?;
        }
        for s in strips.iter().filter(|s| !s.chain.closed) {
            for e in &s.ends {
                if e.ruling_cut.len() != s.n + 1 || e.ruling_cut.iter().any(Option::is_none) {
                    return Err(PlanFail::Hard(BlendError::BlendDegenerate));
                }
            }
        }
        place_crossings(self, &mut strips, &mut pts, &out)?;

        let blend = self.new_blend_id();
        let kind = match spec.profile {
            BlendProfile::Chamfer { .. } => BlendKind::Chamfer,
            BlendProfile::Custom { .. } => BlendKind::Profile,
            _ => BlendKind::Fillet,
        };

        // C5: every strip segment's rulings run forward.
        for s in &strips {
            check_strip_lengths(self, s, &pts)?;
        }
        // C1: shortened edges stay ordered.
        check_shortened(self, &out, &pts)?;

        // Modified faces.
        let mut modified: BTreeSet<FaceId> = BTreeSet::new();
        for &(f, _) in out
            .face_runs
            .keys()
            .chain(out.before.keys())
            .chain(out.after.keys())
        {
            if !out.removed.contains(&f) {
                modified.insert(f);
            }
        }
        let mut add: Vec<PatchFace> = Vec::new();
        for &f in &modified {
            add.push(modified_face(self, f, &out, &mut pts));
        }
        // C2–C4 on the modified faces.
        for (pf, &f) in add.iter().zip(&modified) {
            check_modified_face(self, f, pf, &out, &pts, &strips)?;
        }
        // Swept chains cut their side faces and add their strips, in the
        // order of their start points (D-228: by geometry, not edge ids).
        swept.sort_by_key(|c| super::chain::lex_key(self.vertices()[c.verts[0]].position));
        let mut swept_strips: Vec<PatchFace> = Vec::new();
        let mut fallback = None;
        for chain in &swept {
            let refused = super::swept::plan_swept(
                self,
                chain,
                &spec.profile,
                blend,
                kind,
                &picked,
                &mut pts,
                &mut out,
                &mut add,
                &mut swept_strips,
            )?;
            fallback = fallback.or(refused);
        }
        // A piece a later chain cut again left its place empty.
        add.retain(|f| !f.outer.is_empty());
        if modified.iter().any(|f| out.removed.contains(f)) {
            let at = self
                .edge_endpoints(infos[0].edge)
                .map_or(Point3::new(0.0, 0.0, 0.0), |(a, _)| a);
            return Err(PlanFail::Hard(BlendError::UnsupportedVertex { at }));
        }

        // Strip facets.
        let first_new = add.len();
        for s in &strips {
            strip_faces(self, s, blend, kind, &mut pts, &mut out, &mut add)?;
        }
        corner_faces(self, &strips, blend, kind, &mut pts, &mut out, &mut add)?;
        add.extend(swept_strips);
        let new_face_range = first_new..add.len();
        let replaced: Vec<FaceId> = modified.union(&out.removed).copied().collect();
        carry_shortened_meta(self, &mut out, &pts);
        Ok(BlendPlan {
            patch: PolyPatch {
                verts: pts.verts,
                remove: replaced.clone(),
                add,
                edge_meta: out.edge_meta,
                expect_watertight: true,
            },
            replaced,
            new_face_range,
            blend,
            fallback,
        })
    }
}

/// A pass-through station at interior chain vertex `i`: one section point
/// per ruling; where an imprint edge on a side ends at the vertex, it is
/// shortened to the rail and its crossing joins the rail.
fn pass_through(
    ctx: &Ctx,
    strip_index: usize,
    strip: &mut Strip,
    i: usize,
    pts: &mut Pts,
    out: &mut Out,
) -> Result<(), PlanFail> {
    let obj = ctx.obj;
    let v = strip.chain.verts[i];
    let q = strip.qs[i];
    let station: Vec<usize> = strip.secs[i]
        .offsets
        .iter()
        .enumerate()
        .map(|(k, &o)| match strip.pole {
            Some((pk, idx)) if pk == k => idx,
            _ => pts.add(q + o),
        })
        .collect();
    let last_edge = strip.chain.edges.len() - 1;
    let e_in = &strip.chain.edges[if i == 0 { last_edge } else { i - 1 }];
    let e_out = &strip.chain.edges[i];
    let n = strip.n;
    for (side, ruling) in [(0usize, 0usize), (1, n)] {
        let (f_in, f_out) = if side == 0 {
            (e_in.a, e_out.a)
        } else {
            (e_in.b, e_out.b)
        };
        let p = station[ruling];
        if strip.pole.is_some_and(|(pk, _)| pk == ruling) {
            // The cap that collapses onto the pole is removed whole.
            continue;
        }
        if f_in == f_out {
            out.face_runs.insert((f_in, v), vec![p]);
            continue;
        }
        // An edge between the two side faces ends here: a band seam or a
        // tangent joint the rail point lies on, or an imprint.
        let imprints: Vec<EdgeId> = ctx
            .edges_at(v)
            .into_iter()
            .filter(|&e| {
                obj.edge_faces(e)
                    .is_some_and(|(x, y)| (x == f_in && y == f_out) || (x == f_out && y == f_in))
            })
            .collect();
        let [g] = imprints.as_slice() else {
            return Err(PlanFail::Hard(BlendError::UnsupportedVertex { at: q }));
        };
        let (ga, gb) = obj.edge_endpoints(*g).ok_or(BlendError::BlendDegenerate)?;
        if line_distance(pts.pos(p), ga, gb) <= tol::PLANE_DIST {
            out.shortened.entry(*g).or_default().push((v, p));
            out.face_runs.insert((f_in, v), vec![p]);
            out.face_runs.insert((f_out, v), vec![p]);
            continue;
        }
        if !coplanar(obj, f_in, f_out) {
            return Err(PlanFail::Hard(BlendError::UnsupportedVertex { at: q }));
        }
        let far = if ga.approx_eq(q, tol::POINT_MERGE) {
            gb
        } else {
            ga
        };
        let dir_g = (far - q)
            .normalized()
            .map_err(|_| BlendError::BlendDegenerate)?;
        let base = pts.pos(p);
        let w = base - q;
        let prev_seg = if i == 0 { last_edge } else { i - 1 };
        // The rail runs on from the station point along the next segment
        // and back along the previous one, which differ where the chain
        // bends (a rim): the imprint leaves the cut where it first meets
        // either, ahead on the one or behind on the other.
        let crossing = |seg: usize, ahead: bool| -> Option<(f64, f64, Vec3)> {
            let t = strip.chain.seg_dir(obj, seg);
            // base + s t = q + mu g  →  s = (w·g tg − w·t) / (1 − tg²)
            let tg = t.dot(dir_g);
            let denom = 1.0 - tg * tg;
            if denom <= tol::NORMAL_DIRECTION {
                return None;
            }
            let s = (w.dot(dir_g) * tg - w.dot(t)) / denom;
            let on = if ahead {
                s >= -tol::POINT_MERGE
            } else {
                s <= tol::POINT_MERGE
            };
            let mu = (base + t * s - q).dot(dir_g);
            (on && mu > -tol::POINT_MERGE).then_some((mu, s, t))
        };
        let (behind, s, t) = match (crossing(i, true), crossing(prev_seg, false)) {
            (Some(a), Some(b)) if b.0 < a.0 => (true, b.1, b.2),
            (Some(a), _) => (false, a.1, a.2),
            (None, Some(b)) => (true, b.1, b.2),
            (None, None) => return Err(PlanFail::Size(LimitFeature::Imprint)),
        };
        let x = base + t * s;
        let xi = if s.abs() <= tol::POINT_MERGE {
            p
        } else {
            pts.add(x)
        };
        out.shortened.entry(*g).or_default().push((v, xi));
        if xi == p {
            out.face_runs.insert((f_in, v), vec![p]);
            out.face_runs.insert((f_out, v), vec![p]);
            continue;
        }
        // The crossing lies in the segment before or after the station;
        // the face on that side ends at the crossing, the other side's face
        // runs from the station point to the crossing, or the other way
        // where its loop arrives at the vertex along the imprint (the
        // crossing lies on the imprint). Read from the loop, not from
        // distances: the station point and the crossing lie on one rail
        // parallel to the chain edge, so their distances from it tie, and
        // a tie broke by which face was side A.
        let g_far = obj
            .edge_vertex_ids(*g)
            .map(|(a, b)| if a == v { b } else { a });
        let ordered = |f: FaceId| -> Vec<usize> {
            if g_far.is_some() && ctx.prev_in_face(f, v) == g_far {
                vec![xi, p]
            } else {
                vec![p, xi]
            }
        };
        // Its distance along the segment from the segment's start; placed
        // on the ruling as cut once the constructions have run.
        let seg = if behind { prev_seg } else { i };
        if !strip.mids[seg].is_empty() {
            return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
        }
        let start = obj.vertices()[strip.chain.verts[seg]].position;
        let along = (x - start).dot(strip.chain.seg_dir(obj, seg));
        out.crossings.push(Crossing {
            point: xi,
            strip: strip_index,
            seg,
            side,
            from: q,
            dir: dir_g,
            reach: (far - q).length(),
            at_start: !behind,
        });
        if behind {
            out.face_runs.insert((f_in, v), vec![xi]);
            out.face_runs.insert((f_out, v), ordered(f_out));
        } else {
            out.face_runs.insert((f_in, v), ordered(f_in));
            out.face_runs.insert((f_out, v), vec![xi]);
        }
        strip.inserts[seg][side].push((along, xi));
    }
    strip.stations[i] = Some(station);
    Ok(())
}

/// Places each [`Crossing`] where its imprint meets the side ruling as the
/// constructions cut it (from the cut at the segment's start to the one at
/// its end), and its insert's distance along the segment with it. A
/// crossing past the ruling's far cut, or past the imprint's far end, is a
/// size limit: a smaller size keeps it inside. One behind the cut at its
/// own vertex is no size's: the corner is refused.
fn place_crossings(
    obj: &Object,
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &Out,
) -> Result<(), PlanFail> {
    let limit = PlanFail::Size(LimitFeature::Imprint);
    for c in &out.crossings {
        let s = &mut strips[c.strip];
        let k = if c.side == 0 { 0 } else { s.n };
        let (a, b) = (pts.pos(cut(s, c.seg, k)), pts.pos(cut(s, c.seg + 1, k)));
        let len = (b - a).length();
        let t = (b - a).normalized().map_err(|_| limit)?;
        // a + σ t = from + μ dir  →  σ = (w·dir tg − w·t) / (1 − tg²)
        let tg = t.dot(c.dir);
        let denom = 1.0 - tg * tg;
        if denom <= tol::NORMAL_DIRECTION {
            return Err(limit);
        }
        let w = a - c.from;
        let sigma = (w.dot(c.dir) * tg - w.dot(t)) / denom;
        let x = a + t * sigma;
        let mu = (x - c.from).dot(c.dir);
        let (near, far) = if c.at_start {
            (sigma <= tol::POINT_MERGE, sigma >= len - tol::POINT_MERGE)
        } else {
            (sigma >= len - tol::POINT_MERGE, sigma <= tol::POINT_MERGE)
        };
        if near || mu <= tol::POINT_MERGE {
            return Err(PlanFail::Hard(BlendError::UnsupportedVertex { at: c.from }));
        }
        if far || mu >= c.reach - tol::POINT_MERGE {
            return Err(limit);
        }
        pts.snap(c.point, x);
        let start = obj.vertices()[s.chain.verts[c.seg]].position;
        let along = (x - start).dot(s.chain.seg_dir(obj, c.seg));
        for e in s.inserts[c.seg][c.side].iter_mut() {
            if e.1 == c.point {
                e.0 = along;
            }
        }
    }
    Ok(())
}

fn line_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let l = ab.length();
    if l <= tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    (p - a).cross(ab).length() / l
}

/// The cut points of ruling `k` at chain vertex `i` of `s`.
fn cut(s: &Strip, i: usize, k: usize) -> usize {
    let last = s.chain.verts.len() - 1;
    if s.chain.closed {
        s.stations[i].as_ref().expect("closed station")[k]
    } else if i == 0 {
        s.ends[0].ruling_cut[k].expect("checked")
    } else if i == last {
        s.ends[1].ruling_cut[k].expect("checked")
    } else {
        s.stations[i].as_ref().expect("interior station")[k]
    }
}

/// C5: every segment's rulings run forward by at least `POINT_MERGE`.
fn check_strip_lengths(obj: &Object, s: &Strip, pts: &Pts) -> Result<(), PlanFail> {
    for i in 0..s.chain.verts.len() - 1 {
        let t = s.chain.seg_dir(obj, i);
        let mut subs: Vec<Vec<usize>> = vec![(0..=s.n).map(|k| cut(s, i, k)).collect()];
        subs.extend(s.mids[i].iter().map(|m| m.pts.clone()));
        subs.push((0..=s.n).map(|k| cut(s, i + 1, k)).collect());
        for w in subs.windows(2) {
            for (k, (&ia, &ib)) in w[0].iter().zip(&w[1]).enumerate() {
                if s.pole.is_some_and(|(pk, _)| pk == k) {
                    continue;
                }
                let (a, b) = (pts.pos(ia), pts.pos(ib));
                if (b - a).dot(t) < tol::POINT_MERGE {
                    return Err(PlanFail::Size(LimitFeature::ShortEdge));
                }
            }
        }
    }
    Ok(())
}

/// The surviving piece of each shortened edge keeps the original's
/// softness and crease: with an end moved to a new point the patch creates
/// the piece afresh, which would otherwise drop them, so a revolved base's
/// soft seams came back as drawn lines. A circle or polygon claim does not
/// carry (the moved end is off the circle, and the piece is no longer a
/// whole side). Explicit metadata wins.
fn carry_shortened_meta(obj: &Object, out: &mut Out, pts: &Pts) {
    let mut carried: Vec<(usize, usize, EdgeMeta)> = Vec::new();
    for (&e, ends) in &out.shortened {
        let Some(edge) = obj.edges().get(e) else {
            continue;
        };
        if !edge.soft && !edge.crease {
            continue;
        }
        let Some((va, vb)) = obj.edge_vertex_ids(e) else {
            continue;
        };
        let end = |v: VertexId| {
            ends.iter()
                .find(|(w, _)| *w == v)
                .map(|&(_, p)| p)
                .or_else(|| pts.existing_index(v))
        };
        let (Some(a), Some(b)) = (end(va), end(vb)) else {
            continue;
        };
        carried.push((
            a,
            b,
            EdgeMeta {
                soft: edge.soft,
                crease: edge.crease,
                ..EdgeMeta::default()
            },
        ));
    }
    for (a, b, m) in carried {
        out.edge_meta.entry((a.min(b), a.max(b))).or_insert(m);
    }
}

/// C1: every shortened unpicked edge keeps its far end ahead of its new end.
fn check_shortened(obj: &Object, out: &Out, pts: &Pts) -> Result<(), PlanFail> {
    for (&e, ends) in &out.shortened {
        let Some((va, vb)) = obj.edge_vertex_ids(e) else {
            continue;
        };
        let (pa, pb) = (obj.vertices()[va].position, obj.vertices()[vb].position);
        let len = (pb - pa).length();
        let dir = (pb - pa) / len;
        // Parameter of each end after shortening (from pa).
        let mut ta = 0.0;
        let mut tb = len;
        for &(v, p) in ends {
            let s = (pts.pos(p) - pa).dot(dir);
            if v == va {
                ta = s;
            } else if v == vb {
                tb = s;
            }
        }
        if tb - ta < tol::POINT_MERGE {
            return Err(PlanFail::Size(LimitFeature::NeighborEdge));
        }
    }
    Ok(())
}

/// The re-added copy of a modified face: its loops with each replaced vertex
/// swapped for its run, its attributes and plane kept.
fn modified_face(obj: &Object, f: FaceId, out: &Out, pts: &mut Pts) -> PatchFace {
    let ring = |lid, pts: &mut Pts| -> Vec<usize> {
        let mut r: Vec<usize> = Vec::new();
        for h in obj.loop_half_edges(lid) {
            let v = obj.half_edges()[h].origin;
            if let Some(b) = out.before.get(&(f, v)) {
                r.extend(b.iter().copied());
            }
            match out.face_runs.get(&(f, v)) {
                Some(run) => r.extend(run.iter().copied()),
                None => r.push(pts.existing(obj, v)),
            }
            if let Some(a) = out.after.get(&(f, v)) {
                r.extend(a.iter().copied());
            }
        }
        let mut dedup: Vec<usize> = Vec::with_capacity(r.len());
        for i in r {
            if dedup.last() != Some(&i) {
                dedup.push(i);
            }
        }
        while dedup.len() > 1 && dedup.first() == dedup.last() {
            dedup.pop();
        }
        dedup
    };
    let face = &obj.faces()[f];
    let outer = ring(face.outer_loop, pts);
    let holes = face.inner_loops.iter().map(|&l| ring(l, pts)).collect();
    PatchFace {
        outer,
        holes,
        attrs: face.attrs(),
        plane: Some(face.plane),
    }
}

/// 2D coordinates in a face plane.
fn to2(plane: &Plane, p: Point3) -> [f64; 2] {
    let (u, v) = crate::triangulate::plane_basis(plane.normal());
    [p.to_vec().dot(u), p.to_vec().dot(v)]
}

fn inside_2d(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

fn on_boundary_2d(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    (0..n).any(|i| seg_dist_2d(p, ring[i], ring[(i + 1) % n]) <= tol::POINT_MERGE)
}

fn seg_dist_2d(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 <= 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    };
    let d = [p[0] - a[0] - ab[0] * t, p[1] - a[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

/// Whether segments ab and cd cross at a point interior to both.
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let (o1, o2, o3, o4) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    let eps = tol::POINT_MERGE * tol::POINT_MERGE;
    ((o1 > eps && o2 < -eps) || (o1 < -eps && o2 > eps))
        && ((o3 > eps && o4 < -eps) || (o3 < -eps && o4 > eps))
}

/// C2–C4 on one modified face.
fn check_modified_face(
    obj: &Object,
    f: FaceId,
    pf: &PatchFace,
    out: &Out,
    pts: &Pts,
    strips: &[Strip],
) -> Result<(), PlanFail> {
    let face = &obj.faces()[f];
    let plane = face.plane;
    let orig_outer: Vec<[f64; 2]> = obj
        .loop_positions(face.outer_loop)
        .map(|p| to2(&plane, p))
        .collect();
    let orig_holes: Vec<Vec<[f64; 2]>> = face
        .inner_loops
        .iter()
        .map(|&l| obj.loop_positions(l).map(|p| to2(&plane, p)).collect())
        .collect();
    let new_pts: BTreeSet<usize> = out
        .face_runs
        .iter()
        .chain(out.before.iter())
        .chain(out.after.iter())
        .filter(|((g, _), _)| *g == f)
        .flat_map(|(_, run)| run.iter().copied())
        .filter(|&i| matches!(pts.verts[i], PVert::New(_)))
        .collect();
    // C2: new points inside the original face.
    for &i in new_pts.iter().filter(|i| !out.fill_points.contains(i)) {
        let p = to2(&plane, pts.pos(i));
        if !(inside_2d(p, &orig_outer) || on_boundary_2d(p, &orig_outer)) {
            return Err(PlanFail::Size(LimitFeature::AdjacentFace));
        }
        if orig_holes
            .iter()
            .any(|h| inside_2d(p, h) && !on_boundary_2d(p, h))
        {
            return Err(PlanFail::Size(LimitFeature::Hole));
        }
    }
    let outer: Vec<[f64; 2]> = pf.outer.iter().map(|&i| to2(&plane, pts.pos(i))).collect();
    // C3: holes strictly inside the new outer loop.
    for (hi, &lid) in face.inner_loops.iter().enumerate() {
        let h = &orig_holes[hi];
        let crosses = (0..h.len()).any(|i| {
            let (a, b) = (h[i], h[(i + 1) % h.len()]);
            (0..outer.len()).any(|k| segments_cross(a, b, outer[k], outer[(k + 1) % outer.len()]))
        });
        if crosses || h.iter().any(|&p| !inside_2d(p, &outer)) {
            // An imprint is a hole whose other side is a coplanar face.
            let imprint = obj.loop_half_edges(lid).next().is_some_and(|he| {
                obj.half_edges()[he].twin.is_some_and(|t| {
                    let g = obj.loops()[obj.half_edges()[t].loop_id].face;
                    coplanar(obj, f, g)
                })
            });
            return Err(PlanFail::Size(if imprint {
                LimitFeature::Imprint
            } else {
                LimitFeature::Hole
            }));
        }
    }
    // C4: the new outer loop is simple (segments touching new points against
    // every segment).
    let n = pf.outer.len();
    for i in 0..n {
        let (ia, ib) = (pf.outer[i], pf.outer[(i + 1) % n]);
        if !(new_pts.contains(&ia) || new_pts.contains(&ib)) {
            continue;
        }
        for k in 0..n {
            if k == i || (k + 1) % n == i || (i + 1) % n == k {
                continue;
            }
            let (c, d) = (outer[k], outer[(k + 1) % n]);
            if segments_cross(outer[i], outer[(i + 1) % n], c, d) {
                let (ic, id) = (pf.outer[k], pf.outer[(k + 1) % n]);
                let other_rail = strips.iter().any(|s| {
                    s.ends.iter().any(|e| {
                        e.ruling_cut.contains(&Some(ic)) || e.ruling_cut.contains(&Some(id))
                    })
                });
                return Err(PlanFail::Size(if other_rail {
                    LimitFeature::OppositeBlend
                } else {
                    LimitFeature::AdjacentFace
                }));
            }
        }
    }
    Ok(())
}

/// The facets of one strip, with their attributes and seam metadata.
fn strip_faces(
    obj: &Object,
    s: &Strip,
    blend: BlendId,
    kind: BlendKind,
    pts: &mut Pts,
    out: &mut Out,
    add: &mut Vec<PatchFace>,
) -> Result<(), PlanFail> {
    let last = s.chain.verts.len() - 1;
    let first = &s.chain.edges[0];
    let (fa, fb) = (&obj.faces()[first.a], &obj.faces()[first.b]);
    let material = if fa.material == fb.material {
        fa.material
    } else {
        None
    };
    let back_material = if fa.back_material == fb.back_material {
        fa.back_material
    } else {
        None
    };
    for i in 0..last {
        // The sub-stations of segment i: its two stations and any mid-edge
        // stations between them (variable radius).
        let mut subs: Vec<(Vec<usize>, &Section, Point3)> = Vec::new();
        subs.push((
            (0..=s.n).map(|k| cut(s, i, k)).collect(),
            &s.secs[i],
            s.qs[i],
        ));
        for m in &s.mids[i] {
            subs.push((m.pts.clone(), &m.sec, m.q));
        }
        subs.push((
            (0..=s.n).map(|k| cut(s, i + 1, k)).collect(),
            &s.secs[i + 1],
            s.qs[i + 1],
        ));
        let nsub = subs.len() - 1;
        let t = s.chain.seg_dir(obj, i);
        for j in 0..nsub {
            let (c0, sec0, q0) = (&subs[j].0, subs[j].1, subs[j].2);
            let (c1, sec1, q1) = (&subs[j + 1].0, subs[j + 1].1, subs[j + 1].2);
            for k in 0..s.n {
                let mut ring: Vec<usize> = vec![c0[k]];
                // Points inserted on ruling k (c0 → c1) and on ruling k + 1
                // (c1 → c0), and whether a construction added any.
                let mut ins_k: Vec<usize> = Vec::new();
                let mut ins_k1: Vec<usize> = Vec::new();
                let mut extra = false;
                if k == 0 && nsub == 1 {
                    let mut ins = s.inserts[i][0].clone();
                    ins.sort_by(|a, b| a.0.total_cmp(&b.0));
                    ins_k = ins.iter().map(|x| x.1).collect();
                    ring.extend(ins_k.iter().copied());
                }
                ring.push(c1[k]);
                if i + 1 == last && j + 1 == nsub {
                    let e = s.ends[1].facet_extra.get(k).cloned().unwrap_or_default();
                    extra |= !e.is_empty();
                    ring.extend(e);
                }
                ring.push(c1[k + 1]);
                if k + 1 == s.n && nsub == 1 {
                    let mut ins = s.inserts[i][1].clone();
                    ins.sort_by(|a, b| b.0.total_cmp(&a.0));
                    ins_k1 = ins.iter().map(|x| x.1).collect();
                    ring.extend(ins_k1.iter().copied());
                }
                ring.push(c0[k + 1]);
                if i == 0 && j == 0 {
                    let e = s.ends[0].facet_extra.get(k).cloned().unwrap_or_default();
                    extra |= !e.is_empty();
                    ring.extend(e.into_iter().rev());
                }
                let mut dedup: Vec<usize> = Vec::with_capacity(ring.len());
                for x in ring {
                    if dedup.last() != Some(&x) && !dedup.contains(&x) {
                        dedup.push(x);
                    }
                }
                if dedup.len() < 3 {
                    return Err(PlanFail::Size(LimitFeature::ShortEdge));
                }
                // A quad between stations of different radii need not be
                // planar: split it along P_k^j–P_{k+1}^{j+1}.
                let planar = |ring: &[usize]| {
                    let ps: Vec<Point3> = ring.iter().map(|&x| pts.pos(x)).collect();
                    Plane::from_polygon(&ps).is_ok_and(|pl| {
                        ps.iter()
                            .all(|&p| pl.signed_distance(p).abs() <= tol::PLANE_DIST)
                    })
                };
                // (Points inserted on its rulings lie on them, so each stays
                // on its piece.)
                let quad = dedup.len() == 4 || (!extra && dedup.len() > 4);
                let pieces: Vec<Vec<usize>> = if quad && !planar(&dedup) {
                    out.meta(
                        c0[k],
                        c1[k + 1],
                        EdgeMeta {
                            soft: true,
                            ..EdgeMeta::default()
                        },
                    );
                    let mut first = vec![c0[k]];
                    first.extend(ins_k.iter().copied());
                    first.extend([c1[k], c1[k + 1]]);
                    let mut second = vec![c0[k], c1[k + 1]];
                    second.extend(ins_k1.iter().copied());
                    second.push(c0[k + 1]);
                    vec![first, second]
                } else {
                    vec![dedup]
                };
                for piece in pieces {
                    let face = strip_face(obj, s, i, k, piece, (sec0, q0), (sec1, q1), t, pts)?;
                    add.push(PatchFace {
                        attrs: crate::topo::FaceAttrs {
                            attrs: crate::topo::AttrSlot::NONE,
                            material,
                            back_material,
                            uv_frame: None,
                            surface: face.attrs.surface,
                            blend: Some(BlendTag {
                                id: blend,
                                kind,
                                role: BlendRole::Strip,
                            }),
                        },
                        ..face
                    });
                }
            }
            // Soft seams along interior rulings; rulings of a rim are arcs
            // of circles about its axis.
            let rim = match s.chain.edges[i].kind {
                EdgeKind::Rim { axis_point, axis } => Some((axis_point, axis)),
                _ => None,
            };
            for k in 0..=s.n {
                let (a, b) = (c0[k], c1[k]);
                if a == b {
                    continue;
                }
                let curve = rim.and_then(|(ap, ax)| {
                    let (tt, rv) = axial(pts.pos(a), ap, ax);
                    (rv.length() > tol::POINT_MERGE).then(|| CurveGeom {
                        center: ap + ax * tt,
                        radius: rv.length(),
                    })
                });
                let soft = sec0.soft.get(k).copied().unwrap_or(false);
                if soft || curve.is_some() {
                    out.meta(
                        a,
                        b,
                        EdgeMeta {
                            soft,
                            curve,
                            ..EdgeMeta::default()
                        },
                    );
                }
            }
            if j > 0 || i > 0 || s.chain.closed {
                for k in 0..s.n {
                    out.meta(
                        c0[k],
                        c0[k + 1],
                        EdgeMeta {
                            soft: true,
                            ..EdgeMeta::default()
                        },
                    );
                }
            }
        }
    }
    Ok(())
}

/// One strip facet: oriented, with its plane and verified claim (the
/// caller sets the tag and materials).
#[allow(clippy::too_many_arguments)]
fn strip_face(
    obj: &Object,
    s: &Strip,
    i: usize,
    k: usize,
    ring: Vec<usize>,
    (sec0, q0): (&Section, Point3),
    (sec1, q1): (&Section, Point3),
    t: Vec3,
    pts: &Pts,
) -> Result<PatchFace, PlanFail> {
    let positions: Vec<Point3> = ring.iter().map(|&x| pts.pos(x)).collect();
    let plane =
        Plane::from_polygon(&positions).map_err(|_| PlanFail::Hard(BlendError::BlendDegenerate))?;
    let g = positions
        .iter()
        .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
        / positions.len() as f64;
    // Orientation: a round faces away from its arc center over a convex
    // edge (toward it over a concave one), which holds for any sweep. Any
    // other section faces into the region between the edge and the section
    // polygon (the cut; the fill faces out of it), read from the polygon's
    // winding about the strip.
    let convex = s.convexity == Convexity::Convex;
    let arc_mid = if sec0.round {
        sec0.arc_center
            .zip(sec1.arc_center)
            .map(|(a, b)| (q0 + a).to_vec() * 0.5 + (q1 + b).to_vec() * 0.5)
    } else {
        None
    };
    let (side, want_positive) = match arc_mid {
        Some(c) => (plane.normal().dot(c - g), !convex),
        None => {
            let mut poly = vec![Vec3::new(0.0, 0.0, 0.0)];
            poly.extend(sec0.offsets.iter().copied());
            let wind = (0..poly.len())
                .map(|j| poly[j].cross(poly[(j + 1) % poly.len()]))
                .fold(Vec3::new(0.0, 0.0, 0.0), |a, b| a + b)
                .dot(t)
                .signum();
            let into_cut = t.cross(sec0.offsets[k + 1] - sec0.offsets[k]) * wind;
            (plane.normal().dot(into_cut), convex)
        }
    };
    if side.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return Err(PlanFail::Hard(BlendError::BlendDegenerate));
    }
    let (ring, plane) = if (side > 0.0) == want_positive {
        (ring, plane)
    } else {
        (ring.into_iter().rev().collect::<Vec<_>>(), plane.flipped())
    };
    // The facet claims the strip's surface when every point is on it
    // (map-or-drop).
    let surface = segment_claim(obj, s, i, k).filter(|claim| {
        let ps: Vec<Point3> = ring.iter().map(|&x| pts.pos(x)).collect();
        claim.holds_on(&ps, &plane, tol::PLANE_DIST)
    });
    Ok(PatchFace {
        outer: ring,
        holes: Vec::new(),
        attrs: crate::topo::FaceAttrs {
            attrs: crate::topo::AttrSlot::NONE,
            material: None,
            back_material: None,
            uv_frame: None,
            surface,
            blend: None,
        },
        plane: Some(plane),
    })
}

/// The surface segment `i` of strip `s` would claim, before verification.
fn segment_claim(obj: &Object, s: &Strip, i: usize, k: usize) -> Option<SurfaceRef> {
    let sec = &s.secs[i];
    let q = s.qs[i];
    let arc = sec.facet_arcs.get(k).copied().flatten();
    match s.variable {
        Variable::General => return None,
        Variable::Linear => {
            // A linear taper is a cone through the two end sections.
            let (c0, c1) = (q + sec.arc_center?, s.qs[i + 1] + s.secs[i + 1].arc_center?);
            let (r0, r1) = (sec.radius?, s.secs[i + 1].radius?);
            if (r1 - r0).abs() <= tol::POINT_MERGE {
                return Some(SurfaceRef::Cylinder {
                    axis_point: c0,
                    axis: s.chain.seg_dir(obj, i),
                    radius: r0,
                });
            }
            let span = (c1 - c0).length();
            let axis = if r1 > r0 {
                (c1 - c0).normalized().ok()?
            } else {
                (c0 - c1).normalized().ok()?
            };
            return Some(SurfaceRef::Cone {
                apex: c0 - (c1 - c0) * (r0 / (r1 - r0)),
                axis,
                half_angle: trig::asin((r1 - r0).abs() / span),
            });
        }
        Variable::Constant => {}
    }
    match s.chain.edges[i].kind {
        EdgeKind::Swept => None,
        EdgeKind::PlanePlane | EdgeKind::AlongAxis => {
            let (c, r) = arc?;
            Some(SurfaceRef::Cylinder {
                axis_point: q + c,
                axis: s.chain.seg_dir(obj, i),
                radius: r,
            })
        }
        EdgeKind::Rim { axis_point, axis } => {
            let radial = axial(q, axis_point, axis).1.normalized().ok()?;
            match arc {
                Some((c, r)) => {
                    let (t, rv) = axial(q + c, axis_point, axis);
                    let major = rv.dot(radial);
                    SurfaceRef::torus_or_sphere(axis_point + axis * t, axis, major.max(0.0), r)
                }
                None if sec.offsets.len() == 2 => {
                    // A chamfer: the line P_0–P_N in the meridian plane.
                    let (p0, p1) = (q + sec.offsets[0], q + *sec.offsets.last()?);
                    let (t0, r0) = axial(p0, axis_point, axis);
                    let (t1, r1) = axial(p1, axis_point, axis);
                    let (r0, r1) = (r0.dot(radial), r1.dot(radial));
                    if (r0 - r1).abs() <= tol::POINT_MERGE {
                        return Some(SurfaceRef::Cylinder {
                            axis_point,
                            axis,
                            radius: 0.5 * (r0 + r1),
                        });
                    }
                    if (t0 - t1).abs() <= tol::POINT_MERGE {
                        return None;
                    }
                    let t_apex = t0 - r0 * (t1 - t0) / (r1 - r0);
                    let t_mid = 0.5 * (t0 + t1);
                    let dir = if t_mid >= t_apex { axis } else { -axis };
                    Some(SurfaceRef::Cone {
                        apex: axis_point + axis * t_apex,
                        axis: dir,
                        half_angle: trig::atan(((r1 - r0) / (t1 - t0)).abs()),
                    })
                }
                None => None,
            }
        }
    }
}

/// Rim limits (WP06 §2.2): the arc center may not cross the axis; a ball
/// center on the axis makes a hemisphere whose cap collapses onto one pole
/// point and is consumed whole.
fn rim_checks(obj: &Object, s: &mut Strip, pts: &mut Pts, out: &mut Out) -> Result<(), PlanFail> {
    let mut pole_side: Option<usize> = None;
    for i in 0..s.chain.edges.len() {
        let EdgeKind::Rim { axis_point, axis } = s.chain.edges[i].kind else {
            continue;
        };
        let sec = &s.secs[i];
        let q = s.qs[i];
        let radial = axial(q, axis_point, axis)
            .1
            .normalized()
            .map_err(|_| BlendError::BlendDegenerate)?;
        // A band rail must stay on the band's first row.
        let info = &s.chain.edges[i];
        for (side, seg) in [(0usize, sec.seg_a), (1, sec.seg_b)] {
            let f = if side == 0 { info.a } else { info.b };
            if obj.faces()[f].surface.is_some() && seg > 0 {
                return Err(PlanFail::Size(LimitFeature::FacetWidth));
            }
        }
        if let Some(c) = sec.arc_center {
            let major = axial(q + c, axis_point, axis).1.dot(radial);
            if major < -tol::POINT_MERGE {
                return Err(PlanFail::Size(LimitFeature::RimRadius));
            }
            let side = if obj.faces()[info.a].surface.is_none() {
                0
            } else {
                1
            };
            // The cap-side ruling reaches the axis: exactly, or so nearly
            // that the cap left behind encloses less than the area cutoff
            // (a point, for every purpose), which is the same solid.
            let collapsed = major.abs() <= tol::POINT_MERGE || {
                let ruling = if side == 0 { 0 } else { s.n };
                let ring: Vec<Point3> = (0..s.chain.verts.len() - 1)
                    .map(|j| s.qs[j] + s.secs[j].offsets[ruling])
                    .collect();
                s.chain.closed && crate::serialize::outer_ring_area_plane(&ring).is_err()
            };
            if collapsed {
                pole_side = Some(side);
            }
        }
    }
    let Some(side) = pole_side else {
        return Ok(());
    };
    let info = &s.chain.edges[0];
    let cap = if side == 0 { info.a } else { info.b };
    let face = &obj.faces()[cap];
    let loop_verts: BTreeSet<VertexId> = obj
        .loop_half_edges(face.outer_loop)
        .map(|h| obj.half_edges()[h].origin)
        .collect();
    let chain_verts: BTreeSet<VertexId> = s.chain.verts.iter().copied().collect();
    if !s.chain.closed || !face.inner_loops.is_empty() || loop_verts != chain_verts {
        return Err(PlanFail::Size(LimitFeature::RimRadius));
    }
    let EdgeKind::Rim { axis_point, axis } = info.kind else {
        return Err(PlanFail::Hard(BlendError::BlendDegenerate));
    };
    let ruling = if side == 0 { 0 } else { s.n };
    let (t, _) = axial(s.qs[0] + s.secs[0].offsets[ruling], axis_point, axis);
    let pole = pts.add(axis_point + axis * t);
    s.pole = Some((ruling, pole));
    out.removed.insert(cap);
    Ok(())
}

/// Seam crossing (WP06 §3): when the side-B rail of a straight edge along
/// a faceted cylinder lands past the first facet, the facets before it are
/// consumed and the one it lands on is trimmed.
fn seam_crossing(obj: &Object, s: &mut Strip, out: &mut Out) -> Result<(), PlanFail> {
    let info = &s.chain.edges[0];
    if info.kind != EdgeKind::AlongAxis {
        return Ok(());
    }
    let seg_a = s.secs.iter().map(|x| x.seg_a).max().unwrap_or(0);
    let seg_b = s.secs.iter().map(|x| x.seg_b).max().unwrap_or(0);
    if s.secs.iter().any(|x| x.seg_b != seg_b) || seg_a > 0 {
        return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
    }
    if seg_b == 0 {
        return Ok(());
    }
    if s.chain.edges.len() != 1 {
        return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
    }
    let faces = facet_faces(obj, info, 1, s.chain.verts[0])?;
    if faces.len() <= seg_b {
        return Err(PlanFail::Size(LimitFeature::FacetWidth));
    }
    for &f in &faces[..seg_b] {
        out.removed.insert(f);
    }
    s.trimmed_b = Some(faces[seg_b]);
    Ok(())
}

/// The faces vertex constructions made, oriented by the strip rule about
/// their vertex, tagged as corners, soft along every edge.
fn corner_faces(
    obj: &Object,
    strips: &[Strip],
    blend: BlendId,
    kind: BlendKind,
    pts: &mut Pts,
    out: &mut Out,
    add: &mut Vec<PatchFace>,
) -> Result<(), PlanFail> {
    let faces = std::mem::take(&mut out.corner_faces);
    let (material, back_material) = strips
        .first()
        .map(|s| {
            let e = &s.chain.edges[0];
            let (fa, fb) = (&obj.faces()[e.a], &obj.faces()[e.b]);
            (
                if fa.material == fb.material {
                    fa.material
                } else {
                    None
                },
                if fa.back_material == fb.back_material {
                    fa.back_material
                } else {
                    None
                },
            )
        })
        .unwrap_or((None, None));
    for cf in faces {
        let positions: Vec<Point3> = cf.ring.iter().map(|&x| pts.pos(x)).collect();
        let plane = Plane::from_polygon(&positions)
            .map_err(|_| PlanFail::Hard(BlendError::BlendDegenerate))?;
        let g = positions
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
            / positions.len() as f64;
        let (side, want_positive) = match cf.claim {
            Some(SurfaceRef::Sphere { center, .. }) => {
                (plane.normal().dot(center.to_vec() - g), !cf.convex)
            }
            _ => (plane.normal().dot(cf.at.to_vec() - g), cf.convex),
        };
        let (ring, plane) = if (side > 0.0) == want_positive {
            (cf.ring, plane)
        } else {
            (
                cf.ring.into_iter().rev().collect::<Vec<_>>(),
                plane.flipped(),
            )
        };
        let surface = cf.claim.filter(|c| {
            let ps: Vec<Point3> = ring.iter().map(|&x| pts.pos(x)).collect();
            c.holds_on(&ps, &plane, tol::PLANE_DIST)
        });
        for &(a, b) in &cf.soft {
            out.meta(
                a,
                b,
                EdgeMeta {
                    soft: true,
                    ..EdgeMeta::default()
                },
            );
        }
        add.push(PatchFace {
            outer: ring,
            holes: Vec::new(),
            attrs: crate::topo::FaceAttrs {
                attrs: crate::topo::AttrSlot::NONE,
                material,
                back_material,
                uv_frame: None,
                surface,
                blend: Some(BlendTag {
                    id: blend,
                    kind: cf.kind.unwrap_or(kind),
                    role: cf.role,
                }),
            },
            plane: Some(plane),
        });
    }
    Ok(())
}

/// The frame of a bullnose (WP07 §2): the face `F` it consumes, its two
/// side faces, its two edges, and the radius (half the face's width).
pub(crate) struct Bullnose {
    pub f: FaceId,
    pub sides: [FaceId; 2],
    pub edges: [EdgeId; 2],
    pub r: f64,
}

/// Checks the bullnose preconditions and returns its frame.
pub(crate) fn bullnose_frame(
    obj: &Object,
    infos: &[super::classify::EdgeInfo],
) -> Result<Bullnose, PlanFail> {
    let bad = PlanFail::Hard(BlendError::BullnoseNeedsParallelSides);
    let [i1, i2] = infos else {
        return Err(bad);
    };
    let common = [i1.a, i1.b]
        .into_iter()
        .find(|f| *f == i2.a || *f == i2.b)
        .ok_or(bad)?;
    let other = |i: &super::classify::EdgeInfo| if i.a == common { i.b } else { i.a };
    let (s1, s2) = (other(i1), other(i2));
    let face = &obj.faces()[common];
    let (n_f, n1, n2) = (
        face.plane.normal(),
        obj.faces()[s1].plane.normal(),
        obj.faces()[s2].plane.normal(),
    );
    let flat = |f: FaceId| obj.faces()[f].surface.is_none();
    if !(flat(common) && flat(s1) && flat(s2))
        || !face.inner_loops.is_empty()
        || i1.d_a.dot(i2.d_a).abs() < 1.0 - tol::NORMAL_DIRECTION
        || n1.dot(n2) > -1.0 + tol::NORMAL_DIRECTION
        || n1.dot(n_f).abs() > tol::NORMAL_DIRECTION
        || n2.dot(n_f).abs() > tol::NORMAL_DIRECTION
    {
        return Err(bad);
    }
    // F's outer loop: e1, one end edge, e2, the other end edge.
    let loop_edges: Vec<EdgeId> = obj
        .loop_half_edges(face.outer_loop)
        .map(|h| obj.half_edges()[h].edge)
        .collect();
    if loop_edges.len() != 4 {
        return Err(bad);
    }
    let p1 = loop_edges.iter().position(|&e| e == i1.edge).ok_or(bad)?;
    if loop_edges[(p1 + 2) % 4] != i2.edge {
        return Err(bad);
    }
    let (a, b) = obj.edge_endpoints(i1.edge).ok_or(bad)?;
    let (c, _) = obj.edge_endpoints(i2.edge).ok_or(bad)?;
    let d = (b - a).normalized().map_err(|_| bad)?;
    let w = ((c - a) - d * (c - a).dot(d)).length();
    Ok(Bullnose {
        f: common,
        sides: [s1, s2],
        edges: [i1.edge, i2.edge],
        r: w / 2.0,
    })
}

/// Plans a bullnose: `F` is consumed by a half-round of radius `w/2`
/// between its two sides; each end face takes the section's run-out.
fn plan_bullnose(
    obj: &Object,
    infos: &[super::classify::EdgeInfo],
    n: usize,
) -> Result<BlendPlan, PlanFail> {
    let bn = bullnose_frame(obj, infos)?;
    let unsupported = |at: Point3| PlanFail::Hard(BlendError::UnsupportedVertex { at });
    let n_f = obj.faces()[bn.f].plane.normal();
    let n_s1 = obj.faces()[bn.sides[0]].plane.normal();
    let r = bn.r;
    let (va, vb) = obj
        .edge_vertex_ids(bn.edges[0])
        .ok_or(BlendError::BlendDegenerate)?;
    let (pa, pb) = (obj.vertices()[va].position, obj.vertices()[vb].position);
    let t = (pb - pa)
        .normalized()
        .map_err(|_| BlendError::BlendDegenerate)?;
    let (ea, eb) = obj
        .edge_vertex_ids(bn.edges[1])
        .ok_or(BlendError::BlendDegenerate)?;
    let mut pts = Pts::new();
    let mut out = Out::default();
    // The section at e1's vertex u: the half circle from S1 over F to S2.
    let section = |u: Point3| -> (Point3, Vec<Point3>) {
        let p2 = obj.vertices()[ea].position;
        let foot = p2 + t * (u - p2).dot(t);
        let o = u + (foot - u) * 0.5;
        let c = o - n_f * r;
        let ps = (0..=n)
            .map(|k| {
                let a = std::f64::consts::PI * k as f64 / n as f64;
                c + (n_s1 * trig::cos(a) + n_f * trig::sin(a)) * r
            })
            .collect();
        (c, ps)
    };
    let mut cuts: Vec<Vec<usize>> = Vec::with_capacity(2);
    let mut centers: Vec<Point3> = Vec::with_capacity(2);
    for (u, toward) in [(va, -t), (vb, t)] {
        let up = obj.vertices()[u].position;
        // The end face across F's end edge at u, and e2's vertex on it.
        let end_edge = obj
            .loop_half_edges(obj.faces()[bn.f].outer_loop)
            .map(|h| obj.half_edges()[h].edge)
            .find(|&e| {
                e != bn.edges[0]
                    && e != bn.edges[1]
                    && obj
                        .edge_vertex_ids(e)
                        .is_some_and(|(x, y)| x == u || y == u)
            })
            .ok_or(unsupported(up))?;
        let (x, y) = obj.edge_vertex_ids(end_edge).ok_or(unsupported(up))?;
        let u2 = if x == u { y } else { x };
        if u2 != ea && u2 != eb {
            return Err(unsupported(up));
        }
        let (fx, fy) = obj.edge_faces(end_edge).ok_or(unsupported(up))?;
        let e_face = if fx == bn.f { fy } else { fx };
        let plane_e = obj.faces()[e_face].plane;
        let (c, ps) = section(up);
        let mut e_pts: Vec<usize> = Vec::with_capacity(n + 1);
        for p in ps {
            let dn = toward.dot(plane_e.normal());
            if dn.abs() <= tol::NORMALIZE_MIN_LENGTH {
                return Err(unsupported(up));
            }
            e_pts.push(pts.add(p + toward * (-plane_e.signed_distance(p) / dn)));
        }
        // End face: the run [u, u2] becomes E_0..E_N in its loop order.
        let ctx_prev = |f: FaceId, v: VertexId| {
            obj.face_loops(f).into_iter().find_map(|l| {
                obj.loop_half_edges(l).find_map(|h| {
                    let he = obj.half_edges()[h];
                    (obj.half_edges()[he.next].origin == v).then_some(he.origin)
                })
            })
        };
        if ctx_prev(e_face, u2) == Some(u) {
            out.face_runs.insert((e_face, u), e_pts.clone());
            out.face_runs.insert((e_face, u2), Vec::new());
        } else {
            let mut rev = e_pts.clone();
            rev.reverse();
            out.face_runs.insert((e_face, u2), rev);
            out.face_runs.insert((e_face, u), Vec::new());
        }
        out.face_runs.insert((bn.sides[0], u), vec![e_pts[0]]);
        out.face_runs.insert((bn.sides[1], u2), vec![e_pts[n]]);
        let ve = obj.vertex_edges();
        let between = |v: VertexId, f: FaceId, g: FaceId| {
            ve.get(&v).into_iter().flatten().copied().find(|&e| {
                obj.edge_faces(e)
                    .is_some_and(|(p, q)| (p == f && q == g) || (p == g && q == f))
            })
        };
        if let Some(e) = between(u, bn.sides[0], e_face) {
            out.shortened.entry(e).or_default().push((u, e_pts[0]));
        }
        if let Some(e) = between(u2, bn.sides[1], e_face) {
            out.shortened.entry(e).or_default().push((u2, e_pts[n]));
        }
        if toward.dot(plane_e.normal()).abs() >= 1.0 - tol::NORMAL_DIRECTION {
            for k in 0..n {
                out.meta(
                    e_pts[k],
                    e_pts[k + 1],
                    EdgeMeta {
                        curve: Some(CurveGeom {
                            center: c,
                            radius: r,
                        }),
                        ..EdgeMeta::default()
                    },
                );
            }
        }
        cuts.push(e_pts);
        centers.push(c);
    }
    out.removed.insert(bn.f);
    check_shortened(obj, &out, &pts)?;
    let mut modified: BTreeSet<FaceId> = BTreeSet::new();
    for &(f, _) in out.face_runs.keys() {
        if !out.removed.contains(&f) {
            modified.insert(f);
        }
    }
    let mut add: Vec<PatchFace> = Vec::new();
    for &f in &modified {
        add.push(modified_face(obj, f, &out, &mut pts));
    }
    for (pf, &f) in add.iter().zip(&modified) {
        check_modified_face(obj, f, pf, &out, &pts, &[])?;
    }
    let blend = obj.new_blend_id();
    let first_new = add.len();
    let fa = &obj.faces()[bn.sides[0]];
    let fb = &obj.faces()[bn.sides[1]];
    let material = (fa.material == fb.material)
        .then_some(fa.material)
        .flatten();
    let back_material = (fa.back_material == fb.back_material)
        .then_some(fa.back_material)
        .flatten();
    let c_mid = centers[0].to_vec() * 0.5 + centers[1].to_vec() * 0.5;
    let claim = SurfaceRef::Cylinder {
        axis_point: centers[0],
        axis: t,
        radius: r,
    };
    for k in 0..n {
        let ring = vec![cuts[0][k], cuts[1][k], cuts[1][k + 1], cuts[0][k + 1]];
        let positions: Vec<Point3> = ring.iter().map(|&x| pts.pos(x)).collect();
        let plane = Plane::from_polygon(&positions)
            .map_err(|_| PlanFail::Hard(BlendError::BlendDegenerate))?;
        let g = positions
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
            / 4.0;
        let (ring, plane) = if plane.normal().dot(c_mid - g) < 0.0 {
            (ring, plane)
        } else {
            (ring.into_iter().rev().collect::<Vec<_>>(), plane.flipped())
        };
        let surface = claim
            .holds_on(&positions, &plane, tol::PLANE_DIST)
            .then_some(claim);
        add.push(PatchFace {
            outer: ring,
            holes: Vec::new(),
            attrs: crate::topo::FaceAttrs {
                attrs: crate::topo::AttrSlot::NONE,
                material,
                back_material,
                uv_frame: None,
                surface,
                blend: Some(BlendTag {
                    id: blend,
                    kind: BlendKind::Fillet,
                    role: BlendRole::Strip,
                }),
            },
            plane: Some(plane),
        });
        if k > 0 {
            out.meta(
                cuts[0][k],
                cuts[1][k],
                EdgeMeta {
                    soft: true,
                    ..EdgeMeta::default()
                },
            );
        }
    }
    let new_face_range = first_new..add.len();
    let replaced: Vec<FaceId> = modified.union(&out.removed).copied().collect();
    carry_shortened_meta(obj, &mut out, &pts);
    let _ = (vb, eb);
    Ok(BlendPlan {
        patch: PolyPatch {
            verts: pts.verts,
            remove: replaced.clone(),
            add,
            edge_meta: out.edge_meta,
            expect_watertight: true,
        },
        replaced,
        new_face_range,
        blend,
        fallback: None,
    })
}

/// A radius law along a chain (WP07 §5.2): the profile radius at both ends
/// unless a handle replaces it, interior handles in between, monotone cubic
/// Hermite in arc length (Fritsch–Carlson); `linear` when there are no
/// interior handles on one straight chain.
pub(crate) struct RadiusLaw {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ms: Vec<f64>,
    pub linear: bool,
}

impl RadiusLaw {
    pub(crate) fn at(&self, x: f64) -> f64 {
        let n = self.xs.len();
        let i = (0..n - 1).find(|&i| x <= self.xs[i + 1]).unwrap_or(n - 2);
        let (x0, x1) = (self.xs[i], self.xs[i + 1]);
        let h = x1 - x0;
        if self.linear || h <= 0.0 {
            let t = if h > 0.0 { (x - x0) / h } else { 0.0 };
            return self.ys[i] + (self.ys[i + 1] - self.ys[i]) * t;
        }
        let t = ((x - x0) / h).clamp(0.0, 1.0);
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[i]
            + (t3 - 2.0 * t2 + t) * h * self.ms[i]
            + (-2.0 * t3 + 3.0 * t2) * self.ys[i + 1]
            + (t3 - t2) * h * self.ms[i + 1]
    }
    /// The interior points (handles) strictly inside the chain.
    fn knots(&self) -> &[f64] {
        &self.xs
    }
}

/// The spec's radius law over a chain of length `len`, or `None` without
/// handles.
fn radius_law(spec: &BlendSpec, chain: &Chain, len: f64) -> Result<Option<RadiusLaw>, PlanFail> {
    if spec.handles.is_empty() {
        return Ok(None);
    }
    let out_of_range = PlanFail::Hard(BlendError::StopOutOfRange);
    let BlendProfile::Round { radius, .. } = spec.profile else {
        return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
    };
    if chain.closed || chain.edges.iter().any(|e| e.kind != EdgeKind::PlanePlane) {
        return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
    }
    let mut hs: Vec<(f64, f64)> = spec.handles.iter().map(|h| (h.at, h.radius)).collect();
    hs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut r0, mut r1) = (radius, radius);
    let mut inner: Vec<(f64, f64)> = Vec::new();
    // The chain the tool measured crossed the wasm boundary in single
    // precision, so an end handle can land a few ten-millionths past or
    // short of the end: within `end_tol` of an end it is that end. Of two
    // handles in one end's window the nearer wins (an exact end beats one
    // a hair inside it), whatever their order.
    let end_tol = tol::POINT_MERGE.max(len * tol::BLEND_HANDLE_END_SLACK);
    let (mut d0, mut d1) = (f64::INFINITY, f64::INFINITY);
    for (at, r) in hs {
        if !(at.is_finite() && r.is_finite()) || r <= tol::POINT_MERGE {
            return Err(out_of_range);
        }
        if at.abs() <= end_tol {
            if at.abs() < d0 {
                d0 = at.abs();
                r0 = r;
            }
        } else if (at - len).abs() <= end_tol {
            if (at - len).abs() < d1 {
                d1 = (at - len).abs();
                r1 = r;
            }
        } else if at > 0.0 && at < len {
            inner.push((at, r));
        } else {
            return Err(out_of_range);
        }
    }
    let mut pts = vec![(0.0, r0)];
    pts.extend(inner);
    pts.push((len, r1));
    let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
    let n = xs.len();
    // Fritsch–Carlson tangents.
    let d: Vec<f64> = (0..n - 1)
        .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
        .collect();
    let mut ms = vec![0.0; n];
    ms[0] = d[0];
    ms[n - 1] = d[n - 2];
    for i in 1..n - 1 {
        ms[i] = if d[i - 1] * d[i] <= 0.0 {
            0.0
        } else {
            (d[i - 1] + d[i]) / 2.0
        };
    }
    for i in 0..n - 1 {
        if d[i] == 0.0 {
            ms[i] = 0.0;
            ms[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (ms[i] / d[i], ms[i + 1] / d[i]);
        let h = a * a + b * b;
        if h > 9.0 {
            let tau = 3.0 / h.sqrt();
            ms[i] = tau * a * d[i];
            ms[i + 1] = tau * b * d[i];
        }
    }
    Ok(Some(RadiusLaw {
        linear: n == 2,
        xs,
        ys,
        ms,
    }))
}

/// Mid-edge stations for a general radius law: every handle and 8 equal
/// subdivisions of each span between consecutive law points. Side faces
/// get the new rail points in loop order.
#[allow(clippy::too_many_arguments)]
fn mid_stations(
    obj: &Object,
    vertex_edges: &BTreeMap<VertexId, Vec<EdgeId>>,
    s: &mut Strip,
    arc: &[f64],
    law: &RadiusLaw,
    profile_at: &dyn Fn(f64) -> BlendProfile,
    pts: &mut Pts,
    out: &mut Out,
) -> Result<(), PlanFail> {
    let knots = law.knots().to_vec();
    let mut lens: Vec<f64> = Vec::new();
    for w in knots.windows(2) {
        for j in 0..8 {
            lens.push(w[0] + (w[1] - w[0]) * j as f64 / 8.0);
        }
    }
    lens.push(*knots.last().expect("two knots"));
    for i in 0..s.chain.edges.len() {
        let (a, b) = (arc[i], arc[i + 1]);
        let v = s.chain.verts[i];
        let t = s.chain.seg_dir(obj, i);
        let q_i = s.qs[i];
        for &x in lens
            .iter()
            .filter(|&&x| x > a + tol::POINT_MERGE && x < b - tol::POINT_MERGE)
        {
            let sec = station_section(obj, vertex_edges, &s.chain.edges[i], v, &profile_at(x))?;
            let q = q_i + t * (x - a);
            let p: Vec<usize> = sec.offsets.iter().map(|&o| pts.add(q + o)).collect();
            s.mids[i].push(Mid { q, sec, pts: p });
        }
        if s.mids[i].is_empty() {
            continue;
        }
        if s.inserts[i].iter().any(|x| !x.is_empty()) {
            return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
        }
        // Side A runs v_i → v_{i+1}; side B the other way.
        let info = &s.chain.edges[i];
        let a_pts: Vec<usize> = s.mids[i].iter().map(|m| m.pts[0]).collect();
        let b_pts: Vec<usize> = s.mids[i].iter().rev().map(|m| m.pts[s.n]).collect();
        out.after.entry((info.a, v)).or_default().extend(a_pts);
        let mut before = b_pts;
        before.extend(out.before.remove(&(info.b, v)).unwrap_or_default());
        out.before.insert((info.b, v), before);
    }
    Ok(())
}

/// Stopped ends (WP07 §5.1): the strip starts `stops.start` from the chain
/// start (ends `stops.end` from its end), the edge stays sharp up to `R`,
/// and a fan of triangles from `R` closes the gap.
fn stop_ends(
    obj: &Object,
    s: &mut Strip,
    stops: super::BlendStops,
    pts: &mut Pts,
    out: &mut Out,
) -> Result<(), PlanFail> {
    let out_of_range = PlanFail::Hard(BlendError::StopOutOfRange);
    if s.chain.closed {
        return Err(out_of_range);
    }
    let last = s.chain.verts.len() - 1;
    for (which, dist) in [(0usize, stops.start), (1, stops.end)] {
        if !dist.is_finite() || dist < 0.0 {
            return Err(out_of_range);
        }
        if dist == 0.0 {
            continue;
        }
        let (seg, v_end, v_next) = if which == 0 {
            (0, s.chain.verts[0], s.chain.verts[1])
        } else {
            (last - 1, s.chain.verts[last], s.chain.verts[last - 1])
        };
        let info = s.chain.edges[seg].clone();
        if info.kind != EdgeKind::PlanePlane {
            return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
        }
        let p_end = obj.vertices()[v_end].position;
        let len = (obj.vertices()[v_next].position - p_end).length();
        let inward = (obj.vertices()[v_next].position - p_end) / len;
        let idx = s.end_index(which);
        let sec = s.secs[idx].clone();
        let lr = sec.offsets[0].length().max(sec.offsets[s.n].length());
        if dist >= len - tol::POINT_MERGE || dist - lr < tol::POINT_MERGE {
            return Err(out_of_range);
        }
        let p = p_end + inward * dist;
        let r_pt = pts.add(p_end + inward * (dist - lr));
        let station: Vec<usize> = sec.offsets.iter().map(|&o| pts.add(p + o)).collect();
        s.qs[idx] = p;
        s.stop[which] = Some(r_pt);
        s.ends[which] = StripEnd {
            ruling_cut: station.iter().copied().map(Some).collect(),
            facet_extra: vec![Vec::new(); s.n],
        };
        // Side A's half-edge runs along the chain, side B's against it.
        if which == 0 {
            out.after
                .entry((info.a, v_end))
                .or_default()
                .splice(0..0, [r_pt, station[0]]);
            out.before
                .entry((info.b, v_end))
                .or_default()
                .extend([station[s.n], r_pt]);
        } else {
            out.before
                .entry((info.a, v_end))
                .or_default()
                .extend([station[0], r_pt]);
            out.after
                .entry((info.b, v_end))
                .or_default()
                .splice(0..0, [r_pt, station[s.n]]);
        }
        let soft: Vec<(usize, usize)> = (1..s.n).map(|k| (r_pt, station[k])).collect();
        for k in 0..s.n {
            out.corner_faces.push(CornerFace {
                ring: vec![r_pt, station[k], station[k + 1]],
                claim: None,
                kind: None,
                role: BlendRole::Runout,
                at: p,
                convex: s.convexity == Convexity::Convex,
                soft: soft.clone(),
            });
        }
    }
    Ok(())
}

/// A linear taper's balls envelope a cone; each ball touches it along its
/// characteristic circle, not along the circle square to the edge. Moves
/// every section's interior points onto that circle (the rails, the feet on
/// the two faces, already lie on it), so strip facets span cone generators.
fn taper_onto_cone(
    obj: &Object,
    chain: &Chain,
    secs: &mut [Section],
) -> Result<Option<Point3>, PlanFail> {
    let m = chain.verts.len();
    let q = |i: usize| obj.vertices()[chain.verts[i]].position;
    let center = |i: usize, s: &Section| s.arc_center.map(|c| q(i) + c);
    let (Some(c0), Some(c1)) = (center(0, &secs[0]), center(m - 1, &secs[m - 1])) else {
        return Ok(None);
    };
    let (Some(r0), Some(r1)) = (secs[0].radius, secs[m - 1].radius) else {
        return Ok(None);
    };
    let dc = c1 - c0;
    let span = dc.length();
    if span <= tol::POINT_MERGE || (r1 - r0).abs() <= tol::POINT_MERGE {
        return Ok(None);
    }
    let axis = dc / span;
    let dr = (r1 - r0) / span;
    for (i, sec) in secs.iter_mut().enumerate() {
        let (Some(ac), Some(r)) = (sec.arc_center, sec.radius) else {
            continue;
        };
        let c = q(i) + ac;
        // The characteristic circle: (p − c)·axis = −r·dr, |p − c| = r.
        let cc = c - axis * (r * dr);
        let rho = (r * r - (r * dr).powi(2)).max(0.0).sqrt();
        let n = sec.offsets.len() - 1;
        let p0 = q(i) + sec.offsets[0];
        let pn = q(i) + sec.offsets[n];
        let u = (p0 - cc)
            .normalized()
            .map_err(|_| BlendError::BlendDegenerate)?;
        let v = (pn - cc) / rho;
        let cosphi = u.dot(v).clamp(-1.0, 1.0);
        let phi = trig::acos(cosphi);
        let w = (v - u * cosphi)
            .normalized()
            .map_err(|_| BlendError::BlendDegenerate)?;
        for k in 1..n {
            let a = phi * k as f64 / n as f64;
            let p = cc + (u * trig::cos(a) + w * trig::sin(a)) * rho;
            sec.offsets[k] = p - q(i);
        }
    }
    Ok(Some(c0 - dc * (r0 / (r1 - r0))))
}
