//! `hew.*` commands of the true-curves effort (object). Declarations live in
//! `registry.rs`; this module owns the handlers.

use super::{CmdError, Ctx, Handler};
use crate::locate;
use crate::refusal::Refusal;
use kernel::{EntityRef, ObjectId};
use serde_json::{Value, json};

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        // ==== true-curves: smooth ====
        "hew.object.smooth" => smooth,
        "hew.object.crease" => crease,
        "hew.solid.bake" => bake,
        // ==== end true-curves: smooth ====
        // ==== true-curves: imports ====
        "hew.solid.soften" => soften,
        // ==== end true-curves: imports ====
        _ => return None,
    })
}

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

fn unknown_entity(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_entity",
            &format!("'{id}' does not name a live entity of the required kind in this document."),
        )
        .with_detail(json!({ "id": id })),
    )
}

/// A public id naming a live object (world object or definition member).
fn resolve_object(ctx: &Ctx, public: &str) -> Result<ObjectId, CmdError> {
    match ctx.resolver().resolve(public) {
        Some(EntityRef::Object(id)) if ctx.doc.object(id).is_some() => Ok(id),
        Some(other) if !matches!(other, EntityRef::Object(_)) => Err(CmdError::Params(format!(
            "'{public}' does not name an object"
        ))),
        _ => Err(unknown_entity(public)),
    }
}

fn public_id_of(ctx: &Ctx, entity: EntityRef) -> String {
    let sid = ctx
        .doc
        .sid_of(&entity)
        .expect("an entity this command just touched always carries a stable id");
    crate::ids::public_id(&entity, sid)
}

// ==== true-curves: smooth ====

/// `hew.object.smooth`: sets an object's Smooth level (0 = Off, 1–3).
fn smooth(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        level: u8,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    ctx.doc.set_smooth_level(object, p.level)?;
    Ok(json!({}))
}

/// `hew.object.crease`: sets or clears the crease flag on the located
/// edges' logical edges (`propagate: "tangent"` also follows tangent
/// continuations). Every edge must be on one object.
fn crease(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        edges: Vec<Value>,
        #[serde(default = "yes")]
        crease: bool,
        #[serde(default)]
        propagate: Option<String>,
    }
    fn yes() -> bool {
        true
    }
    let p: P = parse_params(params)?;
    let tangent = match p.propagate.as_deref() {
        None | Some("none") => false,
        Some("tangent") => true,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "propagate must be \"none\" or \"tangent\", not {other:?}"
            )));
        }
    };
    if p.edges.is_empty() {
        return Err(CmdError::Params(
            "edges needs at least one edge locator".into(),
        ));
    }
    let mut object = None;
    let mut edges: Vec<kernel::EdgeId> = Vec::new();
    for loc in &p.edges {
        let e = locate::resolve_edge(ctx, loc)?;
        if object.is_some_and(|o| o != e.object) {
            return Err(CmdError::Refusal(Refusal::api(
                "edges_span_objects",
                "Every edge must be on the same object.",
            )));
        }
        object = Some(e.object);
        let obj = ctx
            .doc
            .object(e.object)
            .ok_or_else(|| unknown_entity("edge"))?;
        for x in obj.logical_edge(e.edge, tangent) {
            if !edges.contains(&x) {
                edges.push(x);
            }
        }
    }
    let object = object.expect("at least one edge resolved");
    ctx.doc.set_crease(object, &edges, p.crease)?;
    Ok(json!({ "edges": edges.len() }))
}

/// `hew.solid.bake`: replaces a smoothed object's cage with its Smooth
/// surface and clears the level.
fn bake(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    if ctx.doc.smooth_level(object) == 0 {
        return Err(CmdError::Refusal(Refusal::api(
            "amount_out_of_range",
            "This object has no Smooth level to bake.",
        )));
    }
    ctx.doc.bake(object)?;
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

// ==== end true-curves: smooth ====

// ==== true-curves: imports ====

/// `hew.solid.soften`: softens the edges that bend less than `max_angle`
/// (radians), so a faceted import shades and selects as smooth walls.
fn soften(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        max_angle: f64,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    let (n, _) = ctx.doc.soften_by_angle(object, p.max_angle)?;
    Ok(json!({ "softened": n }))
}

// ==== end true-curves: imports ====
