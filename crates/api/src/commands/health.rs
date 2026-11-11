//! Model health: `hew.doc.health` and the repairs `hew.solid.cap`,
//! `hew.solid.delete_faces`, `hew.solid.mark_surface`, `hew.solid.thicken`
//! (the import-health design §4 and §7). Each repair is one undo entry; a
//! batch is one `hew.doc.transact` frame over them. Every one takes a world
//! object or a component-definition member (a member's repair reaches
//! every instance), and keeps the object's id: an object that becomes a
//! solid is the same object.

use super::{CmdError, Ctx, Handler};
use crate::locate;
use crate::refusal::Refusal;
use kernel::{EntityRef, ObjectId};
use serde_json::{Value, json};

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.doc.health" => health,
        "hew.solid.cap" => cap,
        "hew.solid.mend" => mend,
        "hew.solid.delete_faces" => delete_faces,
        "hew.solid.mark_surface" => mark_surface,
        "hew.solid.thicken" => thicken,
        _ => return None,
    })
}

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

fn public_id(ctx: &Ctx, entity: EntityRef) -> String {
    let sid = ctx
        .doc
        .sid_of(&entity)
        .expect("a live entity always carries a stable id");
    crate::ids::public_id(&entity, sid)
}

/// A public id naming a live object (world object or definition member).
fn resolve_object(ctx: &Ctx, public: &str) -> Result<ObjectId, CmdError> {
    match ctx.resolver().resolve(public) {
        Some(EntityRef::Object(id)) if ctx.doc.object(id).is_some() => Ok(id),
        Some(other) if !matches!(other, EntityRef::Object(_)) => Err(CmdError::Params(format!(
            "'{public}' does not name an object"
        ))),
        // Unknown, or an object no longer in the model (deleted, undone).
        _ => Err(CmdError::Refusal(
            Refusal::api(
                "unknown_entity",
                &format!(
                    "'{public}' does not name a live entity of the required kind in this document."
                ),
            )
            .with_detail(json!({ "id": public })),
        )),
    }
}

fn health(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        #[serde(default)]
        objects: Option<Vec<String>>,
    }
    let p: P = parse_params(params)?;
    let scope = match &p.objects {
        Some(ids) => Some(
            ids.iter()
                .map(|id| resolve_object(ctx, id))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        None => None,
    };
    let ctx_ref: &Ctx = ctx;
    Ok(crate::health::report_json(
        ctx_ref.doc,
        scope.as_deref(),
        &|e| public_id(ctx_ref, e),
    ))
}

fn cap(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        #[serde(default)]
        rims: Option<Vec<usize>>,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    let (report, _change) = ctx.doc.cap_rims(object, p.rims.as_deref())?;
    // Face tokens for the new faces (§5.4): "cap.1", "cap.2", … in the
    // order they were added, so a following push_pull can name one.
    for (n, &face) in report.faces.iter().enumerate() {
        ctx.mint_face_token(&format!("cap.{}", n + 1), object, face);
    }
    Ok(json!({
        "object_id": p.object,
        "faces": report.faces.len(),
        "closed": report.closed,
        "reversed": report.reversed,
    }))
}

/// `hew.solid.mend`: Model Health's Mend (merge nearby points, remove
/// zero-width folds, separate pinched parts, fill small gaps) as one undo
/// entry. Omitted fields give the one-click Mend
/// ([`crate::health::mend_plan_from_json`]); pieces the split cuts loose
/// are new objects, returned by id.
fn mend(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let public = params
        .get("object")
        .and_then(Value::as_str)
        .ok_or_else(|| CmdError::Params("missing 'object'".into()))?
        .to_string();
    let object = resolve_object(ctx, &public)?;
    let obj = ctx.doc.object(object).expect("resolved above");
    let plan = crate::health::mend_plan_from_json(params, obj).map_err(CmdError::Params)?;
    let (report, pieces, _change) = ctx.doc.mend(object, &plan)?;
    let ids: Vec<String> = pieces
        .iter()
        .map(|&o| public_id(ctx, EntityRef::Object(o)))
        .collect();
    let mut out = crate::health::mend_report_json(&report, ids);
    out["object_id"] = json!(public);
    Ok(out)
}

fn delete_faces(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        #[serde(default)]
        faces: Option<Vec<Value>>,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    // No faces named: every face, which deletes the object — how a batch
    // drops a component member (an importer's partition sheet) that
    // hew.entity.delete, a tree-node command, cannot reach.
    let Some(locators) = &p.faces else {
        let all: Vec<kernel::FaceId> = ctx
            .doc
            .object(object)
            .map(|o| o.faces().keys().collect())
            .unwrap_or_default();
        ctx.doc.delete_faces(object, &all)?;
        return Ok(json!({ "object_id": p.object, "deleted": true, "object_ids": [] }));
    };
    let mut faces = Vec::with_capacity(locators.len());
    for locator in locators {
        let f = locate::resolve_face(ctx, locator)?;
        if f.object != object {
            return Err(CmdError::Params(format!(
                "a face locator resolved to a face of another object than '{}'",
                p.object
            )));
        }
        faces.push(f.face);
    }
    let (split_off, _change) = ctx.doc.delete_faces(object, &faces)?;
    let deleted = ctx.doc.object(object).is_none();
    let object_ids: Vec<String> = split_off
        .iter()
        .map(|&o| public_id(ctx, EntityRef::Object(o)))
        .collect();
    Ok(json!({ "object_id": p.object, "deleted": deleted, "object_ids": object_ids }))
}

fn mark_surface(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        #[serde(default = "yes")]
        accepted: bool,
    }
    fn yes() -> bool {
        true
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    ctx.doc.set_accepted_surface(object, p.accepted)?;
    Ok(json!({ "object_id": p.object, "accepted": p.accepted }))
}

fn thicken(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        distance: f64,
        #[serde(default)]
        centered: bool,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    let (start, end) = if p.centered {
        (-p.distance / 2.0, p.distance / 2.0)
    } else {
        (0.0, p.distance)
    };
    let (split_off, _change) = ctx.doc.thicken_sheet(object, start, end)?;
    let object_ids: Vec<String> = split_off
        .iter()
        .map(|&o| public_id(ctx, EntityRef::Object(o)))
        .collect();
    Ok(json!({ "object_id": p.object, "object_ids": object_ids }))
}
