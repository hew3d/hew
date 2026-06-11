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
//! [`SnapKind`] wins (the enum's declaration order IS the priority order,
//! strongest first — `SnapKind`'s `Ord` reflects it and tools may rely on
//! that). Among candidates of equal kind, the one nearest the ray wins; ties
//! break toward the one nearest the ray origin (closest to camera).
//!
//! # Locking
//!
//! A [`SnapLock`] (shift-lock or arrow keys in SketchUp terms) constrains the
//! result to a line through the query's `anchor`. With a lock active, every
//! candidate is projected onto the locked line before ranking, and the
//! returned snap keeps the candidate's `kind`/`source` so the UI can still
//! say *why* (e.g. "on axis, from endpoint").
//!
//! M0 status: the complete query API with real types; `add_object`,
//! `remove_object`, and `resolve` are `todo!()` stubs. Implementation
//! brings a spatial index behind `InferenceScene` — the API deliberately
//! hides it so the index strategy can change freely.

use kernel::{EdgeId, FaceId, Object, ObjectId, Plane, Point3, Transform, Vec3, VertexId};

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
    /// On the midpoint of an edge.
    Midpoint,
    /// On the apparent intersection of two edges.
    Intersection,
    /// Anywhere along an edge.
    OnEdge,
    /// Anywhere on a face.
    OnFace,
    /// On a model axis (or locked direction) through the anchor.
    OnAxis,
    /// Direction parallel to a reference edge (M2; needs a reference).
    Parallel,
    /// Direction perpendicular to a reference edge (M2; needs a reference).
    Perpendicular,
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
    /// The owning Object in the document.
    pub object: ObjectId,
    /// The element within that Object.
    pub element: ElementRef,
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
    /// The inference direction for directional snaps (axis / parallel /
    /// perpendicular), for drawing the dashed guide line.
    pub direction: Option<Vec3>,
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
    /// Pick-cone half-angle in radians. The UI computes it from its snap
    /// radius in pixels and the camera FOV, keeping this crate
    /// screen-agnostic.
    pub aperture: f64,
}

/// A snappable point with provenance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenePoint {
    /// World-space position.
    pub position: Point3,
    /// Where it came from.
    pub source: SnapSource,
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
    /// Where it came from.
    pub source: SnapSource,
}

/// The engine's view of the scene: world-space snap candidates extracted
/// from every Object, refreshed incrementally as Objects change.
///
/// The spatial index lives behind this type; the public API exposes
/// only candidates and queries so the indexing strategy stays swappable.
#[derive(Debug, Clone, Default)]
pub struct InferenceScene {
    points: Vec<ScenePoint>,
    segments: Vec<SceneSegment>,
    faces: Vec<SceneFace>,
}

impl InferenceScene {
    /// An empty scene.
    pub fn new() -> InferenceScene {
        InferenceScene::default()
    }

    /// Candidate counts as (points, segments, faces) — cheap introspection
    /// for tests and debug overlays.
    pub fn candidate_counts(&self) -> (usize, usize, usize) {
        (self.points.len(), self.segments.len(), self.faces.len())
    }

    /// Extracts snap candidates from `object` (vertices, edges with
    /// midpoints derived at query time, faces) transformed by `placement`
    /// into world space, replacing any candidates previously registered for
    /// `id`.
    ///
    /// Cost model: linear in the Object's elements; called on Object
    /// creation and after each committed mutation, never per-frame.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn add_object(&mut self, id: ObjectId, object: &Object, placement: &Transform) {
        todo!("M1: extract world-space candidates and (re)index them")
    }

    /// Drops all candidates registered for `id`. Unknown ids are a no-op —
    /// removal must be idempotent so document undo can call it freely.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn remove_object(&mut self, id: ObjectId) {
        todo!("M1: drop candidates for the object")
    }

    /// Answers one inference query (see the module docs for the priority and
    /// locking model). Returns `None` when nothing falls inside the pick
    /// cone and no lock/anchor produces a directional snap — the tool then
    /// uses its own fallback (e.g. ground-plane intersection).
    ///
    /// Must be cheap enough to call on every mouse-move at interactive
    /// rates; that budget is what the spatial index exists for.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn resolve(&self, query: &SnapQuery) -> Option<Snap> {
        todo!("M1: cone query against the spatial index + priority ranking")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_kind_priority_is_declaration_order() {
        // Strongest first; tools sort by this.
        assert!(SnapKind::Endpoint < SnapKind::Midpoint);
        assert!(SnapKind::Midpoint < SnapKind::Intersection);
        assert!(SnapKind::Intersection < SnapKind::OnEdge);
        assert!(SnapKind::OnEdge < SnapKind::OnFace);
        assert!(SnapKind::OnFace < SnapKind::OnAxis);
        assert!(SnapKind::OnAxis < SnapKind::Parallel);
        assert!(SnapKind::Parallel < SnapKind::Perpendicular);
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
}
