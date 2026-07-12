// Outside the determinism-critical kernel scope (kernel / inference /
// tessellate / mesh-heal). Like `dae-import`, this crate's maps are parse-time
// node/mesh lookups; the shared `mesh-heal` pass (in scope) owns output
// determinism. The workspace clippy.toml ban is suppressed here (submodules too).
#![allow(clippy::disallowed_types)]

//! glTF 2.0 / GLB import for Hew.
//!
//! The export-side sibling of `dae-import`: where COLLADA carries SketchUp
//! models, glTF/GLB is the open mesh-interchange format Blender and most DCC
//! tools round-trip, and what Hew's own export emits.
//!
//! I/O-free and WASM-bound (DEVELOPMENT.md rule 1 — depends on `kernel` +
//! `mesh-heal`, never the reverse). Parsing is in-memory (`gltf::Gltf::from_slice`,
//! handles both `.glb` and `.gltf`); embedded buffers/images are resolved from
//! the GLB binary chunk or `data:` URIs (external-file URIs are reported, not
//! fetched — there is no filesystem here). Texture bytes stay **encoded** (the
//! `image` pixel decoder is deliberately excluded to keep the WASM bundle small).
//!
//! # Pipeline
//!
//! ```text
//! glTF/GLB bytes ──Gltf::from_slice──> gltf::Document (+ GLB blob)
//!   ──buffers.rs──> resolved buffer bytes (blob / data-URI)
//!   ──convert.rs──> per-glTF-mesh raw positions / triangles / material refs / UVs
//!       ──mesh_heal::heal_mesh──> weld + dedup + T-junction + orient + coplanar-merge
//!       ──mesh_heal::uv::fit_uv_frame──> per-face affine UV frames
//!   ──material.rs──> Vec<kernel::Material> (base color + embedded texture)
//!   ──> kernel::ImportScene (defs for shared meshes, instances/objects/groups)
//! ```
//!
//! The heal pass is what turns glTF triangle soup back into *editable* n-gon
//! faces and watertight solids — the whole reason this is a kernel-side crate
//! rather than three.js mesh loading.

pub mod buffers;
pub mod convert;
pub mod material;

/// Typed failures from [`import`].
#[derive(Debug, Clone)]
pub enum GltfError {
    /// The bytes could not be parsed as glTF 2.0 / GLB.
    Parse(String),
    /// The file uses a feature Hew does not support for import.
    Unsupported(String),
}

impl std::fmt::Display for GltfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GltfError::Parse(msg) => write!(f, "glTF parse error: {msg}"),
            GltfError::Unsupported(msg) => write!(f, "unsupported glTF feature: {msg}"),
        }
    }
}

impl std::error::Error for GltfError {}

/// A parsed glTF/GLB, ready for `Document::ingest`.
pub struct GltfScene {
    /// The import recipe (`ingest` consumes it).
    pub scene: kernel::ImportScene,
    /// External resource URIs (buffers/images) that could not be resolved
    /// in-memory; the affected geometry/textures are dropped, not faked
    /// (DEVELOPMENT.md rule 4) — pass to `ingest` as `textures_missing`.
    pub missing: Vec<String>,
    /// User-visible conversion warnings (non-manifold splits — rule 4:
    /// decomposition happens loudly, never silently). Surface these in the
    /// import report.
    pub warnings: Vec<String>,
}

/// Parse glTF 2.0 / GLB bytes into a kernel [`ImportScene`](kernel::ImportScene)
/// recipe. I/O-free: all referenced data must be embedded (GLB blob or `data:`
/// URIs).
///
/// On success returns a [`GltfScene`]: the scene, the unresolved external
/// resource URIs, and any conversion warnings (non-manifold splits). A totally
/// unparseable file returns `Err(GltfError)`.
pub fn import(bytes: &[u8]) -> Result<GltfScene, GltfError> {
    // The `gltf-json` validator itself can panic on hostile input (e.g. a
    // primitive whose POSITION accessor index is out of range of an empty
    // accessors array indexes `root.accessors[i]` unchecked in 1.4.1), so
    // a byte-mutated file could crash the app instead of failing typed.
    // Contain the whole parse/convert pipeline: a panic becomes a Parse
    // error at this boundary (DEVELOPMENT.md rule 3 — malformed input
    // fails typed, never crashes). On wasm32 (panic = abort) the wrapper
    // is a passthrough; the trap surfaces as a load failure there.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| import_inner(bytes))).unwrap_or_else(
        |_| {
            Err(GltfError::Parse(
                "malformed glTF: parser rejected the file".to_string(),
            ))
        },
    )
}

fn import_inner(bytes: &[u8]) -> Result<GltfScene, GltfError> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| GltfError::Parse(e.to_string()))?;
    let (buffers, mut missing) = buffers::resolve(&gltf);
    let (scene, mat_missing, warnings) = convert::build_scene(&gltf, &buffers)?;
    missing.extend(mat_missing);
    Ok(GltfScene {
        scene,
        missing,
        warnings,
    })
}
