//! Executable specs for the flat push/pull path's fold guard (#93, KB-35).
//!
//! Translating a face's boundary ring in place reshapes every neighbor face
//! that shares an edge with it: the neighbor's boundary sweeps across the
//! region between where the ring was and where it goes. The flat path folds
//! a neighbor into a self-intersecting outline exactly when that swept
//! region holds a fixed part of the neighbor's boundary — a vertex, or an
//! edge spanning it. The contract under test:
//!
//! - the guard refuses only such a fold, not any neighbor vertex that
//!   happens to lie ahead of the face somewhere off to the side (a block on
//!   a cylinder pulled sideways, #93; a step's low top pushed down beside a
//!   slot in the high part, KB-35);
//! - a fold the flat path cannot make is routed to the boolean path:
//!   [`Object::push_pull_overshoots`] answers `true` for it, and
//!   [`Object::push_through`] builds the right solid (a slot that reaches
//!   the step severs the high part: two pieces);
//! - two footprints that only touch along an edge are not overlap
//!   (`footprints_overlap`'s own doc), so the slot reaching the step reaches
//!   the boolean path through the fold test, not through that contact.

use kernel::trig;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use kernel::{FaceId, Object, Plane, Point3, Profile, PushPullError, Transform, Vec3};
use proptest::prelude::*;

/// Signed volume of a closed object (divergence theorem over its faces).
fn volume(obj: &Object) -> f64 {
    let mut v = 0.0;
    for face in obj.faces().values() {
        let n = face.plane.normal();
        let mut area = 0.0;
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            let ring: Vec<Point3> = obj.loop_positions(l).collect();
            let mut acc = Vec3::new(0.0, 0.0, 0.0);
            for i in 0..ring.len() {
                let (a, b) = (ring[i].to_vec(), ring[(i + 1) % ring.len()].to_vec());
                acc = acc + a.cross(b);
            }
            area += acc.dot(n) * 0.5;
        }
        let p = obj
            .loop_positions(face.outer_loop)
            .next()
            .expect("a vertex");
        v += area * n.dot(p.to_vec()) / 3.0;
    }
    v
}

/// The face of `obj` with outward normal `n` whose plane passes through `p`.
fn face_at(obj: &Object, n: Vec3, p: Point3) -> FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().approx_eq(n, 1e-9) && f.plane.signed_distance(p).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .expect("face exists")
}

/// A prism of `outer` (a CCW ring about its own normal) swept `depth` along
/// that normal.
fn prism(outer: Vec<Point3>, depth: f64) -> Object {
    let plane = Plane::from_polygon(&outer).expect("plane");
    let profile = Profile::new(plane, outer, vec![]).expect("profile");
    Object::from_extrusion(&profile, depth).expect("extrude")
}

/// What the app does with a push/pull: the boolean path when the flat one
/// overshoots, the flat path otherwise. Returns the resulting solids.
fn push(obj: &Object, face: FaceId, distance: f64) -> Result<Vec<Object>, PushPullError> {
    if obj.push_pull_overshoots(face, distance) {
        Ok(obj
            .push_through(face, distance)?
            .split_connected_components())
    } else {
        let mut o = obj.clone();
        o.push_pull(face, distance)?;
        Ok(vec![o])
    }
}

// ------------------------------------------------------------------- #93

/// A 30-gon "cylinder" (radius 0.049, 50 mm tall) with a 16 x 12 mm block
/// 20 mm tall on its top, its corners 10 mm from the axis. The 30-gon's
/// phase puts one rim vertex 22 µm ahead of the block's +x wall, far off to
/// the side (at y = 48 mm), as in the reported file.
fn block_on_cylinder() -> (Object, FaceId) {
    const R: f64 = 0.049;
    const XW: f64 = 0.008;
    let phase = trig::acos((XW + 2.2e-5) / R);
    let rim: Vec<Point3> = (0..30)
        .map(|i| {
            let a = phase + TAU * f64::from(i) / 30.0;
            Point3::new(R * trig::cos(a), R * trig::sin(a), 0.0)
        })
        .collect();
    let mut obj = prism(rim, 0.05);
    let top = face_at(&obj, Vec3::new(0.0, 0.0, 1.0), Point3::new(0.0, 0.0, 0.05));
    let block = [
        Point3::new(-XW, -0.006, 0.05),
        Point3::new(XW, -0.006, 0.05),
        Point3::new(XW, 0.006, 0.05),
        Point3::new(-XW, 0.006, 0.05),
    ];
    let sub = obj.split_face_inner(top, &block).expect("imprint").sub_face;
    obj.extrude_sub_face(sub, 0.02).expect("raise the block");
    let wall = face_at(&obj, Vec3::new(1.0, 0.0, 0.0), Point3::new(XW, 0.0, 0.06));
    (obj, wall)
}

#[test]
fn pulling_a_blocks_wall_on_a_cylinder_is_not_refused() {
    let (obj, wall) = block_on_cylinder();
    let before = volume(&obj);
    assert!(
        !obj.push_pull_overshoots(wall, 0.005),
        "nothing lies across the block's +x wall"
    );
    let mut pulled = obj.clone();
    pulled
        .push_pull(wall, 0.005)
        .expect("a 5 mm pull of the block's wall: nothing is in the way");
    pulled.validate().expect("valid");
    // The wall moved 5 mm; the block grew by its 12 x 20 mm side times 5 mm.
    face_at(
        &pulled,
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(0.013, 0.0, 0.06),
    );
    assert!((volume(&pulled) - before - 0.012 * 0.02 * 0.005).abs() < 1e-12);
    // And back: the inverse push restores it.
    let wall = face_at(
        &pulled,
        Vec3::new(1.0, 0.0, 0.0),
        Point3::new(0.013, 0.0, 0.06),
    );
    pulled.push_pull(wall, -0.005).expect("push it back");
    assert!((volume(&pulled) - before).abs() < 1e-12);
}

/// Pushed back toward the axis, the block's wall passes rim vertices that
/// lie behind it (nearer the axis than the wall, far off to the side): they
/// are beside the strip its bottom edge sweeps, so they fold nothing.
#[test]
fn pushing_a_blocks_wall_back_toward_the_axis_is_not_refused() {
    let (obj, wall) = block_on_cylinder();
    let mut pushed = obj.clone();
    pushed
        .push_pull(wall, -0.012)
        .expect("a 12 mm push of the block's wall toward the axis");
    pushed.validate().expect("valid");
}

/// The reported file pulled "sometimes": the same block, any of its five
/// faces, any distance either way. Whether a pull works must not depend on
/// which face the pointer found or how far it went, so every one of them
/// builds a valid solid (or, past the far side, routes to the boolean path).
#[test]
fn every_face_of_a_block_on_a_cylinder_pulls_at_every_distance() {
    let (obj, _) = block_on_cylinder();
    let z = 0.06;
    let faces = [
        (Vec3::new(1.0, 0.0, 0.0), Point3::new(0.008, 0.0, z)),
        (Vec3::new(-1.0, 0.0, 0.0), Point3::new(-0.008, 0.0, z)),
        (Vec3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.006, z)),
        (Vec3::new(0.0, -1.0, 0.0), Point3::new(0.0, -0.006, z)),
        (Vec3::new(0.0, 0.0, 1.0), Point3::new(0.0, 0.0, 0.07)),
    ];
    for (n, p) in faces {
        let face = face_at(&obj, n, p);
        for step in -240..=240 {
            let d = f64::from(step) * 0.00005;
            if d.abs() < 1e-4 {
                continue;
            }
            let pieces = push(&obj, face, d)
                .unwrap_or_else(|e| panic!("face {n:?} pulled {d}: refused with {e:?}"));
            for piece in &pieces {
                piece
                    .validate()
                    .unwrap_or_else(|e| panic!("face {n:?} pulled {d}: invalid {e:?}"));
            }
        }
    }
}

// ---------------------------------------------------------------- KB-35

/// The KB-35 step, in a frame rotated by `turn` about +x: a 1 m deep solid
/// whose side profile is a high part (x 0..1, 2 m tall) and a low part
/// (x 1..2, 1 m tall), with a slot through the high part at z 0.3..0.6 open
/// at x = 0 and running to `slot_end`. Returns the solid, its low top, and
/// the rotation (to read results back in the step's own frame).
fn step_with_slot(turn: f64, slot_end: f64) -> (Object, FaceId, Transform) {
    let rot = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), turn).expect("rotation");
    let at = |x: f64, z: f64| rot.apply_point(Point3::new(x, 0.0, z));
    // The extrusion runs along the outline's own normal (its winding),
    // 1 m deep.
    let outline = vec![
        at(0.0, 0.0),
        at(0.0, 0.3),
        at(slot_end, 0.3),
        at(slot_end, 0.6),
        at(0.0, 0.6),
        at(0.0, 2.0),
        at(1.0, 2.0),
        at(1.0, 1.0),
        at(2.0, 1.0),
        at(2.0, 0.0),
    ];
    let obj = prism(outline, 1.0);
    let up = rot.apply_vector(Vec3::new(0.0, 0.0, 1.0));
    // The low top: normal along the step's +z, through (1.5, *, 1).
    let low_top = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().approx_eq(up, 1e-9)
                && f.plane.signed_distance(at(1.5, 1.0)).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .expect("the low top");
    (obj, low_top, rot)
}

/// KB-35: the slot ends 0.1 m short of the step, so pushing the low top
/// down 0.5 m folds nothing (the slot's corners lie beside the swept
/// strip); it takes the flat path in every orientation and lands the top
/// at z = 0.5.
#[test]
fn pushing_a_steps_low_top_down_beside_a_slot_is_not_refused() {
    for turn in [0.0, FRAC_PI_2, PI, 3.0 * FRAC_PI_2] {
        let (obj, low_top, rot) = step_with_slot(turn, 0.9);
        let before = volume(&obj);
        assert!(
            !obj.push_pull_overshoots(low_top, -0.5),
            "turn {turn}: nothing to cut through"
        );
        let mut pushed = obj.clone();
        pushed
            .push_pull(low_top, -0.5)
            .unwrap_or_else(|e| panic!("turn {turn}: the push is refused: {e:?}"));
        pushed.validate().expect("valid");
        assert!(
            (volume(&pushed) - (before - 0.5)).abs() < 1e-9,
            "turn {turn}: the low part lost 1 x 1 x 0.5"
        );
        let up = rot.apply_vector(Vec3::new(0.0, 0.0, 1.0));
        face_at(&pushed, up, rot.apply_point(Point3::new(1.5, 0.5, 0.5)));
    }
}

/// The same push with the slot reaching the step: the low top would sink
/// past the slot's ceiling, folding the side faces' outlines over the
/// slot's end. That is a real fold, so the push routes to the boolean path,
/// whose subtract severs the high part above the slot: two solids.
#[test]
fn pushing_a_steps_low_top_past_a_slot_that_reaches_it_cuts_through() {
    for turn in [0.0, FRAC_PI_2, PI, 3.0 * FRAC_PI_2] {
        let (obj, low_top, _) = step_with_slot(turn, 1.0);
        let before = volume(&obj);
        // The flat path alone refuses it: the guard is the backstop.
        assert_eq!(
            obj.clone().push_pull(low_top, -0.5).unwrap_err(),
            PushPullError::NonManifoldResult,
            "turn {turn}"
        );
        assert!(
            obj.push_pull_overshoots(low_top, -0.5),
            "turn {turn}: the fold routes to the boolean path"
        );
        let pieces = push(&obj, low_top, -0.5).expect("cut through");
        assert_eq!(pieces.len(), 2, "turn {turn}: the high part is severed");
        let total: f64 = pieces.iter().map(volume).sum();
        assert!((total - (before - 0.5)).abs() < 1e-9, "turn {turn}");
        // Short of the slot's ceiling it is still the flat path.
        assert!(!obj.push_pull_overshoots(low_top, -0.3), "turn {turn}");
        let mut pushed = obj.clone();
        pushed
            .push_pull(low_top, -0.3)
            .expect("a push short of the slot");
    }
}

/// The guard still refuses what really folds on the flat path: a step's
/// low top pulled up past the high top would turn the riser inside out.
/// That routes to the union, which grows the low part over the step.
#[test]
fn pulling_a_steps_low_top_past_the_high_top_still_folds() {
    let (obj, low_top, _) = step_with_slot(0.0, 0.9);
    assert_eq!(
        obj.clone().push_pull(low_top, 1.5).unwrap_err(),
        PushPullError::NonManifoldResult
    );
    // Short of the high top, the flat path pulls.
    let mut pulled = obj.clone();
    pulled
        .push_pull(low_top, 0.5)
        .expect("a pull short of the high top");
}

// ------------------------------------------------------------- property

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Over steps with a slot of random height and reach, in any of four
    /// orientations, any push of the low top short of the floor goes
    /// through the route the app takes and never refuses; when it takes the
    /// flat path the result is one valid solid whose volume is the boolean
    /// subtract's (the flat path folding the slot open would have kept one
    /// solid where the subtract cuts two).
    #[test]
    fn a_step_push_short_of_the_floor_always_lands(
        turn in 0usize..4,
        z0 in 0.05f64..0.8,
        h in 0.05f64..0.15,
        reaches in any::<bool>(),
        short in 0.02f64..0.3,
        depth in 0.05f64..0.95,
    ) {
        let z1 = z0 + h;
        // A push landing on the slot's ceiling or floor exactly is a
        // touching contact either path may refuse; stay clear of it.
        prop_assume!(((1.0 - depth) - z1).abs() > 1e-3 && ((1.0 - depth) - z0).abs() > 1e-3);
        let slot_end = if reaches { 1.0 } else { 1.0 - short };
        let rot = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), turn as f64 * FRAC_PI_2).unwrap();
        let at = |x: f64, z: f64| rot.apply_point(Point3::new(x, 0.0, z));
        let outline = vec![
            at(0.0, 0.0), at(0.0, z0), at(slot_end, z0), at(slot_end, z1), at(0.0, z1),
            at(0.0, 2.0), at(1.0, 2.0), at(1.0, 1.0), at(2.0, 1.0), at(2.0, 0.0),
        ];
        let obj = prism(outline, 1.0);
        let up = rot.apply_vector(Vec3::new(0.0, 0.0, 1.0));
        let low_top = obj.faces().iter()
            .find(|(_, f)| f.plane.normal().approx_eq(up, 1e-9)
                && f.plane.signed_distance(at(1.5, 1.0)).abs() < 1e-9)
            .map(|(id, _)| id).unwrap();
        let before = volume(&obj);

        let pieces = push(&obj, low_top, -depth);
        prop_assert!(pieces.is_ok(), "refused: {:?}", pieces.err());
        let pieces = pieces.unwrap();
        let total: f64 = pieces.iter().map(volume).sum();
        prop_assert!((total - (before - depth)).abs() < 1e-9);
        for p in &pieces {
            prop_assert!(p.validate().is_ok());
        }
        if !obj.push_pull_overshoots(low_top, -depth) {
            let cut = obj.push_through(low_top, -depth).unwrap().split_connected_components();
            prop_assert_eq!(cut.len(), 1, "the flat path kept one solid where the cut makes {}", cut.len());
        }
    }
}

// ------------------------------------------------- swing triangle (v1.5 D2)

/// A box `[0,2] x [0,1] x [0,1]` whose front wall (y = 0) carries an
/// imprinted diagonal from its top-left corner `(0,0,1)` down to
/// `(1,0,0)`, and a second imprint: a small square at `square` (x, z
/// ranges) on the wall. Returns the box and its top.
fn box_with_diagonal(square: ([f64; 2], [f64; 2])) -> (Object, FaceId) {
    let mut obj = prism(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
        1.0,
    );
    let front = face_at(&obj, Vec3::new(0.0, -1.0, 0.0), Point3::new(1.0, 0.0, 0.5));
    let ([x0, x1], [z0, z1]) = square;
    obj.split_face_inner(
        front,
        &[
            Point3::new(x0, 0.0, z0),
            Point3::new(x0, 0.0, z1),
            Point3::new(x1, 0.0, z1),
            Point3::new(x1, 0.0, z0),
        ],
    )
    .expect("the second imprint");
    obj.split_face(
        front,
        &[Point3::new(0.0, 0.0, 1.0), Point3::new(1.0, 0.0, 0.0)],
    )
    .expect("the diagonal");
    obj.validate().expect("valid");
    let top = face_at(&obj, Vec3::new(0.0, 0.0, 1.0), Point3::new(1.0, 0.5, 1.0));
    (obj, top)
}

/// Everything about an object a refused push must leave as it was.
fn snapshot(obj: &Object) -> String {
    format!("{obj:?}")
}

/// The four assertions of a fold the flat path cannot make, for a push of
/// `top` by `long` (it folds) and by `short` (it does not).
fn assert_fold_routes_to_the_boolean(obj: &Object, top: FaceId, long: f64, short: f64, what: &str) {
    let before = volume(obj);
    let area = 2.0; // the box's top
    assert!(
        obj.push_pull_overshoots(top, long),
        "{what}: the long push folds, so it routes to the boolean"
    );
    assert!(
        !obj.push_pull_overshoots(top, short),
        "{what}: the short push stays flat"
    );
    let pieces = push(obj, top, long).unwrap_or_else(|e| panic!("{what}: refused: {e:?}"));
    for p in &pieces {
        p.validate().expect("valid");
    }
    let total: f64 = pieces.iter().map(volume).sum();
    assert!(
        (total - (before + area * long)).abs() < 1e-9,
        "{what}: volume {total}, expected {}",
        before + area * long
    );
    let mut bare = obj.clone();
    let snap = snapshot(&bare);
    assert_eq!(
        bare.push_pull(top, long).unwrap_err(),
        PushPullError::NonManifoldResult,
        "{what}: the flat path alone refuses"
    );
    assert_eq!(snapshot(&bare), snap, "{what}: the object is untouched");
    let mut flat = obj.clone();
    flat.push_pull(top, short).expect("the short push");
    flat.validate().expect("valid");
    assert!(
        (volume(&flat) - (before + area * short)).abs() < 1e-9,
        "{what}"
    );
}

/// The swing triangle, pulled out: the top's corner `(0,0,1)` rises and the
/// diagonal swings up into the wall face that shares the top's front edge,
/// sweeping the square imprinted there.
#[test]
fn a_diagonal_swinging_up_over_an_imprint_routes_to_the_boolean() {
    // Above the diagonal (z > 1 - x), inside the triangle a 1 m pull sweeps.
    let (obj, top) = box_with_diagonal(([0.3, 0.4], [0.8, 0.9]));
    assert_fold_routes_to_the_boolean(&obj, top, 1.0, 0.05, "pull");
}

/// The swing triangle, pushed in: the corner sinks and the diagonal swings
/// down into the wall face below it, which meets the top at that corner
/// only, sweeping the square imprinted there.
#[test]
fn a_diagonal_swinging_down_over_an_imprint_routes_to_the_boolean() {
    // Below the diagonal (z < 1 - x), inside the triangle a 0.6 m push
    // sweeps.
    let (obj, top) = box_with_diagonal(([0.1, 0.2], [0.55, 0.65]));
    assert_fold_routes_to_the_boolean(&obj, top, -0.6, -0.05, "push");
}

// ------------------------------------------------- spanning edge (v1.5 D2)

/// A step (high part x 0..1, 2 m; low part x 1..2, 1 m; 1 m deep along +y)
/// whose front side face (y = 0) carries an imprinted triangle with one
/// edge from `(0.9, 0.95)` to `(1.5, 0.3)` (x, z): across the strip the low
/// top's front edge sweeps on a push down, its two ends outside it (one
/// beside the strip, under the high part; one below it). No vertex of the
/// side face is in the strip.
fn step_with_spanning_imprint() -> (Object, FaceId) {
    let at = |x: f64, z: f64| Point3::new(x, 0.0, z);
    let mut obj = prism(
        vec![
            at(0.0, 0.0),
            at(0.0, 2.0),
            at(1.0, 2.0),
            at(1.0, 1.0),
            at(2.0, 1.0),
            at(2.0, 0.0),
        ],
        1.0,
    );
    let side = face_at(&obj, Vec3::new(0.0, -1.0, 0.0), at(0.5, 0.5));
    obj.split_face_inner(side, &[at(0.9, 0.95), at(0.8, 0.3), at(1.5, 0.3)])
        .expect("the imprint");
    obj.validate().expect("valid");
    let low_top = face_at(&obj, Vec3::new(0.0, 0.0, 1.0), Point3::new(1.5, 0.5, 1.0));
    (obj, low_top)
}

#[test]
fn an_edge_spanning_the_swept_strip_routes_to_the_boolean() {
    let (obj, low_top) = step_with_spanning_imprint();
    // The edge reaches the strip at x = 1, z ~ 0.84: a 0.5 m push crosses
    // it, a 0.1 m one does not. The low top is 1 x 1 m.
    let before = volume(&obj);
    assert!(obj.push_pull_overshoots(low_top, -0.5));
    assert!(!obj.push_pull_overshoots(low_top, -0.1));
    let pieces = push(&obj, low_top, -0.5).expect("cut down");
    for p in &pieces {
        p.validate().expect("valid");
    }
    let total: f64 = pieces.iter().map(volume).sum();
    assert!((total - (before - 0.5)).abs() < 1e-9, "volume {total}");
    let mut bare = obj.clone();
    let snap = snapshot(&bare);
    assert_eq!(
        bare.push_pull(low_top, -0.5).unwrap_err(),
        PushPullError::NonManifoldResult
    );
    assert_eq!(snapshot(&bare), snap, "the object is untouched");
    let mut flat = obj.clone();
    flat.push_pull(low_top, -0.1).expect("the short push");
    flat.validate().expect("valid");
    assert!((volume(&flat) - (before - 0.1)).abs() < 1e-9);
}

// ------------------------------------------- the two predicates agree (D2)

/// Whether the closed 2D polygon `ring` is simple: no two non-adjacent
/// edges meet.
fn ring_is_simple(ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let meets = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
        let (d1, d2) = (cross(c, d, a), cross(c, d, b));
        let (d3, d4) = (cross(a, b, c), cross(a, b, d));
        let eps = 1e-12;
        ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
            && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    };
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if meets(ring[i], ring[(i + 1) % n], ring[j], ring[(j + 1) % n]) {
                return false;
            }
        }
    }
    true
}

/// Every face outline of `obj` is simple in its plane, and every face it
/// shares (by id) with `before` faces the way it did: no outline crossed
/// itself or turned inside out.
fn outlines_simple_and_facing(obj: &Object, before: &Object) -> Result<(), String> {
    for (id, f) in obj.faces().iter() {
        let n = f.plane.normal();
        let helper = if n.x.abs() < 0.9 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let u = n.cross(helper);
        let u = u * (1.0 / u.length());
        let v = n.cross(u);
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let ring: Vec<[f64; 2]> = obj
                .loop_positions(l)
                .map(|p| [p.to_vec().dot(u), p.to_vec().dot(v)])
                .collect();
            if !ring_is_simple(&ring) {
                return Err(format!("face {id:?}: an outline crosses itself"));
            }
        }
        if let Some(g) = before.faces().get(id)
            && g.plane.normal().dot(n) <= 0.0
        {
            return Err(format!("face {id:?} turned inside out"));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Over random stepped prisms in four orientations, with random
    /// imprinted triangles on a side face and, sometimes, an imprinted
    /// diagonal from a corner of a top: whenever `push_pull_overshoots` is
    /// false, the flat path does not refuse `NonManifoldResult` (the fold
    /// guard and `flat_translation_folds` agree), and when it pushes, the
    /// result validates, its volume is `before + area * distance`, and
    /// every face outline is simple and faces as it did.
    #[test]
    fn the_flat_path_and_the_overshoot_test_agree(
        turn in 0usize..4,
        hl in 0.3f64..1.5,
        hh in 0.3f64..2.0,
        xs in 0.4f64..1.6,
        tris in proptest::collection::vec(
            ((0.02f64..0.98, 0.02f64..0.98), (0.02f64..0.98, 0.02f64..0.98), (0.02f64..0.98, 0.02f64..0.98)),
            0..3,
        ),
        nick in proptest::option::of((0.02f64..0.9, 0.05f64..0.9)),
        diagonal in proptest::option::of(0.05f64..0.95),
        high in any::<bool>(),
        frac in -0.98f64..1.5,
    ) {
        prop_assume!((hl - hh).abs() > 0.05);
        let w = 2.0;
        let rot = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), turn as f64 * FRAC_PI_2).unwrap();
        let at = |x: f64, z: f64| rot.apply_point(Point3::new(x, 0.0, z));
        let outline = vec![
            at(0.0, 0.0), at(0.0, hh), at(xs, hh), at(xs, hl), at(w, hl), at(w, 0.0),
        ];
        let mut obj = prism(outline, 1.0);
        let side_n = rot.apply_vector(Vec3::new(0.0, -1.0, 0.0));
        let side_face = |obj: &Object, x: f64, z: f64| {
            obj.faces().iter().find(|(_, f)| {
                f.plane.normal().approx_eq(side_n, 1e-9) && f.plane.signed_distance(at(x, z)).abs() < 1e-9
            }).map(|(id, _)| id)
        };
        // Imprinted triangles, each in the unit cell under the lower top
        // scaled to the part it lands on (refused ones are skipped).
        let top_at = |x: f64| if x < xs { hh } else { hl };
        for ((ax, az), (bx, bz), (cx, cz)) in tris {
            let pt = |fx: f64, fz: f64| {
                let x = fx * w;
                (x, fz * top_at(x).min(top_at(x)) * 0.98)
            };
            let (a, b, c) = (pt(ax, az), pt(bx, bz), pt(cx, cz));
            let area2 = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if area2.abs() < 1e-3 {
                continue;
            }
            let Some(f) = side_face(&obj, a.0, a.1) else { continue };
            let ring = [at(a.0, a.1), at(b.0, b.1), at(c.0, c.1)];
            let mut trial = obj.clone();
            if trial.split_face_inner(f, &ring).is_ok() && trial.validate().is_ok() {
                obj = trial;
            }
        }
        // A small square under the low part's far top corner, where a
        // diagonal from that corner swings (on either side of it).
        if let Some((nx, nz)) = nick {
            let (x, z) = (w - nx * (w - xs), hl * (1.0 - nz));
            let e = 0.02;
            if let Some(f) = side_face(&obj, x, z) {
                let ring = [at(x - e, z - e), at(x + e, z - e), at(x + e, z + e), at(x - e, z + e)];
                let mut trial = obj.clone();
                if x + e < w && z - e > 0.0 && trial.split_face_inner(f, &ring).is_ok() && trial.validate().is_ok() {
                    obj = trial;
                }
            }
        }
        // A diagonal from the far corner of the low part's top down to the
        // floor (the swing-triangle setting).
        if let Some(u) = diagonal {
            let (x_top, z_top) = (w, hl);
            if let Some(f) = obj.faces().iter().find(|(_, f)| {
                f.plane.normal().approx_eq(side_n, 1e-9)
                    && obj.loop_positions(f.outer_loop).any(|p| p.approx_eq(at(x_top, z_top), 1e-9))
            }).map(|(id, _)| id) {
                let mut trial = obj.clone();
                if trial.split_face(f, &[at(x_top, z_top), at(u * w, 0.0)]).is_ok()
                    && trial.validate().is_ok()
                {
                    obj = trial;
                }
            }
        }
        let up = rot.apply_vector(Vec3::new(0.0, 0.0, 1.0));
        let (top_x, top_z, part_w) = if high { (xs / 2.0, hh, xs) } else { ((xs + w) / 2.0, hl, w - xs) };
        let face = obj.faces().iter().find(|(_, f)| {
            f.plane.normal().approx_eq(up, 1e-9) && f.plane.signed_distance(at(top_x, top_z)).abs() < 1e-9
        }).map(|(id, _)| id).unwrap();
        let distance = frac * top_z;
        prop_assume!(distance.abs() > 0.01);
        // A push landing exactly on the other top is a touching contact.
        let other = if high { hl } else { hh };
        prop_assume!((top_z + distance - other).abs() > 1e-3);

        if !obj.push_pull_overshoots(face, distance) {
            let before = volume(&obj);
            let mut flat = obj.clone();
            match flat.push_pull(face, distance) {
                Err(PushPullError::NonManifoldResult) => {
                    prop_assert!(false, "overshoots is false but the flat path folds");
                }
                Err(_) => {}
                Ok(_) => {
                    prop_assert!(flat.validate().is_ok());
                    let expected = before + part_w * 1.0 * distance;
                    prop_assert!((volume(&flat) - expected).abs() < 1e-9,
                        "volume {} expected {}", volume(&flat), expected);
                    let simple = outlines_simple_and_facing(&flat, &obj);
                    prop_assert!(simple.is_ok(), "{:?}", simple.err());
                }
            }
        }
    }
}
