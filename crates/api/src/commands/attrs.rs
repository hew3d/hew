//! Attribute dictionaries: hew.attr.* (docs/HEW_API.md §8).

use super::entity::unknown_entity;
use super::{CmdError, Ctx, Handler};
use crate::refusal::Refusal;
use kernel::{AttrTarget, AttrValue, AttrValueError, EntityRef};
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

/// `{"target": "<public id>"}` or `{"target": "document"}` →
/// `kernel::AttrTarget` (docs/HEW_API.md §8: every addressable entity,
/// plus the document itself, plus tags via their `tag_<hex>` public id).
fn resolve_target(ctx: &Ctx, target: &str) -> Result<AttrTarget, CmdError> {
    if target == "document" {
        return Ok(AttrTarget::Document);
    }
    let entity: EntityRef = ctx
        .resolver()
        .resolve(target)
        .ok_or_else(|| unknown_entity(target))?;
    Ok(AttrTarget::Entity(entity))
}

/// docs/HEW_API.md §8: the `hew` prefix (`"hew"`, `"hew.*"`) is reserved
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
/// Checked before the target is resolved, so the answer is a property of
/// the request alone and never varies with document state.
fn reject_reserved_namespace(ns: &str) -> Result<(), CmdError> {
    if ns == "hew" || ns.starts_with("hew.") {
        return Err(CmdError::Refusal(
            Refusal::api(
                "reserved_attr_namespace",
                "The \"hew\" attribute namespace is reserved for Hew's own data. Store yours under a namespace you own, like \"com.example.myapp\".",
            )
            .with_detail(serde_json::json!({ "ns": ns })),
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
    target: String,
    #[serde(default)]
    ns: Option<String>,
}

fn get(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: GetParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let target = resolve_target(ctx, &p.target)?;
    let dict = ctx.doc.attr_get(&target)?;
    let mut out = serde_json::Map::new();
    if let Some(dict) = dict {
        match &p.ns {
            Some(ns) => {
                if let Some(entries) = dict.get(ns) {
                    out.insert(
                        ns.clone(),
                        serde_json::Value::Object(
                            entries
                                .iter()
                                .map(|(k, v)| (k.clone(), v.to_json()))
                                .collect(),
                        ),
                    );
                }
            }
            None => {
                for (ns, entries) in dict {
                    out.insert(
                        ns.clone(),
                        serde_json::Value::Object(
                            entries
                                .iter()
                                .map(|(k, v)| (k.clone(), v.to_json()))
                                .collect(),
                        ),
                    );
                }
            }
        }
    }
    Ok(serde_json::Value::Object(out))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    target: String,
    ns: String,
    key: String,
    value: Value,
}

fn set(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SetParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    reject_reserved_namespace(&p.ns)?;
    let target = resolve_target(ctx, &p.target)?;
    let value = AttrValue::from_json(&p.value).map_err(attr_value_error)?;
    ctx.doc.attr_set(target, &p.ns, &p.key, value)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteParams {
    target: String,
    ns: String,
    #[serde(default)]
    key: Option<String>,
}

fn delete(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: DeleteParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    reject_reserved_namespace(&p.ns)?;
    let target = resolve_target(ctx, &p.target)?;
    ctx.doc.attr_delete(target, &p.ns, p.key.as_deref())?;
    Ok(serde_json::json!({}))
}
