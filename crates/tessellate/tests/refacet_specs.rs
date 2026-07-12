//! Executable specs for export re-faceting (docs/design/true-curves.md
//! stage 6): a pristine stamped cylinder band re-facets from its analytic
//! definition at the requested resolution; anything less than pristine
//! falls back to stored facets; and the emitted triangle soup is manifold
//! at ANY resolution — the caps are re-triangulated with bitwise the same
//! rim points the new wall quads use.

use std::collections::BTreeMap;

use kernel::{CurveGeom, Object, Plane, Point3, Sketch};
use tessellate::export_triangles;

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

fn circle_sketch(center: Point3, radius: f64, n: usize) -> Sketch {
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom { center, radius }).unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
            0.0,
        )
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    s
}

fn extrude(s: &Sketch, h: f64) -> Object {
    let region = s.regions().keys().next().unwrap();
    let profile = s.profile(region).unwrap();
    Object::from_extrusion(&profile, h).unwrap()
}

/// A square block with an analytic circular hole, extruded — a tunnel.
fn drilled_block(hole_radius: f64, n: usize, h: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((-3.0, -3.0), (3.0, -3.0)),
        ((3.0, -3.0), (3.0, 3.0)),
        ((3.0, 3.0), (-3.0, 3.0)),
        ((-3.0, 3.0), (-3.0, -3.0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: hole_radius,
    })
    .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(hole_radius * a.cos(), hole_radius * a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let region = s
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .unwrap();
    let profile = s.profile(region).unwrap();
    Object::from_extrusion(&profile, h).unwrap()
}

/// D-shape: semicircular analytic arc closed by a straight chord.
fn d_solid(n: usize, h: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    s.add_segment(Point3::new(-1.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    extrude(&s, h)
}

// ------------------------------------------------------------ oracles

/// Manifoldness oracle over the soup: every directed edge appears exactly
/// once and its reverse exactly once. Point identity is BITWISE — the
/// re-faceting contract is shared f64 values, not tolerance welding.
fn assert_manifold(soup: &[f64]) {
    assert!(soup.len().is_multiple_of(9), "flat triangle soup");
    type Key = ([u64; 3], [u64; 3]);
    let bits = |x: f64, y: f64, z: f64| -> [u64; 3] { [x.to_bits(), y.to_bits(), z.to_bits()] };
    let mut directed: BTreeMap<Key, usize> = BTreeMap::new();
    for tri in soup.chunks_exact(9) {
        let v = [
            bits(tri[0], tri[1], tri[2]),
            bits(tri[3], tri[4], tri[5]),
            bits(tri[6], tri[7], tri[8]),
        ];
        for k in 0..3 {
            let a = v[k];
            let b = v[(k + 1) % 3];
            assert_ne!(a, b, "degenerate zero-length edge in soup");
            *directed.entry((a, b)).or_insert(0) += 1;
        }
    }
    for (&(a, b), &count) in &directed {
        assert_eq!(count, 1, "directed edge appears more than once");
        assert_eq!(
            directed.get(&(b, a)).copied().unwrap_or(0),
            1,
            "unmatched directed edge — the soup is not closed/manifold"
        );
    }
}

/// Signed volume of the (closed, outward-wound) soup.
fn soup_volume(soup: &[f64]) -> f64 {
    let mut vol = 0.0;
    for t in soup.chunks_exact(9) {
        let (ax, ay, az) = (t[0], t[1], t[2]);
        let (bx, by, bz) = (t[3], t[4], t[5]);
        let (cx, cy, cz) = (t[6], t[7], t[8]);
        vol += ax * (by * cz - bz * cy) + ay * (bz * cx - bx * cz) + az * (bx * cy - by * cx);
    }
    vol / 6.0
}

/// Volume of an inscribed n-gon prism of radius r, height h.
fn prism_volume(n: usize, r: f64, h: f64) -> f64 {
    0.5 * (n as f64) * r * r * (2.0 * std::f64::consts::PI / (n as f64)).sin() * h
}

fn contains_point(soup: &[f64], p: Point3) -> bool {
    soup.chunks_exact(3).any(|c| {
        (c[0] - p.x).abs() == 0.0 && (c[1] - p.y).abs() == 0.0 && (c[2] - p.z).abs() == 0.0
    })
}

// ------------------------------------------------------------ specs

#[test]
fn cylinder_refacets_to_the_requested_resolution() {
    let obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 24), 1.0);

    let stored = export_triangles(&obj, 0).unwrap();
    assert_manifold(&stored);
    let v24 = prism_volume(24, 0.5, 1.0);
    assert!((soup_volume(&stored) - v24).abs() < 1e-12);

    let refined = export_triangles(&obj, 96).unwrap();
    assert_manifold(&refined);
    // 96 wall quads (192 triangles) + two 96-gon caps (94 triangles each).
    assert_eq!(refined.len() / 9, 192 + 2 * 94);
    let v96 = prism_volume(96, 0.5, 1.0);
    assert!(
        (soup_volume(&refined) - v96).abs() < 1e-12,
        "volume is exactly the inscribed 96-gon prism"
    );
    assert!(soup_volume(&refined) > soup_volume(&stored));
}

#[test]
fn downsampling_is_a_legitimate_resolution_choice() {
    let obj = extrude(&circle_sketch(Point3::new(1.0, -2.0, 0.0), 0.5, 48), 0.7);
    let coarse = export_triangles(&obj, 12).unwrap();
    assert_manifold(&coarse);
    assert!((soup_volume(&coarse) - prism_volume(12, 0.5, 0.7)).abs() < 1e-12);
}

#[test]
fn d_profile_arc_band_refacets_and_preserves_its_seams() {
    let obj = d_solid(12, 0.5);
    let soup = export_triangles(&obj, 200).unwrap();
    assert_manifold(&soup);
    // The arc endpoints are anchors: preserved bit-exact, so the chord wall
    // still meets the refined band at the original seam vertices. Read the
    // chord wall's actual corners from the object (sin(pi) is 1.2e-16, not
    // 0.0 — bit-exactness is against the stored geometry, not the ideal).
    let chord_wall = obj
        .faces()
        .values()
        .find(|f| f.surface.is_none() && f.plane.normal().y < -0.5)
        .expect("chord wall");
    for p in obj.loop_positions(chord_wall.outer_loop) {
        assert!(contains_point(&soup, p), "anchor {p:?} preserved");
    }
    // Refinement really happened: more material than the 12-gon arc.
    let stored = export_triangles(&obj, 0).unwrap();
    assert!(soup_volume(&soup) > soup_volume(&stored));
}

#[test]
fn tunnel_rim_refacets_the_hole() {
    let obj = drilled_block(1.0, 16, 1.0);
    let stored = export_triangles(&obj, 0).unwrap();
    assert_manifold(&stored);
    let refined = export_triangles(&obj, 128).unwrap();
    assert_manifold(&refined);
    // A refined hole removes MORE material (the true circle contains the
    // inscribed 16-gon), so the solid's volume strictly decreases.
    assert!(soup_volume(&refined) < soup_volume(&stored));
    let block = 6.0 * 6.0 * 1.0;
    let expected = block - prism_volume(128, 1.0, 1.0);
    assert!((soup_volume(&refined) - expected).abs() < 1e-9);
}

#[test]
fn boolean_cut_band_falls_back_to_stored_facets() {
    // A planar cut through the wall makes fragments that are no longer
    // pristine chord quads: the whole group exports at stored facets, and
    // the output is byte-identical to the resolution-0 path.
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let cyl = extrude(&s, 1.0);
    let cutter = {
        let mut cs = Sketch::on_plane(ground());
        for (a, b) in [
            ((0.55, -2.0), (2.0, -2.0)),
            ((2.0, -2.0), (2.0, 2.0)),
            ((2.0, 2.0), (0.55, 2.0)),
            ((0.55, 2.0), (0.55, -2.0)),
        ] {
            cs.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        extrude(&cs, 1.0)
    };
    let obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let refined = export_triangles(&obj, 96).unwrap();
    let stored = export_triangles(&obj, 0).unwrap();
    assert_manifold(&refined);
    assert_eq!(refined, stored, "honest fallback: stored facets verbatim");
}

#[test]
fn cap_imprint_survives_the_rim_refinement() {
    // An imprinted ring strictly inside the cap does not touch the rim: the
    // band still refines and the imprint's own loops are preserved.
    let mut obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24), 1.0);
    let top_cap = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_none() && f.plane.normal().z > 0.5)
        .map(|(fid, _)| fid)
        .unwrap();
    let d = 0.4;
    obj.split_face_inner(
        top_cap,
        &[
            Point3::new(-d, -d, 1.0),
            Point3::new(d, -d, 1.0),
            Point3::new(d, d, 1.0),
            Point3::new(-d, d, 1.0),
        ],
    )
    .unwrap();

    let soup = export_triangles(&obj, 96).unwrap();
    assert_manifold(&soup);
    let stored = export_triangles(&obj, 0).unwrap();
    assert!(soup_volume(&soup) > soup_volume(&stored), "band refined");
    assert!(
        contains_point(&soup, Point3::new(d, d, 1.0)),
        "imprint kept"
    );
}

#[test]
fn rim_vertex_with_a_drawn_edge_becomes_an_anchor() {
    // Split the top cap along a diameter between two opposite rim vertices:
    // those vertices now carry non-band edges, so refinement must preserve
    // them bit-exact while resampling between them — and stay manifold.
    let mut obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24), 1.0);
    let top_cap = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_none() && f.plane.normal().z > 0.5)
        .map(|(fid, _)| fid)
        .unwrap();
    // The path endpoints snap to the two opposite rim vertices; read their
    // exact stored positions (drawn trigonometry never lands on round
    // coordinates).
    let rim_at = |target: Point3| -> Point3 {
        obj.vertices()
            .values()
            .map(|v| v.position)
            .find(|p| p.approx_eq(target, 1e-9))
            .expect("rim vertex near target")
    };
    let a = rim_at(Point3::new(1.0, 0.0, 1.0));
    let b = rim_at(Point3::new(-1.0, 0.0, 1.0));
    obj.split_face(top_cap, &[a, b]).unwrap();

    let soup = export_triangles(&obj, 90).unwrap();
    assert_manifold(&soup);
    assert!(contains_point(&soup, a), "anchor preserved");
    assert!(contains_point(&soup, b), "anchor preserved");
    let stored = export_triangles(&obj, 0).unwrap();
    assert!(soup_volume(&soup) > soup_volume(&stored), "still refined");
}

#[test]
fn refinement_that_would_cross_cap_geometry_demotes_honestly() {
    // A coarse (8-gon) hole leaves a wide sagitta zone between its chords
    // and the true circle. An imprint placed straddling the true circle in
    // that zone is clear of the stored facets but would be crossed by the
    // refined rim: the band demotes and the export equals stored facets.
    let mut obj = drilled_block(1.0, 8, 1.0);
    let top_cap = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_none() && f.plane.normal().z > 0.5 && !f.inner_loops.is_empty())
        .map(|(fid, _)| fid)
        .unwrap();
    obj.split_face_inner(
        top_cap,
        &[
            Point3::new(0.93, 0.36, 1.0),
            Point3::new(1.05, 0.36, 1.0),
            Point3::new(1.05, 0.42, 1.0),
            Point3::new(0.93, 0.42, 1.0),
        ],
    )
    .unwrap();

    let refined = export_triangles(&obj, 64).unwrap();
    let stored = export_triangles(&obj, 0).unwrap();
    assert_manifold(&refined);
    assert_eq!(
        refined, stored,
        "collision with cap geometry demotes to stored facets"
    );
}

#[test]
fn export_is_deterministic() {
    let obj = d_solid(12, 0.5);
    let a = export_triangles(&obj, 77).unwrap();
    let b = export_triangles(&obj, 77).unwrap();
    assert_eq!(a, b);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

    /// The headline invariant: the exported soup is manifold at ANY
    /// resolution, for full circles and partial arcs alike.
    #[test]
    fn prop_soup_is_manifold_at_any_resolution(
        n_draw in 8usize..40,
        n_export in 0u32..300,
        shape in 0u8..3,
        r in 0.2f64..3.0,
        h in 0.2f64..2.0,
    ) {
        let obj = match shape {
            0 => extrude(&circle_sketch(Point3::new(0.5, -0.25, 0.0), r, n_draw), h),
            1 => {
                // D-shape at radius r.
                let mut s = Sketch::on_plane(ground());
                s.begin_curve_with(CurveGeom {
                    center: Point3::new(0.0, 0.0, 0.0),
                    radius: r,
                })
                .unwrap();
                let p = |i: usize| {
                    let a = std::f64::consts::PI * (i as f64) / (n_draw as f64);
                    Point3::new(r * a.cos(), r * a.sin(), 0.0)
                };
                for i in 0..n_draw {
                    s.add_segment(p(i), p(i + 1)).unwrap();
                }
                s.end_curve();
                s.add_segment(Point3::new(-r, 0.0, 0.0), Point3::new(r, 0.0, 0.0))
                    .unwrap();
                extrude(&s, h)
            }
            _ => drilled_block(r, n_draw, h),
        };
        let soup = export_triangles(&obj, n_export).unwrap();
        assert_manifold(&soup);
        proptest::prop_assert!(soup_volume(&soup) > 0.0);
    }
}

// ------------------------------------------------- smooth shading (stage 4)
// These live here rather than lib.rs's unit tests because they need the
// same analytic-solid builders.

use kernel::MaterialPalette;
use tessellate::tessellate;

#[test]
fn stamped_walls_shade_with_true_cylinder_normals() {
    let center = Point3::new(1.0, 2.0, 0.0);
    let obj = extrude(&circle_sketch(center, 0.5, 24), 1.0);
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();

    // Every wall vertex normal is the unit radial direction at that vertex
    // (horizontal, pointing away from the axis); cap vertices keep their
    // flat +-Z normals. Distinguish by the normal's z component.
    let mut wall_vertices = 0;
    for (p, n) in mesh
        .positions
        .chunks_exact(3)
        .zip(mesh.normals.chunks_exact(3))
    {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-5, "unit normal");
        if n[2].abs() < 0.5 {
            wall_vertices += 1;
            assert_eq!(n[2], 0.0, "cylinder normal is horizontal");
            let rx = p[0] as f64 - center.x;
            let ry = p[1] as f64 - center.y;
            let rlen = (rx * rx + ry * ry).sqrt();
            let dot = (n[0] as f64 * rx + n[1] as f64 * ry) / rlen;
            assert!(
                (dot - 1.0).abs() < 1e-6,
                "outer wall normal is the outward radial direction (dot {dot})"
            );
        }
    }
    assert!(
        wall_vertices >= 24 * 4,
        "every wall corner was smooth-shaded"
    );

    // Adjacent facets share bitwise-identical normals at their shared seam
    // positions — that is what makes the shading seamless.
    use std::collections::BTreeMap;
    let mut normal_at: BTreeMap<[u32; 3], Vec<[u32; 3]>> = BTreeMap::new();
    for (p, n) in mesh
        .positions
        .chunks_exact(3)
        .zip(mesh.normals.chunks_exact(3))
    {
        if n[2].abs() < 0.5 {
            normal_at
                .entry([p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
                .or_default()
                .push([n[0].to_bits(), n[1].to_bits(), n[2].to_bits()]);
        }
    }
    for (_, normals) in normal_at {
        for w in normals.windows(2) {
            assert_eq!(w[0], w[1], "seam vertices shade identically");
        }
    }
}

#[test]
fn hole_walls_shade_inward() {
    let obj = drilled_block(1.0, 16, 1.0);
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    for (p, n) in mesh
        .positions
        .chunks_exact(3)
        .zip(mesh.normals.chunks_exact(3))
    {
        // Tunnel wall vertices: on the r=1 cylinder about the origin.
        let r = ((p[0] * p[0] + p[1] * p[1]) as f64).sqrt();
        if n[2].abs() < 0.5 && (r - 1.0).abs() < 1e-6 && p[0].abs() < 2.9 {
            let dot = n[0] as f64 * p[0] as f64 / r + n[1] as f64 * p[1] as f64 / r;
            assert!(dot < -0.99, "hole wall normal points toward the axis");
        }
    }
}

#[test]
fn wall_seams_are_soft_and_rims_stay_hard() {
    let obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 24), 1.0);
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    // 24 vertical seams soft; 48 rim edges (top + bottom) hard.
    assert_eq!(mesh.soft_edge_positions.len(), 24 * 6);
    assert_eq!(mesh.edge_positions.len(), 48 * 6);
    // Soft seams are the vertical ones.
    for seg in mesh.soft_edge_positions.chunks_exact(6) {
        assert!(
            (seg[0], seg[1]) == (seg[3], seg[4]),
            "soft seam is vertical"
        );
        assert_ne!(seg[2], seg[5]);
    }
}

/// Serializes render buffers for the byte-golden comparison below: tagged
/// sections, `u64` little-endian element count, then each element's exact
/// bit pattern little-endian. MUST stay textually identical to the
/// generator that produced the fixture (see the test comment).
#[allow(clippy::too_many_arguments)]
fn mesh_golden_bytes(
    positions: &[f32],
    normals: &[f32],
    indices: &[u32],
    colors: &[f32],
    uvs: &[f32],
    edge_positions: &[f32],
    group_starts: &[u32],
    group_counts: &[u32],
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut f32s = |tag: u8, data: &[f32]| {
        out.push(tag);
        out.extend((data.len() as u64).to_le_bytes());
        for v in data {
            out.extend(v.to_bits().to_le_bytes());
        }
    };
    f32s(b'P', positions);
    f32s(b'N', normals);
    f32s(b'C', colors);
    f32s(b'U', uvs);
    f32s(b'E', edge_positions);
    let mut u32s = |tag: u8, data: &[u32]| {
        out.push(tag);
        out.extend((data.len() as u64).to_le_bytes());
        for v in data {
            out.extend(v.to_le_bytes());
        }
    };
    u32s(b'I', indices);
    u32s(b'S', group_starts);
    u32s(b'K', group_counts);
    out
}

#[test]
fn unattributed_objects_are_unchanged_by_the_edge_split() {
    // A plain box: no claims anywhere -> the soft buffer is empty and every
    // edge is where it always was (flat shading intact).
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((0.0, 0.0), (1.0, 0.0)),
        ((1.0, 0.0), (1.0, 1.0)),
        ((1.0, 1.0), (0.0, 1.0)),
        ((0.0, 1.0), (0.0, 0.0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let obj = extrude(&s, 1.0);
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    assert!(mesh.soft_edge_positions.is_empty());
    assert_eq!(mesh.edge_positions.len(), 12 * 6);
    // Flat normals only: every face's corners share the face normal.
    for n in mesh.normals.chunks_exact(3) {
        assert!(n.iter().filter(|c| c.abs() > 0.5).count() == 1);
    }

    // Byte-for-byte: this box's buffers are the EXACT bytes the
    // pre-smooth-shading tessellator emitted. The fixture was generated at
    // commit 0d213cd (the last commit before smooth shading landed) by
    // building this same box and dumping every buffer through a serializer
    // textually identical to `mesh_golden_bytes`. The box uses only exact
    // dyadic coordinates and axis-aligned planes, so the bytes are
    // machine-stable. If a deliberate tessellation change ever breaks this,
    // regenerate the fixture with a dump of the new output and account for
    // the change where the appendix of docs/design/true-curves.md records
    // this spec.
    assert_eq!(mesh.groups.len(), 1);
    assert_eq!(mesh.groups[0].material, None);
    let starts: Vec<u32> = mesh.groups.iter().map(|g| g.start).collect();
    let counts: Vec<u32> = mesh.groups.iter().map(|g| g.count).collect();
    let bytes = mesh_golden_bytes(
        &mesh.positions,
        &mesh.normals,
        &mesh.indices,
        &mesh.colors,
        &mesh.uvs,
        &mesh.edge_positions,
        &starts,
        &counts,
    );
    let golden = include_bytes!("golden/unattributed_box_mesh.golden");
    // Compare PER SECTION so a failure names the diverging buffer and the
    // exact element, instead of dumping two unlocalized kilobyte arrays.
    // The format is self-describing (tag byte + u64 element count + 4-byte
    // elements), so both streams parse with the same walk.
    let split_sections = |bytes: &[u8]| -> Vec<(char, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            // A short stream (a truncated golden fixture) must fail with a
            // message, not a raw slice-index panic that never reaches the
            // per-section diagnostics below.
            assert!(
                i + 9 <= bytes.len(),
                "stream truncated inside a section header at byte {i} \
                 (of {}): regenerate the golden fixture",
                bytes.len()
            );
            let tag = bytes[i] as char;
            let len = u64::from_le_bytes(bytes[i + 1..i + 9].try_into().unwrap()) as usize;
            let start = i + 9;
            let end = start + len * 4;
            assert!(
                end <= bytes.len(),
                "stream truncated inside section '{tag}': header claims \
                 {len} elements (bytes {start}..{end}) but the stream ends \
                 at {}: regenerate the golden fixture",
                bytes.len()
            );
            out.push((tag, bytes[start..end].to_vec()));
            i = end;
        }
        out
    };
    let section_name = |t: char| match t {
        'P' => "positions",
        'N' => "normals",
        'C' => "colors",
        'U' => "uvs",
        'E' => "edge_positions",
        'I' => "indices",
        'S' => "group_starts",
        'K' => "group_counts",
        _ => "unknown",
    };
    let got = split_sections(&bytes);
    let want = split_sections(golden);
    assert_eq!(got.len(), want.len(), "section count differs from golden");
    for ((gt, gd), (wt, wd)) in got.iter().zip(&want) {
        assert_eq!(gt, wt, "section order differs from golden");
        if gd != wd {
            let n = gd.len().min(wd.len());
            let byte = (0..n).find(|&k| gd[k] != wd[k]).unwrap_or(n);
            let elem = byte / 4;
            let word = |d: &[u8]| {
                d.get(elem * 4..elem * 4 + 4)
                    .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
            };
            panic!(
                "section '{gt}' ({}) diverges from the pre-smooth-shading \
                 golden at element {elem}: got {:?} (as f32: {:?}), want \
                 {:?} (as f32: {:?}); section lengths {} vs {} bytes",
                section_name(*gt),
                word(gd),
                word(gd).map(f32::from_bits),
                word(wd),
                word(wd).map(f32::from_bits),
                gd.len(),
                wd.len(),
            );
        }
    }
    // Backstop: the streams as a whole (headers included) match exactly.
    assert_eq!(
        bytes.as_slice(),
        &golden[..],
        "a plain box's tessellation must be byte-identical to the \
         pre-smooth-shading output"
    );
}

#[test]
fn seam_against_an_unattributed_coplanar_wall_stays_hard() {
    // The seam-dissolve-gate profile: a plain edge collinear with a
    // one-facet arc chain. The shared vertical seam separates an attributed
    // wall from an unattributed one -> HARD, never suppressed.
    let mut s = Sketch::on_plane(ground());
    s.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    let radius = (0.25f64 + 25.0).sqrt();
    s.begin_curve_with(CurveGeom {
        center: Point3::new(1.5, 5.0, 0.0),
        radius,
    })
    .unwrap();
    s.add_segment(Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    s.end_curve();
    for (a, b) in [
        ((2.0, 0.0), (2.0, 1.0)),
        ((2.0, 1.0), (0.0, 1.0)),
        ((0.0, 1.0), (0.0, 0.0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let obj = extrude(&s, 1.0);
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    assert!(
        mesh.soft_edge_positions.is_empty(),
        "a single-facet band has no interior seams; the boundary with the \
         plain wall must not soften"
    );
}

#[test]
fn d_profile_interior_seams_soften_but_side_seams_stay_hard() {
    let obj = d_solid(12, 0.5);
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    // 11 interior vertical seams of the 12-facet arc band; the two seams
    // against the chord wall stay hard.
    assert_eq!(mesh.soft_edge_positions.len(), 11 * 6);
}
