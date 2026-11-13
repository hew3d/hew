//! Multi-document routing for `--live` (docs/agents/HEW_API.md §4.2, §11.2).
//!
//! The socket transport in `live.rs` used to bind every connection to ONE
//! window at accept time, so an automation could only ever reach whichever
//! document happened to be front-most when it connected. This module is
//! the routing layer between that transport and the per-window webviews
//! (each of which owns exactly one document and one `api::Connection` per
//! client — `crates/wasm-api/src/live.rs`):
//!
//! - It keeps a registry of the open documents with host-lifetime-unique
//!   ids (`doc_N`, never reused: window labels are never reused, so an id
//!   outlives its window and a stale one can never name a different
//!   document).
//! - It answers the host-wide commands no single webview can: the
//!   document list (`hew.meta.documents`, and the `documents` of the
//!   `hello` reply), `hew.doc.attach` to a named document, and the window
//!   lifecycle (`hew.doc.new`/`open` into a NEW window, `close`, `focus`).
//! - It routes every other frame to the connection's current window — or,
//!   when the frame carries a top-level `"document"` member, to that
//!   document for just that one request.
//!
//! A client connection therefore holds one wasm-side session per window it
//! has touched. A session is opened lazily the first time the connection
//! addresses a window: the shell replays the client's own `hello` into it
//! (the webview's `api::Connection` needs one), then attaches. Nothing in
//! this path raises, focuses, or otherwise disturbs a window: delivery is
//! `emit_to(label)`, which reaches a background window exactly as it
//! reaches the front one.
//!
//! The pure halves — frame classification, reply shaping, the document
//! list — are free functions with unit tests; the stateful half needs a
//! live `AppHandle`.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::live::{ConnPayload, FramePayload, LiveApi, ReplyEvent, REPLY_TIMEOUT};

// `crates/api/src/envelope.rs`'s `codes`, mirrored (this crate is its own
// workspace and has no `api` dependency, only raw JSON-RPC text).
const REFUSED: i64 = -32000;
const INVALID_PARAMS: i64 = -32602;
const NO_DOCUMENT: i64 = -32002;
const INTERNAL_FAULT: i64 = -32003;
const NOT_READY: i64 = -32004;

/// How long a window that has not yet finished starting its WASM kernel
/// (a freshly created one, typically) is given before a request addressed
/// to it is refused `window_not_ready`.
const WINDOW_READY_TIMEOUT: Duration = Duration::from_secs(45);

/// One readiness probe's patience: the window's bridge may not have
/// registered its listener yet, in which case the emit lands nowhere and
/// only this timeout notices.
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long `hew.doc.open` waits for a file to load and `hew.doc.close`
/// for the window's decision (a save prompt cannot happen here — `discard`
/// or a refusal — but a large document's teardown takes a moment).
const OP_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a window gets to answer `hew.doc.close` — the question is "any
/// unsaved work?", answered from memory, so silence for this long means a hung page.
const CLOSE_ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

// ------------------------------------------------------------- shared state

/// What the shell knows about each document window beyond what Tauri
/// reports. Session-only; nothing here is persisted.
#[derive(Default)]
pub(crate) struct DocState {
    next_id: u32,
    /// window label → `doc_N`.
    ids: HashMap<String, String>,
    /// window label → the client-chosen label from `hew.doc.new`/`open`.
    labels: HashMap<String, String>,
    /// window label → unsaved changes, as last reported by the webview
    /// (`set_document_dirty`).
    dirty: HashMap<String, bool>,
    /// window label → the document's persistent id (`document_id`,
    /// HEW_FILE_FORMAT.md §2), as last reported by the webview
    /// (`set_document_id`) after an open or a save; absent = none yet.
    document_ids: HashMap<String, String>,
}

impl DocState {
    /// The id for `label`, minted on first sight.
    pub(crate) fn id_for(&mut self, label: &str) -> String {
        if let Some(id) = self.ids.get(label) {
            return id.clone();
        }
        self.next_id += 1;
        let id = format!("doc_{}", self.next_id);
        self.ids.insert(label.to_string(), id.clone());
        id
    }

    pub(crate) fn set_dirty(&mut self, label: &str, dirty: bool) {
        self.dirty.insert(label.to_string(), dirty);
    }

    /// Records (or, with `None`, clears) the persistent id of window
    /// `label`'s document.
    pub(crate) fn set_document_id(&mut self, label: &str, id: Option<String>) {
        match id {
            Some(id) => {
                self.document_ids.insert(label.to_string(), id);
            }
            None => {
                self.document_ids.remove(label);
            }
        }
    }
}

/// `hew://document-id`'s payload: the id this shell gave a window's
/// document, which every event that window emits names
/// (docs/agents/HEW_API.md §4.5).
#[derive(Clone, serde::Serialize)]
pub(crate) struct DocumentIdPayload {
    pub(crate) id: String,
}

/// Tells document window `label` its document's id (minting it if this is
/// the first time anything asked). The window calls `frontend_ready` once
/// its listeners are up, which is when this runs.
pub(crate) fn emit_document_id(app: &AppHandle, label: &str) {
    let Some(state) = app.try_state::<LiveApi>() else {
        return;
    };
    let id = state.docs.lock().expect("docs mutex").id_for(label);
    let _ = app.emit_to(label, "hew://document-id", DocumentIdPayload { id });
}

/// One window as the pure document-list builder sees it.
pub(crate) struct WindowFacts {
    pub label: String,
    pub title: String,
    pub path: Option<String>,
}

/// Builds the `hew.meta.documents` entries, in window creation order. Pure:
/// `front` is the user's front-most document window, `attached` this
/// connection's current one.
pub(crate) fn build_documents(
    state: &mut DocState,
    windows: &[WindowFacts],
    front: Option<&str>,
    attached: Option<&str>,
) -> Vec<Value> {
    windows
        .iter()
        .map(|w| {
            let mut entry = json!({
                "id": state.id_for(&w.label),
                "title": w.title,
                "path": w.path,
                "dirty": state.dirty.get(&w.label).copied().unwrap_or(false),
                "document_id": state.document_ids.get(&w.label),
                "front": front == Some(w.label.as_str()),
                "attached": attached == Some(w.label.as_str()),
            });
            if let Some(label) = state.labels.get(&w.label) {
                entry["label"] = Value::String(label.clone());
            }
            entry
        })
        .collect()
}

// -------------------------------------------------------- frame classification

/// The commands the shell answers itself rather than forwarding.
const HOST_METHODS: &[&str] = &[
    "hew.doc.save",
    "hew.doc.revert",
    "hew.meta.documents",
    "hew.doc.attach",
    "hew.doc.new",
    "hew.doc.open",
    "hew.doc.close",
    "hew.doc.focus",
];

/// What to do with one client frame.
#[derive(Debug, PartialEq)]
pub(crate) enum Routed {
    /// A notification for a shell-answered command: nothing to run, no
    /// reply (§4.1).
    Drop,
    /// A shell-answered command. `wrapped` is true when it arrived as the
    /// sole command of a `hew.doc.transact` envelope — the canonical MCP
    /// invocation of a solitary command (§6.4, §13) — and so is answered
    /// in the transaction result shape.
    Host {
        id: Value,
        method: &'static str,
        params: Value,
        wrapped: bool,
    },
    /// Everything else: forward to a webview. `line` has the envelope's
    /// `document` member removed; `document` is that member, the target
    /// for this one request. `is_hello` marks the reply as one whose
    /// `documents` the shell fills in.
    Forward {
        line: String,
        document: Option<String>,
        needs_reply: bool,
        is_hello: bool,
    },
}

/// Classifies `line`. An unparseable frame is forwarded untouched — the
/// webview's dispatcher answers it with its own parse error.
pub(crate) fn classify(line: &str) -> Routed {
    classify_skipping(line, None)
}

/// [`classify`], treating `skip` (a method the shell would normally answer)
/// as an ordinary forwarded command.
fn classify_skipping(line: &str, skip: Option<&str>) -> Routed {
    let host_methods: Vec<&'static str> = HOST_METHODS
        .iter()
        .copied()
        .filter(|m| Some(*m) != skip)
        .collect();
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return Routed::Forward {
            line: line.to_string(),
            document: None,
            needs_reply: true,
            is_hello: false,
        };
    };
    // `id: null` is a notification, same rule as the dispatcher's.
    let id = value.get("id").filter(|id| !id.is_null()).cloned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    // The envelope's `document` addresses this one request (§4.2). For the
    // commands that take a target it is that command's default — an
    // explicit `params.document` wins — so `hew.doc.close` addressed by
    // envelope closes THAT document, never the connection's current one.
    let envelope_document = value
        .get("document")
        .and_then(Value::as_str)
        .map(str::to_string);
    let with_envelope = |method: &str, mut params: Value| -> Value {
        if matches!(
            method,
            "hew.doc.attach"
                | "hew.doc.close"
                | "hew.doc.focus"
                | "hew.doc.save"
                | "hew.doc.revert"
        ) {
            if let (Some(doc), Some(object)) = (&envelope_document, params.as_object_mut()) {
                object
                    .entry("document")
                    .or_insert_with(|| Value::String(doc.clone()));
            }
        }
        params
    };

    if let Some(method) = method.as_deref() {
        if let Some(host_method) = host_methods.iter().find(|m| **m == method) {
            return match id {
                Some(id) => Routed::Host {
                    id,
                    method: host_method,
                    params: with_envelope(host_method, params),
                    wrapped: false,
                },
                None => Routed::Drop,
            };
        }
        if method == "hew.doc.transact" {
            let commands = params.get("commands").and_then(Value::as_array);
            if let Some([only]) = commands.map(Vec::as_slice) {
                let inner = only.get("method").and_then(Value::as_str);
                if let Some(host_method) = host_methods.iter().find(|m| Some(**m) == inner) {
                    return match id {
                        Some(id) => Routed::Host {
                            id,
                            method: host_method,
                            params: with_envelope(
                                host_method,
                                only.get("params").cloned().unwrap_or_else(|| json!({})),
                            ),
                            wrapped: true,
                        },
                        None => Routed::Drop,
                    };
                }
            }
        }
    }

    let document = value
        .as_object_mut()
        .and_then(|o| o.remove("document"))
        .and_then(|d| d.as_str().map(str::to_string));
    let stripped = if document.is_some() {
        value.to_string()
    } else {
        line.to_string()
    };
    Routed::Forward {
        line: stripped,
        document,
        needs_reply: id.is_some(),
        is_hello: method.as_deref() == Some("hew.meta.hello"),
    }
}

fn ok_reply(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// A command result in the shape the client's envelope asked for.
fn command_reply(id: &Value, method: &str, wrapped: bool, result: Value) -> String {
    if wrapped {
        ok_reply(id, json!({ "results": [result], "label": method }))
    } else {
        ok_reply(id, result)
    }
}

fn error_reply(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// §4.4's canonical refusal shape, from a [`api::Refusal`]-shaped pair.
fn refusal_reply(id: &Value, method: &str, name: &str, explanation: &str, detail: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": REFUSED,
            "message": "refused",
            "data": {
                "refusal": name,
                "failed_index": 0,
                "failed_method": method,
                "detail": detail,
                "explanation": explanation,
            }
        }
    })
    .to_string()
}

/// A refusal ready to turn into a reply.
#[derive(Debug)]
struct Refused {
    name: &'static str,
    explanation: String,
    detail: Value,
}

impl Refused {
    fn new(name: &'static str, explanation: impl Into<String>) -> Refused {
        Refused {
            name,
            explanation: explanation.into(),
            detail: json!({}),
        }
    }
}

/// Resolves a client's document reference against `documents`: exact id,
/// else exact path, label, or title (the last three only when unambiguous).
/// Identical to `api::resolve_document` — kept here, rather than as a
/// dependency on that crate, because this shell has none on it; the pair
/// are pinned together by the shared fixtures in both test modules.
fn resolve<'a>(documents: &'a [Value], query: &str) -> Result<&'a Value, Refused> {
    let field = |d: &Value, key: &str| d.get(key).and_then(Value::as_str).map(str::to_string);
    if let Some(d) = documents
        .iter()
        .find(|d| field(d, "id").as_deref() == Some(query))
    {
        return Ok(d);
    }
    for key in ["path", "label", "title"] {
        let hits: Vec<&Value> = documents
            .iter()
            .filter(|d| field(d, key).as_deref() == Some(query))
            .collect();
        match hits.as_slice() {
            [] => {}
            [one] => return Ok(one),
            many => {
                let ids: Vec<String> = many.iter().filter_map(|d| field(d, "id")).collect();
                return Err(Refused {
                    name: "ambiguous_document",
                    explanation: format!(
                        "\"{query}\" matches {} documents by {key} ({}); pass one of their ids instead.",
                        many.len(),
                        ids.join(", ")
                    ),
                    detail: json!({ "matches": ids, "documents": documents }),
                });
            }
        }
    }
    Err(Refused {
        name: "unknown_document",
        explanation: format!(
            "No open document is identified by \"{query}\" (match is by id, path, label, or title); hew.meta.documents lists them."
        ),
        detail: json!({ "documents": documents }),
    })
}

/// Replaces a `hello` reply's `documents` with the host-wide list.
fn patch_hello_documents(reply: &str, documents: &[Value]) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(reply) else {
        return reply.to_string();
    };
    match value.pointer_mut("/result/documents") {
        Some(slot) => {
            *slot = Value::Array(documents.to_vec());
            value.to_string()
        }
        None => reply.to_string(),
    }
}

// ------------------------------------------------------------ per-connection

/// One client connection's routing state. The connection's CURRENT window
/// lives in `LiveApi::conn_window` (shared, so closing a window can detach
/// every connection bound to it); what is here is private to the
/// connection's own thread.
pub(crate) struct ConnRoute {
    pub conn_id: u32,
    /// The client's sanitized `hello`, replayed into each further window
    /// this connection touches.
    hello: Option<String>,
    /// Windows holding a wasm-side session for this connection → whether
    /// that session has been attached. The accept-time window starts
    /// opened-but-unattached (the client's own `hello` already went
    /// there); the client attaches it, as it always has.
    opened: HashMap<String, bool>,
    probe_seq: u32,
}

impl ConnRoute {
    /// A connection accepted while `window_label` was the front window: its
    /// session there is opened by the caller (the transport emits
    /// `hew://api-connection-open` itself, before the first frame).
    pub(crate) fn new(conn_id: u32, window_label: &str) -> ConnRoute {
        ConnRoute {
            conn_id,
            hello: None,
            opened: HashMap::from([(window_label.to_string(), false)]),
            probe_seq: 0,
        }
    }

    /// Ends every session this connection holds — the transport calls this
    /// once, when the peer goes away.
    pub(crate) fn close_sessions(&mut self, app: &AppHandle) {
        for (label, _) in self.opened.drain() {
            if app.get_webview_window(&label).is_some() {
                let _ = app.emit_to(
                    &label,
                    "hew://api-connection-close",
                    ConnPayload {
                        conn_id: self.conn_id,
                    },
                );
            }
        }
    }
}

/// What the transport does after a frame.
pub(crate) enum Outcome {
    /// A notification — nothing to write.
    NoReply,
    Reply(String),
    /// Write the reply, then close the connection (a timed-out dispatch
    /// is still running; see `synthesize_timeout_reply`).
    ReplyThenClose(String),
    /// A hard transport failure (the window vanished mid-flight): close.
    Close,
}

#[derive(Debug)]
enum SendError {
    Timeout,
    WindowGone,
}

/// Sends one frame to `label`'s webview for connection `conn_id` and waits
/// for the matching `hew://api-reply`. Replies are correlated per
/// connection (one request in flight at a time), not per request id.
fn send_and_await(
    app: &AppHandle,
    label: &str,
    conn_id: u32,
    line: &str,
    timeout: Duration,
) -> Result<String, SendError> {
    let (tx, rx) = mpsc::channel::<ReplyEvent>();
    let state = app.state::<LiveApi>();
    state
        .pending_replies
        .lock()
        .expect("pending_replies mutex")
        .insert(conn_id, tx);
    if app
        .emit_to(
            label,
            "hew://api-frame",
            FramePayload {
                conn_id,
                frame: line,
            },
        )
        .is_err()
    {
        state
            .pending_replies
            .lock()
            .expect("pending_replies mutex")
            .remove(&conn_id);
        return Err(SendError::WindowGone);
    }
    // Wait in slices so a window destroyed mid-request is noticed within a
    // moment rather than after the full timeout: nothing else wakes us. A
    // frame the window queued behind a plugin command restarts the clock
    // when it is finally dispatched (`ReplyEvent::Started`).
    let mut deadline = Instant::now() + timeout;
    let outcome = loop {
        let slice = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        match rx.recv_timeout(slice) {
            Ok(ReplyEvent::Started) => deadline = Instant::now() + timeout,
            Ok(ReplyEvent::Reply(reply)) => break Ok(reply),
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(SendError::WindowGone),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if app.get_webview_window(label).is_none() {
                    break Err(SendError::WindowGone);
                }
                if Instant::now() >= deadline {
                    break Err(SendError::Timeout);
                }
            }
        }
    };
    if outcome.is_err() {
        state
            .pending_replies
            .lock()
            .expect("pending_replies mutex")
            .remove(&conn_id);
    }
    outcome
}

/// Sends a frame that needs no reply.
fn send_notification(app: &AppHandle, label: &str, conn_id: u32, line: &str) -> bool {
    app.emit_to(
        label,
        "hew://api-frame",
        FramePayload {
            conn_id,
            frame: line,
        },
    )
    .is_ok()
}

fn error_code(reply: &str) -> Option<i64> {
    serde_json::from_str::<Value>(reply)
        .ok()?
        .pointer("/error/code")?
        .as_i64()
}

fn reply_id(reply: &str) -> Option<Value> {
    serde_json::from_str::<Value>(reply)
        .ok()?
        .get("id")
        .cloned()
}

/// Makes sure `conn` holds a ready, attached session in window `label`.
///
/// A window that has not finished booting is probed until it answers: the
/// probe is the client's own `hello` (with a private id) — success proves
/// the webview's listener is registered AND its WASM kernel is up. An
/// explicitly addressed window is an attached one: the session is
/// attached before the first real frame, so a request that names its
/// document never answers `-32002`.
fn ensure_session(app: &AppHandle, conn: &mut ConnRoute, label: &str) -> Result<(), Refused> {
    let was_open = conn.opened.contains_key(label);
    let result = ensure_session_inner(app, conn, label);
    if result.is_err() && !was_open {
        // A failed attempt may have announced the connection to the window;
        // end that session so the webview does not keep an orphan.
        let _ = app.emit_to(
            label,
            "hew://api-connection-close",
            ConnPayload {
                conn_id: conn.conn_id,
            },
        );
        crate::live::clear_window_subscriptions(app, conn.conn_id, label);
    }
    result
}

fn ensure_session_inner(app: &AppHandle, conn: &mut ConnRoute, label: &str) -> Result<(), Refused> {
    match conn.opened.get(label) {
        Some(true) => return Ok(()),
        Some(false) => return attach_session(app, conn, label),
        None => {}
    }
    let Some(hello) = conn.hello.clone() else {
        return Err(Refused::new(
            "window_not_ready",
            "hew.meta.hello has not completed on this connection yet.",
        ));
    };
    let deadline = Instant::now() + WINDOW_READY_TIMEOUT;
    loop {
        // Announced on every attempt, not once: a window still booting has
        // no listener yet, so the first announcement can be lost (the
        // bridge treats a repeat as a no-op).
        let _ = app.emit_to(
            label,
            "hew://api-connection-open",
            ConnPayload {
                conn_id: conn.conn_id,
            },
        );
        if app.get_webview_window(label).is_none() {
            return Err(Refused::new(
                "unknown_document",
                "That document's window has closed.",
            ));
        }
        conn.probe_seq += 1;
        let probe_id = format!("hew-probe-{}", conn.probe_seq);
        let mut frame: Value = serde_json::from_str(&hello).unwrap_or_else(|_| json!({}));
        frame["id"] = Value::String(probe_id.clone());
        match send_and_await(app, label, conn.conn_id, &frame.to_string(), PROBE_TIMEOUT) {
            Ok(reply)
                if reply_id(&reply).and_then(|v| v.as_str().map(str::to_string))
                    == Some(probe_id) =>
            {
                match error_code(&reply) {
                    None => break,
                    Some(NOT_READY) => {}
                    Some(_) => {
                        return Err(Refused::new(
                            "window_not_ready",
                            format!("The window refused this connection's hello: {reply}"),
                        ))
                    }
                }
            }
            // A stale reply to an earlier probe, or none at all yet.
            Ok(_) | Err(SendError::Timeout) => {}
            Err(SendError::WindowGone) => {
                return Err(Refused::new(
                    "unknown_document",
                    "That document's window has closed.",
                ))
            }
        }
        if Instant::now() >= deadline {
            return Err(Refused::new(
                "window_not_ready",
                "The window did not finish starting within 45 seconds.",
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    attach_session(app, conn, label)
}

/// Attaches `conn`'s (already hello'd) session in `label` — the wasm-side
/// `hew.doc.attach` — and records it.
fn attach_session(app: &AppHandle, conn: &mut ConnRoute, label: &str) -> Result<(), Refused> {
    conn.probe_seq += 1;
    let attach = json!({
        "jsonrpc": "2.0",
        "id": format!("hew-probe-{}", conn.probe_seq),
        "method": "hew.doc.attach",
        "params": {}
    })
    .to_string();
    match send_and_await(app, label, conn.conn_id, &attach, REPLY_TIMEOUT) {
        Ok(reply) if error_code(&reply).is_none() => {
            conn.opened.insert(label.to_string(), true);
            Ok(())
        }
        Ok(reply) => Err(Refused::new(
            "window_not_ready",
            format!("The window refused the attach: {reply}"),
        )),
        Err(e) => {
            if matches!(e, SendError::Timeout) {
                // The attach may still be waiting in the window (queued
                // behind a plugin command): end this connection's session
                // there, so the webview drops it and its late reply can
                // never answer a later request. The next use opens a fresh
                // session.
                let _ = app.emit_to(
                    label,
                    "hew://api-connection-close",
                    ConnPayload {
                        conn_id: conn.conn_id,
                    },
                );
                conn.opened.remove(label);
                crate::live::clear_window_subscriptions(app, conn.conn_id, label);
            }
            Err(Refused::new(
                "window_not_ready",
                "The window did not accept the attach.",
            ))
        }
    }
}

// ------------------------------------------------------------ app-side facts

fn windows_facts(app: &AppHandle) -> Vec<WindowFacts> {
    let paths = crate::document_paths_snapshot(app);
    crate::window_list(app)
        .into_iter()
        .map(|w| WindowFacts {
            path: paths.get(&w.label).cloned(),
            label: w.label,
            title: strip_dirty_mark(&w.title),
        })
        .collect()
}

/// A window's title carries the "• " unsaved mark (`deriveTitle`,
/// app/src/io/documentSession.ts); the API reports that as `dirty`, so the
/// `title` is the bare document name.
fn strip_dirty_mark(title: &str) -> String {
    title.strip_prefix("• ").unwrap_or(title).to_string()
}

fn front_label(app: &AppHandle) -> Option<String> {
    crate::active_document_window(app).map(|w| w.label().to_string())
}

fn current_window(app: &AppHandle, conn_id: u32) -> Option<String> {
    let label = app
        .state::<LiveApi>()
        .conn_window
        .lock()
        .expect("conn_window mutex")
        .get(&conn_id)
        .cloned()?;
    if app.get_webview_window(&label).is_some() {
        Some(label)
    } else {
        // The window closed under the connection: detach it (§4.2 —
        // subsequent requests answer -32002).
        app.state::<LiveApi>()
            .conn_window
            .lock()
            .expect("conn_window mutex")
            .remove(&conn_id);
        None
    }
}

fn set_current_window(app: &AppHandle, conn_id: u32, label: &str) {
    app.state::<LiveApi>()
        .conn_window
        .lock()
        .expect("conn_window mutex")
        .insert(conn_id, label.to_string());
}

/// The document list from `conn`'s point of view. `attached` is the
/// connection's current document only once its session there is actually
/// attached (the accept-time window is bound but not yet attached until the
/// client says so — a request to it would still answer -32002).
fn documents_for(app: &AppHandle, conn: &ConnRoute) -> Vec<Value> {
    let attached =
        current_window(app, conn.conn_id).filter(|label| conn.opened.get(label) == Some(&true));
    let windows = windows_facts(app);
    let front = front_label(app);
    let state = app.state::<LiveApi>();
    let mut docs = state.docs.lock().expect("docs mutex");
    build_documents(&mut docs, &windows, front.as_deref(), attached.as_deref())
}

fn label_of_document(app: &AppHandle, document: &Value) -> Option<String> {
    let id = document.get("id")?.as_str()?;
    let state = app.state::<LiveApi>();
    let docs = state.docs.lock().expect("docs mutex");
    docs.ids
        .iter()
        .find(|(_, v)| v.as_str() == id)
        .map(|(label, _)| label.clone())
}

/// Resolves the `document` param (or, absent, the connection's current
/// window) to a window label and its list entry.
fn target_of(
    app: &AppHandle,
    conn: &ConnRoute,
    named: Option<&str>,
) -> Result<(String, Value), Refused> {
    let documents = documents_for(app, conn);
    match named {
        Some(query) => {
            let found = resolve(&documents, query)?.clone();
            let label = label_of_document(app, &found)
                .ok_or_else(|| Refused::new("unknown_document", "That document is gone."))?;
            Ok((label, found))
        }
        None => {
            let label = current_window(app, conn.conn_id).ok_or_else(|| {
                Refused::new(
                    "unknown_document",
                    "This connection is not attached to a document; pass `document`.",
                )
            })?;
            let entry = documents
                .into_iter()
                .find(|d| label_of_document(app, d).as_deref() == Some(label.as_str()))
                .ok_or_else(|| Refused::new("unknown_document", "That document is gone."))?;
            Ok((label, entry))
        }
    }
}

fn entry_for_label(app: &AppHandle, conn: &ConnRoute, label: &str) -> Value {
    documents_for(app, conn)
        .into_iter()
        .find(|d| label_of_document(app, d).as_deref() == Some(label))
        .unwrap_or_else(|| json!({}))
}

// ------------------------------------------------------------ webview ops

/// Why a webview-side operation did not complete.
struct OpError {
    refusal: String,
    message: String,
}

/// Asks `label`'s webview to run one operation that only it can do with
/// the document's real state (load a file into the live session, decide
/// whether closing would lose work) and waits for its answer.
fn run_op(app: &AppHandle, label: &str, op: &str, args: Value) -> Result<(), OpError> {
    run_op_within(app, label, op, args, OP_TIMEOUT)
}

/// [`run_op`] with its own patience.
fn run_op_within(
    app: &AppHandle,
    label: &str,
    op: &str,
    args: Value,
    timeout: Duration,
) -> Result<(), OpError> {
    let state = app.state::<LiveApi>();
    let op_id = state.next_op_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<Value>();
    state
        .pending_ops
        .lock()
        .expect("pending_ops mutex")
        .insert(op_id, tx);
    if app
        .emit_to(
            label,
            "hew://api-op",
            json!({ "opId": op_id, "op": op, "args": args }),
        )
        .is_err()
    {
        state
            .pending_ops
            .lock()
            .expect("pending_ops mutex")
            .remove(&op_id);
        return Err(OpError {
            refusal: "unknown_document".into(),
            message: "That document's window has closed.".into(),
        });
    }
    let outcome = rx.recv_timeout(timeout);
    state
        .pending_ops
        .lock()
        .expect("pending_ops mutex")
        .remove(&op_id);
    match outcome {
        Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(true) => Ok(()),
        Ok(v) => Err(OpError {
            refusal: v
                .get("refusal")
                .and_then(Value::as_str)
                .unwrap_or("window_not_ready")
                .to_string(),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("The window could not do that.")
                .to_string(),
        }),
        Err(_) => Err(OpError {
            refusal: "window_not_ready".into(),
            message: "The window did not answer in time.".into(),
        }),
    }
}

// --------------------------------------------------------------- host commands

fn wants_front(params: &Value) -> Result<bool, Refused> {
    match params.get("focus").and_then(Value::as_str) {
        None | Some("background") => Ok(false),
        Some("front") => Ok(true),
        Some(other) => Err(Refused::new(
            "invalid_focus",
            format!("`focus` is \"background\" or \"front\", not \"{other}\"."),
        )),
    }
}

fn record_label(app: &AppHandle, window_label: &str, params: &Value) {
    if let Some(label) = params.get("label").and_then(Value::as_str) {
        app.state::<LiveApi>()
            .docs
            .lock()
            .expect("docs mutex")
            .labels
            .insert(window_label.to_string(), label.to_string());
    }
}

/// Creates a blank document window for an API client and returns its
/// label. Unless `front`, the new window is created unfocused and slotted
/// behind the user's front window (macOS orders it there exactly; elsewhere
/// the front window is re-raised, which needs the app to have been active).
fn create_window(app: &AppHandle, front: bool) -> Result<String, Refused> {
    let anchor = crate::active_document_window(app).ok_or_else(|| {
        Refused::new(
            "host_capability_missing",
            "No document window is open to create the new one beside.",
        )
    })?;
    #[cfg(not(target_os = "macos"))]
    let was_focused = anchor.is_focused().unwrap_or(false);
    let anchor_label = anchor.label().to_string();
    let label =
        tauri::async_runtime::block_on(crate::create_document_window(app, &anchor, None, front))
            .map_err(|e| {
                Refused::new(
                    "window_not_ready",
                    format!("Could not create a window: {e}"),
                )
            })?;
    if !front {
        #[cfg(target_os = "macos")]
        crate::order_window_behind(app, &label, &anchor_label);
        #[cfg(not(target_os = "macos"))]
        if was_focused {
            let _ = crate::refocus_window(app, &anchor_label);
        }
    }
    Ok(label)
}

fn destroy_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.destroy();
    }
}

fn cmd_new_inner(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    let front = wants_front(params)?;
    let label = create_window(app, front)?;
    record_label(app, &label, params);
    if let Err(e) = ensure_session(app, conn, &label) {
        destroy_window(app, &label);
        return Err(e);
    }
    set_current_window(app, conn.conn_id, &label);
    Ok(json!({ "document": entry_for_label(app, conn, &label) }))
}

fn cmd_open_inner(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    let front = wants_front(params)?;
    let Some(path) = params.get("path").and_then(Value::as_str) else {
        return Err(Refused::new("invalid_path", "`path` is required."));
    };
    if !std::path::Path::new(path).is_absolute() {
        return Err(Refused::new(
            "invalid_path",
            "`path` must be absolute: the app's working directory is not the caller's (hew-cli makes it absolute for you).",
        ));
    }
    // A `.hew` document, or a `.skp` opened as the document's own file
    // (the app's open-skp path); other formats merge through hew.doc.import.
    let is_document = |p: &str| {
        let lower = p.to_ascii_lowercase();
        lower.ends_with(".hew") || lower.ends_with(".skp")
    };
    if !is_document(path) {
        return Err(Refused::new(
            "load_failed",
            "hew.doc.open opens .hew and .skp documents; use hew.doc.import to merge other formats.",
        ));
    }
    // The same file is already open in a window: attach to it rather than
    // opening a second writer on one file.
    if let Some(existing) = crate::window_label_for_path(app, path) {
        ensure_session(app, conn, &existing)?;
        set_current_window(app, conn.conn_id, &existing);
        record_label(app, &existing, params);
        if front {
            let _ = crate::focus_window_by_label(app, &existing);
        }
        return Ok(json!({
            "document": entry_for_label(app, conn, &existing),
            "already_open": true,
        }));
    }
    // Validate before granting anything: the file must exist, and what it
    // resolves to (a symlink named *.hew or *.skp included) must itself be
    // a document file. Only then does the approval — which stands in for
    // the native dialog pick — go in, and for write too, since the
    // document is opened bound to this path and its Save writes back to it.
    let canonical = match std::fs::canonicalize(path) {
        Ok(c) if c.is_file() && is_document(&c.to_string_lossy()) => c,
        Ok(_) => {
            return Err(Refused::new(
                "load_failed",
                format!("{path}: not a .hew or .skp file"),
            ))
        }
        Err(e) => return Err(Refused::new("load_failed", format!("{path}: {e}"))),
    };
    crate::approve_file(app, &canonical, true);
    let label = create_window(app, front)?;
    // Claim the path for the new window NOW, before its (slow) load, so a
    // second open of the same file attaches to this window instead of
    // racing it into a second writer.
    crate::claim_document_path(app, &label, path);
    record_label(app, &label, params);
    if let Err(e) = ensure_session(app, conn, &label) {
        destroy_window(app, &label);
        return Err(e);
    }
    // The window answered a probe, so its bridge is listening: tell it its
    // document's id before the load, whose `document_opened` names it (the
    // window's own `frontend_ready` report can land after the load).
    emit_document_id(app, &label);
    if let Err(e) = run_op(app, &label, "open", json!({ "path": path })) {
        destroy_window(app, &label);
        return Err(Refused::new(
            if e.refusal == "load_failed" {
                "load_failed"
            } else {
                "window_not_ready"
            },
            format!("{path}: {}", e.message),
        ));
    }
    wait_for_title(app, &label, path);
    set_current_window(app, conn.conn_id, &label);
    Ok(json!({ "document": entry_for_label(app, conn, &label) }))
}

/// A freshly loaded document's title reaches the shell a beat after the
/// load itself (the webview pushes it from an effect); give it a moment
/// so the entry `hew.doc.open` answers with names the document rather than
/// "Untitled". Best-effort: a slow push just means the stale title.
fn wait_for_title(app: &AppHandle, label: &str, path: &str) {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let deadline = Instant::now() + Duration::from_millis(2000);
    while Instant::now() < deadline {
        let titled = crate::window_list(app)
            .into_iter()
            .any(|w| w.label == label && strip_dirty_mark(&w.title).contains(&name));
        if titled {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The window-lifecycle commands run one at a time across connections: the
/// last-document check, the already-open check, and window creation each
/// read state the others change.
fn cmd_new(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    let state = app.state::<LiveApi>();
    let _lifecycle = state.window_ops.lock().unwrap_or_else(|e| e.into_inner());
    cmd_new_inner(app, conn, params)
}

fn cmd_open(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    let state = app.state::<LiveApi>();
    let _lifecycle = state.window_ops.lock().unwrap_or_else(|e| e.into_inner());
    cmd_open_inner(app, conn, params)
}

fn cmd_close(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    let state = app.state::<LiveApi>();
    let _lifecycle = state.window_ops.lock().unwrap_or_else(|e| e.into_inner());
    cmd_close_inner(app, conn, params)
}

fn cmd_attach(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    if let Some(extra) = params
        .as_object()
        .and_then(|o| o.keys().find(|k| k.as_str() != "document"))
    {
        return Err(Refused::new(
            "invalid_params",
            format!("unknown field `{extra}`"),
        ));
    }
    let named = params.get("document").and_then(Value::as_str);
    let (label, _) = match named {
        Some(_) => target_of(app, conn, named)?,
        None => match current_window(app, conn.conn_id) {
            // No argument: stay on (or fall back to) the current window.
            Some(_) => target_of(app, conn, None)?,
            None => {
                let front = front_label(app).ok_or_else(|| {
                    Refused::new("unknown_document", "No document window is open.")
                })?;
                let documents = documents_for(app, conn);
                let entry = documents
                    .into_iter()
                    .find(|d| label_of_document(app, d).as_deref() == Some(front.as_str()))
                    .unwrap_or_else(|| json!({}));
                (front, entry)
            }
        },
    };
    ensure_session(app, conn, &label)?;
    set_current_window(app, conn.conn_id, &label);
    Ok(json!({ "document": entry_for_label(app, conn, &label) }))
}

fn cmd_close_inner(
    app: &AppHandle,
    conn: &mut ConnRoute,
    params: &Value,
) -> Result<Value, Refused> {
    let named = params.get("document").and_then(Value::as_str);
    let discard = params
        .get("discard")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (label, entry) = target_of(app, conn, named)?;
    if windows_facts(app).len() <= 1 {
        return Err(Refused::new(
            "last_document",
            "That is the only open document; closing it would leave nothing for a live connection to attach to. Close it from the app, or open another document first.",
        ));
    }
    // No session is needed to close a window (the op channel is its own),
    // so a wedged webview cannot hold this up the way it would a request.
    // A window that does not answer within a moment is wedged: with
    // `discard` the shell destroys it itself — the one way to close a
    // document whose page has hung — and says nothing was saved.
    if let Err(e) = run_op_within(
        app,
        &label,
        "close",
        json!({ "discard": discard }),
        CLOSE_ANSWER_TIMEOUT,
    ) {
        if discard && e.refusal == "window_not_ready" {
            crate::clear_recovery_for_label(app, &label);
            destroy_window(app, &label);
        } else {
            let name = match e.refusal.as_str() {
                "unsaved_changes" => "unsaved_changes",
                "unknown_document" => "unknown_document",
                _ => "window_not_ready",
            };
            return Err(Refused {
                name,
                explanation: e.message,
                detail: json!({ "document": entry }),
            });
        }
    }
    // The webview destroys its own window after answering; wait for it so
    // the list the next call sees is already without it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.get_webview_window(&label).is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(40));
    }
    app.state::<LiveApi>()
        .conn_window
        .lock()
        .expect("conn_window mutex")
        .retain(|_, l| *l != label);
    conn.opened.remove(&label);
    Ok(json!({ "closed": entry }))
}

/// Whether the app should perform this `hew.doc.save` itself: always when a
/// `path` was named, and for a bare save when the target document already
/// has a file to save back to.
fn save_is_app_side(app: &AppHandle, conn: &ConnRoute, params: &Value) -> bool {
    // Handing the bytes back writes nothing: the dispatcher does it.
    if params.get("return_bytes").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if params.get("path").and_then(Value::as_str).is_some() {
        return true;
    }
    let named = params.get("document").and_then(Value::as_str);
    match target_of(app, conn, named) {
        Ok((label, _)) => crate::document_paths_snapshot(app).contains_key(&label),
        Err(_) => false,
    }
}

/// Whether the app should perform this `hew.doc.revert`: the target document
/// has a file to re-read.
fn revert_is_app_side(app: &AppHandle, conn: &ConnRoute, params: &Value) -> bool {
    let named = params.get("document").and_then(Value::as_str);
    match target_of(app, conn, named) {
        Ok((label, _)) => crate::document_paths_snapshot(app).contains_key(&label),
        Err(_) => false,
    }
}

/// The display length formats `hew.doc.save`'s `units` names: keep in step
/// with `crates/api`'s `LENGTH_FORMATS`, which this crate does not depend
/// on (`app/src/settings/units.ts`'s `LengthFormat` is the third copy).
const LENGTH_FORMATS: &[&str] = &["m", "cm", "mm", "arch", "frac_in", "dec_in"];

/// The stream releases a SketchUp file is saved in, newest first (the
/// major number of each release's version string): keep in step with
/// `crates/api`'s `SKP_STREAM_RELEASES`, which this crate does not depend
/// on. Adding a release is one number here and there.
const SKP_STREAM_RELEASES: &[u8] = &[20, 19, 18, 17, 16, 15, 14, 13, 8, 7, 6, 5, 4, 3];

/// The wire name of a stream release: `skp8`, `skp2017`.
fn stream_format_name(release: u8) -> String {
    let year = if release >= 13 {
        2000 + u32::from(release)
    } else {
        u32::from(release)
    };
    format!("skp{year}")
}

/// Every name `hew.doc.save`'s `format` takes.
fn doc_format_names() -> Vec<String> {
    ["hew".to_string(), "skp".to_string()]
        .into_iter()
        .chain(SKP_STREAM_RELEASES.iter().map(|&r| stream_format_name(r)))
        .collect()
}

fn is_doc_format(name: &str) -> bool {
    doc_format_names().iter().any(|n| n == name)
}

/// `hew.doc.save` in the desktop app: the window saves through its own save
/// path — the file is written, the unsaved mark cleared, the document bound
/// to the file (title, recents, "open the same file" focus) exactly as if
/// the user had pressed Save — instead of handing bytes to a client that
/// would leave the app still calling the document dirty.
fn cmd_save(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    if let Some(extra) = params.as_object().and_then(|o| {
        o.keys().find(|k| {
            !matches!(
                k.as_str(),
                "path" | "document" | "copy" | "format" | "units" | "compat"
            )
        })
    }) {
        return Err(Refused::new(
            "invalid_params",
            format!("unknown field `{extra}`"),
        ));
    }
    let named = params.get("document").and_then(Value::as_str);
    let (label, entry) = target_of(app, conn, named)?;
    let path = params.get("path").and_then(Value::as_str);
    let copy = params.get("copy").and_then(Value::as_bool).unwrap_or(false);
    if copy && path.is_none() {
        return Err(Refused::new("invalid_params", "`copy` needs a `path`"));
    }
    // The same `format` and `units` the headless host takes; the window
    // resolves what they leave open (the document's own container).
    let format = params.get("format").and_then(Value::as_str);
    if params.get("format").is_some() && !format.is_some_and(is_doc_format) {
        return Err(Refused::new(
            "invalid_params",
            format!("`format` is one of {:?}", doc_format_names()),
        ));
    }
    let units = params.get("units").and_then(Value::as_str);
    if params.get("units").is_some() && !units.is_some_and(|u| LENGTH_FORMATS.contains(&u)) {
        return Err(Refused::new(
            "invalid_params",
            format!("`units` is one of {LENGTH_FORMATS:?}"),
        ));
    }
    let compat = params.get("compat").and_then(Value::as_bool);
    if params.get("compat").is_some() && compat.is_none() {
        return Err(Refused::new("invalid_params", "`compat` is a boolean"));
    }
    if compat == Some(true) && !format.is_none_or(|f| f == "skp") {
        return Err(Refused::new(
            "invalid_params",
            "`compat` applies to the 2021-and-later container (format \"skp\")",
        ));
    }
    if let Some(path) = path {
        if !std::path::Path::new(path).is_absolute() {
            return Err(Refused::new(
                "invalid_path",
                "`path` must be absolute: the app's working directory is not the caller's (hew-cli makes it absolute for you).",
            ));
        }
        // A document is saved to a .hew or .skp file: without this an
        // authenticated client could point Save at any file the user owns
        // and have the app overwrite it with document bytes.
        let lower = path.to_ascii_lowercase();
        let skp = lower.ends_with(".skp");
        if !skp && !lower.ends_with(".hew") {
            return Err(Refused::new(
                "invalid_path",
                "`path` must end in .hew or .skp — hew.doc.save writes a Hew or SketchUp document.",
            ));
        }
        // One format's bytes never go under the other's extension.
        if format.is_some_and(|f| (f != "hew") != skp) {
            return Err(Refused::new(
                "format_mismatch",
                format!(
                    "`format` {} contradicts the extension of {path}",
                    format.unwrap_or("")
                ),
            ));
        }
        if !std::path::Path::new(path)
            .parent()
            .is_some_and(|dir| dir.is_dir())
        {
            return Err(Refused {
                name: "save_failed",
                explanation: format!("{path}: the folder does not exist"),
                detail: json!({ "path": path }),
            });
        }
        // Two windows must never be two writers of one file (the same rule
        // `hew.doc.open` keeps): refuse a path another window holds.
        if let Some(holder) = crate::window_label_for_path(app, path) {
            if holder != label {
                return Err(Refused {
                    name: "file_open_elsewhere",
                    explanation: format!(
                        "{path} is open in another window; saving there would leave two windows writing one file."
                    ),
                    detail: json!({ "document": entry }),
                });
            }
        }
        // The authenticated client holds the user's filesystem rights; this
        // stands in for the Save As dialog's approval of the destination.
        crate::approve_file(app, std::path::Path::new(path), true);
    }
    let mut op = json!({ "path": path, "copy": copy });
    if let Some(f) = format {
        op["format"] = json!(f);
    }
    if let Some(u) = units {
        op["units"] = json!(u);
    }
    if let Some(c) = compat {
        op["compat"] = json!(c);
    }
    if let Err(e) = run_op(app, &label, "save", op) {
        return Err(Refused {
            name: match e.refusal.as_str() {
                "unknown_document" => "unknown_document",
                "format_mismatch" => "format_mismatch",
                _ => "save_failed",
            },
            explanation: e.message,
            detail: json!({ "document": entry }),
        });
    }
    Ok(json!({}))
}

/// `hew.doc.revert` in the desktop app: the window re-reads its own file
/// through the open path the UI uses and clears its history, keeping its
/// camera and panels. Only reached for a document that has a file; one
/// without falls through to the dispatcher, which refuses
/// `nothing_to_revert`. The window refuses the same way for a document
/// with no changes, as only it knows.
fn cmd_revert(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    if let Some(extra) = params
        .as_object()
        .and_then(|o| o.keys().find(|k| k.as_str() != "document"))
    {
        return Err(Refused::new(
            "invalid_params",
            format!("unknown field `{extra}`"),
        ));
    }
    let named = params.get("document").and_then(Value::as_str);
    let (label, entry) = target_of(app, conn, named)?;
    if let Err(e) = run_op(app, &label, "revert", json!({})) {
        return Err(Refused {
            name: match e.refusal.as_str() {
                "unknown_document" => "unknown_document",
                "nothing_to_revert" => "nothing_to_revert",
                _ => "load_failed",
            },
            explanation: e.message,
            detail: json!({ "document": entry }),
        });
    }
    Ok(json!({}))
}

fn cmd_focus(app: &AppHandle, conn: &mut ConnRoute, params: &Value) -> Result<Value, Refused> {
    let named = params.get("document").and_then(Value::as_str);
    let (label, _) = target_of(app, conn, named)?;
    crate::focus_window_by_label(app, &label).map_err(|e| Refused::new("window_not_ready", e))?;
    Ok(json!({ "document": entry_for_label(app, conn, &label) }))
}

fn run_host_command(
    app: &AppHandle,
    conn: &mut ConnRoute,
    method: &str,
    params: &Value,
) -> Result<Value, Refused> {
    match method {
        "hew.meta.documents" => Ok(json!({ "documents": documents_for(app, conn) })),
        "hew.doc.attach" => cmd_attach(app, conn, params),
        "hew.doc.new" => cmd_new(app, conn, params),
        "hew.doc.open" => cmd_open(app, conn, params),
        "hew.doc.close" => cmd_close(app, conn, params),
        "hew.doc.focus" => cmd_focus(app, conn, params),
        "hew.doc.save" => cmd_save(app, conn, params),
        "hew.doc.revert" => cmd_revert(app, conn, params),
        _ => Err(Refused::new("unimplemented", method.to_string())),
    }
}

/// An unsubscribe answered by window `label`'s session applies to the
/// connection, not to that one session: pass it to the connection's other
/// attached sessions, so an old window stops sending what was unsubscribed.
/// Best effort; a session that does not answer is left as it is.
fn unsubscribe_elsewhere(app: &AppHandle, conn: &ConnRoute, label: &str, line: &str, reply: &str) {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if request.get("method").and_then(Value::as_str) != Some("hew.event.unsubscribe")
        || error_code(reply).is_some()
    {
        return;
    }
    let others: Vec<String> = conn
        .opened
        .iter()
        .filter(|(other, attached)| {
            **attached && other.as_str() != label && app.get_webview_window(other).is_some()
        })
        .map(|(other, _)| other.clone())
        .collect();
    for (n, other) in others.into_iter().enumerate() {
        let frame = json!({
            "jsonrpc": "2.0",
            "id": format!("hew-unsub-{n}"),
            "method": "hew.event.unsubscribe",
            "params": request.get("params").cloned().unwrap_or_else(|| json!({})),
        })
        .to_string();
        if let Ok(other_reply) = send_and_await(app, &other, conn.conn_id, &frame, PROBE_TIMEOUT) {
            crate::live::note_subscriptions(app, conn.conn_id, &other, &frame, &other_reply);
        }
    }
}

// ------------------------------------------------------------------ entry point

/// Handles one sanitized client frame for `conn` and says what the
/// transport should do next. This is the single place a frame is routed;
/// both socket transports call it.
pub(crate) fn handle_frame(app: &AppHandle, conn: &mut ConnRoute, line: &str) -> Outcome {
    let mut routed = classify(line);
    // `hew.doc.save` is the shell's only when the app can do the save
    // itself (a path was given, or the document already has a file); an
    // unsaved document with no path falls through to the dispatcher, which
    // hands the bytes back for the client to write. `hew.doc.revert` is the
    // shell's only for a document that has a file; one without falls
    // through to the dispatcher, which refuses `nothing_to_revert`.
    if let Routed::Host {
        method: method @ ("hew.doc.save" | "hew.doc.revert"),
        params,
        ..
    } = &routed
    {
        let app_side = if *method == "hew.doc.save" {
            save_is_app_side(app, conn, params)
        } else {
            revert_is_app_side(app, conn, params)
        };
        if !app_side {
            routed = classify_skipping(line, Some(*method));
        }
    }
    match routed {
        Routed::Drop => Outcome::NoReply,
        Routed::Host {
            id,
            method,
            params,
            wrapped,
        } => {
            if conn.hello.is_none() {
                return Outcome::Reply(error_reply(&id, NOT_READY, "hew.meta.hello first"));
            }
            // An invalid-params refusal is protocol-level (-32602), not a
            // typed refusal — mirror the dispatcher's own classes.
            match run_host_command(app, conn, method, &params) {
                Ok(result) => Outcome::Reply(command_reply(&id, method, wrapped, result)),
                Err(r)
                    if r.name == "invalid_params"
                        || r.name == "invalid_path"
                        || r.name == "invalid_focus" =>
                {
                    Outcome::Reply(error_reply(&id, INVALID_PARAMS, &r.explanation))
                }
                Err(r) => {
                    Outcome::Reply(refusal_reply(&id, method, r.name, &r.explanation, r.detail))
                }
            }
        }
        Routed::Forward {
            line,
            document,
            needs_reply,
            is_hello,
        } => {
            // Remember the (sanitized) hello for session replay.
            if is_hello && conn.hello.is_none() {
                conn.hello = Some(line.clone());
            }
            let id = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            let target = match document.as_deref() {
                Some(query) => match target_of(app, conn, Some(query)) {
                    Ok((label, _)) => Ok(label),
                    Err(r) => Err(refusal_reply(
                        &id,
                        "hew.doc.attach",
                        r.name,
                        &r.explanation,
                        r.detail,
                    )),
                },
                None => current_window(app, conn.conn_id)
                    .ok_or_else(|| error_reply(&id, NO_DOCUMENT, "no document attached")),
            };
            let label = match target {
                Ok(label) => label,
                Err(reply) => {
                    return if needs_reply {
                        Outcome::Reply(reply)
                    } else {
                        Outcome::NoReply
                    }
                }
            };
            if document.is_some() {
                if let Err(r) = ensure_session(app, conn, &label) {
                    return if needs_reply {
                        Outcome::Reply(refusal_reply(
                            &id,
                            "hew.doc.attach",
                            r.name,
                            &r.explanation,
                            r.detail,
                        ))
                    } else {
                        Outcome::NoReply
                    };
                }
            }
            if !needs_reply {
                return if send_notification(app, &label, conn.conn_id, &line) {
                    Outcome::NoReply
                } else {
                    Outcome::Close
                };
            }
            match send_and_await(app, &label, conn.conn_id, &line, REPLY_TIMEOUT) {
                Ok(reply) => {
                    if is_hello {
                        let docs = documents_for(app, conn);
                        Outcome::Reply(patch_hello_documents(&reply, &docs))
                    } else {
                        crate::live::note_subscriptions(app, conn.conn_id, &label, &line, &reply);
                        unsubscribe_elsewhere(app, conn, &label, &line, &reply);
                        Outcome::Reply(reply)
                    }
                }
                Err(SendError::Timeout) => {
                    // The connection closes: abort whatever compound the
                    // window holds for its forward, so the document is
                    // not left held.
                    let _ = app.emit_to(
                        label.as_str(),
                        "hew://api-abort",
                        ConnPayload {
                            conn_id: conn.conn_id,
                        },
                    );
                    Outcome::ReplyThenClose(crate::live::synthesize_timeout_reply(&line))
                }
                Err(SendError::WindowGone) => {
                    // The window closed mid-request: detach, answer §4.2's
                    // -32002 and keep the connection for a re-attach.
                    app.state::<LiveApi>()
                        .conn_window
                        .lock()
                        .expect("conn_window mutex")
                        .remove(&conn.conn_id);
                    crate::live::clear_window_subscriptions(app, conn.conn_id, &label);
                    Outcome::Reply(error_reply(
                        &id,
                        NO_DOCUMENT,
                        "the attached document's window closed",
                    ))
                }
            }
        }
    }
}

/// Internal-fault reply helper kept for the transport's unreachable arms.
#[allow(dead_code)]
pub(crate) fn internal_fault(id: &Value, message: &str) -> String {
    error_reply(id, INTERNAL_FAULT, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(label: &str, title: &str, path: Option<&str>) -> WindowFacts {
        WindowFacts {
            label: label.into(),
            title: title.into(),
            path: path.map(str::to_string),
        }
    }

    #[test]
    fn document_ids_are_stable_per_window_and_never_reused() {
        let mut state = DocState::default();
        let a = build_documents(
            &mut state,
            &[
                facts("main", "House", None),
                facts("main-2", "Untitled", None),
            ],
            Some("main"),
            Some("main-2"),
        );
        assert_eq!(a[0]["id"], "doc_1");
        assert_eq!(a[1]["id"], "doc_2");
        assert_eq!(a[0]["front"], true);
        assert_eq!(a[1]["attached"], true);
        // `main` closes, a new window arrives: the freed id is NOT recycled.
        let b = build_documents(
            &mut state,
            &[
                facts("main-2", "Untitled", None),
                facts("main-3", "Scratch", None),
            ],
            None,
            None,
        );
        assert_eq!(b[0]["id"], "doc_2");
        assert_eq!(b[1]["id"], "doc_3");
    }

    #[test]
    fn the_unsaved_mark_is_not_part_of_the_title() {
        assert_eq!(strip_dirty_mark("• House"), "House");
        assert_eq!(strip_dirty_mark("House"), "House");
        assert_eq!(strip_dirty_mark("A • B"), "A • B");
    }

    #[test]
    fn documents_carry_dirty_path_and_label() {
        let mut state = DocState::default();
        state.set_dirty("main", true);
        state.labels.insert("main".into(), "scratch-a".into());
        let docs = build_documents(
            &mut state,
            &[facts("main", "House", Some("/m/house.hew"))],
            None,
            None,
        );
        assert_eq!(docs[0]["dirty"], true);
        assert_eq!(docs[0]["path"], "/m/house.hew");
        assert_eq!(docs[0]["label"], "scratch-a");
    }

    #[test]
    fn documents_carry_the_reported_document_id() {
        let mut state = DocState::default();
        let windows = [facts("main", "House", None), facts("main-2", "B", None)];
        let docs = build_documents(&mut state, &windows, None, None);
        assert_eq!(docs[0]["document_id"], Value::Null, "none reported yet");
        let id = "3f2c9107-5ed4-40a8-806b-1ec297405d83";
        state.set_document_id("main", Some(id.into()));
        let docs = build_documents(&mut state, &windows, None, None);
        assert_eq!(docs[0]["document_id"], id);
        assert_eq!(docs[1]["document_id"], Value::Null);
        state.set_document_id("main", None);
        let docs = build_documents(&mut state, &windows, None, None);
        assert_eq!(docs[0]["document_id"], Value::Null);
    }

    #[test]
    fn resolve_prefers_id_then_path_label_title_and_refuses_ambiguity() {
        let docs = vec![
            json!({"id":"doc_1","title":"Untitled","path":null,"label":"a"}),
            json!({"id":"doc_2","title":"Untitled","path":"/m/x.hew"}),
            json!({"id":"doc_3","title":"House","path":null}),
        ];
        assert_eq!(resolve(&docs, "doc_3").unwrap()["id"], "doc_3");
        assert_eq!(resolve(&docs, "/m/x.hew").unwrap()["id"], "doc_2");
        assert_eq!(resolve(&docs, "a").unwrap()["id"], "doc_1");
        assert_eq!(resolve(&docs, "House").unwrap()["id"], "doc_3");
        assert_eq!(
            resolve(&docs, "Untitled").unwrap_err().name,
            "ambiguous_document"
        );
        assert_eq!(resolve(&docs, "nope").unwrap_err().name, "unknown_document");
    }

    #[test]
    fn host_commands_are_recognized_bare_and_as_a_sole_transact_command() {
        let bare = classify(
            r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{"document":"doc_2"}}"#,
        );
        assert_eq!(
            bare,
            Routed::Host {
                id: json!(1),
                method: "hew.doc.attach",
                params: json!({"document":"doc_2"}),
                wrapped: false
            }
        );
        let wrapped = classify(
            r#"{"jsonrpc":"2.0","id":"t","method":"hew.doc.transact","params":{"commands":[{"method":"hew.doc.new","params":{"focus":"front"}}]}}"#,
        );
        assert_eq!(
            wrapped,
            Routed::Host {
                id: json!("t"),
                method: "hew.doc.new",
                params: json!({"focus":"front"}),
                wrapped: true
            }
        );
    }

    #[test]
    fn an_envelope_document_becomes_the_default_target_of_close_focus_and_attach() {
        // Bare and transact-wrapped (the shape MCP sends): the envelope's
        // `document` must reach the command, or `hew.doc.close` with
        // `discard` would close the connection's CURRENT document instead.
        let bare = classify(
            r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.close","params":{"discard":true},"document":"doc_2"}"#,
        );
        assert_eq!(
            bare,
            Routed::Host {
                id: json!(1),
                method: "hew.doc.close",
                params: json!({"discard":true,"document":"doc_2"}),
                wrapped: false
            }
        );
        let wrapped = classify(
            r#"{"jsonrpc":"2.0","id":2,"method":"hew.doc.transact","params":{"commands":[{"method":"hew.doc.focus"}]},"document":"doc_3"}"#,
        );
        assert_eq!(
            wrapped,
            Routed::Host {
                id: json!(2),
                method: "hew.doc.focus",
                params: json!({"document":"doc_3"}),
                wrapped: true
            }
        );
        // An explicit param wins over the envelope.
        let both = classify(
            r#"{"jsonrpc":"2.0","id":3,"method":"hew.doc.close","params":{"document":"doc_1"},"document":"doc_2"}"#,
        );
        match both {
            Routed::Host { params, .. } => assert_eq!(params["document"], "doc_1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn save_is_a_shell_command_until_the_router_decides_the_app_cannot_do_it() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.save","params":{"path":"/m/a.hew","copy":true},"document":"doc_2"}"#;
        match classify(line) {
            Routed::Host { method, params, .. } => {
                assert_eq!(method, "hew.doc.save");
                assert_eq!(params["copy"], true);
                assert_eq!(
                    params["document"], "doc_2",
                    "envelope document is the target"
                );
            }
            other => panic!("{other:?}"),
        }
        // A bare save with nothing to save to falls through to the webview's
        // dispatcher (which hands the bytes back), envelope document stripped.
        match classify_skipping(line, Some("hew.doc.save")) {
            Routed::Forward { document, line, .. } => {
                assert_eq!(document.as_deref(), Some("doc_2"));
                assert!(!line.contains("\"document\""));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn revert_is_a_shell_command_addressed_like_save() {
        let line =
            r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.revert","params":{},"document":"doc_2"}"#;
        match classify(line) {
            Routed::Host { method, params, .. } => {
                assert_eq!(method, "hew.doc.revert");
                assert_eq!(params["document"], "doc_2");
            }
            other => panic!("{other:?}"),
        }
        // A document with no file falls through to the dispatcher.
        match classify_skipping(line, Some("hew.doc.revert")) {
            Routed::Forward { document, .. } => assert_eq!(document.as_deref(), Some("doc_2")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_transact_with_other_commands_is_forwarded_whole() {
        let two = r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.transact","params":{"commands":[{"method":"hew.doc.new"},{"method":"hew.sketch.draw_rect"}]}}"#;
        assert!(matches!(classify(two), Routed::Forward { .. }));
    }

    #[test]
    fn host_command_notifications_are_dropped() {
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","method":"hew.doc.close","params":{}}"#),
            Routed::Drop
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":null,"method":"hew.doc.close","params":{}}"#),
            Routed::Drop
        );
    }

    #[test]
    fn the_envelope_document_member_is_stripped_and_extracted() {
        let routed = classify(
            r#"{"jsonrpc":"2.0","id":4,"method":"hew.query.scene","params":{},"document":"doc_2"}"#,
        );
        let Routed::Forward {
            line,
            document,
            needs_reply,
            is_hello,
        } = routed
        else {
            panic!("should forward");
        };
        assert_eq!(document.as_deref(), Some("doc_2"));
        assert!(needs_reply && !is_hello);
        assert!(!line.contains("document"));
        assert!(line.contains("hew.query.scene"));
    }

    #[test]
    fn plain_frames_forward_untouched_and_garbage_is_forwarded_for_a_parse_error() {
        let plain = r#"{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}"#;
        match classify(plain) {
            Routed::Forward { line, document, .. } => {
                assert_eq!(line, plain);
                assert_eq!(document, None);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify("{nope"),
            Routed::Forward {
                needs_reply: true,
                ..
            }
        ));
    }

    #[test]
    fn hello_is_marked_and_its_documents_are_patched() {
        match classify(
            r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
        ) {
            Routed::Forward { is_hello, .. } => assert!(is_hello),
            other => panic!("{other:?}"),
        }
        let reply = r#"{"jsonrpc":"2.0","id":0,"result":{"protocol":1,"documents":[]}}"#;
        let patched = patch_hello_documents(reply, &[json!({"id":"doc_1"})]);
        let v: Value = serde_json::from_str(&patched).unwrap();
        assert_eq!(v["result"]["documents"][0]["id"], "doc_1");
        // An error reply has no documents to patch and is left alone.
        let err = r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32602,"message":"x"}}"#;
        assert_eq!(patch_hello_documents(err, &[]), err);
    }

    #[test]
    fn transact_wrapping_matches_the_dispatchers_shape() {
        let r: Value = serde_json::from_str(&command_reply(
            &json!(1),
            "hew.doc.focus",
            true,
            json!({"a":1}),
        ))
        .unwrap();
        assert_eq!(r["result"]["results"][0]["a"], 1);
        assert_eq!(r["result"]["label"], "hew.doc.focus");
        let r: Value = serde_json::from_str(&command_reply(
            &json!(1),
            "hew.doc.focus",
            false,
            json!({"a":1}),
        ))
        .unwrap();
        assert_eq!(r["result"]["a"], 1);
    }

    #[test]
    fn refusal_replies_use_the_canonical_error_data_shape() {
        let r: Value = serde_json::from_str(&refusal_reply(
            &json!(9),
            "hew.doc.close",
            "unsaved_changes",
            "why",
            json!({"k":1}),
        ))
        .unwrap();
        assert_eq!(r["error"]["code"], -32000);
        assert_eq!(r["error"]["data"]["refusal"], "unsaved_changes");
        assert_eq!(r["error"]["data"]["failed_method"], "hew.doc.close");
        assert_eq!(r["error"]["data"]["failed_index"], 0);
        assert_eq!(r["error"]["data"]["detail"]["k"], 1);
    }
}
