//! Executable specs: undoing a face merge restores each dissolved face's own
//! attribute state (material, UV frame, analytic surface) from the merge
//! report's snapshot — never a fresh copy re-derived from the surviving
//! face. Without the snapshot, undo resurrected analytic surface claims a
//! face had legitimately lost under the map-or-drop contract
//! (the true-curves design) and lost paint the dissolved face carried.
//! Both merge/split pairs are covered: the inner (imprint) pair and the
//! outer (boundary-split) pair.

use kernel::{
    CurveGeom, Document, FaceId, History, KernelOp, KernelOpReport, Object, Plane, Point3, Sketch,
    Vec3,
};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

/// An extruded analytic cylinder: every wall facet carries its SurfaceRef.
fn analytic_cylinder(radius: f64, n: usize, height: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius,
    })
    .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let region = s.regions().keys().next().unwrap();
    Object::from_extrusion(&s.profile(region).unwrap(), height).unwrap()
}

/// A small quad strictly inside `wall`, on its plane, around its centroid.
fn inner_quad(obj: &Object, wall: FaceId, half: f64) -> Vec<Point3> {
    let plane = obj.faces()[wall].plane;
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[wall].outer_loop).collect();
    let inv = 1.0 / ring.len() as f64;
    let c = ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
        Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
    });
    let u = (ring[1] - ring[0]).normalized().unwrap();
    let v = plane.normal().cross(u);
    vec![
        c + u * -half + v * -half,
        c + u * half + v * -half,
        c + u * half + v * half,
        c + u * -half + v * half,
    ]
}

/// The F6 repro, inner pair, analytic surface: a sub-face that legitimately
/// LOST its inherited SurfaceRef (extrude drops it — the face left its chord
/// plane; collapse does not restore it) must come back from an undone
/// `merge_inner_face` still without it. Re-deriving from the current parent
/// would resurrect the retired claim, invisibly to the validator (the
/// collapsed sub-face is coincidentally back on the chord plane).
#[test]
fn undo_of_merge_inner_face_does_not_resurrect_a_dropped_surface() {
    let mut obj = analytic_cylinder(1.0, 8, 2.0);
    let mut history = History::new();

    let wall = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_some())
        .map(|(id, _)| id)
        .unwrap();
    let wall_surface = obj.faces()[wall].surface;
    let loop_path = inner_quad(&obj, wall, 0.05);

    // Imprint: the sub-face inherits the wall's claim (same chord plane).
    let report = history
        .apply(
            &mut obj,
            KernelOp::SplitFaceInner {
                face: wall,
                loop_path,
                restore: None,
                curve: None,
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("wrong report kind")
    };
    let sub = r.sub_face;
    assert_eq!(obj.faces()[sub].surface, wall_surface, "imprint inherits");

    // Raise: the sub-face leaves its chord plane — the claim drops.
    history
        .apply(
            &mut obj,
            KernelOp::ExtrudeSubFace {
                sub_face: sub,
                distance: 0.02,
            },
        )
        .unwrap();
    assert_eq!(obj.faces()[sub].surface, None, "extrude drops the claim");

    // Flatten: back on the chord plane, but the drop is one-way.
    history
        .apply(&mut obj, KernelOp::CollapseSubFace { sub_face: sub })
        .unwrap();
    assert_eq!(obj.faces()[sub].surface, None, "collapse does not restore");

    // Dissolve, then undo: the re-created sub-face must carry what the
    // dissolved one carried (surface: None) — not the parent's live claim.
    history
        .apply(&mut obj, KernelOp::MergeInnerFace { sub_face: sub })
        .unwrap();
    history.undo(&mut obj).unwrap();

    // Re-query: the parent is the (only) face with an inner loop.
    let parent = obj
        .faces()
        .iter()
        .find(|(_, f)| !f.inner_loops.is_empty())
        .map(|(id, _)| id)
        .unwrap();
    let sub_face = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.inner_loops.is_empty()
                && f.plane.normal().approx_eq(
                    obj.faces()[parent].plane.normal(),
                    kernel::tol::NORMAL_DIRECTION,
                )
                && obj.loop_positions(f.outer_loop).count() == 4
                && f.outer_loop != obj.faces()[parent].outer_loop
                && {
                    // The imprinted quad is tiny; the wall is not.
                    let pts: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
                    (pts[0] - pts[1]).length() < 0.2
                }
        })
        .map(|(id, _)| id)
        .unwrap();
    assert_eq!(
        obj.faces()[sub_face].surface,
        None,
        "undo must not resurrect the dropped analytic claim"
    );
    assert_eq!(
        obj.faces()[parent].surface,
        wall_surface,
        "the parent keeps its own claim"
    );
}

/// Inner pair, material: paint a sub-face its own color, dissolve it, undo —
/// the paint comes back. Before the snapshot fix the re-created sub-face
/// silently inherited the parent's material.
#[test]
fn undo_of_merge_inner_face_restores_the_sub_faces_own_paint() {
    let mut doc = Document::new();
    let red = doc.add_material(kernel::Material {
        name: "Red".to_string(),
        color: kernel::Rgba8::rgb(220, 40, 40),
        texture: None,
    });

    // A plain box to imprint on.
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            ((0.0, 0.0), (2.0, 0.0)),
            ((2.0, 0.0), (2.0, 2.0)),
            ((2.0, 2.0), (0.0, 2.0)),
            ((0.0, 2.0), (0.0, 0.0)),
        ] {
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    let region = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (oid, _) = doc.extrude_region(s, region, 1.0).unwrap();

    // Imprint a quad on the top face.
    let top = doc
        .object(oid)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), kernel::tol::NORMAL_DIRECTION)
        })
        .map(|(id, _)| id)
        .unwrap();
    let loop_path = vec![
        Point3::new(0.5, 0.5, 1.0),
        Point3::new(1.5, 0.5, 1.0),
        Point3::new(1.5, 1.5, 1.0),
        Point3::new(0.5, 1.5, 1.0),
    ];
    let (report, _) = doc
        .apply_object_op(
            oid,
            KernelOp::SplitFaceInner {
                face: top,
                loop_path,
                restore: None,
                curve: None,
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("wrong report kind")
    };
    let sub = r.sub_face;

    // Paint the sub-face its own color, then dissolve it.
    doc.paint_face(oid, sub, Some(red)).unwrap();
    doc.apply_object_op(oid, KernelOp::MergeInnerFace { sub_face: sub })
        .unwrap();

    // Undo the dissolve: the sub-face returns WITH its paint.
    doc.undo().unwrap();
    let obj = doc.object(oid).unwrap();
    let painted = obj
        .faces()
        .values()
        .filter(|f| f.material == Some(red))
        .count();
    assert_eq!(painted, 1, "the dissolved sub-face's own paint is restored");
}

/// Outer pair, material: split a face, paint the two halves differently,
/// merge them, undo — each half gets ITS OWN material back, matched by
/// geometry. Before the snapshot fix both halves inherited the survivor's.
#[test]
fn undo_of_merge_faces_restores_each_sides_own_paint() {
    let mut doc = Document::new();
    let red = doc.add_material(kernel::Material {
        name: "Red".to_string(),
        color: kernel::Rgba8::rgb(220, 40, 40),
        texture: None,
    });
    let blue = doc.add_material(kernel::Material {
        name: "Blue".to_string(),
        color: kernel::Rgba8::rgb(40, 40, 220),
        texture: None,
    });

    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            ((0.0, 0.0), (2.0, 0.0)),
            ((2.0, 0.0), (2.0, 2.0)),
            ((2.0, 2.0), (0.0, 2.0)),
            ((0.0, 2.0), (0.0, 0.0)),
        ] {
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    let region = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (oid, _) = doc.extrude_region(s, region, 1.0).unwrap();

    let top = doc
        .object(oid)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), kernel::tol::NORMAL_DIRECTION)
        })
        .map(|(id, _)| id)
        .unwrap();

    // Split the top in half at x = 1.
    let (report, _) = doc
        .apply_object_op(
            oid,
            KernelOp::SplitFace {
                face: top,
                path: vec![Point3::new(1.0, 0.0, 1.0), Point3::new(1.0, 2.0, 1.0)],
                restore: None,
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplit(r) = report else {
        panic!("wrong report kind")
    };

    // Identify the halves geometrically and paint them differently.
    let half_at = |doc: &Document, x: f64| -> FaceId {
        doc.object(oid)
            .unwrap()
            .faces()
            .iter()
            .find(|(fid, f)| {
                r.new_faces.contains(fid) && {
                    let pts: Vec<Point3> = doc
                        .object(oid)
                        .unwrap()
                        .loop_positions(f.outer_loop)
                        .collect();
                    let cx = pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64;
                    (cx - x).abs() < 0.25
                }
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let left = half_at(&doc, 0.5);
    let right = half_at(&doc, 1.5);
    doc.paint_face(oid, left, Some(red)).unwrap();
    doc.paint_face(oid, right, Some(blue)).unwrap();

    // Merge them back (the raw primitive has no material gate), then undo.
    let cut_edge = r.new_edges[0];
    doc.apply_object_op(oid, KernelOp::MergeFaces { edge: cut_edge })
        .unwrap();
    doc.undo().unwrap();

    // Each half must carry ITS OWN paint, on the correct side.
    let obj = doc.object(oid).unwrap();
    let material_at = |x: f64| -> Option<kernel::MaterialId> {
        obj.faces()
            .values()
            .find(|f| {
                f.plane
                    .normal()
                    .approx_eq(Vec3::new(0.0, 0.0, 1.0), kernel::tol::NORMAL_DIRECTION)
                    && {
                        let pts: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
                        let cx = pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64;
                        (cx - x).abs() < 0.25
                    }
            })
            .and_then(|f| f.material)
    };
    assert_eq!(material_at(0.5), Some(red), "left half keeps its red");
    assert_eq!(material_at(1.5), Some(blue), "right half keeps its blue");
}

/// Outer pair, analytic surface: splitting an attributed wall facet and
/// merging it back round-trips the SurfaceRef through undo on both
/// fragments.
#[test]
fn undo_of_merge_faces_round_trips_wall_attribution() {
    let mut obj = analytic_cylinder(1.0, 8, 2.0);
    let mut history = History::new();

    let wall = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_some())
        .map(|(id, _)| id)
        .unwrap();
    let wall_surface = obj.faces()[wall].surface;

    // Split the wall horizontally at mid-height: both fragments inherit.
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[wall].outer_loop).collect();
    // Wall quads are [a_near, b_near, b_far, a_far]; midpoints of the two
    // vertical edges give a boundary-to-boundary cut.
    let m0 = Point3::new(
        (ring[0].x + ring[3].x) / 2.0,
        (ring[0].y + ring[3].y) / 2.0,
        (ring[0].z + ring[3].z) / 2.0,
    );
    let m1 = Point3::new(
        (ring[1].x + ring[2].x) / 2.0,
        (ring[1].y + ring[2].y) / 2.0,
        (ring[1].z + ring[2].z) / 2.0,
    );
    let report = history
        .apply(
            &mut obj,
            KernelOp::SplitFace {
                face: wall,
                path: vec![m0, m1],
                restore: None,
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplit(r) = report else {
        panic!("wrong report kind")
    };
    for f in r.new_faces {
        assert_eq!(obj.faces()[f].surface, wall_surface, "fragments inherit");
    }

    // Merge back, then undo the merge: both fragments carry the claim again.
    history
        .apply(
            &mut obj,
            KernelOp::MergeFaces {
                edge: r.new_edges[0],
            },
        )
        .unwrap();
    history.undo(&mut obj).unwrap();
    let attributed = obj
        .faces()
        .values()
        .filter(|f| f.surface == wall_surface)
        .count();
    assert_eq!(
        attributed, 9,
        "8 original walls minus 1 split into 2 = 9 attributed facets"
    );
    obj.validate().unwrap();
}

/// The attribute snapshot is best-effort, never a merge refusal: a
/// sub-nanometer sliver quad (constructible only through the imported
/// polygon-soup entry path — drawn geometry cannot reach it) merges exactly
/// as it did before snapshots existed, and records no restore anchor for
/// the unpinnable face (the fallback: undo inherits the survivor's
/// attributes for that side). Undoing THIS merge fails typed — the inverse
/// split's path grazes the sliver's far boundary within `tol::POINT_MERGE`,
/// so `split_face` refuses it — exactly as it did before the snapshot
/// mechanism existed (verified empirically at the pre-snapshot commit):
/// undo fails typed and leaves the object untouched, never corrupts.
#[test]
fn sliver_faces_merge_with_best_effort_snapshots() {
    // A unit quad sharing its x = 1 edge with a 1e-12-wide sliver quad —
    // far below tol::POINT_MERGE, so no interior point can be pinned in the
    // sliver, while from_polygons happily accepts it.
    let w = 1e-12;
    let obj = Object::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(1.0 + w, 0.0, 0.0),
            Point3::new(1.0 + w, 1.0, 0.0),
        ],
        &[vec![0, 1, 2, 3], vec![1, 4, 5, 2]],
    )
    .unwrap();
    let mut obj = obj;
    let mut history = History::new();

    let shared_edge = obj
        .edges()
        .iter()
        .find(|(_, e)| e.twin_half_edge.is_some())
        .map(|(id, _)| id)
        .unwrap();

    // Forward behavior preserved: the merge succeeds at 1e-12 width.
    let report = history
        .apply(&mut obj, KernelOp::MergeFaces { edge: shared_edge })
        .unwrap();
    assert_eq!(obj.faces().len(), 1, "sliver merges exactly as before");

    // The fallback is exercised: the unpinnable sliver has no anchor, the
    // healthy quad keeps one.
    let KernelOpReport::FaceMerge(r) = report else {
        panic!("wrong report kind")
    };
    let anchors = r.prior_attrs.iter().filter(|a| a.is_some()).count();
    assert_eq!(
        anchors, 1,
        "one best-effort anchor: the quad pins, the sliver cannot"
    );

    // Undo of a sub-POINT_MERGE sliver merge fails typed (the inverse
    // split's path grazes the far boundary within tolerance) — the
    // PRE-EXISTING sliver undo limitation, unchanged by the snapshot
    // mechanism — and the strong guarantee holds: the object is untouched.
    assert!(
        history.undo(&mut obj).is_err(),
        "sliver undo fails typed, as it did pre-snapshot"
    );
    assert_eq!(obj.faces().len(), 1, "failed undo leaves the merge intact");
    obj.validate().unwrap();
}

/// The same construction at healthy widths pins BOTH anchors and undo
/// restores the geometry — the fallback is the exception, not the rule.
/// 1e-8 sits in the band an earlier version of the snapshot wrongly
/// refused to merge; both widths must keep merging and round-tripping.
#[test]
fn healthy_widths_pin_both_snapshot_anchors() {
    for w in [1e-6f64, 1e-8] {
        let mut obj = Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(1.0 + w, 0.0, 0.0),
                Point3::new(1.0 + w, 1.0, 0.0),
            ],
            &[vec![0, 1, 2, 3], vec![1, 4, 5, 2]],
        )
        .unwrap();
        let mut history = History::new();
        let shared_edge = obj
            .edges()
            .iter()
            .find(|(_, e)| e.twin_half_edge.is_some())
            .map(|(id, _)| id)
            .unwrap();
        let report = history
            .apply(&mut obj, KernelOp::MergeFaces { edge: shared_edge })
            .unwrap();
        let KernelOpReport::FaceMerge(r) = report else {
            panic!("wrong report kind")
        };
        assert!(
            r.prior_attrs.iter().all(|a| a.is_some()),
            "width {w:e} pins both anchors"
        );
        history.undo(&mut obj).unwrap();
        assert_eq!(obj.faces().len(), 2, "width {w:e} round-trips");
        obj.validate().unwrap();
    }
}
