//! The app's selection over the bus: `hew.selection.get`, `set`, `clear`
//! (docs/agents/HEW_API.md §7, "Selection and view state").
//!
//! All three are `Served::Host` and `ReadOnly`: the selection is not
//! document state, so it records no undo entry and rides a transaction as a
//! read would, rolling back with an aborted envelope (the host discards its
//! copy). This module validates and shapes; the host holds the selection
//! and merges a pick into it ([`crate::selection::merge`]).

use super::entity::unknown_entity;
use super::{CmdError, Ctx, Handler};
use crate::host::{SelectMode, SelectionEntry};
use crate::refusal::Refusal;
use crate::selection::{entry_live, selection_json};
use serde::Deserialize;
use serde_json::Value;

/// The most ids one `hew.selection.set` names.
pub const MAX_SELECTION_IDS: usize = 10_000;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.selection.get" => get,
        "hew.selection.set" => set,
        "hew.selection.clear" => clear,
        _ => return None,
    })
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {}

fn get(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    let state = ctx.host.selection().map_err(CmdError::Refusal)?;
    Ok(selection_json(ctx.doc, &state.entries))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    ids: Vec<String>,
    #[serde(default)]
    mode: Option<String>,
}

/// Parses a `mode` name; absent is `replace`.
fn parse_mode(mode: Option<&str>) -> Result<SelectMode, CmdError> {
    Ok(match mode {
        None | Some("replace") => SelectMode::Replace,
        Some("add") => SelectMode::Add,
        Some("subtract") => SelectMode::Subtract,
        Some("toggle") => SelectMode::Toggle,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "unknown mode \"{other}\"; expected replace, add, subtract, or toggle"
            )));
        }
    })
}

/// Resolves `ids` to selection entries, refusing what the app could never
/// hold as a selection: an id that names no live selectable entity
/// (`unknown_entity`), picked nodes mixed with a guide or an annotation, or
/// more than one guide or annotation (`selection_mixed_kinds`), and a hidden
/// node or sketch (`selection_target_not_visible`). An id named twice
/// counts once.
pub(crate) fn resolve_selection(
    ctx: &Ctx,
    ids: &[String],
) -> Result<Vec<SelectionEntry>, CmdError> {
    let resolver = ctx.resolver();
    let mut resolved: Vec<(&str, SelectionEntry)> = Vec::with_capacity(ids.len());
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        let entry = resolver
            .resolve(id)
            .as_ref()
            .and_then(SelectionEntry::from_entity)
            .filter(|e| entry_live(ctx.doc, *e))
            .ok_or_else(|| unknown_entity(id))?;
        if seen.insert(entry) {
            resolved.push((id, entry));
        }
    }
    let picked: Vec<SelectionEntry> = resolved.iter().map(|(_, e)| *e).collect();
    check_one_kind(&picked)?;
    for (id, entry) in &resolved {
        let hidden = match *entry {
            SelectionEntry::Object(o) => ctx.doc.node_effectively_hidden(kernel::NodeId::Object(o)),
            SelectionEntry::Group(g) => ctx.doc.node_effectively_hidden(kernel::NodeId::Group(g)),
            SelectionEntry::Instance(i) => {
                ctx.doc.node_effectively_hidden(kernel::NodeId::Instance(i))
            }
            SelectionEntry::Sketch(s) => !ctx.doc.sketch_shown(s),
            SelectionEntry::Guide(_) | SelectionEntry::Annotation(_) => false,
        };
        if hidden {
            return Err(CmdError::Refusal(
                Refusal::api(
                    "selection_target_not_visible",
                    "That entity is hidden. Show it first, or isolate it.",
                )
                .with_detail(serde_json::json!({ "id": id })),
            ));
        }
    }
    Ok(resolved.into_iter().map(|(_, e)| e).collect())
}

/// The one-kind rule: nodes and sketches in any mix, or exactly one guide,
/// or exactly one annotation.
fn check_one_kind(entries: &[SelectionEntry]) -> Result<(), CmdError> {
    let singular = entries
        .iter()
        .any(|e| matches!(e, SelectionEntry::Guide(_) | SelectionEntry::Annotation(_)));
    if singular && entries.len() > 1 {
        return Err(CmdError::Refusal(Refusal::api(
            "selection_mixed_kinds",
            "A selection holds nodes, or one guide, or one annotation, never a mix.",
        )));
    }
    Ok(())
}

fn set(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let SetParams { ids, mode } = parse(params)?;
    if ids.len() > MAX_SELECTION_IDS {
        return Err(CmdError::Params(format!(
            "ids names {} entities; at most {MAX_SELECTION_IDS} are allowed",
            ids.len()
        )));
    }
    let mode = parse_mode(mode.as_deref())?;
    let entries = resolve_selection(ctx, &ids)?;
    // The rule holds for the result too: adding a guide to nodes, or a
    // second annotation to one, refuses rather than mixing.
    let current = ctx.host.selection().map_err(CmdError::Refusal)?;
    check_one_kind(&crate::selection::merge(&current.entries, &entries, mode))?;
    let state = ctx
        .host
        .set_selection(entries, mode)
        .map_err(CmdError::Refusal)?;
    Ok(selection_json(ctx.doc, &state.entries))
}

fn clear(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    let state = ctx
        .host
        .set_selection(Vec::new(), SelectMode::Replace)
        .map_err(CmdError::Refusal)?;
    Ok(selection_json(ctx.doc, &state.entries))
}
