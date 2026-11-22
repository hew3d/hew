//! Executable specs for "smoothness survives what it should"
//! (true-curves.md §3.3 rules 1–3): booleans re-derive soft seams, creases,
//! circle claims and blend-id uniqueness; a dropped claim leaves soft seams
//! (E1) on solids and in world sketches; and one kernel predicate,
//! `Object::edge_smoothness`, classifies every edge.

use crate::common;
use kernel::trig;

use std::f64::consts::{PI, TAU};

use common::curves::*;
use kernel::{
    BlendId, BlendKind, BlendRole, BlendTag, BooleanOp, CurveGeom, Document, EdgeId,
    EdgeSmoothness, Object, Point3, Sketch, SketchCurveKind, SurfaceRef, Transform, Vec3, tol,
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/smoothness")
        .join(name)
}

/// Two plain boxes overlapping by half, with no claims or soft edges.
fn plain_pair() -> (Object, Object) {
    let a = box_xyz(1.0, 1.0, 1.0);
    let mut b = box_xyz(1.0, 1.0, 1.0);
    b.apply_transform(&Transform::translation(Vec3::new(0.5, 0.5, 0.5)))
        .unwrap();
    (a, b)
}

#[test]
fn boolean_without_claims_is_byte_identical() {
    // The expected bytes were captured from the boolean before boolean
    // re-derivation existed: a result with no claims, tags or soft edges
    // must not move by a byte (this guards every replay fixture).
    let (a, b) = plain_pair();
    for (op, name) in [
        (BooleanOp::Union, "union.bin"),
        (BooleanOp::Subtract, "subtract.bin"),
    ] {
        let r = Object::boolean(op, &a, &b, &Transform::IDENTITY).unwrap();
        let bytes = r.encode(&|_| 0);
        let path = fixture(name);
        if std::env::var("REGENERATE_SMOOTHNESS_FIXTURES").is_ok() {
            std::fs::write(&path, &bytes).unwrap();
        }
        let captured = std::fs::read(&path).expect("fixture captured before re-derivation");
        // Captured as a v10 buffer: read back and written by today's
        // writer, so only the layout's newer per-face flags move, never
        // the geometry.
        let expected = Object::decode(&captured, &|_| None)
            .expect("the fixture opens")
            .encode(&|_| 0);
        assert_eq!(bytes, expected, "{name}: a claim-free boolean is unchanged");
    }
}

// ------------------------------------------------------------ helpers

fn edge_ends(o: &Object, e: EdgeId) -> (Point3, Point3) {
    o.edge_endpoints(e).unwrap()
}

fn seg_dist(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn inside_box(p: Point3, lo: Point3, hi: Point3, margin: f64) -> bool {
    p.x > lo.x - margin
        && p.x < hi.x + margin
        && p.y > lo.y - margin
        && p.y < hi.y + margin
        && p.z > lo.z - margin
        && p.z < hi.z + margin
}

/// A box with corners `lo`, `hi`.
fn box_at(lo: Point3, hi: Point3) -> Object {
    let mut b = box_xyz(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z);
    b.apply_transform(&Transform::translation(lo - Point3::new(0.0, 0.0, 0.0)))
        .unwrap();
    b
}

/// A ring: a circle of radius `r` (in the XZ plane, centered at `(R, 0, 0)`)
/// swept around a 24-gon circle path of radius `R` about +Z.
fn swept_ring(big_r: f64, r: f64) -> Object {
    let profile = circle_profile(p3(big_r, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), r, 24);
    let n = 24;
    let path: Vec<Point3> = (0..n)
        .map(|i| {
            let a = TAU * i as f64 / n as f64;
            p3(big_r * trig::cos(a), big_r * trig::sin(a), 0.0)
        })
        .collect();
    let curves = vec![
        Some(CurveGeom {
            center: p3(0.0, 0.0, 0.0),
            radius: big_r
        });
        n
    ];
    Object::from_follow_me(&profile, &path, true, &curves).unwrap()
}

// ---------------------------------------------------------- 1. ring seams

#[test]
fn boolean_keeps_soft_seams_of_a_swept_ring() {
    let ring = swept_ring(1.0, 0.2);
    assert_valid_solid(&ring);
    let source_soft: Vec<(Point3, Point3)> = ring
        .edges()
        .iter()
        .filter(|(_, e)| e.soft)
        .map(|(id, _)| edge_ends(&ring, id))
        .collect();
    assert!(
        !source_soft.is_empty(),
        "a swept ring is born with soft seams"
    );
    // A box overlapping the +x+y quarter of the ring.
    let (lo, hi) = (p3(0.3, 0.3, -0.5), p3(2.0, 2.0, 0.5));
    let b = box_at(lo, hi);
    let result = Object::boolean(BooleanOp::Union, &ring, &b, &Transform::IDENTITY).unwrap();
    assert_valid_solid(&result);
    let mut soft_on_seams = 0;
    for (eid, _) in result.edges() {
        let (p, q) = edge_ends(&result, eid);
        let on_seam = source_soft.iter().any(|&(a, c)| {
            seg_dist(p, a, c) <= tol::POINT_MERGE && seg_dist(q, a, c) <= tol::POINT_MERGE
        });
        if on_seam {
            assert!(
                result.edges()[eid].soft,
                "a surviving piece of a ring seam stays soft ({p:?}–{q:?})"
            );
            soft_on_seams += 1;
            assert!(
                !(inside_box(p, lo, hi, -tol::POINT_MERGE)
                    && inside_box(q, lo, hi, -tol::POINT_MERGE)),
                "no seam survives inside the box"
            );
        }
    }
    let wholly_outside = source_soft
        .iter()
        .filter(|&&(a, c)| !inside_box(a, lo, hi, 1e-6) && !inside_box(c, lo, hi, 1e-6))
        .count();
    assert!(
        soft_on_seams >= wholly_outside,
        "every seam outside the box survives soft ({soft_on_seams} < {wholly_outside})"
    );
}

// ------------------------------------------------------ 2. rim claims

#[test]
fn boolean_regains_cylinder_rim_claims() {
    let (r, h, n) = (0.5, 1.0, 48);
    let cyl = cylinder(r, h, n);
    // A notch through the +x side near the top.
    let notch = box_at(p3(0.4, -0.1, 0.8), p3(0.7, 0.1, 1.2));
    let result = Object::boolean(BooleanOp::Subtract, &cyl, &notch, &Transform::IDENTITY).unwrap();
    assert_valid_solid(&result);
    let top_rim: Vec<EdgeId> = result
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = edge_ends(&result, e);
            let on = |x: Point3| {
                (x.z - h).abs() <= 1e-12 && ((x.x * x.x + x.y * x.y).sqrt() - r).abs() <= 1e-9
            };
            on(p) && on(q)
        })
        .collect();
    assert!(
        top_rim.len() >= n - 6 && top_rim.len() < n,
        "most of the top rim survives the notch ({})",
        top_rim.len()
    );
    for &e in &top_rim {
        assert_eq!(
            result.edges()[e].curve,
            Some(CurveGeom {
                center: p3(0.0, 0.0, h),
                radius: r
            }),
            "an intact top rim chord regains its circle claim"
        );
    }
    // The wall's analytic rims still cover everything but the notch.
    let rims = result.analytic_rims();
    let top = rims
        .iter()
        .find(|rim| (rim.center.z - h).abs() <= 1e-9)
        .expect("top rim");
    // Angles in the rim's own (u, v) frame: world +x is the notch, −x the
    // far side.
    let angle_of =
        |rim: &kernel::AnalyticRim, d: Vec3| trig::atan2(d.dot(rim.basis_v), d.dot(rim.basis_u));
    let (notch, far) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0));
    assert!(
        top.covers(angle_of(top, far)),
        "the far side of the top rim is covered"
    );
    assert!(!top.covers(angle_of(top, notch)), "the notched arc is not");
    // The re-derived edge claims describe the same circle the wall's rim
    // does, so the drawn-circle rims (deduplicated against the walls') add
    // no second top rim.
    assert!(
        !result
            .edge_curve_rims()
            .iter()
            .any(|rim| (rim.center.z - h).abs() <= 1e-9),
        "the regained rim claims coincide with the wall's own rim"
    );
}

// -------------------------------------------- 3. rounded corners, two trims

#[test]
fn tub_corners_stay_smooth_through_two_trims() {
    let mut tub = rounded_rect_prism(1.0, 0.6, 0.15, 0.4, 6);
    tub.apply_transform(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), PI / 6.0).unwrap())
        .unwrap();
    let cut1 = box_at(p3(-0.1, -1.0, 0.3), p3(0.1, 1.0, 0.6));
    let cut2 = box_at(p3(-1.0, -0.05, 0.35), p3(1.0, 0.05, 0.6));
    let once = Object::boolean(BooleanOp::Subtract, &tub, &cut1, &Transform::IDENTITY).unwrap();
    let twice = Object::boolean(BooleanOp::Subtract, &once, &cut2, &Transform::IDENTITY).unwrap();
    assert_valid_solid(&twice);
    let mut seams = 0;
    for (eid, e) in twice.edges() {
        let Some(t) = e.twin_half_edge else { continue };
        let face = |h| twice.faces()[twice.loops()[twice.half_edges()[h].loop_id].face].surface;
        if let (Some(a), Some(b)) = (face(e.half_edge), face(t))
            && a.same_surface(&b)
        {
            seams += 1;
            assert!(e.soft, "a corner facet seam is soft after two trims");
            assert_eq!(twice.edge_smoothness(eid), EdgeSmoothness::Seam);
        }
    }
    assert!(seams >= 4 * 5, "the corner seams survive");
}

// ------------------------------------------- 4. crease by provenance

#[test]
fn crease_survives_boolean_by_provenance() {
    let mut a = unit_box();
    let bottom_front = edge_between(&a, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    a.set_edge_crease_for_test(bottom_front, true);
    a.validate().unwrap();
    // B overlaps the box's back-top corner region and splits the creased
    // edge nowhere; a second B straddles x = 0.5 across the front face and
    // splits it.
    let b = box_at(p3(0.25, -0.5, 0.25), p3(0.75, 0.5, 0.75));
    let r = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    assert_valid_solid(&r);
    let creased: Vec<(Point3, Point3)> = r
        .edges()
        .iter()
        .filter(|(_, e)| e.crease)
        .map(|(id, _)| edge_ends(&r, id))
        .collect();
    assert!(!creased.is_empty(), "the crease survives");
    for (p, q) in &creased {
        assert!(seg_dist(*p, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)) <= tol::POINT_MERGE);
        assert!(seg_dist(*q, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)) <= tol::POINT_MERGE);
    }
    // Every piece of the original edge is creased.
    let pieces = r
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = edge_ends(&r, e);
            seg_dist(p, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)) <= tol::POINT_MERGE
                && seg_dist(q, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)) <= tol::POINT_MERGE
        })
        .count();
    assert_eq!(pieces, creased.len());
}

// ------------------------------------------- 5. blend id uniqueness

#[test]
fn boolean_renumbers_operand_b_blend_ids() {
    let t = |id| BlendTag {
        id: BlendId(id),
        kind: BlendKind::Fillet,
        role: BlendRole::Strip,
    };
    let mut a = unit_box();
    let top_a = face_with_normal(&a, Z);
    a.set_face_blend_for_test(top_a, Some(t(1)));
    let mut b = box_at(p3(0.5, 0.5, 0.5), p3(1.5, 1.5, 1.5));
    let bottom_b = face_with_normal(&b, -Z);
    let side_b = face_with_normal(&b, Vec3::new(1.0, 0.0, 0.0));
    b.set_face_blend_for_test(bottom_b, Some(t(1)));
    b.set_face_blend_for_test(side_b, Some(t(2)));
    let r = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    assert_valid_solid(&r);
    let ids: std::collections::BTreeSet<u32> = r
        .faces()
        .values()
        .filter_map(|f| f.blend.map(|t| t.id.0))
        .collect();
    assert_eq!(
        ids,
        [1, 2, 3].into_iter().collect(),
        "B's ids move past A's largest: no two blends share an id"
    );
}

// ------------------------------------------- 6. non-uniform scale (E1)

#[test]
fn non_uniform_scale_leaves_soft_seams() {
    let mut cyl = cylinder(0.5, 1.0, 48);
    let seams_before: usize = cyl
        .edges()
        .keys()
        .filter(|&e| cyl.edge_smoothness(e) == EdgeSmoothness::Seam)
        .count();
    assert_eq!(seams_before, 48);
    cyl.apply_transform(&Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    assert_valid_solid(&cyl);
    assert!(
        cyl.faces().values().all(|f| f.surface.is_none()),
        "claims drop"
    );
    let soft = cyl.edges().values().filter(|e| e.soft).count();
    assert_eq!(soft, 48, "every former wall seam is soft");
    let seams_after = cyl
        .edges()
        .keys()
        .filter(|&e| cyl.edge_smoothness(e) == EdgeSmoothness::Seam)
        .count();
    assert_eq!(seams_after, 48);
}

// ------------------------------------- 7–8. stretched sketch chains (E1)

/// A document holding one ground sketch with a closed chain of `n` facets
/// (a circle or a hexagon), scaled (2, 1, 1), saved and reloaded, then
/// extruded. Returns the reloaded document's extruded object.
fn stretched_chain_extruded(kind: SketchCurveKind, n: usize) -> Object {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk: &mut Sketch = doc.sketch_mut(s).unwrap();
        sk.begin_curve_with_kind(
            CurveGeom {
                center: p3(0.0, 0.0, 0.0),
                radius: 0.5,
            },
            kind,
        )
        .unwrap();
        let p = |i: usize| {
            let a = TAU * (i % n) as f64 / n as f64;
            p3(0.5 * trig::cos(a), 0.5 * trig::sin(a), 0.0)
        };
        for i in 0..n {
            sk.add_segment(p(i), p(i + 1)).unwrap();
        }
        sk.end_curve();
    }
    doc.end_sketch_gesture(s).unwrap();
    doc.transform_sketch(s, &Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    let mut doc = Document::load(&doc.save()).unwrap();
    let s = doc.sketch_ids()[0];
    let region = doc.extrudable_regions(s).unwrap()[0];
    let (obj, _) = doc.extrude_region(s, region, 1.0).unwrap();
    doc.object(obj).unwrap().clone()
}

fn wall_seams(o: &Object) -> Vec<EdgeId> {
    o.edges()
        .keys()
        .filter(|&e| {
            let (p, q) = edge_ends(o, e);
            (p.z - q.z).abs() > 0.5
        })
        .collect()
}

#[test]
fn stretched_sketch_circle_extrudes_smooth() {
    let o = stretched_chain_extruded(SketchCurveKind::Circle, 24);
    assert_valid_solid(&o);
    let seams = wall_seams(&o);
    assert_eq!(seams.len(), 24);
    for e in seams {
        assert!(o.edges()[e].soft, "a stretched circle's walls shade smooth");
    }
}

#[test]
fn stretched_sketch_polygon_extrudes_hard() {
    let o = stretched_chain_extruded(SketchCurveKind::Polygon, 6);
    assert_valid_solid(&o);
    for e in wall_seams(&o) {
        assert!(!o.edges()[e].soft, "a stretched hexagon keeps hard sides");
        assert_eq!(o.edge_smoothness(e), EdgeSmoothness::Hard);
    }
}

// ----------------------------------------- 9. the predicate itself

#[test]
fn edge_smoothness_classification() {
    let (w, d, r, h, n) = (1.0, 0.6, 0.1, 0.3, 6);
    let mut o = rounded_rect_prism(w, d, r, h, n);
    let mut tangent = 0;
    let mut seam = 0;
    for e in o.edges().keys().collect::<Vec<_>>() {
        let (p, q) = edge_ends(&o, e);
        let vertical = (p.z - q.z).abs() > 0.5 * h;
        let class = o.edge_smoothness(e);
        if !vertical {
            assert_eq!(class, EdgeSmoothness::Hard, "rim edges are real creases");
            continue;
        }
        let on_corner_circle = |x: Point3| {
            let cx = if x.x > 0.0 { w / 2.0 - r } else { -w / 2.0 + r };
            let cy = if x.y > 0.0 { d / 2.0 - r } else { -d / 2.0 + r };
            (trig::hypot(x.x - cx, x.y - cy) - r).abs() <= 1e-9
        };
        // A wall-to-arc joint sits where a straight wall meets the arc: one
        // face claims a cylinder, the other is a plane, tangent there.
        let faces = |e: EdgeId| {
            let ed = o.edges()[e];
            let f = |h| o.faces()[o.loops()[o.half_edges()[h].loop_id].face].surface;
            (f(ed.half_edge), f(ed.twin_half_edge.unwrap()))
        };
        match faces(e) {
            (Some(_), Some(_)) => {
                assert_eq!(class, EdgeSmoothness::Seam, "arc facet seams are seams");
                seam += 1;
            }
            _ => {
                assert!(on_corner_circle(p));
                assert_eq!(
                    class,
                    EdgeSmoothness::Tangent,
                    "wall-to-arc joints are tangent"
                );
                tangent += 1;
            }
        }
    }
    assert_eq!(tangent, 8);
    assert_eq!(seam, 4 * (n - 1));

    // An imprinted rectangle on the top face stays Hard (decision I-20).
    let top = face_with_normal(&o, Z);
    o.split_face_inner(
        top,
        &[
            p3(-0.2, -0.1, h),
            p3(0.2, -0.1, h),
            p3(0.2, 0.1, h),
            p3(-0.2, 0.1, h),
        ],
    )
    .unwrap();
    let imprint = edge_between(&o, p3(-0.2, -0.1, h), p3(0.2, -0.1, h));
    assert_eq!(o.edge_smoothness(imprint), EdgeSmoothness::Hard);

    // Creasing one arc seam makes it Hard.
    let seam_edge = o
        .edges()
        .keys()
        .find(|&e| o.edge_smoothness(e) == EdgeSmoothness::Seam)
        .unwrap();
    o.set_edge_crease_for_test(seam_edge, true);
    o.validate().unwrap();
    assert_eq!(o.edge_smoothness(seam_edge), EdgeSmoothness::Hard);
    let _ = SurfaceRef::kind_name;
}

#[test]
fn a_softened_tangent_joint_outside_a_blend_stays_hidden() {
    // A blend marks the joints inside it soft and they still read as
    // tangent joints; anywhere else a soft mark is the user's Soften, and
    // a softened wall-to-arc joint hides like a facet seam.
    let mut o = rounded_rect_prism(1.0, 0.6, 0.1, 0.3, 6);
    let joint = o
        .edges()
        .keys()
        .find(|&e| o.edge_smoothness(e) == EdgeSmoothness::Tangent)
        .unwrap();
    o.set_edge_soft_for_test(joint, true);
    o.validate().unwrap();
    assert_eq!(o.edge_smoothness(joint), EdgeSmoothness::Seam);
}
