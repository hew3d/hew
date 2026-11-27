//! Folder installs: the reader every folder read goes through (the root
//! recorded at the pick is the only root; no link anywhere; the file opened
//! is the file checked), the watcher's listing, developer mode, the review
//! marker, and a review accepted in place.

#![cfg(all(hew_preview, feature = "fs"))]

mod common;

use plugin_host::InstallError;
use plugin_host::InstallSource;
use plugin_host::store::{
    self, DevMode, FolderOutcome, NEEDS_CONSENT, NEEDS_CONSENT_REASON, PluginDir,
};
use serde_json::json;
use std::path::{Path, PathBuf};

const ID: &str = "com.example.folder";

/// A scratch folder removed when the test ends; canonical, as a pick is.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hew-plugin-folder-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(std::fs::canonicalize(&dir).unwrap())
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

fn running() -> semver::Version {
    semver::Version::parse("1.4.0").unwrap()
}

fn manifest(extra: serde_json::Value) -> String {
    let mut m = json!({
        "manifest": 1, "id": ID, "name": "Folder", "version": "1.0.0", "api": 1,
        "main": "lib/deep/main.js", "permissions": ["model.read"]
    });
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    m.to_string()
}

/// A plugin folder at `root/name` with its code two folders down.
fn folder(root: &Path, name: &str) -> PathBuf {
    let src = root.join(name);
    std::fs::create_dir_all(src.join("lib/deep")).unwrap();
    std::fs::write(src.join("hew-plugin.json"), manifest(json!({}))).unwrap();
    std::fs::write(src.join("lib/deep/main.js"), b"// inside").unwrap();
    src
}

/// Installs the folder at `src` (canonical) and answers yes.
fn install(dir: &PluginDir, src: &Path) {
    let package = store::read_package_dir(src).unwrap();
    let source = InstallSource::Dir {
        path: src.to_string_lossy().into_owned(),
    };
    let pending = store::stage(dir, package, source, &running(), 1_000).unwrap();
    store::commit(dir, pending, true, 2_000).unwrap().unwrap();
}

fn read(dir: &PluginDir, rel: &str) -> Result<Vec<u8>, InstallError> {
    store::read_file(dir, ID, rel, DevMode::On)
}

fn name(r: Result<Vec<u8>, InstallError>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e.name(),
    }
}

/// A copy of the folder elsewhere, holding `secret` where the code was.
#[cfg(unix)]
fn decoy(root: &Path) -> PathBuf {
    let d = root.join("decoy");
    std::fs::create_dir_all(d.join("lib/deep")).unwrap();
    std::fs::write(d.join("hew-plugin.json"), manifest(json!({}))).unwrap();
    std::fs::write(d.join("lib/deep/main.js"), b"secret").unwrap();
    d
}

#[test]
fn a_folder_install_reads_in_place_and_only_with_developer_mode_on() {
    let scratch = Scratch::new("on-off");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    assert_eq!(read(&dir, "lib/deep/main.js").unwrap(), b"// inside");
    assert_eq!(
        name(store::read_file(&dir, ID, "lib/deep/main.js", DevMode::Off)),
        "developer_mode_off"
    );
    assert_eq!(
        name(store::read_file(&dir, ID, "hew-plugin.json", DevMode::Off)),
        "developer_mode_off"
    );
    assert_eq!(
        store::read_review(&dir, ID, DevMode::Off)
            .unwrap_err()
            .name(),
        "developer_mode_off"
    );
    // A bad path is refused for what it is before anything else.
    assert_eq!(
        name(store::read_file(&dir, ID, "../x", DevMode::Off)),
        "package_path_invalid"
    );
}

#[test]
fn a_deleted_folder_is_folder_missing() {
    let scratch = Scratch::new("deleted");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    std::fs::remove_dir_all(&src).unwrap();
    assert_eq!(name(read(&dir, "lib/deep/main.js")), "folder_missing");
    assert_eq!(
        store::folder_snapshot(&src).unwrap_err().name(),
        "folder_missing"
    );
    // A file where the folder was is not a folder either.
    std::fs::write(&src, b"x").unwrap();
    assert_eq!(name(read(&dir, "lib/deep/main.js")), "folder_missing");
    // A file missing inside a folder that is there is a bad path.
    std::fs::remove_file(&src).unwrap();
    folder(&scratch.0, "src");
    assert_eq!(name(read(&dir, "lib/deep/nope.js")), "package_path_invalid");
}

#[cfg(unix)]
#[test]
fn a_root_swapped_for_a_link_after_install_is_refused() {
    let scratch = Scratch::new("root-link");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    let decoy = decoy(&scratch.0);
    std::fs::remove_dir_all(&src).unwrap();
    std::os::unix::fs::symlink(&decoy, &src).unwrap();
    assert_eq!(name(read(&dir, "lib/deep/main.js")), "package_path_invalid");
    assert_eq!(name(read(&dir, "hew-plugin.json")), "package_path_invalid");
    assert_eq!(
        store::folder_snapshot(&src).unwrap_err().name(),
        "package_path_invalid"
    );
    // A picked folder may not be a link either.
    assert_eq!(
        store::read_package_dir(&src).unwrap_err().name(),
        "package_path_invalid"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_above_the_root_swapped_for_a_link_is_refused() {
    let scratch = Scratch::new("ancestor-link");
    let dir = scratch.store();
    let parent = scratch.0.join("work");
    let src = folder(&parent, "src");
    install(&dir, &src);
    // Same names under another folder, then `work` becomes a link to it:
    // the recorded path still reaches a real folder, but not the one
    // picked.
    let elsewhere = scratch.0.join("elsewhere");
    std::fs::rename(&parent, &elsewhere).unwrap();
    std::fs::write(elsewhere.join("src/lib/deep/main.js"), b"secret").unwrap();
    std::os::unix::fs::symlink(&elsewhere, &parent).unwrap();
    assert_eq!(name(read(&dir, "lib/deep/main.js")), "package_path_invalid");
}

#[cfg(unix)]
#[test]
fn a_link_at_any_segment_is_refused() {
    for segment in ["lib", "lib/deep", "lib/deep/main.js"] {
        let scratch = Scratch::new("segment-link");
        let dir = scratch.store();
        let src = folder(&scratch.0, "src");
        install(&dir, &src);
        let decoy = decoy(&scratch.0);
        let at = src.join(segment);
        if at.is_dir() {
            std::fs::remove_dir_all(&at).unwrap();
        } else {
            std::fs::remove_file(&at).unwrap();
        }
        std::os::unix::fs::symlink(decoy.join(segment), &at).unwrap();
        assert_eq!(
            name(read(&dir, "lib/deep/main.js")),
            "package_path_invalid",
            "{segment}"
        );
        // The watcher lists the link and does not enter it.
        let snap = store::folder_snapshot(&src).unwrap();
        assert_eq!(snap.0[segment].0, 'l', "{segment}");
        assert!(
            !snap.0.keys().any(|k| k.starts_with(&format!("{segment}/"))),
            "{segment}: {:?}",
            snap.0.keys()
        );
        // A review reads the folder under the package rules: refused.
        assert!(store::read_package_dir(&src).is_err(), "{segment}");
    }
}

/// A file swapped for a link to a file outside (and a folder on the way
/// swapped for a link to a folder outside) as fast as a thread can, while
/// another reads: every read answers the folder's own bytes or a refusal,
/// never the outside file's. The window between the checks and the open
/// cannot be hit on purpose; this hits it as often as it can.
#[cfg(unix)]
#[test]
fn a_file_swapped_between_the_check_and_the_open_is_never_read() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let scratch = Scratch::new("swap");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    let decoy = decoy(&scratch.0);
    let stop = AtomicBool::new(false);
    let (mut inside, mut refused) = (0usize, 0usize);
    std::thread::scope(|s| {
        s.spawn(|| {
            let target = src.join("lib/deep/main.js");
            let deep = src.join("lib/deep");
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                n += 1;
                let link = src.join(format!("lib/link-{n}"));
                let file = src.join(format!("lib/file-{n}"));
                // The file, then a link to the outside one, renamed over.
                let _ = std::os::unix::fs::symlink(decoy.join("lib/deep/main.js"), &link);
                let _ = std::fs::rename(&link, &target);
                let _ = std::fs::write(&file, b"// inside");
                let _ = std::fs::rename(&file, &target);
                // The folder on the way, swapped for a link and back.
                let aside = src.join(format!("lib/aside-{n}"));
                let _ = std::fs::rename(&deep, &aside);
                let _ = std::os::unix::fs::symlink(decoy.join("lib/deep"), &deep);
                let _ = std::fs::remove_file(&deep);
                let _ = std::fs::rename(&aside, &deep);
            }
        });
        for _ in 0..3000 {
            match read(&dir, "lib/deep/main.js") {
                Ok(bytes) => {
                    assert_eq!(bytes, b"// inside", "the outside file was read");
                    inside += 1;
                }
                Err(e) => {
                    assert_eq!(e.name(), "package_path_invalid", "{e}");
                    refused += 1;
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
    });
    assert_eq!(inside + refused, 3000);
}

#[test]
fn a_marked_plugin_reads_nothing_from_its_folder() {
    let scratch = Scratch::new("marked");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "permissions": ["model.read", "view"] })),
    )
    .unwrap();
    let found = read(&dir, "hew-plugin.json");
    assert_eq!(
        store::apply_folder_manifest(&dir, ID, found, &running()).unwrap(),
        FolderOutcome::Marked("permissions_changed")
    );
    for rel in ["lib/deep/main.js", "hew-plugin.json"] {
        assert_eq!(name(read(&dir, rel)), "dev_consent_pending", "{rel}");
    }
    let (record, stored) = store::installed(&dir, ID).unwrap();
    assert_eq!(record.extra[NEEDS_CONSENT], json!(true));
    assert_eq!(
        record.extra[NEEDS_CONSENT_REASON],
        json!("permissions_changed")
    );
    assert!(store::needs_consent(&record));
    assert_eq!(stored.permissions.len(), 1, "the stored manifest is kept");
    assert_eq!(record.grant.len(), 1, "the grant is kept");
    // A damaged marker still blocks; only an explicit false clears it.
    for (value, blocked) in [
        (json!("yes"), true),
        (json!(1), true),
        (json!(false), false),
    ] {
        let mut r = record.clone();
        r.extra.insert(NEEDS_CONSENT.into(), value.clone());
        assert_eq!(store::needs_consent(&r), blocked, "{value}");
    }
}

#[test]
fn the_watcher_listing_follows_no_link_and_stops_at_the_caps() {
    let scratch = Scratch::new("snapshot");
    let src = folder(&scratch.0, "src");
    let a = store::folder_snapshot(&src).unwrap();
    assert_eq!(
        a.0.keys().map(String::as_str).collect::<Vec<_>>(),
        ["hew-plugin.json", "lib", "lib/deep", "lib/deep/main.js"]
    );
    assert_eq!(store::folder_snapshot(&src).unwrap(), a, "stable");
    std::fs::write(src.join("lib/deep/main.js"), b"// inside, longer").unwrap();
    assert_ne!(
        store::folder_snapshot(&src).unwrap(),
        a,
        "a size change shows"
    );
    #[cfg(unix)]
    {
        // A link to a big folder outside is one entry, not entered.
        let outside = scratch.0.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        for n in 0..10 {
            std::fs::write(outside.join(format!("{n}")), b"x").unwrap();
        }
        std::os::unix::fs::symlink(&outside, src.join("out")).unwrap();
        let s = store::folder_snapshot(&src).unwrap();
        assert_eq!(s.0["out"].0, 'l');
        assert!(!s.0.keys().any(|k| k.starts_with("out/")));
        std::fs::remove_file(src.join("out")).unwrap();
    }
    // One file over the per-file cap.
    let big = std::fs::File::create(src.join("big.bin")).unwrap();
    big.set_len(plugin_host::MAX_ENTRY_BYTES as u64 + 1)
        .unwrap();
    drop(big);
    assert_eq!(
        store::folder_snapshot(&src).unwrap_err().name(),
        "package_too_large"
    );
    std::fs::remove_file(src.join("big.bin")).unwrap();
    // Too many entries.
    let many = src.join("many");
    std::fs::create_dir_all(&many).unwrap();
    for n in 0..plugin_host::MAX_PACKAGE_ENTRIES {
        std::fs::write(many.join(format!("{n}")), b"").unwrap();
    }
    assert_eq!(
        store::folder_snapshot(&src).unwrap_err().name(),
        "package_too_large"
    );
}

#[test]
fn a_review_is_accepted_in_place() {
    let scratch = Scratch::new("review");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    let (before, _) = store::installed(&dir, ID).unwrap();
    // Nothing to review yet.
    assert_eq!(
        store::read_review(&dir, ID, DevMode::On)
            .unwrap_err()
            .name(),
        "no_review_pending"
    );
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "permissions": ["view"], "version": "1.1.0" })),
    )
    .unwrap();
    let found = read(&dir, "hew-plugin.json");
    store::apply_folder_manifest(&dir, ID, found, &running()).unwrap();
    let (package, source) = store::read_review(&dir, ID, DevMode::On).unwrap();
    let pending = store::stage_review(&dir, package, source, &running(), 3_000).unwrap();
    assert!(pending.consent.review);
    assert_eq!(
        pending.consent.review_line.as_deref(),
        Some(plugin_host::REVIEW_LINE),
        "a permissions change is the permissions line"
    );
    let update = pending.consent.update.clone().unwrap();
    assert_eq!(update.added[0].name, "view");
    assert_eq!(update.removed[0].name, "model.read");
    // A pending that is too old, or not a review, is refused.
    let mut old = pending.clone();
    old.created_at_ms = 0;
    assert_eq!(
        store::commit_review(&dir, old, store::PENDING_TTL_MS + 1)
            .unwrap_err()
            .name(),
        "pending_expired"
    );
    let mut plain = pending.clone();
    plain.consent.review = false;
    assert_eq!(
        store::commit_review(&dir, plain, 3_000).unwrap_err().name(),
        "pending_unknown"
    );
    let record = store::commit_review(&dir, pending.clone(), 3_000).unwrap();
    assert!(!store::needs_consent(&record));
    assert!(!record.extra.contains_key(NEEDS_CONSENT_REASON));
    assert_eq!(record.installed_at_ms, before.installed_at_ms);
    assert_eq!(record.version.to_string(), "1.1.0");
    assert_eq!(
        record.grant.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        ["view"]
    );
    let (_, stored) = store::installed(&dir, ID).unwrap();
    assert_eq!(stored.version.to_string(), "1.1.0");
    assert!(
        !dir.root().join(ID).join("previous").exists(),
        "no rotation"
    );
    assert_eq!(read(&dir, "lib/deep/main.js").unwrap(), b"// inside");
    // The same review again is stale: the record it was staged against
    // is gone.
    assert_eq!(
        store::commit_review(&dir, pending, 3_000)
            .unwrap_err()
            .name(),
        "pending_stale"
    );
}

/// The folder's manifest bytes, as the shell's comparison reads them.
fn folder_manifest(src: &Path) -> Result<Vec<u8>, InstallError> {
    Ok(std::fs::read(src.join("hew-plugin.json")).unwrap())
}

fn marked(dir: &PluginDir) -> bool {
    store::needs_consent(&store::installed(dir, ID).unwrap().0)
}

#[test]
fn a_mark_is_cleared_when_the_folder_manifest_is_put_back() {
    let scratch = Scratch::new("unmark");
    let dir = scratch.store();
    let src = folder(&scratch.0, "src");
    install(&dir, &src);
    let original = std::fs::read(src.join("hew-plugin.json")).unwrap();
    let apply = |dir: &PluginDir| {
        store::apply_folder_manifest(dir, ID, folder_manifest(&src), &running()).unwrap()
    };
    // A typo marks it; fixing the typo clears the mark.
    std::fs::write(src.join("hew-plugin.json"), b"{ not json").unwrap();
    assert_eq!(apply(&dir), FolderOutcome::Marked("manifest_unreadable"));
    std::fs::write(src.join("hew-plugin.json"), &original).unwrap();
    assert_eq!(apply(&dir), FolderOutcome::Cleared);
    assert!(!marked(&dir));
    assert_eq!(read(&dir, "lib/deep/main.js").unwrap(), b"// inside");
    // A permission added and taken out again: the same.
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "permissions": ["model.read", "view"] })),
    )
    .unwrap();
    assert_eq!(apply(&dir), FolderOutcome::Marked("permissions_changed"));
    // Another change still needing review keeps the mark, with its reason.
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "permissions": ["view"] })),
    )
    .unwrap();
    assert_eq!(apply(&dir), FolderOutcome::Unchanged);
    let (record, _) = store::installed(&dir, ID).unwrap();
    assert!(store::needs_consent(&record));
    assert_eq!(
        store::needs_consent_reason(&record),
        Some("permissions_changed")
    );
    std::fs::write(src.join("hew-plugin.json"), b"{").unwrap();
    assert_eq!(apply(&dir), FolderOutcome::Marked("manifest_unreadable"));
    assert_eq!(
        store::needs_consent_reason(&store::installed(&dir, ID).unwrap().0),
        Some("manifest_unreadable")
    );
    // Put back with only a version bump: cleared, and the copy refreshed.
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "version": "1.0.1" })),
    )
    .unwrap();
    assert_eq!(apply(&dir), FolderOutcome::Cleared);
    let (record, stored) = store::installed(&dir, ID).unwrap();
    assert!(!store::needs_consent(&record));
    assert!(!record.extra.contains_key(NEEDS_CONSENT_REASON));
    assert_eq!(record.version.to_string(), "1.0.1");
    assert_eq!(stored.version.to_string(), "1.0.1");
}

/// One field of the manifest changed in the folder, and whether the
/// change needs the person's review (and why) or refreshes the copy.
#[test]
fn every_manifest_field_either_needs_review_or_refreshes() {
    let command = |extra: serde_json::Value| {
        let mut c = json!({
            "id": "count", "class": "read_only", "summary": "Counts.",
            "params": { "type": "object", "additionalProperties": false },
            "result": { "type": "object", "additionalProperties": false }
        });
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        json!({ "commands": [c] })
    };
    let base = json!({ "contributes": command(json!({})) });
    let rows: Vec<(&str, serde_json::Value, Option<&str>)> = vec![
        (
            "name",
            json!({ "name": "Hew Official Exporter" }),
            Some("consent_text_changed"),
        ),
        (
            "activationReason",
            json!({ "activationReason": "Trust me." }),
            Some("consent_text_changed"),
        ),
        (
            "command class",
            json!({ "contributes": command(json!({ "class": "mutating" })) }),
            Some("authority_changed"),
        ),
        (
            "command permission",
            json!({ "contributes": command(json!({ "permission": "model.write" })) }),
            Some("authority_changed"),
        ),
        (
            "presave",
            json!({ "contributes": { "commands": base["contributes"]["commands"], "presave": true } }),
            Some("authority_changed"),
        ),
        (
            "menus",
            json!({ "contributes": { "commands": base["contributes"]["commands"], "menus": [] } }),
            Some("authority_changed"),
        ),
        ("version", json!({ "version": "1.2.0" }), None),
        (
            "description",
            json!({ "description": "Now with more." }),
            None,
        ),
        ("main", json!({ "main": "lib/deep/other.js" }), None),
        (
            "command summary",
            json!({ "contributes": command(json!({ "summary": "Counts things." })) }),
            None,
        ),
        (
            "command timeout",
            json!({ "contributes": command(json!({ "timeout_ms": 1000 })) }),
            None,
        ),
        (
            "command added",
            json!({ "contributes": { "commands": [
            base["contributes"]["commands"][0],
            { "id": "other", "class": "mutating", "summary": "Other.",
              "params": { "type": "object", "additionalProperties": false },
              "result": { "type": "object", "additionalProperties": false } }
        ] } }),
            None,
        ),
        (
            "command added with a weaker permission",
            json!({ "contributes": { "commands": [
            base["contributes"]["commands"][0],
            { "id": "other", "class": "mutating", "summary": "Other.", "permission": "model.read",
              "params": { "type": "object", "additionalProperties": false },
              "result": { "type": "object", "additionalProperties": false } }
        ] } }),
            Some("authority_changed"),
        ),
        (
            "command removed",
            json!({ "contributes": { "commands": [] } }),
            None,
        ),
        (
            "activation without startup",
            json!({ "activation": ["on_command"] }),
            None,
        ),
    ];
    for (what, change, reason) in rows {
        let scratch = Scratch::new("fields");
        let dir = scratch.store();
        let src = folder(&scratch.0, "src");
        std::fs::write(src.join("hew-plugin.json"), manifest(base.clone())).unwrap();
        install(&dir, &src);
        let mut edited = base.clone();
        for (k, v) in change.as_object().unwrap() {
            edited[k] = v.clone();
        }
        std::fs::write(src.join("hew-plugin.json"), manifest(edited)).unwrap();
        let outcome =
            store::apply_folder_manifest(&dir, ID, folder_manifest(&src), &running()).unwrap();
        match reason {
            Some(reason) => assert_eq!(outcome, FolderOutcome::Marked(reason), "{what}"),
            None => assert_eq!(outcome, FolderOutcome::Refreshed, "{what}"),
        }
    }
}

#[test]
fn a_command_name_another_plugin_holds_is_refused_at_install_and_at_refresh() {
    let scratch = Scratch::new("collide");
    let dir = scratch.store();
    let command = |id: &str| {
        json!({ "commands": [{
            "id": id, "class": "read_only", "summary": "Runs.",
            "params": { "type": "object", "additionalProperties": false },
            "result": { "type": "object", "additionalProperties": false }
        }] })
    };
    // com.example.folder.tool holds `run`: com.example.folder.tool.run.
    let other = scratch.0.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("hew-plugin.json"),
        manifest(json!({ "id": "com.example.folder.tool", "main": "main.js", "contributes": command("run") })),
    )
    .unwrap();
    std::fs::write(other.join("main.js"), b"//").unwrap();
    let package = store::read_package_dir(&other).unwrap();
    let source = InstallSource::Dir {
        path: other.to_string_lossy().into_owned(),
    };
    let pending = store::stage(&dir, package, source, &running(), 1_000).unwrap();
    store::commit(&dir, pending, true, 2_000).unwrap().unwrap();

    // Installing a plugin whose `tool.run` is the same name is refused.
    let src = folder(&scratch.0, "src");
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "contributes": command("tool.run") })),
    )
    .unwrap();
    let package = store::read_package_dir(&src).unwrap();
    let source = InstallSource::Dir {
        path: src.to_string_lossy().into_owned(),
    };
    assert_eq!(
        store::stage(&dir, package, source, &running(), 1_000)
            .unwrap_err()
            .name(),
        "command_name_taken"
    );
    // Installed without it, then the folder adds it: marked, not refreshed.
    std::fs::write(src.join("hew-plugin.json"), manifest(json!({}))).unwrap();
    install(&dir, &src);
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "contributes": command("tool.run") })),
    )
    .unwrap();
    assert_eq!(
        store::apply_folder_manifest(&dir, ID, folder_manifest(&src), &running()).unwrap(),
        FolderOutcome::Marked("command_name_taken")
    );
    // Renamed: the mark clears and the copy follows.
    std::fs::write(
        src.join("hew-plugin.json"),
        manifest(json!({ "contributes": command("tool.go") })),
    )
    .unwrap();
    assert_eq!(
        store::apply_folder_manifest(&dir, ID, folder_manifest(&src), &running()).unwrap(),
        FolderOutcome::Cleared
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_text_stops_the_watcher_listing() {
    use std::os::unix::ffi::OsStrExt;
    let scratch = Scratch::new("bytes");
    let src = folder(&scratch.0, "src");
    for b in [0xffu8, 0xfe] {
        let bytes = [b'm', b, b'.', b'j', b's'];
        std::fs::write(src.join(std::ffi::OsStr::from_bytes(&bytes)), b"x").unwrap();
    }
    assert_eq!(
        store::folder_snapshot(&src).unwrap_err().name(),
        "package_path_invalid"
    );
}
