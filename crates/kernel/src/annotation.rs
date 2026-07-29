//! Dimension and leader-text annotations (docs/design/dimensions-text.md): a
//! guides-style side collection — non-solid, non-topological document
//! entities that annotate the model without ever affecting watertightness.
//! Mirrors [`crate::guide::Guide`]'s role: the kernel owns the geometry
//! (anchors, offsets, captured analytics); the *displayed* measurement text
//! is computed app-side from the anchors and the document's units
//! (`text_override` replaces it when the user has overridden it).
//!
//! ## Anchoring and re-anchoring
//!
//! An [`Anchor`] names the node it was picked on (`None` for a free-floating
//! point) plus the world-space point itself. Full topological associativity
//! (a durable reference into a face/edge/vertex that survives a rebuild)
//! isn't something Hew's rebuild-heavy kernel ops offer yet, so annotations
//! use **geometric re-anchoring** instead (approved design, see the design
//! doc's "Associativity" section): [`crate::document::Document`] carries an
//! anchor's point through the exact world-space map of a rigid/affine bake
//! or instance-pose change on its node (`Document::reanchor_touched`), and
//! flags an annotation `detached` when its node is deleted or consumed
//! (`Document::reevaluate_liveness_recorded`), when a captured
//! [`Annotation::RadialDimension`] circle can't survive a non-similarity
//! map, or when a [`Annotation::LinearDimension`] with only one anchor
//! riding the touched transform can't re-derive a sane placement (its two
//! points coincide, or the retained offset is collinear with the new
//! baseline). In every one of these cases the whole record is frozen at its
//! pre-transform values rather than partially written — "frozen but
//! coherent". `detached` is **stored**, not derived: it is fundamentally an
//! event record (was the last mutation touching this node one this
//! annotation could follow) that current geometry alone can't always
//! reconstruct — see `Document::reanchor_touched`'s doc comment for why a
//! non-similarity map's detach specifically can't be re-derived at undo
//! time.
//!
//! A detached annotation is never auto-repaired by a later mutation: once
//! set, only an explicit [`crate::document::Document::update_annotation`]
//! call that actually changes an anchor/geometry field (the user re-picking
//! anchors) clears it — a text-only edit through the same method (typing a
//! `text_override`, or a leader's `text`) leaves it exactly as it was; see
//! that method's doc comment.
//!
//! `Document::make_component`/`Document::explode_instance` consume a world
//! object/instance node into (or out of) a shared definition, which is
//! exactly the same kind of liveness-killing event `delete_node` and the
//! operand-consuming ops are — an annotation anchored to the killed node
//! detaches the same way. Neither op REMAPS such an anchor onto the new
//! node that took over its geometry (the fresh instance a fold creates, or
//! the baked object an explode creates); that is future work. Until it
//! lands, folding/exploding a node an annotation is anchored to always
//! detaches rather than silently continuing to track the wrong node.

use crate::document::NodeId;
use crate::math::{Plane, Point3, Vec3};

/// Where an annotation attaches: the node whose geometry re-anchoring tracks
/// (`None` for a free-floating anchor, which is never re-anchored and never
/// detaches), plus the actual point, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// The node this anchor tracks, or `None` if it names a bare point in
    /// space.
    pub node: Option<NodeId>,
    /// The anchor's position, in world space.
    pub point: Point3,
}

/// Radius vs. diameter presentation of a [`Annotation::RadialDimension`].
/// Purely a display choice over the same captured circle — it does not
/// change `curve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialKind {
    /// Displayed as a radius (center to circumference).
    Radius,
    /// Displayed as a diameter (circumference to circumference through the
    /// center).
    Diameter,
}

/// The analytic circle/arc a [`Annotation::RadialDimension`] measures,
/// captured at creation time (Hew's true curves make this exact, rather
/// than measured off a tessellated mesh approximation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapturedCurve {
    /// Circle center, in world space.
    pub center: Point3,
    /// Circle radius, in meters (> 0).
    pub radius: f64,
    /// The plane the circle lies in.
    pub plane: Plane,
}

/// A dimension or leader-text annotation: a guides-style document entity
/// (see the module doc comment). Kernel-owned geometry only — no rendered
/// text layout; `text_override`, where present, replaces the app-computed
/// measurement string verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    /// A linear dimension between two anchors, with the dimension line
    /// offset out of the anchors' line by `offset` (SketchUp's drag-out
    /// gesture) and drawn in `plane`.
    LinearDimension {
        /// The dimension's first endpoint.
        a: Anchor,
        /// The dimension's second endpoint.
        b: Anchor,
        /// Placement offset of the dimension line from the `a`-`b` line, in
        /// world space.
        offset: Vec3,
        /// The plane the dimension line and extension lines are drawn in.
        plane: Plane,
        /// Replaces the computed measurement text when present.
        text_override: Option<String>,
    },
    /// A radius/diameter dimension measuring an analytic circle or arc,
    /// captured at creation.
    RadialDimension {
        /// The point the leader points to (on the circle/arc).
        anchor: Anchor,
        /// Radius vs. diameter presentation.
        kind: RadialKind,
        /// The captured analytic circle/arc.
        curve: CapturedCurve,
        /// Direction of the leader line from `anchor`, in world space.
        leader_dir: Vec3,
        /// Replaces the computed measurement text when present.
        text_override: Option<String>,
    },
    /// Free-form leader text: an anchor, a leader-line offset to where the
    /// text sits, and its content.
    LeaderText {
        /// The point the leader points to.
        anchor: Anchor,
        /// Placement offset of the text from `anchor`, in world space.
        offset: Vec3,
        /// The text content (kernel stores it verbatim; layout/rendering is
        /// app-side).
        text: String,
    },
}

impl Annotation {
    /// Every node this annotation's anchors name (`Anchor.node == Some(_)`),
    /// in a fixed order. Not deduplicated — callers only need
    /// liveness/membership checks.
    pub(crate) fn anchored_nodes(&self) -> Vec<NodeId> {
        match self {
            Annotation::LinearDimension { a, b, .. } => {
                [a.node, b.node].into_iter().flatten().collect()
            }
            Annotation::RadialDimension { anchor, .. } | Annotation::LeaderText { anchor, .. } => {
                anchor.node.into_iter().collect()
            }
        }
    }

    /// True if any of this annotation's anchors names a node in `nodes`.
    pub(crate) fn touches_any(&self, nodes: &[NodeId]) -> bool {
        self.anchored_nodes().iter().any(|n| nodes.contains(n))
    }

    /// Whether `self` and `other` (already known to be the same variant —
    /// callers match on discriminant first) are identical apart from their
    /// display-text field (`text_override` on a dimension, `text` on a
    /// leader). Used by [`crate::document::Document::update_annotation`] to
    /// tell a text-only edit (Tab-toggling `<>` back to the app-computed
    /// measurement, or typing an override) apart from one that actually
    /// re-picks anchors/geometry — only the latter re-asserts "this
    /// placement is fresh" and clears a stale `detached` warning; see that
    /// method's doc comment.
    ///
    /// Whole-value compare, not a hand-picked field list: normalize away
    /// just the text field on a clone of each side, then compare the rest
    /// with derived [`PartialEq`] — so a future field added to any variant
    /// is geometry by default (clears `detached` on change) unless it is
    /// explicitly folded into the text-only side here.
    pub(crate) fn geometry_eq(&self, other: &Annotation) -> bool {
        fn without_display_text(mut annotation: Annotation) -> Annotation {
            match &mut annotation {
                Annotation::LinearDimension { text_override, .. }
                | Annotation::RadialDimension { text_override, .. } => *text_override = None,
                Annotation::LeaderText { text, .. } => text.clear(),
            }
            annotation
        }
        without_display_text(self.clone()) == without_display_text(other.clone())
    }

    /// Carries this annotation's anchors through the exact world-space map
    /// `t` for every anchor whose node is in `touched` — see
    /// `Document::reanchor_touched` for the full contract. Returns `None`
    /// when nothing here references a touched node (the caller leaves
    /// `detached` alone); otherwise `Some(ok)`, where `ok` is `false` when
    /// this annotation's record cannot be carried through `t` coherently —
    /// a [`Annotation::RadialDimension`] whose captured circle cannot
    /// survive a non-similarity `t`, or a [`Annotation::LinearDimension`]
    /// whose dimension-line placement cannot be re-derived (see below). In
    /// every `Some(false)` case the whole record is left at its
    /// pre-transform values — "frozen but coherent", never a partial write.
    pub(crate) fn reanchor(
        &mut self,
        touched: &[NodeId],
        t: &crate::transform::Transform,
    ) -> Option<bool> {
        let on = |n: Option<NodeId>| n.is_some_and(|n| touched.contains(&n));
        match self {
            Annotation::LinearDimension {
                a,
                b,
                offset,
                plane,
                ..
            } => {
                let (ta, tb) = (on(a.node), on(b.node));
                if !ta && !tb {
                    return None;
                }
                if ta && tb {
                    // Both anchors ride the SAME touched transform under
                    // one `t` — the rigid-assembly case. `offset`/`plane`
                    // describe the dimension line's placement relative to
                    // `a`/`b`; a single rigid/affine map applied to the
                    // whole assembly preserves that relationship exactly,
                    // so carrying the whole record through `t` verbatim
                    // stays exact.
                    a.point = t.apply_point(a.point);
                    b.point = t.apply_point(b.point);
                    *offset = t.apply_vector(*offset);
                    *plane = t
                        .apply_plane(plane)
                        .expect("t was already validated invertible before this call");
                    return Some(true);
                }
                // Exactly one anchor moved. Carrying `offset`/`plane`
                // through `t` here would rotate the dimension-line
                // placement even when the OTHER anchor sits still and the
                // touched one only rotated about its own point (both
                // endpoints numerically unchanged) — a placement change
                // with no geometric cause. Instead: map the moved anchor's
                // point through `t`, keep the other anchor exactly, and
                // re-derive `offset`/`plane` from the two (possibly new)
                // points plus the OLD offset projected perpendicular to
                // the new a-b line (what a placement vector must be).
                let old_offset = *offset;
                let new_a = if ta { t.apply_point(a.point) } else { a.point };
                let new_b = if tb { t.apply_point(b.point) } else { b.point };
                let derived = (new_b - new_a).normalized().ok().and_then(|dir| {
                    let perp = old_offset - dir * old_offset.dot(dir);
                    let normal = dir.cross(perp).normalized().ok()?;
                    Plane::from_point_normal(new_a, normal)
                        .ok()
                        .map(|plane| (perp, plane))
                });
                match derived {
                    Some((new_offset, new_plane)) => {
                        a.point = new_a;
                        b.point = new_b;
                        *offset = new_offset;
                        *plane = new_plane;
                        Some(true)
                    }
                    // Degenerate: the two points coincide (no direction),
                    // or the retained offset is collinear with the new a-b
                    // line (nothing perpendicular survives to derive a
                    // plane from). Freeze the WHOLE record at its
                    // pre-transform values — coherent, not garbage — same
                    // posture as `RadialDimension`'s non-similarity detach
                    // below.
                    None => Some(false),
                }
            }
            Annotation::RadialDimension {
                anchor,
                curve,
                leader_dir,
                ..
            } => {
                if !on(anchor.node) {
                    return None;
                }
                match t.similarity_scale() {
                    Some(scale) => {
                        anchor.point = t.apply_point(anchor.point);
                        *leader_dir = t.apply_vector(*leader_dir);
                        curve.center = t.apply_point(curve.center);
                        curve.radius *= scale;
                        curve.plane = t
                            .apply_plane(&curve.plane)
                            .expect("t was already validated invertible before this call");
                        Some(true)
                    }
                    // A non-similarity map (squash/shear) would turn the
                    // circle into an ellipse, which `CapturedCurve` cannot
                    // represent. Freeze the WHOLE annotation coherently —
                    // `anchor`, `leader_dir`, AND `curve` all keep their
                    // pre-transform values — rather than moving the leader
                    // to point at a stale circle from a new, unrelated
                    // position (an internally inconsistent record).
                    // `detached` is the signal; nothing here moves.
                    None => Some(false),
                }
            }
            Annotation::LeaderText { anchor, offset, .. } => {
                if !on(anchor.node) {
                    return None;
                }
                anchor.point = t.apply_point(anchor.point);
                *offset = t.apply_vector(*offset);
                Some(true)
            }
        }
    }
}
