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

/// The grid `hew.query.signature` rounds vertex coordinates to (meters):
/// 1 µm, in the part's own frame.
pub const SIGNATURE_QUANTUM: f64 = 1e-6;

/// How far apart, on each world axis, a posed vertex of a linked set and
/// the joiner's matching vertex may be for `Document::join_linked` to
/// accept the pose (meters). Two vertices the signature rounds into the
/// same 1 µm cell of a rotated frame can sit up to √3 µm apart along a
/// world axis, so the check allows two quanta: anything the signature
/// calls identical passes, and a pose off by a visible amount does not.
pub const LINK_POSE_MATCH: f64 = 2.0 * SIGNATURE_QUANTUM;

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

/// Smallest sine of the angle between an edge and a face's plane at which
/// the inference layer offers the point where the edge passes through the
/// face (dimensionless).
///
/// The crossing point is `a + (b − a) · da / (da − db)` from the endpoints'
/// signed plane distances, and `|da − db|` is the edge's length times this
/// sine. Each distance carries rounding of order ε · X for coordinates of
/// magnitude X (the plane's offset is one f64), so the point's error along
/// the edge is about ε · X / sine. Below this sine the edge lies in the
/// plane for every practical purpose — an edge 1 m long whose ends are a
/// few nanometres either side of the plane, as a transform chain or a
/// quantized import leaves a flush panel's edge — and the "crossing" is an
/// arbitrary point along it, which must not be offered as a snap. At 1e-6
/// (a fifth of an arc second) the error bound is ε · X / 1e-6, about a
/// tenth of a nanometre at metre scale and some 20 µm at 100 km, and no edge a
/// person draws through a face is anywhere near it.
pub const EDGE_FACE_GRAZING_SINE: f64 = 1e-6;

/// Smallest sine of the angle between two faces' planes at which the
/// inference layer offers the line where the faces meet (dimensionless).
///
/// Two planes meet along the line `p + t · (n₁ × n₂)`, and `|n₁ × n₂|` is
/// this sine. A point of the line is recovered from the planes' offsets
/// divided by that sine squared, so each offset's rounding (of order ε · X
/// for coordinates of magnitude X) moves the line across the faces by about
/// ε · X / sine. Below this sine the planes are parallel for every practical
/// purpose — two faces a quantized import or a transform chain left a few
/// nanoradians from coplanar — and the "line" is an arbitrary stripe of
/// either face, which must not be offered as a snap. The same figure as
/// [`EDGE_FACE_GRAZING_SINE`] for the same reason: at 1e-6 the error bound
/// is a tenth of a nanometre at metre scale, and no two faces a person
/// builds to meet are anywhere near it.
pub const FACE_MEETING_PARALLEL_SINE: f64 = 1e-6;

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

/// Maximum deviation from unit length or from exact perpendicularity a
/// candidate drawing-axes basis vector may carry and still count as
/// orthonormal (dimensionless; `crate::axes::AxesFrame`).
///
/// Matches [`NORMAL_DIRECTION`]'s magnitude (two unit directions differing by
/// less than this are the same direction): the tool-parity axes tool builds
/// its candidate frame the same way — normalizing clicked/snapped directions
/// in f64 — so the same few-ULP budget applies. Loose enough to admit any
/// frame built from genuinely orthogonal input, tight enough to reject a
/// frame that is only approximately orthonormal (rule 4: refused, never
/// silently re-orthonormalized).
pub const AXES_ORTHONORMAL: f64 = 1e-9;

/// Scene camera drift, position components (meters): the eye and target of
/// the live camera must each sit within this of a Scene's captured camera
/// for the Scene to count as still matching (docs/design/scenes.md §3.1).
/// A tween landing or an orbit-and-return leaves float noise far below this;
/// a deliberate move is far above it.
pub const SCENE_CAMERA_POSITION: f64 = 1e-4;

/// Scene camera drift, direction components (dimensionless): the `up`
/// vector's tolerance under the same test.
pub const SCENE_CAMERA_DIRECTION: f64 = 1e-4;

/// Scene camera drift, field of view (degrees).
pub const SCENE_CAMERA_FOV_DEG: f64 = 1e-2;

/// Section-plane normal unit-length tolerance (dimensionless): a stored or
/// supplied normal whose length is further than this from 1 is refused, not
/// renormalized (rule 4). Looser than [`AXES_ORTHONORMAL`] because the
/// normal originates in the app's double-precision normalization of a face
/// normal, not in exact kernel construction.
pub const SECTION_NORMAL_UNIT: f64 = 1e-6;

/// Mend's default weld distance as a fraction of the object's diameter
/// (dimensionless; `repair::mend`): about 2 µm on a 63 mm print — past
/// the 1 µm grid an STL written to three decimals of a millimeter sits
/// on, so the points such a file meant as one meet, and far below any
/// printed feature (a 0.5 mm stroke is 250 times wider).
pub const WELD_DEFAULT_REL: f64 = 3e-5;

/// The shortest weld distance Mend accepts (meters; [`POINT_MERGE`] × 10):
/// anything closer is already one point to the kernel. An object whose
/// planarity tolerance is tighter (native geometry) welds at most that far.
pub const WELD_MIN: f64 = POINT_MERGE * 10.0;
