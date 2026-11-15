//! The convex clip (WP05 §7.3): a convex polyhedral cell, starting as a
//! cube about a vertex, clipped by half-spaces — the incident faces and the
//! blend strips' facet planes. At an all-convex vertex the clipped cell is
//! the blended solid near the vertex (an all-concave vertex clips the air
//! instead), so one algorithm covers run-outs at any valence, non-prismatic
//! miters and chamfer apexes.

use std::collections::BTreeMap;

use crate::ids::{EdgeId, FaceId};
use crate::math::{Plane, Point3, Vec3};
use crate::tol;

/// What produced a cell face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CellTag {
    /// One of the six faces of the initial cube.
    Box(u8),
    /// An incident face of the object.
    Face(FaceId),
    /// Facet `k` of the strip of `edge`.
    Facet { edge: EdgeId, k: usize },
    /// An extra plane (the flat chamfer corner).
    Extra(u8),
}

/// A face of a [`ConvexCell`]: its tag, its plane (outward), and its vertex
/// loop (CCW seen from outside).
#[derive(Debug, Clone)]
pub(crate) struct CellFace {
    pub tag: CellTag,
    pub plane: Plane,
    pub verts: Vec<usize>,
}

/// A convex polyhedron.
#[derive(Debug, Clone)]
pub(crate) struct ConvexCell {
    pub verts: Vec<Point3>,
    pub faces: Vec<CellFace>,
}

/// The clip removed the whole cell.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClipEmpty;

impl ConvexCell {
    /// The axis-aligned cube of half-size `half` about `center`, outward
    /// planes.
    pub(crate) fn cube(center: Point3, half: f64) -> ConvexCell {
        let p = |x: f64, y: f64, z: f64| {
            Point3::new(
                center.x + x * half,
                center.y + y * half,
                center.z + z * half,
            )
        };
        let verts = vec![
            p(-1.0, -1.0, -1.0),
            p(1.0, -1.0, -1.0),
            p(1.0, 1.0, -1.0),
            p(-1.0, 1.0, -1.0),
            p(-1.0, -1.0, 1.0),
            p(1.0, -1.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(-1.0, 1.0, 1.0),
        ];
        let quads: [([usize; 4], Vec3); 6] = [
            ([0, 3, 2, 1], Vec3::new(0.0, 0.0, -1.0)),
            ([4, 5, 6, 7], Vec3::new(0.0, 0.0, 1.0)),
            ([0, 1, 5, 4], Vec3::new(0.0, -1.0, 0.0)),
            ([2, 3, 7, 6], Vec3::new(0.0, 1.0, 0.0)),
            ([1, 2, 6, 5], Vec3::new(1.0, 0.0, 0.0)),
            ([3, 0, 4, 7], Vec3::new(-1.0, 0.0, 0.0)),
        ];
        let faces = quads
            .iter()
            .enumerate()
            .map(|(i, (q, n))| CellFace {
                tag: CellTag::Box(i as u8),
                plane: Plane::from_point_normal(verts[q[0]], *n).expect("unit axis normal"),
                verts: q.to_vec(),
            })
            .collect();
        ConvexCell { verts, faces }
    }

    /// Keeps the half-space `plane.signed_distance(p) ≤ 0`, adding the cap
    /// face tagged `tag`. Vertices within `PLANE_DIST` of the plane count as
    /// on it; a crossing point is created once per cell edge; the cap is
    /// ordered CCW about the plane normal; vertices within `POINT_MERGE`
    /// merge (smallest index wins) and faces left with fewer than three
    /// vertices drop. A plane that removes nothing adds no cap.
    pub(crate) fn clip(&mut self, plane: &Plane, tag: CellTag) -> Result<(), ClipEmpty> {
        let d: Vec<f64> = self
            .verts
            .iter()
            .map(|&p| plane.signed_distance(p))
            .collect();
        let used: Vec<usize> = {
            let mut u: Vec<usize> = self.faces.iter().flat_map(|f| f.verts.clone()).collect();
            u.sort_unstable();
            u.dedup();
            u
        };
        let out = |i: usize| d[i] > tol::PLANE_DIST;
        let inn = |i: usize| d[i] < -tol::PLANE_DIST;
        if !used.iter().any(|&i| out(i)) {
            return Ok(());
        }
        if !used.iter().any(|&i| inn(i)) {
            return Err(ClipEmpty);
        }
        let mut crossing: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        let mut on_cap: Vec<usize> = Vec::new();
        let mut new_faces = Vec::with_capacity(self.faces.len() + 1);
        for f in &self.faces {
            let n = f.verts.len();
            let mut poly: Vec<usize> = Vec::with_capacity(n + 2);
            for k in 0..n {
                let (a, b) = (f.verts[k], f.verts[(k + 1) % n]);
                if !out(a) {
                    poly.push(a);
                    if !inn(a) {
                        on_cap.push(a);
                    }
                }
                if (inn(a) && out(b)) || (out(a) && inn(b)) {
                    let key = (a.min(b), a.max(b));
                    let idx = *crossing.entry(key).or_insert_with(|| {
                        let (pa, pb) = (self.verts[a], self.verts[b]);
                        let s = d[a] / (d[a] - d[b]);
                        self.verts.push(pa + (pb - pa) * s);
                        self.verts.len() - 1
                    });
                    poly.push(idx);
                    on_cap.push(idx);
                }
            }
            if poly.len() >= 3 {
                new_faces.push(CellFace {
                    tag: f.tag,
                    plane: f.plane,
                    verts: poly,
                });
            }
        }
        // Cap: every vertex now on the plane, CCW about its normal.
        on_cap.sort_unstable();
        on_cap.dedup();
        if on_cap.len() >= 3 {
            let (u, v) = crate::triangulate::plane_basis(plane.normal());
            let c = on_cap
                .iter()
                .fold(Vec3::new(0.0, 0.0, 0.0), |s, &i| s + self.verts[i].to_vec())
                / on_cap.len() as f64;
            let ang = |i: usize| {
                let r = self.verts[i].to_vec() - c;
                r.dot(v).atan2(r.dot(u))
            };
            on_cap.sort_by(|&a, &b| ang(a).total_cmp(&ang(b)).then(a.cmp(&b)));
            new_faces.push(CellFace {
                tag,
                plane: *plane,
                verts: on_cap,
            });
        }
        self.faces = new_faces;
        self.merge();
        if self.faces.len() < 4 {
            return Err(ClipEmpty);
        }
        Ok(())
    }

    /// Merges vertices within `POINT_MERGE` (union-find, smallest index
    /// wins), rewrites loops, drops repeated consecutive indices and faces
    /// left with fewer than three vertices.
    fn merge(&mut self) {
        let n = self.verts.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while p[r] != r {
                r = p[r];
            }
            let mut j = i;
            while p[j] != r {
                let nx = p[j];
                p[j] = r;
                j = nx;
            }
            r
        }
        let mut used: Vec<usize> = self.faces.iter().flat_map(|f| f.verts.clone()).collect();
        used.sort_unstable();
        used.dedup();
        for (x, &i) in used.iter().enumerate() {
            for &j in &used[x + 1..] {
                if self.verts[i].approx_eq(self.verts[j], tol::POINT_MERGE) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        let (lo, hi) = (ri.min(rj), ri.max(rj));
                        parent[hi] = lo;
                    }
                }
            }
        }
        for f in &mut self.faces {
            let mapped: Vec<usize> = f.verts.iter().map(|&i| find(&mut parent, i)).collect();
            let mut out: Vec<usize> = Vec::with_capacity(mapped.len());
            for &i in &mapped {
                if out.last() != Some(&i) {
                    out.push(i);
                }
            }
            while out.len() > 1 && out.first() == out.last() {
                out.pop();
            }
            f.verts = out;
        }
        self.faces.retain(|f| f.verts.len() >= 3);
    }

    /// Whether vertex `i` lies on one of the initial cube's planes.
    pub(crate) fn on_box(&self, i: usize, boxes: &[Plane]) -> bool {
        boxes
            .iter()
            .any(|b| b.signed_distance(self.verts[i]).abs() <= tol::PLANE_DIST)
    }
}
