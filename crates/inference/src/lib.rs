//! Snapping/inference engine: pure geometry queries over the whole scene.
//!
//! The SketchUp-feel half of the product (ARCHITECTURE.md, risk #2): tools ask
//! "the cursor is along this ray — where does the user *mean*?", and this
//! crate answers with a [`Snap`]: a position, why it snapped, and to what.
//!
//! Boundaries (DEVELOPMENT.md rule 1): UI-free, I/O-free, renderer-free. The UI
//! converts pixels to a [`PickRay`] + cone aperture and draws the cues; this
//! crate only does geometry. It reads kernel Objects but NEVER mutates —
//! inference queries against the whole scene, while geometry merging stays
//! within the active Object (ARCHITECTURE.md).
//!
//! # Priority model
//!
//! When several candidates fall inside the pick cone, the strongest
//! [`SnapKind::rank_group`] wins (the enum's declaration order IS the
//! priority order, strongest first — `SnapKind`'s `Ord` reflects it and tools
//! may rely on that; the rank group collapses only the three *exact named
//! point* kinds and the two *line* kinds, see below). Within one rank group the candidate with the
//! smallest **normalized** angular distance wins; equal-distance ties break
//! toward the stronger `SnapKind`, then toward the one nearest the ray origin
//! (closest to camera).
//!
//! # Gravity (per-kind weighting)
//!
//! Not every kind deserves the same pull. A drawn circle's exact center and
//! its four quadrant points are what a user aims at; the many endpoints and
//! midpoints of the facets approximating that circle are noise around them.
//! [`SnapWeights`] gives each kind a *weight* `w` that scales the pick cone
//! for that kind alone: the kind is admitted out to `w * aperture` from the
//! ray axis, and its angular distance is divided by `w` before ranking. The
//! normalized distance every candidate is ranked by is therefore "the
//! aperture at which this candidate would just have been admitted" — one
//! scale, directly comparable across kinds, and `w = 1` ranks exactly as
//! unweighted (admission still honors the base reach below).
//!
//! Separately from weights, named points — endpoints, crossings, midpoints,
//! centers, quadrants — are always admitted out to [`POINT_SNAP_REACH`] apertures,
//! since a point is a far smaller target than a line ([`SnapWeights::reach`]
//! is the larger of the two). That base reach changes admission only, never
//! the ranking distance.
//!
//! Two guards keep that from turning into a land grab.
//!
//! *Reach never steals.* A candidate admitted only because its weight widened
//! the cone — angular distance past its kind's base reach (the query's own
//! `aperture`, or [`POINT_SNAP_REACH`] of it for a named point) — ranks behind
//! **every** candidate inside the plain aperture, whatever its kind. Gravity
//! reaches into space nothing else was competing for; it never overrules the
//! thing the cursor is actually on. Without this, a circle quadrant two
//! apertures away would beat the face directly under the cursor, and hovering
//! a surface near a circle would yank the cursor off it.
//!
//! *Weighting cannot invert the coarse priority order.* A face would beat a
//! vertex, since an on-face hit is at angular distance zero by construction.
//! So kinds are ranked by [`SnapKind::rank_group`] before distance, and
//! weights trade places only *within* a group. Two groups hold more than
//! one kind: `Endpoint`/`Center`/`Quadrant`, the three kinds that name an
//! exact point of the geometry rather than a derived one — the group the ask
//! is about — and `OnEdge`/`OnIntersectionLine`, a real edge and the virtual
//! one where two owners' faces meet, two lines the nearer of which is the
//! one aimed at. Those are the only places the ordering moves.
//!
//! [`SnapWeights::uniform`] is "precision mode": every weight 1.0, so ranking
//! falls back to raw angular distance and a user working inside a dense
//! cluster can reach a point the default gravity would otherwise swallow.
//! Which key (if any) selects it is entirely the caller's business — this
//! crate knows nothing about keyboards (DEVELOPMENT.md rule 1).
//!
//! # Occlusion
//!
//! The pick cone is a screen-space projection with no depth buffer, so on its
//! own it "sees through" solids: a hidden back edge or vertex (which outranks a
//! face on [`SnapKind`] priority) would beat the visible front face under the
//! cursor. After ranking, `resolve` therefore walks the sorted list and returns
//! the first candidate that is *visible* — not hidden behind an opaque face
//! along the ray to it (see `is_occluded`). Only what you can see can snap,
//! matching SketchUp. A tool-supplied `constraint_plane` is a separate, additive
//! filter (it restricts candidates to the active drawing plane).
//!
//! That is the default [`FacePolicy::Opaque`]. A whole-model view mode that
//! draws faces see-through or not at all changes what "visible" means, so the
//! scene carries a [`FacePolicy`] set once when the mode changes
//! ([`InferenceScene::set_face_policy`]), not a flag on every query:
//!
//! | policy | faces occlude | `OnFace` snaps, `pick_face` |
//! |---|---|---|
//! | [`FacePolicy::Opaque`] (shaded) | yes | yes |
//! | [`FacePolicy::SeeThrough`] (X-ray) | no | yes, nearest face along the ray |
//! | [`FacePolicy::Opaque`] or [`FacePolicy::SeeThrough`] with a [`PickScope`] | as above | yes; `pick_face` returns the nearest in-scope face along the ray, else the nearest |
//! | [`FacePolicy::Absent`] (wireframe) | no | none |
//!
//! Under `SeeThrough` every edge and named point in the model is reachable,
//! hidden or not, and a face is still the target wherever nothing stronger
//! is under the cursor: face candidates all sit at angular distance zero, so
//! the nearest-depth tie-break picks the front one. Under `Absent` faces do
//! not exist for input at all, sketch-region fills included.
//!
//! While something is open for editing, the caller sets the editing scope
//! ([`InferenceScene::set_pick_scope`]): the owners the open edit can touch.
//! Wherever faces are targets (`Opaque` and `SeeThrough`) a
//! [`InferenceScene::pick_face`] then passes over every face outside the
//! scope and lands on the nearest one inside it, so a tool reaches the open
//! component's face through the solid in front of it. The editing view
//! draws everything outside the scope translucent, so what is in scope is
//! visible through it even when the view mode is shaded: what can be seen
//! can be picked. When no in-scope face lies along the ray, the pick is
//! exactly what it is without a scope. Under `Absent` faces are not targets
//! and the scope changes nothing, and snapping ([`InferenceScene::resolve`])
//! and the geometry-only picks ([`InferenceScene::pick_face_geometry_from`])
//! never consult it.
//!
//! # Locking
//!
//! A [`SnapLock`] (shift-lock or arrow keys in SketchUp terms) constrains the
//! result to a line through the query's `anchor`. With a lock active, every
//! candidate is projected onto the locked line before ranking, and the
//! returned snap keeps the candidate's `kind`/`source` so the UI can still
//! say *why* (e.g. "on axis, from endpoint").
//!
//! # Storage and indexing
//!
//! World-object candidates are baked into world space at registration and
//! pruned through a lazily rebuilt AABB BVH (see `index`) before the exact
//! per-candidate tests run. Component-instance candidates are stored **once
//! per definition member**, in definition space ([`InferenceScene::set_def_member`],
//! or [`InferenceScene::set_def_member_deferred`], which takes the member's
//! bounds and rims at registration and extracts its geometry the first time
//! a query reaches one of its placements, through the query's
//! [`MemberSource`]); each placement is a lightweight `(instance, member, pose)` record
//! ([`InferenceScene::add_placement`]), resolved through a two-level walk —
//! a top-level BVH over placement world boxes, descending into the member's
//! persistent definition-space tree with pose-mapped node tests. Exact tests
//! always run in world space on `pose.apply_point(definition_position)`, the
//! same computation per-placement baking used to perform, so query results
//! are unchanged — only registration cost (once per member, not once per
//! placement) and memory (one copy per definition) collapse. Constant-count
//! candidates (guides, world axes/origin) and gesture-scoped ones
//! (sketch/transient segments) stay on a linear walk. The API deliberately
//! hides the storage so the index strategy can change without touching
//! callers. Intersection snaps (`SnapKind::Intersection`) are emitted where
//! a guide line crosses a segment (sketch or object edge) or another guide
//! line — the crossing is precisely why the guide was drawn — where an axis
//! line crosses an edge, and where edges of two different owners (Objects,
//! placements, sketches) truly cross in 3D: separate Objects never merge, so
//! nothing else puts a point there. That last test pairs only the edges
//! through the pick cone, and at most [`CROSSING_PAIR_CAP`] pairs a query.
//! The same tier also holds the point where an edge of one owner passes
//! THROUGH a face of another (a panel's edge entering a wall it is set
//! into, with nothing imprinted on the wall): the edge is on one side of
//! the face's plane at one end and the other side at the other end, and
//! the point where it crosses lies inside the face's outer loop and
//! outside its holes. Edges through the pick cone are paired with the
//! faces the cone reaches, nearest pairs first and at most
//! [`PIERCE_PAIR_CAP`] pairs a query, and the snap carries the edge and the
//! face it pierces ([`Snap::pierced_face`]). Where a face of one owner meets
//! a face of another, the stretch of their planes' intersection line inside
//! both faces is a virtual edge (`SnapKind::OnIntersectionLine`, with a
//! `Midpoint`): the faces the cone reaches are paired across owners, front
//! to back and at most [`MEETING_PAIR_CAP`] pairs a query, and the snap
//! carries both faces and the line's ends ([`Snap::meeting_line`]).

use std::cell::{Cell, OnceCell, Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    AnalyticRim, AxesFrame, EdgeId, FaceId, Guide, GuideId, InstanceId, Object, ObjectId, Plane,
    Point3, SketchCurveId, SketchCurveRim, SketchEdgeId, SketchId, SketchRegionId, SketchVertexId,
    Transform, Vec3, VertexId, tol,
};

mod index;

use index::{Aabb, DefBounds, DefIndex, Members, PruneTolerance, SceneIndex, region_aabb};

/// A picking ray in world space (UI derives it from the camera + cursor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickRay {
    /// Ray start (the camera/near-plane point).
    pub origin: Point3,
    /// Ray direction; need not be normalized.
    pub direction: Vec3,
}

/// Why a position snapped. **Declaration order is priority order, strongest
/// first**; `Ord` follows it (smaller = stronger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    /// Exactly on an existing vertex.
    Endpoint,
    /// On the true center of a drawn circle or arc, derived from the
    /// solid's analytic surface references (`kernel::SurfaceRef`,
    /// the true-curves design) — the exact drawn center, not a facet
    /// artifact. A center beats everything derived (midpoints,
    /// intersections, edges, faces).
    ///
    /// Against an `Endpoint` it is a **peer**, not a subordinate: both sit in
    /// rank group 0 (see [`SnapKind::rank_group`]), so which one wins is
    /// decided by weighted angular distance. A real vertex at the same spot
    /// still wins — equal distance breaks toward the stronger declared kind —
    /// but a center the cursor is merely *near* out-pulls a facet endpoint it
    /// is nearer to, which is the whole point of the gravity model (crate
    /// docs, *Gravity*).
    Center,
    /// On a quadrant point of a drawn circle or arc's rim — the four
    /// cardinal points of the exact analytic circle, offered only over the
    /// angular range the facets actually cover. Derived from the same
    /// surface references as [`SnapKind::Center`], and ranked exactly as it
    /// is: a peer of `Endpoint` in rank group 0, carrying the same gravity
    /// weight, with a center at the same spot still winning on declared
    /// order.
    Quadrant,
    /// On a crossing: a guide line or an axis line through an edge, two
    /// guide lines, two edges of different owners that truly cross in
    /// 3D (both edges in the snap's provenance, see [`Snap::crossed_edge`]),
    /// or an edge of one owner passing through a face of another (the edge
    /// and the face in the provenance, see [`Snap::pierced_face`]).
    /// Screen-space "apparent" crossings of edges that only overlap on
    /// screen are not offered, and neither is an edge that merely ends on
    /// a face or lies in its plane: the first is an [`SnapKind::Endpoint`],
    /// the second has no single point.
    ///
    /// Outranks [`SnapKind::Midpoint`]: every edge has a midpoint, crossings
    /// are rare, so a cursor near both means the crossing (a midpoint a
    /// tenth of a millimeter away no longer steals it). Still below the
    /// exact named points (endpoint, center, quadrant), which are geometry
    /// rather than a construction from two lines.
    Intersection,
    /// On the midpoint of an edge.
    Midpoint,
    /// On the point of a drawn circle or arc's rim where the segment from
    /// the tool's anchor is tangent to the exact analytic circle. Needs an
    /// anchor ([`SnapQuery::anchor`]); offered only over the covered
    /// angular range. Beats bare OnEdge (it is a *specific* point of the
    /// rim) but loses to explicit points (endpoints, quadrants, midpoints,
    /// intersections).
    Tangent,
    /// On a construction guide: a line or point the user placed
    /// deliberately as a drawing aid. Sits ABOVE [`SnapKind::OnAxis`]
    /// (tool-parity playtest-2 review finding D) — a guide is a choice the
    /// user made on purpose; a soft-axis snap is the system merely
    /// GUESSING the user's intended direction, and a guess must not
    /// silently override a deliberate placement, even when both resolve to
    /// the exact same point. As a necessary CONSEQUENCE of sitting above
    /// `OnAxis` (not a separately chosen ordering — `rank_group`'s total
    /// order forces it), a guide now also outranks plain
    /// [`SnapKind::OnEdge`]/[`SnapKind::OnFace`], which `OnAxis` already
    /// outranked. Still loses to every exact point kind above it
    /// (Endpoint/Center/Quadrant/Intersection/Midpoint/Tangent) and to an
    /// EXPLICIT axis/direction hold (`query.lock`, which bypasses this
    /// ranking entirely and always wins regardless of `SnapKind`).
    OnGuide,
    /// On a model axis through the frame origin, OR — the tool-parity
    /// playtest2 soft-axis-inference design §2c — through the query's
    /// `anchor`, when the drag direction from it lands close enough (its
    /// own dedicated angular tolerance, `SOFT_AXIS_APERTURE`, not the
    /// query's point-snap `aperture`) to one of the three frame axes:
    /// SketchUp's "the line turns green and locks softly onto the axis"
    /// feel. Beats plain [`SnapKind::OnEdge`]/[`SnapKind::OnFace`] (a
    /// directional cue the user is actively aiming for outranks merely
    /// being somewhere on a face) — but loses to [`SnapKind::OnGuide`]
    /// (finding D): an INFERRED direction must not beat a DELIBERATE one.
    /// Still loses to every exact point kind above it, including
    /// [`SnapKind::Tangent`].
    OnAxis,
    /// Anywhere along an edge.
    OnEdge,
    /// Anywhere along the line where a face of one owner meets a face of
    /// another: the stretch of the two planes' intersection line that lies
    /// inside both faces (outer loops, outside their holes). Separate
    /// Objects never merge, so the line is no stored edge — it exists only
    /// as geometry — yet it is a line a person sees and aims at (a slab
    /// passing through a wall draws four of them on the wall's face). A
    /// virtual edge: the same cone reach as [`SnapKind::OnEdge`], the same
    /// rank group, and under a real edge at the same spot — equal distance
    /// breaks toward the stronger declared kind. The line runs through both
    /// faces' interiors: where one face only touches the other's plane
    /// along its own edge (a box flush against a wall) there is no line,
    /// only that real edge. The line's ends are the piercings and crossings
    /// of the faces' edges ([`SnapKind::Intersection`]) and its middle is a
    /// [`SnapKind::Midpoint`], both offered as on a real edge — except that
    /// a line on a chord facet of a curved surface ([`SceneFace::surface_facet`])
    /// is one facet's piece of a curve, and offers no middle. Both faces
    /// are in the provenance: the first as `source` (or
    /// `sketch_region_source`), the second with the line's ends in
    /// [`Snap::meeting_line`]. Never offered to a selection pick
    /// ([`SnapWeights::for_picking`]): there is no one entity under the
    /// cursor to select.
    OnIntersectionLine,
    /// Anywhere on a face.
    OnFace,
    /// Direction parallel to a reference edge (M2; needs a reference).
    Parallel,
    /// Direction perpendicular to a reference edge (M2; needs a reference).
    Perpendicular,
}

impl SnapKind {
    /// Every kind, in declaration (priority) order. This is the index space
    /// [`SnapWeights`] stores its weights in.
    pub const ALL: [SnapKind; SnapKind::COUNT] = [
        SnapKind::Endpoint,
        SnapKind::Center,
        SnapKind::Quadrant,
        SnapKind::Intersection,
        SnapKind::Midpoint,
        SnapKind::Tangent,
        SnapKind::OnGuide,
        SnapKind::OnAxis,
        SnapKind::OnEdge,
        SnapKind::OnIntersectionLine,
        SnapKind::OnFace,
        SnapKind::Parallel,
        SnapKind::Perpendicular,
    ];

    /// How many kinds exist (the length of [`SnapKind::ALL`]).
    pub const COUNT: usize = 13;

    /// This kind's slot in [`SnapKind::ALL`]. The `match` is exhaustive, so a
    /// new variant cannot be added without visiting this.
    const fn index(self) -> usize {
        match self {
            SnapKind::Endpoint => 0,
            SnapKind::Center => 1,
            SnapKind::Quadrant => 2,
            SnapKind::Intersection => 3,
            SnapKind::Midpoint => 4,
            SnapKind::Tangent => 5,
            SnapKind::OnGuide => 6,
            SnapKind::OnAxis => 7,
            SnapKind::OnEdge => 8,
            SnapKind::OnIntersectionLine => 9,
            SnapKind::OnFace => 10,
            SnapKind::Parallel => 11,
            SnapKind::Perpendicular => 12,
        }
    }

    /// The coarse priority band this kind competes in (smaller = stronger).
    ///
    /// Ranking is by group first, so gravity ([`SnapWeights`]) can only trade
    /// places *within* a group — it can never let a face outrank a vertex.
    /// Every group holds exactly one kind except two. The first holds the
    /// three kinds naming an **exact point of the geometry**: a vertex
    /// ([`SnapKind::Endpoint`]), a circle's true center
    /// ([`SnapKind::Center`]), and a circle's cardinal points
    /// ([`SnapKind::Quadrant`]). Those three are peers — which one the user
    /// meant is a question of how close they aimed, weighted by how much each
    /// is worth — whereas everything below is derived geometry whose ordering
    /// is a fixed editorial decision (a crossing beats a midpoint beats an
    /// edge beats a face, always). The edge group holds the two kinds of
    /// **line the cursor slides along**: a real edge ([`SnapKind::OnEdge`])
    /// and the line where two owners' faces meet
    /// ([`SnapKind::OnIntersectionLine`]). They are peers for the same
    /// reason — both are lines, and the nearer one is the one aimed at — so
    /// a meeting line two pixels away is not lost to a real edge seven
    /// pixels off; the real edge still wins the equal-distance case below.
    ///
    /// Within a group, equal normalized distance still breaks toward the
    /// stronger `SnapKind`, so a real vertex at the same spot as a center
    /// keeps winning (the invariant [`SnapKind::Center`]'s docs promise),
    /// and a real edge keeps winning over a meeting line that runs along it.
    ///
    /// `OnGuide` and `OnAxis` each sit in their OWN group here, immediately
    /// after `Tangent` and before `OnEdge`, in that order (tool-parity
    /// playtest-2 review finding D) — `OnGuide` first, because a
    /// deliberately-placed guide must beat an INFERRED soft-axis cue, then
    /// `OnAxis`, which still BEATS (a smaller group number than) plain
    /// `OnEdge`/`OnFace` — a directional cue the user is actively aiming for
    /// outranks merely being somewhere on a face or edge. `OnGuide` sitting
    /// above `OnAxis` transitively means it also beats `OnEdge`/`OnFace` —
    /// not a separately chosen ordering, just the unavoidable consequence of
    /// a single total order (the same kind of forced consequence that
    /// previously made `OnAxis` beat `OnGuide`, before this reorder). Both
    /// still lose to every exact point kind above them, including `Tangent`.
    pub const fn rank_group(self) -> u8 {
        match self {
            // The exact-named-point band.
            SnapKind::Endpoint | SnapKind::Center | SnapKind::Quadrant => 0,
            SnapKind::Intersection => 1,
            SnapKind::Midpoint => 2,
            SnapKind::Tangent => 3,
            SnapKind::OnGuide => 4,
            SnapKind::OnAxis => 5,
            // The line band: a real edge and a virtual one (faces meeting).
            SnapKind::OnEdge | SnapKind::OnIntersectionLine => 6,
            SnapKind::OnFace => 7,
            SnapKind::Parallel => 8,
            SnapKind::Perpendicular => 9,
        }
    }

    /// How far, in multiples of the query `aperture`, this kind is admitted
    /// before any gravity weight applies — see [`POINT_SNAP_REACH`].
    pub const fn point_reach(self) -> f64 {
        match self {
            SnapKind::Endpoint
            | SnapKind::Center
            | SnapKind::Quadrant
            | SnapKind::Intersection
            | SnapKind::Midpoint => POINT_SNAP_REACH,
            _ => GRAVITY_NEUTRAL,
        }
    }
}

/// The neutral gravity weight: the kind behaves exactly as it did before
/// weighting existed (admitted within `aperture`, ranked on raw angular
/// distance).
pub const GRAVITY_NEUTRAL: f64 = 1.0;

/// The standard gravity of a drawn curve's analytic center and quadrant
/// points ([`SnapKind::Center`], [`SnapKind::Quadrant`]).
///
/// Two and a half pick radii: against the app's 8 px acquire radius that is a
/// 20 px reach, close to the 16 px radius at which an already-held snap is
/// released — a circle's center and quadrants pull about as far as a held
/// snap resists, which is the "sticky" feel this is after. Large enough to
/// swallow the facet endpoints crowding a quadrant on a 24-segment circle,
/// small enough that aiming squarely at a facet endpoint still gets it (that
/// endpoint's normalized distance is then ~0, and nothing beats zero).
pub const GRAVITY_ANALYTIC_POINT: f64 = 2.5;

/// How much further than the query `aperture` a named point — an endpoint,
/// a crossing, a midpoint, a drawn curve's center or quadrant — is admitted.
///
/// The app derives `aperture` from one pixel radius shared by every kind,
/// but points and lines are not equally easy to hit: a line or axis is a
/// long target the cursor crosses anywhere along, while a point is one
/// spot. With a single radius, a corner sitting on an axis or edge the user
/// is already sliding along was reachable only by landing within that
/// radius exactly, and the sliding snap won everywhere else. 1.75 turns the
/// app's 8 px acquire radius into 14 px for points while lines keep 8 px.
/// This is admission and the "reach never steals" guard only; the ranking
/// still divides by the gravity weight alone, so how a center out-pulls a
/// facet endpoint is unchanged. A gravity weight above this reach (a
/// center's or quadrant's [`GRAVITY_ANALYTIC_POINT`]) still admits out to
/// that weight instead. A held point releases at
/// [`POINT_RELEASE_MARGIN`] past this, not at a line's ratio.
pub const POINT_SNAP_REACH: f64 = 1.75;

/// How far past its [`POINT_SNAP_REACH`] a held point lets go, as a
/// fraction of the hysteresis release query's widening over the acquire
/// radius: 0.5 of the app's 8 px → 16 px widening is 4 px, so a point
/// acquired at 14 px releases at 18 px.
///
/// The release distance is also the dead zone after clicking a point: a
/// tool anchored on a just-clicked endpoint stays on it until the cursor
/// leaves that radius. A line's full ratio would have held a point out to
/// 28 px, and even a line's 8 px margin (22 px) left the cursor visibly
/// stuck on a midpoint well after leaving it. 4 px still stops the cue from
/// flickering at the acquire boundary, and moves the release only 2 px past
/// the 16 px it was before points reached further.
pub const POINT_RELEASE_MARGIN: f64 = 0.5;

/// The largest weight [`SnapWeights::with`] accepts. Weights widen the pick
/// cone, and the spatial index's prune cone widens with them (see
/// `SnapWeights::max_indexed`); past a point that stops being a prune. It is
/// a policy bound on a ranking parameter, not a geometric tolerance.
pub const GRAVITY_MAX: f64 = 8.0;

/// Soft-axis inference's own angular aperture (tool-parity playtest2 §2c),
/// in degrees — the pick-cone half-angle for the anchor-relative
/// [`SnapKind::OnAxis`] candidates `resolve` generates when the query
/// carries an `anchor`. Deliberately its OWN fixed value rather than the
/// query's `aperture` (which the UI derives from an 8px point-snap radius —
/// typically well under a degree, far too tight for "I'm aiming roughly
/// this way"): the point-snap aperture answers "is the cursor pixel-precise
/// on this candidate", soft axis answers "is the drag direction from the
/// anchor roughly aligned with this one" — a categorically wider, purely
/// angular question with no pixel/FOV conversion of its own. 5° sits in the
/// middle of the ~4-6° a directional "soft lock" reads as generous but not
/// promiscuous; picked and documented here rather than left a bare literal.
pub const SOFT_AXIS_APERTURE_DEG: f64 = 5.0;

/// [`SOFT_AXIS_APERTURE_DEG`] in radians — [`cone_test`]'s native unit.
pub const SOFT_AXIS_APERTURE: f64 = SOFT_AXIS_APERTURE_DEG * std::f64::consts::PI / 180.0;

/// The soft axis's on-screen limit, in multiples of the query `aperture`:
/// besides lying within [`SOFT_AXIS_APERTURE`] of the drag direction, the
/// axis line's point nearest the cursor must also be within this many pick
/// radii of it. 1.0 makes the axis capture and let go exactly like an edge
/// (the app's 8 px acquire, 16 px release): an inferred direction gets no
/// more pull on screen than a real line. 1.25 still held a long drag 20 px
/// off the axis, longer than anything else on screen.
///
/// The angle alone is measured from the anchor, so its reach grows with the
/// drag: 5° is a quarter metre at the end of a 3 m drag, and the hysteresis
/// release query's doubled cone half a metre. Near the end of a long drag
/// the axis captured the cursor from across the screen and held it far past
/// any point the user was aiming for — a corner on the axis was reachable
/// only by landing on it pixel-exactly. Capping the reach on screen keeps
/// the angle in charge for short drags, where 5° is a few pixels, and the
/// cap in charge for long ones. Kept below [`POINT_SNAP_REACH`] so a point
/// on the axis always captures before the axis can hold the cursor off it.
pub const SOFT_AXIS_SCREEN_REACH: f64 = 1.0;

/// Candidates whose weighted distances differ by no more than this fraction
/// of the query aperture rank as EQUALLY distant, so the nearest-depth
/// tie-break decides between them (see the ranking sort in `resolve_impl`).
///
/// An eighth of an 8 px aperture is ONE PIXEL's worth of angle (or radius)
/// at any zoom: two candidates the cursor cannot tell apart on screen are
/// the same target, and the one in front is the one the user sees. The
/// previous value, one millionth, only tied points stacked EXACTLY along
/// the ray — and a "Top" view is never exactly vertical (the orbit camera
/// stops a hair short of straight down), so a framed wall's corner stack —
/// top plate, sole plate, slab — fanned out by millimetres in the ray's
/// frame and the DEEPEST one won whenever the tilt happened to lean its
/// way: a Dimension clicked on a wall top in plan view landed on the slab
/// two and a half metres below it, with nothing on screen to say so. Still
/// far above the ~1e-12 noise a placed instance's pose composition
/// introduces, and well under the distance a hand deliberately aims by.
pub const RANK_TIE_FRACTION: f64 = 0.125;

/// Below this angle (degrees) between the pick ray and a candidate soft-axis
/// direction, the axis is treated as too EDGE-ON to trust and no candidate
/// is generated for it at all.
///
/// `closest_point_on_line_to_ray`'s own guard (`tol::NORMALIZE_MIN_LENGTH`)
/// only catches EXACT parallelism; merely NEAR-parallel already makes its
/// result ill-conditioned, because on screen the axis is foreshortened down
/// to a sliver a few pixels long — a pixel of mouse motion then swings the
/// resolved point arbitrarily far along it. Measured as the angle between
/// the (both unit) ray and axis directions, which needs no screen/pixel
/// conversion: a ray nearly parallel to the axis direction is, by
/// construction, sighting almost exactly down it, wherever the camera
/// happens to sit — the same underlying condition "a few pixels of screen
/// length" describes, reached without any screen-space math in this
/// screen-agnostic crate. 3° is comfortably past ordinary drawing angles
/// (a user drawing broadside to an axis is nowhere near this) while still
/// catching the genuinely degenerate "sighting straight down it" case.
/// `axis_is_edge_on` compares against `cos` of this (computed there, not
/// pre-baked as a literal, so changing this constant can never silently
/// drift out of sync with the threshold it drives).
pub const SOFT_AXIS_EDGE_ON_DEG: f64 = 3.0;

/// Fraction of the query `aperture` within which the pick RAY counts as
/// hovering the lock's anchor itself, rather than merely passing near it.
///
/// A locked resolve needs this distinction because the reconstructed
/// cursor ray never re-crosses a hovered point EXACTLY (pixel rounding
/// and unprojection leave a sub-pixel miss), so with the cursor visually
/// ON the anchor the directional fallback's station is that miss
/// verbatim: a noise-scale, noise-SIGNED offset, not a direction the
/// user chose. A tool that derives a typed-entry direction from the
/// resolved station (Move's signed-distance commit, Line's rubber-band
/// direction) would amplify that accident into a full-magnitude move the
/// WRONG WAY. Within this fraction of the aperture, therefore, the
/// no-winner directional fallback returns the anchor exactly (zero
/// displacement — what the gesture indicates), and the wrong-side
/// candidate disqualification is suspended (the fallback station's sign
/// is itself noise there). This is deliberately a fact about the RAY
/// only, consumed only by those two ray-judging rules — the winner
/// cull's noise test compares each CANDIDATE against the fallback point
/// itself, with no aperture band involved, so real geometry keeps
/// winning at its true station however the ray sits relative to the
/// anchor.
///
/// 0.25 — a quarter of the pick aperture, i.e. 2 px of the app's 8 px snap
/// radius — splits the bands with real margin on both sides: the
/// reconstruction noise is at most ~half a pixel (≈ 6% of the aperture)
/// regardless of zoom, while the closest pinned "genuinely aiming near
/// the anchor" case (the rehomed-plane sweep's d = 0.05 step, which must
/// land at the aimed station, not the anchor) puts the anchor ≈ 60% of
/// the aperture off the ray. Being aperture-relative keeps the split
/// zoom-independent — both the noise band and the aperture are
/// pixel-derived quantities.
pub const LOCK_ON_ANCHOR_APERTURE_FRACTION: f64 = 0.25;

/// Per-kind snap gravity — see the crate docs' *Gravity* section.
///
/// A weight `w` for a kind means: admit that kind out to `w * aperture` from
/// the ray axis, and divide its angular distance by `w` before ranking.
/// [`GRAVITY_NEUTRAL`] is the unweighted behavior.
///
/// [`SnapWeights::default`] is the shipped profile (analytic centers and
/// quadrants boosted, everything else neutral); [`SnapWeights::uniform`] is
/// precision mode.
///
/// Weights are a *ranking* parameter, not geometry, and a snap query has no
/// error channel (it answers `Option<Snap>`), so [`SnapWeights::with`]
/// **clamps** its argument into `[GRAVITY_NEUTRAL, GRAVITY_MAX]` (mapping a
/// non-finite value to `GRAVITY_NEUTRAL`) rather than refusing it. That is
/// deliberately not a breach of DEVELOPMENT.md rule 4, which forbids nudging
/// *geometry* to make an operation succeed: no coordinate is touched here,
/// only how far a preference reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapWeights {
    /// Indexed by `SnapKind::index`.
    by_kind: [f64; SnapKind::COUNT],
    /// A selection pick rather than an inference: named points get no
    /// [`POINT_SNAP_REACH`] (see [`SnapWeights::for_picking`]).
    picking: bool,
}

impl SnapWeights {
    /// Every kind neutral — **precision mode**. Ranking within a rank group
    /// falls back to raw angular distance, so the nearest candidate wins and
    /// a point the default gravity would swallow becomes reachable.
    pub const fn uniform() -> SnapWeights {
        SnapWeights {
            by_kind: [GRAVITY_NEUTRAL; SnapKind::COUNT],
            picking: false,
        }
    }

    /// The shipped gravity profile: analytic curve centers and quadrant
    /// points pull at [`GRAVITY_ANALYTIC_POINT`], every other kind neutral.
    ///
    /// Only those two kinds are boosted, deliberately. They are the only
    /// kinds that exist *because* an exact analytic curve was drawn, so
    /// boosting them promotes the points a user aims at without demoting
    /// anything: the facet endpoints and midpoints they have to out-pull are
    /// precisely the candidates crowded around them. Demoting facet points
    /// directly would need a per-candidate "is this a curve facet?" flag the
    /// scene does not carry, and would also weaken a plain line's endpoints,
    /// which nothing asked for.
    pub const fn standard() -> SnapWeights {
        let mut w = SnapWeights::uniform();
        w.by_kind[SnapKind::Center.index()] = GRAVITY_ANALYTIC_POINT;
        w.by_kind[SnapKind::Quadrant.index()] = GRAVITY_ANALYTIC_POINT;
        w
    }

    /// This profile with `kind`'s weight replaced, clamped into
    /// `[GRAVITY_NEUTRAL, GRAVITY_MAX]` (non-finite → [`GRAVITY_NEUTRAL`]).
    pub fn with(mut self, kind: SnapKind, weight: f64) -> SnapWeights {
        let w = if weight.is_finite() {
            weight.clamp(GRAVITY_NEUTRAL, GRAVITY_MAX)
        } else {
            GRAVITY_NEUTRAL
        };
        self.by_kind[kind.index()] = w;
        self
    }

    /// `kind`'s weight.
    pub fn weight(&self, kind: SnapKind) -> f64 {
        self.by_kind[kind.index()]
    }

    /// This profile for a selection pick: named points are admitted at the
    /// plain aperture, like lines, instead of [`POINT_SNAP_REACH`] of it.
    ///
    /// The wider reach is an inference aid — it lets a drawing tool land on
    /// a corner it is sliding toward. A click that SELECTS something asks a
    /// different question, "what is under the cursor", and there a midpoint
    /// 10 px away is not under it: with the reach, the middle of a face
    /// under about 28 px across picked one of its edges instead of the face.
    pub const fn for_picking(mut self) -> SnapWeights {
        self.picking = true;
        self
    }

    /// `kind`'s admission reach before gravity, in multiples of the query
    /// aperture: [`SnapKind::point_reach`], or the plain aperture when
    /// picking ([`SnapWeights::for_picking`]).
    ///
    /// `widening` is how many acquire radii this query's aperture spans —
    /// the app's hysteresis release query runs at twice the acquire radius
    /// and says so in [`SnapQuery::soft_axis_aperture_scale`]; every other
    /// query passes 1. A held point releases [`POINT_RELEASE_MARGIN`] of
    /// the widening past its reach rather than the full ratio (see there).
    pub fn base_reach(&self, kind: SnapKind, widening: f64) -> f64 {
        let point = kind.point_reach();
        if self.picking || point <= GRAVITY_NEUTRAL {
            return GRAVITY_NEUTRAL;
        }
        let w = widening.max(GRAVITY_NEUTRAL);
        (point + (w - GRAVITY_NEUTRAL) * POINT_RELEASE_MARGIN) / w
    }

    /// How far, in multiples of the query aperture, `kind` is admitted: its
    /// gravity weight, but never less than its [`SnapWeights::base_reach`].
    pub fn reach(&self, kind: SnapKind, widening: f64) -> f64 {
        self.weight(kind).max(self.base_reach(kind, widening))
    }

    /// The largest reach ([`SnapWeights::reach`]) among the kinds the
    /// spatial index prunes for.
    ///
    /// The index prunes points and segments with a cone of half-angle
    /// `aperture`; a weighted kind is admitted out to `w * aperture`, so the
    /// prune cone must be widened by this factor or a boosted candidate the
    /// exact test would have accepted is discarded before it is ever tested —
    /// and `resolve` would stop agreeing with `resolve_linear`.
    ///
    /// The kinds concerned are exactly those sourced from indexed candidate
    /// sets: [`SnapKind::Endpoint`] (points), [`SnapKind::Midpoint`] and
    /// [`SnapKind::OnEdge`] (segments), [`SnapKind::Intersection`] (guide ×
    /// indexed-segment crossings, and the cone-pruned faces of the piercing
    /// walk), and [`SnapKind::OnIntersectionLine`] (the cone-pruned faces of
    /// the meeting-line walk). `OnFace` is absent on purpose: faces are
    /// pruned by ray crossing for it, not by the cone, so no aperture scales
    /// them. Centers, quadrants, tangents, guides and axes are absent
    /// because they are never indexed — they are walked linearly, which is
    /// why the shipped profile's boost costs the prune nothing.
    fn max_indexed(&self, widening: f64) -> f64 {
        [
            SnapKind::Endpoint,
            SnapKind::Midpoint,
            SnapKind::OnEdge,
            SnapKind::Intersection,
            SnapKind::OnIntersectionLine,
        ]
        .into_iter()
        .map(|k| self.reach(k, widening))
        .fold(GRAVITY_NEUTRAL, f64::max)
    }
}

impl Default for SnapWeights {
    fn default() -> SnapWeights {
        SnapWeights::standard()
    }
}

/// The scene element a snap derives from, for highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRef {
    /// A kernel vertex.
    Vertex(VertexId),
    /// A kernel edge.
    Edge(EdgeId),
    /// A kernel face.
    Face(FaceId),
}

/// Which Object (and which element of it) produced a snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapSource {
    /// The geometry-owning Object: a world solid, or — when `instance` is set —
    /// the component definition member the element belongs to (so kernel ops
    /// route correctly via [`kernel::Document::apply_def_op`]).
    pub object: ObjectId,
    /// The element within that Object.
    pub element: ElementRef,
    /// The placing component instance, if this candidate came from one:
    /// `None` for a plain world object, `Some` for instanced geometry. Lets two
    /// instances of one definition coexist without colliding, and tells the UI
    /// which placement was hit (for selection/highlight).
    pub instance: Option<InstanceId>,
}

impl SnapSource {
    /// Whether this candidate's owner is inside `scope`: a placement's
    /// candidate by its instance, a world object's by its object id (crate
    /// docs, *Occlusion*). Membership only, so a stale id in the scope
    /// matches nothing.
    pub fn in_scope(&self, scope: &PickScope) -> bool {
        match self.instance {
            Some(i) => scope.instances.contains(&i),
            None => scope.objects.contains(&self.object),
        }
    }
}

/// A resolved snap: where the cursor should land and why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    /// The snapped position in world space.
    pub position: Point3,
    /// What kind of snap this is (drives the cue color/glyph).
    pub kind: SnapKind,
    /// Provenance for highlighting; `None` for pure-direction snaps like
    /// [`SnapKind::OnAxis`].
    pub source: Option<SnapSource>,
    /// Committed-sketch provenance: which sketch edge a Midpoint/OnEdge snap
    /// derives from, when it came from a sketch rather than an Object.
    /// Mutually exclusive with `source`. Lets tools use a sketch edge as a
    /// reference (Tape Measure parallel guides) without object plumbing.
    pub sketch_source: Option<(SketchId, SketchEdgeId)>,
    /// Committed sketch-region provenance: which region an `OnFace` snap
    /// derives from, when the cursor is on a drawn region's fill rather than a
    /// solid's face. Mutually exclusive with `source`/`sketch_source`. Lets a
    /// tool resolve a click on a region's fill to that exact region — so the
    /// Select tool's click can match the occlusion-aware hover cue (interior
    /// fill → that region; nearer region beats a solid behind it).
    pub sketch_region_source: Option<(SketchId, SketchRegionId)>,
    /// Committed sketch-CURVE provenance: which drawn curve chain a
    /// Center/Quadrant/Tangent snap derives from — the analytic points a
    /// drawn (unextruded) circle, arc, or regular polygon publishes about
    /// itself. Mutually exclusive with the three fields above.
    ///
    /// Deliberately its own field rather than a
    /// [`sketch_source`](Snap::sketch_source) with some representative facet
    /// edge attached: a circle's CENTER lies on no edge at all, and
    /// `sketch_source` is consumed as a *direction* reference (Tape Measure's
    /// parallel guides), so a stand-in edge there would be a lie with a
    /// real consumer. Sitting alongside `sketch_region_source` as its own
    /// thing is the established pattern.
    ///
    /// Lets the Select tool resolve a center/quadrant click to the curve the
    /// point belongs to — the same entity clicking the curve's rim selects —
    /// instead of falling through to a ray re-probe that lands on whatever
    /// region happens to be under the cursor.
    pub sketch_curve_source: Option<(SketchId, SketchCurveId)>,
    /// The inference direction for directional snaps (axis / parallel /
    /// perpendicular), for drawing the dashed guide line.
    pub direction: Option<Vec3>,
    /// Under an axis lock, the candidate's own position BEFORE it was
    /// projected onto the locked line (`position` is the projection). The
    /// UI draws the tie between the two so a "Midpoint on red axis" reads
    /// as the midpoint it came from, not a point that happens to lie on
    /// the lock. `None` for every unprojected snap and for the bare lock
    /// fallback.
    pub projected_from: Option<Point3>,
    /// The SECOND edge of an edge × edge crossing ([`SnapKind::Intersection`]
    /// where edges of two different owners pass through each other): the
    /// first edge is in [`source`](Snap::source) or
    /// [`sketch_source`](Snap::sketch_source) as usual, this is the other
    /// one. `None` for every other snap, including the guide and axis
    /// crossings (those cross a line, not a second edge).
    pub crossed_edge: Option<CrossedEdge>,
    /// The face an edge passes THROUGH ([`SnapKind::Intersection`] where an
    /// edge of one owner pierces a face of another): the edge is in
    /// [`source`](Snap::source) or [`sketch_source`](Snap::sketch_source)
    /// as usual — the point lies on it — and this is the face. `None` for
    /// every other snap. Lets a tool adopt the pierced face (its plane, its
    /// owner) the way it adopts the face an `OnFace` snap names.
    pub pierced_face: Option<PiercedFace>,
    /// The line where a face of one owner meets a face of another
    /// ([`SnapKind::OnIntersectionLine`], or the [`SnapKind::Midpoint`] of
    /// that line): its two ends and the SECOND face. The first face is in
    /// [`source`](Snap::source) or
    /// [`sketch_region_source`](Snap::sketch_region_source) as usual — the
    /// point lies on it — and this is the other one, with the stretch of
    /// line the point lies on, so a cue can draw the virtual edge and a
    /// tool can adopt either face's plane. `None` for every other snap.
    pub meeting_line: Option<MeetingLine>,
}

/// The line where two owners' faces meet (see [`Snap::meeting_line`]): the
/// stretch of the planes' intersection line inside both faces, and the
/// second of the two faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeetingLine {
    /// One end of the stretch, in world space.
    pub a: Point3,
    /// The other end.
    pub b: Point3,
    /// The second face; the first is the snap's `source` or
    /// `sketch_region_source`.
    pub face: PiercedFace,
}

/// A face named by a snap's provenance beyond its first: a solid's face
/// (world or placed) or a committed sketch region's fill — the face of an
/// edge × face piercing ([`Snap::pierced_face`]) or the second face of a
/// face × face meeting line ([`MeetingLine::face`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiercedFace {
    /// An Object's face (world or placed).
    Object(SnapSource),
    /// A committed sketch region's face.
    SketchRegion(SketchId, SketchRegionId),
}

/// One edge of an edge × edge crossing: an Object edge or a committed
/// sketch edge (see [`Snap::crossed_edge`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossedEdge {
    /// An Object's edge (world or placed).
    Object(SnapSource),
    /// A committed sketch's edge.
    Sketch(SketchId, SketchEdgeId),
}

/// The most edge pairs one query tests for edge × edge crossings.
///
/// The pair test runs over the edges that pass through the pick cone. That
/// is a handful on open geometry, but a vertex where many Objects meet (a
/// framed wall's corner) or a dense import seen from afar puts hundreds of
/// edges in the cone, and pairs grow with the square. A query therefore
/// takes at most this many steps of its pair walk, nearest edges first (by
/// angular distance from the ray), so the crossing under the cursor is
/// still found. Each step tests one cross-owner pair or steps over a run
/// of one owner's own edges, so a capped query tests at most this many
/// pairs (exactly this many when no such run lies in its way); the pairs
/// it leaves untested are counted in [`InferenceScene::crossing_stats`]
/// and traced at debug level, never silent. Edges of one owner never pair,
/// so a cone full of one Object's edges costs no steps at all. 2,000 pairs
/// is every pair of about 63 edges.
pub const CROSSING_PAIR_CAP: usize = 2000;

/// The most edge × face pairs one query tests for a piercing.
///
/// The piercing test pairs the edges through the pick cone (the same pool
/// the edge × edge test uses) with the faces the cone reaches — faces some
/// point of whose material or boundary passes within the pick cone, world,
/// placed and drawn-region faces alike. Both sets are a handful on open
/// geometry, but a cone through a dense import can hold hundreds of each,
/// and pairs grow with the product. A query therefore tests at most this
/// many cross-owner pairs, in order of the pair's distance from the ray —
/// the nearer of edge and face deciding nothing, the farther deciding the
/// order (`walk_pierce_pairs`) — so the piercing under the cursor, whose
/// edge and face both pass close to the ray, is tested before pairs with
/// anything farther off, and a cap cuts off the farthest pairs first. The
/// pairs a capped query leaves untested are counted in
/// [`InferenceScene::crossing_stats`] and traced at debug level, never
/// silent. A face pairs with no edge of its own owner, and a face no edge
/// in the pool is foreign to is never pooled, so a cone full of one Object
/// pools and tests nothing. The same budget as [`CROSSING_PAIR_CAP`], spent
/// separately: a query may do both walks.
pub const PIERCE_PAIR_CAP: usize = 2000;

/// The most face × face pairs one query tests for a meeting line.
///
/// The meeting-line test pairs the faces the pick cone reaches (the same
/// pool the piercing test draws on, admitted at the larger of
/// [`SnapKind::OnIntersectionLine`]'s and [`SnapKind::Midpoint`]'s reach)
/// with each other, across owners. A handful on open geometry; a cone through a dense import can
/// hold hundreds, and pairs grow with the square. A query therefore tests
/// at most this many cross-owner pairs, the faces taken front to back
/// along the ray — every pair among the first `j` faces before any pair
/// with the next (`walk_cross_owner_pairs`, the edge × edge walk's order)
/// — so the line in view, whose faces are the first the ray meets, is
/// tested before pairs of faces hidden behind it, however many there are
/// and whatever order the scene was built in, and a cap cuts off the
/// farthest pairs first. The pairs a capped query leaves untested are counted in
/// [`InferenceScene::crossing_stats`] and traced at debug level, never
/// silent. Faces of one owner never pair (a solid's own faces meet at its
/// own edges), so a cone full of one Object tests nothing, and nothing is
/// pooled unless faces of two owners are in the cone. The same budget as
/// [`CROSSING_PAIR_CAP`] and [`PIERCE_PAIR_CAP`], spent separately.
pub const MEETING_PAIR_CAP: usize = 2000;

/// Cumulative edge × edge crossing, edge × face piercing and face × face
/// meeting counters for one scene (see [`InferenceScene::crossing_stats`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrossingStats {
    /// Cross-owner edge pairs tested for a crossing.
    pub pairs_tested: u64,
    /// Steps the pair walk took: every pair tested, plus every step over a
    /// run of one owner's own edges (which pair with nothing). Each query
    /// takes at most [`CROSSING_PAIR_CAP`].
    pub pair_steps: u64,
    /// Queries the cap stopped with cross-owner pairs still untested: each
    /// tested as many pairs as its [`CROSSING_PAIR_CAP`] steps reached
    /// (exactly the cap when the pool holds no same-owner run to step
    /// over) and counted the rest in `pairs_skipped`.
    pub queries_capped: u64,
    /// Cross-owner edge pairs those capped queries left untested.
    pub pairs_skipped: u64,
    /// Faces pooled for the piercing walk: faces the pick cone reaches
    /// whose owner some pooled edge is foreign to. Zero whenever no edge is
    /// in the cone or every edge and face there belongs to one owner — the
    /// walk then pools (and pays for) nothing.
    pub pierce_faces_pooled: u64,
    /// Cross-owner edge × face pairs tested for a piercing. Each query
    /// tests at most [`PIERCE_PAIR_CAP`].
    pub pierce_pairs_tested: u64,
    /// Queries [`PIERCE_PAIR_CAP`] stopped with cross-owner edge × face
    /// pairs still untested.
    pub pierce_queries_capped: u64,
    /// Cross-owner edge × face pairs those capped queries left untested.
    pub pierce_pairs_skipped: u64,
    /// Faces pooled for the meeting-line walk: faces the pick cone reaches
    /// at the larger of [`SnapKind::OnIntersectionLine`]'s and
    /// [`SnapKind::Midpoint`]'s reach, counted whenever the walk builds its
    /// pool. It builds one only when the boxes of faces of two owners or
    /// more meet the cone — solid faces through the spatial index, drawn
    /// regions through their own boxes — so hovering one owner's faces,
    /// with any number of other owners' faces and drawn regions elsewhere
    /// in the scene, pools (and pays for) nothing. (The linear reference,
    /// `InferenceScene::resolve_linear`, has no boxes and gates on every
    /// owner in the scene: it may pool where the indexed path does not,
    /// with the same snap.)
    pub meeting_faces_pooled: u64,
    /// Cross-owner face × face pairs tested for a meeting line. Each query
    /// tests at most [`MEETING_PAIR_CAP`].
    pub meeting_pairs_tested: u64,
    /// Queries [`MEETING_PAIR_CAP`] stopped with cross-owner face × face
    /// pairs still untested.
    pub meeting_queries_capped: u64,
    /// Cross-owner face × face pairs those capped queries left untested.
    pub meeting_pairs_skipped: u64,
}

/// Internal candidate provenance: an Object element, a committed sketch edge,
/// a committed sketch region, or a committed sketch curve chain. Split back
/// into the [`Snap`] provenance fields when the winning candidate becomes a
/// snap.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Provenance {
    Object(SnapSource),
    SketchEdge(SketchId, SketchEdgeId),
    SketchRegion(SketchId, SketchRegionId),
    SketchCurve(SketchId, SketchCurveId),
    /// An edge × edge crossing: the first edge, then the second.
    Crossing(CrossedEdge, CrossedEdge),
    /// An edge × face piercing: the edge, then the face it passes through.
    Pierce(CrossedEdge, PiercedFace),
    /// A face × face meeting line: the first face, then the line with the
    /// second.
    Meeting(PiercedFace, MeetingLine),
}

/// The four mutually-exclusive [`Snap`] provenance fields, plus the
/// crossing's second edge ([`Snap::crossed_edge`]), the piercing's face
/// ([`Snap::pierced_face`]) and the meeting line ([`Snap::meeting_line`]).
type SplitProvenance = (
    Option<SnapSource>,
    Option<(SketchId, SketchEdgeId)>,
    Option<(SketchId, SketchRegionId)>,
    Option<(SketchId, SketchCurveId)>,
    Option<CrossedEdge>,
    Option<PiercedFace>,
    Option<MeetingLine>,
);

impl Provenance {
    /// Split into the public [`Snap`] provenance fields (object, sketch edge,
    /// sketch region, sketch curve) — exactly one is `Some` for a real
    /// candidate — and the crossed edge, `Some` only for an edge × edge
    /// crossing (whose first edge fills the object or sketch-edge field),
    /// the pierced face, `Some` only for an edge × face piercing (whose
    /// edge fills that field the same way), and the meeting line, `Some`
    /// only for a face × face meeting (whose first face fills the object or
    /// sketch-region field).
    fn split(this: Option<Provenance>) -> SplitProvenance {
        // The first edge of a crossing or the edge of a piercing fills the
        // object or sketch-edge field: the point lies on it.
        let edge = |e: CrossedEdge| match e {
            CrossedEdge::Object(s) => (Some(s), None),
            CrossedEdge::Sketch(sid, eid) => (None, Some((sid, eid))),
        };
        match this {
            Some(Provenance::Object(s)) => (Some(s), None, None, None, None, None, None),
            Some(Provenance::SketchEdge(sid, eid)) => {
                (None, Some((sid, eid)), None, None, None, None, None)
            }
            Some(Provenance::SketchRegion(sid, rid)) => {
                (None, None, Some((sid, rid)), None, None, None, None)
            }
            Some(Provenance::SketchCurve(sid, cid)) => {
                (None, None, None, Some((sid, cid)), None, None, None)
            }
            Some(Provenance::Crossing(first, second)) => {
                let (object, sketch) = edge(first);
                (object, sketch, None, None, Some(second), None, None)
            }
            Some(Provenance::Pierce(e, face)) => {
                let (object, sketch) = edge(e);
                (object, sketch, None, None, None, Some(face), None)
            }
            // The first face fills the object or sketch-region field: the
            // point lies on it (as on the second).
            Some(Provenance::Meeting(first, line)) => match first {
                PiercedFace::Object(s) => (Some(s), None, None, None, None, None, Some(line)),
                PiercedFace::SketchRegion(sid, rid) => {
                    (None, None, Some((sid, rid)), None, None, None, Some(line))
                }
            },
            None => (None, None, None, None, None, None, None),
        }
    }
}

/// The three model axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// +X (red in SketchUp tradition).
    X,
    /// +Y (green).
    Y,
    /// +Z (blue).
    Z,
}

impl Axis {
    /// The unit vector of this axis.
    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::new(1.0, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, 1.0, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, 1.0),
        }
    }
}

/// A direction constraint for the current tool gesture (see module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapLock {
    /// Locked to a model axis through the anchor.
    Axis(Axis),
    /// Locked to an arbitrary direction through the anchor (e.g. "hold to
    /// keep this inference").
    Direction(Vec3),
}

/// How [`SnapQuery::aperture`] is interpreted (docs/design/camera.md §1).
/// Perspective projection's apparent size shrinks with depth, so its natural
/// pick tolerance is an angular CONE around the ray — a candidate at twice
/// the depth gets twice the world-space slack, matching how twice as much
/// world moves under one screen pixel there. Parallel (orthographic)
/// projection's apparent size is depth-INDEPENDENT, so the matching pick
/// tolerance is a constant-world-radius CYLINDER around the ray instead —
/// using the cone there would make distant geometry ever harder to snap even
/// though it looks no smaller on screen.
///
/// The two coincide exactly for a candidate at the query's own reference
/// depth (typically the current orbit target's distance): cylinder radius =
/// `target_distance · tan(cone_aperture)` reproduces the SAME world-space
/// tolerance the cone gives at that one depth. They diverge for any OTHER
/// depth — nearer/farther candidates get a wider/narrower cone under `Cone`,
/// but the identical radius under `Cylinder` (see
/// `cylinder_cone_equivalence_at_target_distance` and
/// `cylinder_cone_diverge_off_target_depth` below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApertureMode {
    /// `aperture` is a half-angle in radians (perspective).
    #[default]
    Cone,
    /// `aperture` is a world-space radius in meters (parallel projection).
    Cylinder,
}

/// One inference request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapQuery {
    /// The pick ray under the cursor.
    pub ray: PickRay,
    /// The tool's fixed point (e.g. a line's first endpoint). Required for
    /// axis/parallel/perpendicular inference; point snaps work without it.
    pub anchor: Option<Point3>,
    /// Active direction lock, if any.
    pub lock: Option<SnapLock>,
    /// Pick tolerance around the ray, interpreted per [`ApertureMode`]:
    /// a half-angle in radians (`Cone`) or a world-space radius in meters
    /// (`Cylinder`). The UI computes it from its snap radius in pixels and
    /// either the camera FOV (`Cone`) or `CameraRig::worldPerPixel`
    /// (`Cylinder`), keeping this crate screen-agnostic either way.
    pub aperture: f64,
    /// Which of the two `aperture` means (default `Cone` — see
    /// [`ApertureMode`]).
    pub aperture_mode: ApertureMode,
    /// Active drawing-plane constraint. When `Some`, only candidates whose
    /// position lies on this plane (within [`tol::PLANE_DIST`]) are considered —
    /// drawing on a face must not "see through" the solid and snap to occluded,
    /// off-plane geometry (hidden edges/midpoints/vertices). `OnFace` candidates
    /// then naturally collapse to the coplanar (active) face. `None` keeps the
    /// unconstrained behavior (free-space / ground drawing).
    pub constraint_plane: Option<Plane>,
    /// Per-kind snap gravity for this query (see the crate docs' *Gravity*
    /// section). [`SnapWeights::default`] is the shipped profile;
    /// [`SnapWeights::uniform`] is precision mode. It lives on the query, not
    /// on the scene, because it is a property of the gesture in progress — a
    /// held modifier key, a tool that wants quadrants emphasized — and two
    /// queries against one scene may legitimately want different answers.
    /// This crate never learns what selects it (DEVELOPMENT.md rule 1).
    pub weights: SnapWeights,
    /// Multiplies [`SOFT_AXIS_APERTURE`] for THIS query's anchor-relative
    /// soft-axis candidates only (tool-parity playtest-2 review finding E) —
    /// `None` (or a non-finite/non-positive value) behaves exactly as
    /// before, i.e. `SOFT_AXIS_APERTURE` unscaled. `OnAxis` is in the app's
    /// `STICKY_KINDS` (`snapService.ts`), but the origin-relative widening
    /// its acquire/release hysteresis normally applies works by re-querying
    /// at a WIDER `aperture` — which this candidate never reads (it has its
    /// own fixed angular tolerance, deliberately not `aperture`; see that
    /// constant's doc). Without a dedicated knob a held soft-axis snap had
    /// no hysteresis at all: it dropped the instant the cursor left the
    /// unscaled 5° cone, unlike every other sticky kind. The app's release
    /// query sets this to the SAME ratio it already widens the pixel radius
    /// by (`SNAP_BREAK_RADIUS_PX / SNAP_RADIUS_PX`), so soft-axis hysteresis
    /// matches the "feel" of every other sticky kind without inventing a
    /// second, independently-tuned constant. The ranking tie band
    /// ([`RANK_TIE_FRACTION`]) is divided by this same ratio, so a widened
    /// query still ranks at the acquire query's pixel resolution and can
    /// never reorder candidates the acquire query already admitted.
    pub soft_axis_aperture_scale: Option<f64>,
    /// Keep PRECISE POINT candidates ([`SnapKind::Endpoint`],
    /// [`SnapKind::Midpoint`], [`SnapKind::Center`], [`SnapKind::Quadrant`],
    /// [`SnapKind::Intersection`]) that lie OFF the `constraint_plane`
    /// instead of filtering them out. No effect without a
    /// `constraint_plane`, and never loosens the occlusion cull — a hidden
    /// off-plane point stays unsnappable, so the see-through protection the
    /// plane filter was introduced for is preserved by the cull alone.
    ///
    /// Exists for tools that can HONOUR an off-plane point, in one of three
    /// ways. `LineTool`'s plane-mode chain re-homes onto a new sketch plane
    /// when a committed point leaves the frozen one (the 3d-line defect), so
    /// a visible vertex in an EARLIER sketch of the same chain — e.g. the
    /// chain's own origin after axis-locked segments carried it across two
    /// re-homes — is a legitimate target its frozen plane must not veto.
    /// `TapeMeasureTool`'s measure mode instead keeps its plane immutable and
    /// PROJECTS an off-plane candidate onto it, but only while disclosing
    /// that the landed point is a projection (its `snapProjected()`, surfaced
    /// as the inference chip's "projected" qualifier) rather than silently
    /// presenting the projection as the literal snap. `TapeMeasureTool`'s
    /// PARALLEL-guide stage uses a third pattern: it keeps neither the raw
    /// point nor a plane-projected point, only the candidate's scalar
    /// component along the resolved offset direction (the in-plane `u` an
    /// arrow-locked axis, a frozen gesture plane, or a first-click face
    /// anchor implies) — but the SAME disclosure obligation applies whenever
    /// that discards a nonzero off-plane component. Tools that commit into
    /// one immutable plane with no such disclosure (Rectangle/Circle/Polygon/
    /// Arc, and Line's face mode) leave this `false`: a snap they would have
    /// to project back onto the plane afterwards would LIE about its
    /// position, and an undisclosed candidate that cannot be honoured must
    /// not be offered.
    ///
    /// Sliding/directional kinds (`OnEdge`, `OnFace`, `OnAxis`, `OnGuide`,
    /// `Tangent`) stay plane-filtered even when this is set: they are not
    /// discrete targets a re-home can pin down, and off-plane they mostly
    /// reintroduce the see-through noise the filter exists to remove.
    pub off_plane_points: bool,
    /// Under a non-occluding [`FacePolicy`] (X-ray, Wireframe), keep precise
    /// POINT candidates AND `OnEdge` that lie off the `constraint_plane`:
    /// there is no see-through to protect against, and hidden geometry
    /// behind the constraining face is what those modes are for. Opt-in, for
    /// the tools that honor such a snap by dropping it onto their plane and
    /// saying so (the draw tools' "projected" cue); a tool that commits the
    /// snapped point as-is leaves this `false` and gets exactly the plain
    /// plane filter in every mode. No effect under [`FacePolicy::Opaque`].
    pub see_through_points: bool,
}

/// A snappable point with provenance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenePoint {
    /// World-space position.
    pub position: Point3,
    /// Where it came from.
    pub source: SnapSource,
}

/// A rim circle of a claimed cylinder in world space, for tangent
/// inference (mirrors [`kernel::AnalyticRim`] through a placement).
#[derive(Debug, Clone, PartialEq)]
struct SceneRim {
    center: Point3,
    /// Unit rim-plane normal (the cylinder axis).
    axis: Vec3,
    radius: f64,
    /// Unit angular-frame basis (perpendicular to `axis`).
    u: Vec3,
    v: Vec3,
    /// Merged coverage intervals in the (u, v) frame; `None` = full circle
    /// (see [`kernel::AnalyticRim::coverage`]). Angles are similarity
    /// invariant, so the object-space intervals apply verbatim.
    coverage: Option<Vec<[f64; 2]>>,
    source: SnapSource,
}

impl SceneRim {
    /// Whether `angle` (radians in the (u, v) frame) is covered, within the
    /// same tolerance rule as [`kernel::AnalyticRim::covers`].
    fn covers(&self, angle: f64) -> bool {
        let Some(intervals) = &self.coverage else {
            return true;
        };
        let eps = tol::POINT_MERGE / self.radius;
        let tau = 2.0 * std::f64::consts::PI;
        let mut a = angle;
        while a >= std::f64::consts::PI {
            a -= tau;
        }
        while a < -std::f64::consts::PI {
            a += tau;
        }
        intervals.iter().any(|&[s, e]| {
            (a >= s - eps && a <= e + eps) || (a + tau >= s - eps && a + tau <= e + eps)
        })
    }

    /// The rim point at `angle` in the (u, v) frame.
    fn point_at(&self, angle: f64) -> Point3 {
        self.center + self.u * (self.radius * angle.cos()) + self.v * (self.radius * angle.sin())
    }
}

/// A snappable segment (kernel edge in world space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSegment {
    /// One endpoint.
    pub a: Point3,
    /// The other endpoint.
    pub b: Point3,
    /// Where it came from.
    pub source: SnapSource,
}

/// A snappable planar face region (kernel face in world space).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFace {
    /// The supporting plane.
    pub plane: Plane,
    /// Outer boundary in cycle order (containment tests happen against it).
    pub boundary: Vec<Point3>,
    /// Inner loops (holes) in cycle order. A ray that lands inside any of
    /// these is NOT on the face — it passes through the hole (e.g. the annular
    /// parent of an imprinted sub-face). Empty for ordinary faces.
    pub holes: Vec<Vec<Point3>>,
    /// Whether the face is a chord facet of an analytic surface (the kernel
    /// face carries a `surface` reference: an extruded arc's or circle's
    /// side wall). Where such a face meets another owner's face, the
    /// meeting is a curve cut into one stretch per facet, and a stretch's
    /// middle is no point of it (`SnapKind::OnIntersectionLine`).
    pub surface_facet: bool,
    /// Where it came from.
    pub source: SnapSource,
}

/// A snappable/occluding planar face from a committed sketch region (a drawn
/// but not-yet-extruded shape's closed loop), in world space. Unlike
/// [`SceneFace`] it carries no [`SnapSource`] (a sketch region isn't a
/// selectable Object element, so it snaps with `source: None`), but it does
/// carry its [`SketchRegionId`] so an `OnFace` snap on the fill resolves to
/// this exact region. Registering these makes a drawn region a first-class
/// hoverable face: the cursor snaps to it ([`SnapKind::OnFace`]) and it
/// occludes geometry behind it, instead of the ray passing through to the
/// ground/box beneath, matching how a solid's face behaves.
#[derive(Debug, Clone, PartialEq)]
pub struct SketchRegionFace {
    /// Which region this is (within its owning sketch): the provenance an
    /// `OnFace` snap carries, so a click on the fill resolves to this exact
    /// region.
    pub region: SketchRegionId,
    /// The region's supporting plane.
    pub plane: Plane,
    /// Outer boundary in cycle order (ray-polygon containment tests against it).
    pub boundary: Vec<Point3>,
    /// Inner loops (holes) in cycle order; empty for a hole-free region. A ray
    /// through a hole is NOT on the face (mirrors [`SceneFace::holes`]).
    pub holes: Vec<Vec<Point3>>,
}

/// A definition-space snap point: one member vertex, stored once and shared
/// by every placement (world position = `pose.apply_point(position)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DefPoint {
    pub(crate) position: Point3,
    pub(crate) vertex: VertexId,
}

/// A definition-space snap segment (one member edge; see [`DefPoint`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DefSegment {
    pub(crate) a: Point3,
    pub(crate) b: Point3,
    pub(crate) edge: EdgeId,
}

/// A definition-space snap face (one member face; see [`DefPoint`]). The
/// plane is carried in definition space and mapped per placement via
/// [`Transform::apply_plane`] — the same inverse-transpose-safe path
/// registration-time baking used, so mirrored and non-uniformly scaled
/// placements keep exact parity with the old per-placement storage.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DefFace {
    pub(crate) plane: Plane,
    pub(crate) boundary: Vec<Point3>,
    pub(crate) holes: Vec<Vec<Point3>>,
    /// See [`SceneFace::surface_facet`].
    pub(crate) surface_facet: bool,
    pub(crate) face: FaceId,
}

/// One definition member's shared snap data. Its bounds and rims come
/// from its object at registration ([`InferenceScene::set_def_member`],
/// [`InferenceScene::set_def_member_deferred`]); its geometry
/// ([`DefGeometry`]) either then or, deferred, the first time a query
/// reaches one of its placements, through the query's [`MemberSource`].
/// Every placement of the member resolves against this one copy. A
/// document of thousands of placed components holds millions of member
/// vertices, edges and faces: extracting them all at open costs a large
/// part of the memory a browser allows, and a session touches few of them.
#[derive(Debug, Clone)]
pub(crate) struct DefMember {
    /// Class boxes: what every placement's world boxes derive from.
    pub(crate) bounds: DefBounds,
    /// Definition-space analytic rims (coverage-bearing only), materialized
    /// per placement at query time (see [`DefMember::rims_at`]) — rims are
    /// few per scene and linear-walked, so placements pay no baking cost and
    /// every removal/replace path stays automatic.
    pub(crate) rims: Vec<AnalyticRim>,
    /// Definition-space centers of polygons drawn on the member's faces
    /// ([`Object::edge_polygon_centers`]), offered like rim centers.
    pub(crate) polygon_centers: Vec<(Point3, kernel::FaceId)>,
    /// The member's candidates and trees, once extracted.
    geometry: OnceCell<DefGeometry>,
}

/// A definition member's definition-space candidates — the same element
/// walk [`InferenceScene::register`] does for world objects, minus the
/// world transform (applied per placement at query time instead) — and
/// the persistent per-member spatial trees over them.
#[derive(Debug, Clone)]
pub(crate) struct DefGeometry {
    pub(crate) points: Vec<DefPoint>,
    pub(crate) segments: Vec<DefSegment>,
    pub(crate) faces: Vec<DefFace>,
    pub(crate) index: DefIndex,
}

impl DefMember {
    /// The member of `object`, its geometry extracted now or left for the
    /// first query that needs it.
    fn of(object: &Object, extract_now: bool) -> DefMember {
        // Analytic rims, gated on surviving coverage exactly like the world
        // path (`register`): a vacant rim offers no candidates at all.
        // Imprinted circle edges (a circle drawn on a member's face) join the
        // wall rims, exactly as on the world path.
        let walls = object.analytic_rims();
        let drawn = object.edge_curve_rims_with(&walls);
        let rims = walls
            .into_iter()
            .chain(drawn)
            .filter(AnalyticRim::has_coverage)
            .collect();
        let geometry = OnceCell::new();
        if extract_now {
            let _ = geometry.set(DefGeometry::extract(object));
        }
        DefMember {
            bounds: DefBounds::of(object),
            rims,
            polygon_centers: object.edge_polygon_centers(),
            geometry,
        }
    }

    /// Materializes this member's analytic-rim candidates under one
    /// placement, pushing them into the caller's per-query vectors: the
    /// identical center/quadrant mapping the world path
    /// ([`InferenceScene::register`]) bakes at registration time, and the
    /// same map-or-drop similarity gate for the tangent rims — so a placed
    /// cylinder snaps exactly like the world object it instances.
    fn rims_at(
        &self,
        pl: &Placement,
        centers: &mut Vec<ScenePoint>,
        quadrants: &mut Vec<ScenePoint>,
        rims: &mut Vec<SceneRim>,
    ) {
        let similarity = pl.pose.similarity_scale();
        for &(c, face) in &self.polygon_centers {
            centers.push(ScenePoint {
                position: pl.pose.apply_point(c),
                source: SnapSource {
                    object: pl.member,
                    element: ElementRef::Face(face),
                    instance: Some(pl.instance),
                },
            });
        }
        for rim in &self.rims {
            let source = SnapSource {
                object: pl.member,
                element: ElementRef::Face(rim.rep),
                instance: Some(pl.instance),
            };
            centers.push(ScenePoint {
                position: pl.pose.apply_point(rim.center),
                source,
            });
            // Quadrant points transform as plain points: under any affine
            // pose they stay on the (possibly elliptical) rim curve.
            for q in rim.quadrant_points() {
                quadrants.push(ScenePoint {
                    position: pl.pose.apply_point(q),
                    source,
                });
            }
            // Tangency needs a genuine circle: map the rim under
            // similarities, drop it otherwise (never approximate).
            if let Some(scale) = similarity {
                let map_unit = |w: Vec3| pl.pose.apply_vector(w).normalized();
                if let (Ok(axis), Ok(u), Ok(v)) = (
                    map_unit(rim.axis),
                    map_unit(rim.basis_u),
                    map_unit(rim.basis_v),
                ) {
                    rims.push(SceneRim {
                        center: pl.pose.apply_point(rim.center),
                        axis,
                        radius: rim.radius * scale,
                        u,
                        v,
                        coverage: rim.coverage.clone(),
                        source,
                    });
                }
            }
        }
    }
}

impl DefGeometry {
    /// Extracts `object`'s vertices/edges/faces into definition-space
    /// candidates and builds the trees over them.
    fn extract(object: &Object) -> DefGeometry {
        let mut points = Vec::new();
        let mut segments = Vec::new();
        let mut faces = Vec::new();
        for (vid, vertex) in object.vertices() {
            points.push(DefPoint {
                position: vertex.position,
                vertex: vid,
            });
        }
        for (eid, edge) in object.edges() {
            let half_edges = object.half_edges();
            let he = &half_edges[edge.half_edge];
            let origin_vid = he.origin;
            let dest_vid = half_edges[he.next].origin;
            segments.push(DefSegment {
                a: object.vertices()[origin_vid].position,
                b: object.vertices()[dest_vid].position,
                edge: eid,
            });
        }
        for (fid, face) in object.faces() {
            let boundary: Vec<Point3> = object.loop_positions(face.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = face
                .inner_loops
                .iter()
                .map(|&lid| object.loop_positions(lid).collect())
                .collect();
            faces.push(DefFace {
                plane: face.plane,
                boundary,
                holes,
                surface_facet: face.surface.is_some(),
                face: fid,
            });
        }
        let index = DefIndex::build(&points, &segments, &faces);
        DefGeometry {
            points,
            segments,
            faces,
            index,
        }
    }

    /// Materializes one placed point: the member's vertex under the
    /// placement's pose, sourced to the instance.
    fn point_at(&self, li: usize, pl: &Placement) -> ScenePoint {
        let p = &self.points[li];
        ScenePoint {
            position: pl.pose.apply_point(p.position),
            source: SnapSource {
                object: pl.member,
                element: ElementRef::Vertex(p.vertex),
                instance: Some(pl.instance),
            },
        }
    }

    /// Materializes one placed segment (see [`DefGeometry::point_at`]).
    fn segment_at(&self, li: usize, pl: &Placement) -> SceneSegment {
        let s = &self.segments[li];
        SceneSegment {
            a: pl.pose.apply_point(s.a),
            b: pl.pose.apply_point(s.b),
            source: SnapSource {
                object: pl.member,
                element: ElementRef::Edge(s.edge),
                instance: Some(pl.instance),
            },
        }
    }

    /// Materializes one placed face (see [`DefGeometry::point_at`]), or `None`
    /// for a singular pose — the same faces registration-time baking used to
    /// skip, so both storage schemes emit the same candidate set.
    fn face_at(&self, li: usize, pl: &Placement) -> Option<SceneFace> {
        let f = &self.faces[li];
        let plane = pl.pose.apply_plane(&f.plane).ok()?;
        Some(SceneFace {
            plane,
            boundary: f.boundary.iter().map(|&p| pl.pose.apply_point(p)).collect(),
            holes: f
                .holes
                .iter()
                .map(|hole| hole.iter().map(|&p| pl.pose.apply_point(p)).collect())
                .collect(),
            surface_facet: f.surface_facet,
            source: SnapSource {
                object: pl.member,
                element: ElementRef::Face(f.face),
                instance: Some(pl.instance),
            },
        })
    }
}

/// Where a query finds the object of a deferred definition member the
/// first time it needs the member's geometry (see
/// [`InferenceScene::set_def_member_deferred`]): `None` for a member it
/// does not know, which then offers nothing.
pub type MemberSource<'a> = &'a dyn Fn(ObjectId) -> Option<&'a Object>;

/// One placement of a definition member: everything an instance contributes
/// to the scene, now that the geometry itself is shared (see [`DefMember`]).
/// Registration order is the candidate enumeration order for placements, so
/// it participates in ranking tie-breaks; callers drive registration from
/// deterministic document iteration, keeping resolve results reproducible.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placement {
    pub(crate) instance: InstanceId,
    pub(crate) member: ObjectId,
    pub(crate) pose: Transform,
}

/// A construction guide registered with the inference engine, in world
/// space. Guides carry no topology and have no `SnapSource` — like the world
/// origin/axes, they snap with `source: None` (see [`SnapKind::OnGuide`]).
#[derive(Debug, Clone, Copy, PartialEq)]
struct SceneGuide {
    id: GuideId,
    geom: SceneGuideGeom,
}

/// A bare endpoint-pair segment with no `SnapSource` provenance — used for
/// sketch and transient candidates (Part 1, Phase B), which aren't yet
/// selectable kernel elements. Endpoints/midpoints derived from these snap
/// exactly like [`SceneSegment`]'s, just with `source: None` (mirroring how a
/// guide or the world axes snap with no provenance).
#[derive(Debug, Clone, Copy, PartialEq)]
struct BareSegment {
    a: Point3,
    b: Point3,
}

/// The geometry of a [`SceneGuide`], mirroring [`kernel::Guide`] in world
/// space (guides carry no placement/instance — they're authored directly in
/// world coordinates, unlike Object geometry).
#[derive(Debug, Clone, Copy, PartialEq)]
enum SceneGuideGeom {
    /// A construction line through `origin` along unit `direction`, over
    /// `span` (the whole line, a ray, or a segment): only that stretch
    /// snaps, and only a crossing on it.
    Line {
        origin: Point3,
        direction: Vec3,
        span: kernel::GuideSpan,
    },
    /// A single construction point.
    Point { position: Point3 },
}

/// Whether faces hide what is behind them, and whether they are input
/// targets at all — the picking rule of a whole-model view mode (crate docs,
/// *Occlusion*). Set on the scene with [`InferenceScene::set_face_policy`];
/// [`FacePolicy::Opaque`] is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FacePolicy {
    /// Faces occlude: only visible geometry snaps, and faces are snap
    /// ([`SnapKind::OnFace`]) and [`InferenceScene::pick_face`] targets.
    #[default]
    Opaque,
    /// Nothing occludes: every edge and point snaps, hidden or not. Faces
    /// stay targets, and the nearest one along the ray wins.
    SeeThrough,
    /// Nothing occludes and faces are not targets: no `OnFace` candidate
    /// (sketch-region fills included) and no [`InferenceScene::pick_face`]
    /// hit. Edges and points still snap, so an Object is reached through
    /// its edges.
    Absent,
}

impl FacePolicy {
    /// Whether faces hide geometry behind them from snapping.
    pub const fn occludes(self) -> bool {
        matches!(self, FacePolicy::Opaque)
    }

    /// Whether faces are snap and pick targets.
    pub const fn faces_are_targets(self) -> bool {
        !matches!(self, FacePolicy::Absent)
    }
}

/// What an open edit can touch: wherever faces are targets
/// ([`FacePolicy::Opaque`] and [`FacePolicy::SeeThrough`]),
/// [`InferenceScene::pick_face`] lands on the nearest face inside it (crate
/// docs, *Occlusion*). Set with [`InferenceScene::set_pick_scope`]; no scope
/// (`None`) is the top level, where nothing is skipped. Membership, not
/// eligibility: a candidate is in scope when its owner is listed
/// ([`SnapSource::in_scope`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickScope {
    /// World objects: a session frame's members, or the entered object.
    pub objects: BTreeSet<ObjectId>,
    /// Placements whose geometry is in scope: a session frame's surfaced
    /// member instances, or the one instance of a legacy in-context edit.
    pub instances: BTreeSet<InstanceId>,
}

/// Kinds of sketch candidate a registered sketch can have, as bits of
/// `InferenceScene::sketch_kinds`.
const SKETCH_SEGMENTS: u8 = 1;
const SKETCH_VERTICES: u8 = 2;
const SKETCH_RIMS: u8 = 4;
const SKETCH_POLYGON_CENTERS: u8 = 8;
const SKETCH_FACES: u8 = 16;

/// The engine's view of the scene: world-space snap candidates extracted
/// from every Object, refreshed incrementally as Objects change.
///
/// The spatial index lives behind this type; the public API exposes
/// only candidates and queries so the indexing strategy stays swappable.
#[derive(Debug, Clone)]
pub struct InferenceScene {
    points: Vec<ScenePoint>,
    segments: Vec<SceneSegment>,
    faces: Vec<SceneFace>,
    /// True circle centers derived from objects' analytic surface
    /// references (see [`SnapKind::Center`]): few per scene, registered
    /// alongside `points` and resolved on a linear walk (never indexed).
    centers: Vec<ScenePoint>,
    /// Rim quadrant points (see [`SnapKind::Quadrant`]): the covered
    /// cardinal points of each claimed cylinder's two rim circles. Same
    /// lifecycle and linear-walk resolution as `centers`.
    quadrants: Vec<ScenePoint>,
    /// Rim circles for tangent inference (see [`SnapKind::Tangent`]):
    /// exact center/axis/radius plus angular coverage, in world space.
    /// Registered only for placements that preserve circles (world objects
    /// and similarity-posed instances) — a non-uniformly scaled instance's
    /// rims are ellipses the reference cannot represent, so they are
    /// dropped, never approximated. Linear walk, like `centers`.
    rims: Vec<SceneRim>,
    guides: Vec<SceneGuide>,
    /// Persistent sketch candidates (committed sketch edges, not yet kernel
    /// Objects): keyed by `SketchId` so a caller can replace one sketch's
    /// segments without touching another's. No `SnapSource` provenance —
    /// sketch elements aren't selectable in this phase.
    ///
    /// The optional `SketchCurveId` is the drawn curve chain this segment is a
    /// facet of (a circle's/arc's chord), `None` for a plain line or a
    /// rectangle side. It gives a facet VERTEX's [`SnapKind::Endpoint`] snap
    /// the curve chain it belongs to via [`Snap::sketch_curve_source`] — so
    /// clicking a circle exactly on a facet vertex selects the curve, the same
    /// as clicking its rim edge — WITHOUT giving that vertex a
    /// [`Snap::sketch_source`] (a facet vertex is not a direction reference,
    /// and `sketch_source` is consumed as one by Tape Measure). The facet
    /// EDGE's Midpoint/OnEdge keep their [`Provenance::SketchEdge`]: an edge
    /// genuinely is a direction reference.
    sketch_segments: Vec<(SketchId, SketchEdgeId, Option<SketchCurveId>, BareSegment)>,
    /// Which kinds of sketch candidate (`SKETCH_*` bits) each registered
    /// sketch has in the vectors below. A removal, and the replacement at
    /// the start of each `add_sketch_*`, walks a vector only for a sketch
    /// that has entries in it: a document of tens of thousands of small
    /// sketches registers each once at open, and walking every vector
    /// for every one of them to remove nothing was quadratic.
    sketch_kinds: BTreeMap<SketchId, u8>,
    /// Committed sketch *vertices*, keyed by `SketchId`, carrying their
    /// `SketchVertexId` so the per-vertex edit tool (Phase D) can pick an exact
    /// vertex to drag. Registered/cleared alongside `sketch_segments`.
    sketch_vertices: Vec<(SketchId, SketchVertexId, Point3)>,
    /// Committed sketch *curve rims* (drawn circles/arcs carrying an
    /// analytic [`kernel::CurveGeom`]), keyed by `SketchId` like
    /// `sketch_segments`: each offers its exact center, covered quadrant
    /// points, and anchor-based tangents — the sketch-level analogue of
    /// `centers`/`quadrants`/`rims`, resolved on the same linear walk. They
    /// are not `SnapSource` elements (a sketch curve is not an Object), so
    /// they snap with `source: None`, but they DO carry
    /// [`Snap::sketch_curve_source`] — the curve chain the point belongs to.
    /// Registered/cleared alongside `sketch_segments`, so a drawn circle's
    /// true center snaps BEFORE any extrusion exists.
    sketch_rims: Vec<(SketchId, SketchCurveRim)>,
    /// Committed sketch *polygon centers* — the exact drawn center of each
    /// regular polygon chain ([`kernel::Sketch::polygon_centers`]), keyed by
    /// `SketchId` like `sketch_segments`.
    ///
    /// Separate from `sketch_rims` because a polygon has no rim: its sides
    /// ARE its geometry, so the circumcircle it was drawn from supplies a
    /// center and nothing else — no quadrant points (they would lie on no
    /// edge) and no tangents. Same lifecycle, same linear walk, same
    /// [`SnapKind::Center`] and same [`Snap::sketch_curve_source`] provenance
    /// the circle case emits, so nothing downstream has to learn a new kind
    /// or a second way of naming a drawn curve.
    sketch_polygon_centers: Vec<(SketchId, SketchCurveId, Point3)>,
    /// Committed sketch *region faces* (closed regions of drawn shapes), keyed
    /// by `SketchId` like `sketch_segments`. Each registers a hoverable,
    /// occluding face so the cursor snaps to a drawn region ([`SnapKind::OnFace`])
    /// and the region hides geometry behind it — exactly like a solid's face —
    /// instead of the ray passing through to the ground/box beneath. No
    /// `SnapSource` provenance (sketch regions aren't Object elements; like
    /// guides they snap with `source: None`). Registered/cleared alongside
    /// `sketch_segments`; few per scene, so linear-walked, never indexed.
    /// Each carries its box ([`region_aabb`]), taken once here, so the face
    /// walks of `resolve_impl` prune drawn regions by box like the indexed
    /// solid faces instead of testing every region on every query.
    sketch_faces: Vec<(SketchId, SketchRegionFace, Aabb)>,
    /// Transient (in-progress) segments — e.g. the line tool's current
    /// rubber-band chain — published every frame and never persisted. Cleared
    /// wholesale by [`InferenceScene::clear_transient`], not per-id.
    transient_segments: Vec<BareSegment>,
    /// When `false`, guide candidates are suppressed (View ▸ Guides off): a
    /// hidden guide must not snap or flash a cue. Defaults to `true`.
    guides_enabled: bool,
    /// When `false`, the world-origin/axis candidates are suppressed (View ▸
    /// Axes off): hidden axes must not snap or flash a cue. Defaults to `true`.
    axes_enabled: bool,
    /// The document's movable drawing axes (tool-parity design §4), pushed
    /// in by [`InferenceScene::set_axes_frame`] the way `axes_enabled` is
    /// pushed by [`InferenceScene::set_axes_enabled`]. The origin/axis
    /// candidates below snap to THIS frame's origin and directions, not the
    /// literal world axes — so moving the drawing axes moves what "on
    /// axis" means for inference exactly as it does for drawing. Defaults
    /// to [`AxesFrame::IDENTITY`] (world).
    axes_frame: AxesFrame,
    /// Whether faces occlude and are targets (the view mode's picking rule,
    /// pushed by [`InferenceScene::set_face_policy`]). Defaults to
    /// [`FacePolicy::Opaque`].
    face_policy: FacePolicy,
    /// The editing scope [`InferenceScene::pick_face`] favors wherever
    /// faces are targets (pushed by
    /// [`InferenceScene::set_pick_scope`]). `None` (the default) is the top
    /// level: no scope.
    pick_scope: Option<PickScope>,
    /// Lazily rebuilt spatial index over `points`/`segments`/`faces`; `None`
    /// means dirty (a mutator ran since the last build). Interior mutability
    /// because the hot pointer-move queries (`resolve`, `pick_face`) take
    /// `&self` while a rebuild must write the cache. Panic-free by
    /// construction: the only `borrow_mut` lives in
    /// [`InferenceScene::spatial_index`], whose build reads the candidate
    /// Vecs and calls nothing that touches this cell again (no reentrancy);
    /// `RefCell` additionally makes the scene `!Sync`, so a future threaded
    /// caller fails to compile instead of racing.
    spatial: RefCell<Option<SceneIndex>>,
    /// Cumulative count of exact ray-vs-face occlusion tests (see
    /// [`InferenceScene::occlusion_face_tests`]). `Cell` because queries
    /// take `&self`; single-threaded for the same reason as `spatial`.
    occlusion_tests: Cell<u64>,
    /// World-object ids currently registered via
    /// [`InferenceScene::add_object`] — exactly the ids for which candidates
    /// with `instance == None` can exist (`register` only emits such
    /// candidates on the `add_object` path). Lets
    /// [`InferenceScene::remove_object`] answer "nothing to remove" in
    /// O(log owners) instead of three O(scene) retain passes: bulk registration (document
    /// load, undo/redo re-registration) calls the replace-semantics `add_*`
    /// once per object, and paying a full-scene scan for each never-present
    /// id made that accidentally quadratic.
    world_owners: BTreeSet<ObjectId>,
    /// Instance ids currently registered via
    /// [`InferenceScene::add_placement`] — exactly the ids for which
    /// placements can exist (across every definition member the instance
    /// places). Same fast path for [`InferenceScene::remove_instance`].
    instance_owners: BTreeSet<InstanceId>,
    /// Member ids for which placements may exist (inserted by
    /// [`InferenceScene::add_placement`]) — the fast path for
    /// [`InferenceScene::remove_def_member`], which reconcile calls for
    /// EVERY touched non-world object: without it each never-registered id
    /// pays a full placement scan, the same accidental quadratic the other
    /// owner sets exist to prevent. Like `instance_owners`, entries may be
    /// stale ("may exist", not "exist") — a stale entry costs one wasted
    /// retain pass, never a wrong skip.
    member_owners: BTreeSet<ObjectId>,
    /// Cumulative count of candidates walked by the removal retain passes
    /// (see [`InferenceScene::removal_candidates_visited`]). Plain `u64`
    /// (not `Cell`) because removal takes `&mut self`. Instance removal no
    /// longer visits candidates at all — placements are records, not
    /// candidate spans — so only world-object removal contributes.
    removal_visits: u64,
    /// Definition-space snap geometry, one entry per registered definition
    /// member, shared by every placement of that member. Survives
    /// [`InferenceScene::clear_solids`] (visibility changes never touch
    /// definition geometry); a caller switching documents must start from a
    /// fresh scene, since member ids from another document could collide.
    /// `BTreeMap` (not a hash map) per the determinism rule
    /// (DEVELOPMENT.md §7).
    def_members: BTreeMap<ObjectId, DefMember>,
    /// Live placements, in registration order (the enumeration order for
    /// placed candidates — see [`Placement`]).
    placements: Vec<Placement>,
    /// Cumulative count of definition-member extraction passes (see
    /// [`InferenceScene::def_extractions`]).
    def_extractions: Cell<u64>,
    /// Cumulative edge × edge crossing counters (see
    /// [`InferenceScene::crossing_stats`]); `Cell` for the same reason as
    /// `occlusion_tests`.
    crossing_stats: Cell<CrossingStats>,
}

impl Default for InferenceScene {
    fn default() -> Self {
        InferenceScene {
            points: Vec::new(),
            segments: Vec::new(),
            faces: Vec::new(),
            centers: Vec::new(),
            quadrants: Vec::new(),
            rims: Vec::new(),
            guides: Vec::new(),
            sketch_segments: Vec::new(),
            sketch_kinds: BTreeMap::new(),
            sketch_vertices: Vec::new(),
            sketch_rims: Vec::new(),
            sketch_polygon_centers: Vec::new(),
            sketch_faces: Vec::new(),
            transient_segments: Vec::new(),
            guides_enabled: true,
            axes_enabled: true,
            axes_frame: AxesFrame::IDENTITY,
            face_policy: FacePolicy::Opaque,
            pick_scope: None,
            spatial: RefCell::new(None),
            occlusion_tests: Cell::new(0),
            world_owners: BTreeSet::new(),
            instance_owners: BTreeSet::new(),
            member_owners: BTreeSet::new(),
            removal_visits: 0,
            def_members: BTreeMap::new(),
            placements: Vec::new(),
            def_extractions: Cell::new(0),
            crossing_stats: Cell::new(CrossingStats::default()),
        }
    }
}

impl InferenceScene {
    /// An empty scene.
    pub fn new() -> InferenceScene {
        InferenceScene::default()
    }

    /// Enable/disable guide snapping (View ▸ Guides). Hidden guides must not
    /// snap or flash a cue; the registered guides are kept, only their
    /// candidate emission is gated.
    pub fn set_guides_enabled(&mut self, enabled: bool) {
        self.guides_enabled = enabled;
    }

    /// Enable/disable world-origin/axis snapping (View ▸ Axes).
    pub fn set_axes_enabled(&mut self, enabled: bool) {
        self.axes_enabled = enabled;
    }

    /// Whether guide snapping is enabled ([`Self::set_guides_enabled`]).
    pub fn guides_enabled(&self) -> bool {
        self.guides_enabled
    }

    /// Whether world-origin/axis snapping is enabled
    /// ([`Self::set_axes_enabled`]).
    pub fn axes_enabled(&self) -> bool {
        self.axes_enabled
    }

    /// Sets whether faces occlude and whether they are snap/pick targets —
    /// the picking rule of the view mode (crate docs, *Occlusion*). Applies
    /// to every later [`Self::resolve`] and [`Self::pick_face`]; the
    /// registered geometry is untouched, so switching is instant.
    pub fn set_face_policy(&mut self, policy: FacePolicy) {
        self.face_policy = policy;
    }

    /// The current [`FacePolicy`].
    pub fn face_policy(&self) -> FacePolicy {
        self.face_policy
    }

    /// Sets the editing scope (crate docs, *Occlusion*): under
    /// [`FacePolicy::Opaque`] and [`FacePolicy::SeeThrough`], every later
    /// [`Self::pick_face`] returns the nearest face whose owner is in
    /// `scope`, or the nearest face of any owner when none in scope lies
    /// along the ray. `None` clears it; an empty scope is a scope (it holds
    /// nothing, so every pick falls back to the nearest face). Ignored
    /// under [`FacePolicy::Absent`] (no face is a target), by snapping, and
    /// by the geometry-only picks. View state like the face policy: the registered
    /// geometry is untouched.
    pub fn set_pick_scope(&mut self, scope: Option<PickScope>) {
        self.pick_scope = scope;
    }

    /// The current editing scope ([`Self::set_pick_scope`]), if any.
    pub fn pick_scope(&self) -> Option<&PickScope> {
        self.pick_scope.as_ref()
    }

    /// Sets the movable drawing axes frame (tool-parity design §4) that
    /// origin/axis inference candidates snap to. Pushed by the wasm boundary
    /// on every reconciled document mutation (including a document load or
    /// an axes-move undo/redo), so this scene's candidates never lag the
    /// document's own [`kernel::Document::axes`].
    pub fn set_axes_frame(&mut self, frame: AxesFrame) {
        self.axes_frame = frame;
    }

    /// Candidate counts as (points, segments, faces) — cheap introspection
    /// for tests and debug overlays. Counts what queries can *see*: world
    /// candidates plus each placement's share of its member's candidates
    /// (definition geometry with no live placement contributes nothing, and
    /// neither does a deferred member no query has extracted yet).
    pub fn candidate_counts(&self) -> (usize, usize, usize) {
        let (mut p, mut s, mut f) = (self.points.len(), self.segments.len(), self.faces.len());
        for pl in &self.placements {
            if let Some(m) = self
                .def_members
                .get(&pl.member)
                .and_then(|m| m.geometry.get())
            {
                p += m.points.len();
                s += m.segments.len();
                // Placed faces resolve through `apply_plane`, which refuses
                // singular poses (`face_at` → None) — mirror its determinant
                // gate so a degenerate placement's faces aren't counted as
                // visible.
                if pl.pose.determinant().abs() >= tol::NORMALIZE_MIN_LENGTH {
                    f += m.faces.len();
                }
            }
        }
        (p, s, f)
    }

    /// Cumulative number of definition-member extraction passes across the
    /// scene's lifetime: one per [`InferenceScene::set_def_member`], and one
    /// per deferred member ([`InferenceScene::set_def_member_deferred`]) the
    /// first time a query reaches it — cheap introspection for tests and
    /// debug overlays, like [`InferenceScene::occlusion_face_tests`]. Shared
    /// definition storage exists to make this scale with *definitions*, not
    /// placements: the registration perf-sanity spec asserts registering N
    /// instances of one member costs exactly one extraction.
    pub fn def_extractions(&self) -> u64 {
        self.def_extractions.get()
    }

    /// Cumulative number of exact ray-vs-face tests performed by occlusion
    /// culling across all queries so far — cheap introspection for tests and
    /// debug overlays, like [`InferenceScene::candidate_counts`]. The
    /// spatial index exists to keep the per-query delta far below the total
    /// face count; the perf-sanity spec asserts exactly that.
    pub fn occlusion_face_tests(&self) -> u64 {
        self.occlusion_tests.get()
    }

    /// Cumulative edge × edge crossing and edge × face piercing counters
    /// across all queries so far: pairs tested, and the queries (and their
    /// pairs) skipped for passing [`CROSSING_PAIR_CAP`] or
    /// [`PIERCE_PAIR_CAP`] — cheap introspection for tests and debug
    /// overlays, like [`InferenceScene::occlusion_face_tests`].
    pub fn crossing_stats(&self) -> CrossingStats {
        self.crossing_stats.get()
    }

    /// Cumulative number of candidates walked by the retain passes of
    /// [`InferenceScene::remove_object`] and
    /// [`InferenceScene::remove_instance`] across all calls so far — cheap
    /// introspection for tests and debug overlays, like
    /// [`InferenceScene::occlusion_face_tests`]. The owner-set fast path
    /// exists to make removal of a never-registered id visit zero candidates
    /// (bulk registration calls the replace-semantics `add_*` once per
    /// object, so anything else is accidentally quadratic in scene size);
    /// the removal perf-sanity spec asserts exactly that.
    pub fn removal_candidates_visited(&self) -> u64 {
        self.removal_visits
    }

    /// The spatial index, rebuilding it first if a mutator marked it dirty.
    ///
    /// Rebuild cost is O(n log n) in the candidate count and amortizes
    /// across the many pointer-move queries between committed mutations —
    /// mutators only invalidate, they never rebuild. Panic-free: this holds
    /// the crate's only `borrow_mut`, and [`SceneIndex::build`] reads the
    /// candidate Vecs without re-entering the cell (see the `spatial` field
    /// docs).
    fn spatial_index(&self) -> Ref<'_, SceneIndex> {
        if self.spatial.borrow().is_none() {
            *self.spatial.borrow_mut() = Some(SceneIndex::build(
                &self.points,
                &self.segments,
                &self.faces,
                &self.placements,
                &self.def_members,
            ));
        }
        Ref::map(self.spatial.borrow(), |slot| {
            slot.as_ref()
                .expect("built above; nothing can mutate the scene through &self in between")
        })
    }

    /// Extracts snap candidates from `object` (vertices, edges with
    /// midpoints derived at query time, faces) transformed by `placement`
    /// into world space, replacing any candidates previously registered for
    /// `id`.
    ///
    /// Cost model: linear in the Object's elements; called on Object
    /// creation and after each committed mutation, never per-frame.
    pub fn add_object(&mut self, id: ObjectId, object: &Object, placement: &Transform) {
        // Replace semantics: drop any prior candidates for this id first.
        self.remove_object(id);
        // Record the owner before registering: `id` is now the one world
        // object whose candidates carry `instance == None`, which is exactly
        // what `remove_object`'s fast path keys on.
        self.world_owners.insert(id);
        self.register(object, placement, id);
    }

    /// Registers (or re-registers) the shared definition-space snap geometry
    /// of one component-definition member. Replace semantics: any prior
    /// geometry for `member` is dropped, and existing placements of `member`
    /// resolve against the new copy from the next query on — this is how a
    /// definition edit propagates to every placement in one extraction.
    ///
    /// Cost model: linear in the member's elements (plus the per-member tree
    /// build), paid once per member per geometry change — never per
    /// placement, which is the entire point of the shared storage (see the
    /// module docs). Callers re-registering placements cheaply gate on
    /// [`InferenceScene::has_def_member`] to skip this when the geometry is
    /// already current.
    pub fn set_def_member(&mut self, member: ObjectId, object: &Object) {
        self.def_members.insert(member, DefMember::of(object, true));
        self.def_extractions.set(self.def_extractions.get() + 1);
        // Placement-level world boxes derive from the member's class boxes:
        // the top-level index is stale even though no placement changed.
        *self.spatial.get_mut() = None;
    }

    /// [`Self::set_def_member`] with the member's geometry left for the
    /// first query that reaches one of its placements, which extracts it
    /// through its [`MemberSource`] ([`Self::resolve_from`],
    /// [`Self::pick_face_from`]); the member's bounds and rims are taken
    /// now. A query given no source, or one that does not know the
    /// member, offers nothing of it.
    pub fn set_def_member_deferred(&mut self, member: ObjectId, object: &Object) {
        self.def_members
            .insert(member, DefMember::of(object, false));
        *self.spatial.get_mut() = None;
    }

    /// The member's geometry, extracted through `source` the first time.
    fn geometry<'s>(
        &'s self,
        member: ObjectId,
        source: MemberSource<'_>,
    ) -> Option<&'s DefGeometry> {
        let m = self.def_members.get(&member)?;
        if let Some(g) = m.geometry.get() {
            return Some(g);
        }
        let object = source(member)?;
        self.def_extractions.set(self.def_extractions.get() + 1);
        let _ = m.geometry.set(DefGeometry::extract(object));
        m.geometry.get()
    }

    /// Whether shared definition geometry is currently registered for
    /// `member` (see [`InferenceScene::set_def_member`]).
    pub fn has_def_member(&self, member: ObjectId) -> bool {
        self.def_members.contains_key(&member)
    }

    /// Drops the shared definition geometry registered for `member`, along
    /// with every placement of it (a placement without geometry can't
    /// snap). Unknown ids are a no-op — callers invalidate freely: the
    /// reconcile path calls this for every touched non-world object, using
    /// removal as the staleness signal that makes the next placement
    /// registration re-extract.
    pub fn remove_def_member(&mut self, member: ObjectId) {
        let had_geometry = self.def_members.remove(&member).is_some();
        // Owner-set fast path, mirroring `remove_object`/`remove_instance`:
        // reconcile calls this for every touched non-world id, and a
        // never-registered id must not pay a placement scan (see
        // `member_owners`).
        let may_have_placements = self.member_owners.remove(&member);
        if !had_geometry && !may_have_placements {
            return; // nothing registered: keep the index valid
        }
        self.placements.retain(|p| p.member != member);
        // `instance_owners` deliberately keeps ids whose last placement just
        // vanished: the set means "placements may exist", so a stale entry
        // only costs one wasted retain pass on a later remove_instance.
        *self.spatial.get_mut() = None;
    }

    /// Registers one placement of definition member `member` by `instance`
    /// at `pose`. **Additive** — an instance places every member of its
    /// definition, one placement each; the caller clears an instance's prior
    /// placements with [`InferenceScene::remove_instance`] before
    /// re-registering. The member's geometry should already be registered
    /// ([`InferenceScene::set_def_member`]); a placement of an unregistered
    /// member is inert (it emits no candidates) until the geometry arrives.
    pub fn add_placement(&mut self, instance: InstanceId, member: ObjectId, pose: &Transform) {
        debug_assert!(
            self.has_def_member(member),
            "register the member's geometry before placing it"
        );
        // Record the owners (idempotent): placements for `instance` and for
        // `member` now exist, which is exactly what `remove_instance`'s and
        // `remove_def_member`'s fast paths key on.
        self.instance_owners.insert(instance);
        self.member_owners.insert(member);
        self.placements.push(Placement {
            instance,
            member,
            pose: *pose,
        });
        *self.spatial.get_mut() = None;
    }

    /// Extracts `object`'s vertices/edges/faces into world-space candidates
    /// owned by `owner` — the [`InferenceScene::add_object`] path (world
    /// solids only; instanced geometry lives in shared definition storage,
    /// see [`InferenceScene::set_def_member`]).
    fn register(&mut self, object: &Object, placement: &Transform, owner: ObjectId) {
        // The candidate Vecs are about to change shape: drop the spatial
        // index and let the next query rebuild it. Invalidation is
        // per-committed-op (this is never called per-frame), so the rebuild
        // amortizes across the many pointer-move queries in between.
        *self.spatial.get_mut() = None;

        // --- Vertices -> ScenePoint (Endpoint source) ---
        for (vid, vertex) in object.vertices() {
            self.points.push(ScenePoint {
                position: placement.apply_point(vertex.position),
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Vertex(vid),
                    instance: None,
                },
            });
        }

        // --- Edges -> SceneSegment (midpoints derived at query time) ---
        for (eid, edge) in object.edges() {
            // Each edge references a half-edge; get its two endpoint vertices.
            let half_edges = object.half_edges();
            let he = &half_edges[edge.half_edge];
            let origin_vid = he.origin;
            let dest_vid = half_edges[he.next].origin;
            let a = placement.apply_point(object.vertices()[origin_vid].position);
            let b = placement.apply_point(object.vertices()[dest_vid].position);
            self.segments.push(SceneSegment {
                a,
                b,
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Edge(eid),
                    instance: None,
                },
            });
        }

        // --- Faces -> SceneFace (plane + outer-loop boundary) ---
        for (fid, face) in object.faces() {
            // Apply placement to the plane via apply_plane (handles normals
            // under non-uniform scale via inverse-transpose — KERNEL_GUIDE trap).
            let world_plane = match placement.apply_plane(&face.plane) {
                Ok(p) => p,
                Err(_) => continue, // singular placement: skip this face
            };
            // Boundary: outer loop positions transformed into world space.
            let boundary: Vec<Point3> = object
                .loop_positions(face.outer_loop)
                .map(|p| placement.apply_point(p))
                .collect();
            // Holes: each inner loop transformed into world space.
            let holes: Vec<Vec<Point3>> = face
                .inner_loops
                .iter()
                .map(|&lid| {
                    object
                        .loop_positions(lid)
                        .map(|p| placement.apply_point(p))
                        .collect()
                })
                .collect();
            self.faces.push(SceneFace {
                plane: world_plane,
                boundary,
                holes,
                surface_facet: face.surface.is_some(),
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Face(fid),
                    instance: None,
                },
            });
        }

        // --- Analytic rims -> Center / Quadrant / Tangent candidates ---
        // Derived from the object's surface references
        // (the true-curves design): each claimed cylinder's two rim
        // circles yield a Center at the exact axis point, Quadrant points
        // over the covered angular range, and — when the placement
        // preserves circles — the rim itself for anchor-based tangent
        // resolution at query time.
        // Circles drawn on a face (imprinted circle edges, `edge_curve_rims`)
        // offer the same candidates as wall rims: a center, covered quadrants,
        // and a tangent rim — a drawn circle on a face snaps like one on the
        // ground.
        let similarity = placement.similarity_scale();
        // Polygons drawn on a face offer their centers, as on the ground.
        for (c, face) in object.edge_polygon_centers() {
            self.centers.push(ScenePoint {
                position: placement.apply_point(c),
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Face(face),
                    instance: None,
                },
            });
        }
        let walls = object.analytic_rims();
        let drawn = object.edge_curve_rims_with(&walls);
        for rim in walls.into_iter().chain(drawn) {
            // A rim with zero surviving arc (a slant-cut station) offers no
            // candidates AT ALL: its center would be the center of no
            // surviving circle, its quadrant set is empty by construction,
            // and tangency has no arc to touch. Same gate as the kernel's
            // own `analytic_cap_centers`.
            if !rim.has_coverage() {
                continue;
            }
            let source = SnapSource {
                object: owner,
                element: ElementRef::Face(rim.rep),
                instance: None,
            };
            self.centers.push(ScenePoint {
                position: placement.apply_point(rim.center),
                source,
            });
            // Quadrant points transform as plain points: under any affine
            // placement they stay on the (possibly elliptical) rim curve.
            for q in rim.quadrant_points() {
                self.quadrants.push(ScenePoint {
                    position: placement.apply_point(q),
                    source,
                });
            }
            // Tangency needs a genuine circle: map the rim under
            // similarities, drop it otherwise (never approximate — the
            // kernel's map-or-drop rule, applied at the query layer).
            if let Some(scale) = similarity {
                let map_unit = |w: Vec3| placement.apply_vector(w).normalized();
                if let (Ok(axis), Ok(u), Ok(v)) = (
                    map_unit(rim.axis),
                    map_unit(rim.basis_u),
                    map_unit(rim.basis_v),
                ) {
                    self.rims.push(SceneRim {
                        center: placement.apply_point(rim.center),
                        axis,
                        radius: rim.radius * scale,
                        u,
                        v,
                        coverage: rim.coverage.clone(),
                        source,
                    });
                }
            }
        }
    }

    /// Drops all **world-object** candidates registered for `id` (instanced
    /// candidates are keyed by instance, see [`InferenceScene::remove_instance`]).
    /// Unknown ids are a no-op — removal must be idempotent so document undo can
    /// call it freely, and the no-op never scans candidates (see
    /// [`InferenceScene::removal_candidates_visited`]).
    pub fn remove_object(&mut self, id: ObjectId) {
        // Owner-set fast path: candidates matching `world` below exist only
        // for ids in `world_owners` (`register` emits `instance == None`
        // candidates solely on the `add_object` path, which inserts). For a
        // never-registered id the retain passes would walk every candidate
        // to remove nothing, so idempotent callers — document load and
        // undo/redo re-register N objects, each `add_*` starting with this
        // removal — went accidentally quadratic. Nothing is removed here, no
        // index shifts, so the spatial index stays valid too: skip the dirty.
        if !self.world_owners.remove(&id) {
            return;
        }
        // The retain passes shift every index behind the removed candidates,
        // so the whole spatial index is stale: mark it dirty for a lazy full
        // rebuild on the next query (per-committed-op, never per-frame).
        *self.spatial.get_mut() = None;
        self.removal_visits += (self.points.len()
            + self.segments.len()
            + self.faces.len()
            + self.centers.len()
            + self.quadrants.len()
            + self.rims.len()) as u64;
        let world = |s: &SnapSource| s.object == id && s.instance.is_none();
        self.points.retain(|p| !world(&p.source));
        self.segments.retain(|s| !world(&s.source));
        self.faces.retain(|f| !world(&f.source));
        self.centers.retain(|c| !world(&c.source));
        self.quadrants.retain(|c| !world(&c.source));
        self.rims.retain(|r| !world(&r.source));
    }

    /// Drops all placements registered for `instance` (across every
    /// definition member it places); the member geometry itself stays, since
    /// other instances (or a later re-registration) share it. Idempotent, so
    /// document undo can call it freely — and removal never scans
    /// *candidates* at all (see
    /// [`InferenceScene::removal_candidates_visited`]): placements are
    /// lightweight records, so even the non-empty case is one retain pass
    /// over the placement Vec, not the candidate storage.
    pub fn remove_instance(&mut self, instance: InstanceId) {
        // Owner-set fast path, mirroring `remove_object`: placements for
        // `instance` exist only for ids in `instance_owners`
        // (`add_placement` inserts before pushing), so a never-registered id
        // has nothing to remove — return without touching the still-valid
        // spatial index.
        if !self.instance_owners.remove(&instance) {
            return;
        }
        self.placements.retain(|p| p.instance != instance);
        *self.spatial.get_mut() = None;
    }

    /// Drops every object-sourced candidate and every placement at once,
    /// leaving guides, sketches, transient segments — and shared definition
    /// geometry — registered. For bulk visibility rebuilds (e.g. applying a
    /// whole hidden set): removing N registered owners one at a time scans
    /// the world-candidate vectors once per owner, while clearing and
    /// re-registering the visible remainder is one linear pass in total —
    /// each re-registration's replace-semantics removal hits the empty-owner
    /// fast path, and each placement re-registration reuses the surviving
    /// definition geometry ([`InferenceScene::has_def_member`]) instead of
    /// re-extracting it. Definition geometry is safe to keep precisely
    /// because visibility changes never alter geometry; callers switching
    /// documents start from a fresh scene (see the `def_members` field
    /// docs).
    pub fn clear_solids(&mut self) {
        *self.spatial.get_mut() = None;
        self.points.clear();
        self.segments.clear();
        self.faces.clear();
        self.centers.clear();
        self.quadrants.clear();
        self.rims.clear();
        self.world_owners.clear();
        self.instance_owners.clear();
        self.member_owners.clear();
        self.placements.clear();
    }

    /// Registers (or re-registers) one construction guide as a
    /// snap target. Replace semantics, mirroring [`InferenceScene::add_object`]:
    /// drops any prior candidate for `id` first, so callers can call this on
    /// every guide creation/edit without tracking whether it's new.
    pub fn add_guide(&mut self, id: GuideId, guide: &Guide) {
        self.remove_guide(id);
        let geom = match *guide {
            Guide::Line {
                origin,
                direction,
                span,
            } => SceneGuideGeom::Line {
                origin,
                direction,
                span,
            },
            Guide::Point { position } => SceneGuideGeom::Point { position },
        };
        self.guides.push(SceneGuide { id, geom });
    }

    /// Drops the candidate registered for guide `id`. Unknown ids are a
    /// no-op — removal must be idempotent so document undo/redo can call it
    /// freely (mirroring [`InferenceScene::remove_object`]).
    pub fn remove_guide(&mut self, id: GuideId) {
        self.guides.retain(|g| g.id != id);
    }

    /// Number of guides currently registered — cheap introspection for tests
    /// (kept separate from [`InferenceScene::candidate_counts`] so that
    /// existing callers of the points/segments/faces tuple are unaffected).
    pub fn guide_count(&self) -> usize {
        self.guides.len()
    }

    /// Registers (or re-registers) the committed segments of sketch `id` as
    /// snap candidates: each segment's endpoints and derived midpoint resolve
    /// exactly like a kernel edge's, but with `source: None` (sketch elements
    /// aren't selectable in this phase — no `ElementRef` variant exists for
    /// them yet). Replace semantics, mirroring [`InferenceScene::add_object`]:
    /// drops any prior candidates for `id` first, so callers can call this on
    /// every sketch mutation (add/remove segment, extrude) without tracking
    /// whether `id` was already registered.
    ///
    /// Plain-line convenience: every segment is registered with no owning
    /// curve chain. Callers that have curve-facet segments (a drawn circle's
    /// or arc's chords) use [`InferenceScene::add_sketch_edges`] so a facet
    /// vertex's Endpoint snap can name its curve.
    pub fn add_sketch(&mut self, id: SketchId, segments: &[(SketchEdgeId, Point3, Point3)]) {
        self.remove_sketch(id);
        self.sketch_segments.extend(
            segments
                .iter()
                .map(|&(eid, a, b)| (id, eid, None, BareSegment { a, b })),
        );
        self.note_sketch_kind(id, SKETCH_SEGMENTS);
    }

    pub fn add_sketch_edges(
        &mut self,
        id: SketchId,
        segments: &[(SketchEdgeId, Option<SketchCurveId>, Point3, Point3)],
    ) {
        self.remove_sketch(id);
        self.sketch_segments.extend(
            segments
                .iter()
                .map(|&(eid, cid, a, b)| (id, eid, cid, BareSegment { a, b })),
        );
        self.note_sketch_kind(id, SKETCH_SEGMENTS);
    }

    pub fn add_sketch_vertices(&mut self, id: SketchId, vertices: &[(SketchVertexId, Point3)]) {
        if self.drop_sketch_kind(id, SKETCH_VERTICES) {
            self.sketch_vertices.retain(|(sid, _, _)| *sid != id);
        }
        self.sketch_vertices
            .extend(vertices.iter().map(|&(vid, p)| (id, vid, p)));
        self.note_sketch_kind(id, SKETCH_VERTICES);
    }

    pub fn add_sketch_curves(&mut self, id: SketchId, rims: &[SketchCurveRim]) {
        if self.drop_sketch_kind(id, SKETCH_RIMS) {
            self.sketch_rims.retain(|(sid, _)| *sid != id);
        }
        self.sketch_rims
            .extend(rims.iter().map(|r| (id, r.clone())));
        self.note_sketch_kind(id, SKETCH_RIMS);
    }

    pub fn add_sketch_polygon_centers(
        &mut self,
        id: SketchId,
        centers: &[(SketchCurveId, Point3)],
    ) {
        if self.drop_sketch_kind(id, SKETCH_POLYGON_CENTERS) {
            self.sketch_polygon_centers.retain(|(sid, _, _)| *sid != id);
        }
        self.sketch_polygon_centers
            .extend(centers.iter().map(|&(cid, c)| (id, cid, c)));
        self.note_sketch_kind(id, SKETCH_POLYGON_CENTERS);
    }

    pub fn add_sketch_faces(&mut self, id: SketchId, faces: &[SketchRegionFace]) {
        if self.drop_sketch_kind(id, SKETCH_FACES) {
            self.sketch_faces.retain(|(sid, ..)| *sid != id);
        }
        self.sketch_faces.extend(
            faces
                .iter()
                .map(|f| (id, f.clone(), region_aabb(&f.plane, &f.boundary))),
        );
        self.note_sketch_kind(id, SKETCH_FACES);
    }

    /// Records that `id` now has candidates of `kind` registered.
    fn note_sketch_kind(&mut self, id: SketchId, kind: u8) {
        *self.sketch_kinds.entry(id).or_insert(0) |= kind;
    }

    /// Forgets that `id` has candidates of `kind`; `true` when it had some
    /// (so the caller must walk that kind's vector to drop them).
    fn drop_sketch_kind(&mut self, id: SketchId, kind: u8) -> bool {
        let Some(kinds) = self.sketch_kinds.get_mut(&id) else {
            return false;
        };
        let had = *kinds & kind != 0;
        *kinds &= !kind;
        if *kinds == 0 {
            self.sketch_kinds.remove(&id);
        }
        had
    }

    /// Unregisters every candidate of one sketch, walking only the vectors
    /// it has entries in (none, for a sketch never registered).
    pub fn remove_sketch(&mut self, id: SketchId) {
        let Some(kinds) = self.sketch_kinds.remove(&id) else {
            return;
        };
        if kinds & SKETCH_SEGMENTS != 0 {
            self.sketch_segments.retain(|(sid, _, _, _)| *sid != id);
        }
        if kinds & SKETCH_VERTICES != 0 {
            self.sketch_vertices.retain(|(sid, _, _)| *sid != id);
        }
        if kinds & SKETCH_RIMS != 0 {
            self.sketch_rims.retain(|(sid, _)| *sid != id);
        }
        if kinds & SKETCH_POLYGON_CENTERS != 0 {
            self.sketch_polygon_centers.retain(|(sid, _, _)| *sid != id);
        }
        if kinds & SKETCH_FACES != 0 {
            self.sketch_faces.retain(|(sid, ..)| *sid != id);
        }
    }

    /// Publishes one transient (in-progress) segment as a snap candidate —
    /// e.g. a just-placed point in the line tool's current chain, which never
    /// touches the kernel sketch until the gesture commits. Additive; callers
    /// typically call [`InferenceScene::clear_transient`] then re-publish the
    /// whole current chain each time it changes (a one-frame lag between
    /// publish and the next `resolve` is expected — see wasm-api docs).
    pub fn add_transient_segment(&mut self, a: Point3, b: Point3) {
        self.transient_segments.push(BareSegment { a, b });
    }

    /// Drops every transient segment. Idempotent.
    pub fn clear_transient(&mut self) {
        self.transient_segments.clear();
    }

    /// Answers one inference query (see the module docs for the priority and
    /// locking model). Returns `None` when nothing falls inside the pick
    /// cone and no lock/anchor produces a directional snap — the tool then
    /// uses its own fallback (e.g. ground-plane intersection).
    ///
    /// Must be cheap enough to call on every mouse-move at interactive
    /// rates: the spatial index (lazily rebuilt after committed mutations)
    /// prunes the point/segment/face candidates to a conservative superset
    /// before the exact tests run.
    pub fn resolve(&self, query: &SnapQuery) -> Option<Snap> {
        self.resolve_from(query, &|_| None)
    }

    /// [`Self::resolve`] for a scene with deferred definition members:
    /// `source` serves the object of a member the query first reaches
    /// (see [`Self::set_def_member_deferred`]).
    pub fn resolve_from(&self, query: &SnapQuery, source: MemberSource<'_>) -> Option<Snap> {
        let index = self.spatial_index();
        self.resolve_impl(query, Some(&index), source)
    }

    /// Reference implementation of [`resolve`](Self::resolve) with the
    /// spatial index bypassed — an honest full linear scan, kept so the
    /// executable specs and property tests can assert the indexed path
    /// returns byte-for-byte identical snaps (DEVELOPMENT.md rule 3). Not
    /// part of the supported API.
    #[doc(hidden)]
    pub fn resolve_linear(&self, query: &SnapQuery) -> Option<Snap> {
        self.resolve_impl(query, None, &|_| None)
    }

    /// Shared body of `resolve`/`resolve_linear`: with `index == None` every
    /// candidate is scanned; with `Some` only the index's conservative
    /// superset is, in ascending-index (= linear emission) order, so the
    /// exact tests, ranking, tie-breaks, and occlusion behave identically on
    /// both paths.
    fn resolve_impl(
        &self,
        query: &SnapQuery,
        index: Option<&SceneIndex>,
        source: MemberSource<'_>,
    ) -> Option<Snap> {
        // Normalize the ray direction; degenerate direction -> None.
        let dir = match query.ray.direction.normalized() {
            Ok(d) => d,
            Err(_) => return None,
        };
        let origin = query.ray.origin;
        let aperture = query.aperture;
        let mode = query.aperture_mode;
        let weights = query.weights;
        // The soft-axis (anchor-relative `OnAxis`) candidate's own angular
        // tolerance, optionally scaled for this query (playtest-2 review
        // finding E — see `SnapQuery::soft_axis_aperture_scale`'s doc).
        // Computed once here so BOTH the candidate-generation cone test and
        // the ranking pass's "extended" (reach-never-steals) reference below
        // agree on the same widened value.
        let soft_axis_aperture = match query.soft_axis_aperture_scale {
            Some(scale) if scale.is_finite() && scale > 0.0 => SOFT_AXIS_APERTURE * scale,
            _ => SOFT_AXIS_APERTURE,
        };
        // How many acquire radii this query's aperture spans (the release
        // query declares 2 through the same field): the tie band below and
        // a named point's reach (`SnapWeights::base_reach`) both measure
        // against the acquire radius, not the widened one.
        let widening = match query.soft_axis_aperture_scale {
            Some(scale) if scale.is_finite() && scale > 0.0 => scale,
            _ => 1.0,
        };

        // Per-kind gravity (crate docs, *Gravity*): a candidate of `kind` is
        // ADMITTED within `w * aperture` instead of `aperture`. The angular
        // distance is stored raw — the ranking pass below is where the weight
        // divides it, so that both the plain-aperture reach test and the
        // weighted ranking read the same untouched number. With every weight
        // at GRAVITY_NEUTRAL these are exactly the old cone tests. `mode`
        // (`ApertureMode`) picks the cone/cylinder interpretation of
        // `aperture` (design camera.md §1); both closures forward it as-is.
        let wcone = |point: Point3, kind: SnapKind| -> Option<(f64, f64)> {
            cone_test(
                origin,
                dir,
                point,
                aperture * weights.reach(kind, widening),
                mode,
            )
        };
        let wsegment = |a: Point3, b: Point3, kind: SnapKind| -> Option<(Point3, f64, f64)> {
            segment_cone_hit(
                origin,
                dir,
                a,
                b,
                aperture * weights.reach(kind, widening),
                mode,
            )
        };

        // Bounds the prune shape's radius growth per unit depth for the
        // index's conservative node test — under `Cone`, `tan(aperture)`
        // (linear-in-depth cone radius); at or past a 90° half-angle the
        // cone covers the whole front half-space, so the radius prune is
        // disabled (`None`); FRAC_PI_2 is a domain bound, not a tolerance.
        // The cutoff backs off by tol::CONE_SLACK: within that band of π/2
        // the tangent is so ill-conditioned that the node test's guard band
        // (see `Aabb::maybe_in_cone`) could no longer provably cover the
        // exact test's rounding, so those cones are treated as the whole
        // front half-space too. Under `Cylinder` the prune shape's radius is
        // simply constant (`Aabb::maybe_in_cylinder` needs no such guard band
        // — a plain Euclidean distance has no trig amplification). Only
        // pruning strength is affected either way — the exact tests always
        // use their own kind's weighted aperture.
        //
        // The prune shape is widened by the largest weight among the kinds
        // the index serves (`SnapWeights::max_indexed`): a boosted kind is
        // admitted further off-axis than `aperture`, so pruning at `aperture`
        // would discard candidates the exact test accepts and `resolve` would
        // stop agreeing with `resolve_linear`. The shipped profile boosts only
        // linear-walked kinds, so it leaves this factor at 1.0.
        let prune_aperture = aperture * weights.max_indexed(widening);
        let prune_tolerance = match mode {
            ApertureMode::Cone => {
                let tan_aperture = (prune_aperture < std::f64::consts::FRAC_PI_2 - tol::CONE_SLACK)
                    .then(|| prune_aperture.tan());
                PruneTolerance::Cone(tan_aperture)
            }
            ApertureMode::Cylinder => PruneTolerance::Cylinder(Some(prune_aperture)),
        };

        // Candidate index sets. The spatial index prunes to a conservative
        // superset (the exact tests below re-filter); the linear reference
        // takes everything. World candidates are indices into the baked
        // Vecs; placed candidates are (placement, member-local) pairs
        // materialized into world space below — with the identical
        // `apply_point`/`apply_plane` per-placement baking used to run at
        // registration, so both storage schemes produce bit-equal positions.
        let (point_ids, segment_ids, face_ids) = match index {
            Some(ix) => (
                ix.points_in_cone(origin, dir, prune_tolerance),
                ix.segments_in_cone(origin, dir, prune_tolerance),
                ix.faces_crossing_ray(origin, dir),
            ),
            None => (
                (0..self.points.len()).collect::<Vec<_>>(),
                (0..self.segments.len()).collect::<Vec<_>>(),
                (0..self.faces.len()).collect::<Vec<_>>(),
            ),
        };
        let geom = |member: ObjectId| self.geometry(member, source);
        let members = Members {
            map: &self.def_members,
            geometry: &geom,
        };
        let (placed_point_ids, placed_segment_ids, placed_face_ids) = match index {
            Some(ix) => (
                ix.placed_points_in_cone(&self.placements, &members, origin, dir, prune_tolerance),
                ix.placed_segments_in_cone(
                    &self.placements,
                    &members,
                    origin,
                    dir,
                    prune_tolerance,
                ),
                ix.placed_faces_crossing_ray(&self.placements, &members, origin, dir),
            ),
            None => (
                self.all_placed(source, |g| g.points.len()),
                self.all_placed(source, |g| g.segments.len()),
                self.all_placed(source, |g| g.faces.len()),
            ),
        };
        let placed_points: Vec<ScenePoint> = placed_point_ids
            .iter()
            .filter_map(|&(pi, li)| {
                let pl = &self.placements[pi];
                Some(geom(pl.member)?.point_at(li, pl))
            })
            .collect();
        // Each placed segment's placement index rides alongside it: it is
        // the segment's owner for the edge × edge crossing test below.
        let (placed_segments, placed_segment_placements): (Vec<SceneSegment>, Vec<usize>) =
            placed_segment_ids
                .iter()
                .filter_map(|&(pi, li)| {
                    let pl = &self.placements[pi];
                    Some((geom(pl.member)?.segment_at(li, pl), pi))
                })
                .unzip();
        // Singular poses drop out here (`face_at` is `None`), exactly as
        // registration-time baking skipped them.
        let placed_faces: Vec<SceneFace> = placed_face_ids
            .iter()
            .filter_map(|&(pi, li)| {
                let pl = &self.placements[pi];
                geom(pl.member)?.face_at(li, pl)
            })
            .collect();

        // Collect all candidates that fall inside the pick cone.
        // Tuple: (kind, angular_dist, depth, position, source, direction)
        let mut candidates: Vec<Candidate> = Vec::new();

        // --- Endpoint candidates: from ScenePoints (world, then placed —
        //     ascending emission order on both index paths, so ranking ties
        //     break identically) ---
        let world_points = point_ids.iter().map(|&pi| &self.points[pi]);
        for sp in world_points.chain(placed_points.iter()) {
            if let Some((ang, depth)) = wcone(sp.position, SnapKind::Endpoint) {
                candidates.push((
                    SnapKind::Endpoint,
                    ang,
                    depth,
                    sp.position,
                    Some(Provenance::Object(sp.source)),
                    None,
                ));
            }
        }

        // --- Placed analytic candidates: each placement materializes its
        //     member's rims on the fly (few per scene, linear like the world
        //     ones below — never indexed, so the indexed and reference paths
        //     see the identical set). ---
        let mut placed_centers: Vec<ScenePoint> = Vec::new();
        let mut placed_quadrants: Vec<ScenePoint> = Vec::new();
        let mut placed_rims: Vec<SceneRim> = Vec::new();
        for pl in &self.placements {
            if let Some(dm) = self.def_members.get(&pl.member) {
                dm.rims_at(
                    pl,
                    &mut placed_centers,
                    &mut placed_quadrants,
                    &mut placed_rims,
                );
            }
        }

        // --- Center candidates: true circle centers (linear walk — few per
        //     scene and deliberately outside the spatial index, so the
        //     indexed and reference paths see the identical set). ---
        for cp in self.centers.iter().chain(placed_centers.iter()) {
            if let Some((ang, depth)) = wcone(cp.position, SnapKind::Center) {
                candidates.push((
                    SnapKind::Center,
                    ang,
                    depth,
                    cp.position,
                    Some(Provenance::Object(cp.source)),
                    None,
                ));
            }
        }

        // --- Quadrant candidates: covered cardinal points of the rim
        //     circles. Same linear-walk rationale as centers. ---
        for qp in self.quadrants.iter().chain(placed_quadrants.iter()) {
            if let Some((ang, depth)) = wcone(qp.position, SnapKind::Quadrant) {
                candidates.push((
                    SnapKind::Quadrant,
                    ang,
                    depth,
                    qp.position,
                    Some(Provenance::Object(qp.source)),
                    None,
                ));
            }
        }

        // --- Sketch-curve candidates: a drawn (unextruded) circle or arc
        //     offers its exact center and covered quadrant points, so
        //     Center/Quadrant snapping exists BEFORE any extrusion. They carry
        //     `Provenance::SketchCurve` — not an Object `SnapSource` (a sketch
        //     curve is not an Object element) and not a `SketchEdge` (a
        //     center lies on no edge), but the curve CHAIN they describe, so a
        //     click on one selects the same curve clicking its rim does.
        //     Linear walk, like `centers`. ---
        for (sid, rim) in &self.sketch_rims {
            let prov = Some(Provenance::SketchCurve(*sid, rim.curve));
            if let Some((ang, depth)) = wcone(rim.center, SnapKind::Center) {
                candidates.push((SnapKind::Center, ang, depth, rim.center, prov, None));
            }
            for q in rim.quadrant_points() {
                if let Some((ang, depth)) = wcone(q, SnapKind::Quadrant) {
                    candidates.push((SnapKind::Quadrant, ang, depth, q, prov, None));
                }
            }
        }

        // --- Polygon centers: a drawn regular polygon's exact center, the
        //     one analytic point its circumcircle legitimately supplies. Same
        //     kind, same curve-chain provenance, same linear walk as the rim
        //     centers above — a polygon simply contributes no quadrants or
        //     tangents. ---
        for (sid, cid, c) in &self.sketch_polygon_centers {
            if let Some((ang, depth)) = wcone(*c, SnapKind::Center) {
                candidates.push((
                    SnapKind::Center,
                    ang,
                    depth,
                    *c,
                    Some(Provenance::SketchCurve(*sid, *cid)),
                    None,
                ));
            }
        }

        // --- Tangent candidates: for each rim circle, the two points where
        //     a segment from the tool's anchor touches the exact circle —
        //     computed per query (they depend on the anchor), offered only
        //     over the covered angular range, and only when the anchor lies
        //     strictly outside the circle in its own plane. Linear walk,
        //     like centers. ---
        if let Some(anchor) = query.anchor {
            for rim in self.rims.iter().chain(placed_rims.iter()) {
                let Some(angles) =
                    tangent_angles(anchor, rim.center, rim.axis, rim.radius, rim.u, rim.v)
                else {
                    continue; // anchor inside or on the circle: no tangent
                };
                for angle in angles {
                    if !rim.covers(angle) {
                        continue;
                    }
                    let pos = rim.point_at(angle);
                    if let Some((ang, depth)) = wcone(pos, SnapKind::Tangent) {
                        candidates.push((
                            SnapKind::Tangent,
                            ang,
                            depth,
                            pos,
                            Some(Provenance::Object(rim.source)),
                            None,
                        ));
                    }
                }
            }
            // Sketch-curve rims tangent-snap identically, carrying the same
            // curve-chain provenance (see the Center/Quadrant walk above) —
            // a tangent point lies exactly ON the drawn curve, so it names
            // the same curve its center and quadrants do.
            for (sid, rim) in &self.sketch_rims {
                let Some(angles) = tangent_angles(
                    anchor,
                    rim.center,
                    rim.axis,
                    rim.radius,
                    rim.basis_u,
                    rim.basis_v,
                ) else {
                    continue;
                };
                for angle in angles {
                    if !rim.covers(angle) {
                        continue;
                    }
                    let pos = rim.center
                        + rim.basis_u * (rim.radius * angle.cos())
                        + rim.basis_v * (rim.radius * angle.sin());
                    if let Some((ang, depth)) = wcone(pos, SnapKind::Tangent) {
                        candidates.push((
                            SnapKind::Tangent,
                            ang,
                            depth,
                            pos,
                            Some(Provenance::SketchCurve(*sid, rim.curve)),
                            None,
                        ));
                    }
                }
            }
        }

        // The edges that pass through the pick cone at `Intersection`'s
        // reach, with their owners: the pool the edge × edge crossing test
        // below pairs up. A crossing inside the cone lies on both of its
        // edges, so each of them passes at least that close to the ray —
        // nothing outside this pool can contribute an admissible crossing.
        // When the reach equals `OnEdge`'s (always, unless a caller weights
        // the two differently) the OnEdge test's own answer decides it.
        let crossing_reach_shared = weights.reach(SnapKind::Intersection, widening)
            == weights.reach(SnapKind::OnEdge, widening);
        let mut crossing_pool: Vec<CrossingEdge> = Vec::new();

        // --- Segment candidates: Midpoint and OnEdge (world, then placed) ---
        let world_segments = || segment_ids.iter().map(|&si| &self.segments[si]);
        let world_segment_count = segment_ids.len();
        for (k, seg) in world_segments().chain(placed_segments.iter()).enumerate() {
            let mid = midpoint(seg.a, seg.b);

            // Midpoint candidate: emitted when the midpoint itself is in the cone.
            if let Some((ang, depth)) = wcone(mid, SnapKind::Midpoint) {
                candidates.push((
                    SnapKind::Midpoint,
                    ang,
                    depth,
                    mid,
                    Some(Provenance::Object(seg.source)),
                    None,
                ));
            }

            // OnEdge candidate: the closest point on the segment to the ray,
            // if it lies within the cone. Emit even when the midpoint is also
            // in the cone — priority ranking handles "Midpoint beats OnEdge".
            let on_edge = wsegment(seg.a, seg.b, SnapKind::OnEdge);
            let near = if crossing_reach_shared {
                on_edge
            } else {
                wsegment(seg.a, seg.b, SnapKind::Intersection)
            };
            if let Some((_, ang, _)) = near {
                let owner = match k.checked_sub(world_segment_count) {
                    None => EdgeOwner::World(seg.source.object),
                    Some(placed) => EdgeOwner::Placement(placed_segment_placements[placed]),
                };
                crossing_pool.push(CrossingEdge {
                    a: seg.a,
                    b: seg.b,
                    ang,
                    owner,
                    edge: CrossedEdge::Object(seg.source),
                });
            }
            if let Some((pos, ang, depth)) = on_edge {
                // Skip if this is the same point as the midpoint (it would be
                // a duplicate; the Midpoint candidate already covers it with
                // the stronger kind).
                if !pos.approx_eq(mid, tol::POINT_MERGE) {
                    candidates.push((
                        SnapKind::OnEdge,
                        ang,
                        depth,
                        pos,
                        Some(Provenance::Object(seg.source)),
                        None,
                    ));
                }
            }
        }

        // --- Sketch and transient segment candidates: Endpoint, Midpoint,
        //     and OnEdge. A committed sketch segment's Midpoint/OnEdge carry
        //     its (SketchId, SketchEdgeId) so tools can use the edge as a
        //     direction reference (Tape Measure parallel guides). Its
        //     ENDPOINTS instead carry the drawn CURVE they facet, when there
        //     is one (a circle/arc chord): a vertex is not a direction
        //     reference, but it IS part of the curve, so clicking a circle on
        //     a facet vertex selects the curve — the vertex-side analogue of
        //     the Center/Quadrant fix above. A plain line's endpoints, and
        //     transient segments, carry nothing. ---
        let bare_segments = self
            .sketch_segments
            .iter()
            .map(|&(sid, eid, cid, ref seg)| {
                let edge_prov = Some(Provenance::SketchEdge(sid, eid));
                let endpoint_prov = cid.map(|c| Provenance::SketchCurve(sid, c));
                (edge_prov, endpoint_prov, seg)
            })
            .chain(self.transient_segments.iter().map(|seg| (None, None, seg)));
        for (edge_prov, endpoint_prov, seg) in bare_segments {
            for endpoint in [seg.a, seg.b] {
                if let Some((ang, depth)) = wcone(endpoint, SnapKind::Endpoint) {
                    candidates.push((
                        SnapKind::Endpoint,
                        ang,
                        depth,
                        endpoint,
                        endpoint_prov,
                        None,
                    ));
                }
            }

            let mid = midpoint(seg.a, seg.b);
            if let Some((ang, depth)) = wcone(mid, SnapKind::Midpoint) {
                candidates.push((SnapKind::Midpoint, ang, depth, mid, edge_prov, None));
            }

            let on_edge = wsegment(seg.a, seg.b, SnapKind::OnEdge);
            // Committed sketch edges join the crossing pool, owned by their
            // sketch; a transient (in-progress) segment has no owner yet and
            // stays out.
            if let Some(Provenance::SketchEdge(sid, eid)) = edge_prov {
                let near = if crossing_reach_shared {
                    on_edge
                } else {
                    wsegment(seg.a, seg.b, SnapKind::Intersection)
                };
                if let Some((_, ang, _)) = near {
                    crossing_pool.push(CrossingEdge {
                        a: seg.a,
                        b: seg.b,
                        ang,
                        owner: EdgeOwner::Sketch(sid),
                        edge: CrossedEdge::Sketch(sid, eid),
                    });
                }
            }
            if let Some((pos, ang, depth)) = on_edge
                && !pos.approx_eq(mid, tol::POINT_MERGE)
            {
                candidates.push((SnapKind::OnEdge, ang, depth, pos, edge_prov, None));
            }
        }

        // --- Edge × edge crossings between different owners (#51b) ---
        // Separate Objects never merge, so where an edge of one passes
        // through an edge of another there is no vertex — yet the crossing
        // is exactly what a user drawing between them aims at. Every pair of
        // pooled edges (above: the ones through the pick cone) from
        // DIFFERENT owners — two Objects, two placements, an Object and a
        // sketch, two sketches — is tested for a true 3D crossing
        // (`segment_crossing`: closest points within `tol::POINT_MERGE`,
        // strictly inside both edges, since an endpoint has its own
        // Endpoint snap). Edges of one owner never pair: the kernel splits
        // edges within an Object or a sketch, so a crossing there is
        // already a vertex. Snapped as `Intersection`, the same tier as the
        // guide and axis crossings, with both edges as provenance.
        //
        // At most `CROSSING_PAIR_CAP` steps are taken, nearest edges first:
        // the pool is ordered by each edge's angular distance from the ray
        // (stable, so ties keep emission order and the result is
        // deterministic), and every pair among the first `j` edges is
        // tested before any pair involving edge `j + 1`. Only cross-owner
        // pairs are visited (`walk_cross_owner_pairs`), so a dense spot of
        // one Object costs nothing here. A crossing in the cone lies on two
        // edges near the ray, so in a dense spot the crossing under the
        // cursor is still found; what the cap leaves untested is counted
        // and traced, never dropped silently.
        let two_owners = crossing_pool
            .first()
            .is_some_and(|first| crossing_pool.iter().any(|e| e.owner != first.owner));
        if two_owners {
            crossing_pool.sort_by(|x, y| x.ang.total_cmp(&y.ang));
        }
        let walk = walk_cross_owner_pairs(
            &crossing_pool,
            |e| e.owner,
            CROSSING_PAIR_CAP,
            |e, f| {
                if let Some(p) = segment_crossing(e.a, e.b, f.a, f.b)
                    && let Some((ang, depth)) = wcone(p, SnapKind::Intersection)
                {
                    candidates.push((
                        SnapKind::Intersection,
                        ang,
                        depth,
                        p,
                        Some(Provenance::Crossing(e.edge, f.edge)),
                        None,
                    ));
                }
            },
        );
        if walk.steps > 0 {
            let mut stats = self.crossing_stats.get();
            stats.pairs_tested += walk.tested as u64;
            stats.pair_steps += walk.steps as u64;
            if walk.skipped > 0 {
                stats.queries_capped += 1;
                stats.pairs_skipped += walk.skipped as u64;
                tracing::debug!(
                    target: "inference::crossings",
                    edges = crossing_pool.len(),
                    skipped = walk.skipped,
                    cap = CROSSING_PAIR_CAP,
                    "edge crossing pairs past the cap left untested",
                );
            }
            self.crossing_stats.set(stats);
        }

        // --- Face candidates: OnFace (world, then placed) ---
        // Gravity does not apply here: `face_cone_hit` is a ray-vs-face
        // intersection that ignores the aperture entirely and reports angular
        // distance 0 (the face IS under the cursor), so scaling an aperture it
        // never reads, and dividing a zero, would both be no-ops. An `OnFace`
        // weight is therefore inert by construction — the same reason
        // `SnapWeights::max_indexed` leaves OnFace out.
        // Under `FacePolicy::Absent` faces are not targets at all.
        let faces_are_targets = self.face_policy.faces_are_targets();
        let world_faces = face_ids.iter().map(|&fi| &self.faces[fi]);
        for face in world_faces
            .chain(placed_faces.iter())
            .filter(|_| faces_are_targets)
        {
            if let Some((pos, ang, depth)) = face_cone_hit(
                origin,
                dir,
                &face.plane,
                &face.boundary,
                &face.holes,
                aperture,
            ) {
                candidates.push((
                    SnapKind::OnFace,
                    ang,
                    depth,
                    pos,
                    Some(Provenance::Object(face.source)),
                    None,
                ));
            }
        }

        // --- Sketch region faces: OnFace, carrying sketch-region provenance ---
        // A drawn (unextruded) region is a first-class hoverable face: the
        // cursor snaps on its fill exactly like a solid's face, and the
        // occlusion cull below (which now counts these too) stops the ray
        // passing through it to the ground/box behind. It has no Object
        // `source`, but it DOES carry `SketchRegion` provenance so a click on
        // the fill can resolve to this exact region — letting the Select
        // tool's click match this occlusion-aware hover (interior fill → the
        // region; a nearer region beats a solid behind it).
        for (sid, face, _) in self.sketch_faces.iter().filter(|_| faces_are_targets) {
            if let Some((pos, ang, depth)) = face_cone_hit(
                origin,
                dir,
                &face.plane,
                &face.boundary,
                &face.holes,
                aperture,
            ) {
                candidates.push((
                    SnapKind::OnFace,
                    ang,
                    depth,
                    pos,
                    Some(Provenance::SketchRegion(*sid, face.region)),
                    None,
                ));
            }
        }

        // --- Faces the pick cone reaches: the pool of the two face walks ---
        // The edge × face piercing walk and the face × face meeting-line
        // walk below both pair faces the pick CONE reaches — not the ray
        // query the On Face candidates above use: the ray need not hit a
        // face for a piercing or a meeting line on it to be under the
        // cursor. The spatial index prunes world and placed faces to a
        // superset by box (`faces_in_cone`, `placed_faces_in_cone`); then
        // every face — world, placed, and the linearly walked drawn regions
        // alike — is kept only if some point of its material or boundary
        // actually passes within the cone (`face_cone_key`), the exact test
        // both the indexed and the linear reference path apply, so the two
        // pool the same faces in the same order and agree even when a cap
        // cuts a walk short. Curved surfaces are their planar facets here as
        // everywhere in this crate. Faces are targets only under a face
        // policy that has them (`faces_are_targets`): in Wireframe faces do
        // not exist for input, piercings and meeting lines included; in
        // X-ray they do, and the occlusion cull being off there makes a
        // point inside a solid reachable, exactly like every other point.
        //
        // The key is computed once per face, at the larger of the two walks'
        // reaches, and each walk keeps the faces within its own: the key is
        // the face's distance from the ray wherever it is admitted at all,
        // the same number at any aperture that admits it (`face_cone_key`),
        // so each walk pools exactly the faces it would have pooled alone.
        //
        // Nothing is pooled unless it can pair. The piercing walk needs an
        // edge in the cone and a face of another owner than some pooled
        // edge's; the meeting walk needs faces of two owners in the cone.
        // Both are decided on the box-pruned owners before any face is
        // materialized or keyed — solid faces pruned by the index, drawn
        // regions by the box each carries — so hovering one Object's own
        // geometry costs the cone walk and nothing more, however many
        // regions are drawn elsewhere. (On the linear reference path the
        // box prune is absent and this gate sees every owner in the scene;
        // it only gates work, and a face the exact test admits lies in its
        // own box, so the pools agree on both paths.) Placed faces
        // materialize on demand — every one the cone reaches when the
        // meeting walk runs, else only the foreign ones — each with its
        // placement index, its owner.
        let pierce_reach = aperture * weights.reach(SnapKind::Intersection, widening);
        // The meeting walk offers a line's points at On Intersection Line's
        // reach and its middle at Midpoint's (a point kind, so the wider):
        // its faces are pooled at the larger, or a midpoint the cursor is
        // within reach of would go unoffered whenever the line itself is
        // past a plain aperture. The middle lies on both faces, so both
        // pass within its distance of the ray.
        let meeting_reach = aperture
            * weights
                .reach(SnapKind::OnIntersectionLine, widening)
                .max(weights.reach(SnapKind::Midpoint, widening));
        let pool_owners: BTreeSet<EdgeOwner> = crossing_pool.iter().map(|e| e.owner).collect();
        let foreign = |owner: EdgeOwner| pool_owners.len() > 1 || !pool_owners.contains(&owner);
        let pierce_wanted = faces_are_targets && !crossing_pool.is_empty();
        // A selection pick asks what is under the cursor; a meeting line is
        // two owners' faces at once, nothing to select
        // (`SnapKind::OnIntersectionLine`).
        let meeting_wanted = faces_are_targets && !weights.picking;
        if pierce_wanted || meeting_wanted {
            let world_cone_ids = match index {
                Some(ix) => ix.faces_in_cone(origin, dir, prune_tolerance),
                None => (0..self.faces.len()).collect::<Vec<_>>(),
            };
            let placed_cone_ids = match index {
                Some(ix) => ix.placed_faces_in_cone(
                    &self.placements,
                    &members,
                    origin,
                    dir,
                    prune_tolerance,
                ),
                None => self.all_placed(source, |g| g.faces.len()),
            };
            // Drawn regions are walked linearly, never indexed, but each
            // carries its box, so the indexed path prunes them by the same
            // superset test the index applies to solid faces; the linear
            // reference keeps them all, as it keeps every solid face.
            let sketch_cone_ids: Vec<usize> = match index {
                Some(_) => self
                    .sketch_faces
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, _, bx))| bx.admits(origin, dir, prune_tolerance))
                    .map(|(i, _)| i)
                    .collect(),
                None => (0..self.sketch_faces.len()).collect(),
            };
            let cone_owners: BTreeSet<EdgeOwner> = world_cone_ids
                .iter()
                .map(|&fi| EdgeOwner::World(self.faces[fi].source.object))
                .chain(
                    placed_cone_ids
                        .iter()
                        .map(|&(pi, _)| EdgeOwner::Placement(pi)),
                )
                .chain(
                    sketch_cone_ids
                        .iter()
                        .map(|&si| EdgeOwner::Sketch(self.sketch_faces[si].0)),
                )
                .collect();
            let meeting_possible = meeting_wanted && cone_owners.len() > 1;
            let pierce_possible = pierce_wanted && cone_owners.iter().any(|&o| foreign(o));
            if meeting_possible || pierce_possible {
                let key_reach = match (pierce_possible, meeting_possible) {
                    (true, true) => pierce_reach.max(meeting_reach),
                    (true, false) => pierce_reach,
                    _ => meeting_reach,
                };
                let key_of = |plane: &Plane, boundary: &[Point3], holes: &[Vec<Point3>]| {
                    face_cone_key(origin, dir, plane, boundary, holes, key_reach, mode)
                };
                let include = |owner: EdgeOwner| meeting_possible || foreign(owner);
                let placed_cone: Vec<(SceneFace, usize, (f64, f64))> = placed_cone_ids
                    .iter()
                    .filter_map(|&(pi, li)| {
                        if !include(EdgeOwner::Placement(pi)) {
                            return None;
                        }
                        let pl = &self.placements[pi];
                        let face = geom(pl.member)?.face_at(li, pl)?;
                        let key = key_of(&face.plane, &face.boundary, &face.holes)?;
                        Some((face, pi, key))
                    })
                    .collect();
                let mut cone_faces: Vec<ConeFace<'_>> = Vec::new();
                for &fi in &world_cone_ids {
                    let f = &self.faces[fi];
                    let owner = EdgeOwner::World(f.source.object);
                    if !include(owner) {
                        continue;
                    }
                    if let Some((key, depth)) = key_of(&f.plane, &f.boundary, &f.holes) {
                        cone_faces.push(ConeFace {
                            plane: &f.plane,
                            boundary: &f.boundary,
                            holes: &f.holes,
                            owner,
                            face: PiercedFace::Object(f.source),
                            surface_facet: f.surface_facet,
                            key,
                            depth,
                        });
                    }
                }
                cone_faces.extend(placed_cone.iter().map(|(f, pi, (key, depth))| ConeFace {
                    plane: &f.plane,
                    boundary: &f.boundary,
                    holes: &f.holes,
                    owner: EdgeOwner::Placement(*pi),
                    face: PiercedFace::Object(f.source),
                    surface_facet: f.surface_facet,
                    key: *key,
                    depth: *depth,
                }));
                for &si in &sketch_cone_ids {
                    let (sid, f, _) = &self.sketch_faces[si];
                    let owner = EdgeOwner::Sketch(*sid);
                    if !include(owner) {
                        continue;
                    }
                    if let Some((key, depth)) = key_of(&f.plane, &f.boundary, &f.holes) {
                        cone_faces.push(ConeFace {
                            plane: &f.plane,
                            boundary: &f.boundary,
                            holes: &f.holes,
                            owner,
                            face: PiercedFace::SketchRegion(*sid, f.region),
                            surface_facet: false,
                            key,
                            depth,
                        });
                    }
                }
                // Nearest the ray first, and among faces equally near —
                // every face the ray passes through is at 0 — nearest the
                // eye first; stable, so what still ties keeps pool order
                // (world, placed, drawn regions, each ascending), the same
                // on the indexed and the linear path.
                cone_faces
                    .sort_by(|x, y| x.key.total_cmp(&y.key).then(x.depth.total_cmp(&y.depth)));

                // --- Edge × face piercings between different owners ---
                // A panel set into a wall, with nothing imprinted on the
                // wall: the panel's edge passes through the wall's face, and
                // separate Objects never merge, so there is no vertex, no
                // edge, nothing at the point where it does — hovering it
                // flickered between On Face and On Edge and never offered a
                // point. Every edge in the crossing pool (the edges through
                // the pick cone at `Intersection`'s reach — a piercing in
                // the cone lies on its edge, so that edge is in the pool) is
                // tested against every pooled face of a DIFFERENT owner
                // (`segment_pierces_face`: strictly through the plane,
                // inside the outer loop, outside the holes). An edge that
                // ends on a face is a T meeting the Endpoint snap names; an
                // edge in a face's plane has no single point; neither is
                // offered. Within one owner an edge never pierces a face: a
                // solid's edges end on its faces, and the kernel splits an
                // Object's or a sketch's own crossings into vertices.
                // Snapped as `Intersection`, the same tier as every other
                // crossing (above Midpoint, below the exact named points, so
                // a vertex at the point still wins; above On Edge and On
                // Face, so the point beats the two things it lies on),
                // carrying the edge as the first provenance and the face as
                // `pierced_face`.
                //
                // At most `PIERCE_PAIR_CAP` pairs, nearest first on both
                // sides — the edges by angular distance (the pool is sorted
                // as for the edge × edge walk, sorted here when that walk
                // had one owner and left it as emitted) and the faces by
                // how close they come to the ray — in order of the pair's
                // distance (`walk_pierce_pairs`); what the cap leaves
                // untested is counted and traced, never dropped silently.
                if pierce_possible {
                    let pierce_faces: Vec<ConeFace<'_>> = cone_faces
                        .iter()
                        .filter(|f| f.key <= pierce_reach && foreign(f.owner))
                        .copied()
                        .collect();
                    if !pierce_faces.is_empty() {
                        if !two_owners {
                            crossing_pool.sort_by(|x, y| x.ang.total_cmp(&y.ang));
                        }
                        let walk = walk_pierce_pairs(
                            &crossing_pool,
                            &pierce_faces,
                            PIERCE_PAIR_CAP,
                            |e, f| {
                                if let Some(p) =
                                    segment_pierces_face(e.a, e.b, f.plane, f.boundary, f.holes)
                                    && let Some((ang, depth)) = wcone(p, SnapKind::Intersection)
                                {
                                    candidates.push((
                                        SnapKind::Intersection,
                                        ang,
                                        depth,
                                        p,
                                        Some(Provenance::Pierce(e.edge, f.face)),
                                        None,
                                    ));
                                }
                            },
                        );
                        let mut stats = self.crossing_stats.get();
                        stats.pierce_faces_pooled += pierce_faces.len() as u64;
                        stats.pierce_pairs_tested += walk.tested as u64;
                        if walk.skipped > 0 {
                            stats.pierce_queries_capped += 1;
                            stats.pierce_pairs_skipped += walk.skipped as u64;
                            tracing::debug!(
                                target: "inference::crossings",
                                edges = crossing_pool.len(),
                                faces = pierce_faces.len(),
                                skipped = walk.skipped,
                                cap = PIERCE_PAIR_CAP,
                                "edge piercing pairs past the cap left untested",
                            );
                        }
                        self.crossing_stats.set(stats);
                    }
                }

                // --- Face × face meeting lines between different owners ---
                // A slab passing through a wall draws four lines on the
                // wall's face where the slab's faces meet it. Separate
                // Objects never merge, so none of them is an edge — there is
                // nothing on the wall but its face — yet each is a line a
                // person sees and aims at, and hovering it offered only On
                // Face until the cursor reached one of the slab's edges.
                // Every pair of pooled faces from DIFFERENT owners is cut
                // against each other (`face_meeting_segments`: the stretch
                // of the two planes' intersection line inside both faces,
                // outer loops and holes alike; parallel planes make no
                // line), and each stretch is offered as a virtual edge: the
                // point of it nearest the ray as `OnIntersectionLine` (the
                // same cone reach and rank group as On Edge, so the nearer
                // of a real edge and a meeting line wins, and a real edge
                // running along the line wins the tie), and its middle as
                // a `Midpoint`, exactly as on a real edge. Its ends need no
                // candidate here: they are where an edge of one face
                // pierces the other, or where their edges cross — the
                // `Intersection` walks above — or a vertex of one lying on
                // the other, an `Endpoint`. Faces of one owner never pair:
                // a solid's own faces meet at its own edges, and a sketch's
                // regions meet at its own lines. Both faces are the
                // provenance: the first as `source` or
                // `sketch_region_source`, the second with the stretch's
                // ends in `meeting_line`, so a cue can draw the line.
                //
                // Where either face is a chord facet of an analytic surface
                // (`ConeFace::surface_facet`: a slab through a cylinder's
                // wall), the meeting is a curve cut into one stretch per
                // facet. Each stretch is still offered as the line, so the
                // cursor slides along the curve as along a real chain of
                // facet edges, but no stretch offers its middle: the middle
                // of one facet's piece is no point of the curve anyone aims
                // at, and a curve would otherwise scatter a Midpoint over
                // every facet. (An imported mesh's facets carry no surface
                // and are planar faces like any other, as their own edges
                // are.)
                //
                // At most `MEETING_PAIR_CAP` cross-owner pairs, the faces
                // taken front to back: by the depth along the ray at which
                // each comes nearest it (`ConeFace::depth`), every pair
                // among the first `j` faces before any pair with the next
                // (`walk_cross_owner_pairs`, the edge × edge walk's order).
                // Every face the ray passes through is equally near it, so
                // nearness alone would leave a stack of faces hidden behind
                // the one in view in the order they were added, and a cap
                // filled with their pairs before the line in plain view;
                // front to back, the faces the line in view lies on — the
                // first the ray meets — pair first, and the cap cuts off
                // pairs behind them. What the cap leaves untested is
                // counted and traced, never dropped silently. A pool of one
                // owner takes no step.
                //
                // A pair is cut only when its line can come near the ray:
                // the planes' line, clipped to both faces' boxes (each box
                // taken once per face, not per pair), must pass within the
                // walk's reach of the ray (`meeting_line_reaches`), or no
                // point of a stretch, its middle included, could be offered.
                // Only then are the two faces' rings walked
                // (`meeting_segments_on`), with buffers reused across pairs.
                if meeting_possible {
                    let mut meeting_faces: Vec<MeetingFace<'_>> = cone_faces
                        .iter()
                        .filter(|f| f.key <= meeting_reach)
                        .map(|&f| MeetingFace {
                            bounds: region_aabb(f.plane, f.boundary),
                            face: f,
                        })
                        .collect();
                    meeting_faces.sort_by(|x, y| {
                        x.face
                            .depth
                            .total_cmp(&y.face.depth)
                            .then(x.face.key.total_cmp(&y.face.key))
                    });
                    let mut scratch = MeetingScratch::default();
                    let walk = walk_cross_owner_pairs(
                        &meeting_faces,
                        |m| m.face.owner,
                        MEETING_PAIR_CAP,
                        |m, n| {
                            // One order for the pair, whichever face the ray
                            // passes nearer: the smaller owner first. The
                            // walk's own order follows the cursor, and a pair
                            // that swapped as it moved would swap the line's
                            // provenance and the direction of its ends under
                            // a held snap.
                            let (m, n) = if m.face.owner < n.face.owner {
                                (m, n)
                            } else {
                                (n, m)
                            };
                            let (f, g) = (&m.face, &n.face);
                            let Some(line) = planes_meeting_line(f.plane, g.plane, f.boundary)
                            else {
                                return;
                            };
                            if !meeting_line_reaches(
                                line,
                                m.bounds,
                                n.bounds,
                                origin,
                                dir,
                                meeting_reach,
                                mode,
                            ) {
                                return;
                            }
                            let offer_middle = !(f.surface_facet || g.surface_facet);
                            meeting_segments_on(
                                line,
                                (f.plane, f.boundary, f.holes),
                                (g.plane, g.boundary, g.holes),
                                &mut scratch,
                            );
                            for &(a, b) in &scratch.out {
                                let prov = Some(Provenance::Meeting(
                                    f.face,
                                    MeetingLine { a, b, face: g.face },
                                ));
                                let mid = midpoint(a, b);
                                if offer_middle
                                    && let Some((ang, depth)) = wcone(mid, SnapKind::Midpoint)
                                {
                                    candidates.push((
                                        SnapKind::Midpoint,
                                        ang,
                                        depth,
                                        mid,
                                        prov,
                                        None,
                                    ));
                                }
                                if let Some((pos, ang, depth)) =
                                    wsegment(a, b, SnapKind::OnIntersectionLine)
                                    && !(offer_middle && pos.approx_eq(mid, tol::POINT_MERGE))
                                {
                                    candidates.push((
                                        SnapKind::OnIntersectionLine,
                                        ang,
                                        depth,
                                        pos,
                                        prov,
                                        None,
                                    ));
                                }
                            }
                        },
                    );
                    let mut stats = self.crossing_stats.get();
                    stats.meeting_faces_pooled += meeting_faces.len() as u64;
                    self.crossing_stats.set(stats);
                    if walk.steps > 0 {
                        let mut stats = self.crossing_stats.get();
                        stats.meeting_pairs_tested += walk.tested as u64;
                        if walk.skipped > 0 {
                            stats.meeting_queries_capped += 1;
                            stats.meeting_pairs_skipped += walk.skipped as u64;
                            tracing::debug!(
                                target: "inference::crossings",
                                faces = meeting_faces.len(),
                                skipped = walk.skipped,
                                cap = MEETING_PAIR_CAP,
                                "face meeting pairs past the cap left untested",
                            );
                        }
                        self.crossing_stats.set(stats);
                    }
                }
            }
        }

        // The active lock's direction, resolved and normalized ONCE, ahead
        // of candidate generation: the reach-reference gate just below
        // needs it, and the lock context further down reuses it verbatim.
        // A present-but-degenerate `SnapLock::Direction` refuses the whole
        // query, exactly as the lock context always did.
        let locked_dir: Option<Vec3> = match (query.lock, query.anchor) {
            (Some(lock), Some(_)) => {
                let raw = match lock {
                    SnapLock::Axis(Axis::X) => self.axes_frame.x,
                    SnapLock::Axis(Axis::Y) => self.axes_frame.y,
                    SnapLock::Axis(Axis::Z) => self.axes_frame.z(),
                    SnapLock::Direction(v) => v,
                };
                match raw.normalized() {
                    Ok(d) => Some(d),
                    Err(_) => return None,
                }
            }
            _ => None,
        };
        // Reach reference for a candidate LINE's clamp (see
        // `closest_point_on_line_to_ray`): the anchor ONLY when an active
        // lock is collinear with the candidate's direction — the one case
        // where the winner cull's fallback-equality test needs the two
        // computations of the same line to clamp identically — and `None`
        // for every other candidate, which keeps its own natural origin
        // and bound. An UNCONDITIONAL anchor reference was a regression in
        // the common case: an anchor exists from the first click of nearly
        // every drawing tool, and with no lock the winner's raw position
        // is returned unprojected, so an anchor sitting far along a
        // hovered axis inflated the near-edge-on clamp bound from the
        // camera-to-origin scale to the camera-to-anchor's-foot scale and
        // moved the resolved point ~100 m along the identical axis, ray
        // and camera. Collinearity reads the codebase's parallel-floor
        // convention — sin² of the angle against
        // `tol::NORMALIZE_MIN_LENGTH` — the same test
        // `closest_point_on_line_to_ray`'s own parallel branch uses.
        let reach_for = |line_dir: Vec3| -> Option<Point3> {
            match locked_dir {
                Some(ld) => {
                    let b = line_dir.dot(ld);
                    if 1.0 - b * b < tol::NORMALIZE_MIN_LENGTH {
                        query.anchor
                    } else {
                        None
                    }
                }
                None => None,
            }
        };

        // --- Drawing-axes origin and axis candidates ---
        // The origin snaps as a strong Endpoint; the three axes snap as
        // OnAxis (weakest meaningful kind, so object geometry still wins).
        // Suppressed when axes are hidden (View ▸ Axes off) so a hidden axis
        // never snaps or flashes a cue. Reads `axes_frame` (tool-parity
        // design §4), not the literal world origin/X/Y/Z — moving the
        // drawing axes moves what "on axis" means for inference exactly as
        // it does for drawing (world identity is this frame's default, so
        // an unmoved document snaps exactly as before).
        if self.axes_enabled {
            let frame_origin = self.axes_frame.origin;
            if let Some((ang, depth)) = wcone(frame_origin, SnapKind::Endpoint) {
                candidates.push((SnapKind::Endpoint, ang, depth, frame_origin, None, None));
            }
            for adir in [self.axes_frame.x, self.axes_frame.y, self.axes_frame.z()] {
                let pos =
                    closest_point_on_line_to_ray(frame_origin, adir, reach_for(adir), origin, dir);
                if let Some((ang, depth)) = wcone(pos, SnapKind::OnAxis) {
                    candidates.push((SnapKind::OnAxis, ang, depth, pos, None, Some(adir)));
                }
            }

            // --- Soft axis inference (tool-parity playtest2 §2c) ---
            // When the query carries an `anchor` AND NO active hard lock,
            // ALSO offer the three axis lines THROUGH IT: "if you're
            // dragging roughly along an axis from where you started, snap
            // to it" — SketchUp's soft axis-lock feel. Gated on
            // `query.lock.is_none()` (playtest-2 defect: a hard lock's own
            // fallback line, below, must never be hijacked by a DIFFERENT
            // axis's soft candidate winning the ranking and getting
            // projected onto the locked line — see that branch's doc for
            // why the projection collapses to exactly `anchor` when it is).
            // Uses its OWN, wider `SOFT_AXIS_APERTURE` rather than
            // `wcone`'s query-derived `aperture` — seeing "roughly this
            // way" is a categorically looser question than point-snap
            // precision (see that constant's doc) — optionally scaled by
            // `query.soft_axis_aperture_scale` (playtest-2 review finding
            // E), the app's magnetic-hysteresis release query's only way to
            // reach this candidate at all, since it never reads the widened
            // `aperture` that hysteresis normally works through.
            if let (Some(anchor), None) = (query.anchor, query.lock) {
                for adir in [self.axes_frame.x, self.axes_frame.y, self.axes_frame.z()] {
                    // Edge-on guard: skip a candidate axis the ray is
                    // sighting nearly straight down BEFORE computing its
                    // closest point at all — `closest_point_on_line_to_ray`
                    // only guards EXACT parallelism, and merely
                    // near-parallel already makes its result swing
                    // arbitrarily far for a pixel of mouse motion.
                    if axis_is_edge_on(dir, adir) {
                        continue;
                    }
                    // Admission uses `soft_axis_deviation` — a
                    // camera-position-independent angle, NOT a plain
                    // `cone_test` on the (potentially very distant) point
                    // this axis line comes closest to the ray at (see that
                    // function's doc for the tool-parity playtest-2 defect
                    // this replaces: for anything but a near-straight-
                    // overhead camera, `cone_test`'s eye-relative angle lets
                    // this candidate be admitted tens of degrees outside
                    // `SOFT_AXIS_APERTURE_DEG`).
                    if let Some(ang) =
                        soft_axis_deviation(anchor, adir, origin, dir, soft_axis_aperture)
                    {
                        // No reach reference: soft axes exist only when NO
                        // lock is active (the gate above), so there is no
                        // fallback to clamp consistently with — and the line
                        // passes through the anchor anyway, so `Some(anchor)`
                        // would be the same origin bit for bit.
                        let pos = closest_point_on_line_to_ray(anchor, adir, None, origin, dir);
                        // On-screen cap (`SOFT_AXIS_SCREEN_REACH`): the
                        // angle above is measured from the anchor, so on a
                        // long drag it alone would capture from far across
                        // the screen.
                        let on_screen =
                            cone_test(origin, dir, pos, aperture * SOFT_AXIS_SCREEN_REACH, mode)
                                .is_some();
                        let depth = (pos - origin).dot(dir);
                        if on_screen && depth > 0.0 {
                            candidates.push((SnapKind::OnAxis, ang, depth, pos, None, Some(adir)));
                        }
                    }
                }
            }
        }

        // --- Axis × edge crossings ---
        // Where an axis line — a drawing axis through the frame origin, or
        // (with an anchor and no hard lock) a soft axis through the anchor
        // — crosses an edge, the crossing itself is what the user is after:
        // "along the red axis from where I started, out to that edge".
        // Without this candidate the two ingredients FIGHT: `OnAxis` sits a
        // rank group above `OnEdge`, so hovering the far edge of a face
        // while dragging along an axis resolved to the axis line's
        // ray-nearest point and the edge was unreachable at all (the Line
        // tool "visibly following the red axis" with "no snapping to the
        // edge whatsoever" — the only way onto it was a hard lock plus a
        // projected midpoint). Snapped as `Intersection`, the same tier a
        // guide × edge crossing gets (above Midpoint, below the exact named
        // points: a real vertex at the crossing still wins), carrying the EDGE's
        // provenance (a tool may adopt the face that edge belongs to — the
        // crossing is a point ON that edge) and the axis DIRECTION (the cue
        // draws the axis-coloured line and names the axis, exactly like a
        // plain `OnAxis` snap). Admission is the ordinary point aperture at
        // the crossing point itself: the cursor is pixel-close to the
        // crossing, which already implies the drag runs along the axis, so
        // no separate soft-axis deviation test is needed — and none of the
        // edge-on guard either, since the crossing is pure line×segment
        // geometry with no ray in it. Edge sets are the cone-pruned ones
        // the segment candidates above walked: a crossing outside the cone
        // could never be admitted, and its segment would have to pass
        // through the cone to hold a point inside it. Suppressed with the
        // axes hidden, like every other axis candidate.
        if self.axes_enabled {
            let frame_origin = self.axes_frame.origin;
            let frame_axes = [self.axes_frame.x, self.axes_frame.y, self.axes_frame.z()];
            let mut axis_lines: Vec<(Point3, Vec3)> =
                frame_axes.iter().map(|&d| (frame_origin, d)).collect();
            if let (Some(anchor), None) = (query.anchor, query.lock) {
                axis_lines.extend(frame_axes.iter().map(|&d| (anchor, d)));
            }
            let world_segments_for_axes = world_segments();
            let mut cross = |o: Point3, d: Vec3, a: Point3, b: Point3, prov: Option<Provenance>| {
                if let Some(p) = line_segment_intersection(o, d, a, b)
                    && let Some((ang, depth)) = wcone(p, SnapKind::Intersection)
                {
                    candidates.push((SnapKind::Intersection, ang, depth, p, prov, Some(d)));
                }
            };
            for &(ao, ad) in &axis_lines {
                for seg in world_segments_for_axes
                    .clone()
                    .chain(placed_segments.iter())
                {
                    cross(ao, ad, seg.a, seg.b, Some(Provenance::Object(seg.source)));
                }
                for &(sid, eid, _cid, ref seg) in &self.sketch_segments {
                    cross(ao, ad, seg.a, seg.b, Some(Provenance::SketchEdge(sid, eid)));
                }
                for seg in &self.transient_segments {
                    cross(ao, ad, seg.a, seg.b, None);
                }
            }
        }

        // --- Construction guide candidates ---
        // A guide point is a precise snap (Endpoint-tier, like the world
        // origin); a guide line snaps as OnGuide, carrying its direction for
        // the dashed cue exactly like an axis snap. Source is always None —
        // guides carry no topology to highlight. Suppressed when guides are
        // hidden (View ▸ Guides off) so a hidden guide never snaps or cues.
        if self.guides_enabled {
            for guide in &self.guides {
                match guide.geom {
                    SceneGuideGeom::Point { position } => {
                        if let Some((ang, depth)) = wcone(position, SnapKind::Endpoint) {
                            candidates.push((SnapKind::Endpoint, ang, depth, position, None, None));
                        }
                    }
                    SceneGuideGeom::Line {
                        origin: go,
                        direction: gd,
                        span,
                    } => {
                        // Reach reference only for a guide COLLINEAR with an
                        // active lock (`reach_for`): that guide must clamp
                        // exactly as the lock's own fallback does, however it
                        // happens to be parameterized (round-7 review
                        // CRITICAL — see the function's invariant); every
                        // other guide keeps its own natural origin and bound
                        // (round-8 review MAJOR).
                        let pos = closest_point_on_line_to_ray(go, gd, reach_for(gd), origin, dir);
                        // A ray or a segment snaps only where it exists.
                        if !span.contains((pos - go).dot(gd)) {
                            continue;
                        }
                        if let Some((ang, depth)) = wcone(pos, SnapKind::OnGuide) {
                            candidates.push((SnapKind::OnGuide, ang, depth, pos, None, Some(gd)));
                        }
                    }
                }
            }
        }

        // --- Guide-intersection candidates ---
        // The point where a guide line crosses a sketch segment, an object
        // edge, or another guide line is precisely why the guide was drawn —
        // snap it as SnapKind::Intersection (above Midpoint, below the exact
        // named points, so a real vertex at the crossing still wins). Like plain
        // guide snaps: no provenance, suppressed when guides are hidden.
        if self.guides_enabled {
            let guide_lines: Vec<(Point3, Vec3, kernel::GuideSpan)> = self
                .guides
                .iter()
                .filter_map(|g| match g.geom {
                    SceneGuideGeom::Line {
                        origin,
                        direction,
                        span,
                    } => Some((origin, direction, span)),
                    SceneGuideGeom::Point { .. } => None,
                })
                .collect();
            // A crossing counts only on the stretch of a guide that exists.
            let on =
                |o: Point3, d: Vec3, s: &kernel::GuideSpan, p: Point3| s.contains((p - o).dot(d));

            // Guide × segment (object edges near the ray — world and placed
            // alike — plus every live sketch segment; sketch candidates stay
            // on the linear walk).
            let emit = |p: Point3, candidates: &mut Vec<Candidate>| {
                if let Some((ang, depth)) = wcone(p, SnapKind::Intersection) {
                    candidates.push((SnapKind::Intersection, ang, depth, p, None, None));
                }
            };
            for &(go, gd, ref gs) in &guide_lines {
                for seg in world_segments().chain(placed_segments.iter()) {
                    if let Some(p) = line_segment_intersection(go, gd, seg.a, seg.b)
                        && on(go, gd, gs, p)
                    {
                        emit(p, &mut candidates);
                    }
                }
                for (_, _, _, seg) in &self.sketch_segments {
                    if let Some(p) = line_segment_intersection(go, gd, seg.a, seg.b)
                        && on(go, gd, gs, p)
                    {
                        emit(p, &mut candidates);
                    }
                }
            }
            // Guide × guide.
            for (i, &(ao, ad, ref a_span)) in guide_lines.iter().enumerate() {
                for &(bo, bd, ref b_span) in guide_lines.iter().skip(i + 1) {
                    if let Some(p) = line_line_intersection(ao, ad, bo, bd)
                        && on(ao, ad, a_span, p)
                        && on(bo, bd, b_span, p)
                    {
                        emit(p, &mut candidates);
                    }
                }
            }
        }

        // --- Constrain to the active drawing plane, if any. ---
        // Drawing on a face must not snap to occluded, off-plane geometry: the
        // pick cone is a screen-space projection that "sees through" the solid,
        // so without this a hidden bottom-edge midpoint (which outranks OnFace)
        // would win. Keep only candidates lying on the plane; this also collapses
        // OnFace to the coplanar (active) face. The lock-fallback line below is
        // intentionally NOT constrained — it's a directional inference, not a
        // candidate snap.
        //
        // `off_plane_points` (the 3d-line defect, see the field's doc) keeps
        // PRECISE POINT kinds even off the plane for the one tool contract
        // that can honour them (a re-homing line chain): a chain that
        // re-homed across sketch planes must still snap back to its own
        // origin vertex, which lives on an EARLIER sketch's plane. Occlusion
        // culling below still rejects hidden ones, so the see-through
        // protection this filter was introduced for is preserved.
        //
        // Under a non-occluding face policy (X-ray, Wireframe) there is no
        // see-through to protect against: hidden geometry is meant to be
        // reached, through the face the tool is constrained to (SketchUp's
        // Line tool snapping to an endpoint inside a solid in X-ray). So the
        // precise point kinds pass exactly as `off_plane_points` lets them,
        // and so do edges — but only for a query that opts in
        // (`see_through_points`): a tool that drops such a snap onto its
        // plane and discloses it. Every other query keeps the plain filter.
        if let Some(plane) = query.constraint_plane {
            let see_through = query.see_through_points && !self.face_policy.occludes();
            candidates.retain(|c| {
                let point_kind = matches!(
                    c.0,
                    SnapKind::Endpoint
                        | SnapKind::Midpoint
                        | SnapKind::Center
                        | SnapKind::Quadrant
                        | SnapKind::Intersection
                );
                plane.signed_distance(c.3).abs() <= tol::PLANE_DIST
                    || ((query.off_plane_points || see_through) && point_kind)
                    || (see_through
                        && matches!(c.0, SnapKind::OnEdge | SnapKind::OnIntersectionLine))
            });
        }

        // --- Rank (crate docs, *Gravity*): candidates inside the plain
        //     aperture first, then strongest rank group, then smallest
        //     WEIGHTED angular distance, then the stronger SnapKind, then
        //     nearest the ray origin (smallest depth).
        //
        //     `extended` is the "reach never steals" guard. A candidate only
        //     admitted because its weight widened the cone ranks behind
        //     everything inside the plain aperture, whatever its kind — so a
        //     circle quadrant two apertures away can never yank the cursor
        //     off the face it is sitting on. With uniform weights nothing is
        //     ever extended and the key vanishes.
        //
        //     The rank group is what stops gravity inverting the coarse
        //     priority order: an on-face hit sits at angular distance zero by
        //     construction, so a purely distance-first rank would let a face
        //     beat a vertex. Only two groups hold more than one kind — the
        //     exact points (Endpoint/Center/Quadrant) and the lines
        //     (OnEdge/OnIntersectionLine) — so those are the only places a
        //     weight can reorder anything; every other group is a single
        //     kind, where dividing by one constant leaves the old (ang,
        //     depth) order untouched.
        //
        //     Dividing by the weight puts every candidate's distance on one
        //     comparable scale — "the aperture at which this candidate would
        //     just have been admitted".
        //
        //     The SnapKind tie-break sits *between* distance and depth so two
        //     candidates at equal weighted distance still resolve to the
        //     stronger kind — a real vertex exactly on a circle's center keeps
        //     winning, as `SnapKind::Center`'s docs promise.
        //
        //     `OnAxis` measures "extended" against `soft_axis_aperture`
        //     (§2c's fixed tolerance, optionally scaled — finding E) instead
        //     of the query's own (typically far tighter, point-snap)
        //     `aperture` — the ONE exception to "reach never steals" this
        //     crate has. It is not a breach of that guard's purpose: the
        //     guard exists so a gravity-WEIGHTED reach past the query's own
        //     scale never outranks something genuinely inside it; OnAxis's
        //     wider reach isn't gravity widening a shared scale, it is its
        //     own dedicated directional tolerance (tool-parity playtest2
        //     §2c) — comparing it against the *point* aperture would make
        //     "extended" fire on nearly every soft-axis hit and demote it
        //     behind anything else in view, defeating the ranking §2c asks
        //     for (soft axis beats plain OnEdge/OnFace). A query that scales
        //     `soft_axis_aperture` up (the hysteresis release query) still
        //     marks anything beyond the UNSCALED tolerance as extended —
        //     using the scaled value here, not the bare constant, so reach
        //     the scale itself bought in still never steals from a candidate
        //     genuinely inside the normal aperture either. ---
        // Two candidates whose weighted distances differ by less than
        // RANK_TIE_FRACTION of the aperture are the SAME distance for
        // ranking: the difference is float noise (a placed instance's
        // corner composed through its pose vs. a world vertex read
        // exactly), and letting it decide would make the tie-break below
        // — nearest depth first — meaningless exactly where it matters
        // most: a Top view of a framed wall stacks the slab corner, the
        // sole plate, the stud top, and both top plates on ONE pixel, and
        // the user pointing at the wall top means the nearest one.
        //
        // The tie is a QUANTIZATION (`rank_distance_bucket`), never an
        // epsilon comparison: "equal when within `tie`" is not transitive,
        // so it is not a total order, and the standard sort detects that
        // on a dense candidate set (a framed floor plan under a Top view)
        // and panics — which poisoned the whole resolver. Integer buckets
        // and `f64::total_cmp` make the ordering total by construction.
        // …and measured against the ACQUIRE aperture, not a widened one: the
        // app's hysteresis-release query re-runs this resolve at twice the
        // pixel radius and declares that ratio in `soft_axis_aperture_scale`
        // (that field's doc). Scaling the tie band with it would let a
        // reach-only widening reorder two candidates the normal query had
        // already admitted — the held target losing its own hold to a
        // nearer-depth neighbour a pixel further from the cursor — so the
        // band is divided back down to one pixel of the unscaled radius.
        let tie = aperture / widening * RANK_TIE_FRACTION;
        let rank_key = |c: &Candidate| {
            let ref_aperture = if c.0 == SnapKind::OnAxis {
                soft_axis_aperture.max(aperture)
            } else {
                aperture * weights.base_reach(c.0, widening)
            };
            let extended = u8::from(c.1 > ref_aperture);
            (
                extended,
                c.0.rank_group(),
                rank_distance_bucket(c.1 / weights.weight(c.0), tie),
            )
        };
        candidates.sort_by(|a, b| {
            let (ea, ga, da) = rank_key(a);
            let (eb, gb, db) = rank_key(b);
            ea.cmp(&eb)
                .then(ga.cmp(&gb))
                .then(da.cmp(&db))
                .then(a.0.cmp(&b.0))
                .then(a.2.total_cmp(&b.2))
        });

        // --- Resolve the lock (direction + anchor) BEFORE picking a winner:
        //     under a lock the winner's projection outcome is part of what
        //     makes it a winner at all (see the cull below). `SnapLock::Axis`
        //     resolves through the drawing-axes frame (tool-parity design
        //     §4), not the literal world X/Y/Z, so an axis-locked drawing
        //     gesture follows a moved frame exactly like the OnAxis snap
        //     candidates above (world identity is this frame's default, so
        //     an unmoved document locks exactly as before). ---
        // Beyond the direction + anchor, a lock context carries the
        // gesture-side facts the winner cull and the fallback below need:
        //
        //   `fall` — the directional fallback point itself: the locked
        //   line's point nearest the ray, exactly what the no-winner
        //   branch below returns. Computed once here because the winner
        //   cull ALSO needs it: a ray-sliding candidate that resolves to
        //   this same point is contributing nothing but the ray (see the
        //   cull's noise bullet).
        //
        //   `t_fallback` — `fall`'s signed station (along `lock_dir`, from
        //   the anchor): where the cursor itself is pointing along the
        //   lock, and therefore which SIDE of the anchor the gesture
        //   indicates — but ONLY while the cursor is aiming at the lock
        //   line at all (`on_lock_line`) and not at the anchor itself
        //   (`on_anchor`).
        //
        //   `on_lock_line` — the pick ray passes within the query
        //   `aperture` of `fall`, i.e. at pixel resolution the cursor is
        //   ON the lock's guide line. This is what makes `t_fallback` a
        //   statement about the USER's intent rather than about parallax.
        //   `fall` is the lock line's point nearest the ray whether the
        //   cursor is a pixel or a screenful away from that line, and off
        //   the line its station is set by where the eye happens to be —
        //   swing the camera around a Z-locked gesture and `t_fallback`
        //   flips sign over ground geometry that never moved. Reading it
        //   as "the side the user indicated" there is what made most
        //   projected inference points unsnappable, and made WHICH ones
        //   snapped change with every orbit: a ground vertex a metre
        //   below the anchor is squarely under the cursor, but the ray's
        //   closest approach to the lock line passes ABOVE the anchor
        //   whenever the vertex is on the far side of it, so the
        //   wrong-side gate below threw the vertex away. On the line the
        //   reading is sound (the cursor is literally pointing along the
        //   lock), which is the case that gate was written for.
        //
        //   `on_anchor` — the pick ray passes within
        //   `LOCK_ON_ANCHOR_APERTURE_FRACTION` of the aperture of the
        //   anchor: at pixel resolution the cursor is ON the anchor, so the
        //   gesture indicates ZERO displacement — the directional
        //   fallback's own station is then sub-pixel reconstruction noise
        //   with an arbitrary SIGN, not a direction the user chose (see
        //   that constant's doc for the Move+Alt wrong-way repro this
        //   closes). A strictly narrower ring on the same line as
        //   `on_lock_line`, and like it deliberately a fact about the RAY
        //   only, consumed only by the fallback clamp and the wrong-side
        //   gate below — the winner cull judges each CANDIDATE by its own
        //   position, so real geometry keeps winning at its true station
        //   wherever the cursor is.
        let lock_ctx = match (locked_dir, query.anchor) {
            (Some(d), Some(anchor)) => {
                let on_anchor = cone_test(
                    origin,
                    dir,
                    anchor,
                    aperture * LOCK_ON_ANCHOR_APERTURE_FRACTION,
                    mode,
                )
                .is_some();
                // The lock line passes THROUGH the anchor, so the
                // anchor as reach reference is bit-for-bit the old
                // behaviour — and the shared reference is what makes
                // a collinear guide/axis candidate clamp to exactly
                // this same point (the cull's noise-equality test).
                let fall = closest_point_on_line_to_ray(anchor, d, Some(anchor), origin, dir);
                let t_fallback = (fall - anchor).dot(d);
                let on_lock_line = cone_test(origin, dir, fall, aperture, mode).is_some();
                Some(LockCtx {
                    lock_dir: d,
                    anchor,
                    on_anchor,
                    on_lock_line,
                    t_fallback,
                    fall,
                })
            }
            _ => None,
        };

        // --- Occlusion cull: walk the ranked list and take the first candidate
        //     that isn't hidden behind an opaque face. A solid must not let a
        //     draw/select snap "see through" it to a higher-priority back edge
        //     or vertex — only what's visible from the eye should snap. Lazy by
        //     design: usually the top candidate is visible, so this costs one
        //     visibility test. The lock-fallback line below is a *directional*
        //     inference, not a candidate, so it is intentionally never culled.
        //
        //     Under an active lock, a candidate must ALSO survive its own lock
        //     projection to win: the only thing a candidate contributes to a
        //     locked gesture is where it lands on the locked line, and one
        //     whose projection collapses onto the anchor contributes nothing —
        //     it names the point the segment already starts from, which no
        //     tool can commit. Without this, a legitimate candidate lying in
        //     the plane through the anchor perpendicular to the lock (the
        //     anchor's own vertex, or — the 3d-line staircase's third,
        //     axis-locked segment, where the lock is normal to the frozen
        //     sketch plane — ANY candidate the constraint plane kept) would
        //     win on angular rank, project to the anchor, and turn an
        //     ordinary locked click into a degenerate-segment refusal. Such a
        //     candidate is skipped, not fatal: the next candidate with a real
        //     projection wins, else the directional fallback line below.
        //
        //     The disqualification line is `tol::POINT_MERGE` because that IS
        //     the definition of an uncommittable segment: the kernel's own
        //     `SketchError::DegenerateSegment` refuses exactly the segments
        //     whose endpoints are within `POINT_MERGE`, and the app-side
        //     commit gates (`LineTool`'s `DEGENERATE_SEGMENT_EPS`) mirror the
        //     same constant. All three layers reading one threshold is what
        //     closes the gap completely: any looser line here (or any
        //     stricter gate there) reopens a band of projections that one
        //     side returns as a real winner and the other still refuses.
        //
        //     Surviving a collapsed projection is NOT sufficient, though: the
        //     fall-through must never hand the gesture a station the user did
        //     not indicate. Two further disqualifications close the two ways
        //     it could (both found by the Move+Alt donut copy landing BELOW
        //     the ground it was copied up from):
        //
        //     * A ray-sliding candidate (`OnEdge`/`OnFace`/`OnAxis`/
        //       `OnGuide`, whose positions are computed FROM the ray) that
        //       resolves to the SAME point the no-winner fallback would
        //       return (`fall`, the locked line's ray-nearest point) is
        //       contributing nothing but the ray itself: its "station" is
        //       the ray's own sub-pixel reconstruction miss, verbatim — a
        //       noise-scale, noise-signed number that a typed Move/Line
        //       commit would amplify into a full move the wrong way (the
        //       Move+Alt donut repro: the world axis COLLINEAR with the
        //       lock line slides to exactly this point). The test is
        //       equality with `fall` — deliberately not a proximity band
        //       around the anchor (a ray-based zone discards real geometry
        //       the cursor is squarely on) and not a depth-scaled station
        //       envelope (metres wide at depth, swallowing genuine far
        //       stations): a real edge crossing the cone metres along the
        //       lock is never equal to the fallback point, at any depth.
        //       The comparison is sound because
        //       `closest_point_on_line_to_ray` is parameterization-
        //       invariant under this query's reach reference (the anchor —
        //       see that function's invariant): a guide or axis collinear
        //       with the lock line clamps to exactly the fallback's own
        //       point however it happens to be parameterized, so equality
        //       genuinely detects "same construction" (round-7 review
        //       CRITICAL: before that invariant, a collinear guide with an
        //       origin near the camera clamped metres from the fallback
        //       and won with the corrupted station). `tol::POINT_MERGE`
        //       absorbs the residual fp rounding between the two
        //       computations (~1e-13 at scene scale for a flipped or
        //       re-derived direction); a genuinely coincident candidate AT
        //       the fallback point is culled harmlessly (the fallback
        //       returns the identical position, only kind/provenance
        //       differ).
        //       Point-kind candidates are never tested: their positions
        //       are geometry, not ray constructions (a real vertex
        //       nanometres along the lock must still resolve at its true
        //       station — see the committable-band spec).
        //
        //     * When the cursor is aiming AT the lock line (`on_lock_line`)
        //       but not at the anchor (`on_anchor`), the gesture indicates a
        //       definite side of the anchor — the sign of `t_fallback` — and
        //       a candidate whose own station lies on the OPPOSITE side is
        //       not a better answer than the directional fallback, it is a
        //       wrong answer (e.g. a stray vertex the extended pick ray
        //       reaches BEHIND the station the user is pointing at, which
        //       outranks everything by sitting dead-on the ray). Skipped the
        //       same way.
        //
        //       `on_lock_line` is what confines this rule to the gesture it
        //       was written for. OFF the lock line the cursor is pointing at
        //       GEOMETRY, not at a station, and `t_fallback` is then a fact
        //       about parallax — where the eye happens to put the ray's
        //       closest approach to the line — not about intent. Reading it
        //       as intent there is what made most projected inference points
        //       unsnappable, with the surviving set changing on every orbit
        //       (see `lock_ctx`): the whole POINT of a projected snap is to
        //       name a station by hovering geometry that lies well off the
        //       lock line, and that geometry is as legitimately below the
        //       anchor as above it — dropping a floating rectangle back onto
        //       a ground vertex is the same gesture as lifting it, in the
        //       other direction. The side test is additionally gated on
        //       `t_fallback` being meaningfully nonzero (`> POINT_MERGE`),
        //       since with the ray aimed exactly down the anchor's
        //       perpendicular the "side" is itself noise. ---
        let occludes = self.face_policy.occludes();
        let winner = candidates.iter().copied().find(|c| {
            if occludes && self.is_occluded(origin, c.3, index, source) {
                return false;
            }
            match lock_ctx {
                Some(lk) => {
                    let proj = project_onto_line(lk.anchor, lk.lock_dir, c.3);
                    if proj.approx_eq(lk.anchor, tol::POINT_MERGE) {
                        return false; // collapses onto the anchor: uncommittable
                    }
                    if matches!(
                        c.0,
                        SnapKind::OnEdge
                            | SnapKind::OnIntersectionLine
                            | SnapKind::OnFace
                            | SnapKind::OnAxis
                            | SnapKind::OnGuide
                    ) && c.3.approx_eq(lk.fall, tol::POINT_MERGE)
                    {
                        return false; // the candidate IS the fallback point: ray, not geometry
                    }
                    if lk.on_lock_line
                        && !lk.on_anchor
                        && lk.t_fallback.abs() > tol::POINT_MERGE
                        && (proj - lk.anchor).dot(lk.lock_dir) * lk.t_fallback < 0.0
                    {
                        return false; // wrong side of the anchor
                    }
                    true
                }
                None => true,
            }
        });

        // TRACE only — `resolve` runs on every pointer move, so this is a
        // firehose filtered out by default; raise the capture level to debug a
        // bad snap (the inference winner + candidate count — docs/dev/DIAGNOSTICS.md).
        tracing::trace!(
            target: "inference::resolve",
            candidates = candidates.len(),
            winner = ?winner.as_ref().map(|c| c.0),
        );

        // --- Handle locking ---
        match lock_ctx {
            Some(lk) => {
                let (lock_dir, anchor, on_anchor, fall) =
                    (lk.lock_dir, lk.anchor, lk.on_anchor, lk.fall);
                if let Some((kind, _ang, _depth, pos, prov, _cdir)) = winner.as_ref() {
                    // A candidate snapped: project its position onto the locked line.
                    let projected = project_onto_line(anchor, lock_dir, *pos);
                    let (
                        source,
                        sketch_source,
                        sketch_region_source,
                        sketch_curve_source,
                        crossed_edge,
                        pierced_face,
                        meeting_line,
                    ) = Provenance::split(*prov);
                    Some(Snap {
                        position: projected,
                        kind: *kind,
                        source,
                        sketch_source,
                        sketch_region_source,
                        sketch_curve_source,
                        direction: Some(lock_dir),
                        projected_from: Some(*pos),
                        crossed_edge,
                        pierced_face,
                        meeting_line,
                    })
                } else {
                    // Nothing snapped: the directional fallback (`fall`, the
                    // locked line's ray-nearest point, from `lock_ctx`) —
                    // unless the cursor is ON the anchor itself
                    // (`on_anchor`, see `lock_ctx`), in which case that
                    // point's station is only the ray's sub-pixel
                    // reconstruction noise and the honest answer is the
                    // anchor: zero displacement, exactly what the gesture
                    // indicates. Tools already treat a zero-displacement
                    // locked resolve correctly (Move's typed commit
                    // defaults to the POSITIVE lock direction; Line refuses
                    // the degenerate click) — it is the noise-SIGNED
                    // near-zero station they cannot survive.
                    let locked_pos = if on_anchor { anchor } else { fall };
                    Some(Snap {
                        position: locked_pos,
                        kind: SnapKind::OnAxis,
                        source: None,
                        sketch_source: None,
                        sketch_region_source: None,
                        sketch_curve_source: None,
                        direction: Some(lock_dir),
                        projected_from: None,
                        crossed_edge: None,
                        pierced_face: None,
                        meeting_line: None,
                    })
                }
            }
            _ => {
                // No lock (or lock with no anchor): return the top-ranked
                // candidate that is actually visible (see the occlusion cull above).
                winner.map(|(kind, _ang, _depth, pos, prov, snap_dir)| {
                    let (
                        source,
                        sketch_source,
                        sketch_region_source,
                        sketch_curve_source,
                        crossed_edge,
                        pierced_face,
                        meeting_line,
                    ) = Provenance::split(prov);
                    Snap {
                        position: pos,
                        kind,
                        source,
                        sketch_source,
                        sketch_region_source,
                        sketch_curve_source,
                        direction: snap_dir,
                        projected_from: None,
                        crossed_edge,
                        pierced_face,
                        meeting_line,
                    }
                })
            }
        }
    }

    /// Picks the nearest face the ray passes *through* — face selection for
    /// tools like push/pull, distinct from [`resolve`](Self::resolve).
    ///
    /// Unlike `resolve`, this ignores the snap-priority model and the pick
    /// cone entirely: a face is a candidate iff the ray actually crosses its
    /// boundary polygon (in front of the origin), and the nearest such face
    /// wins. The drawing snap prefers endpoints/edges, so it is the wrong tool
    /// for "what surface is under the cursor"; this is the right one. Returns
    /// the face's [`SnapSource`], or `None` if the ray hits no face.
    /// Returns the picked face's [`SnapSource`] and the ray-distance (depth)
    /// to the hit, so a caller can reject a hit beyond its render far plane —
    /// the raw world-ray otherwise "sees" solids the user cannot (a drag must
    /// never move an object off-screen).
    ///
    /// Always `None` under [`FacePolicy::Absent`]: faces are not targets.
    /// With a [`PickScope`] set (under [`FacePolicy::Opaque`] or
    /// [`FacePolicy::SeeThrough`]), the nearest face whose owner is in
    /// scope wins; with none in scope along the ray, the nearest face of
    /// any owner (crate docs, *Occlusion*).
    pub fn pick_face(&self, ray: &PickRay) -> Option<(SnapSource, f64)> {
        self.pick_face_from(ray, &|_| None)
    }

    /// [`Self::pick_face`] for a scene with deferred definition members
    /// (see [`Self::resolve_from`]).
    pub fn pick_face_from(
        &self,
        ray: &PickRay,
        source: MemberSource<'_>,
    ) -> Option<(SnapSource, f64)> {
        let index = self.spatial_index();
        self.pick_face_impl(ray, Some(&index), source, true)
    }

    /// [`Self::pick_face_from`] that ignores the [`FacePolicy`] and the
    /// [`PickScope`]: the nearest face the ray passes through, even in
    /// Wireframe. For camera questions (where to orbit around), not editing
    /// ones — a face that is no input target is still where the geometry
    /// is.
    pub fn pick_face_geometry_from(
        &self,
        ray: &PickRay,
        source: MemberSource<'_>,
    ) -> Option<(SnapSource, f64)> {
        let index = self.spatial_index();
        self.pick_face_impl(ray, Some(&index), source, false)
    }

    /// Reference implementation of [`pick_face`](Self::pick_face) with the
    /// spatial index bypassed, mirroring
    /// [`resolve_linear`](Self::resolve_linear) (DEVELOPMENT.md rule 3). Not
    /// part of the supported API.
    #[doc(hidden)]
    pub fn pick_face_linear(&self, ray: &PickRay) -> Option<(SnapSource, f64)> {
        self.pick_face_impl(ray, None, &|_| None, true)
    }

    /// Every face the ray passes through, nearest first, ties in candidate
    /// order — the full hit list [`Self::pick_face`] chooses from, on the
    /// linear path, ignoring the policy and the scope. The reference the
    /// property tests check the scoped pick against (DEVELOPMENT.md rule
    /// 3). Not part of the supported API.
    #[doc(hidden)]
    pub fn pick_face_hits_linear(&self, ray: &PickRay) -> Vec<(SnapSource, f64)> {
        let Ok(dir) = ray.direction.normalized() else {
            return Vec::new();
        };
        let origin = ray.origin;
        let none: MemberSource<'_> = &|_| None;
        let geom = |member: ObjectId| self.geometry(member, none);
        let placed: Vec<SceneFace> = self
            .all_placed(none, |g| g.faces.len())
            .into_iter()
            .filter_map(|(pi, li)| {
                let pl = &self.placements[pi];
                geom(pl.member)?.face_at(li, pl)
            })
            .collect();
        let mut hits: Vec<(SnapSource, f64)> = self
            .faces
            .iter()
            .chain(placed.iter())
            .filter_map(|face| {
                face_cone_hit(origin, dir, &face.plane, &face.boundary, &face.holes, 0.0)
                    .map(|(_pos, _ang, depth)| (face.source, depth))
            })
            .collect();
        // Stable: equal depths keep candidate order, matching the picks'
        // strict `<` (lowest candidate index wins a tie).
        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        hits
    }

    /// Shared body of `pick_face`/`pick_face_linear`. The indexed superset
    /// is scanned in ascending order with the same strict `<` depth
    /// comparison, so equal-depth ties resolve to the lowest candidate
    /// index on both paths.
    ///
    /// Two running bests: the nearest face of any owner, and the nearest
    /// whose owner is in the [`PickScope`]. The in-scope one is returned
    /// for a policy-respecting pick with a scope set (faces are targets, so
    /// the policy is `Opaque` or `SeeThrough`: `Absent` returned above);
    /// otherwise (and whenever nothing in scope is hit) the overall
    /// nearest, exactly as without a scope.
    fn pick_face_impl(
        &self,
        ray: &PickRay,
        index: Option<&SceneIndex>,
        source: MemberSource<'_>,
        respect_policy: bool,
    ) -> Option<(SnapSource, f64)> {
        if respect_policy && !self.face_policy.faces_are_targets() {
            return None;
        }
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        let face_ids: Vec<usize> = match index {
            Some(ix) => ix.faces_crossing_ray(origin, dir),
            None => (0..self.faces.len()).collect(),
        };
        let geom = |member: ObjectId| self.geometry(member, source);
        let members = Members {
            map: &self.def_members,
            geometry: &geom,
        };
        let placed_face_ids: Vec<(usize, usize)> = match index {
            Some(ix) => ix.placed_faces_crossing_ray(&self.placements, &members, origin, dir),
            None => self.all_placed(source, |g| g.faces.len()),
        };
        let placed_faces: Vec<SceneFace> = placed_face_ids
            .iter()
            .filter_map(|&(pi, li)| {
                let pl = &self.placements[pi];
                geom(pl.member)?.face_at(li, pl)
            })
            .collect();
        // Faces are targets here (`Absent` returned above), so the scope
        // applies under both `Opaque` and `SeeThrough`: the editing view
        // draws what is outside the scope translucent, so what is inside it
        // is visible either way.
        let scope = if respect_policy {
            self.pick_scope.as_ref()
        } else {
            None
        };
        let mut best: Option<(f64, SnapSource)> = None;
        let mut best_in_scope: Option<(f64, SnapSource)> = None;
        let world_faces = face_ids.iter().map(|&fi| &self.faces[fi]);
        for face in world_faces.chain(placed_faces.iter()) {
            // `face_cone_hit` ignores its aperture arg for faces (a face hit
            // is pure ray-polygon containment), so any value works here.
            let Some((_pos, _ang, depth)) =
                face_cone_hit(origin, dir, &face.plane, &face.boundary, &face.holes, 0.0)
            else {
                continue;
            };
            if best.as_ref().is_none_or(|(d, _)| depth < *d) {
                best = Some((depth, face.source));
            }
            if let Some(scope) = scope
                && face.source.in_scope(scope)
                && best_in_scope.as_ref().is_none_or(|(d, _)| depth < *d)
            {
                best_in_scope = Some((depth, face.source));
            }
        }
        best_in_scope
            .or(best)
            .map(|(depth, source)| (source, depth))
    }

    /// Every `(placement, member-local)` candidate pair of one class, in
    /// enumeration order — the linear reference's counterpart to the
    /// two-level index walks. Placements of an unregistered member are
    /// skipped, matching the indexed paths (their world boxes are empty).
    fn all_placed(
        &self,
        source: MemberSource<'_>,
        count: impl Fn(&DefGeometry) -> usize,
    ) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (pi, pl) in self.placements.iter().enumerate() {
            if let Some(g) = self.geometry(pl.member, source) {
                out.extend((0..count(g)).map(|li| (pi, li)));
            }
        }
        out
    }

    /// Visibility test for snap occlusion: is `pos` hidden behind an opaque
    /// face, as seen from `origin`?
    ///
    /// The pick cone is a screen-space projection that "sees through" solids,
    /// so without this a snap candidate on the *far* side of a solid (a hidden
    /// back edge/vertex, or the interior of a back face) — which can outrank
    /// the visible front face on [`SnapKind`] priority alone — would win. We
    /// cast the ray `origin -> pos` and treat `pos` as occluded iff some face
    /// crosses that ray strictly nearer than `pos` itself.
    ///
    /// A [`tol::OCCLUSION_REL`] skin keeps the face `pos` lies on (and faces
    /// sharing its edge) from self-occluding it: those are hit at depth ≈ the
    /// distance to `pos`, not nearer. A ray that passes through a face's *hole*
    /// is not occluded by that face (`face_cone_hit` already rejects holes), so
    /// snaps seen through an imprinted opening stay visible.
    fn is_occluded(
        &self,
        origin: Point3,
        pos: Point3,
        index: Option<&SceneIndex>,
        source: MemberSource<'_>,
    ) -> bool {
        let to_pos = pos - origin;
        let dist = to_pos.length();
        let dir = match to_pos.normalized() {
            Ok(d) => d,
            Err(_) => return false, // candidate at the eye — nothing can occlude it
        };
        let near_threshold = dist * (1.0 - tol::OCCLUSION_REL);
        // The exact hole-aware test, shared verbatim by the indexed and
        // linear paths and by every face kind (world, placed, sketch region).
        // The counter feeds `occlusion_face_tests` — the introspection the
        // perf-sanity spec uses to prove the index prunes.
        let face_occludes = |plane: &Plane, boundary: &[Point3], holes: &[Vec<Point3>]| {
            self.occlusion_tests.set(self.occlusion_tests.get() + 1);
            // `face_cone_hit` ignores its aperture arg for faces (pure
            // ray-polygon containment); 0.0 is fine.
            face_cone_hit(origin, dir, plane, boundary, holes, 0.0)
                .is_some_and(|(_pos, _ang, depth)| depth < near_threshold)
        };
        let occludes = |face: &SceneFace| face_occludes(&face.plane, &face.boundary, &face.holes);
        // A placed face materializes on demand and skips singular poses,
        // exactly like the candidate paths (`DefGeometry::face_at`).
        let geom = |member: ObjectId| self.geometry(member, source);
        let members = Members {
            map: &self.def_members,
            geometry: &geom,
        };
        let occludes_placed = |pi: usize, li: usize| {
            let pl = &self.placements[pi];
            geom(pl.member)
                .and_then(|g| g.face_at(li, pl))
                .is_some_and(|face| face_occludes(&face.plane, &face.boundary, &face.holes))
        };
        // Sketch region faces occlude just like solid faces (a drawn region
        // hides what's behind it). Few per scene, so always a linear walk —
        // they're not in the spatial index (mirroring the candidate paths).
        let sketch_occludes = || {
            self.sketch_faces
                .iter()
                .any(|(_sid, f, _)| face_occludes(&f.plane, &f.boundary, &f.holes))
        };
        // Sketch faces are linear-walked in both branches (they carry no index
        // membership); the boolean is order-independent, so testing them last
        // preserves the index's early-out on the common no-occlusion path.
        match index {
            // Early-out walks: only subtrees whose boxes the ray enters
            // nearer than the threshold can hold an occluder, and each walk
            // stops at the first face that actually occludes. World faces
            // first, placed faces second — order can't change the boolean.
            Some(ix) => {
                ix.any_face_hit_before(origin, dir, near_threshold, |fi| occludes(&self.faces[fi]))
                    || ix.any_placed_face_hit_before(
                        &self.placements,
                        &members,
                        origin,
                        dir,
                        near_threshold,
                        occludes_placed,
                    )
                    || sketch_occludes()
            }
            None => {
                self.faces.iter().any(occludes)
                    || self
                        .all_placed(source, |g| g.faces.len())
                        .into_iter()
                        .any(|(pi, li)| occludes_placed(pi, li))
                    || sketch_occludes()
            }
        }
    }

    /// Picks the live sketch whose nearest edge is closest to the ray, for
    /// whole-sketch selection of a free-standing (not-yet-extruded) sketch.
    ///
    /// Unlike `pick_face`, a sketch edge has no thickness, so this uses the
    /// same pick-cone model as [`InferenceScene::resolve`]: a sketch is a
    /// candidate iff some point on one of its edges falls within `aperture`
    /// radians of the ray axis, and among candidates the one with the
    /// smallest angular distance wins (depth breaks ties, nearest to the ray
    /// origin first) — mirroring `OnEdge` ranking in `resolve`. Registered
    /// transient segments (no owning sketch) are not candidates here.
    /// Returns `None` if the ray hits no live sketch edge within `aperture`.
    pub fn pick_sketch(&self, ray: &PickRay, aperture: f64) -> Option<SketchId> {
        self.pick_sketch_edge(ray, aperture).map(|(id, _)| id)
    }

    /// Like [`InferenceScene::pick_sketch`], but says WHICH edge was hit:
    /// the `(SketchId, SketchEdgeId)` of the live sketch segment nearest the
    /// ray within `aperture` (same ranking — smallest angular distance,
    /// depth breaks ties). The Select tool's per-edge pick. `None` if the
    /// ray hits no live sketch edge within `aperture`.
    pub fn pick_sketch_edge(
        &self,
        ray: &PickRay,
        aperture: f64,
    ) -> Option<(SketchId, SketchEdgeId)> {
        self.pick_sketch_edge_preferring(ray, aperture, &|_| false)
    }

    /// [`InferenceScene::pick_sketch_edge`] that favors the sketches
    /// `prefer` accepts: the best-ranked edge of a preferred sketch when one
    /// is within `aperture`, else the best-ranked edge of any sketch — the
    /// sketch analog of the scoped [`InferenceScene::pick_face`] (crate
    /// docs, *Occlusion*). The caller decides when to prefer (an editing
    /// scope, wherever faces are targets); with nothing preferred within
    /// reach the result is exactly `pick_sketch_edge`'s.
    pub fn pick_sketch_edge_preferring(
        &self,
        ray: &PickRay,
        aperture: f64,
        prefer: &dyn Fn(SketchId) -> bool,
    ) -> Option<(SketchId, SketchEdgeId)> {
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        // (angular_dist, depth, sketch, edge)
        let mut best: Option<(f64, f64, SketchId, SketchEdgeId)> = None;
        let mut best_preferred: Option<(f64, f64, SketchId, SketchEdgeId)> = None;
        for &(id, eid, _cid, ref seg) in &self.sketch_segments {
            let Some((_pos, ang, depth)) =
                segment_cone_hit(origin, dir, seg.a, seg.b, aperture, ApertureMode::Cone)
            else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|&(a, d, _, _)| (ang, depth) < (a, d))
            {
                best = Some((ang, depth, id, eid));
            }
            if best_preferred
                .as_ref()
                .is_none_or(|&(a, d, _, _)| (ang, depth) < (a, d))
                && prefer(id)
            {
                best_preferred = Some((ang, depth, id, eid));
            }
        }
        best_preferred.or(best).map(|(_, _, id, eid)| (id, eid))
    }

    /// Picks the committed sketch *vertex* nearest the ray (Phase D per-vertex
    /// edit). Uses the same pick-cone model as [`InferenceScene::pick_sketch`]
    /// but tests vertex points (via [`cone_test`]) rather than edges, returning
    /// the owning sketch, the exact `SketchVertexId`, and its world position.
    /// Smallest angular distance wins; depth breaks ties (nearest first).
    /// Returns `None` if no registered sketch vertex falls within `aperture`.
    pub fn pick_sketch_vertex(
        &self,
        ray: &PickRay,
        aperture: f64,
    ) -> Option<(SketchId, SketchVertexId, Point3)> {
        self.pick_sketch_vertex_preferring(ray, aperture, &|_| false)
    }

    /// [`InferenceScene::pick_sketch_vertex`] that favors the sketches
    /// `prefer` accepts, with the same rule as
    /// [`InferenceScene::pick_sketch_edge_preferring`]: the best-ranked
    /// vertex of a preferred sketch when one is within `aperture`, else
    /// exactly `pick_sketch_vertex`'s result.
    pub fn pick_sketch_vertex_preferring(
        &self,
        ray: &PickRay,
        aperture: f64,
        prefer: &dyn Fn(SketchId) -> bool,
    ) -> Option<(SketchId, SketchVertexId, Point3)> {
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        // (angular_dist, depth, id, vertex, position)
        let mut best: Option<(f64, f64, SketchId, SketchVertexId, Point3)> = None;
        let mut best_preferred: Option<(f64, f64, SketchId, SketchVertexId, Point3)> = None;
        for &(id, vid, pos) in &self.sketch_vertices {
            let Some((ang, depth)) = cone_test(origin, dir, pos, aperture, ApertureMode::Cone)
            else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|&(a, d, _, _, _)| (ang, depth) < (a, d))
            {
                best = Some((ang, depth, id, vid, pos));
            }
            if best_preferred
                .as_ref()
                .is_none_or(|&(a, d, _, _, _)| (ang, depth) < (a, d))
                && prefer(id)
            {
                best_preferred = Some((ang, depth, id, vid, pos));
            }
        }
        best_preferred
            .or(best)
            .map(|(_, _, id, vid, pos)| (id, vid, pos))
    }
}

/// Internal candidate tuple used inside `resolve`:
/// `(kind, angular_dist, depth, position, source, direction)`.
///
/// The angular distance is the raw angle off the ray axis; gravity weighting
/// (crate docs, *Gravity*) is applied in the ranking pass, not here, so the
/// same number serves both the "is this inside the plain aperture?" reach test
/// and the weighted comparison.
type Candidate = (SnapKind, f64, f64, Point3, Option<Provenance>, Option<Vec3>);

// ---------------------------------------------------------------------------
// Geometry helpers (crate-private)
// ---------------------------------------------------------------------------

/// Returns `(distance, depth)` if `point` is inside the pick tolerance (in
/// front of the ray and within `aperture` of the ray axis), otherwise `None`.
/// The returned `distance` — and what `aperture` means — depends on `mode`
/// ([`ApertureMode`]): under `Cone` both are an angle in radians (the
/// original pick-cone test); under `Cylinder` both are a world-space
/// perpendicular distance in meters, constant with depth rather than
/// growing with it. Either way the same value is compared directly against
/// `aperture` for admission AND returned raw for the caller's weighted
/// ranking (see `InferenceScene::resolve_impl`'s `wcone`) — the two modes
/// never mix within one query, so the unit only needs to be self-consistent.
///
/// `dir` must already be normalized.
/// The ranking distance quantized to `tie`-wide buckets: candidates in the
/// same bucket rank as equally distant and fall through to the nearest-depth
/// tie-break (see [`RANK_TIE_FRACTION`]). A bucket is an integer, so the
/// order it induces is total — an "equal within epsilon" comparison is not,
/// and `sort_by` panics on one. A non-finite or non-positive `tie` (a zero
/// aperture) degrades to plain distance order rather than dividing by zero.
/// Saturates at the `i64` range for absurd inputs instead of wrapping.
pub fn rank_distance_bucket(distance: f64, tie: f64) -> i64 {
    let usable_tie = tie.is_finite() && tie > 0.0;
    if !usable_tie {
        return distance.to_bits() as i64 & i64::MAX;
    }
    let q = (distance / tie).round();
    if q.is_nan() {
        i64::MAX
    } else {
        q.clamp(i64::MIN as f64, i64::MAX as f64) as i64
    }
}

fn cone_test(
    origin: Point3,
    dir: Vec3,
    point: Point3,
    aperture: f64,
    mode: ApertureMode,
) -> Option<(f64, f64)> {
    let to_point = point - origin;
    let depth = to_point.dot(dir); // signed distance along ray
    if depth <= 0.0 {
        return None; // behind the ray origin
    }
    match mode {
        ApertureMode::Cone => {
            let dist_sq = to_point.length_squared();
            if dist_sq < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
                // Point is essentially at the ray origin; treat as angle 0.
                return Some((0.0, depth));
            }
            // cos(angle) = depth / dist; angle = acos(depth / dist).
            let cos_angle = (depth / dist_sq.sqrt()).min(1.0);
            let angle = cos_angle.acos();
            if angle <= aperture {
                Some((angle, depth))
            } else {
                None
            }
        }
        ApertureMode::Cylinder => {
            // Perpendicular (rejection) component of `to_point` off the ray
            // axis — constant-radius tolerance, independent of `depth`.
            let perp = to_point - dir * depth;
            let perp_dist = perp.length();
            if perp_dist <= aperture {
                Some((perp_dist, depth))
            } else {
                None
            }
        }
    }
}

/// The two angles (in the rim's `u`/`v` frame) where a segment from
/// `anchor` is tangent to the circle `(center, axis, radius)`, or `None`
/// when the anchor's in-plane projection lies inside or on the circle (no
/// tangent exists). Shared by the object-rim and sketch-curve tangent
/// walks so both resolve the identical geometry.
fn tangent_angles(
    anchor: Point3,
    center: Point3,
    axis: Vec3,
    radius: f64,
    u: Vec3,
    v: Vec3,
) -> Option<[f64; 2]> {
    let d = anchor - center;
    let in_plane = d - axis * d.dot(axis);
    let dist = in_plane.length();
    if dist <= radius + tol::POINT_MERGE {
        return None;
    }
    let phi = in_plane.dot(v).atan2(in_plane.dot(u));
    let alpha = (radius / dist).acos();
    Some([phi + alpha, phi - alpha])
}

/// Returns the midpoint of a segment.
fn midpoint(a: Point3, b: Point3) -> Point3 {
    Point3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5)
}

/// Resolves `segment_cone_hit`'s candidate search: tests a handful of
/// candidate points along the segment through `cone_test` itself and keeps
/// its best (per `cone_test`'s own ranking tuple) admissible result, rather
/// than picking a single point analytically — see the "why not
/// analytically" note below. Always invoked (not just when the ray and the
/// segment's line are (near-)parallel, or when the true closest-line-point
/// lands behind the ray origin): neither of `segment_cone_hit`'s two
/// analytic closest-point candidates, `perp_min_s`/`angle_min_s` below, is
/// trusted outright — both are tested and ranked like any other candidate.
///
/// `depth_a`/`slope` describe how depth varies along the segment: depth is
/// ALWAYS affine in `s` regardless of parallelism,
/// `depth(s) = depth_a + s*slope`, since `depth(0) = dir·(a-origin) = depth_a`
/// and `depth(1) - depth(0) = dir·seg_dir = slope`.
///
/// Why not just use a single analytically-correct `s`? `cone_test`'s two
/// `ApertureMode`s rank points by two DIFFERENT metrics, each optimized by
/// a DIFFERENT point along the segment's infinite line in general:
/// - `Cylinder` ranks by perpendicular distance to the ray's axis line,
///   depth-independent — minimized by the textbook closest-point-between-
///   two-lines construction, `perp_min_s`. But that point can itself still
///   land behind the origin's depth plane (a point can be very close to the
///   axis while still just barely on the ray's "wrong" side).
/// - `Cone` ranks by ANGLE off the axis, i.e. `depth(s)^2 / dist(s)^2` where
///   `dist` is measured from the ray ORIGIN (not the axis line) — a
///   genuinely different optimum from `perp_min_s` in general, since a
///   segment's perpendicular offset from the axis can vary with `s`
///   independently of its depth. `angle_min_s` is that optimum, derived by
///   maximizing `depth(s)^2/dist(s)^2`; its derivative's `s^2` terms cancel,
///   so — like `perp_min_s` — it has exactly one critical point over the
///   whole (unclamped) line.
///
/// Testing both analytic candidates and deferring to `cone_test`'s own
/// ranking sidesteps having to special-case either mode by hand — an
/// irrelevant candidate for a given mode simply loses the ranking to a
/// relevant one, so folding both in unconditionally is always safe.
/// Candidates:
/// - both endpoints (`s = 0, 1`) — whichever the whole segment favors when
///   it's admissible throughout, and the sole survivor when the other end
///   sits behind the ray origin or otherwise fails its own cone_test;
/// - the depth-zero crossing (where the admissible half begins, if the
///   segment straddles it), nudged a hair to either side so it lands
///   strictly admissible rather than exactly on `cone_test`'s excluded
///   `depth <= 0` boundary — the point `Cylinder` actually wants there;
/// - `perp_min_s`/`angle_min_s` (each clamped to the segment and only
///   included when well-defined), the two analytic optima above.
///
/// The old code hard-coded `s = 0` for the parallel case (silently testing
/// the FIRST endpoint even when it sat behind the ray origin's depth plane
/// while the rest of the segment was admissible ahead of it), didn't handle
/// the behind-the-origin case for a non-parallel segment at all, and (in an
/// intermediate version of this fix) returned `perp_min_s` unconditionally
/// for the non-parallel case instead of ranking it against the endpoints —
/// which is correct for `Cylinder` but not for `Cone`, since `perp_min_s`
/// doesn't account for depth at all. Both were caught by
/// `segment_cone_hit_matches_fine_scan_oracle` only once its draws stopped
/// forcing every segment to be exactly parallel to the ray (see that test's
/// doc).
fn segment_edge_fallback_hit(
    origin: Point3,
    dir: Vec3,
    a: Point3,
    seg_dir: Vec3,
    // depth(s) = depth_a + s*slope (see the doc above); perp_min_s/
    // angle_min_s are the two analytic candidates' `s`, if defined (see the
    // doc above) — bundled since `clippy::too_many_arguments` caps a free
    // function at 7 (this would otherwise be 9).
    (depth_a, slope, perp_min_s, angle_min_s): (f64, f64, Option<f64>, Option<f64>),
    aperture: f64,
    mode: ApertureMode,
) -> Option<(Point3, f64, f64)> {
    // A generous-but-tiny nudge off the depth=0 crossing, in units of DEPTH
    // (not the segment parameter — a fixed `s`-space nudge scales its
    // resulting depth error by `slope`, which can be arbitrarily large for a
    // long, steep segment; `segment_cone_hit_matches_fine_scan_oracle`
    // caught the discrepancy this produced against a fine scan landing even
    // closer to the true, unreachable `depth == 0` infimum). Converting to
    // an `s`-space offset by dividing by `slope` keeps the resulting depth
    // comfortably bigger than float noise yet close to that infimum
    // regardless of the segment's scale.
    const CROSSING_DEPTH_NUDGE: f64 = 1e-9;

    let mut candidates = [0.0_f64, 1.0, f64::NAN, f64::NAN, f64::NAN, f64::NAN];
    let mut n = 2;
    if slope.abs() >= tol::NORMALIZE_MIN_LENGTH {
        let s0 = -depth_a / slope; // depth(s0) == 0
        let s_nudge = CROSSING_DEPTH_NUDGE / slope.abs();
        candidates[2] = (s0 + s_nudge).clamp(0.0, 1.0);
        candidates[3] = (s0 - s_nudge).clamp(0.0, 1.0);
        n = 4;
    }
    if let Some(s) = perp_min_s {
        candidates[n] = s;
        n += 1;
    }
    if let Some(s) = angle_min_s {
        candidates[n] = s;
        n += 1;
    }

    let mut best: Option<(Point3, f64, f64)> = None;
    for &s in &candidates[..n] {
        let point = a + seg_dir * s;
        if let Some((metric, depth)) = cone_test(origin, dir, point, aperture, mode)
            && best
                .as_ref()
                .is_none_or(|&(_, m, d)| (metric, depth) < (m, d))
        {
            best = Some((point, metric, depth));
        }
    }
    best
}

// Test-only counters for whether `segment_cone_hit` found a usable
// `perp_min_s` (the raw line-line closest point) to add as a candidate —
// `(general, fallback)` — so its property test can assert the run actually
// exercised both, rather than trusting a property's construction by
// inspection. `thread_local!` rather than a crate-wide atomic: `cargo test`
// gives each `#[test]` fn its own thread, so counts from unrelated tests
// calling this same function can't leak into one test's total. A prior
// version of the property test gave both segment endpoints the SAME
// lateral offset, making every drawn segment exactly parallel to the ray —
// the general (non-parallel) case below never ran across the whole
// property run, and these counters exist so that regresses loudly instead
// of silently.
#[cfg(test)]
thread_local! {
    static SEGMENT_CONE_HIT_BRANCH_COUNTS: std::cell::Cell<(u64, u64)> =
        const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
fn record_general_branch_hit() {
    SEGMENT_CONE_HIT_BRANCH_COUNTS.with(|c| {
        let (general, fallback) = c.get();
        c.set((general + 1, fallback));
    });
}

#[cfg(test)]
fn record_fallback_branch_hit() {
    SEGMENT_CONE_HIT_BRANCH_COUNTS.with(|c| {
        let (general, fallback) = c.get();
        c.set((general, fallback + 1));
    });
}

/// Finds the closest point on the segment [a, b] to the pick ray, and returns
/// `(position, angular_distance, depth)` if that point lies within `aperture`.
///
/// `dir` must already be normalized.
fn segment_cone_hit(
    origin: Point3,
    dir: Vec3,
    a: Point3,
    b: Point3,
    aperture: f64,
    mode: ApertureMode,
) -> Option<(Point3, f64, f64)> {
    // Closest point between two lines (ray and segment-as-line), then clamp
    // to the segment [0, 1].
    //
    // Ray:     P(t) = origin + t * dir        (t >= 0 for in-front)
    // Segment: Q(s) = a + s * seg_dir         (s in [0, 1])
    let seg_dir = b - a;
    let seg_len_sq = seg_dir.length_squared();
    if seg_len_sq < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        // Degenerate segment (endpoints coincide); treat as a point.
        return cone_test(origin, dir, a, aperture, mode).map(|(ang, depth)| (a, ang, depth));
    }

    let w = origin - a;
    let b_coef = dir.dot(seg_dir); // dot(ray_dir, seg_dir)
    let e = dir.dot(w); // dir · (origin - a) — note depth(s=0) = -e
    let f = seg_dir.dot(w); // seg_dir · (origin - a)

    // denom = |dir|^2 * |seg_dir|^2 - (dir . seg_dir)^2, but |dir|=1 so:
    //       = seg_len_sq - b_coef^2
    let denom = seg_len_sq - b_coef * b_coef;
    let depth_a = -e; // depth(s=0); depth(s) = depth_a + s*b_coef throughout.

    // Segment parameter of the closest point between the (unit-direction)
    // ray and the segment line: s = (f - (dir·seg_dir)(dir·w)) / denom. (The
    // earlier `seg_len_sq * e - b_coef * f` form was the ray parameter's
    // numerator and clamped to the wrong endpoint — caught by
    // segment_closest_point_clamps_to_endpoints.) Kept only as a candidate
    // in `segment_edge_fallback_hit`, not returned directly: it minimizes
    // raw perpendicular distance to the ray's axis line (the `Cylinder`
    // metric) — a different objective from `Cone`'s angle metric, and (per
    // `segment_edge_fallback_hit`'s doc) can itself still land behind the
    // ray origin (`depth(s) = s*b_coef - e = s*b_coef + depth_a`,
    // `cone_test`'s `depth`, since `dir` is unit).
    let perp_min_s = if denom.abs() >= tol::NORMALIZE_MIN_LENGTH {
        let s = ((f - b_coef * e) / denom).clamp(0.0, 1.0);
        (s * b_coef + depth_a > 0.0).then_some(s)
    } else {
        None
    };

    // The point of MINIMUM ANGLE (as opposed to minimum perpendicular
    // distance) along the segment's infinite line — see
    // `segment_edge_fallback_hit`'s doc for why this is a genuinely
    // different point from `perp_min_s` in general. Maximizing
    // `depth(s)^2 / dist(s)^2` (`dist` from the ray ORIGIN) reduces, after
    // its `s^2` terms cancel, to one linear equation in `s`:
    // `q = |origin - a|^2` and `r = (a - origin)·seg_dir = -f` play the role
    // `depth_a`/`b_coef` play for depth, but for `dist_sq(s) = q + 2*r*s +
    // seg_len_sq*s^2` instead.
    let q = w.length_squared();
    let r = -f;
    let angle_denom = b_coef * r - seg_len_sq * depth_a;
    let angle_min_s = if angle_denom.abs() >= tol::NORMALIZE_MIN_LENGTH {
        let s = (r * depth_a - b_coef * q) / angle_denom;
        (0.0..=1.0).contains(&s).then_some(s)
    } else {
        None
    };

    #[cfg(test)]
    if perp_min_s.is_some() {
        record_general_branch_hit();
    } else {
        record_fallback_branch_hit();
    }

    segment_edge_fallback_hit(
        origin,
        dir,
        a,
        seg_dir,
        (depth_a, b_coef, perp_min_s, angle_min_s),
        aperture,
        mode,
    )
}

/// Ray-face intersection: returns `(position, angular_distance, depth)` if
/// the ray hits the face plane in front of the origin and the hit point lies
/// inside the boundary polygon.
///
/// `dir` must already be normalized.
fn face_cone_hit(
    origin: Point3,
    dir: Vec3,
    plane: &Plane,
    boundary: &[Point3],
    holes: &[Vec<Point3>],
    _aperture: f64,
) -> Option<(Point3, f64, f64)> {
    let n = plane.normal();
    let denom = n.dot(dir);
    if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        return None; // ray is parallel to the plane
    }
    // t = (offset - n·origin) / (n·dir)
    let t = -plane.signed_distance(origin) / denom;
    if t <= 0.0 {
        return None; // intersection is behind the ray origin
    }
    let hit = origin + dir * t;

    // Point-in-polygon test: project to plane's local 2D axes.
    if !point_in_polygon(hit, boundary, n) {
        return None;
    }
    // Reject hits that land inside a hole: the ray passes through the opening,
    // not the face material (e.g. the annular parent of an imprinted sub-face).
    if holes.iter().any(|hole| point_in_polygon(hit, hole, n)) {
        return None;
    }

    // The depth is t (already the ray parameter with normalized dir).
    // For an on-face snap, the angular distance from the ray axis is 0
    // (the ray goes through the hit point by definition). Use t as depth.
    Some((hit, 0.0, t))
}

/// 2D point-in-polygon test using ray casting.
///
/// Projects `point` and all `boundary` vertices onto the plane defined by
/// `normal`, using an orthonormal basis derived from `normal`, then runs the
/// standard ray-casting test.
fn point_in_polygon(point: Point3, boundary: &[Point3], normal: Vec3) -> bool {
    if boundary.len() < 3 {
        return false;
    }
    // Build a local 2D basis on the plane.
    let (u, v) = plane_basis(normal);

    // Project into 2D.
    let to2d = |p: Point3| -> (f64, f64) {
        let pv = p.to_vec();
        (u.dot(pv), v.dot(pv))
    };

    let (px, py) = to2d(point);
    let verts: Vec<(f64, f64)> = boundary.iter().map(|&p| to2d(p)).collect();
    let n = verts.len();

    // Standard ray-casting: count crossings of a ray from (px, py) in +x.
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = verts[i];
        let (xj, yj) = verts[j];
        // Edge i->j crosses the horizontal ray from (px, py) if one endpoint
        // is above and the other at or below py, and the crossing x > px.
        if (yi > py) != (yj > py) {
            let cross_x = xj + (py - yj) * (xi - xj) / (yi - yj);
            if px < cross_x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Constructs an orthonormal basis (u, v) on a plane with unit normal `n`,
/// such that u × v = n (right-handed).
fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    // Pick a helper vector not parallel to n.
    let helper = if n.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = helper
        .cross(n)
        .normalized()
        .expect("helper is never parallel to a unit normal");
    let v = n.cross(u);
    (u, v)
}

/// The facts a locked [`SnapQuery`] contributes to picking and reporting a
/// winner, resolved once per query before the winner cull runs (see the
/// `lock_ctx` binding in `resolve_impl`, which documents each field's role
/// and the defects the two ray-judging booleans close).
///
/// A struct rather than a tuple purely for safety: `on_anchor` and
/// `on_lock_line` are adjacent booleans on the same line, and swapping them
/// silently would restore the wrong-side over-culling this type exists to
/// keep confined.
#[derive(Debug, Clone, Copy)]
struct LockCtx {
    /// The lock's unit direction, resolved through the drawing-axes frame.
    lock_dir: Vec3,
    /// The gesture's fixed point; the lock line passes through it.
    anchor: Point3,
    /// The pick ray is on the anchor itself at pixel resolution.
    on_anchor: bool,
    /// The pick ray is on the lock's guide line at pixel resolution — what
    /// makes `t_fallback` a statement of intent rather than of parallax.
    on_lock_line: bool,
    /// `fall`'s signed station along `lock_dir` from `anchor`.
    t_fallback: f64,
    /// The lock line's point nearest the ray: the no-winner fallback.
    fall: Point3,
}

/// Projects `point` onto the line `anchor + t * dir` (dir must be unit).
/// Returns the projected point, which lies exactly on the line.
/// The point where the infinite line `(o, d)` crosses the segment `[a, b]`,
/// or `None` when they are parallel, skew beyond [`tol::POINT_MERGE`], or
/// the crossing falls outside the segment (with [`tol::POINT_MERGE`] of
/// world-distance slack at the endpoints). The returned point lies ON the
/// segment (real geometry), not on the guide.
fn line_segment_intersection(o: Point3, d: Vec3, a: Point3, b: Point3) -> Option<Point3> {
    let e = b - a;
    let seg_len2 = e.dot(e);
    if seg_len2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return None; // degenerate segment
    }
    let dd = d.dot(d);
    if dd < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return None; // degenerate guide direction
    }
    // Solve min |o + s·d − (a + t·e)| for (s, t) — standard line/line closest
    // points. denom / (dd·seg_len2) = sin²θ between the directions; treat the
    // pair as parallel below the same normalize floor segment_cone_hit uses.
    let w = o - a;
    let de = d.dot(e);
    let denom = dd * seg_len2 - de * de;
    if denom < dd * seg_len2 * tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let dw = d.dot(w);
    let ew = e.dot(w);
    let t = (dd * ew - de * dw) / denom;
    // Endpoint slack in world distance, expressed in the parameter t.
    let t_slack = tol::POINT_MERGE / seg_len2.sqrt();
    if t < -t_slack || t > 1.0 + t_slack {
        return None;
    }
    let t = t.clamp(0.0, 1.0);
    let on_seg = Point3::new(a.x + e.x * t, a.y + e.y * t, a.z + e.z * t);
    let s = (de * t - dw) / dd; // closest param on the guide for that t
    let on_line = Point3::new(o.x + d.x * s, o.y + d.y * s, o.z + d.z * s);
    if (on_seg - on_line).length() > tol::POINT_MERGE {
        return None; // skew — the lines pass near, not through, each other
    }
    Some(on_seg)
}

/// Who owns an edge, for the edge × edge crossing test: edges pair only
/// across owners. A placed edge's owner is its placement (index into
/// `InferenceScene::placements`), so two placements of one member — or two
/// members of one instance — are separate geometry, as they are to the
/// kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EdgeOwner {
    World(ObjectId),
    Placement(usize),
    Sketch(SketchId),
}

/// One edge in a query's crossing pool (see `resolve_impl`).
#[derive(Debug, Clone, Copy)]
struct CrossingEdge {
    a: Point3,
    b: Point3,
    /// The edge's angular distance from the pick ray (its OnEdge metric):
    /// the order pairs are tested in.
    ang: f64,
    owner: EdgeOwner,
    edge: CrossedEdge,
}

/// How many pairs of `pool` have different owners: all pairs, less those
/// within each owner's group.
fn cross_owner_pairs<T>(pool: &[T], owner: impl Fn(&T) -> EdgeOwner) -> usize {
    let mut owners: Vec<EdgeOwner> = pool.iter().map(owner).collect();
    owners.sort_unstable();
    let pairs = |n: usize| n * n.saturating_sub(1) / 2;
    let same: usize = owners
        .chunk_by(|x, y| x == y)
        .map(|group| pairs(group.len()))
        .sum();
    pairs(owners.len()) - same
}

/// What [`walk_cross_owner_pairs`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PairWalk {
    /// Cross-owner pairs visited.
    tested: usize,
    /// Loop steps taken: every pair visited, plus every step over a run of
    /// the later edge's own owner. Bounded by the cap.
    steps: usize,
    /// Cross-owner pairs the cap left unvisited (0 when the walk ran to
    /// its end).
    skipped: usize,
}

/// Visits the cross-owner pairs `(e, f)` of `pool` (`e` before `f`) in
/// nearest-first order: every pair among the first `j` items before any
/// pair with item `j + 1`, and the earlier item ascending within those. At
/// most `cap` steps are taken, and every step counts: a pair visited, or a
/// skip over a run of `f`'s own owner. Items are the edges of the edge ×
/// edge crossing walk or the faces of the face × face meeting walk; `owner`
/// reads an item's owner.
///
/// The pool is cut into maximal runs of one owner, and item `f` walks the
/// runs before its own, stepping over those of its owner whole. Two
/// neighboring runs never share an owner, so the runs `f` steps over number
/// at most one more than the other-owner runs it visits, and the walk costs
/// at most `2 * pairs + items` steps, never the `items²` of every pair. A
/// pool of one owner returns at once, with no step taken.
fn walk_cross_owner_pairs<T>(
    pool: &[T],
    owner: impl Fn(&T) -> EdgeOwner,
    cap: usize,
    mut visit: impl FnMut(&T, &T),
) -> PairWalk {
    let mut walk = PairWalk {
        tested: 0,
        steps: 0,
        skipped: 0,
    };
    let Some(first) = pool.first() else {
        return walk;
    };
    let first_owner = owner(first);
    if pool.iter().all(|e| owner(e) == first_owner) {
        return walk;
    }
    // Maximal runs of one owner, as `start..end` index ranges.
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, e) in pool.iter().enumerate() {
        match runs.last_mut() {
            Some(r) if owner(&pool[r.start]) == owner(e) => r.end = i + 1,
            _ => runs.push(i..i + 1),
        }
    }
    for (r, run) in runs.iter().enumerate() {
        let run_owner = owner(&pool[run.start]);
        for f in &pool[run.clone()] {
            for earlier in &runs[..r] {
                if walk.steps == cap {
                    walk.skipped = cross_owner_pairs(pool, &owner) - walk.tested;
                    return walk;
                }
                if owner(&pool[earlier.start]) == run_owner {
                    walk.steps += 1;
                    continue;
                }
                for e in &pool[earlier.clone()] {
                    if walk.steps == cap {
                        walk.skipped = cross_owner_pairs(pool, &owner) - walk.tested;
                        return walk;
                    }
                    walk.steps += 1;
                    walk.tested += 1;
                    visit(e, f);
                }
            }
        }
    }
    walk
}

/// Where segments `[a0, a1]` and `[b0, b1]` truly cross in 3D: their closest
/// points lie within [`tol::POINT_MERGE`] of each other and strictly inside
/// both segments — more than [`tol::POINT_MERGE`] from all four endpoints,
/// since an endpoint already has its own Endpoint snap. Returns the
/// midpoint of the two closest points. `None` for parallel (including
/// collinear, overlapping) or degenerate segments, which have no single
/// crossing.
fn segment_crossing(a0: Point3, a1: Point3, b0: Point3, b1: Point3) -> Option<Point3> {
    let da = a1 - a0;
    let db = b1 - b0;
    let aa = da.dot(da);
    let bb = db.dot(db);
    if aa < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH
        || bb < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH
    {
        return None; // degenerate segment
    }
    let ab = da.dot(db);
    // denom / (aa·bb) = sin²θ — the same parallel floor as
    // `line_line_intersection`.
    let denom = aa * bb - ab * ab;
    if denom < aa * bb * tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let w = a0 - b0;
    let aw = da.dot(w);
    let bw = db.dot(w);
    let s = (ab * bw - bb * aw) / denom;
    let t = (aa * bw - ab * aw) / denom;
    if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
        return None; // the lines' closest approach is off a segment
    }
    let pa = a0 + da * s;
    let pb = b0 + db * t;
    if (pa - pb).length() > tol::POINT_MERGE {
        return None; // skew: the edges pass near, not through, each other
    }
    let p = midpoint(pa, pb);
    if [a0, a1, b0, b1]
        .iter()
        .any(|&end| p.approx_eq(end, tol::POINT_MERGE))
    {
        return None; // at an endpoint: the Endpoint snap covers it
    }
    Some(p)
}

/// One face the pick cone reaches, in a query's face pool (see
/// `resolve_impl`): a world or placed solid face, or a committed sketch
/// region's fill, with the owner its edges belong to — an edge never pierces
/// its own owner's faces, and faces of one owner never meet along a virtual
/// line — and how close it comes to the pick ray and at what depth
/// (`face_cone_key`), the orders the piercing and meeting-line walks take
/// faces in.
#[derive(Clone, Copy)]
struct ConeFace<'a> {
    plane: &'a Plane,
    boundary: &'a [Point3],
    holes: &'a [Vec<Point3>],
    owner: EdgeOwner,
    face: PiercedFace,
    /// See [`SceneFace::surface_facet`]; `false` for a drawn region.
    surface_facet: bool,
    /// The face's distance from the ray in the query's aperture units
    /// (`CrossingEdge::ang`'s units): 0 when the ray passes through it.
    key: f64,
    /// How far along the ray the face comes nearest it: where the ray
    /// passes through the face, or the depth of the boundary point that
    /// passes nearest (`face_cone_key`). Orders faces front to back.
    depth: f64,
}

/// A face of the meeting-line walk: the pooled face and its box
/// ([`region_aabb`]), taken once per face so a pair's reject
/// (`meeting_line_reaches`) costs no walk over either ring.
struct MeetingFace<'a> {
    face: ConeFace<'a>,
    bounds: Aabb,
}

/// How close a face comes to the pick ray, for the face pools, and at what
/// depth: `Some((0, t))` when the ray passes through the face's material,
/// `t` the depth of that point along the ray, else the smallest distance
/// from the ray (an angle under `Cone`, metres under `Cylinder` —
/// `cone_test`'s units) at which a point of the face's boundary, outer loop
/// or hole, passes it, with the smallest depth among the boundary edges'
/// nearest points within `aperture`; or `None` when no point of the face
/// is within `aperture` of the ray. Exact for the face as a region, not a box: the
/// cone is convex, so when it meets the face's material and the ray itself
/// misses the material, a straight path inside the cone from the ray's
/// point in the face's plane to a point of the material crosses the
/// boundary — and with the ray parallel to the plane the cone's trace in
/// the plane is unbounded while the face is not, so the boundary is met
/// again. The indexed and the linear reference path both apply it to
/// every face they see, so they pool the same faces (the index is a
/// superset by box, and every point this test accepts lies in the face's
/// box) in the same order.
fn face_cone_key(
    origin: Point3,
    dir: Vec3,
    plane: &Plane,
    boundary: &[Point3],
    holes: &[Vec<Point3>],
    aperture: f64,
    mode: ApertureMode,
) -> Option<(f64, f64)> {
    if let Some((_, _, depth)) = face_cone_hit(origin, dir, plane, boundary, holes, aperture) {
        return Some((0.0, depth));
    }
    let mut nearest: Option<(f64, f64)> = None;
    for ring in std::iter::once(boundary).chain(holes.iter().map(Vec::as_slice)) {
        for (i, &a) in ring.iter().enumerate() {
            let b = ring[(i + 1) % ring.len()];
            if let Some((_, dist, depth)) = segment_cone_hit(origin, dir, a, b, aperture, mode) {
                nearest = Some(nearest.map_or((dist, depth), |(n, d): (f64, f64)| {
                    (n.min(dist), d.min(depth))
                }));
            }
        }
    }
    nearest
}

/// Visits the cross-owner pairs `(e, f)` of `edges` × `faces` — each sorted
/// nearest the ray first (`CrossingEdge::ang`, `ConeFace::key`) — in order
/// of the pair's distance from the ray, the farther of the two: the sorted
/// lists are merged, and each item, as it is reached, pairs with every item
/// of the other list reached before it. The piercing under the cursor,
/// whose edge and face both pass close to the ray, is therefore tested
/// before any pair with a farther edge or face, and a cap cuts off the
/// farthest pairs first. Pairs of one owner are skipped free (an edge never
/// pierces its own owner's faces); at most `cap` cross-owner pairs are
/// visited, and the rest are counted as skipped. One owner on both sides
/// visits nothing.
fn walk_pierce_pairs<'a>(
    edges: &[CrossingEdge],
    faces: &[ConeFace<'a>],
    cap: usize,
    mut visit: impl FnMut(&CrossingEdge, &ConeFace<'a>),
) -> PairWalk {
    let mut walk = PairWalk {
        tested: 0,
        steps: 0,
        skipped: 0,
    };
    let Some(first_edge) = edges.first() else {
        return walk;
    };
    if faces.is_empty() {
        return walk;
    }
    // One owner on both sides: nothing to pair.
    if edges.iter().all(|e| e.owner == first_edge.owner)
        && faces.iter().all(|f| f.owner == first_edge.owner)
    {
        return walk;
    }
    debug_assert!(
        edges.is_sorted_by(|x, y| x.ang <= y.ang),
        "edges nearest first"
    );
    debug_assert!(
        faces.is_sorted_by(|x, y| x.key <= y.key),
        "faces nearest first"
    );
    // How many pairs the walk owes: every edge against every face not of
    // its owner.
    let cross_pairs = || -> usize {
        edges
            .iter()
            .map(|e| faces.iter().filter(|f| f.owner != e.owner).count())
            .sum()
    };
    // Visits one pair unless it is one owner's; `false` once the cap is
    // reached.
    let mut try_pair = |e: &CrossingEdge, f: &ConeFace<'a>| -> bool {
        if e.owner == f.owner {
            return true;
        }
        if walk.tested == cap {
            return false;
        }
        walk.tested += 1;
        walk.steps += 1;
        visit(e, f);
        true
    };
    let (mut i, mut j) = (0, 0);
    'merge: while i < edges.len() || j < faces.len() {
        // Take the nearer head; it pairs with everything already taken on
        // the other side, so every pair visited now is at least as far as
        // every pair visited before it.
        let take_edge = j == faces.len() || (i < edges.len() && edges[i].ang <= faces[j].key);
        if take_edge {
            for f in &faces[..j] {
                if !try_pair(&edges[i], f) {
                    break 'merge;
                }
            }
            i += 1;
        } else {
            for e in &edges[..i] {
                if !try_pair(e, &faces[j]) {
                    break 'merge;
                }
            }
            j += 1;
        }
    }
    walk.skipped = cross_pairs() - walk.tested;
    walk
}

/// Where segment `[a, b]` passes through the planar face with `plane`,
/// outer `boundary` and `holes`: `a` and `b` lie strictly on opposite sides
/// of the plane — each further than [`tol::PLANE_DIST`] from it — the
/// segment meets the plane at more than [`tol::EDGE_FACE_GRAZING_SINE`],
/// and the point where it crosses lies inside the boundary polygon and
/// outside every hole (the same containment test an `OnFace` hit uses).
/// `None` when either endpoint is on the plane (an edge that ends on the
/// face is a T meeting the Endpoint snap already names; an edge lying in
/// the plane has no single crossing point), when the edge grazes the plane
/// (its ends a few nanometres either side, as a transform chain leaves a
/// flush panel's edge: in the plane for every purpose, and the crossing an
/// arbitrary point along it), when both endpoints are on one side (which
/// covers every edge parallel to the plane that is not in it), or when the
/// crossing misses the face's material. A crossing exactly on the face's
/// boundary is the face's own edge crossing this one — `segment_crossing`'s
/// case — and may or may not be reported here too; both land on the same
/// point at the same tier.
///
/// Precision: each signed distance carries rounding of order ε times the
/// coordinate magnitude (the plane's offset is one f64, so a local origin
/// would not remove it), and the crossing's error along the edge is that
/// divided by the sine of the edge's angle to the plane; the grazing gate
/// bounds it (see the tolerance's doc), and at ordinary angles it is far
/// below [`tol::POINT_MERGE`] even 100 km from the origin.
fn segment_pierces_face(
    a: Point3,
    b: Point3,
    plane: &Plane,
    boundary: &[Point3],
    holes: &[Vec<Point3>],
) -> Option<Point3> {
    let da = plane.signed_distance(a);
    let db = plane.signed_distance(b);
    if da.abs() <= tol::PLANE_DIST || db.abs() <= tol::PLANE_DIST {
        return None; // an endpoint on the plane: a T meeting, or in-plane
    }
    if (da > 0.0) == (db > 0.0) {
        return None; // both ends on one side: no crossing (parallel included)
    }
    // `da - db` is the edge's length times the sine of its angle to the
    // plane; below the grazing sine the edge is in the plane for every
    // practical purpose.
    if (da - db).abs() < (b - a).length() * tol::EDGE_FACE_GRAZING_SINE {
        return None; // grazing: no usable crossing point
    }
    let t = da / (da - db);
    let p = a + (b - a) * t;
    let n = plane.normal();
    if !point_in_polygon(p, boundary, n) {
        return None; // the crossing is beside the face
    }
    if holes.iter().any(|hole| point_in_polygon(p, hole, n)) {
        return None; // the crossing is through a hole in the face
    }
    Some(p)
}

/// A planar face as the meeting-line test sees it: its plane, its outer
/// loop in cycle order, and its holes.
type FaceGeom<'a> = (&'a Plane, &'a [Point3], &'a [Vec<Point3>]);

/// The stretches of the line where the planes of faces `a` and `b` meet
/// that lie inside BOTH faces — inside each outer loop and outside each
/// hole — as `(start, end)` pairs in world space, in order along the line.
/// The reference form of the meeting walk's test: `resolve_impl` runs the
/// same [`planes_meeting_line`] and [`meeting_segments_on`] with a cheap
/// reject between them and buffers kept across pairs.
#[cfg(test)]
fn face_meeting_segments(a: FaceGeom<'_>, b: FaceGeom<'_>) -> Vec<(Point3, Point3)> {
    let Some(line) = planes_meeting_line(a.0, b.0, a.1) else {
        return Vec::new();
    };
    let mut scratch = MeetingScratch::default();
    meeting_segments_on(line, a, b, &mut scratch);
    scratch.out
}

/// The line where planes `a` and `b` meet, as `(p, d)`: `d` the unit cross
/// product of the normals and `p` a point on both planes — the one nearest
/// the origin, re-based at the foot of `near`'s first vertex (the first
/// face's) so parameters along the line stay at the faces' own scale.
/// `None` when the planes are parallel within
/// [`tol::FACE_MEETING_PARALLEL_SINE`] (coplanar ones included: they meet
/// in a region, not a line) or `near` is empty.
///
/// Precision: `p` carries the planes' offset rounding divided by the sine
/// of their angle, which the parallel gate bounds (see the tolerance's
/// doc).
fn planes_meeting_line(a: &Plane, b: &Plane, near: &[Point3]) -> Option<(Point3, Vec3)> {
    let &first = near.first()?;
    let na = a.normal();
    let nb = b.normal();
    let cross = na.cross(nb);
    let sine = cross.length();
    if sine < tol::FACE_MEETING_PARALLEL_SINE {
        return None; // parallel (or coplanar): no line
    }
    let d = cross / sine;
    // With planes `n · x = o`, the point nearest the origin on both is
    // `(o_a (n_b × u) + o_b (u × n_a)) / |u|²`, `u = n_a × n_b`.
    let oa = na.dot(a.point().to_vec());
    let ob = nb.dot(b.point().to_vec());
    let nearest = (nb.cross(cross) * oa + cross.cross(na) * ob) / (sine * sine);
    let p0 = Point3::new(nearest.x, nearest.y, nearest.z);
    Some((p0 + d * (first - p0).dot(d), d))
}

/// Whether the meeting line `(p, d)` of two faces with boxes `a` and `b`
/// can hold a point within `reach` of the pick ray: the part of the line
/// inside both boxes ([`Aabb::line_span_in_both`]) passes that near the ray
/// (`segment_cone_hit`). Every stretch of the line inside both faces lies
/// in both boxes, so `false` means no point of any stretch — its middle
/// included — passes within `reach`, and the pair's rings need no walk. The
/// reach carries the index prune's guard band ([`tol::CONE_SLACK`]), so a
/// point `cone_test` admits on its own rounding is never rejected here on
/// this test's; the reject only ever skips work.
fn meeting_line_reaches(
    (p, d): (Point3, Vec3),
    a: Aabb,
    b: Aabb,
    origin: Point3,
    dir: Vec3,
    reach: f64,
    mode: ApertureMode,
) -> bool {
    let Some((t0, t1)) = a.line_span_in_both(b, p, d) else {
        return false;
    };
    let reach = reach * (1.0 + tol::CONE_SLACK) + tol::CONE_SLACK;
    segment_cone_hit(origin, dir, p + d * t0, p + d * t1, reach, mode).is_some()
}

/// Buffers [`meeting_segments_on`] works in, kept across the pairs of one
/// walk so a pair allocates nothing once they have grown. `out` holds the
/// last pair's stretches.
#[derive(Default)]
struct MeetingScratch {
    ts: Vec<f64>,
    above: Vec<(f64, f64)>,
    below: Vec<(f64, f64)>,
    in_a: Vec<(f64, f64)>,
    in_b: Vec<(f64, f64)>,
    both: Vec<(f64, f64)>,
    out: Vec<(Point3, Point3)>,
}

/// Fills `scratch.out` with the stretches of the meeting line `(p, d)` of
/// faces `a` and `b` (from [`planes_meeting_line`]) that lie inside BOTH
/// faces — inside each outer loop and outside each hole — as `(start, end)`
/// pairs in world space, in order along the line. Usually one; a hole or a
/// concave loop in either face can cut the line into several, and most
/// pairs of faces have none. A stretch shorter than [`tol::POINT_MERGE`] —
/// the kernel's own definition of a degenerate segment — is dropped, and
/// stretches that touch end to end (the line passing a vertex that touches
/// it from inside the face) are one stretch.
///
/// Each face is cut to the chords of the line through its INTERIOR by the
/// even-odd rule over every ring: a ring edge whose ends lie on opposite
/// sides of the line crosses it at one parameter, and the sorted parameters
/// pair up into chords — a closed ring crosses a line an even number of
/// times, and a hole's chord inside the outer loop's splits it in two. A
/// vertex within [`tol::PLANE_DIST`] of the other face's plane lies ON the
/// line (its distance from the line in its own face is that over the
/// planes' sine), and is classified the same way at every rotation and
/// scale rather than by which side rounding puts it: the chords are taken
/// once with every on-line vertex counted above the line and once below,
/// and the face's chords are where both agree. That is the line through the
/// face's interior: a face that only touches the line along its own edge (a
/// box flush against a wall: its top face meets the wall along its back
/// edge) yields nothing there — the real edge is the line, and it already
/// snaps — while a face the line crosses yields the same chords either way.
/// The two faces' chord lists are then intersected.
fn meeting_segments_on(
    (p, d): (Point3, Vec3),
    a: FaceGeom<'_>,
    b: FaceGeom<'_>,
    scratch: &mut MeetingScratch,
) {
    let MeetingScratch {
        ts,
        above,
        below,
        in_a,
        in_b,
        both,
        out,
    } = scratch;
    out.clear();
    // The distance from the line, within a face, at which a vertex lies on
    // the other face's plane.
    let sine = a.0.normal().cross(b.0.normal()).length();
    let on_line = tol::PLANE_DIST / sine;
    // The chords `[t0, t1]` of the line inside one face, in order, with
    // every on-line vertex counted on the side `on_above` says.
    let chords = |(plane, boundary, holes): FaceGeom<'_>,
                  on_above: bool,
                  ts: &mut Vec<f64>,
                  into: &mut Vec<(f64, f64)>| {
        // In the face's plane, perpendicular to the line: which side of the
        // line a vertex lies on.
        let side = d.cross(plane.normal());
        let at = |v: Point3| -> (f64, bool, f64) {
            let s = (v - p).dot(side);
            let on = s.abs() <= on_line;
            (s, if on { on_above } else { s > 0.0 }, (v - p).dot(d))
        };
        ts.clear();
        for ring in std::iter::once(boundary).chain(holes.iter().map(Vec::as_slice)) {
            if ring.len() < 3 {
                continue;
            }
            for (i, &v0) in ring.iter().enumerate() {
                let v1 = ring[(i + 1) % ring.len()];
                let (s0, up0, t0) = at(v0);
                let (s1, up1, t1) = at(v1);
                if up0 == up1 {
                    continue;
                }
                // An on-line end is where the edge meets the line; else the
                // ends lie strictly either side and the crossing is between.
                ts.push(if s0.abs() <= on_line {
                    t0
                } else if s1.abs() <= on_line {
                    t1
                } else {
                    t0 + s0 / (s0 - s1) * (t1 - t0)
                });
            }
        }
        ts.sort_by(f64::total_cmp);
        into.clear();
        into.extend(ts.chunks_exact(2).map(|c| (c[0], c[1])));
    };
    let interior = |face: FaceGeom<'_>,
                    ts: &mut Vec<f64>,
                    above: &mut Vec<(f64, f64)>,
                    below: &mut Vec<(f64, f64)>,
                    into: &mut Vec<(f64, f64)>| {
        chords(face, true, ts, above);
        chords(face, false, ts, below);
        intersect_spans(above, below, into);
    };
    interior(a, ts, above, below, in_a);
    if in_a.is_empty() {
        return;
    }
    interior(b, ts, above, below, in_b);
    intersect_spans(in_a, in_b, both);
    // Join stretches that touch end to end, then drop degenerate ones.
    let mut joined: Option<(f64, f64)> = None;
    for &(lo, hi) in both.iter() {
        match joined {
            Some((jlo, jhi)) if lo - jhi <= tol::POINT_MERGE => joined = Some((jlo, jhi.max(hi))),
            Some((jlo, jhi)) => {
                if jhi - jlo > tol::POINT_MERGE {
                    out.push((p + d * jlo, p + d * jhi));
                }
                joined = Some((lo, hi));
            }
            None => joined = Some((lo, hi)),
        }
    }
    if let Some((jlo, jhi)) = joined
        && jhi - jlo > tol::POINT_MERGE
    {
        out.push((p + d * jlo, p + d * jhi));
    }
}

/// The intersection of two sorted lists of disjoint closed spans, sorted,
/// into `out` (spans that only touch give a zero-length span; the caller
/// drops what is too short).
fn intersect_spans(a: &[(f64, f64)], b: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    out.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo <= hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// The point where two infinite lines cross, or `None` when parallel or
/// skew beyond [`tol::POINT_MERGE`]. Returns the midpoint of the closest
/// pair (exact crossing → the crossing itself).
fn line_line_intersection(ao: Point3, ad: Vec3, bo: Point3, bd: Vec3) -> Option<Point3> {
    let w = ao - bo;
    let aa = ad.dot(ad);
    let bb = bd.dot(bd);
    if aa < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH
        || bb < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH
    {
        return None; // degenerate direction
    }
    let ab = ad.dot(bd);
    // denom / (aa·bb) = sin²θ — same parallel floor as the segment case.
    let denom = aa * bb - ab * ab;
    if denom < aa * bb * tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let aw = ad.dot(w);
    let bw = bd.dot(w);
    let s = (ab * bw - bb * aw) / denom;
    let t = (aa * bw - ab * aw) / denom;
    let pa = Point3::new(ao.x + ad.x * s, ao.y + ad.y * s, ao.z + ad.z * s);
    let pb = Point3::new(bo.x + bd.x * t, bo.y + bd.y * t, bo.z + bd.z * t);
    if (pa - pb).length() > tol::POINT_MERGE {
        return None;
    }
    Some(Point3::new(
        (pa.x + pb.x) / 2.0,
        (pa.y + pb.y) / 2.0,
        (pa.z + pb.z) / 2.0,
    ))
}

fn project_onto_line(anchor: Point3, dir: Vec3, point: Point3) -> Point3 {
    let t = (point - anchor).dot(dir);
    anchor + dir * t
}

/// The largest distance [`closest_point_on_line_to_ray`] will place its
/// result from `line_origin`, as a multiple of the distance from
/// `ray_origin` to `line_origin` — playtest-2 review finding F.
///
/// The formula's `t` is divided by `1 - b²` (`b` = cos of the angle between
/// the line and the ray), so as the ray approaches parallel to the line the
/// result races toward infinity — and does so in a way that is NOT bounded
/// by scene scale on its own: empirically, a camera ~35m from `line_origin`
/// sighting exactly [`SOFT_AXIS_EDGE_ON_DEG`] (3°) off parallel resolves to
/// a point roughly 400m away, and a single pixel of mouse motion (~0.06° of
/// ray-direction change) swings that point by several METERS. The edge-on
/// guard only excludes candidates BELOW that angle; it does nothing to
/// bound the result at or past it, and [`tol::NORMALIZE_MIN_LENGTH`]'s own
/// guard (the function's only other safeguard) catches merely EXACT
/// parallelism, not merely-near. No fixed edge-on angle threshold can fix
/// this on its own either: the swing scales linearly with camera distance
/// at a FIXED angle, so a threshold tuned for one scene scale is
/// simultaneously too loose for a larger one and needless for a smaller
/// one. Clamping `t` directly, scaled to the query's own distance instead
/// of a hardcoded absolute, is scale-invariant and protects every caller of
/// this function (soft-axis, the literal origin axis, guide lines, and the
/// hard axis lock), not just the one with an edge-on guard.
///
/// 5.0 was chosen empirically (see `closest_point_on_line_to_ray_stays_bounded_near_the_edge_on_angle`):
/// it fully saturates (swing → 0 for a single-pixel ray perturbation) the
/// 3-5° range right at the edge-on boundary, while leaving well-conditioned,
/// broadside-ish rays (where the unclamped result is already well within
/// this reach) completely untouched.
///
/// [`soft_axis_deviation`] does NOT reuse this constant (tool-parity
/// delta-review finding: reach-clamp regression). An earlier revision
/// clamped that function's own ray-side parameter (`s`, the point `Q`)
/// against this same eye-to-anchor-scaled bound, but `s` there has a
/// different scale entirely: for a dead-on aim it is essentially the
/// camera-to-target depth along the RAY, which grows with drag length, not
/// with eye-to-anchor distance. Reusing this bound made an ordinary
/// "zoom in close, drag a long way along the axis" interaction — drag
/// length a few times the eye-to-anchor distance — false-reject a
/// mathematically dead-on aim. See that function's doc for why no clamp is
/// needed there at all.
const MAX_AXIS_REACH_FACTOR: f64 = 5.0;

/// Closest point on the line `line_origin + t * line_dir` to the ray
/// `ray_origin + s * ray_dir`. Returns the point on the line (not the ray).
///
/// If lines are parallel, returns the point on the line closest to
/// `ray_origin`.
///
/// `t` is clamped to `±MAX_AXIS_REACH_FACTOR` times the distance from
/// `ray_origin` to the clamp's reference origin (playtest-2 review finding
/// F) — see that constant's doc for why an unclamped result is unsafe near
/// (not just at) exact parallelism, and why a distance-scaled clamp is the
/// fix rather than a differently-tuned angle threshold.
///
/// `reach_ref` — the query's own reference point for the clamp (the
/// gesture's anchor), when it has one. INVARIANT: for a fixed ray and a
/// fixed `reach_ref`, any two `(line_origin, ±line_dir)` descriptions of
/// the same geometric line produce the same clamped point (to fp
/// rounding), because both the clamp's center and its bound are derived
/// from `reach_ref`'s own foot on the line — a fact about the line as a
/// SET — never from whichever origin happens to parameterize it. Without
/// this, a construction guide collinear with an axis-locked line but
/// parameterized from an origin near the camera clamped METRES away from
/// where the lock's own fallback (parameterized from the anchor) clamped
/// the identical construction, and the locked winner cull's "this
/// candidate IS the fallback point" noise test could not see they were
/// the same thing (round-7 review CRITICAL). With no `reach_ref` (an
/// unanchored query), the line's own origin is the reference, exactly as
/// before — those callers compare nothing across parameterizations.
fn closest_point_on_line_to_ray(
    line_origin: Point3,
    line_dir: Vec3,
    reach_ref: Option<Point3>,
    ray_origin: Point3,
    ray_dir: Vec3,
) -> Point3 {
    // Re-origin the line at the reference's own foot on it (for a line
    // THROUGH the reference — the lock fallback, the soft-axis candidates —
    // this is exactly the reference itself, bit for bit, so those callers
    // are untouched).
    let origin = match reach_ref {
        Some(r) => project_onto_line(line_origin, line_dir, r),
        None => line_origin,
    };
    // Standard closest-point-between-two-lines derivation.
    let w = origin - ray_origin;
    let b = line_dir.dot(ray_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        // Lines are parallel: project ray_origin onto the line.
        return project_onto_line(origin, line_dir, ray_origin);
    }
    let d = line_dir.dot(w);
    let e = ray_dir.dot(w);
    let t = (b * e - d) / denom;
    let bound = w.length() * MAX_AXIS_REACH_FACTOR;
    let t = t.clamp(-bound, bound);
    origin + line_dir * t
}

/// True iff `axis_dir` is too close to parallel with `ray_dir` for a
/// soft-axis candidate computed from it (`closest_point_on_line_to_ray`) to
/// be trustworthy — see [`SOFT_AXIS_EDGE_ON_DEG`]. Both arguments must
/// already be unit vectors; the comparison is orientation-free (an axis
/// sighted from either end is equally edge-on).
fn axis_is_edge_on(ray_dir: Vec3, axis_dir: Vec3) -> bool {
    let cos_edge_on = (SOFT_AXIS_EDGE_ON_DEG * std::f64::consts::PI / 180.0).cos();
    ray_dir.dot(axis_dir).abs() >= cos_edge_on
}

/// The soft-axis candidate's admission angle (radians): find `Q`, the point
/// on the pick RAY `(origin, dir)` closest to the candidate axis LINE
/// (through `anchor`, direction `axis_dir`) — the ray-side counterpart of
/// [`closest_point_on_line_to_ray`]'s result (call it `P`), which lies on
/// the axis by construction and so can never itself express deviation from
/// it. The admission angle is the angle between `(Q - anchor)` and
/// `axis_dir`, taking the along-axis component as `abs()` so a backward
/// drag — aiming at a point on the axis on `anchor`'s OTHER side — reads
/// identically to a forward one (the axis is a full line through `anchor`,
/// not a ray from it). `None` when `anchor` is behind the ray origin, when
/// `Q` itself resolves behind the ray origin, when the ray is
/// (near-)parallel to the axis, or when the resulting angle exceeds
/// `soft_axis_aperture`.
///
/// This is the SECOND generation of this function (tool-parity delta-review
/// finding 1). The FIRST generation evaluated at the point on the ray at
/// `anchor`'s own depth ALONG THE RAY (`origin + dir * dir.dot(anchor -
/// origin)`), using the axis-perpendicular component of that point's offset
/// from `anchor`, divided by the EYE-relative depth. That mixes reference
/// frames — an axis-relative numerator over an eye-relative denominator is
/// not a valid angle — and for a dead-on aim (the ray genuinely passes
/// through a point ON the axis) the two only coincide when `dir` happens to
/// be perpendicular to `axis_dir`, i.e. a near-straight-overhead camera,
/// which is what every fixture in this suite used before this finding.
/// From an ordinary oblique camera the old formula grew without bound as
/// drag length increased even for a mathematically dead-on aim: anchor
/// (2,3,9), axis +X, eye (-8,-7,17) — an ordinary 3/4 orbit — read 0.09° at
/// a 0.05m drag, climbed past the 5° aperture by roughly 2.8m, and reached
/// 27° at 20m, silently dropping the soft axis mid-drag while the user kept
/// aiming exactly along it the whole time.
///
/// The `Q`-based angle used here is EXACT for a dead-on aim, at every drag
/// length and every camera position, not just near-overhead ones: when the
/// ray truly meets the axis, the closest point on each line IS that meeting
/// point, so `Q = P` and the perpendicular component of `Q - anchor` is
/// exactly zero — no reference-frame mixing is possible because there is
/// only one frame (the axis's own). `Q`'s own ray parameter (`s`) is
/// deliberately left UNCLAMPED (tool-parity delta-review finding: an
/// earlier revision clamped it against [`MAX_AXIS_REACH_FACTOR`] times the
/// eye-to-anchor distance, copying [`closest_point_on_line_to_ray`]'s
/// clamp — but that bound is the wrong scale here. There, `t` is a point
/// returned to the caller for use as a snap position, and near parallelism
/// makes that returned POINT swing without bound for a single pixel of
/// mouse motion, which is what the clamp guards. Here, `s` is never
/// returned; only the ANGLE derived from it is, and that angle is
/// well-conditioned regardless of how large `s` gets: `perp` (the
/// perpendicular distance from `Q` to the axis line) is exactly the
/// minimized point-to-line distance between the two lines, which stays
/// bounded as the lines approach parallel, so `perp.atan2(along.abs())`
/// does not blow up even when `s` itself does. Clamping `s` to a bound
/// scaled by eye-to-anchor distance instead reintroduced false rejection of
/// dead-on aims whenever drag length exceeded roughly `MAX_AXIS_REACH_FACTOR`
/// times that distance — an everyday "zoom in close, drag a long way along
/// the axis" interaction. The only guard against actual ill-conditioning
/// (division by a near-zero `denom`, i.e. exact or near-exact parallelism)
/// is the `denom.abs() < tol::NORMALIZE_MIN_LENGTH` check above; this call
/// site is additionally behind [`axis_is_edge_on`], which keeps `denom`
/// bounded well away from that floor in practice.
///
/// This is NOT a camera-proof measure, and cannot be made one from this
/// function's inputs alone: being exactly zero for a dead-on aim at every
/// drag length and every camera forces it (up to reparametrization) to be
/// equivalent to the true minimum distance between the ray and the axis as
/// infinite lines — i.e. to whether they are coplanar — and two skew lines
/// can be made coplanar (miss distance exactly zero, so this angle reads
/// exactly zero too) by an off-axis target whose direction from `anchor`
/// happens to lie in the plane spanned by `axis_dir` and the eye-to-anchor
/// vector, however far off-axis that target actually is. That configuration
/// is reachable from an ordinary, non-edge-on camera position — see
/// `soft_axis_deviation_can_still_be_fooled_by_a_near_coplanar_camera` for a
/// disclosed, worked example — so this is a known, accepted residual gap,
/// not a fixed one. Genuinely closing it would need real screen-space
/// information (the camera's actual projection), which this function does
/// not receive and which would be a wasm-api surface change.
fn soft_axis_deviation(
    anchor: Point3,
    axis_dir: Vec3,
    origin: Point3,
    dir: Vec3,
    soft_axis_aperture: f64,
) -> Option<f64> {
    let depth_anchor = dir.dot(anchor - origin);
    if depth_anchor <= 0.0 {
        return None; // anchor is behind the ray origin
    }
    // Same derivation as `closest_point_on_line_to_ray`, but solved for the
    // RAY's own parameter (`s`, the point `Q`) rather than the axis line's
    // (`t`, the point `P`) — see that function for the shared setup.
    let w = anchor - origin;
    let b = axis_dir.dot(dir);
    let denom = 1.0 - b * b;
    if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        return None; // (near-)parallel to the axis; axis_is_edge_on already guards this call site
    }
    let d = axis_dir.dot(w);
    let e = dir.dot(w);
    let s = (e - b * d) / denom;
    if s <= 0.0 {
        return None; // Q itself is behind the ray origin
    }
    let q = origin + dir * s;
    let offset = q - anchor;
    let along = offset.dot(axis_dir);
    let perp = (offset - axis_dir * along).length();
    let angle = perp.atan2(along.abs());
    (angle <= soft_axis_aperture).then_some(angle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::TestRunner;

    #[test]
    fn snap_kind_priority_is_declaration_order() {
        // Strongest first; tools sort by this, and it is `resolve`'s
        // within-rank-group tie-break. The coarse resolution order is
        // `SnapKind::rank_group`, which coarsens this without contradicting it
        // (see `rank_groups_coarsen_the_declaration_order_without_reordering_it`).
        assert!(SnapKind::Endpoint < SnapKind::Center);
        assert!(SnapKind::Center < SnapKind::Quadrant);
        // A crossing outranks a midpoint (midpoints are on every edge,
        // crossings are rare), but stays below the exact named points.
        assert!(SnapKind::Quadrant < SnapKind::Intersection);
        assert!(SnapKind::Intersection < SnapKind::Midpoint);
        assert!(SnapKind::Midpoint < SnapKind::Tangent);
        // OnGuide then OnAxis sit between Tangent and OnEdge, in that order
        // (tool-parity playtest-2 review finding D): a deliberately-placed
        // guide outranks an INFERRED soft-axis cue, which in turn still
        // outranks plain edge/face hovering (and, transitively, OnGuide
        // outranks edge/face too — see `rank_group`'s doc). Both still lose
        // to every exact point kind, Tangent included.
        assert!(SnapKind::Tangent < SnapKind::OnGuide);
        assert!(SnapKind::OnGuide < SnapKind::OnAxis);
        assert!(SnapKind::OnAxis < SnapKind::OnEdge);
        assert!(SnapKind::OnEdge < SnapKind::OnFace);
        assert!(SnapKind::OnFace < SnapKind::Parallel);
        assert!(SnapKind::Parallel < SnapKind::Perpendicular);
    }

    #[test]
    fn snap_kind_all_is_a_complete_bijection_onto_its_index_space() {
        assert_eq!(SnapKind::ALL.len(), SnapKind::COUNT);
        for (i, kind) in SnapKind::ALL.iter().enumerate() {
            assert_eq!(kind.index(), i, "{kind:?} is out of place in ALL");
        }
    }

    /// Rank groups may only coarsen the declaration order, never contradict
    /// it: a stronger kind can share a weaker one's group, but can never land
    /// in a later one. Otherwise `SnapKind`'s `Ord` — which tools rely on and
    /// which is still the within-group tie-break — would disagree with the
    /// order candidates actually resolve in.
    #[test]
    fn rank_groups_coarsen_the_declaration_order_without_reordering_it() {
        for a in SnapKind::ALL {
            for b in SnapKind::ALL {
                if a < b {
                    assert!(
                        a.rank_group() <= b.rank_group(),
                        "{a:?} is declared stronger than {b:?} but ranks lower"
                    );
                }
            }
        }
    }

    /// Exactly two groups hold more than one kind: the exact-named-point
    /// band, and the line band (a real edge and the virtual edge where two
    /// owners' faces meet). Gravity can reorder candidates ONLY inside a
    /// group, so this is precisely the blast radius of the whole feature —
    /// worth pinning so widening it is a deliberate act.
    #[test]
    fn only_the_exact_point_kinds_and_the_line_kinds_share_a_rank_group() {
        for kind in SnapKind::ALL {
            let peers: Vec<SnapKind> = SnapKind::ALL
                .into_iter()
                .filter(|k| k.rank_group() == kind.rank_group())
                .collect();
            let expected: Vec<SnapKind> = match kind {
                SnapKind::Endpoint | SnapKind::Center | SnapKind::Quadrant => {
                    vec![SnapKind::Endpoint, SnapKind::Center, SnapKind::Quadrant]
                }
                SnapKind::OnEdge | SnapKind::OnIntersectionLine => {
                    vec![SnapKind::OnEdge, SnapKind::OnIntersectionLine]
                }
                other => vec![other],
            };
            assert_eq!(peers, expected, "unexpected rank-group membership");
        }
    }

    #[test]
    fn uniform_weights_are_neutral_everywhere() {
        let w = SnapWeights::uniform();
        for kind in SnapKind::ALL {
            assert_eq!(w.weight(kind), GRAVITY_NEUTRAL);
        }
    }

    /// The shipped profile boosts the analytic curve points and nothing else
    /// — in particular nothing the spatial index prunes for, which is why it
    /// leaves the prune cone untouched.
    #[test]
    fn the_standard_profile_boosts_only_the_analytic_points() {
        let w = SnapWeights::default();
        assert_eq!(w, SnapWeights::standard());
        for kind in SnapKind::ALL {
            let expected = match kind {
                SnapKind::Center | SnapKind::Quadrant => GRAVITY_ANALYTIC_POINT,
                _ => GRAVITY_NEUTRAL,
            };
            assert_eq!(w.weight(kind), expected, "{kind:?}");
        }
        assert_eq!(w.max_indexed(1.0), POINT_SNAP_REACH);
    }

    #[test]
    fn max_indexed_tracks_indexed_kinds_and_ignores_linear_walked_ones() {
        let base = SnapWeights::default();
        for kind in [
            SnapKind::Endpoint,
            SnapKind::Midpoint,
            SnapKind::OnEdge,
            SnapKind::Intersection,
        ] {
            assert_eq!(base.with(kind, 4.0).max_indexed(1.0), 4.0, "{kind:?}");
        }
        for kind in [
            SnapKind::Center,
            SnapKind::Quadrant,
            SnapKind::Tangent,
            SnapKind::OnFace,
            SnapKind::OnGuide,
            SnapKind::OnAxis,
            SnapKind::Parallel,
            SnapKind::Perpendicular,
        ] {
            assert_eq!(
                base.with(kind, GRAVITY_MAX).max_indexed(1.0),
                POINT_SNAP_REACH,
                "{kind:?} is never indexed, so it must not widen the prune"
            );
        }
    }

    /// Weights are a ranking parameter with no error channel, so out-of-range
    /// values are clamped rather than refused (see `SnapWeights`'s docs).
    #[test]
    fn out_of_range_weights_are_clamped_not_refused() {
        let base = SnapWeights::uniform();
        assert_eq!(
            base.with(SnapKind::Center, 1e9).weight(SnapKind::Center),
            GRAVITY_MAX
        );
        assert_eq!(
            base.with(SnapKind::Center, 0.0).weight(SnapKind::Center),
            GRAVITY_NEUTRAL
        );
        assert_eq!(
            base.with(SnapKind::Center, -3.0).weight(SnapKind::Center),
            GRAVITY_NEUTRAL
        );
        assert_eq!(
            base.with(SnapKind::Center, f64::NAN)
                .weight(SnapKind::Center),
            GRAVITY_NEUTRAL
        );
        assert_eq!(
            base.with(SnapKind::Center, f64::INFINITY)
                .weight(SnapKind::Center),
            GRAVITY_NEUTRAL
        );
    }

    #[test]
    fn axes_are_unit_and_orthogonal() {
        let (x, y, z) = (Axis::X.unit(), Axis::Y.unit(), Axis::Z.unit());
        assert_eq!(x.dot(y), 0.0);
        assert_eq!(y.dot(z), 0.0);
        assert_eq!(x.cross(y), z);
    }

    #[test]
    fn empty_scene_has_no_candidates() {
        assert_eq!(InferenceScene::new().candidate_counts(), (0, 0, 0));
    }

    #[test]
    fn degenerate_ray_direction_resolves_to_none() {
        // A zero-length ray direction has no axis; resolve must return None,
        // never panic in normalize.
        let scene = InferenceScene::new();
        let query = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::ORIGIN,
                direction: Vec3::ZERO,
            },
            anchor: None,
            lock: None,
            aperture: 0.3,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };
        assert!(scene.resolve(&query).is_none());
    }

    #[test]
    fn segment_crossing_finds_only_true_interior_crossings() {
        let p = Point3::new;
        // A true crossing, off both midpoints, in a tilted frame.
        let x = segment_crossing(
            p(0.0, 0.0, 0.0),
            p(2.0, 2.0, 2.0),
            p(0.5, 0.5, -1.0),
            p(0.5, 0.5, 3.0),
        )
        .expect("crosses at (0.5, 0.5, 0.5)");
        assert!(x.approx_eq(p(0.5, 0.5, 0.5), tol::POINT_MERGE), "{x:?}");
        // Skew by 1e-6: no crossing.
        assert_eq!(
            segment_crossing(
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(0.5, 1e-6, -1.0),
                p(0.5, 1e-6, 1.0),
            ),
            None
        );
        // The lines cross, but off the second segment.
        assert_eq!(
            segment_crossing(
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(0.5, 0.0, 0.5),
                p(0.5, 0.0, 1.0),
            ),
            None
        );
        // A T: the second segment ENDS on the first one's interior.
        assert_eq!(
            segment_crossing(
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(0.5, 0.0, 0.0),
                p(0.5, 0.0, 1.0),
            ),
            None
        );
        // Two edges sharing an endpoint.
        assert_eq!(
            segment_crossing(
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(2.0, 1.0, 0.0),
            ),
            None
        );
        // Collinear and overlapping: no single crossing.
        assert_eq!(
            segment_crossing(
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(3.0, 0.0, 0.0),
            ),
            None
        );
        // Degenerate.
        assert_eq!(
            segment_crossing(
                p(0.0, 0.0, 0.0),
                p(0.0, 0.0, 0.0),
                p(-1.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
            ),
            None
        );
    }

    proptest! {
        /// Two segments built to pass through a common interior point `p`
        /// cross there, in either argument order; nudged apart along their
        /// common normal by 1e-6 they do not.
        #[test]
        fn segment_crossing_recovers_a_built_crossing(
            p in (-5.0f64..5.0, -5.0f64..5.0, -5.0f64..5.0),
            u in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            v in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            s in 0.05f64..0.95,
            t in 0.05f64..0.95,
            len_a in 0.1f64..4.0,
            len_b in 0.1f64..4.0,
        ) {
            let p = Point3::new(p.0, p.1, p.2);
            let u = Vec3::new(u.0, u.1, u.2);
            let v = Vec3::new(v.0, v.1, v.2);
            let n = u.cross(v);
            prop_assume!(u.length() > 0.1 && v.length() > 0.1);
            // Well away from parallel.
            prop_assume!(n.length() > 0.1 * u.length() * v.length());
            let (u, v) = (u.normalized().unwrap(), v.normalized().unwrap());
            let a0 = p - u * (s * len_a);
            let a1 = p + u * ((1.0 - s) * len_a);
            let b0 = p - v * (t * len_b);
            let b1 = p + v * ((1.0 - t) * len_b);
            let x = segment_crossing(a0, a1, b0, b1);
            prop_assert!(x.is_some_and(|x| x.approx_eq(p, 1e-9)), "{:?}", x);
            let y = segment_crossing(b0, b1, a0, a1);
            prop_assert!(y.is_some_and(|y| y.approx_eq(p, 1e-9)), "{:?}", y);
            let off = n.normalized().unwrap() * 1e-6;
            prop_assert_eq!(segment_crossing(a0, a1, b0 + off, b1 + off), None);
        }
    }

    #[test]
    fn cross_owner_pairs_counts_only_pairs_across_owners() {
        let edge = |owner| CrossingEdge {
            a: Point3::new(0.0, 0.0, 0.0),
            b: Point3::new(1.0, 0.0, 0.0),
            ang: 0.0,
            owner,
            edge: CrossedEdge::Sketch(SketchId::default(), SketchEdgeId::default()),
        };
        let w = EdgeOwner::World(ObjectId::default());
        let pool = [
            edge(w),
            edge(EdgeOwner::Placement(0)),
            edge(w),
            edge(EdgeOwner::Placement(1)),
            edge(EdgeOwner::Sketch(SketchId::default())),
        ];
        // 5 edges = 10 pairs, less the one pair inside the World group.
        assert_eq!(cross_owner_pairs(&pool, |e| e.owner), 9);
        assert_eq!(cross_owner_pairs(&pool[..1], |e| e.owner), 0);
        assert_eq!(cross_owner_pairs(&[edge(w), edge(w)], |e| e.owner), 0);
    }

    /// A pool edge owned by `owner`, its index in `ang` so a visit names it.
    fn walk_edge(owner: EdgeOwner, i: usize) -> CrossingEdge {
        CrossingEdge {
            a: Point3::new(0.0, 0.0, 0.0),
            b: Point3::new(1.0, 0.0, 0.0),
            ang: i as f64,
            owner,
            edge: CrossedEdge::Sketch(SketchId::default(), SketchEdgeId::default()),
        }
    }

    /// Every pair the walk visits, as pool indices, with its result.
    fn walk_pairs(pool: &[CrossingEdge], cap: usize) -> (Vec<(usize, usize)>, PairWalk) {
        let mut seen = Vec::new();
        let walk = walk_cross_owner_pairs(
            pool,
            |e| e.owner,
            cap,
            |e, f| seen.push((e.ang as usize, f.ang as usize)),
        );
        (seen, walk)
    }

    /// The unit square in the plane z = 0, as a face.
    fn unit_square_face() -> (Plane, Vec<Point3>) {
        let boundary = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        (Plane::from_polygon(&boundary).unwrap(), boundary)
    }

    #[test]
    fn segment_pierces_face_finds_only_true_piercings() {
        let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        let (plane, square) = unit_square_face();
        let no_holes: &[Vec<Point3>] = &[];
        // Straight through the middle.
        let hit = segment_pierces_face(
            p(0.3, 0.4, -1.0),
            p(0.3, 0.4, 2.0),
            &plane,
            &square,
            no_holes,
        );
        assert!(hit.is_some_and(|q| q.approx_eq(p(0.3, 0.4, 0.0), tol::POINT_MERGE)));
        // Slanted through the middle: the crossing is where z = 0.
        let hit = segment_pierces_face(
            p(0.0, 0.5, -1.0),
            p(1.0, 0.5, 1.0),
            &plane,
            &square,
            no_holes,
        );
        assert!(hit.is_some_and(|q| q.approx_eq(p(0.5, 0.5, 0.0), tol::POINT_MERGE)));
        // Through the plane but beside the face.
        assert_eq!(
            segment_pierces_face(
                p(1.5, 0.5, -1.0),
                p(1.5, 0.5, 1.0),
                &plane,
                &square,
                no_holes
            ),
            None
        );
        // Through a hole in the face.
        let hole = vec![
            p(0.25, 0.25, 0.0),
            p(0.75, 0.25, 0.0),
            p(0.75, 0.75, 0.0),
            p(0.25, 0.75, 0.0),
        ];
        assert_eq!(
            segment_pierces_face(
                p(0.5, 0.5, -1.0),
                p(0.5, 0.5, 1.0),
                &plane,
                &square,
                std::slice::from_ref(&hole)
            ),
            None
        );
        // Beside the hole, still on the face: a hit.
        assert!(
            segment_pierces_face(
                p(0.1, 0.5, -1.0),
                p(0.1, 0.5, 1.0),
                &plane,
                &square,
                &[hole]
            )
            .is_some()
        );
        // Ending on the face (a T): the Endpoint snap's, not a piercing.
        assert_eq!(
            segment_pierces_face(
                p(0.5, 0.5, -1.0),
                p(0.5, 0.5, 0.0),
                &plane,
                &square,
                no_holes
            ),
            None
        );
        assert_eq!(
            segment_pierces_face(
                p(0.5, 0.5, 0.0),
                p(0.5, 0.5, 1.0),
                &plane,
                &square,
                no_holes
            ),
            None
        );
        // Lying in the plane: no single point.
        assert_eq!(
            segment_pierces_face(
                p(-1.0, 0.5, 0.0),
                p(2.0, 0.5, 0.0),
                &plane,
                &square,
                no_holes
            ),
            None
        );
        // Parallel to the plane, off it: nothing.
        assert_eq!(
            segment_pierces_face(
                p(-1.0, 0.5, 0.2),
                p(2.0, 0.5, 0.2),
                &plane,
                &square,
                no_holes
            ),
            None
        );
        // Both ends on one side.
        assert_eq!(
            segment_pierces_face(
                p(0.5, 0.5, 0.1),
                p(0.5, 0.5, 1.0),
                &plane,
                &square,
                no_holes
            ),
            None
        );
    }

    proptest! {
        /// A segment built to pass through an interior point `p` of a random
        /// planar quad pierces it there, in either direction; slid along the
        /// plane normal until one endpoint sits on the plane it does not,
        /// and moved wholly to one side it does not.
        #[test]
        fn segment_pierces_face_recovers_a_built_piercing(
            c in (-5.0f64..5.0, -5.0f64..5.0, -5.0f64..5.0),
            u in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            v in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            d in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            su in -0.9f64..0.9,
            sv in -0.9f64..0.9,
            s in 0.05f64..0.95,
            len in 0.1f64..4.0,
        ) {
            let c = Point3::new(c.0, c.1, c.2);
            let u = Vec3::new(u.0, u.1, u.2);
            let v = Vec3::new(v.0, v.1, v.2);
            let d = Vec3::new(d.0, d.1, d.2);
            prop_assume!(u.length() > 0.1 && v.length() > 0.1 && d.length() > 0.1);
            let n = u.cross(v);
            prop_assume!(n.length() > 0.1 * u.length() * v.length());
            let n_unit = n.normalized().unwrap();
            // Well away from parallel to the plane.
            prop_assume!(d.normalized().unwrap().dot(n_unit).abs() > 0.1);
            // A parallelogram face around `c`, and a point inside it.
            let boundary = vec![c - u - v, c + u - v, c + u + v, c - u + v];
            let plane = Plane::from_polygon(&boundary).unwrap();
            let p = c + u * su + v * sv;
            let d = d.normalized().unwrap();
            let a = p - d * (s * len);
            let b = p + d * ((1.0 - s) * len);
            let hit = segment_pierces_face(a, b, &plane, &boundary, &[]);
            prop_assert!(hit.is_some_and(|q| q.approx_eq(p, 1e-9)), "{:?}", hit);
            let hit = segment_pierces_face(b, a, &plane, &boundary, &[]);
            prop_assert!(hit.is_some_and(|q| q.approx_eq(p, 1e-9)), "{:?}", hit);
            // Slid so `a` lands on the plane: a T meeting, not a piercing.
            let slide = d * (s * len);
            prop_assert_eq!(segment_pierces_face(a + slide, b + slide, &plane, &boundary, &[]), None);
            // Wholly on one side.
            let away = d * len;
            prop_assert_eq!(segment_pierces_face(a + away, b + away, &plane, &boundary, &[]), None);
            prop_assert_eq!(segment_pierces_face(a - away, b - away, &plane, &boundary, &[]), None);
        }
    }

    /// The meeting stretches of two faces given as (plane, outer loop, holes).
    fn meeting(
        a: (&Plane, &[Point3], &[Vec<Point3>]),
        b: (&Plane, &[Point3], &[Vec<Point3>]),
    ) -> Vec<(Point3, Point3)> {
        face_meeting_segments(a, b)
    }

    /// `(a, b)` is the segment `[p, q]` in either orientation.
    fn is_segment(seg: (Point3, Point3), p: Point3, q: Point3, eps: f64) -> bool {
        (seg.0.approx_eq(p, eps) && seg.1.approx_eq(q, eps))
            || (seg.0.approx_eq(q, eps) && seg.1.approx_eq(p, eps))
    }

    /// Two crossing quads meet along one stretch clipped to both; beyond
    /// either face's edge there is no line, parallel faces make none, and
    /// a hole in one face cuts the stretch in two.
    #[test]
    fn face_meeting_segments_clips_the_line_to_both_faces() {
        let p = Point3::new;
        // The wall's near face: x = 1, y 0..2, z 0..1.
        let wall = vec![
            p(1.0, 0.0, 0.0),
            p(1.0, 2.0, 0.0),
            p(1.0, 2.0, 1.0),
            p(1.0, 0.0, 1.0),
        ];
        let wall_plane = Plane::from_polygon(&wall).unwrap();
        // A slab's top: z = 0.5, x 0.5..1.2, y 0.6..1.4 — through the wall.
        let top = vec![
            p(0.5, 0.6, 0.5),
            p(1.2, 0.6, 0.5),
            p(1.2, 1.4, 0.5),
            p(0.5, 1.4, 0.5),
        ];
        let top_plane = Plane::from_polygon(&top).unwrap();
        let segs = meeting((&wall_plane, &wall, &[]), (&top_plane, &top, &[]));
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(
            is_segment(segs[0], p(1.0, 0.6, 0.5), p(1.0, 1.4, 0.5), 1e-9),
            "{segs:?}"
        );
        // The same stretch with the faces swapped.
        let swapped = meeting((&top_plane, &top, &[]), (&wall_plane, &wall, &[]));
        assert_eq!(swapped.len(), 1);
        assert!(is_segment(
            swapped[0],
            p(1.0, 0.6, 0.5),
            p(1.0, 1.4, 0.5),
            1e-9
        ));

        // The slab lifted above the wall: its plane crosses the wall's, the
        // wall's face ends below — no stretch.
        let high: Vec<Point3> = top.iter().map(|&q| q + Vec3::new(0.0, 0.0, 0.7)).collect();
        let high_plane = Plane::from_polygon(&high).unwrap();
        assert!(meeting((&wall_plane, &wall, &[]), (&high_plane, &high, &[])).is_empty());
        // Stopping short of the wall: the planes still meet, the slab's
        // face is not there.
        let short: Vec<Point3> = top.iter().map(|&q| q - Vec3::new(0.3, 0.0, 0.0)).collect();
        let short_plane = Plane::from_polygon(&short).unwrap();
        assert!(meeting((&wall_plane, &wall, &[]), (&short_plane, &short, &[])).is_empty());
        // Parallel: a second wall behind the first; coplanar: the first
        // wall against itself.
        let behind: Vec<Point3> = wall.iter().map(|&q| q + Vec3::new(0.3, 0.0, 0.0)).collect();
        let behind_plane = Plane::from_polygon(&behind).unwrap();
        assert!(meeting((&wall_plane, &wall, &[]), (&behind_plane, &behind, &[])).is_empty());
        assert!(meeting((&wall_plane, &wall, &[]), (&wall_plane, &wall, &[])).is_empty());

        // A window in the wall straddling the line, y 0.9..1.1, z 0.4..0.7:
        // two stretches, one each side of it.
        let window = vec![vec![
            p(1.0, 0.9, 0.4),
            p(1.0, 0.9, 0.7),
            p(1.0, 1.1, 0.7),
            p(1.0, 1.1, 0.4),
        ]];
        let segs = meeting((&wall_plane, &wall, &window), (&top_plane, &top, &[]));
        assert_eq!(segs.len(), 2, "{segs:?}");
        // In order along the line, whichever way the normals' cross product
        // happens to point.
        let (low, high) = if segs[0].0.y < segs[1].0.y {
            (segs[0], segs[1])
        } else {
            (segs[1], segs[0])
        };
        assert!(
            is_segment(low, p(1.0, 0.6, 0.5), p(1.0, 0.9, 0.5), 1e-9),
            "{segs:?}"
        );
        assert!(
            is_segment(high, p(1.0, 1.1, 0.5), p(1.0, 1.4, 0.5), 1e-9),
            "{segs:?}"
        );
        // A window the slab passes wholly through: no stretch at all.
        let wide = vec![vec![
            p(1.0, 0.5, 0.4),
            p(1.0, 0.5, 0.7),
            p(1.0, 1.5, 0.7),
            p(1.0, 1.5, 0.4),
        ]];
        assert!(meeting((&wall_plane, &wall, &wide), (&top_plane, &top, &[])).is_empty());
    }

    /// Two faces a hair from parallel — the stripe either makes on the other
    /// is nowhere in particular — yield nothing, right down to the gate.
    #[test]
    fn nearly_parallel_faces_make_no_meeting_line() {
        let p = Point3::new;
        let floor = vec![
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(0.0, 2.0, 0.0),
        ];
        let floor_plane = Plane::from_polygon(&floor).unwrap();
        // A plate tilted by `sine` about the x axis, crossing the floor along
        // y = 1.
        let tilted = |sine: f64| {
            let n = Vec3::new(0.0, -sine, (1.0 - sine * sine).sqrt());
            let v = Vec3::new(0.0, n.z, -n.y); // in-plane, across the line
            let c = p(1.0, 1.0, 0.0);
            vec![
                c - v + Vec3::new(-1.0, 0.0, 0.0),
                c - v + Vec3::new(1.0, 0.0, 0.0),
                c + v + Vec3::new(1.0, 0.0, 0.0),
                c + v + Vec3::new(-1.0, 0.0, 0.0),
            ]
        };
        let graze = tilted(tol::FACE_MEETING_PARALLEL_SINE * 0.5);
        let graze_plane = Plane::from_polygon(&graze).unwrap();
        assert!(meeting((&floor_plane, &floor, &[]), (&graze_plane, &graze, &[])).is_empty());
        let steep = tilted(tol::FACE_MEETING_PARALLEL_SINE * 4.0);
        let steep_plane = Plane::from_polygon(&steep).unwrap();
        let segs = meeting((&floor_plane, &floor, &[]), (&steep_plane, &steep, &[]));
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert!(
            is_segment(segs[0], p(0.0, 1.0, 0.0), p(2.0, 1.0, 0.0), 1e-6),
            "{segs:?}"
        );
    }

    proptest! {
        /// A quad built through a random chord `[p, q]` of a random planar
        /// quad meets it along exactly that chord, whichever face comes
        /// first; moved wholly to one side of the first face it meets
        /// nothing, and made parallel to it, nothing.
        #[test]
        fn face_meeting_segments_recovers_a_built_meeting(
            c in (-5.0f64..5.0, -5.0f64..5.0, -5.0f64..5.0),
            u in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            v in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            w in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            (pu, pv) in (-0.9f64..0.9, -0.9f64..0.9),
            (qu, qv) in (-0.9f64..0.9, -0.9f64..0.9),
            half in 0.05f64..2.0,
        ) {
            let c = Point3::new(c.0, c.1, c.2);
            let u = Vec3::new(u.0, u.1, u.2);
            let v = Vec3::new(v.0, v.1, v.2);
            let w = Vec3::new(w.0, w.1, w.2);
            prop_assume!(u.length() > 0.1 && v.length() > 0.1 && w.length() > 0.1);
            let n = u.cross(v);
            prop_assume!(n.length() > 0.1 * u.length() * v.length());
            let n_unit = n.normalized().unwrap();
            // The second face's other direction, well off the first plane.
            prop_assume!(w.normalized().unwrap().dot(n_unit).abs() > 0.1);
            // The first face: a parallelogram around `c`. The chord `[p, q]`
            // joins two interior points, well apart.
            let a_loop = vec![c - u - v, c + u - v, c + u + v, c - u + v];
            let a_plane = Plane::from_polygon(&a_loop).unwrap();
            let p = c + u * pu + v * pv;
            let q = c + u * qu + v * qv;
            prop_assume!((q - p).length() > 0.05);
            // The second face: the parallelogram spanned by the chord and
            // `w`, centered on the chord so it crosses the first plane there.
            let b_loop = vec![p - w * half, q - w * half, q + w * half, p + w * half];
            let b_plane = Plane::from_polygon(&b_loop).unwrap();
            let segs = face_meeting_segments((&a_plane, &a_loop, &[]), (&b_plane, &b_loop, &[]));
            prop_assert_eq!(segs.len(), 1, "{:?}", segs);
            prop_assert!(is_segment(segs[0], p, q, 1e-7), "{:?} vs {:?}-{:?}", segs, p, q);
            let swapped = face_meeting_segments((&b_plane, &b_loop, &[]), (&a_plane, &a_loop, &[]));
            prop_assert_eq!(swapped.len(), 1, "{:?}", swapped);
            prop_assert!(is_segment(swapped[0], p, q, 1e-7), "{:?}", swapped);
            // Wholly to one side of the first plane: no meeting.
            let away = w * (half * 1.5);
            let moved: Vec<Point3> = b_loop.iter().map(|&x| x + away).collect();
            let moved_plane = Plane::from_polygon(&moved).unwrap();
            prop_assert!(face_meeting_segments((&a_plane, &a_loop, &[]), (&moved_plane, &moved, &[])).is_empty());
            // Parallel to it: none either.
            let lifted: Vec<Point3> = a_loop.iter().map(|&x| x + n_unit * half).collect();
            let lifted_plane = Plane::from_polygon(&lifted).unwrap();
            prop_assert!(face_meeting_segments((&a_plane, &a_loop, &[]), (&lifted_plane, &lifted, &[])).is_empty());
        }
    }

    /// A face built in local coordinates, then turned and moved by `pose`:
    /// its world loop and the plane fitted to it, as a scene face's would be.
    fn posed(pose: &Transform, local: &[Point3]) -> (Plane, Vec<Point3>) {
        let ring: Vec<Point3> = local.iter().map(|&q| pose.apply_point(q)).collect();
        (Plane::from_polygon(&ring).unwrap(), ring)
    }

    /// The wall's near face (x = 1, y 0..2, z 0..1) and, at z = 0.5, a
    /// face on the far side of it given by its (x, y) corners.
    fn wall_and_top(pose: &Transform, top: &[(f64, f64)]) -> [(Plane, Vec<Point3>); 2] {
        let p = Point3::new;
        let wall = [
            p(1.0, 0.0, 0.0),
            p(1.0, 2.0, 0.0),
            p(1.0, 2.0, 1.0),
            p(1.0, 0.0, 1.0),
        ];
        let top: Vec<Point3> = top.iter().map(|&(x, y)| p(x, y, 0.5)).collect();
        [posed(pose, &wall), posed(pose, &top)]
    }

    /// Stretches of a meeting line, as `face_meeting_segments` returns them.
    type Stretches = Vec<(Point3, Point3)>;

    /// The meeting stretches of two posed faces without holes, both orders.
    fn meeting_both_ways([(wp, w), (tp, t)]: &[(Plane, Vec<Point3>); 2]) -> (Stretches, Stretches) {
        (
            face_meeting_segments((wp, w, &[]), (tp, t, &[])),
            face_meeting_segments((tp, t, &[]), (wp, w, &[])),
        )
    }

    proptest! {
        /// A face whose own edge lies along the line, and whose interior
        /// does not reach it — a box flush against a wall, its top meeting
        /// the wall along its back edge — meets the other face along
        /// nothing, at every rotation and placement: each vertex of that
        /// edge is ON the line within `tol::PLANE_DIST`, never on whichever
        /// side rounding puts it. (The strict side test this replaced
        /// returned the full edge for some rotations, a truncated stretch
        /// for others, and nothing for the rest — and the truncated
        /// stretch's middle was a Midpoint at no middle of anything.) The
        /// real edge there is the line, and it snaps as itself.
        ///
        /// A face that crosses the line still yields exactly its chord at
        /// every rotation, and so does a concave face whose boundary runs
        /// along the line for part of the way: an L whose inner edge lies
        /// on the line yields the stretch through its interior, not the
        /// stretch along that edge.
        #[test]
        fn a_meeting_along_a_faces_own_edge_is_decided_the_same_at_every_rotation(
            axis in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            angle in -3.2f64..3.2,
            at in (-50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0),
        ) {
            let axis = Vec3::new(axis.0, axis.1, axis.2);
            prop_assume!(axis.length() > 0.1);
            let pose = Transform::rotation(axis, angle)
                .unwrap()
                .then(&Transform::translation(Vec3::new(at.0, at.1, at.2)));
            let on = |y: f64| pose.apply_point(Point3::new(1.0, y, 0.5));

            // Flush: the top's back edge (1, 0.6..1.4) lies in the wall.
            let flush = wall_and_top(&pose, &[(1.0, 0.6), (2.0, 0.6), (2.0, 1.4), (1.0, 1.4)]);
            let (one, other) = meeting_both_ways(&flush);
            prop_assert!(one.is_empty() && other.is_empty(), "{:?} / {:?}", one, other);

            // Crossing: the top runs from in front of the wall to behind it.
            let through = wall_and_top(&pose, &[(0.5, 0.6), (2.0, 0.6), (2.0, 1.4), (0.5, 1.4)]);
            let (one, other) = meeting_both_ways(&through);
            for segs in [&one, &other] {
                prop_assert_eq!(segs.len(), 1, "{:?}", segs);
                prop_assert!(is_segment(segs[0], on(0.6), on(1.4), 1e-9), "{:?}", segs);
            }

            // An L through the wall whose inner edge (1, 1.0..1.4) lies on
            // the line: only y 0.6..1.0 is inside it.
            let ell = wall_and_top(
                &pose,
                &[(0.5, 0.6), (2.0, 0.6), (2.0, 1.0), (1.0, 1.0), (1.0, 1.4), (0.5, 1.4)],
            );
            let (one, other) = meeting_both_ways(&ell);
            for segs in [&one, &other] {
                prop_assert_eq!(segs.len(), 1, "{:?}", segs);
                prop_assert!(is_segment(segs[0], on(0.6), on(1.0), 1e-9), "{:?}", segs);
            }
        }

        /// The stretches are exactly where the line lies inside both faces,
        /// for concave faces with holes in any two planes: sampled along
        /// the line, a point is in a stretch exactly when it is inside each
        /// face's outer loop and outside its holes (`point_in_polygon`, the
        /// On Face test), except within a hair of a stretch's end. Every
        /// stretch lies on both planes, and the order the faces come in
        /// changes nothing.
        #[test]
        fn face_meeting_segments_are_the_line_inside_both_faces(
            a in face_with_hole(),
            b in face_with_hole(),
        ) {
            let (a_plane, a_outer, a_holes) = a;
            let (b_plane, b_outer, b_holes) = b;
            let Some((p, d)) = planes_meeting_line(&a_plane, &b_plane, &a_outer) else {
                return Ok(()); // parallel: nothing to sample
            };
            let segs = face_meeting_segments(
                (&a_plane, &a_outer, &a_holes),
                (&b_plane, &b_outer, &b_holes),
            );
            let swapped = face_meeting_segments(
                (&b_plane, &b_outer, &b_holes),
                (&a_plane, &a_outer, &a_holes),
            );
            // The same stretches; swapping the faces turns the line around,
            // so they come in the opposite order.
            prop_assert_eq!(segs.len(), swapped.len(), "{:?} / {:?}", segs, swapped);
            for (s, t) in segs.iter().zip(swapped.iter().rev()) {
                prop_assert!(is_segment(*s, t.0, t.1, 1e-7), "{:?} / {:?}", segs, swapped);
            }
            let spans: Vec<(f64, f64)> = segs
                .iter()
                .map(|&(s, e)| {
                    let (u, v) = ((s - p).dot(d), (e - p).dot(d));
                    (u.min(v), u.max(v))
                })
                .collect();
            for &(s, e) in &segs {
                for end in [s, e] {
                    prop_assert!(a_plane.signed_distance(end).abs() < 1e-7, "{:?}", end);
                    prop_assert!(b_plane.signed_distance(end).abs() < 1e-7, "{:?}", end);
                }
            }
            let inside = |plane: &Plane, outer: &[Point3], holes: &[Vec<Point3>], x: Point3| {
                let n = plane.normal();
                point_in_polygon(x, outer, n) && !holes.iter().any(|h| point_in_polygon(x, h, n))
            };
            let (lo, hi) = a_outer
                .iter()
                .chain(&b_outer)
                .map(|&q| (q - p).dot(d))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), t| (l.min(t), h.max(t)));
            let samples = 400;
            for k in 0..=samples {
                let t = lo + (hi - lo) * (k as f64) / (samples as f64);
                if spans.iter().any(|&(u, v)| (t - u).abs() < 1e-6 || (t - v).abs() < 1e-6) {
                    continue; // a stretch's end: either answer is right
                }
                let x = p + d * t;
                let expected = inside(&a_plane, &a_outer, &a_holes, x)
                    && inside(&b_plane, &b_outer, &b_holes, x);
                let got = spans.iter().any(|&(u, v)| u <= t && t <= v);
                prop_assert_eq!(got, expected, "at {} of {:?}: {:?}", t, (lo, hi), segs);
            }
        }
    }

    proptest! {
        /// The meeting walk's cheap reject only skips work: when the line,
        /// clipped to both faces' boxes, does not reach the ray, no point
        /// of any stretch — its middle included — passes within the reach,
        /// under either aperture mode.
        #[test]
        fn the_meeting_reject_never_drops_a_reachable_stretch(
            a in face_with_hole(),
            b in face_with_hole(),
            eye in (-4.0f64..4.0, -4.0f64..4.0, -4.0f64..4.0),
            aim in (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            reach in 0.001f64..0.3,
            cylinder in proptest::bool::ANY,
        ) {
            let (a_plane, a_outer, a_holes) = a;
            let (b_plane, b_outer, b_holes) = b;
            let origin = Point3::new(eye.0, eye.1, eye.2);
            let Ok(dir) = (Point3::new(aim.0, aim.1, aim.2) - origin).normalized() else {
                return Ok(());
            };
            let mode = if cylinder { ApertureMode::Cylinder } else { ApertureMode::Cone };
            let Some(line) = planes_meeting_line(&a_plane, &b_plane, &a_outer) else {
                return Ok(());
            };
            let reaches = meeting_line_reaches(
                line,
                region_aabb(&a_plane, &a_outer),
                region_aabb(&b_plane, &b_outer),
                origin,
                dir,
                reach,
                mode,
            );
            if !reaches {
                let segs = face_meeting_segments(
                    (&a_plane, &a_outer, &a_holes),
                    (&b_plane, &b_outer, &b_holes),
                );
                for &(s, e) in &segs {
                    prop_assert!(segment_cone_hit(origin, dir, s, e, reach, mode).is_none(), "{:?}", segs);
                    prop_assert!(cone_test(origin, dir, midpoint(s, e), reach, mode).is_none(), "{:?}", segs);
                }
            }
        }
    }

    /// A random concave face with a hole, in a random plane through a point
    /// near the origin: a star-shaped outer loop (5 to 8 vertices at radii
    /// 0.6 to 1 around its center) and, half the time, a star-shaped hole
    /// (3 to 6 vertices at radii 0.05 to 0.4), which then lies inside the
    /// outer loop's material whatever the draw.
    fn face_with_hole() -> impl Strategy<Value = (Plane, Vec<Point3>, Vec<Vec<Point3>>)> {
        (
            (-0.5f64..0.5, -0.5f64..0.5, -0.5f64..0.5),
            (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            proptest::collection::vec(0.6f64..1.0, 5..9),
            proptest::option::of(proptest::collection::vec(0.05f64..0.4, 3..7)),
            0.0f64..6.3,
        )
            .prop_filter_map("a usable normal", |(c, n, outer, hole, phase)| {
                let n = Vec3::new(n.0, n.1, n.2).normalized().ok()?;
                let (u, v) = plane_basis(n);
                let c = Point3::new(c.0, c.1, c.2);
                let star = |radii: &[f64]| -> Vec<Point3> {
                    radii
                        .iter()
                        .enumerate()
                        .map(|(i, &r)| {
                            let a = phase + std::f64::consts::TAU * i as f64 / radii.len() as f64;
                            c + u * (r * a.cos()) + v * (r * a.sin())
                        })
                        .collect()
                };
                let outer = star(&outer);
                let plane = Plane::from_polygon(&outer).ok()?;
                let holes = hole.map(|h| vec![star(&h)]).unwrap_or_default();
                Some((plane, outer, holes))
            })
    }

    /// A pool edge owned by `owner` at angular distance `ang`; its index
    /// rides in `a.x` so a visit can name it (the geometry itself is never
    /// read by the walk).
    fn walk_edge_at(owner: EdgeOwner, ang: f64, i: usize) -> CrossingEdge {
        CrossingEdge {
            a: Point3::new(i as f64, 0.0, 0.0),
            b: Point3::new(1.0, 0.0, 0.0),
            ang,
            owner,
            edge: CrossedEdge::Sketch(SketchId::default(), SketchEdgeId::default()),
        }
    }

    /// A pool face owned by `owner` at distance `key`; `holes.len()` is its
    /// index so a visit can name it (the geometry itself is never read by
    /// the walk).
    fn walk_face<'a>(
        owner: EdgeOwner,
        key: f64,
        i: usize,
        plane: &'a Plane,
        boundary: &'a [Point3],
        holes: &'a [Vec<Point3>],
    ) -> ConeFace<'a> {
        ConeFace {
            plane,
            boundary,
            holes: &holes[..i],
            owner,
            face: PiercedFace::SketchRegion(SketchId::default(), SketchRegionId::default()),
            surface_facet: false,
            key,
            depth: 0.0,
        }
    }

    #[test]
    fn the_pierce_walk_visits_foreign_pairs_nearest_first_within_the_cap() {
        let (plane, square) = unit_square_face();
        let holes: Vec<Vec<Point3>> = (0..5).map(|_| Vec::new()).collect();
        let w = EdgeOwner::World(ObjectId::default());
        let p0 = EdgeOwner::Placement(0);
        let p1 = EdgeOwner::Placement(1);
        // Edges nearest first: World at 0.5, Placement(0) at 1.5, World at 2.5.
        let edges = [
            walk_edge_at(w, 0.5, 0),
            walk_edge_at(p0, 1.5, 1),
            walk_edge_at(w, 2.5, 2),
        ];
        // Faces nearest first, owners mixed.
        let faces = [
            walk_face(p1, 0.0, 0, &plane, &square, &holes),
            walk_face(w, 1.0, 1, &plane, &square, &holes),
            walk_face(p0, 2.0, 2, &plane, &square, &holes),
            walk_face(w, 3.0, 3, &plane, &square, &holes),
            walk_face(p1, 10.0, 4, &plane, &square, &holes),
        ];
        let run = |cap: usize| {
            let mut seen = Vec::new();
            let walk = walk_pierce_pairs(&edges, &faces, cap, |e, f| {
                seen.push((e.a.x as usize, f.holes.len()))
            });
            (seen, walk)
        };
        // Uncapped: pairs in order of the farther of edge and face — every
        // cross-owner pair once, none of one owner.
        let (seen, walk) = run(usize::MAX);
        assert_eq!(
            seen,
            vec![
                (0, 0), // edge 0 (0.5) × face 0 (0.0)
                (1, 0), // edge 1 (1.5) × faces 0, 1
                (1, 1),
                (0, 2), // face 2 (2.0) × edge 0 (edge 1 is its owner)
                (2, 0), // edge 2 (2.5) × faces 0, 2 (face 1 is its owner)
                (2, 2),
                (1, 3), // face 3 (3.0) × edge 1 (edges 0, 2 are its owner)
                (0, 4), // face 4 (10.0) × every edge
                (1, 4),
                (2, 4),
            ]
        );
        assert_eq!((walk.tested, walk.skipped), (10, 0));
        // Capped at 4: the four nearest pairs, the rest counted.
        let (seen, walk) = run(4);
        assert_eq!(seen, vec![(0, 0), (1, 0), (1, 1), (0, 2)]);
        assert_eq!((walk.tested, walk.skipped), (4, 6));
    }

    #[test]
    fn the_pierce_walk_does_no_work_with_one_shared_owner() {
        let (plane, square) = unit_square_face();
        let holes: Vec<Vec<Point3>> = vec![Vec::new(), Vec::new()];
        let w = EdgeOwner::World(ObjectId::default());
        let edges = [walk_edge_at(w, 0.0, 0), walk_edge_at(w, 1.0, 1)];
        let faces = [
            walk_face(w, 0.0, 0, &plane, &square, &holes),
            walk_face(w, 1.0, 1, &plane, &square, &holes),
        ];
        let walk = walk_pierce_pairs(&edges, &faces, usize::MAX, |_, _| {
            panic!("one owner pairs nothing")
        });
        assert_eq!(
            walk,
            PairWalk {
                tested: 0,
                steps: 0,
                skipped: 0
            }
        );
        // No edges, or no faces: likewise.
        let walk = walk_pierce_pairs(&[], &faces, usize::MAX, |_, _| panic!("no edges"));
        assert_eq!(walk.tested, 0);
        let walk = walk_pierce_pairs(&edges, &[], usize::MAX, |_, _| panic!("no faces"));
        assert_eq!(walk.tested, 0);
    }

    /// Far from the origin, where every coordinate carries rounding of
    /// order 1e-11: a crossing at an ordinary angle is still exact to the
    /// scene tolerance, and a grazing edge — its ends a few nanometres
    /// either side of the plane — is refused rather than placed anywhere.
    #[test]
    fn segment_pierces_face_far_from_the_origin() {
        let far = 1e5;
        // A unit square tilted 37° off the ground (normal (0.6, 0, 0.8)),
        // 100 km out along x and y.
        let o = Point3::new(far, far, 0.0);
        let u = Vec3::new(0.8, 0.0, -0.6);
        let v = Vec3::new(0.0, 1.0, 0.0);
        let n = Vec3::new(0.6, 0.0, 0.8);
        let boundary = vec![o, o + u, o + u + v, o + v];
        let plane = Plane::from_polygon(&boundary).unwrap();
        let p = o + u * 0.3 + v * 0.4;
        // Through `p` at 45° to the plane.
        let d = (n + u).normalized().unwrap();
        let hit = segment_pierces_face(p - d * 0.7, p + d * 0.9, &plane, &boundary, &[]);
        assert!(
            hit.is_some_and(|q| q.approx_eq(p, tol::POINT_MERGE)),
            "{hit:?} vs {p:?}"
        );
        // Grazing: a metre-long edge in the plane to within 2 nm at each
        // end, on opposite sides. Past PLANE_DIST, so only the grazing gate
        // refuses it — here and at metre scale alike.
        let graze = |o: Point3| {
            segment_pierces_face(
                o + u * 0.0 + v * 0.5 + n * 2e-9,
                o + u * 1.0 + v * 0.5 - n * 2e-9,
                &Plane::from_polygon(&[o, o + u, o + u + v]).unwrap(),
                &[o, o + u, o + u + v, o + v],
                &[],
            )
        };
        assert_eq!(graze(o), None);
        assert_eq!(graze(Point3::new(0.0, 0.0, 0.0)), None);
        // Just past the grazing sine the crossing is offered, where it is.
        let steep = segment_pierces_face(
            o + v * 0.5 + n * 2e-6,
            o + u * 1.0 + v * 0.5 - n * 2e-6,
            &plane,
            &boundary,
            &[],
        );
        assert!(
            steep.is_some_and(|q| q.approx_eq(o + u * 0.5 + v * 0.5, 1e-5)),
            "{steep:?}"
        );
    }

    #[test]
    fn the_pair_walk_does_no_work_with_one_owner() {
        let w = EdgeOwner::World(ObjectId::default());
        let pool: Vec<_> = (0..5000).map(|i| walk_edge(w, i)).collect();
        let (seen, walk) = walk_pairs(&pool, CROSSING_PAIR_CAP);
        assert!(seen.is_empty());
        assert_eq!(
            walk,
            PairWalk {
                tested: 0,
                steps: 0,
                skipped: 0
            }
        );
        let (seen, walk) = walk_pairs(&[], CROSSING_PAIR_CAP);
        assert!(seen.is_empty());
        assert_eq!(walk.steps, 0);
    }

    /// Many edges of one owner and two foreign ones, wherever the foreign
    /// two fall in the order: the walk visits exactly the cross-owner
    /// pairs, in the order the all-pairs loop would test them, in at most
    /// twice the pairs plus the edges steps (the all-pairs loop would take
    /// about 125,000 here).
    #[test]
    fn the_pair_walk_visits_only_cross_owner_pairs_nearest_first() {
        let w = EdgeOwner::World(ObjectId::default());
        let (a, b) = (EdgeOwner::Placement(0), EdgeOwner::Placement(1));
        let n = 500;
        for (pa, pb) in [(0, 1), (n, n + 1), (0, n + 1), (n / 2, n / 3), (7, 8)] {
            let pool: Vec<_> = (0..n + 2)
                .map(|i| {
                    let owner = if i == pa {
                        a
                    } else if i == pb {
                        b
                    } else {
                        w
                    };
                    walk_edge(owner, i)
                })
                .collect();
            let reference: Vec<(usize, usize)> = (1..pool.len())
                .flat_map(|j| (0..j).map(move |i| (i, j)))
                .filter(|&(i, j)| pool[i].owner != pool[j].owner)
                .collect();
            let (seen, walk) = walk_pairs(&pool, usize::MAX);
            assert_eq!(seen, reference, "foreign at {pa}, {pb}");
            assert_eq!(walk.tested, cross_owner_pairs(&pool, |e| e.owner));
            assert_eq!(walk.tested, 2 * n + 1);
            assert!(
                walk.steps <= 2 * walk.tested + pool.len(),
                "{} steps for {} pairs",
                walk.steps,
                walk.tested
            );
            assert_eq!(walk.skipped, 0);

            // Under a cap, the walk is the same walk cut short: the first
            // pairs of it, never more steps than the cap.
            let (cut, walk) = walk_pairs(&pool, 300);
            assert_eq!(walk.steps, 300);
            assert_eq!(walk.skipped, reference.len() - cut.len());
            assert!(walk.skipped > 0);
            assert_eq!(cut, reference[..cut.len()], "foreign at {pa}, {pb}");
            assert_eq!(walk.tested, cut.len());
        }
    }

    /// Owners interleaved edge by edge (no run longer than one): every
    /// step is a pair or a single skip, and a cap that lands exactly on the
    /// last pair leaves nothing skipped.
    #[test]
    fn the_pair_walk_counts_every_step_and_only_caps_with_pairs_left() {
        let pool: Vec<_> = (0..40)
            .map(|i| walk_edge(EdgeOwner::Placement(i % 3), i))
            .collect();
        let (seen, full) = walk_pairs(&pool, usize::MAX);
        assert_eq!(seen.len(), cross_owner_pairs(&pool, |e| e.owner));
        let (_, exact) = walk_pairs(&pool, full.steps);
        assert_eq!(exact, full);
        assert_eq!(exact.skipped, 0);
        let (_, short) = walk_pairs(&pool, full.steps - 1);
        assert_eq!(short.steps, full.steps - 1);
        assert_eq!(short.skipped, full.tested - short.tested);
        assert_eq!(short.skipped, 1, "the last step of this walk is a pair");
    }

    #[test]
    fn cone_test_excludes_points_behind_the_ray() {
        let origin = Point3::new(0.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        // Directly ahead: included, angular distance 0.
        let ahead = cone_test(
            origin,
            dir,
            Point3::new(0.0, 0.0, 5.0),
            0.3,
            ApertureMode::Cone,
        );
        assert!(ahead.is_some());
        assert!(ahead.unwrap().0.abs() < tol::NORMAL_DIRECTION);
        // Directly behind: excluded regardless of how wide the cone is.
        assert!(
            cone_test(
                origin,
                dir,
                Point3::new(0.0, 0.0, -5.0),
                3.0,
                ApertureMode::Cone
            )
            .is_none()
        );
    }

    #[test]
    fn segment_closest_point_clamps_to_endpoints() {
        // Ray parallel to +z offset in x; the segment lies along x at z=5.
        // The closest point to the ray is beyond the segment's b end, so it
        // must clamp to b, not run off the line.
        let origin = Point3::new(10.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let hit = segment_cone_hit(
            origin,
            dir,
            Point3::new(0.0, 0.0, 5.0),
            Point3::new(2.0, 0.0, 5.0),
            3.0,
            ApertureMode::Cone,
        );
        let (pos, _ang, depth) = hit.expect("segment is in front and within the wide cone");
        assert!(pos.approx_eq(Point3::new(2.0, 0.0, 5.0), tol::POINT_MERGE));
        assert!((depth - 5.0).abs() < tol::POINT_MERGE);
    }

    /// Reviewer's exact-parallel counterexample: a segment collinear with
    /// the ray's own axis (offset 0.05 m to the side), straddling the ray
    /// origin's depth plane (a behind at z=-5, b ahead at z=5). The old
    /// code's parallel branch hard-coded `s=0`, which is exactly `a` —
    /// behind the origin — so it rejected a segment that is plainly
    /// admissible near its `b` end (and, for `Cylinder`, admissible at
    /// EVERY depth, since the 0.05 m offset is exactly constant along an
    /// exactly-parallel segment). Both `ApertureMode`s must find a hit.
    #[test]
    fn exact_parallel_segment_straddling_the_origin_still_hits() {
        let origin = Point3::new(0.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let a = Point3::new(0.05, 0.0, -5.0);
        let b = Point3::new(0.05, 0.0, 5.0);

        let cone_hit = segment_cone_hit(origin, dir, a, b, 0.1, ApertureMode::Cone)
            .expect("b end reads a tiny angle off-axis at depth 5 — well within a 0.1 rad cone");
        assert!(cone_hit.2 > 0.0, "depth must be in front of the ray origin");
        assert!(
            cone_hit.0.approx_eq(b, tol::POINT_MERGE),
            "Cone favors the farther (smaller-angle) admissible end"
        );

        let cyl_hit = segment_cone_hit(origin, dir, a, b, 0.1, ApertureMode::Cylinder)
            .expect("the 0.05 m offset is within a 0.1 m cylinder at every depth along an exactly-parallel segment");
        assert!(cyl_hit.2 > 0.0, "depth must be in front of the ray origin");
        assert!(
            cyl_hit.1 <= 0.1 + tol::POINT_MERGE,
            "perpendicular offset must respect the aperture"
        );
    }

    /// A NON-parallel ("general branch") segment straddling the ray origin's
    /// depth plane: the line-line closest point (the segment's true nearest
    /// approach to the ray's infinite line) lands at s=0.5 with depth -0.5 —
    /// behind the origin — even though `b` (s=1, depth 2) is clearly
    /// admissible. The pre-fix code handed `cone_test` that single
    /// unclamped-closest point and accepted its rejection, never trying
    /// anywhere else on the segment. This is the "may be pre-existing"
    /// Cone-mode half the review called out, reproduced without any
    /// near-parallel geometry at all.
    ///
    /// The two modes pick OPPOSITE ends of the admissible sub-interval here
    /// (see `segment_edge_fallback_hit`'s doc): `Cone`'s angle shrinks with
    /// depth for a roughly-fixed lateral offset, so it favors the far `b`
    /// end; `Cylinder`'s offset is depth-independent, so its ranking
    /// tie-break favors the near (smallest-admissible-depth) point close to
    /// the depth=0 crossing instead. Expected values cross-checked against a
    /// 200,001-sample linear scan in Python.
    #[test]
    fn general_branch_segment_straddling_the_origin_still_hits() {
        let origin = Point3::new(0.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let a = Point3::new(1.0, 0.0, -3.0);
        let b = Point3::new(-1.0, 0.0, 2.0);

        let (cone_pos, cone_ang, cone_depth) =
            segment_cone_hit(origin, dir, a, b, 0.5, ApertureMode::Cone)
                .expect("b end (angle ~0.4636 rad, depth 2) is within a 0.5 rad cone");
        assert!(cone_pos.approx_eq(b, 1e-4), "Cone lands at the far (b) end");
        assert!((cone_ang - 0.463_647_609_000_806_15).abs() < 1e-4);
        assert!((cone_depth - 2.0).abs() < 1e-4);

        let (cyl_pos, cyl_perp, cyl_depth) =
            segment_cone_hit(origin, dir, a, b, 1.5, ApertureMode::Cylinder)
                .expect("the near-crossing point (perp ~0.2, depth ~0) is within a 1.5 m cylinder");
        assert!(
            (cyl_perp - 0.2).abs() < 1e-3,
            "Cylinder lands near the depth=0 crossing, not at an endpoint"
        );
        assert!(
            cyl_depth > 0.0 && cyl_depth < 1e-3,
            "hugging the crossing from the admissible side"
        );
        assert!(
            !cyl_pos.approx_eq(b, 1e-3),
            "Cylinder's pick is NOT the same point Cone picked"
        );
    }

    /// A fine-grained oracle: scans `s` linearly across the segment and
    /// returns the best (per `cone_test`'s own ranking tuple) admissible
    /// sample — the ground truth `segment_cone_hit` (and its fallback) is
    /// checked against below, since a closed-form derivation would just
    /// re-litigate the same reasoning the implementation itself relies on.
    fn scan_oracle(
        origin: Point3,
        dir: Vec3,
        a: Point3,
        b: Point3,
        aperture: f64,
        mode: ApertureMode,
    ) -> Option<(f64, f64)> {
        const STEPS: u32 = 20_000;
        let mut best: Option<(f64, f64)> = None;
        for i in 0..=STEPS {
            let s = f64::from(i) / f64::from(STEPS);
            let point = a + (b - a) * s;
            if let Some((metric, depth)) = cone_test(origin, dir, point, aperture, mode)
                && best.is_none_or(|(m, d)| (metric, depth) < (m, d))
            {
                best = Some((metric, depth));
            }
        }
        best
    }

    /// Property test (DEVELOPMENT.md rule 3): for segments constructed to
    /// straddle the ray origin's depth plane — one endpoint behind it, one
    /// ahead, by construction — `segment_cone_hit` must (a) find a hit
    /// whenever the fine `scan_oracle` does, in BOTH `ApertureMode`s,
    /// directly targeting the false-rejection bug this whole fix addresses,
    /// and (b) never report a WORSE (larger-ranking) metric than the oracle
    /// found by more than a hair — the analytic/nudged point this function
    /// picks should equal or beat a 20,001-sample discrete scan, not trail
    /// meaningfully behind it.
    ///
    /// An earlier version of this test drew ONE lateral offset and reused it
    /// for both `a` and `b`, so `b - a` was exactly parallel to `dir` on
    /// every single case — the general (non-parallel closest-point) branch
    /// in `segment_cone_hit` never ran across the whole property run, and
    /// its depth guard was backed only by the one hand-written
    /// `general_branch_segment_straddling_the_origin_still_hits` example
    /// above. Fixed by drawing `lateral_a`/`lateral_b` INDEPENDENTLY, so a
    /// typical draw is genuinely non-parallel, while `force_parallel` still
    /// deliberately recreates the exact-parallel case some of the time so
    /// the near-/exact-parallel path through `segment_edge_fallback_hit`
    /// stays covered too. This is written against `TestRunner` directly
    /// (rather than the `proptest!` macro used elsewhere in this crate)
    /// because it needs to reset `SEGMENT_CONE_HIT_BRANCH_COUNTS` before the
    /// run and read it back after — the macro's generated `#[test]` fn has
    /// no seam for that setup/teardown around the many per-case calls.
    #[test]
    fn segment_cone_hit_matches_fine_scan_oracle() {
        SEGMENT_CONE_HIT_BRANCH_COUNTS.with(|c| c.set((0, 0)));

        let strategy = (
            (-1.0f64..1.0, -1.0f64..1.0, -1.0f64..1.0),
            -10.0f64..-0.01,
            0.01f64..10.0,
            prop::bool::weighted(0.15),
            (-0.5f64..0.5, -0.5f64..0.5),
            (-0.5f64..0.5, -0.5f64..0.5),
            0.05f64..1.4,
            0.05f64..2.0,
        );

        let mut runner = TestRunner::new(ProptestConfig::with_cases(6000));
        let result = runner.run(
            &strategy,
            |(
                dir_seed,
                depth_a,
                depth_b,
                force_parallel,
                lateral_a,
                lateral_b,
                aperture_cone,
                aperture_cyl,
            )| {
                let dir = match Vec3::new(dir_seed.0, dir_seed.1, dir_seed.2).normalized() {
                    Ok(d) => d,
                    Err(_) => return Ok(()), // degenerate draw; skip
                };
                let origin = Point3::ORIGIN;
                // An orthonormal (u, v) basis for the plane perpendicular to
                // `dir`, so `a`/`b` can each be placed at their OWN
                // independently-drawn lateral offset — genuinely
                // non-parallel in the typical draw — unless
                // `force_parallel` says to reuse `lateral_a` for `b` too,
                // recreating the exact-parallel case on purpose.
                let helper = if dir.x.abs() < 0.9 {
                    Vec3::new(1.0, 0.0, 0.0)
                } else {
                    Vec3::new(0.0, 1.0, 0.0)
                };
                let u = dir
                    .cross(helper)
                    .normalized()
                    .expect("helper never parallel to dir");
                let v = dir.cross(u);

                let lateral_b = if force_parallel { lateral_a } else { lateral_b };

                let a = origin + dir * depth_a + u * lateral_a.0 + v * lateral_a.1;
                let b = origin + dir * depth_b + u * lateral_b.0 + v * lateral_b.1;

                for (aperture, mode) in [
                    (aperture_cone, ApertureMode::Cone),
                    (aperture_cyl, ApertureMode::Cylinder),
                ] {
                    let oracle = scan_oracle(origin, dir, a, b, aperture, mode);
                    let actual = segment_cone_hit(origin, dir, a, b, aperture, mode);

                    if let Some((oracle_metric, _)) = oracle {
                        let (_, actual_metric, actual_depth) = actual.unwrap_or_else(|| {
                            panic!(
                                "oracle found an admissible point (metric {oracle_metric}) but \
                                 segment_cone_hit found none ({mode:?}, aperture {aperture})"
                            )
                        });
                        prop_assert!(actual_depth > 0.0);
                        prop_assert!(
                            actual_metric <= oracle_metric + 1e-6,
                            "segment_cone_hit's metric {actual_metric} beat the oracle's \
                             {oracle_metric} ({mode:?}, aperture {aperture}) — the oracle isn't \
                             actually the best"
                        );
                    }
                }
                Ok(())
            },
        );
        if let Err(err) = result {
            panic!("{err}");
        }

        // Structural proof of branch coverage, not hope: both of
        // `segment_cone_hit`'s return paths must have actually run a
        // nontrivial number of times (6000 cases x 2 `ApertureMode`s = up to
        // 12000 opportunities each, since branch selection is geometry-only
        // and identical across both modes for a given `a`/`b`/`dir`).
        let (general, fallback) = SEGMENT_CONE_HIT_BRANCH_COUNTS.with(|c| c.get());
        assert!(
            general > 1000,
            "general (non-parallel closest-point) branch ran only {general} times across 6000 \
             cases x 2 modes — independent lateral offsets should make it the common case"
        );
        assert!(
            fallback > 1000,
            "fallback (near-/exact-parallel, or behind-origin) branch ran only {fallback} times \
             across 6000 cases x 2 modes — force_parallel draws and depth-straddling geometry \
             should both reach it"
        );
    }

    #[test]
    fn projection_lands_exactly_on_the_locked_line() {
        // Project an arbitrary point onto the X axis through the origin:
        // the result must be exactly (x, 0, 0).
        let p = project_onto_line(
            Point3::ORIGIN,
            Vec3::new(1.0, 0.0, 0.0),
            Point3::new(3.7, 9.0, -4.0),
        );
        assert_eq!(p.y, 0.0);
        assert_eq!(p.z, 0.0);
        assert!((p.x - 3.7).abs() < tol::POINT_MERGE);
    }

    #[test]
    fn angular_tiebreak_prefers_the_candidate_nearer_the_axis() {
        // Two vertices of a unit cube sit in a wide cone; the one the ray
        // points more directly at must win the equal-kind tiebreak.
        let cube = kernel::Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap();
        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
        // Aim almost straight at the (1,1,1) corner from far along the
        // diagonal; (0,0,0) is also on the cone but much farther off-axis.
        let eye = Point3::new(4.0, 4.0, 4.0);
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: eye,
                    direction: Point3::new(1.0, 1.0, 1.0) - eye,
                },
                anchor: None,
                lock: None,
                aperture: 0.6,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("a corner is within the cone");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(
            snap.position
                .approx_eq(Point3::new(1.0, 1.0, 1.0), tol::POINT_MERGE)
        );
    }

    /// A unit cube as an inference scene, for the constraint-plane tests.
    fn cube_scene() -> InferenceScene {
        let cube = kernel::Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap();
        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
        scene
    }

    /// Drawing on the top face must not "see through" the solid. A ray aimed
    /// into the top face interior whose wide cone *also* catches the hidden
    /// bottom corner (an Endpoint, the strongest kind, nearer the axis than its
    /// top twin) used to dive to that off-plane corner when unconstrained —
    /// the rectangle-on-face abort bug. Two independent guards now prevent it:
    /// occlusion culling (the front face hides the bottom corner from the eye,
    /// so it can't win even unconstrained), AND the constraint plane (which
    /// additionally restricts candidates to the active drawing plane). Assert
    /// BOTH keep the snap on z = 1.
    #[test]
    fn constraint_plane_excludes_occluded_off_plane_snaps() {
        let scene = cube_scene();
        // Straight-down ray entering the top face interior at (0.3, 0.3); the
        // wide cone also catches the hidden bottom corner (0,0,0).
        let ray = PickRay {
            origin: Point3::new(0.3, 0.3, 4.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };

        // Unconstrained: occlusion culls the hidden bottom corner (it sits
        // behind the top face along the ray to it), so the snap stays at z = 1.
        let free = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray,
                anchor: None,
                lock: None,
                aperture: 0.6,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("something visible in the wide cone");
        assert!(
            free.position.z > 0.5,
            "occlusion must keep the unconstrained snap on the visible top, \
             not dive to the hidden bottom: {:?}",
            free.position
        );

        // Constrained to the top plane: independently keeps the snap on z = 1.
        let top =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let constrained = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray,
                anchor: None,
                lock: None,
                aperture: 0.6,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: Some(top),
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("an on-plane candidate (top face / its edges) remains");
        assert!(
            top.signed_distance(constrained.position).abs() <= tol::PLANE_DIST,
            "constrained snap lies on the active plane: {:?}",
            constrained.position
        );
    }

    /// Core occlusion regression (bug report,  era): hovering the
    /// centre of a solid's top face must snap to that visible face, NOT pass
    /// through to a hidden back-side edge/vertex. Mirrors push/pulling a circle
    /// into a faceted cylinder, then drawing/measuring on its top: the dense
    /// far-rim facet edges (`OnEdge`, which outranks `OnFace`) must be culled.
    #[test]
    fn unconstrained_snap_does_not_see_through_to_hidden_back_geometry() {
        let scene = cube_scene();
        // Straight down through the centre of the top face, with a tight cone so
        // only the face interior is in range (the corners are ~13° off-axis,
        // well outside). The bottom face's OnFace candidate (z=0) and the hidden
        // bottom geometry are all occluded by the top, so the visible top face
        // must win — proving the hover lands ON the face, not through it.
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(0.5, 0.5, 4.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("the visible top face is under the cursor");
        assert_eq!(
            snap.kind,
            SnapKind::OnFace,
            "centre-of-face hover must land on the visible face, got {:?} at {:?}",
            snap.kind,
            snap.position
        );
        assert!(
            (snap.position.z - 1.0).abs() <= tol::PLANE_DIST,
            "snap must sit on the visible top (z=1), not the hidden bottom: {:?}",
            snap.position
        );
    }

    /// Occlusion must not over-cull: a snap target that is genuinely *visible*
    /// (in front of, or beside, any face) still snaps. Here the top-front edge
    /// of the cube is unobstructed from a front-corner eye, so it wins as
    /// `OnEdge` even though deeper cube faces also fall in the cone.
    #[test]
    fn occlusion_keeps_visible_front_geometry() {
        let scene = cube_scene();
        // Eye straight out in front of the +X face, aimed at its centre. The
        // four corners of that face are visible Endpoints (~13° off-axis); the
        // back face's corners (x=0) are hidden behind the cube. Occlusion must
        // cull the back corners but keep a front corner — so the snap lands on
        // the visible front face (x≈1), never diving to the hidden back.
        let eye = Point3::new(4.0, 0.5, 0.5);
        let target = Point3::new(1.0, 0.5, 0.5);
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: eye,
                    direction: target - eye,
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("a visible +X-face corner is in the cone");
        assert_eq!(
            snap.kind,
            SnapKind::Endpoint,
            "a visible front corner must still snap, got {:?} at {:?}",
            snap.kind,
            snap.position
        );
        assert!(
            (snap.position.x - 1.0).abs() <= tol::POINT_MERGE,
            "snap must stay on the visible front face (x=1), not cull through to the back: {:?}",
            snap.position
        );
    }

    /// A ray through an imprinted hole must still reach the geometry visible
    /// *through* the opening — occlusion uses the same hole-aware ray-face test
    /// as `pick_face`, so a face does not occlude what shows through its hole.
    #[test]
    fn occlusion_ignores_geometry_seen_through_a_hole() {
        let mut cube = unit_cube();
        let top = cube
            .faces()
            .iter()
            .find(|(_, f)| {
                f.plane
                    .normal()
                    .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
            })
            .map(|(id, _)| id)
            .unwrap();
        // Imprint an inner square, then PULL it down would be ideal, but for a
        // pure-occlusion check we just rely on the annular parent having a hole:
        // a ray down the hole centre is NOT occluded by the parent top face, so
        // the sub-face (coplanar, at z=1) is the visible snap rather than being
        // hidden. (Regression guard that holes punch through occlusion.)
        cube.split_face_inner(
            top,
            &[
                Point3::new(0.25, 0.25, 1.0),
                Point3::new(0.75, 0.25, 1.0),
                Point3::new(0.75, 0.75, 1.0),
                Point3::new(0.25, 0.75, 1.0),
            ],
        )
        .unwrap();
        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);

        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(0.5, 0.5, 4.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("the sub-face seen through the parent's hole is visible");
        assert!(
            (snap.position.z - 1.0).abs() <= tol::PLANE_DIST,
            "snap stays on the visible coplanar top (z=1): {:?}",
            snap.position
        );
    }

    /// The constraint plane excludes only *off-plane* geometry — on-plane
    /// vertices/edges/midpoints still snap with their proper kind.
    #[test]
    fn constraint_plane_keeps_on_plane_geometry() {
        let scene = cube_scene();
        // Aim near the top corner (1,1,1) from above-diagonal; with the top plane
        // constraint the corner (an Endpoint, on z=1) must still win.
        let eye = Point3::new(2.0, 2.0, 4.0);
        let top =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: eye,
                    direction: Point3::new(1.0, 1.0, 1.0) - eye,
                },
                anchor: None,
                lock: None,
                aperture: 0.6,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: Some(top),
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("the on-plane top corner is still snappable");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(
            snap.position
                .approx_eq(Point3::new(1.0, 1.0, 1.0), tol::POINT_MERGE)
        );
    }

    /// Builds a unit cube via `from_polygons` (top face has normal +Z).
    fn unit_cube() -> Object {
        kernel::Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap()
    }

    #[test]
    fn pick_face_through_a_hole_returns_the_sub_face_not_the_annular_parent() {
        // Imprint a sub-face on a cube's top, then a ray straight down through
        // the inner rectangle must pick the SUB-FACE — not the annular parent,
        // whose outer boundary still contains the point but whose hole does
        // not. (Picking the parent here was the NonManifoldResult push/pull
        // bug: translating a holed face whose hole edges are twinned with the
        // sub-face is non-manifold.)
        let mut cube = unit_cube();
        let top = cube
            .faces()
            .iter()
            .find(|(_, f)| {
                f.plane
                    .normal()
                    .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
            })
            .map(|(id, _)| id)
            .unwrap();
        let sub_face = cube
            .split_face_inner(
                top,
                &[
                    Point3::new(0.25, 0.25, 1.0),
                    Point3::new(0.75, 0.25, 1.0),
                    Point3::new(0.75, 0.75, 1.0),
                    Point3::new(0.25, 0.75, 1.0),
                ],
            )
            .unwrap()
            .sub_face;

        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);

        // Straight down through the centre of the inner rectangle.
        let (through_hole, hole_depth) = scene
            .pick_face(&PickRay {
                origin: Point3::new(0.5, 0.5, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            })
            .expect("the ray crosses the top of the cube");
        assert_eq!(
            through_hole.element,
            ElementRef::Face(sub_face),
            "a ray through the hole picks the sub-face, not the parent"
        );
        assert!(hole_depth > 0.0, "the hit is in front of the ray origin");

        // Through the annular ring (clear of the hole): still the parent.
        let (through_ring, _depth) = scene
            .pick_face(&PickRay {
                origin: Point3::new(0.1, 0.1, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            })
            .expect("the ray crosses the annular top");
        assert_eq!(
            through_ring.element,
            ElementRef::Face(top),
            "a ray through the ring picks the annular parent"
        );
    }

    // -----------------------------------------------------------------------
    // FIX A: closest_point_on_line_to_ray sign regression
    // -----------------------------------------------------------------------

    /// The closest point on the Z-axis line to a ray pointing in -X from
    /// (10, 0, 5) must be (0, 0, 5), NOT (0, 0, -5) (which the wrong-sign
    /// formula would return).
    #[test]
    fn closest_point_on_line_to_ray_correct_sign() {
        // Line: Z axis through origin.
        let line_origin = Point3::ORIGIN;
        let line_dir = Vec3::new(0.0, 0.0, 1.0); // unit +Z

        // Ray: starts at (10, 0, 5), points in -X.
        let ray_origin = Point3::new(10.0, 0.0, 5.0);
        let ray_dir = Vec3::new(-1.0, 0.0, 0.0); // unit -X

        let pt = closest_point_on_line_to_ray(line_origin, line_dir, None, ray_origin, ray_dir);

        // The Z-axis point closest to this ray is at z=5 (same height as the
        // ray origin), i.e. (0, 0, 5). The wrong-sign formula gives (0, 0, -5).
        assert!(
            pt.approx_eq(Point3::new(0.0, 0.0, 5.0), tol::POINT_MERGE),
            "expected (0,0,5) but got {:?}",
            pt
        );
    }

    /// Near (but not exactly) edge-on, `closest_point_on_line_to_ray`'s
    /// result must stay within a bounded distance of `line_origin` —
    /// playtest-2 review finding F. The 3° `SOFT_AXIS_EDGE_ON_DEG` guard
    /// only excludes candidates BELOW that angle; it does nothing to bound
    /// the result AT or just past it, where the function is still severely
    /// ill-conditioned (`tol::NORMALIZE_MIN_LENGTH`'s own guard only catches
    /// angles below roughly 0.00006°). Without a clamp, a ray exactly at the
    /// 3° edge-on boundary — a perfectly ordinary "roughly edge-on" viewing
    /// angle, not a contrived extreme — resolves to a point ~400m from the
    /// line's own origin against a camera only ~35m away, and a single
    /// pixel's worth of mouse movement (~0.06°) swings that point by several
    /// METERS (empirically measured while investigating this finding).
    #[test]
    fn closest_point_on_line_to_ray_stays_bounded_near_the_edge_on_angle() {
        let line_origin = Point3::ORIGIN;
        let line_dir = Vec3::new(1.0, 0.0, 0.0); // world +X

        // Camera ~34.6m from the line's origin, aimed 3° off parallel to the
        // axis — exactly the boundary the edge-on guard admits.
        let ray_origin = Point3::new(-20.0, 20.0, 20.0);
        let ref_dist = (line_origin - ray_origin).length();
        let theta = 3.0_f64.to_radians();
        let ray_dir = Vec3::new(theta.cos(), theta.sin(), 0.0)
            .normalized()
            .unwrap();

        let pt = closest_point_on_line_to_ray(line_origin, line_dir, None, ray_origin, ray_dir);
        let reach = (pt - line_origin).length();
        assert!(
            reach <= ref_dist * MAX_AXIS_REACH_FACTOR + tol::POINT_MERGE,
            "candidate reached {reach:.1}m along the axis from a camera only {ref_dist:.1}m away \
             (bound was {:.1}m) — unbounded near the edge-on angle",
            ref_dist * MAX_AXIS_REACH_FACTOR
        );
    }

    /// The clamp does not distort a WELL-conditioned (broadside-ish) result:
    /// the ordinary case pinned by `closest_point_on_line_to_ray_correct_sign`
    /// above must still land exactly on the true closest point, unclamped.
    #[test]
    fn closest_point_on_line_to_ray_clamp_does_not_affect_well_conditioned_rays() {
        let line_origin = Point3::ORIGIN;
        let line_dir = Vec3::new(0.0, 0.0, 1.0);
        let ray_origin = Point3::new(10.0, 0.0, 5.0);
        let ray_dir = Vec3::new(-1.0, 0.0, 0.0);
        let pt = closest_point_on_line_to_ray(line_origin, line_dir, None, ray_origin, ray_dir);
        assert!(pt.approx_eq(Point3::new(0.0, 0.0, 5.0), tol::POINT_MERGE));
    }

    // -----------------------------------------------------------------------
    // Soft axis inference (tool-parity playtest2 §2c)
    // -----------------------------------------------------------------------

    /// A ray genuinely broadside to the axis (the common case — looking down
    /// at the ground while dragging along a horizontal axis) is never
    /// edge-on, however far off-axis it drifts.
    #[test]
    fn axis_is_edge_on_false_when_broadside() {
        let ray_dir = Vec3::new(0.0, 0.0, -1.0); // straight down
        let axis_dir = Vec3::new(1.0, 0.0, 0.0); // horizontal
        assert!(!axis_is_edge_on(ray_dir, axis_dir));
    }

    /// A ray sighting nearly straight down the axis — either direction along
    /// it — is edge-on. Orientation-free: `-axis_dir` is just as degenerate.
    #[test]
    fn axis_is_edge_on_true_when_nearly_parallel_either_direction() {
        let axis_dir = Vec3::new(0.0, 0.0, 1.0);
        let two_deg = (2.0_f64).to_radians();
        let nearly_parallel = Vec3::new(two_deg.sin(), 0.0, two_deg.cos())
            .normalized()
            .unwrap();
        assert!(axis_is_edge_on(nearly_parallel, axis_dir));
        assert!(axis_is_edge_on(-nearly_parallel, axis_dir));
    }

    /// Just past the edge-on threshold, the axis reads as trustworthy again —
    /// the guard isn't accidentally swallowing ordinary steep-but-valid
    /// viewing angles.
    #[test]
    fn axis_is_edge_on_false_just_past_the_threshold() {
        let axis_dir = Vec3::new(0.0, 0.0, 1.0);
        let ten_deg = (10.0_f64).to_radians();
        let mostly_parallel = Vec3::new(ten_deg.sin(), 0.0, ten_deg.cos())
            .normalized()
            .unwrap();
        assert!(!axis_is_edge_on(mostly_parallel, axis_dir));
    }

    /// Exactly perpendicular is the least edge-on a ray can be — the opposite
    /// end of the same test, pinning the guard doesn't fire on ordinary
    /// geometry near it either.
    #[test]
    fn axis_is_edge_on_false_when_perpendicular() {
        let ray_dir = Vec3::new(1.0, 0.0, 0.0);
        let axis_dir = Vec3::new(0.0, 1.0, 0.0);
        assert!(!axis_is_edge_on(ray_dir, axis_dir));
    }

    // -----------------------------------------------------------------------
    // `soft_axis_deviation` (tool-parity delta-review finding 1): a matrix
    // over camera position x drag length x true off-axis angle, so a single
    // pair of point tests can't let a false-admission and a false-rejection
    // bug both through unnoticed the way the first two generations of this
    // function did (one over-admitted at any distance from a near-overhead
    // camera; the other, its replacement, under-admitted a genuinely dead-on
    // aim at any distance from anything BUT a near-overhead camera).
    // -----------------------------------------------------------------------

    /// The four `matrix_cameras` orientations as unit eye-to-anchor
    /// directions, factored out so both the original far distances and the
    /// close ones below (tool-parity delta-review finding: reach-clamp
    /// regression) sight the axis from the same set of angles — only the
    /// distance changes.
    fn matrix_camera_directions() -> Vec<(&'static str, Vec3)> {
        let five_deg = 5.0_f64.to_radians();
        let near_edge_on_dir =
            Vec3::new(five_deg.cos(), five_deg.sin() * 0.6, five_deg.sin() * 0.8)
                .normalized()
                .unwrap();
        vec![
            ("overhead", Vec3::new(0.0, 0.0, 1.0)),
            (
                "3/4 orbit",
                Vec3::new(-10.0, -10.0, 8.0).normalized().unwrap(),
            ),
            (
                "low oblique",
                Vec3::new(16.0, 11.0, -5.0).normalized().unwrap(),
            ),
            ("near-edge-on (5deg)", -near_edge_on_dir),
        ]
    }

    /// Four cameras spanning the ordinary range an orbiting user actually
    /// uses: straight overhead (the shape every fixture before this finding
    /// happened to use), an ordinary 3/4 orbit (the exact camera from the
    /// finding that caught the second-generation bug), a low, shallow
    /// oblique angle, and a camera just past the 3° `axis_is_edge_on`
    /// cutoff — the most ill-conditioned non-excluded viewing angle there
    /// is. None of these is a contrived or adversarial position. Eye
    /// distances (31, √264, √402, 200 respectively) are the original
    /// absolute camera positions this matrix has used since its
    /// introduction, now expressed as `matrix_camera_directions` scaled by
    /// distance instead of hardcoded points — same cameras, same numbers.
    fn matrix_cameras(anchor: Point3) -> Vec<(&'static str, Point3)> {
        let distances = [31.0, 264.0_f64.sqrt(), 402.0_f64.sqrt(), 200.0];
        matrix_camera_directions()
            .into_iter()
            .zip(distances)
            .map(|((name, dir), dist)| (name, anchor + dir * dist))
            .collect()
    }

    /// Cameras sighted from CLOSE up — 0.3 to 2 units from the anchor,
    /// rather than `matrix_cameras`'s 16-200 — at four different azimuths
    /// around the axis, so the eye-to-anchor distance is small relative to
    /// a multi-metre drag.
    ///
    /// This closes a blind spot in the original matrix (tool-parity
    /// delta-review finding: reach-clamp regression): every camera there
    /// sat far enough from the anchor, relative to the drag lengths tested,
    /// that a since-removed clamp bounding `soft_axis_deviation`'s ray
    /// parameter to `MAX_AXIS_REACH_FACTOR` (5x) the eye-to-anchor distance
    /// was never actually approached — so a regression that fires only when
    /// drag length exceeds roughly 5x eye distance (an everyday "zoom in
    /// close, drag a long way along the axis" interaction) went undetected.
    ///
    /// Deliberately BROADSIDE (perpendicular to `axis_dir`) rather than
    /// reusing `matrix_camera_directions`'s oblique orientations: for a
    /// dead-on drag of length `len` from an eye `dist` from the anchor,
    /// `anchor` itself stays in front of the ray (the precondition
    /// `soft_axis_deviation` requires) only while `b_cam < dist / len`,
    /// where `b_cam` is the dot of `axis_dir` with the eye-to-anchor
    /// direction — a short derivation from the shared `w`/`b` setup, not an
    /// empirical tuning, and since `dist / len` is always positive, only a
    /// POSITIVE `b_cam` can ever violate it. At this sweep's tightest corner
    /// (`dist` = 0.3, `len` = 30) that bound is `b_cam < 0.01`, i.e. under a
    /// degree off exactly broadside: of `matrix_camera_directions`'s four
    /// orientations, only "low oblique" (`b_cam` ≈ +0.80) is positive, and
    /// it violates that bound well before the sweep's extremes, which is why
    /// reusing it here would produce spurious `depth_anchor <= 0` rejections
    /// that are about camera/target geometry going behind-the-eye, not about
    /// the clamp regression this matrix exists to catch. ("3/4 orbit",
    /// `b_cam` ≈ -0.62, never violates it at any `dist`/`len` — reusing it
    /// would just duplicate what the broadside directions below already
    /// cover.) A user who is both close to the anchor AND
    /// dragging far along the axis is, by this same bound, necessarily
    /// looking close to broadside at that point in the drag — so broadside
    /// is not a narrowing of the regime, it is what the regime actually is.
    ///
    /// Each azimuth is paired with its own off-axis perturbation direction
    /// (also perpendicular to `axis_dir`, and to the azimuth itself) for the
    /// caller's 25-degrees-off check, rather than one fixed perpendicular
    /// shared by every camera: a fixed choice coinciding with a camera's own
    /// azimuth (e.g. perturbing toward +Y while sighting from along +Y)
    /// makes eye, anchor, and the off-axis target coplanar — the disclosed
    /// residual gap `soft_axis_deviation_can_still_be_fooled_by_a_near_coplanar_camera`
    /// documents — which would make this matrix's rejection check spuriously
    /// fail on exactly that known, already-accepted characteristic instead
    /// of on anything this fix is about. The four azimuths here pair up
    /// into two mutually-perpendicular sets (+Y/+Z, and the two diagonals),
    /// so each is its partner's perturbation direction.
    fn close_matrix_cameras(anchor: Point3) -> Vec<(String, Point3, Vec3)> {
        let azimuths: [(&str, Vec3, Vec3); 4] = [
            (
                "broadside +Y",
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
            (
                "broadside +Z",
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            (
                "broadside +Y+Z",
                Vec3::new(0.0, 1.0, 1.0).normalized().unwrap(),
                Vec3::new(0.0, -1.0, 1.0).normalized().unwrap(),
            ),
            (
                "broadside -Y+Z",
                Vec3::new(0.0, -1.0, 1.0).normalized().unwrap(),
                Vec3::new(0.0, 1.0, 1.0).normalized().unwrap(),
            ),
        ];
        let distances = [0.3, 0.5, 1.0, 2.0];
        azimuths
            .into_iter()
            .flat_map(|(name, dir, perp)| {
                distances
                    .into_iter()
                    .map(move |dist| (format!("{name} @ {dist}m"), anchor + dir * dist, perp))
            })
            .collect()
    }

    /// The drag lengths the finding pinned explicitly (0.05m up to a
    /// multi-metre wall at 20m), plus the finding's own anchor/axis so the
    /// numbers here are directly comparable to the ones pasted in the
    /// finding and in the fix's commit message.
    #[test]
    fn soft_axis_deviation_matrix_admits_dead_on_and_rejects_25_degrees_off() {
        let anchor = Point3::new(2.0, 3.0, 9.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        let perp = Vec3::new(0.0, 1.0, 0.0);
        let lengths = [0.05, 1.0, 3.0, 10.0, 20.0];
        let theta_off = 25.0_f64.to_radians(); // 5x SOFT_AXIS_APERTURE_DEG — the original bug's own angle

        for (name, eye) in matrix_cameras(anchor) {
            for &len in &lengths {
                // Dead-on: the ray is aimed at a point genuinely ON the
                // axis. Must ADMIT, at (numerically) exactly zero deviation,
                // regardless of camera or drag length.
                let dead_on = anchor + axis_dir * len;
                let dir_dead_on = (dead_on - eye).normalized().unwrap();
                let angle =
                    soft_axis_deviation(anchor, axis_dir, eye, dir_dead_on, SOFT_AXIS_APERTURE);
                assert!(
                    angle.is_some_and(|a| a < 1e-6),
                    "{name} at {len}m dead-on: expected ~0 deviation (admitted), got {angle:?}"
                );

                // 25 degrees off (five times the ~5 degree aperture): must
                // REJECT, at every camera and every drag length, matching
                // `soft_axis_admission_does_not_widen_with_an_oblique_camera`'s
                // requirement but swept over camera x length instead of one
                // fixed pair.
                let off_axis =
                    anchor + axis_dir * (len * theta_off.cos()) + perp * (len * theta_off.sin());
                let dir_off = (off_axis - eye).normalized().unwrap();
                let angle_off =
                    soft_axis_deviation(anchor, axis_dir, eye, dir_off, SOFT_AXIS_APERTURE);
                assert!(
                    angle_off.is_none(),
                    "{name} at {len}m, 25 degrees off axis: expected rejection, got {angle_off:?}"
                );
            }
        }
    }

    /// The axis through `anchor` is a full line, not a ray from it: aiming
    /// dead-on at a point on the axis on the anchor's OTHER side (negative
    /// `t`) must read the same near-zero deviation as a forward drag —
    /// pinning the `along.abs()` in `soft_axis_deviation`, without which a
    /// backward drag would read close to 180 degrees instead of 0.
    #[test]
    fn soft_axis_deviation_backward_drag_reads_the_same_as_forward() {
        let anchor = Point3::new(2.0, 3.0, 9.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        let eye = Point3::new(-8.0, -7.0, 17.0);
        for len in [-0.05, -1.0, -3.0, -10.0, -20.0] {
            let target = anchor + axis_dir * len;
            let dir = (target - eye).normalized().unwrap();
            let angle = soft_axis_deviation(anchor, axis_dir, eye, dir, SOFT_AXIS_APERTURE);
            assert!(
                angle.is_some_and(|a| a < 1e-6),
                "backward drag at {len}m: expected ~0 deviation (admitted), got {angle:?}"
            );
        }
    }

    /// A drag of essentially zero length — `Q` lands essentially on `anchor`
    /// itself — is well-conditioned: no NaN, no panic, and (since the ray is
    /// still dead-on) admission at ~zero deviation, not a spurious rejection
    /// from `atan2`'s `(0, 0)` corner.
    #[test]
    fn soft_axis_deviation_near_zero_drag_is_well_conditioned() {
        let anchor = Point3::new(2.0, 3.0, 9.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        let eye = Point3::new(-8.0, -7.0, 17.0);
        let target = anchor + axis_dir * 1e-6;
        let dir = (target - eye).normalized().unwrap();
        let angle = soft_axis_deviation(anchor, axis_dir, eye, dir, SOFT_AXIS_APERTURE);
        assert!(
            angle.is_some_and(|a| a.is_finite() && a < 1e-3),
            "near-zero drag: expected a finite, ~0 deviation, got {angle:?}"
        );
    }

    /// The exact counterexample from the reach-clamp regression finding: a
    /// plain broadside camera 2 units from the anchor, dragging dead-on
    /// along the axis for 20 units — an ordinary "zoom in close, then drag
    /// a much longer line along an axis" interaction, not a contrived one.
    /// The since-removed clamp bounded this function's ray parameter to
    /// `MAX_AXIS_REACH_FACTOR` (5x) the eye-to-anchor distance (2m here, so
    /// a 10m bound) — well inside the 20m drag — and so rejected a
    /// mathematically exact (3e-12 degree) dead-on aim as if it were 5.77
    /// degrees off.
    #[test]
    fn soft_axis_deviation_close_camera_long_drag_counterexample_is_admitted() {
        let anchor = Point3::new(0.0, 0.0, 0.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        let eye = Point3::new(0.0, 2.0, 0.0);
        let target = Point3::new(20.0, 0.0, 0.0);
        let dir = (target - eye).normalized().unwrap();
        let angle = soft_axis_deviation(anchor, axis_dir, eye, dir, SOFT_AXIS_APERTURE);
        assert!(
            angle.is_some_and(|a| a < 1e-6),
            "expected ~0 deviation (admitted), got {angle:?}"
        );
    }

    /// The regime the original matrix missed entirely (tool-parity
    /// delta-review finding: reach-clamp regression): camera CLOSE to the
    /// anchor (0.3-2 units — `close_matrix_cameras`), drag LONG (2-30m, up
    /// to 100x the eye distance). Dead-on must still admit at (numerically)
    /// exactly zero deviation and a genuine 25-degree-off aim must still be
    /// rejected, exactly as `soft_axis_deviation_matrix_admits_dead_on_and_rejects_25_degrees_off`
    /// requires of its own (far-camera) matrix — this is that same
    /// assertion, just over the opposite corner of camera-distance x
    /// drag-length space.
    ///
    /// This is deliberately NOT extended here with a "camera far, drag very
    /// long" case reusing `matrix_cameras`'s oblique orientations, because
    /// that corner isn't uniformly coherent across all four of them: from
    /// `close_matrix_cameras`'s doc, `depth_anchor = dist * (dist - len *
    /// b_cam) / L` for some always-positive `L`, so `anchor` stays in front
    /// of a dead-on ray (`depth_anchor > 0`) unless `b_cam >= dist / len` —
    /// and since `dist / len` is always positive, that can only happen when
    /// `b_cam` itself is positive. Of `matrix_cameras`'s four cameras, only
    /// "low oblique" has positive `b_cam` (≈ +0.80), with a genuine finite
    /// crossing at `len = dist / b_cam ≈ 20.05 / 0.80 ≈ 25.1m` — comfortably
    /// past this matrix's own 20m cap, not "as short as ~13-16m" as an
    /// earlier revision of this comment claimed. "3/4 orbit" (`b_cam` ≈
    /// -0.62), "overhead" (`b_cam` = 0), and "near-edge-on" (`b_cam` ≈
    /// -0.996) all have `b_cam <= 0`, so `dist - len * b_cam` is strictly
    /// increasing in `len` and never crosses zero AT ANY drag length — for
    /// those three, "camera far, drag very long" is exactly as geometrically
    /// coherent as the close-camera corner this test covers, and was simply
    /// untested: neither this matrix (capped at 20m) nor
    /// `close_matrix_cameras`'s (which never puts a camera farther than 2
    /// units out) ever paired a far eye with a drag long enough to approach,
    /// let alone clear, `MAX_AXIS_REACH_FACTOR` (5x) that eye's own
    /// distance — the regime the since-removed clamp actually misfired in.
    /// `soft_axis_deviation_far_camera_long_drag_matrix_admits_dead_on_and_rejects_25_degrees_off`
    /// closes that gap, restricted to "overhead" and "3/4 orbit" — two of
    /// those three `b_cam <= 0` orientations, excluding "low oblique" for
    /// the reason above, and ALSO excluding "near-edge-on" (see that test's
    /// own doc for why: an independent, already-documented ill-conditioning
    /// reason, not a `depth_anchor <= 0` one) — at drag lengths well beyond
    /// 5x each included camera's eye distance.
    #[test]
    fn soft_axis_deviation_close_camera_matrix_admits_dead_on_and_rejects_25_degrees_off() {
        let anchor = Point3::new(2.0, 3.0, 9.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        let lengths = [2.0, 5.0, 10.0, 20.0, 30.0];
        let theta_off = 25.0_f64.to_radians();

        for (name, eye, perp) in close_matrix_cameras(anchor) {
            for &len in &lengths {
                let dead_on = anchor + axis_dir * len;
                let dir_dead_on = (dead_on - eye).normalized().unwrap();
                let angle =
                    soft_axis_deviation(anchor, axis_dir, eye, dir_dead_on, SOFT_AXIS_APERTURE);
                assert!(
                    angle.is_some_and(|a| a < 1e-6),
                    "{name} at {len}m dead-on: expected ~0 deviation (admitted), got {angle:?}"
                );

                let off_axis =
                    anchor + axis_dir * (len * theta_off.cos()) + perp * (len * theta_off.sin());
                let dir_off = (off_axis - eye).normalized().unwrap();
                let angle_off =
                    soft_axis_deviation(anchor, axis_dir, eye, dir_off, SOFT_AXIS_APERTURE);
                assert!(
                    angle_off.is_none(),
                    "{name} at {len}m, 25 degrees off axis: expected rejection, got {angle_off:?}"
                );
            }
        }
    }

    /// The far-camera counterpart to
    /// `soft_axis_deviation_close_camera_matrix_admits_dead_on_and_rejects_25_degrees_off`,
    /// closing the gap that test's doc identifies (tool-parity delta-review
    /// finding: the doc comment there previously claimed this corner was
    /// untestable, which was wrong for two of `matrix_cameras`'s four
    /// cameras — see that test's doc for the corrected derivation): camera
    /// FAR from the anchor (`matrix_cameras`'s 16-200 units) and drag long
    /// enough to clear `MAX_AXIS_REACH_FACTOR` (5x) even the farther of the
    /// two included cameras' own eye distance — the ratio at which the
    /// since-removed eye-distance-scaled clamp on `soft_axis_deviation`'s
    /// ray parameter misfired. Restricted to "overhead" and "3/4 orbit"
    /// (both `b_cam <= 0`, so both stay in front of a dead-on ray at every
    /// drag length): "low oblique" (`b_cam` ≈ +0.80) has its own genuine
    /// finite crossing at ~25.1m and stays excluded for that reason, exactly
    /// as before. "near-edge-on (5deg)" is ALSO excluded here, for a
    /// different, already-documented reason: it is deliberately the most
    /// ill-conditioned non-excluded camera in the matrix (see
    /// `matrix_cameras`'s doc), sighted only 3 degrees off parallel with the
    /// axis, and at these extreme drag lengths its 25-degree-off target
    /// direction drifts into the same near-coplanar degeneracy documented on
    /// `soft_axis_deviation_can_still_be_fooled_by_a_near_coplanar_camera`
    /// (verified: it reads a genuine ~4.5-degree angle there, comfortably
    /// under the aperture — a real instance of that disclosed gap, not a
    /// `depth_anchor <= 0` rejection and not a new defect) — a second,
    /// independent reason a real single click-drag wouldn't put it here
    /// anyway, since edge-on cameras make the soft-axis candidate untrusted
    /// well before this (see `axis_is_edge_on`, gating the real call site).
    #[test]
    fn soft_axis_deviation_far_camera_long_drag_matrix_admits_dead_on_and_rejects_25_degrees_off() {
        let anchor = Point3::new(2.0, 3.0, 9.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        let perp = Vec3::new(0.0, 1.0, 0.0);
        // Well beyond 5x each included camera's own eye distance (31m and
        // ~16.25m for "overhead" and "3/4 orbit" respectively — up to ~97x
        // the closer of the two).
        let lengths = [1200.0, 2000.0, 3000.0];
        let theta_off = 25.0_f64.to_radians();

        for (name, eye) in matrix_cameras(anchor)
            .into_iter()
            .filter(|(name, _)| *name == "overhead" || *name == "3/4 orbit")
        {
            for &len in &lengths {
                let dead_on = anchor + axis_dir * len;
                let dir_dead_on = (dead_on - eye).normalized().unwrap();
                let angle =
                    soft_axis_deviation(anchor, axis_dir, eye, dir_dead_on, SOFT_AXIS_APERTURE);
                assert!(
                    angle.is_some_and(|a| a < 1e-6),
                    "{name} at {len}m dead-on: expected ~0 deviation (admitted), got {angle:?}"
                );

                let off_axis =
                    anchor + axis_dir * (len * theta_off.cos()) + perp * (len * theta_off.sin());
                let dir_off = (off_axis - eye).normalized().unwrap();
                let angle_off =
                    soft_axis_deviation(anchor, axis_dir, eye, dir_off, SOFT_AXIS_APERTURE);
                assert!(
                    angle_off.is_none(),
                    "{name} at {len}m, 25 degrees off axis: expected rejection, got {angle_off:?}"
                );
            }
        }
    }

    /// Disclosed, accepted residual gap (see `soft_axis_deviation`'s doc):
    /// being exactly zero for a dead-on aim at every drag length and every
    /// camera forces this measure to be equivalent to whether the ray and
    /// the axis, as infinite lines, are coplanar — and an off-axis target
    /// whose direction from `anchor` happens to lie in the plane spanned by
    /// the axis and the eye-to-anchor vector reads as zero deviation
    /// regardless of how far off-axis it actually is. This camera is
    /// ordinary (a near-overhead 3/4 orbit, not edge-on, not contrived) and
    /// the drag is a genuine 25 degrees off axis — five times the aperture —
    /// yet it is admitted well inside tolerance. This is a known
    /// characteristic pinned so a future change to this function doesn't
    /// silently narrow or widen it without the change being visible here;
    /// it is not something this fix claims to have closed (closing it for
    /// good needs real screen-space information this function doesn't
    /// receive).
    #[test]
    fn soft_axis_deviation_can_still_be_fooled_by_a_near_coplanar_camera() {
        let anchor = Point3::new(2.0, 3.0, 9.0);
        let axis_dir = Vec3::new(1.0, 0.0, 0.0);
        // Directly above `anchor` (same x, y) — coplanar with the axis and
        // with any drag confined to the x/z plane.
        let eye = Point3::new(2.0, 3.0, 40.0);
        let theta = 25.0_f64.to_radians();
        let len = 3.0;
        // Off-axis toward +Z — in the SAME (x, z) plane as `eye` and the
        // axis, which is exactly the degenerate alignment.
        let perp = Vec3::new(0.0, 0.0, 1.0);
        let target = anchor + axis_dir * (len * theta.cos()) + perp * (len * theta.sin());
        let dir = (target - eye).normalized().unwrap();
        let angle = soft_axis_deviation(anchor, axis_dir, eye, dir, SOFT_AXIS_APERTURE);
        assert!(
            angle.is_some(),
            "documenting the known gap: a genuinely 25-degree-off aim from a near-coplanar \
             camera is still admitted (got {angle:?}) — see this test's doc"
        );
    }

    // -----------------------------------------------------------------------
    // FIX B: world-axis and origin snapping
    // -----------------------------------------------------------------------

    /// A ray that passes near the +X axis (but well away from the origin)
    /// should snap to OnAxis with direction ≈ (1, 0, 0).
    #[test]
    fn resolve_snaps_to_x_axis() {
        let scene = InferenceScene::new(); // empty — no objects

        // Ray origin at (5, 3, 0.05), looking in the -Y direction. The
        // closest approach to the X axis is (5, 0, 0) — well away from the
        // world origin (0,0,0), so the origin Endpoint is not on this ray at
        // all, and the result should be OnAxis along +X.
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(5.0, 3.0, 0.05),
                    direction: Vec3::new(0.0, -1.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("X-axis point (5,0,0) is within the cone");
        assert_eq!(snap.kind, SnapKind::OnAxis, "kind should be OnAxis");
        let dir = snap.direction.expect("OnAxis snap must carry a direction");
        assert!(
            dir.approx_eq(Vec3::new(1.0, 0.0, 0.0), tol::NORMAL_DIRECTION),
            "direction should be +X, got {:?}",
            dir
        );
    }

    /// A ray aimed straight at the world origin should snap to
    /// `SnapKind::Endpoint` at `Point3::ORIGIN`.
    #[test]
    fn resolve_snaps_to_world_origin() {
        let scene = InferenceScene::new();

        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(10.0, 0.0, 0.0),
                    direction: Vec3::new(-1.0, 0.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("origin is directly on the ray");
        assert_eq!(snap.kind, SnapKind::Endpoint, "origin snaps as Endpoint");
        assert!(
            snap.position.approx_eq(Point3::ORIGIN, tol::POINT_MERGE),
            "position should be origin, got {:?}",
            snap.position
        );
    }

    /// Object geometry (Endpoint at a cube vertex) outranks an axis snap even
    /// when the vertex also lies near a world axis.
    #[test]
    fn object_vertex_outranks_axis_snap() {
        // The cube has a vertex at (1, 0, 0) which lies ON the X axis.
        // A ray aimed directly at that vertex must resolve to Endpoint, not OnAxis.
        let scene = cube_scene();

        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(5.0, 0.0, 0.0),
                    direction: Vec3::new(-1.0, 0.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("cube vertex (1,0,0) is on this ray");
        assert_eq!(
            snap.kind,
            SnapKind::Endpoint,
            "object vertex must beat axis snap, got {:?}",
            snap
        );
        assert!(
            snap.position
                .approx_eq(Point3::new(1.0, 0.0, 0.0), tol::POINT_MERGE),
            "position should be the cube vertex, got {:?}",
            snap.position
        );
    }

    // -----------------------------------------------------------------------
    // : construction guide snapping
    // -----------------------------------------------------------------------

    /// A registered guide line resolves to `SnapKind::OnGuide`, carrying the
    /// guide's direction, at a position on the line near the ray.
    #[test]
    fn resolve_snaps_to_guide_line() {
        let mut scene = InferenceScene::new();
        let guide_dir = Vec3::new(0.0, 1.0, 0.0); // +Y
        let guide = Guide::Line {
            origin: Point3::new(2.0, 0.0, 0.0),
            direction: guide_dir,
            span: kernel::GuideSpan::INFINITE,
        };
        scene.add_guide(GuideId::default(), &guide);
        assert_eq!(scene.guide_count(), 1);

        // Ray crosses the guide line (the Y axis through x=2, z=0) from the side.
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(2.0, 5.0, 3.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("ray passes through the guide line");
        assert_eq!(snap.kind, SnapKind::OnGuide);
        assert_eq!(snap.direction, Some(guide_dir));
        // The snapped position must lie on the guide line (x=2, z=0).
        assert!((snap.position.x - 2.0).abs() < tol::POINT_MERGE);
        assert!(snap.position.z.abs() < tol::POINT_MERGE);
    }

    /// A registered guide point resolves to `SnapKind::Endpoint` at that
    /// point — a guide point is a precise snap, same tier as a real vertex.
    #[test]
    fn resolve_snaps_to_guide_point() {
        let mut scene = InferenceScene::new();
        let guide_pos = Point3::new(3.0, 4.0, 0.0);
        scene.add_guide(
            GuideId::default(),
            &Guide::Point {
                position: guide_pos,
            },
        );

        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(3.0, 4.0, 10.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("ray points straight at the guide point");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(snap.position.approx_eq(guide_pos, tol::POINT_MERGE));
    }

    /// Real object geometry coincident with a guide still wins: Endpoint and
    /// OnEdge both outrank OnGuide.
    #[test]
    fn object_geometry_outranks_coincident_guide() {
        let mut scene = cube_scene();
        // A guide line running along the cube's vertical edge through (1,1,*),
        // which coincides with the cube's edge from (1,1,0) to (1,1,1).
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(1.0, 1.0, 0.0),
                direction: Vec3::new(0.0, 0.0, 1.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );

        // Aim straight at the cube vertex (1,1,1), which also lies exactly on
        // the guide line: the vertex Endpoint must win over OnGuide.
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(1.0, 1.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("cube vertex and guide line are both on this ray");
        assert_eq!(
            snap.kind,
            SnapKind::Endpoint,
            "a coincident object vertex must outrank the guide, got {:?}",
            snap
        );

        // Aim at the midpoint of that same edge (0.5 up) from a ray that does
        // NOT pass through either vertex (offset to the side and angled), so
        // only the midpoint and the guide line (both within the cone, at
        // very different angles from the vertices) compete.
        let mid_snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(1.5, 1.0, 0.5),
                    direction: Vec3::new(-1.0, 0.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("cube edge midpoint and guide line are both on this ray");
        assert_eq!(
            mid_snap.kind,
            SnapKind::Midpoint,
            "a coincident edge midpoint must outrank the guide, got {:?}",
            mid_snap
        );
    }

    /// `constraint_plane` drops an off-plane guide candidate exactly like any
    /// other off-plane candidate.
    #[test]
    fn constraint_plane_drops_off_plane_guide() {
        let mut scene = InferenceScene::new();
        // A horizontal guide line at x=10, z=5, well off the z=0 constraint
        // plane AND well clear of the world axes/origin: a guide placed ON
        // an axis — the original x=0 fixture coincidentally was — would let
        // this test's ray tie against the origin's own Z-axis candidate
        // instead of exercising the guide filter this test is actually
        // about (a guide now outranks OnAxis regardless — playtest-2 review
        // finding D — but the tie would still muddy which mechanism the
        // test is checking); x=10 keeps every axis candidate well outside
        // the aperture.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(10.0, 0.0, 5.0),
                direction: Vec3::new(1.0, 0.0, 0.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );

        let ray = PickRay {
            origin: Point3::new(10.0, 5.0, 5.0),
            direction: Vec3::new(0.0, -1.0, 0.0),
        };

        // Unconstrained: the guide line snaps.
        let free = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray,
                anchor: None,
                lock: None,
                aperture: 0.3,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("the guide line is on this ray");
        assert_eq!(free.kind, SnapKind::OnGuide);

        // Constrained to the ground plane (z=0): the off-plane guide is
        // filtered out, leaving nothing to snap to.
        let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let constrained = scene.resolve(&SnapQuery {
            weights: SnapWeights::default(),
            ray,
            anchor: None,
            lock: None,
            aperture: 0.3,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: Some(ground),
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        });
        assert!(
            constrained.is_none(),
            "off-plane guide must be filtered out, got {:?}",
            constrained
        );
    }

    /// A deliberately-placed guide must win over a soft-axis inference, even
    /// when both resolve to the SAME point (playtest-2 review finding D): a
    /// user placed the guide on purpose; an inferred axis (a cue the system
    /// is merely GUESSING at) must not silently outrank it.
    ///
    /// The guide line and the anchor-relative soft-axis line are set up as
    /// the exact same infinite line (both run along world +X through y=3),
    /// so both candidates resolve to the literal same position, angle, and
    /// depth — the tie can only be broken by `SnapKind` rank, not distance.
    #[test]
    fn guide_outranks_a_coincident_soft_axis_inference() {
        let mut scene = InferenceScene::new();
        // A guide along world +X through y=3 — NOT the literal drawing axis
        // (which runs through the origin, y=0) — placed well clear of it so
        // this can't accidentally exercise the origin-relative OnAxis
        // candidate instead of the anchor-relative soft-axis one.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(-5.0, 3.0, 0.0),
                direction: Vec3::new(1.0, 0.0, 0.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );

        // The anchor sits ON that same line, so the soft-axis candidate
        // through it (world +X from the anchor) is the exact same infinite
        // line as the guide — dragging from here "roughly along +X" is
        // indistinguishable, at the resolved point, from being on the guide.
        let anchor = Point3::new(0.0, 3.0, 0.0);

        // Ray aimed straight down at (5, 3, 0) — exactly on that shared line.
        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(5.0, 3.0, 10.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: Some(anchor),
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("both the guide and the soft-axis line pass through this ray");
        assert_eq!(
            snap.kind,
            SnapKind::OnGuide,
            "a deliberately-placed guide must outrank a coincident soft-axis inference, got {:?}",
            snap
        );
    }

    /// The anchor-relative soft-axis candidate honors
    /// `soft_axis_aperture_scale` (playtest-2 review finding E) — this is
    /// what lets the app's magnetic-hysteresis RELEASE query hold onto an
    /// already-acquired soft-axis snap, since that candidate never reads
    /// the widened `aperture` every other sticky kind's hysteresis works
    /// through (see `SnapQuery::soft_axis_aperture_scale`'s doc).
    ///
    /// The line-to-line separation between the ray and the anchor-relative
    /// +X line, referenced against how far along the axis the closest
    /// approach actually lands (`soft_axis_deviation`'s admission angle —
    /// see its doc, tool-parity delta-review finding 1), sits at EXACTLY
    /// 7° — comfortably past the base `SOFT_AXIS_APERTURE_DEG` (5°, so an
    /// ordinary/unscaled query must NOT snap) but comfortably within a
    /// 2x-scaled aperture (10°, the app's own acquire/release radius ratio
    /// — `SNAP_BREAK_RADIUS_PX / SNAP_RADIUS_PX` in `snapService.ts`).
    ///
    /// `h` here is `tan(7°) * 5`, not `* 3`: the reference length is the
    /// axis-relative reach of the ray's closest approach to the anchor-
    /// relative X line, not (as an earlier, now-corrected generation of
    /// `soft_axis_deviation` used) the eye-relative depth at which the ray
    /// reaches the anchor's OWN depth. For this ray (perpendicular to the
    /// axis, x held constant at 5 throughout) those are two genuinely
    /// different lengths — 5 (the eye's, and so the ray's, x-offset from
    /// the anchor) vs. 3 (the eye's y-offset from the anchor) — and every
    /// OTHER near-overhead fixture in this suite happens not to notice
    /// because it was built with those two lengths equal by construction
    /// (eye positioned directly above the TARGET, drop height matching drag
    /// length). This fixture is the one place that asymmetry was exposed
    /// once `soft_axis_deviation` stopped using the eye-relative one.
    #[test]
    fn soft_axis_snap_honors_soft_axis_aperture_scale_for_hysteresis() {
        let scene = InferenceScene::new();
        // Lifted 1 m off the ground so the world X axis (a different line
        // now) sits far outside every cone below.
        let anchor = Point3::new(0.0, 0.0, 1.0);
        // Right-triangle construction (mirrors `resolve_snaps_to_x_axis`):
        // ray origin (5, 3, h) looking along -Y. Because the ray direction
        // is perpendicular to the X axis, the closest point on the
        // anchor-relative X line is exactly (5, 0, 0) regardless of `h` —
        // at axis-relative reach 5 — so the perpendicular separation is
        // exactly `h` and the resulting angle is exactly `atan(h / 5)`.
        let seven_deg: f64 = 7.0_f64.to_radians();
        let h = seven_deg.tan() * 5.0;
        let ray = PickRay {
            origin: Point3::new(5.0, 3.0, 1.0 + h),
            direction: Vec3::new(0.0, -1.0, 0.0),
        };

        let unscaled = SnapQuery {
            weights: SnapWeights::default(),
            ray,
            anchor: Some(anchor),
            lock: None,
            // The eye sees the 7° offset as ~11.6° (0.202 rad: `h` against
            // the 3 m eye-to-axis distance). 0.21 rad lets the soft axis's
            // on-screen cap (`SOFT_AXIS_SCREEN_REACH` × 0.21) admit it, so
            // only the angular tolerance decides.
            aperture: 0.21,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };
        assert!(
            scene.resolve(&unscaled).is_none(),
            "7° off the anchor-relative soft-axis line must miss the unscaled 5° tolerance"
        );

        let widened = SnapQuery {
            soft_axis_aperture_scale: Some(2.0),
            off_plane_points: false,
            see_through_points: false,
            ..unscaled
        };
        let snap = scene
            .resolve(&widened)
            .expect("a 2x-scaled query (the app's hysteresis release query) must still reach 7°");
        assert_eq!(snap.kind, SnapKind::OnAxis);
        assert_eq!(snap.direction, Some(Vec3::new(1.0, 0.0, 0.0)));
    }

    /// A long-drag fixture shared by the soft-axis cap and point-reach
    /// tests: the anchor at a 3 m cube's top corner (0, 0, 3), the far top
    /// corner (3, 0, 3) on the +X soft axis through it, an ordinary 3/4
    /// view, and a pick aperture of 8 px at a 60° field of view over 900 px.
    const LONG_DRAG_EYE: Point3 = Point3::new(1.5, -8.0, 8.0);
    const LONG_DRAG_APERTURE: f64 = 8.0 * (60.0 * std::f64::consts::PI / 180.0) / 900.0;

    fn long_drag_query(target: Point3, weights: SnapWeights) -> SnapQuery {
        SnapQuery {
            weights,
            ray: PickRay {
                origin: LONG_DRAG_EYE,
                direction: (target - LONG_DRAG_EYE).normalized().unwrap(),
            },
            anchor: Some(Point3::new(0.0, 0.0, 3.0)),
            lock: None,
            aperture: LONG_DRAG_APERTURE,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        }
    }

    /// The angle, seen from `LONG_DRAG_EYE`, between two points — how far
    /// apart they are on screen, in the aperture's own unit.
    fn long_drag_screen_angle(a: Point3, b: Point3) -> f64 {
        let da = (a - LONG_DRAG_EYE).normalized().unwrap();
        let db = (b - LONG_DRAG_EYE).normalized().unwrap();
        da.dot(db).clamp(-1.0, 1.0).acos()
    }

    /// Near the end of a 3 m drag along an axis, a cursor 20 cm off the axis
    /// is well inside the 5° drag-direction tolerance (about 3.8°) but many
    /// pick radii away on screen: the soft axis must not capture it, while a
    /// cursor a fraction of a pick radius off still does. Before the
    /// on-screen cap, the angle alone captured from 25 cm away (and held
    /// out to 50 cm through the release query's doubled cone).
    #[test]
    fn soft_axis_capture_is_capped_on_screen_at_the_end_of_a_long_drag() {
        let scene = InferenceScene::new();
        let off = Point3::new(2.8, 0.0, 3.2);
        let on_axis = Point3::new(2.8, 0.0, 3.0);
        assert!(
            long_drag_screen_angle(off, on_axis) > SOFT_AXIS_SCREEN_REACH * LONG_DRAG_APERTURE,
            "fixture: the 20 cm offset must sit past the on-screen cap"
        );
        let snap = scene.resolve(&long_drag_query(off, SnapWeights::default()));
        assert!(
            snap.is_none_or(|s| s.kind != SnapKind::OnAxis),
            "20 cm off the axis at the end of a 3 m drag must not capture, got {snap:?}"
        );

        let near = Point3::new(2.8, 0.0, 3.004);
        let snap = scene
            .resolve(&long_drag_query(near, SnapWeights::default()))
            .expect("4 mm off the axis is within the on-screen cap");
        assert_eq!(snap.kind, SnapKind::OnAxis);
        assert_eq!(snap.direction, Some(Vec3::new(1.0, 0.0, 0.0)));
    }

    /// A corner sitting on the soft axis captures out to `POINT_SNAP_REACH`
    /// pick radii (the cursor 1.5 radii short of it, still ON the axis line
    /// the soft axis would otherwise hold), in both gravity profiles; past
    /// that reach the axis wins again.
    #[test]
    fn a_corner_on_the_soft_axis_captures_at_point_reach() {
        let mut scene = InferenceScene::new();
        let corner = Point3::new(3.0, 0.0, 3.0);
        scene.add_sketch(
            SketchId::default(),
            &[(SketchEdgeId::default(), corner, Point3::new(3.0, 3.0, 3.0))],
        );
        // Walk back along the axis until the corner sits 1.5 radii away on
        // screen — past the old 1-radius reach, inside the new one.
        let at_radii = |radii: f64| {
            let mut x = 3.0;
            while long_drag_screen_angle(Point3::new(x, 0.0, 3.0), corner)
                < radii * LONG_DRAG_APERTURE
            {
                x -= 1e-4;
            }
            Point3::new(x, 0.0, 3.0)
        };
        for weights in [SnapWeights::default(), SnapWeights::uniform()] {
            let snap = scene
                .resolve(&long_drag_query(at_radii(1.5), weights))
                .expect("the corner is within point reach");
            assert_eq!(snap.kind, SnapKind::Endpoint, "{weights:?}");
            assert!(
                snap.position.approx_eq(corner, tol::POINT_MERGE),
                "{weights:?}"
            );

            let snap = scene
                .resolve(&long_drag_query(at_radii(POINT_SNAP_REACH + 0.1), weights))
                .expect("past point reach the soft axis still holds");
            assert_eq!(snap.kind, SnapKind::OnAxis, "{weights:?}");
        }
    }

    /// A selection pick ([`SnapWeights::for_picking`]) gets no point reach:
    /// a corner 1.5 pick radii off the cursor, which an inference query
    /// captures, is not under the cursor for a click that selects.
    #[test]
    fn a_selection_pick_gets_no_point_reach() {
        let mut scene = InferenceScene::new();
        let corner = Point3::new(3.0, 0.0, 3.0);
        scene.add_sketch(
            SketchId::default(),
            &[(SketchEdgeId::default(), corner, Point3::new(3.0, 3.0, 3.0))],
        );
        let mut x = 3.0;
        while long_drag_screen_angle(Point3::new(x, 0.0, 3.0), corner) < 1.5 * LONG_DRAG_APERTURE {
            x -= 1e-4;
        }
        let aim = Point3::new(x, 0.0, 3.0);
        for weights in [SnapWeights::default(), SnapWeights::uniform()] {
            let infer = SnapQuery {
                anchor: None,
                ..long_drag_query(aim, weights)
            };
            assert_eq!(
                scene.resolve(&infer).map(|s| s.kind),
                Some(SnapKind::Endpoint)
            );
            assert_eq!(scene.resolve(&infer), scene.resolve_linear(&infer));
            let pick = SnapQuery {
                weights: weights.for_picking(),
                ..infer
            };
            assert_eq!(scene.resolve(&pick), None, "{weights:?}");
            assert_eq!(scene.resolve_linear(&pick), None, "{weights:?}");
        }
    }

    /// The hysteresis release query (twice the acquire aperture, declared
    /// through `soft_axis_aperture_scale`) holds a point out to 2.25 acquire
    /// radii — its 1.75 reach plus `POINT_RELEASE_MARGIN` — not the full
    /// 3.5 that scaling the reach by the ratio would give.
    #[test]
    fn a_held_point_releases_with_the_margin_a_line_gets() {
        let mut scene = InferenceScene::new();
        let corner = Point3::new(3.0, 0.0, 3.0);
        scene.add_sketch(
            SketchId::default(),
            &[(SketchEdgeId::default(), corner, Point3::new(3.0, 3.0, 3.0))],
        );
        let release = |radii: f64| {
            let mut x = 3.0;
            while long_drag_screen_angle(Point3::new(x, 0.0, 3.0), corner)
                < radii * LONG_DRAG_APERTURE
            {
                x -= 1e-5;
            }
            let q = SnapQuery {
                anchor: None,
                aperture: 2.0 * LONG_DRAG_APERTURE,
                soft_axis_aperture_scale: Some(2.0),
                ..long_drag_query(Point3::new(x, 0.0, 3.0), SnapWeights::default())
            };
            assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
            scene.resolve(&q).map(|s| s.kind)
        };
        assert_eq!(release(2.1), Some(SnapKind::Endpoint));
        assert_eq!(release(2.4), None);
    }

    /// The point reach is for named points only: an edge 1.5 pick radii off
    /// the cursor, a metre from its midpoint and ends, is not admitted.
    #[test]
    fn point_reach_does_not_widen_an_edge() {
        let mut scene = InferenceScene::new();
        let a = Point3::new(3.0, -1.0, 3.0);
        let b = Point3::new(3.0, 3.0, 3.0);
        scene.add_sketch(SketchId::default(), &[(SketchEdgeId::default(), a, b)]);
        let on_edge = Point3::new(3.0, 2.0, 3.0);
        let mut x = 3.0;
        while long_drag_screen_angle(Point3::new(x, 2.0, 3.0), on_edge) < 1.5 * LONG_DRAG_APERTURE {
            x -= 1e-4;
        }
        let query = SnapQuery {
            anchor: None,
            ..long_drag_query(Point3::new(x, 2.0, 3.0), SnapWeights::default())
        };
        let snap = scene.resolve(&query);
        assert!(
            snap.is_none_or(|s| s.kind != SnapKind::OnEdge),
            "an edge 1.5 radii away must not capture, got {snap:?}"
        );
        let query = SnapQuery {
            anchor: None,
            ..long_drag_query(Point3::new(3.0 - 1e-4, 2.0, 3.0), SnapWeights::default())
        };
        assert_eq!(
            scene.resolve(&query).map(|s| s.kind),
            Some(SnapKind::OnEdge)
        );
    }

    /// `remove_guide` is idempotent and unregisters: a removed guide no
    /// longer snaps, and removing an unknown id is a no-op (no panic).
    #[test]
    fn remove_guide_is_idempotent_and_unregisters() {
        let mut scene = InferenceScene::new();
        let id = GuideId::default();
        // Placed well away from the world origin/axes (x=20,y=20) so the
        // tight aperture below can't pick up the ambient origin/axis
        // candidates once the guide itself is removed.
        scene.add_guide(
            id,
            &Guide::Point {
                position: Point3::new(20.0, 20.0, 1.0),
            },
        );
        assert_eq!(scene.guide_count(), 1);

        let query = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(20.0, 20.0, 10.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };
        assert!(scene.resolve(&query).is_some());

        scene.remove_guide(id);
        assert_eq!(scene.guide_count(), 0);
        let after = scene.resolve(&query);
        assert!(
            after.is_none(),
            "removed guide must no longer snap, got {:?}",
            after
        );

        // Idempotent: removing again (and removing an id that was never
        // registered) must not panic.
        scene.remove_guide(id);
        scene.remove_guide(GuideId::default());
    }

    // -----------------------------------------------------------------------
    // Phase B: sketch + transient candidates
    // -----------------------------------------------------------------------

    /// A registered sketch segment's endpoint and its midpoint each resolve
    /// as a snap along a ray through them, with `source: None` (sketch edges
    /// have no Object provenance; the midpoint carries `sketch_source`
    /// instead — see `sketch_segment_snaps_carry_sketch_provenance`).
    #[test]
    fn sketch_segment_endpoint_and_midpoint_snap() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let a = Point3::new(0.0, 0.0, 0.0);
        let b = Point3::new(2.0, 0.0, 0.0);
        scene.add_sketch(id, &[(SketchEdgeId::default(), a, b)]);

        // Ray straight at endpoint `b`.
        let endpoint_snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(2.0, 0.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("sketch endpoint is on this ray");
        assert_eq!(endpoint_snap.kind, SnapKind::Endpoint);
        assert!(endpoint_snap.position.approx_eq(b, tol::POINT_MERGE));
        assert!(endpoint_snap.source.is_none());

        // Ray straight at the midpoint (1, 0, 0).
        let mid_snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(1.0, 0.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("sketch midpoint is on this ray");
        assert_eq!(mid_snap.kind, SnapKind::Midpoint);
        assert!(
            mid_snap
                .position
                .approx_eq(Point3::new(1.0, 0.0, 0.0), tol::POINT_MERGE)
        );
        assert!(mid_snap.source.is_none());
    }

    /// A committed sketch segment's Midpoint and OnEdge snaps carry the
    /// owning `(SketchId, SketchEdgeId)` in `sketch_source`, so tools can
    /// use the edge as a reference (Tape Measure parallel guides). Endpoint
    /// snaps are vertex snaps and carry none; transient segments carry none.
    #[test]
    fn sketch_segment_snaps_carry_sketch_provenance() {
        let mut scene = InferenceScene::new();
        let sid = SketchId::default();
        let eid = SketchEdgeId::default();
        // y=3 (not 0): kept clear of the world X axis (tool-parity playtest2
        // §2c — `OnAxis` now outranks `OnEdge`) so this edge's on-edge point
        // doesn't coincidentally tie against the ambient origin-relative
        // axis candidate; this test is about sketch provenance, not axis
        // ranking.
        let a = Point3::new(0.0, 3.0, 0.0);
        let b = Point3::new(2.0, 3.0, 0.0);
        scene.add_sketch(sid, &[(eid, a, b)]);

        let query_at = |target: Point3| SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(target.x, target.y, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };

        // Midpoint: carries the sketch provenance.
        let mid = scene
            .resolve(&query_at(Point3::new(1.0, 3.0, 0.0)))
            .expect("midpoint on ray");
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert_eq!(mid.sketch_source, Some((sid, eid)));

        // On-edge (off-midpoint interior point): carries it too.
        let on_edge = scene
            .resolve(&query_at(Point3::new(0.5, 3.0, 0.0)))
            .expect("on-edge on ray");
        assert_eq!(on_edge.kind, SnapKind::OnEdge);
        assert_eq!(on_edge.sketch_source, Some((sid, eid)));

        // Endpoint: a vertex snap — no edge provenance.
        let endpoint = scene.resolve(&query_at(b)).expect("endpoint on ray");
        assert_eq!(endpoint.kind, SnapKind::Endpoint);
        assert_eq!(endpoint.sketch_source, None);

        // A transient segment's snaps carry none.
        let mut scene2 = InferenceScene::new();
        scene2.add_transient_segment(a, b);
        let t_mid = scene2
            .resolve(&query_at(Point3::new(1.0, 3.0, 0.0)))
            .expect("transient midpoint on ray");
        assert_eq!(t_mid.kind, SnapKind::Midpoint);
        assert_eq!(t_mid.sketch_source, None);
    }

    /// `remove_sketch` unregisters a sketch's candidates; removing an unknown
    /// id is a no-op.
    #[test]
    fn remove_sketch_unregisters_its_candidates() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let a = Point3::new(20.0, 20.0, 0.0);
        let b = Point3::new(22.0, 20.0, 0.0);
        scene.add_sketch(id, &[(SketchEdgeId::default(), a, b)]);

        let query = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(20.0, 20.0, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };
        assert!(scene.resolve(&query).is_some());

        scene.remove_sketch(id);
        assert!(
            scene.resolve(&query).is_none(),
            "removed sketch must no longer snap"
        );

        // Idempotent / unknown id is a no-op.
        scene.remove_sketch(id);
        scene.remove_sketch(SketchId::default());
    }

    /// A guide line crossing a sketch segment snaps as Intersection exactly
    /// at the crossing — the reason the guide was drawn. A real vertex at
    /// the same spot still outranks it, and hidden guides emit nothing.
    #[test]
    fn guide_crossing_a_sketch_segment_snaps_as_intersection() {
        let mut scene = InferenceScene::new();
        // Horizontal sketch segment y=1, x in 0..2.
        scene.add_sketch(
            SketchId::default(),
            &[(
                SketchEdgeId::default(),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(2.0, 1.0, 0.0),
            )],
        );
        // Vertical guide line through x = 0.5.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(0.5, 0.0, 0.0),
                direction: Vec3::new(0.0, 1.0, 0.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );

        let query_at = |x: f64, y: f64| SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(x, y, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.02,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };

        let snap = scene
            .resolve(&query_at(0.5, 1.0))
            .expect("the crossing snaps");
        assert_eq!(snap.kind, SnapKind::Intersection);
        assert!(
            snap.position
                .approx_eq(Point3::new(0.5, 1.0, 0.0), tol::POINT_MERGE)
        );

        // The segment's endpoint still outranks the intersection when the
        // guide passes through it.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(0.0, 0.0, 0.0),
                direction: Vec3::new(0.0, 1.0, 0.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );
        let at_vertex = scene
            .resolve(&query_at(0.0, 1.0))
            .expect("the vertex snaps");
        assert_eq!(at_vertex.kind, SnapKind::Endpoint);

        // Hidden guides emit neither OnGuide nor Intersection.
        scene.set_guides_enabled(false);
        let hidden = scene.resolve(&query_at(0.5, 1.0));
        assert!(hidden.is_none_or(|s| s.kind != SnapKind::Intersection));
    }

    /// Two crossing guide lines snap as Intersection at their crossing.
    #[test]
    fn crossing_guides_snap_as_intersection() {
        let mut scene = InferenceScene::new();
        // add_guide has replace semantics per id — mint two DISTINCT GuideIds
        // from a real Document so both guides coexist.
        let mut doc = kernel::Document::new();
        let g1 = doc
            .add_guide_line(Point3::new(0.3, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0))
            .expect("guide 1");
        let g2 = doc
            .add_guide_line(Point3::new(0.0, 0.7, 0.0), Vec3::new(1.0, 0.0, 0.0))
            .expect("guide 2");
        scene.add_guide(
            g1,
            &Guide::Line {
                origin: Point3::new(0.3, 0.0, 0.0),
                direction: Vec3::new(0.0, 1.0, 0.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );
        scene.add_guide(
            g2,
            &Guide::Line {
                origin: Point3::new(0.0, 0.7, 0.0),
                direction: Vec3::new(1.0, 0.0, 0.0),
                span: kernel::GuideSpan::INFINITE,
            },
        );

        let snap = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(0.3, 0.7, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.02,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("the guide crossing snaps");
        assert_eq!(snap.kind, SnapKind::Intersection);
        assert!(
            snap.position
                .approx_eq(Point3::new(0.3, 0.7, 0.0), tol::POINT_MERGE)
        );
    }

    /// `pick_sketch_edge` returns WHICH edge was hit, not just the sketch:
    /// two edges registered with distinct real ids resolve to the right one
    /// depending on where the ray points.
    #[test]
    fn pick_sketch_edge_distinguishes_edges_within_one_sketch() {
        // Mint two genuinely distinct SketchEdgeIds from a real kernel
        // sketch (slotmap keys — Default would alias them).
        let mut sk = kernel::Sketch::on_plane(
            kernel::Plane::from_polygon(&[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ])
            .unwrap(),
        );
        sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
            .unwrap();
        sk.add_segment(Point3::new(0.0, 5.0, 0.0), Point3::new(2.0, 5.0, 0.0))
            .unwrap();
        let ids: Vec<SketchEdgeId> = sk.edges().keys().collect();
        let [e_low, e_high] = [ids[0], ids[1]];
        let seg_of = |eid: SketchEdgeId| {
            let e = &sk.edges()[eid];
            (
                eid,
                sk.vertices()[e.from].position,
                sk.vertices()[e.to].position,
            )
        };

        let mut scene = InferenceScene::new();
        let sid = SketchId::default();
        scene.add_sketch(sid, &[seg_of(e_low), seg_of(e_high)]);

        let ray_at = |x: f64, y: f64| PickRay {
            origin: Point3::new(x, y, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        // Which id is which depends on which segment sits at y=0 vs y=5.
        let e = &sk.edges()[e_low];
        let low_is_y0 = sk.vertices()[e.from].position.y.abs() < 1e-9;
        let (y0_edge, y5_edge) = if low_is_y0 {
            (e_low, e_high)
        } else {
            (e_high, e_low)
        };

        assert_eq!(
            scene.pick_sketch_edge(&ray_at(1.0, 0.0), 0.05),
            Some((sid, y0_edge)),
        );
        assert_eq!(
            scene.pick_sketch_edge(&ray_at(1.0, 5.0), 0.05),
            Some((sid, y5_edge)),
        );
        assert_eq!(scene.pick_sketch_edge(&ray_at(50.0, 50.0), 0.05), None);
    }

    /// FIX A: a registered sketch region face is a first-class hoverable face
    /// — an interior ray snaps `OnFace` on it, and it occludes stronger
    /// candidates behind it (an `Endpoint` beyond the face) exactly like a
    /// solid's face, so the cursor no longer "passes through" a drawn region.
    #[test]
    fn sketch_region_face_snaps_on_face_and_occludes() {
        let mut scene = InferenceScene::new();
        let sid = SketchId::default();
        // A sketch segment with one endpoint BEHIND the region centre (0,0,0)
        // and one well outside its projection (3,0,0). Both are Endpoints (the
        // strongest kind) that would win outright but for occlusion. Register
        // it FIRST — `add_sketch` clears the sketch's faces (replace
        // semantics), matching production order where faces are added last.
        scene.add_sketch(
            sid,
            &[(
                SketchEdgeId::default(),
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(3.0, 0.0, 0.0),
            )],
        );
        // A unit square region on the plane z = 1 (normal +z).
        let rid = SketchRegionId::default();
        let plane =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        scene.add_sketch_faces(
            sid,
            &[SketchRegionFace {
                region: rid,
                plane,
                boundary: vec![
                    Point3::new(-1.0, -1.0, 1.0),
                    Point3::new(1.0, -1.0, 1.0),
                    Point3::new(1.0, 1.0, 1.0),
                    Point3::new(-1.0, 1.0, 1.0),
                ],
                holes: vec![],
            }],
        );

        // A ray straight down through the region's centre: the face occludes
        // the endpoint behind it, so the region's OnFace wins.
        let over_fill = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(0.0, 0.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("a snap over the region fill");
        assert_eq!(over_fill.kind, SnapKind::OnFace);
        assert!(
            over_fill
                .position
                .approx_eq(Point3::new(0.0, 0.0, 1.0), tol::POINT_MERGE),
            "snap landed at {:?}, expected the region plane z=1",
            over_fill.position
        );
        assert!(
            over_fill.source.is_none(),
            "a sketch region carries no Object provenance (source: None)"
        );
        assert_eq!(
            over_fill.sketch_region_source,
            Some((sid, rid)),
            "the OnFace snap carries the region's (sketch, region) provenance"
        );

        // A ray aimed at the (3,0,0) endpoint crosses z=1 at x=2.4 — OUTSIDE
        // the square — so the region neither snaps nor occludes there: the
        // endpoint still wins, proving occlusion is bounded to the polygon.
        let past_edge = scene
            .resolve(&SnapQuery {
                weights: SnapWeights::default(),
                ray: PickRay {
                    origin: Point3::new(0.0, 0.0, 5.0),
                    direction: Vec3::new(3.0, 0.0, -5.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                aperture_mode: ApertureMode::Cone,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
                see_through_points: false,
            })
            .expect("a snap past the region edge");
        assert_eq!(past_edge.kind, SnapKind::Endpoint);
        assert!(
            past_edge
                .position
                .approx_eq(Point3::new(3.0, 0.0, 0.0), tol::POINT_MERGE)
        );
    }

    /// `pick_sketch` returns the id of the sketch whose edge the ray passes
    /// nearest to, within the aperture; a ray that hits nothing returns `None`.
    #[test]
    fn pick_sketch_returns_the_nearest_sketch_within_aperture() {
        let mut scene = InferenceScene::new();
        let near = SketchId::default();
        scene.add_sketch(
            near,
            &[(
                SketchEdgeId::default(),
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(2.0, 0.0, 0.0),
            )],
        );

        let ray = PickRay {
            origin: Point3::new(1.0, 0.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        assert_eq!(scene.pick_sketch(&ray, 0.05), Some(near));

        // A ray well off to the side hits nothing within a tight aperture.
        let miss_ray = PickRay {
            origin: Point3::new(50.0, 50.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        assert_eq!(scene.pick_sketch(&miss_ray, 0.05), None);

        scene.remove_sketch(near);
        assert_eq!(
            scene.pick_sketch(&ray, 0.05),
            None,
            "removed sketch is no longer pickable"
        );
    }

    /// `pick_sketch_vertex` returns the exact `SketchVertexId` of the nearest
    /// registered sketch vertex within the aperture, and `None` after removal.
    #[test]
    fn pick_sketch_vertex_returns_the_nearest_vertex() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let mut sk = kernel::Sketch::on_plane(
            Plane::from_polygon(&[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ])
            .unwrap(),
        );
        sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
            .unwrap();
        let verts: Vec<_> = sk
            .vertices()
            .iter()
            .map(|(vid, v)| (vid, v.position))
            .collect();
        scene.add_sketch_vertices(id, &verts);
        let target = verts
            .iter()
            .find(|(_, p)| p.approx_eq(Point3::new(2.0, 0.0, 0.0), tol::POINT_MERGE))
            .map(|(vid, _)| *vid)
            .unwrap();

        // A ray straight down onto the (2,0,0) corner picks that exact vertex.
        let ray = PickRay {
            origin: Point3::new(2.0, 0.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        let hit = scene.pick_sketch_vertex(&ray, 0.05).expect("vertex on ray");
        assert_eq!((hit.0, hit.1), (id, target));
        assert!(
            hit.2
                .approx_eq(Point3::new(2.0, 0.0, 0.0), tol::POINT_MERGE)
        );

        // A ray down the middle of the edge (1,0,0) is too far from any vertex.
        let mid_ray = PickRay {
            origin: Point3::new(1.0, 0.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        assert_eq!(scene.pick_sketch_vertex(&mid_ray, 0.05), None);

        scene.remove_sketch(id);
        assert_eq!(scene.pick_sketch_vertex(&ray, 0.05), None);
    }

    /// A transient segment's endpoint snaps like a sketch segment's;
    /// `clear_transient` removes it.
    #[test]
    fn transient_segment_endpoint_snaps_and_clears() {
        let mut scene = InferenceScene::new();
        let a = Point3::new(5.0, 5.0, 0.0);
        let b = Point3::new(7.0, 5.0, 0.0);
        scene.add_transient_segment(a, b);

        let query = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(7.0, 5.0, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
            see_through_points: false,
        };
        let snap = scene
            .resolve(&query)
            .expect("transient endpoint is on this ray");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(snap.position.approx_eq(b, tol::POINT_MERGE));
        assert!(snap.source.is_none());

        scene.clear_transient();
        assert!(
            scene.resolve(&query).is_none(),
            "cleared transient segment must no longer snap"
        );
    }
}
