//! **A closed set of faces is a solid** (wireframe faces and solids §5,
//! rule 4).
//!
//! A *face* here is a faced region of a free sketch of the frame, detached
//! or not, that takes part ([`Document::sketch_takes_part`], §16 R11):
//! shown, and in no group or in the group an open session is editing. When
//! an edit leaves faces that enclose a space with no gap, they become a
//! solid in the same undo step:
//!
//! 1. **Gather.** Every face connected to a seed face through shared
//!    corners. Faces with the same loop on one plane (one wall drawn twice,
//!    say as a Detached shape and as ordinary lines) are one face: the
//!    ordinary one, else the earlier drawn; the other stays a sketch
//!    (§16 R22). Rings are **refined** first: a corner of one ring lying on
//!    another ring's edge splits that edge, so faces that meet at a
//!    T-junction share the same segments. Faces that lie over each other
//!    otherwise decline the cells they bound, naming the edges where they
//!    do.
//! 2. **Rims.** Open edges (one face) chain into loops. A loop is
//!    *cappable on Object O* when it is closed, simple and planar and it and
//!    its interior lie on O's faces on that plane; O must be visible in the
//!    frame. Its **cap** is the flat polygon it bounds, cappable loops inside
//!    it being holes.
//! 3. **Prune.** A face with an open edge on no cappable rim goes, and the
//!    rims are found again, until nothing changes: flaps fall away.
//! 4. **Cells.** At every edge the faces (caps included) are ordered by
//!    angle around it; each wedge between two neighbours joins the two face
//!    sides facing into it. The classes of face sides are the cells; a
//!    *bounded* cell has positive volume wound outward, and one is kept
//!    when a seed face bounds it.
//! 5. **Build.** Each kept cell is built from its refined rings through
//!    the strict polygon builder ([`crate::tol::PLANE_DIST`], with holes),
//!    and must come out watertight and valid; anything else declines it,
//!    typed, and leaves the faces as they were. Nothing is repaired.
//! 6. **Commit** (§5.4). No caps: a new Object. Caps on one growable Object
//!    O, no detached face, the cell outside O: O grows (a union, then the
//!    coplanar seams dissolved with O's own coplanar edges kept) into a new
//!    Object carrying O's name, tags and parent, O hidden. Any other caps: a
//!    new Object with its caps as faces. The cell inside O: nothing (§9 D3).
//!    A refused union: a new Object, reported as
//!    [`CellOutcome::Unjoined`]. The consumed regions' scaffolding leaves
//!    each sketch (an emptied sketch is deleted), and loose free lines of
//!    the cell's kind (ordinary or detached, §16 R14) lying along the new
//!    solid's edges go.
//!
//! The search runs inside the edit's own undo step after the close pass
//! ([`Document::close_faces_through`]): a draft, and a move, copy or vertex
//! drag of free drawing. Never on load, import, undo or redo.

use std::collections::{BTreeMap, BTreeSet};

use super::draft::{along, face_on_plane_either, uncovered_parts};
use super::*;
use crate::error::TopologyError;
use crate::geom2d::{interior_point_of_loops, point_inside_polygon, point_near_segment};
use crate::sketch::{SketchEdgeId, SketchRegionId};
use crate::tol;

/// Where a cell search starts ([`Document::close_cells`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellSeed {
    /// Every faced region of this free sketch (Make Solid's input: no
    /// "made by this edit" condition).
    Sketch(SketchId),
    /// Every face whose ring runs along this segment (an edge an edit
    /// added or moved), whichever sketch holds it.
    Edge(Point3, Point3),
}

/// Which Objects a cell may grow, and which it may close against
/// (wireframe faces §5.4, "growable").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellScope {
    /// The world (`None`) or the component definition the faces live in.
    pub frame: Option<ComponentId>,
    /// Inside an entered Object, only that Object grows; `None` at the top
    /// level and in a group or component session, where any Object
    /// [`Document::drawable_objects_on_plane`]'s owner rule accepts does.
    pub object: Option<ObjectId>,
    /// Objects the view hides (the app's Hide): never closed against,
    /// never grown.
    pub hidden: BTreeSet<ObjectId>,
}

/// What one cell, or one gathered set of faces that closed no cell, came
/// to ([`Document::close_cells`]).
#[derive(Debug, Clone, PartialEq)]
pub enum CellOutcome {
    /// A new Object.
    Created(ObjectId),
    /// The cell closed against Object `from`, which grew into `to` (`from`
    /// is hidden, `to` carries its name, tags and parent).
    Grew { from: ObjectId, to: ObjectId },
    /// The cell closed against Object `with` but the union refused, so it
    /// became `object`, a solid of its own.
    Unjoined { object: ObjectId, with: ObjectId },
    /// No solid; the faces stay as they were.
    Declined(CellDecline),
}

/// Why faces made no solid, with what a viewer can mark.
#[derive(Debug, Clone, PartialEq)]
pub struct CellDecline {
    pub reason: CellDeclineReason,
    /// The gathered faces' open edges on no cappable rim, in the frame.
    pub open_edges: Vec<(Point3, Point3)>,
    /// Closed loops of open edges whose corners lie on no one plane.
    pub non_planar_loops: Vec<Vec<Point3>>,
    /// For [`CellDeclineReason::OverlappingFaces`]: the edges along which
    /// two faces lie over each other (wireframe faces §16 R22), in the
    /// frame; empty for every other reason.
    pub overlapping_edges: Vec<(Point3, Point3)>,
}

/// The typed reason of a [`CellDecline`].
#[derive(Debug, Clone, PartialEq)]
pub enum CellDeclineReason {
    /// No closed cell: open edges remain once the flaps are pruned.
    NotClosed,
    /// Two faces lie over each other along an edge.
    OverlappingFaces,
    /// The cell lies inside the Object its caps are on (§9 D3).
    InsideSolid(ObjectId),
    /// Both sides of one face bound the cell, or both bound the outside
    /// while beside a cell (a lid over a sealed pocket).
    SelfTouching,
    /// The cell's faces build no valid solid.
    Invalid(TopologyError),
}

/// What a set of faces is missing before it can close (wireframe faces §6,
/// [`Document::faces_open_edges`]): the overlay Make Solid shows when it
/// declines. Coordinates are in the faces' frame (world for world
/// sketches, definition-local for a definition's).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenEdges {
    /// The open edges (one face) on no cappable rim and on none of
    /// `non_planar_loops`, each once.
    pub open_edges: Vec<(Point3, Point3)>,
    /// Closed loops of open edges whose corners lie on no one plane, each
    /// as its corners in order (the loop closes back to the first).
    pub non_planar_loops: Vec<Vec<Point3>>,
    /// The edges along which two faces lie over each other, where faces
    /// that are not the same loop overlap (§16 R22), each once.
    pub overlapping_edges: Vec<(Point3, Point3)>,
}

impl OpenEdges {
    /// How many open edges there are.
    pub fn count(&self) -> usize {
        self.open_edges.len()
    }

    /// Whether nothing is missing.
    pub fn is_empty(&self) -> bool {
        self.open_edges.is_empty()
            && self.non_planar_loops.is_empty()
            && self.overlapping_edges.is_empty()
    }
}

/// What a cells pass wrote, not yet recorded.
#[derive(Debug, Default)]
pub(super) struct CellsPass {
    pub(super) actions: Vec<DocAction>,
    pub(super) outcomes: Vec<CellOutcome>,
    pub(super) objects: Vec<ObjectId>,
    pub(super) sketches: Vec<SketchId>,
    pub(super) components: Vec<ComponentId>,
    pub(super) instances: Vec<InstanceId>,
}

/// The open edges on no cappable rim and the non-planar rim loops of a
/// component's first prune round.
type FirstRound = (Vec<(usize, usize)>, Vec<Vec<usize>>);

/// A face of the pool: one faced region of a free sketch.
#[derive(Debug, Clone)]
struct PoolFace {
    sketch: SketchId,
    region: SketchRegionId,
    detached: bool,
    normal: Vec3,
    /// Canonical vertex ids: the outer ring (counter-clockwise about
    /// `normal`), then the holes (clockwise).
    rings: Vec<Vec<usize>>,
}

/// A face of one gathered component, refined; a cap has no pool face.
#[derive(Debug, Clone)]
struct CFace {
    pool: Option<usize>,
    cap_on: Option<ObjectId>,
    normal: Vec3,
    rings: Vec<Vec<usize>>,
}

/// Points merged within [`tol::POINT_MERGE`], found through a bucket grid.
#[derive(Debug, Default)]
struct Verts {
    pts: Vec<Point3>,
    grid: BTreeMap<(i64, i64, i64), Vec<usize>>,
}

/// The bucket edge of [`Verts`]' grid: a bucketing size, not a tolerance
/// (two points within [`tol::POINT_MERGE`] fall in one bucket or in
/// neighbouring ones, which the lookup visits).
const BUCKET: f64 = tol::POINT_MERGE * 4096.0;

impl Verts {
    fn id(&mut self, p: Point3) -> usize {
        let cell = |x: f64| (x / BUCKET).floor() as i64;
        let near = |x: f64| -> Vec<i64> {
            let c = cell(x);
            let mut v = vec![c];
            if cell(x - 2.0 * tol::POINT_MERGE) != c {
                v.push(c - 1);
            }
            if cell(x + 2.0 * tol::POINT_MERGE) != c {
                v.push(c + 1);
            }
            v
        };
        for &i in &near(p.x) {
            for &j in &near(p.y) {
                for &k in &near(p.z) {
                    if let Some(ids) = self.grid.get(&(i, j, k)) {
                        for &id in ids {
                            if self.pts[id].approx_eq(p, tol::POINT_MERGE) {
                                return id;
                            }
                        }
                    }
                }
            }
        }
        let id = self.pts.len();
        self.pts.push(p);
        self.grid
            .entry((cell(p.x), cell(p.y), cell(p.z)))
            .or_default()
            .push(id);
        id
    }
}

/// A face's loop as a key that two faces with the same loop share: each
/// ring without its straight-through corners (a corner lying on the segment
/// between its neighbours, within [`tol::POINT_MERGE`], is no corner of the
/// loop: the same wall drawn once as a Rectangle and once with an extra
/// point along a side is the same loop), rotated to start at its least
/// vertex and read the way round whose second vertex is the lesser (facing
/// does not matter), the holes sorted after the outer ring.
fn loop_key(pts: &[Point3], rings: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let canon = |ring: &Vec<usize>| -> Vec<usize> {
        let mut ring = ring.clone();
        while ring.len() > 3 {
            let n = ring.len();
            let straight = (0..n).find(|&k| {
                let (a, v, b) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
                point_near_segment(pts[v], pts[a], pts[b], tol::POINT_MERGE)
                    && (pts[v] - pts[a]).dot(pts[b] - pts[v]) > 0.0
            });
            match straight {
                Some(k) => {
                    ring.remove(k);
                }
                None => break,
            }
        }
        let n = ring.len();
        let start = (0..n).min_by_key(|&k| ring[k]).unwrap_or(0);
        let fwd: Vec<usize> = (0..n).map(|k| ring[(start + k) % n]).collect();
        let back: Vec<usize> = (0..n).map(|k| ring[(start + n - k) % n]).collect();
        fwd.min(back)
    };
    let mut holes: Vec<Vec<usize>> = rings.iter().skip(1).map(canon).collect();
    holes.sort();
    rings.first().map(canon).into_iter().chain(holes).collect()
}

/// `ring` without consecutive repeats (two corners merged into one).
fn dedup_ring(mut ring: Vec<usize>) -> Vec<usize> {
    ring.dedup();
    while ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    ring
}

/// Twice the area of `ring` about `normal`, signed (positive
/// counter-clockwise).
fn ring_area(pts: &[Point3], ring: &[usize], normal: Vec3) -> f64 {
    let n = ring.len();
    let o = pts[ring[0]];
    let mut s = Vec3::new(0.0, 0.0, 0.0);
    for i in 1..n.saturating_sub(1) {
        s = s + (pts[ring[i]] - o).cross(pts[ring[i + 1]] - o);
    }
    s.dot(normal)
}

/// A unit vector perpendicular to unit `d`.
fn perpendicular(d: Vec3) -> Vec3 {
    let axis = if d.x.abs() <= d.y.abs() && d.x.abs() <= d.z.abs() {
        Vec3::new(1.0, 0.0, 0.0)
    } else if d.y.abs() <= d.z.abs() {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let v = d.cross(axis);
    v * (1.0 / v.length())
}

/// Undirected edge key.
fn ekey(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

/// Union-find over face sides.
struct Classes(Vec<usize>);

impl Classes {
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }
    fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.0[hi] = lo;
        }
    }
}

/// One open-edge loop of a component.
#[derive(Debug, Clone)]
struct Rim {
    /// Its corners in order (closed), or `None` for a branching or open
    /// chain.
    cycle: Option<Vec<usize>>,
    edges: Vec<(usize, usize)>,
}

/// What a cappable rim closes against: the Object, the face normal it
/// meets there (pointing out of the Object), and the rim's plane.
#[derive(Debug, Clone, Copy)]
struct CapOn {
    object: ObjectId,
    normal: Vec3,
    plane: Plane,
}

/// How a kept cell is committed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Commit {
    Create,
    Grow(ObjectId),
}

/// A built cell waiting for its commit.
struct Planned {
    solid: Object,
    commit: Commit,
    /// The cell's edges by the kind of the faces that hold them,
    /// `[ordinary, detached]`: a loose line is trimmed along the edges a
    /// face of its own kind holds (§16 R14). A mixed cell (ordinary and
    /// detached faces, which Make Solid or a detached shape closing on an
    /// ordinary face can gather) so trims each kind along its own faces'
    /// edges, both along an edge the two kinds share.
    edges: [Vec<(Point3, Point3)>; 2],
}

/// What one try of [`Document::make_solid_from_sketches`] wrote, not yet
/// recorded.
#[derive(Default)]
struct MadeSolid {
    outcomes: Vec<CellOutcome>,
    actions: Vec<DocAction>,
    change: DocChange,
    /// The free sketches the close pass wrote a face into.
    closed: Vec<SketchId>,
}

/// What the analysis of the whole pool decided, before anything changes.
#[derive(Default)]
struct Plan {
    cells: Vec<Planned>,
    consumed: BTreeMap<SketchId, BTreeSet<SketchRegionId>>,
    declines: Vec<CellDecline>,
}

impl Document {
    /// **Make solids from closed faces** (wireframe faces §5): searches
    /// from `seeds` for cells the faces of `scope.frame` enclose and commits
    /// each as §5.4 says, as ONE undo step of its own (the automatic search
    /// after a draft or a sketch move runs the same pass inside that edit's
    /// step). Returns one outcome per cell, and a
    /// [`CellOutcome::Declined`] per gathered set of faces that closed
    /// none, with its open edges and non-planar loops. When nothing
    /// closes, nothing changes (this search writes no faces of its own; a
    /// Make Solid that makes a solid keeps its close pass's faces whole,
    /// [`Document::make_solid_from_sketches`]).
    pub fn close_cells(&mut self, seeds: &[CellSeed], scope: &CellScope) -> Vec<CellOutcome> {
        let pass = self.cells_pass(seeds, scope, true);
        if pass.actions.is_empty() {
            return pass.outcomes;
        }
        self.record_cells(pass.actions, &pass.outcomes);
        pass.outcomes
    }

    /// **Make Solid from faces** (wireframe faces §6, §16 R26): first the
    /// close pass ([`Document::close_faces_through`]) seeded by every edge
    /// of every selected sketch that takes part in its frame (§16 R11),
    /// each edge's own sketch its home, a detached sketch's own plane a
    /// candidate as a detached draft's is, so faces an older build never
    /// formed are made from the lines already there; then the cells search
    /// of [`Document::close_cells`] seeded by every faced region of
    /// `sketches` ([`CellSeed::Sketch`], no "made by this edit" condition)
    /// and of every sketch the pass wrote a face into, each in the
    /// automatic scope of its frame (every growable Object of the frame but
    /// those the view hides). What it makes is committed as §5.4 says, the
    /// pass's faces with it, as ONE undo step labeled "Make Solid".
    ///
    /// Make Solid makes a solid, or nothing at all; adoption is
    /// all-or-nothing (§16 R33). When at least one solid forms, the whole
    /// pass is kept, faces on shapes that stayed open included (the faces a
    /// draw of the same lines would have made), and the Declined outcomes
    /// beside the solid describe the document as it stands after the pass.
    /// When the search after the pass makes no solid, the search runs again
    /// on the document as it is, without the pass's faces; when that makes
    /// none either, nothing changes and nothing is recorded, and the
    /// untouched document's declines are returned (with their open edges
    /// and non-planar loops, as [`Document::faces_open_edges`] lists them).
    /// When nothing closes, nothing changes. Returns one outcome per
    /// cell, a [`CellOutcome::Declined`] per gathered set of faces that
    /// closed none, and the change. A sketch that holds no face of its
    /// frame (an Object's or a group's drawing, a sketch on a hidden tag)
    /// contributes nothing.
    ///
    /// # Errors
    /// [`DocumentError::UnknownSketch`] for a stale or deleted sketch; the
    /// document is untouched.
    pub fn make_solid_from_sketches(
        &mut self,
        sketches: &[SketchId],
    ) -> Result<(Vec<CellOutcome>, DocChange), DocumentError> {
        let by_frame = self.seeds_by_frame(sketches)?;
        // With the pass, on a copy: adopted only if it makes a solid.
        let mut trial = self.clone();
        if let Some(made) = trial.make_solid_pass(&by_frame, true) {
            *self = trial;
            return Ok(self.record_make_solid(made));
        }
        let made = self.make_solid_pass(&by_frame, false);
        Ok(match made {
            Some(made) => self.record_make_solid(made),
            None => {
                // Nothing is made: the declines of the document as it is.
                let mut outcomes = Vec::new();
                for (frame, seeds) in &by_frame {
                    let scope = self.auto_cell_scope(*frame);
                    let plan = self.plan_cells(seeds, &scope, true);
                    outcomes.extend(plan.declines.into_iter().map(CellOutcome::Declined));
                }
                (outcomes, DocChange::default())
            }
        })
    }

    /// One try of [`Document::make_solid_from_sketches`], per frame of
    /// `by_frame`: the close pass over the selected sketches' edges first
    /// when `with_pass`, then the cells search. Writes, and returns what it
    /// wrote unrecorded, when a solid was made; `None` (the document then
    /// possibly written, so a try that may fail runs on a copy) when none
    /// was.
    fn make_solid_pass(
        &mut self,
        by_frame: &BTreeMap<Option<ComponentId>, Vec<CellSeed>>,
        with_pass: bool,
    ) -> Option<MadeSolid> {
        let mut made = MadeSolid::default();
        for (&frame, seeds) in by_frame {
            let mut seeds = seeds.clone();
            if with_pass {
                let mut edges: Vec<super::draft::ClosingEdge> = Vec::new();
                for seed in &seeds {
                    let CellSeed::Sketch(s) = *seed else { continue };
                    if !self.cell_sketch(s, frame) {
                        continue;
                    }
                    let sk = &self.sketches[s];
                    for e in sk.edges().values() {
                        edges.push((
                            s,
                            sk.vertices()[e.from].position,
                            sk.vertices()[e.to].position,
                        ));
                    }
                }
                let pass = self.close_faces_pass(&edges, frame, false, true);
                made.actions.extend(pass.actions);
                for &s in &pass.closed {
                    let seed = CellSeed::Sketch(s);
                    if !seeds.contains(&seed) {
                        seeds.push(seed);
                    }
                }
                merge_unique(&mut made.change.sketches_touched, pass.touched);
                merge_unique(&mut made.closed, pass.closed);
            }
            let scope = self.auto_cell_scope(frame);
            let cells = self.cells_pass(&seeds, &scope, true);
            made.outcomes.extend(cells.outcomes);
            made.actions.extend(cells.actions);
            merge_unique(&mut made.change.objects_touched, cells.objects);
            merge_unique(&mut made.change.sketches_touched, cells.sketches);
            merge_unique(&mut made.change.components_touched, cells.components);
            merge_unique(&mut made.change.instances_touched, cells.instances);
        }
        made.outcomes
            .iter()
            .any(|o| !matches!(o, CellOutcome::Declined(_)))
            .then_some(made)
    }

    /// Records a Make Solid that made a solid as ONE undo step (the solids'
    /// rows leading, [`Document::record_cells`]) and returns its outcomes
    /// and change.
    fn record_make_solid(&mut self, made: MadeSolid) -> (Vec<CellOutcome>, DocChange) {
        self.record_cells(made.actions, &made.outcomes);
        self.faces_closed_last = made.closed;
        (made.outcomes, made.change)
    }

    /// What the faces of `sketches` are missing before they can close
    /// (wireframe faces §6): the open edges and non-planar loops that the
    /// cells search seeded as [`Document::make_solid_from_sketches`] would
    /// decline with, and the edges along which faces that are not the same
    /// loop lie over each other (§16 R22). Empty when every gathered set
    /// closes. Read-only; a
    /// stale or deleted sketch contributes nothing.
    pub fn faces_open_edges(&self, sketches: &[SketchId]) -> OpenEdges {
        let live: Vec<SketchId> = sketches
            .iter()
            .copied()
            .filter(|s| self.sketches.contains_key(*s) && !self.sketch_tombstoned(s))
            .collect();
        let by_frame = self.seeds_by_frame(&live).unwrap_or_default();
        let mut out = OpenEdges::default();
        let mut open: Vec<(Point3, Point3)> = Vec::new();
        let same = |(a, b): (Point3, Point3), (c, d): (Point3, Point3)| {
            (a, b) == (c, d) || (a, b) == (d, c)
        };
        for (frame, seeds) in by_frame {
            let scope = self.auto_cell_scope(frame);
            let plan = self.plan_cells(&seeds, &scope, true);
            for d in plan.declines {
                for l in d.non_planar_loops {
                    if !out.non_planar_loops.contains(&l) {
                        out.non_planar_loops.push(l);
                    }
                }
                open.extend(d.open_edges);
                for e in d.overlapping_edges {
                    if !out.overlapping_edges.iter().any(|&f| same(e, f)) {
                        out.overlapping_edges.push(e);
                    }
                }
            }
        }
        // An edge of a non-planar loop is shown as that loop, not twice.
        let on_loop = |e: (Point3, Point3)| {
            out.non_planar_loops
                .iter()
                .any(|l| (0..l.len()).any(|i| same(e, (l[i], l[(i + 1) % l.len()]))))
        };
        let mut edges: Vec<(Point3, Point3)> = Vec::new();
        for e in open {
            if !on_loop(e) && !edges.iter().any(|&f| same(e, f)) {
                edges.push(e);
            }
        }
        out.open_edges = edges;
        out
    }

    /// `sketches` as [`CellSeed::Sketch`] seeds grouped by the frame each
    /// lives in (in frame order).
    fn seeds_by_frame(
        &self,
        sketches: &[SketchId],
    ) -> Result<BTreeMap<Option<ComponentId>, Vec<CellSeed>>, DocumentError> {
        let mut by_frame: BTreeMap<Option<ComponentId>, Vec<CellSeed>> = BTreeMap::new();
        for &s in sketches {
            if !self.sketches.contains_key(s) || self.sketch_tombstoned(&s) {
                return Err(DocumentError::UnknownSketch);
            }
            let seed = CellSeed::Sketch(s);
            let list = by_frame
                .entry(self.def_sketches.get(&s).copied())
                .or_default();
            if !list.contains(&seed) {
                list.push(seed);
            }
        }
        Ok(by_frame)
    }

    /// Records a cells pass's rows as ONE undo step of its own. The
    /// solids' rows lead, so the step describes itself as Make Solid
    /// (`DocAction::CreatedObjectFromFaces` / `GrewObjectFromFaces` first;
    /// the rest keep their order — each row restores a disjoint part).
    fn record_cells(&mut self, actions: Vec<DocAction>, outcomes: &[CellOutcome]) {
        let (mut lead, rest): (Vec<DocAction>, Vec<DocAction>) =
            actions.into_iter().partition(|a| {
                matches!(
                    a,
                    DocAction::CreatedObjectFromFaces { .. }
                        | DocAction::GrewObjectFromFaces { .. }
                )
            });
        lead.extend(rest);
        self.undo.push(DocAction::Compound {
            actions: lead,
            meta: None,
        });
        self.commit_new_action();
        self.cells_last = outcomes.to_vec();
        self.debug_validate();
    }

    /// The cells the most recent recorded edit that ran the search made or
    /// declined (as [`DraftReport::cells`] lists them); empty after any
    /// other recorded edit. Undo and redo leave it as it was.
    pub fn cells_by_last_edit(&self) -> &[CellOutcome] {
        &self.cells_last
    }

    /// Sets the Objects the view hides (the app's Hide, session-only): the
    /// automatic cell search never closes against or grows them.
    pub fn set_view_hidden_objects(&mut self, objects: impl IntoIterator<Item = ObjectId>) {
        self.view_hidden = objects.into_iter().collect();
    }

    /// The automatic scope of an edit in `frame`: every growable Object of
    /// the frame but those the view hides.
    pub(super) fn auto_cell_scope(&self, frame: Option<ComponentId>) -> CellScope {
        CellScope {
            frame,
            object: None,
            hidden: self.view_hidden.clone(),
        }
    }

    /// Whether free sketch `s` takes part in the cells of `frame`
    /// ([`Document::sketch_takes_part`], wireframe faces §16 R11): shown,
    /// in the frame, and in no group or in the group an open session is
    /// editing.
    fn cell_sketch(&self, s: SketchId, frame: Option<ComponentId>) -> bool {
        self.sketch_takes_part(s, frame)
    }

    /// Whether Object `o` can be closed against in `scope`: it takes part
    /// in `scope.frame` ([`Document::object_takes_part`]) and the scope
    /// does not hide it.
    fn cell_visible(&self, o: ObjectId, scope: &CellScope) -> bool {
        !scope.hidden.contains(&o) && self.object_takes_part(o, scope.frame)
    }

    /// Whether Object `o` may grow in `scope` (§5.4): visible, accepted by
    /// the drawing owner rule, inside the entered Object if there is one,
    /// and inside the open edit session.
    fn cell_growable(&self, o: ObjectId, scope: &CellScope) -> bool {
        // `cell_visible` holds the drawing owner rule (top-level or the
        // definition's member) already.
        self.cell_visible(o, scope)
            && scope.object.is_none_or(|only| only == o)
            && self.explode_scope_object(o).is_ok()
    }

    /// The search and its writes, unrecorded ([`Document::close_cells`]).
    /// `report_unclosed`: whether a gathered set that closed nothing is
    /// reported (with its open edges) — the automatic search leaves those
    /// out, since most edits close nothing.
    pub(super) fn cells_pass(
        &mut self,
        seeds: &[CellSeed],
        scope: &CellScope,
        report_unclosed: bool,
    ) -> CellsPass {
        let mut out = CellsPass::default();
        if seeds.is_empty() {
            return out;
        }
        let plan = self.plan_cells(seeds, scope, report_unclosed);
        for d in &plan.declines {
            out.outcomes.push(CellOutcome::Declined(d.clone()));
        }
        if plan.cells.is_empty() {
            return out;
        }
        self.commit_cells(plan, scope, &mut out);
        out
    }

    // ------------------------------------------------------------ analysis

    /// Gathers, refines, prunes, walks and builds; changes nothing.
    fn plan_cells(&self, seeds: &[CellSeed], scope: &CellScope, report_unclosed: bool) -> Plan {
        let frame = scope.frame;
        let mut verts = Verts::default();
        let mut pool: Vec<PoolFace> = Vec::new();
        for (s, sk) in self.sketches.iter() {
            if !self.cell_sketch(s, frame) {
                continue;
            }
            let normal = sk.plane().normal();
            for (rid, r) in sk.regions().iter() {
                if r.faceless {
                    continue;
                }
                let mut rings = Vec::with_capacity(1 + r.holes.len());
                for ring in std::iter::once(&r.outer).chain(r.holes.iter()) {
                    let ids: Vec<usize> = ring
                        .iter()
                        .map(|&v| verts.id(sk.vertices()[v].position))
                        .collect();
                    rings.push(dedup_ring(ids));
                }
                if rings.iter().any(|r| r.len() < 3) {
                    continue;
                }
                pool.push(PoolFace {
                    sketch: s,
                    region: rid,
                    detached: sk.detached(),
                    normal,
                    rings,
                });
            }
        }
        let pts = verts.pts;
        // ---- one face per loop (§16 R22) ----
        // Faces with the same loop (the same corners in the same cyclic
        // order, either way round, and the same holes) on one plane are the
        // same wall drawn twice — a Detached shape and ordinary drawing
        // never merge, and two sketches facing apart do not arrange — so
        // they are one face here: an ordinary one over a detached one, else
        // the earlier drawn (the lower stable id). The others take no part
        // and stay as they are; a seed on one seeds the face kept.
        let keep_of: Vec<usize> = {
            let rank = |i: usize| {
                let s = pool[i].sketch;
                (
                    pool[i].detached,
                    self.sid_of(&EntityRef::Sketch(s)).unwrap_or(u64::MAX),
                    s,
                )
            };
            let keys: Vec<Vec<Vec<usize>>> =
                pool.iter().map(|f| loop_key(&pts, &f.rings)).collect();
            let mut best: BTreeMap<&Vec<Vec<usize>>, usize> = BTreeMap::new();
            for (i, k) in keys.iter().enumerate() {
                let e = best.entry(k).or_insert(i);
                if rank(i) < rank(*e) {
                    *e = i;
                }
            }
            keys.iter().map(|k| best[k]).collect()
        };
        // ---- seeds ----
        let mut is_seed = vec![false; pool.len()];
        for (i, f) in pool.iter().enumerate() {
            let seeded = seeds.iter().any(|seed| match *seed {
                CellSeed::Sketch(s) => f.sketch == s,
                CellSeed::Edge(a, b) => f.rings.iter().any(|ring| {
                    (0..ring.len()).any(|k| {
                        let (c, d) = (pts[ring[k]], pts[ring[(k + 1) % ring.len()]]);
                        along(c, d, a, b) || along(a, b, c, d)
                    })
                }),
            });
            is_seed[keep_of[i]] |= seeded;
        }
        if !is_seed.iter().any(|&s| s) {
            return Plan::default();
        }
        // ---- gather: components through shared corners ----
        let mut at_vertex: Vec<Vec<usize>> = vec![Vec::new(); pts.len()];
        for (i, f) in pool.iter().enumerate() {
            if keep_of[i] != i {
                continue;
            }
            for &v in f.rings.iter().flatten() {
                if at_vertex[v].last() != Some(&i) {
                    at_vertex[v].push(i);
                }
            }
        }
        let mut comp_of = vec![usize::MAX; pool.len()];
        let mut plan = Plan::default();
        for start in 0..pool.len() {
            if !is_seed[start] || comp_of[start] != usize::MAX {
                continue;
            }
            let mut comp = vec![start];
            comp_of[start] = start;
            let mut k = 0;
            while k < comp.len() {
                let f = comp[k];
                k += 1;
                for &v in pool[f].rings.iter().flatten() {
                    for &g in &at_vertex[v] {
                        if comp_of[g] == usize::MAX {
                            comp_of[g] = start;
                            comp.push(g);
                        }
                    }
                }
            }
            comp.sort_unstable();
            self.plan_component(
                &pool,
                &pts,
                &comp,
                &is_seed,
                scope,
                report_unclosed,
                &mut plan,
            );
        }
        plan
    }

    /// The cells of one gathered component, into `plan`.
    #[allow(clippy::too_many_arguments)]
    fn plan_component(
        &self,
        pool: &[PoolFace],
        pts: &[Point3],
        comp: &[usize],
        is_seed: &[bool],
        scope: &CellScope,
        report_unclosed: bool,
        plan: &mut Plan,
    ) {
        // One plane holds no cell: the common case (drawing on the
        // ground), decided before any refinement.
        let first = &pool[comp[0]];
        let p0 = pts[first.rings[0][0]];
        let n0 = first.normal;
        let coplanar = comp.iter().all(|&f| {
            pool[f]
                .rings
                .iter()
                .flatten()
                .all(|&v| (pts[v] - p0).dot(n0).abs() <= tol::PLANE_DIST)
        });
        if coplanar && !report_unclosed {
            return;
        }
        // ---- refine: corners lying on another ring's edge split it ----
        let mut local: Vec<usize> = comp
            .iter()
            .flat_map(|&f| pool[f].rings.iter().flatten().copied())
            .collect();
        local.sort_unstable();
        local.dedup();
        local.sort_by(|&a, &b| pts[a].x.total_cmp(&pts[b].x).then(a.cmp(&b)));
        let xs: Vec<f64> = local.iter().map(|&v| pts[v].x).collect();
        let refine = |ring: &[usize]| -> Vec<usize> {
            let n = ring.len();
            let mut out = Vec::with_capacity(n);
            for k in 0..n {
                let (i, j) = (ring[k], ring[(k + 1) % n]);
                out.push(i);
                let (a, b) = (pts[i], pts[j]);
                let m = 2.0 * tol::POINT_MERGE;
                let lo = xs.partition_point(|&x| x < a.x.min(b.x) - m);
                let hi = xs.partition_point(|&x| x <= a.x.max(b.x) + m);
                let ab = b - a;
                let len2 = ab.length_squared();
                let mut on: Vec<(f64, usize)> = local[lo..hi]
                    .iter()
                    .copied()
                    .filter(|&v| v != i && v != j)
                    .filter(|&v| point_near_segment(pts[v], a, b, tol::POINT_MERGE))
                    .map(|v| ((pts[v] - a).dot(ab) / len2, v))
                    .filter(|&(t, _)| t > 0.0 && t < 1.0)
                    .collect();
                on.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
                out.extend(on.into_iter().map(|(_, v)| v));
            }
            dedup_ring(out)
        };
        let mut faces: Vec<CFace> = comp
            .iter()
            .map(|&f| CFace {
                pool: Some(f),
                cap_on: None,
                normal: pool[f].normal,
                rings: pool[f].rings.iter().map(|r| refine(r)).collect(),
            })
            .collect();
        let edge_map = |faces: &[CFace], active: &[bool]| {
            let mut m: BTreeMap<(usize, usize), Vec<(usize, bool)>> = BTreeMap::new();
            for (fi, f) in faces.iter().enumerate() {
                if !active[fi] {
                    continue;
                }
                for ring in &f.rings {
                    for k in 0..ring.len() {
                        let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                        m.entry(ekey(a, b)).or_default().push((fi, a < b));
                    }
                }
            }
            m
        };
        // ---- rims and prune ----
        let mut active = vec![true; faces.len()];
        // The first round's open edges on no cappable rim, and its
        // non-planar loops: what a decline reports.
        let mut first_round: Option<FirstRound> = None;
        let caps: Vec<(Vec<usize>, CapOn)> = loop {
            let edges = edge_map(&faces, &active);
            let open: Vec<(usize, usize)> = edges
                .iter()
                .filter(|(_, v)| v.len() == 1)
                .map(|(&k, _)| k)
                .collect();
            let rims = chain_rims(&open);
            let mut cappable: Vec<(Vec<usize>, CapOn)> = Vec::new();
            let mut bad: BTreeSet<(usize, usize)> = BTreeSet::new();
            let mut non_planar: Vec<Vec<usize>> = Vec::new();
            for rim in &rims {
                let cap = rim.cycle.as_ref().and_then(|cycle| {
                    let corners: Vec<Point3> = cycle.iter().map(|&v| pts[v]).collect();
                    if !on_one_plane(&corners) {
                        non_planar.push(cycle.clone());
                        return None;
                    }
                    if coplanar {
                        // Faces on one plane bound no volume, capped or
                        // not (a cap lies on that plane too).
                        return None;
                    }
                    self.rim_cap_on(&corners, scope)
                        .map(|on| (cycle.clone(), on))
                });
                match cap {
                    Some(c) => cappable.push(c),
                    None => bad.extend(rim.edges.iter().copied()),
                }
            }
            if first_round.is_none() {
                first_round = Some((bad.iter().copied().collect(), non_planar));
            }
            let mut removed = false;
            for &e in &bad {
                for &(fi, _) in &edges[&e] {
                    if active[fi] {
                        active[fi] = false;
                        removed = true;
                    }
                }
            }
            if !removed {
                break cappable;
            }
        };
        let decline = |reason: CellDeclineReason| {
            let (open, loops) = first_round.clone().unwrap_or_default();
            CellDecline {
                reason,
                open_edges: open.iter().map(|&(a, b)| (pts[a], pts[b])).collect(),
                non_planar_loops: loops
                    .iter()
                    .map(|c| c.iter().map(|&v| pts[v]).collect())
                    .collect(),
                overlapping_edges: Vec::new(),
            }
        };
        if !active.iter().any(|&a| a) {
            if report_unclosed {
                plan.declines.push(decline(CellDeclineReason::NotClosed));
            }
            return;
        }
        // ---- caps ----
        for cap in cap_faces(pts, &caps) {
            faces.push(cap);
            active.push(true);
        }
        // ---- cells: the wedges at every edge join face sides ----
        let edges = edge_map(&faces, &active);
        let mut classes = Classes((0..2 * faces.len()).collect());
        // Faces lying over each other along an edge: their order there is
        // arbitrary, so no cell they bound is built and nothing beside
        // them is consumed; cells elsewhere are unaffected.
        let mut tied: BTreeSet<usize> = BTreeSet::new();
        // The edges where they do (§16 R22: what Make Solid marks).
        let mut tied_edges: Vec<(Point3, Point3)> = Vec::new();
        for (&(a, b), around) in &edges {
            if around.len() < 2 {
                // Cannot happen once pruned and capped; no cell then.
                if report_unclosed {
                    plan.declines.push(decline(CellDeclineReason::NotClosed));
                }
                return;
            }
            let d = pts[b] - pts[a];
            let d = d * (1.0 / d.length());
            let v = perpendicular(d);
            let w = d.cross(v);
            let mut at: Vec<(f64, Vec3, usize, bool)> = around
                .iter()
                .map(|&(fi, fwd)| {
                    let t = if fwd { d } else { d * -1.0 };
                    let u = faces[fi].normal.cross(t);
                    let u = u * (1.0 / u.length());
                    (u.dot(w).atan2(u.dot(v)), u, fi, fwd)
                })
                .collect();
            at.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.2.cmp(&y.2)));
            let n = at.len();
            for k in 0..n {
                let (_, u0, f0, fwd0) = at[k];
                let (_, u1, f1, fwd1) = at[(k + 1) % n];
                if u0.dot(u1) > 0.0 && u0.cross(u1).dot(d).abs() <= tol::FACE_MEETING_PARALLEL_SINE
                {
                    tied.extend([f0, f1]);
                    if tied_edges.last() != Some(&(pts[a], pts[b])) {
                        tied_edges.push((pts[a], pts[b]));
                    }
                }
                // The side of f0 facing increasing angle, and that of f1
                // facing decreasing angle (side index 2f + 1 is the side
                // the normal points into).
                classes.join(2 * f0 + usize::from(fwd0), 2 * f1 + usize::from(!fwd1));
            }
        }
        let mut cells: BTreeMap<usize, Vec<(usize, bool)>> = BTreeMap::new();
        for (fi, _) in faces.iter().enumerate().filter(|&(fi, _)| active[fi]) {
            for plus in [false, true] {
                let c = classes.find(2 * fi + usize::from(plus));
                cells.entry(c).or_default().push((fi, plus));
            }
        }
        // ---- bounded cells a seed face bounds ----
        let origin = pts[faces[0].rings[0][0]];
        let mut status: BTreeMap<usize, Option<bool>> = BTreeMap::new(); // None: exterior
        let mut found = false;
        let mut overlap_reported = false;
        let mut built: Vec<(usize, Planned)> = Vec::new();
        for (&c, sides) in &cells {
            if sides.iter().any(|(fi, _)| tied.contains(fi)) {
                // Neither built nor taken for the outside.
                status.insert(c, Some(false));
                let seeded = sides
                    .iter()
                    .any(|&(fi, _)| faces[fi].pool.is_some_and(|p| is_seed[p]));
                if seeded && !overlap_reported {
                    overlap_reported = true;
                    found = true;
                    let mut d = decline(CellDeclineReason::OverlappingFaces);
                    d.overlapping_edges = tied_edges.clone();
                    plan.declines.push(d);
                }
                continue;
            }
            let mut six_v = 0.0;
            for &(fi, plus) in sides {
                for ring in &faces[fi].rings {
                    let o: Vec<Vec3> = oriented(ring, plus)
                        .iter()
                        .map(|&v| pts[v] - origin)
                        .collect();
                    for k in 1..o.len().saturating_sub(1) {
                        six_v += o[0].dot(o[k].cross(o[k + 1]));
                    }
                }
            }
            if six_v <= 0.0 {
                status.insert(c, None);
                continue;
            }
            status.insert(c, Some(false));
            let seeded = sides
                .iter()
                .any(|&(fi, _)| faces[fi].pool.is_some_and(|p| is_seed[p]));
            if !seeded {
                continue;
            }
            found = true;
            if sides.iter().any(|&(fi, plus)| sides.contains(&(fi, !plus))) {
                plan.declines.push(decline(CellDeclineReason::SelfTouching));
                continue;
            }
            // Inside the Object a cap is on: nothing (§9 D3). A cap faces out
            // of its Object, and the side facing INTO the cell is `plus`.
            if let Some(o) = sides
                .iter()
                .find(|&&(fi, plus)| faces[fi].cap_on.is_some() && !plus)
                .and_then(|&(fi, _)| faces[fi].cap_on)
            {
                plan.declines
                    .push(decline(CellDeclineReason::InsideSolid(o)));
                continue;
            }
            let solid = match build_cell(pts, &faces, sides) {
                Ok(s) => s,
                Err(reason) => {
                    plan.declines.push(decline(reason));
                    continue;
                }
            };
            let on: BTreeSet<ObjectId> = sides
                .iter()
                .filter_map(|&(fi, _)| faces[fi].cap_on)
                .collect();
            let detached = sides
                .iter()
                .any(|&(fi, _)| faces[fi].pool.is_some_and(|p| pool[p].detached));
            let commit = match on.iter().next() {
                Some(&o) if on.len() == 1 && !detached && self.cell_growable(o, scope) => {
                    Commit::Grow(o)
                }
                _ => Commit::Create,
            };
            status.insert(c, Some(true));
            let mut edges: [Vec<(Point3, Point3)>; 2] = [Vec::new(), Vec::new()];
            for &(fi, _) in sides {
                let Some(p) = faces[fi].pool else { continue };
                for ring in &faces[fi].rings {
                    for k in 0..ring.len() {
                        let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                        edges[usize::from(pool[p].detached)].push((pts[a], pts[b]));
                    }
                }
            }
            built.push((
                c,
                Planned {
                    solid,
                    commit,
                    edges,
                },
            ));
        }
        if !found {
            if report_unclosed {
                plan.declines.push(decline(CellDeclineReason::NotClosed));
            }
            return;
        }
        // A drawn face with the outside on both sides: a zero-thickness lid,
        // such as a plug over a tube whose inner rim is capped only as a
        // hole, sealing a pocket of air against the solid. Committing the
        // cells beside it would leave that membrane over a sealed cavity, so
        // this gathered set (one connected component; other components of
        // the same search are planned on their own and unaffected) declines,
        // typed (both sides of one face bound the outside), and nothing of
        // it changes. Building the pocket as its own cell is deliberately
        // not tried: a wrong solid is worse than no solid. Precondition: an
        // active face has every edge shared, since the prune and caps left
        // no open edge (an edge with one face returned NotClosed above), so
        // the face is no flap but a wall the outside wraps around.
        if !built.is_empty() {
            let membrane = (0..faces.len()).find(|&fi| {
                faces[fi].pool.is_some()
                    && active[fi]
                    && status.get(&classes.find(2 * fi)) == Some(&None)
                    && status.get(&classes.find(2 * fi + 1)) == Some(&None)
            });
            if let Some(fi) = membrane {
                debug_assert!(
                    faces[fi].rings.iter().all(|r| (0..r.len()).all(|k| {
                        edges
                            .get(&ekey(r[k], r[(k + 1) % r.len()]))
                            .is_some_and(|around| around.len() >= 2)
                    })),
                    "a membrane face has an open edge — kernel bug"
                );
                plan.declines.push(decline(CellDeclineReason::SelfTouching));
                return;
            }
        }
        // ---- consumed faces: each side committed or the outside ----
        let ok = |c: usize| match status.get(&c) {
            Some(None) => Some(false),
            Some(Some(true)) => Some(true),
            _ => None, // a bounded cell left as it is
        };
        for (fi, f) in faces.iter().enumerate() {
            let Some(p) = f.pool else { continue };
            if !active[fi] {
                continue;
            }
            let (a, b) = (ok(classes.find(2 * fi)), ok(classes.find(2 * fi + 1)));
            if let (Some(a), Some(b)) = (a, b)
                && (a || b)
            {
                plan.consumed
                    .entry(pool[p].sketch)
                    .or_default()
                    .insert(pool[p].region);
            }
        }
        plan.cells.extend(built.into_iter().map(|(_, p)| p));
    }

    /// The Object rim `corners` (closed, planar) can be capped on, if any
    /// (§5.2 step 2): a visible Object of the frame whose faces on the
    /// rim's plane (either facing) hold every corner and edge midpoint of
    /// the rim and a point inside it.
    fn rim_cap_on(&self, corners: &[Point3], scope: &CellScope) -> Option<CapOn> {
        let plane = Plane::from_polygon(corners).ok()?;
        let inside = interior_point_of_loops(corners, &[], plane.normal())?;
        let n = corners.len();
        let mut lo = corners[0];
        let mut hi = corners[0];
        for p in corners {
            lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        for (id, rec) in self.objects.iter() {
            if !self.cell_visible(id, scope) {
                continue;
            }
            let obj = &rec.object;
            let regions: Vec<(Vec3, Vec<Point3>, Vec<Vec<Point3>>)> = obj
                .faces()
                .iter()
                .filter(|&(f, _)| face_on_plane_either(obj, f, &plane))
                .map(|(_, face)| {
                    (
                        face.plane.normal(),
                        obj.loop_positions(face.outer_loop).collect(),
                        face.inner_loops
                            .iter()
                            .map(|&l| obj.loop_positions(l).collect())
                            .collect(),
                    )
                })
                .collect();
            if regions.is_empty() {
                continue;
            }
            // A cheap cut: the faces' box must hold the rim's.
            let m = tol::POINT_MERGE;
            let in_box = regions.iter().flat_map(|r| r.1.iter()).fold(
                (
                    Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
                    Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
                ),
                |(a, b), p| {
                    (
                        Point3::new(a.x.min(p.x), a.y.min(p.y), a.z.min(p.z)),
                        Point3::new(b.x.max(p.x), b.y.max(p.y), b.z.max(p.z)),
                    )
                },
            );
            if lo.x < in_box.0.x - m
                || lo.y < in_box.0.y - m
                || lo.z < in_box.0.z - m
                || hi.x > in_box.1.x + m
                || hi.y > in_box.1.y + m
                || hi.z > in_box.1.z + m
            {
                continue;
            }
            let on_faces = |p: Point3| {
                regions
                    .iter()
                    .any(|(nf, o, h)| crate::geom2d::point_on_region(p, o, h, *nf))
            };
            let rim_on = (0..n).all(|i| {
                let (a, b) = (corners[i], corners[(i + 1) % n]);
                on_faces(a) && on_faces(Point3::ORIGIN + (a.to_vec() + b.to_vec()) * 0.5)
            });
            if !rim_on {
                continue;
            }
            let holder = regions.iter().find(|(nf, o, h)| {
                point_inside_polygon(inside, o, *nf)
                    && !h.iter().any(|hole| point_inside_polygon(inside, hole, *nf))
            });
            if let Some((nf, _, _)) = holder {
                return Some(CapOn {
                    object: id,
                    normal: *nf,
                    plane,
                });
            }
        }
        None
    }

    // ------------------------------------------------------------ commit

    /// Writes `plan` (§5.4), the rows into `out`.
    fn commit_cells(&mut self, plan: Plan, scope: &CellScope, out: &mut CellsPass) {
        let frame = scope.frame;
        let mut object_rows: Vec<DocAction> = Vec::new();
        let mut grown: BTreeMap<ObjectId, ObjectId> = BTreeMap::new();
        // The new solids' edges, by the kind of face holding each:
        // [ordinary, detached] (§16 R14).
        let mut solid_edges: [Vec<(Point3, Point3)>; 2] = [Vec::new(), Vec::new()];
        for Planned {
            solid,
            commit,
            edges: [ordinary, detached],
        } in plan.cells
        {
            solid_edges[0].extend(ordinary);
            solid_edges[1].extend(detached);
            let target = match commit {
                Commit::Grow(o) => {
                    let mut o = o;
                    while let Some(&next) = grown.get(&o) {
                        o = next;
                    }
                    Some(o)
                }
                Commit::Create => None,
            };
            let union = target.and_then(|o| {
                let base = &self.objects[o].object;
                let mut result =
                    Object::boolean(BooleanOp::Union, base, &solid, &Transform::IDENTITY).ok()?;
                result.merge_coplanar_faces(&base.coplanar_edge_segments());
                (result.watertight() == WatertightState::Watertight && result.validate().is_ok())
                    .then_some(result)
            });
            match (target, union) {
                (Some(o), Some(result)) => {
                    let (owner, name, tags) = {
                        let rec = &self.objects[o];
                        (rec.owner, rec.name.clone(), rec.tags.clone())
                    };
                    let id = self.insert_object_record(ObjectRecord {
                        object: result,
                        history: History::new(),
                        hidden: false,
                        owner,
                        name,
                        tags,
                    });
                    // The grown solid takes the old one's place: in a group
                    // it replaces it in the member list (never reached while
                    // only top-level Objects take part, kept so the tree
                    // holds whatever grows), else it joins its owner.
                    if self.objects[o].group_parent().is_some() {
                        self.swap_group_member(o, id);
                    } else {
                        self.join_owner(id, owner, out);
                    }
                    self.objects[o].hidden = true;
                    let reanchored = self.reevaluate_liveness_recorded(&[NodeId::Object(o)]);
                    let carried = self.carry_owned_sketches(&[o], &[id]);
                    out.sketches
                        .extend(carried.iter().flat_map(DocAction::touched_sketches));
                    // The copies the grown solid holds as its own have no
                    // row (they ride with it), but the change names them.
                    out.sketches.extend(self.object_owned_sketches(id));
                    object_rows.push(DocAction::GrewObjectFromFaces {
                        from: o,
                        to: id,
                        reanchored,
                    });
                    object_rows.extend(carried);
                    grown.insert(o, id);
                    out.objects.extend([o, id]);
                    out.outcomes.push(CellOutcome::Grew { from: o, to: id });
                }
                (target, _) => {
                    let owner = match frame {
                        Some(def) => ObjectOwner::Definition { def, parent: None },
                        None => ObjectOwner::World { parent: None },
                    };
                    let id = self.insert_object_record(ObjectRecord {
                        object: solid,
                        history: History::new(),
                        hidden: false,
                        owner,
                        name: None,
                        tags: Vec::new(),
                    });
                    self.join_owner(id, owner, out);
                    object_rows.push(DocAction::CreatedObjectFromFaces { id });
                    out.objects.push(id);
                    out.outcomes.push(match target {
                        Some(with) => CellOutcome::Unjoined { object: id, with },
                        None => CellOutcome::Created(id),
                    });
                }
            }
        }

        // ---- the faces' sketches: scaffolding, then loose lines ----
        let mut next: BTreeMap<SketchId, Sketch> = BTreeMap::new();
        for (s, regions) in &plan.consumed {
            let mut sk = self.sketches[*s].clone();
            let gone = sk.regions_scaffolding(regions);
            sk.remove_edges(&gone);
            next.insert(*s, sk);
        }
        // Loose lines go only along edges a face of their own kind holds
        // (§16 R14): an ordinary cell trims ordinary lines, a detached cell
        // detached ones, a mixed cell each kind along its own faces.
        let candidates: Vec<SketchId> = self
            .sketches
            .keys()
            .filter(|&s| self.cell_sketch(s, frame))
            .collect();
        for s in candidates {
            let cur = next.get(&s).unwrap_or(&self.sketches[s]);
            let along = &solid_edges[usize::from(cur.detached())];
            if let Some(trimmed) = trim_loose_along(cur, along) {
                next.insert(s, trimmed);
            }
        }
        let mut sketch_rows: Vec<DocAction> = Vec::new();
        for (s, after) in next {
            let before = std::mem::replace(&mut self.sketches[s], after);
            sketch_rows.push(DocAction::SketchGesture {
                sketch: s,
                before: Box::new(before),
                after: Box::new(self.sketches[s].clone()),
                created: false,
            });
            if self.sketches[s].edges().is_empty() {
                self.hidden_sketches.insert(s);
                sketch_rows.push(DocAction::DeletedSketch { sketch: s });
            }
            out.sketches.push(s);
        }
        out.actions.extend(sketch_rows);
        out.actions.extend(object_rows);
        if let Some(def) = frame {
            out.components.push(def);
            out.instances = self.placing_instances(def);
        }
    }

    /// Puts `to` where `from` stands in the member list of `from`'s group
    /// (a grow's commit, and its redo; its undo swaps back). Nothing when
    /// `from` is in no group or its group does not list it.
    pub(super) fn swap_group_member(&mut self, from: ObjectId, to: ObjectId) {
        let Some(g) = self.objects.get(from).and_then(|r| r.group_parent()) else {
            return;
        };
        if let Some(rec) = self.groups.get_mut(g)
            && let Some(slot) = rec.members.iter_mut().find(|m| **m == NodeId::Object(from))
        {
            *slot = NodeId::Object(to);
        }
    }

    /// Lists a new Object with its owner: a group's member list, or the
    /// definition's.
    fn join_owner(&mut self, id: ObjectId, owner: ObjectOwner, out: &mut CellsPass) {
        match owner {
            ObjectOwner::World { parent: Some(g) }
            | ObjectOwner::Definition {
                parent: Some(g), ..
            } => {
                self.groups[g].members.push(NodeId::Object(id));
            }
            ObjectOwner::Definition { def, parent: None } => {
                self.components[def].members.push(NodeId::Object(id));
                out.components.push(def);
            }
            ObjectOwner::World { parent: None } => {}
        }
    }
}

/// `ring` wound for the side `plus` (the side its face's normal points
/// into faces the cell, so the outward winding is reversed).
fn oriented(ring: &[usize], plus: bool) -> Vec<usize> {
    let mut r = ring.to_vec();
    if plus {
        r.reverse();
    }
    r
}

/// Whether `corners` lie on one plane within [`tol::PLANE_DIST`].
fn on_one_plane(corners: &[Point3]) -> bool {
    Plane::from_polygon(corners).is_ok_and(|p| {
        corners
            .iter()
            .all(|&q| p.signed_distance(q).abs() <= tol::PLANE_DIST)
    })
}

/// The open edges `open` as rims: each connected set of them, a cycle when
/// every corner has two.
fn chain_rims(open: &[(usize, usize)]) -> Vec<Rim> {
    let mut adj: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(a, b) in open {
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut rims = Vec::new();
    for &start in adj.keys() {
        if seen.contains(&start) {
            continue;
        }
        let mut stack = vec![start];
        let mut verts = Vec::new();
        seen.insert(start);
        while let Some(v) = stack.pop() {
            verts.push(v);
            for &w in &adj[&v] {
                if seen.insert(w) {
                    stack.push(w);
                }
            }
        }
        let set: BTreeSet<usize> = verts.iter().copied().collect();
        let edges: Vec<(usize, usize)> = open
            .iter()
            .copied()
            .filter(|(a, _)| set.contains(a))
            .collect();
        let simple = verts.len() >= 3 && verts.iter().all(|v| adj[v].len() == 2);
        let cycle = simple.then(|| {
            let mut cyc = vec![start];
            let mut prev = start;
            let mut cur = adj[&start][0];
            while cur != start {
                cyc.push(cur);
                let n = &adj[&cur];
                let nxt = if n[0] == prev { n[1] } else { n[0] };
                prev = cur;
                cur = nxt;
            }
            cyc
        });
        let cycle = cycle.filter(|c| c.len() == verts.len());
        rims.push(Rim { cycle, edges });
    }
    rims
}

/// The caps for cappable rims (§5.2 step 2): per Object and plane, each rim
/// not inside another is a cap's outer ring, and each rim directly inside
/// one is a hole of it; rings wound about the Object's outward face normal.
fn cap_faces(pts: &[Point3], caps: &[(Vec<usize>, CapOn)]) -> Vec<CFace> {
    let mut out = Vec::new();
    let ring_pts = |r: &[usize]| r.iter().map(|&v| pts[v]).collect::<Vec<_>>();
    // Nesting depth among rims on the same Object and plane.
    let contains = |outer: &(Vec<usize>, CapOn), inner: &(Vec<usize>, CapOn)| {
        outer.1.object == inner.1.object
            && inner
                .0
                .iter()
                .all(|&v| outer.1.plane.signed_distance(pts[v]).abs() <= tol::PLANE_DIST)
            && {
                let poly = ring_pts(&outer.0);
                let n = outer.1.plane.normal();
                inner.0.iter().any(|&v| {
                    !crate::geom2d::point_on_ring(pts[v], &poly)
                        && point_inside_polygon(pts[v], &poly, n)
                })
            }
    };
    let depth: Vec<usize> = (0..caps.len())
        .map(|i| {
            (0..caps.len())
                .filter(|&j| j != i && contains(&caps[j], &caps[i]))
                .count()
        })
        .collect();
    for (i, (ring, on)) in caps.iter().enumerate() {
        if depth[i] % 2 == 1 {
            continue;
        }
        let mut outer = ring.clone();
        if ring_area(pts, &outer, on.normal) < 0.0 {
            outer.reverse();
        }
        let mut rings = vec![outer];
        for (j, (hole, _)) in caps.iter().enumerate() {
            if depth[j] == depth[i] + 1 && contains(&caps[i], &caps[j]) {
                let mut h = hole.clone();
                if ring_area(pts, &h, on.normal) > 0.0 {
                    h.reverse();
                }
                rings.push(h);
            }
        }
        out.push(CFace {
            pool: None,
            cap_on: Some(on.object),
            normal: on.normal,
            rings,
        });
    }
    out
}

/// Builds one cell (§5.3): its faces wound outward, through the strict
/// builder with holes; it must be watertight and valid.
fn build_cell(
    pts: &[Point3],
    faces: &[CFace],
    sides: &[(usize, bool)],
) -> Result<Object, CellDeclineReason> {
    let mut index: BTreeMap<usize, usize> = BTreeMap::new();
    let mut positions: Vec<Point3> = Vec::new();
    let mut map = |v: usize| -> usize {
        *index.entry(v).or_insert_with(|| {
            positions.push(pts[v]);
            positions.len() - 1
        })
    };
    let mut outers: Vec<Vec<usize>> = Vec::new();
    let mut holes: Vec<Vec<Vec<usize>>> = Vec::new();
    for &(fi, plus) in sides {
        let f = &faces[fi];
        let mut rings = f.rings.iter().map(|r| oriented(r, plus));
        let outer = rings.next().expect("a face has an outer ring");
        outers.push(outer.into_iter().map(&mut map).collect());
        holes.push(
            rings
                .map(|r| r.into_iter().map(&mut map).collect())
                .collect(),
        );
    }
    let obj = Object::from_polygons_with_holes(&positions, &outers, &holes)
        .map_err(CellDeclineReason::Invalid)?;
    if obj.watertight() != WatertightState::Watertight {
        return Err(CellDeclineReason::NotClosed);
    }
    obj.validate().map_err(CellDeclineReason::Invalid)?;
    Ok(obj)
}

/// `sk` without the parts of its loose edges (on no region's ring) lying
/// along one of `along_edges`; `None` when nothing goes, or when the
/// remainder cannot be redrawn (the sketch is then left whole).
fn trim_loose_along(sk: &Sketch, along_edges: &[(Point3, Point3)]) -> Option<Sketch> {
    if along_edges.is_empty() {
        return None;
    }
    let pos = |v| sk.vertices()[v].position;
    let mut ring_pairs = BTreeSet::new();
    for r in sk.regions().values() {
        for ring in std::iter::once(&r.outer).chain(r.holes.iter()) {
            for k in 0..ring.len() {
                let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                ring_pairs.insert(if a < b { (a, b) } else { (b, a) });
            }
        }
    }
    let bounding: BTreeSet<SketchEdgeId> = sk
        .edges()
        .iter()
        .filter(|(_, e)| {
            let (a, b) = (e.from, e.to);
            ring_pairs.contains(&if a < b { (a, b) } else { (b, a) })
        })
        .map(|(id, _)| id)
        .collect();
    let m = tol::POINT_MERGE;
    let mut gone: BTreeSet<SketchEdgeId> = BTreeSet::new();
    let mut rest: Vec<(Point3, Point3)> = Vec::new();
    for (eid, e) in sk.edges().iter() {
        if bounding.contains(&eid) {
            continue;
        }
        let (a, b) = (pos(e.from), pos(e.to));
        let cover: Vec<(Point3, Point3)> = along_edges
            .iter()
            .copied()
            .filter(|&(c, d)| {
                c.x.min(d.x) <= a.x.max(b.x) + m
                    && c.x.max(d.x) >= a.x.min(b.x) - m
                    && c.y.min(d.y) <= a.y.max(b.y) + m
                    && c.y.max(d.y) >= a.y.min(b.y) - m
                    && c.z.min(d.z) <= a.z.max(b.z) + m
                    && c.z.max(d.z) >= a.z.min(b.z) - m
            })
            .collect();
        if cover.is_empty() {
            continue;
        }
        let parts = uncovered_parts(a, b, cover);
        if parts.len() == 1 && parts[0] == (a, b) {
            continue;
        }
        gone.insert(eid);
        rest.extend(parts);
    }
    if gone.is_empty() {
        return None;
    }
    let mut next = sk.clone();
    next.remove_edges(&gone);
    for (a, b) in rest {
        if next.add_segment(a, b).is_err() {
            return None;
        }
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box from `z0` to `z0 + 1` over the unit square, wound outward.
    fn block(z0: f64) -> Object {
        let z1 = z0 + 1.0;
        let positions = vec![
            Point3::new(0., 0., z0),
            Point3::new(1., 0., z0),
            Point3::new(1., 1., z0),
            Point3::new(0., 1., z0),
            Point3::new(0., 0., z1),
            Point3::new(1., 0., z1),
            Point3::new(1., 1., z1),
            Point3::new(0., 1., z1),
        ];
        let outers = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let holes = vec![Vec::new(); outers.len()];
        Object::from_polygons_with_holes(&positions, &outers, &holes).unwrap()
    }

    /// The grow commit's defensive path, set up directly (no drawing reaches
    /// it while only top-level Objects take part): a grouped Object grows,
    /// the new one takes its place in the group with its owner (parent
    /// included), the tree stays whole, and undo and redo swap the member
    /// back and forth.
    #[test]
    fn a_grouped_objects_grow_takes_its_place_in_the_group() {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0., 0., 1.)).unwrap();
        let (r, _) = doc
            .draft(ground, None, &[], "Rectangle", |w| {
                let q = [
                    Point3::new(0., 0., 0.),
                    Point3::new(1., 0., 0.),
                    Point3::new(1., 1., 0.),
                    Point3::new(0., 1., 0.),
                ];
                for k in 0..4 {
                    w.add_segment(q[k], q[(k + 1) % 4])?;
                }
                Ok(())
            })
            .unwrap();
        let s = r.sketches[0];
        let region = doc.extrudable_regions(s).unwrap()[0];
        let bx = doc.extrude_region(s, region, 1.0).unwrap().0;
        let (g, _) = doc.group_nodes(&[NodeId::Object(bx)]).unwrap();
        let before = doc.state_hash();

        let plan = Plan {
            cells: vec![Planned {
                solid: block(1.0),
                commit: Commit::Grow(bx),
                edges: [Vec::new(), Vec::new()],
            }],
            ..Plan::default()
        };
        let scope = doc.auto_cell_scope(None);
        let mut out = CellsPass::default();
        doc.commit_cells(plan, &scope, &mut out);
        let [CellOutcome::Grew { from, to }] = out.outcomes[..] else {
            panic!("{:?}", out.outcomes)
        };
        assert_eq!(from, bx);
        doc.record_cells(out.actions, &out.outcomes);
        assert_eq!(doc.objects[to].owner, doc.objects[bx].owner, "owner kept");
        assert_eq!(doc.node_parent(NodeId::Object(to)), Some(g));
        assert_eq!(doc.group_members(g).unwrap(), vec![NodeId::Object(to)]);
        let after = doc.state_hash();

        doc.undo().unwrap();
        assert_eq!(doc.group_members(g).unwrap(), vec![NodeId::Object(bx)]);
        assert_eq!(doc.state_hash(), before);
        doc.redo().unwrap();
        assert_eq!(doc.group_members(g).unwrap(), vec![NodeId::Object(to)]);
        assert_eq!(doc.state_hash(), after);
    }
}
