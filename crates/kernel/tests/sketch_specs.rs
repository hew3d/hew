//! Executable specs for the `Sketch` draft context and `Profile` validation
//! (DEVELOPMENT.md rule 3). Same rules as `op_specs.rs`: every test is
//! `#[ignore]`d until its operation is implemented; un-ignore in the same PR;
//! never weaken an assertion — escalate instead.
//!
//! All sketches here live on the XY plane (z = 0); `pt(x, y)` builds points
//! on it. The sticky-rule numbering in comments refers to the module docs of
//! `kernel::sketch`.

use kernel::{Plane, Point3, Profile, ProfileError, Sketch, SketchError, tol};
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
#[ignore = "spec for Sketch::add_segment: rule 1 — coincident endpoints merge"]
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
#[ignore = "spec for Sketch::add_segment: rule 2 — endpoint on an edge interior splits it (T-junction)"]
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
#[ignore = "spec for Sketch::add_segment: rule 3 — proper crossings split both edges"]
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
#[ignore = "spec for Sketch::add_segment: rule 4 — collinear overlap merges, never stacks"]
fn collinear_overlap_does_not_stack_edges() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(2.0, 0.0)).unwrap();
    s.add_segment(pt(1.0, 0.0), pt(3.0, 0.0)).unwrap();
    // Result is three abutting edges over [0,1], [1,2], [2,3]: no edge
    // covers another, no zero-length fragments.
    assert_eq!(counts(&s), (4, 3, 0));
}

#[test]
#[ignore = "spec for Sketch::add_segment: rule 5 — closing a circuit creates a region"]
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
#[ignore = "spec for Sketch::add_segment: a circuit inside a region becomes its hole"]
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
}

#[test]
#[ignore = "spec for Sketch::add_segment: a chord across a region splits it in two"]
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
#[ignore = "spec for Sketch::add_segment: off-plane points are typed errors, never projected"]
fn off_plane_point_is_rejected_unchanged() {
    let mut s = xy_sketch();
    let err = s
        .add_segment(pt(0.0, 0.0), Point3::new(1.0, 0.0, 1.0))
        .unwrap_err();
    assert_eq!(err, SketchError::PointOffPlane { which: 1 });
    assert_eq!(counts(&s), (0, 0, 0)); // strong guarantee
}

#[test]
#[ignore = "spec for Sketch::add_segment: degenerate segments are rejected, including after merging"]
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
#[ignore = "spec for Sketch::remove_edge: removing a boundary edge dissolves its region"]
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
#[ignore = "spec for Sketch::remove_edge: vertices used by nothing else are removed"]
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
#[ignore = "spec for Sketch::remove_edge: stale handles are typed errors"]
fn remove_edge_rejects_stale_handle() {
    let mut s = xy_sketch();
    s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap();
    let edge = s.edges().keys().next().unwrap();
    s.remove_edge(edge).unwrap();
    assert_eq!(s.remove_edge(edge).unwrap_err(), SketchError::UnknownEdge);
}

// -------------------------------------------------------- profile extraction

#[test]
#[ignore = "spec for Sketch::profile: a closed region exports as a valid Profile"]
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
#[ignore = "spec for Sketch::profile: region handles die when mutations reshape them"]
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

// --------------------------------------------------- insertion-order property

proptest! {
    #[test]
    #[ignore = "spec for Sketch::add_segment: region detection is insertion-order independent"]
    fn rectangle_closes_in_any_insertion_order(order in Just(vec![0usize, 1, 2, 3]).prop_shuffle()) {
        let segs = rect_segments(0.0, 0.0, 2.0, 1.0);
        let mut s = xy_sketch();
        for &i in order.iter() {
            s.add_segment(segs[i].0, segs[i].1).unwrap();
        }
        prop_assert_eq!(counts(&s), (4, 4, 1));
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
