//! Registry-completeness's byte-identity half (docs/HEW_API.md §14): "the
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

use api::codegen::{generate_api_reference, generate_ts_sdk};
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
