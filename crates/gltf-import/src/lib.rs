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

/// Parse glTF 2.0 / GLB bytes into a kernel [`ImportScene`](kernel::ImportScene)
/// recipe. I/O-free: all referenced data must be embedded (GLB blob or `data:`
/// URIs).
///
/// On success returns `(ImportScene, missing)` where `missing` lists external
/// resource URIs (buffers/images) that could not be resolved in-memory; the
/// affected geometry/textures are dropped, not faked (DEVELOPMENT.md rule 4). A
/// totally unparseable file returns `Err(GltfError)`.
pub fn import(bytes: &[u8]) -> Result<(kernel::ImportScene, Vec<String>), GltfError> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| GltfError::Parse(e.to_string()))?;
    let (buffers, mut missing) = buffers::resolve(&gltf);
    let (scene, mat_missing) = convert::build_scene(&gltf, &buffers)?;
    missing.extend(mat_missing);
    Ok((scene, missing))
}
