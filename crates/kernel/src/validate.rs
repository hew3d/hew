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
    /// and faces agree about each other; every loop is simple (no vertex
    /// twice), and loops of one face share at most isolated vertices, never
    /// an edge (no edge has both sides on one face); face boundaries lie on their stored
    /// plane (within the object's `planarity_tol` — strict `PLANE_DIST` for
    /// native geometry, wider `IMPORT_PLANE_DIST` for imports); every hole
    /// loop lies geometrically inside its face's outer boundary (checked
    /// conservatively: a hole with NO vertex inside the ring is invalid);
    /// vertices and their `outgoing`
    /// half-edges agree; every face is in exactly one shell; and the
    /// watertightness flag matches the actual topology; every face and edge
    /// attribute slot names a non-empty dictionary in this Object's table.
    pub fn validate(&self) -> Result<(), TopologyError> {
        // Finiteness first. Every geometric predicate below (planarity,
        // hole containment, curve radius, plane normalization) is a `>`/`<`
        // comparison, which IEEE-754 evaluates false against NaN — so they
        // all fail OPEN on a non-finite coordinate and would wave garbage
        // through. Rejecting non-finite vertices up front is the invariant
        // that lets the rest of the validator trust its arithmetic.
        for (v, vert) in &self.vertices {
            let p = vert.position;
            if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
                return Err(TopologyError::VertexNotFinite { vertex: v });
            }
        }

        // Attribute slots (face and edge dictionaries) point into this
        // Object's own table, at a dictionary that holds something: an
        // empty one is spelled `AttrSlot::NONE`, never a slot.
        let slot_ok = |slot: crate::topo::AttrSlot| {
            slot.is_none()
                || self
                    .attr_dicts
                    .get(slot.0 as usize)
                    .is_some_and(|d| !d.is_empty())
        };
        if !self.faces.values().all(|f| slot_ok(f.attrs)) {
            return Err(TopologyError::DanglingHandle {
                context: "face attribute slot",
            });
        }
        if !self.edges.values().all(|e| slot_ok(e.attrs)) {
            return Err(TopologyError::DanglingHandle {
                context: "edge attribute slot",
            });
        }

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

        // Every loop is simple: no vertex twice in one loop. (Loops of one
        // face may share a vertex at an isolated point — a hole touching the
        // outer loop, or two holes touching — but never an edge, below.)
        let mut visited_in: SecondaryMap<crate::ids::VertexId, crate::ids::LoopId> =
            SecondaryMap::new();
        for (l, lp) in &self.loops {
            let mut current = lp.first_half_edge;
            loop {
                let he = &self.half_edges[current];
                if visited_in.insert(he.origin, l) == Some(l) {
                    return Err(TopologyError::LoopNotSimple { loop_id: l });
                }
                current = he.next;
                if current == lp.first_half_edge {
                    break;
                }
            }
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
            // The two sides of an edge lie on two different faces: loops of
            // one face never share an edge.
            if let Some(t) = edge.twin_half_edge {
                let face_of = |h: crate::ids::HalfEdgeId| {
                    self.half_edges
                        .get(h)
                        .and_then(|he| self.loops.get(he.loop_id))
                        .map(|lp| lp.face)
                };
                if face_of(edge.half_edge).is_some() && face_of(edge.half_edge) == face_of(t) {
                    return Err(TopologyError::EdgeWithinOneFace { edge: e });
                }
            }
            // A SOFT mark (design §7b) claims the edge is an interior facet
            // seam — a boundary edge cannot be one.
            if edge.soft && edge.twin_half_edge.is_none() {
                return Err(TopologyError::EdgeHalfEdgeMismatch { edge: e });
            }
            // A present analytic circle claim must agree with the geometry it
            // describes (map-or-drop's enforcement half): both endpoints are
            // chord facets of the circle, so each lies within tolerance of
            // `radius` from `center`, and the radius is non-degenerate.
            if let Some(crate::sketch::CurveGeom { center, radius }) = edge.curve {
                let mismatch = TopologyError::EdgeCurveMismatch { edge: e };
                if !radius.is_finite()
                    || !(center.x.is_finite() && center.y.is_finite() && center.z.is_finite())
                    || radius <= crate::tol::POINT_MERGE
                {
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
            // NONE of its vertices are inside the outer ring or on it —
            // because a fully displaced hole is what a wrong ownership
            // assignment produces, while a vertex grazing the outer boundary
            // within tolerance must not fail legitimate geometry. (A
            // partially displaced hole cannot arise from valid operations at
            // all: cut paths are refused if they cross hole territory.) The
            // on-ring clause matters: an imported hole whose every vertex
            // sits on the outer ring (SketchUp draws them) is a boundary
            // case for the strict ray test, whose verdict on such points
            // differs between native and wasm float rounding — the same
            // document must load everywhere.
            if !face.inner_loops.is_empty() {
                let outer_pts: Vec<crate::math::Point3> =
                    self.loop_positions(face.outer_loop).collect();
                let normal = face.plane.normal();
                let on_ring = |p: crate::math::Point3| {
                    let n = outer_pts.len();
                    (0..n).any(|i| {
                        crate::geom2d::point_near_segment(
                            p,
                            outer_pts[i],
                            outer_pts[(i + 1) % n],
                            crate::tol::POINT_MERGE,
                        )
                    })
                };
                for &inner_id in &face.inner_loops {
                    let hole_pts: Vec<crate::math::Point3> =
                        self.loop_positions(inner_id).collect();
                    let any_inside = hole_pts.iter().any(|&p| {
                        crate::geom2d::point_inside_polygon(p, &outer_pts, normal) || on_ring(p)
                    });
                    // No stretch of a hole edge leaves the face: no proper
                    // crossing, and each piece of a hole edge between the
                    // outer vertices on it lies inside the outer ring or on
                    // it (within the object's planarity tolerance, so
                    // imported rings that graze it still load). A native
                    // object's hole never runs along an outer edge (two
                    // loops never share an edge, and drafting makes such a
                    // run one edge); an imported hole may (SketchUp draws
                    // them).
                    let imported = self.planarity_tol > crate::tol::PLANE_DIST;
                    let leaves = crate::geom2d::hole_leaves_outer(
                        &outer_pts,
                        &hole_pts,
                        normal,
                        self.planarity_tol.max(crate::tol::POINT_MERGE),
                        imported,
                    );
                    if !any_inside || leaves {
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

        // Last, after every structural check: the file loader recomputes a
        // face's plane from its outer loop, and a face it could not
        // recompute would make the saved document unopenable — so the
        // loader's own predicate holds for every live face.
        for (f, face) in &self.faces {
            let outer_ring: Vec<crate::math::Point3> =
                self.loop_positions(face.outer_loop).collect();
            if crate::serialize::outer_ring_plane(&outer_ring).is_err() {
                return Err(TopologyError::FaceTooSmallToReopen { face: f });
            }
        }

        Ok(())
    }

    /// Debug-build invariant check; call as the last step of every public
    /// mutation. Compiles to nothing in release builds.
    ///
    /// A face too small to reopen from a file is not a broken invariant but
    /// geometry user input can reach (a cut a fraction of a micron from a
    /// corner, a tiny scale): every mutation's always-on `validate()`
    /// backstop refuses it typed, so this debug check lets it through to
    /// that refusal. It is reported last, so nothing structural hides
    /// behind it.
    #[inline]
    pub(crate) fn check_invariants(&self) {
        #[cfg(debug_assertions)]
        match self.validate() {
            Ok(()) | Err(TopologyError::FaceTooSmallToReopen { .. }) => {}
            Err(violation) => panic!("kernel invariant violated after mutation: {violation}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
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

    /// A hole whose every vertex sits ON the outer ring (SketchUp draws such
    /// faces) is legitimate: the strict interior ray test answers
    /// platform-dependently for boundary points (natively it happens to
    /// admit two of these four; the wasm build admitted none of a real
    /// model's), so the on-ring clause is what keeps an imported document
    /// loadable everywhere. Its edges run along the ring or cut corners
    /// from vertex to vertex — touches, never proper crossings.
    #[test]
    fn hole_with_all_vertices_on_the_outer_ring_validates() {
        let positions = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
            // hole: a corner cut whose vertices all sit on the right and
            // top edges
            Point3::new(4.0, 1.0, 0.0),
            Point3::new(4.0, 3.0, 0.0),
            Point3::new(3.0, 4.0, 0.0),
            Point3::new(1.0, 4.0, 0.0),
        ];
        let faces = vec![vec![0, 1, 2, 3]];
        let holes = vec![vec![vec![4, 5, 6, 7]]];
        let obj = Object::from_polygons_with_holes_import(
            &positions,
            &faces,
            &holes,
            &[crate::material::FaceMaterial::default()],
            &[None],
        )
        .expect("builds");
        assert!(obj.validate().is_ok(), "on-ring hole is inside its face");
    }

    /// A hole that touches the ring at one vertex but runs out through it
    /// elsewhere is displaced, not inside: the on-ring clause alone would
    /// accept it, the crossing test refuses it. Built valid, then the
    /// hole's vertices are moved (a builder refuses such input outright).
    #[test]
    fn hole_grazing_the_ring_but_crossing_it_is_refused() {
        let positions = [
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
        let holes = vec![vec![vec![4, 5, 6, 7]]];
        let mut obj = Object::from_polygons_with_holes_import(
            &positions,
            &faces,
            &holes,
            &[crate::material::FaceMaterial::default()],
            &[None],
        )
        .expect("builds");
        assert!(obj.validate().is_ok());
        let hole = obj
            .faces
            .values()
            .find_map(|f| f.inner_loops.first().copied())
            .expect("the hole");
        // Shares B=(4,0) with the ring, then leaves the square and re-enters
        // across the top edge.
        let moved = [
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(10.0, 1.0, 0.0),
            Point3::new(10.0, 5.0, 0.0),
            Point3::new(2.0, 5.0, 0.0),
        ];
        let vids: Vec<_> = obj
            .loop_half_edges(hole)
            .map(|h| obj.half_edges[h].origin)
            .collect();
        for (vid, p) in vids.into_iter().zip(moved) {
            obj.vertices[vid].position = p;
        }
        assert!(
            matches!(obj.validate(), Err(TopologyError::HoleOutsideFace { .. })),
            "a crossing hole is never accepted"
        );
    }

    /// The L-shaped face (0,0) (4,0) (4,1) (1,1) (1,4) (0,4) with a hole
    /// built inside it, then moved so its three vertices sit on three of the
    /// L's corners, (0,0) (1,4) (4,1): no hole edge properly crosses the
    /// ring and every hole vertex is on it, yet its edge from (1,4) to
    /// (4,1) runs outside the face, past the reflex corner. Returns the
    /// object.
    pub(crate) fn l_with_a_hole_running_outside() -> Object {
        let positions = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 1.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
            Point3::new(0.2, 0.2, 0.0),
            Point3::new(0.5, 0.8, 0.0),
            Point3::new(0.8, 0.5, 0.0),
        ];
        let faces = vec![vec![0, 1, 2, 3, 4, 5]];
        let holes = vec![vec![vec![6, 7, 8]]];
        let mut obj = Object::from_polygons_with_holes_import(
            &positions,
            &faces,
            &holes,
            &[crate::material::FaceMaterial::default()],
            &[None],
        )
        .expect("builds");
        assert!(obj.validate().is_ok());
        let hole = obj
            .faces
            .values()
            .find_map(|f| f.inner_loops.first().copied())
            .expect("the hole");
        let vids: Vec<_> = obj
            .loop_half_edges(hole)
            .map(|h| obj.half_edges[h].origin)
            .collect();
        for (vid, p) in vids.into_iter().zip([
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 4.0, 0.0),
            Point3::new(4.0, 1.0, 0.0),
        ]) {
            obj.vertices[vid].position = p;
        }
        obj
    }

    #[test]
    fn a_hole_on_the_ring_running_outside_past_a_reflex_corner_is_refused() {
        let obj = l_with_a_hole_running_outside();
        assert!(
            matches!(obj.validate(), Err(TopologyError::HoleOutsideFace { .. })),
            "a hole edge outside the face is never accepted"
        );
    }

    /// Builds a one-face object with `outer` and a hole made inside it as
    /// a small copy of `hole` shrunk toward its centroid (so the builder
    /// accepts it), then moves the hole's vertices onto `hole`. Imported
    /// (the wider planarity tolerance) unless `native`.
    pub(crate) fn face_with_moved_hole(
        outer: &[(f64, f64)],
        hole: &[(f64, f64)],
        native: bool,
    ) -> Object {
        let p = |&(x, y): &(f64, f64)| Point3::new(x, y, 0.0);
        let c = hole.iter().fold((0.0, 0.0), |a, h| (a.0 + h.0, a.1 + h.1));
        let c = (c.0 / hole.len() as f64, c.1 / hole.len() as f64);
        let small: Vec<(f64, f64)> = hole
            .iter()
            .map(|h| (c.0 + 0.05 * (h.0 - c.0), c.1 + 0.05 * (h.1 - c.1)))
            .collect();
        let positions: Vec<Point3> = outer.iter().chain(small.iter()).map(p).collect();
        let faces = vec![(0..outer.len()).collect::<Vec<_>>()];
        let holes = vec![vec![
            (outer.len()..outer.len() + hole.len()).collect::<Vec<_>>(),
        ]];
        let mut obj = Object::from_polygons_with_holes_import(
            &positions,
            &faces,
            &holes,
            &[crate::material::FaceMaterial::default()],
            &[None],
        )
        .expect("builds");
        if native {
            obj.planarity_tol = crate::tol::PLANE_DIST;
        }
        assert!(obj.validate().is_ok(), "valid before the move");
        let l = obj
            .faces
            .values()
            .find_map(|f| f.inner_loops.first().copied())
            .expect("the hole");
        let vids: Vec<_> = obj
            .loop_half_edges(l)
            .map(|h| obj.half_edges[h].origin)
            .collect();
        // The hole loop starts at its first given vertex.
        for (vid, h) in vids.into_iter().zip(hole) {
            obj.vertices[vid].position = p(h);
        }
        obj
    }

    /// The notch case below, as an object.
    pub(crate) fn bay_with_a_hole_through_it() -> Object {
        let outer = [
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 20.0),
            (6.0, 20.0),
            (6.0, 7.0),
            (6.0, 4.0),
            (4.0, 4.0),
            (4.0, 6.0),
            (4.0, 20.0),
            (0.0, 20.0),
        ];
        let hole = [(1.0, 4.5), (19.0, 13.5), (10.0, 1.0)];
        face_with_moved_hole(&outer, &hole, false)
    }

    #[test]
    fn a_hole_edge_leaving_through_a_notch_between_two_ring_vertices_is_refused() {
        // A 20 x 20 square with an open-top bay at x in [4, 6], y in [4, 20]
        // whose sides carry ring vertices T1 = (4, 6) and T2 = (6, 7). The
        // hole's edge from P (1, 4.5) to Q (19, 13.5) runs through T1 and T2:
        // its ends and its midpoint (10, 9) are in material, nothing crosses
        // properly, but its stretch from T1 to T2 lies in the bay, outside
        // the face.
        let obj = bay_with_a_hole_through_it();
        assert!(
            matches!(obj.validate(), Err(TopologyError::HoleOutsideFace { .. })),
            "{:?}",
            obj.validate()
        );
    }

    #[test]
    fn an_imported_hole_grazing_its_ring_within_the_planarity_tolerance_loads() {
        // The ring dents in by `dent` at (4, 2), between ring vertices
        // (4, 1) and (4, 3); the hole's edge from (4, 1) to (4, 3) runs
        // straight past the dent, outside the face by `dent` at its middle.
        let ring = |dent: f64| {
            vec![
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 1.0),
                (4.0 - dent, 2.0),
                (4.0, 3.0),
                (4.0, 4.0),
                (0.0, 4.0),
            ]
        };
        let hole = [(4.0, 1.0), (2.0, 2.0), (4.0, 3.0)];
        // A hair past POINT_MERGE, well within the import tolerance: loads.
        let grazing = face_with_moved_hole(&ring(1e-6), &hole, false);
        assert!(grazing.planarity_tol > 1e-6);
        assert!(grazing.validate().is_ok(), "{:?}", grazing.validate());
        // Past the import tolerance: the hole runs outside.
        let outside = face_with_moved_hole(&ring(5e-3), &hole, false);
        assert!(matches!(
            outside.validate(),
            Err(TopologyError::HoleOutsideFace { .. })
        ));
        // A native object holds the strict tolerance: the same hair is out.
        let native = face_with_moved_hole(&ring(1e-6), &hole, true);
        assert!(matches!(
            native.validate(),
            Err(TopologyError::HoleOutsideFace { .. })
        ));
    }

    #[test]
    fn an_imported_hole_clipping_its_ring_by_a_graze_loads_and_a_native_one_does_not() {
        // A hole edge clipping the L's reflex corner about 7 um outside it
        // (crossing both corner edges), and a hole vertex 1 um past a
        // square's edge: within the import tolerance, a graze that loads;
        // on a native object, the exact rule refuses both. A real crossing,
        // 1 cm past the edge, is refused imported too.
        let l = [
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        let clip = [(1.5, 0.50001), (0.5, 0.5), (0.50001, 1.5)];
        let square = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let poke = |out: f64| [(2.0, 1.0), (2.0, 3.0), (4.0 + out, 2.0)];
        for (outer, hole) in [(&l[..], clip.to_vec()), (&square[..], poke(1e-6).to_vec())] {
            let imported = face_with_moved_hole(outer, &hole, false);
            assert!(imported.validate().is_ok(), "{:?}", imported.validate());
            let native = face_with_moved_hole(outer, &hole, true);
            assert!(matches!(
                native.validate(),
                Err(TopologyError::HoleOutsideFace { .. })
            ));
        }
        let far = face_with_moved_hole(&square, &poke(1e-2), false);
        assert!(matches!(
            far.validate(),
            Err(TopologyError::HoleOutsideFace { .. })
        ));
    }

    /// The imported face whose hole edge runs along its outer edge, as an
    /// object (the validator accepts it; the sweep checks do not).
    pub(crate) fn imported_hole_along_the_outer_edge() -> Object {
        let outer = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let hole = [(4.0, 1.0), (2.0, 2.0), (4.0, 3.0)];
        face_with_moved_hole(&outer, &hole, false)
    }

    #[test]
    fn a_hole_edge_running_along_an_outer_edge_is_imported_only() {
        // A hole whose edge (4, 1)-(4, 3) lies along the outer edge x = 4
        // (distinct vertices, no shared edge). SketchUp draws such faces, so
        // an imported one loads (the all-vertices-on-the-ring spec above);
        // a native face never holds one (drafting makes that run one edge,
        // and two loops never share an edge: `EdgeWithinOneFace` when they
        // do topologically), so a native object with it is HoleOutsideFace.
        let outer = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let hole = [(4.0, 1.0), (2.0, 2.0), (4.0, 3.0)];
        let imported = face_with_moved_hole(&outer, &hole, false);
        assert!(imported.validate().is_ok(), "{:?}", imported.validate());
        let native = face_with_moved_hole(&outer, &hole, true);
        assert!(matches!(
            native.validate(),
            Err(TopologyError::HoleOutsideFace { .. })
        ));
    }

    #[test]
    fn a_hole_edge_through_a_reflex_corner_is_a_point_contact_and_validates() {
        // The L (0,0) (2,0) (2,1) (1,1) (1,2) (0,2) with the hole triangle
        // (0.5, 1.5) (1.5, 0.5) (0.4, 0.4): the hole edge's midpoint is
        // exactly the reflex corner (1, 1). A point contact, not an escape.
        let outer = [
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        let hole = [(0.5, 1.5), (1.5, 0.5), (0.4, 0.4)];
        for native in [false, true] {
            let obj = face_with_moved_hole(&outer, &hole, native);
            assert!(
                obj.validate().is_ok(),
                "native {native}: {:?}",
                obj.validate()
            );
        }
    }

    #[test]
    fn validator_catches_a_loop_visiting_a_vertex_twice() {
        // An open triangle whose third half-edge is re-rooted at the first
        // vertex: links and loop closure stay intact, the loop visits that
        // vertex twice.
        let mut tri = Object::triangle();
        let l = tri.loops.keys().next().unwrap();
        let hes: Vec<HalfEdgeId> = tri.loop_half_edges(l).collect();
        let first = tri.half_edges[hes[0]].origin;
        tri.half_edges[hes[2]].origin = first;
        assert_eq!(
            tri.validate(),
            Err(TopologyError::LoopNotSimple { loop_id: l })
        );
    }

    #[test]
    fn validator_catches_an_edge_with_one_face_on_both_sides() {
        // Two triangles sharing an edge, then the second's loop handed to
        // the first as a hole: the shared edge has the first face on both
        // sides.
        let mut obj = Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
            ],
            &[vec![0, 1, 2], vec![2, 1, 3]],
        )
        .unwrap();
        let ids: Vec<crate::ids::FaceId> = obj.faces.keys().collect();
        let (keep, gone) = (ids[0], ids[1]);
        let l = obj.faces[gone].outer_loop;
        obj.loops[l].face = keep;
        obj.loops[l].kind = LoopKind::Inner;
        obj.faces[keep].inner_loops.push(l);
        obj.faces.remove(gone);
        for shell in obj.shells.values_mut() {
            shell.faces.retain(|&f| f != gone);
        }
        assert!(
            matches!(obj.validate(), Err(TopologyError::EdgeWithinOneFace { .. })),
            "{:?}",
            obj.validate()
        );
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
