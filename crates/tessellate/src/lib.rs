//! Kernel topology -> renderer-agnostic buffers.
//!
//! Isolates the kernel from the renderer (ARCHITECTURE.md): the output is plain
//! f32/u32 arrays a WebGL2 viewport can upload directly. Flat-shaded look:
//! vertices are duplicated per face so each face keeps its own normal, and
//! unique edges come out as line segments for the SketchUp-style display.
//! Faces stamped with an analytic surface reference
//! (the true-curves design) are the exception: they shade with the
//! true per-vertex surface normal, and the seams interior to one logical
//! wall land in a separate, suppressed soft-edge buffer.
//!
//! Triangulation supports non-convex faces and faces with holes via ear
//! clipping with hole bridging (Eberly "Triangulation by Ear Clipping").

use kernel::triangulate::{UnbridgeableHole, bridge_holes, ear_clip};
use kernel::{FaceId, MaterialId, MaterialPalette, Object, Rgba8};

mod refacet;

/// Orthonormal (u, v) plane basis for a unit normal. Re-exported from
/// [`kernel::triangulate`] so a caller resolving a face's planar UV
/// projection outside this crate (`crates/mesh-export`'s material-aware
/// export) uses the identical basis [`tessellate`] does, rather than a
/// second, potentially-diverging copy.
pub use kernel::triangulate::plane_basis;
pub use refacet::{
    MAX_SEGMENTS_PER_TURN, MIN_SEGMENTS_PER_TURN, export_triangles, export_triangles_with_faces,
};

/// Default face color when no material is assigned (or the id is stale):
/// a neutral light gray (`0xcccccc`). Exposed so other crates resolving a
/// face's effective color (`crates/mesh-export`'s material-aware export)
/// use the exact same fallback as the live renderer's own [`tessellate`],
/// rather than a second copy of the magic number.
pub const DEFAULT_MATERIAL_RGBA: Rgba8 = Rgba8::rgb(0xcc, 0xcc, 0xcc);

/// Tessellation failed; the Object uses topology this version cannot
/// triangulate honestly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TessellateError {
    /// Face has fewer than 3 boundary vertices (kernel invariant breach).
    DegenerateFace {
        /// The offending face.
        face: FaceId,
    },
    /// A hole loop could not be bridged into the face's boundary without
    /// crossing other geometry, so the face has no honest triangulation
    /// under this algorithm. Unreachable for kernel-valid faces (disjoint
    /// holes strictly inside the outer loop always leave a visible bridge
    /// target — see [`kernel::triangulate::bridge_holes`]) but one: a face
    /// of an imported object saved before imports were checked for
    /// overlapping holes (`kernel::TopologyError::HolesOverlap`, held of
    /// imports where they are built so such documents still open). A caller
    /// drawing many objects skips the one that fails rather than failing
    /// them all. Failing loudly instead of emitting a self-crossing ring is
    /// DEVELOPMENT.md rule 4.
    UnbridgeableHole {
        /// The offending face.
        face: FaceId,
    },
    /// The Smooth surface could not be built (a cage too dense for its
    /// level, or a non-manifold cage).
    Subdivide(kernel::ShapeError),
}

impl std::fmt::Display for TessellateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TessellateError::DegenerateFace { face } => {
                write!(f, "face {face:?} has fewer than 3 vertices")
            }
            TessellateError::UnbridgeableHole { face } => {
                write!(f, "face {face:?} has a hole that cannot be bridged")
            }
            TessellateError::Subdivide(e) => write!(f, "the smooth surface failed: {e}"),
        }
    }
}

impl std::error::Error for TessellateError {}

/// Describes a contiguous run of the index buffer that belongs to one material.
/// Used by the renderer to set up `THREE.BufferGeometry.addGroup(start, count, i)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialGroup {
    /// The material covering this index range, or `None` = default material.
    pub material: Option<MaterialId>,
    /// The range draws faces' painted backs ([`kernel::Face::back_material`]):
    /// a copy of each such face wound the other way, lying on the face. A
    /// renderer draws it front-side only and in front of the face's own
    /// back side (which an open shell otherwise shows); exporters of the
    /// solid's surface skip it.
    pub back: bool,
    /// First index (into `RenderMesh::indices`) in this run.
    pub start: u32,
    /// Number of indices (always a multiple of 3) in this run.
    pub count: u32,
}

/// Flat-shaded render buffers for one Object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderMesh {
    /// Triangle vertex positions, xyz per vertex, duplicated per face.
    pub positions: Vec<f32>,
    /// Per-vertex normals (constant across each face).
    pub normals: Vec<f32>,
    /// Triangle indices into `positions`, sorted so each material's triangles
    /// are contiguous (see `groups`).
    pub indices: Vec<u32>,
    /// Per-vertex RGB colors (3 floats, range 0–1), parallel to `positions`.
    pub colors: Vec<f32>,
    /// Per-vertex UV coordinates (2 floats), parallel to `positions`.
    pub uvs: Vec<f32>,
    /// Contiguous runs of the index buffer per material (used by the renderer
    /// to build `addGroup` calls). Always at least one entry.
    pub groups: Vec<MaterialGroup>,
    /// Line-segment endpoints (xyz pairs), one segment per unique **hard**
    /// edge — every edge except the interior facet seams of one logical
    /// curved wall (see `soft_edge_positions`).
    pub edge_positions: Vec<f32>,
    /// Line-segment endpoints (xyz pairs) of the **soft** edges: interior
    /// seams between two facets claiming the same analytic surface
    /// (`kernel::SurfaceRef`, the true-curves design stage 4). The
    /// viewport suppresses these — a drawn cylinder reads as one smooth
    /// wall, not 24 pin-striped facets — while cap rims and the seams
    /// against unattributed faces stay hard. Derived at tessellation time
    /// from surface equality; nothing new is persisted.
    pub soft_edge_positions: Vec<f32>,
    /// Line-segment endpoints (xyz pairs) of the **tangent** edges: edges
    /// between two different surfaces that meet tangentially (fillet rails,
    /// a corner patch's border, a rounded rectangle's wall-to-corner joints),
    /// classified by `Object::edge_smoothness`: the structure lines of a
    /// curved part. A crease inside one blend (a miter, two strips meeting
    /// over a sharp edge) is a hard edge, not one of these. Drawn thin by
    /// View ▸ Tangent Edges; line drawings treat them like soft edges.
    /// Empty for an object with no analytic claims.
    pub tangent_edge_positions: Vec<f32>,
    /// Per-face vertex range into `positions`/`normals`/`colors`/`uvs`:
    /// `(face, base, count)`, `base`/`count` in VERTICES (not floats) — face
    /// `f`'s vertices occupy `[base*3, (base+count)*3)` of `positions` and
    /// `[base*2, (base+count)*2)` of `uvs`. Lets a caller patch one face's
    /// `uv` range in place (the Position Texture tool's live preview,
    /// paint-tool design §3) without re-tessellating the whole object. Empty
    /// faces (a degenerate 0-vertex outer loop) never occur — tessellation
    /// would already have failed with `DegenerateFace` before this is built.
    pub face_ranges: Vec<(FaceId, u32, u32)>,
    /// Line-segment endpoints (xyz pairs) of every edge of the cage, for
    /// the wire overlay of a smoothed object ([`tessellate_smoothed`]).
    /// Empty from [`tessellate`].
    pub cage_edge_positions: Vec<f32>,
}

impl RenderMesh {
    /// The faces' own triangles, three indices each: every group but the
    /// painted backs ([`MaterialGroup::back`]), which lie on them.
    pub fn front_triangles(&self) -> impl Iterator<Item = &[u32]> + '_ {
        self.groups.iter().filter(|g| !g.back).flat_map(|g| {
            self.indices[g.start as usize..(g.start + g.count) as usize]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| t.as_slice())
        })
    }
}

/// Tessellates an Object into flat-shaded triangle and edge-line buffers.
///
/// `palette` is the document's material palette; it is used to resolve each
/// face's `material` field into per-vertex colors and UV coordinates.
///
/// Faces are triangulated via ear clipping (supports non-convex faces and
/// faces with holes). Triangles wind counter-clockwise seen from outside —
/// front faces under the WebGL default.
///
/// The index buffer is grouped by material so each material's triangles are
/// contiguous (see `RenderMesh::groups`). Positions/normals/colors/uvs remain
/// in face order (not sorted); only indices are reordered.
///
/// [`tessellate_with`] with no inherited material: what a world Object (or
/// a definition shown on its own) draws.
pub fn tessellate(
    object: &Object,
    palette: &MaterialPalette,
) -> Result<RenderMesh, TessellateError> {
    tessellate_filtered(object, palette, None, None)
}

/// [`tessellate`] for an Object drawn through a placement whose material
/// is `inherited` (the kernel's `Document::inherited_material`): a face
/// resolves to its own material, else the Object's base, else `inherited`,
/// for the baked colors and the group key alike. Back paint is untouched.
pub fn tessellate_with(
    object: &Object,
    palette: &MaterialPalette,
    inherited: Option<MaterialId>,
) -> Result<RenderMesh, TessellateError> {
    tessellate_filtered(object, palette, inherited, None)
}

/// Tessellates only the listed faces of an Object: the same code as
/// [`tessellate`] with every other face skipped, so each listed face gets
/// exactly the vertices, normals, colors and UVs it would get in the full
/// mesh. Indices address the returned buffers (a compact mesh holding just
/// those faces). The edge-line buffers stay empty. Ids that are not faces of
/// `object` are ignored.
///
/// # Errors
/// As [`tessellate`], for the listed faces only.
pub fn tessellate_faces(
    object: &Object,
    faces: &[FaceId],
    palette: &MaterialPalette,
) -> Result<RenderMesh, TessellateError> {
    tessellate_filtered(object, palette, None, Some(faces))
}

fn tessellate_filtered(
    object: &Object,
    palette: &MaterialPalette,
    inherited: Option<MaterialId>,
    only: Option<&[FaceId]>,
) -> Result<RenderMesh, TessellateError> {
    // First pass: collect per-face geometry without committing index order.
    // Each entry: (material_id, base_vertex, triangles, vertex_data).
    struct FaceData {
        material: Option<MaterialId>,
        back: bool,
        base: u32,
        triangles: Vec<[usize; 3]>,
    }

    let mut mesh = RenderMesh::default();
    let mut face_data: Vec<FaceData> = Vec::new();
    if only.is_none() {
        // Sized to the corners up front (a bridged hole adds two more): an
        // output grown by doubling copies itself over and over on a dense
        // scan.
        let holes = object.loops().len().saturating_sub(object.faces().len());
        let corners = object.half_edges().len() + 2 * holes;
        mesh.positions.reserve(corners * 3);
        mesh.normals.reserve(corners * 3);
        mesh.colors.reserve(corners * 3);
        mesh.uvs.reserve(corners * 2);
        face_data.reserve(object.faces().len());
    }

    for (face_id, face) in object.faces() {
        if only.is_some_and(|keep| !keep.contains(&face_id)) {
            continue;
        }
        let normal = face.plane.normal();
        let n = [normal.x as f32, normal.y as f32, normal.z as f32];

        // Analytic smooth shading (the true-curves design stage 4): a
        // chord facet of a stamped cylinder wall shades with the TRUE
        // surface normal at each vertex — the radial direction off the
        // axis, oriented to the same side the facet faces (outer walls
        // radiate outward, hole walls inward). Adjacent facets of one wall
        // then share identical vertex normals along their seams, so the
        // wall renders as one smooth surface with no per-edge data.
        let smooth: Option<(kernel::Point3, kernel::Vec3, f64)> = face.surface.and_then(|sr| {
            let kernel::SurfaceRef::Cylinder {
                axis_point, axis, ..
            } = sr
            else {
                return None;
            };
            // A chord plane never contains the axis; the side it faces
            // fixes the radial sign (same derivation as the kernel's
            // whole-wall push/pull).
            let side = face.plane.signed_distance(axis_point);
            (side.abs() > kernel::tol::POINT_MERGE).then_some((
                axis_point,
                axis,
                if side < 0.0 { 1.0 } else { -1.0 },
            ))
        });

        // The other analytic classes (sphere, torus, cone) shade the same
        // way through the claim's own normal, flipped to the side the facet
        // faces; a vertex where that normal is undefined or edge-on to the
        // facet shades flat.
        let analytic_other: Option<kernel::SurfaceRef> = face
            .surface
            .filter(|sr| !matches!(sr, kernel::SurfaceRef::Cylinder { .. }));

        // SOFT-edge smoothing (design §7b): a vertex on a marked soft
        // edge shades with the average of the incident faces' plane
        // normals, so both walls of a smooth sweep joint agree at their
        // shared vertices and the facet crease disappears. Accumulated by
        // exact endpoint position; the analytic cylinder path above takes
        // precedence where it applies.
        // One walk per loop gathers the corners (in loop order, as
        // `loop_positions` lists them) and the soft-edge normals.
        let mut soft_at: Vec<(kernel::Point3, kernel::Vec3)> = Vec::new();
        let mut outer_3d: Vec<[f64; 3]> = Vec::new();
        let mut holes_3d: Vec<Vec<[f64; 3]>> = Vec::with_capacity(face.inner_loops.len());
        {
            let half_edges = object.half_edges();
            let vertices = object.vertices();
            let mut visit = |loop_id, corners: &mut Vec<[f64; 3]>| {
                let first = object.loops()[loop_id].first_half_edge;
                let mut h = first;
                loop {
                    let he = &half_edges[h];
                    let a = vertices[he.origin].position;
                    corners.push([a.x, a.y, a.z]);
                    let edge = &object.edges()[he.edge];
                    if edge.soft
                        && let Some(twin_h) = edge.twin_half_edge
                    {
                        let other = if twin_h == h { edge.half_edge } else { twin_h };
                        let nb_loop = half_edges[other].loop_id;
                        let nb_face = &object.faces()[object.loops()[nb_loop].face];
                        let nb_n = nb_face.plane.normal();
                        let b = vertices[half_edges[he.next].origin].position;
                        for p in [a, b] {
                            if let Some(entry) = soft_at.iter_mut().find(|(q, _)| *q == p) {
                                entry.1 = entry.1 + nb_n;
                            } else {
                                soft_at.push((p, nb_n));
                            }
                        }
                    }
                    h = he.next;
                    if h == first {
                        break;
                    }
                }
            };
            visit(face.outer_loop, &mut outer_3d);
            for &il in &face.inner_loops {
                let mut corners = Vec::new();
                visit(il, &mut corners);
                holes_3d.push(corners);
            }
        }

        // A blend face (a fillet strip, corner or runout facet) shades
        // smooth across the joints inside its blend and meets the face it is
        // tangent to along a rail with that face's normal; the kernel reads
        // the fan around each corner (normals only, no vertex moves). It
        // takes precedence over the per-face paths below.
        let blend_at: std::collections::BTreeMap<[u64; 3], [f32; 3]> = if face.blend.is_some() {
            object
                .blend_shading_normals(face_id)
                .into_iter()
                .map(|(p, n)| {
                    (
                        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()],
                        [n.x as f32, n.y as f32, n.z as f32],
                    )
                })
                .collect()
        } else {
            std::collections::BTreeMap::new()
        };

        // Build the orthonormal 2D basis (u, v) such that u × v = normal.
        let (u_ax, v_ax) = plane_basis(normal);

        // Resolve the *effective* material for color, UV world_size, AND the
        // group key: a face with no own material falls back to the object's
        // base material ( follow-up), so faces grown by extrude/boolean and
        // textured bases all render consistently; with neither, to the
        // material the placement passes down.
        let material_id = face.material.or(object.default_material()).or(inherited);
        let (color, world_size) = match material_id.and_then(|id| palette.get(id)) {
            Some(mat) => {
                let c = mat.color;
                let ws = mat
                    .texture
                    .as_ref()
                    .map(|t| t.world_size)
                    .unwrap_or([1.0, 1.0]);
                (c, ws)
            }
            None => (DEFAULT_MATERIAL_RGBA, [1.0, 1.0]),
        };
        let cr = color.r as f32 / 255.0;
        let cg = color.g as f32 / 255.0;
        let cb = color.b as f32 / 255.0;

        if outer_3d.len() < 3 {
            return Err(TessellateError::DegenerateFace { face: face_id });
        }

        // Build the (possibly bridged) simple polygon for ear clipping.
        // Each entry is [x3d, y3d, z3d, u2d, v2d].
        let poly = bridge_holes(&outer_3d, &holes_3d, u_ax, v_ax)
            .map_err(|UnbridgeableHole| TessellateError::UnbridgeableHole { face: face_id })?;

        // Ear-clip the polygon and emit triangles.
        let base = (mesh.positions.len() / 3) as u32;

        // Append polygon vertices (positions, normals, colors, UVs).
        for &[x, y, z, u2d, v2d] in &poly {
            mesh.positions.extend([x as f32, y as f32, z as f32]);
            let vertex_normal = match smooth {
                _ if blend_at.contains_key(&[x.to_bits(), y.to_bits(), z.to_bits()]) => {
                    blend_at[&[x.to_bits(), y.to_bits(), z.to_bits()]]
                }
                Some((axis_point, axis, sign)) => {
                    let d = kernel::Point3::new(x, y, z) - axis_point;
                    let radial = d - axis * d.dot(axis);
                    match (radial * sign).normalized() {
                        Ok(u) => [u.x as f32, u.y as f32, u.z as f32],
                        // A vertex on the axis cannot happen on a valid
                        // claim (radius > 0); shade flat rather than guess.
                        Err(_) => n,
                    }
                }
                None if analytic_other.is_some() => {
                    let p = kernel::Point3::new(x, y, z);
                    analytic_other
                        .and_then(|sr| sr.normal_at(p))
                        .and_then(|a| {
                            let along = a.dot(normal);
                            (along.abs() > kernel::tol::NORMALIZE_MIN_LENGTH)
                                .then(|| if along < 0.0 { -a } else { a })
                        })
                        .map_or(n, |a| [a.x as f32, a.y as f32, a.z as f32])
                }
                None => {
                    let p = kernel::Point3::new(x, y, z);
                    match soft_at.iter().find(|(q, _)| *q == p) {
                        Some((_, acc)) => match (normal + *acc).normalized() {
                            Ok(avg) => [avg.x as f32, avg.y as f32, avg.z as f32],
                            Err(_) => n, // opposed normals: honest flat
                        },
                        None => n,
                    }
                }
            };
            mesh.normals.extend(vertex_normal);
            mesh.colors.extend([cr, cg, cb]);
            // UV: if the face has an oriented UV frame (imported texcoords), use
            // it — `uv = frame.apply(p)`. Otherwise fall back to the planar
            // projection divided by world_size. Untextured/Hew-drawn faces
            // (no frame) produce byte-identical output to previous behaviour.
            let (fu, fv) = if let Some(frame) = face.uv_frame {
                let uv = frame.apply(kernel::Point3::new(x, y, z));
                (uv[0] as f32, uv[1] as f32)
            } else {
                ((u2d / world_size[0]) as f32, (v2d / world_size[1]) as f32)
            };
            mesh.uvs.extend([fu, fv]);
        }
        mesh.face_ranges.push((face_id, base, poly.len() as u32));

        // Run ear clipping (local indices into poly), keeping only the
        // triangles the f32 buffers can hold with their winding (see
        // `tri_winds_forward_in_f32`).
        let triangles: Vec<[usize; 3]> = ear_clip(&poly)
            .into_iter()
            .filter(|&tri| tri_winds_forward_in_f32(&poly, normal, tri))
            .collect();

        // A painted back: the same polygon again, facing the other way,
        // in the back's color and with its texture's scale.
        if let Some(back) = face.back_material.filter(|&b| Some(b) != material_id) {
            let back_base = (mesh.positions.len() / 3) as u32;
            let (bcolor, bws) = match palette.get(back) {
                Some(mat) => (
                    mat.color,
                    mat.texture
                        .as_ref()
                        .map(|t| t.world_size)
                        .unwrap_or([1.0, 1.0]),
                ),
                None => (DEFAULT_MATERIAL_RGBA, [1.0, 1.0]),
            };
            let front_normals = mesh.normals[base as usize * 3..].to_vec();
            for (k, &[x, y, z, u2d, v2d]) in poly.iter().enumerate() {
                mesh.positions.extend([x as f32, y as f32, z as f32]);
                mesh.normals
                    .extend(front_normals[k * 3..k * 3 + 3].iter().map(|c| -c));
                mesh.colors.extend([
                    bcolor.r as f32 / 255.0,
                    bcolor.g as f32 / 255.0,
                    bcolor.b as f32 / 255.0,
                ]);
                let (fu, fv) = if let Some(frame) = face.uv_frame {
                    let uv = frame.apply(kernel::Point3::new(x, y, z));
                    (uv[0] as f32, uv[1] as f32)
                } else {
                    ((u2d / bws[0]) as f32, (v2d / bws[1]) as f32)
                };
                mesh.uvs.extend([fu, fv]);
            }
            face_data.push(FaceData {
                material: Some(back),
                back: true,
                base: back_base,
                triangles: triangles.iter().map(|&[i, j, k]| [i, k, j]).collect(),
            });
        }

        face_data.push(FaceData {
            material: material_id,
            back: false,
            base,
            triangles,
        });
    }

    // Second pass: bucket each face's triangles by material so a material's
    // indices are contiguous, then emit one group per bucket. Buckets are kept
    // in first-seen order — distinct materials per object are few, so a
    // linear-scan bucket list is ample and needs no ordering on `MaterialId`.
    // Painted backs bucket apart from fronts, after them.
    let mut buckets: Vec<(Option<MaterialId>, bool, Vec<u32>)> = Vec::new();
    for fd in face_data
        .iter()
        .filter(|fd| !fd.back)
        .chain(face_data.iter().filter(|fd| fd.back))
    {
        let bi = match buckets
            .iter()
            .position(|(m, back, _)| *m == fd.material && *back == fd.back)
        {
            Some(i) => i,
            None => {
                buckets.push((fd.material, fd.back, Vec::new()));
                buckets.len() - 1
            }
        };
        let run = &mut buckets[bi].2;
        for [i, j, k] in &fd.triangles {
            run.extend([
                fd.base + *i as u32,
                fd.base + *j as u32,
                fd.base + *k as u32,
            ]);
        }
    }
    for (material, back, run) in buckets {
        let start = mesh.indices.len() as u32;
        let count = run.len() as u32;
        mesh.indices.extend(run);
        mesh.groups.push(MaterialGroup {
            material,
            back,
            start,
            count,
        });
    }

    // Edge lines are independent of material grouping. One kernel
    // predicate (`Object::edge_smoothness`, true-curves.md §3.3 rule 3)
    // routes each edge: a real crease to the hard buffer, a facet seam of
    // one smooth surface to the suppressed soft buffer, and a joint between
    // two tangent surfaces (a fillet rail) to the tangent buffer.
    for (eid, edge) in object.edges() {
        if only.is_some() {
            break;
        }
        let he = &object.half_edges()[edge.half_edge];
        let from = object.vertices()[he.origin].position;
        let to = object.vertices()[object.half_edges()[he.next].origin].position;
        let buffer = match object.edge_smoothness(eid) {
            kernel::EdgeSmoothness::Hard => &mut mesh.edge_positions,
            kernel::EdgeSmoothness::Seam => &mut mesh.soft_edge_positions,
            kernel::EdgeSmoothness::Tangent => &mut mesh.tangent_edge_positions,
        };
        buffer.extend([
            from.x as f32,
            from.y as f32,
            from.z as f32,
            to.x as f32,
            to.y as f32,
            to.z as f32,
        ]);
    }

    Ok(mesh)
}

/// True if triangle `tri` of `poly` still winds with `normal` once its
/// corners are rounded to the `f32` the render buffers hold.
///
/// Ear clipping emits every triangle with a strictly positive f64 area, but
/// the fan it builds from one vertex across a hole's chain of chords ends
/// at that vertex's tangent to the hole, and the last fan triangle before
/// the tangent is as thin as the tangent point happens to fall close to a
/// chain vertex — a zero-to-chord-length lottery that regularly lands under
/// the f32 quantum of the coordinates (about 2e-7 m at 2 m from the
/// origin). The ring forces that triangle: by the time it is clipped, the
/// fan apex is the only ring vertex visible from the chord, so no other
/// ear choice and no edge flip can replace it. Written to the buffers it
/// would wind backwards or collapse: a back-facing triangle under the
/// renderer's culling, with a face normal disagreeing with its vertices.
/// Its footprint is below what the buffers can represent — after rounding,
/// its two neighbours already cover it between them — so it is left out
/// rather than written inverted. The gate is the sign of the output's own
/// winding, not a distance tolerance: a triangle with any representable
/// altitude keeps a positive area under rounding, so only sub-quantum
/// slivers are affected and the face's coverage is unchanged.
fn tri_winds_forward_in_f32(poly: &[[f64; 5]], normal: kernel::Vec3, tri: [usize; 3]) -> bool {
    let at = |i: usize| kernel::Point3::new(poly[i][0], poly[i][1], poly[i][2]);
    winds_forward_in_f32(at(tri[0]), at(tri[1]), at(tri[2]), normal)
}

/// True if the triangle `a b c` still winds with `normal` once each corner
/// is rounded to `f32` — the gate the render buffers apply to every
/// triangle (see `tri_winds_forward_in_f32`, which says why a sliver can
/// fail it), shared with the mesh exporters, which store `f32` too and
/// apply it at their own scale: pass the corners in the coordinates the
/// writer rounds (millimeters for STL, meters for glTF and USDZ). `normal`
/// need not be unit; only the sign of the rounded winding against it
/// counts, and a collapsed triangle (zero rounded area) fails.
pub fn winds_forward_in_f32(
    a: kernel::Point3,
    b: kernel::Point3,
    c: kernel::Point3,
    normal: kernel::Vec3,
) -> bool {
    let rounded = |p: kernel::Point3| {
        kernel::Point3::new(
            f64::from(p.x as f32),
            f64::from(p.y as f32),
            f64::from(p.z as f32),
        )
    };
    let (a, b, c) = (rounded(a), rounded(b), rounded(c));
    (b - a).cross(c - a).dot(normal) > 0.0
}

/// Tessellates an Object's Smooth surface at `level` (1..=3): the
/// Catmull–Clark subdivision of the object as a cage
/// ([`kernel::subdiv::subdivide`]), smooth-shaded with normals that split
/// across creases. Each subdivided face takes its source cage face's
/// material and UV mapping (the point projected onto the source face's
/// plane); `face_ranges` holds one range per cage face covering all its
/// child vertices. Crease edges go to `edge_positions`; every cage edge to
/// `cage_edge_positions` for the wire overlay. Painted backs are not drawn
/// on a smoothed surface.
pub fn tessellate_smoothed(
    object: &Object,
    palette: &MaterialPalette,
    level: u8,
) -> Result<RenderMesh, TessellateError> {
    tessellate_smoothed_with(object, palette, level, None)
}

/// [`tessellate_smoothed`] for an Object drawn through a placement whose
/// material is `inherited`: a cage face resolves to its own material, else
/// the Object's base, else `inherited`, as [`tessellate_with`] does.
pub fn tessellate_smoothed_with(
    object: &Object,
    palette: &MaterialPalette,
    level: u8,
    inherited: Option<MaterialId>,
) -> Result<RenderMesh, TessellateError> {
    let m = kernel::subdiv::subdivide(object, level).map_err(TessellateError::Subdivide)?;
    let normals = kernel::subdiv::corner_normals(&m);
    let mut by_source: std::collections::BTreeMap<FaceId, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, &src) in m.source.iter().enumerate() {
        by_source.entry(src).or_default().push(i);
    }
    let mut mesh = RenderMesh::default();
    let mut buckets: Vec<(Option<MaterialId>, Vec<u32>)> = Vec::new();
    for (face_id, face) in object.faces() {
        let Some(children) = by_source.get(&face_id) else {
            continue;
        };
        let normal = face.plane.normal();
        let (u_ax, v_ax) = kernel::triangulate::plane_basis(normal);
        let material_id = face.material.or(object.default_material()).or(inherited);
        let (color, world_size) = match material_id.and_then(|id| palette.get(id)) {
            Some(mat) => (
                mat.color,
                mat.texture
                    .as_ref()
                    .map(|t| t.world_size)
                    .unwrap_or([1.0, 1.0]),
            ),
            None => (DEFAULT_MATERIAL_RGBA, [1.0, 1.0]),
        };
        let rgb = [
            color.r as f32 / 255.0,
            color.g as f32 / 255.0,
            color.b as f32 / 255.0,
        ];
        let base = (mesh.positions.len() / 3) as u32;
        let bi = match buckets.iter().position(|(mm, _)| *mm == material_id) {
            Some(i) => i,
            None => {
                buckets.push((material_id, Vec::new()));
                buckets.len() - 1
            }
        };
        for &c in children {
            let f = &m.faces[c];
            let first = (mesh.positions.len() / 3) as u32;
            for (k, &pi) in f.iter().enumerate() {
                let p = m.points[pi as usize];
                mesh.positions.extend([p.x as f32, p.y as f32, p.z as f32]);
                let n = normals[c][k];
                mesh.normals.extend([n.x as f32, n.y as f32, n.z as f32]);
                mesh.colors.extend(rgb);
                // The point projected onto the source face's plane, mapped
                // as the flat tessellation maps a point of that face.
                let on = p - normal * face.plane.signed_distance(p);
                let (fu, fv) = match face.uv_frame {
                    Some(frame) => {
                        let uv = frame.apply(on);
                        (uv[0] as f32, uv[1] as f32)
                    }
                    None => {
                        let v = on.to_vec();
                        (
                            (v.dot(u_ax) / world_size[0]) as f32,
                            (v.dot(v_ax) / world_size[1]) as f32,
                        )
                    }
                };
                mesh.uvs.extend([fu, fv]);
            }
            let run = &mut buckets[bi].1;
            for k in 1..f.len() as u32 - 1 {
                run.extend([first, first + k, first + k + 1]);
            }
        }
        let count = (mesh.positions.len() / 3) as u32 - base;
        mesh.face_ranges.push((face_id, base, count));
    }
    for (material, run) in buckets {
        let start = mesh.indices.len() as u32;
        let count = run.len() as u32;
        mesh.indices.extend(run);
        mesh.groups.push(MaterialGroup {
            material,
            back: false,
            start,
            count,
        });
    }
    if mesh.groups.is_empty() {
        mesh.groups.push(MaterialGroup {
            material: None,
            back: false,
            start: 0,
            count: 0,
        });
    }
    for &(a, b) in &m.creases {
        let (p, q) = (m.points[a as usize], m.points[b as usize]);
        mesh.edge_positions
            .extend([p.x, p.y, p.z, q.x, q.y, q.z].map(|c| c as f32));
    }
    for e in object.edges().keys() {
        if let Some((p, q)) = object.edge_endpoints(e) {
            mesh.cage_edge_positions
                .extend([p.x, p.y, p.z, q.x, q.y, q.z].map(|c| c as f32));
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{MaterialPalette, Object, Point3, Profile};

    /// f32 round-off allowance for unit-length checks in tests only; kernel
    /// geometric tolerances live in `kernel::tol`.
    const UNIT_LENGTH_TOL_F32: f32 = 1e-6;

    /// Test epsilon for area comparisons — generous enough for f64 arithmetic
    /// on geometry in the [0, ~50] range the fixtures use.
    const AREA_TOL: f64 = 1e-9;

    #[test]
    fn smoothed_mesh_matches_subdivide_and_has_cage_wire() {
        let cube = unit_cube();
        let palette = MaterialPalette::default();
        let mesh = tessellate_smoothed(&cube, &palette, 2).unwrap();
        let sub = kernel::subdiv::subdivide(&cube, 2).unwrap();
        // One vertex per child-face corner, two triangles per quad.
        let corners: usize = sub.faces.iter().map(Vec::len).sum();
        assert_eq!(mesh.positions.len(), corners * 3);
        assert_eq!(mesh.indices.len(), sub.faces.len() * 6);
        for p in mesh.positions.as_chunks::<3>().0 {
            let q = sub
                .points
                .iter()
                .any(|s| (s.x as f32, s.y as f32, s.z as f32) == (p[0], p[1], p[2]));
            assert!(q, "every vertex is a subdivision point");
        }
        // One range per cage face, covering all its children.
        assert_eq!(mesh.face_ranges.len(), 6);
        let covered: u32 = mesh.face_ranges.iter().map(|r| r.2).sum();
        assert_eq!(covered as usize, corners);
        // The wire overlay is the cage; no crease, no hard lines.
        assert_eq!(mesh.cage_edge_positions.len(), 12 * 6);
        assert!(mesh.edge_positions.is_empty());
        // Smooth shading: normals are unit and not all axis-aligned.
        assert!(
            mesh.normals
                .as_chunks::<3>()
                .0
                .iter()
                .any(|n| n[0].abs() > 0.1 && n[1].abs() > 0.1)
        );
        // The flat path keeps the overlay empty.
        assert!(
            tessellate(&cube, &palette)
                .unwrap()
                .cage_edge_positions
                .is_empty()
        );
    }

    #[test]
    fn tessellate_faces_matches_the_full_mesh_for_the_listed_faces() {
        let cube = unit_cube();
        let palette = MaterialPalette::default();
        let full = tessellate(&cube, &palette).unwrap();
        let two: Vec<FaceId> = cube.faces().keys().take(2).collect();
        let part = tessellate_faces(&cube, &two, &palette).unwrap();
        assert_eq!(part.face_ranges.len(), 2);
        assert_eq!(part.indices.len(), 2 * 6);
        assert!(part.edge_positions.is_empty());
        for (f, base, count) in &part.face_ranges {
            let (_, fbase, fcount) = full.face_ranges.iter().find(|r| r.0 == *f).unwrap();
            assert_eq!(count, fcount);
            let a = &part.positions[*base as usize * 3..(*base + *count) as usize * 3];
            let b = &full.positions[*fbase as usize * 3..(*fbase + *fcount) as usize * 3];
            assert_eq!(a, b);
        }
        assert!(
            tessellate_faces(&cube, &[], &palette)
                .unwrap()
                .indices
                .is_empty()
        );
    }

    fn unit_cube() -> Object {
        Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_rounded_edges_strips_share_one_normal_per_position_and_the_walls_stay_flat() {
        check_blended_top_front(
            kernel::BlendProfile::Round {
                radius: 0.2,
                segments: 6,
            },
            true,
        );
    }

    /// A variable-radius strip is a swept surface with no analytic claim, so
    /// only the blend shading gives its rails the walls' normals. Fails if
    /// the blend shading path is off.
    #[test]
    fn a_variable_radius_strips_rails_take_the_walls_normals() {
        let cube = unit_cube();
        let top_front = cube
            .edges()
            .keys()
            .find(|&e| {
                let (p, q) = cube.edge_endpoints(e).unwrap();
                p.y == 0.0 && q.y == 0.0 && p.z == 1.0 && q.z == 1.0
            })
            .unwrap();
        let (rounded, report) = cube
            .blend_edges(&kernel::BlendSpec {
                edges: vec![top_front],
                profile: kernel::BlendProfile::Round {
                    radius: 0.1,
                    segments: 6,
                },
                primary: None,
                corners: kernel::CornerStyle::Default,
                stops: None,
                handles: vec![
                    kernel::RadiusHandle {
                        at: 0.0,
                        radius: 0.1,
                    },
                    kernel::RadiusHandle {
                        at: 0.5,
                        radius: 0.3,
                    },
                ],
            })
            .unwrap();
        let mesh = tessellate(&rounded, &MaterialPalette::default()).unwrap();
        let blend: std::collections::BTreeSet<_> = report.new_faces.iter().copied().collect();
        let (mut on_front, mut on_top) = (0, 0);
        for &(face, base, count) in &mesh.face_ranges {
            if !blend.contains(&face) {
                continue;
            }
            for v in base..base + count {
                let i = v as usize * 3;
                let (y, z) = (mesh.positions[i + 1], mesh.positions[i + 2]);
                let n = [mesh.normals[i], mesh.normals[i + 1], mesh.normals[i + 2]];
                if y.abs() < 1e-6 && z < 1.0 - 1e-6 {
                    assert!(
                        (n[1] + 1.0).abs() < 1e-3 && n[2].abs() < 1e-3,
                        "front rail {n:?}"
                    );
                    on_front += 1;
                } else if (z - 1.0).abs() < 1e-6 && y > 1e-6 {
                    assert!(
                        (n[2] - 1.0).abs() < 1e-3 && n[1].abs() < 1e-3,
                        "top rail {n:?}"
                    );
                    on_top += 1;
                }
            }
        }
        assert!(on_front > 0 && on_top > 0);
    }

    fn check_blended_top_front(profile: kernel::BlendProfile, expect_inside: bool) {
        let cube = unit_cube();
        let top_front = cube
            .edges()
            .keys()
            .find(|&e| {
                let (p, q) = cube.edge_endpoints(e).unwrap();
                p.y == 0.0 && q.y == 0.0 && p.z == 1.0 && q.z == 1.0
            })
            .unwrap();
        let (rounded, report) = cube
            .blend_edges(&kernel::BlendSpec {
                edges: vec![top_front],
                profile,
                primary: None,
                corners: kernel::CornerStyle::Default,
                stops: None,
                handles: Vec::new(),
            })
            .unwrap();
        let mesh = tessellate(&rounded, &MaterialPalette::default()).unwrap();
        let strips: std::collections::BTreeSet<_> = report.new_faces.iter().copied().collect();
        let mut by_pos: std::collections::BTreeMap<[u32; 3], Vec<[f32; 3]>> = Default::default();
        for &(face, base, count) in &mesh.face_ranges {
            for v in base..base + count {
                let i = v as usize * 3;
                let p = &mesh.positions[i..i + 3];
                let n = [mesh.normals[i], mesh.normals[i + 1], mesh.normals[i + 2]];
                if strips.contains(&face) {
                    by_pos
                        .entry([p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
                        .or_default()
                        .push(n);
                } else {
                    // The box's own faces are untouched: axis-aligned.
                    assert!(n.iter().filter(|c| c.abs() > 0.999).count() == 1, "{n:?}");
                }
            }
        }
        assert!(!by_pos.is_empty());
        let (mut on_front, mut on_top, mut inside) = (0, 0, 0);
        for (p, ns) in by_pos {
            for n in &ns {
                assert!(
                    (0..3).all(|k| (n[k] - ns[0][k]).abs() < 1e-5),
                    "{p:?}: {ns:?}"
                );
            }
            let (y, z) = (f32::from_bits(p[1]), f32::from_bits(p[2]));
            let n = ns[0];
            assert!(n[0].abs() < 1e-5, "{p:?}: {n:?}");
            if y.abs() < 1e-6 && z < 1.0 - 1e-6 {
                // The rail on the front wall carries the wall's normal.
                assert!(
                    (n[1] + 1.0).abs() < 1e-4 && n[2].abs() < 1e-4,
                    "front rail {n:?}"
                );
                on_front += 1;
            } else if (z - 1.0).abs() < 1e-6 && y > 1e-6 {
                // The rail on the top carries the top's normal.
                assert!(
                    (n[2] - 1.0).abs() < 1e-4 && n[1].abs() < 1e-4,
                    "top rail {n:?}"
                );
                on_top += 1;
            } else {
                // A joint inside the fillet shades between the two: it is
                // neither wall's normal, so a flat per-facet normal fails.
                assert!(n[1] < -1e-3 && n[2] > 1e-3, "interior joint {n:?}");
                inside += 1;
            }
        }
        assert!(on_front > 0 && on_top > 0);
        assert_eq!(inside > 0, expect_inside);
    }

    fn xy_plane() -> kernel::Plane {
        kernel::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
    }

    /// Signed area of 2D triangle (a, b, c). Positive = CCW.
    fn signed_area_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
        0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
    }

    /// Project a `Point3` onto the XY plane (z=0 → u=x, v=y).
    fn xy_proj(p: Point3) -> [f64; 2] {
        [p.x, p.y]
    }

    /// True if point (px, py) is strictly inside the polygon given by (x,y) pairs
    /// using ray casting.
    fn point_in_poly_2d(px: f64, py: f64, pts: &[[f64; 2]]) -> bool {
        let n = pts.len();
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let xi = pts[i][0];
            let yi = pts[i][1];
            let xj = pts[j][0];
            let yj = pts[j][1];
            if (yi > py) != (yj > py) {
                let cross_x = xj + (py - yj) * (xi - xj) / (yi - yj);
                if px < cross_x {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    #[test]
    fn tetrahedron_buffers_have_expected_shape() {
        let mesh = tessellate(&Object::tetrahedron(), &MaterialPalette::default()).unwrap();
        // 4 triangular faces, vertices duplicated per face.
        assert_eq!(mesh.positions.len(), 4 * 3 * 3);
        assert_eq!(mesh.normals.len(), mesh.positions.len());
        // One triangle per face.
        assert_eq!(mesh.indices.len(), 4 * 3);
        // 6 unique edges, two xyz endpoints each.
        assert_eq!(mesh.edge_positions.len(), 6 * 2 * 3);
    }

    #[test]
    fn quad_faces_fan_into_two_triangles() {
        let mesh = tessellate(&unit_cube(), &MaterialPalette::default()).unwrap();
        // 6 quads -> 24 duplicated corners, 12 triangles, 12 unique edges.
        assert_eq!(mesh.positions.len(), 24 * 3);
        assert_eq!(mesh.indices.len(), 12 * 3);
        assert_eq!(mesh.edge_positions.len(), 12 * 2 * 3);
    }

    #[test]
    fn normals_are_unit_length() {
        let mesh = tessellate(&Object::tetrahedron(), &MaterialPalette::default()).unwrap();
        for n in mesh.normals.as_chunks::<3>().0 {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!(
                (len - 1.0).abs() < UNIT_LENGTH_TOL_F32,
                "normal length {len}"
            );
        }
    }

    #[test]
    fn degenerate_face_gives_error() {
        // A single triangle face (open mesh) - we can directly check that a
        // polygon with fewer than 3 verts causes DegenerateFace. We test by
        // checking that a valid mesh with triangles does NOT return DegenerateFace.
        // The DegenerateFace path is only reachable if the kernel produces a
        // loop with <3 verts, which is prevented by the kernel validator; we
        // test the error variant exists and displays correctly.
        use kernel::FaceId;
        let dummy_face = FaceId::default();
        let err = TessellateError::DegenerateFace { face: dummy_face };
        let s = format!("{err}");
        assert!(s.contains("fewer than 3"));
    }

    #[test]
    fn non_convex_face_triangulates() {
        // L-shaped profile (6 vertices, non-convex), extruded to get a solid.
        // The L-shape: 2 wide at the bottom, 1 wide at the top.
        //
        //  (0,2) - (1,2)
        //    |       |
        //  (0,1) - (1,1) - (2,1)
        //                    |
        //          (0,0) - (2,0)
        //
        // CCW winding from +Z as required by Profile (area = 2×2 - 1×1 = 3):
        let l_outer = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ];
        // Shoelace check: should be positive (CCW).
        let pts2d: Vec<[f64; 2]> = l_outer.iter().map(|p| xy_proj(*p)).collect();
        let shoelace: f64 = {
            let n = pts2d.len();
            let mut sum = 0.0f64;
            for i in 0..n {
                let j = (i + 1) % n;
                sum += pts2d[i][0] * pts2d[j][1] - pts2d[j][0] * pts2d[i][1];
            }
            sum * 0.5
        };
        // Area of L = 2×1 (bottom) + 1×1 (top-left) = 2+1 = 3
        assert!(
            (shoelace - 3.0).abs() < AREA_TOL,
            "L-shape CCW shoelace area = {shoelace} (expected 3.0)"
        );

        let profile = Profile::new(xy_plane(), l_outer.clone(), vec![]).unwrap();
        let solid = Object::from_extrusion(&profile, 1.0).unwrap();
        let mesh = tessellate(&solid, &MaterialPalette::default()).unwrap();

        // Find the two cap faces (those with normal along ±Z) — they are the
        // L-shaped caps with 6 vertices each.
        // For each cap, collect its triangle vertices from mesh.positions and
        // check geometric correctness.

        // Instead: find all cap triangles by scanning index triples and checking
        // they share the cap normal. The total triangle count over the whole
        // solid should be correct.
        // Total: 2 L-caps (each 6-2=4 tris) + 6 side walls (each 2 tris) = 8+12=20
        // But easier: just check the sub-assertions for the L-cap below.

        // --- Directly test the ear-clip on the L-shape polygon ---
        let l_5d: Vec<[f64; 5]> = l_outer
            .iter()
            .map(|p| {
                let pu = p.x;
                let pv = p.y;
                [p.x, p.y, p.z, pu, pv]
            })
            .collect();
        let tris = ear_clip(&l_5d);

        // n-2 = 4 triangles for a 6-vertex polygon.
        assert_eq!(
            tris.len(),
            4,
            "L-shape: expected 4 triangles, got {}",
            tris.len()
        );

        // Sum of triangle areas should equal the polygon's shoelace area (3.0).
        let total_tri_area: f64 = tris
            .iter()
            .map(|&[i, j, k]| {
                let a = [l_5d[i][3], l_5d[i][4]];
                let b = [l_5d[j][3], l_5d[j][4]];
                let c = [l_5d[k][3], l_5d[k][4]];
                signed_area_2d(a, b, c)
            })
            .sum();
        assert!(
            (total_tri_area - shoelace).abs() < AREA_TOL,
            "L-shape: triangle area sum {total_tri_area} ≠ polygon area {shoelace}"
        );

        // Every triangle must have positive (CCW) area.
        for &[i, j, k] in &tris {
            let a = [l_5d[i][3], l_5d[i][4]];
            let b = [l_5d[j][3], l_5d[j][4]];
            let c = [l_5d[k][3], l_5d[k][4]];
            let area = signed_area_2d(a, b, c);
            assert!(
                area > 0.0,
                "L-shape: triangle [{i},{j},{k}] has non-positive area {area}"
            );
        }

        // Tessellate must succeed (no error).
        assert!(!mesh.indices.is_empty());
    }

    #[test]
    fn washer_face_triangulates() {
        // 4x4 outer square with a centered 2x2 hole, extruded.
        let profile = Profile::new(
            xy_plane(),
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(4.0, 0.0, 0.0),
                Point3::new(4.0, 4.0, 0.0),
                Point3::new(0.0, 4.0, 0.0),
            ],
            vec![vec![
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(1.0, 3.0, 0.0),
                Point3::new(3.0, 3.0, 0.0),
                Point3::new(3.0, 1.0, 0.0),
            ]],
        )
        .unwrap();

        let solid = Object::from_extrusion(&profile, 2.0).unwrap();

        // Must tessellate without error.
        let mesh = tessellate(&solid, &MaterialPalette::default()).unwrap();
        assert!(
            !mesh.indices.is_empty(),
            "washer tessellation produced no triangles"
        );

        // For each cap face (faces with ±Z normal), verify area and hole containment.
        let outer_area = 4.0 * 4.0; // 16.0
        let hole_area = 2.0 * 2.0; // 4.0
        let expected_cap_area = outer_area - hole_area; // 12.0

        // Find cap faces: those whose plane normal is ≈ ±(0,0,1).
        let mut cap_face_count = 0;
        for (_face_id, face) in solid.faces() {
            let n = face.plane.normal();
            if (n.z.abs() - 1.0).abs() > 0.01 {
                continue; // not a cap face
            }
            cap_face_count += 1;

            // This face has inner loops (holes), so build its polygon via
            // bridge_holes and check its triangulation.
            // Use the face's own plane normal to build the 2D basis (the
            // near cap has normal=-Z so its basis differs from the far cap).
            let cap_normal = face.plane.normal();
            let (u_ax, v_ax) = plane_basis(cap_normal);

            let outer_3d: Vec<[f64; 3]> = solid
                .loop_positions(face.outer_loop)
                .map(|p| [p.x, p.y, p.z])
                .collect();
            let holes_3d: Vec<Vec<[f64; 3]>> = face
                .inner_loops
                .iter()
                .map(|&lid| solid.loop_positions(lid).map(|p| [p.x, p.y, p.z]).collect())
                .collect();

            // Build the hole polygon projected into this cap's 2D basis for
            // centroid-in-hole testing.
            let hole_2d_proj: Vec<[f64; 2]> = holes_3d
                .first()
                .map(|h| {
                    h.iter()
                        .map(|&p| {
                            let pu = p[0] * u_ax.x + p[1] * u_ax.y + p[2] * u_ax.z;
                            let pv = p[0] * v_ax.x + p[1] * v_ax.y + p[2] * v_ax.z;
                            [pu, pv]
                        })
                        .collect()
                })
                .unwrap_or_default();

            let poly = bridge_holes(&outer_3d, &holes_3d, u_ax, v_ax)
                .expect("washer cap must be bridgeable");
            let tris = ear_clip(&poly);

            // Sum of triangle areas == expected_cap_area within AREA_TOL.
            let total_area: f64 = tris
                .iter()
                .map(|&[i, j, k]| {
                    let a = [poly[i][3], poly[i][4]];
                    let b = [poly[j][3], poly[j][4]];
                    let c = [poly[k][3], poly[k][4]];
                    signed_area_2d(a, b, c)
                })
                .sum();
            assert!(
                (total_area - expected_cap_area).abs() < AREA_TOL,
                "washer cap: triangle area sum {total_area} ≠ expected {expected_cap_area}"
            );

            // No triangle's centroid lies inside the hole polygon (in 2D basis).
            if !hole_2d_proj.is_empty() {
                for &[i, j, k] in &tris {
                    let ax = poly[i][3];
                    let ay = poly[i][4];
                    let bx = poly[j][3];
                    let by = poly[j][4];
                    let cx = poly[k][3];
                    let cy = poly[k][4];
                    let centroid_u = (ax + bx + cx) / 3.0;
                    let centroid_v = (ay + by + cy) / 3.0;
                    assert!(
                        !point_in_poly_2d(centroid_u, centroid_v, &hole_2d_proj),
                        "washer cap: triangle [{i},{j},{k}] centroid ({centroid_u},{centroid_v}) is inside the hole"
                    );
                }
            }
        }

        assert_eq!(
            cap_face_count, 2,
            "expected 2 cap faces, found {cap_face_count}"
        );
    }

    /// A painted face yields its material color in `colors`, and the index
    /// buffer is grouped so the painted face's triangles are contiguous in one
    /// group while the unpainted faces are in another.
    #[test]
    fn painted_face_color_and_grouped_indices() {
        use kernel::{Document, Material, Rgba8, SketchRegionId};

        // Build a unit-square extrusion via the document so we can paint a face.
        let mut doc = Document::new();
        let plane = kernel::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let sk = doc.add_sketch(plane);
        let s = doc.sketch_mut(sk).unwrap();
        for (ax, ay, bx, by) in [
            (0.0f64, 0.0, 1.0, 0.0),
            (1.0, 0.0, 1.0, 1.0),
            (1.0, 1.0, 0.0, 1.0),
            (0.0, 1.0, 0.0, 0.0),
        ] {
            s.add_segment(Point3::new(ax, ay, 0.0), Point3::new(bx, by, 0.0))
                .unwrap();
        }
        let regions = doc.extrudable_regions(sk).unwrap();
        let region_id: SketchRegionId = regions[0];
        let (obj_id, _) = doc.extrude_region(sk, region_id, 1.0).unwrap();

        // Add a red material and paint the top face (normal ≈ +Z).
        let red_id = doc.add_material(Material::solid("red", Rgba8::rgb(255, 0, 0)));

        let top_face_id = {
            doc.object(obj_id)
                .unwrap()
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid)
                .unwrap()
        };
        doc.paint_face(obj_id, top_face_id, Some(red_id)).unwrap();

        let object = doc.object(obj_id).unwrap();
        let palette = doc.materials();
        let mesh = tessellate(object, palette).unwrap();

        // There must be colors and uvs parallel to positions.
        let n_vertices = mesh.positions.len() / 3;
        assert_eq!(
            mesh.colors.len(),
            n_vertices * 3,
            "colors not parallel to positions"
        );
        assert_eq!(
            mesh.uvs.len(),
            n_vertices * 2,
            "uvs not parallel to positions"
        );

        // There must be exactly 2 groups: one for the red material, one for None.
        assert_eq!(mesh.groups.len(), 2, "expected 2 material groups");
        let red_group = mesh.groups.iter().find(|g| g.material == Some(red_id));
        let default_group = mesh.groups.iter().find(|g| g.material.is_none());
        assert!(red_group.is_some(), "no group for red material");
        assert!(default_group.is_some(), "no group for default material");

        // The red-material vertices must have color (1,0,0).
        let rg = red_group.unwrap();
        assert!(rg.count > 0, "red group has no indices");
        for idx_pos in (rg.start as usize..(rg.start + rg.count) as usize).step_by(1) {
            let vi = mesh.indices[idx_pos] as usize;
            let r = mesh.colors[vi * 3];
            let g_val = mesh.colors[vi * 3 + 1];
            let b = mesh.colors[vi * 3 + 2];
            assert!(
                (r - 1.0).abs() < 1e-5 && g_val < 1e-5 && b < 1e-5,
                "red-group vertex color is ({r},{g_val},{b}), expected (1,0,0)"
            );
        }

        // The index buffer covers all triangles exactly once.
        let total_indices: u32 = mesh.groups.iter().map(|g| g.count).sum();
        assert_eq!(
            total_indices as usize,
            mesh.indices.len(),
            "group counts don't sum to index buffer length"
        );
    }

    /// An object base material colors every *unpainted* face (the effective
    /// material falls back to the base), so the whole solid is one group/color.
    #[test]
    fn object_base_material_colors_all_unpainted_faces() {
        use kernel::{Document, Material, Rgba8};

        let mut doc = Document::new();
        let plane = kernel::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let sk = doc.add_sketch(plane);
        let s = doc.sketch_mut(sk).unwrap();
        for (ax, ay, bx, by) in [
            (0.0f64, 0.0, 1.0, 0.0),
            (1.0, 0.0, 1.0, 1.0),
            (1.0, 1.0, 0.0, 1.0),
            (0.0, 1.0, 0.0, 0.0),
        ] {
            s.add_segment(Point3::new(ax, ay, 0.0), Point3::new(bx, by, 0.0))
                .unwrap();
        }
        let region = doc.extrudable_regions(sk).unwrap()[0];
        let (obj_id, _) = doc.extrude_region(sk, region, 1.0).unwrap();

        // Set the object base to red; no face is painted individually.
        let red = doc.add_material(Material::solid("red", Rgba8::rgb(255, 0, 0)));
        doc.set_object_material(obj_id, Some(red)).unwrap();

        let mesh = tessellate(doc.object(obj_id).unwrap(), doc.materials()).unwrap();

        // Every unpainted face resolves to the base → one group, all red.
        assert_eq!(mesh.groups.len(), 1, "all faces share the base material");
        assert_eq!(mesh.groups[0].material, Some(red));
        for c in mesh.colors.as_chunks::<3>().0 {
            assert!(
                (c[0] - 1.0).abs() < 1e-5 && c[1] < 1e-5 && c[2] < 1e-5,
                "vertex color {c:?} is not the base red"
            );
        }
    }

    /// An inherited (placement) material fills only what neither the face
    /// nor the Object's base paints.
    #[test]
    fn inherited_material_fills_unpainted_faces_only() {
        use kernel::{Document, Material, Rgba8};

        let mut doc = Document::new();
        let sk = doc.add_sketch(xy_plane());
        let s = doc.sketch_mut(sk).unwrap();
        for (ax, ay, bx, by) in [
            (0.0f64, 0.0, 1.0, 0.0),
            (1.0, 0.0, 1.0, 1.0),
            (1.0, 1.0, 0.0, 1.0),
            (0.0, 1.0, 0.0, 0.0),
        ] {
            s.add_segment(Point3::new(ax, ay, 0.0), Point3::new(bx, by, 0.0))
                .unwrap();
        }
        let region = doc.extrudable_regions(sk).unwrap()[0];
        let (obj_id, _) = doc.extrude_region(sk, region, 1.0).unwrap();
        let inherited = doc.add_material(Material::solid("inherited", Rgba8::rgb(0, 0, 255)));
        let own = doc.add_material(Material::solid("own", Rgba8::rgb(255, 0, 0)));
        let base = doc.add_material(Material::solid("base", Rgba8::rgb(0, 255, 0)));

        // Unpainted: everything lands in the inherited group.
        let mesh = tessellate_with(
            doc.object(obj_id).unwrap(),
            doc.materials(),
            Some(inherited),
        )
        .unwrap();
        assert_eq!(mesh.groups.len(), 1);
        assert_eq!(mesh.groups[0].material, Some(inherited));
        // `tessellate` is the no-inheritance case.
        let plain = tessellate(doc.object(obj_id).unwrap(), doc.materials()).unwrap();
        assert_eq!(plain.groups[0].material, None);

        // A face with its own paint keeps it.
        let face = doc.object(obj_id).unwrap().faces().keys().next().unwrap();
        doc.paint_face(obj_id, face, Some(own)).unwrap();
        let mesh = tessellate_with(
            doc.object(obj_id).unwrap(),
            doc.materials(),
            Some(inherited),
        )
        .unwrap();
        let mats: Vec<_> = mesh.groups.iter().map(|g| g.material).collect();
        assert!(mats.contains(&Some(own)) && mats.contains(&Some(inherited)));
        let painted = mesh.face_ranges.iter().find(|r| r.0 == face).unwrap();
        let c = &mesh.colors[painted.1 as usize * 3..painted.1 as usize * 3 + 3];
        assert!(
            (c[0] - 1.0).abs() < 1e-5 && c[2] < 1e-5,
            "own paint wins: {c:?}"
        );

        // An Object with a base never shows the inherited material.
        doc.set_object_material(obj_id, Some(base)).unwrap();
        let mesh = tessellate_with(
            doc.object(obj_id).unwrap(),
            doc.materials(),
            Some(inherited),
        )
        .unwrap();
        assert!(
            mesh.groups.iter().all(|g| g.material != Some(inherited)),
            "the base beats the inherited material"
        );
    }
}
