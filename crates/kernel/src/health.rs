//! Model health: what an open (non-watertight) Object's boundary looks like,
//! and what kind of thing that makes it. Pure queries — diagnosing changes
//! nothing (DEVELOPMENT.md rule 4: no silent repair; every repair is a
//! separate, explicit operation).
//!
//! An Object is watertight exactly when every half-edge has a twin. The
//! half-edges without one are the object's boundary; chained head to tail
//! they form **rims**, the edges of the holes. A rim's shape and size say
//! what the object is:
//!
//! - a **flat sheet** (every vertex on one plane: a pane of glass, a face
//!   drawn on a wall) or a **bent sheet** (one rim around the whole object,
//!   not planar: a folded card, a roof surface) is a surface SketchUp lets
//!   people draw on purpose, not a defect;
//! - **cappable** (one planar rim around the whole object: a box or a bowl
//!   missing one face) closes with a single face;
//! - **holes** (only rims small relative to the object) close rim by rim;
//! - **mixed** needs a person; **non-manifold** (a boundary vertex where
//!   the rim branches, or a rim that does not close) is not to be trusted
//!   for any repair.

use std::collections::BTreeMap;

use crate::document::{AttrTarget, DocChange, Document, DocumentError, EntityRef};
use crate::ids::{ComponentId, HalfEdgeId, ObjectId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::topo::{Object, WatertightState};

/// How large a rim must be, relative to its object, to be the object's
/// outer edge rather than a hole in it: the rim's diameter against the
/// object's (see [`diameter`]). A classification heuristic, not a geometric
/// tolerance (so it lives here, not in `tol`): a box missing its lid has a
/// rim the size of the box; a bolt hole in a plate is a small fraction of
/// the plate.
pub const OUTER_RIM_FRACTION: f64 = 0.5;

/// One chain of boundary half-edges.
#[derive(Debug, Clone, PartialEq)]
pub struct Rim {
    /// The boundary half-edges in chain order (each ends where the next
    /// starts).
    pub half_edges: Vec<HalfEdgeId>,
    /// The rim's best-fit plane when every rim vertex lies within the
    /// object's planarity tolerance of it.
    pub plane: Option<Plane>,
    /// Summed length of the rim's half-edges, in meters.
    pub perimeter: f64,
    /// The rim's diameter (see [`diameter`]), in meters.
    pub extent: f64,
    /// Which connected island of faces the rim bounds (index into the
    /// object's face components, in face-storage order).
    pub component: usize,
    /// Whether the rim spans the object or is a hole in it.
    pub role: RimRole,
    /// The chain came back to where it started. An open chain is only
    /// possible on a non-manifold boundary.
    pub closed: bool,
    /// The rim is planar and runs along the edge of a flat part of the
    /// surface lying on its plane (a basin's flat lip): a face across it
    /// would lie on that part's back, so no face closes it. A gap in a
    /// flat part (a drain in a basin's floor) is not a sheet edge.
    pub sheet_edge: bool,
}

/// Whether a rim is the object's outer edge or a hole in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RimRole {
    /// Spans the object: its extent is at least [`OUTER_RIM_FRACTION`] of
    /// the object's.
    Outer,
    /// Smaller than that.
    Hole,
}

/// Something worth pointing at on an open object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// Rim `rim` (index into [`Diagnosis::rims`]) is open.
    OpenRim { rim: usize, role: RimRole },
    /// A boundary vertex with more than one outgoing boundary half-edge (a
    /// bow-tie), or where a rim stops without closing.
    BranchingBoundaryVertex { vertex: VertexId },
    /// The object is several unconnected islands of faces.
    DisconnectedIslands { count: usize },
}

/// What an object is, judged by its boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    /// Watertight.
    Solid,
    /// Every vertex on one plane; `holes` is how many rims are holes in the
    /// sheet's islands.
    FlatSheet { holes: usize },
    /// One rim around the whole object, not planar.
    BentSheet,
    /// One planar rim around the whole object: a single face closes it.
    Cappable,
    /// Only rims that are holes; `planar` of them can take a face each.
    Holes { planar: usize, warped: usize },
    /// A rim around the object plus others, or several around it.
    Mixed,
    /// A branching boundary vertex or a rim that does not close.
    NonManifold,
}

impl Verdict {
    /// A sheet: a surface someone drew on purpose, not an open solid.
    pub fn is_surface(self) -> bool {
        matches!(self, Verdict::FlatSheet { .. } | Verdict::BentSheet)
    }

    /// Open and not a surface: something that needs attention.
    pub fn needs_attention(self) -> bool {
        !matches!(self, Verdict::Solid) && !self.is_surface()
    }
}

/// An object's verdict and the evidence for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnosis {
    pub verdict: Verdict,
    pub rims: Vec<Rim>,
    pub findings: Vec<Finding>,
}

/// Diagnose `obj` from its boundary half-edges.
///
/// The rules, in order:
/// 1. no boundary half-edge → [`Verdict::Solid`];
/// 2. a branching boundary vertex, or a rim that does not close →
///    [`Verdict::NonManifold`] (rims are still walked, so they can be
///    drawn, but they are not trusted for repair);
/// 3. otherwise each island of faces with rims is judged on its own — its
///    rims sized against the island, not the object, so a loose sheet
///    beside a solid is a sheet and two open boxes are two boxes:
///    - every vertex within tolerance of the island's largest face → a
///      flat sheet;
///    - only rims smaller than the island → holes;
///    - one rim around the island, off any plane → a bent sheet;
///    - every rim planar (at least one around the island) → cappable, a
///      face per rim closes it (a box missing its lid, a short pipe open
///      at both ends);
///    - anything else → mixed;
/// 4. the object is what its open islands agree on: all flat sheets → a
///    flat sheet; all sheets → a bent sheet; all cappable → cappable; all
///    cappable or holes → holes; else mixed.
pub fn diagnose(obj: &Object) -> Diagnosis {
    let he = obj.half_edges();
    let mut boundary: Vec<HalfEdgeId> = he
        .iter()
        .filter(|(_, h)| h.twin.is_none())
        .map(|(id, _)| id)
        .collect();
    if boundary.is_empty() {
        debug_assert_eq!(obj.watertight(), WatertightState::Watertight);
        return Diagnosis {
            verdict: Verdict::Solid,
            rims: Vec::new(),
            findings: Vec::new(),
        };
    }
    boundary.sort_unstable();
    let origin = |h: HalfEdgeId| he[h].origin;
    let end = |h: HalfEdgeId| he[he[h].next].origin;
    let position = |v: VertexId| obj.vertices()[v].position;

    // Boundary half-edges leaving each vertex.
    let mut leaving: BTreeMap<VertexId, Vec<HalfEdgeId>> = BTreeMap::new();
    for &h in &boundary {
        leaving.entry(origin(h)).or_default().push(h);
    }
    let mut findings = Vec::new();
    let mut branching = false;
    for (&v, out) in &leaving {
        if out.len() > 1 {
            branching = true;
            findings.push(Finding::BranchingBoundaryVertex { vertex: v });
        }
    }

    let tol = obj.planarity_tol;
    let islands = islands(obj);
    let face_of = |h: HalfEdgeId| obj.loops()[he[h].loop_id].face;

    // Chain the boundary into rims: each half-edge continues with the
    // (lowest-id) unused boundary half-edge leaving its end vertex.
    let mut used: BTreeMap<HalfEdgeId, ()> = BTreeMap::new();
    let mut rims = Vec::new();
    for &start in &boundary {
        if used.contains_key(&start) {
            continue;
        }
        let mut chain = vec![start];
        used.insert(start, ());
        let first = origin(start);
        let mut cur = start;
        let closed = loop {
            let at = end(cur);
            if at == first {
                break true;
            }
            let next = leaving
                .get(&at)
                .and_then(|out| out.iter().copied().find(|h| !used.contains_key(h)));
            let Some(next) = next else {
                findings.push(Finding::BranchingBoundaryVertex { vertex: at });
                break false;
            };
            used.insert(next, ());
            chain.push(next);
            cur = next;
        };
        let points: Vec<Point3> = chain.iter().map(|&h| position(origin(h))).collect();
        let perimeter = chain
            .iter()
            .map(|&h| (position(end(h)) - position(origin(h))).length())
            .sum();
        let extent = diameter(&points);
        let component = islands.of_face[&face_of(start)];
        let role = if extent >= OUTER_RIM_FRACTION * islands.diameter[component] {
            RimRole::Outer
        } else {
            RimRole::Hole
        };
        let plane = planar_fit(&points, tol);
        // The boundary runs counter-clockwise about the faces beside it,
        // so a flat face beside the rim faces the way the rim's plane
        // does when it is the rim's own edge, and the other way around a
        // gap.
        let sheet_edge = plane.is_some_and(|pl| {
            chain.iter().any(|&h| {
                let face = &obj.faces()[face_of(h)];
                face.plane.normal().dot(pl.normal()) > 0.0
                    && obj
                        .loop_positions(face.outer_loop)
                        .all(|p| pl.signed_distance(p).abs() <= tol)
            })
        });
        rims.push(Rim {
            sheet_edge,
            plane,
            perimeter,
            extent,
            component,
            role,
            closed,
            half_edges: chain,
        });
    }
    let unclosed = rims.iter().any(|r| !r.closed);
    for (i, r) in rims.iter().enumerate() {
        findings.push(Finding::OpenRim {
            rim: i,
            role: r.role,
        });
    }
    if islands.count > 1 {
        findings.push(Finding::DisconnectedIslands {
            count: islands.count,
        });
    }

    let verdict = if branching || unclosed {
        Verdict::NonManifold
    } else {
        // Each open island's verdict, then what they agree on.
        let mut open: BTreeMap<usize, Vec<&Rim>> = BTreeMap::new();
        for r in &rims {
            open.entry(r.component).or_default().push(r);
        }
        let per_island: Vec<Verdict> = open
            .iter()
            .map(|(&island, rims)| island_verdict(islands.flat[island], rims))
            .collect();
        combine(&per_island)
    };
    Diagnosis {
        verdict,
        rims,
        findings,
    }
}

/// One open island's verdict from its flatness and its rims (rule 3 of
/// [`diagnose`]).
fn island_verdict(flat: bool, rims: &[&Rim]) -> Verdict {
    if flat {
        return Verdict::FlatSheet {
            holes: rims.len().saturating_sub(1),
        };
    }
    // Planar for a verdict means a face can close it.
    let planar = rims
        .iter()
        .filter(|r| r.plane.is_some() && !r.sheet_edge)
        .count();
    let outer = rims.iter().filter(|r| r.role == RimRole::Outer).count();
    if outer == 0 {
        Verdict::Holes {
            planar,
            warped: rims.len() - planar,
        }
    } else if rims.len() == 1 && planar == 0 {
        Verdict::BentSheet
    } else if planar == rims.len() {
        Verdict::Cappable
    } else {
        Verdict::Mixed
    }
}

/// The object's verdict from its open islands' (rule 4 of [`diagnose`]).
fn combine(islands: &[Verdict]) -> Verdict {
    let all = |f: fn(&Verdict) -> bool| islands.iter().all(f);
    if all(|v| matches!(v, Verdict::FlatSheet { .. })) {
        let holes = islands
            .iter()
            .map(|v| match v {
                Verdict::FlatSheet { holes } => *holes,
                _ => 0,
            })
            .sum();
        Verdict::FlatSheet { holes }
    } else if all(|v| v.is_surface()) {
        Verdict::BentSheet
    } else if all(|v| matches!(v, Verdict::Cappable)) {
        Verdict::Cappable
    } else if all(|v| matches!(v, Verdict::Cappable | Verdict::Holes { warped: 0, .. })) {
        // Every rim planar, some smaller than their island: cap each.
        Verdict::Holes {
            planar: islands
                .iter()
                .map(|v| match v {
                    Verdict::Holes { planar, .. } => *planar,
                    _ => 1,
                })
                .sum(),
            warped: 0,
        }
    } else {
        Verdict::Mixed
    }
}

/// The object's islands of faces (joined through twinned half-edges): each
/// face's island, and per island its diameter and whether it is flat.
struct Islands {
    of_face: BTreeMap<crate::ids::FaceId, usize>,
    count: usize,
    diameter: Vec<f64>,
    flat: Vec<bool>,
}

fn islands(obj: &Object) -> Islands {
    let (of_face, count) = face_components(obj);
    let he = obj.half_edges();
    let mut vertices: Vec<std::collections::BTreeSet<VertexId>> = vec![Default::default(); count];
    // The island's largest face (by its outer loop's area), whose plane a
    // flat island lies on: a sliver face's plane is too noisy to judge by.
    let mut largest: Vec<(f64, Option<crate::ids::FaceId>)> = vec![(-1.0, None); count];
    for (fid, face) in obj.faces().iter() {
        let island = of_face[&fid];
        let ring: Vec<Point3> = obj
            .loop_half_edges(face.outer_loop)
            .map(|h| obj.vertices()[he[h].origin].position)
            .collect();
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            for h in obj.loop_half_edges(l) {
                vertices[island].insert(he[h].origin);
            }
        }
        let area = newell(&ring).length();
        if area > largest[island].0 {
            largest[island] = (area, Some(fid));
        }
    }
    let tol = obj.planarity_tol;
    let diameter = vertices
        .iter()
        .map(|vs| {
            let pts: Vec<Point3> = vs.iter().map(|&v| obj.vertices()[v].position).collect();
            diameter(&pts)
        })
        .collect();
    let flat = (0..count)
        .map(|i| {
            let Some(f) = largest[i].1 else { return true };
            let plane = obj.faces()[f].plane;
            vertices[i]
                .iter()
                .all(|&v| plane.signed_distance(obj.vertices()[v].position).abs() <= tol)
        })
        .collect();
    Islands {
        of_face,
        count,
        diameter,
        flat,
    }
}

/// Twice the vector area of a closed polygon (Newell's method).
fn newell(points: &[Point3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        n = n + Vec3::new(
            (p.y - q.y) * (p.z + q.z),
            (p.z - q.z) * (p.x + q.x),
            (p.x - q.x) * (p.y + q.y),
        );
    }
    n
}

/// The face islands: each face's island index, and how many there are.
/// Faces are joined through twinned half-edges.
pub(crate) fn face_components(obj: &Object) -> (BTreeMap<crate::ids::FaceId, usize>, usize) {
    let faces: Vec<crate::ids::FaceId> = obj.faces().iter().map(|(id, _)| id).collect();
    let index: BTreeMap<crate::ids::FaceId, usize> =
        faces.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let he = obj.half_edges();
    for (_, h) in he.iter() {
        if let Some(t) = h.twin {
            let a = index[&obj.loops()[h.loop_id].face];
            let b = index[&obj.loops()[he[t].loop_id].face];
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    // Number the islands in face-storage order.
    let mut label: BTreeMap<usize, usize> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (i, &f) in faces.iter().enumerate() {
        let root = find(&mut parent, i);
        let next = label.len();
        let l = *label.entry(root).or_insert(next);
        out.insert(f, l);
    }
    (out, label.len())
}

/// How far across the points are, the same however they are turned or
/// listed: the farthest point from their centroid, then the farthest from
/// that one, and the distance between the two (linear time). Exact for a
/// box or a rectangle, and never more than the true diameter; an
/// axis-aligned bounding box would grow and shrink as the object turns.
/// (Starting from the centroid rather than the first point keeps the
/// answer from depending on the order the points come in.)
pub fn diameter(points: &[Point3]) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    // Each axis summed in sorted order, so the centroid is the same to the
    // bit however the points are listed.
    let axis = |f: fn(&Point3) -> f64| {
        let mut c: Vec<f64> = points.iter().map(f).collect();
        c.sort_by(f64::total_cmp);
        c.iter().sum::<f64>() / c.len() as f64
    };
    let center = Point3::new(axis(|p| p.x), axis(|p| p.y), axis(|p| p.z));
    // Ties go to the lowest point, so the listing order never decides.
    let farthest = |from: Point3| {
        points
            .iter()
            .copied()
            .max_by(|a, b| {
                (*a - from)
                    .length()
                    .total_cmp(&(*b - from).length())
                    .then(b.x.total_cmp(&a.x))
                    .then(b.y.total_cmp(&a.y))
                    .then(b.z.total_cmp(&a.z))
            })
            .unwrap_or(from)
    };
    let a = farthest(center);
    let b = farthest(a);
    (b - a).length()
}

/// The points' Newell plane through their centroid, when every point lies
/// within `tol` of it.
pub(crate) fn planar_fit(points: &[Point3], tol: f64) -> Option<Plane> {
    if points.len() < 3 {
        return None;
    }
    let n = newell(points);
    let c = points.iter().fold(Vec3::ZERO, |c, p| c + p.to_vec());
    let centroid = Point3::ORIGIN + c * (1.0 / points.len() as f64);
    let plane = Plane::from_point_normal(centroid, n.normalized().ok()?).ok()?;
    points
        .iter()
        .all(|&p| plane.signed_distance(p).abs() <= tol)
        .then_some(plane)
}

// ------------------------------------------------------------ document

/// The attribute namespace and key that mark an open object as a surface
/// someone accepted (`true`): the health list hides it unless asked. A
/// flag, not a kind — the object stays exactly as open as it is.
pub const SURFACE_NS: &str = "hew.health";
/// See [`SURFACE_NS`].
pub const SURFACE_KEY: &str = "surface";

/// One open object, as the health report lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthRow {
    pub object: ObjectId,
    pub diagnosis: Diagnosis,
    /// The component definition the object is a member of; a repair to it
    /// reaches every instance.
    pub definition: Option<ComponentId>,
    /// Where the object appears in the model: 1 for a world object, the
    /// number of placements (nested ones included) for a definition member.
    pub placements: usize,
    /// Marked as an accepted surface ([`SURFACE_NS`]).
    pub accepted_surface: bool,
    /// Index into [`HealthReport::families`] when the object shares a rim
    /// with another: pieces the importer split one shape into.
    pub seam_family: Option<usize>,
    /// The rims (indices into the diagnosis's rims, ascending) another
    /// row's rim coincides with: the seams of a split shape. Rejoining
    /// closes only these; the piece's other rims are openings.
    pub shared_rims: Vec<usize>,
}

/// Open objects that share rims: one shape the importer split at its bad
/// edges (a solid with an internal partition becomes two open halves and
/// the partition sheet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeamFamily {
    pub members: Vec<ObjectId>,
    /// How many rims the members share.
    pub shared_rims: usize,
}

/// Every open object in a scope, diagnosed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HealthReport {
    /// One per non-solid object, in object order.
    pub rows: Vec<HealthRow>,
    pub families: Vec<SeamFamily>,
    /// How many objects in the scope are solid (they have no row).
    pub solids: usize,
}

/// How many objects of each kind, for an import report or a badge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HealthCounts {
    pub objects: usize,
    pub solids: usize,
    pub flat_sheets: usize,
    pub bent_sheets: usize,
    pub cappable: usize,
    pub holes: usize,
    pub mixed: usize,
    pub non_manifold: usize,
    pub seam_families: usize,
}

impl HealthCounts {
    /// Sheets: surfaces someone drew on purpose.
    pub fn surfaces(&self) -> usize {
        self.flat_sheets + self.bent_sheets
    }

    /// Open objects that are not surfaces.
    pub fn needing_attention(&self) -> usize {
        self.cappable + self.holes + self.mixed + self.non_manifold
    }
}

impl HealthReport {
    /// The report's counts, per object (a definition member once).
    pub fn counts(&self) -> HealthCounts {
        let mut c = HealthCounts {
            objects: self.solids + self.rows.len(),
            solids: self.solids,
            seam_families: self.families.len(),
            ..HealthCounts::default()
        };
        for r in &self.rows {
            match r.diagnosis.verdict {
                Verdict::Solid => c.solids += 1,
                Verdict::FlatSheet { .. } => c.flat_sheets += 1,
                Verdict::BentSheet => c.bent_sheets += 1,
                Verdict::Cappable => c.cappable += 1,
                Verdict::Holes { .. } => c.holes += 1,
                Verdict::Mixed => c.mixed += 1,
                Verdict::NonManifold => c.non_manifold += 1,
            }
        }
        c
    }
}

impl Document {
    /// Diagnose every live object (world objects and definition members),
    /// or only `scope`'s: a row for each one that is not solid. Pure.
    pub fn health_report(&self, scope: Option<&[ObjectId]>) -> HealthReport {
        let ids: Vec<ObjectId> = match scope {
            Some(ids) => ids.to_vec(),
            None => self.live_object_ids(),
        };
        let mut placements: BTreeMap<ObjectId, usize> = BTreeMap::new();
        if ids
            .iter()
            .any(|&o| self.object_owner_component(o).is_some())
        {
            for (o, _, _) in self.expanded_placements() {
                *placements.entry(o).or_default() += 1;
            }
        }
        let mut report = HealthReport::default();
        for &id in &ids {
            let Some(obj) = self.object(id) else { continue };
            // The flag is kept honest by the validator: a solid needs no
            // walk of its half-edges.
            if obj.watertight() == WatertightState::Watertight {
                report.solids += 1;
                continue;
            }
            let diagnosis = diagnose(obj);
            if diagnosis.verdict == Verdict::Solid {
                report.solids += 1;
                continue;
            }
            let definition = self.object_owner_component(id);
            report.rows.push(HealthRow {
                object: id,
                definition,
                placements: if definition.is_some() {
                    placements.get(&id).copied().unwrap_or(0)
                } else {
                    1
                },
                accepted_surface: self.is_accepted_surface(id),
                seam_family: None,
                shared_rims: Vec::new(),
                diagnosis,
            });
        }
        self.group_seam_families(&mut report);
        report
    }

    /// The object's verdict and rims; `None` for a stale or hidden handle.
    pub fn object_health(&self, id: ObjectId) -> Option<Diagnosis> {
        self.object(id).map(diagnose)
    }

    /// Whether `id` carries the accepted-surface flag ([`SURFACE_NS`]).
    pub fn is_accepted_surface(&self, id: ObjectId) -> bool {
        let target = AttrTarget::Entity(EntityRef::Object(id));
        matches!(
            self.attr_get(&target),
            Ok(Some(dict)) if dict
                .get(SURFACE_NS)
                .and_then(|ns| ns.get(SURFACE_KEY))
                == Some(&crate::attr::AttrValue::Bool(true))
        )
    }

    /// Mark an open object as a surface someone accepted, or clear the mark
    /// (the import-health design R3): one undoable attribute write, no
    /// geometry. Setting what is already set records nothing. A solid
    /// refuses to take the mark (there is nothing open to accept); clearing
    /// is always allowed, so a mark that outlived a repair can go.
    ///
    /// # Errors
    /// [`DocumentError::UnknownObject`] for a stale or hidden handle;
    /// [`DocumentError::Repair`] with
    /// [`RepairError::ObjectIsSolid`](crate::repair::RepairError::ObjectIsSolid)
    /// when marking a solid.
    pub fn set_accepted_surface(
        &mut self,
        id: ObjectId,
        accepted: bool,
    ) -> Result<DocChange, DocumentError> {
        let obj = self.object(id).ok_or(DocumentError::UnknownObject)?;
        if accepted && obj.watertight() == WatertightState::Watertight {
            return Err(DocumentError::Repair(
                crate::repair::RepairError::ObjectIsSolid,
            ));
        }
        if self.is_accepted_surface(id) == accepted {
            return Ok(DocChange::default());
        }
        let target = AttrTarget::Entity(EntityRef::Object(id));
        if accepted {
            self.attr_set(
                target,
                SURFACE_NS,
                SURFACE_KEY,
                crate::attr::AttrValue::Bool(true),
            )
        } else {
            self.attr_delete(target, SURFACE_NS, Some(SURFACE_KEY))
        }
    }

    /// Join rows whose closed rims coincide — the same vertices on a grid a
    /// tenth of the object's planarity tolerance, in the same coordinate
    /// space (world, or one definition's) — into seam families. The
    /// importer's split pieces share exactly welded coordinates, so they
    /// always match; the grid only merges noise that lands in one cell.
    fn group_seam_families(&self, report: &mut HealthReport) {
        type Key = (Option<ComponentId>, Vec<(i64, i64, i64)>);
        // Each rim's key → the (row, rim) pairs that have it.
        let mut by_rim: BTreeMap<Key, Vec<(usize, usize)>> = BTreeMap::new();
        for (row_i, row) in report.rows.iter().enumerate() {
            let Some(obj) = self.object(row.object) else {
                continue;
            };
            let q = obj.planarity_tol / 10.0;
            let he = obj.half_edges();
            for (rim_i, rim) in row
                .diagnosis
                .rims
                .iter()
                .enumerate()
                .filter(|(_, r)| r.closed)
            {
                let mut key: Vec<(i64, i64, i64)> = rim
                    .half_edges
                    .iter()
                    .map(|&h| {
                        let p = obj.vertices()[he[h].origin].position;
                        (
                            (p.x / q).round() as i64,
                            (p.y / q).round() as i64,
                            (p.z / q).round() as i64,
                        )
                    })
                    .collect();
                key.sort_unstable();
                by_rim
                    .entry((row.definition, key))
                    .or_default()
                    .push((row_i, rim_i));
            }
        }
        let mut parent: Vec<usize> = (0..report.rows.len()).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut shared_at: Vec<usize> = Vec::new();
        for uses in by_rim.values() {
            let mut distinct: Vec<usize> = uses.iter().map(|&(r, _)| r).collect();
            distinct.sort_unstable();
            distinct.dedup();
            if distinct.len() < 2 {
                continue;
            }
            for &(r, rim) in uses {
                report.rows[r].shared_rims.push(rim);
            }
            for w in distinct.windows(2) {
                let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
            shared_at.push(distinct[0]);
        }
        let roots: Vec<usize> = (0..report.rows.len())
            .map(|i| find(&mut parent, i))
            .collect();
        let mut size: BTreeMap<usize, usize> = BTreeMap::new();
        for &r in &roots {
            *size.entry(r).or_default() += 1;
        }
        let mut family_of_root: BTreeMap<usize, usize> = BTreeMap::new();
        for (i, &root) in roots.iter().enumerate() {
            if size[&root] < 2 {
                continue;
            }
            let next = family_of_root.len();
            let f = *family_of_root.entry(root).or_insert(next);
            if f == report.families.len() {
                report.families.push(SeamFamily {
                    members: Vec::new(),
                    shared_rims: 0,
                });
            }
            report.families[f].members.push(report.rows[i].object);
            report.rows[i].seam_family = Some(f);
        }
        for i in shared_at {
            let root = find(&mut parent, i);
            if let Some(&f) = family_of_root.get(&root) {
                report.families[f].shared_rims += 1;
            }
        }
        for row in &mut report.rows {
            row.shared_rims.sort_unstable();
            row.shared_rims.dedup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    /// A box `size` with face `missing` (0 bottom, 1 top, 2.. sides)
    /// left out, under `rotate` (a rigid rotation applied to every corner)
    /// and moved to `at`.
    fn open_box(
        size: [f64; 3],
        missing: usize,
        rotate: &dyn Fn(Vec3) -> Vec3,
        at: Point3,
    ) -> Object {
        let [a, b, c] = size;
        let corners = [
            p(0.0, 0.0, 0.0),
            p(a, 0.0, 0.0),
            p(a, b, 0.0),
            p(0.0, b, 0.0),
            p(0.0, 0.0, c),
            p(a, 0.0, c),
            p(a, b, c),
            p(0.0, b, c),
        ];
        let positions: Vec<Point3> = corners.iter().map(|&q| at + rotate(q.to_vec())).collect();
        let faces: Vec<Vec<usize>> = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let kept: Vec<Vec<usize>> = faces
            .into_iter()
            .enumerate()
            .filter(|&(i, _)| i != missing)
            .map(|(_, f)| f)
            .collect();
        Object::from_polygons(&positions, &kept).expect("an open box builds")
    }

    fn face_perimeter(size: [f64; 3], missing: usize) -> f64 {
        let [a, b, c] = size;
        match missing {
            0 | 1 => 2.0 * (a + b),
            2 | 4 => 2.0 * (a + c),
            _ => 2.0 * (b + c),
        }
    }

    #[test]
    fn a_watertight_object_is_solid_with_no_rims() {
        for obj in [Object::tetrahedron(), open_box_closed()] {
            let d = diagnose(&obj);
            assert_eq!(d.verdict, Verdict::Solid);
            assert!(d.rims.is_empty() && d.findings.is_empty());
        }
    }

    fn open_box_closed() -> Object {
        let positions = [
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        Object::from_polygons(&positions, &faces).unwrap()
    }

    #[test]
    fn a_single_face_is_a_flat_sheet_with_one_planar_rim() {
        let d = diagnose(&Object::triangle());
        assert_eq!(d.verdict, Verdict::FlatSheet { holes: 0 });
        assert_eq!(d.rims.len(), 1);
        assert!(d.rims[0].plane.is_some() && d.rims[0].closed);
        assert_eq!(d.rims[0].role, RimRole::Outer);
    }

    #[test]
    fn a_tilted_face_is_a_sheet_never_a_cappable_box() {
        // One quad tilted 30° about x: its axis-aligned box has height, but
        // it is a single plane.
        let t = 30f64.to_radians();
        let tilt = |v: Vec3| Vec3::new(v.x, v.y * t.cos(), v.y * t.sin());
        let q = [
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ];
        let positions: Vec<Point3> = q
            .iter()
            .map(|&c| Point3::ORIGIN + tilt(c.to_vec()))
            .collect();
        let obj = Object::from_polygons(&positions, &[vec![0, 1, 2, 3]]).unwrap();
        assert_eq!(diagnose(&obj).verdict, Verdict::FlatSheet { holes: 0 });
    }

    #[test]
    fn a_box_missing_a_face_is_cappable_by_that_face() {
        let obj = open_box([2.0, 1.0, 0.5], 1, &|v| v, Point3::ORIGIN);
        let d = diagnose(&obj);
        assert_eq!(d.verdict, Verdict::Cappable);
        assert_eq!(d.rims.len(), 1);
        let rim = &d.rims[0];
        assert_eq!(rim.role, RimRole::Outer);
        assert!(rim.closed && rim.plane.is_some());
        assert!((rim.perimeter - 6.0).abs() < 1e-12, "{}", rim.perimeter);
        assert_eq!(rim.half_edges.len(), 4);
        // The rim lies on the missing top's plane.
        let plane = rim.plane.unwrap();
        assert!(plane.signed_distance(p(1.0, 0.5, 0.5)).abs() < 1e-12);
    }

    #[test]
    fn a_bent_sheet_has_one_outer_rim_off_any_plane() {
        // Two faces folded along a crease: one rim around both, not planar.
        let positions = [
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(1.0, 2.0, 1.0),
            p(0.0, 2.0, 1.0),
        ];
        let obj = Object::from_polygons(&positions, &[vec![0, 1, 2, 3], vec![3, 2, 4, 5]]).unwrap();
        let d = diagnose(&obj);
        assert_eq!(d.verdict, Verdict::BentSheet);
        assert!(d.rims[0].plane.is_none());
    }

    #[test]
    fn a_small_hole_in_a_closed_box_is_holes_planar() {
        // A box whose top has a square hole: the top is a face with an
        // inner loop that nothing else bounds.
        let positions = [
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 4.0, 0.0),
            p(0.0, 4.0, 0.0),
            p(0.0, 0.0, 4.0),
            p(4.0, 0.0, 4.0),
            p(4.0, 4.0, 4.0),
            p(0.0, 4.0, 4.0),
            p(1.5, 1.5, 4.0),
            p(2.5, 1.5, 4.0),
            p(2.5, 2.5, 4.0),
            p(1.5, 2.5, 4.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let mut holes = vec![Vec::new(); 6];
        holes[1] = vec![vec![8, 11, 10, 9]];
        let obj = Object::from_polygons_with_holes_import(
            &positions, &faces, &holes, &[None; 6], &[None; 6],
        )
        .unwrap();
        let d = diagnose(&obj);
        assert_eq!(
            d.verdict,
            Verdict::Holes {
                planar: 1,
                warped: 0
            }
        );
        assert_eq!(d.rims[0].role, RimRole::Hole);
        assert!((d.rims[0].perimeter - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_bow_tie_vertex_is_non_manifold_and_islands_are_reported() {
        // Two triangles touching only at vertex 0: two boundary half-edges
        // leave it.
        let positions = [
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(-1.0, 0.0, 1.0),
            p(-1.0, -1.0, 1.0),
        ];
        let obj = Object::from_polygons(&positions, &[vec![0, 1, 2], vec![0, 3, 4]]).unwrap();
        let d = diagnose(&obj);
        assert_eq!(d.verdict, Verdict::NonManifold);
        assert!(
            d.findings
                .iter()
                .any(|f| matches!(f, Finding::BranchingBoundaryVertex { .. }))
        );
        assert!(
            d.findings
                .contains(&Finding::DisconnectedIslands { count: 2 })
        );
    }

    fn quad(a: Point3, b: Point3, c: Point3, d: Point3) -> (Vec<Point3>, Vec<Vec<usize>>) {
        (vec![a, b, c, d], vec![vec![0, 1, 2, 3]])
    }

    /// Several meshes as one object (an imported group's single recipe).
    fn merged(parts: &[(Vec<Point3>, Vec<Vec<usize>>)]) -> Object {
        let mut positions = Vec::new();
        let mut faces = Vec::new();
        for (ps, fs) in parts {
            let base = positions.len();
            positions.extend_from_slice(ps);
            faces.extend(
                fs.iter()
                    .map(|f| f.iter().map(|i| i + base).collect::<Vec<_>>()),
            );
        }
        let n = faces.len();
        Object::from_polygons_with_holes_import(
            &positions,
            &faces,
            &vec![Vec::new(); n],
            &vec![None; n],
            &vec![None; n],
        )
        .expect("builds")
    }

    fn polys(obj: &Object) -> (Vec<Point3>, Vec<Vec<usize>>) {
        obj.to_polygons()
    }

    #[test]
    fn a_face_with_a_hole_is_a_flat_sheet_with_one_hole() {
        let positions = [
            p(0.0, 0.0, 0.0),
            p(4.0, 0.0, 0.0),
            p(4.0, 4.0, 0.0),
            p(0.0, 4.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(2.0, 1.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(1.0, 2.0, 0.0),
        ];
        let obj = Object::from_polygons_with_holes_import(
            &positions,
            &[vec![0, 1, 2, 3]],
            &[vec![vec![4, 7, 6, 5]]],
            &[None],
            &[None],
        )
        .unwrap();
        assert_eq!(diagnose(&obj).verdict, Verdict::FlatSheet { holes: 1 });
    }

    #[test]
    fn a_sheet_with_a_noisy_sliver_face_is_still_flat() {
        // The review's case: a 2 mm sliver with 0.1 mm of noise first, a
        // 10 m square second, all within the import tolerance of z = 0.
        // Judged by the sliver's tilted plane it was "cappable".
        let positions = [
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 0.002, 0.0001),
            p(0.0, 0.002, 0.0001),
            p(0.0, -10.0, 0.0),
            p(10.0, -10.0, 0.0),
        ];
        let obj = Object::from_polygons_with_holes_import(
            &positions,
            &[vec![0, 1, 2, 3], vec![1, 0, 4, 5]],
            &[Vec::new(), Vec::new()],
            &[None, None],
            &[None, None],
        )
        .unwrap();
        assert_eq!(diagnose(&obj).verdict, Verdict::FlatSheet { holes: 0 });
    }

    #[test]
    fn each_island_is_judged_against_itself() {
        let o = Point3::ORIGIN;
        let closed = polys(&open_box_closed());
        let open = |x: f64| polys(&open_box([1.0, 1.0, 1.0], 1, &|v| v, p(x, 0.0, 0.0)));
        let sheet = quad(
            p(5.0, 0.0, 0.0),
            p(6.0, 0.0, 0.0),
            p(6.0, 1.0, 0.0),
            p(5.0, 1.0, 0.0),
        );
        // A loose sheet beside a solid is a sheet, not a small hole.
        let d = diagnose(&merged(&[closed, sheet.clone()]));
        assert_eq!(d.verdict, Verdict::FlatSheet { holes: 0 });
        assert!(
            d.findings
                .contains(&Finding::DisconnectedIslands { count: 2 })
        );
        // Two open boxes are two cappable boxes.
        assert_eq!(
            diagnose(&merged(&[open(0.0), open(5.0)])).verdict,
            Verdict::Cappable
        );
        // Two sheets on different planes are sheets.
        let tilted = quad(
            p(0.0, 0.0, 3.0),
            p(1.0, 0.0, 4.0),
            p(1.0, 1.0, 4.0),
            p(0.0, 1.0, 3.0),
        );
        assert_eq!(
            diagnose(&merged(&[sheet, tilted])).verdict,
            Verdict::FlatSheet { holes: 0 }
        );
        let _ = o;
    }

    /// A square tube of `height`, open at both ends.
    fn tube(height: f64) -> Object {
        let h = height;
        let positions = [
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, h),
            p(1.0, 0.0, h),
            p(1.0, 1.0, h),
            p(0.0, 1.0, h),
        ];
        Object::from_polygons(
            &positions,
            &[
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_tube_open_at_both_ends_closes_with_a_face_per_end() {
        // Short: both ends span it — cappable, not "needs a person".
        let short = diagnose(&tube(0.2));
        assert_eq!(short.verdict, Verdict::Cappable);
        assert_eq!(short.rims.len(), 2);
        // Long: both ends are small against it — two planar holes.
        assert_eq!(
            diagnose(&tube(5.0)).verdict,
            Verdict::Holes {
                planar: 2,
                warped: 0
            }
        );
    }

    #[test]
    fn a_bent_sheet_with_a_planar_hole_is_mixed() {
        // The folded card of `a_bent_sheet...` with a small square hole in
        // its first face: a warped outer rim and a planar hole.
        let positions = [
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(1.0, 2.0, 1.0),
            p(0.0, 2.0, 1.0),
            p(0.4, 0.4, 0.0),
            p(0.6, 0.4, 0.0),
            p(0.6, 0.6, 0.0),
            p(0.4, 0.6, 0.0),
        ];
        let obj = Object::from_polygons_with_holes_import(
            &positions,
            &[vec![0, 1, 2, 3], vec![3, 2, 4, 5]],
            &[vec![vec![6, 9, 8, 7]], Vec::new()],
            &[None, None],
            &[None, None],
        )
        .unwrap();
        let d = diagnose(&obj);
        assert_eq!(d.verdict, Verdict::Mixed);
        assert_eq!(d.rims.iter().filter(|r| r.plane.is_some()).count(), 1);
    }

    #[test]
    fn the_object_verdict_is_what_its_islands_agree_on() {
        let fs = |h| Verdict::FlatSheet { holes: h };
        let holes = |p| Verdict::Holes {
            planar: p,
            warped: 0,
        };
        assert_eq!(combine(&[fs(1), fs(2)]), fs(3));
        assert_eq!(combine(&[fs(0), Verdict::BentSheet]), Verdict::BentSheet);
        assert_eq!(
            combine(&[Verdict::Cappable, Verdict::Cappable]),
            Verdict::Cappable
        );
        assert_eq!(combine(&[Verdict::Cappable, holes(2)]), holes(3));
        assert_eq!(combine(&[Verdict::Cappable, fs(0)]), Verdict::Mixed);
        assert_eq!(
            combine(&[
                holes(1),
                Verdict::Holes {
                    planar: 0,
                    warped: 1
                }
            ]),
            Verdict::Mixed
        );
    }

    #[test]
    fn surfaces_and_attention_split_the_verdicts() {
        assert!(Verdict::FlatSheet { holes: 2 }.is_surface());
        assert!(Verdict::BentSheet.is_surface());
        assert!(!Verdict::Solid.needs_attention() && !Verdict::BentSheet.needs_attention());
        for v in [
            Verdict::Cappable,
            Verdict::Holes {
                planar: 1,
                warped: 0,
            },
            Verdict::Mixed,
            Verdict::NonManifold,
        ] {
            assert!(v.needs_attention() && !v.is_surface(), "{v:?}");
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        /// Any box missing any one face, at any size, rotation and place,
        /// has one planar rim as long as the missing face's perimeter: the
        /// box is cappable when that face spans the box, and has a planar
        /// hole when it is a small end (a long tube's open end). Rotation
        /// covers the tilted-face trap.
        #[test]
        fn any_open_box_has_one_planar_rim_the_missing_faces_size(
            a in 0.05f64..20.0, b in 0.05f64..20.0, c in 0.05f64..20.0,
            missing in 0usize..6,
            yaw in 0.0f64..std::f64::consts::TAU, pitch in -1.5f64..1.5,
            x in -100.0f64..100.0, y in -100.0f64..100.0, z in -100.0f64..100.0,
        ) {
            let face_diag = match missing {
                0 | 1 => (a * a + b * b).sqrt(),
                2 | 4 => (a * a + c * c).sqrt(),
                _ => (b * b + c * c).sqrt(),
            };
            let box_diag = (a * a + b * b + c * c).sqrt();
            let ratio = face_diag / box_diag;
            // Right at the threshold the rounding of a rotated box decides.
            proptest::prop_assume!((ratio - OUTER_RIM_FRACTION).abs() > 1e-6);
            let rot = move |v: Vec3| {
                let (sy, cy) = yaw.sin_cos();
                let (sp, cp) = pitch.sin_cos();
                let v = Vec3::new(v.x * cy - v.y * sy, v.x * sy + v.y * cy, v.z);
                Vec3::new(v.x, v.y * cp - v.z * sp, v.y * sp + v.z * cp)
            };
            let obj = open_box([a, b, c], missing, &rot, p(x, y, z));
            let d = diagnose(&obj);
            let want_verdict = if ratio > OUTER_RIM_FRACTION {
                Verdict::Cappable
            } else {
                Verdict::Holes { planar: 1, warped: 0 }
            };
            proptest::prop_assert_eq!(d.verdict, want_verdict);
            proptest::prop_assert_eq!(d.rims.len(), 1);
            let want = face_perimeter([a, b, c], missing);
            proptest::prop_assert!((d.rims[0].perimeter - want).abs() < 1e-9 * want.max(1.0));
        }
    }
}
