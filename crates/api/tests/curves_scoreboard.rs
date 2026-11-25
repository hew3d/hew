//! The curves scoreboard (docs/dev/DEVELOPMENT.md, "The curves
//! scoreboard"): every model under `corpus/curves/` is rebuilt through the
//! Hew API, every case is run at every size of its sweep, and each run is
//! classed as
//!
//! - `correct`: a valid, watertight solid that passed the checks below;
//! - `limit`: refused, and the refusal names what is in the way;
//! - `generic`: refused without a cause the user could act on;
//! - `wrong`: a result that fails a check, or a panic.
//!
//! The table is checked in as `corpus/curves/SCOREBOARD.tsv`. This test
//! fails when a fresh run differs from it, and says which cases got worse.
//! `scripts/scoreboard.sh` rewrites the table and prints the same
//! comparison.
//!
//! The blend commands are Preview features, so the whole file is compiled
//! into a preview build only.
#![cfg(hew_preview)]

use kernel::trig;
use kernel::{Document, ObjectId, Point3, WatertightState};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

mod common;
use common::*;

/// A defect in a row, as a panic payload: the run fails with its message
/// where an engine panic becomes a `wrong` line of the table.
struct RowDefect(String);

fn row_defect(message: String) -> ! {
    std::panic::panic_any(RowDefect(message))
}

/// Refusals that say the row is wrong (a locator that finds nothing, a
/// name that does not exist), not what the engine can do.
fn is_row_defect(code: &str) -> bool {
    matches!(
        code,
        "locator_missed"
            | "ambiguous_locator"
            | "unknown_entity"
            | "unknown_profile"
            | "edges_span_objects"
            | "unknown_edge"
            | "empty_spec"
    )
}

// ---------------------------------------------------------------- checks

/// What the checks read from an object before and after.
#[derive(Clone, Copy)]
struct Shape {
    volume: f64,
    min: Point3,
    max: Point3,
    edge_length: f64,
}

fn shape_of(doc: &Document, id: ObjectId) -> Option<Shape> {
    let obj = doc.object(id)?;
    let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for v in obj.vertices().values() {
        let p = v.position;
        min = Point3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
        max = Point3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
    }
    let edge_length = obj
        .edges()
        .keys()
        .filter_map(|e| obj.edge_endpoints(e))
        .map(|(a, b)| (b - a).length())
        .sum();
    Some(Shape {
        volume: obj.volume()?,
        min,
        max,
        edge_length,
    })
}

/// The checks a built result must pass. `None` is a pass; `Some` says which
/// check failed.
fn check_result(
    doc: &Document,
    id: ObjectId,
    before: Option<(Shape, &kernel::topo::Object)>,
    size: f64,
    volume: Option<&str>,
) -> Option<String> {
    let Some(obj) = doc.object(id) else {
        return Some("the object is gone".into());
    };
    if let Err(e) = obj.validate() {
        return Some(format!("invalid: {e:?}"));
    }
    if obj.watertight() != WatertightState::Watertight {
        return Some("not watertight".into());
    }
    let (Some((before, solid)), Some(after)) = (before, shape_of(doc, id)) else {
        return Some("no volume".into());
    };
    // A blend never reaches outside the shape it was cut from.
    let eps = 1e-9;
    let grew = after.min.x < before.min.x - eps
        || after.min.y < before.min.y - eps
        || after.min.z < before.min.z - eps
        || after.max.x > before.max.x + eps
        || after.max.y > before.max.y + eps
        || after.max.z > before.max.z + eps;
    if grew {
        return Some("grew past its bounds".into());
    }
    // It moves some volume, and no more than a square of the size along
    // every edge the object had (a chamfer moves half of that on its own
    // chain, a round less).
    let moved = after.volume - before.volume;
    if moved == 0.0 {
        return Some("volume unchanged".into());
    }
    if moved.abs() > before.edge_length * size * size {
        return Some(format!(
            "moved {} of volume, more than its edges allow",
            sig(moved.abs() * 1e9, "mm³")
        ));
    }
    match volume {
        Some("less") if moved > 0.0 => return Some("volume grew".into()),
        Some("more") if moved < 0.0 => return Some("volume shrank".into()),
        _ => {}
    }
    check_edge_classes(obj, Some(solid))
}

// -------------------------------------------------------- edge classes

/// How one edge should read, from the geometry alone: the kernel's
/// `edge_smoothness` must agree with it on every edge of every model and
/// every built result (the structure lines draw exactly the `Tangent`
/// class, true-curves finish plan W4).
///
/// - `Seam`: a facet seam of one smooth surface. Both faces claim the same
///   surface, or neither claims one and the edge is marked soft (a facet
///   seam of a faceted surface the engine knows only as facets).
/// - `Tangent`: a joint between two different surfaces, at least one of
///   them claimed, whose true normals agree at both ends: a fillet's rail, a
///   corner patch's border, two strips of one blend meeting smoothly, a
///   slot's flat wall against its round end.
/// - A soft edge between two different surfaces reads `Seam` unless both
///   faces belong to one blend and are tangent: a blend marks the joints
///   inside it soft, but anywhere else the mark is the user's Soften.
/// - `Hard`: everything else, which must be a real crease: two different
///   surfaces whose true normals part by at least [`CREASE_MIN`] at an end
///   (a strip's end against an end face, a miter, two strips meeting over
///   a sharp edge), or two unclaimed faces with no soft mark.
///
/// Between a true normal gap of [`TANGENT_MAX`] and [`CREASE_MIN`] the edge
/// is neither, and that is a failure in itself. An unclaimed strip facet
/// of a fillet against a face of no blend (a swept strip's rail) has no
/// true normal to read; it is judged against the strip's own turn
/// ([`swept_rail_reading`]). Any other unclaimed fillet facet against a
/// face of no blend fails.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EdgeReads {
    Hard,
    Seam,
    Tangent,
}

/// The largest gap between two unit normals (a chord) that reads as
/// tangent: far looser than the kernel's own, so float noise never fails
/// the check, and far tighter than any facet.
const TANGENT_MAX: f64 = 1e-6;
/// The smallest gap between two unit normals (a chord, about 1°) that
/// reads as a crease.
const CREASE_MIN: f64 = 0.0175;
/// The smallest gap between two unit normals (a chord, 60°) at which a
/// strip facet against a face of no blend reads as the strip's end, a
/// crease, rather than its rail.
const STRIP_END_MIN: f64 = 1.0;

/// The face across `rail` from `f`.
fn across_rail(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    rail: kernel::ids::EdgeId,
) -> Option<kernel::ids::FaceId> {
    let e = &obj.edges()[rail];
    let face_of = |h: kernel::ids::HalfEdgeId| obj.loops()[obj.half_edges()[h].loop_id].face;
    let (a, b) = (face_of(e.half_edge), face_of(e.twin_half_edge?));
    Some(if a == f { b } else { a })
}

/// How far a strip facet `f` turns across its strip from its rail edge
/// `rail`, whether that took the hop, and the normal of the facet it was
/// read to; `n` is `f`'s normal. The kernel's own reading
/// ([`kernel::topo::Object::swept_rail_gap`]), so the scoreboard and the
/// kernel judge a rail's tangency by one measure: the smallest gap (a
/// chord) between `n` and a strip facet of the same blend across an edge of
/// `f` that touches neither end of the rail, or, for a triangle, one facet
/// further on. `None` when neither finds one.
fn rail_turn(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    n: kernel::Vec3,
    rail: kernel::ids::EdgeId,
) -> Option<(f64, bool, kernel::Vec3)> {
    let wall = across_rail(obj, f, rail)?;
    let g = obj.swept_rail_gap(f, n, rail, wall)?;
    Some((g.turn, g.hop, g.next))
}

/// How the rail edge `rail` between an unclaimed strip facet `f` of a
/// fillet and `wall`, a face of no blend, should read. The strip only
/// approximates a round, so the rail's tangency is judged against the
/// strip's own turn across its section: `g` is how far `f`'s normal parts
/// from the wall's true normal (a chord, the larger at the rail's two
/// ends), and `t` how far the strip turns from `f` to the next facet in
/// from the rail ([`rail_turn`]: across an edge of `f` that touches
/// neither end of the rail, or, for a triangle, one facet further on; the
/// smallest such turn, so a doubtful rail fails rather than passes). Both
/// are the kernel's own reading ([`kernel::RailGap`]), so the line between
/// tangent and creased is drawn in one place. A
/// rail whose gap is no more than that turn is tangent; a gap of
/// [`STRIP_END_MIN`] or more is the strip ending square against the face
/// it stops at, a crease.
///
/// A rail whose gap is more than that turn but short of a square end reads
/// as a crease where the ball rode a convex seam of the solid the strip was
/// cut from (`before`, when given; [`rides_a_seam`]): the rail lies on that
/// seam, and the strip leaves it with a normal within the seam's own fan.
/// Anywhere else such a rail is neither, and fails.
///
/// Fails on a rail with nothing to judge it by: a soft rail (a strip's
/// joint is never a facet seam of its side), a facet with no strip facet
/// in from its rail, or a turn of [`STRIP_END_MIN`] or more, or less than
/// [`CREASE_MIN`]. Limit: a rail farther than [`STRIP_END_MIN`] off its
/// tangent passes as a crease.
fn swept_rail_reading(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    wall: &kernel::topo::Face,
    rail: kernel::ids::EdgeId,
    ends: [Point3; 2],
    before: Option<&kernel::topo::Object>,
) -> Result<Option<EdgeReads>, String> {
    rail_reading(
        obj,
        f,
        obj.faces()[f].plane.normal(),
        wall,
        rail,
        ends,
        before,
    )
}

/// [`swept_rail_reading`] with `n` as the strip facet's normal.
fn rail_reading(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    n: kernel::Vec3,
    wall: &kernel::topo::Face,
    rail: kernel::ids::EdgeId,
    ends: [Point3; 2],
    before: Option<&kernel::topo::Object>,
) -> Result<Option<EdgeReads>, String> {
    if obj.edges()[rail].soft {
        return Err("a soft rail between a strip facet and a face of no blend".into());
    }
    let Some(w) = across_rail(obj, f, rail) else {
        return Err("a rail with one face".into());
    };
    if !std::ptr::eq(&obj.faces()[w], wall) {
        return Err("a rail judged against a face it does not border".into());
    }
    let Some(gap) = obj.swept_rail_gap(f, n, rail, w) else {
        return Err("a strip facet with no neighbor across the strip to judge its rail by".into());
    };
    let t = gap.turn;
    if t >= STRIP_END_MIN {
        return Err(format!(
            "a strip facet that turns {} to its neighbor across the strip",
            sig(t, "")
        ));
    }
    if t < CREASE_MIN {
        return Err("a strip facet with no turn to judge its rail by".into());
    }
    let g = gap.lean;
    if gap.tangent() {
        Ok(Some(EdgeReads::Tangent))
    } else if g >= STRIP_END_MIN {
        Ok(Some(EdgeReads::Hard))
    } else if let Some(b) = before {
        // The ball rode a convex seam of the solid it was cut from: the
        // rail lies on that seam and the strip leaves it inside the seam's
        // fan, to within the strip's own turn read inside the strip (not off
        // this facet, whose turn moves with it), so the rail is that seam's
        // crease, carried on.
        let Some(inner) = interior_turn(obj, f, gap.next) else {
            return Err("no interior turn to judge a seam ride by".into());
        };
        if rides_a_seam(b, ends, n, inner) {
            Ok(Some(EdgeReads::Hard))
        } else {
            Err(format!(
                "strip facet neither on its rail's tangent, ending square nor riding a seam (normals {} apart, the strip turns {} inside)",
                sig(g, ""),
                sig(inner, "")
            ))
        }
    } else {
        Err(format!(
            "strip facet neither on its rail's tangent nor ending square (normals {} apart, the strip turns {})",
            sig(g, ""),
            sig(t, "")
        ))
    }
}

/// The strip's own turn one step in from rail facet `f`: from the facet the
/// rail's turn was read to (the one whose plane normal is `next`, across the
/// strip from `f`, or through the hop), to the facet past it, read as the
/// kernel reads a rail's turn with the edge toward `f` as the rail. A
/// measure of the strip that does not move with `f`'s own normal. `None`
/// where the strip has no such pair.
fn interior_turn(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    next: kernel::Vec3,
) -> Option<f64> {
    let tag = obj.faces()[f].blend?;
    let face_of = |h: kernel::ids::HalfEdgeId| obj.loops()[obj.half_edges()[h].loop_id].face;
    let strip = |g: kernel::ids::FaceId| {
        obj.faces()[g]
            .blend
            .is_some_and(|b| b.id == tag.id && b.role == kernel::topo::BlendRole::Strip)
    };
    // The faces across each edge of `g`, with the edge.
    let across = |g: kernel::ids::FaceId| -> Vec<(kernel::ids::FaceId, kernel::ids::EdgeId)> {
        obj.loop_half_edges(obj.faces()[g].outer_loop)
            .filter_map(|h| {
                let twin = obj.half_edges()[h].twin?;
                Some((face_of(twin), obj.half_edges()[h].edge))
            })
            .collect()
    };
    // The facet the rail's turn was read to: across from `f`, or one facet
    // further on (the hop); with the edge it shares with `f` or with the
    // facet between.
    let mut found = None;
    'search: for (g, e) in across(f) {
        if !strip(g) {
            continue;
        }
        if obj.faces()[g].plane.normal() == next {
            found = Some((g, e));
            break;
        }
        for (h, e2) in across(g) {
            if h != f && strip(h) && obj.faces()[h].plane.normal() == next {
                found = Some((h, e2));
                break 'search;
            }
        }
    }
    let (inner, edge) = found?;
    let toward = obj
        .edge_faces_for_test(edge)
        .map(|(a, b)| if a == inner { b } else { a })?;
    obj.swept_rail_gap(inner, next, edge, toward)
        .map(|g| g.turn)
}

/// How far off a line a rail end may lie and still lie on it: the walk puts
/// a rail point on its seam exactly, so only float noise.
const ON_SEAM: f64 = 1e-9;

/// Whether the rail from `ends[0]` to `ends[1]` lies on a convex edge of
/// `before` (both ends on the edge's line, to [`ON_SEAM`], and within its
/// extent), and the strip facet's normal `n` lies within that edge's
/// normal fan, the arc of normals between its two faces' planes, to within
/// `t` (a chord: the turn the tangency check allows).
fn rides_a_seam(before: &kernel::topo::Object, ends: [Point3; 2], n: kernel::Vec3, t: f64) -> bool {
    before.edges().keys().any(|e| {
        let Some((a, b)) = before.edge_endpoints(e) else {
            return false;
        };
        let d = b - a;
        let len = d.length();
        if len <= ON_SEAM {
            return false;
        }
        let u = d / len;
        let on = ends.iter().all(|&x| {
            let s = (x - a).dot(u);
            let off = (x - a) - u * s;
            off.length() <= ON_SEAM && s >= -ON_SEAM && s <= len + ON_SEAM
        });
        on && seam_fan(before, e).is_some_and(|(n1, n2)| fan_gap(n, n1, n2) <= t)
    })
}

/// The plane normals of `e`'s two faces where `e` is a convex edge of
/// `obj` (the second face's far corners lie behind the first face's plane).
fn seam_fan(
    obj: &kernel::topo::Object,
    e: kernel::ids::EdgeId,
) -> Option<(kernel::Vec3, kernel::Vec3)> {
    let edge = &obj.edges()[e];
    let face_of = |h: kernel::ids::HalfEdgeId| obj.loops()[obj.half_edges()[h].loop_id].face;
    let (f1, f2) = (face_of(edge.half_edge), face_of(edge.twin_half_edge?));
    let (p1, p2) = (obj.faces()[f1].plane, obj.faces()[f2].plane);
    let far = obj
        .loop_positions(obj.faces()[f2].outer_loop)
        .map(|x| p1.signed_distance(x))
        .fold(0.0_f64, |m, d| if d.abs() > m.abs() { d } else { m });
    (far < -ON_SEAM).then(|| (p1.normal(), p2.normal()))
}

/// How far (a chord) unit normal `n` lies from the arc of unit normals
/// running the short way from `n1` to `n2`.
fn fan_gap(n: kernel::Vec3, n1: kernel::Vec3, n2: kernel::Vec3) -> f64 {
    let ends = (n - n1).length().min((n - n2).length());
    let Ok(axis) = n1.cross(n2).normalized() else {
        return ends;
    };
    // `n` with its part along the arc's axis removed, if it falls between
    // the arc's ends.
    let Ok(m) = (n - axis * n.dot(axis)).normalized() else {
        return ends;
    };
    if n1.cross(m).dot(axis) >= 0.0 && m.cross(n2).dot(axis) >= 0.0 {
        (n - m).length()
    } else {
        ends
    }
}

fn expected_reading(
    obj: &kernel::topo::Object,
    edge: kernel::ids::EdgeId,
    before: Option<&kernel::topo::Object>,
) -> Result<Option<EdgeReads>, String> {
    let e = &obj.edges()[edge];
    let Some(twin) = e.twin_half_edge else {
        return Ok(None);
    };
    if e.crease {
        // A crease the user set reads hard whatever the geometry.
        return Ok(Some(EdgeReads::Hard));
    }
    let face_id = |h: kernel::ids::HalfEdgeId| obj.loops()[obj.half_edges()[h].loop_id].face;
    let face_of = |h: kernel::ids::HalfEdgeId| &obj.faces()[face_id(h)];
    let (fa, fb) = (face_of(e.half_edge), face_of(twin));
    let true_normal = |f: &kernel::topo::Face, x: Point3| {
        let n = f.plane.normal();
        match f.surface.and_then(|s| s.normal_at(x)) {
            Some(a) if a.dot(n) < 0.0 => -a,
            Some(a) => a,
            None => n,
        }
    };
    let (p, q) = obj
        .edge_endpoints(edge)
        .ok_or_else(|| "an edge without endpoints".to_string())?;
    match (fa.surface, fb.surface) {
        (Some(a), Some(b)) if a.same_surface(&b) => return Ok(Some(EdgeReads::Seam)),
        (None, None) => {
            if fa.blend.is_some() != fb.blend.is_some() {
                let (strip_h, wall) = if fa.blend.is_some() {
                    (e.half_edge, fb)
                } else {
                    (twin, fa)
                };
                let tag = obj.faces()[face_id(strip_h)].blend.expect("one blend face");
                if tag.kind == kernel::topo::BlendKind::Fillet {
                    if tag.role != kernel::topo::BlendRole::Strip {
                        return Err("an unclaimed fillet facet against a face of no blend".into());
                    }
                    return swept_rail_reading(obj, face_id(strip_h), wall, edge, [p, q], before);
                }
            }
            return Ok(Some(if e.soft {
                EdgeReads::Seam
            } else {
                EdgeReads::Hard
            }));
        }
        _ => {}
    }
    // At least one claim, two different surfaces: tangent or a crease.
    let gap = [p, q]
        .into_iter()
        .map(|x| (true_normal(fa, x) - true_normal(fb, x)).length())
        .fold(0.0, f64::max);
    let one_blend = fa.blend.zip(fb.blend).is_some_and(|(a, b)| a.id == b.id);
    if e.soft && !one_blend {
        // Softened on purpose (Soften): hidden like a facet seam.
        Ok(Some(EdgeReads::Seam))
    } else if gap <= TANGENT_MAX {
        Ok(Some(EdgeReads::Tangent))
    } else if e.soft {
        Ok(Some(EdgeReads::Seam))
    } else if gap >= CREASE_MIN {
        Ok(Some(EdgeReads::Hard))
    } else {
        Err(format!(
            "two surfaces neither tangent nor creased (normals {} apart)",
            sig(gap, "")
        ))
    }
}

/// What is wrong with an edge the kernel reads `got` where the geometry
/// says `want`; `None` when nothing is.
fn reading_problem(got: EdgeReads, want: Result<Option<EdgeReads>, String>) -> Option<String> {
    match want {
        Ok(None) => None,
        Ok(Some(want)) if want == got => None,
        Ok(Some(want)) => Some(format!("{got:?} where {want:?} is right")),
        Err(why) => Some(why),
    }
}

/// Checks every interior edge of `obj`, made from `before` where that is
/// known. `None` is a pass; `Some` names the first edge that reads wrong,
/// and how many do.
fn check_edge_classes(
    obj: &kernel::topo::Object,
    before: Option<&kernel::topo::Object>,
) -> Option<String> {
    let mut first = None;
    let mut count = 0usize;
    for edge in obj.edges().keys() {
        let got = match obj.edge_smoothness(edge) {
            kernel::EdgeSmoothness::Hard => EdgeReads::Hard,
            kernel::EdgeSmoothness::Seam => EdgeReads::Seam,
            kernel::EdgeSmoothness::Tangent => EdgeReads::Tangent,
        };
        let problem = reading_problem(got, expected_reading(obj, edge, before));
        if let Some(problem) = problem {
            count += 1;
            if first.is_none() {
                let at = obj
                    .edge_endpoints(edge)
                    .map(|(p, q)| {
                        let mid = p + (q - p) * 0.5;
                        point(&json!([mid.x, mid.y, mid.z]))
                    })
                    .unwrap_or_default();
                first = Some(format!("edge at {at}: {problem}"));
            }
        }
    }
    first.map(|f| format!("{count} edges read wrong; first, {f}"))
}

// --------------------------------------------------------------- classes

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Wrong,
    Generic,
    Limit,
    Correct,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Wrong => "wrong",
            Class::Generic => "generic",
            Class::Limit => "limit",
            Class::Correct => "correct",
        }
    }
    fn parse(s: &str) -> Option<Class> {
        Some(match s {
            "wrong" => Class::Wrong,
            "generic" => Class::Generic,
            "limit" => Class::Limit,
            "correct" => Class::Correct,
            _ => return None,
        })
    }
}

/// Three significant digits: enough to read, and steady across platforms.
// `log10` only counts the digits to print. The platform's rounding of it can
// differ only where `v` is within an ulp of a power of ten, and there the one
// extra digit printed is a zero, which the trim below removes, so the text is
// the same everywhere.
#[allow(clippy::disallowed_methods)]
fn sig(v: f64, unit: &str) -> String {
    if v == 0.0 {
        return format!("0{unit}");
    }
    if !v.is_finite() {
        return format!("{v}{unit}");
    }
    let digits = (2 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.digits$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    format!("{s}{unit}")
}

fn point(v: &Value) -> String {
    // Below a nanometer a coordinate is zero: noise there would print
    // differently from one machine to the next.
    let c = |i: usize| {
        let mm = v[i].as_f64().unwrap_or(f64::NAN) * 1e3;
        sig(if mm.abs() < 1e-6 { 0.0 } else { mm }, "")
    };
    format!("({}, {}, {})mm", c(0), c(1), c(2))
}

/// Classes a refusal: `limit` when it names what is in the way, `generic`
/// when it only says the engine cannot do it. A refusal this table does not
/// know is `generic`, so a new variant has to earn its place here.
fn class_of_refusal(code: &str, detail: &Value, asked: Option<f64>) -> (Class, String) {
    match code {
        "radius_too_large" => {
            let by = detail["limited_by"].as_str().unwrap_or("unknown");
            let largest = detail["max"].as_f64().unwrap_or(f64::NAN);
            let max = sig(largest * 1e3, "mm");
            // A refusal that says a larger size than the one asked for
            // would fit contradicts itself.
            if asked.is_some_and(|size| largest > size * (1.0 + 1e-6)) {
                return (
                    Class::Wrong,
                    format!("{code} max={max} by={by}, above the size refused"),
                );
            }
            // A size that fits, not shown to be the largest, names nothing in
            // the way.
            let class = if by == "unknown" || by == "known_fit" {
                Class::Generic
            } else {
                Class::Limit
            };
            (class, format!("{code} max={max} by={by}"))
        }
        "profile_too_wide" => (
            Class::Limit,
            format!(
                "{code} needed={} available={}",
                sig(detail["needed"].as_f64().unwrap_or(f64::NAN) * 1e3, "mm"),
                sig(detail["available"].as_f64().unwrap_or(f64::NAN) * 1e3, "mm"),
            ),
        ),
        "angle_does_not_fit" => (
            Class::Limit,
            format!(
                "{code} max={}",
                sig(detail["max"].as_f64().unwrap_or(f64::NAN), "rad")
            ),
        ),
        "blend_faces_flush" | "obstructed" => {
            (Class::Limit, format!("{code} at={}", point(&detail["at"])))
        }
        "profile_needs_convex_edge"
        | "bullnose_needs_parallel_sides"
        | "edge_is_smooth"
        | "stop_out_of_range"
        | "segments_out_of_range"
        | "blend_needs_solid" => (Class::Limit, code.to_string()),
        // The corner named is part of the row: a refusal that moves to
        // another corner is a change the board should show.
        "unsupported_vertex" => (
            Class::Generic,
            format!("{code} at={}", point(&detail["at"])),
        ),
        _ => (Class::Generic, code.to_string()),
    }
}

// ------------------------------------------------------------------- run

/// One line of the table.
#[derive(Clone, PartialEq, Debug)]
struct Row {
    model: String,
    case: String,
    size: String,
    /// `fits` (must build), `over` (cannot exist), or empty.
    expect: String,
    class: Class,
    detail: String,
}

impl Row {
    fn key(&self) -> (String, String, String) {
        (self.model.clone(), self.case.clone(), self.size.clone())
    }
    /// How good the answer is, expectation counted in: a named limit where
    /// nothing can exist is as good as a build.
    fn rank(&self) -> u8 {
        match (self.class, self.expect.as_str()) {
            (Class::Limit, "over") => 3,
            (Class::Correct, _) => 3,
            (Class::Limit, _) => 2,
            (Class::Generic, _) => 1,
            (Class::Wrong, _) => 0,
        }
    }
    fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.model,
            self.case,
            self.size,
            self.expect,
            self.class.name(),
            self.detail.replace(['\t', '\r', '\n'], " ")
        )
    }
    fn parse(line: &str) -> Option<Row> {
        let f: Vec<&str> = line.split('\t').collect();
        (f.len() == 6 && Class::parse(f[4]).is_some()).then(|| Row {
            model: f[0].into(),
            case: f[1].into(),
            size: f[2].into(),
            expect: f[3].into(),
            class: Class::parse(f[4]).unwrap_or(Class::Wrong),
            detail: f[5].into(),
        })
    }
}

fn length(name: &str, what: &str, s: &str) -> f64 {
    parse_length(s).unwrap_or_else(|| panic!("{name}: {what} {s:?} is not a length with a unit"))
}

/// Undoes back to the built model and says whether that restored it.
fn rewind(s: &mut Session, depth: usize, hash: u64) -> bool {
    while s.doc.undo_depth() > depth {
        if s.doc.undo().is_err() {
            return false;
        }
    }
    s.doc.undo_depth() == depth && s.doc.state_hash() == hash
}

fn run_model(name: &str, model: &Model, rows: &mut Vec<Row>) {
    let mut session = Session::build(model, name);
    for id in session.doc.live_object_ids() {
        if let Some(why) = session
            .doc
            .object(id)
            .and_then(|o| check_edge_classes(o, None))
        {
            panic!("{name}: the built model's {why}");
        }
    }
    let depth = session.doc.undo_depth();
    let hash = session.doc.state_hash();
    let labels = session.labels.clone();

    for case in &model.cases {
        let sweep = case.sweep.clone().unwrap_or_else(|| {
            match case.method.as_str() {
                "hew.solid.chamfer" => "distance",
                _ => "radius",
            }
            .to_string()
        });
        let fits_to = case.fits_to.as_deref().map(|s| length(name, "fits_to", s));
        let never_above = case
            .never_above
            .as_deref()
            .map(|s| length(name, "never_above", s));
        let first = rows.len();
        let mut sweep_said: Vec<(f64, Option<f64>)> = Vec::new();
        for size_label in &case.sizes {
            let size = length(name, "size", size_label);
            let expect = if never_above.is_some_and(|m| size > m * (1.0 + 1e-9)) {
                "over"
            } else if fits_to.is_some_and(|m| size <= m * (1.0 + 1e-9)) {
                "fits"
            } else {
                ""
            };
            // Each run gets a thread of its own: a panic anywhere in it is
            // caught at the join, and nothing the engine keeps per thread
            // outlives it.
            let ran = std::thread::scope(|scope| {
                let session = &mut session;
                let sweep = sweep.as_str();
                scope
                    .spawn(move || run_case(session, name, case, sweep, size))
                    .join()
            });
            let (mut class, mut detail, max, panicked) = match ran {
                Ok((class, detail, max)) => (class, detail, max, false),
                Err(payload) => {
                    if let Some(RowDefect(message)) = payload.downcast_ref::<RowDefect>() {
                        panic!("{message}");
                    }
                    // The message without its source location, so the
                    // table does not change when a line number does.
                    let message = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("no message");
                    let line = message.lines().next().unwrap_or_default();
                    let short: String = line.chars().take(120).collect();
                    (Class::Wrong, format!("panic: {short}"), None, true)
                }
            };
            if class == Class::Correct && expect == "over" {
                class = Class::Wrong;
                detail = "built at a size the shape cannot hold".into();
            }
            // A panic can leave the document half changed: start over
            // rather than trust an undo of it.
            if panicked || !rewind(&mut session, depth, hash) {
                if class != Class::Wrong {
                    class = Class::Wrong;
                    detail = "undo did not restore the model".into();
                }
                session = Session::build(model, name);
                assert!(
                    session.doc.undo_depth() == depth && session.doc.state_hash() == hash,
                    "{name}: the model did not rebuild the same"
                );
            }
            session.labels = labels.clone();
            sweep_said.push((size, max));
            rows.push(Row {
                model: name.to_string(),
                case: case.id.clone(),
                size: size_label.clone(),
                expect: expect.to_string(),
                class,
                detail,
            });
        }
        below_a_build(&mut rows[first..], &sweep_said);
    }
}

/// A `radius_too_large` that names a largest size below a smaller size its
/// own sweep built (and passed every check) contradicts itself as surely as one that
/// names a size above the one refused: those rows of one case's sweep are
/// `wrong`. `said` holds each row's size and the `max` its refusal named.
fn below_a_build(rows: &mut [Row], said: &[(f64, Option<f64>)]) {
    let built: Vec<(f64, String)> = rows
        .iter()
        .zip(said)
        .filter(|(r, _)| r.class == Class::Correct)
        .map(|(r, &(size, _))| (size, r.size.clone()))
        .collect();
    for (row, &(asked, max)) in rows.iter_mut().zip(said) {
        let Some(max) = max else { continue };
        if row.class == Class::Wrong {
            continue;
        }
        // The limit is the largest size below the one refused, to within
        // the search's own precision.
        if let Some((_, label)) = built
            .iter()
            .filter(|&&(size, _)| size < asked && size > max * (1.0 + 1e-6) + 2e-9)
            .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            row.class = Class::Wrong;
            row.detail = format!("{}, below {label}, which builds", row.detail);
        }
    }
}

/// One run: its class and detail, and the `max` a `radius_too_large`
/// refusal of the swept call named.
fn run_case(
    s: &mut Session,
    name: &str,
    case: &Case,
    sweep: &str,
    size: f64,
) -> (Class, String, Option<f64>) {
    for (i, frame) in case.before.iter().enumerate() {
        match s.frame(frame) {
            Called::Ok(_) => {}
            Called::Refused(code, detail) => {
                if is_row_defect(&code) {
                    row_defect(format!("{name}/{}: before[{i}]: {code} {detail}", case.id));
                }
                let (class, text) = class_of_refusal(&code, &detail, None);
                return (class, format!("before[{i}]: {text}"), None);
            }
            Called::Broken(why) => {
                row_defect(format!("{name}/{}: before[{i}] failed: {why}", case.id))
            }
        }
    }
    let mut params = substitute(&case.params, &s.labels)
        .unwrap_or_else(|e| row_defect(format!("{name}/{}: {e}", case.id)));
    params
        .as_object_mut()
        .unwrap_or_else(|| row_defect(format!("{name}/{}: params must be an object", case.id)))
        .insert(sweep.to_string(), json!(size));

    // The object is not known until the call answers, so every live object
    // is measured first.
    let before: BTreeMap<ObjectId, (Shape, kernel::topo::Object)> = s
        .doc
        .live_object_ids()
        .into_iter()
        .filter_map(|id| {
            let solid = s.doc.object(id)?.clone();
            shape_of(&s.doc, id).map(|shape| (id, (shape, solid)))
        })
        .collect();

    match s.call(&case.method, params) {
        Called::Broken(why) => row_defect(format!("{name}/{}: {why}", case.id)),
        Called::Refused(code, detail) => {
            if is_row_defect(&code) {
                row_defect(format!("{name}/{}: {code} {detail}", case.id));
            }
            let (class, text) = class_of_refusal(&code, &detail, Some(size));
            let max = (code == "radius_too_large")
                .then(|| detail["max"].as_f64())
                .flatten();
            (class, text, max)
        }
        Called::Ok(result) => {
            let Some(id) = result["object_id"].as_str().and_then(|p| s.object(p)) else {
                return (Class::Wrong, "the result names no object".into(), None);
            };
            // The engine says what size it built. Only the plain forms
            // report the swept number back.
            let plain = matches!(sweep, "radius" | "distance")
                && case.params.get("profile").is_none()
                && case.params.get("angle").is_none();
            if let (true, Some(built)) = (plain, result["size"].as_f64())
                && (built - size).abs() > 1e-9 * size.max(1.0)
            {
                return (
                    Class::Wrong,
                    format!(
                        "built at {}, not the size asked for",
                        sig(built * 1e3, "mm")
                    ),
                    None,
                );
            }
            match check_result(
                &s.doc,
                id,
                before.get(&id).map(|(shape, solid)| (*shape, solid)),
                size,
                case.volume.as_deref(),
            ) {
                Some(why) => (Class::Wrong, why, None),
                None => (Class::Correct, String::new(), None),
            }
        }
    }
}

fn load_models() -> Vec<(String, Model)> {
    let dir = corpus_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let model: Model =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!model.cases.is_empty(), "{name}: no cases");
            let mut seen = std::collections::BTreeSet::new();
            for case in &model.cases {
                assert!(
                    seen.insert(case.id.clone()),
                    "{name}: two cases are named {:?}",
                    case.id
                );
                assert!(!case.sizes.is_empty(), "{name}/{}: no sizes", case.id);
                let mut sizes = std::collections::BTreeSet::new();
                for size in &case.sizes {
                    assert!(
                        sizes.insert(size),
                        "{name}/{}: size {size:?} is listed twice",
                        case.id
                    );
                }
            }
            (name, model)
        })
        .collect()
}

// ----------------------------------------------------------------- table

const HEADER: &str = "model\tcase\tsize\texpect\tclass\tdetail";

fn render(rows: &[Row]) -> String {
    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# The curves scoreboard. Written by scripts/scoreboard.sh; do not edit."
    );
    let _ = writeln!(
        out,
        "# {} cases: {} correct, {} limit, {} generic, {} wrong.",
        rows.len(),
        count(&|r| r.class == Class::Correct),
        count(&|r| r.class == Class::Limit),
        count(&|r| r.class == Class::Generic),
        count(&|r| r.class == Class::Wrong),
    );
    let _ = writeln!(
        out,
        "# Open: {} refused or wrong at a size that must build; {} limits where nothing larger can exist.",
        count(&|r| r.expect == "fits" && r.class != Class::Correct),
        count(&|r| r.expect == "over" && r.class == Class::Limit),
    );
    let _ = writeln!(out, "{HEADER}");
    for r in rows {
        let _ = writeln!(out, "{}", r.line());
    }
    out
}

fn parse_table(text: &str) -> Vec<Row> {
    text.lines()
        .filter(|l| !l.starts_with('#') && *l != HEADER && !l.is_empty())
        .map(|l| Row::parse(l).unwrap_or_else(|| panic!("not a line of the table: {l:?}")))
        .collect()
}

/// The comparison, worst news first. Empty when the tables agree.
fn compare(old: &[Row], new: &[Row]) -> String {
    let old_by: BTreeMap<_, _> = old.iter().map(|r| (r.key(), r)).collect();
    let new_by: BTreeMap<_, _> = new.iter().map(|r| (r.key(), r)).collect();
    let mut worse = Vec::new();
    let mut better = Vec::new();
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut gone = Vec::new();
    let show = |r: &Row| {
        if r.detail.is_empty() {
            r.class.name().to_string()
        } else {
            format!("{} ({})", r.class.name(), r.detail)
        }
    };
    for r in new {
        let id = format!("{}/{} at {}", r.model, r.case, r.size);
        match old_by.get(&r.key()) {
            None => added.push(format!("  {id}: {}", show(r))),
            Some(o) if **o == *r => {}
            Some(o) => {
                let line = format!("  {id}: {} -> {}", show(o), show(r));
                match r.rank().cmp(&o.rank()) {
                    std::cmp::Ordering::Less => worse.push(line),
                    std::cmp::Ordering::Greater => better.push(line),
                    std::cmp::Ordering::Equal => changed.push(line),
                }
            }
        }
    }
    for o in old {
        if !new_by.contains_key(&o.key()) {
            gone.push(format!(
                "  {}/{} at {}: {}",
                o.model,
                o.case,
                o.size,
                show(o)
            ));
        }
    }
    let mut out = String::new();
    for (title, lines) in [
        ("WORSE", worse),
        ("BETTER", better),
        ("CHANGED (same class of answer)", changed),
        ("NEW", added),
        ("GONE", gone),
    ] {
        if !lines.is_empty() {
            let _ = writeln!(out, "{title}: {}", lines.len());
            for l in lines {
                let _ = writeln!(out, "{l}");
            }
        }
    }
    out
}

#[test]
fn the_scoreboard_matches_the_checked_in_table() {
    // Models are independent, so each runs on its own thread; the table
    // keeps the models in name order whatever finishes first.
    let models = load_models();
    let rows: Vec<Row> = std::thread::scope(|scope| {
        let running: Vec<_> = models
            .iter()
            .map(|(name, model)| {
                scope.spawn(move || {
                    let started = std::time::Instant::now();
                    let mut rows = Vec::new();
                    run_model(name, model, &mut rows);
                    eprintln!("{name}: {:.1?}", started.elapsed());
                    rows
                })
            })
            .collect();
        running
            .into_iter()
            .flat_map(|thread| match thread.join() {
                Ok(rows) => rows,
                // A defect in a row: fail with its message.
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    });

    let path = corpus_dir().join("SCOREBOARD.tsv");
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    let old = parse_table(&on_disk);
    let fresh = render(&rows);
    let diff = compare(&old, &rows);
    let summary: String = fresh.lines().take(3).skip(1).collect::<Vec<_>>().join("\n");

    if std::env::var_os("UPDATE_SCOREBOARD").is_some_and(|v| v == "1") {
        std::fs::write(&path, &fresh).unwrap();
        println!("{summary}");
        if diff.is_empty() {
            println!("# No case changed since the last run.");
        } else {
            println!("{diff}");
        }
        return;
    }
    assert!(
        diff.is_empty(),
        "the curves scoreboard changed. Run scripts/scoreboard.sh, read the comparison, and \
         commit corpus/curves/SCOREBOARD.tsv with the change.\n{diff}"
    );
    // The same cases in another order, or an edited summary: the file must
    // be exactly what the script writes.
    assert!(
        on_disk == fresh,
        "corpus/curves/SCOREBOARD.tsv is not what a run writes (its order or its summary \
         differs). Run scripts/scoreboard.sh and commit the result."
    );
}

#[test]
fn lengths_parse_with_their_units() {
    let close = |s: &str, want: f64| {
        let got = parse_length(s).unwrap_or_else(|| panic!("{s} parses"));
        assert!((got - want).abs() < 1e-12, "{s}: {got} vs {want}");
    };
    close("3mm", 0.003);
    close("2.5cm", 0.025);
    close("0.1m", 0.1);
    close("1/8in", 0.003175);
    close("0.5in", 0.0127);
    close("-2mm", -0.002);
    assert_eq!(parse_length("round"), None);
    assert_eq!(parse_length("1/0in"), None);
    assert_eq!(parse_length("12"), None);
}

#[test]
fn a_worse_case_is_named_first() {
    let row = |size: &str, class, detail: &str| Row {
        model: "box".into(),
        case: "top".into(),
        size: size.into(),
        expect: String::new(),
        class,
        detail: detail.into(),
    };
    let old = [
        row("1mm", Class::Correct, ""),
        row("2mm", Class::Generic, "unsupported_edge"),
        row(
            "3mm",
            Class::Limit,
            "radius_too_large max=2mm by=adjacent_face",
        ),
    ];
    let new = [
        row(
            "1mm",
            Class::Limit,
            "radius_too_large max=0.5mm by=short_edge",
        ),
        row("2mm", Class::Correct, ""),
        row(
            "3mm",
            Class::Limit,
            "radius_too_large max=2.5mm by=adjacent_face",
        ),
        row("4mm", Class::Wrong, "not watertight"),
    ];
    let diff = compare(&old, &new);
    assert!(diff.starts_with("WORSE: 1\n  box/top at 1mm: correct -> limit"));
    assert!(diff.contains("BETTER: 1\n  box/top at 2mm"));
    assert!(diff.contains("CHANGED (same class of answer): 1\n  box/top at 3mm"));
    assert!(diff.contains("NEW: 1\n  box/top at 4mm: wrong (not watertight)"));
    assert_eq!(compare(&new, &new), "");
    // The table round-trips through its file form.
    assert_eq!(parse_table(&render(&new)), new);
    // A tab in a detail cannot split its line into more fields.
    let tabbed = row("5mm", Class::Wrong, "panic: a\tb");
    let read = parse_table(&render(std::slice::from_ref(&tabbed)));
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].detail, "panic: a b");
    // A line that is not a row is refused, not skipped.
    assert!(Row::parse("box\ttop\t1mm\t\tCorrect\t").is_none());
    assert!(Row::parse("box\ttop\t1mm\tcorrect").is_none());
}

#[test]
fn a_refusal_that_contradicts_itself_is_wrong() {
    let detail = json!({ "max": 0.004, "limited_by": "adjacent_face" });
    // Refused at 5 mm with 4 mm as the largest that fits: a named limit.
    assert_eq!(
        class_of_refusal("radius_too_large", &detail, Some(0.005)).0,
        Class::Limit
    );
    // Refused at 3 mm while saying 4 mm fits.
    assert_eq!(
        class_of_refusal("radius_too_large", &detail, Some(0.003)).0,
        Class::Wrong
    );
    let unknown = json!({ "max": 0.004, "limited_by": "unknown" });
    assert_eq!(
        class_of_refusal("radius_too_large", &unknown, Some(0.005)).0,
        Class::Generic
    );
    assert_eq!(
        class_of_refusal("unsupported_edge", &json!({}), Some(0.005)).0,
        Class::Generic
    );
    assert!(is_row_defect("ambiguous_locator"));
    assert!(!is_row_defect("radius_too_large"));
}

#[test]
fn a_limit_below_a_size_its_sweep_built_is_wrong() {
    let at = |size: &str, class: Class, detail: &str| Row {
        model: "knob".into(),
        case: "rim".into(),
        size: size.into(),
        expect: String::new(),
        class,
        detail: detail.into(),
    };
    let mut rows = vec![
        at("0.1mm", Class::Correct, ""),
        at("1mm", Class::Generic, "radius_too_large max=0mm by=unknown"),
        at(
            "2mm",
            Class::Limit,
            "radius_too_large max=0.1mm by=facet_width",
        ),
        at(
            "3mm",
            Class::Limit,
            "radius_too_large max=0.05mm by=facet_width",
        ),
        at("4mm", Class::Generic, "unsupported_vertex"),
    ];
    let said = [
        (0.0001, None),
        (0.001, Some(0.0)),
        (0.002, Some(0.0001)),
        (0.003, Some(0.00005)),
        (0.004, None),
    ];
    below_a_build(&mut rows, &said);
    let classes: Vec<Class> = rows.iter().map(|r| r.class).collect();
    // 0.1 mm built: a maximum of 0 or 0.05 mm contradicts it; 0.1 mm
    // itself, and a refusal that names no size, do not.
    assert_eq!(
        classes,
        [
            Class::Correct,
            Class::Wrong,
            Class::Limit,
            Class::Wrong,
            Class::Generic
        ]
    );
    assert_eq!(
        rows[1].detail,
        "radius_too_large max=0mm by=unknown, below 0.1mm, which builds"
    );
}

#[test]
fn a_clean_swept_round_reads_its_rails_as_tangent() {
    // A 12-sided boss 40 mm across on a 60 mm square plate, its foot
    // rounded as one swept loop: every rail of the strip reads as a tangent
    // joint, to the check and to the kernel, and every edge of the result
    // reads as its geometry says.
    use kernel::loft::{LoftProfile, LoftSpec};
    let prism = |pts: &[(f64, f64)], z0: f64, z1: f64| {
        let prof = |z: f64| LoftProfile {
            points: pts.iter().map(|&(x, y)| Point3::new(x, y, z)).collect(),
            curves: Vec::new(),
            smooth: Vec::new(),
            through: Vec::new(),
            start_offset: 0,
        };
        kernel::Object::from_loft(&LoftSpec {
            profiles: vec![prof(z0), prof(z1)],
            apex: None,
            rails: Vec::new(),
            stations: None,
        })
        .unwrap()
    };
    let ring: Vec<(f64, f64)> = (0..12)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 12.0;
            (0.02 * trig::cos(a), 0.02 * trig::sin(a))
        })
        .collect();
    let s = 0.03;
    let plate = prism(&[(-s, -s), (s, -s), (s, s), (-s, s)], 0.0, 0.004);
    let boss = prism(&ring, 0.002, 0.01);
    let joined = kernel::Object::boolean(
        kernel::BooleanOp::Union,
        &plate,
        &boss,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let seam: Vec<_> = joined
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = joined.edge_endpoints(e).unwrap();
            [p, q]
                .iter()
                .all(|r| (r.z - 0.004).abs() < 1e-9 && trig::hypot(r.x, r.y) < 0.021)
        })
        .collect();
    assert_eq!(seam.len(), 12);
    for r in [0.0005, 0.001, 0.002] {
        let spec = kernel::BlendSpec {
            edges: seam.clone(),
            profile: kernel::BlendProfile::Round {
                radius: r,
                segments: 4,
            },
            primary: None,
            corners: kernel::CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        };
        let after = joined
            .blend_swept_result_for_test(&spec)
            .unwrap_or_else(|| panic!("{r}: the swept round refused"));
        assert_eq!(check_edge_classes(&after, None), None, "{r}");
        let rails = rails_of(&after);
        assert!(!rails.is_empty(), "{r}: the round has rails");
        for (e, f, w) in rails {
            let (p, q) = after.edge_endpoints(e).unwrap();
            assert_eq!(
                swept_rail_reading(&after, f, &after.faces()[w], e, [p, q], None),
                Ok(Some(EdgeReads::Tangent)),
                "{r}: rail {e:?}"
            );
            assert_eq!(
                after.edge_smoothness(e),
                kernel::EdgeSmoothness::Tangent,
                "{r}: rail {e:?}"
            );
        }
    }
}

/// A 12-sided boss on a 60 mm square plate, its top ring smaller and turned
/// half a side, so its walls are triangles; and the edges of its foot.
fn twisted_boss() -> (kernel::Object, Vec<kernel::ids::EdgeId>) {
    use kernel::loft::{LoftProfile, LoftSpec};
    let ring = |r: f64, turn: f64, z: f64| LoftProfile {
        points: (0..12)
            .map(|i| {
                let a = std::f64::consts::TAU * (i as f64 + turn) / 12.0;
                Point3::new(r * trig::cos(a), r * trig::sin(a), z)
            })
            .collect(),
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let boss = kernel::Object::from_loft(&LoftSpec {
        profiles: vec![ring(0.02, 0.0, 0.0), ring(0.016, 0.5, 0.012)],
        apex: None,
        rails: Vec::new(),
        stations: None,
    })
    .unwrap();
    let s = 0.03;
    let sq = LoftProfile {
        points: vec![
            Point3::new(-s, -s, -0.004),
            Point3::new(s, -s, -0.004),
            Point3::new(s, s, -0.004),
            Point3::new(-s, s, -0.004),
        ],
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let mut top = sq.clone();
    for p in &mut top.points {
        p.z = 0.004;
    }
    let plate = kernel::Object::from_loft(&LoftSpec {
        profiles: vec![sq, top],
        apex: None,
        rails: Vec::new(),
        stations: None,
    })
    .unwrap();
    let joined = kernel::Object::boolean(
        kernel::BooleanOp::Union,
        &plate,
        &boss,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let seam = joined
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = joined.edge_endpoints(e).unwrap();
            [p, q]
                .iter()
                .all(|r| (r.z - 0.004).abs() < 1e-9 && trig::hypot(r.x, r.y) < 0.0205)
        })
        .collect();
    (joined, seam)
}

fn round_spec(edges: Vec<kernel::ids::EdgeId>, r: f64) -> kernel::BlendSpec {
    kernel::BlendSpec {
        edges,
        profile: kernel::BlendProfile::Round {
            radius: r,
            segments: 4,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

/// The rails of `obj`: (edge, strip facet, wall face).
fn rails_of(
    obj: &kernel::Object,
) -> Vec<(
    kernel::ids::EdgeId,
    kernel::ids::FaceId,
    kernel::ids::FaceId,
)> {
    let face_of = |h: kernel::ids::HalfEdgeId| obj.loops()[obj.half_edges()[h].loop_id].face;
    obj.edges()
        .iter()
        .filter_map(|(e, ed)| {
            let (a, b) = (face_of(ed.half_edge), face_of(ed.twin_half_edge?));
            let strip = |f: kernel::ids::FaceId| {
                obj.faces()[f]
                    .blend
                    .is_some_and(|t| t.role == kernel::topo::BlendRole::Strip)
            };
            if strip(a) && obj.faces()[b].blend.is_none() {
                Some((e, a, b))
            } else if strip(b) && obj.faces()[a].blend.is_none() {
                Some((e, b, a))
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn a_strip_of_triangles_reads_its_rails_through_the_next_facet() {
    // On a boss whose walls are triangles the walk splits the strip's
    // quads, so a facet on the rail is a triangle whose every edge touches
    // the rail: its turn is read one facet further in, and every rail
    // reads tangent.
    let (joined, seam) = twisted_boss();
    assert_eq!(seam.len(), 24);
    for r in [0.0005, 0.001, 0.002] {
        let after = joined
            .blend_swept_result_for_test(&round_spec(seam.clone(), r))
            .unwrap_or_else(|| panic!("{r}: the swept round refused"));
        assert_eq!(check_edge_classes(&after, None), None, "{r}");
        let rails = rails_of(&after);
        let mut hops = 0;
        for &(e, f, w) in &rails {
            let n = after.faces()[f].plane.normal();
            let (_, hop, _) = rail_turn(&after, f, n, e).expect("a turn to judge by");
            hops += usize::from(hop);
            let (p, q) = after.edge_endpoints(e).unwrap();
            assert_eq!(
                swept_rail_reading(&after, f, &after.faces()[w], e, [p, q], None),
                Ok(Some(EdgeReads::Tangent)),
                "{r}: rail {e:?}"
            );
        }
        assert!(hops > 0, "{r}: no rail took the hop");
    }
}

#[test]
fn a_strip_facet_tilted_off_its_rail_fails() {
    // The same strip with one triangle on the rail turned most of the way
    // toward the facet beyond it: its rail parts from the wall by more than
    // the strip turns there, and the check fails it.
    let (joined, seam) = twisted_boss();
    let after = joined
        .blend_swept_result_for_test(&round_spec(seam, 0.001))
        .expect("the swept round builds");
    let (e, f, w, m) = rails_of(&after)
        .into_iter()
        .find_map(|(e, f, w)| {
            let n = after.faces()[f].plane.normal();
            match rail_turn(&after, f, n, e) {
                Some((_, true, m)) => Some((e, f, w, m)),
                _ => None,
            }
        })
        .expect("a rail that took the hop");
    let n = after.faces()[f].plane.normal();
    let tilted = (n + (m - n) * 0.75).normalized().unwrap();
    let (p, q) = after.edge_endpoints(e).unwrap();
    let wall = &after.faces()[w];
    assert_eq!(
        rail_reading(&after, f, n, wall, e, [p, q], None),
        Ok(Some(EdgeReads::Tangent))
    );
    let got = rail_reading(&after, f, tilted, wall, e, [p, q], None);
    assert!(
        got.as_ref()
            .is_err_and(|why| why.starts_with("strip facet neither")),
        "{got:?}"
    );
}

/// A boss of the `xy` loop (millimeters) on a 60 mm square plate 4 mm
/// thick, standing 5 mm above it, its top ring `top_scale` times the size
/// of its foot (1 for upright walls, less for walls leaning in); and the
/// edges of its foot.
fn boss_on_plate(
    loop_mm: &[(f64, f64)],
    top_scale: f64,
) -> (kernel::Object, Vec<kernel::ids::EdgeId>) {
    use kernel::loft::{LoftProfile, LoftSpec};
    let ring = |pts: &[(f64, f64)], z: f64, k: f64| LoftProfile {
        points: pts
            .iter()
            .map(|&(x, y)| Point3::new(x * k, y * k, z))
            .collect(),
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let loft = |a: LoftProfile, b: LoftProfile| {
        kernel::Object::from_loft(&LoftSpec {
            profiles: vec![a, b],
            apex: None,
            rails: Vec::new(),
            stations: None,
        })
        .unwrap()
    };
    let s = 0.03;
    let square = [(-s, -s), (s, -s), (s, s), (-s, s)];
    let plate = loft(ring(&square, 0.0, 1.0), ring(&square, 0.004, 1.0));
    let foot: Vec<(f64, f64)> = loop_mm.iter().map(|&(x, y)| (x * 1e-3, y * 1e-3)).collect();
    // From 2 mm into the plate to 5 mm above it, the size of the loop where
    // it meets the plate's top.
    let (z0, zt) = (0.002, 0.009);
    let k0 = 1.0 + (1.0 - top_scale) * (0.004 - z0) / (zt - 0.004);
    let boss = loft(ring(&foot, z0, k0), ring(&foot, zt, top_scale));
    let joined = kernel::Object::boolean(
        kernel::BooleanOp::Union,
        &plate,
        &boss,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let seam: Vec<_> = joined
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = joined.edge_endpoints(e).unwrap();
            [p, q]
                .iter()
                .all(|r| (r.z - 0.004).abs() < 1e-9 && r.x.abs() < 0.0299 && r.y.abs() < 0.0299)
        })
        .collect();
    assert_eq!(seam.len(), loop_mm.len(), "the seam is the boss's foot");
    (joined, seam)
}

/// Asserts that the swept round of `seam` at each of `sizes` builds and
/// every edge of it reads as its geometry says.
fn rounds_read_clean(obj: &kernel::Object, seam: &[kernel::ids::EdgeId], sizes: &[f64]) {
    for &r in sizes {
        let spec = kernel::BlendSpec {
            edges: seam.to_vec(),
            profile: kernel::BlendProfile::Round {
                radius: r,
                segments: 6,
            },
            primary: None,
            corners: kernel::CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        };
        let after = obj
            .blend_swept_result_for_test(&spec)
            .unwrap_or_else(|| panic!("{r}: the swept round refused"));
        assert_eq!(check_edge_classes(&after, Some(obj)), None, "{r}");
    }
}

/// Where a swept round of a boss whose walls lean in refuses: the boss's
/// corner at (-10, 12) mm, on the plate, the first corner where the ball's
/// contact would slide down the corner's edge and back up as it turns about
/// it, folding the strip onto itself along the edge.
const FOLDING_CORNER: (f64, f64, f64) = (-10.0, 12.0, 4.0);

/// Asserts that the swept round of `seam` at `r` refuses with
/// `UnsupportedVertex` at the corner where it would fold, `at` (mm).
fn refuses_at_the_fold(
    what: &str,
    obj: &kernel::Object,
    seam: &[kernel::ids::EdgeId],
    r: f64,
    at: (f64, f64, f64),
) {
    match obj.blend_swept_refusal_for_test(&round_of(seam, r)) {
        Some(kernel::BlendError::UnsupportedVertex { at: p }) => assert!(
            (p - Point3::new(at.0 * 1e-3, at.1 * 1e-3, at.2 * 1e-3)).length() <= 1e-9,
            "{what} {r}: refused at {p:?}"
        ),
        other => panic!("{what} {r}: {other:?}, not a refusal at the folding corner"),
    }
}

/// A boss with a notch: a convex corner at (0, 12) mm, a short wall to a
/// valley at (-0.3, 11.6) mm, and the next wall on.
const NOTCH: [(f64, f64); 6] = [
    (-10.0, -10.0),
    (10.0, -10.0),
    (10.0, 10.0),
    (0.0, 12.0),
    (-0.3, 11.6),
    (-10.0, 12.0),
];

#[test]
fn a_swept_round_past_a_notch_reads_every_edge_as_its_geometry_says() {
    // Rolling round the notch's corner the ball steps from the corner's
    // edge to the wall past the valley at one chain point, through the
    // corner ball touching both; the rail crosses the valley's seam there,
    // and the strip beside each part of it leaves along the section of the
    // ball's contact on that wall.
    let (obj, seam) = boss_on_plate(&NOTCH, 1.0);
    rounds_read_clean(&obj, &seam, &[0.0005, 0.001, 0.0015]);
}

#[test]
fn a_swept_round_past_an_upright_valley_reads_every_edge_as_its_geometry_says() {
    // A corner at (0, 12) mm and a valley 2.8 mm on at (-2, 10) mm, the
    // walls upright: the ball turns about the corners' edges in place and
    // sits in the valley, and every edge reads as its geometry says.
    let valley = [
        (-10.0, -10.0),
        (10.0, -10.0),
        (10.0, 10.0),
        (0.0, 12.0),
        (-2.0, 10.0),
        (-10.0, 12.0),
    ];
    let (obj, seam) = boss_on_plate(&valley, 1.0);
    rounds_read_clean(&obj, &seam, &[0.0005, 0.001, 0.0015]);
}

#[test]
fn a_swept_round_past_a_leaning_notch_is_refused_where_it_would_fold() {
    // Walls leaning in: turning about a corner's edge the ball's contact
    // slides down the edge and back up, and the strip would fold onto
    // itself there. The round is refused, naming the corner, and the object
    // is left as it was.
    let (obj, seam) = boss_on_plate(&NOTCH, 0.85);
    let before = obj.clone();
    refuses_at_the_fold("leaning notch", &obj, &seam, 0.0005, FOLDING_CORNER);
    assert!(kernel::same_geometry_for_test(&before, &obj, 0.0));
}

#[test]
fn a_swept_round_past_a_leaning_valley_is_refused_where_it_would_fold() {
    // A corner at (0, 12) mm and a valley 2.8 mm on at (-2, 10) mm, the
    // walls leaning in: refused where the strip would fold, as past the
    // notch, and the object left as it was.
    let valley = [
        (-10.0, -10.0),
        (10.0, -10.0),
        (10.0, 10.0),
        (0.0, 12.0),
        (-2.0, 10.0),
        (-10.0, 12.0),
    ];
    let (obj, seam) = boss_on_plate(&valley, 0.85);
    let before = obj.clone();
    refuses_at_the_fold("leaning valley", &obj, &seam, 0.0005, FOLDING_CORNER);
    assert!(kernel::same_geometry_for_test(&before, &obj, 0.0));
}

#[test]
fn the_faceted_plinths_named_limit_builds_and_reads_correct() {
    // A refusal names a largest size: that size must build, and the result
    // must pass every check a built row passes.
    let (name, model) = load_models()
        .into_iter()
        .find(|(name, _)| name == "faceted-plinth-seam")
        .expect("the faceted plinth model");
    let case = model
        .cases
        .iter()
        .find(|c| c.id == "seam-loop-round")
        .expect("the loop case");
    let mut session = Session::build(&model, &name);
    let (class, detail, max) = run_case(&mut session, &name, case, "radius", 0.0025);
    assert_eq!(class, Class::Limit, "{detail}");
    let max = max.expect("the refusal names a largest size");
    assert!(max > 0.0017 && max < 0.0025, "{max}");
    let (class, detail, _) = run_case(&mut session, &name, case, "radius", max);
    assert_eq!(class, Class::Correct, "{max}: {detail}");
}

#[test]
fn the_faceted_plinths_named_limit_is_the_same_whatever_size_is_asked() {
    // Asked just above the limit or far past it, the refusal names one
    // largest size, within 1%, limited by the wall; and that size grown by
    // 1% does not build. Asked just above the limit, the swept attempt
    // finds the size too large for the wall (its rail reaches the wall's
    // hard rim) rather than refusing the corner outright, so the search
    // is the same as from far above.
    let (name, model) = load_models()
        .into_iter()
        .find(|(name, _)| name == "faceted-plinth-seam")
        .expect("the faceted plinth model");
    let case = model
        .cases
        .iter()
        .find(|c| c.id == "seam-loop-round")
        .expect("the loop case");
    let mut session = Session::build(&model, &name);
    let named = |session: &mut Session, asked: f64| {
        let (class, detail, max) = run_case(session, &name, case, "radius", asked);
        assert_eq!(class, Class::Limit, "{asked}: {detail}");
        assert!(detail.ends_with("by=adjacent_face"), "{asked}: {detail}");
        max.expect("a largest size")
    };
    let limit = named(&mut session, 0.0025);
    for asked in [limit * 1.005, limit * 1.02, 0.002, 0.004, 0.01] {
        let max = named(&mut session, asked);
        assert!(
            (max - limit).abs() <= 0.01 * limit,
            "{asked}: {max} against {limit}"
        );
    }
    let (class, detail, _) = run_case(&mut session, &name, case, "radius", limit * 1.01);
    assert_eq!(class, Class::Limit, "{}: {detail}", limit * 1.01);
    assert!(
        detail.starts_with("radius_too_large") && detail.ends_with("by=adjacent_face"),
        "{}: {detail}",
        limit * 1.01
    );
}

/// Where a round of a boss whose walls lean in is refused: the corner the
/// plain blend cannot build, at (10, -10) mm on the plate, which answers
/// once the swept blend finds no size that rolls past its corners.
const LEANING_CORNER: (f64, f64, f64) = (10.0, -10.0, 4.0);

/// Asserts that rounding `seam` of `obj` at `r` is refused with
/// `UnsupportedVertex` at `at` (mm), naming no size.
fn refused_at_corner(
    what: &str,
    obj: &kernel::Object,
    seam: &[kernel::ids::EdgeId],
    r: f64,
    at: (f64, f64, f64),
) {
    match obj.blend_edges(&round_of(seam, r)) {
        Err(kernel::BlendError::UnsupportedVertex { at: p }) => assert!(
            (p - Point3::new(at.0 * 1e-3, at.1 * 1e-3, at.2 * 1e-3)).length() <= 1e-9,
            "{what} {r}: refused at {p:?}"
        ),
        other => panic!(
            "{what} {r}: {:?}, not the corner refusal",
            other.map(|_| ())
        ),
    }
}

#[test]
fn a_leaning_notch_bosss_round_is_refused_at_its_corner_whatever_size_is_asked() {
    // A drawn wall 5 mm high leaning in: past what a round of its base can
    // turn at the corners, every size asked is refused at a corner and none
    // is named as the largest that fits.
    // Isolated sizes within 1 nm of 4.6035 mm and 6.3571 mm build; the
    // refusal is not continuous there.
    let (obj, seam) = boss_on_plate(&NOTCH, 0.85);
    for r in [
        0.0005,
        0.003,
        0.004,
        0.007,
        0.0073741221427917475,
        0.008,
        0.01,
        0.02,
    ] {
        refused_at_corner("leaning notch", &obj, &seam, r, LEANING_CORNER);
    }
}

#[test]
fn a_plain_leaning_bosss_round_is_refused_at_its_corner() {
    // A square boss with its walls leaning in, no notch: its base round is
    // refused at a corner at every size, the size past the wall's height
    // (7.2 mm) too, since no smaller size turns its corners either; with
    // its walls all but upright (a lean of 0.23°) the round builds, its
    // plate rail where a ball on the plate and the wall puts it.
    let square = [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)];
    let (obj, seam) = boss_on_plate(&square, 0.85);
    // Also just past 7.016 mm, where a ball on the plate and the wall first
    // touches the wall above its top edge.
    for r in [
        0.0005, 0.001, 0.003, 0.0070165, 0.007017, 0.0070174, 0.007018, 0.00702, 0.0072, 0.01,
    ] {
        refused_at_corner("plain leaning boss", &obj, &seam, r, LEANING_CORNER);
    }
    let (obj, seam) = boss_on_plate(&square, 0.998);
    let r = 0.0005;
    let (after, _) = obj
        .blend_edges(&round_of(&seam, r))
        .expect("an all but upright boss rounds at 0.5 mm");
    let off = rails_off_their_balls(&obj, &after, r, &square, 4.0 * r);
    assert!(off.is_empty(), "{off:#?}");
}

#[test]
fn a_profile_other_than_a_round_keeps_its_refusal_past_a_leaning_notch() {
    // Only a round's refusal names the largest size that builds where the
    // swept blend finds it too large for a side; a profile or a chamfer the
    // swept blend cannot make keeps the refusal it always had (here the
    // corner at (10, -10) mm the catalogue cannot build), with no search.
    let (obj, seam) = boss_on_plate(&NOTCH, 0.85);
    let spec = |profile: kernel::BlendProfile| kernel::BlendSpec {
        edges: seam.clone(),
        profile,
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    };
    for profile in [
        kernel::BlendProfile::Custom {
            shape: (kernel::bundled_profiles()[0].2)(),
            width: 0.05,
            arc_segments: 4,
        },
        kernel::BlendProfile::Chamfer {
            setback_a: 0.02,
            setback_b: 0.02,
        },
    ] {
        let searches = kernel::limit_searches_for_test();
        let got = obj.blend_edges(&spec(profile.clone()));
        assert!(
            matches!(got, Err(kernel::BlendError::UnsupportedVertex { .. })),
            "{profile:?}: {:?}",
            got.map(|_| ())
        );
        assert_eq!(kernel::limit_searches_for_test(), searches, "{profile:?}");
    }
}

/// Rounds the plinth's seam at `size` through the API, with the facet
/// walk's rails checked against their own contacts: what the check found,
/// and how deep the strip lies inside the solid it was cut from.
fn plinth_round_checked(size: f64) -> Option<(Vec<String>, f64)> {
    let (name, model) = load_models()
        .into_iter()
        .find(|(name, _)| name == "faceted-plinth-seam")
        .expect("the faceted plinth model");
    let case = model
        .cases
        .iter()
        .find(|c| c.id == "seam-loop-round")
        .expect("the loop case");
    let mut s = Session::build(&model, &name);
    let mut params = substitute(&case.params, &s.labels).expect("the case's params");
    params
        .as_object_mut()
        .expect("params")
        .insert("radius".to_string(), json!(size));
    let before: BTreeMap<ObjectId, kernel::Object> = s
        .doc
        .live_object_ids()
        .into_iter()
        .filter_map(|id| s.doc.object(id).map(|o| (id, o.clone())))
        .collect();
    let (called, found) = kernel::swept_rail_check_for_test(|| s.call(&case.method, params));
    let Called::Ok(result) = called else {
        return None;
    };
    let id = result["object_id"]
        .as_str()
        .and_then(|p| s.object(p))
        .expect("the result's object");
    let after = s.doc.object(id).expect("the rounded object");
    let (depth, _) = before[&id].blend_strip_penetration_for_test(after, size);
    Some((found, depth))
}

#[test]
fn every_swept_rail_of_the_faceted_plinth_is_its_own_contact_and_clears_the_solid() {
    // Near the largest size the wall takes, the ball rides the plinth's
    // seams beside its top: every rail point the strip is built on is that
    // station's own contact on its foot, and no point of the strip lies
    // inside the solid it was cut from.
    let mut failed = Vec::new();
    for size in [0.00172, 0.00175, 0.00179, 0.0018] {
        let Some((found, depth)) = plinth_round_checked(size) else {
            failed.push(format!("{size}: the round did not build"));
            continue;
        };
        if !found.is_empty() {
            failed.push(format!("{size}: {found:#?}"));
        }
        if depth > kernel::tol::SWEPT_SLACK {
            failed.push(format!("{size}: strip {depth} m inside"));
        }
    }
    // Just past the named limit (1.8046 to 1.8048 mm), where the ride's
    // rails once left the solid: built or refused, never inside it.
    if let Some((found, depth)) = plinth_round_checked(0.0018047) {
        if !found.is_empty() {
            failed.push(format!("0.0018047: {found:#?}"));
        }
        if depth > kernel::tol::SWEPT_SLACK {
            failed.push(format!("0.0018047: strip {depth} m inside"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn every_swept_rail_past_a_leaning_notch_or_valley_is_refused_where_it_would_fold() {
    // Past a leaning notch or valley the round is refused at the corner
    // where its strip would fold (see the specs above), so there is no rail
    // to check; a round that builds there fails this spec until the fold is
    // built and the rails can be read.
    let valley = [
        (-10.0, -10.0),
        (10.0, -10.0),
        (10.0, 10.0),
        (0.0, 12.0),
        (-2.0, 10.0),
        (-10.0, 12.0),
    ];
    for (what, lp) in [("notch", &NOTCH[..]), ("valley", &valley[..])] {
        let (obj, seam) = boss_on_plate(lp, 0.85);
        refuses_at_the_fold(what, &obj, &seam, 0.0005, FOLDING_CORNER);
    }
}

/// The faceted plinth's 1.8 mm round, its rail along one of the plinth's
/// seams a crease, and the solid it was cut from.
fn seam_ride_round() -> (kernel::Object, kernel::Object) {
    let (obj, after, _) = seam_ride_round_recorded();
    (obj, after)
}

/// [`seam_ride_round`], with the record of the build.
fn seam_ride_round_recorded() -> (kernel::Object, kernel::Object, kernel::StationRecord) {
    let (name, model) = load_models()
        .into_iter()
        .find(|(name, _)| name == "faceted-plinth-seam")
        .expect("the faceted plinth model");
    let case = model
        .cases
        .iter()
        .find(|c| c.id == "seam-loop-round")
        .expect("the loop case");
    let mut s = Session::build(&model, &name);
    let mut params = substitute(&case.params, &s.labels).expect("the case's params");
    params
        .as_object_mut()
        .expect("params")
        .insert("radius".to_string(), json!(0.0018));
    let before: Vec<(ObjectId, kernel::Object)> = s
        .doc
        .live_object_ids()
        .into_iter()
        .filter_map(|id| s.doc.object(id).map(|o| (id, o.clone())))
        .collect();
    let (called, record) = kernel::swept_balls_for_test(|| s.call(&case.method, params));
    let Called::Ok(result) = called else {
        panic!("the plinth rounds at 1.8 mm");
    };
    let id = result["object_id"]
        .as_str()
        .and_then(|p| s.object(p))
        .expect("the result's object");
    let obj = before
        .into_iter()
        .find(|(b, _)| *b == id)
        .expect("the solid it was cut from")
        .1;
    (
        obj,
        s.doc.object(id).expect("the rounded object").clone(),
        record,
    )
}

#[test]
fn a_hard_rail_on_no_seam_of_the_solid_it_was_cut_from_is_wrong() {
    // The same result read against a solid with no edge where its creased
    // rail runs: that rail is a crease of nothing, and reads wrong.
    let (obj, after) = seam_ride_round();
    assert_eq!(check_edge_classes(&after, Some(&obj)), None);
    let (elsewhere, _) = boss_on_plate(
        &[(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)],
        0.85,
    );
    let why = check_edge_classes(&after, Some(&elsewhere)).expect("the creased rail reads wrong");
    assert!(why.contains("neither on its rail's tangent"), "{why}");
}

#[test]
fn a_seam_ride_rail_whose_strip_leaves_outside_the_seams_fan_is_wrong() {
    // A seam of the plinth carries the rail; the strip facet's own normal
    // lies in the seam's fan, its reverse does not.
    let (obj, after) = seam_ride_round();
    let (rail, f) = after
        .edges()
        .keys()
        .filter(|&e| {
            after.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
                && expected_reading(&after, e, None).is_err()
        })
        .find_map(|e| {
            let edge = &after.edges()[e];
            let face_of =
                |h: kernel::ids::HalfEdgeId| after.loops()[after.half_edges()[h].loop_id].face;
            let (fa, fb) = (face_of(edge.half_edge), face_of(edge.twin_half_edge?));
            let strip = |f: kernel::ids::FaceId| {
                after.faces()[f]
                    .blend
                    .is_some_and(|t| t.role == kernel::topo::BlendRole::Strip)
            };
            match (strip(fa), strip(fb)) {
                (true, false) if after.faces()[fb].blend.is_none() => Some((e, fa)),
                (false, true) if after.faces()[fa].blend.is_none() => Some((e, fb)),
                _ => None,
            }
        })
        .expect("the creased rail");
    let (p, q) = after.edge_endpoints(rail).expect("its ends");
    let n = after.faces()[f].plane.normal();
    let (t, _, _) = rail_turn(&after, f, n, rail).expect("the strip's turn");
    assert!(rides_a_seam(&obj, [p, q], n, t));
    assert!(!rides_a_seam(&obj, [p, q], -n, t));
    let wall = &after.faces()[across_rail(&after, f, rail).expect("the wall")];
    // Read as the scoreboard reads it, with the solid the round was cut
    // from: the strip's own normal is the seam's crease; the strip flipped
    // reads wrong.
    assert_eq!(
        rail_reading(&after, f, n, wall, rail, [p, q], Some(&obj)),
        Ok(Some(EdgeReads::Hard))
    );
    let read = rail_reading(&after, f, -n, wall, rail, [p, q], Some(&obj));
    assert!(read.is_err(), "{read:?}");
}

#[test]
fn a_creased_seam_ride_rail_the_kernel_calls_tangent_is_wrong() {
    // Where the geometry says the rail is the seam's crease, a kernel that
    // reads it tangent fails the row, whatever else holds.
    let (obj, after) = seam_ride_round();
    let rail = after
        .edges()
        .keys()
        .find(|&e| {
            after.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
                && expected_reading(&after, e, None).is_err()
        })
        .expect("a rail only the seam reading explains");
    assert_eq!(
        expected_reading(&after, rail, Some(&obj)),
        Ok(Some(EdgeReads::Hard))
    );
    assert_eq!(
        reading_problem(
            EdgeReads::Tangent,
            expected_reading(&after, rail, Some(&obj))
        ),
        Some("Tangent where Hard is right".to_string())
    );
}

/// Whether `p`, in the plane of a polygon with normal `n`, lies inside the
/// polygon `ring` (even-odd, in the plane's own 2D frame).
fn in_ring(ring: &[Point3], p: Point3, n: kernel::Vec3) -> bool {
    let helper = if n.x.abs() < 0.9 {
        kernel::Vec3::new(1.0, 0.0, 0.0)
    } else {
        kernel::Vec3::new(0.0, 1.0, 0.0)
    };
    let u = n.cross(helper).normalized().expect("a frame");
    let v = n.cross(u);
    let flat = |x: Point3| ((x - p).dot(u), (x - p).dot(v));
    let k = ring.len();
    let mut inside = false;
    for i in 0..k {
        let (ax, ay) = flat(ring[i]);
        let (bx, by) = flat(ring[(i + 1) % k]);
        if (ay > 0.0) != (by > 0.0) && ax + (0.0 - ay) * (bx - ax) / (by - ay) > 0.0 {
            inside = !inside;
        }
    }
    inside
}

/// The distance from `x` to the nearest point of `obj`'s surface.
fn surface_distance(obj: &kernel::Object, x: Point3) -> f64 {
    let segment = |a: Point3, b: Point3| {
        let d = b - a;
        let l2 = d.dot(d);
        let t = if l2 > 0.0 {
            ((x - a).dot(d) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (x - (a + d * t)).length()
    };
    obj.faces()
        .iter()
        .map(|(_, face)| {
            let n = face.plane.normal();
            let sd = face.plane.signed_distance(x);
            let foot = x - n * sd;
            let rings: Vec<Vec<Point3>> = std::iter::once(face.outer_loop)
                .chain(face.inner_loops.iter().copied())
                .map(|l| obj.loop_positions(l).collect())
                .collect();
            let inside =
                in_ring(&rings[0], foot, n) && !rings[1..].iter().any(|h| in_ring(h, foot, n));
            if inside {
                sd.abs()
            } else {
                rings
                    .iter()
                    .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
                    .map(|(a, b)| segment(a, b))
                    .fold(f64::INFINITY, f64::min)
            }
        })
        .fold(f64::INFINITY, f64::min)
}

/// Where a round's rails do not rest the ball on both of its sides: for
/// every rail of `after` (a strip facet against a face of no blend), points
/// along it farther than `clear` (across the plate) from every one of
/// `corners`, each with its ball's center `r` out along the face's normal;
/// that center must lie `r` from the surface of `before`, the solid the
/// round was cut from, to [`kernel::tol::SWEPT_SLACK`].
fn rails_off_their_balls(
    before: &kernel::Object,
    after: &kernel::Object,
    r: f64,
    corners: &[(f64, f64)],
    clear: f64,
) -> Vec<String> {
    let face_of = |h: kernel::ids::HalfEdgeId| after.loops()[after.half_edges()[h].loop_id].face;
    let strip = |f: kernel::ids::FaceId| {
        after.faces()[f]
            .blend
            .is_some_and(|t| t.role == kernel::topo::BlendRole::Strip)
    };
    let mut off = Vec::new();
    for (e, edge) in after.edges() {
        let Some(twin) = edge.twin_half_edge else {
            continue;
        };
        let (fa, fb) = (face_of(edge.half_edge), face_of(twin));
        let wall = match (strip(fa), strip(fb)) {
            (true, false) if after.faces()[fb].blend.is_none() => fb,
            (false, true) if after.faces()[fa].blend.is_none() => fa,
            _ => continue,
        };
        let (p, q) = after.edge_endpoints(e).expect("its ends");
        let n = after.faces()[wall].plane.normal();
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let x = p + (q - p) * t;
            let near = corners.iter().any(|&(cx, cy)| {
                let (dx, dy) = (x.x - cx * 1e-3, x.y - cy * 1e-3);
                (dx * dx + dy * dy).sqrt() <= clear
            });
            if near {
                continue;
            }
            let d = surface_distance(before, x + n * r);
            if (d - r).abs() > kernel::tol::SWEPT_SLACK {
                off.push(format!(
                    "rail at ({:.3}, {:.3}, {:.3}) mm: its ball's center lies {:.4} mm from the solid",
                    x.x * 1e3,
                    x.y * 1e3,
                    x.z * 1e3,
                    d * 1e3
                ));
            }
        }
    }
    off
}

/// The leaning and upright valley bosses' and the leaning notch boss's
/// loops, with how much their tops shrink.
/// A boss fixture: its name, its loop (mm) and how much its top shrinks.
type WallFixture = (&'static str, Vec<(f64, f64)>, f64);

fn wall_fixtures() -> Vec<WallFixture> {
    let valley = vec![
        (-10.0, -10.0),
        (10.0, -10.0),
        (10.0, 10.0),
        (0.0, 12.0),
        (-2.0, 10.0),
        (-10.0, 12.0),
    ];
    vec![
        ("upright valley", valley.clone(), 1.0),
        ("leaning valley", valley, 0.85),
        ("leaning notch", NOTCH.to_vec(), 0.85),
    ]
}

fn round_of(seam: &[kernel::ids::EdgeId], r: f64) -> kernel::BlendSpec {
    kernel::BlendSpec {
        edges: seam.to_vec(),
        profile: kernel::BlendProfile::Round {
            radius: r,
            segments: 6,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

#[test]
fn a_swept_round_along_a_straight_boss_wall_rests_its_ball_on_the_wall_and_the_plate() {
    // Between its corners a straight wall carries the ball resting on the
    // wall and the plate at once: every rail point there has its ball's
    // center a radius from the solid (the plate rail a radius out from the
    // wall, the wall rail a radius up from the plate where the wall is
    // upright). Within four radii of a corner of the boss the ball turns
    // about the corner or sits in a valley and is not read.
    // The leaning bosses' rounds are refused where they would fold.
    let mut failed = Vec::new();
    for (what, lp, scale) in wall_fixtures() {
        let (obj, seam) = boss_on_plate(&lp, scale);
        for r in [0.0005, 0.001, 0.0015] {
            if scale < 1.0 {
                refuses_at_the_fold(what, &obj, &seam, r, FOLDING_CORNER);
                continue;
            }
            let after = obj
                .blend_swept_result_for_test(&round_of(&seam, r))
                .unwrap_or_else(|| panic!("{what} {r}: the round refused"));
            let off = rails_off_their_balls(&obj, &after, r, &lp, 4.0 * r);
            if !off.is_empty() {
                failed.push(format!(
                    "{what} {r}: {} points, first {}",
                    off.len(),
                    off[0]
                ));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

/// How far each strip sample of `after` (every strip facet's edge midpoints
/// and centroid) lies from the envelope of the walk's balls, in sagittas of
/// the strip's own facets: the distance to the nearest point of the path of
/// ball centers (straight between consecutive stations, the chain closed),
/// less the radius there, over `r·(1 − cos(turn/2))`. The balls are the last
/// round's of `record`. Between two stations the envelope is the balls'
/// linear blend, so a turn about a seam spanned by two stations is read
/// against its chord, not its arc. Each facet is read against its own balls
/// only (those one of its corners lies on), and the stretch between two of
/// them that are neighbors. A
/// facet with a corner at a bridge's
/// rail point (the fill under a ball that bridges a valley seam) is left
/// out: it closes the solid under the ball, off the ball by design.
fn envelope_sagittas(after: &kernel::Object, record: &kernel::StationRecord) -> Vec<(f64, Point3)> {
    let cs = record.balls.last().expect("a round's balls");
    let m = cs.len();
    let mut devs = Vec::new();
    for (_, face) in after.faces() {
        if !face
            .blend
            .is_some_and(|t| t.role == kernel::topo::BlendRole::Strip)
        {
            continue;
        }
        let ring: Vec<Point3> = after.loop_positions(face.outer_loop).collect();
        let bridge = ring.iter().any(|v| {
            record
                .bridges
                .iter()
                .any(|(b, _)| (*v - *b).length() <= kernel::tol::SWEPT_SLACK)
        });
        if bridge {
            continue;
        }
        let k = ring.len();
        let mut samples: Vec<Point3> = (0..k)
            .map(|i| ring[i] + (ring[(i + 1) % k] - ring[i]) * 0.5)
            .collect();
        let o = Point3::new(0.0, 0.0, 0.0);
        let g = ring
            .iter()
            .fold(kernel::Vec3::new(0.0, 0.0, 0.0), |a, p| a + (*p - o))
            / k as f64;
        samples.push(o + g);
        // The facet's own balls: those a corner of it lies on (a section's
        // points lie on its ball). The envelope is read between them only,
        // so a sample near where the chain turns is not read against a ball
        // across the turn.
        let owns = |i: usize| {
            let (c, r, _) = cs[i];
            ring.iter()
                .any(|v| ((*v - c).length() - r).abs() <= kernel::tol::SWEPT_SLACK)
        };
        // A stretch is a pair of neighboring balls the facet both lies on,
        // or one ball alone.
        let mut segs: Vec<(usize, usize)> = Vec::new();
        for i in 0..m {
            if owns(i) {
                segs.push((i, i));
                if owns((i + 1) % m) {
                    segs.push((i, (i + 1) % m));
                }
            }
        }
        if segs.is_empty() {
            segs = (0..m).map(|i| (i, (i + 1) % m)).collect();
        }
        for x in samples {
            let mut best = (f64::INFINITY, 0.0, 0.0);
            for &(i, j) in &segs {
                let (pa, ra, ta) = cs[i];
                let (pb, rb, tb) = cs[j];
                let d = pb - pa;
                let l2 = d.dot(d);
                let t = if l2 > 0.0 {
                    ((x - pa).dot(d) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let dist = (x - (pa + d * t)).length();
                if dist < best.0 {
                    best = (dist, ra + (rb - ra) * t, ta + (tb - ta) * t);
                }
            }
            let (dist, r, turn) = best;
            let sagitta = r * (1.0 - kernel::trig::cos(0.5 * turn));
            devs.push(((dist - r).abs() / sagitta, x));
        }
    }
    assert!(
        devs.iter().all(|d| d.0.is_finite()),
        "a deviation that is not a number"
    );
    devs.sort_by(|a, b| a.0.total_cmp(&b.0));
    devs
}

/// The median, 90th percentile and largest of sorted `devs`, and where the
/// largest is.
fn spread(devs: &[(f64, Point3)]) -> (f64, f64, f64, Point3) {
    let n = devs.len();
    let at = |p: f64| devs[((n as f64 - 1.0) * p).round() as usize].0;
    (at(0.5), at(0.9), devs[n - 1].0, devs[n - 1].1)
}

/// The largest median, 90th percentile and largest deviation of a round's
/// strip from the envelope of its balls, in sagittas ([`envelope_sagittas`]).
/// Measured with the bridges' fill left out: median 1.00 (the plinth) to
/// 1.01 (the upright valley), 90th percentile 1.08 to 1.65, largest 13.2
/// (the plinth at 1.5 mm, at (14.08, 13.38, 4.59) mm) and 14.1 (the upright
/// valley at every size, beside its valley). Each largest is one band of
/// the strip about one ball where the side's contact changes foot across
/// faces that share no facet: the plinth's from the contact on its x = y
/// seam to the one on the facet past the next seam, 36.9° about the ball;
/// the valley's from one wall's contact to the other's, 50.6°. That band is
/// open; the largest bound holds it. The upright valley also reads 13.4 at
/// its corner at (0, 12) mm, where the strip runs from the corner's ball
/// straight to the valley's. The bounds leave a tenth over each.
const ENVELOPE_MEDIAN_MAX: f64 = 1.11;
const ENVELOPE_P90_MAX: f64 = 1.82;
const ENVELOPE_MAX: f64 = 15.5;
/// The deepest a bridge's fill may lie past its ball, over the radius:
/// measured 0.149 (the upright valley's valley seam, 0.5745 mm from a 0.5 mm
/// ball's center) and 0.061 (the plinth's seam beside its x = y seam), with
/// a tenth over.
const BRIDGE_FILL_MAX: f64 = 0.165;

#[test]
fn a_swept_round_lies_on_the_envelope_of_its_balls() {
    // Every strip sample lies within a few of the strip's own sagittas of
    // the envelope of the balls it was built from: the bosses' straight
    // walls and corners, and the faceted plinth near its named limit.
    let mut failed = Vec::new();
    let mut judge = |what: String, after: &kernel::Object, record: &kernel::StationRecord| {
        let devs = envelope_sagittas(after, record);
        let (median, p90, max, at) = spread(&devs);
        let fill = record.bridges.iter().map(|b| b.1).fold(0.0, f64::max);
        if fill > BRIDGE_FILL_MAX {
            failed.push(format!(
                "{what}: a bridge's fill {fill:.4} of the radius deep"
            ));
        }
        if median > ENVELOPE_MEDIAN_MAX || p90 > ENVELOPE_P90_MAX || max > ENVELOPE_MAX {
            failed.push(format!(
                "{what}: median {median:.2}, p90 {p90:.2}, max {max:.2} at ({:.2}, {:.2}, {:.2}) mm",
                at.x * 1e3,
                at.y * 1e3,
                at.z * 1e3
            ));
        }
    };
    for (what, lp, scale) in wall_fixtures() {
        let (obj, seam) = boss_on_plate(&lp, scale);
        for r in [0.0005, 0.001, 0.0015] {
            // The leaning bosses' rounds are refused where they would fold.
            if scale < 1.0 {
                refuses_at_the_fold(what, &obj, &seam, r, FOLDING_CORNER);
                continue;
            }
            let (after, record) = kernel::swept_balls_for_test(|| {
                obj.blend_swept_result_for_test(&round_of(&seam, r))
            });
            let after = after.unwrap_or_else(|| panic!("{what} {r}: the round refused"));
            judge(format!("{what} {r}"), &after, &record);
        }
    }
    let (name, model) = load_models()
        .into_iter()
        .find(|(name, _)| name == "faceted-plinth-seam")
        .expect("the faceted plinth model");
    let case = model
        .cases
        .iter()
        .find(|c| c.id == "seam-loop-round")
        .expect("the loop case");
    for size in [0.0015, 0.0017, 0.0018] {
        let mut s = Session::build(&model, &name);
        let mut params = substitute(&case.params, &s.labels).expect("the case's params");
        params
            .as_object_mut()
            .expect("params")
            .insert("radius".to_string(), json!(size));
        let (called, record) = kernel::swept_balls_for_test(|| s.call(&case.method, params));
        let Called::Ok(result) = called else {
            panic!("plinth {size}: the round did not build");
        };
        let id = result["object_id"]
            .as_str()
            .and_then(|p| s.object(p))
            .expect("the result's object");
        let after = s.doc.object(id).expect("the rounded object");
        judge(format!("plinth {size}"), after, &record);
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn a_seam_ride_reading_keeps_to_convex_seams_within_their_ends_and_their_fan() {
    // A 20 mm square boss standing 5 mm on a plate: its top edges are
    // convex seams, its foot edges (where it meets the plate) concave.
    let (obj, _) = boss_on_plate(
        &[(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)],
        1.0,
    );
    let mm = |x: f64, y: f64, z: f64| Point3::new(x * 1e-3, y * 1e-3, z * 1e-3);
    let tilt = |deg: f64| {
        let a = deg.to_radians();
        // From the front wall's normal (0, -1, 0) toward the top's (0, 0, 1).
        kernel::Vec3::new(0.0, -kernel::trig::cos(a), kernel::trig::sin(a))
    };
    let t = 0.1;
    // On the front top edge, its strip normal between the wall's and the
    // top's: a ride.
    let top = [mm(-5.0, -10.0, 9.0), mm(5.0, -10.0, 9.0)];
    assert!(rides_a_seam(&obj, top, tilt(45.0), t));
    // The same along the front foot edge, a concave seam: none.
    let foot = [mm(-5.0, -10.0, 4.0), mm(5.0, -10.0, 4.0)];
    assert!(!rides_a_seam(&obj, foot, tilt(-45.0), t));
    assert!(!rides_a_seam(&obj, foot, tilt(45.0), t));
    // On the top edge's line past its end: none.
    let past = [mm(10.5, -10.0, 9.0), mm(12.0, -10.0, 9.0)];
    assert!(!rides_a_seam(&obj, past, tilt(45.0), t));
    // Beside the top edge, parallel to it and off its line by twice the
    // slack: none.
    let beside = [
        Point3::new(-0.005, -0.01 - 2.0 * ON_SEAM, 0.009),
        Point3::new(0.005, -0.01 - 2.0 * ON_SEAM, 0.009),
    ];
    assert!(!rides_a_seam(&obj, beside, tilt(45.0), t));
    // Past the wall's end of the fan by more than `t` (10°, a chord of
    // 0.174): none; by less (4°, a chord of 0.070): a ride.
    assert!(!rides_a_seam(&obj, top, tilt(-10.0), t));
    assert!(rides_a_seam(&obj, top, tilt(-4.0), t));
}

/// The angle (degrees) a chord between two unit normals spans.
fn chord_degrees(c: f64) -> f64 {
    2.0 * kernel::trig::asin(0.5 * c) * (180.0 / std::f64::consts::PI)
}

#[test]
fn a_swept_rails_turn_and_lean_are_the_rounds_own_on_a_square_boss() {
    // A square boss stands upright on a plate, so the round at its foot
    // turns a right angle; at six segments its strip turns 15° from one
    // facet to the next, and a rail tangent to its wall leans off it by half
    // that, 7.5°. Every rail here reads its turn directly (a quad on the
    // rail). Measured: 15° and 7.5° to 1e-6° along the straight walls, and
    // within 0.084° and 0.049° on the corners' turns, whose facets twist a
    // little; the bounds are 0.1° and 0.06°.
    let (obj, seam) = boss_on_plate(
        &[(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)],
        1.0,
    );
    let after = obj
        .blend_swept_result_for_test(&round_of(&seam, 0.001))
        .expect("the square boss rounds at 1 mm");
    let rails = rails_of(&after);
    assert!(!rails.is_empty());
    let mut straight = 0;
    for (e, f, w) in rails {
        let g = after
            .swept_rail_gap(f, after.faces()[f].plane.normal(), e, w)
            .expect("a turn to read");
        assert!(!g.hop, "{e:?}: a quad on the rail");
        let (turn, lean) = (chord_degrees(g.turn), chord_degrees(g.lean));
        assert!((turn - 15.0).abs() <= 0.1, "{e:?}: turns {turn}°");
        assert!((lean - 7.5).abs() <= 0.06, "{e:?}: leans {lean}°");
        if (turn - 15.0).abs() <= 1e-6 && (lean - 7.5).abs() <= 1e-6 {
            straight += 1;
        }
    }
    assert!(straight >= 8, "{straight} rails along the straight walls");
}

#[test]
fn a_swept_rails_turn_and_lean_are_the_rounds_own_through_the_hop() {
    // On the twisted boss half the rails on its wall triangles read their
    // turn through the hop (a triangle on the rail), half directly. The round between the plate
    // and a wall triangle turns as far as their normals part, so at four
    // segments the strip turns a quarter of that. Measured: the turn within
    // 0.241° of a quarter of the arc; the lean within 1.20° of half the
    // turn, the rail's triangle twisted off the section's first chord. The
    // bounds are 0.3° and 1.3°.
    let (joined, seam) = twisted_boss();
    let after = joined
        .blend_swept_result_for_test(&round_spec(seam, 0.001))
        .expect("the twisted boss rounds at 1 mm");
    let up = kernel::Vec3::new(0.0, 0.0, 1.0);
    let (mut read, mut hops) = (0, 0);
    for (e, f, w) in rails_of(&after) {
        let nw = after.faces()[w].plane.normal();
        if (nw - up).length() <= 1e-9 {
            // A rail on the plate: its wall is not this face.
            continue;
        }
        let g = after
            .swept_rail_gap(f, after.faces()[f].plane.normal(), e, w)
            .expect("a turn to read");
        hops += usize::from(g.hop);
        let arc = kernel::trig::acos(nw.dot(up).clamp(-1.0, 1.0)) * (180.0 / std::f64::consts::PI);
        let (turn, lean) = (chord_degrees(g.turn), chord_degrees(g.lean));
        assert!(
            (turn - arc / 4.0).abs() <= 0.3,
            "{e:?}: turns {turn}°, arc {arc}°"
        );
        assert!(
            (lean - turn / 2.0).abs() <= 1.3,
            "{e:?}: leans {lean}°, turns {turn}°"
        );
        read += 1;
    }
    assert!(read >= 12, "{read} rails on the walls");
    assert!(hops >= 6, "{hops} rails read through the hop");
}

#[test]
fn a_seam_rides_lean_is_its_walls_against_the_first_chord_of_its_ball() {
    // The plinth's crease along its seam: the lean read off the strip facet
    // is the angle between the wall facet's normal and the normal of the
    // section's first chord, from the ball the walk built that station on
    // (its center, the rail, the section's next point). The facet on this
    // rail is a triangle with its third corner on one end's section only, so
    // that end's chord is the one read; the triangle reaches to the other
    // end's station, where the contact has turned along the seam. Measured
    // at 1.8 mm: 18.20° off the facet, 20.68° off the chord, 2.49° apart;
    // the bound is 2.6°.
    let (_, after, record) = seam_ride_round_recorded();
    let balls = record.balls.last().expect("the walk's balls");
    let (rail, f, w) = rails_of(&after)
        .into_iter()
        .find(|&(e, _, _)| {
            after.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
                && expected_reading(&after, e, None).is_err()
        })
        .expect("the creased rail");
    let n = after.faces()[f].plane.normal();
    let g = after.swept_rail_gap(f, n, rail, w).expect("a turn to read");
    let (p, q) = after.edge_endpoints(rail).expect("its ends");
    let ring: Vec<Point3> = after.loop_positions(after.faces()[f].outer_loop).collect();
    let k = ring.len();
    let nw = after.faces()[w].plane.normal();
    let mut leans = Vec::new();
    for v in [p, q] {
        let (c, _, _) = *balls
            .iter()
            .find(|(c, r, _)| ((v - *c).length() - r).abs() <= kernel::tol::SWEPT_SLACK)
            .expect("the ball the rail lies on");
        let i = ring
            .iter()
            .position(|x| (*x - v).length() <= kernel::tol::POINT_MERGE)
            .expect("the rail's corner");
        let other = if v == p { q } else { p };
        // The section's next point: the facet's corner beside this one,
        // not the rail's other end, on this station's own ball (a triangle
        // on the rail has it at one end only).
        let Some(next) = [ring[(i + 1) % k], ring[(i + k - 1) % k]]
            .into_iter()
            .find(|x| {
                (*x - other).length() > kernel::tol::POINT_MERGE
                    && ((*x - c).length() - balls.iter().find(|b| b.0 == c).map_or(0.0, |b| b.1))
                        .abs()
                        <= kernel::tol::SWEPT_SLACK
            })
        else {
            continue;
        };
        let mid = v + (next - v) * 0.5;
        let chord = (mid - c).normalized().expect("a chord off the center");
        let chord = if chord.dot(n) < 0.0 { -chord } else { chord };
        leans.push(chord_degrees((nw - chord).length()));
    }
    let from_ball = leans.into_iter().fold(0.0, f64::max);
    let lean = chord_degrees(g.lean);
    assert!(from_ball > 0.0, "no end's chord to read");
    assert!(
        (lean - from_ball).abs() <= 2.6,
        "leans {lean}° off the facet, {from_ball}° off the ball's chord"
    );
}

#[test]
fn a_seam_ride_reads_as_one_only_while_its_strip_stays_in_the_seams_fan() {
    // The plinth's crease along its seam, its strip normal turned about the
    // seam out past the end of the seam's fan: within the strip's own turn
    // (read inside the strip, so it does not move with the turned facet) of
    // the fan it is a ride, past it it is not.
    let (obj, after) = seam_ride_round();
    let (rail, f, _) = rails_of(&after)
        .into_iter()
        .find(|&(e, _, _)| {
            after.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
                && expected_reading(&after, e, None).is_err()
        })
        .expect("the creased rail");
    let (p, q) = after.edge_endpoints(rail).expect("its ends");
    let n = after.faces()[f].plane.normal();
    let gap = after
        .swept_rail_gap(f, n, rail, across_rail(&after, f, rail).expect("the wall"))
        .expect("a turn to read");
    let t = interior_turn(&after, f, gap.next).expect("an interior turn");
    let (n1, n2) = obj
        .edges()
        .keys()
        .find_map(|e| {
            let (a, b) = obj.edge_endpoints(e)?;
            let d = b - a;
            let on = [p, q].iter().all(|&x| {
                let s = (x - a).dot(d) / d.dot(d);
                (x - (a + d * s)).length() <= ON_SEAM
            });
            if on { seam_fan(&obj, e) } else { None }
        })
        .expect("the seam the rail rides");
    let axis = n1.cross(n2).normalized().expect("a seam with a fan");
    let rotate = |v: kernel::Vec3, a: f64| {
        v * kernel::trig::cos(a)
            + axis.cross(v) * kernel::trig::sin(a)
            + axis * axis.dot(v) * (1.0 - kernel::trig::cos(a))
    };
    // Past `n1`, away from `n2`: the way that turns `n1` off the fan.
    let away = if fan_gap(rotate(n1, -0.01), n1, n2) > 0.0 {
        -1.0
    } else {
        1.0
    };
    let line = 2.0 * kernel::trig::asin(0.5 * t);
    for (k, rides) in [(0.5, true), (1.5, false)] {
        let v = rotate(n1, away * k * line);
        assert_eq!(
            rides_a_seam(&obj, [p, q], v, t),
            rides,
            "{k} of the strip's turn past the fan (a chord of {} against {t})",
            fan_gap(v, n1, n2)
        );
    }
}

#[test]
fn a_round_taller_than_its_bosss_wall_is_refused_not_built_across_it() {
    // A square boss 5 mm tall on a plate, its base rounded wider than a
    // ball resting on the plate and the wall can touch the wall: refused as
    // too large, never a strip run from corner to corner. Upright at 6 mm
    // the catalogue names the wall's top edge, and the swept blend finds
    // its rail on that hard edge; leaning in at 7.2 mm (a ball touches the
    // wall above its top past 7.016 mm) the swept blend finds the ball at
    // mid-wall leaving the wall past its top edge.
    let square = [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)];
    for (scale, r) in [(1.0, 0.006), (0.85, 0.0072)] {
        let (obj, seam) = boss_on_plate(&square, scale);
        let spec = round_of(&seam, r);
        // Leaning, no smaller size turns its corners either, so the answer
        // is the corner refusal (see the plain leaning boss's spec).
        let got = obj.blend_edges(&spec).map(|_| ());
        if scale < 1.0 {
            assert!(
                matches!(got, Err(kernel::BlendError::UnsupportedVertex { .. })),
                "{scale} {r}: {got:?}"
            );
        } else {
            assert!(
                matches!(got, Err(kernel::BlendError::RadiusTooLarge { .. })),
                "{scale} {r}: {got:?}"
            );
        }
        assert_eq!(
            obj.blend_swept_refusal_for_test(&spec),
            Some(kernel::BlendError::RadiusTooLarge {
                max: 0.0,
                limited_by: kernel::LimitFeature::AdjacentFace
            }),
            "{scale} {r}"
        );
    }
}
