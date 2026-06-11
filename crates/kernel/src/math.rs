//! Minimal f64 vector/point/plane math.
//!
//! Hand-rolled by design (decision recorded): zero dependencies,
//! and every tolerance decision is explicit — comparison methods take a
//! tolerance argument; the named constants live in [`crate::tol`].

use std::ops::{Add, Div, Mul, Neg, Sub};

use crate::tol;

/// Error from a math operation that refuses to guess (no silent repair).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    /// Vector shorter than [`tol::NORMALIZE_MIN_LENGTH`]; no direction exists.
    DegenerateVector,
    /// Points do not span a plane (collinear, coincident, or fewer than 3).
    DegeneratePlane,
}

impl std::fmt::Display for MathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MathError::DegenerateVector => {
                write!(f, "vector too short to normalize")
            }
            MathError::DegeneratePlane => {
                write!(f, "points do not span a plane")
            }
        }
    }
}

impl std::error::Error for MathError {}

/// A direction/displacement in 3D, f64 meters.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// X component (meters).
    pub x: f64,
    /// Y component (meters).
    pub y: f64,
    /// Z component (meters).
    pub z: f64,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Constructs a vector from components.
    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// Dot product.
    pub fn dot(self, other: Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Cross product (right-handed).
    pub fn cross(self, other: Vec3) -> Vec3 {
        Vec3 {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// Squared length; cheaper than [`Vec3::length`] for comparisons.
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    /// Euclidean length.
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    /// Unit vector in this direction, or [`MathError::DegenerateVector`] if
    /// the length is below [`tol::NORMALIZE_MIN_LENGTH`].
    pub fn normalized(self) -> Result<Vec3, MathError> {
        let len = self.length();
        if len < tol::NORMALIZE_MIN_LENGTH {
            return Err(MathError::DegenerateVector);
        }
        Ok(self / len)
    }

    /// True if the two vectors differ by less than `tolerance` in length.
    pub fn approx_eq(self, other: Vec3, tolerance: f64) -> bool {
        (self - other).length_squared() <= tolerance * tolerance
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, rhs: f64) -> Vec3 {
        Vec3::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<f64> for Vec3 {
    type Output = Vec3;
    fn div(self, rhs: f64) -> Vec3 {
        Vec3::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// A position in 3D, f64 meters.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point3 {
    /// X coordinate (meters).
    pub x: f64,
    /// Y coordinate (meters).
    pub y: f64,
    /// Z coordinate (meters).
    pub z: f64,
}

impl Point3 {
    /// The origin.
    pub const ORIGIN: Point3 = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Constructs a point from coordinates.
    pub const fn new(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    /// The displacement of this point from the origin.
    pub fn to_vec(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// True if the two points are within `tolerance` of each other.
    pub fn approx_eq(self, other: Point3, tolerance: f64) -> bool {
        (self - other).length_squared() <= tolerance * tolerance
    }
}

impl Sub for Point3 {
    type Output = Vec3;
    fn sub(self, rhs: Point3) -> Vec3 {
        Vec3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Add<Vec3> for Point3 {
    type Output = Point3;
    fn add(self, rhs: Vec3) -> Point3 {
        Point3::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub<Vec3> for Point3 {
    type Output = Point3;
    fn sub(self, rhs: Vec3) -> Point3 {
        Point3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

/// An oriented plane: the set of points `p` with `normal · p == offset`.
///
/// `normal` is always unit length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    normal: Vec3,
    offset: f64,
}

impl Plane {
    /// Best-fit plane through a closed polygon's vertices, oriented by the
    /// winding order (counter-clockwise winding seen from the normal side),
    /// via Newell's method.
    ///
    /// Fails with [`MathError::DegeneratePlane`] if the points do not span a
    /// plane. Planarity of the input is NOT checked here; callers enforce it
    /// against [`tol::PLANE_DIST`].
    pub fn from_polygon(points: &[Point3]) -> Result<Plane, MathError> {
        if points.len() < 3 {
            return Err(MathError::DegeneratePlane);
        }
        let mut n = Vec3::ZERO;
        let mut centroid = Vec3::ZERO;
        for (i, &p) in points.iter().enumerate() {
            let q = points[(i + 1) % points.len()];
            n.x += (p.y - q.y) * (p.z + q.z);
            n.y += (p.z - q.z) * (p.x + q.x);
            n.z += (p.x - q.x) * (p.y + q.y);
            centroid = centroid + p.to_vec();
        }
        let normal = n.normalized().map_err(|_| MathError::DegeneratePlane)?;
        let centroid = centroid / (points.len() as f64);
        Ok(Plane {
            normal,
            offset: normal.dot(centroid),
        })
    }

    /// The unit normal.
    pub fn normal(&self) -> Vec3 {
        self.normal
    }

    /// Signed distance from `p` to the plane (positive on the normal side).
    pub fn signed_distance(&self, p: Point3) -> f64 {
        self.normal.dot(p.to_vec()) - self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_follows_right_hand_rule() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert!(
            x.cross(y)
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
        );
    }

    #[test]
    fn normalize_rejects_degenerate() {
        assert_eq!(Vec3::ZERO.normalized(), Err(MathError::DegenerateVector));
    }

    #[test]
    fn ccw_polygon_normal_points_up() {
        let pts = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let plane = Plane::from_polygon(&pts).unwrap();
        assert!(
            plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
        );
        assert!(plane.signed_distance(Point3::new(0.5, 0.5, 2.0)) > 0.0);
        assert!(plane.signed_distance(Point3::new(0.5, 0.5, 0.0)).abs() <= tol::PLANE_DIST);
    }

    #[test]
    fn collinear_points_do_not_span_a_plane() {
        let pts = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ];
        assert_eq!(Plane::from_polygon(&pts), Err(MathError::DegeneratePlane));
    }
}
