//! The on-disk store, in a temporary folder: the layout an install writes,
//! declining, reinstalling, rolling back, uninstalling, the `read_file`
//! jail, the dependents cascade, the `installed.json` rules, folder
//! installs, `HEW_PLUGIN_DIR`, and each plugin's saved data.

#![cfg(all(hew_preview, feature = "fs"))]

mod common;

use common::*;
use plugin_host::store::{self, DevMode, Listed, PLUGIN_DIR_ENV, PluginDir};
use plugin_host::{InstallError, InstallSource, Installed, Package, read_package_zip};
use serde_json::json;
use std::path::{Path, PathBuf};

/// A scratch folder removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("hew-plugin-host-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn store(&self) -> PluginDir {
        PluginDir::at(self.0.join("plugins"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ID: &str = "com.example.fixture";

fn running() -> semver::Version {
    semver::Version::parse("1.4.0").unwrap()
}

fn package(id: &str, version: &str, extra: serde_json::Value, files: &[(&str, &[u8])]) -> Package {
    let mut m = json!({
        "manifest": 1, "id": id, "name": "Fixture", "version": version, "api": 1,
        "main": "dist/background.js", "permissions": ["model.read"]
    });
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("hew-plugin.json".into(), m.to_string().into_bytes()),
        (
            "dist/background.js".into(),
            format!("// {version}").into_bytes(),
        ),
    ];
    entries.extend(files.iter().map(|(n, b)| (n.to_string(), b.to_vec())));
    read_package_zip(&stored_zip(&entries)).expect("the fixture parses")
}

fn file_source() -> InstallSource {
    InstallSource::File {
        name: "fixture.hewplugin".into(),
    }
}

fn install(dir: &PluginDir, package: Package) -> Installed {
    let pending = store::stage(dir, package, file_source(), &running(), 1_000).expect("stages");
    store::commit(dir, pending, true, 2_000)
        .expect("commits")
        .expect("accepted")
}

fn ids(dir: &PluginDir) -> Vec<String> {
    store::list(dir)
        .into_iter()
        .map(|l| match l {
            Listed::Installed { installed, .. } => installed.id,
            Listed::Unreadable { id, .. } => format!("unreadable:{id}"),
        })
        .collect()
}

/// Every path under `root`, relative, sorted, folders marked with `/`.
fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let rel = e
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if e.file_type().unwrap().is_dir() {
                out.push(format!("{rel}/"));
                stack.push(e.path());
            } else {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn stage_then_commit_writes_the_layout() {
    let scratch = Scratch::new("layout");
    let dir = scratch.store();
    let pending = store::stage(
        &dir,
        package(ID, "1.0.0", json!({}), &[("icon.svg", b"<svg/>")]),
        file_source(),
        &running(),
        10,
    )
    .unwrap();
    assert_eq!(pending.token.len(), 32);
    assert!(pending.consent.unsigned);
    assert!(
        !dir.root().exists() || tree(dir.root()).is_empty(),
        "staging writes nothing"
    );
    let record = store::commit(&dir, pending, true, 20).unwrap().unwrap();
    assert_eq!(
        tree(dir.root()),
        [
            ".staging/",
            "com.example.fixture/",
            "com.example.fixture/files/",
            "com.example.fixture/files/dist/",
            "com.example.fixture/files/dist/background.js",
            "com.example.fixture/files/hew-plugin.json",
            "com.example.fixture/files/icon.svg",
            "com.example.fixture/hew-plugin.json",
            "com.example.fixture/installed.json",
        ]
    );
    assert_eq!(record.schema, 1);
    assert_eq!(record.version.to_string(), "1.0.0");
    assert_eq!(record.installed_at_ms, 20);
    assert!(record.enabled, "a fresh install is on");
    assert!(!record.startup_allowed, "but may not start with Hew yet");
    assert_eq!(
        record.grant,
        [plugin_manifest::PermissionName::parse("model.read").unwrap()]
    );
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.root().join(ID).join("installed.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk["schema"], 1);
    assert_eq!(
        on_disk["source"],
        json!({ "kind": "file", "name": "fixture.hewplugin" })
    );
    assert_eq!(ids(&dir), [ID]);
    let (again, manifest) = store::installed(&dir, ID).unwrap();
    assert_eq!(again, record);
    assert_eq!(manifest.id, ID);
}

#[test]
fn declining_leaves_nothing() {
    let scratch = Scratch::new("decline");
    let dir = scratch.store();
    let pending = store::stage(
        &dir,
        package(ID, "1.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    assert_eq!(store::commit(&dir, pending, false, 1).unwrap(), None);
    assert!(!dir.root().exists() || tree(dir.root()).is_empty());
    assert!(store::list(&dir).is_empty());
}

#[test]
fn an_expired_pending_install_is_refused() {
    let scratch = Scratch::new("expired");
    let dir = scratch.store();
    let pending = store::stage(
        &dir,
        package(ID, "1.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    assert_eq!(
        store::commit(&dir, pending, true, store::PENDING_TTL_MS + 1),
        Err(InstallError::PendingExpired)
    );
    assert!(store::list(&dir).is_empty());
}

#[test]
fn reinstalling_rotates_previous_and_drops_the_older_one() {
    let scratch = Scratch::new("rotate");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    let root = dir.root().join(ID);
    let version_of = |p: &Path| {
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(p.join("installed.json")).unwrap()).unwrap();
        v["version"].as_str().unwrap().to_string()
    };
    assert_eq!(version_of(&root), "2.0.0");
    assert_eq!(version_of(&root.join("previous")), "1.0.0");
    assert!(!root.join("previous/previous").exists());
    install(&dir, package(ID, "3.0.0", json!({}), &[]));
    assert_eq!(version_of(&root), "3.0.0");
    assert_eq!(version_of(&root.join("previous")), "2.0.0");
    assert!(
        !root.join("previous/previous").exists(),
        "only one previous is kept"
    );
    match &store::list(&dir)[0] {
        Listed::Installed {
            previous_version, ..
        } => assert_eq!(previous_version.as_deref(), Some("2.0.0")),
        other => panic!("{other:?}"),
    }
    assert_eq!(tree(&dir.root().join(".staging")), Vec::<String>::new());
}

#[test]
fn an_update_shows_what_changed_and_keeps_the_users_choices() {
    let scratch = Scratch::new("update");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    store::set_enabled(&dir, ID, false).unwrap();
    let pending = store::stage(
        &dir,
        package(
            ID,
            "2.0.0",
            json!({ "permissions": ["model.read", "view"] }),
            &[],
        ),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    let update = pending.consent.update.clone().expect("an update");
    assert_eq!(update.from_version, "1.0.0");
    assert_eq!(update.added.len(), 1);
    assert_eq!(pending.consent.replaces.as_ref().unwrap().version, "1.0.0");
    let record = store::commit(&dir, pending, true, 1).unwrap().unwrap();
    assert!(!record.enabled, "an update does not turn a plugin back on");
}

#[test]
fn rollback_swaps_and_swaps_back() {
    let scratch = Scratch::new("rollback");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    assert_eq!(
        store::rollback(&dir, ID, &running()),
        Err(InstallError::NoPreviousVersion(ID.into()))
    );
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    let back = store::rollback(&dir, ID, &running()).unwrap();
    assert_eq!(back.version.to_string(), "1.0.0");
    assert_eq!(
        store::read_file(&dir, ID, "dist/background.js", DevMode::Off).unwrap(),
        b"// 1.0.0"
    );
    let forward = store::rollback(&dir, ID, &running()).unwrap();
    assert_eq!(forward.version.to_string(), "2.0.0");
    assert_eq!(
        store::read_file(&dir, ID, "dist/background.js", DevMode::Off).unwrap(),
        b"// 2.0.0"
    );
    assert_eq!(ids(&dir), [ID]);
}

#[test]
fn uninstall_removes_everything() {
    let scratch = Scratch::new("uninstall");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    install(&dir, package("com.example.other", "1.0.0", json!({}), &[]));
    store::uninstall(&dir, ID).unwrap();
    assert!(!dir.root().join(ID).exists());
    assert_eq!(ids(&dir), ["com.example.other"]);
    assert_eq!(
        store::uninstall(&dir, ID),
        Err(InstallError::NotInstalled(ID.into()))
    );
}

#[test]
fn read_file_is_jailed() {
    let scratch = Scratch::new("jail");
    let dir = scratch.store();
    install(
        &dir,
        package(ID, "1.0.0", json!({}), &[("icon.svg", b"<svg/>")]),
    );
    // A file outside the plugin's folder, to be sure nothing reaches it.
    std::fs::write(dir.root().join("secret.txt"), b"secret").unwrap();
    assert_eq!(
        store::read_file(&dir, ID, "icon.svg", DevMode::Off).unwrap(),
        b"<svg/>"
    );
    for bad in [
        "../x",
        "files/../installed.json",
        "../installed.json",
        "../../secret.txt",
        "/etc/passwd",
        "dist\\background.js",
        "C:/x",
        "",
        "dist/",
        "./icon.svg",
        "nope.js",
        "dist",
    ] {
        assert!(
            matches!(
                store::read_file(&dir, ID, bad, DevMode::Off),
                Err(InstallError::PackagePathInvalid(_))
            ),
            "{bad:?}"
        );
    }
    // installed.json and the stored manifest sit beside files/, not in it.
    assert!(store::read_file(&dir, ID, "installed.json", DevMode::Off).is_err());
    // An id that is not one never becomes a path.
    for bad_id in ["..", "../plugins", "com.example.fixture/..", "a"] {
        assert!(
            store::read_file(&dir, bad_id, "icon.svg", DevMode::Off).is_err(),
            "{bad_id}"
        );
    }
}

#[cfg(unix)]
#[test]
fn read_file_refuses_a_link_planted_in_the_store() {
    let scratch = Scratch::new("planted");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    std::fs::write(scratch.0.join("outside.txt"), b"outside").unwrap();
    let files = dir.root().join(ID).join("files");
    std::os::unix::fs::symlink(scratch.0.join("outside.txt"), files.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(&scratch.0, files.join("up")).unwrap();
    assert!(store::read_file(&dir, ID, "link.txt", DevMode::Off).is_err());
    assert!(store::read_file(&dir, ID, "up/outside.txt", DevMode::Off).is_err());
}

#[test]
fn disabling_cascades_to_dependents() {
    let scratch = Scratch::new("cascade");
    let dir = scratch.store();
    install(&dir, package("com.example.base", "1.0.0", json!({}), &[]));
    let needs = |id: &str, on: &str| {
        package(
            id,
            "1.0.0",
            json!({ "requires": [{ "id": on, "range": "*" }] }),
            &[],
        )
    };
    install(&dir, needs("com.example.mid", "com.example.base"));
    install(&dir, needs("com.example.top", "com.example.mid"));
    install(&dir, package("com.example.alone", "1.0.0", json!({}), &[]));
    assert_eq!(
        store::dependents_of(&dir, "com.example.base"),
        ["com.example.mid"]
    );
    assert_eq!(
        store::uninstall(&dir, "com.example.base"),
        Err(InstallError::DependencyMissing(vec![
            "com.example.mid".into()
        ]))
    );
    let (base, also) = store::set_enabled(&dir, "com.example.base", false).unwrap();
    assert!(!base.enabled);
    assert_eq!(also, ["com.example.mid", "com.example.top"]);
    for id in ["com.example.mid", "com.example.top"] {
        assert!(!store::installed(&dir, id).unwrap().0.enabled, "{id}");
    }
    assert!(
        store::installed(&dir, "com.example.alone")
            .unwrap()
            .0
            .enabled
    );
    assert!(store::dependents_of(&dir, "com.example.base").is_empty());
    store::uninstall(&dir, "com.example.base").expect("no enabled dependents now");
}

#[test]
fn turning_a_plugin_on_by_hand_allows_it_to_start_with_hew() {
    let scratch = Scratch::new("startup");
    let dir = scratch.store();
    let record = install(
        &dir,
        package(
            ID,
            "1.0.0",
            json!({ "activation": ["on_startup"], "activationReason": "Watches the model." }),
            &[],
        ),
    );
    assert!(record.enabled && !record.startup_allowed);
    let (off, _) = store::set_enabled(&dir, ID, false).unwrap();
    assert!(!off.startup_allowed);
    let (on, _) = store::set_enabled(&dir, ID, true).unwrap();
    assert!(on.enabled && on.startup_allowed);
    // An update that keeps on_startup keeps the permission; one that adds
    // it anew asks again.
    let kept = install(
        &dir,
        package(
            ID,
            "1.1.0",
            json!({ "activation": ["on_startup"], "activationReason": "Watches the model." }),
            &[],
        ),
    );
    assert!(kept.startup_allowed);
    install(&dir, package("com.example.late", "1.0.0", json!({}), &[]));
    store::set_enabled(&dir, "com.example.late", true).unwrap();
    let newly = install(
        &dir,
        package(
            "com.example.late",
            "2.0.0",
            json!({ "activation": ["on_startup"], "activationReason": "Now it starts." }),
            &[],
        ),
    );
    assert!(!newly.startup_allowed);
}

#[test]
fn set_grant_holds_only_declared_permissions() {
    let scratch = Scratch::new("grant");
    let dir = scratch.store();
    install(
        &dir,
        package(
            ID,
            "1.0.0",
            json!({ "permissions": ["model.read", "view"], "optionalPermissions": ["clipboard"] }),
            &[],
        ),
    );
    let p = |s: &str| plugin_manifest::PermissionName::parse(s).unwrap();
    let record = store::set_grant(&dir, ID, vec![p("model.read")], vec![p("clipboard")]).unwrap();
    assert_eq!(record.grant, [p("model.read")]);
    assert_eq!(record.optional_granted, [p("clipboard")]);
    assert_eq!(
        store::set_grant(&dir, ID, vec![p("model.write")], vec![])
            .unwrap_err()
            .name(),
        "unknown_permission"
    );
    assert_eq!(
        store::set_grant(&dir, ID, vec![], vec![p("view")])
            .unwrap_err()
            .name(),
        "unknown_permission"
    );
}

#[test]
fn unknown_fields_survive_a_rewrite_and_a_newer_record_is_never_rewritten() {
    let scratch = Scratch::new("schema");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    let path = dir.root().join(ID).join("installed.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["from_the_future"] = json!({ "a": 1 });
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    store::set_enabled(&dir, ID, false).unwrap();
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["from_the_future"], json!({ "a": 1 }));
    assert_eq!(after["enabled"], false);

    v["schema"] = json!(2);
    let newer = serde_json::to_vec(&v).unwrap();
    std::fs::write(&path, &newer).unwrap();
    assert!(matches!(&store::list(&dir)[0], Listed::Unreadable { id, .. } if id == ID));
    assert_eq!(
        store::set_enabled(&dir, ID, true).unwrap_err().name(),
        "installed_unreadable"
    );
    assert_eq!(
        store::stage(
            &dir,
            package(ID, "9.0.0", json!({}), &[]),
            file_source(),
            &running(),
            0
        )
        .unwrap_err()
        .name(),
        "installed_unreadable"
    );
    assert_eq!(std::fs::read(&path).unwrap(), newer, "never rewritten");
    store::uninstall(&dir, ID).expect("it can still be removed");
}

#[test]
fn an_interrupted_install_is_ignored_and_then_cleared() {
    let scratch = Scratch::new("interrupted");
    let dir = scratch.store();
    let token = "0123456789abcdef0123456789abcdef";
    let stale = dir.root().join(".staging").join(format!("{ID}-{token}"));
    std::fs::create_dir_all(stale.join("files")).unwrap();
    std::fs::write(stale.join("installed.json"), b"{}").unwrap();
    // Another plugin's staging entry whose id starts like this one's.
    let neighbor = dir.root().join(".staging").join(format!("{ID}-x-{token}"));
    std::fs::create_dir_all(&neighbor).unwrap();
    assert!(store::list(&dir).is_empty());
    let pending = store::stage(
        &dir,
        package(ID, "1.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    assert!(!stale.exists(), "the next stage of the same id clears it");
    assert!(neighbor.exists(), "and nothing else");
    store::commit(&dir, pending, true, 1).unwrap();
    assert_eq!(ids(&dir), [ID]);
}

#[test]
fn installs_that_must_not_happen_do_not() {
    let scratch = Scratch::new("refused");
    let dir = scratch.store();
    let refused = |p: Package| {
        store::stage(&dir, p, file_source(), &running(), 0)
            .unwrap_err()
            .name()
    };
    assert_eq!(
        refused(package("org.hew3d.cutlist", "1.0.0", json!({}), &[])),
        "reserved_id"
    );
    assert_eq!(
        refused(package(ID, "1.0.0", json!({ "hew": ">=9" }), &[])),
        "hew_version_unsupported"
    );
    assert_eq!(
        refused(package(
            ID,
            "1.0.0",
            json!({ "requires": [{ "id": "com.example.base", "range": "*" }] }),
            &[]
        )),
        "dependency_missing"
    );
    assert!(store::list(&dir).is_empty());
}

#[test]
fn a_folder_install_reads_in_place() {
    let scratch = Scratch::new("folder");
    let dir = scratch.store();
    let src = scratch.0.join("src");
    std::fs::create_dir_all(src.join("dist")).unwrap();
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest_json(ID, &["model.read"]),
    )
    .unwrap();
    std::fs::write(src.join("dist/background.js"), b"// folder").unwrap();
    let package = store::read_package_dir(&src).unwrap();
    // The host records the canonical path the person picked.
    let src = std::fs::canonicalize(&src).unwrap();
    let source = InstallSource::Dir {
        path: src.to_string_lossy().into_owned(),
    };
    let pending = store::stage(&dir, package, source.clone(), &running(), 0).unwrap();
    let record = store::commit(&dir, pending, true, 1).unwrap().unwrap();
    assert_eq!(record.source, source);
    assert!(
        !dir.root().join(ID).join("files").exists(),
        "no copy is kept"
    );
    assert_eq!(
        store::read_file(&dir, ID, "dist/background.js", DevMode::On).unwrap(),
        b"// folder"
    );
    std::fs::write(src.join("dist/background.js"), b"// edited").unwrap();
    assert_eq!(
        store::read_file(&dir, ID, "dist/background.js", DevMode::On).unwrap(),
        b"// edited",
        "read from the folder"
    );
    assert!(store::read_file(&dir, ID, "../src/hew-plugin.json", DevMode::On).is_err());
}

#[test]
fn a_folder_package_follows_the_package_rules() {
    let scratch = Scratch::new("folder-rules");
    let src = scratch.0.join("src");
    std::fs::create_dir_all(src.join("dist")).unwrap();
    std::fs::write(src.join("hew-plugin.json"), manifest_json(ID, &[])).unwrap();
    std::fs::write(src.join("dist/background.js"), b"x").unwrap();
    store::read_package_dir(&src).expect("a plain folder reads");
    // Through the canonical path: on Windows that is the `\\?\` form, which
    // keeps a trailing dot instead of silently dropping it.
    let trailing = std::fs::canonicalize(&src).unwrap().join("trailing.");
    std::fs::write(&trailing, b"x").unwrap();
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "package_path_invalid"
    );
    std::fs::remove_file(&trailing).unwrap();
    std::fs::write(
        src.join("big.bin"),
        vec![0u8; plugin_host::MAX_ENTRY_BYTES + 1],
    )
    .unwrap();
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "package_too_large"
    );
    std::fs::remove_file(src.join("big.bin")).unwrap();
    std::fs::remove_file(src.join("hew-plugin.json")).unwrap();
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "manifest_missing"
    );
}

#[test]
fn a_folder_package_with_a_name_that_is_not_ascii_is_refused() {
    let scratch = Scratch::new("folder-ascii");
    let src = scratch.0.join("src");
    std::fs::create_dir_all(src.join("dist")).unwrap();
    std::fs::write(src.join("hew-plugin.json"), manifest_json(ID, &[])).unwrap();
    std::fs::write(src.join("dist/background.js"), b"x").unwrap();
    // The composed and decomposed forms of one name. Where the file system
    // keeps them apart both exist; where it merges them one does; either
    // way the folder is refused.
    std::fs::write(src.join("caf\u{e9}.js"), b"1").unwrap();
    std::fs::write(src.join("cafe\u{301}.js"), b"2").unwrap();
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "package_path_invalid"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_package_with_a_link_is_refused() {
    let scratch = Scratch::new("folder-link");
    let src = scratch.0.join("src");
    std::fs::create_dir_all(src.join("dist")).unwrap();
    std::fs::write(src.join("hew-plugin.json"), manifest_json(ID, &[])).unwrap();
    std::fs::write(src.join("dist/background.js"), b"x").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", src.join("dist/hosts")).unwrap();
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "package_path_invalid"
    );
    std::fs::remove_file(src.join("dist/hosts")).unwrap();
    std::os::unix::fs::symlink(&src, scratch.0.join("linked")).unwrap();
    assert!(
        store::read_package_dir(&scratch.0.join("linked")).is_err(),
        "the folder itself may not be a link"
    );
}

/// The only test that touches the variable, so no other test can see it
/// change.
#[test]
fn hew_plugin_dir_overrides_the_store_location() {
    let scratch = Scratch::new("env");
    let chosen = scratch.0.join("elsewhere");
    // SAFETY: no other test in this binary reads or writes the
    // environment.
    unsafe { std::env::set_var(PLUGIN_DIR_ENV, &chosen) };
    let resolved = PluginDir::resolve().expect("resolves");
    assert_eq!(resolved.root(), chosen.as_path());
    // A relative value is not trusted; the app data folder is used.
    unsafe { std::env::set_var(PLUGIN_DIR_ENV, "relative/plugins") };
    let fallback = PluginDir::resolve();
    unsafe { std::env::remove_var(PLUGIN_DIR_ENV) };
    let default = PluginDir::resolve();
    assert_eq!(fallback, default);
    if let Some(default) = default {
        assert!(
            default.root().ends_with("com.hew3d.Hew/plugins"),
            "{default:?}"
        );
    }
}

/// The example at `plugins/examples/manifest-only` (what the install flow
/// is tried with by hand) is a valid package, as a folder and zipped, and
/// its consent shows the model-reading line and the network warning.
#[test]
fn the_manifest_only_example_installs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/manifest-only");
    let package = store::read_package_dir(&src).expect("the example reads as a folder");
    assert_eq!(
        package.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["dist/background.js", "hew-plugin.json"]
    );
    let zipped = stored_zip(
        &package
            .files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect::<Vec<_>>(),
    );
    assert_eq!(read_package_zip(&zipped).unwrap(), package);
    let consent = plugin_host::consent(&package.manifest, &file_source(), None, None, None);
    assert_eq!(
        consent.permissions[0].text,
        "Read the model: every object, component, material, tag, and attribute"
    );
    assert_eq!(
        consent.warnings,
        ["Can send what it reads from your model to api.example.com."]
    );
    let scratch = Scratch::new("example");
    let record = install(&scratch.store(), package);
    assert!(record.enabled);
}

#[test]
fn commit_checks_requires_again_and_writes_nothing_when_they_fail() {
    let scratch = Scratch::new("recheck");
    let dir = scratch.store();
    install(&dir, package("com.example.base", "1.0.0", json!({}), &[]));
    let needs_base = || {
        package(
            ID,
            "1.0.0",
            json!({ "requires": [{ "id": "com.example.base", "range": "*" }] }),
            &[],
        )
    };
    // The dependency is removed between stage and commit.
    let pending = store::stage(&dir, needs_base(), file_source(), &running(), 0).unwrap();
    store::uninstall(&dir, "com.example.base").unwrap();
    assert_eq!(
        store::commit(&dir, pending, true, 1),
        Err(InstallError::DependencyMissing(vec![
            "com.example.base *".into()
        ]))
    );
    assert!(!dir.root().join(ID).exists());
    assert_eq!(tree(&dir.root().join(".staging")), Vec::<String>::new());
    assert!(store::list(&dir).is_empty());

    // A cycle that appears between stage and commit: an older copy of
    // this plugin is installed, the update needs base, and base is then
    // updated to need this plugin.
    install(&dir, package("com.example.base", "1.0.0", json!({}), &[]));
    install(&dir, package(ID, "0.9.0", json!({}), &[]));
    let pending = store::stage(&dir, needs_base(), file_source(), &running(), 0).unwrap();
    install(
        &dir,
        package(
            "com.example.base",
            "2.0.0",
            json!({ "requires": [{ "id": ID, "range": "*" }] }),
            &[],
        ),
    );
    assert_eq!(
        store::commit(&dir, pending, true, 1).unwrap_err().name(),
        "dependency_cycle"
    );
    assert_eq!(
        store::installed(&dir, ID).unwrap().0.version.to_string(),
        "0.9.0",
        "the installed copy is untouched"
    );
    assert!(!dir.root().join(ID).join("previous").exists());
}

#[test]
fn previous_answers_what_a_rollback_brings_back() {
    let scratch = Scratch::new("previous");
    let dir = scratch.store();
    assert_eq!(
        store::previous(&dir, ID),
        Err(InstallError::NotInstalled(ID.into()))
    );
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    assert_eq!(
        store::previous(&dir, ID),
        Err(InstallError::NoPreviousVersion(ID.into()))
    );
    install(
        &dir,
        package(
            ID,
            "2.0.0",
            json!({ "permissions": ["model.read", "view"] }),
            &[],
        ),
    );
    let (record, manifest) = store::previous(&dir, ID).unwrap();
    assert_eq!(record.version.to_string(), "1.0.0");
    assert_eq!(manifest.version.to_string(), "1.0.0");
    assert_eq!(record.grant.len(), 1);
    let path = dir.root().join(ID).join("previous").join("installed.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["schema"] = json!(2);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        store::previous(&dir, ID).unwrap_err().name(),
        "installed_unreadable"
    );
    assert_eq!(
        store::rollback(&dir, ID, &running()).unwrap_err().name(),
        "installed_unreadable"
    );
    assert_eq!(
        store::installed(&dir, ID).unwrap().0.version.to_string(),
        "2.0.0"
    );
}

#[test]
fn a_pending_install_goes_stale_when_what_is_installed_changes() {
    let stale = |r: Result<Option<Installed>, InstallError>| {
        assert_eq!(r.unwrap_err(), InstallError::PendingStale);
    };
    // Not installed at stage, installed by commit.
    let scratch = Scratch::new("stale-new");
    let dir = scratch.store();
    let pending = store::stage(
        &dir,
        package(ID, "2.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    stale(store::commit(&dir, pending, true, 1));
    assert_eq!(
        store::installed(&dir, ID).unwrap().0.version.to_string(),
        "1.0.0"
    );
    assert_eq!(
        store::previous(&dir, ID),
        Err(InstallError::NoPreviousVersion(ID.into()))
    );

    // Installed at stage, uninstalled by commit.
    let pending = store::stage(
        &dir,
        package(ID, "2.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    store::uninstall(&dir, ID).unwrap();
    stale(store::commit(&dir, pending, true, 1));
    assert!(store::list(&dir).is_empty(), "nothing written");

    // Installed at stage, its record rewritten by commit.
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    let pending = store::stage(
        &dir,
        package(ID, "2.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    store::set_enabled(&dir, ID, false).unwrap();
    let before = std::fs::read(dir.root().join(ID).join("installed.json")).unwrap();
    stale(store::commit(&dir, pending, true, 1));
    assert_eq!(
        std::fs::read(dir.root().join(ID).join("installed.json")).unwrap(),
        before
    );

    // Unchanged, it commits.
    let pending = store::stage(
        &dir,
        package(ID, "2.0.0", json!({}), &[]),
        file_source(),
        &running(),
        0,
    )
    .unwrap();
    store::commit(&dir, pending, true, 1).unwrap();
    assert_eq!(
        store::installed(&dir, ID).unwrap().0.version.to_string(),
        "2.0.0"
    );
}

#[test]
fn too_many_permissions_are_refused_in_a_folder() {
    let scratch = Scratch::new("folder-perms");
    let src = scratch.0.join("src");
    std::fs::create_dir_all(src.join("dist")).unwrap();
    let permissions: Vec<String> = (0..=plugin_host::MAX_PERMISSIONS)
        .map(|i| format!("network:h{i}.example.com"))
        .collect();
    let refs: Vec<&str> = permissions.iter().map(String::as_str).collect();
    std::fs::write(src.join("hew-plugin.json"), manifest_json(ID, &refs)).unwrap();
    std::fs::write(src.join("dist/background.js"), b"x").unwrap();
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "too_many_permissions"
    );
}

#[test]
fn storage_saves_reads_lists_and_deletes() {
    let scratch = Scratch::new("storage");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    assert_eq!(store::storage_list(&dir, ID).unwrap(), Vec::<String>::new());
    assert_eq!(store::storage_get(&dir, ID, "a").unwrap(), None);
    assert!(!dir.root().join(ID).join("storage.json").exists());

    store::storage_set(&dir, ID, "b", "2").unwrap();
    store::storage_set(&dir, ID, "a", "\"one\"").unwrap();
    store::storage_set(&dir, ID, "a", "{\"x\":1}").unwrap();
    assert_eq!(
        store::storage_get(&dir, ID, "a").unwrap().as_deref(),
        Some("{\"x\":1}")
    );
    assert_eq!(store::storage_list(&dir, ID).unwrap(), ["a", "b"]);
    // One JSON object of strings, in the plugin's folder.
    let file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.root().join(ID).join("storage.json")).unwrap())
            .unwrap();
    assert_eq!(file, json!({ "a": "{\"x\":1}", "b": "2" }));

    assert!(store::storage_delete(&dir, ID, "a").unwrap());
    assert!(!store::storage_delete(&dir, ID, "a").unwrap());
    assert_eq!(store::storage_list(&dir, ID).unwrap(), ["b"]);
    // The file goes with its last key.
    assert!(store::storage_delete(&dir, ID, "b").unwrap());
    assert!(!dir.root().join(ID).join("storage.json").exists());
    // No temporary file is left behind.
    store::storage_set(&dir, ID, "c", "3").unwrap();
    let names: Vec<String> = std::fs::read_dir(dir.root().join(ID))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("storage"))
        .collect();
    assert_eq!(names, ["storage.json"]);
}

#[test]
fn storage_keys_are_bounded() {
    let scratch = Scratch::new("storage-keys");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    let long = "k".repeat(plugin_host::MAX_STORAGE_KEY_BYTES + 1);
    for key in ["", long.as_str()] {
        assert_eq!(
            store::storage_set(&dir, ID, key, "v"),
            Err(InstallError::StorageKeyInvalid)
        );
        assert_eq!(
            store::storage_get(&dir, ID, key),
            Err(InstallError::StorageKeyInvalid)
        );
        assert_eq!(
            store::storage_delete(&dir, ID, key),
            Err(InstallError::StorageKeyInvalid)
        );
    }
    let longest = "k".repeat(plugin_host::MAX_STORAGE_KEY_BYTES);
    store::storage_set(&dir, ID, &longest, "v").unwrap();
    // Keys are only ever map keys: path-like text is just text.
    store::storage_set(&dir, ID, "../../installed.json", "v").unwrap();
    assert_eq!(
        store::storage_list(&dir, ID).unwrap(),
        ["../../installed.json", longest.as_str()]
    );
    assert!(dir.root().join(ID).join("installed.json").is_file());
}

#[test]
fn storage_is_refused_past_the_quota_and_writes_nothing() {
    let scratch = Scratch::new("storage-quota");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    let cap = plugin_host::MAX_STORAGE_BYTES;
    assert_eq!(cap, 5 * 1024 * 1024);
    // Too large on its own.
    assert_eq!(
        store::storage_set(&dir, ID, "a", &"x".repeat(cap)),
        Err(InstallError::StorageQuota)
    );
    assert!(!dir.root().join(ID).join("storage.json").exists());
    // Two values that fit alone but not together.
    let half = "x".repeat(cap / 2);
    store::storage_set(&dir, ID, "a", &half).unwrap();
    let before = std::fs::read(dir.root().join(ID).join("storage.json")).unwrap();
    assert_eq!(
        store::storage_set(&dir, ID, "b", &half),
        Err(InstallError::StorageQuota)
    );
    assert_eq!(
        std::fs::read(dir.root().join(ID).join("storage.json")).unwrap(),
        before
    );
    assert_eq!(store::storage_list(&dir, ID).unwrap(), ["a"]);
    // Replacing a value is measured on the result.
    store::storage_set(&dir, ID, "a", &half).unwrap();
    assert_eq!(
        InstallError::StorageQuota.name(),
        "storage_quota",
        "the refusal the app branches on"
    );
}

#[test]
fn storage_needs_an_installed_plugin_and_never_creates_its_folder() {
    let scratch = Scratch::new("storage-missing");
    let dir = scratch.store();
    let other = "com.example.missing";
    assert_eq!(
        store::storage_set(&dir, other, "a", "1"),
        Err(InstallError::NotInstalled(other.into()))
    );
    assert_eq!(
        store::storage_get(&dir, other, "a"),
        Err(InstallError::NotInstalled(other.into()))
    );
    assert_eq!(
        store::storage_list(&dir, other),
        Err(InstallError::NotInstalled(other.into()))
    );
    assert!(!dir.root().join(other).exists());
    // A bad id is refused by the grammar before any path is built.
    for bad in [
        "../x",
        "a",
        "hew.x",
        "a.b/../c",
        "",
        "con.example",
        "com1.tools",
        "lpt9.x",
        "nul.b",
    ] {
        assert!(matches!(
            store::storage_set(&dir, bad, "a", "1"),
            Err(InstallError::Manifest(_))
        ));
    }
    assert!(!dir.root().exists());
    // A folder with no record is not a plugin either.
    std::fs::create_dir_all(dir.root().join(other)).unwrap();
    assert_eq!(
        store::storage_set(&dir, other, "a", "1"),
        Err(InstallError::NotInstalled(other.into()))
    );
    assert!(!dir.root().join(other).join("storage.json").exists());
}

#[test]
fn unreadable_storage_is_refused_and_left_as_it_is() {
    let scratch = Scratch::new("storage-bad");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    let path = dir.root().join(ID).join("storage.json");
    for bad in [&b"[1,2]"[..], b"{\"a\":1}", b"not json"] {
        std::fs::write(&path, bad).unwrap();
        assert_eq!(
            store::storage_set(&dir, ID, "a", "1"),
            Err(InstallError::StorageUnreadable)
        );
        assert_eq!(
            store::storage_get(&dir, ID, "a"),
            Err(InstallError::StorageUnreadable)
        );
        assert_eq!(std::fs::read(&path).unwrap(), bad);
    }
    // A link in its place is not followed (links are made this way on
    // Unix only).
    #[cfg(unix)]
    {
        std::fs::remove_file(&path).unwrap();
        let outside = scratch.0.join("outside.json");
        std::fs::write(&outside, b"{\"a\":\"secret\"}").unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert_eq!(
            store::storage_get(&dir, ID, "a"),
            Err(InstallError::StorageUnreadable)
        );
        assert_eq!(
            store::storage_set(&dir, ID, "a", "1"),
            Err(InstallError::StorageUnreadable)
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"{\"a\":\"secret\"}");
    }
}

#[test]
fn storage_follows_the_plugin_through_updates_and_rollbacks() {
    let scratch = Scratch::new("storage-versions");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    store::storage_set(&dir, ID, "count", "1").unwrap();
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    assert_eq!(
        store::storage_get(&dir, ID, "count").unwrap().as_deref(),
        Some("1"),
        "an update keeps the saved data"
    );
    store::storage_set(&dir, ID, "count", "2").unwrap();
    store::rollback(&dir, ID, &running()).unwrap();
    assert_eq!(
        store::installed(&dir, ID).unwrap().0.version.to_string(),
        "1.0.0"
    );
    assert_eq!(
        store::storage_get(&dir, ID, "count").unwrap().as_deref(),
        Some("2"),
        "a rollback keeps the latest saved data"
    );
    store::storage_set(&dir, ID, "count", "3").unwrap();
    store::rollback(&dir, ID, &running()).unwrap();
    assert_eq!(
        store::storage_get(&dir, ID, "count").unwrap().as_deref(),
        Some("3")
    );
    // Data deleted before an update does not come back on a rollback.
    assert!(store::storage_delete(&dir, ID, "count").unwrap());
    store::rollback(&dir, ID, &running()).unwrap();
    assert_eq!(store::storage_list(&dir, ID).unwrap(), Vec::<String>::new());
    // A reinstall of the same version keeps it too.
    store::storage_set(&dir, ID, "kept", "yes").unwrap();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    assert_eq!(
        store::storage_get(&dir, ID, "kept").unwrap().as_deref(),
        Some("yes")
    );
}

#[test]
fn uninstalling_removes_the_saved_data() {
    let scratch = Scratch::new("storage-uninstall");
    let dir = scratch.store();
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    store::storage_set(&dir, ID, "a", "1").unwrap();
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    // Both the current version and the one kept for a rollback hold a copy.
    assert!(dir.root().join(ID).join("storage.json").exists());
    assert!(
        dir.root()
            .join(ID)
            .join("previous")
            .join("storage.json")
            .exists()
    );
    store::uninstall(&dir, ID).unwrap();
    assert!(!dir.root().join(ID).exists());
    // A different package installed under the same id starts empty.
    install(&dir, package(ID, "1.0.0", json!({}), &[]));
    assert_eq!(store::storage_list(&dir, ID).unwrap(), Vec::<String>::new());
    store::storage_set(&dir, ID, "a", "1").unwrap();
    store::purge(&dir, ID).unwrap();
    assert!(!dir.root().join(ID).exists());
}

#[test]
fn a_rollback_follows_the_install_rules_and_moves_nothing_when_refused() {
    let command = |id: &str| {
        json!({ "commands": [{
            "id": id, "class": "read_only", "summary": "Runs.",
            "params": { "type": "object", "additionalProperties": false },
            "result": { "type": "object", "additionalProperties": false }
        }] })
    };
    // 1.0 has `tool.run`; 2.0 does not, so another plugin may take the name.
    let scratch = Scratch::new("rollback-names");
    let dir = scratch.store();
    install(
        &dir,
        package(
            ID,
            "1.0.0",
            json!({ "contributes": command("tool.run") }),
            &[],
        ),
    );
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    let other = "com.example.fixture.tool";
    install(
        &dir,
        package(
            other,
            "1.0.0",
            json!({ "contributes": command("run") }),
            &[],
        ),
    );
    let before = tree(dir.root());
    for refused in [
        store::check_rollback(&dir, ID, &running()).unwrap_err(),
        store::rollback(&dir, ID, &running()).unwrap_err(),
    ] {
        assert_eq!(refused.name(), "command_name_taken");
    }
    assert_eq!(tree(dir.root()), before, "nothing moved");
    assert_eq!(
        store::installed(&dir, ID).unwrap().0.version.to_string(),
        "2.0.0"
    );

    // 1.0 required a plugin that is gone since.
    let scratch = Scratch::new("rollback-requires");
    let dir = scratch.store();
    let dep = "com.example.dep";
    install(&dir, package(dep, "1.0.0", json!({}), &[]));
    install(
        &dir,
        package(
            ID,
            "1.0.0",
            json!({ "requires": [{ "id": dep, "range": "^1" }] }),
            &[],
        ),
    );
    install(&dir, package(ID, "2.0.0", json!({}), &[]));
    store::uninstall(&dir, dep).unwrap();
    let before = tree(dir.root());
    assert_eq!(
        store::rollback(&dir, ID, &running()).unwrap_err().name(),
        "dependency_missing"
    );
    assert_eq!(tree(dir.root()), before, "nothing moved");
}
