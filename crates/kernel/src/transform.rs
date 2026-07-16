//! Affine transforms (3x3 linear part + translation), f64 meters.
//!
//! Unlike the operation stubs around it, this module is implemented eagerly:
//! transforming planes/normals under non-uniform scale is a classic
//! correctness trap (normals map by the **inverse-transpose** of the linear
//! part, not the linear part itself), and the safest place for that knowledge
//! is one tested implementation that nothing else ever hand-rolls.
//!
//! Used by: Object placement in the Document, booleans
//! ([`crate::topo::Object`]-to-Object frame mapping), move/rotate/scale tools.

use crate::math::{Plane, Point3, Vec3};
use crate::tol;

/// Error from constructing or inverting a transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformError {
    /// The linear part has no inverse (determinant ≈ 0); planes and inverses
    /// are undefined.
    Singular,
    /// A rotation axis too short to define a direction
    /// (below [`tol::NORMALIZE_MIN_LENGTH`]).
    DegenerateAxis,
    /// The linear part flips orientation (determinant < 0, e.g. a negative
    /// scale). Baking it into a solid would invert every face's winding /
    /// outward normal, so it is refused rather than silently producing an
    /// inside-out object.
    Reflection,
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransformError::Singular => write!(f, "transform is singular"),
            TransformError::DegenerateAxis => {
                write!(f, "rotation axis too short to define a direction")
            }
            TransformError::Reflection => {
                write!(f, "transform flips orientation (negative determinant)")
            }
        }
    }
}

impl std::error::Error for TransformError {}

/// An affine map `p -> L·p + t` with row-major linear part `L` and
/// translation `t`.
///
/// Covers everything the modeler needs through M2: rigid moves, rotations,
/// uniform and non-uniform scale, and their compositions. Not a projective
/// transform; there is no perspective row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Row-major 3x3 linear part.
    linear: [[f64; 3]; 3],
    /// Translation applied after the linear part.
    translation: Vec3,
}

impl Transform {
    /// The identity map.
    pub const IDENTITY: Transform = Transform {
        linear: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: Vec3::ZERO,
    };

    /// Pure translation by `offset`.
    pub fn translation(offset: Vec3) -> Transform {
        Transform {
            translation: offset,
            ..Transform::IDENTITY
        }
    }

    /// Uniform scale about the origin. A `factor` of 0 constructs fine but is
    /// singular: [`Transform::inverse`] and [`Transform::apply_plane`] will
    /// fail on it.
    pub fn uniform_scale(factor: f64) -> Transform {
        Transform::scale(Vec3::new(factor, factor, factor))
    }

    /// Per-axis scale about the origin.
    pub fn scale(factors: Vec3) -> Transform {
        Transform {
            linear: [
                [factors.x, 0.0, 0.0],
                [0.0, factors.y, 0.0],
                [0.0, 0.0, factors.z],
            ],
            translation: Vec3::ZERO,
        }
    }

    /// Rotation by `angle` radians about `axis` through the origin
    /// (right-handed), via the Rodrigues formula.
    pub fn rotation(axis: Vec3, angle: f64) -> Result<Transform, TransformError> {
        let k = axis
            .normalized()
            .map_err(|_| TransformError::DegenerateAxis)?;
        let (sin, cos) = angle.sin_cos();
        let one_c = 1.0 - cos;
        Ok(Transform {
            linear: [
                [
                    cos + k.x * k.x * one_c,
                    k.x * k.y * one_c - k.z * sin,
                    k.x * k.z * one_c + k.y * sin,
                ],
                [
                    k.y * k.x * one_c + k.z * sin,
                    cos + k.y * k.y * one_c,
                    k.y * k.z * one_c - k.x * sin,
                ],
                [
                    k.z * k.x * one_c - k.y * sin,
                    k.z * k.y * one_c + k.x * sin,
                    cos + k.z * k.z * one_c,
                ],
            ],
            translation: Vec3::ZERO,
        })
    }

    /// Builds an affine transform from a row-major 3×4 matrix
    /// `[m00 m01 m02 tx, m10 m11 m12 ty, m20 m21 m22 tz]`: the first three
    /// entries of each row are the linear part, the fourth is the translation.
    /// The UI boundary passes transforms this way (e.g. a decomposed three.js
    /// matrix). Invertibility/orientation are checked where it is applied
    /// ([`crate::topo::Object::apply_transform`]), not here.
    pub fn from_affine(rows: &[f64; 12]) -> Transform {
        Transform {
            linear: [
                [rows[0], rows[1], rows[2]],
                [rows[4], rows[5], rows[6]],
                [rows[8], rows[9], rows[10]],
            ],
            translation: Vec3::new(rows[3], rows[7], rows[11]),
        }
    }

    /// Unpacks to a row-major 3×4 matrix `[m00 m01 m02 tx, m10 m11 m12 ty, m20
    /// m21 m22 tz]` — the inverse of [`Transform::from_affine`]. The UI boundary
    /// reads instance poses back this way (e.g. to build a three.js matrix).
    pub fn to_affine(&self) -> [f64; 12] {
        let l = &self.linear;
        let t = self.translation;
        [
            l[0][0], l[0][1], l[0][2], t.x, //
            l[1][0], l[1][1], l[1][2], t.y, //
            l[2][0], l[2][1], l[2][2], t.z,
        ]
    }

    /// The transform that applies `self` first, then `second`.
    pub fn then(&self, second: &Transform) -> Transform {
        let a = &second.linear;
        let b = &self.linear;
        let mut linear = [[0.0; 3]; 3];
        for (i, row) in linear.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        Transform {
            linear,
            translation: second.apply_vector(self.translation) + second.translation,
        }
    }

    /// Maps a position: `L·p + t`.
    pub fn apply_point(&self, p: Point3) -> Point3 {
        Point3::ORIGIN + self.apply_vector(p.to_vec()) + self.translation
    }

    /// Maps a displacement/direction: `L·v` (no translation).
    ///
    /// NOT for surface normals — normals must go through
    /// [`Transform::apply_plane`] (inverse-transpose).
    pub fn apply_vector(&self, v: Vec3) -> Vec3 {
        let l = &self.linear;
        Vec3::new(
            l[0][0] * v.x + l[0][1] * v.y + l[0][2] * v.z,
            l[1][0] * v.x + l[1][1] * v.y + l[1][2] * v.z,
            l[2][0] * v.x + l[2][1] * v.y + l[2][2] * v.z,
        )
    }

    /// Maps a plane so that transformed points stay on the transformed plane.
    ///
    /// Internally maps three points spanning the plane and refits, which is
    /// equivalent to the inverse-transpose normal rule and also keeps the
    /// orientation convention (winding) consistent under reflections.
    pub fn apply_plane(&self, plane: &Plane) -> Result<Plane, TransformError> {
        if self.determinant().abs() < tol::NORMALIZE_MIN_LENGTH {
            return Err(TransformError::Singular);
        }
        let n = plane.normal();
        // Offset of the plane recovered through public API: n·p = offset, so
        // signed_distance(origin) = -offset.
        let offset = -plane.signed_distance(Point3::ORIGIN);
        let anchor = Point3::ORIGIN + n * offset;
        let (u, v) = orthonormal_basis(n);
        let triangle = [
            self.apply_point(anchor),
            self.apply_point(anchor + u),
            self.apply_point(anchor + v),
        ];
        Plane::from_polygon(&triangle).map_err(|_| TransformError::Singular)
    }

    /// The uniform scale factor of this map when its linear part is a
    /// similarity (rotation × uniform scale), or `None` otherwise.
    ///
    /// A similarity is exactly the class of maps that keep a circle a circle
    /// (of scaled radius), which is what lets analytic curve/surface
    /// metadata map through a transform instead of being dropped
    /// (the true-curves design). Checked as `Lᵀ·L ≈ s²·I` on the column
    /// vectors, within [`tol::NORMAL_DIRECTION`] relative (dimensionless).
    pub fn similarity_scale(&self) -> Option<f64> {
        let l = &self.linear;
        let col = |j: usize| Vec3::new(l[0][j], l[1][j], l[2][j]);
        let (c0, c1, c2) = (col(0), col(1), col(2));
        let (n0, n1, n2) = (
            c0.length_squared(),
            c1.length_squared(),
            c2.length_squared(),
        );
        let s2 = (n0 + n1 + n2) / 3.0;
        if s2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
            return None;
        }
        let tol_rel = tol::NORMAL_DIRECTION * s2;
        if (n0 - s2).abs() > tol_rel || (n1 - s2).abs() > tol_rel || (n2 - s2).abs() > tol_rel {
            return None;
        }
        if c0.dot(c1).abs() > tol_rel || c0.dot(c2).abs() > tol_rel || c1.dot(c2).abs() > tol_rel {
            return None;
        }
        Some(s2.sqrt())
    }

    /// Determinant of the linear part. Negative means the transform flips
    /// orientation (a reflection); zero means singular.
    pub fn determinant(&self) -> f64 {
        let l = &self.linear;
        l[0][0] * (l[1][1] * l[2][2] - l[1][2] * l[2][1])
            - l[0][1] * (l[1][0] * l[2][2] - l[1][2] * l[2][0])
            + l[0][2] * (l[1][0] * l[2][1] - l[1][1] * l[2][0])
    }

    /// The inverse map, or [`TransformError::Singular`].
    pub fn inverse(&self) -> Result<Transform, TransformError> {
        let det = self.determinant();
        if det.abs() < tol::NORMALIZE_MIN_LENGTH {
            return Err(TransformError::Singular);
        }
        let l = &self.linear;
        // Adjugate / determinant.
        let inv = [
            [
                (l[1][1] * l[2][2] - l[1][2] * l[2][1]) / det,
                (l[0][2] * l[2][1] - l[0][1] * l[2][2]) / det,
                (l[0][1] * l[1][2] - l[0][2] * l[1][1]) / det,
            ],
            [
                (l[1][2] * l[2][0] - l[1][0] * l[2][2]) / det,
                (l[0][0] * l[2][2] - l[0][2] * l[2][0]) / det,
                (l[0][2] * l[1][0] - l[0][0] * l[1][2]) / det,
            ],
            [
                (l[1][0] * l[2][1] - l[1][1] * l[2][0]) / det,
                (l[0][1] * l[2][0] - l[0][0] * l[2][1]) / det,
                (l[0][0] * l[1][1] - l[0][1] * l[1][0]) / det,
            ],
        ];
        let inverse = Transform {
            linear: inv,
            translation: Vec3::ZERO,
        };
        let translation = -inverse.apply_vector(self.translation);
        Ok(Transform {
            linear: inv,
            translation,
        })
    }
}

/// Any two unit vectors completing `n` (assumed unit length) to a
/// right-handed orthonormal frame, with `u × v = n`.
fn orthonormal_basis(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = helper
        .cross(n)
        .normalized()
        .expect("helper is never parallel to a unit n");
    let v = n.cross(u);
    (u, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only roundtrip slack; kernel geometric tolerances live in
    /// `crate::tol`.
    const TEST_EPS: f64 = 1e-9;

    fn sample_points() -> Vec<Point3> {
        vec![
            Point3::ORIGIN,
            Point3::new(1.0, 2.0, 3.0),
            Point3::new(-4.5, 0.25, 7.0),
            Point3::new(100.0, -50.0, 0.001),
        ]
    }

    #[test]
    fn from_affine_packs_rows_as_linear_plus_translation() {
        // A row-major 3x4: scale-by-2 on the diagonal + translation (7,8,9).
        let t = Transform::from_affine(&[
            2.0, 0.0, 0.0, 7.0, //
            0.0, 2.0, 0.0, 8.0, //
            0.0, 0.0, 2.0, 9.0,
        ]);
        let mapped = t.apply_point(Point3::new(1.0, 1.0, 1.0));
        assert!(mapped.approx_eq(Point3::new(9.0, 10.0, 11.0), TEST_EPS));
        assert!((t.determinant() - 8.0).abs() < TEST_EPS);
    }

    #[test]
    fn to_affine_is_the_inverse_of_from_affine() {
        let rows = [
            2.0, 0.5, 0.0, 7.0, //
            0.0, 3.0, 1.0, 8.0, //
            0.0, 0.0, 0.5, 9.0,
        ];
        assert_eq!(Transform::from_affine(&rows).to_affine(), rows);
    }

    #[test]
    fn inverse_roundtrips_points() {
        let t = Transform::translation(Vec3::new(3.0, -1.0, 2.0))
            .then(&Transform::rotation(Vec3::new(1.0, 2.0, 0.5), 1.1).unwrap())
            .then(&Transform::scale(Vec3::new(2.0, 3.0, 0.5)));
        let inv = t.inverse().unwrap();
        for p in sample_points() {
            let back = inv.apply_point(t.apply_point(p));
            assert!(back.approx_eq(p, TEST_EPS), "{p:?} -> {back:?}");
        }
    }

    #[test]
    fn composition_applies_in_order() {
        let scale_then_move =
            Transform::uniform_scale(2.0).then(&Transform::translation(Vec3::new(1.0, 0.0, 0.0)));
        let p = scale_then_move.apply_point(Point3::new(1.0, 1.0, 1.0));
        assert!(p.approx_eq(Point3::new(3.0, 2.0, 2.0), TEST_EPS), "{p:?}");
    }

    #[test]
    fn rotation_is_rigid() {
        let r = Transform::rotation(Vec3::new(0.3, -1.0, 2.0), 0.7).unwrap();
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert!((r.apply_vector(v).length() - v.length()).abs() < TEST_EPS);
        assert!((r.determinant() - 1.0).abs() < TEST_EPS);
    }

    #[test]
    fn degenerate_axis_is_rejected() {
        assert_eq!(
            Transform::rotation(Vec3::ZERO, 1.0).unwrap_err(),
            TransformError::DegenerateAxis
        );
    }

    #[test]
    fn singular_transform_has_no_inverse() {
        assert_eq!(
            Transform::uniform_scale(0.0).inverse().unwrap_err(),
            TransformError::Singular
        );
    }

    /// The trap this module exists for: under non-uniform scale, the naive
    /// "transform the normal like a vector" answer is wrong; points must stay
    /// on the transformed plane.
    #[test]
    fn plane_transform_survives_non_uniform_scale() {
        let polygon = [
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
        ];
        let plane = Plane::from_polygon(&polygon).unwrap();
        let t = Transform::scale(Vec3::new(1.0, 2.0, 4.0))
            .then(&Transform::translation(Vec3::new(0.5, 0.0, -3.0)));
        let mapped = t.apply_plane(&plane).unwrap();
        for p in polygon {
            let d = mapped.signed_distance(t.apply_point(p));
            assert!(d.abs() < TEST_EPS, "point left its plane by {d}");
        }
        // Orientation is preserved: a point on the normal side stays there.
        let above = Point3::new(1.0, 1.0, 1.0);
        assert!(plane.signed_distance(above) > 0.0);
        assert!(mapped.signed_distance(t.apply_point(above)) > 0.0);
    }

    #[test]
    fn plane_transform_rejects_singular() {
        let plane = Plane::from_polygon(&[
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
        ])
        .unwrap();
        let squash = Transform::scale(Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(
            squash.apply_plane(&plane).unwrap_err(),
            TransformError::Singular
        );
    }
}
