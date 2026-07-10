//! Per-face affine UV fit for COLLADA import (ARCHITECTURE.md extension).
//!
//! Given a face's corner 3D positions (in definition-local or world space) and
//! the corresponding per-corner UV coordinates from a COLLADA `TEXCOORD` source,
//! fit the affine map `uv = s·p + u0, t·p + v0` by least squares.
//!
//! # Algorithm
//!
//! 1. Build a face-plane 2D basis `(e1, e2)` from the Newell normal and the
//!    first edge: `e1 = normalize(p1 - p0)`, `e2 = n × e1`, origin `o = p0`.
//! 2. Express every corner in 2D: `a_i = (p_i - o)·e1`, `b_i = (p_i - o)·e2`.
//! 3. Solve two independent 3-coefficient affine systems:
//!    `u = α·a + β·b + γ`   and   `v = δ·a + ε·b + ζ`
//!    via normal equations on the 2D coords (least-squares, handles n ≥ 3 corners).
//! 4. Lift back to 3D: `s = α·e1 + β·e2`, `u0 = γ − s·o` and symmetrically for t.
//! 5. Residual guard: if `max_i |frame.apply(p_i) − (u_i,v_i)|` exceeds
//!    [`UV_AFFINE_RESIDUAL_TOL`], return `None` (non-affine mapping — fall back
//!    to, don't store a bad frame).

use kernel::{Point3, UvFrame, Vec3, tol};

/// Maximum per-corner UV residual (in UV-space units) beyond which a fitted
/// frame is rejected as non-affine and `None` is returned instead of a bad frame.
///
/// SketchUp's per-face UV mapping is always exactly affine for flat faces.
/// A residual larger than this tolerance would indicate a projective mapping or
/// a floating-point breakdown; in practice this should be well below 1e-4 for
/// valid SketchUp exports.
pub const UV_AFFINE_RESIDUAL_TOL: f64 = 1e-3;

/// Compute the Newell method face normal from a polygon's vertex list.
///
/// Returns a zero vector for degenerate (collinear / fewer than 3 distinct)
/// polygons. The returned vector is NOT unit-length; the caller normalizes.
fn newell_normal(pts: &[Point3]) -> Vec3 {
    let n = pts.len();
    let mut nx = 0.0f64;
    let mut ny = 0.0f64;
    let mut nz = 0.0f64;
    for i in 0..n {
        let cur = pts[i];
        let nxt = pts[(i + 1) % n];
        nx += (cur.y - nxt.y) * (cur.z + nxt.z);
        ny += (cur.z - nxt.z) * (cur.x + nxt.x);
        nz += (cur.x - nxt.x) * (cur.y + nxt.y);
    }
    Vec3::new(nx, ny, nz)
}

/// Fit a [`UvFrame`] from corner positions and corresponding UV coordinates.
///
/// `positions` and `uvs` must be parallel slices of length ≥ 3.
/// Returns `None` if the fit is numerically degenerate (collinear corners) or
/// if the affine residual exceeds [`UV_AFFINE_RESIDUAL_TOL`].
pub fn fit_uv_frame(positions: &[Point3], uvs: &[[f64; 2]]) -> Option<UvFrame> {
    debug_assert_eq!(
        positions.len(),
        uvs.len(),
        "positions and uvs must be parallel"
    );
    if positions.len() < 3 || uvs.len() < 3 {
        return None;
    }

    // Reject non-finite input outright: a single NaN corner poisons the
    // normal-equation sums, and every residual computed from a NaN-carrying
    // frame is itself NaN — which compares FALSE against the residual guard
    // (IEEE-754), so the guard would silently pass a poisoned frame through.
    if positions
        .iter()
        .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()))
        || uvs
            .iter()
            .any(|uv| !(uv[0].is_finite() && uv[1].is_finite()))
    {
        return None;
    }

    // ── Step 1: face-plane orthonormal basis ───────────────────────────────────
    let o = positions[0];
    let raw_normal = newell_normal(positions);
    let raw_len =
        (raw_normal.x * raw_normal.x + raw_normal.y * raw_normal.y + raw_normal.z * raw_normal.z)
            .sqrt();
    if raw_len < tol::NORMALIZE_MIN_LENGTH {
        return None; // degenerate face
    }
    let n = Vec3::new(
        raw_normal.x / raw_len,
        raw_normal.y / raw_len,
        raw_normal.z / raw_len,
    );

    // e1 = normalize(p1 - p0)
    let e1_raw = Vec3::new(
        positions[1].x - o.x,
        positions[1].y - o.y,
        positions[1].z - o.z,
    );
    let e1_len = (e1_raw.x * e1_raw.x + e1_raw.y * e1_raw.y + e1_raw.z * e1_raw.z).sqrt();
    if e1_len < tol::NORMALIZE_MIN_LENGTH {
        return None; // p0 == p1: degenerate
    }
    let e1 = Vec3::new(e1_raw.x / e1_len, e1_raw.y / e1_len, e1_raw.z / e1_len);

    // e2 = n × e1 (already unit since n and e1 are orthonormal unit vectors in
    // the face plane — but n is the Newell normal which may not be exactly
    // orthogonal to e1 for non-planar input; normalise defensively)
    let e2_raw = n.cross(e1);
    let e2_len = (e2_raw.x * e2_raw.x + e2_raw.y * e2_raw.y + e2_raw.z * e2_raw.z).sqrt();
    if e2_len < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let e2 = Vec3::new(e2_raw.x / e2_len, e2_raw.y / e2_len, e2_raw.z / e2_len);

    // ── Step 2: project corners to 2D plane coords ─────────────────────────────
    let m = positions.len();
    let mut a_coords = Vec::with_capacity(m); // (p - o)·e1
    let mut b_coords = Vec::with_capacity(m); // (p - o)·e2
    for p in positions {
        let dx = p.x - o.x;
        let dy = p.y - o.y;
        let dz = p.z - o.z;
        a_coords.push(dx * e1.x + dy * e1.y + dz * e1.z);
        b_coords.push(dx * e2.x + dy * e2.y + dz * e2.z);
    }

    // ── Step 3: least-squares fit of two affine maps ───────────────────────────
    // For each map `q = α·a + β·b + γ` (u-map and v-map), normal equations:
    //   [Σa²   Σab  Σa ] [α]   [Σau]
    //   [Σab   Σb²  Σb ] [β] = [Σbu]
    //   [Σa    Σb   n  ] [γ]   [Σu ]
    let n_f = m as f64;
    let mut sum_aa = 0.0f64;
    let mut sum_ab = 0.0f64;
    let mut sum_bb = 0.0f64;
    let mut sum_a = 0.0f64;
    let mut sum_b = 0.0f64;
    let mut sum_au = 0.0f64;
    let mut sum_bu = 0.0f64;
    let mut sum_u = 0.0f64;
    let mut sum_av = 0.0f64;
    let mut sum_bv = 0.0f64;
    let mut sum_v = 0.0f64;

    for i in 0..m {
        let a = a_coords[i];
        let b = b_coords[i];
        let u = uvs[i][0];
        let v = uvs[i][1];
        sum_aa += a * a;
        sum_ab += a * b;
        sum_bb += b * b;
        sum_a += a;
        sum_b += b;
        sum_au += a * u;
        sum_bu += b * u;
        sum_u += u;
        sum_av += a * v;
        sum_bv += b * v;
        sum_v += v;
    }

    // Solve 3×3 system M·x = r via Cramer's rule (exact for 3 unknowns).
    // M = [[sum_aa, sum_ab, sum_a],
    //      [sum_ab, sum_bb, sum_b],
    //      [sum_a,  sum_b,  n_f ]]
    let det = sum_aa * (sum_bb * n_f - sum_b * sum_b) - sum_ab * (sum_ab * n_f - sum_b * sum_a)
        + sum_a * (sum_ab * sum_b - sum_bb * sum_a);

    if det.abs() < 1e-30 {
        return None; // collinear corners: system degenerate
    }
    let inv_det = 1.0 / det;

    // Solve for u-map (α, β, γ).
    let alpha = inv_det
        * (sum_au * (sum_bb * n_f - sum_b * sum_b) - sum_ab * (sum_bu * n_f - sum_b * sum_u)
            + sum_a * (sum_bu * sum_b - sum_bb * sum_u));
    let beta = inv_det
        * (sum_aa * (sum_bu * n_f - sum_b * sum_u) - sum_au * (sum_ab * n_f - sum_b * sum_a)
            + sum_a * (sum_ab * sum_u - sum_bu * sum_a));
    let gamma = inv_det
        * (sum_aa * (sum_bb * sum_u - sum_b * sum_bu) - sum_ab * (sum_ab * sum_u - sum_bu * sum_a)
            + sum_au * (sum_ab * sum_b - sum_bb * sum_a));

    // Solve for v-map (δ, ε, ζ).
    let delta = inv_det
        * (sum_av * (sum_bb * n_f - sum_b * sum_b) - sum_ab * (sum_bv * n_f - sum_b * sum_v)
            + sum_a * (sum_bv * sum_b - sum_bb * sum_v));
    let epsilon = inv_det
        * (sum_aa * (sum_bv * n_f - sum_b * sum_v) - sum_av * (sum_ab * n_f - sum_b * sum_a)
            + sum_a * (sum_ab * sum_v - sum_bv * sum_a));
    let zeta = inv_det
        * (sum_aa * (sum_bb * sum_v - sum_b * sum_bv) - sum_ab * (sum_ab * sum_v - sum_bv * sum_a)
            + sum_av * (sum_ab * sum_b - sum_bb * sum_a));

    // ── Step 4: lift back to 3D gradient vectors ───────────────────────────────
    // s = α·e1 + β·e2
    let s = Vec3::new(
        alpha * e1.x + beta * e2.x,
        alpha * e1.y + beta * e2.y,
        alpha * e1.z + beta * e2.z,
    );
    // u0 = γ − s·o  (so that s·o + u0 = γ = the constant for origin)
    let u0 = gamma - (s.x * o.x + s.y * o.y + s.z * o.z);

    // t = δ·e1 + ε·e2
    let t = Vec3::new(
        delta * e1.x + epsilon * e2.x,
        delta * e1.y + epsilon * e2.y,
        delta * e1.z + epsilon * e2.z,
    );
    let v0 = zeta - (t.x * o.x + t.y * o.y + t.z * o.z);

    let frame = UvFrame::new(s, t, u0, v0);

    // ── Step 5: residual guard ─────────────────────────────────────────────────
    let mut max_residual = 0.0f64;
    for i in 0..m {
        let fitted = frame.apply(positions[i]);
        let du = fitted[0] - uvs[i][0];
        let dv = fitted[1] - uvs[i][1];
        let r = (du * du + dv * dv).sqrt();
        // A non-finite residual (numerical breakdown despite the input checks
        // above, e.g. overflow to inf) must reject the frame. It cannot be
        // left to the max/threshold comparisons: NaN compares false against
        // both, so it would silently pass the guard.
        if !r.is_finite() {
            return None;
        }
        if r > max_residual {
            max_residual = r;
        }
    }

    if max_residual > UV_AFFINE_RESIDUAL_TOL {
        // Non-affine or degenerate mapping; fall back to world_size projection.
        return None;
    }

    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::Point3;

    /// A known-affine mapping with 4 corners should recover the frame exactly.
    #[test]
    fn fit_known_affine_quad() {
        // XY plane, z=0.  Define a simple affine UV map:
        //   u = 2·x + 0·y + 0·z + 0.5   (s = (2,0,0), u0 = 0.5)
        //   v = 0·x + 3·y + 0·z - 1.0   (t = (0,3,0), v0 = -1.0)
        let pts = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let uvs: Vec<[f64; 2]> = pts
            .iter()
            .map(|p| [2.0 * p.x + 0.5, 3.0 * p.y - 1.0])
            .collect();

        let frame = fit_uv_frame(&pts, &uvs).expect("should fit affine mapping");

        // Each corner should reproduce the source UV within tight tolerance.
        for (p, uv) in pts.iter().zip(uvs.iter()) {
            let fitted = frame.apply(*p);
            assert!(
                (fitted[0] - uv[0]).abs() < 1e-9,
                "u residual at {p:?}: {} vs {}",
                fitted[0],
                uv[0]
            );
            assert!(
                (fitted[1] - uv[1]).abs() < 1e-9,
                "v residual at {p:?}: {} vs {}",
                fitted[1],
                uv[1]
            );
        }
    }

    /// Collinear corners should return None (degenerate system).
    #[test]
    fn fit_collinear_returns_none() {
        let pts = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ];
        let uvs: Vec<[f64; 2]> = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        let result = fit_uv_frame(&pts, &uvs);
        assert!(result.is_none(), "collinear corners must return None");
    }

    /// A non-finite UV (or position) must yield `None`, never a poisoned
    /// frame: NaN residuals compare false against the residual guard, so
    /// without an explicit finiteness check a NaN-carrying frame is
    /// returned as `Some` and corrupts the stored material mapping.
    #[test]
    fn fit_rejects_non_finite_input() {
        let pts = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let good: Vec<[f64; 2]> = pts
            .iter()
            .map(|p| [2.0 * p.x + 0.5, 3.0 * p.y - 1.0])
            .collect();

        let mut nan_uv = good.clone();
        nan_uv[2][0] = f64::NAN;
        assert!(
            fit_uv_frame(&pts, &nan_uv).is_none(),
            "NaN corner UV must be rejected"
        );

        let mut inf_uv = good.clone();
        inf_uv[1][1] = f64::INFINITY;
        assert!(
            fit_uv_frame(&pts, &inf_uv).is_none(),
            "infinite corner UV must be rejected"
        );

        let mut nan_pos = pts.clone();
        nan_pos[1] = Point3::new(f64::NAN, 0.0, 0.0);
        assert!(
            fit_uv_frame(&nan_pos, &good).is_none(),
            "NaN corner position must be rejected"
        );
    }

    /// Frame on a tilted plane should also work.
    #[test]
    fn fit_tilted_triangle() {
        // Triangle in the plane z = x (45° tilt around Y).
        let pts = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        // Identity UV map: u = x, v = y (after projection).
        let uvs: Vec<[f64; 2]> = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let frame = fit_uv_frame(&pts, &uvs).expect("should fit tilted triangle");
        for (p, uv) in pts.iter().zip(uvs.iter()) {
            let fitted = frame.apply(*p);
            assert!(
                (fitted[0] - uv[0]).abs() < 1e-9,
                "u residual: {} vs {}",
                fitted[0],
                uv[0]
            );
            assert!(
                (fitted[1] - uv[1]).abs() < 1e-9,
                "v residual: {} vs {}",
                fitted[1],
                uv[1]
            );
        }
    }
}
