//! Named tolerance constants (DEVELOPMENT.md rule 6).
//!
//! All kernel lengths are f64 meters. This module is the only place an
//! epsilon literal may appear; everything else refers to these by name.

/// Two points closer than this are considered coincident (meters).
pub const POINT_MERGE: f64 = 1e-9;

/// Maximum distance from a face's plane for a vertex to count as lying on it
/// (meters).
pub const PLANE_DIST: f64 = 1e-9;

/// Planarity tolerance for faces built from *imported* foreign geometry
/// (meters). COLLADA/SketchUp coordinates are f32, so a face the user drew flat
/// arrives up to ~1e-4 m off its best-fit plane purely from single-precision
/// quantization — far past [`PLANE_DIST`]'s nanometer gate, which is meant for
/// f64 geometry built by exact kernel construction. This wider gate accepts such
/// faces as the single planar polygon they represent (preserving editability)
/// while still rejecting genuinely warped surfaces (centimeter-scale and up).
/// Import-only: native ops and booleans keep [`PLANE_DIST`].
pub const IMPORT_PLANE_DIST: f64 = 1e-3;

/// Vectors shorter than this cannot be meaningfully normalized (meters).
pub const NORMALIZE_MIN_LENGTH: f64 = 1e-12;

/// Two unit normals whose difference is shorter than this are considered the
/// same direction (dimensionless).
pub const NORMAL_DIRECTION: f64 = 1e-9;
