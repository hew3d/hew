//! `openskp::Model` -> `kernel::ImportScene`.
//!
//! Mapping:
//! - a node's [`openskp::Node::is_group`] tells apart SketchUp's two
//!   placement classes, which Hew maps very differently:
//!   - `Some(false)` (a `CComponentInstance`) OR `None` (the legacy
//!     byte-scan path, which cannot tell them apart — treated as a
//!     component for backward compatibility): each **definition geometry
//!     run** becomes a [`DefRecipe`] (one variant per inherited material,
//!     since SketchUp's default-material faces render with the placing
//!     instance's paint — Hew expresses that as the def mesh's
//!     `base_material`), and the placement becomes an [`ImportNode::Instance`]
//!     — a definition's in-definition child placements are NESTED member
//!     instances of its `DefRecipe` (poses rebased to the definition's
//!     local frame), so a SketchUp assembly's sharing survives: N
//!     placements stay N instances of one definition;
//!   - `Some(true)` (a `CGroup`): SketchUp groups are independent
//!     containers, not shared definitions — Hew has no equivalent of "one
//!     group definition, many placements" (that IS what a component is).
//!     Each placement converts (and re-heals) independently: the node's own
//!     run meshes bake the node's pose into their positions (world pose at
//!     the world level; rebased into the enclosing definition's local frame
//!     when nested inside one, since a Hew `ImportNode::Mesh` carries no
//!     pose of its own) and children convert recursively (components become
//!     Instances, nested groups become Groups the same way), producing an
//!     [`ImportNode::Group`]. A group with exactly one resulting mesh and no
//!     children collapses to a bare [`ImportNode::Mesh`] instead — SketchUp
//!     needs wrapper groups to isolate geometry; Hew's solids-first model
//!     doesn't — unless the group itself is hidden (a bare mesh has nowhere
//!     to carry that).
//! - pure containers without geometry runs become [`ImportNode::Group`]s
//!   with **absolute world poses** exactly like `dae-import`;
//! - a definition of drawing alone (a wire-frame component, a light rig of
//!   edges) opens as a definition of its sketches, placed as the file
//!   places it; a loop its edges close is a loop with no face, as in
//!   SketchUp (the kernel's sketch rule 6);
//! - the **root run** (loose model geometry) becomes a world
//!   [`ImportNode::Mesh`];
//! - layers map to tags — the FULL layer list (empty and hidden layers
//!   included) is declared as [`ImportTag`]s so the document keeps it, with
//!   hidden layers becoming hidden-by-default tags;
//! - hidden-layer CONTENT imports (tagged with its hidden tag, hidden by
//!   default in the UI, never dropped — a layer is visibility state, not
//!   existence); hidden *instances* (per-entity hide) still drop, matching
//!   SketchUp's own exports, and faces always import — hidden faces are
//!   display state, and dropping them would open solids;
//! - guides map to [`ImportGuide`]s: a guide among the model's own
//!   entities as it is, a guide inside a definition once per placement of
//!   that definition, carried through the placement's world pose (that is
//!   where SketchUp draws it), tagged with its own layer and then each
//!   container's above it, so it shows and hides with the layers the way
//!   SketchUp shows it scene by scene; one hidden with Edit > Hide, or
//!   inside a placement hidden that way, stays out, as a Hew guide has no
//!   hidden state to carry it in;
//! - dimensions and labels become annotations: the model's own as they
//!   are, one inside a group or component placed once through that
//!   placement's world pose, tagged with its own layer and then each
//!   container's above it (SketchUp shows it only while the group shows);
//!   one under a hidden container, or in a definition placed more than
//!   once, stays out (the file keeps it).
//!
//! Both containers OpenSKP reads (the 2026 ZIP container every current
//! SketchUp saves, and the 2017 `CArchive` stream) come out as the same
//! `Model`, so nothing here is container-specific. Everything OpenSKP hands
//! us is already metres; poses come from the composed row-major world
//! matrices, whose first 12 entries are exactly `Transform::from_affine`'s
//! layout.
//!
//! Importer keys: scene hidden sets and active section cuts name entities
//! by SketchUp **persistent id**, the one identity both containers carry.
//! The composed [`openskp::Node`] tree does not carry pids itself, but it is
//! built in the exact order of the reader's instance lists — top-level
//! nodes from `Model::instances` outside every geometry run, children from
//! the placing run's `GeometryRun::placed` — so the converter walks those
//! lists in step with the tree to recover each node's pid. A pid of 0 (the
//! 2017 legacy byte-scan path, which cannot resolve them) yields no key.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use kernel::{
    AttrDict, CameraProjection, CameraState, CurveAnalytic, CurveGeom, DefRecipe, ImportAttrTarget,
    ImportGuide, ImportNode, ImportScene, ImportSceneExtra, ImportSceneRecipe, ImportSketch,
    ImportTag, MeshExtra, MeshRecipe, Plane, Point3, SectionPlaneState, SketchCurveKind,
    SurfaceRef, Transform, UvFrame, Vec3,
};
use mesh_heal::uv::fit_uv_frame;
use mesh_heal::{heal_mesh_as_written, heal_mesh_keeping_faces};
use openskp::{Owner, Value};

use crate::extras::{self, Dicts, HewModel, Mapped};
use crate::material::MatTable;
use crate::{AnnotationOrigin, Origin, Provenance};

/// Importer keys past every persistent id: the keys of meshes no placement
/// names (a definition's own geometry, the model's loose geometry, a
/// group's own geometry beside its children).
const SYNTHETIC_KEYS: u64 = 1 << 40;

/// What the `hew` dictionary says about one placement.
#[derive(Default, Clone)]
struct PlacementHew {
    unnamed: bool,
    tags: Option<Vec<Vec<String>>>,
}

pub(crate) struct Output {
    pub scene: ImportScene,
    pub textures_missing: Vec<String>,
    pub warnings: Vec<String>,
    pub provenance: Provenance,
}

#[cfg(test)]
pub(crate) fn convert(model: Rc<openskp::Model>) -> Output {
    let mut job = ConvertJob::begin(model);
    job.step(usize::MAX);
    job.finish()
}

/// A conversion under way: the runs of loose geometry and the top-level
/// placements still to read, a few at a time ([`ConvertJob::step`]) so an
/// open can say how far it has come; [`ConvertJob::finish`] reads the
/// rest (images, guides, tags, scenes, annotations) and hands the recipe
/// over.
pub(crate) struct ConvertJob {
    cv: Converter,
    def_indexes: BTreeSet<usize>,
    roots: Vec<ImportNode>,
    tree: Vec<openskp::Node>,
    top_level: Vec<(Option<String>, u32)>,
    /// Runs of loose world geometry, in file order.
    loose: Vec<usize>,
    next_loose: usize,
    next_top: usize,
}

impl ConvertJob {
    pub(crate) fn begin(model: Rc<openskp::Model>) -> ConvertJob {
        let mats = MatTable::build(&model);

        // Declared archive-map indexes that belong to component/group definitions;
        // geometry runs carrying one of these are definition runs, everything else
        // is loose world geometry.
        let def_indexes: BTreeSet<usize> = model
            .definition_links
            .iter()
            .map(|&(dr, _)| dr as usize)
            .collect();

        // Every persistent id some scene hides: such a group must stay a Group
        // node (a bare Mesh carries no key for the scene's hidden set to name).
        let scene_hidden: BTreeSet<u32> = model
            .scenes
            .iter()
            .flat_map(|s| s.hidden_entities.iter().copied())
            .collect();

        let dicts = Dicts::of(&model);
        let hew = HewModel::of(&dicts);
        // Every placement's own name and Hew dictionary, by persistent id.
        let names: BTreeMap<u32, String> = model
            .instances
            .iter()
            .filter_map(|i| Some((i.pid, i.name.clone().filter(|n| !n.is_empty())?)))
            .collect();
        let mut placement_hew: BTreeMap<u32, PlacementHew> = BTreeMap::new();
        if hew.written_by_hew {
            for (ri, run) in model.geometry.iter().enumerate() {
                for (pi, p) in run.placed.iter().enumerate() {
                    let Some(d) = dicts.hew(Owner::Instance(ri, pi)) else {
                        continue;
                    };
                    let tags = match extras::entry(d, "tags") {
                        Some(Value::Array(paths)) => {
                            Some(paths.iter().filter_map(extras::path_of).collect())
                        }
                        _ => None,
                    };
                    placement_hew.insert(
                        p.pid,
                        PlacementHew {
                            unnamed: extras::entry(d, "unnamed") == Some(&Value::Bool(true)),
                            tags,
                        },
                    );
                }
            }
        }

        let cv = Converter {
            model: model.clone(),
            mats,
            scene_hidden,
            def_variants: BTreeMap::new(),
            degraded_variants: BTreeMap::new(),
            image_defs: BTreeMap::new(),
            def_sketch_frames: Vec::new(),
            hidden_lines_left: BTreeSet::new(),
            unplaceable_lines: 0,
            image_warned: BTreeSet::new(),
            defs: Vec::new(),
            warnings: Vec::new(),
            dicts,
            hew,
            names,
            placement_hew,
            next_key: SYNTHETIC_KEYS,
            sketches: Vec::new(),
            run_variants: BTreeMap::new(),
            def_frames: Vec::new(),
            provenance: Provenance::default(),
            claims: None,
        };

        // ── The composed instance tree ─────────────────────────────────────────
        // `scene()` maps the top-level instances in filter order; zip against the
        // same filter to recover each root placement's own name (nested placements
        // have no names — they fall back to their def's name) and its persistent
        // id (the importer key; see the module docs).
        let top_level: Vec<(Option<String>, u32)> = model
            .instances
            .iter()
            .filter(|i| {
                !model
                    .geometry
                    .iter()
                    .any(|r| r.start <= i.offset && i.offset < r.end)
            })
            .map(|i| (i.name.clone().filter(|n| !n.is_empty()), i.pid))
            .collect();
        let tree = model.scene();
        debug_assert_eq!(tree.len(), top_level.len());
        let loose: Vec<usize> = (0..model.geometry.len())
        .filter(|&ri| !matches!(model.geometry[ri].def_index, Some(di) if def_indexes.contains(&di)))
        .collect();
        ConvertJob {
            cv,
            def_indexes,
            roots: Vec::new(),
            tree,
            top_level,
            loose,
            next_loose: 0,
            next_top: 0,
        }
    }

    /// Items read so far, out of all of them: the runs of loose geometry
    /// and the top-level placements.
    pub(crate) fn progress(&self) -> (usize, usize) {
        (
            self.next_loose + self.next_top,
            self.loose.len() + self.top_level.len(),
        )
    }

    /// Definition recipes built so far.
    pub(crate) fn shapes(&self) -> usize {
        self.cv.defs.len()
    }

    /// Read up to `items` more; true once everything is read.
    pub(crate) fn step(&mut self, items: usize) -> bool {
        let mut done = 0usize;
        let model = self.cv.model.clone();
        while done < items && self.next_loose < self.loose.len() {
            let ri = self.loose[self.next_loose];
            self.next_loose += 1;
            let run = &model.geometry[ri];
            // A file Hew wrote says when its loose geometry is drawn sketches.
            if self.cv.hew.loose_sketches {
                let planes = self.cv.sketch_planes(Owner::Model);
                let sketches = self.cv.run_sketches(ri, &planes, None);
                self.cv.sketches.extend(sketches);
                done += 1;
                continue;
            }
            let key = self.cv.synthetic_key();
            self.cv.provenance.nodes.push((key, Origin::Loose));
            // Loose edges that bound no face are drafting lines; a save back
            // keeps them as the file's while they are unchanged.
            let lines = self.cv.free_edge_sketches(ri, &Transform::IDENTITY, None);
            let from = self.cv.sketches.len();
            self.cv.sketches.extend(lines);
            self.cv
                .provenance
                .loose_sketches
                .extend(from..self.cv.sketches.len());
            // Loose geometry has no placement to carry a layer or a hidden
            // flag, so its faces carry them: each connected piece on one layer
            // is a mesh with that layer's tag, and a piece whose every face is
            // hidden a hidden mesh (a run of one visible kind is read whole).
            match loose_pieces(&run.mesh) {
                Some(parts) => {
                    for ((slot, hidden), faces) in parts {
                        let tags = slot.map(|s| self.cv.layer_tags(s)).unwrap_or_default();
                        for recipe in self.cv.mesh_recipes_of(
                            ri,
                            "Model".to_string(),
                            0,
                            &Transform::IDENTITY,
                            Some(key),
                            Some(LoosePart {
                                faces: &faces,
                                tags,
                                hidden,
                            }),
                        ) {
                            self.roots.push(ImportNode::Mesh(recipe));
                        }
                    }
                }
                None => {
                    for recipe in self.cv.mesh_recipes(
                        ri,
                        "Model".to_string(),
                        0,
                        &Transform::IDENTITY,
                        Some(key),
                    ) {
                        self.roots.push(ImportNode::Mesh(recipe));
                    }
                }
            }
            done += 1;
        }
        while done < items && self.next_top < self.top_level.len() {
            let i = self.next_top;
            self.next_top += 1;
            let (own_name, pid) = self.top_level[i].clone();
            if let Some(n) = self.cv.convert_node(&self.tree[i], own_name, pid) {
                self.roots.push(n);
            }
            done += 1;
        }
        let (read, all) = self.progress();
        read >= all
    }

    /// Read the rest and hand the recipe over.
    pub(crate) fn finish(mut self) -> Output {
        self.step(usize::MAX);
        let ConvertJob {
            mut cv,
            def_indexes,
            mut roots,
            tree,
            ..
        } = self;
        let model = cv.model.clone();
        let model: &openskp::Model = &model;

        // ── Placed images ──────────────────────────────────────────────────────
        // An image among the model's own entities is a placement of its image
        // definition, whose one face is painted with the picture: it comes in
        // as an Instance of that definition, keyed by its persistent id (a
        // scene hiding it names it). Images inside groups and components come
        // in with them (`image_children`).
        for ri in 0..model.geometry.len() {
            let is_def =
                matches!(model.geometry[ri].def_index, Some(di) if def_indexes.contains(&di));
            if !is_def {
                roots.extend(cv.image_children(ri, None));
            }
        }

        // ── Guides ─────────────────────────────────────────────────────────────
        let guides = world_guides(model, &tree, &|slot| cv.layer_tags(slot));

        if cv.unplaceable_lines > 0 {
            cv.warnings.push(format!(
                "{} line{} with coordinates that are not numbers {} left out",
                cv.unplaceable_lines,
                if cv.unplaceable_lines == 1 { "" } else { "s" },
                if cv.unplaceable_lines == 1 {
                    "was"
                } else {
                    "were"
                }
            ));
        }
        let hidden_lines = cv.hidden_lines_left.len();
        if hidden_lines > 0 {
            cv.warnings.push(format!(
            "{} hidden line{} {} left out (SketchUp shows them only with hidden geometry on); a save back over the file keeps {}",
            hidden_lines,
            if hidden_lines == 1 { "" } else { "s" },
            if hidden_lines == 1 { "was" } else { "were" },
            if hidden_lines == 1 { "it" } else { "them" },
        ));
        }

        // ── Parse anomalies -> user-visible warnings ───────────────────────────
        // Clean files parse with zero desync diagnostics on either container (an
        // OpenSKP regression guarantee); anything else means content may be
        // missing and is said out loud, never papered over (rule 4 spirit; fixes
        // go upstream). Converter-emitted warnings (non-manifold splits) come
        // first — they are per-object and actionable. Reader notes AGGREGATE
        // past a handful: a badly damaged file can desync tens of thousands of
        // times, and 32k warning strings help nobody (a damaged production model
        // produced 32k).
        const DESYNC_DETAIL_CAP: usize = 8;
        let mut warnings = std::mem::take(&mut cv.warnings);
        let desyncs: Vec<&openskp::Diagnostic> =
            model.diagnostics.iter().filter(|d| d.is_desync()).collect();
        warnings.extend(desyncs.iter().take(DESYNC_DETAIL_CAP).map(|d| match d {
            // A 2026 record the reader skipped: damaged, or a tag it does not
            // know (what a newer release's addition looks like).
            openskp::Diagnostic::Skipped { class, at, bytes } => format!(
                "reader skipped a record it could not decode ({class}, {bytes} \
             bytes at 0x{at:x}); content from it may be missing"
            ),
            other => format!("parser recovered from a malformed section: {other:?}"),
        }));
        if desyncs.len() > DESYNC_DETAIL_CAP {
            warnings.push(format!(
                "parser recovered from {} more malformed sections (content from \
             them may be missing)",
                desyncs.len() - DESYNC_DETAIL_CAP
            ));
        }

        // ── The declared tag list: every named non-default layer ──────────────
        // Hidden layers become hidden-by-default tags; empty layers survive too
        // (the document keeps the full source layer list).
        let tags: Vec<ImportTag> = model
            .layers
            .iter()
            .enumerate()
            // The list-first layer is the default (Layer0) — it maps to
            // "untagged", exactly as `layer_tags` treats slot 0.
            .filter(|(i, l)| *i > 0 && !l.name.is_empty())
            .map(|(_, l)| ImportTag {
                path: cv.tag_path(&l.name),
                hidden: !l.visible,
            })
            .collect();
        for l in model.layers.iter().skip(1).filter(|l| !l.name.is_empty()) {
            cv.provenance
                .tags
                .push((cv.tag_path(&l.name), l.name.clone()));
        }

        // ── Saved view + scenes ────────────────────────────────────────────────
        // The document camera and every page: name, camera, and the hidden
        // entity / hidden layer sets as node keys (persistent ids) and tag
        // paths. Ids that name faces or edges carry no node and drop out in
        // `ingest`; the default layer (list-first) has no tag to hide.
        let camera = model
            .camera
            .as_ref()
            .map(|c| camera_state(c, cv.hew.written_by_hew));
        let tag_paths: BTreeSet<&Vec<String>> = tags.iter().map(|t| &t.path).collect();
        // Section planes live in their owning entity list's local frame; a
        // scene's active-section id resolves through the run holding it and
        // that run's FIRST placement's world pose (the model root for loose
        // planes). A plane inside a multiply placed definition is ambiguous in
        // SketchUp too; the first placement is the documented choice.
        let section_world = section_planes_world(model, &tree);
        let saves_section = |s: &openskp::Scene| -> bool {
            s.saved.has(openskp::SceneProperties::ACTIVE_SECTION_PLANES)
        };
        let scene_section = |s: &openskp::Scene| -> Option<SectionPlaneState> {
            s.active_section_planes
                .iter()
                .find_map(|pid| section_world.get(pid).copied())
        };
        let scenes: Vec<ImportSceneRecipe> = model
            .scenes
            .iter()
            .map(|s| ImportSceneRecipe {
                name: s.name.clone(),
                description: s.description.clone(),
                camera: s
                    .camera
                    .as_ref()
                    .map(|c| camera_state(c, cv.hew.written_by_hew)),
                hidden_keys: Some(s.hidden_entities.iter().map(|&k| k as u64).collect()),
                hidden_tags: Some(
                    s.hidden_layers
                        .iter()
                        .filter(|&&i| i != 0)
                        .filter_map(|&i| model.layers.get(i))
                        .map(|l| cv.tag_path(&l.name))
                        .filter(|p| tag_paths.contains(p))
                        .collect(),
                ),
                // A scene captures its cut only when it saved the section-planes
                // property; a scene that did not leaves the cut alone (`None`),
                // and one that did with nothing active captures "no cut".
                section: saves_section(s).then(|| scene_section(s)),
                display: cv.hew.scene_display.get(&s.name).copied(),
                // The scene's saved style: its backdrop and axes.
                view_style: s
                    .rendering
                    .as_ref()
                    .filter(|_| s.saved.has(openskp::SceneProperties::STYLE_AND_FOG))
                    .map(view_style_of),
            })
            .collect();
        // The scene active at save time is the one whose camera the saved view
        // repeats exactly; its section cut is what SketchUp showed, so the
        // document adopts it (the hidden state already lives in the entities'
        // and layers' own flags) — provided the scene saved the section-planes
        // property at all; a scene that did not leaves the model's own cut in
        // force. With no such scene the model's own state decides: the root
        // entity list's active plane cuts (both containers record which plane
        // that is), and failing that — a file with planes but none active, or
        // the legacy byte-scan path, which resolves no ids — the first plane
        // placed at the root arrives placed but not cutting, ready to switch
        // on. Hew has one section plane, so planes inside definitions (which
        // cut only their own contents) are not adopted.
        let saved_scene = model.camera.as_ref().and_then(|saved| {
            model
                .scenes
                .iter()
                .find(|s| s.camera.as_ref().is_some_and(|c| c == saved))
        });
        let root_run = model.geometry.iter().find(|r| r.def_index.is_none());
        let section = match saved_scene {
            Some(s) if saves_section(s) => scene_section(s),
            _ => root_run.and_then(|root| {
                root.active_section
                    .and_then(|pid| section_world.get(&pid).copied())
                    .or_else(|| {
                        root.sections.iter().find_map(|sp| {
                            section_world.get(&sp.pid).map(|s| SectionPlaneState {
                                active: false,
                                ..*s
                            })
                        })
                    })
            }),
        };

        // How the model is shown: its display settings' backdrop and axes
        // (`kernel::ViewStyle`). A Hew-written file says whether those came
        // from a view style at all; a document without one keeps Hew's look.
        let view_style = model
            .rendering
            .as_ref()
            .filter(|_| !cv.hew.written_by_hew || cv.hew.view_style)
            .map(view_style_of);

        let attrs = cv.attributes();
        let (mut annotations, mut annotation_tags) = cv.hew_annotations();
        let places = Placements::of(&cv, &tree, model);
        let (theirs, left) = cv.sketchup_annotations(&places);
        for (a, tags, origin) in theirs {
            cv.provenance.annotations.push((annotations.len(), origin));
            annotations.push(a);
            annotation_tags.push(tags);
        }
        if left > 0 {
            warnings.push(format!(
                "{left} of SketchUp's dimensions and texts are attached in a way Hew cannot \
             place yet and were left out; a save back over the file keeps them"
            ));
        }
        let Converter {
            mats,
            defs,
            sketches,
            provenance,
            ..
        } = cv;
        Output {
            scene: ImportScene {
                materials: mats.materials,
                defs,
                roots,
                guides,
                tags,
                camera,
                scenes,
                section,
                view_style,
                extra: ImportSceneExtra {
                    sketches,
                    attrs,
                    annotations,
                    annotation_tags,
                },
            },
            textures_missing: mats.textures_missing,
            warnings,
            provenance,
        }
    }
}

struct Converter {
    model: Rc<openskp::Model>,
    mats: MatTable,
    /// Persistent ids named by any scene's hidden-entity list.
    scene_hidden: BTreeSet<u32>,
    /// (geometry-run index, inherited material slot) -> def index in `defs`
    /// (`None` = the run has no importable faces at that variant).
    def_variants: BTreeMap<(usize, u16), Option<usize>>,
    /// Variants whose nested children were DROPPED because the first
    /// placement's world matrix was singular (uninvertible — the def-local
    /// rebase is impossible from that placement). A later placement with an
    /// invertible matrix upgrades the cached recipe in place, so one
    /// zero-scaled placement can't permanently strip a shared definition.
    degraded_variants: BTreeMap<(usize, u16), usize>,
    /// Image definition (by map index) -> its recipe in `defs`, built on
    /// first use (`None` when it holds no face to show the picture on).
    image_defs: BTreeMap<u32, Option<usize>>,
    /// The sketches of the definitions being built, innermost last: a
    /// group's drawing inside a definition joins the definition once its
    /// index is known (`offer_sketches`).
    def_sketch_frames: Vec<Vec<ImportSketch>>,
    /// Hidden edges bounding no face, left out (SketchUp shows them only
    /// with hidden geometry on, and a drafting line has no hidden state),
    /// by list and edge: each counted once however many times its list
    /// is built.
    hidden_lines_left: BTreeSet<(usize, usize)>,
    /// Edges left out because a coordinate of theirs is not a number (a
    /// run the reader misread), counted for the open report.
    unplaceable_lines: usize,
    /// Images already reported as having no picture, by placement id.
    image_warned: BTreeSet<u32>,
    defs: Vec<DefRecipe>,
    /// User-visible conversion warnings (non-manifold splits — rule 4:
    /// decomposition happens loudly, never silently).
    warnings: Vec<String>,
    dicts: Dicts,
    hew: HewModel,
    /// Every placement's own name, by persistent id.
    names: BTreeMap<u32, String>,
    /// What a Hew-written file says about each placement, by pid.
    placement_hew: BTreeMap<u32, PlacementHew>,
    next_key: u64,
    /// Sketches a Hew-written file's loose geometry was drawn as.
    sketches: Vec<ImportSketch>,
    /// Geometry run -> the definition recipes built from it (one per
    /// inherited-material variant).
    run_variants: BTreeMap<usize, Vec<usize>>,
    provenance: Provenance,
    /// The claims of the run last read, under its bake, for the pieces
    /// still to come from it (see [`RunClaims`]).
    claims: Option<RunClaims>,
    /// While a definition's children convert: the inverse of the
    /// definition placement's world pose, innermost last. A group's own
    /// geometry inside one bakes straight into the definition's frame.
    def_frames: Vec<Transform>,
}

impl Converter {
    /// A node's children paired with their persistent ids: the composed
    /// tree lists a node's children in exactly the order of its run's
    /// `placed` list (see the module docs), and a node without a run has
    /// no children.
    fn children_with_pids<'n>(
        &self,
        node: &'n openskp::Node,
    ) -> impl Iterator<Item = (&'n openskp::Node, u32)> + use<'_, 'n> {
        let pids = node
            .run
            .map(|ri| self.model.geometry[ri].placed.as_slice())
            .unwrap_or(&[]);
        debug_assert!(node.children.is_empty() || node.children.len() == pids.len());
        node.children.iter().zip(pids.iter().map(|p| p.pid))
    }

    /// Every child of `node` converted (pruned children drop out).
    fn convert_children(&mut self, node: &openskp::Node) -> Vec<ImportNode> {
        let pairs: Vec<(&openskp::Node, u32)> = self.children_with_pids(node).collect();
        pairs
            .into_iter()
            .filter_map(|(c, pid)| {
                let name = self.names.get(&pid).cloned();
                self.convert_node(c, name, pid)
            })
            .collect()
    }

    /// One composed scene node -> an import node, `None` when pruned
    /// (hidden instance, hidden layer, or nothing importable below).
    /// `pid` is the placing object's persistent id (0 = unknown).
    fn convert_node(
        &mut self,
        node: &openskp::Node,
        own_name: Option<String>,
        pid: u32,
    ) -> Option<ImportNode> {
        // NOTHING visibility-related is dropped: a hidden INSTANCE
        // (per-entity hide) imports as a user-hidden node (persisted view
        // state, manifest v6), and hidden-LAYER content imports carrying
        // its layer tag whose hidden-by-default flag keeps it invisible
        // until the user shows the tag. Hidden is visibility state, not
        // existence.
        let hidden = node.hidden;

        let pose = pose_of(&node.world);
        let mut tags = self.layer_tags(node.layer);
        // A Hew-written placement says when the node had no name (its
        // definition's name is SketchUp's own then) and when it had more
        // tags than the one layer holds.
        let mut own_name = own_name;
        if let Some(h) = self.placement_hew.get(&pid).cloned() {
            if h.unnamed {
                own_name = Some(String::new());
            }
            if let Some(t) = h.tags {
                tags = t;
            }
        }

        match node.run {
            // A SketchUp GROUP placement (`is_group == Some(true)`): groups
            // are independent containers, not shared definitions — each
            // placement converts on its own (deep copy, no shared def).
            Some(ri) if matches!(node.is_group, Some(true)) => {
                self.group_node(ri, node, own_name, tags, hidden, pid)
            }
            // A placement of a definition — with or without in-definition
            // child placements. The whole subtree is ONE nested definition
            // (children become member instances with def-local poses), so
            // N placements of a SketchUp assembly stay N instances of one
            // shared definition — the nested-component parity fix.
            // `is_group == Some(false)` (component) or `None` (legacy
            // byte-scan path, which cannot tell components and groups
            // apart — kept on this path for backward compatibility).
            Some(ri) => {
                let def = self.assembly_variant(ri, node.material, node)?;
                self.origin(pid, |pid| Origin::Instance { pid });
                Some(ImportNode::Instance {
                    def,
                    pose,
                    name: own_name.filter(|n| !n.is_empty()),
                    tags,
                    hidden,
                    key: key_of(pid),
                })
            }
            // A pure container with no geometry run of its own: a world
            // group wrapping its converted children (world poses).
            None => {
                let children = self.convert_children(node);
                if children.is_empty() {
                    return None;
                }
                let name = own_name
                    .or_else(|| node.definition.clone())
                    .unwrap_or_default();
                self.origin(pid, |pid| Origin::Group { pid });
                Some(ImportNode::Group {
                    name,
                    children,
                    tags,
                    hidden,
                    key: key_of(pid),
                })
            }
        }
    }

    /// A SketchUp GROUP node with a geometry run: converts to a Hew `Group`
    /// (own meshes + recursively converted children) or, in the common
    /// "wrapper around one solid" case, a bare `Mesh` — never a shared
    /// `DefRecipe`, since SketchUp groups are logically unique (unlike
    /// components, a group is not "one definition, many placements").
    ///
    /// The node's own run bakes its pose into its mesh positions (group
    /// geometry is def-local in the source; a Hew `ImportNode::Mesh`
    /// carries no pose): `pose_of(&node.world)` at the top, and inside a
    /// component definition that pose composed with the definition
    /// placement's inverse, landing the positions in def-local coordinates
    /// in one step.
    fn group_node(
        &mut self,
        run_idx: usize,
        node: &openskp::Node,
        own_name: Option<String>,
        tags: Vec<Vec<String>>,
        hidden: bool,
        pid: u32,
    ) -> Option<ImportNode> {
        let name = own_name
            .or_else(|| node.definition.clone())
            .unwrap_or_default();
        // World geometry for a group at the top; inside a definition, the
        // definition's frame (world, then the definition placement's
        // inverse) — one transform, so a circle claim survives whenever the
        // group's own placement is a similarity, however the definition
        // itself is placed.
        let bake = match self.def_frames.last() {
            Some(inv) => pose_of(&node.world).then(inv),
            None => pose_of(&node.world),
        };
        let mut children = self.convert_children(node);
        // The group's own images come in with its children, at their
        // world poses.
        children.extend(self.image_children(run_idx, Some(&pose_of(&node.world))));
        // The group's own drawing: edges bounding no face, as sketches the
        // group holds (by its key; a placement without an id leaves them
        // free drawing).
        let sketches = self.run_drawing(run_idx, &bake, key_of(pid));
        // Alone, the group's geometry stands for the group and takes its
        // key; beside children it is geometry of its own.
        let own_key = if children.is_empty() {
            self.origin(pid, |pid| Origin::GroupGeometry { pid });
            key_of(pid)
        } else {
            let k = self.synthetic_key();
            self.provenance
                .nodes
                .push((k, Origin::GroupOwnGeometry { pid }));
            Some(k)
        };
        // A Hew-written group whose list is drawn sketches: its faces are
        // the regions those sketches close, not a solid.
        let mut own_meshes = if self.list_is_sketches(run_idx) {
            Vec::new()
        } else {
            self.mesh_recipes(run_idx, name.clone(), node.material, &bake, own_key)
        };

        // The "simple solid" special case: SketchUp needs a wrapper group to
        // isolate geometry; Hew's solids-first model doesn't. The mesh
        // carries the group's hidden flag and its key (so a scene that hides
        // the group hides it) — except where the placement has no id for
        // the key, when a hidden or scene-hidden group stays a group rather
        // than lose that state quietly (rule 4 spirit).
        let scene_hidden = pid != 0 && self.scene_hidden.contains(&pid);
        if children.is_empty()
            && sketches.is_empty()
            && own_meshes.len() == 1
            && (own_key.is_some() || (!hidden && !scene_hidden))
        {
            let mut mesh = own_meshes.pop().expect("len checked above");
            mesh.tags = tags;
            mesh.extra.hidden = hidden;
            return Some(ImportNode::Mesh(mesh));
        }
        // Several meshes (a non-manifold split) under one group: the group
        // takes the placement's key, its meshes one of their own.
        if children.is_empty() && own_meshes.len() > 1 {
            let k = self.synthetic_key();
            self.provenance
                .nodes
                .push((k, Origin::GroupOwnGeometry { pid }));
            for m in &mut own_meshes {
                m.extra.key = Some(k);
            }
        }

        let mut all_children: Vec<ImportNode> =
            own_meshes.into_iter().map(ImportNode::Mesh).collect();
        all_children.extend(children);
        if all_children.is_empty() && sketches.is_empty() {
            return None;
        }
        // A group of lines alone is a group with no members holding its
        // drawing.
        self.offer_sketches(sketches);
        self.origin(pid, |pid| Origin::Group { pid });
        Some(ImportNode::Group {
            name,
            children: all_children,
            tags,
            hidden,
            key: key_of(pid),
        })
    }

    /// A run's drawing, as sketches held by `group` (a key) — `None` for
    /// free drawing — in the frame `bake` carries the run into. A
    /// Hew-written list comes back as the sketches it was drawn as; any
    /// other list's edges bounding no face are fitted to planes.
    fn run_drawing(
        &mut self,
        run_idx: usize,
        bake: &Transform,
        group: Option<u64>,
    ) -> Vec<ImportSketch> {
        let planes = if self.hew.written_by_hew {
            self.def_owner(run_idx)
                .map(|o| self.sketch_planes(o))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // (A list Hew carried as the file's own records no planes, and is
        // read like any SketchUp list.)
        let mut out = if planes.is_empty() {
            self.free_edge_sketches(run_idx, bake, None)
        } else {
            let mut sketches = self.run_sketches(run_idx, &planes, None);
            if sketches.iter_mut().all(|s| bake_sketch(s, bake)) {
                sketches
            } else {
                // (A placement with no inverse: fitted from the edges.)
                self.free_edge_sketches(run_idx, bake, None)
            }
        };
        for s in &mut out {
            s.group = group;
        }
        out
    }

    /// Sketches made while a definition is being built join that
    /// definition once its index is known; outside one they are free.
    fn offer_sketches(&mut self, sketches: Vec<ImportSketch>) {
        match self.def_sketch_frames.last_mut() {
            Some(frame) => frame.extend(sketches),
            None => self.sketches.extend(sketches),
        }
    }

    /// A run's edges that bound no face, as drafting sketches. A welded
    /// curve's members go together onto the plane fitted through them (a
    /// circle on a roof slope is one sketch, not a chord per vertical
    /// plane); every other edge joins the first plane holding both its
    /// ends, else starts one of its own (an axis plane where one holds the
    /// edge, else the vertical plane through it). Ends are set onto their
    /// plane ([`DRAWING_ON_PLANE`]); an arc's circle claim rides along
    /// when its center is on the plane too. An edge's layer becomes its
    /// sketch's tag, so edges on different layers go to different sketches
    /// of the same plane (a welded curve takes its first edge's layer).
    /// Hidden edges are left out and counted (`hidden_lines_left`).
    /// Coordinates carried through `bake`.
    fn free_edge_sketches(
        &mut self,
        run_idx: usize,
        bake: &Transform,
        group: Option<u64>,
    ) -> Vec<ImportSketch> {
        let run = &self.model.geometry[run_idx];
        let mesh = &run.mesh;
        let mut on_face: BTreeSet<(u32, u32)> = BTreeSet::new();
        for f in &mesh.faces {
            for ring in std::iter::once(&f.outer).chain(f.holes.iter()) {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    on_face.insert((a.min(b), a.max(b)));
                }
            }
        }
        let at = |i: u32| {
            let v = mesh.vertices[i as usize];
            bake.apply_point(Point3::new(v[0], v[1], v[2]))
        };
        // The free edges, each with its welded curve and its layer.
        let mut free: Vec<([Point3; 2], Option<u32>, u16)> = Vec::new();
        let mut hidden: Vec<(usize, usize)> = Vec::new();
        for (ei, e) in mesh.edges.iter().enumerate() {
            if on_face.contains(&(e.v0.min(e.v1), e.v0.max(e.v1))) {
                continue;
            }
            if e.hidden {
                hidden.push((run_idx, ei));
                continue;
            }
            let (p, q) = (at(e.v0), at(e.v1));
            // A coordinate that is not a number comes from a stray run the
            // reader misread, never from SketchUp: such an edge has no
            // place to be, and is counted rather than drawn as nothing.
            if ![p, q]
                .iter()
                .all(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite())
            {
                self.unplaceable_lines += 1;
                continue;
            }
            if p.approx_eq(q, 1e-9) {
                continue;
            }
            free.push(([p, q], e.curve, e.layer));
        }
        let mut out: Vec<ImportSketch> = Vec::new();
        // Each sketch's layer, parallel to `out`: a sketch holds one
        // layer's edges, so the layer can be its tag.
        let mut layers: Vec<u16> = Vec::new();
        let mut chains: BTreeMap<(usize, u32), Vec<usize>> = BTreeMap::new();
        let mut placed = vec![false; free.len()];
        let new_sketch =
            |out: &mut Vec<ImportSketch>, layers: &mut Vec<u16>, plane: Plane, layer: u16, tags| {
                out.push(ImportSketch {
                    plane,
                    def: None,
                    group,
                    segments: Vec::new(),
                    curves: Vec::new(),
                    tags,
                });
                layers.push(layer);
                out.len() - 1
            };
        // A segment set onto its sketch's plane; `None` when that leaves
        // nothing of it.
        let add = |out: &mut Vec<ImportSketch>, si: usize, p: Point3, q: Point3| {
            let (p, q) = (onto(&out[si].plane, p), onto(&out[si].plane, q));
            if p.approx_eq(q, 1e-9) {
                return None;
            }
            out[si].segments.push([p, q]);
            Some(out[si].segments.len() - 1)
        };
        // Curves first, whole: one plane through all of a curve's ends.
        let mut by_curve: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, (_, c, _)) in free.iter().enumerate() {
            if let Some(c) = c {
                by_curve.entry(*c).or_default().push(i);
            }
        }
        for (c, members) in by_curve {
            let pts: Vec<Point3> = members.iter().flat_map(|&i| free[i].0).collect();
            let Some(plane) = fit_plane(&pts).filter(|pl| pts.iter().all(|&p| on_plane(pl, p)))
            else {
                // Not one plane's curve (a welded 3D polyline): its edges
                // go one by one, as plain lines.
                continue;
            };
            let layer = free[members[0]].2;
            let si = out
                .iter()
                .zip(&layers)
                .position(|(s, &l)| l == layer && pts.iter().all(|&p| on_plane(&s.plane, p)))
                .unwrap_or_else(|| {
                    new_sketch(&mut out, &mut layers, plane, layer, self.layer_tags(layer))
                });
            for &i in &members {
                let [p, q] = free[i].0;
                if let Some(seg) = add(&mut out, si, p, q) {
                    chains.entry((si, c)).or_default().push(seg);
                }
                placed[i] = true;
            }
        }
        for (i, ([p, q], _, layer)) in free.iter().enumerate() {
            if placed[i] {
                continue;
            }
            let (p, q, layer) = (*p, *q, *layer);
            let si =
                match out.iter().zip(&layers).position(|(s, &l)| {
                    l == layer && on_plane(&s.plane, p) && on_plane(&s.plane, q)
                }) {
                    Some(si) => si,
                    None => {
                        let Some(plane) = plane_through(p, q) else {
                            continue;
                        };
                        new_sketch(&mut out, &mut layers, plane, layer, self.layer_tags(layer))
                    }
                };
            add(&mut out, si, p, q);
        }
        let scale = uniform_scale(bake);
        for ((si, c), members) in chains {
            let plane = out[si].plane;
            let analytic = run
                .curves
                .get(c as usize)
                .and_then(|cv| cv.arc.as_ref())
                .and_then(|a| {
                    let scale = scale?;
                    let center =
                        bake.apply_point(Point3::new(a.center_m[0], a.center_m[1], a.center_m[2]));
                    // A circle claim stands only where its center is on
                    // the sketch's plane.
                    on_plane(&plane, center).then(|| CurveAnalytic {
                        geom: CurveGeom {
                            center: onto(&plane, center),
                            radius: a.radius_m * scale,
                        },
                        kind: SketchCurveKind::Circle,
                    })
                });
            out[si].curves.push((members, analytic));
        }
        self.hidden_lines_left.extend(hidden);
        out
    }

    /// Whether a Hew-written list says its own loose geometry is drawn
    /// sketches (`loose=sketches` on its definition): its faces are the
    /// regions those sketches close, not a solid.
    fn list_is_sketches(&self, run_idx: usize) -> bool {
        self.hew.written_by_hew
            && self.def_owner(run_idx).is_some_and(|o| {
                self.dicts.hew_entry(o, "loose") == Some(&Value::Text("sketches".into()))
            })
    }

    /// Def index for `(run, inherited material)`, building the `DefRecipe`
    /// on first use — its own (possibly split non-manifold) meshes PLUS its
    /// in-definition child placements as nested member instances, rebased
    /// from the composed tree's absolute poses into the definition's local
    /// frame. Every placement of the definition shares the one recipe.
    ///
    /// A `None` placeholder is cached before recursing so a (malformed,
    /// impossible-in-SketchUp) self-referential subtree drops the cyclic
    /// child instead of recursing forever; `ingest` would refuse the cycle
    /// typed anyway.
    fn assembly_variant(
        &mut self,
        run_idx: usize,
        eff_slot: u16,
        node: &openskp::Node,
    ) -> Option<usize> {
        if let Some(&cached) = self.def_variants.get(&(run_idx, eff_slot)) {
            // A degraded recipe (children dropped — singular first
            // placement) upgrades from the first placement that CAN rebase.
            if let Some(&di) = self.degraded_variants.get(&(run_idx, eff_slot))
                && !node.children.is_empty()
                && let Ok(inv) = pose_of(&node.world).inverse()
            {
                self.def_frames.push(inv);
                self.def_sketch_frames.push(Vec::new());
                let converted = self.convert_children(node);
                let mut drawing = self.def_sketch_frames.pop().unwrap_or_default();
                self.def_frames.pop();
                let children: Vec<ImportNode> = converted
                    .into_iter()
                    .map(|mut n| {
                        rebase_to_local(&mut n, &inv);
                        n
                    })
                    .collect();
                if !children.is_empty() {
                    self.defs[di].children.extend(children);
                    for s in &mut drawing {
                        s.def = Some(di);
                    }
                    self.sketches.extend(drawing);
                    self.degraded_variants.remove(&(run_idx, eff_slot));
                }
            }
            return cached;
        }
        self.def_variants.insert((run_idx, eff_slot), None);
        self.def_sketch_frames.push(Vec::new());
        let name = self.def_name(run_idx);
        // A Hew-written definition says what its own loose geometry is: a
        // solid (that may have had no name of its own), or drawn sketches.
        let def_owner = self.def_owner(run_idx);
        let def_hew =
            |cv: &Self, k: &str| def_owner.and_then(|o| cv.dicts.hew_entry(o, k).cloned());
        let sketches_here = self.list_is_sketches(run_idx);
        let mesh_name =
            if self.hew.written_by_hew && def_hew(self, "unnamed") == Some(Value::Bool(true)) {
                String::new()
            } else {
                name.clone().unwrap_or_default()
            };
        let key = self.synthetic_key();
        if let Some(Owner::Definition(definition)) = def_owner {
            self.provenance.nodes.push((
                key,
                Origin::DefinitionGeometry {
                    definition,
                    slot: eff_slot,
                },
            ));
        }
        let meshes = if sketches_here {
            Vec::new()
        } else {
            self.mesh_recipes(
                run_idx,
                mesh_name,
                eff_slot,
                &Transform::IDENTITY,
                Some(key),
            )
        };
        let mut children: Vec<ImportNode> = meshes.into_iter().map(ImportNode::Mesh).collect();
        // The definition's own images, in its frame.
        children.extend(self.image_children(run_idx, None));
        // Its own edges bounding no face, as drafting (a Hew-written
        // definition's drawing comes back through its recorded planes
        // below instead).
        if !sketches_here {
            let own = self.free_edge_sketches(run_idx, &Transform::IDENTITY, None);
            self.offer_sketches(own);
        }
        let mut degraded = false;
        // Child placements rebase to the definition's local frame:
        // local = world_def⁻¹ ∘ world_child. A singular world pose cannot
        // be rebased — its children are dropped loudly, never guessed at.
        match pose_of(&node.world).inverse() {
            Ok(inv) => {
                self.def_frames.push(inv);
                let converted = self.convert_children(node);
                self.def_frames.pop();
                for mut n in converted {
                    rebase_to_local(&mut n, &inv);
                    children.push(n);
                }
            }
            Err(_) => {
                if !node.children.is_empty() {
                    degraded = true;
                    self.warnings.push(format!(
                        "'{}' has a singular placement matrix; its {} nested \
                         placement(s) were dropped (restored if another \
                         placement can express them)",
                        name.as_deref().unwrap_or("unnamed component"),
                        node.children.len(),
                    ));
                }
            }
        }
        let planes = match (sketches_here, def_owner) {
            (true, Some(o)) => self.sketch_planes(o),
            _ => Vec::new(),
        };
        let mut drawing = self.def_sketch_frames.pop().unwrap_or_default();
        let built = if children.is_empty() && planes.is_empty() && drawing.is_empty() {
            None
        } else {
            // (A definition of drawing alone — a wire-frame component, a
            // light rig of edges — is a definition of its sketches; the
            // kernel keeps it as it keeps a group of lines alone.)
            self.defs.push(DefRecipe { name, children });
            let di = self.defs.len() - 1;
            if let Some(Owner::Definition(definition)) = def_owner {
                let name = self.model.definitions[definition].name.clone();
                self.provenance.defs.push((di, name, eff_slot));
            }
            if !planes.is_empty() {
                let sketches = self.run_sketches(run_idx, &planes, Some(di));
                self.sketches.extend(sketches);
            }
            for s in &mut drawing {
                s.def = Some(di);
            }
            self.sketches.extend(drawing);
            self.run_variants.entry(run_idx).or_default().push(di);
            Some(di)
        };
        self.def_variants.insert((run_idx, eff_slot), built);
        if degraded && let Some(di) = built {
            self.degraded_variants.insert((run_idx, eff_slot), di);
        }
        built
    }

    /// The definition name behind a geometry run, when linked.
    fn def_name(&self, run_idx: usize) -> Option<String> {
        let di = self.model.geometry[run_idx].def_index?;
        self.model
            .definition_links
            .iter()
            .find(|&&(dr, _)| dr as usize == di)
            .map(|&(_, k)| self.model.definitions[k].name.clone())
            .filter(|n| !n.is_empty())
    }

    /// One geometry run's mesh -> a healed `MeshRecipe`. `None` when no face
    /// survives (empty run, or everything hidden).
    ///
    /// `eff_slot` is the placing instance's inherited material (0 = none): it
    /// becomes `base_material`, which default-material faces resolve to.
    ///
    /// `bake` is baked into the resulting positions (and, transitively, the
    /// fitted UV frames): `Transform::IDENTITY` for definition-owned and
    /// loose-world runs (already in the right coordinate space), or a
    /// SketchUp GROUP's own world pose (`group_node`) — matching how
    /// `dae-import` bakes world-mesh transforms.
    ///
    /// Usually one recipe. A NON-MANIFOLD run (which `from_polygons` would
    /// reject whole) is decomposed by [`mesh_heal::split::split_non_manifold`]
    /// into several open-shell recipes — loudly (a warning names the mesh
    /// and the piece count), never silently (rule 4).
    fn mesh_recipes(
        &mut self,
        run_idx: usize,
        name: String,
        eff_slot: u16,
        bake: &Transform,
        key: Option<u64>,
    ) -> Vec<MeshRecipe> {
        self.mesh_recipes_of(run_idx, name, eff_slot, bake, key, None)
    }

    /// [`Self::mesh_recipes`] over some of the run's faces only (by index),
    /// tagged as given.
    fn mesh_recipes_of(
        &mut self,
        run_idx: usize,
        name: String,
        eff_slot: u16,
        bake: &Transform,
        key: Option<u64>,
        part: Option<LoosePart<'_>>,
    ) -> Vec<MeshRecipe> {
        self.ensure_claims(run_idx, bake);
        let mesh = &self.model.geometry[run_idx].mesh;

        let positions: Vec<Point3> = mesh
            .vertices
            .iter()
            .map(|v| Point3::new(v[0], v[1], v[2]))
            .collect();

        let mut faces: Vec<Vec<usize>> = Vec::new();
        // Each face's own index rides through the heal in place of its
        // material: the heal may turn a face over, and which of its two
        // SketchUp sides faces front decides its paint and its texture
        // placement (`face_paints`).
        let mut labels: Vec<u32> = Vec::new();
        let mut holes: Vec<Vec<Vec<usize>>> = Vec::new();

        for (fi, f) in mesh.faces.iter().enumerate() {
            if part.as_ref().is_some_and(|p| !p.faces.contains(&fi)) {
                continue;
            }
            // Faces always import — hidden faces and hidden-layer faces
            // included. Dropping them would open closed solids (a hidden face
            // on a box is display state, not geometry), and SketchUp's own
            // exports keep them (the  `.dae` ground truth: layers.dae
            // ships its hidden-layer box; hidden-entities.dae is watertight).
            // Visibility pruning applies at the INSTANCE level only, matching
            // the export rule.
            labels.push(fi as u32);
            faces.push(f.outer.iter().map(|&vi| vi as usize).collect());
            holes.push(
                f.holes
                    .iter()
                    .map(|ring| ring.iter().map(|&vi| vi as usize).collect())
                    .collect(),
            );
        }
        let no_uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); faces.len()];

        if faces.is_empty() {
            return Vec::new();
        }

        // Native tolerances: `.skp` coordinates are exact f64, like COLLADA
        // text (the glTF f32 relaxation does not apply). SketchUp's faces
        // are the faces the user drew, so coplanar neighbors stay apart: a
        // line drawn across a face survives, and a file Hew wrote reads
        // back as the faces it wrote.
        // Geometry Hew built (its definition, or the loose geometry, says
        // so): read as written, so it reads back exactly — no repair, which
        // would turn faces over or merge duplicates Hew kept; a mesh that is
        // not manifold still splits below, faces as they are. A file's own
        // geometry a save back carried is repaired as on any open.
        let built = match self.model.geometry[run_idx].def_index {
            None => self.hew.loose_built,
            Some(_) => {
                self.def_owner(run_idx)
                    .and_then(|o| self.dicts.hew_entry(o, "built"))
                    == Some(&Value::Bool(true))
            }
        };
        let (positions, faces, healed_labels, healed_uvs, healed_holes) =
            if self.hew.written_by_hew && built {
                heal_mesh_as_written(&positions, &faces, &labels, &no_uvs, &holes, bake)
            } else {
                heal_mesh_keeping_faces(&positions, &faces, &labels, &no_uvs, &holes, bake)
            };
        if faces.is_empty() {
            return Vec::new();
        }

        let (tags, hidden) = match part {
            Some(p) => (p.tags, p.hidden),
            None => (self.mesh_layer_tags(run_idx), false),
        };
        let base_material = if eff_slot != 0 {
            self.mats.dense(eff_slot)
        } else {
            kernel::NO_MATERIAL
        };

        // Non-manifold runs split into buildable open shells (rule 4: the
        // decomposition is reported, the geometry is never repaired).
        if let Some(pieces) = mesh_heal::split::split_non_manifold(
            &positions,
            &faces,
            &healed_labels,
            &healed_uvs,
            &healed_holes,
        ) {
            self.warnings.push(format!(
                "'{}' came in as {} separate piece{} because some of its edges \
                 are shared by more than two faces; the shape itself is unchanged",
                if name.is_empty() {
                    "unnamed mesh"
                } else {
                    &name
                },
                pieces.len(),
                if pieces.len() == 1 { "" } else { "s" },
            ));
            // A piece that closes on its own faces outward, as a whole mesh
            // that closes does (the mesh it came from could not tell: the
            // closed-shell test needs it whole) — a cavity a closed piece
            // encloses keeping its inward faces.
            let claims = self
                .claims
                .as_ref()
                .expect("the run's claims are built above");
            return mesh_heal::split::orient_pieces_outward(pieces)
                .into_iter()
                .map(|piece| {
                    let paints = self.face_paints(
                        run_idx,
                        eff_slot,
                        bake,
                        &piece.positions,
                        &piece.faces,
                        &piece.face_materials,
                    );
                    let mut extra = claims.narrow(&piece.positions);
                    extra.key = key;
                    extra.hidden = hidden;
                    extra.face_back_materials = paints.back;
                    recipe_from_arrays(
                        name.clone(),
                        piece.positions,
                        piece.faces,
                        paints.front,
                        paints.uvs,
                        piece.face_holes,
                        base_material,
                        tags.clone(),
                        extra,
                    )
                })
                .collect();
        }

        let paints = self.face_paints(run_idx, eff_slot, bake, &positions, &faces, &healed_labels);
        let mut extra = self
            .claims
            .as_ref()
            .expect("the run's claims are built above")
            .narrow(&positions);
        extra.key = key;
        extra.hidden = hidden;
        extra.face_back_materials = paints.back;
        vec![recipe_from_arrays(
            name,
            positions,
            faces,
            paints.front,
            paints.uvs,
            healed_holes,
            base_material,
            tags,
            extra,
        )]
    }

    /// Each healed face's paint, front and back, and its corner UVs, from
    /// the SketchUp face its label names. A face the heal turned over shows
    /// its SketchUp back in front: its paints swap, and its texture is
    /// placed as that side's. An unpainted front takes what the placement
    /// passes down (`base_material`), and a back painted like its front is
    /// no back paint at all.
    fn face_paints(
        &self,
        run_idx: usize,
        eff_slot: u16,
        bake: &Transform,
        positions: &[Point3],
        faces: &[Vec<usize>],
        labels: &[u32],
    ) -> FacePaints {
        let mesh = &self.model.geometry[run_idx].mesh;
        let local = bake.inverse().ok();
        // Under a mirroring bake a ring's winding turns over with it: the
        // side it winds counter-clockwise about is then the SketchUp face's
        // back.
        let mirrored = bake.determinant() < 0.0;
        let mut out = FacePaints::default();
        for (ring, &label) in faces.iter().zip(labels) {
            let f = &mesh.faces[label as usize];
            let source: Vec<Point3> = f
                .outer
                .iter()
                .map(|&vi| {
                    let v = mesh.vertices[vi as usize];
                    bake.apply_point(Point3::new(v[0], v[1], v[2]))
                })
                .collect();
            let healed: Vec<Point3> = ring.iter().map(|&i| positions[i]).collect();
            let turned = (newell(&healed).dot(newell(&source)) < 0.0) != mirrored;
            let (front, back, side) = if turned {
                (f.back_material, f.front_material, openskp::Side::Back)
            } else {
                (f.front_material, f.back_material, openskp::Side::Front)
            };
            let slot = |s: Option<u16>| s.filter(|&s| s != 0);
            let (front, back) = (slot(front), slot(back));
            let dense = |s: Option<u16>| s.map_or(kernel::NO_MATERIAL, |s| self.mats.dense(s));
            out.front.push(dense(front));
            out.back.push(if back == front {
                kernel::NO_MATERIAL
            } else {
                dense(back)
            });
            // Corner UVs whenever the face's front shows a texture: its own
            // placement on that side, or SketchUp's identity placement.
            let tex_slot = front.unwrap_or(eff_slot);
            let uvs = match local {
                Some(local) if self.mats.is_textured(tex_slot) => {
                    let size = self.mats.applied_size_in(tex_slot).unwrap_or((1.0, 1.0));
                    match f.uv_xform(side, size) {
                        Some(x) => healed
                            .iter()
                            .map(|&p| {
                                let q = local.apply_point(p);
                                x.apply([q.x, q.y, q.z])
                            })
                            .collect(),
                        None => Vec::new(),
                    }
                }
                _ => Vec::new(),
            };
            out.uvs.push(uvs);
        }
        if out.back.iter().all(|&b| b == kernel::NO_MATERIAL) {
            out.back.clear();
        }
        out
    }

    /// SketchUp's own dimensions and texts (not Hew's, which
    /// [`Converter::hew_annotations`] reads) as Hew annotations, each with
    /// its layer's tag and its origin; and how many were left in the file.
    /// What Hew places: a linear dimension whose frame is decoded (`variant`
    /// 0, 1 or 2) between anchors it can find, a text with a pushpin leader,
    /// a radius or diameter dimension on an arc. An anchor is a point, a
    /// vertex, an edge's midpoint, a point along an edge, or an arc's
    /// center, of the model's own geometry or of
    /// geometry placed exactly once (or named by its instance path). Each
    /// linear dimension measures along its axis, as SketchUp's does.
    #[allow(clippy::type_complexity)]
    fn sketchup_annotations(
        &self,
        places: &Placements<'_>,
    ) -> (
        Vec<(kernel::Annotation, Vec<Vec<String>>, AnnotationOrigin)>,
        usize,
    ) {
        let m = &*self.model;
        let ours = |o: Owner| self.dicts.hew_entry(o, "annotation").is_some();
        let p3 = |v: [f64; 3]| Point3::new(v[0], v[1], v[2]);
        let v3 = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        let free = |point: Point3| kernel::Anchor { node: None, point };
        // The world pose of the list a record is in, and the layers of the
        // containers above it: the model's own (none), or a group's or
        // component's placed exactly once (a record in a definition placed
        // several times would need one annotation per placement, and is
        // left out). A record under a hidden container is left out too:
        // SketchUp shows it only while every container above it shows.
        let frame_of = |def_index: Option<usize>| -> Option<(Transform, &[u16])> {
            match def_index {
                None => Some((Transform::IDENTITY, &[][..])),
                Some(di) => {
                    let ri = m.geometry.iter().position(|r| r.def_index == Some(di))?;
                    let (layers, hidden) = places.only_chain(ri)?;
                    let pose = places.only_pose(ri)?;
                    (!hidden).then_some((pose, layers.as_slice()))
                }
            }
        };
        // A record's tags: its own layer first (what SketchUp shows for it,
        // and what a save back writes), then each container's above it,
        // so it hides with the group it is in the way the group's own
        // drawing does.
        let tags_through = |own: u16, chain: &[u16]| -> Vec<Vec<String>> {
            let mut tags = self.layer_tags(own);
            for &slot in chain {
                for t in self.layer_tags(slot) {
                    if !tags.contains(&t) {
                        tags.push(t);
                    }
                }
            }
            tags
        };
        // A free anchor is in its list's frame; an anchored one resolves
        // through its entity's own placement.
        let anchor_in = |frame: &Transform, a: &openskp::Anchor| -> Option<Point3> {
            match (a.kind, a.entity) {
                (0 | 1, None) => Some(frame.apply_point(p3(a.point_m))),
                _ => places.anchor(a),
            }
        };
        let mut out = Vec::new();
        let mut left = 0usize;
        for (i, d) in m.dimensions.iter().enumerate() {
            if ours(Owner::Dimension(i)) {
                continue;
            }
            let made = (|| {
                if d.hidden {
                    return None;
                }
                let (frame, chain) = frame_of(d.def_index)?;
                let a = anchor_in(&frame, &d.start)?;
                let b = anchor_in(&frame, &d.end)?;
                let (along, offset, n) = dimension_geometry(
                    &frame,
                    d.variant,
                    v3(d.normal),
                    v3(d.x_axis),
                    b - a,
                    d.offset_m,
                )?;
                let b = a + along * (b - a).dot(along);
                Some((
                    kernel::Annotation::LinearDimension {
                        a: free(a),
                        b: free(b),
                        offset,
                        plane: Plane::from_point_normal(a, n).ok()?,
                        text_override: dimension_override(&d.text_override),
                    },
                    tags_through(d.layer, chain),
                ))
            })();
            match made {
                Some((a, tags)) => out.push((
                    a,
                    tags,
                    AnnotationOrigin::Dimension {
                        index: i,
                        pid: d.pid,
                    },
                )),
                None => left += 1,
            }
        }
        for (i, t) in m.texts.iter().enumerate() {
            if ours(Owner::Text(i)) {
                continue;
            }
            let made = (|| {
                // A screen text (kind 0 anchor, placed in the view) has no
                // place in the model.
                if t.hidden || t.anchor.kind == 0 {
                    return None;
                }
                let (frame, chain) = frame_of(t.def_index)?;
                Some((
                    kernel::Annotation::LeaderText {
                        anchor: free(anchor_in(&frame, &t.anchor)?),
                        offset: frame.apply_vector(v3(t.leader_offset)),
                        text: t.content.clone(),
                    },
                    tags_through(t.layer, chain),
                ))
            })();
            match made {
                Some((a, tags)) => out.push((
                    a,
                    tags,
                    AnnotationOrigin::Text {
                        index: i,
                        pid: t.pid,
                    },
                )),
                None => left += 1,
            }
        }
        for (i, r) in m.radial_dimensions.iter().enumerate() {
            if ours(Owner::RadialDimension(i)) {
                continue;
            }
            let made = (|| {
                if r.hidden {
                    return None;
                }
                let (_, chain) = frame_of(r.def_index)?;
                let (ri, world) = places.edge_list(r.def_index, r.edge?)?;
                let run = &m.geometry[ri];
                let e = run.mesh.edges.iter().find(|e| Some(e.pid) == r.edge)?;
                let arc = run.curves.get(e.curve? as usize)?.arc.as_ref()?;
                let c = world.apply_point(p3(arc.center_m));
                let xa = world.apply_vector(v3(arc.x_axis_m)).normalized().ok()?;
                let ya = world.apply_vector(v3(arc.y_axis_m)).normalized().ok()?;
                let n = world.apply_vector(v3(arc.normal)).normalized().ok()?;
                let radius = world.apply_vector(v3(arc.x_axis_m)).length() * arc.radius_m
                    / v3(arc.x_axis_m).length().max(f64::MIN_POSITIVE);
                let radial = xa * r.leader[0].cos() + ya * r.leader[0].sin();
                let a = kernel::Annotation::RadialDimension {
                    anchor: free(c + radial * radius),
                    kind: if r.diameter {
                        kernel::RadialKind::Diameter
                    } else {
                        kernel::RadialKind::Radius
                    },
                    curve: kernel::CapturedCurve {
                        center: c,
                        radius,
                        plane: Plane::from_point_normal(c, n).ok()?,
                    },
                    // Negative is outward (§10.12).
                    leader_dir: if r.leader[1] < 0.0 {
                        radial
                    } else {
                        radial * -1.0
                    },
                    text_override: dimension_override(&r.text_override),
                };
                Some((a, tags_through(r.layer, chain)))
            })();
            match made {
                Some((a, tags)) => out.push((
                    a,
                    tags,
                    AnnotationOrigin::RadialDimension {
                        index: i,
                        pid: r.pid,
                    },
                )),
                None => left += 1,
            }
        }
        (out, left)
    }

    /// The images placed in a run, as instances of their image definitions
    /// (`image_def`): in the run's own frame, or carried `to_world` by the
    /// placement the run is reached through (a group's children are world
    /// nodes). Each keyed by its persistent id and tagged by its layer.
    fn image_children(&mut self, run_idx: usize, to_world: Option<&Transform>) -> Vec<ImportNode> {
        let images: Vec<openskp::ImagePlacement> = self.model.geometry[run_idx].images.clone();
        let mut out = Vec::new();
        for im in images {
            let Some(def) = self.image_def(im.defref) else {
                if self.image_warned.insert(im.pid) {
                    self.warnings.push(format!(
                        "an image (id {}) has no picture to show and was left out; a save back \
                         over the file keeps it",
                        im.pid
                    ));
                }
                continue;
            };
            let local = pose_of(&matrix_of(&im.transform));
            let pose = match to_world {
                Some(w) => local.then(w),
                None => local,
            };
            self.origin(im.pid, |pid| Origin::Image { pid });
            out.push(ImportNode::Instance {
                def,
                pose,
                name: None,
                tags: self.layer_tags(im.layer),
                hidden: im.hidden,
                key: key_of(im.pid),
            });
        }
        out
    }

    /// The definition recipe for an image definition (by map index), built
    /// once: its face, painted with the picture.
    fn image_def(&mut self, defref: u32) -> Option<usize> {
        if let Some(&d) = self.image_defs.get(&defref) {
            return d;
        }
        let ri = self
            .model
            .geometry
            .iter()
            .position(|r| r.def_index == Some(defref as usize));
        let built = ri.and_then(|ri| {
            let name = self.def_name(ri);
            let key = self.synthetic_key();
            let meshes = self.mesh_recipes(
                ri,
                name.clone().unwrap_or_default(),
                0,
                &Transform::IDENTITY,
                Some(key),
            );
            if meshes.is_empty() {
                return None;
            }
            self.defs.push(DefRecipe {
                name,
                children: meshes.into_iter().map(ImportNode::Mesh).collect(),
            });
            Some(self.defs.len() - 1)
        });
        self.image_defs.insert(defref, built);
        built
    }

    /// The dimensions and texts a Hew-written file carries, as the Hew
    /// annotations they were written from (SketchUp's own annotations are
    /// not read: a save back over the file keeps them as they are).
    /// Each with its record's layer's tag.
    #[allow(clippy::type_complexity)]
    fn hew_annotations(&self) -> (Vec<kernel::Annotation>, Vec<Vec<Vec<String>>>) {
        if !self.hew.written_by_hew {
            return (Vec::new(), Vec::new());
        }
        let m = &*self.model;
        let owners = (0..m.texts.len())
            .map(|i| (Owner::Text(i), m.texts[i].layer))
            .chain((0..m.dimensions.len()).map(|i| (Owner::Dimension(i), m.dimensions[i].layer)))
            .chain(
                (0..m.radial_dimensions.len())
                    .map(|i| (Owner::RadialDimension(i), m.radial_dimensions[i].layer)),
            );
        let (mut out, mut tags): (Vec<kernel::Annotation>, Vec<Vec<Vec<String>>>) = owners
            .filter_map(|(o, layer)| {
                let a = extras::decode_annotation(self.dicts.hew_entry(o, "annotation")?)?;
                Some((a, self.layer_tags(layer)))
            })
            .unzip();
        if let Some(Value::Array(rest)) = self.dicts.hew_entry(Owner::Model, "annotations") {
            for a in rest.iter().filter_map(extras::decode_annotation) {
                out.push(a);
                tags.push(Vec::new());
            }
        }
        (out, tags)
    }

    /// Record the origin of a placement's node, by its persistent id (no
    /// origin without one: the legacy byte-scan path).
    fn origin(&mut self, pid: u32, f: impl FnOnce(u32) -> Origin) {
        if pid != 0 {
            self.provenance.nodes.push((u64::from(pid), f(pid)));
        }
    }

    fn synthetic_key(&mut self) -> u64 {
        let k = self.next_key;
        self.next_key += 1;
        k
    }

    /// The tag path a layer stands for: the path a Hew-written file
    /// recorded for it, else the layer name as one segment.
    fn tag_path(&self, layer: &str) -> Vec<String> {
        self.hew
            .tag_paths
            .get(layer)
            .cloned()
            .unwrap_or_else(|| vec![layer.to_string()])
    }

    /// The owner of the definition behind a geometry run.
    fn def_owner(&self, run_idx: usize) -> Option<Owner> {
        let di = self.model.geometry[run_idx].def_index?;
        self.model
            .definition_links
            .iter()
            .find(|&&(dr, _)| dr as usize == di)
            .map(|&(_, k)| Owner::Definition(k))
    }

    /// The sketch planes a Hew-written list recorded, `(origin, normal)`.
    fn sketch_planes(&self, owner: Owner) -> Vec<Plane> {
        let Some(Value::Array(items)) = self.dicts.hew_entry(owner, "sketch_planes") else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|v| {
                let Value::Array(n) = v else { return None };
                let f: Vec<f64> = n
                    .iter()
                    .map(|x| match x {
                        Value::Double(d) if d.is_finite() => Some(*d),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let [ox, oy, oz, nx, ny, nz] = f.as_slice() else {
                    return None;
                };
                Plane::from_point_normal(Point3::new(*ox, *oy, *oz), Vec3::new(*nx, *ny, *nz)).ok()
            })
            .collect()
    }

    /// A Hew-written list's loose geometry back as the sketches it was
    /// drawn as: each edge goes to the first recorded plane holding both
    /// its ends, welded curves come back as curve chains (an arc's circle
    /// as a circle claim, a polygon's center from its curve's `hew`
    /// dictionary). A sketch whose edges all share one layer takes it as
    /// its tag (the layer a save back wrote for the sketch's first tag). An
    /// edge on no recorded plane is reported and left out.
    fn run_sketches(
        &mut self,
        run_idx: usize,
        planes: &[Plane],
        def: Option<usize>,
    ) -> Vec<ImportSketch> {
        let run = &self.model.geometry[run_idx];
        let mesh = &run.mesh;
        let at = |i: u32| {
            let v = mesh.vertices[i as usize];
            Point3::new(v[0], v[1], v[2])
        };
        const ON: f64 = 1e-7;
        let mut out: Vec<ImportSketch> = planes
            .iter()
            .map(|&plane| ImportSketch {
                plane,
                def,
                group: None,
                segments: Vec::new(),
                curves: Vec::new(),
                tags: Vec::new(),
            })
            .collect();
        // The layers of each sketch's edges, parallel to `out`.
        let mut layers: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); planes.len()];
        // curve index -> (sketch, its segment indices)
        let mut chains: BTreeMap<(usize, u32), Vec<usize>> = BTreeMap::new();
        let mut off = 0usize;
        for (ei, e) in mesh.edges.iter().enumerate() {
            let (p, q) = (at(e.v0), at(e.v1));
            // The sketch the edge names, when the list recorded one per
            // edge (two sketches can share a plane); else the first plane
            // holding both its ends.
            let named = match self.dicts.hew_entry(Owner::Edge(run_idx, ei), "sketch") {
                Some(Value::Int(i)) => usize::try_from(*i).ok().filter(|&i| i < planes.len()),
                _ => None,
            };
            let Some(si) = named.or_else(|| {
                planes.iter().position(|pl| {
                    pl.signed_distance(p).abs() <= ON && pl.signed_distance(q).abs() <= ON
                })
            }) else {
                off += 1;
                continue;
            };
            let seg = out[si].segments.len();
            out[si].segments.push([p, q]);
            layers[si].insert(e.layer);
            if let Some(c) = e.curve {
                chains.entry((si, c)).or_default().push(seg);
            }
        }
        for (sk, layers) in out.iter_mut().zip(&layers) {
            if let (1, Some(&layer)) = (layers.len(), layers.first()) {
                sk.tags = self.layer_tags(layer);
            }
        }
        for ((si, c), members) in chains {
            let polygon = match self
                .dicts
                .hew_entry(Owner::Curve(run_idx, c as usize), "polygon")
            {
                Some(Value::Array(v)) => match v.as_slice() {
                    [
                        Value::Double(x),
                        Value::Double(y),
                        Value::Double(z),
                        Value::Double(r),
                    ] => Some(CurveAnalytic {
                        geom: CurveGeom {
                            center: Point3::new(*x, *y, *z),
                            radius: *r,
                        },
                        kind: SketchCurveKind::Polygon,
                    }),
                    _ => None,
                },
                _ => None,
            };
            let analytic = polygon.or_else(|| {
                let a = run.curves.get(c as usize)?.arc.as_ref()?;
                Some(CurveAnalytic {
                    geom: CurveGeom {
                        center: Point3::new(a.center_m[0], a.center_m[1], a.center_m[2]),
                        radius: a.radius_m,
                    },
                    kind: SketchCurveKind::Circle,
                })
            });
            out[si].curves.push((members, analytic));
        }
        if off > 0 {
            self.warnings.push(format!(
                "{off} drawn edge{} lay on no recorded sketch plane and {} left out",
                if off == 1 { "" } else { "s" },
                if off == 1 { "was" } else { "were" },
            ));
        }
        out.retain(|s| !s.segments.is_empty());
        out
    }

    /// Build the claims of run `run_idx` under `bake` ([`RunClaims`]),
    /// unless the run last read was this one under this bake: a loose run
    /// is read piece by piece, each taking its own claims from the one
    /// index.
    fn ensure_claims(&mut self, run_idx: usize, bake: &Transform) {
        if self
            .claims
            .as_ref()
            .is_some_and(|c| c.run_idx == run_idx && c.bake == *bake)
        {
            return;
        }
        let run = &self.model.geometry[run_idx];
        let mesh = &run.mesh;
        let points: Vec<Point3> = mesh
            .vertices
            .iter()
            .map(|v| bake.apply_point(Point3::new(v[0], v[1], v[2])))
            .collect();
        let scale = uniform_scale(bake);
        let mut claims: Vec<Claim> = Vec::new();
        let flat = flat_joins(mesh);
        for e in &mesh.edges {
            // A hidden edge between two faces of one plane draws nothing in
            // SketchUp; soft, it draws nothing in Hew either and smooths no
            // shading there, the faces' normals being one.
            let hidden_flat = e.hidden && flat.contains(&(e.v0.min(e.v1), e.v0.max(e.v1)));
            if e.soft || e.smooth || hidden_flat {
                claims.push(Claim::Soft(e.v0, e.v1));
            }
            if let (Some(c), Some(s)) = (e.curve, scale)
                && let Some(a) = run.curves.get(c as usize).and_then(|c| c.arc.as_ref())
            {
                let center =
                    bake.apply_point(Point3::new(a.center_m[0], a.center_m[1], a.center_m[2]));
                claims.push(Claim::Curve(
                    e.v0,
                    e.v1,
                    CurveGeom {
                        center,
                        radius: a.radius_m * s,
                    },
                ));
            }
        }
        let mut surfaces: Vec<SurfaceRef> = Vec::new();
        if self.hew.written_by_hew
            && let Some(s) = scale
        {
            for fi in 0..mesh.faces.len() {
                let Some(SurfaceRef::Cylinder {
                    axis_point,
                    axis,
                    radius,
                }) = self
                    .dicts
                    .hew_entry(Owner::Face(run_idx, fi), "surface")
                    .and_then(extras::surface_of)
                else {
                    continue;
                };
                let Ok(axis) = bake.apply_vector(axis).normalized() else {
                    continue;
                };
                let surf = SurfaceRef::Cylinder {
                    axis_point: bake.apply_point(axis_point),
                    axis,
                    radius: radius * s,
                };
                if !surfaces.contains(&surf) {
                    surfaces.push(surf);
                }
            }
        }
        self.claims = Some(RunClaims::new(run_idx, *bake, points, claims, surfaces));
    }

    /// Every dictionary of the file, other than Hew's own, as Hew
    /// attributes on what it hangs on: the model, a definition (each of its
    /// material variants), a material, a layer, a placement (by its id).
    fn attributes(&self) -> Vec<(ImportAttrTarget, AttrDict)> {
        let mut out = Vec::new();
        for k in self.dicts.owners().collect::<Vec<_>>() {
            let (target, owner): (Vec<ImportAttrTarget>, Owner) = match extras::mapped(k) {
                Mapped::Model => (vec![ImportAttrTarget::Document], Owner::Model),
                Mapped::Definition(i) => {
                    let map = self.model.definitions.get(i).and_then(|d| d.map_index);
                    let variants: Vec<ImportAttrTarget> = self
                        .model
                        .geometry
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| map.is_some() && r.def_index == map)
                        .flat_map(|(ri, _)| self.run_variants.get(&ri).cloned().unwrap_or_default())
                        .map(ImportAttrTarget::Def)
                        .collect();
                    (variants, Owner::Definition(i))
                }
                Mapped::Material(i) => {
                    let dense = self
                        .model
                        .material_links
                        .iter()
                        .find(|&&(_, idx)| idx == i)
                        .map(|&(slot, _)| self.mats.dense(slot))
                        .filter(|&d| d != kernel::NO_MATERIAL);
                    (
                        dense
                            .map(|d| ImportAttrTarget::Material(d as usize))
                            .into_iter()
                            .collect(),
                        Owner::Material(i),
                    )
                }
                Mapped::Layer(i) => {
                    let path = self
                        .model
                        .layers
                        .get(i)
                        .filter(|l| i > 0 && !l.name.is_empty())
                        .map(|l| self.tag_path(&l.name));
                    (
                        path.map(ImportAttrTarget::Tag).into_iter().collect(),
                        Owner::Layer(i),
                    )
                }
                Mapped::Instance(ri, pi) => {
                    let pid = self
                        .model
                        .geometry
                        .get(ri)
                        .and_then(|r| r.placed.get(pi))
                        .map(|p| p.pid)
                        .filter(|&p| p != 0);
                    (
                        pid.map(|p| ImportAttrTarget::Node(u64::from(p)))
                            .into_iter()
                            .collect(),
                        Owner::Instance(ri, pi),
                    )
                }
                Mapped::Guide | Mapped::Other => continue,
            };
            let mut dict = self.dicts.attrs(owner);
            if matches!(owner, Owner::Model) {
                // SketchUp's own model dictionaries (and the ones a writer
                // makes for every file) describe the file, not the user's
                // data; a save back to the file carries them from it.
                dict.retain(|name, _| !extras::sketchup_owned(name));
            }
            if dict.is_empty() {
                continue;
            }
            for t in target {
                out.push((t, dict.clone()));
            }
        }
        out
    }

    /// Tag paths for an entity on `slot`: the layer name as a single-segment
    /// path, for named non-default layers only.
    fn layer_tags(&self, slot: u16) -> Vec<Vec<String>> {
        if slot == 0 {
            return Vec::new();
        }
        self.model
            .layer_of(slot)
            .filter(|l| !l.name.is_empty())
            .map(|l| vec![self.tag_path(&l.name)])
            .unwrap_or_default()
    }

    /// Object-level tags for a run's mesh: Hew tags are per-object, `.skp`
    /// layers are per-face — when every face agrees on ONE non-default
    /// layer, carry it; mixed-layer meshes carry none.
    fn mesh_layer_tags(&self, run_idx: usize) -> Vec<Vec<String>> {
        let mesh = &self.model.geometry[run_idx].mesh;
        let slots: BTreeSet<u16> = mesh.faces.iter().map(|f| f.layer).collect();
        match (slots.len(), slots.first()) {
            (1, Some(&slot)) => self.layer_tags(slot),
            _ => Vec::new(),
        }
    }
}

/// The edges (vertex pairs, lower index first) where exactly two of the
/// mesh's faces meet on one plane, facing the same way.
/// How far a drafting line's end may sit off its sketch plane and still
/// be on it: the wander of a file's coordinates (SketchUp keeps inches to
/// about a nanometer; a long edge drifts further), a tenth of a micron,
/// far below anything drawn or shown. The kernel's own plane gate is a
/// nanometer (`tol::PLANE_DIST`), so an end this far off is set onto the
/// plane when the sketch is built (`onto`).
const DRAWING_ON_PLANE: f64 = 1e-7;

/// A plane holding the line `p`–`q`: the first axis plane that does (z,
/// then x, then y), else the vertical plane through the line. `None` for
/// a degenerate line.
fn plane_through(p: Point3, q: Point3) -> Option<Plane> {
    let dir = (q - p).normalized().ok()?;
    for n in [
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ] {
        // Both ends on the plane, as a distance (an angle test lets a long
        // edge drift off).
        if (q - p).dot(n).abs() <= DRAWING_ON_PLANE {
            return Plane::from_point_normal(p, n).ok();
        }
    }
    let n = dir.cross(Vec3::new(0.0, 0.0, 1.0)).normalized().ok()?;
    Plane::from_point_normal(p, n).ok()
}

/// The plane through a set of points that are coplanar: the first point,
/// the farthest from it, and the one farthest from their line. `None`
/// when they are collinear (or fewer than three).
fn fit_plane(pts: &[Point3]) -> Option<Plane> {
    let a = *pts.first()?;
    let far = |p: &Point3| (*p - a).length();
    let b = *pts.iter().max_by(|x, y| far(x).total_cmp(&far(y)))?;
    let ab = (b - a).normalized().ok()?;
    let off = |p: &Point3| {
        let v = *p - a;
        (v - ab * v.dot(ab)).length()
    };
    let c = *pts.iter().max_by(|x, y| off(x).total_cmp(&off(y)))?;
    if off(&c) <= DRAWING_ON_PLANE {
        return None;
    }
    let n = ab.cross(c - a).normalized().ok()?;
    Plane::from_point_normal(a, n).ok()
}

/// `p` set onto `plane` along its normal.
fn onto(plane: &Plane, p: Point3) -> Point3 {
    p + plane.normal() * -plane.signed_distance(p)
}

/// Whether `p` lies on `plane` as drafting goes ([`DRAWING_ON_PLANE`]).
fn on_plane(plane: &Plane, p: Point3) -> bool {
    plane.signed_distance(p).abs() <= DRAWING_ON_PLANE
}

/// Carries a sketch read in a list's frame through `t`: its ends and
/// plane point as points, its normal as a normal (the inverse transpose,
/// so a stretched or sheared placement keeps the ends on the plane); a
/// circle claim keeps its radius scaled under a similarity, else it is
/// dropped. `false` when `t` has no inverse, the sketch untouched.
fn bake_sketch(s: &mut ImportSketch, t: &Transform) -> bool {
    let Ok(inv) = t.inverse() else {
        return false;
    };
    // M⁻ᵀ·n over the row-major affine (M⁻¹[j][i] = a[4j + i]).
    let (a, n) = (inv.to_affine(), s.plane.normal());
    let normal = Vec3::new(
        a[0] * n.x + a[4] * n.y + a[8] * n.z,
        a[1] * n.x + a[5] * n.y + a[9] * n.z,
        a[2] * n.x + a[6] * n.y + a[10] * n.z,
    );
    let Ok(plane) = Plane::from_point_normal(t.apply_point(s.plane.point()), normal) else {
        return false;
    };
    s.plane = plane;
    for seg in &mut s.segments {
        seg[0] = t.apply_point(seg[0]);
        seg[1] = t.apply_point(seg[1]);
    }
    let scale = uniform_scale(t);
    for (_, analytic) in &mut s.curves {
        *analytic = analytic.take().and_then(|mut a| {
            a.geom.center = t.apply_point(a.geom.center);
            a.geom.radius *= scale?;
            Some(a)
        });
    }
    true
}

fn flat_joins(mesh: &openskp::Mesh) -> BTreeSet<(u32, u32)> {
    let mut users: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        for ring in std::iter::once(&f.outer).chain(f.holes.iter()) {
            for (i, &a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                users.entry((a.min(b), a.max(b))).or_default().push(fi);
            }
        }
    }
    users
        .into_iter()
        .filter(|(_, fs)| {
            let [f, g] = fs.as_slice() else { return false };
            let (n, m) = (mesh.faces[*f].normal, mesh.faces[*g].normal);
            n[0] * m[0] + n[1] * m[1] + n[2] * m[2] > 1.0 - 1e-9
        })
        .map(|(k, _)| k)
        .collect()
}

/// Where the file's geometry lies in the world, for placing what anchors
/// to it: each placement's world pose by its instance path (the persistent
/// ids of the placing instances from the model down), each geometry run's
/// placements, and each vertex's and edge's run by persistent id.
struct Placements<'a> {
    model: &'a openskp::Model,
    by_path: BTreeMap<Vec<u32>, (usize, Transform)>,
    of_run: BTreeMap<usize, Vec<Transform>>,
    /// Per placement of a run, in `of_run` order: the layers of every
    /// placement on the way down to it (nearest last), and whether any of
    /// them is hidden. What a record in the run inherits for its
    /// visibility: SketchUp shows an entity only while every container
    /// above it shows.
    chain_of_run: BTreeMap<usize, Vec<(Vec<u16>, bool)>>,
    vertex_runs: BTreeMap<u32, Vec<(usize, usize)>>,
    edge_runs: BTreeMap<u32, Vec<(usize, usize)>>,
    curve_runs: BTreeMap<u32, Vec<(usize, usize)>>,
}

impl<'a> Placements<'a> {
    fn of(cv: &Converter, tree: &[openskp::Node], model: &'a openskp::Model) -> Placements<'a> {
        let mut p = Placements {
            model,
            by_path: BTreeMap::new(),
            of_run: BTreeMap::new(),
            chain_of_run: BTreeMap::new(),
            vertex_runs: BTreeMap::new(),
            edge_runs: BTreeMap::new(),
            curve_runs: BTreeMap::new(),
        };
        let top: Vec<u32> = model
            .instances
            .iter()
            .filter(|i| {
                !model
                    .geometry
                    .iter()
                    .any(|r| r.start <= i.offset && i.offset < r.end)
            })
            .map(|i| i.pid)
            .collect();
        fn visit(
            cv: &Converter,
            node: &openskp::Node,
            pid: u32,
            path: &mut Vec<u32>,
            layers: &mut Vec<u16>,
            hidden: bool,
            p: &mut Placements<'_>,
        ) {
            path.push(pid);
            layers.push(node.layer);
            let hidden = hidden || node.hidden;
            if let Some(ri) = node.run {
                let pose = pose_of(&node.world);
                p.by_path.insert(path.clone(), (ri, pose));
                p.of_run.entry(ri).or_default().push(pose);
                p.chain_of_run
                    .entry(ri)
                    .or_default()
                    .push((layers.clone(), hidden));
            }
            let kids: Vec<(&openskp::Node, u32)> = cv.children_with_pids(node).collect();
            for (c, cpid) in kids {
                visit(cv, c, cpid, path, layers, hidden, p);
            }
            layers.pop();
            path.pop();
        }
        let mut path = Vec::new();
        let mut layers = Vec::new();
        for (node, pid) in tree.iter().zip(top) {
            visit(cv, node, pid, &mut path, &mut layers, false, &mut p);
        }
        for (ri, run) in model.geometry.iter().enumerate() {
            if run.def_index.is_none() {
                p.of_run.entry(ri).or_default().push(Transform::IDENTITY);
                p.chain_of_run
                    .entry(ri)
                    .or_default()
                    .push((Vec::new(), false));
            }
            for (vi, &pid) in run.mesh.vertex_pids.iter().enumerate() {
                if pid != 0 {
                    p.vertex_runs.entry(pid).or_default().push((ri, vi));
                }
            }
            for (ei, e) in run.mesh.edges.iter().enumerate() {
                if e.pid != 0 {
                    p.edge_runs.entry(e.pid).or_default().push((ri, ei));
                }
            }
            for (ci, c) in run.curves.iter().enumerate() {
                if c.pid != 0 {
                    p.curve_runs.entry(c.pid).or_default().push((ri, ci));
                }
            }
        }
        p
    }

    /// The one world pose of a run's geometry: the model's own, or a run
    /// placed exactly once.
    fn only_pose(&self, ri: usize) -> Option<Transform> {
        match self.of_run.get(&ri).map(Vec::as_slice) {
            Some([one]) => Some(*one),
            _ => None,
        }
    }

    /// The containers above a run placed exactly once: their layers, and
    /// whether one of them is hidden.
    fn only_chain(&self, ri: usize) -> Option<&(Vec<u16>, bool)> {
        match self.chain_of_run.get(&ri).map(Vec::as_slice) {
            Some([one]) => Some(one),
            _ => None,
        }
    }

    /// Where an entity of `runs` (by persistent id) lies, through `path`
    /// when the anchor names one, else through the one run holding it,
    /// placed once.
    fn locate(
        &self,
        runs: Option<&Vec<(usize, usize)>>,
        path: &[u32],
    ) -> Option<(usize, usize, Transform)> {
        let runs = runs?;
        if !path.is_empty() {
            if let Some(&(ri, pose)) = self.by_path.get(path)
                && let Some(&(_, k)) = runs.iter().find(|&&(r, _)| r == ri)
            {
                return Some((ri, k, pose));
            }
            // A path followed only part of the way (a step the file lost):
            // the one placement below it holding the entity, as a writer
            // looks for it; else the one run holding it, as with no path.
            let mut below = self
                .by_path
                .iter()
                .filter(|(k, _)| k.len() > path.len() && k.starts_with(path))
                .filter_map(|(_, &(ri, pose))| {
                    runs.iter()
                        .find(|&&(r, _)| r == ri)
                        .map(|&(_, k)| (ri, k, pose))
                });
            if let (Some(hit), None) = (below.next(), below.next()) {
                return Some(hit);
            }
        }
        match runs.as_slice() {
            [(ri, k)] => Some((*ri, *k, self.only_pose(*ri)?)),
            _ => None,
        }
    }

    /// An anchor's world point: a free point (kind 0 or 1, among the
    /// model's own entities), a vertex (kind 2), an edge's midpoint (kind
    /// 4), a point along an edge at its parameter (kind 5), where the lines
    /// of two edges meet (kind 9), or an arc's center (kind 10).
    fn anchor(&self, a: &openskp::Anchor) -> Option<Point3> {
        let p3 = |v: [f64; 3]| Point3::new(v[0], v[1], v[2]);
        match (a.kind, a.entity) {
            (0 | 1, None) => Some(p3(a.point_m)),
            (2, Some(e)) => {
                let (ri, vi, pose) = self.locate(self.vertex_runs.get(&e), &a.path)?;
                Some(pose.apply_point(p3(self.vertex(ri, vi)?)))
            }
            (4, Some(e)) => {
                let (ri, ei, pose) = self.locate(self.edge_runs.get(&e), &a.path)?;
                let (p, q) = self.edge_ends(ri, ei)?;
                Some(pose.apply_point(p + (q - p) * 0.5))
            }
            (5, Some(e)) => {
                let (ri, ei, pose) = self.locate(self.edge_runs.get(&e), &a.path)?;
                let (p, q) = self.edge_ends(ri, ei)?;
                let t = a.point_m[0];
                Some(pose.apply_point(p + (q - p) * t))
            }
            (9, Some(e)) => {
                let line = |e: u32, path: &[u32]| -> Option<(Point3, Vec3)> {
                    let (ri, ei, pose) = self.locate(self.edge_runs.get(&e), path)?;
                    let (p, q) = self.edge_ends(ri, ei)?;
                    let (p, q) = (pose.apply_point(p), pose.apply_point(q));
                    Some((p, q - p))
                };
                let (p, u) = line(e, &a.path)?;
                let (q, v) = line(a.second?, &a.second_path)?;
                // The closest points of the two lines, met halfway (they
                // cross, so they coincide but for rounding).
                let w = p - q;
                let (uu, uv, vv) = (u.dot(u), u.dot(v), v.dot(v));
                let den = uu * vv - uv * uv;
                if den <= 1e-12 * uu * vv {
                    return None;
                }
                let s = (uv * v.dot(w) - vv * u.dot(w)) / den;
                let t = (uu * v.dot(w) - uv * u.dot(w)) / den;
                let (a, b) = (p + u * s, q + v * t);
                Some(a + (b - a) * 0.5)
            }
            (10, Some(e)) => {
                let (ri, ci, pose) = self.locate(self.curve_runs.get(&e), &a.path)?;
                let arc = self.model.geometry.get(ri)?.curves.get(ci)?.arc.as_ref()?;
                Some(pose.apply_point(p3(arc.center_m)))
            }
            _ => None,
        }
    }

    /// The run and world pose of the list a radial dimension's edge is in.
    fn edge_list(&self, def_index: Option<usize>, edge: u32) -> Option<(usize, Transform)> {
        let (ri, _, pose) = self.locate(self.edge_runs.get(&edge), &[])?;
        let _ = def_index;
        Some((ri, pose))
    }

    fn vertex(&self, ri: usize, vi: usize) -> Option<[f64; 3]> {
        self.model.geometry.get(ri)?.mesh.vertices.get(vi).copied()
    }

    fn edge_ends(&self, ri: usize, ei: usize) -> Option<(Point3, Point3)> {
        let mesh = &self.model.geometry.get(ri)?.mesh;
        let e = mesh.edges.get(ei)?;
        let p = |v: u32| {
            mesh.vertices
                .get(v as usize)
                .map(|v| Point3::new(v[0], v[1], v[2]))
        };
        Some((p(e.v0)?, p(e.v1)?))
    }
}

/// [`Converter::face_paints`]: front and back paint per face (dense, or
/// `NO_MATERIAL`; `back` empty when no face has one) and corner UVs.
#[derive(Default)]
struct FacePaints {
    front: Vec<u32>,
    back: Vec<u32>,
    uvs: Vec<Vec<[f64; 2]>>,
}

/// A ring's Newell normal (unnormalized): which way it winds.
fn newell(ring: &[Point3]) -> Vec3 {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for (i, a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        n = n + Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    n
}

/// Some of a loose run's faces, read as a mesh of their own: the face
/// indices, the tags they take, and whether the mesh is hidden.
/// What a run says about its edges beyond faces, under one bake: soft and
/// smooth edges, welded arcs, and (in a Hew-written file) the cylinders its
/// faces are facets of. Built once per run ([`Converter::ensure_claims`])
/// and indexed by the run's vertices, so each mesh read from the run takes
/// only the claims on its own edges ([`RunClaims::narrow`]): a run read as
/// many pieces (its loose geometry by layer, a non-manifold run split
/// apart) hands each piece its own, not the whole run's over again.
struct RunClaims {
    run_idx: usize,
    bake: Transform,
    /// The run's vertices, baked.
    points: Vec<Point3>,
    /// Vertices bucketed on a grid of the import tolerance, the one the
    /// kernel matches a claim's endpoints with.
    grid: BTreeMap<[i64; 3], Vec<u32>>,
    claims: Vec<Claim>,
    /// Each vertex's claims, as indexes into `claims`.
    incident: Vec<Vec<u32>>,
    surfaces: Vec<SurfaceRef>,
}

/// One edge claim, by the run's vertex indexes.
enum Claim {
    Soft(u32, u32),
    Curve(u32, u32, CurveGeom),
}

impl Claim {
    fn ends(&self) -> (u32, u32) {
        match self {
            Claim::Soft(a, b) | Claim::Curve(a, b, _) => (*a, *b),
        }
    }
}

impl RunClaims {
    const TOL: f64 = kernel::tol::IMPORT_PLANE_DIST;

    fn cell(p: Point3) -> [i64; 3] {
        // Clamped so a neighboring cell of an absurd coordinate (a mutated
        // file's) can still be named without overflowing.
        [p.x, p.y, p.z].map(|c| (c / Self::TOL).floor().clamp(-1e15, 1e15) as i64)
    }

    fn new(
        run_idx: usize,
        bake: Transform,
        points: Vec<Point3>,
        claims: Vec<Claim>,
        surfaces: Vec<SurfaceRef>,
    ) -> RunClaims {
        let mut grid: BTreeMap<[i64; 3], Vec<u32>> = BTreeMap::new();
        for (i, &p) in points.iter().enumerate() {
            grid.entry(Self::cell(p)).or_default().push(i as u32);
        }
        let mut incident: Vec<Vec<u32>> = vec![Vec::new(); points.len()];
        for (ci, c) in claims.iter().enumerate() {
            let (a, b) = c.ends();
            for v in [a, b] {
                if let Some(list) = incident.get_mut(v as usize) {
                    list.push(ci as u32);
                }
            }
        }
        RunClaims {
            run_idx,
            bake,
            points,
            grid,
            claims,
            incident,
            surfaces,
        }
    }

    /// The claims on the edges among `positions` (a piece of the run after
    /// its heal: endpoints within the import tolerance of the run's
    /// vertices), in the run's order, as the kernel adopts them.
    fn narrow(&self, positions: &[Point3]) -> MeshExtra {
        let mut marked: BTreeSet<u32> = BTreeSet::new();
        for &p in positions {
            let c = Self::cell(p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &v in self
                            .grid
                            .get(&[c[0] + dx, c[1] + dy, c[2] + dz])
                            .into_iter()
                            .flatten()
                        {
                            if (self.points[v as usize] - p).length() <= Self::TOL {
                                marked.insert(v);
                            }
                        }
                    }
                }
            }
        }
        let mut keep: BTreeSet<u32> = BTreeSet::new();
        for &v in &marked {
            for &ci in &self.incident[v as usize] {
                let (a, b) = self.claims[ci as usize].ends();
                if marked.contains(&a) && marked.contains(&b) {
                    keep.insert(ci);
                }
            }
        }
        let mut extra = MeshExtra::default();
        for ci in keep {
            let at = |v: u32| self.points[v as usize];
            match &self.claims[ci as usize] {
                Claim::Soft(a, b) => extra.soft_edges.push([at(*a), at(*b)]),
                Claim::Curve(a, b, g) => extra.edge_curves.push(([at(*a), at(*b)], *g)),
            }
        }
        extra.surfaces = self.surfaces.clone();
        extra
    }
}

struct LoosePart<'a> {
    faces: &'a BTreeSet<usize>,
    tags: Vec<Vec<String>>,
    hidden: bool,
}

/// A loose mesh's faces by (layer, hidden): each connected piece (faces
/// sharing a vertex) goes under its layer — `None` when its faces mix
/// layers — and whether every one of its faces is hidden. `None` when the
/// whole mesh is one visible kind, and is read whole.
#[allow(clippy::type_complexity)]
fn loose_pieces(mesh: &openskp::Mesh) -> Option<BTreeMap<(Option<u16>, bool), BTreeSet<usize>>> {
    // Union-find over vertices, joined through each face's rings.
    let mut parent: Vec<usize> = (0..mesh.vertices.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for f in &mesh.faces {
        let mut ring = f.outer.iter().chain(f.holes.iter().flatten());
        let Some(&first) = ring.next() else { continue };
        for &v in ring {
            let (a, b) = (
                find(&mut parent, first as usize),
                find(&mut parent, v as usize),
            );
            parent[a] = b;
        }
    }
    // Each piece: its one layer (or none) and whether all of it is hidden.
    let mut pieces: BTreeMap<usize, (Option<u16>, bool, Vec<usize>)> = BTreeMap::new();
    for (i, f) in mesh.faces.iter().enumerate() {
        let Some(&v) = f.outer.first() else { continue };
        let root = find(&mut parent, v as usize);
        let p = pieces
            .entry(root)
            .or_insert((Some(f.layer), true, Vec::new()));
        if p.0 != Some(f.layer) {
            p.0 = None;
        }
        p.1 &= f.hidden;
        p.2.push(i);
    }
    let mut out: BTreeMap<(Option<u16>, bool), BTreeSet<usize>> = BTreeMap::new();
    for (layer, hidden, faces) in pieces.into_values() {
        out.entry((layer, hidden)).or_default().extend(faces);
    }
    let whole = out.len() == 1 && out.keys().next().is_some_and(|&(_, hidden)| !hidden);
    (!whole).then_some(out)
}

/// Healed (or split-piece) parallel arrays -> a `MeshRecipe`, fitting the
/// per-face affine UV frames from the corner UVs (same as dae-import).
#[allow(clippy::too_many_arguments)]
fn recipe_from_arrays(
    name: String,
    positions: Vec<Point3>,
    faces: Vec<Vec<usize>>,
    face_materials: Vec<u32>,
    corner_uvs: Vec<Vec<[f64; 2]>>,
    face_holes: Vec<Vec<Vec<usize>>>,
    base_material: u32,
    tags: Vec<Vec<String>>,
    extra: MeshExtra,
) -> MeshRecipe {
    let face_uv_frames: Vec<Option<UvFrame>> = faces
        .iter()
        .zip(corner_uvs.iter())
        .map(|(face, uvs)| {
            if uvs.len() == face.len() && uvs.len() >= 3 {
                let corner_pos: Vec<Point3> = face.iter().map(|&vi| positions[vi]).collect();
                fit_uv_frame(&corner_pos, uvs)
            } else {
                None
            }
        })
        .collect();
    MeshRecipe {
        name,
        positions,
        faces,
        face_materials,
        face_uv_frames,
        face_holes,
        base_material,
        tags,
        extra,
    }
}

/// The uniform scale of a similarity transform (rotation, reflection,
/// uniform scale, translation), or `None` for any other: circle claims map
/// only under a similarity.
fn uniform_scale(t: &Transform) -> Option<f64> {
    let a = t.to_affine();
    let col = |c: usize| Vec3::new(a[c], a[4 + c], a[8 + c]);
    let (x, y, z) = (col(0), col(1), col(2));
    let s = x.length();
    let tol = 1e-9 * s.max(1.0);
    let similar = s > 0.0
        && (y.length() - s).abs() <= tol
        && (z.length() - s).abs() <= tol
        && x.dot(y).abs() <= tol * s
        && y.dot(z).abs() <= tol * s
        && z.dot(x).abs() <= tol * s;
    similar.then_some(s)
}

/// A placement's 13-f64 transform (§4e: the 3×3 by rows, then the
/// translation in inches) as a row-major 4×4 in metres.
fn matrix_of(t: &[f64; 13]) -> [f64; 16] {
    [
        t[0],
        t[1],
        t[2],
        t[9] / openskp::INCH,
        t[3],
        t[4],
        t[5],
        t[10] / openskp::INCH,
        t[6],
        t[7],
        t[8],
        t[11] / openskp::INCH,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// A composed row-major 4×4 world matrix (metres) -> kernel `Transform`:
/// its first 12 entries are exactly `from_affine`'s row-major 3×4 layout.
fn pose_of(world: &[f64; 16]) -> Transform {
    let rows: [f64; 12] = world[0..12].try_into().expect("4x4 has 12 affine entries");
    Transform::from_affine(&rows)
}

/// Rebase a converted subtree's ABSOLUTE poses into a definition's local
/// frame (`inv` = the definition placement's inverse world pose). Instances
/// compose; groups carry no transform, so they recurse. Meshes are
/// def-local already: a component's own geometry is, and a SketchUp
/// GROUP's own (`group_node`) bakes straight into the frame of the
/// definition it converts under.
fn rebase_to_local(node: &mut ImportNode, inv: &Transform) {
    match node {
        ImportNode::Instance { pose, .. } => *pose = pose.then(inv),
        ImportNode::Group { children, .. } => {
            for c in children {
                rebase_to_local(c, inv);
            }
        }
        ImportNode::Mesh(_) => {}
    }
}

/// The importer key of a placed object: its persistent id, the identity a
/// scene's hidden-entity list names. 0 means the reader could not resolve
/// one (the 2017 legacy byte-scan path), and such a node carries no key.
fn key_of(pid: u32) -> Option<u64> {
    (pid != 0).then_some(u64::from(pid))
}

/// The part of a style Hew shows (`kernel::ViewStyle`): the backdrop
/// and whether the model axes are drawn.
fn view_style_of(r: &openskp::RenderingOptions) -> kernel::ViewStyle {
    let rgb = |c: [u8; 4]| [c[0], c[1], c[2]];
    kernel::ViewStyle {
        background: rgb(r.background),
        sky: r.sky.then(|| rgb(r.sky_color)),
        ground: r.ground.then(|| rgb(r.ground_color)),
        axes: r.model_axes,
    }
}

/// The file's camera as Hew's: a parallel view keeps the height
/// SketchUp framed it at, whatever the eye distance; a perspective view
/// takes the height it spans at the target (the file's stored height is
/// stale under perspective — SketchUp writes its default there), unless
/// Hew wrote the file, whose perspective cameras store the height they
/// mean.
fn camera_state(c: &openskp::Camera, hew_written: bool) -> CameraState {
    let m = |v: [f64; 3]| Point3::new(v[0], v[1], v[2]);
    let (eye, target) = (m(c.eye_m), m(c.target_m));
    let h = c.parallel_height_m;
    let parallel_height = if (!c.perspective || hew_written) && h.is_finite() && h > 0.0 {
        h
    } else {
        CameraState::height_at_target(eye, target, c.fov_deg)
    };
    CameraState {
        projection: if c.perspective {
            CameraProjection::Perspective
        } else {
            CameraProjection::Parallel
        },
        fov_deg: c.fov_deg,
        parallel_height,
        eye,
        target,
        up: Vec3::new(c.up[0], c.up[1], c.up[2]),
    }
}

/// The world pose of every geometry run's FIRST placement in the composed
/// tree, by run index (the root run is not placed and is absent).
fn first_placements(tree: &[openskp::Node]) -> BTreeMap<usize, Transform> {
    let mut first_pose: BTreeMap<usize, Transform> = BTreeMap::new();
    fn visit(nodes: &[openskp::Node], out: &mut BTreeMap<usize, Transform>) {
        for n in nodes {
            if let Some(ri) = n.run {
                out.entry(ri).or_insert_with(|| pose_of(&n.world));
            }
            visit(&n.children, out);
        }
    }
    visit(tree, &mut first_pose);
    first_pose
}

/// Every guide SketchUp draws, as a world-frame Hew guide. A guide among
/// the model's own entities is already world; a guide inside a definition
/// is stored in that definition's local frame and appears once per shown
/// placement of the definition, carried through the placement's world pose
/// (a definition that is never placed contributes nothing, as its geometry
/// doesn't either). A guide SketchUp does not draw stays out: one hidden,
/// one on a hidden layer, or one inside a placement that is hidden or on a
/// hidden layer (a camera rig's field-of-view lines, say) has no Hew guide
/// state to hide in, and a save back keeps the file's own. The legacy
/// byte-scan path reports no owner and its guides pass through as world
/// guides.
/// A `.skp` guide line's stretch as Hew holds it: the file's bounds are
/// line parameters in inches along the guide's own direction, ±1e30 for
/// an end that runs on forever (a whole line, or a ray from the Tape
/// Measure); Hew's are meters along the normalized direction, so each
/// finite bound is scaled by how long the direction is once posed.
fn guide_span(bounds: [f64; 2], direction_length: f64) -> kernel::GuideSpan {
    const OPEN: f64 = 1.0e29;
    let end = |b: f64| (b.abs() < OPEN).then_some(b / openskp::INCH * direction_length);
    kernel::GuideSpan {
        from: end(bounds[0]),
        to: end(bounds[1]),
    }
}

fn world_guides(
    model: &openskp::Model,
    tree: &[openskp::Node],
    layer_tags: &dyn Fn(u16) -> Vec<Vec<String>>,
) -> Vec<ImportGuide> {
    // definition map index -> every placement of it not hidden with Edit >
    // Hide (at any level), as its world pose and the layers of the chain
    // of containers down to it, outermost first.
    let mut placements: BTreeMap<usize, Vec<(Transform, Vec<u16>)>> = BTreeMap::new();
    fn visit(
        nodes: &[openskp::Node],
        model: &openskp::Model,
        chain: &mut Vec<u16>,
        out: &mut BTreeMap<usize, Vec<(Transform, Vec<u16>)>>,
    ) {
        for n in nodes {
            if n.hidden {
                continue;
            }
            chain.push(n.layer);
            if let Some(di) = n.run.and_then(|ri| model.geometry[ri].def_index) {
                out.entry(di)
                    .or_default()
                    .push((pose_of(&n.world), chain.clone()));
            }
            visit(&n.children, model, chain, out);
            chain.pop();
        }
    }
    visit(tree, model, &mut Vec::new(), &mut placements);
    // A guide's tags: its own layer first (what SketchUp shows for it, and
    // what a save back writes), then each container's above it, so it
    // shows and hides with the layers the way SketchUp shows it scene by
    // scene (a hidden layer is a hidden tag).
    let tags_through = |own: u16, chain: &[u16]| -> Vec<Vec<String>> {
        let mut tags = layer_tags(own);
        for &slot in chain {
            for t in layer_tags(slot) {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
        tags
    };

    let mut out = Vec::new();
    for g in &model.guides {
        if g.hidden {
            continue;
        }
        let origin = Point3::new(g.point_m[0], g.point_m[1], g.point_m[2]);
        let direction = Vec3::new(g.direction[0], g.direction[1], g.direction[2]);
        match g.def_index {
            None => out.push(ImportGuide::Line {
                origin,
                direction,
                span: guide_span(g.bounds, direction.length()),
                tags: tags_through(g.layer, &[]),
            }),
            Some(di) => {
                for (pose, chain) in placements.get(&di).map(Vec::as_slice).unwrap_or(&[]) {
                    let posed = pose.apply_vector(direction);
                    out.push(ImportGuide::Line {
                        origin: pose.apply_point(origin),
                        direction: posed,
                        span: guide_span(g.bounds, posed.length()),
                        tags: tags_through(g.layer, chain),
                    });
                }
            }
        }
    }
    // Guide points live in their entity list: the model's are world, a
    // definition's appear once per placement, like its guide lines.
    for run in &model.geometry {
        for gp in &run.guide_points {
            if gp.hidden {
                continue;
            }
            let p = Point3::new(gp.point_m[0], gp.point_m[1], gp.point_m[2]);
            match run.def_index {
                None => out.push(ImportGuide::Point {
                    position: p,
                    tags: tags_through(gp.layer, &[]),
                }),
                Some(di) => {
                    for (pose, chain) in placements.get(&di).map(Vec::as_slice).unwrap_or(&[]) {
                        out.push(ImportGuide::Point {
                            position: pose.apply_point(p),
                            tags: tags_through(gp.layer, chain),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Every section plane in the model, keyed by persistent id, as a world
/// frame Hew section state (metres): the run's local plane carried through
/// the world pose of the run's first placement (identity for the root
/// run). Planes whose placement pose is singular are left out.
///
/// A SketchUp plane's normal points at the side the cut keeps (a file's
/// own scene thumbnails show that side drawn); a Hew cut's normal points
/// at the side it removes, so the normal turns over.
fn section_planes_world(
    model: &openskp::Model,
    tree: &[openskp::Node],
) -> BTreeMap<u32, SectionPlaneState> {
    let first_pose = first_placements(tree);

    let mut out = BTreeMap::new();
    for (ri, run) in model.geometry.iter().enumerate() {
        let is_root = run.def_index.is_none();
        let pose = if is_root {
            Some(Transform::IDENTITY)
        } else {
            first_pose.get(&ri).copied()
        };
        let Some(pose) = pose else {
            continue;
        };
        for sp in &run.sections {
            let [a, b, c, d] = sp.plane;
            let n_local = Vec3::new(a, b, c);
            // A point on the plane: -D · n (inches -> metres).
            let p_local = Point3::new(
                -d * a / openskp::INCH,
                -d * b / openskp::INCH,
                -d * c / openskp::INCH,
            );
            // The normal goes through the plane transform (inverse
            // transpose), not the vector one: a non-uniformly scaled
            // placement would otherwise tilt it off the transformed plane.
            // It names the side the cut keeps, so it maps keeping that
            // side: a mirrored placement must not swap the kept half.
            let Ok(local_plane) = Plane::from_point_normal(p_local, n_local) else {
                continue;
            };
            let Ok(world_plane) = pose.apply_plane_keeping_side(&local_plane) else {
                continue;
            };
            out.insert(
                sp.pid,
                SectionPlaneState {
                    origin: pose.apply_point(p_local),
                    normal: -world_plane.normal(),
                    active: true,
                },
            );
        }
    }
    out
}

/// A linear dimension's world geometry from its record, given in the
/// frame `frame` places it by: the unit direction it measures along, its
/// offset vector, and its plane's normal. `normal` and `x_axis` are the
/// record's own (local) axes and `span` the WORLD vector between its two
/// anchors (variant 0 measures that span within the plane).
///
/// What each variant measures, and where its line sits, as the dimensions
/// SketchUp draws show: 1 along x, offset along n × x; 2 along n × x,
/// offset along -x; 0 the span's direction in the dimension's plane,
/// offset across it (n × that direction). Every one of these is a
/// direction in the LOCAL frame; the dimension line is the image of the
/// local one, so the offset maps as a plain vector (scaled by the frame,
/// on the same side of the measured line under a mirror) rather than being
/// rebuilt as a cross product of mapped axes, which a mirror would turn to
/// the other side. The plane normal maps as a normal, so it stays
/// perpendicular to the dimension's plane under any invertible frame.
fn dimension_geometry(
    frame: &Transform,
    variant: u32,
    normal: Vec3,
    x_axis: Vec3,
    span: Vec3,
    offset_m: f64,
) -> Option<(Vec3, Vec3, Vec3)> {
    let n0 = normal.normalized().ok()?;
    let x0 = x_axis.normalized().ok()?;
    let (along0, off0) = match variant {
        1 => (x0, n0.cross(x0)),
        2 => (n0.cross(x0), x0 * -1.0),
        0 => {
            let v = frame.inverse().ok()?.apply_vector(span);
            let u0 = (v - n0 * v.dot(n0)).normalized().ok()?;
            (u0, n0.cross(u0))
        }
        _ => return None,
    };
    let along = frame.apply_vector(along0).normalized().ok()?;
    let n = frame.apply_normal(n0).normalized().ok()?;
    Some((along, frame.apply_vector(off0) * offset_m, n))
}

/// A dimension record's text as a Hew override: none when empty, and a
/// leading `~` (Hew's mark for a rounded readout) dropped, so a file that
/// carries one never shows it as text the person typed.
fn dimension_override(text: &str) -> Option<String> {
    let t = text.strip_prefix('~').map_or(text, str::trim_start);
    (!t.is_empty()).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    /// The pre-mirror-fix construction (axes mapped as vectors, then
    /// crossed), kept to pin that an identity frame gives bit-identical
    /// results.
    fn dimension_geometry_old(
        frame: &Transform,
        variant: u32,
        normal: Vec3,
        x_axis: Vec3,
        span: Vec3,
        offset_m: f64,
    ) -> (Vec3, Vec3, Vec3) {
        let n = frame.apply_vector(normal).normalized().unwrap();
        let x = frame.apply_vector(x_axis).normalized().unwrap();
        let (along, off) = match variant {
            1 => (x, n.cross(x)),
            2 => (n.cross(x), x * -1.0),
            _ => {
                let u = (span - n * span.dot(n)).normalized().unwrap();
                (u, n.cross(u))
            }
        };
        (along, off * offset_m, n)
    }

    #[test]
    fn dimension_offset_maps_as_a_vector_under_any_frame() {
        let n0 = Vec3::new(0.0, 0.0, 1.0);
        let x0 = Vec3::new(1.0, 0.0, 0.0);
        let span0 = Vec3::new(2.0, 0.5, 0.0);
        let off_m = 0.25;
        let close = |a: Vec3, b: Vec3| (a - b).length() < 1e-12;

        // Identity: bit-identical to the old construction.
        for variant in [0, 1, 2] {
            let new =
                dimension_geometry(&Transform::IDENTITY, variant, n0, x0, span0, off_m).unwrap();
            let old = dimension_geometry_old(&Transform::IDENTITY, variant, n0, x0, span0, off_m);
            assert_eq!(new, old, "variant {variant}");
        }

        // A mirrored group placement (and a rotated, unmirrored control):
        // the offset is the image of the local offset, L(n x x) * offset.
        let rot = Transform::rotation(Vec3::new(0.2, 1.0, 0.4), 0.9).unwrap();
        let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
            .then(&Transform::translation(Vec3::new(3.0, 0.0, 0.0)));
        // A non-orthogonal frame: a shear and a mirror.
        let sheared = Transform::from_affine(&[
            -1.0, 0.4, 0.0, 0.0, //
            0.0, 2.0, 0.3, 0.0, //
            0.0, 0.0, 1.5, 0.0,
        ]);
        for frame in [rot, mirror, sheared] {
            let span = frame.apply_vector(span0);
            for (variant, want) in [
                (1, n0.cross(x0)),
                (2, x0 * -1.0),
                (0, n0.cross(Vec3::new(1.0, 0.25, 0.0).normalized().unwrap())),
            ] {
                let (along, off, n) =
                    dimension_geometry(&frame, variant, n0, x0, span, off_m).unwrap();
                assert!(
                    close(off, frame.apply_vector(want) * off_m),
                    "variant {variant}"
                );
                // The plane normal is perpendicular to the dimension's plane.
                assert!(n.dot(along).abs() < 1e-12 && n.dot(off).abs() < 1e-12);
            }
            if frame.determinant() > 0.0 {
                // The unmirrored rigid control agrees with the old
                // construction (up to rounding).
                for variant in [0, 1, 2] {
                    let (a1, o1, n1) =
                        dimension_geometry(&frame, variant, n0, x0, span, off_m).unwrap();
                    let (a2, o2, n2) = dimension_geometry_old(&frame, variant, n0, x0, span, off_m);
                    assert!(close(o1, o2) && close(n1, n2));
                    assert!(close(a1, a2) || close(a1, a2 * -1.0));
                }
            }
        }
    }

    #[test]
    fn dimension_override_drops_a_leading_tilde() {
        assert_eq!(dimension_override(""), None);
        assert_eq!(dimension_override("~"), None);
        assert_eq!(dimension_override("1' 2\""), Some("1' 2\"".to_string()));
        assert_eq!(dimension_override("~3/16\""), Some("3/16\"".to_string()));
        assert_eq!(dimension_override("~ 0.33 m"), Some("0.33 m".to_string()));
        assert_eq!(dimension_override("about ~3"), Some("about ~3".to_string()));
    }

    use super::*;

    fn parallel(eye: [f64; 3], height: f64) -> openskp::Camera {
        openskp::Camera {
            eye_m: eye,
            target_m: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            perspective: false,
            fov_deg: 35.0,
            parallel_height_m: height,
            two_point_perspective: false,
            description: String::new(),
            undecoded: [1.0, 1000.0],
            aspect_ratio: 0.0,
            scale_2d: 1.0,
            offset_2d: [0.0, 0.0],
        }
    }

    #[test]
    fn a_parallel_view_keeps_sketchups_height() {
        // Eye 155 m up, 20 m of plan in view: both stay as the file has
        // them; the height is the view's, not the eye distance's.
        let s = camera_state(&parallel([0.0, 0.0, 155.0], 20.0), false);
        assert_eq!(s.eye, Point3::new(0.0, 0.0, 155.0));
        assert_eq!(s.parallel_height, 20.0);
        assert_eq!(s.fov_deg, 35.0);
        // A height the file does not give takes the perspective view's.
        let s = camera_state(&parallel([0.0, 0.0, 5.0], 0.0), false);
        assert!(
            (s.parallel_height - CameraState::height_at_target(s.eye, s.target, 35.0)).abs()
                < 1e-12
        );
    }

    /// A face-less circle on a tilted plane: one sketch on the plane
    /// through it, with its circle claim, not a chord per vertical plane.
    #[test]
    fn a_circle_off_the_axis_planes_is_one_sketch_with_its_claim() {
        let n = Vec3::new(0.0, 1.0, 1.0).normalized().unwrap();
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = n.cross(x);
        let r = 0.5;
        let sides = 8usize;
        let mut mesh = openskp::Mesh::default();
        for i in 0..sides {
            let t = i as f64 / sides as f64 * std::f64::consts::TAU;
            let p = Point3::ORIGIN + x * (r * t.cos()) + y * (r * t.sin());
            mesh.vertices.push([p.x, p.y, p.z]);
        }
        for i in 0..sides as u32 {
            mesh.edges.push(openskp::MeshEdge {
                pid: 10 + i,
                v0: i,
                v1: (i + 1) % sides as u32,
                curve: Some(0),
                ..Default::default()
            });
        }
        let run = openskp::GeometryRun {
            mesh,
            curves: vec![openskp::Curve {
                pid: 1,
                members: sides as u32,
                arc: Some(openskp::Arc {
                    center_m: [0.0, 0.0, 0.0],
                    normal: [n.x, n.y, n.z],
                    x_axis_m: [r, 0.0, 0.0],
                    y_axis_m: [y.x * r, y.y * r, y.z * r],
                    radius_m: r,
                    start_rad: 0.0,
                    end_rad: std::f64::consts::TAU,
                }),
            }],
            ..Default::default()
        };
        let mut model = openskp::Model::default();
        model.geometry.push(run);
        let out = convert(Rc::new(model.clone()));
        let sketches = &out.scene.extra.sketches;
        assert_eq!(sketches.len(), 1, "{sketches:?}");
        let sk = &sketches[0];
        assert_eq!(sk.segments.len(), sides);
        assert!(sk.plane.normal().cross(n).length() < 1e-9, "{:?}", sk.plane);
        let (members, claim) = &sk.curves[0];
        assert_eq!(members.len(), sides);
        let claim = claim.as_ref().expect("the circle claim rides along");
        assert!(claim.geom.center.approx_eq(Point3::ORIGIN, 1e-9));
        assert!((claim.geom.radius - r).abs() < 1e-12);
        // The kernel builds it.
        let mut doc = kernel::Document::new();
        let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
        assert!(
            report.sketches.iter().all(|s| s.is_some()),
            "{:?}",
            report.sketches
        );
    }

    /// A loose run read as several pieces (here a visible box and a hidden
    /// one) hands each piece the soft edges among its own vertices, not the
    /// run's whole list over again: a furnished model's loose geometry is
    /// thousands of pieces, and the whole list on each cost gigabytes.
    #[test]
    fn each_piece_of_a_loose_run_carries_only_its_own_soft_edges() {
        let mut mesh = openskp::Mesh::default();
        let add_box = |mesh: &mut openskp::Mesh, x0: f64, soft: usize, hidden: bool| {
            let base = mesh.vertices.len() as u32;
            let corners = [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ];
            for [x, y, z] in corners {
                mesh.vertices.push([x0 + x, y, z]);
            }
            let quads: [[u32; 4]; 6] = [
                [0, 3, 2, 1],
                [4, 5, 6, 7],
                [0, 1, 5, 4],
                [1, 2, 6, 5],
                [2, 3, 7, 6],
                [3, 0, 4, 7],
            ];
            for (k, q) in quads.iter().enumerate() {
                mesh.faces.push(openskp::MeshFace {
                    pid: base + 100 + k as u32,
                    outer: q.iter().map(|&i| base + i).collect(),
                    hidden,
                    ..Default::default()
                });
            }
            let edges: [[u32; 2]; 12] = [
                [0, 1],
                [1, 2],
                [2, 3],
                [3, 0],
                [4, 5],
                [5, 6],
                [6, 7],
                [7, 4],
                [0, 4],
                [1, 5],
                [2, 6],
                [3, 7],
            ];
            for (k, e) in edges.iter().enumerate() {
                mesh.edges.push(openskp::MeshEdge {
                    pid: base + 200 + k as u32,
                    v0: base + e[0],
                    v1: base + e[1],
                    soft: k < soft,
                    ..Default::default()
                });
            }
        };
        add_box(&mut mesh, 0.0, 4, false);
        add_box(&mut mesh, 5.0, 2, true);
        let mut model = openskp::Model::default();
        model.geometry.push(openskp::GeometryRun {
            mesh,
            ..Default::default()
        });
        let out = convert(Rc::new(model));
        let mut soft: Vec<(bool, usize)> = out
            .scene
            .roots
            .iter()
            .filter_map(|n| match n {
                ImportNode::Mesh(m) => Some((m.extra.hidden, m.extra.soft_edges.len())),
                _ => None,
            })
            .collect();
        soft.sort();
        assert_eq!(soft, vec![(false, 4), (true, 2)], "{:?}", out.warnings);
    }

    /// A coordinate that is not a number comes from a run the reader
    /// misread, never from SketchUp: the edge on it is left out and the
    /// open report says how many were.
    #[test]
    fn an_edge_with_a_coordinate_that_is_not_a_number_is_left_out() {
        let bytes = include_bytes!("../tests/fixtures/lines-group.skp");
        let mut model = openskp::Model::parse(bytes).unwrap();
        let plain = convert(Rc::new(model.clone()));
        let lines: usize = plain
            .scene
            .extra
            .sketches
            .iter()
            .map(|s| s.segments.len())
            .sum();
        assert!(lines > 0);
        // Spoil one vertex a bare edge uses (the group of lines: edges and
        // no face).
        let ri = model
            .geometry
            .iter()
            .position(|r| !r.mesh.edges.is_empty() && r.mesh.faces.is_empty())
            .expect("the run of lines alone");
        let run = &mut model.geometry[ri];
        let v = run.mesh.edges[0].v0 as usize;
        run.mesh.vertices[v] = [2.7e-316, 2.8e244, f64::NAN];
        let out = convert(Rc::new(model.clone()));
        let left: usize = out
            .scene
            .extra
            .sketches
            .iter()
            .map(|s| s.segments.len())
            .sum();
        assert!(
            left < lines,
            "the spoiled edge is left out: {left} of {lines}"
        );
        assert!(
            out.scene.extra.sketches.iter().all(|s| s
                .segments
                .iter()
                .flatten()
                .all(|p| { p.x.is_finite() && p.y.is_finite() && p.z.is_finite() })),
            "nothing non-finite reaches the kernel"
        );
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("coordinates that are not numbers")),
            "{:?}",
            out.warnings
        );
    }

    /// A component of lines alone (a wire-frame box, a light rig) is a
    /// definition of its drawing: it opens with its placement, its sketches
    /// owned by the definition, and the kernel keeps it with a closed loop
    /// of its edges as a loop with no face.
    #[test]
    fn a_component_of_lines_alone_opens_as_a_definition_of_its_drawing() {
        let bytes = include_bytes!("../tests/fixtures/lines-group.skp");
        let mut model = openskp::Model::parse(bytes).unwrap();
        let plain = convert(Rc::new(model.clone()));
        assert!(
            plain.scene.defs.is_empty(),
            "a group of lines is a group, not a definition"
        );
        for inst in model
            .instances
            .iter_mut()
            .filter(|i| i.is_group == Some(true))
        {
            inst.is_group = Some(false);
        }
        for run in &mut model.geometry {
            for p in run.placed.iter_mut().filter(|p| p.is_group) {
                p.is_group = false;
            }
        }
        let out = convert(Rc::new(model.clone()));
        assert_eq!(out.scene.defs.len(), 1, "{:?}", out.warnings);
        assert!(
            out.scene
                .roots
                .iter()
                .any(|n| matches!(n, ImportNode::Instance { def: 0, .. })),
            "the component is placed"
        );
        let owned: Vec<_> = out
            .scene
            .extra
            .sketches
            .iter()
            .filter(|s| s.def == Some(0))
            .collect();
        assert!(!owned.is_empty(), "its lines are the definition's sketches");
        assert!(
            !out.warnings.iter().any(|w| w.contains("left out")),
            "{:?}",
            out.warnings
        );
        let mut doc = kernel::Document::new();
        let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
        let cid = report.defs[0].expect("the definition survives");
        assert_eq!(doc.instance_ids().len(), 1, "its instance is placed");
        assert!(doc.component_ids().contains(&cid));
        for s in report.sketches.iter().flatten() {
            let sk = doc.sketch(*s).expect("live");
            assert!(
                sk.regions().values().all(|r| r.faceless),
                "a loop of bare edges has no face"
            );
        }
    }

    /// A guide's bounds (inches along its direction, ±1e30 open) open as
    /// its stretch in meters: half an inch is 0.0127 m, a ray keeps its
    /// open end, and a whole line stays whole.
    #[test]
    fn a_guides_bounds_open_as_its_stretch_in_meters() {
        let bytes = include_bytes!("../tests/fixtures/guide.skp");
        let mut model = openskp::Model::parse(bytes).unwrap();
        let span_of = |model: &openskp::Model| match convert(Rc::new(model.clone())).scene.guides[0]
        {
            ImportGuide::Line { span, .. } => span,
            ImportGuide::Point { .. } => panic!("a line"),
        };
        assert!(
            span_of(&model).is_infinite(),
            "the fixture's guide is a whole line"
        );
        model.guides[0].bounds = [0.0, 0.5];
        let s = span_of(&model);
        assert_eq!(s.from, Some(0.0));
        assert!((s.to.unwrap() - 0.0127).abs() < 1e-12, "{s:?}");
        model.guides[0].bounds = [-2.0, 1e30];
        let s = span_of(&model);
        assert!((s.from.unwrap() + 0.0508).abs() < 1e-12, "{s:?}");
        assert_eq!(s.to, None);
    }

    /// A guide shows and hides with its layer the way SketchUp shows it
    /// scene by scene: it opens tagged with the layer, hidden or not. One
    /// hidden with Edit > Hide stays out (a Hew guide has no hidden state).
    #[test]
    fn a_guide_opens_tagged_with_its_layer_and_a_hidden_one_stays_out() {
        let bytes = include_bytes!("../tests/fixtures/guide.skp");
        let mut model = openskp::Model::parse(bytes).unwrap();
        let tags_of = |g: &ImportGuide| match g {
            ImportGuide::Line { tags, .. } | ImportGuide::Point { tags, .. } => tags.clone(),
        };
        let plain = convert(Rc::new(model.clone()));
        assert_eq!(plain.scene.guides.len(), 1);
        assert!(
            tags_of(&plain.scene.guides[0]).is_empty(),
            "the default layer is no tag"
        );
        // Hidden with Edit > Hide.
        model.guides[0].hidden = true;
        assert_eq!(convert(Rc::new(model.clone())).scene.guides.len(), 0);
        model.guides[0].hidden = false;
        // On a hidden layer: in, carrying the layer's tag, which is hidden.
        model.layers.push(openskp::Layer {
            name: "Rig".to_string(),
            visible: false,
            rgba: [0, 0, 0, 255],
        });
        let slot = model.layer_links.iter().map(|(s, _)| *s).max().unwrap_or(0) + 1;
        model.layer_links.push((slot, model.layers.len() - 1));
        model.guides[0].layer = slot;
        let out = convert(Rc::new(model.clone()));
        assert_eq!(out.scene.guides.len(), 1);
        assert_eq!(tags_of(&out.scene.guides[0]), vec![vec!["Rig".to_string()]]);
        assert!(
            out.scene
                .tags
                .iter()
                .any(|t| t.path == vec!["Rig".to_string()] && t.hidden),
            "the layer is declared as a hidden tag"
        );
        let mut doc = kernel::Document::new();
        doc.ingest(out.scene, out.textures_missing).unwrap();
        let ids = doc.guide_ids();
        assert_eq!(ids.len(), 1);
        assert!(!doc.guide_shown(ids[0]), "hidden through its tag");
        doc.set_tag_hidden(vec!["Rig".to_string()], false);
        assert!(doc.guide_shown(ids[0]));
    }

    /// A dimension or label inside a group shows in SketchUp only while the
    /// group does: it arrives tagged with the group's layer behind its own,
    /// and a hidden group's annotations stay out.
    #[test]
    fn a_groups_annotations_hide_with_the_group() {
        let bytes = include_bytes!("../tests/fixtures/lines-group.skp");
        let mut model = openskp::Model::parse(bytes).unwrap();
        let labels = |out: &Output| -> Vec<(String, Vec<Vec<String>>)> {
            out.scene
                .extra
                .annotations
                .iter()
                .zip(&out.scene.extra.annotation_tags)
                .map(|(a, t)| {
                    let kind = match a {
                        kernel::Annotation::LeaderText { text, .. } => text.clone(),
                        kernel::Annotation::LinearDimension { .. } => "dim".to_string(),
                        _ => "other".to_string(),
                    };
                    (kind, t.clone())
                })
                .collect()
        };
        let plain = labels(&convert(Rc::new(model.clone())));
        assert_eq!(
            plain,
            vec![("dim".to_string(), vec![]), ("lines".to_string(), vec![])],
            "the fixture's group is on the default layer"
        );
        // The group moves to a hidden layer "Notes".
        model.layers.push(openskp::Layer {
            name: "Notes".to_string(),
            visible: false,
            rgba: [0, 0, 0, 255],
        });
        let slot = model.layer_links.iter().map(|(s, _)| *s).max().unwrap_or(0) + 1;
        model.layer_links.push((slot, model.layers.len() - 1));
        let group = model
            .instances
            .iter_mut()
            .find(|i| i.is_group == Some(true))
            .expect("the fixture's one group");
        group.layer = Some(slot);
        let tagged = labels(&convert(Rc::new(model.clone())));
        let notes = vec![vec!["Notes".to_string()]];
        assert_eq!(
            tagged,
            vec![
                ("dim".to_string(), notes.clone()),
                ("lines".to_string(), notes)
            ]
        );
        // Hidden group: its annotations are not shown, so they stay out.
        let group = model
            .instances
            .iter_mut()
            .find(|i| i.is_group == Some(true))
            .unwrap();
        group.hidden = Some(true);
        assert!(labels(&convert(Rc::new(model.clone()))).is_empty());
    }

    #[test]
    fn a_perspective_view_is_as_stored() {
        let mut c = parallel([0.0, 0.0, 155.0], 20.0);
        c.perspective = true;
        let s = camera_state(&c, false);
        assert_eq!((s.eye, s.fov_deg), (Point3::new(0.0, 0.0, 155.0), 35.0));
        // The file's parallel height is not a perspective view's: the
        // height is what this view spans at its target.
        assert!(
            (s.parallel_height - CameraState::height_at_target(s.eye, s.target, 35.0)).abs()
                < 1e-12
        );
        // A file Hew wrote stores the height its perspective camera means.
        let s = camera_state(&c, true);
        assert_eq!(s.parallel_height, 20.0);
    }
}
