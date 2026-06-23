//! Geometry healing for mesh import (shared by `dae-import`  and
//! `gltf-import`; contract).
//!
//! UI-, I/O-, and format-free (DEVELOPMENT.md rule 1 — kernel-only dependency): the
//! caller hands in raw positions/faces/material refs already extracted from its
//! own format, and gets back welded, deduped, oriented, watertight-where-possible
//! geometry ready for a `kernel::MeshRecipe`.
//!
//! Operates per-mesh, producing the deduplicated `(positions, faces)`. Passes,
//! in order:
//!
//! 1. **Unit + up-axis transform** — apply a `unit.meter` scale and rotate the
//!    source up-axis onto Hew's +Z. Positions land in meters, Z-up.
//! 2. **Weld** — bucket by quantized cell; merge positions within
//!    `tol::POINT_MERGE`; remap face indices; drop degenerate (collapsed) faces.
//! 3. **Drop zero-area faces** — collinear sliver triangles from SketchUp
//!    T-junctions are foreign artefacts; drop them (not patched).
//! 4. **Two-sided dedup** — SketchUp "Export Two-Sided Faces" emits each face
//!    twice with opposite winding. Drop the back face (keep front).
//! 5. **T-junction healing** — splice a vertex sitting on another face's edge
//!    into that edge so half-edges pair up manifold (`split_t_junctions`),
//!    turning an otherwise-open shell into a watertight solid.
//! 6. **Orientation** — flip a closed, inside-out shell to outward normals
//!    (`orient_outward`); some SketchUp source faces are reversed (invisible in
//!    SketchUp's double-sided view, but transparent + non-pushable here).
//!
//! No silent geometry repair beyond these well-specified steps: a face that
//! remains degenerate after welding is simply dropped, not patched. Degenerate
//! meshes that `from_polygons` would reject are still emitted — the kernel
//! reports them as skipped (DEVELOPMENT.md rule 4).

use kernel::{Point3, Transform, Vec3, tol};

/// Per-face affine UV-frame fitting from corner positions + corner UVs.
pub mod uv;

// ── Internal type aliases ─────────────────────────────────────────────────────

/// Per-face per-corner UV coordinate list (parallel to faces).
/// Empty inner vec = no TEXCOORD data for that face.
type FaceCornerUvs = Vec<Vec<[f64; 2]>>;

/// Per-face inner-loop (hole) index lists (parallel to faces).
/// Empty inner vec = no holes for that face.
type FaceHoles = Vec<Vec<Vec<usize>>>;

/// Output of the face-filtering steps: (faces, materials, corner_uvs, holes).
type FilteredFaces = (Vec<Vec<usize>>, Vec<u32>, FaceCornerUvs, FaceHoles);

// ── Degenerate-face threshold ─────────────────────────────────────────────────

/// Minimum triangle half-cross-product magnitude (squared) for a face to be
/// considered non-degenerate.  Derived from `tol::POINT_MERGE`: two points
/// that are tol::POINT_MERGE apart produce a triangle of area ~tol::POINT_MERGE²/2,
/// so we use tol::POINT_MERGE² as the area² floor.
///
/// This is **boundary normalization** at the import seam: zero-area sliver
/// triangles from SketchUp T-junction collinear vertices are foreign input
/// artefacts, not kernel invariant violations.  Dropping them here is safe and
/// explicitly documented (DEVELOPMENT.md rule 4 applies to kernel operations; this
/// is pre-kernel filtering).
const MIN_FACE_AREA_SQ: f64 = tol::POINT_MERGE * tol::POINT_MERGE;
use std::collections::HashMap;

// ── Up-axis rotation ──────────────────────────────────────────────────────────

/// Rotation that maps COLLADA Y-up onto Hew +Z-up: rotate +90° about +X.
/// ( Y→Z, Z→−Y )
pub fn y_up_to_z_up() -> Transform {
    Transform::rotation(Vec3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2)
        .expect("X axis is never degenerate")
}

/// Rotation that maps COLLADA X-up onto Hew +Z-up: rotate −90° about +Y.
/// ( X→Z, Z→−X )
pub fn x_up_to_z_up() -> Transform {
    Transform::rotation(Vec3::new(0.0, 1.0, 0.0), -std::f64::consts::FRAC_PI_2)
        .expect("Y axis is never degenerate")
}

/// Build the combined world-space correction transform: uniform scale (COLLADA
/// units → meters) followed by the up-axis rotation onto +Z.
///
/// `unit_meter` is the COLLADA `<unit meter="…">` attribute value
/// (how many meters one COLLADA unit is; 1.0 = meters, 0.01 = centimetres).
/// `up_axis` is the raw COLLADA string (`"Y_UP"`, `"Z_UP"`, `"X_UP"`).
pub fn world_transform(unit_meter: f32, up_axis: &str) -> Transform {
    let scale = Transform::uniform_scale(unit_meter as f64);
    let up_rot = match up_axis {
        "Z_UP" => Transform::IDENTITY,
        "X_UP" => x_up_to_z_up(),
        _ => y_up_to_z_up(), // Y_UP is the default per spec
    };
    scale.then(&up_rot)
}

// ── Weld ──────────────────────────────────────────────────────────────────────

/// Quantize a single coordinate to a bucket key.
#[inline]
fn bucket(v: f64, weld_tol: f64) -> i64 {
    (v / weld_tol).floor() as i64
}

/// Bucket key for a point (3 i64s packed into a tuple).
#[inline]
fn bucket_key(p: Point3, weld_tol: f64) -> (i64, i64, i64) {
    (
        bucket(p.x, weld_tol),
        bucket(p.y, weld_tol),
        bucket(p.z, weld_tol),
    )
}

/// Weld near-coincident positions together at the kernel's native `POINT_MERGE`
/// (1 nm) tolerance — appropriate for high-precision sources (COLLADA stores
/// arbitrary-precision text). For f32-quantised sources (glTF `POSITION` is
/// float32, so coincident vertices land microns apart at metre scale) use
/// [`weld_with_tol`] with a scale-appropriate tolerance instead.
///
/// Returns `(unique_positions, old_to_new)` where `old_to_new[i]` is the index
/// in `unique_positions` that position `i` mapped to.
pub fn weld(positions: &[Point3]) -> (Vec<Point3>, Vec<usize>) {
    weld_with_tol(positions, tol::POINT_MERGE)
}

/// Weld positions, merging any two within `weld_tol` (Euclidean). See [`weld`].
pub fn weld_with_tol(positions: &[Point3], weld_tol: f64) -> (Vec<Point3>, Vec<usize>) {
    // Map (bucket_key) → index in the representative list.
    let mut cell_to_rep: HashMap<(i64, i64, i64), usize> = HashMap::new();
    let mut unique: Vec<Point3> = Vec::new();
    let mut old_to_new: Vec<usize> = Vec::with_capacity(positions.len());

    for &p in positions {
        let key = bucket_key(p, weld_tol);
        // Probe the 3×3×3 neighbourhood for an existing representative within
        // tol::POINT_MERGE (Euclidean).
        let mut found = None;
        'outer: for dk0 in -1i64..=1 {
            for dk1 in -1i64..=1 {
                for dk2 in -1i64..=1 {
                    let probe = (key.0 + dk0, key.1 + dk1, key.2 + dk2);
                    if let Some(&rep_idx) = cell_to_rep.get(&probe) {
                        let rep = unique[rep_idx];
                        let dx = p.x - rep.x;
                        let dy = p.y - rep.y;
                        let dz = p.z - rep.z;
                        if (dx * dx + dy * dy + dz * dz).sqrt() <= weld_tol {
                            found = Some(rep_idx);
                            break 'outer;
                        }
                    }
                }
            }
        }
        if let Some(rep_idx) = found {
            old_to_new.push(rep_idx);
        } else {
            let new_idx = unique.len();
            unique.push(p);
            cell_to_rep.insert(key, new_idx);
            old_to_new.push(new_idx);
        }
    }

    (unique, old_to_new)
}

/// Remap face indices through `old_to_new`, dropping degenerate faces (where
/// a repeat index appears after remapping — collapsed by welding).
/// Returns `(remapped_faces, face_materials, face_corner_uvs, face_holes)` with
/// parallel arrays. When the outer loop degenerates the face AND its holes are
/// dropped. Hole loops that become degenerate after remapping (repeated index or
/// fewer than 3 distinct) are dropped, but the face is kept with the remaining
/// valid holes.
pub fn remap_faces(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    old_to_new: &[usize],
) -> FilteredFaces {
    let mut out_faces: Vec<Vec<usize>> = Vec::new();
    let mut out_mats: Vec<u32> = Vec::new();
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut out_holes: FaceHoles = Vec::new();

    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (((face, &mat), corner_uvs), holes) in faces
        .iter()
        .zip(face_materials.iter())
        .zip(face_corner_uvs.iter())
        .zip(
            face_holes
                .iter()
                .map(Some)
                .chain(std::iter::repeat(None))
                .map(|h| h.unwrap_or(&empty_holes)),
        )
    {
        let remapped: Vec<usize> = face.iter().map(|&i| old_to_new[i]).collect();
        // Drop degenerate outer: any repeated index → collapsed by welding.
        let mut seen = std::collections::HashSet::new();
        if !remapped.iter().all(|i| seen.insert(*i)) || remapped.len() < 3 {
            // Outer degenerated → drop face and its holes.
            continue;
        }
        // Remap hole loops; drop degenerate holes (keep face).
        let remapped_holes: Vec<Vec<usize>> = holes
            .iter()
            .filter_map(|hole| {
                let h: Vec<usize> = hole.iter().map(|&i| old_to_new[i]).collect();
                let mut seen_h = std::collections::HashSet::new();
                if h.iter().all(|i| seen_h.insert(*i)) && h.len() >= 3 {
                    Some(h)
                } else {
                    None // degenerate hole → drop it, keep face
                }
            })
            .collect();
        out_faces.push(remapped);
        out_mats.push(mat);
        out_uvs.push(corner_uvs.clone());
        out_holes.push(remapped_holes);
    }

    (out_faces, out_mats, out_uvs, out_holes)
}

/// Drop positions referenced by no face and remap face indices to the compacted
/// list (preserving first-use order).
///
/// A COLLADA `<mesh>` shares one `<vertices>` source across every primitive
/// group, so it carries vertices used only by `<lines>` edges (SketchUp exports
/// standalone edges as lines, which we ignore for solid import) or by unreferenced
/// hole vertices. After healing, those positions are referenced by no emitted
/// face, and `Object::from_polygons` rejects the whole mesh with "vertex N is
/// not used by any face". Compacting them away is import boundary normalization
/// — foreign-input artefacts at the seam, not kernel geometry (DEVELOPMENT.md rule 4).
///
/// Face material / corner-UV arrays are parallel to `faces` and need no change.
/// Hole vertices are counted as referenced and remapped along with face vertices.
// Return type uses FaceHoles (Vec<Vec<Vec<usize>>>) which clippy flags as
// complex; it matches the module's canonical alias and is intentional.
#[allow(clippy::type_complexity)]
pub fn compact_unused(
    positions: &[Point3],
    faces: &[Vec<usize>],
    face_holes: &[Vec<Vec<usize>>],
) -> (Vec<Point3>, Vec<Vec<usize>>, Vec<Vec<Vec<usize>>>) {
    let mut old_to_new = vec![usize::MAX; positions.len()];
    let mut compact: Vec<Point3> = Vec::new();
    let mut out_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    let mut out_holes: Vec<Vec<Vec<usize>>> = Vec::with_capacity(faces.len());

    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (face, holes) in faces
        .iter()
        .zip(face_holes.iter().map(Some).chain(std::iter::repeat(None)))
    {
        let holes = holes.unwrap_or(&empty_holes);
        let mut new_face = Vec::with_capacity(face.len());
        for &vi in face {
            if old_to_new[vi] == usize::MAX {
                old_to_new[vi] = compact.len();
                compact.push(positions[vi]);
            }
            new_face.push(old_to_new[vi]);
        }
        out_faces.push(new_face);
        // Remap hole vertices (count them as referenced).
        let new_holes: Vec<Vec<usize>> = holes
            .iter()
            .map(|hole| {
                hole.iter()
                    .map(|&vi| {
                        if old_to_new[vi] == usize::MAX {
                            old_to_new[vi] = compact.len();
                            compact.push(positions[vi]);
                        }
                        old_to_new[vi]
                    })
                    .collect()
            })
            .collect();
        out_holes.push(new_holes);
    }

    (compact, out_faces, out_holes)
}

// ── Two-sided dedup ───────────────────────────────────────────────────────────

/// Detect and remove the "back" duplicate from SketchUp's two-sided export.
///
/// A face pair is two-sided if their vertex-id sets are equal (as sorted
/// tuples) but their cyclic order is the reverse of each other.  We keep the
/// first occurrence (the "front" face) and drop the second.
///
/// Identical windings are NOT deduplicated — only opposite-winding duplicates.
/// Deduplication is keyed on the OUTER loop only; a back-face's holes are
/// dropped with it. The algorithm is idempotent.
pub fn dedup_two_sided(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
) -> FilteredFaces {
    // Canonical key: sorted vertex ids as a Vec<usize>.
    // Value: the first face's canonical direction (for opposite-winding check).
    let mut seen: HashMap<Vec<usize>, Vec<usize>> = HashMap::new();
    let mut out_faces: Vec<Vec<usize>> = Vec::new();
    let mut out_mats: Vec<u32> = Vec::new();
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut out_holes: FaceHoles = Vec::new();

    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (((face, &mat), corner_uvs), holes) in faces
        .iter()
        .zip(face_materials.iter())
        .zip(face_corner_uvs.iter())
        .zip(
            face_holes
                .iter()
                .map(Some)
                .chain(std::iter::repeat(None))
                .map(|h| h.unwrap_or(&empty_holes)),
        )
    {
        let mut key = face.clone();
        key.sort_unstable();

        if let Some(first) = seen.get(&key) {
            // Check opposite winding: the reverse of `first` should be
            // a cyclic rotation of `face`.
            let reversed: Vec<usize> = first.iter().rev().cloned().collect();
            if is_cyclic_equal(&reversed, face) {
                // Back-face duplicate → drop (holes dropped with it).
                continue;
            }
            // Same winding or different positions → keep (don't over-dedup).
        } else {
            seen.insert(key, face.clone());
        }
        out_faces.push(face.clone());
        out_mats.push(mat);
        out_uvs.push(corner_uvs.clone());
        out_holes.push(holes.clone());
    }

    (out_faces, out_mats, out_uvs, out_holes)
}

/// True if `a` and `b` have the same length and `a` is a cyclic rotation of `b`.
fn is_cyclic_equal(a: &[usize], b: &[usize]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let n = a.len();
    (0..n).any(|shift| (0..n).all(|i| a[i] == b[(i + shift) % n]))
}

// ── T-junction healing ────────────────────────────────────────────────────────

/// If `p` lies on the **open** segment `(a, b)` — strictly between the endpoints
/// and within `tol::POINT_MERGE` perpendicular distance — return its parameter
/// `t ∈ (0, 1)` (so `closest = a + (b−a)·t`). Otherwise `None`.
///
/// "Strictly between" excludes points within `tol::POINT_MERGE` of either
/// endpoint, so shared corners are never treated as T-junctions.
fn point_on_open_segment(p: Point3, a: Point3, b: Point3) -> Option<f64> {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= tol::POINT_MERGE * tol::POINT_MERGE {
        return None; // degenerate edge
    }
    let len = len_sq.sqrt();
    let t = (p - a).dot(ab) / len_sq;
    // Reject if the foot of the perpendicular is within tol of either endpoint.
    let margin = tol::POINT_MERGE / len;
    if t <= margin || t >= 1.0 - margin {
        return None;
    }
    let closest = a + ab * t;
    if (p - closest).length_squared() <= tol::POINT_MERGE * tol::POINT_MERGE {
        Some(t)
    } else {
        None
    }
}

/// Linearly interpolate two corner UVs at parameter `t`.
#[inline]
fn lerp_uv(ua: [f64; 2], ub: [f64; 2], t: f64) -> [f64; 2] {
    [ua[0] + t * (ub[0] - ua[0]), ua[1] + t * (ub[1] - ua[1])]
}

/// Heal T-junctions: where a vertex lies on the interior of another face's edge
/// (a "T"), splice it into that edge so the half-edges pair up manifold.
///
/// SketchUp's triangulation routinely places a vertex mid-edge of a coplanar
/// neighbour. After welding, the long edge `(v0, v2)` has no manifold twin
/// (the neighbour carries `(v2, v1)` and `(v1, v0)` instead), leaving the shell
/// open. Splitting `(v0, v2)` into `(v0, v1), (v1, v2)` restores pairing. The
/// resulting "straight" (collinear) corner is accepted by `from_polygons`
/// (`crates/kernel/src/build.rs`): it rejects repeated indices, non-planar
/// faces, and duplicate directed edges — but not collinear corners.
///
/// Runs after `dedup_two_sided` so it only touches the kept front shell.
/// Idempotent: a second pass finds no interior hits (endpoints are excluded).
///
/// Hole loops are threaded through unchanged (conservative: hole edges are not
/// spliced). Face order and count are preserved so the parallel `face_holes`
/// array stays aligned.
pub fn split_t_junctions(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
) -> FilteredFaces {
    // Candidate vertices: those used as a corner by some face (outer loops only).
    let mut used: Vec<bool> = vec![false; positions.len()];
    for face in faces {
        for &v in face {
            if v < used.len() {
                used[v] = true;
            }
        }
    }
    let candidates: Vec<usize> = (0..positions.len()).filter(|&i| used[i]).collect();

    let mut out_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(faces.len());

    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (face, corner_uvs) in faces.iter().zip(face_corner_uvs.iter()) {
        let n = face.len();
        // UVs are usable only when there is one per corner.
        let has_uv = corner_uvs.len() == n && n > 0;
        let mut new_face: Vec<usize> = Vec::with_capacity(n);
        let mut new_uv: Vec<[f64; 2]> = Vec::with_capacity(n);

        for k in 0..n {
            let a = face[k];
            let b = face[(k + 1) % n];
            new_face.push(a);
            if has_uv {
                new_uv.push(corner_uvs[k]);
            }

            // Collect interior vertices on edge (a, b), sorted by parameter.
            let pa = positions[a];
            let pb = positions[b];
            let mut hits: Vec<(f64, usize)> = candidates
                .iter()
                .filter(|&&v| v != a && v != b)
                .filter_map(|&v| point_on_open_segment(positions[v], pa, pb).map(|t| (t, v)))
                .collect();
            hits.sort_by(|x, y| x.0.total_cmp(&y.0));

            for (t, v) in hits {
                new_face.push(v);
                if has_uv {
                    new_uv.push(lerp_uv(corner_uvs[k], corner_uvs[(k + 1) % n], t));
                }
            }
        }

        out_faces.push(new_face);
        out_uvs.push(if has_uv { new_uv } else { Vec::new() });
    }

    // Holes pass through unchanged (conservative — no T-junction healing on
    // hole edges). face_holes may be shorter than faces when all entries are
    // empty; fill with empty_holes for any missing entries.
    let out_holes: FaceHoles = (0..faces.len())
        .map(|i| {
            face_holes
                .get(i)
                .cloned()
                .unwrap_or_else(|| empty_holes.clone())
        })
        .collect();

    (out_faces, face_materials.to_vec(), out_uvs, out_holes)
}

/// Signed volume × 6 of an oriented face set (fan-triangulated). Positive when
/// the faces are wound CCW-from-outside (outward normals); negative when the
/// whole shell is inside-out. Meaningful only for a closed mesh.
fn signed_volume6(faces: &[Vec<usize>], positions: &[Point3]) -> f64 {
    let mut v6 = 0.0;
    for face in faces {
        if face.len() < 3 {
            continue;
        }
        let p0 = positions[face[0]].to_vec();
        for i in 1..face.len() - 1 {
            let p1 = positions[face[i]].to_vec();
            let p2 = positions[face[i + 1]].to_vec();
            v6 += p0.dot(p1.cross(p2));
        }
    }
    v6
}

/// True if every directed edge `(a, b)` of the face set has its reverse
/// `(b, a)` — i.e. the mesh is closed (manifold or not). Orientation is only
/// well-defined for a closed shell, so we gate the flip on this.
fn is_closed(faces: &[Vec<usize>]) -> bool {
    use std::collections::HashSet;
    let mut dir: HashSet<(usize, usize)> = HashSet::new();
    for face in faces {
        let n = face.len();
        for k in 0..n {
            dir.insert((face[k], face[(k + 1) % n]));
        }
    }
    dir.iter().all(|&(a, b)| dir.contains(&(b, a)))
}

/// Normalize a **closed** shell to outward orientation: if its signed volume is
/// negative the whole solid is inside-out (e.g. SketchUp faces reversed in the
/// source model — invisible there because SketchUp renders double-sided, but a
/// single-sided renderer culls every face and push/pull inverts). Reverse every
/// face's winding (and its corner UVs) so normals point outward.
///
/// When flipping, hole loops are also reversed (their winding is relative to the
/// outer loop's normal, so they must be flipped together).
///
/// Open shells are left untouched — orientation is ambiguous without a closed
/// volume, and flipping could make things worse. Import boundary normalization,
/// same category as weld / two-sided dedup / T-junction healing.
pub fn orient_outward(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
) -> FilteredFaces {
    // Only a closed, inside-out shell is flipped.
    if !is_closed(faces) || signed_volume6(faces, positions) >= 0.0 {
        let out_holes: FaceHoles = (0..faces.len())
            .map(|i| face_holes.get(i).cloned().unwrap_or_default())
            .collect();
        return (
            faces.to_vec(),
            face_materials.to_vec(),
            face_corner_uvs.to_vec(),
            out_holes,
        );
    }

    let out_faces: Vec<Vec<usize>> = faces
        .iter()
        .map(|f| f.iter().rev().copied().collect())
        .collect();
    let out_uvs: Vec<Vec<[f64; 2]>> = face_corner_uvs
        .iter()
        .map(|uvs| uvs.iter().rev().copied().collect())
        .collect();
    // Reverse each hole loop when flipping the shell.
    let out_holes: FaceHoles = (0..faces.len())
        .map(|i| {
            face_holes
                .get(i)
                .map(|holes| {
                    holes
                        .iter()
                        .map(|h| h.iter().rev().copied().collect())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    (out_faces, face_materials.to_vec(), out_uvs, out_holes)
}

// ── Consistent orientation (flood fill) ─────────────────────────────────────────

/// True if `face`, optionally read in reverse, traverses the directed edge
/// `a -> b` along its outer loop.
fn outer_traverses(face: &[usize], reversed: bool, a: usize, b: usize) -> bool {
    let m = face.len();
    for k in 0..m {
        let (mut x, mut y) = (face[k], face[(k + 1) % m]);
        if reversed {
            std::mem::swap(&mut x, &mut y);
        }
        if x == a && y == b {
            return true;
        }
    }
    false
}

/// Make adjacent faces *consistently* wound: across every interior (manifold)
/// edge the two incident faces must traverse it in OPPOSITE directions. SketchUp
/// sometimes exports a shell with a subset of faces reversed relative to their
/// neighbours — invisible in its double-sided view, but `from_polygons` rejects
/// it ("directed edge … traversed by more than one face"). A breadth-first flood
/// over edge adjacency flips faces into agreement with a per-connected-component
/// seed.
///
/// Only edges shared by *exactly two* faces drive propagation. Boundary edges
/// (one face) and genuinely non-manifold edges (>2 faces) are skipped: a
/// non-manifold mesh therefore stays inconsistent and is still reported as
/// skipped downstream — no silent repair of non-solid input (DEVELOPMENT.md rule 4).
///
/// Idempotent on already-consistent meshes (the seed face is never flipped and
/// every neighbour already agrees, so no face is reversed). Runs before
/// [`orient_outward`], which then flips the now-consistent shell so its normals
/// point outward. Hole loops are reversed together with their face, matching
/// `orient_outward`; final hole winding is fixed by [`normalize_hole_winding`].
pub fn orient_consistent(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
) -> FilteredFaces {
    use std::collections::VecDeque;

    let n = faces.len();

    // Undirected outer edge -> incident face indices.
    let mut edge_faces: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (fi, face) in faces.iter().enumerate() {
        let m = face.len();
        for k in 0..m {
            let a = face[k];
            let b = face[(k + 1) % m];
            let key = if a < b { (a, b) } else { (b, a) };
            edge_faces.entry(key).or_default().push(fi);
        }
    }

    let mut flip = vec![false; n];
    let mut visited = vec![false; n];

    for seed in 0..n {
        if visited[seed] {
            continue;
        }
        visited[seed] = true;
        let mut queue = VecDeque::new();
        queue.push_back(seed);
        while let Some(fi) = queue.pop_front() {
            let face = &faces[fi];
            let m = face.len();
            for k in 0..m {
                // The directed edge this face (in its FINAL orientation) traverses.
                let (mut a, mut b) = (face[k], face[(k + 1) % m]);
                if flip[fi] {
                    std::mem::swap(&mut a, &mut b);
                }
                let key = if a < b { (a, b) } else { (b, a) };
                let incident = &edge_faces[&key];
                if incident.len() != 2 {
                    continue; // boundary or non-manifold edge: don't propagate
                }
                let nb = if incident[0] == fi {
                    incident[1]
                } else {
                    incident[0]
                };
                if visited[nb] {
                    continue; // already decided; conflicts left for the kernel to reject
                }
                // The neighbour must traverse the reverse edge (b -> a). Pick the
                // flip that makes it do so.
                let nb_face = &faces[nb];
                let need_flip = if outer_traverses(nb_face, false, b, a) {
                    false
                } else if outer_traverses(nb_face, true, b, a) {
                    true
                } else {
                    continue; // shared vertex pair but not a shared edge; ignore
                };
                visited[nb] = true;
                flip[nb] = need_flip;
                queue.push_back(nb);
            }
        }
    }

    let mut out_faces: Vec<Vec<usize>> = Vec::with_capacity(n);
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(n);
    let mut out_holes: FaceHoles = Vec::with_capacity(n);
    for i in 0..n {
        if flip[i] {
            out_faces.push(faces[i].iter().rev().copied().collect());
            out_uvs.push(face_corner_uvs[i].iter().rev().copied().collect());
            out_holes.push(
                face_holes
                    .get(i)
                    .map(|hs| {
                        hs.iter()
                            .map(|h| h.iter().rev().copied().collect())
                            .collect()
                    })
                    .unwrap_or_default(),
            );
        } else {
            out_faces.push(faces[i].clone());
            out_uvs.push(face_corner_uvs[i].clone());
            out_holes.push(face_holes.get(i).cloned().unwrap_or_default());
        }
    }

    (out_faces, face_materials.to_vec(), out_uvs, out_holes)
}

// ── Coplanar merge ────────────────────────────────────────────────────────────

/// Unit (Newell) normal of a polygon, or `None` if degenerate.
fn face_normal(face: &[usize], positions: &[Point3]) -> Option<Vec3> {
    let n = face.len();
    let mut nrm = Vec3::new(0.0, 0.0, 0.0);
    for k in 0..n {
        let a = positions[face[k]];
        let b = positions[face[(k + 1) % n]];
        nrm = nrm
            + Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
    }
    nrm.normalized().ok()
}

/// Reverse each hole loop that winds the SAME direction as its face's outer loop
/// so every hole ends up wound OPPOSITE the outer — Hew's native convention
/// ("outer CCW, inner CW seen from the face normal", `from_faces_with_holes`).
///
/// COLLADA `<h>` winding is unspecified, and the tessellator's ear-clip hole
/// bridging needs holes against the outer or it fills/garbles the opening.
/// Returns `face_holes` with each hole normalized; idempotent. A face with a
/// degenerate outer normal is left untouched.
pub fn normalize_hole_winding(
    faces: &[Vec<usize>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
) -> Vec<Vec<Vec<usize>>> {
    faces
        .iter()
        .enumerate()
        .map(|(fi, outer)| {
            let holes = match face_holes.get(fi) {
                Some(h) if !h.is_empty() => h,
                _ => return Vec::new(),
            };
            let Some(outer_n) = face_normal(outer, positions) else {
                return holes.clone();
            };
            holes
                .iter()
                .map(|hole| match face_normal(hole, positions) {
                    // Same direction as the outer loop → reverse to oppose it.
                    Some(hn) if hn.dot(outer_n) > 0.0 => hole.iter().rev().copied().collect(),
                    _ => hole.clone(),
                })
                .collect()
        })
        .collect()
}

/// Whether every vertex of `loop_verts` lies within `tol::PLANE_DIST` of the
/// loop's best-fit (Newell normal + centroid) plane — i.e. planar to the same
/// tolerance the kernel's `from_polygons` enforces. A degenerate normal → not
/// planar (caller falls back).
fn loop_is_planar(loop_verts: &[usize], positions: &[Point3], plane_tol: f64) -> bool {
    let normal = match face_normal(loop_verts, positions) {
        Some(n) => n,
        None => return false,
    };
    let n = loop_verts.len() as f64;
    let mut centroid = Vec3::new(0.0, 0.0, 0.0);
    for &v in loop_verts {
        centroid = centroid + positions[v].to_vec();
    }
    centroid = centroid / n;
    loop_verts
        .iter()
        .all(|&v| normal.dot(positions[v].to_vec() - centroid).abs() <= plane_tol)
}

/// Minimal union-find over `0..n`.
struct UnionFind {
    parent: Vec<usize>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// Cosine tolerance for "same plane normal": ~0.08°. Triangles of one flat
/// SketchUp face are exactly coplanar; real edges meet at much larger angles,
/// and slightly-curved surfaces stay just under this and are left un-merged.
const COPLANAR_DOT: f64 = 1.0 - 1e-6;

/// Merge coplanar, edge-adjacent, same-material faces back into single polygons.
///
/// COLLADA always triangulates, so a flat face the user drew in SketchUp arrives
/// as a triangle fan. This rebuilds the original polygon so push/pull, selection,
/// and inference treat it as one face.
///
/// **Non-regressive by construction:** faces are only merged when their union
/// has a boundary that chains into exactly one simple loop (every boundary
/// vertex has a single successor and one walk covers all boundary edges). A
/// group with holes, pinch points, or a split boundary is left as its original
/// triangles — never producing a polygon `from_polygons` would reject. Adjacency
/// requires a shared edge + (near-)identical normal + same material, so the
/// merged polygon is planar and keeps the constituent triangles' outward winding.
///
/// **Holed faces are never unioned:** a face with any holes is forced to stay a
/// singleton (passed through via `keep_original` with its holes intact). Merged
/// triangle clusters always emit an empty holes entry. This keeps `face_holes`
/// correctly aligned with `out_faces` after the merge.
pub fn merge_coplanar(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
    plane_tol: f64,
) -> FilteredFaces {
    let nf = faces.len();
    if nf == 0 {
        return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    }

    // Per-face normals; faces with a degenerate normal never merge.
    let normals: Vec<Option<Vec3>> = faces.iter().map(|f| face_normal(f, positions)).collect();

    // True when face fi has at least one hole (prevents it from being unioned).
    let has_holes: Vec<bool> = (0..nf)
        .map(|fi| face_holes.get(fi).map(|h| !h.is_empty()).unwrap_or(false))
        .collect();

    // Map each undirected edge → the faces touching it (for adjacency).
    let mut edge_faces: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (fi, face) in faces.iter().enumerate() {
        let n = face.len();
        for k in 0..n {
            let (a, b) = (face[k], face[(k + 1) % n]);
            let key = if a < b { (a, b) } else { (b, a) };
            edge_faces.entry(key).or_default().push(fi);
        }
    }

    // Union faces that share an edge, same material, (near-)identical normal,
    // AND neither face has holes (holed faces stay singletons).
    let mut uf = UnionFind::new(nf);
    for sharing in edge_faces.values() {
        for i in 0..sharing.len() {
            for j in (i + 1)..sharing.len() {
                let (fa, fb) = (sharing[i], sharing[j]);
                // Never union a face that has holes.
                if has_holes[fa] || has_holes[fb] {
                    continue;
                }
                if face_materials[fa] != face_materials[fb] {
                    continue;
                }
                match (normals[fa], normals[fb]) {
                    (Some(na), Some(nb)) if na.dot(nb) >= COPLANAR_DOT => uf.union(fa, fb),
                    _ => {}
                }
            }
        }
    }

    // Bucket faces by cluster root.
    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for fi in 0..nf {
        clusters.entry(uf.find(fi)).or_default().push(fi);
    }

    let mut out_faces: Vec<Vec<usize>> = Vec::with_capacity(nf);
    let mut out_mats: Vec<u32> = Vec::new();
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut out_holes: FaceHoles = Vec::new();

    // Deterministic order: process clusters by their smallest face index.
    let mut roots: Vec<usize> = clusters.keys().copied().collect();
    roots.sort_by_key(|r| clusters[r].iter().copied().min().unwrap());

    for root in roots {
        let group = &clusters[&root];
        let keep_original = |out_faces: &mut Vec<Vec<usize>>,
                             out_mats: &mut Vec<u32>,
                             out_uvs: &mut Vec<Vec<[f64; 2]>>,
                             out_holes: &mut FaceHoles| {
            for &fi in group {
                out_faces.push(faces[fi].clone());
                out_mats.push(face_materials[fi]);
                out_uvs.push(face_corner_uvs[fi].clone());
                out_holes.push(face_holes.get(fi).cloned().unwrap_or_default());
            }
        };

        if group.len() == 1 {
            keep_original(&mut out_faces, &mut out_mats, &mut out_uvs, &mut out_holes);
            continue;
        }

        // Boundary = directed edges of the group whose reverse is NOT in the
        // group. Record the (source vertex → (dest, uv-at-source)) successor.
        let mut dir: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
        for &fi in group {
            let face = &faces[fi];
            let n = face.len();
            for k in 0..n {
                dir.insert((face[k], face[(k + 1) % n]));
            }
        }
        // succ: vertex a → (b, uv_at_a). Reject (fallback) if a vertex has >1
        // outgoing boundary edge (pinch / non-simple boundary).
        let mut succ: HashMap<usize, (usize, Option<[f64; 2]>)> = HashMap::new();
        let mut boundary_count = 0usize;
        let mut simple = true;
        for &fi in group {
            let face = &faces[fi];
            let uvs = &face_corner_uvs[fi];
            let has_uv = uvs.len() == face.len();
            let n = face.len();
            for k in 0..n {
                let a = face[k];
                let b = face[(k + 1) % n];
                if dir.contains(&(b, a)) {
                    continue; // internal edge
                }
                boundary_count += 1;
                let uv = if has_uv { Some(uvs[k]) } else { None };
                if succ.insert(a, (b, uv)).is_some() {
                    simple = false;
                }
            }
        }

        if !simple || boundary_count == 0 {
            keep_original(&mut out_faces, &mut out_mats, &mut out_uvs, &mut out_holes);
            continue;
        }

        // Walk the successor chain into one loop; must cover every boundary edge.
        let start = *succ.keys().next().unwrap();
        let mut loop_verts: Vec<usize> = Vec::with_capacity(boundary_count);
        let mut loop_uvs: Vec<[f64; 2]> = Vec::with_capacity(boundary_count);
        let mut any_uv_missing = false;
        let mut cur = start;
        for _ in 0..boundary_count {
            let (next, uv) = match succ.get(&cur) {
                Some(&x) => x,
                None => break,
            };
            loop_verts.push(cur);
            match uv {
                Some(u) => loop_uvs.push(u),
                None => any_uv_missing = true,
            }
            cur = next;
            if cur == start {
                break;
            }
        }

        let single_loop = cur == start && loop_verts.len() == boundary_count;
        // A clean polygon needs ≥3 distinct vertices and no repeats.
        let distinct: std::collections::HashSet<usize> = loop_verts.iter().copied().collect();
        if !single_loop || loop_verts.len() < 3 || distinct.len() != loop_verts.len() {
            keep_original(&mut out_faces, &mut out_mats, &mut out_uvs, &mut out_holes);
            continue;
        }

        // Planarity gate: a single triangle is always exactly planar, but a
        // merged polygon off an angled face may not be. `plane_tol` must match
        // what the kernel will accept for this source: 1 nm (`PLANE_DIST`) for
        // exact f64 (COLLADA text), but ~1 mm (`IMPORT_PLANE_DIST`) for f32
        // (glTF) where coincident-plane vertices land microns off — otherwise
        // every flat surface falls back to triangles, exploding face count and
        // memory. Falling back to triangles is always safe (never trades an
        // importable mesh for a skipped one).
        if !loop_is_planar(&loop_verts, positions, plane_tol) {
            keep_original(&mut out_faces, &mut out_mats, &mut out_uvs, &mut out_holes);
            continue;
        }

        // Accept the merged polygon (no holes — merged triangles never have holes).
        out_faces.push(loop_verts);
        out_mats.push(face_materials[group[0]]);
        out_uvs.push(if any_uv_missing { Vec::new() } else { loop_uvs });
        out_holes.push(Vec::new()); // merged clusters have no holes
    }

    (out_faces, out_mats, out_uvs, out_holes)
}

// ── Full heal pipeline ────────────────────────────────────────────────────────

/// Apply the full heal pipeline to one mesh's raw positions and faces.
///
/// `bake_tf` is the combined transform to apply to positions: for world meshes
/// it is `acc.then(&world_tf)` (COLLADA placement + unit/up-axis); for def
/// geometry it is `Transform::IDENTITY` (positions stay in COLLADA units so
/// the instance pose can apply the correction once at placement time).
///
/// Returns `(healed_positions, healed_faces, healed_face_materials,
/// healed_face_corner_uvs, healed_face_holes)`.  `healed_face_corner_uvs` and
/// `healed_face_holes` are parallel to `healed_faces`.
pub fn heal_mesh(
    raw_positions: &[Point3],
    raw_faces: &[Vec<usize>],
    raw_face_materials: &[u32],
    raw_face_corner_uvs: &[Vec<[f64; 2]>],
    raw_face_holes: &[Vec<Vec<usize>>],
    bake_tf: &Transform,
) -> (
    Vec<Point3>,
    Vec<Vec<usize>>,
    Vec<u32>,
    FaceCornerUvs,
    FaceHoles,
) {
    heal_mesh_with_tol(
        raw_positions,
        raw_faces,
        raw_face_materials,
        raw_face_corner_uvs,
        raw_face_holes,
        bake_tf,
        tol::POINT_MERGE,
        tol::PLANE_DIST,
    )
}

/// Like [`heal_mesh`] but with explicit tolerances for f32-quantised sources
/// (glTF `POSITION` is float32, so coincident vertices land microns apart at
/// metre scale):
/// - `weld_tol`: vertices within this distance merge. The native 1 nm
///   `POINT_MERGE` is far too tight for f32 and leaves every shared edge split
///   (a "leaky" shell).
/// - `merge_plane_tol`: a merged coplanar polygon must be planar to this. The
///   native 1 nm `PLANE_DIST` rejects every f32 flat surface, so coplanar
///   triangles never coalesce — exploding face count and memory. Pass the
///   kernel's import planarity (`IMPORT_PLANE_DIST`, 1 mm).
#[allow(clippy::too_many_arguments)]
pub fn heal_mesh_with_tol(
    raw_positions: &[Point3],
    raw_faces: &[Vec<usize>],
    raw_face_materials: &[u32],
    raw_face_corner_uvs: &[Vec<[f64; 2]>],
    raw_face_holes: &[Vec<Vec<usize>>],
    bake_tf: &Transform,
    weld_tol: f64,
    merge_plane_tol: f64,
) -> (
    Vec<Point3>,
    Vec<Vec<usize>>,
    Vec<u32>,
    FaceCornerUvs,
    FaceHoles,
) {
    // 1. Apply bake transform (unit + up-axis for world meshes; identity for defs).
    let transformed: Vec<Point3> = raw_positions
        .iter()
        .map(|&p| bake_tf.apply_point(p))
        .collect();

    // 2. Weld.
    let (unique_positions, old_to_new) = weld_with_tol(&transformed, weld_tol);
    let (remapped_faces, remapped_mats, remapped_uvs, remapped_holes) = remap_faces(
        raw_faces,
        raw_face_materials,
        raw_face_corner_uvs,
        raw_face_holes,
        &old_to_new,
    );

    // 3. Drop zero-area faces (collinear T-junction slivers from SketchUp).
    //    This is boundary normalization at the import seam: these are foreign
    //    input artefacts, not kernel invariant violations (DEVELOPMENT.md rule 4).
    let (nondegenerate_faces, nondegenerate_mats, nondegenerate_uvs, nondegenerate_holes) =
        drop_zero_area_faces(
            &remapped_faces,
            &remapped_mats,
            &remapped_uvs,
            &remapped_holes,
            &unique_positions,
        );

    // 4. Two-sided dedup.
    let (dedup_faces, dedup_mats, dedup_uvs, dedup_holes) = dedup_two_sided(
        &nondegenerate_faces,
        &nondegenerate_mats,
        &nondegenerate_uvs,
        &nondegenerate_holes,
    );

    // 5. T-junction healing: splice mid-edge vertices into the faces whose edges
    //    they sit on, so half-edges pair up manifold (SketchUp triangulation
    //    leaves coplanar T-junctions that otherwise read as an open shell).
    //    Hole loops are threaded through unchanged (conservative).
    let (split_faces, split_mats, split_uvs, split_holes) = split_t_junctions(
        &dedup_faces,
        &dedup_mats,
        &dedup_uvs,
        &dedup_holes,
        &unique_positions,
    );

    // 5b. Consistent orientation: flood-fill so adjacent faces agree across every
    //    manifold edge. SketchUp occasionally reverses a subset of a shell's
    //    faces (invisible in its double-sided view), which otherwise reads as
    //    "directed edge traversed by more than one face" and gets the whole mesh
    //    skipped. Non-manifold meshes are left inconsistent (still skipped).
    let (consistent_faces, consistent_mats, consistent_uvs, consistent_holes) =
        orient_consistent(&split_faces, &split_mats, &split_uvs, &split_holes);

    // 6. Orientation: flip a closed, inside-out shell to outward normals (some
    //    SketchUp source faces are reversed — invisible there, transparent +
    //    non-pushable here). Hole loops are reversed together with the outer.
    let (oriented_faces, oriented_mats, oriented_uvs, oriented_holes) = orient_outward(
        &consistent_faces,
        &consistent_mats,
        &consistent_uvs,
        &consistent_holes,
        &unique_positions,
    );

    // 7. Coplanar merge: COLLADA always triangulates, so a flat SketchUp face
    //    arrives as a fan of triangles. Merge coplanar, edge-connected,
    //    same-material triangles back into one polygon face so push/pull and
    //    selection treat them as the single face the user drew. Non-regressive:
    //    a group that wouldn't form a clean simple-loop polygon is left as-is.
    //    Faces with holes are never merged — they stay as singletons.
    let (final_faces, final_mats, final_uvs, final_holes) = merge_coplanar(
        &oriented_faces,
        &oriented_mats,
        &oriented_uvs,
        &oriented_holes,
        &unique_positions,
        merge_plane_tol,
    );

    // 7b. Normalize hole winding so every inner loop winds OPPOSITE its outer
    //    loop (Hew's native convention; see `Object::from_faces_with_holes`).
    //    COLLADA does not guarantee `<h>` winding, and the ear-clipping
    //    tessellator's hole bridging fills/garbles a hole wound with the outer.
    let final_holes = normalize_hole_winding(&final_faces, &final_holes, &unique_positions);

    // 8. Compact: drop positions referenced by no surviving face (the shared
    //    <vertices> source includes edge-only / hole-only vertices that
    //    `from_polygons` would reject). Runs last so the final face set defines
    //    what's referenced. Hole vertices are counted as referenced.
    let (compact_positions, compact_faces, compact_holes) =
        compact_unused(&unique_positions, &final_faces, &final_holes);

    (
        compact_positions,
        compact_faces,
        final_mats,
        final_uvs,
        compact_holes,
    )
}

/// Drop faces whose area is below [`MIN_FACE_AREA_SQ`].
///
/// Triangles from SketchUp can include collinear slivers (three points on a
/// line, zero cross-product). The general polygon case is handled by testing
/// the fan-triangulation area sum. Only triangles degenerate in this way in
/// practice (SketchUp always emits triangles); the fan check is correct for
/// all convex polygons.
///
/// This runs AFTER welding so positions are already in their final (potentially
/// meter-scale) coordinates. Holes are carried parallel to faces; they are
/// dropped together with their (degenerate) face.
fn drop_zero_area_faces(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
) -> FilteredFaces {
    let mut out_faces = Vec::new();
    let mut out_mats = Vec::new();
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut out_holes: FaceHoles = Vec::new();

    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (((face, &mat), corner_uvs), holes) in faces
        .iter()
        .zip(face_materials.iter())
        .zip(face_corner_uvs.iter())
        .zip(
            face_holes
                .iter()
                .map(Some)
                .chain(std::iter::repeat(None))
                .map(|h| h.unwrap_or(&empty_holes)),
        )
    {
        if face.len() < 3 {
            continue; // already filtered by remap_faces, but be safe
        }
        // Fan-triangulate from face[0] and accumulate cross-product magnitudes squared.
        let p0 = positions[face[0]];
        let mut area_sq_sum = 0.0_f64;
        for i in 1..(face.len() - 1) {
            let p1 = positions[face[i]];
            let p2 = positions[face[i + 1]];
            let v1 = p1 - p0;
            let v2 = p2 - p0;
            let cross = v1.cross(v2);
            // |cross|² = (2·triangle_area)² ; using this avoids a sqrt
            area_sq_sum += cross.dot(cross);
        }
        if area_sq_sum > MIN_FACE_AREA_SQ {
            out_faces.push(face.clone());
            out_mats.push(mat);
            out_uvs.push(corner_uvs.clone());
            out_holes.push(holes.clone());
        }
        // Faces with area_sq_sum ≤ MIN_FACE_AREA_SQ are collinear slivers;
        // drop silently (import boundary normalization, not silent repair).
    }

    (out_faces, out_mats, out_uvs, out_holes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// glTF positions are f32, so vertices coincident in the source land
    /// microns apart at metre scale. The native 1 nm weld must NOT merge such a
    /// gap (it can't, by design), but `weld_with_tol` at a scale-appropriate
    /// tolerance must — otherwise the shell reads as "leaky". Guards.
    #[test]
    fn weld_with_tol_merges_f32_scale_gap() {
        // ~3 µm apart at 30 m — representative of an f32 round-trip gap.
        let a = Point3::new(30.0, 0.0, 0.0);
        let b = Point3::new(30.0 + 3e-6, 0.0, 0.0);

        let (tight, _) = weld(&[a, b]);
        assert_eq!(tight.len(), 2, "1 nm weld must not merge a 3 µm gap");

        let (coarse, map) = weld_with_tol(&[a, b], 1e-5);
        assert_eq!(coarse.len(), 1, "10 µm weld merges the f32-scale gap");
        assert_eq!(map[0], map[1]);
    }

    /// Weld is idempotent: running it twice gives the same result.
    #[test]
    fn weld_idempotent() {
        let pts = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            // Near-duplicate of index 0.
            Point3::new(tol::POINT_MERGE * 0.4, 0.0, 0.0),
        ];
        let (u1, map1) = weld(&pts);
        // Remap the unique set through itself.
        let (u2, _) = weld(&u1);
        assert_eq!(
            u1.len(),
            u2.len(),
            "weld of already-welded set must be unchanged"
        );
        // The near-duplicate should have merged.
        assert!(u1.len() < pts.len(), "near-duplicate merged");
        assert_eq!(
            map1[0], map1[3],
            "near-duplicate maps to same representative"
        );
    }

    /// Two-sided dedup is idempotent.
    #[test]
    fn dedup_two_sided_idempotent() {
        let faces = vec![
            vec![0usize, 1, 2, 3],
            vec![3usize, 2, 1, 0], // opposite winding duplicate
        ];
        let mats = vec![0u32, 0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(), Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];
        let (f1, m1, u1, h1) = dedup_two_sided(&faces, &mats, &uvs, &holes);
        let (f2, m2, _, _) = dedup_two_sided(&f1, &m1, &u1, &h1);
        assert_eq!(f1, f2);
        assert_eq!(m1, m2);
        assert_eq!(f1.len(), 1, "back face removed");
    }

    /// Compaction drops positions referenced by no face and remaps indices.
    #[test]
    fn compact_unused_drops_orphans() {
        // 4 positions; vertex 1 is an orphan (e.g. an edge-only / hole-only vert).
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(9.0, 9.0, 9.0), // orphan — used by no face
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let faces = vec![vec![0usize, 2, 3]];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new()];
        let (compact, out_faces, _) = compact_unused(&positions, &faces, &holes);

        assert_eq!(compact.len(), 3, "orphan dropped");
        assert!(
            !compact.iter().any(|p| p.x == 9.0),
            "the orphan position is gone"
        );
        // Face reindexed to the compacted list, same geometry.
        let remapped: Vec<Point3> = out_faces[0].iter().map(|&i| compact[i]).collect();
        assert_eq!(
            remapped,
            vec![positions[0], positions[2], positions[3]],
            "face still references the same three points"
        );
    }

    /// A hole wound the same direction as its outer loop is reversed to oppose
    /// it; the result is idempotent.
    #[test]
    fn normalize_hole_winding_reverses_same_winding() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
        ];
        let faces = vec![vec![0usize, 1, 2, 3]]; // outer CCW (normal +z)
        let holes = vec![vec![vec![4usize, 5, 6, 7]]]; // hole CCW — same as outer
        let out = normalize_hole_winding(&faces, &holes, &positions);
        assert_eq!(out[0][0], vec![7, 6, 5, 4], "reversed to oppose the outer");
        let out2 = normalize_hole_winding(&faces, &out, &positions);
        assert_eq!(out2[0][0], vec![7, 6, 5, 4], "idempotent");
    }

    /// Compaction is a no-op (identity) when every position is referenced.
    #[test]
    fn compact_unused_noop_when_all_referenced() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let faces = vec![vec![0usize, 1, 2]];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new()];
        let (compact, out_faces, _) = compact_unused(&positions, &faces, &holes);
        assert_eq!(compact, positions);
        assert_eq!(out_faces, faces);
    }

    /// Two faces with same winding are NOT deduplicated.
    #[test]
    fn dedup_same_winding_kept() {
        let faces = vec![
            vec![0usize, 1, 2, 3],
            vec![0usize, 1, 2, 3], // same winding
        ];
        let mats = vec![0u32, 0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(), Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];
        let (f, _, _, _) = dedup_two_sided(&faces, &mats, &uvs, &holes);
        assert_eq!(f.len(), 2, "same-winding duplicates are kept");
    }

    /// A vertex sitting on the interior of another face's edge is spliced in,
    /// and its corner UV is the linear interpolation of the edge's endpoints.
    #[test]
    fn split_t_junctions_splices_mid_edge_vertex() {
        // Quad [0,1,2,3] with vertex 4 = (1,0,0) on its bottom edge (0→1).
        // Vertex 4 is "used" by the second face, so it's a healing candidate.
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0), // 0
            Point3::new(2.0, 0.0, 0.0), // 1
            Point3::new(2.0, 1.0, 0.0), // 2
            Point3::new(0.0, 1.0, 0.0), // 3
            Point3::new(1.0, 0.0, 0.0), // 4 — midpoint of edge (0,1)
        ];
        let faces = vec![vec![0usize, 1, 2, 3], vec![0usize, 4, 3]];
        let mats = vec![7u32, 9u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            Vec::new(),
        ];

        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];
        let (out_faces, out_mats, out_uvs, _) =
            split_t_junctions(&faces, &mats, &uvs, &holes, &positions);

        // Vertex 4 spliced into the quad's first edge; the triangle is untouched.
        assert_eq!(
            out_faces[0],
            vec![0, 4, 1, 2, 3],
            "mid-edge vertex spliced in"
        );
        assert_eq!(out_faces[1], vec![0, 4, 3], "triangle unchanged");
        // Materials preserved per face.
        assert_eq!(out_mats, mats);
        // The spliced corner UV is the lerp at t=0.5 of [0,0]→[1,0].
        assert_eq!(
            out_faces[0].len(),
            out_uvs[0].len(),
            "UVs stay parallel to corners"
        );
        assert_eq!(out_uvs[0][1], [0.5, 0.0], "interpolated corner UV");
        assert!(out_uvs[1].is_empty(), "UV-less face stays UV-less");
    }

    /// T-junction healing is idempotent: a second pass finds nothing to splice.
    #[test]
    fn split_t_junctions_idempotent() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
        ];
        let faces = vec![vec![0usize, 1, 2, 3], vec![0usize, 4, 3]];
        let mats = vec![0u32, 0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(), Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];

        let (f1, m1, u1, h1) = split_t_junctions(&faces, &mats, &uvs, &holes, &positions);
        let (f2, m2, _, _) = split_t_junctions(&f1, &m1, &u1, &h1, &positions);
        assert_eq!(f1, f2, "second pass must change nothing");
        assert_eq!(m1, m2);
    }

    /// An inside-out closed cube (negative signed volume) is flipped to outward;
    /// an already-outward cube and an open shell are left unchanged.
    #[test]
    fn orient_outward_flips_inverted_closed_shell() {
        // Unit cube, outward-wound (CCW from outside) → positive volume.
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 1.0, 1.0),
        ];
        let outward = vec![
            vec![0usize, 3, 2, 1], // bottom (−Z)
            vec![4usize, 5, 6, 7], // top    (+Z)
            vec![0usize, 1, 5, 4], // front  (−Y)
            vec![1usize, 2, 6, 5], // right  (+X)
            vec![2usize, 3, 7, 6], // back   (+Y)
            vec![3usize, 0, 4, 7], // left   (−X)
        ];
        assert!(signed_volume6(&outward, &positions) > 0.0);

        // Reverse every face → inside-out (negative volume).
        let inverted: Vec<Vec<usize>> = outward
            .iter()
            .map(|f| f.iter().rev().copied().collect())
            .collect();
        assert!(signed_volume6(&inverted, &positions) < 0.0);

        let mats = vec![0u32; 6];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 6];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 6];

        // Inverted → flipped back to outward.
        let (f, _, _, _) = orient_outward(&inverted, &mats, &uvs, &holes, &positions);
        assert!(
            signed_volume6(&f, &positions) > 0.0,
            "inside-out shell must be flipped outward"
        );

        // Already outward → unchanged.
        let (f2, _, _, _) = orient_outward(&outward, &mats, &uvs, &holes, &positions);
        assert_eq!(f2, outward, "outward shell left as-is");

        // Open shell (drop the top) → never flipped, even if negative.
        let open: Vec<Vec<usize>> = inverted[1..].to_vec();
        let open_mats = [0u32; 5];
        let open_uvs: [Vec<[f64; 2]>; 5] = Default::default();
        let open_holes: [Vec<Vec<usize>>; 5] = Default::default();
        let (f3, _, _, _) = orient_outward(&open, &open_mats, &open_uvs, &open_holes, &positions);
        assert_eq!(f3, open, "open shell orientation is left untouched");
    }

    /// Count directed edges traversed by more than one face — the exact condition
    /// `from_polygons` rejects ("non-manifold or inconsistent winding").
    fn dup_directed_edges(faces: &[Vec<usize>]) -> usize {
        let mut dir: HashMap<(usize, usize), usize> = HashMap::new();
        for f in faces {
            let n = f.len();
            for k in 0..n {
                *dir.entry((f[k], f[(k + 1) % n])).or_default() += 1;
            }
        }
        dir.values().filter(|&&c| c > 1).count()
    }

    /// A closed cube with a subset of its faces reversed (inconsistent winding —
    /// what SketchUp sometimes exports) is flood-filled back to a single
    /// consistent orientation, so no directed edge is traversed twice. An
    /// already-consistent mesh is returned unchanged (idempotent).
    #[test]
    fn orient_consistent_repairs_mixed_winding_cube() {
        // Topology-only: cube connectivity, no positions needed.
        let outward = vec![
            vec![0usize, 3, 2, 1], // bottom (−Z)
            vec![4usize, 5, 6, 7], // top    (+Z)
            vec![0usize, 1, 5, 4], // front  (−Y)
            vec![1usize, 2, 6, 5], // right  (+X)
            vec![2usize, 3, 7, 6], // back   (+Y)
            vec![3usize, 0, 4, 7], // left   (−X)
        ];
        assert_eq!(
            dup_directed_edges(&outward),
            0,
            "a consistently-wound cube has no duplicate directed edges"
        );

        // Reverse three of the six faces → mixed winding (some shared edges now
        // traversed the same way by both incident faces).
        let mut mixed = outward.clone();
        for i in [1usize, 3, 5] {
            mixed[i].reverse();
        }
        assert!(
            dup_directed_edges(&mixed) > 0,
            "reversing a subset breaks winding consistency"
        );

        let mats = vec![0u32; 6];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 6];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 6];

        let (fixed, _, _, _) = orient_consistent(&mixed, &mats, &uvs, &holes);
        assert_eq!(
            dup_directed_edges(&fixed),
            0,
            "flood fill removes every duplicate directed edge"
        );

        // Idempotent on an already-consistent mesh: the seed is never flipped and
        // every neighbour already agrees, so the output equals the input.
        let (again, _, _, _) = orient_consistent(&outward, &mats, &uvs, &holes);
        assert_eq!(again, outward, "consistent mesh is left untouched");
    }

    /// A non-manifold edge (shared by 3 faces) is excluded from propagation, so
    /// `orient_consistent` cannot — and must not — make it consistent. The mesh
    /// stays rejectable, which is correct: solids-first kernel refuses non-solids
    /// rather than silently deleting a face (DEVELOPMENT.md rule 4).
    #[test]
    fn orient_consistent_leaves_nonmanifold_unrepaired() {
        // Three faces all sharing the undirected edge {0,1} (vertices 2,3,4 are
        // the three distinct apexes). `orient_consistent` is purely topological,
        // so no positions are needed.
        let faces = vec![vec![0usize, 1, 2], vec![0usize, 1, 3], vec![0usize, 1, 4]];
        let mats = vec![0u32; 3];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 3];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 3];
        let (out, _, _, _) = orient_consistent(&faces, &mats, &uvs, &holes);
        assert!(
            dup_directed_edges(&out) > 0,
            "a genuinely non-manifold edge can never be made consistent"
        );
    }

    /// Eight cube corners (outward-wound quads available below).
    fn cube_positions() -> Vec<Point3> {
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 1.0, 1.0),
        ]
    }

    /// A triangulated cube (12 triangles) merges back to 6 quad faces.
    #[test]
    fn merge_coplanar_reassembles_triangulated_cube() {
        let positions = cube_positions();
        let quads = [
            [0usize, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        // Fan-triangulate each quad.
        let mut tris: Vec<Vec<usize>> = Vec::new();
        for q in &quads {
            tris.push(vec![q[0], q[1], q[2]]);
            tris.push(vec![q[0], q[2], q[3]]);
        }
        let mats = vec![0u32; tris.len()];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); tris.len()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); tris.len()];

        let (f, m, _, _) = merge_coplanar(&tris, &mats, &uvs, &holes, &positions, tol::PLANE_DIST);
        assert_eq!(f.len(), 6, "cube merges to 6 faces");
        assert!(
            f.iter().all(|face| face.len() == 4),
            "each merged face is a quad"
        );
        assert_eq!(m.len(), 6);
    }

    /// Faces that are NOT coplanar (a folded pair sharing an edge) are kept
    /// separate, even though they're adjacent and same-material.
    #[test]
    fn merge_coplanar_keeps_non_coplanar_faces_separate() {
        // Two triangles sharing edge (0,1) but in different planes (a fold).
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0), // +Z plane
            Point3::new(0.0, 0.0, 1.0), // bends up into +Y plane
        ];
        let tris = vec![vec![0usize, 1, 2], vec![1, 0, 3]];
        let mats = vec![0u32; 2];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 2];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 2];

        let (f, _, _, _) = merge_coplanar(&tris, &mats, &uvs, &holes, &positions, tol::PLANE_DIST);
        assert_eq!(f.len(), 2, "non-coplanar faces are not merged");
    }

    /// Two adjacent triangles whose normals are within the coplanar tolerance
    /// but whose union is *not* planar to `tol::PLANE_DIST` fall back to the
    /// original triangles — never emit a polygon the kernel would reject.
    #[test]
    fn merge_coplanar_rejects_non_planar_union() {
        // Quad [0,1,2,3] with vertex 3 lifted off the z=0 plane by 1e-4: small
        // enough that the two triangles' normals are within COPLANAR_DOT, large
        // enough that the merged quad exceeds tol::PLANE_DIST (1e-9).
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 1e-4),
        ];
        let tris = vec![vec![0usize, 1, 2], vec![0, 2, 3]];
        let mats = vec![0u32; 2];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 2];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 2];

        let (f, _, _, _) = merge_coplanar(&tris, &mats, &uvs, &holes, &positions, tol::PLANE_DIST);
        assert_eq!(
            f.len(),
            2,
            "non-planar union must fall back to its triangles"
        );
    }

    /// Coplanar but different-material triangles stay separate (don't blend a
    /// textured face into a colored one).
    #[test]
    fn merge_coplanar_respects_material_boundaries() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        // Two coplanar triangles of one quad, but different materials.
        let tris = vec![vec![0usize, 1, 2], vec![0, 2, 3]];
        let mats = vec![0u32, 1u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 2];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 2];

        let (f, _, _, _) = merge_coplanar(&tris, &mats, &uvs, &holes, &positions, tol::PLANE_DIST);
        assert_eq!(f.len(), 2, "different materials are not merged");
    }

    /// A point within tol of an endpoint is NOT a T-junction (no false splice).
    #[test]
    fn point_on_open_segment_excludes_endpoints() {
        let a = Point3::new(0.0, 0.0, 0.0);
        let b = Point3::new(1.0, 0.0, 0.0);
        assert!(point_on_open_segment(Point3::new(0.5, 0.0, 0.0), a, b).is_some());
        assert!(point_on_open_segment(a, a, b).is_none(), "endpoint a");
        assert!(point_on_open_segment(b, a, b).is_none(), "endpoint b");
        // Off the segment (perpendicular distance too large).
        assert!(point_on_open_segment(Point3::new(0.5, 0.5, 0.0), a, b).is_none());
    }

    /// Y-up → Z-up transform: point (0,1,0) in Y-up lands at (0,0,1) in Z-up.
    #[test]
    fn y_up_to_z_up_maps_correctly() {
        let tf = y_up_to_z_up();
        let p = tf.apply_point(Point3::new(0.0, 1.0, 0.0));
        assert!((p.x).abs() < 1e-9);
        assert!((p.y).abs() < 1e-9);
        assert!((p.z - 1.0).abs() < 1e-9, "Y→Z: z={}", p.z);
    }

    /// Unit scale: 100 cm units → 1 m.
    #[test]
    fn unit_scale_centimeters() {
        let tf = world_transform(0.01, "Z_UP");
        let p = tf.apply_point(Point3::new(100.0, 0.0, 0.0));
        // unit_meter is f32 (0.01f32); f32→f64 precision loss is ~1e-8.
        assert!((p.x - 1.0).abs() < 1e-6, "100 cm → 1 m: x={}", p.x);
    }
}
