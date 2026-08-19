//! Hidden-line removal for printed and exported line drawings
//! (docs/design/printing.md §7b).
//!
//! Given tessellated, posed solids and a camera, `line_drawing` returns the
//! line work a draftsperson would ink: every **hard** edge, the
//! **silhouettes** of curved walls (soft facet seams where the surface turns
//! away from the viewer), and the **section-cut outline** where an active
//! section plane slices geometry — each split into its visible pieces (and,
//! on request, its hidden pieces, for dashed hidden-line drawings). Output is
//! 2D, in view-plane metres, y up, origin at the camera target, so a caller
//! at drawing scale `r` (paper/model) multiplies by `r` and is on paper.
//!
//! Approach (analytic, not raster): project everything to the view plane;
//! for each candidate segment gather the triangles whose projected bounding
//! box overlaps it (uniform grid), clip the segment's parameter range against
//! the triangle's 2D footprint, and inside that range compare the segment's
//! depth with the occluder plane's depth — both are linear in the projected
//! parameter (parallel: depth itself; perspective: 1/depth, the classic
//! perspective-correct interpolation), so a hidden run is a single sub-interval
//! found by one root. A segment lying ON an occluder's plane (its own face, a
//! coplanar neighbour, the shared face of two touching solids) is never hidden
//! by it: "behind" means behind by more than a depth tolerance proportional
//! to the scene size. Hidden intervals are unioned and subtracted.
//!
//! Determinism: f64 arithmetic in a fixed traversal order; tolerances are the
//! named constants below. Every input triangle counts against a
//! candidate×occluder budget so a pathological model returns
//! `HlrError::TooComplex` (the caller falls back to raster) instead of
//! spinning.
//!
//! Kernel purity (DEVELOPMENT.md rule 1): depends on `kernel` and
//! `tessellate` only. Which items to draw — hidden nodes, hidden tags, a
//! Scene's leaf sets — is the caller's decision (softrender's
//! `document_items_hiding` already resolves it for snapshots).

pub mod svg;

use kernel::{Point3, Transform, Vec3};
use tessellate::RenderMesh;

/// One drawable: a tessellated mesh at a pose, tagged with the stable id its
/// segments report.
pub struct Item<'a> {
    pub mesh: &'a RenderMesh,
    pub pose: Transform,
    pub sid: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// Orthographic along the view direction (the drawing-scale case).
    Parallel,
    /// Perspective from `eye`; output coordinates are on the plane through
    /// `target` perpendicular to the view direction.
    Perspective,
}

/// A view: eye, target, up, projection (same shape as softrender's camera).
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Vec3,
    pub projection: Projection,
}

/// A section cut: the side `normal` points toward is REMOVED (the viewport's
/// convention — lowering a horizontal section removes the top and exposes
/// the interior below); points with `(p - origin) · normal <= 0` are kept.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub origin: Point3,
    pub normal: Vec3,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub section: Option<Section>,
    /// Also return the hidden pieces (kind `Hidden`) for dashed hidden lines.
    pub include_hidden: bool,
    /// Also return non-silhouette soft facet seams (kind `Soft`).
    pub include_soft: bool,
    /// Maximum candidate×occluder pair tests before `TooComplex`.
    pub budget: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            section: None,
            include_hidden: false,
            include_soft: false,
            budget: DEFAULT_BUDGET,
        }
    }
}

/// Default pair budget: comfortably above a room-sized furniture model
/// (~50k edges × ~200 nearby triangles), well below anything that hangs.
pub const DEFAULT_BUDGET: u64 = 40_000_000;

/// Depth tolerance as a fraction of the scene's projected diagonal: a
/// segment counts as hidden only when it lies deeper than the occluder by
/// more than this. `tessellate` positions are `f32` (≈1e-7 relative), so 1e-5
/// leaves two decades of margin for a coplanar edge on its own face while
/// still resolving any step a printer could show.
pub const DEPTH_TOL_REL: f64 = 1e-5;
/// Projected pieces shorter than this fraction of the diagonal are dropped,
/// and a hidden run that ends within it of a segment's end is snapped to the
/// end. This is where the depth tolerance would otherwise leave slivers: an
/// edge disappearing behind a face at a shared vertex is "not yet deeper
/// than the tolerance" for its first few tolerances of length. 5e-5 of the
/// diagonal is 50 µm on a metre — a tenth of the finest printed stroke.
pub const MIN_PIECE_REL: f64 = 5e-5;
/// Projected triangles with less area than (diagonal × this)² are edge-on and
/// occlude nothing.
pub const MIN_AREA_REL: f64 = 1e-6;
/// Perspective near plane, as a fraction of the eye–target distance.
pub const NEAR_REL: f64 = 1e-3;

/// What kind of line a segment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A hard edge (every real edge except curved-wall facet seams).
    Hard,
    /// A soft seam where the curved wall turns away from the viewer.
    Silhouette,
    /// A soft seam that is not a silhouette (only with `include_soft`).
    Soft,
    /// Where the section plane cuts a face.
    Section,
    /// A hidden piece of any of the above (only with `include_hidden`).
    Hidden,
}

/// A visible (or hidden) piece of line, view-plane metres, y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seg {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub kind: Kind,
    pub sid: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineDrawing {
    pub segs: Vec<Seg>,
    /// Bounding box of every returned segment (min, max), or None when empty.
    pub bounds: Option<([f64; 2], [f64; 2])>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlrError {
    /// More candidate×occluder tests than the budget allows.
    TooComplex { pairs: u64, budget: u64 },
    /// Eye on the target, or up parallel to the view direction with no
    /// perpendicular to fall back to.
    DegenerateCamera,
}

impl std::fmt::Display for HlrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HlrError::TooComplex { pairs, budget } => {
                write!(
                    f,
                    "line drawing too complex: {pairs} candidate/occluder pairs (budget {budget})"
                )
            }
            HlrError::DegenerateCamera => write!(f, "degenerate camera"),
        }
    }
}

impl std::error::Error for HlrError {}

// ---------------------------------------------------------------- frame

/// The camera frame: orthonormal `right`/`upv`/`dir` and the eye→target
/// distance. `project` maps world points to (x, y, z) where x/y are
/// view-plane metres about the target and z is the depth along `dir` FROM THE
/// EYE for perspective (so 1/z is the perspective-correct interpolant) and
/// from the target plane for parallel.
#[derive(Debug, Clone, Copy)]
struct Frame {
    eye: Point3,
    target: Point3,
    right: Vec3,
    upv: Vec3,
    dir: Vec3,
    /// Eye→target distance (perspective's projection-plane distance).
    dist: f64,
    perspective: bool,
}

/// A world point in the camera frame.
#[derive(Debug, Clone, Copy)]
struct V {
    /// View-plane coords (metres about the target) — for perspective these
    /// are the *unprojected* offsets; use `px()`/`py()` for the projection.
    x: f64,
    y: f64,
    /// Depth: perspective — distance from the eye along `dir`; parallel —
    /// signed distance from the target plane along `dir`.
    z: f64,
}

impl Frame {
    fn new(cam: &Camera) -> Result<Frame, HlrError> {
        let to_target = cam.target - cam.eye;
        let dist = to_target.length();
        if dist <= 0.0 || !dist.is_finite() {
            return Err(HlrError::DegenerateCamera);
        }
        let dir = to_target * (1.0 / dist);
        let mut right = cross(dir, cam.up);
        if right.length() < 1e-9 {
            // Looking straight along `up` (a plan or an underside): take
            // world +X as "right" (else +Y when the view runs along X),
            // orthogonalized against the direction — the same rule the app's
            // print pass uses, so vector and raster pages agree.
            let seed = if dir.x.abs() < 0.9 {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            right = seed - dir * dot(seed, dir);
            if right.length() < 1e-9 {
                return Err(HlrError::DegenerateCamera);
            }
        }
        let right = right * (1.0 / right.length());
        let upv = cross(right, dir);
        Ok(Frame {
            eye: cam.eye,
            target: cam.target,
            right,
            upv,
            dir,
            dist,
            perspective: cam.projection == Projection::Perspective,
        })
    }

    fn view(&self, p: Point3) -> V {
        let rel = p - self.target;
        let x = dot(rel, self.right);
        let y = dot(rel, self.upv);
        let d = dot(rel, self.dir);
        let z = if self.perspective { d + self.dist } else { d };
        V { x, y, z }
    }

    /// Projected view-plane coordinates (metres about the target).
    #[inline]
    fn proj(&self, v: &V) -> [f64; 2] {
        if self.perspective {
            let s = self.dist / v.z;
            [v.x * s, v.y * s]
        } else {
            [v.x, v.y]
        }
    }

    /// The depth metric that is linear along a projected segment: z for
    /// parallel, -dist/z for perspective. Larger = farther.
    #[inline]
    fn metric(&self, z: f64) -> f64 {
        if self.perspective { -self.dist / z } else { z }
    }
}

#[inline]
fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

#[inline]
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

// ------------------------------------------------------------ geometry

/// A world-space triangle after posing (and section clipping).
#[derive(Debug, Clone, Copy)]
struct Tri {
    a: Point3,
    b: Point3,
    c: Point3,
}

/// A candidate line in world space.
#[derive(Debug, Clone, Copy)]
struct Cand {
    a: Point3,
    b: Point3,
    kind: Kind,
    sid: u64,
}

/// A projected occluder: 2D footprint + plane in the frame's (x, y, metric)
/// space, where metric is linear over the footprint.
#[derive(Debug, Clone, Copy)]
struct Occ {
    p: [[f64; 2]; 3],
    /// Bounding box of the footprint.
    min: [f64; 2],
    max: [f64; 2],
    /// Metric as an affine function of projected (x, y): m = k0 + kx·x + ky·y.
    k0: f64,
    kx: f64,
    ky: f64,
    /// Plane in world space for the "segment lies on the plane" test.
    n: Vec3,
    /// n · p for points on the plane.
    c: f64,
    /// Winding sign of the 2D footprint (+1 CCW, -1 CW).
    sign: f64,
}

fn tri_normal(t: &Tri) -> Vec3 {
    cross(t.b - t.a, t.c - t.a)
}

/// Clip a triangle to the kept half-space of a section (`(p - o)·n <= 0`).
/// Returns 0, 1, or 2 triangles plus the cut segment (if the plane crosses
/// the triangle's interior).
fn clip_tri(t: &Tri, sec: &Section) -> (Vec<Tri>, Option<(Point3, Point3)>) {
    let d = |p: Point3| dot(p - sec.origin, sec.normal);
    let pts = [t.a, t.b, t.c];
    let ds = [d(t.a), d(t.b), d(t.c)];
    let inside: Vec<usize> = (0..3).filter(|&i| ds[i] <= 0.0).collect();
    match inside.len() {
        3 => (vec![*t], None),
        0 => (Vec::new(), None),
        _ => {
            // Walk the polygon, emitting kept vertices and edge crossings.
            let mut poly: Vec<Point3> = Vec::with_capacity(4);
            let mut cut: Vec<Point3> = Vec::with_capacity(2);
            for i in 0..3 {
                let j = (i + 1) % 3;
                let (pi, pj) = (pts[i], pts[j]);
                let (di, dj) = (ds[i], ds[j]);
                if di <= 0.0 {
                    poly.push(pi);
                }
                if (di <= 0.0) != (dj <= 0.0) {
                    let t = di / (di - dj);
                    let x = pi + (pj - pi) * t;
                    poly.push(x);
                    cut.push(x);
                }
            }
            let mut out = Vec::with_capacity(2);
            if poly.len() >= 3 {
                out.push(Tri {
                    a: poly[0],
                    b: poly[1],
                    c: poly[2],
                });
                if poly.len() == 4 {
                    out.push(Tri {
                        a: poly[0],
                        b: poly[2],
                        c: poly[3],
                    });
                }
            }
            let seg = if cut.len() == 2 {
                Some((cut[0], cut[1]))
            } else {
                None
            };
            (out, seg)
        }
    }
}

/// Clip a candidate segment to the kept half-space.
fn clip_seg(a: Point3, b: Point3, sec: &Section) -> Option<(Point3, Point3)> {
    let da = dot(a - sec.origin, sec.normal);
    let db = dot(b - sec.origin, sec.normal);
    match (da <= 0.0, db <= 0.0) {
        (true, true) => Some((a, b)),
        (false, false) => None,
        (true, false) => {
            let t = da / (da - db);
            Some((a, a + (b - a) * t))
        }
        (false, true) => {
            let t = da / (da - db);
            Some((a + (b - a) * t, b))
        }
    }
}

// ------------------------------------------------------------- grid

struct Grid {
    min: [f64; 2],
    inv_cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(occs: &[Occ]) -> Grid {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for o in occs {
            for k in 0..2 {
                min[k] = min[k].min(o.min[k]);
                max[k] = max[k].max(o.max[k]);
            }
        }
        if occs.is_empty() || !min[0].is_finite() {
            return Grid {
                min: [0.0, 0.0],
                inv_cell: 1.0,
                nx: 1,
                ny: 1,
                cells: vec![Vec::new()],
            };
        }
        let w = (max[0] - min[0]).max(1e-12);
        let h = (max[1] - min[1]).max(1e-12);
        // Aim for ~4 triangles per cell, capped so the grid stays small.
        let n = ((occs.len() as f64 / 4.0).sqrt().ceil() as usize).clamp(1, 512);
        let cell = (w.max(h)) / n as f64;
        let nx = ((w / cell).ceil() as usize).clamp(1, 1024);
        let ny = ((h / cell).ceil() as usize).clamp(1, 1024);
        let mut cells = vec![Vec::new(); nx * ny];
        let inv_cell = 1.0 / cell;
        for (i, o) in occs.iter().enumerate() {
            let (x0, y0) = (
                cell_index(o.min[0], min[0], inv_cell, nx),
                cell_index(o.min[1], min[1], inv_cell, ny),
            );
            let (x1, y1) = (
                cell_index(o.max[0], min[0], inv_cell, nx),
                cell_index(o.max[1], min[1], inv_cell, ny),
            );
            for cy in y0..=y1 {
                for cx in x0..=x1 {
                    cells[cy * nx + cx].push(i as u32);
                }
            }
        }
        Grid {
            min,
            inv_cell,
            nx,
            ny,
            cells,
        }
    }

    /// Visit each occluder index overlapping the box once.
    fn query(
        &self,
        min: [f64; 2],
        max: [f64; 2],
        stamp: &mut [u32],
        stamp_id: u32,
        out: &mut Vec<u32>,
    ) {
        out.clear();
        let (x0, y0) = (
            cell_index(min[0], self.min[0], self.inv_cell, self.nx),
            cell_index(min[1], self.min[1], self.inv_cell, self.ny),
        );
        let (x1, y1) = (
            cell_index(max[0], self.min[0], self.inv_cell, self.nx),
            cell_index(max[1], self.min[1], self.inv_cell, self.ny),
        );
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                for &i in &self.cells[cy * self.nx + cx] {
                    if stamp[i as usize] != stamp_id {
                        stamp[i as usize] = stamp_id;
                        out.push(i);
                    }
                }
            }
        }
    }
}

#[inline]
fn cell_index(v: f64, min: f64, inv_cell: f64, n: usize) -> usize {
    let i = ((v - min) * inv_cell).floor();
    if i.is_nan() || i < 0.0 {
        0
    } else {
        (i as usize).min(n - 1)
    }
}

// ------------------------------------------------------ main routine

/// Compute the line drawing. See the module doc.
pub fn line_drawing(
    items: &[Item<'_>],
    camera: &Camera,
    opts: &Options,
) -> Result<LineDrawing, HlrError> {
    let frame = Frame::new(camera)?;

    // ---- 1. world-space triangles + candidates, per item
    let mut tris: Vec<Tri> = Vec::new();
    let mut cands: Vec<Cand> = Vec::new();
    for item in items {
        let m = item.mesh;
        let pos = |i: usize| -> Point3 {
            item.pose.apply_point(Point3::new(
                m.positions[i * 3] as f64,
                m.positions[i * 3 + 1] as f64,
                m.positions[i * 3 + 2] as f64,
            ))
        };
        // Triangles (before section clipping) with facing, for silhouettes.
        let mut item_tris: Vec<Tri> = Vec::with_capacity(m.indices.len() / 3);
        for t in m.indices.chunks_exact(3) {
            item_tris.push(Tri {
                a: pos(t[0] as usize),
                b: pos(t[1] as usize),
                c: pos(t[2] as usize),
            });
        }
        // Silhouettes: soft edges whose two facets face opposite ways. Edge
        // key = exact f32 endpoint bits (shared vertices come from the same
        // kernel point, so they match bit-for-bit) in DEFINITION space,
        // before posing.
        // BTreeMap, not HashMap: only looked up, never iterated — but the
        // kernel-class determinism lint holds here too.
        let mut adjacency: std::collections::BTreeMap<[u32; 6], Vec<(usize, bool)>> =
            std::collections::BTreeMap::new();
        if !m.soft_edge_positions.is_empty() {
            let raw = |i: usize| -> [u32; 3] {
                [
                    m.positions[i * 3].to_bits(),
                    m.positions[i * 3 + 1].to_bits(),
                    m.positions[i * 3 + 2].to_bits(),
                ]
            };
            for (ti, t) in m.indices.chunks_exact(3).enumerate() {
                let tri = &item_tris[ti];
                let n = tri_normal(tri);
                let facing = if frame.perspective {
                    let centroid = Point3::new(
                        (tri.a.x + tri.b.x + tri.c.x) / 3.0,
                        (tri.a.y + tri.b.y + tri.c.y) / 3.0,
                        (tri.a.z + tri.b.z + tri.c.z) / 3.0,
                    );
                    dot(n, centroid - frame.eye) < 0.0
                } else {
                    dot(n, frame.dir) < 0.0
                };
                for e in 0..3 {
                    let (i, j) = (t[e] as usize, t[(e + 1) % 3] as usize);
                    adjacency
                        .entry(edge_key(raw(i), raw(j)))
                        .or_default()
                        .push((ti, facing));
                }
            }
        }
        for seg in m.edge_positions.chunks_exact(6) {
            let a = item
                .pose
                .apply_point(Point3::new(seg[0] as f64, seg[1] as f64, seg[2] as f64));
            let b = item
                .pose
                .apply_point(Point3::new(seg[3] as f64, seg[4] as f64, seg[5] as f64));
            cands.push(Cand {
                a,
                b,
                kind: Kind::Hard,
                sid: item.sid,
            });
        }
        for seg in m.soft_edge_positions.chunks_exact(6) {
            let ka = [seg[0].to_bits(), seg[1].to_bits(), seg[2].to_bits()];
            let kb = [seg[3].to_bits(), seg[4].to_bits(), seg[5].to_bits()];
            let kind = match adjacency.get(&edge_key(ka, kb)) {
                Some(list) if list.iter().any(|(_, f)| *f) && list.iter().any(|(_, f)| !*f) => {
                    Kind::Silhouette
                }
                _ => Kind::Soft,
            };
            if kind == Kind::Soft && !opts.include_soft {
                continue;
            }
            let a = item
                .pose
                .apply_point(Point3::new(seg[0] as f64, seg[1] as f64, seg[2] as f64));
            let b = item
                .pose
                .apply_point(Point3::new(seg[3] as f64, seg[4] as f64, seg[5] as f64));
            cands.push(Cand {
                a,
                b,
                kind,
                sid: item.sid,
            });
        }
        // Section clipping.
        if let Some(sec) = &opts.section {
            for t in &item_tris {
                let (kept, cut) = clip_tri(t, sec);
                tris.extend(kept);
                if let Some((a, b)) = cut {
                    cands.push(Cand {
                        a,
                        b,
                        kind: Kind::Section,
                        sid: item.sid,
                    });
                }
            }
        } else {
            tris.extend(item_tris);
        }
    }
    if let Some(sec) = &opts.section {
        cands = cands
            .into_iter()
            .filter_map(|c| {
                if c.kind == Kind::Section {
                    return Some(c);
                }
                clip_seg(c.a, c.b, sec).map(|(a, b)| Cand { a, b, ..c })
            })
            .collect();
    }

    // ---- 2. project occluders
    let near = frame.dist * NEAR_REL;
    let mut occs: Vec<Occ> = Vec::with_capacity(tris.len());
    let mut all_min = [f64::INFINITY; 2];
    let mut all_max = [f64::NEG_INFINITY; 2];
    let mut pending: Vec<(Tri, [V; 3])> = Vec::with_capacity(tris.len());
    for t in &tris {
        let vs = [frame.view(t.a), frame.view(t.b), frame.view(t.c)];
        if frame.perspective && vs.iter().any(|v| v.z <= near) {
            // Behind or on the near plane: not an occluder (a proper clip
            // would be more faithful; such triangles are off-page anyway).
            continue;
        }
        let ps = [frame.proj(&vs[0]), frame.proj(&vs[1]), frame.proj(&vs[2])];
        for p in &ps {
            for k in 0..2 {
                all_min[k] = all_min[k].min(p[k]);
                all_max[k] = all_max[k].max(p[k]);
            }
        }
        pending.push((*t, vs));
    }
    // Candidates contribute to the extent too (a lone segment scene).
    for c in &cands {
        for p in [c.a, c.b] {
            let v = frame.view(p);
            if frame.perspective && v.z <= near {
                continue;
            }
            let q = frame.proj(&v);
            for k in 0..2 {
                all_min[k] = all_min[k].min(q[k]);
                all_max[k] = all_max[k].max(q[k]);
            }
        }
    }
    let diag = if all_min[0].is_finite() {
        ((all_max[0] - all_min[0]).powi(2) + (all_max[1] - all_min[1]).powi(2))
            .sqrt()
            .max(1e-9)
    } else {
        1.0
    };
    let depth_tol = diag * DEPTH_TOL_REL;
    let min_area = (diag * MIN_AREA_REL).powi(2);
    let min_piece = diag * MIN_PIECE_REL;

    for (t, vs) in &pending {
        let ps = [frame.proj(&vs[0]), frame.proj(&vs[1]), frame.proj(&vs[2])];
        let area2 = (ps[1][0] - ps[0][0]) * (ps[2][1] - ps[0][1])
            - (ps[2][0] - ps[0][0]) * (ps[1][1] - ps[0][1]);
        if area2.abs() * 0.5 < min_area {
            continue;
        }
        // Metric as affine function of projected coords: solve from the three
        // vertices (m_i = k0 + kx x_i + ky y_i).
        let ms = [
            frame.metric(vs[0].z),
            frame.metric(vs[1].z),
            frame.metric(vs[2].z),
        ];
        let det = area2;
        let (x0, y0, x1, y1, x2, y2) = (ps[0][0], ps[0][1], ps[1][0], ps[1][1], ps[2][0], ps[2][1]);
        // Solve [x1-x0 y1-y0; x2-x0 y2-y0] [kx ky]^T = [m1-m0; m2-m0]
        let (dx1, dy1, dx2, dy2) = (x1 - x0, y1 - y0, x2 - x0, y2 - y0);
        let (dm1, dm2) = (ms[1] - ms[0], ms[2] - ms[0]);
        let kx = (dm1 * dy2 - dm2 * dy1) / det;
        let ky = (dx1 * dm2 - dx2 * dm1) / det;
        let k0 = ms[0] - kx * x0 - ky * y0;
        let n = tri_normal(t);
        let c = dot(n, t.a - Point3::ORIGIN);
        occs.push(Occ {
            p: ps,
            min: [x0.min(x1).min(x2), y0.min(y1).min(y2)],
            max: [x0.max(x1).max(x2), y0.max(y1).max(y2)],
            k0,
            kx,
            ky,
            n,
            c,
            sign: if area2 >= 0.0 { 1.0 } else { -1.0 },
        });
    }
    let grid = Grid::new(&occs);

    // ---- 3. per candidate: hidden intervals → pieces
    let mut out: Vec<Seg> = Vec::new();
    let mut stamp = vec![0u32; occs.len()];
    let mut stamp_id = 0u32;
    let mut hits: Vec<u32> = Vec::new();
    let mut hidden: Vec<(f64, f64)> = Vec::new();
    let mut pairs: u64 = 0;
    for c in &cands {
        let (va, vb) = (frame.view(c.a), frame.view(c.b));
        if frame.perspective && (va.z <= near || vb.z <= near) {
            // Crossing the near plane: drop (off-page for any sane framing).
            continue;
        }
        let (pa, pb) = (frame.proj(&va), frame.proj(&vb));
        let len = ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2)).sqrt();
        if len < min_piece {
            continue;
        }
        let (ma, mb) = (frame.metric(va.z), frame.metric(vb.z));
        // Segment plane-distance helper (world space) for the on-plane test.
        let smin = [pa[0].min(pb[0]), pa[1].min(pb[1])];
        let smax = [pa[0].max(pb[0]), pa[1].max(pb[1])];
        stamp_id = stamp_id.wrapping_add(1);
        if stamp_id == 0 {
            stamp.iter_mut().for_each(|s| *s = 0);
            stamp_id = 1;
        }
        grid.query(smin, smax, &mut stamp, stamp_id, &mut hits);
        hidden.clear();
        pairs += hits.len() as u64;
        if pairs > opts.budget {
            return Err(HlrError::TooComplex {
                pairs,
                budget: opts.budget,
            });
        }
        for &oi in &hits {
            let o = &occs[oi as usize];
            // On the occluder's plane (own face, coplanar neighbour): never
            // hidden by it.
            let nlen = o.n.length();
            if nlen > 0.0 {
                let da = (dot(o.n, c.a - Point3::ORIGIN) - o.c) / nlen;
                let db = (dot(o.n, c.b - Point3::ORIGIN) - o.c) / nlen;
                if da.abs() <= depth_tol && db.abs() <= depth_tol {
                    continue;
                }
            }
            // Clip the projected segment's parameter range to the footprint.
            let (mut s0, mut s1) = (0.0f64, 1.0f64);
            let mut inside = true;
            for e in 0..3 {
                let (p, q) = (o.p[e], o.p[(e + 1) % 3]);
                // Half-plane: sign · cross(q - p, x - p) >= 0 is inside.
                let ex = q[0] - p[0];
                let ey = q[1] - p[1];
                let fa = o.sign * (ex * (pa[1] - p[1]) - ey * (pa[0] - p[0]));
                let fb = o.sign * (ex * (pb[1] - p[1]) - ey * (pb[0] - p[0]));
                // f(s) = fa + (fb - fa) s >= 0
                if fa >= 0.0 && fb >= 0.0 {
                    continue;
                }
                if fa < 0.0 && fb < 0.0 {
                    inside = false;
                    break;
                }
                let root = fa / (fa - fb);
                if fa < 0.0 {
                    s0 = s0.max(root);
                } else {
                    s1 = s1.min(root);
                }
                if s0 >= s1 {
                    inside = false;
                    break;
                }
            }
            if !inside || s0 >= s1 {
                continue;
            }
            // Depth comparison on [s0, s1]: g(s) = m_seg(s) - m_plane(s) - thr(s).
            let g = |s: f64| -> f64 {
                let x = pa[0] + (pb[0] - pa[0]) * s;
                let y = pa[1] + (pb[1] - pa[1]) * s;
                let m_seg = ma + (mb - ma) * s;
                let m_plane = o.k0 + o.kx * x + o.ky * y;
                let thr = if frame.perspective {
                    // m = -dist/z → Δm ≈ dist·Δz/z²; z along the segment via 1/z linear.
                    let inv_z = (1.0 / va.z) + ((1.0 / vb.z) - (1.0 / va.z)) * s;
                    frame.dist * depth_tol * inv_z * inv_z
                } else {
                    depth_tol
                };
                m_seg - m_plane - thr
            };
            let (g0, g1) = (g(s0), g(s1));
            match (g0 > 0.0, g1 > 0.0) {
                (true, true) => hidden.push((s0, s1)),
                (false, false) => {}
                (true, false) => {
                    let r = s0 + (s1 - s0) * (g0 / (g0 - g1));
                    hidden.push((s0, r));
                }
                (false, true) => {
                    let r = s0 + (s1 - s0) * (g0 / (g0 - g1));
                    hidden.push((r, s1));
                }
            }
        }
        // Union hidden intervals, subtract from [0,1]. Snap run ends that
        // fall within a piece-length of the segment's ends (see MIN_PIECE_REL).
        let snap = min_piece / len;
        for h in hidden.iter_mut() {
            if h.0 < snap {
                h.0 = 0.0;
            }
            if h.1 > 1.0 - snap {
                h.1 = 1.0;
            }
        }
        hidden.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut merged: Vec<(f64, f64)> = Vec::with_capacity(hidden.len());
        for &(a, b) in &hidden {
            if let Some(last) = merged.last_mut()
                && a <= last.1
            {
                last.1 = last.1.max(b);
                continue;
            }
            merged.push((a, b));
        }
        let emit = |out: &mut Vec<Seg>, s0: f64, s1: f64, kind: Kind| {
            if (s1 - s0) * len < min_piece {
                return;
            }
            let a = [pa[0] + (pb[0] - pa[0]) * s0, pa[1] + (pb[1] - pa[1]) * s0];
            let b = [pa[0] + (pb[0] - pa[0]) * s1, pa[1] + (pb[1] - pa[1]) * s1];
            out.push(Seg {
                a,
                b,
                kind,
                sid: c.sid,
            });
        };
        let mut cursor = 0.0;
        for &(a, b) in &merged {
            if a > cursor {
                emit(&mut out, cursor, a, c.kind);
            }
            if opts.include_hidden {
                emit(&mut out, a.max(cursor), b, Kind::Hidden);
            }
            cursor = cursor.max(b);
        }
        if cursor < 1.0 {
            emit(&mut out, cursor, 1.0, c.kind);
        }
    }

    let bounds = if out.is_empty() {
        None
    } else {
        let mut mn = [f64::INFINITY; 2];
        let mut mx = [f64::NEG_INFINITY; 2];
        for s in &out {
            for p in [s.a, s.b] {
                for k in 0..2 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
        }
        Some((mn, mx))
    };
    Ok(LineDrawing { segs: out, bounds })
}

#[inline]
fn edge_key(a: [u32; 3], b: [u32; 3]) -> [u32; 6] {
    if a <= b {
        [a[0], a[1], a[2], b[0], b[1], b[2]]
    } else {
        [b[0], b[1], b[2], a[0], a[1], a[2]]
    }
}
