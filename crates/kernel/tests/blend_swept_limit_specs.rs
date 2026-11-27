//! Executable specs for the size limit of a blend on swept chains: the
//! largest size named when the asked one is too large does not depend on
//! the size asked, and no size above it builds.
//!
//! The model is the scoreboard's grooved knob: its twelve outer rim arcs
//! chamfer up to about 0.2135 mm, refuse from there to about 0.2150 mm
//! (where the chamfer meets the corner of the groove facet the arc ends
//! on), build again up to about 0.2213 mm and are too large past it. A
//! search that took a refusal in that window for "too large" named its
//! lower edge or its upper one depending on the sizes it happened to try.

use crate::common;

use common::curves::*;
use kernel::{BlendError, BlendProfile, BlendSpec, CornerStyle, EdgeId, LimitFeature, Object, tol};

fn chamfer(edges: &[EdgeId], d: f64) -> BlendSpec {
    BlendSpec {
        edges: edges.to_vec(),
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

/// The knob and its twelve outer rim arcs as one spec's edges.
fn knob_rim() -> (Object, Vec<EdgeId>) {
    let (obj, chains) = grooved_knob(0.0);
    let edges = chains.into_iter().flatten().collect();
    (obj, edges)
}

/// Whether the rim chamfers at `d` in one attempt the swept chains offer
/// (from either end, as the commit tries them), with no limit searched.
fn builds(obj: &Object, edges: &[EdgeId], d: f64) -> bool {
    (0..2).any(|attempt| {
        obj.blend_swept_attempt_for_test(&chamfer(edges, d), attempt)
            .is_ok()
    })
}

/// The limit a refusal of the rim chamfer at `asked` names, with how many
/// limit searches and attempts it took.
fn limit_from(obj: &Object, edges: &[EdgeId], asked: f64) -> (f64, LimitFeature, usize, usize) {
    let searches = kernel::limit_searches_for_test();
    let attempts = kernel::limit_attempts_for_test();
    let refused = obj.blend_edges(&chamfer(edges, asked)).map(|_| ());
    let searches = kernel::limit_searches_for_test() - searches;
    let attempts = kernel::limit_attempts_for_test() - attempts;
    match refused {
        Err(BlendError::RadiusTooLarge { max, limited_by }) => {
            (max, limited_by, searches, attempts)
        }
        other => panic!("{} mm: {other:?}", asked * 1e3),
    }
}

/// The attempts one search of the rim may make: the asked size, the floor,
/// the halving scan down to the first size that builds and the bisection
/// to the size limit (22 to 27 on this rim).
const RIM_ATTEMPTS: usize = 30;

#[test]
fn a_grooved_knob_rim_chamfer_names_one_limit_from_any_asked_size() {
    let (obj, edges) = knob_rim();
    let mut named = Vec::new();
    for asked_mm in [1.0, 2.0, 3.0, 4.0, 6.0, 10.0, 26.0] {
        let (max, by, searches, attempts) = limit_from(&obj, &edges, asked_mm * 1e-3);
        assert_eq!(by, LimitFeature::FacetWidth, "{asked_mm} mm: {max}");
        assert_eq!(searches, 1, "{asked_mm} mm: one search");
        assert!(
            attempts <= RIM_ATTEMPTS,
            "{asked_mm} mm: {attempts} attempts"
        );
        named.push((asked_mm, max));
    }
    let (_, first) = named[0];
    assert!(first > 0.0, "{named:?}");
    for &(asked_mm, max) in &named {
        assert!(
            (max - first).abs() <= tol::POINT_MERGE,
            "asked {asked_mm} mm names {:.7} mm, asked {} mm names {:.7} mm",
            max * 1e3,
            named[0].0,
            first * 1e3
        );
    }
}

#[test]
fn no_knob_rim_chamfer_limit_lies_below_a_size_that_builds() {
    let (obj, edges) = knob_rim();
    // 10 mm is the asked size whose search once landed in the refused
    // window and named its lower edge.
    let (max, _, _, _) = limit_from(&obj, &edges, 10e-3);
    assert!(
        obj.blend_edges(&chamfer(&edges, max)).is_ok(),
        "the limit builds: {:.7} mm",
        max * 1e3
    );
    // Every 0.5 um from 0.200 mm to 0.230 mm, through both buildable runs,
    // the refused window between them and the size limit.
    let step = 0.5e-6;
    let mut built_above = Vec::new();
    let mut built_near = false;
    for i in 0..=60 {
        let d = 0.200e-3 + step * i as f64;
        if !builds(&obj, &edges, d) {
            continue;
        }
        if d > max + tol::POINT_MERGE {
            built_above.push(d * 1e3);
        } else if d >= max - 2.0 * step {
            built_near = true;
        }
    }
    assert!(
        built_above.is_empty(),
        "the limit {:.7} mm is below sizes that build: {built_above:?} mm",
        max * 1e3
    );
    assert!(
        built_near,
        "no size of the sweep within two steps under the limit {:.7} mm builds",
        max * 1e3
    );
}
