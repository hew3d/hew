//! `SolidSpec`, the solid operand of the `hew.compute.*` commands and of a
//! machining through a placement (docs/agents/HEW_API.md §7 "Compute",
//! Addendum B6): an entity's solid, a transient prism, or a box, always
//! resolved to a transient [`kernel::Object`] in WORLD coordinates that no
//! document record holds.
//!
//! - `{ "id": "obj_…", "pose"?: [12] }` / `{ "path": [instance, …, object],
//!   "pose"?: [12] }`: exactly one Object (a group or instance refuses
//!   `not_an_object`), at its world pose, or at `pose` when given (the
//!   pose REPLACES the world pose).
//! - `{ "prism": { "plane": { "origin", "normal" }, "loops": [[[x,y,z], …],
//!   …], "distance" } }`: the first loop is the outer boundary, the rest
//!   holes; built the way `hew.solid.extrude` builds a solid, on a scratch
//!   sketch.
//! - `{ "box": { "min", "max", "pose"?: [12] } }`: sugar for the prism of
//!   its bottom rectangle.
//!
//! A pose that mirrors (determinant < 0) refuses `mirrored_pose`: kernel
//! transforms never reflect a solid.

use crate::commands::{CmdError, Ctx};
use crate::geom;
use crate::locate;
use crate::refusal::Refusal;
use kernel::{Document, DocumentError, NodeId, Object, Plane, Point3, Transform, Vec3};
use serde::Deserialize;
use serde_json::Value;

/// The most triangles one computation takes in, summed over its inputs
/// (an upper bound from face loop sizes, [`check_compute_size`]).
pub const COMPUTE_MAX_TRIANGLES: usize = 2_000_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    path: Option<Vec<String>>,
    #[serde(default)]
    pose: Option<Vec<f64>>,
    #[serde(default)]
    prism: Option<PrismParams>,
    #[serde(rename = "box", default)]
    box_: Option<BoxParams>,
}

/// `{ origin, normal }` in world coordinates.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlaneParams {
    pub origin: [f64; 3],
    pub normal: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrismParams {
    plane: PlaneParams,
    loops: Vec<Vec<[f64; 3]>>,
    distance: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoxParams {
    min: [f64; 3],
    max: [f64; 3],
    #[serde(default)]
    pose: Option<Vec<f64>>,
}

fn params_err(msg: &str) -> CmdError {
    CmdError::Params(msg.to_string())
}

fn point(v: [f64; 3]) -> Point3 {
    Point3::new(v[0], v[1], v[2])
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// `mirrored_pose`: a pose that reflects the solid.
pub fn mirrored_pose() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "mirrored_pose",
        "That pose mirrors the part. Compute with the un-mirrored pose and flip the result yourself.",
    ))
}

/// `not_an_object`: a solid operand named a group or an instance.
pub fn not_an_object() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "not_an_object",
        "A solid operand is one object. List a group's or instance's objects with hew.query.occurrences, then name one.",
    ))
}

/// A world plane `{ origin, normal }`.
pub(crate) fn plane_of(p: &PlaneParams) -> Result<Plane, CmdError> {
    if !finite(&p.origin) || !finite(&p.normal) {
        return Err(params_err("plane coordinates must be finite numbers"));
    }
    Plane::from_point_normal(
        point(p.origin),
        Vec3::new(p.normal[0], p.normal[1], p.normal[2]),
    )
    .map_err(|_| params_err("degenerate plane normal"))
}

/// A row-major 3×4 pose: twelve finite numbers, invertible, not mirroring.
fn pose_of(v: &[f64]) -> Result<Transform, CmdError> {
    let rows: [f64; 12] = v
        .try_into()
        .map_err(|_| params_err("pose must be 12 numbers (a row-major 3×4 matrix)"))?;
    if !finite(&rows) {
        return Err(params_err("pose must be finite numbers"));
    }
    let t = Transform::from_affine(&rows);
    if t.determinant() < 0.0 {
        return Err(mirrored_pose());
    }
    if t.inverse().is_err() {
        return Err(params_err("pose is singular"));
    }
    Ok(t)
}

fn transformed(mut object: Object, pose: &Transform) -> Result<Object, CmdError> {
    if *pose != Transform::IDENTITY {
        object
            .apply_transform(pose)
            .map_err(|e| CmdError::from(DocumentError::Transform(e)))?;
    }
    Ok(object)
}

/// Resolves one `SolidSpec` to a transient world-space Object.
pub fn resolve_solid(ctx: &Ctx, spec: &Value) -> Result<Object, CmdError> {
    let p: SpecParams = serde_json::from_value(spec.clone()).map_err(|e| {
        CmdError::Params(format!(
            "a solid is {{id|path, pose?}}, {{prism}}, or {{box}}: {e}"
        ))
    })?;
    let forms = [
        p.id.is_some(),
        p.path.is_some(),
        p.prism.is_some(),
        p.box_.is_some(),
    ]
    .iter()
    .filter(|&&b| b)
    .count();
    if forms != 1 {
        return Err(params_err(
            "a solid names exactly one of id, path, prism, or box",
        ));
    }
    if p.pose.is_some() && (p.prism.is_some() || p.box_.is_some()) {
        return Err(params_err(
            "pose sits beside id or path; a box carries its own pose, a prism is placed by its plane",
        ));
    }
    if let Some(prism) = &p.prism {
        if prism.loops.iter().flatten().any(|q| !finite(q)) {
            return Err(params_err("loop points must be finite numbers"));
        }
        let plane = plane_of(&prism.plane)?;
        let rings: Vec<Vec<Point3>> = prism
            .loops
            .iter()
            .map(|l| l.iter().copied().map(point).collect())
            .collect();
        return build_prism(plane, &rings, prism.distance);
    }
    if let Some(b) = &p.box_ {
        if !finite(&b.min) || !finite(&b.max) {
            return Err(params_err("box corners must be finite numbers"));
        }
        if (0..3).any(|k| b.min[k] >= b.max[k]) {
            return Err(params_err("a box needs min < max on every axis"));
        }
        let pose = b.pose.as_deref().map(pose_of).transpose()?;
        let (lo, hi) = (b.min, b.max);
        let plane = Plane::from_point_normal(point(lo), Vec3::new(0.0, 0.0, 1.0))
            .map_err(|_| CmdError::Internal("the box base plane is degenerate".into()))?;
        let ring = vec![
            Point3::new(lo[0], lo[1], lo[2]),
            Point3::new(hi[0], lo[1], lo[2]),
            Point3::new(hi[0], hi[1], lo[2]),
            Point3::new(lo[0], hi[1], lo[2]),
        ];
        let solid = build_prism(plane, &[ring], hi[2] - lo[2])?;
        return match pose {
            Some(pose) => transformed(solid, &pose),
            None => Ok(solid),
        };
    }
    let occ = match (&p.id, &p.path) {
        (Some(id), None) => locate::resolve_world_node(ctx, id)?,
        (None, Some(path)) => locate::resolve_occurrence_path(ctx, path)?,
        _ => unreachable!("exactly one form was counted above"),
    };
    let NodeId::Object(oid) = occ.node else {
        return Err(not_an_object());
    };
    let object = ctx
        .doc
        .object(oid)
        .ok_or_else(|| CmdError::Internal("dangling object behind an occurrence".into()))?
        .clone();
    let pose = match &p.pose {
        // The given pose replaces the world pose.
        Some(v) => pose_of(v)?,
        None => {
            if occ.pose.determinant() < 0.0 {
                return Err(mirrored_pose());
            }
            occ.pose
        }
    };
    transformed(object, &pose)
}

/// The most points the loops of one scratch region may hold, summed. The
/// triangle cap alone would allow `COMPUTE_MAX_TRIANGLES / 6` points
/// ([`prism_triangle_bound`] counts six triangles per point), but the
/// region is found by building a scratch sketch whose cost still grows
/// faster than linearly with the point count (about 25 s for 8,000 points
/// in a debug build), so the cap keeps one call to seconds in a release
/// build.
pub const MAX_LOOP_POINTS: usize = 20_000;

/// The [`triangle_bound`] of the prism `rings` would make, from the point
/// count alone: per point, a side quad (four) and one point of each cap.
pub(crate) fn prism_triangle_bound(rings: &[Vec<Point3>]) -> usize {
    rings.iter().map(Vec::len).sum::<usize>().saturating_mul(6)
}

/// Draws `rings` (outer first, then holes) on a scratch sketch on `plane`
/// and answers the region holding the material between them, with the
/// scratch document. Every loop is checked, and `-32602` names what is
/// wrong: a loop under three points, a point off the plane, two
/// consecutive points that coincide, loops that cross or touch (a bowtie,
/// a loop through one point twice, a hole across the outer loop or another
/// hole), or a hole not inside the outer loop. More than
/// [`MAX_LOOP_POINTS`] points is `-32602` before anything is built.
///
/// The crossing check is the sketch's own: simple, disjoint loops of `N`
/// points in all draw exactly `N` vertices and `N` edges, and any
/// crossing, touch, or revisit splits an edge or merges two vertices,
/// which changes a count. The segments go in through the bulk path
/// ([`kernel::Sketch::add_segments`]), so the build is near-linear.
pub(crate) fn scratch_region(
    plane: Plane,
    rings: &[Vec<Point3>],
) -> Result<(Document, kernel::SketchId, kernel::SketchRegionId), CmdError> {
    let Some((outer, holes)) = rings.split_first() else {
        return Err(params_err("loops needs at least the outer loop"));
    };
    if rings.iter().any(|r| r.len() < 3) {
        return Err(params_err("every loop needs at least 3 points"));
    }
    let points: usize = rings.iter().map(Vec::len).sum();
    if points > MAX_LOOP_POINTS {
        return Err(CmdError::Params(format!(
            "the loops hold {points} points; at most {} fit in one call",
            group_thousands(MAX_LOOP_POINTS)
        )));
    }
    for (i, ring) in rings.iter().enumerate() {
        for (j, &q) in ring.iter().enumerate() {
            if plane.signed_distance(q).abs() > kernel::tol::PLANE_DIST {
                return Err(CmdError::Params(format!(
                    "loop {i}, point {j} lies off the plane"
                )));
            }
            let next = (j + 1) % ring.len();
            if q.approx_eq(ring[next], kernel::tol::POINT_MERGE) {
                return Err(CmdError::Params(format!(
                    "loop {i}: points {j} and {next} coincide"
                )));
            }
        }
    }
    let segs: Vec<(Point3, Point3)> = rings
        .iter()
        .flat_map(|ring| (0..ring.len()).map(move |k| (ring[k], ring[(k + 1) % ring.len()])))
        .collect();
    let mut doc = Document::new();
    let s = doc.add_sketch(plane);
    {
        let sk = doc
            .sketch_mut(s)
            .ok_or_else(|| CmdError::Internal("scratch sketch vanished".into()))?;
        sk.add_segments(&segs).map_err(|e| {
            CmdError::Params(format!("the loops cannot be drawn on the plane: {e}"))
        })?;
        if sk.vertices().len() != points || sk.edges().len() != points {
            return Err(params_err(
                "the loops cross or touch; every loop must be simple, and apart from the others",
            ));
        }
    }
    // The loops are simple and disjoint, so one point of a hole tells
    // whether the whole hole is inside the outer loop.
    let normal = plane.normal();
    if holes
        .iter()
        .any(|hole| !geom::ring_contains(outer, normal, hole[0]))
    {
        return Err(params_err(
            "every later loop is a hole and must lie strictly inside the first",
        ));
    }
    let inside = geom::interior_point_of_loops(outer, holes, normal)
        .ok_or_else(|| params_err("the loops enclose no area"))?;
    let sketch = doc
        .sketch(s)
        .ok_or_else(|| CmdError::Internal("scratch sketch vanished".into()))?;
    let region = doc
        .extrudable_regions(s)?
        .into_iter()
        .find(|&r| sketch.region_contains_point(r, inside).unwrap_or(false))
        .ok_or_else(|| params_err("the loops cross or touch; they enclose no single region"))?;
    let shape = &sketch.regions()[region];
    if shape.holes.len() != holes.len() {
        return Err(params_err(
            "every later loop is a hole and must lie inside the first, not inside another hole",
        ));
    }
    Ok((doc, s, region))
}

/// A transient prism: `rings` on `plane` swept by `distance` along the
/// plane normal, exactly as `hew.solid.extrude` sweeps a drawn region.
/// `compute_too_large` from the point count before anything is built, and
/// `-32602` when the prism is too large for its area, volume, or extent to
/// be a finite number.
pub fn build_prism(plane: Plane, rings: &[Vec<Point3>], distance: f64) -> Result<Object, CmdError> {
    if !distance.is_finite() || distance == 0.0 {
        return Err(params_err("distance must be a finite, non-zero number"));
    }
    too_large(prism_triangle_bound(rings), COMPUTE_MAX_TRIANGLES)?;
    let (mut doc, s, region) = scratch_region(plane, rings)?;
    let (object, _) = doc.extrude_region(s, region, distance)?;
    let object = doc
        .object(object)
        .cloned()
        .ok_or_else(|| CmdError::Internal("the extruded prism vanished".into()))?;
    check_finite_measure(&object)?;
    Ok(object)
}

/// `-32602` unless an Object's area, volume, and bounding box are finite:
/// geometry so large they overflow cannot be measured, and a non-finite
/// number would serialize as `null`, which reads as an open shell.
pub fn check_finite_measure(object: &Object) -> Result<(), CmdError> {
    let bbox_finite = geom::bbox(object.vertices().values().map(|v| v.position))
        .is_none_or(|(lo, hi)| finite(&[lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]));
    if object.surface_area().is_finite()
        && object.volume().is_none_or(f64::is_finite)
        && bbox_finite
    {
        Ok(())
    } else {
        Err(params_err(
            "that solid is too large to measure: its area or volume is past the largest number there is; scale it down",
        ))
    }
}

/// The fan-triangulation upper bound of an Object's triangle count: each
/// face contributes the vertex count of its outer loop and of every hole.
fn triangle_bound(object: &Object) -> usize {
    object
        .faces()
        .values()
        .map(|f| {
            std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .map(|l| object.loop_half_edges(l).count())
                .sum::<usize>()
        })
        .sum()
}

/// `compute_too_large` past [`COMPUTE_MAX_TRIANGLES`], checked after the
/// operands resolve and before any boolean or slice runs.
pub fn check_compute_size(solids: &[&Object]) -> Result<(), CmdError> {
    check_compute_size_limited(solids, COMPUTE_MAX_TRIANGLES)
}

/// [`check_compute_size`] with the limit as a parameter, so the refusal is
/// testable on a small fixture.
pub(crate) fn check_compute_size_limited(solids: &[&Object], limit: usize) -> Result<(), CmdError> {
    too_large(solids.iter().map(|o| triangle_bound(o)).sum(), limit)
}

/// `compute_too_large` when `triangles` is past `limit`.
fn too_large(triangles: usize, limit: usize) -> Result<(), CmdError> {
    if triangles > limit {
        return Err(CmdError::Refusal(
            Refusal::api(
                "compute_too_large",
                &format!(
                    "That geometry has more than {} triangles, which is too much to compute on at once. Compute on one part, or on a smaller region.",
                    group_thousands(limit)
                ),
            )
            .with_detail(serde_json::json!({ "triangles": triangles, "limit": limit })),
        ));
    }
    Ok(())
}

fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Object {
        let plane =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let ring = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        build_prism(plane, &[ring], 1.0).unwrap()
    }

    /// The cap counts face-loop vertices before any work: a box is six quads,
    /// 24, so a limit of 47 refuses two of them with the count and the limit.
    #[test]
    fn compute_too_large_refuses_before_work() {
        let cube = unit_box();
        assert!(check_compute_size_limited(&[&cube], 24).is_ok());
        match check_compute_size_limited(&[&cube, &cube], 47) {
            Err(CmdError::Refusal(r)) => {
                assert_eq!(r.name, "compute_too_large");
                assert_eq!(r.detail["triangles"], 48);
                assert_eq!(r.detail["limit"], 47);
            }
            other => panic!("expected compute_too_large, got {other:?}"),
        }
        assert!(
            check_compute_size(&[&cube]).is_ok(),
            "the real cap is far above a box"
        );
    }

    #[test]
    fn the_cap_reads_with_thousands_separators() {
        assert_eq!(group_thousands(COMPUTE_MAX_TRIANGLES), "2,000,000");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
    }
}
