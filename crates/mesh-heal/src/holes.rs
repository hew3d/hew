//! Overlapping hole loops in imported faces.
//!
//! A face whose holes overlap (`kernel::TopologyError::HolesOverlap`) has no
//! region between them to draw, and the kernel's import builder refuses the
//! whole mesh for it. SketchUp can hold such a face: a circle drawn twice a
//! few hundredths of a millimeter apart leaves one face with two nearly
//! identical hole loops. [`resolve_overlapping_holes`] settles each such
//! face before the mesh is built, and says what it did so the importer can
//! report it (DEVELOPMENT.md rule 4: loud, never silent):
//!
//! - two holes that trace one outline within the import tolerance
//!   ([`kernel::tol::IMPORT_PLANE_DIST`]) are one hole drawn twice: one loop
//!   is dropped, keeping the loop other faces use (the hole's walls), else
//!   the first;
//! - any other overlap leaves the face out of the mesh. Nothing is merged:
//!   a union of two near-identical outlines would be all micron edges.

use std::collections::BTreeMap;

use kernel::{Plane, Point3, tol};

/// What [`resolve_overlapping_holes`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HoleOverlaps {
    /// Hole loops dropped as a second copy of another hole of their face.
    pub duplicates_dropped: usize,
    /// Faces whose holes overlap otherwise, ascending: the caller leaves
    /// them out of the mesh.
    pub faces_left_out: Vec<usize>,
}

impl HoleOverlaps {
    /// Nothing overlapped.
    pub fn is_empty(&self) -> bool {
        self.duplicates_dropped == 0 && self.faces_left_out.is_empty()
    }
}

/// Settle every face of a mesh whose hole loops overlap: drop the second
/// copy of a hole drawn twice from `face_holes`, and list each face whose
/// holes overlap any other way (see the module docs). `faces` and
/// `face_holes` are parallel, index lists into `positions`. A dropped
/// loop's vertices may be left unused; the caller compacts
/// ([`crate::compact_unused`]) after leaving the listed faces out.
pub fn resolve_overlapping_holes(
    positions: &[Point3],
    faces: &[Vec<usize>],
    face_holes: &mut [Vec<Vec<usize>>],
) -> HoleOverlaps {
    let mut out = HoleOverlaps::default();
    if !face_holes.iter().any(|h| h.len() >= 2) {
        return out;
    }
    let ring = |ids: &[usize]| -> Vec<Point3> {
        ids.iter()
            .filter_map(|&i| positions.get(i).copied())
            .collect()
    };
    // Which faces use each undirected edge, to tell a hole with walls from
    // a stray copy.
    let mut users: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (fi, outer) in faces.iter().enumerate() {
        let holes = face_holes.get(fi).map(Vec::as_slice).unwrap_or(&[]);
        for lp in std::iter::once(outer).chain(holes) {
            for k in 0..lp.len() {
                let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
                let list = users.entry((a.min(b), a.max(b))).or_default();
                if !list.contains(&fi) {
                    list.push(fi);
                }
            }
        }
    }
    let used_elsewhere = |fi: usize, lp: &[usize]| {
        (0..lp.len()).any(|k| {
            let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
            users
                .get(&(a.min(b), a.max(b)))
                .is_some_and(|l| l.iter().any(|&g| g != fi))
        })
    };
    for (fi, outer) in faces.iter().enumerate() {
        let Some(holes) = face_holes.get_mut(fi) else {
            continue;
        };
        if holes.len() < 2 {
            continue;
        }
        let Ok(plane) = Plane::from_polygon(&ring(outer)) else {
            continue; // the builder refuses the face itself
        };
        loop {
            let rings: Vec<Vec<Point3>> = holes.iter().map(|h| ring(h)).collect();
            let Some((i, j)) = kernel::first_overlapping_holes(&rings, plane.normal()) else {
                break;
            };
            if !kernel::rings_trace_each_other(&rings[i], &rings[j], tol::IMPORT_PLANE_DIST) {
                out.faces_left_out.push(fi);
                break;
            }
            let drop = if used_elsewhere(fi, &holes[j]) && !used_elsewhere(fi, &holes[i]) {
                i
            } else {
                j
            };
            holes.remove(drop);
            out.duplicates_dropped += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20 x 20 square face at z = 0 (CCW from +Z) and `rings` as its holes
    /// (each CW from +Z); returns positions, faces and holes.
    #[allow(clippy::type_complexity)]
    fn face(rings: &[Vec<(f64, f64)>]) -> (Vec<Point3>, Vec<Vec<usize>>, Vec<Vec<Vec<usize>>>) {
        let mut positions = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(20.0, 0.0, 0.0),
            Point3::new(20.0, 20.0, 0.0),
            Point3::new(0.0, 20.0, 0.0),
        ];
        let mut holes = Vec::new();
        for r in rings {
            let base = positions.len();
            positions.extend(r.iter().map(|&(x, y)| Point3::new(x, y, 0.0)));
            holes.push((base..base + r.len()).collect::<Vec<_>>());
        }
        (positions, vec![vec![0, 1, 2, 3]], vec![holes])
    }

    fn circle(cx: f64, cy: f64, r: f64, phase: f64) -> Vec<(f64, f64)> {
        (0..24)
            .map(|i| {
                let a = -std::f64::consts::TAU * (i as f64 / 24.0 + phase);
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    }

    fn square(x0: f64, y0: f64, s: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x0, y0 + s), (x0 + s, y0 + s), (x0 + s, y0)]
    }

    #[test]
    fn a_hole_drawn_twice_keeps_the_first_copy() {
        let (positions, faces, mut holes) = face(&[
            circle(10.0, 10.0, 0.004, 0.0),
            circle(10.0 + 3e-5, 10.0, 0.004, 0.3),
        ]);
        let first = holes[0][0].clone();
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert_eq!(
            out,
            HoleOverlaps {
                duplicates_dropped: 1,
                faces_left_out: Vec::new()
            }
        );
        assert_eq!(holes[0], vec![first]);
    }

    #[test]
    fn a_hole_drawn_twice_keeps_the_copy_its_walls_use() {
        let (mut positions, mut faces, mut holes) = face(&[
            circle(10.0, 10.0, 0.004, 0.0),
            circle(10.0 + 3e-5, 10.0, 0.004, 0.3),
        ]);
        // A wall quad below the second copy's first edge.
        let walled = holes[0][1].clone();
        let (a, b) = (walled[0], walled[1]);
        let base = positions.len();
        positions.push(positions[a] + kernel::Vec3::new(0.0, 0.0, -1.0));
        positions.push(positions[b] + kernel::Vec3::new(0.0, 0.0, -1.0));
        faces.push(vec![b, a, base, base + 1]);
        holes.push(Vec::new());
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert_eq!(out.duplicates_dropped, 1);
        assert!(out.faces_left_out.is_empty());
        assert_eq!(holes[0], vec![walled]);
    }

    #[test]
    fn holes_crossing_each_other_leave_the_face_out() {
        let (positions, faces, mut holes) = face(&[square(5.0, 5.0, 4.0), square(7.0, 7.0, 4.0)]);
        let before = holes.clone();
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert_eq!(
            out,
            HoleOverlaps {
                duplicates_dropped: 0,
                faces_left_out: vec![0]
            }
        );
        assert_eq!(holes, before, "a face left out keeps its loops");
    }

    #[test]
    fn disjoint_holes_are_untouched() {
        let (positions, faces, mut holes) = face(&[
            square(2.0, 2.0, 3.0),
            square(5.0, 5.0, 3.0),
            square(12.0, 2.0, 3.0),
        ]);
        let before = holes.clone();
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(holes, before);
    }

    #[test]
    fn an_empty_hole_loop_beside_a_valid_one_is_left_to_the_builder() {
        // An empty loop survives index filtering; it bounds nothing, so it
        // overlaps nothing here, and the builder refuses it typed.
        let (positions, faces, mut holes) = face(&[Vec::new(), square(2.0, 2.0, 3.0)]);
        let before = holes.clone();
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(holes, before);
        let recipe = kernel::MeshRecipe {
            name: "plate".into(),
            positions,
            face_materials: vec![kernel::NO_MATERIAL; faces.len()],
            face_uv_frames: vec![None; faces.len()],
            faces,
            face_holes: holes,
            base_material: kernel::NO_MATERIAL,
            tags: Vec::new(),
            extra: kernel::MeshExtra::default(),
        };
        assert_eq!(
            recipe.check_builds(),
            Err(kernel::TopologyError::DegenerateFace { face: 0 })
        );
        // And beside a real overlap, the overlap is still found.
        let (positions, faces, mut holes) = face(&[
            Vec::new(),
            square(5.0, 5.0, 4.0),
            vec![(1.0, 1.0), (2.0, 2.0)],
            square(7.0, 7.0, 4.0),
        ]);
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert_eq!(out.faces_left_out, vec![0]);
    }

    #[test]
    fn a_resolved_face_builds() {
        let (positions, faces, mut holes) = face(&[
            circle(10.0, 10.0, 0.004, 0.0),
            circle(10.0 + 3e-5, 10.0, 0.004, 0.3),
            square(2.0, 2.0, 3.0),
        ]);
        let recipe = |positions: &[Point3], faces: &[Vec<usize>], holes: &[Vec<Vec<usize>>]| {
            kernel::MeshRecipe {
                name: "plate".into(),
                positions: positions.to_vec(),
                face_materials: vec![kernel::NO_MATERIAL; faces.len()],
                face_uv_frames: vec![None; faces.len()],
                faces: faces.to_vec(),
                face_holes: holes.to_vec(),
                base_material: kernel::NO_MATERIAL,
                tags: Vec::new(),
                extra: kernel::MeshExtra::default(),
            }
        };
        assert_eq!(
            recipe(&positions, &faces, &holes).check_builds(),
            Err(kernel::TopologyError::OverlappingHoles {
                face: 0,
                hole: 0,
                other: 1
            })
        );
        let out = resolve_overlapping_holes(&positions, &faces, &mut holes);
        assert_eq!(out.duplicates_dropped, 1);
        let (positions, faces, holes) = crate::compact_unused(&positions, &faces, &holes);
        assert_eq!(recipe(&positions, &faces, &holes).check_builds(), Ok(()));
    }
}
