//! Booleans on a real sculpted STL, through the real import. The file is
//! not in the repository: set `HEW_PAWN_STL` to `Cats_Pawn.stl` (from the
//! "Dogs v Cats Chess Set", units mm) and run with `--ignored --release`.
//! The kernel's `boolean_tidy_specs.rs` covers the same ground on an
//! in-repo dense mesh, so CI does not need the file.
//!
//! The STL healer merges neighboring triangles whose normals agree into one
//! polygon, so a sculpt carries thousands of faces that are flat only to
//! the import tolerance. A boolean against them used to refuse as a
//! degenerate contact; each of these must now return a watertight solid.
//!
//! A cut through a dense mesh also leaves corner slivers at every size down
//! to the merge distance where it passes near a mesh vertex. The boolean
//! merges every one its own cut leaves narrower than `tol::FEATURE_MIN`
//! (0.1 µm) into its neighbors and reports it (`BooleanTidy`), so every
//! placement commits with no nudge: three booleans at seven cutter
//! placements, then the box's top plane aimed at ±2 to 150 nm from the
//! pawn vertex nearest it. Each result must be solid and valid with a sane
//! volume, keep no cut face under the feature floor, save and reopen to
//! the same state hash, tessellate, and undo to the start. The plane at
//! ±0.5 nm, inside the merge distance, is run and printed but may refuse:
//! a known limit of the arrangement, not of the tidy (see the attempt
//! loop).

use kernel::trig;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use kernel::ObjectId;
use kernel::tol;
use kernel::{BooleanOp, BooleanTidy, Document, Object, Plane, Point3, Vec3};

/// The kernel's boolean log line, captured: `faces_split` and
/// `faces_passed_through` from the last `boolean_refine` event.
#[derive(Default)]
struct Counts {
    split: Option<u64>,
    passed: Option<u64>,
}

struct Capture(Arc<Mutex<Counts>>);

#[derive(Default)]
struct Visit {
    op: String,
    split: Option<u64>,
    passed: Option<u64>,
}

impl tracing::field::Visit for Visit {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "faces_split" => self.split = Some(value),
            "faces_passed_through" => self.passed = Some(value),
            _ => {}
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "op" {
            self.op = value.to_string();
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target() == "kernel::op" {
            let mut v = Visit::default();
            event.record(&mut v);
            if v.op == "boolean_refine" {
                let mut c = self.0.lock().unwrap();
                c.split = v.split;
                c.passed = v.passed;
            }
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// A native prism: the regular `n`-gon of radius `r` about `(cx, cy)` on
/// the plane `z = z0`, extruded to `z1`.
fn add_disc(doc: &mut Document, cx: f64, cy: f64, r: f64, n: usize, z0: f64, z1: f64) -> ObjectId {
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            (cx + r * trig::cos(a), cy + r * trig::sin(a))
        })
        .collect();
    add_prism(doc, &pts, z0, z1)
}

fn add_box(doc: &mut Document, lo: Point3, hi: Point3) -> ObjectId {
    let pts = [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)];
    add_prism(doc, &pts, lo.z, hi.z)
}

fn add_prism(doc: &mut Document, xy: &[(f64, f64)], z0: f64, z1: f64) -> ObjectId {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, z0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..xy.len() {
        let (a, b) = (xy[i], xy[(i + 1) % xy.len()]);
        sk.add_segment(Point3::new(a.0, a.1, z0), Point3::new(b.0, b.1, z0))
            .unwrap();
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, z1 - z0).unwrap().0
}

/// Largest distance of any face vertex from its stored plane, and how many
/// faces are past the native `PLANE_DIST`.
fn census(obj: &Object) -> (usize, f64) {
    let mut count = 0;
    let mut worst = 0.0f64;
    for f in obj.faces().values() {
        let d = std::iter::once(f.outer_loop)
            .chain(f.inner_loops.iter().copied())
            .flat_map(|l| obj.loop_positions(l))
            .map(|p| f.plane.signed_distance(p).abs())
            .fold(0.0, f64::max);
        if d > tol::PLANE_DIST {
            count += 1;
        }
        worst = worst.max(d);
    }
    (count, worst)
}

fn bounds(obj: &Object) -> (Point3, Point3) {
    let mut lo = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for v in obj.vertices().values() {
        let p = v.position;
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

/// Every vertex position of `obj`, by bits.
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

/// Faces whose outer ring is under the old area-only floor (twice the
/// area under `NORMALIZE_MIN_LENGTH`), the thinnest loop in the object,
/// and the thinnest loop holding a point that is no vertex of `operands`
/// (a loop the cut made).
fn sliver_census(obj: &Object, operands: &BTreeSet<Bits>) -> (usize, f64, f64) {
    let mut under = 0;
    let mut thinnest = f64::INFINITY;
    let mut thinnest_cut = f64::INFINITY;
    for f in obj.faces().values() {
        let ring: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
        if Plane::from_polygon_import(&ring).is_err() {
            under += 1;
        }
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let ring: Vec<Point3> = obj.loop_positions(l).collect();
            let w = width(&ring);
            thinnest = thinnest.min(w);
            if ring.iter().any(|&p| !operands.contains(&bits(p))) {
                thinnest_cut = thinnest_cut.min(w);
            }
        }
    }
    (under, thinnest, thinnest_cut)
}

/// The cutter offsets each boolean is run at (mm): the unshifted placement,
/// then six shifted ones (each refused `arrangement_failed` under the old
/// area-only face floor, and seven of the twenty-one refused
/// `result_too_thin` before the tidy).
const SHIFTS_MM: [(f64, f64, f64); 7] = [
    (0.0, 0.0, 0.0),
    (0.1, 0.0, 0.0),
    (-0.1, 0.0, 0.0),
    (0.0, 0.1, 0.0),
    (0.0, -0.1, 0.0),
    (0.0, 0.0, 0.1),
    (0.037, 0.051, -0.023),
];

/// How far the box's top plane is put from the pawn vertex nearest it, in
/// nanometers, either side.
const AIMED_NM: [f64; 7] = [0.5, 2.0, 5.0, 11.4, 30.0, 80.0, 150.0];

/// The unshifted results' volumes from the runs before the tidy (m³).
const DISC_UNION: f64 = 1.6527e-5;
const BOX_UNION: f64 = 1.5017e-5;
const BAR_SUBTRACT: f64 = 1.4123e-5;

#[test]
#[ignore]
fn pawn_booleans_return_watertight_solids() {
    let path = std::env::var("HEW_PAWN_STL").expect("set HEW_PAWN_STL to Cats_Pawn.stl");
    let bytes = std::fs::read(path).unwrap();
    let counts = Arc::new(Mutex::new(Counts::default()));
    tracing::subscriber::set_global_default(Capture(counts.clone())).unwrap();

    let t = Instant::now();
    let scene = stl_import::import(&bytes, 0.001, Some("Cats_Pawn")).unwrap();
    let mut doc = Document::new();
    doc.ingest(scene.scene, scene.missing).unwrap();
    let pawn = doc.visible_object_ids()[0];
    let obj = doc.object(pawn).unwrap();
    let (non_flat, worst) = census(obj);
    let (lo, hi) = bounds(obj);
    let pawn_volume = obj.volume().expect("the pawn is solid");
    println!(
        "import {:.2?}: {} faces, {} vertices, {non_flat} non-flat (worst {worst:.3e} m), \
         bounds {lo:?}..{hi:?}, volume {pawn_volume:.6e} m3",
        t.elapsed(),
        obj.faces().len(),
        obj.vertices().len(),
    );

    let (cx, cy) = ((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
    let w = (hi.x - lo.x).min(hi.y - lo.y);
    let mm = 0.001;
    let box_lo = Point3::new(cx - 0.1 * w, cy - 0.1 * w, lo.z + 12.0 * mm);
    let box_hi = Point3::new(cx + 0.7 * w, cy + 0.1 * w, lo.z + 18.0 * mm);
    type Make = Box<dyn Fn(&mut Document, Vec3) -> ObjectId>;
    let cases: Vec<(&str, BooleanOp, f64, Make)> = vec![
        (
            "union with a disc, z 25 to 30 mm",
            BooleanOp::Union,
            DISC_UNION,
            Box::new(move |d, o| {
                add_disc(
                    d,
                    cx + o.x,
                    cy + o.y,
                    0.45 * w,
                    48,
                    25.0 * mm + o.z,
                    30.0 * mm + o.z,
                )
            }),
        ),
        (
            "union with a box inside the body",
            BooleanOp::Union,
            BOX_UNION,
            Box::new(move |d, o| add_box(d, box_lo + o, box_hi + o)),
        ),
        (
            "subtract a bar through it",
            BooleanOp::Subtract,
            BAR_SUBTRACT,
            Box::new(move |d, o| {
                add_box(
                    d,
                    Point3::new(lo.x - 5.0 * mm, cy - 2.0 * mm, lo.z + 20.0 * mm) + o,
                    Point3::new(hi.x + 5.0 * mm, cy + 2.0 * mm, lo.z + 24.0 * mm) + o,
                )
            }),
        ),
    ];

    let mut failures = Vec::new();
    let mut tidied = 0;
    let t_all = Instant::now();
    for (name, op, expected, make) in &cases {
        for &(sx, sy, sz) in &SHIFTS_MM {
            let shift = Vec3::new(sx * mm, sy * mm, sz * mm);
            let label = format!("{name}, shifted ({sx}, {sy}, {sz}) mm");
            match attempt(
                &mut doc,
                &counts,
                &label,
                *op,
                pawn,
                pawn_volume,
                |d| make(d, shift),
                &mut failures,
            ) {
                Some((v, tidy)) => {
                    if !tidy.is_empty() {
                        tidied += 1;
                    }
                    if (sx, sy, sz) == (0.0, 0.0, 0.0) && ((v - expected) / expected).abs() > 1e-4 {
                        failures.push(format!(
                            "{label}: volume {v:.6e} is not the earlier {expected:.4e}"
                        ));
                    }
                }
                None => failures.push(format!("{label}: refused")),
            }
        }
    }
    println!(
        "{tidied} of {} placements tidied; all in {:.2?}",
        cases.len() * SHIFTS_MM.len(),
        t_all.elapsed()
    );

    // The box's top plane aimed at the pawn vertex nearest it, inside the
    // top's footprint, from either side.
    let margin = 0.5 * mm;
    let target = doc
        .object(pawn)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .filter(|p| {
            p.x > box_lo.x + margin
                && p.x < box_hi.x - margin
                && p.y > box_lo.y + margin
                && p.y < box_hi.y - margin
        })
        .min_by(|p, q| {
            (p.z - box_hi.z)
                .abs()
                .total_cmp(&(q.z - box_hi.z).abs())
                .then(p.x.total_cmp(&q.x))
        })
        .expect("a pawn vertex under the box top");
    println!("aimed at the pawn vertex {target:?}");
    for d_nm in AIMED_NM.iter().flat_map(|&d| [-d, d]) {
        let top = Point3::new(box_hi.x, box_hi.y, target.z + d_nm * 1e-9);
        let label = format!("union with the box, top {d_nm} nm from the vertex");
        // Within about a nanometer (the refused offsets are a lumpy set
        // within about 1.2 nm, some past the merge distance) the
        // arrangement can drop seam pieces under a nanometer around the
        // vertex and leave the cutter's top
        // unsplit (a known limit before any tidy; kernel
        // `boolean_tidy_specs::cuts_aimed_at_a_mesh_vertex_commit`): a
        // refusal there is printed, not a failure.
        if d_nm.abs() * 1e-9 < tol::POINT_MERGE {
            attempt(
                &mut doc,
                &counts,
                &label,
                BooleanOp::Union,
                pawn,
                pawn_volume,
                |d| add_box(d, box_lo, top),
                &mut failures,
            );
            continue;
        }
        if attempt(
            &mut doc,
            &counts,
            &label,
            BooleanOp::Union,
            pawn,
            pawn_volume,
            |d| add_box(d, box_lo, top),
            &mut failures,
        )
        .is_none()
        {
            failures.push(format!("{label}: refused"));
        }
    }

    // One placement twice: the same bytes.
    let shift = Vec3::new(0.037 * mm, 0.051 * mm, -0.023 * mm);
    let mut runs = Vec::new();
    for _ in 0..2 {
        let cutter = add_box(&mut doc, box_lo + shift, box_hi + shift);
        let (id, _) = doc.boolean(BooleanOp::Union, pawn, cutter).unwrap();
        runs.push(doc.object(id).unwrap().encode(&|_| 0));
        doc.undo().unwrap();
    }
    if runs[0] != runs[1] {
        failures.push("box union run twice: different bytes".into());
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// One boolean of the pawn against a fresh cutter. A result is checked
/// (solid, valid, sane volume, no cut face under the feature floor, a tidy
/// within its bounds that kept nothing, tessellates, saves and reopens to
/// the same state hash, undo returns to the start), with any shortfall
/// added to `failures`, and its volume and tidy returned; a refusal is
/// printed and `None` returned, the document untouched.
#[allow(clippy::too_many_arguments)]
fn attempt(
    doc: &mut Document,
    counts: &Arc<Mutex<Counts>>,
    name: &str,
    op: BooleanOp,
    pawn: ObjectId,
    pawn_volume: f64,
    make: impl Fn(&mut Document) -> ObjectId,
    failures: &mut Vec<String>,
) -> Option<(f64, BooleanTidy)> {
    // The cutter stays in the document beside the pawn afterwards; each
    // attempt draws its own.
    let cutter = make(doc);
    let before = doc.state_hash();
    let cutter_volume = doc.object(cutter).unwrap().volume().unwrap();
    let mut operands = vertex_bits(doc.object(pawn).unwrap());
    operands.extend(vertex_bits(doc.object(cutter).unwrap()));
    *counts.lock().unwrap() = Counts::default();
    let _ = doc.take_boolean_tidy();
    let t = Instant::now();
    let result = doc.boolean(op, pawn, cutter);
    let elapsed = t.elapsed();
    let tidy = doc.take_boolean_tidy();
    let (split, passed) = {
        let c = counts.lock().unwrap();
        (c.split, c.passed)
    };
    let answer = match result {
        Ok((id, _)) => {
            let r = doc.object(id).unwrap();
            let solid = doc.object_solid(id);
            let valid = r.validate();
            let v = r.volume();
            let (under, thinnest, thinnest_cut) = sliver_census(r, &operands);
            let sane = match (op, v) {
                (BooleanOp::Union, Some(v)) => {
                    v >= pawn_volume.max(cutter_volume) * (1.0 - 1e-9)
                        && v <= (pawn_volume + cutter_volume) * (1.0 + 1e-9)
                }
                (BooleanOp::Subtract, Some(v)) => {
                    v < pawn_volume && v >= (pawn_volume - cutter_volume) * (1.0 - 1e-9)
                }
                _ => false,
            };
            let faces = r.faces().len();
            let t_tess = Instant::now();
            let tess = tessellate::tessellate(r, doc.materials());
            let tess_time = t_tess.elapsed();
            let t_save = Instant::now();
            let saved = doc.save();
            let save_time = t_save.elapsed();
            let t_load = Instant::now();
            let reopened = Document::load(&saved);
            let load_time = t_load.elapsed();
            let reopen_same = reopened
                .as_ref()
                .map(|re| re.state_hash() == doc.state_hash())
                .unwrap_or(false);
            println!(
                "{name}: Ok in {elapsed:.2?}; split {split:?}, passed through {passed:?}; \
                 tidy merged {} kept {} moved {} (max {:.3e} m) faces split {} sites {} \
                 {:?}; {faces} faces ({under} under the old floor, thinnest {thinnest:.3e} m, \
                 thinnest cut {thinnest_cut:.3e} m), solid {solid}, valid {:?}, volume {v:?} \
                 (pawn {pawn_volume:.6e}, cutter {cutter_volume:.6e}, sane {sane}); \
                 tessellate {:?} in {tess_time:.2?}; save {} bytes in {save_time:.2?}, \
                 reopen {:?} in {load_time:.2?}, same hash {reopen_same}",
                tidy.slivers_merged,
                tidy.slivers_kept,
                tidy.points_moved,
                tidy.max_move,
                tidy.faces_split,
                tidy.sites_total,
                tidy.sites.first(),
                valid.as_ref().map(|_| ()),
                tess.as_ref().map(|m| m.indices.len() / 3),
                saved.len(),
                reopened.as_ref().map(|_| ()),
            );
            if !solid || valid.is_err() || !sane {
                failures.push(format!("{name}: not a valid, sane solid"));
            }
            if tidy.max_move > tol::TIDY_REACH || tidy.slivers_kept != 0 {
                failures.push(format!("{name}: tidy out of bounds {tidy:?}"));
            }
            if thinnest_cut < tol::FEATURE_MIN {
                failures.push(format!(
                    "{name}: a cut face {thinnest_cut:.3e} m wide is left"
                ));
            }
            if tess.is_err() {
                failures.push(format!("{name}: tessellate {:?}", tess.err()));
            }
            if !reopen_same {
                failures.push(format!("{name}: does not reopen identically"));
            }
            let volume = v.unwrap_or(f64::NAN);
            doc.undo().unwrap();
            assert_eq!(
                doc.state_hash(),
                before,
                "{name}: undo returns to the start"
            );
            Some((volume, tidy))
        }
        Err(e) => {
            println!("{name}: refused in {elapsed:.2?}: {e:?}");
            None
        }
    };
    assert_eq!(doc.state_hash(), before, "{name}: back at the start");
    answer
}
