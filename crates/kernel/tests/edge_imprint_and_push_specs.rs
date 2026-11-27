//! Executable specs for two push/pull-and-draw behaviors users hit on a
//! P-shaped slab (a stem plus a wider bowl):
//!
//! - Pushing the bowl's east face inward PAST the plane of the stem's east
//!   face (a co-facing wall) must carve the stem rather than refuse: the
//!   overshoot classifier now counts co-facing walls whose footprint
//!   touches the pushed face's, so the push routes to the subtract path.
//! - A rectangle drawn on the top face from one edge's midpoint to
//!   another's — two of its sides lying ON the boundary — must imprint as a
//!   boundary-to-boundary chord instead of being refused as "not strictly
//!   inside" ([`Object::plan_loop_imprint`]).

use kernel::{Document, KernelOp, LoopImprintPlan, Object, Plane, Point3, Vec3};

use crate::face_arrangement_support;
use face_arrangement_support::{draw_loop, face_plane, group_faces, region_face_in};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// The P slab in metres: stem x 0..0.0213 × y 0..0.1015, bowl x 0.0213..0.0369
/// × y 0.053..0.1015, 8.5 mm thick — the exact proportions of the reported
/// file.
const SX: f64 = 0.0213;
const BX: f64 = 0.0369;
const BY: f64 = 0.053;
const NY: f64 = 0.1015;
const H: f64 = 0.0085;

fn p_slab(doc: &mut Document) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let pts = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(SX, 0.0, 0.0),
        Point3::new(SX, BY, 0.0),
        Point3::new(BX, BY, 0.0),
        Point3::new(BX, NY, 0.0),
        Point3::new(0.0, NY, 0.0),
    ];
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..pts.len() {
            sk.add_segment(pts[i], pts[(i + 1) % pts.len()])
                .expect("segment");
        }
    }
    let regions = doc.extrudable_regions(s).expect("regions");
    assert_eq!(regions.len(), 1);
    doc.extrude_region(s, regions[0], H).expect("extrude").0
}

/// The face of `obj` with outward normal `n` whose plane passes through `p`.
fn face_at(obj: &Object, n: Vec3, p: Point3) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().approx_eq(n, 1e-9) && f.plane.signed_distance(p).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .expect("face exists")
}

fn bbox(obj: &Object) -> (Point3, Point3) {
    let mut lo = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for v in obj.vertices().values() {
        let p = v.position;
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

// ------------------------------------------------------------ push past

#[test]
fn pushing_past_a_co_facing_wall_routes_to_the_subtract_and_carves_the_stem() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let bowl_east = face_at(
        doc.object(id).unwrap(),
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(BX, 0.08, 0.004),
    );

    // Short of the stem face plane: the flat path stays in charge.
    assert!(
        !doc.object(id)
            .unwrap()
            .push_pull_overshoots(bowl_east, -0.010)
    );
    // Past it: the subtract path.
    assert!(
        doc.object(id)
            .unwrap()
            .push_pull_overshoots(bowl_east, -0.020)
    );

    let (results, _) = doc
        .push_pull_through(id, bowl_east, -0.020)
        .expect("push past the stem face carves rather than refuses");
    assert_eq!(results.len(), 1, "one solid comes back");
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.watertight(), kernel::WatertightState::Watertight);
    // The bowl now ends 20 mm west of where it was, notching into the stem.
    let east_now = BX - 0.020;
    assert!(
        out.faces().iter().any(|(_, f)| {
            f.plane.normal().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9)
                && f.plane
                    .signed_distance(Point3::new(east_now, 0.08, 0.004))
                    .abs()
                    < 1e-9
        }),
        "the pushed face sits at its new depth"
    );
    let (lo, hi) = bbox(out);
    assert!(
        (hi.x - SX).abs() < 1e-9,
        "the stem's east face is still the outermost x"
    );
    assert!((lo.x).abs() < 1e-9 && (hi.y - NY).abs() < 1e-9);
    // Nothing above y = BY reaches past the new bowl depth.
    for v in out.vertices().values() {
        if v.position.y > BY + 1e-9 {
            assert!(
                v.position.x <= east_now + 1e-9,
                "stem material past the notch is gone"
            );
        }
    }
    // Undo restores the original object verbatim.
    let before = doc.save();
    doc.undo().expect("undo");
    doc.redo().expect("redo");
    assert_eq!(doc.save(), before);
}

#[test]
fn a_push_that_stays_inside_its_own_bowl_keeps_the_flat_path() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let bowl_east = face_at(
        doc.object(id).unwrap(),
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(BX, 0.08, 0.004),
    );
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: bowl_east,
            distance: -0.010,
        },
    )
    .expect("flat push within the bowl");
    let (_, hi) = bbox(doc.object(id).unwrap());
    assert!((hi.x - (BX - 0.010)).abs() < 1e-9);
}

// ------------------------------------------------------------ pull past

/// The stem's east face of a fresh P slab (x = SX over y 0..BY).
fn stem_east(doc: &Document, id: kernel::ObjectId) -> kernel::FaceId {
    face_at(
        doc.object(id).unwrap(),
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(SX, 0.02, 0.004),
    )
}

#[test]
fn pulling_the_stem_past_the_bowls_end_routes_to_the_union_and_grows_the_stem() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let stem = stem_east(&doc, id);

    // Short of the bowl's east plane (15.6 mm away): the flat path.
    assert!(!doc.object(id).unwrap().push_pull_overshoots(stem, 0.010));
    // Past it: the union path.
    assert!(doc.object(id).unwrap().push_pull_overshoots(stem, 0.020));

    let (results, _) = doc
        .push_pull_through(id, stem, 0.020)
        .expect("a pull past the bowl's end grows the stem rather than refusing");
    assert_eq!(results.len(), 1, "one solid comes back");
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.watertight(), kernel::WatertightState::Watertight);
    let stem_now = SX + 0.020;
    let (lo, hi) = bbox(out);
    assert!(
        (hi.x - stem_now).abs() < 1e-9,
        "the stem is the outermost x now"
    );
    assert!(lo.x.abs() < 1e-9 && (hi.y - NY).abs() < 1e-9 && (hi.z - H).abs() < 1e-9);
    // The bowl kept its own end: nothing above y = BY reaches past it.
    for v in out.vertices().values() {
        if v.position.y > BY + 1e-9 {
            assert!(v.position.x <= BX + 1e-9, "the bowl did not grow");
        }
    }
    // The grown stem's east face is one face at the new depth, and the
    // bowl's east face is still there — a P whose stem outgrew its bowl.
    let east_faces: Vec<f64> = out
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9))
        .map(|(_, f)| f.plane.signed_distance(Point3::new(0.0, 0.0, 0.0)).abs())
        .collect();
    assert_eq!(east_faces.len(), 2, "east faces: {east_faces:?}");
    assert!(east_faces.iter().any(|&d| (d - stem_now).abs() < 1e-9));
    assert!(east_faces.iter().any(|&d| (d - BX).abs() < 1e-9));
    let before = doc.save();
    doc.undo().expect("undo");
    doc.redo().expect("redo");
    assert_eq!(doc.save(), before);
}

#[test]
fn pulling_a_notched_bowl_back_out_past_the_stem_restores_the_slab() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let original_faces = doc.object(id).unwrap().faces().len();
    let original_vertices = doc.object(id).unwrap().vertices().len();
    let bowl_east = face_at(
        doc.object(id).unwrap(),
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(BX, 0.08, 0.004),
    );
    // Notch the bowl 20 mm into the stem (the subtract path)...
    let (notched, _) = doc
        .push_pull_through(id, bowl_east, -0.020)
        .expect("push past the stem face");
    let notched = notched[0];
    let notch_x = BX - 0.020;
    let notched_east = face_at(
        doc.object(notched).unwrap(),
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(notch_x, 0.08, 0.004),
    );
    // ...then pull that face straight back out to where it started. The
    // stem's east plane is 4.4 mm ahead; 20 mm crosses it.
    assert!(
        !doc.object(notched)
            .unwrap()
            .push_pull_overshoots(notched_east, 0.004)
    );
    assert!(
        doc.object(notched)
            .unwrap()
            .push_pull_overshoots(notched_east, 0.020)
    );
    let (results, _) = doc
        .push_pull_through(notched, notched_east, 0.020)
        .expect("pulling the notched face back out grows the bowl rather than refusing");
    assert_eq!(results.len(), 1);
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.watertight(), kernel::WatertightState::Watertight);
    // The slab is back to its original shape: same extents, same face and
    // vertex counts, no leftover seam where the notch was.
    let (lo, hi) = bbox(out);
    assert!(lo.x.abs() < 1e-9 && lo.y.abs() < 1e-9 && lo.z.abs() < 1e-9);
    assert!((hi.x - BX).abs() < 1e-9 && (hi.y - NY).abs() < 1e-9 && (hi.z - H).abs() < 1e-9);
    assert_eq!(out.faces().len(), original_faces);
    assert_eq!(out.vertices().len(), original_vertices);
}

#[test]
fn pulling_the_stem_exactly_to_the_bowls_end_leaves_one_flush_east_face() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let stem = stem_east(&doc, id);
    let depth = BX - SX;
    assert!(doc.object(id).unwrap().push_pull_overshoots(stem, depth));
    let (results, _) = doc
        .push_pull_through(id, stem, depth)
        .expect("a pull flush with the bowl's end is a plain slab");
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.watertight(), kernel::WatertightState::Watertight);
    let east_faces = out
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9))
        .count();
    assert_eq!(east_faces, 1, "the stem and bowl ends merged into one face");
    assert_eq!(out.faces().len(), 6, "a plain box");
}

#[test]
fn a_pull_that_stays_short_of_the_bowls_end_keeps_the_flat_path() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let stem = stem_east(&doc, id);
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: stem,
            distance: 0.010,
        },
    )
    .expect("flat pull short of the bowl's end");
    let out = doc.object(id).unwrap();
    assert!(out.validate().is_ok());
    let (_, hi) = bbox(out);
    assert!(
        (hi.x - BX).abs() < 1e-9,
        "the bowl is still the outermost x"
    );
    assert!(
        out.vertices()
            .values()
            .any(|v| (v.position.x - (SX + 0.010)).abs() < 1e-9)
    );
}

/// A U-shaped slab: outer x 0..3 × y 0..2 with the slot x 1..2 × y 0.5..2
/// cut from the top, 0.2 tall. The arms' inner faces face each other across
/// the slot.
fn u_slab(doc: &mut Document) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let pts = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(3.0, 0.0, 0.0),
        Point3::new(3.0, 2.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(2.0, 0.5, 0.0),
        Point3::new(1.0, 0.5, 0.0),
        Point3::new(1.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..pts.len() {
            sk.add_segment(pts[i], pts[(i + 1) % pts.len()])
                .expect("segment");
        }
    }
    let regions = doc.extrudable_regions(s).expect("regions");
    assert_eq!(regions.len(), 1);
    doc.extrude_region(s, regions[0], 0.2).expect("extrude").0
}

#[test]
fn pulling_one_arm_of_a_u_across_the_slot_into_the_other_bridges_it() {
    let mut doc = Document::new();
    let id = u_slab(&mut doc);
    // The west arm's inner face: x = 1, normal +x, across a 1 m slot from
    // the east arm's inner face at x = 2 (normal -x).
    let inner = face_at(
        doc.object(id).unwrap(),
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.1),
    );
    assert!(!doc.object(id).unwrap().push_pull_overshoots(inner, 0.5));
    assert!(doc.object(id).unwrap().push_pull_overshoots(inner, 1.0));
    assert!(doc.object(id).unwrap().push_pull_overshoots(inner, 1.5));
    // Short of the far arm: the flat path narrows the slot.
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: inner,
            distance: 0.5,
        },
    )
    .expect("flat pull across half the slot");
    assert!(
        doc.object(id)
            .unwrap()
            .vertices()
            .values()
            .any(|v| (v.position.x - 1.5).abs() < 1e-9 && (v.position.y - 0.5).abs() < 1e-9)
    );
    doc.undo().expect("undo");
    // Into the far arm: the union fills the slot — a plain 3 × 2 box.
    let (results, _) = doc
        .push_pull_through(id, inner, 1.5)
        .expect("a pull into the facing arm bridges the slot rather than refusing");
    assert_eq!(results.len(), 1);
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.watertight(), kernel::WatertightState::Watertight);
    let (lo, hi) = bbox(out);
    assert!(lo.x.abs() < 1e-9 && lo.y.abs() < 1e-9);
    assert!((hi.x - 3.0).abs() < 1e-9 && (hi.y - 2.0).abs() < 1e-9 && (hi.z - 0.2).abs() < 1e-9);
    assert_eq!(
        out.faces().len(),
        6,
        "the slot is gone: {}",
        out.faces().len()
    );
    // Exactly flush with the far arm is the same box.
    doc.undo().expect("undo");
    let (results, _) = doc
        .push_pull_through(id, inner, 1.0)
        .expect("a pull flush with the facing arm closes the slot");
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.faces().len(), 6, "flush: {}", out.faces().len());
}

// -------------------------------------------------------- pinned corner

/// A P whose bowl is THINNER than its stem: stem x 0..1 × y 0..2, 0.85 tall;
/// bowl x 0..3 × y 2..4, 0.44 tall. The stem's north face above the bowl is
/// exposed, and its bottom-east corner is where the bowl's top, the stem's
/// east wall, and the bowl's south wall (the part east of the stem) meet.
fn thin_bowl_p(doc: &mut Document) -> kernel::ObjectId {
    let boxed = |doc: &mut Document, lo: Point3, hi: Point3| {
        let s = doc.add_sketch(ground());
        let pts = [
            Point3::new(lo.x, lo.y, 0.0),
            Point3::new(hi.x, lo.y, 0.0),
            Point3::new(hi.x, hi.y, 0.0),
            Point3::new(lo.x, hi.y, 0.0),
        ];
        {
            let sk = doc.sketch_mut(s).expect("sketch");
            for i in 0..pts.len() {
                sk.add_segment(pts[i], pts[(i + 1) % pts.len()])
                    .expect("segment");
            }
        }
        let regions = doc.extrudable_regions(s).expect("regions");
        doc.extrude_region(s, regions[0], hi.z).expect("extrude").0
    };
    let stem = boxed(doc, Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 2.0, 0.85));
    let bowl = boxed(doc, Point3::new(0.0, 2.0, 0.0), Point3::new(3.0, 4.0, 0.44));
    doc.boolean(kernel::BooleanOp::Union, stem, bowl)
        .expect("union")
        .0
}

#[test]
fn pulling_the_exposed_stem_face_over_a_thinner_bowl_grows_the_stem_any_distance() {
    let mut doc = Document::new();
    let id = thin_bowl_p(&mut doc);
    let exposed = face_at(
        doc.object(id).unwrap(),
        Vec3::new(0.0, 1.0, 0.0),
        Point3::new(0.5, 2.0, 0.6),
    );
    let faces_before = doc.object(id).unwrap().faces().len();
    // Its bottom-east corner is pinned by the bowl's south wall, so even a
    // short pull is realized as a boolean (the flat translate would bend
    // that wall off its plane) — and the bowl's own north face, whose ring
    // is free, is not.
    assert!(doc.object(id).unwrap().push_pull_overshoots(exposed, 0.5));
    assert!(doc.object(id).unwrap().push_pull_overshoots(exposed, -0.3));
    let bowl_north = face_at(
        doc.object(id).unwrap(),
        Vec3::new(0.0, 1.0, 0.0),
        Point3::new(1.5, 4.0, 0.2),
    );
    assert!(
        !doc.object(id)
            .unwrap()
            .push_pull_overshoots(bowl_north, 0.5)
    );

    // Short of the bowl's end, exactly flush with it, and past it: all
    // three grow the stem's upper block over (and beyond) the bowl.
    for (d, y_max) in [(0.5, 4.0), (2.0, 4.0), (2.5, 4.5)] {
        let (results, _) = doc
            .push_pull_through(id, exposed, d)
            .unwrap_or_else(|e| panic!("pull {d}: {e:?}"));
        let out = doc.object(results[0]).unwrap();
        assert!(out.validate().is_ok(), "pull {d}");
        assert_eq!(out.watertight(), kernel::WatertightState::Watertight);
        let (lo, hi) = bbox(out);
        assert!(lo.x.abs() < 1e-9 && lo.y.abs() < 1e-9 && lo.z.abs() < 1e-9);
        assert!((hi.y - y_max).abs() < 1e-9, "pull {d}: {hi:?}");
        assert!((hi.z - 0.85).abs() < 1e-9);
        // The upper block now reaches y = 2 + d at full height...
        assert!(out.vertices().values().any(|v| {
            (v.position.y - (2.0 + d)).abs() < 1e-9 && (v.position.z - 0.85).abs() < 1e-9
        }));
        // ...and the bowl's south-east corner stayed put.
        assert!(out.vertices().values().any(|v| {
            (v.position.x - 1.0).abs() < 1e-9
                && (v.position.y - 2.0).abs() < 1e-9
                && (v.position.z - 0.44).abs() < 1e-9
        }));
        doc.undo().expect("undo");
        assert_eq!(doc.object(id).unwrap().faces().len(), faces_before);
    }

    // Pushing it back into the stem notches the upper block instead.
    let (results, _) = doc
        .push_pull_through(id, exposed, -0.3)
        .expect("push into the stem");
    let out = doc.object(results[0]).unwrap();
    assert!(out.validate().is_ok());
    assert!(
        out.vertices()
            .values()
            .any(|v| { (v.position.y - 1.7).abs() < 1e-9 && (v.position.z - 0.85).abs() < 1e-9 })
    );
}

// --------------------------------------------------------- loop imprint

#[test]
fn a_rectangle_from_edge_midpoint_to_edge_midpoint_plans_as_one_chord() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let top = face_at(
        doc.object(id).unwrap(),
        Vec3::new(0.0, 0.0, 1.0),
        Point3::new(0.01, 0.01, H),
    );
    let west_mid = Point3::new(0.0, NY / 2.0, H);
    let north_mid = Point3::new(BX / 2.0, NY, H);
    // The rectangle spanned by those two points: two sides on the boundary.
    let rect = [
        west_mid,
        Point3::new(north_mid.x, west_mid.y, H),
        north_mid,
        Point3::new(0.0, NY, H),
    ];
    let plan = doc
        .object(id)
        .unwrap()
        .plan_loop_imprint(top, &rect)
        .expect("plan");
    match plan {
        LoopImprintPlan::Chords(chords) => {
            assert_eq!(chords.len(), 1);
            assert_eq!(chords[0].len(), 3, "north-mid → inner corner → west-mid");
            let ends = [chords[0][0], chords[0][2]];
            assert!(ends.contains(&north_mid) && ends.contains(&west_mid));
            assert_eq!(chords[0][1], Point3::new(north_mid.x, west_mid.y, H));
            // And it actually cuts: the top face becomes two faces, the
            // rectangle being one of them.
            let faces_before = doc.object(id).unwrap().faces().len();
            let (report, _) = doc
                .apply_object_op(
                    id,
                    KernelOp::SplitFace {
                        face: top,
                        path: chords[0].clone(),
                        restore: None,
                        curves: Vec::new(),
                    },
                )
                .expect("chord split");
            let kernel::KernelOpReport::FaceSplit(r) = report else {
                panic!("split report")
            };
            assert_eq!(doc.object(id).unwrap().faces().len(), faces_before + 1);
            let out = doc.object(id).unwrap();
            assert!(out.validate().is_ok());
            let inner = Point3::new(north_mid.x / 2.0, (west_mid.y + NY) / 2.0, H);
            assert!(r.new_faces.iter().any(|&nf| {
                let f = &out.faces()[nf];
                f.plane.signed_distance(inner).abs() < 1e-9
            }));
        }
        other => panic!("expected a chord plan, got {other:?}"),
    }
}

#[test]
fn loops_clear_of_the_boundary_stay_inner_and_a_spanning_rectangle_plans_two_chords() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let obj = doc.object(id).unwrap();
    let top = face_at(obj, Vec3::new(0.0, 0.0, 1.0), Point3::new(0.01, 0.01, H));
    let inner = [
        Point3::new(0.005, 0.01, H),
        Point3::new(0.015, 0.01, H),
        Point3::new(0.015, 0.03, H),
        Point3::new(0.005, 0.03, H),
    ];
    assert!(matches!(
        obj.plan_loop_imprint(top, &inner),
        Ok(LoopImprintPlan::Inner(_))
    ));
    // West edge to the stem's east edge, across the stem: two straight chords.
    let spanning = [
        Point3::new(0.0, 0.01, H),
        Point3::new(SX, 0.01, H),
        Point3::new(SX, 0.03, H),
        Point3::new(0.0, 0.03, H),
    ];
    match obj.plan_loop_imprint(top, &spanning).expect("plan") {
        LoopImprintPlan::Chords(chords) => {
            assert_eq!(chords.len(), 2);
            assert!(chords.iter().all(|c| c.len() == 2));
        }
        other => panic!("expected two chords, got {other:?}"),
    }
    // A loop that only TOUCHES the boundary at a corner has no shared side:
    // still the inner plan (and that path's own strictness applies).
    let touching = [
        Point3::new(0.0, 0.01, H),
        Point3::new(0.01, 0.01, H),
        Point3::new(0.01, 0.02, H),
        Point3::new(0.0, 0.02, H),
    ];
    // Its west side (x = 0 from y 0.01 to 0.02) lies ON the west edge: one
    // chord of three points, not a touching corner.
    match obj.plan_loop_imprint(top, &touching).expect("plan") {
        LoopImprintPlan::Chords(chords) => assert_eq!((chords.len(), chords[0].len()), (1, 4)),
        other => panic!("expected one chord, got {other:?}"),
    }
}

// ------------------------------------------------- orchestrated imprint

#[test]
fn drawing_routes_an_edge_hugging_rectangle_to_a_chord_and_the_region_pushes() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let top = face_at(
        doc.object(id).unwrap(),
        Vec3::new(0.0, 0.0, 1.0),
        Point3::new(0.01, 0.01, H),
    );
    let west_mid = Point3::new(0.0, NY / 2.0, H);
    let north_mid = Point3::new(BX / 2.0, NY, H);
    let rect = vec![
        west_mid,
        Point3::new(north_mid.x, west_mid.y, H),
        north_mid,
        Point3::new(0.0, NY, H),
    ];
    let depth = doc.undo_depth();
    let plane = face_plane(&doc, id, top);
    draw_loop(&mut doc, None, id, top, &rect, None).expect("edge-hugging rectangle draws");
    assert_eq!(doc.undo_depth(), depth + 1);
    let region = region_face_in(&doc, None, id, plane, &rect).expect("the drawn region");
    let obj = doc.object(id).unwrap();
    let centroid = Point3::new(north_mid.x / 2.0, (west_mid.y + NY) / 2.0, H);
    assert!(obj.face_contains_point(region, centroid));
    let report_region = region;
    // The drawing is topology: the top is now two faces, the cut running
    // along the rectangle's two free sides.
    assert_eq!(
        group_faces(obj, &plane).len(),
        2,
        "the top splits into the rectangle and the rest"
    );
    assert!(doc.object_owned_sketches(id).is_empty());
    // The region is a real face: pull it up 5 mm and the solid stays solid.
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: report_region,
            distance: 0.005,
        },
    )
    .expect("push/pull the drawn region");
    let obj = doc.object(id).unwrap();
    assert!(obj.validate().is_ok());
    assert!(obj.faces().iter().any(|(_, f)| {
        f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9)
            && f.plane
                .signed_distance(Point3::new(centroid.x, centroid.y, H + 0.005))
                .abs()
                < 1e-9
    }));
    doc.undo().expect("undo pull");
    doc.undo().expect("undo imprint");
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.object(id).unwrap().faces().len(), 8);
}

#[test]
fn a_concave_loop_hugging_one_edge_still_names_its_own_region() {
    // A C shape whose back runs along the west edge: its vertex average
    // sits in the C's notch, OUTSIDE the loop, so a centroid-based pick
    // would hand back the notch as the drawn region.
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let top = face_at(
        doc.object(id).unwrap(),
        Vec3::new(0.0, 0.0, 1.0),
        Point3::new(0.01, 0.01, H),
    );
    let c_loop = vec![
        Point3::new(0.0, 0.02, H),
        Point3::new(0.012, 0.02, H),
        Point3::new(0.012, 0.03, H),
        Point3::new(0.003, 0.03, H),
        Point3::new(0.003, 0.06, H),
        Point3::new(0.012, 0.06, H),
        Point3::new(0.012, 0.07, H),
        Point3::new(0.0, 0.07, H),
    ];
    let n = c_loop.len() as f64;
    let vertex_average = c_loop.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
        Point3::new(acc.x + p.x / n, acc.y + p.y / n, acc.z + p.z / n)
    });
    let in_notch = Point3::new(0.0075, 0.045, H);
    let in_back = Point3::new(0.0015, 0.045, H);
    let in_arm = Point3::new(0.0075, 0.025, H);
    assert!(
        (vertex_average.x - in_notch.x).abs() < 1e-3
            && (vertex_average.y - in_notch.y).abs() < 1e-9
    );

    let plane = face_plane(&doc, id, top);
    draw_loop(&mut doc, None, id, top, &c_loop, None).expect("edge-hugging C draws");
    let region = region_face_in(&doc, None, id, plane, &c_loop).expect("the drawn region");
    let obj = doc.object(id).unwrap();
    assert!(obj.validate().is_ok());
    assert_eq!(group_faces(obj, &plane).len(), 2, "the C and the rest");
    assert!(obj.face_contains_point(region, in_back));
    assert!(obj.face_contains_point(region, in_arm));
    assert!(!obj.face_contains_point(region, in_notch));
    let other = group_faces(obj, &plane)
        .into_iter()
        .find(|&f| f != region)
        .expect("the other face");
    assert!(obj.face_contains_point(other, in_notch));
    assert!(!obj.face_contains_point(other, in_back));
}

#[test]
fn drawing_bundles_two_chords_into_one_undo_entry() {
    let mut doc = Document::new();
    let id = p_slab(&mut doc);
    let top = face_at(
        doc.object(id).unwrap(),
        Vec3::new(0.0, 0.0, 1.0),
        Point3::new(0.01, 0.01, H),
    );
    let spanning = vec![
        Point3::new(0.0, 0.01, H),
        Point3::new(SX, 0.01, H),
        Point3::new(SX, 0.03, H),
        Point3::new(0.0, 0.03, H),
    ];
    let depth = doc.undo_depth();
    let plane = face_plane(&doc, id, top);
    draw_loop(&mut doc, None, id, top, &spanning, None).expect("spanning rectangle draws");
    assert_eq!(doc.undo_depth(), depth + 1, "two chords, one entry");
    let region = region_face_in(&doc, None, id, plane, &spanning).expect("the drawn region");
    let obj = doc.object(id).unwrap();
    assert_eq!(obj.faces().len(), 10);
    assert!(obj.face_contains_point(region, Point3::new(0.01, 0.02, H)));
    doc.undo().expect("undo");
    assert_eq!(doc.object(id).unwrap().faces().len(), 8);
}
