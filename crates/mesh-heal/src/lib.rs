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
//! 0. **Index validation** — a face loop referencing a vertex index outside
//!    the position array (corrupt or crafted import data) is foreign garbage:
//!    the face is dropped (a garbage hole loop is dropped, its face kept), so
//!    untrusted input can never panic the pipeline.
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

use kernel::{Plane, Point3, Transform, Vec3, tol};

/// Non-manifold mesh splitting: cut kernel-rejectable meshes into
/// buildable open shells (never repaired, always reported by the caller).
pub mod split;
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

/// Full output of [`heal_mesh_with_tol`]: (positions, faces, face_materials,
/// face_corner_uvs, face_holes, dropped_degenerate_count) — the final `usize`
/// is step 7a's kernel-degenerate drop count, which callers surface in their
/// import warnings (a geometry-altering drop is never silent).
type HealedMeshWithStats = (
    Vec<Point3>,
    Vec<Vec<usize>>,
    Vec<u32>,
    FaceCornerUvs,
    FaceHoles,
    usize,
);

use std::collections::BTreeMap;

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
    let mut cell_to_rep: BTreeMap<(i64, i64, i64), usize> = BTreeMap::new();
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
                    // Saturating: corrupted input can carry coordinates that
                    // bucket to i64::MAX/MIN, where +1 overflows in debug.
                    let probe = (
                        key.0.saturating_add(dk0),
                        key.1.saturating_add(dk1),
                        key.2.saturating_add(dk2),
                    );
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
        let mut seen = std::collections::BTreeSet::new();
        if !remapped.iter().all(|i| seen.insert(*i)) || remapped.len() < 3 {
            // Outer degenerated → drop face and its holes.
            continue;
        }
        // Remap hole loops; drop degenerate holes (keep face).
        let remapped_holes: Vec<Vec<usize>> = holes
            .iter()
            .filter_map(|hole| {
                let h: Vec<usize> = hole.iter().map(|&i| old_to_new[i]).collect();
                let mut seen_h = std::collections::BTreeSet::new();
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

/// Drop faces whose outer loop references a vertex index outside the position
/// array, and hole loops likewise (the face is kept when only a hole is
/// garbage). Runs before anything else in [`heal_mesh_with_tol`] so untrusted
/// import data (a crafted glTF index buffer, a corrupt source file) can never
/// panic the pipeline with an out-of-bounds index.
///
/// This is **boundary normalization** at the import seam: an index that
/// points outside the mesh's own vertex array is foreign garbage that no
/// healing step could interpret, not kernel geometry (DEVELOPMENT.md rule 4
/// applies to kernel operations; this is pre-kernel filtering, the same
/// category as `drop_zero_area_faces`).
fn drop_out_of_range_faces(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    position_count: usize,
) -> FilteredFaces {
    let mut out_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    let mut out_mats: Vec<u32> = Vec::with_capacity(faces.len());
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(faces.len());
    let mut out_holes: FaceHoles = Vec::with_capacity(faces.len());

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
        if !face.iter().all(|&i| i < position_count) {
            continue; // garbage outer loop → drop the face (and its holes)
        }
        let valid_holes: Vec<Vec<usize>> = holes
            .iter()
            .filter(|hole| hole.iter().all(|&i| i < position_count))
            .cloned()
            .collect();
        out_faces.push(face.clone());
        out_mats.push(mat);
        out_uvs.push(corner_uvs.clone());
        out_holes.push(valid_holes);
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
    let mut seen: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
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
/// and within `on_tol` perpendicular distance — return its parameter
/// `t ∈ (0, 1)` (so `closest = a + (b−a)·t`). Otherwise `None`.
///
/// "Strictly between" excludes points within `on_tol` of either endpoint, so
/// shared corners are never treated as T-junctions.
///
/// `on_tol` is the caller's weld tolerance: a genuine T-vertex from an
/// f32-quantised source sits off the exact edge line by the same order of
/// noise the weld pass merges, so the on-segment gate must scale with it
/// (the native `tol::POINT_MERGE` would reject every f32-sourced T-junction).
fn point_on_open_segment(p: Point3, a: Point3, b: Point3, on_tol: f64) -> Option<f64> {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= on_tol * on_tol {
        return None; // degenerate edge
    }
    let len = len_sq.sqrt();
    let t = (p - a).dot(ab) / len_sq;
    // Reject if the foot of the perpendicular is within tol of either endpoint.
    let margin = on_tol / len;
    if t <= margin || t >= 1.0 - margin {
        return None;
    }
    let closest = a + ab * t;
    if (p - closest).length_squared() <= on_tol * on_tol {
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

/// Uniform spatial grid over the T-junction candidate vertices, using the
/// same quantized bucketing as welding ([`bucket_key`]). Built once per
/// [`split_t_junctions`] call so each edge queries only the vertices near its
/// own segment instead of linearly scanning every candidate in the mesh —
/// the full scan is O(edges × vertices), quadratic in mesh size, and
/// dominates large SketchUp imports.
///
/// The grid is pruning-only: [`CandidateGrid::gather`] returns a superset of
/// every candidate that can lie within `on_tol` (the caller's on-segment
/// tolerance) of the queried segment, and the exact
/// [`point_on_open_segment`] predicate then decides membership — so per-edge
/// results are identical to the full scan.
struct CandidateGrid {
    /// Cell edge length (meters): the mesh's mean outer-loop edge length, so
    /// a typical edge overlaps O(1) cells and a typical cell holds O(1)
    /// vertices. The cell size itself is query granularity, not a geometric
    /// tolerance; it is floored at `2·on_tol` so the sample-spacing budget
    /// `cell − on_tol` in [`CandidateGrid::gather`] stays positive (with at
    /// least half the cell as budget).
    cell: f64,
    /// The on-segment tolerance [`point_on_open_segment`] is queried with —
    /// the pruning radius the gather walk must cover.
    on_tol: f64,
    /// `bucket_key(position, cell)` → candidate vertex ids, ascending.
    cells: BTreeMap<(i64, i64, i64), Vec<usize>>,
    /// Lazily memoized 3×3×3-neighbourhood unions (sorted vertex ids), keyed
    /// by the centre cell. The cell size tracks the mean edge length, so a
    /// typical edge samples only one or two distinct cells — memoizing the
    /// union lets the O(edges) queries share it instead of re-probing 27
    /// cells each. Bounded: each occupied cell stores at most its 27
    /// neighbours' ids, and only cells some edge actually samples are built.
    neighborhoods: BTreeMap<(i64, i64, i64), Vec<usize>>,
}

impl CandidateGrid {
    /// Bucket `candidates` (ascending vertex ids) by quantized position.
    /// `on_tol` is the on-segment tolerance the grid's queries must cover.
    fn build(
        candidates: &[usize],
        positions: &[Point3],
        faces: &[Vec<usize>],
        on_tol: f64,
    ) -> Self {
        // Mean outer-loop edge length (the same edges gather() is queried
        // with). A non-finite mean (garbage coordinates) falls to the floor;
        // gather() then serves such edges via its full-scan fallback.
        let mut total = 0.0_f64;
        let mut count = 0usize;
        for face in faces {
            let n = face.len();
            for k in 0..n {
                let ab = positions[face[(k + 1) % n]] - positions[face[k]];
                total += ab.length();
                count += 1;
            }
        }
        let mean = if count > 0 { total / count as f64 } else { 0.0 };
        // Floor: cell must exceed on_tol for gather()'s sample-spacing
        // budget `cell − on_tol` to stay positive; 2·on_tol keeps at least
        // half the cell as budget.
        let min_cell = 2.0 * on_tol;
        let cell = if mean.is_finite() {
            mean.max(min_cell)
        } else {
            min_cell
        };

        let mut cells: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
        for &v in candidates {
            cells
                .entry(bucket_key(positions[v], cell))
                .or_default()
                .push(v);
        }
        Self {
            cell,
            on_tol,
            cells,
            neighborhoods: BTreeMap::new(),
        }
    }

    /// The sorted union of the candidate ids in the 3×3×3 cells around
    /// `base`, memoized. Ids are unique without a dedup: every candidate
    /// lives in exactly one cell. (Associated fn over disjoint field borrows
    /// so the memo can be grown while `cells` is read.)
    fn neighborhood<'a>(
        cells: &BTreeMap<(i64, i64, i64), Vec<usize>>,
        memo: &'a mut BTreeMap<(i64, i64, i64), Vec<usize>>,
        base: (i64, i64, i64),
    ) -> &'a [usize] {
        memo.entry(base).or_insert_with(|| {
            let mut ids: Vec<usize> = Vec::new();
            for dx in -1i64..=1 {
                for dy in -1i64..=1 {
                    for dz in -1i64..=1 {
                        // Saturating for the same reason as weld_with_tol's probe.
                        let key = (
                            base.0.saturating_add(dx),
                            base.1.saturating_add(dy),
                            base.2.saturating_add(dz),
                        );
                        if let Some(cell_ids) = cells.get(&key) {
                            ids.extend_from_slice(cell_ids);
                        }
                    }
                }
            }
            ids.sort_unstable();
            ids
        })
    }

    /// Collect into `out` a superset of every candidate vertex that can lie
    /// within `on_tol` of segment `(pa, pb)`, in ascending vertex-id order
    /// (so downstream stable sorting ties out exactly like the full scan,
    /// which also visits candidates in ascending order).
    ///
    /// Coverage argument — why no on-segment vertex is ever missed: sample
    /// points are placed along the segment at Euclidean spacing
    /// ≤ `cell − on_tol`. A hit vertex `p` lies within `on_tol` of
    /// some segment point `q`, and `q` lies within one sample spacing of its
    /// nearest sample `s`, so per axis `|p − s| ≤ (cell − on_tol) +
    /// on_tol = cell` — and two coordinates at most one cell apart
    /// quantize (monotonic floor, [`bucket`]) to indices at most 1 apart.
    /// Every possible hit therefore lives in the 3×3×3 cell neighbourhood of
    /// some sample's cell, all of which are gathered. The one-cell inflation
    /// also covers a vertex sitting exactly on a cell boundary: floor
    /// assigns it to one side deterministically, and both sides are visited.
    ///
    /// Degenerate segments (`pa == pb` within tolerance) gather their local
    /// neighbourhood; the predicate then rejects every vertex, exactly as
    /// the full scan does. A segment so long (or non-finite) that walking it
    /// would visit more cells than the grid has falls back to gathering
    /// every occupied cell — never worse than the full scan, and the same
    /// superset guarantee holds trivially.
    fn gather(&mut self, pa: Point3, pb: Point3, out: &mut Vec<usize>) {
        out.clear();

        let ab = pb - pa;
        let len = ab.length();
        let max_step = self.cell - self.on_tol; // > 0: cell ≥ 2·on_tol
        let steps = (len / max_step).ceil();
        if !steps.is_finite() || steps > self.cells.len() as f64 {
            // Full-scan fallback (cost cap + non-finite guard). Cells are in
            // key order with ascending ids inside; a global sort restores
            // ascending vertex-id order across cells.
            for ids in self.cells.values() {
                out.extend_from_slice(ids);
            }
            out.sort_unstable();
            return;
        }

        let n = (steps as usize).max(1);
        // Each coordinate of `pa + ab·t` is weakly monotone in `t` (even under
        // rounding), so sample cell keys repeat only consecutively — skipping
        // a repeated key never skips a cell seen earlier in the walk. Distinct
        // samples' 3×3×3 neighbourhoods can still overlap, so ids are deduped
        // after the sort (a stray duplicate is harmless there, so per-axis
        // monotonicity is a perf observation, not a correctness requirement).
        let mut prev_base: Option<(i64, i64, i64)> = None;
        let mut sampled_cells = 0usize;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let s = pa + ab * t;
            let base = bucket_key(s, self.cell);
            if prev_base == Some(base) {
                continue;
            }
            prev_base = Some(base);
            sampled_cells += 1;
            out.extend_from_slice(Self::neighborhood(
                &self.cells,
                &mut self.neighborhoods,
                base,
            ));
        }
        // A single sampled cell yields its memoized union verbatim — already
        // sorted and unique. Multiple cells' unions can overlap: sort to
        // restore the ascending vertex-id order the full scan iterates in,
        // then dedup the overlap.
        if sampled_cells > 1 {
            out.sort_unstable();
            out.dedup();
        }
    }
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
/// A face's own vertices are never spliced into its own loop (a weakly-simple
/// polygon whose corner touches the interior of one of its other edges would
/// otherwise come out with a repeated index, which `from_polygons` rejects as
/// `DegenerateFace` — turning valid input into a skipped face).
///
/// `on_tol` is the on-segment tolerance (see [`point_on_open_segment`]):
/// callers pass the same weld tolerance the mesh was welded with, so
/// f32-quantised sources recognise T-vertices at their own precision. It is
/// floored at `tol::POINT_MERGE`, the tightest meaningful gate.
///
/// Hole loops are threaded through unchanged (conservative: hole edges are not
/// spliced). Face order and count are preserved so the parallel `face_holes`
/// array stays aligned.
///
/// Candidate lookup goes through a [`CandidateGrid`] (one build per call), so
/// the cost is near-linear in mesh size instead of the full scan's
/// O(edges × vertices) — with per-edge results identical to that scan (the
/// grid only prunes vertices provably beyond the on-segment tolerance;
/// see `split_t_junctions_reference` and the property test pinning identity).
pub fn split_t_junctions(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
    on_tol: f64,
) -> FilteredFaces {
    split_t_junctions_impl(
        faces,
        face_materials,
        face_corner_uvs,
        face_holes,
        positions,
        on_tol,
        true,
    )
}

/// Reference implementation of [`split_t_junctions`] with the candidate grid
/// bypassed — an honest linear scan of every candidate vertex per edge, kept
/// so the executable specs and property tests can assert the grid path
/// returns byte-for-byte identical results (DEVELOPMENT.md rule 3),
/// mirroring `ear_clip_reference` in crates/tessellate. Not part of the
/// supported API.
#[cfg(test)]
fn split_t_junctions_reference(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
    on_tol: f64,
) -> FilteredFaces {
    split_t_junctions_impl(
        faces,
        face_materials,
        face_corner_uvs,
        face_holes,
        positions,
        on_tol,
        false,
    )
}

/// Shared body of `split_t_junctions`/`split_t_junctions_reference`: with
/// `use_grid` set, each edge tests only the candidates a [`CandidateGrid`]
/// gathers near its segment; without it, every candidate is tested. Both
/// paths visit candidates in ascending vertex-id order and share
/// [`point_on_open_segment`] and the stable sort by parameter `t`, so they
/// emit identical output on any input.
fn split_t_junctions_impl(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
    on_tol: f64,
    use_grid: bool,
) -> FilteredFaces {
    // The tightest meaningful on-segment gate is the native point-merge
    // tolerance; floor a degenerate caller tolerance there so the grid's
    // sample-spacing budget stays positive.
    let on_tol = on_tol.max(tol::POINT_MERGE);

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

    let mut grid = use_grid.then(|| CandidateGrid::build(&candidates, positions, faces, on_tol));
    // Per-edge scratch, reused across the loop to avoid reallocation.
    let mut near: Vec<usize> = Vec::new();

    let mut out_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(faces.len());

    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (face, corner_uvs) in faces.iter().zip(face_corner_uvs.iter()) {
        let n = face.len();
        // UVs are usable only when there is one per corner.
        let has_uv = corner_uvs.len() == n && n > 0;
        let mut new_face: Vec<usize> = Vec::with_capacity(n);
        let mut new_uv: Vec<[f64; 2]> = Vec::with_capacity(n);

        // Vertices already on this face's boundary: its own corners, plus
        // any vertex spliced into one of its earlier edges. A face's own
        // vertex must never be spliced into its own loop — the repeated
        // index would make `from_polygons` reject the (otherwise valid)
        // face as `DegenerateFace`.
        let mut boundary: std::collections::BTreeSet<usize> = face.iter().copied().collect();

        for k in 0..n {
            let a = face[k];
            let b = face[(k + 1) % n];
            new_face.push(a);
            if has_uv {
                new_uv.push(corner_uvs[k]);
            }

            // Collect interior vertices on edge (a, b), sorted by parameter.
            // With the grid, only candidates near the segment are tested — a
            // superset of every possible hit (see CandidateGrid::gather), in
            // the same ascending vertex-id order as the full scan, so the
            // stable sort below breaks equal-t ties identically.
            let pa = positions[a];
            let pb = positions[b];
            let edge_candidates: &[usize] = match grid.as_mut() {
                Some(g) => {
                    g.gather(pa, pb, &mut near);
                    &near
                }
                None => &candidates,
            };
            let mut hits: Vec<(f64, usize)> = edge_candidates
                .iter()
                .filter(|&&v| !boundary.contains(&v))
                .filter_map(|&v| {
                    point_on_open_segment(positions[v], pa, pb, on_tol).map(|t| (t, v))
                })
                .collect();
            hits.sort_by(|x, y| x.0.total_cmp(&y.0));

            for (t, v) in hits {
                boundary.insert(v);
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
///
/// Tetrahedra are summed relative to the shell's own first vertex, not the
/// world origin: for a closed shell the sum is independent of the reference
/// point, but summing from the origin makes every term O(offset³) for a shell
/// baked far from the origin, and the near-cancelling sum's rounding error
/// (O(offset³·ε)) can exceed the true O(extent³) volume and flip its sign.
fn signed_volume6(faces: &[Vec<usize>], positions: &[Point3]) -> f64 {
    let Some(reference) = faces.iter().find(|f| f.len() >= 3).map(|f| positions[f[0]]) else {
        return 0.0;
    };
    let mut v6 = 0.0;
    for face in faces {
        if face.len() < 3 {
            continue;
        }
        let p0 = positions[face[0]] - reference;
        for i in 1..face.len() - 1 {
            let p1 = positions[face[i]] - reference;
            let p2 = positions[face[i + 1]] - reference;
            v6 += p0.dot(p1.cross(p2));
        }
    }
    v6
}

/// Winding-number magnitude above which a point counts as enclosed by a
/// closed shell. The generalized winding number is ±1 for a point strictly
/// inside a closed oriented shell and 0 strictly outside; the half-integer
/// midpoint discriminates the two robustly (this is a topological threshold,
/// not a length tolerance).
const WINDING_ENCLOSED: f64 = 0.5;

/// Generalized winding number of point `p` with respect to an oriented face
/// set: the sum of the signed solid angles its fan triangles subtend at `p`
/// (Van Oosterom–Strackee), divided by 4π. For a closed shell this is ±1
/// when `p` is strictly inside (sign per orientation) and 0 strictly
/// outside, and it needs no epsilon tie-breaking on rays through edges or
/// vertices — unlike ray-parity counting.
fn winding_number(p: Point3, faces: &[Vec<usize>], positions: &[Point3]) -> f64 {
    let mut total = 0.0_f64;
    for face in faces {
        if face.len() < 3 {
            continue;
        }
        let a = positions[face[0]] - p;
        for i in 1..face.len() - 1 {
            let b = positions[face[i]] - p;
            let c = positions[face[i + 1]] - p;
            let (la, lb, lc) = (a.length(), b.length(), c.length());
            let det = a.dot(b.cross(c));
            let denom = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
            total += 2.0 * det.atan2(denom);
        }
    }
    total / (4.0 * std::f64::consts::PI)
}

/// True if every directed edge `(a, b)` of the face set has its reverse
/// `(b, a)` — i.e. the mesh is closed (manifold or not). Orientation is only
/// well-defined for a closed shell, so we gate the flip on this.
fn is_closed(faces: &[Vec<usize>]) -> bool {
    use std::collections::BTreeSet;
    let mut dir: BTreeSet<(usize, usize)> = BTreeSet::new();
    for face in faces {
        let n = face.len();
        for k in 0..n {
            dir.insert((face[k], face[(k + 1) % n]));
        }
    }
    dir.iter().all(|&(a, b)| dir.contains(&(b, a)))
}

/// Normalize each **closed** shell to outward orientation: if a shell's signed
/// volume is negative that solid is inside-out (e.g. SketchUp faces reversed in
/// the source model — invisible there because SketchUp renders double-sided,
/// but a single-sided renderer culls every face and push/pull inverts). Reverse
/// each of its faces' winding (and corner UVs) so normals point outward.
///
/// The decision is made **per connected component** (faces joined by shared
/// undirected edges — the same adjacency [`orient_consistent`]'s flood fill
/// walks): one raw mesh can carry several disjoint shells (a multi-primitive
/// glTF mesh, a SketchUp geometry run, a multi-group COLLADA `<mesh>`), and a
/// global closed/volume test would let the largest shell mask an inside-out
/// sibling — or wrongly flip a correct one.
///
/// A closed negative shell **enclosed by** a closed positive shell is left
/// alone: that is a hollow solid's cavity, and inward winding is its correct
/// orientation (the kernel's boolean subtraction emits exactly this shape —
/// cavity walls facing into the removed volume). Only free-standing
/// inside-out shells are flipped.
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
    let n = faces.len();

    // Union faces across shared undirected edges into components. Hole rings
    // participate in adjacency too: a face filling another face's hole pairs
    // its outer edges against the hole's edges, and the two must share one
    // flip decision — flipping only one side would invert a clean pairing
    // into duplicate directed edges.
    let mut uf = UnionFind::new(n);
    let mut edge_first: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let empty_holes: Vec<Vec<usize>> = Vec::new();
    for (fi, face) in faces.iter().enumerate() {
        let holes = face_holes.get(fi).unwrap_or(&empty_holes);
        for ring in std::iter::once(face).chain(holes.iter()) {
            let m = ring.len();
            for k in 0..m {
                let (a, b) = (ring[k], ring[(k + 1) % m]);
                let key = if a < b { (a, b) } else { (b, a) };
                match edge_first.get(&key) {
                    Some(&f0) => uf.union(f0, fi),
                    None => {
                        edge_first.insert(key, fi);
                    }
                }
            }
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for fi in 0..n {
        components.entry(uf.find(fi)).or_default().push(fi);
    }

    // Classify each component once: its faces, whether it is closed, and its
    // signed volume.
    struct Comp {
        face_ids: Vec<usize>,
        faces: Vec<Vec<usize>>,
        closed: bool,
        volume6: f64,
    }
    let comps: Vec<Comp> = components
        .into_values()
        .map(|face_ids| {
            let comp_faces: Vec<Vec<usize>> = face_ids.iter().map(|&f| faces[f].clone()).collect();
            let closed = is_closed(&comp_faces);
            let volume6 = if closed {
                signed_volume6(&comp_faces, positions)
            } else {
                0.0
            };
            Comp {
                face_ids,
                faces: comp_faces,
                closed,
                volume6,
            }
        })
        .collect();

    // Per component: only a closed, inside-out, FREE-STANDING shell is
    // flipped. A closed negative shell enclosed by a closed positive one is
    // a hollow-solid cavity whose inward winding is the kernel's own
    // convention (boolean subtraction emits cavity walls facing into the
    // removed volume) — "correcting" it would turn a hollow solid inside
    // out. Enclosure is tested by the winding number of one of the negative
    // shell's vertices with respect to each positive closed shell.
    let mut flip = vec![false; n];
    for (i, comp) in comps.iter().enumerate() {
        if !comp.closed || comp.volume6 >= 0.0 {
            continue;
        }
        let Some(probe) = comp.faces.iter().find(|f| !f.is_empty()).map(|f| f[0]) else {
            continue;
        };
        let p = positions[probe];
        let enclosed = comps.iter().enumerate().any(|(j, other)| {
            j != i
                && other.closed
                && other.volume6 > 0.0
                && winding_number(p, &other.faces, positions).abs() > WINDING_ENCLOSED
        });
        if !enclosed {
            for &f in &comp.face_ids {
                flip[f] = true;
            }
        }
    }

    apply_winding_flips(&flip, faces, face_materials, face_corner_uvs, face_holes)
}

/// Emit `(faces, materials, corner_uvs, holes)` with every face whose `flip`
/// flag is set reversed (outer loop, corner UVs, and hole loops together).
/// Shared by [`orient_outward`] and [`orient_consistent`].
fn apply_winding_flips(
    flip: &[bool],
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
) -> FilteredFaces {
    let n = faces.len();
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
    let mut edge_faces: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
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

    apply_winding_flips(&flip, faces, face_materials, face_corner_uvs, face_holes)
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
    let mut edge_faces: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
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
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
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
        let mut dir: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
        for &fi in group {
            let face = &faces[fi];
            let n = face.len();
            for k in 0..n {
                dir.insert((face[k], face[(k + 1) % n]));
            }
        }
        // succ: vertex a → (b, uv_at_a). Reject (fallback) if a vertex has >1
        // outgoing boundary edge (pinch / non-simple boundary). A BTreeMap, not
        // a HashMap: the loop walk below seeds at `succ.keys().next()`, so the
        // merged polygon's start vertex (hence its winding rotation) must be the
        // deterministic smallest id, not a hash-seed-dependent one.
        let mut succ: BTreeMap<usize, (usize, Option<[f64; 2]>)> = BTreeMap::new();
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
        let distinct: std::collections::BTreeSet<usize> = loop_verts.iter().copied().collect();
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
///
/// NOTE: this wrapper discards [`heal_mesh_with_tol`]'s dropped-degenerate
/// face count, so its callers (dae-import, skp-import) do not yet surface
/// that drop in their import warnings the way stl-import and gltf-import do.
/// Wiring them is a known follow-up; switch those callers to
/// `heal_mesh_with_tol` when doing it.
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
    let (positions, faces, mats, uvs, holes, _dropped_degenerate) = heal_mesh_with_tol(
        raw_positions,
        raw_faces,
        raw_face_materials,
        raw_face_corner_uvs,
        raw_face_holes,
        bake_tf,
        tol::POINT_MERGE,
        tol::PLANE_DIST,
    );
    (positions, faces, mats, uvs, holes)
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
///
/// The final `usize` is the number of faces step 7a removed as
/// kernel-degenerate ([`drop_kernel_degenerate_faces`]) — callers surface it
/// in their import warnings so a geometry-altering drop is never silent.
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
) -> HealedMeshWithStats {
    // 0. Validate face/hole vertex indices against the position array:
    //    untrusted import data (crafted glTF index buffers, corrupt sources)
    //    must be dropped here, never panic a later unchecked index.
    let (valid_faces, valid_mats, valid_uvs, valid_holes) = drop_out_of_range_faces(
        raw_faces,
        raw_face_materials,
        raw_face_corner_uvs,
        raw_face_holes,
        raw_positions.len(),
    );

    // 1. Apply bake transform (unit + up-axis for world meshes; identity for defs).
    let transformed: Vec<Point3> = raw_positions
        .iter()
        .map(|&p| bake_tf.apply_point(p))
        .collect();

    // 2. Weld.
    let (unique_positions, old_to_new) = weld_with_tol(&transformed, weld_tol);
    let (remapped_faces, remapped_mats, remapped_uvs, remapped_holes) = remap_faces(
        &valid_faces,
        &valid_mats,
        &valid_uvs,
        &valid_holes,
        &old_to_new,
    );

    // 3. Drop zero-area faces (collinear T-junction slivers from SketchUp).
    //    The sliver gate is on effective height at `weld_tol` — the source's
    //    own precision — never on raw size.
    //    This is boundary normalization at the import seam: these are foreign
    //    input artefacts, not kernel invariant violations (DEVELOPMENT.md rule 4).
    let (nondegenerate_faces, nondegenerate_mats, nondegenerate_uvs, nondegenerate_holes) =
        drop_zero_area_faces(
            &remapped_faces,
            &remapped_mats,
            &remapped_uvs,
            &remapped_holes,
            &unique_positions,
            weld_tol,
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
    //    The on-segment gate scales with `weld_tol`: an f32-quantised source's
    //    T-vertices sit off the exact edge line by weld-scale noise, which the
    //    native 1 nm gate would reject wholesale.
    //    Hole loops are threaded through unchanged (conservative).
    let (split_faces, split_mats, split_uvs, split_holes) = split_t_junctions(
        &dedup_faces,
        &dedup_mats,
        &dedup_uvs,
        &dedup_holes,
        &unique_positions,
        weld_tol,
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
    let (merged_faces, merged_mats, merged_uvs, merged_holes) = merge_coplanar(
        &oriented_faces,
        &oriented_mats,
        &oriented_uvs,
        &oriented_holes,
        &unique_positions,
        merge_plane_tol,
    );

    // 7a. Drop any face the KERNEL would reject as degenerate. `drop_zero_area_faces`
    //    (step 3) gates on effective HEIGHT relative to `weld_tol` — the source's
    //    coincidence precision — but `Object::from_polygons`'s degeneracy gate is
    //    `Plane::from_polygon`, which fails when the Newell normal magnitude (= 2×
    //    area) falls below the ABSOLUTE floor `tol::NORMALIZE_MIN_LENGTH`. A small
    //    sliver can clear the height gate yet trip the absolute one, and because
    //    `Document::ingest` builds each object all-or-nothing, ONE such face out of
    //    ~110k skips the ENTIRE model (as a 3DBenchy-class real-world STL routinely
    //    does). Filter with the kernel's OWN predicate here so the healer's output
    //    is buildable by construction — the two agree, no threshold drift. This is
    //    import-boundary normalization (foreign-input artefact), not silent kernel
    //    repair; a dropped face may open the shell, and the object then arrives
    //    honestly LEAKY rather than skipped. The count is returned so callers put
    //    the drop in their import warnings — never silent.
    //    Placement is invariant for this face class: `merge_coplanar` can neither
    //    absorb nor produce one (its adjacency test requires `face_normal` — the
    //    same Newell criterion — to succeed on both sides), so filtering before or
    //    after step 7 yields the same result; after keeps the pipeline's
    //    drop-passes-last shape.
    let (final_faces, final_mats, final_uvs, final_holes) = drop_kernel_degenerate_faces(
        &merged_faces,
        &merged_mats,
        &merged_uvs,
        &merged_holes,
        &unique_positions,
    );
    let dropped_degenerate = merged_faces.len() - final_faces.len();

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
        dropped_degenerate,
    )
}

/// Drop degenerate sliver faces: those whose effective height (relative to
/// their longest edge) is at or below `weld_tol`.
///
/// Triangles from SketchUp can include collinear slivers (three points on a
/// line, zero cross-product). The general polygon case is handled by testing
/// the fan-triangulation area sum. Only triangles degenerate in this way in
/// practice (SketchUp always emits triangles); the fan check is correct for
/// all convex polygons.
///
/// The gate is on sliver HEIGHT, not raw area — a raw-area floor is
/// dimensionally wrong (the fan sum is m⁴, a tolerance floor m²) and turns
/// into a shape-blind size cull at f32-scale tolerances, dropping well-formed
/// small triangles. A face is a sliver when its total fan area `A` satisfies
/// `2A ≤ longest_edge · weld_tol`, i.e. its effective height
/// `2A / longest_edge` is within the source's own coincidence precision
/// (`weld_tol`, the native `tol::POINT_MERGE` for f64 sources): every point
/// of the face is within weld-noise of its longest edge's line, degenerate at
/// that source's precision regardless of how long the face is.
///
/// This is **boundary normalization** at the import seam: zero-area sliver
/// triangles from SketchUp T-junction collinear vertices are foreign input
/// artefacts, not kernel invariant violations. Dropping them here is safe and
/// explicitly documented (DEVELOPMENT.md rule 4 applies to kernel operations;
/// this is pre-kernel filtering).
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
    weld_tol: f64,
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
        // Fan-triangulate from face[0] and accumulate cross-product
        // magnitudes: Σ|cross| = Σ 2·triangle_area = 2·A (total fan area).
        let p0 = positions[face[0]];
        let mut area2 = 0.0_f64; // 2 × total fan area
        for i in 1..(face.len() - 1) {
            let p1 = positions[face[i]];
            let p2 = positions[face[i + 1]];
            let v1 = p1 - p0;
            let v2 = p2 - p0;
            area2 += v1.cross(v2).length();
        }
        // Longest outer-loop edge (squared — compared squared to avoid the
        // sqrt): the base the effective height is measured against.
        let m = face.len();
        let mut longest_edge_sq = 0.0_f64;
        for k in 0..m {
            let e = positions[face[(k + 1) % m]] - positions[face[k]];
            longest_edge_sq = longest_edge_sq.max(e.length_squared());
        }
        // Keep iff effective height 2A/longest_edge exceeds weld_tol,
        // i.e. (2A)² > longest_edge² · weld_tol².
        if area2 * area2 > longest_edge_sq * weld_tol * weld_tol {
            out_faces.push(face.clone());
            out_mats.push(mat);
            out_uvs.push(corner_uvs.clone());
            out_holes.push(holes.clone());
        }
        // Faces at or below the height gate are collinear slivers;
        // drop silently (import boundary normalization, not silent repair).
    }

    (out_faces, out_mats, out_uvs, out_holes)
}

/// Drop any face the kernel's `Object::from_polygons` would reject as a
/// `DegenerateFace` — i.e. one whose outer loop `Plane::from_polygon` cannot
/// fit a plane through (Newell normal magnitude below the ABSOLUTE floor
/// `tol::NORMALIZE_MIN_LENGTH`). Uses the kernel's own predicate so the healer
/// and the kernel agree exactly, with no threshold drift: a near-degenerate
/// sliver that clears `drop_zero_area_faces`'s scale-relative height gate but
/// trips the kernel's absolute one is removed HERE, at the import boundary,
/// instead of skipping the whole object at ingest (which is all-or-nothing per
/// object — one bad face out of a real STL's ~110k would drop the entire
/// model).
///
/// Only the OUTER loop is tested (that is what `Plane::from_polygon` fits in
/// `from_polygons_impl`); hole loops ride with their face. A dropped face may
/// open the shell — the object then arrives honestly leaky, never patched or
/// silently repaired (DEVELOPMENT.md rule 4 governs kernel operations; this is
/// pre-kernel boundary filtering, the same category as `drop_zero_area_faces`).
///
/// CAVEAT — this predicate matches the kernel's `DegenerateFace` only in
/// combination with the upstream pipeline: the kernel ALSO raises
/// `DegenerateFace` for a repeated vertex index within a loop
/// (`build_validated_loop`), which a plane fit cannot detect (a self-revisiting
/// loop can have a nonzero Newell normal). That trigger cannot fire here today
/// because every path into step 7a guarantees repeat-free loops —
/// `remap_faces` drops post-weld repeats, `merge_coplanar` re-checks
/// distinctness, and `split_t_junctions` never splices a face's own vertex
/// back into its loop. Any new step inserted between the weld and 7a (or a
/// caller bypassing `remap_faces`) must preserve that invariant, or a repeat
/// face will pass this filter and reopen the whole-object ingest skip.
fn drop_kernel_degenerate_faces(
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
    positions: &[Point3],
) -> FilteredFaces {
    let mut out_faces = Vec::with_capacity(faces.len());
    let mut out_mats = Vec::with_capacity(faces.len());
    let mut out_uvs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(faces.len());
    let mut out_holes: FaceHoles = Vec::with_capacity(faces.len());

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
        let pts: Vec<Point3> = face.iter().map(|&i| positions[i]).collect();
        // Keep iff the kernel could fit a plane through the outer loop — the
        // exact `from_polygons` DegenerateFace criterion.
        if Plane::from_polygon(&pts).is_ok() {
            out_faces.push(face.clone());
            out_mats.push(mat);
            out_uvs.push(corner_uvs.clone());
            out_holes.push(holes.clone());
        }
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
            split_t_junctions(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE);

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

    /// Vertices sitting EXACTLY on candidate-grid cell boundaries are still
    /// found, and the grid path matches the full-scan reference byte for
    /// byte. The mesh is built so the mean edge length — hence the cell
    /// size — is exactly 1.0 m: 6 + 4 unit-sum quad edges plus two
    /// zero-length edges from a degenerate 2-gon ring (10 m over 10 edges),
    /// which puts every integer-coordinate vertex exactly on a cell
    /// boundary in all three axes and exercises degenerate (a == b within
    /// tolerance) edges at the same time.
    #[test]
    fn split_t_junctions_grid_matches_reference_on_cell_boundaries() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0), // 0
            Point3::new(2.0, 0.0, 0.0), // 1
            Point3::new(2.0, 1.0, 0.0), // 2
            Point3::new(0.0, 1.0, 0.0), // 3
            Point3::new(1.0, 0.0, 0.0), // 4 — midpoint of edge (0,1)
            Point3::new(1.0, 1.0, 0.0), // 5 — midpoint of edge (2,3)
            Point3::new(5.0, 5.0, 0.0), // 6 — degenerate 2-gon vertex
        ];
        let faces = vec![
            vec![0usize, 1, 2, 3], // edges of length 2, 1, 2, 1
            vec![0usize, 4, 5, 3], // edges of length 1, 1, 1, 1
            vec![6usize, 6],       // two zero-length edges (degenerate ring)
        ];
        let mats = vec![0u32, 1, 2];
        let uvs: Vec<Vec<[f64; 2]>> = vec![
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            Vec::new(),
            Vec::new(),
        ];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 3];

        let grid = split_t_junctions(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE);
        let reference =
            split_t_junctions_reference(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE);
        assert_eq!(grid, reference, "grid path must equal the full scan");
        assert_eq!(
            grid.0[0],
            vec![0, 4, 1, 2, 5, 3],
            "both boundary-sitting mid-edge vertices spliced in"
        );
        assert_eq!(grid.0[2], vec![6, 6], "degenerate ring passes through");
    }

    /// Grid ≡ reference on a mesh stressing the gather walk: an edge long
    /// enough to span many grid cells (and to trip the full-scan cost-cap
    /// fallback), plus candidates a hair inside and a hair outside the
    /// on-segment tolerance — the borderline hits/misses must be decided
    /// identically by both paths.
    #[test]
    fn split_t_junctions_grid_matches_reference_on_long_edges_and_tol_probes() {
        let mut positions = vec![
            Point3::new(0.0, 0.0, 0.0),    // 0
            Point3::new(1000.0, 0.0, 0.0), // 1
            Point3::new(1000.0, 1.0, 0.0), // 2
            Point3::new(0.0, 1.0, 0.0),    // 3
            // On the long bottom edge, 0.4·tol off-axis → within tolerance.
            Point3::new(250.0, 0.4 * tol::POINT_MERGE, 0.0), // 4
            // 2·tol off-axis → outside tolerance, must NOT be spliced.
            Point3::new(500.0, 2.0 * tol::POINT_MERGE, 0.0), // 5
            // Exactly on the long edge.
            Point3::new(750.0, 0.0, 0.0), // 6
            // Mid-length quad: a 100 m edge crosses several cells without
            // tripping the fallback, with a probe halfway along it.
            Point3::new(0.0, 30.0, 0.0),   // 7
            Point3::new(100.0, 30.0, 0.0), // 8
            Point3::new(100.0, 31.0, 0.0), // 9
            Point3::new(0.0, 31.0, 0.0),   // 10
            Point3::new(50.0, 30.0, 0.0),  // 11 — on edge (7,8)
        ];
        let mut faces = vec![vec![0usize, 1, 2, 3], vec![7usize, 8, 9, 10]];
        // Tiny triangles make each probe a used vertex (a candidate).
        for &probe in &[4usize, 5, 6, 11] {
            let p = positions[probe];
            let base = positions.len();
            positions.push(Point3::new(p.x + 0.5, p.y + 2.0, p.z));
            positions.push(Point3::new(p.x - 0.5, p.y + 2.0, p.z));
            faces.push(vec![probe, base, base + 1]);
        }
        // Many short edges drag the mean edge length (the cell size) far
        // below the 1000 m edge, so that edge spans many more cells than the
        // grid holds and takes the fallback.
        for i in 0..30 {
            let base = positions.len();
            let x = -10.0 - i as f64 * 3.0;
            positions.push(Point3::new(x, 0.0, 0.0));
            positions.push(Point3::new(x + 1.0, 0.0, 0.0));
            positions.push(Point3::new(x + 1.0, 1.0, 0.0));
            faces.push(vec![base, base + 1, base + 2]);
        }
        let mats: Vec<u32> = (0..faces.len() as u32).collect();
        let uvs: Vec<Vec<[f64; 2]>> = faces
            .iter()
            .map(|f| f.iter().map(|&v| [v as f64, 0.0]).collect())
            .collect();
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); faces.len()];

        let grid = split_t_junctions(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE);
        let reference =
            split_t_junctions_reference(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE);
        assert_eq!(grid, reference, "grid path must equal the full scan");
        assert_eq!(
            grid.0[0],
            vec![0, 4, 6, 1, 2, 3],
            "in-tolerance probes spliced in t-order; out-of-tolerance probe excluded"
        );
        assert_eq!(
            grid.0[1],
            vec![7, 11, 8, 9, 10],
            "mid-walk probe on the 100 m edge spliced in"
        );
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

        let (f1, m1, u1, h1) =
            split_t_junctions(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE);
        let (f2, m2, _, _) = split_t_junctions(&f1, &m1, &u1, &h1, &positions, tol::POINT_MERGE);
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
        let mut dir: BTreeMap<(usize, usize), usize> = BTreeMap::new();
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
        assert!(
            point_on_open_segment(Point3::new(0.5, 0.0, 0.0), a, b, tol::POINT_MERGE).is_some()
        );
        assert!(
            point_on_open_segment(a, a, b, tol::POINT_MERGE).is_none(),
            "endpoint a"
        );
        assert!(
            point_on_open_segment(b, a, b, tol::POINT_MERGE).is_none(),
            "endpoint b"
        );
        // Off the segment (perpendicular distance too large).
        assert!(
            point_on_open_segment(Point3::new(0.5, 0.5, 0.0), a, b, tol::POINT_MERGE).is_none()
        );
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

    /// Foreign input with face/hole vertex indices outside the position
    /// array must be dropped at the heal entry point, not panic the import
    /// (a crafted glTF index buffer reaches `heal_mesh` unvalidated).
    #[test]
    fn heal_mesh_drops_out_of_range_face_indices() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        // Outer loop references vertex 99 (>= positions.len()) → face dropped.
        let faces = vec![vec![0usize, 1, 99]];
        let mats = vec![0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new()];
        let (_, out_faces, _, _, _) = heal_mesh(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
        );
        assert!(out_faces.is_empty(), "out-of-range face dropped, no panic");

        // A hole loop with an out-of-range index is dropped; its face is kept.
        let faces = vec![vec![0usize, 1, 2]];
        let holes = vec![vec![vec![0usize, 1, 99]]];
        let (_, out_faces, _, _, out_holes) = heal_mesh(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
        );
        assert_eq!(out_faces.len(), 1, "face with a garbage hole is kept");
        assert!(out_holes[0].is_empty(), "the garbage hole is dropped");
    }

    /// A face's own vertex must never be spliced into its own loop: a
    /// weakly-simple polygon (notch apex E touching the interior of its own
    /// edge (A,B)) would otherwise come out with a repeated index —
    /// `[0, 4, 1, 2, 3, 4, 5, 6]` — which `from_polygons` rejects as
    /// `DegenerateFace`, turning valid input into a skipped face.
    #[test]
    fn t_junction_never_splices_a_faces_own_vertex() {
        // Rectangle with a triangular notch cut from the top; the notch apex
        // E = index 4 touches the bottom edge (A,B) at its interior.
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),   // 0 A
            Point3::new(10.0, 0.0, 0.0),  // 1 B
            Point3::new(10.0, 10.0, 0.0), // 2 C
            Point3::new(6.0, 10.0, 0.0),  // 3 D
            Point3::new(5.0, 0.0, 0.0),   // 4 E — on edge (A,B)
            Point3::new(4.0, 10.0, 0.0),  // 5 F
            Point3::new(0.0, 10.0, 0.0),  // 6 G
        ];
        let faces = vec![vec![0usize, 1, 2, 3, 4, 5, 6]];
        let mats = vec![0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new()];
        let (_, out_faces, _, _, _) = heal_mesh(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
        );
        for face in &out_faces {
            let mut seen = std::collections::BTreeSet::new();
            assert!(
                face.iter().all(|v| seen.insert(*v)),
                "face repeats a vertex index: {face:?}"
            );
        }
    }

    /// The T-junction on-segment test must scale with the caller's
    /// `weld_tol`: an f32-quantised (glTF) source leaves a genuine T-vertex
    /// microns off the exact edge line, far outside the native 1 nm gate but
    /// well inside the scale-appropriate weld tolerance.
    #[test]
    fn t_junction_tolerance_scales_with_weld_tol() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),   // 0
            Point3::new(20.0, 0.0, 0.0),  // 1
            Point3::new(20.0, 10.0, 0.0), // 2
            Point3::new(0.0, 10.0, 0.0),  // 3
            // 2 µm off edge (0,1) — f32 noise at this scale.
            Point3::new(10.0, 2e-6, 0.0), // 4
            Point3::new(10.0, 5.0, 1.0),  // 5
            Point3::new(11.0, 5.0, 1.0),  // 6
        ];
        // The distant triangle makes vertex 4 a used (candidate) vertex.
        let faces = vec![vec![0usize, 1, 2, 3], vec![4usize, 5, 6]];
        let mats = vec![0u32, 1u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(), Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];

        // At a 10 µm weld tolerance the T-vertex must be spliced in.
        let (_, out_faces, _, _, _, _) = heal_mesh_with_tol(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
            1e-5,
            tol::PLANE_DIST,
        );
        assert_eq!(
            out_faces[0].len(),
            5,
            "T-vertex 2 µm off the edge is spliced at weld_tol=1e-5, got {:?}",
            out_faces[0]
        );

        // At the native 1 nm tolerance it must NOT be (2 µm is a real gap).
        let (_, out_faces, _, _, _) = heal_mesh(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
        );
        assert_eq!(
            out_faces[0].len(),
            4,
            "native tolerance leaves the off-edge vertex alone"
        );
    }

    /// The zero-area sliver filter must scale with the caller's `weld_tol`:
    /// a collinear sliver whose height is f32 quantization noise (1 µm) is
    /// zero-area at glTF precision, but sits far above the native
    /// `POINT_MERGE²` floor.
    #[test]
    fn sliver_filter_scales_with_weld_tol() {
        let positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.5, 1e-6, 0.0), // 1 µm off the base line
        ];
        let faces = vec![vec![0usize, 1, 2]];
        let mats = vec![0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new()];

        let (_, coarse_faces, _, _, _, _) = heal_mesh_with_tol(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
            1e-5,
            tol::PLANE_DIST,
        );
        assert!(
            coarse_faces.is_empty(),
            "1 µm-high sliver is dropped at weld_tol=1e-5"
        );

        let (_, native_faces, _, _, _) = heal_mesh(
            &positions,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
        );
        assert_eq!(
            native_faces.len(),
            1,
            "at native precision the same triangle is real geometry"
        );
    }

    /// The sliver filter gates on effective HEIGHT (2·area / longest edge),
    /// not raw area: a raw-area floor of `weld_tol²` is dimensionally wrong
    /// (area² is m⁴, the floor m²) and becomes a shape-blind size cull at
    /// glTF tolerances — a well-formed 9 mm right triangle must survive a
    /// 0.1 mm weld tolerance, while a genuine weld-scale-height sliver with
    /// long edges must still be dropped.
    #[test]
    fn sliver_filter_gates_on_height_not_size() {
        let mats = vec![0u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new()];

        // Well-formed right triangle, 9 mm legs: keep at weld_tol = 1e-4.
        let small = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(9e-3, 0.0, 0.0),
            Point3::new(0.0, 9e-3, 0.0),
        ];
        let faces = vec![vec![0usize, 1, 2]];
        let (_, kept, _, _, _, _) = heal_mesh_with_tol(
            &small,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
            1e-4,
            tol::PLANE_DIST,
        );
        assert_eq!(
            kept.len(),
            1,
            "a well-formed 9 mm triangle is real geometry at weld_tol=1e-4"
        );

        // Genuine sliver: 1 m base, 50 µm height — below the 100 µm weld
        // tolerance, so it is quantization noise regardless of its length.
        let sliver = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.5, 5e-5, 0.0),
        ];
        let (_, dropped, _, _, _, _) = heal_mesh_with_tol(
            &sliver,
            &faces,
            &mats,
            &uvs,
            &holes,
            &Transform::IDENTITY,
            1e-4,
            tol::PLANE_DIST,
        );
        assert!(
            dropped.is_empty(),
            "a 50 µm-high sliver is noise at weld_tol=1e-4, whatever its length"
        );
    }

    /// A closed inward-wound shell nested INSIDE a closed outward shell is a
    /// hollow-solid cavity — the kernel's own convention (boolean subtraction
    /// emits cavity walls facing into the removed volume). It must NOT be
    /// "corrected" to outward; only free-standing inside-out shells flip.
    #[test]
    fn orient_outward_preserves_enclosed_cavity() {
        // Outer 2×2×2 cube (0..2) wound outward; inner 1×1×1 cube (0.5..1.5)
        // wound inward (a cavity).
        let mut positions: Vec<Point3> = cube_positions()
            .iter()
            .map(|p| Point3::new(p.x * 2.0, p.y * 2.0, p.z * 2.0))
            .collect();
        positions.extend(
            cube_positions()
                .iter()
                .map(|p| Point3::new(p.x + 0.5, p.y + 0.5, p.z + 0.5)),
        );
        let outward = |off: usize| -> Vec<Vec<usize>> {
            vec![
                vec![off, off + 3, off + 2, off + 1],
                vec![off + 4, off + 5, off + 6, off + 7],
                vec![off, off + 1, off + 5, off + 4],
                vec![off + 1, off + 2, off + 6, off + 5],
                vec![off + 2, off + 3, off + 7, off + 6],
                vec![off + 3, off, off + 4, off + 7],
            ]
        };
        let mut faces = outward(0);
        faces.extend(
            outward(8)
                .iter()
                .map(|f| f.iter().rev().copied().collect::<Vec<_>>()),
        );
        assert!(
            signed_volume6(&faces[6..], &positions) < 0.0,
            "cavity setup"
        );

        let mats = vec![0u32; 12];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 12];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 12];
        let (out, _, _, _) = orient_outward(&faces, &mats, &uvs, &holes, &positions);
        assert!(
            signed_volume6(&out[..6], &positions) > 0.0,
            "outer shell stays outward"
        );
        assert!(
            signed_volume6(&out[6..], &positions) < 0.0,
            "enclosed cavity must keep its inward winding"
        );
    }

    /// Two disjoint closed shells in one raw mesh must each get their own
    /// outward-orientation decision: a global signed-volume sum lets the
    /// larger correct shell mask the smaller inside-out one (or a dominant
    /// inverted shell wrongly flip a correct one).
    #[test]
    fn orient_outward_orients_each_disjoint_shell() {
        // Shell A: 2×2×2 cube at the origin (volume 8). Shell B: unit cube
        // at (5,0,0) (volume 1).
        let mut positions: Vec<Point3> = cube_positions()
            .iter()
            .map(|p| Point3::new(p.x * 2.0, p.y * 2.0, p.z * 2.0))
            .collect();
        positions.extend(
            cube_positions()
                .iter()
                .map(|p| Point3::new(p.x + 5.0, p.y, p.z)),
        );
        let outward = |off: usize| -> Vec<Vec<usize>> {
            vec![
                vec![off, off + 3, off + 2, off + 1],
                vec![off + 4, off + 5, off + 6, off + 7],
                vec![off, off + 1, off + 5, off + 4],
                vec![off + 1, off + 2, off + 6, off + 5],
                vec![off + 2, off + 3, off + 7, off + 6],
                vec![off + 3, off, off + 4, off + 7],
            ]
        };
        let invert = |faces: &[Vec<usize>]| -> Vec<Vec<usize>> {
            faces
                .iter()
                .map(|f| f.iter().rev().copied().collect())
                .collect()
        };
        let mats = vec![0u32; 12];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 12];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 12];

        // Case 1: big shell correct, small shell inverted (combined volume
        // positive — the global gate would skip the needed flip).
        let mut faces = outward(0);
        faces.extend(invert(&outward(8)));
        let (out, _, _, _) = orient_outward(&faces, &mats, &uvs, &holes, &positions);
        assert!(
            signed_volume6(&out[..6], &positions) > 0.0,
            "correct big shell stays outward"
        );
        assert!(
            signed_volume6(&out[6..], &positions) > 0.0,
            "inverted small shell is flipped outward"
        );

        // Case 2: big shell inverted, small shell correct (combined volume
        // negative — the global gate would flip BOTH, corrupting B).
        let mut faces = invert(&outward(0));
        faces.extend(outward(8));
        let (out, _, _, _) = orient_outward(&faces, &mats, &uvs, &holes, &positions);
        assert!(
            signed_volume6(&out[..6], &positions) > 0.0,
            "inverted big shell is flipped outward"
        );
        assert!(
            signed_volume6(&out[6..], &positions) > 0.0,
            "correct small shell is left alone"
        );
    }

    /// Signed volume must be computed relative to a local reference point:
    /// summing tetrahedra from the world origin loses the whole signal to
    /// cancellation for a shell far from the origin (offset³·ε ≫ extent³),
    /// flipping the orientation decision.
    #[test]
    fn orient_outward_is_exact_far_from_origin() {
        // Unit cube at (1e8, 1e8, 1e8): with origin-based tetrahedra the
        // fan-sum for the INVERTED cube comes out ≈ +2e8 (true value −6),
        // so the flip is skipped.
        let positions: Vec<Point3> = cube_positions()
            .iter()
            .map(|p| Point3::new(p.x + 1e8, p.y + 1e8, p.z + 1e8))
            .collect();
        let outward = vec![
            vec![0usize, 3, 2, 1],
            vec![4usize, 5, 6, 7],
            vec![0usize, 1, 5, 4],
            vec![1usize, 2, 6, 5],
            vec![2usize, 3, 7, 6],
            vec![3usize, 0, 4, 7],
        ];
        let inverted: Vec<Vec<usize>> = outward
            .iter()
            .map(|f| f.iter().rev().copied().collect())
            .collect();
        let mats = vec![0u32; 6];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 6];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 6];

        let (flipped, _, _, _) = orient_outward(&inverted, &mats, &uvs, &holes, &positions);
        assert_eq!(
            flipped, outward,
            "far-from-origin inside-out cube must still be flipped outward"
        );

        let (kept, _, _, _) = orient_outward(&outward, &mats, &uvs, &holes, &positions);
        assert_eq!(kept, outward, "far-from-origin outward cube left as-is");
    }

    /// THE identity (property): the grid-accelerated [`split_t_junctions`]
    /// emits byte-identical output to the full-scan reference on arbitrary
    /// meshes (DEVELOPMENT.md rule 3). The lattice positions force exact
    /// collinear/T-junction configurations; per-coordinate jitter of
    /// ±0.4·POINT_MERGE / ±2·POINT_MERGE probes both sides of the on-segment
    /// tolerance (and both sides of grid cell boundaries); the scale factor
    /// varies the cell size across six orders of magnitude; rings of length
    /// 1–6 with repeats include degenerate and zero-length edges.
    #[test]
    fn property_grid_split_equals_reference() {
        use proptest::prelude::*;
        // 24 lattice points on a 4×3×2 grid — dense enough that random faces
        // share collinear runs and mid-edge vertices.
        let ring = proptest::collection::vec(0usize..24, 1..7);
        let mesh = proptest::collection::vec(ring, 0..14);
        let jitter = proptest::collection::vec(-2i8..=2, 72);
        let scale = prop_oneof![Just(0.001), Just(1.0), Just(1000.0)];
        proptest!(
            ProptestConfig::with_cases(512),
            |(faces in mesh, jit in jitter, s in scale)| {
                let off = |j: i8| {
                    f64::from(j) * if j.abs() == 1 { 0.4 } else { 1.0 } * tol::POINT_MERGE
                };
                let positions: Vec<Point3> = (0..24usize)
                    .map(|i| {
                        Point3::new(
                            (i % 4) as f64 * s + off(jit[3 * i]),
                            ((i / 4) % 3) as f64 * s + off(jit[3 * i + 1]),
                            (i / 12) as f64 * s + off(jit[3 * i + 2]),
                        )
                    })
                    .collect();
                let mats: Vec<u32> = (0..faces.len() as u32).collect();
                // Corner UVs on every face so spliced-in UV lerping is compared too.
                let uvs: Vec<Vec<[f64; 2]>> = faces
                    .iter()
                    .map(|f| {
                        f.iter()
                            .enumerate()
                            .map(|(k, &v)| [v as f64, k as f64])
                            .collect()
                    })
                    .collect();
                let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); faces.len()];
                prop_assert_eq!(
                    split_t_junctions(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE),
                    split_t_junctions_reference(&faces, &mats, &uvs, &holes, &positions, tol::POINT_MERGE)
                );
            }
        );
    }

    /// `drop_kernel_degenerate_faces` removes exactly the faces the kernel's
    /// `Plane::from_polygon` would reject (Newell normal below
    /// `tol::NORMALIZE_MIN_LENGTH`), keeping well-formed faces — so no face the
    /// kernel calls `DegenerateFace` ever reaches `from_polygons`.
    #[test]
    fn drop_kernel_degenerate_faces_matches_the_kernel_gate() {
        let positions = vec![
            // A well-formed triangle (2·area = 1, far above the floor).
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            // A near-degenerate sliver: base 1e-5, height 1e-8 → 2·area = 1e-13,
            // below NORMALIZE_MIN_LENGTH (1e-12), so Plane::from_polygon fails.
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1e-5, 0.0, 1.0),
            Point3::new(5e-6, 1e-8, 1.0),
        ];
        let faces = vec![vec![0usize, 1, 2], vec![3usize, 4, 5]];
        let mats = vec![7u32, 9u32];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(), Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];

        // The sliver really is kernel-degenerate; the good triangle is not.
        assert!(Plane::from_polygon(&[positions[0], positions[1], positions[2]]).is_ok());
        assert!(Plane::from_polygon(&[positions[3], positions[4], positions[5]]).is_err());

        let (out_faces, out_mats, _uvs, _holes) =
            drop_kernel_degenerate_faces(&faces, &mats, &uvs, &holes, &positions);
        assert_eq!(
            out_faces,
            vec![vec![0usize, 1, 2]],
            "only the sliver is dropped"
        );
        assert_eq!(
            out_mats,
            vec![7u32],
            "per-face payloads follow the surviving face"
        );
    }
}
