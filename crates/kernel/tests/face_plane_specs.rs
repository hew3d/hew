//! Each face's plane is stored in the geometry buffer (v11,
//! HEW_FILE_FORMAT.md §3.1, §3.3 "Face plane recomputation").
//!
//! A buffer before v11 stored no planes: the reader refit each face's plane
//! from its outer ring, and a face flat to the plane it carried but not to
//! the refit one saved to a file that would not open. A v11 writer stores
//! every plane the refit does not give bit for bit, the reader uses it, and
//! a face reopens on exactly the plane it was validated against. A buffer
//! before v11 keeps the refit, with a fixed list of fallback planes for a
//! face whose points stray from it.

use kernel::trig;
use std::collections::BTreeMap;

use crate::common;

use kernel::{DecodeError, Document, Object, Plane, Point3, TopologyError, Transform, Vec3};
use proptest::prelude::*;

type Bits = (u64, u64, u64);

fn bits(p: Point3) -> Bits {
    (p.x.to_bits(), p.y.to_bits(), p.z.to_bits())
}

/// Every face's plane by bits (normal, then the origin's signed distance),
/// keyed by its outer ring from the ring's smallest point, so two copies of
/// one object compare equal whatever their slot order.
fn planes(obj: &Object) -> BTreeMap<Vec<Bits>, [u64; 4]> {
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

/// How many faces have a boundary point farther than the object's
/// tolerance from the plane refit from their outer ring: faces a v9 reader,
/// which gave every face that plane, would reject as not flat.
fn faces_off_their_refit(obj: &Object) -> usize {
    obj.faces()
        .values()
        .filter(|f| {
            let ring = canonical(obj.loop_positions(f.outer_loop).collect());
            let Ok(refit) = Plane::from_polygon(&ring) else {
                return true;
            };
            std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .flat_map(|l| obj.loop_positions(l))
                .any(|p| refit.signed_distance(p).abs() > obj.planarity_tol())
        })
        .count()
}

/// Encodes and decodes `obj`: valid, the same bytes, every plane bit for
/// bit.
fn reopens_on_its_planes(obj: &Object, what: &str) {
    let bytes = obj.encode(&|_| 0);
    let back = Object::decode(&bytes, &|_| None).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert_eq!(back.encode(&|_| 0), bytes, "{what}: reopens identically");
    assert_eq!(planes(&back), planes(obj), "{what}: reopens on its planes");
}

/// A rectangle `w` by `h` drawn at `origin` on the plane with normal `n`
/// (spanned by `u` and `v`), extruded `depth`.
fn extruded_rectangle(
    doc: &mut Document,
    origin: Point3,
    (n, u, v): (Vec3, Vec3, Vec3),
    (w, h, depth): (f64, f64, f64),
) -> Option<kernel::ObjectId> {
    let s = doc.add_sketch(Plane::from_point_normal(origin, n).ok()?);
    let pts = [
        origin,
        origin + u * w,
        origin + u * w + v * h,
        origin + v * h,
    ];
    let sk = doc.sketch_mut(s)?;
    for i in 0..4 {
        sk.add_segment(pts[i], pts[(i + 1) % 4]).ok()?;
    }
    let r = *doc.extrudable_regions(s).ok()?.first()?;
    Some(doc.extrude_region(s, r, depth).ok()?.0)
}

/// An extrusion 1,800 km from the origin, on a tilted plane: far out the
/// Newell sum the refit uses loses enough to the rounding of coordinates
/// that large that one face's points are a nanometer and more off the
/// refit plane, though all of them are within the tolerance of the plane
/// the face carries. A file that stores no plane could not reopen it; one
/// that stores it does, on that plane.
#[test]
fn an_extrusion_far_from_the_origin_reopens_on_its_planes() {
    let mut doc = Document::new();
    let n = Vec3::new(-0.5850796056516863, -0.5877382159769943, -0.558789445614936);
    let u = Vec3::new(-0.7087078702700594, 0.7055020585492834, 0.0);
    let v = Vec3::new(0.3942271041769502, 0.3960184779311485, -0.8293095655238474);
    let origin = Point3::new(1722520.6716612568, 735900.2563239258, 0.4306831869323101);
    let id = extruded_rectangle(
        &mut doc,
        origin,
        (n, u, v),
        (0.6807101532624806, 0.938587559638842, 0.06131548501384983),
    )
    .expect("the extrusion commits");
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    assert!(faces_off_their_refit(obj) > 0, "the extrusion has the gap");
    reopens_on_its_planes(obj, "far extrusion");
    let saved = doc.save();
    let back = Document::load(&saved).unwrap();
    assert_eq!(back.save(), saved, "the document reopens to the same bytes");
    assert_eq!(back.state_hash(), doc.state_hash());
}

/// Whether a transform's answer is one the face-plane rule allows: `Ok`,
/// or the typed refusal for a face rounding would leave off its plane, and
/// that only far from the origin.
fn transform_answer_is_allowed(
    answer: &Result<kernel::DocChange, kernel::DocumentError>,
    far: f64,
) -> bool {
    match answer {
        Ok(_) => true,
        Err(kernel::DocumentError::Transform(kernel::TransformError::FaceNotFlat)) => far > 1e4,
        Err(_) => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Rectangles extruded on tilted planes from the origin out to
    /// thousands of kilometers, then turned about their corner and moved:
    /// every object validates and reopens from its own encoding on the
    /// same planes, bit for bit. A transform succeeds, or (only beyond
    /// 10 km, where rounding can leave a turned face off any plane) is
    /// refused as `FaceNotFlat` with the object untouched.
    #[test]
    fn every_face_reopens_on_its_plane(
        normal in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
        far in prop::sample::select(vec![0.0, 1.0, 1e3, 1e4, 1e5, 1e6, 3e6]),
        at in (0.0f64..1.0, 0.0f64..1.0, 0.0f64..1.0),
        size in (0.01f64..2.0, 0.01f64..2.0, 0.01f64..2.0),
        turn in ((-1.0f64..1.0, -1.0f64..1.0, 0.1f64..1.0), 0.0f64..3.0),
        shift in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
    ) {
        let Ok(n) = Vec3::new(normal.0, normal.1, normal.2).normalized() else {
            return Ok(());
        };
        let u = n.cross(Vec3::new(0.0, 0.0, 1.0)).normalized()
            .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
        let v = n.cross(u);
        let origin = Point3::new(far * (1.0 + at.0), far * at.1, at.2);
        let mut doc = Document::new();
        let Some(id) = extruded_rectangle(&mut doc, origin, (n, u, v), size) else {
            return Ok(());
        };
        let check = |doc: &Document, what: &str| -> Result<(), TestCaseError> {
            let obj = doc.object(id).unwrap();
            prop_assert!(obj.validate().is_ok(), "{}: {:?}", what, obj.validate());
            let bytes = obj.encode(&|_| 0);
            let back = Object::decode(&bytes, &|_| None);
            prop_assert!(back.is_ok(), "{}: {:?}", what, back.err());
            let back = back.unwrap();
            prop_assert_eq!(back.encode(&|_| 0), bytes, "{}", what);
            prop_assert_eq!(planes(&back), planes(obj), "{}", what);
            Ok(())
        };
        check(&doc, "extruded")?;
        let ((ax, ay, az), angle) = turn;
        let about = Transform::translation(Point3::ORIGIN - origin)
            .then(&Transform::rotation(Vec3::new(ax, ay, az), angle).unwrap())
            .then(&Transform::translation(origin - Point3::ORIGIN));
        let before = doc.object(id).unwrap().encode(&|_| 0);
        let turned = doc.transform_object(id, &about);
        prop_assert!(transform_answer_is_allowed(&turned, far), "turned at {}: {:?}", far, turned);
        if turned.is_err() {
            prop_assert_eq!(doc.object(id).unwrap().encode(&|_| 0), before.clone(), "a refusal changes nothing");
        }
        check(&doc, "turned")?;
        let by = Transform::translation(Vec3::new(shift.0, shift.1, shift.2) * (1.0 + far));
        let moved = doc.transform_object(id, &by);
        prop_assert!(transform_answer_is_allowed(&moved, far), "moved at {}: {:?}", far, moved);
        check(&doc, "moved")?;
    }
}

/// The case that once failed: a 1 cm box drawn on a tilted plane about
/// 1,980 m from the origin, turned 2.9 rad about its corner and then moved
/// 665 m. Mapping each plane through the foot of its perpendicular from
/// the origin let rounding grow with the square of that distance and left
/// faces 1.6 nm (turned) and 1.9 nm (moved) off the planes they carried,
/// past the 1 nm tolerance. Each step now succeeds, or refuses typed; it
/// never panics, and what it commits validates and reopens.
#[test]
fn a_small_box_far_out_turned_and_moved_stays_on_its_planes() {
    let mut doc = Document::new();
    let n = Vec3::new(-0.2234100139431907, 0.777023731571268, -0.3159577631765442)
        .normalized()
        .unwrap();
    let u = n.cross(Vec3::new(0.0, 0.0, 1.0)).normalized().unwrap();
    let v = n.cross(u);
    let origin = Point3::new(1983.0850745338844, 951.9560284026388, 0.18057721557255224);
    let id = extruded_rectangle(&mut doc, origin, (n, u, v), (0.01, 0.01, 0.01))
        .expect("the box commits");
    let axis = Vec3::new(0.43780035440101694, -0.4521027727483684, 0.5826254246504377);
    let about = Transform::translation(Point3::ORIGIN - origin)
        .then(&Transform::rotation(axis, 2.9).unwrap())
        .then(&Transform::translation(origin - Point3::ORIGIN));
    let by = Transform::translation(Vec3::new(
        -260.41925899586323,
        309.06018428415354,
        528.099055134821,
    ));
    for (t, what) in [(about, "turned"), (by, "moved")] {
        match doc.transform_object(id, &t) {
            Ok(_) => {}
            Err(kernel::DocumentError::Transform(kernel::TransformError::FaceNotFlat)) => {}
            Err(e) => panic!("{what}: {e:?}"),
        }
        let obj = doc.object(id).unwrap();
        obj.validate().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        reopens_on_its_planes(obj, what);
    }
}

// ───────────────────────────────────────── crafted buffers

/// The unit cube's corners.
fn cube_positions() -> Vec<[f64; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ]
}

/// The cube's faces, wound outward; the first is the top.
fn cube_faces() -> Vec<Vec<u32>> {
    vec![
        vec![4, 5, 6, 7],
        vec![0, 3, 2, 1],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
}

/// Hand-encodes a geometry buffer (HEW_FILE_FORMAT.md §3.1) at `version`
/// (9, 10 or 11) of hole-free plain faces, watertight when every edge is
/// shared. At v11 each face writes the plane
/// flag `planes[i]` gives: `None` for 0, `Some(bytes)` for 1 and then the
/// raw 32 bytes, or a lone flag byte given as `Some` of one byte. From v10
/// each face ends in its (empty) crease and blend blocks.
fn craft(
    version: u32,
    positions: &[[f64; 3]],
    faces: &[Vec<u32>],
    planes: &[Option<Vec<u8>>],
) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"HEWG");
    b.extend_from_slice(&version.to_le_bytes());
    // Watertight when every directed edge has its reverse.
    let edges: std::collections::BTreeSet<(u32, u32)> = faces
        .iter()
        .flat_map(|f| (0..f.len()).map(move |i| (f[i], f[(i + 1) % f.len()])))
        .collect();
    b.push(u8::from(
        edges.iter().all(|&(a, c)| edges.contains(&(c, a))),
    ));
    b.extend_from_slice(&kernel::NO_MATERIAL.to_le_bytes());
    b.push(0u8); // native
    b.extend_from_slice(&(positions.len() as u32).to_le_bytes());
    for q in positions {
        for c in q {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    b.extend_from_slice(&(faces.len() as u32).to_le_bytes());
    for (i, outer) in faces.iter().enumerate() {
        b.extend_from_slice(&kernel::NO_MATERIAL.to_le_bytes());
        b.push(0u8); // uv frame
        b.push(0u8); // surface
        b.extend_from_slice(&(outer.len() as u32).to_le_bytes());
        for v in outer {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&0u32.to_le_bytes()); // holes
        if version >= 11 {
            match planes.get(i).cloned().flatten() {
                None => b.push(0u8),
                Some(raw) if raw.len() == 1 => b.push(raw[0]),
                Some(raw) => {
                    b.push(1u8);
                    b.extend_from_slice(&raw);
                }
            }
        }
        b.extend_from_slice(&[0u8, 0, 0, 0]); // soft, curves, polygons, back paint
        b.extend_from_slice(&[0u8, 0]); // face and edge dictionaries
        if version >= 10 {
            b.extend_from_slice(&[0u8, 0]); // creases, blend tag
        }
    }
    b
}

fn plane_bytes(n: [f64; 3], offset: f64) -> Vec<u8> {
    n.iter()
        .chain(std::iter::once(&offset))
        .flat_map(|c| c.to_le_bytes())
        .collect()
}

/// The top face of a decoded cube (its normal points up).
fn top(obj: &Object) -> Plane {
    obj.faces()
        .values()
        .map(|f| f.plane)
        .find(|p| p.normal().z > 0.5)
        .unwrap()
}

/// A v11 reader uses the stored plane as written: the top face stored 0.4
/// nm above its corners (within the native tolerance) reopens on exactly
/// that plane, not on the one its ring refits to, and writes it back.
#[test]
fn a_stored_plane_is_the_face_plane_as_written() {
    let offset = 1.0 + 4e-10;
    let buf = craft(
        11,
        &cube_positions(),
        &cube_faces(),
        &[Some(plane_bytes([0.0, 0.0, 1.0], offset))],
    );
    let obj = Object::decode(&buf, &|_| None).unwrap();
    let plane = top(&obj);
    assert_eq!(plane.normal(), Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(-plane.signed_distance(Point3::ORIGIN), offset);
    assert_eq!(
        obj.encode(&|_| 0).len(),
        buf.len(),
        "the plane is written back"
    );
}

/// Flag 0 at v11 is the refit, and a stored plane equal to the refit bit
/// for bit is written back as flag 0: a plain cube grows by one byte per
/// face over v10.
#[test]
fn a_plane_the_refit_gives_is_one_flag_byte() {
    let v9 = craft(9, &cube_positions(), &cube_faces(), &[]);
    let v10 = craft(10, &cube_positions(), &cube_faces(), &[]);
    let v11 = craft(11, &cube_positions(), &cube_faces(), &[]);
    let from_v9 = Object::decode(&v9, &|_| None).unwrap();
    let from_v10 = Object::decode(&v10, &|_| None).unwrap();
    let from_v11 = Object::decode(&v11, &|_| None).unwrap();
    assert_eq!(planes(&from_v9), planes(&from_v11));
    assert_eq!(planes(&from_v10), planes(&from_v11));
    assert_eq!(v11.len(), v10.len() + 6);
    let refit = top(&from_v11);
    let stored = craft(
        11,
        &cube_positions(),
        &cube_faces(),
        &[Some(plane_bytes(
            [0.0, 0.0, 1.0],
            -refit.signed_distance(Point3::ORIGIN),
        ))],
    );
    let back = Object::decode(&stored, &|_| None).unwrap();
    assert_eq!(back.encode(&|_| 0).len(), v11.len());
}

/// A stored plane that is not finite, not unit, or faces against its
/// loop's winding, and a plane flag that is neither 0 nor 1, are corrupt;
/// a stored plane its face's points are off is not flat.
#[test]
fn a_stored_plane_the_face_cannot_have_is_refused() {
    let corrupt = |raw: Vec<u8>, what: &str| {
        let buf = craft(11, &cube_positions(), &cube_faces(), &[Some(raw)]);
        match Object::decode(&buf, &|_| None) {
            Err(DecodeError::Corrupt { .. }) => {}
            other => panic!("{what}: {other:?}"),
        }
    };
    corrupt(plane_bytes([f64::NAN, 0.0, 1.0], 1.0), "a NaN normal");
    corrupt(
        plane_bytes([0.0, 0.0, 1.0], f64::INFINITY),
        "an infinite offset",
    );
    corrupt(plane_bytes([0.0, 0.0, 1.0 + 1e-6], 1.0), "a long normal");
    corrupt(plane_bytes([0.0, 0.0, -1.0], -1.0), "a plane facing in");
    corrupt(vec![2u8], "a plane flag of 2");
    let off = craft(
        11,
        &cube_positions(),
        &cube_faces(),
        &[Some(plane_bytes([0.0, 0.0, 1.0], 1.0 + 1e-8))],
    );
    assert!(matches!(
        Object::decode(&off, &|_| None),
        Err(DecodeError::InvalidTopology(
            TopologyError::FaceGeometryNotPlanar { .. }
        ))
    ));
}

/// A v9 cube whose top corner is 0.1 µm high, its sides beside that corner
/// triangulated so only the top is warped: no plane in the fallback list
/// holds all four corners within the native tolerance, so it is refused
/// as not flat, as every v9 reader refused it.
#[test]
fn a_v9_face_no_fallback_plane_fits_is_still_refused() {
    let mut positions = cube_positions();
    positions[6][2] += 1e-7;
    let faces = vec![
        vec![4, 5, 6, 7],
        vec![0, 3, 2, 1],
        vec![0, 1, 5, 4],
        vec![1, 2, 6],
        vec![1, 6, 5],
        vec![2, 3, 7],
        vec![2, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let buf = craft(9, &positions, &faces, &[]);
    assert!(matches!(
        Object::decode(&buf, &|_| None),
        Err(DecodeError::InvalidTopology(
            TopologyError::FaceGeometryNotPlanar { .. }
        ))
    ));
    // The same top a thousandth of the height off is within tolerance of
    // the refit, which every reader gives it.
    let mut positions = cube_positions();
    positions[6][2] += 1e-12;
    let obj = Object::decode(&craft(9, &positions, &faces, &[]), &|_| None).unwrap();
    obj.validate().unwrap();
}

// ───────────────────────────────────────── derived planes stay derived

/// How many faces carry a plane other than the one their outer ring refits
/// to bit for bit: the faces a file stores a plane for.
fn faces_off_their_own_refit(obj: &Object) -> usize {
    obj.faces()
        .values()
        .filter(|f| {
            let ring = canonical(obj.loop_positions(f.outer_loop).collect());
            Plane::from_polygon(&ring).map_or(true, |refit| {
                let (a, b) = (refit.normal(), f.plane.normal());
                [a.x, a.y, a.z, refit.signed_distance(Point3::ORIGIN)].map(f64::to_bits)
                    != [b.x, b.y, b.z, f.plane.signed_distance(Point3::ORIGIN)].map(f64::to_bits)
            })
        })
        .count()
}

/// A turn about a tilted axis through a point off the origin, then a move.
fn turn_and_move() -> [Transform; 2] {
    let at = Point3::new(3.25, -1.5, 0.75);
    [
        Transform::translation(Point3::ORIGIN - at)
            .then(&Transform::rotation(Vec3::new(0.3, -0.8, 0.5), 1.1).unwrap())
            .then(&Transform::translation(at - Point3::ORIGIN)),
        Transform::translation(Vec3::new(120.5, -37.25, 9.125)),
    ]
}

/// A face whose plane is its outer ring's refit keeps that through a move
/// and a turn: the transform gives it the mapped ring's refit, so a drawn
/// box, and an imported mesh, still store no plane (every face is the one
/// flag byte), and reopen on the same planes.
#[test]
fn a_box_and_an_imported_mesh_store_no_planes_after_a_move_and_a_turn() {
    let mut doc = Document::new();
    let id = extruded_rectangle(
        &mut doc,
        Point3::new(0.5, 0.25, 0.0),
        (
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (0.6, 0.4, 0.3),
    )
    .unwrap();
    let mut solid = doc.object(id).unwrap().clone();
    // An imported cube, one corner a micron off its top: flat to the
    // import tolerance, not to any plane exactly.
    let mut positions: Vec<Point3> = cube_positions()
        .into_iter()
        .map(|[x, y, z]| Point3::new(x, y, z))
        .collect();
    positions[6].z += 1e-6;
    let faces: Vec<Vec<usize>> = cube_faces()
        .into_iter()
        .map(|f| f.into_iter().map(|i| i as usize).collect())
        .collect();
    let mut mesh = Object::from_polygons_import_for_test(&positions, &faces).unwrap();
    for (obj, what) in [(&mut solid, "box"), (&mut mesh, "imported mesh")] {
        assert_eq!(faces_off_their_own_refit(obj), 0, "{what}: as built");
        for (t, step) in turn_and_move().iter().zip(["turned", "moved"]) {
            obj.apply_transform(t).unwrap();
            assert_eq!(faces_off_their_own_refit(obj), 0, "{what}: {step}");
            reopens_on_its_planes(obj, what);
        }
    }
}

/// A square sheet 10 m on a side, notched half a meter in along its near
/// and far edges, whose loop runs `n` vertices 0.9 nm above the square's
/// plane near one side and one vertex 0.9 nm below it in the middle of
/// each notch, its corners on the plane (so the corners alone make the
/// triples of largest area).
/// Every point is within the native 1 nm of the square's plane, but the
/// refit, through the vertices' centroid (mostly above), misses the low
/// ones, and so does any least-squares tilt.
fn sheet_off_its_refit(n: usize) -> Vec<[f64; 3]> {
    let h = 0.9e-9;
    let mut v = vec![[0.0, 0.0, 0.0]];
    v.extend((0..n).map(|k| [1.0 + 0.01 * k as f64, 0.5, h]));
    v.push([5.0, 0.5, -h]);
    v.push([10.0, 0.0, 0.0]);
    v.push([10.0, 10.0, 0.0]);
    v.push([5.0, 9.5, -h]);
    v.extend((0..n).rev().map(|k| [1.0 + 0.01 * k as f64, 9.5, h]));
    v.push([0.0, 10.0, 0.0]);
    v
}

/// A v9 face whose points the refit and the least-squares plane both miss
/// opens on the next fallback, the plane through its vertex triple of
/// largest area (here three corners, so exactly the square's plane). A
/// ring longer than that search's 64-vertex cap skips it, and the face is
/// refused as not flat, as every v9 reader refused it.
#[test]
fn a_v9_face_only_the_largest_triple_fits_opens_on_it() {
    let opens = sheet_off_its_refit(20);
    assert!(opens.len() <= 64);
    let pts: Vec<Point3> = opens
        .iter()
        .map(|&[x, y, z]| Point3::new(x, y, z))
        .collect();
    let refit = Plane::from_polygon(&canonical(pts.clone())).unwrap();
    assert!(
        pts.iter().any(|&p| refit.signed_distance(p).abs() > 1e-9),
        "the refit misses a point"
    );
    let face: Vec<u32> = (0..opens.len() as u32).collect();
    let obj = Object::decode(&craft(9, &opens, &[face], &[]), &|_| None).unwrap();
    obj.validate().unwrap();
    let plane = obj.faces().values().next().unwrap().plane;
    assert_eq!(
        plane.normal(),
        Vec3::new(0.0, 0.0, 1.0),
        "not a least-squares tilt"
    );
    assert_eq!(plane.signed_distance(Point3::ORIGIN), 0.0);

    let refused = sheet_off_its_refit(40);
    assert!(refused.len() > 64);
    let face: Vec<u32> = (0..refused.len() as u32).collect();
    assert!(matches!(
        Object::decode(&craft(9, &refused, &[face], &[]), &|_| None),
        Err(DecodeError::InvalidTopology(
            TopologyError::FaceGeometryNotPlanar { .. }
        ))
    ));
}

/// A face carrying a plane other than its ring's refit has that plane
/// mapped by a transform; when the map squashes it to no direction, the
/// face falls back to its mapped ring's refit, and only a face neither
/// fits is refused typed, before anything moves. Squashed by
/// (1e6, 1e-9, 1e-8), whose inverse-transpose takes the x normal to a
/// vector 1e-17 long: a 1 km cube whose +x face stores its plane one ulp
/// off the refit maps onto its rings' refits, validates and reopens, as
/// the same cube with every plane its ring's refit does. One whose +x face
/// has a vertex mid-edge 0.5 nm out along x (within the tolerance of the
/// face's plane) is refused `Singular`: the squash carries that vertex
/// 0.5 mm off a face 1 µm by 10 µm, so the mapped ring's refit holds none
/// of it either.
#[test]
fn a_plane_the_map_squashes_to_no_direction_falls_back_to_the_refit() {
    let km: Vec<[f64; 3]> = cube_positions()
        .into_iter()
        .map(|p| p.map(|c| c * 1000.0))
        .collect();
    let mut planes = vec![None; 6];
    planes[3] = Some(plane_bytes(
        [1.0, 0.0, 0.0],
        f64::from_bits(1000.0f64.to_bits() + 1),
    ));
    let stored = Object::decode(&craft(11, &km, &cube_faces(), &planes), &|_| None).unwrap();
    stored.validate().unwrap();
    let squash = Transform::scale(Vec3::new(1e6, 1e-9, 1e-8));
    let mut obj = stored.clone();
    obj.apply_transform(&squash).unwrap();
    obj.validate().unwrap();
    reopens_on_its_planes(&obj, "one ulp off, squashed");
    let mut derived = Object::decode(&craft(11, &km, &cube_faces(), &[]), &|_| None).unwrap();
    derived.apply_transform(&squash).unwrap();
    derived.validate().unwrap();
    reopens_on_its_planes(&derived, "refits, squashed");

    // Vertex 8 between corners 1 and 2, on the +x face and the bottom.
    let mut bent = km.clone();
    bent.push([1000.0 + 0.5e-9, 500.0, 0.0]);
    let mut faces = cube_faces();
    faces[1] = vec![0, 3, 2, 8, 1];
    faces[3] = vec![1, 8, 2, 6, 5];
    let stored = Object::decode(&craft(11, &bent, &faces, &[]), &|_| None).unwrap();
    stored.validate().unwrap();
    let mut obj = stored.clone();
    assert_eq!(
        obj.apply_transform(&squash),
        Err(kernel::TransformError::Singular)
    );
    assert_eq!(obj.encode(&|_| 0), stored.encode(&|_| 0), "nothing moved");
}

// ──────────────────────────────── faces too large for a plane

/// The unit cube scaled by `s`, as `from_polygons` input.
fn scaled_cube(s: f64) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let positions = cube_positions()
        .into_iter()
        .map(|[x, y, z]| Point3::new(x * s, y * s, z * s))
        .collect();
    let faces = cube_faces()
        .into_iter()
        .map(|f| f.into_iter().map(|i| i as usize).collect())
        .collect();
    (positions, faces)
}

/// A face so large that its Newell vector (twice its area, in square
/// meters) overflows its length has no plane: a cube 1e100 m on a side
/// would get a zero normal, which no file can reopen, so building it is
/// refused typed (DEVELOPMENT.md rule 4), in debug and release alike. A
/// cube 1e50 m on a side is far from that limit and builds, validates and
/// reopens on its planes.
#[test]
fn a_cube_too_large_for_its_face_planes_is_refused_typed() {
    let (positions, faces) = scaled_cube(1e100);
    assert_eq!(
        Plane::from_polygon(&faces[0].iter().map(|&i| positions[i]).collect::<Vec<_>>()),
        Err(kernel::MathError::DegeneratePlane)
    );
    assert!(matches!(
        Object::from_polygons(&positions, &faces),
        Err(TopologyError::DegenerateFace { .. })
    ));
    // Bytes holding such a cube (none can be written now) are refused
    // typed on open: the loader has no refit for its faces.
    let huge: Vec<[f64; 3]> = positions.iter().map(|p| [p.x, p.y, p.z]).collect();
    for version in [9, 10, 11] {
        assert!(matches!(
            Object::decode(&craft(version, &huge, &cube_faces(), &[]), &|_| None),
            Err(DecodeError::Corrupt { .. })
        ));
    }

    let (positions, faces) = scaled_cube(1e50);
    let obj = Object::from_polygons(&positions, &faces).expect("a 1e50 m cube builds");
    obj.validate().unwrap();
    reopens_on_its_planes(&obj, "1e50 m cube");
}

// ──────────────────────────────── curved faces far from the origin

/// A cylinder of radius 0.25 m in 48 facets, 0.4 m tall, drawn on the
/// ground at `(far, far)`: its wall facets claim the cylinder.
fn far_cylinder(doc: &mut Document, far: f64) -> kernel::ObjectId {
    let c = Point3::new(far, far, 0.0);
    let s = doc.add_sketch(Plane::from_point_normal(c, Vec3::new(0.0, 0.0, 1.0)).unwrap());
    let sk = doc.sketch_mut(s).unwrap();
    sk.begin_curve_with(kernel::CurveGeom {
        center: c,
        radius: 0.25,
    })
    .unwrap();
    let p = |i: usize| {
        let a = std::f64::consts::TAU * (i % 48) as f64 / 48.0;
        Point3::new(far + 0.25 * trig::cos(a), far + 0.25 * trig::sin(a), 0.0)
    };
    for i in 0..48 {
        sk.add_segment(p(i), p(i + 1)).unwrap();
    }
    sk.end_curve();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 0.4).unwrap().0
}

/// How many faces claim an analytic surface.
fn claimed(obj: &Object) -> usize {
    obj.faces().values().filter(|f| f.surface.is_some()).count()
}

/// A cylinder 1,000 km out turned 0.7 rad about (1, 1, 0) once committed
/// a wall facet on its mapped ring's refit, which far out tilts by its
/// vertices' rounding over the facet's 3 cm width past what the facet's
/// cylinder claim allows (its axis parallel to the plane within
/// `NORMAL_DIRECTION`): a debug build panicked, and a release build saved
/// a file that would not reopen. Each face now takes a plane its mapped
/// claim agrees with (the mapped plane keeps the exact turn of the
/// normal), so from 1 km to 1,000 km every turn and uniform scale either
/// commits, keeping every claim, validating and reopening on its planes,
/// or refuses typed with nothing moved.
#[test]
fn a_cylinder_far_out_turned_and_scaled_keeps_its_claims_and_reopens() {
    for far in [1e3, 1e5, 1e6, 3e6] {
        let mut doc = Document::new();
        let id = far_cylinder(&mut doc, far);
        let walls = claimed(doc.object(id).unwrap());
        assert_eq!(walls, 48, "{far}: every wall facet claims the cylinder");
        let pivot = Point3::new(far, far, 0.2);
        let about = |l: Transform| {
            Transform::translation(Point3::ORIGIN - pivot)
                .then(&l)
                .then(&Transform::translation(pivot - Point3::ORIGIN))
        };
        let steps = [
            (
                "turned 0.7",
                Transform::rotation(Vec3::new(1.0, 1.0, 0.0), 0.7).unwrap(),
            ),
            (
                "turned 1.3",
                Transform::rotation(Vec3::new(0.2, -1.0, 0.4), 1.3).unwrap(),
            ),
            ("scaled 1.5", Transform::uniform_scale(1.5)),
            (
                "turned 2.9",
                Transform::rotation(Vec3::new(-0.7, 0.1, 0.3), 2.9).unwrap(),
            ),
            ("scaled 0.37", Transform::uniform_scale(0.37)),
        ];
        let mut committed = 0;
        for (what, l) in steps {
            let what = format!("{far} m, {what}");
            let before = doc.object(id).unwrap().encode(&|_| 0);
            match doc.transform_object(id, &about(l)) {
                Ok(_) => committed += 1,
                Err(kernel::DocumentError::Transform(kernel::TransformError::FaceNotFlat))
                    if far > 1e6 =>
                {
                    assert_eq!(doc.object(id).unwrap().encode(&|_| 0), before, "{what}");
                }
                Err(e) => panic!("{what}: {e:?}"),
            }
            let obj = doc.object(id).unwrap();
            obj.validate().unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(claimed(obj), walls, "{what}: the claims ride along");
            reopens_on_its_planes(obj, &what);
            let back = Document::load(&doc.save()).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(back.state_hash(), doc.state_hash(), "{what}: reopens");
        }
        if far <= 1e6 {
            assert_eq!(committed, steps.len(), "{far} m: every step commits");
        }
    }
}

/// A solid of revolution about +Z through the origin, in 24 segments: a
/// ring of radius 0.2 centered 1 m out (a torus) or the triangle under the
/// line from (0.5, 0) to (0, 1) (a cone and its base).
fn revolved(torus: bool) -> Object {
    use kernel::document::{RevolveProfile, RevolveSpec};
    let p3 = Point3::new;
    let xz = Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
    let mut doc = Document::new();
    let s = doc.add_sketch(xz);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        if torus {
            sk.begin_curve_with(kernel::CurveGeom {
                center: p3(1.0, 0.0, 0.5),
                radius: 0.2,
            })
            .unwrap();
            let at = |k: usize| {
                let a = std::f64::consts::TAU * (k % 24) as f64 / 24.0;
                p3(1.0 + 0.2 * trig::cos(a), 0.0, 0.5 + 0.2 * trig::sin(a))
            };
            for k in 0..24 {
                sk.add_segment(at(k), at(k + 1)).unwrap();
            }
            sk.end_curve();
        } else {
            sk.add_segment(p3(0.0, 0.0, 0.0), p3(0.5, 0.0, 0.0))
                .unwrap();
            sk.add_segment(p3(0.5, 0.0, 0.0), p3(0.0, 0.0, 1.0))
                .unwrap();
            sk.add_segment(p3(0.0, 0.0, 1.0), p3(0.0, 0.0, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (id, _) = doc
        .revolve(
            RevolveProfile::Region { sketch: s, region },
            &RevolveSpec {
                axis_point: p3(0.0, 0.0, 0.0),
                axis: Vec3::new(0.0, 0.0, 1.0),
                angle: std::f64::consts::TAU,
                segments: 24,
            },
        )
        .unwrap();
    doc.object(id).unwrap().clone()
}

/// Every analytic class carried 1,000 km out, turned 0.7 rad about
/// (1, 1, 0) and scaled 1.7 there keeps every claim, validates and
/// reopens from its own encoding on the same planes.
#[test]
fn every_surface_class_far_out_keeps_its_claims_and_reopens() {
    let mut doc = Document::new();
    let cyl = far_cylinder(&mut doc, 0.0);
    let solids = [
        ("cylinder", doc.object(cyl).unwrap().clone()),
        ("sphere", common::curves::revolved_dome(8, 24)),
        ("torus", revolved(true)),
        ("cone", revolved(false)),
    ];
    let far = 1e6;
    let pivot = Point3::new(far, far, 0.1);
    let about = |l: Transform| {
        Transform::translation(Point3::ORIGIN - pivot)
            .then(&l)
            .then(&Transform::translation(pivot - Point3::ORIGIN))
    };
    for (name, solid) in solids {
        let claims = common::curves::claim_census(&solid);
        assert!(claims.contains_key(name), "{name}: {claims:?}");
        let mut obj = solid.clone();
        for (what, t) in [
            ("moved", Transform::translation(Vec3::new(far, far, 0.0))),
            (
                "turned",
                about(Transform::rotation(Vec3::new(1.0, 1.0, 0.0), 0.7).unwrap()),
            ),
            ("scaled", about(Transform::uniform_scale(1.7))),
        ] {
            let what = format!("{name} {what}");
            obj.apply_transform(&t)
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            obj.validate().unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(common::curves::claim_census(&obj), claims, "{what}");
            reopens_on_its_planes(&obj, &what);
        }
    }
}

/// Far enough out, the claim check is what refuses: a cylinder 3,000 km out
/// turned about any of these axes leaves a wall facet no plane that
/// reopens from a file and agrees with its mapped cylinder, so the turn
/// answers `face_not_flat` with the document, the object's bytes and its
/// claims untouched. (With the claim test removed from the plane search
/// these turns commit, so the refusal is the claim's.)
#[test]
fn a_cylinder_turned_far_out_is_refused_by_its_claim_with_nothing_moved() {
    let cases = [
        (3e6, Vec3::new(1.0, 1.0, 0.0), 0.7),
        (3e6, Vec3::new(0.2, -1.0, 0.4), 1.3),
        (3e6, Vec3::new(-0.7, 0.1, 0.3), 0.3),
    ];
    for (far, axis, angle) in cases {
        let mut doc = Document::new();
        let id = far_cylinder(&mut doc, far);
        let pivot = Point3::new(far, far, 0.2);
        let turn = Transform::translation(Point3::ORIGIN - pivot)
            .then(&Transform::rotation(axis, angle).unwrap())
            .then(&Transform::translation(pivot - Point3::ORIGIN));
        let what = format!("{far} m, {axis:?} by {angle}");
        let bytes = doc.object(id).unwrap().encode(&|_| 0);
        let hash = doc.state_hash();
        let r = doc.transform_object(id, &turn);
        assert!(
            matches!(
                r,
                Err(kernel::DocumentError::Transform(
                    kernel::TransformError::FaceNotFlat
                ))
            ),
            "{what}: {r:?}"
        );
        assert_eq!(doc.object(id).unwrap().encode(&|_| 0), bytes, "{what}");
        assert_eq!(doc.state_hash(), hash, "{what}: document unchanged");
        assert_eq!(claimed(doc.object(id).unwrap()), 48, "{what}");
        doc.object(id).unwrap().validate().unwrap();
    }
}
