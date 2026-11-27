//! The offset engine (true-curves.md §5.6): new positions for a face set's
//! vertices moved a fixed distance along the solid's normals. Shell builds
//! on it, and Project's follow mode reuses it.
//!
//! Per vertex, the incident faces split into groups at hard edges; each
//! group contributes one constraint "lie `distance` inside this group's
//! surface". A linear solve on the offset tangent planes gives a first
//! point, and Gauss–Newton on each claimed group's true distance function
//! moves it onto the offset claims exactly (decision I-25), so the offset
//! faces' claims verify.

use crate::trig;
use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{FaceId, HalfEdgeId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::shape_error::ShapeError;
use crate::surface::EdgeSmoothness;
use crate::tol;
use crate::topo::{FaceAttrs, Object, SurfaceRef};

/// The Gauss–Newton iteration cap of the refinement: a step count, not a
/// tolerance.
const OFFSET_NEWTON_STEPS: usize = 8;

/// One group's constraint at a vertex: zero on the group's offset surface,
/// growing outward from the material.
#[derive(Debug, Clone, Copy)]
enum Constraint {
    /// `n·(x − v) + d`: an unclaimed group's offset tangent plane.
    Plane { n: Vec3, v: Point3 },
    /// `s·dist(x) + d` on a claimed group's surface.
    Claim { surface: SurfaceRef, s: f64 },
    /// `n·(x − v)`: a plane the point must stay in, whatever the offset
    /// (an opened face's plane, where Shell's rim lies).
    Fixed { n: Vec3, v: Point3 },
}

impl Constraint {
    /// The value and the (outward) gradient at `x`, for inward offset `d`.
    fn eval(&self, x: Point3, d: f64) -> Option<(f64, Vec3)> {
        match *self {
            Constraint::Plane { n, v } => Some((n.dot(x - v) + d, n)),
            Constraint::Claim { surface, s } => {
                let g = surface.normal_at(x)?;
                Some((s * surface.distance(x) + d, g * s))
            }
            Constraint::Fixed { n, v } => Some((n.dot(x - v), n)),
        }
    }
}

/// The incident faces of `v` in cyclic order around it, each with the
/// half-edge leaving `v` in that face. Also whether the fan closes (an open
/// object's boundary vertex does not).
fn fan(obj: &Object, v: VertexId) -> (Vec<(FaceId, HalfEdgeId)>, bool) {
    let hes = obj.half_edges();
    let start = obj.vertices()[v].outgoing;
    let mut out = Vec::new();
    let mut h = start;
    loop {
        out.push((obj.loops()[hes[h].loop_id].face, h));
        // The next outgoing half-edge around `v`: across the edge arriving
        // at `v` in this face.
        let Some(t) = hes[hes[h].prev].twin else {
            return (out, false);
        };
        h = t;
        if h == start || out.len() > hes.len() {
            return (out, true);
        }
    }
}

/// The interior angle of `face` at the corner where half-edge `h` leaves.
fn corner_angle(obj: &Object, h: HalfEdgeId) -> f64 {
    let hes = obj.half_edges();
    let pos = |v: VertexId| obj.vertices()[v].position;
    let p = pos(hes[h].origin);
    let a = pos(hes[hes[h].next].origin) - p;
    let b = pos(hes[hes[h].prev].origin) - p;
    let (la, lb) = (a.length(), b.length());
    if la <= tol::POINT_MERGE || lb <= tol::POINT_MERGE {
        return 0.0;
    }
    trig::acos((a.dot(b) / (la * lb)).clamp(-1.0, 1.0))
}

/// `s` of a claimed face: +1 when the face's outward normal at its
/// centroid points away from the claim's center or axis, else −1.
fn claim_sign(obj: &Object, face: FaceId, surface: &SurfaceRef) -> Option<f64> {
    let f = &obj.faces()[face];
    let pts: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
    let n = pts.len() as f64;
    let c = pts
        .iter()
        .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| acc + p.to_vec())
        * (1.0 / n);
    let g = surface.normal_at(Point3::new(c.x, c.y, c.z))?;
    Some(if g.dot(f.plane.normal()) >= 0.0 {
        1.0
    } else {
        -1.0
    })
}

/// The groups of faces in `faces` around `v`: consecutive faces joined
/// across a non-hard edge share a group. Each group is listed by its faces
/// with their corner half-edges.
fn groups_at(
    obj: &Object,
    faces: &BTreeSet<FaceId>,
    v: VertexId,
) -> Vec<Vec<(FaceId, HalfEdgeId)>> {
    let (ring, closed) = fan(obj, v);
    let n = ring.len();
    // `joined[i]`: ring[i] and ring[i + 1] are both offset faces and meet
    // at `v` across a non-hard edge (the edge arriving at `v` in ring[i]).
    let hes = obj.half_edges();
    let joined: Vec<bool> = (0..n)
        .map(|i| {
            if !closed && i + 1 == n {
                return false;
            }
            let (fa, h) = ring[i];
            let fb = ring[(i + 1) % n].0;
            faces.contains(&fa)
                && faces.contains(&fb)
                && obj.edge_smoothness(hes[hes[h].prev].edge) != EdgeSmoothness::Hard
        })
        .collect();
    // Start a group after a break, so a group never wraps the ring's end.
    let Some(first_break) = (0..n).find(|&i| !joined[i]) else {
        // Every consecutive pair joined: one group around the whole vertex.
        return vec![ring];
    };
    let mut groups: Vec<Vec<(FaceId, HalfEdgeId)>> = Vec::new();
    let mut current: Vec<(FaceId, HalfEdgeId)> = Vec::new();
    for k in 1..=n {
        let i = (first_break + k) % n;
        if faces.contains(&ring[i].0) {
            current.push(ring[i]);
        }
        if !joined[i] && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
    }
    groups
}

/// The constraint and the tangent-plane normal of one group at `v`.
fn group_constraint(
    obj: &Object,
    group: &[(FaceId, HalfEdgeId)],
    v: Point3,
) -> Option<(Constraint, Vec3)> {
    let claimed = group
        .iter()
        .filter_map(|&(f, _)| obj.faces()[f].surface.map(|s| (f, s)))
        .min_by_key(|&(f, _)| f);
    if let Some((f, surface)) = claimed {
        let plane_n = obj.faces()[f].plane.normal();
        let n = match surface.normal_at(v) {
            Some(n) if n.dot(plane_n) >= 0.0 => n,
            Some(n) => n * -1.0,
            None => plane_n,
        };
        let s = claim_sign(obj, f, &surface)?;
        return Some((Constraint::Claim { surface, s }, n));
    }
    let n = if group.len() == 1 {
        obj.faces()[group[0].0].plane.normal()
    } else {
        let sum = group.iter().fold(Vec3::new(0.0, 0.0, 0.0), |acc, &(f, h)| {
            acc + obj.faces()[f].plane.normal() * corner_angle(obj, h)
        });
        sum.normalized().ok()?
    };
    Some((Constraint::Plane { n, v }, n))
}

/// Solves `n_g·(x − v) = −d` for every normal: exactly for up to three
/// independent normals, by least squares beyond (accepted only when every
/// row holds within `PLANE_DIST`). Near-parallel normals count once.
fn solve_planes(v: Point3, normals: &[Vec3], d: f64) -> Option<Point3> {
    let mut rows: Vec<Vec3> = Vec::new();
    for &n in normals {
        if rows
            .iter()
            .all(|r| r.dot(n).abs() < 1.0 - tol::NORMAL_DIRECTION)
        {
            rows.push(n);
        }
    }
    let residual_ok = |x: Point3| {
        normals
            .iter()
            .all(|n| (n.dot(x - v) + d).abs() <= tol::PLANE_DIST)
    };
    let x = match rows.len() {
        0 => return None,
        1 => v - rows[0] * d,
        2 => solve_two(v, rows[0], rows[1], d),
        3 => match solve3(rows[0], rows[1], rows[2], [-d, -d, -d]) {
            Some(delta) => v + delta,
            None => best_pair(v, &rows, d)?,
        },
        _ => {
            // Normal equations (NᵀN) δ = Nᵀ(−d·1).
            let mut m = [[0.0f64; 3]; 3];
            let mut rhs = [0.0f64; 3];
            for r in &rows {
                let a = [r.x, r.y, r.z];
                for i in 0..3 {
                    for j in 0..3 {
                        m[i][j] += a[i] * a[j];
                    }
                    rhs[i] += a[i] * -d;
                }
            }
            match solve3_mat(m, rhs) {
                Some(delta) => v + delta,
                None => best_pair(v, &rows, d)?,
            }
        }
    };
    residual_ok(x).then_some(x)
}

/// The two-plane solve `x = v + α n1 + β n2`.
fn solve_two(v: Point3, n1: Vec3, n2: Vec3, d: f64) -> Point3 {
    let c = n1.dot(n2);
    // [1 c; c 1][α; β] = [−d; −d] → α = β = −d / (1 + c).
    let a = -d / (1.0 + c);
    v + n1 * a + n2 * a
}

/// Rank-two rows (normals spanning a plane): the most independent pair
/// solved exactly; the caller checks the other rows.
fn best_pair(v: Point3, rows: &[Vec3], d: f64) -> Option<Point3> {
    let mut best: Option<(f64, usize, usize)> = None;
    for i in 0..rows.len() {
        for j in i + 1..rows.len() {
            let c = rows[i].dot(rows[j]).abs();
            if best.is_none_or(|(bc, _, _)| c < bc) {
                best = Some((c, i, j));
            }
        }
    }
    let (_, i, j) = best?;
    Some(solve_two(v, rows[i], rows[j], d))
}

fn solve3(a: Vec3, b: Vec3, c: Vec3, rhs: [f64; 3]) -> Option<Vec3> {
    solve3_mat([[a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z]], rhs)
}

/// Cramer's rule; `None` when `|det| ≤ NORMALIZE_MIN_LENGTH`.
fn solve3_mat(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<Vec3> {
    let det3 = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let det = det3(m);
    if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let col = |k: usize| {
        let mut mk = m;
        for (i, row) in mk.iter_mut().enumerate() {
            row[k] = r[i];
        }
        det3(mk) / det
    };
    Some(Vec3::new(col(0), col(1), col(2)))
}

/// Gauss–Newton on the true constraints from `x` (decision I-25). `None`
/// when it does not converge within [`OFFSET_NEWTON_STEPS`] or a step's
/// system is singular.
fn refine(mut x: Point3, rows: &[Constraint], d: f64) -> Option<Point3> {
    for step in 0..=OFFSET_NEWTON_STEPS {
        let evals: Vec<(f64, Vec3)> = rows.iter().map(|c| c.eval(x, d)).collect::<Option<_>>()?;
        if evals.iter().all(|(g, _)| g.abs() <= tol::PLANE_DIST) {
            return Some(x);
        }
        if step == OFFSET_NEWTON_STEPS {
            return None;
        }
        // Rows whose gradients are parallel to an earlier row's describe
        // the same surface locally (two tangent groups); keep one.
        let mut kept: Vec<(f64, Vec3)> = Vec::new();
        for &(g, j) in &evals {
            if kept
                .iter()
                .all(|&(_, k)| k.dot(j).abs() < 1.0 - tol::NORMAL_DIRECTION)
            {
                kept.push((g, j));
            }
        }
        let delta = match kept.len() {
            1 => {
                let (g, j) = kept[0];
                j * (-g / j.length_squared())
            }
            2 => {
                // Δ = −Jᵀ(JJᵀ)⁻¹g, JJᵀ 2×2.
                let ((g1, j1), (g2, j2)) = (kept[0], kept[1]);
                let (a, b, c) = (j1.dot(j1), j1.dot(j2), j2.dot(j2));
                let det = a * c - b * b;
                if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
                    return None;
                }
                let l1 = (c * g1 - b * g2) / det;
                let l2 = (a * g2 - b * g1) / det;
                (j1 * l1 + j2 * l2) * -1.0
            }
            3 => {
                let (j1, j2, j3) = (kept[0].1, kept[1].1, kept[2].1);
                solve3(j1, j2, j3, [-kept[0].0, -kept[1].0, -kept[2].0])?
            }
            _ => {
                let mut m = [[0.0f64; 3]; 3];
                let mut rhs = [0.0f64; 3];
                for &(g, j) in &kept {
                    let a = [j.x, j.y, j.z];
                    for i in 0..3 {
                        for k in 0..3 {
                            m[i][k] += a[i] * a[k];
                        }
                        rhs[i] -= a[i] * g;
                    }
                }
                solve3_mat(m, rhs)?
            }
        };
        x = x + delta;
    }
    None
}

/// New positions for every vertex of `faces`, offset `distance` along the
/// INWARD direction (−outward normal; `distance < 0` offsets outward).
///
/// # Errors
/// `UnsupportedShellCorner { at }` where a vertex's offset surfaces do not
/// meet in one point, or the refinement onto the offset claims does not
/// converge.
pub(crate) fn offset_vertices(
    obj: &Object,
    faces: &BTreeSet<FaceId>,
    distance: f64,
) -> Result<BTreeMap<VertexId, Point3>, ShapeError> {
    let mut touched: BTreeSet<VertexId> = BTreeSet::new();
    for &f in faces {
        let face = &obj.faces()[f];
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            for h in obj.loop_half_edges(l) {
                touched.insert(obj.half_edges()[h].origin);
            }
        }
    }
    let mut out = BTreeMap::new();
    for v in touched {
        let p = obj.vertices()[v].position;
        let refuse = ShapeError::UnsupportedShellCorner { at: p };
        let groups = groups_at(obj, faces, v);
        let mut rows = Vec::with_capacity(groups.len());
        let mut normals = Vec::with_capacity(groups.len());
        for g in &groups {
            let (c, n) = group_constraint(obj, g, p).ok_or(refuse)?;
            rows.push(c);
            normals.push(n);
        }
        let x = solve_planes(p, &normals, distance).ok_or(refuse)?;
        let x = if rows.iter().any(|r| matches!(r, Constraint::Claim { .. })) {
            refine(x, &rows, distance).ok_or(refuse)?
        } else {
            x
        };
        out.insert(v, x);
    }
    Ok(out)
}

/// Vertex `v`'s offset point, as [`offset_vertices`] places it, held in
/// `plane` as well: Gauss–Newton on every group's true constraint plus the
/// plane, from `start`. `None` when it does not converge (or a group has
/// no constraint).
pub(crate) fn offset_vertex_in_plane(
    obj: &Object,
    faces: &BTreeSet<FaceId>,
    v: VertexId,
    distance: f64,
    start: Point3,
    plane: &Plane,
) -> Option<Point3> {
    let p = obj.vertices()[v].position;
    let mut rows = Vec::new();
    for g in &groups_at(obj, faces, v) {
        rows.push(group_constraint(obj, g, p)?.0);
    }
    rows.push(Constraint::Fixed {
        n: plane.normal(),
        v: p - plane.normal() * plane.signed_distance(p),
    });
    refine(start, &rows, distance)
}

/// The claim of `face` offset by `distance` inward, or `None` when the face
/// is unclaimed or the offset claim would be degenerate (a radius at or
/// under `POINT_MERGE`; the caller refuses `WallTooThick`).
pub(crate) fn offset_claim(obj: &Object, face: FaceId, distance: f64) -> Option<SurfaceRef> {
    let surface = obj.faces()[face].surface?;
    let s = claim_sign(obj, face, &surface)?;
    let shrink = |r: f64| {
        let r = r - s * distance;
        (r > tol::POINT_MERGE).then_some(r)
    };
    Some(match surface {
        SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius,
        } => SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius: shrink(radius)?,
        },
        SurfaceRef::Sphere { center, radius } => SurfaceRef::Sphere {
            center,
            radius: shrink(radius)?,
        },
        SurfaceRef::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } => SurfaceRef::Torus {
            center,
            axis,
            major_radius,
            minor_radius: shrink(minor_radius)?,
        },
        // The offset of a cone at distance `s·d` inward is the same cone
        // with its apex moved `s·d / sin α` along the axis (into the nappe
        // for an inward offset of a convex cone): with `h' = h − δ`,
        // `D'(x) = D(x) + δ·sin α`, which is zero where `D(x) = −s·d`.
        SurfaceRef::Cone {
            apex,
            axis,
            half_angle,
        } => SurfaceRef::Cone {
            apex: apex + axis * (s * distance / trig::sin(half_angle)),
            axis,
            half_angle,
        },
    })
}

/// The position of patch vertex `i`.
pub(crate) fn patch_pos(obj: &Object, patch: &PolyPatch, i: usize) -> Point3 {
    match patch.verts[i] {
        PVert::Existing(v) => obj.vertices()[v].position,
        PVert::New(p) => p,
    }
}

fn key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

/// Adds a face of offset points to `patch`: as given when its points lie
/// within `PLANE_DIST` of one plane (`plane`, or the outer loop's Newell
/// plane), else split into triangles joined by soft diagonals (its loop
/// edges keep the metadata the caller set). The face, or each triangle,
/// keeps `attrs.surface` only where the claim `holds_on` its points within
/// `claim_tol`. Returns how many faces were added; `None` when a plane
/// cannot be fitted (a collapsed face) or the face cannot be triangulated.
pub(crate) fn push_face(
    obj: &Object,
    patch: &mut PolyPatch,
    outer: Vec<usize>,
    holes: Vec<Vec<usize>>,
    attrs: FaceAttrs,
    plane: Option<Plane>,
    claim_tol: f64,
) -> Option<usize> {
    let pos = |patch: &PolyPatch, i: usize| patch_pos(obj, patch, i);
    let all: Vec<usize> = outer
        .iter()
        .chain(holes.iter().flatten())
        .copied()
        .collect();
    let pts: Vec<Point3> = all.iter().map(|&i| pos(patch, i)).collect();
    let outer_pts: Vec<Point3> = outer.iter().map(|&i| pos(patch, i)).collect();
    let fit = match plane {
        Some(p) => p,
        None => Plane::from_polygon(&outer_pts).ok()?,
    };
    let keep_claim = |pts: &[Point3], plane: &Plane| FaceAttrs {
        surface: attrs.surface.filter(|s| s.holds_on(pts, plane, claim_tol)),
        ..attrs
    };
    if pts
        .iter()
        .all(|&p| fit.signed_distance(p).abs() <= tol::PLANE_DIST)
    {
        let attrs = keep_claim(&pts, &fit);
        patch.add.push(PatchFace {
            outer,
            holes,
            attrs,
            plane,
        });
        return Some(1);
    }
    let hole_pts: Vec<Vec<Point3>> = holes
        .iter()
        .map(|h| h.iter().map(|&i| pos(patch, i)).collect())
        .collect();
    let tris = crate::triangulate::triangulate_face(&outer_pts, &hole_pts, fit.normal()).ok()?;
    let mut loop_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for ring in std::iter::once(&outer).chain(holes.iter()) {
        for k in 0..ring.len() {
            loop_edges.insert(key(ring[k], ring[(k + 1) % ring.len()]));
        }
    }
    let mut n = 0;
    for [a, b, c] in tris {
        let tri = vec![all[a], all[b], all[c]];
        let tri_pts: Vec<Point3> = tri.iter().map(|&i| pos(patch, i)).collect();
        let tri_plane = Plane::from_polygon(&tri_pts).ok()?;
        for k in 0..3 {
            let e = key(tri[k], tri[(k + 1) % 3]);
            if !loop_edges.contains(&e) {
                patch.edge_meta.insert(
                    e,
                    EdgeMeta {
                        soft: true,
                        ..EdgeMeta::default()
                    },
                );
            }
        }
        patch.add.push(PatchFace {
            outer: tri,
            holes: Vec::new(),
            attrs: keep_claim(&tri_pts, &tri_plane),
            plane: Some(tri_plane),
        });
        n += 1;
    }
    Some(n)
}
