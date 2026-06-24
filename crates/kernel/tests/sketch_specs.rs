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
