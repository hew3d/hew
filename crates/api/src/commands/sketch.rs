//! Drawing commands: hew.sketch.* (docs/HEW_API.md §7, §7 semantics
//! notes; the kernel-surface recipes are api-kernel-map.md §1.1/§1.2).
//!
//! Every drawing command resolves a plane spec (docs/HEW_API.md §7) into
//! one of three targets: a fresh sketch (`{"ground": true}` /
//! `{origin, normal}`), an existing one (`{"sketch": id}`), or a solid's
//! face (`{"face": <locator>}` — the face-imprint path, drawing directly
//! on a face instead of into a sketch). The first two bracket their
//! edits in exactly one [`kernel::Document::begin_sketch_gesture`] /
//! `end_sketch_gesture` pair — the only way a sketch mutation becomes
//! undo-visible (api-kernel-map.md §1.1, gap 7) — and return
//! `{sketch, region_ids, …}`. The face-imprint path never touches a
//! sketch at all: it goes straight through
//! [`kernel::Document::apply_object_op`] with `KernelOp::SplitFace`
//! (an open, boundary-to-boundary cut — `draw_line`, and `draw_arc` when
//! it does not close on itself) or `KernelOp::SplitFaceInner` (a closed
//! loop strictly inside the face — `draw_rect`/`draw_circle`/
//! `draw_polygon`, and `draw_arc` when its sweep is a full turn),
//! returning `{object_id}` plus transaction-scoped face tokens (§5.2,
//! §5.4) — a solid's faces have no public id of their own, so tokens are
//! the only way a later command in the same transaction can address
//! what this one just cut. A face resolving to a component-DEFINITION
//! member routes through [`kernel::Document::apply_def_op`] instead of
//! `apply_object_op` (same `KernelOp` construction, def-scoped) — the
//! edit lands on the shared definition, so every instance of the
//! component picks it up at once (see [`apply_face_op`]).
//!
//! Coordinate frame (v1, deliberate, not a gap): a face locator (§5.2)
//! always names an OBJECT (`{object, at|ray}` or a `$face` token), never
//! an instance, so drawing on a definition member's face is always
//! addressed directly against that member — there is no "which
//! instance" for a locator to even carry. The points a caller supplies
//! are therefore always in the resolved object's own frame, full stop.
//! For a definition member that frame is the DEFINITION-local frame —
//! whatever frame the member's geometry is stored in, never remapped
//! through any instance's pose. For a definition minted in-session by
//! [`kernel::Document::make_component`] (or `place_text`) that frame
//! happens to equal the world frame at the moment of creation ("No
//! geometry moves") and never moves again; an IMPORTED definition keeps
//! whatever local frame it was authored in, which need not resemble any
//! world placement of its instances. Callers therefore address member
//! faces in the member's own stored coordinates (query the member's
//! geometry first when in doubt), not at an instance's world position.
//! Imprinting THROUGH a specific posed instance (mapping a world-space
//! gesture back into definition space via that instance's pose, the way
//! `split_face_inner_in_instance` and the UI's in-instance face tools
//! do) is simply not expressible at v1: it would need a locator shape
//! that names an instance, which does not exist yet. Adding one is
//! additive future work, not a behavior change here.
//!
//! There are no circle/rect/arc/polygon primitives in the kernel — every
//! shape is faceted here into a chain of `Sketch::add_segment` calls (or,
//! on a face, a `Vec<Point3>` path/loop), circles/arcs/polygons in
//! sketch mode additionally bracketed in a `begin_curve_with[_kind]` …
//! `end_curve` run so the facets carry their analytic ancestry
//! (api-kernel-map.md §1.2); on a face the same analytic claim travels
//! as `SplitFaceInner`'s `curve` field instead.

use super::{CmdError, Ctx, Handler};
use crate::geom;
use crate::ids;
use crate::locate::{self, FaceRef, PlaneTarget};
use crate::refusal::Refusal;
use kernel::{
    CurveGeom, EntityRef, KernelOp, KernelOpReport, ObjectId, Plane, Point3, SketchCurveId,
    SketchCurveKind, SketchId, SketchRegionId, Vec3,
};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.sketch.draw_line" => draw_line,
        "hew.sketch.draw_rect" => draw_rect,
        "hew.sketch.draw_circle" => draw_circle,
        "hew.sketch.draw_arc" => draw_arc,
        "hew.sketch.draw_polygon" => draw_polygon,
        "hew.sketch.offset" => offset,
        _ => return None,
    })
}

// ------------------------------------------------------------- plumbing

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

fn unknown_entity(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_entity",
            &format!("'{id}' does not name a live entity of the required kind in this document."),
        )
        .with_detail(serde_json::json!({ "id": id })),
    )
}

fn sketch_err(e: kernel::SketchError) -> CmdError {
    CmdError::from(kernel::DocumentError::Sketch(e))
}

/// Commits a resolved plane spec (docs/HEW_API.md §7) to a concrete
/// sketch: mints a fresh one for `NewPlane` (the first gesture folds its
/// creation into one undo step — api-kernel-map.md §1.1), looks up an
/// existing one for `Sketch`. Callers branch on `PlaneTarget::Face`
/// themselves, before this ever runs — the face-imprint path never
/// touches a sketch (see the module doc comment), so reaching this arm
/// with a `Face` target is a caller bug, not a user-facing refusal.
fn commit_target(ctx: &mut Ctx, target: PlaneTarget) -> Result<(SketchId, Plane), CmdError> {
    match target {
        PlaneTarget::NewPlane(plane) => {
            let sketch = ctx.doc.add_sketch(plane);
            Ok((sketch, plane))
        }
        PlaneTarget::Sketch(id) => {
            let plane = ctx
                .doc
                .sketch(id)
                .ok_or_else(|| {
                    CmdError::Internal("sketch vanished between resolve and commit".into())
                })?
                .plane();
            Ok((id, plane))
        }
        PlaneTarget::Face(_) => Err(CmdError::Internal(
            "commit_target called with a Face plane target — the caller should have routed \
             through the face-imprint path instead"
                .into(),
        )),
    }
}

// --------------------------------------------------- face-imprint plumbing

/// Applies a face-imprint `KernelOp` (`SplitFace`/`SplitFaceInner`) to
/// `object`, routing through the shared-definition path
/// ([`kernel::Document::apply_def_op`]) when `object` is a
/// component-definition member, so every instance of the component
/// picks up the cut at once — otherwise the plain world path
/// ([`kernel::Document::apply_object_op`]). Both take the identical
/// `KernelOp`; only the document method (and which entities a
/// `DocChange` names) differs (api-kernel-map.md §1.3). See the module
/// doc comment for the coordinate-frame decision this deliberately does
/// NOT do (no world→def-local remap through any instance pose).
fn apply_face_op(
    ctx: &mut Ctx,
    object: ObjectId,
    op: KernelOp,
) -> Result<KernelOpReport, CmdError> {
    let (report, _change) = match ctx.doc.object_owner_component(object) {
        Some(component) => ctx.doc.apply_def_op(component, object, op)?,
        None => ctx.doc.apply_object_op(object, op)?,
    };
    Ok(report)
}

/// A resolved face's supporting plane — the face-imprint analog of a
/// sketch's `plane()`, used to build the same in-plane (u, v) basis
/// [`plane_basis`] gives sketch mode.
fn face_plane(ctx: &Ctx, face: FaceRef) -> Result<Plane, CmdError> {
    let object = ctx.doc.object(face.object).ok_or_else(|| {
        CmdError::Internal("object vanished between face resolution and use".into())
    })?;
    let f = object
        .faces()
        .get(face.face)
        .ok_or_else(|| CmdError::Internal("face vanished between resolution and use".into()))?;
    Ok(f.plane)
}

/// The `loop_not_strictly_inside` refusal, built the same way any other
/// kernel-sourced refusal is (`Refusal::from_document_error`) so its name
/// and UI copy stay identical to the kernel's own — this is the API
/// minting the SAME refusal the kernel would eventually raise itself,
/// just earlier and more reliably (see [`require_strictly_inside_face`]).
fn loop_not_strictly_inside_early() -> CmdError {
    CmdError::from(kernel::DocumentError::Op(kernel::KernelOpError::Sticky(
        kernel::StickyError::LoopNotStrictlyInside { index: 0 },
    )))
}

/// Every point in a would-be face-imprint loop is strictly inside the
/// target face — clear of its outer boundary and every hole, not merely
/// on the correct side of it.
///
/// This exists because the kernel's own equivalent check
/// (`split_face_inner_impl`'s per-vertex `point_inside_polygon` gate,
/// `ops.rs`) is an unguarded even-odd ray cast with no tolerance band: a
/// point landing within a few ULPs of a boundary edge — exactly what
/// happens when a caller-supplied point (`draw_arc`'s `close: "pie"`
/// center, say) sits exactly ON the face's boundary — resolves to inside
/// or outside by whichever way two independently rounded floating-point
/// computations happen to fall, not by geometry. Left unguarded, that can
/// let a hole loop through that touches the parent face's outer loop at a
/// vertex: pinched, degenerate topology committed as a false "Ok".
///
/// The fix is a distance-to-boundary guard ahead of the ordinary
/// containment test: a point within [`geom::API_SURFACE_TOL`] of any
/// edge of the outer loop or a hole counts as "on" it (the same
/// tolerance — and the same "on a surface" concept — `locate::resolve_face`
/// already uses for point-based face resolution), refused before it ever
/// reaches the kernel. Uses [`geom::face_contains`] for the ordinary
/// inside/outside half of the test and `crate::locate::face_rings` to
/// fetch the face's loops — both already-public, already-used API
/// helpers; nothing kernel-side changes.
fn require_strictly_inside_face(
    ctx: &Ctx,
    face: FaceRef,
    normal: Vec3,
    pts: &[Point3],
) -> Result<(), CmdError> {
    let (outer, holes) = locate::face_rings(ctx, face)?;
    let near_ring = |ring: &[Point3], p: Point3| {
        let n = ring.len();
        (0..n).any(|i| {
            geom::point_segment_distance(p, ring[i], ring[(i + 1) % n]).0 <= geom::API_SURFACE_TOL
        })
    };
    for &p in pts {
        let strictly_inside = !near_ring(&outer, p)
            && !holes.iter().any(|h| near_ring(h, p))
            && geom::face_contains(&outer, &holes, normal, p);
        if !strictly_inside {
            return Err(loop_not_strictly_inside_early());
        }
    }
    Ok(())
}

/// The face-imprint creation result (docs/HEW_API.md §5.4): a solid's
/// faces have no public id of their own (§5.2), so — unlike sketch
/// mode's `{sketch, region_ids, …}` — this is just the object's public
/// id; whichever faces the cut produced are addressed through the face
/// tokens the caller mints alongside this.
fn face_imprint_result(ctx: &Ctx, object: ObjectId) -> Value {
    serde_json::json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) })
}

fn public_id_of(ctx: &Ctx, entity: EntityRef) -> String {
    let sid = ctx
        .doc
        .sid_of(&entity)
        .expect("an object this command just touched always carries a stable id");
    ids::public_id(&entity, sid)
}

/// Face-imprint path for an OPEN, boundary-to-boundary path: `draw_line`
/// always, `draw_arc` when its sweep does not close a full turn
/// (api-kernel-map.md §1.2's per-tool table: Line/Arc face mode →
/// `split_face`). Mints two face tokens, `"a"`/`"b"`, naming the two
/// faces the cut produced — cheap, since
/// [`kernel::FaceSplitReport::new_faces`] names them directly.
fn draw_chain_on_face(ctx: &mut Ctx, face: FaceRef, path: Vec<Point3>) -> Result<Value, CmdError> {
    let op = KernelOp::SplitFace {
        face: face.face,
        path,
        restore: None,
    };
    let report = apply_face_op(ctx, face.object, op)?;
    let KernelOpReport::FaceSplit(r) = report else {
        return Err(CmdError::Internal(
            "apply_object_op(SplitFace) returned an unexpected report kind".into(),
        ));
    };
    ctx.mint_face_token("a", face.object, r.new_faces[0]);
    ctx.mint_face_token("b", face.object, r.new_faces[1]);
    Ok(face_imprint_result(ctx, face.object))
}

/// Face-imprint path for a CLOSED loop strictly inside the face:
/// `draw_rect`/`draw_circle`/`draw_polygon` always, `draw_arc` when its
/// sweep closes a full turn (api-kernel-map.md §1.2's per-tool table).
/// `curve` carries the drawn circle's analytic identity (`Some`) so a
/// later push-through re-attributes the tunnel walls as a cylinder
/// (mirrors [`kernel::Document`]'s `split_face_inner_with_curve`
/// wasm-side counterpart); `None` for rect/polygon/open-turn arcs, which
/// have no such claim. Mints `"face"` (the new sub-face) and `"parent"`
/// (the reshaped parent, now carrying the loop as a hole) — both directly
/// named by [`kernel::FaceSplitInnerReport`], so equally cheap to mint.
fn draw_loop_on_face(
    ctx: &mut Ctx,
    face: FaceRef,
    loop_path: Vec<Point3>,
    curve: Option<CurveGeom>,
) -> Result<Value, CmdError> {
    let op = KernelOp::SplitFaceInner {
        face: face.face,
        loop_path,
        restore: None,
        curve,
    };
    let report = apply_face_op(ctx, face.object, op)?;
    let KernelOpReport::FaceSplitInner(r) = report else {
        return Err(CmdError::Internal(
            "apply_object_op(SplitFaceInner) returned an unexpected report kind".into(),
        ));
    };
    ctx.mint_face_token("face", face.object, r.sub_face);
    ctx.mint_face_token("parent", face.object, r.parent);
    Ok(face_imprint_result(ctx, face.object))
}

/// `n` evenly spaced points around a circle's circumference in `plane`'s
/// own (u, v) frame — the closed vertex ring `draw_circle`/`draw_polygon`
/// face mode imprints as a loop, and (chorded pairwise) the same
/// construction plane mode facets into `n` `add_segment` segments.
fn ring_points(plane: &Plane, center: Point3, radius: f64, n: usize) -> Vec<Point3> {
    let (u, v) = plane_basis(plane);
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            center + u * (radius * a.cos()) + v * (radius * a.sin())
        })
        .collect()
}

/// `n + 1` points tracing an arc's sweep from `start_angle` in `plane`'s
/// own (u, v) frame — consecutive pairs are `draw_arc`'s `n` chords in
/// plane mode, and the whole vector is the face-mode path/loop.
fn arc_points(
    plane: &Plane,
    center: Point3,
    radius: f64,
    start_angle: f64,
    sweep: f64,
    n: usize,
) -> Vec<Point3> {
    let (u, v) = plane_basis(plane);
    (0..=n)
        .map(|i| {
            let a = start_angle + sweep * i as f64 / n as f64;
            center + u * (radius * a.cos()) + v * (radius * a.sin())
        })
        .collect()
}

/// The other two corners of an axis-aligned-in-plane rectangle whose
/// `a`/`b` are two OPPOSITE corners, built by mixing their (u, v)
/// components in `plane`'s own frame — on-plane by construction; `a`/`b`
/// themselves pass through untouched, so an off-plane input still
/// surfaces the kernel's own refusal rather than being silently
/// projected (docs/HEW_API.md §7 semantics notes). Returns all four
/// corners in loop order, NOT closed back to `a` — plane mode appends
/// that closing repeat itself for its `add_segment` chain; a face-mode
/// loop path takes the four unique vertices as-is.
fn rect_corners(plane: &Plane, a: Point3, b: Point3) -> [Point3; 4] {
    let (u, v) = plane_basis(plane);
    let origin = plane.point();
    let ua = (a - origin).dot(u);
    let va = (a - origin).dot(v);
    let ub = (b - origin).dot(u);
    let vb = (b - origin).dot(v);
    let p2 = origin + u * ub + v * va;
    let p4 = origin + u * ua + v * vb;
    [a, p2, b, p4]
}

/// An orthonormal in-plane basis for `plane`, deterministic and
/// world-axis-aligned where possible: for the ground plane (normal +Z)
/// this is exactly world X/Y. `reference` is world X unless the plane's
/// normal is too close to it, in which case world Y — Gram-Schmidt
/// against whichever reference is farther from parallel.
fn plane_basis(plane: &Plane) -> (Vec3, Vec3) {
    let n = plane.normal();
    let reference = if n.x.abs() > 0.9 {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = (reference - n * reference.dot(n))
        .normalized()
        .expect("reference chosen not parallel to the plane normal");
    let v = n.cross(u);
    (u, v)
}

/// Nets a run of [`kernel::SegmentAdded`] reports down to the regions
/// that exist now and did not before the command started — a region
/// created then invalidated by a later segment in the same command never
/// appears (docs/HEW_API.md §5.4: "which faces/regions" is normative,
/// not a raw dump of every intermediate report).
fn net_created_regions(reports: &[kernel::SegmentAdded]) -> Vec<SketchRegionId> {
    let mut created: Vec<SketchRegionId> = Vec::new();
    for r in reports {
        created.retain(|c| !r.regions_removed.contains(c));
        for &c in &r.regions_created {
            if !created.contains(&c) {
                created.push(c);
            }
        }
    }
    created
}

/// The creation-result shape (docs/HEW_API.md §5.4, §6.1): the sketch's
/// public id, the honest full `region_ids` list, plus a convenience
/// singular `region_id` when exactly one region resulted — the §6.1
/// example chains `draw_circle` into `extrude` through exactly that
/// field. `curve_id` is present when this command began a curve chain.
fn draw_result(
    ctx: &Ctx,
    sketch: SketchId,
    regions: &[SketchRegionId],
    curve: Option<SketchCurveId>,
) -> Value {
    let sketch_sid = ctx
        .doc
        .sid_of(&EntityRef::Sketch(sketch))
        .expect("a sketch this command touched always carries a stable id");
    let region_ids: Vec<String> = regions
        .iter()
        .map(|&r| ids::region_id(sketch_sid, r))
        .collect();
    let mut out = serde_json::json!({
        "sketch": ids::public_id(&EntityRef::Sketch(sketch), sketch_sid),
        "region_ids": region_ids,
    });
    if region_ids.len() == 1 {
        out["region_id"] = Value::String(region_ids[0].clone());
    }
    if let Some(c) = curve {
        out["curve_id"] = Value::String(ids::curve_id(sketch_sid, c));
    }
    out
}

// -------------------------------------------------------------- commands

pub(super) fn draw_line(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        points: Vec<Value>,
    }
    let p: P = parse_params(params)?;
    if p.points.len() < 2 {
        return Err(CmdError::Params("draw_line needs at least 2 points".into()));
    }
    let target = locate::resolve_plane(ctx, &p.plane)?;
    let pts: Vec<Point3> = p
        .points
        .iter()
        .map(|v| locate::resolve_point(ctx, v))
        .collect::<Result<_, _>>()?;

    if let PlaneTarget::Face(face_ref) = target {
        return draw_chain_on_face(ctx, face_ref, pts);
    }
    let (sketch, _plane) = commit_target(ctx, target)?;

    let reports = super::run_sketch_gesture(ctx, sketch, |s| {
        let mut reports = Vec::with_capacity(pts.len() - 1);
        for w in pts.windows(2) {
            reports.push(s.add_segment(w[0], w[1]).map_err(sketch_err)?);
        }
        Ok(reports)
    })?;

    let regions = net_created_regions(&reports);
    Ok(draw_result(ctx, sketch, &regions, None))
}

pub(super) fn draw_rect(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        corner_a: Value,
        corner_b: Value,
    }
    let p: P = parse_params(params)?;
    let target = locate::resolve_plane(ctx, &p.plane)?;
    let a = locate::resolve_point(ctx, &p.corner_a)?;
    let b = locate::resolve_point(ctx, &p.corner_b)?;

    if let PlaneTarget::Face(face_ref) = target {
        let plane = face_plane(ctx, face_ref)?;
        let corners = rect_corners(&plane, a, b);
        return draw_loop_on_face(ctx, face_ref, corners.to_vec(), None);
    }
    let (sketch, plane) = commit_target(ctx, target)?;
    let [a, p2, b, p4] = rect_corners(&plane, a, b);
    let loop_pts = [a, p2, b, p4, a];

    let reports = super::run_sketch_gesture(ctx, sketch, |s| {
        let mut reports = Vec::with_capacity(4);
        for w in loop_pts.windows(2) {
            reports.push(s.add_segment(w[0], w[1]).map_err(sketch_err)?);
        }
        Ok(reports)
    })?;

    let regions = net_created_regions(&reports);
    Ok(draw_result(ctx, sketch, &regions, None))
}

pub(super) fn draw_circle(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        center: Value,
        radius: f64,
        #[serde(default)]
        segments: Option<usize>,
    }
    let p: P = parse_params(params)?;
    let n = match p.segments {
        Some(n) if n < kernel::MIN_CIRCLE_SEGMENTS => {
            return Err(CmdError::Refusal(Refusal::api(
                "segments_below_floor",
                "A circle needs at least 24 segments — below that it stops being a circle and becomes a polygon. Use the Polygon tool for a coarser shape.",
            )));
        }
        Some(n) if n > kernel::MAX_CIRCLE_SEGMENTS => {
            return Err(CmdError::Refusal(Refusal::api(
                "segments_above_cap",
                "That is more segments than a circle can hold. Enter a smaller count.",
            )));
        }
        Some(n) => n,
        None => 48,
    };
    let target = locate::resolve_plane(ctx, &p.plane)?;
    let center = locate::resolve_point(ctx, &p.center)?;
    let geom = CurveGeom {
        center,
        radius: p.radius,
    };

    if let PlaneTarget::Face(face_ref) = target {
        let plane = face_plane(ctx, face_ref)?;
        let ring = ring_points(&plane, center, p.radius, n);
        return draw_loop_on_face(ctx, face_ref, ring, Some(geom));
    }
    let (sketch, plane) = commit_target(ctx, target)?;
    let ring = ring_points(&plane, center, p.radius, n);

    let (curve, reports) = super::run_sketch_gesture(ctx, sketch, |s| {
        let curve = s.begin_curve_with(geom).map_err(sketch_err)?;
        let mut reports = Vec::with_capacity(n);
        for i in 0..n {
            reports.push(
                s.add_segment(ring[i], ring[(i + 1) % n])
                    .map_err(sketch_err)?,
            );
        }
        s.end_curve();
        Ok((curve, reports))
    })?;

    let regions = net_created_regions(&reports);
    Ok(draw_result(ctx, sketch, &regions, Some(curve)))
}

/// How a drawn arc's ends are closed (mirrors the UI Arc tool's Alt-cycled
/// completion modes, `app/src/tools/ArcTool.ts`'s `ArcCompletion`): `Open`
/// is the bare arc (today's only behavior before this param existed);
/// `Pie` closes it to the center with two radii (a closed wedge); `Segment`
/// closes it with the chord (a closed circular segment). `Pie`/`Segment`
/// both commit a closed profile — a region in plane/sketch mode, a
/// `SplitFaceInner` loop imprint in face mode — exactly like
/// `draw_rect`/`draw_circle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArcClose {
    Open,
    Pie,
    Segment,
}

impl ArcClose {
    fn parse(raw: Option<&str>) -> Result<ArcClose, CmdError> {
        match raw {
            None | Some("open") => Ok(ArcClose::Open),
            Some("pie") => Ok(ArcClose::Pie),
            Some("segment") => Ok(ArcClose::Segment),
            Some(other) => Err(CmdError::Params(format!(
                "close must be \"open\", \"pie\", or \"segment\" (got \"{other}\")"
            ))),
        }
    }
}

pub(super) fn draw_arc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        center: Value,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        #[serde(default)]
        segments: Option<usize>,
        #[serde(default)]
        close: Option<String>,
    }
    let p: P = parse_params(params)?;
    let close = ArcClose::parse(p.close.as_deref())?;
    let sweep = p.end_angle - p.start_angle;
    if sweep == 0.0 {
        return Err(CmdError::Params(
            "start_angle and end_angle must differ".into(),
        ));
    }
    // A sweep beyond one full turn is degenerate — its chords would wrap
    // and cross each other pairwise (hanging the sketch's intersection
    // graph), and the shape it means is a circle. Refused, never clamped.
    if sweep.abs() > std::f64::consts::TAU + 1e-9 {
        return Err(CmdError::Params(
            "arc sweep exceeds a full turn — draw a circle instead".into(),
        ));
    }
    // A sweep of exactly one full turn IS a closed loop — geometrically a
    // circle, just drawn through the arc command — so its face-mode path
    // mirrors `draw_circle`'s (`SplitFaceInner`, carrying the curve
    // claim). Anything short of a full turn is the open "D" chain
    // `draw_line`'s face mode already handles (`SplitFace`), which is what
    // an arc that does not close on itself actually is (api-kernel-map.md
    // §1.2's per-tool table: Arc face mode → `split_face` / `split_face_inner`).
    let closes_a_full_turn = (sweep.abs() - std::f64::consts::TAU).abs() <= 1e-9;
    // A full-turn sweep is already a closed loop (the circle special
    // case below) — `close: "pie"`/`"segment"` would mean closing an
    // already-closed shape a second time, which names no additional
    // geometry. Refused as a static parameter conflict, the same
    // register as the sweep-range checks above. Checked before `segments`
    // below so this — the more fundamental conflict — is what a caller
    // sees when both are wrong at once.
    if closes_a_full_turn && close != ArcClose::Open {
        return Err(CmdError::Params(
            "close must be \"open\" for a full-turn sweep — it already forms a closed loop".into(),
        ));
    }
    // An arc is not gated by the circle density floor (a two-point "D"
    // shape is a legitimate one-segment OPEN arc); it shares only the
    // cap, since an absurdly dense arc is the same runaway-facet-count
    // concern a circle has. `close: "pie"`/`"segment"` needs a floor of
    // its own, though: with a single chord, `"segment"`'s implicit
    // closing edge (arc-end → arc-start) retraces that SAME chord —
    // a 2-point "loop" with no area at all, for any sweep — and
    // `"pie"`'s loop (arc-start, arc-end, center) degenerates to a
    // zero-area sliver whenever the center lands on the chord's own line
    // (exactly a half-turn sweep, but every single-chord pie is one
    // sweep away from it). `polygon_is_simple` is vacuous for a
    // 2-or-3-point "polygon" (there are no non-adjacent edge pairs to
    // cross), and nothing else checks loop area, so neither shape is
    // caught downstream. Two chords fixes both at once: they put a THIRD
    // point strictly between the arc's ends, and no two distinct points
    // on a circle plus that circle's own center can ever be collinear
    // with a third distinct point of the same circle — so a closed
    // loop's area is provably nonzero for every sweep once it has at
    // least this many chords. `MIN_CLOSED_ARC_SEGMENTS` is that floor,
    // applied identically to the default AND to an explicit `segments`
    // — sketch mode and face mode share this one `n` computation, so the
    // two paths agree by construction.
    const MIN_CLOSED_ARC_SEGMENTS: usize = 2;
    let min_segments = if close == ArcClose::Open {
        1
    } else {
        MIN_CLOSED_ARC_SEGMENTS
    };
    let n = match p.segments {
        Some(n) if n < min_segments => {
            return Err(CmdError::Params(if close == ArcClose::Open {
                "an arc needs at least 1 segment".into()
            } else {
                format!(
                    "close: \"pie\"/\"segment\" needs at least {MIN_CLOSED_ARC_SEGMENTS} segments — \
                     a single chord's closing edge can't form a non-degenerate closed loop (got {n})"
                )
            }));
        }
        Some(n) if n > kernel::MAX_CIRCLE_SEGMENTS => {
            return Err(CmdError::Refusal(Refusal::api(
                "segments_above_cap",
                "That is more segments than a circle can hold. Enter a smaller count.",
            )));
        }
        Some(n) => n,
        // The default is capped exactly like an explicit count: a huge
        // sweep must never turn into an unbounded facet loop. Floored at
        // `min_segments` so a small-sweep `"pie"`/`"segment"` (whose
        // proportional density would otherwise round down to 1 chord —
        // any sweep under ~11°) never silently produces the same
        // degenerate loop an explicit `segments: 1` would.
        None => ((48.0 * sweep.abs() / std::f64::consts::TAU)
            .round()
            .max(min_segments as f64) as usize)
            .min(kernel::MAX_CIRCLE_SEGMENTS),
    };
    let target = locate::resolve_plane(ctx, &p.plane)?;
    let center = locate::resolve_point(ctx, &p.center)?;
    let geom = CurveGeom {
        center,
        radius: p.radius,
    };

    if let PlaneTarget::Face(face_ref) = target {
        let plane = face_plane(ctx, face_ref)?;
        let mut pts = arc_points(&plane, center, p.radius, p.start_angle, sweep, n);
        if closes_a_full_turn {
            pts.pop(); // last point coincides with the first at a full turn
            return draw_loop_on_face(ctx, face_ref, pts, Some(geom));
        }
        return match close {
            // Open, boundary-to-boundary — unchanged from before this
            // param existed.
            ArcClose::Open => draw_chain_on_face(ctx, face_ref, pts),
            // A closed loop strictly inside the face — `draw_loop_on_face`
            // closes the implicit last→first edge for us, which for a
            // segment IS the chord (arc-end → arc-start). No analytic
            // curve claim travels: a partial arc's chords are not a full
            // circle (matches the open-arc case's own `None`).
            ArcClose::Segment => {
                require_strictly_inside_face(ctx, face_ref, plane.normal(), &pts)?;
                draw_loop_on_face(ctx, face_ref, pts, None)
            }
            // Same, plus the two explicit radii: append the exact same
            // `center` value used to build the arc's own points (no
            // recomputation drift), so the implicit closing edge
            // (center → arc-start) plus the appended edge (arc-end →
            // center) form the wedge.
            ArcClose::Pie => {
                pts.push(center);
                // `center` is caller-supplied, unlike the arc's own
                // points — nothing upstream guarantees it is clear of
                // the face's own boundary. The kernel's own
                // `SplitFaceInner` validation would ordinarily be the
                // backstop for that (every loop vertex must sit
                // strictly inside), but its point-in-polygon test is a
                // bare even-odd ray cast with no tolerance band: a
                // center placed exactly ON the face's outer loop can
                // resolve to "inside" by whichever way two
                // independently rounded floats happen to fall, letting
                // a pinched, boundary-touching hole loop through as a
                // false "Ok". Checked here, before dispatch, with a
                // tolerance-aware test the kernel's has none of.
                require_strictly_inside_face(ctx, face_ref, plane.normal(), &pts)?;
                draw_loop_on_face(ctx, face_ref, pts, None)
            }
        };
    }
    let (sketch, plane) = commit_target(ctx, target)?;
    let pts = arc_points(&plane, center, p.radius, p.start_angle, sweep, n);

    let (curve, reports) = super::run_sketch_gesture(ctx, sketch, |s| {
        let curve = s.begin_curve_with(geom).map_err(sketch_err)?;
        let mut reports = Vec::with_capacity(n);
        for w in pts.windows(2) {
            reports.push(s.add_segment(w[0], w[1]).map_err(sketch_err)?);
        }
        s.end_curve();
        // The arc's own curve chain ends here — the closing edges
        // (pie/segment) are plain lines, not part of the analytic arc,
        // exactly mirroring ArcTool.ts's `curveSegments` split. Regions
        // close through the same sticky machinery `add_segment` always
        // uses; `net_created_regions` below nets it out like any other
        // chain.
        let first = pts[0];
        let last = pts[pts.len() - 1];
        match close {
            ArcClose::Open => {}
            ArcClose::Segment => {
                reports.push(s.add_segment(last, first).map_err(sketch_err)?);
            }
            ArcClose::Pie => {
                reports.push(s.add_segment(last, center).map_err(sketch_err)?);
                reports.push(s.add_segment(center, first).map_err(sketch_err)?);
            }
        }
        Ok((curve, reports))
    })?;

    let regions = net_created_regions(&reports);
    Ok(draw_result(ctx, sketch, &regions, Some(curve)))
}

pub(super) fn draw_polygon(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        plane: Value,
        center: Value,
        radius: f64,
        sides: usize,
    }
    let p: P = parse_params(params)?;
    if p.sides < 3 {
        return Err(CmdError::Params("a polygon needs at least 3 sides".into()));
    }
    let target = locate::resolve_plane(ctx, &p.plane)?;
    let center = locate::resolve_point(ctx, &p.center)?;
    let geom = CurveGeom {
        center,
        radius: p.radius,
    };
    let n = p.sides;

    if let PlaneTarget::Face(face_ref) = target {
        let plane = face_plane(ctx, face_ref)?;
        let ring = ring_points(&plane, center, p.radius, n);
        return draw_loop_on_face(ctx, face_ref, ring, None);
    }
    let (sketch, plane) = commit_target(ctx, target)?;
    let ring = ring_points(&plane, center, p.radius, n);

    let (curve, reports) = super::run_sketch_gesture(ctx, sketch, |s| {
        let curve = s
            .begin_curve_with_kind(geom, SketchCurveKind::Polygon)
            .map_err(sketch_err)?;
        let mut reports = Vec::with_capacity(n);
        for i in 0..n {
            reports.push(
                s.add_segment(ring[i], ring[(i + 1) % n])
                    .map_err(sketch_err)?,
            );
        }
        s.end_curve();
        Ok((curve, reports))
    })?;

    let regions = net_created_regions(&reports);
    Ok(draw_result(ctx, sketch, &regions, Some(curve)))
}

pub(super) fn offset(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        region: String,
        distance: f64,
    }
    let p: P = parse_params(params)?;
    let (sketch, region) = ctx
        .resolver()
        .resolve_region(&p.region)
        .ok_or_else(|| unknown_entity(&p.region))?;

    let report = super::run_sketch_gesture(ctx, sketch, |s| {
        s.offset_region(region, p.distance).map_err(sketch_err)
    })?;

    let sketch_sid = ctx
        .doc
        .sid_of(&EntityRef::Sketch(sketch))
        .expect("a sketch this command touched always carries a stable id");
    let region_ids: Vec<String> = report
        .regions_created
        .iter()
        .map(|&r| ids::region_id(sketch_sid, r))
        .collect();
    let curve_ids: Vec<String> = report
        .new_curves
        .iter()
        .map(|&c| ids::curve_id(sketch_sid, c))
        .collect();
    let mut out = serde_json::json!({
        "sketch": ids::public_id(&EntityRef::Sketch(sketch), sketch_sid),
        "region_ids": region_ids,
        "curve_ids": curve_ids,
    });
    if region_ids.len() == 1 {
        out["region_id"] = Value::String(region_ids[0].clone());
    }
    Ok(out)
}
