//! COLLADA document → `ImportScene` recipe (contract).
//!
//! Walks the parsed `dae_parser::Document`:
//! - `<asset>` → unit scale + up-axis correction transform
//! - `<library_geometries>` → raw positions/faces/material bindings
//! - `<library_materials>` / `<library_effects>` → `MaterialTable`
//! - `<library_nodes>` → shared `DefRecipe`s (component definitions)
//! - `<visual_scene>` → `ImportNode` tree
//!
//! No I/O; all external data passes in through `dae_bytes` and `images`.

use std::collections::HashMap;

use dae_parser::{Document as DaeDoc, Geometry, Node, ParseLibrary, Source, VisualScene};
use kernel::{ImportNode, ImportScene, MeshRecipe, Point3, Transform, UvFrame, Vec3};

use crate::heal::{heal_mesh, world_transform};
use crate::material::build_material_table;
use crate::uv::fit_uv_frame;
use crate::{DaeError, ImageMap};

// ── Url helper ────────────────────────────────────────────────────────────────

/// Extract the plain string from a `dae_parser::Url`.
/// Fragment URLs (`#foo`) return `"foo"` (without the `#`).
fn url_as_str(url: &dae_parser::Url) -> &str {
    match url {
        dae_parser::Url::Fragment(s) => s.as_str(),
        dae_parser::Url::Other(s) => s.as_str(),
    }
}

// ── Public entry ──────────────────────────────────────────────────────────────

pub fn parse_dae(
    dae_bytes: &[u8],
    images: &ImageMap,
) -> Result<(ImportScene, Vec<String>), DaeError> {
    let doc = DaeDoc::try_from(dae_bytes).map_err(|e| DaeError::Parse(format!("{e:?}")))?;

    // ── Asset: unit scale + up-axis ────────────────────────────────────────
    let unit_meter = doc.asset.unit.meter; // f32
    let up_axis = doc.asset.up_axis.to_str(); // "Y_UP" | "Z_UP" | "X_UP"
    let world_tf = world_transform(unit_meter, up_axis);

    // ── Materials ──────────────────────────────────────────────────────────
    let mat_table = build_material_table(&doc, images);
    let textures_missing = mat_table.textures_missing;
    let id_to_dense = &mat_table.id_to_dense;

    // ── Geometry map: geometry-id → parsed positions+faces ────────────────
    // (raw, before heal; heal is applied per-node placement)
    let geom_map = build_geometry_map(&doc);

    // ── Library nodes map (for instance_node references) ──────────────────
    // node-id → Node (cloned for shared access)
    let lib_nodes = build_lib_node_map(&doc);

    // ── Build DefRecipes from library_nodes ────────────────────────────────
    // A library node becomes a ComponentDef; any instance_node in the visual
    // scene that references it becomes an ImportNode::Instance.
    let mut defs: Vec<kernel::DefRecipe> = Vec::new();
    let mut lib_node_to_def_idx: HashMap<String, usize> = HashMap::new();

    for (node_id, node) in &lib_nodes {
        let def_idx = defs.len();
        let mut meshes: Vec<MeshRecipe> = Vec::new();
        // Def geometry is kept raw (COLLADA units). world_tf is NOT applied here;
        // it is carried in Instance.pose = acc.then(&world_tf) at the placement site.
        collect_meshes_from_node(node, &geom_map, id_to_dense, &mut meshes);
        if !meshes.is_empty() {
            // The library node's `name` attribute carries the friendly component
            // name (e.g. "Counter_Base"); fall back to its id.
            let name = node.name.clone().or_else(|| node.id.clone());
            defs.push(kernel::DefRecipe { name, meshes });
            lib_node_to_def_idx.insert(node_id.clone(), def_idx);
        }
    }

    // ── Walk the visual scene ──────────────────────────────────────────────
    let roots = if let Some(vs) = doc.get_visual_scene() {
        walk_visual_scene(
            vs,
            &geom_map,
            id_to_dense,
            &world_tf,
            &lib_node_to_def_idx,
            &lib_nodes,
        )
    } else {
        Vec::new()
    };

    let scene = ImportScene {
        materials: mat_table.materials,
        defs,
        roots,
    };

    Ok((scene, textures_missing))
}

// ── Geometry map ─────────────────────────────────────────────────────────────

/// Raw geometry: positions + polygon lists + per-face data.
struct RawMesh {
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_material_symbols: Vec<String>, // parallel to faces
    /// Per-face per-corner UV coordinates. If present for a face (non-empty
    /// inner vec), then `face_corner_uvs[i].len() == faces[i].len()`. If the
    /// primitive had no TEXCOORD input the inner vec is empty.
    face_corner_uvs: Vec<Vec<[f64; 2]>>, // parallel to faces
    /// Per-face inner-loop index lists (for COLLADA `<ph>` / `<h>` holes).
    /// `face_holes[i]` is face `i`'s list of hole loops; empty = no holes.
    /// Holes carry vertex indices only (no per-corner UVs).
    face_holes: Vec<Vec<Vec<usize>>>, // parallel to faces
}

fn build_geometry_map(doc: &DaeDoc) -> HashMap<String, Vec<RawMesh>> {
    let mut map: HashMap<String, Vec<RawMesh>> = HashMap::new();

    for lib_elem in &doc.library {
        let Some(lib) = Geometry::extract_element(lib_elem) else {
            continue;
        };
        for geom in &lib.items {
            let Some(geom_id) = &geom.id else { continue };
            let Some(mesh) = geom.element.as_mesh() else {
                continue;
            };

            let raw_meshes = extract_raw_meshes(mesh);
            map.insert(geom_id.clone(), raw_meshes);
        }
    }

    map
}

fn extract_raw_meshes(mesh: &dae_parser::Mesh) -> Vec<RawMesh> {
    // Find the POSITION source via the vertices element.
    let positions = extract_positions(mesh);

    // SketchUp emits one <triangles> / <polylist> group per material for a
    // single solid shell. All groups share the same <vertices> source (same
    // positions array) but carry different `material` symbols.  Merging them
    // into ONE RawMesh preserves the shell topology and lets `from_polygons`
    // see a watertight solid instead of per-material fragments.
    //
    // Strategy: collect all polygon primitive groups into one merged RawMesh,
    // carrying a parallel per-face material-symbol list. <lines> groups are
    // ignored (unsupported for solid import).
    let mut merged_faces: Vec<Vec<usize>> = Vec::new();
    let mut merged_face_mat_syms: Vec<String> = Vec::new();
    let mut merged_face_corner_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut merged_face_holes: Vec<Vec<Vec<usize>>> = Vec::new();

    for prim in &mesh.elements {
        let raw = extract_primitive(prim, &positions, mesh);
        if let Some(r) = raw {
            merged_faces.extend(r.faces);
            merged_face_mat_syms.extend(r.face_material_symbols);
            merged_face_corner_uvs.extend(r.face_corner_uvs);
            merged_face_holes.extend(r.face_holes);
        }
    }

    if merged_faces.is_empty() {
        return Vec::new();
    }

    // Single merged RawMesh for the whole <mesh> element.
    vec![RawMesh {
        positions,
        faces: merged_faces,
        face_material_symbols: merged_face_mat_syms,
        face_corner_uvs: merged_face_corner_uvs,
        face_holes: merged_face_holes,
    }]
}

fn extract_positions(mesh: &dae_parser::Mesh) -> Vec<Point3> {
    // The <vertices> element points to the POSITION <source>.
    let Some(vertices) = &mesh.vertices else {
        return Vec::new();
    };
    let pos_input = vertices.position_input();

    // Resolve the source URL to an actual Source in this mesh.
    let src_url = &pos_input.source;
    // url_as_str strips '#' from Fragment URLs already.
    let src_id = url_as_str(src_url);
    let Some(source) = mesh
        .sources
        .iter()
        .find(|s| s.id.as_deref() == Some(src_id))
    else {
        return Vec::new();
    };

    extract_xyz_from_source(source)
}

fn extract_xyz_from_source(source: &Source) -> Vec<Point3> {
    let Some(arr) = &source.array else {
        return Vec::new();
    };
    let dae_parser::ArrayElement::Float(fa) = arr else {
        return Vec::new();
    };
    let stride = source.accessor.stride.max(3);
    let offset = source.accessor.offset;
    let count = source.accessor.count;
    let floats = &fa.val;

    let mut positions = Vec::with_capacity(count);
    for i in 0..count {
        let base = offset + i * stride;
        if base + 2 < floats.len() {
            positions.push(Point3::new(
                floats[base] as f64,
                floats[base + 1] as f64,
                floats[base + 2] as f64,
            ));
        }
    }
    positions
}

/// Extract faces from a single primitive element.
///
/// `mesh` is the parent `<mesh>` element, used to resolve TEXCOORD sources.
fn extract_primitive(
    prim: &dae_parser::Primitive,
    positions: &[Point3],
    mesh: &dae_parser::Mesh,
) -> Option<RawMesh> {
    match prim {
        dae_parser::Primitive::Triangles(tris) => extract_triangles(tris, positions, mesh),
        dae_parser::Primitive::PolyList(pl) => extract_polylist(pl, positions, mesh),
        dae_parser::Primitive::Polygons(polys) => extract_polygons(polys, positions, mesh),
        _ => None, // Lines, strips, fans etc. — unsupported for solid import
    }
}

fn find_vertex_offset(inputs: &dae_parser::InputList) -> Option<usize> {
    inputs
        .inputs
        .iter()
        .find(|i| i.semantic == dae_parser::Semantic::Vertex)
        .map(|i| i.offset as usize)
}

/// Find the TEXCOORD input offset and resolve its 2-float source array from
/// the parent mesh's source list. Returns `None` if no TEXCOORD input is
/// present, or if the source is malformed.
fn find_texcoord_source(
    inputs: &dae_parser::InputList,
    mesh: &dae_parser::Mesh,
) -> Option<(usize, Vec<[f64; 2]>)> {
    let tc_input = inputs
        .inputs
        .iter()
        .find(|i| i.semantic == dae_parser::Semantic::TexCoord)?;
    let tc_offset = tc_input.offset as usize;
    let src_id = url_as_str(&tc_input.source);
    let source = mesh
        .sources
        .iter()
        .find(|s| s.id.as_deref() == Some(src_id))?;
    let uvs = extract_uv2_from_source(source);
    if uvs.is_empty() {
        None
    } else {
        Some((tc_offset, uvs))
    }
}

/// Extract a flat list of `[u, v]` pairs from a COLLADA float source with
/// stride ≥ 2 (ignores any extra channels beyond UV).
fn extract_uv2_from_source(source: &dae_parser::Source) -> Vec<[f64; 2]> {
    let Some(arr) = &source.array else {
        return Vec::new();
    };
    let dae_parser::ArrayElement::Float(fa) = arr else {
        return Vec::new();
    };
    let stride = source.accessor.stride.max(2);
    let offset = source.accessor.offset;
    let count = source.accessor.count;
    let floats = &fa.val;
    let mut uvs = Vec::with_capacity(count);
    for i in 0..count {
        let base = offset + i * stride;
        if base + 1 < floats.len() {
            uvs.push([floats[base] as f64, floats[base + 1] as f64]);
        }
    }
    uvs
}

fn extract_triangles(
    tris: &dae_parser::Triangles,
    positions: &[Point3],
    mesh: &dae_parser::Mesh,
) -> Option<RawMesh> {
    let prim_data = tris.data.prim.as_deref()?;
    let vertex_offset = find_vertex_offset(&tris.inputs)?;
    let stride = tris.inputs.stride;
    let mat_symbol = tris.material.clone().unwrap_or_default();

    // Optional TEXCOORD source.
    let tc_info = find_texcoord_source(&tris.inputs, mesh);

    let mut faces = Vec::new();
    let mut face_mat_syms = Vec::new();
    let mut face_corner_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut face_holes: Vec<Vec<Vec<usize>>> = Vec::new();
    let num_tris = tris.count;

    for t in 0..num_tris {
        let base = t * 3 * stride;
        let mut face = Vec::with_capacity(3);
        let mut corner_uvs: Vec<[f64; 2]> = Vec::new();
        let mut ok = true;
        for v in 0..3 {
            let idx = base + v * stride + vertex_offset;
            if idx < prim_data.len() {
                let vi = prim_data[idx] as usize;
                if vi < positions.len() {
                    face.push(vi);
                    // Extract texcoord for this corner.
                    if let Some((tc_offset, ref tc_data)) = tc_info {
                        let tc_idx = base + v * stride + tc_offset;
                        if tc_idx < prim_data.len() {
                            let tci = prim_data[tc_idx] as usize;
                            if tci < tc_data.len() {
                                corner_uvs.push(tc_data[tci]);
                            } else {
                                corner_uvs.push([0.0, 0.0]);
                            }
                        } else {
                            corner_uvs.push([0.0, 0.0]);
                        }
                    }
                } else {
                    ok = false;
                    break;
                }
            } else {
                ok = false;
                break;
            }
        }
        if ok && face.len() == 3 {
            faces.push(face);
            face_mat_syms.push(mat_symbol.clone());
            // Only include UV data if we successfully collected all corners.
            if tc_info.is_some() && corner_uvs.len() == 3 {
                face_corner_uvs.push(corner_uvs);
            } else {
                face_corner_uvs.push(Vec::new());
            }
            // Triangles never have holes.
            face_holes.push(Vec::new());
        }
    }

    Some(RawMesh {
        positions: positions.to_vec(),
        faces,
        face_material_symbols: face_mat_syms,
        face_corner_uvs,
        face_holes,
    })
}

fn extract_polylist(
    pl: &dae_parser::PolyList,
    positions: &[Point3],
    mesh: &dae_parser::Mesh,
) -> Option<RawMesh> {
    let vertex_offset = find_vertex_offset(&pl.inputs)?;
    let stride = pl.inputs.stride;
    let mat_symbol = pl.material.clone().unwrap_or_default();
    let prim_data = &pl.data.prim;
    let vcount = &pl.data.vcount;

    // Optional TEXCOORD source.
    let tc_info = find_texcoord_source(&pl.inputs, mesh);

    let mut faces = Vec::new();
    let mut face_mat_syms = Vec::new();
    let mut face_corner_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut face_holes: Vec<Vec<Vec<usize>>> = Vec::new();
    let mut prim_idx = 0usize;

    for &vc in vcount.iter() {
        let vc = vc as usize;
        let mut face = Vec::with_capacity(vc);
        let mut corner_uvs: Vec<[f64; 2]> = Vec::new();
        let mut ok = true;
        for _ in 0..vc {
            let idx = prim_idx + vertex_offset;
            if idx < prim_data.len() {
                let vi = prim_data[idx] as usize;
                if vi < positions.len() {
                    face.push(vi);
                    // Extract texcoord for this corner.
                    if let Some((tc_offset, ref tc_data)) = tc_info {
                        let tc_idx = prim_idx + tc_offset;
                        if tc_idx < prim_data.len() {
                            let tci = prim_data[tc_idx] as usize;
                            if tci < tc_data.len() {
                                corner_uvs.push(tc_data[tci]);
                            } else {
                                corner_uvs.push([0.0, 0.0]);
                            }
                        } else {
                            corner_uvs.push([0.0, 0.0]);
                        }
                    }
                } else {
                    ok = false;
                }
            } else {
                ok = false;
            }
            prim_idx += stride;
        }
        if ok && face.len() >= 3 {
            faces.push(face);
            face_mat_syms.push(mat_symbol.clone());
            // Only include UV data if we successfully collected all corners.
            if tc_info.is_some() && corner_uvs.len() == vc {
                face_corner_uvs.push(corner_uvs);
            } else {
                face_corner_uvs.push(Vec::new());
            }
            // Polylist never has holes.
            face_holes.push(Vec::new());
        }
    }

    Some(RawMesh {
        positions: positions.to_vec(),
        faces,
        face_material_symbols: face_mat_syms,
        face_corner_uvs,
        face_holes,
    })
}

fn extract_polygons(
    polys: &dae_parser::Polygons,
    positions: &[Point3],
    mesh: &dae_parser::Mesh,
) -> Option<RawMesh> {
    let vertex_offset = find_vertex_offset(&polys.inputs)?;
    let stride = polys.inputs.stride;
    let mat_symbol = polys.material.clone().unwrap_or_default();

    // Optional TEXCOORD source.
    let tc_info = find_texcoord_source(&polys.inputs, mesh);

    let mut faces = Vec::new();
    let mut face_mat_syms = Vec::new();
    let mut face_corner_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut face_holes: Vec<Vec<Vec<usize>>> = Vec::new();

    // polys.data is PolygonGeom which Derefs to Vec<PolygonHole>.
    // Each PolygonHole has:
    //   .verts: Box<[u32]>        — outer loop index data
    //   .hole: Vec<Box<[u32]>>   — inner hole loops (one per <h> element)
    for poly_hole in polys.data.iter() {
        let prim = &poly_hole.verts;
        let mut face = Vec::new();
        let mut corner_uvs: Vec<[f64; 2]> = Vec::new();
        let mut ok = true;
        let num_verts = prim.len() / stride;
        for v in 0..num_verts {
            let base = v * stride;
            let idx = base + vertex_offset;
            if idx < prim.len() {
                let vi = prim[idx] as usize;
                if vi < positions.len() {
                    face.push(vi);
                    // Extract texcoord for this corner.
                    if let Some((tc_offset, ref tc_data)) = tc_info {
                        let tc_idx = base + tc_offset;
                        if tc_idx < prim.len() {
                            let tci = prim[tc_idx] as usize;
                            if tci < tc_data.len() {
                                corner_uvs.push(tc_data[tci]);
                            } else {
                                corner_uvs.push([0.0, 0.0]);
                            }
                        } else {
                            corner_uvs.push([0.0, 0.0]);
                        }
                    }
                } else {
                    ok = false;
                    break;
                }
            }
        }
        if ok && face.len() >= 3 {
            // Only include UV data if we successfully collected all corners.
            if tc_info.is_some() && corner_uvs.len() == face.len() {
                face_corner_uvs.push(corner_uvs);
            } else {
                face_corner_uvs.push(Vec::new());
            }
            faces.push(face);
            face_mat_syms.push(mat_symbol.clone());

            // Read inner hole loops from <h> elements. Each <h> is a flat
            // index array with the same stride as the outer loop; we only
            // extract vertex indices (holes carry no per-corner UVs in the
            // recipe). Skip a hole loop that resolves to fewer than 3 valid
            // vertex indices.
            let mut this_face_holes: Vec<Vec<usize>> = Vec::new();
            for hole_data in &poly_hole.hole {
                let hole_num_verts = hole_data.len() / stride;
                let mut hole_loop: Vec<usize> = Vec::with_capacity(hole_num_verts);
                let mut hole_ok = true;
                for v in 0..hole_num_verts {
                    let base = v * stride;
                    let idx = base + vertex_offset;
                    if idx < hole_data.len() {
                        let vi = hole_data[idx] as usize;
                        if vi < positions.len() {
                            hole_loop.push(vi);
                        } else {
                            hole_ok = false;
                            break;
                        }
                    } else {
                        hole_ok = false;
                        break;
                    }
                }
                if hole_ok && hole_loop.len() >= 3 {
                    this_face_holes.push(hole_loop);
                }
                // Holes with < 3 valid indices are silently skipped (degenerate).
            }
            face_holes.push(this_face_holes);
        }
    }

    Some(RawMesh {
        positions: positions.to_vec(),
        faces,
        face_material_symbols: face_mat_syms,
        face_corner_uvs,
        face_holes,
    })
}

// ── Library nodes ─────────────────────────────────────────────────────────────

fn build_lib_node_map(doc: &DaeDoc) -> HashMap<String, Node> {
    let mut map = HashMap::new();
    for lib_elem in &doc.library {
        let Some(lib) = dae_parser::Node::extract_element(lib_elem) else {
            continue;
        };
        for node in &lib.items {
            if let Some(id) = &node.id {
                map.insert(id.clone(), node.clone());
            }
        }
    }
    map
}

// ── Visual scene walker ───────────────────────────────────────────────────────

fn walk_visual_scene(
    vs: &VisualScene,
    geom_map: &HashMap<String, Vec<RawMesh>>,
    id_to_dense: &HashMap<String, u32>,
    world_tf: &Transform,
    lib_node_to_def_idx: &HashMap<String, usize>,
    lib_nodes: &HashMap<String, Node>,
) -> Vec<ImportNode> {
    let mut roots = Vec::new();
    for node in &vs.nodes {
        // Root node accumulation: pure COLLADA-unit transform (no world_tf yet).
        let acc = compose_node_local(node);
        if let Some(import_node) = convert_node(
            node,
            &acc,
            world_tf,
            geom_map,
            id_to_dense,
            lib_node_to_def_idx,
            lib_nodes,
        ) {
            roots.push(import_node);
        }
    }
    roots
}

fn dae_to_kernel_transform(t: &dae_parser::Transform) -> Transform {
    match t {
        dae_parser::Transform::Matrix(m) => {
            // COLLADA <matrix> is ROW-MAJOR 4×4:
            //   c[0..4]  = row 0: [m00, m01, m02, tx]
            //   c[4..8]  = row 1: [m10, m11, m12, ty]
            //   c[8..12] = row 2: [m20, m21, m22, tz]
            //   c[12..16]= row 3: [0,   0,   0,   1 ] (ignored)
            // from_affine expects the same row-major 3×4 layout, so pass
            // the first 12 elements straight through.
            let c = &m.0;
            Transform::from_affine(&[
                c[0] as f64,
                c[1] as f64,
                c[2] as f64,
                c[3] as f64,
                c[4] as f64,
                c[5] as f64,
                c[6] as f64,
                c[7] as f64,
                c[8] as f64,
                c[9] as f64,
                c[10] as f64,
                c[11] as f64,
            ])
        }
        dae_parser::Transform::Translate(tr) => {
            let t = &tr.0;
            Transform::translation(Vec3::new(t[0] as f64, t[1] as f64, t[2] as f64))
        }
        dae_parser::Transform::Scale(s) => {
            let v = &s.0;
            Transform::scale(Vec3::new(v[0] as f64, v[1] as f64, v[2] as f64))
        }
        dae_parser::Transform::Rotate(r) => {
            let v = &r.0;
            // [axis_x, axis_y, axis_z, angle_degrees]
            let axis = Vec3::new(v[0] as f64, v[1] as f64, v[2] as f64);
            let angle_deg = v[3] as f64;
            let angle_rad = angle_deg.to_radians();
            Transform::rotation(axis, angle_rad).unwrap_or(Transform::IDENTITY)
        }
        dae_parser::Transform::LookAt(_) | dae_parser::Transform::Skew(_) => {
            Transform::IDENTITY // not supported for solid geometry
        }
    }
}

/// Convert one COLLADA `Node` to an `ImportNode`.
///
/// `acc` is the accumulated COLLADA-unit transform from the root down to this
/// node (no `world_tf` baked in yet). `world_tf` is applied only when
/// producing final positions (world meshes) or instance poses.
///
/// Transform composition rules (kernel: `a.then(&b)` = apply a first, then b):
///   - child accumulation: `child_local.then(&acc)` → child-local is innermost,
///     parent accumulation is outermost → `acc(child_local(p))`.
///   - world mesh bake:    `acc.then(&world_tf)`     → COLLADA units first, unit-scale last.
///   - instance pose:      `acc.then(&world_tf)`     → same; def geometry stays raw (inches).
#[allow(clippy::only_used_in_recursion)]
fn convert_node(
    node: &Node,
    acc: &Transform, // accumulated COLLADA-unit transform (no world_tf)
    world_tf: &Transform,
    geom_map: &HashMap<String, Vec<RawMesh>>,
    id_to_dense: &HashMap<String, u32>,
    lib_node_to_def_idx: &HashMap<String, usize>,
    lib_nodes: &HashMap<String, Node>,
) -> Option<ImportNode> {
    // Case 1: instance_geometry → world mesh(es)
    // Case 2: instance_node → Instance referencing a DefRecipe
    // Case 3: child nodes → Group

    // World transform = COLLADA-unit placement composed with unit/up-axis correction.
    let world_bake = acc.then(world_tf);

    // Collect geometry instances into direct mesh nodes.
    let mut mesh_nodes: Vec<ImportNode> = Vec::new();

    for ig in &node.instance_geometry {
        let geom_id = url_as_str(&ig.url).to_string();
        if let Some(raw_meshes) = geom_map.get(&geom_id) {
            // Build material symbol → dense index map from this instance's bind_material.
            let sym_to_dense = build_sym_to_dense(ig, id_to_dense);

            for raw in raw_meshes {
                // World meshes: bake `acc.then(&world_tf)` into positions so they
                // land in meters, Z-up world space.
                let face_mats: Vec<u32> = raw
                    .face_material_symbols
                    .iter()
                    .map(|sym| {
                        sym_to_dense
                            .get(sym)
                            .copied()
                            .unwrap_or(kernel::NO_MATERIAL)
                    })
                    .collect();

                let (positions, faces, healed_mats, healed_uvs, healed_holes) =
                    crate::heal::heal_mesh(
                        &raw.positions,
                        &raw.faces,
                        &face_mats,
                        &raw.face_corner_uvs,
                        &raw.face_holes,
                        &world_bake,
                    );

                if !faces.is_empty() {
                    // Fit per-face UV frames from healed positions + healed corner UVs.
                    let face_uv_frames: Vec<Option<UvFrame>> = faces
                        .iter()
                        .zip(healed_uvs.iter())
                        .map(|(face, corner_uvs)| {
                            if corner_uvs.len() == face.len() && corner_uvs.len() >= 3 {
                                let corner_pos: Vec<Point3> =
                                    face.iter().map(|&vi| positions[vi]).collect();
                                fit_uv_frame(&corner_pos, corner_uvs)
                            } else {
                                None
                            }
                        })
                        .collect();

                    let name = node
                        .name
                        .clone()
                        .or_else(|| node.id.clone())
                        .unwrap_or_else(|| geom_id.clone());
                    mesh_nodes.push(ImportNode::Mesh(MeshRecipe {
                        name,
                        positions,
                        faces,
                        face_materials: healed_mats,
                        face_uv_frames,
                        face_holes: healed_holes,
                        base_material: kernel::NO_MATERIAL,
                    }));
                }
            }
        }
    }

    // Collect instance_node references → ImportNode::Instance.
    let mut instance_nodes: Vec<ImportNode> = Vec::new();
    for inst_node in &node.instance_node {
        let node_ref_id = url_as_str(&inst_node.url);
        if let Some(&def_idx) = lib_node_to_def_idx.get(node_ref_id) {
            // Pose = acc.then(&world_tf): def-local (raw COLLADA inches) → world meters.
            // The def geometry is built raw (IDENTITY), so the pose carries everything.
            instance_nodes.push(ImportNode::Instance {
                def: def_idx,
                pose: world_bake,
            });
        }
    }

    // Recurse into children.
    let mut child_nodes: Vec<ImportNode> = Vec::new();
    for child in &node.children {
        let child_local = compose_node_local(child);
        // Child accumulation: child-local is innermost (applied first to p),
        // parent acc is outermost (applied second).
        // child_acc(p) = acc(child_local(p)) = child_local.then(&acc)(p).
        let child_acc = child_local.then(acc);
        if let Some(cn) = convert_node(
            child,
            &child_acc,
            world_tf,
            geom_map,
            id_to_dense,
            lib_node_to_def_idx,
            lib_nodes,
        ) {
            child_nodes.push(cn);
        }
    }

    // Combine all results.
    let all: Vec<ImportNode> = mesh_nodes
        .into_iter()
        .chain(instance_nodes)
        .chain(child_nodes)
        .collect();

    match all.len() {
        0 => None,
        1 => Some(all.into_iter().next().unwrap()),
        _ => {
            // Wrap in a group.
            let name = node
                .name
                .clone()
                .or_else(|| node.id.clone())
                .unwrap_or_default();
            Some(ImportNode::Group {
                name,
                children: all,
            })
        }
    }
}

/// Compose just the local transforms of a node (without world_tf).
///
/// COLLADA spec: the first listed transform is outermost, i.e. the combined
/// matrix is `T0 · T1 · … · Tn` applied to a column vector (`T0` multiplied
/// last). With the kernel convention where `a.then(&b)` means "apply `a`
/// first, then `b`", outermost = applied last = rightmost in `.then()` chains.
/// We prepend each new transform so T0 ends up outermost:
///   - after T0: mat = T0
///   - after T1: mat = T1.then(&T0)  →  T0·T1·p  ✓
///   - after Tn: mat = Tn.then(&…).then(&T0)
fn compose_node_local(node: &Node) -> Transform {
    let mut mat = Transform::IDENTITY;
    for t in &node.transforms {
        let k = dae_to_kernel_transform(t);
        // Prepend: k is applied FIRST (innermost), current mat SECOND (outermost).
        mat = k.then(&mat);
    }
    mat
}

/// Build symbol → dense_index from an `Instance<Geometry>`.
fn build_sym_to_dense(
    ig: &dae_parser::Instance<Geometry>,
    id_to_dense: &HashMap<String, u32>,
) -> HashMap<String, u32> {
    let mut sym_map: HashMap<String, u32> = HashMap::new();
    for im in ig.instance_materials() {
        let mat_id = url_as_str(&im.target);
        if let Some(&dense) = id_to_dense.get(mat_id) {
            sym_map.insert(im.symbol.clone(), dense);
        }
    }
    sym_map
}

/// Collect meshes from a library node (def-local space).
///
/// Def geometry is kept in raw COLLADA units (e.g. inches) because the
/// Instance.pose = `acc.then(&world_tf)` already carries the full unit-scale +
/// up-axis correction. Baking `world_tf` here would double-apply it.
///
/// We pass `Transform::IDENTITY` to `heal_mesh` so positions are only welded
/// and deduped, not unit-scaled. The kernel applies the pose when baking the
/// instance into world space.
fn collect_meshes_from_node(
    node: &Node,
    geom_map: &HashMap<String, Vec<RawMesh>>,
    id_to_dense: &HashMap<String, u32>,
    meshes: &mut Vec<MeshRecipe>,
) {
    for ig in &node.instance_geometry {
        let geom_id = url_as_str(&ig.url).to_string();
        if let Some(raw_meshes) = geom_map.get(&geom_id) {
            let sym_to_dense = build_sym_to_dense(ig, id_to_dense);

            for raw in raw_meshes {
                let face_mats: Vec<u32> = raw
                    .face_material_symbols
                    .iter()
                    .map(|sym| {
                        sym_to_dense
                            .get(sym)
                            .copied()
                            .unwrap_or(kernel::NO_MATERIAL)
                    })
                    .collect();

                // Def positions stay in COLLADA units (raw). IDENTITY → no transform baked.
                let (positions, faces, healed_mats, healed_uvs, healed_holes) = heal_mesh(
                    &raw.positions,
                    &raw.faces,
                    &face_mats,
                    &raw.face_corner_uvs,
                    &raw.face_holes,
                    &Transform::IDENTITY,
                );

                if !faces.is_empty() {
                    // Fit per-face UV frames from def-local positions + healed corner UVs.
                    let face_uv_frames: Vec<Option<UvFrame>> = faces
                        .iter()
                        .zip(healed_uvs.iter())
                        .map(|(face, corner_uvs)| {
                            if corner_uvs.len() == face.len() && corner_uvs.len() >= 3 {
                                let corner_pos: Vec<Point3> =
                                    face.iter().map(|&vi| positions[vi]).collect();
                                fit_uv_frame(&corner_pos, corner_uvs)
                            } else {
                                None
                            }
                        })
                        .collect();

                    let name = node
                        .name
                        .clone()
                        .or_else(|| node.id.clone())
                        .unwrap_or_else(|| geom_id.clone());
                    meshes.push(MeshRecipe {
                        name,
                        positions,
                        faces,
                        face_materials: healed_mats,
                        face_uv_frames,
                        face_holes: healed_holes,
                        base_material: kernel::NO_MATERIAL,
                    });
                }
            }
        }
    }

    // Recurse into children of the library node.
    for child in &node.children {
        collect_meshes_from_node(child, geom_map, id_to_dense, meshes);
    }
}
