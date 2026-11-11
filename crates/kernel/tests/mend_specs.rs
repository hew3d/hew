//! Executable specs for Mend (the mesh-repair design §8): merge nearby
//! points, separate pinched parts and fill small gaps on one open object
//! as one explicit, undoable repair.

use kernel::health::{Finding, RimRole, Verdict, diagnose};
use kernel::repair::{FillScope, MendPlan, RepairError};
use kernel::{
    DefRecipe, Document, DocumentError, ImportNode, ImportScene, MeshRecipe, NodeId, Object,
    ObjectId, Point3, Transform, Vec3, WatertightState,
};
use proptest::prelude::*;

// ───────────────────────────────────────────────────────────── helpers

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// An imported-tolerance object (the meshes Mend is for); points no face
/// uses are dropped.
fn imported(positions: &[Point3], faces: &[Vec<usize>]) -> Object {
    let mut map = vec![usize::MAX; positions.len()];
    let mut pos = Vec::new();
    let faces: Vec<Vec<usize>> = faces
        .iter()
        .map(|f| {
            f.iter()
                .map(|&v| {
                    if map[v] == usize::MAX {
                        map[v] = pos.len();
                        pos.push(positions[v]);
                    }
                    map[v]
                })
                .collect()
        })
        .collect();
    Object::from_polygons_import_for_test(&pos, &faces).expect("the fixture builds")
}

fn plan(weld: Option<f64>, split: bool, fill: Option<FillScope>) -> MendPlan {
    MendPlan {
        weld,
        folds: false,
        split,
        fill,
        require_solid: false,
    }
}

fn volume(o: &Object) -> f64 {
    let mut six = 0.0;
    for f in o.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let q: Vec<Vec3> = o.loop_positions(l).map(|x| x.to_vec()).collect();
            for i in 1..q.len().saturating_sub(1) {
                six += q[0].dot(q[i].cross(q[i + 1]));
            }
        }
    }
    six / 6.0
}

fn surface_area(o: &Object) -> f64 {
    o.faces()
        .values()
        .map(|f| {
            let q: Vec<Point3> = o.loop_positions(f.outer_loop).collect();
            let mut n = Vec3::ZERO;
            for i in 1..q.len() - 1 {
                n = n + (q[i] - q[0]).cross(q[i + 1] - q[0]);
            }
            0.5 * n.length()
        })
        .sum()
}

fn closed(o: &Object) -> bool {
    o.watertight() == WatertightState::Watertight
}

fn branching(o: &Object) -> usize {
    diagnose(o)
        .findings
        .iter()
        .filter(|f| matches!(f, Finding::BranchingBoundaryVertex { .. }))
        .count()
}

/// An id-free fingerprint: every face as its rings' positions (quantized
/// to a nanometer, rotated to the lowest point) and its normal, sorted.
type FacePrint = (Vec<Vec<[i64; 3]>>, [i64; 3]);

/// A mesh with holes: positions, outer rings, each face's hole rings.
type HoledMesh = (Vec<Point3>, Vec<Vec<usize>>, Vec<Vec<Vec<usize>>>);

fn fingerprint(o: &Object) -> Vec<FacePrint> {
    let q = |v: f64| (v * 1e9).round() as i64;
    let mut out: Vec<_> = o
        .faces()
        .values()
        .map(|f| {
            let ring = |l| -> Vec<[i64; 3]> {
                let mut r: Vec<[i64; 3]> = o
                    .loop_positions(l)
                    .map(|x| [q(x.x), q(x.y), q(x.z)])
                    .collect();
                let k = (0..r.len()).min_by_key(|&i| r[i]).unwrap();
                r.rotate_left(k);
                r
            };
            let mut rings = vec![ring(f.outer_loop)];
            let mut holes: Vec<_> = f.inner_loops.iter().map(|&l| ring(l)).collect();
            holes.sort();
            rings.extend(holes);
            let n = f.plane.normal();
            (rings, [q(n.x), q(n.y), q(n.z)])
        })
        .collect();
    out.sort();
    out
}

/// The eight corners of an axis box from `lo` to `hi`.
fn corners(lo: Point3, hi: Point3) -> Vec<Point3> {
    vec![
        p(lo.x, lo.y, lo.z),
        p(hi.x, lo.y, lo.z),
        p(hi.x, hi.y, lo.z),
        p(lo.x, hi.y, lo.z),
        p(lo.x, lo.y, hi.z),
        p(hi.x, lo.y, hi.z),
        p(hi.x, hi.y, hi.z),
        p(lo.x, hi.y, hi.z),
    ]
}

/// The six outward faces of a box over corners 0..8.
fn box_faces() -> Vec<Vec<usize>> {
    vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
}

/// A box whose six faces each have private corners (24 points), every
/// copy of a corner moved by `jitter(face, corner)` — an STL that was
/// never welded.
fn split_corner_box(
    size: [f64; 3],
    rotate: &dyn Fn(Vec3) -> Vec3,
    jitter: &dyn Fn(usize, usize) -> Vec3,
) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let c = corners(p(0.0, 0.0, 0.0), p(size[0], size[1], size[2]));
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    for (fi, f) in box_faces().iter().enumerate() {
        let mut ring = Vec::new();
        for &v in f {
            ring.push(positions.len());
            positions.push(Point3::ORIGIN + rotate(c[v].to_vec() + jitter(fi, v)));
        }
        faces.push(ring);
    }
    (positions, faces)
}

fn rotation(yaw: f64, pitch: f64) -> impl Fn(Vec3) -> Vec3 {
    move |v: Vec3| {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let v = Vec3::new(v.x * cy - v.y * sy, v.x * sy + v.y * cy, v.z);
        Vec3::new(v.x, v.y * cp - v.z * sp, v.y * sp + v.z * cp)
    }
}

/// A unit direction from two angles.
fn dir(a: f64, b: f64) -> Vec3 {
    Vec3::new(a.cos() * b.cos(), a.sin() * b.cos(), b.sin())
}

/// A UV sphere of radius `r`: poles as single points, triangles only.
fn sphere(n_lat: usize, n_lon: usize, r: f64) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let mut pos = vec![p(0.0, 0.0, -r)];
    for i in 1..n_lat {
        let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * i as f64 / n_lat as f64;
        for j in 0..n_lon {
            let lon = std::f64::consts::TAU * j as f64 / n_lon as f64;
            pos.push(p(
                r * lat.cos() * lon.cos(),
                r * lat.cos() * lon.sin(),
                r * lat.sin(),
            ));
        }
    }
    pos.push(p(0.0, 0.0, r));
    let top = pos.len() - 1;
    let at = |i: usize, j: usize| 1 + (i - 1) * n_lon + j % n_lon;
    let mut faces = Vec::new();
    for j in 0..n_lon {
        faces.push(vec![0, at(1, j + 1), at(1, j)]);
    }
    for i in 1..n_lat - 1 {
        for j in 0..n_lon {
            faces.push(vec![at(i, j), at(i, j + 1), at(i + 1, j + 1)]);
            faces.push(vec![at(i, j), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    for j in 0..n_lon {
        faces.push(vec![top, at(n_lat - 1, j), at(n_lat - 1, j + 1)]);
    }
    (pos, faces)
}

fn centroid(pos: &[Point3], f: &[usize]) -> Vec3 {
    f.iter().fold(Vec3::ZERO, |s, &i| s + pos[i].to_vec()) * (1.0 / f.len() as f64)
}

/// The face rings of `o` by exact position bits, to tell new faces from
/// old.
fn face_keys(o: &Object) -> std::collections::BTreeSet<Vec<[u64; 3]>> {
    o.faces()
        .values()
        .map(|f| {
            let mut r: Vec<[u64; 3]> = o
                .loop_positions(f.outer_loop)
                .map(|x| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()])
                .collect();
            r.sort();
            r
        })
        .collect()
}

const D: f64 = 1e-5;

// ──────────────────────────────────────────────────────────── 1–5 weld

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Spec 1. Six quads with 24 private corners, each copy moved at most
    /// d/3, weld to 8 points: watertight, nothing moved past d, the volume
    /// the box's (exact without jitter; within surface × d with it).
    #[test]
    fn weld_closes_a_box_whose_corners_were_split(
        a in 0.01f64..10.0, b in 0.01f64..10.0, c in 0.01f64..10.0,
        yaw in 0.0f64..std::f64::consts::TAU, pitch in -1.5f64..1.5,
        jitter in 0.0f64..1.0, seed in 0u64..1000,
    ) {
        let rot = rotation(yaw, pitch);
        let shake = move |f: usize, v: usize| {
            let h = (seed as f64 + f as f64 * 7.3 + v as f64 * 1.9).sin();
            dir(h * 11.0, h * 5.0) * (jitter * D / 3.0)
        };
        let (pos, faces) = split_corner_box([a, b, c], &rot, &shake);
        let mut obj = imported(&pos, &faces);
        let (report, pieces) = obj.mend(&plan(Some(D), false, None)).unwrap();
        prop_assert!(pieces.is_empty());
        prop_assert!(closed(&obj));
        prop_assert_eq!(obj.vertices().len(), 8);
        prop_assert_eq!(report.welded_vertices, 16);
        prop_assert!(report.weld_max_move <= D);
        let v0 = a * b * c;
        let slack = if jitter == 0.0 { 1e-9 * v0 } else { surface_area(&obj) * D };
        prop_assert!((volume(&obj) - v0).abs() <= slack, "{} vs {}", volume(&obj), v0);
    }

    /// Spec 2. A corner whose three copies lie farther than d apart stays
    /// open; only the near copies count as welded.
    #[test]
    fn weld_leaves_a_gap_wider_than_the_distance(gap in 1.01f64..10.0) {
        let shake = move |f: usize, v: usize| {
            if v != 0 {
                return Vec3::ZERO;
            }
            // Corner 0 is on faces 0, 2 and 5: one copy each way.
            match f {
                0 => Vec3::new(gap * D, 0.0, 0.0),
                2 => Vec3::new(0.0, gap * D, 0.0),
                _ => Vec3::new(0.0, 0.0, gap * D),
            }
        };
        let (pos, faces) = split_corner_box([1.0, 1.0, 1.0], &|v| v, &shake);
        let mut obj = imported(&pos, &faces);
        let (report, _) = obj.mend(&plan(Some(D), false, None)).unwrap();
        prop_assert!(!closed(&obj));
        prop_assert_eq!(report.welded_vertices, 14);
        prop_assert_eq!(obj.vertices().len(), 10);
    }
}

/// Spec 3. A second Mend finds nothing; a chain of points 0.9 d apart
/// (each within d of the next, the whole far wider) never moves a point
/// farther than d, so no cluster is wider than 2 d.
#[test]
fn weld_is_idempotent_and_never_moves_a_point_past_the_cap() {
    let (pos, faces) = split_corner_box([1.0, 2.0, 3.0], &|v| v, &|_, _| Vec3::ZERO);
    let mut obj = imported(&pos, &faces);
    obj.mend(&plan(Some(D), false, None)).unwrap();
    assert_eq!(
        obj.mend(&plan(Some(D), false, None)).unwrap_err(),
        RepairError::NothingToMend
    );

    // A fan from an apex over a chain of 1000 points 0.9 d apart.
    let mut pos: Vec<Point3> = (0..1000).map(|i| p(i as f64 * 0.9 * D, 0.0, 0.0)).collect();
    pos.push(p(0.0045, 0.01, 0.0));
    let apex = pos.len() - 1;
    let faces: Vec<Vec<usize>> = (0..999).map(|i| vec![i, i + 1, apex]).collect();
    let mut obj = imported(&pos, &faces);
    let census = obj.mend_census(&plan(Some(D), false, None)).unwrap();
    assert!(census.weld_vertices > 0);
    let (report, _) = obj.mend(&plan(Some(D), false, None)).unwrap();
    assert!(report.weld_max_move <= D, "{}", report.weld_max_move);
    assert_eq!(report.welded_vertices, census.weld_vertices);
    // What is left of the chain: points more than d/2... every kept
    // point is a root, and roots are more than d apart from each other's
    // members — at least one root per 2 d of chain.
    let kept = obj.vertices().len() - 1;
    assert!(kept >= (999.0 * 0.9 / 2.0) as usize, "{kept}");
    assert!(obj.validate().is_ok());
}

/// Spec 4.
#[test]
fn weld_distance_out_of_range_refuses() {
    let (pos, faces) = split_corner_box([1.0, 1.0, 1.0], &|v| v, &|_, _| Vec3::ZERO);
    let obj = imported(&pos, &faces);
    for d in [0.0, f64::NAN, f64::INFINITY, -1e-6, 1e-9, 2e-3] {
        let mut o = obj.clone();
        assert_eq!(
            o.mend(&plan(Some(d), true, None)).unwrap_err(),
            RepairError::WeldDistanceOutOfRange,
            "{d}"
        );
    }
}

/// Spec 5. Two closed boxes with private corners on the face they share:
/// welding makes each shared edge an edge of four faces. Alone that
/// refuses; with the split, two watertight boxes come back.
#[test]
fn a_weld_that_folds_faces_refuses_unless_split_is_on() {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    pos.extend(corners(p(1.0, 0.0, 0.0), p(2.0, 1.0, 1.0)));
    let mut faces = box_faces();
    faces.extend(
        box_faces()
            .into_iter()
            .map(|f| f.into_iter().map(|i| i + 8).collect()),
    );
    let obj = imported(&pos, &faces);
    assert!(closed(&obj));
    let mut alone = obj.clone();
    assert_eq!(
        alone.mend(&plan(Some(D), false, None)).unwrap_err(),
        RepairError::WeldFoldsFaces
    );
    let mut with = obj.clone();
    let (report, pieces) = with.mend(&plan(Some(D), true, None)).unwrap();
    assert_eq!(report.welded_vertices, 4);
    assert_eq!(report.split_edges, 4);
    assert_eq!(pieces.len(), 1);
    assert!(closed(&with) && closed(&pieces[0]));
    assert!((volume(&with) - 1.0).abs() < 1e-12 && (volume(&pieces[0]) - 1.0).abs() < 1e-12);
}

/// Spec 17. A triangle the weld shrinks to two points is dropped and
/// counted; the box around it closes.
#[test]
fn degenerate_faces_a_weld_makes_are_dropped_and_counted() {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    // q on the front-top edge, d/2 from corner 5.
    pos.push(p(1.0 - D / 2.0, 0.0, 1.0));
    let q = 8;
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, q, 6, 7],
        vec![q, 5, 6],
        vec![0, 1, 5, q, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let mut obj = imported(&pos, &faces);
    assert!(closed(&obj));
    let (report, _) = obj.mend(&plan(Some(D), false, None)).unwrap();
    assert_eq!(report.welded_vertices, 1);
    assert_eq!(report.collapsed_faces, 1);
    assert_eq!(obj.faces().len(), 6);
    assert!(closed(&obj));
}

// ─────────────────────────────────────────────────────────── 6–7 split

/// `k` open cones (no base) sharing one apex, pointing apart.
fn cones(k: usize, segments: usize) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let mut pos = vec![p(0.0, 0.0, 0.0)];
    let mut faces = Vec::new();
    for c in 0..k {
        let axis = dir(
            std::f64::consts::TAU * c as f64 / k as f64,
            0.3 * c as f64 - 0.4,
        );
        let helper = if axis.z.abs() < 0.9 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let u = axis.cross(helper).normalized().unwrap();
        let v = axis.cross(u);
        let base = pos.len();
        for s in 0..segments {
            let t = std::f64::consts::TAU * s as f64 / segments as f64;
            pos.push(Point3::ORIGIN + axis * 2.0 + (u * t.cos() + v * t.sin()) * 0.3);
        }
        for s in 0..segments {
            faces.push(vec![0, base + (s + 1) % segments, base + s]);
        }
    }
    (pos, faces)
}

/// Spec 6.
#[test]
fn split_separates_two_cones_touching_at_their_apex() {
    let (pos, faces) = cones(2, 12);
    let mut obj = imported(&pos, &faces);
    let (report, pieces) = obj.mend(&plan(None, true, None)).unwrap();
    assert_eq!(report.split_vertices, 1);
    assert_eq!(pieces.len(), 1);
    assert_eq!(report.pieces, 1);
    for o in std::iter::once(&obj).chain(&pieces) {
        assert_eq!(o.faces().len(), 12);
        assert_eq!(branching(o), 0);
        assert!(o.validate().is_ok());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// Spec 6, any number of cones.
    #[test]
    fn k_cones_at_one_apex_come_apart_into_k_objects(k in 2usize..7, segments in 3usize..16) {
        let (pos, faces) = cones(k, segments);
        let mut obj = imported(&pos, &faces);
        let (report, pieces) = obj.mend(&plan(None, true, None)).unwrap();
        prop_assert_eq!(pieces.len(), k - 1);
        prop_assert_eq!(report.split_vertices, 1);
        for o in std::iter::once(&obj).chain(&pieces) {
            prop_assert_eq!(branching(o), 0);
            prop_assert_eq!(o.faces().len(), segments);
        }
    }
}

/// Spec 7. A square ring of quads missing one corner cell: the notch
/// and the bore meet at one point, so the boundary runs through it twice
/// (a figure eight). The split gives that point a copy per side, and the
/// ring, cut open there, is one object with one closed rim round it (a
/// ring cut through is a strip: one edge all round).
#[test]
fn split_a_pinched_ring_leaves_one_closed_rim_and_one_object() {
    let at = |x: usize, y: usize| y * 4 + x;
    let pos: Vec<Point3> = (0..4)
        .flat_map(|y| (0..4).map(move |x| p(x as f64, y as f64, 0.0)))
        .collect();
    let mut faces = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            if (x, y) == (1, 1) || (x, y) == (0, 0) {
                continue;
            }
            faces.push(vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)]);
        }
    }
    let mut obj = imported(&pos, &faces);
    assert_eq!(branching(&obj), 1);
    let (report, pieces) = obj.mend(&plan(None, true, None)).unwrap();
    assert_eq!(report.split_vertices, 1);
    assert_eq!(report.pieces, 0);
    assert!(pieces.is_empty());
    let d = diagnose(&obj);
    assert_eq!(d.rims.len(), 1);
    assert!(d.rims[0].closed);
    assert_eq!(d.rims[0].half_edges.len(), 12 + 4);
    assert_eq!(branching(&obj), 0);
}

// ─────────────────────────────────────────────────────────── 8–12 fill

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Spec 8. A bent hole cut anywhere in an imported sphere closes:
    /// watertight, the volume within 1 % of the whole sphere's, no fill
    /// touching the faces around it, every fill triangle facing the way
    /// the removed patch faced (within 60°).
    #[test]
    fn fill_closes_a_warped_hole_in_a_sphere(
        a in 0.0f64..std::f64::consts::TAU, b in -1.2f64..1.2, radius in 0.15f64..0.5,
    ) {
        let (pos, faces) = sphere(24, 48, 1.0);
        let whole = imported(&pos, &faces);
        let u = dir(a, b);
        let kept: Vec<Vec<usize>> = faces
            .iter()
            .filter(|f| (centroid(&pos, f).normalized().unwrap() - u).length() > radius)
            .cloned()
            .collect();
        prop_assume!(kept.len() < faces.len());
        let mut obj = imported(&pos, &kept);
        let d = diagnose(&obj);
        prop_assume!(d.rims.len() == 1 && d.rims[0].closed && branching(&obj) == 0);
        let n = d.rims[0].half_edges.len();
        prop_assume!((5..=60).contains(&n));
        prop_assume!(d.rims[0].role == RimRole::Hole);
        let before = face_keys(&obj);
        let (report, _) = obj.mend(&plan(None, true, Some(FillScope::Holes))).unwrap();
        prop_assert!(closed(&obj), "{report:?}");
        prop_assert_eq!(report.filled_rims, 1);
        let v0 = volume(&whole);
        prop_assert!((volume(&obj) - v0).abs() <= 0.01 * v0);
        let all: Vec<_> = obj.faces().keys().collect();
        prop_assert!(kernel::repair::mend::improper_contacts_for_test(&obj, &all).is_empty());
        for f in obj.faces().values() {
            let ring: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
            let mut key: Vec<[u64; 3]> = ring.iter().map(|x| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()]).collect();
            key.sort();
            if !before.contains(&key) {
                prop_assert!(f.plane.normal().dot(u) >= 0.5, "a fill face turns away");
            }
        }
    }
}

/// A tall tube of `n` sides, its bottom one flat face, its top rim
/// bent (alternate points raised): a hole small against the tube.
fn bent_tube(n: usize) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let mut pos = Vec::new();
    for j in 0..n {
        let t = std::f64::consts::TAU * j as f64 / n as f64;
        pos.push(p(t.cos(), t.sin(), 0.0));
    }
    for j in 0..n {
        let t = std::f64::consts::TAU * j as f64 / n as f64;
        let z = 10.0 + if j % 2 == 0 { 0.05 } else { -0.05 };
        pos.push(p(t.cos(), t.sin(), z));
    }
    let mut faces = vec![(0..n).rev().collect::<Vec<_>>()];
    for j in 0..n {
        let (a, b) = (j, (j + 1) % n);
        faces.push(vec![a, b, n + b]);
        faces.push(vec![a, n + b, n + a]);
    }
    (pos, faces)
}

/// Spec 9. A bent hole of 500 edges, past the triangulation's bound,
/// fills with a fan: one new point, 500 triangles, closed.
#[test]
fn fill_uses_a_fan_past_the_mwt_bound() {
    let (pos, faces) = bent_tube(500);
    let mut obj = imported(&pos, &faces);
    let d = diagnose(&obj);
    assert_eq!(d.rims.len(), 1);
    assert_eq!(d.rims[0].role, RimRole::Hole);
    assert!(d.rims[0].plane.is_none(), "the rim is bent");
    let (report, _) = obj.mend(&plan(None, true, Some(FillScope::Holes))).unwrap();
    assert!(closed(&obj));
    assert_eq!(report.fill_triangles, 500);
    assert_eq!(obj.vertices().len(), 1001);
}

/// A 4 m box whose top has a 2 m square hole, a 1 m column rising from
/// an opening in its floor up through the hole and 2 m past the top, and
/// a small square hole in its left wall.
fn box_with_column() -> HoledMesh {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(4.0, 4.0, 4.0)); // 0..8
    let sq = |x0: f64, x1: f64, z: f64| [p(x0, x0, z), p(x1, x0, z), p(x1, x1, z), p(x0, x1, z)];
    pos.extend(sq(1.0, 3.0, 4.0)); // 8..12 top hole
    pos.extend(sq(1.5, 2.5, 0.0)); // 12..16 column foot
    pos.extend(sq(1.5, 2.5, 6.0)); // 16..20 column top
    pos.extend([
        p(0.0, 1.0, 1.0),
        p(0.0, 2.0, 1.0),
        p(0.0, 2.0, 2.0),
        p(0.0, 1.0, 2.0),
    ]); // 20..24
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
        // The column: walls facing its axis (the air inside it reaches
        // the floor's opening), then its top.
        vec![12, 16, 17, 13],
        vec![13, 17, 18, 14],
        vec![14, 18, 19, 15],
        vec![15, 19, 16, 12],
        vec![16, 19, 18, 17],
    ];
    let mut holes = vec![Vec::new(); faces.len()];
    holes[0] = vec![vec![12, 13, 14, 15]];
    holes[1] = vec![vec![8, 11, 10, 9]];
    holes[5] = vec![vec![20, 21, 22, 23]];
    (pos, faces, holes)
}

fn import_doc(
    meshes: Vec<MeshRecipe>,
    defs: Vec<DefRecipe>,
    extra_roots: Vec<ImportNode>,
) -> Document {
    let mut roots: Vec<ImportNode> = meshes.into_iter().map(ImportNode::Mesh).collect();
    roots.extend(extra_roots);
    let mut doc = Document::new();
    doc.ingest(
        ImportScene {
            materials: Vec::new(),
            defs,
            roots,
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            geo: None,
            extra: Default::default(),
        },
        Vec::new(),
    )
    .expect("the fixture imports");
    doc
}

fn recipe(
    name: &str,
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    holes: Vec<Vec<Vec<usize>>>,
) -> MeshRecipe {
    let n = faces.len();
    MeshRecipe {
        name: name.into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: holes,
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

fn only(doc: &Document) -> ObjectId {
    let ids = doc.live_object_ids();
    assert_eq!(ids.len(), 1);
    ids[0]
}

/// Spec 10. The top hole's fill would cut through the column: it is
/// skipped and reported, the wall's hole is filled, and the object is
/// committed open; asking for a solid refuses.
#[test]
fn fill_skips_a_rim_a_face_pokes_through() {
    let (pos, faces, holes) = box_with_column();
    let mut doc = import_doc(
        vec![recipe("Tank", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    let id = only(&doc);
    assert_eq!(diagnose(doc.object(id).unwrap()).rims.len(), 2);
    let fill = plan(None, true, Some(FillScope::Holes));
    let strict = MendPlan {
        require_solid: true,
        ..fill.clone()
    };
    assert_eq!(
        doc.mend(id, &strict).unwrap_err(),
        DocumentError::Repair(RepairError::StillOpen)
    );
    let (report, pieces, _) = doc.mend(id, &fill).unwrap();
    assert!(pieces.is_empty());
    assert_eq!(report.filled_rims, 1);
    let obj = doc.object(id).unwrap();
    let d = diagnose(obj);
    assert_eq!(d.rims.len(), 1, "the top hole stays");
    assert!(report.skipped_rims.iter().any(|s| {
        s.reason == RepairError::FillCrossesFaces
            && s.object == 0
            && s.rim.is_some_and(|r| {
                d.rims[r].half_edges.len() == 4
                    && d.rims[r]
                        .plane
                        .is_some_and(|pl| pl.signed_distance(p(2.0, 2.0, 4.0)).abs() < 1e-9)
            })
    }));
    assert!(!closed(obj));
}

/// Spec 11. A bent sheet's edge is not a hole: the one-click scope
/// leaves it (nothing to mend); naming the rim fills it.
#[test]
fn holes_scope_never_fills_an_outer_rim_but_rims_may() {
    let pos = [
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(1.0, 2.0, 1.0),
        p(0.0, 2.0, 1.0),
    ];
    let mut obj = imported(&pos, &[vec![0, 1, 2, 3], vec![3, 2, 4, 5]]);
    assert_eq!(diagnose(&obj).verdict, Verdict::BentSheet);
    assert_eq!(
        obj.clone()
            .mend(&plan(None, true, Some(FillScope::Holes)))
            .unwrap_err(),
        RepairError::NothingToMend
    );
    let census = obj
        .mend_census(&plan(None, true, Some(FillScope::Holes)))
        .unwrap();
    assert!(census.fill_rims.is_empty());
    assert_eq!(census.outer_rims, 1);
    let (report, _) = obj
        .mend(&plan(None, true, Some(FillScope::Rims(vec![0]))))
        .unwrap();
    assert_eq!(report.filled_rims, 1);
    assert!(closed(&obj));
    let all: Vec<_> = obj.faces().keys().collect();
    assert!(kernel::repair::mend::improper_contacts_for_test(&obj, &all).is_empty());
}

/// A unit box whose top face has a square hole.
fn holed_box() -> HoledMesh {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(4.0, 4.0, 4.0));
    pos.extend([
        p(1.5, 1.5, 4.0),
        p(2.5, 1.5, 4.0),
        p(2.5, 2.5, 4.0),
        p(1.5, 2.5, 4.0),
    ]);
    let faces = box_faces();
    let mut holes = vec![Vec::new(); 6];
    holes[1] = vec![vec![8, 11, 10, 9]];
    (pos, faces, holes)
}

/// Spec 12. A flat hole takes one face, exactly as Close an opening
/// gives it.
#[test]
fn a_planar_hole_fills_with_one_face_as_cap_rims_would() {
    let (pos, faces, holes) = holed_box();
    let mut doc = import_doc(
        vec![recipe("Box", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    let id = only(&doc);
    let mut capped = doc.object(id).unwrap().clone();
    capped.cap_rims(&[0]).unwrap();
    let (report, _, _) = doc
        .mend(id, &plan(None, true, Some(FillScope::Holes)))
        .unwrap();
    assert_eq!((report.filled_rims, report.fill_triangles), (1, 0));
    let mended = doc.object(id).unwrap();
    assert!(closed(mended));
    assert_eq!(fingerprint(mended), fingerprint(&capped));
}

// ───────────────────────────────────────────────────── 13–16 the document

fn state(doc: &Document) -> (u64, usize) {
    (doc.state_hash(), doc.live_object_ids().len())
}

/// Spec 13. One undo step; undo gives the object back exactly and hides
/// the pieces, redo brings both back.
#[test]
fn mend_is_one_undo_step_and_undo_restores_the_object_exactly() {
    let (pos, faces) = cones(2, 10);
    let mut doc = import_doc(
        vec![recipe("Cones", pos, faces, vec![Vec::new(); 20])],
        Vec::new(),
        Vec::new(),
    );
    let id = only(&doc);
    let before = state(&doc);
    let print_before = fingerprint(doc.object(id).unwrap());
    let depth = doc.history_entries().undo.len();
    let (_, pieces, _) = doc
        .mend(id, &plan(Some(D), true, Some(FillScope::Holes)))
        .unwrap();
    assert_eq!(pieces.len(), 1);
    let entries = doc.history_entries().undo;
    assert_eq!(entries.len(), depth + 1);
    assert_eq!(entries.last().unwrap().label, "Mend object");
    let after = state(&doc);
    let print_after = fingerprint(doc.object(id).unwrap());
    doc.undo().unwrap();
    assert_eq!(state(&doc), before);
    assert_eq!(fingerprint(doc.object(id).unwrap()), print_before);
    assert!(doc.object(pieces[0]).is_none(), "the piece is hidden");
    doc.redo().unwrap();
    assert_eq!(state(&doc), after);
    assert_eq!(fingerprint(doc.object(id).unwrap()), print_after);
    assert!(doc.object(pieces[0]).is_some());
}

/// Every piece a Mend separates keeps the object's name and tags, and one
/// undo puts the original back: the pieces are hidden, not kept.
#[test]
fn mend_pieces_carry_the_name_and_tags_and_one_undo_restores_the_original() {
    // A pinched ring and three cones joined at one apex, side by side.
    let at = |x: usize, y: usize| y * 4 + x;
    let mut pos: Vec<Point3> = (0..4)
        .flat_map(|y| (0..4).map(move |x| p(x as f64, y as f64, 0.0)))
        .collect();
    let mut faces: Vec<Vec<usize>> = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            if (x, y) == (1, 1) || (x, y) == (0, 0) {
                continue;
            }
            faces.push(vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)]);
        }
    }
    let (cone_pos, cone_faces) = cones(3, 8);
    let offset = pos.len();
    pos.extend(cone_pos.iter().map(|q| p(q.x + 10.0, q.y, q.z + 5.0)));
    faces.extend(
        cone_faces
            .iter()
            .map(|f| f.iter().map(|i| i + offset).collect::<Vec<usize>>()),
    );
    // The ring's notched corner leaves one grid point unused; the import
    // wants every point on a face.
    let mut map = vec![usize::MAX; pos.len()];
    let mut used: Vec<Point3> = Vec::new();
    for f in &mut faces {
        for v in f.iter_mut() {
            if map[*v] == usize::MAX {
                map[*v] = used.len();
                used.push(pos[*v]);
            }
            *v = map[*v];
        }
    }
    let n = faces.len();
    let mut mesh = recipe("Casting", used, faces, vec![Vec::new(); n]);
    mesh.tags = vec![
        vec!["Parts".to_string(), "Cast".to_string()],
        vec!["Review".to_string()],
    ];
    let mut doc = import_doc(vec![mesh], Vec::new(), Vec::new());
    let id = only(&doc);
    let tags = doc.node_tags(NodeId::Object(id)).to_vec();
    assert_eq!(tags.len(), 2, "the fixture's tags came through the import");
    let name = doc.object_name(id).map(str::to_string);
    assert_eq!(name.as_deref(), Some("Casting"));
    let before = state(&doc);
    let print_before = fingerprint(doc.object(id).unwrap());
    let depth = doc.history_entries().undo.len();

    let (report, pieces, _) = doc
        .mend(id, &plan(Some(D), true, Some(FillScope::Holes)))
        .unwrap();
    assert!(pieces.len() >= 2, "the mend separates pieces: {report:?}");
    assert_eq!(doc.history_entries().undo.len(), depth + 1);
    for piece in std::iter::once(&id).chain(&pieces) {
        assert_eq!(doc.object_name(*piece).map(str::to_string), name);
        assert_eq!(doc.node_tags(NodeId::Object(*piece)), tags.as_slice());
    }

    doc.undo().unwrap();
    assert_eq!(state(&doc), before);
    assert_eq!(fingerprint(doc.object(id).unwrap()), print_before);
    assert_eq!(doc.object_name(id).map(str::to_string), name);
    assert_eq!(doc.node_tags(NodeId::Object(id)), tags.as_slice());
    for piece in &pieces {
        assert!(
            doc.object(*piece).is_none(),
            "a piece is hidden by the undo"
        );
    }
}

/// A shape with every kind of defect: a sphere with a bent hole, its
/// first faces' corners private and jittered, and a triangle hanging off
/// one point (a bow-tie).
fn defect_soup() -> (Vec<Point3>, Vec<Vec<usize>>) {
    let (mut pos, mut faces) = sphere(10, 16, 1.0);
    let u = dir(0.4, 0.3);
    faces.retain(|f| (centroid(&pos, f).normalized().unwrap() - u).length() > 0.45);
    for (k, f) in faces.iter_mut().enumerate().take(6) {
        for (m, v) in f.iter_mut().enumerate() {
            let q = pos[*v] + dir(k as f64 * 2.1 + m as f64, m as f64 * 0.7) * (D / 4.0);
            pos.push(q);
            *v = pos.len() - 1;
        }
    }
    let south = 0;
    pos.push(p(0.3, 0.1, -1.6));
    pos.push(p(-0.2, 0.25, -1.7));
    faces.push(vec![south, pos.len() - 2, pos.len() - 1]);
    // Drop points nothing uses.
    let mut map = vec![usize::MAX; pos.len()];
    let mut out = Vec::new();
    for f in faces.iter_mut() {
        for v in f.iter_mut() {
            if map[*v] == usize::MAX {
                map[*v] = out.len();
                out.push(pos[*v]);
            }
            *v = map[*v];
        }
    }
    (out, faces)
}

/// Spec 14. The same shape with its faces in another order mends to the
/// same geometry, and two runs agree.
#[test]
fn mend_is_deterministic_under_face_permutation() {
    let (pos, faces) = defect_soup();
    let all = MendPlan::all(D);
    let run = |faces: &[Vec<usize>]| {
        let mut o = imported(&pos, faces);
        let (report, pieces) = o.mend(&all).unwrap();
        let mut prints: Vec<_> = pieces.iter().map(fingerprint).collect();
        prints.sort();
        (
            fingerprint(&o),
            prints,
            report.welded_vertices,
            report.filled_rims,
            closed(&o),
        )
    };
    let first = run(&faces);
    assert_eq!(run(&faces), first);
    let mut reversed = faces.clone();
    reversed.reverse();
    assert_eq!(run(&reversed), first);
    let mut shuffled = faces.clone();
    let n = shuffled.len();
    for i in 0..n {
        shuffled.swap(i, (i * 7919 + 13) % n);
    }
    assert_eq!(run(&shuffled), first);
    assert!(first.4, "the soup mends to a solid");
}

/// Spec 15. Mending a definition's member reaches both placements.
#[test]
fn mend_inside_a_definition_reaches_every_instance() {
    let (pos, faces) = split_corner_box([1.0, 1.0, 1.0], &|v| v, &|f, v| {
        Vec3::new(1.0, 0.5, 0.25) * ((f + v) as f64 * D / 40.0)
    });
    let n = faces.len();
    let place = |x: f64| ImportNode::Instance {
        def: 0,
        pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut doc = import_doc(
        Vec::new(),
        vec![DefRecipe {
            name: Some("Bin".into()),
            children: vec![ImportNode::Mesh(recipe(
                "Bin",
                pos,
                faces,
                vec![Vec::new(); n],
            ))],
        }],
        vec![place(0.0), place(5.0)],
    );
    let member = only(&doc);
    assert!(
        doc.visible_object_ids().is_empty(),
        "the box is a member, not a world object"
    );
    let (report, _, change) = doc.mend(member, &MendPlan::all(D)).unwrap();
    assert!(report.closed);
    assert_eq!(change.instances_touched.len(), 2);
    assert!(closed(doc.object(member).unwrap()));
}

/// Spec 16. Where nothing crosses, the preview's counts are the report's.
#[test]
fn census_counts_match_the_report_when_nothing_crosses() {
    let (mut pos, mut faces, mut holes) = holed_box();
    // A bow-tie of two triangles beside the box: two islands touching at
    // one point.
    let base = pos.len();
    pos.extend([
        p(6.0, 0.0, 0.0),
        p(7.0, 0.0, 0.0),
        p(7.0, 1.0, 0.0),
        p(5.0, 0.0, 1.0),
        p(5.0, -1.0, 1.0),
    ]);
    faces.push(vec![base, base + 1, base + 2]);
    faces.push(vec![base, base + 3, base + 4]);
    holes.push(Vec::new());
    holes.push(Vec::new());
    let mut doc = import_doc(
        vec![recipe("Mixed", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    let id = only(&doc);
    let all = plan(None, true, Some(FillScope::Holes));
    let census = doc.mend_census(id, &all).unwrap();
    let (report, pieces, _) = doc.mend(id, &all).unwrap();
    assert_eq!(census.branching_vertices, report.split_vertices);
    assert_eq!(census.islands - 1, report.pieces);
    assert_eq!(census.fill_rims.len(), report.filled_rims);
    assert_eq!(pieces.len(), 2);
    assert!(report.closed);

    // And the weld's.
    let (pos, faces) = split_corner_box([1.0, 2.0, 3.0], &|v| v, &|_, _| Vec3::ZERO);
    let mut obj = imported(&pos, &faces);
    let weld = plan(Some(D), false, None);
    let census = obj.mend_census(&weld).unwrap();
    let (report, _) = obj.mend(&weld).unwrap();
    assert_eq!(census.weld_vertices, report.welded_vertices);
    assert_eq!(census.weld_points.len(), 24);
}

/// A seam no wider than the weld distance (one side's point a hair off
/// the other side's edge) is stitched shut: no new face, closed.
#[test]
fn a_seam_narrower_than_the_distance_is_stitched_shut() {
    // A box whose front face is cut in two by a vertical line at x = 0.5;
    // the top's front edge knows nothing of the cut point, which sits
    // d/4 below it: a T-junction crack.
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    pos.push(p(0.5, 0.0, 0.0)); // 8
    pos.push(p(0.5, 0.0, 1.0 - D / 4.0)); // 9
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 8, 9, 4],
        vec![8, 1, 5, 9],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
        vec![0, 1, 8],
    ];
    // The bottom must know the cut point too: replace it.
    let mut faces = faces;
    faces[0] = vec![0, 3, 2, 1, 8];
    faces.pop();
    let mut obj = imported(&pos, &faces);
    let d = diagnose(&obj);
    assert_eq!(d.rims.len(), 1);
    let (report, _) = obj.mend(&plan(None, true, Some(FillScope::Holes))).unwrap();
    assert_eq!(
        (
            report.filled_rims,
            report.stitched_rims,
            report.fill_triangles
        ),
        (1, 1, 0)
    );
    assert!(closed(&obj));
    assert_eq!(obj.faces().len(), 7);
}

// ──────────────────────────────────────────────────────────────── folds

/// The seam box of `a_seam_narrower_than_the_distance_is_stitched_shut`
/// with a fold lying in its crack: a face no wider than d/4, from the
/// corner past the cut point, that makes the crack's sides double back.
fn folded_seam_box() -> Object {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    pos.push(p(0.5, 0.0, 0.0)); // 8
    pos.push(p(0.5, 0.0, 1.0 - D / 4.0)); // 9
    pos.push(p(0.75, 0.0, 1.0 - D / 8.0)); // 10
    let faces = vec![
        vec![0, 3, 2, 1, 8],
        vec![4, 5, 6, 7],
        vec![0, 8, 9, 4],
        vec![8, 1, 5, 9],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
        vec![4, 9, 10],
    ];
    imported(&pos, &faces)
}

/// A zero-width fold in a seam keeps it from being stitched; the fold
/// step removes it (reported) and the seam closes.
#[test]
fn a_zero_width_fold_in_a_seam_is_removed_and_the_seam_closes() {
    let obj = folded_seam_box();
    let without = plan(None, true, Some(FillScope::Holes));
    let mut kept = obj.clone();
    match kept.mend(&without) {
        Ok((report, _)) => assert!(!report.closed && report.folds_removed == 0, "{report:?}"),
        Err(e) => assert_ne!(e, RepairError::Invalid),
    }
    let with = MendPlan {
        folds: true,
        ..without.clone()
    };
    let census = obj.mend_census(&with).unwrap();
    assert_eq!(census.folds, 1);
    let mut mended = obj.clone();
    let (report, pieces) = mended.mend(&with).unwrap();
    assert!(pieces.is_empty());
    assert_eq!(report.folds_removed, 1);
    assert!(report.closed, "{report:?}");
    assert_eq!(mended.faces().len(), 7);
    assert!((volume(&mended) - 1.0).abs() < 1e-5);
}

/// Folds removed without the split may not leave parts touching at one
/// point: two triangles joined only through a sliver refuse, untouched;
/// with the split they come apart.
#[test]
fn removing_a_fold_that_would_pinch_refuses_without_the_split() {
    let pos = [
        p(0.0, 0.0, 0.0),
        p(-1.0, 0.5, 0.0),
        p(-0.2, 1.0, 0.0),
        p(-0.2 + D / 4.0, 1.0, 0.0),
        p(1.0, 0.5, 0.0),
    ];
    // A, the sliver (0, 2, 3), B.
    let faces = vec![vec![0, 2, 1], vec![0, 3, 2], vec![0, 4, 3]];
    let obj = imported(&pos, &faces);
    let folds_only = MendPlan {
        weld: None,
        folds: true,
        split: false,
        fill: None,
        require_solid: false,
    };
    let mut o = obj.clone();
    assert_eq!(
        o.mend(&folds_only).unwrap_err(),
        RepairError::FoldRemovalPinches
    );
    assert_eq!(fingerprint(&o), fingerprint(&obj));
    let mut o = obj.clone();
    let (report, pieces) = o
        .mend(&MendPlan {
            split: true,
            ..folds_only
        })
        .unwrap();
    assert_eq!(report.folds_removed, 1);
    assert_eq!(pieces.len(), 1);
    assert_eq!(branching(&o) + branching(&pieces[0]), 0);
}

// ─────────────────────────────────────────────────── review follow-ups

/// F5: a mesh far from the origin welds without overflowing its grid.
#[test]
fn a_weld_far_from_the_origin_keeps_its_grid_in_range() {
    let far = Vec3::new(1e15, -1e15, 1e15);
    let (pos, faces) = split_corner_box([1.0, 1.0, 1.0], &move |v| v, &|_, _| Vec3::ZERO);
    let pos: Vec<Point3> = pos.into_iter().map(|q| q + far).collect();
    let mut obj = imported(&pos, &faces);
    let census = obj.mend_census(&plan(Some(D), false, None)).unwrap();
    assert_eq!(census.weld_vertices, 16);
    let (report, _) = obj.mend(&plan(Some(D), false, None)).unwrap();
    assert_eq!(report.welded_vertices, 16);
}

/// F6: the preview refuses what the Mend refuses before any work.
#[test]
fn the_census_refuses_what_the_mend_refuses() {
    let (pos, faces) = split_corner_box([1.0, 1.0, 1.0], &|v| v, &|_, _| Vec3::ZERO);
    let obj = imported(&pos, &faces);
    for bad in [
        plan(Some(2e-3), true, None),
        plan(None, true, Some(FillScope::Rims(vec![99]))),
    ] {
        let want = obj.clone().mend(&bad).unwrap_err();
        assert_eq!(obj.mend_census(&bad).unwrap_err(), want);
    }
    let (pos, faces) = cones(2, 8);
    let n = faces.len();
    let mut doc = import_doc(
        Vec::new(),
        vec![DefRecipe {
            name: Some("C".into()),
            children: vec![ImportNode::Mesh(recipe(
                "C",
                pos,
                faces,
                vec![Vec::new(); n],
            ))],
        }],
        Vec::new(),
    );
    let member = only(&doc);
    assert!(doc.mend_census(member, &MendPlan::all(D)).is_ok());
    let _ = &mut doc;
}

/// F4: a face repeated exactly (a doubled surface) is a fold: one copy
/// goes and the cube stays one closed object.
#[test]
fn an_exactly_repeated_face_is_removed_as_a_fold() {
    let pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    let mut faces = box_faces();
    faces.push(faces[1].clone());
    // The builder refuses a doubled directed edge, so the soup reaches
    // Mend as the import would leave it: the copy on its own points.
    let mut pos = pos;
    let copy: Vec<usize> = faces[6]
        .iter()
        .map(|&v| {
            pos.push(pos[v]);
            pos.len() - 1
        })
        .collect();
    faces[6] = copy;
    let mut obj = imported(&pos, &faces);
    let (report, pieces) = obj.mend(&MendPlan::all(D)).unwrap();
    assert!(pieces.is_empty(), "{report:?}");
    assert_eq!(report.folds_removed, 1);
    assert!(report.closed);
    assert_eq!(obj.faces().len(), 6);
}

/// F12: a raised weld distance never takes a thin wall that is there.
#[test]
fn a_raised_weld_distance_leaves_thin_walls_alone() {
    // An open-topped box with 0.2 mm walls: the wall tops are 0.2 mm
    // strips along the open edge.
    let (o, i, h) = (0.02, 0.0196, 0.02);
    let mut pos = corners(p(0.0, 0.0, 0.0), p(o, o, h));
    let off = (o - i) / 2.0;
    pos.extend(corners(p(off, off, off), p(off + i, off + i, h)));
    let mut faces: Vec<Vec<usize>> = box_faces()
        .into_iter()
        .enumerate()
        .filter(|&(k, _)| k != 1)
        .map(|(_, f)| f)
        .collect();
    faces.extend(
        box_faces()
            .into_iter()
            .enumerate()
            .filter(|&(k, _)| k != 1)
            .map(|(_, f)| f.into_iter().rev().map(|v| v + 8).collect()),
    );
    // The rim strips between the walls, at the top.
    faces.push(vec![4, 5, 13, 12]);
    faces.push(vec![5, 6, 14, 13]);
    faces.push(vec![6, 7, 15, 14]);
    faces.push(vec![7, 4, 12, 15]);
    let obj = imported(&pos, &faces);
    assert!(closed(&obj));
    // Nothing is open, so open a gap: drop one strip.
    let mut faces2 = faces.clone();
    faces2.pop();
    let mut obj = imported(&pos, &faces2);
    // 0.25 mm: past the 0.2 mm strips' width, short of the 0.28 mm
    // between a wall's inner and outer corners, so no corner merges.
    let raised = MendPlan {
        weld: Some(2.5e-4),
        ..MendPlan::all(D)
    };
    let census = obj.mend_census(&raised).unwrap();
    assert_eq!(census.folds, 0, "a 0.2 mm strip is not a fold");
    if let Ok((report, _)) = obj.clone().mend(&raised) {
        assert_eq!(report.folds_removed, 0);
    }
    // 0.5 mm merges each wall's inner corners onto its outer ones,
    // pressing the walls flat: refused, in the preview as in the commit.
    let pressing = MendPlan {
        weld: Some(5e-4),
        ..MendPlan::all(D)
    };
    assert_eq!(
        obj.mend_census(&pressing).unwrap_err(),
        RepairError::WeldFlattensObject
    );
    assert_eq!(
        obj.mend(&pressing).unwrap_err(),
        RepairError::WeldFlattensObject
    );
}

/// F15: an outer edge left open is no skipped gap.
#[test]
fn an_outer_edge_is_not_reported_as_a_skipped_gap() {
    let (pos, faces) = cones(2, 12);
    let mut obj = imported(&pos, &faces);
    let (report, _) = obj.mend(&MendPlan::all(D)).unwrap();
    assert!(report.skipped_rims.is_empty(), "{:?}", report.skipped_rims);
}

/// F17: the diameter is the same to the bit however the points are listed.
#[test]
fn the_diameter_does_not_depend_on_the_listing() {
    let (pos, _) = sphere(9, 13, 0.37);
    let pos: Vec<Point3> = pos
        .iter()
        .enumerate()
        .map(|(k, q)| *q + Vec3::new((k as f64 * 0.7).sin() * 0.01, 0.0, 0.0))
        .collect();
    let d0 = kernel::health::diameter(&pos);
    let mut rev = pos.clone();
    rev.reverse();
    let mut mixed = pos.clone();
    let n = mixed.len();
    for k in 0..n {
        mixed.swap(k, (k * 37 + 11) % n);
    }
    assert_eq!(kernel::health::diameter(&rev).to_bits(), d0.to_bits());
    assert_eq!(kernel::health::diameter(&mixed).to_bits(), d0.to_bits());
}

/// The preview counts the pinches the split makes, inside closed surfaces
/// too: a cube missing two triangles that meet at one corner (a pinch on
/// its rims), with a second cube touching it at another corner (a pinch no
/// rim runs through).
#[test]
fn the_census_counts_every_point_the_split_splits() {
    let tri_box = |lo: Point3, hi: Point3| -> (Vec<Point3>, Vec<Vec<usize>>) {
        let pos = corners(lo, hi);
        let faces = box_faces()
            .into_iter()
            .flat_map(|q| [vec![q[0], q[1], q[2]], vec![q[0], q[2], q[3]]])
            .collect();
        (pos, faces)
    };
    let (mut pos, mut faces) = tri_box(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    // Corner 6 (1,1,1): drop one top triangle and one right-side triangle
    // through it that share no edge.
    let top = faces.iter().position(|f| f == &vec![4, 6, 7]).unwrap();
    faces.remove(top);
    let side = faces.iter().position(|f| f == &vec![1, 2, 6]).unwrap();
    faces.remove(side);
    // A second closed cube on corner 0 (0,0,0), below and behind.
    let (pos2, faces2) = tri_box(p(-1.0, -1.0, -1.0), p(0.0, 0.0, 0.0));
    let base = pos.len();
    pos.extend(pos2);
    for f in faces2 {
        faces.push(
            f.into_iter()
                .map(|v| if v == 6 { 0 } else { v + base })
                .collect(),
        );
    }
    let mut obj = imported(&pos, &faces);
    let all = plan(None, true, None);
    let census = obj.mend_census(&all).unwrap();
    let (report, _) = obj.mend(&all).unwrap();
    assert_eq!(report.split_vertices, 2);
    assert_eq!(census.branching_vertices, report.split_vertices);
    assert_eq!(census.split_points.len(), 2);
}

/// The preview runs the Mend itself on a copy, so its counts are the
/// commit's: points merged, faces collapsed, folds removed, points and
/// edges split, pieces, rims filled and left open.
fn assert_census_is_the_report(obj: &Object, plan: &MendPlan) {
    let census = obj.mend_census(plan).unwrap();
    let mut o = obj.clone();
    let (report, _) = o.mend(plan).unwrap();
    assert_eq!(census.weld_vertices, report.welded_vertices, "merged");
    assert_eq!(census.collapsed_faces, report.collapsed_faces, "collapsed");
    assert_eq!(census.folds, report.folds_removed, "folds");
    assert_eq!(
        census.branching_vertices, report.split_vertices,
        "split points"
    );
    assert_eq!(census.split_edges, report.split_edges, "split edges");
    assert_eq!(census.islands - 1, report.pieces, "pieces");
    assert_eq!(census.pieces, report.pieces, "pieces");
    assert_eq!(census.filled_rims, report.filled_rims, "filled");
    assert_eq!(census.skipped_rims, report.skipped_rims.len(), "left open");
    assert!(census.fill_curved <= census.filled_rims);
}

#[test]
fn the_census_is_the_report_on_a_soup_with_every_defect() {
    let (pos, faces) = defect_soup();
    let obj = imported(&pos, &faces);
    assert_census_is_the_report(&obj, &MendPlan::all(D));
    assert_census_is_the_report(&folded_seam_box(), &MendPlan::all(D));
    let (pos, faces) = cones(3, 9);
    assert_census_is_the_report(&imported(&pos, &faces), &MendPlan::all(D));
    let (pos, faces, holes) = holed_box();
    let doc = import_doc(
        vec![recipe("Box", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    assert_census_is_the_report(doc.object(only(&doc)).unwrap(), &MendPlan::all(D));
}

/// A flat fill beside its plane adds no drawing: the seam between the
/// fill and the face around the hole is made soft and counted.
#[test]
fn a_flat_fill_beside_its_plane_adds_no_drawing() {
    let (pos, faces, holes) = holed_box();
    let mut doc = import_doc(
        vec![recipe("Box", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    let id = only(&doc);
    assert!(doc.object_drawing_planes(id).is_empty());
    let (report, _, _) = doc
        .mend(id, &plan(None, true, Some(FillScope::Holes)))
        .unwrap();
    assert!(report.closed);
    assert_eq!(report.soft_seams, 4);
    assert!(
        doc.object_drawing_planes(id).is_empty(),
        "no new shape on the top"
    );
}

// ─────────────────────────────────────────────────── review 2 follow-ups

/// F1: with the split off, a weld that crowds an edge refuses in the
/// preview as in the commit.
#[test]
fn the_census_refuses_a_weld_that_folds_faces_as_the_mend_does() {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    pos.extend(corners(p(1.0, 0.0, 0.0), p(2.0, 1.0, 1.0)));
    let mut faces = box_faces();
    faces.extend(
        box_faces()
            .into_iter()
            .map(|f| f.into_iter().map(|i| i + 8).collect()),
    );
    let obj = imported(&pos, &faces);
    let alone = MendPlan {
        weld: Some(D),
        folds: true,
        split: false,
        fill: None,
        require_solid: false,
    };
    assert_eq!(
        obj.clone().mend(&alone).unwrap_err(),
        RepairError::WeldFoldsFaces
    );
    assert_eq!(
        obj.mend_census(&alone).unwrap_err(),
        RepairError::WeldFoldsFaces
    );
}

/// F10: a weld that makes two faces one ring, with the folds on and the
/// split off, removes the copy instead of refusing — and the preview
/// counts that fold.
#[test]
fn a_weld_made_repeat_is_a_fold_not_a_refusal() {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    let mut faces = box_faces();
    // A second copy of the top on its own points, a hair above.
    let base = pos.len();
    for v in [4, 5, 6, 7] {
        pos.push(pos[v] + Vec3::new(0.0, 0.0, D / 4.0));
    }
    faces.push(vec![base, base + 1, base + 2, base + 3]);
    let obj = imported(&pos, &faces);
    let plan = MendPlan {
        weld: Some(D),
        folds: true,
        split: false,
        fill: None,
        require_solid: false,
    };
    let census = obj.mend_census(&plan).unwrap();
    let mut o = obj.clone();
    let (report, _) = o.mend(&plan).unwrap();
    assert_eq!(report.folds_removed, 1);
    assert_eq!(census.folds, report.folds_removed);
    assert!(report.closed);
}

/// F8: an edge through a merged point keeps its marks.
#[test]
fn an_edge_through_a_welded_point_keeps_its_soft_mark() {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    // The back face's copy of corner 6, a third of d lower in x: the
    // weld's root (the lowest point), so corner 6 moves onto it.
    pos.push(pos[6] - Vec3::new(D / 3.0, 0.0, 0.0));
    let mut faces = box_faces();
    faces[4] = vec![2, 3, 7, 8];
    let (c5, c6) = (p(1.0, 0.0, 1.0), p(1.0, 1.0, 1.0));
    // The edge from corner 5 to corner 6 is shared by the top and the
    // right face, so it can be soft before the weld.
    let mut mesh = recipe("Box", pos, faces, Vec::new());
    mesh.extra.soft_edges = vec![[c5, c6]];
    let mut doc = import_doc(vec![mesh], Vec::new(), Vec::new());
    let id = only(&doc);
    let at = |o: &Object, a: Point3, b: Point3| {
        o.edges()
            .iter()
            .find(|(_, e)| {
                let h = &o.half_edges()[e.half_edge];
                let (x, y) = (
                    o.vertices()[h.origin].position,
                    o.vertices()[o.half_edges()[h.next].origin].position,
                );
                (x.approx_eq(a, D) && y.approx_eq(b, D)) || (x.approx_eq(b, D) && y.approx_eq(a, D))
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let before = doc.object(id).unwrap();
    assert!(
        before.edges()[at(before, c5, c6)].soft,
        "the fixture's edge is soft"
    );
    let (report, _, _) = doc.mend(id, &plan(Some(D), true, None)).unwrap();
    assert_eq!(report.welded_vertices, 1);
    let obj = doc.object(id).unwrap();
    assert!(closed(obj));
    assert!(
        obj.edges()[at(obj, c5, c6)].soft,
        "the soft mark survives the weld"
    );
}

/// F9: the seams a planar fill leaves between two faces on one plane are
/// soft.
#[test]
fn a_planar_fill_softens_its_seams() {
    let (pos, faces, holes) = holed_box();
    let mut doc = import_doc(
        vec![recipe("Box", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    let id = only(&doc);
    let (report, _, _) = doc
        .mend(id, &plan(None, true, Some(FillScope::Holes)))
        .unwrap();
    assert_eq!(report.soft_seams, 4);
    // The soft edges are the hole's four sides, where the cap meets the
    // top around it; every other edge stays hard.
    let obj = doc.object(id).unwrap();
    let key = |a: Point3, b: Point3| {
        let k = |x: Point3| [x.x.to_bits(), x.y.to_bits(), x.z.to_bits()];
        let (a, b) = (k(a), k(b));
        if a <= b { (a, b) } else { (b, a) }
    };
    let soft: std::collections::BTreeSet<_> = obj
        .edges()
        .values()
        .filter(|e| e.soft)
        .map(|e| {
            let h = &obj.half_edges()[e.half_edge];
            key(
                obj.vertices()[h.origin].position,
                obj.vertices()[obj.half_edges()[h.next].origin].position,
            )
        })
        .collect();
    let ring = [
        p(1.5, 1.5, 4.0),
        p(2.5, 1.5, 4.0),
        p(2.5, 2.5, 4.0),
        p(1.5, 2.5, 4.0),
    ];
    let seams: std::collections::BTreeSet<_> =
        (0..4).map(|k| key(ring[k], ring[(k + 1) % 4])).collect();
    assert_eq!(soft, seams);
}

// ─────────────────────────────────────────────────── review 3 follow-ups

/// A closed prism over `ring` (counterclockwise from +z), `h` tall.
fn prism(ring: &[(f64, f64)], h: f64) -> Object {
    let n = ring.len();
    let mut pos: Vec<Point3> = ring.iter().map(|&(x, y)| p(x, y, 0.0)).collect();
    pos.extend(ring.iter().map(|&(x, y)| p(x, y, h)));
    let mut faces = vec![(0..n).rev().collect::<Vec<_>>(), (n..2 * n).collect()];
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, n + j, n + i]);
    }
    imported(&pos, &faces)
}

/// A weld that joins two points of one face ring that are not neighbors
/// (an hourglass whose neck is narrower than the distance) must not open
/// a solid: with the split on as with it off, it refuses and the object
/// is untouched; the preview refuses the same.
#[test]
fn a_weld_across_a_narrow_neck_never_opens_a_solid() {
    let ring = [
        (0.0, 0.0),
        (2.0, 0.0),
        (1.0, 1.0 - 2e-6),
        (2.0, 2.0),
        (0.0, 2.0),
        (1.0, 1.0 + 2e-6),
    ];
    let obj = prism(&ring, 1.0);
    assert!(closed(&obj));
    assert_eq!(obj.faces().len(), 8);
    let all = MendPlan::all(D);
    let mut o = obj.clone();
    assert_eq!(o.mend(&all).unwrap_err(), RepairError::MendOpensObject);
    assert_eq!(fingerprint(&o), fingerprint(&obj), "untouched");
    assert_eq!(
        obj.mend_census(&all).unwrap_err(),
        RepairError::MendOpensObject
    );
    let alone = MendPlan {
        split: false,
        ..all
    };
    assert_eq!(
        obj.clone().mend(&alone).unwrap_err(),
        RepairError::WeldFoldsFaces
    );
}

/// A closed box thinner than the merge distance would be pressed into
/// two faces around no volume: refused, the object untouched. A thin but
/// real plate (1 mm) welded at the default scale still closes with its
/// volume.
#[test]
fn a_weld_never_presses_a_thin_solid_flat() {
    let thin = imported(&corners(p(0.0, 0.0, 0.0), p(1.0, 1.0, 1e-5)), &box_faces());
    assert!(closed(&thin));
    for d in [4e-5, 1e-3] {
        let strict = MendPlan {
            require_solid: true,
            ..MendPlan::all(d)
        };
        for plan in [strict, MendPlan::all(d)] {
            let mut o = thin.clone();
            assert_eq!(
                o.mend(&plan).unwrap_err(),
                RepairError::WeldFlattensObject,
                "{d}"
            );
            assert_eq!(fingerprint(&o), fingerprint(&thin), "untouched");
            assert_eq!(
                thin.mend_census(&plan).unwrap_err(),
                RepairError::WeldFlattensObject
            );
        }
    }

    let shake = |f: usize, v: usize| dir(f as f64 * 1.3 + v as f64, v as f64 * 0.7) * (D / 3.0);
    let (pos, faces) = split_corner_box([1.0, 1.0, 1e-3], &|v| v, &shake);
    let mut plate = imported(&pos, &faces);
    let (report, pieces) = plate.mend(&MendPlan::all(D)).unwrap();
    assert!(pieces.is_empty());
    assert_eq!(report.welded_vertices, 16);
    assert!(closed(&plate));
    assert!((volume(&plate).abs() - 1e-3).abs() <= surface_area(&plate) * D);
}

/// A point merged across a sliver's opposite edge would turn the sliver
/// over (its normal against its neighbors'): refused, split or not.
#[test]
fn a_weld_never_turns_a_face_over() {
    let h = 3e-6;
    let pos = [
        p(0.0, 0.0, 0.0),  // a
        p(1.0, 0.0, 0.0),  // b
        p(0.5, h, 0.0),    // c: the sliver's tip, h above a–b
        p(0.0, 1.0, 0.0),  // t1
        p(1.0, 1.0, 0.0),  // t2
        p(0.5, -h, 0.0),   // q: across a–b, within the distance of c
        p(0.0, -1.0, 0.0), // l1
        p(1.0, -1.0, 0.0), // l2
    ];
    let faces = vec![
        vec![0, 1, 2],
        vec![0, 2, 3],
        vec![2, 1, 4],
        vec![2, 4, 3],
        vec![1, 0, 6, 5, 7],
        vec![5, 6, 7],
    ];
    let obj = imported(&pos, &faces);
    assert!(obj.faces().values().all(|f| f.plane.normal().z > 0.0));
    for split in [false, true] {
        let weld = MendPlan {
            weld: Some(D),
            folds: false,
            split,
            fill: None,
            require_solid: false,
        };
        let mut o = obj.clone();
        assert_eq!(o.mend(&weld).unwrap_err(), RepairError::WeldFlipsFaces);
        assert_eq!(fingerprint(&o), fingerprint(&obj), "untouched");
        assert_eq!(
            obj.mend_census(&weld).unwrap_err(),
            RepairError::WeldFlipsFaces
        );
    }
}

/// A hole narrower than the merge distance closes by the weld alone: the
/// preview counts no fill for it, as the result does.
#[test]
fn a_pinhole_the_weld_closes_is_not_counted_as_a_fill() {
    let mut pos = corners(p(0.0, 0.0, 0.0), p(4.0, 4.0, 4.0));
    let r = 2e-6;
    pos.extend([
        p(2.0 - r, 2.0 - r, 4.0),
        p(2.0 + r, 2.0 - r, 4.0),
        p(2.0 + r, 2.0 + r, 4.0),
        p(2.0 - r, 2.0 + r, 4.0),
    ]);
    let mut holes = vec![Vec::new(); 6];
    holes[1] = vec![vec![8, 11, 10, 9]];
    let doc = import_doc(
        vec![recipe("Box", pos, box_faces(), holes)],
        Vec::new(),
        Vec::new(),
    );
    let obj = doc.object(only(&doc)).unwrap().clone();
    assert_eq!(diagnose(&obj).rims.len(), 1);
    let census = obj.mend_census(&MendPlan::all(D)).unwrap();
    assert!(census.fill_rims.is_empty(), "{:?}", census.fill_rims);
    assert_eq!(census.filled_rims, 0);
    assert_census_is_the_report(&obj, &MendPlan::all(D));
    let mut o = obj.clone();
    let (report, _) = o.mend(&MendPlan::all(D)).unwrap();
    assert!(report.closed);
}

/// A rim the fill refuses (a face pokes through it) is not counted as a
/// fill in the preview: it is counted as left open, as in the result.
#[test]
fn the_census_counts_a_refused_fill_as_left_open() {
    let (pos, faces, holes) = box_with_column();
    let doc = import_doc(
        vec![recipe("Tank", pos, faces, holes)],
        Vec::new(),
        Vec::new(),
    );
    let obj = doc.object(only(&doc)).unwrap().clone();
    for plan in [plan(None, true, Some(FillScope::Holes)), MendPlan::all(D)] {
        let census = obj.mend_census(&plan).unwrap();
        assert_eq!(census.filled_rims, 1);
        assert_eq!(census.skipped_rims, 1);
        assert_eq!(census.fill_rims.len(), 1);
        assert_census_is_the_report(&obj, &plan);
    }
}
