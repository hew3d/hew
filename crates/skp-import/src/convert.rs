//! `openskp::Model` -> `kernel::ImportScene`.
//!
//! Mapping:
//! - each **definition geometry run** becomes a [`DefRecipe`] (one variant per
//!   inherited material, since SketchUp's default-material faces render with
//!   the placing instance's paint — Hew expresses that as the def mesh's
//!   `base_material`);
//! - the composed instance tree ([`openskp::Model::scene`]) becomes
//!   [`ImportNode::Instance`]s (leaves) and [`ImportNode::Group`]s (placements
//!   whose definition itself places children), with **absolute world poses**
//!   exactly like `dae-import`;
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
//! - guides map to [`ImportGuide`]s.
//!
//! Everything OpenSKP hands us is already metres; poses come from the
//! composed row-major world matrices, whose first 12 entries are exactly
//! `Transform::from_affine`'s layout.

use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    DefRecipe, ImportGuide, ImportNode, ImportScene, ImportTag, MeshRecipe, Point3, Transform,
    UvFrame, Vec3,
};
use mesh_heal::heal_mesh;
use mesh_heal::uv::fit_uv_frame;

use crate::material::MatTable;

pub(crate) struct Output {
    pub scene: ImportScene,
    pub textures_missing: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) fn convert(model: &openskp::Model) -> Output {
    let mats = MatTable::build(model);

    // Declared archive-map indexes that belong to component/group definitions;
    // geometry runs carrying one of these are definition runs, everything else
    // is loose world geometry.
    let def_indexes: BTreeSet<usize> = model
        .definition_links
        .iter()
        .map(|&(dr, _)| dr as usize)
        .collect();

    let mut cv = Converter {
        model,
        mats,
        def_variants: BTreeMap::new(),
        defs: Vec::new(),
        warnings: Vec::new(),
    };

    let mut roots: Vec<ImportNode> = Vec::new();

    // ── Loose world geometry ───────────────────────────────────────────────
    for (ri, run) in model.geometry.iter().enumerate() {
        let is_def = matches!(run.def_index, Some(di) if def_indexes.contains(&di));
        if is_def {
            continue;
        }
        for recipe in cv.mesh_recipes(ri, "Model".to_string(), 0) {
            roots.push(ImportNode::Mesh(recipe));
        }
    }

    // ── The composed instance tree ─────────────────────────────────────────
    // `scene()` maps the top-level instances in filter order; zip against the
    // same filter to recover each root placement's own name (nested placements
    // have no names in the 2017 format — they fall back to their def's name).
    let top_names: Vec<Option<String>> = model
        .instances
        .iter()
        .filter(|i| {
            !model
                .geometry
                .iter()
                .any(|r| r.start <= i.offset && i.offset < r.end)
        })
        .map(|i| i.name.clone().filter(|n| !n.is_empty()))
        .collect();
    for (node, own_name) in model.scene().iter().zip(top_names) {
        if let Some(n) = cv.convert_node(node, own_name) {
            roots.push(n);
        }
    }

    // ── Guides ─────────────────────────────────────────────────────────────
    let guides: Vec<ImportGuide> = model
        .guides
        .iter()
        .map(|g| ImportGuide::Line {
            origin: Point3::new(g.point_m[0], g.point_m[1], g.point_m[2]),
            direction: Vec3::new(g.direction[0], g.direction[1], g.direction[2]),
        })
        .collect();

    // ── Parse anomalies -> user-visible warnings ───────────────────────────
    // Clean 2017 files parse with zero desync diagnostics (an OpenSKP
    // regression guarantee); anything else means content may be missing and
    // is said out loud, never papered over (rule 4 spirit; fixes go upstream).
    // Converter-emitted warnings (non-manifold splits) come first — they are
    // per-object and actionable. Desync notes AGGREGATE past a handful: a
    // badly damaged file can desync tens of thousands of times, and 32k
    // warning strings help nobody (a damaged production model produced 32k).
    const DESYNC_DETAIL_CAP: usize = 8;
    let mut warnings = std::mem::take(&mut cv.warnings);
    let desyncs: Vec<&openskp::Diagnostic> =
        model.diagnostics.iter().filter(|d| d.is_desync()).collect();
    warnings.extend(
        desyncs
            .iter()
            .take(DESYNC_DETAIL_CAP)
            .map(|d| format!("parser recovered from a malformed section: {d:?}")),
    );
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
            path: vec![l.name.clone()],
            hidden: !l.visible,
        })
        .collect();

    let Converter { mats, defs, .. } = cv;
    Output {
        scene: ImportScene {
            materials: mats.materials,
            defs,
            roots,
            guides,
            tags,
        },
        textures_missing: mats.textures_missing,
        warnings,
    }
}

struct Converter<'a> {
    model: &'a openskp::Model,
    mats: MatTable,
    /// (geometry-run index, inherited material slot) -> def index in `defs`
    /// (`None` = the run has no importable faces at that variant).
    def_variants: BTreeMap<(usize, u16), Option<usize>>,
    defs: Vec<DefRecipe>,
    /// User-visible conversion warnings (non-manifold splits — rule 4:
    /// decomposition happens loudly, never silently).
    warnings: Vec<String>,
}

impl Converter<'_> {
    /// One composed scene node -> an import node, `None` when pruned
    /// (hidden instance, hidden layer, or nothing importable below).
    fn convert_node(
        &mut self,
        node: &openskp::Node,
        own_name: Option<String>,
    ) -> Option<ImportNode> {
        // NOTHING visibility-related is dropped: a hidden INSTANCE
        // (per-entity hide) imports as a user-hidden node (persisted view
        // state, manifest v6), and hidden-LAYER content imports carrying
        // its layer tag whose hidden-by-default flag keeps it invisible
        // until the user shows the tag. Hidden is visibility state, not
        // existence.
        let hidden = node.hidden;

        let pose = pose_of(&node.world);
        let tags = self.layer_tags(node.layer);

        // The placement's own geometry (a def variant keyed by the inherited
        //q material, so default-painted faces pick up the instance paint).
        let own_instance = node
            .run
            .and_then(|ri| self.def_variant(ri, node.material))
            .map(|def| ImportNode::Instance {
                def,
                pose,
                name: own_name.clone(),
                tags: tags.clone(),
                hidden,
            });

        // Children (in-definition placements), expanded per placement site
        // with their absolute world poses.
        let children: Vec<ImportNode> = node
            .children
            .iter()
            .filter_map(|c| self.convert_node(c, None))
            .collect();

        match (own_instance, children.is_empty()) {
            (own, false) => {
                let name = own_name
                    .or_else(|| node.definition.clone())
                    .unwrap_or_default();
                Some(ImportNode::Group {
                    name,
                    children: own.into_iter().chain(children).collect(),
                    tags,
                    hidden,
                })
            }
            (own, true) => own,
        }
    }

    /// Def index for `(run, inherited material)`, building the `DefRecipe`
    /// variant on first use. A non-manifold run splits into several meshes
    /// under ONE definition, so every placement keeps all its pieces.
    fn def_variant(&mut self, run_idx: usize, eff_slot: u16) -> Option<usize> {
        if let Some(&cached) = self.def_variants.get(&(run_idx, eff_slot)) {
            return cached;
        }
        let name = self.def_name(run_idx);
        let meshes = self.mesh_recipes(run_idx, name.clone().unwrap_or_default(), eff_slot);
        let built = if meshes.is_empty() {
            None
        } else {
            self.defs.push(DefRecipe { name, meshes });
            Some(self.defs.len() - 1)
        };
        self.def_variants.insert((run_idx, eff_slot), built);
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
    /// Usually one recipe. A NON-MANIFOLD run (which `from_polygons` would
    /// reject whole) is decomposed by [`mesh_heal::split::split_non_manifold`]
    /// into several open-shell recipes — loudly (a warning names the mesh
    /// and the piece count), never silently (rule 4).
    fn mesh_recipes(&mut self, run_idx: usize, name: String, eff_slot: u16) -> Vec<MeshRecipe> {
        let mesh = &self.model.geometry[run_idx].mesh;

        let positions: Vec<Point3> = mesh
            .vertices
            .iter()
            .map(|v| Point3::new(v[0], v[1], v[2]))
            .collect();

        let mut faces: Vec<Vec<usize>> = Vec::new();
        let mut face_mats: Vec<u32> = Vec::new();
        let mut corner_uvs: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut holes: Vec<Vec<Vec<usize>>> = Vec::new();

        for f in &mesh.faces {
            // Faces always import — hidden faces and hidden-layer faces
            // included. Dropping them would open closed solids (a hidden face
            // on a box is display state, not geometry), and SketchUp's own
            // exports keep them (the  `.dae` ground truth: layers.dae
            // ships its hidden-layer box; hidden-entities.dae is watertight).
            // Visibility pruning applies at the INSTANCE level only, matching
            // theq export rule.
            // Front side wins; a back-only paint still beats no material.
            let own_slot = f.front_material.or(f.back_material).unwrap_or(0);
            let uv_side = if f.front_material.is_some() || f.back_material.is_none() {
                openskp::Side::Front
            } else {
                openskp::Side::Back
            };
            face_mats.push(if own_slot != 0 {
                self.mats.dense(own_slot)
            } else {
                kernel::NO_MATERIAL
            });

            // Corner UVs whenever the face's effective material is textured:
            // an explicit placement uses its CFaceTextureCoords, a painted
            // side without one uses SketchUp's identity placement — both via
            // `uv_xform`.
            let tex_slot = if own_slot != 0 { own_slot } else { eff_slot };
            let uvs: Vec<[f64; 2]> = if self.mats.is_textured(tex_slot) {
                let size = self.mats.applied_size_in(tex_slot).unwrap_or((1.0, 1.0));
                match f.uv_xform(uv_side, size) {
                    Some(x) => f
                        .outer
                        .iter()
                        .map(|&vi| x.apply(mesh.vertices[vi as usize]))
                        .collect(),
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };

            faces.push(f.outer.iter().map(|&vi| vi as usize).collect());
            holes.push(
                f.holes
                    .iter()
                    .map(|ring| ring.iter().map(|&vi| vi as usize).collect())
                    .collect(),
            );
            corner_uvs.push(uvs);
        }

        if faces.is_empty() {
            return Vec::new();
        }

        // Native tolerances: `.skp` coordinates are exact f64, like COLLADA
        // text (the glTF f32 relaxation does not apply).
        let (positions, faces, healed_mats, healed_uvs, healed_holes) = heal_mesh(
            &positions,
            &faces,
            &face_mats,
            &corner_uvs,
            &holes,
            &Transform::IDENTITY,
        );
        if faces.is_empty() {
            return Vec::new();
        }

        let tags = self.mesh_layer_tags(run_idx);
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
            &healed_mats,
            &healed_uvs,
            &healed_holes,
        ) {
            self.warnings.push(format!(
                "'{}' is non-manifold; imported as {} open shell{} \
                 (split at non-manifold edges, geometry unchanged)",
                if name.is_empty() {
                    "unnamed mesh"
                } else {
                    &name
                },
                pieces.len(),
                if pieces.len() == 1 { "" } else { "s" },
            ));
            return pieces
                .into_iter()
                .map(|piece| {
                    recipe_from_arrays(
                        name.clone(),
                        piece.positions,
                        piece.faces,
                        piece.face_materials,
                        piece.face_corner_uvs,
                        piece.face_holes,
                        base_material,
                        tags.clone(),
                    )
                })
                .collect();
        }

        vec![recipe_from_arrays(
            name,
            positions,
            faces,
            healed_mats,
            healed_uvs,
            healed_holes,
            base_material,
            tags,
        )]
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
            .map(|l| vec![vec![l.name.clone()]])
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
    }
}

/// A composed row-major 4×4 world matrix (metres) -> kernel `Transform`:
/// its first 12 entries are exactly `from_affine`'s row-major 3×4 layout.
fn pose_of(world: &[f64; 16]) -> Transform {
    let rows: [f64; 12] = world[0..12].try_into().expect("4x4 has 12 affine entries");
    Transform::from_affine(&rows)
}
