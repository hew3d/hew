//! Smooth (true-curves.md §6): an object's subdivision level, stored as the
//! first-party attribute `hew.smooth` / `level`, and Bake, which replaces
//! the cage with its subdivided solid. The surface itself is
//! [`crate::subdiv`]'s.

use std::collections::BTreeMap;

use crate::math::Plane;
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch};
use crate::shape_error::ShapeError;
use crate::subdiv::subdivide;
use crate::tol;
use crate::topo::{FaceAttrs, Object, WatertightState};

/// First-party attribute namespace of the Smooth level (true-curves.md §3.2).
pub const SMOOTH_NS: &str = "hew.smooth";
/// The level's key: an integer 1..=3; absent means Off.
pub const SMOOTH_LEVEL_KEY: &str = "level";
/// The highest Smooth level.
pub const MAX_SMOOTH_LEVEL: u8 = 3;

/// The level an attribute value encodes: 1..=3 for an integer in range,
/// else 0 (Off).
pub fn level_of(value: Option<&crate::attr::AttrValue>) -> u8 {
    match value {
        Some(crate::attr::AttrValue::Int(n)) if (1..=i64::from(MAX_SMOOTH_LEVEL)).contains(n) => {
            *n as u8
        }
        _ => 0,
    }
}

fn key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

impl Object {
    /// The cage's subdivided solid at `level`, as real geometry: each
    /// subdivided quad that is planar within `PLANE_DIST` is a face, any
    /// other is split along its 0–2 diagonal. Faces take their source cage
    /// face's material, back material and UV frame (no claim, no blend tag);
    /// crease edges stay creased and every other interior edge is soft.
    /// Pure.
    ///
    /// # Errors
    /// [`subdivide`]'s; `ShapeDegenerate` when the result does not build
    /// or a closed cage gives an open result.
    pub fn baked(&self, level: u8) -> Result<Object, ShapeError> {
        let mesh = subdivide(self, level)?;
        let mut patch = PolyPatch {
            verts: mesh.points.iter().map(|&p| PVert::New(p)).collect(),
            expect_watertight: self.watertight() == WatertightState::Watertight,
            ..PolyPatch::default()
        };
        let mut edges: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        // The source cage face of each patch face, for its dictionary.
        let mut sources: Vec<crate::ids::FaceId> = Vec::new();
        for (f, &src) in mesh.faces.iter().zip(&mesh.source) {
            let face = &self.faces()[src];
            let attrs = FaceAttrs {
                material: face.material,
                back_material: face.back_material,
                uv_frame: face.uv_frame,
                surface: None,
                blend: None,
                // Interned into the baked Object's own table below.
                attrs: crate::topo::AttrSlot::NONE,
            };
            let idx: Vec<usize> = f.iter().map(|&i| i as usize).collect();
            let pts: Vec<_> = idx.iter().map(|&i| mesh.points[i]).collect();
            let planar = Plane::from_polygon(&pts).is_ok_and(|pl| {
                pts.iter()
                    .all(|&p| pl.signed_distance(p).abs() <= tol::PLANE_DIST)
            });
            let polys: Vec<Vec<usize>> = if planar || idx.len() == 3 {
                vec![idx]
            } else {
                vec![vec![idx[0], idx[1], idx[2]], vec![idx[0], idx[2], idx[3]]]
            };
            for poly in polys {
                for k in 0..poly.len() {
                    *edges
                        .entry(key(poly[k], poly[(k + 1) % poly.len()]))
                        .or_default() += 1;
                }
                patch.add.push(PatchFace {
                    outer: poly,
                    holes: Vec::new(),
                    attrs,
                    plane: None,
                });
                sources.push(src);
            }
        }
        for (e, uses) in edges {
            let crease = mesh.creases.contains(&(e.0 as u32, e.1 as u32));
            patch.edge_meta.insert(
                e,
                EdgeMeta {
                    soft: !crease && uses == 2,
                    crease: crease && uses == 2,
                    ..EdgeMeta::default()
                },
            );
        }
        let built = Object::empty()
            .apply_patch(&patch)
            .map_err(|_| ShapeError::ShapeDegenerate)?;
        let mut out = built.object;
        // Each subdivided face keeps its cage face's dictionary.
        for (&nf, &src) in built.faces.iter().zip(&sources) {
            if let Some(dict) = self.face_attrs(src) {
                out.set_face_attrs(nf, dict.clone());
            }
        }
        out.planarity_tol = self.planarity_tol;
        out.default_material = self.default_material;
        out.validate().map_err(|_| ShapeError::ShapeDegenerate)?;
        Ok(out)
    }
}

// ==== true-curves: imports ====

impl Object {
    /// Marks soft every interior, non-creased, non-soft edge whose bend
    /// (the angle between its two faces' normals) lies strictly between
    /// `NORMAL_DIRECTION` and `max_angle` — an imported faceted surface
    /// then shades and selects as one smooth wall. Edges between coplanar
    /// faces (drawn shapes) and creased edges are left alone. Returns the
    /// softened object and how many edges changed. Pure.
    ///
    /// # Errors
    /// `AmountOutOfRange` unless `0 < max_angle < π`; `ShapeDegenerate` if
    /// the result fails validation.
    pub fn soften_by_angle(&self, max_angle: f64) -> Result<(Object, usize), ShapeError> {
        if !(max_angle > 0.0 && max_angle < std::f64::consts::PI) {
            return Err(ShapeError::AmountOutOfRange);
        }
        let mut out = self.clone();
        let mut count = 0;
        let ids: Vec<crate::ids::EdgeId> = self.edges().keys().collect();
        for e in ids {
            let edge = &self.edges()[e];
            let Some(twin) = edge.twin_half_edge else {
                continue;
            };
            if edge.soft || edge.crease {
                continue;
            }
            let face_of = |h| self.loops()[self.half_edges()[h].loop_id].face;
            let (na, nb) = (
                self.faces()[face_of(edge.half_edge)].plane.normal(),
                self.faces()[face_of(twin)].plane.normal(),
            );
            let bend = na.dot(nb).clamp(-1.0, 1.0).acos();
            if bend > tol::NORMAL_DIRECTION && bend < max_angle {
                out.edges[e].soft = true;
                count += 1;
            }
        }
        out.validate().map_err(|_| ShapeError::ShapeDegenerate)?;
        Ok((out, count))
    }
}

// ==== end true-curves: imports ====
