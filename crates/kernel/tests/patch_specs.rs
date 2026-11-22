//! Executable specs for the shared surgery primitive (decision I-7):
//! `Object::apply_patch`, `to_patch`, `planarize`, and the
//! self-intersection guard, driven through their `…_for_test` hooks.

use crate::common;
use kernel::trig;

use std::collections::BTreeMap;

use common::curves::*;
use kernel::patch::{
    EdgeMeta, PVert, PatchFace, PolyPatch, TriSoup, improper_contact_for_test, planarize_for_test,
    plane_shortcut_census_for_test, self_intersection_for_test,
};
use kernel::{FaceId, Object, Point3, TopologyError, Transform, Vec3, WatertightState};
use proptest::prelude::*;

/// Every face as (outer ring, hole rings, plane normal bits), sorted, plus
/// the per-face attributes — an id-free fingerprint of the geometry.
fn fingerprint(o: &Object) -> Vec<String> {
    let mut v: Vec<String> = o
        .faces()
        .values()
        .map(|f| {
            let ring = |l| -> Vec<[u64; 3]> {
                let mut r: Vec<[u64; 3]> = o
                    .loop_positions(l)
                    .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                    .collect();
                // Rotate to the smallest point: ring order is kept.
                let k = (0..r.len()).min_by_key(|&i| r[i]).unwrap();
                r.rotate_left(k);
                r
            };
            let mut holes: Vec<Vec<[u64; 3]>> = f.inner_loops.iter().map(|&l| ring(l)).collect();
            holes.sort();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}",
                ring(f.outer_loop),
                holes,
                f.attrs(),
                f.plane.normal(),
                f.plane.signed_distance(Point3::new(0.0, 0.0, 0.0))
            )
        })
        .collect();
    v.sort();
    v
}

/// Every edge's metadata keyed by its sorted endpoint bits.
fn edge_census(o: &Object) -> BTreeMap<[u64; 6], String> {
    o.edges()
        .iter()
        .map(|(id, e)| {
            let (a, b) = o.edge_endpoints(id).unwrap();
            let k = |p: Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            let (ka, kb) = (k(a), k(b));
            let (lo, hi) = if ka <= kb { (ka, kb) } else { (kb, ka) };
            (
                [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]],
                format!(
                    "{:?}|{}|{}|{:?}|{:?}",
                    e.twin_half_edge.is_some(),
                    e.soft,
                    e.crease,
                    e.curve,
                    e.polygon
                ),
            )
        })
        .collect()
}

fn vertex_index(o: &Object) -> BTreeMap<kernel::VertexId, usize> {
    o.vertices()
        .keys()
        .enumerate()
        .map(|(i, v)| (v, i))
        .collect()
}

// ------------------------------------------------------ identity

#[test]
fn identity_patch_reproduces_the_object() {
    for o in [
        unit_box(),
        cylinder(0.5, 1.0, 24),
        plate_with_hole(1.0, 0.1, 0.2, 24),
        rounded_rect_prism(1.0, 0.6, 0.1, 0.3, 6),
    ] {
        let r = o.apply_patch_for_test(&o.to_patch_for_test()).unwrap();
        assert_valid_solid(&r.object);
        assert_eq!(
            fingerprint(&r.object),
            fingerprint(&o),
            "geometry and attributes"
        );
        assert_eq!(edge_census(&r.object), edge_census(&o), "edge metadata");
        assert_eq!(claim_census(&r.object), claim_census(&o));
        assert_eq!(r.object.faces().len(), o.faces().len());
        assert_eq!(r.object.analytic_rims().len(), o.analytic_rims().len());
    }
}

// ------------------------------------------------ untouched faces

/// The unit box's top face replaced by two triangles across its diagonal.
fn split_top(o: &Object, diagonal: EdgeMeta) -> (PolyPatch, FaceId) {
    let top = face_with_normal(o, Z);
    let idx = vertex_index(o);
    let ring: Vec<usize> = o
        .loop_half_edges(o.faces()[top].outer_loop)
        .map(|h| idx[&o.half_edges()[h].origin])
        .collect();
    let verts: Vec<PVert> = o.vertices().keys().map(PVert::Existing).collect();
    let attrs = o.faces()[top].attrs();
    let tri = |a: usize, b: usize, c: usize| PatchFace {
        outer: vec![ring[a], ring[b], ring[c]],
        holes: vec![],
        attrs,
        plane: None,
    };
    let mut edge_meta = BTreeMap::new();
    edge_meta.insert((ring[0].min(ring[2]), ring[0].max(ring[2])), diagonal);
    (
        PolyPatch {
            verts,
            remove: vec![top],
            add: vec![tri(0, 1, 2), tri(0, 2, 3)],
            edge_meta,
            expect_watertight: true,
        },
        top,
    )
}

#[test]
fn untouched_faces_keep_ids() {
    let o = unit_box();
    let soft = EdgeMeta {
        soft: true,
        ..EdgeMeta::default()
    };
    let (patch, top) = split_top(&o, soft);
    let r = o.apply_patch_for_test(&patch).unwrap();
    assert_valid_solid(&r.object);
    let before: Vec<FaceId> = o.faces().keys().filter(|&f| f != top).collect();
    for f in &before {
        assert!(
            r.object.faces().contains_key(*f),
            "untouched face keeps its id"
        );
        assert_eq!(fingerprint_face(&o, *f), fingerprint_face(&r.object, *f));
    }
    assert_eq!(r.object.faces().len(), 7);
    let soft_edges = r.object.edges().values().filter(|e| e.soft).count();
    assert_eq!(soft_edges, 1, "the diagonal is soft");
    assert!((signed_volume(&r.object) - 1.0).abs() <= 1e-12);
}

fn fingerprint_face(o: &Object, f: FaceId) -> Vec<Point3> {
    o.loop_positions(o.faces()[f].outer_loop).collect()
}

#[test]
fn metadata_inherited_by_vertex_pair() {
    let mut o = unit_box();
    let e = edge_between(&o, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    o.set_edge_crease_for_test(e, true);
    o.validate().unwrap();
    // Replace the top face (one of the creased edge's faces) by itself.
    let (patch, _) = split_top(&o, EdgeMeta::default());
    let r = o.apply_patch_for_test(&patch).unwrap();
    let e2 = edge_between(&r.object, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    assert!(
        r.object.edges()[e2].crease,
        "the crease survives the face's replacement"
    );
    assert_eq!(r.object.edges().values().filter(|e| e.crease).count(), 1);
}

// ------------------------------------------------------ refusals

#[test]
fn open_result_refused_when_watertight_expected() {
    let o = unit_box();
    let top = face_with_normal(&o, Z);
    let patch = PolyPatch {
        verts: vec![],
        remove: vec![top],
        add: vec![],
        edge_meta: BTreeMap::new(),
        expect_watertight: true,
    };
    assert!(matches!(
        o.apply_patch_for_test(&patch),
        Err(TopologyError::WatertightFlagMismatch { .. })
    ));
    let open = PolyPatch {
        expect_watertight: false,
        ..patch
    };
    let r = o.apply_patch_for_test(&open).unwrap();
    assert_eq!(r.object.watertight(), WatertightState::Open);
    r.object.validate().unwrap();
}

#[test]
fn non_manifold_refused() {
    // A second copy of the top face added without removing the first: every
    // top edge would be traversed twice in one direction.
    let o = unit_box();
    let (mut patch, _) = split_top(&o, EdgeMeta::default());
    patch.remove.clear();
    assert!(matches!(
        o.apply_patch_for_test(&patch),
        Err(TopologyError::NonManifoldEdge { .. })
    ));
    // Strong guarantee.
    assert_eq!(o.faces().len(), 6);
}

#[test]
fn non_planar_added_face_refused() {
    let o = unit_box();
    let top = face_with_normal(&o, Z);
    let idx = vertex_index(&o);
    let mut verts: Vec<PVert> = o.vertices().keys().map(PVert::Existing).collect();
    let ring: Vec<usize> = o
        .loop_half_edges(o.faces()[top].outer_loop)
        .map(|h| idx[&o.half_edges()[h].origin])
        .collect();
    // Lift one corner of the replacement quad off the plane by 1 mm.
    let lifted = verts.len();
    let p = o.vertices()[o.vertices().keys().nth(ring[2]).unwrap()].position;
    verts.push(PVert::New(Point3::new(p.x, p.y, p.z + 1e-3)));
    let patch = PolyPatch {
        verts,
        remove: vec![top],
        add: vec![PatchFace {
            outer: vec![ring[0], ring[1], lifted, ring[3]],
            holes: vec![],
            attrs: o.faces()[top].attrs(),
            plane: None,
        }],
        edge_meta: BTreeMap::new(),
        expect_watertight: false,
    };
    assert!(matches!(
        o.apply_patch_for_test(&patch),
        Err(TopologyError::NonPlanarFace { face: 0 })
    ));
}

#[test]
fn degenerate_and_out_of_range_input_refused() {
    let o = unit_box();
    let base = PolyPatch {
        verts: vec![PVert::New(p3(0.0, 0.0, 5.0)), PVert::New(p3(1.0, 0.0, 5.0))],
        remove: vec![],
        add: vec![PatchFace {
            outer: vec![0, 1, 7],
            holes: vec![],
            attrs: o.faces().values().next().unwrap().attrs(),
            plane: None,
        }],
        edge_meta: BTreeMap::new(),
        expect_watertight: false,
    };
    assert!(matches!(
        o.apply_patch_for_test(&base),
        Err(TopologyError::InvalidVertexIndex { face: 0, index: 7 })
    ));
    let mut two = base.clone();
    two.add[0].outer = vec![0, 1];
    assert!(matches!(
        o.apply_patch_for_test(&two),
        Err(TopologyError::DegenerateFace { face: 0 })
    ));
    let mut repeat = base;
    repeat.add[0].outer = vec![0, 1, 1];
    assert!(matches!(
        o.apply_patch_for_test(&repeat),
        Err(TopologyError::DegenerateFace { face: 0 })
    ));
}

// ------------------------------------------------- property test

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// A random face of a random extruded polygon replaced by its fan
    /// triangulation (diagonals soft): valid, watertight, same volume.
    #[test]
    fn random_face_retriangulations_stay_valid(
        n in 3usize..9,
        r in 0.2f64..2.0,
        h in 0.1f64..2.0,
        pick in 0usize..64,
    ) {
        let pts: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                (r * trig::cos(a), r * trig::sin(a))
            })
            .collect();
        let o = prism(&pts, h);
        let faces: Vec<FaceId> = o.faces().keys().collect();
        let f = faces[pick % faces.len()];
        let idx = vertex_index(&o);
        let ring: Vec<usize> = o
            .loop_half_edges(o.faces()[f].outer_loop)
            .map(|he| idx[&o.half_edges()[he].origin])
            .collect();
        let mut edge_meta = BTreeMap::new();
        let mut add = Vec::new();
        for k in 1..ring.len() - 1 {
            add.push(PatchFace {
                outer: vec![ring[0], ring[k], ring[k + 1]],
                holes: vec![],
                attrs: o.faces()[f].attrs(),
                plane: None,
            });
            if k + 1 < ring.len() - 1 {
                let (a, b) = (ring[0], ring[k + 1]);
                edge_meta.insert((a.min(b), a.max(b)), EdgeMeta { soft: true, ..EdgeMeta::default() });
            }
        }
        let patch = PolyPatch {
            verts: o.vertices().keys().map(PVert::Existing).collect(),
            remove: vec![f],
            add,
            edge_meta,
            expect_watertight: true,
        };
        let res = o.apply_patch_for_test(&patch).unwrap();
        assert_valid_solid(&res.object);
        let (v0, v1) = (signed_volume(&o), signed_volume(&res.object));
        prop_assert!((v0 - v1).abs() <= 1e-12 * v0.abs().max(1.0));
        prop_assert_eq!(res.object.edges().values().filter(|e| e.soft).count(), ring.len() - 3);
    }
}

// ------------------------------------------------------ planarize

/// The unit box as a triangle soup (two triangles per face, diagonal 0–2),
/// points in vertex slot order, original edges with their metadata.
fn box_soup(o: &Object) -> TriSoup {
    let idx = vertex_index(o);
    let points: Vec<Point3> = o.vertices().values().map(|v| v.position).collect();
    let mut tris = Vec::new();
    for (fi, (_, f)) in o.faces().iter().enumerate() {
        let ring: Vec<u32> = o
            .loop_half_edges(f.outer_loop)
            .map(|h| idx[&o.half_edges()[h].origin] as u32)
            .collect();
        tris.push(([ring[0], ring[1], ring[2]], fi as u32, f.attrs()));
        tris.push(([ring[0], ring[2], ring[3]], fi as u32, f.attrs()));
    }
    let mut original_edges = BTreeMap::new();
    for (id, _) in o.edges().iter() {
        let (a, b) = o.edge_endpoints(id).unwrap();
        let find = |p: Point3| points.iter().position(|q| *q == p).unwrap() as u32;
        let (ia, ib) = (find(a), find(b));
        original_edges.insert((ia.min(ib), ia.max(ib)), EdgeMeta::default());
    }
    TriSoup {
        points,
        tris,
        original_edges,
    }
}

#[test]
fn planarize_regroups_an_untouched_triangulated_box_into_six_quads() {
    let o = unit_box();
    let soup = box_soup(&o);
    let patch = planarize_for_test(&o, &soup);
    assert_eq!(patch.add.len(), 6);
    assert!(
        patch
            .add
            .iter()
            .all(|f| f.outer.len() == 4 && f.holes.is_empty())
    );
    let r = o.apply_patch_for_test(&patch).unwrap();
    assert_valid_solid(&r.object);
    assert!((signed_volume(&r.object) - 1.0).abs() <= 1e-12);
    assert!(r.object.edges().values().all(|e| !e.soft));
}

#[test]
fn planarize_twisted_box_side_faces_become_triangles_with_soft_diagonals() {
    let o = unit_box();
    let mut soup = box_soup(&o);
    // Twist the top four points 30° about the box's vertical axis.
    let rot = Transform::translation(Vec3::new(-0.5, -0.5, 0.0))
        .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 30f64.to_radians()).unwrap())
        .then(&Transform::translation(Vec3::new(0.5, 0.5, 0.0)));
    for p in soup.points.iter_mut() {
        if p.z > 0.5 {
            *p = rot.apply_point(*p);
        }
    }
    let patch = planarize_for_test(&o, &soup);
    let tris = patch.add.iter().filter(|f| f.outer.len() == 3).count();
    let quads = patch.add.iter().filter(|f| f.outer.len() == 4).count();
    assert_eq!(
        (tris, quads),
        (8, 2),
        "four sides become 8 triangles; top and bottom stay quads"
    );
    let r = o.apply_patch_for_test(&patch).unwrap();
    assert_valid_solid(&r.object);
    assert_eq!(
        r.object.edges().values().filter(|e| e.soft).count(),
        4,
        "the side diagonals are soft"
    );
    assert_eq!(
        r.object.edges().values().filter(|e| !e.soft).count(),
        12,
        "the box edges stay hard"
    );
}

// ------------------------------------------- self-intersection guard

#[test]
fn self_intersection_ignores_an_edge_ending_just_above_a_face_beside_it() {
    // An L-shaped slab whose top is z = 0, and over its notch a tetrahedron
    // with an edge from 0.5 nm to 1.5 nm above that plane. The edge's line
    // meets the plane back over the slab, but the edge itself never does:
    // no contact.
    let mut slab = prism(
        &[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 0.4),
            (1.0, 0.4),
            (1.0, 1.0),
            (0.0, 1.0),
        ],
        0.1,
    );
    slab.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, -0.1)))
        .unwrap();
    let (mut pts, mut faces) = slab.to_polygons();
    let tet = convex_solid(
        &[
            p3(1.2, 0.7, 5e-10),
            p3(1.8, 0.7, 1.5e-9),
            p3(1.5, 0.9, 0.3),
            p3(1.5, 0.5, 0.3),
        ],
        &[vec![0, 1, 2], vec![0, 3, 1], vec![0, 2, 3], vec![1, 3, 2]],
    );
    let (tp, tf) = tet.to_polygons();
    let off = pts.len();
    pts.extend(tp);
    faces.extend(
        tf.iter()
            .map(|f| f.iter().map(|i| i + off).collect::<Vec<_>>()),
    );
    let both = Object::from_polygons(&pts, &faces).unwrap();
    both.validate().unwrap();
    assert!(self_intersection_for_test(&both).is_none());
    // The plane shortcut passes over no pair the full test would refuse,
    // here where an edge comes within a nanometer of the plane.
    assert_eq!(plane_shortcut_census_for_test(&both).1, 0);
}

#[test]
fn self_intersection_finds_interpenetrating_shells() {
    // Two overlapping unit boxes as ONE object (two shells, no boolean): the
    // faces interpenetrate, which the structural validator cannot see.
    let a = unit_box();
    let (pa, fa) = a.to_polygons();
    let mut pts = pa.clone();
    let mut faces = fa.clone();
    let off = pts.len();
    pts.extend(
        pa.iter()
            .map(|p| Point3::new(p.x + 0.5, p.y + 0.5, p.z + 0.5)),
    );
    faces.extend(
        fa.iter()
            .map(|f| f.iter().map(|i| i + off).collect::<Vec<_>>()),
    );
    let both = Object::from_polygons(&pts, &faces).unwrap();
    both.validate().unwrap();
    assert!(self_intersection_for_test(&both).is_some());
    // The plane shortcut passes over no pair the full test refuses.
    assert_eq!(plane_shortcut_census_for_test(&both).1, 0);
    // Separate boxes, and a clean solid, report nothing.
    let mut apart = pts.clone();
    for p in apart.iter_mut().skip(off) {
        *p = Point3::new(p.x + 2.0, p.y, p.z);
    }
    let separate = Object::from_polygons(&apart, &faces).unwrap();
    assert!(self_intersection_for_test(&separate).is_none());
    assert!(self_intersection_for_test(&cylinder(0.5, 1.0, 48)).is_none());
    // A candidate list restricts the search to pairs involving a candidate.
    let first_box_top = both
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().z > 0.5 && both.loop_positions(f.outer_loop).all(|p| p.z == 1.0)
        })
        .map(|(id, _)| id)
        .unwrap();
    assert!(improper_contact_for_test(&both, &[first_box_top]).is_some());
}

#[test]
fn improper_contact_is_fast_on_a_large_object() {
    // A 20k-face extrusion (a 19,998-gon prism, built directly); 10
    // candidates.
    let n = 19_998;
    let mut pts: Vec<Point3> = Vec::with_capacity(2 * n);
    for z in [0.0, 0.5] {
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            pts.push(p3(trig::cos(a), trig::sin(a), z));
        }
    }
    let mut faces: Vec<Vec<usize>> = vec![(0..n).rev().collect(), (n..2 * n).collect()];
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, n + j, n + i]);
    }
    let o = Object::from_polygons(&pts, &faces).unwrap();
    assert_eq!(o.faces().len(), 20_000);
    // Candidates are what an operation adds: small faces (wall facets), not
    // the 20k-vertex caps whose bounds every wall overlaps.
    let cands: Vec<FaceId> = o
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().z.abs() < 0.5)
        .map(|(id, _)| id)
        .take(10)
        .collect();
    let t = std::time::Instant::now();
    assert!(improper_contact_for_test(&o, &cands).is_none());
    assert!(
        t.elapsed() < std::time::Duration::from_secs(1),
        "took {:?}",
        t.elapsed()
    );
}
