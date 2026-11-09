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
    Connection, DispatchOutcome, DocFormat, Grant, Host, NoHost, Refusal, Request, RequestId,
    Response, SaveOutcome, SnapshotParams, SnapshotResult, codes,
};
use kernel::{Document, Plane, Point3};
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// --------------------------------------------------------------- fixtures

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// One document-level edit — a fresh box, one undo entry (Lane C: the
/// sketch's creation folds into `extrude_region`'s own entry, matching
/// `kernel/tests/saved_mark_specs.rs`'s `build_box`) — for the
/// `mark_saved`/`at_saved_mark` coverage below, which needs a document
/// that has actually moved off the clean-at-depth-0 default.
fn edit_the_document(doc: &mut Document) {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let corners = [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ];
        for (a, b) in corners {
            sk.add_segment(a, b).expect("rectangle segment");
        }
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("live");
    doc.extrude_region(s, regions[0], 0.5).expect("extrude box");
}

// ------------------------------------------------------------- test host

/// A host that implements every effect in memory: no filesystem, no
/// renderer, just enough behavior to prove `commands/doc.rs` parses
/// params and shapes results correctly. Not a stand-in for `hew-cli`'s
/// real `CliHost` (crates/hew-cli owns that) — purely a test double.
#[derive(Default)]
struct FakeHost {
    /// The last path `save_document` was called with.
    last_save_path: Option<String>,
    /// The `format` `save_document` was last called with.
    last_save_format: Option<DocFormat>,
    /// The `units` the last `save_document`/`export_document` was given.
    last_save_units: Option<String>,
    last_export_units: Option<String>,
    /// The `compat` the last `save_document` / `export_document` was given.
    last_save_compat: bool,
    last_export_compat: bool,
    /// If set, a save to a path returns this instead of a bare write.
    save_outcome: Option<SaveOutcome>,
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
        format: Option<DocFormat>,
        units: Option<&str>,
        compat: bool,
    ) -> Result<SaveOutcome, Refusal> {
        self.last_save_compat = compat;
        self.last_save_format = format;
        self.last_save_units = units.map(str::to_string);
        match path {
            Some(p) => {
                self.last_save_path = Some(p.to_string());
                // A host that writes the file itself returns no bytes.
                Ok(match self.save_outcome.take() {
                    Some(o) => o,
                    None => SaveOutcome::written(),
                })
            }
            None => Err(Refusal::api("path_required", "no working path yet")),
        }
    }

    fn export_document(
        &mut self,
        doc: &Document,
        format: &str,
        _path: Option<&str>,
        segments_per_turn: u32,
        units: Option<&str>,
        selection: Option<&[api::locate::RenderRoot]>,
        compat: bool,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        self.last_export_compat = compat;
        self.last_export_units = units.map(str::to_string);
        // A selection exports through the real writers, so its tests see
        // real bytes.
        if let Some(roots) = selection {
            return mesh_export::export_occurrences(doc, roots, format, segments_per_turn, true)
                .map(Some)
                .map_err(|e| Refusal::api(e.name(), &e.message()));
        }
        self.export_result
            .take()
            .unwrap_or_else(|| Ok(Some(vec![1, 2, 3])))
    }

    fn import_document(
        &mut self,
        _doc: &mut Document,
        _path: &str,
        _options: &api::ImportOptions,
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
                depth: None,
                normals_png: None,
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
            depth: None,
            normals_png: None,
        })
    }
}

// ----------------------------------------------------------------- fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
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
    let DispatchOutcome::Reply(r) =
        conn.dispatch(&REGISTRY, doc, host, req(1, "hew.doc.attach", json!({})))
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
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, host, req(id, method, params))
    else {
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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

/// `format` reaches the host as the typed enum, and these wire names
/// are the only ones the command takes.
#[test]
fn save_forwards_each_format_to_the_host_as_the_enum() {
    let mut formats = vec![("hew".to_string(), DocFormat::Hew)];
    formats.push(("skp".to_string(), DocFormat::Skp));
    for &release in api::SKP_STREAM_RELEASES {
        let year = api::skp_stream_year(release);
        formats.push((format!("skp{year}"), DocFormat::SkpStream(release)));
    }
    // Every release has its own wire name, 3 to 8 and 2013 to 2020.
    assert_eq!(formats.len(), 16);
    for (wire, expected) in formats {
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = Document::new();
        let mut host = FakeHost::default();
        hello_attach(&mut conn, &mut doc, &mut host);
        call_ok(
            &mut conn,
            &mut doc,
            &mut host,
            2,
            "hew.doc.save",
            json!({ "path": "out.x", "format": wire }),
        );
        assert_eq!(host.last_save_format, Some(expected), "{wire}");
    }
}

/// The wire names are exactly `skp`, `skp3` to `skp8` and `skp2013` to
/// `skp2020`; a name that is not one of them is no format, and each
/// format names itself back.
#[test]
fn doc_format_wire_names_are_exact() {
    for bad in [
        "skp2021", "skp9", "skp12", "skp13", "skp20", "skp17", "skp03", "skp2017 ", "SKP", "skp0",
        "skp2012", "",
    ] {
        assert_eq!(DocFormat::from_name(bad), None, "{bad:?}");
    }
    for name in DocFormat::wire_names() {
        let f = DocFormat::from_name(&name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(f.as_str(), name);
    }
    // The 2026 container first, then newest to oldest.
    let skp = DocFormat::skp_wire_names();
    assert_eq!(skp.first().map(String::as_str), Some("skp"));
    assert_eq!(skp.get(1).map(String::as_str), Some("skp2020"));
    assert_eq!(skp.last().map(String::as_str), Some("skp3"));
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.skp", "format": "skp2021" }),
    );
    assert!(r.error.is_some(), "skp2021 is no format");
}

#[test]
fn save_without_a_format_leaves_the_choice_to_the_host() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.skp" }),
    );
    assert_eq!(host.last_save_format, None);
}

#[test]
fn save_and_export_hand_named_units_to_the_host() {
    // A SketchUp file's display units, by hew.view.units's names; absent,
    // the host decides (None reaches it).
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.skp", "units": "mm" }),
    );
    assert_eq!(host.last_save_units.as_deref(), Some("mm"));
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.doc.save",
        json!({ "path": "out.skp" }),
    );
    assert_eq!(host.last_save_units, None);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.doc.export",
        json!({ "format": "skp2017", "units": "arch" }),
    );
    assert_eq!(host.last_export_units.as_deref(), Some("arch"));
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        5,
        "hew.doc.export",
        json!({ "format": "skp" }),
    );
    assert_eq!(host.last_export_units, None);
}

#[test]
fn compat_reaches_the_host_for_the_2021_and_later_container_and_is_refused_elsewhere() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.skp", "format": "skp", "compat": true }),
    );
    assert!(host.last_save_compat);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.doc.save",
        json!({ "path": "out.skp", "format": "skp" }),
    );
    assert!(!host.last_save_compat, "absent means by content");
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.doc.export",
        json!({ "format": "skp", "compat": true }),
    );
    assert!(host.last_export_compat);
    for (id, method, params) in [
        (
            5,
            "hew.doc.save",
            json!({ "path": "out.skp", "format": "skp2017", "compat": true }),
        ),
        (
            6,
            "hew.doc.save",
            json!({ "path": "out.hew", "format": "hew", "compat": true }),
        ),
        (
            7,
            "hew.doc.export",
            json!({ "format": "skp2017", "compat": true }),
        ),
        (
            8,
            "hew.doc.export",
            json!({ "format": "stl", "compat": true }),
        ),
    ] {
        let r = call(&mut conn, &mut doc, &mut host, id, method, params.clone());
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS, "{params}");
    }
}

#[test]
fn units_outside_the_length_formats_or_off_a_skp_export_are_refused_before_the_host() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    for (id, method, params) in [
        (
            2,
            "hew.doc.save",
            json!({ "path": "out.skp", "units": "furlongs" }),
        ),
        (
            3,
            "hew.doc.export",
            json!({ "format": "skp", "units": "in" }),
        ),
        (
            4,
            "hew.doc.export",
            json!({ "format": "stl", "units": "mm" }),
        ),
    ] {
        let r = call(&mut conn, &mut doc, &mut host, id, method, params.clone());
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS, "{params}");
    }
    assert_eq!(host.last_save_units, None);
    assert_eq!(host.last_export_units, None);
}

#[test]
fn save_refuses_an_unknown_format_before_the_host_hears_of_it() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.x", "format": "obj" }),
    );
    assert!(r.error.is_some(), "an unknown format is a params error");
    assert_eq!(host.last_save_path, None, "the host was never called");
}

/// A `.skp` save reports what the file could not hold, under
/// `simplified` / `writer_notes`, next to the format it resolved.
#[test]
fn save_reports_simplified_and_writer_notes_for_a_skp() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        save_outcome: Some(SaveOutcome {
            format: Some(DocFormat::SkpStream(17)),
            simplified: vec!["curved faces".to_string()],
            writer_notes: vec!["dropped scenes".to_string()],
            ..SaveOutcome::default()
        }),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.skp", "format": "skp2017" }),
    );
    assert_eq!(
        result,
        json!({
            "format": "skp2017",
            "simplified": ["curved faces"],
            "writer_notes": ["dropped scenes"],
        })
    );
}

/// A `.hew` save reports its format but no SketchUp-only keys.
#[test]
fn save_of_a_hew_reports_no_skp_keys() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        save_outcome: Some(SaveOutcome {
            format: Some(DocFormat::Hew),
            ..SaveOutcome::default()
        }),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.hew" }),
    );
    assert_eq!(result, json!({ "format": "hew" }));
}

/// Lane C's saved mark: a host that writes the file itself (`CliHost`'s
/// shape — `save_document` returns `None`) has completed the save the
/// instant the command returns `Ok`, so `hew.doc.save` marks the document
/// clean right there.
#[test]
fn save_marks_the_document_clean_when_the_host_writes_it_itself() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    assert!(!doc.at_saved_mark(), "an edit was just made");
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.save",
        json!({ "path": "out.hew" }),
    );
    assert!(
        doc.at_saved_mark(),
        "the host's own write completed the save"
    );
}

/// The `LiveHost` shape — `save_document` returns bytes rather than
/// writing them — has NOT completed the save when the command returns:
/// the caller still has to write those bytes somewhere. Marking the
/// document clean here would be a lie the app could never see through;
/// the live app calls `Scene::mark_saved` itself once its own write
/// succeeds (crates/wasm-api).
#[test]
fn save_leaves_the_document_dirty_when_the_host_only_returns_bytes() {
    struct BytesOnlyHost;
    impl Host for BytesOnlyHost {
        fn save_document(
            &mut self,
            doc: &Document,
            _path: Option<&str>,
            _format: Option<DocFormat>,
            _units: Option<&str>,
            _compat: bool,
        ) -> Result<SaveOutcome, Refusal> {
            Ok(SaveOutcome::with_bytes(doc.save_for_persistence()))
        }
    }

    let mut conn = Connection::new(Grant::app(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    assert!(!doc.at_saved_mark(), "an edit was just made");
    let mut host = BytesOnlyHost;
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(&mut conn, &mut doc, &mut host, 2, "hew.doc.save", json!({}));
    assert!(
        !doc.at_saved_mark(),
        "the bytes haven't been written anywhere yet"
    );
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
            _format: Option<DocFormat>,
            _units: Option<&str>,
            _compat: bool,
        ) -> Result<SaveOutcome, Refusal> {
            assert!(path.is_none(), "the caller strips the path for such a host");
            Ok(SaveOutcome::with_bytes(doc.save_for_persistence()))
        }
    }

    let mut conn = Connection::new(Grant::app(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(&mut conn, &mut doc, &mut host, 2, "hew.doc.save", json!({}));
    assert_eq!(data["refusal"], "path_required");
}

#[test]
fn export_with_no_path_returns_bytes_base64() {
    let mut conn = Connection::new(Grant::core(), "test");
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

/// The SketchUp formats are export formats like any other: the command
/// forwards the name and echoes it back.
#[test]
fn export_takes_the_skp_formats() {
    for format in ["skp", "skp2017", "skp2018", "skp2019", "skp2020"] {
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = Document::new();
        let mut host = FakeHost::default();
        hello_attach(&mut conn, &mut doc, &mut host);
        let result = call_ok(
            &mut conn,
            &mut doc,
            &mut host,
            2,
            "hew.doc.export",
            json!({ "format": format }),
        );
        assert_eq!(result["bytes_base64"], "AQID");
        assert_eq!(result["format"], format);
    }
}

#[test]
fn export_with_a_path_the_host_wrote_returns_no_bytes() {
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        snapshot_result: Some(Ok(SnapshotResult {
            png: vec![9, 9, 9],
            width: 64,
            height: 64,
            id_buffer: Some(vec![0, 0, 1, 0]),
            id_palette: vec!["obj_1".to_string()],
            depth: None,
            normals_png: None,
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let mut host = FakeHost {
        snapshot_result: Some(Ok(SnapshotResult {
            png: vec![9, 9, 9],
            width: 64,
            height: 64,
            id_buffer: Some(vec![0, 0, 1, 0]),
            id_palette: vec!["obj_1".to_string()],
            depth: None,
            normals_png: None,
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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

// ------------------------------------------------------------ hew.doc.revert

/// A host with a saved file whose `open_document` loads a clean, empty
/// document — what `hew-cli`'s `CliHost` does with the file on disk.
struct SavedFileHost {
    path: Option<String>,
    reloaded: bool,
}

impl Host for SavedFileHost {
    fn working_document_path(&self) -> Option<&str> {
        self.path.as_deref()
    }
    fn open_document(&mut self, doc: &mut Document, path: &str) -> Result<(), Refusal> {
        assert_eq!(Some(path), self.path.as_deref(), "reloads the saved file");
        *doc = Document::new();
        self.reloaded = true;
        Ok(())
    }
}

#[test]
fn revert_without_a_saved_file_refuses_nothing_to_revert() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    let mut host = SavedFileHost {
        path: None,
        reloaded: false,
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.revert",
        json!({}),
    );
    assert_eq!(data["refusal"], "nothing_to_revert");
    assert!(!host.reloaded);
    assert!(!doc.at_saved_mark(), "a refusal leaves the document alone");
}

#[test]
fn revert_of_an_unchanged_document_refuses_nothing_to_revert() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    doc.mark_saved();
    let mut host = SavedFileHost {
        path: Some("a.hew".into()),
        reloaded: false,
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.revert",
        json!({}),
    );
    assert_eq!(data["refusal"], "nothing_to_revert");
    assert!(!host.reloaded, "a clean document is not re-read");
}

#[test]
fn revert_of_a_changed_saved_document_reloads_it_clean_with_no_history() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    assert!(doc.can_undo());
    let mut host = SavedFileHost {
        path: Some("a.hew".into()),
        reloaded: false,
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.revert",
        json!({}),
    );
    assert!(host.reloaded);
    assert!(doc.at_saved_mark());
    assert!(!doc.can_undo() && !doc.can_redo());
}

#[test]
fn revert_refuses_load_failed_and_leaves_the_document_unchanged() {
    struct GoneFileHost;
    impl Host for GoneFileHost {
        fn working_document_path(&self) -> Option<&str> {
            Some("a.hew")
        }
        fn open_document(&mut self, _doc: &mut Document, _path: &str) -> Result<(), Refusal> {
            Err(Refusal::api("load_failed", "no such file"))
        }
    }
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    let depth = doc.undo_depth();
    let mut host = GoneFileHost;
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.revert",
        json!({}),
    );
    assert_eq!(data["refusal"], "load_failed");
    assert!(!doc.at_saved_mark(), "the document is still changed");
    assert_eq!(doc.undo_depth(), depth, "its history is untouched");
    assert!(doc.can_undo());
}

#[test]
fn revert_refuses_host_capability_missing_on_a_host_that_cannot_open() {
    struct PathOnlyHost;
    impl Host for PathOnlyHost {
        fn working_document_path(&self) -> Option<&str> {
            Some("a.hew")
        }
    }
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    edit_the_document(&mut doc);
    let mut host = PathOnlyHost;
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.revert",
        json!({}),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

// ------------------------------------------------- selection-scoped export

/// Two 1 m boxes side by side, built straight on the kernel.
fn two_boxes(doc: &mut Document) -> (String, String) {
    let mut ids = Vec::new();
    for x0 in [0.0, 3.0] {
        let s = doc.add_sketch(ground());
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let c = [
            Point3::new(x0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 1.0, 0.0),
            Point3::new(x0, 1.0, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).expect("segment");
        }
        let regions = doc.extrudable_regions(s).expect("live");
        let (o, _) = doc.extrude_region(s, regions[0], 1.0).expect("extrude");
        let e = kernel::EntityRef::Object(o);
        ids.push(api::ids::public_id(&e, doc.sid_of(&e).expect("sid")));
    }
    (ids[0].clone(), ids[1].clone())
}

fn stl_triangles(bytes_base64: &str) -> u32 {
    let bytes = decode_base64(bytes_base64);
    u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]])
}

/// Standard base64 decode, enough for the tests' own STL payloads.
fn decode_base64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let mut out = Vec::new();
    for chunk in s.as_bytes().chunks(4) {
        let n = chunk.iter().fold(0u32, |acc, &c| (acc << 6) | val(c));
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let b = n.to_be_bytes();
        out.extend_from_slice(&b[1..4 - pad]);
    }
    out
}

#[test]
fn an_stl_export_of_one_of_two_boxes_has_twelve_triangles() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let (a, _b) = two_boxes(&mut doc);
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let one = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.export",
        json!({ "format": "stl", "selection": [a] }),
    );
    assert_eq!(stl_triangles(one["bytes_base64"].as_str().unwrap()), 12);
    // The occurrence-locator form names the same box.
    let by_path = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.doc.export",
        json!({ "format": "stl", "selection": [{ "path": [a] }] }),
    );
    assert_eq!(by_path["bytes_base64"], one["bytes_base64"]);
}

#[test]
fn an_export_selection_refuses_empty_unknown_and_sketchup_formats() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let (a, _) = two_boxes(&mut doc);
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let empty = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.doc.export",
        json!({ "format": "stl", "selection": [] }),
    );
    assert_eq!(empty.error.unwrap().code, codes::INVALID_PARAMS);
    let unknown = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.doc.export",
        json!({ "format": "stl", "selection": ["obj_ffff"] }),
    );
    assert_eq!(unknown["refusal"], "unknown_entity");
    let skp = call(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.doc.export",
        json!({ "format": "skp", "selection": [a] }),
    );
    assert_eq!(skp.error.unwrap().code, codes::INVALID_PARAMS);
}

/// A host that records the line-drawing params it was handed.
#[derive(Default)]
struct DrawingHost {
    last: Option<api::LineDrawingParams>,
}

impl Host for DrawingHost {
    fn line_drawing(
        &mut self,
        _doc: &Document,
        params: &api::LineDrawingParams,
    ) -> Result<api::LineDrawingResult, Refusal> {
        self.last = Some(params.clone());
        Ok(api::LineDrawingResult {
            svg: None,
            segments: Vec::new(),
            kinds: Vec::new(),
            ids: Vec::new(),
            bounds: None,
            count: 0,
        })
    }
}

#[test]
fn a_line_drawing_selection_reaches_the_host_resolved() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let (_, b) = two_boxes(&mut doc);
    let mut host = DrawingHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.line_drawing",
        json!({ "format": "segments", "selection": [b] }),
    );
    let sel = host.last.take().unwrap().selection.expect("a selection");
    assert_eq!(sel.len(), 1);
    let api::locate::RenderRoot::Occurrence(occ) = &sel[0] else {
        panic!("a world node resolves to its occurrence");
    };
    assert!(occ.path.is_empty());
    let whole = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.view.line_drawing",
        json!({ "format": "segments" }),
    );
    assert_eq!(whole["count"], 0);
    assert!(host.last.take().unwrap().selection.is_none());
}

// ------------------------------------------- snapshot selection, background

#[test]
fn a_snapshot_selection_reaches_the_host_resolved_with_the_default_background() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let (a, _) = two_boxes(&mut doc);
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "selection": [a], "background": "transparent" }),
    );
    let p = host.last_snapshot_params.take().expect("host was called");
    assert_eq!(p.selection.expect("a selection").len(), 1);
    assert_eq!(p.background, [0, 0, 0, 0]);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.view.snapshot",
        json!({}),
    );
    let p = host.last_snapshot_params.take().expect("host was called");
    assert!(p.selection.is_none());
    assert_eq!(
        p.background,
        api::commands::doc::DEFAULT_SNAPSHOT_BACKGROUND
    );
}

#[test]
fn a_snapshot_selection_accepts_a_definition_id() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let obj = {
        edit_the_document(&mut doc);
        doc.visible_object_ids()[0]
    };
    let (component, _, _) = doc
        .make_component(&[kernel::NodeId::Object(obj)])
        .expect("fold");
    let e = kernel::EntityRef::Component(component);
    let cmp = api::ids::public_id(&e, doc.sid_of(&e).expect("sid"));
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.snapshot",
        json!({ "selection": [cmp] }),
    );
    let sel = host
        .last_snapshot_params
        .take()
        .and_then(|p| p.selection)
        .expect("a selection");
    assert_eq!(sel, vec![api::locate::RenderRoot::Definition(component)]);
}

#[test]
fn snapshot_validation_rows_answer_invalid_params_with_their_messages() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let (a, _) = two_boxes(&mut doc);
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let scene_id = "scn_0";
    let cases = [
        (
            json!({ "selection": [a], "scene": scene_id }),
            "selection is mutually exclusive with scene",
        ),
        (
            json!({ "selection": [] }),
            "selection must name at least one entity",
        ),
        (
            json!({ "background": "#fff" }),
            "background is \"transparent\" or a #rrggbb or #rrggbbaa color",
        ),
        (
            json!({ "background": "red" }),
            "background is \"transparent\" or a #rrggbb or #rrggbbaa color",
        ),
    ];
    for (i, (params, message)) in cases.into_iter().enumerate() {
        let r = call(
            &mut conn,
            &mut doc,
            &mut host,
            2 + i as i64,
            "hew.view.snapshot",
            params,
        );
        let err = r.error.expect("refused");
        assert_eq!(err.code, codes::INVALID_PARAMS);
        assert!(err.message.contains(message), "{}", err.message);
    }
    assert!(host.last_snapshot_params.is_none(), "no host call");
    // An empty selection is refused the same way on a line drawing.
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        20,
        "hew.view.line_drawing",
        json!({ "selection": [] }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn selection_rows_check_exclusivity_first_and_refuse_sketch_and_material_ids() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    let (a, _) = two_boxes(&mut doc);
    let sketch = doc.add_sketch(ground());
    let sketch_id = {
        let e = kernel::EntityRef::Sketch(sketch);
        api::ids::public_id(&e, doc.sid_of(&e).expect("sid"))
    };
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let check = |conn: &mut Connection,
                 doc: &mut Document,
                 host: &mut FakeHost,
                 id,
                 method,
                 params: Value,
                 message: &str| {
        let err = call(conn, doc, host, id, method, params)
            .error
            .expect("refused");
        assert_eq!(err.code, codes::INVALID_PARAMS);
        assert!(err.message.contains(message), "{}", err.message);
    };
    // Exclusivity is reported before emptiness, on both commands.
    for (i, method) in ["hew.view.snapshot", "hew.view.line_drawing"]
        .into_iter()
        .enumerate()
    {
        check(
            &mut conn,
            &mut doc,
            &mut host,
            2 + i as i64,
            method,
            json!({ "selection": [], "scene": "scn_0" }),
            "mutually exclusive with scene",
        );
        check(
            &mut conn,
            &mut doc,
            &mut host,
            10 + i as i64,
            method,
            json!({ "selection": [sketch_id.clone()] }),
            "not a sketch or material",
        );
    }
    let _ = a;
}
