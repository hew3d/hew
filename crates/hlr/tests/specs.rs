//! Executable specs for `hlr::line_drawing` (docs/design/printing.md §7b).

mod common;

use common::*;
use hlr::{Item, Kind, Options, Section};
use kernel::{Point3, Transform, Vec3};

const TOL: f64 = 1e-6;

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn unit_cube_top_view_is_the_top_square_only() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([0.5, 0.5, 10.0], [0.5, 0.5, 0.5], [0.0, 1.0, 0.0]);
    let d = hlr::line_drawing(&items, &cam, &Options::default()).unwrap();
    // Four visible hard edges: the top square, 1 m each. The bottom square's
    // four edges are hidden under the top face; the four verticals project
    // to points and are dropped.
    assert_eq!(count(&d, Kind::Hard), 4, "{:?}", d.segs);
    assert!(near(total_len(&d, Kind::Hard), 4.0, 1e-6));
    let (mn, mx) = d.bounds.unwrap();
    assert!(
        near(mn[0], -0.5, TOL)
            && near(mx[0], 0.5, TOL)
            && near(mn[1], -0.5, TOL)
            && near(mx[1], 0.5, TOL)
    );
    // Hidden pieces on request: the bottom square, 4 m.
    let dh = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            include_hidden: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(
        near(total_len(&dh, Kind::Hidden), 4.0, 1e-6),
        "{:?}",
        dh.segs
    );
}

#[test]
fn unit_cube_iso_shows_nine_edges_and_hides_three() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([5.5, -4.5, 5.5], [0.5, 0.5, 0.5], [0.0, 0.0, 1.0]);
    let d = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            include_hidden: true,
            ..Options::default()
        },
    )
    .unwrap();
    // Every edge is either wholly visible or wholly hidden from a generic
    // isometric-ish direction: 9 visible, 3 hidden.
    assert_eq!(count(&d, Kind::Hard), 9, "{:?}", d.segs);
    assert_eq!(count(&d, Kind::Hidden), 3, "{:?}", d.segs);
    // Nothing is lost: visible + hidden projected length = all 12 edges'.
    let full = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            include_hidden: true,
            budget: u64::MAX,
            ..Options::default()
        },
    )
    .unwrap();
    let all: f64 = full
        .segs
        .iter()
        .map(|s| ((s.b[0] - s.a[0]).powi(2) + (s.b[1] - s.a[1]).powi(2)).sqrt())
        .sum();
    // Twelve unit edges projected: three directions × 4 edges each.
    let f = |v: Vec3| {
        let dir = (cam.target - cam.eye).normalized().unwrap();
        (v - dir * v.dot(dir)).length()
    };
    let expected = 4.0
        * (f(Vec3::new(1.0, 0.0, 0.0)) + f(Vec3::new(0.0, 1.0, 0.0)) + f(Vec3::new(0.0, 0.0, 1.0)));
    assert!(near(all, expected, 1e-6), "{all} vs {expected}");
}

#[test]
fn a_box_behind_another_is_hidden_where_it_is_covered() {
    let mut doc = kernel::Document::new();
    let (_, front) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_, back) = box_mesh(&mut doc, 0.25, 3.0, 0.0, 1.75, 4.0, 2.0);
    let items = [
        Item {
            mesh: &front,
            pose: Transform::IDENTITY,
            sid: 1,
        },
        Item {
            mesh: &back,
            pose: Transform::IDENTITY,
            sid: 2,
        },
    ];
    // Front view: looking along +Y from y = -10.
    let cam = parallel([0.5, -10.0, 0.5], [0.5, 0.0, 0.5], [0.0, 0.0, 1.0]);
    let d = hlr::line_drawing(&items, &cam, &Options::default()).unwrap();
    // The back box (1.5 wide × 2 tall) peeks out above (z 1..2) and to the
    // right (x 1..1.75) of the front unit cube. Its bottom edge (z=0) is
    // hidden from x=0.25..1 and visible 1..1.75.
    let back_bottom: Vec<_> = d
        .segs
        .iter()
        .filter(|s| {
            s.sid == 2 && s.kind == Kind::Hard && near(s.a[1], -0.5, TOL) && near(s.b[1], -0.5, TOL)
        })
        .collect();
    let len: f64 = back_bottom.iter().map(|s| (s.b[0] - s.a[0]).abs()).sum();
    // View x = world x - 0.5: visible run world x ∈ [1, 1.75] → 0.75. (The
    // back box's own rear bottom edge is hidden by its front face — a solid
    // seen from the front shows only its front square.)
    assert!(
        near(len, 0.75, 1e-6),
        "back bottom visible length {len}: {back_bottom:?}"
    );
    // The front cube: its front square (4 m); the rear square hides behind
    // its own front face; the four depth edges project to points.
    let front_len = d
        .segs
        .iter()
        .filter(|s| s.sid == 1)
        .map(|s| ((s.b[0] - s.a[0]).powi(2) + (s.b[1] - s.a[1]).powi(2)).sqrt())
        .sum::<f64>();
    assert!(near(front_len, 4.0, 1e-6), "{front_len}");
}

#[test]
fn flush_boxes_share_an_edge_without_phantom_lines() {
    // Two unit boxes side by side sharing the x=1 face: from the front the
    // shared vertical edges at x=1 are visible (both boxes draw them), and no
    // edge is hidden by the coplanar neighbour's face.
    let mut doc = kernel::Document::new();
    let (_, a) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_, b) = box_mesh(&mut doc, 1.0, 0.0, 0.0, 2.0, 1.0, 1.0);
    let items = [
        Item {
            mesh: &a,
            pose: Transform::IDENTITY,
            sid: 1,
        },
        Item {
            mesh: &b,
            pose: Transform::IDENTITY,
            sid: 2,
        },
    ];
    let cam = parallel([1.0, -10.0, 0.5], [1.0, 0.0, 0.5], [0.0, 0.0, 1.0]);
    let d = hlr::line_drawing(&items, &cam, &Options::default()).unwrap();
    // Each box: its front square, 4 m, fully visible — the neighbour's
    // coplanar side face (the shared face) never hides the shared vertical
    // edge, and neither front face hides the other's edges.
    for sid in [1, 2] {
        let l: f64 = d
            .segs
            .iter()
            .filter(|s| s.sid == sid && s.kind == Kind::Hard)
            .map(|s| ((s.b[0] - s.a[0]).powi(2) + (s.b[1] - s.a[1]).powi(2)).sqrt())
            .sum();
        assert!(near(l, 4.0, 1e-6), "sid {sid}: {l}");
    }
    // The shared edge at x=1 is present for BOTH boxes (two coincident 1 m
    // verticals at view x = 0).
    let shared: Vec<_> = d
        .segs
        .iter()
        .filter(|s| near(s.a[0], 0.0, TOL) && near(s.b[0], 0.0, TOL))
        .collect();
    assert_eq!(shared.len(), 2, "{shared:?}");
}

#[test]
fn cylinder_side_view_has_two_silhouettes_and_hides_the_back_rim() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = cylinder_mesh(&mut doc, 0.0, 0.0, 0.0, 0.5, 1.0, 24);
    assert!(
        !mesh.soft_edge_positions.is_empty(),
        "a kernel cylinder has soft seams"
    );
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([0.0, -10.0, 0.5], [0.0, 0.0, 0.5], [0.0, 0.0, 1.0]);
    let d = hlr::line_drawing(&items, &cam, &Options::default()).unwrap();
    // Silhouettes: the two vertical seams at x = ±0.5 (a 24-gon has vertices
    // exactly at ±0.5 on the x axis, so the seams there are where facing
    // flips), 1 m tall each.
    let sil = count(&d, Kind::Silhouette);
    assert!(
        sil >= 2,
        "silhouettes: {sil} {:?}",
        d.segs
            .iter()
            .filter(|s| s.kind == Kind::Silhouette)
            .collect::<Vec<_>>()
    );
    let sil_len = total_len(&d, Kind::Silhouette);
    assert!(near(sil_len, 2.0, 1e-3), "silhouette length {sil_len}");
    // Rims (hard): the front halves of top and bottom rims are visible, the
    // back halves hidden behind the wall — total visible rim length is half
    // of two 24-gon perimeters (each side of a 24-gon of radius 0.5).
    let side = 2.0 * 0.5 * (std::f64::consts::PI / 24.0).sin();
    // Projected rim length in the side view = the x-extent walk = 2×2r per rim
    // (front half spans -0.5..0.5 as a polyline of projected facet edges).
    let _ = side;
    let hard = total_len(&d, Kind::Hard);
    // Each rim's front half projects to a polyline of length ≈ 1.0 (diameter);
    // two rims → ≈ 2.0. Facet chords near the silhouette contribute a hair
    // less than the exact diameter.
    assert!(hard > 1.9 && hard < 2.05, "visible rim length {hard}");
    // No soft seams unless asked.
    assert_eq!(count(&d, Kind::Soft), 0);
    let ds = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            include_soft: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(count(&ds, Kind::Soft) > 0);
}

#[test]
fn section_plane_cuts_a_box_and_emits_the_cut_outline() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    // Horizontal section at z = 0.6, normal +Z: removes the top, keeps below.
    let sec = Section {
        origin: Point3::new(0.5, 0.5, 0.6),
        normal: Vec3::new(0.0, 0.0, 1.0),
    };
    let cam = parallel([0.5, 0.5, 10.0], [0.5, 0.5, 0.5], [0.0, 1.0, 0.0]);
    let d = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            section: Some(sec),
            ..Options::default()
        },
    )
    .unwrap();
    // From above: the cut outline (a unit square) is the visible ring; the
    // bottom face edges are hidden below the kept walls' interior... the box
    // is hollow, so looking down through the cut you see the bottom face
    // edges too (coincident with the cut square in projection).
    let sec_len = total_len(&d, Kind::Section);
    assert!(
        near(sec_len, 4.0, 1e-6),
        "section outline length {sec_len}: {:?}",
        d.segs
            .iter()
            .filter(|s| s.kind == Kind::Section)
            .collect::<Vec<_>>()
    );
    // Nothing above the cut survives: no segment reaches z > 0.6 — in this
    // top view that means every hard edge left is a bottom edge (visible
    // through the open cut) — and the top square is gone (it coincides in
    // projection, so check the front view instead).
    let cam_front = parallel([0.5, -10.0, 0.5], [0.5, 0.0, 0.5], [0.0, 0.0, 1.0]);
    let df = hlr::line_drawing(
        &items,
        &cam_front,
        &Options {
            section: Some(sec),
            ..Options::default()
        },
    )
    .unwrap();
    let (_, mx) = df.bounds.unwrap();
    // View y = world z - 0.5 → max 0.1.
    assert!(
        near(mx[1], 0.1, 1e-6),
        "front view top after cut: {}",
        mx[1]
    );
}

#[test]
fn perspective_cube_from_the_front_right_top_shows_nine_edges() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = perspective([4.0, -3.5, 3.0], [0.5, 0.5, 0.5], [0.0, 0.0, 1.0]);
    let d = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            include_hidden: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(count(&d, Kind::Hard), 9, "{:?}", d.segs);
    assert_eq!(count(&d, Kind::Hidden), 3);
}

#[test]
fn budget_refuses_typed() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([5.5, -4.5, 5.5], [0.5, 0.5, 0.5], [0.0, 0.0, 1.0]);
    let err = hlr::line_drawing(
        &items,
        &cam,
        &Options {
            budget: 1,
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(matches!(err, hlr::HlrError::TooComplex { .. }));
}

#[test]
fn degenerate_camera_is_refused() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.0, 0.0, 1.0]);
    assert_eq!(
        hlr::line_drawing(&items, &cam, &Options::default()).unwrap_err(),
        hlr::HlrError::DegenerateCamera
    );
}

#[test]
fn svg_is_true_size_at_the_given_scale() {
    let mut doc = kernel::Document::new();
    let (_, mesh) = box_mesh(&mut doc, 0.0, 0.0, 0.0, 0.1, 0.1, 0.1);
    let items = [Item {
        mesh: &mesh,
        pose: Transform::IDENTITY,
        sid: 1,
    }];
    let cam = parallel([0.05, 0.05, 10.0], [0.05, 0.05, 0.05], [0.0, 1.0, 0.0]);
    let d = hlr::line_drawing(&items, &cam, &Options::default()).unwrap();
    let svg = hlr::svg::write(
        &d,
        &hlr::svg::SvgStyle {
            ratio: 1.0,
            margin_mm: 0.0,
            ..Default::default()
        },
    );
    // A 100 mm square at 1:1 → 100 mm × 100 mm document.
    assert!(svg.contains("width=\"100mm\" height=\"100mm\""), "{svg}");
    assert!(svg.contains("class=\"hard\""));
    let svg2 = hlr::svg::write(
        &d,
        &hlr::svg::SvgStyle {
            ratio: 0.5,
            margin_mm: 5.0,
            ..Default::default()
        },
    );
    assert!(svg2.contains("width=\"60mm\" height=\"60mm\""), "{svg2}");
}
