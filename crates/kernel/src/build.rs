//! Construction of `Object`s from indexed polygon soup.
//!
//! This is the only public construction path in M0. Invalid input fails with
//! a typed [`TopologyError`]; nothing is repaired silently (DEVELOPMENT.md rule 4).

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use slotmap::SecondaryMap;

use crate::error::TopologyError;
use crate::ids::{EdgeId, FaceId, HalfEdgeId, VertexId};
use crate::material::{FaceMaterial, UvFrame};
use crate::math::{Plane, Point3};
use crate::tol;
use crate::topo::{
    Edge, Face, HalfEdge, Loop, LoopKind, Object, Shell, SurfaceRef, Vertex, WatertightState,
};

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
        Object::from_polygons_impl(positions, faces, None, None, None, tol::PLANE_DIST, None)
    }

    /// Like [`Object::from_polygons`], but assigns each face the material and
    /// per-face [`UvFrame`] at the same index in the parallel `materials` /
    /// `uv_frames` slices ( + the UV-frame extension). Crate-internal: the
    /// boolean assembler uses it so result faces carry their material and UV
    /// frame. Uses the strict [`tol::PLANE_DIST`] gate (kernel-built f64
    /// geometry is exact). Both slices must be parallel to `faces`.
    pub(crate) fn from_polygons_with_materials_and_frames(
        positions: &[Point3],
        faces: &[Vec<usize>],
        materials: &[FaceMaterial],
        uv_frames: &[Option<UvFrame>],
        surfaces: &[Option<SurfaceRef>],
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(
            positions,
            faces,
            Some(materials),
            Some(uv_frames),
            Some(surfaces),
            tol::PLANE_DIST,
            None,
        )
    }

    /// Like [`Object::from_polygons_with_materials_and_frames`], but for
    /// *imported* foreign geometry: uses the wider [`tol::IMPORT_PLANE_DIST`]
    /// planarity gate so f32-quantized SketchUp/COLLADA faces (flat to ~0.1 mm
    /// but past the nanometer [`tol::PLANE_DIST`]) are accepted as the single
    /// planar polygon they represent instead of being skipped. Used by
    /// [`crate::document::Document::ingest`]. All other validation is identical.
    #[allow(dead_code)] // retained as a no-holes import convenience; superseded by
    // from_polygons_with_holes_import for the active ingest path
    pub(crate) fn from_polygons_with_materials_and_frames_import(
        positions: &[Point3],
        faces: &[Vec<usize>],
        materials: &[FaceMaterial],
        uv_frames: &[Option<UvFrame>],
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(
            positions,
            faces,
            Some(materials),
            Some(uv_frames),
            None,
            tol::IMPORT_PLANE_DIST,
            None,
        )
    }

    /// Import path for faces that may carry holes: `holes[i]` is face `i`'s list
    /// of inner-loop index lists (empty = no holes). Validated exactly like the
    /// hole-free import path (manifold edges incl. hole edges, planarity of outer
    /// + hole vertices, no unreferenced vertices) and built at
    ///
    /// `tol::IMPORT_PLANE_DIST`. Used by `crate::document::Document::ingest`.
    pub(crate) fn from_polygons_with_holes_import(
        positions: &[Point3],
        faces: &[Vec<usize>],
        holes: &[Vec<Vec<usize>>],
        materials: &[FaceMaterial],
        uv_frames: &[Option<UvFrame>],
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(
            positions,
            faces,
            Some(materials),
            Some(uv_frames),
            None,
            tol::IMPORT_PLANE_DIST,
            Some(holes),
        )
    }

    fn from_polygons_impl(
        positions: &[Point3],
        faces: &[Vec<usize>],
        materials: Option<&[FaceMaterial]>,
        uv_frames: Option<&[Option<UvFrame>]>,
        surfaces: Option<&[Option<SurfaceRef>]>,
        plane_tol: f64,
        holes: Option<&[Vec<Vec<usize>>]>,
    ) -> Result<Object, TopologyError> {
        if faces.is_empty() || positions.is_empty() {
            return Err(TopologyError::EmptyObject);
        }

        let mut obj = Object::empty();
        // Record the planarity gate this object was built at, so the validator
        // (run on every later mutation and on load) holds it to the same bar —
        // strict for native, wider for imports.
        obj.planarity_tol = plane_tol;

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

        // Directed edge (origin, dest) -> the half-edge traversing it. A
        // BTreeMap (sorted by vertex-id pair), never a HashMap: its iteration
        // order (used to insert edges during twin-pairing below) must be
        // reproducible run-to-run for bit-for-bit determinism.
        let mut directed: BTreeMap<(VertexId, VertexId), HalfEdgeId> = BTreeMap::new();

        let no_holes: Vec<Vec<usize>> = Vec::new();
        for (face_index, polygon) in faces.iter().enumerate() {
            if polygon.len() < 3 {
                return Err(TopologyError::DegenerateFace { face: face_index });
            }
            let face_holes: &[Vec<usize>] = holes
                .and_then(|h| h.get(face_index))
                .map(|v| v.as_slice())
                .unwrap_or(&no_holes);

            // Plane is fit from the outer loop; planarity is checked for the
            // outer loop AND every hole vertex (a hole must lie in the face).
            let pts: Vec<Point3> = polygon
                .iter()
                .map(|&i| {
                    positions
                        .get(i)
                        .copied()
                        .ok_or(TopologyError::InvalidVertexIndex {
                            face: face_index,
                            index: i,
                        })
                })
                .collect::<Result<_, _>>()?;
            let plane = Plane::from_polygon(&pts)
                .map_err(|_| TopologyError::DegenerateFace { face: face_index })?;
            let on_plane = |i: usize| {
                positions
                    .get(i)
                    .is_some_and(|&p| plane.signed_distance(p).abs() <= plane_tol)
            };
            let outer_planar = polygon.iter().all(|&i| on_plane(i));
            let holes_planar = face_holes.iter().flatten().all(|&i| on_plane(i));
            if !outer_planar || !holes_planar {
                return Err(TopologyError::NonPlanarFace { face: face_index });
            }

            // Create the face + its outer loop, then build outer and inner loops.
            let outer_loop_id = obj.loops.insert(Loop {
                face: FaceId::default(),
                first_half_edge: HalfEdgeId::default(),
                kind: LoopKind::Outer,
            });
            let face_id = obj.faces.insert(Face {
                outer_loop: outer_loop_id,
                inner_loops: Vec::new(),
                plane,
                material: materials.and_then(|m| m.get(face_index).copied()).flatten(),
                uv_frame: uv_frames.and_then(|f| f.get(face_index).copied()).flatten(),
                surface: surfaces.and_then(|s| s.get(face_index).copied()).flatten(),
            });
            obj.loops[outer_loop_id].face = face_id;
            build_validated_loop(
                &mut obj,
                &vertex_ids,
                polygon,
                outer_loop_id,
                &mut used,
                &mut directed,
                face_index,
            )?;

            for hole in face_holes {
                let inner_loop_id = obj.loops.insert(Loop {
                    face: face_id,
                    first_half_edge: HalfEdgeId::default(),
                    kind: LoopKind::Inner,
                });
                obj.faces[face_id].inner_loops.push(inner_loop_id);
                build_validated_loop(
                    &mut obj,
                    &vertex_ids,
                    hole,
                    inner_loop_id,
                    &mut used,
                    &mut directed,
                    face_index,
                )?;
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
                            curve: None,
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
                        curve: None,
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
    /// `faces` is a list of `(outer_indices, inner_loops, plane, material,
    /// uv_frame, surface)` tuples where `outer_indices` and each inner-loop
    /// list are index lists into `positions`. All winding must be consistent
    /// (outer CCW, inner CW seen from the face normal) — this function does
    /// not validate; callers are responsible. `material` is the face's
    /// material (`None` = default). `uv_frame` is the per-face affine UV
    /// frame ( ext.; `None` =  `world_size` fallback). `surface` is the
    /// face's analytic surface reference (`None` = plain planar face).
    ///
    /// The public signature/behaviour of [`Object::from_polygons`] is
    /// unchanged; this function is a separate, internal path.
    #[allow(clippy::type_complexity)]
    pub(crate) fn from_faces_with_holes(
        positions: &[Point3],
        faces: &[(
            Vec<usize>,
            Vec<Vec<usize>>,
            Plane,
            FaceMaterial,
            Option<UvFrame>,
            Option<SurfaceRef>,
        )],
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
        // BTreeMap (not HashMap) so the twin-pairing iteration below — which
        // assigns edge ids — is deterministic run-to-run.
        let mut directed: BTreeMap<(VertexId, VertexId), HalfEdgeId> = BTreeMap::new();
        let mut all_face_ids: Vec<FaceId> = Vec::new();

        for (outer_indices, hole_index_lists, plane, material, uv_frame, surface) in faces {
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
                uv_frame: *uv_frame,
                surface: *surface,
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
                            curve: None,
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
                        curve: None,
                    });
                    obj.half_edges[h].edge = edge;
                }
            }
        }

        // Update vertex outgoing pointers to any half-edge originating there.
        // Iterate the half-edge slotmap in its deterministic insertion order so
        // a vertex's cached `outgoing` is reproducible run-to-run; a
        // seed-dependent assignment here previously surfaced as a flaky
        // split_boundary_edge crash.
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

    /// Drop any per-edge circle claim ([`crate::topo::Edge::curve`]) that no
    /// longer describes its (possibly moved) endpoints — the map-or-drop
    /// discipline for `Edge::curve`, mirroring how `Face::surface` is dropped
    /// when a face leaves its chord plane (the true-curves design §4.2).
    ///
    /// Call at the tail of every op that moves a SUBSET of vertices, BEFORE
    /// validation: a subset move can carry a claim's endpoints off its stored
    /// circle (endpoints now `sqrt(r² + d²)` from the fixed center), and a
    /// stale claim panics `check_invariants` in debug or false-refuses at the
    /// release `validate` backstop — a spurious failure of an operation that
    /// has nothing to do with the circle. Dropping degrades gracefully to
    /// flat facets; a stale claim is never kept. The predicate is exactly the
    /// validator's edge-curve check, so anything kept here passes validation.
    pub(crate) fn drop_stale_edge_curves(&mut self) {
        let tol = self.planarity_tol;
        let stale: Vec<crate::ids::EdgeId> = self
            .edges
            .iter()
            .filter_map(|(id, edge)| {
                let g = edge.curve?;
                let h = self.half_edges[edge.half_edge];
                let a = self.vertices[h.origin].position;
                let b = self.vertices[self.half_edges[h.next].origin].position;
                let holds = g.radius.is_finite()
                    && g.radius > crate::tol::POINT_MERGE
                    && ((a - g.center).length() - g.radius).abs() <= tol
                    && ((b - g.center).length() - g.radius).abs() <= tol;
                (!holds).then_some(id)
            })
            .collect();
        for id in stale {
            self.edges[id].curve = None;
        }
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
    directed: &mut BTreeMap<(VertexId, VertexId), HalfEdgeId>,
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

/// Build one validated loop (outer or inner) for [`Object::from_polygons_impl`]:
/// resolves + range-checks indices, rejects a repeated index (`DegenerateFace`),
/// marks vertices used, wires the half-edge ring (next/prev/outgoing + the loop's
/// first half-edge), and registers each directed edge — returning
/// `NonManifoldEdge` if one is already taken. `loop_id`'s `Loop` and the owning
/// `Face` must already exist. Shared by the outer loop and every hole loop so
/// holes get identical validation (decision: import holes, Stage 2).
fn build_validated_loop(
    obj: &mut Object,
    vertex_ids: &[VertexId],
    indices: &[usize],
    loop_id: crate::ids::LoopId,
    used: &mut [bool],
    directed: &mut BTreeMap<(VertexId, VertexId), HalfEdgeId>,
    face_index: usize,
) -> Result<(), TopologyError> {
    if indices.len() < 3 {
        return Err(TopologyError::DegenerateFace { face: face_index });
    }
    let mut ids = Vec::with_capacity(indices.len());
    for &index in indices {
        let &vid = vertex_ids
            .get(index)
            .ok_or(TopologyError::InvalidVertexIndex {
                face: face_index,
                index,
            })?;
        ids.push(vid);
        used[index] = true;
    }
    let mut deduped = indices.to_vec();
    deduped.sort_unstable();
    deduped.dedup();
    if deduped.len() != indices.len() {
        return Err(TopologyError::DegenerateFace { face: face_index });
    }

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
                    from: indices[k],
                    to: indices[(k + 1) % n],
                });
            }
            Entry::Vacant(slot) => {
                slot.insert(he_ids[k]);
            }
        }
    }
    Ok(())
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

    /// A face flat within `IMPORT_PLANE_DIST` but past `PLANE_DIST` (f32-noise
    /// scale) is rejected by the strict native path but accepted by the import
    /// path, which records the wider tolerance so the validator agrees (#35).
    #[test]
    fn near_planar_face_strict_rejects_import_accepts() {
        let (mut positions, faces) = unit_box();
        // Bend the top quad by 1e-5 m: >> PLANE_DIST (1e-9), << IMPORT_PLANE_DIST (1e-3).
        positions[6] = Point3::new(1.0, 1.0, 1.0 + 1e-5);

        // Strict native construction rejects it.
        let err = Object::from_polygons(&positions, &faces).unwrap_err();
        assert!(
            matches!(err, TopologyError::NonPlanarFace { .. }),
            "strict path must reject, got {err:?}"
        );

        // Import construction accepts it and carries the wider tolerance.
        let mats = vec![None; faces.len()];
        let frames = vec![None; faces.len()];
        let obj = Object::from_polygons_with_materials_and_frames_import(
            &positions, &faces, &mats, &frames,
        )
        .expect("import path accepts a near-planar face");
        assert_eq!(obj.planarity_tol, crate::tol::IMPORT_PLANE_DIST);
        // The validator agrees (would panic in debug otherwise).
        obj.validate()
            .expect("import object validates at its own tolerance");
    }

    /// A face with a hole builds one Face carrying one inner loop; all hole
    /// vertices count as used; the single face is an open shell (Stage 2 holes).
    #[test]
    fn import_face_with_hole_builds_inner_loop() {
        // 4×4 quad in z=0 with a centered 2×2 square hole.
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
        ];
        let faces = vec![vec![0, 1, 2, 3]];
        let holes = vec![vec![vec![4, 7, 6, 5]]]; // inner wound opposite the outer
        let obj =
            Object::from_polygons_with_holes_import(&positions, &faces, &holes, &[None], &[None])
                .expect("holed face builds");
        assert_eq!(obj.faces().len(), 1);
        let face = obj.faces().values().next().unwrap();
        assert_eq!(face.inner_loops.len(), 1, "one hole");
        obj.validate().expect("validates"); // no UnreferencedVertex ⇒ hole verts used
        assert_eq!(obj.watertight(), WatertightState::Open);
    }

    /// A hole vertex off the face plane is rejected like any non-planar face.
    #[test]
    fn import_hole_vertex_off_plane_is_rejected() {
        let mut positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
        ];
        positions[4] = Point3::new(1.0, 1.0, 0.5); // lift a hole vertex off-plane
        let err = Object::from_polygons_with_holes_import(
            &positions,
            &[vec![0, 1, 2, 3]],
            &[vec![vec![4, 7, 6, 5]]],
            &[None],
            &[None],
        )
        .unwrap_err();
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
