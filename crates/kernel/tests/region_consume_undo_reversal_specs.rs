//! Repro + mechanism doc for a non-byte-identical undo restore of a
//! region's sketch scaffolding — found during API conformance work on
//! `hew.solid.follow_me` (see
//! `crates/api/tests/commands_sketch_solid.rs`'s
//! `follow_me_sweeps_a_square_profile_around_a_circular_path`, whose NOTE
//! originally attributed this to `follow_me` specifically). FIXED by
//! [`kernel::sketch::Sketch::remove_edges`] freeing vertex/edge slots in
//! descending id order — see that function's doc comment for the fix and
//! its partial-consumption reasoning. Both tests below were `#[ignore]`d
//! while the defect stood; now un-ignored and green.
//!
//! **This was NOT a follow_me-specific bug.** `Document::extrude_region`
//! had the exact same defect, via the exact same shared code path —
//! `Document::commit_region_object_owned` calls `Sketch::remove_edges`
//! then, on undo, `Sketch::restore_edges`, and BOTH `extrude_region` and
//! `follow_me` funnel through that one function with byte-identical inputs
//! (same `profile`/`region_scaffolding` capture order before either op does
//! anything else). There was no step in `follow_me` that touched the
//! profile sketch differently from `extrude_region`.
//!
//! ## Mechanism (as it stood before the fix)
//!
//! When a region's entire boundary is consumed (Model D), every one of its
//! vertices loses its last incident edge and dies too. `Sketch::remove_edges`
//! (crates/kernel/src/sketch.rs) removed them via:
//! ```ignore
//! for vid in touched { // touched: BTreeSet<SketchVertexId>, ASCENDING order
//!     if !self.vertex_has_edges(vid) { self.vertices.remove(vid); }
//! }
//! ```
//! `slotmap::SlotMap::remove` pushes each freed slot onto an internal free
//! list whose next `insert()` reuses the MOST RECENTLY freed slot first
//! (LIFO) — removing slots 0,1,2,3 in that order means the next four
//! inserts reuse them as 3,2,1,0.
//!
//! `Document::commit_region_object_owned` (crates/kernel/src/document.rs)
//! captures the doomed edges as position rows via `scaffolding.iter()` — a
//! `BTreeSet<SketchEdgeId>`, ascending order, which for a freshly-drawn
//! region is the SAME order the original `add_segment` calls ran in (a
//! virgin sketch has no free list, so creation assigns slots 0,1,2,...
//! in call order). On undo, `Sketch::restore_edges` replays those rows
//! through `add_segment_inner` in that same ascending order — but this
//! time the slotmap wasn't virgin: it held exactly those N freed slots in
//! LIFO order, so the k-th vertex materialized during restore got slot
//! `N-1-k` instead of slot `k`. The result was a complete index reversal
//! (`new_i = old_(N-1-i)`) of every vertex id, and in lockstep every
//! edge's `from`/`to` and the region's `outer` winding — geometrically
//! identical, byte-different.
//!
//! ## Why this wasn't caught before
//!
//! No committed kernel spec asserted `extrude_region`+undo byte-identity
//! before this file (searched; none existed). The API test's comment
//! described an ad hoc, uncommitted manual check — this repro showed that
//! check did not hold up:
//! `exact_api_rectangle_extrude_and_follow_me_both_reverse_on_undo` below
//! runs the API test's own rectangle (same plane, same corners, same
//! diagonal-corner draw order) AND its own circular path through
//! `extrude_region`/`follow_me` directly.
//!
//! ## The fix, and why it's contained to `remove_edges`
//!
//! The reversal was a property of the SHARED consume/restore machinery
//! (`remove_edges`'s removal order vs. `SlotMap`'s LIFO reuse), used by
//! every Model D region-consuming op, not just these two — so the fix
//! lives in `remove_edges` itself: freeing vertex and edge slots in
//! DESCENDING id order instead of ascending. This changes only the
//! slotmap's internal free-list order, never which ids are live
//! immediately after a `remove_edges` call, so it cannot move any
//! FORWARD-path (non-undo) byte or hash: verified against the full kernel
//! suite (fuzz, golden, serialize) and the `api` crate's own
//! byte-identity conformance sweep, all unchanged. The only thing it
//! changes is which slots `restore_edges`'s later insertions land in —
//! exactly the undo path this file exercises. Partial consumption (a
//! sketch with surviving geometry alongside the consumed region) is
//! covered by property tests in
//! `region_consume_undo_reversal_property_specs.rs`, per DEVELOPMENT.md's
//! spec-first rule for cross-cutting restore-machinery changes.

use kernel::{CurveGeom, Document, FollowMePath, Plane, Point3, SketchId, SketchRegionId, Vec3};

fn only_region(doc: &Document, s: SketchId) -> SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// A ground-plane circle sketch built the same way
/// `hew.sketch.draw_circle` builds one (`crates/api/src/commands/sketch.rs`'s
/// `ring_points` + `begin_curve_with`/`add_segment` chain, default 48
/// facets) — the exact path geometry
/// `follow_me_sweeps_a_square_profile_around_a_circular_path` sweeps
/// around. Returns the sketch and its full facet ring as a
/// `FollowMePath::SketchEdges` path.
fn draw_circle_path(doc: &mut Document, center: Point3, radius: f64) -> FollowMePath {
    let plane = Plane::from_point_normal(center, Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-defined");
    let s = doc.add_sketch(plane);
    let n = 48usize;
    let ring: Vec<Point3> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            center + Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
        })
        .collect();
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let curve = sk
        .begin_curve_with(CurveGeom { center, radius })
        .expect("curve opens");
    for i in 0..n {
        sk.add_segment(ring[i], ring[(i + 1) % n])
            .expect("circle facet");
    }
    sk.end_curve();
    let edges = sk.curve_edges(curve);
    FollowMePath::SketchEdges { sketch: s, edges }
}

/// Replicates `hew.sketch.draw_rect`'s own corner traversal
/// (crates/api/src/commands/sketch.rs's `draw_rect`/`plane_basis`) so this
/// is provably the same rectangle the API conformance test draws, not a
/// look-alike.
fn build_api_style_rect(doc: &mut Document) -> (SketchId, SketchRegionId) {
    fn plane_basis(plane: &Plane) -> (Vec3, Vec3) {
        let n = plane.normal();
        let reference = if n.x.abs() > 0.9 {
            Vec3::new(0.0, 1.0, 0.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let u = (reference - n * reference.dot(n)).normalized().unwrap();
        let v = n.cross(u);
        (u, v)
    }

    let plane = Plane::from_point_normal(Point3::new(0.1, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0))
        .expect("plane is well-defined");
    let s = doc.add_sketch(plane);
    let (u, v) = plane_basis(&plane);
    let origin = plane.point();
    let a = Point3::new(0.08, 0.0, -0.02);
    let b = Point3::new(0.12, 0.0, 0.02);
    let ua = (a - origin).dot(u);
    let va = (a - origin).dot(v);
    let ub = (b - origin).dot(u);
    let vb = (b - origin).dot(v);
    let p2 = origin + u * ub + v * va;
    let p4 = origin + u * ua + v * vb;
    let loop_pts = [a, p2, b, p4, a];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for w in loop_pts.windows(2) {
        sk.add_segment(w[0], w[1]).expect("segment");
    }
    let region = only_region(doc, s);
    (s, region)
}

/// `extrude_region`'s undo is byte-identical for this ordinary
/// axis-perpendicular square — see the module doc for the mechanism this
/// pins down (it was NOT byte-identical before `remove_edges`'s
/// descending-free fix).
#[test]
fn extrude_region_undo_is_not_byte_identical() {
    fn profile_plane_x(x: f64) -> Plane {
        Plane::from_polygon(&[
            Point3::new(x, 0.0, 0.0),
            Point3::new(x, 1.0, 0.0),
            Point3::new(x, 0.0, 1.0),
        ])
        .expect("vertical plane is well-defined")
    }
    fn draw_profile_rect(
        doc: &mut Document,
        s: SketchId,
        x: f64,
        y0: f64,
        z0: f64,
        y1: f64,
        z1: f64,
    ) {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let corners = [
            (Point3::new(x, y0, z0), Point3::new(x, y1, z0)),
            (Point3::new(x, y1, z0), Point3::new(x, y1, z1)),
            (Point3::new(x, y1, z1), Point3::new(x, y0, z1)),
            (Point3::new(x, y0, z1), Point3::new(x, y0, z0)),
        ];
        for (a, b) in corners {
            sk.add_segment(a, b).expect("profile segment");
        }
    }

    let mut doc = Document::new();
    let ps = doc.add_sketch(profile_plane_x(0.5));
    draw_profile_rect(&mut doc, ps, 0.5, -0.3, 0.9, -0.05, 1.15);
    let region = only_region(&doc, ps);

    let before = doc.save();
    doc.extrude_region(ps, region, 1.0).expect("extrude");
    doc.undo().expect("undo extrude");
    let after = doc.save();

    assert_eq!(
        before, after,
        "extrude_region undo must restore the profile sketch byte-identically"
    );
}

/// The exact geometry from the failing API conformance test
/// (`follow_me_sweeps_a_square_profile_around_a_circular_path`'s profile
/// rectangle AND its circular path), run through BOTH `extrude_region` and
/// `follow_me` from two independent documents, to demonstrate the defect is
/// shared rather than follow_me-specific.
#[test]
fn exact_api_rectangle_extrude_and_follow_me_both_reverse_on_undo() {
    // extrude_region branch.
    let mut doc_a = Document::new();
    let (s_a, r_a) = build_api_style_rect(&mut doc_a);
    let before_a = doc_a.save();
    doc_a.extrude_region(s_a, r_a, 1.0).expect("extrude");
    doc_a.undo().expect("undo extrude");
    let after_a = doc_a.save();
    assert_eq!(
        before_a, after_a,
        "extrude_region undo must be byte-identical (see module doc: reversed ids, pre-fix)"
    );

    // follow_me branch: the SAME circular path
    // `follow_me_sweeps_a_square_profile_around_a_circular_path` sweeps the
    // profile around (radius 0.1, centered at the origin on the ground
    // plane) — the profile's own plane is perpendicular to the path's
    // tangent at the path's start point (0.1, 0, 0), exactly as Follow Me's
    // attachment requires, since that API test already proves this
    // configuration sweeps successfully.
    let mut doc_b = Document::new();
    let path = draw_circle_path(&mut doc_b, Point3::new(0.0, 0.0, 0.0), 0.1);
    let (s_b, r_b) = build_api_style_rect(&mut doc_b);
    let before_b = doc_b.save();
    doc_b.follow_me(s_b, r_b, &path).expect("follow me");
    doc_b.undo().expect("undo follow me");
    let after_b = doc_b.save();
    assert_eq!(
        before_b, after_b,
        "follow_me undo must be byte-identical (see module doc: reversed ids, pre-fix, same mechanism as extrude_region)"
    );
}

/// The region of `s` whose material contains `p` — for sketches holding
/// more than one region, where `only_region` cannot apply.
fn region_at(doc: &Document, s: SketchId, p: Point3) -> SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    let sk = doc.sketch(s).expect("sketch is live");
    regions
        .into_iter()
        .find(|&r| sk.region_contains_point(r, p).expect("region is live"))
        .expect("a region contains the probe point")
}

/// SECOND-GENERATION defect, found by adversarial review of the
/// descending-free fix itself: the original correspondence argument
/// assumed ascending slot-id order == creation order, which any
/// free-then-reuse breaks. Consume a rect (freeing slots), then draw a
/// square's diagonal — the new edge REUSES a freed low slot while its
/// endpoints sit in high slots. Extruding the second triangle then
/// consumes that reused-slot edge alongside older higher-slot edges, and
/// the rows-only restore replayed the reused-slot edge FIRST, welding its
/// endpoints into the wrong (smallest-freed) vertex slots: a 3-cycle slot
/// permutation, byte-different on save. Fixed by recording the doomed
/// vertices in their own ascending-slot order
/// (`RemovedScaffolding::vertices`) and pre-creating them before the row
/// replay — see `Sketch::restore_edges`.
#[test]
fn undo_is_byte_identical_when_a_consumed_edge_sits_in_a_reused_slot() {
    let ground = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-defined");
    let mut doc = Document::new();
    let s = doc.add_sketch(ground);

    // Rect R1 at (0,0)-(1,1) and square S at (2,0)-(3,1), disjoint.
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let rect = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.0, 0.0, 0.0),
    ];
    for w in rect.windows(2) {
        sk.add_segment(w[0], w[1]).expect("rect segment");
    }
    let square = [
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(3.0, 0.0, 0.0),
        Point3::new(3.0, 1.0, 0.0),
        Point3::new(2.0, 1.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
    ];
    for w in square.windows(2) {
        sk.add_segment(w[0], w[1]).expect("square segment");
    }

    // Consume R1: its 4 edge slots and 4 vertex slots hit the free list.
    let r1 = region_at(&doc, s, Point3::new(0.5, 0.5, 0.0));
    doc.extrude_region(s, r1, 0.5).expect("extrude rect");

    // The diagonal reuses a freed low edge slot while referencing the
    // square's high-slot vertices — ascending edge id is no longer
    // creation order.
    doc.sketch_mut(s)
        .expect("sketch is live")
        .add_segment(Point3::new(3.0, 0.0, 0.0), Point3::new(2.0, 1.0, 0.0))
        .expect("diagonal");

    // Consume triangle A ((3,0),(3,1),(2,1)); the shared diagonal
    // survives with triangle B.
    let tri_a = region_at(&doc, s, Point3::new(2.9, 0.8, 0.0));
    doc.extrude_region(s, tri_a, 0.5)
        .expect("extrude triangle A");

    // Consume triangle B ((2,0),(3,0),(2,1)) — the reused-slot diagonal
    // dies here together with two older, higher-slot square edges — and
    // undo it.
    let before = doc.save();
    let tri_b = region_at(&doc, s, Point3::new(2.2, 0.2, 0.0));
    doc.extrude_region(s, tri_b, 0.5)
        .expect("extrude triangle B");
    doc.undo().expect("undo extrude");
    let after = doc.save();
    assert_eq!(
        before, after,
        "undo must be byte-identical even when a consumed edge occupies a reused slot"
    );
}

/// The eraser variant of the reused-slot defect: `Sketch::remove_edge`
/// frees an edge slot whose endpoints BOTH survive (each keeps another
/// incident edge), so a later-drawn edge reuses that low slot while its
/// new endpoint takes a fresh high vertex slot. Consuming the resulting
/// region then dooms vertices whose ascending-slot order differs from the
/// rows' first-encounter order — the same permutation as the
/// extrude-then-draw case, reached without any extrusion beforehand.
#[test]
fn undo_is_byte_identical_after_an_eraser_deletion_reused_a_slot() {
    let ground = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-defined");
    let mut doc = Document::new();
    let s = doc.add_sketch(ground);

    let v0 = Point3::new(0.0, 0.0, 0.0);
    let v1 = Point3::new(1.0, 0.0, 0.0);
    let v2 = Point3::new(0.5, 1.0, 0.0);
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in [(v0, v1), (v1, v2), (v2, v0)] {
        sk.add_segment(a, b).expect("triangle segment");
    }

    // Erase v0-v1: edge slot 0 freed; v0 and v1 both survive.
    let e0 = sk
        .edges()
        .iter()
        .find(|(_, e)| {
            let f = sk.vertices()[e.from].position;
            let t = sk.vertices()[e.to].position;
            (f == v0 && t == v1) || (f == v1 && t == v0)
        })
        .map(|(id, _)| id)
        .expect("triangle base edge exists");
    sk.remove_edge(e0).expect("erase base edge");

    // Re-close through a new low point: v0->P reuses edge slot 0, P takes
    // a fresh high vertex slot.
    let p = Point3::new(0.5, -0.8, 0.0);
    let sk = doc.sketch_mut(s).expect("sketch is live");
    sk.add_segment(v0, p).expect("v0-P");
    sk.add_segment(p, v1).expect("P-v1");

    // Consume the quad v0-P-v1-v2 and undo — all four vertices die, and
    // the reused-slot edge's row sorts first.
    let before = doc.save();
    let quad = region_at(&doc, s, Point3::new(0.5, 0.1, 0.0));
    doc.extrude_region(s, quad, 0.5).expect("extrude quad");
    doc.undo().expect("undo extrude");
    let after = doc.save();
    assert_eq!(
        before, after,
        "undo must be byte-identical after an eraser deletion caused slot reuse"
    );
}

/// A doomed vertex sitting in the sub-tolerance NEAR-ENDPOINT BAND of a
/// long surviving edge (metrically within POINT_MERGE of the segment,
/// parametrically inside the no-split band, farther than POINT_MERGE
/// from either endpoint — only reachable on edges longer than a meter)
/// defeats restore's pre-creation guard, so a faithful, slot-exact
/// restore is impossible. The restore must REFUSE typed
/// (`RestoreConflicts`, document untouched) rather than silently commit
/// a slot-permuted sketch — found by adversarial review of the
/// recorded-vertex-order fix.
#[test]
fn undo_refuses_typed_when_a_doomed_vertex_hides_in_a_survivors_endpoint_band() {
    let ground = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-defined");
    let mut doc = Document::new();
    let s = doc.add_sketch(ground);

    // A long bare survivor edge E, then a triangle whose first vertex p
    // sits 5e-9 m along E's line, 1e-10 m off it: no weld (distance to
    // E's endpoint is 5e-9 > POINT_MERGE), no split (param 5e-10 of a
    // 10 m edge is 5e-11 <= POINT_MERGE as a parameter), but metrically
    // within POINT_MERGE of the closed segment — the guard's blind spot.
    let sk = doc.sketch_mut(s).expect("sketch is live");
    sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(10.0, 0.0, 0.0))
        .expect("survivor edge");
    let p = Point3::new(5e-9, 1e-10, 0.0);
    let q = Point3::new(2.0, 1.0, 0.0);
    let r = Point3::new(1.0, 2.0, 0.0);
    for (a, b) in [(p, q), (q, r), (r, p)] {
        sk.add_segment(a, b).expect("triangle segment");
    }

    let region = only_region(&doc, s);
    doc.extrude_region(s, region, 0.5)
        .expect("extrude triangle");
    let extruded = doc.save();

    match doc.undo() {
        Err(_) => {}
        Ok(_) => {
            // If the kernel someday welds or splits this band at draw
            // time, the configuration stops existing and this pin should
            // be revisited — but a SILENT byte-different restore is the
            // one outcome that must never happen.
            assert_eq!(
                doc.save(),
                extruded,
                "an undo that succeeded here must have been faithful — it was not"
            );
            panic!("expected RestoreConflicts for the endpoint-band configuration");
        }
    }
    assert_eq!(
        doc.save(),
        extruded,
        "a refused undo must leave the document untouched (strong guarantee)"
    );
}
