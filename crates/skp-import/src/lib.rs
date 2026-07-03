//! SketchUp `.skp` import: clean-room `skp` reader -> Hew Objects/Instances.
//!
//! Provenance : the `skp`
//! dependency is the sibling `../OpenSKP` repo's zero-dependency reader for
//! SketchUp 2017 (v17.3.116) files, derived solely from self-authored corpora,
//! their COLLADA ground-truth exports, and public knowledge of MFC `CArchive`
//! serialization. **No Trimble/SketchUp SDK material exists anywhere in this
//! crate's dependency chain** — that is the admissibility test docs/DEVELOPMENT.md
//! rule 7 now states, and it must hold for every future dependency added here.
//!
//!  scope (this skeleton): prove the dependency end-to-end — the crate
//! builds against `skp`, parses a corpus file, and reports topology. The real
//! import pipeline (`skp::Model` -> `mesh-heal` -> kernel Objects, materials,
//! components, layers->tags, guides, native names) is and will mirror
//! `dae-import`'s shape: depends on `kernel`, never the reverse; takes bytes,
//! no filesystem or network.

pub use skp;

/// Topology counts of each geometry run in a `.skp` byte stream.
///
///  probe: the thinnest end-to-end use of the clean-room reader.
/// replaces this as the public surface with the full `import_skp(bytes)`
/// pipeline; it stays useful afterwards as a cheap pre-import inspection.
pub fn probe_topology(bytes: &[u8]) -> Vec<skp::Topology> {
    skp::geometry_runs(bytes)
        .into_iter()
        .map(|run| run.topology)
        .collect()
}
