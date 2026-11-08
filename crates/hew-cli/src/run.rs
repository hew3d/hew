//! `hew-cli run` and `hew-cli dispatch` (docs/agents/HEW_API.md §12): headless by
//! default, against an embedded `crates/api` connection, a `kernel::Document`,
//! and a [`CliHost`]; with `--live`, every envelope goes instead to a
//! running desktop instance over [`crate::live`]'s local socket transport
//! (§11.2). No MCP wrapping involved. Both print their own stdout/stderr
//! per the spec's shell-scripting posture — a script or a one-shot
//! dispatch is meant to be piped through `jq` — and return a small
//! outcome struct (exit code plus, for tests, the response JSON) so
//! `src/main.rs` stays a thin `std::process::exit` shell and
//! `tests/cli.rs` can call them directly without capturing stdout.

use crate::host::CliHost;
use crate::live::{self, LiveOptions};
use api::{Connection, DispatchOutcome, Grant, Host, Registry, Request, RequestId};
use kernel::Document;
use std::path::Path;

/// What [`run_script`] did: the process exit code, plus every response it
/// printed, in order — tests read the latter instead of capturing stdout.
pub struct RunOutcome {
    pub exit_code: i32,
    pub responses: Vec<serde_json::Value>,
}

/// Runs a script file (JSON array or JSONL — see [`parse_script`]), in
/// order. Headless (`live: None`): against a fresh embedded document
/// through one `Connection`. Live (`live: Some(opts)`): every frame is
/// instead forwarded to a running desktop instance over the local socket
/// transport (docs/agents/HEW_API.md §11.2, §12) — the script's own first frame
/// carries the discovery handshake, and, if it means to mutate anything,
/// a `hew.doc.attach` of its own (a live host serves an already-open
/// document rather than creating one — see [`dispatch_live`]'s doc
/// comment for the full rationale, which applies here too). Either way,
/// prints each reply as one JSON line to stdout; on the first error
/// response (refusal or otherwise) prints it to stderr as JSON and stops
/// with exit code 1. `out_path`, if given, saves the resulting document
/// after the script completes — headless only: a live host keeps
/// document persistence user-driven (there is no local `Document` for
/// `--out` to serialize, and the live host refuses a remote
/// `hew.doc.save` outright by design), so `--out` together with `--live`
/// is rejected up front as a usage error rather than attempted and left
/// to fail on the script's last step.
///
/// The script is a raw envelope sequence — it must open with its own
/// `hew.meta.hello` (protocol 1) and, if it means to mutate anything,
/// `hew.doc.new`/`open` (headless) or `hew.doc.attach` (live). Nothing is
/// auto-injected: an agent-authored script and a hand-written one look
/// identical.
pub fn run_script(
    script_path: &Path,
    out_path: Option<&Path>,
    live: Option<&LiveOptions>,
) -> RunOutcome {
    if out_path.is_some() && live.is_some() {
        // There is no local `Document` for `--out` to serialize in live
        // mode — the document lives in the app. A script that wants a
        // file says so with an explicit `hew.doc.save` carrying a path,
        // which this runner writes for you. Silently mapping `--out` onto
        // that would hide which frame did the saving, so it is a usage
        // error instead.
        eprintln!(
            "hew-cli run: --out is not available in live mode: add a hew.doc.save frame with a path to your script, or run headless"
        );
        return RunOutcome {
            exit_code: 2,
            responses: Vec::new(),
        };
    }
    let mut responses = Vec::new();
    let text = match std::fs::read_to_string(script_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hew-cli run: {}: {e}", script_path.display());
            return RunOutcome {
                exit_code: 1,
                responses,
            };
        }
    };
    let frames = match parse_script(&text) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("hew-cli run: {}: {e}", script_path.display());
            return RunOutcome {
                exit_code: 1,
                responses,
            };
        }
    };

    match live {
        None => run_script_embedded(frames, out_path, responses),
        Some(opts) => match run_script_live(frames, opts, &mut responses) {
            Ok(()) => RunOutcome {
                exit_code: 0,
                responses,
            },
            Err(exit_code) => RunOutcome {
                exit_code,
                responses,
            },
        },
    }
}

fn run_script_embedded(
    frames: Vec<serde_json::Value>,
    out_path: Option<&Path>,
    mut responses: Vec<serde_json::Value>,
) -> RunOutcome {
    let mut conn = Connection::new(Grant::core(), "hew-cli:run");
    let registry = Registry::protocol_1();
    let mut doc = Document::new();
    let mut host = CliHost::new();

    for (i, raw) in frames.into_iter().enumerate() {
        let request: Request = match serde_json::from_value(raw) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("hew-cli run: frame {i}: malformed envelope: {e}");
                return RunOutcome {
                    exit_code: 1,
                    responses,
                };
            }
        };
        match conn.dispatch(&registry, &mut doc, &mut host, request) {
            DispatchOutcome::Forward(forward) => {
                unreachable!(
                    "hew-cli registers no plugins, yet {} forwarded",
                    forward.method
                )
            }
            DispatchOutcome::Dropped => {
                // A client-originated notification (no id) — dropped
                // unexecuted, per §4.1; nothing to print.
            }
            DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response) => {
                if record_reply(&mut responses, &response) {
                    return RunOutcome {
                        exit_code: 1,
                        responses,
                    };
                }
            }
        }
    }

    if let Some(out) = out_path {
        let Some(out_str) = out.to_str() else {
            eprintln!("hew-cli run: --out path is not valid UTF-8");
            return RunOutcome {
                exit_code: 1,
                responses,
            };
        };
        if let Err(refusal) = host.save_document(&doc, Some(out_str), None, None) {
            eprintln!(
                "{}",
                serde_json::to_string(&refusal_data(&refusal)).unwrap()
            );
            return RunOutcome {
                exit_code: 1,
                responses,
            };
        }
    }

    RunOutcome {
        exit_code: 0,
        responses,
    }
}

/// Forwards every frame of a script to a live instance. The first frame
/// must be `hew.meta.hello` — that is what carries the discovery token
/// (docs/agents/HEW_API.md §11.2) — everything after it is forwarded as-is (the
/// script is free to include its own `hew.doc.attach`, exactly like any
/// other command it sends). Returns `Err(exit_code)` on the first failure
/// (malformed frame, transport error, or a refused/errored reply), having
/// already printed whatever should be printed.
fn run_script_live(
    frames: Vec<serde_json::Value>,
    opts: &LiveOptions,
    responses: &mut Vec<serde_json::Value>,
) -> Result<(), i32> {
    if frames.is_empty() {
        eprintln!("hew-cli run --live: script is empty");
        return Err(1);
    }
    let first: Request = serde_json::from_value(frames[0].clone()).map_err(|e| {
        eprintln!("hew-cli run --live: frame 0: malformed envelope: {e}");
        1
    })?;
    if first.method != "hew.meta.hello" {
        eprintln!(
            "hew-cli run --live: the script's first frame must be hew.meta.hello — that is what carries the live handshake's discovery token"
        );
        return Err(1);
    }

    let mut session = live::connect_live(opts, first).map_err(|e| {
        eprintln!("hew-cli run --live: {e}");
        1
    })?;
    if record_reply(responses, &session.hello_response) {
        return Err(1);
    }
    // `--document`: bind to it up front, so a script can stay ignorant of
    // which window it was pointed at. A script may still attach (or name a
    // `document` per frame) itself.
    if let Some(document) = opts.document.as_deref()
        && let Err(msg) = attach_live(&mut session, Some(document))
    {
        eprintln!("hew-cli run --live: {msg}");
        return Err(1);
    }

    for (i, raw) in frames.into_iter().enumerate().skip(1) {
        let mut request: Request = serde_json::from_value(raw).map_err(|e| {
            eprintln!("hew-cli run --live: frame {i}: malformed envelope: {e}");
            1
        })?;
        let params = request
            .params
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

        // `hew.library.list`/`describe`/`remove`/`update_meta`: answered
        // entirely on this side, no dispatch to the remote at all — see
        // `dispatch_live`'s identical short-circuit.
        if crate::live_library::is_local_only(&request.method, &params) {
            let response = crate::live_library::dispatch_local(&request.method, &params);
            if record_reply(responses, &response) {
                return Err(1);
            }
            continue;
        }

        let client_params = params.clone();
        let params = match crate::live_library::resolve_insert_item(&request.method, params) {
            Ok(p) => p,
            Err(response) => {
                record_reply(responses, &response);
                return Err(1);
            }
        };
        let params = crate::live_library::force_return_bytes(&request.method, params);
        // A save/export inside a script gets the same client-side write a
        // one-shot dispatch does — see `take_client_write_path`.
        let (params, write_path) = take_client_write_path(&request.method, params);
        request.params = Some(params);
        let method = request.method.clone();
        match session.dispatch(request) {
            // A live session reads replies off the socket; it never
            // forwards.
            Ok(DispatchOutcome::Dropped | DispatchOutcome::Forward(_)) => {}
            Ok(DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) => {
                let value = serde_json::to_value(&response).expect("Response serializes");
                if response.error.is_some() {
                    println!("{value}");
                    responses.push(value);
                    return Err(1);
                }
                if let Some(outcome) =
                    crate::live_library::finish_save(&method, &client_params, &value)
                {
                    match outcome {
                        Ok(rewritten) => {
                            // The canonical JSON-RPC shape every other
                            // recorded reply carries (see `dispatch_live`'s
                            // identical wrap).
                            let wrapped = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": value.get("id").cloned().unwrap_or(serde_json::Value::Null),
                                "result": rewritten,
                            });
                            println!("{wrapped}");
                            responses.push(wrapped);
                        }
                        Err(msg) => {
                            eprintln!("hew-cli run --live: frame {i}: {msg}");
                            return Err(1);
                        }
                    }
                    continue;
                }
                // A save/export we are about to write must NOT go through
                // `record_reply`: that prints the reply and keeps it, and
                // the reply is the whole document or mesh in base64. The
                // caller wants the file, so report the file — the same
                // thing a one-shot dispatch reports.
                if let Some(path) = write_path {
                    let saved = match finish_client_write(&value, &path) {
                        Ok(saved) => saved,
                        Err(msg) => {
                            // The same reply a host that owned the filesystem
                            // would have given, on stdout where a script reads it.
                            let failed = write_failure_reply(&value, &method, &msg);
                            println!("{failed}");
                            responses.push(failed);
                            return Err(1);
                        }
                    };
                    println!("{saved}");
                    responses.push(saved);
                } else if record_reply(responses, &response) {
                    return Err(1);
                }
            }
            Err(e) => {
                eprintln!("hew-cli run --live: frame {i}: {e}");
                return Err(1);
            }
        }
    }
    Ok(())
}

/// Prints one reply as its own JSON line (stdout) and, if it is an error,
/// also its error object (stderr) — shared by both the embedded and live
/// script loops. Returns whether the reply was an error (the caller's
/// signal to stop).
fn record_reply(responses: &mut Vec<serde_json::Value>, response: &api::Response) -> bool {
    let value = serde_json::to_value(response).expect("Response serializes");
    println!("{value}");
    let is_error = response.error.is_some();
    if is_error {
        eprintln!(
            "{}",
            serde_json::to_string(&response.error).expect("ErrorObject serializes")
        );
    }
    responses.push(value);
    is_error
}

/// A script file is a JSON array of envelopes, or JSONL (one envelope per
/// line) — detected by the first non-whitespace byte: `[` means array,
/// anything else means JSONL. Blank lines in a JSONL script are skipped.
fn parse_script(text: &str) -> Result<Vec<serde_json::Value>, String> {
    // Windows tooling writes a UTF-8 BOM by default — Notepad, VS Code's
    // "UTF-8 with BOM", and Windows PowerShell 5.1's `Set-Content
    // -Encoding utf8` all do. A JSON parser sees it as a stray character
    // and answers "expected value at line 1 column 1", which says nothing
    // about the real problem. A script file is exactly the kind of thing
    // someone authors in those editors, so accept the BOM and move on.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<serde_json::Value>>(text).map_err(|e| e.to_string())
    } else {
        trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str::<serde_json::Value>(line).map_err(|e| e.to_string()))
            .collect()
    }
}

/// What [`dispatch_file`] did: the process exit code, plus the requested
/// command's own response (not the internal hello/open plumbing).
pub struct DispatchResult {
    pub exit_code: i32,
    pub response: Option<serde_json::Value>,
}

/// `hew-cli dispatch <method> [params] --file <model.hew>` (docs/agents/HEW_API.md
/// §12): opens `file`, dispatches exactly one envelope built from
/// `method`/`params`, prints the reply to stdout, and — for anything but a
/// read-only command — saves the document back in place. Internal
/// `hello`/`open` plumbing is silent; only the requested command's reply
/// (or a plumbing failure) reaches stdout/stderr.
pub fn dispatch_file(file: &Path, method: &str, params: serde_json::Value) -> DispatchResult {
    let mut conn = Connection::new(Grant::core(), "hew-cli:dispatch");
    let registry = Registry::protocol_1();
    let mut doc = Document::new();
    let mut host = CliHost::new();

    let Some(path) = file.to_str() else {
        eprintln!("hew-cli dispatch: --file path is not valid UTF-8");
        return DispatchResult {
            exit_code: 1,
            response: None,
        };
    };

    if let Some(failure) = run_silently(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.meta.hello",
        serde_json::json!({ "protocol": 1 }),
    ) {
        eprintln!("hew-cli dispatch: hello: {failure}");
        return DispatchResult {
            exit_code: 1,
            response: None,
        };
    }
    if let Some(failure) = run_silently(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.doc.open",
        serde_json::json!({ "path": path }),
    ) {
        eprintln!("hew-cli dispatch: open {path}: {failure}");
        return DispatchResult {
            exit_code: 1,
            response: None,
        };
    }

    // Save back only after a command that can change the document — the
    // registry's own `mutates_document` (a read-only query, a snapshot, a
    // line drawing, a print, or a dry run leaves the file byte-for-byte
    // alone).
    let read_only = registry
        .get(method)
        .map(|c| !c.mutates_document)
        .unwrap_or(false);

    let request = Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("dispatch".to_string())),
        method: method.to_string(),
        params: Some(params),
    };
    // A dry run changed nothing, so nothing is saved after it either.
    let (response, read_only) = match conn.dispatch(&registry, &mut doc, &mut host, request) {
        DispatchOutcome::Reply(response) => (response, read_only),
        DispatchOutcome::DryRun(response) => (response, true),
        DispatchOutcome::Dropped | DispatchOutcome::Forward(_) => {
            eprintln!(
                "hew-cli dispatch: {method} is a notification-shaped request internally — this is a bug"
            );
            return DispatchResult {
                exit_code: 1,
                response: None,
            };
        }
    };
    let value = serde_json::to_value(&response).expect("Response serializes");
    println!("{value}");
    if response.error.is_some() {
        return DispatchResult {
            exit_code: 1,
            response: Some(value),
        };
    }

    if !read_only && let Err(refusal) = host.save_document(&doc, None, None, None) {
        eprintln!(
            "{}",
            serde_json::to_string(&refusal_data(&refusal)).unwrap()
        );
        return DispatchResult {
            exit_code: 1,
            response: Some(value),
        };
    }

    DispatchResult {
        exit_code: 0,
        response: Some(value),
    }
}

/// `hew-cli dispatch <method> [params] --live` (docs/agents/HEW_API.md §12):
/// connects to a running desktop instance, attaches to its document, and
/// forwards exactly one envelope built from `method`/`params`, printing
/// the reply. Unlike `--file`, there is no save-back step: a live edit
/// already lands directly in the app's document and its own undo history
/// the moment the remote answers — nothing here needs a separate
/// persist.
///
/// The attach step, and why it exists here at all: a live host serves a
/// document the user already has open rather than creating one on
/// connect, so a fresh connection is not bound to anything until it asks
/// (docs/agents/HEW_API.md §4.2's `hew.doc.attach`, §10's "document lifecycle
/// stays user-driven"). The embedded/`--file` paths never notice this —
/// `hew.doc.new`/`open` auto-attach on success — but a one-shot `dispatch
/// --live` only ever sends the single envelope the caller asked for, with
/// no chance to run `hew.doc.attach` as a separate step first; without
/// this, every live dispatch would answer `-32002 no document attached`
/// against a real desktop instance. [`crate::mcp::McpServer::new_live`]
/// has the identical problem and the identical fix, via [`attach_live`].
pub fn dispatch_live(
    method: &str,
    params: serde_json::Value,
    opts: &LiveOptions,
) -> DispatchResult {
    // `hew.library.list`/`describe`/`remove`/`update_meta` never touch the
    // live document (docs/design/v1.1-cycle.md's Lane B) — answered
    // entirely on this side, with no live connection needed at all.
    if crate::live_library::is_local_only(method, &params) {
        let response = crate::live_library::dispatch_local(method, &params);
        let value = serde_json::to_value(&response).expect("Response serializes");
        println!("{value}");
        return DispatchResult {
            exit_code: if response.error.is_some() { 1 } else { 0 },
            response: Some(value),
        };
    }

    let hello_request = live::build_hello_request("hew-cli:dispatch");
    let mut session = match live::connect_live(opts, hello_request) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hew-cli dispatch --live: {e}");
            return DispatchResult {
                exit_code: 1,
                response: None,
            };
        }
    };
    if let Err(msg) = attach_live(&mut session, opts.document.as_deref()) {
        eprintln!("hew-cli dispatch --live: {msg}");
        return DispatchResult {
            exit_code: 1,
            response: None,
        };
    }

    // `hew.library.insert {item}`: the remote's own host has no
    // filesystem to resolve `item` with, so this side (which does)
    // resolves it to `bytes_base64` before it ever reaches the wire.
    let client_params = params.clone();
    let params = match crate::live_library::resolve_insert_item(method, params) {
        Ok(p) => p,
        Err(response) => {
            let value = serde_json::to_value(&response).expect("Response serializes");
            println!("{value}");
            return DispatchResult {
                exit_code: 1,
                response: Some(value),
            };
        }
    };
    // `hew.library.save`: force the bytes back so this side can write
    // them to its own library folder after a successful reply (below).
    let params = crate::live_library::force_return_bytes(method, params);

    // `hew.doc.save`/`hew.doc.export` with a path: the live host can
    // produce the bytes (serializing the document, or writing STL/3MF/GLB
    // through `crates/mesh-export`) but has no disk to put them on, so
    // this strips `path` before forwarding, asks for the bytes back, and
    // writes them here, where there IS one. The user typed the same
    // command they would headless and gets the same file — the effect
    // just happens on the side that can perform it.
    let (params, client_write_path) = take_client_write_path(method, params);

    let request = Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("dispatch".to_string())),
        method: method.to_string(),
        params: Some(params),
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) => {
            let value = serde_json::to_value(&response).expect("Response serializes");
            if response.error.is_some() {
                println!("{value}");
                return DispatchResult {
                    exit_code: 1,
                    response: Some(value),
                };
            }
            if let Some(outcome) = crate::live_library::finish_save(method, &client_params, &value)
            {
                return match outcome {
                    Ok(rewritten) => {
                        // The canonical JSON-RPC shape every other
                        // `DispatchResult::response` carries — `id`
                        // correlated to the reply that just arrived, not a
                        // bare object, so a caller parsing `.response`
                        // never has to special-case "this one command's
                        // result isn't wrapped."
                        let wrapped = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": value.get("id").cloned().unwrap_or(serde_json::Value::Null),
                            "result": rewritten,
                        });
                        println!("{wrapped}");
                        DispatchResult {
                            exit_code: 0,
                            response: Some(wrapped),
                        }
                    }
                    Err(msg) => {
                        eprintln!("hew-cli dispatch --live: {msg}");
                        DispatchResult {
                            exit_code: 1,
                            response: Some(value),
                        }
                    }
                };
            }
            if let Some(path) = client_write_path {
                match finish_client_write(&value, &path) {
                    Ok(saved) => println!("{saved}"),
                    Err(msg) => {
                        let failed = write_failure_reply(&value, method, &msg);
                        println!("{failed}");
                        return DispatchResult {
                            exit_code: 1,
                            response: Some(failed),
                        };
                    }
                }
            } else {
                println!("{value}");
            }
            DispatchResult {
                exit_code: 0,
                response: Some(value),
            }
        }
        Ok(DispatchOutcome::Dropped | DispatchOutcome::Forward(_)) => {
            unreachable!("a live session answers an id-carrying request with a reply")
        }
        Err(e) => {
            eprintln!("hew-cli dispatch --live: {e}");
            DispatchResult {
                exit_code: 1,
                response: None,
            }
        }
    }
}

/// Splits a live `hew.doc.save`/`hew.doc.export` envelope into the params
/// to forward and the path to write here afterwards.
///
/// A live host can serialize a document or encode a mesh — neither needs
/// a disk — but it cannot write the result, so it refuses a `path`
/// outright. Stripping the path and writing the returned bytes on this
/// side means the same command a user types headless produces the same
/// file live. Every live entry point does this, so a script and a
/// one-shot dispatch behave identically; wiring it into only one of them
/// is a difference nobody could explain.
pub(crate) fn take_client_write_path(
    method: &str,
    params: serde_json::Value,
) -> (serde_json::Value, Option<String>) {
    match method {
        // `hew.doc.save` is NOT here: the app saves through its own save
        // path (file written, unsaved mark cleared, document bound to the
        // file), which a client-side write could not do. Export has no such
        // state and is still written here.
        "hew.doc.export" => strip_path(params),
        // The live host renders and hands the file back as bytes, but it
        // has no disk: with a `path` it refuses `host_capability_missing`.
        // Stripped here, the same bytes come back inline and are written
        // once, on this side (`finish_client_write`). An older app that
        // cannot render at all still answers through the client-side
        // fallback in `live.rs`.
        "hew.view.snapshot" | "hew.view.line_drawing" => strip_render_path(method, params),
        // A solitary command wrapped in a one-command transaction is the
        // SAME command — §6.4 calls that the canonical way to invoke one,
        // and §13 gives an MCP client no other shape for a command that
        // is not read-only. Handling only the bare name meant every MCP
        // save/export with a path refused live while the identical
        // headless call wrote the file.
        "hew.doc.transact" => {
            let Some(commands) = params.get("commands").and_then(|c| c.as_array()) else {
                return (params, None);
            };
            let [only] = commands.as_slice() else {
                return (params, None);
            };
            let inner_method = match only.get("method").and_then(|m| m.as_str()) {
                Some(m @ ("hew.doc.export" | "hew.view.snapshot" | "hew.view.line_drawing")) => m,
                _ => return (params, None),
            };
            let inner = only.get("params").cloned().unwrap_or_else(empty_object);
            let (inner, path) = if inner_method == "hew.doc.export" {
                strip_path(inner)
            } else {
                strip_render_path(inner_method, inner)
            };
            let Some(path) = path else {
                return (params, None);
            };
            let mut rewritten = params;
            rewritten["commands"][0]["params"] = inner;
            (rewritten, Some(path))
        }
        _ => (params, None),
    }
}

/// [`strip_path`] for a render command, except where the path is not a
/// plain "write the bytes here": a line drawing's `path` applies to `svg`
/// only, so a `segments` request keeps it and the host refuses it as it
/// would headless.
fn strip_render_path(
    method: &str,
    params: serde_json::Value,
) -> (serde_json::Value, Option<String>) {
    if method == "hew.view.line_drawing"
        && params.get("format").and_then(|f| f.as_str()) == Some("segments")
    {
        return (params, None);
    }
    strip_path(params)
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

/// Removes a string `path` field, reporting what it removed.
fn strip_path(params: serde_json::Value) -> (serde_json::Value, Option<String>) {
    let Some(path) = params
        .get("path")
        .and_then(|p| p.as_str())
        .map(String::from)
    else {
        return (params, None);
    };
    let mut stripped = params;
    if let Some(obj) = stripped.as_object_mut() {
        obj.remove("path");
    }
    (stripped, Some(path))
}

/// Decodes standard base64 (the encoding `crates/api` hand-rolls for
/// `bytes_base64`). `None` on any character outside the alphabet, a bad
/// length, or misplaced padding — a malformed payload is never guessed at.
pub(crate) fn decode_base64(s: &str) -> Option<Vec<u8>> {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let s = s.trim().as_bytes();
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && chunk[3] != b'=') {
            return None;
        }
        let mut acc: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = if c == b'=' {
                if i < 4 - pad {
                    return None;
                }
                0
            } else {
                ALPHA.iter().position(|&a| a == c)? as u32
            };
            acc = (acc << 6) | v;
        }
        let bytes = acc.to_be_bytes();
        out.push(bytes[1]);
        if pad < 2 {
            out.push(bytes[2]);
        }
        if pad < 1 {
            out.push(bytes[3]);
        }
    }
    Some(out)
}

/// The JSON-RPC reply for a client-side write that failed: the canonical
/// refusal envelope (`failed_index`, `failed_method`, `detail`,
/// `explanation`) a filesystem host answers for the same failure, with the
/// request's own `id`. `message` is [`write_live_bytes`]'s error — the
/// refusal JSON it built, or plain text for a reply with no bytes in it.
pub(crate) fn write_failure_reply(
    reply: &serde_json::Value,
    method: &str,
    message: &str,
) -> serde_json::Value {
    let id = reply.get("id").cloned().unwrap_or(serde_json::Value::Null);
    match serde_json::from_str::<serde_json::Value>(message) {
        Ok(refusal) if refusal.get("refusal").is_some() => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32000,
                "message": "refused",
                "data": {
                    "refusal": refusal["refusal"],
                    "failed_index": 0,
                    "failed_method": method,
                    "detail": refusal.get("detail").cloned().unwrap_or_else(empty_object),
                    "explanation": refusal["explanation"],
                },
            },
        }),
        _ => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32003, "message": message },
        }),
    }
}

/// Writes what a live reply carried (its document or mesh bytes, a snapshot's
/// images, a line drawing's SVG) to `path` and returns the result a
/// filesystem host would have answered: `{ "saved": path }` for a save or
/// export, and for a render the same members headless gives, with the file
/// paths in place of the inline base64 (`path`, `id_buffer_path`,
/// `depth_path`, `normals_path`).
pub(crate) fn finish_client_write(
    response: &serde_json::Value,
    path: &str,
) -> Result<serde_json::Value, String> {
    let result = response
        .pointer("/result/results/0")
        .or_else(|| response.pointer("/result"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let decode = |b64: &str| -> Result<Vec<u8>, String> {
        decode_base64(b64).ok_or_else(|| save_failed(path, "the render bytes were malformed"))
    };
    if let Some(png) = result.get("png_base64").and_then(|v| v.as_str()) {
        // Decode everything before touching the disk, so malformed bytes
        // leave no partial result.
        let mut files: Vec<(String, Vec<u8>)> = vec![(path.to_string(), decode(png)?)];
        let mut out = serde_json::json!({
            "path": path,
            "width": result["width"],
            "height": result["height"],
        });
        let mut stale = Vec::new();
        for (inline, suffix, file_key) in [
            ("id_buffer_base64", ".ids.bin", "id_buffer_path"),
            ("depth_png_base64", ".depth.png", "depth_path"),
            ("normals_png_base64", ".normals.png", "normals_path"),
        ] {
            let to = format!("{path}{suffix}");
            match result.get(inline).and_then(|v| v.as_str()) {
                Some(b64) => {
                    files.push((to.clone(), decode(b64)?));
                    out[file_key] = serde_json::Value::String(to);
                }
                None => stale.push(to),
            }
        }
        for key in ["id_palette", "depth_near", "depth_far", "depth_bits"] {
            if let Some(v) = result.get(key) {
                out[key] = v.clone();
            }
        }
        // A companion an earlier run left beside this path is not part of
        // this result.
        for old in stale {
            match std::fs::remove_file(&old) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(save_failed(&old, &e.to_string())),
            }
        }
        let mut written: Vec<&str> = Vec::new();
        for (to, bytes) in &files {
            if let Err(msg) = write_file(to, bytes) {
                for done in written {
                    let _ = std::fs::remove_file(done);
                }
                return Err(msg);
            }
            written.push(to);
        }
        return Ok(out);
    }
    if let Some(svg) = result.get("svg").and_then(|v| v.as_str()) {
        write_file(path, svg.as_bytes())?;
        return Ok(serde_json::json!({
            "path": path,
            "count": result["count"],
            "bounds": result["bounds"],
        }));
    }
    write_live_bytes(response, path)?;
    Ok(serde_json::json!({ "saved": path }))
}

/// The `save_failed` refusal text every client-side write failure answers.
fn save_failed(path: &str, why: &str) -> String {
    serde_json::json!({
        "refusal": "save_failed",
        "explanation": format!("{path}: {why}"),
        "detail": { "path": path },
    })
    .to_string()
}

/// `std::fs::write` answering the `save_failed` refusal text
/// [`write_live_bytes`] does, so every client-side write fails alike.
fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| save_failed(path, &e.to_string()))
}

/// Writes the bytes a live `hew.doc.save` or `hew.doc.export` handed back
/// (see [`dispatch_live`]) to `path` — the `.hew` document for save, the
/// exported STL/3MF/GLB for export; both shapes carry the same
/// `bytes_base64` field the live host base64s them into, since it has no
/// filesystem of its own. This side decodes and writes.
pub(crate) fn write_live_bytes(response: &serde_json::Value, path: &str) -> Result<(), String> {
    // Both shapes are real: a bare `hew.doc.save` answers its own result,
    // while the one-command transaction §6.4 calls canonical (and the
    // only shape MCP can send) wraps it in `results`.
    let b64 = response
        .pointer("/result/results/0/bytes_base64")
        .or_else(|| response.pointer("/result/bytes_base64"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "the host returned no document bytes to write".to_string())?;
    let bytes =
        decode_base64(b64).ok_or_else(|| save_failed(path, "the document bytes were malformed"))?;
    // A write that fails here fails for the same reasons it would on a
    // host that owned the filesystem, so it answers the same refusal
    // that host would (`save_failed`, declared for both commands) rather
    // than free text — a client parsing refusals should not have to
    // special-case which side of the connection did the writing.
    write_file(path, &bytes)
}

/// `hew-cli documents --live`: prints one JSON object per open document.
/// `hew.meta.documents` is host-wide, so no attach is needed (or done) —
/// listing the windows must work whatever state the connection is in.
pub fn list_documents_live(opts: &LiveOptions) -> DispatchResult {
    let hello_request = live::build_hello_request("hew-cli:documents");
    let mut session = match live::connect_live(opts, hello_request) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hew-cli documents --live: {e}");
            return DispatchResult {
                exit_code: 1,
                response: None,
            };
        }
    };
    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("live-documents".to_string())),
        method: "hew.meta.documents".to_string(),
        params: Some(serde_json::json!({})),
        document: None,
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) => {
            let value = serde_json::to_value(&response).expect("Response serializes");
            if response.error.is_some() {
                eprintln!("{value}");
                return DispatchResult {
                    exit_code: 1,
                    response: Some(value),
                };
            }
            if let Some(documents) = value
                .pointer("/result/documents")
                .and_then(|d| d.as_array())
            {
                for document in documents {
                    println!("{document}");
                }
            }
            DispatchResult {
                exit_code: 0,
                response: Some(value),
            }
        }
        Ok(DispatchOutcome::Dropped | DispatchOutcome::Forward(_)) => {
            unreachable!("the request carries an id, and a live session never forwards")
        }
        Err(e) => {
            eprintln!("hew-cli documents --live: {e}");
            DispatchResult {
                exit_code: 1,
                response: None,
            }
        }
    }
}

/// `hew-cli events --live` (docs/agents/HEW_API.md §4.5, §12): connects,
/// attaches (to `opts.document`, else the front window), subscribes to
/// `filter` (every event name when `None`), prints the subscribe reply to
/// stderr, then prints every frame that arrives, compact JSON one per line,
/// to stdout until the app closes the connection. Exit 0 when it does, 1 on
/// any other failure.
pub fn events_live(opts: &LiveOptions, filter: Option<Vec<String>>) -> i32 {
    let hello_request = live::build_hello_request("hew-cli:events");
    let mut session = match live::connect_live(opts, hello_request) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hew-cli events --live: {e}");
            return 1;
        }
    };
    let mut out = std::io::stdout().lock();
    events_session(&mut session, opts.document.as_deref(), filter, &mut out)
}

/// [`events_live`] once connected: what it does with the session, writing
/// frames to `out`.
pub fn events_session(
    session: &mut live::LiveSession,
    document: Option<&str>,
    filter: Option<Vec<String>>,
    out: &mut dyn std::io::Write,
) -> i32 {
    if let Err(msg) = attach_live(session, document) {
        eprintln!("hew-cli events --live: {msg}");
        return 1;
    }
    let events: Vec<String> = filter.unwrap_or_else(|| {
        api::events::EVENT_NAMES
            .iter()
            .map(|n| (*n).to_string())
            .collect()
    });
    let request = Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("live-subscribe".to_string())),
        method: "hew.event.subscribe".to_string(),
        params: Some(serde_json::json!({ "events": events })),
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) => {
            let value = serde_json::to_value(&response).expect("Response serializes");
            eprintln!("{value}");
            if response.error.is_some() {
                return 1;
            }
        }
        Ok(DispatchOutcome::Dropped | DispatchOutcome::Forward(_)) => {
            unreachable!("the request carries an id, and a live session never forwards")
        }
        Err(e) => {
            eprintln!("hew-cli events --live: {e}");
            return 1;
        }
    }
    let print = |out: &mut dyn std::io::Write, frame: &serde_json::Value| -> bool {
        writeln!(out, "{frame}").and_then(|()| out.flush()).is_ok()
    };
    for frame in session.take_events() {
        if !print(out, &frame) {
            return 1;
        }
    }
    loop {
        match session.read_frame(None) {
            Ok(frame) => {
                if !print(out, &frame) {
                    return 1;
                }
            }
            Err(live::LiveError::Closed) => return 0,
            Err(e) => {
                eprintln!("hew-cli events --live: {e}");
                return 1;
            }
        }
    }
}

/// Dispatches `hew.doc.attach` (`{}` params) over an already hello'd live
/// session — the shared step [`dispatch_live`] and
/// [`crate::mcp::McpServer::new_live`] both need, since neither gets to
/// run a scripted `hew.doc.attach` of its own the way a `run --live`
/// script can (that script controls every frame it sends). Returns a
/// clear message on refusal or transport error; `Ok(())` once the
/// connection is attached and every subsequent command can proceed.
pub(crate) fn attach_live(
    session: &mut live::LiveSession,
    document: Option<&str>,
) -> Result<(), String> {
    let params = match document {
        Some(document) => serde_json::json!({ "document": document }),
        None => serde_json::json!({}),
    };
    let request = Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("live-attach".to_string())),
        method: "hew.doc.attach".to_string(),
        params: Some(params),
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) => {
            match response.error {
                None => Ok(()),
                Some(err) => {
                    // A refusal explains itself (`unknown_document` lists the
                    // open documents); the bare message alone is just "refused".
                    let why = err
                        .data
                        .as_ref()
                        .and_then(|d| d.get("explanation"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(err.message.as_str());
                    Err(format!(
                        "hew.doc.attach was refused: {why} (code {})",
                        err.code
                    ))
                }
            }
        }
        Ok(DispatchOutcome::Dropped | DispatchOutcome::Forward(_)) => {
            unreachable!("attach's request always carries an id, and a live session never forwards")
        }
        Err(e) => Err(format!("hew.doc.attach: {e}")),
    }
}

/// Dispatches one internal (non-user-visible) command and returns its
/// refusal/error message on failure, or `None` on success.
fn run_silently(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut CliHost,
    method: &str,
    params: serde_json::Value,
) -> Option<String> {
    let request = Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text(format!("internal-{method}"))),
        method: method.to_string(),
        params: Some(params),
    };
    let (DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response)) =
        conn.dispatch(&Registry::protocol_1(), doc, host, request)
    else {
        return Some("dropped unexpectedly".to_string());
    };
    response.error.map(|e| {
        e.data
            .and_then(|d| {
                d.get("explanation")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or(e.message)
    })
}

fn refusal_data(refusal: &api::Refusal) -> serde_json::Value {
    serde_json::json!({
        "refusal": refusal.name,
        "explanation": refusal.explanation,
        "detail": refusal.detail,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_script;

    /// A script written on Windows carries a UTF-8 BOM (Notepad, VS
    /// Code's "UTF-8 with BOM", Windows PowerShell 5.1's `Set-Content
    /// -Encoding utf8`). It must parse, not die with an opaque "expected
    /// value at line 1 column 1" that points at nothing.
    #[test]
    fn a_script_written_on_windows_with_a_bom_parses() {
        let jsonl = "\u{feff}{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hew.meta.hello\"}\n\
                     {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hew.doc.attach\"}\n";
        let frames = parse_script(jsonl).expect("a BOM must not break parsing");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["method"], "hew.meta.hello");

        let array = "\u{feff}[{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hew.meta.hello\"}]";
        let frames = parse_script(array).expect("the array form too");
        assert_eq!(frames.len(), 1);
    }

    /// The ordinary BOM-less forms keep working unchanged.
    #[test]
    fn parse_script_reads_jsonl_and_array_without_a_bom() {
        assert_eq!(
            parse_script("{\"a\":1}\n\n{\"a\":2}\n")
                .expect("jsonl")
                .len(),
            2
        );
        assert_eq!(parse_script("[{\"a\":1}]").expect("array").len(), 1);
    }
}

#[cfg(test)]
mod save_roundtrip_tests {
    use super::{decode_base64, finish_client_write, take_client_write_path, write_failure_reply};
    use serde_json::json;

    /// A live render with a `path` is sent without it and written here, once.
    #[test]
    fn a_live_render_command_has_its_path_stripped_bare_or_wrapped() {
        for method in ["hew.view.snapshot", "hew.view.line_drawing"] {
            let (params, path) =
                take_client_write_path(method, json!({"path": "/tmp/a.png", "width": 64}));
            assert_eq!(path.as_deref(), Some("/tmp/a.png"));
            assert!(params.get("path").is_none());
            assert_eq!(params["width"], 64);

            let wrapped =
                json!({"commands":[{"method": method, "params": {"path": "/tmp/a.png"}}]});
            let (params, path) = take_client_write_path("hew.doc.transact", wrapped);
            assert_eq!(path.as_deref(), Some("/tmp/a.png"));
            assert!(params["commands"][0]["params"].get("path").is_none());

            // No path: nothing to take.
            let (_, path) = take_client_write_path(method, json!({}));
            assert_eq!(path, None);
        }
        // A segments line drawing keeps its path, so the host refuses it as
        // it does headless.
        let (params, path) = take_client_write_path(
            "hew.view.line_drawing",
            json!({"path": "/tmp/a.svg", "format": "segments"}),
        );
        assert_eq!(path, None);
        assert_eq!(params["path"], "/tmp/a.svg");
    }

    #[test]
    fn finishing_a_render_write_puts_every_pass_in_its_file_and_answers_the_headless_shape() {
        let dir = std::env::temp_dir().join(format!("hew-cli-render-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        let path = path.to_str().unwrap();
        let reply = json!({"jsonrpc":"2.0","id":1,"result":{"results":[{
            "png_base64":"Zm9v","width":2,"height":3,
            "id_buffer_base64":"YmFy","id_palette":["a"],
            "depth_png_base64":"YmF6","depth_near":1.0,"depth_far":2.0,"depth_bits":8,
            "normals_png_base64":"cXV4"
        }]}});
        let out = finish_client_write(&reply, path).unwrap();
        assert_eq!(out["path"], path);
        assert_eq!(out["width"], 2);
        assert_eq!(std::fs::read(path).unwrap(), b"foo");
        assert_eq!(std::fs::read(format!("{path}.ids.bin")).unwrap(), b"bar");
        assert_eq!(std::fs::read(format!("{path}.depth.png")).unwrap(), b"baz");
        assert_eq!(
            std::fs::read(format!("{path}.normals.png")).unwrap(),
            b"qux"
        );
        assert_eq!(out["id_buffer_path"], format!("{path}.ids.bin"));
        assert_eq!(out["depth_far"], 2.0);
        assert!(out.get("png_base64").is_none());

        let svg = dir.join("d.svg");
        let svg = svg.to_str().unwrap();
        let reply = json!({"result":{"svg":"<svg/>","count":4,"bounds":[0,0,1,1]}});
        let out = finish_client_write(&reply, svg).unwrap();
        assert_eq!(out, json!({"path": svg, "count": 4, "bounds": [0,0,1,1]}));
        assert_eq!(std::fs::read(svg).unwrap(), b"<svg/>");

        // An export still answers `saved`.
        let stl = dir.join("m.stl");
        let stl = stl.to_str().unwrap();
        let out = finish_client_write(&json!({"result":{"bytes_base64":"Zm9v"}}), stl).unwrap();
        assert_eq!(out, json!({"saved": stl}));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A solitary command wrapped in a one-command transaction is the
    /// same command — §6.4 calls that the canonical way to invoke one,
    /// and it is the ONLY shape an MCP client can use for a command that
    /// is not read-only. Matching just the bare name meant every MCP
    /// save/export with a path refused live while the identical headless
    /// call wrote the file. Found by adversarial review.
    #[test]
    fn a_transact_wrapped_export_has_its_path_stripped_like_a_bare_one() {
        // `hew.doc.save` is deliberately NOT client-written any more: the
        // app saves through its own save path (file, clean mark, binding),
        // so its path is forwarded untouched, bare or wrapped.
        let save = json!({"commands":[{"method":"hew.doc.save","params":{"path":"/tmp/a.hew"}}]});
        let (params, path) = take_client_write_path("hew.doc.transact", save.clone());
        assert_eq!(path, None);
        assert_eq!(params, save);
        let (params, path) = take_client_write_path("hew.doc.save", json!({"path":"/tmp/a.hew"}));
        assert_eq!(path, None);
        assert_eq!(params["path"], "/tmp/a.hew");

        let (params, path) = take_client_write_path(
            "hew.doc.transact",
            json!({"commands":[{"method":"hew.doc.export",
                                "params":{"format":"stl","path":"/tmp/a.stl"}}]}),
        );
        assert_eq!(path.as_deref(), Some("/tmp/a.stl"));
        assert_eq!(params["commands"][0]["params"]["format"], "stl");
    }

    /// A snapshot whose file cannot be written answers the same canonical
    /// `save_failed` refusal an export does.
    #[test]
    fn a_failed_snapshot_write_answers_save_failed() {
        let reply = json!({"jsonrpc":"2.0","id":3,"result":{
            "png_base64":"Zm9v","width":2,"height":3}});
        let message = finish_client_write(&reply, "/no/such/dir/shot.png").unwrap_err();
        let out = write_failure_reply(&reply, "hew.view.snapshot", &message);
        assert_eq!(out["id"], 3);
        assert_eq!(out["error"]["data"]["refusal"], "save_failed");
        assert_eq!(out["error"]["data"]["failed_method"], "hew.view.snapshot");
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hew-cli-cw-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The saved shape: the headless keys, the written file paths in place
    /// of the inline base64, and the passthrough members.
    #[test]
    fn a_client_snapshot_write_answers_the_headless_shape() {
        let dir = scratch("shape");
        let target = dir.join("shot.png").to_str().unwrap().to_string();
        let reply = json!({"jsonrpc":"2.0","id":1,"result":{
            "png_base64":"Zm9v","width":2,"height":3,
            "id_buffer_base64":"aWRz","id_palette":["a"],
            "depth_png_base64":"ZGVwdGg=","depth_near":1.0,"depth_far":2.0,"depth_bits":8,
            "normals_png_base64":"bm9ybQ=="}});
        let out = finish_client_write(&reply, &target).unwrap();
        assert_eq!(out["path"], target.as_str());
        assert_eq!(
            (out["width"].as_u64(), out["height"].as_u64()),
            (Some(2), Some(3))
        );
        assert_eq!(out["id_buffer_path"], format!("{target}.ids.bin").as_str());
        assert_eq!(out["depth_path"], format!("{target}.depth.png").as_str());
        assert_eq!(
            out["normals_path"],
            format!("{target}.normals.png").as_str()
        );
        assert_eq!(out["id_palette"], json!(["a"]));
        assert_eq!(out["depth_near"], 1.0);
        assert_eq!(out["depth_far"], 2.0);
        assert_eq!(out["depth_bits"], 8);
        assert_eq!(
            std::fs::read(format!("{target}.normals.png")).unwrap(),
            b"norm"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rewrite without companions removes the ones an earlier run left.
    #[test]
    fn a_client_snapshot_write_removes_stale_companions() {
        let dir = scratch("stale");
        let target = dir.join("shot.png").to_str().unwrap().to_string();
        for suffix in [".ids.bin", ".depth.png", ".normals.png"] {
            std::fs::write(format!("{target}{suffix}"), b"old").unwrap();
        }
        let reply = json!({"result":{"png_base64":"Zm9v","width":1,"height":1}});
        finish_client_write(&reply, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"foo");
        for suffix in [".ids.bin", ".depth.png", ".normals.png"] {
            assert!(!std::path::Path::new(&format!("{target}{suffix}")).exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A companion that cannot be written takes the PNG down with it.
    #[test]
    fn a_failed_companion_write_leaves_no_partial_result() {
        let dir = scratch("partial");
        let target = dir.join("shot.png").to_str().unwrap().to_string();
        // A directory where the id buffer should go.
        std::fs::create_dir(format!("{target}.ids.bin")).unwrap();
        let reply = json!({"result":{
            "png_base64":"Zm9v","width":1,"height":1,"id_buffer_base64":"aWRz"}});
        let message = finish_client_write(&reply, &target).unwrap_err();
        assert!(message.contains("save_failed"), "{message}");
        assert!(!std::path::Path::new(&target).exists(), "no stray PNG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Malformed base64 answers the canonical -32000 `save_failed` envelope
    /// and writes nothing.
    #[test]
    fn malformed_render_bytes_answer_the_canonical_refusal() {
        let dir = scratch("bad64");
        let target = dir.join("shot.png").to_str().unwrap().to_string();
        let reply = json!({"jsonrpc":"2.0","id":9,"result":{
            "png_base64":"Zm9v","width":1,"height":1,"id_buffer_base64":"!!not base64!!"}});
        let message = finish_client_write(&reply, &target).unwrap_err();
        let out = write_failure_reply(&reply, "hew.view.snapshot", &message);
        assert_eq!(out["error"]["code"], -32000, "{out}");
        assert_eq!(out["error"]["data"]["refusal"], "save_failed");
        assert!(!std::path::Path::new(&target).exists(), "nothing written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_client_write_answers_the_canonical_refusal_envelope() {
        let reply = json!({"jsonrpc":"2.0","id":7,"result":{"bytes_base64":"Zm9v"}});
        let message = json!({
            "refusal": "save_failed",
            "explanation": "/no/such/dir/a.stl: No such file or directory (os error 2)",
            "detail": {"path": "/no/such/dir/a.stl"},
        })
        .to_string();
        let out = write_failure_reply(&reply, "hew.doc.export", &message);
        assert_eq!(out["id"], 7);
        assert_eq!(out["error"]["code"], -32000);
        assert_eq!(out["error"]["data"]["refusal"], "save_failed");
        assert_eq!(out["error"]["data"]["failed_method"], "hew.doc.export");
        assert_eq!(out["error"]["data"]["failed_index"], 0);
        assert_eq!(out["error"]["data"]["detail"]["path"], "/no/such/dir/a.stl");
        // Free text (no bytes in the reply) is an internal fault, not a refusal.
        let plain = write_failure_reply(&reply, "hew.doc.export", "no bytes");
        assert_eq!(plain["error"]["code"], -32003);
    }

    /// Only the one-command form is the same command. A multi-command
    /// transaction, or one wrapping something else, is left alone.
    #[test]
    fn a_live_save_is_never_written_client_side_whatever_its_format() {
        // The app saves through its own Save (it writes the file and binds
        // the document), so hew-cli never takes a save's path to write the
        // bytes itself: not for a .skp, not with a format, not in a
        // one-command transaction.
        for params in [
            json!({ "path": "/m/m.SKP" }),
            json!({ "path": "/m/m.skp", "format": "skp2017", "units": "mm" }),
            json!({ "path": "/m/m.hew" }),
        ] {
            let (out, path) = take_client_write_path("hew.doc.save", params.clone());
            assert_eq!(path, None, "{params}");
            assert_eq!(out, params);
            let envelope = json!({ "commands": [{ "method": "hew.doc.save", "params": params }] });
            let (out, path) = take_client_write_path("hew.doc.transact", envelope.clone());
            assert_eq!(path, None);
            assert_eq!(out, envelope);
        }
    }

    #[test]
    fn take_client_write_path_leaves_other_envelopes_untouched() {
        for envelope in [
            json!({"commands":[{"method":"hew.doc.save","params":{"path":"/tmp/a"}},
                               {"method":"hew.query.scene","params":{}}]}),
            json!({"commands":[{"method":"hew.solid.extrude","params":{"path":"/tmp/a"}}]}),
            json!({"commands":[]}),
            json!({}),
        ] {
            let (_, path) = take_client_write_path("hew.doc.transact", envelope.clone());
            assert!(path.is_none(), "should not strip from {envelope}");
        }
    }

    /// Params that are not an object, or a non-string path, must not
    /// panic or half-strip.
    #[test]
    fn take_client_write_path_tolerates_malformed_params() {
        for params in [
            json!("a string"),
            json!([1, 2]),
            json!(null),
            json!({"path": 7}),
        ] {
            let (out, path) = take_client_write_path("hew.doc.save", params.clone());
            assert!(path.is_none(), "no path to take from {params}");
            assert_eq!(out, params, "and the params pass through unchanged");
        }
    }

    /// The decoder must invert `crates/api`'s encoder for every payload
    /// length class (no padding, one pad byte, two), since a live save
    /// round-trips a whole `.hew` document through it.
    #[test]
    fn decode_base64_inverts_the_encoder_at_every_padding_length() {
        // Reference vectors (RFC 4648 §10) plus a byte-range sweep.
        for (plain, encoded) in [
            (&b""[..], ""),
            (&b"f"[..], "Zg=="),
            (&b"fo"[..], "Zm8="),
            (&b"foo"[..], "Zm9v"),
            (&b"foob"[..], "Zm9vYg=="),
            (&b"fooba"[..], "Zm9vYmE="),
            (&b"foobar"[..], "Zm9vYmFy"),
        ] {
            assert_eq!(decode_base64(encoded).as_deref(), Some(plain), "{encoded}");
        }
        let all: Vec<u8> = (0u8..=255).collect();
        let encoded = {
            // Encode with the same table the api crate uses.
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for c in all.chunks(3) {
                let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
                let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
                for i in 0..4 {
                    if i <= c.len() {
                        out.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
                    } else {
                        out.push('=');
                    }
                }
            }
            out
        };
        assert_eq!(decode_base64(&encoded).as_deref(), Some(&all[..]));
    }

    /// Malformed input is refused, never guessed at — a truncated or
    /// corrupted payload must not silently produce a short document.
    #[test]
    fn decode_base64_refuses_malformed_input() {
        for bad in ["Zg=", "Zm9vY", "Zg===", "Z!==", "=Zm8", "Zm=8"] {
            assert!(decode_base64(bad).is_none(), "should refuse {bad:?}");
        }
    }
}
