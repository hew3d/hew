//! Welded curves: Hew's circle claims (a solid edge's
//! [`kernel::CurveGeom`], a sketch chain's [`kernel::CurveAnalytic`]) as
//! `.skp` curves (`SKP_FORMAT.md` §6.4), so a circle or arc drawn in Hew is
//! one selectable curve in SketchUp rather than loose segments.
//!
//! SketchUp's rules for a curve, from Skpwright's acceptance work
//! (`docs/WRITING.md`): members run in their edges' own start-to-end
//! direction from a chain end, and an arc's start is the start vertex of
//! its first member; a curve whose members break either rule is dropped on
//! open. Chains here are therefore oriented edge by edge, and each arc's
//! frame starts at its first member's start vertex and turns
//! counter-clockwise about its normal through the chain.

use std::collections::BTreeMap;

use kernel::{Point3, Vec3};

/// One candidate member: the run's edge index, and its two run vertex
/// indices with their positions.
#[derive(Clone, Copy)]
pub(crate) struct Member {
    pub edge: usize,
    pub a: u32,
    pub b: u32,
    pub pa: Point3,
    pub pb: Point3,
}

/// What a group of members claims to be.
#[derive(Clone, Copy)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Claim {
    /// Chord facets of this circle: an arc curve.
    Circle { center: Point3, radius: f64 },
    /// A welded run with no analytic geometry (a polygon's sides).
    Plain,
}

/// A chain's members, oriented: each one's `(edge, from, to)`.
type Oriented = Vec<(usize, u32, u32)>;

/// A chain, oriented: each member's `(edge, from, to)`.
pub(crate) struct Chain {
    pub members: Oriented,
    pub curve: skpwright::Curve,
}

/// Split one claim's members into connected, unbranched chains and build
/// each one's curve. A circle claim's chain that cannot form one
/// consistent arc (it turns both ways, or about more than one axis) welds
/// as the arcs it holds ([`arcs_of`]), never as a wrong arc.
pub(crate) fn chains(members: &[Member], claim: Claim) -> Vec<Chain> {
    let mut at: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, m) in members.iter().enumerate() {
        at.entry(m.a).or_default().push(i);
        at.entry(m.b).or_default().push(i);
    }
    let mut used = vec![false; members.len()];
    let mut out = Vec::new();
    // Open chains first, from their ends (a vertex used once, or a branch
    // point, where a chain must stop); whatever is left is closed loops.
    let mut starts: Vec<u32> = at
        .iter()
        .filter(|(_, es)| es.len() != 2)
        .map(|(&v, _)| v)
        .collect();
    starts.sort_unstable();
    let walk = |start: u32, used: &mut Vec<bool>| -> Vec<(usize, u32, u32)> {
        let mut chain = Vec::new();
        let mut v = start;
        loop {
            let next = at
                .get(&v)
                .and_then(|es| es.iter().copied().find(|&i| !used[i]));
            let Some(i) = next else { break };
            used[i] = true;
            let m = members[i];
            let to = if m.a == v { m.b } else { m.a };
            chain.push((m.edge, v, to));
            v = to;
            // Stop at a branch point: it starts chains of its own.
            if at.get(&v).is_some_and(|es| es.len() > 2) || v == start {
                break;
            }
        }
        chain
    };
    for s in starts {
        while at.get(&s).is_some_and(|es| es.iter().any(|&i| !used[i])) {
            let c = walk(s, &mut used);
            if c.is_empty() {
                break;
            }
            out.push(c);
        }
    }
    for i in 0..members.len() {
        if !used[i] {
            let c = walk(members[i].a, &mut used);
            if !c.is_empty() {
                out.push(c);
            }
        }
    }
    let pos: BTreeMap<u32, Point3> = members
        .iter()
        .flat_map(|m| [(m.a, m.pa), (m.b, m.pb)])
        .collect();
    out.into_iter()
        // (A single chord of a circle is an arc of one segment, which
        // SketchUp writes too; a lone edge welds to nothing.)
        .filter(|c| c.len() >= 2 || matches!(claim, Claim::Circle { .. }))
        .flat_map(|c| match claim {
            Claim::Circle { center, radius } => arcs_of(c, &pos, center, radius)
                .into_iter()
                .map(|(c, arc)| (c, Some(arc)))
                .collect(),
            Claim::Plain => vec![(c, None)],
        })
        .map(|(c, arc)| Chain {
            curve: skpwright::Curve {
                pid: 0,
                members: c.len() as u32,
                arc,
            },
            members: c,
        })
        .collect()
}

/// A circle claim's chain as the arcs it holds: one arc when it is one,
/// else split where it turns back (a curve and a longer chord of the same
/// circle closing a triangle, as SketchUp files hold where a circle was
/// redrawn finer). A chord that is no arc on its own stays a loose edge:
/// a wrong arc would misplace it in SketchUp.
fn arcs_of(
    chain: Oriented,
    pos: &BTreeMap<u32, Point3>,
    center: Point3,
    radius: f64,
) -> Vec<(Oriented, skpwright::Arc)> {
    if let Some(arc) = arc_of(&chain, pos, center, radius) {
        return vec![(chain, arc)];
    }
    let mut chain = chain;
    // A closed loop starts over at a chord that does not continue the
    // arc of the one before it, so no arc is cut at an arbitrary vertex.
    let n = chain.len();
    if n > 1 && chain[0].1 == chain[n - 1].2 {
        let breaks =
            |k: usize| arc_of(&[chain[(k + n - 1) % n], chain[k]], pos, center, radius).is_none();
        if let Some(k) = (0..n).find(|&k| breaks(k)) {
            chain.rotate_left(k);
        }
    }
    let mut out = Vec::new();
    let mut run: Oriented = Vec::new();
    let flush = |run: &mut Vec<_>, out: &mut Vec<_>| {
        if let Some(arc) = arc_of(run, pos, center, radius) {
            out.push((std::mem::take(run), arc));
        }
        run.clear();
    };
    for m in chain {
        run.push(m);
        if run.len() > 1 && arc_of(&run, pos, center, radius).is_none() {
            run.pop();
            flush(&mut run, &mut out);
            run.push(m);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// How far (cosine) a chord's own axis may tilt from its arc's: about 18°
/// (a SketchUp arc a few millimeters across, read back from the file's
/// rounded coordinates, tilts a chord by up to 11°).
const ARC_AXIS_COS: f64 = 0.95;

/// The arc an oriented chain sweeps on `center`/`radius`, or `None` when
/// the chain does not turn one way about one axis.
fn arc_of(
    chain: &[(usize, u32, u32)],
    pos: &BTreeMap<u32, Point3>,
    center: Point3,
    radius: f64,
) -> Option<skpwright::Arc> {
    let p = |v: u32| pos[&v] - center;
    let (_, f0, _) = chain[0];
    // The axis the whole chain turns about (every chord's turn summed): a
    // small part's chords, rounded to the file's precision, each tilt off
    // it a little, so no single chord's own axis is the arc's.
    let normal = chain
        .iter()
        .fold(Vec3::ZERO, |n, &(_, f, t)| n + p(f).cross(p(t)))
        .normalized()
        .ok()?;
    // A vertex as the arc sees it: in the arc's plane through the center.
    let flat = |v: u32| {
        let r = p(v);
        r - normal * r.dot(normal)
    };
    let mut sweep = 0.0;
    for &(_, f, t) in chain {
        // Every chord turns the same way about the one axis, within a few
        // degrees of it.
        if p(f).cross(p(t)).normalized().ok()?.dot(normal) < ARC_AXIS_COS {
            return None;
        }
        let (a, b) = (flat(f), flat(t));
        sweep += a.cross(b).dot(normal).atan2(a.dot(b));
    }
    let closed = chain.last().map(|&(_, _, t)| t) == Some(f0);
    // An orthonormal frame in that plane, x through the first vertex.
    let x = flat(f0).normalized().ok()?;
    let y = normal.cross(x);
    let m = |v: Vec3| [v.x, v.y, v.z];
    Some(skpwright::Arc {
        center_m: [center.x, center.y, center.z],
        normal: m(normal),
        x_axis_m: m(x * radius),
        y_axis_m: m(y * radius),
        radius_m: radius,
        start_rad: 0.0,
        end_rad: if closed { std::f64::consts::TAU } else { sweep },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(edge: usize, a: u32, b: u32, pa: Point3, pb: Point3) -> Member {
        Member { edge, a, b, pa, pb }
    }

    fn on_circle(r: f64, deg: f64, z: f64) -> Point3 {
        let t = deg.to_radians();
        Point3::new(r * t.cos(), r * t.sin(), z)
    }

    #[test]
    fn a_single_chord_of_a_circle_is_an_arc_of_one_segment() {
        let (p, q) = (on_circle(1.0, 0.0, 0.0), on_circle(1.0, 30.0, 0.0));
        let out = chains(
            &[member(0, 0, 1, p, q)],
            Claim::Circle {
                center: Point3::ORIGIN,
                radius: 1.0,
            },
        );
        assert_eq!(out.len(), 1);
        let arc = out[0].curve.arc.as_ref().expect("an arc");
        assert!((arc.end_rad - 30f64.to_radians()).abs() < 1e-9);
        // A lone edge of no circle welds to nothing.
        assert!(chains(&[member(0, 0, 1, p, q)], Claim::Plain).is_empty());
    }

    #[test]
    fn chords_a_little_off_the_arcs_plane_still_make_the_arc() {
        // A small arc read back from rounded coordinates: its center sits
        // off the chords' plane, so each chord's own axis tilts.
        let r = 0.0024;
        let pts: Vec<Point3> = (0..6)
            .map(|k| on_circle(r, 20.0 * k as f64, if k % 2 == 0 { 0.0001 } else { 0.0 }))
            .collect();
        let ms: Vec<Member> = (0..5)
            .map(|k| member(k, k as u32, k as u32 + 1, pts[k], pts[k + 1]))
            .collect();
        let out = chains(
            &ms,
            Claim::Circle {
                center: Point3::ORIGIN,
                radius: r,
            },
        );
        assert_eq!(out.len(), 1);
        let arc = out[0].curve.arc.as_ref().expect("an arc");
        // Its frame is orthonormal at the radius, in the arc's plane.
        let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
        let (x, y, n) = (v(arc.x_axis_m), v(arc.y_axis_m), v(arc.normal));
        assert!((x.length() - r).abs() < 1e-12 && (y.length() - r).abs() < 1e-12);
        assert!(x.dot(y).abs() < 1e-15 && x.dot(n).abs() < 1e-15 && y.dot(n).abs() < 1e-15);
        // And it ends where the chain does, seen in its plane.
        let end = x * (arc.end_rad.cos() / r) + y * (arc.end_rad.sin() / r);
        let last = pts[5] - Point3::ORIGIN;
        let last = (last - n * last.dot(n)).normalized().unwrap();
        assert!((end - last).length() < 1e-9, "{end:?} vs {last:?}");
        // A chain that turns back is no one arc: each way is its own.
        let back = [
            member(0, 0, 1, pts[0], pts[1]),
            member(1, 1, 2, pts[1], pts[0]),
        ];
        let out = chains(
            &back,
            Claim::Circle {
                center: Point3::ORIGIN,
                radius: r,
            },
        );
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.members.len() == 1));
    }

    #[test]
    fn a_curve_and_the_chord_it_spans_weld_as_two_arcs() {
        // Two chords of a circle and the longer chord across both close a
        // triangle (a circle SketchUp redrew finer keeps its old chord):
        // the pair is one arc, the long chord another, neither loose —
        // wherever the walk round the loop starts.
        let p: Vec<Point3> = [0.0, 15.0, 30.0]
            .iter()
            .map(|&d| on_circle(1.0, d, 0.0))
            .collect();
        let ms = [
            member(0, 0, 2, p[0], p[2]),
            member(1, 2, 1, p[2], p[1]),
            member(2, 1, 0, p[1], p[0]),
        ];
        for first in 0..3 {
            let mut listed = ms.to_vec();
            listed.rotate_left(first);
            let out = chains(
                &listed,
                Claim::Circle {
                    center: Point3::ORIGIN,
                    radius: 1.0,
                },
            );
            let mut got: Vec<(Vec<usize>, f64)> = out
                .iter()
                .map(|c| {
                    let arc = c.curve.arc.as_ref().expect("an arc");
                    let mut edges: Vec<usize> = c.members.iter().map(|m| m.0).collect();
                    edges.sort_unstable();
                    (edges, arc.end_rad.to_degrees())
                })
                .collect();
            got.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(got.len(), 2, "from {first}: {got:?}");
            assert_eq!(got[0].0, vec![0]);
            assert!((got[0].1 - 30.0).abs() < 1e-9, "{got:?}");
            assert_eq!(got[1].0, vec![1, 2]);
            assert!((got[1].1 - 30.0).abs() < 1e-9, "{got:?}");
        }
    }
}
