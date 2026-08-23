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
use api::{Connection, DispatchOutcome, Host, Profile, Request, RequestId};
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
    let mut conn = Connection::new(Profile::Core, "hew-cli:run");
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
        match conn.dispatch(&mut doc, &mut host, request) {
            DispatchOutcome::Dropped => {
                // A client-originated notification (no id) — dropped
                // unexecuted, per §4.1; nothing to print.
            }
            DispatchOutcome::Reply(response) => {
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
        if let Err(refusal) = host.save_document(&doc, Some(out_str)) {
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

    for (i, raw) in frames.into_iter().enumerate().skip(1) {
        let mut request: Request = serde_json::from_value(raw).map_err(|e| {
            eprintln!("hew-cli run --live: frame {i}: malformed envelope: {e}");
            1
        })?;
        // A save/export inside a script gets the same client-side write a
        // one-shot dispatch does — see `take_client_write_path`.
        let (params, write_path) = take_client_write_path(
            &request.method,
            request
                .params
                .unwrap_or_else(|| serde_json::Value::Object(Default::default())),
        );
        request.params = Some(params);
        match session.dispatch(request) {
            Ok(DispatchOutcome::Dropped) => {}
            Ok(DispatchOutcome::Reply(response)) => {
                // A save/export we are about to write must NOT go through
                // `record_reply`: that prints the reply and keeps it, and
                // the reply is the whole document or mesh in base64. The
                // caller wants the file, so report the file — the same
                // thing a one-shot dispatch reports.
                if let Some(path) = write_path {
                    let value = serde_json::to_value(&response).expect("Response serializes");
                    if response.error.is_some() {
                        println!("{value}");
                        responses.push(value);
                        return Err(1);
                    }
                    if let Err(msg) = write_live_bytes(&value, &path) {
                        eprintln!("hew-cli run --live: frame {i}: {msg}");
                        return Err(1);
                    }
                    let saved = serde_json::json!({ "saved": path });
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
    let mut conn = Connection::new(Profile::Core, "hew-cli:dispatch");
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
    // line drawing, or a print leaves the file byte-for-byte alone).
    let read_only = conn
        .registry()
        .get(method)
        .map(|c| !c.mutates_document)
        .unwrap_or(false);

    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("dispatch".to_string())),
        method: method.to_string(),
        params: Some(params),
    };
    let DispatchOutcome::Reply(response) = conn.dispatch(&mut doc, &mut host, request) else {
        eprintln!(
            "hew-cli dispatch: {method} is a notification-shaped request internally — this is a bug"
        );
        return DispatchResult {
            exit_code: 1,
            response: None,
        };
    };
    let value = serde_json::to_value(&response).expect("Response serializes");
    println!("{value}");
    if response.error.is_some() {
        return DispatchResult {
            exit_code: 1,
            response: Some(value),
        };
    }

    if !read_only && let Err(refusal) = host.save_document(&doc, None) {
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
    if let Err(msg) = attach_live(&mut session) {
        eprintln!("hew-cli dispatch --live: {msg}");
        return DispatchResult {
            exit_code: 1,
            response: None,
        };
    }

    // `hew.doc.save`/`hew.doc.export` with a path: the live host can
    // produce the bytes (serializing the document, or writing STL/3MF/GLB
    // through `crates/mesh-export`) but has no disk to put them on, so
    // this strips `path` before forwarding, asks for the bytes back, and
    // writes them here, where there IS one. The user typed the same
    // command they would headless and gets the same file — the effect
    // just happens on the side that can perform it.
    let (params, client_write_path) = take_client_write_path(method, params);

    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("dispatch".to_string())),
        method: method.to_string(),
        params: Some(params),
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response)) => {
            let value = serde_json::to_value(&response).expect("Response serializes");
            if response.error.is_some() {
                println!("{value}");
                return DispatchResult {
                    exit_code: 1,
                    response: Some(value),
                };
            }
            if let Some(path) = client_write_path {
                match write_live_bytes(&value, &path) {
                    Ok(()) => println!("{}", serde_json::json!({"saved": path})),
                    Err(msg) => {
                        eprintln!("hew-cli dispatch --live: {msg}");
                        return DispatchResult {
                            exit_code: 1,
                            response: Some(value),
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
        Ok(DispatchOutcome::Dropped) => unreachable!("dispatch's request always carries an id"),
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
        "hew.doc.save" | "hew.doc.export" => strip_path(params),
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
            if !matches!(
                only.get("method").and_then(|m| m.as_str()),
                Some("hew.doc.save" | "hew.doc.export")
            ) {
                return (params, None);
            }
            let inner = only.get("params").cloned().unwrap_or_else(empty_object);
            let (inner, path) = strip_path(inner);
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
fn decode_base64(s: &str) -> Option<Vec<u8>> {
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
        decode_base64(b64).ok_or_else(|| "the document bytes were malformed".to_string())?;
    // A write that fails here fails for the same reasons it would on a
    // host that owned the filesystem, so it answers the same refusal
    // that host would (`save_failed`, declared for both commands) rather
    // than free text — a client parsing refusals should not have to
    // special-case which side of the connection did the writing.
    std::fs::write(path, bytes).map_err(|e| {
        serde_json::json!({
            "refusal": "save_failed",
            "explanation": format!("{path}: {e}"),
            "detail": { "path": path },
        })
        .to_string()
    })
}

/// Dispatches `hew.doc.attach` (`{}` params) over an already hello'd live
/// session — the shared step [`dispatch_live`] and
/// [`crate::mcp::McpServer::new_live`] both need, since neither gets to
/// run a scripted `hew.doc.attach` of its own the way a `run --live`
/// script can (that script controls every frame it sends). Returns a
/// clear message on refusal or transport error; `Ok(())` once the
/// connection is attached and every subsequent command can proceed.
pub(crate) fn attach_live(session: &mut live::LiveSession) -> Result<(), String> {
    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("live-attach".to_string())),
        method: "hew.doc.attach".to_string(),
        params: Some(serde_json::json!({})),
    };
    match session.dispatch(request) {
        Ok(DispatchOutcome::Reply(response)) => match response.error {
            None => Ok(()),
            Some(err) => Err(format!(
                "hew.doc.attach was refused: {} (code {})",
                err.message, err.code
            )),
        },
        Ok(DispatchOutcome::Dropped) => {
            unreachable!("attach's request always carries an id")
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
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text(format!("internal-{method}"))),
        method: method.to_string(),
        params: Some(params),
    };
    let DispatchOutcome::Reply(response) = conn.dispatch(doc, host, request) else {
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
    use super::{decode_base64, take_client_write_path};
    use serde_json::json;

    /// A solitary command wrapped in a one-command transaction is the
    /// same command — §6.4 calls that the canonical way to invoke one,
    /// and it is the ONLY shape an MCP client can use for a command that
    /// is not read-only. Matching just the bare name meant every MCP
    /// save/export with a path refused live while the identical headless
    /// call wrote the file. Found by adversarial review.
    #[test]
    fn a_transact_wrapped_save_has_its_path_stripped_like_a_bare_one() {
        let (params, path) = take_client_write_path(
            "hew.doc.transact",
            json!({"commands":[{"method":"hew.doc.save","params":{"path":"/tmp/a.hew"}}]}),
        );
        assert_eq!(path.as_deref(), Some("/tmp/a.hew"));
        assert!(
            params["commands"][0]["params"].get("path").is_none(),
            "the forwarded envelope must not still carry the path: {params}"
        );

        let (params, path) = take_client_write_path(
            "hew.doc.transact",
            json!({"commands":[{"method":"hew.doc.export",
                                "params":{"format":"stl","path":"/tmp/a.stl"}}]}),
        );
        assert_eq!(path.as_deref(), Some("/tmp/a.stl"));
        assert_eq!(params["commands"][0]["params"]["format"], "stl");
    }

    /// Only the one-command form is the same command. A multi-command
    /// transaction, or one wrapping something else, is left alone.
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
