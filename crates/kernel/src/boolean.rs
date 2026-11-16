//! Boolean combination of solids — `Object::boolean` lives in `ops.rs`; this
//! module is its implementation.
//!
//! ARCHITECTURE.md : polygon arrangement reusing the sketch region tracer
//! (not triangle-soup CSG);  extends it to resolve coplanar contact.
//! The work is sliced:
//!
//! - **1a:** operand/transform validation and mapping `b` into `a`'s frame.
//! - **1b:** the unified arrangement pipeline. Every face of each operand is
//!   imprinted with the seam segments where the *other* solid's faces cross it,
//!   partitioned into sub-faces by the sketch region tracer, and each sub-face
//!   kept or dropped (and for subtract, flipped) by testing its interior against
//!   the other solid. The kept sub-faces are welded and rebuilt into a watertight
//!   Object. This single path covers crossing, nested, and disjoint solids alike.
//! - **1c (this commit):** split a disconnected result into one shell per
//!   connected component (subtract that cuts a solid in two; disjoint union), a
//!   hole-robust interior-point test for annular sub-faces, and the property
//!   suite (set-algebra volumes, multi-shell, degenerate refusal).
//!
//! **Coplanar contact is resolved, not refused (ARCHITECTURE.md #19, extends #15).**
//! Each face is also subdivided by the boundaries of any coplanar partner faces,
//! and every sub-face is classified by the coverage-based neighborhood test in
//! [`classify`]: it keeps a sub-face iff inside-result differs across it,
//! evaluating the other operand's membership exactly from a covering coplanar
//! partner (or a ray cast otherwise). Coincident shared patches produced by both
//! operands are deduplicated ([`faces_coincide`]). This handles boxes on a shared
//! ground, shared walls, stacking, coincident solids, and interpenetrating
//! coplanar overlaps.
//!
//! Known gaps (refused, never silently wrong):
//! - **Measure-zero tangency** (`DegenerateContact`) — two solids whose only
//!   contact carries no volume: a shared edge with no adjoining face overlap,
//!   an edge lying within a non-coplanar face, or a lone vertex on a face.
//!   There is nothing to weld the shells into, so assembly reports a
//!   non-watertight result. (Partial coplanar *face* contact — boxes set
//!   side-by-side, a smaller face resting wholly inside a larger one — does
//!   weld, resolved by the coplanar-boundary imprint.)
//! - **Faces of an imported operand that coincide with the other operand's
//!   only to import noise** (an imported base resting on a box top, off by
//!   1e-8): not recognized as coplanar at `tol::PLANE_DIST`, so they refuse
//!   like a touch (`DegenerateContact`).
//! - **An arrangement that does not resolve** (`ArrangementFailed`): a result
//!   face that fails validation as not flat to the plane it carries. A face
//!   that validates reopens on that plane: the file stores it (geometry
//!   buffer v11). A result with no holes is rebuilt face by face from its
//!   rings, so each of its faces carries the refit of its outer ring (or
//!   Newell's fit in ring order when the refit misses a point) rather than
//!   the plane its piece had; a result with holes keeps every piece's plane.
//! - **A result too thin to keep** (`ResultTooThin`): a kept piece with no
//!   plane that the tidy (below) could not merge into a valid solid. A
//!   face keeps a plane when twice its area reaches
//!   `tol::NORMALIZE_MIN_LENGTH` or, failing that, when it is at least
//!   `tol::MIN_FACE_WIDTH` (10 nm) wide (`Plane::from_polygon`); a nudge of
//!   a hundredth of a millimeter clears what is left.
//!
//! **Slivers of the cut are merged, and reported** ([`tidy`]). Nothing
//! narrower than `tol::FEATURE_MIN` (0.1 µm) is a feature, yet a cut
//! through a dense mesh leaves faces at every size down to the merge
//! distance where it passes near a vertex or an edge. When a kept face that
//! holds a point of the cut is narrower than that, or has no plane, the
//! boolean welds the cut's points around it before assembly: deterministic,
//! moving nothing farther than `tol::TIDY_REACH`, never changing an operand,
//! splitting into flat triangles any face it bends off its plane, and
//! counted in a [`BooleanTidy`] that is logged and returned to callers that
//! ask (`Object::boolean_reported`). With no such face the boolean is bit
//! for bit what it is without the tidy. A tidy that cannot give a valid
//! solid falls back to the untidied faces, so a refusal survives only
//! where both fail. A cutter that enters the target by less than the
//! floor removed nothing (`NothingRemoved`), and a result merged away
//! entirely is `EmptyResult`.
//!
//! **Imported operands** are flat only to their own tolerance
//! (`tol::IMPORT_PLANE_DIST`). Before arranging, [`refine_operand`] splits
//! each face of such an operand that is not flat to `tol::PLANE_DIST` and
//! that a face of the other operand comes near into flat triangles through
//! its own vertices (the diagonals come out soft), and marks the other
//! non-flat faces to pass through whole. Flat faces, and every face of a
//! native operand, take the arrangement unchanged; on a face of an
//! imported operand the arrangement still reads points within that
//! operand's tolerance of the face's plane ([`FacePoly::plane_tol`]) and
//! ends a seam exactly on an edge it crosses ([`seam_end`]). A face that
//! cannot be split is refused as `OperandFaceNotFlat`, naming it.
//!
//! The tangency refusal surfaces as a clean refusal (a non-watertight weld caught at assembly)
//! rather than a silently wrong result. One line contact IS resolved: a
//! partner edge lying in a face's plane whose two faces both lie on one side
//! of it (a cylinder tangent to a side face along a generatrix — a circle
//! tangent to a face's edge at one point, pushed through) imprints nothing,
//! so the face stays whole and the result touches itself along that line,
//! every edge still between exactly two faces.
//!
//! **A Subtract that removes nothing is refused as `NothingRemoved`.** When
//! the operands are disjoint, or touch only along a face, an edge, or a
//! corner, the arrangement keeps every sub-face of `a` and no face of `b`,
//! so the assembled result would be `a` itself. Union and intersect have an
//! honest answer there (two shells; `EmptyResult`), but a subtract that hands
//! back an unchanged `a` reads as a cut that silently did nothing, so
//! [`execute`] refuses when the arrangement dropped no sub-face of `a` and
//! kept no sub-face of `b`. That is the classification's own verdict —
//! `b` never enters `a` — so the test adds no tolerance of its own and is
//! the same whether the operands sit at the origin or a hundred kilometers
//! from it. It does inherit the arrangement's: a cutter whose skin lies
//! within `tol::PLANE_DIST` of a face of `a` is coplanar with it to the
//! arrangement, so a cut thinner than that (a nanometer) is refused exactly
//! as a touch is, while any cut the arrangement can resolve stays a cut.
//!
//! Why the result is watertight: a seam is the real intersection of two faces,
//! computed once and imprinted identically on both. Where the seam meets a face
//! boundary, the intersection curve continues onto the neighbouring face through
//! the *same* point, so that neighbour is split there too — T-junctions are
//! consistent by construction, and welding coincident vertices pairs the seam
//! half-edges into twins.
//!
//! One subtlety breaks that "computed once" rule unless guarded: when a face has
//! a coplanar partner *and* a perpendicular neighbour of that partner meets the
//! face exactly along the partner's rim — a cylinder's cap is coplanar with a box
//! face while the cylinder's side walls stand on the cap circle — the rim is
//! offered twice: as the coplanar partner's boundary edge (endpoints are exact
//! partner vertices) and as the perpendicular neighbour's transversal seam
//! (derived from its stored plane). In generic position the two disagree by
//! rounding, and the region tracer turns the near-duplicate collinear pair into
//! sliver vertices that defeat the coincident-patch dedup and leave the weld
//! non-manifold. [`collect_faces`] takes the coplanar boundary as authoritative
//! and drops any transversal seam that lies along it ([`segment_lies_along_any`]).
//!
//! A second guard covers the same failure one step removed from coplanarity:
//! a partner *edge* lying in the face's plane while neither adjacent partner
//! face is coplanar with it — a lying prism whose facet row grazes a box face
//! (the row of a 90°-rotated cylinder carries ~1e-16 of rotation noise, so it
//! sits *near* the plane rather than on it). The membership transition across
//! that tangent line is real, but the only plane-derived seams near it ride the
//! adjacent facets' boundaries and get clipped away by the interval midpoint
//! test, so the face is never split there and a strip inside the partner
//! survives classification — the weld then leaks at T-junctions and the op is
//! refused as a bogus `DegenerateContact`. [`collect_faces`] imprints such
//! in-plane partner edges as authoritative seams, exactly like coplanar rims.

use slotmap::SecondaryMap;
use tracing::info;

use crate::attr::AttrDict;
use crate::error::TopologyError;
use crate::geom2d::{plane_axes, point_inside_polygon};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::material::{FaceMaterial, UvFrame};
use crate::math::{Plane, Point3, Vec3};
use crate::ops::{BooleanError, BooleanOp, BooleanTidy, Operand};
use crate::sketch::{Sketch, SketchError};
use crate::tol;
use std::collections::BTreeMap;

use crate::rederive::Provenance;
use crate::topo::{BlendTag, Object, SurfaceRef, WatertightState};
use crate::transform::Transform;
use ear_clip::triangulate;

mod ear_clip;
mod tidy;

/// A face flattened to position lists for geometric queries: the outer boundary,
/// any hole boundaries, the supporting (oriented) plane, its material,
/// and its UV frame ( extension).
struct FacePoly {
    /// The owning operand's planarity tolerance (`PLANE_DIST` native,
    /// `IMPORT_PLANE_DIST` imported): whether a sub-face cut from this one
    /// may take the small-ring plane an imported object's file accepts.
    plane_tol: f64,
    /// How far this face's own vertices may lie from its plane, as the
    /// arrangement reads it: `PLANE_DIST` for every face flat to that (all
    /// of a native operand's, and the flat ones of an imported operand,
    /// which so take exactly the native path), else how far its farthest
    /// vertex strays (a face of an imported operand that the pre-pass
    /// passes through whole, [`refine_operand`]). Every "is this point on
    /// the plane" test against this face uses it; points are never
    /// projected.
    flat_tol: f64,
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    plane: Plane,
    material: FaceMaterial,
    back_material: FaceMaterial,
    uv_frame: Option<UvFrame>,
    surface: Option<SurfaceRef>,
    blend: Option<BlendTag>,
    /// The operand face's attribute dictionary (empty = none), carried by
    /// value: the result is a new Object with its own slot table.
    attrs: AttrDict,
    /// The operand face this came from (re-derivation's provenance).
    source: Provenance,
    /// The face's own vertices by exact position, sorted: which operand
    /// vertex a sub-face corner is when the arrangement kept it.
    ids: Vec<([u64; 3], VertexId)>,
    /// A face of its operand the other operand's box does not reach: it
    /// is kept or dropped whole, never arranged (see [`execute`]).
    far: bool,
    /// For a triangle of an imported face the pre-pass split
    /// ([`refine_operand`]): the operand and source face, so the diagonals
    /// between the triangles come out soft.
    split_from: Option<(Side, FaceId)>,
    /// An imported face that is not flat and that no face of the other
    /// operand comes near: it is kept or dropped whole, never imprinted.
    pass_through: bool,
    /// A triangle of a split imported face narrower than
    /// `tol::MIN_FACE_WIDTH`: `plane` is the triangle's own Newell plane,
    /// which no face may keep, so every piece of it is a sliver for the
    /// tidy to merge ([`tidy`]).
    no_plane: bool,
}

/// An operand vertex a result corner is: `(side, vertex)` (side 0 = A,
/// 1 = B), or `None` for a point the boolean made (a seam point). Corners
/// that are the same operand vertex stay one vertex in the result, and two
/// distinct vertices of one operand never merge (a mended scan keeps
/// coincident copies at a pinch apart on purpose).
type Ident = Option<(u8, VertexId)>;

fn pos_key(p: Point3) -> [u64; 3] {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

impl FacePoly {
    fn ident(&self, p: Point3) -> Ident {
        let k = pos_key(p);
        self.ids
            .binary_search_by(|(q, _)| q.cmp(&k))
            .ok()
            .map(|i| (self.source.side, self.ids[i].1))
    }
}

/// A sub-face produced by the arrangement: oriented loops in the face's plane.
struct Region {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
}

/// A kept sub-face ready for assembly, with winding already final (flipped for
/// the subtracted operand) and its plane consistent with that winding.
#[derive(Clone)]
struct OrientedFace {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    plane: Plane,
    material: FaceMaterial,
    back_material: FaceMaterial,
    uv_frame: Option<UvFrame>,
    surface: Option<SurfaceRef>,
    blend: Option<BlendTag>,
    /// The source face's attribute dictionary (empty = none).
    attrs: AttrDict,
    /// The operand face this came from (re-derivation's provenance).
    source: Provenance,
    /// The operand vertex each outer / hole corner is, parallel to the loops.
    outer_ids: Vec<Ident>,
    hole_ids: Vec<Vec<Ident>>,
    /// See [`FacePoly::split_from`].
    split_from: Option<(Side, FaceId)>,
    /// The piece has no plane of its own (narrower than
    /// `tol::MIN_FACE_WIDTH`): `plane` is its source face's, a placeholder
    /// the tidy must merge away ([`tidy`]).
    no_plane: bool,
    /// The operand and the index of its face (after the pre-pass) this
    /// piece was cut from.
    origin: (Side, usize),
    /// Faces the tidy split into triangles, one number per source face, so
    /// the diagonals between them come out soft.
    tidy_group: Option<u32>,
}

/// Which operand a batch of faces comes from (selection rules differ for
/// subtract, which is asymmetric).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    A,
    B,
}

impl Side {
    fn operand(self) -> Operand {
        match self {
            Side::A => Operand::A,
            Side::B => Operand::B,
        }
    }
}

/// Implements [`Object::boolean`]; see that method's contract.
pub(crate) fn execute(
    op: BooleanOp,
    a: &Object,
    b: &Object,
    b_to_a: &Transform,
    report: &mut BooleanTidy,
) -> Result<Object, BooleanError> {
    execute_region(op, a, b, b_to_a, report).map(|(o, _)| o)
}

/// What a boolean rebuilt when it kept one operand's untouched faces as
/// they were ([`execute_region`]): the result's new faces, and the faces of
/// each operand it arranged (the "near" faces, which the other operand's
/// box reaches). The document's seam cleanup looks only there.
pub(crate) struct BooleanTouch {
    pub new_faces: Vec<FaceId>,
    pub near: [Vec<FaceId>; 2],
}

/// [`execute`], also returning what it rebuilt when it kept an operand's
/// untouched faces in place.
///
/// Large models: a face of one operand whose (padded) box misses the other
/// operand's box cannot meet it, so it lies wholly inside or outside the
/// result (outside the other solid: kept by union, by subtract on A,
/// dropped otherwise) and is never arranged. When some are kept, the
/// operand with more of them is the base: the result is that operand with
/// its arranged faces swapped for the new ones ([`Object::splice_closed`]),
/// so its untouched faces, their edges, vertices, ids and marks stay as
/// they were, and the cost follows the touched region, not the model.
///
/// What the tidy merged ([`tidy`]) is added to `report`.
///
/// The tidy may move a point of the rebuilt region, or fold one into an
/// edge, where an untouched face of the base holds it too: a sliver can lie
/// within [`tol::TIDY_REACH`] of a face whose box misses the other operand
/// by more than its padding. The base's untouched face would no longer meet
/// the new ones, so the region grows by the base faces around that place
/// and the boolean runs again; after [`MAX_REGION_GROWTH`] rounds every face
/// is arranged, as the full build does.
pub(crate) fn execute_region(
    op: BooleanOp,
    a: &Object,
    b: &Object,
    b_to_a: &Transform,
    report: &mut BooleanTidy,
) -> Result<(Object, Option<BooleanTouch>), BooleanError> {
    if a.watertight() != WatertightState::Watertight {
        return Err(BooleanError::OperandNotSolid { which: Operand::A });
    }
    if b.watertight() != WatertightState::Watertight {
        return Err(BooleanError::OperandNotSolid { which: Operand::B });
    }
    if b_to_a.determinant().abs() < tol::NORMALIZE_MIN_LENGTH {
        return Err(BooleanError::SingularTransform);
    }

    // Everything happens in `a`'s frame.
    let b_in_a = transformed_object(b, b_to_a)?;
    let solids = [RaySolid::new(a), RaySolid::new(&b_in_a)];
    let reach = [solids[0].bounds(), solids[1].bounds()];
    // near[s][i]: face i of operand s meets the other operand's box.
    let mut near: [Vec<bool>; 2] = [0, 1].map(|s| {
        solids[s]
            .faces
            .iter()
            .map(|f| reach[1 - s].is_some_and(|r| boxes_overlap(f.2, r)))
            .collect()
    });
    let mut grown = 0usize;
    let cap = max_region_growth();
    for round in 0..=cap {
        if round == cap {
            // Every face arranged: the full build, which has no seam with
            // an untouched face to keep.
            for n in near.iter_mut() {
                n.fill(true);
            }
        }
        let grow = std::cell::RefCell::new(Vec::new());
        let mut tidy = BooleanTidy::default();
        let answer = execute_near(op, a, &b_in_a, &solids, &near, &mut tidy, &grow);
        let grow = grow.into_inner();
        if grow.is_empty() || round == cap {
            if grown > 0 {
                info!(
                    target: "kernel::op",
                    op = "boolean_region_grown",
                    rounds = round,
                    faces = grown,
                );
            }
            if answer.is_ok() {
                report.absorb(&tidy, &Transform::IDENTITY);
            }
            return answer;
        }
        for (s, i) in grow {
            if !near[s][i] {
                near[s][i] = true;
                grown += 1;
            }
        }
    }
    unreachable!("the last round always answers")
}

/// The most times [`execute_region`] grows the region it rebuilds before
/// arranging every face.
const MAX_REGION_GROWTH: usize = 6;

#[cfg(test)]
thread_local! {
    /// A spec's cap on the growth rounds, in place of [`MAX_REGION_GROWTH`].
    static GROWTH_CAP: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_REGION_GROWTH) };
}

/// The cap [`execute_region`] runs under: [`MAX_REGION_GROWTH`], which a
/// spec may lower to reach the full-build round with a small fixture.
fn max_region_growth() -> usize {
    #[cfg(test)]
    {
        GROWTH_CAP.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        MAX_REGION_GROWTH
    }
}

/// The edges of the base that bound the rebuilt region: each half-edge of
/// a near face whose twin lies on an untouched face, keyed by its two
/// endpoints' bits.
struct RegionSeam {
    edges: Vec<([u64; 3], [u64; 3], crate::ids::HalfEdgeId)>,
}

impl RegionSeam {
    fn of(obj: &Object, near: &SecondaryMap<FaceId, ()>) -> RegionSeam {
        let mut edges = Vec::new();
        for f in near.keys() {
            let face = &obj.faces()[f];
            for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                for h in obj.loop_half_edges(l) {
                    let he = obj.half_edges()[h];
                    let Some(t) = he.twin else { continue };
                    let tf = obj.loops()[obj.half_edges()[t].loop_id].face;
                    if near.contains_key(tf) {
                        continue;
                    }
                    let p = obj.vertices()[he.origin].position;
                    let q = obj.vertices()[obj.half_edges()[he.next].origin].position;
                    edges.push((pos_key(p), pos_key(q), h));
                }
            }
        }
        RegionSeam { edges }
    }

    /// The untouched faces of `obj` around every seam edge `faces` do not
    /// hold whole (by bits, either way round): every face around both of
    /// its ends. Empty when the seam is intact.
    fn disturbed(
        &self,
        obj: &Object,
        near: &SecondaryMap<FaceId, ()>,
        faces: &[OrientedFace],
    ) -> Vec<FaceId> {
        if self.edges.is_empty() {
            return Vec::new();
        }
        let mut held: std::collections::BTreeSet<([u64; 3], [u64; 3])> =
            std::collections::BTreeSet::new();
        for f in faces {
            for ring in std::iter::once(&f.outer).chain(f.holes.iter()) {
                for i in 0..ring.len() {
                    let (p, q) = (pos_key(ring[i]), pos_key(ring[(i + 1) % ring.len()]));
                    held.insert((p.min(q), p.max(q)));
                }
            }
        }
        let mut out = Vec::new();
        for &(p, q, h) in &self.edges {
            if held.contains(&(p.min(q), p.max(q))) {
                continue;
            }
            let start = [Some(h), obj.half_edges()[h].twin];
            for s in start.into_iter().flatten() {
                // Around the half-edge's origin: next of the twin.
                let mut e = s;
                for _ in 0..obj.half_edges().len() {
                    let face = obj.loops()[obj.half_edges()[e].loop_id].face;
                    if !near.contains_key(face) {
                        out.push(face);
                    }
                    let Some(t) = obj.half_edges()[e].twin else {
                        break;
                    };
                    e = obj.half_edges()[t].next;
                    if e == s {
                        break;
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// One run of [`execute_region`] over the faces `near` marks. A tidy that
/// disturbed the seam with the base's untouched faces puts the faces to
/// add in `grow` (side, face index) and answers as the untidied faces do;
/// the caller runs again.
fn execute_near(
    op: BooleanOp,
    a: &Object,
    b_in_a: &Object,
    solids: &[RaySolid<'_>; 2],
    near: &[Vec<bool>; 2],
    report: &mut BooleanTidy,
    grow: &std::cell::RefCell<Vec<(usize, usize)>>,
) -> Result<(Object, Option<BooleanTouch>), BooleanError> {
    let operands: [&Object; 2] = [a, b_in_a];
    // `b` in `a`'s frame holds `b`'s planarity tolerance.
    let b = b_in_a;
    let blend_offset = a.max_blend_id().map_or(0, |m| m.0);
    let far_count = |s: usize| near[s].iter().filter(|&&n| !n).count();
    // Far faces lie outside the other solid: union keeps both sides',
    // subtract keeps A's, intersect none.
    let far_kept = |s: usize| match op {
        BooleanOp::Union => true,
        BooleanOp::Subtract => s == 0,
        BooleanOp::Intersect => false,
    };
    let base: Option<usize> = match op {
        BooleanOp::Union if far_count(0) >= far_count(1) && far_count(0) > 0 => Some(0),
        BooleanOp::Union if far_count(1) > 0 => Some(1),
        BooleanOp::Subtract if far_count(0) > 0 => Some(0),
        _ => None,
    };

    // The faces each side processes, in face order: its near faces, plus
    // (off the base) its kept far faces, passed through whole.
    let polys: [Vec<FacePoly>; 2] = [0, 1].map(|s| {
        let obj = operands[s];
        solids[s]
            .faces
            .iter()
            .zip(&near[s])
            .filter(|&(_, &n)| n || (far_kept(s) && base != Some(s)))
            .map(|(&(fid, _, _), &n)| {
                face_poly(obj, fid, s as u8, if s == 1 { blend_offset } else { 0 }, !n)
            })
            .collect()
    });
    // The rings that touch at a point, from the faces as the operands hold
    // them (splitting a holed face into triangles must not change which
    // tangent contacts are recognized).
    let touch: [Vec<Point3>; 2] = [touch_points(&polys[0]), touch_points(&polys[1])];

    // Imported operands are flat only to their own tolerance. Where a face
    // of the other operand reaches a face that is not flat to
    // `tol::PLANE_DIST`, the face is split into flat triangles through its
    // own vertices; the rest of such faces pass through whole (see
    // `refine_operand`). Native operands skip this, and flat faces take the
    // path they always have.
    let refine_reach = a.planarity_tol + b.planarity_tol;
    let boxes = [FaceBoxes::of(&polys[0]), FaceBoxes::of(&polys[1])];
    let [a_polys, b_polys] = polys;
    let refine = |polys: Vec<FacePoly>, s: usize, side: Side| {
        if operands[s].planarity_tol > tol::PLANE_DIST {
            refine_operand(polys, &boxes[1 - s], refine_reach, side)
        } else {
            Ok((polys, Refined::default()))
        }
    };
    let (a_polys, a_count) = refine(a_polys, 0, Side::A)?;
    let (b_polys, b_count) = refine(b_polys, 1, Side::B)?;
    if a.planarity_tol > tol::PLANE_DIST || b.planarity_tol > tol::PLANE_DIST {
        info!(
            target: "kernel::op",
            op = "boolean_refine",
            faces_split = a_count.split + b_count.split,
            triangles = a_count.triangles + b_count.triangles,
            faces_passed_through = a_count.passed + b_count.passed,
        );
    }
    let polys = [a_polys, b_polys];
    // A pass-through face is no partner of the other operand's faces, every
    // one of which it lies clear of.
    let near_polys: [Vec<&FacePoly>; 2] = [0, 1].map(|s| {
        polys[s]
            .iter()
            .filter(|p| !p.far && !p.pass_through)
            .collect()
    });

    let mut faces = Vec::new();
    let a_dropped = collect_faces(
        op,
        Side::A,
        &polys[0],
        &near_polys[1],
        solids,
        &touch[0],
        &mut faces,
    )?;
    let a_kept = faces.len();
    collect_faces(
        op,
        Side::B,
        &polys[1],
        &near_polys[0],
        solids,
        &touch[1],
        &mut faces,
    )?;
    // A Subtract removes volume exactly when the arrangement changes `a`'s
    // skin: it drops a sub-face of `a` (that patch of `a`'s boundary lies
    // inside `b`, so the material behind it goes — a whole swallowed shell
    // of a multi-shell `a` drops every face it has) or keeps a sub-face of
    // `b` (that patch of `b`'s skin lies inside `a` and becomes the cut's
    // new wall). Neither happening means `b` never enters `a` — the operands
    // are apart, or touch along a face, an edge, or a corner — and the
    // result would be a copy of `a`, which would read as a successful cut
    // that changed nothing. (A coincident patch both operands produce is
    // attributed to `a`, collected first, so a touching `b` contributes
    // nothing here.) This reads the arrangement's own classification, so it
    // adds no tolerance beyond the arrangement's (`tol::PLANE_DIST`,
    // `tol::POINT_MERGE`) and does not depend on where the operands sit.
    // (Faces of `a` that miss `b`'s box are never arranged, so they are
    // neither dropped nor counted among the kept ones above.)
    if op == BooleanOp::Subtract
        && a_dropped == 0
        && faces.len() == a_kept
        && !(faces.is_empty() && base.is_none())
    {
        return Err(BooleanError::NothingRemoved);
    }

    // A face the cut left narrower than `tol::FEATURE_MIN` is merged into
    // its neighbors before the weld (`tidy`); with none, this is exactly
    // the assembly below.
    let planarity_tol = a.planarity_tol.max(b.planarity_tol);
    let tidy_polys = (polys[0].as_slice(), polys[1].as_slice());
    let Some(base) = base else {
        if faces.is_empty() {
            return Err(BooleanError::EmptyResult);
        }
        let ((mut result, prov), tags) =
            tidy::assemble_tidied(op, faces, tidy_polys, planarity_tol, report, false, |f| {
                let tags = split_tags(&f);
                let built = assemble(f, planarity_tol, operands)?;
                let ids: Vec<FaceId> = built.0.faces().keys().collect();
                Ok((built, ids.into_iter().zip(tags).collect::<Vec<_>>()))
            })?;
        // Booleans rebuild edges bare; re-derive the soft seams, creases and
        // circle claims the operands' geometry still supports (true-curves.md
        // §3.3 rule 1), then hold the result to the validator again. The
        // diagonals the boolean drew to split a face come out soft.
        crate::rederive::rederive_after_boolean(&mut result, a, b_in_a, &prov);
        soften_split_diagonals(&mut result, &tags);
        result.check_invariants();
        result
            .validate()
            .map_err(|_| BooleanError::DegenerateContact)?;
        // The result inherits operand A's base material ( follow-up), so a
        // subtract's carved walls from an unpainted cutter resolve to the carved
        // solid's color, and a union keeps the primary operand's base.
        result.default_material = a.default_material;
        return Ok((result, None));
    };

    let near_ids: [Vec<FaceId>; 2] = [0, 1].map(|s| {
        solids[s]
            .faces
            .iter()
            .zip(&near[s])
            .filter(|&(_, &n)| n)
            .map(|(f, _)| f.0)
            .collect()
    });
    // The seam between the rebuilt region and the base's untouched faces:
    // a face list that does not hold every edge of it whole cannot be
    // spliced, and the region grows there instead (`execute_region`).
    // Known limits, accepted: the comparison is by bits, so +0.0 and -0.0
    // are different keys; and a broken seam makes the loop grow rather
    // than try the untidied fallback, so that fallback never runs on it.
    let near_set: SecondaryMap<FaceId, ()> = near_ids[base].iter().map(|&f| (f, ())).collect();
    let seam = RegionSeam::of(operands[base], &near_set);
    let built = tidy::assemble_tidied(op, faces, tidy_polys, planarity_tol, report, true, |f| {
        if !grow.borrow().is_empty() {
            return Err(BooleanError::DegenerateContact);
        }
        let disturbed = seam.disturbed(operands[base], &near_set, &f);
        if !disturbed.is_empty() {
            let index: BTreeMap<FaceId, usize> = solids[base]
                .faces
                .iter()
                .enumerate()
                .map(|(i, f)| (f.0, i))
                .collect();
            grow.borrow_mut().extend(
                disturbed
                    .iter()
                    .filter_map(|f| index.get(f))
                    .map(|&i| (base, i)),
            );
            return Err(BooleanError::DegenerateContact);
        }
        let tags = split_tags(&f);
        let built = assemble_onto(
            base,
            &near_ids[base],
            f,
            planarity_tol,
            operands,
            blend_offset,
        )?;
        let ids = built.1.clone();
        Ok((built, ids.into_iter().zip(tags).collect::<Vec<_>>()))
    });
    if !grow.borrow().is_empty() {
        return Err(BooleanError::DegenerateContact);
    }
    let ((mut result, new_faces, new_prov), tags) = built?;

    // Re-derive the marks of the new faces' edges (as the full build does
    // for every edge), and of the kept faces' edges where a surface claim
    // could change the answer; the full build also drops every polygon
    // claim, which the kept faces' edges follow.
    let new_set: BTreeMap<FaceId, Provenance> = new_faces.iter().copied().zip(new_prov).collect();
    let base_side = base as u8;
    let prov = |f: FaceId| {
        new_set.get(&f).copied().or(Some(Provenance {
            side: base_side,
            face: f,
        }))
    };
    let mut edges: Vec<EdgeId> = Vec::new();
    for &f in &new_faces {
        for l in result.face_loops(f) {
            edges.extend(
                result
                    .loop_half_edges(l)
                    .map(|h| result.half_edges()[h].edge),
            );
        }
    }
    let claimed: Vec<FaceId> = result
        .faces()
        .iter()
        .filter(|(f, face)| face.surface.is_some() && !new_set.contains_key(f))
        .map(|(f, _)| f)
        .collect();
    for &f in &claimed {
        for l in result.face_loops(f) {
            edges.extend(
                result
                    .loop_half_edges(l)
                    .map(|h| result.half_edges()[h].edge),
            );
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let mut sources = near_ids.clone();
    sources[base].extend(claimed.iter().copied());
    let before: Vec<(bool, bool, Option<crate::sketch::CurveGeom>)> = edges
        .iter()
        .map(|&e| {
            let x = result.edges()[e];
            (x.soft, x.crease, x.curve)
        })
        .collect();
    crate::rederive::rederive_edges_after_boolean(
        &mut result,
        operands,
        [&sources[0], &sources[1]],
        &edges,
        &prov,
    );
    soften_split_diagonals(&mut result, &tags);
    let mut check: Vec<FaceId> = new_faces.clone();
    for (&e, was) in edges.iter().zip(&before) {
        let x = result.edges()[e];
        if (x.soft, x.crease, x.curve) != *was {
            for h in std::iter::once(x.half_edge).chain(x.twin_half_edge) {
                check.push(result.loops()[result.half_edges()[h].loop_id].face);
            }
        }
    }
    if result.edges().values().any(|e| e.polygon.is_some()) {
        for e in result.edges.values_mut() {
            e.polygon = None;
        }
    }
    check.sort_unstable();
    check.dedup();
    result
        .validate_local(&check)
        .map_err(|_| BooleanError::DegenerateContact)?;
    result.check_invariants();
    result.default_material = a.default_material;
    Ok((
        result,
        Some(BooleanTouch {
            new_faces,
            near: near_ids,
        }),
    ))
}

/// A copy of `obj` with `t` baked into every vertex position and face plane.
///
/// Connectivity is unchanged, so the result is structurally valid whenever the
/// input is. A singular `t` (or one that would squash a texture frame,
/// which only a near-singular stretch does) is refused as
/// [`BooleanError::SingularTransform`]; boolean callers pass a rigid
/// `b_to_a`, so that never fires in practice. A `t` that would leave a face
/// off its plane, which a rigid map placing the operand far from the
/// origin can, is refused as [`BooleanError::FaceNotFlat`]. A reflecting
/// `t` would bake a mirrored operand (every face turned over), still right
/// side out.
fn transformed_object(obj: &Object, t: &Transform) -> Result<Object, BooleanError> {
    use crate::transform::TransformError;
    let mut out = obj.clone();
    out.apply_transform(t).map_err(|e| match e {
        TransformError::FaceNotFlat => BooleanError::FaceNotFlat,
        TransformError::Singular
        | TransformError::DegenerateAxis
        | TransformError::DegenerateTextureFrame => BooleanError::SingularTransform,
    })?;
    Ok(out)
}

/// Flattens one face of `obj` to a [`FacePoly`].
fn face_poly(obj: &Object, fid: FaceId, side: u8, blend_offset: u32, far: bool) -> FacePoly {
    let f = &obj.faces()[fid];
    let mut ids: Vec<([u64; 3], VertexId)> = std::iter::once(f.outer_loop)
        .chain(f.inner_loops.iter().copied())
        .flat_map(|l| obj.loop_half_edges(l).collect::<Vec<_>>())
        .map(|h| {
            let v = obj.half_edges()[h].origin;
            (pos_key(obj.vertices()[v].position), v)
        })
        .collect();
    ids.sort_unstable();
    let flat_tol = if obj.planarity_tol > tol::PLANE_DIST {
        std::iter::once(f.outer_loop)
            .chain(f.inner_loops.iter().copied())
            .flat_map(|l| obj.loop_positions(l))
            .map(|p| f.plane.signed_distance(p).abs())
            .fold(tol::PLANE_DIST, f64::max)
    } else {
        tol::PLANE_DIST
    };
    FacePoly {
        plane_tol: obj.planarity_tol,
        flat_tol,
        outer: obj.loop_positions(f.outer_loop).collect(),
        holes: f
            .inner_loops
            .iter()
            .map(|&l| obj.loop_positions(l).collect())
            .collect(),
        plane: f.plane,
        material: f.material,
        back_material: f.back_material,
        uv_frame: f.uv_frame,
        surface: f.surface,
        // Operand B's blend ids move past A's largest, so two blends
        // never share an id on the result.
        blend: f.blend.map(|t| BlendTag {
            id: crate::topo::BlendId(t.id.0 + blend_offset),
            ..t
        }),
        attrs: obj.attr_dict(f.attrs).cloned().unwrap_or_default(),
        source: Provenance { side, face: fid },
        ids,
        far,
        split_from: None,
        pass_through: false,
        no_plane: false,
    }
}

/// Points where a face holds two rings through one vertex (a hole touching
/// its outer ring, or another hole), over every face of one operand.
fn touch_points(polys: &[FacePoly]) -> Vec<Point3> {
    polys
        .iter()
        .flat_map(|f| {
            let rings: Vec<&Vec<Point3>> =
                std::iter::once(&f.outer).chain(f.holes.iter()).collect();
            let mut found = Vec::new();
            for i in 0..rings.len() {
                for j in i + 1..rings.len() {
                    for &p in rings[i] {
                        if rings[j].contains(&p) {
                            found.push(p);
                        }
                    }
                }
            }
            found
        })
        .collect()
}

// ─────────────────────────────────────────── imported operands: the pre-pass

/// What [`refine_operand`] did to one operand: how many faces it split,
/// into how many triangles, and how many it passed through whole.
#[derive(Debug, Default, Clone, Copy)]
struct Refined {
    split: usize,
    triangles: usize,
    passed: usize,
}

/// An axis-aligned box `[lo, hi]`.
#[derive(Debug, Clone, Copy)]
struct Aabb {
    lo: Point3,
    hi: Point3,
}

impl Aabb {
    fn of_face(fp: &FacePoly) -> Aabb {
        let mut b = Aabb {
            lo: fp.outer[0],
            hi: fp.outer[0],
        };
        for &p in fp.outer.iter().chain(fp.holes.iter().flatten()) {
            b.lo = Point3::new(b.lo.x.min(p.x), b.lo.y.min(p.y), b.lo.z.min(p.z));
            b.hi = Point3::new(b.hi.x.max(p.x), b.hi.y.max(p.y), b.hi.z.max(p.z));
        }
        b
    }

    fn grown(self, by: f64) -> Aabb {
        let d = Vec3::new(by, by, by);
        Aabb {
            lo: self.lo - d,
            hi: self.hi + d,
        }
    }

    fn meets(&self, o: &Aabb) -> bool {
        boxes_overlap((self.lo, self.hi), (o.lo, o.hi))
    }

    fn union(self, o: &Aabb) -> Aabb {
        Aabb {
            lo: Point3::new(
                self.lo.x.min(o.lo.x),
                self.lo.y.min(o.lo.y),
                self.lo.z.min(o.lo.z),
            ),
            hi: Point3::new(
                self.hi.x.max(o.hi.x),
                self.hi.y.max(o.hi.y),
                self.hi.z.max(o.hi.z),
            ),
        }
    }
}

/// The bounding box of every face of one operand, and of all of them.
struct FaceBoxes {
    faces: Vec<Aabb>,
    all: Option<Aabb>,
}

impl FaceBoxes {
    fn of(polys: &[FacePoly]) -> FaceBoxes {
        let faces: Vec<Aabb> = polys.iter().map(Aabb::of_face).collect();
        let all = faces.iter().copied().reduce(|a, b| a.union(&b));
        FaceBoxes { faces, all }
    }

    /// Whether `b` meets the box of any face.
    fn any_meets(&self, b: &Aabb) -> bool {
        self.all.is_some_and(|all| all.meets(b)) && self.faces.iter().any(|f| f.meets(b))
    }
}

/// How far the farthest vertex of `fp` lies from its stored plane.
fn plane_deviation(fp: &FacePoly) -> f64 {
    fp.outer
        .iter()
        .chain(fp.holes.iter().flatten())
        .map(|&p| fp.plane.signed_distance(p).abs())
        .fold(0.0, f64::max)
}

/// The pre-pass over an imported operand (one whose faces are held only to
/// a tolerance looser than [`tol::PLANE_DIST`]), before the arrangement.
///
/// Seams on a face that is not flat to `tol::PLANE_DIST` would land on two
/// different planes either side of a shared edge, and the weld leaves such
/// places open or refuses. So each face that is **not flat** (a vertex past
/// `tol::PLANE_DIST` from its plane) and that the arrangement will imprint
/// (not one `far` from the other operand, which is kept or dropped whole):
///
/// - **near** the other operand (its box, grown by `reach`, meets the box of
///   a face of `partner`) is replaced by flat triangles through its own
///   vertices, bit for bit ([`triangulate`]), each wound with the face and
///   carrying its paint, UV frame, surface, blend tag, attributes and source
///   face (`split_from`, so the diagonals come out soft). A face that cannot
///   be triangulated is refused as [`BooleanError::OperandFaceNotFlat`].
/// - **far** from it is marked to pass through: nothing of the other
///   operand is near enough to cut it or to share its plane, so it is kept
///   or dropped whole, its loops verbatim.
///
/// Flat faces are left as they are, so an imported operand whose faces are
/// all flat takes exactly the path it takes without the pre-pass. Nothing
/// moves a vertex (DEVELOPMENT.md rule 4).
///
/// A seam that crosses an edge of a triangulated face lies in the boxes of
/// both faces on that edge, so the neighbor is near too: it is flat or
/// triangulated, and both compute the crossing in the same exact regime. A
/// pass-through face shares an edge with a triangulated one only where no
/// seam crosses it, and both keep the edge's own endpoints.
fn refine_operand(
    polys: Vec<FacePoly>,
    partner: &FaceBoxes,
    reach: f64,
    side: Side,
) -> Result<(Vec<FacePoly>, Refined), BooleanError> {
    let mut count = Refined::default();
    let mut out: Vec<FacePoly> = Vec::with_capacity(polys.len());
    for mut fp in polys {
        if fp.far {
            out.push(fp);
            continue;
        }
        let off = plane_deviation(&fp);
        if off <= tol::PLANE_DIST {
            out.push(fp);
            continue;
        }
        if !partner.any_meets(&Aabb::of_face(&fp).grown(reach)) {
            fp.pass_through = true;
            count.passed += 1;
            out.push(fp);
            continue;
        }
        let refuse = BooleanError::OperandFaceNotFlat {
            which: side.operand(),
            face: fp.source.face,
        };
        let normal = fp.plane.normal();
        let tris = triangulate(&fp.outer, &fp.holes, normal).ok_or(refuse)?;
        let all: Vec<Point3> = fp
            .outer
            .iter()
            .chain(fp.holes.iter().flatten())
            .copied()
            .collect();
        count.split += 1;
        count.triangles += tris.len();
        for t in tris {
            let outer = vec![all[t[0]], all[t[1]], all[t[2]]];
            // A triangle with no plane is narrower than the kernel keeps
            // (`tol::MIN_FACE_WIDTH`), not a face that will not flatten: it
            // is arranged on its own Newell plane and every piece of it
            // left for the tidy to merge. Only a triangle with no area at
            // all is too thin to arrange.
            let (plane, no_plane) =
                match crate::serialize::own_ring_plane(&outer, &[], tol::PLANE_DIST) {
                    Ok(plane) => (plane, false),
                    Err(_) => (
                        Plane::from_polygon_unfloored(&outer).ok_or(BooleanError::ResultTooThin)?,
                        true,
                    ),
                };
            if plane.normal().dot(normal) <= 0.0 {
                return Err(refuse);
            }
            out.push(FacePoly {
                plane_tol: fp.plane_tol,
                flat_tol: tol::PLANE_DIST,
                outer,
                holes: Vec::new(),
                plane,
                material: fp.material,
                back_material: fp.back_material,
                uv_frame: fp.uv_frame,
                surface: fp.surface,
                blend: fp.blend,
                attrs: fp.attrs.clone(),
                source: fp.source,
                ids: fp.ids.clone(),
                far: false,
                split_from: Some((side, fp.source.face)),
                pass_through: false,
                no_plane,
            });
        }
    }
    Ok((out, count))
}

/// An operand as a ray-cast target: every face's plane and padded box, in
/// face order, its loops read from the object only for the faces a ray
/// lands in the box of ([`ray_cast`]).
struct RaySolid<'a> {
    obj: &'a Object,
    faces: Vec<(FaceId, Plane, (Point3, Point3))>,
    /// Per face (parallel to `faces`), how far its vertices stray from its
    /// plane, or zero for a face flat to `tol::PLANE_DIST` (every face of a
    /// native operand). Ray casts treat hits that close to a face's
    /// boundary as grazing ([`ray_cast`]).
    off_plane: Vec<f64>,
}

impl<'a> RaySolid<'a> {
    fn new(obj: &'a Object) -> RaySolid<'a> {
        // Each face's box is padded past every point a seam on it can have:
        // twice how far its own vertices stray from its plane (at least
        // `PLANE_DIST`, all a native face may), and a margin for the seam's
        // own rounding. A flat face of an imported object pads exactly as
        // a native one does.
        let imported = obj.planarity_tol > tol::PLANE_DIST;
        let mut faces = Vec::with_capacity(obj.faces().len());
        let mut off_plane = Vec::with_capacity(obj.faces().len());
        for (fid, f) in obj.faces().iter() {
            let mut pts = std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .flat_map(|l| obj.loop_positions(l));
            let first = pts.next().unwrap_or(Point3::ORIGIN);
            let (mut lo, mut hi) = (first, first);
            let mut off = if imported {
                f.plane.signed_distance(first).abs()
            } else {
                0.0
            };
            for p in pts {
                lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
                if imported {
                    off = off.max(f.plane.signed_distance(p).abs());
                }
            }
            let pad = 2.0 * off.max(tol::PLANE_DIST) + 16.0 * tol::POINT_MERGE;
            let bx = (
                Point3::new(lo.x - pad, lo.y - pad, lo.z - pad),
                Point3::new(hi.x + pad, hi.y + pad, hi.z + pad),
            );
            faces.push((fid, f.plane, bx));
            // A flat face grazes exactly as a native one does.
            off_plane.push(if off > tol::PLANE_DIST { off } else { 0.0 });
        }
        RaySolid {
            obj,
            faces,
            off_plane,
        }
    }

    /// The union of the face boxes, `None` for no faces.
    fn bounds(&self) -> Option<(Point3, Point3)> {
        let mut it = self.faces.iter().map(|f| f.2);
        let (mut lo, mut hi) = it.next()?;
        for (a, b) in it {
            lo = Point3::new(lo.x.min(a.x), lo.y.min(a.y), lo.z.min(a.z));
            hi = Point3::new(hi.x.max(b.x), hi.y.max(b.y), hi.z.max(b.z));
        }
        Some((lo, hi))
    }
}

/// Arranges each face of one operand against the other, classifies every
/// sub-face by the coverage-based neighborhood test, and pushes the kept
/// (correctly oriented) ones into `out`. Returns how many sub-faces of this
/// operand the classification dropped.
///
/// A face is subdivided by transversal seams (where non-coplanar partner faces
/// cross its plane) *and* by the boundaries of any coplanar partner faces, so
/// each resulting sub-face is uniformly inside-or-outside every partner. Then
/// [`classify`] keeps it iff it separates inside-result from outside-result.
fn collect_faces(
    op: BooleanOp,
    this_side: Side,
    these: &[FacePoly],
    others: &[&FacePoly],
    solids: &[RaySolid; 2],
    touch_points: &[Point3],
    out: &mut Vec<OrientedFace>,
) -> Result<usize, BooleanError> {
    let mut kept = KeptFaces::new(out);
    // Sub-faces of this operand the classification dropped (both sides of
    // them equally in or out of the result): for a subtract, the patches of
    // `a`'s skin that lie inside `b`.
    let mut dropped = 0usize;
    let other_boxes: Vec<(Point3, Point3)> = others.iter().map(|o| padded_box(o)).collect();
    for (index, fp) in these.iter().enumerate() {
        if fp.far {
            // Untouched by the other operand and kept: passed through whole.
            let face = OrientedFace::new(
                Region {
                    outer: fp.outer.clone(),
                    holes: fp.holes.clone(),
                },
                fp,
                false,
                (this_side, index),
            )?;
            kept.push(face);
            continue;
        }
        if fp.pass_through {
            // Not flat, and no partner face comes near it: not imprinted,
            // one sub-face, its own loops, with no coplanar partner and
            // nothing to avoid.
            let region = Region {
                outer: fp.outer.clone(),
                holes: fp.holes.clone(),
            };
            let ctx = (op, this_side, solids);
            if !keep_region(ctx, (fp, index), region, &[], &[], &mut kept)? {
                dropped += 1;
            }
            continue;
        }

        // Partner faces of the other operand lying in this face's plane.
        let coplanar: Vec<&FacePoly> = others
            .iter()
            .copied()
            .filter(|o| coplanar_planes(fp, o))
            .collect();

        // Imprint: transversal seams (empty for parallel planes) plus coplanar
        // partner boundaries and partner edges lying in this face's plane, minus
        // anything coincident with `fp`'s own edges or an already-collected
        // segment.
        //
        // Coplanar partner boundaries are the *authoritative* imprint for any chord
        // they carry — their endpoints are exact partner vertices. Collect them up
        // front so a transversal seam that merely duplicates one can be dropped.
        let mut authoritative_edges: Vec<(Point3, Point3)> = coplanar
            .iter()
            .flat_map(|cp| {
                loop_edges(&cp.outer)
                    .chain(cp.holes.iter().flat_map(|h| loop_edges(h)))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Partner edges lying in `fp`'s plane whose *faces* are not coplanar with
        // it — tangency rows. Where the partner solid grazes this face's plane
        // along an edge (a lying prism touching a box face along a facet row),
        // the membership transition across that line is real: points of `fp`
        // just past the edge are inside the partner, points before it outside.
        // But the only plane-derived seams near it are the plane×plane lines of
        // the two *adjacent* partner faces, and each rides its face's boundary —
        // clipped to a region it touches only within floating-point noise of the
        // rotation that put it there, so the interval midpoint test drops both
        // and the transition is never imprinted; the strip past the tangent line
        // is then classified by a test point on the wrong side and the weld
        // leaks (a bogus DegenerateContact). The edge itself is the vertex-exact
        // authoritative seam, exactly as a coplanar rim is.
        //
        // Unless the partner only TOUCHES the plane along that edge, from a
        // touch point: when every partner face on the edge lies strictly on
        // one side of `fp`'s plane away from it, `fp`'s points on either side
        // of the line are in the same place relative to the partner, and
        // there is no transition to imprint. A cylinder whose wall is tangent
        // to this face along one generatrix — a circle tangent to an edge at
        // one point, pushed through — touches it so, starting where a face of
        // `fp`'s solid holds two loops through one vertex; imprinting the
        // line would split the face there and leave four faces on one edge.
        // Its transversal copies are dropped too. (A line contact from no
        // touch point — a ridge pressed into a face — stays imprinted and
        // refuses at the weld, as before.)
        let mut tangent_only: Vec<(Point3, Point3)> = Vec::new();
        for &ofp in others {
            if coplanar_planes(fp, ofp) {
                continue; // its edges are already collected above
            }
            let in_plane = |p: Point3, q: Point3| {
                fp.plane.signed_distance(p).abs() <= tol::PLANE_DIST
                    && fp.plane.signed_distance(q).abs() <= tol::PLANE_DIST
            };
            for (p, q) in loop_edges(&ofp.outer).chain(ofp.holes.iter().flat_map(|h| loop_edges(h)))
            {
                if in_plane(p, q) {
                    if (touch_points
                        .iter()
                        .any(|t| t.approx_eq(p, tol::POINT_MERGE))
                        || touch_points
                            .iter()
                            .any(|t| t.approx_eq(q, tol::POINT_MERGE)))
                        && partner_only_touches_along(fp, others, p, q)
                    {
                        tangent_only.push((p, q));
                    } else {
                        authoritative_edges.push((p, q));
                    }
                }
            }
        }

        let mut seams: Vec<(Point3, Point3)> = Vec::new();
        let fp_box = padded_box(fp);
        for (ofp, ob) in others.iter().zip(&other_boxes) {
            // A seam lies on both faces, so faces whose padded boxes are
            // disjoint have none — skipping them changes nothing (KB-25).
            if !boxes_overlap(fp_box, *ob) {
                continue;
            }
            for seg in seam_segments(fp, ofp) {
                // A partner face perpendicular to `fp` that meets it exactly along a
                // coplanar partner's rim — a cylinder's side wall meeting this face
                // along its cap circle — yields a transversal seam collinear with,
                // and covered by, that rim edge. It is the same chord computed two
                // ways; clipped differently (the transversal copy is trimmed to
                // `fp`, the rim edge is not) they seed near-duplicate collinear
                // segments that the region tracer resolves into sliver vertices,
                // breaking the coincident-patch dedup and the final weld. Drop the
                // transversal copy in favour of the vertex-exact coplanar or
                // in-plane partner edge.
                if segment_lies_along_any(seg, &authoritative_edges)
                    || segment_lies_along_any(seg, &tangent_only)
                {
                    continue;
                }
                push_unique_segment(&mut seams, seg, fp);
            }
        }
        for &seg in &authoritative_edges {
            push_unique_segment(&mut seams, seg, fp);
        }

        // Nothing crosses the face: it is its own one region (the region
        // tracer would trace back the same loops), unless a loop has two
        // corners closer than the tracer merges.
        let regions = if seams.is_empty() && !has_merged_corner(fp) {
            vec![Region {
                outer: fp.outer.clone(),
                holes: fp.holes.clone(),
            }]
        } else {
            arrange_face(fp, &seams, this_side)?
        };
        for region in regions {
            let ctx = (op, this_side, solids);
            if !keep_region(
                ctx,
                (fp, index),
                region,
                &tangent_only,
                &coplanar,
                &mut kept,
            )? {
                dropped += 1;
            }
        }
    }
    Ok(dropped)
}

/// Classifies one sub-face `region` of `fp` (face `index` of the operand on
/// `ctx`'s side, in the op `ctx` names against both operands) and, if it
/// lies on the result's boundary, keeps it correctly wound unless an equal
/// patch is already kept. Returns `false` when the classification dropped
/// it.
fn keep_region(
    (op, this_side, solids): (BooleanOp, Side, &[RaySolid; 2]),
    (fp, index): (&FacePoly, usize),
    region: Region,
    tangent_only: &[(Point3, Point3)],
    coplanar: &[&FacePoly],
    kept: &mut KeptFaces<'_>,
) -> Result<bool, BooleanError> {
    let na = fp.plane.normal();
    // Off any line the partner only touches: a point there sits on
    // the partner's boundary, where its membership is undecidable.
    // No such point (every candidate on such a line) is no decidable
    // classification: refused, never guessed.
    let c = interior_point(&region, na, tangent_only).ok_or(BooleanError::DegenerateContact)?;
    // Coplanar partner boundaries can trace regions outside the original
    // face; keep only sub-faces actually inside it.
    if !region_contains(fp, c) {
        return Ok(true);
    }
    let Some(flip) = classify(op, this_side, na, c, solids, coplanar) else {
        return Ok(false);
    };
    let face = OrientedFace::new(region, fp, flip, (this_side, index))?;
    // A coincident shared patch is produced by both operands (e.g.
    // a shared ground for union, or A's top vs B's flipped bottom for
    // subtract). Both describe one boundary face — keep the first.
    kept.push_unless_coincident(face);
    Ok(true)
}

/// The kept faces, indexed for the coincident-patch test.
///
/// [`faces_coincide`]`(f, g)` needs every outer vertex of `f` within
/// [`tol::POINT_MERGE`] of an outer vertex of `g`, so a kept face `f` that
/// coincides with a new face `g` has its first vertex near one of `g`'s.
/// Kept faces are indexed by the cell of their first vertex, and a new face
/// is tested against the faces in the cells around its vertices only: the
/// same answer as testing every kept face (KB-25), in time that does not
/// grow with their number.
struct KeptFaces<'a> {
    out: &'a mut Vec<OrientedFace>,
    by_first: BTreeMap<[i64; 3], Vec<usize>>,
}

impl<'a> KeptFaces<'a> {
    fn new(out: &'a mut Vec<OrientedFace>) -> KeptFaces<'a> {
        let mut by_first: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
        for (i, f) in out.iter().enumerate() {
            by_first.entry(merge_cell(f.outer[0])).or_default().push(i);
        }
        KeptFaces { out, by_first }
    }

    /// Whether a kept face coincides with `g` ([`faces_coincide`]).
    fn any_coincident(&self, g: &OrientedFace) -> bool {
        g.outer.iter().any(|&q| {
            neighbor_cells(merge_cell(q)).any(|c| {
                self.by_first
                    .get(&c)
                    .is_some_and(|list| list.iter().any(|&i| faces_coincide(&self.out[i], g)))
            })
        })
    }

    fn push_unless_coincident(&mut self, face: OrientedFace) {
        if !self.any_coincident(&face) {
            self.push(face);
        }
    }

    fn push(&mut self, face: OrientedFace) {
        self.by_first
            .entry(merge_cell(face.outer[0]))
            .or_default()
            .push(self.out.len());
        self.out.push(face);
    }
}

/// The grid cell of `p` for merge-distance lookups: cells twice
/// [`tol::POINT_MERGE`] wide, so every point within that distance of `p`
/// lies in `p`'s cell or one of its 26 neighbors, rounding included.
fn merge_cell(p: Point3) -> [i64; 3] {
    // Clamped short of the integer range, so a neighbor of the cell a
    // far-off coordinate falls in can still be named.
    let cell = |x: f64| {
        (x / (2.0 * tol::POINT_MERGE))
            .floor()
            .clamp(-4.0e18, 4.0e18) as i64
    };
    [cell(p.x), cell(p.y), cell(p.z)]
}

/// `c` and its 26 neighbors.
fn neighbor_cells(c: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    (-1..=1).flat_map(move |dx| {
        (-1..=1).flat_map(move |dy| (-1..=1).map(move |dz| [c[0] + dx, c[1] + dy, c[2] + dz]))
    })
}

/// Whether a sub-face (interior point `c`, owning-face normal `na`, from operand
/// `this_side`) lies on the result's boundary, and with which winding: `Some(true)`
/// to keep it flipped, `Some(false)` to keep as-is, `None` to drop it.
///
/// Membership is evaluated just inside (`-na`) and just outside (`+na`) the face.
/// This operand's membership there is definitional (the point is on its
/// boundary). The other operand's is exact when a coplanar partner covers `c`
/// (it occupies the `-partner_normal` side), else a ray cast (the point is then
/// off the other's boundary, so parity is clean). The face is kept iff
/// inside-result differs across it.
fn classify(
    op: BooleanOp,
    this_side: Side,
    na: Vec3,
    c: Point3,
    solids: &[RaySolid; 2],
    coplanar: &[&FacePoly],
) -> Option<bool> {
    let others = match this_side {
        Side::A => &solids[1],
        Side::B => &solids[0],
    };
    let (other_in, other_out) = match cover_normal(coplanar, c) {
        Some(no) => (na.dot(no) > 0.0, na.dot(no) < 0.0),
        None => {
            let m = point_in_solid(others, c);
            (m, m)
        }
    };
    // `-na` is inside this operand, `+na` is outside.
    let (a_in, a_out, b_in, b_out) = match this_side {
        Side::A => (true, false, other_in, other_out),
        Side::B => (other_in, other_out, true, false),
    };
    let res_in = op_member(op, a_in, b_in);
    let res_out = op_member(op, a_out, b_out);
    if res_in == res_out {
        return None;
    }
    // `res_out` true ⇒ the result interior is on the `+na` side, so reverse.
    Some(res_out)
}

/// Membership of a point in `A op B`, from its memberships in A and B.
fn op_member(op: BooleanOp, a: bool, b: bool) -> bool {
    match op {
        BooleanOp::Union => a || b,
        BooleanOp::Intersect => a && b,
        BooleanOp::Subtract => a && !b,
    }
}

/// Outward normal of the coplanar partner whose region contains `c`, if any.
fn cover_normal(coplanar: &[&FacePoly], c: Point3) -> Option<Vec3> {
    coplanar
        .iter()
        .find(|cp| region_contains(**cp, c))
        .map(|cp| cp.plane.normal())
}

/// Whether every face of `others` holding the edge `p`–`q` (which lies in
/// `fp`'s plane) lies strictly on one side of that plane away from the edge:
/// every other vertex farther than [`tol::PLANE_DIST`], all on one side. The
/// partner then touches `fp`'s plane along the edge without crossing it.
fn partner_only_touches_along(fp: &FacePoly, others: &[&FacePoly], p: Point3, q: Point3) -> bool {
    let mut side = 0.0f64;
    let mut faces = 0;
    for &o in others {
        let holds = std::iter::once(&o.outer)
            .chain(o.holes.iter())
            .any(|r| loop_edges(r).any(|(a, b)| seg_eq(a, b, p, q)));
        if !holds {
            continue;
        }
        faces += 1;
        for r in std::iter::once(&o.outer).chain(o.holes.iter()) {
            for &x in r {
                if x.approx_eq(p, tol::POINT_MERGE) || x.approx_eq(q, tol::POINT_MERGE) {
                    continue;
                }
                let d = fp.plane.signed_distance(x);
                if d.abs() <= tol::PLANE_DIST || (side != 0.0 && d.signum() != side) {
                    return false;
                }
                side = d.signum();
            }
        }
    }
    faces == 2 && side != 0.0
}

/// True if the two faces lie in the same (parallel and coincident) plane.
fn coplanar_planes(fa: &FacePoly, fb: &FacePoly) -> bool {
    if fa.plane.normal().cross(fb.plane.normal()).length() > tol::NORMAL_DIRECTION {
        return false;
    }
    fa.plane.signed_distance(fb.outer[0]).abs() <= tol::PLANE_DIST
}

/// The edges of a closed loop as `(start, end)` pairs.
fn loop_edges(loop_pts: &[Point3]) -> impl Iterator<Item = (Point3, Point3)> + '_ {
    let n = loop_pts.len();
    (0..n).map(move |i| (loop_pts[i], loop_pts[(i + 1) % n]))
}

/// Appends `seg` to `seams` unless it is degenerate, coincides with an edge of
/// `fp`'s own boundary, or duplicates a segment already collected.
fn push_unique_segment(seams: &mut Vec<(Point3, Point3)>, seg: (Point3, Point3), fp: &FacePoly) {
    let (p, q) = seg;
    if p.approx_eq(q, tol::POINT_MERGE) {
        return;
    }
    if loop_edges(&fp.outer)
        .chain(fp.holes.iter().flat_map(|h| loop_edges(h)))
        .any(|(a, b)| seg_eq(p, q, a, b))
    {
        return;
    }
    if seams.iter().any(|&(a, b)| seg_eq(p, q, a, b)) {
        return;
    }
    seams.push(seg);
}

/// Unordered segment equality within [`tol::POINT_MERGE`].
fn seg_eq(a0: Point3, a1: Point3, b0: Point3, b1: Point3) -> bool {
    (a0.approx_eq(b0, tol::POINT_MERGE) && a1.approx_eq(b1, tol::POINT_MERGE))
        || (a0.approx_eq(b1, tol::POINT_MERGE) && a1.approx_eq(b0, tol::POINT_MERGE))
}

/// True if segment `s` lies *along* any segment in `along`.
fn segment_lies_along_any(s: (Point3, Point3), along: &[(Point3, Point3)]) -> bool {
    along.iter().any(|&c| segment_lies_along(s, c))
}

/// Whether segment `s` is contained in segment `c`: both of `s`'s endpoints lie
/// within [`tol::POINT_MERGE`] of `c` (point-to-segment distance). Because both
/// endpoints sit in `c`'s (convex) tolerance neighborhood, so does all of `s` —
/// the two are the same chord, and `s` carries nothing `c` does not.
///
/// Containment is intentionally one-directional (`s ⊆ c`, not `c ⊆ s`): the only
/// seam we drop is a wall's transversal copy of the coplanar rim it stands on,
/// and clipping that copy to the face can only shorten it relative to the rim
/// edge — so the redundant copy is always the subset. A rim edge shorter than a
/// collinear seam from an *independent* partner does not arise from one
/// watertight solid meeting another along a shared rim.
fn segment_lies_along(s: (Point3, Point3), c: (Point3, Point3)) -> bool {
    point_segment_distance(s.0, c.0, c.1) <= tol::POINT_MERGE
        && point_segment_distance(s.1, c.0, c.1) <= tol::POINT_MERGE
}

/// Whether two oriented faces are the same boundary patch: identical outer
/// vertex set (within tolerance) and the same outward direction. Such a pair
/// arises when both operands claim a coincident coplanar patch; only one belongs
/// in the result.
fn faces_coincide(f: &OrientedFace, g: &OrientedFace) -> bool {
    f.outer.len() == g.outer.len()
        && f.plane.normal().dot(g.plane.normal()) > 0.0
        && f.outer
            .iter()
            .all(|p| g.outer.iter().any(|q| p.approx_eq(*q, tol::POINT_MERGE)))
}

impl OrientedFace {
    /// The kept sub-face `region` of `fp` (face `origin` of its operand),
    /// reversed when `flip`.
    ///
    /// A flipped piece refits its plane to the new winding (the subtracted
    /// solid's walls face into the removed volume). A piece too narrow to
    /// have a plane (`tol::MIN_FACE_WIDTH`) keeps its source face's plane,
    /// turned over, as a placeholder and is marked `no_plane`: the tidy
    /// merges it away, and the untidied result refuses it as too thin. Such
    /// a piece is the operand's fault, [`BooleanError::DegenerateInput`],
    /// when its source face was degenerate itself.
    fn new(
        region: Region,
        fp: &FacePoly,
        flip: bool,
        origin: (Side, usize),
    ) -> Result<OrientedFace, BooleanError> {
        let mut outer_ids: Vec<Ident> = region.outer.iter().map(|&p| fp.ident(p)).collect();
        let mut hole_ids: Vec<Vec<Ident>> = region
            .holes
            .iter()
            .map(|h| h.iter().map(|&p| fp.ident(p)).collect())
            .collect();
        let (outer, holes, plane, no_plane) = if flip {
            outer_ids.reverse();
            for h in &mut hole_ids {
                h.reverse();
            }
            let mut outer = region.outer;
            outer.reverse();
            let holes = region
                .holes
                .into_iter()
                .map(|mut h| {
                    h.reverse();
                    h
                })
                .collect::<Vec<Vec<Point3>>>();
            // The plane the face will carry and a file refit: the refit
            // from the ring when every boundary point fits it, else the
            // ring-order fit, as every other fit site picks
            // ([`crate::serialize::own_ring_plane`]). An imported operand's
            // cut can leave a ring under the area and width floors the file
            // format still accepts for it (D-70).
            let hole_pts: Vec<Point3> = holes.iter().flatten().copied().collect();
            let fit =
                crate::serialize::own_ring_plane(&outer, &hole_pts, tol::PLANE_DIST).or_else(|e| {
                    if fp.plane_tol > tol::PLANE_DIST {
                        crate::serialize::outer_ring_plane(&outer, true)
                    } else {
                        Err(e)
                    }
                });
            match fit {
                Ok(plane) => (outer, holes, plane, fp.no_plane),
                Err(_) if crate::health::ring_is_degenerate(&fp.outer) => {
                    return Err(BooleanError::DegenerateInput {
                        at: centroid(&outer),
                    });
                }
                Err(_) => (outer, holes, fp.plane.flipped(), true),
            }
        } else {
            (region.outer, region.holes, fp.plane, fp.no_plane)
        };
        Ok(OrientedFace {
            outer,
            holes,
            plane,
            material: fp.material,
            back_material: fp.back_material,
            // UV frame propagates to boolean result faces ( ext.), and so
            // does the analytic surface (a sub-face of a chord facet is on
            // the same cylinder; a flipped one still is — the reference is
            // orientation-free).
            uv_frame: fp.uv_frame,
            surface: fp.surface,
            blend: fp.blend,
            // The face's dictionary rides with it, like its paint.
            attrs: fp.attrs.clone(),
            source: fp.source,
            outer_ids,
            hole_ids,
            split_from: fp.split_from,
            no_plane,
            origin,
            tidy_group: None,
        })
    }
}

// ─────────────────────────────────────────────────────────── seam computation

/// `fp`'s bounding box, padded past every point a seam on it can have: its
/// loops lie within `flat_tol` of its plane, and a seam point lies inside
/// the loops' projection onto it.
fn padded_box(fp: &FacePoly) -> (Point3, Point3) {
    let pad = 2.0 * fp.flat_tol + 16.0 * tol::POINT_MERGE;
    let (mut lo, mut hi) = (fp.outer[0], fp.outer[0]);
    for &p in fp.outer.iter().chain(fp.holes.iter().flatten()) {
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (
        Point3::new(lo.x - pad, lo.y - pad, lo.z - pad),
        Point3::new(hi.x + pad, hi.y + pad, hi.z + pad),
    )
}

fn boxes_overlap(a: (Point3, Point3), b: (Point3, Point3)) -> bool {
    a.0.x <= b.1.x
        && b.0.x <= a.1.x
        && a.0.y <= b.1.y
        && b.0.y <= a.1.y
        && a.0.z <= b.1.z
        && b.0.z <= a.1.z
}

/// The intersection segments of two non-coplanar faces: the line of their planes
/// clipped to both face regions. Empty if the planes are parallel or the regions
/// do not overlap along the line.
fn seam_segments(fa: &FacePoly, fb: &FacePoly) -> Vec<(Point3, Point3)> {
    let Some((p0, dir)) = plane_line(&fa.plane, &fb.plane) else {
        return Vec::new();
    };
    let ia = line_region_intervals(p0, dir, fa);
    let ib = line_region_intervals(p0, dir, fb);
    let mut out = Vec::new();
    for &(a0, a1) in &ia {
        for &(b0, b1) in &ib {
            let lo = a0.max(b0);
            let hi = a1.min(b1);
            if hi - lo > tol::POINT_MERGE {
                out.push((seam_end(p0, dir, lo, fa, fb), seam_end(p0, dir, hi, fa, fb)));
            }
        }
    }
    out
}

/// The seam point at parameter `t`. On native faces it is the point of the
/// planes' line. Where `t` is a crossing of an IMPORTED face's own boundary
/// edge (a face flat only to its object's tolerance, so the edge lies off the
/// fitted plane), the seam must end ON that edge, at the same point the face
/// across the edge computes: the edge meets the other face's plane there,
/// computed from the edge's endpoints in canonical order so both faces get
/// identical bits (KB-25; shared by construction, never welded).
fn seam_end(p0: Point3, dir: Vec3, t: f64, fa: &FacePoly, fb: &FacePoly) -> Point3 {
    let on_line = p0 + dir * t;
    for (this, other) in [(fa, fb), (fb, fa)] {
        if this.flat_tol <= tol::PLANE_DIST {
            continue;
        }
        let normal = this.plane.normal();
        let loops = std::iter::once(&this.outer).chain(this.holes.iter());
        for lp in loops {
            let n = lp.len();
            for i in 0..n {
                let (v0, v1) = (lp[i], lp[(i + 1) % n]);
                let Some(s) = line_segment_param(p0, dir, v0, v1, normal) else {
                    continue;
                };
                if (s - t).abs() > tol::POINT_MERGE {
                    continue;
                }
                let (a, b) = if pos_order(v0, v1) {
                    (v0, v1)
                } else {
                    (v1, v0)
                };
                let (da, db) = (
                    other.plane.signed_distance(a),
                    other.plane.signed_distance(b),
                );
                if (da - db).abs() <= tol::NORMALIZE_MIN_LENGTH {
                    continue;
                }
                return a + (b - a) * (da / (da - db));
            }
        }
    }
    on_line
}

/// Whether `a` sorts before (or equals) `b` lexicographically by f64 total order.
fn pos_order(a: Point3, b: Point3) -> bool {
    a.x.total_cmp(&b.x)
        .then(a.y.total_cmp(&b.y))
        .then(a.z.total_cmp(&b.z))
        .is_le()
}

/// A point on, and the unit direction of, the line where two planes meet, or
/// `None` if they are (near) parallel.
fn plane_line(pa: &Plane, pb: &Plane) -> Option<(Point3, Vec3)> {
    let (na, nb) = (pa.normal(), pb.normal());
    let dir = na.cross(nb).normalized().ok()?;
    let c = na.dot(nb);
    let denom = 1.0 - c * c;
    if denom < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let off_a = -pa.signed_distance(Point3::ORIGIN);
    let off_b = -pb.signed_distance(Point3::ORIGIN);
    let p0 =
        Point3::ORIGIN + na * ((off_a - off_b * c) / denom) + nb * ((off_b - off_a * c) / denom);
    Some((p0, dir))
}

/// The parameter intervals (along `dir` from `p0`) where the line lies inside the
/// face region. The line is assumed to lie in the face plane.
fn line_region_intervals(p0: Point3, dir: Vec3, fp: &FacePoly) -> Vec<(f64, f64)> {
    let normal = fp.plane.normal();
    let mut params = Vec::new();
    let mut collect = |loop_pts: &[Point3]| {
        let n = loop_pts.len();
        for i in 0..n {
            if let Some(t) = line_segment_param(p0, dir, loop_pts[i], loop_pts[(i + 1) % n], normal)
            {
                params.push(t);
            }
        }
    };
    collect(&fp.outer);
    for h in &fp.holes {
        collect(h);
    }
    params.sort_by(|a, b| a.partial_cmp(b).unwrap());
    params.dedup_by(|a, b| (*a - *b).abs() <= tol::POINT_MERGE);

    let mut intervals = Vec::new();
    for w in params.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        if hi - lo <= tol::POINT_MERGE {
            continue;
        }
        let mid = p0 + dir * (0.5 * (lo + hi));
        if region_contains(fp, mid) {
            intervals.push((lo, hi));
        }
    }
    intervals
}

/// The parameter `t` along the line `p0 + t·dir` where it crosses the segment
/// `v0`–`v1` (both in the same plane, spanned by `normal`), or `None` if they are
/// parallel or the crossing is off the segment.
fn line_segment_param(p0: Point3, dir: Vec3, v0: Point3, v1: Point3, normal: Vec3) -> Option<f64> {
    let (u, w) = plane_axes(normal);
    let proj = |v: Vec3| (v.dot(u), v.dot(w));
    let (ax, ay) = proj(dir);
    let e = v1 - v0;
    let (ex, ey) = proj(e);
    let (rx, ry) = proj(v0 - p0);
    // Solve [ax, -ex; ay, -ey] · [t, s] = [rx, ry].
    let det = ax * (-ey) - (-ex) * ay;
    if det.abs() < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let t = (rx * (-ey) - (-ex) * ry) / det;
    let s = (ax * ry - ay * rx) / det;
    if (-tol::POINT_MERGE..=1.0 + tol::POINT_MERGE).contains(&s) {
        Some(t)
    } else {
        None
    }
}

// ───────────────────────────────────────────────────────────────── arrangement

/// Partitions `fp` into sub-faces by imprinting its boundary and `seams` onto a
/// fresh sketch on its plane and reading back the traced regions.
///
/// A boundary point off the plane ([`SketchError::PointOffPlane`]) is a face
/// that is not flat enough to imprint, [`BooleanError::OperandFaceNotFlat`]
/// naming it (the pre-pass splits such faces of an imported operand, so this
/// is a backstop); a seam point off it is [`BooleanError::ArrangementFailed`].
/// A zero-length piece of either is a degenerate operand,
/// [`BooleanError::DegenerateInput`] (KB-25). Any other sketch failure is
/// [`BooleanError::ArrangementFailed`].
fn arrange_face(
    fp: &FacePoly,
    seams: &[(Point3, Point3)],
    side: Side,
) -> Result<Vec<Region>, BooleanError> {
    let boundary_err = |e: SketchError, at: Point3| match e {
        SketchError::PointOffPlane { .. } => BooleanError::OperandFaceNotFlat {
            which: side.operand(),
            face: fp.source.face,
        },
        SketchError::DegenerateSegment => BooleanError::DegenerateInput { at },
        _ => BooleanError::ArrangementFailed,
    };
    let mut sketch = Sketch::on_plane_with_tol(fp.plane, fp.flat_tol);
    add_loop(&mut sketch, &fp.outer).map_err(|(e, at)| boundary_err(e, at))?;
    for h in &fp.holes {
        add_loop(&mut sketch, h).map_err(|(e, at)| boundary_err(e, at))?;
    }
    for &(p, q) in seams {
        // A zero-length seam carries no information; skip rather than error.
        if !p.approx_eq(q, tol::POINT_MERGE) {
            sketch.add_segment(p, q).map_err(|e| match e {
                SketchError::DegenerateSegment => BooleanError::DegenerateInput { at: p },
                _ => BooleanError::ArrangementFailed,
            })?;
        }
    }
    Ok(sketch
        .regions()
        .values()
        .map(|r| Region {
            outer: r
                .outer
                .iter()
                .map(|&v| sketch.vertices()[v].position)
                .collect(),
            holes: r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| sketch.vertices()[v].position).collect())
                .collect(),
        })
        .collect())
}

/// Whether a loop of `fp` has consecutive corners within `POINT_MERGE`
/// (which [`add_loop`] would close up).
fn has_merged_corner(fp: &FacePoly) -> bool {
    std::iter::once(&fp.outer).chain(fp.holes.iter()).any(|lp| {
        let n = lp.len();
        (0..n).any(|i| lp[i].approx_eq(lp[(i + 1) % n], tol::POINT_MERGE))
    })
}

/// Adds a closed loop of positions to `sketch` as edges; a refusal comes
/// back with the point it started at.
fn add_loop(sketch: &mut Sketch, loop_pts: &[Point3]) -> Result<(), (SketchError, Point3)> {
    let n = loop_pts.len();
    for i in 0..n {
        let (p, q) = (loop_pts[i], loop_pts[(i + 1) % n]);
        if p.approx_eq(q, tol::POINT_MERGE) {
            continue;
        }
        sketch.add_segment(p, q).map_err(|e| (e, p))?;
    }
    Ok(())
}

/// A point strictly inside the region, robust to holes (annular regions, where
/// every outer-boundary ear centroid would fall in the hole).
///
/// From each outer-edge midpoint we march inward (toward the region interior)
/// until the nearest boundary — outer or hole — and take the half-distance
/// point, which is guaranteed strictly interior because the ray is inside the
/// region from the midpoint until that first crossing. A point within
/// [`tol::POINT_MERGE`] of an `avoid` segment is passed over: other edges'
/// rays, then other depths along them, then the centroid (when it is inside
/// and clear) are tried, and `None` when every one lies on such a segment.
fn interior_point(region: &Region, normal: Vec3, avoid: &[(Point3, Point3)]) -> Option<Point3> {
    let mut boundary: Vec<(Point3, Point3)> = Vec::new();
    let mut push_loop = |pts: &[Point3]| {
        for i in 0..pts.len() {
            boundary.push((pts[i], pts[(i + 1) % pts.len()]));
        }
    };
    push_loop(&region.outer);
    for h in &region.holes {
        push_loop(h);
    }
    let clear = |c: Point3| {
        !avoid
            .iter()
            .any(|&(p, q)| point_segment_distance(c, p, q) <= tol::POINT_MERGE)
    };

    let o = &region.outer;
    let mut any_ray = false;
    // The half-way point first (the only one tried with nothing to avoid,
    // so the common case is unchanged), then other depths along each ray.
    for frac in [0.5, 0.25, 0.75, 0.125, 0.875] {
        for i in 0..o.len() {
            let (a, b) = (o[i], o[(i + 1) % o.len()]);
            let Ok(edge) = (b - a).normalized() else {
                continue;
            };
            // Interior is to the left of a CCW outer edge: normal × edge.
            let Ok(inward) = normal.cross(edge).normalized() else {
                continue;
            };
            let mid = Point3::ORIGIN + (a.to_vec() + b.to_vec()) * 0.5;
            let mut nearest = f64::INFINITY;
            for &(c, d) in &boundary {
                if let Some(t) = line_segment_param(mid, inward, c, d, normal)
                    && t > tol::POINT_MERGE
                    && t < nearest
                {
                    nearest = t;
                }
            }
            if nearest.is_finite() {
                any_ray = true;
                let c = mid + inward * (frac * nearest);
                if clear(c) {
                    return Some(c);
                }
            }
        }
    }
    let c = centroid(o);
    let inside = point_inside_polygon(c, o, normal)
        && !region
            .holes
            .iter()
            .any(|h| point_inside_polygon(c, h, normal));
    if !any_ray || (inside && clear(c)) {
        return Some(c);
    }
    None
}

fn centroid(pts: &[Point3]) -> Point3 {
    let sum = pts.iter().fold(Vec3::ZERO, |acc, p| acc + p.to_vec());
    Point3::ORIGIN + sum / (pts.len() as f64)
}

// ───────────────────────────────────────────────────────────────────── queries

/// True if `p` is strictly inside the face region: inside the outer boundary and
/// outside every hole.
/// A face's plane and loops, as the containment queries read them.
trait Outline {
    fn plane(&self) -> &Plane;
    fn outer(&self) -> &[Point3];
    fn holes(&self) -> &[Vec<Point3>];
}

impl Outline for FacePoly {
    fn plane(&self) -> &Plane {
        &self.plane
    }
    fn outer(&self) -> &[Point3] {
        &self.outer
    }
    fn holes(&self) -> &[Vec<Point3>] {
        &self.holes
    }
}

/// One face's loops, read for a ray hit ([`ray_cast`]).
struct FaceLoops {
    plane: Plane,
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
}

impl Outline for FaceLoops {
    fn plane(&self) -> &Plane {
        &self.plane
    }
    fn outer(&self) -> &[Point3] {
        &self.outer
    }
    fn holes(&self) -> &[Vec<Point3>] {
        &self.holes
    }
}

fn region_contains(fp: &impl Outline, p: Point3) -> bool {
    let n = fp.plane().normal();
    point_inside_polygon(p, fp.outer(), n)
        && !fp.holes().iter().any(|h| point_inside_polygon(p, h, n))
}

/// A few non-axis-aligned ray directions; the first one whose cast grazes no
/// face boundary decides the parity.
const RAY_DIRS: [Vec3; 4] = [
    Vec3::new(0.4651, 0.7345, 0.4949),
    Vec3::new(0.8012, -0.3461, 0.4877),
    Vec3::new(-0.3299, 0.5813, 0.7438),
    Vec3::new(0.1234, -0.5678, 0.8137),
];

/// Point-in-solid by ray-cast parity against a watertight face set. Tries each
/// candidate direction until one casts cleanly (no boundary graze).
fn point_in_solid(solid: &RaySolid, p: Point3) -> bool {
    for d in RAY_DIRS {
        let (count, ambiguous) = ray_cast(solid, p, d);
        if !ambiguous {
            return count % 2 == 1;
        }
    }
    ray_cast(solid, p, RAY_DIRS[0]).0 % 2 == 1
}

/// Counts forward crossings of the ray `p + t·d` (t > 0) through the face set,
/// flagging the cast ambiguous if a hit grazes a face boundary or the ray lies
/// in a face's plane through `p`.
///
/// A hit outside a face's padded box (which holds every point of the face,
/// and every point of its plane inside its outline, with margin) is neither
/// inside the face nor near its boundary, so only faces whose box the hit
/// lands in read their loops: the same count as testing every face.
fn ray_cast(solid: &RaySolid, p: Point3, d: Vec3) -> (usize, bool) {
    let mut count = 0;
    let mut ambiguous = false;
    for (&(fid, plane, (lo, hi)), &off_plane) in solid.faces.iter().zip(&solid.off_plane) {
        let denom = d.dot(plane.normal());
        let signed = plane.signed_distance(p);
        if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
            if signed.abs() <= tol::PLANE_DIST {
                ambiguous = true;
            }
            continue;
        }
        let t = -signed / denom;
        if t <= tol::POINT_MERGE {
            continue;
        }
        let hit = p + d * t;
        if hit.x < lo.x
            || hit.x > hi.x
            || hit.y < lo.y
            || hit.y > hi.y
            || hit.z < lo.z
            || hit.z > hi.z
        {
            continue;
        }
        let obj = solid.obj;
        let f = &obj.faces()[fid];
        let fp = FaceLoops {
            plane,
            outer: obj.loop_positions(f.outer_loop).collect(),
            holes: f
                .inner_loops
                .iter()
                .map(|&l| obj.loop_positions(l).collect())
                .collect(),
        };
        // A face that strays from its plane (an imported one) is only near
        // the plane the hit is computed on: a hit within that stray of its
        // boundary, stretched by how obliquely the ray meets it, may belong
        // to the face next door.
        let band = tol::POINT_MERGE.max(2.0 * off_plane / denom.abs());
        if near_polygon_boundary(hit, &fp, band) {
            ambiguous = true;
            continue;
        }
        if region_contains(&fp, hit) {
            count += 1;
        }
    }
    (count, ambiguous)
}

/// True if `p` lies within `band` of any boundary segment of `fp`.
fn near_polygon_boundary(p: Point3, fp: &impl Outline, band: f64) -> bool {
    let near = |poly: &[Point3]| {
        (0..poly.len())
            .any(|i| point_segment_distance(p, poly[i], poly[(i + 1) % poly.len()]) <= band)
    };
    near(fp.outer()) || fp.holes().iter().any(|h| near(h))
}

/// Distance from `p` to the segment `a`–`b`.
fn point_segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

// ─────────────────────────────────────────────────────────────────── assembly

/// Welds the kept faces into a watertight Object, then splits a disconnected
/// result into one shell per connected component.
/// Builds the result object from the kept faces, and maps each result face
/// to the operand face it came from (both builders insert faces in input
/// order, and a fresh slotmap iterates in insertion order).
fn assemble(
    mut faces: Vec<OrientedFace>,
    planarity_tol: f64,
    operands: [&Object; 2],
) -> Result<(Object, BTreeMap<FaceId, Provenance>), BooleanError> {
    split_edges_at_vertices(&mut faces);
    let mut interner = Interner::new();
    let has_holes = faces.iter().any(|f| !f.holes.is_empty());

    let mut obj = if !has_holes {
        let polys: Vec<Vec<usize>> = faces
            .iter()
            .map(|f| {
                f.outer
                    .iter()
                    .zip(&f.outer_ids)
                    .map(|(&p, &id)| interner.intern_id(p, id))
                    .collect()
            })
            .collect();
        let positions = std::mem::take(&mut interner.positions);
        // Result faces inherit their source face's material, UV frame, and
        // analytic surface.
        let materials: Vec<FaceMaterial> = faces.iter().map(|f| f.material).collect();
        let uv_frames: Vec<Option<UvFrame>> = faces.iter().map(|f| f.uv_frame).collect();
        let surfaces: Vec<Option<SurfaceRef>> = faces.iter().map(|f| f.surface).collect();
        let attrs: Vec<AttrDict> = faces.iter().map(|f| f.attrs.clone()).collect();
        Object::from_polygons_with_attrs_tol(
            &positions,
            &polys,
            &materials,
            &uv_frames,
            &surfaces,
            &attrs,
            planarity_tol,
        )
        .map_err(|e| build_error(e, &faces, operands))?
    } else {
        let with_holes: Vec<_> = faces
            .iter()
            .map(|f| {
                let outer = f
                    .outer
                    .iter()
                    .zip(&f.outer_ids)
                    .map(|(&p, &id)| interner.intern_id(p, id))
                    .collect();
                let holes = f
                    .holes
                    .iter()
                    .zip(&f.hole_ids)
                    .map(|(h, ids)| {
                        h.iter()
                            .zip(ids)
                            .map(|(&p, &id)| interner.intern_id(p, id))
                            .collect()
                    })
                    .collect();
                (
                    outer,
                    holes,
                    f.plane,
                    f.material,
                    f.uv_frame,
                    f.surface,
                    f.attrs.clone(),
                )
            })
            .collect();
        Object::from_faces_with_holes(&interner.positions, &with_holes)
    };
    let backs: Vec<FaceMaterial> = faces.iter().map(|f| f.back_material).collect();
    obj.set_built_back_materials(&backs);
    // Sub-faces inherit their source face's blend tag, like its claim.
    let blends: Vec<Option<BlendTag>> = faces.iter().map(|f| f.blend).collect();
    obj.set_built_blends(&blends);
    // And the records of the blends that survive: B's behind A's, as its
    // tags are.
    let [a, b_in_a] = operands;
    let blend_offset = a.max_blend_id().map_or(0, |m| m.0);
    let seq_offset = a
        .blend_records()
        .values()
        .map(|r| r.seq.saturating_add(1))
        .max()
        .unwrap_or(0);
    let mut records = a.blend_records().clone();
    for (id, rec) in b_in_a.blend_records() {
        records.insert(
            crate::topo::BlendId(id.0.saturating_add(blend_offset)),
            rec.with_ids_offset(blend_offset, seq_offset),
        );
    }
    obj.set_blend_records(records);

    // The result is no more planar than its loosest operand: imported
    // geometry is flat only to `IMPORT_PLANE_DIST`, and boolean faces
    // copied through from such an operand carry those same vertices. The
    // built object defaults to strict `PLANE_DIST`, so without this the
    // validate() backstop below would reject a perfectly good boolean over
    // imported geometry (audit q-kernel-correctness). Never TIGHTENS the
    // default — only loosens toward whatever the operands already tolerated.
    obj.planarity_tol = obj.planarity_tol.max(planarity_tol);

    if obj.watertight() != WatertightState::Watertight {
        return Err(BooleanError::DegenerateContact);
    }
    crate::patch::resplit_shells(&mut obj);
    // Always-on backstop (generalized in): release WASM compiles out
    // check_invariants(), so re-validate the result and refuse a boolean that
    // produced invalid topology rather than commit it. A face that is not
    // flat, or whose plane faces against its winding, is an arrangement
    // that went wrong; one too small to keep a plane
    // (under `tol::MIN_FACE_WIDTH` wide) is a result too thin; anything else
    // is DegenerateContact, the "no manifold result" channel, matching the
    // watertight check above. Validated before the debug invariant check, so
    // a tidied result that fails is a typed answer the tidy falls back from
    // (`tidy`), in every build.
    obj.validate().map_err(validate_error)?;
    obj.check_invariants();
    if obj.faces().len() != faces.len() {
        return Err(BooleanError::DegenerateContact);
    }
    let prov = obj
        .faces()
        .keys()
        .zip(faces.iter().map(|f| f.source))
        .collect();
    Ok((obj, prov))
}

/// The boolean's refusal for a result that fails validation (see
/// [`assemble`]).
fn validate_error(e: TopologyError) -> BooleanError {
    match e {
        TopologyError::NonPlanarFace { .. }
        | TopologyError::FaceGeometryNotPlanar { .. }
        | TopologyError::FacePlaneInvalid { .. } => BooleanError::ArrangementFailed,
        TopologyError::DegenerateFace { .. } | TopologyError::FaceTooSmallToReopen { .. } => {
            BooleanError::ResultTooThin
        }
        _ => BooleanError::DegenerateContact,
    }
}

/// Marks soft every edge between two faces cut from the triangles of one
/// split imported face ([`refine_operand`]), or of one face the tidy split
/// because a weld bent it ([`tidy`]): those diagonals were drawn by the
/// boolean, not by the person, so it adds no visible line beyond the cut
/// itself. `tags` names each such face of `obj` with its split source and
/// tidy group ([`split_tags`]). Runs after the marks are re-derived, which
/// would otherwise clear them.
fn soften_split_diagonals(obj: &mut Object, tags: &[(FaceId, SplitTag)]) {
    if tags.iter().all(|(_, t)| t.0.is_none() && t.1.is_none()) {
        return;
    }
    let tag: SecondaryMap<FaceId, SplitTag> = tags.iter().copied().collect();
    let soft: Vec<EdgeId> = obj
        .edges
        .iter()
        .filter_map(|(id, e)| {
            let twin = e.twin_half_edge?;
            let fa = obj.loops[obj.half_edges[e.half_edge].loop_id].face;
            let fb = obj.loops[obj.half_edges[twin].loop_id].face;
            let (ta, tb) = (tag.get(fa).copied()?, tag.get(fb).copied()?);
            let split = ta.0.is_some() && ta.0 == tb.0;
            let tidied = ta.1.is_some() && ta.1 == tb.1;
            (split || tidied).then_some(id)
        })
        .collect();
    for id in soft {
        let edge = &mut obj.edges[id];
        // A crease is the person's keep-sharp mark; it wins.
        if !edge.crease {
            edge.soft = true;
        }
    }
}

/// A face's split source and tidy group ([`soften_split_diagonals`]).
type SplitTag = (Option<(Side, FaceId)>, Option<u32>);

/// Each kept face's [`SplitTag`], in order.
fn split_tags(faces: &[OrientedFace]) -> Vec<SplitTag> {
    faces.iter().map(|f| (f.split_from, f.tidy_group)).collect()
}

/// The boolean's refusal for a result the builder rejected, naming the
/// cause where the builder names a face (KB-25).
fn build_error(
    e: crate::error::TopologyError,
    faces: &[OrientedFace],
    operands: [&Object; 2],
) -> BooleanError {
    let at = |face: usize| faces.get(face).map_or(Point3::ORIGIN, |f| f.outer[0]);
    // A degenerate result face is the operand's fault only when the
    // operand face it came from is itself degenerate; a sliver cut
    // from a sound face is a piece too thin to keep (one the tidy could
    // not merge away, or a grazing contact between the operands: two
    // equal pipes whose walls graze along a facet row).
    let source_degenerate = |face: usize| {
        faces.get(face).is_some_and(|f| {
            let obj = operands[usize::from(f.source.side)];
            obj.faces().get(f.source.face).is_some_and(|sf| {
                let ring: Vec<Point3> = obj.loop_positions(sf.outer_loop).collect();
                crate::health::ring_is_degenerate(&ring)
            })
        })
    };
    match e {
        crate::error::TopologyError::NonPlanarFace { face } => {
            BooleanError::NonPlanarInput { at: at(face) }
        }
        crate::error::TopologyError::DegenerateFace { face } if source_degenerate(face) => {
            BooleanError::DegenerateInput { at: at(face) }
        }
        // A piece the file format could not reopen, or one with no plane:
        // too thin to keep.
        crate::error::TopologyError::DegenerateFace { .. }
        | crate::error::TopologyError::FaceTooSmallToReopen { .. } => BooleanError::ResultTooThin,
        _ => BooleanError::DegenerateContact,
    }
}

/// [`assemble`] onto a kept operand (`operands[base]`): its faces in
/// `removed` (the ones the boolean arranged) give way to the kept sub-faces
/// `faces`, welded to its untouched faces through the vertices they share
/// (an operand corner stays that vertex) and spliced in place
/// ([`Object::splice_closed`]). Faces, planes and refusals are as the full
/// build makes them; returns the result, its new faces (parallel to
/// `faces`) and their provenance.
fn assemble_onto(
    base: usize,
    removed: &[FaceId],
    mut faces: Vec<OrientedFace>,
    planarity_tol: f64,
    operands: [&Object; 2],
    blend_offset: u32,
) -> Result<(Object, Vec<FaceId>, Vec<Provenance>), BooleanError> {
    use crate::patch::{PVert, PatchFace};
    let base_obj = operands[base];
    split_edges_at_vertices(&mut faces);
    let mut interner = Interner::new();
    let rings: Vec<(Vec<usize>, Vec<Vec<usize>>)> = faces
        .iter()
        .map(|f| {
            let outer = f
                .outer
                .iter()
                .zip(&f.outer_ids)
                .map(|(&p, &id)| interner.intern_id(p, id))
                .collect();
            let holes = f
                .holes
                .iter()
                .zip(&f.hole_ids)
                .map(|(h, ids)| {
                    h.iter()
                        .zip(ids)
                        .map(|(&p, &id)| interner.intern_id(p, id))
                        .collect()
                })
                .collect();
            (outer, holes)
        })
        .collect();
    let verts: Vec<PVert> = interner
        .positions
        .iter()
        .zip(&interner.sides)
        .map(|(&p, sides)| match sides[base] {
            Some(v) => PVert::Existing(v),
            None => PVert::New(p),
        })
        .collect();
    let pos = |i: usize| match verts[i] {
        PVert::Existing(v) => base_obj.vertices()[v].position,
        PVert::New(p) => p,
    };

    // Planes as the full build fits them: Newell from the outer ring, the
    // arrangement's plane once any face of the result has a hole.
    let mut removing: BTreeMap<FaceId, ()> = BTreeMap::new();
    for &f in removed {
        removing.insert(f, ());
    }
    let has_holes = faces.iter().any(|f| !f.holes.is_empty())
        || base_obj
            .faces()
            .iter()
            .any(|(f, face)| !face.inner_loops.is_empty() && !removing.contains_key(&f));
    let plane_tol = planarity_tol.max(tol::PLANE_DIST);
    let imported = plane_tol > tol::PLANE_DIST;
    let mut add: Vec<PatchFace> = Vec::with_capacity(faces.len());
    for (fi, (f, (outer, holes))) in faces.iter().zip(rings).enumerate() {
        let plane = if has_holes {
            f.plane
        } else {
            let pts: Vec<Point3> = outer.iter().map(|&i| pos(i)).collect();
            let degenerate = || crate::error::TopologyError::DegenerateFace { face: fi };
            if pts.len() < 3 {
                return Err(build_error(degenerate(), &faces, operands));
            }
            let plane = Plane::from_polygon(&pts)
                .or_else(|e| {
                    if imported {
                        crate::serialize::outer_ring_plane(&pts, true)
                    } else {
                        Err(e)
                    }
                })
                .map_err(|_| build_error(degenerate(), &faces, operands))?;
            if crate::serialize::outer_ring_plane(&pts, imported).is_err() {
                return Err(build_error(degenerate(), &faces, operands));
            }
            plane
        };
        add.push(PatchFace {
            outer,
            holes,
            attrs: crate::topo::FaceAttrs {
                material: f.material,
                back_material: f.back_material,
                uv_frame: f.uv_frame,
                surface: f.surface,
                blend: f.blend,
                // Interned into the result's own table after the splice.
                attrs: crate::topo::AttrSlot::NONE,
            },
            plane: Some(plane),
        });
    }
    let (mut obj, new_faces) = base_obj
        .splice_closed(removed, &verts, &add, planarity_tol)
        .map_err(|e| match e {
            // The full build leaves a hole-mode face's planarity to the
            // final validation, which refuses as a contact.
            crate::error::TopologyError::NonPlanarFace { .. } if has_holes => {
                BooleanError::DegenerateContact
            }
            e => build_error(e, &faces, operands),
        })?;

    // Each new face takes its source face's dictionary, as the full build
    // gives it; and, as there, no edge of the result keeps one (the kept
    // operand's untouched edges included), so both paths agree.
    for (&nf, f) in new_faces.iter().zip(&faces) {
        if !f.attrs.is_empty() {
            obj.set_face_attrs(nf, f.attrs.clone());
        }
    }
    let dict_edges: Vec<EdgeId> = obj
        .edges()
        .iter()
        .filter(|(_, e)| !e.attrs.is_none())
        .map(|(e, _)| e)
        .collect();
    for e in dict_edges {
        obj.set_edge_attrs(e, AttrDict::new());
    }

    // The blend records of both operands, B's behind A's, as `assemble`
    // keeps them; a kept B's own faces move their tags past A's ids too.
    let [a, b_in_a] = operands;
    let seq_offset = a
        .blend_records()
        .values()
        .map(|r| r.seq.saturating_add(1))
        .max()
        .unwrap_or(0);
    let mut records = a.blend_records().clone();
    for (id, rec) in b_in_a.blend_records() {
        records.insert(
            crate::topo::BlendId(id.0.saturating_add(blend_offset)),
            rec.with_ids_offset(blend_offset, seq_offset),
        );
    }
    obj.set_blend_records(records);
    if base == 1 && blend_offset > 0 {
        let fresh: BTreeMap<FaceId, ()> = new_faces.iter().map(|&f| (f, ())).collect();
        let tagged: Vec<FaceId> = obj
            .faces()
            .iter()
            .filter(|(f, face)| face.blend.is_some() && !fresh.contains_key(f))
            .map(|(f, _)| f)
            .collect();
        for f in tagged {
            if let Some(t) = obj.faces[f].blend.as_mut() {
                t.id = crate::topo::BlendId(t.id.0 + blend_offset);
            }
        }
    }
    let prov = faces.iter().map(|f| f.source).collect();
    Ok((obj, new_faces, prov))
}

/// Splits every kept-face edge at any kept-face vertex lying in its interior,
/// so the weld sees matching half-edges on both sides of every seam.
///
/// Seams are imprinted identically on both faces they cut, and where a seam
/// meets a face boundary the intersection curve normally continues onto the
/// neighbouring face through the same point, so the neighbour is split there
/// too. That continuation can degenerate to a single point: a prism whose
/// bottom rests exactly on a step's top face and reaches past the step's far
/// edge cuts that top face along the prism's side, but the prism's side meets
/// the step's far WALL only at the edge's endpoint, so no seam reaches the
/// wall and its top edge stays whole while the top face's copy of that edge
/// is split — a T-junction the weld cannot pair. Inserting the vertex into
/// every edge it lies on restores the by-construction consistency; the added
/// vertices are collinear with the edge they split, so no face leaves its
/// plane, and a genuine non-manifold contact still fails the watertight check
/// below because the extra vertices never invent a missing twin face.
///
/// An operand's own vertex never splits that operand's own edge (both ends
/// its vertices): the operand was valid as it was, and a vertex of its own
/// within `POINT_MERGE` of one of its edges is part of its shape (a mended
/// scan's sliver), not a T-junction the boolean made.
fn split_edges_at_vertices(faces: &mut [OrientedFace]) {
    let mut interner = Interner::new();
    let mut cand_ids: Vec<[Option<VertexId>; 2]> = Vec::new();
    for f in faces.iter() {
        let ids = f.outer_ids.iter().chain(f.hole_ids.iter().flatten());
        for (&p, &id) in f.outer.iter().chain(f.holes.iter().flatten()).zip(ids) {
            let i = interner.intern(p);
            if i == cand_ids.len() {
                cand_ids.push([None, None]);
            }
            if let Some((side, v)) = id
                && cand_ids[i][usize::from(side)].is_none()
            {
                cand_ids[i][usize::from(side)] = Some(v);
            }
        }
    }
    let vertices = interner.positions;
    // A coarse grid over the kept vertices (cell = the mean edge length):
    // each edge visits only the cells its padded bounding box covers, and
    // candidates are visited in ascending index — the scan order — so the
    // splits are bit-identical to scanning every vertex (KB-25).
    let (mut total, mut count) = (0.0f64, 0usize);
    for f in faces.iter() {
        for ring in std::iter::once(&f.outer).chain(f.holes.iter()) {
            for i in 0..ring.len() {
                total += (ring[(i + 1) % ring.len()] - ring[i]).length();
                count += 1;
            }
        }
    }
    let cell = (total / count.max(1) as f64).max(4.0 * tol::POINT_MERGE);
    let key = |x: f64| (x / cell).floor().clamp(-4.0e18, 4.0e18) as i64;
    let mut grid: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
    for (i, v) in vertices.iter().enumerate() {
        grid.entry([key(v.x), key(v.y), key(v.z)])
            .or_default()
            .push(i);
    }
    let split = |ring: &mut Vec<Point3>, ids: &mut Vec<Ident>| {
        let n = ring.len();
        let mut out: Vec<Point3> = Vec::with_capacity(n);
        let mut out_ids: Vec<Ident> = Vec::with_capacity(n);
        for i in 0..n {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            out.push(p);
            out_ids.push(ids[i]);
            // Both ends one operand's vertices: its own vertices stay off.
            let own = match (ids[i], ids[(i + 1) % n]) {
                (Some((s, _)), Some((t, _))) if s == t => Some(usize::from(s)),
                _ => None,
            };
            let len2 = (q - p).length_squared();
            if len2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
                continue;
            }
            // Cheap box reject first: the scan visits every kept vertex per
            // edge (the same shape as the interning below), and almost none
            // are anywhere near a given edge.
            let (lo, hi) = (
                Point3::new(
                    p.x.min(q.x) - tol::POINT_MERGE,
                    p.y.min(q.y) - tol::POINT_MERGE,
                    p.z.min(q.z) - tol::POINT_MERGE,
                ),
                Point3::new(
                    p.x.max(q.x) + tol::POINT_MERGE,
                    p.y.max(q.y) + tol::POINT_MERGE,
                    p.z.max(q.z) + tol::POINT_MERGE,
                ),
            );
            let mut candidates: Vec<usize> = Vec::new();
            let (k0, k1) = (
                [key(lo.x), key(lo.y), key(lo.z)],
                [key(hi.x), key(hi.y), key(hi.z)],
            );
            let span = (0..3).fold(1u128, |n, i| n.saturating_mul((k1[i] - k0[i]) as u128 + 1));
            if span > grid.len() as u128 {
                // A long edge over a fine grid (a cutter across a scan's
                // facets) would visit more empty cells than there are
                // occupied ones: walk the occupied cells instead.
                for (k, list) in &grid {
                    if (0..3).all(|i| k[i] >= k0[i] && k[i] <= k1[i]) {
                        candidates.extend_from_slice(list);
                    }
                }
            } else {
                for gx in k0[0]..=k1[0] {
                    for gy in k0[1]..=k1[1] {
                        for gz in k0[2]..=k1[2] {
                            if let Some(list) = grid.get(&[gx, gy, gz]) {
                                candidates.extend_from_slice(list);
                            }
                        }
                    }
                }
            }
            candidates.sort_unstable();
            let mut inside: Vec<(f64, Point3, Ident)> = candidates
                .iter()
                .filter(|&&i| own.is_none_or(|s| cand_ids[i][s].is_none()))
                .map(|&i| (&vertices[i], i))
                .filter(|&(&r, _)| {
                    r.x >= lo.x
                        && r.x <= hi.x
                        && r.y >= lo.y
                        && r.y <= hi.y
                        && r.z >= lo.z
                        && r.z <= hi.z
                        && !r.approx_eq(p, tol::POINT_MERGE)
                        && !r.approx_eq(q, tol::POINT_MERGE)
                        && point_segment_distance(r, p, q) <= tol::POINT_MERGE
                })
                .map(|(&r, i)| {
                    let id = cand_ids[i][0]
                        .map(|v| (0u8, v))
                        .or(cand_ids[i][1].map(|v| (1u8, v)));
                    ((r - p).dot(q - p) / len2, r, id)
                })
                .collect();
            inside.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, r, id) in inside {
                out.push(r);
                out_ids.push(id);
            }
        }
        *ring = out;
        *ids = out_ids;
    };
    for f in faces.iter_mut() {
        split(&mut f.outer, &mut f.outer_ids);
        for (h, ids) in f.holes.iter_mut().zip(f.hole_ids.iter_mut()) {
            split(h, ids);
        }
    }
}

/// Interns points, merging within [`tol::POINT_MERGE`]. Welding is what
/// pairs seam half-edges into twins, so the result is watertight.
///
/// A uniform grid of `4·POINT_MERGE` cells indexes the points: a point
/// within `POINT_MERGE` of `p` lies in `p`'s cell or one of its 26
/// neighbors, and the SMALLEST matching index is returned — exactly the
/// first match of a linear scan in insertion order, so results are
/// bit-identical to the scan while the cost stops being quadratic (KB-25).
///
/// [`Interner::intern_id`] also keeps operand vertices apart: a corner that
/// is an operand vertex ([`Ident`]) joins the entry that vertex already has,
/// and never one holding a different vertex of the same operand.
struct Interner {
    positions: Vec<Point3>,
    cells: BTreeMap<[i64; 3], Vec<usize>>,
    ids: BTreeMap<(u8, VertexId), usize>,
    /// Per entry, the vertex of each operand it is (if any).
    sides: Vec<[Option<VertexId>; 2]>,
}

impl Interner {
    fn new() -> Interner {
        Interner {
            positions: Vec::new(),
            cells: BTreeMap::new(),
            ids: BTreeMap::new(),
            sides: Vec::new(),
        }
    }

    /// [`Interner::intern`] for a corner that may be an operand vertex.
    fn intern_id(&mut self, p: Point3, id: Ident) -> usize {
        if let Some(key) = id
            && let Some(&i) = self.ids.get(&key)
        {
            return i;
        }
        let i = self.intern_where(p, |sides| {
            id.is_none_or(|(s, v)| sides[usize::from(s)].is_none_or(|w| w == v))
        });
        if let Some((s, v)) = id {
            self.sides[i][usize::from(s)] = Some(v);
            self.ids.insert((s, v), i);
        }
        i
    }

    fn cell(p: Point3) -> [i64; 3] {
        let c = |x: f64| {
            (x / (4.0 * tol::POINT_MERGE))
                .floor()
                .clamp(-4.0e18, 4.0e18) as i64
        };
        [c(p.x), c(p.y), c(p.z)]
    }

    fn intern(&mut self, p: Point3) -> usize {
        self.intern_where(p, |_| true)
    }

    /// The smallest entry within `POINT_MERGE` of `p` that `fits`, else a
    /// new one.
    fn intern_where(&mut self, p: Point3, fits: impl Fn(&[Option<VertexId>; 2]) -> bool) -> usize {
        let [cx, cy, cz] = Interner::cell(p);
        let mut best: Option<usize> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = self.cells.get(&[cx + dx, cy + dy, cz + dz]) {
                        for &i in list {
                            if best.is_none_or(|b| i < b)
                                && p.approx_eq(self.positions[i], tol::POINT_MERGE)
                                && fits(&self.sides[i])
                            {
                                best = Some(i);
                            }
                        }
                    }
                }
            }
        }
        if let Some(i) = best {
            return i;
        }
        let i = self.positions.len();
        self.positions.push(p);
        self.sides.push([None, None]);
        self.cells.entry([cx, cy, cz]).or_default().push(i);
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_square() -> Region {
        Region {
            outer: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            holes: Vec::new(),
        }
    }

    /// A tangent chord through the square's middle, x = 0.5, carries every
    /// edge's half-way point from the bottom and top edges and the centre:
    /// the point found is off it.
    #[test]
    fn an_interior_point_steps_off_a_line_the_partner_only_touches() {
        let up = Vec3::new(0.0, 0.0, 1.0);
        let chord = (Point3::new(0.5, 0.0, 0.0), Point3::new(0.5, 1.0, 0.0));
        let c = interior_point(&unit_square(), up, &[chord]).expect("a point");
        assert!(point_segment_distance(c, chord.0, chord.1) > tol::POINT_MERGE);
        assert!(c.x > 0.0 && c.x < 1.0 && c.y > 0.0 && c.y < 1.0);
    }

    /// With the two mid-lines both to avoid, every candidate (each edge's
    /// ray at every depth, and the centre) lies on one: no decidable point,
    /// `None`, which the classifier turns into a typed refusal.
    #[test]
    fn no_interior_point_off_every_avoided_line_is_none() {
        let up = Vec3::new(0.0, 0.0, 1.0);
        let lines = [
            (Point3::new(0.5, 0.0, 0.0), Point3::new(0.5, 1.0, 0.0)),
            (Point3::new(0.0, 0.5, 0.0), Point3::new(1.0, 0.5, 0.0)),
        ];
        assert_eq!(interior_point(&unit_square(), up, &lines), None);
        // With nothing to avoid it is the bottom edge's half-way point, as
        // before.
        assert_eq!(
            interior_point(&unit_square(), up, &[]),
            Some(Point3::new(0.5, 0.5, 0.0))
        );
    }

    fn cube(min: Point3, max: Point3) -> Object {
        let (a, b) = (min, max);
        Object::from_polygons(
            &[
                Point3::new(a.x, a.y, a.z),
                Point3::new(b.x, a.y, a.z),
                Point3::new(b.x, b.y, a.z),
                Point3::new(a.x, b.y, a.z),
                Point3::new(a.x, a.y, b.z),
                Point3::new(b.x, a.y, b.z),
                Point3::new(b.x, b.y, b.z),
                Point3::new(a.x, b.y, b.z),
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

    fn unit_cube() -> Object {
        cube(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0))
    }

    /// Divergence-theorem signed volume, fan-triangulating every loop of every
    /// face. Inner (hole) loops are wound opposite to the outer, so summing
    /// their fans subtracts the hole correctly — `to_polygons` drops holes, so
    /// it cannot be used here (coplanar results can be genus > 0).
    fn volume(obj: &Object) -> f64 {
        let mut v6 = 0.0;
        for f in obj.faces().values() {
            for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
                let p: Vec<Vec3> = obj.loop_positions(lid).map(|pt| pt.to_vec()).collect();
                for i in 1..p.len().saturating_sub(1) {
                    v6 += p[0].dot(p[i].cross(p[i + 1]));
                }
            }
        }
        v6 / 6.0
    }

    const VOL_TOL: f64 = 1e-9;

    fn overlapping(shift: Vec3) -> (Object, Object, Transform) {
        (unit_cube(), unit_cube(), Transform::translation(shift))
    }

    /// A boolean over an operand carrying the loose import planarity
    /// tolerance must propagate that tolerance to its result — otherwise the
    /// result defaults to strict `PLANE_DIST` and its own validate() backstop
    /// spuriously refuses import-flat faces copied through from the operand
    /// (audit q-kernel-correctness). The result tolerance is the loosest of
    /// the two operands, never tighter than the default.
    #[test]
    fn boolean_propagates_the_loosest_operand_planarity_tol() {
        let mut a = unit_cube();
        a.planarity_tol = crate::tol::IMPORT_PLANE_DIST;
        let b = unit_cube();
        let t = Transform::translation(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.planarity_tol, crate::tol::IMPORT_PLANE_DIST);

        // Two strict operands keep the strict default (never loosened for free).
        let c = unit_cube();
        let d = unit_cube();
        let r2 = Object::boolean(BooleanOp::Union, &c, &d, &t).unwrap();
        assert_eq!(r2.planarity_tol, crate::tol::PLANE_DIST);
    }

    #[test]
    fn union_of_overlapping_cubes_has_correct_volume() {
        let (a, b, t) = overlapping(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        // 2·1 − overlap(0.5³).
        assert!(
            (volume(&r) - (2.0 - 0.125)).abs() < VOL_TOL,
            "vol {}",
            volume(&r)
        );
    }

    #[test]
    fn intersect_of_overlapping_cubes_is_the_overlap_box() {
        let (a, b, t) = overlapping(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Intersect, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 0.125).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    #[test]
    fn subtract_of_overlapping_cubes_removes_the_overlap() {
        let (a, b, t) = overlapping(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Subtract, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&r) - (1.0 - 0.125)).abs() < VOL_TOL,
            "vol {}",
            volume(&r)
        );
    }

    #[test]
    fn disjoint_union_keeps_both_volumes() {
        let (a, b, t) = overlapping(Vec3::new(5.0, 0.0, 0.0));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 2.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    #[test]
    fn nested_intersect_is_the_inner_solid() {
        let a = unit_cube();
        let inner = cube(Point3::new(0.25, 0.25, 0.25), Point3::new(0.75, 0.75, 0.75));
        let r = Object::boolean(BooleanOp::Intersect, &a, &inner, &Transform::IDENTITY).unwrap();
        r.validate().unwrap();
        assert!((volume(&r) - 0.125).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    // ─────────────────────────────────────────────── coplanar contact

    /// Two boxes meeting at a single shared face (opposite-facing coincident):
    /// the interface vanishes and they fuse into one taller solid.
    #[test]
    fn stacked_union_is_one_tall_box() {
        let a = unit_cube();
        let b = unit_cube();
        let stacked = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &stacked).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 2.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    /// Stacked boxes share no volume, so their intersection is empty.
    #[test]
    fn stacked_intersect_is_empty() {
        let a = unit_cube();
        let b = unit_cube();
        let stacked = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(
            Object::boolean(BooleanOp::Intersect, &a, &b, &stacked).unwrap_err(),
            BooleanError::EmptyResult
        );
    }

    /// Subtracting a box that only touches `a`'s top face removes nothing,
    /// and a subtract that removes nothing is refused rather than returned
    /// as an unchanged `a` (this test asserted the unchanged `a` before the
    /// refusal existed).
    #[test]
    fn stacked_subtract_leaves_a_unchanged() {
        let a = unit_cube();
        let b = unit_cube();
        let stacked = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(
            Object::boolean(BooleanOp::Subtract, &a, &b, &stacked).unwrap_err(),
            BooleanError::NothingRemoved
        );
    }

    /// Two boxes sitting apart: the subtract has nothing to remove.
    #[test]
    fn disjoint_subtract_is_refused() {
        let (a, b, t) = overlapping(Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(
            Object::boolean(BooleanOp::Subtract, &a, &b, &t).unwrap_err(),
            BooleanError::NothingRemoved
        );
        // Union of the same pair still answers honestly: two shells, both
        // volumes kept (see `disjoint_union_keeps_both_volumes`).
        let u = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        assert!((volume(&u) - 2.0).abs() < VOL_TOL);
    }

    /// A prism with an L-shaped cross-section in the xz plane, extruded along
    /// y from `y0` to `y1`. `xz` is the cross-section wound counter-clockwise
    /// with x to the right and z up.
    fn prism_along_y(xz: &[(f64, f64)], y0: f64, y1: f64) -> Object {
        let n = xz.len();
        let mut pts = Vec::with_capacity(2 * n);
        for &(x, z) in xz {
            pts.push(Point3::new(x, y0, z));
        }
        for &(x, z) in xz {
            pts.push(Point3::new(x, y1, z));
        }
        // x × z = −y, so the cross-section's CCW winding faces −y: that is
        // the outward normal of the y0 cap; the y1 cap is wound the other way.
        let mut faces = vec![
            (0..n).collect::<Vec<_>>(),
            (0..n).rev().map(|i| i + n).collect(),
        ];
        for i in 0..n {
            let j = (i + 1) % n;
            // Each side quad traverses the y0 cap's edge in reverse, like the
            // cube helper's side faces do against its bottom face.
            faces.push(vec![j, i, i + n, j + n]);
        }
        let p = Object::from_polygons(&pts, &faces).unwrap();
        p.validate().unwrap();
        assert_eq!(p.watertight(), WatertightState::Watertight);
        assert!(volume(&p) > 0.0, "prism is wound inside out");
        p
    }

    /// A 3/4 prism hugging the cube's (x=1, z=1) edge: its missing quadrant
    /// is exactly the cube's corner, so it touches the cube along two face
    /// strips (x=1 for z in [0.5,1], z=1 for x in [0.5,1]) and shares no
    /// volume. Partial coplanar face contact is RESOLVED by the arrangement
    /// (not refused as DegenerateContact), so before the volume check this
    /// subtract returned the cube unchanged.
    #[test]
    fn subtract_touching_along_a_face_strip_is_refused() {
        let a = unit_cube();
        let hug = prism_along_y(
            &[
                (1.0, 0.5),
                (1.5, 0.5),
                (1.5, 1.5),
                (0.5, 1.5),
                (0.5, 1.0),
                (1.0, 1.0),
            ],
            0.0,
            1.0,
        );
        assert!((volume(&hug) - 0.75).abs() < VOL_TOL);
        assert_eq!(
            Object::boolean(BooleanOp::Subtract, &a, &hug, &Transform::IDENTITY).unwrap_err(),
            BooleanError::NothingRemoved
        );
        // The union of the same pair fuses along the shared strips: one
        // solid, no double-counted volume.
        let u = Object::boolean(BooleanOp::Union, &a, &hug, &Transform::IDENTITY).unwrap();
        u.validate().unwrap();
        assert_eq!(u.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&u) - 1.75).abs() < VOL_TOL,
            "union vol {}",
            volume(&u)
        );
    }

    /// A cutter wholly inside `a` removes volume without touching `a`'s
    /// skin: the result is `a` with a cavity (an inner, inside-out shell).
    /// The refusal is about volume, not about whether faces intersect, so
    /// this must stay Ok.
    /// The refusal is about whether `b`'s skin enters `a`, not about how
    /// much volume goes: a 0.1 mm × 0.1 mm × 10 µm notch (1e-13 m³, a
    /// 1e-13 fraction of the cube) is a real cut and stays Ok.
    #[test]
    fn subtract_of_a_tiny_notch_is_not_refused() {
        let a = unit_cube();
        let notch = cube(
            Point3::new(0.4, 0.4, 1.0 - 1e-5),
            Point3::new(0.4 + 1e-4, 0.4 + 1e-4, 1.0 + 1e-4),
        );
        let r = Object::boolean(BooleanOp::Subtract, &a, &notch, &Transform::IDENTITY).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!(r.faces().len() > a.faces().len(), "the notch has walls");
        assert!(
            (volume(&r) - (1.0 - 1e-13)).abs() < 1e-14,
            "vol {}",
            volume(&r)
        );
    }

    /// A frame map that places the second operand where its faces cannot
    /// stay flat (here a turned cube moved about 5,000 km out) is refused as
    /// the face-off-its-plane refusal the transform gives, not as a
    /// placement scaled down to nothing.
    #[test]
    fn a_frame_map_leaving_a_face_off_its_plane_is_refused_as_such() {
        let edge = 0.7192083385725339;
        let b = cube(Point3::ORIGIN, Point3::new(edge, edge, edge));
        let t = Transform::rotation(
            Vec3::new(0.190371552809935, 0.1638626972541155, -0.04686545674251775),
            0.42065614440870847,
        )
        .unwrap()
        .then(&Transform::translation(Vec3::new(
            4590968.5052802395,
            2150093.0725587867,
            0.6769586979531994,
        )));
        assert_eq!(
            b.clone().apply_transform(&t),
            Err(crate::transform::TransformError::FaceNotFlat),
            "the map leaves a face of the operand off its plane"
        );
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            assert_eq!(
                Object::boolean(op, &unit_cube(), &b, &t).unwrap_err(),
                BooleanError::FaceNotFlat,
                "{op:?}"
            );
        }
    }

    /// A 20 µm cavity (8e-15 m³) wholly inside a meter cube is a real cut too.
    #[test]
    fn subtract_of_a_tiny_enclosed_cavity_is_not_refused() {
        let a = unit_cube();
        let pore = cube(
            Point3::new(0.5, 0.5, 0.5),
            Point3::new(0.5 + 2e-5, 0.5 + 2e-5, 0.5 + 2e-5),
        );
        let r = Object::boolean(BooleanOp::Subtract, &a, &pore, &Transform::IDENTITY).unwrap();
        r.validate().unwrap();
        assert_eq!(r.shells().len(), 2, "outer skin plus the cavity wall");
    }

    /// The test does not depend on where the operands sit: two 1 cm cubes a
    /// meter apart refuse the same way at the origin and 10 km or 100 km
    /// from it, where a volume comparison about the origin would drown in
    /// rounding.
    #[test]
    fn disjoint_subtract_far_from_the_origin_is_refused() {
        for offset in [0.0, 1e3, 1e4, 3e4, 1e5] {
            let o = Point3::new(offset, offset, offset);
            let a = cube(o, Point3::new(o.x + 0.01, o.y + 0.01, o.z + 0.01));
            let b = cube(
                Point3::new(o.x + 1.0, o.y, o.z),
                Point3::new(o.x + 1.01, o.y + 0.01, o.z + 0.01),
            );
            assert_eq!(
                Object::boolean(BooleanOp::Subtract, &a, &b, &Transform::IDENTITY).unwrap_err(),
                BooleanError::NothingRemoved,
                "offset {offset}"
            );
            // And the face-strip contact, which welds, is refused there too.
            let hug = cube(
                Point3::new(o.x + 0.01, o.y, o.z + 0.005),
                Point3::new(o.x + 0.02, o.y + 0.01, o.z + 0.015),
            );
            assert_eq!(
                Object::boolean(BooleanOp::Subtract, &a, &hug, &Transform::IDENTITY).unwrap_err(),
                BooleanError::NothingRemoved,
                "hugging at offset {offset}"
            );
        }
    }

    /// An inside-out target (every loop reversed; still a closed shell, so
    /// it passes the watertight gate) is refused like a right-side-out one:
    /// the test reads the arrangement, not a signed volume whose sign an
    /// inverted shell would flip.
    #[test]
    fn disjoint_subtract_from_an_inside_out_target_is_refused() {
        let (a, b) = (Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        let inside_out = Object::from_polygons(
            &[
                Point3::new(a.x, a.y, a.z),
                Point3::new(b.x, a.y, a.z),
                Point3::new(b.x, b.y, a.z),
                Point3::new(a.x, b.y, a.z),
                Point3::new(a.x, a.y, b.z),
                Point3::new(b.x, a.y, b.z),
                Point3::new(b.x, b.y, b.z),
                Point3::new(a.x, b.y, b.z),
            ],
            &[
                vec![1, 2, 3, 0],
                vec![7, 6, 5, 4],
                vec![4, 5, 1, 0],
                vec![5, 6, 2, 1],
                vec![6, 7, 3, 2],
                vec![7, 4, 0, 3],
            ],
        )
        .unwrap();
        assert_eq!(inside_out.watertight(), WatertightState::Watertight);
        assert!(volume(&inside_out) < 0.0, "wound inside out");
        let cutter = unit_cube();
        let apart = Transform::translation(Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(
            Object::boolean(BooleanOp::Subtract, &inside_out, &cutter, &apart).unwrap_err(),
            BooleanError::NothingRemoved
        );
    }

    /// A two-shell target (a union of two cubes sitting apart) losing one
    /// whole shell to a cutter that encloses it: `b`'s skin never enters a
    /// face of `a`, but every face of the swallowed shell is dropped, and a
    /// dropped face of `a` is a removal. The survivor is the other cube —
    /// also with the cutter flush against the swallowed cube's top.
    #[test]
    fn subtract_swallowing_one_shell_of_two_is_a_real_cut() {
        let near = unit_cube();
        let far = cube(Point3::new(5.0, 0.0, 0.0), Point3::new(6.0, 1.0, 1.0));
        let a = Object::boolean(BooleanOp::Union, &near, &far, &Transform::IDENTITY).unwrap();
        assert_eq!(a.shells().len(), 2);
        for top in [2.0, 1.0] {
            let swallow = cube(Point3::new(-1.0, -1.0, -1.0), Point3::new(2.0, 2.0, top));
            let r = Object::boolean(BooleanOp::Subtract, &a, &swallow, &Transform::IDENTITY)
                .unwrap_or_else(|e| panic!("cutter to z={top}: {e:?}"));
            r.validate().unwrap();
            assert_eq!(r.watertight(), WatertightState::Watertight);
            assert_eq!(r.shells().len(), 1, "one cube left (cutter to z={top})");
            assert!((volume(&r) - 1.0).abs() < VOL_TOL, "vol {}", volume(&r));
        }
    }

    /// The refusal inherits the arrangement's tolerances and nothing more: a
    /// slab taken off the top of a cube is a cut once it is thicker than
    /// `tol::PLANE_DIST` (the cutter's bottom then leaves the top face's
    /// plane), and a touch once it is thinner (the two faces are coplanar to
    /// the arrangement, so no face of `a` is dropped and no face of `b` is
    /// kept).
    #[test]
    fn subtract_thinner_than_plane_dist_is_a_touch_and_thicker_is_a_cut() {
        let a = unit_cube();
        let slab = |d: f64| cube(Point3::new(0.0, 0.0, 1.0 - d), Point3::new(1.0, 1.0, 2.0));
        for d in [tol::PLANE_DIST / 10.0, tol::PLANE_DIST / 2.0] {
            assert_eq!(
                Object::boolean(BooleanOp::Subtract, &a, &slab(d), &Transform::IDENTITY)
                    .unwrap_err(),
                BooleanError::NothingRemoved,
                "d = {d}"
            );
        }
        for d in [tol::PLANE_DIST * 10.0, 1e-6, 1e-3] {
            let r = Object::boolean(BooleanOp::Subtract, &a, &slab(d), &Transform::IDENTITY)
                .unwrap_or_else(|e| panic!("d = {d}: {e:?}"));
            r.validate().unwrap();
            assert!(
                (volume(&r) - (1.0 - d)).abs() < VOL_TOL,
                "d = {d}: vol {}",
                volume(&r)
            );
        }
    }

    #[test]
    fn subtract_of_cutter_inside_a_is_a_cavity_not_refused() {
        let a = unit_cube();
        let inner = cube(Point3::new(0.25, 0.25, 0.25), Point3::new(0.75, 0.75, 0.75));
        let r = Object::boolean(BooleanOp::Subtract, &a, &inner, &Transform::IDENTITY).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert_eq!(r.shells().len(), 2, "outer skin plus the cavity wall");
        assert!(
            (volume(&r) - (1.0 - 0.125)).abs() < VOL_TOL,
            "vol {}",
            volume(&r)
        );
    }

    /// The user's case: two boxes sitting on the ground (shared z=0 *and* z=1
    /// planes, same-facing) overlapping in x/y. Walls cross transversally; caps
    /// are coplanar. Set algebra must hold for all three ops.
    #[test]
    fn ground_boxes_obey_set_algebra() {
        // a = unit cube; b shifted (0.5, 0.5, 0): overlap = 0.5·0.5·1 = 0.25.
        let a = unit_cube();
        let b = unit_cube();
        let t = Transform::translation(Vec3::new(0.5, 0.5, 0.0));
        let vol = |op| {
            let r = Object::boolean(op, &a, &b, &t).unwrap();
            r.validate().unwrap();
            assert_eq!(r.watertight(), WatertightState::Watertight);
            volume(&r)
        };
        assert!((vol(BooleanOp::Union) - 1.75).abs() < VOL_TOL);
        assert!((vol(BooleanOp::Subtract) - 0.75).abs() < VOL_TOL);
        assert!((vol(BooleanOp::Intersect) - 0.25).abs() < VOL_TOL);
    }

    /// Two boxes sharing an exact wall (opposite-facing coincident) fuse on union.
    #[test]
    fn shared_wall_union_fuses() {
        let a = unit_cube();
        let b = unit_cube();
        let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 2.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    /// A pillar standing on the same ground as a slab and poking through it:
    /// shared z=0 plane (same-facing, *different* footprints) with transversal
    /// walls. Subtract drills a through-hole, so the result is genus-1 with an
    /// annular coplanar bottom face — exercises holed coplanar output.
    #[test]
    fn slab_with_pillar_through_it() {
        let slab = cube(Point3::ORIGIN, Point3::new(2.0, 2.0, 1.0)); // vol 4
        let pillar = cube(Point3::new(0.5, 0.5, 0.0), Point3::new(1.5, 1.5, 3.0)); // vol 3
        // Overlap (pillar ∩ slab) = 1×1×1 = 1.
        let u = Object::boolean(BooleanOp::Union, &slab, &pillar, &Transform::IDENTITY).unwrap();
        u.validate().unwrap();
        assert_eq!(u.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&u) - 6.0).abs() < VOL_TOL,
            "union vol {}",
            volume(&u)
        );

        let s = Object::boolean(BooleanOp::Subtract, &slab, &pillar, &Transform::IDENTITY).unwrap();
        s.validate().unwrap();
        assert_eq!(s.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&s) - 3.0).abs() < VOL_TOL,
            "subtract vol {}",
            volume(&s)
        );
    }

    /// Identical operands (all faces coincident, same-facing): union and
    /// intersect are the operand itself, subtract is empty.
    #[test]
    fn coincident_operands() {
        let a = unit_cube();
        let b = unit_cube();
        let id = Transform::IDENTITY;
        let u = Object::boolean(BooleanOp::Union, &a, &b, &id).unwrap();
        u.validate().unwrap();
        assert!(
            (volume(&u) - 1.0).abs() < VOL_TOL,
            "union vol {}",
            volume(&u)
        );
        let i = Object::boolean(BooleanOp::Intersect, &a, &b, &id).unwrap();
        i.validate().unwrap();
        assert!(
            (volume(&i) - 1.0).abs() < VOL_TOL,
            "intersect vol {}",
            volume(&i)
        );
        assert_eq!(
            Object::boolean(BooleanOp::Subtract, &a, &b, &id).unwrap_err(),
            BooleanError::EmptyResult
        );
    }

    /// A faceted cylinder built the way the app does — extrude a circle profile —
    /// so its side walls carry real analytic planes. Axis vertical through
    /// `(cx, cy)`, radius `r`, `n` facets from angle `phase`, height 1 (caps at
    /// z = 0 and z = 1).
    fn extruded_cylinder(cx: f64, cy: f64, r: f64, n: usize, phase: f64) -> Object {
        use crate::sketch::CurveGeom;
        let plane = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let mut s = Sketch::on_plane(plane);
        s.begin_curve_with(CurveGeom {
            center: Point3::new(cx, cy, 0.0),
            radius: r,
        })
        .unwrap();
        let pt = |k: usize| {
            let a = phase + std::f64::consts::TAU * (k as f64) / (n as f64);
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), 0.0)
        };
        for i in 0..n {
            s.add_segment(pt(i), pt(i + 1)).unwrap();
        }
        s.end_curve();
        let region = s.regions().keys().next().unwrap();
        Object::from_extrusion(&s.profile(region).unwrap(), 1.0).unwrap()
    }

    /// A cylinder whose axis coincides exactly with a cube's vertical edge, its
    /// caps flush with the cube's top and bottom faces — the user's bug report.
    /// The volumes genuinely overlap in a quarter-cylinder wedge, so every op
    /// must succeed with a watertight result, and union/intersect must not
    /// depend on operand order.
    ///
    /// Regression: the cylinder's cap is coplanar with the cube's top/bottom
    /// face, and each side wall meets that face *exactly* along the cap's rim.
    /// The rim was imprinted twice — once as the cap boundary (vertex-exact),
    /// once as the side wall's transversal seam (plane-derived) — and once the
    /// operand is nudged into generic position the near-duplicate collinear pair
    /// seeds sliver vertices that defeat the coincident-patch dedup, so the weld
    /// comes out non-manifold and the op is refused as a bogus DegenerateContact.
    /// [`segment_lies_along_any`] drops the redundant transversal copy.
    #[test]
    fn cylinder_axis_on_cube_edge_with_flush_caps() {
        let cube = unit_cube();
        // Extrude off-axis, then translate the axis onto the cube's (x=1, y=0)
        // edge (as a Move would) so the side-wall planes sit in generic position.
        let mut cyl = extruded_cylinder(0.123, 0.789, 0.25, 52, 0.31);
        cyl.apply_transform(&Transform::translation(Vec3::new(1.0 - 0.123, -0.789, 0.0)))
            .unwrap();
        let cyl_vol = volume(&cyl);
        let id = Transform::IDENTITY;

        let run = |op, a: &Object, b: &Object| {
            let r = Object::boolean(op, a, b, &id).unwrap();
            r.validate().unwrap();
            assert_eq!(
                r.watertight(),
                WatertightState::Watertight,
                "op {op:?} not watertight"
            );
            volume(&r)
        };

        // The overlap is a real quarter-wedge of the cylinder — strictly between
        // nothing and the whole cutter — not a measure-zero contact.
        let overlap = run(BooleanOp::Intersect, &cube, &cyl);
        assert!(
            overlap > 1e-3 && overlap < cyl_vol,
            "overlap {overlap} not a proper subset"
        );

        // Intersect and union are commutative; every result obeys inclusion–exclusion.
        assert!((run(BooleanOp::Intersect, &cyl, &cube) - overlap).abs() < VOL_TOL);
        let union = run(BooleanOp::Union, &cube, &cyl);
        assert!((run(BooleanOp::Union, &cyl, &cube) - union).abs() < VOL_TOL);
        assert!(
            (union - (1.0 + cyl_vol - overlap)).abs() < VOL_TOL,
            "union {union}"
        );
        assert!((run(BooleanOp::Subtract, &cube, &cyl) - (1.0 - overlap)).abs() < VOL_TOL);
        assert!((run(BooleanOp::Subtract, &cyl, &cube) - (cyl_vol - overlap)).abs() < VOL_TOL);
    }

    // ─────────────────────────────────── imported operands: refusals

    fn flat_square(z: f64) -> FacePoly {
        let outer = vec![
            Point3::new(0.0, 0.0, z),
            Point3::new(1.0, 0.0, z),
            Point3::new(1.0, 1.0, z),
            Point3::new(0.0, 1.0, z),
        ];
        FacePoly {
            plane_tol: tol::PLANE_DIST,
            flat_tol: tol::PLANE_DIST,
            plane: Plane::from_polygon(&outer).unwrap(),
            outer,
            holes: Vec::new(),
            material: None,
            back_material: None,
            uv_frame: None,
            surface: None,
            blend: None,
            attrs: AttrDict::default(),
            source: Provenance {
                side: 0,
                face: FaceId::default(),
            },
            ids: Vec::new(),
            far: false,
            split_from: None,
            pass_through: false,
            no_plane: false,
        }
    }

    /// A boundary point off the face's plane is a face that is not flat
    /// enough to imprint, named; a seam off it is an arrangement failure.
    #[test]
    fn arrange_face_names_a_face_that_is_not_flat() {
        let mut bent = flat_square(0.0);
        bent.outer[2].z = 1e-6;
        assert_eq!(
            arrange_face(&bent, &[], Side::B).err().expect("refused"),
            BooleanError::OperandFaceNotFlat {
                which: Operand::B,
                face: FaceId::default()
            }
        );
        let flat = flat_square(0.0);
        let off = (Point3::new(0.2, 0.2, 0.0), Point3::new(0.8, 0.8, 1e-6));
        assert_eq!(
            arrange_face(&flat, &[off], Side::A).err().expect("refused"),
            BooleanError::ArrangementFailed
        );
    }

    /// A near face the triangulator cannot split (here wound against its
    /// own plane) is refused by name; far from the partner it passes
    /// through whole instead, and a flat one is left alone.
    #[test]
    fn the_pre_pass_refuses_a_face_it_cannot_split() {
        let mut bent = flat_square(0.0);
        bent.outer[2].z = 1e-6;
        bent.outer.reverse();
        let near = FaceBoxes::of(&[flat_square(0.0)]);
        let err = refine_operand(vec![bent.clone_for_test()], &near, 1e-3, Side::A)
            .err()
            .expect("refused");
        assert_eq!(
            err,
            BooleanError::OperandFaceNotFlat {
                which: Operand::A,
                face: FaceId::default()
            }
        );
        let far = FaceBoxes::of(&[flat_square(5.0)]);
        let (out, count) = refine_operand(vec![bent], &far, 1e-3, Side::A).unwrap();
        assert!(out[0].pass_through && count.passed == 1 && count.split == 0);
        let (out, count) = refine_operand(vec![flat_square(0.0)], &near, 1e-3, Side::A).unwrap();
        assert!(!out[0].pass_through && count.passed == 0 && count.split == 0);
    }

    /// A bent quad near the partner becomes two flat triangles through its
    /// own corners, wound with it and tagged with it.
    #[test]
    fn the_pre_pass_splits_a_near_bent_face_into_flat_triangles() {
        let mut bent = flat_square(0.0);
        bent.outer[2].z = 1e-6;
        let corners = bent.outer.clone();
        let near = FaceBoxes::of(&[flat_square(0.0)]);
        let (out, count) = refine_operand(vec![bent], &near, 1e-3, Side::B).unwrap();
        assert_eq!((count.split, count.triangles, out.len()), (1, 2, 2));
        for t in &out {
            assert_eq!(t.outer.len(), 3);
            assert!(t.outer.iter().all(|p| corners.contains(p)));
            assert!(plane_deviation(t) <= tol::PLANE_DIST);
            assert!(t.plane.normal().z > 0.0);
            assert_eq!(t.split_from, Some((Side::B, FaceId::default())));
        }
    }

    /// A flipped piece takes the plane its points fit. Here a tilted quad
    /// about 1,800 km out, once reversed, refits from its canonical start to
    /// a plane 1.5 nm off one corner, past the 1 nm tolerance; its own
    /// ring-order fit holds every corner, so the piece carries that, as
    /// every other fit site would give it.
    #[test]
    fn a_flipped_piece_far_out_carries_a_plane_its_points_fit() {
        let ring = vec![
            Point3::new(1754047.0409302386, 629998.9682933944, 0.25963847096445924),
            Point3::new(1754047.0262600733, 629998.94911672, 0.25963847096445924),
            Point3::new(1754047.4350446332, 629998.636396333, -0.4754229932904479),
            Point3::new(1754047.4497147985, 629998.6555730074, -0.4754229932904479),
        ];
        let off = |plane: &Plane, pts: &[Point3]| {
            pts.iter()
                .map(|p| plane.signed_distance(*p).abs())
                .fold(0.0, f64::max)
        };
        let mut reversed = ring.clone();
        reversed.reverse();
        let refit = crate::serialize::outer_ring_plane(&reversed, false).unwrap();
        assert!(
            off(&refit, &reversed) > tol::PLANE_DIST,
            "the reversed ring's refit misses a corner"
        );
        let mut fp = flat_square(0.0);
        fp.plane = crate::serialize::own_ring_plane(&ring, &[], tol::PLANE_DIST).unwrap();
        assert!(off(&fp.plane, &ring) <= tol::PLANE_DIST);
        fp.outer = ring.clone();
        let region = Region {
            outer: ring,
            holes: Vec::new(),
        };
        let face = OrientedFace::new(region, &fp, true, (Side::B, 0)).unwrap();
        assert_eq!(face.outer, reversed);
        assert!(!face.no_plane);
        assert!(
            off(&face.plane, &face.outer) <= tol::PLANE_DIST,
            "the flipped piece fits its plane: {:e}",
            off(&face.plane, &face.outer)
        );
        assert!(face.plane.normal().dot(fp.plane.normal()) < 0.0);
    }

    impl FacePoly {
        fn clone_for_test(&self) -> FacePoly {
            FacePoly {
                plane_tol: self.plane_tol,
                flat_tol: self.flat_tol,
                outer: self.outer.clone(),
                holes: self.holes.clone(),
                plane: self.plane,
                material: self.material,
                back_material: self.back_material,
                uv_frame: self.uv_frame,
                surface: self.surface,
                blend: self.blend,
                attrs: self.attrs.clone(),
                source: self.source,
                ids: self.ids.clone(),
                far: self.far,
                split_from: self.split_from,
                pass_through: self.pass_through,
                no_plane: self.no_plane,
            }
        }
    }

    // ─────────────────────────────────── indexed lookups vs their references

    // The references the indexed lookups are checked against: a scan of
    // every vertex, as they were before the index.

    /// The vertices among `candidates` (indices into `vertices`, ascending)
    /// that lie in the interior of the edge `p`–`q`, within
    /// [`tol::POINT_MERGE`], ordered from `p` to `q`.
    fn points_inside_edge(
        p: Point3,
        q: Point3,
        vertices: &[Point3],
        candidates: impl IntoIterator<Item = usize>,
    ) -> Vec<Point3> {
        let len2 = (q - p).length_squared();
        if len2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
            return Vec::new();
        }
        // Cheap box reject first: almost no kept vertex is anywhere near a
        // given edge.
        let (lo, hi) = edge_box(p, q);
        let mut inside: Vec<(f64, Point3)> = candidates
            .into_iter()
            .map(|i| vertices[i])
            .filter(|&r| {
                in_box(r, lo, hi)
                    && !r.approx_eq(p, tol::POINT_MERGE)
                    && !r.approx_eq(q, tol::POINT_MERGE)
                    && point_segment_distance(r, p, q) <= tol::POINT_MERGE
            })
            .map(|r| ((r - p).dot(q - p) / len2, r))
            .collect();
        inside.sort_by(|a, b| a.0.total_cmp(&b.0));
        inside.into_iter().map(|(_, r)| r).collect()
    }

    /// The box of the edge `p`–`q`, padded by [`tol::POINT_MERGE`].
    fn edge_box(p: Point3, q: Point3) -> (Point3, Point3) {
        (
            Point3::new(
                p.x.min(q.x) - tol::POINT_MERGE,
                p.y.min(q.y) - tol::POINT_MERGE,
                p.z.min(q.z) - tol::POINT_MERGE,
            ),
            Point3::new(
                p.x.max(q.x) + tol::POINT_MERGE,
                p.y.max(q.y) + tol::POINT_MERGE,
                p.z.max(q.z) + tol::POINT_MERGE,
            ),
        )
    }

    fn in_box(r: Point3, lo: Point3, hi: Point3) -> bool {
        r.x >= lo.x && r.x <= hi.x && r.y >= lo.y && r.y <= hi.y && r.z >= lo.z && r.z <= hi.z
    }

    /// [`split_edges_at_vertices`] as it was, scanning every kept vertex for
    /// every edge: the reference the indexed version is checked against.
    fn split_edges_at_vertices_linear(faces: &mut [OrientedFace]) {
        let mut vertices: Vec<Point3> = Vec::new();
        for f in faces.iter() {
            for &p in f.outer.iter().chain(f.holes.iter().flatten()) {
                intern_linear(p, &mut vertices);
            }
        }
        let split = |ring: &mut Vec<Point3>| {
            let n = ring.len();
            let mut out: Vec<Point3> = Vec::with_capacity(n);
            for i in 0..n {
                let (p, q) = (ring[i], ring[(i + 1) % n]);
                out.push(p);
                out.extend(points_inside_edge(p, q, &vertices, 0..vertices.len()));
            }
            *ring = out;
        };
        for f in faces.iter_mut() {
            split(&mut f.outer);
            for h in f.holes.iter_mut() {
                split(h);
            }
        }
    }

    /// Interns `p` into `positions` by a scan: the reference [`Interner`] is
    /// checked against.
    fn intern_linear(p: Point3, positions: &mut Vec<Point3>) -> usize {
        if let Some(i) = positions
            .iter()
            .position(|&q| p.approx_eq(q, tol::POINT_MERGE))
        {
            return i;
        }
        positions.push(p);
        positions.len() - 1
    }

    use proptest::prelude::*;

    /// Points on a coarse lattice, each nudged by up to twice the merge
    /// distance, so many pairs merge and many just miss.
    fn clustered_point() -> impl Strategy<Value = Point3> {
        (
            (0i32..4, 0i32..4, 0i32..4),
            (-2.0f64..2.0, -2.0f64..2.0, -2.0f64..2.0),
        )
            .prop_map(|((i, j, k), (a, b, c))| {
                let m = tol::POINT_MERGE;
                Point3::new(
                    f64::from(i) * 0.25 + a * m,
                    f64::from(j) * 0.25 + b * m,
                    f64::from(k) * 0.25 + c * m,
                )
            })
    }

    fn oriented(outer: Vec<Point3>, up: bool) -> OrientedFace {
        OrientedFace {
            outer_ids: vec![None; outer.len()],
            hole_ids: Vec::new(),
            blend: None,
            source: Provenance {
                side: 0,
                face: FaceId::default(),
            },
            outer,
            holes: Vec::new(),
            plane: Plane::from_point_normal(
                Point3::ORIGIN,
                Vec3::new(0.0, 0.0, if up { 1.0 } else { -1.0 }),
            )
            .unwrap(),
            material: None,
            back_material: None,
            uv_frame: None,
            surface: None,
            attrs: AttrDict::default(),
            split_from: None,
            no_plane: false,
            origin: (Side::A, 0),
            tidy_group: None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// The cell-map interner returns what the scan returns, point by point.
        #[test]
        fn interner_matches_the_scan(pts in prop::collection::vec(clustered_point(), 1..120)) {
            let mut indexed = Interner::new();
            let mut linear: Vec<Point3> = Vec::new();
            for p in pts {
                prop_assert_eq!(indexed.intern(p), intern_linear(p, &mut linear));
            }
            prop_assert_eq!(indexed.positions, linear);
        }

        /// Splitting edges at the vertices on them, by the x-sorted index,
        /// gives the rings the scan of every vertex gives.
        #[test]
        fn split_edges_matches_the_scan(
            rings in prop::collection::vec(
                prop::collection::vec(
                    // Lattice points along a few shared lines, nudged, so
                    // vertices lie on other faces' edges.
                    ((0i32..5, 0i32..2), (-1.0f64..1.0, -1.0f64..1.0)).prop_map(|((i, j), (a, b))| {
                        let m = tol::POINT_MERGE;
                        Point3::new(f64::from(i) * 0.25 + a * m, f64::from(j) * 0.5 + b * m, 0.0)
                    }),
                    3..7,
                ),
                1..12,
            )
        ) {
            let mut a: Vec<OrientedFace> = rings.iter().cloned().map(|r| oriented(r, true)).collect();
            let mut b: Vec<OrientedFace> = rings.into_iter().map(|r| oriented(r, true)).collect();
            split_edges_at_vertices(&mut a);
            split_edges_at_vertices_linear(&mut b);
            for (fa, fb) in a.iter().zip(&b) {
                prop_assert_eq!(&fa.outer, &fb.outer);
            }
        }

        /// Keeping faces through the cell index keeps exactly the faces the
        /// scan of every kept face keeps.
        #[test]
        fn kept_faces_match_the_scan(
            faces in prop::collection::vec(
                (prop::collection::vec(clustered_point(), 3..5), any::<bool>()),
                1..60,
            )
        ) {
            let mut indexed: Vec<OrientedFace> = Vec::new();
            {
                let mut kept = KeptFaces::new(&mut indexed);
                for (outer, up) in &faces {
                    kept.push_unless_coincident(oriented(outer.clone(), *up));
                }
            }
            let mut linear: Vec<OrientedFace> = Vec::new();
            for (outer, up) in &faces {
                let g = oriented(outer.clone(), *up);
                if !linear.iter().any(|f| faces_coincide(f, &g)) {
                    linear.push(g);
                }
            }
            prop_assert_eq!(indexed.len(), linear.len());
            for (f, g) in indexed.iter().zip(&linear) {
                prop_assert_eq!(&f.outer, &g.outer);
            }
        }
    }

    // ─────────────────────────────────── the tidy (`tidy`): fallback, identity

    /// A 10 mm box, and a tetrahedron whose 5 mm base lies 5 mm under the
    /// box top and whose apex is `h` above the top's middle (the kernel's
    /// sliver spec geometry).
    fn box_and_tet(h: f64) -> (Object, Object) {
        let mm = 1e-3;
        let side = 10.0 * mm;
        let r = 5.0 * mm / 3f64.sqrt();
        let mut pts: Vec<Point3> = [90.0f64, 210.0, 330.0]
            .iter()
            .map(|d| {
                let t = d.to_radians();
                Point3::new(side / 2.0 + r * t.cos(), side / 2.0 + r * t.sin(), 5.0 * mm)
            })
            .collect();
        pts.push(Point3::new(side / 2.0, side / 2.0, side + h));
        let tet = Object::from_polygons(
            &pts,
            &[vec![0, 2, 1], vec![0, 1, 3], vec![1, 2, 3], vec![2, 0, 3]],
        )
        .unwrap();
        (cube(Point3::ORIGIN, Point3::new(side, side, side)), tet)
    }

    fn run_tidied(
        op: BooleanOp,
        a: &Object,
        b: &Object,
    ) -> (Result<Object, BooleanError>, BooleanTidy) {
        let mut tidy = BooleanTidy::default();
        let r = execute(op, a, b, &Transform::IDENTITY, &mut tidy);
        (r, tidy)
    }

    /// With every tidied result refused, the boolean answers exactly what
    /// the untidied faces give: at 50 nm a valid solid keeping the cap
    /// (reported as kept, nothing merged), at 5 nm the refusal that path
    /// has always given. Without the switch both are tidied.
    #[test]
    fn a_tidy_that_fails_falls_back_to_the_untidied_result() {
        let set = |on: bool| tidy::FORCE_FALLBACK.with(|c| c.set(on));
        let (a, b) = box_and_tet(5e-8);
        set(true);
        let (fell, tidy) = run_tidied(BooleanOp::Union, &a, &b);
        set(false);
        let fell = fell.expect("the untidied union is valid");
        assert!(
            tidy.slivers_kept > 0 && tidy.slivers_merged == 0,
            "{tidy:?}"
        );
        assert_eq!(tidy.points_moved, 0);
        assert!(fell.vertices().len() > 8, "the cap is kept");
        tidy::SKIP_TIDY.with(|c| c.set(true));
        let (plain, _) = run_tidied(BooleanOp::Union, &a, &b);
        tidy::SKIP_TIDY.with(|c| c.set(false));
        assert_eq!(
            fell.encode(&|_| 0),
            plain.unwrap().encode(&|_| 0),
            "exactly the untidied result"
        );
        let (tidied, tidy) = run_tidied(BooleanOp::Union, &a, &b);
        assert_eq!(tidied.unwrap().vertices().len(), 8);
        assert!(tidy.slivers_merged > 0 && tidy.slivers_kept == 0);

        let (a, b) = box_and_tet(5e-9);
        set(true);
        let (fell, tidy) = run_tidied(BooleanOp::Union, &a, &b);
        set(false);
        assert_eq!(fell.err(), Some(BooleanError::ResultTooThin));
        assert_eq!(tidy, BooleanTidy::default(), "untouched on Err");
        assert!(run_tidied(BooleanOp::Union, &a, &b).0.is_ok());
    }

    /// A box from `lo` (corner) and `size`, native or through the import
    /// builder.
    fn any_box(lo: (f64, f64, f64), size: (f64, f64, f64), imported: bool) -> Object {
        let (x, y, z) = lo;
        let (w, d, h) = size;
        let pts = [
            Point3::new(x, y, z),
            Point3::new(x + w, y, z),
            Point3::new(x + w, y + d, z),
            Point3::new(x, y + d, z),
            Point3::new(x, y, z + h),
            Point3::new(x + w, y, z + h),
            Point3::new(x + w, y + d, z + h),
            Point3::new(x, y + d, z + h),
        ];
        let faces = [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        if imported {
            Object::from_polygons_import_for_test(&pts, &faces).unwrap()
        } else {
            Object::from_polygons(&pts, &faces).unwrap()
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        /// Booleans of boxes, native or flat imported, at millimeter to
        /// meter scale and turned a little: whenever the cut leaves no face
        /// under the feature floor (the tidy reports nothing), the answer
        /// is byte for byte the one with no tidy at all.
        #[test]
        fn booleans_with_no_sliver_are_byte_identical_to_no_tidy(
            scale in prop::sample::select(vec![1e-3, 1e-2, 1.0]),
            (ax, ay, az) in (0.0f64..1.0, 0.0f64..1.0, 0.0f64..1.0),
            (bx, by, bz) in (0.0f64..1.5, 0.0f64..1.5, 0.0f64..1.5),
            (sx, sy, sz) in (0.2f64..1.5, 0.2f64..1.5, 0.2f64..1.5),
            turn in -0.3f64..0.3,
            (a_imp, b_imp) in (any::<bool>(), any::<bool>()),
        ) {
            let a = any_box((ax * scale, ay * scale, az * scale), (scale, scale, scale), a_imp);
            let b = any_box(
                (bx * scale, by * scale, bz * scale),
                (sx * scale, sy * scale, sz * scale),
                b_imp,
            );
            let t = Transform::rotation(Vec3::new(0.3, -0.2, 1.0), turn).unwrap();
            for op in [BooleanOp::Union, BooleanOp::Intersect, BooleanOp::Subtract] {
                let mut tidy = BooleanTidy::default();
                let with = execute(op, &a, &b, &t, &mut tidy);
                tidy::SKIP_TIDY.with(|c| c.set(true));
                let without = execute(op, &a, &b, &t, &mut BooleanTidy::default());
                tidy::SKIP_TIDY.with(|c| c.set(false));
                if tidy.is_empty() {
                    match (with, without) {
                        (Ok(x), Ok(y)) => prop_assert_eq!(x.encode(&|_| 0), y.encode(&|_| 0)),
                        (x, y) => prop_assert_eq!(x.err(), y.err()),
                    }
                }
            }
        }
    }

    const MM: f64 = 1e-3;
    const SPHERE_RADIUS: f64 = 5.0 * MM;
    const SPHERE_CENTER: Point3 = Point3::new(2.0 * MM, 3.0 * MM, 7.0 * MM);

    /// An icosphere: an icosahedron subdivided four times (2,562 vertices,
    /// 5,120 triangles), turned off-axis, every vertex on the sphere around
    /// [`SPHERE_CENTER`]. Triangles wound outward.
    fn icosphere() -> (Vec<Point3>, Vec<Vec<usize>>) {
        let t = (1.0 + 5f64.sqrt()) / 2.0;
        let mut pts: Vec<Vec3> = [
            (-1.0, t, 0.0),
            (1.0, t, 0.0),
            (-1.0, -t, 0.0),
            (1.0, -t, 0.0),
            (0.0, -1.0, t),
            (0.0, 1.0, t),
            (0.0, -1.0, -t),
            (0.0, 1.0, -t),
            (t, 0.0, -1.0),
            (t, 0.0, 1.0),
            (-t, 0.0, -1.0),
            (-t, 0.0, 1.0),
        ]
        .iter()
        .map(|&(x, y, z)| Vec3::new(x, y, z).normalized().unwrap())
        .collect();
        let mut tris: Vec<[usize; 3]> = vec![
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        for _ in 0..4 {
            let mut mid: BTreeMap<(usize, usize), usize> = BTreeMap::new();
            let mut midpoint = |a: usize, b: usize, pts: &mut Vec<Vec3>| {
                *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    pts.push(((pts[a] + pts[b]) * 0.5).normalized().unwrap());
                    pts.len() - 1
                })
            };
            let mut next = Vec::with_capacity(tris.len() * 4);
            for [a, b, c] in tris {
                let ab = midpoint(a, b, &mut pts);
                let bc = midpoint(b, c, &mut pts);
                let ca = midpoint(c, a, &mut pts);
                next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
            }
            tris = next;
        }
        let turn = Transform::rotation(Vec3::new(0.3, 0.5, 0.8), 0.37).unwrap();
        let positions = pts
            .iter()
            .map(|&v| {
                let p = turn.apply_point(Point3::ORIGIN + v * SPHERE_RADIUS);
                p + (SPHERE_CENTER - Point3::ORIGIN)
            })
            .collect();
        (positions, tris.into_iter().map(|t| t.to_vec()).collect())
    }

    /// The sphere from `a_tidy_reaching_past_the_rebuilt_region_grows_it`
    /// (integration specs) and its box: 80 nm under the vertex nearest
    /// latitude 31.5, longitude 179, so the tidy reaches past the rebuilt
    /// region and the region must grow at least once.
    fn sphere_and_box() -> (Object, Object) {
        let (pts, faces) = icosphere();
        let s = Object::from_polygons_import_for_test(&pts, &faces).unwrap();
        let (lat, lon) = (31.5f64.to_radians(), 179f64.to_radians());
        let target = SPHERE_CENTER
            + Vec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()) * SPHERE_RADIUS;
        let v = s
            .vertices()
            .values()
            .map(|v| v.position)
            .min_by(|p, q| (*p - target).length().total_cmp(&(*q - target).length()))
            .unwrap();
        let half = 0.4 * MM;
        let (lo, hi) = (
            Point3::new(v.x - half, v.y - half, SPHERE_CENTER.z - 7.0 * MM),
            Point3::new(v.x + half, v.y + half, v.z - 80e-9),
        );
        let c = [
            Point3::new(lo.x, lo.y, lo.z),
            Point3::new(hi.x, lo.y, lo.z),
            Point3::new(hi.x, hi.y, lo.z),
            Point3::new(lo.x, hi.y, lo.z),
            Point3::new(lo.x, lo.y, hi.z),
            Point3::new(hi.x, lo.y, hi.z),
            Point3::new(hi.x, hi.y, hi.z),
            Point3::new(lo.x, hi.y, hi.z),
        ];
        let b = Object::from_polygons(
            &c,
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
        (s, b)
    }

    /// The growth cap ends in the full build: with the region allowed to
    /// grow zero or one round, a tidy that reaches past it (the box under
    /// the sphere's vertex) gets every face arranged instead, and the
    /// boolean still commits a valid, watertight result with its slivers
    /// merged, the same volume as the uncapped run to the tidy's reach.
    #[test]
    fn a_growth_cap_ends_in_the_full_build_and_still_commits() {
        let (s, b) = sphere_and_box();
        let run = |op: BooleanOp, cap: usize| {
            GROWTH_CAP.with(|c| c.set(cap));
            let mut tidy = BooleanTidy::default();
            let r = execute(op, &s, &b, &Transform::IDENTITY, &mut tidy);
            GROWTH_CAP.with(|c| c.set(MAX_REGION_GROWTH));
            (r, tidy)
        };
        for op in [BooleanOp::Union, BooleanOp::Subtract] {
            let (free, _) = run(op, MAX_REGION_GROWTH);
            let free = free.unwrap_or_else(|e| panic!("{op:?} uncapped: {e:?}"));
            for cap in [0, 1] {
                let (r, tidy) = run(op, cap);
                let r = r.unwrap_or_else(|e| panic!("{op:?} cap {cap}: {e:?}"));
                r.validate().unwrap();
                assert_eq!(r.watertight(), WatertightState::Watertight);
                assert!(tidy.slivers_merged > 0, "{op:?} cap {cap}: {tidy:?}");
                let (v, w) = (r.volume().unwrap(), free.volume().unwrap());
                assert!(
                    (v - w).abs() < 1e-6 * w,
                    "{op:?} cap {cap}: volume {v} against {w}"
                );
            }
        }
    }
}
