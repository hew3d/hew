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
    /// A hole loop could not be bridged into the face's boundary without
    /// crossing other geometry, so the face has no honest triangulation
    /// under this algorithm. Unreachable for kernel-valid faces (disjoint
    /// holes strictly inside the outer loop always leave a visible bridge
    /// target — see [`bridge_hole_into_polygon`]); failing loudly instead
    /// of emitting a self-crossing ring is DEVELOPMENT.md rule 4.
    UnbridgeableHole {
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
            TessellateError::UnbridgeableHole { face } => {
                write!(f, "face {face:?} has a hole that cannot be bridged")
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
        let poly = build_polygon_with_holes(&outer_3d, &holes_3d, u_ax, v_ax)
            .map_err(|UnbridgeableHole| TessellateError::UnbridgeableHole { face: face_id })?;

        // Ear-clip the polygon and emit triangles.
        let base = (mesh.positions.len() / 3) as u32;

        // Append polygon vertices (positions, normals, colors, UVs).
        for &[x, y, z, u2d, v2d] in &poly {
            mesh.positions.extend([x as f32, y as f32, z as f32]);
            mesh.normals.extend(n);
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

/// Marker error: a hole has no bridge target that keeps the merged ring
/// weakly simple. `tessellate` reports it as
/// [`TessellateError::UnbridgeableHole`] rather than force a bridge and
/// triangulate a self-crossing ring (DEVELOPMENT.md rule 4).
#[derive(Debug)]
struct UnbridgeableHole;

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
///
/// Every candidate bridge is validated against the current ring AND the
/// not-yet-bridged hole rings before it is spliced (see
/// [`bridge_is_valid`]), so the returned polygon is weakly simple for any
/// kernel-valid face; a hole with no valid bridge target is a typed error,
/// never a silent self-crossing splice.
fn build_polygon_with_holes(
    outer_3d: &[[f64; 3]],
    holes_3d: &[Vec<[f64; 3]>],
    u: kernel::Vec3,
    v: kernel::Vec3,
) -> Result<Vec<[f64; 5]>, UnbridgeableHole> {
    // Build initial poly with 2D coords.
    let mut poly: Vec<[f64; 5]> = outer_3d
        .iter()
        .map(|&p| {
            let [pu, pv] = proj2d(p, u, v);
            [p[0], p[1], p[2], pu, pv]
        })
        .collect();

    if holes_3d.is_empty() {
        return Ok(poly);
    }

    // Project every hole once, then process them sorted by descending max-u.
    // The order is load-bearing: each remaining hole then lies entirely in
    // the half-plane u ≤ M's u-coordinate, which is what lets the Eberly
    // ray construction ignore remaining holes in the common case.
    let mut holes: Vec<Vec<[f64; 5]>> = holes_3d
        .iter()
        .map(|hole| {
            hole.iter()
                .map(|&p| {
                    let [pu, pv] = proj2d(p, u, v);
                    [p[0], p[1], p[2], pu, pv]
                })
                .collect()
        })
        .collect();

    holes.sort_by(|a, b| {
        let max_u_a = a.iter().map(|p| p[3]).fold(f64::NEG_INFINITY, f64::max);
        let max_u_b = b.iter().map(|p| p[3]).fold(f64::NEG_INFINITY, f64::max);
        max_u_b.partial_cmp(&max_u_a).unwrap()
    });

    for i in 0..holes.len() {
        let (hole, remaining) = holes[i..].split_first().unwrap();
        poly = bridge_hole_into_polygon(poly, hole, remaining)?;
    }

    Ok(poly)
}

/// True if the directed segment leaving `apex` toward `q` starts into the
/// region's interior, where the region lies locally to the left of the
/// boundary path `prev → apex → next` (O'Rourke, "Computational Geometry
/// in C", `InCone`). At a pinch vertex the ring visits the same
/// coordinates several times with different neighbours; this test is what
/// selects the occurrence whose angular wedge actually contains the
/// bridge, so zero-width corridor duplicates (empty wedges) are rejected.
fn in_cone_2d(prev: [f64; 2], apex: [f64; 2], next: [f64; 2], q: [f64; 2]) -> bool {
    let left = |a, b, c| signed_area_2d(a, b, c) > 0.0;
    let left_on = |a, b, c| signed_area_2d(a, b, c) >= 0.0;
    if left_on(apex, next, prev) {
        // Convex apex: q must be strictly inside the (< π) wedge.
        left(apex, q, prev) && left(q, apex, next)
    } else {
        // Reflex apex: q must not be inside the complementary wedge.
        !(left_on(apex, q, next) && left_on(q, apex, prev))
    }
}

/// True if bridging `ring[k]` to `hole[m]` keeps the merged ring weakly
/// simple: the bridge starts into the region's interior at both endpoints
/// (in-cone tests, which at duplicated pinch vertices also pick the ring
/// occurrence whose wedge contains the bridge), and its open segment
/// neither passes through a vertex of, nor properly crosses an edge of,
/// the current ring, the hole being bridged, or any not-yet-bridged hole
/// ring. Vertices coincident with an endpoint are permitted (that is the
/// zero-width-corridor hub case); `point_on_segment_2d` is strict, so
/// they don't trip the on-segment test. A ring or hole edge collinear
/// with and overlapping the bridge is caught through its endpoints: any
/// overlap places some vertex of one segment strictly inside the other,
/// and every prior bridge was validated against then-remaining hole
/// vertices, so no existing edge can pass through M or P cleanly.
fn bridge_is_valid(
    ring: &[[f64; 5]],
    k: usize,
    hole: &[[f64; 5]],
    m: usize,
    remaining: &[Vec<[f64; 5]>],
) -> bool {
    let n = ring.len();
    let h = hole.len();
    let p2r = |i: usize| [ring[i][3], ring[i][4]];
    let p2h = |i: usize| [hole[i][3], hole[i][4]];
    let p = p2r(k);
    let mq = p2h(m);
    if p == mq {
        // Zero-length bridge: the hole touches the ring at exactly this
        // vertex (imported geometry pinches annuli like this). The glue
        // crosses nothing, and at a pinch point of a valid face the hole's
        // wedge nests inside the ring's wedge by the validity of the input
        // itself, so the splice is accepted as-is — every direction-based
        // test below would degenerate to zero-area sign noise here.
        return true;
    }
    // Local interiority at the ring endpoint and at the hole endpoint. The
    // hole is traversed in its stored (clockwise-in-uv) order inside the
    // merged CCW ring, so its stored neighbours are its merged-ring
    // neighbours.
    if !in_cone_2d(p2r((k + n - 1) % n), p, p2r((k + 1) % n), mq) {
        return false;
    }
    if !in_cone_2d(p2h((m + h - 1) % h), mq, p2h((m + 1) % h), p) {
        return false;
    }
    // No ring vertex strictly on the open bridge; no ring edge properly
    // crossing it. Edges incident to the endpoints share a coordinate with
    // the bridge, and a proper crossing requires strict straddling, so
    // they need no special-casing.
    for i in 0..n {
        if point_on_segment_2d(p, mq, p2r(i)) {
            return false;
        }
        if segments_intersect_2d(p, mq, p2r(i), p2r((i + 1) % n)) {
            return false;
        }
    }
    // Same for the hole's own ring…
    for i in 0..h {
        if i != m && point_on_segment_2d(p, mq, p2h(i)) {
            return false;
        }
        if segments_intersect_2d(p, mq, p2h(i), p2h((i + 1) % h)) {
            return false;
        }
    }
    // …and for every hole not yet bridged in, which the final polygon must
    // still leave room for. (The vertical-edge snap fix caught one way a
    // bridge could cut a later hole off; this closes the class.)
    for other in remaining {
        let oh = other.len();
        for i in 0..oh {
            let a = [other[i][3], other[i][4]];
            if point_on_segment_2d(p, mq, a) {
                return false;
            }
            let b = [other[(i + 1) % oh][3], other[(i + 1) % oh][4]];
            if segments_intersect_2d(p, mq, a, b) {
                return false;
            }
        }
    }
    true
}

/// Bridge one hole (already projected to `[x, y, z, u2d, v2d]`) into
/// `outer`, returning the merged weakly simple polygon.
///
/// The Eberly ray construction proposes a candidate; if the candidate
/// bridge fails [`bridge_is_valid`] — the ray can land on a zero-width
/// corridor edge, where a float tie between the two coincident edge
/// copies used to pick a splice position whose wedge does not contain
/// the bridge, pinching the ring into a self-crossing — the fallback
/// scans every ring position nearest-first for a valid target (Held's
/// FIST refinement: nearest visible vertex, with the in-cone tests
/// enforcing the cone restriction). For a kernel-valid face a visible
/// target always exists: the region between the hole and the ring is
/// connected, and remaining holes sit in the u ≤ M half-plane while the
/// Eberly triangle sits strictly right of M.
fn bridge_hole_into_polygon(
    outer: Vec<[f64; 5]>,
    hole_verts: &[[f64; 5]],
    remaining: &[Vec<[f64; 5]>],
) -> Result<Vec<[f64; 5]>, UnbridgeableHole> {
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

    // The Eberly candidate from the ray crossing, when one exists.
    let candidate = best_edge_idx.map(|ei| {
        // The crossing lands on edge (ei, ei+1). Choose the mutually visible
        // outer vertex P. The candidate is the endpoint of the edge with the
        // larger u-coordinate (or the intersection itself if it is a vertex).
        let ej = (ei + 1) % n_outer;
        let pi_u = outer[ei][3];
        let pj_u = outer[ej][3];

        // If the intersection is exactly at a vertex, use that vertex. BOTH
        // coordinates must match: comparing u alone made every crossing along
        // a vertical (constant-u) edge "snap" to that edge's start vertex,
        // which funnelled whole rows of holes into one bridge hub whose
        // bridges then crossed the intervening corridors (a self-crossing,
        // unclippable polygon on real vent-hole faces).
        if (pi_u - best_cross_u).abs() < 1e-12 && (outer[ei][4] - best_cross_v).abs() < 1e-12 {
            ei
        } else if (pj_u - best_cross_u).abs() < 1e-12 && (outer[ej][4] - best_cross_v).abs() < 1e-12
        {
            ej
        } else {
            // The crossing is interior; pick the vertex with the larger u.
            // But if any reflex outer vertex lies inside the triangle
            // (M, P, I), prefer the one with the smallest angle from M's
            // +u ray (Eberly's visible-vertex refinement).
            let p_candidate = if pi_u >= pj_u { ei } else { ej };

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
        }
    });

    if let Some(p_idx) = candidate
        && bridge_is_valid(&outer, p_idx, hole_verts, m_idx, remaining)
    {
        return Ok(bridge_at_vertices(&outer, hole_verts, p_idx, m_idx));
    }

    // The ray candidate is blocked (or no crossing was found): fall back to
    // the nearest ring position whose bridge validates. Nearest-first keeps
    // corridors short; validity does the rest.
    let mut order: Vec<usize> = (0..n_outer).collect();
    let dist2 = |i: usize| {
        let du = outer[i][3] - m_u;
        let dv = outer[i][4] - m_v;
        du * du + dv * dv
    };
    order.sort_by(|&i, &j| dist2(i).partial_cmp(&dist2(j)).unwrap());
    for p_idx in order {
        if bridge_is_valid(&outer, p_idx, hole_verts, m_idx, remaining) {
            return Ok(bridge_at_vertices(&outer, hole_verts, p_idx, m_idx));
        }
    }
    Err(UnbridgeableHole)
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

/// Cross-product tolerance `point_on_segment_2d` accepts as "collinear with
/// AB". Shared with the grid pruning stage, which must expand its query box
/// by the band of points this admits (the tolerance divided by |AB|) so it
/// never skips a vertex the exact test could accept.
fn segment_collinear_tol(a: [f64; 2], b: [f64; 2]) -> f64 {
    1e-12 * ((b[0] - a[0]).hypot(b[1] - a[1]) + 1.0)
}

/// True if point P lies strictly on the open segment AB (between A and B,
/// not at the endpoints).
fn point_on_segment_2d(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    // First check collinearity: cross(AB, AP) == 0.
    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    if cross.abs() > segment_collinear_tol(a, b) {
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
    ear_clip_impl(poly, true).0
}

/// Reference implementation of [`ear_clip`] with the two-stage containment
/// pruning bypassed — an honest full scan per ear candidate, kept so the
/// executable specs and property tests can assert the pruned path returns
/// byte-for-byte identical triangulations (DEVELOPMENT.md rule 3), mirroring
/// `resolve_linear` in crates/inference. Not part of the supported API.
#[cfg(test)]
fn ear_clip_reference(poly: &[[f64; 5]]) -> (Vec<[usize; 3]>, ClipCounters) {
    ear_clip_impl(poly, false)
}

/// True if vertex `b` (with its current neighbours `a`, `c` in the remaining
/// polygon) needs the triangle-INTERIOR containment test when it is scanned
/// as a potential ear blocker.
///
/// Ear candidacy classifies `b` as convex when `signed_area_2d(a, b, c) > 0`.
/// In a weakly simple CCW polygon, a strictly convex vertex can never lie
/// strictly inside a candidate ear triangle — only reflex vertices need the
/// interior test (Eberly, "Triangulation by Ear Clipping") — so reflex and
/// degenerate vertices are kept and strictly convex ones are pruned. The
/// pruning covers ONLY the interior half of the blocked predicate: a
/// strictly convex vertex can still lie exactly ON one of a candidate
/// ear's open sides (bridged lattice geometry puts them on the diagonal;
/// imported point-touching holes run whole chains along the other two
/// sides), so the cheap `point_on_segment_2d` half is applied to every
/// remaining vertex, membership or not — see the two-tier scan in
/// [`clip_ring`].
///
/// Conservative band: hole bridging duplicates vertices and creates collinear
/// runs, so a vertex whose classification sits on the convex/reflex boundary
/// must NOT be pruned, or an interior test that could return true would be
/// skipped. `2·area / |c − a|` is `b`'s distance off the chord `a → c`, so
/// `b` is kept unless it clears the chord by more than
/// [`kernel::tol::POINT_MERGE`] on the convex side (coordinates here are the
/// planar projection of kernel geometry, in meters, so the constant applies
/// unchanged).
fn must_containment_test(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let chord = (c[0] - a[0]).hypot(c[1] - a[1]);
    signed_area_2d(a, b, c) <= 0.5 * kernel::tol::POINT_MERGE * chord
}

/// Below this many ring vertices the flat list scan wins over grid upkeep,
/// so the grid stage is skipped. A size cutoff, not a geometric tolerance;
/// typical faces (quads, simple profiles) never reach it.
const GRID_MIN_RING: usize = 16;

/// Relative slack for the enclosed-area budget guard in [`clip_ring`]: the
/// running budget accumulates one rounding error per clipped ear, so a
/// legitimate final ear can overshoot the drifted budget by a few ulps of
/// the ring's initial area (drift ≈ n·ε ≈ 1e-12 of it for the largest real
/// rings). 1e-9 dwarfs that drift while still rejecting any phantom ear of
/// consequence. A crate-local constant rather than `kernel::tol`: it
/// bounds this algorithm's own floating-point drift, not a geometric
/// length (mirrors `segment_collinear_tol`'s crate-local cross tolerance).
const RING_AREA_BUDGET_REL_SLACK: f64 = 1e-9;

/// Uniform grid over the remaining ring vertices' (u2d, v2d) positions —
/// the second pruning stage. A vertex can block an ear candidate only if it
/// lies in the closed candidate triangle (a subset of the triangle's
/// bounding box) or inside `point_on_segment_2d`'s collinearity band around
/// the diagonal, so scanning just the cells overlapping the padded box
/// (see `grid_query_pad`) skips only vertices that cannot block. The grid
/// indexes EVERY remaining ring vertex — not just the interior-test set —
/// because the on-diagonal half of the blocked predicate applies to all of
/// them; positions never change, so a vertex's cell is recomputable at
/// removal time.
struct RingGrid {
    min: [f64; 2],
    /// Cells per unit length on each axis (0-width axes collapse to 1 cell;
    /// the query clamp handles the resulting non-finite cell coordinates).
    inv_cell: [f64; 2],
    cols: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
}

impl RingGrid {
    /// Builds a grid sized for `members` (~1 occupant per cell, apportioned
    /// by the polygon's aspect ratio). `members` is the ring being clipped —
    /// possibly a sub-ring of the whole polygon — and the bounds cover every
    /// polygon vertex, so each member lands inside. Vertices only ever leave
    /// the grid (when clipped off the ring); membership in the interior-test
    /// set is a per-vertex flag consulted at query time, not mirrored here.
    fn build(poly: &[[f64; 5]], members: &[usize]) -> RingGrid {
        let count = members.len();
        let mut min = [f64::INFINITY, f64::INFINITY];
        let mut max = [f64::NEG_INFINITY, f64::NEG_INFINITY];
        for v in poly {
            min[0] = min[0].min(v[3]);
            min[1] = min[1].min(v[4]);
            max[0] = max[0].max(v[3]);
            max[1] = max[1].max(v[4]);
        }
        let width = (max[0] - min[0]).max(0.0);
        let height = (max[1] - min[1]).max(0.0);
        // cols·rows ≈ count with cols/rows ≈ width/height.
        let cols = if height > 0.0 {
            ((count as f64 * width / height).sqrt().ceil() as usize).max(1)
        } else {
            count.max(1)
        };
        let rows = (count / cols).max(1);
        let mut grid = RingGrid {
            min,
            inv_cell: [cols as f64 / width, rows as f64 / height],
            cols,
            rows,
            cells: vec![Vec::new(); cols * rows],
        };
        for &v in members {
            let cell = grid.cell_of([poly[v][3], poly[v][4]]);
            grid.cells[cell].push(v);
        }
        grid
    }

    /// Clamped cell coordinates of a point (Rust's saturating f64→usize cast
    /// maps NaN/negative to 0 and +∞ past the clamp, so degenerate axes and
    /// padded query corners outside the bounds stay in range).
    fn cell_coords(&self, p: [f64; 2]) -> [usize; 2] {
        let col = (((p[0] - self.min[0]) * self.inv_cell[0]) as usize).min(self.cols - 1);
        let row = (((p[1] - self.min[1]) * self.inv_cell[1]) as usize).min(self.rows - 1);
        [col, row]
    }

    fn cell_of(&self, p: [f64; 2]) -> usize {
        let [col, row] = self.cell_coords(p);
        row * self.cols + col
    }

    fn remove(&mut self, vertex: usize, p: [f64; 2]) {
        let cell = self.cell_of(p);
        if let Some(pos) = self.cells[cell].iter().position(|&v| v == vertex) {
            self.cells[cell].swap_remove(pos);
        }
    }
}

/// Padding for the grid query box contributed by one triangle side
/// `a → c` (the caller takes the max over all three sides): the
/// closed-triangle containment is covered by the box itself with
/// [`kernel::tol::POINT_MERGE`] of slack for round-off, and the on-side
/// test additionally admits `point_on_segment_2d`'s collinear band, whose
/// half-width is its cross tolerance over the side length — a point that
/// test accepts sits strictly between `a` and `c` on the major axis and
/// within the band off the line, so it lies inside the side's bounding
/// box (a subset of the triangle's) inflated by the band, and the padded
/// query can never skip it. For a near-zero side (bridge-remnant slivers)
/// the band — and so the padding — grows unboundedly and the query
/// degrades to scanning every cell, which is exactly the conservative
/// full scan.
fn grid_query_pad(a: [f64; 2], c: [f64; 2]) -> f64 {
    let diagonal = (c[0] - a[0]).hypot(c[1] - a[1]);
    kernel::tol::POINT_MERGE.max(segment_collinear_tol(a, c) / diagonal)
}

/// Work counters returned by [`ear_clip_impl`], consumed by the executable
/// specs: `containment_tests` feeds the pruning perf spec
/// (`wall_with_holes_prunes_containment_tests`) and `fallback_passes` makes
/// the no-ear recovery observable, so a reintroduced no-progress spin fails
/// its O(n) bound instead of silently draining the safety counter.
#[derive(Debug, Clone, Copy, Default)]
struct ClipCounters {
    /// Per-vertex blocked-predicate evaluations (either tier).
    containment_tests: usize,
    /// Full ring passes that found no valid ear and entered the fallback.
    fallback_passes: usize,
}

/// Shared body of `ear_clip`/`ear_clip_reference`: with `prune` set, the
/// per-candidate blocked scan is two-tier — vertices in the maintained
/// interior-test set (see [`must_containment_test`]) get the full blocked
/// predicate, every other remaining vertex only the on-side
/// `point_on_segment_2d` half (a strictly convex vertex cannot lie strictly
/// inside the candidate triangle, but can lie exactly on one of its open
/// sides) — and, for large rings, only the [`RingGrid`] cells near the
/// candidate are visited. Without `prune`, every remaining vertex gets the full
/// predicate. Pruning only skips interior tests that cannot return true,
/// so both paths emit identical triangulations on any weakly simple input —
/// which bridging kernel-valid loops produces, and which clipping a
/// validated ear preserves (the no-ear fallbacks in [`clip_ring`] are also
/// prune-independent, so the guarantee survives them).
///
/// Rings are processed off a work stack because the no-ear fallback can
/// split a pinched ring (see [`clip_ring`]) into two independent sub-rings.
fn ear_clip_impl(poly: &[[f64; 5]], prune: bool) -> (Vec<[usize; 3]>, ClipCounters) {
    let n = poly.len();
    if n < 3 {
        return (Vec::new(), ClipCounters::default());
    }
    let mut triangles: Vec<[usize; 3]> = Vec::with_capacity(n - 2);
    let mut counters = ClipCounters::default();
    let mut rings: Vec<Vec<usize>> = vec![(0..n).collect()];
    while let Some(ring) = rings.pop() {
        clip_ring(poly, ring, prune, &mut triangles, &mut rings, &mut counters);
    }
    (triangles, counters)
}

/// Signed area of a ring of `poly` indices, in projected (u, v)
/// coordinates. Positive = CCW. Computed as a triangle fan anchored at the
/// ring's first vertex rather than a raw shoelace: the raw form multiplies
/// absolute coordinates, so its cancellation error grows with the ring's
/// distance from the origin squared (imported models sit at site
/// coordinates hundreds of meters out), and the enclosed-area budget guard
/// in [`clip_ring`] compares this value against vertex-relative
/// [`signed_area_2d`] ear areas to within a slack that error would dwarf.
fn ring_area_2d(poly: &[[f64; 5]], ring: &[usize]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }
    let p0 = [poly[ring[0]][3], poly[ring[0]][4]];
    let mut sum = 0.0f64;
    for k in 1..ring.len() - 1 {
        sum += signed_area_2d(
            p0,
            [poly[ring[k]][3], poly[ring[k]][4]],
            [poly[ring[k + 1]][3], poly[ring[k + 1]][4]],
        );
    }
    sum
}

/// True if the segment joining ring positions `i` and `j` is a diagonal the
/// ring's region can be split along: positive length, touching no other
/// ring vertex, properly crossing no ring edge, both sub-rings positively
/// oriented, and interior to the ring (midpoint even-odd test — the other
/// conditions guarantee the diagonal doesn't leave the region between
/// boundary contacts, so one interior point decides for the whole segment).
fn valid_split_diagonal(poly: &[[f64; 5]], ring: &[usize], i: usize, j: usize) -> bool {
    let m = ring.len();
    let p2 = |k: usize| [poly[ring[k]][3], poly[ring[k]][4]];
    let a = p2(i);
    let b = p2(j);
    if a == b {
        return false;
    }
    // No ring vertex strictly interior to the diagonal.
    for k in 0..m {
        if k == i || k == j {
            continue;
        }
        if point_on_segment_2d(a, b, p2(k)) {
            return false;
        }
    }
    // No proper crossing with any ring edge.
    for k in 0..m {
        let k2 = (k + 1) % m;
        if k == i || k == j || k2 == i || k2 == j {
            continue;
        }
        if segments_intersect_2d(a, b, p2(k), p2(k2)) {
            return false;
        }
    }
    // Both sub-rings enclose area counter-clockwise (a negative side would
    // mean the diagonal cuts a hole loop off, or lies outside the region).
    if ring_area_2d(poly, &ring[i..=j]) <= 0.0
        || ring_area_2d(poly, &[&ring[j..], &ring[..=i]].concat()) <= 0.0
    {
        return false;
    }
    // Midpoint strictly inside the ring (even-odd rule).
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let mut inside = false;
    let mut kp = m - 1;
    for k in 0..m {
        let pk = p2(k);
        let pp = p2(kp);
        if (pk[1] > mid[1]) != (pp[1] > mid[1]) {
            let cross_x = pp[0] + (mid[1] - pp[1]) * (pk[0] - pp[0]) / (pk[1] - pp[1]);
            if mid[0] < cross_x {
                inside = !inside;
            }
        }
        kp = k;
    }
    inside
}

/// Ear-clips one ring (a cyclic list of indices into `poly`), appending
/// triangles to `triangles`. When a pass over the ring finds no valid ear —
/// which cannot happen for a simple polygon (two-ears theorem) but does for
/// the weakly simple output of hole bridging, whose duplicated bridge
/// vertices can pinch the ring — recovery is attempted in order:
///
/// 1. Remove one exactly-degenerate vertex (zero signed area with its
///    neighbours: a coincident bridge remnant, spike tip, or collinear run)
///    without emitting a triangle. The removed triangle has zero area, so
///    the enclosed region is untouched.
/// 2. Split the ring along a valid interior diagonal (see
///    [`valid_split_diagonal`]) into two sub-rings that share the diagonal
///    as an edge. The sub-rings enclose the same region between them, each
///    is strictly smaller than the ring, and pinch-attached hole loops stay
///    attached to whichever side they belong, so this always progresses
///    honestly. Sub-rings go onto `pending` and are clipped independently.
/// 3. Force-remove the first convex vertex (emitting its triangle). The
///    remainder is no longer guaranteed weakly simple, so the reflex-only
///    pruning argument stops holding: pruning is demoted to the full
///    containment scan for the rest of this ring.
/// 4. Give up on this ring: with no convex vertex the remainder winds the
///    wrong way and has no honest triangles left. Stopping keeps the loop
///    from re-scanning an unchanged ring until the safety counter drains —
///    the measured 3-second pathology on real vent-hole faces.
///
/// Independent of the vertex tests, every accepted ear must also fit the
/// ring's remaining enclosed-area budget (the shoelace area at entry,
/// less each clipped ear). A pinched ring can collapse to a zero-area
/// "bowtie" — two coincident boundary passes plus a leftover chord —
/// whose final triangle passes every strict vertex test because its only
/// witness duplicates one of its own corners; its area, though, exceeds
/// the (spent) budget, so the guard rejects it and the degenerate-vertex
/// fallback dissolves the bowtie without emitting. Point-touching hole
/// rings from imported models hit this (see
/// `point_touching_holes_are_area_honest`).
fn clip_ring(
    poly: &[[f64; 5]],
    mut indices: Vec<usize>,
    prune: bool,
    triangles: &mut Vec<[usize; 3]>,
    pending: &mut Vec<Vec<usize>>,
    counters: &mut ClipCounters,
) {
    let n = poly.len();
    let p2 = |i: usize| [poly[i][3], poly[i][4]];
    if indices.len() < 3 {
        return;
    }
    if indices.len() == 3 {
        // Single triangle — emit only if CCW (non-degenerate).
        if signed_area_2d(p2(indices[0]), p2(indices[1]), p2(indices[2])) > 0.0 {
            triangles.push([indices[0], indices[1], indices[2]]);
        }
        return;
    }

    // Enclosed-area budget for the guard described above. The slack covers
    // the one rounding error each subtraction can add; it is relative to
    // the ring's own area, so a zero-area bowtie ring gets zero slack and
    // every positive ear is rejected outright.
    let mut area_budget = ring_area_2d(poly, &indices);
    let area_slack = RING_AREA_BUDGET_REL_SLACK * area_budget.abs();

    // Interior-test set for the pruned path: the reflex (and conservatively,
    // near-degenerate) vertices of the remaining ring. Vertices outside the
    // set still get the on-diagonal half of the blocked predicate — the
    // flag only downgrades the test, never skips the vertex.
    let mut prune = prune;
    let mut in_test_set: Vec<bool> = vec![false; n];
    let mut grid: Option<RingGrid> = None;
    if prune {
        let m = indices.len();
        for k in 0..m {
            let i = indices[k];
            in_test_set[i] = must_containment_test(
                p2(indices[(k + m - 1) % m]),
                p2(i),
                p2(indices[(k + 1) % m]),
            );
        }
        if m >= GRID_MIN_RING {
            grid = Some(RingGrid::build(poly, &indices));
        }
    }

    // Ear clipping with two-stage pruning. Stage 1 (interior-test set):
    // each candidate's scan runs the O(1)-per-vertex full blocked predicate
    // only on the O(r) reflex vertices, and the cheaper on-diagonal half on
    // the rest; the set updates in O(1) per clip (an ear removal changes
    // only its two neighbours' angles). Stage 2 (`RingGrid`, for large
    // rings such as bridged walls full of window holes, where r stays
    // proportional to n): only the grid cells overlapping the candidate
    // triangle's padded bounding box are scanned, so local ears test O(1)
    // vertices of either tier. Together they replace the previous O(n²)
    // containment tests with near-linear work on convex- and hole-dominated
    // polygons alike; the diagonal-crossing sweep below stays O(n) per
    // clipped ear, so the loop is O(n²) in cheap segment tests but no
    // longer in containment tests.
    let mut safety = indices.len() * indices.len() + indices.len(); // iteration bound
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

            let a = p2(prev);
            let b = p2(curr);
            let c = p2(next);

            // Is the vertex at `curr` a convex (ear) vertex?
            // In a CCW polygon, a vertex is convex if cross(b-a, c-a) > 0.
            let ear_area = signed_area_2d(a, b, c);
            if ear_area <= 0.0 {
                continue; // reflex or degenerate
            }
            // Enclosed-area budget guard (see the doc comment above): an
            // honest ear never exceeds what the ring still encloses.
            if ear_area > area_budget + area_slack {
                continue;
            }

            // Is any other polygon vertex strictly inside the triangle
            // (a, b, c), or strictly on one of its OPEN sides? The interior
            // half of the predicate is needed only for interior-test-set
            // vertices (reflex pruning); the on-side half applies to EVERY
            // remaining vertex, because a strictly convex vertex can sit
            // exactly on the diagonal (bridged lattice geometry) — and a
            // pinched ring can run a whole chain exactly along the
            // prev→curr or curr→next edge (imported point-touching holes),
            // where clipping the ear would cover non-region area. Strict
            // on-segment tests keep coordinate-duplicates of the ear's own
            // corners (zero-width corridor hubs) from blocking. The grid
            // narrows both tiers to the vertices near the triangle; the
            // reference path runs the full predicate on everything.
            let mut test_one = |other: usize, full: bool| -> bool {
                if other == prev || other == curr || other == next {
                    return false;
                }
                let p = p2(other);
                counters.containment_tests += 1;
                (full && point_in_triangle_2d(a, b, c, p))
                    || point_on_segment_2d(a, c, p)
                    || point_on_segment_2d(a, b, p)
                    || point_on_segment_2d(b, c, p)
            };
            let mut blocked = false;
            if let (true, Some(grid)) = (prune, &grid) {
                let pad = grid_query_pad(a, c)
                    .max(grid_query_pad(a, b))
                    .max(grid_query_pad(b, c));
                let lo = grid.cell_coords([
                    a[0].min(b[0]).min(c[0]) - pad,
                    a[1].min(b[1]).min(c[1]) - pad,
                ]);
                let hi = grid.cell_coords([
                    a[0].max(b[0]).max(c[0]) + pad,
                    a[1].max(b[1]).max(c[1]) + pad,
                ]);
                'cells: for row in lo[1]..=hi[1] {
                    for col in lo[0]..=hi[0] {
                        for &other in &grid.cells[row * grid.cols + col] {
                            if test_one(other, in_test_set[other]) {
                                blocked = true;
                                break 'cells;
                            }
                        }
                    }
                }
            } else {
                for &other in &indices {
                    if test_one(other, !prune || in_test_set[other]) {
                        blocked = true;
                        break;
                    }
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
                    if segments_intersect_2d(a, c, p2(ei), p2(ej)) {
                        blocked = true;
                        break 'diag;
                    }
                }
            }

            if !blocked {
                // This is an ear.
                triangles.push([prev, curr, next]);
                area_budget -= ear_area;
                indices.remove(idx);
                if prune {
                    // The clipped vertex leaves the ring (and so the grid);
                    // the removal changes the interior angle of exactly its
                    // two neighbours, so only they are reclassified.
                    in_test_set[curr] = false;
                    if let Some(grid) = &mut grid {
                        grid.remove(curr, p2(curr));
                    }
                    let m = indices.len();
                    for pos in [(idx + m - 1) % m, idx % m] {
                        let vp = indices[(pos + m - 1) % m];
                        let vc = indices[pos];
                        let vn = indices[(pos + 1) % m];
                        in_test_set[vc] = must_containment_test(p2(vp), p2(vc), p2(vn));
                    }
                }
                ear_found = true;
                break;
            }
        }

        if !ear_found {
            counters.fallback_passes += 1;
            // No valid ear in a full pass: the ring is pinched or otherwise
            // degenerate. Recover per the doc comment above — degenerate
            // vertex removal, then pinch split, then force removal, then
            // give up — each option strictly shrinking the problem so the
            // loop never re-scans an unchanged ring.
            let m = indices.len();

            // 1. Remove one exactly-degenerate vertex (no triangle emitted).
            let degenerate = (0..m).find(|&idx| {
                let a = p2(indices[(idx + m - 1) % m]);
                let b = p2(indices[idx]);
                let c = p2(indices[(idx + 1) % m]);
                signed_area_2d(a, b, c) == 0.0
            });
            if let Some(idx) = degenerate {
                let curr = indices[idx];
                indices.remove(idx);
                if prune {
                    in_test_set[curr] = false;
                    if let Some(grid) = &mut grid {
                        grid.remove(curr, p2(curr));
                    }
                    let m = indices.len();
                    for pos in [(idx + m - 1) % m, idx % m] {
                        let vp = indices[(pos + m - 1) % m];
                        let vc = indices[pos];
                        let vn = indices[(pos + 1) % m];
                        in_test_set[vc] = must_containment_test(p2(vp), p2(vc), p2(vn));
                    }
                }
                continue;
            }

            // 2. Split along a valid interior diagonal and clip the two
            // sub-rings independently.
            let mut split: Option<(usize, usize)> = None;
            'pairs: for i in 0..m {
                for j in (i + 2)..m {
                    if i == 0 && j == m - 1 {
                        continue; // adjacent around the wrap
                    }
                    if valid_split_diagonal(poly, &indices, i, j) {
                        split = Some((i, j));
                        break 'pairs;
                    }
                }
            }
            if let Some((i, j)) = split {
                // Both sub-rings keep copies of the diagonal's endpoints;
                // the diagonal becomes the closing edge of each.
                let ring_a = indices[i..=j].to_vec();
                let ring_b = [&indices[j..], &indices[..=i]].concat();
                pending.push(ring_a);
                pending.push(ring_b);
                return;
            }

            // 3. Force-remove the first convex vertex. The remainder is no
            // longer guaranteed weakly simple, so the reflex-only argument
            // stops holding: demote to the full (reference) containment
            // scan for the rest of this ring.
            prune = false;
            grid = None;
            let forced = (0..m).find(|&idx| {
                let a = p2(indices[(idx + m - 1) % m]);
                let b = p2(indices[idx]);
                let c = p2(indices[(idx + 1) % m]);
                signed_area_2d(a, b, c) > 0.0
            });
            match forced {
                Some(idx) => {
                    let prev = indices[(idx + m - 1) % m];
                    let curr = indices[idx];
                    let next = indices[(idx + 1) % m];
                    triangles.push([prev, curr, next]);
                    area_budget -= signed_area_2d(p2(prev), p2(curr), p2(next));
                    indices.remove(idx);
                }
                // 4. No convex vertex at all: nothing honest remains.
                None => return,
            }
        }
    }

    // Last triangle — CCW and within the remaining budget (the budget
    // guard's bowtie case can leave a positive-area chord triangle here
    // whose region was already emitted).
    if indices.len() == 3 {
        let prev = indices[0];
        let curr = indices[1];
        let next = indices[2];
        let area = signed_area_2d(p2(prev), p2(curr), p2(next));
        if area > 0.0 && area <= area_budget + area_slack {
            triangles.push([prev, curr, next]);
        }
    }
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

    /// Outer rectangle and CW rectangular hole loops of an "architectural
    /// wall": each hole entry is `[x0, y0, x1, y1]`; holes wind CW in
    /// (x, y), opposite the CCW outer, matching kernel inner loops.
    fn wall_shape(
        width: f64,
        height: f64,
        holes: &[[f64; 4]],
    ) -> (Vec<[f64; 3]>, Vec<Vec<[f64; 3]>>) {
        let outer = vec![
            [0.0, 0.0, 0.0],
            [width, 0.0, 0.0],
            [width, height, 0.0],
            [0.0, height, 0.0],
        ];
        let holes_3d: Vec<Vec<[f64; 3]>> = holes
            .iter()
            .map(|&[x0, y0, x1, y1]| {
                vec![[x0, y0, 0.0], [x0, y1, 0.0], [x1, y1, 0.0], [x1, y0, 0.0]]
            })
            .collect();
        (outer, holes_3d)
    }

    /// Builds the bridged "architectural wall" polygon from [`wall_shape`]
    /// loops, holes bridged into a single weakly simple polygon — the
    /// profiled pathological ear-clipping input.
    fn wall_polygon(width: f64, height: f64, holes: &[[f64; 4]]) -> Vec<[f64; 5]> {
        let (outer, holes_3d) = wall_shape(width, height, holes);
        // u × v = +Z (right-handed), so u2d = x and v2d = y.
        build_polygon_with_holes(
            &outer,
            &holes_3d,
            kernel::Vec3::new(1.0, 0.0, 0.0),
            kernel::Vec3::new(0.0, 1.0, 0.0),
        )
        .expect("wall holes must be bridgeable")
    }

    /// `wall_polygon` with `n` uniform windows, one per unit cell.
    fn wall_polygon_with_n_holes(n: usize) -> Vec<[f64; 5]> {
        let holes: Vec<[f64; 4]> = (0..n)
            .map(|i| [i as f64 + 0.25, 0.25, i as f64 + 0.75, 0.75])
            .collect();
        wall_polygon(n as f64, 1.0, &holes)
    }

    /// A circular hole as a 64-gon, wound CW in (x, y) to match kernel
    /// inner loops (opposite the CCW outer boundary).
    fn circle_hole(cx: f64, cy: f64, r: f64) -> Vec<[f64; 3]> {
        (0..64)
            .map(|i| {
                let a = -(i as f64) * std::f64::consts::TAU / 64.0;
                [cx + r * a.cos(), cy + r * a.sin(), 0.0]
            })
            .collect()
    }

    /// Region area (outer minus holes) of XY-plane loops, by shoelace.
    fn region_area(outer: &[[f64; 3]], holes: &[Vec<[f64; 3]>]) -> f64 {
        let shoelace = |pts: &[[f64; 3]]| {
            let n = pts.len();
            let mut s = 0.0f64;
            for i in 0..n {
                let j = (i + 1) % n;
                s += pts[i][0] * pts[j][1] - pts[j][0] * pts[i][1];
            }
            0.5 * s
        };
        // Holes wind CW: negative area.
        shoelace(outer) + holes.iter().map(|h| shoelace(h)).sum::<f64>()
    }

    /// Bridges `holes` into `outer` on the XY plane and ear-clips (pruned
    /// path), asserting every emitted triangle is CCW (non-degenerate) and
    /// the triangle areas sum exactly to outer-minus-holes — the tessellated
    /// region neither overlaps itself nor misses area. This oracle is
    /// independent of the pruned/reference identity claim: both share the
    /// bridging stage, so only an area check can catch a bridge that pinches
    /// the ring into a self-crossing. Returns the polygon and counters for
    /// callers that pin more.
    fn assert_area_honest(
        outer: &[[f64; 3]],
        holes: &[Vec<[f64; 3]>],
    ) -> (Vec<[f64; 5]>, Vec<[usize; 3]>, ClipCounters) {
        let u = kernel::Vec3::new(1.0, 0.0, 0.0);
        let v = kernel::Vec3::new(0.0, 1.0, 0.0);
        let poly = build_polygon_with_holes(outer, holes, u, v)
            .expect("valid test loops must be bridgeable");
        let (tris, counters) = ear_clip_impl(&poly, true);
        // A no-ear recovery may legitimately fire, but never spin: a pass
        // over the ring either clips, drops a degenerate vertex, splits, or
        // gives up, so passes stay O(n).
        assert!(
            counters.fallback_passes <= poly.len(),
            "no-ear fallback passes {} exceed ring size {} (spin)",
            counters.fallback_passes,
            poly.len()
        );
        let expected = region_area(outer, holes);
        let mut sum = 0.0f64;
        for &[i, j, k] in &tris {
            let area = signed_area_2d(
                [poly[i][3], poly[i][4]],
                [poly[j][3], poly[j][4]],
                [poly[k][3], poly[k][4]],
            );
            assert!(area > 0.0, "triangle [{i},{j},{k}] not CCW: area {area}");
            sum += area;
        }
        assert!(
            (sum - expected).abs() < AREA_TOL,
            "triangle area sum {sum} ≠ region area {expected}"
        );
        (poly, tris, counters)
    }

    /// [`assert_area_honest`], plus completeness: one triangle per clip
    /// (`n − 2`) and zero no-ear fallback passes — the polygon ear-clipped
    /// start to finish without recovery.
    fn assert_clips_completely(outer: &[[f64; 3]], holes: &[Vec<[f64; 3]>]) {
        let (poly, tris, counters) = assert_area_honest(outer, holes);
        assert_eq!(
            tris.len(),
            poly.len() - 2,
            "expected a full ear-clip (no fallback)"
        );
        assert_eq!(counters.fallback_passes, 0, "no-ear fallback fired");
    }

    /// Happy-path scale fixture shaped like the profiled real-model face: a
    /// plain rectangle with two rows of 16 circular 64-gon vent holes
    /// (~2,100 ring vertices). Bridging chains the holes through exact
    /// vertex-to-vertex zero-width corridors; the resulting weakly simple
    /// polygon must ear-clip completely and honestly. (This fixture does
    /// NOT reproduce the historical corridor-piercing pathology — its
    /// corner diagonal misses the nearest hole ring by ~0.002; the
    /// pathological pins are `four_window_wall_triangulates_completely` and
    /// `lattice_wall_pruned_equals_reference`.)
    #[test]
    fn vent_grid_face_triangulates_completely() {
        let outer = [
            [0.0, 0.0, 0.0],
            [0.32, 0.0, 0.0],
            [0.32, 0.16, 0.0],
            [0.0, 0.16, 0.0],
        ];
        let mut holes = Vec::new();
        for row in 0..2 {
            for col in 0..16 {
                holes.push(circle_hole(
                    0.01 + col as f64 * 0.02,
                    0.04 + row as f64 * 0.08,
                    0.007,
                ));
            }
        }
        assert_clips_completely(&outer, &holes);
    }

    /// Regression for the hole-bridging vertex snap: the ray crossing may
    /// only snap to an edge endpoint when BOTH projected coordinates
    /// match. Comparing u alone made every crossing along a vertical
    /// (constant-u) edge snap to that edge's start vertex, bypassing the
    /// visibility refinement — here the upper hole would bridge straight
    /// to the bottom-right corner, piercing the lower hole's arc and
    /// leaving a self-crossing polygon that mangled the triangulation
    /// (missing triangles plus overlapping area, on real vent faces too).
    #[test]
    fn bridge_vertex_snap_matches_both_coordinates() {
        let outer = [
            [0.0, 0.0, 0.0],
            [0.34, 0.0, 0.0],
            [0.34, 0.1, 0.0],
            [0.0, 0.1, 0.0],
        ];
        let holes = vec![
            circle_hole(0.325, 0.03, 0.007),
            circle_hole(0.30, 0.08, 0.007),
        ];
        assert_clips_completely(&outer, &holes);
    }

    /// Distilled from an imported real-model face (a rosette): two holes
    /// that touch each other at two points and share a straight boundary
    /// run. hole 1 is the right half of a diamond annulus; hole 2 is the
    /// left half of the inner diamond, its right boundary running straight
    /// down the shared diameter through an intermediate vertex. Clipping
    /// eventually reduces the pinched ring to a zero-area bowtie whose
    /// leftover triangle spans hole interior; without the enclosed-area
    /// budget guard the tail emitted it, covering hole area once over
    /// (the whole-model symptom: tessellated area exceeding the sum of
    /// the faces' region areas).
    #[test]
    fn point_touching_holes_are_area_honest() {
        let outer = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
        ];
        let holes = vec![
            // Right half-annulus (CW): outer arc radius 4, inner radius 3.
            vec![
                [5.0, 1.0, 0.0],
                [5.0, 2.0, 0.0],
                [8.0, 5.0, 0.0],
                [5.0, 8.0, 0.0],
                [5.0, 9.0, 0.0],
                [9.0, 5.0, 0.0],
            ],
            // Left half inner diamond (CW), diameter split by (5,5).
            vec![
                [5.0, 2.0, 0.0],
                [2.0, 5.0, 0.0],
                [5.0, 8.0, 0.0],
                [5.0, 5.0, 0.0],
            ],
        ];
        assert_area_honest(&outer, &holes);
    }

    /// Regression for reflex-only pruning missing an on-diagonal blocker:
    /// outer (0,0)–(3.6,2.1) with CW holes (3.0,0.6)–(3.3,1.2) and
    /// (2.1,0.9)–(2.7,1.8). After bridging, the duplicate of hole corner
    /// (3.0,0.6) is strictly convex — pruned from the interior-test set —
    /// yet lies exactly on the ear diagonal (3.6,0)→(2.7,0.9) of a later
    /// candidate. At ×11.8 scale the collinear triple (3.6,0), (3.0,0.6),
    /// (2.7,0.9) rounds to a strictly positive area, the candidate becomes
    /// convex, and the reflex-only pruned path clipped the ear the
    /// reference blocked ([1,3,4],[1,4,5] instead of [3,4,5],[1,3,5]). The
    /// on-diagonal half of the blocked predicate now consults every
    /// remaining vertex, so pruned ≡ reference by construction.
    #[test]
    fn lattice_wall_pruned_equals_reference() {
        for s in [1.0, 7.3, 11.8] {
            let holes = [
                [3.0 * s, 0.6 * s, 3.3 * s, 1.2 * s],
                [2.1 * s, 0.9 * s, 2.7 * s, 1.8 * s],
            ];
            let poly = wall_polygon(3.6 * s, 2.1 * s, &holes);
            assert_eq!(
                ear_clip_impl(&poly, true).0,
                ear_clip_reference(&poly).0,
                "pruned diverged from reference at scale {s}"
            );
        }
        // An independently found divergence from the 0.3-grid lattice
        // class (coordinates computed exactly as the generator does).
        let g = |k: u32| 0.3 * k as f64;
        let holes = [
            [g(1), g(4), g(8), g(6)],
            [3.0 + g(1), g(1), 3.0 + g(4), g(2)],
        ];
        let poly = wall_polygon(6.0, 3.0, &holes);
        assert_eq!(
            ear_clip_impl(&poly, true).0,
            ear_clip_reference(&poly).0,
            "pruned diverged from reference on the 0.3-grid wall"
        );
    }

    /// Regression for the bridge splice landing on the wrong duplicate of a
    /// zero-width corridor hub: on this 4-window wall, window 0's +u ray
    /// hits the corridor between window 1 and the outer corner (4,0) — an
    /// edge the ring contains twice, once per direction. A float tie
    /// between the coincident copies used to pick the ring occurrence of
    /// (4,0) whose angular wedge does not contain the bridge, pinching the
    /// ring into a self-crossing that the no-ear fallback force-emitted
    /// from silently: 23 of 26 triangles summing to 4.2768 over a 3.2711
    /// region — overlap exceeding even the outer rectangle. The in-cone
    /// validation now rejects that occurrence and the nearest-visible
    /// fallback picks an honest target.
    #[test]
    fn four_window_wall_triangulates_completely() {
        let holes = [
            [0.05, 0.12446618964663819, 0.55, 0.55],
            [1.05, 0.14478665682206163, 1.7634160442433584, 0.55],
            [2.05, 0.3269601368586668, 2.55, 0.55],
            [3.05, 0.31892744386251753, 3.55, 0.55],
        ];
        let (outer, holes_3d) = wall_shape(4.0, 1.0, &holes);
        assert_clips_completely(&outer, &holes_3d);
    }

    /// A ring with no convex vertex (a CW polygon) emits nothing after
    /// exactly ONE no-ear fallback pass. Guards the no-progress exit of the
    /// fallback via the pass counter: the previous fallback re-scanned the
    /// unchanged ring until an O(n²) safety counter drained (where the
    /// profiled multi-second faces spent their time), which also emitted
    /// nothing — only the counter distinguishes the honest give-up from
    /// the spin.
    #[test]
    fn cw_ring_gives_up_in_one_fallback_pass() {
        let cw_square: Vec<[f64; 5]> = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]
            .iter()
            .map(|&[x, y]| [x, y, 0.0, x, y])
            .collect();
        let (tris, counters) = ear_clip_impl(&cw_square, true);
        assert!(tris.is_empty());
        assert_eq!(
            counters.fallback_passes, 1,
            "CW ring must be abandoned on the first no-ear pass, not re-scanned"
        );
    }

    /// The reflex-set-pruned ear clipping must emit exactly the reference
    /// triangulation on the existing triangulation fixtures: the L-shape
    /// (non-convex, no holes) and the bridged washer polygon (duplicate
    /// bridge vertices, collinear bridge runs).
    #[test]
    fn pruned_ear_clip_equals_reference_on_known_shapes() {
        let l_shape: Vec<[f64; 5]> = [
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ]
        .iter()
        .map(|&[x, y]| [x, y, 0.0, x, y])
        .collect();
        assert_eq!(
            ear_clip_impl(&l_shape, true).0,
            ear_clip_reference(&l_shape).0
        );

        let washer = wall_polygon(4.0, 4.0, &[[1.0, 1.0, 3.0, 3.0]]);
        assert_eq!(
            ear_clip_impl(&washer, true).0,
            ear_clip_reference(&washer).0
        );
    }

    /// Executable perf spec for the two-stage pruning (a containment-test
    /// count, not a wall-clock assertion): on the profiled pathological
    /// input — a wall whose window holes are bridged into one big polygon —
    /// the pruned path must emit the identical triangulation while doing a
    /// fraction of the reference's vertex containment tests. Measured ratio
    /// at 32 holes is ~4.4×; the 3× bound leaves slack while still catching
    /// either pruning stage silently degrading to a full scan.
    #[test]
    fn wall_with_holes_prunes_containment_tests() {
        let poly = wall_polygon_with_n_holes(32);
        let (tris_pruned, counters_pruned) = ear_clip_impl(&poly, true);
        let (tris_reference, counters_reference) = ear_clip_reference(&poly);
        assert_eq!(
            tris_pruned, tris_reference,
            "pruning changed the triangulation"
        );
        let tests_pruned = counters_pruned.containment_tests;
        let tests_reference = counters_reference.containment_tests;
        assert!(
            tests_pruned * 3 < tests_reference,
            "expected pruning to cut containment tests at least 3×: \
             pruned {tests_pruned} vs reference {tests_reference}"
        );
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

            let poly = build_polygon_with_holes(&outer_3d, &holes_3d, u_ax, v_ax)
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
        for c in mesh.colors.chunks_exact(3) {
            assert!(
                (c[0] - 1.0).abs() < 1e-5 && c[1] < 1e-5 && c[2] < 1e-5,
                "vertex color {c:?} is not the base red"
            );
        }
    }

    /// Property-based equivalence tests (DEVELOPMENT.md rule 3), mirroring
    /// the `resolve` ≡ `resolve_linear` suite in crates/inference: on random
    /// polygons the pruned ear clipping must return exactly the reference
    /// triangulation — the reflex set may only prune, never decide.
    mod props {
        use super::*;
        use proptest::prelude::*;

        /// Random star-shaped polygon: vertices at strictly increasing polar
        /// angles that wind around the origin exactly once, with every
        /// angular gap below π (gaps are drawn from (0.75, 1.0) and
        /// normalized to 2π, so the largest is at most 2π/2.25 < π). Every
        /// ray from the origin then crosses exactly one edge, which makes
        /// the polygon simple and CCW by construction — sorting arbitrary
        /// angles is NOT enough (a polygon whose angles don't wrap the
        /// origin can self-intersect on the closing edge).
        fn arb_star_polygon() -> impl Strategy<Value = Vec<[f64; 5]>> {
            prop::collection::vec((0.75f64..1.0, 0.2f64..10.0), 3..48).prop_map(|pts| {
                let total: f64 = pts.iter().map(|&(gap, _)| gap).sum();
                let mut angle = 0.0f64;
                pts.iter()
                    .map(|&(gap, radius)| {
                        let x = radius * angle.cos();
                        let y = radius * angle.sin();
                        angle += std::f64::consts::TAU * gap / total;
                        [x, y, 0.0, x, y]
                    })
                    .collect()
            })
        }

        /// Doubles a polygon by inserting each edge's midpoint — exactly
        /// collinear runs, the configuration the conservative band in
        /// `must_containment_test` exists for.
        fn subdivide(poly: &[[f64; 5]]) -> Vec<[f64; 5]> {
            let n = poly.len();
            let mut out = Vec::with_capacity(2 * n);
            for i in 0..n {
                let a = poly[i];
                let b = poly[(i + 1) % n];
                out.push(a);
                out.push([
                    (a[0] + b[0]) / 2.0,
                    (a[1] + b[1]) / 2.0,
                    (a[2] + b[2]) / 2.0,
                    (a[3] + b[3]) / 2.0,
                    (a[4] + b[4]) / 2.0,
                ]);
            }
            out
        }

        /// Random wall holes, one per unit cell so they are always disjoint
        /// and strictly inside the outer rectangle.
        fn arb_wall_holes() -> impl Strategy<Value = Vec<[f64; 4]>> {
            prop::collection::vec(
                (0.05f64..0.45, 0.05f64..0.45, 0.55f64..0.95, 0.55f64..0.95),
                1..8,
            )
            .prop_map(|cells| {
                cells
                    .iter()
                    .enumerate()
                    .map(|(i, &(x0, y0, x1, y1))| [i as f64 + x0, y0, i as f64 + x1, y1])
                    .collect()
            })
        }

        /// Lattice wall holes: rectangles snapped to a 0.3 grid, one per
        /// 3×3 cell of a `3n × 3` wall. Quantization makes bridge
        /// corridors, hole corners, and ear diagonals collide EXACTLY —
        /// collinear blockers and coincident crossings that the
        /// continuous-float generators above cannot produce (three random
        /// f64 coordinates are essentially never collinear).
        fn arb_lattice_wall_holes() -> impl Strategy<Value = Vec<[f64; 4]>> {
            prop::collection::vec((1u32..=8, 1u32..=8, 1u32..=8, 1u32..=8), 1..8).prop_map(
                |cells| {
                    cells
                        .iter()
                        .enumerate()
                        .map(|(i, &(a, b, c, d))| {
                            let x0 = 0.3 * a.min(b) as f64;
                            let x1 = 0.3 * (a.max(b) + 1) as f64;
                            let y0 = 0.3 * c.min(d) as f64;
                            let y1 = 0.3 * (c.max(d) + 1) as f64;
                            [3.0 * i as f64 + x0, y0, 3.0 * i as f64 + x1, y1]
                        })
                        .collect()
                },
            )
        }

        proptest! {
            /// Random simple polygons: pruned ≡ reference, exactly.
            #[test]
            fn pruned_ear_clip_equals_reference_on_simple_polygons(
                poly in arb_star_polygon(),
            ) {
                prop_assert_eq!(ear_clip_impl(&poly, true).0, ear_clip_reference(&poly).0);
            }

            /// Random simple polygons with exactly-collinear runs: the
            /// on-the-band vertices must stay in the containment-test set,
            /// so the outputs still match exactly.
            #[test]
            fn pruned_ear_clip_equals_reference_with_collinear_runs(
                poly in arb_star_polygon(),
            ) {
                let poly = subdivide(&poly);
                prop_assert_eq!(ear_clip_impl(&poly, true).0, ear_clip_reference(&poly).0);
            }

            /// Random hole-bridged polygons (duplicate bridge vertices):
            /// pruned ≡ reference, exactly.
            #[test]
            fn pruned_ear_clip_equals_reference_on_bridged_polygons(
                holes in arb_wall_holes(),
            ) {
                let poly = wall_polygon(holes.len() as f64, 1.0, &holes);
                prop_assert_eq!(ear_clip_impl(&poly, true).0, ear_clip_reference(&poly).0);
            }

            /// Lattice-quantized bridged polygons (exact collinearity):
            /// pruned ≡ reference, exactly. This is the class where
            /// reflex-only pruning of the on-diagonal blocker diverged.
            #[test]
            fn pruned_ear_clip_equals_reference_on_lattice_walls(
                holes in arb_lattice_wall_holes(),
            ) {
                let poly = wall_polygon(3.0 * holes.len() as f64, 3.0, &holes);
                prop_assert_eq!(ear_clip_impl(&poly, true).0, ear_clip_reference(&poly).0);
            }

            /// Output honesty on random walls, independent of the
            /// pruned/reference identity (both share the bridging stage, so
            /// a bridge that pinches the ring into a self-crossing fools
            /// the identity tests): every emitted triangle is CCW and the
            /// triangle areas sum exactly to outer-minus-holes.
            #[test]
            fn bridged_walls_are_area_honest(holes in arb_wall_holes()) {
                let (outer, holes_3d) = wall_shape(holes.len() as f64, 1.0, &holes);
                assert_area_honest(&outer, &holes_3d);
            }

            /// The same independent oracle over the exact-collinear
            /// lattice walls.
            #[test]
            fn lattice_walls_are_area_honest(holes in arb_lattice_wall_holes()) {
                let (outer, holes_3d) = wall_shape(3.0 * holes.len() as f64, 3.0, &holes);
                assert_area_honest(&outer, &holes_3d);
            }
        }
    }
}
