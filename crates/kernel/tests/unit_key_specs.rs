//! Executable specs for sketch unit keys (`kernel::unit_key`, unified-drafting
//! design §4.5): geometry-derived 64-bit keys that survive handle churn,
//! save/load, and unrelated edits, and change only when the geometry they
//! hash changes.

use kernel::trig;
use std::collections::{BTreeMap, BTreeSet};

use kernel::unit_key::{curve_key, edge_key};
use kernel::{CurveGeom, Document, Plane, Point3, Sketch, SketchIslandId, Transform, Vec3};

fn pt(x: f64, y: f64) -> Point3 {
    Point3::new(x, y, 0.0)
}

fn ground() -> Plane {
    Plane::from_point_normal(pt(0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

fn tilted() -> Plane {
    let n = Vec3::new(0.3, 0.4, 0.866_025_403_784_438_6)
        .normalized()
        .unwrap();
    Plane::from_point_normal(Point3::new(0.1, -0.2, 0.3), n).unwrap()
}

fn draw_rect(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) {
    let c = [pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)];
    for k in 0..4 {
        s.add_segment(c[k], c[(k + 1) % 4]).unwrap();
    }
}

fn vals<K>(m: &BTreeMap<K, u64>) -> BTreeSet<u64> {
    m.values().copied().collect()
}

fn key_set(s: &Sketch) -> BTreeSet<u64> {
    s.unit_keys().islands.values().copied().collect()
}

fn island_at(s: &Sketch, x: f64, y: f64) -> SketchIslandId {
    // The island holding a vertex at (x, y).
    let v = s
        .vertices()
        .iter()
        .find(|(_, v)| v.position.approx_eq(pt(x, y), 1e-9))
        .map(|(id, _)| id)
        .expect("vertex exists");
    let e = s
        .edges()
        .iter()
        .find(|(_, e)| e.from == v || e.to == v)
        .map(|(id, _)| id)
        .unwrap();
    s.island_of_edge(e).unwrap()
}

#[test]
fn edge_key_is_independent_of_direction() {
    for plane in [ground(), tilted()] {
        let a = Point3::new(0.25, 1.5, 0.0);
        let b = Point3::new(-3.0, 0.75, 0.0);
        assert_eq!(edge_key(&plane, a, b), edge_key(&plane, b, a));
        assert_ne!(edge_key(&plane, a, b), edge_key(&plane, a, a));
    }
}

#[test]
fn keys_survive_an_unrelated_edit() {
    let mut s = Sketch::on_plane(ground());
    draw_rect(&mut s, 0.0, 0.0, 1.0, 1.0);
    let before = s.unit_keys();
    let island = *s.islands().keys().collect::<Vec<_>>().first().unwrap();
    let key = before.islands[&island];
    let region = *s.regions().keys().collect::<Vec<_>>().first().unwrap();
    let rkey = before.regions[&region];

    draw_rect(&mut s, 5.0, 5.0, 7.0, 6.0);
    let after = s.unit_keys();
    assert_eq!(after.islands[&island], key);
    assert_eq!(after.regions[&region], rkey);
    for (e, k) in &before.edges {
        assert_eq!(after.edges[e], *k, "an existing edge's key is stable");
    }
    assert_eq!(after.islands.len(), 2);
}

#[test]
fn keys_survive_save_and_load() {
    let mut doc = Document::new();
    let sid = doc.add_sketch(ground());
    doc.begin_sketch_gesture(sid).unwrap();
    {
        let s = doc.sketch_mut(sid).unwrap();
        draw_rect(s, 0.0, 0.0, 1.0, 1.0);
        draw_rect(s, 3.0, 0.0, 4.0, 2.0);
        let c = pt(0.5, 5.0);
        s.begin_curve_with(CurveGeom {
            center: c,
            radius: 0.5,
        })
        .unwrap();
        for i in 0..24 {
            let a = |k: usize| std::f64::consts::TAU * (k as f64) / 24.0;
            s.add_segment(
                pt(c.x + 0.5 * trig::cos(a(i)), c.y + 0.5 * trig::sin(a(i))),
                pt(
                    c.x + 0.5 * trig::cos(a(i + 1)),
                    c.y + 0.5 * trig::sin(a(i + 1)),
                ),
            )
            .unwrap();
        }
        s.end_curve();
    }
    doc.end_sketch_gesture(sid).unwrap();
    let keys = doc.sketch(sid).unwrap().unit_keys();
    assert_eq!(keys.curves.len(), 1);

    let loaded = Document::load(&doc.save()).expect("loads");
    let lid = loaded.sketch_ids()[0];
    let lkeys = loaded.sketch(lid).unwrap().unit_keys();
    assert_eq!(vals(&lkeys.edges), vals(&keys.edges));
    assert_eq!(vals(&lkeys.islands), vals(&keys.islands));
    assert_eq!(vals(&lkeys.regions), vals(&keys.regions));
    assert_eq!(vals(&lkeys.curves), vals(&keys.curves));
}

#[test]
fn island_key_survives_a_recompute_that_changes_its_handle() {
    // Direct: one rectangle, island id in a fresh slot.
    let mut direct = Sketch::on_plane(ground());
    draw_rect(&mut direct, 0.0, 0.0, 1.0, 1.0);
    let direct_island = *direct.islands().keys().collect::<Vec<_>>().first().unwrap();

    // Churned: a scratch segment first, then the rectangle, then the scratch
    // is erased — the rectangle's island lands on a different handle.
    let mut churned = Sketch::on_plane(ground());
    let scratch = churned.add_segment(pt(9.0, 9.0), pt(10.0, 9.0)).unwrap();
    draw_rect(&mut churned, 0.0, 0.0, 1.0, 1.0);
    churned.remove_edge(scratch.new_edges[0]).unwrap();
    assert_eq!(churned.islands().len(), 1);
    let churned_island = *churned
        .islands()
        .keys()
        .collect::<Vec<_>>()
        .first()
        .unwrap();

    assert_ne!(direct_island, churned_island, "the handle really changed");
    assert_eq!(
        direct.unit_keys().islands[&direct_island],
        churned.unit_keys().islands[&churned_island]
    );
    assert_eq!(key_set(&direct), key_set(&churned));
}

#[test]
fn a_clear_in_place_move_changes_only_the_moved_islands_key() {
    let mut s = Sketch::on_plane(ground());
    draw_rect(&mut s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut s, 5.0, 0.0, 6.0, 1.0);
    let a = island_at(&s, 0.0, 0.0);
    let c = island_at(&s, 5.0, 0.0);
    let before = s.unit_keys();

    s.apply_transform_island(a, &Transform::translation(Vec3::new(0.0, 3.0, 0.0)))
        .unwrap();
    let after = s.unit_keys();
    assert_eq!(after.islands.len(), 2);
    assert_ne!(after.islands[&a], before.islands[&a]);
    assert_eq!(after.islands[&c], before.islands[&c]);
}

#[test]
fn curve_key_hashes_center_and_radius() {
    for plane in [ground(), tilted()] {
        let c = plane.point();
        assert_eq!(curve_key(&plane, c, 1.0), curve_key(&plane, c, 1.0));
        assert_ne!(curve_key(&plane, c, 1.0), curve_key(&plane, c, 2.0));
        let c2 = Point3::new(c.x + 1.0, c.y, c.z);
        assert_ne!(curve_key(&plane, c, 1.0), curve_key(&plane, c2, 1.0));
    }
}

type LatticePair = ((i64, i64), (i64, i64));

#[test]
fn keys_of_distinct_lattice_edges_do_not_collide() {
    // Deterministic LCG; 1000 distinct unordered pairs on a 0.25 m lattice.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as i64
    };
    for plane in [ground(), tilted()] {
        let mut seen: BTreeMap<u64, LatticePair> = BTreeMap::new();
        let mut pairs: BTreeSet<LatticePair> = BTreeSet::new();
        while pairs.len() < 1000 {
            let p = (next() % 64, next() % 64);
            let q = (next() % 64, next() % 64);
            if p == q {
                continue;
            }
            let pair = if p < q { (p, q) } else { (q, p) };
            if !pairs.insert(pair) {
                continue;
            }
            let lift = |(i, j): (i64, i64)| {
                let (x, y) = (i as f64 * 0.25, j as f64 * 0.25);
                // Points ON the plane: solve z from the plane equation.
                let n = plane.normal();
                let o = plane.point();
                let z = o.z - (n.x * (x - o.x) + n.y * (y - o.y)) / n.z;
                Point3::new(x, y, z)
            };
            let k = edge_key(&plane, lift(pair.0), lift(pair.1));
            if let Some(prev) = seen.insert(k, pair) {
                panic!("collision: {prev:?} and {pair:?} share key {k:#x}");
            }
        }
        assert_eq!(seen.len(), 1000);
    }
}
