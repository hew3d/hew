//! Acceptance specs for `skp_import::import` (DEVELOPMENT.md rule 3).
//!
//! Fixtures under `tests/fixtures/` are frozen copies of OpenSKP's
//! self-authored corpus (`corpus/2017` + one legacy/future sample each) —
//! committed here so tests run anywhere the workspace builds. Ground truth
//! and corpus stewardship stay upstream in OpenSKP; refresh fixtures when the
//! rev pin advances. Expected numbers below were validated against the
//! paired SketchUp `.dae` exports (see `differential.rs`).

use std::path::Path;

use kernel::{Document, ImportNode, Rgba8};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()))
}

/// Import a fixture and ingest it into a fresh document.
///
/// Clean corpus files never produce parser-RECOVERY warnings. Non-manifold
/// SPLIT notices are legitimate (rule 4's loud decomposition) and pass
/// through — specs that care about them assert on the report instead.
fn ingest(name: &str) -> (kernel::ImportReport, Document) {
    let out = skp_import::import(&fixture(name)).expect(name);
    assert!(
        !out.warnings.iter().any(|w| w.contains("parser recovered")),
        "{name}: clean corpus files parse without recovery warnings; got {:?}",
        out.warnings
    );
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    (report, doc)
}

// ── The probe stays useful ─────────────────────────────────────────────

#[test]
fn box_skp_probes_to_cube_topology() {
    let runs = skp_import::probe_topology(&fixture("box.skp"));
    assert!(
        runs.iter()
            .any(|t| t.vertices == 8 && t.edges == 12 && t.faces == 6),
        "no run had cube topology; got: {runs:?}"
    );
}

// ── Version gate ─────────────────────────────────────────────────────────────

#[test]
fn pre_2017_files_are_refused_with_their_version() {
    match skp_import::import(&fixture("box-v2013.skp")) {
        Err(skp_import::SkpError::UnsupportedVersion { version }) => {
            assert!(version.contains("13"), "got {version}");
        }
        other => panic!(
            "expected UnsupportedVersion, got {other:?}",
            other = other.is_ok()
        ),
    }
}

#[test]
fn post_2017_files_are_refused_with_their_version() {
    match skp_import::import(&fixture("box-v2026.skp")) {
        Err(skp_import::SkpError::UnsupportedVersion { version }) => {
            assert!(version.contains("26"), "got {version}");
            // The error copy must point at the escape hatch every modern
            // SketchUp has ( friendly-error hook).
            let msg = skp_import::SkpError::UnsupportedVersion { version }.to_string();
            assert!(
                msg.contains("Save As"),
                "copy should suggest Save As: {msg}"
            );
        }
        other => panic!(
            "expected UnsupportedVersion, got {other:?}",
            other = other.is_ok()
        ),
    }
}

#[test]
fn non_skp_bytes_are_refused() {
    assert!(matches!(
        skp_import::import(b"not a sketchup file at all"),
        Err(skp_import::SkpError::NotSkp)
    ));
}

// ── Loose geometry ───────────────────────────────────────────────────────────

#[test]
fn box_imports_as_one_watertight_world_object() {
    let (report, doc) = ingest("box.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert!(report.skipped.is_empty());
    // Loose model geometry is a world object, not a def/instance.
    assert_eq!(doc.instance_ids().len(), 0);
    assert_eq!(doc.group_ids().len(), 0);
}

#[test]
fn empty_model_imports_as_nothing() {
    let (report, doc) = ingest("empty.skp");
    assert_eq!(report.objects_created, 0);
    assert!(report.skipped.is_empty());
    assert_eq!(doc.guide_ids().len(), 0);
}

#[test]
fn unplaced_template_definitions_are_not_imported() {
    // Every corpus file carries the template's unplaced "Chris" figure
    // definition; it must not leak into the document.
    let out = skp_import::import(&fixture("box.skp")).unwrap();
    assert!(
        out.scene
            .defs
            .iter()
            .all(|d| d.name.as_deref() != Some("Chris")),
        "unplaced defs stay out"
    );
}

// ── Components, groups, nesting ──────────────────────────────────────────────

#[test]
fn two_instances_share_one_definition() {
    let (report, doc) = ingest("box-component-two-instances.skp");
    // ONE definition object, TWO placements — shared geometry, not copies.
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert_eq!(doc.instance_ids().len(), 2);
    let comp = doc.component_ids();
    assert_eq!(comp.len(), 1);
    assert_eq!(doc.component_name(comp[0]), Some("Box Component"));
}

#[test]
fn nested_components_keep_hierarchy_and_share_the_leaf_def() {
    // nested-3-deep: the model places the outer component; the chain bottoms
    // out in ONE geometric definition placed 4 times (2 × 2 in the middle
    // tier). Hierarchy comes through as NESTED definitions (manifest v15) —
    // one world instance, a 3-deep definition chain, geometry stays shared.
    let (report, doc) = ingest("nested-3-deep.skp");
    assert_eq!(report.objects_created, 1, "one shared leaf def");
    assert_eq!(report.watertight, 1);
    assert_eq!(doc.instance_ids().len(), 1, "one world placement");
    assert_eq!(doc.group_ids().len(), 0, "no flattened wrapper groups");
    assert_eq!(doc.component_ids().len(), 3, "the full definition chain");
    assert_eq!(
        doc.component_ids()
            .iter()
            .map(|&c| doc.def_depth(c))
            .max()
            .unwrap(),
        3,
        "the chain genuinely nests 3 deep"
    );
    assert_eq!(
        doc.expanded_placements().len(),
        4,
        "the shared leaf renders 4 times through the nesting"
    );
}

#[test]
fn groups_and_mixed_definitions_import_structurally() {
    // box-group.skp: a plain SketchUp GROUP wrapping a single box. A group
    // is not a shared definition (SketchUp groups are logically unique,
    // unlike components) — and a group with exactly one resulting mesh and
    // no children is the "simple solid" special case: SketchUp needs a
    // wrapper group to isolate geometry, Hew's solids-first model doesn't.
    // It imports as a bare world object, no instance/group/component at all.
    let (report, doc) = ingest("box-group.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert_eq!(doc.instance_ids().len(), 0);
    assert_eq!(doc.group_ids().len(), 0);
    assert_eq!(doc.component_ids().len(), 0);

    // mixed-definition.skp's outer wrapper is a genuine COMPONENT (not a
    // group) placing another component as a member — unaffected by the
    // group/component split.
    let (report, doc) = ingest("mixed-definition.skp");
    assert_eq!(report.objects_created, 2);
    assert_eq!(report.watertight, 2);
    // The mixed definition (own geometry + a child placement) is ONE
    // nested definition placed once — its child is a member instance.
    assert_eq!(doc.instance_ids().len(), 1);
    assert_eq!(doc.group_ids().len(), 0);
    assert_eq!(doc.component_ids().len(), 2);
    assert_eq!(doc.expanded_placements().len(), 2);
}

// ── Native names (no __HEWMETA__ hex dance — the joy of M25) ────────────────

#[test]
fn house_carries_native_group_and_component_names_with_layer_tags() {
    // "Front Wall" and "Slab" are genuine SketchUp GROUPS (single mesh, no
    // children each) — exactly the misclassification this mapping fix
    // corrects: they used to import as Component instances. Their native
    // name (no __HEWMETA__ hex dance) now lands on the collapsed bare
    // `Mesh` instead of on a wrapping `Instance`.
    let out = skp_import::import(&fixture("house.skp")).unwrap();

    fn walk<'a>(nodes: &'a [ImportNode], hits: &mut Vec<(&'a str, &'a [Vec<String>])>) {
        for n in nodes {
            match n {
                ImportNode::Mesh(m) if !m.name.is_empty() => {
                    hits.push((m.name.as_str(), m.tags.as_slice()))
                }
                ImportNode::Instance {
                    name: Some(name),
                    tags,
                    ..
                } => hits.push((name.as_str(), tags.as_slice())),
                ImportNode::Group { children, .. } => walk(children, hits),
                _ => {}
            }
        }
    }
    let mut named = Vec::new();
    walk(&out.scene.roots, &mut named);

    let front_wall = named
        .iter()
        .find(|(n, _)| *n == "Front Wall")
        .expect("'Front Wall' carries its native name onto the collapsed mesh");
    assert_eq!(
        front_wall.1,
        &[vec!["Exterior Walls".to_string()]],
        "the group's own layer arrives as a tag on the collapsed mesh"
    );
    assert!(named.iter().any(|(n, _)| *n == "Slab"));
}

#[test]
fn skp_group_containing_a_component_stays_a_group_the_component_stays_shared() {
    // The headline case this mapping fix is FOR: a SketchUp GROUP holding a
    // COMPONENT placement alongside its own geometry. house.skp's "Front
    // Entry Door" component nests both kinds side by side — "Front Entry
    // Door Frame" is a genuine group (own geometry only, no children) and
    // "Therma-Tru 36\" Door Slab" is a genuine component, placed inside it.
    // The group's own meshes land directly as members (no def of its own,
    // deep-copied — SketchUp groups are logically unique); the component
    // becomes a member `Instance` referencing ITS OWN shared `DefRecipe`.
    let out = skp_import::import(&fixture("house.skp")).unwrap();
    let front_entry_door = out
        .scene
        .defs
        .iter()
        .find(|d| d.name.as_deref() == Some("Front Entry Door"))
        .expect("house.skp has a 'Front Entry Door' component");

    let group_frame = front_entry_door.children.iter().find_map(|c| match c {
        ImportNode::Group { name, children, .. } => Some((name.as_str(), children.len())),
        _ => None,
    });
    assert_eq!(
        group_frame,
        Some(("Group#769", 6)),
        "the door frame group nests as a Group, not a shared def"
    );

    let slab_def = front_entry_door
        .children
        .iter()
        .find_map(|c| match c {
            ImportNode::Instance { def, .. } => Some(*def),
            _ => None,
        })
        .expect("the door slab component nests as a member Instance");
    assert_eq!(
        out.scene.defs[slab_def].name.as_deref(),
        Some("Therma-Tru 36\" Door Slab"),
        "the component stays a shared definition, not inlined geometry"
    );
}

#[test]
fn house_ingests_with_shared_defs_and_loud_skips() {
    let (report, doc) = ingest("house.skp");
    // Frozen regression numbers (validated against house.dae, which flattens
    // to 69 baked objects with 34 leaky shells — the .skp path keeps shared
    // definitions and watertight solids instead). house's two genuinely
    // non-manifold source meshes DECOMPOSE into open shells (loud split
    // warnings) instead of being rejected: 49 watertight solids + 3 leaky
    // pieces, nothing skipped.
    assert_eq!(report.objects_created, 52);
    assert_eq!(report.watertight, 49);
    assert_eq!(report.leaky, 3);
    // Group/component split (the corrected mapping): most of what the old
    // (pre-`is_group`) heuristic counted as "shared definitions nesting 4
    // deep" were actually SketchUp GROUPS (Front Wall, Slab, the window and
    // door frame internals, ...) — logically unique per placement, not
    // shared. Only the 6 genuine components remain as DefRecipes (Win-Dor
    // OXXO Sliding Door, Double Slider Window, Therma-Tru 36" Door Slab,
    // Front Entry Door, Table Top, "3/4 Birch Plywood"), nesting 2 deep
    // (Front Entry Door -> its door slab). Groups deep-copy instead of
    // sharing, so their member geometry counts once PER PLACEMENT — hence
    // `objects_created` rising even though fewer objects are shared
    // through defs (conservation still holds: validated against house.dae
    // world-space totals in the differential suite). No wrapper groups
    // reach the WORLD level either (every group that isn't a component
    // ends up owned by a definition, or collapses to a bare mesh).
    assert_eq!(doc.instance_ids().len(), 6);
    assert_eq!(doc.group_ids().len(), 0);
    assert_eq!(doc.component_ids().len(), 6);
    assert_eq!(doc.expanded_placements().len(), 56);
    assert_eq!(
        doc.component_ids()
            .iter()
            .map(|&c| doc.def_depth(c))
            .max()
            .unwrap(),
        2
    );
    assert_eq!(report.skipped.len(), 0);
    // Nothing missing: the one material without inline image bytes
    // ("[Wood Floor Light]1") is a shared-texture record whose back-ref
    // OpenSKP (0.2.0) resolves to the owning material's bytes.
    assert_eq!(report.textures_missing, Vec::<String>::new());
}

// ── Materials ────────────────────────────────────────────────────────────────

#[test]
fn textured_material_arrives_with_embedded_image_and_uv_frames() {
    let out = skp_import::import(&fixture("material-one-face.skp")).unwrap();
    let textured: Vec<_> = out
        .scene
        .materials
        .iter()
        .filter(|m| m.texture.is_some())
        .collect();
    assert_eq!(textured.len(), 1);
    let tex = textured[0].texture.as_ref().unwrap();
    assert!(!tex.image.is_empty(), "image bytes embedded in the .skp");
    assert!(tex.world_size[0] > 0.0 && tex.world_size[1] > 0.0);

    // The painted face carries a fitted UV frame.
    let has_uv_frame = out.scene.roots.iter().any(|n| match n {
        ImportNode::Mesh(m) => m.face_uv_frames.iter().any(Option::is_some),
        _ => false,
    });
    assert!(has_uv_frame, "painted face gets a per-face UV frame");
    assert!(out.textures_missing.is_empty());
}

#[test]
fn shared_texture_and_textured_opacity_survive_import() {
    let out = skp_import::import(&fixture("house.skp")).unwrap();
    let find = |name: &str| {
        out.scene
            .materials
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("house has material {name:?}"))
    };

    // "[Wood Floor Light]1" is a shared-texture record: a back-reference to
    // "[Wood Floor Light]"'s image with no inline bytes of its own. OpenSKP
    // 0.2.0 resolves the back-ref, so the sharing material imports textured
    // with the owning material's exact bytes instead of falling back to its
    // average color (and nothing lands in textures_missing).
    let owner_tex = find("[Wood Floor Light]")
        .texture
        .as_ref()
        .expect("owning material is textured")
        .clone();
    let shared_tex = find("[Wood Floor Light]1")
        .texture
        .as_ref()
        .expect("shared-texture record resolves to a texture");
    assert_eq!(
        shared_tex.image, owner_tex.image,
        "shared record carries the owner's image bytes"
    );
    assert!(out.textures_missing.is_empty());

    // Textured opacity folds into the tint alpha exactly like the solid arm:
    // "[Translucent Glass Tinted]" stores 0.52 with its use-opacity flag set
    // -> alpha round(0.52 * 255) = 133; opaque textured materials stay 255.
    let glass = find("[Translucent Glass Tinted]");
    assert!(glass.texture.is_some());
    assert_eq!(glass.color, Rgba8::rgba(255, 255, 255, 133));
    assert_eq!(
        find("[Wood Floor Light]").color,
        Rgba8::rgba(255, 255, 255, 255)
    );
}

#[test]
fn back_painted_face_still_gets_its_material() {
    let out = skp_import::import(&fixture("back-material.skp")).unwrap();
    let mesh = out
        .scene
        .roots
        .iter()
        .find_map(|n| match n {
            ImportNode::Mesh(m) => Some(m),
            _ => None,
        })
        .expect("one world face");
    assert!(
        mesh.face_materials
            .iter()
            .any(|&m| m != kernel::NO_MATERIAL),
        "back-only paint falls back onto the face material"
    );
}

// ── Visibility and layers ────────────────────────────────────────────────────

#[test]
fn hidden_faces_and_hidden_layers_do_not_open_solids() {
    // Hidden faces are display state: dropping them would turn closed solids
    // into leaky shells. SketchUp's own exports keep them (ground truth).
    let (report, _) = ingest("hidden-entities.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1, "the box stays closed");

    // layers.skp: three boxes on three layers (one layer hidden) — all import.
    let (report, _) = ingest("layers.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1, "all three boxes' shells stay closed");
}

// ── Guides ───────────────────────────────────────────────────────────────────

#[test]
fn guides_import_and_undo_with_the_import() {
    let out = skp_import::import(&fixture("guide.skp")).unwrap();
    assert_eq!(out.scene.guides.len(), 1);

    let mut doc = Document::new();
    doc.ingest(out.scene, out.textures_missing).unwrap();
    assert_eq!(doc.guide_ids().len(), 1);
    doc.undo().unwrap();
    assert_eq!(
        doc.guide_ids().len(),
        0,
        "one undo removes the whole import"
    );
}

// ── Robustness odds and ends ─────────────────────────────────────────────────

#[test]
fn cylinder_curve_and_long_names_import_cleanly() {
    let (report, _) = ingest("cylinder.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1, "faceted cylinder is a closed solid");

    // curve.skp is edge-only (a freehand curve, zero faces): Hew imports
    // faces into solids, so this legitimately imports as nothing — same as
    // the `.dae` path, which ignores COLLADA `<lines>`.
    let (report, _) = ingest("curve.skp");
    assert_eq!(report.objects_created, 0);
    assert!(report.skipped.is_empty());

    // A pathologically long entity name must not break anything.
    let (report, _) = ingest("long-name.skp");
    assert!(report.skipped.is_empty());
}

#[test]
fn instance_poses_come_through_scaled_rotated_and_moved() {
    for name in [
        "instance-scaled.skp",
        "component-rotate.skp",
        "component-move.skp",
    ] {
        let out = skp_import::import(&fixture(name)).unwrap();
        fn any_non_identity(nodes: &[ImportNode]) -> bool {
            nodes.iter().any(|n| match n {
                ImportNode::Instance { pose, .. } => *pose != kernel::Transform::IDENTITY,
                ImportNode::Group { children, .. } => any_non_identity(children),
                _ => false,
            })
        }
        assert!(
            any_non_identity(&out.scene.roots),
            "{name}: expected a non-identity placement"
        );
    }
}
