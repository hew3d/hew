//! The true-curves operations across the wasm boundary: blends, revolve,
//! deform, loft, shell, project, path arrays, Smooth and the import repairs.
//! A second `#[wasm_bindgen] impl Scene` block, kept out of `lib.rs` for
//! size; it reaches the handle's private `doc` like any child module.
//!
//! Boundary shapes follow the house rules: ids cross as `u64`, lists as
//! `Vec<u64>`, complex specs as one JSON string in the kernel's serde shape
//! (malformed JSON throws `BadSpec: …`), and reports as dedicated binding
//! classes with plain accessor methods.

#[allow(unused_imports)]
use wasm_bindgen::prelude::*;

#[allow(unused_imports)]
use crate::{ApiError, Scene, doc_err, recording};

use kernel::WatertightState;

// ==== true-curves: offset (Shell, Project) ====
#[wasm_bindgen]
impl Scene {
    /// The object Shell would make, as render buffers, without committing.
    /// `spec_json` is a `ShellSpec`: `{"open": [face ids], "thickness": m}`.
    /// Throws exactly what [`Scene::shell`] would: `BadSpec`,
    /// `UnknownObject`, `ShapeNeedsSolid`, `AmountOutOfRange`,
    /// `ShapeDegenerate`, `UnsupportedShellCorner`, `WallTooThick`,
    /// `SelfIntersecting`.
    pub fn shell_preview(&self, object: u64, spec_json: &str) -> Result<crate::MeshJs, ApiError> {
        let spec: kernel::shell::ShellSpec = parse_spec(spec_json)?;
        let after = self
            .doc
            .shell_preview(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        let mesh = tessellate::tessellate(&after, self.doc.materials())
            .map_err(|e| crate::api_err(&e, &e))?;
        Ok(crate::MeshJs {
            mesh,
            watertight: after.watertight() == WatertightState::Watertight,
        })
    }

    /// Hollows `object` (one undo step, "Shell"). Same spec and errors as
    /// [`Scene::shell_preview`]; recorded after success.
    pub fn shell(&mut self, object: u64, spec_json: &str) -> Result<crate::DocChangeJs, ApiError> {
        self.ensure_not_busy()?;
        let spec: kernel::shell::ShellSpec = parse_spec(spec_json)?;
        let change = self
            .doc
            .shell(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::Shell {
            object,
            spec: spec_json.to_owned(),
        });
        Ok(crate::DocChangeJs { inner: change })
    }
}
/// The imprint spec for `regions` of `sketch` (all when empty) onto `object`.
fn imprint_spec(
    scene: &Scene,
    object: u64,
    sketch: u64,
    regions: &[u64],
) -> Result<kernel::project::ImprintSpec, ApiError> {
    use slotmap::KeyData;
    let regions: Vec<kernel::SketchRegionId> = regions
        .iter()
        .map(|&r| kernel::SketchRegionId::from(KeyData::from_ffi(r)))
        .collect();
    scene
        .doc
        .project_spec_from_sketch(crate::object_id(object), crate::sketch_id(sketch), &regions)
        .map_err(doc_err)
}

/// The depth spec for `faces` of `object`, projected from `sketch`.
fn depth_spec(
    scene: &Scene,
    object: u64,
    sketch: u64,
    faces: &[u64],
    depth: f64,
    mode: u8,
) -> Result<kernel::project::DepthSpec, ApiError> {
    use slotmap::KeyData;
    let imprint = imprint_spec(scene, object, sketch, &[])?;
    let mode = match mode {
        0 => kernel::project::DepthMode::Straight,
        1 => kernel::project::DepthMode::Follow,
        _ => {
            return Err(ApiError::new(
                "BadSpec",
                "mode must be 0 (straight) or 1 (follow)",
            ));
        }
    };
    Ok(kernel::project::DepthSpec {
        faces: faces
            .iter()
            .map(|&f| kernel::FaceId::from(KeyData::from_ffi(f)))
            .collect(),
        dir: imprint.dir,
        depth,
        mode,
        regions: imprint.regions,
    })
}

#[wasm_bindgen]
impl Scene {
    /// The imprint Project would make of `regions` of `sketch` (every
    /// region when empty) on `object`, without committing: the region's
    /// boundary as line segments, xyz xyz per segment. The direction is
    /// the sketch plane's normal toward the object. Throws what
    /// [`Scene::project_imprint`] would: `UnknownObject`, `UnknownSketch`,
    /// `ProjectNeedsClosedShape`, `ProjectMisses`, `ShapeDegenerate` (the
    /// shape lines up exactly with an edge), `AmountOutOfRange`.
    pub fn project_imprint_preview(
        &self,
        object: u64,
        sketch: u64,
        regions: &[u64],
    ) -> Result<Vec<f32>, ApiError> {
        let spec = imprint_spec(self, object, sketch, regions)?;
        let (after, faces) = self
            .doc
            .project_imprint_preview(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        let region: std::collections::BTreeSet<kernel::FaceId> = faces.into_iter().collect();
        let mut out = Vec::new();
        for (e, edge) in after.edges() {
            let face_of = |h| after.loops()[after.half_edges()[h].loop_id].face;
            let sides: Vec<bool> = std::iter::once(edge.half_edge)
                .chain(edge.twin_half_edge)
                .map(|h| region.contains(&face_of(h)))
                .collect();
            if sides.iter().filter(|&&b| b).count() != 1 {
                continue;
            }
            if let Some((p, q)) = after.edge_endpoints(e) {
                out.extend([p.x, p.y, p.z, q.x, q.y, q.z].map(|c| c as f32));
            }
        }
        Ok(out)
    }

    /// Imprints the shape (one undo step, "Project") and returns the
    /// region's face ids, to pass to [`Scene::project_depth`].
    pub fn project_imprint(
        &mut self,
        object: u64,
        sketch: u64,
        regions: &[u64],
    ) -> Result<Vec<u64>, ApiError> {
        self.ensure_not_busy()?;
        use slotmap::Key;
        let spec = imprint_spec(self, object, sketch, regions)?;
        let (faces, change) = self
            .doc
            .project_imprint(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ProjectImprint {
            object,
            sketch,
            regions: regions.to_vec(),
        });
        Ok(faces.iter().map(|f| f.data().as_ffi()).collect())
    }

    /// The object a depth would make, as render buffers, without
    /// committing. `mode` 0 = straight (along the projection, can cut
    /// through), 1 = follow (along the surface). `depth > 0` engraves.
    /// Throws what [`Scene::project_depth`] would: the imprint's lookups,
    /// `BadSpec` (mode), `ShapeNeedsSolid`, `AmountOutOfRange` (no depth,
    /// or an engraving that removes everything), `ShapeDegenerate`,
    /// `WouldSever`, `SelfIntersecting`, `UnsupportedShellCorner`,
    /// `WallTooThick`, and straight mode's boolean refusals.
    pub fn project_depth_preview(
        &self,
        object: u64,
        sketch: u64,
        faces: &[u64],
        depth: f64,
        mode: u8,
    ) -> Result<crate::MeshJs, ApiError> {
        let spec = depth_spec(self, object, sketch, faces, depth, mode)?;
        let after = self
            .doc
            .project_depth_preview(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        let mesh = tessellate::tessellate(&after, self.doc.materials())
            .map_err(|e| crate::api_err(&e, &e))?;
        Ok(crate::MeshJs {
            mesh,
            watertight: after.watertight() == WatertightState::Watertight,
        })
    }

    /// Engraves or embosses the imprinted region (one undo step,
    /// "Project"). Same arguments and errors as
    /// [`Scene::project_depth_preview`].
    pub fn project_depth(
        &mut self,
        object: u64,
        sketch: u64,
        faces: &[u64],
        depth: f64,
        mode: u8,
    ) -> Result<crate::DocChangeJs, ApiError> {
        self.ensure_not_busy()?;
        let spec = depth_spec(self, object, sketch, faces, depth, mode)?;
        let change = self
            .doc
            .project_depth(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ProjectDepth {
            object,
            sketch,
            faces: faces.to_vec(),
            depth,
            mode,
        });
        Ok(crate::DocChangeJs { inner: change })
    }
}
// ==== end true-curves: offset ====

// ==== true-curves: smooth ====
#[wasm_bindgen]
impl Scene {
    /// Sets `object`'s Smooth level: 0 = Off, 1..=3 (one undo step).
    /// Throws `UnknownObject`, `UnknownComponent`, `AmountOutOfRange` (a
    /// level over 3) or `SmoothTooDense` (a cage too dense for that level).
    pub fn set_smooth_level(
        &mut self,
        object: u64,
        level: u8,
    ) -> Result<crate::DocChangeJs, ApiError> {
        self.ensure_not_busy()?;
        let change = self
            .doc
            .set_smooth_level(crate::object_id(object), level)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetSmoothLevel { object, level });
        Ok(crate::DocChangeJs { inner: change })
    }

    /// Whether `object`'s Smooth surface can be drawn at its level: false
    /// once the cage has grown too dense for it (the renderer then shows the
    /// cage, and the app says so). True for an object with no level.
    pub fn smooth_surface_available(&self, object: u64) -> bool {
        let id = crate::object_id(object);
        let level = self.doc.smooth_level(id);
        level == 0
            || self.doc.object(id).is_some_and(|o| {
                kernel::subdiv::subdivided_face_count(o, level) <= kernel::subdiv::MAX_SUBDIV_FACES
            })
    }

    /// `object`'s Smooth level (0 = Off; also 0 for a stale id).
    pub fn smooth_level(&self, object: u64) -> u8 {
        self.doc.smooth_level(crate::object_id(object))
    }

    /// Whether any visible object has a Smooth level (the export dialog's
    /// "Export cages instead" shows only then).
    pub fn has_smoothed_objects(&self) -> bool {
        self.doc.has_smoothed_objects()
    }

    /// Every visible object with a Smooth level: what a refresh that names
    /// no touched objects (an undo, a boolean) checks for a surface that can
    /// no longer be drawn, without reading the whole document.
    pub fn smoothed_object_ids(&self) -> Vec<u64> {
        self.doc
            .smoothed_object_ids()
            .into_iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Replaces a smoothed object's cage with its Smooth surface and
    /// clears the level, as one undo step ("Bake Smooth"). Throws
    /// `AmountOutOfRange` for an object with no level, `SmoothTooDense`
    /// for a cage past the surface cap at its level, `ShapeDegenerate`,
    /// `UnknownObject`, `UnknownComponent`.
    pub fn bake(&mut self, object: u64) -> Result<crate::DocChangeJs, ApiError> {
        self.ensure_not_busy()?;
        let change = self.doc.bake(crate::object_id(object)).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::Bake { object });
        Ok(crate::DocChangeJs { inner: change })
    }

    /// Sets or clears the crease flag on `edges` of `object` (kernel edge
    /// ids, already expanded to their logical edges), one undo step.
    /// Throws `CreaseOnSoftEdge`, `CreaseOnBoundary`, `ShapeDegenerate` (a
    /// stale edge), `UnknownObject`, `UnknownComponent`.
    pub fn set_crease(
        &mut self,
        object: u64,
        edges: &[u64],
        crease: bool,
    ) -> Result<crate::DocChangeJs, ApiError> {
        self.ensure_not_busy()?;
        use slotmap::KeyData;
        let ids: Vec<kernel::EdgeId> = edges
            .iter()
            .map(|&e| kernel::EdgeId::from(KeyData::from_ffi(e)))
            .collect();
        let change = self
            .doc
            .set_crease(crate::object_id(object), &ids, crease)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetCrease {
            object,
            edges: edges.to_vec(),
            crease,
        });
        Ok(crate::DocChangeJs { inner: change })
    }

    /// Whether `edge` of `object` is creased (false for stale ids).
    pub fn edge_is_creased(&self, object: u64, edge: u64) -> bool {
        use slotmap::KeyData;
        self.doc
            .object(crate::object_id(object))
            .and_then(|o| o.edges().get(kernel::EdgeId::from(KeyData::from_ffi(edge))))
            .is_some_and(|e| e.crease)
    }

    /// How `edge` of `object` reads: 0 = hard, 1 = tangent, 2 = soft seam
    /// (`Object::edge_smoothness`), 255 for a stale object or edge. The
    /// Eraser's crease click uses it to tell a soft seam from a sharp edge.
    pub fn edge_smoothness(&self, object: u64, edge: u64) -> u8 {
        use slotmap::KeyData;
        let eid = kernel::EdgeId::from(KeyData::from_ffi(edge));
        let Some(obj) = self.doc.object(crate::object_id(object)) else {
            return 255;
        };
        if obj.edges().get(eid).is_none() {
            return 255;
        }
        match obj.edge_smoothness(eid) {
            kernel::EdgeSmoothness::Hard => 0,
            kernel::EdgeSmoothness::Tangent => 1,
            kernel::EdgeSmoothness::Seam => 2,
        }
    }

    /// The edges of the logical edge containing `edge`, without tangent
    /// propagation (`Object::logical_edge(edge, false)`): what one crease
    /// click flags. Empty for a stale id or an edge that is not sharp.
    pub fn crease_edge_group(&self, object: u64, edge: u64) -> Vec<u64> {
        use slotmap::{Key, KeyData};
        let Some(obj) = self.doc.object(crate::object_id(object)) else {
            return Vec::new();
        };
        let eid = kernel::EdgeId::from(KeyData::from_ffi(edge));
        if obj.edges().get(eid).is_none() {
            return Vec::new();
        }
        obj.logical_edge(eid, false)
            .into_iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }
}
// ==== end true-curves: smooth ====

// ==== true-curves: imports ====
#[wasm_bindgen]
impl Scene {
    /// Softens `object`'s edges that bend less than `max_angle` radians
    /// (one undo step; nothing recorded when nothing changes). Returns how
    /// many edges were softened. Throws `AmountOutOfRange` unless
    /// `0 < max_angle < π`, `UnknownObject`, `UnknownComponent`.
    pub fn soften_by_angle(&mut self, object: u64, max_angle: f64) -> Result<u32, ApiError> {
        self.ensure_not_busy()?;
        let (n, change) = self
            .doc
            .soften_by_angle(crate::object_id(object), max_angle)
            .map_err(doc_err)?;
        if n > 0 {
            self.reconcile(&change);
            recording::record(recording::RecordedCall::SoftenByAngle { object, max_angle });
        }
        Ok(n as u32)
    }
}
// ==== end true-curves: imports ====

#[cfg(test)]
mod tests {
    use slotmap::{Key, KeyData};

    use super::*;

    /// A 1 m box on the ground; returns the object and its top face.
    fn unit_box(scene: &mut Scene) -> (u64, u64) {
        let sketch = scene.begin_ground_sketch().unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let mut region = None;
        for i in 0..4 {
            let ((ax, ay), (bx, by)) = (c[i], c[(i + 1) % 4]);
            let report = scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
            if let Some(&r) = report.inner.regions_created.first() {
                region = Some(r.data().as_ffi());
            }
        }
        let object = scene.extrude_region(sketch, region.unwrap(), 1.0).unwrap();
        let obj = scene
            .doc
            .object(kernel::ObjectId::from(KeyData::from_ffi(object)))
            .unwrap();
        let top = obj
            .faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.5)
            .map(|(id, _)| id.data().as_ffi())
            .unwrap();
        (object, top)
    }

    #[test]
    fn shell_records_and_replays_to_the_same_state() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let (object, top) = unit_box(&mut scene);
        let spec = format!(r#"{{"open":[{top}],"thickness":0.01}}"#);
        let preview = scene.shell_preview(object, &spec).unwrap();
        assert!(preview.watertight);
        scene.shell(object, &spec).unwrap();
        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(json.contains(r#""method":"shell""#));
        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
    }

    #[test]
    fn project_records_and_replays_to_the_same_state() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let (object, _) = unit_box(&mut scene);
        // A 0.2 square on a plane above the box, facing down.
        let sketch = scene
            .begin_sketch_on_plane(0.0, 0.0, 2.0, 0.0, 0.0, 1.0)
            .unwrap();
        let c = [(0.4, 0.4), (0.6, 0.4), (0.6, 0.6), (0.4, 0.6)];
        for i in 0..4 {
            let ((ax, ay), (bx, by)) = (c[i], c[(i + 1) % 4]);
            scene
                .sketch_add_segment(sketch, ax, ay, 2.0, bx, by, 2.0)
                .unwrap();
        }
        let lines = scene.project_imprint_preview(object, sketch, &[]).unwrap();
        assert_eq!(lines.len(), 4 * 6, "the square's four sides");
        let faces = scene.project_imprint(object, sketch, &[]).unwrap();
        assert_eq!(faces.len(), 1);
        let ghost = scene
            .project_depth_preview(object, sketch, &faces, 0.01, 0)
            .unwrap();
        assert!(ghost.watertight);
        scene
            .project_depth(object, sketch, &faces, 0.01, 0)
            .unwrap();
        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(json.contains(r#""method":"project_depth""#));
        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
    }

    #[test]
    fn smooth_crease_bake_record_and_replay() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let (object, _) = unit_box(&mut scene);
        let flat = scene.object_mesh(object).unwrap();
        assert!(flat.cage_edge_positions().is_empty());
        scene.set_smooth_level(object, 2).unwrap();
        assert_eq!(scene.smooth_level(object), 2);
        assert!(scene.has_smoothed_objects());
        let smooth = scene.object_mesh(object).unwrap();
        assert_eq!(
            smooth.cage_edge_positions().len(),
            12 * 6,
            "the cage's 12 edges"
        );
        assert!(
            smooth.positions().len() > flat.positions().len(),
            "the mesh is the smooth surface"
        );
        let obj = scene
            .doc
            .object(kernel::ObjectId::from(KeyData::from_ffi(object)))
            .unwrap();
        let bottom: Vec<u64> = obj
            .edges()
            .keys()
            .filter(|&e| {
                let (p, q) = obj.edge_endpoints(e).unwrap();
                p.z == 0.0 && q.z == 0.0
            })
            .map(|e| e.data().as_ffi())
            .collect();
        scene.set_crease(object, &bottom, true).unwrap();
        assert!(scene.edge_is_creased(object, bottom[0]));
        scene.bake(object).unwrap();
        assert_eq!(scene.smooth_level(object), 0);
        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
    }

    #[test]
    fn edge_smoothness_and_crease_group_read_a_box_and_its_softened_edges() {
        recording::reset();
        let mut scene = Scene::new();
        let (object, _) = unit_box(&mut scene);
        let edge_ids: Vec<u64> = scene
            .doc
            .object(kernel::ObjectId::from(KeyData::from_ffi(object)))
            .unwrap()
            .edges()
            .keys()
            .map(|e| e.data().as_ffi())
            .collect();
        assert_eq!(edge_ids.len(), 12);
        // A box edge is hard and is its own logical edge.
        assert_eq!(scene.edge_smoothness(object, edge_ids[0]), 0);
        assert_eq!(
            scene.crease_edge_group(object, edge_ids[0]),
            vec![edge_ids[0]]
        );
        // Stale ids read as nothing.
        assert_eq!(scene.edge_smoothness(object, u64::MAX), 255);
        assert!(scene.crease_edge_group(object, u64::MAX).is_empty());
        assert_eq!(scene.edge_smoothness(u64::MAX, edge_ids[0]), 255);
        // Softened, every edge is a seam (not creasable) and no longer sharp.
        assert_eq!(
            scene.soften_by_angle(object, 100f64.to_radians()).unwrap(),
            12
        );
        assert_eq!(scene.edge_smoothness(object, edge_ids[0]), 2);
        assert!(scene.crease_edge_group(object, edge_ids[0]).is_empty());
    }

    #[test]
    fn soften_records_and_replays() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let (object, _) = unit_box(&mut scene);
        // A box's edges all bend 90°: nothing under 45°, all twelve under 100°.
        assert_eq!(
            scene.soften_by_angle(object, 45f64.to_radians()).unwrap(),
            0
        );
        assert_eq!(
            scene.soften_by_angle(object, 100f64.to_radians()).unwrap(),
            12
        );
        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert_eq!(json.matches(r#""method":"soften_by_angle""#).count(), 1);
        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
    }

    #[test]
    fn shell_refuses_bad_json_and_bad_specs_typed() {
        let mut scene = Scene::new();
        let (object, _) = unit_box(&mut scene);
        let code = |e: ApiError| format!("{e:?}");
        assert!(code(scene.shell(object, "{").err().unwrap()).contains("BadSpec"));
        assert!(
            code(
                scene
                    .shell(object, r#"{"open":[],"thickness":0.0}"#)
                    .err()
                    .unwrap()
            )
            .contains("AmountOutOfRange")
        );
    }
}
/// Parses a JSON spec in its kernel serde shape; malformed input is
/// `BadSpec: <serde message>`.
pub(crate) fn parse_spec<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, ApiError> {
    serde_json::from_str(json).map_err(|e| ApiError::new("BadSpec", &e.to_string()))
}

// ── shapes: begin ─────────────────────────────────────────────────────
mod shapes {
    use kernel::deform::DeformSpec;
    use kernel::document::{LoftSource, RevolveProfile, RevolveSpec};
    use kernel::{FaceId, SketchRegionId, WatertightState};
    use slotmap::{Key, KeyData};
    use tessellate::tessellate;
    use wasm_bindgen::prelude::*;

    use super::parse_spec;
    use crate::{
        ApiError, DocChangeJs, MeshJs, Scene, api_err, doc_err, instance_id, object_id, recording,
        sketch_id,
    };

    /// Ids cross as decimal strings inside a profile JSON (a `u64` does not
    /// survive a JS number); plain numbers are accepted too.
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum JsonId {
        Str(String),
        Num(u64),
    }

    impl JsonId {
        fn get(&self) -> Result<u64, ApiError> {
            match self {
                JsonId::Num(n) => Ok(*n),
                JsonId::Str(s) => s
                    .parse()
                    .map_err(|_| ApiError::new("BadSpec", &format!("bad id {s:?}"))),
            }
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase", deny_unknown_fields)]
    enum ProfileJson {
        Region { sketch: JsonId, region: JsonId },
        Face { object: JsonId, face: JsonId },
    }

    fn parse_profile(json: &str) -> Result<RevolveProfile, ApiError> {
        Ok(match parse_spec::<ProfileJson>(json)? {
            ProfileJson::Region { sketch, region } => RevolveProfile::Region {
                sketch: sketch_id(sketch.get()?),
                region: SketchRegionId::from(KeyData::from_ffi(region.get()?)),
            },
            ProfileJson::Face { object, face } => RevolveProfile::Face {
                object: object_id(object.get()?),
                face: FaceId::from(KeyData::from_ffi(face.get()?)),
            },
        })
    }

    #[wasm_bindgen]
    impl Scene {
        /// Revolve preview (true-curves.md §5.1): the solid
        /// [`Scene::revolve`] would build, tessellated, without touching
        /// the document. `profile_json` is `{"region":{"sketch":id,
        /// "region":id}}` or `{"face":{"object":id,"face":id}}` (ids as
        /// decimal strings or numbers); `spec_json` is
        /// `{"axis_point":[x,y,z],"axis":[x,y,z],"angle":rad,"segments":n}`.
        /// Throws exactly the commit's codes: `BadSpec`, `UnknownSketch`,
        /// `UnknownRegion`, `UnknownObject`, `UnknownFace`,
        /// `AmountOutOfRange`, `AxisNotInProfilePlane`, `ProfileCrossesAxis`
        /// and Follow Me's other refusals.
        pub fn revolve_preview(
            &self,
            profile_json: &str,
            spec_json: &str,
        ) -> Result<MeshJs, ApiError> {
            let profile = parse_profile(profile_json)?;
            let spec: RevolveSpec = parse_spec(spec_json)?;
            let obj = self.doc.revolve_preview(&profile, &spec).map_err(doc_err)?;
            let mesh = tessellate(&obj, self.doc.materials()).map_err(|e| api_err(&e, &e))?;
            Ok(MeshJs {
                mesh,
                watertight: obj.watertight() == WatertightState::Watertight,
            })
        }

        /// Revolve (true-curves.md §5.1): turns a world region or face about
        /// an axis in its plane into a new world object; returns its id. A
        /// region profile is consumed (one undo step restores it). Inputs
        /// and codes as [`Scene::revolve_preview`]. Recorded.
        pub fn revolve(&mut self, profile_json: &str, spec_json: &str) -> Result<u64, ApiError> {
            let profile = parse_profile(profile_json)?;
            let spec: RevolveSpec = parse_spec(spec_json)?;
            let (id, change) = self.doc.revolve(profile, &spec).map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::Revolve {
                profile: profile_json.to_owned(),
                spec: spec_json.to_owned(),
            });
            Ok(id.data().as_ffi())
        }

        /// [`Scene::revolve`] with a face of an object inside a component
        /// definition, edited through `instance`; `spec_json` is in WORLD
        /// coordinates and mapped through the instance pose. The new object
        /// joins the definition; returns its id. Also throws
        /// `AmbiguousInstanceScale` for a non-similarity pose. Recorded.
        pub fn revolve_in_instance(
            &mut self,
            instance: u64,
            object: u64,
            face: u64,
            spec_json: &str,
        ) -> Result<u64, ApiError> {
            let spec: RevolveSpec = parse_spec(spec_json)?;
            let (id, change) = self
                .doc
                .revolve_in_instance(
                    instance_id(instance),
                    object_id(object),
                    FaceId::from(KeyData::from_ffi(face)),
                    &spec,
                )
                .map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::RevolveInInstance {
                instance,
                object,
                face,
                spec: spec_json.to_owned(),
            });
            Ok(id.data().as_ffi())
        }
    }

    #[wasm_bindgen]
    impl Scene {
        /// Deform preview (true-curves.md §5.3): `object` deformed by
        /// `spec_json` (a `kernel::DeformSpec`: `{"mode":"twist"|"bend"|
        /// "taper","frame":{"origin":[..],"axis":[..],"u":[..],"length":L},
        /// "angle"|"ratio":x,"stations":n?}` or `{"mode":"bend_to_path",
        /// "base_from":[..],"base_to":[..],"path":[[..],..],"closed":b?,
        /// "stations_per_segment":n?}`), tessellated, without touching the
        /// document. Throws the commit's codes: `BadSpec`, `UnknownObject`,
        /// `ShapeNeedsSolid`, `AmountOutOfRange`, `SelfIntersecting` (with
        /// no limit: ask [`Scene::deform_preview_limit`] once), and
        /// `ShapeDegenerate`; plus `PreviewTooLarge` when the object is too
        /// heavy to preview (a capped amount of work; [`Scene::deform`]
        /// still applies it).
        pub fn deform_preview(&self, object: u64, spec_json: &str) -> Result<MeshJs, ApiError> {
            let spec: DeformSpec = parse_spec(spec_json)?;
            let obj = self
                .doc
                .deform_preview(object_id(object), &spec)
                .map_err(doc_err)?;
            let mesh = tessellate(&obj, self.doc.materials()).map_err(|e| api_err(&e, &e))?;
            Ok(MeshJs {
                mesh,
                watertight: obj.watertight() == WatertightState::Watertight,
            })
        }

        /// The largest amount (same sign as the spec's angle or ratio) that
        /// [`Scene::deform`] accepts for `spec_json` — the tool clamps its
        /// drag to it after a `SelfIntersecting` preview. Not available for
        /// Bend to Path (`AmountOutOfRange`).
        pub fn deform_limit(&self, object: u64, spec_json: &str) -> Result<f64, ApiError> {
            let spec: DeformSpec = parse_spec(spec_json)?;
            self.doc
                .deform_limit(object_id(object), &spec)
                .map_err(doc_err)
        }

        /// [`Scene::deform_limit`] under a preview's work cap, for the clamp
        /// of a live drag: the same answer when it answers, or
        /// `PreviewTooLarge` when the search would take too long.
        pub fn deform_preview_limit(&self, object: u64, spec_json: &str) -> Result<f64, ApiError> {
            let spec: DeformSpec = parse_spec(spec_json)?;
            self.doc
                .deform_preview_limit(object_id(object), &spec)
                .map_err(doc_err)
        }

        /// Deform (true-curves.md §5.3): reshapes `object` in place as one
        /// undo step. Inputs and codes as [`Scene::deform_preview`].
        /// Recorded.
        pub fn deform(&mut self, object: u64, spec_json: &str) -> Result<DocChangeJs, ApiError> {
            let spec: DeformSpec = parse_spec(spec_json)?;
            let change = self.doc.deform(object_id(object), &spec).map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::Deform {
                object,
                spec: spec_json.to_owned(),
            });
            Ok(DocChangeJs { inner: change })
        }
    }

    /// A Loft spec across the boundary (true-curves.md §5.4).
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LoftJson {
        sources: Vec<SourceJson>,
        #[serde(default)]
        rails: Vec<RailJson>,
        #[serde(default)]
        start_offsets: Vec<i32>,
        #[serde(default)]
        stations: Option<u32>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase", deny_unknown_fields)]
    enum SourceJson {
        Region { sketch: JsonId, region: JsonId },
        Face { object: JsonId, face: JsonId },
        Vertex([f64; 3]),
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase", deny_unknown_fields)]
    enum RailJson {
        Edges { sketch: JsonId, edges: Vec<JsonId> },
        Face { object: JsonId, face: JsonId },
    }

    type LoftArgs = (
        Vec<LoftSource>,
        Vec<kernel::FollowMePath>,
        Vec<i32>,
        Option<u32>,
    );

    fn parse_loft(json: &str) -> Result<LoftArgs, ApiError> {
        let spec: LoftJson = parse_spec(json)?;
        let mut sources = Vec::with_capacity(spec.sources.len());
        for s in spec.sources {
            sources.push(match s {
                SourceJson::Region { sketch, region } => LoftSource::Region {
                    sketch: sketch_id(sketch.get()?),
                    region: SketchRegionId::from(KeyData::from_ffi(region.get()?)),
                },
                SourceJson::Face { object, face } => LoftSource::Face {
                    object: object_id(object.get()?),
                    face: FaceId::from(KeyData::from_ffi(face.get()?)),
                },
                SourceJson::Vertex([x, y, z]) => LoftSource::Vertex {
                    point: kernel::Point3::new(x, y, z),
                },
            });
        }
        let mut rails = Vec::with_capacity(spec.rails.len());
        for r in spec.rails {
            rails.push(match r {
                RailJson::Edges { sketch, edges } => kernel::FollowMePath::SketchEdges {
                    sketch: sketch_id(sketch.get()?),
                    edges: edges
                        .iter()
                        .map(|e| {
                            e.get()
                                .map(|e| kernel::SketchEdgeId::from(KeyData::from_ffi(e)))
                        })
                        .collect::<Result<_, _>>()?,
                },
                RailJson::Face { object, face } => kernel::FollowMePath::FaceLoop {
                    object: object_id(object.get()?),
                    face: FaceId::from(KeyData::from_ffi(face.get()?)),
                },
            });
        }
        Ok((sources, rails, spec.start_offsets, spec.stations))
    }

    #[wasm_bindgen]
    impl Scene {
        /// Loft preview (true-curves.md §5.4): the solid [`Scene::loft`]
        /// would build, tessellated, without touching the document.
        /// `spec_json` is `{"sources":[{"region":{"sketch":id,"region":id}} |
        /// {"face":{"object":id,"face":id}} | {"vertex":[x,y,z]}],
        /// "rails":[{"edges":{"sketch":id,"edges":[id..]}} |
        /// {"face":{"object":id,"face":id}}]?, "start_offsets":[n..]?,
        /// "stations":n?}` — a vertex only last (the apex). Throws `BadSpec`,
        /// the stale-handle codes, `LoftProfileHasHole`,
        /// `LoftNeedsTwoProfiles`, `LoftTooManyRails`, `RailMissesProfile`,
        /// `AmountOutOfRange`, `ShapeDegenerate`, `SelfIntersecting` and a
        /// rail's Follow Me path codes.
        pub fn loft_preview(&self, spec_json: &str) -> Result<MeshJs, ApiError> {
            let (sources, rails, offsets, stations) = parse_loft(spec_json)?;
            let obj = self
                .doc
                .loft_preview(&sources, &rails, &offsets, stations)
                .map_err(doc_err)?;
            let mesh = tessellate(&obj, self.doc.materials()).map_err(|e| api_err(&e, &e))?;
            Ok(MeshJs {
                mesh,
                watertight: obj.watertight() == WatertightState::Watertight,
            })
        }

        /// Loft (true-curves.md §5.4): a new world object through the
        /// sources in order; the profiles stay as they are. Returns its id.
        /// Inputs and codes as [`Scene::loft_preview`]. Recorded.
        pub fn loft(&mut self, spec_json: &str) -> Result<u64, ApiError> {
            let (sources, rails, offsets, stations) = parse_loft(spec_json)?;
            let (id, change) = self
                .doc
                .loft(&sources, &rails, &offsets, stations)
                .map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::Loft {
                spec: spec_json.to_owned(),
            });
            Ok(id.data().as_ffi())
        }

        /// The loft's start markers: each profile's start candidates in loft
        /// order from its automatic start (its corners where every profile
        /// has the same number of corners, else its original vertices; for
        /// a whole circle started between vertices, those turned onto its
        /// columns), as `[n0, x, y, z, …, n1, x, y, z, …]` (a count, then
        /// the points). A `start_offsets` entry counts along this list.
        /// The list starts at the automatic start whatever `start_offsets`
        /// `spec_json` carries (they are read and checked, and change
        /// nothing here); rails and stations are ignored.
        pub fn loft_start_points(&self, spec_json: &str) -> Result<Vec<f64>, ApiError> {
            let (sources, _, offsets, _) = parse_loft(spec_json)?;
            let rings = self
                .doc
                .loft_start_points(&sources, &offsets)
                .map_err(doc_err)?;
            let mut out = Vec::new();
            for ring in rings {
                out.push(ring.len() as f64);
                for p in ring {
                    out.extend([p.x, p.y, p.z]);
                }
            }
            Ok(out)
        }
    }

    /// Follow Me options from the wasm twins' plain arguments
    /// (`stations == 0` = automatic).
    fn sweep_options(twist: f64, end_scale: f64, stations: u32) -> kernel::SweepOptions {
        kernel::SweepOptions {
            twist,
            end_scale,
            stations: (stations > 0).then_some(stations),
        }
    }

    #[wasm_bindgen]
    impl Scene {
        /// [`Scene::follow_me_along_edges`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_along_edges_with(
            &mut self,
            sketch: u64,
            region: u64,
            path_sketch: u64,
            path_edges: Vec<u64>,
            stop_len: Option<f64>,
            group: Option<u64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_along_edges_opts(
                sketch,
                region,
                path_sketch,
                path_edges,
                stop_len,
                group,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_around_face`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_around_face_with(
            &mut self,
            sketch: u64,
            region: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            group: Option<u64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_around_face_opts(
                sketch,
                region,
                path_object,
                path_face,
                stop_len,
                group,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_merged_around_face`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_merged_around_face_with(
            &mut self,
            sketch: u64,
            region: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_merged_around_face_opts(
                sketch,
                region,
                path_object,
                path_face,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_face_along_edges`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_face_along_edges_with(
            &mut self,
            profile_object: u64,
            profile_face: u64,
            path_sketch: u64,
            path_edges: Vec<u64>,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_face_along_edges_opts(
                profile_object,
                profile_face,
                path_sketch,
                path_edges,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_face_around_face`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_face_around_face_with(
            &mut self,
            profile_object: u64,
            profile_face: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_face_around_face_opts(
                profile_object,
                profile_face,
                path_object,
                path_face,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_around_instance_face`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_around_instance_face_with(
            &mut self,
            sketch: u64,
            region: u64,
            instance: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_around_instance_face_opts(
                sketch,
                region,
                instance,
                path_object,
                path_face,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_along_edges_in_instance`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_along_edges_in_instance_with(
            &mut self,
            instance: u64,
            sketch: u64,
            region: u64,
            path_sketch: u64,
            path_edges: Vec<u64>,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_along_edges_in_instance_opts(
                instance,
                sketch,
                region,
                path_sketch,
                path_edges,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_around_face_in_instance`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_around_face_in_instance_with(
            &mut self,
            instance: u64,
            sketch: u64,
            region: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_around_face_in_instance_opts(
                instance,
                sketch,
                region,
                path_object,
                path_face,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_merged_around_face_in_instance`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_merged_around_face_in_instance_with(
            &mut self,
            instance: u64,
            sketch: u64,
            region: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_merged_around_face_in_instance_opts(
                instance,
                sketch,
                region,
                path_object,
                path_face,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_face_along_edges_in_instance`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_face_along_edges_in_instance_with(
            &mut self,
            instance: u64,
            profile_object: u64,
            profile_face: u64,
            path_sketch: u64,
            path_edges: Vec<u64>,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_face_along_edges_in_instance_opts(
                instance,
                profile_object,
                profile_face,
                path_sketch,
                path_edges,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }

        /// [`Scene::follow_me_face_around_face_in_instance`] with Follow Me twist (radians, right-hand
        /// about the path), end scale (ratio, 1 = none) and stations per
        /// path segment (0 = automatic, one per 15° of twist); true-curves.md
        /// §5.2. Also throws `TwistNeedsOpenPath`. Recorded.
        #[allow(clippy::too_many_arguments)]
        pub fn follow_me_face_around_face_in_instance_with(
            &mut self,
            instance: u64,
            profile_object: u64,
            profile_face: u64,
            path_object: u64,
            path_face: u64,
            stop_len: Option<f64>,
            twist: f64,
            end_scale: f64,
            stations: u32,
        ) -> Result<u64, ApiError> {
            self.follow_me_face_around_face_in_instance_opts(
                instance,
                profile_object,
                profile_face,
                path_object,
                path_face,
                stop_len,
                sweep_options(twist, end_scale, stations),
            )
        }
    }

    #[cfg(test)]
    mod tests {
        use crate::{Scene, recording};

        /// A 0.5 × 1 rectangle in the XZ plane with one side on the Z axis.
        fn axis_rectangle(scene: &mut Scene) -> (u64, u64) {
            let sketch = scene
                .begin_sketch_on_plane(0.0, 0.0, 0.0, 0.0, -1.0, 0.0)
                .unwrap();
            let pts = [(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)];
            let mut region = None;
            for i in 0..4 {
                let (a, b) = (pts[i], pts[(i + 1) % 4]);
                let report = scene
                    .sketch_add_segment(sketch, a.0, 0.0, a.1, b.0, 0.0, b.1)
                    .unwrap();
                if let Some(&r) = report.inner.regions_created.first() {
                    region = Some(slotmap::Key::data(&r).as_ffi());
                }
            }
            (sketch, region.expect("the rectangle closes a region"))
        }

        const SPEC: &str =
            r#"{"axis_point":[0,0,0],"axis":[0,0,1],"angle":6.283185307179586,"segments":24}"#;

        fn profile(sketch: u64, region: u64) -> String {
            format!(r#"{{"region":{{"sketch":"{sketch}","region":"{region}"}}}}"#)
        }

        #[test]
        fn revolve_preview_is_watertight_and_leaves_the_document_alone() {
            let mut scene = Scene::new();
            let (sketch, region) = axis_rectangle(&mut scene);
            let before = scene.state_hash();
            let mesh = scene
                .revolve_preview(&profile(sketch, region), SPEC)
                .unwrap();
            assert!(mesh.watertight);
            assert!(!mesh.positions().is_empty());
            assert_eq!(scene.state_hash(), before);
        }

        #[test]
        fn revolve_malformed_spec_is_bad_spec() {
            let mut scene = Scene::new();
            let (sketch, region) = axis_rectangle(&mut scene);
            let err = scene.revolve(&profile(sketch, region), "{").unwrap_err();
            assert!(err.0.starts_with("BadSpec: "), "{}", err.0);
            let err = scene.revolve(r#"{"region":{"sketch":"x","region":"1"}}"#, SPEC);
            assert!(err.unwrap_err().0.starts_with("BadSpec: "));
            let off = r#"{"axis_point":[0,0,0],"axis":[0,1,0],"angle":1,"segments":24}"#;
            let err = scene.revolve(&profile(sketch, region), off).unwrap_err();
            assert!(err.0.starts_with("AxisNotInProfilePlane: "), "{}", err.0);
        }

        #[test]
        fn twisted_follow_me_records_and_replays() {
            recording::reset();
            let mut scene = Scene::new();
            scene.start_recording();
            let (sketch, region) = axis_rectangle(&mut scene);
            // A straight path along -y, leaving the XZ profile plane.
            let path = scene.begin_ground_sketch().unwrap();
            let report = scene
                .sketch_add_segment(path, 0.25, 0.0, 0.0, 0.25, -2.0, 0.0)
                .unwrap();
            let edges: Vec<u64> = report
                .inner
                .new_edges
                .iter()
                .map(|e| slotmap::Key::data(e).as_ffi())
                .collect();
            let id = scene
                .follow_me_along_edges_with(sketch, region, path, edges, None, None, 0.8, 0.5, 0)
                .unwrap();
            assert!(scene.object_ids().contains(&id));
            scene.stop_recording();
            let golden = scene.state_hash();
            let json = scene.take_recording();
            assert!(json.contains(r#""twist":0.8"#), "{json}");
            assert!(json.contains(r#""end_scale":0.5"#), "{json}");

            let mut replayed = Scene::new();
            assert_eq!(replayed.replay(&json).unwrap(), golden);
            assert_eq!(replayed.save(), scene.save());
        }

        #[test]
        fn plain_follow_me_records_no_sweep_fields() {
            recording::reset();
            let mut scene = Scene::new();
            scene.start_recording();
            let (sketch, region) = axis_rectangle(&mut scene);
            let path = scene.begin_ground_sketch().unwrap();
            let report = scene
                .sketch_add_segment(path, 0.25, 0.0, 0.0, 0.25, -2.0, 0.0)
                .unwrap();
            let edges: Vec<u64> = report
                .inner
                .new_edges
                .iter()
                .map(|e| slotmap::Key::data(e).as_ffi())
                .collect();
            scene
                .follow_me_along_edges(sketch, region, path, edges, None, None)
                .unwrap();
            scene.stop_recording();
            let json = scene.take_recording();
            assert!(
                !json.contains("twist") && !json.contains("end_scale"),
                "{json}"
            );
        }

        #[test]
        fn twist_on_a_closed_path_is_twist_needs_open_path() {
            let mut scene = Scene::new();
            let (sketch, region) = axis_rectangle(&mut scene);
            let path = scene.begin_ground_sketch().unwrap();
            let pts = [(2.0, -1.0), (3.0, -1.0), (3.0, 1.0), (2.0, 1.0)];
            let mut edges = Vec::new();
            for i in 0..4 {
                let (a, b) = (pts[i], pts[(i + 1) % 4]);
                let r = scene
                    .sketch_add_segment(path, a.0, a.1, 0.0, b.0, b.1, 0.0)
                    .unwrap();
                edges.extend(
                    r.inner
                        .new_edges
                        .iter()
                        .map(|e| slotmap::Key::data(e).as_ffi()),
                );
            }
            let err = scene
                .follow_me_along_edges_with(sketch, region, path, edges, None, None, 0.5, 1.0, 0)
                .unwrap_err();
            assert!(err.0.starts_with("TwistNeedsOpenPath: "), "{}", err.0);
        }

        /// The rectangle extruded 0.5 along −y: a box x 0..0.5, z 0..1.
        fn deform_box(scene: &mut Scene) -> u64 {
            let (sketch, region) = axis_rectangle(scene);
            scene.extrude_region(sketch, region, 0.5).unwrap()
        }

        const TWIST: &str = r#"{"mode":"twist","frame":{"origin":[0.25,-0.25,0],"axis":[0,0,1],"u":[1,0,0],"length":1},"angle":0.8}"#;

        #[test]
        fn deform_records_and_replays_and_previews_cleanly() {
            recording::reset();
            let mut scene = Scene::new();
            scene.start_recording();
            let id = deform_box(&mut scene);
            let before = scene.state_hash();
            let mesh = scene.deform_preview(id, TWIST).unwrap();
            assert!(mesh.watertight);
            assert_eq!(scene.state_hash(), before);
            assert_eq!(scene.deform_limit(id, TWIST).unwrap(), 0.8);
            let change = scene.deform(id, TWIST).unwrap();
            assert!(change.objects_touched().contains(&id));
            scene.stop_recording();
            let golden = scene.state_hash();
            let json = scene.take_recording();
            assert!(json.contains(r#""method":"deform""#), "{json}");
            let mut replayed = Scene::new();
            assert_eq!(replayed.replay(&json).unwrap(), golden);
            assert_eq!(replayed.save(), scene.save());
        }

        #[test]
        fn deform_too_far_reports_the_limit() {
            let mut scene = Scene::new();
            let id = deform_box(&mut scene);
            let bend = r#"{"mode":"bend","frame":{"origin":[0.25,-0.25,0],"axis":[0,0,1],"u":[1,0,0],"length":1},"angle":6}"#;
            let Err(err) = scene.deform_preview(id, bend) else {
                panic!("refused")
            };
            // The preview does not search for the limit (that is the
            // drag's one `deform_preview_limit` call); the commit's
            // refusal names it.
            assert!(err.0.starts_with("SelfIntersecting: "), "{}", err.0);
            assert!(!err.0.contains("limit"), "{}", err.0);
            let limit = scene.deform_limit(id, bend).unwrap();
            assert!(limit > 0.0 && limit < 6.0);
            assert_eq!(scene.deform_preview_limit(id, bend).unwrap(), limit);
            let before = scene.state_hash();
            let Err(err) = scene.deform(id, bend) else {
                panic!("refused")
            };
            assert!(err.0.starts_with("SelfIntersecting: "), "{}", err.0);
            assert!(err.0.contains(&format!("(limit {limit})")), "{}", err.0);
            assert_eq!(scene.state_hash(), before);
            let Err(err) = scene.deform(id, "{\"mode\":\"spin\"}") else {
                panic!("refused")
            };
            assert!(err.0.starts_with("BadSpec: "), "{}", err.0);
        }

        /// A square of half-width `h` on the plane z = `z`.
        fn square_at(scene: &mut Scene, h: f64, z: f64) -> (u64, u64) {
            let sketch = scene
                .begin_sketch_on_plane(0.0, 0.0, z, 0.0, 0.0, 1.0)
                .unwrap();
            let c = [(-h, -h), (h, -h), (h, h), (-h, h)];
            let mut region = None;
            for i in 0..4 {
                let (a, b) = (c[i], c[(i + 1) % 4]);
                let r = scene
                    .sketch_add_segment(sketch, a.0, a.1, z, b.0, b.1, z)
                    .unwrap();
                if let Some(&id) = r.inner.regions_created.first() {
                    region = Some(slotmap::Key::data(&id).as_ffi());
                }
            }
            (sketch, region.unwrap())
        }

        #[test]
        fn loft_records_and_replays() {
            recording::reset();
            let mut scene = Scene::new();
            scene.start_recording();
            let (s0, r0) = square_at(&mut scene, 0.5, 0.0);
            let (s1, r1) = square_at(&mut scene, 0.25, 1.0);
            let spec = format!(
                r#"{{"sources":[{{"region":{{"sketch":"{s0}","region":"{r0}"}}}},{{"region":{{"sketch":"{s1}","region":"{r1}"}}}}]}}"#
            );
            let before = scene.state_hash();
            assert!(scene.loft_preview(&spec).unwrap().watertight);
            assert_eq!(scene.state_hash(), before);
            let starts = scene.loft_start_points(&spec).unwrap();
            assert_eq!(starts[0], 4.0);
            let id = scene.loft(&spec).unwrap();
            assert!(scene.object_ids().contains(&id));
            scene.stop_recording();
            let golden = scene.state_hash();
            let json = scene.take_recording();
            assert!(json.contains(r#""method":"loft""#), "{json}");
            let mut replayed = Scene::new();
            assert_eq!(replayed.replay(&json).unwrap(), golden);
            assert_eq!(replayed.save(), scene.save());
        }

        /// Three rectangles of different proportions (the S2 deck), each
        /// drawn with a point in the middle of one side, offer their four
        /// corners as start points each, not their five vertices (D-126).
        #[test]
        fn loft_start_points_are_the_corners_of_rectangles() {
            let mut scene = Scene::new();
            let mut src = Vec::new();
            for (w, h, z) in [(0.5, 0.3, 0.0), (0.6, 0.4, 0.5), (0.55, 0.35, 1.0)] {
                let s = scene
                    .begin_sketch_on_plane(0.0, 0.0, z, 0.0, 0.0, 1.0)
                    .unwrap();
                let c = [(w, -h), (w, h), (0.0, h), (-w, h), (-w, -h)];
                let mut region = None;
                for i in 0..c.len() {
                    let (a, b) = (c[i], c[(i + 1) % c.len()]);
                    let r = scene
                        .sketch_add_segment(s, a.0, a.1, z, b.0, b.1, z)
                        .unwrap();
                    if let Some(&id) = r.inner.regions_created.first() {
                        region = Some(slotmap::Key::data(&id).as_ffi());
                    }
                }
                let r = region.unwrap();
                src.push(format!(r#"{{"region":{{"sketch":"{s}","region":"{r}"}}}}"#));
            }
            let spec = format!(r#"{{"sources":[{}]}}"#, src.join(","));
            let flat = scene.loft_start_points(&spec).unwrap();
            let mut i = 0;
            let mut counts = Vec::new();
            while i < flat.len() {
                let n = flat[i] as usize;
                counts.push(n);
                for k in 0..n {
                    let (x, y) = (flat[i + 1 + 3 * k], flat[i + 2 + 3 * k]);
                    assert!(x.abs() > 0.49 && y.abs() > 0.29, "a corner: {x}, {y}");
                }
                i += 1 + 3 * n;
            }
            assert_eq!(counts, vec![4, 4, 4]);
        }

        #[test]
        fn loft_of_one_profile_is_loft_needs_two_profiles() {
            let mut scene = Scene::new();
            let (s0, r0) = square_at(&mut scene, 0.5, 0.0);
            let spec =
                format!(r#"{{"sources":[{{"region":{{"sketch":"{s0}","region":"{r0}"}}}}]}}"#);
            let err = scene.loft(&spec).unwrap_err();
            assert!(err.0.starts_with("LoftNeedsTwoProfiles: "), "{}", err.0);
            let err = scene.loft(r#"{"sources":[{"cone":1}]}"#).unwrap_err();
            assert!(err.0.starts_with("BadSpec: "), "{}", err.0);
        }

        #[test]
        fn revolve_records_and_replays() {
            recording::reset();
            let mut scene = Scene::new();
            scene.start_recording();
            let (sketch, region) = axis_rectangle(&mut scene);
            let id = scene.revolve(&profile(sketch, region), SPEC).unwrap();
            assert!(scene.object_ids().contains(&id));
            scene.stop_recording();
            let golden = scene.state_hash();
            let json = scene.take_recording();
            assert!(json.contains(r#""method":"revolve""#), "{json}");

            let mut replayed = Scene::new();
            assert_eq!(replayed.replay(&json).unwrap(), golden);
            assert_eq!(replayed.save(), scene.save());
        }
    }
}
// ── shapes: end ───────────────────────────────────────────────────────

use kernel::{CornerProfile, Point3};
use slotmap::Key;

/// Parses a `CornerProfile` from its JSON: `{"kind":"round","radius":r,
/// "segments":n}` or `{"kind":"chamfer","setback_a":a,"setback_b":b}`.
/// Malformed input throws `BadSpec: <serde message>`.
pub(crate) fn parse_corner_profile(json: &str) -> Result<CornerProfile, ApiError> {
    serde_json::from_str(json).map_err(|e| ApiError(format!("BadSpec: {e}")))
}

/// `profile` with its lengths multiplied by `k` (a pose's inverse scale).
pub(crate) fn scale_corner_profile(profile: CornerProfile, k: f64) -> CornerProfile {
    match profile {
        CornerProfile::Round { radius, segments } => CornerProfile::Round {
            radius: radius * k,
            segments,
        },
        CornerProfile::Chamfer {
            setback_a,
            setback_b,
        } => CornerProfile::Chamfer {
            setback_a: setback_a * k,
            setback_b: setback_b * k,
        },
    }
}

// ==== true-curves: blend ====
#[wasm_bindgen]
impl Scene {
    /// Rounds or chamfers the corner at `vertex` of world sketch `sketch`
    /// (`kernel::Sketch::blend_corner`). `profile_json` is a
    /// `kernel::CornerProfile`: `{"kind":"round","radius":r,"segments":n}`
    /// (`r` in meters, `n` in 1..=1024) or
    /// `{"kind":"chamfer","setback_a":a,"setback_b":b}` (meters; `a` runs
    /// along the edge that starts the counter-clockwise order seen from the
    /// sketch plane's normal side).
    ///
    /// Contract. Mutates only that sketch: the two edges at the corner are
    /// replaced by trimmed lines plus the arc (a new circle chain with its
    /// exact center and radius) or the chamfer segment; regions re-derive.
    /// The call is its own undo step: do not bracket it in a sketch
    /// gesture. A refusal changes nothing and records nothing.
    ///
    /// # Errors
    /// `BadSpec` (malformed profile JSON); `UnknownSketch`;
    /// `SketchGestureAlreadyOpen`; `UnknownVertex`; `CornerNotBlendable`
    /// (not exactly two edges, or one is an arc, circle or polygon side);
    /// `CornerIsStraight`; `CornerRadiusTooLarge` (its message carries the
    /// largest size that fits); `SegmentsBelowFloor` / `SegmentsAboveCap`
    /// (segments outside 1..=1024); `DegenerateCurve` (radius not a
    /// positive finite length); `DegenerateSegment` (a setback likewise).
    pub fn sketch_blend_corner(
        &mut self,
        sketch: u64,
        vertex: u64,
        profile_json: &str,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let profile = parse_corner_profile(profile_json)?;
        let (_report, change) = self
            .doc
            .sketch_blend_corner(
                crate::sketch_id(sketch),
                crate::sketch_vertex_id(vertex),
                profile,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SketchBlendCorner {
            sketch,
            vertex,
            profile: profile_json.to_owned(),
        });
        Ok(())
    }

    /// The blendable corner of world sketch `sketch` nearest `(x,y,z)`
    /// within `aperture` meters, as its vertex handle, or `None`. A
    /// blendable corner is a vertex with exactly two straight, non-curve,
    /// non-collinear edges: the corners [`Scene::sketch_blend_corner`]
    /// accepts. For hover highlighting; read-only.
    ///
    /// # Errors
    /// `UnknownSketch`; `BadVector` for a non-finite point or aperture.
    pub fn sketch_blendable_corner_at(
        &self,
        sketch: u64,
        x: f64,
        y: f64,
        z: f64,
        aperture: f64,
    ) -> Result<Option<u64>, ApiError> {
        if ![x, y, z, aperture].iter().all(|v| v.is_finite()) {
            return Err(ApiError::new(
                "BadVector",
                "the point and aperture must be finite",
            ));
        }
        let s = self
            .doc
            .sketch(crate::sketch_id(sketch))
            .ok_or_else(|| crate::stale("UnknownSketch", "sketch"))?;
        let at = Point3::new(x, y, z);
        let mut best: Option<(f64, kernel::SketchVertexId)> = None;
        for v in s.blendable_corners() {
            let d = (s.vertices()[v].position - at).length();
            if d <= aperture && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, v));
            }
        }
        Ok(best.map(|(_, v)| v.data().as_ffi()))
    }

    /// Paints every face of `face`'s logical face — a whole fillet, a whole
    /// cylinder wall — with `material` (`u64::MAX` clears), as one undo step
    /// (`kernel::Document::paint_logical_face`). The Paint tool's click.
    ///
    /// # Errors
    /// `UnknownObject`, `UnknownFace`, `UnknownMaterial`.
    pub fn paint_logical_face(
        &mut self,
        object: u64,
        face: u64,
        material: u64,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let oid = crate::object_id(object);
        let fid = kernel::FaceId::from(slotmap::KeyData::from_ffi(face));
        let mid = crate::material_id_opt(material);
        let change = self
            .doc
            .paint_logical_face(oid, fid, mid)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::PaintLogicalFace {
            object,
            face,
            material,
        });
        Ok(())
    }
}

/// What [`Scene::pick_solid_edge`] found: the sharp edge nearest the ray's
/// hit on a face (or, with `smooth()`, a smooth edge the cursor is on).
/// Coordinates are in the object's local frame; for an instance pick the
/// caller maps them through the instance pose, like `face_boundary`.
#[wasm_bindgen]
pub struct EdgePickJs {
    object: u64,
    instance: Option<u64>,
    edge: u64,
    point: [f64; 3],
    smooth: bool,
}

#[wasm_bindgen]
impl EdgePickJs {
    /// The object that owns the edge (the definition member for an instance).
    pub fn object(&self) -> u64 {
        self.object
    }
    /// The placing instance, if the pick hit instanced geometry.
    pub fn instance(&self) -> Option<u64> {
        self.instance
    }
    /// The edge's handle.
    pub fn edge(&self) -> u64 {
        self.edge
    }
    /// X of the ray's hit point on the face (object-local meters).
    pub fn x(&self) -> f64 {
        self.point[0]
    }
    /// Y of the ray's hit point on the face (object-local meters).
    pub fn y(&self) -> f64 {
        self.point[1]
    }
    /// Z of the ray's hit point on the face (object-local meters).
    pub fn z(&self) -> f64 {
        self.point[2]
    }
    /// True when the nearest edge within the aperture is smooth (a tangent
    /// joint or a facet seam) and no sharp edge is in range: the edge cannot
    /// be blended, and the tool says so.
    pub fn smooth(&self) -> bool {
        self.smooth
    }
}

/// A logical edge for highlighting and sizing: the chain of sharp kernel
/// edges a blend would treat as one.
#[wasm_bindgen]
pub struct LogicalEdgeJs {
    edges: Vec<u64>,
    segments: Vec<f32>,
    dihedral: f64,
    convexity: u8,
    closed: bool,
}

#[wasm_bindgen]
impl LogicalEdgeJs {
    /// The chain's kernel edge handles, end to end.
    pub fn edges(&self) -> Vec<u64> {
        self.edges.clone()
    }
    /// The chain as line segments, xyz pairs in object-local meters.
    pub fn segments(&self) -> Vec<f32> {
        self.segments.clone()
    }
    /// The dihedral angle of the first edge in radians, `NaN` when the
    /// chain is empty.
    pub fn dihedral(&self) -> f64 {
        self.dihedral
    }
    /// 0 when every edge is convex, 1 when every edge is concave, 2 when
    /// mixed (also 2 for an empty chain).
    pub fn convexity(&self) -> u8 {
        self.convexity
    }
    /// Whether the chain closes on itself.
    pub fn closed(&self) -> bool {
        self.closed
    }
}

/// What a committed blend made.
#[wasm_bindgen]
#[derive(Debug)]
pub struct BlendReportJs {
    blends: Vec<u32>,
    faces: Vec<u64>,
    size: f64,
}

#[wasm_bindgen]
impl BlendReportJs {
    /// The blend ids made (two for a two-pass corner).
    pub fn blend_ids(&self) -> Vec<u32> {
        self.blends.clone()
    }
    /// The new blend faces.
    pub fn faces(&self) -> Vec<u64> {
        self.faces.clone()
    }
    /// The size applied (meters), `NaN` for a bullnose.
    pub fn size(&self) -> f64 {
        self.size
    }
}

/// A blend preview: the new faces as render buffers, the faces they
/// replace, the rails, and whether the request was clamped.
#[wasm_bindgen]
pub struct BlendPreviewJs {
    mesh: tessellate::RenderMesh,
    replaced: Vec<u64>,
    rails: Vec<f32>,
    clamped: Option<kernel::BlendLimit>,
    applied: f64,
    corners: u32,
}

#[wasm_bindgen]
impl BlendPreviewJs {
    /// Whether the requested size did not fit and was reduced to the
    /// largest that does.
    pub fn clamped(&self) -> bool {
        self.clamped.is_some()
    }
    /// The largest size that fits, `NaN` when not clamped.
    pub fn max_size(&self) -> f64 {
        self.clamped.map_or(f64::NAN, |l| l.max)
    }
    /// What limits the size (a snake_case `LimitFeature` name), empty when
    /// not clamped.
    pub fn limited_by(&self) -> String {
        self.clamped
            .map_or(String::new(), |l| l.limited_by.name().to_owned())
    }
    /// The size actually previewed (meters), `NaN` for a bullnose.
    pub fn applied_size(&self) -> f64 {
        self.applied
    }
    /// Faces of the current object the result replaces.
    pub fn replaced_faces(&self) -> Vec<u64> {
        self.replaced.clone()
    }
    /// New faces: triangle vertex positions, xyz per vertex.
    pub fn positions(&self) -> Vec<f32> {
        self.mesh.positions.clone()
    }
    /// New faces: per-vertex normals.
    pub fn normals(&self) -> Vec<f32> {
        self.mesh.normals.clone()
    }
    /// New faces: triangle indices.
    pub fn indices(&self) -> Vec<u32> {
        self.mesh.indices.clone()
    }
    /// The rails, the tangent edges between new and kept faces, as xyz pairs.
    pub fn rail_positions(&self) -> Vec<f32> {
        self.rails.clone()
    }
    /// How many three-way corners the picked edges form.
    pub fn three_way_corners(&self) -> u32 {
        self.corners
    }
}

/// The ordinary-case shape every blend preview shares.
fn blend_preview_js(
    doc: &kernel::Document,
    original: Option<&kernel::Object>,
    preview: kernel::document::BlendPreview,
    corners: u32,
) -> Result<BlendPreviewJs, ApiError> {
    // The patch draws every face the result has that the object has not:
    // the blend's own faces and the neighbours it trimmed, which the result
    // rebuilds under new ids. The viewport hides the originals of those
    // neighbours (`replaced`), so a patch of the blend faces alone leaves
    // the solid looking gutted while it is sized.
    // Both sides of that comparison come from the ids: the faces of the
    // object that `after` no longer has are the ones to hide (a resize's
    // own list holds only the blend's faces, and a two-pass blend's only
    // the first pass's), the faces of `after` the object lacks are the
    // ones to draw.
    let (fresh, replaced): (Vec<kernel::FaceId>, Vec<kernel::FaceId>) = match original {
        Some(before) => (
            preview
                .after
                .faces()
                .keys()
                .filter(|f| !before.faces().contains_key(*f))
                .collect(),
            before
                .faces()
                .keys()
                .filter(|f| !preview.after.faces().contains_key(*f))
                .collect(),
        ),
        None => (preview.new_faces.clone(), preview.replaced.clone()),
    };
    let mesh = tessellate::tessellate_faces(&preview.after, &fresh, doc.materials())
        .map_err(|e| crate::api_err(&e, &e))?;
    let mut rails = Vec::new();
    for (eid, edge) in preview.after.edges() {
        if preview.after.edge_smoothness(eid) != kernel::EdgeSmoothness::Tangent {
            continue;
        }
        let face_of = |h: kernel::HalfEdgeId| {
            preview.after.loops()[preview.after.half_edges()[h].loop_id].face
        };
        let Some(twin) = edge.twin_half_edge else {
            continue;
        };
        let (a, b) = (face_of(edge.half_edge), face_of(twin));
        if preview.new_faces.contains(&a) != preview.new_faces.contains(&b)
            && let Some((p, q)) = preview.after.edge_endpoints(eid)
        {
            rails.extend([
                p.x as f32, p.y as f32, p.z as f32, q.x as f32, q.y as f32, q.z as f32,
            ]);
        }
    }
    Ok(BlendPreviewJs {
        mesh,
        replaced: replaced.iter().map(|f| f.data().as_ffi()).collect(),
        rails,
        clamped: preview.clamped,
        applied: preview.applied.size().unwrap_or(f64::NAN),
        corners,
    })
}

/// The number of vertices where three or more of `edges` meet.
fn three_way_corner_count(object: &kernel::Object, edges: &[kernel::EdgeId]) -> u32 {
    let mut count: std::collections::BTreeMap<kernel::VertexId, u32> =
        std::collections::BTreeMap::new();
    let unique: std::collections::BTreeSet<kernel::EdgeId> = edges.iter().copied().collect();
    for &e in &unique {
        let Some(edge) = object.edges().get(e) else {
            continue;
        };
        let he = object.half_edges()[edge.half_edge];
        for v in [he.origin, object.half_edges()[he.next].origin] {
            *count.entry(v).or_default() += 1;
        }
    }
    count.values().filter(|&&n| n >= 3).count() as u32
}

fn report_js(report: &kernel::BlendReport, world_scale: f64) -> BlendReportJs {
    BlendReportJs {
        blends: report.blends.iter().map(|b| b.0).collect(),
        faces: report.new_faces.iter().map(|f| f.data().as_ffi()).collect(),
        size: report.profile.size().map_or(f64::NAN, |v| v * world_scale),
    }
}

fn blend_kind_name(k: kernel::BlendKind) -> &'static str {
    match k {
        kernel::BlendKind::Fillet => "fillet",
        kernel::BlendKind::Chamfer => "chamfer",
        kernel::BlendKind::Profile => "profile",
    }
}

fn edge_id(handle: u64) -> kernel::EdgeId {
    kernel::EdgeId::from(slotmap::KeyData::from_ffi(handle))
}

fn face_id(handle: u64) -> kernel::FaceId {
    kernel::FaceId::from(slotmap::KeyData::from_ffi(handle))
}

fn finite_all(values: &[f64]) -> Result<(), ApiError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(ApiError::new("BadVector", "values must be finite"))
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 0.0 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

#[wasm_bindgen]
impl Scene {
    /// Picks the nearest sharp edge of the face under a ray, for the
    /// Fillet tool's hover. Ray-casts like `pick_face` (the nearest face
    /// hit), then takes the edge of that face's loops (outer or hole)
    /// nearest the hit point within `aperture` meters (world). Only sharp
    /// edges are returned as picks; when the nearest edge in range is
    /// smooth (a tangent joint or a facet seam) and no sharp edge is in
    /// range, the pick has `smooth() == true`. `undefined` when the ray
    /// misses, or nothing is in range.
    ///
    /// Read-only. Coordinates are object-local (an instance pick is mapped
    /// into the definition's frame, and the aperture divided by the pose's
    /// scale).
    ///
    /// # Errors
    /// `BadVector` for a non-finite ray or aperture.
    #[allow(clippy::too_many_arguments)] // a ray (6), like `pick_face`, plus the aperture
    pub fn pick_solid_edge(
        &self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
        aperture: f64,
    ) -> Result<Option<EdgePickJs>, ApiError> {
        finite_all(&[ox, oy, oz, dx, dy, dz, aperture])?;
        let origin = Point3::new(ox, oy, oz);
        let ray = inference::PickRay {
            origin,
            direction: kernel::Vec3::new(dx, dy, dz),
        };
        let Some((source, depth)) = self.inference.pick_face(&ray) else {
            return Ok(None);
        };
        let inference::ElementRef::Face(face) = source.element else {
            return Ok(None);
        };
        let Ok(dir) = ray.direction.normalized() else {
            return Ok(None);
        };
        let mut hit = origin + dir * depth;
        let mut reach = aperture;
        if let Some(i) = source.instance {
            let (Some(pose), true) = (self.doc.instance_pose(i), true) else {
                return Ok(None);
            };
            let Ok(inv) = pose.inverse() else {
                return Ok(None);
            };
            hit = inv.apply_point(hit);
            reach = aperture / pose.similarity_scale().unwrap_or(1.0);
        }
        let Some(obj) = self.doc.object(source.object) else {
            return Ok(None);
        };
        let Some(f) = obj.faces().get(face) else {
            return Ok(None);
        };
        let mut best_sharp: Option<(f64, kernel::EdgeId)> = None;
        let mut best_smooth: Option<(f64, kernel::EdgeId)> = None;
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            for h in obj.loop_half_edges(l) {
                let e = obj.half_edges()[h].edge;
                let Some((a, b)) = obj.edge_endpoints(e) else {
                    continue;
                };
                let d = segment_distance(hit, a, b);
                if d > reach {
                    continue;
                }
                let slot = if obj.edge_convexity(e).is_some() {
                    &mut best_sharp
                } else if obj.edge_smoothness(e) != kernel::EdgeSmoothness::Hard {
                    &mut best_smooth
                } else {
                    continue;
                };
                if slot.is_none_or(|(bd, _)| d < bd) {
                    *slot = Some((d, e));
                }
            }
        }
        let (edge, smooth) = match (best_sharp, best_smooth) {
            (Some((_, e)), _) => (e, false),
            (None, Some((_, e))) => (e, true),
            (None, None) => return Ok(None),
        };
        Ok(Some(EdgePickJs {
            object: source.object.data().as_ffi(),
            instance: source.instance.map(|i| i.data().as_ffi()),
            edge: edge.data().as_ffi(),
            point: [hit.x, hit.y, hit.z],
            smooth,
        }))
    }

    /// The logical edge containing `edge` (`kernel::Object::logical_edge`):
    /// the chain a blend of that edge would take, with `propagate` extending
    /// it along tangent continuations. Empty for an edge that is not sharp.
    ///
    /// Read-only.
    ///
    /// # Errors
    /// `UnknownObject`.
    pub fn logical_edge(
        &self,
        object: u64,
        edge: u64,
        propagate: bool,
    ) -> Result<LogicalEdgeJs, ApiError> {
        let obj = self
            .doc
            .object(crate::object_id(object))
            .ok_or_else(|| crate::stale("UnknownObject", "object"))?;
        Ok(logical_edge_js(
            obj,
            obj.logical_edge(edge_id(edge), propagate),
        ))
    }

    /// The edges a Fillet click on `edge` picks (`kernel::Object::picked_edge`):
    /// its logical edge, and with `propagate` on an imported mesh the
    /// longer seam a user sees there. Empty for an edge that is not sharp.
    ///
    /// Read-only.
    ///
    /// # Errors
    /// `UnknownObject`.
    pub fn picked_edge(
        &self,
        object: u64,
        edge: u64,
        propagate: bool,
    ) -> Result<LogicalEdgeJs, ApiError> {
        let obj = self
            .doc
            .object(crate::object_id(object))
            .ok_or_else(|| crate::stale("UnknownObject", "object"))?;
        Ok(logical_edge_js(
            obj,
            obj.picked_edge(edge_id(edge), propagate),
        ))
    }

    /// The sharp edges of the loop of `face` nearest `(x, y, z)` (its outer
    /// loop or a hole), each expanded to its logical edge, for the Fillet
    /// tool's face pick (`kernel::Object::face_loop_edges`). Read-only.
    ///
    /// # Errors
    /// `UnknownObject`, `UnknownFace`, `BadVector`.
    pub fn face_loop_edges(
        &self,
        object: u64,
        face: u64,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<LogicalEdgeJs, ApiError> {
        finite_all(&[x, y, z])?;
        let obj = self
            .doc
            .object(crate::object_id(object))
            .ok_or_else(|| crate::stale("UnknownObject", "object"))?;
        let fid = face_id(face);
        if obj.faces().get(fid).is_none() {
            return Err(crate::stale("UnknownFace", "face"));
        }
        Ok(logical_edge_js(
            obj,
            obj.face_loop_edges(fid, Point3::new(x, y, z)),
        ))
    }

    /// The two faces on either side of `edge`, as `[face_a, face_b]` (the
    /// face of the edge's first half-edge first), or empty for a boundary
    /// edge. The Fillet tool sizes a blend from the pointer against these
    /// two planes. Read-only.
    ///
    /// # Errors
    /// `UnknownObject`.
    pub fn edge_faces(&self, object: u64, edge: u64) -> Result<Vec<u64>, ApiError> {
        let obj = self
            .doc
            .object(crate::object_id(object))
            .ok_or_else(|| crate::stale("UnknownObject", "object"))?;
        let Some(e) = obj.edges().get(edge_id(edge)) else {
            return Ok(Vec::new());
        };
        let face_of = |h: kernel::HalfEdgeId| {
            obj.loops()[obj.half_edges()[h].loop_id]
                .face
                .data()
                .as_ffi()
        };
        Ok(match e.twin_half_edge {
            Some(t) => vec![face_of(e.half_edge), face_of(t)],
            None => Vec::new(),
        })
    }

    /// Every sharp edge of `object` (`kernel::Object::sharp_edges`), for
    /// "round everything" and for arming a selected object. Read-only.
    ///
    /// # Errors
    /// `UnknownObject`.
    pub fn object_sharp_edges(&self, object: u64) -> Result<Vec<u64>, ApiError> {
        let obj = self
            .doc
            .object(crate::object_id(object))
            .ok_or_else(|| crate::stale("UnknownObject", "object"))?;
        Ok(obj
            .sharp_edges()
            .into_iter()
            .map(|e| e.data().as_ffi())
            .collect())
    }

    /// The visible world objects having a face within `tol` meters of the
    /// point, in handle order. The Fillet tool uses two or more as the
    /// "these are two objects" hint. Read-only.
    ///
    /// # Errors
    /// `BadVector`.
    pub fn objects_touching_point(
        &self,
        x: f64,
        y: f64,
        z: f64,
        tol: f64,
    ) -> Result<Vec<u64>, ApiError> {
        finite_all(&[x, y, z, tol])?;
        let p = Point3::new(x, y, z);
        let mut out = Vec::new();
        for id in self.doc.live_object_ids() {
            if self.doc.object_owner_component(id).is_some() {
                continue;
            }
            let Some(obj) = self.doc.object(id) else {
                continue;
            };
            let touches = obj.faces().iter().any(|(fid, f)| {
                if f.plane.signed_distance(p).abs() > tol {
                    return false;
                }
                if obj.face_contains_point(fid, p) {
                    return true;
                }
                std::iter::once(f.outer_loop)
                    .chain(f.inner_loops.iter().copied())
                    .any(|l| {
                        let ring: Vec<Point3> = obj.loop_positions(l).collect();
                        (0..ring.len()).any(|i| {
                            segment_distance(p, ring[i], ring[(i + 1) % ring.len()]) <= tol
                        })
                    })
            });
            if touches {
                out.push(id.data().as_ffi());
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// The largest size at which `spec_json` (a `kernel::BlendSpec`) still
    /// blends as the commit makes it, in meters
    /// (`kernel::Object::max_blend_size`): where the edges run into existing
    /// fillets, each size is tried with the corner solved again, as the
    /// commit does; on an imported mesh or a generated wall (a Loft's), the
    /// largest size the swept blend rolls where that is larger. Read-only.
    ///
    /// # Errors
    /// `BadSpec`, `UnknownObject`, and the blend errors the engine reports
    /// for the spec (`BlendNeedsSolid`, `EdgeIsSmooth`, `UnsupportedEdge`, …).
    pub fn max_blend_size(&self, object: u64, spec_json: &str) -> Result<f64, ApiError> {
        let spec: kernel::BlendSpec = parse_spec(spec_json)?;
        let obj = self
            .doc
            .object(crate::object_id(object))
            .ok_or_else(|| crate::stale("UnknownObject", "object"))?;
        obj.max_blend_size(&spec)
            .map(|l| l.max)
            .map_err(|e| doc_err(kernel::DocumentError::Blend(e)))
    }

    /// Sizes here are in the object's own (definition) units; a caller
    /// previewing through an instance divides its world sizes by the pose's
    /// scale. Previews `spec_json` (a `kernel::BlendSpec`) on `object` without
    /// committing: the new faces as render buffers, the faces they replace,
    /// the rails, and, when the size did not fit, the largest that does
    /// (the preview shows that size). Pure: never records, never
    /// reconciles. Throws exactly the codes the commit would, except that a
    /// size that merely does not fit is clamped and reported instead.
    ///
    /// # Errors
    /// `BadSpec`, `UnknownObject`, and the `BlendError` codes
    /// (`BlendNeedsSolid`, `EdgeIsSmooth`, `UnsupportedEdge`,
    /// `UnsupportedVertex`, `ProfileTooWide`, `Obstructed`, …).
    pub fn blend_preview(&self, object: u64, spec_json: &str) -> Result<BlendPreviewJs, ApiError> {
        let spec: kernel::BlendSpec = parse_spec(spec_json)?;
        let oid = crate::object_id(object);
        let preview = self.doc.blend_preview(oid, &spec).map_err(doc_err)?;
        let corners = self
            .doc
            .object(oid)
            .map_or(0, |o| three_way_corner_count(o, &spec.edges));
        blend_preview_js(&self.doc, self.doc.object(oid), preview, corners)
    }

    /// [`Scene::blend_preview`] for a size the user typed
    /// (`Document::blend_preview_typed`): a swept blend takes the commit's
    /// rounds, so the preview is what Enter makes. Slower than a pointer
    /// preview on a swept edge; call it once per typed size, not on every
    /// pointer move.
    ///
    /// # Errors
    /// As [`Scene::blend_preview`].
    pub fn blend_preview_typed(
        &self,
        object: u64,
        spec_json: &str,
    ) -> Result<BlendPreviewJs, ApiError> {
        let spec: kernel::BlendSpec = parse_spec(spec_json)?;
        let oid = crate::object_id(object);
        let preview = self.doc.blend_preview_typed(oid, &spec).map_err(doc_err)?;
        let corners = self
            .doc
            .object(oid)
            .map_or(0, |o| three_way_corner_count(o, &spec.edges));
        blend_preview_js(&self.doc, self.doc.object(oid), preview, corners)
    }

    /// Blends the spec's edges as one undo step (`Document::blend_edges`).
    /// Records `BlendEdges`.
    ///
    /// # Errors
    /// As [`Scene::blend_preview`], plus `RadiusTooLarge` when the size does
    /// not fit (its message carries the largest size and what limits it).
    pub fn blend_edges(&mut self, object: u64, spec_json: &str) -> Result<BlendReportJs, ApiError> {
        self.ensure_not_busy()?;
        let spec: kernel::BlendSpec = parse_spec(spec_json)?;
        let (report, change) = self
            .doc
            .blend_edges(crate::object_id(object), &spec)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::BlendEdges {
            object,
            spec: spec_json.to_owned(),
        });
        Ok(report_js(&report, 1.0))
    }

    /// [`Scene::blend_edges`] on `object`, a member of `instance`'s
    /// definition, with the spec's lengths in world meters
    /// (`Document::blend_edges_in_instance`). Records
    /// `BlendEdgesInComponent`.
    ///
    /// # Errors
    /// As [`Scene::blend_edges`], plus `UnknownInstance` and
    /// `NonUniformInstance`.
    pub fn blend_edges_in_component(
        &mut self,
        instance: u64,
        object: u64,
        spec_json: &str,
    ) -> Result<BlendReportJs, ApiError> {
        self.ensure_not_busy()?;
        let spec: kernel::BlendSpec = parse_spec(spec_json)?;
        let scale = self.component_member_scale(instance, object)?;
        let (report, change) = self
            .doc
            .blend_edges_in_instance(
                crate::instance_id(instance),
                crate::object_id(object),
                &spec,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::BlendEdgesInComponent {
            instance,
            object,
            spec: spec_json.to_owned(),
        });
        Ok(report_js(&report, scale))
    }

    /// The blend id of the tagged face, or `undefined` for an ordinary face.
    pub fn blend_of_face(&self, object: u64, face: u64) -> Option<u32> {
        self.doc
            .object(crate::object_id(object))?
            .blend_of_face(face_id(face))
            .map(|b| b.0)
    }

    /// The faces of blend `blend` on `object`, in slot order.
    pub fn blend_faces(&self, object: u64, blend: u32) -> Vec<u64> {
        let Some(obj) = self.doc.object(crate::object_id(object)) else {
            return Vec::new();
        };
        obj.faces()
            .iter()
            .filter(|(_, f)| f.blend.is_some_and(|t| t.id.0 == blend))
            .map(|(id, _)| id.data().as_ffi())
            .collect()
    }

    /// The blends on `object` as JSON:
    /// `[{"id", "kind": "fillet"|"chamfer"|"profile", "faces": ["<id>", …],
    /// "size", "intact"}]`. `size` is meters or `null` when it cannot be
    /// read; `intact` is whether the blend can be removed or resized.
    ///
    /// # Errors
    /// `UnknownObject`.
    pub fn blend_features_json(&self, object: u64) -> Result<String, ApiError> {
        let features = self
            .doc
            .blend_features(crate::object_id(object))
            .map_err(doc_err)?;
        let rows: Vec<serde_json::Value> = features
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id.0,
                    "kind": blend_kind_name(f.kind),
                    "faces": f.faces.iter().map(|x| x.data().as_ffi().to_string()).collect::<Vec<_>>(),
                    "size": f.size,
                    "intact": f.intact,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(rows).to_string())
    }

    /// Previews resizing blend `blend` to `profile_json` (a
    /// `kernel::BlendProfile`). Pure, like [`Scene::blend_preview`], and
    /// clamped the same way: the preview is the resize, which makes the
    /// blend as a fresh fillet of its edges would be.
    ///
    /// # Errors
    /// `BadSpec`, `UnknownObject`, `UnknownBlend`, `BlendModified`,
    /// `DependentBlend` (a later blend that runs into this one cannot be
    /// put back), and the blend errors of the re-blend.
    pub fn resize_blend_preview(
        &self,
        object: u64,
        blend: u32,
        profile_json: &str,
    ) -> Result<BlendPreviewJs, ApiError> {
        let profile: kernel::BlendProfile = parse_spec(profile_json)?;
        let oid = crate::object_id(object);
        let preview = self
            .doc
            .resize_blend_preview(oid, kernel::BlendId(blend), &profile)
            .map_err(doc_err)?;
        blend_preview_js(&self.doc, self.doc.object(oid), preview, 0)
    }

    /// Resizes blend `blend` to `profile_json` (a size change, or a switch
    /// between round and chamfer) as one undo step. Records `ResizeBlend`.
    /// The blend is made again as a fresh fillet of its edges would be:
    /// where they run into other fillets the corner is solved again with
    /// them, larger first (at an equal size the two become one fillet under
    /// a new id), and later fillets that ran into it are put back.
    ///
    /// # Errors
    /// As [`Scene::resize_blend_preview`], plus `RadiusTooLarge`.
    pub fn resize_blend(
        &mut self,
        object: u64,
        blend: u32,
        profile_json: &str,
    ) -> Result<BlendReportJs, ApiError> {
        self.ensure_not_busy()?;
        let profile: kernel::BlendProfile = parse_spec(profile_json)?;
        let (report, change) = self
            .doc
            .resize_blend(crate::object_id(object), kernel::BlendId(blend), &profile)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ResizeBlend {
            object,
            blend,
            profile: profile_json.to_owned(),
        });
        Ok(report_js(&report, 1.0))
    }

    /// [`Scene::resize_blend`] on a member of `instance`'s definition, with
    /// the profile's lengths in world meters. Records
    /// `ResizeBlendInComponent`.
    ///
    /// # Errors
    /// As [`Scene::resize_blend`], plus `UnknownInstance`,
    /// `UnknownObject` (not a member) and `NonUniformInstance`.
    pub fn resize_blend_in_component(
        &mut self,
        instance: u64,
        object: u64,
        blend: u32,
        profile_json: &str,
    ) -> Result<BlendReportJs, ApiError> {
        self.ensure_not_busy()?;
        let profile: kernel::BlendProfile = parse_spec(profile_json)?;
        let s = self.component_member_scale(instance, object)?;
        let (report, change) = self
            .doc
            .resize_blend(
                crate::object_id(object),
                kernel::BlendId(blend),
                &profile.scaled(1.0 / s),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ResizeBlendInComponent {
            instance,
            object,
            blend,
            profile: profile_json.to_owned(),
        });
        Ok(report_js(&report, s))
    }

    /// Removes blend `blend`, restoring its sharp edges, as one undo step.
    /// Records `RemoveBlend`.
    ///
    /// # Errors
    /// `UnknownObject`, `UnknownBlend` (no face carries that id),
    /// `BlendModified` (the geometry around it changed so it cannot be
    /// restored exactly), `DependentBlend` (a later blend that runs into
    /// this one cannot be put back on the restored geometry).
    pub fn remove_blend(&mut self, object: u64, blend: u32) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let change = self
            .doc
            .remove_blend(crate::object_id(object), kernel::BlendId(blend))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RemoveBlend { object, blend });
        Ok(())
    }

    /// [`Scene::remove_blend`] on a member of `instance`'s definition.
    /// Records `RemoveBlendInComponent`.
    ///
    /// # Errors
    /// As [`Scene::remove_blend`], plus `UnknownInstance` and
    /// `UnknownObject` (not a member).
    pub fn remove_blend_in_component(
        &mut self,
        instance: u64,
        object: u64,
        blend: u32,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.component_member_scale(instance, object)?;
        let change = self
            .doc
            .remove_blend(crate::object_id(object), kernel::BlendId(blend))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RemoveBlendInComponent {
            instance,
            object,
            blend,
        });
        Ok(())
    }
}

impl Scene {
    /// The uniform scale of `instance`'s pose when `object` is a member of
    /// its definition. `UnknownInstance`, `UnknownObject` or
    /// `NonUniformInstance` otherwise.
    fn component_member_scale(&self, instance: u64, object: u64) -> Result<f64, ApiError> {
        let iid = crate::instance_id(instance);
        let def = self
            .doc
            .instance_def(iid)
            .ok_or_else(|| crate::stale("UnknownInstance", "instance"))?;
        let pose = self
            .doc
            .instance_pose(iid)
            .ok_or_else(|| crate::stale("UnknownInstance", "instance"))?;
        if self.doc.object_owner_component(crate::object_id(object)) != Some(def) {
            return Err(crate::stale("UnknownObject", "object"));
        }
        pose.similarity_scale().ok_or_else(|| {
            doc_err(kernel::DocumentError::Blend(
                kernel::BlendError::NonUniformInstance,
            ))
        })
    }
}

/// Builds the highlight/sizing summary of an ordered edge chain.
fn logical_edge_js(obj: &kernel::Object, edges: Vec<kernel::EdgeId>) -> LogicalEdgeJs {
    let mut segments = Vec::with_capacity(edges.len() * 6);
    let mut degree: std::collections::BTreeMap<kernel::VertexId, u32> =
        std::collections::BTreeMap::new();
    let (mut convex, mut concave) = (false, false);
    for &e in &edges {
        if let Some((p, q)) = obj.edge_endpoints(e) {
            segments.extend([
                p.x as f32, p.y as f32, p.z as f32, q.x as f32, q.y as f32, q.z as f32,
            ]);
        }
        match obj.edge_convexity(e) {
            Some(kernel::Convexity::Convex) => convex = true,
            Some(kernel::Convexity::Concave) => concave = true,
            None => {}
        }
        if let Some(edge) = obj.edges().get(e) {
            let he = obj.half_edges()[edge.half_edge];
            for v in [he.origin, obj.half_edges()[he.next].origin] {
                *degree.entry(v).or_default() += 1;
            }
        }
    }
    let closed = !edges.is_empty() && degree.values().all(|&n| n == 2);
    LogicalEdgeJs {
        dihedral: edges
            .first()
            .and_then(|&e| obj.edge_dihedral(e))
            .unwrap_or(f64::NAN),
        convexity: match (convex, concave) {
            (true, false) => 0,
            (false, true) => 1,
            _ => 2,
        },
        closed,
        segments,
        edges: edges.iter().map(|e| e.data().as_ffi()).collect(),
    }
}
// ==== end true-curves: blend ====

// ==== true-curves: profiles in the Library ====

/// The bundled starter profiles as JSON, in their display order:
/// `[{"id": "builtin:ogee", "name": "Ogee", "shape": <ProfileShape>}, …]`
/// (`kernel::bundled_profiles`). Pure.
#[wasm_bindgen]
pub fn bundled_profiles_json() -> String {
    let rows: Vec<serde_json::Value> = kernel::bundled_profiles()
        .iter()
        .map(|(id, name, make)| serde_json::json!({ "id": id, "name": name, "shape": make() }))
        .collect();
    serde_json::Value::Array(rows).to_string()
}

/// A profile item's bytes for the bundled profile `id`, drawn 50 mm wide,
/// with `meta_json` (id, name, category `profile`, keywords, collection,
/// savedAt) stamped into the `hew.library` attributes: the "Save a copy" of
/// a built-in.
///
/// # Errors
/// `UnknownProfile` (no such bundled id), `BadLibraryMeta`, and the
/// document errors of building the profile document.
#[wasm_bindgen]
pub fn profile_item_bytes_from_bundled(id: &str, meta_json: &str) -> Result<Vec<u8>, ApiError> {
    let (_, _, make) = kernel::bundled_profiles()
        .iter()
        .find(|(bid, _, _)| *bid == id)
        .ok_or_else(|| ApiError::new("UnknownProfile", "no built-in profile has that id"))?;
    let mut item = kernel::Document::profile_document_from_shape(&make(), 0.05).map_err(doc_err)?;
    crate::apply_library_meta(&mut item, Some(meta_json))?;
    Ok(item.save())
}

/// The shape and drawn width of a profile item's `.hew` bytes, as JSON
/// `{"shape": <ProfileShape>, "width": meters}`.
///
/// # Errors
/// `InvalidProfile` unless the document is one valid open chain; load
/// errors for bytes that are not a document.
#[wasm_bindgen]
pub fn profile_shape_from_item(bytes: &[u8]) -> Result<String, ApiError> {
    let doc = kernel::Document::load(bytes).map_err(|e| crate::api_err(&e, &e))?;
    let (shape, width) = doc.profile_shape_of().map_err(doc_err)?;
    Ok(serde_json::json!({ "shape": shape, "width": width }).to_string())
}

/// A square PNG of what `shape_json` (a `ProfileShape`) cuts: a 4 × 4 × 4
/// box with one top edge blended with the profile (width 1), seen end-on
/// along that edge in parallel projection so the section shows.
///
/// # Errors
/// `BadSpec` (malformed shape), `InvalidProfile`, and the blend errors for
/// the sample box (which never refuse for a valid shape).
#[wasm_bindgen]
pub fn render_profile_thumbnail(shape_json: &str, px: u32) -> Result<Vec<u8>, ApiError> {
    let shape: kernel::ProfileShape = parse_spec(shape_json)?;
    let size = px.clamp(16, 1024);
    let mut doc = kernel::Document::new();
    let side = 4.0;
    let plane = kernel::Plane::from_point_normal(
        Point3::new(0.0, 0.0, 0.0),
        kernel::Vec3::new(0.0, 0.0, 1.0),
    )
    .map_err(|e| crate::api_err(&e, &e))?;
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).map_err(doc_err)?;
    {
        let sk = doc
            .sketch_mut(s)
            .ok_or_else(|| ApiError::new("UnknownSketch", "sketch vanished"))?;
        let c = [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .map_err(|e| crate::api_err(&e, &e))?;
        }
    }
    doc.end_sketch_gesture(s).map_err(doc_err)?;
    let region = doc
        .extrudable_regions(s)
        .map_err(doc_err)?
        .first()
        .copied()
        .ok_or_else(|| ApiError::new("InvalidProfile", "no sample box"))?;
    let (obj, _) = doc.extrude_region(s, region, side).map_err(doc_err)?;
    // The top front edge: y = 0, z = side, along x.
    let edge = {
        let o = doc
            .object(obj)
            .ok_or_else(|| ApiError::new("UnknownObject", "sample box vanished"))?;
        o.edges()
            .keys()
            .find(|&e| {
                o.edge_endpoints(e).is_some_and(|(a, b)| {
                    a.y.abs() < 1e-9
                        && b.y.abs() < 1e-9
                        && (a.z - side).abs() < 1e-9
                        && (b.z - side).abs() < 1e-9
                })
            })
            .ok_or_else(|| ApiError::new("UnknownEdge", "sample edge missing"))?
    };
    // A tall shape (b > 1) is drawn narrower so it fits the sample box and
    // the picture; the view scales with it.
    let s = (3.5 / shape.b.max(1e-6)).min(1.0);
    let spec = kernel::BlendSpec {
        edges: vec![edge],
        profile: kernel::BlendProfile::Custom {
            shape,
            width: s,
            arc_segments: 8,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    };
    doc.blend_edges(obj, &spec).map_err(doc_err)?;
    let items = softrender::document_items(&doc);
    let render_items: Vec<softrender::RenderItem> = items
        .iter()
        .map(|it| softrender::RenderItem {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();
    // Looking along +x at the end face from outside; the corner (y = 0,
    // z = side) sits up and to the right of the view's center.
    let camera = softrender::Camera {
        eye: Point3::new(-20.0, 0.6 * s, side - 0.6 * s),
        target: Point3::new(0.0, 0.6 * s, side - 0.6 * s),
        up: kernel::Vec3::new(0.0, 0.0, 1.0),
        projection: softrender::Projection::Parallel {
            half_height: 1.5 * s,
        },
    };
    let rendered = softrender::render(&render_items, &camera, size, size)
        .map_err(|e| ApiError::new("RenderFailed", &format!("{e:?}")))?;
    Ok(softrender::png::encode(
        &rendered.rgba,
        rendered.width,
        rendered.height,
    ))
}

#[wasm_bindgen]
impl Scene {
    /// A profile item's bytes from the open chain `edges` of world sketch
    /// `sketch` (`Document::profile_document_from_chain`): the chain is
    /// normalized to the ground plane with its corner at the origin. List
    /// the edges from the profile's start (on the bearing face) to its end.
    /// `meta_json` is stamped into the `hew.library` attributes (id, name,
    /// category `profile`, keywords, collection, savedAt).
    ///
    /// # Errors
    /// `UnknownSketch`, `InvalidProfile` (not one valid open chain),
    /// `BadLibraryMeta`.
    pub fn profile_item_bytes_from_chain(
        &self,
        sketch: u64,
        edges: Vec<u64>,
        meta_json: &str,
    ) -> Result<Vec<u8>, ApiError> {
        let ids: Vec<kernel::SketchEdgeId> = edges
            .iter()
            .map(|&e| kernel::SketchEdgeId::from(slotmap::KeyData::from_ffi(e)))
            .collect();
        let mut item = self
            .doc
            .profile_document_from_chain(crate::sketch_id(sketch), &ids)
            .map_err(doc_err)?;
        crate::apply_library_meta(&mut item, Some(meta_json))?;
        Ok(item.save())
    }

    /// The edges of world sketch `sketch` in slot order (usually the order
    /// they were drawn in, but a slot freed by an erase is reused), for
    /// listing a whole drawn chain. The chain's start is the free end of the
    /// first listed edge that has one. Read-only.
    ///
    /// # Errors
    /// `UnknownSketch`.
    pub fn sketch_edge_list(&self, sketch: u64) -> Result<Vec<u64>, ApiError> {
        let sk = self
            .doc
            .sketch(crate::sketch_id(sketch))
            .ok_or_else(|| crate::stale("UnknownSketch", "sketch"))?;
        Ok(sk.edges().keys().map(|e| e.data().as_ffi()).collect())
    }
}

/// A profile item's bytes from an ordered polyline (a chain drawn on a
/// face): `points` is xyz per vertex from the profile's start to its end;
/// `curve_centers` is xyz and `curve_radii` one radius per segment, a NaN
/// radius meaning a straight segment; `normal` is the drawing plane's
/// normal. `meta_json` as for [`Scene::profile_item_bytes_from_chain`].
///
/// # Errors
/// `BadVector` (lengths or non-finite coordinates), `InvalidProfile`,
/// `BadLibraryMeta`.
#[wasm_bindgen]
pub fn profile_item_bytes_from_polyline(
    normal: Vec<f64>,
    points: Vec<f64>,
    curve_centers: Vec<f64>,
    curve_radii: Vec<f64>,
    meta_json: &str,
) -> Result<Vec<u8>, ApiError> {
    let bad = || {
        ApiError::new(
            "BadVector",
            "a profile polyline needs matching, finite arrays",
        )
    };
    if normal.len() != 3
        || !points.len().is_multiple_of(3)
        || points.len() < 6
        || curve_radii.len() != points.len() / 3 - 1
        || curve_centers.len() != curve_radii.len() * 3
        || !points
            .iter()
            .chain(&normal)
            .chain(&curve_centers)
            .all(|v| v.is_finite())
        || normal.iter().map(|v| v * v).sum::<f64>().sqrt() < 1e-9
    {
        return Err(bad());
    }
    let pts: Vec<Point3> = points
        .chunks_exact(3)
        .map(|c| Point3::new(c[0], c[1], c[2]))
        .collect();
    let curves: Vec<Option<kernel::CurveGeom>> = curve_radii
        .iter()
        .zip(curve_centers.chunks_exact(3))
        .map(|(&r, c)| {
            (r.is_finite() && r > 0.0).then(|| kernel::CurveGeom {
                center: Point3::new(c[0], c[1], c[2]),
                radius: r,
            })
        })
        .collect();
    let mut item = kernel::Document::profile_document_from_polyline(
        &pts,
        &curves,
        kernel::Vec3::new(normal[0], normal[1], normal[2]),
    )
    .map_err(doc_err)?;
    crate::apply_library_meta(&mut item, Some(meta_json))?;
    Ok(item.save())
}
// ==== end true-curves: profiles in the Library ====
// ==== Array along Path (WP18) ====
/// The path array's boundary: its own module so its path and handle
/// parsing stay private to it.
mod path_array {
    use wasm_bindgen::prelude::*;

    use kernel::FollowMePath;
    use serde::Deserialize;
    use slotmap::KeyData;

    use crate::{
        ApiError, NodeJs, Scene, doc_err, node_ids, node_js, object_id, recording, sketch_id,
    };

    /// A handle in a JSON spec: a number, or a decimal string (the form that
    /// survives JavaScript's 53-bit integers).
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Handle {
        Num(u64),
        Str(String),
    }

    impl Handle {
        fn get(&self) -> Result<u64, ApiError> {
            match self {
                Handle::Num(n) => Ok(*n),
                Handle::Str(s) => s
                    .parse()
                    .map_err(|_| ApiError::new("BadSpec", "a handle must be an integer")),
            }
        }
    }

    /// The path of an array along a path, in Follow Me's two path forms.
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum PathJson {
        /// Edges of one sketch forming a single connected chain.
        SketchEdges { sketch: Handle, edges: Vec<Handle> },
        /// The outer boundary loop of a solid's face.
        FaceLoop { object: Handle, face: Handle },
    }

    fn parse_path(path_json: &str) -> Result<FollowMePath, ApiError> {
        let path: PathJson = serde_json::from_str(path_json)
            .map_err(|e| ApiError::new("BadSpec", &format!("path: {e}")))?;
        Ok(match path {
            PathJson::SketchEdges { sketch, edges } => FollowMePath::SketchEdges {
                sketch: sketch_id(sketch.get()?),
                edges: edges
                    .iter()
                    .map(|e| {
                        e.get()
                            .map(|k| kernel::SketchEdgeId::from(KeyData::from_ffi(k)))
                    })
                    .collect::<Result<_, _>>()?,
            },
            PathJson::FaceLoop { object, face } => FollowMePath::FaceLoop {
                object: object_id(object.get()?),
                face: kernel::FaceId::from(KeyData::from_ffi(face.get()?)),
            },
        })
    }

    fn path_array_spec(
        count: u32,
        scale_from: f64,
        scale_to: f64,
        align: bool,
    ) -> kernel::PathArraySpec {
        kernel::PathArraySpec {
            count,
            scale_from,
            scale_to,
            align,
        }
    }

    #[wasm_bindgen]
    impl Scene {
        /// Copies a selection along a path (Array along Path). `kinds`/`ids` name
        /// the nodes as in [`Scene::duplicate_selection_array`]. `path_json` is
        /// `{"kind":"sketch_edges","sketch":ID,"edges":[ID,…]}` or
        /// `{"kind":"face_loop","object":ID,"face":ID}` (ids as numbers or
        /// decimal strings). `count` is the total instance count including the
        /// original; copies are spaced evenly by arc length and scaled linearly
        /// from `scale_from` to `scale_to`; `align` turns them to follow the
        /// path. One undoable step. Returns the clone roots in creation order.
        ///
        /// # Errors
        /// - `BadNodeList` — mismatched or bad `kinds`/`ids`.
        /// - `BadSpec` — `path_json` is malformed.
        /// - `AmountOutOfRange` — `count` below 2 or above the array cap, or a
        ///   non-positive scale.
        /// - `PathTooShortForCount`, the Follow Me path errors, and the errors of
        ///   [`Scene::duplicate_selection_array`].
        ///
        /// On error the document is untouched.
        // Scalar args are deliberate boundary ergonomics (docs/dev/DEVELOPMENT.md).
        #[allow(clippy::too_many_arguments)]
        pub fn array_along_path(
            &mut self,
            kinds: &[u8],
            ids: &[u64],
            path_json: &str,
            count: u32,
            scale_from: f64,
            scale_to: f64,
            align: bool,
        ) -> Result<Vec<NodeJs>, ApiError> {
            let nodes = node_ids(kinds, ids)?;
            let path = parse_path(path_json)?;
            let spec = path_array_spec(count, scale_from, scale_to, align);
            let (roots, change) = self
                .doc
                .array_along_path(&nodes, &path, &spec)
                .map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::ArrayAlongPath {
                kinds: kinds.to_vec(),
                ids: ids.to_vec(),
                path: path_json.to_owned(),
                count,
                scale_from,
                scale_to,
                align,
            });
            Ok(roots.into_iter().map(node_js).collect())
        }

        /// The placements [`Scene::array_along_path`] would apply, for the ghost
        /// preview: 12 floats per copy (the row-major 3×4 affine of
        /// [`Scene::transform_object`]), copy 1 first. Never mutates or records.
        /// Throws the path and amount codes of the commit; node errors are the
        /// commit's alone.
        pub fn array_along_path_preview_transforms(
            &self,
            path_json: &str,
            count: u32,
            scale_from: f64,
            scale_to: f64,
            align: bool,
        ) -> Result<Vec<f64>, ApiError> {
            let path = parse_path(path_json)?;
            let spec = path_array_spec(count, scale_from, scale_to, align);
            let placements = self
                .doc
                .path_array_placements(&path, &spec)
                .map_err(doc_err)?;
            Ok(placements.iter().flat_map(|t| t.to_affine()).collect())
        }
    }
}
