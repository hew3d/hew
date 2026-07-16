//! Half-edge mesh elements and the `Object` that owns them.
//!
//! An `Object` is an island of geometry (ARCHITECTURE.md): sticky-geometry rules
//! apply inside it, never across Objects. Its watertightness is tracked
//! explicitly and kept honest by the validator.

use slotmap::SlotMap;

use crate::ids::{EdgeId, FaceId, HalfEdgeId, LoopId, ShellId, VertexId};
use crate::material::{FaceMaterial, UvFrame};
use crate::math::{Plane, Point3, Vec3};

/// A mesh vertex.
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    /// Position in f64 meters.
    pub position: Point3,
    /// One half-edge originating at this vertex.
    pub outgoing: HalfEdgeId,
}

/// One direction of an edge, bounding one loop.
#[derive(Debug, Clone, Copy)]
pub struct HalfEdge {
    /// Vertex this half-edge starts at.
    pub origin: VertexId,
    /// Opposite-direction partner on the adjacent face; `None` on a mesh
    /// boundary (the Object is then not watertight).
    pub twin: Option<HalfEdgeId>,
    /// Next half-edge around the loop.
    pub next: HalfEdgeId,
    /// Previous half-edge around the loop.
    pub prev: HalfEdgeId,
    /// The undirected edge this half-edge belongs to.
    pub edge: EdgeId,
    /// The loop this half-edge bounds.
    pub loop_id: LoopId,
}

/// An undirected edge: one or two half-edges.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    /// Always present.
    pub half_edge: HalfEdgeId,
    /// `None` on a boundary edge.
    pub twin_half_edge: Option<HalfEdgeId>,
    /// The analytic circle this edge is a chord facet of, when known — the
    /// solid-side mirror of [`crate::sketch::SketchEdge::curve`] +
    /// [`crate::sketch::CurveGeom`]. Set when an arc/circle is imprinted onto
    /// a face ([`Object::split_face_inner_with_curve`](crate::Object::split_face_inner_with_curve))
    /// or born on an extrusion cap, so a later push-through of that face can
    /// re-attribute the tunnel walls as [`SurfaceRef::Cylinder`] (the circle's
    /// identity would otherwise die at the imprint, leaving faceted tunnel
    /// walls — the true-curves design, "edge metadata is the on-ramp").
    ///
    /// Obeys the same map-or-drop contract as [`SurfaceRef`]: mapped under a
    /// similarity (center as a point, radius by the uniform scale), dropped
    /// under any non-similarity transform, across a boolean (result edges are
    /// rebuilt fresh with no claim), or when an op moves a subset of vertices
    /// off the stored circle ([`Object::drop_stale_edge_curves`]). The
    /// validator holds a present claim to its geometry: both endpoints lie
    /// within the object's planarity tolerance (`planarity_tol` — the same
    /// gate `Face::surface` uses, `PLANE_DIST` for native geometry, the wider
    /// `IMPORT_PLANE_DIST` for imported) of `radius` from `center`. Persisted
    /// in geometry buffer v5.
    pub curve: Option<crate::sketch::CurveGeom>,
}

/// Whether a loop is a face's outer boundary or a hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    /// Outer boundary, wound counter-clockwise seen from the face normal.
    Outer,
    /// Hole boundary (opposite winding).
    Inner,
}

/// A closed cycle of half-edges bounding a face.
#[derive(Debug, Clone, Copy)]
pub struct Loop {
    /// The face this loop bounds.
    pub face: FaceId,
    /// An arbitrary half-edge on the cycle.
    pub first_half_edge: HalfEdgeId,
    /// Outer boundary or hole.
    pub kind: LoopKind,
}

/// The analytic surface a planar face is a chord facet of — durable
/// metadata over the faceted carrier, never a substitute for the face's
/// `plane` (the true-curves design). A face carrying a `Cylinder`
/// asserts it approximates a patch of that infinite cylinder; angular and
/// axial extent are derived from the face's own vertices, never stored.
///
/// Obeys the map-or-drop contract: every operation either maps the
/// reference exactly (translations, rotations, uniform scale; split and
/// boolean sub-faces inherit — a sub-face of a chord facet lies on the same
/// chord plane of the same cylinder) or removes it (any move of the face
/// off its chord plane, any non-similarity transform). Dropping is not
/// geometry repair — the carrier is untouched; the face merely stops
/// claiming an analytic ancestry it no longer has. The validator holds a
/// present reference to the invariants a chord facet actually satisfies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceRef {
    /// An infinite right circular cylinder.
    Cylinder {
        /// A point on the axis (the profile curve's center at extrusion).
        axis_point: Point3,
        /// Unit axis direction (the extrusion sweep direction).
        axis: Vec3,
        /// Cylinder radius in meters, > [`crate::tol::POINT_MERGE`].
        radius: f64,
    },
}

impl SurfaceRef {
    /// Whether two references describe the same surface geometrically —
    /// tolerance-aware, unlike `==` (which is bitwise). Two cylinders agree
    /// iff they describe the same axis **line** (parallel or antiparallel
    /// axes within [`crate::tol::NORMAL_DIRECTION`], each axis point within
    /// [`crate::tol::POINT_MERGE`] of the other's axis line) and radii
    /// within [`crate::tol::POINT_MERGE`]. This is the grouping predicate
    /// every "logical wall" derivation shares (cap centers, whole-wall
    /// push/pull, soft edges), so they can never disagree about what one
    /// wall is.
    pub fn same_surface(&self, other: &SurfaceRef) -> bool {
        let SurfaceRef::Cylinder {
            axis_point: ap_a,
            axis: ax_a,
            radius: r_a,
        } = *self;
        let SurfaceRef::Cylinder {
            axis_point: ap_b,
            axis: ax_b,
            radius: r_b,
        } = *other;
        let dist_to_line = |p: Point3, origin: Point3, axis: Vec3| -> f64 {
            let d = p - origin;
            (d - axis * d.dot(axis)).length()
        };
        ax_a.dot(ax_b).abs() >= 1.0 - crate::tol::NORMAL_DIRECTION
            && (r_a - r_b).abs() <= crate::tol::POINT_MERGE
            && dist_to_line(ap_b, ap_a, ax_a) <= crate::tol::POINT_MERGE
    }
}

/// The per-face attribute state undo must be able to restore exactly: the
/// three fields a merge destroys along with the dissolved face. Snapshotted
/// by the merge reports and re-applied by the split ops' restore path, so an
/// undone merge never re-derives attributes from the current parent — which
/// would resurrect claims a face had legitimately lost (map-or-drop,
/// the true-curves design) or lose paint the dissolved face carried.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceAttrs {
    /// The face's material (see [`Face::material`]).
    pub material: FaceMaterial,
    /// The face's UV frame (see [`Face::uv_frame`]).
    pub uv_frame: Option<UvFrame>,
    /// The face's analytic surface reference (see [`Face::surface`]).
    pub surface: Option<SurfaceRef>,
}

/// A planar polygonal face; holes are represented as inner loops.
#[derive(Debug, Clone)]
pub struct Face {
    /// The outer boundary.
    pub outer_loop: LoopId,
    /// Hole boundaries (empty in M0; the builder does not create holes yet).
    pub inner_loops: Vec<LoopId>,
    /// The supporting plane, oriented so the outer loop winds CCW seen from
    /// the normal side.
    pub plane: Plane,
    /// The face's material in the [`crate::document::Document`] palette, or
    /// `None` for the default (unpainted) material (ARCHITECTURE.md). Carried by
    /// face-creating ops: a split's children inherit the parent's material; a
    /// boolean's result faces inherit their source face's; freshly extruded
    /// side walls default to `None`.
    pub material: FaceMaterial,
    /// Per-face affine UV frame (ARCHITECTURE.md extension). When `Some`, the
    /// tessellator uses `frame.apply(p)` instead of the  `world_size`
    /// planar projection. Propagated alongside `material` in face-creating ops
    /// (split children and boolean result faces inherit the parent/source frame;
    /// freshly extruded side walls default to `None`).
    pub uv_frame: Option<UvFrame>,
    /// The analytic surface this face is a chord facet of, when known (an
    /// extruded arc/circle side wall). See [`SurfaceRef`] for the
    /// propagation (map-or-drop) contract. Persisted in geometry buffer v4.
    pub surface: Option<SurfaceRef>,
}

impl Face {
    /// This face's restorable attribute state (see [`FaceAttrs`]).
    pub fn attrs(&self) -> FaceAttrs {
        FaceAttrs {
            material: self.material,
            uv_frame: self.uv_frame,
            surface: self.surface,
        }
    }

    /// Overwrites this face's restorable attribute state (undo's restore
    /// path; forward ops derive attributes through their own rules instead).
    pub(crate) fn set_attrs(&mut self, attrs: FaceAttrs) {
        self.material = attrs.material;
        self.uv_frame = attrs.uv_frame;
        self.surface = attrs.surface;
    }
}

/// A connected set of faces. M0 builds a single shell per Object.
#[derive(Debug, Clone)]
pub struct Shell {
    /// Faces belonging to this shell.
    pub faces: Vec<FaceId>,
}

/// Whether an Object's mesh encloses a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatertightState {
    /// Every edge has two half-edges; the mesh encloses a volume.
    Watertight,
    /// The mesh has boundary edges; flagged, never silently tolerated.
    Open,
}

/// An island of half-edge geometry with tracked watertightness.
///
/// All mutation goes through kernel operations that re-establish invariants;
/// in debug builds every public mutation re-runs the topology validator
/// (DEVELOPMENT.md rule 2).
#[derive(Debug, Clone)]
pub struct Object {
    pub(crate) vertices: SlotMap<VertexId, Vertex>,
    pub(crate) half_edges: SlotMap<HalfEdgeId, HalfEdge>,
    pub(crate) edges: SlotMap<EdgeId, Edge>,
    pub(crate) loops: SlotMap<LoopId, Loop>,
    pub(crate) faces: SlotMap<FaceId, Face>,
    pub(crate) shells: SlotMap<ShellId, Shell>,
    pub(crate) watertight: WatertightState,
    /// The object's base material ( follow-up): a face whose own `material`
    /// is `None` resolves to this. New faces (extrude walls, boolean walls) are
    /// created `None`, so they inherit the base — giving a solid a consistent
    /// color/texture "throughout". `None` here means the renderer's neutral
    /// default.
    pub(crate) default_material: FaceMaterial,
    /// Per-object planarity invariant tolerance (meters). Native
    /// geometry built by exact kernel construction uses [`tol::PLANE_DIST`] (the
    /// default). Objects built from *imported* foreign geometry carry
    /// [`tol::IMPORT_PLANE_DIST`] instead, because f32-quantized SketchUp/COLLADA
    /// faces are flat only to ~0.1 mm. `from_polygons` and the validator both
    /// read this, so native objects stay strict while imports are accepted as the
    /// planar polygons they represent. Persisted in geometry buffer v3.
    pub(crate) planarity_tol: f64, // crate::tol::PLANE_DIST by default
}

impl Object {
    /// An object with no elements. Crate-internal: public construction goes
    /// through [`Object::from_polygons`], which rejects empty input.
    pub(crate) fn empty() -> Object {
        Object {
            vertices: SlotMap::with_key(),
            half_edges: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            loops: SlotMap::with_key(),
            faces: SlotMap::with_key(),
            shells: SlotMap::with_key(),
            watertight: WatertightState::Open,
            default_material: None,
            planarity_tol: crate::tol::PLANE_DIST,
        }
    }

    /// Vertex storage (read-only).
    pub fn vertices(&self) -> &SlotMap<VertexId, Vertex> {
        &self.vertices
    }

    /// Half-edge storage (read-only).
    pub fn half_edges(&self) -> &SlotMap<HalfEdgeId, HalfEdge> {
        &self.half_edges
    }

    /// Edge storage (read-only).
    pub fn edges(&self) -> &SlotMap<EdgeId, Edge> {
        &self.edges
    }

    /// Loop storage (read-only).
    pub fn loops(&self) -> &SlotMap<LoopId, Loop> {
        &self.loops
    }

    /// Face storage (read-only).
    pub fn faces(&self) -> &SlotMap<FaceId, Face> {
        &self.faces
    }

    /// Shell storage (read-only).
    pub fn shells(&self) -> &SlotMap<ShellId, Shell> {
        &self.shells
    }

    /// Whether this Object currently encloses a volume.
    pub fn watertight(&self) -> WatertightState {
        self.watertight
    }

    /// The object's base material ( follow-up): the material a face with no
    /// own material resolves to. `None` = the renderer's neutral default.
    pub fn default_material(&self) -> FaceMaterial {
        self.default_material
    }

    /// The half-edges of `loop_id` in cycle order, starting at its
    /// `first_half_edge`.
    ///
    /// Assumes a structurally valid object (loops close); guaranteed by the
    /// validator for anything the kernel hands out.
    pub fn loop_half_edges(&self, loop_id: LoopId) -> impl Iterator<Item = HalfEdgeId> + '_ {
        let first = self.loops[loop_id].first_half_edge;
        let mut current = Some(first);
        std::iter::from_fn(move || {
            let h = current?;
            let next = self.half_edges[h].next;
            current = if next == first { None } else { Some(next) };
            Some(h)
        })
    }

    /// The positions of `loop_id`'s vertices in cycle order.
    pub fn loop_positions(&self, loop_id: LoopId) -> impl Iterator<Item = Point3> + '_ {
        self.loop_half_edges(loop_id)
            .map(|h| self.vertices[self.half_edges[h].origin].position)
    }

    /// An edge's two endpoint positions, in object-local space.
    ///
    /// Mirrors `inference::InferenceScene::register`'s edge extraction: an
    /// `Edge` references one of its two half-edges; the origin of that
    /// half-edge and the origin of its `next` are the edge's two endpoints.
    /// `None` if `edge` is a stale id.
    pub fn edge_endpoints(&self, edge: EdgeId) -> Option<(Point3, Point3)> {
        let e = self.edges.get(edge)?;
        let he = &self.half_edges[e.half_edge];
        let a = self.vertices[he.origin].position;
        let b = self.vertices[self.half_edges[he.next].origin].position;
        Some((a, b))
    }

    /// The true circle centers derived from this object's analytic surface
    /// references ([`SurfaceRef`], the true-curves design): for each
    /// distinct cylinder some faces claim, the axis points at the two axial
    /// extremes of the claiming faces' vertices — the exact centers of the
    /// extruded circle's bottom and top, wherever the caps ended up after
    /// push/pull or booleans. Each center carries a representative claiming
    /// face (the lowest-id one) for snap provenance/highlighting.
    ///
    /// Pure query, deterministic: faces are visited in slot order and
    /// cylinders grouped by first appearance (two references group iff they
    /// describe the same axis line — parallel axes within
    /// [`crate::tol::NORMAL_DIRECTION`], axis points within
    /// [`crate::tol::POINT_MERGE`] of the other's axis line — and radii
    /// within [`crate::tol::POINT_MERGE`]).
    ///
    /// Gated on surviving coverage ([`AnalyticRim::has_coverage`]): a rim
    /// whose station keeps zero arc (a slant-cut top) contributes no
    /// center — offering one would snap to the center of a circle that no
    /// longer exists anywhere on the solid.
    pub fn analytic_cap_centers(&self) -> Vec<(Point3, FaceId)> {
        self.analytic_rims()
            .into_iter()
            .filter(AnalyticRim::has_coverage)
            .map(|rim| (rim.center, rim.rep))
            .collect()
    }

    /// The rim circles derived from this object's analytic surface
    /// references: for each distinct claimed cylinder, the exact circles at
    /// the claiming faces' two axial extremes (the drawn circle's bottom and
    /// top, wherever the caps ended up), with the angular range the claiming
    /// facets actually cover. This is the general form of
    /// [`Object::analytic_cap_centers`] and what quadrant/tangent inference
    /// derives from (the true-curves design).
    ///
    /// Deterministic like `analytic_cap_centers` (slot-order visit,
    /// first-appearance grouping via [`SurfaceRef::same_surface`]); two rims
    /// per group, low extreme first. Coverage is computed PER RIM: only a
    /// claiming facet's boundary edges that actually lie at a rim's axial
    /// station (both endpoints within [`crate::tol::POINT_MERGE`] of the
    /// extreme, measured along the axis) contribute their angular span to
    /// that rim. A notch boolean-cut into one rim therefore uncovers the
    /// notched arc on that rim alone — the intact opposite rim keeps its
    /// coverage and must never mask the notch (snaps offered on a rim arc
    /// that no longer exists would snap to empty space).
    ///
    /// A rim can come back with EMPTY coverage (`Some(vec![])`, see
    /// [`AnalyticRim::has_coverage`]) when no boundary edge survives at its
    /// station — e.g. the top of a slant-cut cylinder. Such rims are still
    /// reported: the two-rims-per-group shape holds for indexing consumers,
    /// and the station is the wall's true axial extreme — real information
    /// about the surviving geometry. Consumers deriving candidates from the
    /// rim *circle* (center, quadrants, tangents) gate on `has_coverage`,
    /// so a vacant rim offers nothing.
    pub fn analytic_rims(&self) -> Vec<AnalyticRim> {
        struct Group {
            axis_point: Point3,
            axis: Vec3,
            radius: f64,
            rep: FaceId,
            t_min: f64,
            t_max: f64,
            u: Vec3,
            v: Vec3,
            /// Outer-loop boundary edges of the claiming facets, as
            /// `(t_a, theta_a, t_b, theta_b)` in the (u, v) frame — kept raw
            /// so each rim can select the edges at ITS axial station once
            /// the extremes are known.
            edges: Vec<[f64; 4]>,
        }
        let mut groups: Vec<Group> = Vec::new();
        for (fid, face) in &self.faces {
            let Some(surface) = face.surface else {
                continue;
            };
            let SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            } = surface;
            let group = groups.iter().position(|g| {
                SurfaceRef::Cylinder {
                    axis_point: g.axis_point,
                    axis: g.axis,
                    radius: g.radius,
                }
                .same_surface(&surface)
            });
            let index = match group {
                Some(i) => i,
                None => {
                    let (u, v) = crate::geom2d::plane_axes(axis);
                    groups.push(Group {
                        axis_point,
                        axis,
                        radius,
                        rep: fid,
                        t_min: f64::INFINITY,
                        t_max: f64::NEG_INFINITY,
                        u,
                        v,
                        edges: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            let g = &mut groups[index];
            // Axial extremes, plus every outer-loop edge's (t, theta) pair.
            // Angles are only meaningful off the axis; a vertex on the axis
            // cannot happen on a valid claim (radius > 0), so an edge with
            // an angle-less endpoint is simply skipped.
            let station = |p: Point3| -> (f64, Option<f64>) {
                let d = p - g.axis_point;
                let t = d.dot(g.axis);
                let radial = d - g.axis * t;
                if radial.length() <= crate::tol::NORMALIZE_MIN_LENGTH {
                    (t, None)
                } else {
                    (t, Some(radial.dot(g.v).atan2(radial.dot(g.u))))
                }
            };
            let positions: Vec<Point3> = self.loop_positions(face.outer_loop).collect();
            for (i, &p) in positions.iter().enumerate() {
                let (t_a, theta_a) = station(p);
                g.t_min = g.t_min.min(t_a);
                g.t_max = g.t_max.max(t_a);
                let (t_b, theta_b) = station(positions[(i + 1) % positions.len()]);
                if let (Some(a), Some(b)) = (theta_a, theta_b) {
                    g.edges.push([t_a, a, t_b, b]);
                }
            }
        }
        groups
            .into_iter()
            .flat_map(|g| {
                // A rim's coverage comes from the boundary edges lying AT
                // that rim: both endpoints at the extreme station (a
                // straight edge with both ends at the station lies wholly
                // at it). Each such edge spans the short way between its
                // endpoint angles — a chord facet's edge subtends less than
                // a half turn by construction.
                let rim_coverage = |t_rim: f64| -> Option<Vec<[f64; 2]>> {
                    let intervals: Vec<[f64; 2]> = g
                        .edges
                        .iter()
                        .filter(|e| {
                            (e[0] - t_rim).abs() <= crate::tol::POINT_MERGE
                                && (e[2] - t_rim).abs() <= crate::tol::POINT_MERGE
                        })
                        .map(|e| {
                            let mut offset = e[3] - e[1];
                            while offset > std::f64::consts::PI {
                                offset -= 2.0 * std::f64::consts::PI;
                            }
                            while offset <= -std::f64::consts::PI {
                                offset += 2.0 * std::f64::consts::PI;
                            }
                            [e[1].min(e[1] + offset), e[1].max(e[1] + offset)]
                        })
                        .collect();
                    merge_angular_intervals(&intervals, g.radius)
                };
                let low = AnalyticRim {
                    center: g.axis_point + g.axis * g.t_min,
                    axis: g.axis,
                    radius: g.radius,
                    rep: g.rep,
                    basis_u: g.u,
                    basis_v: g.v,
                    coverage: rim_coverage(g.t_min),
                };
                let high = AnalyticRim {
                    center: g.axis_point + g.axis * g.t_max,
                    coverage: rim_coverage(g.t_max),
                    ..low.clone()
                };
                [low, high]
            })
            .collect()
    }
}

/// One rim circle of a claimed cylinder — see [`Object::analytic_rims`].
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyticRim {
    /// The exact circle center (an axis point at one axial extreme).
    pub center: Point3,
    /// Unit cylinder axis (the rim circle's plane normal).
    pub axis: Vec3,
    /// The exact drawn radius.
    pub radius: f64,
    /// A representative claiming face (lowest slot id) for snap provenance.
    pub rep: FaceId,
    /// First basis vector of the angular frame (unit, perpendicular to
    /// `axis`); angle 0 lies along it.
    pub basis_u: Vec3,
    /// Second basis vector (unit, `axis × basis_u` handedness via
    /// `geom2d::plane_axes`).
    pub basis_v: Vec3,
    /// Merged angular coverage in the (basis_u, basis_v) frame: `None` =
    /// the full circle; otherwise disjoint `[start, end]` intervals with
    /// `start` normalized into `[-pi, pi)` and `end > start` (an interval
    /// may extend past `pi` — test angles at `angle` and `angle + 2pi`).
    pub coverage: Option<Vec<[f64; 2]>>,
}

impl AnalyticRim {
    /// Whether ANY arc of this rim circle survives. `false` exactly when
    /// coverage is the empty set — a rim whose axial station keeps no
    /// boundary edge at all (e.g. the top of a slant-cut cylinder, where
    /// every facet's upper edge slopes away from the extreme). A vacant rim
    /// still reports its station (the wall's true axial extent), but every
    /// snap candidate derived from the rim CIRCLE — center, quadrants,
    /// tangents — must gate on this: its center would be the center of no
    /// surviving circle, a fabricated point floating in space.
    pub fn has_coverage(&self) -> bool {
        !matches!(&self.coverage, Some(v) if v.is_empty())
    }

    /// Whether `angle` (radians, in the rim's basis frame) falls inside the
    /// covered range, within an angular tolerance derived from
    /// [`crate::tol::POINT_MERGE`] at this radius.
    pub fn covers(&self, angle: f64) -> bool {
        let Some(intervals) = &self.coverage else {
            return true;
        };
        let eps = crate::tol::POINT_MERGE / self.radius;
        let tau = 2.0 * std::f64::consts::PI;
        let mut a = angle;
        while a >= std::f64::consts::PI {
            a -= tau;
        }
        while a < -std::f64::consts::PI {
            a += tau;
        }
        intervals.iter().any(|&[s, e]| {
            (a >= s - eps && a <= e + eps) || (a + tau >= s - eps && a + tau <= e + eps)
        })
    }

    /// The rim's quadrant points — the covered subset of the four points at
    /// angles 0, pi/2, pi, 3pi/2 in the basis frame, on the exact circle.
    pub fn quadrant_points(&self) -> Vec<Point3> {
        [0.0f64, 0.5, 1.0, 1.5]
            .into_iter()
            .map(|q| q * std::f64::consts::PI)
            .filter(|&q| self.covers(q))
            .map(|q| {
                self.center
                    + self.basis_u * (self.radius * q.cos())
                    + self.basis_v * (self.radius * q.sin())
            })
            .collect()
    }
}

/// Merge raw per-facet angular intervals into disjoint coverage, `None` for
/// a (numerically) full circle. Intervals arrive unwrapped around arbitrary
/// anchors; normalize each start into `[-pi, pi)`, sort, and merge with an
/// angular epsilon derived from [`crate::tol::POINT_MERGE`] at `radius` (two
/// facets sharing a vertex must merge into one arc).
fn merge_angular_intervals(intervals: &[[f64; 2]], radius: f64) -> Option<Vec<[f64; 2]>> {
    if intervals.is_empty() {
        return Some(Vec::new());
    }
    let tau = 2.0 * std::f64::consts::PI;
    let eps = crate::tol::POINT_MERGE / radius;
    let mut normalized: Vec<[f64; 2]> = intervals
        .iter()
        .map(|&[s, e]| {
            let span = (e - s).min(tau);
            let mut start = s;
            while start >= std::f64::consts::PI {
                start -= tau;
            }
            while start < -std::f64::consts::PI {
                start += tau;
            }
            [start, start + span]
        })
        .collect();
    normalized.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<[f64; 2]> = Vec::new();
    for iv in normalized {
        match merged.last_mut() {
            Some(last) if iv[0] <= last[1] + eps => last[1] = last[1].max(iv[1]),
            _ => merged.push(iv),
        }
    }
    // Wrap-around: the last interval may reach past pi into the first.
    if merged.len() > 1 {
        let first = merged[0];
        let last = merged.last_mut().expect("nonempty");
        if last[1] >= first[0] + tau - eps {
            last[1] = last[1].max(first[1] + tau);
            merged.remove(0);
        }
    }
    let total: f64 = merged.iter().map(|&[s, e]| e - s).sum();
    if total >= tau - eps {
        None
    } else {
        Some(merged)
    }
}
