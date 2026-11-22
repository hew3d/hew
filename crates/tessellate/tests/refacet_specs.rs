//! Executable specs for export re-faceting (the true-curves design
//! stage 6): a pristine stamped cylinder band re-facets from its analytic
//! definition at the requested resolution; anything less than pristine
//! falls back to stored facets; and the emitted triangle soup is manifold
//! at ANY resolution — the caps are re-triangulated with bitwise the same
//! rim points the new wall quads use.

use kernel::trig;
use std::collections::BTreeMap;

use kernel::{CurveGeom, Document, KernelOp, Object, ObjectId, Plane, Point3, Sketch};
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
            center.x + radius * trig::cos(a),
            center.y + radius * trig::sin(a),
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
        Point3::new(hole_radius * trig::cos(a), hole_radius * trig::sin(a), 0.0)
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
        Point3::new(trig::cos(a), trig::sin(a), 0.0)
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
            "unmatched directed edge {:?} -> {:?} — the soup is not closed/manifold",
            a.map(f64::from_bits),
            b.map(f64::from_bits)
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
    0.5 * (n as f64) * r * r * trig::sin(2.0 * std::f64::consts::PI / (n as f64)) * h
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
                    Point3::new(r * trig::cos(a), r * trig::sin(a), 0.0)
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
    // the change where the appendix of the true-curves design records
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

// --------------------------------------------------- follow me sweeps

/// A circle profile swept along a straight path drawn in several collinear
/// strokes (the follow-me design section 4): one true cylinder built as
/// stacked wall rows sharing interior rims.
fn swept_tube(radius: f64, n: usize, path: &[Point3]) -> Object {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), radius, n);
    let region = s.regions().keys().next().unwrap();
    let profile = s.profile(region).unwrap();
    Object::from_follow_me(&profile, path, false, &[]).unwrap()
}

#[test]
fn collinear_sweep_rows_refacet_as_one_band() {
    // Two collinear path segments -> two stacked wall rows on ONE stamped
    // cylinder. Export re-faceting must engage across the stack exactly as
    // it does for the equivalent single extrusion: the interior rim (fully
    // interior to the wall) vanishes, and the refined solid IS the
    // inscribed 96-gon prism.
    let obj = swept_tube(
        0.5,
        24,
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 0.0, 0.4),
            Point3::new(0.0, 0.0, 1.0),
        ],
    );

    let stored = export_triangles(&obj, 0).unwrap();
    assert_manifold(&stored);
    // 2 rows x 24 wall quads (96 triangles) + two 24-gon caps (22 each).
    assert_eq!(stored.len() / 9, 96 + 2 * 22);
    assert!((soup_volume(&stored) - prism_volume(24, 0.5, 1.0)).abs() < 1e-12);

    let refined = export_triangles(&obj, 96).unwrap();
    assert_manifold(&refined);
    // One merged band: 96 wall quads (192 triangles) + two 96-gon caps —
    // identical shape to the single-extrusion cylinder at this resolution.
    assert_eq!(refined.len() / 9, 192 + 2 * 94);
    assert!(
        (soup_volume(&refined) - prism_volume(96, 0.5, 1.0)).abs() < 1e-12,
        "the refined sweep is exactly the inscribed 96-gon prism"
    );
}

#[test]
fn bent_sweep_walls_deliberately_export_at_stored_facets() {
    // Around a path turn the miter rims are elliptical sections — outside
    // the band machinery's analytic-cap legitimacy. The whole wall demotes
    // to stored facets DELIBERATELY (the follow-me design section 4);
    // this pin makes any accidental future engagement (or a crack) loud.
    let mut s = kernel::Sketch::on_plane(
        Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
        ])
        .unwrap(),
    );
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 0.3,
    })
    .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * ((i % 24) as f64) / 24.0;
        Point3::new(0.0, 0.3 * trig::cos(a), 0.3 * trig::sin(a))
    };
    for i in 0..24 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let region = s.regions().keys().next().unwrap();
    let profile = s.profile(region).unwrap();
    let obj = Object::from_follow_me(
        &profile,
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
        ],
        false,
        &[],
    )
    .unwrap();

    let stored = export_triangles(&obj, 0).unwrap();
    let refined = export_triangles(&obj, 96).unwrap();
    assert_manifold(&refined);
    assert_eq!(
        stored, refined,
        "bent-sweep walls export at drawn resolution — demotion is deliberate"
    );
}

#[test]
fn open_arc_sweep_rows_demote_rather_than_orphan_their_seam() {
    // A D-profile (open arc + chord) swept along two collinear segments:
    // the stacked arc rows share interior rim vertices that the flat chord
    // wall ALSO references. Merging would drop those vertices from the
    // export while the neighbor still uses them (a T-junction), so the
    // group must demote to stored facets instead.
    let mut s = kernel::Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / 12.0;
        Point3::new(trig::cos(a), trig::sin(a), 0.0)
    };
    for i in 0..12 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    s.add_segment(Point3::new(-1.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    let region = s.regions().keys().next().unwrap();
    let profile = s.profile(region).unwrap();
    let obj = Object::from_follow_me(
        &profile,
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 0.0, 0.5),
            Point3::new(0.0, 0.0, 1.0),
        ],
        false,
        &[],
    )
    .unwrap();

    let stored = export_triangles(&obj, 0).unwrap();
    let refined = export_triangles(&obj, 200).unwrap();
    assert_manifold(&refined);
    assert_eq!(
        stored, refined,
        "stacked open-arc rows demote; nothing may orphan the shared seam"
    );
}

// ------------------------------------------- tangent and touching circles

/// A 4 m square box, 1 m tall, with each `(cx, cy, phase)` circle of radius
/// 0.5 drawn on its top (24 segments, vertex 0 at `phase`) and the first
/// circle pushed by `distance`: up for a boss, down for a pocket.
fn box_with_circles(circles: &[(f64, f64, f64)], distance: f64) -> (Document, ObjectId) {
    use std::f64::consts::TAU;
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let c = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
    {
        let sk = doc.sketch_mut(s).unwrap();
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    let (id, _) = doc.extrude_region(s, r, 1.0).unwrap();
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .unwrap();
    let (radius, n) = (0.5, 24);
    for &(cx, cy, phase) in circles {
        let pts: Vec<Point3> = (0..n)
            .map(|i| {
                let a = phase + TAU * i as f64 / n as f64;
                Point3::new(cx + radius * trig::cos(a), cy + radius * trig::sin(a), 1.0)
            })
            .collect();
        doc.draft(top, None, &[id], "Circle", |sk| {
            sk.begin_curve_with(CurveGeom {
                center: Point3::new(cx, cy, 1.0),
                radius,
            })?;
            for i in 0..n {
                sk.add_segment(pts[i], pts[(i + 1) % n])?;
            }
            sk.end_curve();
            Ok(())
        })
        .expect("the circle draws");
    }
    if let Some(&(cx, cy, _)) = circles.first() {
        let obj = doc.object(id).unwrap();
        let disc = obj
            .faces()
            .iter()
            .find(|(_, f)| {
                f.plane.normal().z > 0.999
                    && f.inner_loops.is_empty()
                    && obj.loop_positions(f.outer_loop).count() == n
                    && obj
                        .loop_positions(f.outer_loop)
                        .all(|p| ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt() < 0.5 + 1e-9)
            })
            .map(|(k, _)| k)
            .expect("the circle's disc");
        doc.apply_object_op(
            id,
            KernelOp::PushPull {
                face: disc,
                distance,
            },
        )
        .expect("the disc pushes");
    }
    (doc, id)
}

/// A circle tangent to an edge of the top leaves the top with a hole that
/// touches its outer loop at one vertex. The cylinder band re-facets
/// whichever edge the circle touches, boss or pocket, and the export stays
/// manifold with exactly the refined volume. The inside test answers a
/// point on a ring by which edge it sits on, so the shared vertex used to
/// read outside on two of the four edges and demote the band.
#[test]
fn a_band_tangent_to_any_edge_of_its_face_refacets() {
    use std::f64::consts::PI;
    let edges = [
        ("left", 0.5, 2.0, PI),
        ("right", 3.5, 2.0, 0.0),
        ("front", 2.0, 0.5, -PI / 2.0),
        ("back", 2.0, 3.5, PI / 2.0),
    ];
    for distance in [0.5, -0.5] {
        for (edge, cx, cy, phase) in edges {
            let (doc, id) = box_with_circles(&[(cx, cy, phase)], distance);
            let obj = doc.object(id).unwrap();
            let refined = export_triangles(obj, 96).unwrap();
            assert_ne!(
                refined,
                export_triangles(obj, 0).unwrap(),
                "{edge} edge, push {distance}: the band re-facets"
            );
            assert_manifold(&refined);
            let expected = 16.0 + distance.signum() * prism_volume(96, 0.5, 0.5);
            assert!(
                (soup_volume(&refined) - expected).abs() < 1e-9,
                "{edge} edge, push {distance}: volume {} vs {expected}",
                soup_volume(&refined)
            );
        }
    }
}

/// A hand-built facet of each new analytic class shades with the class's
/// own normal at every vertex, oriented to the side the facet faces
/// (true-curves.md §3.1). The tessellator computes in f64 and stores f32,
/// so the comparison is against the f64 class normal cast to f32, exactly.
#[test]
fn sphere_and_torus_facets_shade_with_analytic_normals() {
    use kernel::{SurfaceRef, Vec3};
    let d = 15f64.to_radians();
    let sphere_pt = |lat: f64, lon: f64| {
        Point3::new(
            trig::cos(lat) * trig::cos(lon),
            trig::cos(lat) * trig::sin(lon),
            trig::sin(lat),
        )
    };
    let torus_pt = |phi: f64, psi: f64| {
        let rho = 2.0 + 0.5 * trig::cos(psi);
        Point3::new(
            rho * trig::cos(phi),
            rho * trig::sin(phi),
            0.5 * trig::sin(psi),
        )
    };
    let cases = [
        (
            [
                sphere_pt(0.0, 0.0),
                sphere_pt(0.0, d),
                sphere_pt(d, d),
                sphere_pt(d, 0.0),
            ],
            SurfaceRef::Sphere {
                center: Point3::new(0.0, 0.0, 0.0),
                radius: 1.0,
            },
        ),
        (
            [
                torus_pt(0.0, 0.0),
                torus_pt(d, 0.0),
                torus_pt(d, 2.0 * d),
                torus_pt(0.0, 2.0 * d),
            ],
            SurfaceRef::Torus {
                center: Point3::new(0.0, 0.0, 0.0),
                axis: Vec3::new(0.0, 0.0, 1.0),
                major_radius: 2.0,
                minor_radius: 0.5,
            },
        ),
    ];
    for (corners, claim) in cases {
        for ring in [corners.to_vec(), corners.iter().rev().copied().collect()] {
            let mut obj = Object::from_polygons(&ring, &[vec![0, 1, 2, 3]]).unwrap();
            let f = obj.faces().keys().next().unwrap();
            obj.set_face_surface_for_test(f, Some(claim));
            obj.validate().unwrap();
            let face_n = obj.faces()[f].plane.normal();
            let mesh = tessellate::tessellate(&obj, &kernel::MaterialPalette::default()).unwrap();
            assert_eq!(mesh.positions.len(), 12, "one quad, four vertices");
            for (p, n) in mesh
                .positions
                .chunks_exact(3)
                .zip(mesh.normals.chunks_exact(3))
            {
                let q = ring
                    .iter()
                    .copied()
                    .find(|q| [q.x as f32, q.y as f32, q.z as f32] == [p[0], p[1], p[2]])
                    .expect("mesh vertex is a facet corner");
                let mut a = claim.normal_at(q).unwrap();
                if a.dot(face_n) < 0.0 {
                    a = -a;
                }
                assert_eq!(
                    [n[0], n[1], n[2]],
                    [a.x as f32, a.y as f32, a.z as f32],
                    "{claim:?}: vertex normal is the oriented class normal"
                );
            }
        }
    }
}

/// Two circles touching inside the top, one pushed into a band and the other
/// left drawn, leave the top with two holes sharing a vertex. The band
/// re-facets beside the drawn circle on any side. (Pushing both is refused:
/// their walls would meet along a line.)
#[test]
fn a_band_touching_a_drawn_circle_refacets_on_any_side() {
    use std::f64::consts::PI;
    let pairs = [
        ("left of", [(1.5, 2.0, 0.0), (2.5, 2.0, PI)]),
        ("right of", [(2.5, 2.0, PI), (1.5, 2.0, 0.0)]),
        ("in front of", [(2.0, 1.5, PI / 2.0), (2.0, 2.5, -PI / 2.0)]),
        ("behind", [(2.0, 2.5, -PI / 2.0), (2.0, 1.5, PI / 2.0)]),
    ];
    for distance in [0.5, -0.5] {
        for (side, circles) in pairs {
            let (doc, id) = box_with_circles(&circles, distance);
            let obj = doc.object(id).unwrap();
            let refined = export_triangles(obj, 96).unwrap();
            assert_ne!(
                refined,
                export_triangles(obj, 0).unwrap(),
                "band {side} the drawn circle, push {distance}: it re-facets"
            );
            assert_manifold(&refined);
            let expected = 16.0 + distance.signum() * prism_volume(96, 0.5, 0.5);
            assert!(
                (soup_volume(&refined) - expected).abs() < 1e-9,
                "band {side} the drawn circle, push {distance}: volume {} vs {expected}",
                soup_volume(&refined)
            );
        }
    }
}

/// A 1 × 0.6 rounded rectangle (corner radius 0.1, 6 facets per arc),
/// extruded 0.3, built the way the Rectangle-with-arcs drawing does.
fn rounded_rect_prism() -> Object {
    let (hx, hy, r, n) = (0.5, 0.3, 0.1, 6usize);
    let mut s = Sketch::on_plane(ground());
    let corners = [
        (hx - r, hy - r, 0.0),
        (-hx + r, hy - r, std::f64::consts::FRAC_PI_2),
        (-hx + r, -hy + r, std::f64::consts::PI),
        (hx - r, -hy + r, 3.0 * std::f64::consts::FRAC_PI_2),
    ];
    let at =
        |cx: f64, cy: f64, a: f64| Point3::new(cx + r * trig::cos(a), cy + r * trig::sin(a), 0.0);
    for (i, &(cx, cy, a0)) in corners.iter().enumerate() {
        s.begin_curve_with(CurveGeom {
            center: Point3::new(cx, cy, 0.0),
            radius: r,
        })
        .unwrap();
        for k in 0..n {
            let (a, b) = (
                a0 + std::f64::consts::FRAC_PI_2 * k as f64 / n as f64,
                a0 + std::f64::consts::FRAC_PI_2 * (k + 1) as f64 / n as f64,
            );
            s.add_segment(at(cx, cy, a), at(cx, cy, b)).unwrap();
        }
        s.end_curve();
        let (nx, ny, na) = corners[(i + 1) % 4];
        s.add_segment(at(cx, cy, a0 + std::f64::consts::FRAC_PI_2), at(nx, ny, na))
            .unwrap();
    }
    extrude(&s, 0.3)
}

fn segments(buf: &[f32]) -> Vec<[f32; 6]> {
    buf.chunks_exact(6)
        .map(|c| [c[0], c[1], c[2], c[3], c[4], c[5]])
        .collect()
}

#[test]
fn tangent_rails_go_to_the_tangent_buffer() {
    let obj = rounded_rect_prism();
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    let tangent = segments(&mesh.tangent_edge_positions);
    // The eight wall-to-arc joints (two per corner), all vertical.
    assert_eq!(tangent.len(), 8, "{tangent:?}");
    for t in &tangent {
        assert!(
            (t[2] - t[5]).abs() > 0.2,
            "tangent joints are the vertical seams"
        );
    }
    // No hard segment sits on a tangent joint.
    let key = |s: &[f32; 6]| {
        let (a, b) = ([s[0], s[1], s[2]], [s[3], s[4], s[5]]);
        let (a, b) = if a.map(f32::to_bits) <= b.map(f32::to_bits) {
            (a, b)
        } else {
            (b, a)
        };
        [a, b]
            .concat()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    };
    let hard: Vec<Vec<u32>> = segments(&mesh.edge_positions).iter().map(key).collect();
    for t in &tangent {
        assert!(!hard.contains(&key(t)));
    }
    // The 4 × 5 arc facet seams are soft; the rims and caps are hard.
    assert_eq!(segments(&mesh.soft_edge_positions).len(), 20);
    assert_eq!(segments(&mesh.edge_positions).len(), 2 * (4 * 6 + 4));
}

#[test]
fn non_uniformly_scaled_cylinder_seams_stay_in_the_soft_buffer() {
    let mut obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 48), 1.0);
    obj.apply_transform(&kernel::Transform::scale(kernel::Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    assert!(obj.faces().values().all(|f| f.surface.is_none()));
    let mesh = tessellate(&obj, &MaterialPalette::default()).unwrap();
    assert_eq!(segments(&mesh.soft_edge_positions).len(), 48);
    assert!(mesh.tangent_edge_positions.is_empty());
    assert_eq!(segments(&mesh.edge_positions).len(), 2 * 48);
}

// ------------------------------------------------- blend bands (WP08 §7)

fn unit_box() -> Object {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let pts = [
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::from_polygons(&pts, &faces).unwrap()
}

fn round(edges: Vec<kernel::EdgeId>, r: f64, n: u32) -> kernel::BlendSpec {
    kernel::BlendSpec {
        edges,
        profile: kernel::BlendProfile::Round {
            radius: r,
            segments: n,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

#[test]
fn straight_fillet_strip_refacets() {
    let obj = unit_box();
    let e = obj
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            a.y.abs() < 1e-12 && b.y.abs() < 1e-12 && a.z > 0.5 && b.z > 0.5
        })
        .unwrap();
    let (filleted, _) = obj.blend_edges(&round(vec![e], 0.1, 6)).unwrap();
    let soup = export_triangles(&filleted, 96).unwrap();
    assert_manifold(&soup);
    // A quarter turn at 96 per turn: 24 rows across the fillet, two
    // triangles each (one row spans the edge's whole length).
    let strip_tris = soup
        .chunks(9)
        .filter(|t| {
            let ys = [t[1], t[4], t[7]];
            let zs = [t[2], t[5], t[8]];
            ys.iter().all(|&y| y > -1e-9 && y < 0.1 + 1e-9)
                && zs.iter().all(|&z| z > 0.9 - 1e-9)
                && ys
                    .iter()
                    .zip(&zs)
                    .any(|(&y, &z)| y > 1e-6 && z < 1.0 - 1e-6)
                && (t[0] - t[3]).abs().max((t[3] - t[6]).abs()) > 0.5
        })
        .count();
    assert_eq!(strip_tris, 24 * 2);
}

fn rim_edges(obj: &Object, z: f64) -> Vec<kernel::EdgeId> {
    obj.edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            (a.z - z).abs() < 1e-12 && (b.z - z).abs() < 1e-12
        })
        .collect()
}

fn torus_soup_points(soup: &[f64], z_min: f64) -> Vec<Point3> {
    soup.chunks(3)
        .map(|c| Point3::new(c[0], c[1], c[2]))
        .filter(|p| p.z > z_min)
        .collect()
}

#[test]
fn filleted_box_refacets_watertight() {
    let obj = unit_box();
    let all: Vec<kernel::EdgeId> = obj.edges().keys().collect();
    let (f, _) = obj.blend_edges(&round(all, 0.1, 6)).unwrap();
    let stored = export_triangles(&f, 0).unwrap();
    let soup = export_triangles(&f, 96).unwrap();
    assert_manifold(&soup);
    let (v_stored, v) = (soup_volume(&stored), soup_volume(&soup));
    // Inscribed refinement: no less than the stored mesh, no more than the
    // true rounded box.
    let r: f64 = 0.1;
    let pi = std::f64::consts::PI;
    let true_volume = 1.0
        - (12.0 * (1.0 - 2.0 * r) * (r * r - pi * r * r / 4.0))
        - 8.0 * (r * r * r - pi * r * r * r / 6.0);
    assert!(v >= v_stored - 1e-12, "{v} vs stored {v_stored}");
    assert!(v <= true_volume + 1e-12, "{v} vs true {true_volume}");
}

#[test]
fn cylinder_rim_torus_refacets_sharing_the_wall_stations() {
    let obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 48), 1.0);
    let (f, _) = obj
        .blend_edges(&round(rim_edges(&obj, 1.0), 0.1, 6))
        .unwrap();
    let soup = export_triangles(&f, 96).unwrap();
    assert_manifold(&soup);
    // Refined: more points around the rail circle than the 48 stored.
    let on_rail: BTreeMap<u64, ()> = torus_soup_points(&soup, 0.85)
        .iter()
        .filter(|p| (p.z - 0.9).abs() < 1e-12 && (trig::hypot(p.x, p.y) - 0.5).abs() < 1e-9)
        .map(|p| (p.x.to_bits() ^ p.y.to_bits().rotate_left(17), ()))
        .collect();
    assert!(on_rail.len() >= 96, "{} rail points", on_rail.len());
}

#[test]
fn cone_chamfer_band_refacets() {
    let obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 48), 1.0);
    let spec = kernel::BlendSpec {
        profile: kernel::BlendProfile::Chamfer {
            setback_a: 0.05,
            setback_b: 0.05,
        },
        ..round(rim_edges(&obj, 1.0), 0.1, 6)
    };
    let (f, _) = obj.blend_edges(&spec).unwrap();
    let soup = export_triangles(&f, 96).unwrap();
    assert_manifold(&soup);
    let top_ring = torus_soup_points(&soup, 0.99)
        .iter()
        .filter(|p| (p.z - 1.0).abs() < 1e-12 && (trig::hypot(p.x, p.y) - 0.45).abs() < 1e-9)
        .count();
    assert!(top_ring > 48 * 3, "{top_ring}");
}

#[test]
fn non_pristine_torus_demotes() {
    let obj = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 48), 1.0);
    let (f, _) = obj
        .blend_edges(&round(rim_edges(&obj, 1.0), 0.1, 6))
        .unwrap();
    // A notch through the torus band.
    let mut cutter = unit_box();
    cutter
        .apply_transform(&kernel::Transform::scale(kernel::Vec3::new(0.1, 0.2, 0.2)))
        .unwrap();
    cutter
        .apply_transform(&kernel::Transform::translation(kernel::Vec3::new(
            -0.05, 0.4, 0.88,
        )))
        .unwrap();
    let notched = Object::boolean(
        kernel::BooleanOp::Subtract,
        &f,
        &cutter,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let soup = export_triangles(&notched, 96).unwrap();
    assert_manifold(&soup);
}

#[test]
fn creases_inside_one_fillet_draw_as_hard_edges() {
    // A box with only its top rim rounded: at each corner the two strips
    // meet over the still-sharp vertical edge. That is a real crease, so
    // it draws as a hard line, not as one of the structure lines (the
    // tangent edges).
    let mut s = Sketch::on_plane(ground());
    let c = [(0.0, 0.0), (0.3, 0.0), (0.3, 0.2), (0.0, 0.2)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let obj = extrude(&s, 0.1);
    let rim: Vec<kernel::EdgeId> = obj
        .sharp_edges()
        .into_iter()
        .filter(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            (p.z - 0.1).abs() < 1e-12 && (q.z - 0.1).abs() < 1e-12
        })
        .collect();
    assert_eq!(rim.len(), 4);
    let (rounded, _) = obj
        .blend_edges(&kernel::BlendSpec {
            edges: rim,
            profile: kernel::BlendProfile::Round {
                radius: 0.01,
                segments: 6,
            },
            primary: None,
            corners: kernel::CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        })
        .unwrap();
    let seams: Vec<kernel::EdgeId> = rounded
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = rounded.edge_faces_for_test(e).unwrap();
            let blend = rounded.blend_of_face(a);
            blend.is_some()
                && blend == rounded.blend_of_face(b)
                && rounded.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
        })
        .collect();
    assert_eq!(seams.len(), 4 * 6, "one six-segment seam per corner");
    let mesh = tessellate(&rounded, &MaterialPalette::default()).unwrap();
    let key = |p: Point3, q: Point3| {
        let k = |p: Point3| [p.x as f32, p.y as f32, p.z as f32];
        let (a, b) = (k(p), k(q));
        if a <= b { [a, b] } else { [b, a] }
    };
    let set = |buf: &[f32]| -> Vec<[[f32; 3]; 2]> {
        segments(buf)
            .into_iter()
            .map(|s| {
                let (a, b) = ([s[0], s[1], s[2]], [s[3], s[4], s[5]]);
                if a <= b { [a, b] } else { [b, a] }
            })
            .collect()
    };
    let hard = set(&mesh.edge_positions);
    let tangent = set(&mesh.tangent_edge_positions);
    for e in seams {
        let (p, q) = rounded.edge_endpoints(e).unwrap();
        assert!(
            hard.contains(&key(p, q)),
            "an in-fillet crease is a hard line"
        );
        assert!(!tangent.contains(&key(p, q)), "it is not a structure line");
    }
}
