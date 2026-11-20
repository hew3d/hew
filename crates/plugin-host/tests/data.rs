//! What a plugin keeps beside its package, in a temporary folder: where
//! the tree lives, per-document storage and its refusals, the one quota for
//! the tree and its running total, links refused and never followed,
//! uninstall removing the tree first, version swaps leaving it alone, the
//! activity log and the console with its rotation.

#![cfg(all(hew_preview, feature = "fs"))]

mod common;

use common::*;
use plugin_host::data::{self, ActivityEntry, ConsoleLine};
use plugin_host::store::{self, PluginDir};
use plugin_host::{InstallError, InstallSource, Package, read_package_zip};
use serde_json::json;
use std::path::{Path, PathBuf};

/// A scratch folder removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("hew-plugin-data-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn store(&self) -> PluginDir {
        PluginDir::at(self.0.join("plugins"))
    }

    fn tree(&self, id: &str) -> PathBuf {
        self.0.join("plugin-data").join(id)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                self.0.join("plugin-data"),
                std::fs::Permissions::from_mode(0o755),
            );
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ID: &str = "com.example.fixture";
const DOC: &str = "3f2c9107-5ed4-40a8-806b-1ec297405d83";
const DOC2: &str = "00000000-0000-4000-8000-000000000002";

fn running() -> semver::Version {
    semver::Version::parse("1.4.0").unwrap()
}

fn package(id: &str, version: &str) -> Package {
    let m = json!({
        "manifest": 1, "id": id, "name": "Fixture", "version": version, "api": 1,
        "main": "dist/background.js", "permissions": ["model.read"]
    });
    let entries: Vec<(String, Vec<u8>)> = vec![
        ("hew-plugin.json".into(), m.to_string().into_bytes()),
        (
            "dist/background.js".into(),
            format!("// {version}").into_bytes(),
        ),
    ];
    read_package_zip(&stored_zip(&entries)).expect("the fixture parses")
}

fn install(dir: &PluginDir, id: &str, version: &str) {
    let source = InstallSource::File {
        name: "fixture.hewplugin".into(),
    };
    let pending = store::stage(dir, package(id, version), source, &running(), 1_000).unwrap();
    store::commit(dir, pending, true, 2_000).unwrap().unwrap();
}

/// Every file under `root` with its bytes, sorted by relative path.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in listing.flatten() {
            let path = e.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if e.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// What the tree holds, as the quota counts it.
fn measured(tree: &Path) -> u64 {
    snapshot(tree)
        .iter()
        .filter(|(rel, _)| !rel.starts_with("usage.json"))
        .map(|(_, b)| (b.len() as u64).max(data::MIN_FILE_COST))
        .sum()
}

fn tally(tree: &Path) -> u64 {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tree.join("usage.json")).unwrap()).unwrap();
    v["bytes"].as_u64().unwrap()
}

fn unreadable<T>() -> Result<T, InstallError> {
    Err(InstallError::StorageUnreadable)
}

fn line(level: &str, text: &str, at: u64) -> ConsoleLine {
    ConsoleLine {
        at,
        level: level.into(),
        text: text.into(),
    }
}

fn entry(window: &str, state: &str, at: u64) -> ActivityEntry {
    ActivityEntry {
        at,
        window: window.into(),
        state: state.into(),
        reason: None,
    }
}

#[test]
fn the_tree_lives_beside_the_store() {
    let scratch = Scratch::new("where");
    assert_eq!(
        scratch.store().data_root(),
        Some(scratch.0.join("plugin-data"))
    );
    // A store under another name gets a tree of its own beside it.
    let other = PluginDir::at(scratch.0.join("second"));
    assert_eq!(
        other.data_root(),
        Some(scratch.0.join("second.plugin-data"))
    );
    assert_eq!(PluginDir::at(PathBuf::from("/")).data_root(), None);
}

#[test]
fn document_storage_saves_reads_lists_and_deletes() {
    let scratch = Scratch::new("doc");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    assert_eq!(data::doc_storage_get(&dir, ID, DOC, "a"), Ok(None));
    assert_eq!(data::doc_storage_list(&dir, ID, DOC), Ok(vec![]));
    // Reading makes nothing.
    assert!(!scratch.0.join("plugin-data").exists());

    data::doc_storage_set(&dir, ID, DOC, "b", "2").unwrap();
    data::doc_storage_set(&dir, ID, DOC, "a", "{\"x\":1}").unwrap();
    data::doc_storage_set(&dir, ID, DOC2, "a", "other").unwrap();
    assert_eq!(
        data::doc_storage_get(&dir, ID, DOC, "a")
            .unwrap()
            .as_deref(),
        Some("{\"x\":1}")
    );
    assert_eq!(data::doc_storage_list(&dir, ID, DOC).unwrap(), ["a", "b"]);
    assert_eq!(data::doc_storage_list(&dir, ID, DOC2).unwrap(), ["a"]);
    // One JSON object of strings per (plugin, document).
    let file = scratch
        .tree(ID)
        .join("documents")
        .join(format!("{DOC}.json"));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(value, json!({ "a": "{\"x\":1}", "b": "2" }));
    // Plugin storage is a separate thing.
    assert_eq!(store::storage_list(&dir, ID).unwrap(), Vec::<String>::new());

    assert_eq!(data::doc_storage_delete(&dir, ID, DOC, "a"), Ok(true));
    assert_eq!(data::doc_storage_delete(&dir, ID, DOC, "a"), Ok(false));
    // The file goes with its last key.
    assert_eq!(data::doc_storage_delete(&dir, ID, DOC, "b"), Ok(true));
    assert!(!file.exists());
    // No temporary file is left behind.
    let names: Vec<String> = snapshot(&scratch.tree(ID))
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        names,
        [format!("documents/{DOC2}.json"), "usage.json".into()]
    );
}

#[test]
fn document_storage_checks_the_ids_and_the_key_before_any_path() {
    let scratch = Scratch::new("doc-ids");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    for bad in [
        "",
        "../x",
        "3F2C9107-5ED4-40A8-806B-1EC297405D83",
        "3f2c9107-5ed4-40a8-806b-1ec297405d83/../../x",
        "../../plugins/com.example.fixture/installed",
    ] {
        assert_eq!(
            data::doc_storage_set(&dir, ID, bad, "a", "1"),
            Err(InstallError::DocumentIdInvalid),
            "{bad:?}"
        );
        assert_eq!(
            data::doc_storage_get(&dir, ID, bad, "a"),
            Err(InstallError::DocumentIdInvalid)
        );
        assert_eq!(
            data::doc_storage_list(&dir, ID, bad),
            Err(InstallError::DocumentIdInvalid)
        );
        assert_eq!(
            data::doc_storage_delete(&dir, ID, bad, "a"),
            Err(InstallError::DocumentIdInvalid)
        );
    }
    // A bad plugin id is refused by the grammar first.
    for bad in [
        "../x",
        "a",
        "hew.x",
        "",
        "con.example",
        "com1.tools",
        "lpt9.x",
        "nul.b",
    ] {
        assert!(matches!(
            data::doc_storage_set(&dir, bad, DOC, "a", "1"),
            Err(InstallError::Manifest(_))
        ));
    }
    let long = "k".repeat(plugin_host::MAX_STORAGE_KEY_BYTES + 1);
    for key in ["", long.as_str()] {
        assert_eq!(
            data::doc_storage_set(&dir, ID, DOC, key, "v"),
            Err(InstallError::StorageKeyInvalid)
        );
        assert_eq!(
            data::doc_storage_get(&dir, ID, DOC, key),
            Err(InstallError::StorageKeyInvalid)
        );
    }
    assert!(!scratch.0.join("plugin-data").exists());
    // Keys are only map keys.
    data::doc_storage_set(&dir, ID, DOC, "../../installed.json", "v").unwrap();
    assert_eq!(
        data::doc_storage_list(&dir, ID, DOC).unwrap(),
        ["../../installed.json"]
    );
}

#[test]
fn every_write_needs_an_installed_plugin_and_creates_nothing_for_one_that_is_not() {
    let scratch = Scratch::new("doc-missing");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    let other = "com.example.missing";
    let not = || InstallError::NotInstalled(other.into());
    assert_eq!(
        data::doc_storage_set(&dir, other, DOC, "a", "1"),
        Err(not())
    );
    assert_eq!(data::doc_storage_get(&dir, other, DOC, "a"), Err(not()));
    assert_eq!(data::doc_storage_list(&dir, other, DOC), Err(not()));
    assert_eq!(data::doc_storage_delete(&dir, other, DOC, "a"), Err(not()));
    assert_eq!(
        data::activity_append(&dir, other, &[entry("main", "activated", 1)]),
        Err(not())
    );
    assert_eq!(data::activity_read(&dir, other), Err(not()));
    assert_eq!(
        data::console_append(&dir, other, &[line("log", "hi", 1)]),
        Err(not())
    );
    assert_eq!(data::console_read(&dir, other, 10), Err(not()));
    assert!(!scratch.0.join("plugin-data").exists());
    // A folder with no record is not a plugin either.
    std::fs::create_dir_all(dir.root().join(other)).unwrap();
    assert_eq!(
        data::doc_storage_set(&dir, other, DOC, "a", "1"),
        Err(not())
    );
    assert!(!scratch.tree(other).exists());
}

#[test]
fn one_document_is_refused_past_its_quota_and_writes_nothing() {
    let scratch = Scratch::new("doc-quota");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    let cap = data::MAX_DOCUMENT_STORAGE_BYTES;
    assert_eq!(cap, 5 * 1024 * 1024);
    assert_eq!(
        data::doc_storage_set(&dir, ID, DOC, "a", &"x".repeat(cap)),
        Err(InstallError::StorageQuota)
    );
    let half = "x".repeat(cap / 2);
    data::doc_storage_set(&dir, ID, DOC, "a", &half).unwrap();
    let file = scratch
        .tree(ID)
        .join("documents")
        .join(format!("{DOC}.json"));
    let before = std::fs::read(&file).unwrap();
    assert_eq!(
        data::doc_storage_set(&dir, ID, DOC, "b", &half),
        Err(InstallError::StorageQuota)
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn the_whole_tree_is_refused_past_its_quota_and_the_total_fails_safe() {
    let scratch = Scratch::new("tree-quota");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    assert_eq!(plugin_host::MAX_PLUGIN_DATA_BYTES, 50 * 1024 * 1024);
    let tree = scratch.tree(ID);
    let big = "x".repeat(data::MAX_DOCUMENT_STORAGE_BYTES - 100);
    let doc = |n: u32| format!("00000000-0000-4000-8000-{n:012}");
    // Ten nearly full documents fit; the eleventh does not.
    for n in 0..10 {
        data::doc_storage_set(&dir, ID, &doc(n), "a", &big).unwrap();
        assert!(tally(&tree) >= measured(&tree), "the total covers the disk");
    }
    assert_eq!(tally(&tree), measured(&tree));
    let refused = data::doc_storage_set(&dir, ID, &doc(10), "a", &big);
    assert_eq!(refused, Err(InstallError::DataQuota));
    assert_eq!(refused.unwrap_err().name(), "storage_quota");
    assert!(
        !tree
            .join("documents")
            .join(format!("{}.json", doc(10)))
            .exists()
    );
    // A small write is refused too: the tree is full.
    assert_eq!(
        data::doc_storage_set(&dir, ID, &doc(10), "a", &"y".repeat(9000)),
        Err(InstallError::DataQuota)
    );
    // A missing total is measured again, and still refuses.
    std::fs::remove_file(tree.join("usage.json")).unwrap();
    assert_eq!(
        data::doc_storage_set(&dir, ID, &doc(10), "a", &big),
        Err(InstallError::DataQuota)
    );
    // So is one that is not a total at all, or a link (never followed).
    std::fs::write(tree.join("usage.json"), b"{\"bytes\":\"0\"}").unwrap();
    assert_eq!(
        data::doc_storage_set(&dir, ID, &doc(10), "a", &big),
        Err(InstallError::DataQuota)
    );
    #[cfg(unix)]
    {
        let outside = scratch.0.join("outside-usage.json");
        std::fs::write(&outside, b"{\"bytes\":0}").unwrap();
        std::fs::remove_file(tree.join("usage.json")).unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("usage.json")).unwrap();
        assert_eq!(
            data::doc_storage_set(&dir, ID, &doc(10), "a", &big),
            Err(InstallError::DataQuota)
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"{\"bytes\":0}");
        // A link there is never rewritten: writes are refused until it goes.
        assert_eq!(
            data::doc_storage_delete(&dir, ID, &doc(0), "a"),
            Err(InstallError::StorageUnreadable)
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"{\"bytes\":0}");
        std::fs::remove_file(tree.join("usage.json")).unwrap();
    }
    // Shrinking is never refused, and makes room.
    assert_eq!(data::doc_storage_delete(&dir, ID, &doc(0), "a"), Ok(true));
    data::doc_storage_set(&dir, ID, &doc(10), "a", &big).unwrap();
    assert_eq!(tally(&tree), measured(&tree));
    // A total left too high (a write cut short) refuses nothing that fits:
    // the tree is measured before a refusal.
    data::doc_storage_delete(&dir, ID, &doc(1), "a").unwrap();
    std::fs::write(tree.join("usage.json"), b"{\"bytes\":999999999999}").unwrap();
    data::doc_storage_set(&dir, ID, &doc(1), "a", "small").unwrap();
    assert_eq!(tally(&tree), measured(&tree));
}

#[test]
fn the_console_and_the_activity_count_toward_the_quota() {
    let scratch = Scratch::new("tree-quota-logs");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    let tree = scratch.tree(ID);
    data::console_append(&dir, ID, &[line("log", "one", 1)]).unwrap();
    data::activity_append(&dir, ID, &[entry("main", "activated", 1)]).unwrap();
    data::doc_storage_set(&dir, ID, DOC, "a", "1").unwrap();
    assert_eq!(tally(&tree), measured(&tree));
    assert_eq!(tally(&tree), 3 * data::MIN_FILE_COST);
}

#[test]
fn hews_own_records_are_never_refused_by_a_full_tree_but_still_count() {
    let scratch = Scratch::new("tree-quota-own");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    let tree = scratch.tree(ID);
    let big = "x".repeat(data::MAX_DOCUMENT_STORAGE_BYTES - 100);
    let doc = |n: u32| format!("00000000-0000-4000-8000-{n:012}");
    for n in 0..10 {
        data::doc_storage_set(&dir, ID, &doc(n), "a", &big).unwrap();
    }
    // The tree is full: a document write is refused ...
    assert_eq!(
        data::doc_storage_set(&dir, ID, &doc(10), "a", &"y".repeat(9000)),
        Err(InstallError::DataQuota)
    );
    // ... but the crash record and the console are not.
    data::activity_append(&dir, ID, &[entry("main", "crashed", 5)]).unwrap();
    assert_eq!(data::activity_read(&dir, ID).unwrap()[0].state, "crashed");
    assert_eq!(
        data::console_append(&dir, ID, &[line("error", "boom", 6)]),
        Ok(1)
    );
    assert_eq!(data::console_read(&dir, ID, 10).unwrap()[0].text, "boom");
    // They still count: the total covers the disk, and documents stay refused.
    assert!(tally(&tree) >= measured(&tree));
    assert_eq!(
        data::doc_storage_set(&dir, ID, &doc(10), "a", &"y".repeat(9000)),
        Err(InstallError::DataQuota)
    );
}

#[cfg(unix)]
#[test]
fn links_in_the_tree_are_refused_never_followed_never_rewritten() {
    use std::os::unix::fs::symlink;
    let scratch = Scratch::new("links");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    let outside = scratch.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.json");
    std::fs::write(&secret, b"{\"a\":\"secret\"}").unwrap();

    // The plugin's tree itself is a link.
    std::fs::create_dir_all(scratch.0.join("plugin-data")).unwrap();
    symlink(&outside, scratch.tree(ID)).unwrap();
    assert_eq!(data::doc_storage_set(&dir, ID, DOC, "a", "1"), unreadable());
    assert_eq!(data::doc_storage_get(&dir, ID, DOC, "a"), unreadable());
    assert_eq!(
        data::console_append(&dir, ID, &[line("log", "x", 1)]),
        unreadable()
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    std::fs::remove_file(scratch.tree(ID)).unwrap();

    // `documents/` is a link.
    std::fs::create_dir_all(scratch.tree(ID)).unwrap();
    symlink(&outside, scratch.tree(ID).join("documents")).unwrap();
    assert_eq!(data::doc_storage_set(&dir, ID, DOC, "a", "1"), unreadable());
    assert_eq!(data::doc_storage_list(&dir, ID, DOC), unreadable());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    std::fs::remove_file(scratch.tree(ID).join("documents")).unwrap();

    // A document's file is a link.
    std::fs::create_dir_all(scratch.tree(ID).join("documents")).unwrap();
    let file = scratch
        .tree(ID)
        .join("documents")
        .join(format!("{DOC}.json"));
    symlink(&secret, &file).unwrap();
    assert_eq!(data::doc_storage_get(&dir, ID, DOC, "a"), unreadable());
    assert_eq!(data::doc_storage_set(&dir, ID, DOC, "a", "1"), unreadable());
    assert_eq!(data::doc_storage_delete(&dir, ID, DOC, "a"), unreadable());
    assert_eq!(std::fs::read(&secret).unwrap(), b"{\"a\":\"secret\"}");
    std::fs::remove_file(&file).unwrap();

    // A link where a write's temporary file goes is removed, not followed.
    symlink(
        &secret,
        scratch
            .tree(ID)
            .join("documents")
            .join(format!("{DOC}.json.tmp")),
    )
    .unwrap();
    data::doc_storage_set(&dir, ID, DOC, "a", "1").unwrap();
    assert_eq!(std::fs::read(&secret).unwrap(), b"{\"a\":\"secret\"}");
    assert!(
        std::fs::symlink_metadata(&file)
            .unwrap()
            .file_type()
            .is_file()
    );

    // The console's files and the activity log are links.
    symlink(&secret, scratch.tree(ID).join("console.log")).unwrap();
    assert_eq!(
        data::console_append(&dir, ID, &[line("log", "x", 1)]),
        unreadable()
    );
    assert_eq!(data::console_read(&dir, ID, 10), unreadable());
    std::fs::remove_file(scratch.tree(ID).join("console.log")).unwrap();
    symlink(&secret, scratch.tree(ID).join("activity.json")).unwrap();
    assert_eq!(
        data::activity_append(&dir, ID, &[entry("main", "crashed", 1)]),
        unreadable()
    );
    assert_eq!(data::activity_read(&dir, ID), unreadable());
    assert_eq!(std::fs::read(&secret).unwrap(), b"{\"a\":\"secret\"}");
    std::fs::remove_file(scratch.tree(ID).join("activity.json")).unwrap();

    // `plugin-data/` itself is a link: nothing is written through it, and
    // an uninstall removes nothing through it.
    std::fs::remove_dir_all(scratch.0.join("plugin-data")).unwrap();
    std::fs::create_dir_all(outside.join(ID)).unwrap();
    std::fs::write(outside.join(ID).join("keep"), b"keep").unwrap();
    symlink(&outside, scratch.0.join("plugin-data")).unwrap();
    assert_eq!(data::doc_storage_set(&dir, ID, DOC, "a", "1"), unreadable());
    store::uninstall(&dir, ID).unwrap();
    assert_eq!(
        std::fs::read(outside.join(ID).join("keep")).unwrap(),
        b"keep"
    );
    assert_eq!(std::fs::read(&secret).unwrap(), b"{\"a\":\"secret\"}");
}

#[test]
fn uninstall_and_purge_remove_the_tree_and_a_new_install_starts_empty() {
    let scratch = Scratch::new("uninstall");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    install(&dir, "com.example.other", "1.0.0");
    data::doc_storage_set(&dir, ID, DOC, "a", "1").unwrap();
    data::console_append(&dir, ID, &[line("warn", "x", 1)]).unwrap();
    data::activity_append(&dir, ID, &[entry("main", "crashed", 1)]).unwrap();
    data::doc_storage_set(&dir, "com.example.other", DOC, "a", "1").unwrap();
    store::uninstall(&dir, ID).unwrap();
    assert!(!scratch.tree(ID).exists());
    // Another plugin's tree is left alone.
    assert_eq!(
        data::doc_storage_list(&dir, "com.example.other", DOC).unwrap(),
        ["a"]
    );
    // A different package under the same id inherits nothing.
    install(&dir, ID, "1.0.0");
    assert_eq!(
        data::doc_storage_list(&dir, ID, DOC).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(data::console_read(&dir, ID, 10).unwrap(), vec![]);
    assert_eq!(data::activity_read(&dir, ID).unwrap(), vec![]);
    data::doc_storage_set(&dir, ID, DOC, "a", "1").unwrap();
    store::purge(&dir, ID).unwrap();
    assert!(!scratch.tree(ID).exists());
    assert!(!dir.root().join(ID).exists());
}

#[cfg(unix)]
#[test]
fn an_uninstall_that_cannot_remove_the_data_leaves_the_plugin_installed() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("uninstall-order");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    data::doc_storage_set(&dir, ID, DOC, "a", "1").unwrap();
    // `plugin-data/` that cannot be changed: its entries cannot be removed.
    let root = scratch.0.join("plugin-data");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = store::uninstall(&dir, ID);
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    if result.is_ok() {
        // Permissions do not bind this user (a superuser): nothing to show.
        return;
    }
    assert_eq!(result.unwrap_err().name(), "io");
    // The record is still there, so the plugin can be removed again: no
    // data is left for a plugin that is gone.
    assert!(store::installed(&dir, ID).is_ok());
    assert!(dir.root().join(ID).join("installed.json").is_file());
    store::uninstall(&dir, ID).unwrap();
    assert!(!scratch.tree(ID).exists());
}

#[test]
fn installs_updates_and_rollbacks_never_touch_the_tree() {
    let scratch = Scratch::new("versions");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    // An install makes no tree.
    assert!(!scratch.0.join("plugin-data").exists());
    data::doc_storage_set(&dir, ID, DOC, "a", "1").unwrap();
    data::console_append(&dir, ID, &[line("info", "hello", 1)]).unwrap();
    data::activity_append(&dir, ID, &[entry("main", "activated", 1)]).unwrap();
    let before = snapshot(&scratch.0.join("plugin-data"));
    install(&dir, ID, "2.0.0");
    assert_eq!(
        snapshot(&scratch.0.join("plugin-data")),
        before,
        "an update"
    );
    store::rollback(&dir, ID, &running()).unwrap();
    assert_eq!(
        snapshot(&scratch.0.join("plugin-data")),
        before,
        "a rollback"
    );
    install(&dir, ID, "1.0.0");
    assert_eq!(
        snapshot(&scratch.0.join("plugin-data")),
        before,
        "a reinstall"
    );
    // A pending install that is turned down or has expired changes nothing.
    let source = InstallSource::File { name: "x".into() };
    let pending = store::stage(&dir, package(ID, "3.0.0"), source, &running(), 1_000).unwrap();
    assert!(store::commit(&dir, pending, true, 1_000 + store::PENDING_TTL_MS + 1).is_err());
    assert_eq!(snapshot(&scratch.0.join("plugin-data")), before);
    // The data stays the plugin's.
    assert_eq!(data::doc_storage_list(&dir, ID, DOC).unwrap(), ["a"]);
}

#[test]
fn activity_keeps_the_last_entries_newest_first() {
    let scratch = Scratch::new("activity");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    let batch: Vec<ActivityEntry> = (0..520)
        .map(|n| entry(if n % 2 == 0 { "main" } else { "main-2" }, "activated", n))
        .collect();
    data::activity_append(&dir, ID, &batch[..300]).unwrap();
    data::activity_append(&dir, ID, &batch[300..]).unwrap();
    let read = data::activity_read(&dir, ID).unwrap();
    assert_eq!(read.len(), data::MAX_ACTIVITY_ENTRIES);
    assert_eq!(read[0].at, 519, "newest first");
    assert_eq!(read[499].at, 20);
    // Entries that break the rules are left out; reasons are cleaned.
    let mut crashed = entry("main", "crashed", 600);
    crashed.reason = Some("  out \u{202e}of\n time ".into());
    data::activity_append(
        &dir,
        ID,
        &[
            entry("main", "exploded", 601),
            entry("main window", "idle", 602),
            crashed,
        ],
    )
    .unwrap();
    let read = data::activity_read(&dir, ID).unwrap();
    assert_eq!(read[0].at, 600);
    assert_eq!(read[0].reason.as_deref(), Some("out of time"));
    assert_eq!(read.len(), data::MAX_ACTIVITY_ENTRIES);
    // A damaged log is read as empty and replaced by the next append.
    std::fs::write(scratch.tree(ID).join("activity.json"), b"not json").unwrap();
    assert_eq!(data::activity_read(&dir, ID).unwrap(), vec![]);
    data::activity_append(&dir, ID, &[entry("main", "idle", 700)]).unwrap();
    assert_eq!(data::activity_read(&dir, ID).unwrap().len(), 1);
}

#[test]
fn the_console_appends_cleans_reads_newest_first_and_rotates_once() {
    let scratch = Scratch::new("console");
    let dir = scratch.store();
    install(&dir, ID, "1.0.0");
    assert_eq!(data::console_read(&dir, ID, 10).unwrap(), vec![]);
    let written = data::console_append(
        &dir,
        ID,
        &[
            line("log", "one", 1),
            line("shout", "dropped", 2),
            line("error", "two\n\u{202e}lines", 3),
        ],
    )
    .unwrap();
    assert_eq!(written, 2);
    let read = data::console_read(&dir, ID, 10).unwrap();
    assert_eq!(read, [line("error", "two lines", 3), line("log", "one", 1)]);
    assert_eq!(data::console_read(&dir, ID, 1).unwrap().len(), 1);
    // At most so many lines per append.
    let many: Vec<ConsoleLine> = (0..100).map(|n| line("info", "x", n)).collect();
    assert_eq!(
        data::console_append(&dir, ID, &many).unwrap(),
        data::MAX_CONSOLE_LINES_PER_APPEND
    );
    // A long text is cut.
    data::console_append(&dir, ID, &[line("log", &"y".repeat(10_000), 9)]).unwrap();
    let last = &data::console_read(&dir, ID, 1).unwrap()[0];
    assert_eq!(last.text.chars().count(), data::MAX_CONSOLE_TEXT_CHARS);

    // Past the size, the log is rotated once: the older lines move to
    // `console.log.1`, which the next rotation replaces.
    let log = scratch.tree(ID).join("console.log");
    let rotated = scratch.tree(ID).join("console.log.1");
    let text = "z".repeat(1_900);
    let mut at = 1_000;
    while !rotated.exists() {
        let batch: Vec<ConsoleLine> = (0..32)
            .map(|_| {
                at += 1;
                line("log", &text, at)
            })
            .collect();
        data::console_append(&dir, ID, &batch).unwrap();
        assert!(std::fs::metadata(&log).unwrap().len() <= data::MAX_CONSOLE_BYTES);
    }
    let first_rotation = std::fs::read(&rotated).unwrap();
    let newest = data::console_read(&dir, ID, data::MAX_CONSOLE_READ).unwrap();
    assert_eq!(newest[0].at, at);
    let rotated_lines: Vec<u64> = newest
        .iter()
        .map(|l| l.at)
        .filter(|at| *at > 1_000)
        .collect();
    assert!(rotated_lines.len() > 32, "lines from both files");
    assert!(
        rotated_lines.windows(2).all(|w| w[0] > w[1]),
        "newest first, across both files"
    );
    while std::fs::read(&rotated).unwrap() == first_rotation {
        let batch: Vec<ConsoleLine> = (0..32)
            .map(|_| {
                at += 1;
                line("log", &text, at)
            })
            .collect();
        data::console_append(&dir, ID, &batch).unwrap();
    }
    let names: Vec<String> = snapshot(&scratch.tree(ID))
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["console.log", "console.log.1", "usage.json"]);
    let all = data::console_read(&dir, ID, 5000).unwrap().len();
    assert!(all > 500 && all < data::MAX_CONSOLE_READ, "{all}");
    // Short lines: more than a read answers.
    for _ in 0..20 {
        let batch: Vec<ConsoleLine> = (0..64)
            .map(|_| {
                at += 1;
                line("log", "short", at)
            })
            .collect();
        data::console_append(&dir, ID, &batch).unwrap();
    }
    assert_eq!(
        data::console_read(&dir, ID, 5000).unwrap().len(),
        data::MAX_CONSOLE_READ
    );
}
