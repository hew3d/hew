//! Conformance coverage for `hew.doc.new/open/save/export/import` and
//! `hew.view.snapshot` (docs/design/api-implementation-conventions.md's
//! testing bar). Every one of these six commands is `Served::Host`
//! (registry.rs): `crates/api` owns envelope validation, profile
//! enforcement, and response shaping, and delegates the actual effect to
//! `ctx.host`. So the coverage here is two-layered:
//!
//! - **`NoHost` refusals** — every command answers `host_capability_missing`
//!   in the canonical §4.4 shape when the host implements nothing (the
//!   scaffold's default posture, and the literal contract a "headless
//!   build with no CLI" would give an agent).
//! - **A `FakeHost`** exercising the success path — this crate has no real
//!   file I/O (kernel-class purity, DEVELOPMENT.md rule 1), so a minimal
//!   in-memory test double stands in for `hew-cli`'s real one and proves
//!   the params-parsing / result-shaping contract: base64 encoding,
//!   `{report}` wrapping, the bytes-vs-path branch of `export`.
//!
//! All six commands are `CommandClass::Solitary` (registry.rs): they run
//! bare, outside any transaction bracket, so the one-envelope-one-undo
//! property this suite otherwise checks does not apply to them — asserted
//! directly below instead of gated as a burn-down property.

use api::{
    Connection, DispatchOutcome, Host, NoHost, Profile, Refusal, Request, RequestId, Response,
    SnapshotParams, SnapshotResult, codes,
};
use kernel::Document;
use serde_json::{Value, json};

// ------------------------------------------------------------- test host

/// A host that implements every effect in memory: no filesystem, no
/// renderer, just enough behavior to prove `commands/doc.rs` parses
/// params and shapes results correctly. Not a stand-in for `hew-cli`'s
/// real `CliHost` (crates/hew-cli owns that) — purely a test double.
#[derive(Default)]
struct FakeHost {
    /// The last path `save_document` was called with.
    last_save_path: Option<String>,
    /// If set, `export_document` returns this instead of its default.
    export_result: Option<Result<Option<Vec<u8>>, Refusal>>,
    /// If set, `import_document` returns this instead of its default.
    import_result: Option<Result<Value, Refusal>>,
    /// If set, `snapshot` returns this instead of its default.
    snapshot_result: Option<Result<SnapshotResult, Refusal>>,
    /// The params `snapshot` was last called with — lets tests assert on
    /// what `commands/doc.rs` resolved (clamped size, validated camera)
    /// before it ever reached the host.
    last_snapshot_params: Option<SnapshotParams>,
    /// Every `(path, bytes)` pair `write_snapshot` was called with, in
    /// call order — `commands/doc.rs` calls this once for the PNG and,
    /// when `include_ids` and `path` are both given, a second time for
    /// the id-buffer sidecar.
    snapshot_writes: Vec<(String, Vec<u8>)>,
    /// If set, `write_snapshot` returns this instead of its default `Ok`.
    write_snapshot_result: Option<Result<(), Refusal>>,
}

impl Host for FakeHost {
    fn new_document(&mut self, doc: &mut Document) -> Result<(), Refusal> {
        *doc = Document::new();
        Ok(())
    }

    fn open_document(&mut self, doc: &mut Document, path: &str) -> Result<(), Refusal> {
        if path == "missing.hew" {
            return Err(Refusal::api("load_failed", "no such file"));
        }
        *doc = Document::new();
        Ok(())
    }

    fn save_document(
        &mut self,
        _doc: &Document,
        path: Option<&str>,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        match path {
            Some(p) => {
                self.last_save_path = Some(p.to_string());
                // A host that writes the file itself returns nothing.
                Ok(None)
            }
            None => Err(Refusal::api("path_required", "no working path yet")),
        }
    }

    fn export_document(
        &mut self,
        _doc: &Document,
        _format: &str,
        _path: Option<&str>,
        _segments_per_turn: u32,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        self.export_result
            .take()
            .unwrap_or_else(|| Ok(Some(vec![1, 2, 3])))
    }

    fn import_document(
        &mut self,
        _doc: &mut Document,
        _path: &str,
        _options: &Value,
    ) -> Result<Value, Refusal> {
        self.import_result
            .take()
            .unwrap_or_else(|| Ok(json!({ "objects_created": 1, "watertight": 1, "leaky": 0, "skipped": [], "textures_missing": [] })))
    }

    fn snapshot(
        &mut self,
        _doc: &Document,
        params: &SnapshotParams,
    ) -> Result<SnapshotResult, Refusal> {
        self.last_snapshot_params = Some(params.clone());
        self.snapshot_result.take().unwrap_or_else(|| {
            Ok(SnapshotResult {
                png: vec![9, 9, 9],
                width: params.width,
                height: params.height,
                id_buffer: None,
                id_palette: Vec::new(),
            })
        })
    }

    fn write_snapshot(&mut self, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        self.snapshot_writes
            .push((path.to_string(), bytes.to_vec()));
        self.write_snapshot_result.take().unwrap_or(Ok(()))
    }
}

/// A host that can render but, like `NoHost`, was never given filesystem
/// access — `write_snapshot` falls through to `Host`'s own refusing
/// default. Proves `host_capability_missing` comes from `write_snapshot`
/// itself (not merely re-surfaced from `snapshot`) when a `path` is given
/// to a host that renders fine but cannot write.
#[derive(Default)]
struct RendersButCannotWriteHost;

impl Host for RendersButCannotWriteHost {
    fn snapshot(
        &mut self,
        _doc: &Document,
        params: &SnapshotParams,
    ) -> Result<SnapshotResult, Refusal> {
        Ok(SnapshotResult {
            png: vec![9, 9, 9],
            width: params.width,
            height: params.height,
            id_buffer: None,
            id_palette: Vec::new(),
        })
    }
}

// ----------------------------------------------------------------- fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        host,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
}

/// `hello` + `attach` — everything but `hew.doc.new`/`open` needs an
/// attached document (§4.2).
fn hello_attach(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) {
    hello(conn, doc, host);
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, host, req(1, "hew.doc.attach", json!({})))
    else {
        panic!("attach replies")
    };
    assert!(r.error.is_none(), "attach failed: {:?}", r.error);
}

fn call(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, host, req(id, method, params)) else {
        panic!("{method} replies")
    };
    r
}

fn call_ok(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, id, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    r.result.expect("a successful reply carries a result")
}

/// Dispatches a command expected to refuse, returning the canonical §4.4
/// `error.data` payload.
fn call_err(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, id, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(
        err.code,
        codes::REFUSED,
        "{method}'s failure should be a typed refusal"
    );
    err.data.expect("a refusal carries the canonical §4.4 data")
}

// ---------------------------------------------------- host_capability_missing

#[test]
fn new_refuses_host_capability_missing_against_nohost() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.new",
        json!({}),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
    assert_eq!(data["detail"]["capability"], "create documents");
}

#[test]
fn open_refuses_host_capability_missing_against_nohost() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.open",
        json!({ "path": "model.hew" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn save_refuses_host_capability_missing_against_nohost() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.save",
        json!({}),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn export_refuses_host_capability_missing_against_nohost() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.export",
        json!({ "format": "stl" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn import_refuses_host_capability_missing_against_nohost() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.import",
        json!({ "path": "model.stl", "units": "m" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn snapshot_refuses_host_capability_missing_against_nohost() {
    // Core now grants hew.view.snapshot specifically (it has a headless
    // render path — docs/design/headless-snapshot.md), so a Core
    // connection reaches the host instead of being turned away at the
    // profile gate; a host with no renderer still refuses typed.
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.view.snapshot",
        json!({}),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

// --------------------------------------------------------------- param errors

#[test]
fn new_rejects_unknown_params() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    let r = call(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.new",
        json!({ "surprise": true }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn open_requires_a_path() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    let r = call(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.open",
        json!({}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn export_requires_a_format() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let r = call(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.export",
        json!({}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn import_requires_a_path() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let r = call(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.import",
        json!({}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

// ------------------------------------------------------------- success paths

#[test]
fn new_replaces_the_document_and_returns_the_empty_result() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello(&mut conn, &mut doc, &mut host);
    let depth_before = doc.undo_depth();
    let result = call_ok(&mut conn, &mut doc, &mut host, 1, "hew.doc.new", json!({}));
    assert_eq!(result, json!({}));
    assert_eq!(doc.undo_depth(), depth_before, "solitary — no undo entry");
}

#[test]
fn open_refuses_load_failed_from_the_host_verbatim() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        1,
        "hew.doc.open",
        json!({ "path": "missing.hew" }),
    );
    assert_eq!(data["refusal"], "load_failed");
}

#[test]
fn save_forwards_the_path_and_returns_the_empty_result() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.hew" }),
    );
    assert_eq!(result, json!({}));
    assert_eq!(host.last_save_path.as_deref(), Some("out.hew"));
}

/// A host with no filesystem of its own (the live WASM boundary) can
/// still serve a save: serializing the document needs no disk, only
/// WRITING the result does. Such a host returns the bytes and the caller
/// writes them — so the command answers `bytes_base64` rather than
/// refusing outright, which is what made `hew.doc.save` unusable live.
#[test]
fn save_returns_bytes_when_the_host_has_no_filesystem() {
    struct BytesOnlyHost;
    impl Host for BytesOnlyHost {
        fn save_document(
            &mut self,
            doc: &Document,
            path: Option<&str>,
        ) -> Result<Option<Vec<u8>>, Refusal> {
            assert!(path.is_none(), "the caller strips the path for such a host");
            Ok(Some(doc.save_for_persistence()))
        }
    }

    let mut conn = Connection::new(Profile::App, "test");
    let mut doc = Document::new();
    let mut host = BytesOnlyHost;
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(&mut conn, &mut doc, &mut host, 3, "hew.doc.save", json!({}));
    let b64 = result["results"][0]["bytes_base64"]
        .as_str()
        .or_else(|| result["bytes_base64"].as_str())
        .expect("a filesystem-less host hands the document bytes back");
    assert!(!b64.is_empty(), "the bytes must be the real document");
}

#[test]
fn save_without_a_path_and_no_prior_save_refuses_path_required() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(&mut conn, &mut doc, &mut host, 2, "hew.doc.save", json!({}));
    assert_eq!(data["refusal"], "path_required");
}

#[test]
fn export_with_no_path_returns_bytes_base64() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.export",
        json!({ "format": "stl" }),
    );
    // base64("\x01\x02\x03") — the RFC 4648 §10 vectors in doc.rs pin the
    // encoder itself; this pins the handler wires bytes through it.
    assert_eq!(result["bytes_base64"], "AQID");
    assert_eq!(result["format"], "stl");
}

#[test]
fn export_with_a_path_the_host_wrote_returns_no_bytes() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        export_result: Some(Ok(None)),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.export",
        json!({ "format": "stl", "path": "out.stl" }),
    );
    assert_eq!(result, json!({ "format": "stl" }));
}

#[test]
fn export_surfaces_nothing_to_export_from_the_host() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        export_result: Some(Err(Refusal::api(
            "nothing_to_export",
            "no watertight solids in this document",
        ))),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.export",
        json!({ "format": "stl" }),
    );
    assert_eq!(data["refusal"], "nothing_to_export");
}

#[test]
fn import_wraps_the_host_report() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.import",
        json!({ "path": "model.stl", "units": "m" }),
    );
    assert_eq!(result["report"]["objects_created"], 1);
}

#[test]
fn import_surfaces_units_required_from_the_host() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        import_result: Some(Err(Refusal::api(
            "units_required",
            "STL carries no units; pass units.",
        ))),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.import",
        json!({ "path": "model.stl" }),
    );
    assert_eq!(data["refusal"], "units_required");
}

#[test]
fn snapshot_returns_png_base64_with_default_size() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "view": "iso" }),
    );
    assert_eq!(result["png_base64"], "CQkJ");
    assert_eq!(result["width"], 512);
    assert_eq!(result["height"], 512);
    assert!(
        result.get("id_buffer_base64").is_none(),
        "include_ids defaults to false"
    );
    assert!(result.get("id_palette").is_none());
}

#[test]
fn snapshot_rejects_camera_and_view_together() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({
            "view": "iso",
            "camera": { "eye": [0.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0] }
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn snapshot_rejects_an_unknown_view_name() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "view": "diagonal" }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn snapshot_rejects_an_unknown_camera_projection() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({
            "camera": { "eye": [0.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "projection": "orthographic" }
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn snapshot_clamps_width_and_height_to_the_declared_bounds() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "width": 999_999, "height": 1 }),
    );
    let params = host
        .last_snapshot_params
        .expect("the host was called with the resolved params");
    assert_eq!(params.width, 2048, "width caps at 2048");
    assert_eq!(params.height, 16, "height floors at 16");
}

#[test]
fn snapshot_includes_id_buffer_and_palette_when_requested() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        snapshot_result: Some(Ok(SnapshotResult {
            png: vec![9, 9, 9],
            width: 64,
            height: 64,
            id_buffer: Some(vec![0, 0, 1, 0]),
            id_palette: vec!["obj_1".to_string()],
        })),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "include_ids": true }),
    );
    assert_eq!(result["id_palette"], json!(["obj_1"]));
    assert!(result["id_buffer_base64"].is_string());
}

#[test]
fn snapshot_with_path_writes_the_file_and_omits_inline_bytes() {
    // Mirrors hew.doc.export's own path posture (docs/agents/HEW_API.md §7): the
    // inline base64 PNG can exceed an MCP client's tool-result budget at
    // any useful resolution, so a caller that only needs the file on disk
    // asks for `path` instead.
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "view": "iso", "path": "/tmp/shot.png" }),
    );
    assert_eq!(result["path"], "/tmp/shot.png");
    assert_eq!(result["width"], 512);
    assert_eq!(result["height"], 512);
    assert!(
        result.get("png_base64").is_none(),
        "path given: no inline bytes"
    );
    assert!(result.get("id_buffer_path").is_none());
    assert!(result.get("id_palette").is_none());
    assert_eq!(
        host.snapshot_writes,
        vec![("/tmp/shot.png".to_string(), vec![9, 9, 9])],
        "the rendered PNG bytes were written to the given path"
    );
}

#[test]
fn snapshot_with_path_and_include_ids_writes_an_id_buffer_sidecar() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        snapshot_result: Some(Ok(SnapshotResult {
            png: vec![9, 9, 9],
            width: 64,
            height: 64,
            id_buffer: Some(vec![0, 0, 1, 0]),
            id_palette: vec!["obj_1".to_string()],
        })),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "include_ids": true, "path": "/tmp/shot.png" }),
    );
    assert_eq!(result["path"], "/tmp/shot.png");
    assert_eq!(result["id_buffer_path"], "/tmp/shot.png.ids.bin");
    assert_eq!(result["id_palette"], json!(["obj_1"]));
    assert!(result.get("png_base64").is_none());
    assert!(result.get("id_buffer_base64").is_none());
    assert_eq!(
        host.snapshot_writes,
        vec![
            ("/tmp/shot.png".to_string(), vec![9, 9, 9]),
            ("/tmp/shot.png.ids.bin".to_string(), vec![0, 0, 1, 0]),
        ],
        "the PNG and the id-buffer sidecar were each written once, in that order"
    );
}

#[test]
fn snapshot_write_failure_surfaces_as_the_hosts_refusal() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        write_snapshot_result: Some(Err(Refusal::api("save_failed", "disk is full"))),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "path": "/tmp/shot.png" }),
    );
    assert_eq!(data["refusal"], "save_failed");
}

#[test]
fn snapshot_with_path_refuses_host_capability_missing_against_nohost() {
    // NoHost's `snapshot` itself refuses before `write_snapshot` is ever
    // reached — the same refusal a bytes-only request gets.
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.view.snapshot",
        json!({ "path": "/tmp/shot.png" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn snapshot_with_path_refuses_host_capability_missing_when_the_host_cannot_write() {
    // A host that renders fine but was never given filesystem access (the
    // asymmetric case `write_snapshot`'s own refusing default exists for)
    // still refuses typed, distinctly from `snapshot`'s own refusal.
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = RendersButCannotWriteHost;
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "path": "/tmp/shot.png" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}
