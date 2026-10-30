//! The transaction executor (docs/agents/HEW_API.md §6): one envelope, atomic,
//! one labeled undo entry, riding the kernel's transaction bracket.
//!
//! Statically checkable defects — unknown methods, misplaced solitary
//! commands, unknown/forward `$ref`/`$face` labels, malformed pointer
//! syntax, unbalanced context — reject the WHOLE envelope (`-32602` /
//! `-32001`) before anything runs. A command's own refusal aborts the
//! bracket with the document exactly as it was, reported in the
//! canonical §4.4 shape with the failing index.

use crate::commands::{self, CmdError, Ctx, FaceTokens};
use crate::grant::Grant;
use crate::host::Host;
use crate::refusal::Refusal;
use crate::registry::{CommandClass, Registry};
use kernel::{CompoundMeta, HistoryOrigin};

/// One command inside a `hew.doc.transact` envelope.
#[derive(Debug, serde::Deserialize)]
pub struct TxCommand {
    pub method: String,
    #[serde(rename = "as", default)]
    pub label: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// The `hew.doc.transact` params.
#[derive(Debug, serde::Deserialize)]
pub struct TxParams {
    #[serde(default)]
    pub label: Option<String>,
    pub commands: Vec<TxCommand>,
    /// Compare-and-swap (docs/agents/HEW_API.md §6.5): refuse
    /// `generation_moved` unless the document's history generation is this
    /// value when the envelope is dispatched.
    #[serde(default)]
    pub expect_generation: Option<u64>,
    /// Coalescing key (§6.5): the commit extends the top undo entry when a
    /// connection with this one's identity (the history origin) committed
    /// it under the same key.
    #[serde(default)]
    pub coalesce: Option<String>,
}

/// The commands that replace the document, which a compound abort cannot
/// restore: refused `document_busy` while a compound is open
/// (docs/agents/PLUGINS.md "Compounds").
pub const DOCUMENT_REPLACING: &[&str] = &[
    "hew.doc.open",
    "hew.doc.new",
    "hew.doc.revert",
    "hew.doc.close",
];

/// The `document_busy` refusal: a document-replacing command while a
/// plugin command holds the document.
pub fn document_busy() -> Refusal {
    Refusal::api(
        "document_busy",
        "A plugin command is running on this document. Wait for it to finish, or cancel it, then try again.",
    )
}

/// How a whole transaction failed.
#[derive(Debug)]
pub enum TxError {
    /// Static envelope defect (`-32602`), before anything ran.
    Invalid(String),
    /// A command outside the grant (`-32001`), with its index.
    NotPermitted(usize),
    /// A restricted grant set a path parameter (`-32001`
    /// `path_not_granted`), with the command's index and the pointer.
    PathNotGranted { index: usize, pointer: &'static str },
    /// A restricted grant without `documents.other` named a document in a
    /// command's `document` parameter (`-32001` `document_not_granted`).
    DocumentNotGranted {
        index: usize,
        document: serde_json::Value,
    },
    /// A command refused at execution — the bracket was aborted, the
    /// document is untouched.
    Refused {
        index: usize,
        method: String,
        refusal: Refusal,
    },
    /// A plugin command shares the envelope with another command
    /// (`-32000` `plugin_command_in_transaction`), with its index. A plugin
    /// command runs only as its own envelope (docs/agents/HEW_API.md §6.4).
    PluginCommandInTransaction(usize),
    /// A kernel invariant failed (`-32003`).
    Internal(String),
}

/// Runs one transaction envelope under `grant`. `identity` becomes the
/// history origin.
pub fn run(
    doc: &mut kernel::Document,
    host: &mut dyn Host,
    registry: &Registry,
    grant: &Grant,
    identity: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, TxError> {
    let parsed: TxParams = serde_json::from_value(params.clone())
        .map_err(|e| TxError::Invalid(format!("malformed transact params: {e}")))?;
    // Before anything else runs, the solitary path and the static pass
    // included.
    precheck(doc, &parsed)?;
    if parsed.commands.is_empty() {
        return Err(TxError::Invalid(
            "a transaction needs at least one command".into(),
        ));
    }

    // §6.4: a solitary command IS reachable as the sole command of its
    // own envelope — that is its canonical invocation from MCP (§13). It
    // runs bare: host effects and history operations are not recorded
    // model mutations a compound entry could roll back, so no bracket
    // opens and no undo entry is added.
    if parsed.commands.len() == 1 {
        let cmd = &parsed.commands[0];
        // A plugin command is never executed here (the dispatcher forwards
        // a sole one); it falls through to the static pass, which refuses.
        if let Some(decl) = registry.get(&cmd.method)
            && decl.class == CommandClass::Solitary
            && !decl.is_plugin()
        {
            if doc.compound_open() && DOCUMENT_REPLACING.contains(&cmd.method.as_str()) {
                return Err(TxError::Refused {
                    index: 0,
                    method: cmd.method.clone(),
                    refusal: document_busy(),
                });
            }
            if !grant.grants(decl) {
                return Err(TxError::NotPermitted(0));
            }
            path_check(grant, &cmd.method, &cmd.params, 0)?;
            check_refs(&cmd.params, &[], 0)?;
            let Some(handler) = commands::handler(&cmd.method) else {
                return Err(TxError::Refused {
                    index: 0,
                    method: cmd.method.clone(),
                    refusal: Refusal::api(
                        "unimplemented",
                        &format!(
                            "{} is declared but not implemented yet in this build.",
                            cmd.method
                        ),
                    ),
                });
            };
            let mut face_tokens = FaceTokens::new();
            let mut ctx = Ctx {
                doc,
                host,
                face_tokens: &mut face_tokens,
                current_label: cmd.label.clone(),
                grant,
            };
            let params = cmd.params.clone().unwrap_or(serde_json::json!({}));
            let label = parsed.label.clone().unwrap_or_else(|| cmd.method.clone());
            return match handler(&mut ctx, &params) {
                Ok(result) => Ok(serde_json::json!({ "results": [result], "label": label })),
                Err(CmdError::Refusal(refusal)) => Err(TxError::Refused {
                    index: 0,
                    method: cmd.method.clone(),
                    refusal,
                }),
                Err(CmdError::Params(msg)) => Err(TxError::Invalid(format!("command 0: {msg}"))),
                Err(CmdError::Internal(msg)) => Err(TxError::Internal(msg)),
            };
        }
    }

    // ── Static pass (§6.2, §6.3, §6.4, §10) — nothing runs on failure ──
    let mut labels: Vec<&str> = Vec::new();
    let mut context_depth: i64 = 0;
    for (i, cmd) in parsed.commands.iter().enumerate() {
        let Some(decl) = registry.get(&cmd.method) else {
            return Err(TxError::Invalid(format!(
                "command {i}: unknown method {}",
                cmd.method
            )));
        };
        if !grant.grants(decl) {
            return Err(TxError::NotPermitted(i));
        }
        path_check(grant, &cmd.method, &cmd.params, i)?;
        if decl.is_plugin() {
            return Err(TxError::PluginCommandInTransaction(i));
        }
        if decl.class == CommandClass::Solitary {
            return Err(TxError::Invalid(format!(
                "command {i}: {} is solitary — it must be the sole command of its own envelope",
                cmd.method
            )));
        }
        // Context balance: every enter exited in this envelope, never
        // closing a frame the envelope didn't open (§6.3).
        match cmd.method.as_str() {
            "hew.context.enter" => context_depth += 1,
            "hew.context.exit" => {
                context_depth -= 1;
                if context_depth < 0 {
                    return Err(TxError::Invalid(format!(
                        "command {i}: hew.context.exit closes a frame this transaction never opened"
                    )));
                }
            }
            _ => {}
        }
        // $ref / $face references must name an EARLIER label.
        check_refs(&cmd.params, &labels, i)?;
        if let Some(label) = &cmd.label {
            if labels.contains(&label.as_str()) {
                return Err(TxError::Invalid(format!(
                    "command {i}: duplicate label '{label}'"
                )));
            }
            labels.push(label);
        }
    }
    if context_depth != 0 {
        return Err(TxError::Invalid(format!(
            "unbalanced context: {context_depth} enter(s) never exited in this envelope"
        )));
    }

    // ── Execute under the kernel bracket ───────────────────────────────
    let bracket = doc.begin_transaction();
    let mut face_tokens: FaceTokens = FaceTokens::new();
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut result_of: std::collections::BTreeMap<String, usize> = Default::default();

    for (i, cmd) in parsed.commands.iter().enumerate() {
        // Substitute $refs against earlier results (dispatch-time — §6.2).
        let params = match substitute_refs(
            cmd.params.clone().unwrap_or(serde_json::json!({})),
            &result_of,
            &results,
        ) {
            Ok(p) => p,
            Err(refusal) => {
                doc.abort_transaction(bracket);
                return Err(TxError::Refused {
                    index: i,
                    method: cmd.method.clone(),
                    refusal,
                });
            }
        };
        let Some(handler) = commands::handler(&cmd.method) else {
            doc.abort_transaction(bracket);
            return Err(TxError::Refused {
                index: i,
                method: cmd.method.clone(),
                refusal: Refusal::api(
                    "unimplemented",
                    &format!(
                        "{} is declared but not implemented yet in this build.",
                        cmd.method
                    ),
                ),
            });
        };
        let mut ctx = Ctx {
            doc,
            host,
            face_tokens: &mut face_tokens,
            current_label: cmd.label.clone(),
            grant,
        };
        match handler(&mut ctx, &params) {
            Ok(result) => {
                if let Some(label) = &cmd.label {
                    result_of.insert(label.clone(), results.len());
                }
                results.push(result);
            }
            Err(CmdError::Refusal(refusal)) => {
                doc.abort_transaction(bracket);
                return Err(TxError::Refused {
                    index: i,
                    method: cmd.method.clone(),
                    refusal,
                });
            }
            Err(CmdError::Params(msg)) => {
                doc.abort_transaction(bracket);
                return Err(TxError::Invalid(format!("command {i}: {msg}")));
            }
            Err(CmdError::Internal(msg)) => {
                doc.abort_transaction(bracket);
                return Err(TxError::Internal(msg));
            }
        }
    }

    let label = parsed
        .label
        .clone()
        .or_else(|| parsed.commands.first().map(|c| c.method.clone()))
        .unwrap_or_else(|| "transaction".to_string());
    doc.commit_transaction(
        bracket,
        CompoundMeta {
            label: label.clone(),
            origin: HistoryOrigin::Connection(identity.to_string()),
            coalesce: parsed.coalesce.clone(),
        },
    )
    .map_err(|e| TxError::Internal(format!("commit failed after balanced execution: {e}")))?;

    Ok(serde_json::json!({ "results": results, "label": label }))
}

/// The envelope-level checks every `hew.doc.transact` gets first, a sole
/// plugin command's included (docs/agents/HEW_API.md §6.5): compare-and-swap
/// (`generation_moved` unless the history generation is `expect_generation`),
/// then the `coalesce` key's length.
pub fn precheck(doc: &kernel::Document, tx: &TxParams) -> Result<(), TxError> {
    if let Some(expected) = tx.expect_generation
        && doc.history_generation() != expected
    {
        let actual = doc.history_generation();
        return Err(TxError::Refused {
            index: 0,
            method: "hew.doc.transact".into(),
            refusal: Refusal::api(
                "generation_moved",
                "The document changed since this envelope was computed. Read it again and resend.",
            )
            .with_detail(serde_json::json!({ "expected": expected, "actual": actual })),
        });
    }
    if let Some(key) = &tx.coalesce
        && (key.is_empty() || key.len() > 64)
    {
        return Err(TxError::Invalid("coalesce must be 1..=64 bytes".into()));
    }
    Ok(())
}

/// The static checks a sole command of an envelope gets before it runs
/// (or, for a plugin command, before it is forwarded): the path and
/// document parameter rules, and `$ref`/`$face` references, of which a
/// sole command can have none that resolve.
pub(crate) fn static_checks(
    grant: &Grant,
    method: &str,
    params: &Option<serde_json::Value>,
    index: usize,
) -> Result<(), TxError> {
    path_check(grant, method, params, index)?;
    check_refs(params, &[], index)
}

/// A restricted grant may not set a path parameter (docs/agents/HEW_API.md
/// §10), checked statically per command so nothing runs on failure. A
/// `$ref` that would fill a path member counts as setting it.
fn path_check(
    grant: &Grant,
    method: &str,
    params: &Option<serde_json::Value>,
    index: usize,
) -> Result<(), TxError> {
    if !grant.is_restricted() {
        return Ok(());
    }
    let params = params.as_ref().unwrap_or(&serde_json::Value::Null);
    if let Some(pointer) = crate::paths::path_param_set(method, params) {
        return Err(TxError::PathNotGranted { index, pointer });
    }
    if !grant.may_address_other_documents()
        && let Some(document) = crate::paths::document_param_set(method, params)
    {
        return Err(TxError::DocumentNotGranted { index, document });
    }
    Ok(())
}

/// Static `$ref`/`$face` label checks: every reference names an earlier
/// label (§6.2 — forward references and unknown labels are `-32602`).
fn check_refs(
    params: &Option<serde_json::Value>,
    labels: &[&str],
    index: usize,
) -> Result<(), TxError> {
    fn walk(v: &serde_json::Value, labels: &[&str], index: usize) -> Result<(), TxError> {
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k == "$ref" || k == "$face" {
                        // A reference object is EXACTLY {"$ref": ...} /
                        // {"$face": ...}: sibling keys would make the
                        // substitution silently partial (§6.2's static
                        // defect posture — reject before anything runs).
                        if map.len() != 1 {
                            return Err(TxError::Invalid(format!(
                                "command {index}: a {k} object must contain exactly that one key"
                            )));
                        }
                        let text = val.as_str().ok_or_else(|| {
                            TxError::Invalid(format!("command {index}: {k} must be a string"))
                        })?;
                        let (label, rest) = text.split_once('#').ok_or_else(|| {
                            TxError::Invalid(format!(
                                "command {index}: {k} '{text}' lacks the '#' separator"
                            ))
                        })?;
                        if rest.is_empty() {
                            return Err(TxError::Invalid(format!(
                                "command {index}: {k} '{text}' names nothing after '#'"
                            )));
                        }
                        if !labels.contains(&label) {
                            return Err(TxError::Invalid(format!(
                                "command {index}: {k} '{text}' names no earlier command label"
                            )));
                        }
                    }
                    walk(val, labels, index)?;
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, labels, index)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    match params {
        Some(v) => walk(v, labels, index),
        None => Ok(()),
    }
}

/// Replaces `{"$ref": "label#/json/pointer"}` values with the pointed-at
/// value from the labeled command's result. A pointer that fails to
/// resolve is the runtime `ref_resolution_failed` refusal (§6.2).
/// `$face` objects are left in place — the locator layer resolves them
/// against the minted tokens.
fn substitute_refs(
    params: serde_json::Value,
    result_of: &std::collections::BTreeMap<String, usize>,
    results: &[serde_json::Value],
) -> Result<serde_json::Value, Refusal> {
    match params {
        serde_json::Value::Object(map) => {
            if map.len() == 1
                && let Some(reference) = map.get("$ref").and_then(serde_json::Value::as_str)
            {
                {
                    let (label, pointer) = reference.split_once('#').expect("checked statically");
                    let index = result_of[label]; // label existence checked statically
                    return results[index].pointer(pointer).cloned().ok_or_else(|| {
                        Refusal::api(
                            "ref_resolution_failed",
                            &format!(
                                "'{reference}' does not resolve: the command labeled '{label}' produced no value at {pointer}."
                            ),
                        )
                        .with_detail(serde_json::json!({ "reference": reference }))
                    });
                }
            }
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k, substitute_refs(v, result_of, results)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(items) => Ok(serde_json::Value::Array(
            items
                .into_iter()
                .map(|v| substitute_refs(v, result_of, results))
                .collect::<Result<_, _>>()?,
        )),
        other => Ok(other),
    }
}
