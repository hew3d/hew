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

/// Maximum ratio of an offset miter join's vertex displacement to the
/// offset distance (dimensionless) — the classic 2D stroke miter limit,
/// applied by `offset::offset_loop`.
///
/// A uniform boundary offset moves every EDGE by exactly `|d|`, but a miter
/// VERTEX moves `d / sin(θ/2)` for interior angle θ: as a corner sharpens
/// toward a needle, the join point runs away unboundedly (a 1° dart offsets
/// its tip ~115·|d|; ~1e7·|d| is reachable), producing a loop that passes
/// every simplicity/winding check while being geometrically absurd for a
/// "uniform band" — so the offset REFUSES typed past this ratio rather than
/// emit it (rule 4's posture: a typed refusal, never a silently wrong Ok).
///
/// 20 admits every corner with interior angle ≳ 5.7° (2·asin(1/20)): a 15°
/// corner's ratio is 1/sin(7.5°) ≈ 7.7 and a 30° corner's ≈ 3.9, both
/// comfortably inside — deliberately far more permissive than the
/// stroke-rendering defaults (SVG miterlimit 4, PostScript 10), because a
/// sharp hand-drawn dart is legitimate CAD geometry; only the needle regime
/// whose miter dwarfs the requested distance is refused.
pub const OFFSET_MITER_LIMIT: f64 = 20.0;

/// Guard band for the inference index's pick-cone node test (dimensionless).
///
/// The exact per-candidate test (`cone_test` in the inference crate) accepts
/// a candidate when `acos(depth / dist) <= aperture`, where `depth` (a dot
/// product) and `dist` (a square-rooted squared length) round independently.
/// The computed cosine therefore carries a few ulps of absolute error —
/// bounded by e ≈ 8 · 2⁻⁵³ ≈ 9e-16 across the dot product, squared length,
/// square root, division, and acos. Because cos θ ≈ 1 − θ²/2 near zero,
/// that error is √-amplified in angle: candidates up to
/// √(a² + 2e) − a ≤ √(2e) ≈ 4.2e-8 rad *outside* the true cone of
/// half-angle `a` can pass (when the quotient rounds to exactly 1.0 the
/// computed angle saturates to 0 and passes ANY aperture). Away from zero
/// the same cosine error inflates the admitted cone's tangent by at most a
/// factor of 1 + e/(sin²a · cos a) ≤ 1 + e/CONE_SLACK for every aperture at
/// least CONE_SLACK short of π/2.
///
/// A conservative box-vs-cone test must therefore admit a band outside the
/// mathematically exact cone: `CONE_SLACK · depth` of extra lateral radius
/// plus a `(1 + CONE_SLACK)` factor on the tangent dominates both regimes,
/// provided cone pruning is disabled for apertures within CONE_SLACK of
/// π/2. 1e-7 (≈ 6.7 · √(f64::EPSILON)) leaves ≥ 2× margin over the
/// 4.2e-8 rad saturation bound and ≥ 10× over the multiplicative bound.
pub const CONE_SLACK: f64 = 1e-7;

/// Relative depth skin for snap-occlusion culling in the inference layer
/// (dimensionless fraction). When deciding whether an opaque face hides a snap
/// candidate, a face counts as an occluder only if it lies at least this
/// fraction nearer than the candidate along the ray *to that candidate*. This
/// keeps a candidate's own (coplanar) face — and faces sharing its edge — from
/// self-occluding it, robustly across the full range of scene scales (a fixed
/// absolute skin would misbehave on very large or very small models).
pub const OCCLUSION_REL: f64 = 1e-6;

/// Maximum miter elongation at a Follow Me path joint (dimensionless).
///
/// At a joint turning by angle θ, the profile ring is carried onto the
/// bisector plane by projecting along the incoming segment; every projected
/// displacement is stretched by the classic stroke miter ratio
/// `1 / cos(θ/2)`. As θ approaches a reversal (θ → 180°) that ratio
/// diverges: a joint only 1e-3 rad short of doubling back stretches ~2000×,
/// committing a "valid" watertight solid whose miter spike is thousands of
/// times the model's own scale — geometrically absurd, yet caught by
/// nothing structural (the spike is manifold, and the advance check bounds
/// only the compressed inner side of a bend, never the stretched outer
/// side).
///
/// A joint whose ratio would exceed this limit refuses as
/// `FollowMeError::PathReverses` — a near-reversal is the same physical
/// configuration as an exact reversal, approached in the limit. 8 admits
/// every ordinary bend with wide margin (a 90° turn stretches 1.41×, 135°
/// stretches 2.61×, even 150° only 3.86×) while refusing joints within
/// ~14° of a full reversal, where no usefully bounded miter exists
/// (the follow-me design §3).
pub const FOLLOW_ME_MITER_LIMIT: f64 = 8.0;
