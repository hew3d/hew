//! Editing blends (WP08 §1–3, decision I-10): what blends an object has,
//! restoring the sharp geometry under one, and removing or resizing it. A
//! blend made since records exist replays what it recorded
//! ([`super::record`]); one without a record re-derives the sharp corner
//! from the planes around the blend, accepted only when re-blending the
//! restored edges reproduces the blend exactly.

use std::collections::{BTreeMap, BTreeSet};

use super::plan::PlanFail;
use super::{BlendError, BlendProfile, BlendReport, BlendSpec, CornerStyle};
use crate::ids::{EdgeId, FaceId};
use crate::math::{Point3, Vec3};
use crate::patch::PVert;
use crate::tol;
use crate::topo::{BlendId, BlendKind, BlendRole, Object, SurfaceRef};

/// One blend on an object, as derived from its tagged faces.
#[derive(Debug, Clone, PartialEq)]
pub struct BlendFeature {
    /// The blend's id.
    pub id: BlendId,
    /// Its kind.
    pub kind: BlendKind,
    /// Its faces, in slot order.
    pub faces: Vec<FaceId>,
    /// The size read back (radius, setback A, or profile width); `None`
    /// when it cannot be derived.
    pub size: Option<f64>,
    /// Whether restoration and its verification succeed: the blend can be
    /// removed or resized.
    pub intact: bool,
}

/// A verified restoration.
pub(crate) struct Restored {
    pub object: Object,
    /// The verified re-blend: restored edges, profile, stops and handles.
    pub spec: BlendSpec,
    /// Faces of `self` re-added by the restoration → their new ids.
    pub face_map: BTreeMap<FaceId, FaceId>,
    /// The restored sharp edges, in the restored object.
    pub edges: Vec<EdgeId>,
    /// The size read back (radius, setback A, profile width).
    pub size: f64,
    /// Later blends taken off ahead of this one, to put back with
    /// [`Object::rewind_blends`] once the edit is made.
    pub dependents: Vec<super::record::Dependent>,
}

impl Restored {
    /// Blends the restored edges as `spec` (a resize: the restored spec at
    /// a new size or profile) under blend `id`, the restored blend's, and
    /// puts the later blends back: the resize, Push/Pull on a blend face,
    /// and the resize preview at any size. The blend is made as a fresh
    /// fillet of those edges would be ([`Object::blend_edges`]): where they
    /// run into other blends, the corner is solved again with them, larger
    /// sizes first, so editing a size gives what removing the blend and
    /// filleting again at the new size gives. The blend's edges are the
    /// edges it was made with, tangent propagation at the re-solve included,
    /// so a fillet that wrapped a larger neighbour's end arc resizes as that
    /// whole run and merges with the neighbour at its size.
    pub(crate) fn reblend(
        &self,
        id: BlendId,
        spec: &BlendSpec,
    ) -> Result<(Object, BlendReport), BlendError> {
        // The resized blend keeps its id (unless the re-solve merges it with
        // an equal neighbour, into one blend under a fresh id, as a fresh
        // fillet does); blends it is put back under keep theirs, so a fresh
        // id stays clear of them.
        let _reserved = super::ReservedBlendIds::new(self.dependents.iter().map(|d| d.id));
        let go = || {
            let (after, report) =
                super::with_forced_blend_id(Some(id), || self.object.blend_edges(spec))?;
            after.rewind_report(report, &self.dependents)
        };
        if self.dependents.is_empty() {
            go()
        } else {
            // Putting a later blend back can restore the blends just made,
            // which replays their records: a preview takes them too.
            super::with_records(go)
        }
    }

    /// The spec a resize to `profile` re-blends: the restored edges with
    /// the new cross-section, the stops and the primary face kept, and the
    /// radius handles scaled with the size when the kind stays (a round's
    /// handles mean nothing to a chamfer). The resize preview uses the same
    /// spec, so it shows what the commit makes.
    pub(crate) fn resized_spec(&self, profile: &BlendProfile) -> BlendSpec {
        let same_kind =
            std::mem::discriminant(profile) == std::mem::discriminant(&self.spec.profile);
        let ratio = match (profile.size(), self.spec.profile.size()) {
            (Some(n), Some(o)) if same_kind && o > 0.0 => n / o,
            _ => 1.0,
        };
        BlendSpec {
            edges: self.edges.clone(),
            profile: profile.clone(),
            primary: self.spec.primary,
            corners: self.spec.corners,
            stops: self.spec.stops,
            handles: if same_kind {
                self.spec
                    .handles
                    .iter()
                    .map(|h| super::RadiusHandle {
                        at: h.at,
                        radius: h.radius * ratio,
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }
    }
}

/// Test hook for [`same_geometry`].
#[doc(hidden)]
pub fn same_geometry_for_test(x: &Object, y: &Object, tolerance: f64) -> bool {
    same_geometry(x, y, tolerance)
}

/// Same faces, as cyclically matching rings of positions within `tol`
/// (winding kept), in any order.
pub(crate) fn same_geometry(x: &Object, y: &Object, tolerance: f64) -> bool {
    let rings = |o: &Object| -> Vec<(Vec<Vec<Point3>>, Point3)> {
        o.faces()
            .values()
            .map(|f| {
                let loops: Vec<Vec<Point3>> = std::iter::once(f.outer_loop)
                    .chain(f.inner_loops.iter().copied())
                    .map(|l| o.loop_positions(l).collect())
                    .collect();
                let c = loops[0]
                    .iter()
                    .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
                    / loops[0].len() as f64;
                (loops, Point3::new(c.x, c.y, c.z))
            })
            .collect()
    };
    let cyclic = |a: &[Point3], b: &[Point3]| {
        a.len() == b.len()
            && (0..b.len()).any(|s| {
                a.iter()
                    .enumerate()
                    .all(|(i, p)| p.approx_eq(b[(i + s) % b.len()], tolerance))
            })
    };
    let same_face = |a: &[Vec<Point3>], b: &[Vec<Point3>]| {
        a.len() == b.len()
            && cyclic(&a[0], &b[0])
            && a[1..].iter().all(|h| b[1..].iter().any(|g| cyclic(h, g)))
    };
    let xs = rings(x);
    let ys = rings(y);
    if xs.len() != ys.len() {
        return false;
    }
    // Faces by centroid in a grid of cells wider than the tolerance, so a
    // match is in the same or a neighboring cell (a scan has 400k faces).
    let grid = CentroidGrid::new(ys.iter().map(|(_, c)| *c), tolerance);
    let mut used = vec![false; ys.len()];
    for (loops, c) in xs {
        let Some(i) = grid
            .near(c)
            .find(|&i| !used[i] && c.approx_eq(ys[i].1, tolerance) && same_face(&loops, &ys[i].0))
        else {
            return false;
        };
        used[i] = true;
    }
    same_marks(x, y, tolerance)
}

/// The same soft and crease edges away from blends, matched by midpoint
/// within `tol`. A blend's own edges (its seams, its rails) are rebuilt
/// with it, so a mark on one is not the restore's to keep.
fn same_marks(x: &Object, y: &Object, tolerance: f64) -> bool {
    let marked = |o: &Object, crease: bool| -> Vec<Point3> {
        let on_blend = |e: &crate::topo::Edge| {
            std::iter::once(e.half_edge)
                .chain(e.twin_half_edge)
                .any(|h| {
                    o.faces()[o.loops()[o.half_edges()[h].loop_id].face]
                        .blend
                        .is_some()
                })
        };
        o.edges()
            .iter()
            .filter(|(_, e)| (if crease { e.crease } else { e.soft }) && !on_blend(e))
            .filter_map(|(id, _)| o.edge_endpoints(id))
            .map(|(a, b)| a + (b - a) * 0.5)
            .collect()
    };
    [false, true].into_iter().all(|crease| {
        let (xs, ys) = (marked(x, crease), marked(y, crease));
        if xs.len() != ys.len() {
            return false;
        }
        let grid = CentroidGrid::new(ys.iter().copied(), tolerance);
        let mut used = vec![false; ys.len()];
        xs.into_iter().all(|m| {
            let Some(i) = grid
                .near(m)
                .find(|&i| !used[i] && m.approx_eq(ys[i], tolerance))
            else {
                return false;
            };
            used[i] = true;
            true
        })
    })
}

/// Points bucketed in cubic cells four tolerances wide.
pub(crate) struct CentroidGrid {
    cell: f64,
    cells: BTreeMap<[i64; 3], Vec<usize>>,
}

impl CentroidGrid {
    pub(crate) fn new(points: impl Iterator<Item = Point3>, tolerance: f64) -> Self {
        let cell = 4.0 * tolerance.max(tol::POINT_MERGE);
        let mut cells: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
        for (i, p) in points.enumerate() {
            cells.entry(Self::key(cell, p)).or_default().push(i);
        }
        Self { cell, cells }
    }

    fn key(cell: f64, p: Point3) -> [i64; 3] {
        [
            (p.x / cell).floor() as i64,
            (p.y / cell).floor() as i64,
            (p.z / cell).floor() as i64,
        ]
    }

    /// Indices in the cell of `p` and its 26 neighbors, in index order.
    pub(crate) fn near(&self, p: Point3) -> impl Iterator<Item = usize> + '_ {
        let k = Self::key(self.cell, p);
        let mut out: Vec<usize> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = self.cells.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) {
                        out.extend(v);
                    }
                }
            }
        }
        out.sort_unstable();
        out.into_iter()
    }
}

impl Object {
    /// Whether a blend face borders `face` (across one of its edges).
    pub fn face_borders_blend(&self, face: FaceId) -> bool {
        self.faces().get(face).is_some()
            && self.face_loops(face).into_iter().any(|l| {
                self.loop_half_edges(l).any(|h| {
                    self.half_edges()[h].twin.is_some_and(|t| {
                        let g = self.loops()[self.half_edges()[t].loop_id].face;
                        self.blend_of_face(g).is_some()
                    })
                })
            })
    }

    /// The blend id of a tagged face.
    pub fn blend_of_face(&self, face: FaceId) -> Option<BlendId> {
        self.faces().get(face).and_then(|f| f.blend).map(|t| t.id)
    }

    /// Whether `edge` is a seam inside one blend: both of its faces carry
    /// the same blend (two strips meeting at a mitered corner, or over a
    /// sharp edge the blend did not round). Pure; display code draws these
    /// with the tangent edges rather than as hard lines.
    pub fn edge_is_inside_one_blend(&self, edge: EdgeId) -> bool {
        self.edge_faces(edge).is_some_and(|(a, b)| {
            let id = self.blend_of_face(a);
            id.is_some() && id == self.blend_of_face(b)
        })
    }

    /// Every blend on the object, ascending id, each verified (pure; its
    /// cost grows with the blend count, so callers cache it).
    pub fn blend_features(&self) -> Vec<BlendFeature> {
        let mut by_id: BTreeMap<BlendId, (BlendKind, Vec<FaceId>)> = BTreeMap::new();
        for (fid, f) in self.faces() {
            if let Some(t) = f.blend {
                by_id
                    .entry(t.id)
                    .or_insert((t.kind, Vec::new()))
                    .1
                    .push(fid);
            }
        }
        by_id
            .into_iter()
            .map(|(id, (kind, faces))| {
                // Intact means the whole edit works: the restoration and,
                // with later blends taken off ahead of it, putting them back.
                let restored = self.restore_blend(id).and_then(|r| {
                    if r.dependents.is_empty() {
                        Ok(r)
                    } else {
                        r.object.rewind_blends(&r.dependents).map(|_| r)
                    }
                });
                let size = match &restored {
                    Ok(r) => Some(r.size),
                    Err(_) => faces.iter().find_map(|&f| match self.faces()[f].surface {
                        Some(SurfaceRef::Cylinder { radius, .. }) => Some(radius),
                        Some(SurfaceRef::Torus { minor_radius, .. }) => Some(minor_radius),
                        Some(SurfaceRef::Sphere { radius, .. }) => Some(radius),
                        _ => None,
                    }),
                };
                BlendFeature {
                    id,
                    kind,
                    faces,
                    size,
                    intact: restored.is_ok(),
                }
            })
            .collect()
    }

    /// Restores the sharp geometry under blend `id` (WP08 §2): by its
    /// record when it has one (taking later blends in the way off ahead of
    /// it, see [`Object::unwind_blend`]), else by derivation verified by
    /// re-blending the restored edges with the derived profile. A record
    /// that does not fit falls back to the derivation.
    ///
    /// # Errors
    /// `UnknownBlend` for an id no face carries; `BlendModified` when the
    /// restoration is undefined or does not reproduce the blend.
    pub(crate) fn restore_blend(&self, id: BlendId) -> Result<Restored, BlendError> {
        if self.has_blend_record(id) {
            match self.unwind_blend(id) {
                Ok(u) => {
                    let region: BTreeSet<FaceId> = self
                        .faces()
                        .iter()
                        .filter(|(_, f)| f.blend.is_some_and(|t| t.id == id))
                        .map(|(fid, _)| fid)
                        .collect();
                    let size = match &u.spec.profile {
                        BlendProfile::Bullnose { .. } => self
                            .bullnose_radius(&region)
                            .ok_or(BlendError::BlendModified)?,
                        p => p.size().ok_or(BlendError::BlendModified)?,
                    };
                    return Ok(Restored {
                        edges: u.spec.edges.clone(),
                        spec: u.spec,
                        face_map: u.face_map,
                        object: u.object,
                        size,
                        dependents: u.dependents,
                    });
                }
                Err(BlendError::UnknownBlend) => return Err(BlendError::UnknownBlend),
                Err(_) => {}
            }
        }
        self.restore_blend_derived(id)
    }

    /// [`Object::restore_blend`] by derivation alone.
    fn restore_blend_derived(&self, id: BlendId) -> Result<Restored, BlendError> {
        let modified = BlendError::BlendModified;
        let region: BTreeSet<FaceId> = self
            .faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| t.id == id))
            .map(|(fid, _)| fid)
            .collect();
        if region.is_empty() {
            return Err(BlendError::UnknownBlend);
        }
        let kind = self.faces()[*region.iter().next().expect("non-empty")]
            .blend
            .expect("tagged")
            .kind;
        let plan = super::restore::restoration(self, &region)?;
        let applied = self.apply_patch(&plan.patch).map_err(|_| modified)?;
        let origin: BTreeMap<FaceId, FaceId> = applied
            .faces
            .iter()
            .copied()
            .zip(plan.neighbours.iter().copied())
            .collect();
        let restored = applied.object;
        // The restored sharp edges: both ends restored corners.
        let restored_positions: Vec<Point3> = plan
            .restored_pts
            .iter()
            .filter_map(|&i| match plan.patch.verts[i] {
                PVert::New(p) => Some(p),
                PVert::Existing(_) => None,
            })
            .collect();
        let is_restored = |p: Point3| {
            restored_positions
                .iter()
                .any(|q| q.approx_eq(p, tol::POINT_MERGE))
        };
        let sides: Option<[FaceId; 2]> = plan.sides.and_then(|(a, b)| {
            let back = |f: FaceId| origin.iter().find(|(_, o)| **o == f).map(|(n, _)| *n);
            Some([back(a)?, back(b)?])
        });
        let must_touch: Option<FaceId> = match plan.touching {
            Some(super::restore::Key::Face(f)) => {
                origin.iter().find(|(_, o)| **o == f).map(|(n, _)| *n)
            }
            Some(super::restore::Key::Virt(i)) => {
                applied.faces.get(plan.neighbours.len() + i).copied()
            }
            None => None,
        };
        let continued = |e: EdgeId| {
            restored.edge_faces(e).is_some_and(|(x, y)| {
                let (Some(&ox), Some(&oy)) = (origin.get(&x), origin.get(&y)) else {
                    return false;
                };
                plan.continued.contains(&(ox.min(oy), ox.max(oy)))
            })
        };
        // A restored edge has strip facets running along it, against one of
        // its faces (an unpicked edge between two restored corners has not).
        let strips: Vec<FaceId> = region
            .iter()
            .copied()
            .filter(|&f| {
                self.faces()[f]
                    .blend
                    .is_some_and(|t| t.role == BlendRole::Strip)
            })
            .collect();
        let along_strip = |e: EdgeId| -> bool {
            let (Some((p, q)), Some((x, y))) = (restored.edge_endpoints(e), restored.edge_faces(e))
            else {
                return false;
            };
            let Ok(d) = (q - p).normalized() else {
                return false;
            };
            // A rail: a strip facet's edge against `side` parallel to the
            // candidate — exactly for flat strips and rims (homothetic
            // chords), within a taper's half-angle on a cone; the arc chords
            // where a strip meets an end face never are.
            let tapered = strips
                .iter()
                .any(|&f| matches!(self.faces()[f].surface, Some(SurfaceRef::Cone { .. })));
            let parallel = if tapered {
                tol::BLEND_RAIL_PARALLEL_TAPERED
            } else {
                1.0 - tol::BLEND_RAIL_PARALLEL_DEFECT
            };
            let rail_on = |side: FaceId| {
                strips.iter().any(|&s| {
                    self.loop_half_edges(self.faces()[s].outer_loop).any(|h| {
                        let he = self.half_edges()[h];
                        let a = self.vertices()[he.origin].position;
                        let b = self.vertices()[self.half_edges()[he.next].origin].position;
                        he.twin.is_some_and(|t| {
                            self.loops()[self.half_edges()[t].loop_id].face == side
                        }) && (b - a)
                            .normalized()
                            .is_ok_and(|u| u.dot(d).abs() >= parallel)
                    })
                })
            };
            [x, y]
                .iter()
                .all(|f| origin.get(f).is_some_and(|&o| rail_on(o)))
        };
        let edges: Vec<EdgeId> = restored
            .edges()
            .keys()
            .filter(|&e| {
                (plan.consumed != super::restore::Consumed::None || along_strip(e)) &&
                (continued(e)
                    || restored
                        .edge_endpoints(e)
                        .is_some_and(|(a, b)| is_restored(a) && is_restored(b)))
                    && restored.edge_convexity(e).is_some()
                    // A bullnose's edges lie against its two sides.
                    && must_touch.is_none_or(|m| {
                        restored.edge_faces(e).is_some_and(|(x, y)| x == m || y == m)
                    })
                    && sides.is_none_or(|ss| {
                        restored
                            .edge_faces(e)
                            .is_some_and(|(x, y)| ss.contains(&x) || ss.contains(&y))
                    })
            })
            .collect();
        if edges.is_empty() {
            return Err(modified);
        }
        let profile =
            self.measured_profile(&restored, &origin, &region, kind, &edges, plan.consumed)?;

        // Verification: re-blending reproduces the blend. A stopped or
        // tapered fillet's stops and end radii are read back too.
        let mut candidates: Vec<BlendSpec> = Vec::new();
        let base = BlendSpec {
            edges: edges.clone(),
            profile: profile.clone(),
            primary: None,
            corners: CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        };
        let ends = chain_ends(&restored, &edges);
        let has_runout = region.iter().any(|&f| {
            self.faces()[f]
                .blend
                .is_some_and(|t| t.role == BlendRole::Runout)
        });
        let tapered = region
            .iter()
            .any(|&f| matches!(self.faces()[f].surface, Some(SurfaceRef::Cone { .. })))
            && kind == BlendKind::Fillet;
        if let (true, Some((a, b, _))) = (has_runout, ends) {
            let (da, db) = (self.stop_at(&region, a, b), self.stop_at(&region, b, a));
            for (start, end) in [(da, db), (db, da)] {
                candidates.push(BlendSpec {
                    stops: Some(super::BlendStops { start, end }),
                    ..base.clone()
                });
            }
        } else if let (true, Some((a, b, len))) = (tapered, ends) {
            let theta = restored.edge_dihedral(edges[0]).ok_or(modified)?;
            let (ra, rb) = (
                self.end_radius(&region, a, b, theta).ok_or(modified)?,
                self.end_radius(&region, b, a, theta).ok_or(modified)?,
            );
            for (r0, r1) in [(ra, rb), (rb, ra)] {
                candidates.push(BlendSpec {
                    profile: match &profile {
                        BlendProfile::Round { segments, .. } => BlendProfile::Round {
                            radius: r0,
                            segments: *segments,
                        },
                        p => p.clone(),
                    },
                    handles: vec![
                        super::RadiusHandle {
                            at: 0.0,
                            radius: r0,
                        },
                        super::RadiusHandle {
                            at: len,
                            radius: r1,
                        },
                    ],
                    ..base.clone()
                });
            }
        } else {
            candidates.push(base);
        }
        let verified = candidates.into_iter().find(|spec| {
            restored
                .blend_edges_plain(spec)
                .is_ok_and(|(again, _)| same_geometry(self, &again, tol::BLEND_VERIFY))
        });
        let Some(spec) = verified else {
            return Err(modified);
        };
        let profile = spec.profile.clone();
        let size = match &profile {
            BlendProfile::Bullnose { .. } => self.bullnose_radius(&region).ok_or(modified)?,
            p => p.size().ok_or(modified)?,
        };
        let face_map: BTreeMap<FaceId, FaceId> = origin.iter().map(|(n, o)| (*o, *n)).collect();
        Ok(Restored {
            spec: spec.clone(),
            face_map,
            object: restored,
            edges,
            size,
            dependents: Vec::new(),
        })
    }

    /// A stopped end's distance (WP07 §5.1) at chain end `a` (the other end
    /// `b`): from `a` to the station where the strip starts, read from the
    /// run-out fan whose apex lies on the edge nearest `a`; 0 without one.
    fn stop_at(&self, region: &BTreeSet<FaceId>, a: Point3, b: Point3) -> f64 {
        let Ok(d) = (b - a).normalized() else {
            return 0.0;
        };
        let on_line = |p: Point3| ((p - a) - d * (p - a).dot(d)).length() <= tol::BLEND_VERIFY;
        region
            .iter()
            .filter(|&&f| {
                self.faces()[f]
                    .blend
                    .is_some_and(|t| t.role == BlendRole::Runout)
            })
            .filter_map(|&f| {
                let ring: Vec<Point3> = self.loop_positions(self.faces()[f].outer_loop).collect();
                let apex = ring.iter().copied().find(|&p| on_line(p))?;
                let other = ring.iter().copied().find(|&p| !on_line(p))?;
                Some(((apex - a).dot(d), (other - a).dot(d)))
            })
            .filter(|&(t_apex, _)| t_apex < (b - a).length() / 2.0)
            .map(|(_, t_station)| t_station)
            .next()
            .unwrap_or(0.0)
    }

    /// A taper's radius at chain end `a` (the other end `b`): the rail on
    /// one face at that end, `r = d·tan(θ/2)` from its setback `d`.
    fn end_radius(
        &self,
        region: &BTreeSet<FaceId>,
        a: Point3,
        b: Point3,
        theta: f64,
    ) -> Option<f64> {
        let d = (b - a).normalized().ok()?;
        // The strip vertices nearest `a` along the edge: the end station.
        let mut best: Option<(f64, f64)> = None;
        for &f in region {
            for p in self.loop_positions(self.faces()[f].outer_loop) {
                let t = (p - a).dot(d);
                let off = ((p - a) - d * t).length();
                if off <= tol::BLEND_VERIFY {
                    continue;
                }
                // At the end station, the rails lie farthest from the edge.
                if best.is_none_or(|(bt, bo)| {
                    t < bt - tol::BLEND_VERIFY || (t <= bt + tol::BLEND_VERIFY && off > bo)
                }) {
                    best = Some((t, off));
                }
            }
        }
        best.map(|(_, off)| off * (theta / 2.0).tan())
    }

    /// A bullnose strip's radius.
    fn bullnose_radius(&self, region: &BTreeSet<FaceId>) -> Option<f64> {
        region.iter().find_map(|&f| match self.faces()[f].surface {
            Some(SurfaceRef::Cylinder { radius, .. }) => Some(radius),
            _ => None,
        })
    }

    /// The profile that re-creates blend `region` on the restored `edges`
    /// (WP08 §2.4): the radius of a round or cove from the strip claims, a
    /// chamfer's setbacks from its rail points.
    fn measured_profile(
        &self,
        restored: &Object,
        origin: &BTreeMap<FaceId, FaceId>,
        region: &BTreeSet<FaceId>,
        kind: BlendKind,
        edges: &[EdgeId],
        consumed: super::restore::Consumed,
    ) -> Result<BlendProfile, BlendError> {
        let modified = BlendError::BlendModified;
        let strips: Vec<FaceId> = region
            .iter()
            .copied()
            .filter(|&f| {
                self.faces()[f]
                    .blend
                    .is_some_and(|t| t.role == BlendRole::Strip)
            })
            .collect();
        // Facets across: strip faces touching one rail face (side A of the
        // first restored edge) count the segments.
        let e0 = edges[0];
        let (fa, fb) = restored.edge_faces(e0).ok_or(modified)?;
        let touches = |f: FaceId, g: FaceId| {
            self.face_loops(f).into_iter().any(|l| {
                self.loop_half_edges(l).any(|h| {
                    self.half_edges()[h]
                        .twin
                        .is_some_and(|t| self.loops()[self.half_edges()[t].loop_id].face == g)
                })
            })
        };
        // A side the restoration re-created has no face in `self`: walk from
        // the other side to the facets' dead end (a hemisphere's pole).
        let (fa, fb) = match (origin.get(&fa), origin.get(&fb)) {
            (None, Some(_)) => (fb, fa),
            _ => (fa, fb),
        };
        let fa_orig = *origin.get(&fa).ok_or(modified)?;
        let fb_orig = origin.get(&fb).copied();
        let (p0, p1) = restored.edge_endpoints(e0).ok_or(modified)?;
        let d = (p1 - p0).normalized().map_err(|_| modified)?;
        let mid = p0 + (p1 - p0) * 0.5;
        // Facets across one section: from the strip facet on face A nearest
        // the restored edge, across rulings along it, to face B.
        let centroid = |f: FaceId| {
            let ps: Vec<Point3> = self.loop_positions(self.faces()[f].outer_loop).collect();
            let c = ps
                .iter()
                .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
                / ps.len() as f64;
            Point3::new(c.x, c.y, c.z)
        };
        let mut cur = strips
            .iter()
            .copied()
            .filter(|&s| touches(s, fa_orig))
            .min_by(|&x, &y| {
                (centroid(x) - mid)
                    .length()
                    .total_cmp(&(centroid(y) - mid).length())
            })
            .ok_or(modified)?;
        let mut n = 1usize;
        let mut seen = BTreeSet::from([cur]);
        let mut walk = vec![cur];
        while fb_orig.is_none_or(|g| !touches(cur, g)) {
            let next = self.face_loops(cur).into_iter().find_map(|l| {
                self.loop_half_edges(l).find_map(|h| {
                    let he = self.half_edges()[h];
                    let g = self.loops()[self.half_edges()[he.twin?].loop_id].face;
                    let a = self.vertices()[he.origin].position;
                    let b = self.vertices()[self.half_edges()[he.next].origin].position;
                    // Rulings run along the restored edge (exactly for flat
                    // strips and rims, within a taper's half-angle for a
                    // cone); a miter edge runs well off it.
                    let along = (b - a).normalized().ok()?.dot(d).abs() >= 0.99;
                    (along && strips.contains(&g) && !seen.contains(&g)).then_some(g)
                })
            });
            let Some(g) = next else {
                if fb_orig.is_none() {
                    break;
                }
                return Err(modified);
            };
            seen.insert(g);
            walk.push(g);
            cur = g;
            n += 1;
        }
        let line_dist = |p: Point3| ((p - p0) - d * (p - p0).dot(d)).length();
        match kind {
            BlendKind::Chamfer => {
                // Rail points: the vertices of this segment's chamfer facet on
                // faces A and B.
                let rail = |side: FaceId| -> Option<f64> {
                    walk.iter().find_map(|&s| {
                        self.loop_positions(self.faces()[s].outer_loop)
                            .filter(|&p| {
                                restored.faces()[side].plane.signed_distance(p).abs()
                                    <= tol::SWEPT_PLANE_SLACK
                            })
                            .map(line_dist)
                            .min_by(f64::total_cmp)
                    })
                };
                let (a, b) = (rail(fa).ok_or(modified)?, rail(fb).ok_or(modified)?);
                Ok(BlendProfile::Chamfer {
                    setback_a: a,
                    setback_b: b,
                })
            }
            BlendKind::Fillet => {
                let claim = strips
                    .iter()
                    .find_map(|&s| self.faces()[s].surface)
                    .ok_or(modified)?;
                let (r, axis_dist) = match claim {
                    SurfaceRef::Cylinder {
                        axis_point, radius, ..
                    } => (radius, line_dist(axis_point)),
                    SurfaceRef::Torus { minor_radius, .. } => (minor_radius, f64::INFINITY),
                    SurfaceRef::Sphere { radius, .. } => (radius, f64::INFINITY),
                    // A linear taper: its end radii are read back with the
                    // verification's handles.
                    SurfaceRef::Cone { .. } => (0.0, f64::INFINITY),
                };
                let segments = u32::try_from(n).map_err(|_| modified)?;
                if consumed == super::restore::Consumed::Bullnose {
                    return Ok(BlendProfile::Bullnose { segments });
                }
                // A cove's arc is centered on the edge itself.
                if axis_dist <= tol::BLEND_VERIFY {
                    Ok(BlendProfile::Cove {
                        radius: r,
                        segments,
                    })
                } else {
                    Ok(BlendProfile::Round {
                        radius: r,
                        segments,
                    })
                }
            }
            BlendKind::Profile => {
                let frame = restored.edge_frame(e0, fa).ok_or(modified)?;
                self.measured_custom(
                    &walk,
                    fa_orig,
                    fb_orig.ok_or(modified)?,
                    p0,
                    d,
                    frame.t_a,
                    frame.t_b,
                )
            }
        }
    }

    /// A router profile read back from one section of its strip (WP08
    /// §2.4): the ruling points across the facets `walk` (face A to face B)
    /// in the restored edge's frame, normalized by the width; facets on one
    /// claimed arc become one arc segment, and the arc subdivision is the
    /// one that reproduces the facet count (straight lines otherwise).
    #[allow(clippy::too_many_arguments)]
    fn measured_custom(
        &self,
        walk: &[FaceId],
        fa: FaceId,
        fb: FaceId,
        p0: Point3,
        d: Vec3,
        ta: Vec3,
        tb: Vec3,
    ) -> Result<BlendProfile, BlendError> {
        let modified = BlendError::BlendModified;
        let shared = |f: FaceId, g: FaceId| -> Option<Point3> {
            self.face_loops(f).into_iter().find_map(|l| {
                self.loop_half_edges(l).find_map(|h| {
                    let he = self.half_edges()[h];
                    let t = he.twin?;
                    (self.loops()[self.half_edges()[t].loop_id].face == g)
                        .then(|| self.vertices()[he.origin].position)
                })
            })
        };
        let mut pts3: Vec<Point3> = vec![shared(walk[0], fa).ok_or(modified)?];
        for w in walk.windows(2) {
            pts3.push(shared(w[0], w[1]).ok_or(modified)?);
        }
        pts3.push(shared(*walk.last().ok_or(modified)?, fb).ok_or(modified)?);
        let (aa, ab, bb) = (ta.dot(ta), ta.dot(tb), tb.dot(tb));
        let det = aa * bb - ab * ab;
        if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
            return Err(modified);
        }
        let uv = |p: Point3| -> [f64; 2] {
            let o = (p - p0) - d * (p - p0).dot(d);
            let (x, y) = (o.dot(ta), o.dot(tb));
            [(x * bb - y * ab) / det, (aa * y - ab * x) / det]
        };
        let raw: Vec<[f64; 2]> = pts3.iter().map(|&p| uv(p)).collect();
        let width = raw[0][0];
        if width <= tol::POINT_MERGE {
            return Err(modified);
        }
        let norm: Vec<[f64; 2]> = raw.iter().map(|p| [p[0] / width, p[1] / width]).collect();
        let n = norm.len() - 1;
        let b = norm[n][1];
        // Arc hints per facet from claims.
        let arc_of = |f: FaceId| -> Option<([f64; 2], SurfaceRef)> {
            match self.faces()[f].surface {
                Some(c @ SurfaceRef::Cylinder { axis_point, .. }) => {
                    let q = uv(axis_point);
                    Some(([q[0] / width, q[1] / width], c))
                }
                _ => None,
            }
        };
        let cross = |o: [f64; 2], a: [f64; 2], c: [f64; 2]| {
            (a[0] - o[0]) * (c[1] - o[1]) - (a[1] - o[1]) * (c[0] - o[0])
        };
        let mut segs: Vec<super::ProfileSeg> = Vec::new();
        let mut k = 0;
        while k < n {
            match arc_of(walk[k]) {
                Some((center, claim)) => {
                    let mut j = k + 1;
                    while j < n && arc_of(walk[j]).is_some_and(|(_, c)| c.same_surface(&claim)) {
                        j += 1;
                    }
                    segs.push(super::ProfileSeg::Arc {
                        to: norm[j],
                        center,
                        ccw: cross(center, norm[k], norm[k + 1]) > 0.0,
                    });
                    k = j;
                }
                None => {
                    segs.push(super::ProfileSeg::Line { to: norm[k + 1] });
                    k += 1;
                }
            }
        }
        let shape = super::ProfileShape { b, segs };
        if let Some(arc_segments) =
            (1..=96u32).find(|&a| shape.sample(a).is_ok_and(|p| p.len() == n + 1))
        {
            return Ok(BlendProfile::Custom {
                shape,
                width,
                arc_segments,
            });
        }
        let lines = super::ProfileShape {
            b,
            segs: norm[1..]
                .iter()
                .map(|&to| super::ProfileSeg::Line { to })
                .collect(),
        };
        Ok(BlendProfile::Custom {
            shape: lines,
            width,
            arc_segments: 1,
        })
    }

    /// Test hook for [`Object::remove_blend`].
    #[doc(hidden)]
    pub fn remove_blend_for_test(&self, id: BlendId) -> Result<(Object, Vec<EdgeId>), BlendError> {
        self.remove_blend(id)
    }

    /// Removes blend `id`, restoring its sharp edges; later blends that ran
    /// into it are put back on the restored geometry.
    pub(crate) fn remove_blend(&self, id: BlendId) -> Result<(Object, Vec<EdgeId>), BlendError> {
        let r = self.restore_blend(id)?;
        let object = r.object.rewind_blends(&r.dependents)?;
        let edges: Vec<EdgeId> = r
            .edges
            .into_iter()
            .filter(|e| object.edges().contains_key(*e))
            .collect();
        Ok((object, edges))
    }

    /// Test hook for [`Object::resize_blend`].
    #[doc(hidden)]
    pub fn resize_blend_for_test(
        &self,
        id: BlendId,
        profile: &BlendProfile,
    ) -> Result<(Object, BlendReport), BlendError> {
        self.resize_blend(id, profile)
    }

    /// Resizes (or changes the profile of) blend `id`: restore, then blend
    /// the restored edges with `profile` as a fresh fillet would (the corner
    /// re-solve included); later blends that ran into it are put back on
    /// the result.
    pub(crate) fn resize_blend(
        &self,
        id: BlendId,
        profile: &BlendProfile,
    ) -> Result<(Object, BlendReport), BlendError> {
        let r = self.restore_blend(id)?;
        r.reblend(id, &r.resized_spec(profile))
    }

    /// Puts `dependents` back on `self` and re-reads `report`'s new faces
    /// from the tags (a put-back blend may trim them).
    pub(crate) fn rewind_report(
        &self,
        mut report: BlendReport,
        dependents: &[super::record::Dependent],
    ) -> Result<(Object, BlendReport), BlendError> {
        if dependents.is_empty() {
            return Ok((self.clone(), report));
        }
        let object = self.rewind_blends(dependents)?;
        report.new_faces = object
            .faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| report.blends.contains(&t.id)))
            .map(|(id, _)| id)
            .collect();
        report
            .edge_map
            .retain(|_, e| object.edges().contains_key(*e));
        Ok((object, report))
    }
}

/// The two ends of a straight chain of `edges` and its length, or `None`
/// when they do not form one open chain.
fn chain_ends(obj: &Object, edges: &[EdgeId]) -> Option<(Point3, Point3, f64)> {
    let mut count: Vec<(Point3, usize)> = Vec::new();
    let mut len = 0.0;
    for &e in edges {
        let (a, b) = obj.edge_endpoints(e)?;
        len += (b - a).length();
        for p in [a, b] {
            match count
                .iter_mut()
                .find(|(q, _)| q.approx_eq(p, tol::POINT_MERGE))
            {
                Some(c) => c.1 += 1,
                None => count.push((p, 1)),
            }
        }
    }
    let ends: Vec<Point3> = count
        .iter()
        .filter(|(_, n)| *n == 1)
        .map(|(p, _)| *p)
        .collect();
    match ends.as_slice() {
        [a, b] => Some((*a, *b, len)),
        _ => None,
    }
}

impl Object {
    /// Push/Pull on a face of a blend (WP08 §5.1): moves the blend surface
    /// `distance` along that facet's outward normal, resizing the blend;
    /// pulled to or past the sharp edge, it removes it.
    ///
    /// # Errors
    /// `UnknownBlend` for an untagged face; `BlendModified`; a bullnose
    /// refuses `UnsupportedEdge` (its size is fixed by its face); then
    /// whatever re-blending refuses.
    pub(crate) fn push_pull_blend(
        &self,
        face: FaceId,
        distance: f64,
    ) -> Result<Object, BlendError> {
        let id = self.blend_of_face(face).ok_or(BlendError::UnknownBlend)?;
        let r = self.restore_blend(id)?;
        let e0 = r.spec.edges[0];
        let theta = r
            .object
            .edge_dihedral(e0)
            .ok_or(BlendError::BlendModified)?;
        let sigma = match r.object.edge_convexity(e0) {
            Some(super::Convexity::Convex) => -1.0,
            _ => 1.0,
        };
        let pm = tol::POINT_MERGE;
        let reblend = |profile: BlendProfile| -> Result<Object, BlendError> {
            r.reblend(
                id,
                &BlendSpec {
                    profile,
                    ..r.spec.clone()
                },
            )
            .map(|(o, _)| o)
        };
        let gap = 1.0 / (theta / 2.0).sin() - 1.0;
        match r.spec.profile.clone() {
            BlendProfile::Round { radius, segments } => {
                let m = radius * gap + sigma * distance;
                if m <= pm {
                    return r.object.rewind_blends(&r.dependents);
                }
                reblend(BlendProfile::Round {
                    radius: m / gap,
                    segments,
                })
            }
            BlendProfile::Cove { radius, segments } => {
                let rr = radius - distance;
                if rr <= pm {
                    return r.object.rewind_blends(&r.dependents);
                }
                reblend(BlendProfile::Cove {
                    radius: rr,
                    segments,
                })
            }
            BlendProfile::Chamfer {
                setback_a,
                setback_b,
            } => {
                // The chamfer line's distance from the corner.
                let chord = (setback_a * setback_a + setback_b * setback_b
                    - 2.0 * setback_a * setback_b * theta.cos())
                .sqrt();
                let h = setback_a * setback_b * theta.sin() / chord;
                let k = (h + sigma * distance) / h;
                if k <= pm {
                    return r.object.rewind_blends(&r.dependents);
                }
                reblend(BlendProfile::Chamfer {
                    setback_a: setback_a * k,
                    setback_b: setback_b * k,
                })
            }
            BlendProfile::Custom {
                shape,
                width,
                arc_segments,
            } => {
                // The section's farthest point from the corner, in the
                // frame where (u, v) map to u·W·tA + v·W·tB.
                let pts = shape.sample(arc_segments)?;
                let h = pts
                    .iter()
                    .map(|(p, _)| {
                        let (x, y) = (p[0] + p[1] * theta.cos(), p[1] * theta.sin());
                        x.hypot(y) * width
                    })
                    .fold(0.0, f64::max);
                let k = (h + sigma * distance) / h;
                if k <= pm {
                    return r.object.rewind_blends(&r.dependents);
                }
                reblend(BlendProfile::Custom {
                    shape,
                    width: width * k,
                    arc_segments,
                })
            }
            BlendProfile::Bullnose { .. } => Err(BlendError::UnsupportedEdge),
        }
    }

    /// Blend-aware Push/Pull of a face beside blends (WP08 §5.2): restore
    /// those blends, push/pull the face, and re-apply each on the same face
    /// pairs. Any failure is an error and the caller falls back to the
    /// ordinary Push/Pull.
    ///
    /// # Errors
    /// `UnknownBlend` when no blend is beside `face`; any restoration,
    /// Push/Pull or re-blend failure.
    pub(crate) fn push_pull_with_blends(
        &self,
        face: FaceId,
        distance: f64,
    ) -> Result<Object, BlendError> {
        // Blends with a face across an edge of `face`.
        let mut ids: BTreeSet<BlendId> = BTreeSet::new();
        for l in self.face_loops(face) {
            for h in self.loop_half_edges(l) {
                if let Some(t) = self.half_edges()[h].twin {
                    let g = self.loops()[self.half_edges()[t].loop_id].face;
                    if let Some(id) = self.blend_of_face(g) {
                        ids.insert(id);
                    }
                }
            }
        }
        if ids.is_empty() || self.blend_of_face(face).is_some() {
            return Err(BlendError::UnknownBlend);
        }
        // Restore each, keeping its edges as face pairs through every later
        // restoration's re-added ids.
        let mut cur = self.clone();
        let mut target = face;
        // (face pairs of its edges, spec, id) per blend taken off.
        type Kept = (Vec<(FaceId, FaceId)>, BlendSpec, BlendId);
        let mut kept: Vec<Kept> = Vec::new();
        // Later blends taken off ahead of a restored one go back last, by
        // their segments (a replayed record keeps face ids, so the pairs
        // of the kept blends need no remapping for them).
        let mut dependents: Vec<super::record::Dependent> = Vec::new();
        for id in ids {
            if !cur
                .faces()
                .values()
                .any(|f| f.blend.is_some_and(|t| t.id == id))
            {
                // Already off, ahead of an earlier blend of this face.
                continue;
            }
            let r = cur.restore_blend(id)?;
            let remap = |f: FaceId| r.face_map.get(&f).copied().unwrap_or(f);
            for (pairs, _, _) in &mut kept {
                for p in pairs.iter_mut() {
                    *p = (remap(p.0), remap(p.1));
                }
            }
            target = remap(target);
            let pairs: Vec<(FaceId, FaceId)> = r
                .spec
                .edges
                .iter()
                .filter_map(|&e| r.object.edge_faces(e))
                .collect();
            kept.push((pairs, r.spec.clone(), id));
            dependents.extend(r.dependents);
            cur = r.object;
        }
        cur.push_pull(target, distance)
            .map_err(|_| BlendError::BlendModified)?;
        // Every blend goes back under its own id.
        let _reserved = super::ReservedBlendIds::new(
            kept.iter()
                .map(|(_, _, id)| *id)
                .chain(dependents.iter().map(|d| d.id)),
        );
        for (pairs, spec, id) in kept {
            let edges: Vec<EdgeId> = cur
                .edges()
                .keys()
                .filter(|&e| {
                    cur.edge_faces(e).is_some_and(|(x, y)| {
                        pairs
                            .iter()
                            .any(|&(a, b)| (a == x && b == y) || (a == y && b == x))
                    })
                })
                .collect();
            if edges.is_empty() {
                return Err(BlendError::BlendModified);
            }
            cur = super::with_forced_blend_id(Some(id), || {
                cur.blend_edges_plain(&BlendSpec { edges, ..spec })
            })?
            .0;
        }
        dependents.sort_by_key(|d| d.seq);
        dependents.dedup_by_key(|d| d.id);
        cur.rewind_blends(&dependents)
    }
}

/// An edge as its two end positions, to follow it through restorations.
pub(crate) type Segment = (Point3, Point3);

/// The edges of `obj` lying along `seg` and overlapping it.
pub(crate) fn edges_on(obj: &Object, seg: Segment) -> Vec<EdgeId> {
    let (a, b) = seg;
    let Ok(d) = (b - a).normalized() else {
        return Vec::new();
    };
    let len = (b - a).length();
    let on_line = |p: Point3| ((p - a) - d * (p - a).dot(d)).length() <= tol::BLEND_VERIFY;
    obj.edges()
        .keys()
        .filter(|&e| {
            obj.edge_endpoints(e).is_some_and(|(p, q)| {
                if !(on_line(p) && on_line(q)) {
                    return false;
                }
                let (tp, tq) = ((p - a).dot(d), (q - a).dot(d));
                tp.max(tq) > tol::POINT_MERGE && tp.min(tq) < len - tol::POINT_MERGE
            })
        })
        .collect()
}

/// (segments, spec, id, may merge): one blend of a corner re-solve, its
/// edges as segments to follow them through the passes before it. The
/// blends the new edges run into may merge with it at equal size (that is
/// the corner); later blends taken off ahead of one of them do not run into
/// the new edges and stay features of their own.
type Group = (Vec<Segment>, BlendSpec, Option<BlendId>, bool);

/// A corner re-solve (WP08 §4) made ready: the blends the spec's edges run
/// into restored, and the blends to put back. The restorations do not
/// depend on the new blend's size, so a limit search prepares once and
/// runs the passes at each size it tries.
pub(crate) struct CornerResolve {
    /// The object with those blends restored.
    restored: Object,
    /// The new blend first, then the restored blends.
    groups: Vec<Group>,
}

impl CornerResolve {
    /// `None` when nothing needs re-solving or a restoration fails (the
    /// caller then plans normally).
    pub(crate) fn prepare(obj: &Object, spec: &BlendSpec) -> Option<CornerResolve> {
        let ve = obj.vertex_edges();
        let mut ids: BTreeSet<BlendId> = BTreeSet::new();
        for &e in &spec.edges {
            let (a, b) = obj.edge_vertex_ids(e)?;
            for v in [a, b] {
                for &x in ve.get(&v).into_iter().flatten() {
                    if let Some((f, g)) = obj.edge_faces(x) {
                        ids.extend(obj.blend_of_face(f));
                        ids.extend(obj.blend_of_face(g));
                    }
                }
            }
        }
        if ids.is_empty() {
            return None;
        }
        // The new edges as segments. Each group keeps the id its blend had
        // (the new spec's: the id forced on it, if any), and later blends
        // taken off ahead of a restored one join as groups of their own.
        let seg = |o: &Object, e: EdgeId| o.edge_endpoints(e);
        let mut groups: Vec<Group> = vec![(
            spec.edges.iter().filter_map(|&e| seg(obj, e)).collect(),
            spec.clone(),
            super::forced_blend_id(),
            true,
        )];
        let mut cur = obj.clone();
        for id in ids {
            if !cur
                .faces()
                .values()
                .any(|f| f.blend.is_some_and(|t| t.id == id))
            {
                // Already off, as a later blend of one restored before it.
                continue;
            }
            let r = cur.restore_blend(id).ok()?;
            groups.push((
                r.spec
                    .edges
                    .iter()
                    .filter_map(|&e| seg(&r.object, e))
                    .collect(),
                r.spec.clone(),
                Some(id),
                true,
            ));
            for d in &r.dependents {
                let located = d.spec.locate(&r.object).ok()?;
                groups.push((d.spec.edges.clone(), located, Some(d.id), false));
            }
            cur = r.object;
        }
        Some(CornerResolve {
            restored: cur,
            groups,
        })
    }

    /// The sizes of the blends the re-solve puts back (the new blend's
    /// neighbours at the corner and the blends taken off ahead of them).
    pub(crate) fn neighbour_sizes(&self) -> Vec<f64> {
        self.groups[1..]
            .iter()
            .filter_map(|(_, s, _, _)| s.profile.size())
            .collect()
    }

    /// Re-applies everything with the new blend as `spec` (the prepared
    /// spec, or the same edges at another size), each blend by `pass`, in
    /// passes of descending size. Fails with the first pass's failure; a
    /// pass whose edges a larger one took entirely fails as too large.
    pub(crate) fn run(
        &self,
        spec: &BlendSpec,
        mut pass: impl FnMut(&Object, &BlendSpec) -> Result<(Object, BlendReport), PlanFail>,
    ) -> Result<(Object, BlendReport), PlanFail> {
        let mut groups = self.groups.clone();
        groups[0].1 = spec.clone();
        // Passes by descending size; equal sizes of one profile kind merge
        // (into one blend under a fresh id).
        let size = |s: &BlendSpec| s.profile.size().unwrap_or(0.0);
        groups.sort_by(|x, y| size(&y.1).total_cmp(&size(&x.1)));
        let mut passes: Vec<Group> = Vec::new();
        for (segs, s, id, merge) in groups {
            match passes.last_mut() {
                Some((psegs, ps, pid, pmerge))
                    if merge
                        && *pmerge
                        && (size(ps) - size(&s)).abs() <= tol::POINT_MERGE
                        && std::mem::discriminant(&ps.profile)
                            == std::mem::discriminant(&s.profile)
                        && ps.stops.is_none()
                        && s.stops.is_none()
                        && ps.handles.is_empty()
                        && s.handles.is_empty() =>
                {
                    psegs.extend(segs);
                    *pid = None;
                }
                _ => passes.push((segs, s, id, merge)),
            }
        }
        let mut blends = Vec::new();
        // A merged pass takes a fresh id clear of every id a later pass
        // puts back.
        let _reserved = super::ReservedBlendIds::new(passes.iter().filter_map(|(_, _, id, _)| *id));
        let mut cur = self.restored.clone();
        for (segs, s, id, _) in passes {
            let mut edges: Vec<EdgeId> = Vec::new();
            for sg in segs {
                for e in edges_on(&cur, sg) {
                    for x in cur.logical_edge(e, true) {
                        if !edges.contains(&x) {
                            edges.push(x);
                        }
                    }
                }
            }
            if edges.is_empty() {
                return Err(PlanFail::Size(super::LimitFeature::Unknown));
            }
            let (next, report) =
                super::with_forced_blend_id(id, || pass(&cur, &BlendSpec { edges, ..s }))?;
            blends.extend(report.blends);
            cur = next;
        }
        let new_faces: Vec<FaceId> = cur
            .faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| blends.contains(&t.id)))
            .map(|(id, _)| id)
            .collect();
        Ok((
            cur,
            BlendReport {
                blends,
                new_faces,
                profile: spec.profile.clone(),
                edge_map: BTreeMap::new(),
            },
        ))
    }
}
