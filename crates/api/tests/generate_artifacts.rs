//! Registry-completeness's byte-identity half (docs/agents/HEW_API.md §14): "the
//! generated artifacts … regenerate byte-identically in CI, so drift
//! between registry and artifact is a build failure rather than a review
//! hazard." This is that check for the two artifacts §9 promises beyond
//! the MCP tool definitions (`crates/hew-cli/src/mcp.rs::generate_tools`,
//! covered by its own module tests):
//!
//! - `app/src/api/hewApi.gen.ts` — the TypeScript client SDK.
//! - `docs/API_REFERENCE.gen.md` — the published API reference.
//!
//! Mirrors `refusal_copy.gen.rs`'s own REGENERATE pattern
//! (`app/src/kernelErrorsDump.test.ts`): a plain `cargo test -p api` only
//! ASSERTS the committed files match a fresh generation; setting
//! `REGENERATE_API_ARTIFACTS=1` WRITES them instead.
//!
//! Regenerate with:
//!
//!   REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts

use api::codegen::{generate_api_reference, generate_plugin_sdk_types, generate_ts_sdk};
use api::registry::Registry;
use std::path::{Path, PathBuf};

const REGEN_COMMAND: &str =
    "REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts";

/// The repo root, from this test binary's manifest dir
/// (`crates/api/Cargo.toml`) up two levels.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root exists")
}

/// Asserts `path` matches `fresh`, or writes it when
/// `REGENERATE_API_ARTIFACTS=1` is set.
fn check_or_write(path: &Path, fresh: &str) {
    if std::env::var_os("REGENERATE_API_ARTIFACTS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("creating {}: {e}", parent.display()));
        }
        std::fs::write(path, fresh).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        return;
    }
    let committed = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "{} is missing or unreadable ({e}) — generate it with: {REGEN_COMMAND}",
            path.display()
        )
    });
    assert_eq!(
        committed,
        fresh,
        "{} is out of date with the crates/api registry — regenerate with: {REGEN_COMMAND}",
        path.display()
    );
}

#[test]
fn ts_sdk_regenerates_byte_identically() {
    let registry = Registry::protocol_1();
    let fresh = generate_ts_sdk(&registry);
    check_or_write(&repo_root().join("app/src/api/hewApi.gen.ts"), &fresh);
}

#[test]
fn api_reference_regenerates_byte_identically() {
    let registry = Registry::protocol_1();
    let fresh = generate_api_reference(&registry);
    check_or_write(&repo_root().join("docs/API_REFERENCE.gen.md"), &fresh);
}

/// Three plugin commands covering a plain object, the defaulted empty
/// schemas, and a dotted local id.
fn plugin_types_fixture() -> Vec<plugin_manifest::CommandContribution> {
    serde_json::from_value(serde_json::json!([
        {
            "id": "sum",
            "class": "read_only",
            "summary": "Adds.",
            "params": {
                "type": "object",
                "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
                "required": ["a", "b"],
                "additionalProperties": false
            },
            "result": {
                "type": "object",
                "properties": { "sum": { "type": "number" } },
                "required": ["sum"],
                "additionalProperties": false
            }
        },
        { "id": "mut", "class": "mutating", "summary": "Mutates." },
        {
            "id": "cutlist.generate",
            "class": "mutating",
            "summary": "Generates a cut list.",
            "params": {
                "type": "object",
                "properties": {
                    "parts": { "type": "array", "items": { "type": "string" } },
                    "grain": { "enum": ["long", "cross"] }
                },
                "required": ["parts"],
                "additionalProperties": false
            }
        }
    ]))
    .expect("fixture parses")
}

#[test]
fn plugin_sdk_types_golden() {
    let fresh = generate_plugin_sdk_types("com.example.lib", &plugin_types_fixture());
    check_or_write(
        &repo_root().join("crates/api/tests/golden/plugin_types.d.ts"),
        &fresh,
    );
}

/// Committed artifacts never contain plugin commands: a registry with a
/// plugin registered generates exactly what protocol 1 alone does.
#[test]
fn committed_artifacts_ignore_plugin_commands() {
    let mut with_plugin = Registry::protocol_1();
    with_plugin
        .register_plugin("com.example.lib", &plugin_types_fixture())
        .expect("fixture registers");
    let bare = Registry::protocol_1();
    assert_eq!(generate_ts_sdk(&with_plugin), generate_ts_sdk(&bare));
    assert_eq!(
        generate_api_reference(&with_plugin),
        generate_api_reference(&bare)
    );
}
