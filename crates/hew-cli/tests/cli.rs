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
/// docs/HEW_API.md §6.2), then `--out`-saves. The resulting `.hew` file
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
/// (docs/HEW_API.md §12) rather than attempted and left to fail on the
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

// ------------------------------------------------------------------ export

/// `CliHost::export_document("stl")` over a real box document: a
/// well-formed binary STL — 80-byte header + `u32` count, then 50 bytes
/// per triangle, and a box triangulates to 12 (2 per face).
#[test]
fn cli_host_exports_a_box_to_well_formed_binary_stl() {
    let doc = box_document();
    let mut host = CliHost::new();
    let bytes = host
        .export_document(&doc, "stl", None, 0)
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
        .export_document(&doc, "stl", Some(out.to_str().unwrap()), 0)
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
    let err = host.export_document(&doc, "stl", None, 0).unwrap_err();
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
        // renumbers dense ids per save (docs/HEW_API.md §5.1), so a fresh
        // `box_document()` ingest need not iterate faces/vertices in the
        // same order a reloaded copy does, even though both describe the
        // same box.
        let reloaded = Document::load(&std::fs::read(&path).unwrap()).expect("reloads");
        let mut host = CliHost::new();
        let direct_bytes = host
            .export_document(&reloaded, format, None, 0)
            .unwrap_or_else(|e| panic!("{format}: direct export succeeds: {e:?}"))
            .unwrap_or_else(|| panic!("{format}: bytes returned inline"));

        assert_eq!(
            dispatched_bytes, direct_bytes,
            "{format}: dispatcher bytes must match a direct export"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
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
            &serde_json::json!({ "units": "m" }),
        )
        .expect("a valid tetrahedron STL imports");

    assert_eq!(report["objects_created"], 1);
    assert_eq!(doc.visible_object_ids().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// STL carries no units of its own — importing without `units` refuses
/// typed rather than guessing (docs/HEW_API.md §7's semantics note).
#[test]
fn cli_host_import_refuses_units_required_without_units() {
    let dir = scratch_dir("import-nounits");
    let path = dir.join("tet.stl");
    std::fs::write(&path, tetrahedron_stl_bytes()).unwrap();

    let mut doc = Document::new();
    let mut host = CliHost::new();
    let err = host
        .import_document(&mut doc, path.to_str().unwrap(), &serde_json::json!({}))
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

/// The whole point of `hew.view.snapshot`'s `path` parameter (docs/HEW_API.md
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
        path: None,
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
    };
    doc.ingest(import, Vec::new()).expect("ingest two boxes");
    let ids = doc.visible_object_ids();
    let (a, b) = (ids[0], ids[1]);

    // Capture a Scene with B hidden and an explicit camera, THEN unhide B
    // live — the snapshot must still honor the Scene's own captured
    // state, not the document's current one.
    doc.set_node_user_hidden(kernel::NodeId::Object(b), true);
    let camera = kernel::CameraState {
        projection: kernel::CameraProjection::Perspective,
        fov_deg: 40.0,
        eye: kernel::Point3::new(5.0, 0.0, 0.0),
        target: kernel::Point3::ORIGIN,
        up: kernel::Vec3::new(0.0, 0.0, 1.0),
    };
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
        path: None,
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
    };
    doc.ingest(import, Vec::new()).expect("ingest two boxes");
    let ids = doc.visible_object_ids();
    let (a, b) = (ids[0], ids[1]);
    let camera = kernel::CameraState {
        projection: kernel::CameraProjection::Perspective,
        fov_deg: 40.0,
        eye: kernel::Point3::new(5.0, 0.0, 0.0),
        target: kernel::Point3::ORIGIN,
        up: kernel::Vec3::new(0.0, 0.0, 1.0),
    };
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
        path: None,
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
        path: None,
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
