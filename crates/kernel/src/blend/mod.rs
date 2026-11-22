//! The blend engine (true-curves.md §3.4, §4): fillets and chamfers as
//! local surgery on the facet carrier. A blend is planned (pure), then
//! applied through the shared patch primitive on a clone, validated and
//! guarded against obstruction before it is returned.
//!
//! Pipeline: classify each picked edge ([`classify`]), group edges into
//! collinear chains with pass-through stations ([`chain`]), solve each
//! station's 2D section ([`section`]), build one construction per chain end
//! vertex ([`vertex`], with the general convex clip in [`clip`]), assemble a
//! patch and check it ([`plan`]), and find the largest size that fits by
//! bisection on plan validity ([`limit`], decision I-8).

mod chain;
mod classify;
mod clip;
mod derive;
mod edit;
mod error;
mod limit;
mod plan;
mod profile;
mod query;
mod record;
mod restore;
mod section;
mod station;
mod swept;
mod vertex;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use edit::{BlendFeature, same_geometry_for_test};
pub use error::{BlendError, LimitFeature};
pub use limit::{limit_attempts_for_test, limit_probes_for_test, limit_searches_for_test};
pub use profile::{ProfileSample, bundled};
pub use record::{BlendRecord, LoopEdit, StoredFace, StoredMark, StoredSpec};

use crate::ids::{EdgeId, FaceId};
use crate::tol;
use crate::topo::{BlendId, Object};

thread_local! {
    /// Whether the blend is a preview (a Fillet tool following the pointer).
    static PREVIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The id the next blend takes instead of a fresh one: a blend put back
    /// after a restoration keeps the id it had.
    static FORCED_ID: std::cell::Cell<Option<BlendId>> = const { std::cell::Cell::new(None) };
    /// Ids a fresh blend must stay clear of: those of blends taken off
    /// that will be put back under their own ids once the edit is made.
    static RESERVED_IDS: std::cell::RefCell<Vec<BlendId>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Whether a preview records its blends anyway: an edit that puts later
    /// blends back may restore the blend it just made, which replays its
    /// record (without one, a stopped blend cannot be restored).
    static RECORD_IN_PREVIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether an attempt takes the facet planners alone, without the swept
    /// retry (test support: the catalog's own refusal).
    static CATALOG_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with blends recorded even in a preview (see `RECORD_IN_PREVIEW`).
pub(crate) fn with_records<T>(f: impl FnOnce() -> T) -> T {
    let before = RECORD_IN_PREVIEW.with(|c| c.replace(true));
    let out = f();
    RECORD_IN_PREVIEW.with(|c| c.set(before));
    out
}

/// Keeps `ids` reserved for as long as it lives (nested reservations
/// stack): a fresh blend id is one past them as well as past the ids on
/// the object.
pub(crate) struct ReservedBlendIds {
    count: usize,
}

impl ReservedBlendIds {
    pub(crate) fn new(ids: impl IntoIterator<Item = BlendId>) -> Self {
        let mut count = 0;
        RESERVED_IDS.with(|r| {
            for id in ids {
                r.borrow_mut().push(id);
                count += 1;
            }
        });
        ReservedBlendIds { count }
    }
}

impl Drop for ReservedBlendIds {
    fn drop(&mut self) {
        RESERVED_IDS.with(|r| {
            let mut v = r.borrow_mut();
            let keep = v.len().saturating_sub(self.count);
            v.truncate(keep);
        });
    }
}

/// Whether a preview is being built (see [`as_preview`]).
/// Whether attempts take the facet planners alone (test support).
pub(crate) fn is_catalog_only() -> bool {
    CATALOG_ONLY.with(std::cell::Cell::get)
}

pub(crate) fn is_preview() -> bool {
    PREVIEW.with(|p| p.get())
}

/// The id forced on the next blend, if any.
pub(crate) fn forced_blend_id() -> Option<BlendId> {
    FORCED_ID.with(|c| c.get())
}

/// Runs `f` with `id` as the id the next blend takes (`None`: a fresh one),
/// and restores whatever was forced before.
pub(crate) fn with_forced_blend_id<T>(id: Option<BlendId>, f: impl FnOnce() -> T) -> T {
    let before = FORCED_ID.with(|c| c.replace(id));
    let out = f();
    FORCED_ID.with(|c| c.set(before));
    out
}

impl Object {
    /// The id a blend being built takes: the forced one (a blend put back
    /// after a restoration, or the second pass of a two-pass corner), else
    /// one past the largest id present on the object's faces, in its
    /// records, or reserved for a blend about to be put back.
    pub(crate) fn new_blend_id(&self) -> BlendId {
        if let Some(id) = forced_blend_id() {
            return id;
        }
        let past = |m: Option<u32>| m.map_or(1, |m| m.saturating_add(1));
        let faces = self.next_blend_id().0;
        let records = past(self.blend_records().keys().map(|id| id.0).max());
        let reserved = RESERVED_IDS.with(|r| past(r.borrow().iter().map(|id| id.0).max()));
        BlendId(faces.max(records).max(reserved))
    }
}

/// Runs `f` as a preview: an imported mesh's refusal does not search for
/// the size that rolls, and a swept chain's surface solve takes one round,
/// so a size that does not fit answers about as fast as one that does (a
/// commit searches; a preview is asked again on every pointer move).
pub(crate) fn as_preview<T>(f: impl FnOnce() -> T) -> T {
    as_preview_with_rounds(1, f)
}

/// [`as_preview`] for a typed size (D-119): the swept blend takes the
/// commit's rounds, so a typed size previews exactly as it commits; only
/// the size searches stay a commit's.
pub(crate) fn as_typed_preview<T>(f: impl FnOnce() -> T) -> T {
    as_preview_with_rounds(swept::AVOID_ROUNDS, f)
}

fn as_preview_with_rounds<T>(rounds: usize, f: impl FnOnce() -> T) -> T {
    let before = PREVIEW.with(|p| p.replace(true));
    let out = swept::with_avoid_rounds(rounds, f);
    PREVIEW.with(|p| p.set(before));
    out
}

/// How close [`Object::largest_that_rolls`], and the limit search's scan
/// under a refusal on an imported mesh, bring their limit, as a fraction of
/// the size.
const SWEPT_LIMIT_PRECISION: f64 = 0.1;

/// One blend request: the edges and the profile. Ids are kernel handles
/// valid against the object the spec is applied to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlendSpec {
    /// Kernel edges to blend, already expanded by the caller (logical edge /
    /// tangent chain). The kernel blends exactly these; order is irrelevant.
    #[serde(with = "crate::ids::serde_ffi_vec")]
    pub edges: Vec<EdgeId>,
    /// The cross-section.
    pub profile: BlendProfile,
    /// Asymmetric chamfer only: the face that receives `setback_a` on every
    /// edge it borders. `None` → for each edge, side A is the face of the
    /// edge's `half_edge`.
    #[serde(default, with = "crate::ids::serde_ffi_opt")]
    pub primary: Option<FaceId>,
    /// How three-way corners are built.
    #[serde(default)]
    pub corners: CornerStyle,
    /// Stopped blends: distances from each chain end.
    #[serde(default)]
    pub stops: Option<BlendStops>,
    /// Variable radius handles.
    #[serde(default)]
    pub handles: Vec<RadiusHandle>,
}

impl BlendSpec {
    /// The spec at primary size `size`: the profile scaled to it and the
    /// radius handles with it, as the limit search tries sizes (unchanged
    /// for a profile with no size).
    pub(crate) fn at_size(&self, size: f64) -> BlendSpec {
        let Some(s0) = self.profile.size().filter(|&s| s > 0.0) else {
            return self.clone();
        };
        let k = size / s0;
        let mut out = self.clone();
        out.profile = self.profile.scaled(k);
        for h in &mut out.handles {
            h.radius *= k;
        }
        out
    }
}

/// A blend cross-section. Sizes are meters; `segments` is the facet count
/// across a round's arc (2..=96).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlendProfile {
    /// A rolling-ball fillet of `radius`.
    Round {
        /// Ball radius.
        radius: f64,
        /// Facets across the arc.
        segments: u32,
    },
    /// A flat chamfer with a setback on each face.
    Chamfer {
        /// Setback on side A (the primary face).
        setback_a: f64,
        /// Setback on side B.
        setback_b: f64,
    },
    /// A concave quarter-round cut into a convex edge.
    Cove {
        /// Cove radius.
        radius: f64,
        /// Facets across the arc.
        segments: u32,
    },
    /// A full half-round consuming the face between two parallel edges.
    Bullnose {
        /// Facets across the half circle.
        segments: u32,
    },
    /// A router profile registered against the two faces.
    Custom {
        /// The normalized shape.
        shape: ProfileShape,
        /// Width on the bearing face (meters).
        width: f64,
        /// Facets per quarter turn of arc.
        arc_segments: u32,
    },
}

impl BlendProfile {
    /// The profile's primary length, which the size limit scales: the
    /// radius of a round or cove, `setback_a` of a chamfer, the width of a
    /// custom profile. A bullnose has no size (`None`).
    pub fn size(&self) -> Option<f64> {
        match self {
            BlendProfile::Round { radius, .. } | BlendProfile::Cove { radius, .. } => Some(*radius),
            BlendProfile::Chamfer { setback_a, .. } => Some(*setback_a),
            BlendProfile::Custom { width, .. } => Some(*width),
            BlendProfile::Bullnose { .. } => None,
        }
    }

    /// The profile with its size scaled by `k` (a chamfer scales both
    /// setbacks by one factor).
    pub fn scaled(&self, k: f64) -> BlendProfile {
        match self.clone() {
            BlendProfile::Round { radius, segments } => BlendProfile::Round {
                radius: radius * k,
                segments,
            },
            BlendProfile::Cove { radius, segments } => BlendProfile::Cove {
                radius: radius * k,
                segments,
            },
            BlendProfile::Chamfer {
                setback_a,
                setback_b,
            } => BlendProfile::Chamfer {
                setback_a: setback_a * k,
                setback_b: setback_b * k,
            },
            BlendProfile::Custom {
                shape,
                width,
                arc_segments,
            } => BlendProfile::Custom {
                shape,
                width: width * k,
                arc_segments,
            },
            b @ BlendProfile::Bullnose { .. } => b,
        }
    }
}

/// How a three-way corner is built (true-curves.md §4.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CornerStyle {
    /// Rolled fillet corners, mitered chamfer apexes.
    #[default]
    Default,
    /// Three fillets meet in miter curves.
    Mitered,
    /// Three chamfers meet in one more flat triangle.
    Flat,
}

/// Whether an edge is convex (material inside the dihedral) or concave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convexity {
    /// The dihedral through the material is under π: a blend removes
    /// material.
    Convex,
    /// The dihedral through the material is over π: a blend adds material.
    Concave,
}

/// A stopped blend's distances from each end of its chain (meters, ≥ 0).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlendStops {
    /// Distance from the chain's start.
    pub start: f64,
    /// Distance from the chain's end.
    pub end: f64,
}

/// A variable-radius handle: a radius at an arc length along the chain.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RadiusHandle {
    /// Arc length from the chain start (meters).
    pub at: f64,
    /// Radius there (meters).
    pub radius: f64,
}

/// A router-style edge profile in NORMALIZED coordinates: corner at (0,0);
/// face A (the bearing face) is the line v = 0 with material at v ≥ 0; face
/// B is the line u = 0 with material at u ≥ 0. The chain starts at (1, 0) on
/// face A and ends at (0, b) on face B. The region between the corner and
/// the chain is cut away (convex edge) or filled (concave edge). Width = 1
/// unit = the setback on face A.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileShape {
    /// End point on face B is (0, b); b > 0.
    pub b: f64,
    /// The segments from (1, 0) to (0, b).
    pub segs: Vec<ProfileSeg>,
}

/// One segment of a [`ProfileShape`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileSeg {
    /// A straight segment from the previous point to `to`.
    Line {
        /// End point.
        to: [f64; 2],
    },
    /// A circular arc from the previous point to `to` about `center`;
    /// `ccw` = counter-clockwise in (u, v).
    Arc {
        /// End point.
        to: [f64; 2],
        /// Arc center.
        center: [f64; 2],
        /// Counter-clockwise in (u, v).
        ccw: bool,
    },
}

/// What a blend produced.
#[derive(Debug, Clone, PartialEq)]
pub struct BlendReport {
    /// One id per pass (two-pass corners give two).
    pub blends: Vec<BlendId>,
    /// The new blend faces, in the result object.
    pub new_faces: Vec<FaceId>,
    /// The sizes actually applied.
    pub profile: BlendProfile,
    /// Picked edges that survived as different `EdgeId`s (two-pass corners,
    /// re-solve).
    pub edge_map: BTreeMap<EdgeId, EdgeId>,
}

/// The largest size that fits and what limits it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendLimit {
    /// The largest size (the profile's primary length) that blends.
    pub max: f64,
    /// The feature that stops it growing (`Unknown` when the requested size
    /// already fits).
    pub limited_by: LimitFeature,
}

impl Object {
    /// Blends `spec.edges` with `spec.profile` on a clone of `self` and
    /// returns the validated result and its report; `self` is unchanged.
    ///
    /// Plans the blend (classification, chains, sections, vertex
    /// constructions, checks), applies it with the shared patch primitive,
    /// then refuses `Obstructed` if any new or modified face passes through
    /// other geometry of the object.
    ///
    /// # Errors
    /// [`BlendError`]: `UnknownEdge`, `EmptySpec`, `BlendNeedsSolid`,
    /// `EdgeIsSmooth`, `UnsupportedEdge`, `UnsupportedVertex`,
    /// `BlendFacesFlush` (the edge's faces meet tangentially at a vertex),
    /// `SegmentsOutOfRange`, `RadiusTooLarge { max, limited_by }` (with the
    /// tight maximum from [`Object::max_blend_size`]), `Obstructed`, and
    /// `BlendDegenerate` (a kernel bug channel).
    ///
    /// Where the spec's edges end at an existing blend, that blend is
    /// restored and re-applied with the new one, larger sizes first (corner
    /// re-solve, WP08 §4), so the result does not depend on the order the
    /// blends were made in.
    pub fn blend_edges(&self, spec: &BlendSpec) -> Result<(Object, BlendReport), BlendError> {
        self.blend_preview_resolved(spec)
            .map(|(after, report, _)| (after, report))
    }

    /// What [`Object::blend_edges`] produces, with the faces of `self` the
    /// result replaces (for previews): the corner re-solve where the spec's
    /// edges run into existing blends, else (or where the re-solve fails)
    /// the plain blend. Where both fail and the plain planner refuses the
    /// corner outright (`UnsupportedVertex`; any other hard refusal stands)
    /// while the re-solve only ran out of room, and a smaller size builds,
    /// the refusal
    /// is the re-solve's, with the limit [`Object::max_blend_size`] finds
    /// (the same re-solve), so a preview clamps there.
    pub(crate) fn blend_preview_resolved(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), BlendError> {
        let resolved = match edit::CornerResolve::prepare(self, spec) {
            None => None,
            Some(corners) => match corners.run(spec, |o, s| o.blend_pass(s)) {
                Ok((after, report)) => {
                    let replaced = self
                        .faces()
                        .keys()
                        .filter(|f| !after.faces().contains_key(*f))
                        .collect();
                    return Ok((after, report, replaced));
                }
                Err(f) => Some(f),
            },
        };
        match self.blend_attempt(spec) {
            Ok(parts) => Ok(self.recorded(parts, spec)),
            Err(plain) => Err(self.named_flush(
                spec,
                match (resolved, plain.0) {
                    (
                        Some(f @ (plan::PlanFail::Size(_) | plan::PlanFail::Obstructed(_))),
                        plan::PlanFail::Hard(BlendError::UnsupportedVertex { .. }),
                    ) => match self.refusal(spec, f, true).unwrap_or_else(|e| e) {
                        // No size builds the corner either way: the planner's
                        // refusal is the truer answer.
                        BlendError::RadiusTooLarge { max, .. } if max <= 0.0 => {
                            self.refuse(spec, plain, true)
                        }
                        e => e,
                    },
                    _ => self.refuse(spec, plain, true),
                },
            )),
        }
    }

    /// `e`, or [`BlendError::BlendFacesFlush`] where `e` refuses a vertex
    /// at which the spec's edges have their two faces tangent (D-234): two
    /// pipes of one radius in a tee meet flush past the branch's sides, so
    /// no ball fits there at any size, and "can't blend this corner" would
    /// send the user hunting for a corner.
    ///
    /// Flush means, for every edge of the spec that ends at the vertex (the
    /// planner's refusal does not name the edge, so one flush edge beside a
    /// refused corner is not enough; D-250): both faces carry a claimed
    /// surface whose normal there is defined, and those normals, each
    /// turned to its face's outward side, agree within
    /// [`tol::BLEND_FLUSH_SIN`] and point the same way. A razor wedge's
    /// walls (normals opposite) and a scan's shallow crease between
    /// unclaimed facets (a Hard crease, not two surfaces running flush)
    /// keep the corner's refusal.
    fn named_flush(&self, spec: &BlendSpec, e: BlendError) -> BlendError {
        let BlendError::UnsupportedVertex { at } = e else {
            return e;
        };
        let claimed = |f: FaceId| {
            self.faces()
                .get(f)
                .and_then(|face| face.surface)
                .and_then(|s| s.normal_at(at))
                .is_some()
        };
        let tangent = |edge: EdgeId| -> bool {
            let Some((a, b)) = self.edge_faces(edge) else {
                return false;
            };
            if !(claimed(a) && claimed(b)) {
                return false;
            }
            match (self.oriented_normal(a, at), self.oriented_normal(b, at)) {
                (Some(na), Some(nb)) => {
                    na.dot(nb) > 0.0 && na.cross(nb).length() <= tol::BLEND_FLUSH_SIN
                }
                _ => false,
            }
        };
        let mut ending_here = spec
            .edges
            .iter()
            .filter(|&&edge| {
                self.edge_endpoints(edge).is_some_and(|(p, q)| {
                    p.approx_eq(at, tol::POINT_MERGE) || q.approx_eq(at, tol::POINT_MERGE)
                })
            })
            .peekable();
        if ending_here.peek().is_some() && ending_here.all(|&edge| tangent(edge)) {
            BlendError::BlendFacesFlush { at }
        } else {
            e
        }
    }

    /// [`Object::blend_edges`] without the corner re-solve.
    pub(crate) fn blend_edges_plain(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport), BlendError> {
        self.blend_preview_parts(spec)
            .map(|(after, report, _)| (after, report))
    }

    /// [`Object::blend_edges_plain`] plus the faces of `self` the result
    /// replaces, for previews.
    pub(crate) fn blend_preview_parts(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), BlendError> {
        match self.blend_attempt(spec) {
            Ok(parts) => Ok(self.recorded(parts, spec)),
            Err(fail) => Err(self.refuse(spec, fail, false)),
        }
    }

    /// One pass of a corner re-solve: [`Object::blend_edges_plain`] with the
    /// failure as it came (the re-solve gives up on any failure, so the
    /// refusal's limit search would be thrown away).
    pub(crate) fn blend_pass(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport), plan::PlanFail> {
        self.blend_attempt(spec)
            .map(|parts| self.recorded(parts, spec))
            .map(|(after, report, _)| (after, report))
            .map_err(|(f, _)| f)
    }

    /// A commit records what each blend replaced (a preview is thrown away
    /// and the commit builds its own).
    fn recorded(
        &self,
        (after, report, replaced): (Object, BlendReport, Vec<FaceId>),
        spec: &BlendSpec,
    ) -> (Object, BlendReport, Vec<FaceId>) {
        if is_preview() && !RECORD_IN_PREVIEW.with(std::cell::Cell::get) {
            return (after, report, replaced);
        }
        let mut after = after;
        for &id in &report.blends {
            self.record_blend(&mut after, id, spec);
        }
        (after, report, replaced)
    }

    /// The typed refusal for a failed [`Object::blend_attempt`]; a size
    /// names the limit with the corner re-solve when `corners` (the
    /// refusal of [`Object::blend_edges`]), else the plain blend's.
    fn refuse(
        &self,
        spec: &BlendSpec,
        (f, swept): (plan::PlanFail, bool),
        corners: bool,
    ) -> BlendError {
        let refusal = if swept {
            self.swept_refusal(spec, f, corners)
        } else {
            self.refusal(spec, f, corners)
        };
        refusal.unwrap_or_else(|e| e)
    }

    /// The plain blend before the record is taken, failing with why and
    /// whether the swept blend was tried too (its refusal then names the
    /// size that rolls).
    #[allow(clippy::type_complexity)]
    pub(crate) fn blend_attempt(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), (plan::PlanFail, bool)> {
        let catalog_only = CATALOG_ONLY.with(std::cell::Cell::get);
        match self.blend_once(spec) {
            Ok(r) => Ok(r),
            Err(f) if catalog_only => Err((f, false)),
            // Faceted chains the catalogue planners cannot turn (imported or
            // unstamped facets) retry as one swept blend; when that fails
            // too, the catalogue's refusal stands (D-41).
            Err(
                f @ plan::PlanFail::Hard(
                    BlendError::UnsupportedVertex { .. } | BlendError::UnsupportedEdge,
                ),
            ) => classify::with_forced_swept(|| self.blend_once(spec)).map_err(|_| (f, true)),
            // So do rounds the facets are too small for where the facets are
            // no edges anyone drew (D-119): an imported mesh, or a chain met
            // by smooth seams (a generated wall such as a Loft's), and a
            // ball rolls across them.
            Err(f @ plan::PlanFail::Size(_))
                if self.facets_roll(spec) && matches!(spec.profile, BlendProfile::Round { .. }) =>
            {
                classify::with_forced_swept(|| self.blend_once(spec)).map_err(|_| (f, true))
            }
            Err(f) => Err((f, false)),
        }
    }

    /// Whether `spec`'s facets are no edges anyone drew, so a round the
    /// facet planners limit by their size rolls across them as a swept
    /// blend (D-74, D-119): an imported mesh, or a chain that a smooth
    /// (soft) seam between unclaimed, unblended faces meets at one of its
    /// vertices (a generated wall such as a Loft's, one smooth surface by
    /// D-47). A drawn solid has no such seam at its rims, and a fillet's
    /// strip or an extruded circle's wall claims its surface, so their
    /// exact planners' limits stand.
    fn facets_roll(&self, spec: &BlendSpec) -> bool {
        if self.planarity_tol > crate::tol::PLANE_DIST {
            return true;
        }
        let plain = |f: FaceId| {
            self.faces()
                .get(f)
                .is_some_and(|x| x.surface.is_none() && x.blend.is_none())
        };
        let generated_seam = |x: EdgeId| {
            self.edges().get(x).is_some_and(|edge| edge.soft)
                && self
                    .edge_faces(x)
                    .is_some_and(|(a, b)| plain(a) && plain(b))
        };
        let ve = self.vertex_edges();
        spec.edges.iter().any(|&e| {
            self.edge_vertex_ids(e).is_some_and(|(a, b)| {
                [a, b]
                    .iter()
                    .any(|v| ve.get(v).into_iter().flatten().any(|&x| generated_seam(x)))
            })
        })
    }

    /// [`Object::blend_edges`] with the facet planners alone (no swept
    /// retry, no search for a size that rolls), optionally as a preview.
    /// Test support only.
    #[doc(hidden)]
    pub fn blend_edges_catalog_only_for_test(
        &self,
        spec: &BlendSpec,
        preview: bool,
    ) -> Result<(Object, BlendReport), BlendError> {
        let before = CATALOG_ONLY.with(|c| c.replace(true));
        let out = if preview {
            as_preview(|| self.blend_edges(spec))
        } else {
            self.blend_edges(spec)
        };
        CATALOG_ONLY.with(|c| c.set(before));
        out
    }

    /// [`Object::blend_edges`] as a preview makes it (one round of the
    /// swept blend's station removal, no swept limit search). Test support
    /// only.
    #[doc(hidden)]
    pub fn blend_edges_as_preview_for_test(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport), BlendError> {
        as_preview(|| self.blend_edges(spec))
    }

    /// One swept attempt, for a spec's diagnosis. Test support only.
    #[doc(hidden)]
    pub fn blend_swept_once_for_test(&self, spec: &BlendSpec) -> bool {
        classify::with_forced_swept(|| self.blend_once(spec)).is_ok()
    }

    /// The result of one swept attempt, `None` where it refuses. Test
    /// support only.
    #[doc(hidden)]
    pub fn blend_swept_result_for_test(&self, spec: &BlendSpec) -> Option<Object> {
        classify::with_forced_swept(|| self.blend_once(spec))
            .ok()
            .map(|(o, _, _)| o)
    }

    /// The refusal for a round on an imported mesh or a generated wall that
    /// neither the catalog nor the swept blend makes: where a smaller ball
    /// rolls, the size is too large and the largest that rolls is the limit
    /// ([`Object::largest_that_rolls`], to within [`SWEPT_LIMIT_PRECISION`]),
    /// limited by the shape of the model; otherwise [`Object::refusal`]'s. A
    /// user reads a refusal with no size as the tool failing for no reason.
    fn swept_refusal(
        &self,
        spec: &BlendSpec,
        f: plan::PlanFail,
        corners: bool,
    ) -> Result<BlendError, BlendError> {
        // An import's preview skips the search (minutes on a scan); a
        // generated wall's is bounded, so its preview names what the commit
        // names.
        let (true, false, BlendProfile::Round { .. }) = (
            self.facets_roll(spec),
            PREVIEW.with(std::cell::Cell::get) && self.planarity_tol > crate::tol::PLANE_DIST,
            &spec.profile,
        ) else {
            return self.refusal(spec, f, corners);
        };
        // A size refusal: its limit is widened to the largest size that
        // rolls (`Object::with_rolling`).
        if matches!(f, plan::PlanFail::Size(_)) {
            return self.refusal(spec, f, corners);
        }
        // A corner or edge the planners refuse outright: the largest size
        // that rolls where one does, or the planners' own limit where it is
        // larger, else the refusal.
        let base = self.refusal(spec, f, corners)?;
        Ok(match (base, self.largest_that_rolls(spec, true)) {
            (e @ BlendError::RadiusTooLarge { max, .. }, Some(m)) if max >= m => e,
            (_, Some(m)) => BlendError::RadiusTooLarge {
                max: m,
                limited_by: LimitFeature::Unknown,
            },
            (e, None) => e,
        })
    }

    /// The typed refusal for a failed attempt at `spec`: a size failure
    /// carries the tight maximum ([`Object::max_blend_size`] when
    /// `corners`, else the plain blend's).
    fn refusal(
        &self,
        spec: &BlendSpec,
        f: plan::PlanFail,
        corners: bool,
    ) -> Result<BlendError, BlendError> {
        Ok(match f {
            // The facet planners refuse a corner outright at some sizes that
            // build smaller (a corner whose run-out reaches the next vertex):
            // where a smaller size builds, it is a size refusal naming that
            // limit, so a preview clamps (D-119). Where none does, the
            // corner's refusal stands. Native solids only: an import's
            // limit is its own search (D-74).
            plan::PlanFail::Hard(e @ BlendError::UnsupportedVertex { .. })
                if spec.profile.size().is_some()
                    && self.planarity_tol <= crate::tol::PLANE_DIST =>
            {
                match self.hard_corner_limit(spec) {
                    Some(limit) => match spec.profile {
                        BlendProfile::Custom { width, .. } => BlendError::ProfileTooWide {
                            needed: width,
                            available: limit.max,
                        },
                        _ => BlendError::RadiusTooLarge {
                            max: limit.max,
                            limited_by: limit.limited_by,
                        },
                    },
                    None => e,
                }
            }
            plan::PlanFail::Hard(e) => e,
            plan::PlanFail::Size(feature) => {
                let facets = if corners {
                    self.max_blend_size_facets(spec)
                } else {
                    self.max_blend_size_plain(spec)
                };
                let (limit, rolled) = self.with_rolling(spec, facets, true);
                let limit = limit?;
                if rolled {
                    return Ok(BlendError::RadiusTooLarge {
                        max: limit.max,
                        limited_by: LimitFeature::Unknown,
                    });
                }
                if let BlendProfile::Custom { width, .. } = spec.profile {
                    return Ok(BlendError::ProfileTooWide {
                        needed: width,
                        available: limit.max,
                    });
                }
                BlendError::RadiusTooLarge {
                    max: limit.max,
                    limited_by: if limit.limited_by == LimitFeature::Unknown {
                        feature
                    } else {
                        limit.limited_by
                    },
                }
            }
            plan::PlanFail::Obstructed(at) => BlendError::Obstructed { at },
        })
    }

    /// [`Object::blend_edges`] with a swept chain's solve against the side
    /// surfaces turned off: the facet walk alone. Test support only.
    #[doc(hidden)]
    pub fn blend_edges_walk_only_for_test(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport), BlendError> {
        swept::without_surface_fallback(|| self.blend_edges(spec))
    }

    /// One attempt at `spec` (plan, apply, guard) with every swept chain run as
    /// attempt `attempt` of the geometric starts and directions it offers
    /// (an open chain: 0 from its end first in x, then y, then z order, 1
    /// from the other end), no other attempt tried after it and no limit
    /// searched. A size refusal reads as `RadiusTooLarge` with `max` 0 and
    /// the feature that limits it. Test support only.
    #[doc(hidden)]
    pub fn blend_swept_attempt_for_test(
        &self,
        spec: &BlendSpec,
        attempt: usize,
    ) -> Result<(), BlendError> {
        let (out, _) = chain::with_swept_attempt(attempt, || self.blend_single_way(spec));
        match out {
            Ok(_) => Ok(()),
            Err(plan::PlanFail::Hard(e)) => Err(e),
            Err(plan::PlanFail::Size(limited_by)) => Err(BlendError::RadiusTooLarge {
                max: 0.0,
                limited_by,
            }),
            Err(plan::PlanFail::Obstructed(at)) => Err(BlendError::Obstructed { at }),
        }
    }

    /// Whether `spec` plans (checks C1–C5 pass), without applying it. Test
    /// support only.
    #[doc(hidden)]
    pub fn blend_plans_for_test(&self, spec: &BlendSpec) -> bool {
        self.plan_blend(spec).is_ok()
    }

    /// One attempt at `spec`'s exact size (see
    /// [`Object::blend_once_as_claimed`]).
    pub(crate) fn blend_once(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), plan::PlanFail> {
        // A rim beside an unclaimed faceted cone (a coaxial Loft's end) is
        // tried as the coaxial rim it is first (D-118); where that does not
        // build, the facets as they are.
        if !classify::is_forced_swept()
            && let Some((work, claims)) = self.with_rim_claims(spec)
            && let Ok((mut after, report, replaced)) = work.blend_once_as_claimed(spec)
        {
            derive::drop_rim_claims(self, &mut after, &claims);
            return Ok((after, report, replaced));
        }
        self.blend_once_as_claimed(spec)
    }

    /// [`Object::blend_once`] with the claims as they are. A mixed-convexity
    /// corner where one odd edge stands square to the face the other two
    /// share is built in two passes (WP06 §7): the odd edges first, then
    /// the rest on that result, their survivors extended along the run-out
    /// arcs pass one left.
    fn blend_once_as_claimed(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), plan::PlanFail> {
        let infos = classify::classify(self, spec)?;
        let Some((first, second)) = two_pass_split(self, &infos)? else {
            return self.blend_single(spec);
        };
        let mut s1 = spec.clone();
        s1.edges = first;
        let (o1, r1, replaced) = self.blend_single(&s1)?;
        let mut edge_map = BTreeMap::new();
        let mut edges: Vec<EdgeId> = Vec::new();
        for e in second {
            let s =
                survivor(self, &o1, e).ok_or(plan::PlanFail::Hard(BlendError::BlendDegenerate))?;
            if s != e {
                edge_map.insert(e, s);
            }
            for x in o1.logical_edge(s, true) {
                if !edges.contains(&x) {
                    edges.push(x);
                }
            }
        }
        let mut s2 = spec.clone();
        s2.edges = edges;
        // The second pass is the same blend: its faces take the first
        // pass's id, so the two are one feature to remove or resize.
        let (o2, _, _) = with_forced_blend_id(Some(r1.blends[0]), || o1.blend_single(&s2))?;
        let blends = vec![r1.blends[0]];
        let new_faces: Vec<FaceId> = o2
            .faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| blends.contains(&t.id)))
            .map(|(id, _)| id)
            .collect();
        Ok((
            o2,
            BlendReport {
                blends,
                new_faces,
                profile: spec.profile.clone(),
                edge_map,
            },
            replaced,
        ))
    }

    /// One pass: plan, apply, guard. Swept chains take the starts and
    /// directions [`chain::canonical_swept`] offers in turn until one
    /// builds (D-228): every one is read off the geometry alone, so a blend
    /// and its resize on a restored object build the same, and the swept
    /// solve's dependence on where the ball starts rolling costs a refusal
    /// only where none builds. The first attempt's refusal is the one
    /// returned. The attempts are tried in step for every swept chain of
    /// the pass (attempt k for all), not in every combination: a spec with
    /// two swept chains that would build only with different attempts per
    /// chain refuses, typed, as a single chain that builds no way does
    /// (review F3; bounded at four passes, where combinations would be
    /// four to the number of chains).
    pub(crate) fn blend_single(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), plan::PlanFail> {
        let (first, offered) = chain::with_swept_attempt(0, || self.blend_single_way(spec));
        let Err(refused) = first else {
            return first;
        };
        // A refusal no start or direction can change (the spec, the
        // profile, a stop or a bullnose on a swept chain, the station cap)
        // is final: retrying would only repeat it (review F12).
        let start_free = matches!(
            refused,
            plan::PlanFail::Hard(
                BlendError::UnknownEdge
                    | BlendError::EmptySpec
                    | BlendError::BlendNeedsSolid
                    | BlendError::EdgeIsSmooth
                    | BlendError::UnsupportedEdge
                    | BlendError::ProfileNeedsConvexEdge
                    | BlendError::BullnoseNeedsParallelSides
                    | BlendError::InvalidProfile
                    | BlendError::StopOutOfRange
                    | BlendError::SegmentsOutOfRange { .. }
                    | BlendError::NonUniformInstance
            )
        );
        if start_free {
            return Err(refused);
        }
        for k in 1..offered {
            if let (Ok(done), _) = chain::with_swept_attempt(k, || self.blend_single_way(spec)) {
                return Ok(done);
            }
        }
        Err(refused)
    }

    /// [`Object::blend_single`] with swept chains as one attempt takes them.
    fn blend_single_way(
        &self,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport, Vec<FaceId>), plan::PlanFail> {
        swept::avoid_clear();
        let mut rounds = 0;
        loop {
            match self.blend_single_once(spec)? {
                Ok(done) => return Ok(done),
                // The surface solve's result failed a check at a point: the
                // stations near it go, a few rounds; then the walk's refusal
                // stands.
                Err((refused, at)) => {
                    rounds += 1;
                    if rounds > swept::avoid_rounds() {
                        return Err(refused);
                    }
                    swept::avoid_near(at);
                }
            }
        }
    }

    /// One plan, apply and guard. A swept chain's surface solve whose result
    /// fails a check answers the walk's refusal and where it failed.
    #[allow(clippy::type_complexity)]
    fn blend_single_once(
        &self,
        spec: &BlendSpec,
    ) -> Result<
        Result<(Object, BlendReport, Vec<FaceId>), (plan::PlanFail, crate::math::Point3)>,
        plan::PlanFail,
    > {
        let planned = self.plan_blend(spec)?;
        let centroid = |o: &Object, f: FaceId| {
            let (s, n) = o.loop_positions(o.faces()[f].outer_loop).fold(
                (crate::math::Vec3::new(0.0, 0.0, 0.0), 0usize),
                |(s, n), p| (s + p.to_vec(), n + 1),
            );
            let c = s / n.max(1) as f64;
            crate::math::Point3::new(c.x, c.y, c.z)
        };
        let applied = match self.apply_patch(&planned.patch) {
            Ok(a) => a,
            Err(er) => {
                // Where the result does not stitch: the point of the edge it
                // names.
                let at = match er {
                    crate::error::TopologyError::NonManifoldEdge { from, .. } => {
                        planned.patch.verts.get(from).map(|v| match *v {
                            crate::patch::PVert::Existing(id) => self.vertices()[id].position,
                            crate::patch::PVert::New(p) => p,
                        })
                    }
                    _ => None,
                };
                return match (planned.fallback, at) {
                    (Some(refused), Some(at)) => Ok(Err((refused, at))),
                    (Some(refused), None) => Err(refused),
                    (None, _) => Err(plan::PlanFail::Hard(BlendError::BlendDegenerate)),
                };
            }
        };
        let new_faces: Vec<FaceId> = applied.faces[planned.new_face_range.clone()].to_vec();
        let candidates: Vec<FaceId> = applied.faces.clone();
        if let Some((f, g)) = crate::patch::improper_contact(&applied.object, &candidates) {
            let at = centroid(&applied.object, f);
            if let Some(refused) = planned.fallback {
                // Between the two faces that cross.
                let mid = at + (centroid(&applied.object, g) - at) * 0.5;
                return Ok(Err((refused, mid)));
            }
            return Err(plan::PlanFail::Obstructed(at));
        }
        Ok(Ok((
            applied.object,
            BlendReport {
                blends: vec![planned.blend],
                new_faces,
                profile: spec.profile.clone(),
                edge_map: BTreeMap::new(),
            },
            planned.replaced,
        )))
    }
}

/// The odd edges and the rest, when a corner needs two passes.
type Passes = Option<(Vec<EdgeId>, Vec<EdgeId>)>;

/// The two passes of WP06 §7, or `None` when no picked corner mixes
/// convexity.
fn two_pass_split(obj: &Object, infos: &[classify::EdgeInfo]) -> Result<Passes, plan::PlanFail> {
    let by_edge: BTreeMap<EdgeId, &classify::EdgeInfo> =
        infos.iter().map(|i| (i.edge, i)).collect();
    let ve = obj.vertex_edges();
    let mut odd: std::collections::BTreeSet<EdgeId> = std::collections::BTreeSet::new();
    let mut majority: std::collections::BTreeSet<EdgeId> = std::collections::BTreeSet::new();
    for (&v, es) in &ve {
        if es.len() != 3 || !es.iter().all(|e| by_edge.contains_key(e)) {
            continue;
        }
        let convex: Vec<bool> = es
            .iter()
            .map(|e| by_edge[e].convexity == Convexity::Convex)
            .collect();
        let n_convex = convex.iter().filter(|&&c| c).count();
        if n_convex == 0 || n_convex == 3 {
            continue;
        }
        let at = obj.vertices()[v].position;
        let unsupported = plan::PlanFail::Hard(BlendError::UnsupportedVertex { at });
        let minority_convex = n_convex == 1;
        let (oi, _) = convex
            .iter()
            .enumerate()
            .find(|(_, c)| **c == minority_convex)
            .expect("one minority edge");
        let o = es[oi];
        let rest: Vec<EdgeId> = es.iter().copied().filter(|&e| e != o).collect();
        // The face the two majority edges share.
        let (a1, b1) = obj.edge_faces(rest[0]).ok_or(unsupported)?;
        let (a2, b2) = obj.edge_faces(rest[1]).ok_or(unsupported)?;
        let t = [a1, b1]
            .into_iter()
            .find(|f| *f == a2 || *f == b2)
            .ok_or(unsupported)?;
        let (p, q) = obj.edge_endpoints(o).ok_or(unsupported)?;
        let d = (q - p).normalized().map_err(|_| unsupported)?;
        if d.dot(obj.faces()[t].plane.normal()).abs() < 1.0 - crate::tol::NORMAL_DIRECTION {
            return Err(unsupported);
        }
        odd.insert(o);
        majority.extend(rest);
    }
    if odd.is_empty() {
        return Ok(None);
    }
    if odd.iter().any(|e| majority.contains(e)) {
        let e = *odd.iter().find(|e| majority.contains(e)).expect("checked");
        let at = obj
            .edge_endpoints(e)
            .map(|(a, _)| a)
            .unwrap_or(crate::math::Point3::new(0.0, 0.0, 0.0));
        return Err(plan::PlanFail::Hard(BlendError::UnsupportedVertex { at }));
    }
    let first: Vec<EdgeId> = odd.into_iter().collect();
    let second: Vec<EdgeId> = infos
        .iter()
        .map(|i| i.edge)
        .filter(|e| !first.contains(e))
        .collect();
    Ok(Some((first, second)))
}

/// The edge of `after` that is what remains of `e` of `before`: `e` itself
/// if unchanged, else the edge from one of its original endpoints along
/// its original line.
fn survivor(before: &Object, after: &Object, e: EdgeId) -> Option<EdgeId> {
    let (va, vb) = before.edge_vertex_ids(e)?;
    if after
        .edge_vertex_ids(e)
        .is_some_and(|(x, y)| (x, y) == (va, vb) || (y, x) == (va, vb))
    {
        return Some(e);
    }
    let (pa, pb) = (
        before.vertices()[va].position,
        before.vertices()[vb].position,
    );
    let len = (pb - pa).length();
    let dir = (pb - pa) / len;
    let on_segment = |p: crate::math::Point3| {
        let s = (p - pa).dot(dir);
        s >= -crate::tol::POINT_MERGE
            && s <= len + crate::tol::POINT_MERGE
            && ((p - pa) - dir * s).length() <= crate::tol::POINT_MERGE
    };
    after.edges().keys().find(|&x| {
        after.edge_vertex_ids(x).is_some_and(|(p, q)| {
            (p == va || p == vb || q == va || q == vb)
                && on_segment(after.vertices()[p].position)
                && on_segment(after.vertices()[q].position)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SurfaceRef;
    use crate::loft::{LoftProfile, LoftSpec};
    use crate::math::{Point3, Vec3};
    use crate::trig;

    /// A prism of the `xy` loop, from z 0 to z 1, its walls unclaimed.
    fn prism(pts: &[(f64, f64)]) -> Object {
        let prof = |z: f64| LoftProfile {
            points: pts.iter().map(|&(x, y)| Point3::new(x, y, z)).collect(),
            curves: Vec::new(),
            smooth: Vec::new(),
            through: Vec::new(),
            start_offset: 0,
        };
        Object::from_loft(&LoftSpec {
            profiles: vec![prof(0.0), prof(1.0)],
            apex: None,
            rails: Vec::new(),
            stations: None,
        })
        .unwrap()
    }

    /// The wall edge of `obj` from `(x, y, 0)` to `(x, y, 1)`.
    fn wall_edge(obj: &Object, x: f64, y: f64) -> EdgeId {
        let at = |p: Point3, z: f64| p.approx_eq(Point3::new(x, y, z), crate::tol::POINT_MERGE);
        obj.edges()
            .keys()
            .find(|&e| {
                obj.edge_endpoints(e)
                    .is_some_and(|(p, q)| (at(p, 0.0) && at(q, 1.0)) || (at(q, 0.0) && at(p, 1.0)))
            })
            .expect("the wall edge")
    }

    /// The edge of `obj` from `p` to `q`.
    fn obj_edge(obj: &Object, p: Point3, q: Point3) -> EdgeId {
        let m = crate::tol::POINT_MERGE;
        obj.edges()
            .keys()
            .find(|&e| {
                obj.edge_endpoints(e).is_some_and(|(a, b)| {
                    (a.approx_eq(p, m) && b.approx_eq(q, m))
                        || (a.approx_eq(q, m) && b.approx_eq(p, m))
                })
            })
            .expect("the edge")
    }

    fn round(edges: Vec<EdgeId>) -> BlendSpec {
        BlendSpec {
            edges,
            profile: BlendProfile::Round {
                radius: 0.01,
                segments: 4,
            },
            primary: None,
            corners: CornerStyle::default(),
            stops: None,
            handles: Vec::new(),
        }
    }

    /// A refusal at `at` that `named_flush` must leave as it is.
    fn stays_unsupported(obj: &Object, edge: EdgeId, at: Point3) {
        let e = BlendError::UnsupportedVertex { at };
        assert_eq!(obj.named_flush(&round(vec![edge]), e), e);
    }

    /// Claims on the two faces of `edge` whose normals at `at` are the
    /// faces' own plane normals: a cylinder about z whose axis stands back
    /// from `at` along the normal.
    fn claim_walls(obj: &mut Object, edge: EdgeId, at: Point3) {
        let (a, b) = obj.edge_faces(edge).unwrap();
        for f in [a, b] {
            let n = obj.faces[f].plane.normal();
            obj.faces[f].surface = Some(SurfaceRef::Cylinder {
                axis_point: at - n * 2.0,
                axis: Vec3::new(0.0, 0.0, 1.0),
                radius: 2.0,
            });
        }
    }

    /// Review F6: a razor wedge (a 1° fin) has its two walls' normals
    /// nearly opposite. They are not tangent, claimed or not: the faces
    /// fold back on each other, they do not run flush.
    #[test]
    fn a_razor_wedge_is_not_flush() {
        let h = trig::tan(0.5_f64.to_radians());
        let mut wedge = prism(&[(0.0, 0.0), (1.0, -h), (1.0, h)]);
        let fin = wall_edge(&wedge, 0.0, 0.0);
        let apex = Point3::new(0.0, 0.0, 0.0);
        stays_unsupported(&wedge, fin, apex);
        // Claimed as smooth surfaces, the walls still face apart.
        claim_walls(&mut wedge, fin, apex);
        stays_unsupported(&wedge, fin, apex);
    }

    /// Review F10: a scan's facets meeting at a 1.5° crease carry no claim:
    /// a Hard crease, not two surfaces running flush, so the refusal stays
    /// the corner's.
    #[test]
    fn a_shallow_unclaimed_crease_is_not_flush() {
        let t = trig::tan(1.5_f64.to_radians());
        let scan = prism(&[(0.0, 0.0), (1.0, 0.0), (2.0, t), (2.0, 1.0), (0.0, 1.0)]);
        let crease = wall_edge(&scan, 1.0, 0.0);
        stays_unsupported(&scan, crease, Point3::new(1.0, 0.0, 0.0));
    }

    /// Two claimed surfaces tangent where the refusal stands are flush, but
    /// only when every edge of the spec ending there is flush (review F7):
    /// the planner's refusal does not say which edge it was.
    #[test]
    fn claimed_tangent_faces_are_flush_only_at_their_own_edge() {
        let t = trig::tan(1.5_f64.to_radians());
        let mut scan = prism(&[(0.0, 0.0), (1.0, 0.0), (2.0, t), (2.0, 1.0), (0.0, 1.0)]);
        let crease = wall_edge(&scan, 1.0, 0.0);
        let at = Point3::new(1.0, 0.0, 0.0);
        let (a, b) = scan.edge_faces(crease).unwrap();
        for f in [a, b] {
            scan.faces[f].surface = Some(SurfaceRef::Cylinder {
                axis_point: Point3::new(1.0, 2.0, 0.0),
                axis: Vec3::new(0.0, 0.0, 1.0),
                radius: 2.0,
            });
        }
        let e = BlendError::UnsupportedVertex { at };
        assert_eq!(
            scan.named_flush(&round(vec![crease]), e),
            BlendError::BlendFacesFlush { at }
        );
        // A second edge of the spec ends there too, its faces at a right
        // angle: the refusal may be that edge's, so it stays the corner's.
        let rim = obj_edge(&scan, at, Point3::new(2.0, t, 0.0));
        assert_eq!(scan.named_flush(&round(vec![crease, rim]), e), e);
    }
}
