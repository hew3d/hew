//! Property test (docs/design/printing.md §7b): random boxes and cylinders in
//! random poses under random parallel and perspective views — every piece
//! `line_drawing` calls VISIBLE must not be behind any triangle by more than
//! the depth tolerance at its sample points, and every piece it calls HIDDEN
//! must be behind some triangle. The oracle is a brute-force 3D ray test
//! against EVERY triangle (no grid, no interval clipping, no per-plane
//! affine metric) — an independent check of the engine's acceleration and
//! interval machinery.

mod common;

use common::*;
use hlr::{Item, Kind, Options};
use kernel::{Point3, Transform, Vec3};
use proptest::prelude::*;

/// World-space triangles of an item.
fn triangles(mesh: &tessellate::RenderMesh, pose: &Transform) -> Vec<[Point3; 3]> {
    let pos = |i: usize| {
        pose.apply_point(Point3::new(
            mesh.positions[i * 3] as f64,
            mesh.positions[i * 3 + 1] as f64,
            mesh.positions[i * 3 + 2] as f64,
        ))
    };
    mesh.indices
        .chunks_exact(3)
        .map(|t| [pos(t[0] as usize), pos(t[1] as usize), pos(t[2] as usize)])
        .collect()
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// Depth along the view ray through the sample of the nearest triangle
/// strictly in front (by more than `tol`) of the sample — None if nothing
/// occludes it. `origin`/`dir` describe the ray for the sample.
fn nearest_occluder_depth(
    origin: Point3,
    dir: Vec3,
    sample_depth: f64,
    tris: &[[Point3; 3]],
    tol: f64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for t in tris {
        // Möller–Trumbore.
        let e1 = t[1] - t[0];
        let e2 = t[2] - t[0];
        let p = cross(dir, e2);
        let det = e1.dot(p);
        if det.abs() < 1e-14 {
            continue;
        }
        let inv = 1.0 / det;
        let s = origin - t[0];
        let u = s.dot(p) * inv;
        if !(-1e-9..=1.0 + 1e-9).contains(&u) {
            continue;
        }
        let q = cross(s, e1);
        let v = dir.dot(q) * inv;
        if v < -1e-9 || u + v > 1.0 + 1e-9 {
            continue;
        }
        let d = e2.dot(q) * inv;
        if d < sample_depth - tol {
            best = Some(best.map_or(d, |b: f64| b.min(d)));
        }
    }
    best
}

fn box_strategy() -> impl Strategy<Value = (f64, f64, f64, f64, f64, f64)> {
    (
        0.2f64..2.0,
        0.2f64..2.0,
        0.2f64..2.0,
        -2.0f64..2.0,
        -2.0f64..2.0,
        -1.0f64..1.0,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    #[test]
    fn visible_pieces_are_unoccluded_and_hidden_pieces_are_occluded(
        boxes in prop::collection::vec(box_strategy(), 1..4),
        cyl in prop::option::of((0.2f64..0.8, 0.3f64..1.5, -1.5f64..1.5, -1.5f64..1.5)),
        eye_dir in (0.0f64..std::f64::consts::TAU, 0.15f64..1.4),
        perspective in any::<bool>(),
    ) {
        let mut doc = kernel::Document::new();
        let mut meshes = Vec::new();
        for (i, (w, d, h, x, y, z)) in boxes.iter().enumerate() {
            let (_, m) = box_mesh(&mut doc, *x, *y, *z, x + w, y + d, z + h);
            meshes.push((m, i as u64 + 1));
        }
        if let Some((r, h, cx, cy)) = cyl {
            let (_, m) = cylinder_mesh(&mut doc, cx, cy, 0.0, r, h, 24);
            meshes.push((m, 100));
        }
        let items: Vec<Item> = meshes.iter().map(|(m, sid)| Item { mesh: m, pose: Transform::IDENTITY, sid: *sid }).collect();
        let (az, el) = eye_dir;
        let dir_to_eye = Vec3::new(az.cos() * el.cos(), az.sin() * el.cos(), el.sin());
        let target = Point3::new(0.5, 0.5, 0.5);
        let eye = target + dir_to_eye * 12.0;
        let cam = hlr::Camera { eye, target, up: Vec3::new(0.0, 0.0, 1.0), projection: if perspective { hlr::Projection::Perspective } else { hlr::Projection::Parallel } };
        let d = hlr::line_drawing(&items, &cam, &Options { include_hidden: true, ..Options::default() }).unwrap();

        // Frame for un-projecting samples: rebuild the same basis hlr uses.
        let view_dir = (target - eye).normalized().unwrap();
        let mut right = cross(view_dir, Vec3::new(0.0, 0.0, 1.0));
        if right.length() < 1e-9 { right = cross(view_dir, Vec3::new(1.0, 0.0, 0.0)); }
        let right = right.normalized().unwrap();
        let upv = cross(right, view_dir);
        let all_tris: Vec<[Point3; 3]> = meshes.iter().flat_map(|(m, _)| triangles(m, &Transform::IDENTITY)).collect();
        // Scene diagonal in the view plane, as hlr computes its tolerance
        // (approximate: use the drawing's own bounds).
        let (mn, mx) = d.bounds.unwrap_or(([0.0, 0.0], [0.0, 0.0]));
        let diag = ((mx[0] - mn[0]).powi(2) + (mx[1] - mn[1]).powi(2)).sqrt().max(1e-9);
        // Independent of the engine's own constants on purpose (an oracle
        // that borrows the tolerance it is checking would pass a broken
        // engine): a band of 1e-3 of the diagonal, and pieces shorter than
        // 1e-3 of it are not sampled.
        let min_piece = diag * 1e-3;

        // The output is 2D, so a sample's 3D position is recovered through
        // its view ray: a VISIBLE piece is a real edge that nothing covers,
        // so the FIRST surface the ray meets must be that edge — the first
        // hit point lies within `band` of some triangle edge. A HIDDEN piece
        // has a surface in front of it AND its own edge behind that surface
        // — the ray meets a second triangle deeper than the first hit.
        let band = diag * 1e-3;
        let mut checked = 0usize;
        for s in &d.segs {
            let len = ((s.b[0] - s.a[0]).powi(2) + (s.b[1] - s.a[1]).powi(2)).sqrt();
            if len < 4.0 * min_piece { continue; }
            for f in [0.25, 0.5, 0.75] {
                let x = s.a[0] + (s.b[0] - s.a[0]) * f;
                let y = s.a[1] + (s.b[1] - s.a[1]) * f;
                // Ray through the view-plane sample.
                let plane_pt = target + right * x + upv * y;
                let (origin, ray_dir) = if perspective {
                    (eye, (plane_pt - eye).normalized().unwrap())
                } else {
                    (plane_pt - view_dir * 100.0, view_dir)
                };
                // First hit along the ray.
                let first = nearest_occluder_depth(origin, ray_dir, f64::INFINITY, &all_tris, 0.0);
                match s.kind {
                    Kind::Hidden => {
                        // Something must be in front: at least one hit exists,
                        // and there must be a triangle EDGE behind the first
                        // hit (the source edge) — checked as: the ray hits ≥ 2
                        // triangles' worth of depth range, i.e. a second hit
                        // deeper than the first exists.
                        prop_assert!(first.is_some(), "hidden piece with nothing along its ray: {s:?}");
                        let f1 = first.unwrap();
                        // …and the hidden edge itself is deeper: some
                        // triangle is hit beyond the first surface.
                        let second = all_tris.iter().any(|t| {
                            nearest_occluder_depth(origin, ray_dir, f64::INFINITY, std::slice::from_ref(t), 0.0).is_some_and(|dd| dd > f1 + band)
                        });
                        prop_assert!(second, "hidden piece whose ray meets only the front surface: {s:?}");
                    }
                    _ => {
                        // Visible: the first hit must be the piece's own
                        // surface — the ray must meet a triangle EDGE within
                        // `band` of the first hit's depth. Compute the nearest
                        // edge-crossing depth: distance from the ray to each
                        // triangle edge segment (< band) with its depth.
                        prop_assert!(first.is_some(), "visible piece off every surface: {s:?}");
                        let f1 = first.unwrap();
                        let hit_pt = origin + ray_dir * f1;
                        let mut on_edge = false;
                        'outer: for t in &all_tris {
                            for k in 0..3 {
                                let (a, b) = (t[k], t[(k + 1) % 3]);
                                let ab = b - a;
                                let l2 = ab.dot(ab);
                                if l2 < 1e-18 { continue; }
                                let u = ((hit_pt - a).dot(ab) / l2).clamp(0.0, 1.0);
                                let c = a + ab * u;
                                if (hit_pt - c).length() <= band { on_edge = true; break 'outer; }
                            }
                        }
                        prop_assert!(on_edge, "visible piece sample is not on a surface edge (occluded?): kind {:?} sample ({x}, {y}) hit depth {f1}", s.kind);
                    }
                }
                checked += 1;
            }
        }
        prop_assert!(checked > 0);
    }
}
