//! `hew-cli` integration tests (docs/design/api-implementation-conventions.md's
//! testing bar, applied to a binary): calling the library functions
//! directly, spawn-free — no subprocess, no piped stdio. Covers `run`
//! (script + `--out`), `dispatch --file`, the two host effects with no
//! `crates/api`-level equivalent (real STL export and STL import), and a
//! real headless render of `hew.view.snapshot` through
//! `CliHost`/`crates/softrender` (docs/design/headless-snapshot.md).

use api::Host;
use hew_cli::host::CliHost;
use kernel::{Document, ImportNode, ImportScene, MeshRecipe, Point3};
use std::path::PathBuf;

/// A minimal RFC 4648 §4 base64 decoder — the inverse of
/// `crates/api/src/commands/doc.rs`'s hand-rolled encoder — so these
/// tests can inspect the actual PNG/id-buffer bytes a snapshot returns,
/// not just the base64 text.
fn base64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4 + 3);
    for chunk in bytes.chunks(4) {
        let n = chunk.iter().fold(0u32, |acc, &c| (acc << 6) | val(c));
        let n = n << (6 * (4 - chunk.len()));
        let produced = match chunk.len() {
            2 => 1,
            3 => 2,
            _ => 3,
        };
        for i in 0..produced {
            out.push(((n >> (16 - 8 * i)) & 0xFF) as u8);
        }
    }
    out
}

// ------------------------------------------------------------- fixtures

/// A fresh, process-unique scratch directory under `std::env::temp_dir()`
/// (never `/tmp` directly — this must work on any OS). Not removed by the
/// caller on purpose within one test process: parallel `#[test]` functions
/// each get their own directory, so nothing races.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hew-cli-test-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir creates");
    dir
}

/// A unit-cube mesh recipe, CCW-wound-from-outside faces — the same shape
/// `crates/kernel/src/build.rs`'s own test fixture uses.
fn unit_box_mesh(name: &str) -> MeshRecipe {
    let positions = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let face_count = faces.len();
    MeshRecipe {
        name: name.to_string(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; face_count],
        face_uv_frames: vec![None; face_count],
        face_holes: vec![Vec::new(); face_count],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

/// A document with exactly one watertight box object, built through
/// `Document::ingest` — a legitimate public construction path
/// (docs/design/api-kernel-map.md §6.1) that needs no sketch/extrude
/// commands (a different wave's scope).
fn box_document() -> Document {
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(unit_box_mesh("Box"))],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(scene, Vec::new()).expect("ingest a plain box");
    doc
}

/// A single-tetrahedron binary STL fixture, built by hand at the byte
/// level (no importer round-trip involved in producing it): 80-byte
/// header, a `u32` triangle count of 4, then 4×50 bytes (zero normal, 3
/// vertices, zero attribute count each) — a minimal but genuine solid.
fn tetrahedron_stl_bytes() -> Vec<u8> {
    let tris: &[[[f32; 3]; 3]] = &[
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
    ];
    let mut out = vec![0u8; 80];
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for tri in tris {
        for _ in 0..3 {
            out.extend_from_slice(&0f32.to_le_bytes());
        }
        for v in tri {
            for c in v {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

// -------------------------------------------------------------- run --out

/// `hew-cli run`: a script that hellos, opens a fresh document, draws a
/// rectangle and extrudes it (chained through a transaction's `$ref`,
/// docs/agents/HEW_API.md §6.2), then `--out`-saves. The resulting `.hew` file
/// reloads with exactly one object.
#[test]
fn run_script_draws_and_extrudes_and_out_saves_one_object() {
    let dir = scratch_dir("run");
    let script_path = dir.join("script.json");
    let out_path = dir.join("model.hew");

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "hew.doc.transact",
            "params": {
                "label": "box",
                "commands": [
                    {
                        "method": "hew.sketch.draw_rect",
                        "as": "rect",
                        "params": {
                            "plane": { "ground": true },
                            "corner_a": [0.0, 0.0, 0.0],
                            "corner_b": [1.0, 1.0, 0.0],
                        },
                    },
                    {
                        "method": "hew.solid.extrude",
                        "params": {
                            "region": { "$ref": "rect#/region_id" },
                            "distance": 0.5,
                        },
                    },
                ],
            },
        },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, Some(&out_path), None);
    assert_eq!(
        outcome.exit_code,
        0,
        "script should succeed: {:?}",
        outcome.responses.last()
    );
    assert_eq!(
        outcome.responses.len(),
        3,
        "one response per id-carrying frame"
    );
    assert!(
        outcome.responses.iter().all(|r| r.get("error").is_none()),
        "no frame should have refused: {:?}",
        outcome.responses
    );

    let bytes = std::fs::read(&out_path).expect("--out wrote the file");
    let reloaded = Document::load(&bytes).expect("the saved document reloads");
    assert_eq!(
        reloaded.visible_object_ids().len(),
        1,
        "the extrude produced exactly one solid"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A script whose first command refuses stops immediately (exit 1) and
/// never reaches `--out` — proving the "first refusal stops the script"
/// contract, not just its happy path.
#[test]
fn run_script_stops_on_the_first_refusal() {
    let dir = scratch_dir("run-refuse");
    let script_path = dir.join("script.json");
    let out_path = dir.join("model.hew");

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        // No hew.doc.new/open first: this needs an attached document.
        { "jsonrpc": "2.0", "id": 2, "method": "hew.query.scene", "params": {} },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, Some(&out_path), None);
    assert_eq!(outcome.exit_code, 1);
    assert!(!out_path.exists(), "a stopped script never reaches --out");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `--out` together with `--live` is rejected up front as a usage error
/// (docs/agents/HEW_API.md §12) rather than attempted and left to fail on the
/// script's last step: a live host keeps document persistence
/// user-driven and refuses a remote `hew.doc.save` outright, so the old
/// behavior (send the whole script, THEN discover a save can never
/// succeed) made an otherwise fully successful script exit 1. The script
/// file here is deliberately never even written — the guard fires before
/// `run_script` touches the filesystem or the network at all.
#[test]
fn run_script_rejects_out_together_with_live() {
    let dir = scratch_dir("run-out-live-usage");
    let script_path = dir.join("script.jsonl"); // never created
    let out_path = dir.join("model.hew");

    let live = hew_cli::live::LiveOptions {
        launch: false,
        instance: None,
        document: None,
    };
    let outcome = hew_cli::run::run_script(&script_path, Some(&out_path), Some(&live));
    assert_eq!(
        outcome.exit_code, 2,
        "--out with --live is a usage error, not a runtime failure"
    );
    assert!(outcome.responses.is_empty());
    assert!(!out_path.exists());
    assert!(
        !script_path.exists(),
        "the guard fires before the script is even read"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------- dispatch --file

/// `hew-cli dispatch --file`: opens a pre-built `.hew` file (written
/// directly through the kernel, no `hew-cli` involved in creating it) and
/// dispatches a single read-only command against it.
#[test]
fn dispatch_file_queries_a_prebuilt_document() {
    let dir = scratch_dir("dispatch");
    let path = dir.join("box.hew");
    std::fs::write(&path, box_document().save()).unwrap();

    let outcome = hew_cli::run::dispatch_file(&path, "hew.query.scene", serde_json::json!({}));
    assert_eq!(outcome.exit_code, 0, "response: {:?}", outcome.response);
    let response = outcome.response.expect("dispatch produced a response");
    assert!(
        response.get("error").is_none(),
        "hew.query.scene should not refuse against a plain box: {response:?}"
    );
    assert!(
        response["result"].get("tree").is_some(),
        "hew.query.scene's result carries a tree: {response:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A read-only dispatch never rewrites the file — the untouched bytes
/// prove `dispatch --file`'s "for a query, just reads" carve-out.
#[test]
fn dispatch_file_read_only_command_does_not_rewrite_the_file() {
    let dir = scratch_dir("dispatch-readonly");
    let path = dir.join("box.hew");
    let original = box_document().save();
    std::fs::write(&path, &original).unwrap();

    let outcome = hew_cli::run::dispatch_file(&path, "hew.query.scene", serde_json::json!({}));
    assert_eq!(outcome.exit_code, 0);

    let after = std::fs::read(&path).unwrap();
    assert_eq!(
        original, after,
        "a query must not rewrite the document file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A dry run changes nothing, so `dispatch --file` never saves after one:
/// on a read-only file it answers the rehearsal and exits 0, where a save
/// would fail.
#[test]
fn dispatch_file_dry_run_does_not_save_back() {
    let dir = scratch_dir("dispatch-dry-run");
    let path = dir.join("box.hew");
    let original = box_document().save();
    std::fs::write(&path, &original).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms.clone()).unwrap();

    let outcome = hew_cli::run::dispatch_file(
        &path,
        "hew.doc.transact",
        serde_json::json!({ "dry_run": true, "commands": [
            { "method": "hew.sketch.draw_rect", "as": "p", "params": { "plane": { "ground": true }, "corner_a": [5, 5, 0], "corner_b": [6, 6, 0] } },
            { "method": "hew.solid.extrude", "params": { "region": { "$ref": "p#/region_id" }, "distance": 1.0 } }
        ] }),
    );
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
    assert_eq!(outcome.exit_code, 0, "response: {:?}", outcome.response);
    let response = outcome.response.expect("a response");
    assert_eq!(response["result"]["dry_run"], true, "{response}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "the file is untouched"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ export

/// `CliHost::export_document("stl")` over a real box document: a
/// well-formed binary STL — 80-byte header + `u32` count, then 50 bytes
/// per triangle, and a box triangulates to 12 (2 per face).
#[test]
fn cli_host_exports_a_box_to_well_formed_binary_stl() {
    let doc = box_document();
    let mut host = CliHost::new();
    let bytes = host
        .export_document(&doc, "stl", None, 0, None, None)
        .expect("a solid box exports")
        .expect("no path given — bytes come back inline");

    assert!(
        bytes.len() > 84,
        "at least the header + count + one triangle"
    );
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
    assert!(count > 0, "a box has triangles");
    assert_eq!(
        bytes.len(),
        84 + count as usize * 50,
        "84-byte prefix + 50 bytes per triangle"
    );
}

/// Given a `path`, `export_document` writes the file itself and returns no
/// inline bytes.
#[test]
fn cli_host_export_with_a_path_writes_the_file_and_returns_no_bytes() {
    let dir = scratch_dir("export-path");
    let out = dir.join("box.stl");
    let doc = box_document();
    let mut host = CliHost::new();

    let result = host
        .export_document(&doc, "stl", Some(out.to_str().unwrap()), 0, None, None)
        .expect("export succeeds");
    assert!(
        result.is_none(),
        "bytes were written to the path, not returned"
    );
    let written = std::fs::read(&out).expect("the host wrote the file");
    assert!(written.len() > 84);

    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty document has nothing watertight to export — refuses
/// `nothing_to_export` rather than writing a zero-triangle STL.
#[test]
fn cli_host_export_refuses_nothing_to_export_on_an_empty_document() {
    let doc = Document::new();
    let mut host = CliHost::new();
    let err = host
        .export_document(&doc, "stl", None, 0, None, None)
        .unwrap_err();
    assert_eq!(err.name, "nothing_to_export");
}

/// `hew.doc.export` dispatched through the real JSON-RPC envelope (open a
/// pre-built file, export "3mf"/"glb"/"usdz") returns `bytes_base64` that
/// decodes to exactly the same bytes a direct `CliHost::export_document`
/// call produces on the same document — the dispatcher adds nothing but the
/// base64 wrapping (`crates/api/src/commands/doc.rs`'s `export_doc`).
#[test]
fn dispatch_export_3mf_glb_and_usdz_match_a_direct_export_byte_for_byte() {
    let dir = scratch_dir("dispatch-export");
    let path = dir.join("box.hew");
    std::fs::write(&path, box_document().save()).unwrap();

    for format in ["3mf", "glb", "usdz"] {
        let outcome = hew_cli::run::dispatch_file(
            &path,
            "hew.doc.export",
            serde_json::json!({ "format": format }),
        );
        assert_eq!(outcome.exit_code, 0, "{format}: {:?}", outcome.response);
        let response = outcome.response.expect("dispatch produced a response");
        let bytes_base64 = response["result"]["bytes_base64"]
            .as_str()
            .unwrap_or_else(|| panic!("{format}: bytes_base64 present in {response:?}"));
        let dispatched_bytes = base64_decode(bytes_base64);

        // Compare against a direct export over the exact same on-disk
        // document, not a freshly re-ingested one: the `.hew` format
        // renumbers dense ids per save (docs/agents/HEW_API.md §5.1), so a fresh
        // `box_document()` ingest need not iterate faces/vertices in the
        // same order a reloaded copy does, even though both describe the
        // same box.
        let reloaded = Document::load(&std::fs::read(&path).unwrap()).expect("reloads");
        let mut host = CliHost::new();
        let direct_bytes = host
            .export_document(&reloaded, format, None, 0, None, None)
            .unwrap_or_else(|e| panic!("{format}: direct export succeeds: {e:?}"))
            .unwrap_or_else(|| panic!("{format}: bytes returned inline"));

        assert_eq!(
            dispatched_bytes, direct_bytes,
            "{format}: dispatcher bytes must match a direct export"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------- texture

/// `hew.material.create {texture: {path}}` headless: the CLI host reads the
/// image and the material carries it at the stated tile size; a missing
/// file refuses `load_failed`.
#[test]
fn cli_host_reads_a_texture_path_for_material_create() {
    use api::{Connection, DispatchOutcome, Grant, Request, RequestId};
    let dir = scratch_dir("texture");
    let file = dir.join("grain.jpg");
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    std::fs::write(&file, jpeg).unwrap();

    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    let mut host = CliHost::new();
    let mut call = |id: i64, method: &str, params: serde_json::Value, doc: &mut Document| {
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
            document: None,
        };
        match conn.dispatch(&api::Registry::protocol_1(), doc, &mut host, request) {
            DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => r,
            DispatchOutcome::Dropped | DispatchOutcome::Forward(_) => unreachable!(),
        }
    };
    call(
        1,
        "hew.meta.hello",
        serde_json::json!({"protocol": 1}),
        &mut doc,
    );
    call(2, "hew.doc.new", serde_json::json!({}), &mut doc);
    let made = call(
        3,
        "hew.material.create",
        serde_json::json!({ "name": "Grain", "texture": { "path": file.to_str().unwrap(), "world_size": [0.25, 2.0] } }),
        &mut doc,
    );
    assert!(made.error.is_none(), "{:?}", made.error);
    let texture = doc
        .materials()
        .values()
        .find(|m| m.name == "Grain")
        .and_then(|m| m.texture.clone())
        .expect("a textured material");
    assert_eq!(texture.image, jpeg);
    assert_eq!(texture.format, kernel::ImageFormat::Jpeg);
    assert_eq!(texture.world_size, [0.25, 2.0]);

    let missing = call(
        4,
        "hew.material.create",
        serde_json::json!({ "name": "X", "texture": { "path": dir.join("nope.png").to_str().unwrap(), "world_size": [1, 1] } }),
        &mut doc,
    );
    assert_eq!(
        missing.error.unwrap().data.unwrap()["refusal"],
        "load_failed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------- compute

/// The headless host serves every `hew.compute.*` command and the dry run:
/// nothing in either reaches the host, and the document is unchanged.
#[test]
fn cli_host_serves_compute_and_dry_runs() {
    use api::{Connection, DispatchOutcome, Grant, Request, RequestId};
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    let mut host = CliHost::new();
    let mut call = |id: i64, method: &str, params: serde_json::Value, doc: &mut Document| {
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
            document: None,
        };
        match conn.dispatch(&api::Registry::protocol_1(), doc, &mut host, request) {
            DispatchOutcome::Reply(r) => (r, false),
            DispatchOutcome::DryRun(r) => (r, true),
            DispatchOutcome::Dropped | DispatchOutcome::Forward(_) => unreachable!(),
        }
    };
    call(
        1,
        "hew.meta.hello",
        serde_json::json!({"protocol": 1}),
        &mut doc,
    );
    call(2, "hew.doc.new", serde_json::json!({}), &mut doc);
    let (made, _) = call(
        3,
        "hew.doc.transact",
        serde_json::json!({ "commands": [
            { "method": "hew.sketch.draw_rect", "as": "p", "params": { "plane": { "ground": true }, "corner_a": [0, 0, 0], "corner_b": [1, 1, 0] } },
            { "method": "hew.solid.extrude", "params": { "region": { "$ref": "p#/region_id" }, "distance": 1.0 } }
        ] }),
        &mut doc,
    );
    assert!(made.error.is_none(), "{:?}", made.error);
    let id = made.result.unwrap()["results"][1]["object_id"].clone();
    let before = doc.save();
    let unit = serde_json::json!({ "box": { "min": [0, 0, 0], "max": [1, 1, 1] } });
    for (method, params) in [
        (
            "hew.compute.measure",
            serde_json::json!({ "solid": { "id": id } }),
        ),
        (
            "hew.compute.boolean",
            serde_json::json!({ "op": "intersect", "a": { "id": id }, "b": unit, "want": ["mesh", "topology", "measure", "shells"] }),
        ),
        (
            "hew.compute.slice",
            serde_json::json!({ "solid": unit, "plane": { "origin": [0.5, 0, 0], "normal": [1, 0, 0] } }),
        ),
        (
            "hew.compute.shells",
            serde_json::json!({ "solid": unit, "want_mesh": true }),
        ),
        (
            "hew.compute.outline",
            serde_json::json!({ "id": id, "which": "largest_face" }),
        ),
        (
            "hew.compute.outline",
            serde_json::json!({ "id": id, "which": "projection" }),
        ),
        (
            "hew.compute.offset",
            serde_json::json!({ "loops": [[[0, 0], [1, 0], [1, 1], [0, 1]]], "distance": 0.1 }),
        ),
    ] {
        let (r, dry) = call(4, method, params, &mut doc);
        assert!(r.error.is_none(), "{method}: {:?}", r.error);
        assert!(!dry);
    }
    for method in ["hew.compute.project", "hew.compute.stretch"] {
        let (r, _) = call(5, method, serde_json::json!({}), &mut doc);
        assert_eq!(r.error.unwrap().data.unwrap()["refusal"], "unimplemented");
    }
    let (r, dry) = call(
        6,
        "hew.doc.transact",
        serde_json::json!({ "dry_run": true, "commands": [
            { "method": "hew.solid.subtract", "as": "cut", "params": { "a": id, "b": id } }
        ] }),
        &mut doc,
    );
    assert!(dry, "answered as a dry run");
    assert_eq!(
        r.error.unwrap().data.unwrap()["refusal"],
        "degenerate_contact",
        "the rehearsal refuses as the commit would"
    );
    assert_eq!(doc.save(), before, "nothing changed");
}

// ------------------------------------------------------------------ import

/// `CliHost::import_document` on a hand-built one-tetrahedron STL fixture:
/// with `units: "m"` it ingests into a fresh document as exactly one
/// object, and the returned report says so.
#[test]
fn cli_host_imports_a_tetrahedron_stl_with_units() {
    let dir = scratch_dir("import");
    let path = dir.join("tet.stl");
    std::fs::write(&path, tetrahedron_stl_bytes()).unwrap();

    let mut doc = Document::new();
    let mut host = CliHost::new();
    let report = host
        .import_document(
            &mut doc,
            path.to_str().unwrap(),
            &api::ImportOptions {
                units: Some("m".to_string()),
                ..Default::default()
            },
        )
        .expect("a valid tetrahedron STL imports");

    assert_eq!(report["objects_created"], 1);
    assert_eq!(doc.visible_object_ids().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// STL carries no units of its own — importing without `units` refuses
/// typed rather than guessing (docs/agents/HEW_API.md §7's semantics note).
#[test]
fn cli_host_import_refuses_units_required_without_units() {
    let dir = scratch_dir("import-nounits");
    let path = dir.join("tet.stl");
    std::fs::write(&path, tetrahedron_stl_bytes()).unwrap();

    let mut doc = Document::new();
    let mut host = CliHost::new();
    let err = host
        .import_document(
            &mut doc,
            path.to_str().unwrap(),
            &api::ImportOptions::default(),
        )
        .unwrap_err();
    assert_eq!(err.name, "units_required");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- snapshot

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// A snapshot's `png_base64` decodes to a well-formed PNG whose IHDR
/// dimensions match the result's own declared `width`/`height`.
fn assert_png_matches_declared_size(result: &serde_json::Value) {
    let png = base64_decode(
        result["png_base64"]
            .as_str()
            .expect("png_base64 is a string"),
    );
    assert_eq!(&png[0..8], &PNG_MAGIC, "PNG signature");
    assert_eq!(&png[12..16], b"IHDR");
    let width = result["width"].as_u64().unwrap() as u32;
    let height = result["height"].as_u64().unwrap() as u32;
    assert_eq!(&png[16..20], &width.to_be_bytes(), "IHDR width");
    assert_eq!(&png[20..24], &height.to_be_bytes(), "IHDR height");
}

/// A real headless render through the whole dispatcher: a script builds
/// two boxes (through envelopes, not `Document::ingest`, so this exercises
/// the same path an agent's `hew_transact` calls would), then a bare
/// `hew.view.snapshot` request with `include_ids: true` renders them. The
/// camera is explicit and top-down so the center pixel is guaranteed to
/// land on the boxes' overlapping footprint, not the background gap a
/// fitted view's angle could otherwise land in.
#[test]
fn run_script_snapshot_renders_both_boxes_and_the_id_buffer_resolves_the_center_pixel() {
    let dir = scratch_dir("snapshot-two-box");
    let script_path = dir.join("script.json");

    let draw_and_extrude = |label: &str, corner_a: [f64; 2], corner_b: [f64; 2]| {
        serde_json::json!({
            "label": label,
            "commands": [
                {
                    "method": "hew.sketch.draw_rect",
                    "as": "rect",
                    "params": {
                        "plane": { "ground": true },
                        "corner_a": [corner_a[0], corner_a[1], 0.0],
                        "corner_b": [corner_b[0], corner_b[1], 0.0],
                    },
                },
                {
                    "method": "hew.solid.extrude",
                    "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.5 },
                },
            ],
        })
    };

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        {
            "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact",
            "params": draw_and_extrude("box a", [0.0, 0.0], [1.5, 1.0]),
        },
        {
            "jsonrpc": "2.0", "id": 4, "method": "hew.doc.transact",
            "params": draw_and_extrude("box b", [1.0, 0.0], [2.5, 1.0]),
        },
        {
            "jsonrpc": "2.0", "id": 5, "method": "hew.view.snapshot",
            "params": {
                "width": 64,
                "height": 64,
                "include_ids": true,
                "camera": {
                    "eye": [1.25, 0.5, 5.0],
                    "target": [1.25, 0.5, 0.25],
                    "up": [0.0, 1.0, 0.0],
                },
            },
        },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, None, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "script should succeed: {:?}",
        outcome.responses.last()
    );
    assert_eq!(outcome.responses.len(), 5);
    assert!(
        outcome.responses.iter().all(|r| r.get("error").is_none()),
        "no frame should have refused: {:?}",
        outcome.responses
    );

    let result = &outcome.responses[4]["result"];
    assert_png_matches_declared_size(result);
    assert_eq!(result["width"], 64);
    assert_eq!(result["height"], 64);

    let palette = result["id_palette"].as_array().expect("id_palette present");
    assert_eq!(palette.len(), 2, "two boxes, two palette entries");
    for id in palette {
        assert!(
            id.as_str().unwrap().starts_with("obj_"),
            "palette entries are public object ids: {id}"
        );
    }

    let id_buffer = base64_decode(
        result["id_buffer_base64"]
            .as_str()
            .expect("id_buffer_base64 present"),
    );
    assert_eq!(id_buffer.len(), 64 * 64 * 2, "u16 little-endian per pixel");
    let center_pixel = 32 * 64 + 32;
    let center_id =
        u16::from_le_bytes([id_buffer[center_pixel * 2], id_buffer[center_pixel * 2 + 1]]);
    assert!(
        center_id >= 1 && (center_id as usize) <= palette.len(),
        "the center pixel — aimed at the boxes' overlapping footprint — resolves to a real palette entry, not background (0)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `view: "front"` renders through the named-standard-view path (fitted
/// to the scene bbox), distinct from the explicit-camera path above.
#[test]
fn run_script_snapshot_with_a_named_front_view_renders() {
    let dir = scratch_dir("snapshot-front-view");
    let script_path = dir.join("script.json");

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        {
            "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact",
            "params": {
                "label": "box",
                "commands": [
                    {
                        "method": "hew.sketch.draw_rect", "as": "rect",
                        "params": { "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [1.0, 1.0, 0.0] },
                    },
                    {
                        "method": "hew.solid.extrude",
                        "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.5 },
                    },
                ],
            },
        },
        {
            "jsonrpc": "2.0", "id": 4, "method": "hew.view.snapshot",
            "params": { "view": "front", "width": 32, "height": 48 },
        },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, None, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "response: {:?}",
        outcome.responses.last()
    );
    let result = &outcome.responses[3]["result"];
    assert_png_matches_declared_size(result);
    assert_eq!(result["width"], 32);
    assert_eq!(result["height"], 48);
    assert!(
        result.get("id_buffer_base64").is_none(),
        "include_ids defaults to false"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole point of `hew.view.snapshot`'s `path` parameter (docs/agents/HEW_API.md
/// §7, mirroring `hew.doc.export`): the inline base64 PNG blows past an MCP
/// tool result's size budget at any useful resolution. Requests the
/// resolution ceiling (2048x2048), writes to a real file through `CliHost`,
/// and asserts the JSON-RPC response itself — not just the omission of
/// `png_base64` — stays small.
#[test]
fn run_script_snapshot_with_a_path_writes_a_real_png_and_stays_small() {
    let dir = scratch_dir("snapshot-path");
    let script_path = dir.join("script.json");
    let png_path = dir.join("shot.png");

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        {
            "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact",
            "params": {
                "label": "box",
                "commands": [
                    {
                        "method": "hew.sketch.draw_rect", "as": "rect",
                        "params": { "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [1.0, 1.0, 0.0] },
                    },
                    {
                        "method": "hew.solid.extrude",
                        "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.5 },
                    },
                ],
            },
        },
        {
            "jsonrpc": "2.0", "id": 4, "method": "hew.view.snapshot",
            "params": {
                "view": "iso", "width": 2048, "height": 2048,
                "path": png_path.to_str().unwrap(),
            },
        },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, None, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "response: {:?}",
        outcome.responses.last()
    );

    let response = &outcome.responses[3];
    let encoded = serde_json::to_vec(response).unwrap();
    assert!(
        encoded.len() < 2048,
        "a path response stays tiny regardless of resolution (2048x2048 requested here) — got {} bytes, the exact MCP tool-result-budget problem path exists to solve",
        encoded.len()
    );

    let result = &response["result"];
    assert_eq!(result["path"], png_path.to_str().unwrap());
    assert_eq!(result["width"], 2048);
    assert_eq!(result["height"], 2048);
    assert!(
        result.get("png_base64").is_none(),
        "path given: no inline bytes"
    );

    let png_bytes = std::fs::read(&png_path).expect("write_snapshot wrote a real file");
    assert_eq!(&png_bytes[0..8], &PNG_MAGIC, "PNG signature");
    assert_eq!(&png_bytes[12..16], b"IHDR");
    assert_eq!(&png_bytes[16..20], &2048u32.to_be_bytes(), "IHDR width");
    assert_eq!(&png_bytes[20..24], &2048u32.to_be_bytes(), "IHDR height");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `path` plus `include_ids` writes the id-buffer as a `<path>.ids.bin`
/// sidecar rather than base64-encoding it inline — same rationale as the
/// PNG itself, and the id-buffer is the larger of the two at any given
/// resolution.
#[test]
fn run_script_snapshot_with_path_and_include_ids_writes_the_id_buffer_sidecar_file() {
    let dir = scratch_dir("snapshot-path-ids");
    let script_path = dir.join("script.json");
    let png_path = dir.join("shot.png");

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        {
            "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact",
            "params": {
                "label": "box",
                "commands": [
                    {
                        "method": "hew.sketch.draw_rect", "as": "rect",
                        "params": { "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [1.0, 1.0, 0.0] },
                    },
                    {
                        "method": "hew.solid.extrude",
                        "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.5 },
                    },
                ],
            },
        },
        {
            "jsonrpc": "2.0", "id": 4, "method": "hew.view.snapshot",
            "params": {
                "view": "iso", "width": 32, "height": 32, "include_ids": true,
                "path": png_path.to_str().unwrap(),
            },
        },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, None, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "response: {:?}",
        outcome.responses.last()
    );

    let result = &outcome.responses[3]["result"];
    let expected_ids_path = format!("{}.ids.bin", png_path.to_str().unwrap());
    assert_eq!(result["id_buffer_path"], expected_ids_path);
    assert!(result["id_palette"].is_array(), "id_palette stays inline");
    assert!(result.get("id_buffer_base64").is_none());

    let id_bytes = std::fs::read(&expected_ids_path).expect("the id-buffer sidecar was written");
    assert_eq!(id_bytes.len(), 32 * 32 * 2, "u16 little-endian per pixel");

    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty document has no visible solids — `hew.view.snapshot` refuses
/// `nothing_to_render` rather than returning a background-only PNG.
#[test]
fn run_script_snapshot_on_an_empty_document_refuses_nothing_to_render() {
    let dir = scratch_dir("snapshot-empty");
    let script_path = dir.join("script.json");

    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.view.snapshot", "params": {} },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();

    let outcome = hew_cli::run::run_script(&script_path, None, None);
    assert_eq!(outcome.exit_code, 1, "the refusal stops the script");
    let error = &outcome.responses[2]["error"];
    assert_eq!(error["data"]["refusal"], "nothing_to_render");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A user-hidden object neither renders nor influences view fitting —
/// the review's hidden-state finding, pinned.
#[test]
fn snapshot_respects_user_hidden_state() {
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![
            ImportNode::Mesh(unit_box_mesh("A")),
            ImportNode::Mesh(unit_box_mesh("B")),
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(scene, Vec::new()).expect("ingest two boxes");
    let ids = doc.visible_object_ids();
    assert_eq!(ids.len(), 2);
    let (a, b) = (ids[0], ids[1]);
    doc.set_node_user_hidden(kernel::NodeId::Object(b), true);

    let mut host = CliHost::new();
    let params = api::host::SnapshotParams {
        width: 128,
        height: 128,
        camera: None,
        view: api::host::StandardView::from_name("iso"),
        scene: None,
        include_ids: true,
        include_depth: false,
        depth_bits: 8,
        include_normals: false,
        path: None,
        selection: None,
        background: softrender::BACKGROUND_RGBA,
    };
    let result = host.snapshot(&doc, &params).expect("renders");
    let public = |id: kernel::ObjectId| {
        api::ids::public_id(
            &kernel::EntityRef::Object(id),
            doc.sid_of(&kernel::EntityRef::Object(id)).unwrap(),
        )
    };
    assert!(
        result.id_palette.contains(&public(a)),
        "visible object present"
    );
    assert!(
        !result.id_palette.contains(&public(b)),
        "user-hidden object never renders"
    );
}

/// `hew.view.snapshot`'s `scene` param renders through the Scene's OWN
/// resolved hidden set and captured camera (`Document::resolve_scene`),
/// not the document's live state — the whole point of a Scene as a saved
/// view. An object the Scene captured as hidden stays hidden even though
/// nothing in the LIVE document is hiding it anymore.
#[test]
fn snapshot_scene_renders_through_the_scenes_own_resolved_state() {
    let mut doc = Document::new();
    let import = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![
            ImportNode::Mesh(unit_box_mesh("A")),
            ImportNode::Mesh(unit_box_mesh("B")),
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(import, Vec::new()).expect("ingest two boxes");
    let ids = doc.visible_object_ids();
    let (a, b) = (ids[0], ids[1]);

    // Capture a Scene with B hidden and an explicit camera, THEN unhide B
    // live — the snapshot must still honor the Scene's own captured
    // state, not the document's current one.
    doc.set_node_user_hidden(kernel::NodeId::Object(b), true);
    let camera = kernel::CameraState::framed(
        kernel::CameraProjection::Perspective,
        40.0,
        kernel::Point3::new(5.0, 0.0, 0.0),
        kernel::Point3::ORIGIN,
        kernel::Vec3::new(0.0, 0.0, 1.0),
    );
    let sid = doc
        .add_scene(
            Some("Snap".to_string()),
            kernel::SceneProps::ALL,
            Some(camera),
            None,
            None,
        )
        .expect("add scene");
    doc.set_node_user_hidden(kernel::NodeId::Object(b), false);

    let mut host = CliHost::new();
    let params = api::host::SnapshotParams {
        width: 128,
        height: 128,
        camera: None,
        view: None,
        scene: Some(sid),
        include_ids: true,
        include_depth: false,
        depth_bits: 8,
        include_normals: false,
        path: None,
        selection: None,
        background: softrender::BACKGROUND_RGBA,
    };
    let result = host.snapshot(&doc, &params).expect("renders");
    let public = |id: kernel::ObjectId| {
        api::ids::public_id(
            &kernel::EntityRef::Object(id),
            doc.sid_of(&kernel::EntityRef::Object(id)).unwrap(),
        )
    };
    assert!(
        result.id_palette.contains(&public(a)),
        "A is visible in the Scene"
    );
    assert!(
        !result.id_palette.contains(&public(b)),
        "B stays hidden per the Scene's captured state, even though it is live-visible now"
    );
}

/// A camera-only Scene captures neither hidden property, so its snapshot
/// renders with the DOCUMENT's live hidden state — what activating it in
/// the app shows — never with everything un-hidden.
#[test]
fn snapshot_camera_only_scene_keeps_the_documents_live_hidden_state() {
    let mut doc = Document::new();
    let import = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![
            ImportNode::Mesh(unit_box_mesh("A")),
            ImportNode::Mesh(unit_box_mesh("B")),
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(import, Vec::new()).expect("ingest two boxes");
    let ids = doc.visible_object_ids();
    let (a, b) = (ids[0], ids[1]);
    let camera = kernel::CameraState::framed(
        kernel::CameraProjection::Perspective,
        40.0,
        kernel::Point3::new(5.0, 0.0, 0.0),
        kernel::Point3::ORIGIN,
        kernel::Vec3::new(0.0, 0.0, 1.0),
    );
    let props = kernel::SceneProps {
        camera: true,
        ..kernel::SceneProps::NONE
    };
    let sid = doc
        .add_scene(Some("Cam".to_string()), props, Some(camera), None, None)
        .expect("add scene");
    // Hide B live AFTER capture: a camera-only Scene leaves it hidden.
    doc.set_node_user_hidden(kernel::NodeId::Object(b), true);

    let mut host = CliHost::new();
    let params = api::host::SnapshotParams {
        width: 128,
        height: 128,
        camera: None,
        view: None,
        scene: Some(sid),
        include_ids: true,
        include_depth: false,
        depth_bits: 8,
        include_normals: false,
        path: None,
        selection: None,
        background: softrender::BACKGROUND_RGBA,
    };
    let result = host.snapshot(&doc, &params).expect("renders");
    let public = |id: kernel::ObjectId| {
        api::ids::public_id(
            &kernel::EntityRef::Object(id),
            doc.sid_of(&kernel::EntityRef::Object(id)).unwrap(),
        )
    };
    assert!(result.id_palette.contains(&public(a)));
    assert!(
        !result.id_palette.contains(&public(b)),
        "the document's live hidden state applies when the Scene captures no hidden property"
    );
}

/// `scene` refuses `unknown_scene`, exactly like any other Scene
/// command, rather than a generic params error.
#[test]
fn snapshot_scene_refuses_unknown_scene() {
    let mut doc = Document::new();
    let import = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(unit_box_mesh("A"))],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(import, Vec::new()).expect("ingest a box");

    let mut host = CliHost::new();
    let params = api::host::SnapshotParams {
        width: 64,
        height: 64,
        camera: None,
        view: None,
        scene: Some(0xffff_ffff),
        include_ids: false,
        include_depth: false,
        depth_bits: 8,
        include_normals: false,
        path: None,
        selection: None,
        background: softrender::BACKGROUND_RGBA,
    };
    let err = host.snapshot(&doc, &params).unwrap_err();
    assert_eq!(err.name, "unknown_scene");
}

/// `hew.print.pdf` and `hew.view.line_drawing` (docs/design/printing.md
/// §9b) through the real CLI host: a 100 mm cube at 1:1 on Letter is one
/// page whose SVG spans exactly 100 mm; at 1:1 a 500 × 300 mm slab tiles
/// 2 × 2 (auto → landscape); the PDF is well-formed with one page per tile;
/// segments come back with public ids; a bad paper name is a params error.
#[test]
fn run_script_print_pdf_and_line_drawing_lay_out_pages_and_draw_to_scale() {
    let dir = scratch_dir("print-pdf");
    let script_path = dir.join("script.json");
    let pdf_path = dir.join("slab.pdf");
    let svg_path = dir.join("cube.svg");
    let cube = serde_json::json!({
        "label": "cube",
        "commands": [
            { "method": "hew.sketch.draw_rect", "as": "rect", "params": { "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [0.1, 0.1, 0.0] } },
            { "method": "hew.solid.extrude", "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.1 } },
        ],
    });
    let script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact", "params": cube },
        { "jsonrpc": "2.0", "id": 4, "method": "hew.view.line_drawing", "params": { "view": "top", "scale": 1.0, "path": svg_path } },
        { "jsonrpc": "2.0", "id": 5, "method": "hew.view.line_drawing", "params": { "view": "iso", "format": "segments", "include_hidden": true } },
        { "jsonrpc": "2.0", "id": 6, "method": "hew.print.pdf", "params": { "view": "top", "scale": 1.0, "paper": "letter" } },
        { "jsonrpc": "2.0", "id": 7, "method": "hew.print.pdf", "params": { "paper": "napkin" } },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script).unwrap()).unwrap();
    let outcome = hew_cli::run::run_script(&script_path, None, None);
    // Frame 7 is a deliberate params error; everything before it succeeds.
    for r in &outcome.responses[..6] {
        assert!(r.get("error").is_none(), "unexpected error: {r}");
    }
    let svg = std::fs::read_to_string(&svg_path).unwrap();
    // A 100 mm square at 1:1 with the writer's 5 mm margin: 110 × 110 mm.
    assert!(svg.contains("width=\"110mm\" height=\"110mm\""), "{svg}");
    assert!(svg.contains("class=\"hard\""));
    let segs = &outcome.responses[4]["result"];
    // Iso cube: 9 visible + 3 hidden.
    assert_eq!(segs["count"], 12);
    let kinds: Vec<&str> = segs["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(kinds.iter().filter(|k| **k == "hidden").count(), 3);
    assert!(
        segs["ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("obj_"))
    );
    let pdf = &outcome.responses[5]["result"];
    assert_eq!(pdf["pages"], 1);
    let bytes = base64_decode(pdf["pdf_base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(
        outcome.responses[6].get("error").is_some(),
        "napkin is not a paper"
    );

    // The slab: 500 × 300 mm at 1:1 → 3 × 2 on Letter (the 10 mm overlap band
    // is reserved inside the printable area, so tiles step 180.5 × 234
    // portrait / 244 × 170.5 landscape; auto keeps portrait on the tie),
    // written to a path.
    let slab = serde_json::json!({
        "label": "slab",
        "commands": [
            { "method": "hew.sketch.draw_rect", "as": "rect", "params": { "plane": { "ground": true }, "corner_a": [1.0, 0.0, 0.0], "corner_b": [1.5, 0.3, 0.0] } },
            { "method": "hew.solid.extrude", "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.02 } },
        ],
    });
    let script2 = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact", "params": slab },
        { "jsonrpc": "2.0", "id": 4, "method": "hew.print.pdf", "params": { "view": "top", "scale": 1.0, "paper": "letter", "style": "shaded", "path": pdf_path } },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&script2).unwrap()).unwrap();
    let outcome = hew_cli::run::run_script(&script_path, None, None);
    let r = &outcome.responses[3];
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(r["result"]["pages"], 6);
    assert_eq!(r["result"]["cols"], 3);
    assert_eq!(r["result"]["rows"], 2);
    let bytes = std::fs::read(&pdf_path).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(
        std::str::from_utf8(&bytes[bytes.len().saturating_sub(64)..])
            .is_ok_and(|s| s.contains("%%EOF"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ==================================================== hew.library.* (headless)

/// `hew.library.*` end to end, headless, through the REAL `CliHost` and a
/// REAL scratch library folder on disk (`HEW_LIBRARY_DIR`) — not a fake
/// `Host` (that's `crates/api/tests/commands_library.rs`'s job) and not a
/// script's own process-internal document (that's the `run_script_draws_
/// and_extrudes_and_out_saves_one_object` test above): draw + extrude a
/// box, save it to the library as a component (the selection resolved
/// in-transaction via `$ref`, §6.1's exact chaining shape), list the
/// library folder and find it there, insert it into a FRESH document, and
/// confirm the geometry actually arrived via `hew.query.scene`.
///
/// `HEW_LIBRARY_DIR` is process-global; this is the only test in this file
/// that sets it, and each test file in this crate is its OWN process
/// (`cargo test` gives every integration-test source file a separate
/// binary), so there is nothing else in this process to race.
#[test]
fn library_save_list_insert_round_trips_geometry_through_a_real_library_folder() {
    let dir = scratch_dir("library-e2e");
    let library_dir = dir.join("library");
    std::fs::create_dir_all(&library_dir).unwrap();
    // SAFETY: see the doc comment above — no other test in this file (and
    // therefore this process) reads or writes this variable.
    unsafe {
        std::env::set_var("HEW_LIBRARY_DIR", &library_dir);
    }

    // 1. Draw + extrude a box, then save the JUST-CREATED object as a
    // library component in the SAME transaction — `selection` resolved
    // through `{"$ref": "box#/object_id"}`, proving `$ref` substitution
    // reaches into an array parameter, not just a bare object one.
    let script_path = dir.join("save.jsonl");
    let build_and_save = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "hew.doc.transact",
            "params": {
                "label": "build and save",
                "commands": [
                    { "method": "hew.sketch.draw_rect", "as": "rect", "params": {
                        "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [1.0, 1.0, 0.0]
                    } },
                    { "method": "hew.solid.extrude", "as": "box", "params": {
                        "region": { "$ref": "rect#/region_id" }, "distance": 1.0
                    } },
                    { "method": "hew.library.save", "params": {
                        "selection": [{ "$ref": "box#/object_id" }],
                        "name": "E2E Table Leg",
                        "keywords": ["e2e", "leg"]
                    } },
                ],
            },
        },
    ]);
    std::fs::write(&script_path, serde_json::to_vec(&build_and_save).unwrap()).unwrap();
    let outcome = hew_cli::run::run_script(&script_path, None, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "build+save script should succeed: {:?}",
        outcome.responses.last()
    );
    let save_result = &outcome.responses[2]["result"]["results"][2];
    let item_id = save_result["id"]
        .as_str()
        .unwrap_or_else(|| panic!("hew.library.save should return an id: {save_result}"))
        .to_string();
    let item_path = save_result["path"]
        .as_str()
        .unwrap_or_else(|| panic!("hew.library.save should return a path: {save_result}"));
    assert!(
        item_path.starts_with("Components/"),
        "a selection save wraps as a component item: {item_path}"
    );
    assert!(
        library_dir.join(item_path).exists(),
        "the item must actually land on disk at {item_path}"
    );

    // 2. `hew.library.list` (via a fresh headless document) finds it.
    let list_script_path = dir.join("list.jsonl");
    let list_script = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.library.list", "params": {} },
    ]);
    std::fs::write(&list_script_path, serde_json::to_vec(&list_script).unwrap()).unwrap();
    let list_outcome = hew_cli::run::run_script(&list_script_path, None, None);
    assert_eq!(
        list_outcome.exit_code,
        0,
        "{:?}",
        list_outcome.responses.last()
    );
    let items = list_outcome.responses[2]["result"]["items"]
        .as_array()
        .expect("hew.library.list returns items");
    assert!(
        items.iter().any(|it| it["id"] == item_id),
        "the saved item must appear in the listing: {items:?}"
    );

    // 3. Insert it into a FRESH document (a separate, empty `.hew` file —
    // exercising the real `dispatch --file` open/mutate/save-back path,
    // not an in-process document the save step happened to leave behind).
    let fresh_doc_path = dir.join("fresh.hew");
    std::fs::write(&fresh_doc_path, Document::new().save()).unwrap();
    let insert_outcome = hew_cli::run::dispatch_file(
        &fresh_doc_path,
        "hew.library.insert",
        serde_json::json!({ "item": item_id, "at": [3.0, 4.0, 0.0] }),
    );
    assert_eq!(
        insert_outcome.exit_code, 0,
        "insert should succeed: {:?}",
        insert_outcome.response
    );
    let insert_result = &insert_outcome.response.unwrap()["result"]["results"][0];
    assert_eq!(insert_result["objects_added"], 1);

    // 4. `hew.query.scene` against the now-saved fresh document confirms
    // the geometry actually arrived: a 1×1×1 watertight solid, its
    // bounding box shifted by exactly a second, differently-placed
    // insert's delta — proving `at` really moved it, without hardcoding
    // `hew.library.save`'s wrap-as-component re-centering convention
    // (extract_item re-expresses the definition in the drawing-axes
    // frame, not necessarily at the object's original world coordinates)
    // into this test's expectations.
    let query_outcome =
        hew_cli::run::dispatch_file(&fresh_doc_path, "hew.query.scene", serde_json::json!({}));
    assert_eq!(query_outcome.exit_code, 0);
    let tree = query_outcome.response.unwrap()["result"]["tree"]
        .as_array()
        .expect("hew.query.scene returns a tree")
        .clone();
    assert_eq!(tree.len(), 1, "exactly the inserted object: {tree:?}");
    assert_eq!(tree[0]["watertight"], true);
    let min = tree[0]["bbox"]["min"]
        .as_array()
        .expect("bbox min is an array")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect::<Vec<_>>();
    let max = tree[0]["bbox"]["max"]
        .as_array()
        .expect("bbox max is an array")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect::<Vec<_>>();
    for i in 0..3 {
        assert!(
            (max[i] - min[i] - 1.0).abs() < 1e-9,
            "the inserted box must keep its original 1×1×1 size: min={min:?} max={max:?}"
        );
    }

    // A second insert at a different `at`, into another fresh document,
    // must land exactly `at`'s delta away from the first — the concrete
    // proof that `at` translates the placement (rather than, say, both
    // inserts coincidentally landing at the same spot).
    let fresh_doc_path_2 = dir.join("fresh2.hew");
    std::fs::write(&fresh_doc_path_2, Document::new().save()).unwrap();
    let insert_outcome_2 = hew_cli::run::dispatch_file(
        &fresh_doc_path_2,
        "hew.library.insert",
        serde_json::json!({ "item": item_id, "at": [3.0 + 10.0, 4.0 + 20.0, 0.0 + 30.0] }),
    );
    assert_eq!(insert_outcome_2.exit_code, 0);
    let query_outcome_2 =
        hew_cli::run::dispatch_file(&fresh_doc_path_2, "hew.query.scene", serde_json::json!({}));
    let tree_2 = query_outcome_2.response.unwrap()["result"]["tree"][0].clone();
    let min_2: Vec<f64> = tree_2["bbox"]["min"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    for (i, delta) in [10.0, 20.0, 30.0].into_iter().enumerate() {
        assert!(
            (min_2[i] - min[i] - delta).abs() < 1e-9,
            "axis {i}: expected the second insert's bbox to shift by {delta} relative to the \
             first (min={min:?}, min_2={min_2:?})"
        );
    }

    unsafe {
        std::env::remove_var("HEW_LIBRARY_DIR");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `hew.view.snapshot {include_depth, include_normals}` end to end through
/// the dispatcher and the headless host: inline passes decode as PNGs of
/// the declared kind, the metric range is sane, and with a `path` each
/// pass lands in its own sidecar file.
#[test]
fn snapshot_depth_and_normals_passes_inline_and_as_sidecars() {
    use api::{Connection, DispatchOutcome, Grant, Request, RequestId};
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    let mut host = CliHost::new();
    let mut id = 0;
    let mut call = |conn: &mut Connection,
                    doc: &mut Document,
                    host: &mut CliHost,
                    method: &str,
                    params: serde_json::Value| {
        id += 1;
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
            document: None,
        };
        match conn.dispatch(&api::Registry::protocol_1(), doc, host, request) {
            DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => r,
            DispatchOutcome::Dropped | DispatchOutcome::Forward(_) => unreachable!(),
        }
    };
    call(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.meta.hello",
        serde_json::json!({"protocol": 1}),
    );
    call(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.doc.new",
        serde_json::json!({}),
    );
    let built = call(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.doc.transact",
        serde_json::json!({"commands":[
            {"method":"hew.sketch.draw_rect","as":"r","params":{"plane":{"ground":true},"corner_a":[0,0,0],"corner_b":[1,1,0]}},
            {"method":"hew.solid.extrude","params":{"region":{"$ref":"r#/region_id"},"distance":1.0}}]}),
    );
    assert!(built.error.is_none(), "{:?}", built.error);

    // Inline.
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.view.snapshot",
        serde_json::json!({"width": 64, "height": 64, "view": "iso", "include_depth": true, "depth_bits": 16, "include_normals": true}),
    );
    let out = r.result.expect("renders");
    let png_header =
        |b64: &str| -> Vec<u8> { api::commands::doc::decode_base64(b64).expect("base64") };
    let depth = png_header(out["depth_png_base64"].as_str().expect("depth inline"));
    assert_eq!(&depth[1..4], b"PNG");
    assert_eq!(&depth[24..26], &[16, 0], "16-bit grayscale");
    let normals = png_header(out["normals_png_base64"].as_str().expect("normals inline"));
    assert_eq!(&normals[24..26], &[8, 2], "8-bit RGB");
    let (near, far) = (
        out["depth_near"].as_f64().unwrap(),
        out["depth_far"].as_f64().unwrap(),
    );
    assert!(near > 0.0 && near < far, "{near} {far}");
    assert_eq!(out["depth_bits"], 16);

    // Sidecars next to a written snapshot.
    let dir = scratch_dir("depth-passes");
    let path = dir.join("shot.png");
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.view.snapshot",
        serde_json::json!({"width": 64, "height": 64, "view": "front", "include_depth": true, "include_normals": true, "path": path.to_str().unwrap()}),
    );
    let out = r.result.expect("renders");
    assert!(path.exists());
    assert!(std::path::Path::new(out["depth_path"].as_str().unwrap()).exists());
    assert!(std::path::Path::new(out["normals_path"].as_str().unwrap()).exists());
    assert!(
        out["depth_path"]
            .as_str()
            .unwrap()
            .ends_with("shot.png.depth.png")
    );
    assert!(out.get("depth_png_base64").is_none());

    // Validation: 12 bits is not a depth format; depth_bits needs include_depth.
    for params in [
        serde_json::json!({"include_depth": true, "depth_bits": 12}),
        serde_json::json!({"depth_bits": 16}),
    ] {
        let r = call(&mut conn, &mut doc, &mut host, "hew.view.snapshot", params);
        assert_eq!(r.error.expect("invalid").code, api::codes::INVALID_PARAMS);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- .skp documents

/// Runs `calls` (method, params) as one script after a hello, headless,
/// and returns every response. Panics if any frame refuses unless
/// `expect_ok` is false.
fn run_calls(dir: &std::path::Path, calls: &[(&str, serde_json::Value)]) -> Vec<serde_json::Value> {
    let mut frames = vec![serde_json::json!(
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } }
    )];
    for (i, (method, params)) in calls.iter().enumerate() {
        frames.push(serde_json::json!(
            { "jsonrpc": "2.0", "id": i + 2, "method": method, "params": params }
        ));
    }
    let script_path = dir.join(format!(
        "script-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&script_path, serde_json::to_vec(&frames).unwrap()).unwrap();
    hew_cli::run::run_script(&script_path, None, None).responses
}

/// The refusal name of the last response, if it refused.
fn last_refusal(responses: &[serde_json::Value]) -> Option<String> {
    responses
        .last()?
        .get("error")?
        .get("data")?
        .get("refusal")?
        .as_str()
        .map(str::to_string)
}

fn add_a_far_box() -> (&'static str, serde_json::Value) {
    (
        "hew.doc.transact",
        serde_json::json!({
            "label": "far box",
            "commands": [
                {
                    "method": "hew.sketch.draw_rect",
                    "as": "rect",
                    "params": {
                        "plane": { "ground": true },
                        "corner_a": [40.0, 40.0, 0.0],
                        "corner_b": [41.0, 41.0, 0.0],
                    },
                },
                {
                    "method": "hew.solid.extrude",
                    "params": { "region": { "$ref": "rect#/region_id" }, "distance": 0.5 },
                },
            ],
        }),
    )
}

fn skp_fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../skp-import/tests/fixtures")
        .join(rel)
}

fn skp_object_count(bytes: &[u8]) -> usize {
    let out = skp_import::import(bytes).expect("the .skp reads back");
    let mut doc = Document::new();
    doc.ingest(out.scene, out.textures_missing)
        .expect("ingest the .skp");
    doc.visible_object_ids().len()
}

fn container_of(bytes: &[u8]) -> skp_import::openskp::Container {
    skp_import::openskp::detect_container(bytes)
}

/// Open a `.skp`, edit, save back to the same path, reopen: the edit is
/// there and the file is still in the container it came in.
fn open_edit_save_reopen(fixture: &str, tag: &str) {
    let dir = scratch_dir(tag);
    let path = dir.join("model.skp");
    std::fs::copy(skp_fixture(fixture), &path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let before = skp_object_count(&original);
    let p = path.to_str().unwrap();

    let responses = run_calls(
        &dir,
        &[
            ("hew.doc.open", serde_json::json!({ "path": p })),
            add_a_far_box(),
            ("hew.doc.save", serde_json::json!({})),
        ],
    );
    assert!(
        responses.iter().all(|r| r.get("error").is_none()),
        "{responses:?}"
    );
    let save = &responses.last().unwrap()["result"];
    assert!(
        save.get("bytes_base64").is_none(),
        "the host wrote the file"
    );
    assert!(save["simplified"].is_array(), "{save}");
    assert!(save["writer_notes"].is_array(), "{save}");

    let saved = std::fs::read(&path).unwrap();
    assert_ne!(saved, original, "the edit changed the file");
    assert_eq!(
        container_of(&saved),
        container_of(&original),
        "the container is unchanged"
    );
    assert_eq!(
        skp_object_count(&saved),
        before + 1,
        "the edit is in the file"
    );

    // And through the API once more: reopen, count.
    let responses = run_calls(
        &dir,
        &[
            ("hew.doc.open", serde_json::json!({ "path": p })),
            ("hew.query.scene", serde_json::json!({})),
        ],
    );
    assert!(responses.iter().all(|r| r.get("error").is_none()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_edit_save_reopen_a_2017_skp() {
    open_edit_save_reopen("box.skp", "skp17");
}

#[test]
fn open_edit_save_reopen_a_2026_skp() {
    open_edit_save_reopen("2026/box.skp", "skp26");
}

/// Saving an opened `.skp` to a new `.hew` path writes real `.hew` bytes
/// and leaves the `.skp` as it was.
#[test]
fn save_a_skp_as_hew_to_a_new_path() {
    let dir = scratch_dir("skp-as-hew");
    let skp = dir.join("model.skp");
    std::fs::copy(skp_fixture("box.skp"), &skp).unwrap();
    let original = std::fs::read(&skp).unwrap();
    let hew = dir.join("copy.hew");

    let responses = run_calls(
        &dir,
        &[
            (
                "hew.doc.open",
                serde_json::json!({ "path": skp.to_str().unwrap() }),
            ),
            (
                "hew.doc.save",
                serde_json::json!({ "path": hew.to_str().unwrap() }),
            ),
        ],
    );
    assert!(
        responses.iter().all(|r| r.get("error").is_none()),
        "{responses:?}"
    );
    assert_eq!(responses.last().unwrap()["result"]["format"], "hew");
    let reloaded = Document::load(&std::fs::read(&hew).unwrap()).expect("real .hew bytes");
    assert_eq!(
        reloaded.visible_object_ids().len(),
        skp_object_count(&original)
    );
    assert_eq!(
        std::fs::read(&skp).unwrap(),
        original,
        "the .skp is untouched"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `.hew` document saved to a `.skp` path writes the 2026 container; an
/// explicit `skp2017` writes the 2017 one.
#[test]
fn save_a_hew_document_as_each_skp_container() {
    let dir = scratch_dir("hew-as-skp");
    let hew = dir.join("model.hew");
    std::fs::write(&hew, box_document().save()).unwrap();
    let a = dir.join("a.skp");
    let b = dir.join("b.skp");
    let responses = run_calls(
        &dir,
        &[
            (
                "hew.doc.open",
                serde_json::json!({ "path": hew.to_str().unwrap() }),
            ),
            (
                "hew.doc.save",
                serde_json::json!({ "path": a.to_str().unwrap() }),
            ),
            (
                "hew.doc.save",
                serde_json::json!({ "path": b.to_str().unwrap(), "format": "skp2017" }),
            ),
        ],
    );
    assert!(
        responses.iter().all(|r| r.get("error").is_none()),
        "{responses:?}"
    );
    use skp_import::openskp::Container;
    assert_eq!(container_of(&std::fs::read(&a).unwrap()), Container::Zip);
    assert_eq!(
        container_of(&std::fs::read(&b).unwrap()),
        Container::Carchive2017
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A format that contradicts the path's extension is refused
/// (`format_mismatch`) and writes nothing, in both directions: one
/// format's bytes never land in the other's file.
#[test]
fn save_with_a_format_that_contradicts_the_extension_refuses() {
    let dir = scratch_dir("mismatch");
    let skp = dir.join("model.skp");
    let hew = dir.join("model.hew");
    std::fs::copy(skp_fixture("box.skp"), &skp).unwrap();
    let original = std::fs::read(&skp).unwrap();

    for (path, format) in [(&skp, "hew"), (&hew, "skp"), (&hew, "skp2017")] {
        let responses = run_calls(
            &dir,
            &[
                (
                    "hew.doc.open",
                    serde_json::json!({ "path": skp.to_str().unwrap() }),
                ),
                (
                    "hew.doc.save",
                    serde_json::json!({ "path": path.to_str().unwrap(), "format": format }),
                ),
            ],
        );
        assert_eq!(
            last_refusal(&responses).as_deref(),
            Some("format_mismatch"),
            "{path:?} + {format}: {responses:?}"
        );
    }
    assert_eq!(
        std::fs::read(&skp).unwrap(),
        original,
        "the .skp is untouched"
    );
    assert!(!hew.exists(), "nothing was written to the .hew path");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `hew.doc.export` writes both SketchUp containers and never makes the
/// export the document's file: a following bare save still refuses
/// `path_required`.
#[test]
fn export_writes_both_skp_formats_without_changing_the_working_file() {
    let dir = scratch_dir("export-skp");
    let hew = dir.join("model.hew");
    std::fs::write(&hew, box_document().save()).unwrap();
    let a = dir.join("a.skp");
    let b = dir.join("b.skp");
    let responses = run_calls(
        &dir,
        &[
            ("hew.doc.new", serde_json::json!({})),
            (
                "hew.doc.import",
                serde_json::json!({ "path": skp_fixture("box.skp").to_str().unwrap() }),
            ),
            (
                "hew.doc.export",
                serde_json::json!({ "format": "skp", "path": a.to_str().unwrap() }),
            ),
            (
                "hew.doc.export",
                serde_json::json!({ "format": "skp2017", "path": b.to_str().unwrap() }),
            ),
            ("hew.doc.save", serde_json::json!({})),
        ],
    );
    use skp_import::openskp::Container;
    assert_eq!(container_of(&std::fs::read(&a).unwrap()), Container::Zip);
    assert_eq!(
        container_of(&std::fs::read(&b).unwrap()),
        Container::Carchive2017
    );
    assert_eq!(last_refusal(&responses).as_deref(), Some("path_required"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `hew.doc.import` of a `.skp` stays additive and now carries the
/// importer's `warnings`.
#[test]
fn import_of_a_skp_is_additive_and_reports_warnings() {
    let mut doc = box_document();
    let before = doc.visible_object_ids().len();
    let mut host = CliHost::new();
    let report = host
        .import_document(
            &mut doc,
            skp_fixture("box.skp").to_str().unwrap(),
            &api::ImportOptions::default(),
        )
        .expect("imports");
    assert!(report["warnings"].is_array(), "{report}");
    assert!(doc.visible_object_ids().len() > before, "additive");
}

/// A failed `.skp` open is a typed `load_failed`, not a panic, and leaves
/// the document alone.
#[test]
fn open_of_a_broken_skp_refuses_load_failed() {
    let dir = scratch_dir("bad-skp");
    let bad = dir.join("bad.skp");
    std::fs::write(&bad, b"not a sketchup file").unwrap();
    let responses = run_calls(
        &dir,
        &[(
            "hew.doc.open",
            serde_json::json!({ "path": bad.to_str().unwrap() }),
        )],
    );
    assert_eq!(last_refusal(&responses).as_deref(), Some("load_failed"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The units a `.skp` opens in: `units` by name, else the save back keeps
/// the file's own and a new file or an export takes meters (a new app's
/// display units — a headless session has no preference of its own).
#[test]
fn skp_display_units_follow_the_app_save_rules_or_a_named_unit() {
    let dir = scratch_dir("skp-units");
    let model = dir.join("model.skp");
    std::fs::copy(skp_fixture("box.skp"), &model).unwrap();
    let unit = |p: &std::path::Path| {
        skp_import::openskp::Model::parse(&std::fs::read(p).unwrap())
            .unwrap()
            .units
            .length_unit
    };
    let own = unit(&model);
    assert_ne!(own, 4, "the fixture is not in meters");
    let (m, p) = (model.to_str().unwrap(), |n: &str| dir.join(n));
    let out = |n: &str| p(n).to_str().unwrap().to_string();
    let r = run_calls(
        &dir,
        &[
            ("hew.doc.open", serde_json::json!({ "path": m })),
            (
                "hew.doc.export",
                serde_json::json!({ "format": "skp", "path": out("export.skp") }),
            ),
            (
                "hew.doc.export",
                serde_json::json!({ "format": "skp2017", "path": out("export-mm.skp"), "units": "mm" }),
            ),
            add_a_far_box(),
            ("hew.doc.save", serde_json::json!({})),
            (
                "hew.doc.save",
                serde_json::json!({ "path": out("named.skp"), "units": "cm" }),
            ),
            ("hew.doc.new", serde_json::json!({})),
            add_a_far_box(),
            (
                "hew.doc.save",
                serde_json::json!({ "path": out("new.skp") }),
            ),
        ],
    );
    assert!(r.iter().all(|x| x.get("error").is_none()), "{r:#?}");
    assert_eq!(unit(&p("export.skp")), 4, "an export: meters");
    assert_eq!(unit(&p("export-mm.skp")), 2, "named: mm");
    assert_eq!(unit(&model), own, "a save back keeps the file's own");
    assert_eq!(unit(&p("named.skp")), 3, "named over the file's own: cm");
    assert_eq!(unit(&p("new.skp")), 4, "a new file: meters");
}

/// `hew.view.line_drawing`'s `selection` through the real CLI host: with
/// two boxes in the document, a drawing of one reports segments only from
/// that one, and fewer than the whole document's.
#[test]
fn a_line_drawing_of_a_selection_reports_segments_only_from_it() {
    use kernel::{EntityRef, NodeId, Plane, RenderRoot};
    let mut doc = Document::new();
    let ground = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut objects = Vec::new();
    for x0 in [0.0, 3.0] {
        let s = doc.add_sketch(ground);
        let sk = doc.sketch_mut(s).unwrap();
        let c = [
            Point3::new(x0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 1.0, 0.0),
            Point3::new(x0, 1.0, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        objects.push(doc.extrude_region(s, regions[0], 1.0).unwrap().0);
    }
    let public = |o| {
        let e = EntityRef::Object(o);
        api::ids::public_id(&e, doc.sid_of(&e).unwrap())
    };
    let mut host = CliHost::new();
    let mut params = api::LineDrawingParams {
        camera: None,
        view: Some(api::StandardView::Iso),
        scene: None,
        include_hidden: false,
        include_soft: false,
        format: api::LineDrawingFormat::Segments,
        scale: 1.0,
        path: None,
        selection: None,
    };
    let whole = host.line_drawing(&doc, &params).unwrap();
    let occ = doc.occurrence_at(&[], NodeId::Object(objects[1])).unwrap();
    params.selection = Some(vec![RenderRoot::Occurrence(occ)]);
    let one = host.line_drawing(&doc, &params).unwrap();
    assert!(one.count > 0 && one.count < whole.count);
    let picked = public(objects[1]);
    assert!(one.ids.iter().all(|id| *id == picked), "{:?}", one.ids);
    assert!(whole.ids.contains(&public(objects[0])));
    // A placement two entries reach is drawn once.
    let occ = doc.occurrence_at(&[], NodeId::Object(objects[1])).unwrap();
    params.selection = Some(vec![
        RenderRoot::Occurrence(occ.clone()),
        RenderRoot::Occurrence(occ),
    ]);
    let twice = host.line_drawing(&doc, &params).unwrap();
    assert_eq!(twice.count, one.count);
}

// ------------------------------------------------ renderer move stability

/// Two unit boxes, one at the origin and one three meters along x.
fn two_boxes() -> (Document, Vec<kernel::ObjectId>) {
    use kernel::Plane;
    let mut doc = Document::new();
    let ground = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut objects = Vec::new();
    for x0 in [0.0, 3.0] {
        let s = doc.add_sketch(ground);
        let sk = doc.sketch_mut(s).unwrap();
        let c = [
            Point3::new(x0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 1.0, 0.0),
            Point3::new(x0, 1.0, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        objects.push(doc.extrude_region(s, regions[0], 1.0).unwrap().0);
    }
    (doc, objects)
}

/// FNV-1a 64: a stable fingerprint for byte-exact pins.
fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn stability_snapshot_params() -> api::SnapshotParams {
    api::SnapshotParams {
        width: 128,
        height: 128,
        camera: None,
        view: Some(api::StandardView::Iso),
        scene: None,
        include_ids: false,
        include_depth: false,
        depth_bits: 8,
        include_normals: false,
        path: None,
        selection: None,
        background: softrender::BACKGROUND_RGBA,
    }
}

/// The renderer's output for a fixed document never changes by accident
/// (it moved crates once; any byte difference is a defect, not a re-pin).
#[test]
fn snapshot_bytes_are_stable() {
    let (doc, _) = two_boxes();
    let r = CliHost::new()
        .snapshot(&doc, &stability_snapshot_params())
        .unwrap();
    assert_eq!(
        (r.width, r.height, r.png.len(), fnv64(&r.png)),
        PIN_SNAPSHOT
    );
}

#[test]
fn line_drawing_segments_are_stable() {
    let (doc, _) = two_boxes();
    let params = api::LineDrawingParams {
        camera: None,
        view: Some(api::StandardView::Iso),
        scene: None,
        include_hidden: true,
        include_soft: false,
        format: api::LineDrawingFormat::Segments,
        scale: 1.0,
        path: None,
        selection: None,
    };
    let r = CliHost::new().line_drawing(&doc, &params).unwrap();
    let text = format!("{:?}{:?}{:?}", r.segments, r.kinds, r.bounds);
    assert_eq!((r.count, fnv64(text.as_bytes())), PIN_SEGMENTS);
}

const PIN_SNAPSHOT: (u32, u32, usize, u64) = (128, 128, 65737, 10977346622869604767);
const PIN_SEGMENTS: (usize, u64) = (24, 15869532378070628210);
