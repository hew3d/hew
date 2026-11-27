//! Executable specs for swept blends on copies of one chain: a solid that
//! repeats a feature by rotation gives every copy the same blend, whatever
//! the rounding of each copy's coordinates. Which copies build must not
//! depend on the last bits of where they stand.

use crate::common;

use common::curves::*;
use kernel::{BlendError, BlendProfile, BlendSpec, CornerStyle, EdgeId, Object, Point3};

fn chamfer(edges: Vec<EdgeId>, d: f64) -> BlendSpec {
    BlendSpec {
        edges,
        profile: BlendProfile::Chamfer {
            setback_a: d,
            setback_b: d,
        },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

/// An attempt's outcome, for comparing copies: built, or the refusal's
/// kind (and the feature a size refusal names). Where a refusal is differs
/// from copy to copy by design, so it is not compared.
fn outcome(r: Result<(), BlendError>) -> String {
    match r {
        Ok(()) => "builds".into(),
        Err(BlendError::RadiusTooLarge { limited_by, .. }) => format!("too large ({limited_by:?})"),
        Err(BlendError::UnsupportedVertex { .. }) => "unsupported vertex".into(),
        Err(BlendError::Obstructed { .. }) => "obstructed".into(),
        Err(BlendError::BlendFacesFlush { .. }) => "blend faces flush".into(),
        Err(e) => format!("{e:?}"),
    }
}

/// The chain's two end points (each on one of its edges only).
fn chain_ends(obj: &Object, chain: &[EdgeId]) -> [Point3; 2] {
    let ends: Vec<Point3> = chain
        .iter()
        .flat_map(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            [a, b]
        })
        .filter(|&p| {
            chain
                .iter()
                .filter(|&&e| {
                    let (a, b) = obj.edge_endpoints(e).unwrap();
                    a.approx_eq(p, LOOKUP_EPS) || b.approx_eq(p, LOOKUP_EPS)
                })
                .count()
                == 1
        })
        .collect();
    assert_eq!(ends.len(), 2, "an open chain has two ends");
    [ends[0], ends[1]]
}

/// Which of a chain's two swept attempts runs it counterclockwise about z:
/// attempt 0 runs from the end first in x, then y order (on the kernel's
/// 1e-9 m start grid), attempt 1 from the other end.
fn counterclockwise_attempt(obj: &Object, chain: &[EdgeId]) -> usize {
    let [p, q] = chain_ends(obj, chain);
    let key = |p: Point3| ((p.x / 1e-9).round() as i64, (p.y / 1e-9).round() as i64);
    let (first, other) = if key(p) <= key(q) { (p, q) } else { (q, p) };
    // The turn from the first end to the other, the short way round.
    let turn = first.x * other.y - first.y * other.x;
    if turn > 0.0 { 0 } else { 1 }
}

/// Blends each of the knob's twelve outer rim arcs alone, run both ways
/// round, at every size of `sizes`, and the twelve together; every copy run
/// the same way round must get the same outcome, and the twelve together
/// must build where each builds alone. Some size must build for every copy
/// both ways round, so agreement is never all twelve refusing throughout.
fn twelve_rim_copies_agree(turn: f64, sizes: &[f64]) {
    let (obj, chains) = grooved_knob(turn);
    assert_eq!(chains.len(), 12, "twelve rim arcs between the grooves");
    assert!(chains.iter().all(|c| c.len() == 2), "two edges each");
    let ccw: Vec<usize> = chains
        .iter()
        .map(|c| counterclockwise_attempt(&obj, c))
        .collect();
    let mut failures = Vec::new();
    let mut built_both_ways = 0;
    for &d in sizes {
        // Per direction (0 counterclockwise, 1 clockwise), each copy's
        // outcome.
        let mut by_way: [Vec<String>; 2] = [Vec::new(), Vec::new()];
        let mut built_alone = [true, true];
        for (k, chain) in chains.iter().enumerate() {
            let spec = chamfer(chain.clone(), d);
            for (attempt, built) in built_alone.iter_mut().enumerate() {
                let o = outcome(obj.blend_swept_attempt_for_test(&spec, attempt));
                *built &= o == "builds";
                let way = if attempt == ccw[k] { 0 } else { 1 };
                by_way[way].push(o);
            }
        }
        for (way, outcomes) in by_way.iter().enumerate() {
            if outcomes.iter().any(|o| *o != outcomes[0]) {
                failures.push(format!(
                    "{:.5} mm, {}: {outcomes:?}",
                    d * 1e3,
                    ["counterclockwise", "clockwise"][way]
                ));
            }
        }
        built_both_ways += usize::from(built_alone == [true, true]);
        let all = chamfer(chains.iter().flatten().copied().collect(), d);
        for (attempt, &built) in built_alone.iter().enumerate() {
            if built {
                let together = outcome(obj.blend_swept_attempt_for_test(&all, attempt));
                if together != "builds" {
                    failures.push(format!(
                        "{:.5} mm: every arc builds alone as attempt {attempt}, together {together}",
                        d * 1e3
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "rotated copies of one rim arc disagree:\n{}",
        failures.join("\n")
    );
    assert!(
        built_both_ways > 0,
        "no size built for every copy both ways round"
    );
}

/// Sizes through the outer rim chamfer's limit (its end facets are about
/// 0.214 mm wide): every 0.5 µm from 0.210 mm to 0.224 mm.
fn sizes_through_the_limit() -> Vec<f64> {
    (0..=28)
        .map(|i| (0.210 + 0.0005 * i as f64) * 1e-3)
        .collect()
}

#[test]
fn every_copy_of_a_grooved_knob_rim_arc_chamfers_alike() {
    twelve_rim_copies_agree(0.0, &sizes_through_the_limit());
}

#[test]
fn every_copy_of_a_turned_grooved_knob_rim_arc_chamfers_alike() {
    twelve_rim_copies_agree(7.3_f64.to_radians(), &sizes_through_the_limit());
}
