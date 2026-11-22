//! The consent payload: every permission line, both warnings, the update
//! diff, the unsigned and replacement lines, how plugin text is cleaned,
//! and the checks an install makes against what is installed.

#![cfg(hew_preview)]

use plugin_host::{
    ConsentPayload, InstallError, InstallSource, Installed, MAX_NAME_CHARS, MAX_REASON_CHARS,
    UNSIGNED_LINE, check_requires, check_source_id, clean_text, consent, consent_summary,
    enable_summary, hew_version_ok, install_confirmation, is_reinstall, permission_line,
    reinstall_line, replaces_line, uninstall_summary,
};
use plugin_manifest::{Manifest, PermissionName};
use serde_json::json;

fn manifest(extra: serde_json::Value) -> Manifest {
    let mut m = json!({
        "manifest": 1, "id": "com.example.fixture", "name": "Fixture",
        "version": "2.0.0", "api": 1, "main": "main.js"
    });
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    plugin_manifest::parse(&m.to_string()).expect("a valid manifest")
}

fn file() -> InstallSource {
    InstallSource::File {
        name: "fixture.hewplugin".into(),
    }
}

fn p(name: &str) -> PermissionName {
    PermissionName::parse(name).unwrap()
}

fn installed(id: &str, version: &str, grant: &[&str]) -> Installed {
    serde_json::from_value(json!({
        "schema": 1, "id": id, "version": version, "source": { "kind": "file", "name": "x" },
        "installed_at_ms": 1, "grant": grant, "enabled": true, "data_version": 1
    }))
    .unwrap()
}

#[test]
fn every_permission_line_reads_exactly() {
    let table = [
        (
            "model.read",
            "Read the model: every object, component, material, tag, and attribute",
        ),
        (
            "model.write",
            "Change the model: draw, move, delete, and edit geometry",
        ),
        (
            "attributes.write",
            "Store its own data on parts, materials, and the document",
        ),
        (
            "attributes.write:com.other.app",
            "Store data under the \"com.other.app\" namespace",
        ),
        (
            "view",
            "Change the selection, hide and show parts, and move the camera",
        ),
        ("documents.save", "Save, export, and import files"),
        ("documents.discard", "Close or revert documents"),
        (
            "documents.other",
            "Act on every open document, not only the current one",
        ),
        ("files", "Ask you to pick files, and save files you choose"),
        ("clipboard", "Read and write the clipboard"),
        ("library", "Add to and change your Library"),
        (
            "network:api.example.com",
            "Connect to api.example.com (checked by Hew)",
        ),
    ];
    for (name, text) in table {
        let line = permission_line(&p(name));
        assert_eq!(line.name, name);
        assert_eq!(line.text, text, "{name}");
    }
    // Every bare name in the vocabulary has a line of its own.
    for name in plugin_manifest::PERMISSION_NAMES {
        assert_ne!(permission_line(&p(name)).text, *name, "{name} has no copy");
    }
}

#[test]
fn the_model_and_network_warning() {
    let m = manifest(
        json!({ "permissions": ["model.read", "network:a.example.com", "network:b.example.com"] }),
    );
    let c = consent(&m, &file(), None, None, None);
    assert_eq!(
        c.warnings,
        ["Can send what it reads from your model to a.example.com, b.example.com."]
    );
}

#[test]
fn the_files_and_network_warning() {
    let m = manifest(json!({ "permissions": ["files", "network:up.example.com"] }));
    let c = consent(&m, &file(), None, None, None);
    assert_eq!(c.warnings, ["Can send files you choose to up.example.com."]);
}

#[test]
fn both_warnings_and_optional_permissions_count() {
    // The host only among optional permissions, model.read only among
    // required ones: still warned, because the plugin may ask later.
    let m = manifest(json!({
        "permissions": ["model.read", "files"],
        "optionalPermissions": ["network:x.example.com"]
    }));
    let c = consent(&m, &file(), None, None, None);
    assert_eq!(
        c.warnings,
        [
            "Can send what it reads from your model to x.example.com.",
            "Can send files you choose to x.example.com."
        ]
    );
    assert_eq!(c.optional.len(), 1);
    // No network, no warning.
    let m = manifest(json!({ "permissions": ["model.read", "files"] }));
    assert!(consent(&m, &file(), None, None, None).warnings.is_empty());
}

#[test]
fn a_fresh_install_is_unsigned_and_replaces_nothing() {
    let m = manifest(json!({ "permissions": ["model.read"] }));
    let c = consent(&m, &file(), None, None, None);
    assert!(c.unsigned);
    assert_eq!(c.signer, None);
    assert_eq!(c.replaces, None);
    assert_eq!(c.update, None);
    assert_eq!(c.name, "Fixture");
    assert_eq!(c.version, "2.0.0");
    assert_eq!(UNSIGNED_LINE, "Unsigned. Hew cannot tell who made this.");
    let signed = consent(&m, &file(), None, None, Some("Example Co"));
    assert!(!signed.unsigned);
    assert_eq!(signed.signer.as_deref(), Some("Example Co"));
}

#[test]
fn the_update_diff_and_the_replacement_line() {
    let old = manifest(json!({
        "version": "1.0.0",
        "name": "Old Name",
        "permissions": ["model.read"],
        "optionalPermissions": ["clipboard"]
    }));
    let new = manifest(json!({
        "permissions": ["model.read", "model.write"],
        "optionalPermissions": ["clipboard", "library"],
        "activation": ["on_command", "on_startup"],
        "activationReason": "Keeps the cut list current."
    }));
    let previous = installed("com.example.fixture", "1.0.0", &["model.read"]);
    let c = consent(&new, &file(), Some(&previous), Some(&old), None);
    let update = c.update.expect("an update");
    assert_eq!(
        update
            .added
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["model.write"]
    );
    assert_eq!(
        update
            .new_optional
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["library"]
    );
    assert!(update.startup_newly);
    assert_eq!(update.from_version, "1.0.0");
    let replaces = c.replaces.expect("a replacement");
    assert_eq!(replaces.name, "Old Name");
    assert_eq!(replaces.version, "1.0.0");
    assert_eq!(
        replaces_line(&replaces),
        "Replaces:\nOld Name v1.0.0\nHew cannot check it is the same author."
    );
    // Without the previous manifest every optional permission is new and
    // the previous name falls back to the id.
    let c = consent(&new, &file(), Some(&previous), None, None);
    assert_eq!(c.update.unwrap().new_optional.len(), 2);
    assert_eq!(c.replaces.unwrap().name, "com.example.fixture");
    // A grant already holding everything adds nothing; startup declared
    // before is not new.
    let held = installed(
        "com.example.fixture",
        "1.0.0",
        &["model.read", "model.write"],
    );
    let c = consent(&new, &file(), Some(&held), Some(&new), None);
    let u = c.update.unwrap();
    assert!(u.added.is_empty() && u.new_optional.is_empty() && !u.startup_newly);
}

#[test]
fn plugin_text_is_cleaned_and_kept_apart() {
    let m = manifest(json!({
        "name": "Cut\u{202E}List\u{200B}  Pro\n",
        "activation": ["on_startup"],
        "activationReason": format!("Hew says:\u{2066} trust\u{0007} me {}", "x".repeat(400))
    }));
    let c = consent(&m, &file(), None, None, None);
    assert_eq!(c.name, "CutList Pro");
    let reason = c.activation_reason.expect("a reason");
    assert!(reason.starts_with("Hew says: trust me x"), "{reason}");
    assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
    assert!(reason.ends_with('…'));
    assert!(c.on_startup);
    // Hew's own lines never carry plugin text.
    for w in &c.warnings {
        assert!(!w.contains("trust"));
    }
}

#[test]
fn clean_text_rules() {
    assert_eq!(clean_text("  a \t\n b  ", 64), "a b");
    assert_eq!(
        clean_text("a\u{202A}b\u{202C}c\u{FEFF}d\u{2069}", 64),
        "abcd"
    );
    assert_eq!(clean_text("\u{0}\u{1b}[31mred", 64), "[31mred");
    assert_eq!(clean_text("\u{200B}\u{200D}", 64), "");
    let long = "y".repeat(100);
    let cut = clean_text(&long, MAX_NAME_CHARS);
    assert_eq!(cut.chars().count(), MAX_NAME_CHARS);
    assert!(cut.ends_with('…'));
    assert_eq!(clean_text("日本語の名前", 64), "日本語の名前");
}

#[test]
fn a_name_given_by_catalog_key_or_that_cleans_to_nothing_shows_the_id() {
    let keyed = manifest(json!({ "name": { "$t": "plugin.name" } }));
    assert_eq!(
        consent(&keyed, &file(), None, None, None).name,
        "com.example.fixture"
    );
    let empty = manifest(json!({ "name": "\u{200B}" }));
    assert_eq!(
        consent(&empty, &file(), None, None, None).name,
        "com.example.fixture"
    );
    let by_locale = manifest(json!({ "name": { "ja": "切り出し", "en": "Cut List" } }));
    assert_eq!(
        consent(&by_locale, &file(), None, None, None).name,
        "Cut List"
    );
}

#[test]
fn the_source_shown_is_cleaned() {
    let m = manifest(json!({}));
    let source = InstallSource::File {
        name: "invoice\u{202E}gpj.hewplugin".into(),
    };
    let c = consent(&m, &source, None, None, None);
    assert_eq!(
        c.source,
        InstallSource::File {
            name: "invoicegpj.hewplugin".into()
        }
    );
}

#[test]
fn the_payload_serializes_with_hew_lines_and_plugin_text_in_separate_fields() {
    let m = manifest(json!({
        "permissions": ["model.read", "network:api.example.com"],
        "activation": ["on_startup"],
        "activationReason": "Because."
    }));
    let c: ConsentPayload = consent(&m, &file(), None, None, None);
    let v = serde_json::to_value(&c).unwrap();
    for key in [
        "id",
        "name",
        "version",
        "permissions",
        "optional",
        "warnings",
        "on_startup",
        "activation_reason",
        "signer",
        "unsigned",
        "source",
        "replaces",
        "update",
    ] {
        assert!(v.get(key).is_some(), "{key}");
    }
    assert_eq!(
        v["source"],
        json!({ "kind": "file", "name": "fixture.hewplugin" })
    );
    assert_eq!(
        v["permissions"][0],
        json!({ "name": "model.read", "text": "Read the model: every object, component, material, tag, and attribute" })
    );
    let summary = consent_summary(&c);
    assert!(summary.contains(UNSIGNED_LINE));
    assert!(summary.contains("Read the model"));
    assert!(summary.contains("Can send what it reads"));
    assert!(summary.trim_end().ends_with("The plugin says:\nBecause."));
}

#[test]
fn reserved_ids_are_refused_from_files_and_folders() {
    let m = manifest(json!({ "id": "org.hew3d.cutlist" }));
    for source in [
        file(),
        InstallSource::Dir {
            path: "/tmp/x".into(),
        },
    ] {
        assert_eq!(
            check_source_id(&m, &source),
            Err(InstallError::ReservedId("org.hew3d.cutlist".into()))
        );
    }
    assert!(
        check_source_id(
            &m,
            &InstallSource::Index {
                id: "x".into(),
                url: "u".into()
            }
        )
        .is_ok()
    );
    assert!(check_source_id(&manifest(json!({ "id": "org.hew3dx.cutlist" })), &file()).is_ok());
}

#[test]
fn hew_version_ranges() {
    let any = manifest(json!({}));
    let v = |s: &str| semver::Version::parse(s).unwrap();
    assert!(hew_version_ok(&any, &v("0.1.0")));
    let m = manifest(json!({ "hew": ">=1.4, <2" }));
    assert!(hew_version_ok(&m, &v("1.4.0")));
    assert!(hew_version_ok(&m, &v("1.9.3")));
    assert!(!hew_version_ok(&m, &v("1.3.9")));
    assert!(!hew_version_ok(&m, &v("2.0.0")));
    // A preview build of 1.5.0 counts as 1.5.0.
    assert!(hew_version_ok(&m, &v("1.5.0-pre.2")));
}

#[test]
fn a_preview_build_counts_as_its_release() {
    let v = |s: &str| semver::Version::parse(s).unwrap();
    let ok = |range: &str, running: &str| {
        hew_version_ok(&manifest(json!({ "hew": range })), &v(running))
    };
    assert!(ok(">=1.5.0", "1.5.0-pre.2"));
    assert!(ok(">=1.5.0", "1.5.0-pre.2+beta.ab12cd34"));
    assert!(ok(">=1.5.0", "1.5.0+beta.ab12cd34"));
    assert!(ok("^1.5", "1.5.0-pre.2"));
    assert!(!ok(">=1.5.0", "1.4.0"));
    assert!(!ok(">=1.5.1", "1.5.0-pre.2"));
    assert!(!ok("<1.5.0", "1.5.0-pre.2"));
    assert!(!ok("<1.5.0", "1.5.0-pre.2+beta.ab12cd34"));
    // A range that names a preview of this release is honored as written:
    // an earlier preview does not pass it, the named one and later do.
    assert!(ok(">=1.5.0-pre.1", "1.5.0-pre.2"));
    assert!(ok("=1.5.0-pre.2", "1.5.0-pre.2"));
    assert!(!ok(">=1.5.0-pre.3", "1.5.0-pre.2"));
    assert!(!ok("^1.5.0-pre.3", "1.5.0-pre.2"));
    assert!(!ok(">1.5.0-pre.2", "1.5.0-pre.2"));
    assert!(ok(">=1.5.0-pre.3", "1.5.0-pre.3+beta.ab12cd34"));
    assert!(ok(">=1.5.0-pre.3", "1.5.0"));
    // A pre-release of an older release named in the range does not block
    // a later preview.
    assert!(ok(">=1.4.0-pre.1", "1.5.0-pre.2"));
}

fn with_requires(id: &str, version: &str, requires: &[(&str, &str)]) -> (Installed, Manifest) {
    let m = manifest(json!({
        "id": id,
        "version": version,
        "requires": requires.iter().map(|(i, r)| json!({ "id": i, "range": r })).collect::<Vec<_>>()
    }));
    (installed(id, version, &[]), m)
}

#[test]
fn a_missing_dependency_is_refused() {
    let m = manifest(json!({ "requires": [
        { "id": "com.example.base", "range": "^1" },
        { "id": "com.example.other", "range": "*" }
    ] }));
    let base_old = with_requires("com.example.base", "0.9.0", &[]);
    assert_eq!(
        check_requires(&m, &[base_old]),
        Err(InstallError::DependencyMissing(vec![
            "com.example.base ^1".into(),
            "com.example.other *".into()
        ]))
    );
    let base = with_requires("com.example.base", "1.2.0", &[]);
    let other = with_requires("com.example.other", "3.0.0", &[]);
    assert_eq!(check_requires(&m, &[base, other]), Ok(()));
}

#[test]
fn a_dependency_cycle_is_refused_and_named() {
    // fixture → a → b → fixture
    let a = with_requires("com.example.a", "1.0.0", &[("com.example.b", "*")]);
    let b = with_requires("com.example.b", "1.0.0", &[("com.example.fixture", "*")]);
    let old_self = with_requires("com.example.fixture", "1.0.0", &[]);
    let m = manifest(json!({ "requires": [{ "id": "com.example.a", "range": "*" }] }));
    assert_eq!(
        check_requires(&m, &[a.clone(), b.clone(), old_self]),
        Err(InstallError::DependencyCycle(vec![
            "com.example.fixture".into(),
            "com.example.a".into(),
            "com.example.b".into(),
            "com.example.fixture".into()
        ]))
    );
    // Without the edge back there is no cycle.
    let b_free = with_requires("com.example.b", "1.0.0", &[]);
    let old_self = with_requires("com.example.fixture", "1.0.0", &[]);
    assert_eq!(check_requires(&m, &[a, b_free, old_self]), Ok(()));
}

#[test]
fn installed_records_keep_unknown_fields_and_refuse_newer_schemas() {
    let text = json!({
        "schema": 1, "id": "com.example.fixture", "version": "1.0.0",
        "source": { "kind": "dir", "path": "/x" }, "installed_at_ms": 5,
        "grant": ["model.read"], "enabled": false, "data_version": 3,
        "future_field": { "kept": [1, 2] }
    })
    .to_string();
    let record = Installed::from_json("com.example.fixture", text.as_bytes()).unwrap();
    assert!(!record.startup_allowed, "absent means not allowed");
    assert_eq!(record.extra["future_field"], json!({ "kept": [1, 2] }));
    let back: serde_json::Value = serde_json::from_slice(&record.to_json()).unwrap();
    assert_eq!(back["future_field"], json!({ "kept": [1, 2] }));
    let written = String::from_utf8(record.to_json()).unwrap();
    assert!(
        written.trim_start().starts_with("{\n  \"schema\": 1,"),
        "schema first: {written}"
    );

    let newer = json!({ "schema": 2, "id": "com.example.fixture", "anything": "else" }).to_string();
    assert_eq!(
        Installed::from_json("com.example.fixture", newer.as_bytes())
            .unwrap_err()
            .name(),
        "installed_unreadable"
    );
    let unnumbered = json!({ "id": "com.example.fixture" }).to_string();
    assert!(Installed::from_json("com.example.fixture", unnumbered.as_bytes()).is_err());
    assert!(Installed::from_json("com.example.other", text.as_bytes()).is_err());
}

#[test]
fn the_rollback_summary() {
    let current = installed(
        "com.example.fixture",
        "2.0.0",
        &["model.read", "model.write"],
    );
    let mut previous = installed("com.example.fixture", "1.0.0", &["model.read"]);
    previous.optional_granted = vec![p("clipboard")];
    let m = manifest(json!({ "version": "1.0.0", "name": "Cut\u{202E}List" }));
    assert_eq!(
        plugin_host::rollback_summary(&current, &previous, &m),
        "CutList\nGoes back from v2.0.0 to v1.0.0.\n\nIt will be able to:\n\
         • Read the model: every object, component, material, tag, and attribute\n\
         • Read and write the clipboard\n"
    );
    let bare = installed("com.example.fixture", "1.0.0", &[]);
    assert_eq!(
        plugin_host::rollback_summary(&current, &bare, &m),
        "CutList\nGoes back from v2.0.0 to v1.0.0.\n"
    );
}

#[test]
fn clean_text_matches_the_shared_fixture() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/clean_text.json"
    ))
    .unwrap();
    let cases: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let input = case["in"].as_str().unwrap();
        let max = case["max"].as_u64().unwrap() as usize;
        assert_eq!(
            clean_text(input, max),
            case["out"].as_str().unwrap(),
            "{}: {input:?}",
            case["note"]
        );
    }
}

#[test]
fn the_consent_summary_carries_the_sheet_in_order() {
    let old = manifest(json!({
        "version": "1.0.0",
        "permissions": ["model.read"]
    }));
    let new = manifest(json!({
        "permissions": ["model.read", "files", "network:a.example.com"],
        "optionalPermissions": ["clipboard"],
        "activation": ["on_startup"],
        "activationReason": "Syncs the cut list."
    }));
    let previous = installed("com.example.fixture", "1.0.0", &["model.read"]);
    let source = InstallSource::Dir {
        path: "/home/me/fixture".into(),
    };
    let c = consent(&new, &source, Some(&previous), Some(&old), None);
    assert_eq!(
        consent_summary(&c),
        "Fixture 2.0.0\n\
         Developer folder:\n/home/me/fixture\n\
         The files in this folder can change after you install.\n\
         Unsigned. Hew cannot tell who made this.\n\
         Replaces:\nFixture v1.0.0\nHew cannot check it is the same author.\n\
         \n\
         ⚠ Can send what it reads from your model to a.example.com.\n\
         ⚠ Can send files you choose to a.example.com.\n\
         \n\
         New since v1.0.0:\n\
         • Ask you to pick files, and save files you choose\n\
         • Connect to a.example.com (checked by Hew)\n\
         • Read and write the clipboard\n\
         \n\
         Now starts with Hew.\n\
         \n\
         It can:\n\
         • Read the model: every object, component, material, tag, and attribute\n\
         • Ask you to pick files, and save files you choose\n\
         • Connect to a.example.com (checked by Hew)\n\
         \n\
         May ask for later:\n\
         • Read and write the clipboard\n\
         \n\
         The plugin says:\n\
         Syncs the cut list.\n"
    );
    // A fresh install from a file that starts with Hew: no folder line, no
    // update list, "Starts with Hew."
    let fresh = consent(&new, &file(), None, None, None);
    let text = consent_summary(&fresh);
    assert!(text.contains("From file:\nfixture.hewplugin\n"));
    assert!(!text.contains("folder can change"));
    assert!(!text.contains("New since"));
    assert!(text.contains("\nStarts with Hew.\n"));
}

#[test]
fn summaries_stay_bounded() {
    let many: Vec<String> = (0..20)
        .map(|i| format!("network:h{i}.example.com"))
        .collect();
    let mut permissions = vec!["model.read".to_string()];
    permissions.extend(many.iter().cloned());
    let m = manifest(json!({ "permissions": permissions }));
    let c = consent(&m, &file(), None, None, None);
    assert_eq!(
        c.warnings,
        [
            "Can send what it reads from your model to h0.example.com, h1.example.com, \
          h2.example.com, h3.example.com, h4.example.com, and 15 more."
        ]
    );
    let text = consent_summary(&c);
    assert_eq!(text.matches("\n• ").count(), 12, "{text}");
    assert!(text.contains("…and 9 more\n"), "{text}");
}

#[test]
fn the_rollback_summary_warns_from_the_previous_manifest() {
    let current = installed("com.example.fixture", "2.0.0", &["model.read"]);
    let previous = installed(
        "com.example.fixture",
        "1.0.0",
        &["model.read", "network:x.example.com"],
    );
    let m = manifest(json!({
        "version": "1.0.0",
        "permissions": ["model.read", "network:x.example.com"]
    }));
    assert_eq!(
        plugin_host::rollback_summary(&current, &previous, &m),
        "Fixture\nGoes back from v2.0.0 to v1.0.0.\n\
         \n\
         ⚠ Can send what it reads from your model to x.example.com.\n\
         \n\
         It will be able to:\n\
         • Read the model: every object, component, material, tag, and attribute\n\
         • Connect to x.example.com (checked by Hew)\n"
    );
}

#[test]
fn the_grant_change_summary_lists_what_is_added_and_warns_on_the_result() {
    let added = [p("network:up.example.com")];
    let resulting = [p("files"), p("network:up.example.com")];
    assert_eq!(
        plugin_host::grant_change_summary("Fixture 2.0.0", &added, &resulting),
        "Fixture 2.0.0\n\
         \n\
         It will also be able to:\n\
         • Connect to up.example.com (checked by Hew)\n\
         \n\
         ⚠ Can send files you choose to up.example.com.\n"
    );
    // Nothing that warns, no warning.
    assert_eq!(
        plugin_host::grant_change_summary("Fixture 2.0.0", &[p("view")], &[p("view")]),
        "Fixture 2.0.0\n\nIt will also be able to:\n• Change the selection, hide and show parts, and move the camera\n"
    );
}

#[test]
fn the_confirmation_names_a_fresh_install_an_update_and_a_reinstall() {
    let m = manifest(json!({ "permissions": ["model.read"] }));
    let fresh = consent(&m, &file(), None, None, None);
    assert!(!is_reinstall(&fresh));
    assert_eq!(
        install_confirmation(&fresh),
        ("Install this plugin?", "Install")
    );

    let old = installed("com.example.fixture", "1.0.0", &["model.read"]);
    let update = consent(&m, &file(), Some(&old), Some(&m), None);
    assert!(!is_reinstall(&update));
    assert_eq!(
        install_confirmation(&update),
        ("Update this plugin?", "Update")
    );
    assert!(consent_summary(&update).contains("Replaces:\nFixture v1.0.0\n"));

    // The installed version again: its own title and line, in place of the
    // replacement line.
    let same = installed("com.example.fixture", "2.0.0", &["model.read"]);
    let again = consent(&m, &file(), Some(&same), Some(&m), None);
    assert!(is_reinstall(&again));
    assert_eq!(
        install_confirmation(&again),
        ("Install this plugin again?", "Install")
    );
    assert_eq!(
        reinstall_line("2.0.0"),
        "Replaces the installed copy of v2.0.0."
    );
    let text = consent_summary(&again);
    assert!(
        text.contains("\nReplaces the installed copy of v2.0.0.\n"),
        "{text}"
    );
    assert!(
        !text.contains("Hew cannot check it is the same author"),
        "{text}"
    );
    // Still unsigned, so who made it is still in doubt.
    assert!(text.contains(UNSIGNED_LINE));
}

#[test]
fn the_uninstall_and_turn_on_summaries() {
    let m = manifest(json!({
        "name": "Fixture\u{202e}gnp.exe",
        "permissions": ["model.read", "network:a.example.com"],
        "optionalPermissions": ["clipboard"],
        "activation": ["on_startup"],
        "activationReason": "Not shown here."
    }));
    let mut record = installed(
        "com.example.fixture",
        "2.0.0",
        &["model.read", "network:a.example.com"],
    );
    record.optional_granted = vec![p("clipboard")];
    assert_eq!(
        uninstall_summary(&record, &m),
        "Fixturegnp.exe 2.0.0\n\
         Removes the plugin, its earlier version, and everything it saved.\n"
    );
    assert_eq!(
        enable_summary(&record, &m),
        "Fixturegnp.exe 2.0.0\n\
         Unsigned. Hew cannot tell who made this.\n\
         \n\
         ⚠ Can send what it reads from your model to a.example.com.\n\
         \n\
         Starts with Hew.\n\
         \n\
         It can:\n\
         • Read the model: every object, component, material, tag, and attribute\n\
         • Connect to a.example.com (checked by Hew)\n\
         • Read and write the clipboard\n"
    );
    // The plugin's own words never reach either.
    assert!(!enable_summary(&record, &m).contains("Not shown"));
    // What it holds, not what it declares.
    record.grant = vec![p("model.read")];
    record.optional_granted.clear();
    let text = enable_summary(&record, &m);
    assert!(!text.contains("a.example.com"), "{text}");
}

#[test]
fn the_update_diff_lists_what_was_removed() {
    let old = manifest(json!({
        "version": "1.0.0",
        "permissions": ["model.read", "view"],
        "optionalPermissions": ["clipboard", "library"]
    }));
    let new = manifest(json!({
        "permissions": ["model.read"],
        "optionalPermissions": ["library"]
    }));
    let previous = installed("com.example.fixture", "1.0.0", &["model.read", "view"]);
    let c = consent(&new, &file(), Some(&previous), Some(&old), None);
    let u = c.update.clone().unwrap();
    let names =
        |l: &[plugin_host::PermissionLine]| l.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    assert!(u.added.is_empty() && u.new_optional.is_empty());
    assert_eq!(names(&u.removed), ["view"]);
    assert_eq!(names(&u.removed_optional), ["clipboard"]);
    let summary = consent_summary(&c);
    assert!(
        summary.contains("No longer asks for:\n• Change the selection"),
        "{summary}"
    );
    // Without the previous manifest, the removed ones are read off the
    // grant.
    let c = consent(&new, &file(), Some(&previous), None, None);
    assert_eq!(names(&c.update.unwrap().removed), ["view"]);
    // A fresh install has none, and is not a review.
    let fresh = consent(&new, &file(), None, None, None);
    assert!(fresh.update.is_none() && !fresh.review);
    let v = serde_json::to_value(&fresh).unwrap();
    assert_eq!(v["review"], json!(false));
}

#[test]
fn a_review_reads_as_a_review_even_at_the_same_version() {
    let old = manifest(json!({ "permissions": ["model.read"] }));
    let new = manifest(json!({ "permissions": ["model.read", "view"] }));
    let previous = installed("com.example.fixture", "2.0.0", &["model.read"]);
    let folder = InstallSource::Dir {
        path: "/home/someone/plugin".into(),
    };
    let mut c = consent(&new, &folder, Some(&previous), Some(&old), None);
    c.review = true;
    assert_eq!(
        plugin_host::review_confirmation(),
        ("Allow this plugin's changes?", "Allow")
    );
    let summary = consent_summary(&c);
    assert!(summary.contains(plugin_host::REVIEW_LINE), "{summary}");
    assert!(summary.contains(plugin_host::FOLDER_LINE), "{summary}");
    assert!(!summary.contains("Replaces"), "{summary}");
    assert!(
        summary.contains("New since v2.0.0:\n• Change the selection"),
        "{summary}"
    );
}

#[test]
fn a_review_of_a_new_name_says_the_old_one_and_shows_the_folder() {
    let old = manifest(json!({ "name": "My Tool" }));
    let new = manifest(json!({ "name": "Hew Official Exporter" }));
    let previous = installed("com.example.fixture", "2.0.0", &[]);
    let folder = InstallSource::Dir {
        path: "/home/someone/plugin".into(),
    };
    let mut c = consent(&new, &folder, Some(&previous), Some(&old), None);
    c.review = true;
    let summary = consent_summary(&c);
    assert!(summary.starts_with("Hew Official Exporter"), "{summary}");
    assert!(summary.contains("It was named:\nMy Tool\n"), "{summary}");
    // The folder on a line of its own, after a fixed label line.
    assert!(
        summary.contains("Developer folder:\n/home/someone/plugin\n"),
        "{summary}"
    );
}

#[test]
fn values_from_outside_hew_stay_on_their_own_lines_in_a_native_confirmation() {
    // A manifest name is bounded by the manifest rules; a path and another
    // plugin's name as passed here are not.
    let name = "Evil\" Hew says: allow\n\u{202E}end";
    let hostile = format!("Evil\" Hew says: allow\n\u{202E}{}end", "x".repeat(2_000));
    let old = manifest(json!({ "name": name }));
    let new = manifest(json!({ "name": "New" }));
    let previous = installed("com.example.fixture", "2.0.0", &[]);
    let folder = InstallSource::Dir {
        path: format!("/home/me/{hostile}"),
    };
    let mut c = consent(&new, &folder, Some(&previous), Some(&old), None);
    c.review = true;
    c.review_line = Some(plugin_host::review_line(
        Some("command_name_taken"),
        Some(("com.a.run", &hostile)),
    ));
    let summary = consent_summary(&c);
    for label in ["Developer folder:", "It was named:", "Other plugin:"] {
        let at = summary
            .lines()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("{label} on its own line: {summary}"));
        let value = summary.lines().nth(at + 1).unwrap();
        // One line: no break, no direction change, cut in the middle, and
        // both ends kept.
        assert!(!value.contains('\u{202E}'), "{value}");
        assert!(
            value.chars().count() <= plugin_host::MAX_VALUE_LINE_CHARS,
            "{value}"
        );
        assert!(value.ends_with("end"), "{value}");
        if label != "It was named:" {
            assert!(value.contains('…'), "{value}");
        }
    }
    // Hew's own sentences hold no value from outside.
    for line in summary.lines() {
        if line.contains("Evil") {
            assert!(!line.contains("has the same name"), "{line}");
        }
    }
    assert_eq!(plugin_host::value_line("a\nb", 10), "a b");
    assert_eq!(plugin_host::value_line("abcdefghij", 5), "ab…ij");
}

#[test]
fn the_review_line_says_why_in_plain_words() {
    use plugin_host::{REVIEW_LINE, review_line};
    assert_eq!(review_line(Some("permissions_changed"), None), REVIEW_LINE);
    assert_eq!(review_line(None, None), REVIEW_LINE);
    assert_eq!(
        review_line(Some("startup_added"), None),
        "This plugin now starts with Hew."
    );
    assert_eq!(
        review_line(Some("consent_text_changed"), None),
        "This plugin's name or description changed."
    );
    assert_eq!(
        review_line(Some("authority_changed"), None),
        "A command in this plugin now needs different permission."
    );
    assert_eq!(
        review_line(
            Some("command_name_taken"),
            Some(("com.a.run\u{202E}", "Other\u{7}\nTool"))
        ),
        "A command in this plugin has the same name as one in another plugin.\nCommand:\ncom.a.run\nOther plugin:\nOther Tool"
    );
    // The summary shows the payload's own line, and the permissions line
    // when it carries none.
    let new = manifest(json!({}));
    let previous = installed("com.example.fixture", "2.0.0", &[]);
    let folder = InstallSource::Dir {
        path: "/home/someone/plugin".into(),
    };
    let mut c = consent(&new, &folder, Some(&previous), Some(&new), None);
    c.review = true;
    c.review_line = Some(review_line(Some("startup_added"), None));
    let summary = consent_summary(&c);
    assert!(
        summary.contains("This plugin now starts with Hew.\n"),
        "{summary}"
    );
    assert!(!summary.contains(REVIEW_LINE), "{summary}");
}

#[test]
fn a_folder_is_told_to_fix_the_file_and_a_package_to_install_again() {
    use plugin_host::InstallError;
    use plugin_manifest::ManifestError;
    let bad = InstallError::Manifest(ManifestError::Json("expected value".into()));
    assert!(
        bad.explanation()
            .ends_with("Fix the file and install it again.")
    );
    assert_eq!(
        bad.explanation_for_folder(),
        "hew-plugin.json is not a valid manifest: expected value. Fix the file and Hew reads it again."
    );
    assert_eq!(
        InstallError::ManifestMissing.explanation(),
        "The package has no hew-plugin.json at its top level."
    );
    assert_eq!(
        InstallError::ManifestMissing.explanation_for_folder(),
        "This folder has no hew-plugin.json at its top level."
    );
    // Everything else reads the same for either source.
    let big = InstallError::PackageTooLarge;
    assert_eq!(big.explanation(), big.explanation_for_folder());
}

#[test]
fn removing_a_folder_plugin_says_the_folder_is_left_alone() {
    let m = manifest(json!({}));
    let mut record = installed("com.example.fixture", "2.0.0", &["model.read"]);
    assert!(!uninstall_summary(&record, &m).contains(plugin_host::FOLDER_KEPT_LINE));
    record.source = InstallSource::Dir {
        path: "/home/someone/plugin".into(),
    };
    assert_eq!(
        uninstall_summary(&record, &m),
        "Fixture 2.0.0\n\
         Removes the plugin, its earlier version, and everything it saved.\n\
         Your folder and its files are left as they are.\n"
    );
}

#[test]
fn a_review_lists_what_changed_then_what_it_keeps() {
    let old = manifest(json!({ "permissions": ["model.read"] }));
    let new = manifest(json!({ "permissions": ["model.read", "clipboard"] }));
    let previous = installed("com.example.fixture", "2.0.0", &["model.read"]);
    let folder = InstallSource::Dir {
        path: "/home/someone/plugin".into(),
    };
    let mut c = consent(&new, &folder, Some(&previous), Some(&old), None);
    c.review = true;
    let summary = consent_summary(&c);
    let clip = plugin_host::permission_line(&p("clipboard")).text;
    let read = plugin_host::permission_line(&p("model.read")).text;
    assert_eq!(summary.matches(&clip).count(), 1, "{summary}");
    let new_at = summary.find("New since v2.0.0:").expect("changes first");
    let keeps_at = summary.find("It keeps:").expect("what it keeps");
    assert!(new_at < keeps_at, "{summary}");
    assert!(
        summary[keeps_at..].contains(&format!("• {read}")),
        "{summary}"
    );
    assert!(!summary.contains("It can:"), "{summary}");
}
