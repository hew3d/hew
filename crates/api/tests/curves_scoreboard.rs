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
    before: Option<Shape>,
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
    let (Some(before), Some(after)) = (before, shape_of(doc, id)) else {
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
    check_edge_classes(obj)
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

/// How far a strip facet `f` turns across its strip from its rail edge
/// `rail`, and whether that took the hop: the smallest gap (a chord)
/// between `f`'s plane normal and that of a strip facet of the same blend
/// across an edge of `f` that touches neither end of the rail; where `f`
/// has none (a triangle, whose every edge touches the rail), the same
/// taken one facet further, through each strip facet beside `f` that has
/// no rail of its own, to the strip facets across its edges that touch
/// neither end of `f`'s rail. `n` is `f`'s normal. Also returns that
/// facet's normal. `None` when neither finds one.
fn rail_turn(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    n: kernel::Vec3,
    rail: kernel::ids::EdgeId,
) -> Option<(f64, bool, kernel::Vec3)> {
    let dest = |h: kernel::ids::HalfEdgeId| obj.half_edges()[obj.half_edges()[h].next].origin;
    let rh = obj.edges()[rail].half_edge;
    let (ra, rb) = (obj.half_edges()[rh].origin, dest(rh));
    let tag = obj.faces()[f].blend.expect("a strip facet");
    let face_of = |h: kernel::ids::HalfEdgeId| obj.loops()[obj.half_edges()[h].loop_id].face;
    let strip = |g: kernel::ids::FaceId| {
        obj.faces()[g]
            .blend
            .is_some_and(|b| b.id == tag.id && b.role == kernel::topo::BlendRole::Strip)
    };
    // The faces across each edge of `g`, with whether the edge touches the
    // rail's ends.
    let across = |g: kernel::ids::FaceId| -> Vec<(kernel::ids::FaceId, bool)> {
        obj.loop_half_edges(obj.faces()[g].outer_loop)
            .filter_map(|h| {
                let twin = obj.half_edges()[h].twin?;
                let touches = [obj.half_edges()[h].origin, dest(h)]
                    .iter()
                    .any(|v| *v == ra || *v == rb);
                Some((face_of(twin), touches))
            })
            .collect()
    };
    let smallest = |c: Vec<kernel::ids::FaceId>| {
        c.into_iter()
            .map(|h| {
                let m = obj.faces()[h].plane.normal();
                ((m - n).length(), m)
            })
            .reduce(|a, b| if b.0 < a.0 { b } else { a })
    };
    let direct: Vec<_> = across(f)
        .into_iter()
        .filter(|&(g, touches)| !touches && strip(g))
        .map(|(g, _)| g)
        .collect();
    if let Some((t, m)) = smallest(direct) {
        return Some((t, false, m));
    }
    let mut hop = Vec::new();
    for (g, _) in across(f) {
        if !strip(g)
            || across(g)
                .iter()
                .any(|&(w, _)| obj.faces()[w].blend.is_none())
        {
            continue;
        }
        hop.extend(
            across(g)
                .into_iter()
                .filter(|&(h, touches)| !touches && h != f && strip(h))
                .map(|(h, _)| h),
        );
    }
    smallest(hop).map(|(t, m)| (t, true, m))
}

/// How the rail edge `rail` between an unclaimed strip facet `f` of a
/// fillet and `wall`, a face of no blend, should read. The strip only
/// approximates a round, so the rail's tangency is judged against the
/// strip's own turn across its section: `g` is how far `f`'s normal parts
/// from the wall's true normal (a chord, the larger at the rail's two
/// ends), and `t` how far the strip turns from `f` to the next facet in
/// from the rail ([`rail_turn`]: across an edge of `f` that touches
/// neither end of the rail, or, for a triangle, one facet further on; the
/// smallest such turn, so a doubtful rail fails rather than passes). A
/// rail whose gap is no more than that turn is tangent; a gap of
/// [`STRIP_END_MIN`] or more is the strip ending square against the face
/// it stops at, a crease.
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
) -> Result<Option<EdgeReads>, String> {
    rail_reading(obj, f, obj.faces()[f].plane.normal(), wall, rail, ends)
}

/// [`swept_rail_reading`] with `n` as the strip facet's normal.
fn rail_reading(
    obj: &kernel::topo::Object,
    f: kernel::ids::FaceId,
    n: kernel::Vec3,
    wall: &kernel::topo::Face,
    rail: kernel::ids::EdgeId,
    ends: [Point3; 2],
) -> Result<Option<EdgeReads>, String> {
    if obj.edges()[rail].soft {
        return Err("a soft rail between a strip facet and a face of no blend".into());
    }
    let Some((t, _, _)) = rail_turn(obj, f, n, rail) else {
        return Err("a strip facet with no neighbor across the strip to judge its rail by".into());
    };
    if t >= STRIP_END_MIN {
        return Err(format!(
            "a strip facet that turns {} to its neighbor across the strip",
            sig(t, "")
        ));
    }
    if t < CREASE_MIN {
        return Err("a strip facet with no turn to judge its rail by".into());
    }
    let g = ends
        .into_iter()
        .map(|x| {
            let w = wall.plane.normal();
            let w = match wall.surface.and_then(|s| s.normal_at(x)) {
                Some(a) if a.dot(w) < 0.0 => -a,
                Some(a) => a,
                None => w,
            };
            (w - n).length()
        })
        .fold(0.0, f64::max);
    if g <= t {
        Ok(Some(EdgeReads::Tangent))
    } else if g >= STRIP_END_MIN {
        Ok(Some(EdgeReads::Hard))
    } else {
        Err(format!(
            "strip facet neither on its rail's tangent nor ending square (normals {} apart, the strip turns {})",
            sig(g, ""),
            sig(t, "")
        ))
    }
}

fn expected_reading(
    obj: &kernel::topo::Object,
    edge: kernel::ids::EdgeId,
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
                    return swept_rail_reading(obj, face_id(strip_h), wall, edge, [p, q]);
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

/// Checks every interior edge of `obj`. `None` is a pass; `Some` names the
/// first edge that reads wrong, and how many do.
fn check_edge_classes(obj: &kernel::topo::Object) -> Option<String> {
    let mut first = None;
    let mut count = 0usize;
    for edge in obj.edges().keys() {
        let got = match obj.edge_smoothness(edge) {
            kernel::EdgeSmoothness::Hard => EdgeReads::Hard,
            kernel::EdgeSmoothness::Seam => EdgeReads::Seam,
            kernel::EdgeSmoothness::Tangent => EdgeReads::Tangent,
        };
        let problem = match expected_reading(obj, edge) {
            Ok(None) => None,
            Ok(Some(want)) if want == got => None,
            Ok(Some(want)) => Some(format!("{got:?} where {want:?} is right")),
            Err(why) => Some(why),
        };
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
            let class = if by == "unknown" {
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
        if let Some(why) = session.doc.object(id).and_then(check_edge_classes) {
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
    let before: BTreeMap<ObjectId, Shape> = s
        .doc
        .live_object_ids()
        .into_iter()
        .filter_map(|id| shape_of(&s.doc, id).map(|shape| (id, shape)))
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
                before.get(&id).copied(),
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
        assert_eq!(check_edge_classes(&after), None, "{r}");
        let rails = rails_of(&after);
        assert!(!rails.is_empty(), "{r}: the round has rails");
        for (e, f, w) in rails {
            let (p, q) = after.edge_endpoints(e).unwrap();
            assert_eq!(
                swept_rail_reading(&after, f, &after.faces()[w], e, [p, q]),
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
        assert_eq!(check_edge_classes(&after), None, "{r}");
        let rails = rails_of(&after);
        let mut hops = 0;
        for &(e, f, w) in &rails {
            let n = after.faces()[f].plane.normal();
            let (_, hop, _) = rail_turn(&after, f, n, e).expect("a turn to judge by");
            hops += usize::from(hop);
            let (p, q) = after.edge_endpoints(e).unwrap();
            assert_eq!(
                swept_rail_reading(&after, f, &after.faces()[w], e, [p, q]),
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
        rail_reading(&after, f, n, wall, e, [p, q]),
        Ok(Some(EdgeReads::Tangent))
    );
    let got = rail_reading(&after, f, tilted, wall, e, [p, q]);
    assert!(
        got.as_ref()
            .is_err_and(|why| why.starts_with("strip facet neither")),
        "{got:?}"
    );
}
