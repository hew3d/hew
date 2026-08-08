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
        degraded_variants: BTreeMap::new(),
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
        for recipe in cv.mesh_recipes(ri, "Model".to_string(), 0, &Transform::IDENTITY) {
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
    /// Variants whose nested children were DROPPED because the first
    /// placement's world matrix was singular (uninvertible — the def-local
    /// rebase is impossible from that placement). A later placement with an
    /// invertible matrix upgrades the cached recipe in place, so one
    /// zero-scaled placement can't permanently strip a shared definition.
    degraded_variants: BTreeMap<(usize, u16), usize>,
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

        match node.run {
            // A SketchUp GROUP placement (`is_group == Some(true)`): groups
            // are independent containers, not shared definitions — each
            // placement converts on its own (deep copy, no shared def).
            Some(ri) if matches!(node.is_group, Some(true)) => {
                self.group_node(ri, node, own_name, tags, hidden)
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
                Some(ImportNode::Instance {
                    def,
                    pose,
                    name: own_name,
                    tags,
                    hidden,
                })
            }
            // A pure container with no geometry run of its own: a world
            // group wrapping its converted children (world poses).
            None => {
                let children: Vec<ImportNode> = node
                    .children
                    .iter()
                    .filter_map(|c| self.convert_node(c, None))
                    .collect();
                if children.is_empty() {
                    return None;
                }
                let name = own_name
                    .or_else(|| node.definition.clone())
                    .unwrap_or_default();
                Some(ImportNode::Group {
                    name,
                    children,
                    tags,
                    hidden,
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
    /// The node's own run bakes `pose_of(&node.world)` into its mesh
    /// positions (group geometry is def-local in the source; a Hew
    /// `ImportNode::Mesh` carries no pose). When this group turns up nested
    /// inside a component definition, `assembly_variant`'s `rebase_to_local`
    /// composes the definition's inverse pose on top afterward, landing the
    /// already-world-baked positions in def-local coordinates — the same
    /// two-step composition `rebase_to_local` already does for `Instance`
    /// poses.
    fn group_node(
        &mut self,
        run_idx: usize,
        node: &openskp::Node,
        own_name: Option<String>,
        tags: Vec<Vec<String>>,
        hidden: bool,
    ) -> Option<ImportNode> {
        let name = own_name
            .or_else(|| node.definition.clone())
            .unwrap_or_default();
        let bake = pose_of(&node.world);
        let mut own_meshes = self.mesh_recipes(run_idx, name.clone(), node.material, &bake);
        let children: Vec<ImportNode> = node
            .children
            .iter()
            .filter_map(|c| self.convert_node(c, None))
            .collect();

        // The "simple solid" special case: SketchUp needs a wrapper group to
        // isolate geometry; Hew's solids-first model doesn't. Skipped when
        // the group itself is hidden — `MeshRecipe` carries no hidden flag
        // of its own, so collapsing would silently drop the group's
        // visibility state (rule 4 spirit: never lose it quietly).
        if children.is_empty() && own_meshes.len() == 1 && !hidden {
            let mut mesh = own_meshes.pop().expect("len checked above");
            mesh.tags = tags;
            return Some(ImportNode::Mesh(mesh));
        }

        let mut all_children: Vec<ImportNode> =
            own_meshes.into_iter().map(ImportNode::Mesh).collect();
        all_children.extend(children);
        if all_children.is_empty() {
            return None;
        }
        Some(ImportNode::Group {
            name,
            children: all_children,
            tags,
            hidden,
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
                let children: Vec<ImportNode> = node
                    .children
                    .iter()
                    .filter_map(|c| self.convert_node(c, None))
                    .map(|mut n| {
                        rebase_to_local(&mut n, &inv);
                        n
                    })
                    .collect();
                if !children.is_empty() {
                    self.defs[di].children.extend(children);
                    self.degraded_variants.remove(&(run_idx, eff_slot));
                }
            }
            return cached;
        }
        self.def_variants.insert((run_idx, eff_slot), None);
        let name = self.def_name(run_idx);
        let meshes = self.mesh_recipes(
            run_idx,
            name.clone().unwrap_or_default(),
            eff_slot,
            &Transform::IDENTITY,
        );
        let mut children: Vec<ImportNode> = meshes.into_iter().map(ImportNode::Mesh).collect();
        let mut degraded = false;
        // Child placements rebase to the definition's local frame:
        // local = world_def⁻¹ ∘ world_child. A singular world pose cannot
        // be rebased — its children are dropped loudly, never guessed at.
        match pose_of(&node.world).inverse() {
            Ok(inv) => {
                for c in &node.children {
                    if let Some(mut n) = self.convert_node(c, None) {
                        rebase_to_local(&mut n, &inv);
                        children.push(n);
                    }
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
        let built = if children.is_empty() {
            None
        } else {
            self.defs.push(DefRecipe { name, children });
            Some(self.defs.len() - 1)
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
    ) -> Vec<MeshRecipe> {
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
        let (positions, faces, healed_mats, healed_uvs, healed_holes) =
            heal_mesh(&positions, &faces, &face_mats, &corner_uvs, &holes, bake);
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

/// Rebase a converted subtree's ABSOLUTE poses into a definition's local
/// frame (`inv` = the definition placement's inverse world pose). Instances
/// compose; groups carry no transform, so they recurse. Meshes are usually
/// already def-local (a component's own geometry never reaches this
/// function's `Mesh` arm) — the exception is a SketchUp GROUP's own meshes
/// (`group_node`), baked to WORLD at conversion time; nested inside a
/// definition, `inv` composes on top to finish the job, landing them in
/// def-local coordinates.
fn rebase_to_local(node: &mut ImportNode, inv: &Transform) {
    match node {
        ImportNode::Instance { pose, .. } => *pose = pose.then(inv),
        ImportNode::Group { children, .. } => {
            for c in children {
                rebase_to_local(c, inv);
            }
        }
        ImportNode::Mesh(m) => rebase_mesh(m, inv),
    }
}

/// Apply `inv` to a mesh recipe's positions, refitting each face's UV frame
/// so the position <-> UV relationship it already renders survives bit-for-
/// bit: the new corner UVs are read off the OLD frame at the OLD positions
/// (the frame's actual current prediction, not a re-derivation of SketchUp's
/// raw values), then `fit_uv_frame` refits against the NEW (rebased)
/// corners — the same fit `recipe_from_arrays` used to build the frame in
/// the first place, just re-run in the new coordinate space.
fn rebase_mesh(m: &mut MeshRecipe, inv: &Transform) {
    let new_positions: Vec<Point3> = m.positions.iter().map(|&p| inv.apply_point(p)).collect();
    for (face, frame) in m.faces.iter().zip(m.face_uv_frames.iter_mut()) {
        if let Some(f) = frame {
            let uvs: Vec<[f64; 2]> = face.iter().map(|&vi| f.apply(m.positions[vi])).collect();
            let new_corners: Vec<Point3> = face.iter().map(|&vi| new_positions[vi]).collect();
            *frame = fit_uv_frame(&new_corners, &uvs);
        }
    }
    m.positions = new_positions;
}
