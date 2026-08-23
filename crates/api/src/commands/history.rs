//! Shared history: hew.history.* (docs/agents/HEW_API.md §7 semantics notes).
//!
//! All three run SOLITARY — bare, never inside a transaction (§6.4): the
//! registry classes them `Solitary`, so dispatch never brackets them with
//! `begin_transaction`/`commit_transaction` and they never add or touch a
//! compound entry themselves. `undo`/`redo` pop or restore the shared
//! document history, whoever authored the top entry.

use super::{CmdError, Ctx, Handler};
use kernel::HistoryOrigin;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.history.undo" => undo,
        "hew.history.redo" => redo,
        "hew.history.status" => status,
        _ => return None,
    })
}

fn origin_json(origin: &HistoryOrigin) -> Value {
    match origin {
        HistoryOrigin::User => serde_json::json!("user"),
        HistoryOrigin::Connection(id) => serde_json::json!({ "connection": id }),
    }
}

/// The top undo entry's label/origin, distinguishing "no entries"
/// (`None`) from "an entry with no recorded `CompoundMeta`" — a
/// UI-authored edit that predates the API, or one committed outside
/// `commit_transaction` — which reports `{"label": null, "origin":
/// "user"}` rather than being confused for an empty stack.
fn top_json(ctx: &Ctx) -> Value {
    if ctx.doc.undo_depth() == 0 {
        return Value::Null;
    }
    match ctx.doc.peek_undo_meta() {
        Some(meta) => serde_json::json!({
            "label": meta.label,
            "origin": origin_json(&meta.origin),
        }),
        None => serde_json::json!({ "label": null, "origin": "user" }),
    }
}

fn empty_params(params: &Value) -> Result<(), CmdError> {
    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    let _: Empty =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    Ok(())
}

fn status(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    empty_params(params)?;
    Ok(serde_json::json!({
        "undo_depth": ctx.doc.undo_depth(),
        "redo_depth": ctx.doc.redo_depth(),
        "top": top_json(ctx),
    }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoParams {
    #[serde(default)]
    expected_label: Option<String>,
}

fn undo(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: UndoParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if let Some(expected) = &p.expected_label {
        let found = ctx.doc.peek_undo_meta().map(|m| m.label.clone());
        if found.as_deref() != Some(expected.as_str()) {
            return Err(CmdError::Refusal(
                crate::refusal::Refusal::api(
                    "expected_label_mismatch",
                    "The top undo entry is not the one expected — the document's history moved since this label was checked. Re-read hew.history.status before undoing.",
                )
                .with_detail(serde_json::json!({ "expected": expected, "found": found })),
            ));
        }
    }
    ctx.doc.undo()?;
    Ok(serde_json::json!({}))
}

fn redo(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    empty_params(params)?;
    ctx.doc.redo()?;
    Ok(serde_json::json!({}))
}
