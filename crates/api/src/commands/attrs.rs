//! Attribute dictionaries: hew.attr.* (docs/agents/HEW_API.md §8).

use super::entity::unknown_entity;
use super::{CmdError, Ctx, Handler};
use crate::locate::{self, AnyEdge};
use crate::refusal::Refusal;
use kernel::{AttrTarget, AttrValue, AttrValueError, EdgeId, EntityRef, FaceId, ObjectId};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.attr.get" => get,
        "hew.attr.set" => set,
        "hew.attr.delete" => delete,
        _ => return None,
    })
}

/// What an attribute call addresses: an entity's (or the document's)
/// dictionary, or a solid face's or edge's.
enum Target {
    Entity(AttrTarget),
    Face(ObjectId, FaceId),
    Edge(ObjectId, EdgeId),
}

/// `"<public id>"` or `"document"` → an entity target (docs/agents/HEW_API.md
/// §8: every addressable entity, plus the document itself, plus tags via
/// their `tag_<hex>` public id); `{"face": <face locator>}` or `{"edge":
/// <edge locator>}` (§5.2) → a solid face or edge, resolved now against
/// the document the command runs in. A sketch edge has no face or edge
/// dictionary (sketch entities keep the entity-level ones) and refuses
/// `not_a_solid_edge`.
fn resolve_target(ctx: &Ctx, target: &Value) -> Result<Target, CmdError> {
    if let Some(target) = target.as_str() {
        if target == "document" {
            return Ok(Target::Entity(AttrTarget::Document));
        }
        let entity: EntityRef = ctx
            .resolver()
            .resolve(target)
            .ok_or_else(|| unknown_entity(target))?;
        return Ok(Target::Entity(AttrTarget::Entity(entity)));
    }
    let shape = || {
        CmdError::Params(
            "target must be an id, \"document\", {\"face\": <locator>}, or {\"edge\": <locator>}"
                .into(),
        )
    };
    let obj = target.as_object().ok_or_else(shape)?;
    if obj.len() != 1 {
        return Err(shape());
    }
    if let Some(face) = obj.get("face") {
        let f = locate::resolve_face(ctx, face)?;
        return Ok(Target::Face(f.object, f.face));
    }
    if let Some(edge) = obj.get("edge") {
        return match locate::resolve_edge_locator(ctx, edge)? {
            AnyEdge::Solid(e) => Ok(Target::Edge(e.object, e.edge)),
            AnyEdge::Sketch(_) => Err(CmdError::Refusal(Refusal::api(
                "not_a_solid_edge",
                "That locator names a sketch edge, which has no edge dictionary. Give a solid's edge with {\"object\", \"at\"}, or store the data on the sketch itself.",
            ))),
        };
    }
    Err(shape())
}

/// A dictionary as the wire's `namespace → key → value` object, all
/// namespaces or just `ns`.
fn dict_json(dict: Option<&kernel::AttrDict>, ns: Option<&str>) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(dict) = dict {
        for (name, entries) in dict {
            if ns.is_some_and(|ns| ns != name) {
                continue;
            }
            out.insert(
                name.clone(),
                Value::Object(
                    entries
                        .iter()
                        .map(|(k, v)| (k.clone(), v.to_json()))
                        .collect(),
                ),
            );
        }
    }
    Value::Object(out)
}

/// A solid face's or edge's dictionary as `hew.query.faces` and
/// `hew.query.topology` report it: the full object, `{}` for none.
pub fn element_attrs_json(dict: Option<&kernel::AttrDict>) -> Value {
    dict_json(dict, None)
}

/// docs/agents/HEW_API.md §8: the `hew` prefix (`"hew"`, `"hew.*"`) is reserved
/// for first-party use. Enforced here, at the API boundary, rather than
/// in `Document::attr_set` — the reservation exists so Hew's own code can
/// claim those namespaces without colliding with a client's data, which
/// means the kernel and the UI must keep writing them freely.
///
/// The test is exact and case-sensitive: `hew` itself and anything under
/// `hew.`, never a namespace that merely begins with those three letters
/// (`hewlett.example` is a legitimate reverse-DNS name and is accepted).
///
/// Reads are deliberately unrestricted — `hew.attr.get` still returns
/// first-party dictionaries. The reservation is on claiming a namespace,
/// not on seeing what a document carries.
///
/// Then the grant's namespace rule (docs/agents/HEW_API.md §8, §10): a
/// plugin writes only under its own id and the namespaces it declared
/// with `attributes.write:<ns>`; a connected app only under its declared
/// namespaces; first-party connections anywhere outside `hew`.
///
/// Checked before the target is resolved, so the answer is a property of
/// the request and the grant alone and never varies with document state.
fn check_namespace(ctx: &Ctx, ns: &str) -> Result<(), CmdError> {
    if ns == "hew" || ns.starts_with("hew.") {
        return Err(CmdError::Refusal(
            Refusal::api(
                "reserved_attr_namespace",
                "The \"hew\" attribute namespace is reserved for Hew's own data. Store yours under a namespace you own, like \"com.example.myapp\".",
            )
            .with_detail(serde_json::json!({ "ns": ns })),
        ));
    }
    if !ctx.grant.may_write_attr_namespace(ns) {
        return Err(CmdError::Refusal(
            Refusal::api(
                "attr_namespace_not_granted",
                &match &ctx.grant.plugin {
                    Some(p) => format!(
                        "This connection may write attributes only under its own namespace, \"{p}\", and names below it. Use that namespace, or ask the user for attributes.write:{ns}."
                    ),
                    None => "This connection may write attributes only under the namespaces it declared when it paired.".to_string(),
                },
            )
            .with_detail(serde_json::json!({
                "ns": ns,
                "plugin": ctx.grant.plugin,
                "app": ctx.grant.app,
            })),
        ));
    }
    Ok(())
}

fn attr_value_error(e: AttrValueError) -> CmdError {
    match e {
        AttrValueError::UnrepresentableNumber => CmdError::Refusal(Refusal::api(
            "unrepresentable_attr_value",
            "That number is too large to store exactly as an attribute value — beyond what a 64-bit integer or a finite float can represent losslessly.",
        )),
        // `AttrValue::from_json` enforces the same depth/finiteness rules
        // `Document::attr_set` validates — reuse the kernel's own machine
        // names so a value that would trip `attr_set`'s validation trips
        // the identical refusal here, one step earlier.
        AttrValueError::TooDeep => CmdError::Refusal(Refusal::api(
            "attr_value_too_deep",
            &format!(
                "Attribute values may nest at most {} levels; this value nests deeper.",
                kernel::attr::MAX_ATTR_DEPTH
            ),
        )),
        AttrValueError::NonFinite => CmdError::Refusal(Refusal::api(
            "non_finite_attr_value",
            "Attribute numbers must be finite (no NaN or infinity).",
        )),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GetParams {
    target: Value,
    #[serde(default)]
    ns: Option<String>,
}

fn get(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: GetParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let dict = match resolve_target(ctx, &p.target)? {
        Target::Entity(target) => ctx.doc.attr_get(&target)?,
        Target::Face(object, face) => ctx.doc.face_attr_get(object, face)?,
        Target::Edge(object, edge) => ctx.doc.edge_attr_get(object, edge)?,
    };
    Ok(dict_json(dict, p.ns.as_deref()))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    target: Value,
    ns: String,
    key: String,
    value: Value,
}

fn set(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SetParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    check_namespace(ctx, &p.ns)?;
    let target = resolve_target(ctx, &p.target)?;
    let value = AttrValue::from_json(&p.value).map_err(attr_value_error)?;
    match target {
        Target::Entity(target) => ctx.doc.attr_set(target, &p.ns, &p.key, value)?,
        Target::Face(object, face) => ctx.doc.face_attr_set(object, face, &p.ns, &p.key, value)?,
        Target::Edge(object, edge) => ctx.doc.edge_attr_set(object, edge, &p.ns, &p.key, value)?,
    };
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteParams {
    target: Value,
    ns: String,
    #[serde(default)]
    key: Option<String>,
}

fn delete(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: DeleteParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    check_namespace(ctx, &p.ns)?;
    let key = p.key.as_deref();
    match resolve_target(ctx, &p.target)? {
        Target::Entity(target) => ctx.doc.attr_delete(target, &p.ns, key)?,
        Target::Face(object, face) => ctx.doc.face_attr_delete(object, face, &p.ns, key)?,
        Target::Edge(object, edge) => ctx.doc.edge_attr_delete(object, edge, &p.ns, key)?,
    };
    Ok(serde_json::json!({}))
}
