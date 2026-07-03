//! Regression: `split_face` with a multi-segment path whose BOTH endpoints
//! lie interior to the SAME boundary edge (a "lens" cut — Arc tool, V-shaped
//! Line chains). ep0's `split_boundary_edge` consumes the shared half-edge,
//! so ep1's stored key is dead; before the fix this panicked
//! ("invalid SlotMap key used", found by the on-face Arc E2E) instead
//! of splitting. ep1 now re-resolves against the current outer loop.

use kernel::{Object, Plane, Point3, Profile, Vec3, WatertightState};

fn box_2x2x1() -> Object {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let profile = Profile::new(
        plane,
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ],
        vec![],
    )
    .unwrap();
    Object::from_extrusion(&profile, 1.0).unwrap()
}

fn top_face(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().dot(Vec3::new(0.0, 0.0, 1.0)) > 0.9)
        .map(|(id, _)| id)
        .expect("top face")
}

fn assert_lens_split(path: &[Point3]) {
    let mut obj = box_2x2x1();
    let face = top_face(&obj);
    let faces_before = obj.faces().len();

    let report = obj
        .split_face(face, path)
        .expect("same-edge lens cut must split, not panic or refuse");
    obj.validate().unwrap();

    assert!(!report.new_faces.is_empty());
    assert_eq!(
        obj.faces().len(),
        faces_before + 1,
        "one face became two (lens + remainder)"
    );
    assert_eq!(
        obj.watertight(),
        WatertightState::Watertight,
        "a face split never opens the shell"
    );
}

/// Minimal shape: a 2-segment V whose endpoints are interior points of the
/// same boundary edge (y = 2) of the top face.
#[test]
fn split_face_v_path_endpoints_on_same_boundary_edge() {
    assert_lens_split(&[
        Point3::new(0.5, 2.0, 1.0),
        Point3::new(1.0, 1.6, 1.0),
        Point3::new(1.5, 2.0, 1.0),
    ]);
}

/// The Arc tool's actual shape: a faceted circular arc from (0.5, 2) to
/// (1.5, 2) bulging into the face (apex y = 1.6) — the exact path from the
/// E2E that uncovered the panic, at the same facet density.
#[test]
fn split_face_arc_path_endpoints_on_same_boundary_edge() {
    // Chord half-length h = 0.5, sagitta s = 0.4 → r = (h² + s²)/(2s) =
    // 0.5125; the minor-arc center sits OPPOSITE the bulge (above the
    // chord): (1.0, 2.0 + (r − s)) = (1.0, 2.1125). Apex (1.0, 1.6).
    let h = 0.5_f64;
    let s = 0.4_f64;
    let r = (h * h + s * s) / (2.0 * s);
    let cy = 2.0 + (r - s);
    let half_sweep = f64::atan2(h, r - s);
    let n = 12; // interior facets
    let mut path = vec![Point3::new(0.5, 2.0, 1.0)];
    for i in 1..n {
        // Sweep from A (left) to B (right) under the chord: angle measured
        // from +y (up) rotating left-to-right through the bulge apex.
        let a = -half_sweep + (2.0 * half_sweep * i as f64) / n as f64;
        path.push(Point3::new(1.0 + r * a.sin(), cy - r * a.cos(), 1.0));
    }
    path.push(Point3::new(1.5, 2.0, 1.0));
    assert_lens_split(&path);
}
