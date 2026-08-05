//! Property campaign for the region-consume undo byte-identity fix (see
//! `region_consume_undo_reversal_specs.rs`'s module doc for the mechanism
//! and the fix itself, [`kernel::sketch::Sketch::remove_edges`]'s doc
//! comment for the ordering proof). DEVELOPMENT.md rule 3 requires
//! property coverage for a change to shared geometric machinery like this
//! one, not just the two fixed examples.
//!
//! Every spec here follows the same shape: draw some geometry, extrude (or
//! follow_me) a subset of it, undo everything, and assert `doc.save()`
//! reproduces the PRE-extrusion bytes exactly — the property the fix
//! establishes. A separate redo/undo round trip checks the same holds
//! after replaying the action, not just after a single undo.

use kernel::{
    CurveGeom, Document, FollowMePath, MIN_CIRCLE_SEGMENTS, Plane, Point3, SketchId,
    SketchRegionId, Vec3,
};
use proptest::prelude::*;
use std::collections::BTreeSet;

// ------------------------------------------------------------------ helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// One shape a generated sketch can hold — a rectangle or a circle, each
/// small enough to fit in its grid cell (see `draw_shape`).
#[derive(Debug, Clone)]
enum Shape {
    Rect { w: f64, h: f64 },
    Circle { r: f64 },
}

fn shape_strategy() -> impl Strategy<Value = Shape> {
    prop_oneof![
        (0.3..1.0f64, 0.3..1.0f64).prop_map(|(w, h)| Shape::Rect { w, h }),
        (0.2..0.6f64).prop_map(|r| Shape::Circle { r }),
    ]
}

/// Draws `shape` centered in grid cell `i` (cells are 4 units apart along
/// x, far wider than any shape's extent, so shapes never touch or weld
/// regardless of the random size/jitter) and returns the ONE region it
/// creates — found by diffing `extrudable_regions` against `before`, the
/// region set prior to drawing this shape.
fn draw_shape(
    doc: &mut Document,
    sketch: SketchId,
    i: usize,
    jitter: f64,
    shape: &Shape,
) -> SketchRegionId {
    let before: BTreeSet<SketchRegionId> = doc
        .extrudable_regions(sketch)
        .expect("sketch is live")
        .into_iter()
        .collect();
    let cx = i as f64 * 4.0 + jitter;
    let cy = jitter * 0.5;
    let sk = doc.sketch_mut(sketch).expect("sketch is live");
    match *shape {
        Shape::Rect { w, h } => {
            let (x0, y0, x1, y1) = (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
            let corners = [
                (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
                (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
                (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
                (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
            ];
            for (a, b) in corners {
                sk.add_segment(a, b).expect("rect segment");
            }
        }
        Shape::Circle { r } => {
            let center = Point3::new(cx, cy, 0.0);
            let n = MIN_CIRCLE_SEGMENTS;
            let ring: Vec<Point3> = (0..n)
                .map(|k| {
                    let a = std::f64::consts::TAU * k as f64 / n as f64;
                    center + Vec3::new(r * a.cos(), r * a.sin(), 0.0)
                })
                .collect();
            sk.begin_curve_with(CurveGeom { center, radius: r })
                .expect("curve opens");
            for k in 0..n {
                sk.add_segment(ring[k], ring[(k + 1) % n])
                    .expect("circle facet");
            }
            sk.end_curve();
        }
    }
    let after = doc.extrudable_regions(sketch).expect("sketch is live");
    after
        .into_iter()
        .find(|r| !before.contains(r))
        .expect("drawing a closed shape creates exactly one new region")
}

/// Undoes `n` actions, then redoes and undoes them again `rounds` times —
/// asserting `doc.save()` matches `expected` (the pre-extrusion snapshot)
/// after every undo, including the ones inside a redo/undo round trip.
fn assert_undo_redo_stable(doc: &mut Document, n: usize, expected: &[u8], rounds: usize) {
    for _ in 0..n {
        doc.undo().expect("undo");
    }
    assert_eq!(
        doc.save(),
        expected,
        "undoing every extrusion must restore the pre-extrusion bytes exactly"
    );
    for round in 0..rounds {
        for _ in 0..n {
            doc.redo().expect("redo");
        }
        for _ in 0..n {
            doc.undo().expect("undo");
        }
        assert_eq!(
            doc.save(),
            expected,
            "round {round}: an undo/redo/undo cycle must stay byte-stable"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Random small sketch (2-4 disjoint rectangles/circles), a random
    /// subset of its regions extruded, then every extrusion undone: the
    /// sketch's bytes must exactly reproduce the pre-extrusion save. Also
    /// covers the PARTIAL-consumption case whenever the random subset
    /// extrudes fewer than all the shapes (surviving, never-touched
    /// regions sit right alongside the restored ones).
    #[test]
    fn extrude_region_subset_undoes_byte_identically(
        picks in prop::collection::vec((shape_strategy(), any::<bool>(), 0.0..0.4f64, 0.2..0.8f64), 2..=4),
    ) {
        let mut doc = Document::new();
        let sketch = doc.add_sketch(ground());

        let mut region_ids = Vec::with_capacity(picks.len());
        for (i, (shape, _, jitter, _)) in picks.iter().enumerate() {
            region_ids.push(draw_shape(&mut doc, sketch, i, *jitter, shape));
        }

        let before = doc.save();
        let mut extruded = 0usize;
        for (region, (_, extrude, _, height)) in region_ids.iter().zip(picks.iter()) {
            if *extrude {
                doc.extrude_region(sketch, *region, *height)
                    .expect("extrude a freshly drawn, still-live region");
                extruded += 1;
            }
        }

        // Nothing extruded is the identity case; still worth confirming
        // there is nothing to undo and the bytes already match.
        if extruded == 0 {
            prop_assert_eq!(doc.save(), before);
        } else {
            assert_undo_redo_stable(&mut doc, extruded, &before, 2);
        }
    }

    /// Same shapes, but each extruded region is swept with `follow_me`
    /// around a shared ground-plane circle instead of straight-extruded —
    /// cheap because `follow_me`'s auto-orientation step
    /// (`Object::from_follow_me_auto`, design §2c) folds an arbitrarily
    /// oriented ground-plane profile onto the path automatically, so no
    /// per-shape path bookkeeping is needed.
    #[test]
    fn follow_me_region_subset_undoes_byte_identically(
        picks in prop::collection::vec((shape_strategy(), any::<bool>(), 0.0..0.4f64), 2..=4),
    ) {
        let mut doc = Document::new();
        let sketch = doc.add_sketch(ground());

        let mut region_ids = Vec::with_capacity(picks.len());
        for (i, (shape, _, jitter)) in picks.iter().enumerate() {
            region_ids.push(draw_shape(&mut doc, sketch, i, *jitter, shape));
        }

        // The path lives on its own sketch, far below the profile shapes'
        // grid (z = -5) so it can never intersect or weld with them.
        let path_plane = Plane::from_point_normal(
            Point3::new(0.0, 0.0, -5.0),
            Vec3::new(0.0, 0.0, 1.0),
        )
        .expect("path plane is well-defined");
        let path_sketch = doc.add_sketch(path_plane);
        let n = MIN_CIRCLE_SEGMENTS;
        let radius = 20.0; // comfortably larger than any profile shape
        let center = Point3::new(0.0, 0.0, -5.0);
        let ring: Vec<Point3> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                center + Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
            })
            .collect();
        let path_sk = doc.sketch_mut(path_sketch).expect("sketch is live");
        let curve = path_sk
            .begin_curve_with(CurveGeom { center, radius })
            .expect("curve opens");
        for k in 0..n {
            path_sk
                .add_segment(ring[k], ring[(k + 1) % n])
                .expect("circle facet");
        }
        path_sk.end_curve();
        let edges = path_sk.curve_edges(curve);
        let path = FollowMePath::SketchEdges {
            sketch: path_sketch,
            edges,
        };

        let before = doc.save();
        let mut extruded = 0usize;
        for (region, (_, extrude, _)) in region_ids.iter().zip(picks.iter()) {
            if *extrude {
                // A profile the auto-orientation fold cannot place (an
                // unlucky degenerate angle against this particular circle)
                // is skipped rather than failing the whole case — the
                // property under test is byte-identity of whatever DID
                // sweep, not that every random shape is sweepable.
                if doc.follow_me(sketch, *region, &path).is_ok() {
                    extruded += 1;
                }
            }
        }

        if extruded == 0 {
            prop_assert_eq!(doc.save(), before);
        } else {
            assert_undo_redo_stable(&mut doc, extruded, &before, 2);
        }
    }

    /// Slot-REUSE coverage — the class the two suites above cannot reach
    /// because they never mutate the sketch between consumptions: a first
    /// wave of shapes is drawn and fully extruded (freeing vertex/edge
    /// slots), then a second wave draws INTO those freed slots, with some
    /// rectangles additionally eraser-cut and re-closed through a fresh
    /// point (`Sketch::remove_edge` frees an edge slot whose endpoints
    /// survive — the other reuse shape). After all that, extruding and
    /// undoing the second wave must still be byte-identical: ascending
    /// slot-id order is NOT creation order here, which is exactly what
    /// broke the rows-only restore (see `RemovedScaffolding`).
    #[test]
    fn interleaved_slot_reuse_extrude_undoes_byte_identically(
        wave_a in prop::collection::vec((shape_strategy(), 0.0..0.4f64, 0.2..0.8f64), 1..=2),
        wave_b in prop::collection::vec((shape_strategy(), 0.0..0.4f64, 0.2..0.8f64, any::<bool>()), 1..=3),
    ) {
        let mut doc = Document::new();
        let sketch = doc.add_sketch(ground());

        // A keeper shape that is never extruded, so consuming all of wave
        // A cannot empty the sketch (an emptied sketch leaves the
        // document — Model D — and wave B would have nothing to draw on).
        draw_shape(&mut doc, sketch, 100, 0.0, &Shape::Rect { w: 0.5, h: 0.5 });

        // Wave A: draw and immediately consume — every slot it used goes
        // to the free list.
        for (i, (shape, jitter, height)) in wave_a.iter().enumerate() {
            let region = draw_shape(&mut doc, sketch, i, *jitter, shape);
            doc.extrude_region(sketch, region, *height)
                .expect("extrude a wave-A shape");
        }

        // Wave B: draw into the freed slots (cells offset far past wave
        // A's), optionally eraser-cutting a rectangle's bottom edge and
        // re-closing it through a point below — a fresh vertex in a high
        // slot welded to an edge in a reused low slot.
        let base = 10usize;
        let mut probes = Vec::with_capacity(wave_b.len());
        for (j, (shape, jitter, _, erase)) in wave_b.iter().enumerate() {
            let i = base + j;
            draw_shape(&mut doc, sketch, i, *jitter, shape);
            let cx = i as f64 * 4.0 + jitter;
            let cy = jitter * 0.5;
            if let (Shape::Rect { w, h }, true) = (shape, erase) {
                let (x0, y0, x1) = (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0);
                let a = Point3::new(x0, y0, 0.0);
                let b = Point3::new(x1, y0, 0.0);
                let sk = doc.sketch_mut(sketch).expect("sketch is live");
                let bottom = sk
                    .edges()
                    .iter()
                    .find(|(_, e)| {
                        let f = sk.vertices()[e.from].position;
                        let t = sk.vertices()[e.to].position;
                        (f == a && t == b) || (f == b && t == a)
                    })
                    .map(|(id, _)| id)
                    .expect("the rectangle's bottom edge exists");
                sk.remove_edge(bottom).expect("erase the bottom edge");
                let dip = Point3::new(cx, y0 - 0.3, 0.0);
                let sk = doc.sketch_mut(sketch).expect("sketch is live");
                sk.add_segment(a, dip).expect("re-close left");
                sk.add_segment(dip, b).expect("re-close right");
            }
            probes.push(Point3::new(cx, cy, 0.0));
        }

        // Extrude every wave-B region (re-queried by containment — the
        // eraser mutations invalidated the draw-time handles), undo them
        // all, and demand the post-wave-B bytes back exactly.
        let before = doc.save();
        for ((_, _, height, _), probe) in wave_b.iter().zip(probes.iter()) {
            let regions = doc.extrudable_regions(sketch).expect("sketch is live");
            let sk = doc.sketch(sketch).expect("sketch is live");
            let region = regions
                .into_iter()
                .find(|&r| sk.region_contains_point(r, *probe).expect("region is live"))
                .expect("a wave-B region contains its own cell center");
            doc.extrude_region(sketch, region, *height)
                .expect("extrude a wave-B shape");
        }
        assert_undo_redo_stable(&mut doc, wave_b.len(), &before, 2);
    }
}

/// Explicit, deterministic partial-consumption pin (not left to proptest's
/// random subset selection): two disjoint rectangles share one sketch;
/// only the first is extruded. Undo must restore the WHOLE sketch's bytes
/// exactly — the surviving second rectangle's scaffolding was never
/// touched, and the first's must come back in its original vertex/edge
/// slots, not the mirror-reversed ones the pre-fix `remove_edges` produced.
#[test]
fn partial_region_consumption_undoes_byte_identically() {
    let mut doc = Document::new();
    let sketch = doc.add_sketch(ground());
    let region_a = draw_shape(&mut doc, sketch, 0, 0.0, &Shape::Rect { w: 0.6, h: 0.4 });
    let _region_b = draw_shape(&mut doc, sketch, 1, 0.0, &Shape::Rect { w: 0.5, h: 0.7 });

    let before = doc.save();
    doc.extrude_region(sketch, region_a, 0.5)
        .expect("extrude the first rectangle only");
    doc.undo().expect("undo the extrusion");
    let after = doc.save();

    assert_eq!(
        before, after,
        "undoing a partial region consumption must restore the whole sketch byte-identically"
    );
}
