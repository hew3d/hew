//! Catmull–Clark subdivision with sharp creases (true-curves.md §6.1): the
//! smoothed surface of an object's cage at Smooth levels 1–3, for display,
//! export and Bake.

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{FaceId, VertexId};
use crate::math::{Point3, Vec3};
use crate::shape_error::ShapeError;
use crate::topo::Object;

/// The most faces a subdivision may produce (`faces × 4^level`).
pub const MAX_SUBDIV_FACES: usize = 4_000_000;

/// A subdivided mesh: polygon faces over shared points.
#[derive(Debug, Clone, PartialEq)]
pub struct SubdivMesh {
    /// The points.
    pub points: Vec<Point3>,
    /// Faces as point-index loops, counter-clockwise from outside (quads
    /// after level ≥ 1).
    pub faces: Vec<Vec<u32>>,
    /// For each face: the cage face it descends from.
    pub source: Vec<FaceId>,
    /// Crease edges as unordered point pairs `(min, max)`.
    pub creases: BTreeSet<(u32, u32)>,
}

fn key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

fn mid(a: Point3, b: Point3) -> Point3 {
    Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0)
}

fn average(pts: impl Iterator<Item = Point3>) -> Point3 {
    let (mut s, mut n) = (Vec3::new(0.0, 0.0, 0.0), 0.0);
    for p in pts {
        s = s + p.to_vec();
        n += 1.0;
    }
    Point3::new(s.x / n, s.y / n, s.z / n)
}

/// The level-0 mesh of a cage: its vertices in slot order, each face's
/// outer loop (a face with holes split into triangles), its crease edges
/// and every boundary edge as creases.
fn cage_mesh(cage: &Object) -> Result<SubdivMesh, ShapeError> {
    let mut index: BTreeMap<VertexId, u32> = BTreeMap::new();
    let mut points = Vec::with_capacity(cage.vertices().len());
    for (v, vert) in cage.vertices() {
        index.insert(v, points.len() as u32);
        points.push(vert.position);
    }
    let ring = |l| -> Vec<u32> {
        cage.loop_half_edges(l)
            .map(|h| index[&cage.half_edges()[h].origin])
            .collect()
    };
    let mut faces = Vec::new();
    let mut source = Vec::new();
    for (f, face) in cage.faces() {
        let outer = ring(face.outer_loop);
        if face.inner_loops.is_empty() {
            faces.push(outer);
            source.push(f);
            continue;
        }
        let holes: Vec<Vec<u32>> = face.inner_loops.iter().map(|&l| ring(l)).collect();
        let all: Vec<u32> = outer
            .iter()
            .chain(holes.iter().flatten())
            .copied()
            .collect();
        let pos = |r: &[u32]| r.iter().map(|&i| points[i as usize]).collect::<Vec<_>>();
        let hole_pts: Vec<Vec<Point3>> = holes.iter().map(|h| pos(h)).collect();
        let tris =
            crate::triangulate::triangulate_face(&pos(&outer), &hole_pts, face.plane.normal())
                .map_err(|_| ShapeError::ShapeDegenerate)?;
        for [a, b, c] in tris {
            faces.push(vec![all[a], all[b], all[c]]);
            source.push(f);
        }
    }
    let mut creases = BTreeSet::new();
    for (e, edge) in cage.edges() {
        if edge.crease || edge.twin_half_edge.is_none() {
            let (a, b) = cage.edge_vertex_ids(e).ok_or(ShapeError::ShapeDegenerate)?;
            creases.insert(key(index[&a], index[&b]));
        }
    }
    Ok(SubdivMesh {
        points,
        faces,
        source,
        creases,
    })
}

/// One Catmull–Clark step with sharp creases.
fn step(m: &SubdivMesh) -> Result<SubdivMesh, ShapeError> {
    let np = m.points.len();
    // Edge → incident faces.
    let mut edge_faces: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for k in 0..f.len() {
            edge_faces
                .entry(key(f[k], f[(k + 1) % f.len()]))
                .or_default()
                .push(fi);
        }
    }
    if edge_faces.values().any(|fs| fs.len() > 2) {
        return Err(ShapeError::ShapeDegenerate);
    }
    let face_pts: Vec<Point3> = m
        .faces
        .iter()
        .map(|f| average(f.iter().map(|&i| m.points[i as usize])))
        .collect();
    let sharp = |e: &(u32, u32), fs: &[usize]| m.creases.contains(e) || fs.len() < 2;
    // Edge points, in edge-key order.
    let mut edge_index: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut edge_pts = Vec::with_capacity(edge_faces.len());
    for (e, fs) in &edge_faces {
        let (a, b) = (m.points[e.0 as usize], m.points[e.1 as usize]);
        let p = if sharp(e, fs) {
            mid(a, b)
        } else {
            average([a, b, face_pts[fs[0]], face_pts[fs[1]]].into_iter())
        };
        edge_index.insert(*e, (np + edge_pts.len()) as u32);
        edge_pts.push(p);
    }
    // Vertex points.
    let mut v_faces: Vec<Vec<usize>> = vec![Vec::new(); np];
    for (fi, f) in m.faces.iter().enumerate() {
        for &i in f {
            v_faces[i as usize].push(fi);
        }
    }
    let mut v_edges: Vec<Vec<(u32, u32)>> = vec![Vec::new(); np];
    for e in edge_faces.keys() {
        v_edges[e.0 as usize].push(*e);
        v_edges[e.1 as usize].push(*e);
    }
    let mut vert_pts = Vec::with_capacity(np);
    for v in 0..np {
        let p = m.points[v];
        let edges = &v_edges[v];
        if edges.is_empty() {
            vert_pts.push(p);
            continue;
        }
        let creased: Vec<&(u32, u32)> = edges.iter().filter(|e| sharp(e, &edge_faces[e])).collect();
        let other = |e: &(u32, u32)| if e.0 as usize == v { e.1 } else { e.0 };
        let q = match creased.len() {
            0 | 1 => {
                let n = edges.len() as f64;
                let fq = average(v_faces[v].iter().map(|&f| face_pts[f]));
                let r = average(edges.iter().map(|e| mid(p, m.points[other(e) as usize])));
                let s = p.to_vec();
                let sum = fq.to_vec() + r.to_vec() * 2.0 + s * (n - 3.0);
                Point3::new(sum.x / n, sum.y / n, sum.z / n)
            }
            2 => {
                let (a, b) = (
                    m.points[other(creased[0]) as usize],
                    m.points[other(creased[1]) as usize],
                );
                let sum = a.to_vec() + p.to_vec() * 6.0 + b.to_vec();
                Point3::new(sum.x / 8.0, sum.y / 8.0, sum.z / 8.0)
            }
            _ => p,
        };
        vert_pts.push(q);
    }
    let base_f = (np + edge_pts.len()) as u32;
    let mut faces = Vec::with_capacity(m.faces.len() * 4);
    let mut source = Vec::with_capacity(m.faces.len() * 4);
    for (fi, f) in m.faces.iter().enumerate() {
        let k = f.len();
        let fp = base_f + fi as u32;
        for i in 0..k {
            let e_next = edge_index[&key(f[i], f[(i + 1) % k])];
            let e_prev = edge_index[&key(f[(i + k - 1) % k], f[i])];
            faces.push(vec![f[i], e_next, fp, e_prev]);
            source.push(m.source[fi]);
        }
    }
    let mut creases = BTreeSet::new();
    for e in &m.creases {
        if let Some(&ep) = edge_index.get(e) {
            creases.insert(key(e.0, ep));
            creases.insert(key(ep, e.1));
        }
    }
    // Boundary edges stay sharp at every level.
    for (e, fs) in &edge_faces {
        if fs.len() < 2 {
            let ep = edge_index[e];
            creases.insert(key(e.0, ep));
            creases.insert(key(ep, e.1));
        }
    }
    let mut points = vert_pts;
    points.extend(edge_pts);
    points.extend(face_pts);
    Ok(SubdivMesh {
        points,
        faces,
        source,
        creases,
    })
}

/// How many faces `cage` subdivides into at `level` without building it:
/// the first step turns each `k`-sided face into `k` quads (a face with
/// holes first splits into `n + 2h − 2` triangles, for `n` loop vertices
/// and `h` holes), and every later step quadruples the count.
pub fn subdivided_face_count(cage: &Object, level: u8) -> usize {
    if level == 0 {
        return cage.faces().len();
    }
    let first: usize = cage
        .faces()
        .values()
        .map(|f| {
            let n: usize = std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .map(|l| cage.loop_half_edges(l).count())
                .sum();
            if f.inner_loops.is_empty() {
                n
            } else {
                3 * (n + 2 * f.inner_loops.len() - 2)
            }
        })
        .sum();
    first.saturating_mul(4usize.saturating_pow(u32::from(level) - 1))
}

/// Subdivides the cage `level` times (1..=3). Pure and deterministic.
///
/// # Errors
/// `AmountOutOfRange` for a level outside 1..=3 or a result over
/// [`MAX_SUBDIV_FACES`] faces; `ShapeDegenerate` for a non-manifold cage
/// or a holed face that cannot be split.
pub fn subdivide(cage: &Object, level: u8) -> Result<SubdivMesh, ShapeError> {
    if !(1..=3).contains(&level) {
        return Err(ShapeError::AmountOutOfRange);
    }
    if subdivided_face_count(cage, level) > MAX_SUBDIV_FACES {
        return Err(ShapeError::AmountOutOfRange);
    }
    let mut m = cage_mesh(cage)?;
    for _ in 0..level {
        m = step(&m)?;
    }
    Ok(m)
}

/// The Newell (area-weighted) normal of a face.
fn newell(m: &SubdivMesh, f: &[u32]) -> Vec3 {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for k in 0..f.len() {
        let (a, b) = (
            m.points[f[k] as usize],
            m.points[f[(k + 1) % f.len()] as usize],
        );
        n = n + Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    n
}

/// Per-corner smooth normals: at each point, the area-weighted average of
/// the face normals of the corners it shares a non-crease edge with
/// (transitively), so normals split across creases. `out[i][k]` is face
/// `i`'s corner `k`.
pub fn corner_normals(mesh: &SubdivMesh) -> Vec<Vec<Vec3>> {
    let fnorm: Vec<Vec3> = mesh.faces.iter().map(|f| newell(mesh, f)).collect();
    // Corners grouped by point; corners of two faces sharing a non-crease
    // edge through the point join one group (union-find over corners).
    let mut corner_id: Vec<Vec<usize>> = Vec::with_capacity(mesh.faces.len());
    let mut corners: Vec<(usize, usize)> = Vec::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        corner_id.push(
            (0..f.len())
                .map(|k| {
                    corners.push((fi, k));
                    corners.len() - 1
                })
                .collect(),
        );
    }
    let mut parent: Vec<usize> = (0..corners.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    // A directed edge (a→b) in face f: corner of a in f.
    let mut by_edge: BTreeMap<(u32, u32), Vec<(usize, usize)>> = BTreeMap::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        let n = f.len();
        for k in 0..n {
            let (a, b) = (f[k], f[(k + 1) % n]);
            // Both corners touching this edge: a's (k) and b's (k + 1).
            by_edge
                .entry(key(a, b))
                .or_default()
                .push((corner_id[fi][k], corner_id[fi][(k + 1) % n]));
        }
    }
    for (e, list) in &by_edge {
        if list.len() != 2 || mesh.creases.contains(e) {
            continue;
        }
        // Join the corners at the same point across the edge.
        let point_of = |c: usize| {
            let (fi, k) = corners[c];
            mesh.faces[fi][k]
        };
        let (x, y) = (list[0], list[1]);
        for cx in [x.0, x.1] {
            for cy in [y.0, y.1] {
                if point_of(cx) == point_of(cy) {
                    let (rx, ry) = (find(&mut parent, cx), find(&mut parent, cy));
                    if rx != ry {
                        parent[rx.max(ry)] = rx.min(ry);
                    }
                }
            }
        }
    }
    let mut sums: BTreeMap<usize, Vec3> = BTreeMap::new();
    for (c, &(fi, _)) in corners.iter().enumerate() {
        let r = find(&mut parent, c);
        let s = sums.entry(r).or_insert(Vec3::new(0.0, 0.0, 0.0));
        *s = *s + fnorm[fi];
    }
    corner_id
        .iter()
        .map(|ids| {
            ids.iter()
                .map(|&c| {
                    let r = find(&mut parent, c);
                    sums[&r].normalized().unwrap_or(Vec3::new(0.0, 0.0, 1.0))
                })
                .collect()
        })
        .collect()
}
