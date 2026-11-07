//! Acceptance specs for `skp_import::import` (DEVELOPMENT.md rule 3).
//!
//! Fixtures under `tests/fixtures/` are frozen copies of Skpwright's
//! self-authored corpus (`corpus/2017`, its 2026 twins under `2026/`, and
//! one pre-2017 sample) — committed here so tests run anywhere the
//! workspace builds. Ground truth and corpus stewardship stay upstream in
//! Skpwright; refresh fixtures when the rev pin advances. Expected numbers
//! below were validated against the paired SketchUp `.dae` exports (see
//! `differential.rs`); `containers.rs` holds every 2026 twin to its 2017
//! original.

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
        !out.warnings
            .iter()
            .any(|w| w.contains("parser recovered") || w.contains("reader skipped")),
        "{name}: clean corpus files parse without recovery warnings; got {:?}",
        out.warnings
    );
    assert!(
        !out.warnings
            .iter()
            .any(|w| w == skp_import::INCOMPLETE_NOTE),
        "{name}: a complete file is never called incomplete"
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

/// `box.skp` (a {17.x} stream file) with its version's two digits replaced.
fn stream_stamped(major: &str) -> Vec<u8> {
    let mut b = fixture("box.skp");
    assert_eq!(
        b[36..44],
        [0x7b, 0, 0x31, 0, 0x37, 0, 0x2e, 0],
        "{{17. header"
    );
    b[38] = major.as_bytes()[0];
    b[40] = major.as_bytes()[1];
    b
}

#[test]
fn stream_files_of_a_release_the_reader_holds_are_imported_at_each_end() {
    // Release 2013 (a real file) and the stamped boundaries of the held
    // set: 2020 is the newest stream release, 2014 sits between.
    let (report, _doc) = ingest("box-v2013.skp");
    assert_eq!(report.objects_created, 1);
    for major in ["14", "18", "20"] {
        assert!(
            skp_import::import(&stream_stamped(major)).is_ok(),
            "stream stamped {{{major}.: imports"
        );
    }
}

#[test]
fn stream_files_of_an_unlisted_release_are_read_with_a_warning() {
    // 9 to 12 were never observed and 21 and later are not stream
    // releases Skpwright lists: the body is a stream container, so Skpwright
    // is asked and reads what it can; the warning says the release is
    // unlisted. A listed release raises none.
    for major in ["21", "25", "12", "10"] {
        let s = skp_import::import(&stream_stamped(major))
            .unwrap_or_else(|e| panic!("{major}: Skpwright reads the box: {e}"));
        assert_eq!(s.scene.roots.len(), 1, "{major}");
        assert!(
            s.warnings
                .first()
                .is_some_and(|w| w.contains(&format!("{{{major}.")) && w.contains("does not list")),
            "{major}: {:?}",
            s.warnings
        );
    }
    let listed = skp_import::import(&stream_stamped("19")).unwrap();
    assert!(listed.warnings.is_empty(), "{:?}", listed.warnings);
}

#[test]
fn stream_files_of_an_unlisted_release_are_refused_only_when_skpwright_refuses() {
    // Cut short, the same stamped file gives Skpwright nothing to read
    // (an error, or an empty model): refused with the file's version.
    for major in ["21", "12"] {
        for cut in [80usize, 1024] {
            let mut b = stream_stamped(major);
            b.truncate(cut);
            match skp_import::import(&b) {
                Err(skp_import::SkpError::UnsupportedVersion { version }) => {
                    assert!(version.starts_with(&format!("{{{major}.")), "{version}");
                }
                other => panic!(
                    "{major}@{cut}: expected UnsupportedVersion, ok={}",
                    other.is_ok()
                ),
            }
        }
    }
    // The same cut of a listed release is damage, not a version problem.
    let mut b = fixture("box.skp");
    b.truncate(1024);
    assert!(matches!(
        skp_import::import(&b),
        Err(skp_import::SkpError::Damaged)
    ));
    let msg = skp_import::SkpError::UnsupportedVersion {
        version: "{21.0.1}".into(),
    }
    .to_string();
    assert!(
        msg.contains("{21.0.1}") && !msg.contains("Version 2017"),
        "{msg}"
    );
}

#[test]
fn an_intact_header_of_an_undecoded_version_is_unsupported_not_damaged() {
    // The 2026 box with everything after its header zeroed, stamped with
    // each version: a decoded major is damage, any other is unsupported.
    let zeroed = |major: &str| {
        let mut cut = fixture("2026/box.skp");
        cut.truncate(0x41 + 256);
        cut[0x41..].fill(0);
        assert_eq!(cut[0x24..0x2a], [0x7b, 0, 0x32, 0, 0x36, 0], "{{26 header");
        cut[0x26] = major.as_bytes()[0];
        cut[0x28] = major.as_bytes()[1];
        cut
    };
    for major in ["21", "25"] {
        assert!(
            matches!(
                skp_import::import(&zeroed(major)),
                Err(skp_import::SkpError::UnsupportedVersion { .. })
            ),
            "{major}: unsupported"
        );
    }
    for major in ["26", "27", "17"] {
        assert!(
            matches!(
                skp_import::import(&zeroed(major)),
                Err(skp_import::SkpError::Damaged)
            ),
            "{major}: damaged"
        );
    }
}

#[test]
fn the_2026_container_is_imported_whatever_its_version_says() {
    // box-v2026.skp: the 1 m box saved by SketchUp 2026 — the container
    // every current release writes, read as a first-class input.
    let (report, doc) = ingest("box-v2026.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert_eq!(doc.instance_ids().len(), 0);
}

#[test]
fn a_current_release_header_with_no_model_behind_it_is_reported_as_damage() {
    // The 2026 box with everything after its header strings zeroed: the
    // header still identifies a {26.x} file, but no ZIP follows. That is a
    // damaged file, and the copy must not send the user off to re-save an
    // already current file.
    let mut cut = fixture("2026/box.skp");
    cut.truncate(0x41 + 256);
    cut[0x41..].fill(0);
    match skp_import::import(&cut) {
        Err(e @ skp_import::SkpError::Damaged) => {
            let msg = e.to_string();
            assert!(msg.contains("damaged"), "says it is damage: {msg}");
            assert!(msg.contains("fresh copy"), "asks for a fresh copy: {msg}");
            assert!(!msg.contains("Save As"), "no re-save advice: {msg}");
        }
        other => panic!("expected Damaged, got {other:?}", other = other.is_ok()),
    }
}

/// `bytes` cut to `fraction` of their length.
fn cut_to(mut bytes: Vec<u8>, fraction: f64) -> Vec<u8> {
    bytes.truncate((bytes.len() as f64 * fraction) as usize);
    bytes
}

#[test]
fn a_2017_file_cut_short_with_nothing_readable_is_refused_as_damaged() {
    // Cut this far, the walk fallback finds no shape at all: the open must
    // not come in as an empty model.
    let box_skp = fixture("box.skp");
    let mut cuts = vec![box_skp[..4096].to_vec()];
    cuts.extend([0.1, 0.25, 0.5].map(|f| cut_to(box_skp.clone(), f)));
    for cut in cuts {
        assert!(
            matches!(skp_import::import(&cut), Err(skp_import::SkpError::Damaged)),
            "{} of {} bytes: expected Damaged",
            cut.len(),
            box_skp.len()
        );
    }
}

#[test]
fn a_2017_file_cut_short_that_still_reads_opens_with_the_incomplete_note() {
    // Cut late, every shape is still there (only the trailing records are
    // lost): the file opens, and says it is incomplete.
    for fraction in [0.75, 0.9, 0.99] {
        let out = skp_import::import(&cut_to(fixture("box.skp"), fraction))
            .unwrap_or_else(|e| panic!("{fraction}: {e}"));
        assert_eq!(
            out.warnings.first().map(String::as_str),
            Some(skp_import::INCOMPLETE_NOTE),
            "{fraction}: the incomplete note leads the warnings"
        );
        let mut doc = Document::new();
        let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
        assert_eq!(report.objects_created, 1, "{fraction}: the box survives");
    }
}

#[test]
fn a_2017_file_cut_anywhere_is_never_opened_silently() {
    // Every cut past the header either is refused as damaged or opens with
    // the incomplete note: the grammar's three ways of running out of data
    // (a read past the end, a count or a string longer than what is left)
    // all count, wherever the cut lands.
    for name in ["box.skp", "box-component-two-instances.skp"] {
        let full = fixture(name);
        for n in (2048..full.len()).step_by(997) {
            match skp_import::import(&full[..n]) {
                Err(skp_import::SkpError::Damaged) => {}
                Ok(out) => assert_eq!(
                    out.warnings.first().map(String::as_str),
                    Some(skp_import::INCOMPLETE_NOTE),
                    "{name} cut to {n} bytes opened without the incomplete note"
                ),
                Err(e) => panic!("{name} cut to {n} bytes: {e}"),
            }
        }
    }
}

#[test]
fn a_2026_file_cut_short_is_refused_as_damaged() {
    for fraction in [0.1, 0.5, 0.9, 0.999] {
        assert!(
            matches!(
                skp_import::import(&cut_to(fixture("2026/box.skp"), fraction)),
                Err(skp_import::SkpError::Damaged)
            ),
            "{fraction}: expected Damaged"
        );
    }
}

#[test]
fn no_complete_fixture_is_called_incomplete() {
    // The whole-file grammar reads every complete 2017 file, so the note
    // never fires on one; a 2026 file never carries it.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for sub in [dir.clone(), dir.join("2026")] {
        for entry in std::fs::read_dir(&sub).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("skp") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            if let Ok(out) = skp_import::import(&bytes) {
                assert!(
                    !out.warnings
                        .iter()
                        .any(|w| w == skp_import::INCOMPLETE_NOTE),
                    "{}: a complete file is never called incomplete",
                    path.display()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 20, "the sweep reached the fixtures ({checked})");
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
        Some(("Front Entry Door Frame", 6)),
        "the door frame group nests as a Group, not a shared def, under its \
         own placement name (not its definition's \"Group#769\")"
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
    // warnings) instead of being rejected: 50 watertight solids + 7 leaky
    // pieces, nothing skipped. (Coplanar faces stay apart as SketchUp holds
    // them; when they merged on import, one more solid split and the
    // pieces were fewer: 49 + 3.)
    assert_eq!(report.objects_created, 57);
    assert_eq!(report.watertight, 50);
    assert_eq!(report.leaky, 7);
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
    assert_eq!(doc.expanded_placements().len(), 60);
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
    // Skpwright (0.2.0) resolves to the owning material's bytes.
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
    // "[Wood Floor Light]"'s image with no inline bytes of its own. Skpwright
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
    // "[Translucent Glass Tinted]" stores a transparency of 0.52 with its
    // flag set (house.dae exports it as an RGB_ZERO transparent colour), so
    // it is 48% opaque -> alpha round(0.48 * 255) = 122; opaque textured
    // materials stay 255.
    let glass = find("[Translucent Glass Tinted]");
    assert!(glass.texture.is_some());
    assert_eq!(glass.color, Rgba8::rgba(255, 255, 255, 122));
    assert_eq!(
        find("[Wood Floor Light]").color,
        Rgba8::rgba(255, 255, 255, 255)
    );
}

#[test]
fn a_face_keeps_its_front_and_back_paint() {
    // back-material.skp: one face painted differently on each side. The
    // front paint is the face's material, the back paint its back.
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
    let painted: Vec<(u32, u32)> = mesh
        .face_materials
        .iter()
        .zip(&mesh.extra.face_back_materials)
        .map(|(&f, &b)| (f, b))
        .filter(|&(_, b)| b != kernel::NO_MATERIAL)
        .collect();
    assert_eq!(painted.len(), 1, "{:?}", mesh.extra.face_back_materials);
    let (front, back) = painted[0];
    assert!(front != kernel::NO_MATERIAL && front != back);
    let name = |d: u32| out.scene.materials[d as usize].name.clone();
    assert_ne!(name(front), name(back));
}

// ── Visibility and layers ────────────────────────────────────────────────────

#[test]
fn hidden_faces_and_hidden_layers_do_not_open_solids() {
    // Hidden faces are display state: dropping them would turn closed solids
    // into leaky shells. SketchUp's own exports keep them (ground truth).
    let (report, _) = ingest("hidden-entities.skp");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1, "the box stays closed");

    // layers.skp: three loose boxes on three layers (one layer hidden) —
    // all import, each its own solid tagged with its layer (loose geometry
    // carries its layer on its faces), the hidden layer's tag hidden.
    let (report, doc) = ingest("layers.skp");
    assert_eq!(report.objects_created, 3);
    assert_eq!(report.watertight, 3, "all three boxes' shells stay closed");
    let mut tags: Vec<String> = doc
        .top_level_nodes()
        .into_iter()
        .map(|n| doc.node_tags(n).concat().join("/"))
        .collect();
    tags.sort();
    assert_eq!(tags, ["BigLayer", "LittleLayer", "MediumLayer"]);
    assert_eq!(doc.tag_meta().filter(|(_, hidden)| *hidden).count(), 1);
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

// ── Scenes and the saved view ────────────────────────────────────────────────

#[test]
fn scenes_arrive_with_cameras_hidden_tags_and_the_saved_view() {
    // two-scenes.skp: two saved scenes; Scene 1 hides one layer. The
    // saved view (the preview thumbnail's camera) lands as the document
    // camera because the document was empty; every scene keeps its own
    // camera, hidden-tag set and hidden-node set (a full set, empty here).
    let (_, doc) = ingest("two-scenes.skp");
    let cam = doc
        .camera_state()
        .expect("saved view adopted into the empty document");
    let inch = skpwright::INCH;
    assert!((cam.eye.x - 326.1769714 / inch).abs() < 1e-6);
    assert!((cam.eye.y - -250.1695099 / inch).abs() < 1e-6);
    assert!((cam.eye.z - 154.6123047 / inch).abs() < 1e-6);
    assert!((cam.fov_deg - 35.0).abs() < 1e-9);

    let scenes = doc.scenes();
    let names: Vec<&str> = scenes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Scene 1", "Scene 2"]);
    let s1 = &scenes[0];
    let c1 = s1.camera.as_ref().expect("Scene 1 captures its camera");
    assert!((c1.eye.x - 56.3224258 / inch).abs() < 1e-6);
    assert!((c1.eye.y - -385.0967712 / inch).abs() < 1e-6);
    assert_eq!(
        s1.hidden_tags.as_ref().map(|t| t.len()),
        Some(1),
        "Scene 1 hides one layer"
    );
    assert_eq!(s1.hidden_nodes.as_ref().map(|n| n.len()), Some(0));
    assert_eq!(s1.section, Some(None), "captured: no section cut");
    let s2 = &scenes[1];
    assert_eq!(s2.hidden_tags.as_ref().map(|t| t.len()), Some(0));
    assert!(s2.camera.is_some());
    // Each scene saved its style: the look comes along, and it is the
    // file's own backdrop (sky and ground on, the model axes drawn).
    for s in scenes {
        let style = s
            .view_style
            .unwrap_or_else(|| panic!("{}: saved style", s.name));
        assert_eq!(style, doc.view_style().unwrap(), "{}", s.name);
        assert!(s.props().display, "{}", s.name);
    }

    // Importing into a document that already has content keeps its view.
    let out = skp_import::import(&fixture("two-scenes.skp")).unwrap();
    let mut doc2 = Document::new();
    let first = skp_import::import(&fixture("box.skp")).unwrap();
    doc2.ingest(first.scene, first.textures_missing).unwrap();
    let before = doc2.camera_state();
    doc2.ingest(out.scene, out.textures_missing).unwrap();
    assert_eq!(
        doc2.camera_state(),
        before,
        "a non-empty document keeps its camera"
    );
    assert_eq!(doc2.scenes().len(), 2, "scenes are still added");
}

#[test]
fn a_scene_that_did_not_save_its_hidden_sets_does_not_claim_them() {
    // A page without the hidden-geometry / visible-layers properties leaves
    // visibility alone when recalled: its sets arrive absent, not empty.
    use skp_import::skpwright::SceneProperties;
    let mut m = skp_import::skpwright::Model::parse(&fixture("two-scenes.skp")).unwrap();
    for s in &mut m.scenes {
        s.saved = SceneProperties(
            s.saved.0
                & !(SceneProperties::HIDDEN_GEOMETRY
                    | SceneProperties::HIDDEN_GEOMETRY_2
                    | SceneProperties::HIDDEN_OBJECTS
                    | SceneProperties::VISIBLE_LAYERS),
        );
        s.hidden_entities.clear();
        s.hidden_layers.clear();
    }
    let bytes = skp_import::skpwright::StreamDocument::from_model(&m, skpwright::Release::R2017)
        .unwrap()
        .write();
    let out = skp_import::import(&bytes).unwrap();
    let mut doc = Document::new();
    doc.ingest(out.scene, out.textures_missing).unwrap();
    assert_eq!(doc.scenes().len(), 2);
    for s in doc.scenes() {
        assert_eq!(s.hidden_nodes, None, "{}", s.name);
        assert_eq!(s.hidden_tags, None, "{}", s.name);
    }
}

#[test]
fn geometry_hew_built_reads_as_written_and_a_files_own_is_repaired() {
    // Two triangles sharing an edge, wound against each other. A file's own
    // mesh like that is repaired on open (one face turned over to agree);
    // marked as built by Hew (`loose_built`), it reads as written — both
    // faces turned as they are, split where they disagree.
    use skp_import::skpwright::{Attributes, Dictionary, Mesh, MeshEdge, MeshFace, Owner, Value};
    let mut m = skp_import::skpwright::Model::parse(&fixture("box.skp")).unwrap();
    let run = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .unwrap();
    let face = |outer: Vec<u32>, z: f64| MeshFace {
        outer,
        normal: [0.0, 0.0, z],
        ..Default::default()
    };
    let edge = |v0: u32, v1: u32| MeshEdge {
        v0,
        v1,
        ..Default::default()
    };
    m.geometry[run].mesh = Mesh {
        vertices: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        faces: vec![face(vec![0, 1, 2], 1.0), face(vec![3, 2, 0], -1.0)],
        edges: vec![edge(0, 1), edge(1, 2), edge(2, 0), edge(2, 3), edge(3, 0)],
        ..Default::default()
    };
    m.geometry[run].curves.clear();
    let normals = |m: &skp_import::skpwright::Model| -> Vec<i64> {
        let bytes = skp_import::skpwright::StreamDocument::from_model(m, skpwright::Release::R2017)
            .unwrap()
            .write();
        let out = skp_import::import(&bytes).unwrap();
        let mut doc = Document::new();
        doc.ingest(out.scene, out.textures_missing).unwrap();
        let mut n: Vec<i64> = doc
            .visible_object_ids()
            .into_iter()
            .flat_map(|o| {
                doc.object(o)
                    .unwrap()
                    .faces()
                    .iter()
                    .map(|(_, f)| f.plane.normal().z.round() as i64)
                    .collect::<Vec<_>>()
            })
            .collect();
        n.sort();
        n
    };
    // A file's own: repaired, the two faces agree.
    let repaired = normals(&m);
    assert_eq!(repaired.len(), 2);
    assert_eq!(repaired[0], repaired[1], "{repaired:?}");
    // Built by Hew: as written.
    m.dictionaries.push(Attributes {
        owner: Owner::Model,
        dictionaries: vec![Dictionary {
            name: "hew".into(),
            entries: vec![
                ("version".into(), Value::Int(1)),
                ("loose_built".into(), Value::Bool(true)),
            ],
            flag: 0,
        }],
    });
    assert_eq!(normals(&m), [-1, 1]);
}

/// The house fixture's open objects, by kind: what Model Health lists for
/// a real SketchUp file. Mostly solids; the open ones are sheets the model
/// drew on purpose and one box the importer split at two internal faces
/// (a seam family of three cappable slices — the middle one open at both
/// ends — and two partition sheets). The import report carries the same
/// counts.
#[test]
fn the_house_fixtures_open_objects_read_as_surfaces_and_cappable_boxes() {
    let (report, doc) = ingest("house.skp");
    let c = doc.health_report(None).counts();
    assert_eq!(
        c, report.health,
        "the import report's counts are the document's"
    );
    assert_eq!(c.objects, report.objects_created);
    assert_eq!((c.solids, c.flat_sheets, c.bent_sheets), (50, 3, 1));
    assert_eq!((c.cappable, c.holes, c.mixed, c.non_manifold), (3, 0, 0, 0));
    assert_eq!(c.seam_families, 1);
    assert_eq!(c.surfaces() + c.needing_attention(), report.leaky);
}

#[test]
fn sketchup_dimensions_open_on_edge_midpoints_arc_centers_and_in_plane() {
    use kernel::Annotation;
    use skpwright::{Anchor, Dimension, Model};
    // cylinder.skp: arcs among the model's own entities. Four dimensions
    // SketchUp can draw on it, written beside it: arc center to a vertex
    // along x (frame 1), an edge's midpoint to a point along n × x (frame
    // 2), a vertex to a point raised off the dimension's plane, which
    // measures the distance in that plane (frame 0), and from where two
    // edges meet.
    let mut m = Model::parse(&fixture("cylinder.skp")).unwrap();
    let ri = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none() && r.curves.iter().any(|c| c.arc.is_some()))
        .expect("an arc among the model's own entities");
    let run = &m.geometry[ri];
    let curve = run.curves.iter().find(|c| c.arc.is_some()).unwrap();
    let center = curve.arc.as_ref().unwrap().center_m;
    let edge = run.mesh.edges.iter().find(|e| e.pid != 0).unwrap();
    let (p, q) = (
        run.mesh.vertices[edge.v0 as usize],
        run.mesh.vertices[edge.v1 as usize],
    );
    let mid = [
        (p[0] + q[0]) / 2.0,
        (p[1] + q[1]) / 2.0,
        (p[2] + q[2]) / 2.0,
    ];
    let vi = run.mesh.vertex_pids.iter().position(|&v| v != 0).unwrap();
    let (vertex, vpid) = (run.mesh.vertices[vi], run.mesh.vertex_pids[vi]);
    let (cpid, epid) = (curve.pid, edge.pid);
    let at = |kind, entity| Anchor {
        kind,
        entity: Some(entity),
        ..Default::default()
    };
    let free = |point_m| Anchor {
        kind: 1,
        point_m,
        ..Default::default()
    };
    let dim = |start, end, variant| Dimension {
        start,
        end,
        normal: [0.0, 0.0, 1.0],
        x_axis: [1.0, 0.0, 0.0],
        offset_m: 0.5,
        variant,
        ..Default::default()
    };
    let raised = [vertex[0] + 3.0, vertex[1] + 4.0, vertex[2] + 7.0];
    // Where two edges meet: the edge above and one sharing its first end.
    let other = run
        .mesh
        .edges
        .iter()
        .find(|e| e.pid != 0 && e.pid != epid && (e.v0 == edge.v0 || e.v1 == edge.v0))
        .unwrap();
    let corner = run.mesh.vertices[edge.v0 as usize];
    let crossing = Anchor {
        kind: 9,
        entity: Some(epid),
        second: Some(other.pid),
        ..Default::default()
    };
    m.dimensions = vec![
        dim(at(10, cpid), at(2, vpid), 1),
        dim(at(4, epid), free([mid[0] + 1.0, mid[1] + 2.0, mid[2]]), 2),
        dim(at(2, vpid), free(raised), 0),
        dim(crossing, free([corner[0] + 1.0, corner[1], corner[2]]), 1),
    ];
    m.texts.clear();
    let bytes = skpwright::StreamDocument::from_model(&m, skpwright::Release::R2017)
        .unwrap()
        .write();

    let out = skp_import::import(&bytes).unwrap();
    let dims: Vec<_> = out
        .scene
        .extra
        .annotations
        .iter()
        .filter_map(|a| match a {
            Annotation::LinearDimension { a, b, offset, .. } => Some((a.point, b.point, *offset)),
            _ => None,
        })
        .collect();
    assert_eq!(dims.len(), 4, "every dimension opens: {:?}", out.warnings);
    let near = |a: kernel::Point3, b: [f64; 3]| {
        (a.x - b[0]).abs() < 1e-9 && (a.y - b[1]).abs() < 1e-9 && (a.z - b[2]).abs() < 1e-9
    };
    let v = |o: kernel::Vec3, b: [f64; 3]| near(kernel::Point3::new(o.x, o.y, o.z), b);
    // Frame 1: from the arc's center, measured along x, offset along y.
    assert!(near(dims[0].0, center));
    assert!(near(dims[0].1, [vertex[0], center[1], center[2]]));
    assert!(v(dims[0].2, [0.0, 0.5, 0.0]));
    // Frame 2: from the edge's midpoint, measured along y, offset along -x.
    assert!(near(dims[1].0, mid));
    assert!(near(dims[1].1, [mid[0], mid[1] + 2.0, mid[2]]));
    assert!(v(dims[1].2, [-0.5, 0.0, 0.0]));
    // Frame 0: the 3-4 run in the plane (5 long), not the 7 rise, offset
    // across it.
    assert!(near(dims[2].0, vertex));
    assert!(near(
        dims[2].1,
        [vertex[0] + 3.0, vertex[1] + 4.0, vertex[2]]
    ));
    assert!(v(dims[2].2, [-0.4, 0.3, 0.0]));
    // Where the two edges meet: their shared corner.
    let c = dims[3].0;
    assert!(
        (c.x - corner[0]).abs() < 1e-9
            && (c.y - corner[1]).abs() < 1e-9
            && (c.z - corner[2]).abs() < 1e-9,
        "{c:?} is not {corner:?}"
    );
}

#[test]
fn a_dimension_on_one_placement_of_a_component_opens_on_that_placement() {
    // dim-second-instance.skp (Skpwright's anchors pair): two placements of
    // one box component, and a dimension SketchUp's Dimension tool drew
    // between two top front corners of the second, 38.75 in apart.
    let out = skp_import::import(&fixture("dim-second-instance.skp")).unwrap();
    let ends: Vec<_> = out
        .scene
        .extra
        .annotations
        .iter()
        .filter_map(|a| match a {
            kernel::Annotation::LinearDimension { a, b, .. } => Some((a.point, b.point)),
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 1);
    let at = |p: kernel::Point3, q: [f64; 3]| {
        let v = [p.x, p.y, p.z].map(|v| v / 0.0254);
        assert!(
            (0..3).all(|k| (v[k] - q[k]).abs() < 1e-6),
            "{v:?} is not {q:?}"
        );
    };
    at(ends[0].0, [48.9375, 0.0, 31.3125]);
    at(ends[0].1, [87.6875, 0.0, 31.3125]);
}

/// An image placed inside a group (or a component) arrives with it: an
/// instance of the image's definition — one face painted with the picture
/// — among the group's members, at the picture's world pose. Both
/// containers; the 2017 one carries the image as an instance record.
#[test]
fn an_image_inside_a_group_arrives_in_that_group() {
    let (_, root) = ingest("2026/feature-pack.skp");
    let root_image = root
        .instance_ids()
        .into_iter()
        .find(|&i| {
            let def = root.instance_def(i).unwrap();
            root.component_name(def)
                .is_some_and(|n| n.starts_with("Image"))
        })
        .expect("feature-pack places one image at the root");
    let want = root.instance_pose(root_image).unwrap();
    for name in ["2026/image-in-group.skp", "image-in-group.skp"] {
        let (_, doc) = ingest(name);
        let group = doc
            .group_ids()
            .into_iter()
            .find(|&g| doc.group_name(g) == Some("Picture group"))
            .unwrap_or_else(|| panic!("{name}: the group arrives"));
        let members = doc.group_members(group).unwrap();
        let image = members
            .iter()
            .find_map(|m| match m {
                kernel::NodeId::Instance(i) => Some(*i),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name}: the image is an instance in the group"));
        let def = doc.instance_def(image).unwrap();
        assert!(
            doc.component_name(def)
                .is_some_and(|n| n.starts_with("Image")),
            "{name}: the instance is of the image's definition"
        );
        let members = doc.def_member_nodes(def).unwrap();
        let [kernel::NodeId::Object(o)] = members.as_slice() else {
            panic!("{name}: the image definition holds its one face");
        };
        let obj = doc.object(*o).unwrap();
        assert!(
            obj.faces().iter().any(|(f, _)| {
                doc.face_material(*o, f)
                    .and_then(|m| doc.material(m))
                    .is_some_and(|m| m.texture.is_some())
            }),
            "{name}: the face is painted with the picture"
        );
        // Where feature-pack shows the picture: the group's pose composed
        // with the image's place inside it.
        let got = doc.instance_pose(image).unwrap();
        for (a, b) in got.to_affine().iter().zip(want.to_affine().iter()) {
            assert!((a - b).abs() < 1e-6, "{name}: pose {got:?} vs {want:?}");
        }
    }
}

/// A group holding only edges arrives as a group with no members holding
/// its drawing: one sketch per plane the edges lie in, in world
/// coordinates (the group's placement applied), and the group's own
/// dimension and label as annotations through the same placement. Both
/// containers.
#[test]
fn a_group_of_lines_arrives_as_a_group_holding_its_drawing() {
    for name in ["2026/lines-group.skp", "lines-group.skp"] {
        let (report, doc) = ingest(name);
        let group = doc
            .group_ids()
            .into_iter()
            .find(|&g| doc.group_name(g) == Some("Lines"))
            .unwrap_or_else(|| panic!("{name}: the group arrives"));
        assert!(
            doc.group_members(group).unwrap().is_empty(),
            "{name}: nothing but lines"
        );
        let held = doc.group_member_sketches(group);
        assert_eq!(
            held.len(),
            1,
            "{name}: the square and its diagonal lie in one plane"
        );
        let sk = doc.sketch(held[0]).unwrap();
        assert_eq!(sk.edges().len(), 5, "{name}");
        // Placed 2 m along x: the square's corners are at x = 2 and 3.
        let xs: std::collections::BTreeSet<i64> = sk
            .vertices()
            .values()
            .map(|v| (v.position.x * 1000.0).round() as i64)
            .collect();
        assert_eq!(xs, [2000, 3000].into(), "{name}");
        assert!(doc.sketch_shown(held[0]), "{name}");
        assert!(
            report.sketches.iter().all(|s| s.is_some()),
            "{name}: every sketch built"
        );
        // The group's dimension and label, through the group's pose.
        let mut dims = 0;
        let mut labels = Vec::new();
        for id in doc.annotation_ids() {
            match doc.annotation(id).unwrap() {
                kernel::Annotation::LinearDimension { a, b, .. } => {
                    dims += 1;
                    assert!(
                        (a.point.x - 2.0).abs() < 1e-9 && (b.point.x - 3.0).abs() < 1e-9,
                        "{name}: {a:?} {b:?}"
                    );
                }
                kernel::Annotation::LeaderText {
                    anchor,
                    offset,
                    text,
                } => {
                    labels.push(text.clone());
                    assert!((anchor.point.x - 2.5).abs() < 1e-9, "{name}: {anchor:?}");
                    assert!((offset.z - 0.3).abs() < 1e-9, "{name}: {offset:?}");
                }
                _ => {}
            }
        }
        assert_eq!((dims, labels), (1, vec!["lines".to_string()]), "{name}");
    }
}

// ── An import built in steps ────────────────────────────────────────────

/// An import built a few items at a time is the import built at once,
/// and says how far it has come.
#[test]
fn an_import_built_in_steps_is_the_import_built_at_once() {
    for name in ["box-component-two-instances.skp", "feature-pack.skp"] {
        let bytes = fixture(name);
        let at_once = skp_import::import(&bytes).expect(name);
        let mut job = skp_import::import_begin(&bytes).expect(name);
        let (done, total) = job.progress();
        assert_eq!(done, 0, "{name}");
        assert!(total > 0, "{name}: something to build");
        let mut steps = 0usize;
        loop {
            let finished = job.step(1);
            steps += 1;
            let (done, t) = job.progress();
            assert_eq!(t, total, "{name}: the total holds");
            assert_eq!(done, steps.min(total), "{name}: one item per step");
            if finished {
                break;
            }
        }
        assert_eq!(steps, total, "{name}: every item took one step");
        let stepped = job.finish().expect(name);
        assert_eq!(stepped.scene.defs.len(), at_once.scene.defs.len(), "{name}");
        assert_eq!(
            stepped.scene.roots.len(),
            at_once.scene.roots.len(),
            "{name}"
        );
        assert_eq!(stepped.warnings, at_once.warnings, "{name}");
        let mut a = Document::new();
        a.ingest(at_once.scene, at_once.textures_missing).unwrap();
        let mut b = Document::new();
        b.ingest(stepped.scene, stepped.textures_missing).unwrap();
        assert_eq!(a.state_hash(), b.state_hash(), "{name}: the same document");
    }
}

/// A radial dimension's label sits where the file's leader ends: on the
/// leader's angle, `radius - leader[1]` (inches, negative outward) from
/// the circle's center. The leader vector carries that length, not a
/// unit length that would throw the label a metre off.
#[test]
fn radial_dimension_leader_ends_where_the_file_puts_it() {
    let bytes = fixture("dimension-radial.skp");
    let m = skpwright::Model::parse(&bytes).unwrap();
    let r = &m.radial_dimensions[0];
    assert!(r.leader[1] < 0.0, "fixture leaves the circle outward");
    let out = skp_import::import(&bytes).unwrap();
    let radials: Vec<_> = out
        .scene
        .extra
        .annotations
        .iter()
        .filter_map(|a| match a {
            kernel::Annotation::RadialDimension {
                anchor,
                curve,
                leader_dir,
                ..
            } => Some((anchor.point, curve.center, curve.radius, *leader_dir)),
            _ => None,
        })
        .collect();
    assert_eq!(radials.len(), 1, "{:?}", out.warnings);
    let (anchor, center, radius, dir) = radials[0];
    let end = anchor + dir;
    let want = radius - r.leader[1] / skpwright::INCH;
    assert!(
        ((end - center).length() - want).abs() < 1e-9,
        "end {:?} is {} from the center, want {want}",
        end,
        (end - center).length()
    );
    assert!(
        dir.dot(anchor - center) > 0.0,
        "an outward leader points away from the center"
    );
}
