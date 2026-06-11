//! Property-based tests (DEVELOPMENT.md rule 3): construct/validate roundtrips
//! over randomly generated closed solids.

use kernel::{Object, Point3, TopologyError, WatertightState};
use proptest::prelude::*;

/// Minimum |det| of the three edge vectors for a generated tetrahedron
/// (6x its volume). Generous bound keeping every face far from degenerate.
const MIN_TETRA_DET: f64 = 1.0;

type Soup = (Vec<Point3>, Vec<Vec<usize>>);

fn arb_point() -> impl Strategy<Value = Point3> {
    (-100.0..100.0f64, -100.0..100.0f64, -100.0..100.0f64)
        .prop_map(|(x, y, z)| Point3::new(x, y, z))
}

/// Positively-oriented tetrahedra with outward-wound faces.
fn arb_tetrahedron() -> impl Strategy<Value = Soup> {
    (arb_point(), arb_point(), arb_point(), arb_point()).prop_filter_map(
        "tetrahedron too close to degenerate",
        |(p0, p1, p2, p3)| {
            let det = (p1 - p0).cross(p2 - p0).dot(p3 - p0);
            if det.abs() < MIN_TETRA_DET {
                return None;
            }
            // A negative determinant means the vertex order is left-handed;
            // swapping two vertices flips it.
            let vertices = if det > 0.0 {
                vec![p0, p1, p2, p3]
            } else {
                vec![p0, p1, p3, p2]
            };
            let faces = vec![vec![0, 2, 1], vec![0, 3, 2], vec![0, 1, 3], vec![1, 2, 3]];
            Some((vertices, faces))
        },
    )
}

/// Axis-aligned boxes (exactly planar quad faces).
fn arb_box() -> impl Strategy<Value = Soup> {
    (
        (-100.0..100.0f64, -100.0..100.0f64, -100.0..100.0f64),
        (0.001..50.0f64, 0.001..50.0f64, 0.001..50.0f64),
    )
        .prop_map(|((x, y, z), (dx, dy, dz))| {
            let vertices = vec![
                Point3::new(x, y, z),
                Point3::new(x + dx, y, z),
                Point3::new(x + dx, y + dy, z),
                Point3::new(x, y + dy, z),
                Point3::new(x, y, z + dz),
                Point3::new(x + dx, y, z + dz),
                Point3::new(x + dx, y + dy, z + dz),
                Point3::new(x, y + dy, z + dz),
            ];
            let faces = vec![
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ];
            (vertices, faces)
        })
}

fn arb_solid() -> impl Strategy<Value = Soup> {
    prop_oneof![arb_tetrahedron(), arb_box()]
}

/// A solid plus an index selecting one of its faces.
fn arb_solid_with_face_index() -> impl Strategy<Value = (Soup, usize)> {
    arb_solid().prop_flat_map(|soup| {
        let face_count = soup.1.len();
        (Just(soup), 0..face_count)
    })
}

fn euler_characteristic(obj: &Object) -> i64 {
    obj.vertices().len() as i64 - obj.edges().len() as i64 + obj.faces().len() as i64
}

proptest! {
    #[test]
    fn closed_solids_build_valid_and_watertight(soup in arb_solid()) {
        let (vertices, faces) = soup;
        let obj = Object::from_polygons(&vertices, &faces).unwrap();
        prop_assert!(obj.validate().is_ok());
        prop_assert_eq!(obj.watertight(), WatertightState::Watertight);
        prop_assert_eq!(euler_characteristic(&obj), 2);
    }

    #[test]
    fn polygon_roundtrip_preserves_topology(soup in arb_solid()) {
        let (vertices, faces) = soup;
        let obj = Object::from_polygons(&vertices, &faces).unwrap();
        let (out_vertices, out_faces) = obj.to_polygons();
        let rebuilt = Object::from_polygons(&out_vertices, &out_faces).unwrap();
        prop_assert!(rebuilt.validate().is_ok());
        prop_assert_eq!(rebuilt.watertight(), obj.watertight());
        prop_assert_eq!(rebuilt.vertices().len(), obj.vertices().len());
        prop_assert_eq!(rebuilt.half_edges().len(), obj.half_edges().len());
        prop_assert_eq!(rebuilt.edges().len(), obj.edges().len());
        prop_assert_eq!(rebuilt.faces().len(), obj.faces().len());
    }

    #[test]
    fn dropping_a_face_opens_the_solid((soup, drop_at) in arb_solid_with_face_index()) {
        let (vertices, mut faces) = soup;
        faces.remove(drop_at);
        let obj = Object::from_polygons(&vertices, &faces).unwrap();
        prop_assert!(obj.validate().is_ok());
        prop_assert_eq!(obj.watertight(), WatertightState::Open);
    }

    #[test]
    fn duplicating_a_face_is_rejected_as_non_manifold(
        (soup, dup_at) in arb_solid_with_face_index()
    ) {
        let (vertices, mut faces) = soup;
        faces.push(faces[dup_at].clone());
        let err = Object::from_polygons(&vertices, &faces).unwrap_err();
        prop_assert!(matches!(err, TopologyError::NonManifoldEdge { .. }), "expected NonManifoldEdge, got {err:?}");
    }
}
