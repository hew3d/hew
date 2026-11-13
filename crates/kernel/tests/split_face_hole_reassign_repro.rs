//! Regression repro for hole reassignment during `split_face`: when a face
//! carrying an inner (hole) loop is cut in two, each hole must be handed to
//! the result face that geometrically contains it. The assignment used the
//! hole ring's VERTEX AVERAGE as a containment probe, but a vertex average
//! is not a guaranteed-interior point of a concave polygon — for a "staple"
//! (U-shaped) hole it falls in the notch, outside the hole entirely, and a
//! cut whose finger occupies exactly that notch region got the hole
//! assigned to the wrong face. Loop-ownership bookkeeping stayed
//! self-consistent, so the pre-fix validator accepted the corrupted result
//! and it survived save/load.
//!
//! Assignment now tests the hole ring itself (every vertex lies strictly on
//! one side of a valid cut), and the validator gained a conservative
//! hole-containment check (`TopologyError::HoleOutsideFace`).
//!
//! Scene: a 100×100×10 slab whose top face carries a concave staple imprint
//! entirely inside what remains face A ("the arch") after a finger-shaped
//! cut; the staple's vertex average (50, 67.5) sits in the notch, inside
//! the finger (face B).

use kernel::{Object, Point3, Vec3};

fn slab() -> Object {
    let (dx, dy, dz) = (100.0, 100.0, 10.0);
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(dx, 0.0, 0.0),
        Point3::new(dx, dy, 0.0),
        Point3::new(0.0, dy, 0.0),
        Point3::new(0.0, 0.0, dz),
        Point3::new(dx, 0.0, dz),
        Point3::new(dx, dy, dz),
        Point3::new(0.0, dy, dz),
    ];
    let f = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::from_polygons(&v, &f).unwrap()
}

/// Even-odd point-in-polygon in the z = 10 plane (x/y coordinates).
fn inside_xy(p: Point3, ring: &[Point3]) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

#[test]
fn concave_hole_is_assigned_to_the_containing_face() {
    let mut obj = slab();
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(id, _)| id)
        .expect("top face exists");

    // Concave "staple" (U-shape opening downward): a band over y in
    // [60, 90] with a notch at x in [40, 60], y in [60, 80]. Its vertex
    // average is (50, 72.5) — inside the notch, OUTSIDE the staple.
    let z = 10.0;
    let staple = vec![
        Point3::new(20.0, 60.0, z),
        Point3::new(40.0, 60.0, z),
        Point3::new(40.0, 80.0, z),
        Point3::new(60.0, 80.0, z),
        Point3::new(60.0, 60.0, z),
        Point3::new(80.0, 60.0, z),
        Point3::new(80.0, 90.0, z),
        Point3::new(20.0, 90.0, z),
    ];
    obj.split_face_inner(top, &staple).expect("staple imprints");

    // Finger cut from the bottom edge up into the notch: face B (the
    // finger) contains the staple's vertex average but no part of the
    // staple itself; face A (the arch) contains the whole staple.
    let path = vec![
        Point3::new(45.0, 0.0, z),
        Point3::new(45.0, 76.0, z),
        Point3::new(55.0, 76.0, z),
        Point3::new(55.0, 0.0, z),
    ];
    obj.split_face(top, &path).expect("finger cut applies");
    obj.validate().expect("valid after cut");

    // Every face that owns holes must geometrically contain them: all hole
    // vertices inside the face's outer ring.
    let mut checked_holes = 0;
    for (fid, face) in obj.faces() {
        let outer: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
        for &il in &face.inner_loops {
            for p in obj.loop_positions(il) {
                assert!(
                    inside_xy(p, &outer),
                    "face {fid:?} owns a hole vertex {p:?} outside its outer ring {outer:?}"
                );
            }
            checked_holes += 1;
        }
    }
    assert_eq!(checked_holes, 1, "the staple hole survives the cut");

    // And the corruption must not be reachable through persistence either:
    // the saved object round-trips through the validating decoder.
    let bytes = obj.encode(&|_| unreachable!("no materials in this repro"));
    let decoded = Object::decode(&bytes, &|_| None).expect("round-trip decodes");
    decoded.validate().expect("round-trip validates");
}

/// A face whose normal splits evenly between two axes (a vertical wall
/// running at 45°) takes a rectangle loop strictly inside it: the hole
/// check's crossing test measures every orientation along one common
/// normal, so same-side points never read as a crossing.
#[test]
fn a_loop_inside_a_diagonal_wall_splits_it() {
    use kernel::{Document, KernelOp, Plane, Point3, Vec3};
    let a = Point3::new(-3.9548952703015607, 5.419604672537572, -2.176461726410028);
    let b = Point3::new(-3.9227148743424785, 5.451785068496655, -2.176461726410028);
    let n = Vec3::new(1.0, -1.0, 0.0) * (1.0 / 2f64.sqrt());
    let (c, d) = (b + n * 0.5, a + n * 0.5);
    let mut doc = Document::new();
    let plane = Plane::from_polygon(&[a, b, c]).unwrap();
    let s = doc.add_sketch(plane);
    for (p, q) in [(a, b), (b, c), (c, d), (d, a)] {
        doc.sketch_mut(s).unwrap().add_segment(p, q).unwrap();
    }
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let h = if plane.normal().z > 0.0 {
        2.176461726410028
    } else {
        -2.176461726410028
    };
    let (o, _) = doc.extrude_region(s, r, h).unwrap();
    let obj = doc.object(o).unwrap();
    let face = obj
        .faces()
        .keys()
        .find(|&f| {
            let ring: Vec<Point3> = obj.loop_positions(obj.faces()[f].outer_loop).collect();
            ring.len() == 4
                && ring.iter().any(|p| p.approx_eq(a, 1e-12))
                && ring.iter().any(|p| p.approx_eq(b, 1e-12))
                && ring.iter().any(|p| p.z.abs() < 1e-12)
        })
        .expect("the diagonal wall");
    let loop_path = vec![
        Point3::new(-3.9500682109076983, 5.424431731931435, -1.632346294807521),
        Point3::new(-3.9420231119179276, 5.432476830921205, -1.632346294807521),
        Point3::new(-3.9420231119179276, 5.432476830921205, -0.544115431602507),
        Point3::new(-3.9500682109076983, 5.424431731931435, -0.544115431602507),
    ];
    doc.apply_object_op(
        o,
        KernelOp::SplitFaceInner {
            face,
            loop_path,
            restore: None,
            curves: Vec::new(),
            edge_attrs: Vec::new(),
        },
    )
    .expect("the loop splits the wall");
    assert_eq!(doc.object(o).unwrap().faces()[face].inner_loops.len(), 1);
}
