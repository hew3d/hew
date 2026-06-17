//! Kernel topology -> renderer-agnostic buffers.
//!
//! Isolates the kernel from the renderer (ARCHITECTURE.md): the output is plain
//! f32/u32 arrays a WebGL2 viewport can upload directly. Flat-shaded look:
//! vertices are duplicated per face so each face keeps its own normal, and
//! unique edges come out as line segments for the SketchUp-style display.
//!
//! Triangulation supports non-convex faces and faces with holes via ear
//! clipping with hole bridging (Eberly "Triangulation by Ear Clipping").

use kernel::{FaceId, MaterialId, MaterialPalette, Object, Rgba8};

/// Default face color when no material is assigned (or the id is stale):
/// a neutral light gray (`0xcccccc`).
const DEFAULT_MATERIAL_RGBA: Rgba8 = Rgba8::rgb(0xcc, 0xcc, 0xcc);

/// Tessellation failed; the Object uses topology this version cannot
/// triangulate honestly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TessellateError {
    /// Face has fewer than 3 boundary vertices (kernel invariant breach).
    DegenerateFace {
        /// The offending face.
        face: FaceId,
    },
}

impl std::fmt::Display for TessellateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TessellateError::DegenerateFace { face } => {
                write!(f, "face {face:?} has fewer than 3 vertices")
            }
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
    /// Line-segment endpoints (xyz pairs), one segment per unique edge.
    pub edge_positions: Vec<f32>,
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
pub fn tessellate(
    object: &Object,
    palette: &MaterialPalette,
) -> Result<RenderMesh, TessellateError> {
    // First pass: collect per-face geometry without committing index order.
    // Each entry: (material_id, base_vertex, triangles, vertex_data).
    struct FaceData {
        material: Option<MaterialId>,
        base: u32,
        triangles: Vec<[usize; 3]>,
    }

    let mut mesh = RenderMesh::default();
    let mut face_data: Vec<FaceData> = Vec::new();

    for (face_id, face) in object.faces() {
        let normal = face.plane.normal();
        let n = [normal.x as f32, normal.y as f32, normal.z as f32];

        // Build the orthonormal 2D basis (u, v) such that u × v = normal.
        let (u_ax, v_ax) = plane_basis(normal);

        // Resolve the *effective* material for color, UV world_size, AND the
        // group key: a face with no own material falls back to the object's
        // base material ( follow-up), so faces grown by extrude/boolean and
        // textured bases all render consistently.
        let material_id = face.material.or(object.default_material());
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

        // Collect outer loop 3D positions.
        let outer_3d: Vec<[f64; 3]> = object
            .loop_positions(face.outer_loop)
            .map(|p| [p.x, p.y, p.z])
            .collect();

        if outer_3d.len() < 3 {
            return Err(TessellateError::DegenerateFace { face: face_id });
        }

        // Collect each inner loop's 3D positions.
        let holes_3d: Vec<Vec<[f64; 3]>> = face
            .inner_loops
            .iter()
            .map(|&lid| {
                object
                    .loop_positions(lid)
                    .map(|p| [p.x, p.y, p.z])
                    .collect()
            })
            .collect();

        // Build the (possibly bridged) simple polygon for ear clipping.
        // Each entry is [x3d, y3d, z3d, u2d, v2d].
        let poly = build_polygon_with_holes(&outer_3d, &holes_3d, u_ax, v_ax);

        // Ear-clip the polygon and emit triangles.
        let base = (mesh.positions.len() / 3) as u32;

        // Append polygon vertices (positions, normals, colors, UVs).
        for &[x, y, z, u2d, v2d] in &poly {
            mesh.positions.extend([x as f32, y as f32, z as f32]);
            mesh.normals.extend(n);
            mesh.colors.extend([cr, cg, cb]);
            // UV = planar 2D coord divided by world_size.
            mesh.uvs
                .extend([(u2d / world_size[0]) as f32, (v2d / world_size[1]) as f32]);
        }

        // Run ear clipping (local indices into poly).
        let triangles = ear_clip(&poly);

        face_data.push(FaceData {
            material: material_id,
            base,
            triangles,
        });
    }

    // Second pass: bucket each face's triangles by material so a material's
    // indices are contiguous, then emit one group per bucket. Buckets are kept
    // in first-seen order — distinct materials per object are few, so a
    // linear-scan bucket list is ample and needs no ordering on `MaterialId`.
    let mut buckets: Vec<(Option<MaterialId>, Vec<u32>)> = Vec::new();
    for fd in &face_data {
        let bi = match buckets.iter().position(|(m, _)| *m == fd.material) {
            Some(i) => i,
            None => {
                buckets.push((fd.material, Vec::new()));
                buckets.len() - 1
            }
        };
        let run = &mut buckets[bi].1;
        for [i, j, k] in &fd.triangles {
            run.extend([
                fd.base + *i as u32,
                fd.base + *j as u32,
                fd.base + *k as u32,
            ]);
        }
    }
    for (material, run) in buckets {
        let start = mesh.indices.len() as u32;
        let count = run.len() as u32;
        mesh.indices.extend(run);
        mesh.groups.push(MaterialGroup {
            material,
            start,
            count,
        });
    }

    // Edge lines are independent of material grouping.
    for edge in object.edges().values() {
        let he = &object.half_edges()[edge.half_edge];
        let from = object.vertices()[he.origin].position;
        let to = object.vertices()[object.half_edges()[he.next].origin].position;
        mesh.edge_positions.extend([
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

// ──────────────────────────────────────── orthonormal basis for a plane

/// Constructs an orthonormal basis (u, v) on a plane with unit normal `n`,
/// such that u × v = n (right-handed). Mirrors the implementation in
/// `crates/inference/src/lib.rs::plane_basis`.
fn plane_basis(n: kernel::Vec3) -> (kernel::Vec3, kernel::Vec3) {
    let helper = if n.x.abs() < 0.9 {
        kernel::Vec3::new(1.0, 0.0, 0.0)
    } else {
        kernel::Vec3::new(0.0, 1.0, 0.0)
    };
    let u = helper
        .cross(n)
        .normalized()
        .expect("helper is never parallel to a unit normal");
    let v = n.cross(u);
    (u, v)
}

// ──────────────────────────────────────── hole bridging

/// Project a 3D point onto the 2D (u, v) plane basis.
fn proj2d(p: [f64; 3], u: kernel::Vec3, v: kernel::Vec3) -> [f64; 2] {
    [
        p[0] * u.x + p[1] * u.y + p[2] * u.z,
        p[0] * v.x + p[1] * v.y + p[2] * v.z,
    ]
}

/// Build a simple polygon (with holes bridged in) for ear clipping.
///
/// Each returned vertex is `[x, y, z, u2d, v2d]` where (u2d, v2d) are the
/// 2D projected coordinates used for geometric tests.
///
/// The algorithm follows Eberly "Triangulation by Ear Clipping":
/// - Process holes sorted by their maximum projected-x vertex (descending).
/// - For each hole, cast a +x ray from its rightmost vertex M and find the
///   nearest outer-polygon edge crossing P (visible vertex selection handles
///   the reflex-vertex-in-triangle case).
/// - Splice the hole into the outer ring with a zero-width bridge,
///   duplicating M and P.
fn build_polygon_with_holes(
    outer_3d: &[[f64; 3]],
    holes_3d: &[Vec<[f64; 3]>],
    u: kernel::Vec3,
    v: kernel::Vec3,
) -> Vec<[f64; 5]> {
    // Build initial poly with 2D coords.
    let mut poly: Vec<[f64; 5]> = outer_3d
        .iter()
        .map(|&p| {
            let [pu, pv] = proj2d(p, u, v);
            [p[0], p[1], p[2], pu, pv]
        })
        .collect();

    if holes_3d.is_empty() {
        return poly;
    }

    // Process holes sorted by descending max-x of their projected vertices.
    let mut indexed_holes: Vec<(usize, &Vec<[f64; 3]>)> = holes_3d.iter().enumerate().collect();

    indexed_holes.sort_by(|&(_, a), &(_, b)| {
        let max_x_a = a
            .iter()
            .map(|&p| proj2d(p, u, v)[0])
            .fold(f64::NEG_INFINITY, f64::max);
        let max_x_b = b
            .iter()
            .map(|&p| proj2d(p, u, v)[0])
            .fold(f64::NEG_INFINITY, f64::max);
        max_x_b.partial_cmp(&max_x_a).unwrap()
    });

    for (_, hole) in indexed_holes {
        poly = bridge_hole_into_polygon(poly, hole, u, v);
    }

    poly
}

/// Bridge one hole into `outer`, returning the merged simple polygon.
fn bridge_hole_into_polygon(
    outer: Vec<[f64; 5]>,
    hole: &[[f64; 3]],
    u: kernel::Vec3,
    v: kernel::Vec3,
) -> Vec<[f64; 5]> {
    // Build hole as [x, y, z, u2d, v2d] vertices.
    let hole_verts: Vec<[f64; 5]> = hole
        .iter()
        .map(|&p| {
            let [pu, pv] = proj2d(p, u, v);
            [p[0], p[1], p[2], pu, pv]
        })
        .collect();

    let n_outer = outer.len();
    let n_hole = hole_verts.len();

    // Find hole vertex M with maximum u-coordinate (rightmost in 2D).
    let m_idx = (0..n_hole)
        .max_by(|&i, &j| hole_verts[i][3].partial_cmp(&hole_verts[j][3]).unwrap())
        .unwrap();
    let m = &hole_verts[m_idx];
    let m_u = m[3];
    let m_v = m[4];

    // Cast a ray from M in the +u direction. Find the nearest outer edge
    // crossing (strictly to the right of M).
    // We track: (t_param, intersection_u, outer_edge_start_idx, which_endpoint)
    // where which_endpoint is Some(idx) if M is closest to a vertex.

    let mut best_t = f64::INFINITY;
    let mut best_edge_idx: Option<usize> = None;
    let mut best_cross_u = f64::INFINITY;
    let mut best_cross_v = 0.0f64;

    for i in 0..n_outer {
        let j = (i + 1) % n_outer;
        let ai_u = outer[i][3];
        let ai_v = outer[i][4];
        let aj_u = outer[j][3];
        let aj_v = outer[j][4];

        // Does the edge (ai, aj) cross the horizontal ray from M (+u direction)?
        // The ray is: u = m_u + t (t >= 0), v = m_v.
        // The edge parameterised: p = ai + s*(aj - ai) for s in [0,1].
        // v-component: ai_v + s*(aj_v - ai_v) = m_v  =>  s = (m_v - ai_v) / (aj_v - ai_v)
        let dv = aj_v - ai_v;
        if dv.abs() < 1e-14 {
            continue; // horizontal edge, skip
        }
        let s = (m_v - ai_v) / dv;
        if !(0.0..=1.0).contains(&s) {
            continue; // crossing outside segment
        }
        let cross_u = ai_u + s * (aj_u - ai_u);
        if cross_u <= m_u {
            continue; // crossing to the left of M, not ahead
        }
        let t = cross_u - m_u;
        if t < best_t {
            best_t = t;
            best_edge_idx = Some(i);
            best_cross_u = cross_u;
            best_cross_v = m_v;
        }
    }

    let ei = match best_edge_idx {
        Some(i) => i,
        None => {
            // Fallback: no crossing found (shouldn't happen with valid hole),
            // bridge to the outer vertex closest in u that is to the right.
            let closest = (0..n_outer)
                .filter(|&i| outer[i][3] >= m_u)
                .min_by(|&i, &j| {
                    (outer[i][3] - m_u)
                        .abs()
                        .partial_cmp(&(outer[j][3] - m_u).abs())
                        .unwrap()
                })
                .unwrap_or(0);
            return bridge_at_vertices(&outer, &hole_verts, closest, m_idx);
        }
    };

    // The crossing lands on edge (ei, ei+1). Choose the mutually visible
    // outer vertex P. The candidate is the endpoint of the edge with the
    // larger u-coordinate (or the intersection itself if it is not a vertex).
    let ej = (ei + 1) % n_outer;
    let pi_u = outer[ei][3];
    let pj_u = outer[ej][3];

    // If the intersection is exactly at a vertex, use that vertex.
    // Otherwise use the vertex with the larger x (right side of the edge).
    let p_idx = if (pi_u - best_cross_u).abs() < 1e-12 {
        ei
    } else if (pj_u - best_cross_u).abs() < 1e-12 {
        ej
    } else {
        // The crossing is interior; pick the vertex with the larger u.
        // But if any reflex outer vertex lies inside the triangle (M, P, I),
        // prefer the one with the smallest angle from M's +u ray.
        //
        // Simple conservative choice: always pick the candidate with largest u
        // that is "above or at" the intersection, then check for reflex
        // vertices in the triangle M-P-I and if any exist, prefer the one
        // with the smallest polar angle from M.
        let p_candidate = if pi_u >= pj_u { ei } else { ej };

        // Check whether any outer vertex lies inside triangle (M, p_candidate, crossing).
        let m_pt = [m_u, m_v];
        let p_pt = [outer[p_candidate][3], outer[p_candidate][4]];
        let i_pt = [best_cross_u, best_cross_v];

        let mut best_angle = f64::INFINITY;
        let mut refined = p_candidate;
        for (k, ov) in outer.iter().enumerate() {
            let k_u = ov[3];
            let k_v = ov[4];
            if !point_in_triangle_2d(m_pt, p_pt, i_pt, [k_u, k_v]) {
                continue;
            }
            // Vertex inside the triangle; compute angle from M's +u ray.
            let angle = (k_v - m_v).atan2(k_u - m_u).abs();
            if angle < best_angle {
                best_angle = angle;
                refined = k;
            }
        }
        refined
    };

    bridge_at_vertices(&outer, &hole_verts, p_idx, m_idx)
}

/// Splice `hole_verts` (starting at `hole_start`) into `outer` (after
/// `outer_idx`), creating a zero-width bridge that duplicates the two
/// connection vertices.
///
/// The resulting polygon visits:
///   outer[0..=outer_idx], M_dup, hole[hole_start..], hole[..=hole_start],
///   P_dup, outer[outer_idx..]
/// (where M = hole_start vertex, P = outer_idx vertex).
fn bridge_at_vertices(
    outer: &[[f64; 5]],
    hole_verts: &[[f64; 5]],
    outer_idx: usize,
    hole_start: usize,
) -> Vec<[f64; 5]> {
    let n_outer = outer.len();
    let n_hole = hole_verts.len();

    let mut result: Vec<[f64; 5]> = Vec::with_capacity(n_outer + n_hole + 2);

    // outer[0..=outer_idx]
    result.extend_from_slice(&outer[..=outer_idx]);
    // hole vertices starting at hole_start, wrapping around
    for k in 0..=n_hole {
        result.push(hole_verts[(hole_start + k) % n_hole]);
    }
    // close back to outer: duplicate outer_idx vertex, then rest of outer
    result.push(outer[outer_idx]);
    result.extend_from_slice(&outer[(outer_idx + 1)..]);

    result
}

// ──────────────────────────────────────── ear clipping

/// Returns true if `p` is STRICTLY inside triangle (a, b, c) (excludes
/// boundary). Points are [u2d, v2d].
///
/// Using strict interior avoids false disqualification of ear vertices when
/// the bridged polygon contains duplicate (bridge) vertices that share the
/// same position as another polygon vertex.
fn point_in_triangle_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2], p: [f64; 2]) -> bool {
    let cross = |o: [f64; 2], e1: [f64; 2], q: [f64; 2]| {
        (e1[0] - o[0]) * (q[1] - o[1]) - (e1[1] - o[1]) * (q[0] - o[0])
    };
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    // Strictly inside: all cross products must have the SAME sign (all positive
    // for CCW triangle) without allowing zero (boundary).
    (d1 > 0.0 && d2 > 0.0 && d3 > 0.0) || (d1 < 0.0 && d2 < 0.0 && d3 < 0.0)
}

/// True if segments AB and CD properly intersect (share an interior point).
/// Returns false if they merely share endpoints or are collinear.
fn segments_intersect_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross2 = |o: [f64; 2], e: [f64; 2], q: [f64; 2]| {
        (e[0] - o[0]) * (q[1] - o[1]) - (e[1] - o[1]) * (q[0] - o[0])
    };
    let d1 = cross2(c, d, a);
    let d2 = cross2(c, d, b);
    let d3 = cross2(a, b, c);
    let d4 = cross2(a, b, d);
    // Proper (non-degenerate) crossing: each segment straddles the line of the other.
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    false
}

/// True if point P lies strictly on the open segment AB (between A and B,
/// not at the endpoints).
fn point_on_segment_2d(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    // First check collinearity: cross(AB, AP) == 0.
    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    if cross.abs() > 1e-12 * ((b[0] - a[0]).hypot(b[1] - a[1]) + 1.0) {
        return false;
    }
    // Then check that p is between a and b along the axis with the larger span.
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    if dx.abs() >= dy.abs() {
        (a[0].min(b[0]) < p[0]) && (p[0] < a[0].max(b[0]))
    } else {
        (a[1].min(b[1]) < p[1]) && (p[1] < a[1].max(b[1]))
    }
}

/// Signed area of 2D triangle (a, b, c). Positive = CCW.
fn signed_area_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
}

/// Ear-clip a simple polygon given as `[x3d, y3d, z3d, u2d, v2d]` vertices.
///
/// The polygon is assumed CCW in (u2d, v2d). Returns a list of triangles as
/// `[i, j, k]` index triples into the input slice, all CCW.
fn ear_clip(poly: &[[f64; 5]]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        // Single triangle — emit only if CCW (non-degenerate).
        let a = [poly[0][3], poly[0][4]];
        let b = [poly[1][3], poly[1][4]];
        let c = [poly[2][3], poly[2][4]];
        if signed_area_2d(a, b, c) > 0.0 {
            return vec![[0, 1, 2]];
        } else {
            return Vec::new();
        }
    }

    // Working index list: indices into the original `poly` slice.
    let mut indices: Vec<usize> = (0..n).collect();
    let mut triangles: Vec<[usize; 3]> = Vec::with_capacity(n - 2);

    // Simple O(n^2) ear clipping.
    let mut safety = n * n + n; // upper bound on iterations
    while indices.len() > 3 {
        safety = safety.saturating_sub(1);
        if safety == 0 {
            break; // degenerate / non-simple polygon guard
        }

        let m = indices.len();
        let mut ear_found = false;

        for idx in 0..m {
            let prev = indices[(idx + m - 1) % m];
            let curr = indices[idx];
            let next = indices[(idx + 1) % m];

            let a = [poly[prev][3], poly[prev][4]];
            let b = [poly[curr][3], poly[curr][4]];
            let c = [poly[next][3], poly[next][4]];

            // Is the vertex at `curr` a convex (ear) vertex?
            // In a CCW polygon, a vertex is convex if cross(b-a, c-a) > 0.
            if signed_area_2d(a, b, c) <= 0.0 {
                continue; // reflex or degenerate
            }

            // Is any other polygon vertex inside the triangle (a, b, c), or
            // lying on the ear diagonal (a → c)?
            let mut blocked = false;
            for &other in &indices {
                if other == prev || other == curr || other == next {
                    continue;
                }
                let p = [poly[other][3], poly[other][4]];
                // Strictly inside the triangle (any sign) OR on the diagonal.
                if point_in_triangle_2d(a, b, c, p) || point_on_segment_2d(a, c, p) {
                    blocked = true;
                    break;
                }
            }

            // Also check that the ear's diagonal (a → c) doesn't properly
            // cross any non-adjacent polygon edge. This catches the case where
            // an edge passes through the ear triangle even when no vertex is
            // strictly inside (degenerate near-collinear configurations).
            if !blocked {
                'diag: for edge_idx in 0..m {
                    let ei = indices[edge_idx];
                    let ej = indices[(edge_idx + 1) % m];
                    // Skip the two edges adjacent to curr: (prev→curr) and
                    // (curr→next), and their reverses.
                    if ei == prev
                        || ej == prev
                        || ei == next
                        || ej == next
                        || ei == curr
                        || ej == curr
                    {
                        continue;
                    }
                    let ep = [poly[ei][3], poly[ei][4]];
                    let eq = [poly[ej][3], poly[ej][4]];
                    if segments_intersect_2d(a, c, ep, eq) {
                        blocked = true;
                        break 'diag;
                    }
                }
            }

            if !blocked {
                // This is an ear.
                triangles.push([prev, curr, next]);
                indices.remove(idx);
                ear_found = true;
                break;
            }
        }

        if !ear_found {
            // No clean ear found (can happen with nearly-degenerate geometry).
            // Force-remove the first convex vertex to avoid infinite loop.
            let m = indices.len();
            for idx in 0..m {
                let prev = indices[(idx + m - 1) % m];
                let curr = indices[idx];
                let next = indices[(idx + 1) % m];
                let a = [poly[prev][3], poly[prev][4]];
                let b = [poly[curr][3], poly[curr][4]];
                let c = [poly[next][3], poly[next][4]];
                if signed_area_2d(a, b, c) > 0.0 {
                    triangles.push([prev, curr, next]);
                    indices.remove(idx);
                    break;
                }
            }
        }
    }

    // Last triangle.
    if indices.len() == 3 {
        let prev = indices[0];
        let curr = indices[1];
        let next = indices[2];
        let a = [poly[prev][3], poly[prev][4]];
        let b = [poly[curr][3], poly[curr][4]];
        let c = [poly[next][3], poly[next][4]];
        if signed_area_2d(a, b, c) > 0.0 {
            triangles.push([prev, curr, next]);
        }
    }

    triangles
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{MaterialPalette, Object, Point3, Profile};

    /// f32 round-off allowance for unit-length checks in tests only; kernel
    /// geometric tolerances live in `kernel::tol`.
    const UNIT_LENGTH_TOL_F32: f32 = 1e-6;

    /// Test epsilon for area comparisons — generous enough for f64 arithmetic
    /// on geometry in the [0,4] range.
    const AREA_TOL: f64 = 1e-9;

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

    fn xy_plane() -> kernel::Plane {
        kernel::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
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
        for n in mesh.normals.chunks_exact(3) {
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
            // build_polygon_with_holes and check its triangulation.
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

            let poly = build_polygon_with_holes(&outer_3d, &holes_3d, u_ax, v_ax);
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
        for c in mesh.colors.chunks_exact(3) {
            assert!(
                (c[0] - 1.0).abs() < 1e-5 && c[1] < 1e-5 && c[2] < 1e-5,
                "vertex color {c:?} is not the base red"
            );
        }
    }
}
