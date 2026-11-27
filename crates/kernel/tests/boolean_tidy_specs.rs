//! A boolean tidies away the slivers its own cut leaves (KB-25).
//!
//! A cut through a dense mesh leaves faces far below a micron where it
//! passes near a mesh vertex or edge. The boolean merges every face its own
//! cut leaves narrower than `tol::FEATURE_MIN` (0.1 µm) into its
//! neighbors, moving no point farther than `tol::TIDY_REACH`, and reports
//! it (`BooleanTidy`); faces it does not cut pass through bit for bit.
//!
//! The dense mesh is an icosphere of 2,562 vertices, 10 mm across, built
//! through the import builder as an STL import would be: the same ground
//! as the ignored pawn test (`crates/stl-import/tests/boolean_pawn.rs`),
//! with no file outside the repository.

use kernel::trig;
use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    BooleanError, BooleanOp, BooleanTidy, Document, ImportNode, ImportScene, MeshRecipe, Object,
    Point3, Transform, Vec3, WatertightState, tol,
};
use proptest::prelude::*;

const MM: f64 = 1e-3;
const RADIUS: f64 = 5.0 * MM;
/// Where the sphere's center sits, off every axis plane.
const CENTER: Point3 = Point3::new(2.0 * MM, 3.0 * MM, 7.0 * MM);

/// The aimed distances (nm), either side of the plane.
const AIMED_NM: [f64; 8] = [0.5, 2.0, 5.0, 11.4, 30.0, 80.0, 150.0, 400.0];

/// An icosphere: an icosahedron subdivided four times (2,562 vertices,
/// 5,120 triangles), turned off-axis, every vertex on the sphere around
/// [`CENTER`]. Triangles wound outward.
fn icosphere() -> (Vec<Point3>, Vec<Vec<usize>>) {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut pts: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalized().unwrap())
    .collect();
    let mut tris: Vec<[usize; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..4 {
        let mut mid: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        let mut midpoint = |a: usize, b: usize, pts: &mut Vec<Vec3>| {
            *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
                pts.push(((pts[a] + pts[b]) * 0.5).normalized().unwrap());
                pts.len() - 1
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let ab = midpoint(a, b, &mut pts);
            let bc = midpoint(b, c, &mut pts);
            let ca = midpoint(c, a, &mut pts);
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    let turn = Transform::rotation(Vec3::new(0.3, 0.5, 0.8), 0.37).unwrap();
    let positions = pts
        .iter()
        .map(|&v| {
            let p = turn.apply_point(Point3::ORIGIN + v * RADIUS);
            p + (CENTER - Point3::ORIGIN)
        })
        .collect();
    (positions, tris.into_iter().map(|t| t.to_vec()).collect())
}

fn sphere() -> Object {
    let (pts, faces) = icosphere();
    let obj = Object::from_polygons_import_for_test(&pts, &faces).unwrap();
    assert!(obj.volume().unwrap() > 0.0, "wound outward");
    obj
}

/// A native box from `lo` to `hi`, its frame turned by `t` about `lo`.
fn native_box(lo: Point3, hi: Point3, t: &Transform) -> Object {
    let c = [
        Point3::new(lo.x, lo.y, lo.z),
        Point3::new(hi.x, lo.y, lo.z),
        Point3::new(hi.x, hi.y, lo.z),
        Point3::new(lo.x, hi.y, lo.z),
        Point3::new(lo.x, lo.y, hi.z),
        Point3::new(hi.x, lo.y, hi.z),
        Point3::new(hi.x, hi.y, hi.z),
        Point3::new(lo.x, hi.y, hi.z),
    ];
    let about = Transform::translation(Point3::ORIGIN - lo)
        .then(t)
        .then(&Transform::translation(lo - Point3::ORIGIN));
    let pts: Vec<Point3> = c.iter().map(|&p| about.apply_point(p)).collect();
    Object::from_polygons(&pts, &box_faces()).unwrap()
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

fn run(op: BooleanOp, a: &Object, b: &Object) -> (Result<Object, BooleanError>, BooleanTidy) {
    let mut tidy = BooleanTidy::default();
    let r = Object::boolean_reported(op, a, b, &Transform::IDENTITY, &mut tidy);
    (r, tidy)
}

type Bits = (u64, u64, u64);

fn bits(p: Point3) -> Bits {
    (p.x.to_bits(), p.y.to_bits(), p.z.to_bits())
}

fn vertex_bits(obj: &Object) -> BTreeSet<Bits> {
    obj.vertices().values().map(|v| bits(v.position)).collect()
}

/// A ring's width: its Newell length over half its perimeter.
fn width(ring: &[Point3]) -> f64 {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    let mut perimeter = 0.0;
    for i in 0..ring.len() {
        let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
        n.x += (p.y - q.y) * (p.z + q.z);
        n.y += (p.z - q.z) * (p.x + q.x);
        n.z += (p.x - q.x) * (p.y + q.y);
        perimeter += (q - p).length();
    }
    n.length() / (perimeter / 2.0)
}

/// The thinnest loop of `r` holding a point that is no vertex of
/// `operands`: a loop the cut made.
fn thinnest_cut_face(r: &Object, operands: &BTreeSet<Bits>) -> f64 {
    let mut thinnest = f64::INFINITY;
    for f in r.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let ring: Vec<Point3> = r.loop_positions(l).collect();
            if ring.iter().any(|&p| !operands.contains(&bits(p))) {
                thinnest = thinnest.min(width(&ring));
            }
        }
    }
    thinnest
}

/// Solid, valid, reopens from its own encoding to the same bytes and
/// planes; the tidy within its bounds and keeping nothing; no cut face
/// under the feature floor; and every operand vertex it keeps kept bit for
/// bit (no result vertex within the merge distance of one without being
/// it).
fn check(r: &Object, tidy: &BooleanTidy, operands: &[&Object], what: &str) {
    assert_eq!(r.watertight(), WatertightState::Watertight, "{what}");
    r.validate()
        .unwrap_or_else(|e| panic!("{what}: invalid {e:?}"));
    let bytes = r.encode(&|_| 0);
    let back = Object::decode(&bytes, &|_| None).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert_eq!(back.encode(&|_| 0), bytes, "{what}: reopens identically");
    assert_eq!(
        planes(&back),
        planes(r),
        "{what}: reopens on the same planes"
    );
    assert!(tidy.max_move <= tol::TIDY_REACH, "{what}: {tidy:?}");
    assert_eq!(tidy.slivers_kept, 0, "{what}: {tidy:?}");
    let originals: BTreeSet<Bits> = operands.iter().flat_map(|o| vertex_bits(o)).collect();
    let thinnest = thinnest_cut_face(r, &originals);
    assert!(
        thinnest >= tol::FEATURE_MIN,
        "{what}: a cut face {thinnest:e} wide is left"
    );
    let originals: Vec<Point3> = operands
        .iter()
        .flat_map(|o| o.vertices().values().map(|v| v.position))
        .collect();
    for q in r.vertices().values().map(|v| v.position) {
        for &p in &originals {
            if (q - p).length() <= tol::POINT_MERGE {
                assert_eq!(bits(q), bits(p), "{what}: operand vertex {p:?} moved");
            }
        }
    }
}

/// The three booleans of `a` and `b`, checked, with the set-algebra
/// volumes: exact to rounding when nothing was tidied, else within the
/// operands' area times the reach.
fn all_three(a: &Object, b: &Object, what: &str) -> [BooleanTidy; 3] {
    let mut tidies = Vec::new();
    let mut volumes = Vec::new();
    for (op, name) in [
        (BooleanOp::Union, "union"),
        (BooleanOp::Subtract, "subtract"),
        (BooleanOp::Intersect, "intersect"),
    ] {
        let (r, tidy) = run(op, a, b);
        let r = r.unwrap_or_else(|e| panic!("{what} {name}: {e:?}"));
        check(&r, &tidy, &[a, b], &format!("{what} {name}"));
        volumes.push(r.volume().unwrap());
        tidies.push(tidy);
    }
    let (va, vb) = (a.volume().unwrap(), b.volume().unwrap());
    let slack = if tidies.iter().all(BooleanTidy::is_empty) {
        1e-15
    } else {
        (a.surface_area() + b.surface_area()) * tol::TIDY_REACH
    };
    assert!(
        (volumes[0] + volumes[2] - (va + vb)).abs() <= slack,
        "{what}: |A∪B| + |A∩B| = |A| + |B|"
    );
    assert!(
        (volumes[1] - (va - volumes[2])).abs() <= slack,
        "{what}: |A−B| = |A| − |A∩B|"
    );
    [tidies[0].clone(), tidies[1].clone(), tidies[2].clone()]
}

/// The sphere vertex nearest the point 60° down from its top toward +x:
/// mid-latitude, where the surface is neither flat nor steep under a
/// horizontal cut.
fn aimed_vertex(s: &Object) -> Point3 {
    let a = 60f64.to_radians();
    let target = CENTER + Vec3::new(trig::sin(a), 0.0, trig::cos(a)) * RADIUS;
    s.vertices()
        .values()
        .map(|v| v.position)
        .min_by(|p, q| (*p - target).length().total_cmp(&(*q - target).length()))
        .unwrap()
}

/// A box 4 mm across around `v`, its top `d` above `v`.
fn box_topped_at(v: Point3, d: f64) -> Object {
    native_box(
        Point3::new(v.x - 2.0 * MM, v.y - 2.0 * MM, CENTER.z - 7.0 * MM),
        Point3::new(v.x + 2.0 * MM, v.y + 2.0 * MM, v.z + d),
        &Transform::IDENTITY,
    )
}

// ─────────────────────────────────────────────── 1. aimed at a vertex

/// The box's top plane put 2 to 400 nm from a mesh vertex, either side,
/// for union, subtract and intersect: every one commits, solid and valid,
/// with no cut face under the feature floor. Up to 80 nm away a sliver is
/// merged; at 400 nm nothing is under the floor and nothing is touched.
///
/// At 0.5 nm, inside the merge distance, is a known limit of the
/// arrangement (the refusals are a lumpy set of offsets within about 1.2 nm
/// of a vertex, some past the 1 nm merge distance, listed in
/// `aimed_cuts_in_the_refused_band_are_pinned`; the sharp rows are
/// `aimed_cuts_within_a_tenth_of_a_nanometer_commit` and
/// `aimed_cuts_in_the_refused_band_are_pinned`), before any tidy: the cut crosses the mesh faces around
/// the vertex in under a nanometer, `seam_segments` drops those pieces,
/// and the chain of seams on the box top is left with two ends 1 to 2 nm
/// apart, too far for the sketch to join, so the top is not split and the
/// weld is refused. Pinned as solid or one of those two refusals.
#[test]
fn cuts_aimed_at_a_mesh_vertex_commit() {
    let s = sphere();
    let v = aimed_vertex(&s);
    for d_nm in AIMED_NM.iter().flat_map(|&d| [-d, d]) {
        let b = box_topped_at(v, d_nm * 1e-9);
        let what = format!("top {d_nm} nm from the vertex");
        if d_nm.abs() * 1e-9 < tol::POINT_MERGE {
            for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
                match run(op, &s, &b) {
                    (Ok(r), tidy) => check(&r, &tidy, &[&s, &b], &format!("{what} {op:?}")),
                    (Err(e), _) => assert!(
                        matches!(
                            e,
                            BooleanError::DegenerateContact | BooleanError::ResultTooThin
                        ),
                        "{what} {op:?}: {e:?}"
                    ),
                }
            }
            continue;
        }
        let tidies = all_three(&s, &b, &what);
        let any = tidies.iter().any(|t| t.slivers_merged > 0);
        if d_nm.abs() <= 80.0 {
            assert!(any, "{what}: a sliver is merged ({tidies:?})");
        }
        if d_nm.abs() >= 400.0 {
            assert!(
                tidies.iter().all(BooleanTidy::is_empty),
                "{what}: nothing is under the floor ({tidies:?})"
            );
        }
    }
}

/// Each aimed case run twice gives the same bytes.
#[test]
fn aimed_cuts_are_deterministic() {
    let s = sphere();
    let v = aimed_vertex(&s);
    for d_nm in [-5.0, 11.4, 80.0] {
        let b = box_topped_at(v, d_nm * 1e-9);
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            let one = run(op, &s, &b).0.unwrap().encode(&|_| 0);
            let two = run(op, &s, &b).0.unwrap().encode(&|_| 0);
            assert_eq!(one, two, "{op:?} {d_nm} nm");
        }
    }
}

/// Through a document: an aimed union commits, saves and reopens to the
/// same state hash, and undo returns to the start; undo clears a report
/// left unread, and redo reports nothing.
#[test]
fn an_aimed_cut_commits_saves_reopens_and_undoes() {
    let (pts, faces) = icosphere();
    let v = aimed_vertex(&sphere());
    for d_nm in [-5.0, 5.0] {
        let mut doc = Document::new();
        ingest(&mut doc, "Sphere", pts.clone(), faces.clone());
        let b = box_topped_at(v, d_nm * 1e-9);
        let (bp, bf) = b.to_polygons();
        ingest(&mut doc, "Box", bp, bf);
        let ids = doc.visible_object_ids();
        let before = doc.state_hash();
        let mut merged = 0;
        let mut cleared = 0;
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            let what = format!("{op:?} {d_nm} nm");
            let (id, _) = doc
                .boolean(op, ids[0], ids[1])
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert!(doc.object_solid(id), "{what}");
            let tidy = doc.take_boolean_tidy();
            assert!(tidy.max_move <= tol::TIDY_REACH, "{what}: {tidy:?}");
            merged += tidy.slivers_merged;
            let reopened = Document::load(&doc.save()).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(reopened.state_hash(), doc.state_hash(), "{what}: reopens");
            doc.undo().unwrap();
            assert_eq!(doc.state_hash(), before, "{what}: undo");
            if tidy.is_empty() {
                continue;
            }
            // Undo clears a report nobody read (the same boolean again, its
            // report left unread, then undone), and redo does not report
            // the boolean a second time.
            doc.boolean(op, ids[0], ids[1]).unwrap();
            doc.undo().unwrap();
            assert!(doc.take_boolean_tidy().is_empty(), "{what}: undo clears it");
            doc.redo().unwrap();
            assert!(
                doc.take_boolean_tidy().is_empty(),
                "{what}: redo reports nothing"
            );
            doc.undo().unwrap();
            assert_eq!(doc.state_hash(), before, "{what}: undo after redo");
            cleared += 1;
        }
        // The sliver lies on the side the union and one of the others
        // keep; the third keeps none.
        assert!(merged > 0, "{d_nm} nm: a sliver is merged");
        assert!(cleared > 0, "{d_nm} nm: a report undo had to clear");
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

// ─────────────────────────────────────────────── 2. aimed at an edge

/// A box's vertical edge passing 5 nm and 50 nm from a mesh edge, at its
/// middle, far from either end: every boolean commits, with no cut face
/// under the feature floor; at 5 nm a sliver is merged.
#[test]
fn a_cutter_edge_aimed_at_a_mesh_edge_commits() {
    let s = sphere();
    let v = aimed_vertex(&s);
    // The edge out of the aimed vertex closest to horizontal: steep enough
    // ground for a vertical line to cross it cleanly.
    let (p, q) = s
        .edges()
        .keys()
        .filter_map(|e| s.edge_endpoints(e))
        .filter(|&(p, q)| bits(p) == bits(v) || bits(q) == bits(v))
        .min_by(|a, b| {
            let tilt = |(p, q): (Point3, Point3)| ((q - p).normalized().unwrap().z).abs();
            tilt(*a).total_cmp(&tilt(*b))
        })
        .unwrap();
    let m = Point3::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0, (p.z + q.z) / 2.0);
    let e = q - p;
    let w = e.cross(Vec3::new(0.0, 0.0, 1.0)).normalized().unwrap();
    for d in [5e-9, 5e-8] {
        let corner = m + w * d;
        let b = native_box(
            Point3::new(
                corner.x - 4.0 * MM,
                corner.y - 4.0 * MM,
                CENTER.z - 7.0 * MM,
            ),
            Point3::new(corner.x, corner.y, CENTER.z + 7.0 * MM),
            &Transform::IDENTITY,
        );
        let what = format!("edge {d:e} from the mesh edge");
        let tidies = all_three(&s, &b, &what);
        if d < 1e-8 {
            assert!(
                tidies.iter().any(|t| t.slivers_merged > 0),
                "{what}: {tidies:?}"
            );
        }
    }
}

// ─────────────────────────────────────────────── 3. two native operands

/// A native cube turned off every axis, cut by a box whose top plane passes
/// 5 nm above one corner, two of whose edges rise through the plane: the
/// cut points on those edges weld to the corner, which leaves the cut face
/// holding a point 5 nm off its plane, so that face is split into flat
/// triangles with soft diagonals and the result stays at the native
/// planarity tolerance.
#[test]
fn a_native_cut_by_a_corner_splits_the_face_it_bends() {
    let side = 10.0 * MM;
    let turn = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.4).unwrap();
    let cube = native_box(Point3::ORIGIN, Point3::new(side, side, side), &turn);
    // A corner with two edges rising from it and one falling.
    let corner = cube
        .vertices()
        .values()
        .map(|v| v.position)
        .find(|&k| {
            let rising = cube
                .edges()
                .keys()
                .filter_map(|e| cube.edge_endpoints(e))
                .filter_map(|(p, q)| {
                    if bits(p) == bits(k) {
                        Some(q)
                    } else if bits(q) == bits(k) {
                        Some(p)
                    } else {
                        None
                    }
                })
                .filter(|o| o.z > k.z)
                .count();
            rising == 2
        })
        .expect("a corner with two rising edges");
    let cutter = native_box(
        Point3::new(-30.0 * MM, -30.0 * MM, -30.0 * MM),
        Point3::new(30.0 * MM, 30.0 * MM, corner.z + 5e-9),
        &Transform::IDENTITY,
    );
    for (op, a, b, name) in [
        (BooleanOp::Intersect, &cube, &cutter, "cube ∩ cutter"),
        (BooleanOp::Subtract, &cutter, &cube, "cutter − cube"),
    ] {
        let (r, tidy) = run(op, a, b);
        let r = r.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        check(&r, &tidy, &[&cube, &cutter], name);
        assert!(tidy.slivers_merged > 0, "{name}: {tidy:?}");
        assert!(tidy.faces_split >= 1, "{name}: {tidy:?}");
        assert_eq!(r.planarity_tol(), tol::PLANE_DIST, "{name}");
        assert!(
            r.edges().values().any(|e| e.soft),
            "{name}: the split face's diagonals are soft"
        );
    }
}

/// A box 0.8 mm across, its top 80 nm under the sphere vertex nearest
/// latitude 31.5°, longitude 179°: the cutter's box reaches only the faces
/// around it, so the boolean rebuilds those and keeps the rest of the
/// sphere as it was. The tidy folds the cut's slivers onto that vertex
/// and into edges where faces the cutter's box misses (by more than their
/// padding, less than `tol::TIDY_REACH`) hold them too; the rebuilt region
/// grows by those faces, so union and subtract still commit with every
/// sliver merged. Spliced onto the sphere's untouched faces as they were,
/// the tidied faces once left the seam open and the boolean fell back to
/// the untidied faces, keeping the slivers.
#[test]
fn a_tidy_reaching_past_the_rebuilt_region_grows_it() {
    let s = sphere();
    let (lat, lon) = (31.5f64.to_radians(), 179f64.to_radians());
    let target = CENTER
        + Vec3::new(
            trig::cos(lat) * trig::cos(lon),
            trig::cos(lat) * trig::sin(lon),
            trig::sin(lat),
        ) * RADIUS;
    let v = s
        .vertices()
        .values()
        .map(|v| v.position)
        .min_by(|p, q| (*p - target).length().total_cmp(&(*q - target).length()))
        .unwrap();
    let half = 0.4 * MM;
    let b = native_box(
        Point3::new(v.x - half, v.y - half, CENTER.z - 7.0 * MM),
        Point3::new(v.x + half, v.y + half, v.z - 80e-9),
        &Transform::IDENTITY,
    );
    for op in [BooleanOp::Union, BooleanOp::Subtract] {
        let what = format!("{op:?}");
        let (r, tidy) = run(op, &s, &b);
        let r = r.unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(tidy.slivers_merged > 0, "{what}: {tidy:?}");
        check(&r, &tidy, &[&s, &b], &what);
    }
}

/// A cutter wholly inside the dense mesh, whose box meets no mesh face's
/// box (so no face of the mesh is rebuilt), is a real boolean either way
/// round: the union is the mesh unchanged, and the subtract leaves the mesh
/// whole with the cutter's walls as a cavity, short exactly the cutter's
/// volume. Nothing is refused as a touch.
#[test]
fn a_cutter_wholly_inside_the_mesh_meets_no_face_and_commits() {
    let s = sphere();
    let v = s.volume().unwrap();
    let h = 0.5 * MM;
    let d = Vec3::new(h, h, h);
    let b = native_box(CENTER - d, CENTER + d, &Transform::IDENTITY);
    let cube = 8.0 * h * h * h;
    for (a, b) in [(&s, &b), (&b, &s)] {
        let u = run(BooleanOp::Union, a, b).0.unwrap();
        u.validate().unwrap();
        assert_eq!(u.faces().len(), s.faces().len());
        assert!((u.volume().unwrap() - v).abs() < 1e-12 * v);
    }
    let r = run(BooleanOp::Subtract, &s, &b).0.unwrap();
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);
    assert_eq!(r.shells().len(), 2, "the skin and the cavity wall");
    assert!((r.volume().unwrap() - (v - cube)).abs() < 1e-9 * cube);
    let i = run(BooleanOp::Intersect, &s, &b).0.unwrap();
    assert!((i.volume().unwrap() - cube).abs() < 1e-9 * cube);
}

// ─────────────────────────────────────────────── 4. far geometry

/// A native block with a 10 nm chamfer on one vertical edge (a side face
/// about 20 nm wide, its own needle), cut far from it: nothing is tidied,
/// and the needle comes through bit for bit.
#[test]
fn an_operands_own_needle_far_from_the_cut_comes_through_untouched() {
    let (block, pts) = chamfered_block(1e-8);
    let n = pts.len() / 2;
    let needle: Vec<Point3> = vec![pts[1], pts[2], pts[n + 2], pts[n + 1]];
    assert!(width(&needle) < tol::FEATURE_MIN && width(&needle) >= tol::MIN_FACE_WIDTH);
    let cutter = native_box(
        Point3::new(-MM, -MM, 3.0 * MM),
        Point3::new(1.0 * MM, 1.0 * MM, 5.0 * MM),
        &Transform::IDENTITY,
    );
    for op in [BooleanOp::Union, BooleanOp::Subtract] {
        let (r, tidy) = run(op, &block, &cutter);
        let r = r.unwrap_or_else(|e| panic!("{op:?}: {e:?}"));
        assert_eq!(tidy, BooleanTidy::default(), "{op:?}");
        let kept = r.faces().values().any(|f| {
            let ring: Vec<Bits> = r.loop_positions(f.outer_loop).map(bits).collect();
            ring.len() == 4 && needle.iter().all(|&p| ring.contains(&bits(p)))
        });
        assert!(kept, "{op:?}: the needle face comes through");
    }
}

/// A native block 4 mm on a side with a chamfer `c` across one vertical
/// edge, and its corners (bottom ring, then top ring).
fn chamfered_block(c: f64) -> (Object, Vec<Point3>) {
    let (s, h) = (4.0 * MM, 4.0 * MM);
    let profile = [(0.0, 0.0), (s - c, 0.0), (s, c), (s, s), (0.0, s)];
    let n = profile.len();
    let mut pts: Vec<Point3> = profile
        .iter()
        .map(|&(x, y)| Point3::new(x, y, 0.0))
        .collect();
    pts.extend(profile.iter().map(|&(x, y)| Point3::new(x, y, h)));
    let mut faces = vec![(0..n).rev().collect::<Vec<_>>(), (n..2 * n).collect()];
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, j + n, i + n]);
    }
    let block = Object::from_polygons(&pts, &faces).unwrap();
    assert!(block.volume().unwrap() > 0.0);
    (block, pts)
}

/// A tetrahedron pointing straight down at `apex`, its base 3 mm above it.
fn spike_down_to(apex: Point3) -> Object {
    let base = apex + Vec3::new(0.0, 0.0, 3.0 * MM);
    let mut pts: Vec<Point3> = [90.0f64, 210.0, 330.0]
        .iter()
        .map(|d| {
            let t = d.to_radians();
            base + Vec3::new(trig::cos(t), trig::sin(t), 0.0) * MM
        })
        .collect();
    pts.push(apex);
    let faces = vec![vec![0, 1, 2], vec![0, 3, 1], vec![1, 3, 2], vec![2, 3, 0]];
    let tet = Object::from_polygons(&pts, &faces).unwrap();
    assert!(tet.volume().unwrap() > 0.0);
    tet
}

/// A spike 5 nm into the block's top, 20 nm inside a 50 nm chamfer: the
/// cut is all under the feature floor, and the weld that merges it also
/// joins the chamfer's two top corners, 71 nm apart, which are both within
/// reach of the cut. What is left differs from the block only by that
/// move, so the cutter removed nothing (`NothingRemoved`) and the block is
/// not disturbed by a cut that took nothing.
#[test]
fn a_cut_under_the_floor_that_moves_a_vertex_of_the_target_removes_nothing() {
    let c = 5e-8;
    let (block, pts) = chamfered_block(c);
    let n = pts.len() / 2;
    let (p, q) = (pts[n + 1], pts[n + 2]);
    assert!((q - p).length() <= tol::TIDY_REACH);
    let middle = Point3::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0, p.z);
    let inward = Vec3::new(-1.0, 1.0, 0.0).normalized().unwrap();
    for depth in [5e-9, 2e-9] {
        let spike = spike_down_to(middle + inward * 2e-8 - Vec3::new(0.0, 0.0, depth));
        assert_eq!(
            run(BooleanOp::Subtract, &block, &spike).0.err(),
            Some(BooleanError::NothingRemoved),
            "{depth:e} deep"
        );
    }
}

// ─────────────────────────────────────────────── 5. properties

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// A box anywhere across the sphere's surface, turned a little: every
    /// boolean commits or gives one of the refusals a boolean could give
    /// before the tidy other than too thin (`DegenerateContact`,
    /// `NothingRemoved`, `ArrangementFailed`); a result is solid, valid,
    /// reopens, leaves no cut face under the feature floor, and the
    /// set-algebra volumes agree within the operands' area times the reach.
    ///
    /// `ResultTooThin` stays possible for a boolean in general, where the
    /// tidy and the untidied faces both fail to give a valid solid, so this
    /// does not claim the kernel never returns it. What it checks is that
    /// for a box on this mesh the tidy always gives a valid solid, so one
    /// here is a finding about the tidy, not an accepted refusal.
    #[test]
    fn boxes_across_the_dense_mesh_resolve(
        (x, y, z) in (-4.0f64..4.0, -4.0f64..4.0, -4.0f64..4.0),
        (w, d, h) in (1.0f64..5.0, 1.0f64..5.0, 1.0f64..5.0),
        (rx, ry, rz) in (-0.3f64..0.3, -0.3f64..0.3, -0.3f64..0.3),
    ) {
        let s = sphere();
        let lo = CENTER + Vec3::new(x, y, z) * MM;
        let turn = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), rx).unwrap()
            .then(&Transform::rotation(Vec3::new(0.0, 1.0, 0.0), ry).unwrap())
            .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), rz).unwrap());
        let b = native_box(lo, lo + Vec3::new(w, d, h) * MM, &turn);
        let mut volumes = BTreeMap::new();
        let mut tidied = false;
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            let (r, tidy) = run(op, &s, &b);
            tidied |= !tidy.is_empty();
            match r {
                Ok(r) => {
                    check(&r, &tidy, &[&s, &b], &format!("{op:?}"));
                    volumes.insert(format!("{op:?}"), r.volume().unwrap());
                }
                Err(BooleanError::EmptyResult) => {
                    volumes.insert(format!("{op:?}"), 0.0);
                }
                Err(e) => prop_assert!(
                    matches!(
                        e,
                        BooleanError::DegenerateContact
                            | BooleanError::NothingRemoved
                            | BooleanError::ArrangementFailed
                    ),
                    "{:?}: {:?}, which the tidy should have resolved for a box on this mesh", op, e
                ),
            }
        }
        if let (Some(u), Some(i)) = (volumes.get("Union"), volumes.get("Intersect")) {
            let slack = if tidied {
                (s.surface_area() + b.surface_area()) * tol::TIDY_REACH
            } else {
                1e-15
            };
            let (va, vb) = (s.volume().unwrap(), b.volume().unwrap());
            prop_assert!((u + i - (va + vb)).abs() <= slack, "{} + {} vs {} + {}", u, i, va, vb);
        }
    }
}

// ─────────────────────────────────────────────── 6. reopening

/// A native 1 mm cube with a 0.4 mm square pillar cut through it: its top
/// and bottom are holed faces, so a boolean on it builds its result from
/// the planes its faces carry rather than refitting them.
fn holed_cube() -> Object {
    let cube = native_box(
        Point3::ORIGIN,
        Point3::new(MM, MM, MM),
        &Transform::IDENTITY,
    );
    let pillar = native_box(
        Point3::new(0.3 * MM, 0.3 * MM, -0.1 * MM),
        Point3::new(0.7 * MM, 0.7 * MM, 1.1 * MM),
        &Transform::IDENTITY,
    );
    Object::boolean(BooleanOp::Subtract, &cube, &pillar, &Transform::IDENTITY).unwrap()
}

/// Encodes and decodes `r`: it reopens, valid, to the same bytes, every
/// face on the plane it carried, bit for bit.
fn reopens(r: &Object, what: &str) {
    let bytes = r.encode(&|_| 0);
    let back = Object::decode(&bytes, &|_| None).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert_eq!(back.encode(&|_| 0), bytes, "{what}: reopens identically");
    assert_eq!(
        planes(&back),
        planes(r),
        "{what}: reopens on the same planes"
    );
}

/// A face's plane by bits: the normal and the signed distance of the
/// origin.
type PlaneBits = [u64; 4];

/// Every face's plane, keyed by its outer ring's points (by bits) from
/// the ring's smallest point on, so two copies of one object compare
/// equal whatever their slot order.
fn planes(obj: &Object) -> BTreeMap<Vec<Bits>, PlaneBits> {
    obj.faces()
        .values()
        .map(|f| {
            let mut ring: Vec<Bits> = obj.loop_positions(f.outer_loop).map(bits).collect();
            let start = (0..ring.len()).min_by_key(|&i| ring[i]).unwrap();
            ring.rotate_left(start);
            let n = f.plane.normal();
            let d = f.plane.signed_distance(Point3::ORIGIN);
            (
                ring,
                [n.x.to_bits(), n.y.to_bits(), n.z.to_bits(), d.to_bits()],
            )
        })
        .collect()
}

/// `ring` rotated to its lexicographically smallest start, positions
/// compared by `total_cmp` (the canonical start a file writes).
fn canonical(ring: Vec<Point3>) -> Vec<Point3> {
    let n = ring.len();
    let cmp = |a: Point3, b: Point3| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    };
    let start = (0..n)
        .min_by(|&i, &j| {
            (0..n)
                .map(|k| cmp(ring[(i + k) % n], ring[(j + k) % n]))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap();
    (0..n).map(|k| ring[(start + k) % n]).collect()
}

/// How many faces of `obj` have a boundary point farther than the native
/// tolerance from the plane refit from their outer ring (Newell's method
/// from the canonical start, what a geometry v9 reader gives every face):
/// faces a v9 reader would reject as not flat.
fn faces_off_their_refit(obj: &Object) -> usize {
    obj.faces()
        .values()
        .filter(|f| {
            let ring = canonical(obj.loop_positions(f.outer_loop).collect());
            let Ok(refit) = kernel::Plane::from_polygon(&ring) else {
                return true;
            };
            std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .flat_map(|l| obj.loop_positions(l))
                .any(|p| refit.signed_distance(p).abs() > tol::PLANE_DIST)
        })
        .count()
}

/// A box flush with the pillar's wall to 2 nm and 0.3 µm below the cube's
/// floor, its top 27 nm under the cube's top: the cut leaves a sliver whose
/// points weld to the cube's corners, which bends the face around them by
/// a nanometer. Every point stays within the native tolerance of the plane
/// the face carries, but not of the plane its outer ring refits to, so a
/// file that stored no plane could not reopen it whole. The file stores
/// the plane (geometry v10), so the face is kept whole and the result
/// reopens on its planes.
#[test]
fn a_tidied_face_off_its_refit_plane_is_kept_and_reopens() {
    let a = holed_cube();
    let b = native_box(
        Point3::new(
            0.000_300_002_037_779_234_7,
            2.005_080_117_152_133_8e-9,
            -3.014_552_702_171_446e-7,
        ),
        Point3::new(
            0.000_700_000_985_988_888_7,
            0.000_643_273_669_279_711_7,
            0.000_999_973_090_409_422_3,
        ),
        &Transform::IDENTITY,
    );
    for op in [BooleanOp::Subtract, BooleanOp::Union] {
        let what = format!("{op:?}");
        let (r, tidy) = run(op, &a, &b);
        let r = r.unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(
            tidy.slivers_merged > 0 && tidy.slivers_kept == 0,
            "{what}: {tidy:?}"
        );
        assert!(
            faces_off_their_refit(&r) > 0,
            "{what}: the bent face is kept"
        );
        r.validate().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        reopens(&r, &what);
    }
}

/// The same gap with no tidy at all: a box whose faces pass a few
/// nanometers from the cube's walls cuts the holed top with points up to
/// the tolerance off its plane on both sides, so the plane the face carries
/// is not the one its outer ring refits to (a geometry v9 file, which
/// stored no planes, could not reopen it). The file stores the plane
/// (geometry v10), so the result commits and reopens on its planes, bit
/// for bit.
#[test]
fn a_result_off_its_refit_plane_commits_and_reopens_on_its_planes() {
    let a = holed_cube();
    let b = native_box(
        Point3::new(
            3.654_319_374_300_190_8e-9,
            0.000_948_560_401_208_869_7,
            4.684_192_556_939_707e-10,
        ),
        Point3::new(
            0.001_000_000_641_040_441_7,
            0.000_999_999_157_048_626_2,
            0.000_699_914_755_518_017_2,
        ),
        &Transform::IDENTITY,
    );
    let (r, _) = run(BooleanOp::Subtract, &a, &b);
    let r = r.unwrap_or_else(|e| panic!("subtract: {e:?}"));
    r.validate().unwrap();
    assert!(faces_off_their_refit(&r) > 0, "the result has the gap");
    reopens(&r, "subtract");
}

/// A geometry v9 buffer (no planes stored) of the union of the holed cube
/// and a box turned a micro-radian, written before planes were stored. Its
/// holed top is flat to the plane it carried and not to the one its outer
/// ring refits to, so a v9 reader rejects it as not flat. A v10 reader
/// tries a fixed list of other planes for such a face and opens it on the
/// first that fits (the least-squares plane through the ring), moving no
/// point; saved again, the face stores that plane.
#[test]
fn a_v9_result_off_its_refit_plane_opens_on_a_fallback_plane() {
    let bytes = include_bytes!("fixtures/v9-boolean-off-refit-plane.bin");
    assert_eq!(&bytes[4..8], &9u32.to_le_bytes(), "a v9 buffer");
    let obj = Object::decode(bytes, &|_| None).unwrap_or_else(|e| panic!("{e:?}"));
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    assert!(faces_off_their_refit(&obj) > 0, "a v9 reader rejects it");
    // No point moved: the vertex table reads back bit for bit.
    let count = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    let stored: BTreeSet<Bits> = (0..count)
        .map(|i| {
            let at = |k: usize| {
                let o = 18 + 24 * i + 8 * k;
                f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap())
            };
            bits(Point3::new(at(0), at(1), at(2)))
        })
        .collect();
    assert_eq!(vertex_bits(&obj), stored);
    reopens(&obj, "v9 fixture");
}

/// A box on the cube's floor whose -y wall sits 9.5 nm inside the pillar's
/// +y wall (the `cc a54bcf0f` seed of `every_committed_result_reopens`):
/// the tidy welds the sliver between the two walls onto the pillar's
/// corner, so the holed bottom's outer ring visits that corner twice and
/// pinches. One piece is the floor, wound with its normal; the other is
/// the pillar's square, wound against it, and is the floor's hole again,
/// not a second face on the floor's plane winding the wrong way.
#[test]
fn a_ring_the_weld_pinches_around_a_hole_keeps_the_hole() {
    let a = holed_cube();
    let b = native_box(
        Point3::new(0.7 * MM, 0.7 * MM - 9.460_638_921_672_576e-9, 0.0),
        Point3::new(0.7 * MM + 0.2 * MM, MM, 0.2 * MM),
        &Transform::IDENTITY,
    );
    let (r, tidy) = run(BooleanOp::Union, &a, &b);
    let r = r.unwrap_or_else(|e| panic!("union: {e:?}"));
    assert_eq!(tidy.slivers_merged, 1, "{tidy:?}");
    assert_eq!(tidy.slivers_kept, 0, "{tidy:?}");
    assert_eq!(tidy.faces_split, 1, "{tidy:?}");
    assert_eq!(r.faces().len(), 14);
    // The floor is the cube's, holed, and the box's bottom beside it.
    let floor: Vec<_> = r
        .faces()
        .values()
        .filter(|f| r.loop_positions(f.outer_loop).all(|p| p.z == 0.0))
        .collect();
    assert_eq!(floor.len(), 2, "two faces on the floor");
    assert!(
        floor.iter().all(|f| f.plane.normal().z < 0.0),
        "every face on the floor faces down"
    );
    let holed: Vec<_> = floor.iter().filter(|f| !f.inner_loops.is_empty()).collect();
    assert_eq!(holed.len(), 1, "the floor keeps its hole");
    assert_eq!(holed[0].inner_loops.len(), 1);
    let corner = bits(Point3::new(0.7 * MM, 0.7 * MM, 0.0));
    assert!(
        r.loop_positions(holed[0].inner_loops[0])
            .any(|p| bits(p) == corner),
        "the hole runs through the pillar's corner"
    );
    check(&r, &tidy, &[&a, &b], "union");
    let v = r.volume().unwrap();
    assert!((v - 8.400_012_614e-10).abs() < 1e-18, "{v:e}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Boxes with faces within a fraction of a micron of the holed cube's
    /// walls, turned a little: every result a boolean commits, tidied or
    /// not, reopens from its own encoding to the same bytes and the same
    /// planes, bit for bit.
    #[test]
    fn every_committed_result_reopens(
        corners in prop::collection::vec(
            (0usize..5, -4e-7f64..4e-7, prop::sample::select(vec![0.01, 0.1, 1.0]), 0.0f64..1.0),
            6,
        ),
        axis in (-1.0f64..1.0, -1.0f64..1.0, 0.1f64..1.0),
        angle in prop::sample::select(vec![0.0, 1e-7, -1e-7, 1e-6, -1e-6, 1e-5]),
    ) {
        let a = holed_cube();
        let grid = [0.0, 0.3 * MM, 0.7 * MM, MM];
        // Index 4 is anywhere across the cube rather than on its grid.
        let at = |(g, jitter, scale, anywhere): (usize, f64, f64, f64)| {
            if g == 4 { anywhere * MM } else { grid[g] + jitter * scale }
        };
        let mut lo = [0.0; 3];
        let mut hi = [0.0; 3];
        for k in 0..3 {
            let (l, h) = (at(corners[k]), at(corners[k + 3]));
            let (l, h) = (l.min(h), l.max(h));
            lo[k] = l;
            hi[k] = if h - l < 0.05 * MM { l + 0.2 * MM } else { h };
        }
        let (lo, hi) = (Point3::new(lo[0], lo[1], lo[2]), Point3::new(hi[0], hi[1], hi[2]));
        let b = native_box(lo, hi, &Transform::IDENTITY);
        let center = Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, (lo.z + hi.z) / 2.0);
        let place = Transform::translation(-center)
            .then(&Transform::rotation(Vec3::new(axis.0, axis.1, axis.2), angle).unwrap())
            .then(&Transform::translation(center));
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            let mut tidy = BooleanTidy::default();
            if let Ok(r) = Object::boolean_reported(op, &a, &b, &place, &mut tidy) {
                let bytes = r.encode(&|_| 0);
                let back = Object::decode(&bytes, &|_| None);
                prop_assert!(back.is_ok(), "{:?} ({:?}): {:?}", op, tidy, back.err());
                let back = back.unwrap();
                prop_assert_eq!(back.encode(&|_| 0), bytes);
                prop_assert_eq!(planes(&back), planes(&r));
            }
        }
    }
}

// ─────────────────────────────────────────────── 7. push/pull through

/// A U slab 2 mm thick whose left arm (y 2 to 6 mm) is shorter than its
/// right (y 2 to 10 mm), and the left arm's inner face: pulled across the
/// gap, through the right arm and `past` beyond its outer face, a through
/// pull (a union) whose prism stands proud of that face by `past`.
fn u_slab(doc: &mut Document) -> (kernel::ObjectId, kernel::FaceId) {
    let ground = kernel::Plane::from_polygon(&[
        Point3::ORIGIN,
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(ground);
    let outline = [
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 10.0),
        (8.0, 10.0),
        (8.0, 2.0),
        (2.0, 2.0),
        (2.0, 6.0),
        (0.0, 6.0),
    ];
    let pts: Vec<Point3> = outline
        .iter()
        .map(|&(x, y)| Point3::new(x * MM, y * MM, 0.0))
        .collect();
    let sketch = doc.sketch_mut(s).unwrap();
    for i in 0..pts.len() {
        sketch
            .add_segment(pts[i], pts[(i + 1) % pts.len()])
            .unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    let id = doc.extrude_region(s, regions[0], 2.0 * MM).unwrap().0;
    (id, inner_face(doc.object(id).unwrap()))
}

/// The U's left arm's inner face, facing +x at x = 2 mm.
fn inner_face(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9)
                && f.plane
                    .signed_distance(Point3::new(2.0 * MM, 4.0 * MM, MM))
                    .abs()
                    < 1e-9
        })
        .map(|(id, _)| id)
        .unwrap()
}

/// A push or pull through is a boolean, and what its tidy merged is
/// reported like any other's: pulled 30 nm past the far wall, the prism's
/// proud rim is merged away and `take_boolean_tidy` says so, for a world
/// object and for a definition member alike.
#[test]
fn a_push_pull_through_reports_its_tidy() {
    let past = 3e-8;
    let distance = 8.0 * MM + past;
    let check = |tidy: &BooleanTidy, what: &str| {
        assert!(tidy.slivers_merged > 0, "{what}: {tidy:?}");
        assert_eq!(tidy.slivers_kept, 0, "{what}: {tidy:?}");
        assert!(
            tidy.max_move > 0.0 && tidy.max_move <= tol::TIDY_REACH,
            "{what}: {tidy:?}"
        );
        for p in &tidy.sites {
            assert!(
                (p.x - (10.0 * MM + past)).abs() < 1e-12,
                "{what}: {p:?} at the far wall"
            );
        }
    };

    let mut doc = Document::new();
    let (id, face) = u_slab(&mut doc);
    assert!(doc.object(id).unwrap().push_pull_overshoots(face, distance));
    let _ = doc.take_boolean_tidy();
    let (pieces, _) = doc.push_pull_through(id, face, distance).unwrap();
    assert_eq!(pieces.len(), 1);
    check(&doc.take_boolean_tidy(), "world");
    reopens(doc.object(pieces[0]).unwrap(), "world");

    let mut doc = Document::new();
    let (id, _) = u_slab(&mut doc);
    let (cid, _, _) = doc.make_component(&[kernel::NodeId::Object(id)]).unwrap();
    let member = doc.def_members(cid).unwrap()[0];
    let face = inner_face(doc.object(member).unwrap());
    let _ = doc.take_boolean_tidy();
    doc.push_pull_through_in_component(cid, member, face, distance)
        .unwrap();
    // The member sits in the definition where the slab was drawn, so its
    // sites are the same points.
    check(&doc.take_boolean_tidy(), "member");
}

// ───────────────────── 1b. the nanometer band around a vertex

/// A top aimed within 0.1 nm of a mesh vertex, either side (and exactly at
/// it), commits for all three operations, solid and valid: closer than the
/// arrangement's own resolution, the cut is the vertex itself. -0.3 nm
/// commits too.
#[test]
fn aimed_cuts_within_a_tenth_of_a_nanometer_commit() {
    let s = sphere();
    let v = aimed_vertex(&s);
    for d_nm in [0.0, 0.01, -0.01, 0.1, -0.1, -0.3] {
        let b = box_topped_at(v, d_nm * 1e-9);
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            let what = format!("top {d_nm} nm from the vertex {op:?}");
            match run(op, &s, &b) {
                (Ok(r), tidy) => check(&r, &tidy, &[&s, &b], &what),
                (Err(e), _) => panic!("{what}: {e:?}"),
            }
        }
    }
}

/// KNOWN LIMIT, pinned as observed (KB-53, and the "cuts within about a
/// nanometer of a vertex or edge" follow-up, to be fixed after the merge):
/// aimed within about a nanometer of a mesh vertex, the cut refuses for a
/// lumpy set of offsets, not a clean band (the set reaches past the 1 nm
/// merge distance). Observed on the sphere's vertex: refused are +0.3 nm
/// (all three ops), ±0.5 nm (all three ops) and +1.1 nm for Intersect
/// only; committing are 0, ±0.01, ±0.1, ±0.2, -0.3, -1.1 and ±1.2 nm. The
/// exact refusals seen today are asserted here so the fix flips them
/// deliberately; when it lands, these rows become must-commit rows. (Since
/// a boolean keeps an operand's faces the cutter does not reach and
/// splices the rest in, +0.5 nm Union and Subtract fail that splice as a
/// non-manifold weld, `DegenerateContact`, where the full rebuild had
/// reported the sliver first, `ResultTooThin`; they refuse either way.)
#[test]
fn aimed_cuts_in_the_refused_band_are_pinned() {
    use BooleanError::{DegenerateContact as Contact, ResultTooThin as Thin};
    let s = sphere();
    let v = aimed_vertex(&s);
    let u = BooleanOp::Union;
    let d = BooleanOp::Subtract;
    let i = BooleanOp::Intersect;
    let rows: [(f64, BooleanOp, BooleanError); 10] = [
        (0.3, u, Contact),
        (0.3, d, Contact),
        (0.3, i, Contact),
        (0.5, u, Contact),
        (0.5, d, Contact),
        (0.5, i, Contact),
        (-0.5, u, Contact),
        (-0.5, d, Contact),
        (-0.5, i, Thin),
        (1.1, i, Contact),
    ];
    for (d_nm, op, expected) in rows {
        let b = box_topped_at(v, d_nm * 1e-9);
        let got = run(op, &s, &b).0.map(|_| ());
        assert_eq!(
            got,
            Err(expected),
            "top {d_nm} nm from the vertex {op:?} (known limit; a fix flips this row)"
        );
    }
}
