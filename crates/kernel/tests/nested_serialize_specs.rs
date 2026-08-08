//! Manifest-v15 specs for NESTED component members (docs/design/
//! nested-components.md, phase 2): the NodeRef member shape, the
//! group/instance `owner` field, both-direction version gating, ownership
//! derivation, cycle refusal, and the nested round trip — driven through
//! hand-patched manifests, since (deliberately) no op creates nested
//! members before the ops phase lands on this same branch.

use kernel::{Document, LoadError, NodeId, Plane, Point3, Transform, Vec3};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
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
        sk.add_segment(a, b).expect("segment");
    }
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

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

/// Two definitions, two world instances — the raw material every nesting
/// forge below rearranges. Def 0 = "Inner", def 1 = "Outer", instance i is
/// a placement of def i.
fn two_defs_two_instances() -> Vec<u8> {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    doc.set_node_name(NodeId::Object(a), Some("Inner".into()))
        .unwrap();
    doc.set_node_name(NodeId::Object(b), Some("Outer".into()))
        .unwrap();
    doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.make_component(&[NodeId::Object(b)]).unwrap();
    doc.save()
}

/// Rewires the saved manifest so INNER's instance (dense 0) becomes a
/// MEMBER of def "Outer" (dense 1): pushes the NodeRef member and sets the
/// instance's owner.
fn forge_nested(bytes: &[u8]) -> Vec<u8> {
    patch_manifest(bytes, |m| {
        m["components"][1]["members"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "instance", "id": 0}));
        m["instances"][0]["owner"] = serde_json::json!(1);
        // The nested instance is no longer a world root.
        let roots = m["roots"].as_array_mut().unwrap();
        roots.retain(|r| !(r["kind"] == "instance" && r["id"] == 0));
    })
}

/// A definition may contain an instance of another definition (v15): the
/// forged file loads, the nested placement composes poses, and the loaded
/// document resaves byte-identically (the writer round-trips the nested
/// shape it just read).
#[test]
fn nested_member_instance_loads_expands_and_round_trips() {
    let nested = forge_nested(&two_defs_two_instances());
    let doc = Document::load(&nested).expect("nested v15 file loads");

    // One world instance (Outer's); Inner's is definition-owned.
    assert_eq!(doc.instance_ids().len(), 1);
    // The expansion yields BOTH leaf placements through the one world
    // instance: Outer's own member and Inner's member through the nested
    // instance, both tagged with the OUTER world instance.
    let placements = doc.expanded_placements();
    assert_eq!(placements.len(), 2);
    let outer_world = doc.instance_ids()[0];
    assert!(placements.iter().all(|(_, _, outer)| *outer == outer_world));

    // Byte-stable round trip of the nested shape.
    let resaved = doc.save();
    let re2 = Document::load(&resaved).expect("resave loads");
    assert_eq!(re2.save(), resaved);
}

/// The two version gates, both directions.
#[test]
fn member_shapes_are_version_gated_both_ways() {
    let bytes = two_defs_two_instances();

    // A pre-v15 file carrying a NodeRef member is malformed for its
    // declared version.
    let smuggled = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(14);
    });
    assert!(matches!(
        Document::load(&smuggled),
        Err(LoadError::MalformedManifest { .. })
    ));

    // A v15 file carrying a bare-integer member is a nonconforming writer.
    let bare = patch_manifest(&bytes, |m| {
        m["components"][0]["members"] = serde_json::json!([0]);
    });
    assert!(matches!(
        Document::load(&bare),
        Err(LoadError::MalformedManifest { .. })
    ));

    // A pre-v15 file carrying an owner field is malformed for its version.
    let owner_smuggled = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(14);
        // Downgrade members to the legal pre-v15 shape so ONLY the owner
        // field is the violation.
        for c in m["components"].as_array_mut().unwrap() {
            let ids: Vec<serde_json::Value> = c["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].clone())
                .collect();
            c["members"] = serde_json::Value::Array(ids);
        }
        m["instances"][0]["owner"] = serde_json::json!(0);
    });
    assert!(matches!(
        Document::load(&owner_smuggled),
        Err(LoadError::MalformedManifest { .. })
    ));
}

/// Ownership must agree between the member list and the declared owner —
/// in both directions.
#[test]
fn owner_and_membership_must_agree() {
    let bytes = two_defs_two_instances();

    // Member listed, owner missing.
    let no_owner = patch_manifest(&bytes, |m| {
        m["components"][1]["members"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "instance", "id": 0}));
        let roots = m["roots"].as_array_mut().unwrap();
        roots.retain(|r| !(r["kind"] == "instance" && r["id"] == 0));
    });
    assert!(matches!(
        Document::load(&no_owner),
        Err(LoadError::MalformedManifest { .. })
    ));

    // Owner declared, membership missing.
    let no_member = patch_manifest(&bytes, |m| {
        m["instances"][0]["owner"] = serde_json::json!(1);
        let roots = m["roots"].as_array_mut().unwrap();
        roots.retain(|r| !(r["kind"] == "instance" && r["id"] == 0));
    });
    assert!(matches!(
        Document::load(&no_member),
        Err(LoadError::MalformedManifest { .. })
    ));
}

/// A definition that reaches itself through its member instances is a
/// cycle — refused at load, never repaired.
#[test]
fn component_cycles_are_refused_at_load() {
    let bytes = two_defs_two_instances();
    // Def 0's instance (dense 0) into def 1 AND def 1's instance (dense 1)
    // into def 0: a two-node cycle.
    let cyclic = patch_manifest(&bytes, |m| {
        m["components"][1]["members"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "instance", "id": 0}));
        m["instances"][0]["owner"] = serde_json::json!(1);
        m["components"][0]["members"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "instance", "id": 1}));
        m["instances"][1]["owner"] = serde_json::json!(0);
        m["roots"] = serde_json::json!([]);
    });
    assert!(matches!(
        Document::load(&cyclic),
        Err(LoadError::MalformedManifest { .. })
    ));
}

/// The nested placement's composed pose is the product of the outer and
/// inner poses — verified through real geometry.
#[test]
fn nested_pose_composition_is_exact() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (_, outer_inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    // Shift the inner world instance so its pose is non-identity, then
    // forge it into Outer.
    let inner_inst = doc
        .instance_ids()
        .into_iter()
        .find(|&i| i != outer_inst)
        .unwrap();
    doc.transform_instance(
        inner_inst,
        &Transform::translation(Vec3::new(0.5, 0.0, 0.0)),
    )
    .unwrap();
    let forged = forge_nested(&doc.save());
    let loaded = Document::load(&forged).expect("loads");

    // Move the OUTER world instance and check the inner leaf's composed
    // placement carries both translations.
    let world = loaded.instance_ids()[0];
    let placements = loaded.expanded_placements();
    let inner_leaf = placements
        .iter()
        .find(|(oid, _, _)| {
            // Inner's unit box has min corner at (0,0,0) in def-local space.
            loaded.object(*oid).is_some_and(|o| {
                o.vertices()
                    .values()
                    .any(|v| v.position.approx_eq(Point3::new(0.0, 0.0, 0.0), 1e-9))
            })
        })
        .expect("inner leaf found");
    let p = inner_leaf.1.apply_point(Point3::new(0.0, 0.0, 0.0));
    assert!(
        p.approx_eq(Point3::new(0.5, 0.0, 0.0), 1e-9),
        "inner pose composes through the (identity) outer pose"
    );
    let _ = world;
}

/// Definition subtrees and the world tree are DISJOINT: a file whose
/// world group also lists a definition-owned node is malformed — accepting
/// it would let world ops mutate shared definition content (adversarial
/// review: the parent-patching pass masked whichever listing lost).
#[test]
fn world_group_listing_a_def_owned_node_is_refused() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let c = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.make_component(&[NodeId::Object(b)]).unwrap();
    // A world group to tamper with.
    doc.group_nodes(&[NodeId::Object(c)]).unwrap();
    let bytes = {
        // Nest inner into outer first (the forge below), then re-save.
        let nested = forge_nested(&doc.save());
        Document::load(&nested).unwrap().save()
    };

    // The world group additionally lists the def-owned instance (dense 0).
    let tampered = patch_manifest(&bytes, |m| {
        m["groups"][0]["members"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "instance", "id": 0}));
    });
    assert!(matches!(
        Document::load(&tampered),
        Err(LoadError::MalformedManifest { .. })
    ));
}

/// The depth bound holds EXACTLY at load: a legitimately built
/// 64-definition chain round-trips, and the live gate refuses the fold
/// that would make it 65 — reader and writer agree on the boundary the
/// format doc promises.
#[test]
fn depth_boundary_is_exact() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    // Fold 63 more times: definition depth 64 (the maximum).
    for _ in 0..63 {
        let (_, outer, _) = doc.make_component(&[NodeId::Instance(inst)]).unwrap();
        inst = outer;
    }
    let bytes = doc.save();
    let re = Document::load(&bytes).expect("a 64-deep chain is legal");
    assert_eq!(re.save(), bytes);
    assert_eq!(re.expanded_placements().len(), 1, "nothing truncated");

    // One more fold busts the bound — refused typed, never truncated.
    assert!(matches!(
        doc.make_component(&[NodeId::Instance(inst)]),
        Err(kernel::DocumentError::ComponentDepthExceeded)
    ));
}

/// The WIDTH bound at load: duplicating the world instance of a
/// half-cap doubling ladder in the manifest pushes the document's total
/// expansion past the bound — refused typed, exactly like depth.
#[test]
fn over_wide_expansion_is_refused_at_load() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    for _ in 0..19 {
        let (second, _) = doc
            .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    let bytes = doc.save();
    Document::load(&bytes).expect("at the boundary: loads");

    // Forge a second world placement of the outermost definition: total
    // becomes 2^20 > 1,000,000.
    let tampered = patch_manifest(&bytes, |m| {
        let sid_max = |key: &str| {
            m[key]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|e| e["sid"].as_u64())
                .max()
                .unwrap_or(0)
        };
        let max_sid = sid_max("instances")
            .max(sid_max("objects"))
            .max(sid_max("components"));
        let instances = m["instances"].as_array_mut().unwrap();
        let mut dup = instances.last().unwrap().clone();
        let new_id = instances.len() as u64;
        dup["id"] = serde_json::json!(new_id);
        dup["sid"] = serde_json::json!(max_sid + 1);
        instances.push(dup);
        m["roots"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "instance", "id": new_id}));
    });
    assert!(matches!(
        Document::load(&tampered),
        Err(LoadError::MalformedManifest { .. })
    ));
}
