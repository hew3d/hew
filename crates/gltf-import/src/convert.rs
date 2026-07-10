//! glTF document → kernel [`ImportScene`].
//!
//! glTF's natural sharing unit is the **mesh** (one mesh referenced by several
//! nodes = instancing). So:
//! - a mesh referenced by ≥2 nodes becomes a [`DefRecipe`]; each referencing
//!   node becomes an [`ImportNode::Instance`] at that node's world pose;
//! - a mesh referenced once is baked into its node's world transform as an
//!   [`ImportNode::Mesh`] (a plain world Object);
//! - a node with children (with or without its own mesh) becomes an
//!   [`ImportNode::Group`] whose transform is baked into its descendants.
//!
//! glTF is right-handed, +Y up, meters; Hew is +Z up, meters. The Y-up→Z-up
//! rotation is applied as the outermost transform at every leaf (bake / pose),
//! mirroring `dae-import`'s `world_tf` handling.

use std::collections::{HashMap, HashSet};

use gltf::mesh::Mode;
use gltf::{Gltf, Mesh, Node};
use kernel::{
    DefRecipe, ImportNode, ImportScene, MeshRecipe, NO_MATERIAL, Point3, Transform, UvFrame,
};
use mesh_heal::heal_mesh_with_tol;
use mesh_heal::uv::fit_uv_frame;

use crate::GltfError;
use crate::material;

/// Raw geometry for one glTF mesh, with all primitives merged into a single
/// position/face list so the heal pass can weld + reassemble across them.
#[derive(Default)]
struct RawMesh {
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_materials: Vec<u32>,
    face_corner_uvs: Vec<Vec<[f64; 2]>>,
    /// Triangles skipped because their index-buffer values exceeded the
    /// primitive's vertex count (corrupt input); reported loudly per mesh.
    skipped_bad_index_tris: usize,
}

/// Shared context for the node walk.
struct Ctx<'a> {
    buffers: &'a [Option<Vec<u8>>],
    /// glTF material index → dense (deduped) kernel material index.
    mat_remap: &'a [u32],
    mesh_to_def: &'a HashMap<usize, usize>,
    world_tf: Transform,
}

/// Build the full `ImportScene`, plus any missing image URIs from materials
/// and user-visible conversion warnings (non-manifold splits).
pub fn build_scene(
    gltf: &Gltf,
    buffers: &[Option<Vec<u8>>],
) -> Result<(ImportScene, Vec<String>, Vec<String>), GltfError> {
    let mat_table = material::build(gltf, buffers);
    let world_tf = mesh_heal::y_up_to_z_up();
    let mut warnings = SplitWarnings::default();

    // Pass 1: count node references to each mesh (instancing signal).
    let mut refcount: HashMap<usize, usize> = HashMap::new();
    for node in scene_roots(gltf) {
        count_meshes(&node, &mut refcount);
    }

    // Pass 2: shared meshes (refcount ≥ 2) become component definitions.
    let mut defs: Vec<DefRecipe> = Vec::new();
    let mut mesh_to_def: HashMap<usize, usize> = HashMap::new();
    for mesh in gltf.document.meshes() {
        if refcount.get(&mesh.index()).copied().unwrap_or(0) < 2 {
            continue;
        }
        let raw = extract_raw_mesh(&mesh, buffers, &mat_table.remap);
        warnings.note_skipped_indices(mesh.index(), &mesh_name(&mesh), raw.skipped_bad_index_tris);
        // Definition geometry stays mesh-local (IDENTITY bake); the per-instance
        // pose carries the node world transform + Y-up→Z-up. A non-manifold
        // mesh splits into several open-shell recipes under this ONE
        // definition, so every placement keeps all its pieces.
        let recipes = build_recipes(
            &raw,
            &Transform::IDENTITY,
            mesh_name(&mesh),
            mesh.index(),
            &mut warnings,
        );
        if !recipes.is_empty() {
            mesh_to_def.insert(mesh.index(), defs.len());
            defs.push(DefRecipe {
                name: Some(mesh_name(&mesh)),
                meshes: recipes,
            });
        }
    }

    // Pass 3: walk the node forest into world-space roots.
    let ctx = Ctx {
        buffers,
        mat_remap: &mat_table.remap,
        mesh_to_def: &mesh_to_def,
        world_tf,
    };
    let mut roots: Vec<ImportNode> = Vec::new();
    for node in scene_roots(gltf) {
        if let Some(n) = ctx.convert_node(&node, &Transform::IDENTITY, &mut warnings) {
            roots.push(n);
        }
    }

    let SplitWarnings {
        messages,
        index_messages,
        ..
    } = warnings;
    let mut warnings = messages;
    cap_split_warnings(&mut warnings);
    // Bad-index reports ride after the (capped) split notices: one line per
    // affected mesh, never absorbed into the non-manifold aggregate.
    warnings.extend(index_messages);
    Ok((
        ImportScene {
            materials: mat_table.materials,
            defs,
            roots,
            guides: Vec::new(),
            // glTF has no layer list; no document-level tags to declare.
            tags: Vec::new(),
        },
        mat_table.missing,
        warnings,
    ))
}

impl Ctx<'_> {
    fn convert_node(
        &self,
        node: &Node,
        acc: &Transform,
        warnings: &mut SplitWarnings,
    ) -> Option<ImportNode> {
        let node_world = node_transform(node).then(acc);

        // This node's own geometry (instance of a shared def, or a baked object).
        let self_node: Option<ImportNode> = node.mesh().and_then(|mesh| {
            if let Some(&def) = self.mesh_to_def.get(&mesh.index()) {
                Some(ImportNode::Instance {
                    def,
                    pose: node_world.then(&self.world_tf),
                    name: node_name(node),
                    tags: Vec::new(),
                    hidden: false,
                })
            } else {
                let raw = extract_raw_mesh(&mesh, self.buffers, self.mat_remap);
                warnings.note_skipped_indices(
                    mesh.index(),
                    &mesh_name(&mesh),
                    raw.skipped_bad_index_tris,
                );
                let name = node_name(node).unwrap_or_else(|| mesh_name(&mesh));
                let mut recipes = build_recipes(
                    &raw,
                    &node_world.then(&self.world_tf),
                    name,
                    mesh.index(),
                    warnings,
                );
                match recipes.len() {
                    0 => None,
                    1 => Some(ImportNode::Mesh(recipes.pop().expect("len checked"))),
                    // A split mesh stays one scene entity: group the
                    // open-shell pieces under the node's name.
                    _ => Some(ImportNode::Group {
                        name: group_name(node),
                        children: recipes.into_iter().map(ImportNode::Mesh).collect(),
                        tags: Vec::new(),
                        hidden: false,
                    }),
                }
            }
        });

        let children: Vec<ImportNode> = node
            .children()
            .filter_map(|c| self.convert_node(&c, &node_world, warnings))
            .collect();

        match (self_node, children.is_empty()) {
            (Some(sn), true) => Some(sn),
            (Some(sn), false) => {
                let mut group_children = Vec::with_capacity(children.len() + 1);
                group_children.push(sn);
                group_children.extend(children);
                Some(ImportNode::Group {
                    name: group_name(node),
                    children: group_children,
                    tags: Vec::new(),
                    hidden: false,
                })
            }
            (None, false) => Some(ImportNode::Group {
                name: group_name(node),
                children,
                tags: Vec::new(),
                hidden: false,
            }),
            (None, true) => None,
        }
    }
}

/// Cap on individual non-manifold split warnings: a badly damaged model can
/// hold hundreds of such meshes, and a wall of near-identical strings helps
/// nobody. Past the cap the remainder aggregates into one line (mirrors
/// skp-import's desync aggregation).
const SPLIT_DETAIL_CAP: usize = 8;

/// Non-manifold split warnings, deduplicated by glTF mesh index. The same
/// mesh CAN heal (and split) more than once: a shared mesh whose definition
/// build yields no recipes under the IDENTITY bake is not entered in
/// `mesh_to_def`, so each referencing node re-heals it under its own bake —
/// and because the weld tolerance and degeneracy culling are scale-dependent,
/// a per-node bake can produce faces (and a split) where the IDENTITY bake
/// produced none. Keying on the mesh index rather than the message text
/// matters because glTF mesh names need not be unique: a DISTINCT mesh whose
/// name and piece count happen to collide still gets its own mandated loud
/// report (rule 4).
#[derive(Default)]
struct SplitWarnings {
    /// glTF mesh indices that have already produced a split warning.
    warned_meshes: HashSet<usize>,
    messages: Vec<String>,
    /// glTF mesh indices that have already produced a bad-index warning
    /// (a non-def mesh is re-extracted once per referencing node).
    warned_bad_indices: HashSet<usize>,
    /// Bad-index warnings, kept apart from `messages` so the non-manifold
    /// aggregation cap never truncates or mislabels them.
    index_messages: Vec<String>,
}

impl SplitWarnings {
    /// Report triangles skipped for out-of-range index-buffer values (rule 4:
    /// dropped foreign garbage is reported loudly), once per glTF mesh.
    fn note_skipped_indices(&mut self, mesh_index: usize, name: &str, skipped: usize) {
        if skipped == 0 || !self.warned_bad_indices.insert(mesh_index) {
            return;
        }
        self.index_messages.push(format!(
            "'{}' index buffer references vertices past the end of its \
             vertex array; {} triangle{} skipped",
            if name.is_empty() {
                "unnamed mesh"
            } else {
                name
            },
            skipped,
            if skipped == 1 { "" } else { "s" },
        ));
    }
}

fn cap_split_warnings(warnings: &mut Vec<String>) {
    if warnings.len() > SPLIT_DETAIL_CAP {
        let extra = warnings.len() - SPLIT_DETAIL_CAP;
        warnings.truncate(SPLIT_DETAIL_CAP);
        warnings.push(format!(
            "{extra} more non-manifold mesh{} imported as open shells \
             (split at non-manifold edges, geometry unchanged)",
            if extra == 1 { "" } else { "es" },
        ));
    }
}

/// Build the `MeshRecipe`s for one raw mesh: heal (welds, dedups, T-junction
/// repair, outward orientation, coplanar-triangle merge) under `bake`, then
/// fit per-face UV frames from healed corner UVs. Usually one recipe; a
/// NON-MANIFOLD mesh (which `from_polygons` would reject whole) is decomposed
/// by [`mesh_heal::split::split_non_manifold`] into several open-shell
/// recipes — loudly (a warning names the mesh and the piece count), never
/// silently (rule 4) — the same contract as skp-import. Empty if nothing
/// survives heal.
fn build_recipes(
    raw: &RawMesh,
    bake: &Transform,
    name: String,
    mesh_index: usize,
    warnings: &mut SplitWarnings,
) -> Vec<MeshRecipe> {
    let no_holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); raw.faces.len()];
    // Bake the node/world transform BEFORE choosing the weld tolerance: the
    // heal pass welds post-bake coordinates, so a tolerance computed from the
    // raw (mesh-local) positions is wrong by the node's scale factor — a
    // scale=100 node would weld world-scale f32 gaps with a local-scale
    // tolerance and leave every seam split. Baking here once (heal then gets
    // the identity) keeps the tolerance and the welded coordinates in the
    // same space.
    let baked: Vec<Point3> = raw.positions.iter().map(|&p| bake.apply_point(p)).collect();
    let (positions, faces, face_materials, healed_uvs, face_holes) = heal_mesh_with_tol(
        &baked,
        &raw.faces,
        &raw.face_materials,
        &raw.face_corner_uvs,
        &no_holes,
        &Transform::IDENTITY,
        gltf_weld_tol(&baked),
        // Merge coplanar triangles at the kernel's import planarity (1 mm), not
        // the 1 nm native tolerance — f32 flat surfaces sit microns off-plane,
        // so the strict gate would leave every wall/floor as triangle soup
        // (huge face count + memory; D-fix for the OOM crash).
        kernel::tol::IMPORT_PLANE_DIST,
    );
    if faces.is_empty() {
        return Vec::new();
    }

    if let Some(pieces) = mesh_heal::split::split_non_manifold(
        &positions,
        &faces,
        &face_materials,
        &healed_uvs,
        &face_holes,
    ) {
        // One loud report per glTF mesh, keyed by mesh index — never
        // suppressed by another mesh's identical message text (see
        // [`SplitWarnings`]).
        if warnings.warned_meshes.insert(mesh_index) {
            warnings.messages.push(format!(
                "'{}' is non-manifold; imported as {} open shell{} \
                 (split at non-manifold edges, geometry unchanged)",
                if name.is_empty() {
                    "unnamed mesh"
                } else {
                    &name
                },
                pieces.len(),
                if pieces.len() == 1 { "" } else { "s" },
            ));
        }
        return pieces
            .into_iter()
            .map(|piece| {
                recipe_from_arrays(
                    name.clone(),
                    piece.positions,
                    piece.faces,
                    piece.face_materials,
                    piece.face_corner_uvs,
                    piece.face_holes,
                )
            })
            .collect();
    }

    vec![recipe_from_arrays(
        name,
        positions,
        faces,
        face_materials,
        healed_uvs,
        face_holes,
    )]
}

/// Healed (or split-piece) parallel arrays -> a `MeshRecipe`, fitting the
/// per-face affine UV frames from the corner UVs.
fn recipe_from_arrays(
    name: String,
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_materials: Vec<u32>,
    corner_uvs: Vec<Vec<[f64; 2]>>,
    face_holes: Vec<Vec<Vec<usize>>>,
) -> MeshRecipe {
    let face_uv_frames: Vec<Option<UvFrame>> = faces
        .iter()
        .zip(corner_uvs.iter())
        .map(|(face, uvs)| {
            if uvs.len() == face.len() && uvs.len() >= 3 {
                let corner_pos: Vec<Point3> = face.iter().map(|&vi| positions[vi]).collect();
                fit_uv_frame(&corner_pos, uvs)
            } else {
                None
            }
        })
        .collect();

    MeshRecipe {
        name,
        positions,
        faces,
        face_materials,
        face_uv_frames,
        face_holes,
        base_material: NO_MATERIAL,
        tags: Vec::new(),
    }
}

/// Choose a weld tolerance for f32-sourced glTF positions.
///
/// glTF `POSITION` is float32, so vertices that were coincident in the authoring
/// tool land up to ~`magnitude · 2.4e-7` apart after the round-trip. The kernel's
/// native 1 nm `POINT_MERGE` is far too tight and would leave every shared edge
/// split (a "leaky" shell), so we scale the tolerance to the mesh's coordinate
/// magnitude (≈4× the worst-case f32 gap), floored for tiny meshes.
///
/// `positions` must be the POST-BAKE (world-space) coordinates the weld will
/// actually run on: welding happens after the node transform is applied, so a
/// tolerance derived from pre-transform locals would be off by the node's
/// scale factor.
fn gltf_weld_tol(positions: &[Point3]) -> f64 {
    let max_abs = positions
        .iter()
        .flat_map(|p| [p.x.abs(), p.y.abs(), p.z.abs()])
        .fold(0.0_f64, f64::max);
    (max_abs * 1e-6).max(1e-7)
}

/// Merge every triangle primitive of a glTF mesh into one raw position/face list.
fn extract_raw_mesh(mesh: &Mesh, buffers: &[Option<Vec<u8>>], mat_remap: &[u32]) -> RawMesh {
    let mut rm = RawMesh::default();
    for prim in mesh.primitives() {
        if prim.mode() != Mode::Triangles {
            continue; // v1 imports triangle meshes only ( — no curves).
        }
        let reader = prim.reader(|b| buffers.get(b.index()).and_then(|o| o.as_deref()));
        let Some(positions) = reader.read_positions() else {
            continue;
        };
        let base = rm.positions.len();
        let prim_positions: Vec<Point3> = positions
            .map(|[x, y, z]| Point3::new(x as f64, y as f64, z as f64))
            .collect();
        let count = prim_positions.len();
        rm.positions.extend(prim_positions);

        let uvs: Option<Vec<[f64; 2]>> = reader.read_tex_coords(0).map(|tc| {
            tc.into_f32()
                .map(|[u, v]| [u as f64, v as f64])
                .collect::<Vec<_>>()
        });

        let mat = prim
            .material()
            .index()
            .and_then(|i| mat_remap.get(i).copied())
            .unwrap_or(NO_MATERIAL);

        let indices: Vec<u32> = match reader.read_indices() {
            Some(ri) => ri.into_u32().collect(),
            None => (0..count as u32).collect(),
        };

        for tri in indices.chunks_exact(3) {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            // An index buffer can carry values past the primitive's POSITION
            // count (corrupt or crafted input — the gltf crate validates JSON
            // references, never buffer values). Such a triangle is foreign
            // garbage: skip it, mirroring dae-import's per-index bound guards
            // and mesh-heal's own entry validation.
            if a >= count || b >= count || c >= count {
                rm.skipped_bad_index_tris += 1;
                continue;
            }
            rm.faces.push(vec![base + a, base + b, base + c]);
            rm.face_materials.push(mat);
            rm.face_corner_uvs.push(match &uvs {
                Some(u) if a < u.len() && b < u.len() && c < u.len() => vec![u[a], u[b], u[c]],
                _ => Vec::new(),
            });
        }
    }
    rm
}

/// Root nodes of the default scene (or the first scene if none is marked).
fn scene_roots(gltf: &Gltf) -> Vec<Node<'_>> {
    let scene = gltf
        .document
        .default_scene()
        .or_else(|| gltf.document.scenes().next());
    match scene {
        Some(s) => s.nodes().collect(),
        None => Vec::new(),
    }
}

fn count_meshes(node: &Node, map: &mut HashMap<usize, usize>) {
    if let Some(mesh) = node.mesh() {
        *map.entry(mesh.index()).or_default() += 1;
    }
    for child in node.children() {
        count_meshes(&child, map);
    }
}

/// Build a kernel `Transform` from a glTF node's column-major 4×4 local matrix.
fn node_transform(node: &Node) -> Transform {
    let m = node.transform().matrix(); // [[f32;4];4], m[col][row]
    Transform::from_affine(&[
        m[0][0] as f64,
        m[1][0] as f64,
        m[2][0] as f64,
        m[3][0] as f64,
        m[0][1] as f64,
        m[1][1] as f64,
        m[2][1] as f64,
        m[3][1] as f64,
        m[0][2] as f64,
        m[1][2] as f64,
        m[2][2] as f64,
        m[3][2] as f64,
    ])
}

fn node_name(node: &Node) -> Option<String> {
    node.name().map(str::to_string)
}

fn group_name(node: &Node) -> String {
    node_name(node).unwrap_or_else(|| format!("node_{}", node.index()))
}

fn mesh_name(mesh: &Mesh) -> String {
    mesh.name()
        .map(str::to_string)
        .unwrap_or_else(|| format!("mesh_{}", mesh.index()))
}
