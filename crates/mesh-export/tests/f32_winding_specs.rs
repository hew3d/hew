//! The writers store `f32`, and a triangle thinner than the `f32` quantum of
//! its coordinates winds backwards or collapses once rounded (the render
//! path's `tessellate::tri_winds_forward_in_f32` says how ear clipping
//! makes one: the last fan triangle before a vertex's tangent to a hole).
//! STL, GLB, and USDZ leave such a triangle out rather than write it with
//! a normal that contradicts its vertices; a triangle with any
//! representable altitude is written as before.
//!
//! The configuration is the one the tessellation's property test first
//! found (`crates/tessellate/tests/touching_hole_specs.rs`,
//! `a_fan_tangent_to_a_hole_writes_no_inverted_sliver`): a 2.9 × 2.6 box
//! top with a 44-facet circle on its right edge and a half-radius circle
//! touching it. Its sliver inverts at meter scale (GLB, USDZ) and survives
//! at millimeter scale (STL): 1000 is not a power of two, so each writer
//! is checked in its own coordinates. A fixed sweep over a subset of the
//! lattice the property test draws from (two box sizes, the rim at one
//! position, radii 1–2, three facet counts, with and without the second
//! circle) holds every writer to the same invariant; it is a deterministic
//! sample, not the property test's coverage.
//!
//! Every triangle of these boxes lies on one of the six bounding planes
//! (the circles are imprints on the top, not pushed), so "winds outward"
//! is exact: a triangle with a constant coordinate on the box's maximum
//! (minimum) side must wind with +axis (−axis), strictly — no collapsed
//! triangle either.

use std::f64::consts::{PI, TAU};
use std::io::Cursor;

use kernel::{CurveGeom, Document, ObjectId, Plane, Point3};

type Tri = [[f64; 3]; 3];

// ------------------------------------------------------------ the model

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground")
}

/// A `w × d × 1` box, its top face's plane.
fn boxed(w: f64, d: f64) -> (Document, ObjectId, Plane) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(w, 0.0, 0.0),
        Point3::new(w, d, 0.0),
        Point3::new(0.0, d, 0.0),
    ];
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
    }
    let r = doc.extrudable_regions(s).expect("regions")[0];
    let (id, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .expect("top");
    (doc, id, top)
}

/// `n` points of the circle about `(cx, cy)` on the top (z = 1), vertex `i`
/// at angle `phase + TAU i / n`.
fn ring(cx: f64, cy: f64, r: f64, n: usize, phase: f64) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = phase + TAU * i as f64 / n as f64;
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), 1.0)
        })
        .collect()
}

/// Draws `pts` as one closed curve on the top through one draft.
fn draw(doc: &mut Document, id: ObjectId, top: Plane, pts: &[Point3], cx: f64, cy: f64, r: f64) {
    doc.draft(top, None, &[id], "Circle", |sk| {
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(cx, cy, 1.0),
            radius: r,
        })?;
        for i in 0..pts.len() {
            sk.add_segment(pts[i], pts[(i + 1) % pts.len()])?;
        }
        sk.end_curve();
        Ok(())
    })
    .expect("the circle draws");
}

/// The lattice case the tessellation's property test draws from (the same
/// builder as `touching_hole_specs.rs`): a `wq × dq` box (in 0.1 m units)
/// with a circle of radius `rq` (same units) and `4·n4` facets, its rim
/// snapped onto edge `side` (0 bottom, 1 right, 2 top, 3 left) at fraction
/// `along`/10 of that edge's free run, and with `second` a half-radius
/// circle touching the first at its far point when it fits clear of every
/// edge. `None` when the circle does not fit the box.
fn touching_case(
    wq: u32,
    dq: u32,
    side: u8,
    rq: u32,
    along: u32,
    n4: usize,
    second: bool,
) -> Option<(Document, ObjectId)> {
    let q = 0.1;
    let (w, d) = (f64::from(wq) * q, f64::from(dq) * q);
    let r = f64::from(rq) * q;
    let n = n4 * 4;
    let t = f64::from(along) / 10.0;
    let (cx, cy, phase) = match side {
        0 => (r + t * (w - 2.0 * r), r, -PI / 2.0),
        1 => (w - r, r + t * (d - 2.0 * r), 0.0),
        2 => (r + t * (w - 2.0 * r), d - r, PI / 2.0),
        _ => (r, r + t * (d - 2.0 * r), PI),
    };
    if !(w > 2.0 * r && d > 2.0 * r) {
        return None;
    }
    let (mut doc, id, top) = boxed(w, d);
    let mut a = ring(cx, cy, r, n, phase);
    a[0] = match side {
        0 => Point3::new(a[0].x, 0.0, 1.0),
        1 => Point3::new(w, a[0].y, 1.0),
        2 => Point3::new(a[0].x, d, 1.0),
        _ => Point3::new(0.0, a[0].y, 1.0),
    };
    draw(&mut doc, id, top, &a, cx, cy, r);
    if second {
        let far = a[n / 2];
        let r2 = 0.5 * r;
        let (dx, dy) = (far.x - cx, far.y - cy);
        let len = dx.hypot(dy);
        let (c2x, c2y) = (far.x + dx / len * r2, far.y + dy / len * r2);
        let m = r2 + 1e-3;
        let inside = |x: f64, y: f64| x > m && y > m && x < w - m && y < d - m;
        if inside(c2x, c2y) {
            let mut b = ring(c2x, c2y, r2, n, phase);
            b[0] = far;
            draw(&mut doc, id, top, &b, c2x, c2y, r2);
        }
    }
    Some((doc, id))
}

/// The fan case: the sliver at the far top-left corner's tangent to the
/// small circle inverts at meter scale.
fn fan_case() -> (Document, ObjectId) {
    touching_case(29, 26, 1, 2, 9, 11, true).expect("the circles fit")
}

// ---------------------------------------------------------- the source

/// How many of the object's export triangles the writer's gate leaves out
/// at `scale` (meters × scale is what the writer rounds) — the gate itself,
/// re-asked through the shared helper, so an output count can be tied to
/// the source: written = source − gated.
fn gated_at(doc: &Document, id: ObjectId, scale: f64) -> (usize, usize) {
    let soup = tessellate::export_triangles(doc.object(id).unwrap(), 0).expect("tessellates");
    let mut gated = 0;
    for tri in soup.chunks(9) {
        let p = |i: usize| Point3::new(tri[i] * scale, tri[i + 1] * scale, tri[i + 2] * scale);
        let (a, b, c) = (p(0), p(3), p(6));
        if !tessellate::winds_forward_in_f32(a, b, c, (b - a).cross(c - a)) {
            gated += 1;
        }
    }
    (soup.len() / 9, gated)
}

// ---------------------------------------------------------- the readers

/// The facets of a binary STL: stored normal and `f32` corners (mm).
fn stl_facets(stl: &[u8]) -> Vec<([f64; 3], Tri)> {
    let n = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
    assert_eq!(stl.len(), 84 + 50 * n, "a well-formed binary STL");
    (0..n)
        .map(|t| {
            let at = 84 + 50 * t;
            let f = |k: usize| {
                f64::from(f32::from_le_bytes(
                    stl[at + 4 * k..at + 4 * k + 4].try_into().unwrap(),
                ))
            };
            (
                [f(0), f(1), f(2)],
                [[f(3), f(4), f(5)], [f(6), f(7), f(8)], [f(9), f(10), f(11)]],
            )
        })
        .collect()
}

/// Every triangle of a GLB, in Hew's Z-up frame (the writer stores Y-up:
/// `(x, y, z) → (x, z, −y)`).
fn glb_triangles(bytes: &[u8]) -> Vec<Tri> {
    assert_eq!(&bytes[0..4], b"glTF");
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    assert_eq!(&bytes[16..20], b"JSON");
    let json: serde_json::Value =
        serde_json::from_slice(&bytes[20..20 + json_len]).expect("valid JSON chunk");
    let bin_header = 20 + json_len;
    let bin_len =
        u32::from_le_bytes(bytes[bin_header..bin_header + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[bin_header + 4..bin_header + 8], b"BIN\0");
    let bin = &bytes[bin_header + 8..bin_header + 8 + bin_len];

    let view_bytes = |accessor: usize| -> &[u8] {
        let acc = &json["accessors"][accessor];
        let bv = &json["bufferViews"][acc["bufferView"].as_u64().unwrap() as usize];
        let start = bv["byteOffset"].as_u64().unwrap() as usize
            + acc["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = bv["byteLength"].as_u64().unwrap() as usize;
        &bin[start..start + len]
    };
    let mut out = Vec::new();
    for mesh in json["meshes"].as_array().unwrap() {
        for prim in mesh["primitives"].as_array().unwrap() {
            let pos: Vec<f64> =
                view_bytes(prim["attributes"]["POSITION"].as_u64().unwrap() as usize)
                    .chunks(4)
                    .map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap())))
                    .collect();
            let idx: Vec<usize> = view_bytes(prim["indices"].as_u64().unwrap() as usize)
                .chunks(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
                .collect();
            let z_up = |i: usize| [pos[i * 3], -pos[i * 3 + 2], pos[i * 3 + 1]];
            for t in idx.chunks(3) {
                out.push([z_up(t[0]), z_up(t[1]), z_up(t[2])]);
            }
        }
    }
    out
}

/// Every triangle of a USDZ's `model.usda` layer, in Hew's Z-up frame.
fn usdz_triangles(bytes: Vec<u8>) -> Vec<Tri> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a valid zip");
    let mut usda = String::new();
    std::io::Read::read_to_string(
        &mut zip.by_name("model.usda").expect("model.usda"),
        &mut usda,
    )
    .unwrap();
    let mut out = Vec::new();
    for mesh in usda.split("def Mesh").skip(1) {
        let points_text = {
            let start =
                mesh.find("point3f[] points = [").expect("points") + "point3f[] points = [".len();
            let end = start + mesh[start..].find(']').expect("closing bracket");
            &mesh[start..end]
        };
        let points: Vec<[f64; 3]> = points_text
            .split('(')
            .skip(1)
            .map(|tuple| {
                let inner = &tuple[..tuple.find(')').expect("closing paren")];
                let v: Vec<f64> = inner
                    .split(',')
                    .map(|s| s.trim().parse().expect("a float"))
                    .collect();
                // Y-up → Z-up.
                [v[0], -v[2], v[1]]
            })
            .collect();
        let idx_text = {
            let start = mesh.find("faceVertexIndices = [").expect("indices")
                + "faceVertexIndices = [".len();
            let end = start + mesh[start..].find(']').expect("closing bracket");
            &mesh[start..end]
        };
        let idx: Vec<usize> = idx_text
            .split(',')
            .map(|s| s.trim().parse().expect("an index"))
            .collect();
        for t in idx.chunks(3) {
            out.push([points[t[0]], points[t[1]], points[t[2]]]);
        }
    }
    out
}

// -------------------------------------------------------- the invariant

fn cross(t: &Tri) -> [f64; 3] {
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

/// Every triangle lies on a bounding plane of the triangles' own box and
/// winds outward from it, strictly (a collapsed triangle fails too).
fn assert_winds_outward(tris: &[Tri], what: &str) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for t in tris {
        for p in t {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    for (i, t) in tris.iter().enumerate() {
        let k = (0..3)
            .find(|&k| t[0][k] == t[1][k] && t[1][k] == t[2][k])
            .unwrap_or_else(|| panic!("{what}: triangle {i} {t:?} is on no axis plane"));
        let c = t[0][k];
        let outward = if c == hi[k] {
            1.0
        } else if c == lo[k] {
            -1.0
        } else {
            panic!("{what}: triangle {i} {t:?} is on an interior plane")
        };
        let winding = cross(t)[k] * outward;
        assert!(
            winding > 0.0,
            "{what}: triangle {i} {t:?} winds {} (cross·outward = {winding})",
            if winding == 0.0 {
                "collapsed"
            } else {
                "inward"
            }
        );
    }
}

/// Every STL facet's stored normal agrees with its `f32` vertices.
fn assert_stl_normals_agree(facets: &[([f64; 3], Tri)], what: &str) {
    for (i, (n, t)) in facets.iter().enumerate() {
        let c = cross(t);
        let dot = c[0] * n[0] + c[1] * n[1] + c[2] * n[2];
        assert!(
            dot > 0.0,
            "{what}: facet {i} normal {n:?} contradicts its vertices {t:?}"
        );
    }
}

/// Exports `doc` through all three `f32` writers and holds each to the
/// invariant, tying its triangle count to the source less what its own
/// scale gates. Returns `(source, gated at mm, gated at m)`.
fn check_every_writer(doc: &Document, id: ObjectId, what: &str) -> (usize, usize, usize) {
    let (source, gated_mm) = gated_at(doc, id, 1000.0);
    let (_, gated_m) = gated_at(doc, id, 1.0);

    let stl = stl_facets(&mesh_export::export_stl(doc, 0, true).expect("stl"));
    let stl_tris: Vec<Tri> = stl.iter().map(|(_, t)| *t).collect();
    assert_winds_outward(&stl_tris, &format!("{what} STL"));
    assert_stl_normals_agree(&stl, &format!("{what} STL"));
    assert_eq!(
        stl.len(),
        source - gated_mm,
        "{what} STL: written = source − gated"
    );

    let glb = glb_triangles(&mesh_export::export_glb(doc, 0, true).expect("glb"));
    assert_winds_outward(&glb, &format!("{what} GLB"));
    assert_eq!(
        glb.len(),
        source - gated_m,
        "{what} GLB: written = source − gated"
    );

    let usdz = usdz_triangles(mesh_export::export_usdz(doc, 0, true).expect("usdz"));
    assert_winds_outward(&usdz, &format!("{what} USDZ"));
    assert_eq!(
        usdz.len(),
        source - gated_m,
        "{what} USDZ: written = source − gated"
    );

    (source, gated_mm, gated_m)
}

// ------------------------------------------------------------- the specs

/// The fan case at meter scale: exactly one source triangle inverts, and
/// GLB and USDZ write every other one, outward.
#[test]
fn a_fan_tangent_to_a_hole_exports_no_inverted_triangle_in_glb_and_usdz() {
    let (doc, id) = fan_case();
    doc.object(id).unwrap().validate().expect("valid");
    let (source, _, gated_m) = check_every_writer(&doc, id, "fan case");
    assert_eq!(
        gated_m, 1,
        "the configuration exercises the gate at meter scale"
    );
    assert!(
        source > 100,
        "a real face, not a trivial one: {source} triangles"
    );
}

/// The same configuration at millimeter scale: the sliver survives the mm
/// rounding (a different grid), so STL writes every source triangle, each
/// facet's stored normal agreeing with its vertices — the gate leaves a
/// normal export exactly as it was.
#[test]
fn a_fan_tangent_to_a_hole_exports_every_triangle_to_stl_with_agreeing_normals() {
    let (doc, id) = fan_case();
    let (source, gated_mm, _) = check_every_writer(&doc, id, "fan case");
    assert_eq!(gated_mm, 0, "nothing inverts at mm scale here");
    let stl = stl_facets(&mesh_export::export_stl(&doc, 0, true).expect("stl"));
    assert_eq!(stl.len(), source);
}

/// A plain box is unchanged by the gate: twelve triangles out of every
/// writer, outward, with STL normals agreeing.
#[test]
fn a_plain_box_still_exports_twelve_outward_triangles_everywhere() {
    let (doc, id, _) = boxed(2.0, 1.5);
    let (source, gated_mm, gated_m) = check_every_writer(&doc, id, "box");
    assert_eq!((source, gated_mm, gated_m), (12, 0, 0));
}

/// A fixed sweep over a subset of the lattice the tessellation's property
/// test draws from (see the module doc for what is left out), the same
/// every run: every writer, every case, outward and tied to the source,
/// with the fan case among them gating at meter scale.
#[test]
fn every_lattice_case_exports_outward_triangles_from_every_writer() {
    let mut cases = 0;
    let mut gated_somewhere = 0;
    for second in [true, false] {
        for (wq, dq) in [(29u32, 26u32), (22, 17)] {
            for side in 0..4u8 {
                for rq in 1..=2u32 {
                    for n4 in [6usize, 11, 18] {
                        let Some((doc, id)) = touching_case(wq, dq, side, rq, 9, n4, second) else {
                            continue;
                        };
                        let what = format!(
                            "case wq={wq} dq={dq} side={side} rq={rq} n4={n4} second={second}"
                        );
                        let (_, gated_mm, gated_m) = check_every_writer(&doc, id, &what);
                        cases += 1;
                        gated_somewhere += usize::from(gated_mm + gated_m > 0);
                    }
                }
            }
        }
    }
    assert!(cases >= 60, "{cases} cases");
    assert!(gated_somewhere >= 1, "the sweep exercises the gate");
}

/// The gate affects nothing a writer reads back for scale: the fan case's
/// GLB bounds are the box's on both corners (the sliver sits at the far
/// top-left corner, on the minimum-x side), so the left-out sliver cost no
/// coverage a viewer could measure.
#[test]
fn the_left_out_sliver_changes_no_extent() {
    let (doc, _) = fan_case();
    let glb = glb_triangles(&mesh_export::export_glb(&doc, 0, true).expect("glb"));
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for t in &glb {
        for p in t {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert!(
        near(lo[0], 0.0) && near(lo[1], 0.0) && near(lo[2], 0.0),
        "{lo:?}"
    );
    assert!(
        near(hi[0], 2.9) && near(hi[1], 2.6) && near(hi[2], 1.0),
        "{hi:?}"
    );
}
