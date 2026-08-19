//! `hew-cli mcp` (docs/HEW_API.md §11.3, §13): a hand-rolled MCP stdio
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
use api::{Connection, DispatchOutcome, Profile, Registry, Request, RequestId};
use kernel::Document;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// The MCP protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// A JSON-RPC error `{code, message}` — the MCP layer's own errors
/// (unknown method, bad params), distinct from a `hew.*` refusal (which
/// travels as a normal tool-call *result*, per §13: refusals are answers).
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
/// instance over [`crate::live`]). `doc` is boxed so the `Live` variant —
/// which is comparatively tiny — doesn't pay for the `Document`-sized
/// space every `Backend` value reserves.
enum Backend {
    Embedded {
        conn: Connection,
        doc: Box<Document>,
        host: CliHost,
    },
    Live(Box<live::LiveSession>),
}

/// One long-lived MCP session. Embedded: a `Connection` hello'd and
/// attached to its own fresh document at construction, so the first
/// `tools/call` an agent makes already has a working document
/// (docs/HEW_API.md §13's intended loop: describe → plan → transact →
/// look). Live: hello'd against a discovered running instance instead —
/// the document is whatever the user already has open.
pub struct McpServer {
    backend: Backend,
    /// The protocol-1 command registry, kept independent of `backend`
    /// (a live session has no local `Connection` to borrow one from) —
    /// used only to shape the `tools/list` surface and the `hew_query`
    /// read-only gate; every actual command dispatch goes through
    /// `backend`, which is what answers with the real (possibly remote)
    /// authority.
    registry: Registry,
    /// The granted profile — `Core` embedded, whatever the remote's hello
    /// reported when live (docs/HEW_API.md §12: live is always `app`, but
    /// this reads it back rather than assuming).
    profile: Profile,
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
"#;

impl McpServer {
    /// Headless: `hello` (protocol 1) then `hew.doc.new`, both internal —
    /// `hew.doc.new` auto-attaches on success (docs/HEW_API.md §4.2), so no
    /// separate `attach` round-trip is needed either.
    pub fn new() -> McpServer {
        let profile = Profile::Core;
        let mut conn = Connection::new(profile, "hew-cli:mcp");
        let mut doc = Document::new();
        let mut host = CliHost::new();
        for (method, params) in [
            ("hew.meta.hello", json!({ "protocol": 1 })),
            ("hew.doc.new", json!({})),
        ] {
            let request = Request {
                jsonrpc: "2.0".to_string(),
                id: Some(RequestId::Text(format!("startup-{method}"))),
                method: method.to_string(),
                params: Some(params),
            };
            let DispatchOutcome::Reply(response) = conn.dispatch(&mut doc, &mut host, request)
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
                conn,
                doc: Box::new(doc),
                host,
            },
            registry: Registry::protocol_1(),
            profile,
        }
    }

    /// Live: discovers (and, with `opts.launch`, starts) a running desktop
    /// instance, hello's it with the discovery token, then attaches to its
    /// document (docs/HEW_API.md §11.2, §12). The attach step matters
    /// because a live host serves a document the user already has open
    /// rather than creating one on connect — unlike [`McpServer::new`]
    /// above, whose embedded `hew.doc.new` auto-attaches for free, a live
    /// session has no such call to piggyback on, so without this every
    /// forwarded tool call (`hew_transact`, `hew_query`,
    /// `hew_describe_scene`, `hew_snapshot`) would answer `-32002 no
    /// document attached` the moment an agent tried to use it
    /// ([`crate::run::dispatch_live`]'s doc comment has the fuller
    /// rationale — this is the same fix, applied here via
    /// [`crate::run::attach_live`]). The granted profile comes back from
    /// the remote's own hello reply.
    pub fn new_live(opts: &LiveOptions) -> Result<McpServer, live::LiveError> {
        let hello_request = live::build_hello_request("hew-cli:mcp");
        let mut session = live::connect_live(opts, hello_request)?;
        if let Err(msg) = crate::run::attach_live(&mut session) {
            return Err(live::LiveError::Protocol(msg));
        }
        let profile = session.granted_profile();
        Ok(McpServer {
            backend: Backend::Live(Box::new(session)),
            registry: Registry::protocol_1(),
            profile,
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
                "tools": generate_tools(&self.registry, self.profile)
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
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let response = match name {
            "hew_capabilities" => self.dispatch_tool("hew.meta.capabilities", json!({}))?,
            "hew_transact" => self.dispatch_tool("hew.doc.transact", arguments)?,
            "hew_describe_scene" => self.dispatch_tool("hew.query.scene", json!({}))?,
            "hew_snapshot" => self.dispatch_tool("hew.view.snapshot", arguments)?,
            "hew_print_pdf" => self.dispatch_tool("hew.print.pdf", arguments)?,
            "hew_line_drawing" => self.dispatch_tool("hew.view.line_drawing", arguments)?,
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
                self.dispatch_tool(method, sub_params)?
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
    /// (docs/HEW_API.md §12: "every envelope the embedded path would
    /// dispatch locally is instead written to the socket") — and returns
    /// the whole JSON-RPC response (result or refusal) as a `Value`. An MCP
    /// tool call always succeeds at the JSON-RPC layer even when the
    /// underlying command refuses; the refusal is the answer, forwarded
    /// verbatim in live mode (§13).
    fn dispatch_tool(&mut self, method: &str, params: Value) -> Result<Value, JsonRpcErr> {
        // A live host can serialize a document or encode a mesh but has no
        // disk to put the result on, so it refuses a `path` and hands the
        // bytes back instead. Strip the path here and write the file after
        // the reply, exactly as the script runner and one-shot dispatch do
        // — otherwise the canonical MCP shape for a solitary command (§6.4:
        // a one-command transaction) would be the ONE live entry point
        // where asking to save somewhere just refuses.
        let (params, write_path) = match &self.backend {
            Backend::Live(_) => crate::run::take_client_write_path(method, params),
            Backend::Embedded { .. } => (params, None),
        };
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("mcp".to_string())),
            method: method.to_string(),
            params: Some(params),
        };
        let outcome = match &mut self.backend {
            Backend::Embedded { conn, doc, host } => conn.dispatch(doc, host, request),
            Backend::Live(session) => session.dispatch(request).map_err(|e| JsonRpcErr {
                code: -32603,
                message: format!("live transport error: {e}"),
            })?,
        };
        let DispatchOutcome::Reply(response) = outcome else {
            return Err(JsonRpcErr {
                code: -32603,
                message: "internal error: envelope dropped unexpectedly".to_string(),
            });
        };
        let value = serde_json::to_value(&response).expect("Response serializes");
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

/// The MCP tool inventory (docs/HEW_API.md §13): a small set of chunky
/// tools generated from the registry rather than one tool per command.
/// `hew_snapshot` is included whenever the connection's profile grants
/// `hew.view.snapshot` — true for both profiles today, since core grants
/// that one `hew.view.*` command specifically (it renders through a
/// headless software rasterizer, no viewport required — see
/// docs/design/headless-snapshot.md); a future app-only `hew.view.*`
/// addition would be the next tool a headless session lacks (an agent
/// never sees a tool it cannot call).
pub fn generate_tools(registry: &Registry, profile: Profile) -> Vec<Value> {
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
            "description": "Run hew.* commands in order as one undo entry. Every edit goes through this. Put a draw and the extrude that consumes it in the SAME call, labeling the draw with \"as\" and referencing its result: {\"$ref\": \"rect#/region_id\"} for an id, {\"$face\": \"circle#face\"} for a face it just cut. If any command refuses, none of them applied.",
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
            "description": "Run one read-only command (hew.query.*, hew.meta.*, hew.attr.get) — measuring, resolving an id, listing faces, raycasting. Nothing here changes the document. Use hew_transact for anything that does.",
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
        .map(|c| profile.grants(c))
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
        .map(|c| profile.grants(c))
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
        .map(|c| profile.grants(c))
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
        let tools = generate_tools(&registry, Profile::Core);
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

    #[test]
    fn app_profile_tool_list_includes_snapshot() {
        let registry = Registry::protocol_1();
        let tools = generate_tools(&registry, Profile::App);
        assert!(tools.iter().any(|t| t["name"] == "hew_snapshot"));
    }

    /// `path` is a passthrough parameter (docs/HEW_API.md §7): the tool
    /// schema must expose it or an MCP client has no way to ask for it —
    /// this is the whole fix for the inline-PNG size problem, so its
    /// absence here would silently undo it.
    #[test]
    fn hew_snapshot_tool_schema_exposes_path() {
        let registry = Registry::protocol_1();
        let tools = generate_tools(&registry, Profile::Core);
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
        for profile in [Profile::Core, Profile::App] {
            let registry = Registry::protocol_1();
            for tool in generate_tools(&registry, profile) {
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
