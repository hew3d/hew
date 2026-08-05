//! Solid operations: hew.solid.* (docs/HEW_API.md §7, §7 semantics
//! notes; the kernel-surface recipes are api-kernel-map.md §1.3/§1.4).
//!
//! `push_pull` reproduces the wasm boundary's three-way branch exactly
//! (api-kernel-map.md §1.3, `Scene::push_pull`): a through-cut routes to
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

use super::{CmdError, Ctx, Handler};
use crate::locate;
use crate::refusal::Refusal;
use kernel::{DocumentError, EntityRef, FollowMePath, KernelOp, NodeId};
use serde_json::Value;

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
/// docs/HEW_API.md §4.4 promises. Unwrapped here, locally, one level
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
    }
    let p: P = parse_params(params)?;
    let (sketch, region) = ctx
        .resolver()
        .resolve_region(&p.region)
        .ok_or_else(|| unknown_entity(&p.region))?;
    let sketch_normal = ctx
        .doc
        .sketch(sketch)
        .ok_or_else(|| unknown_entity(&p.region))?
        .plane()
        .normal();

    let (object, _change) = ctx.doc.extrude_region(sketch, region, p.distance)?;

    // Face tokens (docs/HEW_API.md §5.4, normative here): the cap facing
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

    // A face resolving to a component-DEFINITION member routes through
    // the def-scoped kernel methods below instead of the plain world
    // ones — same three-way branch, shared-geometry edit, so every
    // instance of the component picks it up at once (module doc
    // comment; mirrors `commands/sketch.rs`'s `apply_face_op`).
    let component = ctx.doc.object_owner_component(face_ref.object);

    let overshoots = ctx
        .doc
        .object(face_ref.object)
        .is_some_and(|o| o.push_pull_overshoots(face_ref.face, p.distance));
    if overshoots {
        let (objects, _change) = match component {
            Some(c) => ctx
                .doc
                .push_pull_through_in_component(c, face_ref.object, face_ref.face, p.distance)
                .map_err(push_pull_refusal)?,
            None => ctx
                .doc
                .push_pull_through(face_ref.object, face_ref.face, p.distance)
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
            distance: p.distance,
        }
    } else {
        KernelOp::PushPull {
            face: face_ref.face,
            distance: p.distance,
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
        a: String,
        b: String,
    }
    let p: P = parse_params(params)?;
    let a = resolve_node(ctx, &p.a)?;
    let b = resolve_node(ctx, &p.b)?;
    let (result, _change) = ctx.doc.boolean_nodes(op, a, b)?;
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
