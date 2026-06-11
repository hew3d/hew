//! Topology validator (DEVELOPMENT.md rule 2).
//!
//! `validate()` checks every structural invariant of an [`Object`]. In debug
//! builds, every public kernel mutation must call `check_invariants()` as its
//! last step before returning — new mutations ship with validator coverage in
//! the same PR.

use slotmap::SecondaryMap;

use crate::error::TopologyError;
use crate::tol;
use crate::topo::{LoopKind, Object, WatertightState};

impl Object {
    /// Checks all structural invariants, returning the first violation found.
    ///
    /// Invariants: every stored handle resolves; `next`/`prev` are mutually
    /// inverse; twins are a proper involution agreeing with their edge and
    /// with vertex incidence; every half-edge lies in exactly one loop and
    /// every loop closes; edges and half-edges agree about each other; loops
    /// and faces agree about each other; face boundaries lie on their stored
    /// plane (within [`tol::PLANE_DIST`]); vertices and their `outgoing`
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
                    if face.plane.signed_distance(p).abs() > tol::PLANE_DIST {
                        return Err(TopologyError::FaceGeometryNotPlanar { face: f });
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
    fn validator_catches_dangling_outgoing() {
        let mut tri = Object::triangle();
        let v = tri.vertices.keys().next().unwrap();
        tri.vertices[v].outgoing = HalfEdgeId::default();
        assert!(matches!(
            tri.validate(),
            Err(TopologyError::DanglingHandle { .. })
        ));
    }
}
