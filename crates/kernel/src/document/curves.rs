//! Document entry points of the true-curves operations: blends, revolve,
//! deform, loft, shell, project, path arrays, Smooth and the import
//! repairs. A second `impl Document` block, kept out of `document.rs` for
//! size, the way `draft.rs` holds the drafting engine.
//!
//! Every in-place operation goes through [`Document::reshape`]: it runs a
//! [`ReshapeSpec`] on the live object and records the snapshot undo
//! `DocAction::Reshaped` (decision I-6).

use super::*;
use crate::blend::{BlendError, BlendLimit, BlendProfile, BlendReport, BlendSpec};
use crate::geom2d;
use crate::shape_error::ShapeError;
use crate::sketch::Profile;
use crate::sketch::SketchVertexId;
use crate::sketch_corner::{CornerBlendReport, CornerProfile};

/// Exactly the inputs of one in-place reshape, as values valid against the
/// `before` snapshot (ids inside it, positions, sizes) — never a reference
/// to other document state, so redo can re-run it on the snapshot alone.
#[derive(Debug, Clone, PartialEq)]
pub enum ReshapeSpec {
    /// A uniform scale about the origin. Test support only: exercises the
    /// reshape machinery before the real operations exist.
    #[doc(hidden)]
    Test {
        /// The scale factor.
        scale: f64,
    },
    /// Fillet / chamfer the spec's edges ([`Object::blend_edges`]).
    Blend(BlendSpec),
    /// Remove a blend, restoring its sharp edges.
    RemoveBlend(crate::topo::BlendId),
    /// Re-blend a blend's restored edges with a new profile.
    ResizeBlend(crate::topo::BlendId, BlendProfile),
    /// Push/Pull on a blend face: resize the blend.
    PushPullBlend(FaceId, f64),
    /// Push/Pull of a face beside blends, re-applying them.
    PushPullWithBlends(FaceId, f64),
    /// Hollow the object ([`Object::shelled`]).
    Shell(crate::shell::ShellSpec),
    /// Twist, bend, taper or bend to a path ([`Object::deformed`]).
    Deform(crate::deform::DeformSpec),
    /// Imprint a projected shape ([`Object::project_imprint`]).
    ProjectImprint(crate::project::ImprintSpec),
    /// Engrave or emboss an imprinted region (straight or follow).
    ProjectDepth(crate::project::DepthSpec),
    /// Set or clear the crease flag on edges (Smooth keeps creases sharp).
    SetCrease {
        /// The kernel edges, already expanded to their logical edges.
        edges: Vec<crate::ids::EdgeId>,
        /// The flag to set.
        crease: bool,
    },
    /// Replace the cage with its subdivided solid ([`Object::baked`]).
    Bake {
        /// The Smooth level baked.
        level: u8,
    },
    /// Soften edges bending less than `max_angle` radians
    /// ([`Object::soften_by_angle`]).
    SoftenByAngle {
        /// The largest bend softened, radians.
        max_angle: f64,
    },
}

/// What a reshape produced, beyond the object itself: op-specific results
/// handed back to the caller.
#[derive(Debug, Clone, PartialEq)]
pub enum ReshapeReport {
    /// Nothing beyond the new geometry.
    None,
    /// The blend's ids, new faces and applied sizes.
    Blend(BlendReport),
    /// The faces of an imprinted region.
    Faces(Vec<FaceId>),
    /// How many elements changed (Soften's edges).
    Count(usize),
}

/// The output of [`ReshapeSpec::run`].
#[derive(Debug, Clone)]
pub(crate) struct ReshapeOutput {
    /// The object's new geometry.
    pub after: Object,
    /// Pieces split off into new objects beside it (none yet).
    pub pieces: Vec<Object>,
    /// Op-specific results.
    pub report: ReshapeReport,
}

impl ReshapeSpec {
    /// Pure: runs the reshape on `obj`. The same function serves the forward
    /// operation and redo (which discards `report`), so redo reproduces the
    /// original result bit for bit.
    pub(crate) fn run(&self, obj: &Object) -> Result<ReshapeOutput, DocumentError> {
        match self {
            ReshapeSpec::Test { scale } => {
                let mut after = obj.clone();
                after
                    .apply_transform(&Transform::uniform_scale(*scale))
                    .map_err(DocumentError::Transform)?;
                after
                    .validate()
                    .map_err(|_| DocumentError::Shape(ShapeError::ShapeDegenerate))?;
                Ok(ReshapeOutput {
                    after,
                    pieces: Vec::new(),
                    report: ReshapeReport::None,
                })
            }
            ReshapeSpec::Deform(spec) => Ok(ReshapeOutput {
                after: obj.deformed(spec).map_err(DocumentError::Shape)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
            ReshapeSpec::ProjectImprint(spec) => {
                let (after, faces) = obj.project_imprint(spec).map_err(DocumentError::Shape)?;
                Ok(ReshapeOutput {
                    after,
                    pieces: Vec::new(),
                    report: ReshapeReport::Faces(faces),
                })
            }
            ReshapeSpec::ProjectDepth(spec) => Ok(ReshapeOutput {
                after: project_depth(obj, spec)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
            ReshapeSpec::SetCrease { edges, crease } => Ok(ReshapeOutput {
                after: with_creases(obj, edges, *crease)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
            ReshapeSpec::SoftenByAngle { max_angle } => {
                let (after, n) = obj
                    .soften_by_angle(*max_angle)
                    .map_err(DocumentError::Shape)?;
                Ok(ReshapeOutput {
                    after,
                    pieces: Vec::new(),
                    report: ReshapeReport::Count(n),
                })
            }
            ReshapeSpec::Bake { level } => Ok(ReshapeOutput {
                after: obj.baked(*level).map_err(DocumentError::Shape)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
            ReshapeSpec::Shell(spec) => Ok(ReshapeOutput {
                after: obj.shelled(spec).map_err(DocumentError::Shape)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
            ReshapeSpec::Blend(spec) => {
                let (after, report) = obj.blend_edges(spec).map_err(DocumentError::Blend)?;
                Ok(ReshapeOutput {
                    after,
                    pieces: Vec::new(),
                    report: ReshapeReport::Blend(report),
                })
            }
            ReshapeSpec::RemoveBlend(id) => {
                let (after, _) = obj.remove_blend(*id).map_err(DocumentError::Blend)?;
                Ok(ReshapeOutput {
                    after,
                    pieces: Vec::new(),
                    report: ReshapeReport::None,
                })
            }
            ReshapeSpec::ResizeBlend(id, profile) => {
                let (after, report) = obj
                    .resize_blend(*id, profile)
                    .map_err(DocumentError::Blend)?;
                Ok(ReshapeOutput {
                    after,
                    pieces: Vec::new(),
                    report: ReshapeReport::Blend(report),
                })
            }
            ReshapeSpec::PushPullBlend(face, distance) => Ok(ReshapeOutput {
                after: obj
                    .push_pull_blend(*face, *distance)
                    .map_err(DocumentError::Blend)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
            ReshapeSpec::PushPullWithBlends(face, distance) => Ok(ReshapeOutput {
                after: obj
                    .push_pull_with_blends(*face, *distance)
                    .map_err(DocumentError::Blend)?,
                pieces: Vec::new(),
                report: ReshapeReport::None,
            }),
        }
    }

    /// The history label (Undo "Fillet", "Deform", …).
    pub(crate) fn label(&self) -> &'static str {
        match self {
            ReshapeSpec::Test { .. } => "Reshape",
            ReshapeSpec::Shell(_) => "Shell",
            ReshapeSpec::SetCrease { crease: true, .. } => "Crease",
            ReshapeSpec::SetCrease { crease: false, .. } => "Uncrease",
            ReshapeSpec::Bake { .. } => "Bake Smooth",
            ReshapeSpec::SoftenByAngle { .. } => "Soften Edges",
            ReshapeSpec::Deform(_) => "Deform",
            ReshapeSpec::ProjectImprint(_) | ReshapeSpec::ProjectDepth(_) => "Project",
            ReshapeSpec::Blend(spec) => match spec.profile {
                BlendProfile::Chamfer { .. } => "Chamfer",
                _ => "Fillet",
            },
            ReshapeSpec::RemoveBlend(_) => "Remove Fillet",
            ReshapeSpec::ResizeBlend(..) => "Resize Fillet",
            ReshapeSpec::PushPullBlend(..) | ReshapeSpec::PushPullWithBlends(..) => "Push/Pull",
        }
    }
}

/// What [`Document::blend_preview`] shows: the object a commit would
/// produce, clamped to the largest size that fits.
#[derive(Debug, Clone)]
pub struct BlendPreview {
    /// The object the commit would produce, at the `applied` size.
    pub after: Object,
    /// The new blend faces, in `after`.
    pub new_faces: Vec<FaceId>,
    /// Faces of the current object the result replaces (to mask in a
    /// preview).
    pub replaced: Vec<FaceId>,
    /// The profile actually applied: the spec's unless clamped.
    pub applied: BlendProfile,
    /// The limit, when the spec's size did not fit.
    pub clamped: Option<BlendLimit>,
}

impl Document {
    /// The blends on `object` (WP08 §1), each verified.
    ///
    /// # Errors
    /// The reshape gate's errors.
    pub fn blend_features(
        &self,
        object: ObjectId,
    ) -> Result<Vec<crate::blend::BlendFeature>, DocumentError> {
        self.reshape_target(object)?;
        Ok(self.objects[object].object.blend_features())
    }

    /// Removes blend `id` from `object`, restoring its sharp edges, as one
    /// undo step.
    ///
    /// # Errors
    /// The gate's errors; `Blend(UnknownBlend | BlendModified)`.
    pub fn remove_blend(
        &mut self,
        object: ObjectId,
        id: crate::topo::BlendId,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(object, ReshapeSpec::RemoveBlend(id))
            .map(|(_, c)| c)
    }

    /// Re-blends blend `id`'s restored edges with `profile` (a new size, or
    /// a switch between round and chamfer), as one undo step: as a fresh
    /// fillet of those edges at the new size, so a corner with another
    /// fillet is solved again ([`crate::Object::blend_edges`]).
    ///
    /// # Errors
    /// The gate's errors; `Blend(..)` from restoration or the re-blend.
    pub fn resize_blend(
        &mut self,
        object: ObjectId,
        id: crate::topo::BlendId,
        profile: &BlendProfile,
    ) -> Result<(BlendReport, DocChange), DocumentError> {
        let (report, change) =
            self.reshape(object, ReshapeSpec::ResizeBlend(id, profile.clone()))?;
        match report {
            ReshapeReport::Blend(r) => Ok((r, change)),
            _ => unreachable!("a resize reports a blend"),
        }
    }

    /// Pure: what [`Document::resize_blend`] would produce, clamped like
    /// [`Document::blend_preview`].
    ///
    /// # Errors
    /// As `resize_blend`, except size failures, which clamp.
    pub fn resize_blend_preview(
        &self,
        object: ObjectId,
        id: crate::topo::BlendId,
        profile: &BlendProfile,
    ) -> Result<BlendPreview, DocumentError> {
        self.reshape_target(object)?;
        let obj = &self.objects[object].object;
        let restored = obj.restore_blend(id).map_err(DocumentError::Blend)?;
        let spec = restored.resized_spec(profile);
        let at = &restored.object;
        // The commit's own re-blend (the resized blend under its id, the
        // later blends put back under theirs), so the preview's blend ids
        // and new faces are the commit's.
        let preview = |s: &BlendSpec, clamped: Option<BlendLimit>| {
            restored.reblend(id, s).map(|(after, report)| BlendPreview {
                after,
                new_faces: report.new_faces,
                replaced: obj
                    .faces()
                    .iter()
                    .filter(|(_, f)| f.blend.is_some_and(|t| t.id == id))
                    .map(|(fid, _)| fid)
                    .collect(),
                applied: s.profile.clone(),
                clamped,
            })
        };
        crate::blend::as_preview(|| match preview(&spec, None) {
            Ok(p) => Ok(p),
            Err(e @ (BlendError::RadiusTooLarge { .. } | BlendError::Obstructed { .. })) => {
                // The refusal's own limit (the search it ran, a second
                // would cost as much again); an Obstructed names none, so
                // search, except on an import (minutes on a scan).
                let limit = match e {
                    BlendError::RadiusTooLarge { max, limited_by } => {
                        BlendLimit { max, limited_by }
                    }
                    _ if at.planarity_tol > crate::tol::PLANE_DIST => {
                        return Err(DocumentError::Blend(e));
                    }
                    _ => at.max_blend_size(&spec).map_err(DocumentError::Blend)?,
                };
                // The spec the commit makes at that size (handles scaled).
                let size = spec.profile.size().unwrap_or(1.0);
                let s = restored.resized_spec(&profile.scaled(limit.max / size));
                preview(&s, Some(limit)).map_err(DocumentError::Blend)
            }
            Err(e) => Err(DocumentError::Blend(e)),
        })
    }

    /// Push/Pull on a blend face (WP08 §5.1): resizes the blend; pulled to
    /// or past the sharp edge, removes it. One undo step.
    ///
    /// # Errors
    /// The gate's errors; `Blend(..)`.
    pub fn push_pull_blend(
        &mut self,
        object: ObjectId,
        face: FaceId,
        distance: f64,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(object, ReshapeSpec::PushPullBlend(face, distance))
            .map(|(_, c)| c)
    }

    /// Blend-aware Push/Pull of a face beside blends (WP08 §5.2): the
    /// blends are restored, the face pushed, the blends re-applied, as one
    /// step. On `Err` callers fall back to the ordinary Push/Pull.
    ///
    /// # Errors
    /// The gate's errors; `Blend(UnknownBlend)` when no blend is beside the
    /// face; any restoration, Push/Pull or re-blend failure.
    pub fn push_pull_with_blends(
        &mut self,
        object: ObjectId,
        face: FaceId,
        distance: f64,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(object, ReshapeSpec::PushPullWithBlends(face, distance))
            .map(|(_, c)| c)
    }

    /// Paints every face of `face`'s logical face (WP08 §6) as one undo
    /// step.
    ///
    /// # Errors
    /// `UnknownObject`, `UnknownFace`, `UnknownMaterial`.
    pub fn paint_logical_face(
        &mut self,
        object: ObjectId,
        face: FaceId,
        material: Option<MaterialId>,
    ) -> Result<DocChange, DocumentError> {
        if let Some(id) = material
            && !self.material_is_live(id)
        {
            return Err(DocumentError::UnknownMaterial);
        }
        self.refuse_frozen_object(object)?;
        let rec = match self.objects.get_mut(object) {
            Some(rec) if !rec.hidden => rec,
            _ => return Err(DocumentError::UnknownObject),
        };
        if !rec.object.faces.contains_key(face) {
            return Err(DocumentError::UnknownFace);
        }
        let faces = rec.object.logical_face(face);
        let mut actions = Vec::with_capacity(faces.len());
        for f in faces {
            let face_ref = super::FaceRef::of(&rec.object, f);
            let slot = rec
                .object
                .faces
                .get_mut(f)
                .expect("a logical face's face is live");
            let prev = slot.material;
            slot.material = material;
            actions.push(DocAction::PaintFace {
                object,
                face: face_ref,
                prev,
                next: material,
            });
        }
        self.undo.push(DocAction::Compound {
            actions,
            meta: None,
        });
        self.commit_new_action();
        self.debug_validate();
        Ok(self.paint_change(object))
    }
}

/// `spec` with every length divided by `s` (an instance's uniform scale):
/// radii, setbacks, widths, stop distances and handle radii.
fn spec_in_definition_units(spec: &BlendSpec, s: f64) -> BlendSpec {
    let mut out = spec.clone();
    out.profile = spec.profile.scaled(1.0 / s);
    if let Some(stops) = &mut out.stops {
        stops.start /= s;
        stops.end /= s;
    }
    for h in &mut out.handles {
        h.at /= s;
        h.radius /= s;
    }
    out
}

impl Document {
    /// Fillets or chamfers `spec.edges` of `object` in place, as one undo
    /// step ("Fillet" / "Chamfer").
    ///
    /// # Errors
    /// `UnknownObject` / `UnknownComponent` from the gate, or
    /// [`DocumentError::Blend`] with the engine's refusal; on `Err` nothing
    /// changed.
    pub fn blend_edges(
        &mut self,
        object: ObjectId,
        spec: &BlendSpec,
    ) -> Result<(BlendReport, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "blend_edges", edges = spec.edges.len());
        let (report, change) = self.reshape(object, ReshapeSpec::Blend(spec.clone()))?;
        match report {
            ReshapeReport::Blend(r) => Ok((r, change)),
            _ => unreachable!("a blend reshape reports a blend"),
        }
    }

    /// Pure: what [`Document::blend_edges`] would produce. A size that does
    /// not fit (or that reaches other geometry) is clamped to the largest
    /// that does and reported in `clamped`.
    ///
    /// # Errors
    /// The gate's errors, and every hard refusal of the engine as
    /// [`DocumentError::Blend`].
    pub fn blend_preview(
        &self,
        object: ObjectId,
        spec: &BlendSpec,
    ) -> Result<BlendPreview, DocumentError> {
        crate::blend::as_preview(|| self.blend_preview_now(object, spec))
    }

    /// [`Document::blend_preview`] for a size the user typed (D-119): the
    /// swept blend takes the commit's rounds, so what previews is what the
    /// commit makes (a pointer preview takes one round, as it is asked
    /// again on every move).
    ///
    /// # Errors
    /// As [`Document::blend_preview`].
    pub fn blend_preview_typed(
        &self,
        object: ObjectId,
        spec: &BlendSpec,
    ) -> Result<BlendPreview, DocumentError> {
        crate::blend::as_typed_preview(|| self.blend_preview_now(object, spec))
    }

    /// The body of [`Document::blend_preview`], under the preview mode the
    /// caller set.
    fn blend_preview_now(
        &self,
        object: ObjectId,
        spec: &BlendSpec,
    ) -> Result<BlendPreview, DocumentError> {
        self.reshape_target(object)?;
        let obj = &self.objects[object].object;
        let preview = |s: &BlendSpec, clamped: Option<BlendLimit>| {
            obj.blend_preview_resolved(s)
                .map(|(after, report, replaced)| BlendPreview {
                    after,
                    new_faces: report.new_faces,
                    replaced,
                    applied: s.profile.clone(),
                    clamped,
                })
        };
        match preview(spec, None) {
            Ok(p) => Ok(p),
            Err(e @ (BlendError::RadiusTooLarge { .. } | BlendError::Obstructed { .. })) => {
                // The refusal's own limit: its search is the one a clamp
                // needs (on an imported mesh, the swept blend's, which the
                // facet-walk search cannot find), so it is not run twice.
                let limit = match e {
                    BlendError::RadiusTooLarge { max, limited_by } => {
                        BlendLimit { max, limited_by }
                    }
                    // An Obstructed preview on an import names no size, and
                    // the search would take minutes on a scan: the refusal.
                    _ if obj.planarity_tol > crate::tol::PLANE_DIST => {
                        return Err(DocumentError::Blend(e));
                    }
                    _ => obj.max_blend_size(spec).map_err(DocumentError::Blend)?,
                };
                // As the limit search tried it: handles scale with the size.
                preview(&spec.at_size(limit.max), Some(limit)).map_err(DocumentError::Blend)
            }
            Err(e) => Err(DocumentError::Blend(e)),
        }
    }

    /// [`Document::blend_edges`] on `object`, a member of `instance`'s
    /// definition, with the spec's lengths given in world units: they are
    /// divided by the instance's uniform scale. The edges are already
    /// definition-member ids.
    ///
    /// # Errors
    /// `UnknownInstance`; `UnknownObject` when `object` is not a member of
    /// the instance's definition; `Blend(NonUniformInstance)` for a pose
    /// that is not a similarity; then as [`Document::blend_edges`].
    pub fn blend_edges_in_instance(
        &mut self,
        instance: InstanceId,
        object: ObjectId,
        spec: &BlendSpec,
    ) -> Result<(BlendReport, DocChange), DocumentError> {
        let (def, pose) = self.instance_component(instance)?;
        match self.objects.get(object).map(|r| r.owner) {
            Some(ObjectOwner::Definition { def: d, .. }) if d == def => {}
            _ => return Err(DocumentError::UnknownObject),
        }
        // A blend through a placement edits the shape (OW-1).
        self.refuse_def_edit(instance, def)?;
        let s = pose
            .similarity_scale()
            .ok_or(DocumentError::Blend(BlendError::NonUniformInstance))?;
        self.blend_edges(object, &spec_in_definition_units(spec, s))
    }

    /// The gate every reshape passes, like `repair_target`: a visible world
    /// object (inside the current explode scope) or a member of a visible
    /// definition.
    fn reshape_target(&self, object: ObjectId) -> Result<(), DocumentError> {
        self.repair_target(object)
    }

    /// Runs `spec` on `object` and commits the result as one undo step
    /// (`DocAction::Reshaped`): the object takes the new geometry, each
    /// split-off piece becomes a new object beside it (same owner, group,
    /// name and tags), and the change lists the object, the pieces and,
    /// for a definition member, the definition and its instances.
    ///
    /// # Errors
    /// `UnknownObject` / `UnknownComponent` from the gate, or whatever the
    /// spec refuses with; on `Err` nothing changed.
    pub(crate) fn reshape(
        &mut self,
        object: ObjectId,
        spec: ReshapeSpec,
    ) -> Result<(ReshapeReport, DocChange), DocumentError> {
        self.reshape_target(object)?;
        let out = spec.run(&self.objects[object].object)?;
        let rec = &mut self.objects[object];
        let before = std::mem::replace(&mut rec.object, out.after);
        let (owner, name, tags) = (rec.owner, rec.name.clone(), rec.tags.clone());
        // Split-off pieces are the object's own geometry: an imported
        // object's pieces stay imported, as a slice's do.
        let imported = rec.imported;
        let parent = match owner {
            ObjectOwner::World { parent } | ObjectOwner::Definition { parent, .. } => parent,
        };
        let mut split_off = Vec::with_capacity(out.pieces.len());
        for piece in out.pieces {
            let id = self.insert_object_record(ObjectRecord {
                object: piece,
                history: History::new(),
                hidden: false,
                owner,
                name: name.clone(),
                tags: tags.clone(),
                imported,
                locked: false,
            });
            match (owner, parent) {
                (_, Some(g)) => self.groups[g].members.push(NodeId::Object(id)),
                (ObjectOwner::Definition { def, .. }, None) => {
                    self.components[def].members.push(NodeId::Object(id));
                }
                (ObjectOwner::World { .. }, None) => {}
            }
            split_off.push(id);
        }
        self.undo.push(DocAction::Reshaped {
            object,
            before,
            spec,
            split_off: split_off.clone(),
            parent,
        });
        self.commit_new_action();
        self.debug_validate();
        Ok((out.report, self.repair_change(object, &split_off, parent)))
    }

    /// [`Document::reshape`] with the test spec, for the executable specs.
    /// Test support only.
    #[doc(hidden)]
    pub fn reshape_scale_for_test(
        &mut self,
        object: ObjectId,
        scale: f64,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(object, ReshapeSpec::Test { scale })
            .map(|(_, c)| c)
    }
}

// ───────────────────────────────────────────────────────────────── shell

impl Document {
    /// Hollows `object` per `spec` ([`Object::shelled`]) as one undo step
    /// ("Shell"). The open face ids are the object's own.
    ///
    /// # Errors
    /// The reshape gate's, or `Shape(..)` with the [`Object::shelled`]
    /// refusal; on `Err` nothing changed.
    pub fn shell(
        &mut self,
        object: ObjectId,
        spec: &crate::shell::ShellSpec,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(object, ReshapeSpec::Shell(spec.clone()))
            .map(|(_, c)| c)
    }

    /// The object [`Document::shell`] would produce, without committing.
    /// Refuses exactly as the commit would.
    pub fn shell_preview(
        &self,
        object: ObjectId,
        spec: &crate::shell::ShellSpec,
    ) -> Result<Object, DocumentError> {
        self.reshape_target(object)?;
        self.objects[object]
            .object
            .shelled(spec)
            .map_err(DocumentError::Shape)
    }
}
// ═══════════════════════════════════════════════════════════ revolve

/// The profile a Revolve turns: a closed sketch region, or a solid's face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevolveProfile {
    /// A closed region of a visible world sketch; consumed like Follow Me's.
    Region {
        /// The sketch.
        sketch: SketchId,
        /// The region.
        region: SketchRegionId,
    },
    /// A face of a visible world object; the object is untouched.
    Face {
        /// The object.
        object: ObjectId,
        /// The face.
        face: FaceId,
    },
}

/// A Revolve (true-curves.md §5.1): turn a profile about an axis lying in
/// its plane.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RevolveSpec {
    /// A point on the axis.
    pub axis_point: Point3,
    /// The axis direction (any non-zero length; normalized inside).
    pub axis: Vec3,
    /// Radians, `0 < |angle| ≤ 2π`; the sign is the right-hand rule about
    /// `axis`. A full turn closes the solid.
    pub angle: f64,
    /// Facets per full turn, `3..=1024`.
    pub segments: u32,
}

impl Document {
    /// Revolve (true-curves.md §5.1): turns `profile` about the axis of
    /// `spec` into a new world object, through the Follow Me engine with a
    /// synthesized circular path (every segment attributed to the circle the
    /// profile's reference point travels), so walls are stamped as the
    /// surfaces of revolution they are and a profile touching the axis closes
    /// at its poles. A region profile is consumed exactly as by
    /// [`Document::follow_me`] (one undo step restores it); a face profile
    /// leaves its object untouched. A full turn is closed; a partial turn is
    /// capped at both ends.
    ///
    /// # Errors
    /// `UnknownSketch` / `UnknownObject` / `UnknownFace` / `Sketch` for stale
    /// inputs; [`ShapeError::AmountOutOfRange`] for a zero axis, an angle
    /// outside `(0, 2π]` or segments outside `3..=1024`;
    /// [`ShapeError::AxisNotInProfilePlane`] when the axis does not lie in
    /// the profile's plane; Follow Me's own refusals (`ProfileCrossesAxis`,
    /// `PartialSweepOnPole`, …). The document is untouched on every error.
    pub fn revolve(
        &mut self,
        profile: RevolveProfile,
        spec: &RevolveSpec,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "revolve");
        match profile {
            RevolveProfile::Region { sketch, region } => {
                self.explode_scope_sketch(sketch)?;
                if self.sketch_tombstoned(&sketch) || self.sketch_owner_component(sketch).is_some()
                {
                    return Err(DocumentError::UnknownSketch);
                }
                // The profile's edges leave the sketch (OW-1).
                self.refuse_frozen_sketch(sketch)?;
                let s = self
                    .sketches
                    .get(sketch)
                    .ok_or(DocumentError::UnknownSketch)?;
                let prof = s.profile(region).map_err(DocumentError::Sketch)?;
                let scaffolding = s
                    .region_scaffolding(region)
                    .map_err(DocumentError::Sketch)?;
                let object = revolve_profile(&prof, spec)?;
                Ok(self.commit_region_object(sketch, &scaffolding, object, None, None))
            }
            RevolveProfile::Face { object, face } => {
                self.explode_scope_object(object)?;
                let rec = self
                    .objects
                    .get(object)
                    .filter(|r| !r.hidden && r.is_world())
                    .ok_or(DocumentError::UnknownObject)?;
                let prof = rec
                    .object
                    .profile_from_face(face)
                    .ok_or(DocumentError::UnknownFace)?;
                let swept = revolve_profile(&prof, spec)?;
                Ok(self.commit_generated_world_object(swept, None, "Revolve"))
            }
        }
    }

    /// What [`Document::revolve`] would create, without changing anything.
    pub fn revolve_preview(
        &self,
        profile: &RevolveProfile,
        spec: &RevolveSpec,
    ) -> Result<Object, DocumentError> {
        let prof = match *profile {
            RevolveProfile::Region { sketch, region } => {
                if self.sketch_tombstoned(&sketch) {
                    return Err(DocumentError::UnknownSketch);
                }
                self.sketches
                    .get(sketch)
                    .ok_or(DocumentError::UnknownSketch)?
                    .profile(region)
                    .map_err(DocumentError::Sketch)?
            }
            RevolveProfile::Face { object, face } => self
                .objects
                .get(object)
                .filter(|r| !r.hidden && r.is_world())
                .ok_or(DocumentError::UnknownObject)?
                .object
                .profile_from_face(face)
                .ok_or(DocumentError::UnknownFace)?,
        };
        revolve_profile(&prof, spec)
    }

    /// [`Document::revolve`] with a face of a member of `instance`'s
    /// definition as the profile: the WORLD `spec` is mapped through the
    /// instance's pose into the definition, and the result becomes a new
    /// member of that definition (every instance shows it).
    ///
    /// # Errors
    /// `UnknownInstance`, `UnknownObject` (not a member), `UnknownFace`,
    /// `AmbiguousInstanceScale` for a non-similarity pose, and the errors of
    /// [`Document::revolve`].
    pub fn revolve_in_instance(
        &mut self,
        instance: InstanceId,
        object: ObjectId,
        face: FaceId,
        spec: &RevolveSpec,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "revolve_in_instance");
        let (component, pose) = self.instance_component(instance)?;
        // The result is a new member of the shape (OW-1).
        self.refuse_def_edit(instance, component)?;
        pose.similarity_scale()
            .ok_or(DocumentError::AmbiguousInstanceScale)?;
        let inv = pose.inverse().map_err(DocumentError::Transform)?;
        let local = RevolveSpec {
            axis_point: inv.apply_point(spec.axis_point),
            axis: inv.apply_vector(spec.axis),
            angle: if pose.determinant() < 0.0 {
                -spec.angle
            } else {
                spec.angle
            },
            segments: spec.segments,
        };
        let rec = self
            .objects
            .get(object)
            .filter(|r| {
                !r.hidden
                    && matches!(r.owner, ObjectOwner::Definition { def: d, .. } if d == component)
            })
            .ok_or(DocumentError::UnknownObject)?;
        let prof = rec
            .object
            .profile_from_face(face)
            .ok_or(DocumentError::UnknownFace)?;
        let swept = revolve_profile(&prof, &local)?;
        let id = self.insert_object_record(ObjectRecord {
            object: swept,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::Definition {
                def: component,
                parent: None,
            },
            name: None,
            tags: Vec::new(),
            imported: false,
            locked: false,
        });
        self.components[component].members.push(NodeId::Object(id));
        self.undo.push(DocAction::FollowMeFace {
            result: id,
            merged_base: None,
            reanchored: Vec::new(),
        });
        self.commit_new_action();
        self.debug_validate();
        let instances_touched = self.placing_instances(component);
        Ok((
            id,
            DocChange {
                objects_touched: vec![id],
                components_touched: vec![component],
                instances_touched,
                ..Default::default()
            },
        ))
    }

    /// Inserts `object` as a new top-level world object and records it the
    /// way a face-profile Follow Me records its result (hide on undo).
    fn commit_generated_world_object(
        &mut self,
        object: Object,
        parent: Option<GroupId>,
        label: &'static str,
    ) -> (ObjectId, DocChange) {
        let id = self.insert_object_record(ObjectRecord {
            object,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::World { parent },
            name: None,
            tags: Vec::new(),
            imported: false,
            locked: false,
        });
        if let Some(g) = parent {
            self.groups[g].members.push(NodeId::Object(id));
        }
        self.undo.push(DocAction::Generated { result: id, label });
        self.commit_new_action();
        self.debug_validate();
        (
            id,
            DocChange {
                objects_touched: vec![id],
                groups_touched: parent.into_iter().collect(),
                ..Default::default()
            },
        )
    }
}

/// The revolution of `profile` by `spec`: the synthesized circular path
/// (WP12 §2 steps 2–5) handed to [`Object::from_follow_me`].
pub(crate) fn revolve_profile(
    profile: &Profile,
    spec: &RevolveSpec,
) -> Result<Object, DocumentError> {
    let bad = DocumentError::Shape(ShapeError::AmountOutOfRange);
    let a = spec.axis.normalized().map_err(|_| bad)?;
    let full = std::f64::consts::TAU;
    if !spec.angle.is_finite()
        || spec.angle == 0.0
        || spec.angle.abs() > full + tol::NORMAL_DIRECTION
        || !(3..=1024).contains(&spec.segments)
        || ![spec.axis_point.x, spec.axis_point.y, spec.axis_point.z]
            .iter()
            .all(|c| c.is_finite())
    {
        return Err(bad);
    }
    let plane = profile.plane();
    if plane.normal().dot(a).abs() > tol::NORMAL_DIRECTION
        || plane.signed_distance(spec.axis_point).abs() > tol::PLANE_DIST
    {
        return Err(DocumentError::Shape(ShapeError::AxisNotInProfilePlane));
    }
    let foot = |p: Point3| spec.axis_point + a * (p - spec.axis_point).dot(a);
    let rho = |p: Point3| (p - foot(p)).length();
    // Reference point: the outer loop's area centroid, or the farthest
    // outer vertex when the centroid sits on the axis.
    let outer = profile.outer();
    let centroid = crate::station::area_centroid(outer, plane.normal());
    let g = match centroid {
        Some(c) if rho(c) > tol::POINT_MERGE => c,
        _ => {
            let mut best = outer[0];
            for &p in outer {
                if rho(p) > rho(best) {
                    best = p;
                }
            }
            if rho(best) <= tol::POINT_MERGE {
                return Err(DocumentError::Shape(ShapeError::AxisNotInProfilePlane));
            }
            best
        }
    };
    let c = foot(g);
    let r = rho(g);
    let u = (g - c).normalized().map_err(|_| bad)?;
    let w = a.cross(u);
    let point = |phi: f64| {
        if phi == 0.0 {
            g
        } else {
            c + u * (r * phi.cos()) + w * (r * phi.sin())
        }
    };
    let closed = spec.angle.abs() >= full - tol::NORMAL_DIRECTION;
    // The Follow Me engine closes poles only on a full revolution; on an
    // open arc it cannot see the axis and would refuse as a too-tight
    // path. Name the real reason instead (decisions-shapes D-S5).
    if !closed
        && outer
            .iter()
            .chain(profile.holes().iter().flatten())
            .any(|&p| rho(p) <= tol::POINT_MERGE)
    {
        return Err(DocumentError::FollowMe(
            crate::ops::FollowMeError::PartialSweepOnPole,
        ));
    }
    let (points, n) = if closed {
        let n = spec.segments as usize;
        let sign = spec.angle.signum();
        (
            (0..n)
                .map(|j| point(sign * full * j as f64 / n as f64))
                .collect::<Vec<_>>(),
            n,
        )
    } else {
        let n = ((f64::from(spec.segments) * spec.angle.abs() / full).ceil() as usize).max(1);
        (
            (0..=n)
                .map(|j| point(spec.angle * j as f64 / n as f64))
                .collect::<Vec<_>>(),
            n,
        )
    };
    let curves = vec![
        Some(CurveGeom {
            center: c,
            radius: r
        });
        n
    ];
    Object::from_follow_me(profile, &points, closed, &curves).map_err(DocumentError::FollowMe)
}

// ==== true-curves: blend ====
impl Document {
    /// Rounds or chamfers the corner at `vertex` of world sketch `sketch`
    /// ([`Sketch::blend_corner`]) as ONE undo step: the edit is bracketed in
    /// its own sketch gesture, so callers do not bracket it themselves.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownSketch`] — stale or hidden sketch.
    /// - [`DocumentError::SketchGestureAlreadyOpen`] — a gesture is already
    ///   open (gestures never nest).
    /// - [`DocumentError::Sketch`] — the blend was refused (see
    ///   [`Sketch::blend_corner`]); the sketch is untouched and nothing is
    ///   recorded.
    pub fn sketch_blend_corner(
        &mut self,
        sketch: SketchId,
        vertex: SketchVertexId,
        profile: CornerProfile,
    ) -> Result<(CornerBlendReport, DocChange), DocumentError> {
        self.begin_sketch_gesture(sketch)?;
        let blended = match self.sketch_mut(sketch) {
            Some(s) => s.blend_corner(vertex, profile),
            None => {
                self.cancel_sketch_gesture();
                return Err(DocumentError::UnknownSketch);
            }
        };
        match blended {
            Ok(report) => {
                let change = self.end_sketch_gesture(sketch)?;
                Ok((report, change))
            }
            Err(e) => {
                self.cancel_sketch_gesture();
                Err(DocumentError::Sketch(e))
            }
        }
    }
}
// ==== end true-curves: blend ====

// ═══════════════════════════════════════════════════════════ deform

impl Document {
    /// Deform (true-curves.md §5.3): twists, bends, tapers or bends
    /// `object` to a path in place, as one undo step
    /// (`DocAction::Reshaped`). Definition members are reached only from
    /// inside an open explode or group session (the reshape gate).
    ///
    /// # Errors
    /// `UnknownObject` / `UnknownComponent` from the gate, or
    /// [`DocumentError::Shape`] with [`Object::deformed`]'s refusal; on
    /// `Err` nothing changed.
    pub fn deform(
        &mut self,
        object: ObjectId,
        spec: &crate::deform::DeformSpec,
    ) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "deform");
        self.reshape(object, ReshapeSpec::Deform(spec.clone()))
            .map(|(_, c)| c)
    }

    /// What [`Document::deform`] would make of `object`, without changing
    /// anything, for a live preview ([`Object::deform_preview`]): the
    /// commit's object, capped in work and with no limit search.
    ///
    /// # Errors
    /// [`Document::deform`]'s, except that `SelfIntersecting` carries no
    /// limit, plus `Shape(PreviewTooLarge)` when the preview would pass
    /// its work cap.
    pub fn deform_preview(
        &self,
        object: ObjectId,
        spec: &crate::deform::DeformSpec,
    ) -> Result<Object, DocumentError> {
        self.reshape_target(object)?;
        self.objects[object]
            .object
            .deform_preview(spec)
            .map_err(DocumentError::Shape)
    }

    /// [`Document::deform_limit`] under a preview's work cap
    /// ([`Object::deform_preview_limit`]), for the clamp of a live drag.
    ///
    /// # Errors
    /// [`Document::deform_limit`]'s, plus `Shape(PreviewTooLarge)`.
    pub fn deform_preview_limit(
        &self,
        object: ObjectId,
        spec: &crate::deform::DeformSpec,
    ) -> Result<f64, DocumentError> {
        self.reshape_target(object)?;
        self.objects[object]
            .object
            .deform_preview_limit(spec)
            .map_err(DocumentError::Shape)
    }

    /// The largest amount [`Document::deform`] accepts for `spec`
    /// ([`Object::deform_limit`]).
    pub fn deform_limit(
        &self,
        object: ObjectId,
        spec: &crate::deform::DeformSpec,
    ) -> Result<f64, DocumentError> {
        self.reshape_target(object)?;
        self.objects[object]
            .object
            .deform_limit(spec)
            .map_err(DocumentError::Shape)
    }

    /// A Follow Me path's points and whether it closes — the polyline a
    /// Bend to Path or a Loft rail follows.
    ///
    /// # Errors
    /// What [`Document::follow_me`] raises resolving its path.
    pub fn follow_me_path_points(
        &self,
        path: &FollowMePath,
    ) -> Result<(Vec<Point3>, bool), DocumentError> {
        let (points, closed, _) = self.resolve_follow_me_path(path)?;
        Ok((points, closed))
    }
}

// ═══════════════════════════════════════════════════════════ loft

/// One loft source (true-curves.md §5.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoftSource {
    /// A closed region of a visible world sketch (not consumed).
    Region {
        /// The sketch.
        sketch: SketchId,
        /// The region.
        region: SketchRegionId,
    },
    /// A face of a visible world object (untouched).
    Face {
        /// The object.
        object: ObjectId,
        /// The face.
        face: FaceId,
    },
    /// The apex the loft ends in; only as the last source.
    Vertex {
        /// The point.
        point: Point3,
    },
}

impl Document {
    /// Loft (true-curves.md §5.4): skins a new world object through the
    /// sources in order (an optional final `Vertex` is the apex), guided by
    /// up to two `rails`. Profiles are not consumed. The result joins the
    /// first face profile's group, if any; one undo step hides it again
    /// (`DocAction::Generated`).
    ///
    /// # Errors
    /// `UnknownSketch` / `UnknownObject` / `UnknownFace` / `Sketch(..)` for
    /// stale sources; [`ShapeError::LoftProfileHasHole`] for a profile with
    /// holes; [`ShapeError::ShapeDegenerate`] for an apex that is not last;
    /// Follow Me's path errors for a rail; and [`Object::from_loft`]'s
    /// refusals. On `Err` nothing changed.
    pub fn loft(
        &mut self,
        sources: &[LoftSource],
        rails: &[FollowMePath],
        start_offsets: &[i32],
        stations: Option<u32>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "loft");
        for s in sources {
            match *s {
                LoftSource::Region { sketch, .. } => self.explode_scope_sketch(sketch)?,
                LoftSource::Face { object, .. } => self.explode_scope_object(object)?,
                LoftSource::Vertex { .. } => {}
            }
        }
        for r in rails {
            self.explode_scope_follow_path(r)?;
        }
        let spec = self.loft_spec(sources, rails, start_offsets, stations)?;
        let object = Object::from_loft(&spec).map_err(DocumentError::Shape)?;
        let parent = sources.iter().find_map(|s| match *s {
            LoftSource::Face { object, .. } => {
                self.objects.get(object).and_then(|r| r.group_parent())
            }
            _ => None,
        });
        // The loft lands in its face source's group (OW-1: not a frozen one).
        if let Some(g) = parent {
            self.refuse_frozen(&[NodeId::Group(g)])?;
        }
        Ok(self.commit_generated_world_object(object, parent, "Loft"))
    }

    /// What [`Document::loft`] would create, without changing anything.
    pub fn loft_preview(
        &self,
        sources: &[LoftSource],
        rails: &[FollowMePath],
        start_offsets: &[i32],
        stations: Option<u32>,
    ) -> Result<Object, DocumentError> {
        let spec = self.loft_spec(sources, rails, start_offsets, stations)?;
        Object::from_loft(&spec).map_err(DocumentError::Shape)
    }

    /// Each profile's start candidates in loft order from its automatic
    /// start, for the tool's start markers (`crate::loft::start_points`:
    /// its corners where every profile has the same number of corners, else
    /// its original vertices; a start offset counts along this list).
    pub fn loft_start_points(
        &self,
        sources: &[LoftSource],
        start_offsets: &[i32],
    ) -> Result<Vec<Vec<Point3>>, DocumentError> {
        let spec = self.loft_spec(sources, &[], start_offsets, None)?;
        crate::loft::start_points(&spec).map_err(DocumentError::Shape)
    }

    fn loft_spec(
        &self,
        sources: &[LoftSource],
        rails: &[FollowMePath],
        start_offsets: &[i32],
        stations: Option<u32>,
    ) -> Result<crate::loft::LoftSpec, DocumentError> {
        let mut profiles = Vec::new();
        let mut apex = None;
        for (i, s) in sources.iter().enumerate() {
            let prof = match *s {
                LoftSource::Vertex { point } => {
                    if i + 1 != sources.len() {
                        return Err(DocumentError::Shape(ShapeError::ShapeDegenerate));
                    }
                    apex = Some(point);
                    continue;
                }
                LoftSource::Region { sketch, region } => {
                    if self.sketch_tombstoned(&sketch)
                        || self.sketch_owner_component(sketch).is_some()
                    {
                        return Err(DocumentError::UnknownSketch);
                    }
                    self.sketches
                        .get(sketch)
                        .ok_or(DocumentError::UnknownSketch)?
                        .profile(region)
                        .map_err(DocumentError::Sketch)?
                }
                LoftSource::Face { object, face } => self
                    .objects
                    .get(object)
                    .filter(|r| !r.hidden && r.is_world())
                    .ok_or(DocumentError::UnknownObject)?
                    .object
                    .profile_from_face_with_rims(face)
                    .ok_or(DocumentError::UnknownFace)?,
            };
            if !prof.holes().is_empty() {
                return Err(DocumentError::Shape(ShapeError::LoftProfileHasHole));
            }
            let n = prof.outer().len();
            profiles.push(crate::loft::LoftProfile {
                points: prof.outer().to_vec(),
                curves: (0..n).map(|k| prof.outer_curve(k)).collect(),
                smooth: (0..n).map(|k| prof.outer_smooth(k)).collect(),
                through: Vec::new(),
                start_offset: start_offsets.get(profiles.len()).copied().unwrap_or(0),
            });
        }
        let mut rail_pts = Vec::with_capacity(rails.len());
        for r in rails {
            rail_pts.push(self.follow_me_path_points(r)?.0);
        }
        Ok(crate::loft::LoftSpec {
            profiles,
            apex,
            rails: rail_pts,
            stations,
        })
    }
}

/// Sweep options in a definition's frame: a world twist turns the other way
/// under a reflecting pose; the end scale is a ratio and does not change.
pub(super) fn local_sweep_options(pose: &Transform, opts: SweepOptions) -> SweepOptions {
    SweepOptions {
        twist: if pose.determinant() < 0.0 {
            -opts.twist
        } else {
            opts.twist
        },
        ..opts
    }
}

// ─────────────────────────────────────────────────────────────── project

/// A depth in either mode.
fn project_depth(obj: &Object, spec: &crate::project::DepthSpec) -> Result<Object, DocumentError> {
    match spec.mode {
        crate::project::DepthMode::Straight => crate::project::project_depth_straight(obj, spec),
        crate::project::DepthMode::Follow => {
            obj.project_depth_follow(spec).map_err(DocumentError::Shape)
        }
    }
}

impl Document {
    /// Imprints `spec`'s shape on `object` ([`Object::project_imprint`]) as
    /// one undo step ("Project"), returning the region's faces.
    ///
    /// # Errors
    /// The reshape gate's, or `Shape(..)` with the imprint's refusal.
    pub fn project_imprint(
        &mut self,
        object: ObjectId,
        spec: &crate::project::ImprintSpec,
    ) -> Result<(Vec<FaceId>, DocChange), DocumentError> {
        let (report, change) = self.reshape(object, ReshapeSpec::ProjectImprint(spec.clone()))?;
        let ReshapeReport::Faces(faces) = report else {
            unreachable!("an imprint reports its faces")
        };
        Ok((faces, change))
    }

    /// The imprint [`Document::project_imprint`] would make, without
    /// committing: the object and the region's faces.
    pub fn project_imprint_preview(
        &self,
        object: ObjectId,
        spec: &crate::project::ImprintSpec,
    ) -> Result<(Object, Vec<FaceId>), DocumentError> {
        self.reshape_target(object)?;
        self.objects[object]
            .object
            .project_imprint(spec)
            .map_err(DocumentError::Shape)
    }

    /// Engraves (`depth > 0`) or embosses an imprinted region as one undo
    /// step ("Project"). Straight mode booleans the swept region, so its
    /// refusals (`Boolean(..)`) pass through; a straight depth that would
    /// cut the object apart refuses `Shape(WouldSever)`.
    pub fn project_depth(
        &mut self,
        object: ObjectId,
        spec: &crate::project::DepthSpec,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(object, ReshapeSpec::ProjectDepth(spec.clone()))
            .map(|(_, c)| c)
    }

    /// The object [`Document::project_depth`] would make, without
    /// committing.
    pub fn project_depth_preview(
        &self,
        object: ObjectId,
        spec: &crate::project::DepthSpec,
    ) -> Result<Object, DocumentError> {
        self.reshape_target(object)?;
        project_depth(&self.objects[object].object, spec)
    }
}

impl Document {
    /// The imprint spec projecting `regions` of `sketch` (every face-bearing
    /// region when empty) onto `object`: the sketch plane's normal, signed
    /// to point from the sketch toward the object's bounding-box center.
    ///
    /// # Errors
    /// `UnknownSketch`, `UnknownObject`, `Sketch(..)` for a region that does
    /// not profile, `Shape(ProjectNeedsClosedShape)` when there is no region,
    /// `Shape(ProjectMisses)` when the object's center lies in the sketch
    /// plane (no side to project toward).
    pub fn project_spec_from_sketch(
        &self,
        object: ObjectId,
        sketch: SketchId,
        regions: &[SketchRegionId],
    ) -> Result<crate::project::ImprintSpec, DocumentError> {
        let sk = self.sketch(sketch).ok_or(DocumentError::UnknownSketch)?;
        let obj = self.object(object).ok_or(DocumentError::UnknownObject)?;
        let ids: Vec<SketchRegionId> = if regions.is_empty() {
            sk.regions().keys().collect()
        } else {
            regions.to_vec()
        };
        let mut out = Vec::new();
        for r in ids {
            let Ok(p) = sk.profile(r) else {
                if regions.is_empty() {
                    continue; // a bare-edge loop has no face to project
                }
                return Err(DocumentError::Sketch(
                    sk.profile(r).expect_err("checked above"),
                ));
            };
            let n = p.outer().len();
            let holes = p.holes();
            out.push(crate::project::ProjectedRegion {
                outer: p.outer().to_vec(),
                holes: holes.to_vec(),
                outer_curves: (0..n).map(|k| p.outer_curve(k)).collect(),
                hole_curves: (0..holes.len())
                    .map(|i| (0..holes[i].len()).map(|k| p.hole_curve(i, k)).collect())
                    .collect(),
                outer_smooth: (0..n).map(|k| p.outer_smooth(k)).collect(),
                hole_smooth: (0..holes.len())
                    .map(|i| (0..holes[i].len()).map(|k| p.hole_smooth(i, k)).collect())
                    .collect(),
            });
        }
        if out.is_empty() {
            return Err(DocumentError::Shape(ShapeError::ProjectNeedsClosedShape));
        }
        let (mut lo, mut hi) = (
            Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for v in obj.vertices().values() {
            let p = v.position;
            lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        let center = Point3::new(
            (lo.x + hi.x) / 2.0,
            (lo.y + hi.y) / 2.0,
            (lo.z + hi.z) / 2.0,
        );
        let plane = sk.plane();
        let side = plane.signed_distance(center);
        if side.abs() <= crate::tol::PLANE_DIST {
            return Err(DocumentError::Shape(ShapeError::ProjectMisses));
        }
        let dir = if side > 0.0 {
            plane.normal()
        } else {
            plane.normal() * -1.0
        };
        Ok(crate::project::ImprintSpec { dir, regions: out })
    }
}

// ──────────────────────────────────────────────────────────────── smooth

/// `obj` with `crease` set on `edges`. Refuses a boundary edge
/// (`CreaseOnBoundary`) and, when creasing, a soft seam
/// (`CreaseOnSoftEdge`); a stale edge is `ShapeDegenerate`.
fn with_creases(
    obj: &Object,
    edges: &[crate::ids::EdgeId],
    crease: bool,
) -> Result<Object, DocumentError> {
    let shape = DocumentError::Shape;
    let mut after = obj.clone();
    for &e in edges {
        let Some(edge) = obj.edges().get(e) else {
            return Err(shape(ShapeError::ShapeDegenerate));
        };
        if edge.twin_half_edge.is_none() {
            return Err(shape(ShapeError::CreaseOnBoundary));
        }
        if crease && obj.edge_smoothness(e) == crate::surface::EdgeSmoothness::Seam {
            return Err(shape(ShapeError::CreaseOnSoftEdge));
        }
        after.edges[e].crease = crease;
    }
    after
        .validate()
        .map_err(|_| shape(ShapeError::ShapeDegenerate))?;
    Ok(after)
}

/// The change an attribute write reports: a Smooth level on an object
/// changes what the object shows, so that object is touched; any other
/// attribute touches nothing.
pub(crate) fn attr_display_change(target: &AttrTarget, ns: &str) -> DocChange {
    let mut change = DocChange::default();
    if ns == crate::smooth::SMOOTH_NS
        && let AttrTarget::Entity(crate::document::EntityRef::Object(id)) = target
    {
        change.objects_touched.push(*id);
    }
    change
}

impl Document {
    /// The object's Smooth level: 0 when unset or malformed (a non-integer
    /// value, or one outside 1..=3).
    pub fn smooth_level(&self, object: ObjectId) -> u8 {
        let target = AttrTarget::Entity(crate::document::EntityRef::Object(object));
        crate::smooth::level_of(
            self.attrs
                .get(&target)
                .and_then(|d| d.get(crate::smooth::SMOOTH_NS))
                .and_then(|ns| ns.get(crate::smooth::SMOOTH_LEVEL_KEY)),
        )
    }

    /// Sets the Smooth level (0 = Off, which deletes the key; 1..=3 sets
    /// it) as one undo step, through the attribute table so undo restores
    /// it exactly. Setting the level an object already has changes
    /// nothing and records nothing.
    ///
    /// # Errors
    /// `Shape(AmountOutOfRange)` for a level over 3, `Shape(SmoothTooDense)`
    /// for a cage that would subdivide past the surface cap; the reshape
    /// gate's (`UnknownObject`, `UnknownComponent`).
    pub fn set_smooth_level(
        &mut self,
        object: ObjectId,
        level: u8,
    ) -> Result<DocChange, DocumentError> {
        if level > crate::smooth::MAX_SMOOTH_LEVEL {
            return Err(DocumentError::Shape(ShapeError::AmountOutOfRange));
        }
        self.reshape_target(object)?;
        if self.smooth_level(object) == level {
            return Ok(DocChange::default());
        }
        if level > 0 {
            self.smooth_cap_check(object, level)?;
        }
        let target = AttrTarget::Entity(crate::document::EntityRef::Object(object));
        let mut change = if level == 0 {
            self.attr_delete(
                target,
                crate::smooth::SMOOTH_NS,
                Some(crate::smooth::SMOOTH_LEVEL_KEY),
            )?
        } else {
            self.attr_set(
                target,
                crate::smooth::SMOOTH_NS,
                crate::smooth::SMOOTH_LEVEL_KEY,
                crate::attr::AttrValue::Int(i64::from(level)),
            )?
        };
        self.complete_change(&mut change);
        Ok(change)
    }

    /// Sets `crease` on `edges` of `object` (kernel edges, already expanded
    /// to their logical edges) as one undo step ("Crease"/"Uncrease").
    ///
    /// # Errors
    /// `Shape(CreaseOnSoftEdge)`, `Shape(CreaseOnBoundary)`,
    /// `Shape(ShapeDegenerate)` for a stale edge; the reshape gate's.
    pub fn set_crease(
        &mut self,
        object: ObjectId,
        edges: &[crate::ids::EdgeId],
        crease: bool,
    ) -> Result<DocChange, DocumentError> {
        self.reshape(
            object,
            ReshapeSpec::SetCrease {
                edges: edges.to_vec(),
                crease,
            },
        )
        .map(|(_, c)| c)
    }

    /// Refuses `level` for `object` when its Smooth surface would pass the
    /// face cap (`subdiv::MAX_SUBDIV_FACES`), naming both counts.
    fn smooth_cap_check(&self, object: ObjectId, level: u8) -> Result<(), DocumentError> {
        let faces = crate::subdiv::subdivided_face_count(&self.objects[object].object, level);
        if faces > crate::subdiv::MAX_SUBDIV_FACES {
            return Err(DocumentError::Shape(ShapeError::SmoothTooDense {
                faces,
                cap: crate::subdiv::MAX_SUBDIV_FACES,
            }));
        }
        Ok(())
    }

    /// Replaces the cage with its subdivided solid at its level and clears
    /// the level, as ONE undo step: undo restores the cage and the level.
    ///
    /// # Errors
    /// `Shape(AmountOutOfRange)` when the object has no level;
    /// `Shape(SmoothTooDense)` when the cage grew past the surface cap
    /// since its level was set (or a file carries such a level); the
    /// bake's (`ShapeDegenerate`); the reshape gate's.
    pub fn bake(&mut self, object: ObjectId) -> Result<DocChange, DocumentError> {
        self.reshape_target(object)?;
        let level = self.smooth_level(object);
        if level == 0 {
            return Err(DocumentError::Shape(ShapeError::AmountOutOfRange));
        }
        self.smooth_cap_check(object, level)?;
        let (_, change) = self.reshape(object, ReshapeSpec::Bake { level })?;
        // Fold the level's removal into the bake's own undo entry.
        let target = AttrTarget::Entity(crate::document::EntityRef::Object(object));
        let removed = self.apply_attr_remove(
            &target,
            crate::smooth::SMOOTH_NS,
            Some(crate::smooth::SMOOTH_LEVEL_KEY),
        );
        let bake = self.undo.pop().expect("reshape pushed the bake");
        self.undo.push(DocAction::Compound {
            actions: vec![
                bake,
                DocAction::AttrDeleted {
                    target,
                    ns: crate::smooth::SMOOTH_NS.to_string(),
                    removed,
                    whole_ns: false,
                },
            ],
            meta: None,
        });
        self.debug_validate();
        Ok(change)
    }

    /// Whether any visible object has a Smooth level.
    pub fn has_smoothed_objects(&self) -> bool {
        self.objects
            .iter()
            .any(|(id, r)| !r.hidden && self.smooth_level(id) > 0)
    }

    /// The visible objects with a Smooth level, in id order.
    pub fn smoothed_object_ids(&self) -> Vec<ObjectId> {
        self.objects
            .iter()
            .filter(|(id, r)| !r.hidden && self.smooth_level(*id) > 0)
            .map(|(id, _)| id)
            .collect()
    }
}

/// The True Curves marks of an Object that its vertex set does not show,
/// posed by `pose`: crease segments; each face with a blend tag or an
/// analytic surface claim (any kind, the cylinder included) by its outer
/// ring's centroid, with the tag's kind, role and record digest and the
/// claim mapped by `pose`; and each edge with a circle or polygon claim by
/// its ends, with both claims mapped by `pose`. A claim `pose` cannot map
/// (a non-similarity) is kept as [`PosedClaim::Unmappable`], which matches
/// nothing, so a claim is never silently forgotten by the comparison.
struct CurveMarks {
    creases: Vec<(Point3, Point3)>,
    faces: Vec<(Point3, PosedClaim<crate::topo::SurfaceRef>, String)>,
    edges: Vec<CurveEdgeMark>,
}

/// One edge's circle and polygon claims, posed, with its posed ends.
struct CurveEdgeMark {
    ends: (Point3, Point3),
    curve: PosedClaim<crate::sketch::CurveGeom>,
    polygon: PosedClaim<crate::sketch::CurveGeom>,
}

/// An analytic claim under a pose.
#[derive(Clone, Copy)]
enum PosedClaim<T> {
    None,
    Some(T),
    /// The object carries a claim the pose cannot map.
    Unmappable,
}

impl<T: Copy> PosedClaim<T> {
    fn of(claim: Option<T>, map: impl FnOnce(T) -> Option<T>) -> Self {
        match claim {
            None => PosedClaim::None,
            Some(c) => map(c).map_or(PosedClaim::Unmappable, PosedClaim::Some),
        }
    }

    fn same(&self, other: &Self, eq: impl Fn(&T, &T) -> bool) -> bool {
        match (self, other) {
            (PosedClaim::None, PosedClaim::None) => true,
            (PosedClaim::Some(a), PosedClaim::Some(b)) => eq(a, b),
            _ => false,
        }
    }
}

fn curve_marks(obj: &Object, pose: &Transform) -> CurveMarks {
    let scale = pose.similarity_scale();
    let creases = obj
        .edges()
        .keys()
        .filter(|&e| obj.edges()[e].crease)
        .filter_map(|e| obj.edge_endpoints(e))
        .map(|(a, b)| (pose.apply_point(a), pose.apply_point(b)))
        .collect();
    let map_geom = |g: crate::sketch::CurveGeom| crate::ops::map_curve(Some(g), pose, scale);
    let edges = obj
        .edges()
        .iter()
        .filter(|(_, e)| e.curve.is_some() || e.polygon.is_some())
        .filter_map(|(id, e)| {
            let (a, b) = obj.edge_endpoints(id)?;
            Some(CurveEdgeMark {
                ends: (pose.apply_point(a), pose.apply_point(b)),
                curve: PosedClaim::of(e.curve, map_geom),
                polygon: PosedClaim::of(e.polygon, map_geom),
            })
        })
        .collect();
    let mut faces = Vec::new();
    for face in obj.faces().values() {
        if face.blend.is_none() && face.surface.is_none() {
            continue;
        }
        let surface = PosedClaim::of(face.surface, |s| {
            crate::ops::map_surface(Some(s), pose, scale)
        });
        let tag = face
            .blend
            .map(|t| {
                format!(
                    "{:?}/{:?}/{}",
                    t.kind,
                    t.role,
                    obj.blend_record_digest(t.id).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let ring: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
        let n = ring.len().max(1) as f64;
        let sum = ring.iter().fold(crate::math::Vec3::ZERO, |acc, p| {
            acc + (*p - Point3::ORIGIN)
        });
        let centroid = pose.apply_point(Point3::ORIGIN + sum / n);
        faces.push((centroid, surface, tag));
    }
    CurveMarks {
        creases,
        faces,
        edges,
    }
}

/// Whether two posed surface claims are the same surface to the
/// precision `hew.query.signature` sees ([`crate::tol::LINK_POSE_MATCH`]
/// on lengths and points, the same number on unit-vector components and
/// angles), so anything the signature calls identical links.
fn same_claimed_surface(a: &crate::topo::SurfaceRef, b: &crate::topo::SurfaceRef) -> bool {
    use crate::topo::SurfaceRef as S;
    let tol = crate::tol::LINK_POSE_MATCH;
    let near = |p: Point3, q: Point3| p.approx_eq(q, tol);
    let close = |x: f64, y: f64| (x - y).abs() <= tol;
    let line = |u: crate::math::Vec3, v: crate::math::Vec3| {
        (u - v).length() <= tol || (u + v).length() <= tol
    };
    let ray = |u: crate::math::Vec3, v: crate::math::Vec3| (u - v).length() <= tol;
    let off_line = |p: Point3, origin: Point3, axis: crate::math::Vec3| {
        let d = p - origin;
        (d - axis * d.dot(axis)).length()
    };
    match (*a, *b) {
        (
            S::Cylinder {
                axis_point: pa,
                axis: xa,
                radius: ra,
            },
            S::Cylinder {
                axis_point: pb,
                axis: xb,
                radius: rb,
            },
        ) => line(xa, xb) && close(ra, rb) && off_line(pb, pa, xa) <= tol,
        (
            S::Sphere {
                center: ca,
                radius: ra,
            },
            S::Sphere {
                center: cb,
                radius: rb,
            },
        ) => near(ca, cb) && close(ra, rb),
        (
            S::Torus {
                center: ca,
                axis: xa,
                major_radius: ma,
                minor_radius: na,
            },
            S::Torus {
                center: cb,
                axis: xb,
                major_radius: mb,
                minor_radius: nb,
            },
        ) => near(ca, cb) && line(xa, xb) && close(ma, mb) && close(na, nb),
        (
            S::Cone {
                apex: pa,
                axis: xa,
                half_angle: ha,
            },
            S::Cone {
                apex: pb,
                axis: xb,
                half_angle: hb,
            },
        ) => near(pa, pb) && ray(xa, xb) && close(ha, hb),
        _ => false,
    }
}

impl Document {
    /// Whether `keeper` posed by `pose` and `joiner` carry the same True
    /// Curves marks and analytic claims (creases, blend tags and records,
    /// every face's surface claim, every edge's circle and polygon claim)
    /// and the same Smooth level: what the vertex-set check of
    /// [`Document::join_linked`] cannot see. A look-alike that differs in
    /// any of them would lose it to the shared solid. This is the same set
    /// of facts `hew.query.signature` hashes, so the two agree.
    pub(super) fn same_curve_marks(
        &self,
        keeper: ObjectId,
        joiner: ObjectId,
        pose: &Transform,
    ) -> bool {
        if self.smooth_level(keeper) != self.smooth_level(joiner) {
            return false;
        }
        let a = curve_marks(&self.objects[keeper].object, pose);
        let b = curve_marks(&self.objects[joiner].object, &Transform::IDENTITY);
        let tol = crate::tol::LINK_POSE_MATCH;
        let same_segment = |x: &(Point3, Point3), y: &(Point3, Point3)| {
            (x.0.approx_eq(y.0, tol) && x.1.approx_eq(y.1, tol))
                || (x.0.approx_eq(y.1, tol) && x.1.approx_eq(y.0, tol))
        };
        fn both_ways<T>(xs: &[T], ys: &[T], same: impl Fn(&T, &T) -> bool) -> bool {
            xs.len() == ys.len()
                && xs.iter().all(|x| ys.iter().any(|y| same(x, y)))
                && ys.iter().all(|y| xs.iter().any(|x| same(x, y)))
        }
        let same_geom = |g: &crate::sketch::CurveGeom, h: &crate::sketch::CurveGeom| {
            g.center.approx_eq(h.center, tol) && (g.radius - h.radius).abs() <= tol
        };
        let same_face =
            |x: &(Point3, PosedClaim<crate::topo::SurfaceRef>, String),
             y: &(Point3, PosedClaim<crate::topo::SurfaceRef>, String)| {
                x.2 == y.2 && x.0.approx_eq(y.0, tol) && x.1.same(&y.1, same_claimed_surface)
            };
        let same_edge = |x: &CurveEdgeMark, y: &CurveEdgeMark| {
            same_segment(&x.ends, &y.ends)
                && x.curve.same(&y.curve, same_geom)
                && x.polygon.same(&y.polygon, same_geom)
        };
        both_ways(&a.creases, &b.creases, same_segment)
            && both_ways(&a.faces, &b.faces, same_face)
            && both_ways(&a.edges, &b.edges, same_edge)
    }

    /// Takes the Smooth level a boolean's result received with its first
    /// operand's identity attributes ([`Document::copy_identity_attrs`])
    /// back off, unrecorded (the result is new in the same step):
    /// [`Document::carry_smooth_level`] sets the level a result gets, held
    /// to the face cap.
    pub(super) fn drop_copied_smooth_level(&mut self, id: ObjectId) {
        let key = AttrTarget::Entity(crate::document::EntityRef::Object(id));
        if let Some(dict) = self.attrs.get_mut(&key) {
            dict.remove(crate::smooth::SMOOTH_NS);
            if dict.is_empty() {
                self.attrs.remove(&key);
            }
        }
    }

    /// Carries a Smooth level onto the results of an operation that
    /// replaces objects (a boolean): the highest level among `from` is set
    /// on each of `to`, applied now and returned as the follow-up actions
    /// that record it in the operation's own undo entry. Nothing when no
    /// source has a level.
    pub(super) fn carry_smooth_level(
        &mut self,
        from: &[ObjectId],
        to: &[ObjectId],
    ) -> Vec<DocAction> {
        let level = from
            .iter()
            .map(|&o| self.smooth_level(o))
            .max()
            .unwrap_or(0);
        if level == 0 {
            return Vec::new();
        }
        to.iter()
            .filter_map(|&o| {
                // The highest level up to the operands' that stays under the
                // face cap (as setting a level directly checks); a result
                // too dense for any level stays unsmoothed.
                let level = (1..=level).rev().find(|&l| {
                    crate::subdiv::subdivided_face_count(&self.objects[o].object, l)
                        <= crate::subdiv::MAX_SUBDIV_FACES
                })?;
                let target = AttrTarget::Entity(crate::document::EntityRef::Object(o));
                let value = crate::attr::AttrValue::Int(i64::from(level));
                let prior = self.apply_attr_set(
                    &target,
                    crate::smooth::SMOOTH_NS,
                    crate::smooth::SMOOTH_LEVEL_KEY,
                    value.clone(),
                );
                Some(DocAction::AttrSet {
                    target,
                    ns: crate::smooth::SMOOTH_NS.to_string(),
                    key: crate::smooth::SMOOTH_LEVEL_KEY.to_string(),
                    prior,
                    value,
                })
            })
            .collect()
    }
}

// ==== profiles as library documents (WP11) ====

/// Facets per quarter turn of an arc when a profile is drawn into a
/// document: fine enough to read as a curve, coarse enough to edit.
const PROFILE_DOC_ARC_SEGMENTS: u32 = 8;

/// The chain of `edges` of `sketch`, ordered from its start to its end:
/// its points and, per edge, the analytic curve it belongs to. The chain
/// must be one open chain (two ends of degree 1, every other vertex of
/// degree 2). The start is the end touched by the first listed edge.
fn ordered_chain(
    sk: &Sketch,
    edges: &[SketchEdgeId],
) -> Result<(Vec<Point3>, Vec<Option<crate::sketch::CurveGeom>>), BlendError> {
    let bad = BlendError::InvalidProfile;
    if edges.is_empty() {
        return Err(bad);
    }
    let mut ends: Vec<(SketchEdgeId, SketchVertexId, SketchVertexId)> = Vec::new();
    for &e in edges {
        let ed = sk.edges().get(e).ok_or(bad)?;
        if ed.from == ed.to || ends.iter().any(|&(x, ..)| x == e) {
            return Err(bad);
        }
        ends.push((e, ed.from, ed.to));
    }
    let mut degree: std::collections::BTreeMap<SketchVertexId, u32> =
        std::collections::BTreeMap::new();
    for &(_, a, b) in &ends {
        *degree.entry(a).or_default() += 1;
        *degree.entry(b).or_default() += 1;
    }
    if degree.values().any(|&d| d > 2) || degree.values().filter(|&&d| d == 1).count() != 2 {
        return Err(bad);
    }
    let start = ends
        .iter()
        .flat_map(|&(_, a, b)| [a, b])
        .find(|v| degree[v] == 1)
        .ok_or(bad)?;
    let mut points = vec![sk.vertices()[start].position];
    let mut curves = Vec::new();
    let mut used = vec![false; ends.len()];
    let mut at = start;
    for _ in 0..ends.len() {
        let Some((i, next)) = ends.iter().enumerate().find_map(|(i, &(_, a, b))| {
            if used[i] {
                None
            } else if a == at {
                Some((i, b))
            } else if b == at {
                Some((i, a))
            } else {
                None
            }
        }) else {
            return Err(bad);
        };
        used[i] = true;
        let geom = sk.edge_curve(ends[i].0).and_then(|c| sk.curve_geom(c));
        curves.push(geom);
        points.push(sk.vertices()[next].position);
        at = next;
    }
    if used.iter().any(|&u| !u) {
        return Err(bad);
    }
    Ok((points, curves))
}

impl Document {
    /// A new document holding exactly one world sketch: the profile drawn
    /// by the open chain `edges` of `sketch`, normalized (I-16) so its
    /// corner sits at the origin of the ground plane and its width is the
    /// drawn width. The `.hew` content of a profile Library item. The
    /// chain's direction is its first listed edge's end; list edges from
    /// the profile's start (on the bearing face) to its end.
    ///
    /// # Errors
    /// `UnknownSketch`; `Blend(InvalidProfile)` when the edges are not one
    /// open chain, or the chain does not make a valid profile.
    pub fn profile_document_from_chain(
        &self,
        sketch: SketchId,
        edges: &[SketchEdgeId],
    ) -> Result<Document, DocumentError> {
        let sk = self.sketch(sketch).ok_or(DocumentError::UnknownSketch)?;
        let (points, curves) = ordered_chain(sk, edges).map_err(DocumentError::Blend)?;
        Self::profile_document_from_polyline(&points, &curves, sk.plane().normal())
    }

    /// [`Document::profile_document_from_chain`] for a chain already read
    /// out as an ordered polyline (a drawing on a face, which has no sketch
    /// of its own): `points` from the profile's start to its end, `curves`
    /// the analytic circle of each of the `points.len() - 1` segments (or
    /// `None`), `normal` the drawing plane's normal.
    ///
    /// # Errors
    /// `Blend(InvalidProfile)` when the polyline does not make a valid
    /// profile.
    pub fn profile_document_from_polyline(
        points: &[Point3],
        curves: &[Option<crate::sketch::CurveGeom>],
        normal: Vec3,
    ) -> Result<Document, DocumentError> {
        let (shape, width) = crate::blend::ProfileShape::from_chain(points, curves, normal)
            .map_err(DocumentError::Blend)?;
        Self::profile_document_from_shape(&shape, width)
    }

    /// A new document holding `shape` drawn at `width` meters on the ground
    /// plane with its corner at the origin (the inverse of I-16's frame):
    /// the start `(1, 0)` lies `width` along the plane's first axis, the end
    /// `(0, b)` `b·width` along its second. Arc facets keep their circle.
    ///
    /// # Errors
    /// `Blend(InvalidProfile)` when the shape does not validate or `width`
    /// is not a positive finite length.
    pub fn profile_document_from_shape(
        shape: &crate::blend::ProfileShape,
        width: f64,
    ) -> Result<Document, DocumentError> {
        let bad = DocumentError::Blend(BlendError::InvalidProfile);
        if !(width.is_finite() && width > tol::POINT_MERGE) {
            return Err(bad);
        }
        let samples = shape
            .sample(PROFILE_DOC_ARC_SEGMENTS)
            .map_err(DocumentError::Blend)?;
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let plane =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), normal).map_err(|_| bad)?;
        let (u, v) = crate::geom2d::plane_axes(normal);
        let at = |p: [f64; 2]| -> Point3 {
            Point3::new(0.0, 0.0, 0.0) + u * (p[0] * width) + v * (p[1] * width)
        };
        let mut doc = Document::new();
        let s = doc.add_sketch(plane);
        doc.begin_sketch_gesture(s)?;
        let drawn = (|| -> Result<(), crate::sketch::SketchError> {
            let sk = doc
                .sketch_mut(s)
                .ok_or(crate::sketch::SketchError::UnknownVertex)?;
            let mut open: Option<([f64; 2], f64)> = None;
            for w in samples.windows(2) {
                let (a, (b, hint)) = (w[0].0, w[1]);
                if hint != open {
                    if open.is_some() {
                        sk.end_curve();
                    }
                    if let Some((c, r)) = hint {
                        sk.begin_curve_with(crate::sketch::CurveGeom {
                            center: at(c),
                            radius: r * width,
                        })?;
                    }
                    open = hint;
                }
                sk.add_segment(at(a), at(b))?;
            }
            if open.is_some() {
                sk.end_curve();
            }
            Ok(())
        })();
        match drawn {
            Ok(()) => {
                doc.end_sketch_gesture(s)?;
                Ok(doc)
            }
            Err(e) => {
                doc.cancel_sketch_gesture();
                Err(DocumentError::Sketch(e))
            }
        }
    }

    /// Reads a profile item's document: its single open chain as a shape
    /// and the drawn width (`ProfileShape::from_chain`). The chain's start
    /// is the end whose edge begins at it, as the profile documents this
    /// crate writes do.
    ///
    /// # Errors
    /// `Blend(InvalidProfile)` unless the document holds exactly one world
    /// sketch that is one valid open chain.
    pub fn profile_shape_of(&self) -> Result<(crate::blend::ProfileShape, f64), DocumentError> {
        let bad = DocumentError::Blend(BlendError::InvalidProfile);
        let mut world = self
            .sketch_ids()
            .into_iter()
            .filter(|&s| self.sketch_owner_component(s).is_none());
        let (Some(id), None) = (world.next(), world.next()) else {
            return Err(bad);
        };
        let sk = self.sketch(id).ok_or(DocumentError::UnknownSketch)?;
        // Start at the degree-1 vertex that is the `from` of its edge, the
        // direction the writer draws; ties fall to the first edge.
        let mut edges: Vec<SketchEdgeId> = sk.edges().keys().collect();
        let mut degree: std::collections::BTreeMap<SketchVertexId, u32> =
            std::collections::BTreeMap::new();
        for &e in &edges {
            let ed = &sk.edges()[e];
            *degree.entry(ed.from).or_default() += 1;
            *degree.entry(ed.to).or_default() += 1;
        }
        if let Some(pos) = edges.iter().position(|&e| degree[&sk.edges()[e].from] == 1) {
            edges.rotate_left(pos);
        }
        let (points, curves) = ordered_chain(sk, &edges).map_err(DocumentError::Blend)?;
        crate::blend::ProfileShape::from_chain(&points, &curves, sk.plane().normal())
            .map_err(DocumentError::Blend)
    }
}
/// Ceiling on the instance count of any array, enforced at the trust
/// boundaries (the wasm layer and the Hew API; recorded sessions are plain
/// JSON replayed verbatim, so a hand-edited count must fail typed instead of
/// hanging the engine). The single source of truth: the UI reads it through
/// `Scene::max_array_count`, so the app-side cap cannot drift.
pub const MAX_ARRAY_COUNT: u32 = 1000;

/// What [`Document::array_along_path`] places: how many instances, the scale
/// ramp, and whether copies turn to follow the path.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PathArraySpec {
    /// Total instances INCLUDING the original (2 to [`MAX_ARRAY_COUNT`]).
    pub count: u32,
    /// Uniform scale at the first copy position (the original stays as it
    /// is) ...
    pub scale_from: f64,
    /// ... and at the last; linear in between. Both must be finite and > 0.
    pub scale_to: f64,
    /// Rotate copies to follow the path tangent.
    pub align: bool,
}

/// One path frame as a placement: columns `(t, n, t × n)`, origin `p`.
fn frame_transform(t: Vec3, n: Vec3, p: Point3) -> Transform {
    let b = t.cross(n);
    Transform::from_affine(&[
        t.x, n.x, b.x, p.x, //
        t.y, n.y, b.y, p.y, //
        t.z, n.z, b.z, p.z,
    ])
}

/// The placements of copies `1..count` of a path array over the polyline
/// `points` (see [`Document::array_along_path`] for the construction).
fn path_array_transforms(
    points: &[Point3],
    closed: bool,
    spec: &PathArraySpec,
) -> Result<Vec<Transform>, DocumentError> {
    let shape = |e| DocumentError::Shape(e);
    let finite_positive = |x: f64| x.is_finite() && x > 0.0;
    if spec.count < 2
        || spec.count > MAX_ARRAY_COUNT
        || !finite_positive(spec.scale_from)
        || !finite_positive(spec.scale_to)
    {
        return Err(shape(ShapeError::AmountOutOfRange));
    }
    let nseg = if closed {
        points.len()
    } else {
        points.len().saturating_sub(1)
    };
    if nseg == 0 {
        return Err(shape(ShapeError::PathTooShortForCount));
    }
    let seg = |k: usize| (points[k], points[(k + 1) % points.len()]);
    // Cumulative arc length at the start of each segment, then the total.
    let mut cum = Vec::with_capacity(nseg + 1);
    let mut total = 0.0;
    for k in 0..nseg {
        cum.push(total);
        let (a, b) = seg(k);
        total += (b - a).length();
    }
    cum.push(total);
    if total <= tol::POINT_MERGE {
        return Err(shape(ShapeError::PathTooShortForCount));
    }
    // Directions, then normals parallel-transported across each vertex.
    let mut dirs = Vec::with_capacity(nseg);
    for k in 0..nseg {
        let (a, b) = seg(k);
        dirs.push(
            (b - a)
                .normalized()
                .map_err(|_| shape(ShapeError::ShapeDegenerate))?,
        );
    }
    let mut normals = Vec::with_capacity(nseg);
    normals.push(geom2d::plane_axes(dirs[0]).0);
    for k in 1..nseg {
        let (t_in, t_out, n_in) = (dirs[k - 1], dirs[k], normals[k - 1]);
        let axis = t_in.cross(t_out);
        let rot = if axis.length() <= tol::NORMALIZE_MIN_LENGTH {
            if t_in.dot(t_out) > 0.0 {
                Transform::IDENTITY
            } else {
                // A full reversal has no unique minimal rotation; turn half a
                // revolution about the carried normal (it keeps `n`).
                Transform::rotation(n_in, std::f64::consts::PI)
                    .map_err(|_| shape(ShapeError::ShapeDegenerate))?
            }
        } else {
            Transform::rotation(axis, axis.length().atan2(t_in.dot(t_out)))
                .map_err(|_| shape(ShapeError::ShapeDegenerate))?
        };
        let n = rot
            .apply_vector(n_in)
            .normalized()
            .map_err(|_| shape(ShapeError::ShapeDegenerate))?;
        normals.push(n);
    }
    // (position, tangent, normal) at arc length `s`; a vertex takes its
    // outgoing segment, the very end of an open path its last.
    let frame_at = |s: f64| {
        let k = cum[..nseg].partition_point(|&c| c <= s).max(1) - 1;
        let (a, _) = seg(k);
        (a + dirs[k] * (s - cum[k]), dirs[k], normals[k])
    };
    let frame = |s: f64| {
        let (p, t, n) = frame_at(s);
        if spec.align {
            frame_transform(t, n, p)
        } else {
            Transform::translation(p.to_vec())
        }
    };
    let f0 = frame(0.0);
    let f0_inv = f0.inverse().map_err(DocumentError::Transform)?;
    let copies = spec.count - 1;
    let divisions = if closed { spec.count } else { spec.count - 1 };
    let ramp_steps = f64::from(spec.count.saturating_sub(2).max(1));
    let mut out = Vec::with_capacity(copies as usize);
    for i in 1..=copies {
        let s = total * f64::from(i) / f64::from(divisions);
        let sigma =
            spec.scale_from + (spec.scale_to - spec.scale_from) * f64::from(i - 1) / ramp_steps;
        out.push(
            f0_inv
                .then(&Transform::uniform_scale(sigma))
                .then(&frame(s)),
        );
    }
    Ok(out)
}

impl Document {
    /// The placements of the copies of an array along `path`, in order
    /// (copy 1 first); the original stays where it is. Pure query, used by
    /// the ghost preview and by [`Document::array_along_path`].
    ///
    /// # Errors
    /// As [`Document::array_along_path`].
    pub fn path_array_placements(
        &self,
        path: &FollowMePath,
        spec: &PathArraySpec,
    ) -> Result<Vec<Transform>, DocumentError> {
        let (points, closed, _) = self.resolve_follow_me_path(path)?;
        path_array_transforms(&points, closed, spec)
    }

    /// Copies `nodes` along `path`. Copy `i` (1 to `count − 1`) sits at arc
    /// length `s_i = L·i/(count − 1)` on an open path, or `L·i/count` on a
    /// closed one. Its placement is `F_i ∘ S(σ_i) ∘ F_0⁻¹`: `F_s` is the
    /// path frame at `s` (origin on the path, columns tangent, a
    /// parallel-transported normal, and their cross product; just a
    /// translation when `!align`), `S` a uniform scale about the frame origin,
    /// and `σ_i = scale_from + (scale_to − scale_from)·(i − 1)/max(1,
    /// count − 2)`. Every placement is a similarity, so object claims map and
    /// instance poses stay similarities. One undo step
    /// ([`DocAction::DuplicatedArray`]).
    ///
    /// Returns the clone roots in creation order (every source's copy 1, then
    /// copy 2, …) plus what changed.
    ///
    /// # Errors
    /// - [`ShapeError::AmountOutOfRange`] — `count` is below 2 or above
    ///   [`MAX_ARRAY_COUNT`], or a scale is not a finite positive number.
    /// - [`ShapeError::PathTooShortForCount`] — the path is no longer than
    ///   `POINT_MERGE`.
    /// - Path resolution errors as for Follow Me, and the errors of
    ///   [`Document::duplicate_nodes_array`].
    ///
    /// On `Err` the document is untouched.
    pub fn array_along_path(
        &mut self,
        nodes: &[NodeId],
        path: &FollowMePath,
        spec: &PathArraySpec,
    ) -> Result<(Vec<NodeId>, DocChange), DocumentError> {
        info!(
            target: "kernel::op",
            op = "array_along_path",
            nodes = nodes.len(),
            count = spec.count,
        );
        let placements = self.path_array_placements(path, spec)?;
        self.duplicate_nodes_placements(nodes, &placements, LinkPolicy::Independent)
    }
}

// ==== true-curves: imports ====

impl Document {
    /// Softens `object`'s edges that bend less than `max_angle` radians
    /// ([`Object::soften_by_angle`]) as one undo step ("Soften Edges"),
    /// returning how many changed. Softening nothing changes nothing and
    /// records nothing.
    ///
    /// # Errors
    /// `Shape(AmountOutOfRange)` unless `0 < max_angle < π`; the reshape
    /// gate's.
    pub fn soften_by_angle(
        &mut self,
        object: ObjectId,
        max_angle: f64,
    ) -> Result<(usize, DocChange), DocumentError> {
        self.reshape_target(object)?;
        let (_, n) = self.objects[object]
            .object
            .soften_by_angle(max_angle)
            .map_err(DocumentError::Shape)?;
        if n == 0 {
            return Ok((0, DocChange::default()));
        }
        let (_, change) = self.reshape(object, ReshapeSpec::SoftenByAngle { max_angle })?;
        Ok((n, change))
    }
}

// ==== end true-curves: imports ====
