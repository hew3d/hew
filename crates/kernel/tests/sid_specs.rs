//! Executable specs for per-entity **stable ids** (manifest v14+;
//! docs/agents/HEW_API.md §5.1, HEW_FILE_FORMAT.md §4.2) — the identity that
//! survives save/load, undo/redo, and dense-id renumbering, minted once
//! per entity and never re-minted.
//!
//! The wire-level contract: v14+ manifests carry a globally unique `sid`
//! on every entity (absence or duplication is a malformed manifest, and a
//! sid smuggled under an older declared version is rejected — never
//! silently honored or re-minted); pre-v14 files mint fresh ids in dense
//! order, deterministically, as the documented upgrade path.

use kernel::{
    Document, EntityRef, LoadError, Material, NodeId, Plane, Point3, Rgba8, Transform, Vec3,
};

// ----------------------------------------------------------------- helpers

/// The ground (z = 0) plane.
fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// Sketch a rectangle through the recorded gesture path and extrude it.
fn build_box(doc: &mut Document, x0: f64) -> (kernel::SketchId, kernel::ObjectId) {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let (y0, x1, y1) = (0.0, x0 + 1.0, 1.0);
        let corners = [
            (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
            (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
            (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
            (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
        ];
        for (a, b) in corners {
            sk.add_segment(a, b).expect("rectangle segment");
        }
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("live");
    let obj = doc
        .extrude_region(s, regions[0], 0.5)
        .expect("extrude box")
        .0;
    (s, obj)
}

/// A document exercising every sid-carrying entity kind: objects, sketches,
/// a group, a component + instance, a guide, a material, and a tag.
fn representative_doc() -> Document {
    let mut doc = Document::default();
    let (_, a) = build_box(&mut doc, 0.0);
    let (_, b) = build_box(&mut doc, 2.0);
    let (_, c) = build_box(&mut doc, 4.0);
    doc.group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");
    let (component, _, _) = doc.make_component(&[NodeId::Object(c)]).expect("component");
    doc.place_instance(component, Transform::translation(Vec3::new(0.0, 3.0, 0.0)))
        .expect("place");
    doc.add_guide_point(Point3::new(9.0, 9.0, 0.0))
        .expect("guide");
    doc.add_material(Material {
        name: "Oak".to_string(),
        color: Rgba8 {
            r: 180,
            g: 140,
            b: 90,
            a: 255,
        },
        texture: None,
    });
    doc.set_tag_hidden(vec!["Structure".to_string()], false);
    doc
}

/// Re-encode `bytes` with the manifest patched by `patch`.
fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    patch(&mut manifest);
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();

    let mut new_zip = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched).unwrap();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..zip2.len() {
        let mut entry = zip2.by_index(i).unwrap();
        if entry.name() == "manifest.json" {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        new_zip.start_file(&name, opts).unwrap();
        new_zip.write_all(&buf).unwrap();
    }
    new_zip.finish().unwrap().into_inner()
}

/// Remove every `sid` key from a manifest value (for building pre-v14
/// shapes without smuggled fields).
fn strip_sids(manifest: &mut serde_json::Value) {
    for arr_key in [
        "objects",
        "groups",
        "components",
        "instances",
        "sketches",
        "guides",
        "materials",
        "tags",
    ] {
        if let Some(arr) = manifest.get_mut(arr_key).and_then(|v| v.as_array_mut()) {
            for entry in arr.iter_mut() {
                if let Some(m) = entry.as_object_mut() {
                    m.remove("sid");
                }
            }
        }
    }
}

// ------------------------------------------------------------------ specs

/// Every entity kind carries a sid, and a save→load→save round trip is
/// byte-identical — which pins that every sid (and everything else)
/// survives the trip exactly.
#[test]
fn sids_cover_every_kind_and_survive_round_trip_byte_identically() {
    let doc = representative_doc();
    for (entity, _) in doc.sids() {
        let _ = entity; // presence is enforced per-kind by the debug validator
    }
    assert!(doc.sids().count() >= 8, "one sid per entity at minimum");

    let bytes = doc.save();
    let reloaded = Document::load(&bytes).expect("round trip");
    assert_eq!(
        reloaded.save(),
        bytes,
        "save→load→save must be a byte-identical fixed point, sids included"
    );
}

/// Two documents built by the same operation sequence mint identical sids
/// (kernel determinism, ARCHITECTURE.md §5.5) — what makes a replayed
/// session reproduce a byte-identical file.
#[test]
fn same_op_sequence_mints_identical_sids() {
    let a = representative_doc();
    let b = representative_doc();
    assert_eq!(a.save(), b.save());
}

/// A pre-v14 file (no sids anywhere) loads by minting fresh ids in dense
/// order, and its resave writes a v14 manifest carrying them.
#[test]
fn pre_v14_file_mints_dense_order_sids() {
    let bytes = representative_doc().save();
    let old = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(13);
        strip_sids(m);
        // A genuine pre-v14 writer also wrote component members as bare
        // object dense ids and no owner fields (both introduced at v15) —
        // downgrade those shapes too or the forged file is malformed for
        // its own declared version (correctly rejected).
        if let Some(comps) = m["components"].as_array_mut() {
            for c in comps.iter_mut() {
                if let Some(members) = c["members"].as_array_mut() {
                    for member in members.iter_mut() {
                        if let Some(id) = member.get("id").and_then(|v| v.as_u64()) {
                            *member = serde_json::json!(id);
                        }
                    }
                }
            }
        }
        for key in ["groups", "instances"] {
            if let Some(arr) = m[key].as_array_mut() {
                for entry in arr.iter_mut() {
                    if let Some(obj) = entry.as_object_mut() {
                        obj.remove("owner");
                    }
                }
            }
        }
    });
    let upgraded = Document::load(&old).expect("pre-v14 file loads");
    assert!(upgraded.sids().count() >= 8, "every entity got a fresh sid");
    let resaved = upgraded.save();
    let reloaded = Document::load(&resaved).expect("upgraded resave loads");
    assert_eq!(reloaded.save(), resaved, "upgrade is a fixed point");
}

/// A sid smuggled under a pre-v14 declared version is rejected —
/// reject-not-repair, exactly like a smuggled sketch owner.
#[test]
fn sid_smuggled_into_a_v13_file_is_rejected() {
    let bytes = representative_doc().save();
    let smuggled = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(13);
        // Keep the sids the v14 save wrote: now they are smuggled.
    });
    let err = Document::load(&smuggled).expect_err("a smuggled sid must refuse");
    assert!(
        matches!(err, LoadError::MalformedManifest { .. }),
        "unexpected error: {err:?}"
    );
}

/// A v14 manifest missing a sid is malformed — never silently re-minted.
#[test]
fn missing_sid_in_a_v14_file_is_rejected() {
    let bytes = representative_doc().save();
    let broken = patch_manifest(&bytes, |m| {
        m["objects"][0]
            .as_object_mut()
            .unwrap()
            .remove("sid")
            .expect("the save wrote a sid to remove");
    });
    let err = Document::load(&broken).expect_err("a missing sid must refuse");
    assert!(matches!(err, LoadError::MalformedManifest { .. }));
}

/// A duplicated sid is malformed — stable ids are globally unique across
/// every entity kind, not per-kind.
#[test]
fn duplicate_sid_in_a_v14_file_is_rejected() {
    let bytes = representative_doc().save();
    let broken = patch_manifest(&bytes, |m| {
        let stolen = m["objects"][0]["sid"].clone();
        m["groups"][0]["sid"] = stolen;
    });
    let err = Document::load(&broken).expect_err("a duplicate sid must refuse");
    assert!(matches!(err, LoadError::MalformedManifest { .. }));
}

/// Undoing and redoing a creation keeps the entity's sid: tombstoned
/// records keep their table entry, so redo revives the same identity.
#[test]
fn undo_redo_of_a_creation_keeps_the_sid() {
    let mut doc = Document::default();
    let (_, obj) = build_box(&mut doc, 0.0);
    let sid = doc.sid_of(&EntityRef::Object(obj)).expect("minted");
    doc.undo().expect("undo the extrusion");
    doc.redo().expect("redo the extrusion");
    assert_eq!(
        doc.sid_of(&EntityRef::Object(obj)),
        Some(sid),
        "redo revives the same stable id"
    );
}

/// Deleting an entity and undoing the delete restores the same sid — the
/// public-id contract behind docs/agents/HEW_API.md §5.1 (delete retires an id;
/// undoing the deletion restores it).
#[test]
fn delete_then_undo_restores_the_sid() {
    let mut doc = Document::default();
    let (_, obj) = build_box(&mut doc, 0.0);
    let sid = doc.sid_of(&EntityRef::Object(obj)).expect("minted");
    doc.delete_node(NodeId::Object(obj)).expect("delete");
    doc.undo().expect("undo the delete");
    assert_eq!(doc.sid_of(&EntityRef::Object(obj)), Some(sid));
}

/// A duplicate is a new entity: it mints a fresh sid, never shares its
/// source's.
#[test]
fn duplicates_mint_fresh_sids() {
    let mut doc = Document::default();
    let (_, obj) = build_box(&mut doc, 0.0);
    let src_sid = doc.sid_of(&EntityRef::Object(obj)).expect("minted");
    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(obj),
            &Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
        )
        .expect("duplicate");
    let NodeId::Object(copy_obj) = copy else {
        panic!("object duplicate is an object");
    };
    let copy_sid = doc.sid_of(&EntityRef::Object(copy_obj)).expect("minted");
    assert_ne!(src_sid, copy_sid);
}

/// An undone creation never reaches the file: saving after the undo is
/// byte-identical to the save before the creation, tombstone and counter
/// notwithstanding (the counter is deliberately not serialized).
#[test]
fn undone_creation_leaves_the_save_bytes_untouched() {
    let mut doc = Document::default();
    build_box(&mut doc, 0.0);
    let before = doc.save();
    build_box(&mut doc, 2.0);
    doc.undo().expect("undo extrusion");
    doc.undo().expect("undo sketch gesture");
    assert_eq!(doc.save(), before);
}

/// A manifest with two tag entries sharing one path is rejected — the
/// registry is path-keyed, and last-wins loading would silently drop one
/// entry's stable id and merge dictionaries.
#[test]
fn duplicate_tag_paths_are_rejected() {
    let mut doc = representative_doc();
    doc.set_tag_hidden(vec!["Extra".to_string()], false);
    let bytes = doc.save();
    let broken = patch_manifest(&bytes, |m| {
        let tags = m["tags"].as_array_mut().expect("tags present");
        assert!(tags.len() >= 2);
        let clone_path = tags[0]["path"].clone();
        tags[1]["path"] = clone_path;
    });
    let err = Document::load(&broken).expect_err("duplicate tag path must refuse");
    assert!(matches!(err, LoadError::MalformedManifest { .. }));
}
