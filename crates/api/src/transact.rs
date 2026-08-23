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
use crate::host::Host;
use crate::refusal::Refusal;
use crate::registry::{CommandClass, Profile, Registry};
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
}

/// How a whole transaction failed.
#[derive(Debug)]
pub enum TxError {
    /// Static envelope defect (`-32602`), before anything ran.
    Invalid(String),
    /// A command outside the granted profile (`-32001`), with its index.
    NotPermitted(usize),
    /// A command refused at execution — the bracket was aborted, the
    /// document is untouched.
    Refused {
        index: usize,
        method: String,
        refusal: Refusal,
    },
    /// A kernel invariant failed (`-32003`).
    Internal(String),
}

/// Runs one transaction envelope. `identity` becomes the history origin.
pub fn run(
    doc: &mut kernel::Document,
    host: &mut dyn Host,
    registry: &Registry,
    profile: Profile,
    identity: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, TxError> {
    let parsed: TxParams = serde_json::from_value(params.clone())
        .map_err(|e| TxError::Invalid(format!("malformed transact params: {e}")))?;
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
        if let Some(decl) = registry.get(&cmd.method)
            && decl.class == CommandClass::Solitary
        {
            if !profile.grants(decl) {
                return Err(TxError::NotPermitted(0));
            }
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
        if !profile.grants(decl) {
            return Err(TxError::NotPermitted(i));
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
        },
    )
    .map_err(|e| TxError::Internal(format!("commit failed after balanced execution: {e}")))?;

    Ok(serde_json::json!({ "results": results, "label": label }))
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
