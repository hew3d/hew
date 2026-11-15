//! Executable specs for the `Sketch` draft context and `Profile` validation
//! (DEVELOPMENT.md rule 3). Same rules as `op_specs.rs`: every test is
//! `#[ignore]`d until its operation is implemented; un-ignore in the same PR;
//! never weaken an assertion — escalate instead.
//!
//! All sketches here live on the XY plane (z = 0); `pt(x, y)` builds points
//! on it. The sticky-rule numbering in comments refers to the module docs of
//! `kernel::sketch`.

use kernel::{Plane, Point3, Profile, ProfileError, Sketch, SketchError, SketchVertexId, tol};
use proptest::prelude::*;

fn pt(x: f64, y: f64) -> Point3 {
    Point3::new(x, y, 0.0)
}

fn xy_plane() -> Plane {
    Plane::from_polygon(&[pt(0.0, 0.0), pt(1.0, 0.0), pt(0.0, 1.0)]).unwrap()
}

fn xy_sketch() -> Sketch {
    Sketch::on_plane(xy_plane())
}

fn counts(s: &Sketch) -> (usize, usize, usize) {
    (s.vertices().len(), s.edges().len(), s.regions().len())
}

/// The four sides of an axis-aligned rectangle as (from, to) segments.
fn rect_segments(x0: f64, y0: f64, x1: f64, y1: f64) -> [(Point3, Point3); 4] {
    [
        (pt(x0, y0), pt(x1, y0)),
        (pt(x1, y0), pt(x1, y1)),
        (pt(x1, y1), pt(x0, y1)),
        (pt(x0, y1), pt(x0, y0)),
    ]
}

// ------------------------------------------------------------ sticky rules

#[test]
fn shared_endpoints_merge() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    let report = s.add_segment(pt(1.0, 0.0), pt(1.0, 1.0)).unwrap();
    // Second segment reuses the (1,0) vertex: 3 vertices total, not 4.
    assert_eq!(counts(&s), (3, 2, 0));
    assert_eq!(report.new_vertices.len(), 1);
    assert_eq!(report.new_edges.len(), 1);
}

#[test]
fn t_junction_splits_the_touched_edge() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(2.0, 0.0)).unwrap();
    let report = s.add_segment(pt(1.0, 0.0), pt(1.0, 1.0)).unwrap();
    // The horizontal edge becomes two fragments + the new vertical edge.
    assert_eq!(counts(&s), (4, 3, 0));
    assert_eq!(report.split_edges.len(), 1);
    assert_eq!(report.split_edges[0].1.len(), 2);
}

#[test]
fn crossing_splits_both_edges() {
    let mut s = xy_sketch();
    s.add_segment(pt(-1.0, 0.0), pt(1.0, 0.0)).unwrap();
    let report = s.add_segment(pt(0.0, -1.0), pt(0.0, 1.0)).unwrap();
    // A new vertex at the crossing; each segment contributes two fragments.
    assert_eq!(counts(&s), (5, 4, 0));
    assert!(!report.new_vertices.is_empty());
    assert_eq!(report.split_edges.len(), 1); // the pre-existing edge
    assert_eq!(report.new_edges.len(), 2); // the new segment's two fragments
}

#[test]
fn collinear_overlap_does_not_stack_edges() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(2.0, 0.0)).unwrap();
    s.add_segment(pt(1.0, 0.0), pt(3.0, 0.0)).unwrap();
    // Result is three abutting edges over [0,1], [1,2], [2,3]: no edge
    // covers another, no zero-length fragments.
    assert_eq!(counts(&s), (4, 3, 0));
}

#[test]
fn closing_a_rectangle_creates_one_region() {
    let mut s = xy_sketch();
    let segs = rect_segments(0.0, 0.0, 2.0, 1.0);
    for (from, to) in &segs[..3] {
        let r = s.add_segment(*from, *to).unwrap();
        assert!(
            r.regions_created.is_empty(),
            "no region before the loop closes"
        );
    }
    let last = s.add_segment(segs[3].0, segs[3].1).unwrap();
    assert_eq!(last.regions_created.len(), 1);
    assert_eq!(counts(&s), (4, 4, 1));
    let region = &s.regions()[last.regions_created[0]];
    assert_eq!(region.outer.len(), 4);
    assert!(region.holes.is_empty());
}

#[test]
fn inner_circuit_makes_a_hole_at_millimeter_scale() {
    // The same nesting at millimeter size, inner ring drawn first and last:
    // every cycle's area is a few square millimeters, so finding the
    // unbounded face must not lean on an absolute area slack.
    for inner_first in [true, false] {
        let mut s = xy_sketch();
        let outer = rect_segments(0.0, 0.0, 0.004, 0.004);
        let inner = rect_segments(0.001, 0.001, 0.003, 0.003);
        let order = if inner_first {
            [inner, outer]
        } else {
            [outer, inner]
        };
        for ring in order {
            for (from, to) in ring {
                s.add_segment(from, to).unwrap();
            }
        }
        assert_eq!(s.regions().len(), 2, "inner first: {inner_first}");
        let mut holes: Vec<usize> = s.regions().values().map(|r| r.holes.len()).collect();
        holes.sort_unstable();
        assert_eq!(holes, vec![0, 1], "inner first: {inner_first}");
    }
}

#[test]
fn inner_circuit_makes_a_hole_at_millimeter_scale_far_from_the_origin() {
    // A millimeter ring with a hole, kilometers out: the cycle areas must
    // not lose their precision to the world coordinates.
    for inner_first in [true, false] {
        let mut s = xy_sketch();
        let (x, y) = (12_345.678, -9_876.543);
        let ring = |r: f64| -> Vec<(Point3, Point3)> {
            let at = |k: usize| {
                let a = std::f64::consts::TAU * (k as f64 + 0.3) / 13.0;
                Point3::new(x + r * a.cos(), y + r * a.sin(), 0.0)
            };
            (0..13).map(|k| (at(k), at(k + 1))).collect()
        };
        let outer = ring(0.002);
        let inner = ring(0.001);
        let order = if inner_first {
            [inner, outer]
        } else {
            [outer, inner]
        };
        for ring in order {
            for (from, to) in ring {
                s.add_segment(from, to).unwrap();
            }
        }
        assert_eq!(s.regions().len(), 2, "inner first: {inner_first}");
        let mut holes: Vec<usize> = s.regions().values().map(|r| r.holes.len()).collect();
        holes.sort_unstable();
        assert_eq!(holes, vec![0, 1], "inner first: {inner_first}");
    }
}

#[test]
fn inner_circuit_makes_a_hole() {
    let mut s = xy_sketch();
    for (from, to) in rect_segments(0.0, 0.0, 4.0, 4.0) {
        s.add_segment(from, to).unwrap();
    }
    for (from, to) in rect_segments(1.0, 1.0, 3.0, 3.0) {
        s.add_segment(from, to).unwrap();
    }
    // Two regions: the ring (outer with one hole) and the inner island.
    assert_eq!(s.regions().len(), 2);
    let hole_counts: Vec<usize> = s.regions().values().map(|r| r.holes.len()).collect();
    assert!(hole_counts.contains(&1), "one region carries the hole");
    assert!(hole_counts.contains(&0), "the island has none");

    // Both regions must yield VALID profiles. (Regression: the hole used to be
    // assigned to the inner square's own region — its own reverse-wound
    // boundary — so that region's profile was self-intersecting and extruding
    // it panicked. The hole belongs to the larger ring, not the island.)
    for (rid, region) in s.regions() {
        s.profile(rid).expect("every region yields a valid profile");
        if region.holes.is_empty() {
            // The island is the small inner square (4 corners, ~area 4).
            assert_eq!(region.outer.len(), 4);
        } else {
            // The ring's outer is the big square; its hole is the inner one.
            assert_eq!(region.holes.len(), 1);
        }
    }
}

#[test]
fn chord_across_a_region_splits_it() {
    let mut s = xy_sketch();
    for (from, to) in rect_segments(0.0, 0.0, 2.0, 2.0) {
        s.add_segment(from, to).unwrap();
    }
    assert_eq!(s.regions().len(), 1);
    let report = s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();
    assert_eq!(s.regions().len(), 2);
    assert_eq!(report.regions_removed.len(), 1);
    assert_eq!(report.regions_created.len(), 2);
}

// -------------------------------------------------------------- rejections

#[test]
fn off_plane_point_is_rejected_unchanged() {
    let mut s = xy_sketch();
    let err = s
        .add_segment(pt(0.0, 0.0), Point3::new(1.0, 0.0, 1.0))
        .unwrap_err();
    assert_eq!(err, SketchError::PointOffPlane { which: 1 });
    assert_eq!(counts(&s), (0, 0, 0)); // strong guarantee
}

#[test]
fn degenerate_segment_is_rejected() {
    let mut s = xy_sketch();
    let err = s
        .add_segment(pt(0.0, 0.0), pt(tol::POINT_MERGE / 2.0, 0.0))
        .unwrap_err();
    assert_eq!(err, SketchError::DegenerateSegment);
    assert_eq!(counts(&s), (0, 0, 0));
}

// ------------------------------------------------------------- remove_edge

#[test]
fn removing_a_boundary_edge_dissolves_the_region() {
    let mut s = xy_sketch();
    for (from, to) in rect_segments(0.0, 0.0, 2.0, 2.0) {
        s.add_segment(from, to).unwrap();
    }
    let some_edge = s.edges().keys().next().unwrap();
    let report = s.remove_edge(some_edge).unwrap();
    assert_eq!(report.regions_removed.len(), 1);
    // Corner vertices each still touch one remaining edge: nothing orphaned.
    assert_eq!(counts(&s), (4, 3, 0));
    assert!(report.removed_vertices.is_empty());
}

#[test]
fn removing_a_dangling_edge_removes_its_lonely_vertex() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    s.add_segment(pt(1.0, 0.0), pt(2.0, 0.0)).unwrap();
    let dangling = s
        .edges()
        .iter()
        .find(|(_, e)| {
            s.vertices()[e.to]
                .position
                .approx_eq(pt(2.0, 0.0), tol::POINT_MERGE)
                || s.vertices()[e.from]
                    .position
                    .approx_eq(pt(2.0, 0.0), tol::POINT_MERGE)
        })
        .map(|(id, _)| id)
        .unwrap();
    let report = s.remove_edge(dangling).unwrap();
    // The (2,0) endpoint had no other edge; (1,0) is still used.
    assert_eq!(report.removed_vertices.len(), 1);
    assert_eq!(counts(&s), (2, 1, 0));
}

#[test]
fn remove_edge_rejects_stale_handle() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    let edge = s.edges().keys().next().unwrap();
    s.remove_edge(edge).unwrap();
    assert_eq!(s.remove_edge(edge).unwrap_err(), SketchError::UnknownEdge);
}

// -------------------------------------------------------- profile extraction

#[test]
fn region_exports_as_profile() {
    let mut s = xy_sketch();
    let mut region = None;
    for (from, to) in rect_segments(0.0, 0.0, 2.0, 1.0) {
        let r = s.add_segment(from, to).unwrap();
        if let Some(&id) = r.regions_created.first() {
            region = Some(id);
        }
    }
    let profile = s.profile(region.unwrap()).unwrap();
    assert_eq!(profile.outer().len(), 4);
    assert!(profile.holes().is_empty());
    // Same plane, and every boundary point on it.
    for p in profile.outer() {
        assert!(profile.plane().signed_distance(*p).abs() <= tol::PLANE_DIST);
    }
}

#[test]
fn profile_of_stale_region_is_an_error() {
    let mut s = xy_sketch();
    let mut region = None;
    for (from, to) in rect_segments(0.0, 0.0, 2.0, 2.0) {
        let r = s.add_segment(from, to).unwrap();
        if let Some(&id) = r.regions_created.first() {
            region = Some(id);
        }
    }
    let region = region.unwrap();
    // Splitting the region invalidates the old handle.
    s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();
    assert_eq!(s.profile(region).unwrap_err(), SketchError::UnknownRegion);
}

#[test]
fn unreshaped_region_keeps_its_handle() {
    let mut s = xy_sketch();
    let mut region = None;
    for (from, to) in rect_segments(0.0, 0.0, 2.0, 2.0) {
        let r = s.add_segment(from, to).unwrap();
        if let Some(&id) = r.regions_created.first() {
            region = Some(id);
        }
    }
    let region = region.unwrap();
    // A mutation elsewhere must not invalidate the untouched region's
    // handle, and must not mention it in either report list — "regions die
    // when reshaped" implies they LIVE when not.
    let report = s.add_segment(pt(5.0, 5.0), pt(6.0, 5.0)).unwrap();
    assert!(report.regions_created.is_empty());
    assert!(report.regions_removed.is_empty());
    assert!(s.regions().contains_key(region));
    assert!(s.profile(region).is_ok());
}

// ------------------------------------------- move_vertex (Phase D slice 3)
//
// Topology-PRESERVING vertex drag: the vertex moves and its incident edges
// stretch, but the 2D topology (vertices/edges/regions) is untouched. Any
// move that would require re-topologizing — an incident edge crossing another
// edge, two vertices merging — is refused loudly (rule 4: no silent repair),
// never re-stitched. `move_vertex` returns the vertex's OLD position so the
// document layer can record an exact inverse for undo.

/// The id of the vertex at `p` (panics if none — handles are opaque, so tests
/// address vertices by where they sit).
fn vertex_at(s: &Sketch, p: Point3) -> SketchVertexId {
    s.vertices()
        .iter()
        .find(|(_, v)| v.position.approx_eq(p, tol::POINT_MERGE))
        .map(|(id, _)| id)
        .expect("a vertex at the given position")
}

/// A closed unit-square sketch (one region), the common fixture below.
fn square() -> Sketch {
    let mut s = xy_sketch();
    for (from, to) in rect_segments(0.0, 0.0, 2.0, 2.0) {
        s.add_segment(from, to).unwrap();
    }
    s
}

#[test]
fn move_vertex_nudges_a_corner_keeping_topology() {
    let mut s = square();
    let v = vertex_at(&s, pt(2.0, 2.0));
    // A small nudge keeps the quad simple (convex even) — pure reposition.
    let old = s.move_vertex(v, pt(2.5, 1.8)).unwrap();
    assert!(
        old.approx_eq(pt(2.0, 2.0), 1e-12),
        "returns the old position"
    );
    assert_eq!(counts(&s), (4, 4, 1), "topology untouched");
    assert!(
        s.vertices()[v].position.approx_eq(pt(2.5, 1.8), 1e-12),
        "the vertex moved to its new position"
    );
    // The region survives with valid geometry.
    let region = s.regions().keys().next().unwrap();
    assert!(s.profile(region).is_ok());
}

#[test]
fn move_vertex_round_trips_via_returned_old_position() {
    let mut s = square();
    let v = vertex_at(&s, pt(0.0, 0.0));
    let old = s.move_vertex(v, pt(-0.5, -0.3)).unwrap();
    // Replaying the returned old position is an exact inverse (what undo does).
    s.move_vertex(v, old).unwrap();
    assert!(s.vertices()[v].position.approx_eq(pt(0.0, 0.0), 1e-12));
    assert_eq!(counts(&s), (4, 4, 1));
}

#[test]
fn move_vertex_across_an_edge_is_rejected_unchanged() {
    let mut s = square();
    let v = vertex_at(&s, pt(0.0, 0.0));
    // Drag corner A(0,0) out to (3, 1): its edge to D(0,2) now sweeps across
    // the opposite side B(2,0)-C(2,2). No incident edge collapses and A lands
    // on no vertex, so this is a pure crossing → WouldRetopologize.
    assert_eq!(
        s.move_vertex(v, pt(3.0, 1.0)),
        Err(SketchError::WouldRetopologize)
    );
    // Strong guarantee: the sketch is exactly as it was.
    assert_eq!(counts(&s), (4, 4, 1));
    assert!(s.vertices()[v].position.approx_eq(pt(0.0, 0.0), 1e-12));
}

#[test]
fn move_vertex_collapsing_an_incident_edge_is_degenerate() {
    let mut s = square();
    let v = vertex_at(&s, pt(0.0, 0.0));
    // Onto the ADJACENT corner B(2,0): the shared edge A-B collapses to zero
    // length — degeneracy is caught before the re-topology guard.
    assert_eq!(
        s.move_vertex(v, pt(2.0, 0.0)),
        Err(SketchError::DegenerateSegment)
    );
    assert_eq!(counts(&s), (4, 4, 1));
}

#[test]
fn move_vertex_onto_a_non_adjacent_vertex_is_rejected() {
    let mut s = square();
    let v = vertex_at(&s, pt(0.0, 0.0));
    // Onto the DIAGONAL corner C(2,2): no incident edge of A touches C, so
    // nothing collapses — but the vertices would merge → WouldRetopologize.
    assert_eq!(
        s.move_vertex(v, pt(2.0, 2.0)),
        Err(SketchError::WouldRetopologize)
    );
    assert_eq!(counts(&s), (4, 4, 1));
}

#[test]
fn move_vertex_off_plane_is_rejected_unchanged() {
    let mut s = square();
    let v = vertex_at(&s, pt(0.0, 0.0));
    assert_eq!(
        s.move_vertex(v, Point3::new(0.0, 0.0, 1.0)),
        Err(SketchError::PointOffPlane { which: 0 })
    );
    assert!(s.vertices()[v].position.approx_eq(pt(0.0, 0.0), 1e-12));
}

#[test]
fn move_unknown_vertex_errors() {
    let mut s = square();
    assert_eq!(
        s.move_vertex(SketchVertexId::default(), pt(1.0, 1.0)),
        Err(SketchError::UnknownVertex)
    );
}

// --------------------------------------------------- insertion-order property

proptest! {
    #[test]
    fn rectangle_closes_in_any_insertion_order(order in Just(vec![0usize, 1, 2, 3]).prop_shuffle()) {
        let segs = rect_segments(0.0, 0.0, 2.0, 1.0);
        let mut s = xy_sketch();
        for &i in order.iter() {
            s.add_segment(segs[i].0, segs[i].1).unwrap();
        }
        prop_assert_eq!(counts(&s), (4, 4, 1));
    }

    /// A nudge small enough to keep the unit square a simple quad is always a
    /// topology-preserving move (1 region in, 1 region out) and is reversed
    /// exactly by replaying the returned old position.
    #[test]
    fn small_nudge_preserves_region_and_reverses(
        dx in -0.4f64..0.4,
        dy in -0.4f64..0.4,
    ) {
        let mut s = square();
        let v = vertex_at(&s, pt(0.0, 0.0));
        let target = pt(dx, dy);
        // Skip the rare draw that lands on the corner's own neighbours.
        prop_assume!(!target.approx_eq(pt(0.0, 0.0), 1e-3));
        let old = s.move_vertex(v, target).unwrap();
        prop_assert_eq!(counts(&s), (4, 4, 1));
        s.move_vertex(v, old).unwrap();
        prop_assert!(s.vertices()[v].position.approx_eq(pt(0.0, 0.0), 1e-9));
    }
}

// -------------------------------------------------------- Profile validation

#[test]
fn profile_rejects_wrong_winding() {
    // Clockwise outer boundary (seen from +z).
    let err = Profile::new(
        xy_plane(),
        vec![pt(0.0, 0.0), pt(0.0, 1.0), pt(1.0, 1.0), pt(1.0, 0.0)],
        vec![],
    )
    .unwrap_err();
    assert_eq!(err, ProfileError::WrongWinding);
}

#[test]
fn profile_rejects_bowtie() {
    let err = Profile::new(
        xy_plane(),
        vec![pt(0.0, 0.0), pt(2.0, 2.0), pt(2.0, 0.0), pt(0.0, 2.0)],
        vec![],
    )
    .unwrap_err();
    assert_eq!(err, ProfileError::SelfIntersecting);
}

#[test]
fn profile_rejects_escaped_hole() {
    let err = Profile::new(
        xy_plane(),
        vec![pt(0.0, 0.0), pt(2.0, 0.0), pt(2.0, 2.0), pt(0.0, 2.0)],
        // CW winding (correct for a hole) but outside the outer square.
        vec![vec![pt(5.0, 5.0), pt(5.0, 6.0), pt(6.0, 6.0), pt(6.0, 5.0)]],
    )
    .unwrap_err();
    assert_eq!(err, ProfileError::HoleOutsideOuter);
}

#[test]
fn profile_rejects_off_plane_points() {
    let err = Profile::new(
        xy_plane(),
        vec![pt(0.0, 0.0), pt(1.0, 0.0), Point3::new(1.0, 1.0, 0.5)],
        vec![],
    )
    .unwrap_err();
    assert_eq!(err, ProfileError::PointOffPlane);
}

#[test]
fn valid_washer_profile_constructs() {
    let profile = Profile::new(
        xy_plane(),
        vec![pt(0.0, 0.0), pt(4.0, 0.0), pt(4.0, 4.0), pt(0.0, 4.0)],
        vec![vec![pt(1.0, 1.0), pt(1.0, 3.0), pt(3.0, 3.0), pt(3.0, 1.0)]],
    )
    .unwrap();
    assert_eq!(profile.outer().len(), 4);
    assert_eq!(profile.holes().len(), 1);
}

// -------------------------------------------------------- curve geometry
//
// A curve chain opened with `begin_curve_with` carries the analytic circle
// the drawing tool computed (the true-curves design). The geometry is
// durable across sticky splits, maps under similarity transforms, and DROPS
// (identity kept) under anything that deforms the chain away from its circle
// — the map-or-drop contract. Nothing here touches the facets themselves.

/// Commits an `n`-gon circle chain around `center` and returns its curve id.
fn circle_chain(s: &mut Sketch, center: Point3, radius: f64, n: usize) -> kernel::SketchCurveId {
    let id = s
        .begin_curve_with(kernel::CurveGeom { center, radius })
        .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        pt(center.x + radius * a.cos(), center.y + radius * a.sin())
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    id
}

#[test]
fn begin_curve_with_records_retrievable_geometry() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(1.0, 1.0), 0.5, 12);
    let g = s.curve_geom(id).expect("geometry recorded");
    assert!(g.center.approx_eq(pt(1.0, 1.0), tol::POINT_MERGE));
    assert_eq!(g.radius, 0.5);
    // A plain bracket mints an identity-only chain.
    let plain = s.begin_curve();
    s.end_curve();
    assert_eq!(s.curve_geom(plain), None);
}

#[test]
fn begin_curve_with_rejects_off_plane_center() {
    let mut s = xy_sketch();
    let err = s
        .begin_curve_with(kernel::CurveGeom {
            center: Point3::new(0.0, 0.0, 1.0),
            radius: 0.5,
        })
        .unwrap_err();
    assert_eq!(err, SketchError::PointOffPlane { which: 0 });
    // No bracket opened: the next segment stays a plain line.
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    assert!(s.edges().values().all(|e| e.curve.is_none()));
}

#[test]
fn begin_curve_with_rejects_degenerate_radius() {
    let mut s = xy_sketch();
    for bad in [0.0, tol::POINT_MERGE, -1.0, f64::NAN, f64::INFINITY] {
        let err = s
            .begin_curve_with(kernel::CurveGeom {
                center: pt(0.0, 0.0),
                radius: bad,
            })
            .unwrap_err();
        assert_eq!(err, SketchError::DegenerateCurve, "radius {bad}");
    }
}

#[test]
fn sticky_split_keeps_curve_geometry() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 12);
    // A chord across the circle splits facets; fragments inherit the chain
    // and the chain keeps its analytic definition.
    s.add_segment(pt(-2.0, 0.1), pt(2.0, 0.1)).unwrap();
    let g = s.curve_geom(id).expect("split does not drop geometry");
    assert!(g.center.approx_eq(pt(0.0, 0.0), tol::POINT_MERGE));
    assert_eq!(g.radius, 1.0);
}

#[test]
fn move_vertex_drops_touched_chain_geometry() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 12);
    let v = vertex_at(&s, pt(1.0, 0.0));
    // Nudge one facet corner inward: still topology-preserving, but the
    // chain no longer lies on its drawn circle.
    s.move_vertex(v, pt(0.9, 0.0)).unwrap();
    assert_eq!(s.curve_geom(id), None, "deformed chain drops its circle");
    // Identity survives: the chain still selects as one unit.
    let tagged = s.edges().values().filter(|e| e.curve == Some(id)).count();
    assert_eq!(tagged, 12);
}

#[test]
fn sketch_translate_and_uniform_scale_map_curve_geometry() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(1.0, 0.0), 0.5, 12);

    s.apply_transform(&kernel::Transform::translation(kernel::Vec3::new(
        2.0, 3.0, 0.0,
    )))
    .unwrap();
    let g = s.curve_geom(id).expect("translation maps geometry");
    assert!(g.center.approx_eq(pt(3.0, 3.0), 1e-12));
    assert_eq!(g.radius, 0.5);

    s.apply_transform(&kernel::Transform::uniform_scale(2.0))
        .unwrap();
    let g = s.curve_geom(id).expect("uniform scale maps geometry");
    assert!(g.center.approx_eq(pt(6.0, 6.0), 1e-12));
    assert!((g.radius - 1.0).abs() < 1e-12);
}

#[test]
fn nonuniform_in_plane_scale_drops_curve_geometry() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(1.0, 1.0), 0.5, 12);
    // x-only stretch: the drawn circle becomes an ellipse the metadata
    // cannot describe — geometry drops, identity stays.
    s.apply_transform(&kernel::Transform::scale(kernel::Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    assert_eq!(s.curve_geom(id), None);
    assert!(s.edges().values().any(|e| e.curve == Some(id)));
}

#[test]
fn island_move_maps_only_the_moved_chain() {
    let mut s = xy_sketch();
    let moved = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 12);
    let bystander = circle_chain(&mut s, pt(10.0, 0.0), 1.0, 12);

    let island = s
        .island_of_edge(
            s.edges()
                .iter()
                .find(|(_, e)| e.curve == Some(moved))
                .map(|(id, _)| id)
                .unwrap(),
        )
        .unwrap();
    s.apply_transform_island(
        island,
        &kernel::Transform::translation(kernel::Vec3::new(3.0, 0.0, 0.0)),
    )
    .unwrap();

    let g = s.curve_geom(moved).expect("moved chain maps");
    assert!(g.center.approx_eq(pt(3.0, 0.0), 1e-12));
    let b = s.curve_geom(bystander).expect("bystander untouched");
    assert!(b.center.approx_eq(pt(10.0, 0.0), 1e-12));
}

// -------------------------------------------------------- curve rims
//
// `Sketch::curve_rims` publishes each drawn curve's exact circle (with the
// angular range its surviving edges cover) so inference can offer a sketch
// circle/arc's true center and quadrant points BEFORE any extrusion exists —
// the sketch-level analogue of `Object::analytic_rims`.

#[test]
fn curve_rims_full_circle_offers_center_and_all_quadrants() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(1.0, 2.0), 0.5, 24);

    let rims = s.curve_rims();
    assert_eq!(rims.len(), 1);
    let rim = &rims[0];
    assert_eq!(rim.curve, id);
    assert!(rim.center.approx_eq(pt(1.0, 2.0), 1e-12));
    assert_eq!(rim.radius, 0.5);
    // Full circle: coverage is the whole turn, all four quadrants covered.
    assert_eq!(rim.coverage, None);
    let quads = rim.quadrant_points();
    assert_eq!(quads.len(), 4);
    for q in &quads {
        assert!(((q.to_vec() - rim.center.to_vec()).length() - 0.5).abs() < 1e-12);
        assert!(q.z.abs() < 1e-12);
    }
    // The rim's frame lies in the sketch plane.
    assert!(rim.axis.dot(kernel::Vec3::new(0.0, 0.0, 1.0)).abs() > 1.0 - 1e-12);
}

#[test]
fn curve_rims_arc_covers_only_its_swept_range() {
    let mut s = xy_sketch();
    // A half-turn arc from angle 0 to pi around the origin, radius 1.
    let center = pt(0.0, 0.0);
    s.begin_curve_with(kernel::CurveGeom {
        center,
        radius: 1.0,
    })
    .unwrap();
    let n = 12;
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / (n as f64);
        pt(a.cos(), a.sin())
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();

    let rims = s.curve_rims();
    assert_eq!(rims.len(), 1);
    let rim = &rims[0];
    assert!(rim.center.approx_eq(center, 1e-12));
    // Covered: the swept half (world +Y side). Uncovered: the missing half
    // (world −Y side). `covers` takes angles in the rim's own basis frame,
    // so map the world directions through it rather than assuming
    // basis_u == world X.
    let frame_angle = |d: kernel::Vec3| d.dot(rim.basis_v).atan2(d.dot(rim.basis_u));
    assert!(rim.covers(frame_angle(kernel::Vec3::new(0.0, 1.0, 0.0))));
    assert!(!rim.covers(frame_angle(kernel::Vec3::new(0.0, -1.0, 0.0))));
    // Cardinals +X, +Y, −X survive (arc endpoints inclusive); −Y does not.
    assert_eq!(rim.quadrant_points().len(), 3);
}

#[test]
fn curve_rims_skips_plain_lines_and_dead_curves() {
    let mut s = xy_sketch();
    // Plain segments never publish a rim.
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    assert!(s.curve_rims().is_empty());

    // A curve whose every edge has been deleted no longer publishes one.
    let id = circle_chain(&mut s, pt(5.0, 5.0), 1.0, 24);
    assert_eq!(s.curve_rims().len(), 1);
    for eid in s.curve_edges(id) {
        s.remove_edge(eid).unwrap();
    }
    assert!(s.curve_rims().is_empty());
}

#[test]
fn curve_rims_trims_coverage_when_part_of_the_circle_is_deleted() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);

    // Delete the facets crossing angle 3pi/2 (the -Y cardinal): edges whose
    // midpoint angle falls in the bottom quarter turn.
    for eid in s.curve_edges(id) {
        let e = s.edges()[eid];
        let a = s.vertices()[e.from].position;
        let b = s.vertices()[e.to].position;
        let mid_angle = ((a.y + b.y) * 0.5).atan2((a.x + b.x) * 0.5);
        if mid_angle < -std::f64::consts::FRAC_PI_4
            && mid_angle > -3.0 * std::f64::consts::FRAC_PI_4
        {
            s.remove_edge(eid).unwrap();
        }
    }

    let rims = s.curve_rims();
    assert_eq!(rims.len(), 1);
    let rim = &rims[0];
    // The center survives (an arc still exists) but the deleted cardinal is
    // no longer covered while the opposite one still is (frame-mapped, as in
    // the arc spec above).
    let frame_angle = |d: kernel::Vec3| d.dot(rim.basis_v).atan2(d.dot(rim.basis_u));
    assert!(rim.coverage.is_some());
    assert!(!rim.covers(frame_angle(kernel::Vec3::new(0.0, -1.0, 0.0))));
    assert!(rim.covers(frame_angle(kernel::Vec3::new(0.0, 1.0, 0.0))));
    assert_eq!(rim.quadrant_points().len(), 3);
}

#[test]
fn curve_rims_work_on_a_standing_plane() {
    // An upright sketch (the y = 2 plane, normal +Y): a circle drawn there
    // publishes its rim with the frame in that plane — the detached-sketch
    // case the ground-only path would miss.
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 2.0, 0.0),
        Point3::new(1.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 1.0),
    ])
    .unwrap();
    let mut s = Sketch::on_plane(plane);
    let center = Point3::new(1.0, 2.0, 1.0);
    s.begin_curve_with(kernel::CurveGeom {
        center,
        radius: 0.25,
    })
    .unwrap();
    let n = 24;
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(1.0 + 0.25 * a.cos(), 2.0, 1.0 + 0.25 * a.sin())
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();

    let rims = s.curve_rims();
    assert_eq!(rims.len(), 1);
    let rim = &rims[0];
    assert!(rim.center.approx_eq(center, 1e-12));
    assert_eq!(rim.coverage, None);
    assert!(rim.axis.dot(kernel::Vec3::new(0.0, 1.0, 0.0)).abs() > 1.0 - 1e-12);
    for q in rim.quadrant_points() {
        assert!((q.y - 2.0).abs() < 1e-12); // stays in the standing plane
    }
}

/// The midpoint angle (degrees, in `[0, 360)`) of `eid`'s chord about the
/// world origin — used only to bucket a 12-facet circle's edges into
/// quarters by where they sit, independent of slotmap/id order.
fn edge_mid_angle_deg(s: &Sketch, eid: kernel::SketchEdgeId) -> f64 {
    let e = s.edges()[eid];
    let a = s.vertices()[e.from].position;
    let b = s.vertices()[e.to].position;
    let deg = ((a.y + b.y) * 0.5).atan2((a.x + b.x) * 0.5).to_degrees();
    if deg < 0.0 { deg + 360.0 } else { deg }
}

/// tool-parity playtest2 (Part 1): the rosette bug's root cause is that
/// `curve_rims` unions angular coverage over every live edge sharing a
/// curve id with NO connectivity check — so once a drawn circle has been
/// split into several DISCONNECTED chain fragments (a rotate-copy array
/// crossing it at multiple points, exactly like `curve_chain_stops_at_junctions`)
/// and the user deletes some of those fragments, does `curve_rims` still
/// report a span bridging the gap where a deleted fragment used to be? If it
/// did, quadrant/tangent snaps would be offered on an arc that no longer
/// exists — a second latent defect in the same family as the selection bug.
///
/// This is a read-first spec: it asserts the CORRECT (non-bridging) behavior
/// and is expected to already pass, since `merge_angular_intervals` only
/// merges intervals that are angularly adjacent (within `tol::POINT_MERGE`),
/// never across a real gap. It stands as the regression lock for that
/// finding either way.
#[test]
fn curve_rims_does_not_bridge_coverage_across_disconnected_surviving_fragments() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 12);

    // Two diameters through the circle's own quadrant vertices (0/90/180/270
    // are all facet vertices at n = 12) split the ring into four 90-degree
    // chains WITHOUT splitting any circle edge itself — junctions land
    // exactly on existing vertices, the same mechanism
    // `curve_chain_stops_at_junctions` pins for a diamond.
    s.add_segment(pt(1.0, 0.0), pt(-1.0, 0.0)).unwrap();
    s.add_segment(pt(0.0, 1.0), pt(0.0, -1.0)).unwrap();

    // Delete the second and fourth quarters — the "interior petal" fragments
    // in Kurt's repro — leaving two disjoint surviving arcs: [0,90] and
    // [180,270], with a genuine 90-degree gap on both sides.
    for eid in s.curve_edges(id) {
        let deg = edge_mid_angle_deg(&s, eid);
        if (90.0..180.0).contains(&deg) || (270.0..360.0).contains(&deg) {
            s.remove_edge(eid).unwrap();
        }
    }
    assert_eq!(s.curve_edges(id).len(), 6, "two 3-edge quarters survive");

    let rims = s.curve_rims();
    assert_eq!(rims.len(), 1);
    let rim = &rims[0];
    let coverage = rim
        .coverage
        .clone()
        .expect("partial coverage, not the full-circle None");
    assert_eq!(
        coverage.len(),
        2,
        "two disjoint surviving arcs must stay separate entries, never bridged into one: {coverage:?}"
    );

    let frame_angle = |d: kernel::Vec3| d.dot(rim.basis_v).atan2(d.dot(rim.basis_u));
    // Interior of each surviving quarter: covered.
    assert!(rim.covers(frame_angle(kernel::Vec3::new(1.0, 1.0, 0.0)))); // ~45deg, inside [0,90]
    assert!(rim.covers(frame_angle(kernel::Vec3::new(-1.0, -1.0, 0.0)))); // ~225deg, inside [180,270]
    // Interior of each DELETED quarter — the phantom span, if one existed —
    // must read as NOT covered: no quadrant/tangent candidate belongs there.
    assert!(!rim.covers(frame_angle(kernel::Vec3::new(-1.0, 1.0, 0.0)))); // ~135deg, deleted
    assert!(!rim.covers(frame_angle(kernel::Vec3::new(1.0, -1.0, 0.0)))); // ~315deg, deleted
}

/// Kurt's actual goal in the repro isn't just "select the right fragment" —
/// it's "delete it and keep the rest of the flower". The app's delete path
/// re-derives the connectivity-aware chain via `curve_chain_at` (never the
/// connectivity-blind `curve_edges`) before erasing
/// (`Viewport.tsx`'s `runDelete`/`removeEdgeBatch`), so deleting one
/// fragment of a curve id split into several disconnected pieces must leave
/// every sibling fragment — including ones sharing the very same curve id —
/// completely untouched. This pins that contract at the kernel level, where
/// `curve_chain_at` and `remove_edge` actually live.
#[test]
fn deleting_one_disconnected_fragment_via_its_chain_leaves_sibling_fragments_untouched() {
    let mut s = xy_sketch();
    let id = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 12);
    s.add_segment(pt(1.0, 0.0), pt(-1.0, 0.0)).unwrap();
    s.add_segment(pt(0.0, 1.0), pt(0.0, -1.0)).unwrap();

    let quarter = |s: &Sketch, lo: f64, hi: f64| -> Vec<kernel::SketchEdgeId> {
        s.curve_edges(id)
            .into_iter()
            .filter(|&e| (lo..hi).contains(&edge_mid_angle_deg(s, e)))
            .collect()
    };
    let q0 = quarter(&s, 0.0, 90.0);
    let q1 = quarter(&s, 90.0, 180.0);
    let q2 = quarter(&s, 180.0, 270.0);
    let q3 = quarter(&s, 270.0, 360.0);
    assert_eq!([q0.len(), q1.len(), q2.len(), q3.len()], [3, 3, 3, 3]);

    // Delete only q1's chain, exactly as the app's delete path does: derive
    // the connected chain from one of its edges, then erase each member.
    let chain = s.curve_chain_at(q1[0]);
    assert_eq!(
        chain.len(),
        3,
        "the chain is exactly this one quarter, never the whole curve id"
    );
    for &eid in &chain {
        s.remove_edge(eid).unwrap();
    }

    let remaining = s.curve_edges(id);
    assert_eq!(remaining.len(), 9, "the other three quarters survive");
    for eid in q0.iter().chain(q2.iter()).chain(q3.iter()) {
        assert!(
            remaining.contains(eid),
            "sibling fragment edge {eid:?} survives"
        );
    }
    for eid in &q1 {
        assert!(
            !remaining.contains(eid),
            "the deleted fragment is actually gone"
        );
    }
    // Survivors are each still one coherent connected chain, not fragmented
    // further by the deletion.
    assert_eq!(s.curve_chain_at(q0[0]).len(), 3);
    assert_eq!(s.curve_chain_at(q2[0]).len(), 3);
    assert_eq!(s.curve_chain_at(q3[0]).len(), 3);
}

// ------------------------------------------------------- refacet_curve
//
// Editing a drawn circle's segment count is a REBUILD of that circle, not a
// setting for the next one: the chain keeps its handle and its analytic
// circle, the facets are replaced. These pin the contract in
// `Sketch::refacet_curve`.

/// Every vertex the chain `curve` touches, as its angle about `center` in the
/// XY plane, sorted. Used to compare rings independently of edge id order.
fn ring_angles(s: &Sketch, curve: kernel::SketchCurveId, center: Point3) -> Vec<f64> {
    let mut vs: std::collections::BTreeSet<SketchVertexId> = std::collections::BTreeSet::new();
    for eid in s.curve_edges(curve) {
        let e = s.edges()[eid];
        vs.insert(e.from);
        vs.insert(e.to);
    }
    let mut angles: Vec<f64> = vs
        .into_iter()
        .map(|v| {
            let p = s.vertices()[v].position;
            (p.y - center.y).atan2(p.x - center.x)
        })
        .collect();
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
    angles
}

#[test]
fn refacet_curve_rebuilds_the_ring_and_preserves_the_circle() {
    let mut s = xy_sketch();
    let center = pt(1.0, 2.0);
    let curve = circle_chain(&mut s, center, 0.75, 24);
    assert_eq!(s.regions().len(), 1, "the drawn circle closes a region");
    let before = s.curve_geom(curve).unwrap();

    let report = s.refacet_curve(curve, 48).expect("24 -> 48 re-facets");

    assert_eq!(s.curve_edges(curve).len(), 48, "48 facets now exist");
    assert_eq!(report.new_edges.len(), 48);
    assert_eq!(report.removed_edges.len(), 24);
    assert_eq!(
        s.curve_geom(curve),
        Some(before),
        "the chain keeps its handle AND its exact circle"
    );
    assert_eq!(s.regions().len(), 1, "the region still closes");
    assert_eq!(
        s.edges().len(),
        48,
        "no orphan edges from the old ring survive"
    );
    assert_eq!(s.vertices().len(), 48, "nor orphan vertices");

    // Every new vertex is ON the exact circle.
    for a in ring_angles(&s, curve, center) {
        let p = pt(center.x + 0.75 * a.cos(), center.y + 0.75 * a.sin());
        let hit = s
            .vertices()
            .values()
            .any(|v| v.position.approx_eq(p, tol::POINT_MERGE));
        assert!(hit, "vertex at angle {a} lies on the circle");
    }

    // And it is still a genuine analytic circle to inference: one full rim.
    let rims = s.curve_rims();
    assert_eq!(rims.len(), 1);
    assert_eq!(rims[0].coverage, None, "still the full circle");
}

/// The rebuilt ring is ANCHORED on the old one: it reuses one of the old
/// ring's vertex positions exactly, so a circle whose count is edited does
/// not appear to spin. Stated as "a vertex survives" rather than "the start
/// angle survives" because the anchor is chosen in the sketch plane's own
/// basis frame, which need not agree with world XY.
#[test]
fn refacet_curve_anchors_the_new_ring_on_an_old_vertex() {
    let mut s = xy_sketch();
    let center = pt(0.0, 0.0);
    // Drawn deliberately off a cardinal direction, so a rebuild that reset
    // the phase to zero would be obvious.
    let n = 24usize;
    let phase = 0.31_f64;
    let curve = s
        .begin_curve_with(kernel::CurveGeom {
            center,
            radius: 1.0,
        })
        .unwrap();
    let p = |i: usize| {
        let a = phase + std::f64::consts::TAU * (i as f64) / (n as f64);
        pt(a.cos(), a.sin())
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let old: Vec<Point3> = s.vertices().values().map(|v| v.position).collect();

    s.refacet_curve(curve, 30).expect("re-facets");

    assert!(
        s.vertices().values().any(|v| old
            .iter()
            .any(|o| v.position.approx_eq(*o, tol::POINT_MERGE))),
        "the rebuilt ring shares a vertex with the ring it replaced"
    );
}

#[test]
fn refacet_curve_refuses_counts_outside_the_allowed_range() {
    let mut s = xy_sketch();
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);
    let before = s.clone();

    for n in [0, 1, 3, kernel::MIN_CIRCLE_SEGMENTS - 1] {
        assert_eq!(
            s.refacet_curve(curve, n),
            Err(SketchError::SegmentsBelowFloor),
            "{n} is below the floor"
        );
    }
    assert_eq!(
        s.refacet_curve(curve, kernel::MAX_CIRCLE_SEGMENTS + 1),
        Err(SketchError::SegmentsAboveCap)
    );
    // The floor itself is allowed — the control raises the count, it never
    // lowers it past the density at which a ring stops being a circle.
    assert!(
        s.clone()
            .refacet_curve(curve, kernel::MIN_CIRCLE_SEGMENTS)
            .is_ok()
    );
    assert_eq!(s, before, "a refused re-facet leaves the sketch untouched");
}

#[test]
fn refacet_curve_refuses_unknown_and_identity_only_chains() {
    let mut s = xy_sketch();
    let plain = s.begin_curve();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    s.end_curve();
    assert_eq!(
        s.refacet_curve(plain, 24),
        Err(SketchError::CurveNotAnalytic)
    );

    // A handle naming no chain at all — what a fabricated or stale handle
    // arriving across the wasm boundary looks like. (A handle borrowed from
    // ANOTHER sketch is not the test for this: slotmap keys are per-map, so a
    // foreign key can legitimately name a live local chain.)
    use slotmap::Key as _;
    assert_eq!(
        s.refacet_curve(kernel::SketchCurveId::null(), 24),
        Err(SketchError::UnknownCurve)
    );
}

#[test]
fn refacet_curve_refuses_a_polygon() {
    let mut s = xy_sketch();
    let curve = s
        .begin_curve_with_kind(
            kernel::CurveGeom {
                center: pt(0.0, 0.0),
                radius: 1.0,
            },
            kernel::SketchCurveKind::Polygon,
        )
        .unwrap();
    let p = |i: usize| {
        let a = std::f64::consts::TAU * (i as f64) / 24.0;
        pt(a.cos(), a.sin())
    };
    for i in 0..24 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let before = s.clone();

    // Dense enough to pass every count check — it is refused for WHAT it is,
    // not for how many sides it has.
    assert_eq!(
        s.refacet_curve(curve, 48),
        Err(SketchError::CurveNotRefacetable)
    );
    assert_eq!(s, before);
}

#[test]
fn refacet_curve_refuses_a_circle_glued_to_other_geometry() {
    let mut s = xy_sketch();
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);
    // A chord across the circle: both ends land on the ring, splitting two
    // facets and making four junction vertices.
    s.add_segment(pt(-1.0, 0.0), pt(1.0, 0.0)).unwrap();
    let before = s.clone();

    assert_eq!(
        s.refacet_curve(curve, 48),
        Err(SketchError::CurveNotRefacetable),
        "rebuilding would silently destroy the junction"
    );
    assert_eq!(s, before, "and leaves the sketch exactly as it was");
}

#[test]
fn refacet_curve_refuses_a_partially_erased_circle() {
    let mut s = xy_sketch();
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);
    let victim = s.curve_edges(curve)[0];
    s.remove_edge(victim).unwrap();
    let before = s.clone();

    assert_eq!(
        s.refacet_curve(curve, 48),
        Err(SketchError::CurveNotRefacetable),
        "an arc is not a circle to re-facet"
    );
    assert_eq!(s, before);
}

#[test]
fn refacet_curve_refuses_when_the_denser_ring_would_reach_a_neighbour() {
    // A denser ring sits CLOSER to the exact circle, so it can meet geometry
    // the old coarse ring cleared. That has to be seen, not assumed away.
    let mut s = xy_sketch();
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);
    // The 24-gon's apothem is cos(pi/24) ≈ 0.99144; the exact circle is at
    // 1.0. A segment at x = 0.995 misses every chord of the 24-gon and
    // crosses the 96-gon (apothem ≈ 0.99946).
    s.add_segment(pt(0.995, -0.5), pt(0.995, 0.5)).unwrap();
    let before = s.clone();

    assert_eq!(
        s.refacet_curve(curve, 96),
        Err(SketchError::CurveNotRefacetable)
    );
    assert_eq!(s, before, "refused, with the sketch untouched");
}

proptest! {
    /// The round-trip property: for any n at or above the floor, re-faceting
    /// to n and back leaves the chain's analytic circle untouched, the ring
    /// at the density it started at, and the region still closed.
    #[test]
    fn refacet_round_trip_preserves_the_circle_and_the_region(
        n in kernel::MIN_CIRCLE_SEGMENTS..=200usize,
    ) {
        let mut s = xy_sketch();
        let center = pt(0.25, -0.5);
        let start = 24usize;
        let curve = circle_chain(&mut s, center, 1.5, start);
        let geom = s.curve_geom(curve).unwrap();

        s.refacet_curve(curve, n).unwrap();
        prop_assert_eq!(s.curve_edges(curve).len(), n);
        prop_assert_eq!(s.regions().len(), 1);

        s.refacet_curve(curve, start).unwrap();
        prop_assert_eq!(s.curve_edges(curve).len(), start);
        prop_assert_eq!(s.curve_geom(curve), Some(geom));
        prop_assert_eq!(s.regions().len(), 1);
        prop_assert_eq!(s.curve_rims().len(), 1);
        prop_assert_eq!(s.curve_rims()[0].coverage.clone(), None);

        // Every vertex is ON the exact circle, to the kernel's own point
        // tolerance — the facets approximate the circle, they never drift
        // off it.
        for v in s.vertices().values() {
            let r = (v.position - geom.center).length();
            prop_assert!(
                (r - geom.radius).abs() <= tol::POINT_MERGE,
                "vertex at radius {} vs {}", r, geom.radius
            );
        }

        // The ring is regular: `start` vertices, evenly spaced.
        let angles = ring_angles(&s, curve, center);
        prop_assert_eq!(angles.len(), start);
        let step = std::f64::consts::TAU / (start as f64);
        for w in angles.windows(2) {
            prop_assert!((w[1] - w[0] - step).abs() < 1e-9, "even spacing");
        }
    }

    /// No-visible-rotation, stated as the property it actually is: ONE
    /// re-facet reuses one of the old ring's vertex positions, whatever the
    /// new density. (Ring position is NOT preserved across a round trip and
    /// is deliberately not claimed — two densities put their vertices on
    /// different angular lattices, so returning to the original count cannot
    /// in general return to the original vertices.)
    #[test]
    fn refacet_anchors_the_new_ring_on_an_old_vertex(
        n in kernel::MIN_CIRCLE_SEGMENTS..=200usize,
    ) {
        let mut s = xy_sketch();
        let center = pt(-2.0, 0.5);
        let curve = circle_chain(&mut s, center, 0.9, 24);
        let old: Vec<Point3> = s.vertices().values().map(|v| v.position).collect();

        s.refacet_curve(curve, n).unwrap();

        prop_assert!(
            s.vertices()
                .values()
                .any(|v| old.iter().any(|o| v.position.approx_eq(*o, tol::POINT_MERGE))),
            "the rebuilt {}-gon shares no vertex with the 24-gon it replaced", n
        );
    }
}

#[test]
fn refacet_curve_refuses_two_disjoint_rings_under_one_chain() {
    // Degree-2-everywhere admits a disjoint UNION of cycles, and a ring that
    // closes a wide gap with one long secant already covers the full turn by
    // itself — leaving the circular segment behind that secant free for a
    // second, wholly separate ring on the same circle. Without a connectivity
    // check that state passes every other gate, and the rebuild deletes the
    // second ring in silence.
    //
    // Reachable through the ordinary API: one curve bracket, twelve segments.
    let mut s = xy_sketch();
    let on_circle = |deg: f64| {
        let a = deg.to_radians();
        pt(a.cos(), a.sin())
    };

    let curve = s
        .begin_curve_with(kernel::CurveGeom {
            center: pt(0.0, 0.0),
            radius: 1.0,
        })
        .unwrap();
    // Ring A: 0°,30°,…,240°, closed by ONE long secant 240° → 0°.
    for k in 0..8 {
        s.add_segment(on_circle(k as f64 * 30.0), on_circle((k + 1) as f64 * 30.0))
            .unwrap();
    }
    s.add_segment(on_circle(240.0), on_circle(0.0)).unwrap();
    // Ring B: a triangle inscribed inside the segment cut off by that secant.
    // None of its chords crosses ring A, so the sticky rules insert it clean.
    for (a, b) in [(280.0, 300.0), (300.0, 320.0), (320.0, 280.0)] {
        s.add_segment(on_circle(a), on_circle(b)).unwrap();
    }
    s.end_curve();

    // The state really is two separate rings under one chain id.
    assert_eq!(s.curve_edges(curve).len(), 12);
    assert_eq!(s.islands().len(), 2, "two disjoint rings");
    let before = s.clone();

    assert_eq!(
        s.refacet_curve(curve, 24),
        Err(SketchError::CurveNotRefacetable),
        "refusing beats silently deleting the second ring"
    );
    assert_eq!(s, before, "and the sketch is untouched");
}

#[test]
fn refacet_curve_updates_a_region_that_holds_the_circle_as_a_hole() {
    // A circle drawn inside a rectangle is a hole in the rectangle's region as
    // well as a region of its own. Re-faceting replaces every vertex of that
    // hole boundary, so the enclosing region has to be refreshed too — a
    // region keeps its id when its OUTER cycle is unchanged, which is exactly
    // this case, so the hole list must be rewritten in place rather than left
    // pointing at deleted vertices.
    let mut s = xy_sketch();
    for (a, b) in rect_segments(-5.0, -5.0, 5.0, 5.0) {
        s.add_segment(a, b).unwrap();
    }
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);

    let outer_id = s
        .regions()
        .iter()
        .find(|(_, r)| r.outer.len() == 4)
        .map(|(id, _)| id)
        .expect("the rectangle's region");
    let hole_before = s.regions()[outer_id].holes[0].clone();
    assert_eq!(hole_before.len(), 24);

    s.refacet_curve(curve, 48).expect("the circle re-facets");

    // The enclosing region survived with its id, and its hole now tracks the
    // rebuilt ring.
    let hole_after = s.regions()[outer_id].holes[0].clone();
    assert_eq!(hole_after.len(), 48, "the hole tracks the rebuilt ring");
    for v in &hole_after {
        assert!(
            s.vertices().contains_key(*v),
            "the hole must not reference a deleted vertex"
        );
    }
    for v in &hole_before {
        assert!(
            !hole_after.contains(v),
            "no vertex of the old ring may survive in the hole"
        );
    }
}

#[test]
fn refacet_curve_leaves_no_stale_edge_handle_aliasing_a_new_edge() {
    // Re-faceting to the SAME count is the adversarial case for handles: it
    // frees exactly as many edge slots as it then allocates, so the slotmap
    // reuses every index. Handles are generational, so a handle held across
    // the rebuild — the app's `sketch-curve` selection holds one — must fail
    // to resolve rather than silently name a different edge at the same slot.
    let mut s = xy_sketch();
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);
    let old_edges = s.curve_edges(curve);

    s.refacet_curve(curve, 24).expect("re-facets");

    for old in &old_edges {
        assert!(
            !s.edges().contains_key(*old),
            "a stale edge handle resolved to a live edge"
        );
    }
    // Non-vacuous only if the slots really were reused; assert that too, so
    // this cannot quietly become a test of a case that no longer arises.
    let slot = |e: &kernel::SketchEdgeId| {
        use slotmap::Key as _;
        e.data().as_ffi() & 0xffff_ffff
    };
    let old_slots: std::collections::BTreeSet<u64> = old_edges.iter().map(slot).collect();
    let new_slots: std::collections::BTreeSet<u64> =
        s.curve_edges(curve).iter().map(slot).collect();
    assert_eq!(
        old_slots, new_slots,
        "the rebuild should reuse the freed slots, or this proves nothing"
    );
}

#[test]
fn refacet_curve_mints_a_new_island_id_as_documented() {
    // `CurveRefaceted`'s doc comment states island ids do NOT survive a
    // re-facet and are deliberately unreported. Pin that, so the claim cannot
    // silently become false — a caller reading the doc is entitled to rely on
    // re-querying being necessary, and equally on it being sufficient.
    let mut s = xy_sketch();
    let curve = circle_chain(&mut s, pt(0.0, 0.0), 1.0, 24);
    assert_eq!(s.islands().len(), 1);
    let before = s.islands().keys().next().unwrap();

    s.refacet_curve(curve, 48).expect("re-facets");

    assert_eq!(s.islands().len(), 1, "still exactly one island");
    let after = s.islands().keys().next().unwrap();
    assert_ne!(before, after, "the island is a new one, as documented");
    // And re-querying is sufficient: the fresh id resolves to the whole ring.
    assert_eq!(s.islands()[after].edges.len(), 48);
}

// ============================================================================
// Stage 0 of unified drafting: a moved shape arranges exactly like a drawn
// one; contact is metric (length-independent); history replays byte-exactly.
// ============================================================================

use kernel::{Document, SketchIslandId, Transform, Vec3};

/// One region as comparable data: its outer ring plus its hole rings.
#[derive(Debug, Clone)]
struct RegionRings {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
}

fn region_rings(s: &Sketch) -> Vec<RegionRings> {
    s.regions()
        .values()
        .map(|r| {
            let ring = |ids: &[SketchVertexId]| -> Vec<Point3> {
                ids.iter().map(|&v| s.vertices()[v].position).collect()
            };
            RegionRings {
                outer: ring(&r.outer),
                holes: r.holes.iter().map(|h| ring(h)).collect(),
            }
        })
        .collect()
}

/// Cyclic ring equality within `tol::POINT_MERGE`.
fn ring_eq(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len()
        && (0..a.len())
            .any(|k| (0..a.len()).all(|i| a[i].approx_eq(b[(i + k) % a.len()], tol::POINT_MERGE)))
}

/// Multiset equality under a pairwise predicate (greedy matching is exact
/// here because equal rings are interchangeable).
fn multiset_eq<T>(a: &[T], b: &[T], eq: impl Fn(&T, &T) -> bool) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut used = vec![false; b.len()];
    a.iter()
        .all(|x| match (0..b.len()).find(|&j| !used[j] && eq(x, &b[j])) {
            Some(j) => {
                used[j] = true;
                true
            }
            None => false,
        })
}

fn regions_eq(a: &Sketch, b: &Sketch) -> bool {
    let (ra, rb) = (region_rings(a), region_rings(b));
    multiset_eq(&ra, &rb, |x, y| {
        ring_eq(&x.outer, &y.outer) && multiset_eq(&x.holes, &y.holes, |p, q| ring_eq(p, q))
    })
}

/// The edge set as sorted, undirected, quantized endpoint pairs.
fn edge_set(s: &Sketch) -> Vec<((i64, i64), (i64, i64))> {
    let q = |p: Point3| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64);
    let mut v: Vec<_> = s
        .edges()
        .values()
        .map(|e| {
            let (a, b) = (
                q(s.vertices()[e.from].position),
                q(s.vertices()[e.to].position),
            );
            if a <= b { (a, b) } else { (b, a) }
        })
        .collect();
    v.sort_unstable();
    v
}

fn vertex_xy(s: &Sketch, x: f64, y: f64) -> SketchVertexId {
    s.vertices()
        .iter()
        .find(|(_, v)| v.position.approx_eq(pt(x, y), tol::POINT_MERGE))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no vertex at ({x}, {y})"))
}

fn island_xy(s: &Sketch, x: f64, y: f64) -> SketchIslandId {
    let v = vertex_xy(s, x, y);
    let e = s
        .edges()
        .iter()
        .find(|(_, e)| e.from == v || e.to == v)
        .map(|(id, _)| id)
        .expect("vertex has an edge");
    s.island_of_edge(e).expect("edge has an island")
}

fn draw_rect_into(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) {
    for (a, b) in rect_segments(x0, y0, x1, y1) {
        s.add_segment(a, b).unwrap();
    }
}

fn shift(r: (f64, f64, f64, f64), dx: f64, dy: f64) -> (f64, f64, f64, f64) {
    (r.0 + dx, r.1 + dy, r.2 + dx, r.3 + dy)
}

fn translate(dx: f64, dy: f64) -> Transform {
    Transform::translation(Vec3::new(dx, dy, 0.0))
}

type Rect = (f64, f64, f64, f64);

/// Draw `fixed`, draw `moved` far away, move it by `(dx, dy)`; compare with a
/// fresh sketch where `fixed` and the translated `moved` were just drawn.
fn assert_move_equals_draw(fixed: Rect, moved: Rect, dx: f64, dy: f64, expect_regions: usize) {
    let mut s = xy_sketch();
    draw_rect_into(&mut s, fixed.0, fixed.1, fixed.2, fixed.3);
    draw_rect_into(&mut s, moved.0, moved.1, moved.2, moved.3);
    let island = island_xy(&s, moved.0, moved.1);
    s.apply_transform_island(island, &translate(dx, dy))
        .expect("the move is accepted");

    let mut drawn = xy_sketch();
    draw_rect_into(&mut drawn, fixed.0, fixed.1, fixed.2, fixed.3);
    let m = shift(moved, dx, dy);
    draw_rect_into(&mut drawn, m.0, m.1, m.2, m.3);

    assert_eq!(drawn.regions().len(), expect_regions, "fixture sanity");
    assert_eq!(edge_set(&s), edge_set(&drawn), "same edges");
    assert!(
        regions_eq(&s, &drawn),
        "moved: {:?}\ndrawn: {:?}",
        region_rings(&s),
        region_rings(&drawn)
    );
}

#[test]
fn move_onto_exact_coincidence_equals_draw_in_place() {
    assert_move_equals_draw((0.0, 0.0, 2.0, 2.0), (5.0, 0.0, 7.0, 2.0), -5.0, 0.0, 1);
}

#[test]
fn move_onto_half_overlap_equals_draw_in_place() {
    assert_move_equals_draw((0.0, 0.0, 2.0, 2.0), (5.0, 0.0, 7.0, 2.0), -6.0, 0.0, 3);
}

#[test]
fn move_across_a_corner_equals_draw_in_place() {
    assert_move_equals_draw((0.0, 0.0, 2.0, 2.0), (5.0, 5.0, 7.0, 7.0), -4.0, -4.0, 3);
}

#[test]
fn move_inside_a_larger_rectangle_becomes_its_hole() {
    let mut s = xy_sketch();
    draw_rect_into(&mut s, 0.0, 0.0, 10.0, 10.0);
    draw_rect_into(&mut s, 20.0, 0.0, 22.0, 2.0);
    let island = island_xy(&s, 20.0, 0.0);
    s.apply_transform_island(island, &translate(-18.0, 3.0))
        .unwrap();
    let with_hole = region_rings(&s)
        .iter()
        .filter(|r| r.holes.len() == 1)
        .count();
    assert_eq!(
        with_hole, 1,
        "the big region adopts the moved shape as a hole"
    );
    assert_move_equals_draw(
        (0.0, 0.0, 10.0, 10.0),
        (20.0, 0.0, 22.0, 2.0),
        -18.0,
        3.0,
        2,
    );
}

#[test]
fn move_around_a_smaller_rectangle_adopts_it_as_a_hole() {
    let mut s = xy_sketch();
    draw_rect_into(&mut s, 4.0, 4.0, 5.0, 5.0);
    draw_rect_into(&mut s, 20.0, 0.0, 28.0, 8.0);
    let island = island_xy(&s, 20.0, 0.0);
    s.apply_transform_island(island, &translate(-18.0, 1.0))
        .unwrap();
    let with_hole = region_rings(&s)
        .iter()
        .filter(|r| r.holes.len() == 1)
        .count();
    assert_eq!(with_hole, 1, "the moved ring surrounds the small one");
    assert_move_equals_draw((4.0, 4.0, 5.0, 5.0), (20.0, 0.0, 28.0, 8.0), -18.0, 1.0, 2);
}

#[test]
fn batch_move_does_not_arrange_against_a_moved_islands_old_position() {
    // A lands where C WAS; C moves away in the same batch. The result must be
    // the final configuration drawn fresh, not A arranged with C's old edges.
    let mut s = xy_sketch();
    draw_rect_into(&mut s, 0.0, 0.0, 2.0, 2.0); // A
    draw_rect_into(&mut s, 4.0, 1.0, 6.0, 3.0); // C: overlaps A's destination
    let a = island_xy(&s, 0.0, 0.0);
    let c = island_xy(&s, 4.0, 1.0);
    assert_ne!(a, c);
    s.apply_transform_islands(&[a, c], &translate(3.0, 0.0))
        .unwrap();

    let mut drawn = xy_sketch();
    draw_rect_into(&mut drawn, 3.0, 0.0, 5.0, 2.0);
    draw_rect_into(&mut drawn, 7.0, 1.0, 9.0, 3.0);
    assert_eq!(edge_set(&s), edge_set(&drawn));
    assert!(regions_eq(&s, &drawn));
    assert_eq!(s.regions().len(), 2);
}

#[test]
fn vertex_drag_onto_an_edge_interior_splits_that_edge() {
    let build = |end: (f64, f64)| {
        let mut s = xy_sketch();
        s.add_segment(pt(10.0, 0.0), pt(14.0, 0.0)).unwrap();
        s.add_segment(pt(11.0, 2.0), pt(end.0, end.1)).unwrap();
        s
    };
    let mut s = build((12.0, 3.0));
    let v = vertex_xy(&s, 12.0, 3.0);
    s.move_vertex_arranging(v, pt(12.0, 0.0)).unwrap();
    let drawn = build((12.0, 0.0));
    assert_eq!(s.edges().len(), 3, "the long edge split in two");
    assert_eq!(edge_set(&s), edge_set(&drawn));
    assert!(regions_eq(&s, &drawn));
}

#[test]
fn vertex_drag_onto_a_non_adjacent_vertex_welds() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    s.add_segment(pt(1.0, 0.0), pt(1.0, 1.0)).unwrap();
    s.add_segment(pt(1.0, 1.0), pt(0.0, 1.0)).unwrap();
    let v = vertex_xy(&s, 0.0, 1.0);
    s.move_vertex_arranging(v, pt(0.0, 0.0)).unwrap();

    let mut drawn = xy_sketch();
    drawn.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    drawn.add_segment(pt(1.0, 0.0), pt(1.0, 1.0)).unwrap();
    drawn.add_segment(pt(1.0, 1.0), pt(0.0, 0.0)).unwrap();
    assert_eq!(s.vertices().len(), 3, "the two ends are one vertex");
    assert_eq!(s.regions().len(), 1, "the triangle closes");
    assert_eq!(edge_set(&s), edge_set(&drawn));
    assert!(regions_eq(&s, &drawn));
}

#[test]
fn vertex_drag_across_an_edge_splits_it() {
    let mut s = xy_sketch();
    s.add_segment(pt(3.0, 0.0), pt(3.0, 4.0)).unwrap();
    s.add_segment(pt(0.0, 1.0), pt(1.0, 1.0)).unwrap();
    let v = vertex_xy(&s, 1.0, 1.0);
    s.move_vertex_arranging(v, pt(5.0, 1.0)).unwrap();

    let mut drawn = xy_sketch();
    drawn.add_segment(pt(3.0, 0.0), pt(3.0, 4.0)).unwrap();
    drawn.add_segment(pt(0.0, 1.0), pt(5.0, 1.0)).unwrap();
    assert_eq!(edge_set(&s), edge_set(&drawn));
    assert_eq!(s.edges().len(), 4, "both lines split at the crossing");
    assert!(regions_eq(&s, &drawn));
}

#[test]
fn vertex_drag_onto_the_adjacent_vertex_still_refuses() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    s.add_segment(pt(1.0, 0.0), pt(1.0, 1.0)).unwrap();
    let v = vertex_xy(&s, 1.0, 0.0);
    let before = s.clone();
    let err = s.move_vertex_arranging(v, pt(0.0, 0.0)).unwrap_err();
    assert_eq!(err, SketchError::DegenerateSegment);
    assert!(s == before, "a refusal leaves the sketch untouched");
}

// -------------------------------------------------- history through Document

fn ground_plane() -> Plane {
    Plane::from_point_normal(pt(0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

fn doc_with_rects(rects: &[Rect]) -> (Document, kernel::SketchId) {
    let mut doc = Document::new();
    let sid = doc.add_sketch(ground_plane());
    doc.begin_sketch_gesture(sid).unwrap();
    for r in rects {
        draw_rect_into(doc.sketch_mut(sid).unwrap(), r.0, r.1, r.2, r.3);
    }
    doc.end_sketch_gesture(sid).unwrap();
    (doc, sid)
}

/// Run `op`, then undo and redo it; every state's save bytes must match the
/// state it restores.
fn assert_op_replays_byte_exactly(
    doc: &mut Document,
    op: impl FnOnce(&mut Document),
) -> Result<(), TestCaseError> {
    let before = doc.save();
    op(doc);
    let after = doc.save();
    if after == before {
        return Ok(()); // a refused / no-op edit records nothing
    }
    doc.undo().expect("undo succeeds");
    prop_assert_eq!(&doc.save(), &before, "undo is byte-identical");
    doc.redo().expect("redo succeeds");
    prop_assert_eq!(&doc.save(), &after, "redo is byte-identical");
    Ok(())
}

#[test]
fn arranging_island_move_undoes_and_redoes_byte_exactly() {
    let (mut doc, sid) = doc_with_rects(&[(0.0, 0.0, 2.0, 2.0), (5.0, 0.0, 7.0, 2.0)]);
    let island = island_xy(doc.sketch(sid).unwrap(), 5.0, 0.0);
    let before = doc.save();
    doc.transform_sketch_island(sid, island, &translate(-6.0, 0.0))
        .expect("the arranging move is accepted");
    let after = doc.save();
    assert_ne!(before, after);
    assert_eq!(doc.sketch(sid).unwrap().regions().len(), 3);
    doc.undo().unwrap();
    assert_eq!(doc.save(), before, "undo is byte-identical");
    doc.redo().unwrap();
    assert_eq!(doc.save(), after, "redo is byte-identical");
}

#[test]
fn arranging_vertex_drag_undoes_and_redoes_byte_exactly() {
    let (mut doc, sid) = doc_with_rects(&[(0.0, 0.0, 2.0, 2.0)]);
    {
        doc.begin_sketch_gesture(sid).unwrap();
        let s = doc.sketch_mut(sid).unwrap();
        s.add_segment(pt(3.0, 1.0), pt(4.0, 1.0)).unwrap();
        doc.end_sketch_gesture(sid).unwrap();
    }
    let v = vertex_xy(doc.sketch(sid).unwrap(), 3.0, 1.0);
    let before = doc.save();
    // Drag the loose end across the rectangle's right side.
    doc.move_sketch_vertex(sid, v, pt(1.0, 1.0))
        .expect("accepted");
    let after = doc.save();
    assert_ne!(before, after);
    doc.undo().unwrap();
    assert_eq!(doc.save(), before, "undo is byte-identical");
    doc.redo().unwrap();
    assert_eq!(doc.save(), after, "redo is byte-identical");
}

fn lattice_rect() -> impl Strategy<Value = Rect> {
    (0..12i32, 0..12i32, 1..6i32, 1..6i32).prop_map(|(x, y, w, h)| {
        let (x, y) = (f64::from(x) * 0.5, f64::from(y) * 0.5);
        (x, y, x + f64::from(w) * 0.5, y + f64::from(h) * 0.5)
    })
}

proptest! {
    /// Random lattice rectangles, one random island moved by a random lattice
    /// translation (often onto the others), undone, redone: bytes identical
    /// each way.
    #[test]
    fn island_move_undo_redo_is_byte_identical(
        rects in proptest::collection::vec(lattice_rect(), 1..5),
        pick in any::<usize>(),
        dx in -8..8i32,
        dy in -8..8i32,
    ) {
        let (mut doc, sid) = doc_with_rects(&rects);
        let ids: Vec<SketchIslandId> = doc.sketch(sid).unwrap().islands().keys().collect();
        prop_assume!(!ids.is_empty());
        let island = ids[pick % ids.len()];
        let t = translate(f64::from(dx) * 0.5, f64::from(dy) * 0.5);
        assert_op_replays_byte_exactly(&mut doc, |d| {
            let _ = d.transform_sketch_island(sid, island, &t);
        })?;
    }
}

// ------------------------------------------------------- metric contact

/// A plane with an exact in-plane basis, so tests can place points at exact
/// in-plane offsets on any orientation.
struct Frame {
    plane: Plane,
    o: Point3,
    e1: Vec3,
    e2: Vec3,
}

impl Frame {
    fn new(normal: Vec3) -> Frame {
        let n = normal.normalized().unwrap();
        let reference = if n.x.abs() < 0.9 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let e1 = n.cross(reference).normalized().unwrap();
        let e2 = n.cross(e1).normalized().unwrap();
        let o = Point3::new(0.0, 0.0, 0.0);
        Frame {
            plane: Plane::from_point_normal(o, n).unwrap(),
            o,
            e1,
            e2,
        }
    }

    fn p(&self, x: f64, y: f64) -> Point3 {
        Point3::new(
            self.o.x + x * self.e1.x + y * self.e2.x,
            self.o.y + x * self.e1.y + y * self.e2.y,
            self.o.z + x * self.e1.z + y * self.e2.z,
        )
    }
}

fn frames() -> Vec<Frame> {
    vec![
        Frame::new(Vec3::new(0.0, 0.0, 1.0)),
        Frame::new(Vec3::new(0.3, 0.4, 0.866_025_403_784_438_6)),
    ]
}

const LENGTHS: [f64; 3] = [1e-3, 1.0, 1e3];

#[test]
fn contact_is_metric_a_near_end_splits_regardless_of_edge_length() {
    for f in frames() {
        for &len in &LENGTHS {
            for (off, splits) in [(0.9e-9, true), (1.1e-9, false)] {
                let mut s = Sketch::on_plane(f.plane);
                s.add_segment(f.p(0.0, 0.0), f.p(len, 0.0)).unwrap();
                // A segment ending `off` above the long edge's interior.
                let added = s.add_segment(f.p(len / 2.0, len), f.p(len / 2.0, off));
                let added = added.unwrap_or_else(|e| panic!("len {len} off {off}: {e:?}"));
                assert_eq!(
                    !added.split_edges.is_empty(),
                    splits,
                    "len {len}, off {off:e}"
                );
                assert_eq!(
                    s.edges().len(),
                    if splits { 3 } else { 2 },
                    "len {len}, off {off:e}"
                );
            }
        }
    }
}

#[test]
fn a_near_end_proper_crossing_splits_both_for_every_length() {
    // Acute crossing (5 degrees) whose crossing point lies 2e-9 m from an end
    // of each segment, on opposite sides so the two ends are 4e-9 apart (no
    // vertex merge): each end is within POINT_MERGE of the other segment
    // (2e-9 * sin 5deg), so each touches the other and both split.
    let angle = 5.0_f64.to_radians();
    let d = (angle.cos(), angle.sin());
    let tip = 2e-9;
    for f in frames() {
        for &len in &LENGTHS {
            let x = (len - tip, 0.0);
            let mut s = Sketch::on_plane(f.plane);
            let first = s.add_segment(f.p(0.0, 0.0), f.p(len, 0.0)).unwrap();
            assert_eq!(first.new_edges.len(), 1);
            let second = s
                .add_segment(
                    f.p(x.0 - tip * d.0, -tip * d.1),
                    f.p(x.0 + len * d.0, len * d.1),
                )
                .unwrap();
            assert_eq!(
                second.split_edges.len(),
                1,
                "len {len}: first segment splits"
            );
            assert_eq!(
                second.split_edges[0].1.len(),
                2,
                "len {len}: into two fragments"
            );
            // Four vertices: s1's ends and s2's ends, plus nothing else — each
            // end of one lies ON the other, so the 4e-9 m middle fragment is
            // shared (it is s1's tail and s2's head), and three edges remain:
            // s1's head, the shared middle, and s2's tail.
            assert_eq!(s.vertices().len(), 4, "len {len}");
            assert_eq!(s.edges().len(), 3, "len {len}: both segments split");
        }
    }
}

/// Found by the world-sketch fuzzer (pre-existing: reproduces on the commit
/// before the metric-contact work). Offsetting inward the C-shaped region
/// that wraps most of a rectangle draws a C-shaped loop around it. The
/// region tracer assigned holes by the loop's vertex centroid, which for a
/// C lies inside the rectangle, so the rectangle's region took the loop as
/// a HOLE lying outside its own outer ring and `profile()` failed
/// `MalformedRegion`. Holes are now placed by a point strictly inside the
/// hole loop.
#[test]
fn offset_ring_enclosing_a_region_does_not_malform_it() {
    let mut s = xy_sketch();
    draw_rect_into(&mut s, 4.0, 2.5, 7.0, 4.0);
    let first = s.regions().keys().next().unwrap();
    s.offset_region(first, 0.25).unwrap();
    draw_rect_into(&mut s, 2.0, 1.0, 4.0, 3.0);
    // The region wrapping the inner rectangle: a notched ring, no holes.
    let wrap = s
        .regions()
        .iter()
        .find(|(_, r)| r.outer.len() == 10)
        .map(|(id, _)| id)
        .expect("the wrapping region exists");
    s.offset_region(wrap, -0.1).unwrap();
    for rid in s.regions().keys() {
        assert!(s.profile(rid).is_ok(), "region {rid:?} has no profile");
    }
}

// ------------------------------------------------------- loops with no face

/// A closed loop can be edges alone (rule 6): it keeps that while nothing
/// touches it, and it has no profile to extrude.
#[test]
fn a_loop_of_edges_alone_has_no_face_and_keeps_none_while_untouched() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 2.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    let rid = s.regions().keys().next().expect("a region");
    assert!(!s.regions()[rid].faceless, "a drawn loop has a face");
    s.make_regions_faceless();
    assert!(s.regions()[rid].faceless);
    assert_eq!(s.profile(rid).unwrap_err(), SketchError::RegionHasNoFace);
    // Drawing elsewhere leaves it as it is, id and state.
    s.add_segment(pt(5.0, 5.0), pt(6.0, 5.0)).unwrap();
    assert!(s.regions()[rid].faceless);
    s.set_region_faceless(rid, false).unwrap();
    assert!(s.profile(rid).is_ok());
}

/// Drawing over an edge of a faceless loop gives the loop its face, as in
/// SketchUp, without reshaping it: same region id, nothing else changes.
#[test]
fn drawing_over_an_edge_of_a_faceless_loop_gives_it_a_face() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 2.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    s.make_regions_faceless();
    let rid = s.regions().keys().next().unwrap();
    let before = counts(&s);
    let report = s.add_segment(pt(2.0, 0.0), pt(2.0, 1.0)).unwrap();
    assert_eq!(counts(&s), before, "nothing new is drawn");
    assert!(report.regions_created.is_empty() && report.regions_removed.is_empty());
    assert!(
        !s.regions()[rid].faceless,
        "the loop drawn over has its face"
    );
    assert!(s.profile(rid).is_ok());
}

/// A new edge across a faceless loop reshapes it: the halves are new
/// regions, and a region a drawing step brings about has a face.
#[test]
fn a_new_edge_across_a_faceless_loop_makes_faced_halves() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 2.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    s.make_regions_faceless();
    s.add_segment(pt(0.0, 0.0), pt(2.0, 1.0)).unwrap();
    assert_eq!(s.regions().len(), 2);
    assert!(s.regions().values().all(|r| !r.faceless));
}

/// A faceless loop copied or moved whole is the same loop elsewhere, not
/// one drawn: it stays faceless, while the drawing around it is untouched.
#[test]
fn a_faceless_loop_copied_or_moved_stays_faceless() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 2.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    s.make_regions_faceless();
    let edges: Vec<_> = s.edges().keys().collect();
    s.copy_edges_transformed(
        &edges,
        &kernel::Transform::translation(kernel::Vec3::new(5.0, 0.0, 0.0)),
    )
    .unwrap();
    assert_eq!(s.regions().len(), 2);
    assert!(
        s.regions().values().all(|r| r.faceless),
        "the copy is faceless too"
    );
    // Move the original: it is replayed at its new place, still faceless.
    s.apply_transform_edges(
        &edges,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 5.0, 0.0)),
    )
    .unwrap();
    assert_eq!(s.regions().len(), 2);
    assert!(s.regions().values().all(|r| r.faceless));
    // A loop drawn beside them has its face, as ever.
    for (a, b) in rect_segments(10.0, 10.0, 11.0, 11.0) {
        s.add_segment(a, b).unwrap();
    }
    assert_eq!(s.regions().values().filter(|r| !r.faceless).count(), 1);
}

/// A line that merely ends on a faceless loop's edge splits that edge
/// without closing anything: the loop keeps its state (rule 6), as does
/// a loop healed by erasing such a line.
#[test]
fn a_line_ending_on_a_faceless_loops_edge_leaves_it_faceless() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 2.0, 2.0) {
        s.add_segment(a, b).unwrap();
    }
    s.make_regions_faceless();
    let r = s.add_segment(pt(1.0, 0.0), pt(1.0, -1.0)).unwrap();
    assert_eq!(s.regions().len(), 1);
    assert!(
        s.regions().values().all(|r| r.faceless),
        "a T-junction closes no loop"
    );
    // Erase the stub: the healed loop is the same loop, still faceless.
    let stub = r.new_edges[0];
    s.remove_edge(stub).unwrap();
    assert_eq!(s.regions().len(), 1);
    assert!(s.regions().values().all(|r| r.faceless));
}

/// Erasing an edge never gives a face: two faceless loops merged by erasing
/// the edge between them are one faceless loop, and a faced loop next to a
/// faceless one loses its boundary, and its face, with that edge.
#[test]
fn erasing_an_edge_between_faceless_loops_keeps_the_merged_loop_faceless() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 1.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    for (a, b) in rect_segments(1.0, 0.0, 2.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    s.make_regions_faceless();
    let shared = s
        .edges()
        .iter()
        .find(|(_, e)| {
            let (p, q) = (s.vertices()[e.from].position, s.vertices()[e.to].position);
            (p.x - 1.0).abs() < 1e-9 && (q.x - 1.0).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .expect("the shared edge");
    s.remove_edge(shared).unwrap();
    assert_eq!(s.regions().len(), 1);
    assert!(s.regions().values().all(|r| r.faceless));

    // Faced beside faceless: the merge has no face either.
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 1.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    for (a, b) in rect_segments(1.0, 0.0, 2.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    let left = s
        .regions()
        .iter()
        .find(|(_, r)| {
            r.outer
                .iter()
                .all(|&v| s.vertices()[v].position.x <= 1.0 + 1e-9)
        })
        .map(|(id, _)| id)
        .expect("the left square");
    s.set_region_faceless(left, true).unwrap();
    assert_eq!(s.regions().values().filter(|r| !r.faceless).count(), 1);
    let shared = s
        .edges()
        .iter()
        .find(|(_, e)| {
            let (p, q) = (s.vertices()[e.from].position, s.vertices()[e.to].position);
            (p.x - 1.0).abs() < 1e-9 && (q.x - 1.0).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .unwrap();
    s.remove_edge(shared).unwrap();
    assert_eq!(s.regions().len(), 1);
    assert!(s.regions().values().all(|r| r.faceless));
}

/// Tracing an edge of a loop inside a larger faceless loop faces the inner
/// loop alone; the ring around it keeps no face until its own edge is
/// drawn over.
#[test]
fn tracing_an_inner_loops_edge_faces_that_loop_alone() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 4.0, 4.0) {
        s.add_segment(a, b).unwrap();
    }
    for (a, b) in rect_segments(1.0, 1.0, 2.0, 2.0) {
        s.add_segment(a, b).unwrap();
    }
    s.make_regions_faceless();
    s.add_segment(pt(1.0, 1.0), pt(2.0, 1.0)).unwrap();
    let faced: Vec<_> = s.regions().values().filter(|r| !r.faceless).collect();
    assert_eq!(faced.len(), 1);
    assert_eq!(faced[0].outer.len(), 4);
    assert!(faced[0].holes.is_empty(), "the inner loop, not the ring");
    s.add_segment(pt(0.0, 0.0), pt(4.0, 0.0)).unwrap();
    assert!(s.regions().values().all(|r| !r.faceless));
}

/// A wire moved exactly onto a faced loop does not take the face away.
#[test]
fn a_faceless_loop_moved_onto_a_faced_loop_leaves_the_face() {
    let mut s = xy_sketch();
    for (a, b) in rect_segments(0.0, 0.0, 1.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    for (a, b) in rect_segments(5.0, 0.0, 6.0, 1.0) {
        s.add_segment(a, b).unwrap();
    }
    let wire: Vec<_> = s
        .regions()
        .iter()
        .find(|(_, r)| s.vertices()[r.outer[0]].position.x >= 5.0 - 1e-9)
        .map(|(id, _)| id)
        .into_iter()
        .collect();
    s.set_region_faceless(wire[0], true).unwrap();
    let moved: Vec<_> = s
        .edges()
        .iter()
        .filter(|(_, e)| s.vertices()[e.from].position.x >= 5.0 - 1e-9)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(moved.len(), 4);
    s.apply_transform_edges(
        &moved,
        &kernel::Transform::translation(kernel::Vec3::new(-5.0, 0.0, 0.0)),
    )
    .unwrap();
    assert_eq!(s.regions().len(), 1);
    assert!(s.regions().values().all(|r| !r.faceless), "the face stays");
}

/// A coordinate that is not a number is off every plane: the segment is
/// refused with the point named, never drawn (a distance test alone would
/// let it through, since nothing compares greater than a tolerance when
/// one side is not a number).
#[test]
fn a_point_that_is_not_a_number_is_refused_as_off_the_plane() {
    let mut s = xy_sketch();
    assert_eq!(
        s.add_segment(pt(f64::NAN, 0.0), pt(1.0, 0.0)).unwrap_err(),
        SketchError::PointOffPlane { which: 0 }
    );
    assert_eq!(
        s.add_segment(pt(0.0, 0.0), pt(1.0, f64::INFINITY))
            .unwrap_err(),
        SketchError::PointOffPlane { which: 1 }
    );
    assert_eq!(counts(&s), (0, 0, 0), "nothing was drawn");
}

// ------------------------------------------------ many segments at once

/// Segments added at once build exactly what adding them one by one
/// builds: a tiling of squares (shared sides given twice), then lines
/// across it that cross, end on edges and run along them.
#[test]
fn segments_added_at_once_build_what_one_by_one_builds() {
    let mut segs: Vec<(Point3, Point3)> = Vec::new();
    for i in 0..6 {
        for j in 0..4 {
            let (x0, y0, x1, y1) = (i as f64, j as f64, i as f64 + 1.0, j as f64 + 1.0);
            segs.extend(rect_segments(x0, y0, x1, y1));
        }
    }
    segs.push((pt(-0.5, 0.5), pt(6.5, 3.5))); // crossing many
    segs.push((pt(2.5, -1.0), pt(2.5, 0.0))); // ending on an edge
    segs.push((pt(0.0, 2.0), pt(1.5, 2.0))); // along a side, past its end
    segs.push((pt(3.3, 1.3), pt(3.7, 1.7))); // inside a square, apart
    let mut one = xy_sketch();
    for &(a, b) in &segs {
        one.add_segment(a, b).unwrap();
    }
    let mut bulk = xy_sketch();
    bulk.add_segments_for_test(&segs).unwrap();
    assert_eq!(counts(&bulk), counts(&one));
    let edge_set = |s: &Sketch| -> Vec<[[i64; 2]; 2]> {
        let q = |p: Point3| [(p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64];
        let mut e: Vec<[[i64; 2]; 2]> = s
            .edges()
            .values()
            .map(|ed| {
                let mut pair = [
                    q(s.vertices()[ed.from].position),
                    q(s.vertices()[ed.to].position),
                ];
                pair.sort_unstable();
                pair
            })
            .collect();
        e.sort_unstable();
        e
    };
    assert_eq!(edge_set(&bulk), edge_set(&one));
    let rings = |s: &Sketch| -> Vec<Vec<[i64; 2]>> {
        let mut r: Vec<Vec<[i64; 2]>> = s
            .regions()
            .values()
            .map(|r| {
                let mut ring: Vec<[i64; 2]> = r
                    .outer
                    .iter()
                    .map(|&v| {
                        let p = s.vertices()[v].position;
                        [(p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64]
                    })
                    .collect();
                ring.sort_unstable();
                ring
            })
            .collect();
        r.sort();
        r
    };
    assert_eq!(rings(&bulk), rings(&one));
    assert_eq!(bulk.islands().len(), one.islands().len());
    // Into a sketch that already has drawing, the same holds.
    let mut one2 = xy_sketch();
    one2.add_segment(pt(0.5, 0.5), pt(5.5, 0.5)).unwrap();
    let mut bulk2 = one2.clone();
    for &(a, b) in &segs {
        one2.add_segment(a, b).unwrap();
    }
    bulk2.add_segments_for_test(&segs).unwrap();
    assert_eq!(edge_set(&bulk2), edge_set(&one2));
    assert_eq!(rings(&bulk2), rings(&one2));
}

/// Segments added at once each on its own curve build what adding them
/// one by one inside each curve's bracket builds: every edge on the same
/// curve, a circle crossing lines and another circle, an arc drawn over a
/// line already there taking that line into its curve, and plain lines
/// outside every curve.
#[test]
fn segments_added_at_once_on_curves_build_what_one_by_one_builds() {
    let polygon = |cx: f64, cy: f64, r: f64, n: usize, from: usize, to: usize| {
        (from..to)
            .map(|i| {
                let a = |k: usize| std::f64::consts::TAU * k as f64 / n as f64;
                (
                    pt(cx + r * a(i).cos(), cy + r * a(i).sin()),
                    pt(cx + r * a(i + 1).cos(), cy + r * a(i + 1).sin()),
                )
            })
            .collect::<Vec<_>>()
    };
    let plain = vec![
        (pt(-3.0, 0.0), pt(3.0, 0.0)),
        (pt(0.0, -3.0), pt(0.0, 3.0)),
        (pt(1.0, 0.0), pt(1.5, 0.5)),
    ];
    // An "arc" of two facets whose first facet retraces part of the x axis.
    let arc_over_line = vec![
        (pt(-2.0, 0.0), pt(-1.0, 0.0)),
        (pt(-1.0, 0.0), pt(-0.5, 0.8)),
    ];
    let curves = vec![
        polygon(0.0, 0.0, 2.0, 24, 0, 24),
        polygon(1.5, 1.0, 1.0, 16, 0, 16),
        polygon(-1.0, -1.0, 0.7, 12, 0, 5),
        arc_over_line,
    ];
    let mut one = xy_sketch();
    for &(a, b) in &plain {
        one.add_segment(a, b).unwrap();
    }
    for c in &curves {
        one.begin_curve();
        for &(a, b) in c {
            one.add_segment(a, b).unwrap();
        }
        one.end_curve();
    }
    let mut bulk = xy_sketch();
    let mut segs: Vec<(Point3, Point3, Option<kernel::SketchCurveId>)> =
        plain.iter().map(|&(a, b)| (a, b, None)).collect();
    for c in &curves {
        let id = bulk.begin_curve();
        bulk.end_curve();
        segs.extend(c.iter().map(|&(a, b)| (a, b, Some(id))));
    }
    bulk.add_segments_on_curves_for_test(&segs).unwrap();
    assert_eq!(counts(&bulk), counts(&one));
    let edge_set = |s: &Sketch| {
        let q = |p: Point3| [(p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64];
        let mut e: Vec<([[i64; 2]; 2], Option<kernel::SketchCurveId>)> = s
            .edges()
            .values()
            .map(|ed| {
                let mut pair = [
                    q(s.vertices()[ed.from].position),
                    q(s.vertices()[ed.to].position),
                ];
                pair.sort_unstable();
                (pair, ed.curve)
            })
            .collect();
        e.sort_unstable_by_key(|a| a.0);
        e
    };
    assert_eq!(
        edge_set(&bulk),
        edge_set(&one),
        "the same edges on the same curves"
    );
    let adopted = edge_set(&one)
        .iter()
        .filter(|(p, c)| *p == [[-2_000_000, 0], [-1_000_000, 0]] && c.is_some())
        .count();
    assert_eq!(adopted, 1, "the arc took the line it was drawn over");
    assert_eq!(bulk.regions().len(), one.regions().len());
    assert_eq!(bulk.islands().len(), one.islands().len());
}

/// A segment the sticky rules refuse stops the bulk add where it is, with
/// the refusal add_segment gives, the segments before it in place.
#[test]
fn a_segment_at_infinity_is_refused_before_the_bulk_add_starts() {
    let mut s = xy_sketch();
    let err = s
        .add_segments_for_test(&[
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(0.0, 0.0), pt(f64::INFINITY, 0.0)),
        ])
        .unwrap_err();
    assert_eq!(err, SketchError::PointOffPlane { which: 1 });
    assert_eq!(counts(&s), (0, 0, 0), "refused before anything was drawn");
    // Finite coordinates whose span overflows still finish.
    let far = f64::MAX / 2.0;
    let segs = [(pt(-far, 0.0), pt(far, 0.0)), (pt(0.0, 1.0), pt(0.0, 2.0))];
    let mut all = xy_sketch();
    let at_once = all.add_segments_for_test(&segs);
    let mut one = xy_sketch();
    let mut one_by_one = Ok(());
    for &(a, b) in &segs {
        if let Err(e) = one.add_segment(a, b) {
            one_by_one = Err(e);
            break;
        }
    }
    assert_eq!(at_once, one_by_one);
    assert_eq!(counts(&all), counts(&one));
}

/// A segment added at once that runs along an edge the same insertion
/// splits retraces the fragment instead of doubling it, as adding it on
/// its own does.
#[test]
fn a_bulk_segment_along_a_split_edge_retraces_the_fragment() {
    // Beyond one end: P-Q then M-R with M on PQ and R past Q.
    let mut one = xy_sketch();
    one.add_segment(pt(0.0, 0.0), pt(2.0, 0.0)).unwrap();
    one.add_segment(pt(1.0, 0.0), pt(3.0, 0.0)).unwrap();
    let mut all = xy_sketch();
    all.add_segments_for_test(&[(pt(0.0, 0.0), pt(2.0, 0.0)), (pt(1.0, 0.0), pt(3.0, 0.0))])
        .unwrap();
    assert_eq!(counts(&all), counts(&one));
    assert_eq!(counts(&all), (4, 3, 0));
    // Inside: P-Q then A-B strictly within it.
    let mut all = xy_sketch();
    all.add_segments_for_test(&[(pt(0.0, 0.0), pt(4.0, 0.0)), (pt(1.0, 0.0), pt(3.0, 0.0))])
        .unwrap();
    assert_eq!(counts(&all), (4, 3, 0));
}

#[test]
fn a_refused_segment_stops_the_bulk_add_with_its_refusal() {
    let mut s = xy_sketch();
    let err = s
        .add_segments_for_test(&[
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(0.0, 0.0), Point3::new(1.0, 1.0, 5.0)),
            (pt(2.0, 0.0), pt(3.0, 0.0)),
        ])
        .unwrap_err();
    assert_eq!(err, SketchError::PointOffPlane { which: 1 });
    assert_eq!(counts(&s), (2, 1, 0));
}
