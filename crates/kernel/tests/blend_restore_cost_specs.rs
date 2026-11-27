//! The cost of listing an object's blends ([`Object::blend_features`]),
//! which restores every blend to verify it. A blend's record finds its
//! edges again by their end positions; [`kernel::locate_work_for_test`]
//! counts that search's work. It builds one grid of the object's edges
//! about the positions and looks only at the edges near each one, so its
//! work grows with the object rather than with the blend times the object
//! (a scan of every edge per position cost a rim of 10,000 facets most of
//! a minute).
//!
//! `blend_features_first_call_times` (ignored) prints the time of a first
//! call across object sizes, and `one_blend_against_the_list_times`
//! (ignored) the time of asking about one blend ([`Object::blend_feature`])
//! against the whole list on a rim of 2,500 edges; run them optimized:
//!
//! ```text
//! cargo test --release -p kernel --test all blend_restore_cost_specs -- --ignored --nocapture
//! ```

use crate::common;

use common::curves::*;
use kernel::{BlendProfile, BlendSpec, CornerStyle, EdgeId, Object, Point3};

fn round(edges: Vec<EdgeId>, radius: f64) -> BlendSpec {
    BlendSpec {
        edges,
        profile: BlendProfile::Round {
            radius,
            segments: 4,
        },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

/// A cylinder of `n` claimed facets with its top rim filleted, and its
/// bottom rim too when `blends` is 2: the later fillet trims the walls the
/// first one trimmed, so restoring the first takes the second off and puts
/// it back.
fn filleted_rims(n: usize, blends: usize) -> Object {
    let c = claimed_cylinder(n);
    let (one, _) = c
        .blend_edges(&round(sharp_edges_at_height(&c, 0.1), 0.01))
        .unwrap();
    if blends == 1 {
        return one;
    }
    let (two, _) = one
        .blend_edges(&round(sharp_edges_at_height(&one, 0.0), 0.01))
        .unwrap();
    two
}

/// A unit box whose bottom is a `k` × `k` grid of quads (`k² + 5` faces),
/// with its top edge along y = 0 filleted, and the one along y = 1 too when
/// `blends` is 2: small blends on a large object.
fn gridded_box(k: usize, blends: usize) -> Object {
    let idx = |i: usize, j: usize| i * (k + 1) + j;
    let mut pts: Vec<Point3> = Vec::new();
    for i in 0..=k {
        for j in 0..=k {
            pts.push(p3(i as f64 / k as f64, j as f64 / k as f64, 0.0));
        }
    }
    let t = pts.len();
    pts.extend([
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ]);
    let mut faces: Vec<Vec<usize>> = Vec::new();
    for i in 0..k {
        for j in 0..k {
            faces.push(vec![
                idx(i, j),
                idx(i, j + 1),
                idx(i + 1, j + 1),
                idx(i + 1, j),
            ]);
        }
    }
    faces.push(vec![t, t + 1, t + 2, t + 3]);
    let side = |ring: Vec<usize>, top: [usize; 2]| {
        let mut f = ring;
        f.extend(top);
        f
    };
    faces.push(side((0..=k).map(|i| idx(i, 0)).collect(), [t + 1, t]));
    faces.push(side((0..=k).map(|j| idx(k, j)).collect(), [t + 2, t + 1]));
    faces.push(side(
        (0..=k).rev().map(|i| idx(i, k)).collect(),
        [t + 3, t + 2],
    ));
    faces.push(side((0..=k).rev().map(|j| idx(0, j)).collect(), [t, t + 3]));
    let b = convex_solid(&pts, &faces);
    let e1 = edge_between(&b, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    let (one, _) = b.blend_edges(&round(vec![e1], 0.1)).unwrap();
    if blends == 1 {
        return one;
    }
    let e2 = edge_between(&one, p3(0.0, 1.0, 1.0), p3(1.0, 1.0, 1.0));
    one.blend_edges(&round(vec![e2], 0.1)).unwrap().0
}

/// Every blend of a filleted rim restores and verifies, and finding the
/// rim's edges again on each restore costs work linear in the object: the
/// grid's build (every edge visited, every sample taken) and its queries
/// (cells read, edges tested), per edge of the object, are the same at 256
/// and 1,024 facets. A scan of every edge per stored edge grows with the
/// square of the rim, and a grid built per stored edge with the rim times
/// the object; both fail. [`PER_EDGE`] is about 1.25 times the measured
/// work.
// The work counter counts only with debug assertions on.
#[cfg(debug_assertions)]
#[test]
fn finding_a_rims_edges_again_is_linear_in_the_object() {
    for blends in [1usize, 2] {
        let mut per_edge = Vec::new();
        for n in [256usize, 1024] {
            let obj = filleted_rims(n, blends);
            let before = kernel::locate_work_for_test();
            let features = obj.blend_features();
            let work = kernel::locate_work_for_test() - before;
            assert_eq!(features.len(), blends);
            assert!(
                features.iter().all(|f| f.intact),
                "n = {n}, {blends} blends: {features:?}"
            );
            let w = work as f64 / obj.edges().len() as f64;
            assert!(
                w <= PER_EDGE[blends - 1],
                "n = {n}, {blends} blends: locate work {w:.2} per edge of the object"
            );
            per_edge.push(w);
        }
        assert!(
            per_edge[1] <= 1.1 * per_edge[0],
            "{blends} blends: locate work per edge grew from {:.2} to {:.2}",
            per_edge[0],
            per_edge[1]
        );
    }
}

/// Locate work per edge of the object, at most, with one and two filleted
/// rims (measured: 5.25 and 15.1).
#[cfg(debug_assertions)]
const PER_EDGE: [f64; 2] = [6.6, 18.9];

/// Prints the time of a first [`Object::blend_features`] call (each blend
/// restored and verified) on objects of about 1,000 to 13,000 faces.
#[test]
#[ignore = "a timing to read, not a check; run it optimized"]
fn blend_features_first_call_times() {
    let time = |label: &str, obj: &Object| {
        let start = std::time::Instant::now();
        let features = obj.blend_features();
        let ms = start.elapsed().as_secs_f64() * 1e3;
        assert!(features.iter().all(|f| f.intact), "{label}");
        eprintln!(
            "{label:<28} faces {:>6}  blends {}  {ms:>9.1} ms",
            obj.faces().len(),
            features.len()
        );
    };
    for k in [31usize, 50, 70, 100] {
        for blends in [1, 2] {
            time(&format!("gridded box k={k}"), &gridded_box(k, blends));
        }
    }
    for n in [80usize, 192, 384, 768, 1024, 1428] {
        for blends in [1, 2] {
            time(&format!("filleted rims n={n}"), &filleted_rims(n, blends));
        }
    }
}

/// Prints, on a claimed cylinder of 2,500 rim edges with one and with two
/// filleted rims, the time of the whole list (each blend verified, what a
/// hover once asked for) against asking about each blend alone (what a
/// click on it asks for now).
#[test]
#[ignore = "a timing to read, not a check; run it optimized"]
fn one_blend_against_the_list_times() {
    for blends in [1usize, 2] {
        let obj = filleted_rims(2500, blends);
        let start = std::time::Instant::now();
        let features = obj.blend_features();
        let list_ms = start.elapsed().as_secs_f64() * 1e3;
        assert_eq!(features.len(), blends);
        assert!(features.iter().all(|f| f.intact));
        eprintln!(
            "rims n=2500, {blends} blend(s), faces {:>6}: list {list_ms:>9.1} ms",
            obj.faces().len()
        );
        for (i, row) in features.iter().enumerate() {
            let start = std::time::Instant::now();
            let one = obj.blend_feature(row.id);
            let ms = start.elapsed().as_secs_f64() * 1e3;
            assert_eq!(one.as_ref(), Some(row));
            eprintln!(
                "    blend {} alone ({}): {ms:>9.1} ms",
                row.id.0,
                if i == 0 { "first rim" } else { "second rim" }
            );
        }
    }
}
