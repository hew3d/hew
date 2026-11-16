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
    ///
    /// The ring spans a plane when its Newell vector (twice its area) is at
    /// least [`tol::NORMALIZE_MIN_LENGTH`] long, or, failing that, when it
    /// is at least [`tol::MIN_FACE_WIDTH`] wide: the Newell length is at
    /// least that times half the ring's perimeter. A ring that passes the
    /// first test gets exactly the plane [`Plane::from_polygon_import`]
    /// gives it. This is the one predicate the builder, the validator and
    /// the file loader share for "a face too small to keep".
    ///
    /// A ring so large that its Newell vector overflows its length (about
    /// 1e77 m across, the vector's components near 1e154) has no plane
    /// either: dividing by an infinite length would give a zero normal, a
    /// plane no face may carry and no file reopens on, so it fails with
    /// [`MathError::DegeneratePlane`] too.
    pub fn from_polygon(points: &[Point3]) -> Result<Plane, MathError> {
        let (n, centroid) = Self::newell(points)?;
        let len = n.length();
        if len == f64::INFINITY {
            return Err(MathError::DegeneratePlane);
        }
        // A NaN length takes the area path too, so a non-finite ring comes
        // out exactly as it always has.
        let normal = if len.is_nan() || len >= tol::NORMALIZE_MIN_LENGTH {
            n.normalized().map_err(|_| MathError::DegeneratePlane)?
        } else {
            // `len > 0.0` keeps an all-coincident ring (perimeter 0) out.
            if len > 0.0 && len >= tol::MIN_FACE_WIDTH * half_perimeter(points) {
                n / len
            } else {
                return Err(MathError::DegeneratePlane);
            }
        };
        Ok(Plane {
            normal,
            offset: normal.dot(centroid),
        })
    }

    /// [`Plane::from_polygon`] with only the area floor: the ring spans a
    /// plane when its Newell vector is at least
    /// [`tol::NORMALIZE_MIN_LENGTH`] long, so the smallest face it keeps is
    /// about a micron square whatever its shape. The mesh importers decide
    /// which foreign faces to drop with this, so a file heals to the same
    /// face set it always has. Everything the kernel itself checks uses
    /// [`Plane::from_polygon`], which accepts every ring this does and gives
    /// it the identical plane, except a ring whose Newell vector overflows
    /// its length: this gives that one a zero normal, so the heal keeps the
    /// face, and the build refuses the mesh typed instead of dropping it.
    pub fn from_polygon_import(points: &[Point3]) -> Result<Plane, MathError> {
        let (n, centroid) = Self::newell(points)?;
        let normal = n.normalized().map_err(|_| MathError::DegeneratePlane)?;
        Ok(Plane {
            normal,
            offset: normal.dot(centroid),
        })
    }


    /// The ring's Newell vector (twice its vector area) and its vertex
    /// centroid, summed in one fixed order so both plane constructors
    /// agree bit for bit.
    fn newell(points: &[Point3]) -> Result<(Vec3, Vec3), MathError> {
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
        Ok((n, centroid / (points.len() as f64)))
    }
}

/// Half the length of the closed ring through `points`, summed in ring
/// order.
fn half_perimeter(points: &[Point3]) -> f64 {
    let mut perimeter = 0.0;
    for (i, &p) in points.iter().enumerate() {
        perimeter += (points[(i + 1) % points.len()] - p).length();
    }
    perimeter / 2.0
}


impl Plane {
    /// Plane through `point` with the given `normal` (which need not be unit
    /// length — it is normalized here).
    ///
    /// Fails with [`MathError::DegenerateVector`] if `normal` is shorter than
    /// [`tol::NORMALIZE_MIN_LENGTH`] (no direction).
    pub fn from_point_normal(point: Point3, normal: Vec3) -> Result<Plane, MathError> {
        let unit = normal.normalized()?;
        Ok(Plane {
            normal: unit,
            offset: unit.dot(point.to_vec()),
        })
    }

    /// The plane's offset: `normal · p` for every point `p` on it.
    pub fn offset(&self) -> f64 {
        self.offset
    }

    /// The same plane facing the other way: the normal and the offset
    /// negated, exactly (no renormalization).
    pub fn flipped(&self) -> Plane {
        Plane {
            normal: -self.normal,
            offset: -self.offset,
        }
    }

    /// The unit normal.
    pub fn normal(&self) -> Vec3 {
        self.normal
    }

    /// Whether `other` is this plane bit for bit: every normal component
    /// and the offset carry the same bits.
    pub(crate) fn same_bits(&self, other: &Plane) -> bool {
        self.normal.x.to_bits() == other.normal.x.to_bits()
            && self.normal.y.to_bits() == other.normal.y.to_bits()
            && self.normal.z.to_bits() == other.normal.z.to_bits()
            && self.offset.to_bits() == other.offset.to_bits()
    }

    /// Signed distance from `p` to the plane (positive on the normal side).
    pub fn signed_distance(&self, p: Point3) -> f64 {
        self.normal.dot(p.to_vec()) - self.offset
    }

    /// A representative point on the plane: the foot of the perpendicular from
    /// the origin (`normal * offset`). Handy for building an in-plane frame.
    pub fn point(&self) -> Point3 {
        let v = self.normal * self.offset;
        Point3::new(v.x, v.y, v.z)
    }

    /// Reconstructs a plane from a normal and offset stored exactly as
    /// serialized (HEW_FILE_FORMAT.md's `[nx, ny, nz, offset]`) — used only
    /// by sketch-plane decode. Takes both fields **verbatim**, unlike
    /// [`Plane::from_point_normal`]: that constructor renormalizes `normal`
    /// and recomputes `offset` via a dot product against a derived point,
    /// which silently drifts by a few ULPs whenever the stored normal is not
    /// *bit-exactly* unit length — routine after a composed rotation
    /// transform — and would otherwise break `Document::save`/`load`'s exact
    /// `state_hash` round-trip for any sketch on such a plane.
    ///
    /// Fails with [`MathError::DegenerateVector`] if `normal`'s length is not
    /// within [`tol::NORMAL_DIRECTION`] of 1 (a corrupt/tampered file) —
    /// rejected typed rather than silently renormalized or accepted as-is.
    pub(crate) fn from_unit_normal_offset(normal: Vec3, offset: f64) -> Result<Plane, MathError> {
        if (normal.length() - 1.0).abs() > tol::NORMAL_DIRECTION {
            return Err(MathError::DegenerateVector);
        }
        Ok(Plane { normal, offset })
    }
}

// Serde as a plain `[x, y, z]` array — the shape every JSON spec at the
// wasm and API boundaries uses for a point or a vector.
impl serde::Serialize for Vec3 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        [self.x, self.y, self.z].serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for Vec3 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
        let [x, y, z] = <[f64; 3]>::deserialize(d)?;
        Ok(Vec3::new(x, y, z))
    }
}

impl serde::Serialize for Point3 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        [self.x, self.y, self.z].serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for Point3 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Point3, D::Error> {
        let [x, y, z] = <[f64; 3]>::deserialize(d)?;
        Ok(Point3::new(x, y, z))
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
    fn from_point_normal_normalizes_and_passes_through_point() {
        // Deliberately non-unit normal; the point must lie on the plane and the
        // normal must come back unit length.
        let p = Point3::new(2.0, 0.0, 5.0);
        let plane = Plane::from_point_normal(p, Vec3::new(0.0, 0.0, 3.0)).unwrap();
        assert!(
            plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
        );
        assert!(plane.signed_distance(p).abs() <= tol::PLANE_DIST);
        // A point one unit along the normal is one unit off the plane.
        assert!((plane.signed_distance(Point3::new(2.0, 0.0, 6.0)) - 1.0).abs() <= tol::PLANE_DIST);
    }

    #[test]
    fn from_point_normal_rejects_zero_normal() {
        assert_eq!(
            Plane::from_point_normal(Point3::ORIGIN, Vec3::ZERO),
            Err(MathError::DegenerateVector)
        );
    }

    #[test]
    fn collinear_points_do_not_span_a_plane() {
        let pts = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ];
        assert_eq!(Plane::from_polygon(&pts), Err(MathError::DegeneratePlane));
        assert_eq!(
            Plane::from_polygon_import(&pts),
            Err(MathError::DegeneratePlane)
        );
    }

    /// A triangle with a base of `base` along x and apex `height` above
    /// its midpoint, `x0` from the origin: 2A = base · height, width (2A
    /// over half the perimeter) just under `height` for a long base.
    fn sliver(x0: f64, base: f64, height: f64) -> [Point3; 3] {
        [
            Point3::new(x0, 0.0, 0.0),
            Point3::new(x0 + base, 0.0, 0.0),
            Point3::new(x0 + base / 2.0, height, 0.0),
        ]
    }

    fn width(pts: &[Point3]) -> (f64, f64) {
        let (n, _) = Plane::newell(pts).unwrap();
        let mut perimeter = 0.0;
        for i in 0..pts.len() {
            perimeter += (pts[(i + 1) % pts.len()] - pts[i]).length();
        }
        (n.length(), n.length() / (perimeter / 2.0))
    }

    #[test]
    fn area_branch_is_unchanged_bit_for_bit() {
        // Just above the area floor: 2A = 1e-6 · 2e-6 = 2e-12.
        let pts = sliver(0.05, 1e-6, 2e-6);
        assert!(width(&pts).0 >= tol::NORMALIZE_MIN_LENGTH);
        let a = Plane::from_polygon(&pts).unwrap();
        let b = Plane::from_polygon_import(&pts).unwrap();
        assert_eq!(a.normal.x.to_bits(), b.normal.x.to_bits());
        assert_eq!(a.normal.y.to_bits(), b.normal.y.to_bits());
        assert_eq!(a.normal.z.to_bits(), b.normal.z.to_bits());
        assert_eq!(a.offset.to_bits(), b.offset.to_bits());
        // An ordinary face, far from the origin, the same.
        let big = [
            Point3::new(123.4, -5.0, 7.0),
            Point3::new(124.4, -5.0, 7.5),
            Point3::new(124.0, -3.0, 8.0),
            Point3::new(123.1, -4.0, 7.2),
        ];
        assert_eq!(Plane::from_polygon(&big), Plane::from_polygon_import(&big));
    }

    #[test]
    fn width_branch_keeps_a_ring_under_the_area_floor() {
        // 0.3 µm high over a 1 µm base, 5 cm out: 2A = 3e-13, under the
        // area floor, but 0.3 µm wide.
        let pts = sliver(0.05, 1e-6, 3e-7);
        let (len, w) = width(&pts);
        assert!(len < tol::NORMALIZE_MIN_LENGTH && w >= tol::MIN_FACE_WIDTH);
        assert_eq!(
            Plane::from_polygon_import(&pts),
            Err(MathError::DegeneratePlane)
        );
        let plane = Plane::from_polygon(&pts).unwrap();
        assert!((plane.normal().length() - 1.0).abs() <= 1e-15);
        assert!(
            plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
        );
        for p in pts {
            assert!(plane.signed_distance(p).abs() <= tol::PLANE_DIST);
        }
    }

    #[test]
    fn width_branch_boundary() {
        // A 10 µm base: just over and just under 10 nm high. Both are far
        // under the area floor (2A ~ 1e-13).
        let above = sliver(0.0, 1e-5, 1.001e-8);
        let below = sliver(0.0, 1e-5, 0.999e-8);
        let (la, wa) = width(&above);
        let (lb, wb) = width(&below);
        assert!(la < tol::NORMALIZE_MIN_LENGTH && lb < tol::NORMALIZE_MIN_LENGTH);
        assert!(wa >= tol::MIN_FACE_WIDTH, "width {wa:e}");
        assert!(wb < tol::MIN_FACE_WIDTH, "width {wb:e}");
        assert!(Plane::from_polygon(&above).is_ok());
        assert_eq!(Plane::from_polygon(&below), Err(MathError::DegeneratePlane));
        // A needle long enough to clear the area floor passes the first
        // branch whatever its width, exactly as before.
        let needle = sliver(0.0, 1.0, 2e-12);
        assert!(width(&needle).0 >= tol::NORMALIZE_MIN_LENGTH);
        assert_eq!(
            Plane::from_polygon(&needle),
            Plane::from_polygon_import(&needle)
        );
        // Coincident points: no perimeter, no plane.
        let p = Point3::new(1.0, 2.0, 3.0);
        assert_eq!(
            Plane::from_polygon(&[p, p, p]),
            Err(MathError::DegeneratePlane)
        );
    }

    #[test]
    fn a_ring_whose_newell_length_overflows_has_no_plane() {
        let square = |s: f64| {
            [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(s, 0.0, 0.0),
                Point3::new(s, s, 0.0),
                Point3::new(0.0, s, 0.0),
            ]
        };
        // 2A = 2e200, finite, but its square (inside the length) is not.
        let huge = square(1e100);
        let (n, _) = Plane::newell(&huge).unwrap();
        assert!(n.z.is_finite() && n.length() == f64::INFINITY);
        assert_eq!(Plane::from_polygon(&huge), Err(MathError::DegeneratePlane));
        // Just under the overflow the plane is the import's, bit for bit.
        let large = square(1e76);
        let plane = Plane::from_polygon(&large).unwrap();
        assert_eq!(plane.normal(), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(Ok(plane), Plane::from_polygon_import(&large));
    }
}
