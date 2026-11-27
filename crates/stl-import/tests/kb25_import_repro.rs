//! KB-25 reproduction through the real STL import pipeline (parse → heal →
//! coplanar merge → `Document::ingest`): a finely faceted sphere in
//! millimeters, then a boolean with a native disc overlapping its base —
//! the shape of the reported failure (a sculpted pawn on a base disc).

use kernel::trig;
use kernel::{BooleanOp, CurveGeom, Document, DocumentError, ObjectId, Plane, Point3, Vec3};

/// Binary STL bytes of a UV sphere (`n_lat` × `n_lon`, radius `r` in the
/// file's units), triangles only, f32 as STL stores them.
pub fn sphere_stl(n_lat: usize, n_lon: usize, r: f64) -> Vec<u8> {
    noisy_sphere_stl(n_lat, n_lon, r, 0.0)
}

/// [`sphere_stl`] with a deterministic radial ripple of relative amplitude
/// `noise` (a sculpt's surface: no two neighbouring triangles coplanar).
pub fn noisy_sphere_stl(n_lat: usize, n_lon: usize, r: f64, noise: f64) -> Vec<u8> {
    let p = |i: usize, j: usize| -> [f32; 3] {
        let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * i as f64 / n_lat as f64;
        let lon = std::f64::consts::TAU * (j % n_lon) as f64 / n_lon as f64;
        let rr = r
            * (1.0
                + noise
                    * trig::cos(lat)
                    * (trig::sin(7.0 * lat) * trig::cos(11.0 * lon)
                        + trig::sin(5.0 * lon) * trig::cos(13.0 * lat)));
        [
            (rr * trig::cos(lat) * trig::cos(lon)) as f32,
            (rr * trig::cos(lat) * trig::sin(lon)) as f32,
            (rr * trig::sin(lat)) as f32,
        ]
    };
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    for i in 0..n_lat {
        for j in 0..n_lon {
            let (a, b, c, d) = (p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j));
            if i > 0 {
                tris.push([a, b, c]);
            }
            if i + 1 < n_lat {
                tris.push([a, c, d]);
            }
        }
    }
    let mut out = vec![0u8; 80];
    out.extend((tris.len() as u32).to_le_bytes());
    for t in &tris {
        out.extend([0f32; 3].iter().flat_map(|v| v.to_le_bytes()));
        for v in t {
            out.extend(v.iter().flat_map(|c| c.to_le_bytes()));
        }
        out.extend([0u8; 2]);
    }
    out
}

/// Imports `stl` (millimeters) into `doc`; returns the one object.
pub fn import_into(doc: &mut Document, stl: &[u8]) -> ObjectId {
    let before = doc.visible_object_ids();
    let scene = stl_import::import(stl, 0.001, Some("sphere")).unwrap();
    doc.ingest(scene.scene, scene.missing).unwrap();
    let ids: Vec<ObjectId> = doc
        .visible_object_ids()
        .into_iter()
        .filter(|o| !before.contains(o))
        .collect();
    assert_eq!(ids.len(), 1, "one imported object");
    ids[0]
}

/// A native disc of radius `r` (m), 10 mm thick, whose top sits `overlap`
/// above the sphere's bottom at z = −`sphere_r`.
pub fn disc(doc: &mut Document, r: f64, sphere_r: f64, overlap: f64) -> ObjectId {
    let z0 = -sphere_r + overlap - 0.01;
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, z0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(0.0, 0.0, z0),
            radius: r,
        })
        .unwrap();
        let n = 48;
        let q = |i: usize| {
            let a = std::f64::consts::TAU * (i % n) as f64 / n as f64;
            Point3::new(r * trig::cos(a), r * trig::sin(a), z0)
        };
        for i in 0..n {
            sk.add_segment(q(i), q(i + 1)).unwrap();
        }
        sk.end_curve();
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 0.01).unwrap().0
}

fn union(n_lat: usize, n_lon: usize) -> Result<ObjectId, DocumentError> {
    union_stl(&sphere_stl(n_lat, n_lon, 20.0))
}

fn union_stl(stl: &[u8]) -> Result<ObjectId, DocumentError> {
    let mut doc = Document::new();
    let s = import_into(&mut doc, stl);
    eprintln!(
        "imported: {} faces, planarity tol {:?}",
        doc.object(s).unwrap().faces().len(),
        doc.object(s).unwrap().watertight()
    );
    let d = disc(&mut doc, 0.015, 0.02, 0.003);
    doc.boolean(BooleanOp::Union, s, d).map(|(o, _)| o)
}

#[test]
fn kb25_import_union_with_overlapping_disc() {
    let r = union(60, 120);
    assert!(r.is_ok(), "union refused: {r:?}");
}

#[test]
fn kb25_probe() {
    if std::env::var("KB25_PROBE").is_err() {
        return;
    }
    let (a, b, noise): (usize, usize, f64) = (
        std::env::var("KB25_LAT").unwrap().parse().unwrap(),
        std::env::var("KB25_LON").unwrap().parse().unwrap(),
        std::env::var("KB25_NOISE").unwrap().parse().unwrap(),
    );
    let t = std::time::Instant::now();
    let r = union_stl(&noisy_sphere_stl(a, b, 20.0, noise));
    eprintln!(
        "probe {a}x{b} noise {noise}: {:?} in {:?}",
        r.map(|_| ()),
        t.elapsed()
    );
}

#[test]
fn kb25_import_noisy_sphere_unions_with_a_disc() {
    // The reported shape: a sculpt-like surface (no two neighboring
    // triangles coplanar, merged by import into faces flat only to the import
    // tolerance) on a native base disc.
    let r = union_stl(&noisy_sphere_stl(60, 120, 20.0, 0.01));
    assert!(r.is_ok(), "union refused: {r:?}");
}

#[test]
fn kb25_import_with_zero_area_triangles_succeeds_or_names_the_cause() {
    // Two zero-area triangles (collinear corners on existing edges), as in
    // the reported pawn: import heals them, and the union either succeeds or
    // refuses with a named cause — never the misleading degenerate contact.
    let mut stl = noisy_sphere_stl(60, 120, 20.0, 0.01);
    let count = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
    let tri = |k: usize| -> [[f32; 3]; 3] {
        let base = 84 + k * 50 + 12;
        let v = |i: usize| -> [f32; 3] {
            let at = |c: usize| {
                f32::from_le_bytes(
                    stl[base + 12 * i + 4 * c..base + 12 * i + 4 * c + 4]
                        .try_into()
                        .unwrap(),
                )
            };
            [at(0), at(1), at(2)]
        };
        [v(0), v(1), v(2)]
    };
    let mut extra: Vec<[[f32; 3]; 3]> = Vec::new();
    for k in [count / 3, 2 * count / 3] {
        let [a, b, _] = tri(k);
        let m = [
            (a[0] + b[0]) / 2.0,
            (a[1] + b[1]) / 2.0,
            (a[2] + b[2]) / 2.0,
        ];
        extra.push([a, m, b]);
    }
    stl[80..84].copy_from_slice(&((count + extra.len()) as u32).to_le_bytes());
    for t in &extra {
        stl.extend([0f32; 3].iter().flat_map(|v| v.to_le_bytes()));
        for v in t {
            stl.extend(v.iter().flat_map(|c| c.to_le_bytes()));
        }
        stl.extend([0u8; 2]);
    }
    match union_stl(&stl) {
        Ok(_) => {}
        Err(DocumentError::Boolean(kernel::BooleanError::DegenerateContact)) => {
            panic!("refused as a bogus degenerate contact")
        }
        Err(e) => eprintln!("named refusal: {e:?}"),
    }
}
