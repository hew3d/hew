// Test-only, and outside the determinism-critical kernel scope (kernel /
// inference / tessellate / mesh-heal): transcendentals here build the
// inputs and expectations of a native test, never a document a user keeps.
// Suppress the workspace clippy.toml ban for this integration-test crate.
#![allow(clippy::disallowed_methods)]

//! Print line art of a solid whose top holds a circle tangent to its front
//! edge at one point (a disc face in a hole touching the top's outer loop):
//! seen from above, the visible hard line work is the top square and the
//! circle, whole, and a pocket pushed from the disc draws the same outline.

mod common;

use common::*;
use hlr::{Item, Kind, Options};
use kernel::{CurveGeom, KernelOp, Plane, Point3, Transform};

fn draw_tangent_circle(doc: &mut kernel::Document, id: kernel::ObjectId) -> f64 {
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .unwrap();
    let n = 72;
    let mut ring: Vec<Point3> = (0..n)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 72.0;
            Point3::new(1.0 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin(), 1.0)
        })
        .collect();
    ring[0] = Point3::new(1.0, 0.0, 1.0);
    doc.draft(top, None, &[id], "Circle", |sk| {
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(1.0, 0.5, 1.0),
            radius: 0.5,
        })?;
        for i in 0..ring.len() {
            sk.add_segment(ring[i], ring[(i + 1) % ring.len()])?;
        }
        sk.end_curve();
        Ok(())
    })
    .unwrap();
    (0..ring.len())
        .map(|i| (ring[(i + 1) % ring.len()] - ring[i]).length())
        .sum()
}

fn top_view_hard_length(doc: &kernel::Document, id: kernel::ObjectId) -> f64 {
    let mesh = tessellate::tessellate(doc.object(id).unwrap(), doc.materials()).unwrap();
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([1.0, 1.0, 10.0], [1.0, 1.0, 0.5], [0.0, 1.0, 0.0]);
    let d = hlr::line_drawing(&items, &cam, &Options::default()).unwrap();
    total_len(&d, Kind::Hard)
}

#[test]
fn a_touching_hole_draws_the_square_and_the_whole_circle_from_above() {
    let mut doc = kernel::Document::new();
    let (id, _) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let circle = draw_tangent_circle(&mut doc, id);
    let len = top_view_hard_length(&doc, id);
    assert!((len - (8.0 + circle)).abs() < 1e-6, "hard line work {len}");
}

#[test]
fn a_pocket_from_a_touching_hole_draws_the_same_outline_from_above() {
    let mut doc = kernel::Document::new();
    let (id, _) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let circle = draw_tangent_circle(&mut doc, id);
    let obj = doc.object(id).unwrap();
    let disc = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9 && f.inner_loops.is_empty())
        .map(|(f, _)| f)
        .unwrap();
    doc.apply_object_op(
        id,
        KernelOp::ExtrudeSubFace {
            sub_face: disc,
            distance: -0.4,
        },
    )
    .expect("the pocket");
    // The pocket's floor rim lies straight under the top rim, so seen from
    // above the outline is the same square and circle.
    let len = top_view_hard_length(&doc, id);
    assert!((len - (8.0 + circle)).abs() < 1e-6, "hard line work {len}");
}
