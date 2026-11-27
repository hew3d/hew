//! `hew-cli library index <folder> [--check] [flags]`
//! (docs/design/shared-library-sources.md 6.1, 6.4, 7.6). The oracle for
//! package S3: every spec here fails until the subcommand exists.
//!
//! Items are REAL Library items, made once per process by running
//! `hew.library.save` through `hew_cli::run::run_script` against a scratch
//! `HEW_LIBRARY_DIR`, then copied into a fresh folder per test. The binary
//! under test is run as a subprocess (`CARGO_BIN_EXE_hew-cli`).
//! `HEW_LIBRARY_DIR` is set once, inside the `OnceLock` initializer, and every
//! test calls `template()` first so nothing reads the environment while it is
//! being written.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

// ------------------------------------------------------------- fixtures

fn scratch_dir(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hew-cli-libindex-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_envelopes(dir: &Path, tag: &str, out: Option<&Path>, commands: serde_json::Value) {
    let script = dir.join(format!("{tag}.json"));
    let frames = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact",
          "params": { "label": tag, "commands": commands } },
    ]);
    std::fs::write(&script, serde_json::to_vec(&frames).unwrap()).unwrap();
    let outcome = hew_cli::run::run_script(&script, out, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "{tag}: {:?}",
        outcome.responses.last()
    );
}

fn boxed(size: f64) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({ "method": "hew.sketch.draw_rect", "as": "rect", "params": {
            "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [size, size, 0.0] } }),
        serde_json::json!({ "method": "hew.solid.extrude", "as": "box", "params": {
            "region": { "$ref": "rect#/region_id" }, "distance": size } }),
    ]
}

/// Draws a box and saves it as one Library item.
fn save_box(
    dir: &Path,
    name: &str,
    category: &str,
    keywords: &[&str],
    collection: Option<&str>,
    size: f64,
) {
    let mut cmds = boxed(size);
    let mut params = serde_json::json!({
        "selection": [{ "$ref": "box#/object_id" }],
        "name": name, "category": category, "keywords": keywords,
    });
    if let Some(c) = collection {
        params["collection"] = c.into();
    }
    cmds.push(serde_json::json!({ "method": "hew.library.save", "params": params }));
    run_envelopes(
        dir,
        &format!("save-{name}"),
        None,
        serde_json::Value::Array(cmds),
    );
}

fn hew_files(root: &Path) -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "hew") {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(root, &mut v);
    v.sort();
    v
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

struct Template {
    dir: PathBuf,
    /// Library-relative `/` path of the item whose changed bytes are in
    /// `changed_bytes` (same id, different geometry).
    changed_rel: String,
    changed_bytes: Vec<u8>,
    /// A Library item with no geometry: its thumbnail cannot be rendered.
    empty_item: Vec<u8>,
    /// A `.hew` with no `hew.library` attrs at all.
    plain_hew: Vec<u8>,
    /// `hew.library.list` of the five-item library, taken before the
    /// environment is pointed elsewhere.
    listing: Vec<serde_json::Value>,
}

fn template() -> &'static Template {
    static T: OnceLock<Template> = OnceLock::new();
    T.get_or_init(|| {
        let root = scratch_dir("template");
        let lib = root.join("library");
        std::fs::create_dir_all(&lib).unwrap();
        // SAFETY: every test calls `template()` before anything else, so this
        // runs while no other thread in the process touches the environment.
        unsafe {
            std::env::set_var("HEW_LIBRARY_DIR", &lib);
        }
        save_box(&root, "Alpha Box", "component", &["box", "alpha"], Some("Boxes"), 1.0);
        save_box(&root, "Beta Box", "component", &["box"], None, 2.0);
        save_box(&root, "Gamma Box", "model", &[], Some("Boxes"), 3.0);
        save_box(&root, "Delta Box", "component", &["delta", "square"], None, 0.5);
        save_box(&root, "Epsilon Box", "model", &["eps"], Some("Other"), 4.0);
        let files = hew_files(&lib);
        assert_eq!(files.len(), 5, "five items saved: {files:?}");
        // Snapshot the five items, then overwrite the first in place (same id).
        let dir = root.join("five");
        copy_tree(&lib, &dir);
        let first = files[0].clone();
        let rel = first.strip_prefix(&lib).unwrap().to_string_lossy().replace('\\', "/");
        // Find the id the first item carries via the library's own listing.
        let listing = lib_list(&root);
        let id = listing
            .iter()
            .find(|e| e["path"].as_str() == Some(rel.as_str()))
            .and_then(|e| e["id"].as_str())
            .unwrap_or_else(|| panic!("listing has {rel}: {listing:?}"))
            .to_string();
        let mut cmds = boxed(9.0);
        cmds.push(serde_json::json!({ "method": "hew.library.save", "params": {
            "selection": [{ "$ref": "box#/object_id" }], "name": "Alpha Box",
            "category": "component", "replace": id } }));
        run_envelopes(&root, "replace", None, serde_json::Value::Array(cmds));
        let changed_bytes = std::fs::read(&first).unwrap();
        assert_ne!(changed_bytes, std::fs::read(dir.join(&rel)).unwrap());

        // A Library item with nothing to render: an empty document saved whole.
        let empty_lib = root.join("empty-lib");
        std::fs::create_dir_all(&empty_lib).unwrap();
        unsafe {
            std::env::set_var("HEW_LIBRARY_DIR", &empty_lib);
        }
        run_envelopes(
            &root,
            "empty",
            None,
            serde_json::json!([{ "method": "hew.library.save", "params": { "name": "Empty Thing" } }]),
        );
        let empty_files = hew_files(&empty_lib);
        assert_eq!(empty_files.len(), 1, "{empty_files:?}");
        let empty_item = std::fs::read(&empty_files[0]).unwrap();

        // A plain model file: no hew.library attrs.
        let plain = root.join("plain.hew");
        run_envelopes(&root, "plain", Some(&plain), serde_json::Value::Array(boxed(1.0)));
        let plain_hew = std::fs::read(&plain).unwrap();

        Template { dir, changed_rel: rel, changed_bytes, empty_item, plain_hew, listing }
    })
}

/// `hew.library.list` over the template library, via one script.
fn lib_list(root: &Path) -> Vec<serde_json::Value> {
    let script = root.join("list.json");
    let frames = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.library.list", "params": {} },
    ]);
    std::fs::write(&script, serde_json::to_vec(&frames).unwrap()).unwrap();
    let outcome = hew_cli::run::run_script(&script, None, None);
    assert_eq!(outcome.exit_code, 0, "{:?}", outcome.responses.last());
    let r = &outcome.responses[2]["result"];
    r["items"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("list shape: {r}"))
}

/// A fresh copy of the five items as an index folder.
fn five() -> PathBuf {
    let t = template();
    let dir = scratch_dir("folder");
    copy_tree(&t.dir, &dir);
    dir
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hew-cli"))
        .args(args)
        .env_remove("HEW_LIBRARY_DIR")
        .output()
        .expect("hew-cli runs")
}

fn index(folder: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["library", "index", folder.to_str().unwrap()];
    args.extend_from_slice(extra);
    cli(&args)
}

fn text(o: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn ok(o: &Output) {
    assert!(
        o.status.success(),
        "expected success, got {:?}: {}",
        o.status,
        text(o)
    );
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every file under `root`: relative path -> (bytes, mtime).
fn snapshot(root: &Path) -> BTreeMap<String, (Vec<u8>, std::time::SystemTime)> {
    fn walk(root: &Path, d: &Path, out: &mut BTreeMap<String, (Vec<u8>, std::time::SystemTime)>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let mt = std::fs::metadata(&p).unwrap().modified().unwrap();
                out.insert(rel(root, &p), (std::fs::read(&p).unwrap(), mt));
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(root, root, &mut m);
    m
}

/// Asserts `bytes` is a PNG (signature, IHDR, IEND) and returns (w, h).
fn png_dims(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() > 33 + 12, "too short for a PNG");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&bytes[8..12], &13u32.to_be_bytes(), "IHDR length");
    assert_eq!(&bytes[12..16], b"IHDR");
    assert_eq!(
        &bytes[bytes.len() - 12..],
        b"\0\0\0\0IEND\xaeB`\x82",
        "IEND chunk ends the file"
    );
    let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    assert!(w > 0 && h > 0);
    (w, h)
}

fn index_json(folder: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(folder.join("hew.json")).expect("hew.json written"))
        .unwrap()
}

// ---------------------------------------------------------------- specs

#[test]
fn index_writes_hew_json_with_one_entry_per_item() {
    let _ = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test"]));
    let doc = index_json(&folder);
    assert_eq!(doc["hew"], 1);
    assert_eq!(doc["name"], "Test");
    let items = doc["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{doc}");
    let listing = template().listing.clone();
    let paths: Vec<&str> = items.iter().map(|i| i["path"].as_str().unwrap()).collect();
    let mut sorted = paths.clone();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(paths, sorted, "items sorted by path");
    let on_disk: Vec<String> = hew_files(&folder).iter().map(|p| rel(&folder, p)).collect();
    assert_eq!(
        paths,
        on_disk.iter().map(String::as_str).collect::<Vec<_>>()
    );
    for item in items {
        let path = item["path"].as_str().unwrap();
        assert!(!path.contains('\\') && !path.starts_with('/'), "{path}");
        let bytes = std::fs::read(folder.join(path)).unwrap();
        assert_eq!(item["sha256"], sha(&bytes).as_str(), "{path}");
        assert_eq!(item["bytes"], bytes.len() as u64, "{path}");
        let want = listing
            .iter()
            .find(|e| e["id"] == item["id"])
            .unwrap_or_else(|| panic!("an item with id {}", item["id"]));
        assert_eq!(item["name"], want["name"], "{path}");
        let cat = want["category"].as_str().unwrap();
        let want_cat = match cat {
            "component" => "Components",
            "model" => "Models",
            "material" => "Materials",
            "profile" => "Profiles",
            other => panic!("{other}"),
        };
        assert_eq!(item["category"], want_cat, "{path}");
        let kw: Vec<&str> = want["keywords"]
            .as_array()
            .map(|a| a.iter().filter_map(|k| k.as_str()).collect())
            .unwrap_or_default();
        let got: Vec<&str> = item["keywords"]
            .as_array()
            .map(|a| a.iter().filter_map(|k| k.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(got, kw, "{path}");
        match want["collection"].as_str() {
            Some(c) => assert_eq!(item["collection"], c, "{path}"),
            None => assert!(item["collection"].is_null(), "{path}"),
        }
    }
}

#[test]
fn index_writes_a_valid_png_thumbnail_per_item_named_by_its_sha256() {
    let _ = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test"]));
    let doc = index_json(&folder);
    for item in doc["items"].as_array().unwrap() {
        let sha = item["sha256"].as_str().unwrap();
        assert_eq!(item["thumbnail"], format!("thumbnails/{sha}.png").as_str());
        let png =
            std::fs::read(folder.join(format!("thumbnails/{sha}.png"))).expect("thumbnail file");
        // 256 px: the size the app renders Library thumbnails
        // (app/src/App.tsx and app/src/panels/LibraryDialog.tsx pass 256).
        assert_eq!(png_dims(&png), (256, 256));
    }
    let n = std::fs::read_dir(folder.join("thumbnails"))
        .unwrap()
        .count();
    assert_eq!(n, 5, "one thumbnail per item, no strays");
}

#[test]
fn written_index_reparses_with_the_sources_crate() {
    let _ = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test", "--license", "CC0-1.0"]));
    let bytes = std::fs::read(folder.join("hew.json")).unwrap();
    let parsed = sources::parse_index(&bytes).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(parsed.name, "Test");
    assert_eq!(parsed.license.as_deref(), Some("CC0-1.0"));
    assert_eq!(parsed.items.len(), 5);
    assert!(parsed.items.iter().all(|i| i.thumbnail.is_some()));
}

#[test]
fn check_exits_zero_on_a_fresh_index_and_writes_nothing() {
    let _ = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test"]));
    let before = snapshot(&folder);
    ok(&index(&folder, &["--check"]));
    assert_eq!(snapshot(&folder), before, "--check wrote nothing");
}

#[test]
fn check_names_a_changed_file_and_writes_nothing() {
    let t = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test"]));
    std::fs::write(folder.join(&t.changed_rel), &t.changed_bytes).unwrap();
    let before = snapshot(&folder);
    let out = index(&folder, &["--check"]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains(&t.changed_rel),
        "names the file: {}",
        text(&out)
    );
    assert_eq!(
        snapshot(&folder),
        before,
        "--check wrote nothing (contents and mtimes)"
    );
}

#[test]
fn check_names_an_added_file() {
    let t = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test"]));
    // A genuine new item: another Library item under a new name.
    let new_rel = "Extra/newcomer.hew";
    std::fs::create_dir_all(folder.join("Extra")).unwrap();
    std::fs::write(folder.join(new_rel), &t.empty_item).unwrap();
    let before = snapshot(&folder);
    let out = index(&folder, &["--check"]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("newcomer.hew"), "{}", text(&out));
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn check_names_a_missing_file() {
    let t = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test"]));
    std::fs::remove_file(folder.join(&t.changed_rel)).unwrap();
    let before = snapshot(&folder);
    let out = index(&folder, &["--check"]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains(&t.changed_rel), "{}", text(&out));
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn no_existing_index_and_no_name_is_an_error_naming_the_flag() {
    let _ = template();
    let folder = five();
    let out = index(&folder, &[]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("--name"), "{}", text(&out));
    assert!(!folder.join("hew.json").exists(), "nothing written");
    assert!(!folder.join("thumbnails").exists(), "nothing written");
}

#[test]
fn an_existing_header_is_kept_and_flags_override_it() {
    let _ = template();
    let folder = five();
    std::fs::write(
        folder.join("hew.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "hew": 1, "name": "Old Name", "description": "Old description",
            "author": "Old Author", "homepage": "https://example.com/old",
            "license": "LicenseRef-Mine", "licenseUrl": "https://example.com/license",
            "items": [], "plugins": [],
        }))
        .unwrap(),
    )
    .unwrap();
    ok(&index(&folder, &[]));
    let doc = index_json(&folder);
    assert_eq!(doc["name"], "Old Name");
    assert_eq!(doc["description"], "Old description");
    assert_eq!(doc["author"], "Old Author");
    assert_eq!(doc["homepage"], "https://example.com/old");
    assert_eq!(doc["license"], "LicenseRef-Mine");
    assert_eq!(doc["licenseUrl"], "https://example.com/license");
    assert_eq!(doc["items"].as_array().unwrap().len(), 5);

    ok(&index(
        &folder,
        &[
            "--name",
            "New Name",
            "--description",
            "New description",
            "--author",
            "New Author",
            "--homepage",
            "https://example.com/new",
            "--license",
            "MIT",
            "--license-url",
            "https://example.com/mit",
        ],
    ));
    let doc = index_json(&folder);
    assert_eq!(doc["name"], "New Name");
    assert_eq!(doc["description"], "New description");
    assert_eq!(doc["author"], "New Author");
    assert_eq!(doc["homepage"], "https://example.com/new");
    assert_eq!(doc["license"], "MIT");
    assert_eq!(doc["licenseUrl"], "https://example.com/mit");

    // A single flag overrides only its own field.
    ok(&index(&folder, &["--author", "Third"]));
    let doc = index_json(&folder);
    assert_eq!(doc["author"], "Third");
    assert_eq!(doc["name"], "New Name");
    assert_eq!(doc["license"], "MIT");
}

#[test]
fn a_file_with_no_library_id_is_refused_by_name_and_nothing_is_written() {
    let t = template();
    let folder = five();
    std::fs::write(folder.join("not-an-item.hew"), &t.plain_hew).unwrap();
    let out = index(&folder, &["--name", "Test"]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("not-an-item.hew"), "{}", text(&out));
    assert!(!folder.join("hew.json").exists(), "no hew.json written");
}

#[test]
fn an_item_whose_thumbnail_cannot_render_is_indexed_without_one_and_warned_about() {
    let t = template();
    let folder = five();
    std::fs::write(folder.join("empty-thing.hew"), &t.empty_item).unwrap();
    let out = index(&folder, &["--name", "Test"]);
    ok(&out);
    let doc = index_json(&folder);
    let items = doc["items"].as_array().unwrap();
    assert_eq!(items.len(), 6);
    let empty = items
        .iter()
        .find(|i| i["path"] == "empty-thing.hew")
        .expect("indexed");
    assert!(
        empty.get("thumbnail").is_none_or(|v| v.is_null()),
        "{empty}"
    );
    assert!(
        items
            .iter()
            .filter(|i| i["path"] != "empty-thing.hew")
            .all(|i| i["thumbnail"].is_string())
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("empty-thing.hew"),
        "warning on stderr names it: {err}"
    );
    // It still parses and --check is clean.
    sources::parse_index(&std::fs::read(folder.join("hew.json")).unwrap()).unwrap();
    ok(&index(&folder, &["--check"]));
}

#[test]
fn rerunning_on_an_unchanged_folder_is_byte_identical() {
    let _ = template();
    let folder = five();
    ok(&index(&folder, &["--name", "Test", "--author", "Someone"]));
    let first = snapshot(&folder);
    ok(&index(&folder, &[]));
    let second = snapshot(&folder);
    let strip =
        |m: &BTreeMap<String, (Vec<u8>, std::time::SystemTime)>| -> BTreeMap<String, Vec<u8>> {
            m.iter().map(|(k, v)| (k.clone(), v.0.clone())).collect()
        };
    assert_eq!(
        strip(&first),
        strip(&second),
        "hew.json and thumbnails identical"
    );
    assert!(first.contains_key("hew.json"));
}

#[test]
fn dotfiles_hew_json_and_thumbnails_are_not_indexed_as_items() {
    let t = template();
    let folder = five();
    let some_item = hew_files(&folder).remove(0);
    let bytes = std::fs::read(&some_item).unwrap();
    std::fs::write(folder.join(".hidden.hew"), &bytes).unwrap();
    std::fs::create_dir_all(folder.join(".git")).unwrap();
    std::fs::write(folder.join(".git/inside.hew"), &bytes).unwrap();
    std::fs::create_dir_all(folder.join("thumbnails")).unwrap();
    std::fs::write(folder.join("thumbnails/stray.hew"), &t.plain_hew).unwrap();
    std::fs::write(folder.join("notes.txt"), b"not an item").unwrap();
    ok(&index(&folder, &["--name", "Test"]));
    let doc = index_json(&folder);
    let items = doc["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{doc}");
    for i in items {
        let p = i["path"].as_str().unwrap();
        assert!(
            !p.starts_with('.') && !p.contains("/.") && !p.starts_with("thumbnails/"),
            "{p}"
        );
    }
    // A second run (hew.json and thumbnails now exist) still sees five.
    ok(&index(&folder, &[]));
    assert_eq!(index_json(&folder)["items"].as_array().unwrap().len(), 5);
    ok(&index(&folder, &["--check"]));
}

#[test]
fn library_with_no_arguments_prints_usage_and_exits_nonzero() {
    let _ = template();
    let out = cli(&["library"]);
    assert!(!out.status.success());
    let t = text(&out);
    assert!(
        t.contains("library index"),
        "usage names the subcommand: {t}"
    );
    assert!(
        !t.contains("unknown command"),
        "`library` is a known command: {t}"
    );
}

#[test]
fn top_level_help_lists_library_index() {
    let _ = template();
    let out = cli(&["--help"]);
    ok(&out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("library index"),
        "{}",
        text(&out)
    );
}
