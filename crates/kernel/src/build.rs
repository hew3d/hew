//! Construction of `Object`s from indexed polygon soup.
//!
//! This is the only public construction path in M0. Invalid input fails with
//! a typed [`TopologyError`]; nothing is repaired silently (DEVELOPMENT.md rule 4).

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use slotmap::SecondaryMap;

use crate::error::TopologyError;
use crate::ids::{EdgeId, FaceId, HalfEdgeId, VertexId};
use crate::material::FaceMaterial;
use crate::math::{Plane, Point3};
use crate::tol;
use crate::topo::{Edge, Face, HalfEdge, Loop, LoopKind, Object, Shell, Vertex, WatertightState};

impl Object {
    /// Builds an Object from positions and faces given as CCW-wound (seen
    /// from outside) index lists into `positions`.
    ///
    /// Rejects, with typed errors: empty input, out-of-range or repeated
    /// indices, degenerate or non-planar polygons, unreferenced vertices, and
    /// directed edges traversed twice (more than two faces on an edge, or
    /// inconsistent winding). Boundary edges are allowed: the result is then
    /// flagged [`WatertightState::Open`].
    pub fn from_polygons(
        positions: &[Point3],
        faces: &[Vec<usize>],
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(positions, faces, None)
    }

    /// Like [`Object::from_polygons`], but assigns each face the material at the
    /// same index in `materials`. Crate-internal: the boolean assembler
    /// uses it so result faces inherit their source face's material. `materials`
    /// must be parallel to `faces`.
    pub(crate) fn from_polygons_with_materials(
        positions: &[Point3],
        faces: &[Vec<usize>],
        materials: &[FaceMaterial],
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(positions, faces, Some(materials))
    }

    fn from_polygons_impl(
        positions: &[Point3],
        faces: &[Vec<usize>],
        materials: Option<&[FaceMaterial]>,
    ) -> Result<Object, TopologyError> {
        if faces.is_empty() || positions.is_empty() {
            return Err(TopologyError::EmptyObject);
        }

        let mut obj = Object::empty();

        let vertex_ids: Vec<VertexId> = positions
            .iter()
            .map(|&p| {
                obj.vertices.insert(Vertex {
                    position: p,
                    outgoing: HalfEdgeId::default(),
                })
            })
            .collect();
        let mut used = vec![false; positions.len()];

        // Directed edge (origin, dest) -> the half-edge traversing it.
        let mut directed: HashMap<(VertexId, VertexId), HalfEdgeId> = HashMap::new();

        for (face_index, polygon) in faces.iter().enumerate() {
            if polygon.len() < 3 {
                return Err(TopologyError::DegenerateFace { face: face_index });
            }
            let mut ids = Vec::with_capacity(polygon.len());
            for &index in polygon {
                let &vid = vertex_ids
                    .get(index)
                    .ok_or(TopologyError::InvalidVertexIndex {
                        face: face_index,
                        index,
                    })?;
                ids.push(vid);
                used[index] = true;
            }
            let mut deduped = polygon.clone();
            deduped.sort_unstable();
            deduped.dedup();
            if deduped.len() != polygon.len() {
                return Err(TopologyError::DegenerateFace { face: face_index });
            }

            let pts: Vec<Point3> = polygon.iter().map(|&i| positions[i]).collect();
            let plane = Plane::from_polygon(&pts)
                .map_err(|_| TopologyError::DegenerateFace { face: face_index })?;
            if pts
                .iter()
                .any(|&p| plane.signed_distance(p).abs() > tol::PLANE_DIST)
            {
                return Err(TopologyError::NonPlanarFace { face: face_index });
            }

            let loop_id = obj.loops.insert(Loop {
                face: FaceId::default(),
                first_half_edge: HalfEdgeId::default(),
                kind: LoopKind::Outer,
            });
            let face_id = obj.faces.insert(Face {
                outer_loop: loop_id,
                inner_loops: Vec::new(),
                plane,
                material: materials.and_then(|m| m.get(face_index).copied()).flatten(),
            });
            obj.loops[loop_id].face = face_id;

            let he_ids: Vec<HalfEdgeId> = ids
                .iter()
                .map(|&origin| {
                    obj.half_edges.insert(HalfEdge {
                        origin,
                        twin: None,
                        next: HalfEdgeId::default(),
                        prev: HalfEdgeId::default(),
                        edge: EdgeId::default(),
                        loop_id,
                    })
                })
                .collect();
            let n = he_ids.len();
            for k in 0..n {
                let h = he_ids[k];
                obj.half_edges[h].next = he_ids[(k + 1) % n];
                obj.half_edges[h].prev = he_ids[(k + n - 1) % n];
                obj.vertices[ids[k]].outgoing = h;
            }
            obj.loops[loop_id].first_half_edge = he_ids[0];

            for k in 0..n {
                let key = (ids[k], ids[(k + 1) % n]);
                match directed.entry(key) {
                    Entry::Occupied(_) => {
                        return Err(TopologyError::NonManifoldEdge {
                            from: polygon[k],
                            to: polygon[(k + 1) % n],
                        });
                    }
                    Entry::Vacant(slot) => {
                        slot.insert(he_ids[k]);
                    }
                }
            }
        }

        if let Some(index) = used.iter().position(|&u| !u) {
            return Err(TopologyError::UnreferencedVertex { index });
        }

        // Pair twins: the directed edge (a, b) matches (b, a) on the
        // neighbouring face. Unpaired half-edges sit on the mesh boundary.
        for (&(a, b), &h) in &directed {
            match directed.get(&(b, a)) {
                Some(&t) => {
                    // Each unordered pair is processed once.
                    if a < b {
                        let edge = obj.edges.insert(Edge {
                            half_edge: h,
                            twin_half_edge: Some(t),
                        });
                        obj.half_edges[h].edge = edge;
                        obj.half_edges[t].edge = edge;
                        obj.half_edges[h].twin = Some(t);
                        obj.half_edges[t].twin = Some(h);
                    }
                }
                None => {
                    let edge = obj.edges.insert(Edge {
                        half_edge: h,
                        twin_half_edge: None,
                    });
                    obj.half_edges[h].edge = edge;
                }
            }
        }

        let all_faces: Vec<FaceId> = obj.faces.keys().collect();
        obj.shells.insert(Shell { faces: all_faces });

        obj.watertight = if obj.half_edges.values().all(|he| he.twin.is_some()) {
            WatertightState::Watertight
        } else {
            WatertightState::Open
        };

        obj.check_invariants();
        Ok(obj)
    }

    /// Crate-internal construction path that supports faces with inner loops
    /// (holes). Used by [`Object::from_extrusion`].
    ///
    /// `faces` is a list of `(outer_indices, inner_loops, plane, material)`
    /// tuples where `outer_indices` and each inner-loop list are index lists
    /// into `positions`. All winding must be consistent (outer CCW, inner CW
    /// seen from the face normal) — this function does not validate; callers are
    /// responsible. `material` is the face's material (`None` = default).
    ///
    /// The public signature/behaviour of [`Object::from_polygons`] is
    /// unchanged; this function is a separate, internal path.
    #[allow(clippy::type_complexity)]
    pub(crate) fn from_faces_with_holes(
        positions: &[Point3],
        faces: &[(Vec<usize>, Vec<Vec<usize>>, Plane, FaceMaterial)],
    ) -> Object {
        let mut obj = Object::empty();

        // Insert all vertices.
        let vertex_ids: Vec<VertexId> = positions
            .iter()
            .map(|&p| {
                obj.vertices.insert(Vertex {
                    position: p,
                    // Will be set below.
                    outgoing: HalfEdgeId::default(),
                })
            })
            .collect();

        // Directed half-edge map for twin pairing: (origin, dest) -> HalfEdgeId.
        let mut directed: HashMap<(VertexId, VertexId), HalfEdgeId> = HashMap::new();
        let mut all_face_ids: Vec<FaceId> = Vec::new();

        for (outer_indices, hole_index_lists, plane, material) in faces {
            // ---- outer loop ----
            let outer_loop_id = obj.loops.insert(Loop {
                face: FaceId::default(),
                first_half_edge: HalfEdgeId::default(),
                kind: LoopKind::Outer,
            });

            let face_id = obj.faces.insert(Face {
                outer_loop: outer_loop_id,
                inner_loops: Vec::new(),
                plane: *plane,
                material: *material,
            });
            obj.loops[outer_loop_id].face = face_id;
            all_face_ids.push(face_id);

            build_loop(
                &mut obj,
                &vertex_ids,
                outer_indices,
                outer_loop_id,
                &mut directed,
            );

            // ---- inner loops (holes) ----
            for hole_indices in hole_index_lists {
                let inner_loop_id = obj.loops.insert(Loop {
                    face: face_id,
                    first_half_edge: HalfEdgeId::default(),
                    kind: LoopKind::Inner,
                });
                obj.faces[face_id].inner_loops.push(inner_loop_id);

                build_loop(
                    &mut obj,
                    &vertex_ids,
                    hole_indices,
                    inner_loop_id,
                    &mut directed,
                );
            }
        }

        // Pair twins.
        for (&(a, b), &h) in &directed {
            match directed.get(&(b, a)) {
                Some(&t) => {
                    if a < b {
                        let edge = obj.edges.insert(Edge {
                            half_edge: h,
                            twin_half_edge: Some(t),
                        });
                        obj.half_edges[h].edge = edge;
                        obj.half_edges[t].edge = edge;
                        obj.half_edges[h].twin = Some(t);
                        obj.half_edges[t].twin = Some(h);
                    }
                }
                None => {
                    let edge = obj.edges.insert(Edge {
                        half_edge: h,
                        twin_half_edge: None,
                    });
                    obj.half_edges[h].edge = edge;
                }
            }
        }

        // Update vertex outgoing pointers to any half-edge originating there.
        // Iterate the half-edge slotmap (deterministic insertion order), NOT the
        // `directed` HashMap: a vertex's cached `outgoing` must not depend on the
        // per-process HashMap seed, or kernel output varies run-to-run (this seed
        // dependence previously surfaced as a flaky split_boundary_edge crash).
        for (h, he) in obj.half_edges.iter() {
            obj.vertices[he.origin].outgoing = h;
        }

        obj.shells.insert(Shell {
            faces: all_face_ids,
        });

        obj.watertight = if obj.half_edges.values().all(|he| he.twin.is_some()) {
            WatertightState::Watertight
        } else {
            WatertightState::Open
        };

        obj
    }

    /// Exports the mesh back to indexed polygon soup (outer loops only; the
    /// M0 builder never creates holes).
    pub fn to_polygons(&self) -> (Vec<Point3>, Vec<Vec<usize>>) {
        let mut index_of: SecondaryMap<VertexId, usize> = SecondaryMap::new();
        let mut positions = Vec::with_capacity(self.vertices.len());
        for (vid, vertex) in &self.vertices {
            index_of.insert(vid, positions.len());
            positions.push(vertex.position);
        }
        let mut polygons = Vec::with_capacity(self.faces.len());
        for shell in self.shells.values() {
            for &face in &shell.faces {
                let outer = self.faces[face].outer_loop;
                polygons.push(
                    self.loop_half_edges(outer)
                        .map(|h| index_of[self.half_edges[h].origin])
                        .collect(),
                );
            }
        }
        (positions, polygons)
    }

    /// A single unit right triangle in the XY plane: the smallest valid open
    /// Object.
    pub fn triangle() -> Object {
        Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            &[vec![0, 1, 2]],
        )
        .expect("unit triangle is valid by construction")
    }

    /// A unit corner tetrahedron: the smallest watertight Object.
    pub fn tetrahedron() -> Object {
        Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
            ],
            &[
                vec![0, 2, 1], // bottom, outward -z
                vec![0, 3, 2], // outward -x
                vec![0, 1, 3], // outward -y
                vec![1, 2, 3], // slanted, outward (1,1,1)
            ],
        )
        .expect("unit tetrahedron is valid by construction")
    }
}

/// Inserts half-edges for `indices` into `loop_id`, wiring next/prev, and
/// records (origin, dest) → HalfEdgeId in `directed`.  Edge IDs and twins are
/// set by the caller after all faces have been processed.
fn build_loop(
    obj: &mut Object,
    vertex_ids: &[VertexId],
    indices: &[usize],
    loop_id: crate::ids::LoopId,
    directed: &mut HashMap<(VertexId, VertexId), HalfEdgeId>,
) {
    let ids: Vec<VertexId> = indices.iter().map(|&i| vertex_ids[i]).collect();
    let n = ids.len();
    let he_ids: Vec<HalfEdgeId> = ids
        .iter()
        .map(|&origin| {
            obj.half_edges.insert(HalfEdge {
                origin,
                twin: None,
                next: HalfEdgeId::default(),
                prev: HalfEdgeId::default(),
                edge: EdgeId::default(),
                loop_id,
            })
        })
        .collect();
    for k in 0..n {
        let h = he_ids[k];
        obj.half_edges[h].next = he_ids[(k + 1) % n];
        obj.half_edges[h].prev = he_ids[(k + n - 1) % n];
    }
    obj.loops[loop_id].first_half_edge = he_ids[0];
    for k in 0..n {
        directed.insert((ids[k], ids[(k + 1) % n]), he_ids[k]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> (Vec<Point3>, Vec<Vec<usize>>) {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 1.0, 1.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1], // bottom, -z
            vec![4, 5, 6, 7], // top, +z
            vec![0, 1, 5, 4], // -y
            vec![1, 2, 6, 5], // +x
            vec![2, 3, 7, 6], // +y
            vec![3, 0, 4, 7], // -x
        ];
        (positions, faces)
    }

    #[test]
    fn triangle_is_valid_and_open() {
        let tri = Object::triangle();
        tri.validate().unwrap();
        assert_eq!(tri.watertight(), WatertightState::Open);
        assert_eq!(tri.vertices().len(), 3);
        assert_eq!(tri.half_edges().len(), 3);
        assert_eq!(tri.edges().len(), 3);
        assert_eq!(tri.faces().len(), 1);
    }

    #[test]
    fn tetrahedron_is_watertight() {
        let tet = Object::tetrahedron();
        tet.validate().unwrap();
        assert_eq!(tet.watertight(), WatertightState::Watertight);
        assert_eq!(tet.vertices().len(), 4);
        assert_eq!(tet.edges().len(), 6);
        assert_eq!(tet.faces().len(), 4);
        assert_eq!(tet.half_edges().len(), 12);
        // Euler characteristic of a sphere-like solid: V - E + F = 2.
        let euler =
            tet.vertices().len() as i64 - tet.edges().len() as i64 + tet.faces().len() as i64;
        assert_eq!(euler, 2);
    }

    #[test]
    fn box_with_quad_faces_is_watertight() {
        let (positions, faces) = unit_box();
        let cube = Object::from_polygons(&positions, &faces).unwrap();
        cube.validate().unwrap();
        assert_eq!(cube.watertight(), WatertightState::Watertight);
        assert_eq!(cube.vertices().len(), 8);
        assert_eq!(cube.edges().len(), 12);
        assert_eq!(cube.faces().len(), 6);
    }

    #[test]
    fn missing_face_yields_open_object() {
        let (positions, mut faces) = unit_box();
        faces.pop();
        let open = Object::from_polygons(&positions, &faces).unwrap();
        open.validate().unwrap();
        assert_eq!(open.watertight(), WatertightState::Open);
    }

    #[test]
    fn duplicate_face_is_rejected_as_non_manifold() {
        let (positions, mut faces) = unit_box();
        faces.push(faces[0].clone());
        let err = Object::from_polygons(&positions, &faces).unwrap_err();
        assert!(
            matches!(err, TopologyError::NonManifoldEdge { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn flipped_winding_is_rejected() {
        let (positions, mut faces) = unit_box();
        faces[1].reverse();
        let err = Object::from_polygons(&positions, &faces).unwrap_err();
        assert!(
            matches!(err, TopologyError::NonManifoldEdge { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn non_planar_face_is_rejected() {
        let (mut positions, faces) = unit_box();
        positions[6] = Point3::new(1.0, 1.0, 1.5); // bend the top quad
        let err = Object::from_polygons(&positions, &faces).unwrap_err();
        assert!(
            matches!(err, TopologyError::NonPlanarFace { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn out_of_range_index_is_rejected() {
        let err = Object::from_polygons(
            &[
                Point3::ORIGIN,
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            &[vec![0, 1, 9]],
        )
        .unwrap_err();
        assert_eq!(err, TopologyError::InvalidVertexIndex { face: 0, index: 9 });
    }

    #[test]
    fn unreferenced_vertex_is_rejected() {
        let err = Object::from_polygons(
            &[
                Point3::ORIGIN,
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(5.0, 5.0, 5.0),
            ],
            &[vec![0, 1, 2]],
        )
        .unwrap_err();
        assert_eq!(err, TopologyError::UnreferencedVertex { index: 3 });
    }

    #[test]
    fn empty_input_is_rejected() {
        let err = Object::from_polygons(&[], &[]).unwrap_err();
        assert_eq!(err, TopologyError::EmptyObject);
    }

    #[test]
    fn to_polygons_roundtrips() {
        let (positions, faces) = unit_box();
        let cube = Object::from_polygons(&positions, &faces).unwrap();
        let (out_positions, out_faces) = cube.to_polygons();
        let rebuilt = Object::from_polygons(&out_positions, &out_faces).unwrap();
        rebuilt.validate().unwrap();
        assert_eq!(rebuilt.watertight(), cube.watertight());
        assert_eq!(rebuilt.vertices().len(), cube.vertices().len());
        assert_eq!(rebuilt.edges().len(), cube.edges().len());
        assert_eq!(rebuilt.faces().len(), cube.faces().len());
    }
}
