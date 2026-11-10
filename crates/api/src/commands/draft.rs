//! Drafting-engine commands (unified drafting §4, §7): the plane is the
//! drafting context, and everything drawn on it — a solid's face edges,
//! the drawing kept on those faces, the free sketches of the void — is
//! one arrangement that [`kernel::Document::draft`] lifts, edits, and
//! lowers back as ONE undo step.
//!
//! Two surfaces live here:
//!
//! - Face-target drawing for `hew.sketch.draw_*` ([`draw_on_face`]): the
//!   same geometry the ground path draws, through the draft engine, so a
//!   loose line on a face, an open path touching no edge, and a shape
//!   overhanging the face edge are all accepted (they used to refuse). The
//!   result keeps the `object_id` and the face tokens and adds `units`, the
//!   plane units the draw touched.
//! - The unit verbs `hew.sketch.units` (query) and
//!   `move`/`rotate`/`scale`/`copy`/`delete`, each taking a `plane` (any
//!   plane spec the draw verbs accept, or a bare sketch id) and unit keys:
//!   `isl_<hex16>` (an island: every drawing edge in it), `edg_<hex16>`
//!   (one edge), `crv_<hex16>` (every facet of one circle), `rgn_<hex16>`
//!   (a region's ring edges that are drawing edges). Keys come from
//!   `hew.sketch.units` and are geometry-derived (`kernel::unit_key`):
//!   stable until the geometry they hash changes.
//!
//! A solid's own edges are never drawing edges; moving or deleting one is
//! topology editing, which the kernel refuses
//! (`draft_boundary_edge_touched`).
//!
//! A **detached** sketch (detached shapes D5, D10) is in no plane's
//! arrangement: it is its own drafting context. A `{sketch: id}` plane spec
//! naming one lists and edits exactly that sketch, with no object and no
//! other drawing taking part; a plane or face spec never sees it. A draw
//! with `detached: true` ([`draw_detached`]) makes one new detached sketch.

use super::{Closed, CmdError, Ctx, Handler};
use crate::geom;
use crate::ids;
use crate::locate::{self, FaceRef, PlaneTarget};
use crate::refusal::Refusal;
use kernel::{
    ComponentId, CurveGeom, DocumentError, DraftEdge, DraftStore, EntityRef, ObjectId, Plane,
    PlaneUnits, Point3, Sketch, SketchCurveKind, SketchEdgeId, SketchError, SketchId, Transform,
    Vec3,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.sketch.units" => units,
        "hew.sketch.move" => move_units,
        "hew.sketch.rotate" => rotate_units,
        "hew.sketch.scale" => scale_units,
        "hew.sketch.copy" => copy_units,
        "hew.sketch.delete" => delete_units,
        _ => return None,
    })
}

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

// ------------------------------------------------------------- context

/// A resolved drafting context: the plane, the frame it lives in (`None`
/// for the world, a component for a definition member), and every object
/// with a drawable face on it — or, for a detached sketch, that sketch
/// alone (`detached`, with no objects).
pub(super) struct DraftCtx {
    pub plane: Plane,
    pub frame: Option<ComponentId>,
    pub objects: Vec<ObjectId>,
    pub detached: Option<SketchId>,
}

/// `addressed` is the object whose face the caller named: it always takes
/// part, even where the plane's drawable set leaves it out (a group member,
/// which the UI only draws on once the group is entered).
pub(super) fn context_of(
    ctx: &Ctx,
    plane: Plane,
    frame: Option<ComponentId>,
    addressed: Option<ObjectId>,
) -> DraftCtx {
    let mut objects = ctx.doc.drawable_objects_on_plane(plane, frame);
    if let Some(o) = addressed
        && !objects.contains(&o)
        && ctx.doc.object(o).is_some()
    {
        objects.push(o);
    }
    DraftCtx {
        plane,
        frame,
        objects,
        detached: None,
    }
}

/// A resolved face's supporting plane.
pub(super) fn face_plane(ctx: &Ctx, face: FaceRef) -> Result<Plane, CmdError> {
    let object = ctx.doc.object(face.object).ok_or_else(|| {
        CmdError::Internal("object vanished between face resolution and use".into())
    })?;
    let f = object
        .faces()
        .get(face.face)
        .ok_or_else(|| CmdError::Internal("face vanished between resolution and use".into()))?;
    Ok(f.plane)
}

/// Resolves a plane spec (`{ground}`, `{origin,normal}`, `{face}`,
/// `{sketch}`, or a bare sketch id string) to its drafting context. A
/// detached sketch's context is that sketch alone.
pub(super) fn resolve_context(ctx: &Ctx, spec: &Value) -> Result<DraftCtx, CmdError> {
    let as_sketch;
    let spec = match spec.as_str() {
        Some(s) => {
            as_sketch = json!({ "sketch": s });
            &as_sketch
        }
        None => spec,
    };
    match locate::resolve_plane(ctx, spec)? {
        PlaneTarget::NewPlane(plane) => Ok(context_of(ctx, plane, None, None)),
        PlaneTarget::Sketch(id) => {
            let sk = ctx
                .doc
                .sketch(id)
                .ok_or_else(|| CmdError::Internal("sketch vanished after resolve".into()))?;
            let plane = sk.plane();
            if sk.detached() {
                return Ok(DraftCtx {
                    plane,
                    frame: ctx.doc.sketch_owner_component(id),
                    objects: Vec::new(),
                    detached: Some(id),
                });
            }
            let frame = ctx.doc.sketch_owner_component(id).or_else(|| {
                ctx.doc
                    .sketch_owner_object(id)
                    .and_then(|o| ctx.doc.object_owner_component(o))
            });
            Ok(context_of(ctx, plane, frame, None))
        }
        PlaneTarget::Face(face) => {
            let plane = face_plane(ctx, face)?;
            let frame = ctx.doc.object_owner_component(face.object);
            Ok(context_of(ctx, plane, frame, Some(face.object)))
        }
    }
}

/// The context's drawing as units: the plane's arrangement, or a detached
/// sketch alone. A detached sketch an edit emptied (and so deleted) lists
/// nothing.
pub(super) fn units_of(ctx: &Ctx, dc: &DraftCtx) -> Result<PlaneUnits, CmdError> {
    match dc.detached {
        Some(s) if ctx.doc.sketch(s).is_none() => Ok(PlaneUnits::default()),
        Some(s) => Ok(ctx.doc.detached_sketch_units(s)?),
        None => {
            // An edit may have consumed an object of the context.
            let objects: Vec<ObjectId> = dc
                .objects
                .iter()
                .copied()
                .filter(|&o| ctx.doc.object(o).is_some())
                .collect();
            Ok(ctx.doc.plane_units(dc.plane, dc.frame, &objects)?)
        }
    }
}

/// Opens the draft for a context: the plane's arrangement, or a detached
/// sketch's own context.
fn begin(ctx: &mut Ctx, dc: &DraftCtx, label: &str) -> Result<(), CmdError> {
    match dc.detached {
        Some(s) => ctx.doc.begin_draft_detached(s, label)?,
        None => ctx
            .doc
            .begin_draft(dc.plane, dc.frame, &dc.objects, label)?,
    }
    Ok(())
}

/// Whether `s` is a detached sketch.
fn is_detached(ctx: &Ctx, s: SketchId) -> bool {
    ctx.doc.sketch(s).is_some_and(Sketch::detached)
}

// ---------------------------------------------------------------- refusals

pub(super) fn unknown_unit(key: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_unit",
            &format!(
                "'{key}' names no drawing unit (island, edge, curve, or region) on that plane. \
                 List the plane's units with hew.sketch.units and pass keys exactly as given."
            ),
        )
        .with_detail(json!({ "key": key })),
    )
}

pub(super) fn region_not_pushable() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "region_not_pushable",
        "The face cannot hold that shape (it is too thin to be a face of the solid), \
         so it is kept as drawing only. Move or reshape the lines, then try again.",
    ))
}

fn point_not_on_face(index: usize) -> CmdError {
    CmdError::from(DocumentError::Op(kernel::KernelOpError::Sticky(
        kernel::StickyError::PointNotOnFace { index },
    )))
}

// ---------------------------------------------------------------- unit JSON

fn pt(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn ring_json(ring: &[Point3]) -> Value {
    Value::Array(ring.iter().map(|&p| pt(p)).collect())
}

fn key_str(prefix: &str, key: u64) -> String {
    format!("{prefix}_{key:016x}")
}

fn public_opt(ctx: &Ctx, entity: EntityRef) -> Option<String> {
    ctx.doc
        .sid_of(&entity)
        .map(|sid| ids::public_id(&entity, sid))
}

/// `(store, object_id, sketch_id)` of a drawing edge.
fn edge_store(ctx: &Ctx, store: &DraftStore) -> (&'static str, Option<String>, Option<String>) {
    match *store {
        DraftStore::Topology(o) => ("topology", public_opt(ctx, EntityRef::Object(o)), None),
        DraftStore::Owned(o, s) => (
            "owned",
            public_opt(ctx, EntityRef::Object(o)),
            public_opt(ctx, EntityRef::Sketch(s)),
        ),
        DraftStore::Free(s) => ("free", None, public_opt(ctx, EntityRef::Sketch(s))),
    }
}

fn with_ids(mut v: Value, object_id: Option<String>, sketch_id: Option<String>) -> Value {
    if let Some(o) = object_id {
        v["object_id"] = Value::String(o);
    }
    if let Some(s) = sketch_id {
        v["sketch_id"] = Value::String(s);
    }
    v
}

/// Marks a free store that is a detached sketch (`"detached": true`;
/// absent otherwise, so a plain listing reads as it always did).
fn mark_detached(ctx: &Ctx, mut v: Value, store: &DraftStore) -> Value {
    if let DraftStore::Free(s) = *store
        && is_detached(ctx, s)
    {
        v["detached"] = Value::Bool(true);
    }
    v
}

/// The flat `{kind, key, store, object_id?, sketch_id?}` entry for a
/// drawing edge.
fn edge_entry(ctx: &Ctx, e: &DraftEdge) -> Value {
    let (store, object_id, sketch_id) = edge_store(ctx, &e.store);
    let v = with_ids(
        json!({ "kind": "edge", "key": key_str("edg", e.key), "store": store }),
        object_id,
        sketch_id,
    );
    mark_detached(ctx, v, &e.store)
}

/// The flat entry for a region: topology when a face is it, owned when the
/// solid cannot hold it, free when it lies over the void.
fn region_entry(ctx: &Ctx, r: &kernel::DraftRegion) -> Value {
    let (store, object_id, sketch_id) = match (&r.over, &r.extrude) {
        (Some((o, Some(_))), _) => ("topology", public_opt(ctx, EntityRef::Object(*o)), None),
        (Some((o, None)), _) => ("owned", public_opt(ctx, EntityRef::Object(*o)), None),
        (None, Some((s, _))) => ("free", None, public_opt(ctx, EntityRef::Sketch(*s))),
        (None, None) => ("free", None, None),
    };
    let v = with_ids(
        json!({ "kind": "region", "key": key_str("rgn", r.key), "store": store }),
        object_id,
        sketch_id,
    );
    match (&r.over, &r.extrude) {
        (None, Some((s, _))) => mark_detached(ctx, v, &DraftStore::Free(*s)),
        _ => v,
    }
}

fn near_seg(p: Point3, a: Point3, b: Point3) -> bool {
    geom::point_segment_distance(p, a, b).0 <= geom::API_SURFACE_TOL
}

fn ring_edges(ring: &[Point3]) -> impl Iterator<Item = (Point3, Point3)> + '_ {
    (0..ring.len()).map(move |i| (ring[i], ring[(i + 1) % ring.len()]))
}

/// The regions whose boundary carries any of `segs` (a segment's midpoint
/// lies on one of the region's ring edges).
fn regions_touching<'a>(
    pu: &'a PlaneUnits,
    segs: &[(Point3, Point3)],
) -> Vec<&'a kernel::DraftRegion> {
    pu.regions
        .iter()
        .filter(|r| {
            let rings = std::iter::once(&r.outer).chain(r.holes.iter());
            let mut hit = false;
            for ring in rings {
                for (ra, rb) in ring_edges(ring) {
                    if segs.iter().any(|&(a, b)| {
                        let m =
                            Point3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5);
                        near_seg(m, ra, rb)
                    }) {
                        hit = true;
                    }
                }
            }
            hit
        })
        .collect()
}

/// The plane units a draw touched: every drawing edge lying along one of
/// the drawn segments, then every region whose boundary carries one.
fn touched_entries(ctx: &Ctx, pu: &PlaneUnits, segs: &[(Point3, Point3)]) -> Vec<Value> {
    let mut out = Vec::new();
    for u in &pu.units {
        for e in &u.edges {
            if segs
                .iter()
                .any(|&(a, b)| near_seg(e.a, a, b) && near_seg(e.b, a, b))
            {
                out.push(edge_entry(ctx, e));
            }
        }
    }
    for r in regions_touching(pu, segs) {
        out.push(region_entry(ctx, r));
    }
    out
}

/// Units in `after` whose key is not in `before`: what an edit made.
fn new_entries(ctx: &Ctx, before: &PlaneUnits, after: &PlaneUnits) -> Vec<Value> {
    let old_edges: BTreeSet<u64> = before
        .units
        .iter()
        .flat_map(|u| u.edges.iter().map(|e| e.key))
        .collect();
    let old_regions: BTreeSet<u64> = before.regions.iter().map(|r| r.key).collect();
    let mut out = Vec::new();
    for u in &after.units {
        for e in &u.edges {
            if !old_edges.contains(&e.key) {
                out.push(edge_entry(ctx, e));
            }
        }
    }
    for r in &after.regions {
        if !old_regions.contains(&r.key) {
            out.push(region_entry(ctx, r));
        }
    }
    out
}

fn units_json(ctx: &Ctx, plane: &Plane, pu: &PlaneUnits) -> Value {
    let islands: Vec<Value> = pu
        .units
        .iter()
        .map(|u| {
            let edges: Vec<Value> = u
                .edges
                .iter()
                .map(|e| {
                    let (store, object_id, sketch_id) = edge_store(ctx, &e.store);
                    let mut v = mark_detached(
                        ctx,
                        with_ids(
                            json!({
                                "key": key_str("edg", e.key),
                                "a": pt(e.a),
                                "b": pt(e.b),
                                "store": store,
                            }),
                            object_id,
                            sketch_id,
                        ),
                        &e.store,
                    );
                    if let (Some(g), Some(k)) = (e.curve, e.curve_key) {
                        v["curve"] = json!({
                            "key": key_str("crv", k),
                            "center": pt(g.center),
                            "radius": g.radius,
                        });
                    }
                    v
                })
                .collect();
            json!({ "key": key_str("isl", u.key), "edges": edges })
        })
        .collect();
    let regions: Vec<Value> = pu
        .regions
        .iter()
        .map(|r| {
            let mut v = json!({
                "key": key_str("rgn", r.key),
                "outer": ring_json(&r.outer),
                "holes": r.holes.iter().map(|h| ring_json(h)).collect::<Vec<_>>(),
                "extrudable": r.extrude.is_some(),
            });
            if let Some((o, face)) = r.over
                && let Some(id) = public_opt(ctx, EntityRef::Object(o))
            {
                v["object_id"] = Value::String(id.clone());
                if face.is_some()
                    && let Some(at) =
                        geom::interior_point_of_loops(&r.outer, &r.holes, plane.normal())
                {
                    v["face"] = json!({ "object": id, "at": pt(at) });
                }
            }
            v
        })
        .collect();
    json!({ "units": islands, "regions": regions })
}

// ------------------------------------------------------------- unit keys

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnitKind {
    Island,
    Edge,
    Curve,
    Region,
}

/// Parses `isl_`/`edg_`/`crv_`/`rgn_` followed by exactly 16 hex digits.
/// Anything else (including the compound `rgn_<sketch>_<bits>` ids of
/// per-sketch regions) is an unknown unit.
pub(super) fn parse_unit_key(s: &str) -> Result<(UnitKind, u64), CmdError> {
    let (kind, hex) = [
        ("isl_", UnitKind::Island),
        ("edg_", UnitKind::Edge),
        ("crv_", UnitKind::Curve),
        ("rgn_", UnitKind::Region),
    ]
    .into_iter()
    .find_map(|(p, k)| s.strip_prefix(p).map(|h| (k, h)))
    .ok_or_else(|| unknown_unit(s))?;
    if hex.len() != 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unknown_unit(s));
    }
    let key = u64::from_str_radix(hex, 16).map_err(|_| unknown_unit(s))?;
    Ok((kind, key))
}

fn edge_keys_of_ring(plane: &Plane, ring: &[Point3]) -> Vec<u64> {
    ring_edges(ring)
        .map(|(a, b)| kernel::edge_key(plane, a, b))
        .collect()
}

/// Expands unit keys to drawing-edge keys. An `edg_` key that names no
/// drawing edge is passed through: it may be a solid's own edge, which the
/// draft finds and the kernel then refuses; a key that names no edge at all
/// is caught once the draft is open.
fn expand_keys(plane: &Plane, pu: &PlaneUnits, raw: &[String]) -> Result<Vec<u64>, CmdError> {
    let drawing: BTreeSet<u64> = pu
        .units
        .iter()
        .flat_map(|u| u.edges.iter().map(|e| e.key))
        .collect();
    let mut out: BTreeSet<u64> = BTreeSet::new();
    for s in raw {
        let (kind, key) = parse_unit_key(s)?;
        match kind {
            UnitKind::Island => {
                let u = pu
                    .units
                    .iter()
                    .find(|u| u.key == key)
                    .ok_or_else(|| unknown_unit(s))?;
                out.extend(u.edges.iter().map(|e| e.key));
            }
            UnitKind::Edge => {
                if !drawing.contains(&key) {
                    // A ring edge that is no drawing is a solid's own edge.
                    let solid_edge = pu.regions.iter().any(|r| {
                        std::iter::once(&r.outer)
                            .chain(r.holes.iter())
                            .any(|ring| edge_keys_of_ring(plane, ring).contains(&key))
                    });
                    return Err(if solid_edge {
                        CmdError::from(DocumentError::DraftBoundaryEdgeTouched)
                    } else {
                        unknown_unit(s)
                    });
                }
                out.insert(key);
            }
            UnitKind::Curve => {
                let hits: Vec<u64> = pu
                    .units
                    .iter()
                    .flat_map(|u| u.edges.iter())
                    .filter(|e| e.curve_key == Some(key))
                    .map(|e| e.key)
                    .collect();
                if hits.is_empty() {
                    return Err(unknown_unit(s));
                }
                out.extend(hits);
            }
            UnitKind::Region => {
                let r = pu
                    .regions
                    .iter()
                    .find(|r| r.key == key)
                    .ok_or_else(|| unknown_unit(s))?;
                let ring_keys = std::iter::once(&r.outer)
                    .chain(r.holes.iter())
                    .flat_map(|ring| edge_keys_of_ring(plane, ring));
                let mine: Vec<u64> = ring_keys.filter(|k| drawing.contains(k)).collect();
                if mine.is_empty() {
                    // Every ring edge is a solid's own: topology editing.
                    return Err(CmdError::from(DocumentError::DraftBoundaryEdgeTouched));
                }
                out.extend(mine);
            }
        }
    }
    Ok(out.into_iter().collect())
}

// ------------------------------------------------------------- edit runner

fn sketch_err(e: SketchError) -> CmdError {
    CmdError::from(DocumentError::Sketch(e))
}

/// Closes the open draft as one undo step and answers what it closed; on
/// a refusal the draft is cancelled so nothing stays open.
fn finish_draft(ctx: &mut Ctx) -> Result<Closed, CmdError> {
    match ctx.doc.end_draft() {
        Ok((report, _)) => Ok(Closed::of(&report)),
        Err(e) => {
            ctx.doc.cancel_draft();
            Err(CmdError::from(e))
        }
    }
}

/// Runs one unit edit as one drafting gesture: resolves the context and
/// keys, opens the draft, hands `edit` the working sketch and the selected
/// edge ids, lowers the result, and answers `{units}` (what the edit made).
fn edit_units(
    ctx: &mut Ctx,
    plane_spec: &Value,
    raw: &[String],
    label: &str,
    edit: impl FnOnce(&mut Sketch, &[SketchEdgeId]) -> Result<(), SketchError>,
) -> Result<Value, CmdError> {
    if raw.is_empty() {
        return Err(CmdError::Params("units needs at least one key".into()));
    }
    let dc = resolve_context(ctx, plane_spec)?;
    let before = units_of(ctx, &dc)?;
    let keys = expand_keys(&dc.plane, &before, raw)?;

    begin(ctx, &dc, label)?;
    // (A listed edge another plane's line crosses in the lift names every
    // piece of it: `draft_edges_for_keys`.)
    let (ids, missing) = ctx.doc.draft_edges_for_keys(&keys);
    if let Some(&missing) = missing.first() {
        ctx.doc.cancel_draft();
        return Err(unknown_unit(&key_str("edg", missing)));
    }
    let edited = edit(ctx.doc.draft_sketch_mut().expect("draft open"), &ids);
    if let Err(e) = edited {
        ctx.doc.cancel_draft();
        return Err(sketch_err(e));
    }
    let closed = finish_draft(ctx)?;
    let after = units_of(ctx, &dc)?;
    let mut out = json!({ "units": new_entries(ctx, &before, &after) });
    closed.annotate(ctx, &mut out);
    Ok(out)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveP {
    plane: Value,
    units: Vec<String>,
    offset: Value,
}

fn move_units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MoveP = parse_params(params)?;
    let d = locate::parse_xyz(&p.offset)?;
    let t = Transform::translation(Vec3::new(d.x, d.y, d.z));
    edit_units(ctx, &p.plane, &p.units, "Move", move |w, ids| {
        w.apply_transform_edges(ids, &t)
    })
}

fn copy_units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MoveP = parse_params(params)?;
    let d = locate::parse_xyz(&p.offset)?;
    let t = Transform::translation(Vec3::new(d.x, d.y, d.z));
    edit_units(ctx, &p.plane, &p.units, "Copy", move |w, ids| {
        w.copy_edges_transformed(ids, &t)
    })
}

/// `translation(-c) · M · translation(c)`: `m` applied about `c`.
fn about(c: Point3, m: &Transform) -> Transform {
    Transform::translation(Vec3::new(-c.x, -c.y, -c.z))
        .then(m)
        .then(&Transform::translation(Vec3::new(c.x, c.y, c.z)))
}

fn rotate_units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        units: Vec<String>,
        axis_point: Value,
        angle: f64,
    }
    let p: P = parse_params(params)?;
    if !p.angle.is_finite() {
        return Err(CmdError::Params("angle must be finite".into()));
    }
    let c = locate::resolve_point(ctx, &p.axis_point)?;
    // The axis is the drafting plane's normal.
    let dc = resolve_context(ctx, &p.plane)?;
    let rot = Transform::rotation(dc.plane.normal(), p.angle)
        .map_err(|_| CmdError::Params("degenerate rotation axis".into()))?;
    let t = about(c, &rot);
    edit_units(ctx, &p.plane, &p.units, "Rotate", move |w, ids| {
        w.apply_transform_edges(ids, &t)
    })
}

fn scale_units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        units: Vec<String>,
        center: Value,
        #[serde(default)]
        factor: Option<f64>,
        #[serde(default)]
        factors: Option<[f64; 3]>,
    }
    let p: P = parse_params(params)?;
    let f = match (p.factor, p.factors) {
        (Some(f), None) => [f, f, f],
        (None, Some(f)) => f,
        _ => {
            return Err(CmdError::Params(
                "scale needs exactly one of factor or factors".into(),
            ));
        }
    };
    if f.iter().any(|x| !x.is_finite() || *x == 0.0) {
        return Err(CmdError::Params(
            "scale factors must be finite and non-zero".into(),
        ));
    }
    let c = locate::resolve_point(ctx, &p.center)?;
    let t = about(c, &Transform::scale(Vec3::new(f[0], f[1], f[2])));
    edit_units(ctx, &p.plane, &p.units, "Scale", move |w, ids| {
        w.apply_transform_edges(ids, &t)
    })
}

fn delete_units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        units: Vec<String>,
    }
    let p: P = parse_params(params)?;
    edit_units(ctx, &p.plane, &p.units, "Delete", |w, ids| {
        for &e in ids {
            if w.edges().contains_key(e) {
                w.remove_edge(e)?;
            }
        }
        Ok(())
    })
}

fn units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
    }
    let p: P = parse_params(params)?;
    let dc = resolve_context(ctx, &p.plane)?;
    let pu = units_of(ctx, &dc)?;
    Ok(units_json(ctx, &dc.plane, &pu))
}

/// `hew.sketch.offset` of a face or plane region by unit key.
pub(super) fn offset_region_by_key(
    ctx: &mut Ctx,
    plane_spec: &Value,
    region: &str,
    distance: f64,
) -> Result<Value, CmdError> {
    let (kind, key) = parse_unit_key(region)?;
    if kind != UnitKind::Region {
        return Err(unknown_unit(region));
    }
    let dc = resolve_context(ctx, plane_spec)?;
    let before = units_of(ctx, &dc)?;
    if !before.regions.iter().any(|r| r.key == key) {
        return Err(unknown_unit(region));
    }
    begin(ctx, &dc, "Offset")?;
    let Some(rid) = ctx.doc.draft_region_by_key(key) else {
        ctx.doc.cancel_draft();
        return Err(unknown_unit(region));
    };
    let edited = ctx
        .doc
        .draft_sketch_mut()
        .expect("draft open")
        .offset_region(rid, distance);
    if let Err(e) = edited {
        ctx.doc.cancel_draft();
        return Err(sketch_err(e));
    }
    finish_draft(ctx)?;
    let after = units_of(ctx, &dc)?;
    Ok(json!({ "units": new_entries(ctx, &before, &after) }))
}

// ------------------------------------------------------ face-target drawing

/// One drawn polyline, optionally claiming a circle (or a regular polygon's
/// center) so its facets keep their analytic identity.
pub(super) struct Run {
    pub pts: Vec<Point3>,
    pub claim: Option<(CurveGeom, SketchCurveKind)>,
}

impl Run {
    pub fn plain(pts: Vec<Point3>) -> Run {
        Run { pts, claim: None }
    }

    pub fn curve(pts: Vec<Point3>, geom: CurveGeom, kind: SketchCurveKind) -> Run {
        Run {
            pts,
            claim: Some((geom, kind)),
        }
    }
}

/// Closes a ring of points back onto its first point.
pub(super) fn closed(mut ring: Vec<Point3>) -> Vec<Point3> {
    if let Some(&first) = ring.first() {
        ring.push(first);
    }
    ring
}

fn on_or_in(ring: &[Point3], normal: Vec3, p: Point3) -> bool {
    ring_edges(ring).any(|(a, b)| near_seg(p, a, b)) || geom::face_contains(ring, &[], normal, p)
}

/// Draws `runs` as ONE new detached sketch on `plane` in `frame` (detached
/// shapes D3, D10): nothing on the plane is lifted, so the shape neither
/// splits nor joins a face or other drawing, and extruding it always makes
/// a new Object. Returns the new sketch, the curve chain carrying a run's
/// claimed curve (if any), and what the draw closed (a detached line closes
/// faces with other detached drawing and Objects' edges, §4.3). One undo
/// step; a refusal changes nothing.
pub(super) fn draw_detached(
    ctx: &mut Ctx,
    plane: Plane,
    frame: Option<ComponentId>,
    label: &str,
    runs: Vec<Run>,
) -> Result<(SketchId, Option<kernel::SketchCurveId>, Closed), CmdError> {
    let (report, _) = ctx.doc.draft_detached_new(plane, frame, label, |w| {
        for run in &runs {
            if let Some((g, kind)) = run.claim {
                w.begin_curve_with_kind(g, kind)?;
            }
            for win in run.pts.windows(2) {
                w.add_segment(win[0], win[1])?;
            }
            if run.claim.is_some() {
                w.end_curve();
            }
        }
        Ok(())
    })?;
    let sketch = *report
        .sketches
        .first()
        .ok_or_else(|| CmdError::Internal("a detached draw made no sketch".into()))?;
    // The lowering rebuilt the drawing, so the claimed curve is found in
    // the sketch as stored (one claimed run per draw command).
    let curve = runs
        .iter()
        .any(|r| r.claim.is_some())
        .then(|| {
            ctx.doc
                .sketch(sketch)
                .and_then(|sk| sk.edges().values().find_map(|e| e.curve))
        })
        .flatten();
    Ok((sketch, curve, Closed::of(&report)))
}

/// [`draw_on_face`]'s sibling for `detached: true`: the same plane and frame
/// as a face draw (and the same `point_not_on_face` check), but the shape
/// becomes one new detached sketch ([`draw_detached`]) and the face is
/// left whole.
pub(super) fn draw_detached_on_face(
    ctx: &mut Ctx,
    face: FaceRef,
    label: &str,
    runs: Vec<Run>,
) -> Result<(SketchId, Option<kernel::SketchCurveId>, Closed), CmdError> {
    let plane = face_plane(ctx, face)?;
    let frame = ctx.doc.object_owner_component(face.object);
    check_on_face(&plane, &runs)?;
    draw_detached(ctx, plane, frame, label, runs)
}

/// Refuses the first drawn point off the face's plane (`point_not_on_face`,
/// indexed across every run).
fn check_on_face(plane: &Plane, runs: &[Run]) -> Result<(), CmdError> {
    let mut index = 0usize;
    for run in runs {
        for &p in &run.pts {
            if plane.signed_distance(p).abs() > kernel::tol::PLANE_DIST {
                return Err(point_not_on_face(index));
            }
            index += 1;
        }
    }
    Ok(())
}

/// Draws `runs` on the face's plane through the draft engine and answers
/// `{object_id, units}`, minting face tokens from the faces the draw
/// produced: for a closed shape `"face"` (the face carrying the drawn
/// region) and `"parent"` (the face across the cut); for an open path `"a"`
/// and `"b"` (faces either side of it). A draw that produced no face (a
/// loose line, a shape over the void) mints none.
pub(super) fn draw_on_face(
    ctx: &mut Ctx,
    face: FaceRef,
    label: &str,
    runs: Vec<Run>,
) -> Result<Value, CmdError> {
    let plane = face_plane(ctx, face)?;
    let frame = ctx.doc.object_owner_component(face.object);
    check_on_face(&plane, &runs)?;
    let dc = context_of(ctx, plane, frame, Some(face.object));
    let (report, _) = ctx.doc.draft(plane, frame, &dc.objects, label, |w| {
        for run in &runs {
            if let Some((g, kind)) = run.claim {
                w.begin_curve_with_kind(g, kind)?;
            }
            for win in run.pts.windows(2) {
                w.add_segment(win[0], win[1])?;
            }
            if run.claim.is_some() {
                w.end_curve();
            }
        }
        Ok(())
    })?;

    let pu = units_of(ctx, &dc)?;
    let segs: Vec<(Point3, Point3)> = runs
        .iter()
        .flat_map(|r| r.pts.windows(2).map(|w| (w[0], w[1])))
        .collect();
    let entries = touched_entries(ctx, &pu, &segs);

    // Face tokens: the face-backed regions the drawn geometry bounds.
    let touching: Vec<(ObjectId, kernel::FaceId, &kernel::DraftRegion)> =
        regions_touching(&pu, &segs)
            .into_iter()
            .filter_map(|r| match r.over {
                Some((o, Some(f))) => Some((o, f, r)),
                _ => None,
            })
            .collect();
    let all_pts: Vec<Point3> = runs.iter().flat_map(|r| r.pts.iter().copied()).collect();
    let is_closed = all_pts.len() >= 4
        && all_pts
            .first()
            .zip(all_pts.last())
            .is_some_and(|(a, b)| a.approx_eq(*b, geom::API_SURFACE_TOL));
    if is_closed {
        let mut ring: Vec<Point3> = all_pts.clone();
        ring.pop();
        ring.dedup_by(|a, b| a.approx_eq(*b, geom::API_SURFACE_TOL));
        let normal = plane.normal();
        let inside = |r: &kernel::DraftRegion| r.outer.iter().all(|&p| on_or_in(&ring, normal, p));
        let carrying = touching.iter().find(|(_, _, r)| inside(r));
        let parent = touching.iter().find(|(_, _, r)| !inside(r));
        if let Some(&(o, f, _)) = carrying {
            ctx.mint_face_token("face", o, f);
        }
        if let Some(&(o, f, _)) = parent {
            ctx.mint_face_token("parent", o, f);
        }
    } else {
        if let Some(&(o, f, _)) = touching.first() {
            ctx.mint_face_token("a", o, f);
        }
        if let Some(&(o, f, _)) = touching.get(1) {
            ctx.mint_face_token("b", o, f);
        }
    }

    let object_id = public_opt(ctx, EntityRef::Object(face.object))
        .expect("an object this command just touched always carries a stable id");
    let mut out = json!({ "object_id": object_id, "units": entries });
    Closed::of(&report).annotate(ctx, &mut out);
    Ok(out)
}
