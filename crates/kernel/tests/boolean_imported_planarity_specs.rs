//! Booleans against imported geometry that is flat only to the import
//! tolerance (KB-25).
//!
//! An imported face is accepted when its vertices lie within
//! `tol::IMPORT_PLANE_DIST` of its plane, but the boolean's arrangement
//! imprints each face on a sketch that takes only points within
//! `tol::PLANE_DIST`. Every boolean that reached such a face used to refuse
//! as a degenerate contact. Now the boolean splits the faces the other
//! operand reaches into flat triangles through their own vertices, passes
//! the others through whole, and leaves flat faces on the old path.
//!
//! The operand here is a unit box built through the import builder with its
//! top corner `(1, 1, 1)` raised by `d`, which bends only the top quad.

use kernel::{BooleanError, BooleanOp, Object, Point3, Transform, WatertightState, tol};
use proptest::prelude::*;

const BENDS: [f64; 4] = [1e-8, 1e-7, 1e-6, 1e-5];
const OPS: [BooleanOp; 3] = [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect];

fn corners(lo: Point3, hi: Point3) -> Vec<Point3> {
    vec![
        Point3::new(lo.x, lo.y, lo.z),
        Point3::new(hi.x, lo.y, lo.z),
        Point3::new(hi.x, hi.y, lo.z),
        Point3::new(lo.x, hi.y, lo.z),
        Point3::new(lo.x, lo.y, hi.z),
        Point3::new(hi.x, lo.y, hi.z),
        Point3::new(hi.x, hi.y, hi.z),
        Point3::new(lo.x, hi.y, hi.z),
    ]
}

fn box_faces() -> Vec<Vec<usize>> {
    vec![
        vec![0, 3, 2, 1], // bottom, -z
        vec![4, 5, 6, 7], // top, +z
        vec![0, 1, 5, 4], // -y
        vec![1, 2, 6, 5], // +x
        vec![2, 3, 7, 6], // +y
        vec![3, 0, 4, 7], // -x
    ]
}

fn native_box(lo: Point3, hi: Point3) -> Object {
    Object::from_polygons(&corners(lo, hi), &box_faces()).unwrap()
}

/// The box `lo..hi` through the import builder, its top `hi` corner raised
/// by `d`.
fn bent_box(lo: Point3, hi: Point3, d: f64) -> Object {
    let mut pts = corners(lo, hi);
    pts[6].z += d;
    let obj = Object::from_polygons_import_for_test(&pts, &box_faces()).unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj
}

fn unit_bent(d: f64) -> Object {
    bent_box(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0), d)
}

fn volume(obj: &Object) -> f64 {
    obj.volume().expect("a solid")
}

/// Ok, watertight, valid, and at the import tolerance.
fn solid(r: Result<Object, BooleanError>, what: &str) -> Object {
    let r = r.unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert_eq!(r.watertight(), WatertightState::Watertight, "{what}");
    r.validate()
        .unwrap_or_else(|e| panic!("{what}: invalid {e:?}"));
    r
}

fn has_vertex(obj: &Object, p: Point3) -> bool {
    obj.vertices().values().any(|v| v.position == p)
}

/// The outer-loop positions of every face.
fn face_rings(obj: &Object) -> Vec<Vec<Point3>> {
    obj.faces()
        .values()
        .map(|f| obj.loop_positions(f.outer_loop).collect())
        .collect()
}

fn soft_edges(obj: &Object) -> Vec<(Point3, Point3)> {
    obj.edges()
        .values()
        .filter(|e| e.soft)
        .map(|e| {
            let h = obj.half_edges()[e.half_edge];
            let a = obj.vertices()[h.origin].position;
            let b = obj.vertices()[obj.half_edges()[h.next].origin].position;
            (a, b)
        })
        .collect()
}

fn run(op: BooleanOp, a: &Object, b: &Object) -> Result<Object, BooleanError> {
    Object::boolean(op, a, b, &Transform::IDENTITY)
}

// ─────────────────────────────────────────── 1. cutter away from the bent face

/// A cutter through the lower half never comes near the bent top: the top
/// passes through whole, its four corners bit for bit.
#[test]
fn a_cutter_away_from_the_bent_face_leaves_it_whole() {
    let cutter = native_box(Point3::new(0.25, 0.25, -0.5), Point3::new(0.75, 0.75, 0.5));
    for d in BENDS {
        let a = unit_bent(d);
        let top: Vec<Point3> = a
            .faces()
            .values()
            .map(|f| a.loop_positions(f.outer_loop).collect::<Vec<_>>())
            .find(|r| r.iter().any(|p| p.z > 1.0))
            .unwrap();
        for op in OPS {
            let what = format!("{op:?} d={d:e}");
            let r = solid(run(op, &a, &cutter), &what);
            assert_eq!(r.planarity_tol(), tol::IMPORT_PLANE_DIST, "{what}");
            let expect = match op {
                BooleanOp::Union => 1.0 + 0.125,
                BooleanOp::Subtract => 1.0 - 0.125,
                BooleanOp::Intersect => 0.125,
            };
            assert!(
                (volume(&r) - expect).abs() <= d,
                "{what}: vol {}",
                volume(&r)
            );
            let kept_top = face_rings(&r)
                .into_iter()
                .find(|ring| ring.len() == 4 && top.iter().all(|p| ring.contains(p)));
            match op {
                BooleanOp::Intersect => assert!(kept_top.is_none(), "{what}"),
                _ => assert!(
                    kept_top.is_some(),
                    "{what}: the top is one quad, bit for bit"
                ),
            }
        }
    }
}

// ─────────────────────────────────────────── 2. cutter through the bent face

/// A cutter through the bent top: the top is split into flat triangles
/// through its own corners, the cut goes through, every surviving corner
/// of the box is bit-identical, and the diagonal the split drew is soft.
#[test]
fn a_cutter_through_the_bent_face_cuts_it() {
    let cutter = native_box(Point3::new(0.25, 0.25, 0.5), Point3::new(0.75, 0.75, 1.5));
    for d in BENDS {
        let a = unit_bent(d);
        let originals: Vec<Point3> = a.vertices().values().map(|v| v.position).collect();
        for op in OPS {
            let what = format!("{op:?} d={d:e}");
            let r = solid(run(op, &a, &cutter), &what);
            assert_eq!(r.planarity_tol(), tol::IMPORT_PLANE_DIST, "{what}");
            let expect = match op {
                BooleanOp::Union => 1.0 + 0.125,
                BooleanOp::Subtract => 1.0 - 0.125,
                BooleanOp::Intersect => 0.125,
            };
            assert!(
                (volume(&r) - expect).abs() <= d,
                "{what}: vol {}",
                volume(&r)
            );
            if op != BooleanOp::Intersect {
                for &p in &originals {
                    assert!(has_vertex(&r, p), "{what}: corner {p:?} kept bit for bit");
                }
            }
            // The split's diagonal runs from (1, 0) to (0, 1) across the
            // top; what is left of it is soft, and nothing else is.
            let soft = soft_edges(&r);
            assert!(!soft.is_empty(), "{what}: the diagonal is soft");
            for (p, q) in soft {
                for x in [p, q] {
                    assert!(
                        (x.x + x.y - 1.0).abs() < 1e-9,
                        "{what}: soft edge off the diagonal"
                    );
                    assert!(x.z >= 1.0 - 1e-12, "{what}: soft edge below the top");
                }
            }
        }
    }
}

// ─────────────────────────────────────────── 3. both operands imported and bent

#[test]
fn two_bent_imported_operands_cross() {
    for d in BENDS {
        let a = unit_bent(d);
        let b = bent_box(Point3::new(0.5, 0.5, 0.5), Point3::new(1.5, 1.5, 1.5), d);
        for op in OPS {
            let what = format!("{op:?} d={d:e}");
            let r = solid(run(op, &a, &b), &what);
            let expect = match op {
                BooleanOp::Union => 2.0 - 0.125,
                BooleanOp::Subtract => 1.0 - 0.125,
                BooleanOp::Intersect => 0.125,
            };
            assert!(
                (volume(&r) - expect).abs() <= 2.0 * d,
                "{what}: vol {}",
                volume(&r)
            );
        }
    }
}

// ─────────────────────────────────────────── 4. a bent face with a hole

/// A 2×2 square frame with a 1×1 square through-hole, 1 tall, imported,
/// its top (the face with the hole) bent at the outer corner (2, 2, 1).
fn bent_frame(d: f64) -> Object {
    let mut pts = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(2.0, 0.0, 1.0),
        Point3::new(2.0, 2.0, 1.0),
        Point3::new(0.0, 2.0, 1.0),
        Point3::new(0.5, 0.5, 0.0),
        Point3::new(1.5, 0.5, 0.0),
        Point3::new(1.5, 1.5, 0.0),
        Point3::new(0.5, 1.5, 0.0),
        Point3::new(0.5, 0.5, 1.0),
        Point3::new(1.5, 0.5, 1.0),
        Point3::new(1.5, 1.5, 1.0),
        Point3::new(0.5, 1.5, 1.0),
    ];
    pts[6].z += d;
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
        vec![9, 8, 12, 13],
        vec![10, 9, 13, 14],
        vec![11, 10, 14, 15],
        vec![8, 11, 15, 12],
    ];
    let mut holes = vec![Vec::new(); faces.len()];
    holes[0] = vec![vec![8, 9, 10, 11]];
    holes[1] = vec![vec![12, 15, 14, 13]];
    let obj = Object::from_polygons_with_holes_import_for_test(&pts, &faces, &holes).unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj
}

#[test]
fn a_bent_face_with_a_hole_is_cut_across_its_rim_and_its_hole() {
    let across_rim = native_box(Point3::new(1.75, 0.75, 0.5), Point3::new(2.25, 1.25, 1.5));
    let across_hole = native_box(Point3::new(1.25, 0.75, 0.5), Point3::new(1.75, 1.25, 1.5));
    for d in BENDS {
        let a = bent_frame(d);
        // The bend lifts the top over an area of 4 by at most `d`, so any
        // volume read off it is within a few `d` of the flat frame's.
        assert!((volume(&a) - 3.0).abs() <= 4.0 * d);
        for (name, cutter) in [("rim", &across_rim), ("hole", &across_hole)] {
            for op in OPS {
                let what = format!("{op:?} across the {name}, d={d:e}");
                let r = solid(run(op, &a, cutter), &what);
                let expect = match op {
                    BooleanOp::Union => 3.0 + 0.25 - 0.0625,
                    BooleanOp::Subtract => 3.0 - 0.0625,
                    BooleanOp::Intersect => 0.0625,
                };
                assert!(
                    (volume(&r) - expect).abs() <= 4.0 * d,
                    "{what}: vol {}",
                    volume(&r)
                );
            }
        }
    }
}

// ─────────────────────────────────────────── 5. refusals keep their names

/// Boxes meeting only along an edge still refuse as a degenerate contact,
/// and a disjoint subtract still removes nothing, whether the operand is
/// native or imported and bent.
#[test]
fn refusals_keep_their_names_with_a_bent_operand() {
    let edge_touch = native_box(Point3::new(1.0, -1.0, 0.0), Point3::new(2.0, 0.0, 1.0));
    let apart = native_box(Point3::new(3.0, 0.0, 0.0), Point3::new(4.0, 1.0, 1.0));
    let native = native_box(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
    let mut operands = vec![("native", native)];
    for d in BENDS {
        operands.push(("bent", unit_bent(d)));
    }
    for (name, a) in &operands {
        assert_eq!(
            run(BooleanOp::Union, a, &edge_touch).unwrap_err(),
            BooleanError::DegenerateContact,
            "{name}"
        );
        assert_eq!(
            run(BooleanOp::Subtract, a, &apart).unwrap_err(),
            BooleanError::NothingRemoved,
            "{name}"
        );
    }
}

// ─────────────────────────────────────────── properties

/// A random exact prism: a regular `n`-gon of radius `r` about `(cx, cy)`
/// turned by `turn`, from `z0` to `z1`, through the native or the import
/// builder.
fn prism(
    (cx, cy): (f64, f64),
    r: f64,
    n: usize,
    turn: f64,
    (z0, z1): (f64, f64),
    import: bool,
) -> Object {
    let mut pts = Vec::with_capacity(2 * n);
    for z in [z0, z1] {
        for k in 0..n {
            let a = turn + std::f64::consts::TAU * k as f64 / n as f64;
            pts.push(Point3::new(cx + r * a.cos(), cy + r * a.sin(), z));
        }
    }
    let mut faces = vec![(0..n).rev().collect::<Vec<_>>(), (n..2 * n).collect()];
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, j + n, i + n]);
    }
    if import {
        Object::from_polygons_import_for_test(&pts, &faces).unwrap()
    } else {
        Object::from_polygons(&pts, &faces).unwrap()
    }
}

/// The two encodings differ only in the one byte that says "imported".
fn same_but_imported(native: &[u8], imported: &[u8]) -> bool {
    native.len() == imported.len()
        && native
            .iter()
            .zip(imported)
            .filter(|(a, b)| a != b)
            .map(|(&a, &b)| (a, b))
            .collect::<Vec<_>>()
            == vec![(0u8, 1u8)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Flat faces of an imported operand take exactly the native path: the
    /// same boolean of the same exact boxes and prisms, built natively and
    /// through the import builder, gives the same result (or the same
    /// refusal), byte for byte apart from the imported flag.
    #[test]
    fn imported_flat_operands_match_native_byte_for_byte(
        (ax, ay, az) in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
        (aw, ad, ah) in (0.2f64..2.0, 0.2f64..2.0, 0.2f64..2.0),
        (cx, cy) in (-1.0f64..2.0, -1.0f64..2.0),
        r in 0.1f64..1.5,
        n in 3usize..9,
        turn in 0.0f64..std::f64::consts::TAU,
        (z0, h) in (-1.0f64..1.5, 0.1f64..2.0),
        op_pick in 0usize..3,
        swap in any::<bool>(),
    ) {
        let lo = Point3::new(ax, ay, az);
        let hi = Point3::new(ax + aw, ay + ad, az + ah);
        let boxes = (
            Object::from_polygons(&corners(lo, hi), &box_faces()).unwrap(),
            Object::from_polygons_import_for_test(&corners(lo, hi), &box_faces()).unwrap(),
        );
        let prisms = (
            prism((cx, cy), r, n, turn, (z0, z0 + h), false),
            prism((cx, cy), r, n, turn, (z0, z0 + h), true),
        );
        let op = OPS[op_pick];
        let (native, imported) = if swap {
            (run(op, &prisms.0, &boxes.0), run(op, &prisms.1, &boxes.1))
        } else {
            (run(op, &boxes.0, &prisms.0), run(op, &boxes.1, &prisms.1))
        };
        match (native, imported) {
            (Ok(n), Ok(i)) => {
                let (nb, ib) = (n.encode(&|_| 0), i.encode(&|_| 0));
                prop_assert!(same_but_imported(&nb, &ib), "results differ");
            }
            (Err(n), Err(i)) => prop_assert_eq!(n, i),
            (n, i) => prop_assert!(false, "native {:?} vs imported {:?}", n.map(|_| ()), i.map(|_| ())),
        }
    }

    /// A cutter anywhere across the bent top, at any bend: the result is a
    /// solid or one of the refusals that were always possible, never one of
    /// the two new ones, and the volumes add up.
    #[test]
    fn cuts_across_a_bent_face_resolve(
        d in 1e-8f64..1e-5,
        (x0, y0) in (-0.5f64..0.9, -0.5f64..0.9),
        (w, l) in (0.05f64..1.0, 0.05f64..1.0),
        (z0, z1) in (0.2f64..0.95, 1.05f64..1.6),
    ) {
        let a = unit_bent(d);
        let b = native_box(Point3::new(x0, y0, z0), Point3::new(x0 + w, y0 + l, z1));
        let mut vols = Vec::new();
        for op in [BooleanOp::Union, BooleanOp::Intersect] {
            match run(op, &a, &b) {
                Ok(r) => {
                    prop_assert_eq!(r.watertight(), WatertightState::Watertight);
                    prop_assert!(r.validate().is_ok());
                    vols.push(volume(&r));
                }
                Err(e) => {
                    prop_assert!(
                        matches!(
                            e,
                            BooleanError::DegenerateContact
                                | BooleanError::NothingRemoved
                                | BooleanError::EmptyResult
                        ),
                        "{:?}", e
                    );
                }
            }
        }
        match run(BooleanOp::Subtract, &a, &b) {
            Ok(r) => {
                prop_assert_eq!(r.watertight(), WatertightState::Watertight);
                prop_assert!(r.validate().is_ok());
            }
            Err(e) => prop_assert!(
                matches!(
                    e,
                    BooleanError::DegenerateContact
                        | BooleanError::NothingRemoved
                        | BooleanError::EmptyResult
                ),
                "{:?}", e
            ),
        }
        if let [u, i] = vols[..] {
            let sum = volume(&a) + volume(&b);
            prop_assert!((u + i - sum).abs() <= 10.0 * d, "{} + {} vs {}", u, i, sum);
        }
    }
}
