//! Topology validator (DEVELOPMENT.md rule 2).
//!
//! `validate()` checks every structural invariant of an [`Object`]. In debug
//! builds, every public kernel mutation must call `check_invariants()` as its
//! last step before returning — new mutations ship with validator coverage in
//! the same PR.

use slotmap::SecondaryMap;

use crate::error::TopologyError;
use crate::topo::{LoopKind, Object, WatertightState};

impl Object {
    /// Checks all structural invariants, returning the first violation found.
    ///
    /// Invariants: every stored handle resolves; `next`/`prev` are mutually
    /// inverse; twins are a proper involution agreeing with their edge and
    /// with vertex incidence; every half-edge lies in exactly one loop and
    /// every loop closes; edges and half-edges agree about each other; loops
    /// and faces agree about each other; face boundaries lie on their stored
    /// plane (within the object's `planarity_tol` — strict `PLANE_DIST` for
    /// native geometry, wider `IMPORT_PLANE_DIST` for imports); every hole
    /// loop lies geometrically inside its face's outer boundary (checked
    /// conservatively: a hole with NO vertex inside the ring is invalid);
    /// vertices and their `outgoing`
    /// half-edges agree; every face is in exactly one shell; and the
    /// watertightness flag matches the actual topology.
    pub fn validate(&self) -> Result<(), TopologyError> {
        let mut is_origin: SecondaryMap<_, ()> = SecondaryMap::new();

        // Half-edge link integrity.
        for (h, he) in &self.half_edges {
            if !self.vertices.contains_key(he.origin) {
                return Err(TopologyError::DanglingHandle {
                    context: "half-edge origin",
                });
            }
            is_origin.insert(he.origin, ());
            let next = self
                .half_edges
                .get(he.next)
                .ok_or(TopologyError::DanglingHandle {
                    context: "half-edge next",
                })?;
            let prev = self
                .half_edges
                .get(he.prev)
                .ok_or(TopologyError::DanglingHandle {
                    context: "half-edge prev",
                })?;
            if next.prev != h || prev.next != h {
                return Err(TopologyError::BrokenLink { half_edge: h });
            }
            if !self.loops.contains_key(he.loop_id) {
                return Err(TopologyError::DanglingHandle {
                    context: "half-edge loop",
                });
            }
            let edge = self
                .edges
                .get(he.edge)
                .ok_or(TopologyError::DanglingHandle {
                    context: "half-edge edge",
                })?;
            if edge.half_edge != h && edge.twin_half_edge != Some(h) {
                return Err(TopologyError::EdgeHalfEdgeMismatch { edge: he.edge });
            }
            if let Some(t) = he.twin {
                let twin = self
                    .half_edges
                    .get(t)
                    .ok_or(TopologyError::DanglingHandle {
                        context: "half-edge twin",
                    })?;
                if t == h || twin.twin != Some(h) || twin.edge != he.edge {
                    return Err(TopologyError::BrokenTwin { half_edge: h });
                }
                // The twin runs the opposite direction: it must start where
                // this half-edge ends.
                if twin.origin != next.origin {
                    return Err(TopologyError::BrokenTwin { half_edge: h });
                }
            }
        }

        // Every half-edge lies in exactly one loop, and loops close.
        let mut seen: SecondaryMap<_, ()> = SecondaryMap::new();
        for (l, lp) in &self.loops {
            if !self.half_edges.contains_key(lp.first_half_edge) {
                return Err(TopologyError::DanglingHandle {
                    context: "loop first half-edge",
                });
            }
            let mut current = lp.first_half_edge;
            for _ in 0..=self.half_edges.len() {
                let he = &self.half_edges[current];
                if he.loop_id != l {
                    return Err(TopologyError::LoopMembership { half_edge: current });
                }
                if seen.insert(current, ()).is_some() {
                    return Err(TopologyError::LoopMembership { half_edge: current });
                }
                current = he.next;
                if current == lp.first_half_edge {
                    break;
                }
            }
            if current != lp.first_half_edge {
                return Err(TopologyError::BrokenLink { half_edge: current });
            }
        }
        if seen.len() != self.half_edges.len() {
            let orphan = self
                .half_edges
                .keys()
                .find(|&h| !seen.contains_key(h))
                .expect("count mismatch implies an unvisited half-edge");
            return Err(TopologyError::LoopMembership { half_edge: orphan });
        }

        // Edges agree with their half-edges.
        for (e, edge) in &self.edges {
            let primary =
                self.half_edges
                    .get(edge.half_edge)
                    .ok_or(TopologyError::DanglingHandle {
                        context: "edge half-edge",
                    })?;
            if primary.edge != e || primary.twin != edge.twin_half_edge {
                return Err(TopologyError::EdgeHalfEdgeMismatch { edge: e });
            }
            if let Some(t) = edge.twin_half_edge {
                let twin = self
                    .half_edges
                    .get(t)
                    .ok_or(TopologyError::DanglingHandle {
                        context: "edge twin half-edge",
                    })?;
                if twin.edge != e || twin.twin != Some(edge.half_edge) {
                    return Err(TopologyError::EdgeHalfEdgeMismatch { edge: e });
                }
            }
            // A present analytic circle claim must agree with the geometry it
            // describes (map-or-drop's enforcement half): both endpoints are
            // chord facets of the circle, so each lies within tolerance of
            // `radius` from `center`, and the radius is non-degenerate.
            if let Some(crate::sketch::CurveGeom { center, radius }) = edge.curve {
                let mismatch = TopologyError::EdgeCurveMismatch { edge: e };
                if !radius.is_finite() || radius <= crate::tol::POINT_MERGE {
                    return Err(mismatch);
                }
                let a = self.vertices[primary.origin].position;
                let b = self.vertices[self.half_edges[primary.next].origin].position;
                for p in [a, b] {
                    if ((p - center).length() - radius).abs() > self.planarity_tol {
                        return Err(mismatch);
                    }
                }
            }
        }

        // Loops and faces agree; face boundaries are planar.
        for (f, face) in &self.faces {
            let outer = self
                .loops
                .get(face.outer_loop)
                .ok_or(TopologyError::DanglingHandle {
                    context: "face outer loop",
                })?;
            if outer.face != f || outer.kind != LoopKind::Outer {
                return Err(TopologyError::LoopFaceMismatch {
                    loop_id: face.outer_loop,
                });
            }
            for &inner_id in &face.inner_loops {
                let inner = self
                    .loops
                    .get(inner_id)
                    .ok_or(TopologyError::DanglingHandle {
                        context: "face inner loop",
                    })?;
                if inner.face != f || inner.kind != LoopKind::Inner {
                    return Err(TopologyError::LoopFaceMismatch { loop_id: inner_id });
                }
            }
            let loop_ids = std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied());
            for loop_id in loop_ids {
                for p in self.loop_positions(loop_id) {
                    if face.plane.signed_distance(p).abs() > self.planarity_tol {
                        return Err(TopologyError::FaceGeometryNotPlanar { face: f });
                    }
                }
            }
            // Hole containment: a hole loop must lie inside its face's outer
            // boundary. Checked conservatively — the hole is invalid only if
            // NONE of its vertices are inside the outer ring — because a
            // fully displaced hole is what a wrong ownership assignment
            // produces, while a vertex grazing the outer boundary within
            // tolerance must not fail legitimate geometry. (A partially
            // displaced hole cannot arise from valid operations at all: cut
            // paths are refused if they cross hole territory.)
            if !face.inner_loops.is_empty() {
                let outer_pts: Vec<crate::math::Point3> =
                    self.loop_positions(face.outer_loop).collect();
                let normal = face.plane.normal();
                for &inner_id in &face.inner_loops {
                    let any_inside = self
                        .loop_positions(inner_id)
                        .any(|p| crate::geom2d::point_inside_polygon(p, &outer_pts, normal));
                    if !any_inside {
                        return Err(TopologyError::HoleOutsideFace {
                            face: f,
                            loop_id: inner_id,
                        });
                    }
                }
            }
            // A present analytic surface must agree with the geometry it
            // claims to describe (map-or-drop's enforcement half): a chord
            // facet of a cylinder has its plane parallel to the axis, at
            // most one radius away, with every vertex inside the cylinder.
            if let Some(crate::topo::SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            }) = face.surface
            {
                let mismatch = TopologyError::FaceSurfaceMismatch { face: f };
                if (axis.length() - 1.0).abs() > crate::tol::NORMAL_DIRECTION
                    || !radius.is_finite()
                    || radius <= crate::tol::POINT_MERGE
                {
                    return Err(mismatch);
                }
                if axis.dot(face.plane.normal()).abs() > crate::tol::NORMAL_DIRECTION {
                    return Err(mismatch);
                }
                if face.plane.signed_distance(axis_point).abs() > radius + self.planarity_tol {
                    return Err(mismatch);
                }
                let dist_to_axis = |p: crate::math::Point3| {
                    let d = p - axis_point;
                    (d - axis * d.dot(axis)).length()
                };
                let loop_ids =
                    std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied());
                for loop_id in loop_ids {
                    for p in self.loop_positions(loop_id) {
                        if dist_to_axis(p) > radius + self.planarity_tol {
                            return Err(mismatch);
                        }
                    }
                }
            }
        }
        for (l, lp) in &self.loops {
            let face = self
                .faces
                .get(lp.face)
                .ok_or(TopologyError::DanglingHandle {
                    context: "loop face",
                })?;
            let belongs = face.outer_loop == l || face.inner_loops.contains(&l);
            if !belongs {
                return Err(TopologyError::LoopFaceMismatch { loop_id: l });
            }
        }

        // Vertices agree with their outgoing half-edges, no orphan vertices.
        for (v, vertex) in &self.vertices {
            let outgoing =
                self.half_edges
                    .get(vertex.outgoing)
                    .ok_or(TopologyError::DanglingHandle {
                        context: "vertex outgoing",
                    })?;
            if outgoing.origin != v {
                return Err(TopologyError::VertexOutgoingMismatch { vertex: v });
            }
            if !is_origin.contains_key(v) {
                return Err(TopologyError::OrphanVertex { vertex: v });
            }
        }

        // Every face lies in exactly one shell.
        let mut shell_count: SecondaryMap<_, u32> = SecondaryMap::new();
        for shell in self.shells.values() {
            for &f in &shell.faces {
                if !self.faces.contains_key(f) {
                    return Err(TopologyError::DanglingHandle {
                        context: "shell face",
                    });
                }
                *shell_count.entry(f).expect("face key is live").or_insert(0) += 1;
            }
        }
        for f in self.faces.keys() {
            if shell_count.get(f).copied().unwrap_or(0) != 1 {
                return Err(TopologyError::FaceShellMembership { face: f });
            }
        }

        // The watertightness flag is honest.
        let expected = if !self.half_edges.is_empty()
            && self.half_edges.values().all(|he| he.twin.is_some())
        {
            WatertightState::Watertight
        } else {
            WatertightState::Open
        };
        if self.watertight != expected {
            return Err(TopologyError::WatertightFlagMismatch { expected });
        }

        Ok(())
    }

    /// Debug-build invariant check; call as the last step of every public
    /// mutation. Compiles to nothing in release builds.
    #[inline]
    pub(crate) fn check_invariants(&self) {
        #[cfg(debug_assertions)]
        if let Err(violation) = self.validate() {
            panic!("kernel invariant violated after mutation: {violation}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topo::Vertex;
    use crate::{HalfEdgeId, Point3};

    #[test]
    fn validator_catches_corrupted_twin() {
        let mut tet = Object::tetrahedron();
        let some_he = tet.half_edges.keys().next().unwrap();
        tet.half_edges[some_he].twin = Some(some_he);
        assert!(matches!(
            tet.validate(),
            Err(TopologyError::BrokenTwin { .. } | TopologyError::EdgeHalfEdgeMismatch { .. })
        ));
    }

    #[test]
    fn validator_catches_broken_next_link() {
        let mut tet = Object::tetrahedron();
        let keys: Vec<_> = tet.half_edges.keys().collect();
        // Point a half-edge's next at itself: prev(next(h)) != h.
        tet.half_edges[keys[0]].next = keys[0];
        assert!(tet.validate().is_err());
    }

    #[test]
    fn validator_catches_dishonest_watertight_flag() {
        let mut tri = Object::triangle();
        tri.watertight = WatertightState::Watertight;
        assert_eq!(
            tri.validate(),
            Err(TopologyError::WatertightFlagMismatch {
                expected: WatertightState::Open
            })
        );
    }

    #[test]
    fn validator_catches_orphan_vertex() {
        let mut tet = Object::tetrahedron();
        let outgoing = tet.half_edges.keys().next().unwrap();
        tet.vertices.insert(Vertex {
            position: Point3::ORIGIN,
            outgoing,
        });
        assert!(matches!(
            tet.validate(),
            Err(TopologyError::VertexOutgoingMismatch { .. } | TopologyError::OrphanVertex { .. })
        ));
    }

    #[test]
    fn validator_catches_displaced_hole() {
        // A slab whose top face carries a small imprinted hole on its left
        // half; splitting the top and handing the hole to the coplanar RIGHT
        // half leaves every ownership pointer self-consistent while the hole
        // lies entirely outside its owner's outer ring.
        let mut obj = Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(10.0, 10.0, 0.0),
                Point3::new(0.0, 10.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(10.0, 0.0, 1.0),
                Point3::new(10.0, 10.0, 1.0),
                Point3::new(0.0, 10.0, 1.0),
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
        .unwrap();
        let top = obj
            .faces
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.9)
            .map(|(id, _)| id)
            .unwrap();
        obj.split_face_inner(
            top,
            &[
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(2.0, 1.0, 1.0),
                Point3::new(2.0, 2.0, 1.0),
                Point3::new(1.0, 2.0, 1.0),
            ],
        )
        .unwrap();
        obj.split_face(
            top,
            &[Point3::new(5.0, 0.0, 1.0), Point3::new(5.0, 10.0, 1.0)],
        )
        .unwrap();
        assert!(obj.validate().is_ok(), "well-formed before displacement");

        let (owner, hole) = obj
            .faces
            .iter()
            .find_map(|(id, f)| f.inner_loops.first().map(|&il| (id, il)))
            .expect("the hole survived the split");
        let other = obj
            .faces
            .iter()
            .find(|(id, f)| *id != owner && f.plane.normal().z > 0.9)
            .map(|(id, _)| id)
            .expect("the coplanar sibling exists");
        obj.faces[owner].inner_loops.retain(|&il| il != hole);
        obj.faces[other].inner_loops.push(hole);
        obj.loops[hole].face = other;
        assert!(matches!(
            obj.validate(),
            Err(TopologyError::HoleOutsideFace { .. })
        ));
    }

    #[test]
    fn validator_catches_dangling_outgoing() {
        let mut tri = Object::triangle();
        let v = tri.vertices.keys().next().unwrap();
        tri.vertices[v].outgoing = HalfEdgeId::default();
        assert!(matches!(
            tri.validate(),
            Err(TopologyError::DanglingHandle { .. })
        ));
    }

    #[test]
    fn validator_catches_lying_surface_reference() {
        use crate::math::Vec3;
        use crate::topo::SurfaceRef;
        let mut tet = Object::tetrahedron();
        let f = tet.faces.keys().next().unwrap();
        // Claim a face is a chord facet of a cylinder it cannot be on (its
        // vertices sit farther from the axis than the radius).
        tet.faces[f].surface = Some(SurfaceRef::Cylinder {
            axis_point: Point3::new(10.0, 10.0, 0.0),
            axis: Vec3::new(0.0, 0.0, 1.0),
            radius: 0.001,
        });
        assert!(matches!(
            tet.validate(),
            Err(TopologyError::FaceSurfaceMismatch { .. })
        ));

        // A non-unit axis is caught too.
        tet.faces[f].surface = Some(SurfaceRef::Cylinder {
            axis_point: Point3::ORIGIN,
            axis: Vec3::new(0.0, 0.0, 2.0),
            radius: 100.0,
        });
        assert!(matches!(
            tet.validate(),
            Err(TopologyError::FaceSurfaceMismatch { .. })
        ));
    }

    #[test]
    fn validator_catches_lying_edge_curve() {
        use crate::sketch::CurveGeom;
        let mut tet = Object::tetrahedron();
        let e = tet.edges.keys().next().unwrap();
        // Claim an edge is a chord facet of a circle its endpoints are not on
        // (its endpoints sit far from `radius` off the center).
        tet.edges[e].curve = Some(CurveGeom {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 0.001,
        });
        assert!(matches!(
            tet.validate(),
            Err(TopologyError::EdgeCurveMismatch { .. })
        ));

        // A degenerate radius is caught too.
        tet.edges[e].curve = Some(CurveGeom {
            center: Point3::ORIGIN,
            radius: 0.0,
        });
        assert!(matches!(
            tet.validate(),
            Err(TopologyError::EdgeCurveMismatch { .. })
        ));
    }
}
