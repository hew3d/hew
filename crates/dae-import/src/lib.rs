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

pub mod heal;
pub mod material;
pub mod meta;
pub mod parse;
pub mod uv;

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

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse COLLADA 1.4 bytes and pre-resolved images into a kernel `ImportScene`
/// recipe. I/O-free: images are resolved by the host and passed in.
///
/// On success returns the `(ImportScene, textures_missing)` pair where
/// `textures_missing` lists image URIs that could not be found in `images`.
/// Per-mesh geometry issues are NOT errors here — the mesh is emitted as-is;
/// if its geometry is degenerate, `Document::ingest` will skip and report it.
///
/// A totally unparseable file returns `Err(DaeError)`.
pub fn import(
    dae_bytes: &[u8],
    images: &ImageMap,
) -> Result<(kernel::ImportScene, Vec<String>), DaeError> {
    parse::parse_dae(dae_bytes, images)
}
