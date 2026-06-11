//! Kernel topology -> renderer-agnostic buffers.
//!
//! Isolates the kernel from the renderer (ARCHITECTURE.md): the output is plain
//! f32/u32 arrays a WebGL2 viewport can upload directly. Flat-shaded look:
//! vertices are duplicated per face so each face keeps its own normal, and
//! unique edges come out as line segments for the SketchUp-style display.
//!
//! M0 scope: convex planar faces, fan triangulation. Faces with holes are a
//! typed error, not a wrong picture.

use kernel::{FaceId, Object};

/// Tessellation failed; the Object uses topology this version cannot
/// triangulate honestly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TessellateError {
    /// Face has inner loops (holes); hole triangulation lands post-M0.
    HolesUnsupported {
        /// The offending face.
        face: FaceId,
    },
    /// Face has fewer than 3 boundary vertices (kernel invariant breach).
    DegenerateFace {
        /// The offending face.
        face: FaceId,
    },
}

impl std::fmt::Display for TessellateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TessellateError::HolesUnsupported { face } => {
                write!(
                    f,
                    "face {face:?} has holes, which M0 tessellation does not support"
                )
            }
            TessellateError::DegenerateFace { face } => {
                write!(f, "face {face:?} has fewer than 3 vertices")
            }
        }
    }
}

impl std::error::Error for TessellateError {}

/// Flat-shaded render buffers for one Object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderMesh {
    /// Triangle vertex positions, xyz per vertex, duplicated per face.
    pub positions: Vec<f32>,
    /// Per-vertex normals (constant across each face).
    pub normals: Vec<f32>,
    /// Triangle indices into `positions`.
    pub indices: Vec<u32>,
    /// Line-segment endpoints (xyz pairs), one segment per unique edge.
    pub edge_positions: Vec<f32>,
}

/// Tessellates an Object into flat-shaded triangle and edge-line buffers.
///
/// Faces are fan-triangulated in loop order, so triangles wind
/// counter-clockwise seen from outside — front faces under the WebGL default.
pub fn tessellate(object: &Object) -> Result<RenderMesh, TessellateError> {
    let mut mesh = RenderMesh::default();

    for (face_id, face) in object.faces() {
        if !face.inner_loops.is_empty() {
            return Err(TessellateError::HolesUnsupported { face: face_id });
        }
        let normal = face.plane.normal();
        let base = (mesh.positions.len() / 3) as u32;
        let mut corner_count: u32 = 0;
        for p in object.loop_positions(face.outer_loop) {
            mesh.positions.extend([p.x as f32, p.y as f32, p.z as f32]);
            mesh.normals
                .extend([normal.x as f32, normal.y as f32, normal.z as f32]);
            corner_count += 1;
        }
        if corner_count < 3 {
            return Err(TessellateError::DegenerateFace { face: face_id });
        }
        for i in 1..corner_count - 1 {
            mesh.indices.extend([base, base + i, base + i + 1]);
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::Point3;

    /// f32 round-off allowance for unit-length checks in tests only; kernel
    /// geometric tolerances live in `kernel::tol`.
    const UNIT_LENGTH_TOL_F32: f32 = 1e-6;

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
    fn tetrahedron_buffers_have_expected_shape() {
        let mesh = tessellate(&Object::tetrahedron()).unwrap();
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
        let mesh = tessellate(&unit_cube()).unwrap();
        // 6 quads -> 24 duplicated corners, 12 triangles, 12 unique edges.
        assert_eq!(mesh.positions.len(), 24 * 3);
        assert_eq!(mesh.indices.len(), 12 * 3);
        assert_eq!(mesh.edge_positions.len(), 12 * 2 * 3);
    }

    #[test]
    fn normals_are_unit_length() {
        let mesh = tessellate(&Object::tetrahedron()).unwrap();
        for n in mesh.normals.chunks_exact(3) {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!(
                (len - 1.0).abs() < UNIT_LENGTH_TOL_F32,
                "normal length {len}"
            );
        }
    }
}
