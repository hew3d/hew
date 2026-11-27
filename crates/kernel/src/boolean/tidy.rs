//! The boolean's tidy: merging away the pieces of its own cut narrower than
//! [`tol::FEATURE_MIN`] (0.1 µm), the one bounded exception to
//! DEVELOPMENT.md rule 4's "never welds".
//!
//! A cut through a dense mesh leaves faces at every size down to the merge
//! distance where it passes near a vertex or an edge. Points merge at
//! [`tol::POINT_MERGE`] and a face needs [`tol::MIN_FACE_WIDTH`] to keep a
//! plane, so a piece between the two is distinct but cannot be kept. None
//! of it is anything a person drew, can see or can make, so the boolean
//! merges it into its neighbors, here, on the kept faces before they are
//! welded into a solid:
//!
//! - **Only where the cut made a sliver.** A kept face is a *trigger* when
//!   a ring of it holds a point the cut made (one that is no vertex of
//!   either operand, by bits) and is narrower than `FEATURE_MIN`
//!   ([`ring_width`]), or when the face has no plane of its own. With no
//!   trigger nothing here runs and the boolean is bit-identical to one
//!   without the tidy. Faces made only of operand vertices are never
//!   triggers: an operand's own needles are not this cut's business.
//! - **Bounded, deterministic weld.** The points of trigger faces, and
//!   every result or operand vertex within [`tol::TIDY_REACH`] of one, are
//!   visited in a fixed order (vertices of `a`, of `b`, then cut points,
//!   each in first-appearance order). Each joins the nearest anchor within
//!   reach of it and of every point already merged into it (ties: the
//!   earlier anchor), or becomes an anchor. Anchors never move, so no point
//!   moves farther than `TIDY_REACH` and clusters cannot chain along a
//!   strip; operand vertices win, so cut points slide onto them.
//! - **Points on edges.** An anchor within reach of the inside of an edge
//!   that ends at an anchor is inserted into it, on every face holding the
//!   edge, so a needle whose tip is far from both ends folds into the
//!   edge it hugs.
//! - **Rings cleaned.** Repeated points and spikes go, a ring under three
//!   points goes (its face with it, when it is the outer ring), and two
//!   faces that became one sheet seen from both sides cancel. A ring that
//!   pinches (visits a point twice) splits there, and each piece's winding
//!   says what it is, whichever ring it came from: wound with the face's
//!   normal it bounds a face (an outer lobe, or an island left inside a
//!   pinched hole), wound against it it is a hole (an inward lobe of the
//!   outer ring, or a lobe of a hole), given to the one face piece that
//!   contains it. A piece with no winding, or a hole no one piece
//!   contains, abandons the tidy.
//! - **Planes.** A face whose points all stay within the result's planarity
//!   tolerance of its plane keeps it into assembly (which, for a result
//!   with no holes, rebuilds every face on its outer ring's refit; see
//!   the boolean module's refusals). Otherwise a triangle takes its own
//!   plane and a larger face is split into flat triangles whose diagonals
//!   come out soft (`faces_split`).
//! - **At most four rounds**, while triggers remain (a moved point can
//!   thin a neighbor).
//!
//! Then the faces are welded and validated as always. A tidy that cannot
//! give a valid solid is discarded and the untidied faces take the path
//! they always have, so nothing that committed without the tidy stops
//! committing; [`BooleanError::ResultTooThin`] survives only where both
//! fail. Everything the tidy does is counted in a [`BooleanTidy`] and
//! logged (`kernel::op`, `op = "boolean_tidy"`).

#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use tracing::info;

use super::ear_clip::triangulate;
use super::{FacePoly, Ident, OrientedFace, Side, point_segment_distance};
use crate::math::{Plane, Point3, Vec3, ring_width};
use crate::ops::{BooleanError, BooleanOp, BooleanTidy};
use crate::tol;
use crate::transform::Transform;

/// A point's identity: its coordinates' bits.
type Key = [u64; 3];

fn key(p: Point3) -> Key {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

/// Every original position the weld moved, and where it is now: by the
/// original's bits, (original, current).
type Moves = BTreeMap<Key, (Point3, Point3)>;

/// The most weld rounds a boolean runs.
const MAX_ROUNDS: usize = 4;

#[cfg(test)]
thread_local! {
    /// Test switch: refuse every tidied result, so the fallback runs.
    pub(super) static FORCE_FALLBACK: Cell<bool> = const { Cell::new(false) };
    /// Test switch: no tidy at all, the path every boolean took before it.
    pub(super) static SKIP_TIDY: Cell<bool> = const { Cell::new(false) };
}

/// `build` (the boolean's assembly) of the kept `faces`, after merging away
/// the pieces of the cut narrower than [`tol::FEATURE_MIN`] (see the module
/// docs). `polys` are both operands' faces as the arrangement saw them
/// (`b`'s in `a`'s frame); what was merged is added to `report`. With
/// `partial`, the faces are only the region the boolean rebuilt (its
/// untouched faces stay in the operand they belong to), so a tidy that
/// merges them all away is no empty result in itself.
pub(super) fn assemble_tidied<T>(
    op: BooleanOp,
    faces: Vec<OrientedFace>,
    polys: (&[FacePoly], &[FacePoly]),
    planarity_tol: f64,
    report: &mut BooleanTidy,
    partial: bool,
    build: impl Fn(Vec<OrientedFace>) -> Result<T, BooleanError>,
) -> Result<T, BooleanError> {
    #[cfg(test)]
    if SKIP_TIDY.with(Cell::get) {
        return build(faces);
    }
    // The cheap test first: no ring narrower than the feature floor, no
    // tidy, and the boolean is exactly what it was without one.
    let originals = faces.iter().any(is_thin).then(|| Originals::of(polys));
    let triggers = originals
        .as_ref()
        .map_or(0, |o| faces.iter().filter(|f| o.is_trigger(f)).count());
    let Some(originals) = originals.filter(|_| triggers > 0) else {
        return build(faces);
    };
    let untidied = faces.clone();
    let mut tidy = BooleanTidy::default();
    let welded = Weld::new(faces, &originals, polys, planarity_tol).run(&mut tidy);
    let failed = match welded {
        Ok((faces, _)) if faces.is_empty() && !partial => return Err(BooleanError::EmptyResult),
        Ok((mut faces, moved)) => {
            // A cutter that entered `a` by less than the feature floor
            // removed nothing.
            if op == BooleanOp::Subtract && removes_nothing(&faces, polys.0, &moved) {
                return Err(BooleanError::NothingRemoved);
            }
            reident(&mut faces, &untidied);
            match assemble_checked(faces, &build) {
                Ok(obj) => {
                    log(&tidy, None);
                    report.absorb(&tidy, &Transform::IDENTITY);
                    return Ok(obj);
                }
                Err(e) => format!("{e:?}"),
            }
        }
        Err(why) => why.to_string(),
    };
    // The tidy could not give a valid solid: the untidied faces take the
    // path they always have. A piece with no plane is what that path
    // refuses as too thin.
    if untidied.iter().any(|f| f.no_plane) {
        return Err(BooleanError::ResultTooThin);
    }
    let obj = build(untidied)?;
    let kept = BooleanTidy {
        slivers_kept: u32::try_from(triggers).unwrap_or(u32::MAX),
        ..BooleanTidy::default()
    };
    log(&kept, Some(&failed));
    report.absorb(&kept, &Transform::IDENTITY);
    Ok(obj)
}

/// `build` of a tidied face list (the test switch can refuse it).
fn assemble_checked<T>(
    faces: Vec<OrientedFace>,
    build: &impl Fn(Vec<OrientedFace>) -> Result<T, BooleanError>,
) -> Result<T, BooleanError> {
    #[cfg(test)]
    if FORCE_FALLBACK.with(Cell::get) {
        return Err(BooleanError::ArrangementFailed);
    }
    build(faces)
}

/// Gives every corner of the tidied `faces` the operand vertex it is
/// ([`Ident`]), from the untidied faces it came from: the weld moves points
/// only onto points that were already there (anchors never move), so each
/// corner's position is one a corner had before, bit for bit. A corner
/// takes the identity its own source face gave that position, else the
/// first one any face gave it; a position no face gave one is a point the
/// cut made.
fn reident(faces: &mut [OrientedFace], untidied: &[OrientedFace]) {
    let mut by_origin: BTreeMap<((u8, usize), Key), Ident> = BTreeMap::new();
    let mut any: BTreeMap<Key, Ident> = BTreeMap::new();
    let side = |s: Side| match s {
        Side::A => 0u8,
        Side::B => 1u8,
    };
    for f in untidied {
        let ids = f.outer_ids.iter().chain(f.hole_ids.iter().flatten());
        for (&p, &id) in f.outer.iter().chain(f.holes.iter().flatten()).zip(ids) {
            if id.is_some() {
                let origin = (side(f.origin.0), f.origin.1);
                by_origin.entry((origin, key(p))).or_insert(id);
                any.entry(key(p)).or_insert(id);
            }
        }
    }
    for f in faces.iter_mut() {
        let origin = (side(f.origin.0), f.origin.1);
        let look = |p: &Point3| {
            by_origin
                .get(&(origin, key(*p)))
                .or_else(|| any.get(&key(*p)))
                .copied()
                .flatten()
        };
        f.outer_ids = f.outer.iter().map(look).collect();
        f.hole_ids = f
            .holes
            .iter()
            .map(|h| h.iter().map(look).collect())
            .collect();
    }
}

fn log(tidy: &BooleanTidy, fell_back: Option<&str>) {
    info!(
        target: "kernel::op",
        op = "boolean_tidy",
        slivers_merged = tidy.slivers_merged,
        slivers_kept = tidy.slivers_kept,
        points_moved = tidy.points_moved,
        max_move = tidy.max_move,
        faces_split = tidy.faces_split,
        sites_total = tidy.sites_total,
        first_site = ?tidy.sites.first(),
        fell_back = fell_back.unwrap_or(""),
    );
}

/// Whether a ring is narrower than the feature floor.
fn thin(ring: &[Point3]) -> bool {
    ring_width(ring) < tol::FEATURE_MIN
}

/// Whether a face has a ring narrower than the feature floor or no plane:
/// the necessary condition for a trigger, without the vertex lookup.
fn is_thin(f: &OrientedFace) -> bool {
    f.no_plane || thin(&f.outer) || f.holes.iter().any(|h| thin(h))
}

/// Both operands' vertices, by bits: a result point that is none of them
/// is a point the cut made.
struct Originals {
    a: Vec<Key>,
    b: Vec<Key>,
}

impl Originals {
    fn of((a_polys, b_polys): (&[FacePoly], &[FacePoly])) -> Originals {
        let keys = |polys: &[FacePoly]| {
            let mut v: Vec<Key> = polys
                .iter()
                .flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten()))
                .map(|&p| key(p))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        Originals {
            a: keys(a_polys),
            b: keys(b_polys),
        }
    }

    fn in_a(&self, k: &Key) -> bool {
        self.a.binary_search(k).is_ok()
    }

    fn in_b(&self, k: &Key) -> bool {
        self.b.binary_search(k).is_ok()
    }

    fn has_new(&self, ring: &[Point3]) -> bool {
        ring.iter().any(|&p| {
            let k = key(p);
            !self.in_a(&k) && !self.in_b(&k)
        })
    }

    /// A face the tidy must act on: a ring narrower than the feature floor
    /// holding a point the cut made, or no plane at all.
    fn is_trigger(&self, f: &OrientedFace) -> bool {
        f.no_plane
            || (thin(&f.outer) && self.has_new(&f.outer))
            || f.holes.iter().any(|h| thin(h) && self.has_new(h))
    }
}

/// Points sorted by x, for "is any within `TIDY_REACH` of p" lookups.
struct Near {
    pts: Vec<Point3>,
}

impl Near {
    fn new(mut pts: Vec<Point3>) -> Near {
        pts.sort_by(|p, q| p.x.total_cmp(&q.x).then_with(|| key(*p).cmp(&key(*q))));
        pts.dedup_by_key(|p| key(*p));
        Near { pts }
    }

    /// The points within `reach` of `p` along x, in x order.
    fn around(&self, p: Point3, reach: f64) -> &[Point3] {
        let lo = self.pts.partition_point(|q| q.x < p.x - reach);
        let hi = self.pts.partition_point(|q| q.x <= p.x + reach);
        &self.pts[lo..hi.max(lo)]
    }

    fn any_within(&self, p: Point3, reach: f64) -> bool {
        self.around(p, reach)
            .iter()
            .any(|&q| (q - p).length() <= reach)
    }
}

/// Why a tidy gave up (the untidied faces are used instead).
#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
enum Abandon {
    /// A face whose rings pinched apart had a hole no single piece of it
    /// could be told to own.
    PinchedHoledFace,
    /// A piece of a pinched ring had no winding to tell a face from a hole
    /// (a zero or non-finite area).
    UnorientedPinch,
    /// A face bent by the weld could not be split into triangles.
    NoTriangulation,
}

impl std::fmt::Display for Abandon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Abandon::PinchedHoledFace => {
                write!(f, "a pinched face had a hole no one piece owned")
            }
            Abandon::UnorientedPinch => {
                write!(f, "a piece of a pinched ring had no winding")
            }
            Abandon::NoTriangulation => write!(f, "a bent face would not triangulate"),
        }
    }
}

/// The weld's working state over the kept faces.
struct Weld<'a> {
    faces: Vec<OrientedFace>,
    /// A number per face, kept while the face is, so a trigger can be
    /// followed across rounds.
    ids: Vec<u32>,
    next_id: u32,
    next_group: u32,
    originals: &'a Originals,
    polys: (&'a [FacePoly], &'a [FacePoly]),
    planarity_tol: f64,
    /// For a point other points merged into: every original position now
    /// at it (its own included).
    origins: BTreeMap<Key, Vec<Point3>>,
    /// Every original position that moved, and where it is now.
    moved: Moves,
    /// Operand vertices that moved away: never anchors again.
    gone: BTreeSet<Key>,
    /// Anchors something moved to, in the order they were first used.
    sites: Vec<Point3>,
    site_keys: BTreeSet<Key>,
    /// Every face that was a trigger in some round.
    ever: BTreeSet<u32>,
    faces_split: u32,
}

impl<'a> Weld<'a> {
    fn new(
        faces: Vec<OrientedFace>,
        originals: &'a Originals,
        polys: (&'a [FacePoly], &'a [FacePoly]),
        planarity_tol: f64,
    ) -> Weld<'a> {
        let n = u32::try_from(faces.len()).unwrap_or(u32::MAX);
        Weld {
            ids: (0..n).collect(),
            next_id: n,
            next_group: 0,
            faces,
            originals,
            polys,
            planarity_tol,
            origins: BTreeMap::new(),
            moved: BTreeMap::new(),
            gone: BTreeSet::new(),
            sites: Vec::new(),
            site_keys: BTreeSet::new(),
            ever: BTreeSet::new(),
            faces_split: 0,
        }
    }

    fn fresh_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn triggers(&self) -> Vec<usize> {
        (0..self.faces.len())
            .filter(|&i| self.originals.is_trigger(&self.faces[i]))
            .collect()
    }

    /// Runs the rounds and fills `tidy`; the tidied faces and where every
    /// point that moved went (by its original position), or why it gave
    /// up.
    fn run(mut self, tidy: &mut BooleanTidy) -> Result<(Vec<OrientedFace>, Moves), Abandon> {
        for _ in 0..MAX_ROUNDS {
            let triggers = self.triggers();
            if triggers.is_empty() {
                break;
            }
            for &i in &triggers {
                self.ever.insert(self.ids[i]);
            }
            self.round(&triggers)?;
        }
        let remaining: BTreeSet<u32> = self.triggers().into_iter().map(|i| self.ids[i]).collect();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        tidy.slivers_merged = count(self.ever.difference(&remaining).count());
        tidy.slivers_kept = count(remaining.len());
        let mut points = 0usize;
        for &(from, to) in self.moved.values() {
            if key(from) != key(to) {
                points += 1;
                tidy.max_move = tidy.max_move.max((to - from).length());
            }
        }
        tidy.points_moved = count(points);
        tidy.faces_split = self.faces_split;
        tidy.sites_total = count(self.sites.len());
        tidy.sites = self
            .sites
            .iter()
            .take(BooleanTidy::MAX_SITES)
            .copied()
            .collect();
        Ok((self.faces, self.moved))
    }

    /// One round over the current `triggers`.
    fn round(&mut self, triggers: &[usize]) -> Result<(), Abandon> {
        let reach = tol::TIDY_REACH;
        let near = Near::new(
            triggers
                .iter()
                .flat_map(|&i| {
                    let f = &self.faces[i];
                    f.outer.iter().chain(f.holes.iter().flatten()).copied()
                })
                .collect(),
        );

        // Candidates, ranked: vertices of `a`, of `b`, then cut points,
        // each in first-appearance order.
        let mut cands: BTreeMap<Key, ((u8, usize), Point3)> = BTreeMap::new();
        let mut offer = |p: Point3, rank: (u8, usize), gone: &BTreeSet<Key>| {
            let k = key(p);
            if !gone.contains(&k) && near.any_within(p, reach) {
                cands.entry(k).or_insert((rank, p));
            }
        };
        for (class, polys) in [(0u8, self.polys.0), (1u8, self.polys.1)] {
            let points = polys
                .iter()
                .flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten()));
            for (i, &p) in points.enumerate() {
                offer(p, (class, i), &self.gone);
            }
        }
        let points = self
            .faces
            .iter()
            .flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten()));
        for (i, &p) in points.enumerate() {
            let k = key(p);
            let class = if self.originals.in_a(&k) {
                0
            } else if self.originals.in_b(&k) {
                1
            } else {
                2
            };
            offer(p, (class, i), &self.gone);
        }
        let mut order: Vec<(Key, (u8, usize), Point3)> =
            cands.into_iter().map(|(k, (r, p))| (k, r, p)).collect();
        order.sort_by_key(|&(k, r, _)| (r, k));
        let order: Vec<Point3> = order.into_iter().map(|(_, _, p)| p).collect();
        let (anchors, to_anchor) = choose_anchors(&order, &self.origins);
        let anchor_keys: BTreeSet<Key> = anchors.iter().map(|&a| key(a)).collect();

        // Move every result point to its anchor.
        let mut changed = vec![false; self.faces.len()];
        let mut live = vec![false; anchors.len()];
        let mut done: BTreeSet<Key> = BTreeSet::new();
        for (fi, f) in self.faces.iter_mut().enumerate() {
            for ring in std::iter::once(&mut f.outer).chain(f.holes.iter_mut()) {
                for p in ring.iter_mut() {
                    let k = key(*p);
                    let Some(&ai) = to_anchor.get(&k) else {
                        continue;
                    };
                    live[ai] = true;
                    let a = anchors[ai];
                    let ka = key(a);
                    if ka == k {
                        continue;
                    }
                    if done.insert(k) {
                        let from = self.origins.remove(&k).unwrap_or_else(|| vec![*p]);
                        for &o in &from {
                            self.moved.insert(key(o), (o, a));
                        }
                        self.origins
                            .entry(ka)
                            .or_insert_with(|| vec![a])
                            .extend(from);
                        self.gone.insert(k);
                        if self.site_keys.insert(ka) {
                            self.sites.push(a);
                        }
                    }
                    *p = a;
                    changed[fi] = true;
                }
            }
        }

        // An anchor within reach of the inside of an edge that ends at an
        // anchor goes into it, on every face holding the edge.
        let live_anchors = Near::new(
            anchors
                .iter()
                .zip(&live)
                .filter(|&(_, &l)| l)
                .map(|(&a, _)| a)
                .collect(),
        );
        for (fi, f) in self.faces.iter_mut().enumerate() {
            for ring in std::iter::once(&mut f.outer).chain(f.holes.iter_mut()) {
                if insert_anchors_on_edges(ring, &anchor_keys, &live_anchors) {
                    changed[fi] = true;
                }
            }
        }

        // Clean the rings that changed; split pinched faces.
        let old = std::mem::take(&mut self.faces);
        let old_ids = std::mem::take(&mut self.ids);
        let mut dirty: Vec<bool> = Vec::new();
        for ((mut f, id), changed) in old.into_iter().zip(old_ids).zip(changed) {
            if !changed {
                self.faces.push(f);
                self.ids.push(id);
                dirty.push(false);
                continue;
            }
            let outers = clean_ring(std::mem::take(&mut f.outer));
            let holes: Vec<Vec<Vec<Point3>>> = std::mem::take(&mut f.holes)
                .into_iter()
                .map(clean_ring)
                .collect();
            // A sliver's plane is a placeholder: no winding to read.
            let normal = (!f.no_plane).then(|| f.plane.normal());
            let pieces = split_pinched(outers, holes, normal)?;
            let several = pieces.len() > 1;
            for (outer, holes) in pieces {
                let mut piece = f.clone();
                piece.outer = outer;
                piece.holes = holes;
                self.faces.push(piece);
                let piece_id = if several { self.fresh_id() } else { id };
                self.ids.push(piece_id);
                dirty.push(true);
            }
        }

        // Two faces that became one sheet seen from both sides cancel.
        let mut drop = vec![false; self.faces.len()];
        let mut sheets: BTreeMap<Vec<Key>, Vec<usize>> = BTreeMap::new();
        for (i, f) in self.faces.iter().enumerate() {
            if f.holes.is_empty() && f.outer.iter().any(|&p| anchor_keys.contains(&key(p))) {
                let mut ks: Vec<Key> = f.outer.iter().map(|&p| key(p)).collect();
                ks.sort_unstable();
                sheets.entry(ks).or_default().push(i);
            }
        }
        for group in sheets.values() {
            for (n, &i) in group.iter().enumerate() {
                if drop[i] {
                    continue;
                }
                let partner = group[n + 1..].iter().copied().find(|&j| {
                    !drop[j] && reversed_cycle(&self.faces[i].outer, &self.faces[j].outer)
                });
                if let Some(j) = partner {
                    drop[i] = true;
                    drop[j] = true;
                }
            }
        }

        // Planes for the faces that changed.
        let old = std::mem::take(&mut self.faces);
        let old_ids = std::mem::take(&mut self.ids);
        for (i, (f, id)) in old.into_iter().zip(old_ids).enumerate() {
            if drop[i] {
                continue;
            }
            if !dirty[i] {
                self.faces.push(f);
                self.ids.push(id);
                continue;
            }
            self.replane(f, id)?;
        }
        Ok(())
    }

    /// Gives a face the weld changed a plane: its own when every point is
    /// still within the result's planarity tolerance of it; a triangle's
    /// own Newell plane; else flat triangles with soft diagonals. The
    /// plane given here is the one assembly checks the face against only
    /// for a result with holes: a hole-free result is rebuilt on each
    /// face's ring refit (`Object::from_polygons_with_attrs_tol`).
    fn replane(&mut self, mut f: OrientedFace, id: u32) -> Result<(), Abandon> {
        let points = || f.outer.iter().chain(f.holes.iter().flatten()).copied();
        let within =
            |plane: &Plane| points().all(|p| plane.signed_distance(p).abs() <= self.planarity_tol);
        if !f.no_plane && within(&f.plane) {
            self.faces.push(f);
            self.ids.push(id);
            return Ok(());
        }
        if f.outer.len() == 3 && f.holes.is_empty() {
            if let Ok(plane) = crate::serialize::own_ring_plane(&f.outer, &[], self.planarity_tol) {
                f.plane = plane;
                f.no_plane = false;
            } else {
                f.no_plane = true;
            }
            self.faces.push(f);
            self.ids.push(id);
            return Ok(());
        }
        let tris =
            triangulate(&f.outer, &f.holes, f.plane.normal()).ok_or(Abandon::NoTriangulation)?;
        let all: Vec<Point3> = f
            .outer
            .iter()
            .chain(f.holes.iter().flatten())
            .copied()
            .collect();
        let group = self.next_group;
        self.next_group += 1;
        self.faces_split += 1;
        for t in tris {
            let outer = vec![all[t[0]], all[t[1]], all[t[2]]];
            let mut piece = f.clone();
            piece.holes = Vec::new();
            match crate::serialize::own_ring_plane(&outer, &[], self.planarity_tol) {
                Ok(plane) => {
                    piece.plane = plane;
                    piece.no_plane = false;
                }
                Err(_) => piece.no_plane = true,
            }
            piece.outer = outer;
            piece.tidy_group = Some(group);
            self.faces.push(piece);
            let fresh = self.fresh_id();
            self.ids.push(fresh);
        }
        Ok(())
    }
}

/// The anchors for `order` (the candidates, best first), greedily: each
/// joins the nearest anchor within [`tol::TIDY_REACH`] of it and of every
/// original position already merged into it (`origins`; ties go to the
/// earlier anchor), or becomes one. Anchors never move, so nothing moves
/// farther than the reach and no cluster chains along a strip. Returns the
/// anchors and, for every candidate, the index of its anchor.
fn choose_anchors(
    order: &[Point3],
    origins: &BTreeMap<Key, Vec<Point3>>,
) -> (Vec<Point3>, BTreeMap<Key, usize>) {
    let reach = tol::TIDY_REACH;
    let mut anchors: Vec<Point3> = Vec::new();
    let mut to_anchor: BTreeMap<Key, usize> = BTreeMap::new();
    for p in order {
        let k = key(*p);
        let merged: &[Point3] = origins
            .get(&k)
            .map(Vec::as_slice)
            .unwrap_or(std::slice::from_ref(p));
        let mut best: Option<(usize, f64)> = None;
        for (ai, &a) in anchors.iter().enumerate() {
            let d = (a - *p).length();
            if d <= reach
                && merged.iter().all(|&o| (a - o).length() <= reach)
                && best.is_none_or(|(_, bd)| d < bd)
            {
                best = Some((ai, d));
            }
        }
        let ai = match best {
            Some((ai, _)) => ai,
            None => {
                anchors.push(*p);
                anchors.len() - 1
            }
        };
        to_anchor.insert(k, ai);
    }
    (anchors, to_anchor)
}

/// Inserts into each edge of `ring` that ends at an anchor every live
/// anchor within [`tol::TIDY_REACH`] of its inside (and not within reach of
/// either end), ordered along the edge. Whether anything went in.
fn insert_anchors_on_edges(
    ring: &mut Vec<Point3>,
    anchor_keys: &BTreeSet<Key>,
    live: &Near,
) -> bool {
    let reach = tol::TIDY_REACH;
    let n = ring.len();
    let mut out: Vec<Point3> = Vec::with_capacity(n);
    let mut any = false;
    for i in 0..n {
        let (p, q) = (ring[i], ring[(i + 1) % n]);
        out.push(p);
        if !anchor_keys.contains(&key(p)) && !anchor_keys.contains(&key(q)) {
            continue;
        }
        let len2 = (q - p).length_squared();
        if len2 == 0.0 {
            continue;
        }
        let (lo_x, hi_x) = (p.x.min(q.x), p.x.max(q.x));
        let mid = Point3::new((lo_x + hi_x) / 2.0, p.y, p.z);
        let span = (hi_x - lo_x) / 2.0 + reach;
        let mut inside: Vec<(f64, Point3)> = live
            .around(mid, span)
            .iter()
            .copied()
            .filter(|&a| {
                !a.approx_eq(p, reach)
                    && !a.approx_eq(q, reach)
                    && point_segment_distance(a, p, q) <= reach
            })
            .map(|a| ((a - p).dot(q - p) / len2, a))
            .collect();
        if inside.is_empty() {
            continue;
        }
        inside.sort_by(|x, y| x.0.total_cmp(&y.0).then_with(|| key(x.1).cmp(&key(y.1))));
        any = true;
        out.extend(inside.into_iter().map(|(_, a)| a));
    }
    if any {
        *ring = out;
    }
    any
}

/// The rings a welded ring falls into: repeated points and spikes
/// (`a, b, a`) removed until none is left, then split wherever it visits a
/// point twice. Rings under three points are gone.
fn clean_ring(mut ring: Vec<Point3>) -> Vec<Vec<Point3>> {
    loop {
        let before = ring.len();
        dedup_cyclic(&mut ring);
        drop_one_spike(&mut ring);
        if ring.len() == before {
            break;
        }
    }
    if ring.len() < 3 {
        return Vec::new();
    }
    let mut seen: BTreeMap<Key, usize> = BTreeMap::new();
    for (j, &p) in ring.iter().enumerate() {
        if let Some(&i) = seen.get(&key(p)) {
            let inner = ring[i..j].to_vec();
            let mut rest = ring[j..].to_vec();
            rest.extend_from_slice(&ring[..i]);
            let mut out = clean_ring(rest);
            out.extend(clean_ring(inner));
            return out;
        }
        seen.insert(key(p), j);
    }
    vec![ring]
}

/// A face, as outer ring and holes.
type Piece = (Vec<Point3>, Vec<Vec<Point3>>);

/// The faces a welded face falls into, from what [`clean_ring`] made of its
/// outer ring (`outer_parts`) and of each of its holes (`hole_parts`, one
/// list per hole), on a face whose plane faces `normal`.
///
/// When the outer ring stays one ring and no hole splits, that is the face,
/// as it was: no winding is consulted. Otherwise a ring that visited a point
/// twice came apart into pieces whose windings say what they are, wherever
/// they came from: a piece wound with `normal` bounds a face (an outer
/// lobe, or an island a pinched hole left), one wound against it is a hole
/// (an inward lobe of the outer ring, or a lobe of a hole). Each hole goes
/// to the one face that contains it, tested at its first point that is no
/// point of any face ring. A piece with no winding abandons
/// ([`Abandon::UnorientedPinch`]), and so does a hole with no one face to
/// own it, or holes with no face at all ([`Abandon::PinchedHoledFace`]).
///
/// A face with no plane of its own (`normal` is `None`: a `no_plane`
/// sliver, whose plane is a placeholder) has no winding to read, so it
/// splits without one ([`split_unoriented`]).
fn split_pinched(
    outer_parts: Vec<Vec<Point3>>,
    hole_parts: Vec<Vec<Vec<Point3>>>,
    normal: Option<Vec3>,
) -> Result<Vec<Piece>, Abandon> {
    if outer_parts.is_empty() {
        return Ok(Vec::new());
    }
    if outer_parts.len() == 1 && hole_parts.iter().all(|h| h.len() <= 1) {
        let outer = outer_parts.into_iter().next().expect("one ring");
        return Ok(vec![(outer, hole_parts.into_iter().flatten().collect())]);
    }
    let Some(normal) = normal else {
        return split_unoriented(outer_parts, hole_parts);
    };
    let mut shells: Vec<Piece> = Vec::new();
    let mut holes: Vec<Vec<Point3>> = Vec::new();
    for ring in outer_parts
        .into_iter()
        .chain(hole_parts.into_iter().flatten())
    {
        let along = Plane::from_polygon_unfloored(&ring)
            .map(|p| p.normal().dot(normal))
            .filter(|d| *d != 0.0 && d.is_finite())
            .ok_or(Abandon::UnorientedPinch)?;
        if along > 0.0 {
            shells.push((ring, Vec::new()));
        } else {
            holes.push(ring);
        }
    }
    if shells.is_empty() {
        return Err(Abandon::PinchedHoledFace);
    }
    let shell_keys: BTreeSet<Key> = shells
        .iter()
        .flat_map(|(ring, _)| ring.iter().map(|&p| key(p)))
        .collect();
    for hole in holes {
        let probe = hole
            .iter()
            .copied()
            .find(|&p| !shell_keys.contains(&key(p)))
            .ok_or(Abandon::PinchedHoledFace)?;
        let mut owners = shells
            .iter()
            .enumerate()
            .filter(|(_, (ring, _))| crate::geom2d::point_inside_polygon(probe, ring, normal))
            .map(|(i, _)| i);
        match (owners.next(), owners.next()) {
            (Some(i), None) => shells[i].1.push(hole),
            _ => return Err(Abandon::PinchedHoledFace),
        }
    }
    Ok(shells)
}

/// [`split_pinched`] for a face with no plane of its own: its placeholder
/// plane says nothing about which way a piece winds, so none is read.
/// One outer ring is the face with every hole piece; several, with no hole
/// left, are a face each, for the tidy to merge away; several with holes
/// abandon ([`Abandon::PinchedHoledFace`]), there being no winding to say
/// which piece owns them.
fn split_unoriented(
    outer_parts: Vec<Vec<Point3>>,
    hole_parts: Vec<Vec<Vec<Point3>>>,
) -> Result<Vec<Piece>, Abandon> {
    let holes: Vec<Vec<Point3>> = hole_parts.into_iter().flatten().collect();
    if outer_parts.len() == 1 {
        let outer = outer_parts.into_iter().next().expect("one ring");
        return Ok(vec![(outer, holes)]);
    }
    if !holes.is_empty() {
        return Err(Abandon::PinchedHoledFace);
    }
    Ok(outer_parts
        .into_iter()
        .map(|ring| (ring, Vec::new()))
        .collect())
}

/// Removes consecutive repeats of a point, the last against the first too.
fn dedup_cyclic(ring: &mut Vec<Point3>) {
    ring.dedup_by_key(|p| key(*p));
    while ring.len() > 1 && key(ring[0]) == key(ring[ring.len() - 1]) {
        ring.pop();
    }
}

/// Removes the first spike `a, b, a` (its `b` and one `a`).
fn drop_one_spike(ring: &mut Vec<Point3>) {
    let n = ring.len();
    if n < 3 {
        return;
    }
    for k in 0..n {
        if key(ring[(k + n - 1) % n]) == key(ring[(k + 1) % n]) {
            let (x, y) = (k, (k + 1) % n);
            ring.remove(x.max(y));
            ring.remove(x.min(y));
            return;
        }
    }
}

/// Whether `y` is `x` run backward, from any starting point.
fn reversed_cycle(x: &[Point3], y: &[Point3]) -> bool {
    let n = x.len();
    if n != y.len() || n == 0 {
        return false;
    }
    let Some(o) = y.iter().position(|&q| key(q) == key(x[0])) else {
        return false;
    };
    (0..n).all(|i| key(x[i]) == key(y[(o + n - i) % n]))
}

/// Whether a subtract removed nothing: no piece of `b` is left, and the
/// points left are exactly the vertices of `a`, each where it was or where
/// the weld `moved` it (no farther than [`tol::TIDY_REACH`]). The result
/// then differs from `a` only by moves under the feature floor, so the
/// cutter took nothing, and committing it would only disturb `a`: a weld
/// can move a vertex of `a` onto another one within reach of it (an
/// operand's own sub-floor feature beside the cut).
fn removes_nothing(faces: &[OrientedFace], a_polys: &[FacePoly], moved: &Moves) -> bool {
    if faces.iter().any(|f| f.origin.0 != Side::A) {
        return false;
    }
    let now = |p: &Point3| moved.get(&key(*p)).map_or(key(*p), |&(_, to)| key(to));
    let of_a: BTreeSet<Key> = a_polys
        .iter()
        .flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten()))
        .map(now)
        .collect();
    let left: BTreeSet<Key> = faces
        .iter()
        .flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten()))
        .map(|p| key(*p))
        .collect();
    of_a == left
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    #[test]
    fn repeated_points_go() {
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 0.0),
        ];
        assert_eq!(
            clean_ring(ring),
            vec![vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)]]
        );
    }

    #[test]
    fn spikes_go_repeatedly() {
        // 0,0 → 1,0 → 2,0 → 1,0 → 1,1: the spike at 2,0 leaves 1,0 once.
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(2.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
        ];
        assert_eq!(
            clean_ring(ring),
            vec![vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)]]
        );
        // A ring that is all spike is gone.
        let ring = vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 0.0)];
        assert!(clean_ring(ring).is_empty());
        let ring = vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0), p(1.0, 0.0)];
        assert!(clean_ring(ring).is_empty());
    }

    #[test]
    fn a_pinched_ring_splits_in_two() {
        // A bow tie through 1,1.
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(2.0, 0.0),
            p(2.0, 2.0),
            p(1.0, 1.0),
            p(0.0, 2.0),
        ];
        let out = clean_ring(ring);
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0],
            vec![p(1.0, 1.0), p(0.0, 2.0), p(0.0, 0.0)],
            "the part holding the start"
        );
        assert_eq!(out[1], vec![p(1.0, 1.0), p(2.0, 0.0), p(2.0, 2.0)]);
        // A pinch whose second part is under three points leaves one ring.
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(2.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
        ];
        assert_eq!(clean_ring(ring).len(), 1);
    }

    const UP: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    /// An axis-aligned square from `(x0, y0)` to `(x1, y1)`, wound with +z.
    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point3> {
        vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)]
    }

    fn reversed(mut ring: Vec<Point3>) -> Vec<Point3> {
        ring.reverse();
        ring
    }

    #[test]
    fn an_unpinched_face_comes_through_as_it_was() {
        let outer = square(0.0, 0.0, 4.0, 4.0);
        let hole = reversed(square(1.0, 1.0, 2.0, 2.0));
        let out = split_pinched(
            vec![outer.clone()],
            vec![vec![hole.clone()], vec![]],
            Some(UP),
        );
        assert_eq!(out, Ok(vec![(outer, vec![hole.clone()])]));
        // No winding is consulted: a degenerate outer ring passes too.
        let flat = vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)];
        let out = split_pinched(vec![flat.clone()], vec![], Some(UP));
        assert_eq!(out, Ok(vec![(flat, vec![])]));
        // An outer ring that cleaned away takes the face, holes and all.
        assert_eq!(
            split_pinched(vec![], vec![vec![hole]], Some(UP)),
            Ok(vec![])
        );
    }

    #[test]
    fn an_inward_lobe_of_the_outer_ring_is_a_hole() {
        // A square whose ring dips in from its corner (0,0) to trace a small
        // square against the face's winding: the weld pinched it there.
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(1.0, 2.0),
            p(2.0, 2.0),
            p(2.0, 1.0),
            p(1.0, 1.0),
            p(0.0, 0.0),
            p(4.0, 0.0),
            p(4.0, 4.0),
            p(0.0, 4.0),
        ];
        let parts = clean_ring(ring);
        assert_eq!(parts.len(), 2);
        let out = split_pinched(parts, vec![], Some(UP)).unwrap();
        assert_eq!(out.len(), 1, "one face: {out:?}");
        let (outer, holes) = &out[0];
        assert_eq!(outer.len(), 4);
        assert_eq!(holes.len(), 1);
        assert!(crate::geom2d::signed_area_on_plane(&holes[0], UP) < 0.0);
        assert!(holes[0].contains(&p(1.0, 2.0)));
    }

    #[test]
    fn a_bow_tie_is_two_faces() {
        // A ring through 1,1 twice whose two lobes both wind with +z.
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(2.0, 1.0),
            p(2.0, 2.0),
            p(1.0, 2.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
        ];
        let parts = clean_ring(ring);
        assert_eq!(parts.len(), 2);
        let out = split_pinched(parts, vec![], Some(UP)).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(outer, holes)| holes.is_empty()
            && crate::geom2d::signed_area_on_plane(outer, UP) > 0.0));
    }

    #[test]
    fn a_pinched_hole_leaves_an_island() {
        // A hole whose ring crosses itself at 3,3: one lobe winds against
        // +z (a hole), the other with it (an island face in the hole's
        // place), beside an untouched second hole.
        let outer = square(0.0, 0.0, 10.0, 10.0);
        let pinched = vec![
            p(2.0, 2.0),
            p(2.0, 3.0),
            p(3.0, 3.0),
            p(4.0, 3.0),
            p(4.0, 4.0),
            p(3.0, 4.0),
            p(3.0, 3.0),
            p(3.0, 2.0),
        ];
        let other = reversed(square(6.0, 6.0, 7.0, 7.0));
        let parts = clean_ring(pinched);
        assert_eq!(parts.len(), 2);
        let out = split_pinched(
            vec![outer.clone()],
            vec![parts, vec![other.clone()]],
            Some(UP),
        )
        .unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].0, outer, "the outer piece comes first");
        assert_eq!(out[0].1.len(), 2, "the hole's lobe and the other hole");
        assert!(out[0].1.contains(&other));
        let (island, island_holes) = &out[1];
        assert!(island_holes.is_empty());
        assert!(crate::geom2d::signed_area_on_plane(island, UP) > 0.0);
        assert!(island.contains(&p(4.0, 4.0)));
    }

    #[test]
    fn a_piece_with_no_winding_or_no_owner_abandons() {
        // A pinched ring one of whose lobes is a line: no winding.
        let line = vec![p(5.0, 0.0), p(6.0, 0.0), p(7.0, 0.0)];
        let out = split_pinched(
            vec![square(0.0, 0.0, 4.0, 4.0), line.clone()],
            vec![],
            Some(UP),
        );
        assert_eq!(out, Err(Abandon::UnorientedPinch));
        // Every piece against the normal: holes with no face.
        let out = split_pinched(
            vec![
                reversed(square(0.0, 0.0, 1.0, 1.0)),
                reversed(square(2.0, 0.0, 3.0, 1.0)),
            ],
            vec![],
            Some(UP),
        );
        assert_eq!(out, Err(Abandon::PinchedHoledFace));
        // A hole outside both lobes of a bow tie.
        let out = split_pinched(
            vec![square(0.0, 0.0, 1.0, 1.0), square(1.0, 1.0, 2.0, 2.0)],
            vec![vec![reversed(square(5.0, 5.0, 6.0, 6.0))]],
            Some(UP),
        );
        assert_eq!(out, Err(Abandon::PinchedHoledFace));
        // A hole inside one lobe goes to it.
        let hole = reversed(square(0.25, 0.25, 0.5, 0.5));
        let out = split_pinched(
            vec![square(0.0, 0.0, 1.0, 1.0), square(1.0, 1.0, 2.0, 2.0)],
            vec![vec![hole.clone()]],
            Some(UP),
        )
        .unwrap();
        assert_eq!(out[0].1, vec![hole]);
        assert!(out[1].1.is_empty());
    }

    /// A sliver with no plane of its own carries a placeholder normal that
    /// says nothing about its windings: a pinch that leaves one lobe wound
    /// against it is still two faces, as every pinch of a hole-free face
    /// was before windings were read, for the tidy to merge away; read
    /// against the placeholder, that lobe would be a hole with no owner and
    /// abandon the tidy.
    #[test]
    fn a_pinched_sliver_with_no_plane_splits_without_reading_windings() {
        // Through 1,1 twice: one lobe with +z, one against it.
        let ring = vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(1.0, 2.0),
            p(2.0, 2.0),
            p(2.0, 1.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
        ];
        let parts = clean_ring(ring);
        assert_eq!(parts.len(), 2);
        let windings: Vec<f64> = parts
            .iter()
            .map(|r| crate::geom2d::signed_area_on_plane(r, UP))
            .collect();
        assert!(windings.iter().any(|&a| a > 0.0) && windings.iter().any(|&a| a < 0.0));
        assert_eq!(
            split_pinched(parts.clone(), vec![], Some(UP)),
            Err(Abandon::PinchedHoledFace),
            "read against a normal, the lobe against it has no owner"
        );
        let out = split_pinched(parts.clone(), vec![], None).unwrap();
        assert_eq!(
            out,
            parts
                .iter()
                .map(|r| (r.clone(), vec![]))
                .collect::<Vec<_>>(),
            "every lobe is a face"
        );
        // One outer ring keeps every piece of its holes, unread.
        let outer = square(0.0, 0.0, 4.0, 4.0);
        let holes = vec![vec![
            square(1.0, 1.0, 2.0, 2.0),
            reversed(square(2.0, 2.0, 3.0, 3.0)),
        ]];
        assert_eq!(
            split_pinched(vec![outer.clone()], holes.clone(), None),
            Ok(vec![(outer, holes[0].clone())])
        );
        // Several lobes and a hole: no winding to say which owns it.
        assert_eq!(
            split_pinched(
                parts,
                vec![vec![reversed(square(0.25, 0.25, 0.5, 0.5))]],
                None
            ),
            Err(Abandon::PinchedHoledFace)
        );
    }

    #[test]
    fn cycles_compare_from_any_start() {
        let x = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)];
        let same = [p(1.0, 1.0), p(0.0, 0.0), p(1.0, 0.0)];
        let back = [p(1.0, 0.0), p(0.0, 0.0), p(1.0, 1.0)];
        assert!(reversed_cycle(&x, &back));
        assert!(!reversed_cycle(&x, &same));
    }

    #[test]
    fn anchors_go_first_come_and_never_chain() {
        let none = BTreeMap::new();
        // Best first: an operand vertex, then cut points 0.15 µm and
        // 0.3 µm along. The first joins it; the second is out of its reach
        // and anchors itself, though it is within reach of the first.
        let v = p(1e-3, 0.0);
        let near = p(1e-3 + 1.5e-7, 0.0);
        let far = p(1e-3 + 3e-7, 0.0);
        let (anchors, to) = choose_anchors(&[v, near, far], &none);
        assert_eq!(anchors, vec![v, far]);
        assert_eq!([to[&key(v)], to[&key(near)], to[&key(far)]], [0, 0, 1]);
        // Equidistant from two anchors: the earlier wins.
        let (l, r) = (p(0.0, 0.0), p(3e-7, 0.0));
        let mid = p(1.5e-7, 0.0);
        let (anchors, to) = choose_anchors(&[l, r, mid], &none);
        assert_eq!(anchors, vec![l, r]);
        assert_eq!(to[&key(mid)], 0);
        // A point that others merged into joins only an anchor within
        // reach of all of them.
        let merged_into = p(0.0, 1e-7);
        let origins: BTreeMap<Key, Vec<Point3>> =
            [(key(merged_into), vec![merged_into, p(0.0, 2.9e-7)])].into();
        let anchor = p(0.0, -9e-8);
        let (anchors, _) = choose_anchors(&[anchor, merged_into], &origins);
        assert_eq!(
            anchors,
            vec![anchor, merged_into],
            "would carry 2.9e-7 too far"
        );
    }

    #[test]
    fn edges_take_the_anchors_inside_them_in_order() {
        let a = p(0.0, 0.0);
        let b = p(1e-3, 0.0);
        let c = p(1e-3, 1e-3);
        let near1 = p(6e-4, 5e-8);
        let near2 = p(3e-4, -5e-8);
        let far = p(5e-4, 5e-7);
        let keys: BTreeSet<Key> = [a].iter().map(|&q| key(q)).collect();
        let live = Near::new(vec![near1, near2, far, a]);
        let mut ring = vec![a, b, c];
        assert!(insert_anchors_on_edges(&mut ring, &keys, &live));
        assert_eq!(ring, vec![a, near2, near1, b, c]);
        // An edge with no anchor at either end is left alone.
        let mut ring = vec![b, c, p(0.0, 1e-3)];
        assert!(!insert_anchors_on_edges(&mut ring, &keys, &live));
    }
}
