//! Golden manifests (docs/agents/PLUGINS.md "Manifest"): every file under
//! `tests/golden/valid/` parses and serializes to its sibling
//! `<name>.expected.json`; every file under `tests/golden/invalid/` fails
//! with the error name in its sibling `<name>.error`.
//!
//! Regenerate the expected serializations (after reviewing the change)
//! with `REGENERATE_MANIFEST_GOLDENS=1 cargo test -p plugin-manifest`.

use std::path::{Path, PathBuf};

fn fixtures(dir: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(dir);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("fixture directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().ends_with(".expected.json")
        })
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no fixtures in {}", root.display());
    out
}

#[test]
fn valid_manifests_parse_and_serialize_to_their_goldens() {
    let regenerate = std::env::var_os("REGENERATE_MANIFEST_GOLDENS").is_some();
    for path in fixtures("valid") {
        let text = std::fs::read_to_string(&path).expect("fixture");
        let manifest = plugin_manifest::parse(&text)
            .unwrap_or_else(|e| panic!("{} should parse: {} ({e:?})", path.display(), e.name()));
        let got = serde_json::to_value(&manifest).expect("serializes");
        let expected_path = path.with_extension("expected.json");
        if regenerate {
            std::fs::write(
                &expected_path,
                serde_json::to_string_pretty(&got).unwrap() + "\n",
            )
            .unwrap();
            continue;
        }
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&expected_path)
                .unwrap_or_else(|_| panic!("missing {}", expected_path.display())),
        )
        .expect("expected golden parses");
        assert_eq!(got, expected, "{} diverged from its golden", path.display());
        // The serialized form is itself a valid manifest that parses back
        // to the same value.
        let again = plugin_manifest::parse(&serde_json::to_string(&got).unwrap())
            .unwrap_or_else(|e| panic!("{} re-parse: {}", path.display(), e.name()));
        assert_eq!(again, manifest, "{} does not round-trip", path.display());
    }
}

#[test]
fn invalid_manifests_fail_with_their_named_error() {
    for path in fixtures("invalid") {
        let text = std::fs::read_to_string(&path).expect("fixture");
        let expected_path = path.with_extension("error");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("missing {}", expected_path.display()));
        match plugin_manifest::parse(&text) {
            Ok(_) => panic!("{} should not parse", path.display()),
            Err(e) => {
                assert_eq!(
                    e.name(),
                    expected.trim(),
                    "{}: wrong error ({e:?})",
                    path.display()
                );
                assert!(!e.explanation().is_empty());
            }
        }
    }
}

/// `too-many-commands.json` really holds `MAX_COMMANDS + 1` otherwise valid
/// commands, so the fixture tests the limit and not some other defect.
#[test]
fn too_many_commands_fixture_is_one_over_the_limit() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/invalid/too-many-commands.json");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let commands = value["contributes"]["commands"].as_array().unwrap();
    assert_eq!(commands.len(), plugin_manifest::MAX_COMMANDS + 1);
    let mut trimmed = value.clone();
    trimmed["contributes"]["commands"]
        .as_array_mut()
        .unwrap()
        .pop();
    plugin_manifest::parse(&trimmed.to_string()).expect("one fewer command parses");
}
