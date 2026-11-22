//! KB-25: booleans against large imported meshes (true-curves.md §7.1;
//! WP20 Part 1). A synthetic imported sphere stands in for the sculpted STL
//! the bug was reported against (rounded through f32 as STL import does,
//! built at the import planarity tolerance).

use crate::common;
use kernel::trig;

use common::curves::*;
use kernel::{BooleanOp, Object, Transform, Vec3};

fn disc_under(r: f64, overlap: f64, sphere_r: f64) -> Object {
    let mut d = cylinder(r, 0.01, 48);
    d.apply_transform(&Transform::translation(Vec3::new(
        0.0,
        0.0,
        -sphere_r + overlap - 0.01,
    )))
    .unwrap();
    d
}

#[test]
fn kb25_union_with_overlapping_disc() {
    let s = imported_sphere(60, 120, 0.02, 0);
    assert_valid_solid(&s);
    let disc = disc_under(0.015, 0.003, 0.02);
    let r = Object::boolean(BooleanOp::Union, &s, &disc, &Transform::IDENTITY);
    let r = r.expect("union with an overlapping disc succeeds");
    assert_valid_solid(&r);
}

#[test]
fn kb25_subtract_bar_through() {
    let s = imported_sphere(60, 120, 0.02, 0);
    let bar = {
        let mut b = box_xyz(0.002, 0.002, 0.06);
        b.apply_transform(&Transform::translation(Vec3::new(-0.001, -0.001, -0.03)))
            .unwrap();
        b
    };
    let r = Object::boolean(BooleanOp::Subtract, &s, &bar, &Transform::IDENTITY)
        .expect("subtracting a bar through the sphere succeeds");
    assert_valid_solid(&r);
}

#[test]
fn kb25_union_box_inside() {
    let s = imported_sphere(60, 120, 0.02, 0);
    let inner = {
        let mut b = box_xyz(0.01, 0.01, 0.01);
        b.apply_transform(&Transform::translation(Vec3::new(-0.005, -0.005, -0.005)))
            .unwrap();
        b
    };
    let r = Object::boolean(BooleanOp::Union, &s, &inner, &Transform::IDENTITY)
        .expect("a box inside unions to the sphere");
    assert_valid_solid(&r);
    assert_eq!(r.faces().len(), s.faces().len(), "the result is the sphere");
}

#[test]
fn kb25_degenerate_triangles_never_reach_a_boolean() {
    // Zero-area triangles are refused where an object is built (an import
    // heals them away first), so no boolean ever sees one: the case KB-25's
    // two zero-area pawn triangles stand for is a refusal at the builder,
    // named, never a bogus DegenerateContact downstream.
    let (pts, tris) = sphere_soup(60, 120, 0.02, 2);
    assert!(matches!(
        Object::from_polygons_import_for_test(&pts, &tris),
        Err(kernel::TopologyError::DegenerateFace { .. })
    ));
}

/// A non-planar imported face (flat only to the import tolerance) meets a
/// native disc: the arrangement threads the face's own tolerance, and the
/// seam ends on the imported face's edges where its neighbors put them, so
/// the union welds (H2 + the seam-endpoint fix).
#[test]
fn kb25_non_planar_imported_faces_union() {
    let s = warped_box();
    assert_valid_solid(&s);
    // A disc standing through the warped top face, off-center toward the
    // lifted corner so its rim crosses the warped side faces' neighbors.
    let mut disc = cylinder(0.012, 0.03, 48);
    disc.apply_transform(&Transform::translation(Vec3::new(0.012, 0.012, 0.005)))
        .unwrap();
    let r = Object::boolean(BooleanOp::Union, &s, &disc, &Transform::IDENTITY)
        .expect("a disc through warped imported faces unions");
    assert_valid_solid(&r);
}

/// A 40 mm box with one top corner lifted 0.2 mm: the top and the two
/// side faces at that corner are warped quads, flat only to ~0.05 mm —
/// within IMPORT_PLANE_DIST, far beyond PLANE_DIST.
fn warped_box() -> Object {
    let (a, w) = (0.02, 0.0002);
    let pts = vec![
        p3(-a, -a, -a),
        p3(a, -a, -a),
        p3(a, a, -a),
        p3(-a, a, -a),
        p3(-a, -a, a),
        p3(a, -a, a),
        p3(a, a, a + w),
        p3(-a, a, a),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::from_polygons_import_for_test(&pts, &faces).expect("warped quads import")
}

/// A native box resting on a warped imported top face. The warped quad
/// lies a few µm above the box's bottom there, so the contact is a genuine
/// shallow overlap, not a coplanar one: it is seamed like any transversal
/// pair, and the union welds with the box's volume less that sliver.
#[test]
fn kb25_native_box_resting_on_a_warped_imported_face() {
    let s = warped_box();
    let mut b = box_xyz(0.01, 0.01, 0.01);
    b.apply_transform(&Transform::translation(Vec3::new(-0.015, -0.015, 0.02)))
        .unwrap();
    let r = Object::boolean(BooleanOp::Union, &s, &b, &Transform::IDENTITY)
        .expect("a box resting on a warped imported face unions");
    assert_valid_solid(&r);
    let dv = signed_volume(&r) - signed_volume(&s) - 1e-6;
    assert!((-1e-7..=0.0).contains(&dv), "volume change {dv:e}");
}

#[test]
fn kb25_large_perf() {
    if std::env::var("HEW_PERF").is_err() {
        return;
    }
    let s = imported_sphere(500, 500, 0.02, 0);
    let disc = disc_under(0.015, 0.003, 0.02);
    let t = std::time::Instant::now();
    let r = Object::boolean(BooleanOp::Union, &s, &disc, &Transform::IDENTITY).unwrap();
    let el = t.elapsed();
    eprintln!("kb25_large_perf: {:?} for {} faces", el, s.faces().len());
    assert_valid_solid(&r);
    assert!(el < std::time::Duration::from_secs(60), "took {el:?}");
}

/// A closed pawn-like lathe in millimeters (coordinates rounded through
/// f32, then scaled to meters, as an STL import is), its flat foot a fan
/// of triangles on z = 0, built as an imported object.
fn imported_lathe_mm(n: usize) -> Object {
    let prof = [
        (9.0, 0.0),
        (9.0, 3.0),
        (6.0, 4.5),
        (4.0, 12.0),
        (7.0, 14.0),
        (3.0, 16.0),
        (5.0, 22.0),
    ];
    let mm = |v: f64| f64::from(v as f32) * 0.001;
    let mut pts = vec![p3(0.0, 0.0, 0.0)];
    for &(r, z) in &prof {
        for j in 0..n {
            let a = std::f64::consts::TAU * j as f64 / n as f64;
            pts.push(p3(mm(r * trig::cos(a)), mm(r * trig::sin(a)), mm(z)));
        }
    }
    pts.push(p3(0.0, 0.0, mm(27.0)));
    let top = pts.len() - 1;
    let ring = |i: usize, j: usize| 1 + i * n + (j % n);
    let mut tris: Vec<Vec<usize>> = Vec::new();
    for j in 0..n {
        tris.push(vec![0, ring(0, j + 1), ring(0, j)]);
    }
    for i in 0..prof.len() - 1 {
        for j in 0..n {
            tris.push(vec![ring(i, j), ring(i, j + 1), ring(i + 1, j + 1)]);
            tris.push(vec![ring(i, j), ring(i + 1, j + 1), ring(i + 1, j)]);
        }
    }
    for j in 0..n {
        tris.push(vec![
            ring(prof.len() - 1, j),
            ring(prof.len() - 1, j + 1),
            top,
        ]);
    }
    Object::from_polygons_import_for_test(&pts, &tris).expect("imported lathe builds")
}

#[test]
fn kb25_union_with_a_flush_base_disc_at_millimeter_scale() {
    // The playtest's pawn on a base disc sharing the ground plane: the
    // foot's fan rests inside the disc's bottom and the volumes overlap.
    // Both operand orders weld, and a native foot does too.
    let pawn = imported_lathe_mm(48);
    assert_valid_solid(&pawn);
    let disc = cylinder(0.016, 0.004, 48);
    for (a, b) in [(&pawn, &disc), (&disc, &pawn)] {
        let r = Object::boolean(BooleanOp::Union, a, b, &Transform::IDENTITY)
            .expect("a flush base disc unions");
        assert_valid_solid(&r);
    }
    let foot = cylinder(0.009, 0.027, 48);
    let r = Object::boolean(BooleanOp::Union, &foot, &disc, &Transform::IDENTITY)
        .expect("a millimeter cylinder on a flush disc unions");
    assert_valid_solid(&r);
}

/// Volume of A ∩ B for the tee below, computed without the kernel: both
/// pipes share one polygon section P (A in xy, B in xz), so the overlap is
/// the integral over x of chord(x) (B's z-extent) times the length of A's
/// y-chord at x that lies at or beyond `end` (where B begins). Piecewise
/// quadratic between breakpoints, so Simpson per piece is exact.
fn tee_intersection_volume(a: &Object, end: f64) -> f64 {
    let base: Vec<(f64, f64)> = a
        .vertices()
        .values()
        .map(|v| v.position)
        .filter(|p| p.z.abs() < 1e-9)
        .map(|p| (p.x, p.y))
        .collect();
    let (cx, cy) = (
        base.iter().map(|p| p.0).sum::<f64>() / base.len() as f64,
        base.iter().map(|p| p.1).sum::<f64>() / base.len() as f64,
    );
    let mut poly = base;
    poly.sort_by(|p, q| {
        trig::atan2(p.1 - cy, p.0 - cx)
            .partial_cmp(&trig::atan2(q.1 - cy, q.0 - cx))
            .unwrap()
    });
    let n = poly.len();
    // y-chord [bot, top] of the convex polygon at x.
    let chord = |x: f64| -> (f64, f64) {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..n {
            let (p, q) = (poly[i], poly[(i + 1) % n]);
            if (p.0 - x) * (q.0 - x) <= 0.0 && p.0 != q.0 {
                let y = p.1 + (q.1 - p.1) * (x - p.0) / (q.0 - p.0);
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
        (lo, hi)
    };
    let f = |x: f64| -> f64 {
        let (lo, hi) = chord(x);
        if lo > hi {
            return 0.0;
        }
        (hi - lo) * (hi - lo.max(end)).max(0.0)
    };
    let mut xs: Vec<f64> = poly.iter().map(|p| p.0).collect();
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        if (p.1 - end) * (q.1 - end) < 0.0 {
            xs.push(p.0 + (q.0 - p.0) * (end - p.1) / (q.1 - p.1));
        }
    }
    xs.sort_by(|p, q| p.partial_cmp(q).unwrap());
    xs.dedup_by(|p, q| (*p - *q).abs() < 1e-15);
    // Chords are evaluated strictly inside each piece's ends to dodge the
    // vertical-edge ambiguity at the extremes.
    xs.windows(2)
        .map(|w| {
            let (l, r) = (w[0], w[1]);
            let e = (r - l) * 1e-12;
            let (l, r) = (l + e, r - e);
            (r - l) / 6.0 * (f(l) + 4.0 * f((l + r) / 2.0) + f(r))
        })
        .sum()
}

#[test]
fn kb25_grazing_equal_pipes_name_the_contact_not_an_operand() {
    // Two equal 32-sided pipes in a tee whose branch ends just past the
    // main axis: a branch generator crosses the main pipe's vertex edge
    // while the end cap slices the facet beside it. Neither operand has a
    // zero-area face, so a refusal may only ever name the grazing contact
    // (nudge), never a degenerate operand. Since the boolean keeps each
    // operand's own vertices apart and leaves faces no seam crosses as
    // they are (D-220, D-223), the sliver no longer forms and the tee
    // unions to the exact volume: A + B − (A ∩ B). Ended at the axis, the
    // same tee unions.
    let tee = |end: f64| {
        let mut a = cylinder(0.1, 0.8, 32);
        a.apply_transform(&Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
            .unwrap();
        let mut b = cylinder(0.1, 0.8, 32);
        b.apply_transform(
            &Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -std::f64::consts::FRAC_PI_2).unwrap(),
        )
        .unwrap();
        b.apply_transform(&Transform::translation(Vec3::new(3.0, end, 0.4)))
            .unwrap();
        (
            Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY),
            Object::boolean(BooleanOp::Subtract, &a, &b, &Transform::IDENTITY),
            signed_volume(&a),
            signed_volume(&b),
            tee_intersection_volume(&a, end),
        )
    };
    let (union, subtract, va, vb, inter) = tee(-0.0074);
    let expected = va + vb - inter;
    assert!(
        !matches!(union, Err(kernel::BooleanError::DegenerateInput { .. })),
        "{union:?}"
    );
    let union = union.expect("the grazing tee unions");
    assert_valid_solid(&union);
    assert!(
        (signed_volume(&union) - expected).abs() <= 1e-12 * expected,
        "union volume {} against A + B − (A ∩ B) = {expected}",
        signed_volume(&union)
    );
    // Subtract flips the cutter's walls; a sliver there is the same contact.
    assert!(
        !matches!(subtract, Err(kernel::BooleanError::DegenerateInput { .. })),
        "{subtract:?}"
    );
    let subtract = subtract.expect("the grazing tee subtracts");
    assert_valid_solid(&subtract);
    assert!(
        (signed_volume(&subtract) - (va - inter)).abs() <= 1e-12 * (va - inter),
        "subtract volume {} against A − (A ∩ B) = {}",
        signed_volume(&subtract),
        va - inter
    );
    let tee = |end: f64| tee(end).0;
    assert_valid_solid(&tee(0.0).expect("ended at the axis, the tee unions"));
}

#[test]
fn kb25_union_with_a_disc_just_under_a_vertex() {
    // The playtest pawn: the base's top face passed 0.36 µm under one of the
    // imported mesh's vertices, so the cut left a sub-micron triangle. Its
    // normal is well defined; the union succeeds and the result saves and
    // reopens.
    let s = imported_sphere(60, 120, 0.02, 0);
    let v = s
        .vertices()
        .values()
        .map(|v| v.position)
        .filter(|p| p.z < -0.012 && p.z > -0.016)
        .min_by(|a, b| a.z.partial_cmp(&b.z).unwrap())
        .unwrap();
    let mut disc = cylinder(0.03, 0.01, 48);
    let top = v.z - 3.6e-7;
    disc.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, top - 0.01)))
        .unwrap();
    let r = Object::boolean(BooleanOp::Union, &s, &disc, &Transform::IDENTITY)
        .expect("union with a disc just under a vertex succeeds");
    assert_valid_solid(&r);
    let back = Object::decode(&r.encode(&|_| 0), &|_| None).expect("the result reopens");
    assert_eq!(back.faces().len(), r.faces().len());
    // Pushing the base's top down moves the faces around the small facet;
    // they keep a plane instead of refusing as degenerate.
    let mut pushed = r.clone();
    let top_face = pushed
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().z > 0.999
                && pushed
                    .loop_positions(f.outer_loop)
                    .all(|p| (p.z - top).abs() < 1e-9)
        })
        .map(|(id, _)| id)
        .expect("the base's top face");
    pushed
        .push_pull(top_face, -0.001)
        .expect("the base's top pushes down");
    assert_valid_solid(&pushed);
    // The same contact holds for the other booleans (D-70 is a rule of the
    // geometry, not of Union): each cut leaves the same small facet.
    for (op, a, b) in [
        (BooleanOp::Subtract, &s, &disc),
        (BooleanOp::Subtract, &disc, &s),
        (BooleanOp::Intersect, &s, &disc),
    ] {
        let r = Object::boolean(op, a, b, &Transform::IDENTITY)
            .unwrap_or_else(|e| panic!("{op:?} with a disc just under a vertex: {e:?}"));
        assert_valid_solid(&r);
    }
}
