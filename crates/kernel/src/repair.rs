//! Model health repairs on one Object (the import-health design §4): close
//! open rims with faces, and remove faces from an open object. Each is an
//! explicit operation a person asks for (DEVELOPMENT.md rule 4: diagnosis
//! never repairs); each works on a copy and commits only a result that
//! passes the validator, so a refusal leaves the object untouched.
//!
//! Neither operation reaches a solid. Capping has nothing to close on one,
//! and removing faces from one would make a watertight solid leaky — the
//! same stance that leaves single-face deletion out of the solid ops
//! (ARCHITECTURE.md §2.10).

use std::collections::{BTreeMap, BTreeSet};

pub mod mend;
pub use mend::{
    FILL_DIHEDRAL_WEIGHT, FILL_MWT_MAX_EDGES, FillScope, MendCensus, MendPlan, MendReport,
    SkippedRim, default_weld_distance, weld_range,
};

use crate::error::TopologyError;
use crate::geom2d::{boundaries_contact, point_inside_polygon, polygon_is_simple};
use crate::health::{Finding, Rim, diagnose};
use crate::ids::{FaceId, HalfEdgeId, LoopId, ShellId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::topo::{AttrSlot, Face, HalfEdge, Loop, LoopKind, Object, WatertightState};

/// The thinnest solid [`Object::thicken_sheet`] makes: a micron, far above
/// the point-merge tolerance its walls' corners must clear, far below any
/// thickness someone means (a pane of glass is millimeters).
pub const MIN_THICKNESS: f64 = 1e-6;

/// Why a repair refused. The object is untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairError {
    /// The object is watertight: there is nothing to cap, and removing
    /// faces would open a solid.
    ObjectIsSolid,
    /// A rim index past the object's rims (the caller's diagnosis is
    /// stale), or no rim was named.
    UnknownRim,
    /// A face handle that is not in the object.
    UnknownFace,
    /// The rim does not lie on one plane within the object's planarity
    /// tolerance, so no single flat face can close it.
    RimNotPlanar,
    /// The rim, seen on its plane, crosses or touches itself.
    RimSelfIntersecting,
    /// A face along the rim lies on the cap's plane facing the other way:
    /// the object is a flat sheet there, and a cap would be a face lying
    /// on its back.
    CapWouldBeCoplanar,
    /// The rim passes through a vertex where the boundary branches, or does
    /// not close: there is no single loop to put a face in.
    NonManifoldRim,
    /// The cap would cover another opening or a face on its plane, touch or
    /// cross another opening there, sit inside a wider opening it does not
    /// fill a gap of (a tube's end within the ring around it: close them
    /// together), or wind the same way as a rim it should be a hole in.
    CapsOverlap,
    /// Every face would go; deleting the object is the operation for that.
    RemovesEverything,
    /// Only a flat sheet (every face on one plane per island, no branching
    /// edge) can be pulled into a solid by thickening.
    NotAFlatSheet,
    /// The thickness is not a finite length of at least
    /// [`MIN_THICKNESS`].
    ThicknessTooSmall,
    /// Two of the sheet's faces meet along a line without sharing an edge
    /// there (a T-junction): thickening would build a wall on each side of
    /// that seam, a slit of no width inside the solid.
    UnjoinedSeam,
    /// The result failed validation (the violation is logged). A kernel
    /// bug, surfaced instead of committed.
    Invalid,
    /// Mend found nothing to do: no points to merge, nothing pinched, one
    /// island, no gap it could fill.
    NothingToMend,
    /// The weld distance is not a finite length within the object's range
    /// ([`mend::weld_range`]).
    WeldDistanceOutOfRange,
    /// Merging the points would leave an edge shared by more than two
    /// faces, or by two running the same way; separating pinched parts
    /// (the split) cuts those apart.
    WeldFoldsFaces,
    /// Merging the points would collapse every face.
    WeldCollapsesObject,
    /// A fill would touch or cross the object's faces.
    FillCrossesFaces,
    /// The gap is too small or too thin to take a face.
    FillRimTooSmall,
    /// Asked for a solid, the Mend would leave an object open.
    StillOpen,
    /// Removing the zero-width folds would leave parts touching at a
    /// point; separating pinched parts (the split) takes them apart.
    FoldRemovalPinches,
    /// Merging the points would turn a face over (a point merged across
    /// the face's opposite edge).
    WeldFlipsFaces,
    /// The Mend would leave the object more open than it was: a solid
    /// that would not stay one, or edges left open that were not (a weld
    /// pinching a narrow part shut).
    MendOpensObject,
    /// Merging the points would press opposite sides of a thin part
    /// together, leaving a shell around no volume.
    WeldFlattensObject,
}

impl std::fmt::Display for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepairError::ObjectIsSolid => write!(f, "the object is already a solid"),
            RepairError::UnknownRim => write!(f, "no such open rim on the object"),
            RepairError::UnknownFace => write!(f, "no such face on the object"),
            RepairError::RimNotPlanar => write!(f, "the open rim is not flat"),
            RepairError::RimSelfIntersecting => write!(f, "the open rim crosses itself"),
            RepairError::CapWouldBeCoplanar => {
                write!(f, "the cap would lie flat on the surface it closes")
            }
            RepairError::NonManifoldRim => write!(f, "the open rim branches or does not close"),
            RepairError::CapsOverlap => {
                write!(f, "the cap would overlap another open rim on its plane")
            }
            RepairError::RemovesEverything => write!(f, "that would remove every face"),
            RepairError::NotAFlatSheet => write!(f, "only a flat surface can be thickened"),
            RepairError::ThicknessTooSmall => write!(f, "the thickness is too small"),
            RepairError::UnjoinedSeam => {
                write!(f, "the surface has a seam whose faces do not share edges")
            }
            RepairError::Invalid => write!(f, "the repair produced invalid topology"),
            RepairError::NothingToMend => write!(f, "there is nothing to mend"),
            RepairError::WeldDistanceOutOfRange => {
                write!(f, "the distance for merging points is out of range")
            }
            RepairError::WeldFoldsFaces => write!(
                f,
                "merging those points would fold faces onto each other; separate pinched parts too"
            ),
            RepairError::WeldCollapsesObject => {
                write!(f, "merging those points would collapse every face")
            }
            RepairError::FillCrossesFaces => write!(f, "filling the gap would cross other faces"),
            RepairError::FillRimTooSmall => write!(f, "the gap is too small to fill"),
            RepairError::StillOpen => write!(f, "the result would still be open"),
            RepairError::FoldRemovalPinches => write!(
                f,
                "removing those folds would leave parts touching at a point; separate pinched parts too"
            ),
            RepairError::WeldFlipsFaces => {
                write!(f, "merging those points would turn a face over")
            }
            RepairError::MendOpensObject => {
                write!(f, "the mend would leave the object more open than it is")
            }
            RepairError::WeldFlattensObject => write!(
                f,
                "merging those points would press opposite sides together"
            ),
        }
    }
}

impl std::error::Error for RepairError {}

/// What [`Object::cap_rims`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct CapReport {
    /// The new faces, one per outer cap (a rim capped as another cap's hole
    /// adds a loop, not a face).
    pub faces: Vec<FaceId>,
    /// The object became watertight.
    pub closed: bool,
    /// Closing it showed an island was inside out, so its faces were
    /// turned over.
    pub reversed: bool,
}

/// What [`Object::delete_faces`] did.
#[derive(Debug, Clone)]
pub struct DeleteFacesReport {
    /// Islands the deletion cut loose, as objects of their own; the object
    /// keeps its largest island.
    pub pieces: Vec<Object>,
    /// The remaining object is watertight (the deleted faces were a stray
    /// sheet beside a closed shell).
    pub closed: bool,
}

/// The rims of `obj` that a face can close ([`rim_is_cappable`]) — what
/// capping "every rim that can take a face" means.
pub fn cappable_rims(obj: &Object) -> Vec<usize> {
    let d = diagnose(obj);
    (0..d.rims.len())
        .filter(|&i| rim_is_cappable(obj, &d, i))
        .collect()
}

/// Whether rim `i` of `d` (`obj`'s diagnosis) can take a face: closed,
/// planar, not the edge of a flat part of the surface
/// ([`Rim::sheet_edge`]), and not through a vertex where the boundary
/// branches. The cap can still refuse on what lies around it (another
/// opening or a face on its plane, a crossing).
pub fn rim_is_cappable(obj: &Object, d: &crate::health::Diagnosis, i: usize) -> bool {
    let Some(r) = d.rims.get(i) else {
        return false;
    };
    r.closed
        && r.plane.is_some()
        && !r.sheet_edge
        && !r.half_edges.iter().any(|&h| {
            let v = obj.half_edges[h].origin;
            d.findings
                .iter()
                .any(|f| matches!(f, Finding::BranchingBoundaryVertex { vertex } if *vertex == v))
        })
}

/// Log a validation failure and refuse with [`RepairError::Invalid`].
fn invalid(e: TopologyError) -> RepairError {
    tracing::warn!(target: "kernel::op", error = %e, "repair result failed validation");
    RepairError::Invalid
}

/// One rim's cap polygon: the rim's vertices in the cap's winding (the
/// reverse of the boundary's), with the plane they wind around.
struct CapLoop {
    points: Vec<Point3>,
    plane: Plane,
}

impl Object {
    /// Close rims `rims` (indices into [`diagnose`]'s rims for this object)
    /// with planar faces. Each new face's loop runs against the rim, so
    /// every rim half-edge gains its twin and the face's normal agrees with
    /// its neighbors'. Rims capped together on one plane nest: a rim inside
    /// another becomes a hole of that one's face (the open end of a
    /// thick-walled pipe closes as one ring). New faces take the object's
    /// base material and no UV frame.
    ///
    /// An island the caps close whose signed volume is negative — built
    /// inside out, which only shows once it encloses something — is turned
    /// over, unless it lies inside another closed island (a cavity).
    ///
    /// # Errors
    /// See [`RepairError`]; on any error the object is untouched.
    pub fn cap_rims(&mut self, rims: &[usize]) -> Result<CapReport, RepairError> {
        if self.watertight == WatertightState::Watertight {
            return Err(RepairError::ObjectIsSolid);
        }
        let d = diagnose(self);
        let wanted: BTreeSet<usize> = rims.iter().copied().collect();
        if wanted.is_empty() || wanted.iter().any(|&r| r >= d.rims.len()) {
            return Err(RepairError::UnknownRim);
        }
        let branching: BTreeSet<VertexId> = d
            .findings
            .iter()
            .filter_map(|f| match f {
                Finding::BranchingBoundaryVertex { vertex } => Some(*vertex),
                _ => None,
            })
            .collect();
        let tol = self.planarity_tol;

        // Every requested rim must be a clean planar loop.
        let mut caps: BTreeMap<usize, CapLoop> = BTreeMap::new();
        for &r in &wanted {
            let rim = &d.rims[r];
            if !rim.closed
                || rim
                    .half_edges
                    .iter()
                    .any(|&h| branching.contains(&self.half_edges[h].origin))
            {
                return Err(RepairError::NonManifoldRim);
            }
            if rim.plane.is_none() {
                // Newell's normal vanishes for a loop whose lobes wind
                // against each other: on one plane, but crossing itself.
                return Err(if self.rim_on_one_plane(rim) {
                    RepairError::RimSelfIntersecting
                } else {
                    RepairError::RimNotPlanar
                });
            }
            let cap = self.cap_loop(rim).ok_or(RepairError::RimSelfIntersecting)?;
            if !polygon_is_simple(&cap.points) {
                return Err(RepairError::RimSelfIntersecting);
            }
            if self.cap_lies_on_rim_faces(rim, &cap.plane) {
                return Err(RepairError::CapWouldBeCoplanar);
            }
            caps.insert(r, cap);
        }

        // Every other rim, which a cap must not cover: a coplanar one it
        // touches, contains, or sits inside, and any other with a vertex on
        // the cap's plane inside it (a warped or branching opening a flat
        // face would close over).
        let others: Vec<Vec<Point3>> = d
            .rims
            .iter()
            .enumerate()
            .filter(|(i, _)| !wanted.contains(i))
            .map(|(_, rim)| {
                rim.half_edges
                    .iter()
                    .map(|&h| self.vertices[self.half_edges[h].origin].position)
                    .collect()
            })
            .collect();

        // Nesting on each plane: a rim's parent is the innermost requested
        // rim around it. Even depth caps as a face, odd as that face's hole.
        let on_plane = |a: &CapLoop, b: &CapLoop| {
            b.points
                .iter()
                .all(|&p| a.plane.signed_distance(p).abs() <= tol)
        };
        let contains = |outer: &CapLoop, inner: &CapLoop| {
            point_inside_polygon(inner.points[0], &outer.points, outer.plane.normal())
        };
        let closed_face: BTreeSet<FaceId> = self
            .face_components()
            .into_iter()
            .filter(|faces| self.island_is_closed(faces))
            .flatten()
            .collect();
        let mut parent: BTreeMap<usize, Option<usize>> = BTreeMap::new();
        let mut depth: BTreeMap<usize, usize> = BTreeMap::new();
        for (&r, cap) in &caps {
            let normal = cap.plane.normal();
            let near = |p: &Point3| cap.plane.signed_distance(*p).abs() <= tol;
            // A gap in a surface (every face along the rim on the cap's
            // plane, facing its way) may sit inside a wider opening: the
            // surface around it is there. A cap closing the end of a tube
            // inside a wider opening would float in mid-air.
            let fills_gap = self.cap_fills_gap(&d.rims[r], &cap.plane);
            for other in &others {
                let covers = if other.iter().all(near) {
                    boundaries_contact(&cap.points, other)
                        || point_inside_polygon(other[0], &cap.points, normal)
                        || (!fills_gap && point_inside_polygon(cap.points[0], other, normal))
                } else {
                    other
                        .iter()
                        .any(|p| near(p) && point_inside_polygon(*p, &cap.points, normal))
                };
                if covers {
                    return Err(RepairError::CapsOverlap);
                }
            }
            // Nor may it cover a face of an open island already lying on its
            // plane. A closed island's face there is a solid resting on the
            // opening: contact, like two objects touching, not overlap.
            let on_rim: BTreeSet<VertexId> = d.rims[r]
                .half_edges
                .iter()
                .map(|&h| self.half_edges[h].origin)
                .collect();
            let covered = self.faces.iter().any(|(f, face)| {
                if closed_face.contains(&f) {
                    return false;
                }
                let ring: Vec<HalfEdgeId> = self.loop_half_edges(face.outer_loop).collect();
                ring.iter()
                    .all(|&h| near(&self.vertices[self.half_edges[h].origin].position))
                    && ring.iter().any(|&h| {
                        let v = self.half_edges[h].origin;
                        !on_rim.contains(&v)
                            && point_inside_polygon(self.vertices[v].position, &cap.points, normal)
                    })
            });
            if covered {
                return Err(RepairError::CapsOverlap);
            }
            let mut around: Vec<usize> = Vec::new();
            for (&s, other) in &caps {
                if s == r || !on_plane(cap, other) {
                    continue;
                }
                if boundaries_contact(&cap.points, &other.points) {
                    return Err(RepairError::CapsOverlap);
                }
                if contains(other, cap) {
                    around.push(s);
                }
            }
            depth.insert(r, around.len());
            // The innermost container is the one the most others contain.
            let inner_most = around.iter().copied().max_by_key(|&s| {
                caps.iter()
                    .filter(|&(&t, c)| t != s && on_plane(c, &caps[&s]) && contains(c, &caps[&s]))
                    .count()
            });
            parent.insert(r, inner_most);
        }
        let mut holes: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (&r, &dp) in &depth {
            if dp % 2 == 1 {
                let p = parent[&r].expect("an odd-depth rim sits inside another");
                if caps[&r].plane.normal().dot(caps[&p].plane.normal()) >= 0.0 {
                    return Err(RepairError::CapsOverlap);
                }
                holes.entry(p).or_default().push(r);
            }
        }

        let mut obj = self.clone();
        let shell_of = obj.shell_of_faces();
        let mut faces = Vec::new();
        for (&r, &dp) in &depth {
            if dp % 2 == 1 {
                continue;
            }
            let rim = &d.rims[r];
            let face = obj.faces.insert(Face {
                outer_loop: LoopId::default(),
                inner_loops: Vec::new(),
                plane: caps[&r].plane,
                material: None,
                back_material: None,
                uv_frame: None,
                surface: None,
                attrs: AttrSlot::NONE,
            });
            let outer = obj.add_cap_loop(&rim.half_edges, face, LoopKind::Outer);
            obj.faces[face].outer_loop = outer;
            for &h in holes.get(&r).map(Vec::as_slice).unwrap_or_default() {
                let inner = obj.add_cap_loop(&d.rims[h].half_edges, face, LoopKind::Inner);
                obj.faces[face].inner_loops.push(inner);
            }
            let neighbor = obj.loops[obj.half_edges[rim.half_edges[0]].loop_id].face;
            let shell: ShellId = shell_of[&neighbor];
            obj.shells[shell].faces.push(face);
            faces.push(face);
        }

        let closed = obj.refresh_watertight();
        let reversed = obj.orient_closed_islands();
        obj.validate().map_err(invalid)?;
        *self = obj;
        Ok(CapReport {
            faces,
            closed,
            reversed,
        })
    }

    /// Remove `faces` from this open object. Edges the faces shared with
    /// kept faces become boundary; edges and vertices nothing else uses go
    /// with them. If the faces that remain fall apart into several islands,
    /// the object keeps the largest (most faces) and the others are
    /// returned as objects of their own, so every object stays one piece.
    ///
    /// # Errors
    /// See [`RepairError`]; on any error the object is untouched. A solid
    /// refuses ([`RepairError::ObjectIsSolid`]); removing every face
    /// refuses ([`RepairError::RemovesEverything`]).
    pub fn delete_faces(&mut self, faces: &[FaceId]) -> Result<DeleteFacesReport, RepairError> {
        if self.watertight == WatertightState::Watertight {
            return Err(RepairError::ObjectIsSolid);
        }
        let doomed: BTreeSet<FaceId> = faces.iter().copied().collect();
        if doomed.is_empty() || doomed.iter().any(|&f| !self.faces.contains_key(f)) {
            return Err(RepairError::UnknownFace);
        }
        if doomed.len() == self.faces.len() {
            return Err(RepairError::RemovesEverything);
        }
        let mut obj = self.clone();
        obj.remove_faces(&doomed);

        let mut components = obj.face_components();
        let mut pieces = Vec::new();
        if components.len() > 1 {
            // Keep the largest island; ties go to the first.
            let keep = (0..components.len())
                .max_by(|&a, &b| {
                    components[a]
                        .len()
                        .cmp(&components[b].len())
                        .then(b.cmp(&a))
                })
                .expect("several components");
            components.swap_remove(keep);
            for comp in &components {
                let mut piece = obj.rebuild_component(comp);
                piece.carry_edge_claims(&obj);
                piece.orient_closed_islands();
                piece.validate().map_err(invalid)?;
                pieces.push(piece);
            }
            let cut: BTreeSet<FaceId> = components.into_iter().flatten().collect();
            obj.remove_faces(&cut);
        }

        let closed = obj.refresh_watertight();
        obj.orient_closed_islands();
        obj.validate().map_err(invalid)?;
        *self = obj;
        Ok(DeleteFacesReport { pieces, closed })
    }

    /// Pull a flat sheet into solids (the import-health design R4): each
    /// island of faces is swept along its own face normal from offset
    /// `start` to offset `end` (meters; `0, d` pulls it `d` out of its front
    /// like Push/Pull, `-d/2, d/2` centers the thickness on it). The
    /// sheet's faces appear on both sides, keeping their subdivision,
    /// materials and UV frames (shifted with them), and every open edge
    /// becomes a wall. Returns one closed object per island, the island
    /// with the most faces first; a closed island beside the sheet comes
    /// back unchanged, as a solid of its own. The sheet itself is not
    /// changed.
    ///
    /// # Errors
    /// [`RepairError::ObjectIsSolid`], [`RepairError::NotAFlatSheet`]
    /// (any other verdict, a bent or branching sheet included),
    /// [`RepairError::ThicknessTooSmall`], or [`RepairError::Invalid`].
    pub fn thicken_sheet(&self, start: f64, end: f64) -> Result<Vec<Object>, RepairError> {
        if self.watertight == WatertightState::Watertight {
            return Err(RepairError::ObjectIsSolid);
        }
        if !start.is_finite() || !end.is_finite() || (end - start).abs() < MIN_THICKNESS {
            return Err(RepairError::ThicknessTooSmall);
        }
        if !matches!(
            diagnose(self).verdict,
            crate::health::Verdict::FlatSheet { .. }
        ) {
            return Err(RepairError::NotAFlatSheet);
        }
        let (lo, hi) = if start < end {
            (start, end)
        } else {
            (end, start)
        };
        let mut islands = self.face_components();
        // Most faces first; the stable sort keeps ties in face order.
        islands.sort_by_key(|faces| std::cmp::Reverse(faces.len()));
        islands
            .iter()
            .map(|faces| {
                // A closed island beside the sheet (the verdict judges open
                // islands only) is a solid already: it comes along as is.
                if !self.island_is_closed(faces) {
                    self.thicken_island(faces, lo, hi)
                } else {
                    Ok(self.rebuild_component(faces))
                }
            })
            .collect()
    }

    /// One island of [`Object::thicken_sheet`]: a lower copy of the faces
    /// turned to face away from the solid, an upper copy facing out, and a
    /// wall under every boundary half-edge.
    #[allow(clippy::type_complexity)] // from_faces_with_holes' spec tuple
    fn thicken_island(&self, faces: &[FaceId], lo: f64, hi: f64) -> Result<Object, RepairError> {
        let he = &self.half_edges;
        let largest = faces
            .iter()
            .copied()
            .max_by(|&a, &b| {
                let area = |f: FaceId| {
                    let ring: Vec<Point3> = self.loop_positions(self.faces[f].outer_loop).collect();
                    crate::geom2d::signed_area_on_plane(&ring, self.faces[f].plane.normal()).abs()
                };
                area(a).total_cmp(&area(b))
            })
            .expect("an island has a face");
        let normal = self.faces[largest].plane.normal();
        if self.has_unjoined_seam(faces) {
            return Err(RepairError::UnjoinedSeam);
        }

        // Each vertex twice: at the lower offset and at the upper.
        let mut positions: Vec<Point3> = Vec::new();
        let mut index: BTreeMap<VertexId, (usize, usize)> = BTreeMap::new();
        let mut at = |v: VertexId, positions: &mut Vec<Point3>| {
            *index.entry(v).or_insert_with(|| {
                let p = self.vertices[v].position;
                positions.push(p + normal * lo);
                positions.push(p + normal * hi);
                (positions.len() - 2, positions.len() - 1)
            })
        };
        let shifted = |frame: Option<crate::material::UvFrame>, by: f64| {
            frame.map(|mut f| {
                let d = normal * by;
                f.u0 -= f.s.dot(d);
                f.v0 -= f.t.dot(d);
                f
            })
        };
        let plane_of = |ring: &[usize], positions: &[Point3]| {
            let pts: Vec<Point3> = ring.iter().map(|&i| positions[i]).collect();
            Plane::from_polygon(&pts).map_err(|_| {
                tracing::warn!(target: "kernel::op", "thicken: degenerate face ring");
                RepairError::Invalid
            })
        };

        let mut specs: Vec<crate::build::FaceSpec> = Vec::with_capacity(faces.len() * 3);
        for &f in faces {
            let face = &self.faces[f];
            let mut lower_rings = Vec::new();
            let mut upper_rings = Vec::new();
            for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                let pairs: Vec<(usize, usize)> = self
                    .loop_half_edges(l)
                    .map(|h| at(he[h].origin, &mut positions))
                    .collect();
                lower_rings.push(pairs.iter().rev().map(|p| p.0).collect::<Vec<_>>());
                upper_rings.push(pairs.iter().map(|p| p.1).collect::<Vec<_>>());
            }
            for (mut rings, by) in [(lower_rings, lo), (upper_rings, hi)] {
                let outer = rings.remove(0);
                let plane = plane_of(&outer, &positions)?;
                // The lower copy faces the way the sheet's back did, and
                // shows the back's paint.
                let material = if by == lo {
                    face.back_material.or(face.material)
                } else {
                    face.material
                };
                specs.push((
                    outer,
                    rings,
                    plane,
                    material,
                    shifted(face.uv_frame, by),
                    None,
                    // Both copies are the sheet's face, dictionary included.
                    self.face_attrs(f).cloned().unwrap_or_default(),
                ));
            }
        }
        for &f in faces {
            let face = &self.faces[f];
            for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                for h in self.loop_half_edges(l).filter(|&h| he[h].twin.is_none()) {
                    let (a_lo, a_hi) = at(he[h].origin, &mut positions);
                    let (b_lo, b_hi) = at(he[he[h].next].origin, &mut positions);
                    let wall = vec![a_lo, b_lo, b_hi, a_hi];
                    let plane = plane_of(&wall, &positions)?;
                    specs.push((
                        wall,
                        Vec::new(),
                        plane,
                        None,
                        None,
                        None,
                        crate::attr::AttrDict::new(),
                    ));
                }
            }
        }
        let mut obj = Object::from_faces_with_holes(&positions, &specs);
        obj.default_material = self.default_material;
        obj.planarity_tol = self.planarity_tol;
        // The sheet's edges keep their curve claims and soft flags on both
        // copies, each claim's center moved with its copy.
        for by in [lo, hi] {
            let mut moved = self.clone();
            let shift = normal * by;
            for v in moved.vertices.values_mut() {
                v.position = v.position + shift;
            }
            for e in moved.edges.values_mut() {
                if let Some(c) = e.curve.as_mut() {
                    c.center = c.center + shift;
                }
            }
            obj.carry_edge_claims(&moved);
        }
        if obj.watertight != WatertightState::Watertight || obj.signed_volume() <= 0.0 {
            tracing::warn!(target: "kernel::op", "thicken: result is not a closed, outward shell");
            return Err(RepairError::Invalid);
        }
        obj.validate().map_err(invalid)?;
        Ok(obj)
    }

    /// Turn every face over: each loop runs the other way and each plane
    /// faces the other side. Handles are kept; edges, twins and per-face
    /// attributes are unchanged. Turns an inside-out shell right side out.
    pub fn reverse_all_faces(&mut self) {
        let all: BTreeSet<FaceId> = self.faces.keys().collect();
        self.reverse_faces(&all);
        self.check_invariants();
    }

    /// Turn `faces` over — a set closed under twinning (whole islands), so
    /// every twin pair still runs opposite afterwards.
    pub(crate) fn reverse_faces(&mut self, faces: &BTreeSet<FaceId>) {
        let loops: Vec<LoopId> = faces
            .iter()
            .flat_map(|&f| {
                let face = &self.faces[f];
                std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied())
            })
            .collect();
        self.reverse_loops(&loops);
        for &f in faces {
            let face = &mut self.faces[f];
            face.plane = face.plane.flipped();
        }
    }

    /// Runs each of `loops` the other way, leaving every face plane as it
    /// is: the half of turning a face over that a map which mirrors a loop
    /// IN its plane needs (the loop's vertices moved; its face still faces
    /// the same side). The set must be closed under twinning, so every twin
    /// pair still runs opposite afterwards. Handles, edges and twins are
    /// kept.
    pub(crate) fn reverse_loops(&mut self, loops: &[LoopId]) {
        let hs: Vec<HalfEdgeId> = loops
            .iter()
            .flat_map(|&l| self.loop_half_edges(l).collect::<Vec<_>>())
            .collect();
        let flipped: Vec<(HalfEdgeId, VertexId)> = hs
            .iter()
            .map(|&h| (h, self.half_edges[self.half_edges[h].next].origin))
            .collect();
        for (h, origin) in flipped {
            let he = &mut self.half_edges[h];
            he.origin = origin;
            std::mem::swap(&mut he.next, &mut he.prev);
        }
        // A vertex's outgoing half-edge may now start elsewhere.
        let mut leaving: BTreeMap<VertexId, HalfEdgeId> = BTreeMap::new();
        for (h, he) in &self.half_edges {
            leaving.entry(he.origin).or_insert(h);
        }
        for (v, vert) in self.vertices.iter_mut() {
            if self.half_edges[vert.outgoing].origin != v {
                vert.outgoing = leaving[&v];
            }
        }
    }

    /// Whether two open edges of `faces` run along each other in opposite
    /// directions — faces meeting at a seam without sharing its edges (a
    /// T-junction), which leaves a slit of no width rather than an opening.
    fn has_unjoined_seam(&self, faces: &[FaceId]) -> bool {
        let segs: Vec<(Point3, Point3)> = faces
            .iter()
            .flat_map(|&f| {
                let face = &self.faces[f];
                std::iter::once(face.outer_loop)
                    .chain(face.inner_loops.iter().copied())
                    .flat_map(|l| self.loop_half_edges(l).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .filter(|&h| self.half_edges[h].twin.is_none())
            .map(|h| {
                let he = &self.half_edges[h];
                (
                    self.vertices[he.origin].position,
                    self.vertices[self.half_edges[he.next].origin].position,
                )
            })
            .collect();
        let tol = crate::tol::POINT_MERGE.max(self.planarity_tol * 1e-3);
        segs.iter().enumerate().any(|(i, &(a, b))| {
            let ab = b - a;
            let len = ab.length();
            if len <= tol {
                return false;
            }
            let dir = ab * (1.0 / len);
            segs.iter().enumerate().any(|(j, &(c, d))| {
                if i == j || (d - c).dot(dir) >= 0.0 {
                    return false;
                }
                // Both ends of c–d on the line through a–b ...
                let off = |p: Point3| {
                    let v = p - a;
                    (v - dir * v.dot(dir)).length()
                };
                if off(c) > tol || off(d) > tol {
                    return false;
                }
                // ... and overlapping a–b by more than a point.
                let (t0, t1) = ((c - a).dot(dir), (d - a).dot(dir));
                let (lo, hi) = (t0.min(t1), t0.max(t1));
                hi.min(len) - lo.max(0.0) > tol
            })
        })
    }

    /// Whether every half-edge of `faces` (one island) has a twin.
    fn island_is_closed(&self, faces: &[FaceId]) -> bool {
        faces.iter().all(|&f| {
            let face = &self.faces[f];
            std::iter::once(face.outer_loop)
                .chain(face.inner_loops.iter().copied())
                .all(|l| {
                    self.loop_half_edges(l)
                        .all(|h| self.half_edges[h].twin.is_some())
                })
        })
    }

    /// Turn over every closed island whose signed volume is negative and
    /// that no other closed island contains (a negative island inside
    /// another is a cavity, already right). Open islands have no inside to
    /// judge. Whether anything was turned.
    fn orient_closed_islands(&mut self) -> bool {
        let islands = self.face_components();
        let closed: Vec<bool> = islands
            .iter()
            .map(|faces| self.island_is_closed(faces))
            .collect();
        let volume = |faces: &[FaceId]| {
            let mut six_v = 0.0;
            for &f in faces {
                let face = &self.faces[f];
                for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                    let p: Vec<Vec3> = self.loop_positions(l).map(|pt| pt.to_vec()).collect();
                    for i in 1..p.len().saturating_sub(1) {
                        six_v += p[0].dot(p[i].cross(p[i + 1]));
                    }
                }
            }
            six_v
        };
        let shell = |faces: &[FaceId]| -> Vec<crate::ops::ShellFace> {
            faces
                .iter()
                .map(|&f| {
                    let face = &self.faces[f];
                    (
                        face.plane,
                        self.loop_positions(face.outer_loop).collect(),
                        face.inner_loops
                            .iter()
                            .map(|&l| self.loop_positions(l).collect())
                            .collect(),
                    )
                })
                .collect()
        };
        let mut turn: BTreeSet<FaceId> = BTreeSet::new();
        for (i, faces) in islands.iter().enumerate() {
            if !closed[i] || volume(faces) >= 0.0 {
                continue;
            }
            let sample = self
                .loop_positions(self.faces[faces[0]].outer_loop)
                .next()
                .expect("a face loop has vertices");
            let cavity = islands.iter().enumerate().any(|(j, other)| {
                j != i
                    && closed[j]
                    && volume(other) > 0.0
                    && crate::ops::point_in_shell_faces(&shell(other), sample)
            });
            if !cavity {
                turn.extend(faces.iter().copied());
            }
        }
        if turn.is_empty() {
            return false;
        }
        self.reverse_faces(&turn);
        true
    }

    /// Give this object's edges the curve claims and soft flags of the
    /// edges of `source` they coincide with — a piece rebuilt from
    /// `source`'s faces (which rebuilds edges bare) keeps what its edges
    /// were. A soft flag needs an interior edge, so it only carries onto
    /// one.
    fn carry_edge_claims(&mut self, source: &Object) {
        type Key = ([u64; 3], [u64; 3]);
        let key = |a: Point3, b: Point3| -> Key {
            let k = |p: Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            let (ka, kb) = (k(a), k(b));
            if ka <= kb { (ka, kb) } else { (kb, ka) }
        };
        let claims: BTreeMap<Key, (Option<crate::sketch::CurveGeom>, bool)> = source
            .edges
            .keys()
            .filter_map(|e| {
                let (a, b) = source.edge_endpoints(e)?;
                let edge = &source.edges[e];
                (edge.curve.is_some() || edge.soft).then(|| (key(a, b), (edge.curve, edge.soft)))
            })
            .collect();
        if claims.is_empty() {
            return;
        }
        let ids: Vec<crate::ids::EdgeId> = self.edges.keys().collect();
        for e in ids {
            let Some((a, b)) = self.edge_endpoints(e) else {
                continue;
            };
            if let Some(&(curve, soft)) = claims.get(&key(a, b)) {
                let edge = &mut self.edges[e];
                edge.curve = curve;
                edge.soft = soft && edge.twin_half_edge.is_some();
            }
        }
    }

    /// Divergence-theorem signed volume over every loop (holes wind
    /// opposite and subtract): positive for a right-side-out closed shell,
    /// negative for an inside-out one.
    pub fn signed_volume(&self) -> f64 {
        let mut six_v = 0.0;
        for f in self.faces.values() {
            for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
                let p: Vec<Vec3> = self.loop_positions(lid).map(|pt| pt.to_vec()).collect();
                for i in 1..p.len().saturating_sub(1) {
                    six_v += p[0].dot(p[i].cross(p[i + 1]));
                }
            }
        }
        six_v / 6.0
    }

    /// The enclosed volume of a watertight Object, in cubic model units;
    /// `None` for an open one (an open shell encloses nothing, and a guess
    /// would be a silent repair).
    pub fn volume(&self) -> Option<f64> {
        (self.watertight() == crate::topo::WatertightState::Watertight)
            .then(|| self.signed_volume().abs())
    }

    /// The summed area of every face (holes subtracted), in square model
    /// units. Defined for open Objects too.
    pub fn surface_area(&self) -> f64 {
        let ring_area = |lid| {
            let p: Vec<Vec3> = self.loop_positions(lid).map(|pt| pt.to_vec()).collect();
            let mut sum = Vec3::new(0.0, 0.0, 0.0);
            for i in 0..p.len() {
                sum = sum + p[i].cross(p[(i + 1) % p.len()]);
            }
            (sum * 0.5).length()
        };
        self.faces
            .values()
            .map(|f| {
                let holes: f64 = f.inner_loops.iter().map(|&l| ring_area(l)).sum();
                (ring_area(f.outer_loop) - holes).max(0.0)
            })
            .sum()
    }

    /// The cap polygon for `rim`: its vertices against the boundary's
    /// direction, with the plane that winding defines. `None` for a rim too
    /// short to bound an area.
    fn cap_loop(&self, rim: &Rim) -> Option<CapLoop> {
        let n = rim.half_edges.len();
        if n < 3 {
            return None;
        }
        let at = |k: usize| self.vertices[self.half_edges[rim.half_edges[k]].origin].position;
        let points: Vec<Point3> = std::iter::once(at(0)).chain((1..n).rev().map(at)).collect();
        let plane = Plane::from_polygon(&points).ok()?;
        Some(CapLoop { points, plane })
    }

    /// Whether `rim`'s vertices lie on one plane (within the object's
    /// planarity tolerance), judged by the plane through three well-spread
    /// vertices rather than by the winding, so a loop enclosing no net area
    /// still answers. Collinear vertices span no plane: false.
    fn rim_on_one_plane(&self, rim: &Rim) -> bool {
        let pts: Vec<Point3> = rim
            .half_edges
            .iter()
            .map(|&h| self.vertices[self.half_edges[h].origin].position)
            .collect();
        let Some(&a) = pts.first() else {
            return false;
        };
        let far = |from: Point3| {
            pts.iter()
                .copied()
                .max_by(|p, q| (*p - from).length().total_cmp(&(*q - from).length()))
                .unwrap_or(from)
        };
        let b = far(a);
        let c = pts
            .iter()
            .copied()
            .max_by(|p, q| {
                let area = |r: Point3| (b - a).cross(r - a).length();
                area(*p).total_cmp(&area(*q))
            })
            .unwrap_or(a);
        let Ok(plane) = Plane::from_point_normal(a, (b - a).cross(c - a)) else {
            return false;
        };
        pts.iter()
            .all(|&p| plane.signed_distance(p).abs() <= self.planarity_tol)
    }

    /// Whether a cap on `plane` would lie on top of a face along `rim`: one
    /// on the plane (within the object's planarity tolerance) facing the
    /// other way — the rim is the edge of a flat sheet, and the cap would
    /// be its back. A neighbor on the plane facing the same way is the
    /// face around a gap the cap fills (a hole in a flat plate), which is
    /// fine.
    fn cap_lies_on_rim_faces(&self, rim: &Rim, plane: &Plane) -> bool {
        let tol = self.planarity_tol;
        rim.half_edges.iter().any(|&h| {
            let face = &self.faces[self.loops[self.half_edges[h].loop_id].face];
            face.plane.normal().dot(plane.normal()) < 0.0
                && self
                    .loop_positions(face.outer_loop)
                    .all(|p| plane.signed_distance(p).abs() <= tol)
        })
    }

    /// Whether every face along `rim` lies on `plane` facing the same way:
    /// the rim is a gap in a surface, which a cap on that plane fills.
    fn cap_fills_gap(&self, rim: &Rim, plane: &Plane) -> bool {
        let tol = self.planarity_tol;
        rim.half_edges.iter().all(|&h| {
            let face = &self.faces[self.loops[self.half_edges[h].loop_id].face];
            face.plane.normal().dot(plane.normal()) > 0.0
                && self
                    .loop_positions(face.outer_loop)
                    .all(|p| plane.signed_distance(p).abs() <= tol)
        })
    }

    /// Add a loop of new half-edges twinning the boundary chain `rim`
    /// (each ends where the next starts), running against it, to `face`.
    fn add_cap_loop(&mut self, rim: &[HalfEdgeId], face: FaceId, kind: LoopKind) -> LoopId {
        let n = rim.len();
        let lid = self.loops.insert(Loop {
            face,
            first_half_edge: HalfEdgeId::default(),
            kind,
        });
        let twins: Vec<HalfEdgeId> = rim
            .iter()
            .map(|&h| {
                let b = self.half_edges[h];
                debug_assert!(b.twin.is_none(), "a rim half-edge is boundary");
                debug_assert_eq!(self.edges[b.edge].half_edge, h);
                self.half_edges.insert(HalfEdge {
                    origin: self.half_edges[b.next].origin,
                    twin: Some(h),
                    next: HalfEdgeId::default(),
                    prev: HalfEdgeId::default(),
                    edge: b.edge,
                    loop_id: lid,
                })
            })
            .collect();
        // The twin of rim[k] runs from rim[k]'s end back to its start, which
        // is where the twin of rim[k-1] begins.
        for k in 0..n {
            let t = twins[k];
            self.half_edges[t].next = twins[(k + n - 1) % n];
            self.half_edges[t].prev = twins[(k + 1) % n];
            self.half_edges[rim[k]].twin = Some(t);
            let edge = self.half_edges[t].edge;
            self.edges[edge].twin_half_edge = Some(t);
        }
        self.loops[lid].first_half_edge = twins[0];
        lid
    }

    /// Which shell each face is in.
    fn shell_of_faces(&self) -> BTreeMap<FaceId, ShellId> {
        self.shells
            .iter()
            .flat_map(|(s, shell)| shell.faces.iter().map(move |&f| (f, s)))
            .collect()
    }

    /// Recompute the watertight flag from the half-edges; true when closed.
    fn refresh_watertight(&mut self) -> bool {
        let closed = self.half_edges.values().all(|he| he.twin.is_some());
        self.watertight = if closed {
            WatertightState::Watertight
        } else {
            WatertightState::Open
        };
        closed
    }

    /// Remove `faces` with their loops and half-edges. A kept half-edge whose
    /// twin went becomes boundary (its edge loses any soft flag, which only
    /// an interior edge may carry); edges and vertices left with no
    /// half-edge go too, and shells left empty are dropped.
    fn remove_faces(&mut self, faces: &BTreeSet<FaceId>) {
        for &f in faces {
            let Some(face) = self.faces.remove(f) else {
                continue;
            };
            for lid in std::iter::once(face.outer_loop).chain(face.inner_loops) {
                let hs: Vec<HalfEdgeId> = self.loop_half_edges(lid).collect();
                for h in hs {
                    let he = self
                        .half_edges
                        .remove(h)
                        .expect("a loop's half-edge is live");
                    match he.twin.filter(|&t| self.half_edges.contains_key(t)) {
                        Some(t) => {
                            self.half_edges[t].twin = None;
                            let edge = &mut self.edges[he.edge];
                            edge.half_edge = t;
                            edge.twin_half_edge = None;
                            edge.soft = false;
                        }
                        None => {
                            self.edges.remove(he.edge);
                        }
                    }
                }
                self.loops.remove(lid);
            }
        }
        for shell in self.shells.values_mut() {
            shell.faces.retain(|f| !faces.contains(f));
        }
        self.shells.retain(|_, shell| !shell.faces.is_empty());
        let mut leaving: BTreeMap<VertexId, HalfEdgeId> = BTreeMap::new();
        for (h, he) in &self.half_edges {
            leaving.entry(he.origin).or_insert(h);
        }
        self.vertices.retain(|v, _| leaving.contains_key(&v));
        for (v, vert) in self.vertices.iter_mut() {
            if !self.half_edges.contains_key(vert.outgoing)
                || self.half_edges[vert.outgoing].origin != v
            {
                vert.outgoing = leaving[&v];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five unit quads in a row; the second, fourth and fifth share soft
    /// interior edges with their right-hand neighbors.
    fn soft_strip() -> Object {
        let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
        let positions: Vec<Point3> = (0..=5)
            .map(|i| p(i as f64, 0.0))
            .chain((0..=5).map(|i| p(i as f64, 1.0)))
            .collect();
        let faces: Vec<Vec<usize>> = (0..5).map(|i| vec![i, i + 1, i + 7, i + 6]).collect();
        let mut obj = Object::from_polygons(&positions, &faces).unwrap();
        // Every interior edge soft.
        for e in obj.edges.values_mut() {
            if e.twin_half_edge.is_some() {
                e.soft = true;
            }
        }
        obj
    }

    /// A 4 m square sheet of two faces whose shared line has a T-junction:
    /// the lower face's edge runs (3,2)→(1,2) in one piece, the upper
    /// face's in two, (1,2)→(2,2)→(3,2), so the seam between them is open.
    fn slit_sheet() -> Object {
        let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
        let positions = vec![
            p(0.0, 0.0),
            p(4.0, 0.0),
            p(4.0, 2.0),
            p(3.0, 2.0),
            p(1.0, 2.0),
            p(0.0, 2.0),
            p(2.0, 2.0),
            p(4.0, 4.0),
            p(0.0, 4.0),
        ];
        let faces = vec![vec![0, 1, 2, 3, 4, 5], vec![5, 4, 6, 3, 2, 7, 8]];
        Object::from_polygons(&positions, &faces).unwrap()
    }

    #[test]
    fn a_sheet_with_an_unjoined_seam_refuses_to_thicken() {
        let obj = slit_sheet();
        assert!(matches!(
            diagnose(&obj).verdict,
            crate::health::Verdict::FlatSheet { .. }
        ));
        assert_eq!(
            obj.thicken_sheet(0.0, 0.1).unwrap_err(),
            RepairError::UnjoinedSeam
        );
    }

    #[test]
    fn a_thickened_sheet_keeps_its_soft_edges_on_both_sides() {
        let solids = soft_strip().thicken_sheet(0.0, 0.5).unwrap();
        assert_eq!(solids.len(), 1);
        // Four interior edges, soft above and below.
        assert_eq!(solids[0].edges().values().filter(|e| e.soft).count(), 8);
    }

    #[test]
    fn a_split_off_piece_keeps_its_soft_edges() {
        let mut obj = soft_strip();
        let middle = obj
            .faces
            .iter()
            .find(|(_, f)| {
                obj.loop_positions(f.outer_loop)
                    .all(|q| (2.0..=3.0).contains(&q.x))
            })
            .map(|(id, _)| id)
            .unwrap();
        let report = obj.delete_faces(&[middle]).unwrap();
        assert_eq!(report.pieces.len(), 1);
        let piece = &report.pieces[0];
        assert_eq!(piece.faces().len(), 2);
        // The piece's one interior edge (x = 4) is still soft.
        let soft: Vec<_> = piece.edges().values().filter(|e| e.soft).collect();
        assert_eq!(soft.len(), 1);
        // And the kept island's too (x = 1).
        assert_eq!(obj.edges().values().filter(|e| e.soft).count(), 1);
    }
}
