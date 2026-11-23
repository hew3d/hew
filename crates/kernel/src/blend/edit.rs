//! Editing blends (WP08 §1–3, decision I-10): what blends an object has,
//! restoring the sharp geometry under one, and removing or resizing it. A
//! blend made since records exist replays what it recorded
//! ([`super::record`]); one without a record re-derives the sharp corner
//! from the planes around the blend, accepted only when re-blending the
//! restored edges reproduces the blend exactly.

use crate::trig;
use std::collections::{BTreeMap, BTreeSet};

use super::plan::PlanFail;
use super::{BlendError, BlendProfile, BlendReport, BlendSpec, CornerStyle};
use crate::ids::{EdgeId, FaceId};
use crate::math::{Point3, Vec3};
use crate::patch::PVert;
use crate::tol;
use crate::topo::{BlendId, BlendKind, BlendRole, Object, SurfaceRef};

thread_local! {
    /// How many times [`Object::blend_features`] has run on this thread
    /// (test support).
    static FEATURE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times [`Object::blend_features`] has run on the calling thread:
/// a [`crate::Document::blend_features`] answered from memory does not run
/// it. Test support only.
#[doc(hidden)]
pub fn blend_feature_scans_for_test() -> usize {
    FEATURE_SCANS.with(std::cell::Cell::get)
}

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

    /// Every blend on the object, ascending id, each verified (pure; its
    /// cost grows with the blend count, so callers cache it).
    pub fn blend_features(&self) -> Vec<BlendFeature> {
        FEATURE_SCANS.with(|c| c.set(c.get() + 1));
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
        best.map(|(_, off)| off * trig::tan(theta / 2.0))
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
        let gap = 1.0 / trig::sin(theta / 2.0) - 1.0;
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
                    - 2.0 * setback_a * setback_b * trig::cos(theta))
                .sqrt();
                let h = setback_a * setback_b * trig::sin(theta) / chord;
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
                        let (x, y) = (p[0] + p[1] * trig::cos(theta), p[1] * trig::sin(theta));
                        trig::hypot(x, y) * width
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
    let Some(test) = SegmentTest::new(seg) else {
        return Vec::new();
    };
    add_locate_work(obj.edges().len());
    obj.edges()
        .keys()
        .filter(|&e| obj.edge_endpoints(e).is_some_and(|(p, q)| test.holds(p, q)))
        .collect()
}

/// Whether an edge lies along a segment and overlaps it: both ends within
/// [`tol::BLEND_VERIFY`] of the segment's line, and the stretch between
/// them reaching more than [`tol::POINT_MERGE`] past each end of the
/// segment toward the other.
struct SegmentTest {
    a: Point3,
    d: Vec3,
    len: f64,
}

impl SegmentTest {
    /// `None` for a segment too short to have a direction.
    fn new((a, b): Segment) -> Option<SegmentTest> {
        let d = (b - a).normalized().ok()?;
        Some(SegmentTest {
            a,
            d,
            len: (b - a).length(),
        })
    }

    fn holds(&self, p: Point3, q: Point3) -> bool {
        let (a, d) = (self.a, self.d);
        let on_line = |x: Point3| ((x - a) - d * (x - a).dot(d)).length() <= tol::BLEND_VERIFY;
        if !(on_line(p) && on_line(q)) {
            return false;
        }
        let (tp, tq) = ((p - a).dot(d), (q - a).dot(d));
        tp.max(tq) > tol::POINT_MERGE && tp.min(tq) < self.len - tol::POINT_MERGE
    }
}

/// The edges of one object near a set of segments, by the cells of a grid
/// they pass through, so the edges along each of many segments
/// ([`StoredSpec::locate`]) are found among the few near the segment rather
/// than by testing every edge of the object for every segment.
/// [`EdgeCells::edges_on`] answers exactly as [`edges_on`]: the same edges
/// in the same (slot) order.
///
/// The grid covers the box of the segments it is built for, grown by twice
/// `BLEND_VERIFY` (the reach) and one cell more. Each edge is cut to that
/// box and entered in the cell of each of a row of points along the piece,
/// at most a cell apart and including both ends. Why no edge is missed: an
/// edge [`SegmentTest`] accepts has both ends within `BLEND_VERIFY` of the
/// segment's line, so every point of it is (the distance to a line is
/// convex along a straight edge), and it overlaps the segment along the
/// line, so one of its points lies within `BLEND_VERIFY` of a point of the
/// segment: inside the segment's box grown by the reach, so inside the
/// piece. A sample lies within half a cell of that point, so in a cell at
/// most one step from the box's cells on each axis, and the query reads
/// those cells and one more each way. The reach is twice what the test
/// allows and the cut box a cell larger, so rounding has no say.
///
/// Everything the grid cannot answer cheaply goes to the full scan: a
/// segment whose grown box leaves the grid's box or spans too many cells,
/// and every segment once the grid would take more samples than
/// [`EdgeCells::budget`] allows (tiny segments spread over a large box: a
/// damaged record). An edge piece that would take too many samples is kept
/// in a list every query reads.
///
/// [`StoredSpec::locate`]: super::record::StoredSpec
pub(crate) struct EdgeCells<'a> {
    obj: &'a Object,
    cell: f64,
    /// The segments' box grown by the reach: queries inside it read the
    /// grid. `None` when there is no grid (no segments, or abandoned).
    inside: Option<(Point3, Point3)>,
    cells: BTreeMap<[i64; 3], Vec<EdgeId>>,
    /// Edges too long for the grid, read by every query.
    wide: Vec<EdgeId>,
    /// The grid went over its budget: every query scans.
    abandoned: bool,
}

/// The most points an edge is sampled at; a longer piece goes to
/// [`EdgeCells::wide`].
const EDGE_SAMPLES_MAX: f64 = 4096.0;
/// The most cells a query reads; a larger segment scans every edge.
const QUERY_CELLS_MAX: i64 = 4096;
/// The samples a grid may take, per edge of the object and in all
/// ([`EdgeCells::budget`]).
const GRID_SAMPLES_PER_EDGE: usize = 32;
const GRID_SAMPLES_BASE: usize = 65_536;

/// The box of `pts` grown by `grow`; `None` for no points.
fn grown_box(pts: impl Iterator<Item = Point3>, grow: f64) -> Option<(Point3, Point3)> {
    let mut out: Option<(Point3, Point3)> = None;
    for p in pts {
        out = Some(match out {
            None => (p, p),
            Some((lo, hi)) => (
                Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
            ),
        });
    }
    out.map(|(lo, hi)| {
        (
            Point3::new(lo.x - grow, lo.y - grow, lo.z - grow),
            Point3::new(hi.x + grow, hi.y + grow, hi.z + grow),
        )
    })
}

/// The part of the edge `p`–`q` inside the box (Liang–Barsky), as its end
/// parameters; `None` when it misses the box.
fn clip(p: Point3, q: Point3, (lo, hi): (Point3, Point3)) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (a, b, l, h) in [
        (p.x, q.x, lo.x, hi.x),
        (p.y, q.y, lo.y, hi.y),
        (p.z, q.z, lo.z, hi.z),
    ] {
        let d = b - a;
        if d == 0.0 {
            if a < l || a > h {
                return None;
            }
            continue;
        }
        let (u, v) = ((l - a) / d, (h - a) / d);
        let (u, v) = (u.min(v), u.max(v));
        t0 = t0.max(u);
        t1 = t1.min(v);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

fn contains_box((lo, hi): (Point3, Point3), (a, b): (Point3, Point3)) -> bool {
    lo.x <= a.x && lo.y <= a.y && lo.z <= a.z && b.x <= hi.x && b.y <= hi.y && b.z <= hi.z
}

impl<'a> EdgeCells<'a> {
    /// The grid for `obj` about `segs` (the segments it will be asked
    /// about), its cell their mean length, so a query reads a few cells.
    pub(crate) fn new(obj: &'a Object, segs: &[Segment]) -> EdgeCells<'a> {
        let total: f64 = segs.iter().map(|(a, b)| (*b - *a).length()).sum();
        let mean = total / segs.len().max(1) as f64;
        let cell = mean.max(4.0 * tol::BLEND_VERIFY);
        let reach = 2.0 * tol::BLEND_VERIFY;
        let mut inside = grown_box(segs.iter().flat_map(|&(a, b)| [a, b]), reach);
        let mut cells: BTreeMap<[i64; 3], Vec<EdgeId>> = BTreeMap::new();
        let mut wide = Vec::new();
        let mut abandoned = false;
        if let Some(bx) = inside {
            let cut = grown_box([bx.0, bx.1].into_iter(), cell).expect("two points");
            let mut left = Self::budget(obj);
            'edges: for e in obj.edges().keys() {
                add_locate_work(1);
                let Some((p, q)) = obj.edge_endpoints(e) else {
                    continue;
                };
                let Some((t0, t1)) = clip(p, q, cut) else {
                    continue;
                };
                let (a, b) = (p + (q - p) * t0, p + (q - p) * t1);
                let steps = ((b - a).length() / cell).ceil();
                if steps.is_nan() || steps > EDGE_SAMPLES_MAX {
                    wide.push(e);
                    continue;
                }
                let steps = steps.max(1.0) as usize;
                let mut last: Option<[i64; 3]> = None;
                for i in 0..=steps {
                    if left == 0 {
                        abandoned = true;
                        break 'edges;
                    }
                    left -= 1;
                    add_locate_work(1);
                    let k = cell_key(cell, a + (b - a) * (i as f64 / steps as f64));
                    if last != Some(k) {
                        let list = cells.entry(k).or_default();
                        if list.last() != Some(&e) {
                            list.push(e);
                        }
                        last = Some(k);
                    }
                }
            }
        }
        if abandoned {
            inside = None;
            cells.clear();
            wide.clear();
        }
        EdgeCells {
            obj,
            cell,
            inside,
            cells,
            wide,
            abandoned,
        }
    }

    /// The most samples the grid of `obj` may take before it is abandoned:
    /// [`GRID_SAMPLES_PER_EDGE`] per edge and [`GRID_SAMPLES_BASE`] more.
    /// A filleted rim's grid takes under two per edge of the object.
    fn budget(obj: &Object) -> usize {
        GRID_SAMPLES_PER_EDGE
            .saturating_mul(obj.edges().len())
            .saturating_add(GRID_SAMPLES_BASE)
    }

    /// [`edges_on`]`(obj, seg)`.
    pub(crate) fn edges_on(&self, seg: Segment) -> Vec<EdgeId> {
        let Some(test) = SegmentTest::new(seg) else {
            return Vec::new();
        };
        let reach = 2.0 * tol::BLEND_VERIFY;
        let query = grown_box([seg.0, seg.1].into_iter(), reach).expect("two points");
        if !self.inside.is_some_and(|b| contains_box(b, query)) {
            note_path(if self.abandoned {
                Path::Abandoned
            } else {
                Path::Outside
            });
            return edges_on(self.obj, seg);
        }
        let lo = cell_key(self.cell, query.0).map(|k| k.saturating_sub(1));
        let hi = cell_key(self.cell, query.1).map(|k| k.saturating_add(1));
        let count = (0..3).fold(1i64, |n, i| {
            n.saturating_mul(hi[i].saturating_sub(lo[i]).saturating_add(1))
        });
        if count > QUERY_CELLS_MAX {
            note_path(Path::TooManyCells);
            return edges_on(self.obj, seg);
        }
        note_path(if self.wide.is_empty() {
            Path::Grid
        } else {
            Path::GridWithWide
        });
        add_locate_work(usize::try_from(count).unwrap_or(usize::MAX));
        let mut near: Vec<EdgeId> = self.wide.clone();
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if let Some(es) = self.cells.get(&[x, y, z]) {
                        near.extend(es);
                    }
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        add_locate_work(near.len());
        near.retain(|&e| {
            self.obj
                .edge_endpoints(e)
                .is_some_and(|(p, q)| test.holds(p, q))
        });
        near
    }
}

/// The grid cell of `p`.
fn cell_key(cell: f64, p: Point3) -> [i64; 3] {
    [
        (p.x / cell).floor() as i64,
        (p.y / cell).floor() as i64,
        (p.z / cell).floor() as i64,
    ]
}

/// The way an [`EdgeCells::edges_on`] query was answered (test support).
#[derive(Clone, Copy)]
enum Path {
    /// The grid, with no wide edges.
    Grid,
    /// The grid and its wide edges.
    GridWithWide,
    /// The scan: the segment leaves the grid's box.
    Outside,
    /// The scan: the segment spans too many cells.
    TooManyCells,
    /// The scan: the grid went over its budget.
    Abandoned,
}

#[cfg(test)]
thread_local! {
    /// The queries answered each [`Path`] on this thread.
    static PATHS: std::cell::Cell<[usize; 5]> = const { std::cell::Cell::new([0; 5]) };
}

#[inline]
fn note_path(path: Path) {
    #[cfg(test)]
    PATHS.with(|c| {
        let mut n = c.get();
        n[path as usize] += 1;
        c.set(n);
    });
    #[cfg(not(test))]
    let _ = path;
}

thread_local! {
    /// The locate work done on this thread (test support): see
    /// [`locate_work_for_test`].
    static LOCATE_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts locate work in a build with debug assertions, where the specs
/// that read it run.
#[inline]
fn add_locate_work(n: usize) {
    if cfg!(debug_assertions) {
        LOCATE_WORK.with(|c| c.set(c.get().saturating_add(n)));
    }
}

/// The work finding edges along segments has done on the calling thread:
/// for [`edges_on`], every edge tested; for [`EdgeCells`], every edge its
/// build visits and every sample it takes, and per query the cells read
/// and the edges tested. Always zero in a build without debug assertions.
/// Test support only.
#[doc(hidden)]
pub fn locate_work_for_test() -> usize {
    LOCATE_WORK.with(std::cell::Cell::get)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Plane;
    use proptest::prelude::*;

    /// A convex solid from its polygons, each turned to face away from
    /// the vertex centroid.
    fn convex(points: &[Point3], faces: &[Vec<usize>]) -> Object {
        let c = points
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
            / points.len() as f64;
        let oriented: Vec<Vec<usize>> = faces
            .iter()
            .map(|f| {
                let ring: Vec<Point3> = f.iter().map(|&i| points[i]).collect();
                let n = Plane::from_polygon(&ring).unwrap().normal();
                let fc = ring
                    .iter()
                    .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
                    / ring.len() as f64;
                if n.dot(fc - c) < 0.0 {
                    f.iter().rev().copied().collect()
                } else {
                    f.clone()
                }
            })
            .collect();
        Object::from_polygons(points, &oriented).unwrap()
    }

    fn unit_box() -> Object {
        let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        convex(
            &[
                p(0.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(1.0, 1.0, 0.0),
                p(0.0, 1.0, 0.0),
                p(0.0, 0.0, 1.0),
                p(1.0, 0.0, 1.0),
                p(1.0, 1.0, 1.0),
                p(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
    }

    /// A cylinder of radius 0.1 and height 0.1 with `n` wall facets that
    /// claim it.
    fn cylinder(n: usize) -> Object {
        let ring = |z: f64| {
            (0..n).map(move |i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                Point3::new(0.1 * trig::cos(a), 0.1 * trig::sin(a), z)
            })
        };
        let points: Vec<Point3> = ring(0.0).chain(ring(0.1)).collect();
        let mut faces: Vec<Vec<usize>> = vec![(0..n).collect(), (n..2 * n).collect()];
        for i in 0..n {
            let j = (i + 1) % n;
            faces.push(vec![i, j, n + j, n + i]);
        }
        let mut obj = convex(&points, &faces);
        let walls: Vec<FaceId> = obj
            .faces()
            .iter()
            .filter(|(_, f)| f.plane.normal().z.abs() < 0.5)
            .map(|(id, _)| id)
            .collect();
        for w in walls {
            obj.set_face_surface_for_test(
                w,
                Some(SurfaceRef::Cylinder {
                    axis_point: Point3::new(0.0, 0.0, 0.0),
                    axis: Vec3::new(0.0, 0.0, 1.0),
                    radius: 0.1,
                }),
            );
        }
        obj
    }

    fn round(edges: Vec<EdgeId>, radius: f64) -> BlendSpec {
        BlendSpec {
            edges,
            profile: BlendProfile::Round {
                radius,
                segments: 4,
            },
            primary: None,
            corners: CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        }
    }

    fn edges_at_height(obj: &Object, z: f64) -> Vec<EdgeId> {
        obj.edges()
            .keys()
            .filter(|&e| {
                obj.edge_endpoints(e).is_some_and(|(p, q)| {
                    (p.z - z).abs() <= tol::POINT_MERGE && (q.z - z).abs() <= tol::POINT_MERGE
                })
            })
            .collect()
    }

    /// Objects with the blends and restorations the record replay locates
    /// segments on, and the segments their records hold.
    fn corpus() -> Vec<(Object, Vec<Segment>)> {
        let mut objs: Vec<Object> = Vec::new();
        let b = unit_box();
        let top = edges_at_height(&b, 1.0);
        let (one, _) = b.blend_edges(&round(vec![top[0]], 0.1)).unwrap();
        let (rim, _) = b.blend_edges(&round(top.clone(), 0.1)).unwrap();
        let c = cylinder(48);
        let (crim, _) = c
            .blend_edges(&round(edges_at_height(&c, 0.1), 0.01))
            .unwrap();
        let (both, _) = crim
            .blend_edges(&round(edges_at_height(&crim, 0.0), 0.01))
            .unwrap();
        objs.extend([b, one, rim, c, crim, both]);
        let mut out = Vec::new();
        for o in &objs {
            let mut stored: Vec<Segment> = Vec::new();
            for (&id, rec) in o.blend_records() {
                stored.extend(rec.spec.edges.iter().copied());
                if let Ok(r) = o.restore_blend(id) {
                    out.push((r.object, rec.spec.edges.clone()));
                }
            }
            out.push((o.clone(), stored));
        }
        out
    }

    /// A segment `len` long along every edge of `obj`, off its middle.
    fn pieces(obj: &Object, len: f64) -> Vec<Segment> {
        obj.edges()
            .keys()
            .filter_map(|e| {
                let (p, q) = obj.edge_endpoints(e)?;
                let u = (q - p).normalized().ok()?;
                let m = p + (q - p) * 0.37;
                Some((m, m + u * len))
            })
            .collect()
    }

    /// A segment a micrometer long along every edge of `obj`.
    fn micro(obj: &Object) -> Vec<Segment> {
        pieces(obj, 1e-6)
    }

    /// Micrometer segments along every edge of `obj` at 19 places from end
    /// to end, so some fall near every kind of cell boundary.
    fn micro_along(obj: &Object) -> Vec<Segment> {
        let mut out = Vec::new();
        for e in obj.edges().keys() {
            let Some((p, q)) = obj.edge_endpoints(e) else {
                continue;
            };
            let Ok(u) = (q - p).normalized() else {
                continue;
            };
            for k in 1..20 {
                let m = p + (q - p) * (k as f64 / 20.0);
                out.push((m, m + u * 1e-6));
            }
        }
        out
    }

    /// Segments about every edge of `obj`: the edge, reversed, its middle,
    /// grown past both ends, one end only, beside it within and past
    /// `BLEND_VERIFY`, along its line beyond it, and micrometers of it.
    fn probes(obj: &Object) -> Vec<Segment> {
        let mut out = Vec::new();
        for e in obj.edges().keys() {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            let d = q - p;
            let side = {
                let n = d.cross(Vec3::new(0.3, 0.5, 0.8));
                n.normalized().unwrap_or(Vec3::new(1.0, 0.0, 0.0))
            };
            let at = |t: f64| p + d * t;
            out.extend([
                (p, q),
                (q, p),
                (at(0.25), at(0.75)),
                (at(-1.0), at(2.0)),
                (at(0.5), at(3.0)),
                (at(1.0), at(2.0)),
                (at(1.5), at(2.5)),
                (
                    p + side * (0.5 * tol::BLEND_VERIFY),
                    q + side * (0.5 * tol::BLEND_VERIFY),
                ),
                (
                    p + side * (3.0 * tol::BLEND_VERIFY),
                    q + side * (3.0 * tol::BLEND_VERIFY),
                ),
                (p, p),
            ]);
        }
        out.extend(micro(obj));
        out.extend(micro_along(obj));
        out
    }

    /// The queries answered each [`Path`] and the locate work done while
    /// `f` ran.
    fn traced<R>(f: impl FnOnce() -> R) -> ([usize; 5], usize, R) {
        let (p0, w0) = (PATHS.with(std::cell::Cell::get), locate_work_for_test());
        let out = f();
        let p1 = PATHS.with(std::cell::Cell::get);
        let mut d = [0; 5];
        for i in 0..5 {
            d[i] = p1[i] - p0[i];
        }
        (d, locate_work_for_test() - w0, out)
    }

    const GRID: usize = Path::Grid as usize;
    const WIDE: usize = Path::GridWithWide as usize;
    const OUTSIDE: usize = Path::Outside as usize;
    const TOO_MANY: usize = Path::TooManyCells as usize;
    const ABANDONED: usize = Path::Abandoned as usize;

    /// [`EdgeCells::edges_on`] answers as [`edges_on`], the scan of every
    /// edge kept as its reference, for every segment the records hold and
    /// every probe about every edge, with the grid built for the stored
    /// segments, for all of them, for micrometer segments along every edge
    /// (cells so small the long edges go to the wide list), for one short
    /// and one long segment elsewhere (queries outside the grid), and for
    /// millimeter segments along every edge (cells small enough that edges
    /// take many samples, so the micrometer probes all along them test their
    /// spacing). Each sizing is held to the way it must answer: built for
    /// all the segments, the grid answers most of them at under a quarter of
    /// the scan's work (measured: about an eighth, the long probes reading
    /// many cells); the micrometer grid of a box reads its wide edges and
    /// sends whole edges to the scan as too many cells; a grid elsewhere
    /// sends them out.
    #[test]
    fn edge_cells_answer_as_the_scan() {
        let mut checked = 0;
        let mut by_sizing = [[0usize; 5]; 6];
        // Over the objects of 300 edges or more, built for all segments: the
        // grid's work on the queries it answers, and the scan's for them.
        let (mut grid_work, mut scan_work) = (0usize, 0usize);
        for (obj, stored) in corpus() {
            let mut segs = stored.clone();
            segs.extend(probes(&obj));
            let sizes: [Vec<Segment>; 6] = [
                stored.clone(),
                segs.clone(),
                micro(&obj),
                vec![(Point3::new(0.0, 0.0, 0.0), Point3::new(1e-6, 0.0, 0.0))],
                vec![(Point3::new(0.0, 0.0, 0.0), Point3::new(10.0, 0.0, 0.0))],
                pieces(&obj, 1e-3),
            ];
            for (i, sized) in sizes.iter().enumerate() {
                let cells = EdgeCells::new(&obj, sized);
                // Micrometer cells over a filleted rim go over the budget.
                assert!(i == 2 || !cells.abandoned, "sizing {i}: within the budget");
                if i == 2 && !cells.abandoned {
                    assert!(
                        !cells.wide.is_empty(),
                        "micrometer cells leave long edges wide"
                    );
                }
                for &s in &segs {
                    let (paths, work, got) = traced(|| cells.edges_on(s));
                    assert_eq!(got, edges_on(&obj, s), "sizing {i}, segment {s:?}");
                    for k in 0..5 {
                        by_sizing[i][k] += paths[k];
                    }
                    if i == 1 && obj.edges().len() >= 300 && paths[GRID] + paths[WIDE] == 1 {
                        grid_work += work;
                        scan_work += obj.edges().len();
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 10_000, "{checked}");
        let all = |n: [usize; 5]| n.iter().sum::<usize>();
        let on_grid = |n: [usize; 5]| n[GRID] + n[WIDE];
        assert!(
            on_grid(by_sizing[1]) * 10 >= all(by_sizing[1]) * 8,
            "built for all segments, the grid answers most: {:?}",
            by_sizing[1]
        );
        assert!(
            scan_work > 0 && grid_work * 4 <= scan_work,
            "grid work {grid_work} against the scan's {scan_work}"
        );
        assert!(
            by_sizing[2][WIDE] > 0,
            "the wide list is read: {:?}",
            by_sizing[2]
        );
        assert!(
            on_grid(by_sizing[5]) * 2 >= all(by_sizing[5]),
            "millimeter cells answer on the grid: {:?}",
            by_sizing[5]
        );
        assert!(
            by_sizing[2][TOO_MANY] > 0,
            "too many cells scan: {:?}",
            by_sizing[2]
        );
        assert!(
            by_sizing[3][OUTSIDE] == all(by_sizing[3]) - by_sizing[3][TOO_MANY]
                && by_sizing[3][OUTSIDE] > 0,
            "a grid elsewhere sends queries out: {:?}",
            by_sizing[3]
        );
        assert!(
            by_sizing
                .iter()
                .enumerate()
                .all(|(i, n)| i == 2 || n[ABANDONED] == 0)
        );
    }

    /// A record of micrometer segments spread over a large box (a damaged
    /// file) would sample every edge thousands of times: past its budget
    /// the grid is abandoned and every query takes the scan, answering as
    /// it does.
    #[test]
    fn a_grid_over_its_budget_is_abandoned_for_the_scan() {
        let c = cylinder(48);
        let (obj, _) = c
            .blend_edges(&round(edges_at_height(&c, 0.1), 0.01))
            .unwrap();
        // Two micrometer segments at opposite corners well outside the
        // object: one box over all of it, cells a micrometer wide.
        let far = |x: f64| (Point3::new(x, x, x), Point3::new(x + 1e-6, x, x));
        let sized = vec![far(-1.0), far(1.0)];
        let (_, work, cells) = traced(|| EdgeCells::new(&obj, &sized));
        assert!(cells.abandoned);
        assert!(cells.cells.is_empty() && cells.wide.is_empty());
        assert!(
            work <= EdgeCells::budget(&obj) + obj.edges().len() + 1,
            "the build stops at its budget: {work}"
        );
        let mut segs = probes(&obj);
        segs.extend(sized);
        let mut paths = [0usize; 5];
        for &s in &segs {
            let (p, _, got) = traced(|| cells.edges_on(s));
            assert_eq!(got, edges_on(&obj, s), "segment {s:?}");
            for k in 0..5 {
                paths[k] += p[k];
            }
        }
        assert_eq!(paths[ABANDONED], paths.iter().sum::<usize>(), "{paths:?}");
    }

    /// Random segments about random edges of a filleted cylinder and a
    /// filleted box: stretches along an edge's line, nudged off it by up to
    /// about twice `BLEND_VERIFY`, one in three a micrometer long. The grid
    /// is built about the segments themselves, about them with cells of a
    /// millimeter or of a tenth of one (the box's long edges go wide), or
    /// about them and the object's whole diagonal, so every query falls
    /// inside it. At least one answer in 20
    /// must find edges (measured: one in ten), half the queries must be
    /// answered by the grid (measured: three in four; the rest span too many
    /// tenth-of-a-millimeter cells), and some must read the wide list.
    #[test]
    fn edge_cells_answer_as_the_scan_for_random_segments() {
        use proptest::test_runner::{Config, TestRunner};
        let objects: Vec<Object> = {
            let c = cylinder(24);
            let b = unit_box();
            let top = edges_at_height(&b, 1.0);
            vec![
                c.blend_edges(&round(edges_at_height(&c, 0.1), 0.01))
                    .unwrap()
                    .0,
                b.blend_edges(&round(top, 0.1)).unwrap().0,
            ]
        };
        let strategy = (
            proptest::collection::vec(
                (
                    0usize..10_000,
                    -1.5f64..2.5,
                    -1.5f64..2.5,
                    -1.5f64..1.5,
                    -1.5f64..1.5,
                ),
                1..40,
            ),
            0usize..4,
            0usize..2,
        );
        let tally = std::cell::Cell::new((0usize, 0usize, 0usize, 0usize));
        let mut runner = TestRunner::new(Config {
            cases: 96,
            failure_persistence: None,
            ..Config::default()
        });
        runner
            .run(&strategy, |(picks, sizing, which)| {
                let obj = &objects[which];
                let edges: Vec<EdgeId> = obj.edges().keys().collect();
                let segs: Vec<Segment> = picks
                    .iter()
                    .map(|&(k, t0, t1, u, v)| {
                        let (p, q) = obj.edge_endpoints(edges[k % edges.len()]).unwrap();
                        let d = q - p;
                        let off = Vec3::new(u, v, 0.5 * u - v) * tol::BLEND_VERIFY;
                        let a = p + d * t0 + off;
                        // One pick in three is a micrometer piece of the
                        // edge's line from `a`.
                        let b = match (k / 7) % 3 {
                            0 => a + d.normalized().unwrap() * (1e-6 * (1.0 + t1.abs())),
                            _ => p + d * t1 + off,
                        };
                        (a, b)
                    })
                    .collect();
                let mut sized = segs.clone();
                match sizing {
                    0 => {}
                    1 | 2 => {
                        // Enough short segments to bring the mean, so the
                        // cell, down to a millimeter (every edge sampled many
                        // times) or a tenth of one (the box's long edges go
                        // wide).
                        let len = if sizing == 1 { 1e-3 } else { 1e-4 };
                        let total: f64 = segs.iter().map(|(a, b)| (*b - *a).length()).sum();
                        let n = (total / len).ceil() as usize;
                        let a = segs[0].0;
                        sized.extend(std::iter::repeat_n((a, a + Vec3::new(len, 0.0, 0.0)), n));
                    }
                    _ => {
                        let bx =
                            grown_box(obj.vertices().values().map(|v| v.position), 0.0).unwrap();
                        sized.push(bx);
                    }
                }
                let cells = EdgeCells::new(obj, &sized);
                let (mut n, mut found, mut grid, mut wide) = tally.get();
                for &s in &segs {
                    let (paths, _, got) = traced(|| cells.edges_on(s));
                    prop_assert_eq!(&got, &edges_on(obj, s), "segment {:?}", s);
                    n += 1;
                    found += usize::from(!got.is_empty());
                    grid += paths[GRID] + paths[WIDE];
                    wide += paths[WIDE];
                }
                tally.set((n, found, grid, wide));
                Ok(())
            })
            .unwrap();
        let (n, found, grid, wide) = tally.get();
        assert!(found * 20 >= n, "{found} of {n} queries find an edge");
        assert!(grid * 2 >= n, "{grid} of {n} queries on the grid");
        assert!(wide > 0, "no query read the wide list");
    }
}
