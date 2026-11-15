//! `hew.*` commands of the true-curves effort (array). Declarations live in
//! `registry.rs`; this module owns the handlers.

use super::{CmdError, Ctx, Handler, entity, solid};
use crate::refusal::Refusal;
use kernel::{EntityRef, MAX_ARRAY_COUNT, NodeId, PathArraySpec};
use serde_json::{Value, json};

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.array.path" => path,
        _ => return None,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    nodes: Vec<String>,
    path: Value,
    count: u32,
    #[serde(default)]
    scale: Option<Ramp>,
    #[serde(default = "default_align")]
    align: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Ramp {
    from: f64,
    to: f64,
}

fn default_align() -> bool {
    true
}

/// `hew.array.path`: copies nodes along a path with an optional scale ramp
/// and alignment (one undo step). Result `{"node_ids": [...]}`: the copies
/// in creation order, original excluded.
fn path(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: PathParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    // The same trust cap `hew.entity.move`'s array copy applies.
    if p.count > MAX_ARRAY_COUNT {
        return Err(CmdError::Refusal(
            Refusal::api(
                "array_count_too_large",
                &format!("An array places at most {MAX_ARRAY_COUNT} instances at once."),
            )
            .with_detail(json!({ "max": MAX_ARRAY_COUNT })),
        ));
    }
    let nodes: Vec<NodeId> = p
        .nodes
        .iter()
        .map(|id| entity::resolve_node(ctx, id))
        .collect::<Result<_, _>>()?;
    let ramp = p.scale.unwrap_or(Ramp { from: 1.0, to: 1.0 });
    let spec = PathArraySpec {
        count: p.count,
        scale_from: ramp.from,
        scale_to: ramp.to,
        align: p.align,
    };
    let path = solid::follow_me_path(ctx, &p.path)?;
    let (roots, _change) = ctx.doc.array_along_path(&nodes, &path, &spec)?;
    let resolver = ctx.resolver();
    let node_ids: Vec<String> = roots
        .iter()
        .map(|&n| {
            let entity = match n {
                NodeId::Object(o) => EntityRef::Object(o),
                NodeId::Group(g) => EntityRef::Group(g),
                NodeId::Instance(i) => EntityRef::Instance(i),
            };
            resolver.public_of(ctx.doc, &entity).expect("just created")
        })
        .collect();
    Ok(json!({ "node_ids": node_ids }))
}
