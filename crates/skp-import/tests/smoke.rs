//!  smoke test: the clean-room `skp` dependency works end-to-end.
//!
//! Reads a corpus file from the sibling `../OpenSKP` repo rather than
//! committing a binary fixture here — this crate only builds when that
//! sibling checkout exists (path dependency), so the corpus is always
//! available wherever the test can run, and the ground truth stays in one
//! place. (Test-only filesystem use; the library surface itself takes bytes.)

use std::path::Path;

fn corpus(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../OpenSKP/corpus")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("corpus file {} unreadable: {e}", path.display()))
}

#[test]
fn box_skp_parses_to_a_single_cube_topology() {
    let runs = skp_import::probe_topology(&corpus("box.skp"));
    assert!(!runs.is_empty(), "box.skp yielded no geometry runs");
    assert!(
        runs.iter()
            .any(|t| t.vertices == 8 && t.edges == 12 && t.faces == 6),
        "no run had cube topology (8 vertices / 12 edges / 6 faces); got: {runs:?}"
    );
}
