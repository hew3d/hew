// Outside the determinism-critical kernel scope (kernel / inference /
// tessellate / mesh-heal). This importer's maps are keyed by COLLADA id strings
// for parse-time lookup, not iterated to build kernel output (the shared heal
// pass in `mesh-heal` — which IS in scope — already enforces determinism). The
// workspace clippy.toml ban is suppressed here (covers submodules too).
#![allow(clippy::disallowed_types)]

//! COLLADA (`.dae`) import for Hew.
//!
//! I/O-free: images are resolved by the host and passed in — this crate never
//! touches the filesystem (DEVELOPMENT.md rule 1). The only public entry point is
//! [`import`].
//!
//! # Pipeline
//!
//! ```text
//! DAE bytes ──parse──> dae_parser::Document
//!                ──parse.rs──> raw positions / faces / material refs
//!                ──heal.rs──> weld + two-sided dedup + unit/up-axis
//!                ──material.rs──> Vec<kernel::Material> + dense indices
//!                ──> (ImportScene, textures_missing)
//! ```

use std::collections::HashMap;

use kernel::ImageFormat;

/// Geometry healing now lives in the shared `mesh-heal` crate; this
/// re-export keeps `dae_import::heal::…` paths (incl. tests) working unchanged.
pub use mesh_heal as heal;
/// UV-frame fitting also moved to `mesh-heal`; re-exported so
/// `dae_import::uv::…` / `crate::uv::…` paths keep working unchanged.
pub use mesh_heal::uv;
pub mod material;
pub mod meta;
pub mod parse;

// ── Public types ─────────────────────────────────────────────────────────────

/// Resolved image map: URI → (raw encoded bytes, format).
///
/// Built by the host (Tauri: read from disk relative to the `.dae`; Web: from
/// user-picked files). URIs are the `<init_from>` values from the COLLADA
/// `<library_images>` section, possibly relative (e.g. `"textures/wood.png"`).
pub type ImageMap = HashMap<String, (Vec<u8>, ImageFormat)>;

/// Typed failures from [`import`].
#[derive(Debug, Clone)]
pub enum DaeError {
    /// The file could not be parsed as valid COLLADA XML.
    Parse(String),
    /// The file uses a feature Hew does not support for import.
    Unsupported(String),
}

impl std::fmt::Display for DaeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaeError::Parse(msg) => write!(f, "COLLADA parse error: {msg}"),
            DaeError::Unsupported(msg) => write!(f, "unsupported COLLADA feature: {msg}"),
        }
    }
}

impl std::error::Error for DaeError {}

/// A parsed `.dae`, ready for `Document::ingest`.
pub struct DaeScene {
    /// The import recipe (`ingest` consumes it).
    pub scene: kernel::ImportScene,
    /// Image URIs that could not be found in the host-resolved `images` map
    /// (their materials fall back to plain color) — pass to `ingest` as
    /// `textures_missing`.
    pub textures_missing: Vec<String>,
    /// User-visible conversion warnings (non-manifold splits — DEVELOPMENT.md
    /// rule 4: decomposition happens loudly, never silently). Surface these
    /// in the import report.
    pub warnings: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Upper bound on the size of a `.dae` we will hand to the XML parser. COLLADA
/// is verbose text, so even a heavily detailed SketchUp export stays well under
/// this; the cap exists so a hostile file can't drive the underlying XML reader
/// into a resource-exhaustion DoS (e.g. RUSTSEC-2026-0194 / -0195 in the
/// transitive `quick-xml`, or any future parser pathology) — worst-case cost is
/// bounded by a known maximum input rather than by the attacker. Defence in
/// depth, not a substitute for keeping the XML backend patched.
pub const MAX_DAE_BYTES: usize = 512 * 1024 * 1024;

/// Parse COLLADA 1.4 bytes and pre-resolved images into a kernel `ImportScene`
/// recipe. I/O-free: images are resolved by the host and passed in.
///
/// On success returns a [`DaeScene`]: the scene, the image URIs that could not
/// be found in `images`, and any conversion warnings (non-manifold splits).
/// Per-mesh geometry issues are NOT errors here — the mesh is emitted as-is;
/// if its geometry is degenerate, `Document::ingest` will skip and report it.
///
/// A totally unparseable file returns `Err(DaeError)`.
pub fn import(dae_bytes: &[u8], images: &ImageMap) -> Result<DaeScene, DaeError> {
    if dae_bytes.len() > MAX_DAE_BYTES {
        return Err(DaeError::Unsupported(format!(
            "COLLADA file is {} bytes, over the {} byte import limit",
            dae_bytes.len(),
            MAX_DAE_BYTES
        )));
    }
    parse::parse_dae(dae_bytes, images)
}
