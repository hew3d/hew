//! The drafting engine across the wasm boundary (docs/design/unified-drafting.md
//! §4, §6): one open draft at a time, edited with sketch-level calls and
//! lowered as one undo step. A third `#[wasm_bindgen] impl Scene` block,
//! kept out of `lib.rs` for size; it reaches the handle's private `doc`
//! like any child module.
//!
//! Boundary conventions:
//! - Every point and length crossing here is in WORLD space. An `*_in_instance`
//!   draft stores the instance pose on the open draft and maps every later
//!   argument through its inverse and every returned point through it.
//! - A draft handle is a counter value (`u64`), refused with `StaleDraft` once
//!   the draft ended, was cancelled, or the document was replaced.
//! - Keys ([`kernel::edge_key`] and friends) are 64-bit hashes: they cross
//!   JSON as lowercase 16-digit hex strings, never numbers, and cross the
//!   call boundary as `u64`. Object/sketch/face ids ride JSON as numbers,
//!   exactly as `sketch_unit_keys` does.

use kernel::{
    ComponentId, DraftStore, FaceId, ObjectId, Plane, PlaneUnits, Point3, Sketch, SketchEdgeId,
    SketchId, Transform, Vec3,
};
use slotmap::{Key, KeyData};
use wasm_bindgen::prelude::*;

use crate::{
    ApiError, Scene, SegmentAddedJs, api_err, doc_err, instance_id, object_id, parse_affine,
    recording, sketch_id, stale,
};

/// An instance pose, both ways (definition-local to world, and back).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pose {
    fwd: Transform,
    inv: Transform,
}

impl Pose {
    fn to_local(self, p: Point3) -> Point3 {
        self.inv.apply_point(p)
    }

    fn to_world(self, p: Point3) -> Point3 {
        self.fwd.apply_point(p)
    }

    /// The pose's uniform scale, or the typed refusal under non-uniform
    /// scale (a world-space length cannot map unambiguously).
    fn scale(&self) -> Result<f64, ApiError> {
        self.fwd.similarity_scale().ok_or_else(|| {
            ApiError::new(
                "AmbiguousInstanceScale",
                "the instance pose is not a similarity; a length cannot map through it",
            )
        })
    }

    /// A world transform as the equivalent definition-local one.
    fn local_transform(&self, world: &Transform) -> Transform {
        self.fwd.then(world).then(&self.inv)
    }
}

/// The open draft's session record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenDraft {
    handle: u64,
    pose: Option<Pose>,
}

/// What a drafting context lifts (detached shapes §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftScope {
    /// The plane's arrangement: its drawing and the objects taking part.
    Plane,
    /// A detached shape's own context, nothing else on the plane taking
    /// part: `Some(s)` is detached sketch `s` alone; `None` draws ONE new
    /// detached sketch ([`Scene::draft_begin_detached`]).
    Detached(Option<SketchId>),
}

/// Which form of context JSON a [`DraftCtx`] was resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CtxForm {
    /// `{"plane":[..]}`, optionally with `"instance"`.
    Plane,
    /// `{"plane":[..],"object":id}`: one object's own drawing.
    ObjectPlane,
    /// `{"face":{..}}`, optionally with `"instance"`.
    Face,
    /// `{"sketch":id}`, optionally with `"instance"`.
    Sketch,
}

/// A resolved drafting context: the plane in its frame, the frame, the
/// instance pose (if any), the objects that take part, what it lifts, the
/// form it was given in, and the instance it was seen through.
struct DraftCtx {
    plane: Plane,
    frame: Option<ComponentId>,
    pose: Option<Pose>,
    objects: Vec<ObjectId>,
    scope: DraftScope,
    form: CtxForm,
    instance: Option<u64>,
}

fn p3(a: [f64; 3]) -> Point3 {
    Point3::new(a[0], a[1], a[2])
}

fn triple(v: &[f64], what: &str) -> Result<[f64; 3], ApiError> {
    let a: &[f64; 3] = v
        .try_into()
        .map_err(|_| ApiError(format!("BadVector: {what} must be an xyz triple")))?;
    if a.iter().any(|x| !x.is_finite()) {
        return Err(ApiError(format!(
            "BadVector: {what} must be finite numbers"
        )));
    }
    Ok(*a)
}

fn hex(key: u64) -> String {
    format!("{key:016x}")
}

fn world_json(pose: Option<Pose>, p: Point3) -> serde_json::Value {
    let w = pose.map_or(p, |pz| pz.to_world(p));
    serde_json::json!([w.x, w.y, w.z])
}

fn stale_draft() -> ApiError {
    stale("StaleDraft", "draft")
}

/// Parses a context's JSON.
fn parse_ctx(ctx_json: &str) -> Result<serde_json::Value, ApiError> {
    serde_json::from_str(ctx_json)
        .map_err(|e| ApiError(format!("BadContext: malformed context JSON: {e}")))
}

/// The form of a context's JSON, as [`Scene::ctx_from_json`] reads it
/// (`plane` before `face` before `sketch`); `None` for none of them.
fn ctx_form(v: &serde_json::Value) -> Option<CtxForm> {
    if v.get("plane").is_some() {
        Some(match v.get("object") {
            None | Some(serde_json::Value::Null) => CtxForm::Plane,
            Some(_) => CtxForm::ObjectPlane,
        })
    } else if v.get("face").is_some() {
        Some(CtxForm::Face)
    } else if v.get("sketch").is_some() {
        Some(CtxForm::Sketch)
    } else {
        None
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn dist_to_segment(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 <= 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length()
}

/// Whether `p` lies inside the polygon `ring` (even-odd, projected along
/// the dominant axis of `normal`).
fn in_ring(p: Point3, ring: &[Point3], normal: Vec3) -> bool {
    let (ax, ay, az) = (normal.x.abs(), normal.y.abs(), normal.z.abs());
    let proj = |q: Point3| -> (f64, f64) {
        if az >= ax && az >= ay {
            (q.x, q.y)
        } else if ay >= ax {
            (q.x, q.z)
        } else {
            (q.y, q.z)
        }
    };
    let (px, py) = proj(p);
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let (x1, y1) = proj(ring[i]);
        let (x2, y2) = proj(ring[(i + 1) % n]);
        if (y1 > py) != (y2 > py) && px < (x2 - x1) * (py - y1) / (y2 - y1) + x1 {
            inside = !inside;
        }
    }
    inside
}

/// The area of the polygon `ring` (its Newell normal along `normal`).
fn ring_area(ring: &[Point3], normal: Vec3) -> f64 {
    let n = ring.len();
    let mut acc = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        acc = acc
            + Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
    }
    (acc.dot(normal) * 0.5).abs()
}

impl Scene {
    /// Resolves instance `instance` to its definition and pose.
    fn draft_instance_pose(&self, instance: u64) -> Result<(ComponentId, Pose), ApiError> {
        let iid = instance_id(instance);
        let def = self
            .doc
            .instance_def(iid)
            .filter(|_| !self.hidden_instances.contains(&iid))
            .ok_or_else(|| stale("UnknownInstance", "instance"))?;
        let fwd = self
            .doc
            .instance_pose(iid)
            .ok_or_else(|| stale("UnknownInstance", "instance"))?;
        let inv = fwd.inverse().map_err(|e| api_err(&e, &e))?;
        Ok((def, Pose { fwd, inv }))
    }

    /// The objects that take part in a draft on `plane` in `frame`: those
    /// with a face on it, minus this session's hidden objects.
    fn draft_participants(&self, plane: Plane, frame: Option<ComponentId>) -> Vec<ObjectId> {
        self.doc
            .drawable_objects_on_plane(plane, frame)
            .into_iter()
            .filter(|o| !self.hidden_objects.contains(o))
            .collect()
    }

    fn ctx_plane(&self, world: [f64; 6], instance: Option<u64>) -> Result<DraftCtx, ApiError> {
        let plane = Plane::from_point_normal(
            Point3::new(world[0], world[1], world[2]),
            Vec3::new(world[3], world[4], world[5]),
        )
        .map_err(|e| api_err(&e, &e))?;
        let (plane, frame, pose) = match instance {
            Some(i) => {
                let (def, pose) = self.draft_instance_pose(i)?;
                // Side-keeping, as `begin_sketch_on_plane_in_instance`
                // maps it: the local plane's normal is the pose-inverse
                // image of the user's up, under a mirrored pose too.
                let local = pose
                    .inv
                    .apply_plane_keeping_side(&plane)
                    .map_err(|e| api_err(&e, &e))?;
                (local, Some(def), Some(pose))
            }
            None => (plane, None, None),
        };
        let objects = self.draft_participants(plane, frame);
        Ok(DraftCtx {
            plane,
            frame,
            pose,
            objects,
            scope: DraftScope::Plane,
            form: CtxForm::Plane,
            instance,
        })
    }

    /// One object's own drawing: `plane` in the object's own frame (a
    /// definition member's local space), and the object as the only
    /// participant — how a renderer lists the drawing it overlays on that
    /// object's shared geometry.
    fn ctx_object_plane(&self, local: [f64; 6], object: u64) -> Result<DraftCtx, ApiError> {
        let oid = object_id(object);
        if self.doc.object(oid).is_none() {
            return Err(stale("UnknownObject", "object"));
        }
        let plane = Plane::from_point_normal(
            Point3::new(local[0], local[1], local[2]),
            Vec3::new(local[3], local[4], local[5]),
        )
        .map_err(|e| api_err(&e, &e))?;
        Ok(DraftCtx {
            plane,
            frame: self.doc.object_owner_component(oid),
            pose: None,
            objects: vec![oid],
            scope: DraftScope::Plane,
            form: CtxForm::ObjectPlane,
            instance: None,
        })
    }

    fn ctx_face(
        &self,
        object: u64,
        face: u64,
        instance: Option<u64>,
    ) -> Result<DraftCtx, ApiError> {
        let oid = object_id(object);
        if self.hidden_objects.contains(&oid) {
            return Err(stale("UnknownObject", "object"));
        }
        let obj = self
            .doc
            .object(oid)
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        let plane = obj
            .faces()
            .get(FaceId::from(KeyData::from_ffi(face)))
            .ok_or_else(|| stale("UnknownFace", "face"))?
            .plane;
        let owner = self.doc.object_owner_component(oid);
        let (frame, pose) = match instance {
            Some(i) => {
                let (def, pose) = self.draft_instance_pose(i)?;
                if owner != Some(def) {
                    return Err(ApiError::new(
                        "UnknownObject",
                        "the object is not a member of the instance's definition",
                    ));
                }
                (Some(def), Some(pose))
            }
            None => (owner, None),
        };
        let objects = self.draft_participants(plane, frame);
        Ok(DraftCtx {
            plane,
            frame,
            pose,
            objects,
            scope: DraftScope::Plane,
            form: CtxForm::Face,
            instance,
        })
    }

    /// A sketch's context: its plane and frame. A plain sketch's context is
    /// the whole plane's arrangement; a detached sketch's is the sketch
    /// alone (detached shapes D5), with no object taking part.
    fn ctx_sketch(&self, sketch: u64, instance: Option<u64>) -> Result<DraftCtx, ApiError> {
        let sid = sketch_id(sketch);
        let sk = self
            .doc
            .sketch(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let (plane, detached) = (sk.plane(), sk.detached());
        let owner = self.doc.sketch_owner_component(sid);
        let (frame, pose) = match instance {
            Some(i) => {
                let (def, pose) = self.draft_instance_pose(i)?;
                if owner != Some(def) {
                    return Err(ApiError::new(
                        "UnknownSketch",
                        "the sketch is not owned by the instance's definition",
                    ));
                }
                (Some(def), Some(pose))
            }
            None => (owner, None),
        };
        let (objects, scope) = if detached {
            (Vec::new(), DraftScope::Detached(Some(sid)))
        } else {
            (self.draft_participants(plane, frame), DraftScope::Plane)
        };
        Ok(DraftCtx {
            plane,
            frame,
            pose,
            objects,
            scope,
            form: CtxForm::Sketch,
            instance,
        })
    }

    /// Parses a `{"plane":[..]} | {"face":{..}} | {"sketch":id}` context,
    /// each with an optional `"instance"` (or `{"plane":[..],"object":id}`).
    /// A detached sketch's context is that sketch alone.
    fn ctx_from_json(&self, ctx_json: &str) -> Result<DraftCtx, ApiError> {
        self.ctx_from_value(&parse_ctx(ctx_json)?)
    }

    /// [`Scene::ctx_from_json`] of the parsed JSON.
    fn ctx_from_value(&self, v: &serde_json::Value) -> Result<DraftCtx, ApiError> {
        let instance = match v.get("instance") {
            None | Some(serde_json::Value::Null) => None,
            Some(i) => Some(i.as_u64().ok_or_else(|| {
                ApiError::new("BadContext", "instance must be an unsigned integer id")
            })?),
        };
        let id = |x: &serde_json::Value, what: &str| {
            x.as_u64().ok_or_else(|| {
                ApiError(format!("BadContext: {what} must be an unsigned integer id"))
            })
        };
        if let Some(p) = v.get("plane") {
            let nums: Vec<f64> = p
                .as_array()
                .map(|a| a.iter().filter_map(serde_json::Value::as_f64).collect())
                .unwrap_or_default();
            let arr: [f64; 6] = nums
                .as_slice()
                .try_into()
                .map_err(|_| ApiError::new("BadContext", "plane must be [px,py,pz,nx,ny,nz]"))?;
            if arr.iter().any(|x| !x.is_finite()) {
                return Err(ApiError::new("BadContext", "plane must be finite numbers"));
            }
            match v.get("object") {
                None | Some(serde_json::Value::Null) => self.ctx_plane(arr, instance),
                Some(o) if instance.is_none() => self.ctx_object_plane(arr, id(o, "object")?),
                Some(_) => Err(ApiError::new(
                    "BadContext",
                    "\"object\" and \"instance\" cannot be combined",
                )),
            }
        } else if let Some(f) = v.get("face") {
            self.ctx_face(
                id(&f["object"], "face.object")?,
                id(&f["face"], "face.face")?,
                instance,
            )
        } else if let Some(s) = v.get("sketch") {
            self.ctx_sketch(id(s, "sketch")?, instance)
        } else {
            Err(ApiError::new(
                "BadContext",
                "context must name a \"plane\", \"face\", or \"sketch\"",
            ))
        }
    }

    /// Opens the draft for `ctx` and returns its handle. `handle` is the
    /// counter value to use (replay passes the recorded one).
    fn draft_open(&mut self, ctx: DraftCtx, label: &str) -> Result<u64, ApiError> {
        // A document swap (load) drops the kernel's draft; forget ours too.
        if self.open_draft.is_some() && self.doc.draft_sketch().is_none() {
            self.open_draft = None;
        }
        match ctx.scope {
            DraftScope::Plane => self
                .doc
                .begin_draft(ctx.plane, ctx.frame, &ctx.objects, label),
            DraftScope::Detached(Some(s)) => self.doc.begin_draft_detached(s, label),
            DraftScope::Detached(None) => self
                .doc
                .begin_draft_detached_new(ctx.plane, ctx.frame, label),
        }
        .map_err(doc_err)?;
        self.next_draft_handle += 1;
        let handle = self.next_draft_handle;
        self.open_draft = Some(OpenDraft {
            handle,
            pose: ctx.pose,
        });
        Ok(handle)
    }

    /// Validates `draft` and returns its pose.
    fn draft_check(&mut self, draft: u64) -> Result<Option<Pose>, ApiError> {
        match self.open_draft {
            Some(d) if d.handle == draft && self.doc.draft_sketch().is_some() => Ok(d.pose),
            Some(_) if self.doc.draft_sketch().is_none() => {
                self.open_draft = None;
                Err(stale_draft())
            }
            _ => Err(stale_draft()),
        }
    }

    /// Runs `f` on the draft's working sketch. With `backup`, a failing
    /// edit restores the sketch exactly (the kernel's strong guarantee
    /// held at this boundary even for multi-step edits).
    fn draft_edit<T>(
        &mut self,
        draft: u64,
        backup: bool,
        f: impl FnOnce(&mut Sketch, Option<Pose>) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let pose = self.draft_check(draft)?;
        let sk = self.doc.draft_sketch_mut().ok_or_else(stale_draft)?;
        let saved = backup.then(|| sk.clone());
        let out = f(sk, pose);
        if let (Err(_), Some(saved)) = (&out, saved) {
            *sk = saved;
        }
        out
    }

    /// The working-sketch edges named by `keys`; refuses unknown keys.
    fn draft_edges(&self, keys: &[u64]) -> Result<Vec<SketchEdgeId>, ApiError> {
        let edges = self.doc.draft_edges_by_key(keys);
        let sk = self.doc.draft_sketch().ok_or_else(stale_draft)?;
        let plane = sk.plane();
        let found: std::collections::BTreeSet<u64> = edges
            .iter()
            .filter_map(|&e| {
                let ed = sk.edges().get(e)?;
                Some(kernel::edge_key(
                    &plane,
                    sk.vertices()[ed.from].position,
                    sk.vertices()[ed.to].position,
                ))
            })
            .collect();
        if let Some(k) = keys.iter().find(|k| !found.contains(k)) {
            return Err(ApiError(format!(
                "UnknownEdge: no draft edge has key {}",
                hex(*k)
            )));
        }
        Ok(edges)
    }

    fn draft_world_transform(
        pose: Option<Pose>,
        affine: &[f64],
    ) -> Result<([f64; 12], Transform), ApiError> {
        let rows = parse_affine(affine)?;
        let world = Transform::from_affine(&rows);
        let local = pose.map_or(world, |p| p.local_transform(&world));
        Ok((rows, local))
    }
}

#[wasm_bindgen]
impl Scene {
    // -------------------------------------------------------- lifecycle

    /// Opens the one draft on a world plane (ground = `0,0,0,0,0,1`) and
    /// returns its handle. Participants are the live, non-hidden objects
    /// with a face on the plane. Everything drawn until [`Scene::draft_end`]
    /// lowers as ONE undo step labeled `label`.
    ///
    /// # Errors
    /// `DegenerateVector` (normal too short), `DraftAlreadyOpen`, and
    /// whatever the lift refuses.
    // Scalar xyz args are deliberate boundary ergonomics (docs/dev/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn draft_begin_plane(
        &mut self,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        label: &str,
    ) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let ctx = self.ctx_plane([px, py, pz, nx, ny, nz], None)?;
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginPlane {
            draft: handle,
            px,
            py,
            pz,
            nx,
            ny,
            nz,
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// [`Scene::draft_begin_plane`] inside `instance`'s definition: the
    /// plane is in WORLD space and maps through the pose inverse; every
    /// later point of the draft is mapped likewise.
    // Scalar xyz args are deliberate boundary ergonomics (docs/dev/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn draft_begin_plane_in_instance(
        &mut self,
        instance: u64,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        label: &str,
    ) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let ctx = self.ctx_plane([px, py, pz, nx, ny, nz], Some(instance))?;
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginPlaneInInstance {
            draft: handle,
            instance,
            px,
            py,
            pz,
            nx,
            ny,
            nz,
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// Opens the draft on `face` of `object` (its plane; the object's
    /// owning definition as frame). Refuses a stale or hidden object/face.
    pub fn draft_begin_face(
        &mut self,
        object: u64,
        face: u64,
        label: &str,
    ) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let ctx = self.ctx_face(object, face, None)?;
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginFace {
            draft: handle,
            object,
            face,
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// [`Scene::draft_begin_face`] for a member of `instance`'s definition,
    /// points in world space through the instance pose.
    pub fn draft_begin_face_in_instance(
        &mut self,
        instance: u64,
        object: u64,
        face: u64,
        label: &str,
    ) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let ctx = self.ctx_face(object, face, Some(instance))?;
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginFaceInInstance {
            draft: handle,
            instance,
            object,
            face,
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// Opens the draft on `sketch`'s plane and frame. On a detached sketch
    /// the draft is that sketch's own context: nothing else on the plane
    /// takes part, and everything lowers back into it (a Line chain
    /// continues in the sketch its first segment created).
    pub fn draft_begin_sketch(&mut self, sketch: u64, label: &str) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let ctx = self.ctx_sketch(sketch, None)?;
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginSketch {
            draft: handle,
            sketch,
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// Opens the draft on a context given exactly as [`Scene::plane_units_json`]
    /// takes it (`{"plane":[..]} | {"face":{..}} | {"sketch":id}`, each with
    /// an optional `"instance"`). A caller holding unit keys from
    /// `plane_units_json(ctx)` opens its edit with the SAME `ctx`, so both
    /// resolve the identical plane and the keys name the same edges. A
    /// `{"sketch":id}` context on a detached sketch opens that sketch's own
    /// context, as [`Scene::draft_begin_sketch`] does.
    pub fn draft_begin_ctx(&mut self, ctx_json: &str, label: &str) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let ctx = self.ctx_from_json(ctx_json)?;
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginCtx {
            draft: handle,
            ctx: ctx_json.to_string(),
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// Opens a draft that draws ONE new **detached** shape (detached shapes
    /// §4): drawing that takes part in no plane arrangement, so it neither
    /// splits nor joins the face or the drawing under it, and extruding it
    /// always makes a new Object. `ctx_json` is a `{"plane":[..]}` or
    /// `{"face":{"object":id,"face":id}}` context (each with an optional
    /// `"instance"`), giving the plane and frame only: nothing on the plane
    /// is lifted. Everything drawn until [`Scene::draft_end`] lowers into
    /// one new detached sketch, which the report's `sketches` names; to
    /// continue it, open it with [`Scene::draft_begin_ctx`] (`{"sketch":id}`)
    /// or [`Scene::draft_begin_sketch`].
    ///
    /// # Errors
    /// `BadContext` for a malformed context, a `{"sketch":id}` context, or
    /// the object-scoped `{"plane":[..],"object":id}` form; otherwise as
    /// [`Scene::draft_begin_ctx`].
    pub fn draft_begin_detached(&mut self, ctx_json: &str, label: &str) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        // The form is checked before anything resolves, so a wrong form is
        // refused as one whatever it names.
        let v = parse_ctx(ctx_json)?;
        match ctx_form(&v) {
            Some(CtxForm::Plane | CtxForm::Face) | None => {}
            Some(CtxForm::Sketch) => {
                return Err(ApiError::new(
                    "BadContext",
                    "a detached shape starts on a plane or a face; to continue a detached sketch open it with draft_begin_ctx",
                ));
            }
            Some(CtxForm::ObjectPlane) => {
                return Err(ApiError::new(
                    "BadContext",
                    "a detached shape starts on a plane or a face, not on one object's own drawing",
                ));
            }
        }
        let mut ctx = self.ctx_from_value(&v)?;
        ctx.scope = DraftScope::Detached(None);
        ctx.objects.clear();
        let handle = self.draft_open(ctx, label)?;
        recording::record(recording::RecordedCall::DraftBeginDetached {
            draft: handle,
            ctx: ctx_json.to_string(),
            label: label.to_string(),
        });
        Ok(handle)
    }

    /// Lowers the draft as one undo step and closes it. Returns the report
    /// as JSON `{"objects":[id..],"sketches":[id..],"kept_as_sketch":[[[x,y,z],[x,y,z]],..]}`
    /// (world coordinates). A refusal leaves the draft open.
    pub fn draft_end(&mut self, draft: u64) -> Result<String, ApiError> {
        self.ensure_not_busy()?;
        let pose = self.draft_check(draft)?;
        let (report, change) = self.doc.end_draft().map_err(doc_err)?;
        self.open_draft = None;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DraftEnd { draft });
        let out = serde_json::json!({
            "objects": report.objects.iter().map(|o| o.data().as_ffi()).collect::<Vec<_>>(),
            "sketches": report.sketches.iter().map(|s| s.data().as_ffi()).collect::<Vec<_>>(),
            "kept_as_sketch": report
                .kept_as_sketch
                .iter()
                .map(|&(a, b)| serde_json::json!([world_json(pose, a), world_json(pose, b)]))
                .collect::<Vec<_>>(),
        });
        Ok(out.to_string())
    }

    /// Abandons the draft; the document is untouched. `false` for a stale
    /// handle.
    pub fn draft_cancel(&mut self, draft: u64) -> Result<bool, ApiError> {
        self.ensure_not_busy()?;
        if self.draft_check(draft).is_err() {
            return Ok(false);
        }
        self.doc.cancel_draft();
        self.open_draft = None;
        recording::record(recording::RecordedCall::DraftCancel { draft });
        Ok(true)
    }

    // ------------------------------------------------------ history bracket

    /// Opens a history bracket: every edit until
    /// [`Scene::history_bracket_commit`] lands as ONE undo step, for a
    /// gesture that spans several edits (a selection transform that moves
    /// objects and the drawing on and beside them).
    /// [`Scene::history_bracket_abort`] restores the document exactly as it
    /// was at open. One bracket at a time; it does not nest.
    pub fn history_bracket_open(&mut self) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        if self.open_bracket.is_some() {
            return Err(stale("HistoryBracketOpen", "history bracket"));
        }
        self.open_bracket = Some(self.doc.begin_transaction());
        recording::record(recording::RecordedCall::HistoryBracketOpen);
        Ok(())
    }

    /// Commits the open bracket as one undo step labeled `label`. `false`
    /// when nothing inside it changed the document (no step is added). A
    /// bracket the edits left unbalanced (an undo ran inside it, a session
    /// or drafting gesture left open) refuses and restores the document as
    /// it was at open.
    pub fn history_bracket_commit(&mut self, label: &str) -> Result<bool, ApiError> {
        self.ensure_not_busy()?;
        let Some(txn) = self.open_bracket.take() else {
            return Err(stale("NoHistoryBracket", "history bracket"));
        };
        let meta = kernel::CompoundMeta {
            label: label.to_string(),
            origin: kernel::HistoryOrigin::User,
            coalesce: None,
        };
        match self.doc.commit_transaction(txn, meta) {
            Ok(committed) => {
                recording::record(recording::RecordedCall::HistoryBracketCommit {
                    label: label.to_string(),
                });
                Ok(committed)
            }
            Err(e) => {
                // The refusal restored the document as at open: an abort,
                // and recorded as one so the session replays.
                recording::record(recording::RecordedCall::HistoryBracketAbort);
                self.open_draft = None;
                self.full_resync();
                Err(doc_err(e))
            }
        }
    }

    /// Discards everything since [`Scene::history_bracket_open`]: the
    /// document, its undo and redo stacks included, is exactly as it was.
    /// A no-op without an open bracket.
    pub fn history_bracket_abort(&mut self) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let Some(txn) = self.open_bracket.take() else {
            return Ok(());
        };
        recording::record(recording::RecordedCall::HistoryBracketAbort);
        self.doc.abort_transaction(txn);
        self.open_draft = None;
        self.full_resync();
        Ok(())
    }

    // ------------------------------------------------------------ edits

    /// Inserts a segment with full sticky semantics into the draft.
    // Scalar xyz args are deliberate boundary ergonomics (docs/dev/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn draft_add_segment(
        &mut self,
        draft: u64,
        ax: f64,
        ay: f64,
        az: f64,
        bx: f64,
        by: f64,
        bz: f64,
    ) -> Result<SegmentAddedJs, ApiError> {
        self.ensure_not_busy()?;
        let report = self.draft_edit(draft, false, |sk, pose| {
            let (a, b) = (Point3::new(ax, ay, az), Point3::new(bx, by, bz));
            let (a, b) = pose.map_or((a, b), |p| (p.to_local(a), p.to_local(b)));
            sk.add_segment(a, b).map_err(|e| api_err(&e, &e))
        })?;
        recording::record(recording::RecordedCall::DraftAddSegment {
            draft,
            a: [ax, ay, az],
            b: [bx, by, bz],
        });
        Ok(SegmentAddedJs { inner: report })
    }

    fn draft_curve_with(
        &mut self,
        draft: u64,
        center: &[f64],
        radius: f64,
        kind: kernel::SketchCurveKind,
    ) -> Result<(), ApiError> {
        let c = triple(center, "center")?;
        self.draft_edit(draft, false, |sk, pose| {
            let (center, radius) = match pose {
                Some(p) => (p.to_local(p3(c)), radius / p.scale()?),
                None => (p3(c), radius),
            };
            sk.begin_curve_with_kind(kernel::CurveGeom { center, radius }, kind)
                .map(|_| ())
                .map_err(|e| api_err(&e, &e))
        })
    }

    /// Starts a circle chain (center on the draft plane, radius in meters)
    /// whose facets the following segments approximate.
    pub fn draft_begin_curve_with(
        &mut self,
        draft: u64,
        center: &[f64],
        radius: f64,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_curve_with(draft, center, radius, kernel::SketchCurveKind::Circle)?;
        recording::record(recording::RecordedCall::DraftBeginCurveWith {
            draft,
            center: triple(center, "center")?,
            radius,
        });
        Ok(())
    }

    /// [`Scene::draft_begin_curve_with`] for a regular polygon (its
    /// circumcircle contributes only a center).
    pub fn draft_begin_polygon_with(
        &mut self,
        draft: u64,
        center: &[f64],
        radius: f64,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_curve_with(draft, center, radius, kernel::SketchCurveKind::Polygon)?;
        recording::record(recording::RecordedCall::DraftBeginPolygonWith {
            draft,
            center: triple(center, "center")?,
            radius,
        });
        Ok(())
    }

    /// Opens a curve bracket with no analytic circle: segments added until
    /// `draft_end_curve` are one curve chain that selects and erases as one
    /// (an arc too flat to have a circle, drawn as its chord).
    pub fn draft_begin_curve(&mut self, draft: u64) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_edit(draft, false, |sk, _| {
            sk.begin_curve();
            Ok(())
        })?;
        recording::record(recording::RecordedCall::DraftBeginCurve { draft });
        Ok(())
    }

    /// Closes the curve chain opened by `draft_begin_curve`,
    /// `draft_begin_curve_with`, or `draft_begin_polygon_with`.
    pub fn draft_end_curve(&mut self, draft: u64) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_edit(draft, false, |sk, _| {
            sk.end_curve();
            Ok(())
        })?;
        recording::record(recording::RecordedCall::DraftEndCurve { draft });
        Ok(())
    }

    /// Erases the draft edges with these keys (a unit's edge keys from
    /// `plane_units_json`). Unknown keys refuse; a solid's own edges are
    /// not drawing edges and cannot be named.
    pub fn draft_remove_edges(&mut self, draft: u64, keys: &[u64]) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_check(draft)?;
        let edges = self.draft_edges(keys)?;
        self.draft_edit(draft, true, |sk, _| {
            for &e in &edges {
                // An earlier removal may have already taken this one with it.
                if sk.edges().contains_key(e) {
                    sk.remove_edge(e).map_err(|e| api_err(&e, &e))?;
                }
            }
            Ok(())
        })?;
        recording::record(recording::RecordedCall::DraftRemoveEdges {
            draft,
            keys: keys.to_vec(),
        });
        Ok(())
    }

    /// Moves the draft edges with these keys by `affine` (world, 12 floats
    /// row-major 3x4), arranging where they land.
    pub fn draft_transform_edges(
        &mut self,
        draft: u64,
        keys: &[u64],
        affine: &[f64],
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let pose = self.draft_check(draft)?;
        let (rows, t) = Self::draft_world_transform(pose, affine)?;
        let edges = self.draft_edges(keys)?;
        self.draft_edit(draft, true, |sk, _| {
            sk.apply_transform_edges(&edges, &t)
                .map_err(|e| api_err(&e, &e))
        })?;
        recording::record(recording::RecordedCall::DraftTransformEdges {
            draft,
            keys: keys.to_vec(),
            affine: rows,
        });
        Ok(())
    }

    /// Copies the draft edges with these keys by `affine` (as
    /// [`Scene::draft_transform_edges`]), leaving the originals.
    pub fn draft_copy_edges(
        &mut self,
        draft: u64,
        keys: &[u64],
        affine: &[f64],
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let pose = self.draft_check(draft)?;
        let (rows, t) = Self::draft_world_transform(pose, affine)?;
        let edges = self.draft_edges(keys)?;
        self.draft_edit(draft, true, |sk, _| {
            sk.copy_edges_transformed(&edges, &t)
                .map_err(|e| api_err(&e, &e))
        })?;
        recording::record(recording::RecordedCall::DraftCopyEdges {
            draft,
            keys: keys.to_vec(),
            affine: rows,
        });
        Ok(())
    }

    /// Drags the vertex at `(x,y,z)` to `(tx,ty,tz)`, arranging.
    // Scalar xyz args are deliberate boundary ergonomics (docs/dev/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn draft_move_vertex(
        &mut self,
        draft: u64,
        x: f64,
        y: f64,
        z: f64,
        tx: f64,
        ty: f64,
        tz: f64,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let pose = self.draft_check(draft)?;
        let at = pose.map_or(Point3::new(x, y, z), |p| p.to_local(Point3::new(x, y, z)));
        let to = pose.map_or(Point3::new(tx, ty, tz), |p| {
            p.to_local(Point3::new(tx, ty, tz))
        });
        let v = self
            .doc
            .draft_vertex_at(at)
            .ok_or_else(|| ApiError::new("UnknownVertex", "no draft vertex at that position"))?;
        self.draft_edit(draft, true, |sk, _| {
            sk.move_vertex_arranging(v, to)
                .map(|_| ())
                .map_err(|e| api_err(&e, &e))
        })?;
        recording::record(recording::RecordedCall::DraftMoveVertex {
            draft,
            at: [x, y, z],
            to: [tx, ty, tz],
        });
        Ok(())
    }

    /// Offsets the region with key `region_key` by `distance` (world
    /// length; positive grows the material).
    pub fn draft_offset_region(
        &mut self,
        draft: u64,
        region_key: u64,
        distance: f64,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_check(draft)?;
        let region = self.doc.draft_region_by_key(region_key).ok_or_else(|| {
            ApiError(format!(
                "UnknownRegion: no draft region has key {}",
                hex(region_key)
            ))
        })?;
        self.draft_edit(draft, true, |sk, pose| {
            let d = match pose {
                Some(p) => distance / p.scale()?,
                None => distance,
            };
            sk.offset_region(region, d)
                .map(|_| ())
                .map_err(|e| api_err(&e, &e))
        })?;
        recording::record(recording::RecordedCall::DraftOffsetRegion {
            draft,
            region: region_key,
            distance,
        });
        Ok(())
    }

    /// Rebuilds the circle with key `curve_key` at `segments` facets.
    pub fn draft_refacet_curve(
        &mut self,
        draft: u64,
        curve_key: u64,
        segments: u32,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.draft_check(draft)?;
        let curves = self.doc.draft_curves_by_key(curve_key);
        if curves.is_empty() {
            return Err(ApiError(format!(
                "UnknownCurve: no draft circle has key {}",
                hex(curve_key)
            )));
        }
        self.draft_edit(draft, true, |sk, _| {
            for c in &curves {
                sk.refacet_curve(*c, segments as usize)
                    .map_err(|e| api_err(&e, &e))?;
            }
            Ok(())
        })?;
        recording::record(recording::RecordedCall::DraftRefacetCurve {
            draft,
            curve: curve_key,
            segments,
        });
        Ok(())
    }

    // ---------------------------------------------------------- queries

    /// The planes `object` carries drawing on, in its OWN frame (a definition
    /// member's local space), as `[px,py,pz,nx,ny,nz]` per plane, flattened:
    /// planes where a drawn shape split one of its faces, and planes of
    /// sketches it owns. List each with
    /// `plane_units_json({"plane":[..],"object":id})`. Empty for a stale or
    /// hidden object.
    pub fn object_drawing_planes(&self, object: u64) -> Vec<f64> {
        let mut out = Vec::new();
        for p in self.doc.object_drawing_planes(object_id(object)) {
            let n = p.normal();
            let o = Point3::ORIGIN + n * (-p.signed_distance(Point3::ORIGIN));
            out.extend([o.x, o.y, o.z, n.x, n.y, n.z]);
        }
        out
    }

    /// The drawing on a context's plane as units and regions, as JSON
    /// (all points world; keys lowercase 16-digit hex strings):
    /// `{"units":[{"key","edges":[{"key","a":[x,y,z],"b":[x,y,z],
    /// "store":{"kind":"topology","object":id}|{"kind":"owned","object":id,"sketch":id}|{"kind":"free","sketch":id[,"detached":true]},
    /// "curve":{"center":[..],"radius":r|null}|null}],
    /// "curves":[{"key","curve_key","kind":"circle"|"polygon","center":[..],"radius":r|null,"edges":["<hex>"..]}]}],
    /// (a radius is null inside a non-uniformly scaled instance, where the
    /// circle is an ellipse),
    /// (a curve run is the piece of one circle, arc, or polygon a click
    /// selects: its facets reachable through points nothing else uses),
    /// "regions":[{"key","outer":[[x,y,z]..],"holes":[[[..]..]..],
    /// "over":{"object":id,"face":id|null,"why":"sliver"|"other"|null}|null,
    /// "extrude":{"sketch":id,"region":id}|null}]}` (`why` says why a region
    /// over a solid has no face).
    /// `ctx_json` is `{"plane":[px,py,pz,nx,ny,nz]}`, `{"face":{"object":id,"face":id}}`
    /// or `{"sketch":id}`, each with an optional `"instance":id`; or
    /// `{"plane":[..],"object":id}` — that plane in the object's own frame
    /// with only that object taking part. A free store that is a detached
    /// sketch carries `"detached":true` (omitted otherwise). A plane or face
    /// context lists the plane's arrangement, which no detached sketch is
    /// in; `{"sketch":id}` on a detached sketch lists exactly that sketch.
    /// Read-only.
    pub fn plane_units_json(&self, ctx_json: &str) -> Result<String, ApiError> {
        let ctx = self.ctx_from_json(ctx_json)?;
        Ok(self.plane_units_value(&ctx)?.0.to_string())
    }

    /// Every plane the document's free world drawing lies on, with that
    /// plane's listing, as one JSON array `[{plane: [px,py,pz,nx,ny,nz],
    /// units: <the plane_units_json value>}]` ([`Document::free_plane_units`]).
    /// The Outliner reads the whole document's drawing through this in one
    /// call rather than one `plane_units_json` per plane, which on a model
    /// of tens of thousands of small sketches took minutes.
    ///
    /// A detached sketch (in no plane's arrangement) is its own entry,
    /// after the planes: `{plane, sketch: id, detached: true, units}`.
    pub fn plane_units_all_json(&self) -> Result<String, ApiError> {
        let mut out = Vec::new();
        for (listing, units) in self
            .doc
            .free_plane_units(None, &self.hidden_objects.iter().copied().collect())
        {
            let plane = listing.plane();
            let n = plane.normal();
            let o = Point3::ORIGIN + n * (-plane.signed_distance(Point3::ORIGIN));
            let mut entry = serde_json::json!({
                "plane": [o.x, o.y, o.z, n.x, n.y, n.z],
                "units": self.units_value(None, &units)?,
            });
            if let kernel::PlaneListing::Detached { sketch, .. } = listing {
                entry["sketch"] = serde_json::json!(sketch.data().as_ffi());
                entry["detached"] = serde_json::json!(true);
            }
            out.push(entry);
        }
        Ok(serde_json::Value::Array(out).to_string())
    }

    /// The exact live preview of offsetting the region `region_key` of a
    /// context's drawing by `distance` (positive grows it), read without
    /// opening a draft: the kernel's offset loops in the legacy
    /// `[loopCount, n0, x,y,z * n0, n1, ...]` wire format, outer loop first,
    /// world coordinates. What `draft_offset_region` would then arrange into
    /// the drawing starts from exactly these loops. On a `{"sketch":id}`
    /// context naming a detached sketch the region is that sketch's own.
    pub fn offset_region_preview(
        &self,
        ctx_json: &str,
        region_key: u64,
        distance: f64,
    ) -> Result<Vec<f64>, ApiError> {
        let ctx = self.ctx_from_json(ctx_json)?;
        let profile = match ctx.scope {
            DraftScope::Detached(Some(s)) => self.doc.detached_region_profile(s, region_key),
            _ => self
                .doc
                .plane_region_profile(ctx.plane, ctx.frame, &ctx.objects, region_key),
        }
        .map_err(doc_err)?;
        let d = match ctx.pose {
            Some(p) => distance / p.scale()?,
            None => distance,
        };
        let off = kernel::offset_profile(&profile, d).map_err(|e| api_err(&e, &e))?;
        let mut out: Vec<f64> = vec![(1 + off.holes.len()) as f64];
        for lp in std::iter::once(&off.outer).chain(off.holes.iter()) {
            out.push(lp.points.len() as f64);
            for &p in &lp.points {
                let q = ctx.pose.map_or(p, |pose| pose.to_world(p));
                out.extend([q.x, q.y, q.z]);
            }
        }
        Ok(out)
    }

    /// Picks in a context's plane: the ray (`[ox,oy,oz,dx,dy,dz]`, world)
    /// meets the plane; the nearest drawing edge within `tolerance_world`
    /// gives `{"edge":"<hex>","unit":"<hex>","curve":"<hex>"|null,"run":"<hex>"|null}`
    /// (`run`: the edge's curve run key, for circles, arcs, and polygons); else
    /// the region holding the point gives `{"region":"<hex>"}`; else (or a
    /// parallel/behind-the-origin ray) `"null"`. A solid's own edges never
    /// pick.
    ///
    /// A `plane` or `face` context also probes the detached sketches on
    /// that plane in that frame (detached shapes D16), which are in no
    /// plane's arrangement: the nearest edge within tolerance wins, a tie
    /// going to the detached sketch; with no edge in reach, a detached
    /// region holding the point (the smallest, when several do) wins over
    /// the plane's own. A hit on a detached sketch carries
    /// `"ctx":{"sketch":id}` (plus `"instance":id` when the request had
    /// one), the context its keys belong to; without `ctx` the keys are the
    /// request's own context's. A world detached sketch that is hidden (by
    /// a tag or a hidden group) is not probed. A `{"sketch":id}` context on
    /// a detached sketch picks within that sketch only.
    pub fn pick_plane_unit(
        &self,
        ctx_json: &str,
        ray: &[f64],
        tolerance_world: f64,
    ) -> Result<String, ApiError> {
        let r: &[f64; 6] = ray
            .try_into()
            .map_err(|_| ApiError::new("BadVector", "ray must be [ox,oy,oz,dx,dy,dz]"))?;
        if r.iter().any(|x| !x.is_finite()) || !tolerance_world.is_finite() {
            return Err(ApiError::new(
                "BadVector",
                "ray and tolerance must be finite",
            ));
        }
        let ctx = self.ctx_from_json(ctx_json)?;
        let (o, d) = (Point3::new(r[0], r[1], r[2]), Vec3::new(r[3], r[4], r[5]));
        // The hit is found in the plane's own frame; distances are measured
        // in the world, so any pose (a stretched instance too) picks.
        let (o, d) = match ctx.pose {
            Some(p) => (p.to_local(o), p.inv.apply_vector(d)),
            None => (o, d),
        };
        let world = |q: Point3| ctx.pose.map_or(q, |p| p.to_world(q));
        let n = ctx.plane.normal();
        let den = n.dot(d);
        if den.abs() < 1e-12 {
            return Ok("null".to_string());
        }
        let t = -ctx.plane.signed_distance(o) / den;
        if t < 0.0 {
            return Ok("null".to_string());
        }
        let hit = o + d * t;
        let hit_world = world(hit);
        // The context's own listing first, then (plane and face contexts)
        // each detached sketch on the plane, in its own context.
        let mut listings: Vec<(Option<SketchId>, PlaneUnits)> = vec![(None, self.ctx_units(&ctx)?)];
        if matches!(ctx.form, CtxForm::Plane | CtxForm::Face) {
            for s in self.doc.detached_sketches_on(ctx.plane, ctx.frame) {
                // Only a shown shape takes a click from what is under it
                // (a world sketch hidden by a tag or group, as
                // `pick_sketch_region` skips; a definition's sketch is
                // shown through its instances).
                if ctx.frame.is_none() && !self.doc.sketch_shown(s) {
                    continue;
                }
                listings.push((Some(s), self.doc.detached_sketch_units(s).map_err(doc_err)?));
            }
        }
        let mut best: Option<(f64, usize, &kernel::DraftUnit, &kernel::DraftEdge)> = None;
        for (i, (sep, units)) in listings.iter().enumerate() {
            for u in &units.units {
                for e in &u.edges {
                    let dist = dist_to_segment(hit_world, world(e.a), world(e.b));
                    if dist > tolerance_world {
                        continue;
                    }
                    // Nearest wins; a tie goes to a detached sketch over
                    // the plane's own drawing.
                    let better = best.is_none_or(|(b, bi, _, _)| {
                        dist < b || (dist <= b && sep.is_some() && listings[bi].0.is_none())
                    });
                    if better {
                        best = Some((dist, i, u, e));
                    }
                }
            }
        }
        let with_ctx = |mut v: serde_json::Value, i: usize| {
            if let Some(s) = listings[i].0 {
                let mut c = serde_json::json!({ "sketch": s.data().as_ffi() });
                if let Some(inst) = ctx.instance {
                    c["instance"] = serde_json::json!(inst);
                }
                v["ctx"] = c;
            }
            v.to_string()
        };
        if let Some((_, i, u, e)) = best {
            let curve = e
                .curve_key
                .map(|k| serde_json::Value::String(hex(k)))
                .unwrap_or(serde_json::Value::Null);
            // The curve run the edge belongs to (circle, arc, or polygon).
            let run = u
                .curves
                .iter()
                .find(|r| r.edges.contains(&e.key))
                .map(|r| serde_json::Value::String(hex(r.key)))
                .unwrap_or(serde_json::Value::Null);
            return Ok(with_ctx(
                serde_json::json!({
                    "edge": hex(e.key),
                    "unit": hex(u.key),
                    "curve": curve,
                    "run": run,
                }),
                i,
            ));
        }
        let holds = |reg: &kernel::DraftRegion| {
            in_ring(hit, &reg.outer, n) && !reg.holes.iter().any(|h| in_ring(hit, h, n))
        };
        // A detached region holding the point is what the cursor is on
        // (the tie goes to it); the innermost when detached shapes overlap.
        let mut inner: Option<(f64, usize, u64)> = None;
        for (i, (_, units)) in listings.iter().enumerate().skip(1) {
            for reg in units.regions.iter().filter(|r| holds(r)) {
                let area = ring_area(&reg.outer, n)
                    - reg.holes.iter().map(|h| ring_area(h, n)).sum::<f64>();
                if inner.is_none_or(|(a, _, _)| area < a) {
                    inner = Some((area, i, reg.key));
                }
            }
        }
        if let Some((_, i, key)) = inner {
            return Ok(with_ctx(serde_json::json!({ "region": hex(key) }), i));
        }
        if let Some(reg) = listings[0].1.regions.iter().find(|r| holds(r)) {
            return Ok(serde_json::json!({ "region": hex(reg.key) }).to_string());
        }
        Ok("null".to_string())
    }
}

impl Scene {
    /// The plane-units JSON value for `ctx` (and the raw units).
    fn plane_units_value(
        &self,
        ctx: &DraftCtx,
    ) -> Result<(serde_json::Value, kernel::PlaneUnits), ApiError> {
        let units = self.ctx_units(ctx)?;
        let value = self.units_value(ctx.pose, &units)?;
        Ok((value, units))
    }

    /// The units of a context: its plane's arrangement, or a detached
    /// sketch alone (a new detached shape's context lists nothing yet).
    fn ctx_units(&self, ctx: &DraftCtx) -> Result<PlaneUnits, ApiError> {
        match ctx.scope {
            DraftScope::Plane => self.doc.plane_units(ctx.plane, ctx.frame, &ctx.objects),
            DraftScope::Detached(Some(s)) => self.doc.detached_sketch_units(s),
            DraftScope::Detached(None) => Ok(PlaneUnits::default()),
        }
        .map_err(doc_err)
    }

    /// The JSON of one plane's listing, points through `pose` when the
    /// plane is a definition's seen through a placement.
    fn units_value(
        &self,
        pose: Option<Pose>,
        units: &kernel::PlaneUnits,
    ) -> Result<serde_json::Value, ApiError> {
        let w = |p: Point3| world_json(pose, p);
        // A radius in the world: scaled by a similarity pose; a stretched
        // instance turns the circle into an ellipse, which has none (null).
        let scale = |r: f64| -> Result<serde_json::Value, ApiError> {
            Ok(match pose {
                Some(p) => p
                    .scale()
                    .ok()
                    .map_or(serde_json::Value::Null, |s| serde_json::json!(r * s)),
                None => serde_json::json!(r),
            })
        };
        let mut unit_vals = Vec::new();
        for u in &units.units {
            let mut edges = Vec::new();
            for e in &u.edges {
                let store = match e.store {
                    DraftStore::Topology(o) => {
                        serde_json::json!({"kind":"topology","object":o.data().as_ffi()})
                    }
                    DraftStore::Owned(o, s) => serde_json::json!({
                        "kind":"owned","object":o.data().as_ffi(),"sketch":s.data().as_ffi()
                    }),
                    DraftStore::Free(s) => {
                        let mut v = serde_json::json!({"kind":"free","sketch":s.data().as_ffi()});
                        // A detached shape says so; omitted otherwise, so a
                        // plain listing reads as it always did.
                        if self.doc.sketch(s).is_some_and(Sketch::detached) {
                            v["detached"] = serde_json::json!(true);
                        }
                        v
                    }
                };
                let curve = match e.curve {
                    Some(g) => {
                        serde_json::json!({"center": w(g.center), "radius": scale(g.radius)?})
                    }
                    None => serde_json::Value::Null,
                };
                edges.push(serde_json::json!({
                    "key": hex(e.key),
                    "a": w(e.a),
                    "b": w(e.b),
                    "store": store,
                    "curve": curve,
                }));
            }
            let mut curves = Vec::new();
            for r in &u.curves {
                curves.push(serde_json::json!({
                    "key": hex(r.key),
                    "curve_key": hex(r.curve_key),
                    "kind": match r.kind {
                        kernel::SketchCurveKind::Circle => "circle",
                        kernel::SketchCurveKind::Polygon => "polygon",
                    },
                    "center": w(r.geom.center),
                    "radius": scale(r.geom.radius)?,
                    "edges": r.edges.iter().map(|&k| hex(k)).collect::<Vec<_>>(),
                }));
            }
            unit_vals
                .push(serde_json::json!({"key": hex(u.key), "edges": edges, "curves": curves}));
        }
        let regions: Vec<serde_json::Value> = units
            .regions
            .iter()
            .map(|r| {
                let over = match r.over {
                    Some((o, f)) => serde_json::json!({
                        "object": o.data().as_ffi(),
                        "face": f.map(|f| f.data().as_ffi()),
                        "why": r.why.map(|w| match w {
                            kernel::DraftRegionWhy::Sliver => "sliver",
                            kernel::DraftRegionWhy::Other => "other",
                        }),
                    }),
                    None => serde_json::Value::Null,
                };
                let extrude = match r.extrude {
                    Some((s, rid)) => serde_json::json!({
                        "sketch": s.data().as_ffi(),
                        "region": rid.data().as_ffi(),
                    }),
                    None => serde_json::Value::Null,
                };
                serde_json::json!({
                    "key": hex(r.key),
                    "outer": r.outer.iter().map(|&p| w(p)).collect::<Vec<_>>(),
                    "holes": r.holes.iter().map(|h| h.iter().map(|&p| w(p)).collect::<Vec<_>>()).collect::<Vec<_>>(),
                    "over": over,
                    "extrude": extrude,
                })
            })
            .collect();
        Ok(serde_json::json!({"units": unit_vals, "regions": regions}))
    }

    /// Replay support: the next `draft_begin_*` mints exactly `handle`.
    pub(crate) fn draft_set_next_handle(&mut self, handle: u64) {
        self.next_draft_handle = handle.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const GROUND: &str = r#"{"plane":[0,0,0,0,0,1]}"#;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn units(scene: &Scene, ctx: &str) -> Value {
        json(&scene.plane_units_json(ctx).unwrap())
    }

    fn rect(scene: &mut Scene, d: u64, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) {
        for (a, b) in [
            ([x0, y0], [x1, y0]),
            ([x1, y0], [x1, y1]),
            ([x1, y1], [x0, y1]),
            ([x0, y1], [x0, y0]),
        ] {
            scene
                .draft_add_segment(d, a[0], a[1], z, b[0], b[1], z)
                .unwrap();
        }
    }

    /// Drafts a ground rectangle, ends it, and extrudes the region the
    /// drafting query names.
    fn ground_box(scene: &mut Scene, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> u64 {
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Rectangle")
            .unwrap();
        rect(scene, d, x0, y0, x1, y1, 0.0);
        scene.draft_end(d).unwrap();
        let u = units(scene, GROUND);
        let ex = &u["regions"][0]["extrude"];
        let (s, r) = (
            ex["sketch"].as_u64().unwrap(),
            ex["region"].as_u64().unwrap(),
        );
        scene.extrude_region(s, r, h).unwrap()
    }

    fn top_face(scene: &Scene, obj: u64) -> u64 {
        let o = scene.doc.object(object_id(obj)).unwrap();
        o.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.9)
            .map(|(id, _)| id.data().as_ffi())
            .unwrap()
    }

    fn edge_keys(u: &Value) -> Vec<u64> {
        u["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|un| un["edges"].as_array().unwrap())
            .map(|e| u64::from_str_radix(e["key"].as_str().unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn ground_draft_rectangle_is_one_region_with_extrude() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Rectangle")
            .unwrap();
        rect(&mut scene, d, 0.0, 0.0, 2.0, 1.0, 0.0);
        let report = json(&scene.draft_end(d).unwrap());
        assert_eq!(report["objects"].as_array().unwrap().len(), 0);
        assert_eq!(report["sketches"].as_array().unwrap().len(), 1);
        let u = units(&scene, GROUND);
        assert_eq!(u["regions"].as_array().unwrap().len(), 1);
        let r = &u["regions"][0];
        assert!(r["over"].is_null());
        assert!(r["extrude"]["sketch"].is_u64() && r["extrude"]["region"].is_u64());
        assert_eq!(r["key"].as_str().unwrap().len(), 16);
        let e = &u["units"][0]["edges"][0];
        assert_eq!(e["store"]["kind"], "free");
        assert!(e["curve"].is_null());
    }

    /// Keys listed for a context name the same edges in a draft opened on
    /// that context, and on a face context reached through a plane rebuilt
    /// from the face's numbers (last-bit differences in the normal).
    #[test]
    fn keys_from_a_context_name_edges_in_a_draft_on_it() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Rectangle").unwrap();
        rect(&mut scene, d, 0.5, 0.5, 1.5, 1.5, 1.0);
        scene.draft_end(d).unwrap();
        let tilt = 1.0 + f64::EPSILON;
        for ctx in [
            r#"{"plane":[0,0,1,0,0,1]}"#.to_string(),
            format!(r#"{{"plane":[0,0,1,0,0,{tilt}]}}"#),
        ] {
            let u = units(&scene, &ctx);
            let key = u["units"][0]["edges"][0]["key"]
                .as_str()
                .unwrap()
                .to_string();
            let k = u64::from_str_radix(&key, 16).unwrap();
            let d = scene.draft_begin_ctx(&ctx, "Erase").unwrap();
            scene.draft_remove_edges(d, &[k]).unwrap();
            assert!(scene.draft_cancel(d).unwrap());
        }
    }

    /// The offset preview is the kernel's own offset of the listed region,
    /// read without a draft: a 2 x 1 rectangle grown by 0.5 spans 3 x 2.
    #[test]
    fn offset_region_preview_is_the_kernels_offset() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Rectangle")
            .unwrap();
        rect(&mut scene, d, 0.0, 0.0, 2.0, 1.0, 0.0);
        scene.draft_end(d).unwrap();
        let u = units(&scene, GROUND);
        let key = u64::from_str_radix(u["regions"][0]["key"].as_str().unwrap(), 16).unwrap();
        let before = scene.doc.save();
        let data = scene.offset_region_preview(GROUND, key, 0.5).unwrap();
        assert_eq!(data[0], 1.0, "one loop");
        let n = data[1] as usize;
        let xs: Vec<f64> = (0..n).map(|i| data[2 + 3 * i]).collect();
        let ys: Vec<f64> = (0..n).map(|i| data[3 + 3 * i]).collect();
        let span = |v: &[f64]| {
            v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!((span(&xs) - 3.0).abs() < 1e-9 && (span(&ys) - 2.0).abs() < 1e-9);
        assert_eq!(scene.doc.save(), before, "a preview changes nothing");
        assert!(
            scene.offset_region_preview(GROUND, key, -0.6).is_err(),
            "a collapse refuses"
        );
    }

    /// An object's drawing planes come back in its own frame, and the
    /// object-scoped context lists only that object's drawing on them.
    #[test]
    fn object_drawing_planes_list_through_the_object_context() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        assert!(scene.object_drawing_planes(b).is_empty());
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Rectangle").unwrap();
        rect(&mut scene, d, 0.5, 0.5, 1.5, 1.5, 1.0);
        scene.draft_end(d).unwrap();
        let planes = scene.object_drawing_planes(b);
        assert_eq!(planes.len(), 6);
        assert!((planes[2] - 1.0).abs() < 1e-12 && (planes[5] - 1.0).abs() < 1e-12);
        let ctx = format!(
            r#"{{"plane":[{},{},{},{},{},{}],"object":{b}}}"#,
            planes[0], planes[1], planes[2], planes[3], planes[4], planes[5]
        );
        let u = units(&scene, &ctx);
        assert_eq!(u["units"].as_array().unwrap().len(), 1);
        assert!(
            scene
                .plane_units_json(&format!(
                    r#"{{"plane":[0,0,1,0,0,1],"object":{b},"instance":1}}"#
                ))
                .is_err()
        );
    }

    /// A circle drawn on a box top inside a rectangle keeps its exact circle
    /// through pushes of both faces and the undos of them (the regression
    /// once caught by a recorded session, whose face-imprint calls are gone).
    #[test]
    fn an_enclosed_circle_keeps_its_circle_through_push_pull_and_undo() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 1.0, 1.0, 1.0);
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Circle").unwrap();
        let (c, r, n) = ([0.5, 0.5, 1.0], 0.1792616828425053, 44);
        scene.draft_begin_curve_with(d, &c, r).unwrap();
        for i in 0..n {
            let (a0, a1) = (
                std::f64::consts::TAU * i as f64 / n as f64,
                std::f64::consts::TAU * (i + 1) as f64 / n as f64,
            );
            scene
                .draft_add_segment(
                    d,
                    0.5 + r * a0.cos(),
                    0.5 + r * a0.sin(),
                    1.0,
                    0.5 + r * a1.cos(),
                    0.5 + r * a1.sin(),
                    1.0,
                )
                .unwrap();
        }
        scene.draft_end_curve(d).unwrap();
        scene.draft_end(d).unwrap();
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Rectangle").unwrap();
        rect(&mut scene, d, 0.0886, 0.0886, 0.9114, 0.9114, 1.0);
        scene.draft_end(d).unwrap();

        let top = r#"{"plane":[0,0,1,0,0,1]}"#;
        let face_at = |scene: &Scene, x: f64, y: f64| -> u64 {
            let u = units(scene, top);
            let hit = scene
                .pick_plane_unit(top, &[x, y, 5.0, 0.0, 0.0, -1.0], 1e-6)
                .unwrap();
            let key = json(&hit)["region"].as_str().unwrap().to_string();
            u["regions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["key"] == key)
                .unwrap()["over"]["face"]
                .as_u64()
                .unwrap()
        };
        let circle_kept = |scene: &Scene| {
            let u = units(scene, top);
            u["units"].as_array().unwrap().iter().any(|un| {
                un["curves"].as_array().unwrap().iter().any(|run| {
                    run["kind"] == "circle" && run["edges"].as_array().unwrap().len() == n
                })
            })
        };
        assert!(circle_kept(&scene));
        let ring = face_at(&scene, 0.15, 0.5);
        scene.push_pull(b, ring, -0.25138878721056135).unwrap();
        let disc = face_at(&scene, 0.5, 0.5);
        scene.push_pull(b, disc, 0.485440701784434).unwrap();
        scene.scene_undo().unwrap();
        scene.scene_undo().unwrap();
        assert!(circle_kept(&scene), "back after undoing both pushes");
        for dist in [0.4942099165154381, 0.4319704101192719, 0.4794122863554196] {
            let disc = face_at(&scene, 0.5, 0.5);
            scene.push_pull(b, disc, dist).unwrap();
            scene.scene_undo().unwrap();
            assert!(circle_kept(&scene), "back after undoing a push of {dist}");
        }
        let ring = face_at(&scene, 0.15, 0.5);
        scene.push_pull(b, ring, -0.34953480563320155).unwrap();
        scene.scene_undo().unwrap();
        assert!(circle_kept(&scene), "the circle is still a circle");
    }

    /// A history bracket lands several edits as one undo step; an aborted
    /// one leaves the document, history included, exactly as it was.
    #[test]
    fn a_history_bracket_is_one_step_or_nothing() {
        let mut scene = Scene::new();
        let draw = |scene: &mut Scene, x: f64| {
            let d = scene
                .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Rectangle")
                .unwrap();
            rect(scene, d, x, 0.0, x + 1.0, 1.0, 0.0);
            scene.draft_end(d).unwrap();
        };
        draw(&mut scene, 0.0);
        let before = scene.save();
        let depth = scene.undo_depth();

        scene.history_bracket_open().unwrap();
        assert!(
            scene.history_bracket_open().is_err(),
            "brackets do not nest"
        );
        draw(&mut scene, 2.0);
        draw(&mut scene, 4.0);
        scene.history_bracket_abort().unwrap();
        assert_eq!(scene.save(), before, "abort restores the document");
        assert_eq!(scene.undo_depth(), depth);

        scene.history_bracket_open().unwrap();
        draw(&mut scene, 2.0);
        draw(&mut scene, 4.0);
        assert!(scene.history_bracket_commit("Move").unwrap());
        assert_eq!(scene.undo_depth(), depth + 1, "one step");
        let after = scene.save();
        scene.scene_undo().unwrap();
        assert_eq!(scene.save(), before, "one undo takes both");
        scene.scene_redo().unwrap();
        assert_eq!(scene.save(), after);

        scene.history_bracket_open().unwrap();
        assert!(!scene.history_bracket_commit("Nothing").unwrap());
        assert_eq!(
            scene.undo_depth(),
            depth + 1,
            "an empty bracket adds no step"
        );
        assert!(scene.history_bracket_commit("Again").is_err());
    }

    /// Inside a stretched instance a drawn circle still lists and picks: its
    /// radius is null (an ellipse has none) and picks measure in the world.
    #[test]
    fn a_circle_in_a_stretched_instance_lists_and_picks() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let inst = scene.make_component(&[0], &[b]).unwrap();
        let def = scene.instance_def(inst).unwrap();
        let member = scene.component_member_objects(def)[0];
        let f = top_face(&scene, member);
        // Draw the circle in definition space, then stretch the instance.
        let d = scene.draft_begin_face(member, f, "Circle").unwrap();
        scene
            .draft_begin_curve_with(d, &[1.0, 1.0, 1.0], 0.5)
            .unwrap();
        for i in 0..16 {
            let (a0, a1) = (
                std::f64::consts::TAU * i as f64 / 16.0,
                std::f64::consts::TAU * (i + 1) as f64 / 16.0,
            );
            scene
                .draft_add_segment(
                    d,
                    1.0 + 0.5 * a0.cos(),
                    1.0 + 0.5 * a0.sin(),
                    1.0,
                    1.0 + 0.5 * a1.cos(),
                    1.0 + 0.5 * a1.sin(),
                    1.0,
                )
                .unwrap();
        }
        scene.draft_end_curve(d).unwrap();
        scene.draft_end(d).unwrap();
        let stretch = [2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.transform_instance(inst, &stretch).unwrap();
        let ctx = format!(r#"{{"face":{{"object":{member},"face":{f}}},"instance":{inst}}}"#);
        let u = units(&scene, &ctx);
        let run = &u["units"][0]["curves"][0];
        assert!(run["radius"].is_null(), "an ellipse has no radius: {run}");
        // The rim's world point at local (1.5, 1) is (3, 1).
        let hit = json(
            &scene
                .pick_plane_unit(&ctx, &[3.0, 1.0, 5.0, 0.0, 0.0, -1.0], 0.05)
                .unwrap(),
        );
        assert!(hit["edge"].is_string() && hit["run"].is_string(), "{hit}");
    }

    #[test]
    fn box_face_rectangle_is_topology() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Rectangle").unwrap();
        rect(&mut scene, d, 0.5, 0.5, 1.5, 1.5, 1.0);
        let report = json(&scene.draft_end(d).unwrap());
        assert_eq!(report["objects"], serde_json::json!([b]));
        let ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let u = units(&scene, &ctx);
        let over: Vec<&Value> = u["regions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["over"]["face"].is_u64())
            .collect();
        assert!(!over.is_empty(), "a region is a face now: {u}");
        let kinds: Vec<&str> = u["units"][0]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["store"]["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.iter().all(|&k| k == "topology"), "{kinds:?}");
    }

    #[test]
    fn loose_line_on_a_face_is_owned_sketch() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Line").unwrap();
        scene
            .draft_add_segment(d, 0.2, 0.2, 1.0, 0.8, 0.5, 1.0)
            .unwrap();
        scene.draft_end(d).unwrap();
        let ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let u = units(&scene, &ctx);
        let e = &u["units"][0]["edges"][0];
        assert_eq!(e["store"]["kind"], "owned");
        assert_eq!(e["store"]["object"].as_u64().unwrap(), b);
        assert!(e["store"]["sketch"].is_u64());
    }

    #[test]
    fn pick_plane_unit_hits_edge_then_region_then_nothing() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        rect(&mut scene, d, 0.0, 0.0, 2.0, 2.0, 0.0);
        scene.draft_end(d).unwrap();
        let u = units(&scene, GROUND);
        // Straight down onto the bottom edge (y = 0), 1 cm off it.
        let hit = json(
            &scene
                .pick_plane_unit(GROUND, &[1.0, 0.01, 5.0, 0.0, 0.0, -1.0], 0.05)
                .unwrap(),
        );
        assert_eq!(hit["unit"], u["units"][0]["key"]);
        assert!(hit["edge"].is_string() && hit["curve"].is_null());
        let inside = json(
            &scene
                .pick_plane_unit(GROUND, &[1.0, 1.0, 5.0, 0.0, 0.0, -1.0], 0.05)
                .unwrap(),
        );
        assert_eq!(inside["region"], u["regions"][0]["key"]);
        let away = scene
            .pick_plane_unit(GROUND, &[9.0, 9.0, 5.0, 0.0, 0.0, -1.0], 0.05)
            .unwrap();
        assert_eq!(away, "null");
        let parallel = scene
            .pick_plane_unit(GROUND, &[1.0, 1.0, 5.0, 1.0, 0.0, 0.0], 0.05)
            .unwrap();
        assert_eq!(parallel, "null");
        let behind = scene
            .pick_plane_unit(GROUND, &[1.0, 1.0, 5.0, 0.0, 0.0, 1.0], 0.05)
            .unwrap();
        assert_eq!(behind, "null");
    }

    #[test]
    fn pick_plane_unit_reports_circle_key() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "C")
            .unwrap();
        scene
            .draft_begin_curve_with(d, &[0.0, 0.0, 0.0], 1.0)
            .unwrap();
        let n = 16;
        for i in 0..n {
            let (a0, a1) = (
                std::f64::consts::TAU * f64::from(i) / f64::from(n),
                std::f64::consts::TAU * f64::from(i + 1) / f64::from(n),
            );
            scene
                .draft_add_segment(d, a0.cos(), a0.sin(), 0.0, a1.cos(), a1.sin(), 0.0)
                .unwrap();
        }
        scene.draft_end_curve(d).unwrap();
        scene.draft_end(d).unwrap();
        let hit = json(
            &scene
                .pick_plane_unit(GROUND, &[1.0, 0.0, 5.0, 0.0, 0.0, -1.0], 0.05)
                .unwrap(),
        );
        assert!(hit["curve"].is_string(), "{hit}");
        let u = units(&scene, GROUND);
        assert_eq!(u["units"][0]["edges"][0]["curve"]["radius"], 1.0);
    }

    #[test]
    fn transform_edges_moves_a_shape() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        rect(&mut scene, d, 0.0, 0.0, 1.0, 1.0, 0.0);
        scene.draft_end(d).unwrap();
        let keys = edge_keys(&units(&scene, GROUND));
        assert_eq!(keys.len(), 4);
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Move")
            .unwrap();
        let shift = [1.0, 0.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.draft_transform_edges(d, &keys, &shift).unwrap();
        // An unknown key refuses and leaves the draft usable.
        let err = scene.draft_remove_edges(d, &[1]).unwrap_err();
        assert!(err.0.starts_with("UnknownEdge"), "{}", err.0);
        scene.draft_end(d).unwrap();
        let u = units(&scene, GROUND);
        let xs: Vec<f64> = u["regions"][0]["outer"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[0].as_f64().unwrap())
            .collect();
        assert!(
            xs.iter()
                .all(|&x| (x - 5.0).abs() < 1e-9 || (x - 6.0).abs() < 1e-9),
            "{xs:?}"
        );
        // Copy keeps the original too; remove erases one shape.
        let keys = edge_keys(&u);
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Copy")
            .unwrap();
        let back = [1.0, 0.0, 0.0, -5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.draft_copy_edges(d, &keys, &back).unwrap();
        scene.draft_end(d).unwrap();
        assert_eq!(
            units(&scene, GROUND)["regions"].as_array().unwrap().len(),
            2
        );
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Erase")
            .unwrap();
        scene.draft_remove_edges(d, &keys).unwrap();
        scene.draft_end(d).unwrap();
        assert_eq!(
            units(&scene, GROUND)["regions"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn move_vertex_and_offset_region() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        rect(&mut scene, d, 0.0, 0.0, 2.0, 2.0, 0.0);
        scene.draft_end(d).unwrap();
        let region = u64::from_str_radix(
            units(&scene, GROUND)["regions"][0]["key"].as_str().unwrap(),
            16,
        )
        .unwrap();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Offset")
            .unwrap();
        scene.draft_offset_region(d, region, -0.5).unwrap();
        assert!(scene.draft_offset_region(d, 7, 1.0).is_err());
        scene
            .draft_move_vertex(d, 2.0, 2.0, 0.0, 3.0, 3.0, 0.0)
            .unwrap();
        assert!(
            scene
                .draft_move_vertex(d, 9.0, 9.0, 0.0, 3.0, 3.0, 0.0)
                .is_err()
        );
        scene.draft_end(d).unwrap();
        assert!(units(&scene, GROUND)["regions"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn cancel_leaves_the_document_byte_identical() {
        let mut scene = Scene::new();
        ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let before = scene.save();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 1.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        rect(&mut scene, d, 0.5, 0.5, 1.5, 1.5, 1.0);
        assert!(scene.draft_cancel(d).unwrap());
        assert_eq!(scene.save(), before);
        assert!(
            !scene.draft_cancel(d).unwrap(),
            "a second cancel is a stale handle"
        );
    }

    #[test]
    fn stale_and_wrong_handles_refuse() {
        let mut scene = Scene::new();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        let err = scene.draft_add_segment(d + 1, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert!(err.err().unwrap().0.starts_with("StaleDraft"));
        // Only one draft at a time.
        let two = scene.draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "R2");
        assert!(two.err().unwrap().0.starts_with("DraftAlreadyOpen"));
        scene.draft_end(d).unwrap();
        let e = scene.draft_end(d).unwrap_err();
        assert!(e.0.starts_with("StaleDraft"), "{}", e.0);
        assert!(
            scene
                .draft_add_segment(d, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0)
                .is_err()
        );
        let d2 = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        assert_ne!(d, d2);
        assert!(scene.draft_begin_face(1, 1, "x").is_err());
        assert!(scene.draft_begin_sketch(1, "x").is_err());
        assert!(scene.plane_units_json("{}").is_err());
    }

    #[test]
    fn hidden_object_does_not_take_part() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        scene.set_hidden(&[b], &[]);
        assert!(scene.draft_begin_face(b, f, "x").is_err());
        let d = scene
            .draft_begin_plane(0.0, 0.0, 1.0, 0.0, 0.0, 1.0, "R")
            .unwrap();
        rect(&mut scene, d, 0.5, 0.5, 1.5, 1.5, 1.0);
        let report = json(&scene.draft_end(d).unwrap());
        assert!(report["objects"].as_array().unwrap().is_empty(), "{report}");
    }

    #[test]
    fn instance_draft_maps_points_through_the_pose() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let inst = scene.make_component(&[0], &[b]).unwrap();
        let def = scene.instance_def(inst).unwrap();
        let shift = [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.transform_instance(inst, &shift).unwrap();
        let member = scene.component_member_objects(def)[0];
        let f = top_face(&scene, member);
        let d = scene
            .draft_begin_face_in_instance(inst, member, f, "Rectangle")
            .unwrap();
        rect(&mut scene, d, 10.5, 0.5, 11.5, 1.5, 1.0);
        let report = json(&scene.draft_end(d).unwrap());
        assert_eq!(report["objects"], serde_json::json!([member]));
        let ctx = format!(r#"{{"face":{{"object":{member},"face":{f}}},"instance":{inst}}}"#);
        let u = units(&scene, &ctx);
        let xs: Vec<f64> = u["units"][0]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|e| [e["a"][0].as_f64().unwrap(), e["b"][0].as_f64().unwrap()])
            .collect();
        assert!(xs.iter().all(|&x| x >= 10.4), "world x: {xs:?}");
        assert!(
            u["regions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["over"]["face"].is_u64())
        );
        // The same context without the instance reads definition-local space.
        let local = units(
            &scene,
            &format!(r#"{{"face":{{"object":{member},"face":{f}}}}}"#),
        );
        let lx = local["units"][0]["edges"][0]["a"][0].as_f64().unwrap();
        assert!(lx < 2.0, "local x: {lx}");
        // Picking through the pose.
        let hit = json(
            &scene
                .pick_plane_unit(&ctx, &[11.0, 0.5, 5.0, 0.0, 0.0, -1.0], 0.05)
                .unwrap(),
        );
        assert!(hit["edge"].is_string(), "{hit}");
        // A plane draft in the instance, from world coordinates.
        let d = scene
            .draft_begin_plane_in_instance(inst, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, "Line")
            .unwrap();
        scene
            .draft_add_segment(d, 10.2, 0.2, 1.0, 10.4, 0.3, 1.0)
            .unwrap();
        scene.draft_cancel(d).unwrap();
        // A stale instance refuses.
        assert!(
            scene
                .draft_begin_face_in_instance(inst + 1000, member, f, "x")
                .is_err()
        );
    }

    /// Draws a detached circle of `n` facets in the plane z = `c[2]` through
    /// `draft_begin_detached(ctx)` and returns the new sketch.
    fn detached_circle(scene: &mut Scene, ctx: &str, c: [f64; 3], r: f64, n: usize) -> u64 {
        let d = scene.draft_begin_detached(ctx, "Circle").unwrap();
        scene.draft_begin_curve_with(d, &c, r).unwrap();
        for i in 0..n {
            let (a0, a1) = (
                std::f64::consts::TAU * i as f64 / n as f64,
                std::f64::consts::TAU * (i + 1) as f64 / n as f64,
            );
            scene
                .draft_add_segment(
                    d,
                    c[0] + r * a0.cos(),
                    c[1] + r * a0.sin(),
                    c[2],
                    c[0] + r * a1.cos(),
                    c[1] + r * a1.sin(),
                    c[2],
                )
                .unwrap();
        }
        scene.draft_end_curve(d).unwrap();
        let report = json(&scene.draft_end(d).unwrap());
        assert!(report["objects"].as_array().unwrap().is_empty(), "{report}");
        let sketches = report["sketches"].as_array().unwrap();
        assert_eq!(sketches.len(), 1, "one new detached sketch: {report}");
        sketches[0].as_u64().unwrap()
    }

    fn sketch_ctx(s: u64) -> String {
        format!(r#"{{"sketch":{s}}}"#)
    }

    /// Detached shapes §4: a detached circle centered on a box's top corner
    /// leaves the face whole, lists only through its own sketch context
    /// (every edge a free store marked detached, its one region extruding
    /// as the sketch's own), lists as its own entry of the whole-document
    /// listing, and extrudes into a new Object.
    #[test]
    fn a_detached_circle_on_a_face_leaves_the_face_whole() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let faces = scene.doc.object(object_id(b)).unwrap().faces().len();
        let face_ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let s = detached_circle(&mut scene, &face_ctx, [2.0, 2.0, 1.0], 0.5, 48);
        assert!(scene.doc.sketch(sketch_id(s)).unwrap().detached());
        assert_eq!(
            scene.doc.object(object_id(b)).unwrap().faces().len(),
            faces,
            "the face is not split"
        );
        let top = units(&scene, &face_ctx);
        assert!(
            top["units"].as_array().unwrap().is_empty(),
            "the face's plane has no drawing: {top}"
        );
        assert_eq!(
            top["regions"].as_array().unwrap().len(),
            1,
            "the face alone"
        );

        let own = units(&scene, &sketch_ctx(s));
        assert_eq!(own["units"].as_array().unwrap().len(), 1);
        let edges = own["units"][0]["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 48, "a whole circle");
        assert!(
            edges.iter().all(
                |e| e["store"] == serde_json::json!({"kind":"free","sketch":s,"detached":true})
            )
        );
        let regions = own["regions"].as_array().unwrap();
        assert_eq!(regions.len(), 1);
        assert!(regions[0]["over"].is_null());
        assert_eq!(regions[0]["extrude"]["sketch"].as_u64().unwrap(), s);

        let all = json(&scene.plane_units_all_json().unwrap());
        let entry = all
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["sketch"].as_u64() == Some(s))
            .expect("listed as its own entry");
        assert_eq!(entry["detached"], true);
        assert!((entry["plane"][2].as_f64().unwrap() - 1.0).abs() < 1e-12);

        let region = regions[0]["extrude"]["region"].as_u64().unwrap();
        let cutter = scene.extrude_region(s, region, -0.5).unwrap();
        assert_ne!(cutter, b);
        assert_eq!(scene.object_ids().len(), 2, "a new Object");
        assert_eq!(
            scene.doc.object(object_id(b)).unwrap().faces().len(),
            faces,
            "the box is untouched"
        );
    }

    /// `draft_begin_detached` takes a plane or face context only; a plain
    /// sketch's listing never shows a detached sketch beside it.
    #[test]
    fn draft_begin_detached_takes_a_plane_or_face_context() {
        let mut scene = Scene::new();
        let s = detached_circle(&mut scene, GROUND, [0.0, 0.0, 0.0], 1.0, 24);
        for ctx in [
            sketch_ctx(s),
            r#"{"plane":[0,0,0,0,0,1],"object":1}"#.to_string(),
            "{}".to_string(),
        ] {
            let err = scene.draft_begin_detached(&ctx, "Circle").unwrap_err();
            assert!(err.0.starts_with("BadContext"), "{ctx}: {}", err.0);
        }
        assert!(
            scene
                .draft_begin_detached(&sketch_ctx(s), "x")
                .unwrap_err()
                .0
                .contains("draft_begin_ctx"),
            "the refusal names the way to continue it"
        );
        // A plain rectangle over the detached circle ignores it.
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Rectangle")
            .unwrap();
        rect(&mut scene, d, -0.5, -0.5, 2.0, 0.5, 0.0);
        scene.draft_end(d).unwrap();
        let ground = units(&scene, GROUND);
        assert_eq!(ground["regions"].as_array().unwrap().len(), 1, "{ground}");
        assert_eq!(edge_keys(&ground).len(), 4, "the rectangle alone");
        assert_eq!(
            edge_keys(&units(&scene, &sketch_ctx(s))).len(),
            24,
            "the circle is whole"
        );
    }

    /// `sketch_is_detached` answers for any sketch handle: true for a
    /// detached shape, false for an ordinary free sketch and for an unknown
    /// handle.
    #[test]
    fn sketch_is_detached_reports_the_mode_of_any_sketch() {
        let mut scene = Scene::new();
        let s = detached_circle(
            &mut scene,
            r#"{"plane":[0,0,0,0,0,1]}"#,
            [0.0, 0.0, 0.0],
            0.5,
            12,
        );
        assert!(scene.sketch_is_detached(s));
        let d = scene
            .draft_begin_plane(0.0, 0.0, 5.0, 0.0, 0.0, 1.0, "Line")
            .unwrap();
        scene
            .draft_add_segment(d, 0.0, 0.0, 5.0, 1.0, 0.0, 5.0)
            .unwrap();
        let plain = json(&scene.draft_end(d).unwrap())["sketches"][0]
            .as_u64()
            .unwrap();
        assert!(!scene.sketch_is_detached(plain));
        assert!(!scene.sketch_is_detached(u64::MAX));
    }

    /// `sketch_owner_object` names the owning Object of an object-owned
    /// sketch and is `None` for a free (detached) sketch and for an unknown
    /// handle.
    #[test]
    fn sketch_owner_object_reports_ownership_of_any_sketch() {
        let mut scene = Scene::new();
        let free = detached_circle(
            &mut scene,
            r#"{"plane":[0,0,0,0,0,1]}"#,
            [5.0, 5.0, 0.0],
            0.5,
            12,
        );
        assert_eq!(scene.sketch_owner_object(free), None);
        assert_eq!(scene.sketch_owner_object(u64::MAX), None);
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Line").unwrap();
        scene
            .draft_add_segment(d, 0.2, 0.2, 1.0, 0.8, 0.5, 1.0)
            .unwrap();
        scene.draft_end(d).unwrap();
        let ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let u = units(&scene, &ctx);
        let owned = u["units"][0]["edges"][0]["store"]["sketch"]
            .as_u64()
            .unwrap();
        assert_eq!(scene.sketch_owner_object(owned), Some(b));
    }

    /// A `{"sketch":id}` context on a detached sketch is the sketch alone:
    /// a draft opened on it (by context or by handle) arranges with its own
    /// drawing and lowers back into it, never into the face under it.
    #[test]
    fn a_detached_sketchs_context_edits_it_alone() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let faces = scene.doc.object(object_id(b)).unwrap().faces().len();
        let face_ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let d = scene.draft_begin_detached(&face_ctx, "Rectangle").unwrap();
        rect(&mut scene, d, 0.5, 0.5, 1.5, 1.5, 1.0);
        let s = json(&scene.draft_end(d).unwrap())["sketches"][0]
            .as_u64()
            .unwrap();
        // A line across it, continuing the sketch: two regions in it.
        let d = scene.draft_begin_ctx(&sketch_ctx(s), "Line").unwrap();
        scene
            .draft_add_segment(d, 1.0, 0.5, 1.0, 1.0, 1.5, 1.0)
            .unwrap();
        let report = json(&scene.draft_end(d).unwrap());
        assert_eq!(report["sketches"], serde_json::json!([s]), "{report}");
        assert!(report["objects"].as_array().unwrap().is_empty());
        let own = units(&scene, &sketch_ctx(s));
        assert_eq!(own["regions"].as_array().unwrap().len(), 2);
        // Move it by handle: still one detached sketch, face still whole.
        let keys = edge_keys(&own);
        let d = scene.draft_begin_sketch(s, "Move").unwrap();
        let shift = [1.0, 0.0, 0.0, 0.25, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.draft_transform_edges(d, &keys, &shift).unwrap();
        scene.draft_end(d).unwrap();
        assert!(scene.doc.sketch(sketch_id(s)).unwrap().detached());
        assert_eq!(scene.doc.object(object_id(b)).unwrap().faces().len(), faces);
        assert!(
            units(&scene, &face_ctx)["units"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // Offset preview and offset in its own context.
        let own = units(&scene, &sketch_ctx(s));
        let key = u64::from_str_radix(own["regions"][0]["key"].as_str().unwrap(), 16).unwrap();
        let data = scene
            .offset_region_preview(&sketch_ctx(s), key, -0.1)
            .unwrap();
        assert_eq!(data[0], 1.0, "one loop");
        assert!(
            scene.offset_region_preview(&face_ctx, key, -0.1).is_err(),
            "the face's plane does not hold the region"
        );
        let d = scene.draft_begin_ctx(&sketch_ctx(s), "Offset").unwrap();
        scene.draft_offset_region(d, key, -0.1).unwrap();
        scene.draft_end(d).unwrap();
        assert_eq!(
            units(&scene, &sketch_ctx(s))["regions"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(scene.doc.object(object_id(b)).unwrap().faces().len(), faces);
    }

    /// Detached shapes D16: a pick on a face context also finds the detached
    /// sketches on its plane and says so with `ctx`; a pick on the detached
    /// sketch's own context is confined to it; through an instance the
    /// returned context carries the instance.
    #[test]
    fn pick_plane_unit_probes_detached_sketches() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let face_ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let s = detached_circle(&mut scene, &face_ctx, [1.0, 1.0, 1.0], 0.5, 24);
        let own = units(&scene, &sketch_ctx(s));
        let down = |x: f64, y: f64| [x, y, 5.0, 0.0, 0.0, -1.0];

        // Inside the disk: the detached region wins the tie with the face.
        let hit = json(
            &scene
                .pick_plane_unit(&face_ctx, &down(1.0, 1.0), 0.01)
                .unwrap(),
        );
        assert_eq!(hit["region"], own["regions"][0]["key"], "{hit}");
        assert_eq!(hit["ctx"], serde_json::json!({ "sketch": s }));
        // On the rim: its edge, with its context.
        let hit = json(
            &scene
                .pick_plane_unit(&face_ctx, &down(1.5, 1.0), 0.01)
                .unwrap(),
        );
        assert!(hit["edge"].is_string() && hit["run"].is_string(), "{hit}");
        assert_eq!(hit["unit"], own["units"][0]["key"]);
        assert_eq!(hit["ctx"], serde_json::json!({ "sketch": s }));
        // Away from it: the face's own region, no ctx.
        let hit = json(
            &scene
                .pick_plane_unit(&face_ctx, &down(0.1, 0.1), 0.01)
                .unwrap(),
        );
        assert!(
            hit["region"].is_string() && hit.get("ctx").is_none(),
            "{hit}"
        );
        // The same on the plane form of the context.
        let plane_ctx = r#"{"plane":[0,0,1,0,0,1]}"#;
        let hit = json(
            &scene
                .pick_plane_unit(plane_ctx, &down(1.0, 1.0), 0.01)
                .unwrap(),
        );
        assert_eq!(hit["ctx"], serde_json::json!({ "sketch": s }));
        // The sketch's own context: confined, and no ctx (it is the request's).
        let hit = json(
            &scene
                .pick_plane_unit(&sketch_ctx(s), &down(1.0, 1.0), 0.01)
                .unwrap(),
        );
        assert_eq!(hit["region"], own["regions"][0]["key"]);
        assert!(hit.get("ctx").is_none(), "{hit}");
        assert_eq!(
            scene
                .pick_plane_unit(&sketch_ctx(s), &down(0.1, 0.1), 0.01)
                .unwrap(),
            "null",
            "the face is not in a detached sketch's context"
        );
        // A smaller detached shape inside it is the one picked inside both.
        let small = detached_circle(&mut scene, &face_ctx, [1.0, 1.0, 1.0], 0.2, 24);
        let hit = json(
            &scene
                .pick_plane_unit(&face_ctx, &down(1.0, 1.0), 0.01)
                .unwrap(),
        );
        assert_eq!(hit["ctx"], serde_json::json!({ "sketch": small }), "{hit}");
        let hit = json(
            &scene
                .pick_plane_unit(&face_ctx, &down(1.0, 1.35), 0.01)
                .unwrap(),
        );
        assert_eq!(hit["ctx"], serde_json::json!({ "sketch": s }), "{hit}");
        // A hidden one (drawn into a group the user hid) takes no click.
        let other = ground_box(&mut scene, 5.0, 5.0, 6.0, 6.0, 1.0);
        let g = scene.group_nodes(&[0], &[other]).unwrap();
        scene
            .doc
            .set_sketch_group(sketch_id(small), Some(crate::group_id(g)))
            .unwrap();
        scene.set_node_user_hidden(1, g, true).unwrap();
        let hit = json(
            &scene
                .pick_plane_unit(&face_ctx, &down(1.0, 1.0), 0.01)
                .unwrap(),
        );
        assert_eq!(hit["ctx"], serde_json::json!({ "sketch": s }), "{hit}");

        // Through an instance: the detached sketch is the definition's, and
        // the pick's context names the instance too.
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let inst = scene.make_component(&[0], &[b]).unwrap();
        let def = scene.instance_def(inst).unwrap();
        let member = scene.component_member_objects(def)[0];
        let f = top_face(&scene, member);
        let shift = [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.transform_instance(inst, &shift).unwrap();
        let ctx = format!(r#"{{"face":{{"object":{member},"face":{f}}},"instance":{inst}}}"#);
        let s = detached_circle(&mut scene, &ctx, [11.0, 1.0, 1.0], 0.5, 24);
        assert!(scene.doc.sketch(sketch_id(s)).unwrap().detached());
        let hit = json(&scene.pick_plane_unit(&ctx, &down(11.0, 1.0), 0.01).unwrap());
        assert_eq!(
            hit["ctx"],
            serde_json::json!({ "sketch": s, "instance": inst }),
            "{hit}"
        );
        // Its listing through that context is in world space.
        let own = units(&scene, &format!(r#"{{"sketch":{s},"instance":{inst}}}"#));
        let x = own["units"][0]["edges"][0]["a"][0].as_f64().unwrap();
        assert!(x > 10.0, "world x: {x}");
    }

    /// `SketchRegionPickJs.detached` says whether the picked region's sketch
    /// is detached; the island copy export copies a detached unit, in plane
    /// too, as a new detached sketch.
    #[test]
    fn region_pick_reports_detached_and_copy_makes_a_new_detached_sketch() {
        let mut scene = Scene::new();
        let s = detached_circle(&mut scene, GROUND, [0.0, 0.0, 0.0], 1.0, 24);
        let d = scene
            .draft_begin_plane(0.0, 0.0, 0.0, 0.0, 0.0, 1.0, "Rectangle")
            .unwrap();
        rect(&mut scene, d, 5.0, 0.0, 6.0, 1.0, 0.0);
        scene.draft_end(d).unwrap();
        let pick = scene
            .pick_sketch_region(0.0, 0.0, 5.0, 0.0, 0.0, -1.0)
            .unwrap();
        assert_eq!(pick.sketch(), s);
        assert!(pick.detached());
        let plain = scene
            .pick_sketch_region(5.5, 0.5, 5.0, 0.0, 0.0, -1.0)
            .unwrap();
        assert!(!plain.detached());

        let islands = scene.sketch_island_ids(s);
        let shift = [1.0, 0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let before = scene.doc.sketch(sketch_id(s)).unwrap().clone();
        let copy = scene.copy_sketch_islands(s, &islands, &shift).unwrap();
        assert_ne!(copy, s);
        assert!(scene.doc.sketch(sketch_id(copy)).unwrap().detached());
        assert_eq!(
            scene.doc.sketch(sketch_id(s)).unwrap(),
            &before,
            "the overlapping original is untouched"
        );
        assert_eq!(edge_keys(&units(&scene, &sketch_ctx(copy))).len(), 24);
    }

    /// A recorded detached draw replays to the same document.
    #[test]
    fn detached_drafting_recording_round_trips() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let face_ctx = format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#);
        let s = detached_circle(&mut scene, &face_ctx, [2.0, 2.0, 1.0], 0.5, 24);
        let d = scene.draft_begin_ctx(&sketch_ctx(s), "Line").unwrap();
        scene
            .draft_add_segment(d, 1.5, 2.0, 1.0, 2.5, 2.0, 1.0)
            .unwrap();
        scene.draft_end(d).unwrap();
        let own = units(&scene, &sketch_ctx(s));
        let ex = &own["regions"][0]["extrude"];
        scene
            .extrude_region(
                ex["sketch"].as_u64().unwrap(),
                ex["region"].as_u64().unwrap(),
                -0.5,
            )
            .unwrap();
        let hash = scene.doc.state_hash();
        let rec = scene.take_recording();
        assert!(rec.contains("\"method\":\"draft_begin_detached\""), "{rec}");
        let mut fresh = Scene::new();
        assert_eq!(fresh.replay(&rec).unwrap(), hash);
    }

    /// A plane draft opened through a MIRRORED instance on the world plane
    /// of a member's top face lands on that face: the context plane maps
    /// into the definition keeping its side, so it matches the face's
    /// outward normal and the face takes part in the draft.
    #[test]
    fn a_plane_draft_through_a_mirrored_instance_lands_on_the_member_face() {
        let mut scene = Scene::new();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let inst = scene.make_component(&[0], &[b]).unwrap();
        let def = scene.instance_def(inst).unwrap();
        // Mirror x, then shift: the box covers x in [3, 5].
        let mirror = [-1.0, 0.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.transform_instance(inst, &mirror).unwrap();
        let member = scene.component_member_objects(def)[0];
        let d = scene
            .draft_begin_plane_in_instance(inst, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, "Rectangle")
            .unwrap();
        rect(&mut scene, d, 3.5, 0.5, 4.5, 1.5, 1.0);
        let report = json(&scene.draft_end(d).unwrap());
        assert_eq!(
            report["objects"],
            serde_json::json!([member]),
            "the member's top face took the drawing: {report}"
        );
    }

    #[test]
    fn drafting_recording_round_trips() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let b = ground_box(&mut scene, 0.0, 0.0, 2.0, 2.0, 1.0);
        let f = top_face(&scene, b);
        let d = scene.draft_begin_face(b, f, "Circle").unwrap();
        scene
            .draft_begin_polygon_with(d, &[1.0, 1.0, 1.0], 0.5)
            .unwrap();
        for i in 0..6u32 {
            let (a0, a1) = (
                std::f64::consts::TAU * f64::from(i) / 6.0,
                std::f64::consts::TAU * f64::from(i + 1) / 6.0,
            );
            scene
                .draft_add_segment(
                    d,
                    1.0 + 0.5 * a0.cos(),
                    1.0 + 0.5 * a0.sin(),
                    1.0,
                    1.0 + 0.5 * a1.cos(),
                    1.0 + 0.5 * a1.sin(),
                    1.0,
                )
                .unwrap();
        }
        scene.draft_end_curve(d).unwrap();
        scene.draft_end(d).unwrap();
        let keys = edge_keys(&units(
            &scene,
            &format!(r#"{{"face":{{"object":{b},"face":{f}}}}}"#),
        ));
        let d = scene.draft_begin_face(b, f, "Move").unwrap();
        let shift = [1.0, 0.0, 0.0, 0.25, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.draft_transform_edges(d, &keys, &shift).unwrap();
        scene.draft_end(d).unwrap();
        let d = scene
            .draft_begin_plane(0.0, 0.0, 5.0, 0.0, 0.0, 1.0, "Abandoned")
            .unwrap();
        scene
            .draft_add_segment(d, 0.0, 0.0, 5.0, 1.0, 0.0, 5.0)
            .unwrap();
        scene.draft_cancel(d).unwrap();
        let hash = scene.doc.state_hash();
        let rec = scene.take_recording();
        let mut fresh = Scene::new();
        assert_eq!(fresh.replay(&rec).unwrap(), hash);
    }
}
