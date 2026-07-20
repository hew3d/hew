//! Raw triangle soup → `kernel::ImportScene`: weld, split into shells, heal,
//! cut residual non-manifold seams.
//!
//! # Pipeline (DESIGN §3)
//!
//! ```text
//! RawTriangles ──scale by unit_scale──> scaled positions
//!   ──mesh_heal::weld_with_tol (coarse f32 weld tol)──> welded positions,
//!       remapped faces (collapses STL's massive vertex duplication; this is
//!       what discovers connectivity)
//!   ──split_into_shells (below)──> one group per geometrically-disjoint shell
//!       — a "plate of parts" becomes N groups
//!   ──per shell──> mesh_heal::heal_mesh_with_tol (dedup, T-junction stitch,
//!       orient outward, coplanar-merge into editable n-gons) ──THEN──>
//!       mesh_heal::split::split_non_manifold (cut any residual non-manifold
//!       seam WITHIN the shell; NO re-weld after — see below)
//!   ──reconstruct_solids (below)──> group each solid with the cavities nested
//!       directly inside it (flipped inward) into ONE recipe — a hollow part
//!       becomes one watertight Object with a void
//!   ──> one kernel::MeshRecipe per assembly / open piece ──> ImportScene
//! ```
//!
//! ## Hollow / nested shells become one Object with a cavity
//!
//! A hollow single-part STL — vase-mode or explicit-wall-thickness models — is
//! an outer shell plus a fully-enclosed, vertex-disjoint inner shell, which
//! `split_into_shells` separates into two shells sharing no geometry. The
//! kernel represents a hollow as ONE watertight Object whose cavity walls wind
//! **inward** (a void in the material), so [`reconstruct_solids`] classifies
//! the clean (closed) shells by containment-nesting depth and regroups them:
//! an even-depth shell (depth 0 solid, depth 2 island inside a cavity, …) roots
//! its own Object; an odd-depth shell (a cavity) is flipped to inward winding
//! and merged into its containing solid's recipe. So a hollow part → one
//! Object with a void; a solid island floating in a cavity → its own separate
//! Object; side-by-side parts → separate Objects (unchanged).
//!
//! This is the fallback the kernel-reuse investigation pointed to rather than
//! `Object::split_solids`: that op classifies components by their *existing*
//! signed-volume sign and only regroups — it never flips winding (verified
//! empirically: two outward-wound nested shells come back as two disjoint
//! solids). STL's heal pass orients every shell outward, so the cavity shells
//! must be detected and flipped inward here regardless; once that containment
//! classification is done, grouping at the recipe level produces the correct
//! watertight hollow through the ordinary `ingest` path — routing the same
//! classification through `split_solids` would additionally require building a
//! combined `Object` and converting the split results back into recipes, with
//! no reuse benefit (the containment classification, the hard part, lives here
//! either way) and no `wasm-api`/`ingest` surface change on this side.
//!
//! ## Heal each shell BEFORE its split — never a re-weld after
//!
//! The per-shell heal and `split_non_manifold` run in this order for a
//! load-bearing reason (the gltf-import order: heal, THEN split).
//! `split_non_manifold`'s pinch/bowtie fallback severs a duplicated directed
//! edge by re-indexing one endpoint onto a fresh **exactly coincident**
//! duplicate vertex — the crack is real, the piece is honestly leaky. But
//! `heal_mesh_with_tol`'s step 2 is itself a weld, and a weld at *any*
//! tolerance ≥ 0 merges two coincident points (distance 0) straight back
//! together — recreating the non-manifold edge, collapsing the face to a
//! degenerate loop, and getting the whole piece silently skipped by
//! `Document::ingest` (a rule-4 geometry-loss violation the reviewer
//! reproduced). So heal runs BEFORE the split; the split's output is handed to
//! the kernel verbatim, never re-welded. (The coarse weld up front and the
//! per-shell heal's own weld are both idempotent on already-welded coords, so
//! neither undoes the other; only a weld placed *after* the split is the bug.)
//!
//! The coarse weld runs up front — before `split_into_shells` — rather than
//! per shell so that connectivity is decided on shared *vertices* only: two
//! parts that merely touch (a face resting on another, which the heal's
//! T-junction stitching would otherwise fuse) stay separate Objects, the
//! maker-expected result for a plate of parts. Heal (with its T-junction
//! stitching) runs only per already-separated shell.
//!
//! ## Why shell-splitting is NOT `mesh_heal::split::split_non_manifold` alone
//!
//! `split_non_manifold` fast-path returns `None` whenever the input already
//! satisfies the kernel's directed-edge precondition
//! (`!has_duplicate_directed_edge`) — and two geometrically disjoint, each
//! individually-manifold shells (the common "plate of parts" STL) satisfy
//! that precondition perfectly: no edge is shared, so none is duplicated.
//! Left to `split_non_manifold` alone, a two-part plate would heal into a
//! SINGLE `Object` holding two disconnected shells (the kernel's
//! `Object::from_polygons` has no connectivity requirement — one `Shell`
//! happily holds every face regardless of how many disjoint pieces they
//! form), not the two separate Objects a maker expects. [`split_into_shells`]
//! does the genuine geometric-connectivity split (union-find over shared
//! welded vertices) that produces one group per shell; `split_non_manifold`
//! is then applied *within* each shell to additionally decompose any
//! non-manifold seam a single physical part may still contain (a fin, an
//! internal partition) — exactly its documented job.

use kernel::{ImportNode, ImportScene, MeshRecipe, NO_MATERIAL, Point3};
use mesh_heal::split::{MeshPiece, split_non_manifold};

use crate::parse::RawTriangles;

/// Fallback base name when the caller passes no usable `name_hint` (STL has no
/// internal name of its own) — DESIGN §3's documented "no filename" case.
const FALLBACK_NAME: &str = "Imported";

/// Cap on individual split-piece diagnostic detail before the remainder is
/// folded into one aggregate line — mirrors `gltf_import::convert::SPLIT_DETAIL_CAP`'s
/// reasoning: a badly damaged plate of parts can hold dozens of pieces, and a
/// wall of near-identical per-piece strings helps nobody.
const DETAIL_CAP: usize = 8;

/// Multiplicative factor applied to a mesh's coordinate magnitude to derive
/// its weld tolerance: STL binary vertices are `f32`, so vertices coincident
/// in the authoring tool land up to `magnitude · 2.4e-7` apart after loading
/// — mirrors `gltf_import::convert::gltf_weld_tol`'s reasoning and constant
/// (glTF `POSITION` is the same `f32` precision).
const STL_WELD_TOL_FACTOR: f64 = 1e-6;
/// Floor on the weld tolerance for small/degenerate meshes (below which the
/// native `kernel::tol::POINT_MERGE` scale is meaningless for f32 input).
const STL_WELD_TOL_FLOOR: f64 = 1e-7;

/// Generalized-winding-number magnitude above which a probe point counts as
/// enclosed by a closed shell: ±1 strictly inside, 0 strictly outside, so the
/// half-integer midpoint discriminates robustly (a topological threshold, not
/// a length tolerance). Mirrors `mesh_heal`'s own private `WINDING_ENCLOSED`.
const WINDING_ENCLOSED: f64 = 0.5;

/// Upper bound on how many of a candidate cavity's vertices the containment
/// test samples against its host (evenly strided across the vertex array).
/// The full O(vertices × host-faces) winding sum would stall a dense scanned
/// hollow (tens of thousands of vertices per wall); a bounded, spread sample
/// still reliably catches a straddling shell (a large fraction of its
/// vertices fall outside), which is all the merge gate needs.
const CONTAINMENT_PROBE_CAP: usize = 256;

/// Build the `ImportScene` (plus user-visible warnings) from raw parsed
/// triangles. `unit_scale` is meters-per-STL-unit (already resolved by the
/// caller — the crate itself is unit-blind).
///
/// Leaky (open-shell) pieces are NOT counted here: `Document::ingest`'s
/// `ImportReport` is the single authoritative source of the watertight/leaky
/// counts the UI shows, so the crate never emits a second, possibly-divergent
/// leaky number of its own (a skipped piece would count leaky pre-ingest but
/// show up under `report.skipped`, not `report.leaky`). The only warning this
/// crate emits is the non-manifold-decomposition one, which the `ImportReport`
/// cannot express. (Hollow/nested shells are reconstructed into one Object
/// with a cavity — correct behavior, not a warning.)
pub fn build_scene(
    raw: RawTriangles,
    unit_scale: f64,
    name_hint: Option<&str>,
) -> (ImportScene, Vec<String>) {
    let mut warnings = Vec::new();

    // 1. Scale to meters. STL carries no units; the caller (UI) decided.
    let scaled: Vec<Point3> = raw
        .positions
        .iter()
        .map(|p| Point3::new(p.x * unit_scale, p.y * unit_scale, p.z * unit_scale))
        .collect();

    // 2. Coarse weld at the f32 tolerance + remap faces. This collapses STL's
    // massive vertex duplication into a shared set and is what discovers
    // connectivity for step 3. Deliberately just the weld (not a full heal):
    // heal's T-junction stitching would fuse parts that merely touch, so it
    // runs only per already-separated shell in step 4 (see module docs).
    let weld_tol = stl_weld_tol(&scaled);
    let n = raw.faces.len();
    let no_mats = vec![NO_MATERIAL; n];
    let no_uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); n];
    let no_holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); n];
    let (welded, old_to_new) = mesh_heal::weld_with_tol(&scaled, weld_tol);
    let (faces, face_materials, face_uvs, face_holes) =
        mesh_heal::remap_faces(&raw.faces, &no_mats, &no_uvs, &no_holes, &old_to_new);
    if faces.is_empty() {
        return (empty_scene(), warnings);
    }

    // 3. Split into geometrically-disjoint shells (see module docs for why
    // this is not `split_non_manifold`).
    let shells = split_into_shells(&welded, &faces, &face_materials, &face_uvs, &face_holes);

    // 4. Heal each shell (dedup, T-junction stitch, orient outward,
    // coplanar-merge). Heal runs BEFORE the non-manifold split so the split's
    // coincident-vertex pinch fix is never re-welded away (see module docs).
    // Kernel-degenerate slivers heal drops (faces the kernel could not build a
    // plane through — one such face used to skip a whole real-world model at
    // ingest) are counted for the warning below: dropping geometry, even
    // unbuildable geometry, is never silent. The count is accumulated BEFORE
    // empty shells are filtered out, so a shell made entirely of such slivers
    // still reports its drops instead of vanishing without a trace.
    let mut degenerate_dropped_total = 0usize;
    let healed: Vec<Shell> = shells
        .into_iter()
        .map(heal_shell)
        .map(|(shell, dropped)| {
            degenerate_dropped_total += dropped;
            shell
        })
        .filter(|shell| !shell.faces.is_empty())
        .collect();

    // 5. Per healed shell: cut any residual non-manifold seam (a fin, an
    // internal partition, a pinch). NO re-weld after the cut — see module docs.
    // A CLEAN shell (split returns None) is one piece and is a candidate for
    // hollow/cavity reconstruction (§6). A non-manifold shell splits into open
    // pieces that are each their own leaky Object — an open piece is never a
    // valid cavity or host, so it is emitted directly.
    let mut clean_shells: Vec<Shell> = Vec::new();
    let mut open_pieces: Vec<MeshPiece> = Vec::new();
    let mut split_piece_total = 0usize;
    let mut split_shell_count = 0usize;

    for shell in healed {
        match split_non_manifold(
            &shell.positions,
            &shell.faces,
            &shell.face_materials,
            &shell.face_corner_uvs,
            &shell.face_holes,
        ) {
            Some(pieces) => {
                split_shell_count += 1;
                split_piece_total += pieces.len();
                for piece in pieces {
                    if !piece.faces.is_empty() {
                        open_pieces.push(piece);
                    }
                }
            }
            None => {
                if !shell.faces.is_empty() {
                    clean_shells.push(shell);
                }
            }
        }
    }

    // 6. Reconstruct hollows: group each solid with the cavities nested
    // directly inside it (flipped inward) into ONE recipe (see module docs).
    let assemblies = reconstruct_solids(&clean_shells);

    let mut recipes: Vec<MeshRecipe> = Vec::with_capacity(assemblies.len() + open_pieces.len());
    for a in assemblies {
        recipes.push(recipe_from_arrays(a.positions, a.faces, a.face_holes));
    }
    for piece in open_pieces {
        recipes.push(recipe_from_arrays(
            piece.positions,
            piece.faces,
            piece.face_holes,
        ));
    }

    if split_shell_count > 0 {
        warnings.push(format!(
            "{split_shell_count} part{} had non-manifold geometry; imported as {split_piece_total} open shell{} (split at non-manifold edges, geometry unchanged)",
            if split_shell_count == 1 { "" } else { "s" },
            if split_piece_total == 1 { "" } else { "s" },
        ));
    }
    if degenerate_dropped_total > 0 {
        warnings.push(format!(
            "{degenerate_dropped_total} degenerate sliver face{} (too thin to build) {} removed; an affected part may arrive leaky",
            if degenerate_dropped_total == 1 { "" } else { "s" },
            if degenerate_dropped_total == 1 { "was" } else { "were" },
        ));
    }
    cap_warnings(&mut warnings);

    // Name from the caller's hint (the file stem), falling back to "Imported"
    // when there is none or it is blank. One Object takes the bare name;
    // multiples get " (2)", " (3)", … suffixes.
    let base_name = name_hint
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(FALLBACK_NAME);
    for (i, recipe) in recipes.iter_mut().enumerate() {
        recipe.name = if i == 0 {
            base_name.to_string()
        } else {
            format!("{base_name} ({})", i + 1)
        };
    }

    let roots: Vec<ImportNode> = recipes.into_iter().map(ImportNode::Mesh).collect();
    (
        ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots,
            guides: Vec::new(),
            // STL has no document-level tag/layer concept.
            tags: Vec::new(),
        },
        warnings,
    )
}

/// An empty import scene (nothing survived heal). `import()` maps a
/// zero-root scene to `StlError::Empty`.
fn empty_scene() -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: Vec::new(),
        tags: Vec::new(),
    }
}

fn cap_warnings(warnings: &mut Vec<String>) {
    if warnings.len() > DETAIL_CAP {
        let extra = warnings.len() - DETAIL_CAP;
        warnings.truncate(DETAIL_CAP);
        warnings.push(format!("{extra} more diagnostic message(s) omitted"));
    }
}

/// Choose a weld tolerance for f32-sourced STL positions — see
/// `STL_WELD_TOL_FACTOR`.
fn stl_weld_tol(positions: &[Point3]) -> f64 {
    let max_abs = positions
        .iter()
        .flat_map(|p| [p.x.abs(), p.y.abs(), p.z.abs()])
        .fold(0.0_f64, f64::max);
    (max_abs * STL_WELD_TOL_FACTOR).max(STL_WELD_TOL_FLOOR)
}

/// Run the full heal pass on one already-separated shell (dedup two-sided,
/// T-junction stitch, orient outward, coplanar-merge into editable n-gons),
/// returning the healed shell plus the number of kernel-degenerate sliver
/// faces heal removed (surfaced in the import warnings — a geometry-altering
/// drop is never silent). The shell is returned EVEN when nothing survived
/// heal (empty faces): the caller filters empty shells but must still see the
/// count, or a shell made entirely of degenerate slivers would vanish with no
/// warning at all. The weld tolerance is recomputed from the shell's own
/// coordinate magnitude, matching the coarse weld's scale; the shell is
/// already welded, so this weld is idempotent and only the subsequent
/// (post-heal) non-manifold split may introduce coincident duplicates, which
/// are then never re-welded.
fn heal_shell(shell: Shell) -> (Shell, usize) {
    let weld_tol = stl_weld_tol(&shell.positions);
    let (positions, faces, face_materials, face_corner_uvs, face_holes, dropped_degenerate) =
        mesh_heal::heal_mesh_with_tol(
            &shell.positions,
            &shell.faces,
            &shell.face_materials,
            &shell.face_corner_uvs,
            &shell.face_holes,
            &kernel::Transform::IDENTITY,
            weld_tol,
            kernel::tol::IMPORT_PLANE_DIST,
        );
    (
        Shell {
            positions,
            faces,
            face_materials,
            face_corner_uvs,
            face_holes,
        },
        dropped_degenerate,
    )
}

/// One reconstructed solid assembly: a solid's outward shell plus the shells
/// of every cavity nested directly inside it (each flipped to inward winding),
/// merged into a single position/face/hole set so `Document::ingest` produces
/// ONE watertight Object with those voids.
struct Assembly {
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_holes: Vec<Vec<Vec<usize>>>,
}

/// True only when `inner` is UNAMBIGUOUSLY inside the closed shell `outer`:
/// every sampled vertex of `inner` tests strictly inside `outer` (generalized
/// winding magnitude > [`WINDING_ENCLOSED`]). A shell that straddles `outer`'s
/// boundary — some vertices in, some out, i.e. the two meshes intersect — is
/// rejected (the honest, conservative result: do NOT flip-and-merge two
/// intersecting shells into a nonsense self-intersecting "watertight" solid).
/// Sampling is bounded and evenly strided ([`CONTAINMENT_PROBE_CAP`]) so a
/// dense hollow does not stall; a straddling shell still fails because a large
/// fraction of its vertices fall outside. The caller has already verified
/// `outer` is closed (winding is only meaningful there).
fn shell_strictly_inside(inner: &Shell, outer: &Shell) -> bool {
    let n = inner.positions.len();
    if n == 0 {
        return false;
    }
    let stride = n.div_ceil(CONTAINMENT_PROBE_CAP).max(1);
    let mut probed = 0usize;
    let mut vi = 0usize;
    while vi < n {
        let w = winding_number(inner.positions[vi], &outer.faces, &outer.positions).abs();
        if w <= WINDING_ENCLOSED {
            // A vertex not strictly inside `outer` → outside or straddling.
            return false;
        }
        probed += 1;
        vi += stride;
    }
    probed > 0
}

/// Classify the clean (already non-manifold-split) shells by containment
/// nesting and regroup them so a hollow single-part STL becomes ONE watertight
/// Object with a cavity (see module docs). Each EVEN-depth shell (depth-0
/// solid, depth-2 island inside a cavity, …) roots its own [`Assembly`] with
/// its outward faces; each ODD-depth shell (a cavity) is flipped to inward
/// winding and merged into its containing solid's assembly.
///
/// Containment gates the geometry mutation, so it is deliberately strict — a
/// false "contained" would flip a shell inward and fuse it into another,
/// silently corrupting a good solid. A shell `i` is contained in `j` only when
/// BOTH are closed (winding is meaningful, and an open shell is never a valid
/// cavity) AND *every* sampled vertex of `i` tests strictly inside `j`
/// ([`shell_strictly_inside`]) — a single-probe test would merge a shell that
/// merely STRADDLES a curved container's boundary (the shells actually
/// intersect) into a nonsense self-intersecting "watertight" mesh. Ambiguous
/// (partly-in, partly-out) or open shells are left as their own standalone
/// Objects — the conservative, honest result. A cheap per-axis bounding-box
/// pre-filter gates the winding work so disjoint parts never pay for it, and a
/// shell's parent is the SMALLEST-volume such container (its immediate host).
fn reconstruct_solids(shells: &[Shell]) -> Vec<Assembly> {
    let closed: Vec<bool> = shells.iter().map(|s| is_closed_shell(&s.faces)).collect();
    let bboxes: Vec<(Point3, Point3)> = shells.iter().map(shell_bbox).collect();
    // |6× signed volume| — the tiebreak that picks the SMALLEST containing
    // shell (the immediate parent) among all that contain a given shell.
    let abs_vol6: Vec<f64> = shells
        .iter()
        .map(|s| shell_signed_volume6(s).abs())
        .collect();

    // parent[i] = the smallest closed shell strictly containing shell i, else None.
    let parent: Vec<Option<usize>> = (0..shells.len())
        .map(|i| {
            // An OPEN shell is never a cavity — it can't be a watertight void,
            // and flipping+merging a leaky shell would corrupt its host and
            // hide the actually-broken part. Force it to depth 0 (its own
            // standalone, correctly-leaky Object).
            if !closed[i] {
                return None;
            }
            (0..shells.len())
                .filter(|&j| {
                    j != i
                        && closed[j]
                        && bbox_contains(bboxes[j], bboxes[i])
                        && shell_strictly_inside(&shells[i], &shells[j])
                })
                .min_by(|&a, &b| abs_vol6[a].total_cmp(&abs_vol6[b]))
        })
        .collect();

    // depth[i] = ancestor count in the nesting forest. Even = solid/island,
    // odd = cavity. The forest is acyclic (a strict container is strictly
    // larger), but a guard bounds the walk defensively.
    let depth: Vec<usize> = (0..shells.len())
        .map(|i| {
            let mut d = 0usize;
            let mut cur = parent[i];
            while let Some(p) = cur {
                d += 1;
                if d > shells.len() {
                    break;
                }
                cur = parent[p];
            }
            d
        })
        .collect();

    // Pass 1: each even-depth shell roots an assembly with its outward faces.
    let mut asm_of_shell: Vec<Option<usize>> = vec![None; shells.len()];
    let mut assemblies: Vec<Assembly> = Vec::new();
    for i in 0..shells.len() {
        if depth[i].is_multiple_of(2) {
            asm_of_shell[i] = Some(assemblies.len());
            assemblies.push(assembly_from_shell(&shells[i]));
        }
    }
    // Pass 2: each odd-depth (cavity) shell is flipped inward and merged into
    // its containing solid's assembly (its parent is even-depth, so already
    // rooted an assembly in pass 1).
    for i in 0..shells.len() {
        if !depth[i].is_multiple_of(2) {
            match parent[i].and_then(|p| asm_of_shell[p]) {
                Some(ai) => merge_cavity_into(&mut assemblies[ai], &shells[i]),
                // Unreachable (an odd-depth shell always has an even-depth
                // parent), but never drop geometry (rule 4): emit it as its
                // own solid instead.
                None => assemblies.push(assembly_from_shell(&shells[i])),
            }
        }
    }
    assemblies
}

/// A solid shell's outward faces as a fresh [`Assembly`] root.
fn assembly_from_shell(shell: &Shell) -> Assembly {
    Assembly {
        positions: shell.positions.clone(),
        faces: shell.faces.clone(),
        face_holes: shell.face_holes.clone(),
    }
}

/// Append a cavity shell to a solid assembly, flipping every loop (outer +
/// holes) to inward winding so the kernel reads it as a void in the material.
/// Vertex indices are offset past the assembly's existing positions.
fn merge_cavity_into(asm: &mut Assembly, cavity: &Shell) {
    let base = asm.positions.len();
    asm.positions.extend(cavity.positions.iter().copied());
    for (fi, face) in cavity.faces.iter().enumerate() {
        asm.faces
            .push(face.iter().rev().map(|&v| v + base).collect());
        let holes = cavity.face_holes.get(fi).cloned().unwrap_or_default();
        asm.face_holes.push(
            holes
                .iter()
                .map(|h| h.iter().rev().map(|&v| v + base).collect())
                .collect(),
        );
    }
}

/// Six times the signed volume of a shell (divergence-theorem sum over its
/// faces, fan-triangulated). Sign follows winding: positive for an
/// outward-wound closed shell. Hole loops (from heal's coplanar merge) wind
/// opposite their face and subtract correctly, so they are included — omitting
/// them over-estimates the volume and can mis-order two near-equal shells,
/// picking the wrong immediate parent (and hence the wrong depth parity).
/// Only the magnitude is used (to order shells by size when picking a cavity's
/// immediate host).
fn shell_signed_volume6(shell: &Shell) -> f64 {
    let empty: Vec<Vec<usize>> = Vec::new();
    let mut v6 = 0.0;
    for (fi, face) in shell.faces.iter().enumerate() {
        let holes = shell.face_holes.get(fi).unwrap_or(&empty);
        for loop_ in std::iter::once(face).chain(holes.iter()) {
            for i in 1..loop_.len().saturating_sub(1) {
                let a = shell.positions[loop_[0]].to_vec();
                let b = shell.positions[loop_[i]].to_vec();
                let c = shell.positions[loop_[i + 1]].to_vec();
                v6 += a.dot(b.cross(c));
            }
        }
    }
    v6
}

/// Build a `MeshRecipe` from bare position/face/hole arrays. STL carries no
/// materials or UVs, so every face gets `NO_MATERIAL` and no UV frame. The
/// name is assigned by the caller once the final Object order is known.
fn recipe_from_arrays(
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_holes: Vec<Vec<Vec<usize>>>,
) -> MeshRecipe {
    let n = faces.len();
    MeshRecipe {
        name: String::new(),
        positions,
        faces,
        face_materials: vec![NO_MATERIAL; n],
        // An empty frame list means every face falls back to the tessellator's
        // world_size UV.
        face_uv_frames: Vec::new(),
        face_holes,
        base_material: NO_MATERIAL,
        tags: Vec::new(),
    }
}

/// World-space axis-aligned bounding box of a shell's vertices.
fn shell_bbox(shell: &Shell) -> (Point3, Point3) {
    let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &shell.positions {
        min = Point3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
        max = Point3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
    }
    (min, max)
}

/// True if `outer` contains `inner` on every axis (inclusive).
fn bbox_contains(outer: (Point3, Point3), inner: (Point3, Point3)) -> bool {
    let (omin, omax) = outer;
    let (imin, imax) = inner;
    omin.x <= imin.x
        && omin.y <= imin.y
        && omin.z <= imin.z
        && imax.x <= omax.x
        && imax.y <= omax.y
        && imax.z <= omax.z
}

/// Generalized winding number of point `p` with respect to an oriented face
/// set (Van Oosterom–Strackee solid angles / 4π): ±1 strictly inside a closed
/// shell, 0 strictly outside, with no epsilon tie-breaking on rays through
/// edges/vertices. Same formula `mesh_heal::winding_number` uses (private
/// there). `faces` index into `positions`.
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

/// True if every directed edge of `faces` has its reverse present too — the
/// same "fully paired half-edges" test `kernel::topo::Object::watertight`
/// applies. Used by [`reconstruct_solids`] to gate cavity classification on
/// closed shells only: an open shell is never a valid cavity, and only a
/// closed shell can host one (winding is meaningful only for a closed
/// surface). `Document::ingest` remains the authoritative source of the
/// `ImportReport::leaky` count the UI displays.
fn is_closed_shell(faces: &[Vec<usize>]) -> bool {
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

/// One geometrically-connected group of faces (positions compacted/local).
struct Shell {
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_materials: Vec<u32>,
    face_corner_uvs: Vec<Vec<[f64; 2]>>,
    face_holes: Vec<Vec<Vec<usize>>>,
}

/// Partition welded faces into connected components by shared vertex id
/// (union-find), returning one compacted [`Shell`] per component in
/// first-face order (deterministic). Two faces are in the same shell iff
/// there is a chain of faces between them each sharing at least one welded
/// vertex with the next — i.e. genuine geometric touching, not merely
/// "close" — which is exactly what turns a downloaded multi-part plate into
/// one Object per part.
fn split_into_shells(
    positions: &[Point3],
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
) -> Vec<Shell> {
    let mut uf = UnionFind::new(positions.len());
    for face in faces {
        if let [first, rest @ ..] = face.as_slice() {
            for &v in rest {
                uf.union(*first, v);
            }
        }
    }

    let mut comp_of_root: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    let mut comp_face_ids: Vec<Vec<usize>> = Vec::new();
    for (fi, face) in faces.iter().enumerate() {
        let Some(&v0) = face.first() else { continue };
        let root = uf.find(v0);
        let c = *comp_of_root.entry(root).or_insert_with(|| {
            comp_face_ids.push(Vec::new());
            comp_face_ids.len() - 1
        });
        comp_face_ids[c].push(fi);
    }

    comp_face_ids
        .into_iter()
        .map(|face_ids| {
            let sub_faces: Vec<Vec<usize>> = face_ids.iter().map(|&fi| faces[fi].clone()).collect();
            let sub_holes: Vec<Vec<Vec<usize>>> =
                face_ids.iter().map(|&fi| face_holes[fi].clone()).collect();
            let (local_positions, local_faces, local_holes) =
                mesh_heal::compact_unused(positions, &sub_faces, &sub_holes);
            let sub_materials: Vec<u32> = face_ids.iter().map(|&fi| face_materials[fi]).collect();
            let sub_uvs: Vec<Vec<[f64; 2]>> = face_ids
                .iter()
                .map(|&fi| face_corner_uvs[fi].clone())
                .collect();
            Shell {
                positions: local_positions,
                faces: local_faces,
                face_materials: sub_materials,
                face_corner_uvs: sub_uvs,
                face_holes: local_holes,
            }
        })
        .collect()
}

/// Minimal union-find (path compression + union by index) over vertex
/// indices, local to shell-splitting. Not shared with `mesh-heal`'s
/// internal (private) union-find used by `orient_outward` — that one is
/// scoped to face indices within a single already-connected shell, a
/// different problem.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let root = self.find(self.parent[x]);
            self.parent[x] = root;
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

#[cfg(test)]
mod specs {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    /// Two triangles sharing an edge (a quad split diagonally) are one shell.
    #[test]
    fn touching_triangles_are_one_shell() {
        let positions = vec![p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.)];
        let faces = vec![vec![0, 1, 2], vec![0, 2, 3]];
        let mats = vec![NO_MATERIAL; 2];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 2];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 2];
        let shells = split_into_shells(&positions, &faces, &mats, &uvs, &holes);
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].faces.len(), 2);
    }

    /// Two triangles with no shared vertex are two separate shells.
    #[test]
    fn disjoint_triangles_are_two_shells() {
        let positions = vec![
            p(0., 0., 0.),
            p(1., 0., 0.),
            p(0., 1., 0.),
            p(10., 10., 10.),
            p(11., 10., 10.),
            p(10., 11., 10.),
        ];
        let faces = vec![vec![0, 1, 2], vec![3, 4, 5]];
        let mats = vec![NO_MATERIAL; 2];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); 2];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); 2];
        let shells = split_into_shells(&positions, &faces, &mats, &uvs, &holes);
        assert_eq!(shells.len(), 2);
        for shell in &shells {
            assert_eq!(shell.faces.len(), 1);
            assert_eq!(shell.positions.len(), 3);
        }
    }

    #[test]
    fn closed_tetrahedron_is_closed() {
        // A tetrahedron: 4 faces, every edge shared by exactly two faces in
        // opposite directions.
        let faces = vec![vec![0, 2, 1], vec![0, 1, 3], vec![1, 2, 3], vec![2, 0, 3]];
        assert!(is_closed_shell(&faces));
    }

    #[test]
    fn open_triangle_is_not_closed() {
        let faces = vec![vec![0usize, 1, 2]];
        assert!(!is_closed_shell(&faces));
    }
}
