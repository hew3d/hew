//! Shared fixtures and invariant checks for the face-arrangement specs
//! (`face_arrangement_specs.rs`, `face_arrangement_stage2_specs.rs`,
//! `face_arrangement_stage3_specs.rs`): a solid's face framed in 2-D, the
//! drawing, cutting, and merging helpers, and the I1–I7 checks every
//! accepting spec asserts.

#![allow(dead_code)]

use std::f64::consts::TAU;

use kernel::{
    ComponentId, CurveGeom, DocChange, Document, DocumentError, DraftReport, DraftUnit, EdgeId,
    FaceId, FaceSplitReport, KernelOp, KernelOpError, KernelOpReport, Object, ObjectId, Plane,
    PlaneUnits, Point3, Sketch, SketchError, StickyError, Transform, Vec3, tol,
};

pub const MM: f64 = 1e-3;

// ---------------------------------------------------------------- setup

pub fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane")
}

/// The in-plane basis the API's face-mode drawing uses (`ring_points` /
/// `plane_basis` in `crates/api/src/commands/sketch.rs`): `(u, v, n)` is
/// right-handed, so a loop CCW in `(u, v)` winds CCW seen from the face.
pub fn plane_basis(plane: &Plane) -> (Vec3, Vec3) {
    let n = plane.normal();
    let reference = if n.x.abs() > 0.9 {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = (reference - n * reference.dot(n))
        .normalized()
        .expect("reference not parallel to the normal");
    (u, n.cross(u))
}

/// A face of a solid to draw on, with a 2-D frame on its plane: `at(x, y)`
/// is the point `x` along `u` and `y` along `v` from the face's
/// `(u, v)`-minimum corner.
pub struct Wall {
    pub doc: Document,
    pub id: ObjectId,
    pub face: FaceId,
    pub plane: Plane,
    pub origin: Point3,
    pub u: Vec3,
    pub v: Vec3,
}

impl Wall {
    pub fn at(&self, x: f64, y: f64) -> Point3 {
        self.origin + self.u * x + self.v * y
    }

    pub fn obj(&self) -> &Object {
        self.doc.object(self.id).expect("object")
    }

    /// Draws the closed loop on `seed`'s plane in one draft.
    pub fn draw_loop(
        &mut self,
        seed: FaceId,
        loop_path: &[Point3],
        curve: Option<CurveGeom>,
    ) -> Result<(DraftReport, DocChange), DocumentError> {
        draw_loop(&mut self.doc, None, self.id, seed, loop_path, curve)
    }

    /// A rectangle `w × h` with its `(u, v)`-minimum corner at `(x, y)`,
    /// CCW seen from the face.
    pub fn rect(&self, x: f64, y: f64, w: f64, h: f64) -> Vec<Point3> {
        vec![
            self.at(x, y),
            self.at(x + w, y),
            self.at(x + w, y + h),
            self.at(x, y + h),
        ]
    }

    /// `n` points on the circle about `(cx, cy)` of radius `r`, vertex `i`
    /// at angle `phase + TAU·i/n` — the API's `ring_points` construction
    /// (phase 0 puts vertex 0 on `+u`).
    pub fn ring(&self, cx: f64, cy: f64, r: f64, n: usize, phase: f64) -> Vec<Point3> {
        let c = self.at(cx, cy);
        (0..n)
            .map(|i| {
                let a = phase + TAU * i as f64 / n as f64;
                c + self.u * (r * a.cos()) + self.v * (r * a.sin())
            })
            .collect()
    }

    pub fn circle_geom(&self, cx: f64, cy: f64, r: f64) -> CurveGeom {
        CurveGeom {
            center: self.at(cx, cy),
            radius: r,
        }
    }
}

/// Extrudes a `w × d` rectangle on `plane` (drawn in that plane's API
/// basis) by `h` through the document, and returns the extrusion's face
/// whose outward normal is `face_normal` (expressed as a closure over the
/// sketch plane's normal), framed at its `(u, v)`-minimum corner.
pub fn wall_of_box(plane: Plane, w: f64, d: f64, h: f64, pick: impl Fn(Vec3) -> Vec3) -> Wall {
    let mut doc = Document::new();
    let s = doc.add_sketch(plane);
    let (pu, pv) = plane_basis(&plane);
    let o = plane.point();
    let corners = [o, o + pu * w, o + pu * w + pv * d, o + pv * d];
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..4 {
            sk.add_segment(corners[i], corners[(i + 1) % 4])
                .expect("segment");
        }
    }
    let regions = doc.extrudable_regions(s).expect("regions");
    assert_eq!(regions.len(), 1);
    let (id, _) = doc.extrude_region(s, regions[0], h).expect("extrude");
    let want = pick(plane.normal());
    let obj = doc.object(id).expect("object");
    let face = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(want, 1e-9))
        .map(|(id, _)| id)
        .expect("a face with the requested normal");
    let fplane = obj.faces()[face].plane;
    let (u, v) = plane_basis(&fplane);
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
    let p0 = ring[0];
    let umin = ring
        .iter()
        .map(|p| (*p - p0).dot(u))
        .fold(f64::INFINITY, f64::min);
    let vmin = ring
        .iter()
        .map(|p| (*p - p0).dot(v))
        .fold(f64::INFINITY, f64::min);
    let origin = p0 + u * umin + v * vmin;
    Wall {
        doc,
        id,
        face,
        plane: fplane,
        origin,
        u,
        v,
    }
}

/// A 20 × 10 × 10 mm box on the ground; the wall is its y = 0 side
/// (outward normal −y; the API basis there is u = +x, v = +z).
pub fn front_wall() -> Wall {
    wall_of_box(ground(), 20.0 * MM, 10.0 * MM, 10.0 * MM, |_| {
        Vec3::new(0.0, -1.0, 0.0)
    })
}

/// A box extruded from a sketch on a plane tilted about two axes, drawing
/// on its far cap (outward normal = the tilted plane's normal) — no axis
/// in the basis is exact, so every point carries rounding.
pub fn tilted_wall() -> Wall {
    let plane = Plane::from_polygon(&[
        Point3::new(0.01, 0.02, 0.003),
        Point3::new(1.01, 0.02, 0.303),
        Point3::new(0.01, 1.02, 0.203),
    ])
    .expect("tilted plane");
    wall_of_box(plane, 20.0 * MM, 10.0 * MM, 10.0 * MM, |n| n)
}

/// A 30 × 30 × 10 mm box on the ground, drawing on its top (+z).
pub fn top_of_box() -> Wall {
    wall_of_box(ground(), 30.0 * MM, 30.0 * MM, 10.0 * MM, |_| {
        Vec3::new(0.0, 0.0, 1.0)
    })
}

/// The maintainer's rectangle: 9.4 × 3.8 mm with its corner 3 mm along
/// and 2 mm up the wall. Returns the imprinted sub-face.
pub const RX: f64 = 3.0 * MM;
pub const RY: f64 = 2.0 * MM;
pub const RW: f64 = 9.4 * MM;
pub const RH: f64 = 3.8 * MM;
pub const R: f64 = 1.9 * MM;

pub fn imprint_rect(w: &mut Wall) -> FaceId {
    let rect = w.rect(RX, RY, RW, RH);
    let seed = w.face;
    draw_region(w, seed, rect, None)
}

/// The face of the wall's group holding the point `(x, y)`.
pub fn face_at(w: &Wall, x: f64, y: f64) -> FaceId {
    let p = w.at(x, y);
    group_faces(w.obj(), &w.plane)
        .into_iter()
        .find(|&f| w.obj().face_contains_point(f, p))
        .expect("a group face holds the point")
}

pub fn sticky(e: DocumentError) -> StickyError {
    match e {
        DocumentError::Op(KernelOpError::Sticky(s)) => s,
        other => panic!("expected a sticky refusal, got {other:?}"),
    }
}

// ------------------------------------------------------------ invariants

/// Faces of `obj` on `plane`, facing the same way — the drawn-on group.
pub fn group_faces(obj: &Object, plane: &Plane) -> Vec<FaceId> {
    obj.faces()
        .iter()
        .filter(|(_, f)| {
            (f.plane.normal() - plane.normal()).length() < 1e-9
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| plane.signed_distance(p).abs() < 1e-9)
        })
        .map(|(id, _)| id)
        .collect()
}

/// `V − E + F − H`, the Euler–Poincaré count for faces with holes. A hole
/// that touches the face's outer loop (or a chain of holes touching it) at
/// shared vertices does not make the face an annulus: its loops are one
/// boundary, so `H` counts, per face, the groups of loops joined through
/// shared vertices, less the group holding the outer loop.
pub fn euler(obj: &Object) -> i64 {
    let mut holes = 0i64;
    for face in obj.faces().values() {
        let loops: Vec<kernel::LoopId> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .collect();
        let verts: Vec<Vec<kernel::VertexId>> = loops
            .iter()
            .map(|&l| {
                obj.loop_half_edges(l)
                    .map(|h| obj.half_edges()[h].origin)
                    .collect()
            })
            .collect();
        let mut root: Vec<usize> = (0..loops.len()).collect();
        fn find(root: &mut [usize], mut i: usize) -> usize {
            while root[i] != i {
                i = root[i];
            }
            i
        }
        for i in 0..loops.len() {
            for j in i + 1..loops.len() {
                if verts[i].iter().any(|v| verts[j].contains(v)) {
                    let (a, b) = (find(&mut root, i), find(&mut root, j));
                    root[a] = b;
                }
            }
        }
        let groups = (0..loops.len())
            .filter(|&i| find(&mut root, i) == i)
            .count();
        holes += groups as i64 - 1;
    }
    obj.vertices().len() as i64 - obj.edges().len() as i64 + obj.faces().len() as i64 - holes
}

/// Whether some group face has two loops sharing a vertex (a hole touching
/// the outer loop or another hole at a point).
pub fn has_touching_loops(obj: &Object, plane: &Plane) -> bool {
    group_faces(obj, plane).into_iter().any(|f| {
        let face = &obj.faces()[f];
        let loops: Vec<Vec<kernel::VertexId>> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .map(|l| {
                obj.loop_half_edges(l)
                    .map(|h| obj.half_edges()[h].origin)
                    .collect()
            })
            .collect();
        (0..loops.len())
            .any(|i| (i + 1..loops.len()).any(|j| loops[i].iter().any(|v| loops[j].contains(v))))
    })
}

pub fn signed_area(ring: &[Point3], n: Vec3) -> f64 {
    let mut acc = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..ring.len() {
        let a = ring[i].to_vec();
        let b = ring[(i + 1) % ring.len()].to_vec();
        acc = acc + a.cross(b);
    }
    0.5 * acc.dot(n)
}

/// Net area (outer minus holes) of every group face, summed.
pub fn group_area(obj: &Object, plane: &Plane) -> f64 {
    let n = plane.normal();
    group_faces(obj, plane)
        .into_iter()
        .map(|f| {
            let face = &obj.faces()[f];
            std::iter::once(face.outer_loop)
                .chain(face.inner_loops.iter().copied())
                .map(|l| signed_area(&obj.loop_positions(l).collect::<Vec<_>>(), n))
                .sum::<f64>()
        })
        .sum()
}

/// No loop of a group face visits a vertex twice.
pub fn no_pinch(obj: &Object, plane: &Plane) -> bool {
    group_faces(obj, plane).into_iter().all(|f| {
        let face = &obj.faces()[f];
        std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .all(|l| {
                let vs: Vec<_> = obj
                    .loop_half_edges(l)
                    .map(|h| obj.half_edges()[h].origin)
                    .collect();
                (0..vs.len()).all(|i| !vs[i + 1..].contains(&vs[i]))
            })
    })
}

pub fn cyclic_match(a: &[Point3], b: &[Point3], either_way: bool) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let n = a.len();
    let fwd = |s: usize| (0..n).all(|i| a[i].approx_eq(b[(i + s) % n], tol::POINT_MERGE));
    let rev = |s: usize| (0..n).all(|i| a[i].approx_eq(b[(s + n - i) % n], tol::POINT_MERGE));
    (0..n).any(|s| fwd(s) || (either_way && rev(s)))
}

/// A face (or ground region) as rings: outer, then holes.
pub type Rings = (Vec<Point3>, Vec<Vec<Point3>>);

pub fn face_rings(obj: &Object, f: FaceId) -> Rings {
    let face = &obj.faces()[f];
    (
        obj.loop_positions(face.outer_loop).collect(),
        face.inner_loops
            .iter()
            .map(|&l| obj.loop_positions(l).collect())
            .collect(),
    )
}

/// A ring without its straight-through vertices (a vertex whose two edges
/// run on one line): the ground keeps the vertex where a deleted line met a
/// straight edge, or where a drawn line running along an edge ended, while
/// the solid heals it away or never inserts it, and both describe the same
/// region.
pub fn without_straight(ring: &[Point3], n: Vec3) -> Vec<Point3> {
    let k = ring.len();
    (0..k)
        .filter(|&i| {
            let (a, b, c) = (ring[(i + k - 1) % k], ring[i], ring[(i + 1) % k]);
            ((b - a).cross(c - b)).dot(n).abs() > 1e-9 * (b - a).length() * (c - b).length()
        })
        .map(|i| ring[i])
        .collect()
}

/// Every ring of `rings` [`without_straight`].
pub fn straightened(rings: &[Rings], n: Vec3) -> Vec<Rings> {
    rings
        .iter()
        .map(|(o, hs)| {
            (
                without_straight(o, n),
                hs.iter().map(|h| without_straight(h, n)).collect(),
            )
        })
        .collect()
}

pub fn rings_match(a: &Rings, b: &Rings, either_way: bool) -> bool {
    if !cyclic_match(&a.0, &b.0, either_way) || a.1.len() != b.1.len() {
        return false;
    }
    let mut rest: Vec<&Vec<Point3>> = b.1.iter().collect();
    for h in &a.1 {
        match rest.iter().position(|c| cyclic_match(h, c, either_way)) {
            Some(i) => {
                rest.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

pub fn multiset_match(a: &[Rings], b: &[Rings], either_way: bool) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut rest: Vec<&Rings> = b.iter().collect();
    for x in a {
        match rest.iter().position(|c| rings_match(x, c, either_way)) {
            Some(i) => {
                rest.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

/// Every face's rings and every edge claim, compared up to tolerance: the
/// object-level equivalence undo and redo must restore.
pub fn objects_equivalent(x: &Object, y: &Object) -> bool {
    let xr: Vec<Rings> = x.faces().keys().map(|f| face_rings(x, f)).collect();
    let yr: Vec<Rings> = y.faces().keys().map(|f| face_rings(y, f)).collect();
    let claims = |o: &Object| o.edges().values().filter(|e| e.curve.is_some()).count();
    multiset_match(&xr, &yr, false) && claims(x) == claims(y)
}

/// The ground oracle (I6): a sketch on the group's plane holding every
/// edge the group had before the draw plus the drawn loop's segments.
pub fn ground_with(plane: Plane, obj_before: &Object, loop_path: &[Point3]) -> Sketch {
    let mut sk = Sketch::on_plane(plane);
    let mut seen = Vec::new();
    for f in group_faces(obj_before, &plane) {
        let face = &obj_before.faces()[f];
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            for h in obj_before.loop_half_edges(l) {
                let e = obj_before.half_edges()[h].edge;
                if seen.contains(&e) {
                    continue;
                }
                seen.push(e);
                let (a, b) = obj_before.edge_endpoints(e).expect("edge");
                sk.add_segment(a, b).expect("group edge on the ground");
            }
        }
    }
    let n = loop_path.len();
    for i in 0..n {
        sk.add_segment(loop_path[i], loop_path[(i + 1) % n])
            .expect("loop segment on the ground");
    }
    sk
}

pub fn ground_regions(sk: &Sketch) -> Vec<Rings> {
    let pos = |id| sk.vertices()[id].position;
    sk.regions()
        .values()
        .map(|r| {
            (
                r.outer.iter().map(|&v| pos(v)).collect(),
                r.holes
                    .iter()
                    .map(|h| h.iter().map(|&v| pos(v)).collect())
                    .collect(),
            )
        })
        .collect()
}

pub fn ground_has_pinch(sk: &Sketch) -> bool {
    sk.regions().values().any(|r| {
        std::iter::once(&r.outer)
            .chain(r.holes.iter())
            .any(|ring| (0..ring.len()).any(|i| ring[i + 1..].contains(&ring[i])))
    })
}

/// The group after the draw equals the ground's regions.
pub fn assert_ground_parity(w: &Wall, before: &Object, loop_path: &[Point3]) {
    let sk = ground_with(w.plane, before, loop_path);
    let ground = ground_regions(&sk);
    let obj = w.obj();
    let faces: Vec<Rings> = group_faces(obj, &w.plane)
        .into_iter()
        .map(|f| face_rings(obj, f))
        .collect();
    assert!(
        multiset_match(&faces, &ground, true),
        "face group ({} faces) differs from the ground ({} regions)",
        faces.len(),
        ground.len()
    );
}

/// I7: the saved document reopens, and saving the reopened document gives
/// the same bytes — no committed state is one the loader rejects.
pub fn assert_reopens(bytes: &[u8]) {
    let reopened = Document::load(bytes)
        .unwrap_or_else(|e| panic!("the saved document does not reopen: {e:?}"));
    assert_eq!(reopened.save(), bytes, "save after reopen differs");
}

/// I1–I5 and I7 around one accepted draw: `before` is the object and `bytes0`
/// the save before it; the document holds the draw as its latest entry.
pub fn assert_invariants(w: &mut Wall, before: &Object, bytes0: &[u8], depth0: usize) {
    let after = w.obj().clone();
    // I1
    after.validate().expect("valid after the draw");
    assert_eq!(after.watertight(), before.watertight(), "watertightness");
    // I2
    assert_eq!(euler(&after), euler(before), "V - E + F - H unchanged");
    // I3
    let (a0, a1) = (group_area(before, &w.plane), group_area(&after, &w.plane));
    assert!(
        (a0 - a1).abs() <= 1e-12 * a0.abs().max(1e-300),
        "group area conserved: {a0} vs {a1}"
    );
    // I4
    assert!(no_pinch(&after, &w.plane), "no pinched loop committed");
    // I5
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo entry");
    let bytes1 = w.doc.save();
    // I7
    assert_reopens(&w.doc.save());
    for _ in 0..3 {
        w.doc.undo().expect("undo the draw");
        assert!(objects_equivalent(w.obj(), before), "undo restores");
        assert_eq!(w.doc.save(), bytes0, "undo restores the saved bytes");
        w.doc.redo().expect("redo the draw");
        assert!(objects_equivalent(w.obj(), &after), "redo restores");
        assert_eq!(w.doc.save(), bytes1, "redo restores the saved bytes");
    }
}

/// Draws `loop_path` on `seed` through one draft, checks I1–I7, and
/// returns the face of the wall's group that now carries the drawn region
/// ([`region_face`]). For a loop that lies wholly on the face (touching or
/// crossing its edges, around imprints), the drawing is face topology and
/// the group equals the ground's regions.
pub fn draw_and_check(
    w: &mut Wall,
    seed: FaceId,
    loop_path: Vec<Point3>,
    curve: Option<CurveGeom>,
) -> FaceId {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let report = w
        .draw_loop(seed, &loop_path, curve)
        .expect("the loop draws")
        .0;
    assert!(
        report.kept_as_sketch.is_empty(),
        "the whole loop landed as face topology"
    );
    assert_ground_parity(w, &before, &loop_path);
    assert_invariants(w, &before, &bytes0, depth0);
    region_face(w, &loop_path).expect("a group face carries the drawn region")
}

/// Draws `loop_path` on `seed` (no invariant checks) and returns the face
/// carrying the drawn region.
pub fn draw_region(
    w: &mut Wall,
    seed: FaceId,
    loop_path: Vec<Point3>,
    curve: Option<CurveGeom>,
) -> FaceId {
    w.draw_loop(seed, &loop_path, curve)
        .expect("the loop draws");
    region_face(w, &loop_path).expect("a group face carries the drawn region")
}

/// Draws a loop the face cannot hold as topology (it touches the face
/// group at exactly one point, so the face's ring would pinch): the draft
/// keeps it as the object's OWNED sketch drawing. Checks I1–I5 and I7, that
/// the group's faces are unchanged, that the report names the island, and
/// that every loop point is drawn in an owned sketch.
pub fn draw_kept_as_sketch(
    w: &mut Wall,
    seed: FaceId,
    loop_path: Vec<Point3>,
    curve: Option<CurveGeom>,
) -> DraftReport {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) = w
        .draw_loop(seed, &loop_path, curve)
        .expect("the loop draws");
    assert!(
        !report.kept_as_sketch.is_empty(),
        "the island the face cannot hold is reported as kept drawing"
    );
    let n = w.plane.normal();
    let rings = |o: &Object| -> Vec<Rings> {
        group_faces(o, &w.plane)
            .into_iter()
            .map(|f| face_rings(o, f))
            .collect()
    };
    assert!(
        multiset_match(
            &straightened(&rings(w.obj()), n),
            &straightened(&rings(&before), n),
            true
        ),
        "the group's faces are unchanged"
    );
    let owned = w.doc.object_owned_sketches(w.id);
    assert!(!owned.is_empty(), "the object owns the drawing");
    // Every loop point is drawn: over the face in the object's owned
    // sketch, past its edge (an overhang) in a free sketch.
    for p in &loop_path {
        assert!(
            w.doc
                .sketch_ids()
                .iter()
                .any(|&s| w.doc.sketch(s).is_some_and(|sk| sk
                    .vertices()
                    .values()
                    .any(|v| v.position.approx_eq(*p, tol::POINT_MERGE)))),
            "loop point {p:?} is drawn in a sketch"
        );
    }
    assert_invariants(w, &before, &bytes0, depth0);
    report
}

// ------------------------------------------------------- drafting helpers

/// The objects a drawing on `plane` in `scope` lands on, with `obj` always
/// among them.
pub fn plane_objects(
    doc: &Document,
    plane: Plane,
    scope: Option<ComponentId>,
    obj: ObjectId,
) -> Vec<ObjectId> {
    let mut objs = doc.drawable_objects_on_plane(plane, scope);
    if !objs.contains(&obj) {
        objs.push(obj);
    }
    objs
}

/// Adds the edges of `pts` to `w` — closed (the last point joins the
/// first) or an open chain — bracketing runs of edges that share an
/// analytic circle claim (`curves[k]` claims edge `k`, `pts[k]` →
/// `pts[k+1]`; empty claims none) in one curve each, as the tools draw a
/// circle, an arc, or a pie.
pub fn add_chain(
    w: &mut Sketch,
    pts: &[Point3],
    curves: &[Option<CurveGeom>],
    closed: bool,
) -> Result<(), SketchError> {
    let n = if closed { pts.len() } else { pts.len() - 1 };
    let claim = |k: usize| curves.get(k).copied().flatten();
    // A closed loop starts where the claim changes, so one circle is one
    // bracket rather than two across the seam.
    let start = if closed {
        (0..n)
            .find(|&k| claim(k) != claim((k + n - 1) % n))
            .unwrap_or(0)
    } else {
        0
    };
    let mut open: Option<CurveGeom> = None;
    for i in 0..n {
        let k = (start + i) % n;
        let c = claim(k);
        if c != open {
            if open.is_some() {
                w.end_curve();
            }
            if let Some(g) = c {
                w.begin_curve_with(g)?;
            }
            open = c;
        }
        w.add_segment(pts[k], pts[(k + 1) % pts.len()])?;
    }
    if open.is_some() {
        w.end_curve();
    }
    Ok(())
}

/// The plane of `face` of `obj`.
pub fn face_plane(doc: &Document, obj: ObjectId, face: FaceId) -> Plane {
    doc.object(obj).expect("object").faces()[face].plane
}

/// One draft over the plane of `face`, with every drawable object of the
/// plane (and `obj`) taking part; `edit` runs on the working sketch.
pub fn draft_on_face(
    doc: &mut Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    face: FaceId,
    label: &str,
    edit: impl FnOnce(&mut Sketch) -> Result<(), SketchError>,
) -> Result<(DraftReport, DocChange), DocumentError> {
    let plane = face_plane(doc, obj, face);
    let objs = plane_objects(doc, plane, scope, obj);
    doc.draft(plane, scope, &objs, label, edit)
}

/// Draws the closed loop `loop_path` on `face`'s plane in one draft;
/// `curve` claims every edge for one circle.
pub fn draw_loop(
    doc: &mut Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    face: FaceId,
    loop_path: &[Point3],
    curve: Option<CurveGeom>,
) -> Result<(DraftReport, DocChange), DocumentError> {
    let curves = curve
        .map(|g| vec![Some(g); loop_path.len()])
        .unwrap_or_default();
    draw_loop_curves(doc, scope, obj, face, loop_path, &curves)
}

/// [`draw_loop`] with a per-edge claim.
pub fn draw_loop_curves(
    doc: &mut Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    face: FaceId,
    loop_path: &[Point3],
    curves: &[Option<CurveGeom>],
) -> Result<(DraftReport, DocChange), DocumentError> {
    draft_on_face(doc, scope, obj, face, "Draw", |w| {
        add_chain(w, loop_path, curves, true)
    })
}

/// Draws the open chain `path` on `face`'s plane in one draft (`curves`
/// empty, or one claim per path edge).
pub fn draw_path(
    doc: &mut Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    face: FaceId,
    path: &[Point3],
    curves: &[Option<CurveGeom>],
) -> Result<(DraftReport, DocChange), DocumentError> {
    draft_on_face(doc, scope, obj, face, "Draw", |w| {
        add_chain(w, path, curves, false)
    })
}

/// The drawing on `plane` as units and regions, over every drawable
/// object of the plane and `obj`.
pub fn units_on(
    doc: &Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    plane: Plane,
) -> PlaneUnits {
    let objs = plane_objects(doc, plane, scope, obj);
    doc.plane_units(plane, scope, &objs).expect("plane units")
}

/// Whether the segment `a`–`b` of a unit passes within the merge
/// tolerance of `p`.
fn passes_through(a: Point3, b: Point3, p: Point3) -> bool {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    (a + ab * t).approx_eq(p, tol::POINT_MERGE)
}

/// The drawing unit with an edge through `p`.
pub fn unit_through(units: &PlaneUnits, p: Point3) -> Option<&DraftUnit> {
    units
        .units
        .iter()
        .find(|u| u.edges.iter().any(|e| passes_through(e.a, e.b, p)))
}

/// The working-sketch edges of `unit` (it was read from the same lift).
fn unit_edges(w: &Sketch, unit: &DraftUnit) -> Vec<kernel::SketchEdgeId> {
    unit.edges
        .iter()
        .map(|e| {
            w.edge_at_positions(e.a, e.b)
                .unwrap_or_else(|| panic!("unit edge {:?}-{:?} in the working sketch", e.a, e.b))
        })
        .collect()
}

/// Moves the drawing unit through `p` by `xf` in one draft.
pub fn move_unit_at(
    doc: &mut Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    plane: Plane,
    p: Point3,
    xf: &Transform,
) -> Result<(DraftReport, DocChange), DocumentError> {
    let objs = plane_objects(doc, plane, scope, obj);
    let units = doc.plane_units(plane, scope, &objs).expect("plane units");
    let unit = unit_through(&units, p)
        .unwrap_or_else(|| panic!("no drawing unit through {p:?}"))
        .clone();
    doc.draft(plane, scope, &objs, "Move", |w| {
        let es = unit_edges(w, &unit);
        w.apply_transform_edges(&es, xf)
    })
}

/// Erases the drawing unit through `p` in one draft.
pub fn delete_unit_at(
    doc: &mut Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    plane: Plane,
    p: Point3,
) -> Result<(DraftReport, DocChange), DocumentError> {
    let objs = plane_objects(doc, plane, scope, obj);
    let units = doc.plane_units(plane, scope, &objs).expect("plane units");
    let unit = unit_through(&units, p)
        .unwrap_or_else(|| panic!("no drawing unit through {p:?}"))
        .clone();
    doc.draft(plane, scope, &objs, "Erase", |w| {
        for e in unit_edges(w, &unit) {
            w.remove_edge(e)?;
        }
        Ok(())
    })
}

/// A point strictly inside the simple polygon `ring` (the centroid of a
/// convex ear), for finding which face holds a loop's interior.
pub fn interior_point(ring: &[Point3], n: Vec3) -> Point3 {
    let k = ring.len();
    let area = signed_area(ring, n);
    for i in 0..k {
        let (a, b, c) = (ring[(i + k - 1) % k], ring[i], ring[(i + 1) % k]);
        let convex = (b - a).cross(c - b).dot(n) * area.signum() > 0.0;
        if !convex {
            continue;
        }
        let centroid = Point3::ORIGIN + (a.to_vec() + b.to_vec() + c.to_vec()) / 3.0;
        let clear = (0..k).all(|j| {
            let p = ring[j];
            p.approx_eq(a, tol::POINT_MERGE)
                || p.approx_eq(b, tol::POINT_MERGE)
                || p.approx_eq(c, tol::POINT_MERGE)
                || !in_triangle(p, a, b, c, n)
        });
        if clear {
            return centroid;
        }
    }
    panic!("no interior point for {ring:?}");
}

fn in_triangle(p: Point3, a: Point3, b: Point3, c: Point3, n: Vec3) -> bool {
    let s = |u: Point3, v: Point3| (v - u).cross(p - u).dot(n);
    let (x, y, z) = (s(a, b), s(b, c), s(c, a));
    (x >= 0.0 && y >= 0.0 && z >= 0.0) || (x <= 0.0 && y <= 0.0 && z <= 0.0)
}

/// The group face carrying the region a loop drew: the drawing's region
/// whose outer ring is `loop_path` (straight-through vertices aside) and
/// the face under it; failing that (the loop was cut into pieces), the
/// group face holding a point strictly inside the loop.
pub fn region_face(w: &Wall, loop_path: &[Point3]) -> Option<FaceId> {
    region_face_in(&w.doc, None, w.id, w.plane, loop_path)
}

/// [`region_face`] for any document, scope, object, and plane.
pub fn region_face_in(
    doc: &Document,
    scope: Option<ComponentId>,
    obj: ObjectId,
    plane: Plane,
    loop_path: &[Point3],
) -> Option<FaceId> {
    let n = plane.normal();
    let units = units_on(doc, scope, obj, plane);
    let want = without_straight(loop_path, n);
    for r in &units.regions {
        let got = without_straight(&r.outer, n);
        if cyclic_match(&got, &want, true)
            && let Some((o, Some(f))) = r.over
            && o == obj
        {
            return Some(f);
        }
    }
    let q = interior_point(loop_path, n);
    let o = doc.object(obj)?;
    group_faces(o, &plane)
        .into_iter()
        .find(|&f| o.face_contains_point(f, q))
}

/// Claims on the object centered at `c` with radius `r`.
pub fn claims_on(obj: &Object, g: CurveGeom) -> usize {
    obj.edges()
        .values()
        .filter_map(|e| e.curve)
        .filter(|c| {
            c.center.approx_eq(g.center, tol::POINT_MERGE) && (c.radius - g.radius).abs() < 1e-15
        })
        .count()
}

// ------------------------------------------- cuts, merges, and refusals

/// A point `(x, y)` millimetres into the wall's frame.
pub fn mm(w: &Wall, x: f64, y: f64) -> Point3 {
    w.at(x * MM, y * MM)
}

/// Imprints a `rw × rh` mm rectangle at `(x, y)` mm on `seed`; the sub-face.
pub fn imprint_mm(w: &mut Wall, seed: FaceId, x: f64, y: f64, rw: f64, rh: f64) -> FaceId {
    let rect = w.rect(x * MM, y * MM, rw * MM, rh * MM);
    draw_region(w, seed, rect, None)
}

pub fn ring_of(obj: &Object, l: kernel::LoopId) -> Vec<Point3> {
    obj.loop_positions(l).collect()
}

/// The live edge with endpoints `a`, `b` (either order).
pub fn edge_at(obj: &Object, a: Point3, b: Point3) -> EdgeId {
    find_edge(obj, a, b).unwrap_or_else(|| panic!("no edge {a:?} – {b:?}"))
}

pub fn find_edge(obj: &Object, a: Point3, b: Point3) -> Option<EdgeId> {
    obj.edges().keys().find(|&e| {
        let (p, q) = obj.edge_endpoints(e).expect("live edge");
        (p.approx_eq(a, tol::POINT_MERGE) && q.approx_eq(b, tol::POINT_MERGE))
            || (p.approx_eq(b, tol::POINT_MERGE) && q.approx_eq(a, tol::POINT_MERGE))
    })
}

/// The object's vertex within [`tol::POINT_MERGE`] of `p`, exactly.
pub fn vertex_at(obj: &Object, p: Point3) -> Point3 {
    obj.vertices()
        .values()
        .map(|v| v.position)
        .find(|q| q.approx_eq(p, tol::POINT_MERGE))
        .unwrap_or_else(|| panic!("no vertex at {p:?}"))
}

/// A ground sketch on `plane` holding every group edge of `obj`, plus the
/// open polyline `add`, minus the edges `remove`: the ground side of I6
/// for a drawn cut (`add`) or a deleted run (`remove`).
pub fn ground_edit(
    plane: Plane,
    obj: &Object,
    add: &[Point3],
    remove: &[(Point3, Point3)],
) -> Sketch {
    let mut sk = Sketch::on_plane(plane);
    let mut seen = Vec::new();
    for f in group_faces(obj, &plane) {
        let face = &obj.faces()[f];
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            for h in obj.loop_half_edges(l) {
                let e = obj.half_edges()[h].edge;
                if seen.contains(&e) {
                    continue;
                }
                seen.push(e);
                let (a, b) = obj.edge_endpoints(e).expect("edge");
                sk.add_segment(a, b).expect("group edge on the ground");
            }
        }
    }
    for w in add.windows(2) {
        sk.add_segment(w[0], w[1])
            .expect("cut segment on the ground");
    }
    for &(a, b) in remove {
        let e = sk
            .edge_at_positions(a, b)
            .unwrap_or_else(|| panic!("ground edge {a:?} – {b:?}"));
        sk.remove_edge(e).expect("remove the ground edge");
    }
    sk
}

/// I6: the wall's face group equals the regions of `sk`. The solid's rings
/// are compared exactly; only the GROUND side may shed straight-through
/// vertices (a line drawn along an edge leaves one on the ground where it
/// ends, and the solid inserts none), so a stray vertex on the solid still
/// fails.
pub fn assert_group_matches_ground(w: &Wall, sk: &Sketch) {
    let n = w.plane.normal();
    let ground = ground_regions(sk);
    let obj = w.obj();
    let faces: Vec<Rings> = group_faces(obj, &w.plane)
        .into_iter()
        .map(|f| face_rings(obj, f))
        .collect();
    assert!(
        multiset_match(&faces, &ground, true)
            || multiset_match(&faces, &straightened(&ground, n), true),
        "face group ({} faces) differs from the ground ({} regions)",
        faces.len(),
        ground.len()
    );
}

/// Applies `op` through the document and checks I1–I5 and I7 around it. The
/// report's handles predate the undo/redo cycles; re-find faces by point.
pub fn apply_and_check(w: &mut Wall, op: KernelOp) -> KernelOpReport {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) = w.doc.apply_object_op(w.id, op).expect("the op applies");
    assert_invariants(w, &before, &bytes0, depth0);
    report
}

/// A cut through the document with I1–I7.
pub fn split_and_check(w: &mut Wall, face: FaceId, path: Vec<Point3>) -> FaceSplitReport {
    let before = w.obj().clone();
    let report = apply_and_check(
        w,
        KernelOp::SplitFace {
            face,
            path: path.clone(),
            restore: None,
            curves: Vec::new(),
        },
    );
    assert_group_matches_ground(w, &ground_edit(w.plane, &before, &path, &[]));
    match report {
        KernelOpReport::FaceSplit(r) => r,
        other => panic!("expected a split report, got {other:?}"),
    }
}

/// Deletes the run holding `edge` through the document with I1–I7: the
/// ground loses exactly the run's edges.
pub fn merge_and_check(w: &mut Wall, edge: EdgeId) -> kernel::FaceMergeReport {
    let before = w.obj().clone();
    let run = run_segments(&before, edge);
    let report = apply_and_check(
        w,
        KernelOp::MergeFaces {
            edge,
            keep_vertices: Vec::new(),
        },
    );
    assert_group_matches_ground(w, &ground_edit(w.plane, &before, &[], &run));
    match report {
        KernelOpReport::FaceMerge(r) => r,
        other => panic!("expected a merge report, got {other:?}"),
    }
}

/// Every edge the two faces of `edge` share (the run a merge dissolves).
pub fn run_segments(obj: &Object, edge: EdgeId) -> Vec<(Point3, Point3)> {
    let e = obj.edges()[edge];
    let la = obj.half_edges()[e.half_edge].loop_id;
    let lb = obj.half_edges()[e.twin_half_edge.expect("interior edge")].loop_id;
    obj.loop_half_edges(la)
        .filter(|&h| {
            obj.half_edges()[h]
                .twin
                .is_some_and(|t| obj.half_edges()[t].loop_id == lb)
        })
        .map(|h| obj.edge_endpoints(obj.half_edges()[h].edge).expect("edge"))
        .collect()
}

/// `op` refuses with exactly `want`, document, saves, and undo depth
/// untouched.
pub fn assert_op_refused(w: &mut Wall, op: KernelOp, want: StickyError) {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let err = w.doc.apply_object_op(w.id, op).expect_err("refused");
    assert_eq!(sticky(err), want);
    assert!(objects_equivalent(w.obj(), &before), "object untouched");
    assert_eq!(w.doc.save(), bytes0, "saved bytes untouched");
    assert_eq!(w.doc.undo_depth(), depth0, "no undo entry");
}

pub fn split_op(face: FaceId, path: Vec<Point3>) -> KernelOp {
    KernelOp::SplitFace {
        face,
        path,
        restore: None,
        curves: Vec::new(),
    }
}

pub fn mm2() -> f64 {
    MM * MM
}

pub fn outer_area(obj: &Object, f: FaceId, n: Vec3) -> f64 {
    signed_area(&ring_of(obj, obj.faces()[f].outer_loop), n)
}

/// `loop_path` as the planner reads it at the merge tolerance: a vertex
/// within POINT_MERGE of one of the group's vertices moves onto it, else
/// one within POINT_MERGE of an edge's interior moves onto the edge. The
/// ground sketch judges near-collinear contacts by parameter rather than
/// by distance, so below POINT_MERGE it can see a sliver (a vertex 0.8 nm
/// outside an edge, joined to a point on it) that is one point to every
/// metric gate; I6 is checked against the ground of the snapped loop.
pub fn snapped_like_the_planner(
    before: &Object,
    plane: &Plane,
    loop_path: &[Point3],
) -> Vec<Point3> {
    let mut verts = Vec::new();
    let mut edges = Vec::new();
    for f in group_faces(before, plane) {
        let face = &before.faces()[f];
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            for h in before.loop_half_edges(l) {
                let e = before.half_edges()[h].edge;
                let (a, b) = before.edge_endpoints(e).expect("edge");
                verts.push(a);
                edges.push((a, b));
            }
        }
    }
    let m = tol::POINT_MERGE;
    loop_path
        .iter()
        .map(|&p| {
            if let Some(&v) = verts.iter().find(|v| v.approx_eq(p, m)) {
                return v;
            }
            edges
                .iter()
                .find_map(|&(a, b)| {
                    let ab = b - a;
                    let t = (p - a).dot(ab) / ab.length_squared();
                    let q = a + ab * t;
                    (t > m && t < 1.0 - m && q.approx_eq(p, m)).then_some(q)
                })
                // Projected within the merge tolerance of the edge's end, it
                // IS that end (no sub-tolerance sliver of the edge is left).
                .map(|q| {
                    verts
                        .iter()
                        .copied()
                        .find(|v| v.approx_eq(q, m))
                        .unwrap_or(q)
                })
                .unwrap_or(p)
        })
        .collect()
}
