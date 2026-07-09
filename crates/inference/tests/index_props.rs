//! Property-based tests for the spatial index (DEVELOPMENT.md rule 3),
//! mirroring `crates/kernel/tests/props.rs`: over random scenes (solids
//! under random placements, plus guides, sketch and transient segments) and
//! random queries (rays, apertures, locks, constraint planes), the indexed
//! hot paths must return byte-for-byte the same results as the linear
//! reference scan (`resolve_linear` / `pick_face_linear`) — the index may
//! only prune, never decide.

use inference::{Axis, InferenceScene, PickRay, SnapLock, SnapQuery};
use kernel::{
    Guide, GuideId, InstanceId, Object, ObjectId, Plane, Point3, SketchId, Transform, Vec3,
};
use proptest::prelude::*;

/// Minimum |det| of the three edge vectors for a generated tetrahedron
/// (6x its volume), as in the kernel's own props: keeps every face far from
/// degenerate so `from_polygons` accepts it.
const MIN_TETRA_DET: f64 = 1.0;

/// Generation gate for usable ray directions (meters²). The engine itself
/// rejects degenerate directions via `normalized()`; this only keeps the
/// generator from wasting cases on them.
const MIN_RAY_LEN_SQ: f64 = 1e-6;

/// Aperture generation bounds, as log₁₀ of radians: log-uniform from
/// 1e-9 rad to 2.5 rad. The low decades reach the regime where the exact
/// cone test's `acos(depth/dist)` saturates (indexed/linear divergence
/// there needs aperture ≲ 3e-7 · depth — unreachable from the earlier
/// uniform 0.005..2.5 draw, which is fully contained in this range); the
/// top crosses FRAC_PI_2, so the "cone is the whole front half-space"
/// branch of the index still gets generated too.
const LOG10_APERTURE_MIN: f64 = -9.0;
const LOG10_APERTURE_MAX: f64 = 0.397_940_008_672_037_6; // log10(2.5)

fn arb_point(range: f64) -> impl Strategy<Value = Point3> {
    (-range..range, -range..range, -range..range).prop_map(|(x, y, z)| Point3::new(x, y, z))
}

/// A solid as polygon soup (positions + CCW face index loops), like the
/// kernel props' `Soup`.
type Soup = (Vec<Point3>, Vec<Vec<usize>>);

/// A solid as polygon soup: an axis-aligned box or a well-conditioned
/// tetrahedron (both accepted by `Object::from_polygons`).
fn arb_solid() -> impl Strategy<Value = Soup> {
    let boxes =
        (arb_point(20.0), (0.1..8.0f64, 0.1..8.0f64, 0.1..8.0f64)).prop_map(|(p, (dx, dy, dz))| {
            let (x, y, z) = (p.x, p.y, p.z);
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
        });
    let tetras = (
        arb_point(20.0),
        arb_point(20.0),
        arb_point(20.0),
        arb_point(20.0),
    )
        .prop_filter_map("tetrahedron too close to degenerate", |(p0, p1, p2, p3)| {
            let det = (p1 - p0).cross(p2 - p0).dot(p3 - p0);
            if det.abs() < MIN_TETRA_DET {
                return None;
            }
            let vertices = if det > 0.0 {
                vec![p0, p1, p2, p3]
            } else {
                vec![p0, p1, p3, p2]
            };
            let faces = vec![vec![0, 2, 1], vec![0, 3, 2], vec![0, 1, 3], vec![1, 2, 3]];
            Some((vertices, faces))
        });
    prop_oneof![boxes, tetras]
}

/// A placement: scale (possibly non-uniform, exercising the
/// inverse-transpose plane path), then a rotation about a model axis, then
/// a translation.
fn arb_placement() -> impl Strategy<Value = Transform> {
    (
        (0.25..3.0f64, 0.25..3.0f64, 0.25..3.0f64),
        prop_oneof![Just(Axis::X), Just(Axis::Y), Just(Axis::Z)],
        0.0..std::f64::consts::TAU,
        arb_point(40.0),
    )
        .prop_map(|((sx, sy, sz), axis, angle, t)| {
            Transform::scale(Vec3::new(sx, sy, sz))
                .then(&Transform::rotation(axis.unit(), angle).expect("model axis is unit"))
                .then(&Transform::translation(t.to_vec()))
        })
}

fn arb_guide() -> impl Strategy<Value = Guide> {
    prop_oneof![
        (
            arb_point(30.0),
            prop_oneof![Just(Axis::X), Just(Axis::Y), Just(Axis::Z)]
        )
            .prop_map(|(origin, axis)| Guide::Line {
                origin,
                direction: axis.unit(),
            }),
        arb_point(30.0).prop_map(|position| Guide::Point { position }),
    ]
}

fn arb_segments() -> impl Strategy<Value = Vec<(Point3, Point3)>> {
    prop::collection::vec((arb_point(30.0), arb_point(30.0)), 0..4)
}

#[derive(Debug, Clone)]
struct SceneSpec {
    solids: Vec<(Soup, Transform)>,
    guides: Vec<Guide>,
    sketch: Vec<(Point3, Point3)>,
    transient: Vec<(Point3, Point3)>,
}

fn arb_scene() -> impl Strategy<Value = SceneSpec> {
    (
        prop::collection::vec((arb_solid(), arb_placement()), 1..5),
        prop::collection::vec(arb_guide(), 0..3),
        arb_segments(),
        arb_segments(),
    )
        .prop_map(|(solids, guides, sketch, transient)| SceneSpec {
            solids,
            guides,
            sketch,
            transient,
        })
}

fn build_scene(spec: &SceneSpec) -> InferenceScene {
    let mut scene = InferenceScene::new();
    for (i, ((positions, faces), placement)) in spec.solids.iter().enumerate() {
        let object = Object::from_polygons(positions, faces).expect("generated solids are valid");
        if i == 0 {
            // One world object; the rest register additively as instances
            // (ids are opaque labels to the scene — see the specs header).
            scene.add_object(ObjectId::default(), &object, placement);
        } else {
            scene.add_instance(
                InstanceId::default(),
                ObjectId::default(),
                &object,
                placement,
            );
        }
    }
    for guide in &spec.guides {
        // Same id: `add_guide` has replace semantics, so only the last one
        // survives — enough to exercise the (linear) guide path.
        scene.add_guide(GuideId::default(), guide);
    }
    scene.add_sketch(SketchId::default(), &spec.sketch);
    for &(a, b) in &spec.transient {
        scene.add_transient_segment(a, b);
    }
    scene
}

#[derive(Debug, Clone)]
struct QuerySpec {
    origin: Point3,
    target: Point3,
    aperture: f64,
    anchor: Option<Point3>,
    lock: Option<SnapLock>,
    plane: Option<(Axis, f64)>,
}

fn arb_query() -> impl Strategy<Value = QuerySpec> {
    (
        arb_point(60.0),
        arb_point(40.0),
        // Log-uniform (see LOG10_APERTURE_MIN/MAX): tiny saturation-window
        // apertures through past-FRAC_PI_2 half-space cones.
        (LOG10_APERTURE_MIN..LOG10_APERTURE_MAX).prop_map(|e| 10f64.powf(e)),
        prop::option::of(arb_point(30.0)),
        prop::option::of(prop_oneof![
            Just(SnapLock::Axis(Axis::X)),
            Just(SnapLock::Axis(Axis::Y)),
            Just(SnapLock::Axis(Axis::Z)),
            Just(SnapLock::Direction(Vec3::new(1.0, 1.0, 0.0))),
        ]),
        prop::option::of((
            prop_oneof![Just(Axis::X), Just(Axis::Y), Just(Axis::Z)],
            -10.0..10.0f64,
        )),
    )
        .prop_filter_map(
            "ray direction too short",
            |(origin, target, aperture, anchor, lock, plane)| {
                if (target - origin).length_squared() < MIN_RAY_LEN_SQ {
                    return None;
                }
                Some(QuerySpec {
                    origin,
                    target,
                    aperture,
                    anchor,
                    lock,
                    plane,
                })
            },
        )
}

fn to_query(spec: &QuerySpec) -> SnapQuery {
    SnapQuery {
        ray: PickRay {
            origin: spec.origin,
            direction: spec.target - spec.origin,
        },
        anchor: spec.anchor,
        lock: spec.lock,
        aperture: spec.aperture,
        constraint_plane: spec.plane.map(|(axis, offset)| {
            Plane::from_point_normal(Point3::ORIGIN + axis.unit() * offset, axis.unit())
                .expect("model axis is unit")
        }),
    }
}

proptest! {
    /// The acceptance criterion for the spatial index: on any scene and any
    /// query, `resolve` (indexed) ≡ `resolve_linear` (full scan) and
    /// `pick_face` ≡ `pick_face_linear`, bit for bit.
    #[test]
    fn indexed_queries_equal_the_linear_reference(
        scene_spec in arb_scene(),
        query_spec in arb_query(),
    ) {
        let scene = build_scene(&scene_spec);
        let query = to_query(&query_spec);
        prop_assert_eq!(scene.resolve(&query), scene.resolve_linear(&query));
        prop_assert_eq!(
            scene.pick_face(&query.ray),
            scene.pick_face_linear(&query.ray)
        );
    }

    /// Equivalence must survive mutation + lazy rebuild: after removing the
    /// world object (index invalidated, then rebuilt on the next query),
    /// the indexed and linear answers still agree.
    #[test]
    fn equivalence_holds_after_invalidation(
        scene_spec in arb_scene(),
        query_spec in arb_query(),
    ) {
        let mut scene = build_scene(&scene_spec);
        let query = to_query(&query_spec);
        // Warm the index, mutate, then re-query: a stale index would show
        // up as a divergence from the linear reference.
        let _ = scene.resolve(&query);
        scene.remove_object(ObjectId::default());
        prop_assert_eq!(scene.resolve(&query), scene.resolve_linear(&query));
        prop_assert_eq!(
            scene.pick_face(&query.ray),
            scene.pick_face_linear(&query.ray)
        );
    }

    /// Equivalence must also survive idempotent removals on a warm index.
    /// Removing an id that may or may not be registered (single-solid specs
    /// register no instances, so the instance removal is a pure no-op there;
    /// the repeated object removal is a no-op always) must keep the indexed
    /// answers equal to the linear reference: a no-op removal changes no
    /// candidates and keeps the index, a real one invalidates it — getting
    /// either side wrong diverges here.
    #[test]
    fn equivalence_holds_after_idempotent_removal(
        scene_spec in arb_scene(),
        query_spec in arb_query(),
    ) {
        let mut scene = build_scene(&scene_spec);
        let query = to_query(&query_spec);
        let _ = scene.resolve(&query); // warm the index
        scene.remove_instance(InstanceId::default());
        prop_assert_eq!(scene.resolve(&query), scene.resolve_linear(&query));
        scene.remove_object(ObjectId::default());
        let _ = scene.resolve(&query); // warm again
        scene.remove_object(ObjectId::default()); // unknown now: no-op
        prop_assert_eq!(scene.resolve(&query), scene.resolve_linear(&query));
        prop_assert_eq!(
            scene.pick_face(&query.ray),
            scene.pick_face_linear(&query.ray)
        );
    }
}
