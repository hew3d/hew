//! Booleans whose cut leaves faces far below a micron (KB-25).
//!
//! A face used to need twice its area of at least 1e-12 m² (about a micron
//! square) to keep a plane, so a cut passing a fraction of a micron from a
//! vertex left a sub-face the kernel refused, and the boolean refused with
//! it. `Plane::from_polygon` now also keeps a face at least
//! `tol::MIN_FACE_WIDTH` (10 nm) wide, and the boolean merges away every
//! face its own cut leaves narrower than `tol::FEATURE_MIN` (0.1 µm),
//! moving no point farther than `tol::TIDY_REACH` and reporting what it
//! merged (`BooleanTidy`). So a cap at least 0.3 µm high is kept exactly,
//! and a lower one is merged into the solid it pokes through: the union is
//! the box, the intersection the tetrahedron, the box minus the
//! tetrahedron a box with the tetrahedron as a void, and the tetrahedron
//! minus the box nothing at all.
//!
//! The geometry: a native 10 mm box and a native tetrahedron with a 5 mm
//! equilateral base 5 mm under the box top and its apex `h` above the top's
//! middle. The tetrahedron pokes through the top in a cap of height `h`
//! whose cross-section is a triangle about `h` across, so every face the
//! cut makes there scales with `h`.

use kernel::trig;
use kernel::{
    BooleanError, BooleanOp, BooleanTidy, Document, ImportNode, ImportScene, MeshRecipe, Object,
    Plane, Point3, Transform, Vec3, WatertightState, tol,
};
use proptest::prelude::*;

const MM: f64 = 1e-3;
const BOX: f64 = 10.0 * MM;
/// The tetrahedron's base: an equilateral triangle this long on a side.
const BASE: f64 = 5.0 * MM;
/// How far under the box top the tetrahedron's base lies.
const DEPTH: f64 = 5.0 * MM;

/// Heights whose cap is kept exactly: a control above the old area floor,
/// then two under it whose every cut face is at least `1.5 ×
/// tol::FEATURE_MIN` wide, so nothing is merged.
const KEPT: [f64; 3] = [3e-6, 1e-6, 3e-7];

/// Heights whose cap is narrower than `tol::FEATURE_MIN`, down to inside
/// the merge distance: merged away.
const TIDIED: [f64; 4] = [5e-8, 2e-8, 5e-9, 5e-10];

fn box_corners() -> Vec<Point3> {
    let (lo, hi) = (0.0, BOX);
    vec![
        Point3::new(lo, lo, lo),
        Point3::new(hi, lo, lo),
        Point3::new(hi, hi, lo),
        Point3::new(lo, hi, lo),
        Point3::new(lo, lo, hi),
        Point3::new(hi, lo, hi),
        Point3::new(hi, hi, hi),
        Point3::new(lo, hi, hi),
    ]
}

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

/// The tetrahedron's four corners: base counterclockwise seen from above,
/// then the apex, `h` over the middle of the box top, each moved by `t`.
fn tet_corners(h: f64, apex_xy: (f64, f64), t: &Transform) -> Vec<Point3> {
    let (cx, cy) = (BOX / 2.0, BOX / 2.0);
    let r = BASE / 3f64.sqrt();
    let z = BOX - DEPTH;
    let mut pts: Vec<Point3> = [90.0f64, 210.0, 330.0]
        .iter()
        .map(|a| {
            let a = a.to_radians();
            Point3::new(cx + r * trig::cos(a), cy + r * trig::sin(a), z)
        })
        .collect();
    pts.push(Point3::new(apex_xy.0, apex_xy.1, BOX + h));
    pts.into_iter().map(|p| t.apply_point(p)).collect()
}

fn tet_faces() -> Vec<Vec<usize>> {
    vec![vec![0, 2, 1], vec![0, 1, 3], vec![1, 2, 3], vec![2, 0, 3]]
}

fn native_box() -> Object {
    Object::from_polygons(&box_corners(), &box_faces()).unwrap()
}

fn tet_at(h: f64, apex_xy: (f64, f64), t: &Transform) -> Object {
    let obj = Object::from_polygons(&tet_corners(h, apex_xy, t), &tet_faces()).unwrap();
    assert!(obj.volume().unwrap() > 0.0, "outward wound");
    obj
}

fn tet(h: f64) -> Object {
    tet_at(h, (BOX / 2.0, BOX / 2.0), &Transform::IDENTITY)
}

fn run(op: BooleanOp, a: &Object, b: &Object) -> Result<Object, BooleanError> {
    Object::boolean(op, a, b, &Transform::IDENTITY)
}

/// [`run`], with what the boolean's tidy merged.
fn run_reported(
    op: BooleanOp,
    a: &Object,
    b: &Object,
) -> (Result<Object, BooleanError>, BooleanTidy) {
    let mut tidy = BooleanTidy::default();
    let r = Object::boolean_reported(op, a, b, &Transform::IDENTITY, &mut tidy);
    (r, tidy)
}

/// Twice the ring's area (its Newell length) and its width (that over half
/// its perimeter).
fn twice_area_and_width(ring: &[Point3]) -> (f64, f64) {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    let mut perimeter = 0.0;
    for i in 0..ring.len() {
        let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
        n.x += (p.y - q.y) * (p.z + q.z);
        n.y += (p.z - q.z) * (p.x + q.x);
        n.z += (p.x - q.x) * (p.y + q.y);
        perimeter += (q - p).length();
    }
    (n.length(), n.length() / (perimeter / 2.0))
}

/// Every loop (outer and holes) of every face.
fn loops(obj: &Object) -> Vec<Vec<Point3>> {
    obj.faces()
        .values()
        .flat_map(|f| std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()))
        .map(|l| obj.loop_positions(l).collect())
        .collect()
}

/// How many loops are under the old area-only floor.
fn under_old_floor(obj: &Object) -> usize {
    loops(obj)
        .iter()
        .filter(|ring| twice_area_and_width(ring).0 < tol::NORMALIZE_MIN_LENGTH)
        .count()
}

fn volume(obj: &Object) -> f64 {
    obj.volume().expect("a solid")
}

/// Every vertex of the operands that the result keeps is kept bit for bit:
/// no result vertex lies within the merge distance of an operand vertex
/// without being it (a vertex merged or moved would land there; a cut
/// vertex a few nanometers from a removed apex is a different point).
fn operand_vertices_bit_identical(r: &Object, operands: &[&Object], what: &str) {
    for o in operands {
        for p in o.vertices().values().map(|v| v.position) {
            for q in r.vertices().values().map(|v| v.position) {
                if (q - p).length() <= tol::POINT_MERGE {
                    assert_eq!(
                        (q.x.to_bits(), q.y.to_bits(), q.z.to_bits()),
                        (p.x.to_bits(), p.y.to_bits(), p.z.to_bits()),
                        "{what}: operand vertex {p:?} moved to {q:?}"
                    );
                }
            }
        }
    }
}

/// Ok, watertight, valid, reopens from its own encoding to the same bytes,
/// and survives a second boolean and a push/pull of its largest face
/// without a panic.
fn check_result(r: Result<Object, BooleanError>, operands: &[&Object], what: &str) -> Object {
    let r = r.unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert_eq!(r.watertight(), WatertightState::Watertight, "{what}");
    r.validate()
        .unwrap_or_else(|e| panic!("{what}: invalid {e:?}"));
    operand_vertices_bit_identical(&r, operands, what);
    let bytes = r.encode(&|_| 0);
    let back = Object::decode(&bytes, &|_| None).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert_eq!(back.encode(&|_| 0), bytes, "{what}: reopens identically");

    // Later work on the result answers, Ok or typed, never a panic.
    let other = Object::from_polygons(
        &box_corners()
            .iter()
            .map(|p| {
                Point3::new(
                    p.x * 0.3 + 8.0 * MM,
                    p.y * 0.3 + 8.0 * MM,
                    p.z * 0.3 + 8.0 * MM,
                )
            })
            .collect::<Vec<_>>(),
        &box_faces(),
    )
    .unwrap();
    let _ = run(BooleanOp::Union, &r, &other);
    let biggest = r
        .faces()
        .iter()
        .map(|(id, f)| {
            let ring: Vec<Point3> = r.loop_positions(f.outer_loop).collect();
            (id, twice_area_and_width(&ring).0)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
        .unwrap();
    let mut pushed = r.clone();
    let _ = pushed.push_pull(biggest, 1.0 * MM);
    r
}

// ─────────────────────────────────────────────── 1. caps at or above 0.3 µm are kept

/// All three booleans, both orders of subtract, at every kept height: Ok,
/// solid, valid, reopenable; nothing merged (every cut face is well over
/// the feature floor); the set-algebra volumes add up; and under the old
/// floor's heights some loop really is under it.
#[test]
fn caps_from_three_tenths_of_a_micron_are_kept() {
    let a = native_box();
    for h in KEPT {
        let corners = tet_corners(h, (BOX / 2.0, BOX / 2.0), &Transform::IDENTITY);
        assert!(
            thinnest_cap_face(&corners) >= 1.5 * tol::FEATURE_MIN,
            "h={h:e}: the spec is not on the edge of the feature floor"
        );
        let b = tet(h);
        let what = |op: &str| format!("{op} h={h:e}");
        let mut results = Vec::new();
        for (op, x, y, name) in [
            (BooleanOp::Union, &a, &b, "union"),
            (BooleanOp::Intersect, &a, &b, "intersect"),
            (BooleanOp::Subtract, &a, &b, "box - tet"),
            (BooleanOp::Subtract, &b, &a, "tet - box"),
        ] {
            let (r, tidy) = run_reported(op, x, y);
            let r = check_result(r, &[&a, &b], &what(name));
            assert_eq!(
                tidy,
                BooleanTidy::default(),
                "{}: nothing merged",
                what(name)
            );
            assert!(
                thinnest_cut_face(&r, &[&a, &b]) >= tol::FEATURE_MIN,
                "{}",
                what(name)
            );
            results.push(r);
        }
        let [u, i, ab, ba] = &results[..] else {
            unreachable!()
        };
        let (va, vb, vi) = (volume(&a), volume(&b), volume(i));
        assert!(
            (volume(u) + vi - (va + vb)).abs() <= 1e-15,
            "{}",
            what("u + i")
        );
        assert!((volume(ab) - (va - vi)).abs() <= 1e-15, "{}", what("a - b"));
        assert!((volume(ba) - (vb - vi)).abs() <= 1e-15, "{}", what("b - a"));
        if h < 1e-6 {
            // 3e-6 and 1e-6 are controls: their caps clear the old floor.
            for (r, name) in [
                (u, "union"),
                (i, "intersect"),
                (ab, "box - tet"),
                (ba, "tet - box"),
            ] {
                assert!(
                    under_old_floor(r) > 0,
                    "{}: exercises the width rule",
                    what(name)
                );
            }
        }
    }
}

/// `tet - box` at 0.3 µm is the cap by itself: four faces, every one under
/// the old floor, a whole object made of them. It saves and reopens.
#[test]
fn the_sub_micron_cap_alone_is_a_solid_that_reopens() {
    let (a, b) = (native_box(), tet(3e-7));
    let cap = check_result(run(BooleanOp::Subtract, &b, &a), &[&a, &b], "cap");
    assert_eq!(cap.faces().len(), 4);
    for f in cap.faces().values() {
        let ring: Vec<Point3> = cap.loop_positions(f.outer_loop).collect();
        let (twice_area, width) = twice_area_and_width(&ring);
        assert!(twice_area < tol::NORMALIZE_MIN_LENGTH, "2A {twice_area:e}");
        assert!(width >= tol::FEATURE_MIN, "width {width:e}");
        assert!(Plane::from_polygon_import(&ring).is_err());
    }
    assert!(volume(&cap) > 0.0);
}

/// Through a document: commits, saves and reopens to the same state hash,
/// and undo returns to the starting hash.
#[test]
fn band_results_commit_save_reopen_and_undo() {
    for (h, op, swap, expect_ok) in [
        (3e-7, BooleanOp::Union, false, true),
        (3e-7, BooleanOp::Intersect, false, true),
        (3e-7, BooleanOp::Subtract, false, true),
        (3e-7, BooleanOp::Subtract, true, true),
        (5e-8, BooleanOp::Union, false, true),
        (5e-8, BooleanOp::Intersect, false, true),
        (5e-8, BooleanOp::Subtract, false, true),
        (5e-8, BooleanOp::Subtract, true, false),
    ] {
        let what = format!("{op:?} swap {swap} h={h:e}");
        let mut doc = Document::new();
        ingest(&mut doc, "Box", box_corners(), box_faces());
        ingest(
            &mut doc,
            "Tet",
            tet_corners(h, (BOX / 2.0, BOX / 2.0), &Transform::IDENTITY),
            tet_faces(),
        );
        let ids = doc.visible_object_ids();
        let (a, b) = if swap {
            (ids[1], ids[0])
        } else {
            (ids[0], ids[1])
        };
        let before = doc.state_hash();
        let r = doc.boolean(op, a, b);
        if !expect_ok {
            // The tetrahedron minus the box under the feature floor: the cap
            // merges into nothing.
            assert!(
                matches!(
                    r,
                    Err(kernel::DocumentError::Boolean(BooleanError::EmptyResult))
                ),
                "{what}: {r:?}"
            );
            assert_eq!(doc.state_hash(), before, "{what}: untouched");
            continue;
        }
        let (id, _) = r.unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(doc.object_solid(id), "{what}");
        let reopened = Document::load(&doc.save()).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert_eq!(reopened.state_hash(), doc.state_hash(), "{what}: reopens");
        doc.undo().unwrap();
        assert_eq!(doc.state_hash(), before, "{what}: undo");
    }
}

fn ingest(doc: &mut Document, name: &str, positions: Vec<Point3>, faces: Vec<Vec<usize>>) {
    let count = faces.len();
    let mesh = MeshRecipe {
        name: name.into(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; count],
        face_uv_frames: vec![None; count],
        face_holes: vec![vec![]; count],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: vec![],
        extra: Default::default(),
    };
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(mesh)],
        guides: vec![],
        tags: vec![],
        camera: None,
        scenes: vec![],
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    doc.ingest(scene, vec![]).unwrap();
}

// ─────────────────────────────────────────────── 2. caps under 0.1 µm are merged away

/// The thinnest loop of `r` holding a vertex that is no vertex of the
/// operands (a loop the cut made), or infinity when there is none.
fn thinnest_cut_face(r: &Object, operands: &[&Object]) -> f64 {
    let bits = |p: Point3| (p.x.to_bits(), p.y.to_bits(), p.z.to_bits());
    let originals: std::collections::BTreeSet<_> = operands
        .iter()
        .flat_map(|o| o.vertices().values().map(|v| bits(v.position)))
        .collect();
    loops(r)
        .iter()
        .filter(|ring| ring.iter().any(|&p| !originals.contains(&bits(p))))
        .map(|ring| twice_area_and_width(ring).1)
        .fold(f64::INFINITY, f64::min)
}

/// Every vertex position of `obj`, by bits, sorted.
fn vertex_bits(obj: &Object) -> Vec<(u64, u64, u64)> {
    let mut v: Vec<_> = obj
        .vertices()
        .values()
        .map(|v| {
            let p = v.position;
            (p.x.to_bits(), p.y.to_bits(), p.z.to_bits())
        })
        .collect();
    v.sort_unstable();
    v
}

/// A tidied result: [`check_result`]'s checks, the tidy merged something
/// (unless `merged_by_weld`: a cap inside the merge distance, which the
/// arrangement's own weld at `tol::POINT_MERGE` already closes), kept
/// nothing, moved no point past its reach, and left no cut face under the
/// feature floor.
fn check_tidied(
    r: Result<Object, BooleanError>,
    tidy: &BooleanTidy,
    operands: &[&Object],
    what: &str,
) -> Object {
    let r = check_result(r, operands, what);
    if tidy.slivers_merged == 0 {
        assert_eq!(tidy, &BooleanTidy::default(), "{what}");
    } else {
        assert!(
            tidy.sites_total > 0 && !tidy.sites.is_empty(),
            "{what}: {tidy:?}"
        );
    }
    assert_eq!(tidy.slivers_kept, 0, "{what}: {tidy:?}");
    assert!(tidy.max_move <= tol::TIDY_REACH, "{what}: {tidy:?}");
    let thinnest = thinnest_cut_face(&r, operands);
    assert!(
        thinnest >= tol::FEATURE_MIN,
        "{what}: a cut face {thinnest:e} wide is left"
    );
    r
}

/// Under the feature floor the cap is merged into whichever solid it pokes
/// through, at every height down to inside the merge distance: the union
/// is the box and the intersection the tetrahedron, vertex for vertex; the
/// box minus the tetrahedron is the box with the tetrahedron as a void
/// (two shells); the tetrahedron minus the box is empty. Each runs twice
/// to the same bytes. Above the merge distance the tidy does the merging
/// and says so; at 0.5 nm the arrangement's own weld already has.
#[test]
fn caps_under_a_tenth_of_a_micron_are_merged_away() {
    let a = native_box();
    for h in TIDIED {
        let b = tet(h);
        let what = |op: &str| format!("{op} h={h:e}");
        let (va, vb) = (volume(&a), volume(&b));
        let tidied = |tidy: &BooleanTidy, op: &str| {
            assert_eq!(
                tidy.slivers_merged > 0,
                h > tol::POINT_MERGE,
                "{}: {tidy:?}",
                what(op)
            );
        };

        let (r, tidy) = run_reported(BooleanOp::Union, &a, &b);
        tidied(&tidy, "union");
        let u = check_tidied(r, &tidy, &[&a, &b], &what("union"));
        assert_eq!(vertex_bits(&u), vertex_bits(&a), "{}", what("union"));
        assert_eq!(u.faces().len(), 6, "{}", what("union"));
        assert!((volume(&u) - va).abs() <= 1e-15, "{}", what("union"));

        let (r, tidy) = run_reported(BooleanOp::Intersect, &a, &b);
        tidied(&tidy, "intersect");
        let i = check_tidied(r, &tidy, &[&a, &b], &what("intersect"));
        assert_eq!(vertex_bits(&i), vertex_bits(&b), "{}", what("intersect"));
        assert_eq!(i.faces().len(), 4, "{}", what("intersect"));
        assert!((volume(&i) - vb).abs() <= 1e-15, "{}", what("intersect"));

        let (r, tidy) = run_reported(BooleanOp::Subtract, &a, &b);
        tidied(&tidy, "box - tet");
        let ab = check_tidied(r, &tidy, &[&a, &b], &what("box - tet"));
        assert_eq!(ab.shells().len(), 2, "{}: a void", what("box - tet"));
        assert!(
            (volume(&ab) - (va - vb)).abs() <= 1e-15,
            "{}",
            what("box - tet")
        );

        assert_eq!(
            run(BooleanOp::Subtract, &b, &a).err(),
            Some(BooleanError::EmptyResult),
            "{}",
            what("tet - box")
        );

        for (op, x, y) in [
            (BooleanOp::Union, &a, &b),
            (BooleanOp::Intersect, &a, &b),
            (BooleanOp::Subtract, &a, &b),
        ] {
            let one = run(op, x, y).unwrap().encode(&|_| 0);
            let two = run(op, x, y).unwrap().encode(&|_| 0);
            assert_eq!(one, two, "{op:?} h={h:e}: deterministic");
        }
    }
}

/// At 5 nm, through a document: every result commits, saves and reopens to
/// the same state hash, and undo returns to the start; the tetrahedron
/// minus the box refuses `EmptyResult` and leaves the document untouched.
#[test]
fn a_cap_five_nanometers_high_is_merged_away() {
    for (op, swap) in [
        (BooleanOp::Union, false),
        (BooleanOp::Intersect, false),
        (BooleanOp::Subtract, false),
        (BooleanOp::Subtract, true),
    ] {
        let what = format!("{op:?} swap {swap}");
        let mut doc = Document::new();
        ingest(&mut doc, "Box", box_corners(), box_faces());
        ingest(
            &mut doc,
            "Tet",
            tet_corners(5e-9, (BOX / 2.0, BOX / 2.0), &Transform::IDENTITY),
            tet_faces(),
        );
        let ids = doc.visible_object_ids();
        let (a, b) = if swap {
            (ids[1], ids[0])
        } else {
            (ids[0], ids[1])
        };
        let before = doc.state_hash();
        match doc.boolean(op, a, b) {
            Ok((id, _)) => {
                assert!(!swap, "{what}: expected EmptyResult");
                assert!(doc.object_solid(id), "{what}");
                let reopened =
                    Document::load(&doc.save()).unwrap_or_else(|e| panic!("{what}: {e:?}"));
                assert_eq!(reopened.state_hash(), doc.state_hash(), "{what}: reopens");
                doc.undo().unwrap();
                assert_eq!(doc.state_hash(), before, "{what}: undo");
            }
            Err(e) => {
                assert!(swap, "{what}: {e:?}");
                assert!(
                    matches!(e, kernel::DocumentError::Boolean(BooleanError::EmptyResult)),
                    "{what}: {e:?}"
                );
                assert_eq!(doc.state_hash(), before, "{what}: untouched");
            }
        }
    }
}

/// A tetrahedron with its apex at `apex` and its base an equilateral
/// triangle of circumradius `r` around `base`, square to the axis, wound
/// outward.
fn tet_toward(apex: Point3, base: Point3, r: f64) -> Object {
    let axis = (apex - base).normalized().unwrap();
    let helper = if axis.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = axis.cross(helper).normalized().unwrap();
    let v = axis.cross(u);
    let mut pts: Vec<Point3> = [90.0f64, 210.0, 330.0]
        .iter()
        .map(|d| {
            let t = d.to_radians();
            base + u * (r * trig::cos(t)) + v * (r * trig::sin(t))
        })
        .collect();
    pts.push(apex);
    let obj = Object::from_polygons(&pts, &tet_faces()).unwrap();
    if obj.volume().unwrap() > 0.0 {
        obj
    } else {
        let flipped: Vec<Vec<usize>> = tet_faces()
            .into_iter()
            .map(|mut f| {
                f.reverse();
                f
            })
            .collect();
        Object::from_polygons(&pts, &flipped).unwrap()
    }
}

/// The apex 5 nm through a side wall, and 5 nm past an edge (outside both
/// faces that meet there). Through the wall the outcomes are those through
/// the top: nothing leaves its plane, so the union is the box and the
/// intersection the tetrahedron, vertex for vertex. Past the edge the apex
/// is the anchor the cut points weld to, so the box's faces there take it
/// as a corner a few nanometers off their planes and are split into flat
/// triangles; the volume moves by no more than their area times the reach.
///
/// One known limit, before the tidy: past the edge, the intersection's
/// pieces of the box inside the tetrahedron are about 3 nm across and sit
/// within a few nanometers of the apex, where the arrangement's ray cast
/// cannot decide whether they are inside it, so they are dropped and the
/// weld is refused `DegenerateContact`, as it was without the tidy. At
/// 20 nm past the edge it resolves.
#[test]
fn caps_through_a_wall_and_past_an_edge_are_merged_away() {
    let a = native_box();
    let r = BASE / 3f64.sqrt();
    let d = 5e-9 / 2f64.sqrt();
    for (b, exact, name) in [
        (
            tet_toward(
                Point3::new(BOX + 5e-9, BOX / 2.0, BOX / 2.0),
                Point3::new(BOX - DEPTH, BOX / 2.0, BOX / 2.0),
                r,
            ),
            true,
            "wall",
        ),
        (
            tet_toward(
                Point3::new(BOX + d, BOX / 2.0, BOX + d),
                Point3::new(BOX - 3.5 * MM, BOX / 2.0, BOX - 3.5 * MM),
                r,
            ),
            false,
            "edge",
        ),
    ] {
        let what = |op: &str| format!("{op} ({name})");
        let (va, vb) = (volume(&a), volume(&b));
        let slack = |r: &Object| r.surface_area() * tol::TIDY_REACH;

        let (r, tidy) = run_reported(BooleanOp::Union, &a, &b);
        let u = check_tidied(r, &tidy, &[&a, &b], &what("union"));
        if exact {
            assert_eq!(vertex_bits(&u), vertex_bits(&a), "{}", what("union"));
            assert_eq!(tidy.faces_split, 0, "{}", what("union"));
        }
        assert!(
            (volume(&u) - va).abs() <= slack(&u).max(1e-15),
            "{}",
            what("union")
        );

        let (r, tidy) = run_reported(BooleanOp::Intersect, &a, &b);
        if exact {
            let i = check_tidied(r, &tidy, &[&a, &b], &what("intersect"));
            assert_eq!(vertex_bits(&i), vertex_bits(&b), "{}", what("intersect"));
            assert!((volume(&i) - vb).abs() <= 1e-15, "{}", what("intersect"));
        } else {
            assert_eq!(
                r.err(),
                Some(BooleanError::DegenerateContact),
                "{}: the known limit",
                what("intersect")
            );
        }

        let (r, tidy) = run_reported(BooleanOp::Subtract, &a, &b);
        let ab = check_tidied(r, &tidy, &[&a, &b], &what("box - tet"));
        assert!(
            (volume(&ab) - (va - vb)).abs() <= slack(&ab).max(1e-15),
            "{}",
            what("box - tet")
        );

        assert_eq!(
            run(BooleanOp::Subtract, &b, &a).err(),
            Some(BooleanError::EmptyResult),
            "{}",
            what("tet - box")
        );
    }
}

/// Each kept case run twice in one process gives the same bytes.
#[test]
fn band_results_are_deterministic() {
    let a = native_box();
    for h in KEPT {
        let b = tet(h);
        for (op, x, y) in [
            (BooleanOp::Union, &a, &b),
            (BooleanOp::Intersect, &a, &b),
            (BooleanOp::Subtract, &a, &b),
            (BooleanOp::Subtract, &b, &a),
        ] {
            let one = run(op, x, y).unwrap().encode(&|_| 0);
            let two = run(op, x, y).unwrap().encode(&|_| 0);
            assert_eq!(one, two, "{op:?} h={h:e}");
        }
    }
}

// ─────────────────────────────────────────────── 3. properties

/// The cap's own faces, computed from the tetrahedron's corners: the
/// cross-section where each side edge meets the top, and the three side
/// triangles up to the apex. Their thinnest width decides whether the cap
/// is kept exactly or merged away.
fn thinnest_cap_face(corners: &[Point3]) -> f64 {
    let apex = corners[3];
    let cut: Vec<Point3> = corners[..3]
        .iter()
        .map(|&p| {
            let t = (BOX - p.z) / (apex.z - p.z);
            p + (apex - p) * t
        })
        .collect();
    let mut thinnest = twice_area_and_width(&cut).1;
    for i in 0..3 {
        let side = [cut[i], cut[(i + 1) % 3], apex];
        thinnest = thinnest.min(twice_area_and_width(&side).1);
    }
    thinnest
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// A cap of any height in [1 nm, 10 µm], the apex anywhere over the
    /// middle of the top, the tetrahedron turned a little: a solid that
    /// reopens, or a refusal for a cap that touches, misses or swallows
    /// (`DegenerateContact`, `NothingRemoved`, `EmptyResult`), never a
    /// panic; and an Ok result leaves no cut face under the feature floor
    /// unless the tidy says it kept one.
    ///
    /// `ResultTooThin` and `ArrangementFailed` stay possible for a boolean
    /// in general (the first only where the tidy and the untidied faces
    /// both fail to give a valid solid), so this does not claim the kernel
    /// never returns them. What it checks is narrower and stronger: for a
    /// cap on a box the tidy always gives a valid solid, so either one here
    /// is a finding about the tidy, not an accepted refusal.
    #[test]
    fn caps_in_the_band_resolve(
        h in 1e-9f64..1e-5,
        (ax, ay) in (3.5e-3f64..6.5e-3, 3.5e-3f64..6.5e-3),
        (rx, ry, rz) in (-0.02f64..0.02, -0.02f64..0.02, -0.5f64..0.5),
    ) {
        let centre = Point3::new(BOX / 2.0, BOX / 2.0, BOX - DEPTH / 2.0);
        let about = |axis: Vec3, angle: f64| {
            Transform::translation(Point3::ORIGIN - centre)
                .then(&Transform::rotation(axis, angle).unwrap())
                .then(&Transform::translation(centre - Point3::ORIGIN))
        };
        let turn = about(Vec3::new(1.0, 0.0, 0.0), rx)
            .then(&about(Vec3::new(0.0, 1.0, 0.0), ry))
            .then(&about(Vec3::new(0.0, 0.0, 1.0), rz));
        // The apex is placed after the turn so its height over the top is h.
        let mut corners = tet_corners(h, (ax, ay), &turn);
        corners[3] = Point3::new(ax, ay, BOX + h);
        let b = Object::from_polygons(&corners, &tet_faces()).unwrap();
        let a = native_box();
        for (op, x, y) in [
            (BooleanOp::Union, &a, &b),
            (BooleanOp::Intersect, &a, &b),
            (BooleanOp::Subtract, &a, &b),
            (BooleanOp::Subtract, &b, &a),
        ] {
            let (r, tidy) = run_reported(op, x, y);
            match r {
                Ok(r) => {
                    prop_assert_eq!(r.watertight(), WatertightState::Watertight);
                    prop_assert!(r.validate().is_ok());
                    let bytes = r.encode(&|_| 0);
                    let back = Object::decode(&bytes, &|_| None);
                    prop_assert!(back.is_ok(), "{:?} reopens: {:?}", op, back.err());
                    prop_assert!(tidy.max_move <= tol::TIDY_REACH);
                    if tidy.slivers_kept == 0 {
                        let thinnest = thinnest_cut_face(&r, &[&a, &b]);
                        prop_assert!(
                            thinnest >= tol::FEATURE_MIN,
                            "{:?}: a cut face {:e} wide is left ({:?})", op, thinnest, tidy
                        );
                    }
                }
                Err(e) => prop_assert!(
                    matches!(
                        e,
                        BooleanError::DegenerateContact
                            | BooleanError::NothingRemoved
                            | BooleanError::EmptyResult
                    ),
                    "{:?}: {:?}, which the tidy should have resolved for a cap on a box", op, e
                ),
            }
        }
    }

    /// For any ring over the area floor the plane is today's, bit for bit;
    /// for any ring in the width band the normal is unit and every vertex
    /// of a flat ring lies on the plane.
    #[test]
    fn plane_from_polygon_agrees_with_the_area_rule_and_holds_in_the_band(
        origin in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
        base in 1e-7f64..1e-4,
        height_frac in 0.0f64..1.0,
        skew in -0.5f64..1.5,
        (theta, phi) in (0.0f64..std::f64::consts::TAU, 0.0f64..std::f64::consts::PI),
    ) {
        // A triangle in a random plane: base along `u`, apex `height` along
        // `v`, the apex foot `skew` of the way along the base.
        let n = Vec3::new(trig::sin(phi) * trig::cos(theta), trig::sin(phi) * trig::sin(theta), trig::cos(phi));
        let helper = if n.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
        let u = n.cross(helper).normalized().unwrap();
        let v = n.cross(u);
        let o = Point3::new(origin.0, origin.1, origin.2);
        // Heights from 10 nm up to past the area floor for this base.
        let lo = tol::MIN_FACE_WIDTH * 1.01;
        let hi = (4.0 * tol::NORMALIZE_MIN_LENGTH / base).max(lo * 2.0);
        let height = lo + (hi - lo) * height_frac;
        let ring = [o, o + u * base, o + u * (base * skew) + v * height];
        let (twice_area, width) = twice_area_and_width(&ring);
        if twice_area >= tol::NORMALIZE_MIN_LENGTH {
            prop_assert_eq!(Plane::from_polygon(&ring), Plane::from_polygon_import(&ring));
        } else if width >= tol::MIN_FACE_WIDTH {
            let plane = Plane::from_polygon(&ring);
            prop_assert!(plane.is_ok());
            let plane = plane.unwrap();
            prop_assert!((plane.normal().length() - 1.0).abs() <= 1e-15);
            for p in ring {
                prop_assert!(plane.signed_distance(p).abs() <= tol::PLANE_DIST);
            }
        }
    }
}
