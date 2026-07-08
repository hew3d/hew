//! Non-manifold mesh splitting (docs/INTEROP.md): cut a mesh the kernel
//! would reject into pieces it accepts, WITHOUT repairing geometry.
//!
//! The kernel's `from_polygons` rejects any mesh where a directed edge
//! `a → b` is traversed by more than one face loop (non-manifold or
//! inconsistently wound input — see `kernel::TopologyError::NonManifoldEdge`).
//! Real-world SketchUp content routinely contains such meshes (fins, walls
//! sharing an edge with a floor, internal partitions); SketchUp's own
//! exporters sidestep the issue by decomposing on export. This pass does the
//! equivalent decomposition at import:
//!
//! 1. **Cut at non-manifold undirected edges** — an undirected edge with
//!    more than two incident face-uses, or whose uses traverse the same
//!    direction twice, stops being a connection: faces connect only across
//!    clean two-use opposite-direction edges. Connected components become
//!    separate meshes. A cut edge turns into ordinary boundary — the pieces
//!    are honestly *open* (leaky) shells, never patched shut.
//! 2. **Pinch fallback (vertex split)** — when both traversals of a
//!    duplicated directed edge end up in the SAME component (connected
//!    around a pinch), the later face-use is detached by re-indexing that
//!    face's endpoints onto duplicated (coincident) vertices. The crack is
//!    real and stays visible; the piece is leaky by construction.
//!
//! Geometry is never altered — no vertex moves, no face drops, no loop
//! rewinding. Every output piece satisfies the kernel's directed-edge
//! precondition (property-tested), so a caller can hand each piece to
//! `from_polygons` knowing the *only* remaining rejections are
//! non-topological (degenerate/non-planar faces).
//!
//! Callers MUST report the split loudly (DEVELOPMENT.md rule 4 — this is
//! boundary decomposition at the import seam, and the user is told).

use std::collections::BTreeMap;

use kernel::Point3;

/// One split-off piece: the same parallel arrays `heal_mesh` produces,
/// re-indexed against its own compacted `positions`.
pub struct MeshPiece {
    pub positions: Vec<Point3>,
    pub faces: Vec<Vec<usize>>,
    pub face_materials: Vec<u32>,
    pub face_corner_uvs: Vec<Vec<[f64; 2]>>,
    pub face_holes: Vec<Vec<Vec<usize>>>,
}

/// Splits a mesh that violates the kernel's directed-edge precondition into
/// pieces that satisfy it. Returns `None` when the mesh is already clean
/// (the common case — one cheap O(edges) scan). `Some(pieces)` otherwise;
/// `pieces.len()` may be 1 when only the pinch fallback fired.
///
/// Inputs are the parallel arrays `heal_mesh` returns (positions, outer
/// rings, per-face materials / corner UVs / hole rings).
pub fn split_non_manifold(
    positions: &[Point3],
    faces: &[Vec<usize>],
    face_materials: &[u32],
    face_corner_uvs: &[Vec<[f64; 2]>],
    face_holes: &[Vec<Vec<usize>>],
) -> Option<Vec<MeshPiece>> {
    if !has_duplicate_directed_edge(faces, face_holes) {
        return None;
    }

    // ── Undirected-edge incidence over ALL loops (outer + holes) ─────────
    // Uses are recorded per face; hole loops belong to their face.
    #[derive(Default)]
    struct EdgeUses {
        /// (face, forward?) — forward = (min,max) traversed as min→max.
        uses: Vec<(usize, bool)>,
    }
    let mut edges: BTreeMap<(usize, usize), EdgeUses> = BTreeMap::new();
    let record = |f: usize, ring: &[usize], edges: &mut BTreeMap<(usize, usize), EdgeUses>| {
        for k in 0..ring.len() {
            let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
            let key = (a.min(b), a.max(b));
            edges.entry(key).or_default().uses.push((f, a <= b));
        }
    };
    for (f, ring) in faces.iter().enumerate() {
        record(f, ring, &mut edges);
        for hole in &face_holes[f] {
            record(f, hole, &mut edges);
        }
    }

    // ── Union faces across CLEAN edges only ──────────────────────────────
    // Clean = exactly two uses, opposite directions, distinct faces. (Two
    // uses by the SAME face — a slit — is not a connection anyway.)
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for eu in edges.values() {
        if let [(f0, d0), (f1, d1)] = eu.uses[..]
            && f0 != f1
            && d0 != d1
        {
            let (r0, r1) = (find(&mut parent, f0), find(&mut parent, f1));
            parent[r0] = r1;
        }
    }

    // ── Gather components (in first-face order, deterministic) ───────────
    let mut comp_of_root: BTreeMap<usize, usize> = BTreeMap::new();
    let mut comp_faces: Vec<Vec<usize>> = Vec::new();
    for f in 0..faces.len() {
        let r = find(&mut parent, f);
        let c = *comp_of_root.entry(r).or_insert_with(|| {
            comp_faces.push(Vec::new());
            comp_faces.len() - 1
        });
        comp_faces[c].push(f);
    }

    // ── Emit pieces, vertex-splitting residual pinches ────────────────────
    let mut pieces: Vec<MeshPiece> = Vec::with_capacity(comp_faces.len());
    for face_ids in &comp_faces {
        // Copy this component's rings (global indices, then pinch-split,
        // then compact).
        let mut rings: Vec<Vec<usize>> = face_ids.iter().map(|&f| faces[f].clone()).collect();
        let mut holes: Vec<Vec<Vec<usize>>> =
            face_ids.iter().map(|&f| face_holes[f].clone()).collect();
        let mut extra_positions: Vec<Point3> = Vec::new(); // appended past globals

        // Pinch fallback: re-scan for duplicate directed edges within the
        // component; detach every repeat face-use by re-indexing that
        // ring's endpoints onto fresh coincident duplicates. Loop until
        // clean — each detach strictly reduces duplicate uses.
        loop {
            let mut seen: BTreeMap<(usize, usize), (usize, bool)> = BTreeMap::new(); // -> (local face, is_hole)
            let mut dup: Option<(usize, bool, usize, usize)> = None; // (local face, is_hole, a, b)
            'scan: for (lf, ring) in rings.iter().enumerate() {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    if seen.insert((a, b), (lf, false)).is_some() {
                        dup = Some((lf, false, a, b));
                        break 'scan;
                    }
                }
                for hole in &holes[lf] {
                    for k in 0..hole.len() {
                        let (a, b) = (hole[k], hole[(k + 1) % hole.len()]);
                        if seen.insert((a, b), (lf, true)).is_some() {
                            dup = Some((lf, true, a, b));
                            break 'scan;
                        }
                    }
                }
            }
            let Some((lf, is_hole, a, b)) = dup else {
                break;
            };
            // Duplicate both endpoints for THIS face (outer ring + its
            // holes re-index together so the face stays self-consistent).
            let base = positions.len() + extra_positions.len();
            let pos_of = |v: usize, extra: &[Point3]| -> Point3 {
                if v < positions.len() {
                    positions[v]
                } else {
                    extra[v - positions.len()]
                }
            };
            let (pa, pb) = (pos_of(a, &extra_positions), pos_of(b, &extra_positions));
            extra_positions.push(pa); // base     = a'
            extra_positions.push(pb); // base + 1 = b'
            let remap = |v: usize| -> usize {
                if v == a {
                    base
                } else if v == b {
                    base + 1
                } else {
                    v
                }
            };
            let _ = is_hole; // the whole face re-indexes either way
            for v in rings[lf].iter_mut() {
                *v = remap(*v);
            }
            for hole in holes[lf].iter_mut() {
                for v in hole.iter_mut() {
                    *v = remap(*v);
                }
            }
        }

        // Compact: global/extra indices → piece-local, keeping only
        // referenced positions, in first-use order (deterministic).
        let mut local: BTreeMap<usize, usize> = BTreeMap::new();
        let mut piece_positions: Vec<Point3> = Vec::new();
        let mut localize = |v: usize, piece_positions: &mut Vec<Point3>| -> usize {
            *local.entry(v).or_insert_with(|| {
                piece_positions.push(if v < positions.len() {
                    positions[v]
                } else {
                    extra_positions[v - positions.len()]
                });
                piece_positions.len() - 1
            })
        };
        let piece_faces: Vec<Vec<usize>> = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|&v| localize(v, &mut piece_positions))
                    .collect()
            })
            .collect();
        let piece_holes: Vec<Vec<Vec<usize>>> = holes
            .iter()
            .map(|face_holes| {
                face_holes
                    .iter()
                    .map(|hole| {
                        hole.iter()
                            .map(|&v| localize(v, &mut piece_positions))
                            .collect()
                    })
                    .collect()
            })
            .collect();

        // Re-run the winding pass on the piece: the pre-split
        // `orient_consistent` cannot propagate across the non-manifold
        // seams this pass just cut, so each side may have seeded its own
        // (opposite) orientation. Within a single piece the surviving
        // shared edges are clean two-use edges, so the flood-fill now
        // succeeds — the same well-specified normalization the main heal
        // pipeline applies, not a repair. (No `orient_outward`: an OPEN
        // shell has no defined inside; the renderer draws open shells
        // double-sided instead of guessing.)
        let piece_mats: Vec<u32> = face_ids.iter().map(|&f| face_materials[f]).collect();
        let piece_uvs: Vec<Vec<[f64; 2]>> = face_ids
            .iter()
            .map(|&f| face_corner_uvs[f].clone())
            .collect();
        let (piece_faces, piece_mats, piece_uvs, piece_holes) =
            crate::orient_consistent(&piece_faces, &piece_mats, &piece_uvs, &piece_holes);

        pieces.push(MeshPiece {
            positions: piece_positions,
            faces: piece_faces,
            face_materials: piece_mats,
            face_corner_uvs: piece_uvs,
            face_holes: piece_holes,
        });
    }

    Some(pieces)
}

/// One cheap scan: does any directed edge (over outer rings AND hole rings)
/// repeat? This is exactly the kernel's `NonManifoldEdge` precondition.
fn has_duplicate_directed_edge(faces: &[Vec<usize>], face_holes: &[Vec<Vec<usize>>]) -> bool {
    let mut seen: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    let mut check = |ring: &[usize]| -> bool {
        for k in 0..ring.len() {
            let e = (ring[k], ring[(k + 1) % ring.len()]);
            if !seen.insert(e) {
                return true;
            }
        }
        false
    };
    for (f, ring) in faces.iter().enumerate() {
        if check(ring) {
            return true;
        }
        for hole in &face_holes[f] {
            if check(hole) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod specs {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    /// No-hole, no-UV convenience caller.
    fn split(positions: &[Point3], faces: &[Vec<usize>]) -> Option<Vec<MeshPiece>> {
        let mats = vec![0u32; faces.len()];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); faces.len()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); faces.len()];
        split_non_manifold(positions, faces, &mats, &uvs, &holes)
    }

    fn assert_clean(piece: &MeshPiece) {
        assert!(
            !has_duplicate_directed_edge(&piece.faces, &piece.face_holes),
            "piece still violates the directed-edge precondition"
        );
        // Every index in range.
        for ring in piece.faces.iter().chain(piece.face_holes.iter().flatten()) {
            for &v in ring {
                assert!(v < piece.positions.len());
            }
        }
    }

    /// A clean quad → None (fast path).
    #[test]
    fn clean_mesh_returns_none() {
        let pos = [p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.)];
        let faces = vec![vec![0, 1, 2, 3]];
        assert!(split(&pos, &faces).is_none());
    }

    /// Two triangles traversing the SAME directed edge (inconsistent
    /// winding, no other connection) → two single-face pieces.
    #[test]
    fn same_direction_pair_splits_into_two() {
        let pos = [p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.), p(1., 1., 1.)];
        // Both traverse 0→1.
        let faces = vec![vec![0, 1, 2], vec![0, 1, 3]];
        let pieces = split(&pos, &faces).expect("must split");
        assert_eq!(pieces.len(), 2);
        for piece in &pieces {
            assert_eq!(piece.faces.len(), 1);
            assert_clean(piece);
        }
    }

    /// Three faces sharing one undirected edge (a fin): the edge is
    /// non-manifold → three pieces.
    #[test]
    fn three_face_fin_splits_into_three() {
        let pos = [
            p(0., 0., 0.),
            p(0., 0., 1.),
            p(1., 0., 0.),
            p(-1., 0., 0.),
            p(0., 1., 0.),
        ];
        // Shared edge 0-1; windings alternate so two of them oppose.
        let faces = vec![vec![0, 1, 2], vec![1, 0, 3], vec![0, 1, 4]];
        let pieces = split(&pos, &faces).expect("must split");
        assert_eq!(pieces.len(), 3);
        for piece in &pieces {
            assert_clean(piece);
        }
    }

    /// Two closed cubes sharing ONE edge (4 uses): cut there → each cube
    /// comes out whole, and each piece is directed-edge-clean with every
    /// edge paired (watertight-shaped).
    #[test]
    fn two_cubes_sharing_an_edge_split_into_watertight_pieces() {
        // Cube A on [0,1]^3, cube B on [1,2]x[0,1]x... sharing the edge
        // x=1,y=0? Simpler: build both cubes explicitly, sharing vertices
        // 1 (1,0,0) and 5 (1,0,1).
        let pos = [
            // cube A 0..8
            p(0., 0., 0.),
            p(1., 0., 0.),
            p(1., 1., 0.),
            p(0., 1., 0.),
            p(0., 0., 1.),
            p(1., 0., 1.),
            p(1., 1., 1.),
            p(0., 1., 1.),
            // cube B extra 8..14 (shares 1 and 5; spans y in [-1,0])
            p(2., 0., 0.),
            p(2., -1., 0.),
            p(1., -1., 0.),
            p(2., 0., 1.),
            p(2., -1., 1.),
            p(1., -1., 1.),
        ];
        let cube = |v: [usize; 8]| -> Vec<Vec<usize>> {
            vec![
                vec![v[0], v[3], v[2], v[1]], // bottom (z-)
                vec![v[4], v[5], v[6], v[7]], // top (z+)
                vec![v[0], v[1], v[5], v[4]], // front
                vec![v[2], v[3], v[7], v[6]], // back
                vec![v[1], v[2], v[6], v[5]], // right
                vec![v[3], v[0], v[4], v[7]], // left
            ]
        };
        let mut faces = cube([0, 1, 2, 3, 4, 5, 6, 7]);
        // Cube B reuses vertices 1 (as its 0-corner) and 5 (its 4-corner):
        // corners: 0:(1,0,0)=1, 1:(2,0,0)=8, 2:(2,-1,0)=9, 3:(1,-1,0)=10,
        //          4:(1,0,1)=5, 5:(2,0,1)=11, 6:(2,-1,1)=12, 7:(1,-1,1)=13
        faces.extend(cube([1, 8, 9, 10, 5, 11, 12, 13]));
        let pieces = split(&pos, &faces).expect("must split");
        assert_eq!(pieces.len(), 2);
        for piece in &pieces {
            assert_eq!(piece.faces.len(), 6);
            assert_clean(piece);
            // Fully paired: every directed edge has its reverse (the cut
            // edge kept both of this cube's own opposite uses).
            let mut set = std::collections::BTreeSet::new();
            for ring in &piece.faces {
                for k in 0..ring.len() {
                    set.insert((ring[k], ring[(k + 1) % ring.len()]));
                }
            }
            for &(a, b) in set.iter() {
                assert!(set.contains(&(b, a)), "unpaired edge {a}->{b}");
            }
        }
    }

    /// A pinch: the two faces share BOTH a clean opposite-direction edge
    /// ({0,1}: 0→1 vs 1→0 — a real connection the cut keeps) and a
    /// same-direction duplicate (2→3 in both). Cutting cannot separate
    /// them, so the vertex-split fallback must fire: one piece, clean,
    /// with the repeat face re-indexed onto duplicated coincident
    /// vertices, and no face lost.
    #[test]
    fn pinch_falls_back_to_vertex_split() {
        let pos = [p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.)];
        let faces = vec![vec![0, 1, 2, 3], vec![1, 0, 2, 3]];
        let pieces = split(&pos, &faces).expect("must split");
        assert_eq!(pieces.len(), 1, "the clean shared edge keeps them together");
        let piece = &pieces[0];
        assert_clean(piece);
        assert_eq!(piece.faces.len(), 2);
        // The detached endpoints were duplicated (coincident, not welded).
        assert_eq!(piece.positions.len(), 6);
    }

    /// THE invariant (property): for arbitrary index soups, every piece
    /// `split_non_manifold` returns satisfies the kernel's directed-edge
    /// precondition, no face is lost, and all indices are in range.
    #[test]
    fn property_every_piece_is_directed_edge_clean() {
        use proptest::prelude::*;
        let ring = proptest::collection::vec(0usize..8, 3..6).prop_filter(
            "ring must not repeat a vertex (degenerate rings are the \
             welder's problem, not the splitter's)",
            |r| {
                let mut s = r.clone();
                s.sort_unstable();
                s.windows(2).all(|w| w[0] != w[1])
            },
        );
        let mesh = proptest::collection::vec(ring, 1..12);
        proptest!(ProptestConfig::with_cases(512), |(faces in mesh)| {
            let positions: Vec<Point3> = (0..8)
                .map(|i| p(f64::from(i as u8), f64::from((i * i) as u8), 0.0))
                .collect();
            if let Some(pieces) = split(&positions, &faces) {
                let total: usize = pieces.iter().map(|pc| pc.faces.len()).sum();
                prop_assert_eq!(total, faces.len(), "no face may be lost");
                for piece in &pieces {
                    prop_assert!(!has_duplicate_directed_edge(
                        &piece.faces,
                        &piece.face_holes
                    ));
                    for ring in &piece.faces {
                        for &v in ring {
                            prop_assert!(v < piece.positions.len());
                        }
                    }
                }
            } else {
                // None must mean already clean.
                let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); faces.len()];
                prop_assert!(!has_duplicate_directed_edge(&faces, &holes));
            }
        });
    }

    /// Every emitted piece is internally consistently wound: no clean
    /// two-use shared edge is traversed the same direction twice — the
    /// per-piece `orient_consistent` re-run must have fixed what the
    /// pre-split pass could not reach across the non-manifold seam.
    #[test]
    fn pieces_come_out_consistently_wound() {
        // Two quads sharing edge 1-2 with the SAME direction (1→2 in both:
        // inconsistent winding) plus a fin at 0-1 making the mesh
        // non-manifold so the splitter engages.
        let pos = [
            p(0., 0., 0.),
            p(1., 0., 0.),
            p(1., 1., 0.),
            p(0., 1., 0.),
            p(2., 0., 0.),
            p(2., 1., 0.),
            p(0., 0., 1.),
        ];
        let faces = vec![
            vec![0, 1, 2, 3],
            vec![4, 5, 2, 1], // traverses 2→1? edges 4→5,5→2,2→1,1→4 — flip:
            vec![1, 2, 5, 4], // SAME direction 1→2 as f0 → inconsistent pair
            vec![0, 1, 6],    // fin sharing 0-1 (0→1 dup with f0) → non-manifold
        ];
        let _ = faces; // (use the inconsistent construction below)
        let faces = vec![vec![0, 1, 2, 3], vec![1, 2, 5, 4], vec![0, 1, 6]];
        let pieces = split(&pos, &faces).expect("must split");
        for piece in &pieces {
            assert_clean(piece);
            // consistency: every shared two-use undirected edge is used in
            // OPPOSITE directions.
            let mut dir: BTreeMap<(usize, usize), Vec<bool>> = BTreeMap::new();
            for ring in &piece.faces {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    dir.entry((a.min(b), a.max(b))).or_default().push(a <= b);
                }
            }
            for (_, uses) in dir {
                if uses.len() == 2 {
                    assert_ne!(uses[0], uses[1], "shared edge wound same way twice");
                }
            }
        }
    }

    /// Materials / UVs / holes ride along with their faces.
    #[test]
    fn per_face_payloads_follow_their_faces() {
        let pos = [p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.), p(1., 1., 1.)];
        let faces = vec![vec![0, 1, 2], vec![0, 1, 3]];
        let mats = vec![7u32, 9u32];
        let uvs = vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], Vec::new()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(), Vec::new()];
        let pieces = split_non_manifold(&pos, &faces, &mats, &uvs, &holes).expect("split");
        assert_eq!(pieces.len(), 2);
        let with7 = pieces
            .iter()
            .find(|pc| pc.face_materials == [7])
            .expect("piece with material 7");
        assert_eq!(with7.face_corner_uvs[0].len(), 3);
        let with9 = pieces
            .iter()
            .find(|pc| pc.face_materials == [9])
            .expect("piece with material 9");
        assert!(with9.face_corner_uvs[0].is_empty());
    }
}
