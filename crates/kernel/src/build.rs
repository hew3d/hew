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

    /// [`Object::from_polygons_with_holes_import`] with no holes, materials
    /// or UV frames, for the executable specs (an imported mesh built at
    /// [`tol::IMPORT_PLANE_DIST`]). Test support only.
    #[doc(hidden)]
    pub fn from_polygons_import_for_test(
        positions: &[Point3],
        faces: &[Vec<usize>],
    ) -> Result<Object, TopologyError> {
        let n = faces.len();
        Object::from_polygons_with_holes_import(
            positions,
            faces,
            &vec![Vec::new(); n],
            &vec![None; n],
            &vec![None; n],
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

    /// Native faces with holes at the strict [`tol::PLANE_DIST`] gate:
    /// `holes[i]` is face `i`'s inner-loop index lists, wound against its
    /// outer loop. Validated exactly like [`Object::from_polygons`]. Used
    /// by the cells search (wireframe faces §5.3), whose faces are drawn,
    /// not imported.
    pub(crate) fn from_polygons_with_holes(
        positions: &[Point3],
        faces: &[Vec<usize>],
        holes: &[Vec<Vec<usize>>],
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(
            positions,
            faces,
            None,
            None,
            None,
            tol::PLANE_DIST,
            Some(holes),
        )
    }

    /// The builder with every column and a caller-chosen planarity gate:
    /// what Mend rebuilds an object's polygon soup through
    /// (`repair::mend`), at the object's own `planarity_tol`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_polygons_full(
        positions: &[Point3],
        faces: &[Vec<usize>],
        holes: &[Vec<Vec<usize>>],
        materials: &[FaceMaterial],
        uv_frames: &[Option<UvFrame>],
        surfaces: &[Option<SurfaceRef>],
        plane_tol: f64,
    ) -> Result<Object, TopologyError> {
        Object::from_polygons_impl(
            positions,
            faces,
            Some(materials),
            Some(uv_frames),
            Some(surfaces),
            plane_tol,
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
        // Storage sized to the recipe up front: an element map that grew by
        // doubling would end up to twice the size its elements need, and
        // an imported document holds millions of them.
        let rings: usize = faces.iter().map(Vec::len).sum::<usize>()
            + holes
                .map(|h| h.iter().flatten().map(Vec::len).sum::<usize>())
                .unwrap_or(0);
        let inner: usize = holes.map(|h| h.iter().map(Vec::len).sum()).unwrap_or(0);
        obj.vertices.reserve(positions.len());
        obj.half_edges.reserve(rings);
        obj.loops.reserve(faces.len() + inner);
        obj.faces.reserve(faces.len());

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
            // The plane a saved file recomputes for this face must exist too
            // (the validator holds every live face to it).
            if crate::serialize::outer_ring_plane(&pts).is_err() {
                return Err(TopologyError::DegenerateFace { face: face_index });
            }
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
                back_material: None,
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
        // One edge per unordered pair: a twinned pair counts once.
        obj.edges.reserve(
            directed
                .keys()
                .filter(|&&(a, b)| a < b || !directed.contains_key(&(b, a)))
                .count(),
        );
        for (&(a, b), &h) in &directed {
            match directed.get(&(b, a)) {
                Some(&t) => {
                    // Each unordered pair is processed once.
                    if a < b {
                        let edge = obj.edges.insert(Edge {
                            half_edge: h,
                            twin_half_edge: Some(t),
                            curve: None,
                            soft: false,
                            polygon: None,
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
                        soft: false,
                        polygon: None,
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
    /// Paints the backs of a freshly built object's faces, in the order
    /// they were built (a fresh object's faces iterate in insertion order).
    pub(crate) fn set_built_back_materials(&mut self, backs: &[FaceMaterial]) {
        for ((_, f), &b) in self.faces.iter_mut().zip(backs) {
            f.back_material = b;
        }
    }

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
                back_material: None,
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
                            soft: false,
                            polygon: None,
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
                        soft: false,
                        polygon: None,
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
    /// The map half of map-or-drop for [`Edge::curve`](crate::topo::Edge::curve)
    /// under a rigid translate of a vertex subset: an edge whose endpoints were
    /// both carried by `sweep` is a chord of the same circle moved by `sweep`,
    /// so its claim moves with it. "Carried" is proven per endpoint, not
    /// inferred: the vertex is in `moved`, existed in `before` (the object
    /// prior to the op), and now sits at its `before` position plus `sweep`.
    /// A vertex the op left behind (a wall's bottom edge keeps the moved
    /// face's original ids at their original positions) or re-created fails
    /// the proof. The translated circle must also hold for both endpoints.
    /// Anything not mapped is left for
    /// [`drop_stale_edge_curves`](Object::drop_stale_edge_curves) to judge, so
    /// this never writes a claim the validator would reject.
    pub(crate) fn map_translated_edge_curves(
        &mut self,
        before: &Object,
        moved: &std::collections::BTreeSet<VertexId>,
        sweep: crate::math::Vec3,
    ) {
        let tol = self.planarity_tol;
        let mapped: Vec<(EdgeId, crate::sketch::CurveGeom)> = self
            .edges
            .iter()
            .filter_map(|(id, edge)| {
                let g = edge.curve?;
                let h = self.half_edges[edge.half_edge];
                let (va, vb) = (h.origin, self.half_edges[h.next].origin);
                let carried = |v: VertexId| {
                    moved.contains(&v)
                        && before.vertices.get(v).is_some_and(|b| {
                            (self.vertices[v].position - (b.position + sweep)).length() <= tol
                        })
                };
                if !(carried(va) && carried(vb)) {
                    return None;
                }
                let center = g.center + sweep;
                let on = |v: VertexId| {
                    ((self.vertices[v].position - center).length() - g.radius).abs() <= tol
                };
                (on(va) && on(vb)).then_some((
                    id,
                    crate::sketch::CurveGeom {
                        center,
                        radius: g.radius,
                    },
                ))
            })
            .collect();
        for (id, g) in mapped {
            self.edges[id].curve = Some(g);
        }
        // A regular polygon's side carried with the sweep keeps its mark,
        // moved with it (its pieces need not end on the circumcircle).
        let polys: Vec<(EdgeId, crate::sketch::CurveGeom)> = self
            .edges
            .iter()
            .filter_map(|(id, edge)| {
                let g = edge.polygon?;
                let h = self.half_edges[edge.half_edge];
                let (va, vb) = (h.origin, self.half_edges[h.next].origin);
                let carried = |v: VertexId| {
                    moved.contains(&v)
                        && before.vertices.get(v).is_some_and(|b| {
                            (self.vertices[v].position - (b.position + sweep)).length() <= tol
                        })
                };
                (carried(va) && carried(vb)).then_some((
                    id,
                    crate::sketch::CurveGeom {
                        center: g.center + sweep,
                        radius: g.radius,
                    },
                ))
            })
            .collect();
        for (id, g) in polys {
            self.edges[id].polygon = Some(g);
        }
    }

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
        // A polygon side (or a piece of one) still lies along a regular
        // polygon's side of its circumcircle.
        let stale: Vec<crate::ids::EdgeId> = self
            .edges
            .iter()
            .filter_map(|(id, edge)| {
                let g = edge.polygon?;
                let h = self.half_edges[edge.half_edge];
                let a = self.vertices[h.origin].position;
                let b = self.vertices[self.half_edges[h.next].origin].position;
                g.polygon_side_count(a, b).is_none().then_some(id)
            })
            .collect();
        for id in stale {
            self.edges[id].polygon = None;
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

impl Object {
    /// Adopt an importer's claims about this freshly built Object's edges
    /// and faces ([`crate::import::MeshExtra`]) wherever they hold, by the
    /// same predicates the validator enforces — a claim the geometry
    /// contradicts is dropped (claims are metadata; dropping one is not
    /// geometry repair). Endpoints match vertices within the Object's
    /// planarity tolerance. Returns how many claims did not hold.
    pub(crate) fn adopt_import_claims(&mut self, extra: &crate::import::MeshExtra) -> usize {
        if extra.soft_edges.is_empty() && extra.edge_curves.is_empty() && extra.surfaces.is_empty()
        {
            return 0;
        }
        let tol = self.planarity_tol;
        // Undirected edge by its two vertices.
        let mut by_pair: std::collections::BTreeMap<(VertexId, VertexId), EdgeId> =
            std::collections::BTreeMap::new();
        for (eid, e) in &self.edges {
            let he = &self.half_edges[e.half_edge];
            let a = he.origin;
            let b = self.half_edges[he.next].origin;
            by_pair.insert(if a < b { (a, b) } else { (b, a) }, eid);
        }
        // Vertices bucketed on a grid of the tolerance, so a lookup checks
        // its own and the neighboring cells.
        let cell = |p: Point3| -> [i64; 3] {
            // Clamped so a neighboring cell of an absurd coordinate can
            // still be named without overflowing.
            [p.x, p.y, p.z].map(|c| (c / tol.max(1e-12)).floor().clamp(-1e15, 1e15) as i64)
        };
        let mut grid: std::collections::BTreeMap<[i64; 3], Vec<VertexId>> =
            std::collections::BTreeMap::new();
        for (vid, v) in &self.vertices {
            grid.entry(cell(v.position)).or_default().push(vid);
        }
        let find = |p: Point3| -> Option<VertexId> {
            let c = cell(p);
            let mut best: Option<(f64, VertexId)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &v in grid
                            .get(&[c[0] + dx, c[1] + dy, c[2] + dz])
                            .into_iter()
                            .flatten()
                        {
                            let d = (self.vertices[v].position - p).length();
                            if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
                                best = Some((d, v));
                            }
                        }
                    }
                }
            }
            best.map(|(_, v)| v)
        };
        let edge_of = |[p, q]: [Point3; 2]| -> Option<EdgeId> {
            let (a, b) = (find(p)?, find(q)?);
            by_pair.get(&if a < b { (a, b) } else { (b, a) }).copied()
        };
        let mut dropped = 0;
        for &pair in &extra.soft_edges {
            match edge_of(pair) {
                // A soft edge is a seam between two faces.
                Some(e) if self.edges[e].twin_half_edge.is_some() => self.edges[e].soft = true,
                _ => dropped += 1,
            }
        }
        for &(pair, g) in &extra.edge_curves {
            let on = |p: Point3| ((p - g.center).length() - g.radius).abs() <= tol;
            let ok = g.radius > crate::tol::POINT_MERGE && on(pair[0]) && on(pair[1]);
            match edge_of(pair).filter(|_| ok) {
                Some(e) => self.edges[e].curve = Some(g),
                None => dropped += 1,
            }
        }
        if !extra.surfaces.is_empty() {
            let fids: Vec<FaceId> = self.faces.keys().collect();
            for f in fids {
                let fits = |s: &SurfaceRef| -> bool {
                    let SurfaceRef::Cylinder {
                        axis_point,
                        axis,
                        radius,
                    } = *s;
                    let face = &self.faces[f];
                    if (axis.length() - 1.0).abs() > crate::tol::NORMAL_DIRECTION
                        || radius <= crate::tol::POINT_MERGE
                        || axis.dot(face.plane.normal()).abs() > crate::tol::NORMAL_DIRECTION
                    {
                        return false;
                    }
                    let dist = |p: Point3| {
                        let d = p - axis_point;
                        (d - axis * d.dot(axis)).length()
                    };
                    // A chord facet's corners lie ON the cylinder, not just
                    // inside it: stricter than the validator's bound, so a
                    // flat face that happens to fit inside is not claimed.
                    std::iter::once(face.outer_loop)
                        .chain(face.inner_loops.iter().copied())
                        .flat_map(|l| self.loop_positions(l).collect::<Vec<_>>())
                        .all(|p| (dist(p) - radius).abs() <= tol)
                };
                if let Some(s) = extra.surfaces.iter().find(|s| fits(s)) {
                    self.faces[f].surface = Some(*s);
                }
            }
        }
        dropped
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

    /// A square face with a triangular hole whose first vertex sits `off`
    /// outside the outer ring's left edge (negative: inside), at edge
    /// length `k`; the rest of the hole is well inside.
    fn square_with_poking_hole(k: f64, off: f64) -> Result<Object, TopologyError> {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(k, 0.0, 0.0),
            Point3::new(k, k, 0.0),
            Point3::new(0.0, k, 0.0),
            Point3::new(-off, 0.5 * k, 0.0),
            Point3::new(0.4 * k, 0.2 * k, 0.0),
            Point3::new(0.4 * k, 0.8 * k, 0.0),
        ];
        // Outer is counter-clockwise; the hole winds the other way.
        let mut obj = Object::from_polygons_with_holes_import(
            &positions,
            &[vec![0, 1, 2, 3]],
            &[vec![vec![4, 6, 5]]],
            &[None],
            &[None],
        )?;
        // Judged as native geometry: an import's planarity tolerance admits a
        // hole that grazes its ring by that much (`geom2d::hole_leaves_outer`),
        // which would hide the nanometre-scale rule these cases pin.
        obj.planarity_tol = crate::tol::PLANE_DIST;
        obj.validate()?;
        Ok(obj)
    }

    /// A hole vertex 10 nm outside a 1 mm outer edge leaves the face: the
    /// hole edges cross the ring beyond `POINT_MERGE`. (The crossing test
    /// once measured square metres against a metre tolerance, so at this
    /// size a 1 um dead band accepted it.)
    #[test]
    fn hole_vertex_ten_nanometres_outside_a_millimetre_edge_is_refused() {
        let err = square_with_poking_hole(1e-3, 1e-8).unwrap_err();
        assert!(
            matches!(err, TopologyError::HoleOutsideFace { .. }),
            "got {err:?}"
        );
    }

    /// A micron outside the same edge is refused too, and a hole vertex
    /// exactly on the edge is a touch and loads.
    #[test]
    fn hole_vertex_a_micron_outside_is_refused_and_one_on_the_edge_loads() {
        let err = square_with_poking_hole(1e-3, 1e-6).unwrap_err();
        assert!(
            matches!(err, TopologyError::HoleOutsideFace { .. }),
            "got {err:?}"
        );
        square_with_poking_hole(1e-3, 0.0).expect("a vertex on the edge is a touch");
    }

    /// The same vertex half a nanometre outside a 100 m edge is a touch
    /// (within `POINT_MERGE`), not a crossing, and loads.
    #[test]
    fn hole_vertex_half_a_nanometre_outside_a_long_edge_is_a_touch() {
        square_with_poking_hole(100.0, 0.5e-9).expect("a touch is not a crossing");
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
