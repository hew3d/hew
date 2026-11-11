//! Solid operations: hew.solid.* (docs/agents/HEW_API.md §7, §7 semantics
//! notes; the kernel-surface recipes are api-kernel-map.md §1.3/§1.4).
//!
//! `push_pull` reproduces the wasm boundary's three-way branch exactly
//! (api-kernel-map.md §1.3, `Scene::push_pull`): a through push or pull
//! (past an opposing or co-facing wall) routes to
//! [`kernel::Document::push_pull_through`], a flat imprinted sub-face
//! routes to `KernelOp::ExtrudeSubFace`, everything else to
//! `KernelOp::PushPull` — both of the latter through
//! [`kernel::Document::apply_object_op`]. When the resolved face's
//! object is a component-DEFINITION member, each of these three routes
//! through its def-scoped sibling instead
//! ([`kernel::Document::push_pull_through_in_component`] /
//! [`kernel::Document::apply_def_op`]) — the shared-geometry edit is
//! then seen by every instance of the component at once, mirroring
//! `commands/sketch.rs`'s face-imprint routing (see that module's doc
//! comment for the coordinate-frame decision: always the definition's
//! own frame, never remapped through an instance's pose).
//!
//! **Editable imprints** (`imprints`, `move_imprint`, `rotate_imprint`,
//! `scale_imprint`, `delete_imprint`): kept for compatibility, implemented
//! over the drafting engine's plane units — `hew.sketch.units`/`move`/
//! `rotate`/`scale`/`delete` are the general form. An imprint is a drawn
//! unit on one of the object's drawing planes
//! ([`kernel::Document::object_drawing_planes`]): a *sub-face* is a region
//! over the object whose outer ring is wholly drawing edges and is a
//! closed loop nothing else meets or touches (a shape other drawing cuts is
//! listed as chords), a *chord* a
//! connected run of drawing edges anchored at both ends on the face's
//! boundary or on other drawing. Moving, turning, scaling, or deleting
//! either is one [`kernel::Document::draft`] over the unit's island (one
//! undo entry). `imprints` is `ReadOnly`; the other four are
//! `ModelMutating`. All four share one **imprint locator**: `{object, at}`
//! picks, on each drawing plane the point lies on, the chord whose line it
//! touches, else the sub-face containing it; `{object, ray}` and
//! `{"$face": …}` name a face and pick the sub-face over it. No face
//! tokens are minted by `imprints`.

use super::draft;
use super::{CmdError, Ctx, Handler};
use crate::geom;
use crate::locate;
use crate::refusal::Refusal;
use kernel::{
    DocumentError, DraftEdge, DraftStore, EntityRef, FollowMePath, KernelOp, NodeId, ObjectId,
    Plane, PlaneUnits, Point3, Transform, Vec3,
};
use serde_json::{Value, json};

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.solid.extrude" => extrude,
        "hew.solid.push_pull" => push_pull,
        "hew.solid.union" => union,
        "hew.solid.subtract" => subtract,
        "hew.solid.intersect" => intersect,
        "hew.solid.slice" => slice,
        "hew.solid.follow_me" => follow_me,
        "hew.solid.imprints" => imprints,
        "hew.solid.move_imprint" => move_imprint,
        "hew.solid.rotate_imprint" => rotate_imprint,
        "hew.solid.scale_imprint" => scale_imprint,
        "hew.solid.delete_imprint" => delete_imprint,
        "hew.solid.from_faces" => from_faces,
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

fn public_id_of(ctx: &Ctx, entity: EntityRef) -> String {
    let sid = ctx
        .doc
        .sid_of(&entity)
        .expect("an entity this command just touched always carries a stable id");
    crate::ids::public_id(&entity, sid)
}

/// A public id resolved to a tree node (Object/Group/Instance) — the
/// operand shape [`kernel::Document::boolean_nodes`] takes.
fn resolve_node(ctx: &Ctx, public: &str) -> Result<NodeId, CmdError> {
    match ctx.resolver().resolve(public) {
        Some(EntityRef::Object(id)) => Ok(NodeId::Object(id)),
        Some(EntityRef::Group(id)) => Ok(NodeId::Group(id)),
        Some(EntityRef::Instance(id)) => Ok(NodeId::Instance(id)),
        Some(_) => Err(CmdError::Params(format!(
            "'{public}' does not name an object, group, or instance"
        ))),
        None => Err(unknown_entity(public)),
    }
}

/// [`kernel::Document::apply_object_op`]/`push_pull_through` route every
/// [`kernel::PushPullError`] through `DocumentError::Op(KernelOpError::PushPull(_))`
/// — a SECOND level of delegation `Refusal::from_document_error`'s
/// variant-name extraction does not unwrap (it takes the leading
/// alphanumeric run of the *first* level's `Debug`, i.e. `KernelOpError`'s
/// own variant name "PushPull", not the `PushPullError` inside it). Left
/// alone, every push/pull refusal — `ObjectNotSolid`, `WouldVanish`,
/// `NonManifoldResult`, … — would surface under the same generic
/// `"push_pull"` machine name, losing exactly the distinction
/// docs/agents/HEW_API.md §4.4 promises. Unwrapped here, locally, one level
/// deeper, without touching the shared `refusal` module another wave
/// owns; `DocumentError`'s own `Display` (used for the explanation) was
/// never affected — only the machine `name` was.
// Push/pull refusals nest two levels deep (`Op(PushPull(..))`);
// `Refusal::from_document_error` unwraps them to the innermost name.
fn push_pull_refusal(e: DocumentError) -> CmdError {
    CmdError::Refusal(Refusal::from_document_error(&e))
}

// -------------------------------------------------------------- commands

pub(super) fn extrude(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        region: String,
        distance: f64,
        #[serde(default)]
        plane: Option<Value>,
    }
    let p: P = parse_params(params)?;
    if let Some(plane) = &p.plane {
        return extrude_plane_region(ctx, plane, &p.region, p.distance);
    }
    let (sketch, region) = ctx
        .resolver()
        .resolve_region(&p.region)
        .ok_or_else(|| unknown_entity(&p.region))?;
    extrude_sketch_region(ctx, sketch, region, p.distance)
}

/// `hew.solid.extrude { plane, region: "rgn_<hex16>", distance }`: the
/// region is one of the drafting plane's units (`hew.sketch.units`). A
/// region over the void extrudes as a new Object; a region that IS a face
/// of a solid push/pulls that face; a region over a face that cannot hold
/// it (a sliver kept as drawing) refuses `region_not_pushable`. With a
/// `{sketch: id}` spec on a detached sketch the regions are that sketch's
/// own, every one over no face: each extrudes as a new Object.
fn extrude_plane_region(
    ctx: &mut Ctx,
    plane_spec: &Value,
    region: &str,
    distance: f64,
) -> Result<Value, CmdError> {
    let (kind, key) = draft::parse_unit_key(region)?;
    if kind != draft::UnitKind::Region {
        return Err(draft::unknown_unit(region));
    }
    let dc = draft::resolve_context(ctx, plane_spec)?;
    let pu = draft::units_of(ctx, &dc)?;
    let r = pu
        .regions
        .iter()
        .find(|r| r.key == key)
        .ok_or_else(|| draft::unknown_unit(region))?;
    match (&r.extrude, &r.over) {
        (Some((sketch, region)), _) => extrude_sketch_region(ctx, *sketch, *region, distance),
        (None, Some((object, Some(face)))) => push_pull_face(
            ctx,
            locate::FaceRef {
                object: *object,
                face: *face,
            },
            distance,
        ),
        (None, Some((_, None))) => Err(draft::region_not_pushable()),
        (None, None) => Err(CmdError::Internal(
            "a plane region has neither a face nor a free sketch to extrude".into(),
        )),
    }
}

fn extrude_sketch_region(
    ctx: &mut Ctx,
    sketch: kernel::SketchId,
    region: kernel::SketchRegionId,
    distance: f64,
) -> Result<Value, CmdError> {
    let sketch_normal = ctx
        .doc
        .sketch(sketch)
        .ok_or_else(|| unknown_entity("region"))?
        .plane()
        .normal();

    let (object, _change) = ctx.doc.extrude_region(sketch, region, distance)?;

    // Face tokens (docs/agents/HEW_API.md §5.4, normative here): the cap facing
    // the sketch's own normal is "top", the opposite cap "base", the
    // rest "side.<n>" in face-iteration order. Defined by alignment with
    // the ORIGINAL sketch normal, not by the sign of `distance` — a
    // negative-distance extrude (sweeping opposite the sketch normal)
    // still names its caps this way, which is the simplest single rule
    // that needs no case split on sign; see the wave report for the
    // tradeoff this glosses over.
    let plan: Vec<(kernel::FaceId, String)> = {
        let obj = ctx
            .doc
            .object(object)
            .expect("the object extrude_region just created is live");
        let mut sides = 0usize;
        obj.faces()
            .iter()
            .map(|(fid, face)| {
                let dot = face.plane.normal().dot(sketch_normal);
                let key = if dot > 0.999 {
                    "top".to_string()
                } else if dot < -0.999 {
                    "base".to_string()
                } else {
                    sides += 1;
                    format!("side.{sides}")
                };
                (fid, key)
            })
            .collect()
    };
    for (fid, key) in plan {
        ctx.mint_face_token(&key, object, fid);
    }

    Ok(serde_json::json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

pub(super) fn push_pull(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        face: Value,
        distance: f64,
    }
    let p: P = parse_params(params)?;
    let face_ref = locate::resolve_face(ctx, &p.face)?;
    push_pull_face(ctx, face_ref, p.distance)
}

fn push_pull_face(
    ctx: &mut Ctx,
    face_ref: locate::FaceRef,
    distance: f64,
) -> Result<Value, CmdError> {
    // A face resolving to a component-DEFINITION member routes through
    // the def-scoped kernel methods below instead of the plain world
    // ones — same three-way branch, shared-geometry edit, so every
    // instance of the component picks it up at once (module doc
    // comment; mirrors `commands/sketch.rs`'s `apply_face_op`).
    let component = ctx.doc.object_owner_component(face_ref.object);

    let overshoots = ctx
        .doc
        .object(face_ref.object)
        .is_some_and(|o| o.push_pull_overshoots(face_ref.face, distance));
    if overshoots {
        let (objects, _change) = match component {
            Some(c) => ctx
                .doc
                .push_pull_through_in_component(c, face_ref.object, face_ref.face, distance)
                .map_err(push_pull_refusal)?,
            None => ctx
                .doc
                .push_pull_through(face_ref.object, face_ref.face, distance)
                .map_err(push_pull_refusal)?,
        };
        let object_ids: Vec<String> = objects
            .iter()
            .map(|&id| public_id_of(ctx, EntityRef::Object(id)))
            .collect();
        return Ok(serde_json::json!({ "object_ids": object_ids }));
    }

    let is_sub = ctx
        .doc
        .object(face_ref.object)
        .is_some_and(|o| o.is_flat_sub_face(face_ref.face));
    let op = if is_sub {
        KernelOp::ExtrudeSubFace {
            sub_face: face_ref.face,
            distance,
        }
    } else {
        KernelOp::PushPull {
            face: face_ref.face,
            distance,
        }
    };
    let (_report, _change) = match component {
        Some(c) => ctx
            .doc
            .apply_def_op(c, face_ref.object, op)
            .map_err(push_pull_refusal)?,
        None => ctx
            .doc
            .apply_object_op(face_ref.object, op)
            .map_err(push_pull_refusal)?,
    };

    Ok(serde_json::json!({ "object_id": public_id_of(ctx, EntityRef::Object(face_ref.object)) }))
}

fn boolean_op(ctx: &mut Ctx, params: &Value, op: kernel::BooleanOp) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        #[serde(default)]
        a: Option<String>,
        #[serde(default)]
        b: Option<String>,
        /// Every operand in order, two or more; replaces `a` and `b`.
        #[serde(default)]
        operands: Vec<String>,
    }
    let p: P = parse_params(params)?;
    let names: Vec<String> = if !p.operands.is_empty() {
        if p.a.is_some() || p.b.is_some() {
            return Err(CmdError::Params(
                "give either operands or a and b, not both".to_string(),
            ));
        }
        p.operands
    } else {
        match (p.a, p.b) {
            (Some(a), Some(b)) => vec![a, b],
            _ => {
                return Err(CmdError::Params(
                    "give a and b, or operands with two or more node ids".to_string(),
                ));
            }
        }
    };
    let nodes: Vec<NodeId> = names
        .iter()
        .map(|n| resolve_node(ctx, n))
        .collect::<Result<_, _>>()?;
    let (result, _change) = ctx.doc.boolean_nodes_many(op, &nodes)?;
    let entity = match result {
        NodeId::Object(id) => EntityRef::Object(id),
        NodeId::Group(id) => EntityRef::Group(id),
        NodeId::Instance(id) => EntityRef::Instance(id),
    };
    Ok(serde_json::json!({ "result": public_id_of(ctx, entity) }))
}

pub(super) fn union(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    boolean_op(ctx, params, kernel::BooleanOp::Union)
}

pub(super) fn subtract(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    boolean_op(ctx, params, kernel::BooleanOp::Subtract)
}

pub(super) fn intersect(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    boolean_op(ctx, params, kernel::BooleanOp::Intersect)
}

pub(super) fn slice(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PlaneSpec {
        origin: Value,
        normal: Value,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        plane: PlaneSpec,
    }
    let p: P = parse_params(params)?;
    let Some(EntityRef::Object(object)) = ctx.resolver().resolve(&p.object) else {
        return Err(unknown_entity(&p.object));
    };
    let origin = locate::resolve_point(ctx, &p.plane.origin)?;
    let normal = locate::parse_dir(&p.plane.normal)?;
    let plane = kernel::Plane::from_point_normal(origin, normal)
        .map_err(|_| CmdError::Params("degenerate plane normal".into()))?;

    let ((positive, negative), _change) = ctx.doc.slice_node(object, &plane)?;
    Ok(serde_json::json!({
        "positive": public_id_of(ctx, EntityRef::Object(positive)),
        "negative": public_id_of(ctx, EntityRef::Object(negative)),
    }))
}

/// How many open edges a `faces_not_closed` / `loop_not_planar` refusal
/// samples in its detail.
const OPEN_EDGE_SAMPLES: usize = 8;

/// `hew.solid.from_faces { sketches }` — Make Solid from drawn faces
/// (wireframe faces §6, §16 R26/R27): it first fills the faces the named
/// free sketches' lines can close (the close pass, as a draw runs it), then
/// runs the cells search seeded by every faced region of those sketches and
/// of the faces so filled ([`kernel::Document::make_solid_from_sketches`]),
/// one undo step; when no solid forms, nothing is kept. The first solid it makes answers `{object_id, grew}`
/// (`grew`: the shape closed against a solid and that solid grew; its new
/// id is `object_id`). Nothing made refuses, the document untouched:
/// `loop_not_planar` when a gap's rim is a closed loop off one plane,
/// else `faces_not_closed` with the open-edge count and up to eight sample
/// segments, else `faces_not_solid` (the faces close but build no solid:
/// `detail.reason` says why; for `overlapping_faces`, `detail.segments`
/// samples the edges two faces lie over each other along). Faces with the
/// same loop on one plane are one face (§16 R22), not an overlap.
pub(super) fn from_faces(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        sketches: Vec<String>,
    }
    let p: P = parse_params(params)?;
    if p.sketches.is_empty() {
        return Err(CmdError::Params("sketches needs at least one id".into()));
    }
    let resolver = ctx.resolver();
    let mut ids = Vec::with_capacity(p.sketches.len());
    for public in &p.sketches {
        match resolver.resolve(public) {
            Some(EntityRef::Sketch(s)) if ctx.doc.sketch(s).is_some() => ids.push(s),
            _ => return Err(unknown_entity(public)),
        }
    }
    let (outcomes, _change) = ctx
        .doc
        .make_solid_from_sketches(&ids)
        .map_err(|e| match e {
            DocumentError::UnknownSketch => unknown_entity(&p.sketches.join(", ")),
            other => CmdError::from(other),
        })?;
    if let Some(solid) = super::solid_of(ctx, &outcomes) {
        return Ok(solid);
    }
    // Nothing was made: say what is missing.
    let missing = ctx.doc.faces_open_edges(&ids);
    let samples: Vec<Value> = missing
        .open_edges
        .iter()
        .take(OPEN_EDGE_SAMPLES)
        .map(|&(a, b)| json!([point_json(a), point_json(b)]))
        .collect();
    let n = missing.count();
    // Faces lying over each other elsewhere in the same faces (§16 R22):
    // named beside the open edges or loops, so the caller learns of them
    // before those are drawn (`reason` and `overlapping_segments`, added to
    // the detail only when there are any).
    let overlap = |mut detail: Value| -> Value {
        if !missing.overlapping_edges.is_empty() {
            detail["reason"] = json!("overlapping_faces");
            detail["overlapping_segments"] = Value::Array(
                missing
                    .overlapping_edges
                    .iter()
                    .take(OPEN_EDGE_SAMPLES)
                    .map(|&(a, b)| json!([point_json(a), point_json(b)]))
                    .collect(),
            );
        }
        detail
    };
    if !missing.non_planar_loops.is_empty() {
        let loops: Vec<Value> = missing
            .non_planar_loops
            .iter()
            .map(|l| Value::Array(l.iter().map(|&q| point_json(q)).collect()))
            .collect();
        return Err(CmdError::Refusal(
            Refusal::api(
                "loop_not_planar",
                "The faces leave a gap whose edges do not lie on one plane, so no face can fill it. Draw a line across the gap so each part is flat, then try again.",
            )
            .with_detail(overlap(json!({
                "loops": loops,
                "open_edges": n,
                "segments": samples,
            }))),
        ));
    }
    let reason = outcomes.iter().find_map(|c| match c {
        kernel::CellOutcome::Declined(d) => match d.reason {
            kernel::CellDeclineReason::NotClosed => None,
            kernel::CellDeclineReason::OverlappingFaces => Some("overlapping_faces"),
            kernel::CellDeclineReason::InsideSolid(_) => Some("inside_solid"),
            kernel::CellDeclineReason::SelfTouching => Some("self_touching"),
            kernel::CellDeclineReason::Invalid(_) => Some("invalid"),
        },
        _ => None,
    });
    match reason {
        Some(reason) if n == 0 => {
            // Where faces lie over each other (§16 R22): up to eight
            // sample edges, as `faces_not_closed` gives its open edges.
            let mut detail = json!({ "reason": reason });
            if reason == "overlapping_faces" {
                detail["segments"] = Value::Array(
                    missing
                        .overlapping_edges
                        .iter()
                        .take(OPEN_EDGE_SAMPLES)
                        .map(|&(a, b)| json!([point_json(a), point_json(b)]))
                        .collect(),
                );
            }
            Err(CmdError::Refusal(
                Refusal::api(
                    "faces_not_solid",
                    match reason {
                        "overlapping_faces" => "Two faces lie on top of each other along the edges in detail.segments, so they enclose no single space. Erase one of them, then try again.",
                        "inside_solid" => "The closed shape lies inside the solid it was drawn against, so nothing is made from it.",
                        "self_touching" => "The faces enclose a space that touches itself across one face, so it builds no solid.",
                        _ => "The faces close, but they build no valid solid.",
                    },
                )
                .with_detail(detail),
            ))
        }
        _ => Err(CmdError::Refusal(
            Refusal::api(
                "faces_not_closed",
                &if n == 0 {
                    "These sketches hold no faces that enclose a space. Draw closed shapes that meet edge to edge, then try again.".to_string()
                } else {
                    format!(
                        "Not closed yet: {n} open edge{} remain{}. Draw the missing faces or lines, then try again.",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "s" } else { "" }
                    )
                },
            )
            .with_detail(overlap(json!({ "open_edges": n, "segments": samples }))),
        )),
    }
}

/// Follow Me has no definition-scoped kernel path yet (`follow_me_face`
/// is world-only), so a face resolving to a component-definition member
/// refuses TYPED — not `unknown_object`, which would falsely tell the
/// caller the id is stale when the very same locator succeeds in
/// `hew.solid.push_pull` and the `hew.sketch.draw_*` face modes.
fn require_world_follow_me_face(ctx: &Ctx, object: kernel::ObjectId) -> Result<(), CmdError> {
    if ctx.doc.object_owner_component(object).is_some() {
        return Err(CmdError::Refusal(Refusal::api(
            "follow_me_in_component_unsupported",
            "Follow Me on a face inside a component definition isn't supported yet — sweep a \
             world object's face instead, or explode the instance first.",
        )));
    }
    Ok(())
}

pub(super) fn follow_me(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        profile: Value,
        path: Value,
    }
    let p: P = parse_params(params)?;

    let path_obj = p
        .path
        .as_object()
        .ok_or_else(|| CmdError::Params("path needs an object".into()))?;
    let path = if let Some(face_locator) = path_obj.get("face") {
        if path_obj.len() != 1 {
            return Err(CmdError::Params("path.face takes no sibling keys".into()));
        }
        let f = locate::resolve_face(ctx, face_locator)?;
        require_world_follow_me_face(ctx, f.object)?;
        FollowMePath::FaceLoop {
            object: f.object,
            face: f.face,
        }
    } else if let Some(curve_val) = path_obj.get("curve") {
        if path_obj.len() != 1 {
            return Err(CmdError::Params("path.curve takes no sibling keys".into()));
        }
        let curve_pub = curve_val
            .as_str()
            .ok_or_else(|| CmdError::Params("path.curve must be a string".into()))?;
        let (sketch, curve) = ctx
            .resolver()
            .resolve_curve(curve_pub)
            .ok_or_else(|| unknown_entity(curve_pub))?;
        let edges = ctx
            .doc
            .sketch(sketch)
            .ok_or_else(|| unknown_entity(curve_pub))?
            .curve_edges(curve);
        FollowMePath::SketchEdges { sketch, edges }
    } else if path_obj.contains_key("edges") {
        // A solid-edge-chain path has no kernel mapping: FollowMePath's
        // edge-chain variant (SketchEdges) takes SKETCH edges, and a
        // resolved solid EdgeId cannot be turned into one — see the wave
        // report.
        return Err(CmdError::Refusal(Refusal::api(
            "unimplemented",
            "an explicit solid-edge-chain path lands after v0 — use path.face (a face's boundary loop) or path.curve (a sketch curve)",
        )));
    } else {
        return Err(CmdError::Params(
            "path needs \"edges\", \"face\", or \"curve\"".into(),
        ));
    };

    if let Some(profile_pub) = p.profile.as_str() {
        let (sketch, region) = ctx
            .resolver()
            .resolve_region(profile_pub)
            .ok_or_else(|| unknown_entity(profile_pub))?;
        let (object, _change) = ctx.doc.follow_me(sketch, region, &path)?;
        Ok(serde_json::json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
    } else if let Some(profile_obj) = p.profile.as_object() {
        let face_val = profile_obj
            .get("face")
            .ok_or_else(|| CmdError::Params("profile object needs \"face\"".into()))?;
        if profile_obj.len() != 1 {
            return Err(CmdError::Params(
                "profile.face takes no sibling keys".into(),
            ));
        }
        let f = locate::resolve_face(ctx, face_val)?;
        require_world_follow_me_face(ctx, f.object)?;
        let (object, _change) = ctx.doc.follow_me_face(f.object, f.face, &path, None)?;
        Ok(serde_json::json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
    } else {
        Err(CmdError::Params(
            "profile needs a region id or {\"face\": <locator>}".into(),
        ))
    }
}

// ------------------------------------------------------- editable imprints
// (module doc comment; docs/agents/HEW_API.md's `hew.solid` semantics notes)

fn point_json(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn finite_vec3(v: [f64; 3]) -> Result<Vec3, CmdError> {
    if v.iter().any(|c| !c.is_finite()) {
        return Err(CmdError::Params("offset components must be finite".into()));
    }
    Ok(Vec3::new(v[0], v[1], v[2]))
}

fn not_an_imprint() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "not_an_imprint",
        "That locator names neither a drawn shape (a region bounded entirely by lines drawn on a \
         face) nor a drawn line run (lines drawn up to a face's edge or to other drawing). Only \
         drawing can be moved, turned, scaled, or deleted this way.",
    ))
}

fn midpoint(a: Point3, b: Point3) -> Point3 {
    Point3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5)
}

fn key_hex(prefix: &str, key: u64) -> String {
    format!("{prefix}_{key:016x}")
}

/// An edge's geometry key on `plane` (the same key `hew.sketch.units` reports).
fn ekey(plane: &Plane, a: Point3, b: Point3) -> u64 {
    kernel::edge_key(plane, a, b)
}

/// Total order on points, so a run reads the same way every time.
fn point_cmp(a: Point3, b: Point3) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x)
        .then(a.y.total_cmp(&b.y))
        .then(a.z.total_cmp(&b.z))
}

fn same_point(a: Point3, b: Point3) -> bool {
    (a - b).length() <= geom::API_SURFACE_TOL
}

/// Orders the segments of one connected run (a simple path or a closed
/// loop) into a polyline. A path starts at its lexicographically smaller
/// end; a closed loop at its smallest vertex, toward its smaller neighbour.
/// Deterministic, so a chord and a curve run over the same edges agree.
fn order_chain(segs: &[(Point3, Point3)]) -> Vec<Point3> {
    let mut verts: Vec<Point3> = Vec::new();
    let mut deg: Vec<usize> = Vec::new();
    let idx = |p: Point3, verts: &mut Vec<Point3>, deg: &mut Vec<usize>| {
        let i = verts
            .iter()
            .position(|&q| same_point(p, q))
            .unwrap_or_else(|| {
                verts.push(p);
                deg.push(0);
                verts.len() - 1
            });
        deg[i] += 1;
        i
    };
    let edges: Vec<(usize, usize)> = segs
        .iter()
        .map(|&(a, b)| (idx(a, &mut verts, &mut deg), idx(b, &mut verts, &mut deg)))
        .collect();
    let ends: Vec<usize> = (0..verts.len()).filter(|&i| deg[i] == 1).collect();
    let start = ends
        .iter()
        .copied()
        .min_by(|&i, &j| point_cmp(verts[i], verts[j]))
        .or_else(|| (0..verts.len()).min_by(|&i, &j| point_cmp(verts[i], verts[j])));
    let Some(mut cur) = start else {
        return Vec::new();
    };
    let mut used = vec![false; edges.len()];
    let mut out = vec![verts[cur]];
    loop {
        let next = edges
            .iter()
            .enumerate()
            .filter(|&(k, &(a, b))| !used[k] && (a == cur || b == cur))
            .map(|(k, &(a, b))| (k, if a == cur { b } else { a }))
            .min_by(|x, y| point_cmp(verts[x.1], verts[y.1]));
        let Some((k, to)) = next else { break };
        used[k] = true;
        cur = to;
        out.push(verts[cur]);
    }
    out
}

fn run_at(path: &[Point3]) -> Point3 {
    match path {
        [a, b, ..] => midpoint(*a, *b),
        [a] => *a,
        [] => Point3::new(0.0, 0.0, 0.0),
    }
}

/// A region over the object whose outer ring is wholly drawing edges.
struct SubFaceImprint {
    region: kernel::DraftRegion,
    /// The island's index in the plane's units.
    unit: usize,
    /// Per outer-ring edge: the circle it is a facet of, if any.
    claims: Vec<Option<kernel::CurveGeom>>,
    face: Option<kernel::FaceId>,
}

/// A connected run of drawing edges anchored at both ends.
struct ChordImprint {
    path: Vec<Point3>,
    edges: Vec<u64>,
    unit: usize,
    claims: Vec<Option<kernel::CurveGeom>>,
}

/// One drawing plane of an object, analysed into its imprints.
struct PlaneImprints {
    plane: Plane,
    frame: Option<kernel::ComponentId>,
    objects: Vec<ObjectId>,
    pu: PlaneUnits,
    /// The unit indices (into `pu.units`) that hold this object's drawing.
    own_units: Vec<usize>,
    sub_faces: Vec<SubFaceImprint>,
    chords: Vec<ChordImprint>,
}

impl PlaneImprints {
    fn unit_keys(&self, unit: usize) -> Vec<u64> {
        self.pu.units[unit].edges.iter().map(|e| e.key).collect()
    }
}

fn owns(store: &DraftStore, object: ObjectId) -> bool {
    match *store {
        DraftStore::Topology(o) | DraftStore::Owned(o, _) => o == object,
        DraftStore::Free(_) => false,
    }
}

fn ring_pairs(ring: &[Point3]) -> Vec<(Point3, Point3)> {
    (0..ring.len())
        .map(|i| (ring[i], ring[(i + 1) % ring.len()]))
        .collect()
}

fn analyze_plane(ctx: &Ctx, object: ObjectId, plane: Plane) -> Result<PlaneImprints, CmdError> {
    let frame = ctx.doc.object_owner_component(object);
    let dc = draft::context_of(ctx, plane, frame, Some(object));
    let pu = ctx.doc.plane_units(plane, frame, &dc.objects)?;

    let own_units: Vec<usize> = pu
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.edges.iter().any(|e| owns(&e.store, object)))
        .map(|(i, _)| i)
        .collect();

    // key -> (unit, curve)
    let mut drawing: std::collections::BTreeMap<u64, (usize, Option<kernel::CurveGeom>)> =
        std::collections::BTreeMap::new();
    for &ui in &own_units {
        for e in &pu.units[ui].edges {
            drawing.insert(e.key, (ui, e.curve));
        }
    }

    // Solid boundary segments: ring edges of any region that are not drawing.
    let mut solid: Vec<(Point3, Point3)> = Vec::new();
    for r in &pu.regions {
        for ring in std::iter::once(&r.outer).chain(r.holes.iter()) {
            for (a, b) in ring_pairs(ring) {
                if !drawing.contains_key(&ekey(&plane, a, b)) {
                    solid.push((a, b));
                }
            }
        }
    }
    let on_solid = |p: Point3| {
        solid
            .iter()
            .any(|&(a, b)| geom::point_segment_distance(p, a, b).0 <= geom::API_SURFACE_TOL)
    };

    let degree_at = |p: Point3| {
        pu.units
            .iter()
            .flat_map(|u| u.edges.iter())
            .filter(|e| owns(&e.store, object))
            .map(|e| usize::from(same_point(e.a, p)) + usize::from(same_point(e.b, p)))
            .sum::<usize>()
    };
    let is_junction = |p: Point3| degree_at(p) >= 3 || on_solid(p);

    // Sub-faces: regions over the object bounded wholly by drawing.
    let mut sub_faces = Vec::new();
    for r in &pu.regions {
        let Some((o, face)) = r.over else { continue };
        if o != object || r.outer.len() < 3 {
            continue;
        }
        // A ring another line meets or the solid's boundary touches is cut
        // into runs (chords), not one shape.
        if r.outer.iter().any(|&q| is_junction(q)) {
            continue;
        }
        let pairs = ring_pairs(&r.outer);
        let found: Vec<Option<&(usize, Option<kernel::CurveGeom>)>> = pairs
            .iter()
            .map(|&(a, b)| drawing.get(&ekey(&plane, a, b)))
            .collect();
        if found.iter().any(Option::is_none) {
            continue;
        }
        let unit = found[0].map(|f| f.0).unwrap_or(0);
        sub_faces.push(SubFaceImprint {
            region: r.clone(),
            unit,
            claims: found.iter().map(|f| f.and_then(|f| f.1)).collect(),
            face,
        });
    }

    // Chords: maximal runs between nodes (a junction, an end, or a touch of
    // the solid's boundary), anchored at both ends.
    let mut chords = Vec::new();
    for &ui in &own_units {
        let edges: Vec<&DraftEdge> = pu.units[ui]
            .edges
            .iter()
            .filter(|e| owns(&e.store, object))
            .collect();
        let mut verts: Vec<Point3> = Vec::new();
        let mut inc: Vec<Vec<usize>> = Vec::new();
        let mut ends: Vec<(usize, usize)> = Vec::new();
        for (k, e) in edges.iter().enumerate() {
            let vi = |p: Point3, verts: &mut Vec<Point3>, inc: &mut Vec<Vec<usize>>| {
                let i = verts
                    .iter()
                    .position(|&q| same_point(p, q))
                    .unwrap_or_else(|| {
                        verts.push(p);
                        inc.push(Vec::new());
                        verts.len() - 1
                    });
                inc[i].push(k);
                i
            };
            let a = vi(e.a, &mut verts, &mut inc);
            let b = vi(e.b, &mut verts, &mut inc);
            ends.push((a, b));
        }
        let anchored: Vec<bool> = (0..verts.len())
            .map(|i| inc[i].len() >= 3 || on_solid(verts[i]))
            .collect();
        let node: Vec<bool> = (0..verts.len())
            .map(|i| inc[i].len() != 2 || anchored[i])
            .collect();
        let mut visited = vec![false; edges.len()];
        for start in 0..verts.len() {
            if !node[start] {
                continue;
            }
            for &first in &inc[start] {
                if visited[first] {
                    continue;
                }
                let mut run: Vec<usize> = Vec::new();
                let mut cur = start;
                let mut k = first;
                let end = loop {
                    visited[k] = true;
                    run.push(k);
                    let (a, b) = ends[k];
                    cur = if a == cur { b } else { a };
                    if node[cur] {
                        break cur;
                    }
                    match inc[cur].iter().copied().find(|&n| !visited[n]) {
                        Some(n) => k = n,
                        None => break cur,
                    }
                };
                if !(anchored[start] && anchored[end]) {
                    continue;
                }
                let segs: Vec<(Point3, Point3)> =
                    run.iter().map(|&k| (edges[k].a, edges[k].b)).collect();
                let path = order_chain(&segs);
                // Per segment of `path`, its edge's circle claim.
                let claims = path
                    .windows(2)
                    .map(|w| {
                        let key = ekey(&plane, w[0], w[1]);
                        edges.iter().find(|e| e.key == key).and_then(|e| e.curve)
                    })
                    .collect();
                chords.push(ChordImprint {
                    path,
                    edges: run.iter().map(|&k| edges[k].key).collect(),
                    unit: ui,
                    claims,
                });
            }
        }
    }

    Ok(PlaneImprints {
        plane,
        frame,
        objects: dc.objects,
        pu,
        own_units,
        sub_faces,
        chords,
    })
}

/// Every analysed drawing plane of `object`.
fn analyze_object(ctx: &Ctx, object: ObjectId) -> Result<Vec<PlaneImprints>, CmdError> {
    ctx.doc
        .object_drawing_planes(object)
        .into_iter()
        .map(|p| analyze_plane(ctx, object, p))
        .collect()
}

/// A resolved imprint: the drafting context and the drawing edges of its
/// island.
struct ImprintTarget {
    object: ObjectId,
    plane: Plane,
    frame: Option<kernel::ComponentId>,
    objects: Vec<ObjectId>,
    keys: Vec<u64>,
}

fn target_of(object: ObjectId, pi: &PlaneImprints, unit: usize) -> ImprintTarget {
    ImprintTarget {
        object,
        plane: pi.plane,
        frame: pi.frame,
        objects: pi.objects.clone(),
        keys: pi.unit_keys(unit),
    }
}

/// Resolves an **imprint locator** (module doc comment) through plane
/// units. `{object, at}`: the point picks, on each of the object's drawing
/// planes it lies on, the chord whose line it touches, else the sub-face
/// that contains it. `{object, ray}` / `{"$face": …}` name a face; the
/// sub-face over it is the imprint. A genuine identity failure (an unknown
/// object or face token) or a malformed locator surfaces as itself; a
/// locator that finds no drawing collapses to `not_an_imprint`.
fn resolve_imprint(ctx: &Ctx, locator: &Value) -> Result<ImprintTarget, CmdError> {
    let obj = locator
        .as_object()
        .ok_or_else(|| CmdError::Params("expected an imprint locator object".into()))?;
    if !obj.contains_key("$face")
        && let Some(at) = obj.get("at")
    {
        let public = obj
            .get("object")
            .and_then(Value::as_str)
            .ok_or_else(|| CmdError::Params("imprint locator needs \"object\"".into()))?;
        let Some(EntityRef::Object(object)) = ctx.resolver().resolve(public) else {
            return Err(unknown_entity(public));
        };
        if ctx.doc.object(object).is_none() {
            return Err(unknown_entity(public));
        }
        let p = locate::parse_xyz(at)?;
        for plane in ctx.doc.object_drawing_planes(object) {
            if plane.signed_distance(p).abs() > geom::API_SURFACE_TOL {
                continue;
            }
            let pi = analyze_plane(ctx, object, plane)?;
            let mut best: Option<(f64, usize)> = None;
            for (ci, c) in pi.chords.iter().enumerate() {
                for w in c.path.windows(2) {
                    let d = geom::point_segment_distance(p, w[0], w[1]).0;
                    if d <= geom::API_SURFACE_TOL && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, ci));
                    }
                }
            }
            if let Some((_, ci)) = best {
                return Ok(target_of(object, &pi, pi.chords[ci].unit));
            }
            let normal = plane.normal();
            if let Some(s) = pi
                .sub_faces
                .iter()
                .find(|s| geom::face_contains(&s.region.outer, &s.region.holes, normal, p))
            {
                return Ok(target_of(object, &pi, s.unit));
            }
        }
        return Err(not_an_imprint());
    }

    match locate::resolve_face(ctx, locator) {
        Ok(face_ref) => {
            let plane = draft::face_plane(ctx, face_ref)?;
            let pi = analyze_plane(ctx, face_ref.object, plane)?;
            pi.sub_faces
                .iter()
                .find(|s| s.face == Some(face_ref.face))
                .map(|s| target_of(face_ref.object, &pi, s.unit))
                .ok_or_else(not_an_imprint)
        }
        Err(CmdError::Refusal(r))
            if matches!(
                r.name.as_str(),
                "unknown_entity" | "face_token_unknown" | "face_token_stale"
            ) =>
        {
            Err(CmdError::Refusal(r))
        }
        Err(CmdError::Params(msg)) => Err(CmdError::Params(msg)),
        Err(CmdError::Internal(msg)) => Err(CmdError::Internal(msg)),
        Err(_) => Err(not_an_imprint()),
    }
}

/// Runs `edit` over the target's island edges as one drafting gesture (one
/// undo step) and answers `{object_id}`.
fn edit_imprint(
    ctx: &mut Ctx,
    target: ImprintTarget,
    label: &str,
    edit: impl FnOnce(&mut kernel::Sketch, &[kernel::SketchEdgeId]) -> Result<(), kernel::SketchError>,
) -> Result<Value, CmdError> {
    let keys = target.keys;
    let plane = target.plane;
    let drafted = ctx
        .doc
        .draft(plane, target.frame, &target.objects, label, |w| {
            let ids: Vec<kernel::SketchEdgeId> = w
                .edges()
                .iter()
                .filter(|(_, e)| {
                    let a = w.vertices()[e.from].position;
                    let b = w.vertices()[e.to].position;
                    keys.contains(&ekey(&plane, a, b))
                })
                .map(|(id, _)| id)
                .collect();
            if ids.len() < keys.len() {
                return Err(kernel::SketchError::UnknownEdge);
            }
            edit(w, &ids)
        });
    match drafted {
        // The old imprint refusal name for a landing off the plane.
        Err(DocumentError::Sketch(kernel::SketchError::PointOffPlane { .. })) => {
            return Err(CmdError::Refusal(Refusal::api(
                "not_in_plane",
                "That change would take the drawing off the plane it lies on. Move, turn, and \
                 scale it within that plane (rotate about the plane's normal; scale about a \
                 point on the plane).",
            )));
        }
        other => {
            other?;
        }
    }
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(target.object)) }))
}

/// `translation(-c) · M · translation(c)`: `m` applied about `c`.
fn about_point(c: Point3, m: &Transform) -> Transform {
    let v = c.to_vec();
    Transform::translation(-v)
        .then(m)
        .then(&Transform::translation(v))
}

fn apply_transform(
    ctx: &mut Ctx,
    target: ImprintTarget,
    label: &str,
    xf: Transform,
) -> Result<Value, CmdError> {
    edit_imprint(ctx, target, label, move |w, ids| {
        w.apply_transform_edges(ids, &xf)
    })
}

/// Per-edge circle claims of an imprint's loop or run, in edge order:
/// `{ center, radius }` for an arc's facet, `null` for a plain edge — how a
/// pie, segment, or edge-to-edge arc reports the arc it keeps.
fn claims_json(curves: &[Option<kernel::CurveGeom>]) -> Vec<Value> {
    curves
        .iter()
        .map(|c| match c {
            Some(g) => json!({ "center": point_json(g.center), "radius": g.radius }),
            None => Value::Null,
        })
        .collect()
}

fn same_circle(a: kernel::CurveGeom, b: kernel::CurveGeom) -> bool {
    same_point(a.center, b.center) && (a.radius - b.radius).abs() <= geom::API_SURFACE_TOL
}

pub(super) fn imprints(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
    }
    let p: P = parse_params(params)?;
    let Some(EntityRef::Object(object_id)) = ctx.resolver().resolve(&p.object) else {
        return Err(unknown_entity(&p.object));
    };
    if ctx.doc.object(object_id).is_none() {
        return Err(unknown_entity(&p.object));
    }
    let planes = analyze_object(ctx, object_id)?;

    let mut list = Vec::new();
    let mut curve_units = Vec::new();
    for pi in &planes {
        let normal = pi.plane.normal();
        let normal_json = point_json(Point3::new(normal.x, normal.y, normal.z));
        for s in &pi.sub_faces {
            let r = &s.region;
            // `at`: a point strictly inside the loop, clear of every hole,
            // so it names THIS region and never one nested in it.
            let at = geom::interior_point_of_loops(&r.outer, &r.holes, normal)
                .unwrap_or_else(|| geom::face_centroid(&r.outer, &[]));
            let curve = match s.claims.first().copied().flatten() {
                Some(g)
                    if s.claims
                        .iter()
                        .all(|c| c.is_some_and(|c| same_circle(c, g))) =>
                {
                    Some(g)
                }
                _ => None,
            };
            let nested = pi
                .sub_faces
                .iter()
                .filter(|o| {
                    o.region.key != r.key
                        && o.region.outer.iter().all(|&q| {
                            geom::face_contains(&r.outer, &[], normal, q)
                                || ring_pairs(&r.outer).iter().any(|&(a, b)| {
                                    geom::point_segment_distance(q, a, b).0 <= geom::API_SURFACE_TOL
                                })
                        })
                })
                .count();
            list.push(json!({
                "kind": "sub_face",
                "at": point_json(at),
                "loop": r.outer.iter().map(|&q| point_json(q)).collect::<Vec<_>>(),
                "holes": r.holes.iter().map(|h| h.iter().map(|&q| point_json(q)).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "curve": curve.map(|c| json!({ "center": point_json(c.center), "radius": c.radius })),
                "curves": claims_json(&s.claims),
                "nested": nested,
                "normal": normal_json,
                "region": key_hex("rgn", r.key),
                "unit": key_hex("isl", pi.pu.units[s.unit].key),
            }));
        }
        for c in &pi.chords {
            list.push(json!({
                "kind": "chord",
                "at": point_json(run_at(&c.path)),
                "path": c.path.iter().map(|&q| point_json(q)).collect::<Vec<_>>(),
                "curves": claims_json(&c.claims),
                "normal": normal_json,
                "unit": key_hex("isl", pi.pu.units[c.unit].key),
            }));
        }

        // Circles and arcs: every run of one curve (a circle other drawing
        // cuts apart is several) as one curve unit.
        let mut groups: Vec<(u64, Vec<&kernel::DraftCurveRun>, usize)> = Vec::new();
        for &ui in &pi.own_units {
            for run in &pi.pu.units[ui].curves {
                if run.kind != kernel::SketchCurveKind::Circle {
                    continue;
                }
                match groups.iter_mut().find(|g| g.0 == run.curve_key) {
                    Some(g) => g.1.push(run),
                    None => groups.push((run.curve_key, vec![run], ui)),
                }
            }
        }
        for (_, runs, ui) in groups {
            let unit = &pi.pu.units[ui];
            let seg_of = |key: u64| unit.edges.iter().find(|e| e.key == key).map(|e| (e.a, e.b));
            let mut degree: Vec<(Point3, usize)> = Vec::new();
            let mut run_json = Vec::new();
            for run in &runs {
                let segs: Vec<(Point3, Point3)> =
                    run.edges.iter().filter_map(|&k| seg_of(k)).collect();
                for &(a, b) in &segs {
                    for q in [a, b] {
                        match degree.iter_mut().find(|d| same_point(d.0, q)) {
                            Some(d) => d.1 += 1,
                            None => degree.push((q, 1)),
                        }
                    }
                }
                let path = order_chain(&segs);
                // The chord entry over these edges carries the locator; a run
                // that is a whole sub-face's outline (a circle no other line
                // cuts) is located inside that sub-face, as its entry is.
                let chord_at = pi
                    .chords
                    .iter()
                    .find(|c| run.edges.iter().all(|k| c.edges.contains(k)))
                    .map(|c| run_at(&c.path));
                let sub_face_at = || {
                    pi.sub_faces
                        .iter()
                        .find(|sf| {
                            let o = &sf.region.outer;
                            let ring: Vec<u64> = (0..o.len())
                                .map(|i| kernel::edge_key(&pi.plane, o[i], o[(i + 1) % o.len()]))
                                .collect();
                            run.edges.iter().all(|k| ring.contains(k))
                        })
                        .and_then(|sf| {
                            geom::interior_point_of_loops(
                                &sf.region.outer,
                                &sf.region.holes,
                                normal,
                            )
                        })
                };
                let at = chord_at
                    .or_else(sub_face_at)
                    .unwrap_or_else(|| run_at(&path));
                run_json.push(json!({
                    "at": point_json(at),
                    "path": path.iter().map(|&q| point_json(q)).collect::<Vec<_>>(),
                }));
            }
            let g = runs[0].geom;
            curve_units.push(json!({
                "closed": degree.iter().all(|d| d.1 == 2),
                "curve": { "center": point_json(g.center), "radius": g.radius },
                "runs": run_json,
            }));
        }
    }
    Ok(json!({ "imprints": list, "curve_units": curve_units }))
}

pub(super) fn move_imprint(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        imprint: Value,
        offset: [f64; 3],
    }
    let p: P = parse_params(params)?;
    let offset = finite_vec3(p.offset)?;
    let target = resolve_imprint(ctx, &p.imprint)?;
    apply_transform(ctx, target, "Move", Transform::translation(offset))
}

pub(super) fn rotate_imprint(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        imprint: Value,
        angle: f64,
        about: Value,
    }
    let p: P = parse_params(params)?;
    if !p.angle.is_finite() {
        return Err(CmdError::Params("angle must be finite".into()));
    }
    // Resolved before the imprint (mirrors `hew.entity.rotate`): a
    // derived-point locator failure should read as its own error, not get
    // masked by an unrelated imprint-resolution refusal.
    let about = locate::resolve_point(ctx, &p.about)?;
    let target = resolve_imprint(ctx, &p.imprint)?;
    // The axis is the drawing plane's normal through `about`, which keeps
    // the drawing in its plane for any `about`.
    let r = Transform::rotation(target.plane.normal(), p.angle)
        .map_err(|_| CmdError::Internal("imprint plane normal is degenerate".into()))?;
    apply_transform(ctx, target, "Rotate", about_point(about, &r))
}

pub(super) fn scale_imprint(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        imprint: Value,
        factor: f64,
        about: Value,
    }
    let p: P = parse_params(params)?;
    if !p.factor.is_finite() || p.factor <= 0.0 {
        return Err(CmdError::Params(
            "scale factor must be finite and positive".into(),
        ));
    }
    let about = locate::resolve_point(ctx, &p.about)?;
    let target = resolve_imprint(ctx, &p.imprint)?;
    // Uniform scale about `about`; the draft refuses `not_in_plane` when
    // `about` is off the drawing plane (the result would leave it).
    let s = Transform::scale(Vec3::new(p.factor, p.factor, p.factor));
    apply_transform(ctx, target, "Scale", about_point(about, &s))
}

pub(super) fn delete_imprint(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        imprint: Value,
    }
    let p: P = parse_params(params)?;
    let target = resolve_imprint(ctx, &p.imprint)?;
    edit_imprint(ctx, target, "Delete", |w, ids| {
        for &e in ids {
            if w.edges().contains_key(e) {
                w.remove_edge(e)?;
            }
        }
        Ok(())
    })
}
