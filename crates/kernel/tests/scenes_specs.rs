//! Executable specs for Scenes and the persisted section plane
//! (docs/design/scenes.md §3, §4; HEW_FILE_FORMAT.md §4.13/§4.14).
//!
//! Identity across save/load is judged by structure and stable ids, never
//! by handle value (`serialize_specs.rs`'s posture).

use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};

use kernel::{
    CameraProjection, CameraState, DisplayState, Document, DocumentError, EntityRef, LoadError,
    NodeId, Plane, Point3, SceneProps, SectionPlaneState, Vec3, tol,
};

// ----------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane")
}

fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

fn extrude_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
    let r = only_region(doc, s);
    doc.extrude_region(s, r, h).expect("extrude box").0
}

fn cam(eye: [f64; 3]) -> CameraState {
    CameraState {
        projection: CameraProjection::Perspective,
        fov_deg: 45.0,
        eye: Point3::new(eye[0], eye[1], eye[2]),
        target: Point3::new(0.0, 0.0, 0.0),
        up: Vec3::new(0.0, 0.0, 1.0),
    }
}

fn display(grid: bool) -> DisplayState {
    DisplayState {
        grid,
        axes: true,
        guides: true,
    }
}

fn plane_z(z: f64, active: bool) -> SectionPlaneState {
    SectionPlaneState {
        origin: Point3::new(0.0, 0.0, z),
        normal: Vec3::new(0.0, 0.0, 1.0),
        active,
    }
}

fn tag(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

/// A three-box document with one tagged, one grouped, one plain box.
struct Fixture {
    doc: Document,
    plain: kernel::ObjectId,
    tagged: kernel::ObjectId,
    grouped: kernel::ObjectId,
    group: kernel::GroupId,
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let plain = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let tagged = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let grouped = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 1.0);
    doc.add_node_tag(NodeId::Object(tagged), tag("Hardware"))
        .expect("tag");
    let (group, _) = doc.group_nodes(&[NodeId::Object(grouped)]).expect("group");
    Fixture {
        doc,
        plain,
        tagged,
        grouped,
        group,
    }
}

/// Re-write `manifest.json` inside `.hew` bytes through `edit`.
fn patch_manifest(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    edit(&mut manifest);
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();

    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    out.start_file("manifest.json", opts).unwrap();
    out.write_all(&patched).unwrap();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if entry.name() == "manifest.json" {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        out.start_file(&name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn manifest_json(bytes: &[u8]) -> serde_json::Value {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    serde_json::from_slice(&manifest_bytes).unwrap()
}

// --------------------------------------------------------- section plane

#[test]
fn section_plane_persists_and_is_not_undoable() {
    let mut f = fixture();
    let before = f.doc.undo_depth();
    f.doc
        .set_section_plane(plane_z(0.5, true))
        .expect("valid plane");
    assert_eq!(
        f.doc.undo_depth(),
        before,
        "section plane is view state, not history"
    );
    let bytes = f.doc.save();
    let loaded = Document::load(&bytes).expect("load");
    assert_eq!(loaded.section_plane(), Some(plane_z(0.5, true)));
    assert_eq!(loaded.save(), bytes, "round trip is byte-identical");
}

#[test]
fn section_plane_absent_writes_no_key() {
    let f = fixture();
    let m = manifest_json(&f.doc.save());
    assert!(m.get("section_plane").is_none());
    assert!(m.get("scenes").is_none());
}

#[test]
fn section_plane_refuses_non_unit_normal() {
    let mut doc = Document::new();
    let bad = SectionPlaneState {
        origin: Point3::new(0.0, 0.0, 0.0),
        normal: Vec3::new(0.0, 0.0, 2.0),
        active: true,
    };
    assert!(matches!(
        doc.set_section_plane(bad),
        Err(DocumentError::InvalidSectionPlane)
    ));
    assert_eq!(doc.section_plane(), None);
}

#[test]
fn stored_section_plane_with_bad_normal_is_rejected_on_load() {
    let mut f = fixture();
    f.doc.set_section_plane(plane_z(0.5, true)).unwrap();
    let bytes = f.doc.save();
    let patched = patch_manifest(&bytes, |m| {
        m["section_plane"]["normal"] = serde_json::json!([0.0, 0.0, 0.5]);
    });
    assert!(matches!(
        Document::load(&patched),
        Err(LoadError::MalformedManifest { .. })
    ));
}

// ------------------------------------------------------------ add/capture

#[test]
fn add_scene_captures_all_five_properties() {
    let mut f = fixture();
    f.doc.set_tag_hidden(tag("Hardware"), true);
    f.doc.set_node_user_hidden(NodeId::Group(f.group), true);
    f.doc.set_section_plane(plane_z(0.5, true)).unwrap();
    let before = f.doc.undo_depth();

    let sid = f
        .doc
        .add_scene(
            None,
            SceneProps::ALL,
            Some(cam([5.0, 5.0, 5.0])),
            Some(display(true)),
            None,
        )
        .expect("add");
    assert_eq!(
        f.doc.undo_depth(),
        before,
        "scenes are outside undo history"
    );

    let scene = f.doc.scene(sid).unwrap();
    assert_eq!(scene.name, "Scene 1");
    assert_eq!(scene.camera, Some(cam([5.0, 5.0, 5.0])));
    assert_eq!(scene.display, Some(display(true)));
    assert_eq!(scene.section, Some(Some(plane_z(0.5, true))));
    let tag_sid = f.doc.sid_of(&EntityRef::Tag(tag("Hardware"))).unwrap();
    assert_eq!(scene.hidden_tags, Some(BTreeSet::from([tag_sid])));
    let group_sid = f.doc.sid_of(&EntityRef::Group(f.group)).unwrap();
    assert_eq!(scene.hidden_nodes, Some(BTreeSet::from([group_sid])));
    assert_eq!(scene.props(), SceneProps::ALL);
}

#[test]
fn add_scene_auto_names_lowest_free_number_and_inserts_after() {
    let mut f = fixture();
    let a = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    let b = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    assert_eq!(f.doc.scene(a).unwrap().name, "Scene 1");
    assert_eq!(f.doc.scene(b).unwrap().name, "Scene 2");
    // Insert after `a` → lands between a and b, and takes the free "Scene 3".
    let c = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, Some(a))
        .unwrap();
    let order: Vec<u64> = f.doc.scenes().iter().map(|s| s.sid).collect();
    assert_eq!(order, vec![a, c, b]);
    assert_eq!(f.doc.scene(c).unwrap().name, "Scene 3");
    // Delete Scene 1; the next auto-name reuses the lowest free number.
    f.doc.remove_scene(a).unwrap();
    let d = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    assert_eq!(f.doc.scene(d).unwrap().name, "Scene 1");
}

#[test]
fn scene_names_are_unique_and_non_empty() {
    let mut f = fixture();
    let a = f
        .doc
        .add_scene(Some("Assembled".into()), SceneProps::ALL, None, None, None)
        .unwrap();
    assert!(matches!(
        f.doc
            .add_scene(Some("Assembled".into()), SceneProps::ALL, None, None, None),
        Err(DocumentError::DuplicateSceneName)
    ));
    assert!(matches!(
        f.doc
            .add_scene(Some("   ".into()), SceneProps::ALL, None, None, None),
        Err(DocumentError::EmptySceneName)
    ));
    let b = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    assert!(matches!(
        f.doc.rename_scene(b, "Assembled".into()),
        Err(DocumentError::DuplicateSceneName)
    ));
    // Renaming to your own name is fine.
    f.doc.rename_scene(a, "Assembled".into()).unwrap();
    f.doc.rename_scene(b, "Cut layout".into()).unwrap();
    assert_eq!(f.doc.scene(b).unwrap().name, "Cut layout");
    assert!(matches!(
        f.doc.rename_scene(9999, "x".into()),
        Err(DocumentError::UnknownScene)
    ));
}

#[test]
fn scene_sids_share_the_entity_counter() {
    let mut f = fixture();
    let sid = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    assert!(
        f.doc.sids().all(|(_, s)| s != sid),
        "no entity shares the scene's sid"
    );
    let later = extrude_box(&mut f.doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    assert!(f.doc.sid_of(&EntityRef::Object(later)).unwrap() > sid);
}

// -------------------------------------------------------- resolve / apply

#[test]
fn apply_restores_visibility_and_section_and_returns_leaf_sets() {
    let mut f = fixture();
    f.doc.set_tag_hidden(tag("Hardware"), true);
    f.doc.set_node_user_hidden(NodeId::Group(f.group), true);
    f.doc.set_section_plane(plane_z(0.5, true)).unwrap();
    let sid = f
        .doc
        .add_scene(
            None,
            SceneProps::ALL,
            Some(cam([1.0, 2.0, 3.0])),
            Some(display(false)),
            None,
        )
        .unwrap();

    // Drift everything away.
    f.doc.set_tag_hidden(tag("Hardware"), false);
    f.doc.set_node_user_hidden(NodeId::Group(f.group), false);
    f.doc.set_node_user_hidden(NodeId::Object(f.plain), true);
    f.doc.clear_section_plane();

    let resolved_pure = f.doc.resolve_scene(sid).unwrap();
    let before = f.doc.undo_depth();
    let resolved = f.doc.apply_scene(sid).unwrap();
    assert_eq!(
        resolved, resolved_pure,
        "apply returns exactly what resolve predicted"
    );
    assert_eq!(f.doc.undo_depth(), before);

    assert!(f.doc.tag_hidden(&tag("Hardware")));
    assert!(f.doc.node_user_hidden(NodeId::Group(f.group)));
    assert!(
        !f.doc.node_user_hidden(NodeId::Object(f.plain)),
        "a full set clears extras"
    );
    assert_eq!(f.doc.section_plane(), Some(plane_z(0.5, true)));

    assert_eq!(resolved.camera, Some(cam([1.0, 2.0, 3.0])));
    assert_eq!(resolved.display, Some(display(false)));
    let objs: BTreeSet<_> = resolved
        .hidden_object_ids
        .clone()
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        objs,
        BTreeSet::from([f.tagged, f.grouped]),
        "tag + group leaf expansion"
    );
    assert_eq!(resolved.hidden_instance_ids, Some(vec![]));
    assert_eq!(resolved.hidden_tag_paths, Some(vec![tag("Hardware")]));
    assert_eq!(resolved.hidden_nodes, Some(vec![NodeId::Group(f.group)]));

    // Idempotent.
    let again = f.doc.apply_scene(sid).unwrap();
    assert_eq!(again, resolved);
}

#[test]
fn uncaptured_properties_are_left_alone_on_apply() {
    let mut f = fixture();
    let props = SceneProps {
        camera: true,
        hidden_nodes: false,
        hidden_tags: true,
        section: false,
        display: false,
    };
    f.doc.set_tag_hidden(tag("Hardware"), true);
    let sid = f
        .doc
        .add_scene(None, props, Some(cam([1.0, 1.0, 1.0])), None, None)
        .unwrap();
    // Now hide a node and place a plane AFTER capture — the scene doesn't
    // capture those, so applying it must not touch them.
    f.doc.set_node_user_hidden(NodeId::Object(f.plain), true);
    f.doc.set_section_plane(plane_z(0.2, true)).unwrap();
    f.doc.set_tag_hidden(tag("Hardware"), false);

    let resolved = f.doc.apply_scene(sid).unwrap();
    assert!(
        f.doc.tag_hidden(&tag("Hardware")),
        "captured tag state restored"
    );
    assert!(
        f.doc.node_user_hidden(NodeId::Object(f.plain)),
        "uncaptured hidden nodes untouched"
    );
    assert_eq!(
        f.doc.section_plane(),
        Some(plane_z(0.2, true)),
        "uncaptured section untouched"
    );
    assert_eq!(resolved.section, None);
    assert_eq!(resolved.display, None);
    assert_eq!(resolved.hidden_nodes, None);
    // Leaf sets describe the POST-apply document: the captured tag half plus
    // the live (uncaptured) user-hidden nodes, so pushing them to the
    // renderer never un-hides what the kernel still holds hidden.
    let objs: BTreeSet<_> = resolved
        .hidden_object_ids
        .clone()
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(objs, BTreeSet::from([f.tagged, f.plain]));
}

#[test]
fn set_scene_props_captures_new_and_drops_unchecked() {
    let mut f = fixture();
    let sid = f
        .doc
        .add_scene(
            None,
            SceneProps::ALL,
            Some(cam([1.0, 1.0, 1.0])),
            Some(display(true)),
            None,
        )
        .unwrap();
    let mut props = SceneProps::ALL;
    props.camera = false;
    props.section = false;
    f.doc.set_scene_props(sid, props, None, None).unwrap();
    let scene = f.doc.scene(sid).unwrap();
    assert_eq!(scene.camera, None);
    assert_eq!(scene.section, None);
    assert!(scene.hidden_tags.is_some());
    // Turning camera back on captures the supplied camera now.
    props.camera = true;
    f.doc
        .set_scene_props(sid, props, Some(cam([9.0, 9.0, 9.0])), None)
        .unwrap();
    assert_eq!(f.doc.scene(sid).unwrap().camera, Some(cam([9.0, 9.0, 9.0])));
}

#[test]
fn update_recaptures_only_flagged_properties() {
    let mut f = fixture();
    let sid = f
        .doc
        .add_scene(
            None,
            SceneProps::ALL,
            Some(cam([1.0, 1.0, 1.0])),
            Some(display(true)),
            None,
        )
        .unwrap();
    f.doc.set_tag_hidden(tag("Hardware"), true);
    let mut only_tags = SceneProps::NONE;
    only_tags.hidden_tags = true;
    f.doc
        .update_scene(sid, only_tags, Some(cam([7.0, 7.0, 7.0])), None)
        .unwrap();
    let scene = f.doc.scene(sid).unwrap();
    assert_eq!(
        scene.camera,
        Some(cam([1.0, 1.0, 1.0])),
        "camera not flagged, unchanged"
    );
    let tag_sid = f.doc.sid_of(&EntityRef::Tag(tag("Hardware"))).unwrap();
    assert_eq!(scene.hidden_tags, Some(BTreeSet::from([tag_sid])));
}

// ------------------------------------------------------------------ drift

#[test]
fn drift_reports_each_property_and_tolerates_camera_noise() {
    let mut f = fixture();
    let sid = f
        .doc
        .add_scene(
            None,
            SceneProps::ALL,
            Some(cam([1.0, 1.0, 1.0])),
            Some(display(true)),
            None,
        )
        .unwrap();
    let live = cam([1.0, 1.0, 1.0]);
    let none = f
        .doc
        .scene_drift(sid, Some(&live), Some(&display(true)))
        .unwrap();
    assert!(!none.any());

    // Float noise well inside tolerance → no drift.
    let mut noisy = live;
    noisy.eye = Point3::new(1.0 + tol::SCENE_CAMERA_POSITION * 0.1, 1.0, 1.0);
    assert!(
        !f.doc
            .scene_drift(sid, Some(&noisy), None)
            .unwrap()
            .props
            .camera
    );
    // A real move → camera drift only.
    let moved = cam([3.0, 1.0, 1.0]);
    let d = f
        .doc
        .scene_drift(sid, Some(&moved), Some(&display(true)))
        .unwrap();
    assert!(d.props.camera && !d.props.display && !d.props.hidden_tags && !d.props.section);

    f.doc.set_tag_hidden(tag("Hardware"), true);
    assert!(
        f.doc
            .scene_drift(sid, None, None)
            .unwrap()
            .props
            .hidden_tags
    );
    f.doc.set_tag_hidden(tag("Hardware"), false);
    f.doc.set_node_user_hidden(NodeId::Object(f.plain), true);
    assert!(
        f.doc
            .scene_drift(sid, None, None)
            .unwrap()
            .props
            .hidden_nodes
    );
    f.doc.set_node_user_hidden(NodeId::Object(f.plain), false);
    f.doc.set_section_plane(plane_z(0.1, true)).unwrap();
    assert!(f.doc.scene_drift(sid, None, None).unwrap().props.section);
    f.doc.clear_section_plane();
    assert!(
        f.doc
            .scene_drift(sid, None, Some(&display(false)))
            .unwrap()
            .props
            .display
    );
    assert!(
        !f.doc.scene_drift(sid, None, None).unwrap().any(),
        "None inputs never drift"
    );
}

// -------------------------------------------------- dangling references

#[test]
fn deleted_node_is_stale_in_memory_pruned_on_save_relinked_by_undo() {
    let mut f = fixture();
    f.doc.set_node_user_hidden(NodeId::Object(f.plain), true);
    f.doc.set_node_user_hidden(NodeId::Group(f.group), true);
    let sid = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    let plain_sid = f.doc.sid_of(&EntityRef::Object(f.plain)).unwrap();
    let group_sid = f.doc.sid_of(&EntityRef::Group(f.group)).unwrap();

    f.doc.delete_node(NodeId::Object(f.plain)).expect("delete");
    // In memory the sid stays; drift counts it stale; resolve skips it.
    assert!(
        f.doc
            .scene(sid)
            .unwrap()
            .hidden_nodes
            .as_ref()
            .unwrap()
            .contains(&plain_sid)
    );
    assert_eq!(f.doc.scene_drift(sid, None, None).unwrap().stale_refs, 1);
    let resolved = f.doc.resolve_scene(sid).unwrap();
    assert_eq!(resolved.hidden_nodes, Some(vec![NodeId::Group(f.group)]));

    // The FILE never carries the dead sid.
    let bytes = f.doc.save();
    let m = manifest_json(&bytes);
    let stored: Vec<u64> = m["scenes"][0]["hidden_nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(stored, vec![group_sid]);
    let loaded = Document::load(&bytes).expect("pruned file loads");
    assert_eq!(loaded.scene_drift(sid, None, None).unwrap().stale_refs, 0);

    // Undo the deletion → the same sid comes back and re-links.
    f.doc.undo().expect("undo delete");
    assert_eq!(f.doc.scene_drift(sid, None, None).unwrap().stale_refs, 0);
    let objs: BTreeSet<_> = f
        .doc
        .resolve_scene(sid)
        .unwrap()
        .hidden_object_ids
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(objs, BTreeSet::from([f.plain, f.grouped]));
}

#[test]
fn deleted_tag_is_pruned_on_save() {
    let mut f = fixture();
    f.doc.set_tag_hidden(tag("Hardware"), true);
    let sid = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    f.doc.delete_tag(&tag("Hardware")).expect("delete tag");
    assert_eq!(f.doc.scene_drift(sid, None, None).unwrap().stale_refs, 1);
    let bytes = f.doc.save();
    let m = manifest_json(&bytes);
    assert_eq!(m["scenes"][0]["hidden_tags"], serde_json::json!([]));
    Document::load(&bytes).expect("loads");
}

#[test]
fn a_dangling_scene_reference_in_a_file_is_rejected() {
    let mut f = fixture();
    f.doc.set_node_user_hidden(NodeId::Object(f.plain), true);
    f.doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    let bytes = f.doc.save();
    let patched = patch_manifest(&bytes, |m| {
        m["scenes"][0]["hidden_nodes"] = serde_json::json!([987654]);
    });
    assert!(matches!(
        Document::load(&patched),
        Err(LoadError::DanglingReference { .. })
    ));
}

// ---------------------------------------------------------- file format

#[test]
fn scenes_round_trip_byte_identical_with_every_property_shape() {
    let mut f = fixture();
    f.doc.set_tag_hidden(tag("Hardware"), true);
    f.doc.set_node_user_hidden(NodeId::Group(f.group), true);
    f.doc.set_section_plane(plane_z(0.5, false)).unwrap();
    let full = f
        .doc
        .add_scene(
            Some("Assembled".into()),
            SceneProps::ALL,
            Some(cam([5.0, 5.0, 5.0])),
            Some(display(true)),
            None,
        )
        .unwrap();
    f.doc
        .set_scene_description(full, "Everything, three-quarter view.".into())
        .unwrap();
    // Captured-no-plane vs not-captured must survive the round trip.
    f.doc.clear_section_plane();
    let none_plane = f
        .doc
        .add_scene(Some("No plane".into()), SceneProps::ALL, None, None, None)
        .unwrap();
    let mut only_cam = SceneProps::NONE;
    only_cam.camera = true;
    let cam_only = f
        .doc
        .add_scene(
            Some("Cam only".into()),
            only_cam,
            Some(cam([0.0, -9.0, 2.0])),
            None,
            None,
        )
        .unwrap();
    f.doc.move_scene(cam_only, 0).unwrap();

    let bytes = f.doc.save();
    let m = manifest_json(&bytes);
    assert_eq!(m["format_version"], 16);
    assert_eq!(m["scenes"].as_array().unwrap().len(), 3);
    assert_eq!(m["scenes"][0]["name"], "Cam only");
    assert!(
        m["scenes"][0].get("hidden_nodes").is_none(),
        "uncaptured = absent"
    );
    assert!(m["scenes"][0].get("section").is_none());
    assert_eq!(m["scenes"][1]["section"]["active"], false);
    assert!(m["scenes"][2].get("section").is_some());
    assert!(
        m["scenes"][2]["section"].is_null(),
        "captured-no-plane = null"
    );

    let loaded = Document::load(&bytes).expect("load");
    assert_eq!(loaded.scenes(), f.doc.scenes());
    assert_eq!(loaded.scene(none_plane).unwrap().section, Some(None));
    assert_eq!(loaded.save(), bytes, "byte-identical after reload");
}

#[test]
fn a_pre_v16_manifest_carrying_scenes_is_rejected() {
    let mut f = fixture();
    f.doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    let bytes = f.doc.save();
    let patched = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(15);
    });
    assert!(matches!(
        Document::load(&patched),
        Err(LoadError::MalformedManifest { .. })
    ));
    let mut g = fixture();
    g.doc.set_section_plane(plane_z(0.5, true)).unwrap();
    let bytes = g.doc.save();
    let patched = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(15);
    });
    assert!(matches!(
        Document::load(&patched),
        Err(LoadError::MalformedManifest { .. })
    ));
}

#[test]
fn duplicate_scene_name_or_sid_in_a_file_is_rejected() {
    let mut f = fixture();
    let a = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    f.doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    let bytes = f.doc.save();
    let dup_name = patch_manifest(&bytes, |m| {
        m["scenes"][1]["name"] = serde_json::json!("Scene 1");
    });
    assert!(matches!(
        Document::load(&dup_name),
        Err(LoadError::MalformedManifest { .. })
    ));
    let dup_sid = patch_manifest(&bytes, |m| {
        m["scenes"][1]["sid"] = serde_json::json!(a);
    });
    assert!(matches!(
        Document::load(&dup_sid),
        Err(LoadError::MalformedManifest { .. })
    ));
    // A scene sid colliding with an ENTITY sid is malformed too.
    let entity_sid = f.doc.sid_of(&EntityRef::Object(f.plain)).unwrap();
    let clash = patch_manifest(&bytes, |m| {
        m["scenes"][1]["sid"] = serde_json::json!(entity_sid);
    });
    assert!(matches!(
        Document::load(&clash),
        Err(LoadError::MalformedManifest { .. })
    ));
}

#[test]
fn loaded_scene_sids_advance_the_mint_counter() {
    let mut f = fixture();
    let sid = f
        .doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    let mut loaded = Document::load(&f.doc.save()).unwrap();
    let later = extrude_box(&mut loaded, 6.0, 0.0, 7.0, 1.0, 1.0);
    assert!(loaded.sid_of(&EntityRef::Object(later)).unwrap() > sid);
    let next = loaded
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();
    assert!(next > sid);
}
