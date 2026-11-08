//! Pure geometry computations: `hew.compute.*` (docs/agents/HEW_API.md §7
//! "Compute", docs/agents/PLUGINS.md "Checking before committing").
//!
//! Every command here reads the document and never changes it: operands
//! resolve to transient world-space Objects ([`crate::solid_spec`]), the
//! kernel's pure functions run on those (`Object::boolean_product`,
//! `Object::slice`, `Object::split_connected_components`,
//! `kernel::offset_profile`), and the answer carries geometry, never an id.
//! Result blocks use the shapes of `hew.query.mesh` and
//! `hew.query.topology` without their clock (a computation is not tied to
//! a document state), with `parts[].id` and `object` `null`.
//!
//! One computation at a time per connection: the dispatcher holds the
//! connection's [`crate::dispatch::ComputeGate`] around each call.

use super::{CmdError, Ctx, Handler};
use crate::commands::parts::{MAX_MESH_BYTES, mesh_block, topology_block};
use crate::geom;
use crate::locate;
use crate::part_frame;
use crate::refusal::Refusal;
use crate::solid_spec::{self, PlaneParams, check_compute_size, resolve_solid};
use kernel::{
    BooleanError, BooleanOp, DocumentError, Object, Plane, Point3, RenderRoot, Transform, Vec3,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// This namespace's slice of the handler table. `hew.compute.project` and
/// `hew.compute.stretch` are declared but have no handler yet (core C6 and
/// C7), so they answer `unimplemented`.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.compute.boolean" => boolean,
        "hew.compute.slice" => slice,
        "hew.compute.shells" => shells,
        "hew.compute.measure" => measure,
        "hew.compute.outline" => outline,
        "hew.compute.offset" => offset,
        _ => return None,
    })
}

fn parse<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

fn p3(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn p2(p: [f64; 2]) -> Value {
    json!([p[0], p[1]])
}

// ------------------------------------------------------------ result blocks

/// Which result blocks a call wants.
#[derive(Clone, Copy, Default)]
struct Want {
    mesh: bool,
    topology: bool,
    measure: bool,
    shells: bool,
}

fn want_of(list: &Option<Vec<String>>) -> Result<Want, CmdError> {
    let Some(list) = list else {
        return Ok(Want {
            measure: true,
            ..Want::default()
        });
    };
    let mut w = Want::default();
    for item in list {
        match item.as_str() {
            "mesh" => w.mesh = true,
            "topology" => w.topology = true,
            "measure" => w.measure = true,
            "shells" => w.shells = true,
            other => {
                return Err(CmdError::Params(format!(
                    "unknown want \"{other}\"; expected mesh, topology, measure, or shells"
                )));
            }
        }
    }
    Ok(w)
}

/// `{ volume, area, bbox: { min, max } }`, plus `centroid` when asked:
/// `volume` and `centroid` are `null` for an Object that is not
/// watertight (an open shell encloses nothing).
fn measure_json(object: &Object, with_centroid: bool) -> Value {
    let volume = object.volume();
    let bbox = geom::bbox(object.vertices().values().map(|v| v.position));
    let mut out = json!({
        "volume": volume,
        "area": object.surface_area(),
        "bbox": match bbox {
            Some((lo, hi)) => json!({ "min": p3(lo), "max": p3(hi) }),
            None => Value::Null,
        },
    });
    if with_centroid {
        out["centroid"] = match volume.and_then(|_| centroid(object)) {
            Some(c) => p3(c),
            None => Value::Null,
        };
    }
    out
}

/// The volume centroid by the divergence theorem: every face loop fanned
/// into triangles, each the base of a tetrahedron to the origin, weighted
/// by its signed volume (holes wind opposite and subtract, as in
/// `Object::signed_volume`).
fn centroid(object: &Object) -> Option<Point3> {
    let mut six_v = 0.0;
    let mut acc = Vec3::new(0.0, 0.0, 0.0);
    for f in object.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let p: Vec<Vec3> = object.loop_positions(l).map(|q| q.to_vec()).collect();
            for i in 1..p.len().saturating_sub(1) {
                let w = p[0].dot(p[i].cross(p[i + 1]));
                six_v += w;
                acc = acc + (p[0] + p[i] + p[i + 1]) * w;
            }
        }
    }
    (six_v.abs() > f64::EPSILON).then(|| {
        let c = acc * (1.0 / (4.0 * six_v));
        Point3::new(c.x, c.y, c.z)
    })
}

/// The wanted blocks for one result Object.
fn blocks(ctx: &Ctx, object: &Object, want: Want) -> Result<Map<String, Value>, CmdError> {
    let mut out = Map::new();
    if want.mesh {
        out.insert(
            "mesh".into(),
            mesh_block(
                ctx.doc,
                &[(object, Transform::IDENTITY, Value::Null)],
                MAX_MESH_BYTES,
            )?,
        );
    }
    if want.topology {
        out.insert(
            "topology".into(),
            topology_block(ctx.doc, object, Transform::IDENTITY, Value::Null),
        );
    }
    if want.measure {
        solid_spec::check_finite_measure(object)?;
        out.insert("measure".into(), measure_json(object, false));
    }
    if want.shells {
        out.insert(
            "shells".into(),
            json!(object.split_connected_components().len()),
        );
    }
    Ok(out)
}

/// A kernel refusal that has no `DocumentError` wrapper, by its variant
/// name (`OffsetCollapsed` → `offset_collapsed`), with the UI's copy when
/// it has one.
fn kernel_refusal(variant: &str, display: &str) -> CmdError {
    let name = crate::refusal::snake_case(variant);
    let explanation = crate::refusal::explanation_for(&name)
        .map(str::to_string)
        .unwrap_or_else(|| display.to_string());
    CmdError::Refusal(Refusal::api(&name, &explanation))
}

// ------------------------------------------------------- hew.compute.boolean

fn boolean(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        op: String,
        a: Value,
        b: Value,
        #[serde(default)]
        want: Option<Vec<String>>,
    }
    let p: P = parse(params)?;
    let op = match p.op.as_str() {
        "union" => BooleanOp::Union,
        "subtract" => BooleanOp::Subtract,
        "intersect" => BooleanOp::Intersect,
        other => {
            return Err(CmdError::Params(format!(
                "unknown op \"{other}\"; expected union, subtract, or intersect"
            )));
        }
    };
    let want = want_of(&p.want)?;
    let a = resolve_solid(ctx, &p.a)?;
    let b = resolve_solid(ctx, &p.b)?;
    check_compute_size(&[&a, &b])?;
    // Both operands are in world space, so b maps into a's frame unchanged.
    match Object::boolean_product(op, &a, &b, &Transform::IDENTITY) {
        Ok(result) => {
            let mut out = blocks(ctx, &result, want)?;
            out.insert("solid".into(), Value::Bool(true));
            Ok(Value::Object(out))
        }
        // Nothing left: an intersect of disjoint solids, a subtract that
        // removes everything. An answer, not a refusal.
        Err(BooleanError::EmptyResult) => Ok(json!({ "solid": false })),
        Err(e) => Err(DocumentError::Boolean(e).into()),
    }
}

// --------------------------------------------------------- hew.compute.slice

fn slice(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        solid: Value,
        plane: PlaneParams,
        #[serde(default)]
        want: Option<Vec<String>>,
    }
    let p: P = parse(params)?;
    let want = want_of(&p.want)?;
    let plane = solid_spec::plane_of(&p.plane)?;
    let solid = resolve_solid(ctx, &p.solid)?;
    check_compute_size(&[&solid])?;
    let (positive, negative) = solid
        .slice(&plane)
        .map_err(|e| CmdError::from(DocumentError::Slice(e)))?;
    Ok(json!({
        "positive": Value::Object(blocks(ctx, &positive, want)?),
        "negative": Value::Object(blocks(ctx, &negative, want)?),
    }))
}

// -------------------------------------------------------- hew.compute.shells

fn shells(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        solid: Value,
        #[serde(default)]
        want_mesh: bool,
    }
    let p: P = parse(params)?;
    let solid = resolve_solid(ctx, &p.solid)?;
    check_compute_size(&[&solid])?;
    let pieces = solid.split_connected_components();
    let mut out = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        solid_spec::check_finite_measure(piece)?;
        let mut entry = json!({ "measure": measure_json(piece, false) });
        if p.want_mesh {
            entry["mesh"] = mesh_block(
                ctx.doc,
                &[(piece, Transform::IDENTITY, Value::Null)],
                MAX_MESH_BYTES,
            )?;
        }
        out.push(entry);
    }
    Ok(json!({ "count": pieces.len(), "shells": out }))
}

// ------------------------------------------------------- hew.compute.measure

fn measure(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        solid: Value,
    }
    let p: P = parse(params)?;
    let solid = resolve_solid(ctx, &p.solid)?;
    check_compute_size(&[&solid])?;
    solid_spec::check_finite_measure(&solid)?;
    Ok(measure_json(&solid, true))
}

// ------------------------------------------------------- loops and edges

/// One boundary segment in frame coordinates, with the circle it is a
/// chord facet of when that circle lies in the frame's xy plane.
#[derive(Clone, Copy)]
struct Seg {
    a: [f64; 2],
    b: [f64; 2],
    circle: Option<([f64; 2], f64)>,
}

/// A circle in frame coordinates: center and radius.
type Circle2 = ([f64; 2], f64);

fn same_circle(x: Option<([f64; 2], f64)>, y: Option<([f64; 2], f64)>) -> bool {
    match (x, y) {
        (Some((c, r)), Some((d, s))) => {
            (c[0] - d[0]).abs() <= 1e-9 && (c[1] - d[1]).abs() <= 1e-9 && (r - s).abs() <= 1e-9
        }
        _ => false,
    }
}

fn signed_area_2d(segs: &[Seg]) -> f64 {
    segs.iter()
        .map(|s| s.a[0] * s.b[1] - s.b[0] * s.a[1])
        .sum::<f64>()
        * 0.5
}

/// A closed run of segments as `{ outer, edges }`, wound counter-clockwise
/// for an outer loop and clockwise for a hole: consecutive facets of one
/// circle merge into one `arc`, a whole loop on one circle is one `circle`,
/// everything else is a `line`.
fn loop_json(mut segs: Vec<Seg>, outer: bool) -> Value {
    if segs.is_empty() {
        return json!({ "outer": outer, "edges": [] });
    }
    if (signed_area_2d(&segs) > 0.0) != outer {
        segs.reverse();
        for s in &mut segs {
            std::mem::swap(&mut s.a, &mut s.b);
        }
    }
    if segs.iter().all(|s| same_circle(s.circle, segs[0].circle)) {
        let (center, radius) = segs[0].circle.expect("same_circle needs a circle");
        return json!({
            "outer": outer,
            "edges": [{ "kind": "circle", "center": p2(center), "radius": radius }],
        });
    }
    // Start at a segment that does not continue its predecessor's circle,
    // so no arc wraps across the start.
    let n = segs.len();
    let start = (0..n)
        .find(|&k| !same_circle(segs[(k + n - 1) % n].circle, segs[k].circle))
        .unwrap_or(0);
    segs.rotate_left(start);
    let mut edges = Vec::new();
    let mut k = 0;
    while k < n {
        let s = segs[k];
        match s.circle {
            None => {
                edges.push(json!({ "kind": "line", "a": p2(s.a), "b": p2(s.b) }));
                k += 1;
            }
            Some((center, radius)) => {
                let mut end = k;
                while end + 1 < n && same_circle(segs[end + 1].circle, s.circle) {
                    end += 1;
                }
                let (u, v) = (
                    [s.a[0] - center[0], s.a[1] - center[1]],
                    [s.b[0] - center[0], s.b[1] - center[1]],
                );
                edges.push(json!({
                    "kind": "arc",
                    "center": p2(center),
                    "radius": radius,
                    "start": p2(s.a),
                    "end": p2(segs[end].b),
                    "ccw": u[0] * v[1] - u[1] * v[0] > 0.0,
                }));
                k = end + 1;
            }
        }
    }
    json!({ "outer": outer, "edges": edges })
}

/// An orthonormal right-handed frame: world origin and axes.
#[derive(Clone, Copy)]
struct Frame {
    origin: Point3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

impl Frame {
    fn project(&self, p: Point3) -> [f64; 2] {
        let d = p - self.origin;
        [d.dot(self.x), d.dot(self.y)]
    }
}

/// `invalid_frame`: an explicit frame that is not orthonormal and
/// right-handed.
fn invalid_frame() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "invalid_frame",
        "That frame's axes are not unit length, square to each other, and right-handed. Send x, y, and z as unit vectors with z = x × y.",
    ))
}

fn frame_of(ctx: &Ctx, occ: &kernel::Occurrence, spec: &Option<Value>) -> Result<Frame, CmdError> {
    match spec {
        None => part_frame_of(ctx, occ),
        Some(Value::String(s)) if s == "part" => part_frame_of(ctx, occ),
        Some(v @ Value::Object(_)) => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct F {
                origin: [f64; 3],
                x: [f64; 3],
                y: [f64; 3],
                z: [f64; 3],
            }
            let f: F = parse(v)?;
            let vec = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
            let (x, y, z) = (vec(f.x), vec(f.y), vec(f.z));
            let tol = 1e-9;
            let unit = |v: Vec3| (v.length() - 1.0).abs() <= tol;
            if ![x, y, z].iter().all(|&v| unit(v))
                || x.dot(y).abs() > tol
                || y.dot(z).abs() > tol
                || z.dot(x).abs() > tol
                || !x.cross(y).approx_eq(z, 1e-6)
            {
                return Err(invalid_frame());
            }
            Ok(Frame {
                origin: Point3::new(f.origin[0], f.origin[1], f.origin[2]),
                x,
                y,
                z,
            })
        }
        Some(_) => Err(CmdError::Params(
            "frame is \"part\" or {origin, x, y, z}".into(),
        )),
    }
}

fn part_frame_of(ctx: &Ctx, occ: &kernel::Occurrence) -> Result<Frame, CmdError> {
    let f = part_frame::part_frame(ctx.doc, occ, crate::scope::Scope::All)?.ok_or_else(|| {
        CmdError::Refusal(Refusal::api(
            "no_part_frame",
            "That entity has nothing to measure: it holds no faces.",
        ))
    })?;
    Ok(Frame {
        origin: f.origin,
        x: f.x,
        y: f.y,
        z: f.z,
    })
}

/// The circle an edge is a chord facet of, in world space, with the axis
/// it turns about: the edge's own claim (its axis the cylinder it rims,
/// else the face it lies on, `hew.query.topology`'s rule), else the
/// cylinder wall it rims (an extruded hole's rim), whose circle seen along
/// the axis is the cylinder's. Under a pose that is not a similarity there
/// is no circle.
fn world_circle(
    object: &Object,
    pose: &Transform,
    edge: kernel::EdgeId,
) -> Option<(Point3, f64, Vec3)> {
    let e = object.edges().get(edge)?;
    let scale = pose.similarity_scale()?;
    let he = &object.half_edges()[e.half_edge];
    let mut faces = vec![object.loops()[he.loop_id].face];
    if let Some(t) = e.twin_half_edge {
        faces.push(object.loops()[object.half_edges()[t].loop_id].face);
    }
    let cylinder = faces.iter().find_map(|f| object.faces()[*f].surface);
    if let Some(c) = e.curve {
        let axis = cylinder
            .map(|kernel::SurfaceRef::Cylinder { axis, .. }| pose.apply_vector(axis))
            .unwrap_or_else(|| pose.apply_normal(object.faces()[faces[0]].plane.normal()));
        return Some((
            pose.apply_point(c.center),
            c.radius * scale,
            axis.normalized().ok()?,
        ));
    }
    // A wall facet's seam runs along the axis and is no chord of anything;
    // only an edge across the axis (a rim) is.
    let kernel::SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    } = cylinder?;
    let (a, b) = (
        object.vertices()[he.origin].position,
        object.vertices()[object.half_edges()[he.next].origin].position,
    );
    let along = (b - a).normalized().ok()?;
    if along.dot(axis.normalized().ok()?).abs() > 1e-6 {
        return None;
    }
    Some((
        pose.apply_point(axis_point),
        radius * scale,
        pose.apply_vector(axis).normalized().ok()?,
    ))
}

fn parallel(a: Vec3, b: Vec3) -> bool {
    a.dot(b).abs() >= 1.0 - 1e-9
}

// ------------------------------------------------------- hew.compute.outline

fn outline(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        path: Option<Vec<String>>,
        #[serde(default)]
        frame: Option<Value>,
        which: String,
    }
    let p: P = parse(params)?;
    let occ = match (&p.id, &p.path) {
        (Some(id), None) => locate::resolve_world_node(ctx, id)?,
        (None, Some(path)) => locate::resolve_occurrence_path(ctx, path)?,
        _ => {
            return Err(CmdError::Params(
                "give exactly one of \"id\" or \"path\"".into(),
            ));
        }
    };
    let frame = frame_of(ctx, &occ, &p.frame)?;
    let leaves: Vec<(&Object, Transform)> = ctx
        .doc
        .render_root_leaves(&RenderRoot::Occurrence(occ.clone()), true)?
        .into_iter()
        .filter_map(|(o, t, _)| ctx.doc.object(o).map(|obj| (obj, t)))
        .collect();
    let solids: Vec<&Object> = leaves.iter().map(|(o, _)| *o).collect();
    check_compute_size(&solids)?;
    let loops = match p.which.as_str() {
        "largest_face" => largest_face_loops(&leaves, &frame),
        "projection" => projection_loops(&leaves, &frame)?,
        other => {
            return Err(CmdError::Params(format!(
                "unknown which \"{other}\"; expected largest_face or projection"
            )));
        }
    };
    Ok(json!({ "loops": loops }))
}

/// The loops of the largest face (ties: the one facing the frame's +z
/// most), projected onto the frame's xy. A circle claim stays analytic
/// when its axis is the frame's z.
fn largest_face_loops(leaves: &[(&Object, Transform)], frame: &Frame) -> Vec<Value> {
    let mut best: Option<(f64, f64, &Object, Transform, kernel::FaceId)> = None;
    for &(object, pose) in leaves {
        for (fid, f) in object.faces() {
            let outer: Vec<Point3> = object
                .loop_positions(f.outer_loop)
                .map(|q| pose.apply_point(q))
                .collect();
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| {
                    object
                        .loop_positions(l)
                        .map(|q| pose.apply_point(q))
                        .collect()
                })
                .collect();
            let area = geom::face_area(&outer, &holes);
            let facing = pose
                .apply_normal(f.plane.normal())
                .normalized()
                .map(|n| n.dot(frame.z))
                .unwrap_or(0.0);
            let better = match best {
                None => true,
                Some((a, fz, ..)) => {
                    let tol = 1e-9 * a.max(1e-12);
                    area > a + tol || ((area - a).abs() <= tol && facing > fz + 1e-12)
                }
            };
            if better {
                best = Some((area, facing, object, pose, fid));
            }
        }
    }
    let Some((_, _, object, pose, fid)) = best else {
        return Vec::new();
    };
    let face = &object.faces()[fid];
    let face_normal = pose
        .apply_normal(face.plane.normal())
        .normalized()
        .unwrap_or(frame.z);
    let analytic = parallel(face_normal, frame.z);
    let segs_of = |l: kernel::LoopId| -> Vec<Seg> {
        object
            .loop_half_edges(l)
            .map(|h| {
                let he = &object.half_edges()[h];
                let a = pose.apply_point(object.vertices()[he.origin].position);
                let b = pose
                    .apply_point(object.vertices()[object.half_edges()[he.next].origin].position);
                let circle = analytic
                    .then(|| world_circle(object, &pose, he.edge))
                    .flatten()
                    .filter(|(_, _, axis)| parallel(*axis, frame.z))
                    .map(|(c, r, _)| (frame.project(c), r));
                Seg {
                    a: frame.project(a),
                    b: frame.project(b),
                    circle,
                }
            })
            .collect()
    };
    std::iter::once(loop_json(segs_of(face.outer_loop), true))
        .chain(
            face.inner_loops
                .iter()
                .map(|&l| loop_json(segs_of(l), false)),
        )
        .collect()
}

/// The silhouette of every face seen along the frame's z, as loops in the
/// frame's xy: every front-facing face's loops drawn on one scratch
/// sketch, whose arrangement splits the plane into cells; the cells inside
/// some face are the silhouette, and their boundary (the directed edges no
/// two kept cells share) is the answer. A boundary segment that is one of
/// the facets of a circle whose axis is the frame's z (by the edge's
/// circle claim or the cylinder it rims) is re-attributed to that circle,
/// so an arc seen face-on stays an arc at any facet count; any other edge,
/// a split facet's pieces included, projects as a line.
fn projection_loops(
    leaves: &[(&Object, Transform)],
    frame: &Frame,
) -> Result<Vec<Value>, CmdError> {
    let to2 = |q: [f64; 2]| Point3::new(q[0], q[1], 0.0);
    let mut faces2d: Vec<(Vec<Point3>, Vec<Vec<Point3>>)> = Vec::new();
    // Every edge that is a chord facet of a circle seen face-on, by its
    // projected ends, with that circle.
    let mut chords: Vec<([f64; 2], [f64; 2], Circle2)> = Vec::new();
    for &(object, pose) in leaves {
        for f in object.faces().values() {
            let n = pose
                .apply_normal(f.plane.normal())
                .normalized()
                .unwrap_or(frame.z);
            if n.dot(frame.z) <= 1e-9 {
                continue;
            }
            let ring = |l: kernel::LoopId| -> Vec<Point3> {
                object
                    .loop_positions(l)
                    .map(|q| to2(frame.project(pose.apply_point(q))))
                    .collect()
            };
            faces2d.push((
                ring(f.outer_loop),
                f.inner_loops.iter().map(|&l| ring(l)).collect(),
            ));
        }
        for (eid, e) in object.edges() {
            if let Some((c, r, axis)) = world_circle(object, &pose, eid)
                && parallel(axis, frame.z)
            {
                let he = &object.half_edges()[e.half_edge];
                let end = |v: kernel::VertexId| {
                    frame.project(pose.apply_point(object.vertices()[v].position))
                };
                chords.push((
                    end(he.origin),
                    end(object.half_edges()[he.next].origin),
                    (frame.project(c), r),
                ));
            }
        }
    }
    if faces2d.is_empty() {
        return Ok(Vec::new());
    }
    let up = Vec3::new(0.0, 0.0, 1.0);
    let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), up)
        .map_err(|_| CmdError::Internal("the projection plane is degenerate".into()))?;
    let mut scratch = kernel::Document::new();
    let s = scratch.add_sketch(plane);
    {
        let sk = scratch
            .sketch_mut(s)
            .ok_or_else(|| CmdError::Internal("scratch sketch vanished".into()))?;
        for (outer, holes) in &faces2d {
            for ring in std::iter::once(outer).chain(holes.iter()) {
                for k in 0..ring.len() {
                    match sk.add_segment(ring[k], ring[(k + 1) % ring.len()]) {
                        Ok(_) | Err(kernel::SketchError::DegenerateSegment) => {}
                        Err(e) => return Err(DocumentError::Sketch(e).into()),
                    }
                }
            }
        }
    }
    let sketch = scratch
        .sketch(s)
        .ok_or_else(|| CmdError::Internal("scratch sketch vanished".into()))?;
    let pos = |v: kernel::SketchVertexId| sketch.vertices()[v].position;
    // Directed boundary edges of the kept cells: outer rings run
    // counter-clockwise and holes clockwise, so an edge two kept cells
    // share appears once each way and cancels.
    let mut directed: Vec<(kernel::SketchVertexId, kernel::SketchVertexId)> = Vec::new();
    for (_, region) in sketch.regions() {
        let outer: Vec<Point3> = region.outer.iter().map(|&v| pos(v)).collect();
        let holes: Vec<Vec<Point3>> = region
            .holes
            .iter()
            .map(|h| h.iter().map(|&v| pos(v)).collect())
            .collect();
        let Some(inside) = geom::interior_point_of_loops(&outer, &holes, up) else {
            continue;
        };
        if !faces2d
            .iter()
            .any(|(o, h)| geom::face_contains(o, h, up, inside))
        {
            continue;
        }
        for ring in std::iter::once(&region.outer).chain(region.holes.iter()) {
            for k in 0..ring.len() {
                directed.push((ring[k], ring[(k + 1) % ring.len()]));
            }
        }
    }
    let set: std::collections::BTreeSet<_> = directed.iter().copied().collect();
    let mut next: std::collections::BTreeMap<kernel::SketchVertexId, Vec<kernel::SketchVertexId>> =
        Default::default();
    for &(a, b) in &directed {
        if !set.contains(&(b, a)) {
            next.entry(a).or_default().push(b);
        }
    }
    // A boundary segment is re-attributed to a circle only when it IS one
    // of the circle's own facets (either direction): the claim on the edge
    // says so, whatever the facet count, so a coarse arc stays an arc and a
    // straight edge whose ends merely sit on a circle stays a line. A facet
    // the arrangement split (another face's edge crossing it) has a piece
    // with an end off the circle and is a line; its pieces on both sides
    // are not facets either, so the arc is not stretched across the split.
    let chord_of = |a: [f64; 2], b: [f64; 2]| -> Option<([f64; 2], f64)> {
        let near =
            |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() <= 1e-9 && (p[1] - q[1]).abs() <= 1e-9;
        chords
            .iter()
            .find(|&&(p, q, _)| (near(a, p) && near(b, q)) || (near(a, q) && near(b, p)))
            .map(|&(_, _, circle)| circle)
    };
    let mut loops: Vec<(f64, Value)> = Vec::new();
    while let Some((&start, _)) = next.iter().find(|(_, outs)| !outs.is_empty()) {
        let mut segs = Vec::new();
        let mut at = start;
        while let Some(to) = next.get_mut(&at).and_then(Vec::pop) {
            let (a, b) = (pos(at), pos(to));
            let (a, b) = ([a.x, a.y], [b.x, b.y]);
            segs.push(Seg {
                a,
                b,
                circle: chord_of(a, b),
            });
            at = to;
            if at == start {
                break;
            }
        }
        let area = signed_area_2d(&segs);
        loops.push((area, loop_json(segs, area > 0.0)));
    }
    // Largest outer first, then the rest by size.
    loops.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(loops.into_iter().map(|(_, v)| v).collect())
}

// -------------------------------------------------------- hew.compute.offset

fn offset(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let _ = ctx;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        loops: Vec<Vec<[f64; 2]>>,
        distance: f64,
        #[serde(default)]
        join: Option<String>,
    }
    let p: P = parse(params)?;
    match p.join.as_deref() {
        None | Some("miter") => {}
        Some("round") => {
            return Err(CmdError::Params("round joins are not available yet".into()));
        }
        Some(other) => {
            return Err(CmdError::Params(format!(
                "unknown join \"{other}\"; expected miter or round"
            )));
        }
    }
    if !p.distance.is_finite() {
        return Err(CmdError::Params("distance must be a finite number".into()));
    }
    if p.loops.iter().flatten().flatten().any(|x| !x.is_finite()) {
        return Err(CmdError::Params(
            "loop points must be finite numbers".into(),
        ));
    }
    let rings: Vec<Vec<Point3>> = p
        .loops
        .iter()
        .map(|l| l.iter().map(|q| Point3::new(q[0], q[1], 0.0)).collect())
        .collect();
    let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .map_err(|_| CmdError::Internal("the offset plane is degenerate".into()))?;
    let (scratch, s, region) = solid_spec::scratch_region(plane, &rings)?;
    let profile = scratch
        .sketch(s)
        .ok_or_else(|| CmdError::Internal("scratch sketch vanished".into()))?
        .profile(region)
        .map_err(|e| CmdError::from(DocumentError::Sketch(e)))?;
    let result = kernel::offset_profile(&profile, p.distance)
        .map_err(|e| kernel_refusal(&format!("{e:?}"), &e.to_string()))?;
    let segs = |l: &kernel::OffsetLoop| -> Vec<Seg> {
        let n = l.points.len();
        (0..n)
            .map(|k| {
                let (a, b) = (l.points[k], l.points[(k + 1) % n]);
                Seg {
                    a: [a.x, a.y],
                    b: [b.x, b.y],
                    circle: l
                        .curves
                        .get(k)
                        .copied()
                        .flatten()
                        .map(|c| ([c.center.x, c.center.y], c.radius)),
                }
            })
            .collect()
    };
    let loops: Vec<Value> = std::iter::once(loop_json(segs(&result.outer), true))
        .chain(result.holes.iter().map(|h| loop_json(segs(h), false)))
        .collect();
    Ok(json!({ "loops": loops }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: [f64; 2], b: [f64; 2]) -> Seg {
        Seg { a, b, circle: None }
    }

    #[test]
    fn a_clockwise_outer_loop_is_rewound() {
        let segs = vec![
            seg([0.0, 0.0], [0.0, 1.0]),
            seg([0.0, 1.0], [1.0, 1.0]),
            seg([1.0, 1.0], [1.0, 0.0]),
            seg([1.0, 0.0], [0.0, 0.0]),
        ];
        let v = loop_json(segs, true);
        assert_eq!(v["edges"][0]["a"], json!([0.0, 0.0]));
        assert_eq!(v["edges"][0]["b"], json!([1.0, 0.0]));
        assert_eq!(v["edges"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn consecutive_facets_of_one_circle_merge_into_one_arc() {
        let c = Some(([0.0, 0.0], 1.0));
        let pt = |t: f64| [t.cos(), t.sin()];
        let mut segs = vec![seg([1.0, -1.0], [1.0, 0.0])];
        for k in 0..4 {
            let (t0, t1) = (k as f64 * 0.25, (k + 1) as f64 * 0.25);
            segs.push(Seg {
                a: pt(t0),
                b: pt(t1),
                circle: c,
            });
        }
        segs.push(seg(pt(1.0), [1.0, -1.0]));
        let v = loop_json(segs, true);
        let edges = v["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 3, "{v}");
        let arc = edges.iter().find(|e| e["kind"] == "arc").unwrap();
        assert_eq!(arc["ccw"], true);
        assert_eq!(arc["start"], json!([1.0, 0.0]));
    }
}
