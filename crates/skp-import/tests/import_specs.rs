//! Acceptance specs for `skp_import::import` (DEVELOPMENT.md rule 3).
//!
//! Fixtures under `tests/fixtures/` are frozen copies of OpenSKP's
//! self-authored corpus (`corpus/2017` + one legacy/future sample each) —
//! committed here so tests run anywhere the workspace builds. Ground truth
//! and corpus stewardship stay upstream in OpenSKP; refresh fixtures when the
//! rev pin advances. Expected numbers below were validated against the
//! paired SketchUp `.dae` exports (see `differential.rs`).

use std::path::Path;

use kernel::{Document, ImportNode};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()))
}

/// Import a fixture and ingest it into a fresh document.
fn ingest(name: &str) -> (kernel::ImportReport, Document) {
    let out = skp_import::import(&fixture(name)).expect(name);
    assert!(
        out.warnings.is_empty(),
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
    // tier). Hierarchy comes through as groups, geometry stays shared.
    let (report, doc) = ingest("nested-3-deep.skp");
    assert_eq!(report.objects_created, 1, "one shared leaf def");
    assert_eq!(report.watertight, 1);
    assert_eq!(doc.instance_ids().len(), 4);
    assert_eq!(doc.group_ids().len(), 3);
}

#[test]
fn groups_and_mixed_definitions_import_structurally() {
    let (report, doc) = ingest("box-group.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(doc.instance_ids().len(), 1);

    let (report, doc) = ingest("mixed-definition.skp");
    assert_eq!(report.objects_created, 2);
    assert_eq!(report.watertight, 2);
    assert_eq!(doc.instance_ids().len(), 2);
    assert_eq!(doc.group_ids().len(), 1);
}

// ── Native names (no __HEWMETA__ hex dance — the joy of M25) ────────────────

#[test]
fn house_carries_native_instance_names_and_layer_tags() {
    let out = skp_import::import(&fixture("house.skp")).unwrap();

    fn walk<'a>(nodes: &'a [ImportNode], hits: &mut Vec<(&'a str, &'a [Vec<String>])>) {
        for n in nodes {
            match n {
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
        .expect("'Front Wall' imports as a native instance name");
    assert_eq!(
        front_wall.1,
        &[vec!["Exterior Walls".to_string()]],
        "the instance's layer arrives as a tag"
    );
    assert!(named.iter().any(|(n, _)| *n == "Slab"));
}

#[test]
fn house_ingests_with_shared_defs_and_loud_skips() {
    let (report, doc) = ingest("house.skp");
    // Frozen regression numbers (validated against house.dae, which flattens
    // to 69 baked objects with 34 leaky shells — the .skp path keeps shared
    // definitions and watertight solids instead).
    assert_eq!(report.objects_created, 32);
    assert_eq!(report.watertight, 32);
    assert_eq!(report.leaky, 0);
    assert_eq!(doc.instance_ids().len(), 60);
    assert_eq!(doc.group_ids().len(), 17);
    // Two source meshes are genuinely non-manifold: rejected + reported,
    // never repaired (rule 4). If an OpenSKP fix upstream makes these import,
    // this count DROPS — update deliberately with the rev bump.
    assert_eq!(report.skipped.len(), 2);
    // One texture has no embedded image bytes in the file.
    assert_eq!(
        report.textures_missing,
        vec!["Wood_Floor_Light.jpg".to_string()]
    );
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
