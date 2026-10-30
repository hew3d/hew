//! `hew-cli mcp` (docs/agents/HEW_API.md §11.3, §13): a hand-rolled MCP stdio
//! server — newline-delimited JSON-RPC, one message per line, which is
//! what the MCP stdio transport (2024-11-05 revision) actually specifies
//! (no LSP-style `Content-Length` framing). Headless by default: each
//! session embeds its own `crates/api` connection + document + [`CliHost`].
//! With `--live`, a session instead discovers and forwards to a running
//! desktop instance through [`crate::live`] — every tool call becomes a
//! forwarded envelope, so "capability priming" (`hew_capabilities`) and
//! every other tool answer the REMOTE app, not this process's own registry
//! evaluation.
//!
//! [`McpServer`] is the whole server as a value — `handle_line` is a pure
//! state transition (one line in, at most one line out), so
//! `crates/hew-cli/tests/cli.rs` and this module's own unit tests drive it
//! directly with literal JSON-RPC strings; nothing here needs a spawned
//! process or real stdio to test.

use crate::host::CliHost;
use crate::live::{self, LiveOptions};
use api::{Connection, DispatchOutcome, Grant, Registry, Request, RequestId};
use kernel::Document;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// The MCP protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// A JSON-RPC error `{code, message}` — the MCP layer's own errors
/// (unknown method, bad params), distinct from a `hew.*` refusal (which
/// travels as a normal tool-call *result*, per §13: refusals are answers).
#[derive(Debug)]
struct JsonRpcErr {
    code: i64,
    message: String,
}

impl JsonRpcErr {
    fn method_not_found(method: &str) -> JsonRpcErr {
        JsonRpcErr {
            code: -32601,
            message: format!("unknown method \"{method}\""),
        }
    }

    fn invalid_params(message: impl Into<String>) -> JsonRpcErr {
        JsonRpcErr {
            code: -32602,
            message: message.into(),
        }
    }
}

/// The two ways an `McpServer` reaches a document: embedded (its own
/// in-process kernel + document) or live (forwarded to a running desktop
/// instance over [`crate::live`]). `doc` and `host` are boxed so the `Live`
/// variant — which is comparatively tiny — doesn't pay for the space every
/// `Backend` value reserves.
enum Backend {
    Embedded {
        conn: Box<Connection>,
        doc: Box<Document>,
        host: Box<CliHost>,
    },
    Live(Box<live::LiveSession>),
}

/// One long-lived MCP session. Embedded: a `Connection` hello'd and
/// attached to its own fresh document at construction, so the first
/// `tools/call` an agent makes already has a working document
/// (docs/agents/HEW_API.md §13's intended loop: describe → plan → transact →
/// look). Live: hello'd against a discovered running instance instead —
/// the document is whatever the user already has open.
pub struct McpServer {
    backend: Backend,
    /// The command registry, kept independent of `backend` (a live session
    /// has no local `Connection` to borrow one from). Embedded, it is the
    /// registry the session's connection dispatches against; live, it is
    /// protocol 1 plus the plugin commands the remote's capabilities listed
    /// at connect, used to shape the `tools/list` surface, the `hew_query`
    /// read-only gate, and the `provider` label on plugin results. Every
    /// actual command dispatch goes through `backend`, which is what
    /// answers with the real (possibly remote) authority.
    registry: Registry,
    /// Answers a plugin command an embedded session forwards. No plugin
    /// host runs inside `hew-cli` yet, so the default answers every
    /// forward `unavailable`.
    forward_responder: fn(&api::ForwardRequest) -> api::ForwardReply,
    /// The grant — `core` embedded, whatever the remote's hello reported
    /// when live (docs/agents/HEW_API.md §12: live is always `app`, but this
    /// reads it back rather than assuming).
    grant: Grant,
}

/// What the MCP client tells the model before it does anything, per the
/// `initialize` response's `instructions` field. An agent meeting Hew
/// for the first time knows nothing about it — Hew is new, and nothing
/// in any model's training describes it — so this has to carry enough to
/// start modeling: the one rule the data model follows, the units, the
/// shape of a build, and where to look up the rest. Everything else is
/// discoverable through `hew_capabilities`, which is why this stays a
/// page rather than a manual.
const INSTRUCTIONS: &str = r#"Hew is a 3D modeler. You build real, watertight solids in a document a
person can open, edit, and 3D-print.

ONE RULE GOVERNS EVERYTHING
Draw a closed shape, push or pull it, and it becomes a discrete
watertight solid — an Object. Objects never merge just because they
touch or overlap. When you want two solids to become one, say so with
hew.solid.union/subtract/intersect. This is the difference from mesh
modelers: you cannot produce a broken solid, and the worst answer you
get is a refusal explaining why.

UNITS
Lengths are meters, always, however the person has their display set. A
24 cm tray is 0.24. Angles are radians. Coordinates are right-handed
with +Z up.

HOW A MODEL GETS BUILT
1. Draw a closed profile on a plane: hew.sketch.draw_rect, draw_circle,
   draw_polygon, draw_arc, or draw_line segments that close a loop.
2. Turn it into a solid: hew.solid.extrude on the region it created.
3. Draw on an existing solid's face to imprint a new region there, then
   hew.solid.push_pull it — outward for a boss, inward (negative
   distance) to carve a recess or hollow a box.
4. Combine, move, rotate, group, paint, and tag from there.

CHAIN COMMANDS IN ONE TRANSACTION
hew_transact runs commands in order as one undo entry. Label a command
with "as", then reference what it made: {"$ref": "rect#/region_id"} for
an id, {"$face": "circle#face"} for a face the previous command cut.
Draw-then-extrude belongs in one transaction, not two.

START HERE
Call hew_capabilities first. It returns every command with its exact
parameters, results, and the refusals it can answer — that is the
authoritative reference, and it is generated from the same registry the
server dispatches through, so it is never out of date. Call
hew_describe_scene to see what already exists before changing it.

CHECK YOUR WORK
hew_snapshot renders the document to a PNG so you can look at what you
built rather than assume. hew_describe_scene reports each object's
bounding box and whether it is solid.

WHEN A COMMAND REFUSES
A refusal is a typed answer, not a crash: it carries a machine name and
an explanation of what to do instead, and the document is untouched.
Read the explanation and change the approach. Repeating the same call
gets the same refusal.

LIBRARY
hew_query serves hew.library.list (browse, filtered by category/query/
collection), hew.library.describe (one item's full metadata and
manifest summary), hew.library.remove, and hew.library.update_meta —
none of them touch the document, so they need no undo bracket.
hew_transact serves hew.library.insert (place a saved item into the
document — re-inserting the same item reuses its existing definition
instead of duplicating it) and hew.library.save (save a selection, or
the whole document with no selection, as a new item). Every command
addresses an item by its id or its library-relative path.

IF YOU ARE NOT SURE HOW TO MODEL SOMETHING
Plan it the way you would in SketchUp, then translate. Hew deliberately
follows SketchUp's interaction model — the same tools, in the same
order, for the same reasons — so a known SketchUp technique for a shape
almost always maps onto Hew commands one for one. Push/Pull, Follow Me,
Offset, and drawing on a face all behave as they do there.

Three differences to keep in mind while translating: a push/pulled
profile is a watertight solid immediately, with no "make it a group"
step; touching solids never merge on their own, so a boolean is
explicit; and a curve is a real analytic circle or arc, not a polyline,
so a hole stays a cylinder through later operations.
"#;

/// Appended to [`INSTRUCTIONS`] when the server is driving a running
/// desktop app rather than a document of its own. The stakes differ: the
/// edits are landing in someone's open window while they watch.
const LIVE_ADDENDUM: &str = r#"
YOU ARE EDITING SOMEONE'S OPEN DOCUMENT
This session is attached to a running Hew window. Your edits appear
there immediately and enter that person's undo history, so they can undo
anything you do. The model may already contain their work: call
hew_describe_scene before you change or delete anything, and do not
clear the document to make room. Frame what you build with
hew.view.zoom_extents so they can see it.

SEVERAL DOCUMENTS MAY BE OPEN
The person may have more than one document open, and you are not limited
to the front one. hew_query {"method":"hew.meta.documents"} lists them
(id, title, path, dirty, front, attached). Every tool accepts an optional
"document" (an id like doc_2, or an exact path, label, or title) that
sends just that one call to that document, in the background, without
changing which window is in front or which document you are attached to.
hew.doc.attach {"document": ...} changes the default for later calls.
Check "attached" before editing if you are unsure where you are.

hew.doc.new and hew.doc.open make a NEW window behind the one the person
is using — never replacing theirs — and attach you to it. Use hew.doc.new
for scratch work (give it a "label" so you can find it again), and
hew.doc.close {"discard": true} to clean up when you are done. Never close
a document you did not create unless asked. hew.doc.focus brings a window
to the front; do that only when the person should look at it.
"#;

impl McpServer {
    /// Headless: `hello` (protocol 1) then `hew.doc.new`, both internal —
    /// `hew.doc.new` auto-attaches on success (docs/agents/HEW_API.md §4.2), so no
    /// separate `attach` round-trip is needed either.
    pub fn new() -> McpServer {
        let grant = Grant::core();
        let registry = Registry::protocol_1();
        let mut conn = Connection::new(grant.clone(), "hew-cli:mcp");
        let mut doc = Document::new();
        let mut host = CliHost::new();
        for (method, params) in [
            ("hew.meta.hello", json!({ "protocol": 1 })),
            ("hew.doc.new", json!({})),
        ] {
            let request = Request {
                document: None,
                jsonrpc: "2.0".to_string(),
                id: Some(RequestId::Text(format!("startup-{method}"))),
                method: method.to_string(),
                params: Some(params),
            };
            let DispatchOutcome::Reply(response) =
                conn.dispatch(&registry, &mut doc, &mut host, request)
            else {
                panic!("{method} is a request, not a notification");
            };
            assert!(
                response.error.is_none(),
                "hew-cli mcp startup failed at {method}: {:?}",
                response.error
            );
        }
        McpServer {
            backend: Backend::Embedded {
                conn: Box::new(conn),
                doc: Box::new(doc),
                host: Box::new(host),
            },
            registry,
            forward_responder: no_plugin_host,
            grant,
        }
    }

    /// Live: discovers (and, with `opts.launch`, starts) a running desktop
    /// instance, hello's it with the discovery token, then attaches to its
    /// document (docs/agents/HEW_API.md §11.2, §12). The attach step matters
    /// because a live host serves a document the user already has open
    /// rather than creating one on connect — unlike [`McpServer::new`]
    /// above, whose embedded `hew.doc.new` auto-attaches for free, a live
    /// session has no such call to piggyback on, so without this every
    /// forwarded tool call (`hew_transact`, `hew_query`,
    /// `hew_describe_scene`, `hew_snapshot`) would answer `-32002 no
    /// document attached` the moment an agent tried to use it
    /// ([`crate::run::dispatch_live`]'s doc comment has the fuller
    /// rationale — this is the same fix, applied here via
    /// [`crate::run::attach_live`]). The grant comes back from the
    /// remote's own hello reply.
    pub fn new_live(opts: &LiveOptions) -> Result<McpServer, live::LiveError> {
        let hello_request = live::build_hello_request("hew-cli:mcp");
        let session = live::connect_live(opts, hello_request)?;
        let mut server = McpServer::from_live_session(session, opts.document.as_deref())?;
        server.subscribe_plugin_state();
        Ok(server)
    }

    /// Asks the app for `plugin_state` events (docs/agents/HEW_API.md §4.5),
    /// so the tool registry follows plugins loading and unloading
    /// ([`McpServer::refresh_on_plugin_state`]). An app too old to know
    /// events refuses; the registry then stays as connected.
    fn subscribe_plugin_state(&mut self) {
        let Backend::Live(session) = &mut self.backend else {
            return;
        };
        let request = Request {
            document: None,
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("mcp-subscribe".to_string())),
            method: "hew.event.subscribe".to_string(),
            params: Some(json!({ "events": ["plugin_state"] })),
        };
        let _ = session.dispatch(request);
    }

    /// Before each tool call and after each live dispatch: when a
    /// `plugin_state` event has arrived (set aside during a reply, or
    /// readable on the socket now, as one sent after the reply that caused
    /// it is), re-reads the app's capabilities into the registry, so a
    /// plugin that loaded or unloaded since is reflected in this tool call
    /// or the next. Other events are dropped (this session subscribed to
    /// nothing else).
    fn refresh_on_plugin_state(&mut self) {
        let Backend::Live(session) = &mut self.backend else {
            return;
        };
        let events = session.poll_events();
        if !events
            .iter()
            .any(|e| e.get("method").and_then(Value::as_str) == Some("hew.event.plugin_state"))
        {
            return;
        }
        let capabilities = Request {
            document: None,
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("mcp-capabilities".to_string())),
            method: "hew.meta.capabilities".to_string(),
            params: Some(json!({})),
        };
        if let Ok(DispatchOutcome::Reply(api::Response {
            result: Some(result),
            ..
        })) = session.dispatch(capabilities)
        {
            self.registry = Registry::from_capabilities(&result);
        }
        // What arrived while re-reading is already in the answer.
        let _ = session.take_events();
    }

    /// The rest of [`McpServer::new_live`] once the handshake is done:
    /// attach, then read the remote's capabilities once so the registry
    /// knows the plugin commands it serves (a capabilities call that fails
    /// leaves protocol 1 alone).
    fn from_live_session(
        mut session: live::LiveSession,
        document: Option<&str>,
    ) -> Result<McpServer, live::LiveError> {
        if let Err(msg) = crate::run::attach_live(&mut session, document) {
            return Err(live::LiveError::Protocol(msg));
        }
        let grant = session.granted_grant();
        let capabilities = Request {
            document: None,
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("mcp-capabilities".to_string())),
            method: "hew.meta.capabilities".to_string(),
            params: Some(json!({})),
        };
        let registry = match session.dispatch(capabilities)? {
            DispatchOutcome::Reply(api::Response {
                result: Some(result),
                ..
            }) => Registry::from_capabilities(&result),
            _ => Registry::protocol_1(),
        };
        Ok(McpServer {
            backend: Backend::Live(Box::new(session)),
            registry,
            forward_responder: no_plugin_host,
            grant,
        })
    }

    /// Handles one newline-delimited JSON-RPC message. Returns the reply
    /// line to write (already `\n`-free — the caller adds the newline), or
    /// `None` for a notification (no `id`), which — per JSON-RPC — gets no
    /// response even when it names an unknown method.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // A malformed frame carries no readable id — respond with
                // `id: null`, the standard JSON-RPC posture for parse
                // errors (there's no way to correlate it to a request).
                return Some(encode_frame(&error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                )));
            }
        };
        let has_id = value.get("id").is_some();
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        let params = value.get("params").cloned().unwrap_or_else(|| json!({}));

        if method == "notifications/initialized" {
            return None;
        }

        let result = match method {
            "initialize" => Ok(self.handle_initialize()),
            "tools/list" => Ok(json!({
                "tools": generate_tools(&self.registry, &self.grant)
            })),
            "tools/call" => self.handle_tools_call(&params),
            _ => Err(JsonRpcErr::method_not_found(method)),
        };

        if !has_id {
            // A client-originated notification naming an unknown method
            // still gets no reply — nothing to correlate it to.
            return None;
        }
        Some(encode_frame(&match result {
            Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
            Err(e) => error_response(id, e.code, &e.message),
        }))
    }

    fn handle_initialize(&self) -> Value {
        // `instructions` is what a client shows the model before its
        // first tool call. Hew is new enough that nothing in a model's
        // training describes it, so this is the difference between an
        // agent that starts modeling and one that guesses.
        let mut instructions = INSTRUCTIONS.to_string();
        if matches!(self.backend, Backend::Live(_)) {
            instructions.push_str(LIVE_ADDENDUM);
        }
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "hew-cli", "version": env!("CARGO_PKG_VERSION") },
            "instructions": instructions,
        })
    }

    fn handle_tools_call(&mut self, params: &Value) -> Result<Value, JsonRpcErr> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| JsonRpcErr::invalid_params("tools/call needs a \"name\""))?;
        let mut arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        // Every tool may name the document it addresses; it rides the
        // envelope, not the command's params (§4.2).
        let document = take_document(&mut arguments)?;
        // A plugin that loaded while this server sat idle is known before
        // the registry is consulted below.
        self.refresh_on_plugin_state();

        let response = match name {
            "hew_capabilities" => {
                self.dispatch_tool("hew.meta.capabilities", json!({}), document)?
            }
            "hew_transact" => self.dispatch_tool("hew.doc.transact", arguments, document)?,
            "hew_describe_scene" => self.dispatch_tool("hew.query.scene", json!({}), document)?,
            "hew_snapshot" => self.dispatch_tool("hew.view.snapshot", arguments, document)?,
            "hew_print_pdf" => self.dispatch_tool("hew.print.pdf", arguments, document)?,
            "hew_line_drawing" => {
                self.dispatch_tool("hew.view.line_drawing", arguments, document)?
            }
            "hew_query" => {
                let method = arguments
                    .get("method")
                    .and_then(Value::as_str)
                    .ok_or_else(|| JsonRpcErr::invalid_params("hew_query needs a \"method\""))?;
                let read_only = self
                    .registry
                    .get(method)
                    .map(|c| c.class == api::CommandClass::ReadOnly)
                    .unwrap_or(false);
                if !read_only {
                    return Err(JsonRpcErr::invalid_params(format!(
                        "hew_query only accepts read-only commands; \"{method}\" is not one (use hew_transact)"
                    )));
                }
                let sub_params = arguments
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                self.dispatch_tool(method, sub_params, document)?
            }
            other => {
                return Err(JsonRpcErr::invalid_params(format!(
                    "unknown tool \"{other}\""
                )));
            }
        };
        Ok(json!({
            "content": [{ "type": "text", "text": response.to_string() }],
        }))
    }

    /// Dispatches one `hew.*` envelope — embedded, through the session's
    /// own `Connection`; live, forwarded over the socket to the remote app
    /// (docs/agents/HEW_API.md §12: "every envelope the embedded path would
    /// dispatch locally is instead written to the socket") — and returns
    /// the whole JSON-RPC response (result or refusal) as a `Value`. An MCP
    /// tool call always succeeds at the JSON-RPC layer even when the
    /// underlying command refuses; the refusal is the answer, forwarded
    /// verbatim in live mode (§13).
    fn dispatch_tool(
        &mut self,
        method: &str,
        params: Value,
        document: Option<String>,
    ) -> Result<Value, JsonRpcErr> {
        let live = matches!(self.backend, Backend::Live(_));

        // `hew.library.list`/`describe`/`remove`/`update_meta` never touch
        // the document (docs/design/v1.1-cycle.md's Lane B) — in `--live`
        // mode they're answered entirely on THIS side (the remote app's
        // own host has no filesystem to serve them with), no dispatch at
        // all. Embedded mode needs none of this: its `CliHost` already
        // serves them directly through the ordinary path below.
        if live && crate::live_library::is_local_only(method, &params) {
            let response = crate::live_library::dispatch_local(method, &params);
            return Ok(serde_json::to_value(&response).expect("Response serializes"));
        }

        let client_params = params.clone();
        // A live host can serialize a document or encode a mesh but has no
        // disk to put the result on, so it refuses a `path` and hands the
        // bytes back instead. Strip the path here and write the file after
        // the reply, exactly as the script runner and one-shot dispatch do
        // — otherwise the canonical MCP shape for a solitary command (§6.4:
        // a one-command transaction) would be the ONE live entry point
        // where asking to save somewhere just refuses. `hew.library.insert`
        // and `hew.library.save` get the identical live-only pre/post
        // resolution the other two live entry points do.
        let (params, write_path) = if live {
            let params = match crate::live_library::resolve_insert_item(method, params) {
                Ok(p) => p,
                Err(response) => {
                    return Ok(serde_json::to_value(&response).expect("Response serializes"));
                }
            };
            let params = crate::live_library::force_return_bytes(method, params);
            crate::run::take_client_write_path(method, params)
        } else {
            (params, None)
        };
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("mcp".to_string())),
            method: method.to_string(),
            params: Some(params),
            document,
        };
        let outcome = match &mut self.backend {
            Backend::Embedded { conn, doc, host } => {
                match conn.dispatch(&self.registry, doc, &mut **host, request) {
                    DispatchOutcome::Forward(forward) => {
                        let reply = (self.forward_responder)(&forward);
                        DispatchOutcome::Reply(conn.complete_forward(
                            &self.registry,
                            forward.token,
                            reply,
                        ))
                    }
                    other => other,
                }
            }
            Backend::Live(session) => session.dispatch(request).map_err(|e| JsonRpcErr {
                code: -32603,
                message: format!("live transport error: {e}"),
            })?,
        };
        self.refresh_on_plugin_state();
        let DispatchOutcome::Reply(response) = outcome else {
            return Err(JsonRpcErr {
                code: -32603,
                message: "internal error: envelope dropped unexpectedly".to_string(),
            });
        };
        let mut value = serde_json::to_value(&response).expect("Response serializes");
        if response.error.is_none() {
            label_plugin_result(&self.registry, method, &client_params, &mut value);
        }
        if response.error.is_none()
            && let Some(outcome) = crate::live_library::finish_save(method, &client_params, &value)
        {
            return match outcome {
                Ok(rewritten) => Ok(serde_json::json!({
                    "jsonrpc": "2.0", "id": "mcp", "result": rewritten
                })),
                Err(message) => Err(JsonRpcErr {
                    code: -32603,
                    message,
                }),
            };
        }
        if let Some(path) = write_path
            && response.error.is_none()
        {
            {
                crate::run::write_live_bytes(&value, &path).map_err(|message| JsonRpcErr {
                    code: -32603,
                    message,
                })?;
                // Report what a filesystem host would have: the file, not
                // a megabyte of base64 the caller has no use for.
                return Ok(serde_json::json!({
                    "jsonrpc": "2.0", "id": "mcp", "result": { "saved": path }
                }));
            }
        }
        Ok(value)
    }
}

impl Default for McpServer {
    fn default() -> McpServer {
        McpServer::new()
    }
}

/// The default [`McpServer`] forward responder: no plugin runs here.
fn no_plugin_host(_: &api::ForwardRequest) -> api::ForwardReply {
    api::ForwardReply::Failed(api::ForwardFailure::Unavailable)
}

/// Labels a successful plugin command's result with its provider, so an
/// agent can tell a plugin's answer from Hew's own (docs/agents/HEW_API.md
/// §13): an object result gains `"provider": "<plugin id>"`; anything else
/// becomes `{ "provider", "value" }`. For a `hew.doc.transact` whose one
/// command is a plugin command, the label goes on `results[0]`.
fn label_plugin_result(registry: &Registry, method: &str, params: &Value, response: &mut Value) {
    let (plugin, target) = if method == "hew.doc.transact" {
        let commands = params.get("commands").and_then(Value::as_array);
        let Some([only]) = commands.map(Vec::as_slice) else {
            return;
        };
        let Some(plugin) = only
            .get("method")
            .and_then(Value::as_str)
            .and_then(|m| registry.get(m))
            .and_then(|c| c.plugin_id())
        else {
            return;
        };
        (plugin, response.pointer_mut("/result/results/0"))
    } else {
        let Some(plugin) = registry.get(method).and_then(|c| c.plugin_id()) else {
            return;
        };
        (plugin, response.get_mut("result"))
    };
    let Some(target) = target else {
        return;
    };
    match target {
        Value::Object(map) => {
            map.insert("provider".to_string(), Value::String(plugin.to_string()));
        }
        other => {
            let value = other.take();
            *other = json!({ "provider": plugin, "value": value });
        }
    }
}

/// Pulls the optional `document` out of a tool call's arguments — it
/// addresses the call (an envelope member), so it must not reach the
/// command's own params, which reject unknown fields.
fn take_document(arguments: &mut Value) -> Result<Option<String>, JsonRpcErr> {
    let Some(object) = arguments.as_object_mut() else {
        return Ok(None);
    };
    match object.remove("document") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(document)) => Ok(Some(document)),
        Some(_) => Err(JsonRpcErr::invalid_params(
            "\"document\" must be a string (a document id, path, label, or title)",
        )),
    }
}

/// The `document` property every tool accepts.
fn document_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional. Send just this call to that open document — a document id (doc_2), or an exact path, label, or title (hew_query hew.meta.documents lists them) — instead of the one you are attached to. It does not change your attachment or the user's front window.",
    })
}

/// The MCP tool inventory (docs/agents/HEW_API.md §13): a small set of chunky
/// tools generated from the registry rather than one tool per command.
/// `hew_snapshot` is included whenever the connection's grant covers
/// `hew.view.snapshot` — true for both first-party grants, since core grants
/// that one `hew.view.*` command specifically (it renders through a
/// headless software rasterizer, no viewport required — see
/// docs/design/headless-snapshot.md); a future app-only `hew.view.*`
/// addition would be the next tool a headless session lacks (an agent
/// never sees a tool it cannot call).
pub fn generate_tools(registry: &Registry, grant: &Grant) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "hew_capabilities",
            // The registry's own summary is written for someone reading
            // the spec; an agent needs to know why this is the first call
            // to make.
            "description": "Every command this server can run, with exact parameters, results, and the refusals each one can answer. Call this first — it is generated from the same registry the server dispatches through, so it cannot drift from what actually works.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }),
        json!({
            "name": "hew_transact",
            "description": "Run hew.* commands in order as one undo entry. Every edit goes through this. Put a draw and the extrude that consumes it in the SAME call, labeling the draw with \"as\" and referencing its result: {\"$ref\": \"rect#/region_id\"} for an id, {\"$face\": \"circle#face\"} for a face it just cut. If any command refuses, none of them applied. Commands provided by installed plugins are listed by hew_capabilities with a provider; their results carry that provider.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "commands": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "method": { "type": "string" },
                                "as": { "type": "string" },
                                "params": { "type": "object" },
                            },
                            "required": ["method"],
                        },
                    },
                },
                "required": ["commands"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "hew_query",
            "description": "Run one read-only command (hew.query.*, hew.meta.*, hew.attr.get) — measuring, resolving an id, listing faces, raycasting — or one of the read-side Library commands (hew.library.list/describe/remove/update_meta): browsing, deleting, or renaming a saved item. None of these change the attached DOCUMENT (remove/update_meta change on-disk library state, not the document itself). Use hew_transact for anything that does. Commands provided by installed plugins are listed by hew_capabilities with a provider; their results carry that provider.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "method": { "type": "string" },
                    "params": { "type": "object" },
                },
                "required": ["method"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "hew_describe_scene",
            "description": "What is in the document right now: every object, group, component and sketch, with names, ids, bounding boxes, and whether each solid is watertight. Call this before changing an existing model, and after building to confirm you made what you intended.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }),
    ];
    let snapshot_granted = registry
        .get("hew.view.snapshot")
        .map(|c| grant.grants(c))
        .unwrap_or(false);
    if snapshot_granted {
        tools.push(json!({
            "name": "hew_snapshot",
            "description": "Render the document to a PNG so you can SEE what you built instead of inferring it from numbers. Worth doing after anything with a shape you are unsure of. Pass a path to write the file; the inline bytes get large fast.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": ["iso", "front", "back", "left", "right", "top", "bottom"] },
                    "camera": { "type": "object" },
                    "width": { "type": "integer", "minimum": 16, "maximum": 2048 },
                    "height": { "type": "integer", "minimum": 16, "maximum": 2048 },
                    "include_ids": { "type": "boolean" },
                    "include_depth": {
                        "type": "boolean",
                        "description": "also return a depth pass (grayscale PNG, near = white, background 0) with depth_near/depth_far in metres — the conditioning image a depth-aware image model expects",
                    },
                    "depth_bits": { "type": "integer", "enum": [8, 16], "description": "bits per pixel of the depth PNG (default 8); needs include_depth" },
                    "include_normals": {
                        "type": "boolean",
                        "description": "also return a camera-space normals pass (RGB PNG; x right, y up, z toward the viewer)",
                    },
                    "path": {
                        "type": "string",
                        "description": "write the PNG here instead of returning it inline — the inline bytes can exceed an MCP tool result's size budget at any useful resolution",
                    },
                },
                "additionalProperties": false,
            },
        }));
    }
    if registry
        .get("hew.print.pdf")
        .map(|c| grant.grants(c))
        .unwrap_or(false)
    {
        tools.push(json!({
            "name": "hew_print_pdf",
            "description": "Print the document to a PDF the way File ▸ Print… does — a drawing at an exact scale (default 1:10, parallel projection, hidden lines removed, tiled across pages with a scale bar and title block), or a one-page standard view. Pass a path; the inline bytes get large.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": ["iso", "front", "back", "left", "right", "top", "bottom"] },
                    "camera": { "type": "object" },
                    "paper": { "description": "letter | legal | tabloid | a5 | a4 | a3 (default a4), or {w_mm, h_mm}" },
                    "orientation": { "type": "string", "enum": ["auto", "portrait", "landscape"] },
                    "mode": { "type": "string", "enum": ["scaled", "standard"] },
                    "scale": { "type": "number", "description": "paper/model, e.g. 0.1 for 1:10, 1 for full size" },
                    "style": { "type": "string", "enum": ["line_art", "shaded"] },
                    "include_hidden": { "type": "boolean" },
                    "units": { "type": "string", "enum": ["metric", "imperial"] },
                    "title": { "type": "string" },
                    "path": { "type": "string", "description": "write the PDF here instead of returning it inline" },
                },
                "additionalProperties": false,
            },
        }));
    }
    if registry
        .get("hew.view.line_drawing")
        .map(|c| grant.grants(c))
        .unwrap_or(false)
    {
        tools.push(json!({
            "name": "hew_line_drawing",
            "description": "A hidden-line drawing of the document from a view or camera: visible edges, curved-wall silhouettes, section-cut outlines, optionally dashed hidden lines — as a true-size SVG at a drawing scale (pass a path) or raw 2D segments.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": ["iso", "front", "back", "left", "right", "top", "bottom"] },
                    "camera": { "type": "object" },
                    "format": { "type": "string", "enum": ["svg", "segments"] },
                    "scale": { "type": "number" },
                    "include_hidden": { "type": "boolean" },
                    "path": { "type": "string" },
                },
                "additionalProperties": false,
            },
        }));
    }
    for tool in &mut tools {
        if tool["name"] == "hew_capabilities" {
            continue;
        }
        tool["inputSchema"]["properties"]["document"] = document_property();
    }
    tools
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// One JSON-RPC frame, encoded compact. The MCP stdio transport is
/// newline-delimited (2024-11-05 revision) — the caller adds the `\n`.
fn encode_frame(value: &Value) -> String {
    serde_json::to_string(value).expect("Value serializes")
}

/// Reads newline-delimited JSON-RPC from stdin and writes replies to
/// stdout until EOF. The process boundary `src/main.rs` calls into; the
/// state machine it drives is [`McpServer::handle_line`]. `live`, if
/// given, discovers (and connects to) a running desktop instance before
/// the loop starts — a discovery/connect failure here means the process
/// exits before ever reading a line of MCP traffic.
pub fn run_stdio(live: Option<&LiveOptions>) -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut server = match live {
        None => McpServer::new(),
        Some(opts) => match McpServer::new_live(opts) {
            Ok(server) => server,
            Err(e) => {
                eprintln!("hew-cli mcp --live: {e}");
                return 1;
            }
        },
    };
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("hew-cli mcp: stdin read error: {e}");
                return 1;
            }
        }
        if let Some(response) = server.handle_line(&line)
            && (writeln!(stdout, "{response}").is_err() || stdout.flush().is_err())
        {
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `com.example.lib`: read-only `sum` (an object result) and `count`
    /// (a bare-number result).
    fn sample_plugin() -> Vec<plugin_manifest::CommandContribution> {
        serde_json::from_value(json!([
            { "id": "sum", "class": "read_only", "summary": "Adds." },
            {
                "id": "count", "class": "read_only", "summary": "Counts.",
                "result": { "type": "object", "properties": {}, "additionalProperties": false }
            }
        ]))
        .unwrap()
    }

    fn tool_text(server: &mut McpServer, call: Value) -> Value {
        let frame = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": call });
        let line = server.handle_line(&frame.to_string()).expect("a reply");
        let v: Value = serde_json::from_str(&line).unwrap();
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().expect(&line)).unwrap()
    }

    #[test]
    fn plugin_results_carry_provider() {
        fn answer(_: &api::ForwardRequest) -> api::ForwardReply {
            api::ForwardReply::Ok(json!({}))
        }
        let mut server = McpServer::new();
        server
            .registry
            .register_plugin("com.example.lib", &sample_plugin())
            .unwrap();
        server.forward_responder = answer;

        let query = tool_text(
            &mut server,
            json!({ "name": "hew_query", "arguments": { "method": "com.example.lib.sum" } }),
        );
        assert_eq!(query["result"], json!({ "provider": "com.example.lib" }));
        let transact = tool_text(
            &mut server,
            json!({ "name": "hew_transact", "arguments": { "commands": [{ "method": "com.example.lib.sum" }] } }),
        );
        assert_eq!(
            transact["result"],
            json!({ "results": [{ "provider": "com.example.lib" }], "label": "com.example.lib.sum" })
        );
        // First-party results are untouched.
        let scene = tool_text(
            &mut server,
            json!({ "name": "hew_describe_scene", "arguments": {} }),
        );
        assert!(scene["result"].get("provider").is_none());

        // A non-object result is wrapped.
        let mut value = json!({ "jsonrpc": "2.0", "id": "mcp", "result": 7 });
        label_plugin_result(
            &server.registry,
            "com.example.lib.count",
            &json!({}),
            &mut value,
        );
        assert_eq!(
            value["result"],
            json!({ "provider": "com.example.lib", "value": 7 })
        );
        let mut value =
            json!({ "jsonrpc": "2.0", "id": "mcp", "result": { "results": [[1]], "label": "x" } });
        label_plugin_result(
            &server.registry,
            "hew.doc.transact",
            &json!({ "commands": [{ "method": "com.example.lib.count" }] }),
            &mut value,
        );
        assert_eq!(
            value["result"]["results"][0],
            json!({ "provider": "com.example.lib", "value": [1] })
        );

        // With no plugin host, the embedded default answers unavailable.
        server.forward_responder = no_plugin_host;
        let refused = tool_text(
            &mut server,
            json!({ "name": "hew_query", "arguments": { "method": "com.example.lib.sum" } }),
        );
        assert_eq!(refused["error"]["data"]["refusal"], "plugin_unavailable");
    }

    /// A fake desktop instance answers hello, attach, and a capabilities
    /// frame listing one plugin command, then one forwarded query: the
    /// live registry knows the command, `hew_query` lets it through, and
    /// its result carries the provider.
    #[cfg(unix)]
    #[test]
    fn live_mcp_lists_plugin_commands_from_capabilities() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let socket_path = std::path::PathBuf::from("/tmp")
            .join(format!("hew-mcp-caps-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("bind the fake socket");
        let mut registry = Registry::protocol_1();
        registry
            .register_plugin("com.example.lib", &sample_plugin())
            .unwrap();
        let plugin_entry = {
            let mut conn = Connection::new(Grant::app(), "fake");
            let mut doc = Document::new();
            let request = |id: &str, method: &str| Request {
                document: None,
                jsonrpc: "2.0".into(),
                id: Some(RequestId::Text(id.into())),
                method: method.into(),
                params: Some(json!({ "protocol": 1 })),
            };
            conn.dispatch(
                &registry,
                &mut doc,
                &mut api::NoHost,
                request("h", "hew.meta.hello"),
            );
            let DispatchOutcome::Reply(caps) = conn.dispatch(
                &registry,
                &mut doc,
                &mut api::NoHost,
                request("c", "hew.meta.capabilities"),
            ) else {
                panic!("capabilities replies")
            };
            caps.result.unwrap()["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == "com.example.lib.sum")
                .unwrap()
                .clone()
        };

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut next = || {
                let mut line = String::new();
                reader.read_line(&mut line).expect("read a frame");
                serde_json::from_str::<Value>(line.trim_end()).unwrap()
            };
            let hello = next();
            writeln!(
                writer,
                "{}",
                json!({ "jsonrpc": "2.0", "id": hello["id"], "result": {
                "protocol": 1, "app": { "name": "hew", "version": "1.3.0" },
                "profile": "app", "encoding": "json", "documents": [] } })
            )
            .unwrap();
            let attach = next();
            assert_eq!(attach["method"], "hew.doc.attach");
            writeln!(
                writer,
                "{}",
                json!({ "jsonrpc": "2.0", "id": attach["id"], "result": {} })
            )
            .unwrap();
            let caps = next();
            assert_eq!(caps["method"], "hew.meta.capabilities");
            writeln!(writer, "{}", json!({ "jsonrpc": "2.0", "id": caps["id"], "result": {
                "commands": [plugin_entry, { "name": "com.bad.x", "provider": { "plugin": "com.bad" } }] } }))
            .unwrap();
            let query = next();
            assert_eq!(query["method"], "com.example.lib.sum");
            writeln!(
                writer,
                "{}",
                json!({ "jsonrpc": "2.0", "id": query["id"], "result": { "sum": 3 } })
            )
            .unwrap();
        });

        let instance = live::Instance {
            socket: socket_path.to_str().unwrap().to_string(),
            token: "t".to_string(),
            pid: std::process::id(),
            version: "1.3.0".to_string(),
            file: std::path::PathBuf::new(),
        };
        let session =
            live::LiveSession::handshake(instance, live::build_hello_request("hew-cli:mcp"))
                .expect("handshake");
        let mut mcp = McpServer::from_live_session(session, None).expect("live MCP connects");
        let sum = mcp
            .registry
            .get("com.example.lib.sum")
            .expect("listed from capabilities");
        assert!(sum.is_plugin());
        assert!(mcp.registry.get("com.bad.x").is_none());

        let answer = tool_text(
            &mut mcp,
            json!({ "name": "hew_query", "arguments": { "method": "com.example.lib.sum" } }),
        );
        assert_eq!(
            answer["result"],
            json!({ "sum": 3, "provider": "com.example.lib" })
        );
        server.join().expect("fake instance finishes");
        let _ = std::fs::remove_file(&socket_path);
    }

    /// `plugin_state` refreshes the live registry: the fake answers
    /// capabilities twice — before and after a plugin loads — and writes the
    /// event ahead of an unrelated tool call's reply.
    #[cfg(unix)]
    #[test]
    fn mcp_refreshes_registry_on_plugin_state() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let socket_path = std::path::PathBuf::from("/tmp")
            .join(format!("hew-mcp-refresh-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("bind the fake socket");
        let mut registry = Registry::protocol_1();
        registry
            .register_plugin("com.example.lib", &sample_plugin())
            .unwrap();
        let capabilities = |registry: &Registry| {
            let mut conn = Connection::new(Grant::app(), "fake");
            let mut doc = Document::new();
            let request = |id: &str, method: &str| Request {
                document: None,
                jsonrpc: "2.0".into(),
                id: Some(RequestId::Text(id.into())),
                method: method.into(),
                params: Some(json!({ "protocol": 1 })),
            };
            conn.dispatch(
                registry,
                &mut doc,
                &mut api::NoHost,
                request("h", "hew.meta.hello"),
            );
            let DispatchOutcome::Reply(caps) = conn.dispatch(
                registry,
                &mut doc,
                &mut api::NoHost,
                request("c", "hew.meta.capabilities"),
            ) else {
                panic!("capabilities replies")
            };
            caps.result.unwrap()
        };
        let before = capabilities(&Registry::protocol_1());
        let after = capabilities(&registry);

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut next = || {
                let mut line = String::new();
                reader.read_line(&mut line).expect("read a frame");
                serde_json::from_str::<Value>(line.trim_end()).unwrap()
            };
            let reply = |writer: &mut std::os::unix::net::UnixStream, id: &Value, result: Value| {
                writeln!(
                    writer,
                    "{}",
                    json!({ "jsonrpc": "2.0", "id": id, "result": result })
                )
                .unwrap();
            };
            let hello = next();
            reply(
                &mut writer,
                &hello["id"],
                json!({
                "protocol": 1, "app": { "name": "hew", "version": "1.3.0" },
                "profile": "app", "encoding": "json", "documents": [] }),
            );
            let mut methods = Vec::new();
            for answer in 0..6 {
                let frame = next();
                methods.push(frame["method"].as_str().unwrap().to_string());
                let result = match frame["method"].as_str().unwrap() {
                    "hew.meta.capabilities" if answer < 3 => before.clone(),
                    "hew.meta.capabilities" => after.clone(),
                    "hew.event.subscribe" => {
                        json!({ "subscribed": ["plugin_state"], "denied": [] })
                    }
                    "com.example.lib.sum" => json!({ "sum": 3 }),
                    _ => json!({}),
                };
                reply(&mut writer, &frame["id"], result);
                if frame["method"] == "hew.query.scene" {
                    // The plugin loaded; the app reports it after the reply
                    // it was sending, as the live bridge orders them.
                    writeln!(
                        writer,
                        "{}",
                        json!({ "jsonrpc": "2.0", "method": "hew.event.plugin_state",
                        "params": { "plugin": "com.example.lib", "state": "loaded" } })
                    )
                    .unwrap();
                }
            }
            methods
        });

        let instance = live::Instance {
            socket: socket_path.to_str().unwrap().to_string(),
            token: "t".to_string(),
            pid: std::process::id(),
            version: "1.3.0".to_string(),
            file: std::path::PathBuf::new(),
        };
        let session =
            live::LiveSession::handshake(instance, live::build_hello_request("hew-cli:mcp"))
                .expect("handshake");
        let mut mcp = McpServer::from_live_session(session, None).expect("live MCP connects");
        mcp.subscribe_plugin_state();
        assert!(mcp.registry.get("com.example.lib.sum").is_none());
        let _ = tool_text(
            &mut mcp,
            json!({ "name": "hew_query", "arguments": { "method": "hew.query.scene" } }),
        );
        // Give the event, sent after that reply, time to land.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // The very next call reaches the newly loaded command: the registry
        // was refreshed before it was consulted.
        let answer = tool_text(
            &mut mcp,
            json!({ "name": "hew_query", "arguments": { "method": "com.example.lib.sum" } }),
        );
        assert_eq!(answer["result"]["sum"], 3, "{answer}");
        assert_eq!(
            server.join().expect("fake instance finishes"),
            vec![
                "hew.doc.attach",
                "hew.meta.capabilities",
                "hew.event.subscribe",
                "hew.query.scene",
                "hew.meta.capabilities",
                "com.example.lib.sum"
            ]
        );
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn every_tool_but_capabilities_accepts_a_document() {
        let registry = Registry::protocol_1();
        for tool in generate_tools(&registry, &Grant::app()) {
            let has = tool["inputSchema"]["properties"].get("document").is_some();
            assert_eq!(
                has,
                tool["name"] != "hew_capabilities",
                "{} document property",
                tool["name"]
            );
        }
    }

    #[test]
    fn take_document_removes_it_from_the_command_arguments() {
        let mut args = json!({ "commands": [], "document": "doc_2" });
        assert_eq!(take_document(&mut args).unwrap().as_deref(), Some("doc_2"));
        assert!(args.get("document").is_none(), "must not reach the params");
        let mut none = json!({ "commands": [] });
        assert_eq!(take_document(&mut none).unwrap(), None);
        let mut null = json!({ "document": null });
        assert_eq!(take_document(&mut null).unwrap(), None);
        let mut bad = json!({ "document": 3 });
        assert!(take_document(&mut bad).is_err());
    }

    #[test]
    fn an_unknown_document_is_refused_by_the_embedded_session_not_ignored() {
        let mut server = McpServer::new();
        let reply = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hew_describe_scene","arguments":{"document":"no-such-doc"}}}"#,
            )
            .expect("a request gets a reply");
        let v: Value = serde_json::from_str(&reply).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown_document"), "{text}");
        // The headless session's own document answers to doc_1.
        let ok = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hew_describe_scene","arguments":{"document":"doc_1"}}}"#,
            )
            .expect("reply");
        let v: Value = serde_json::from_str(&ok).unwrap();
        assert!(
            !v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown_document")
        );
    }

    #[test]
    fn frame_round_trips_compact_json() {
        let v = json!({ "a": 1, "b": [1, 2, 3] });
        let line = encode_frame(&v);
        assert!(!line.contains('\n'));
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn core_profile_tool_list_includes_snapshot() {
        // Core now grants hew.view.snapshot specifically — its headless
        // render path (docs/design/headless-snapshot.md) means a headless
        // MCP session can look at what it built too.
        let registry = Registry::protocol_1();
        let tools = generate_tools(&registry, &Grant::core());
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "hew_capabilities",
                "hew_transact",
                "hew_query",
                "hew_describe_scene",
                "hew_snapshot",
                "hew_print_pdf",
                "hew_line_drawing",
            ]
        );
    }

    /// A plugin grant sees only the tools it can call: without
    /// `model.read` there is no `hew_snapshot`.
    #[test]
    fn plugin_grant_tool_list_follows_its_permissions() {
        let registry = Registry::protocol_1();
        let no_read = Grant::plugin(
            "com.example.p",
            Some("doc_1"),
            [api::Permission::ModelWrite].into_iter().collect(),
        );
        let tools = generate_tools(&registry, &no_read);
        assert!(!tools.iter().any(|t| t["name"] == "hew_snapshot"));
        let read = Grant::plugin(
            "com.example.p",
            Some("doc_1"),
            [api::Permission::ModelRead].into_iter().collect(),
        );
        let tools = generate_tools(&registry, &read);
        assert!(tools.iter().any(|t| t["name"] == "hew_snapshot"));
    }

    #[test]
    fn app_profile_tool_list_includes_snapshot() {
        let registry = Registry::protocol_1();
        let tools = generate_tools(&registry, &Grant::app());
        assert!(tools.iter().any(|t| t["name"] == "hew_snapshot"));
    }

    /// `path` is a passthrough parameter (docs/agents/HEW_API.md §7): the tool
    /// schema must expose it or an MCP client has no way to ask for it —
    /// this is the whole fix for the inline-PNG size problem, so its
    /// absence here would silently undo it.
    #[test]
    fn hew_snapshot_tool_schema_exposes_path() {
        let registry = Registry::protocol_1();
        let tools = generate_tools(&registry, &Grant::core());
        let snapshot = tools
            .iter()
            .find(|t| t["name"] == "hew_snapshot")
            .expect("hew_snapshot is in core's tool list");
        assert_eq!(
            snapshot["inputSchema"]["properties"]["path"]["type"],
            "string"
        );
    }

    #[test]
    fn every_tool_carries_a_name_description_and_input_schema() {
        for profile in [Grant::core(), Grant::app()] {
            let registry = Registry::protocol_1();
            for tool in generate_tools(&registry, &profile) {
                assert!(tool["name"].is_string());
                assert!(tool["description"].is_string());
                assert_eq!(tool["inputSchema"]["type"], "object");
            }
        }
    }

    /// A model meeting Hew has nothing about it in training — Hew is
    /// new. The `initialize` reply is the one chance to say what this is
    /// before the first tool call, so it carries a primer: the data
    /// model's governing rule, the units (an agent will otherwise type
    /// 24 for 24 cm), how a model is actually built, and where to look
    /// the rest up.
    #[test]
    fn initialize_briefs_an_agent_that_has_never_heard_of_hew() {
        let mut server = McpServer::new();
        let out = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("initialize replies");
        let v: Value = serde_json::from_str(&out).unwrap();
        let instructions = v["result"]["instructions"]
            .as_str()
            .expect("the initialize reply carries instructions");

        // Units, because getting this wrong builds a model 100x too big.
        assert!(instructions.contains("meters"));
        assert!(instructions.contains("radians"));
        // The rule the whole data model follows.
        assert!(instructions.contains("watertight"));
        // The build loop and the chaining that makes it one undo entry.
        assert!(instructions.contains("$ref") && instructions.contains("$face"));
        // Where to find everything this primer does not say.
        assert!(instructions.contains("hew_capabilities"));
        // And what to do when a command says no.
        assert!(instructions.contains("refus"));
    }

    /// Hew follows SketchUp's interaction model closely enough that a
    /// known SketchUp technique usually maps command for command, so an
    /// agent stuck on how to model something is told to plan it that way
    /// and translate — along with the differences that would mislead it.
    #[test]
    fn initialize_tells_an_agent_to_fall_back_to_sketchup_technique() {
        let mut server = McpServer::new();
        let out = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("initialize replies");
        let v: Value = serde_json::from_str(&out).unwrap();
        let instructions = v["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("SketchUp"),
            "the fallback is the difference between an agent that models and one that guesses"
        );
        // Translating blind would mislead without these.
        assert!(instructions.contains("boolean"));
    }

    /// Every tool says when to reach for it, not just what it maps to.
    #[test]
    fn each_tool_description_tells_an_agent_when_to_use_it() {
        let mut server = McpServer::new();
        let out = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
            .expect("tools/list replies");
        let v: Value = serde_json::from_str(&out).unwrap();
        for tool in v["result"]["tools"].as_array().unwrap() {
            let desc = tool["description"].as_str().unwrap();
            assert!(
                desc.len() > 80,
                "{} needs more than a restatement of its name: {desc}",
                tool["name"]
            );
        }
    }

    #[test]
    fn initialize_negotiates_the_2024_11_05_revision() {
        let mut server = McpServer::new();
        let line = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("initialize replies");
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(v["result"]["serverInfo"]["name"], "hew-cli");
    }

    #[test]
    fn notifications_initialized_gets_no_reply() {
        let mut server = McpServer::new();
        let out = server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#);
        assert!(out.is_none());
    }

    #[test]
    fn tools_list_reflects_the_headless_core_profile() {
        let mut server = McpServer::new();
        let line = server
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            7,
            "headless core includes hew_snapshot, hew_print_pdf, and hew_line_drawing (headless render paths)"
        );
    }

    #[test]
    fn tools_call_hew_capabilities_returns_the_registry_as_a_tool_result() {
        let mut server = McpServer::new();
        let line = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"hew_capabilities","arguments":{}}}"#,
            )
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let inner: Value = serde_json::from_str(text).unwrap();
        assert!(inner["result"]["commands"].as_array().unwrap().len() >= 50);
    }

    #[test]
    fn tools_call_hew_query_refuses_a_non_read_only_command_before_dispatching() {
        let mut server = McpServer::new();
        let line = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"hew_query","arguments":{"method":"hew.solid.extrude","params":{}}}}"#,
            )
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_hew_describe_scene_dispatches_query_scene() {
        let mut server = McpServer::new();
        let line = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"hew_describe_scene","arguments":{}}}"#,
            )
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let inner: Value = serde_json::from_str(text).unwrap();
        // hew.query.scene is a different wave's command; either it answers
        // (result) or it is still the burn-down `unimplemented` refusal —
        // both are a well-formed dispatch, which is what this test pins.
        assert!(inner.get("result").is_some() || inner.get("error").is_some());
    }

    #[test]
    fn unknown_method_is_json_rpc_method_not_found() {
        let mut server = McpServer::new();
        let line = server
            .handle_line(r#"{"jsonrpc":"2.0","id":6,"method":"nope","params":{}}"#)
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn malformed_json_is_parse_error_with_null_id() {
        let mut server = McpServer::new();
        let line = server.handle_line("{not json").unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["error"]["code"], -32700);
        assert_eq!(v["id"], Value::Null);
    }
}
