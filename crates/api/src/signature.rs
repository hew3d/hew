//! Shape signatures: whether two Objects are the same part or mirrored
//! twins (`hew.query.signature`; docs/agents/PLUGINS.md "Same part,
//! mirrored part"). Pure over a [`Document`].
//!
//! The part's face frame (`l ≥ w ≥ t`, right-handed) fixes three axis
//! lines. Every rotation that maps that axis set onto itself and keeps the
//! extent order is a candidate (four sign choices with det +1, more when
//! extents tie); each expresses the part's vertices about its box center in
//! that frame, quantized to 1 µm, lists each face as its loops (outer loop
//! first, every loop started at its smallest vertex, holes sorted) with the
//! face's surface kind and parameters, sorts the faces, and hashes the
//! result with SHA-256. `shape` is the smallest hash over the det +1
//! candidates and `mirror` the smallest over the det −1 ones, whose loops
//! are read reversed so they run counter-clockwise about the mirrored
//! outward normal, exactly as the mirror image's own loops do.
//!
//! True Curves marks are part of the shape: a face's blend tag (and what
//! its blend's record asks for) follows its loops, and the creased edges,
//! the Smooth level, and the edges' circle and polygon claims are records
//! of their own. Each is written only when present, so a shape without
//! them hashes as it always did. These are the facts
//! `Document::join_linked` compares beyond the vertex set (every face's
//! surface claim, every edge claim, every mark), so two shapes the
//! signature calls the same link, and two it tells apart do not.

use crate::part_frame::{self, PartFrame};
use crate::scope::Scope;
use kernel::{Document, DocumentError, NodeId, Occurrence, Point3, Vec3};
use sha2::{Digest, Sha256};

/// One micrometer, the quantum every coordinate is rounded to.
const QUANTUM: f64 = 1e-6;

/// Relative tolerance under which two extents count as tied.
const EXTENT_TIE: f64 = 1e-9;

/// `hew.query.signature`'s answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    /// Lowercase hex SHA-256, 64 characters.
    pub shape: String,
    /// The same over the reflections.
    pub mirror: String,
    /// The part's face frame.
    pub frame: PartFrame,
}

/// The signature of one Object placement, or `None` when it has no face.
///
/// # Errors
/// [`DocumentError::TooManyOccurrences`] — the walk is past the cap.
pub fn signature(doc: &Document, occ: &Occurrence) -> Result<Option<Signature>, DocumentError> {
    let Some((hashes, frame)) = candidate_hashes(doc, occ)? else {
        return Ok(None);
    };
    let min_of = |want_positive: bool| {
        hashes
            .iter()
            .filter(|(_, det)| (*det > 0) == want_positive)
            .map(|(h, _)| h)
            .min()
            .cloned()
    };
    let (Some(shape), Some(mirror)) = (min_of(true), min_of(false)) else {
        return Ok(None);
    };
    Ok(Some(Signature {
        shape,
        mirror,
        frame,
    }))
}

/// A candidate's hash with its determinant's sign.
type CandidateHash = (String, i32);

/// Every candidate's hash with its determinant, and the part's face frame;
/// [`signature`] takes the smallest of each sign.
fn candidate_hashes(
    doc: &Document,
    occ: &Occurrence,
) -> Result<Option<(Vec<CandidateHash>, PartFrame)>, DocumentError> {
    let NodeId::Object(id) = occ.node else {
        return Ok(None);
    };
    let leaves = crate::scope::leaves_for(doc, occ, Scope::All)?;
    let Some(frame) = part_frame::face_frame(doc, &leaves) else {
        return Ok(None);
    };
    let Some(obj) = doc.object(id) else {
        return Ok(None);
    };
    let pose = occ.pose;
    let pose_mirrored = pose.determinant() < 0.0;
    let basis = [frame.x, frame.y, frame.z];
    let center = frame.origin
        + basis[0] * (frame.extents[0] * 0.5)
        + basis[1] * (frame.extents[1] * 0.5)
        + basis[2] * (frame.extents[2] * 0.5);

    // Every face as world-space loops reading counter-clockwise about the
    // outward normal, with its surface.
    struct WorldFace {
        loops: Vec<Vec<Point3>>,
        surface: Option<WorldSurface>,
        /// The face's blend tag (kind, role) and what its blend's record
        /// asks for (empty for none), as bytes.
        blend: Option<(i64, i64, Vec<u8>)>,
    }
    /// A face's analytic claim in world space (meters, radians).
    enum WorldSurface {
        Cylinder(Vec3, f64),
        Sphere(Point3, f64),
        Torus(Point3, Vec3, f64, f64),
        Cone(Point3, Vec3, f64),
    }
    let mut faces = Vec::with_capacity(obj.faces().len());
    for f in obj.faces().values() {
        let mut loops = Vec::with_capacity(1 + f.inner_loops.len());
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let mut ring: Vec<Point3> =
                obj.loop_positions(l).map(|p| pose.apply_point(p)).collect();
            if pose_mirrored {
                ring.reverse();
            }
            loops.push(ring);
        }
        let surface = match (f.surface, pose.similarity_scale()) {
            (Some(kernel::SurfaceRef::Cylinder { axis, radius, .. }), Some(s)) => {
                Some(WorldSurface::Cylinder(pose.apply_vector(axis), radius * s))
            }
            (Some(kernel::SurfaceRef::Sphere { center, radius }), Some(s)) => {
                Some(WorldSurface::Sphere(pose.apply_point(center), radius * s))
            }
            (
                Some(kernel::SurfaceRef::Torus {
                    center,
                    axis,
                    major_radius,
                    minor_radius,
                }),
                Some(s),
            ) => Some(WorldSurface::Torus(
                pose.apply_point(center),
                pose.apply_vector(axis),
                major_radius * s,
                minor_radius * s,
            )),
            (
                Some(kernel::SurfaceRef::Cone {
                    apex,
                    axis,
                    half_angle,
                }),
                Some(_),
            ) => Some(WorldSurface::Cone(
                pose.apply_point(apex),
                pose.apply_vector(axis),
                half_angle,
            )),
            _ => None,
        };
        let blend = f.blend.map(|t| {
            let kind = match t.kind {
                kernel::BlendKind::Fillet => 0,
                kernel::BlendKind::Chamfer => 1,
                kernel::BlendKind::Profile => 2,
            };
            let role = match t.role {
                kernel::BlendRole::Strip => 0,
                kernel::BlendRole::Corner => 1,
                kernel::BlendRole::Runout => 2,
            };
            let digest = obj.blend_record_digest(t.id).unwrap_or_default();
            (kind, role, digest.into_bytes())
        });
        faces.push(WorldFace {
            loops,
            surface,
            blend,
        });
    }
    // Creased edges, world space; and the Smooth level. Both are written
    // only when present, so a shape without either hashes as it did.
    let creases: Vec<(Point3, Point3)> = obj
        .edges()
        .keys()
        .filter(|&e| obj.edges()[e].crease)
        .filter_map(|e| obj.edge_endpoints(e))
        .map(|(a, b)| (pose.apply_point(a), pose.apply_point(b)))
        .collect();
    let smooth = i64::from(doc.smooth_level(id));
    // Edges' circle and polygon claims, world space (ends, center, radius),
    // written only when present. A pose that is not a similarity cannot
    // carry a circle, so the claims drop there, as a face's surface does.
    let scale = pose.similarity_scale();
    let edge_claims = |claim: fn(&kernel::topo::Edge) -> Option<kernel::CurveGeom>| {
        let Some(s) = scale else {
            return Vec::new();
        };
        obj.edges()
            .iter()
            .filter_map(|(e, edge)| {
                let g = claim(edge)?;
                let (a, b) = obj.edge_endpoints(e)?;
                Some((
                    pose.apply_point(a),
                    pose.apply_point(b),
                    pose.apply_point(g.center),
                    g.radius * s,
                ))
            })
            .collect::<Vec<(Point3, Point3, Point3, f64)>>()
    };
    let circle_edges = edge_claims(|e| e.curve);
    let polygon_edges = edge_claims(|e| e.polygon);

    let mut hashes: Vec<CandidateHash> = Vec::new();
    for (rot, det) in candidates(frame.extents) {
        // The candidate's axes in world space: row k is Σ rot[k][j] · basis[j].
        let axes: [Vec3; 3] = std::array::from_fn(|k| {
            basis[0] * rot[k][0] + basis[1] * rot[k][1] + basis[2] * rot[k][2]
        });
        let q = |p: Point3| -> [i64; 3] {
            let d = p - center;
            std::array::from_fn(|k| (d.dot(axes[k]) / QUANTUM).round() as i64)
        };
        let mut records: Vec<Vec<i64>> = Vec::with_capacity(faces.len());
        for face in &faces {
            let mut loops: Vec<Vec<[i64; 3]>> = face
                .loops
                .iter()
                .map(|ring| {
                    let mut v: Vec<[i64; 3]> = ring.iter().map(|&p| q(p)).collect();
                    if det < 0 {
                        v.reverse();
                    }
                    canonical_start(v)
                })
                .collect();
            let holes = &mut loops[1..];
            holes.sort();
            let mut rec: Vec<i64> = Vec::new();
            // An axis in the candidate frame, quantized; a line (`signed`
            // false) has no sign, so its first nonzero component is made
            // positive.
            let axis_q = |axis: Vec3, signed: bool| -> [i64; 3] {
                let a: [f64; 3] = std::array::from_fn(|k| axis.dot(axes[k]));
                let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
                    .sqrt()
                    .max(f64::MIN_POSITIVE);
                let mut a: [i64; 3] =
                    std::array::from_fn(|k| (a[k] / len / QUANTUM).round() as i64);
                if !signed && a.iter().find(|&&c| c != 0).is_some_and(|&c| c < 0) {
                    a = a.map(|c| -c);
                }
                a
            };
            let len_q = |x: f64| (x / QUANTUM).round() as i64;
            match face.surface {
                None => rec.push(0),
                Some(WorldSurface::Cylinder(axis, radius)) => {
                    rec.push(1);
                    rec.extend(axis_q(axis, false));
                    rec.push(len_q(radius));
                }
                Some(WorldSurface::Sphere(center, radius)) => {
                    rec.push(2);
                    rec.extend(q(center));
                    rec.push(len_q(radius));
                }
                Some(WorldSurface::Torus(center, axis, major, minor)) => {
                    rec.push(3);
                    rec.extend(q(center));
                    rec.extend(axis_q(axis, false));
                    rec.push(len_q(major));
                    rec.push(len_q(minor));
                }
                Some(WorldSurface::Cone(apex, axis, half_angle)) => {
                    rec.push(4);
                    rec.extend(q(apex));
                    rec.extend(axis_q(axis, true));
                    rec.push(len_q(half_angle));
                }
            }
            rec.push(loops.len() as i64);
            for l in &loops {
                rec.push(l.len() as i64);
                for v in l {
                    rec.extend(v);
                }
            }
            // A blend tag (and its record) after the loops, only when the
            // face has one.
            if let Some((kind, role, digest)) = &face.blend {
                rec.extend([-1, *kind, *role, digest.len() as i64]);
                rec.extend(digest.iter().map(|&b| i64::from(b)));
            }
            records.push(rec);
        }
        // One record for the creased edges (each a sorted pair of
        // quantized ends, sorted), one for the Smooth level; both absent
        // when there is nothing to say, and led by markers no face record
        // starts with (a face record starts with its surface kind, 0 to 4).
        if !creases.is_empty() {
            let mut pairs: Vec<[[i64; 3]; 2]> = creases
                .iter()
                .map(|&(a, b)| {
                    let (qa, qb) = (q(a), q(b));
                    if qa <= qb { [qa, qb] } else { [qb, qa] }
                })
                .collect();
            pairs.sort();
            let mut rec = vec![-2, pairs.len() as i64];
            for [a, b] in pairs {
                rec.extend(a);
                rec.extend(b);
            }
            records.push(rec);
        }
        if smooth > 0 {
            records.push(vec![-3, smooth]);
        }
        // One record each for the circle- and polygon-claimed edges (each
        // a sorted pair of quantized ends, then the quantized center and
        // radius; sorted), led by markers of their own.
        for (marker, claimed) in [(-4, &circle_edges), (-5, &polygon_edges)] {
            if claimed.is_empty() {
                continue;
            }
            let mut entries: Vec<[i64; 10]> = claimed
                .iter()
                .map(|&(a, b, c, r)| {
                    let (qa, qb) = (q(a), q(b));
                    let [lo, hi] = if qa <= qb { [qa, qb] } else { [qb, qa] };
                    let qc = q(c);
                    [
                        lo[0],
                        lo[1],
                        lo[2],
                        hi[0],
                        hi[1],
                        hi[2],
                        qc[0],
                        qc[1],
                        qc[2],
                        (r / QUANTUM).round() as i64,
                    ]
                })
                .collect();
            entries.sort();
            let mut rec = vec![marker, entries.len() as i64];
            for e in entries {
                rec.extend(e);
            }
            records.push(rec);
        }
        records.sort();
        let mut h = Sha256::new();
        h.update((records.len() as u64).to_le_bytes());
        for rec in &records {
            h.update((rec.len() as u64).to_le_bytes());
            for x in rec {
                h.update(x.to_le_bytes());
            }
        }
        let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        hashes.push((hex, det));
    }
    Ok(Some((hashes, frame)))
}

/// The loop rotated to start at its smallest vertex.
fn canonical_start(mut v: Vec<[i64; 3]>) -> Vec<[i64; 3]> {
    if let Some((k, _)) = v.iter().enumerate().min_by_key(|(_, p)| **p) {
        v.rotate_left(k);
    }
    v
}

/// Every signed permutation matrix that keeps the extent order (a
/// permutation only moves an axis onto one whose extent ties with it), with
/// its determinant.
fn candidates(extents: [f64; 3]) -> Vec<([[f64; 3]; 3], i32)> {
    let size = extents
        .iter()
        .fold(0.0_f64, |a, &e| a.max(e))
        .max(f64::MIN_POSITIVE);
    let tied = |a: usize, b: usize| (extents[a] - extents[b]).abs() <= EXTENT_TIE * size;
    const PERMS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut out = Vec::new();
    for perm in PERMS {
        if !(0..3).all(|k| perm[k] == k || tied(k, perm[k])) {
            continue;
        }
        let parity = match perm {
            [0, 1, 2] | [1, 2, 0] | [2, 0, 1] => 1,
            _ => -1,
        };
        for signs in 0..8u8 {
            let s: [f64; 3] = std::array::from_fn(|k| if signs >> k & 1 == 1 { -1.0 } else { 1.0 });
            let mut m = [[0.0; 3]; 3];
            for k in 0..3 {
                m[k][perm[k]] = s[k];
            }
            let det = parity * (s[0] * s[1] * s[2]) as i32;
            out.push((m, det));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{candidate_hashes, candidates, signature};
    use kernel::{Document, NodeId, Plane, Point3};

    /// A notched stick: its candidate hashes all differ, so the minimum is
    /// observable.
    fn notched(doc: &mut Document) -> kernel::ObjectId {
        let plane = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let s = doc.add_sketch(plane);
        let pts = [
            (0.0, 0.0),
            (0.5, 0.0),
            (0.5, 0.05),
            (0.3, 0.05),
            (0.3, 0.04),
            (0.2, 0.04),
            (0.2, 0.05),
            (0.0, 0.05),
        ];
        let sk = doc.sketch_mut(s).unwrap();
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, regions[0], 0.05).unwrap().0
    }

    #[test]
    fn the_answer_is_the_minimum_over_every_candidate_of_its_sign() {
        let mut doc = Document::new();
        let o = notched(&mut doc);
        let occ = doc.occurrence_at(&[], NodeId::Object(o)).unwrap();
        let (hashes, _) = candidate_hashes(&doc, &occ).unwrap().unwrap();
        let rot: Vec<_> = hashes.iter().filter(|(_, d)| *d > 0).collect();
        let refl: Vec<_> = hashes.iter().filter(|(_, d)| *d < 0).collect();
        // The hashes differ, so a "first candidate" pick would not pass.
        assert!(rot.iter().any(|(h, _)| *h != rot[0].0));
        assert!(refl.iter().any(|(h, _)| *h != refl[0].0));
        let sig = signature(&doc, &occ).unwrap().unwrap();
        assert_eq!(&sig.shape, rot.iter().map(|(h, _)| h).min().unwrap());
        assert_eq!(&sig.mirror, refl.iter().map(|(h, _)| h).min().unwrap());
    }

    #[test]
    fn distinct_extents_give_four_rotations_and_four_reflections() {
        let c = candidates([3.0, 2.0, 1.0]);
        assert_eq!(c.iter().filter(|(_, d)| *d > 0).count(), 4);
        assert_eq!(c.iter().filter(|(_, d)| *d < 0).count(), 4);
    }

    #[test]
    fn a_tie_doubles_the_candidates_and_a_cube_has_all_48() {
        assert_eq!(candidates([2.0, 2.0, 1.0]).len(), 16);
        assert_eq!(candidates([1.0, 1.0, 1.0]).len(), 48);
    }
}
