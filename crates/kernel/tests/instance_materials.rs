//! Specs for per-instance materials (docs/agents/PLUGINS.md "Materials";
//! docs/dev/HEW_FILE_FORMAT.md §4.4): an instance carries its own material,
//! which a member face shows only when neither the face nor its Object's
//! base is painted, and the nearest painted instance on the path wins.

use kernel::{
    ComponentId, Document, DocumentError, InsertOptions, InstanceId, LoadError, Material,
    MaterialId, MaterialScope, NodeId, ObjectId, Plane, Point3, Rgba8, Transform, Vec3,
};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn extrude_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
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

fn solid(doc: &mut Document, name: &str, r: u8) -> MaterialId {
    doc.add_material(Material::solid(name, Rgba8::rgb(r, 10, 10)))
}

/// A component holding one box, placed once.
fn one_component(doc: &mut Document) -> (ComponentId, InstanceId, ObjectId) {
    let b = extrude_box(doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (c, i, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    (c, i, b)
}

/// Three levels: `a` holds a box, `b` holds `a1` (an instance of `a`),
/// `c` holds `b1` (an instance of `b`), placed in the world as `c1`.
struct Nest {
    doc: Document,
    leaf: ObjectId,
    a1: InstanceId,
    b1: InstanceId,
    c1: InstanceId,
}

fn nest() -> Nest {
    let mut doc = Document::new();
    let leaf = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_a, a1, _) = doc.make_component(&[NodeId::Object(leaf)]).unwrap();
    let (_b, b1, _) = doc.make_component(&[NodeId::Instance(a1)]).unwrap();
    let (_c, c1, _) = doc.make_component(&[NodeId::Instance(b1)]).unwrap();
    Nest {
        doc,
        leaf,
        a1,
        b1,
        c1,
    }
}

#[test]
fn set_and_clear_an_instance_material_with_undo_and_redo() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    assert_eq!(doc.instance_material(inst), Some(None));

    let change = doc.set_instance_material(inst, Some(red)).unwrap();
    assert_eq!(change.instances_touched, vec![inst]);
    assert!(change.objects_touched.is_empty());
    assert_eq!(doc.instance_material(inst), Some(Some(red)));

    doc.undo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(None));
    doc.redo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(red)));

    doc.set_instance_material(inst, None).unwrap();
    assert_eq!(doc.instance_material(inst), Some(None));
    doc.undo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(red)));
}

#[test]
fn set_instance_material_refuses_stale_instances_and_unknown_materials() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    doc.delete_material(red).unwrap();
    assert_eq!(
        doc.set_instance_material(inst, Some(red)),
        Err(DocumentError::UnknownMaterial)
    );
    let blue = solid(&mut doc, "Blue", 20);
    doc.delete_node(NodeId::Instance(inst)).unwrap();
    assert_eq!(doc.instance_material(inst), None);
    assert_eq!(
        doc.set_instance_material(inst, Some(blue)),
        Err(DocumentError::UnknownInstance)
    );
}

#[test]
fn a_nested_placement_takes_its_own_material_and_touches_its_outer_placements() {
    let mut n = nest();
    let red = solid(&mut n.doc, "Red", 200);
    let change = n.doc.set_instance_material(n.b1, Some(red)).unwrap();
    assert!(change.instances_touched.contains(&n.b1));
    assert!(
        change.instances_touched.contains(&n.c1),
        "the world placement that draws the nested one is touched too"
    );
    assert_eq!(n.doc.instance_material(n.b1), Some(Some(red)));
}

#[test]
fn inherited_material_is_the_innermost_painted_instance_on_the_path() {
    let mut n = nest();
    let red = solid(&mut n.doc, "Red", 200);
    let blue = solid(&mut n.doc, "Blue", 20);
    let path = [n.c1, n.b1, n.a1];

    assert_eq!(n.doc.inherited_material(&path), None, "nothing painted");

    n.doc.set_instance_material(n.b1, Some(red)).unwrap();
    assert_eq!(
        n.doc.inherited_material(&path),
        Some(red),
        "the middle level"
    );
    assert_eq!(n.doc.inherited_material(&[n.c1]), None);

    n.doc.set_instance_material(n.c1, Some(blue)).unwrap();
    assert_eq!(
        n.doc.inherited_material(&path),
        Some(red),
        "the nearer of two painted levels wins"
    );
    assert_eq!(n.doc.inherited_material(&[n.c1]), Some(blue));
    assert_eq!(n.doc.inherited_material(&[]), None);
}

#[test]
fn occurrences_report_own_and_effective_materials() {
    let mut n = nest();
    let red = solid(&mut n.doc, "Red", 200);
    let blue = solid(&mut n.doc, "Blue", 20);
    n.doc.set_instance_material(n.b1, Some(red)).unwrap();
    let rows = n.doc.occurrences(None, true).unwrap();
    let leaf = rows
        .iter()
        .find(|r| r.node == NodeId::Object(n.leaf) && r.path == vec![n.c1, n.b1, n.a1])
        .expect("the leaf row");
    assert_eq!(
        leaf.material, None,
        "an object row has no own instance material"
    );
    assert_eq!(leaf.material_effective, Some(red));
    let b1 = rows
        .iter()
        .find(|r| r.node == NodeId::Instance(n.b1))
        .expect("the nested placement row");
    assert_eq!(b1.material, Some(red));
    assert_eq!(b1.material_effective, Some(red));

    // A painted base beats every instance.
    n.doc.set_object_material(n.leaf, Some(blue)).unwrap();
    let rows = n.doc.occurrences(None, true).unwrap();
    let leaf = rows
        .iter()
        .find(|r| r.node == NodeId::Object(n.leaf))
        .unwrap();
    assert_eq!(leaf.material_effective, Some(blue));
}

#[test]
fn expanded_inherited_materials_run_parallel_to_the_expanded_members() {
    let mut n = nest();
    let red = solid(&mut n.doc, "Red", 200);
    let blue = solid(&mut n.doc, "Blue", 20);
    let def_c = n.doc.instance_def(n.c1).unwrap();
    assert_eq!(n.doc.instance_expanded_inherited(n.c1), vec![None]);
    n.doc.set_instance_material(n.c1, Some(blue)).unwrap();
    assert_eq!(n.doc.instance_expanded_inherited(n.c1), vec![Some(blue)]);
    n.doc.set_instance_material(n.a1, Some(red)).unwrap();
    assert_eq!(n.doc.instance_expanded_inherited(n.c1), vec![Some(red)]);
    assert_eq!(
        n.doc.expanded_def_placements_inherited(def_c, None).len(),
        n.doc.expanded_def_placements(def_c).len()
    );
}

#[test]
fn set_definition_material_paints_every_owned_leaf_in_one_entry() {
    let mut doc = Document::new();
    let inner_box = extrude_box(&mut doc, 10.0, 0.0, 11.0, 1.0, 1.0);
    let (_inner, inner_inst, _) = doc.make_component(&[NodeId::Object(inner_box)]).unwrap();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(b)]).unwrap();
    let (outer, _, _) = doc
        .make_component(&[
            NodeId::Object(a),
            NodeId::Group(g),
            NodeId::Instance(inner_inst),
        ])
        .unwrap();
    let red = solid(&mut doc, "Red", 200);
    let depth = doc.undo_depth();

    doc.set_definition_material(outer, Some(red)).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(doc.object(a).unwrap().default_material(), Some(red));
    assert_eq!(
        doc.object(b).unwrap().default_material(),
        Some(red),
        "a member group's leaf"
    );
    assert_eq!(
        doc.object(inner_box).unwrap().default_material(),
        None,
        "another definition's geometry is not this definition's"
    );

    doc.undo().unwrap();
    assert_eq!(doc.object(a).unwrap().default_material(), None);
    assert_eq!(doc.object(b).unwrap().default_material(), None);
    doc.redo().unwrap();
    assert_eq!(doc.object(a).unwrap().default_material(), Some(red));

    let stale = ComponentId::default();
    assert_eq!(
        doc.set_definition_material(stale, Some(red)),
        Err(DocumentError::UnknownComponent)
    );
}

#[test]
fn explode_bakes_the_instance_material_onto_unpainted_objects_only() {
    let mut doc = Document::new();
    let plain = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let based = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let red = solid(&mut doc, "Red", 200);
    let blue = solid(&mut doc, "Blue", 20);
    doc.set_object_material(based, Some(blue)).unwrap();
    let (_, inst, _) = doc
        .make_component(&[NodeId::Object(plain), NodeId::Object(based)])
        .unwrap();
    doc.set_instance_material(inst, Some(red)).unwrap();

    let (created, _) = doc.explode_instance(inst).unwrap();
    let mut bases: Vec<_> = created
        .iter()
        .map(|&o| doc.object(o).unwrap().default_material())
        .collect();
    bases.sort();
    let mut want = vec![Some(red), Some(blue)];
    want.sort();
    assert_eq!(
        bases, want,
        "the unpainted copy takes red, the painted keeps blue"
    );

    doc.undo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(red)));
    doc.redo().unwrap();
    let bases: Vec<_> = created
        .iter()
        .map(|&o| doc.object(o).unwrap().default_material())
        .collect();
    assert!(bases.contains(&Some(red)) && bases.contains(&Some(blue)));

    // Undo clears the bake: once the instance stops using red, nothing does.
    doc.undo().unwrap();
    doc.set_instance_material(inst, None).unwrap();
    assert!(
        doc.unused_materials().contains(&red),
        "undo must clear the baked material off the tombstoned copies"
    );
}

#[test]
fn explode_bakes_the_instance_material_onto_unpainted_nested_instances_only() {
    let mut doc = Document::new();
    let plain = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let painted = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (_, plain_inst, _) = doc.make_component(&[NodeId::Object(plain)]).unwrap();
    let (_, painted_inst, _) = doc.make_component(&[NodeId::Object(painted)]).unwrap();
    let red = solid(&mut doc, "Red", 200);
    let blue = solid(&mut doc, "Blue", 20);
    doc.set_instance_material(painted_inst, Some(blue)).unwrap();
    let (_, outer, _) = doc
        .make_component(&[NodeId::Instance(plain_inst), NodeId::Instance(painted_inst)])
        .unwrap();
    doc.set_instance_material(outer, Some(red)).unwrap();

    doc.explode_instance(outer).unwrap();
    let material_of = |doc: &Document| -> Vec<Option<MaterialId>> {
        let mut v: Vec<_> = doc
            .instance_ids()
            .into_iter()
            .map(|i| doc.instance_material(i).flatten())
            .collect();
        v.sort();
        v
    };
    let mut want = vec![Some(red), Some(blue)];
    want.sort();
    assert_eq!(
        material_of(&doc),
        want,
        "the unpainted nested copy takes red, the painted one keeps blue"
    );

    doc.undo().unwrap();
    assert_eq!(doc.instance_material(outer), Some(Some(red)));
    doc.redo().unwrap();
    assert_eq!(material_of(&doc), want);

    // Undo clears the bake: once the outer stops using red, nothing does.
    doc.undo().unwrap();
    doc.set_instance_material(outer, None).unwrap();
    assert!(
        doc.unused_materials().contains(&red),
        "undo must clear the baked material off the tombstoned instance copy"
    );
    assert!(!doc.unused_materials().contains(&blue));
}

#[test]
fn make_unique_keeps_the_instance_material() {
    let mut doc = Document::new();
    let (c, inst, _) = one_component(&mut doc);
    doc.place_instance(c, Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();
    let red = solid(&mut doc, "Red", 200);
    doc.set_instance_material(inst, Some(red)).unwrap();
    let (new_def, _) = doc.make_unique(inst).unwrap();
    assert_ne!(new_def, c);
    assert_eq!(doc.instance_material(inst), Some(Some(red)));
}

#[test]
fn a_material_used_only_by_an_instance_is_in_use() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    assert!(doc.unused_materials().contains(&red));
    doc.set_instance_material(inst, Some(red)).unwrap();
    assert!(!doc.unused_materials().contains(&red));
    assert_eq!(doc.material_usage(red), 1);
    let report = doc.purge_unused().unwrap();
    assert_eq!(report.materials, 0);
    assert!(doc.material(red).is_some());
}

#[test]
fn deleting_a_material_clears_instances_and_undo_restores_them() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    doc.set_instance_material(inst, Some(red)).unwrap();
    let change = doc.delete_material(red).unwrap();
    assert!(change.instances_touched.contains(&inst));
    assert_eq!(doc.instance_material(inst), Some(None));
    doc.undo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(red)));
    doc.redo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(None));
}

#[test]
fn replace_material_document_scope_switches_instances() {
    let mut doc = Document::new();
    let (_, inst, obj) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    let blue = solid(&mut doc, "Blue", 20);
    doc.set_instance_material(inst, Some(red)).unwrap();
    let change = doc
        .replace_material(MaterialScope::Document, Some(red), Some(blue))
        .unwrap();
    assert!(change.instances_touched.contains(&inst));
    assert_eq!(doc.instance_material(inst), Some(Some(blue)));
    doc.undo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(red)));
    doc.redo().unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(blue)));

    // Object scope never reaches an instance.
    doc.replace_material(MaterialScope::Object(obj), Some(blue), Some(red))
        .unwrap();
    assert_eq!(doc.instance_material(inst), Some(Some(blue)));
}

#[test]
fn library_copies_remap_the_instance_material() {
    let mut doc = Document::new();
    let _pad = solid(&mut doc, "Pad", 1);
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    doc.set_instance_material(inst, Some(red)).unwrap();

    let item = doc.extract_item(&[NodeId::Instance(inst)], false).unwrap();
    let item_inst = item.instance_ids()[0];
    let item_mat = item
        .instance_material(item_inst)
        .flatten()
        .expect("the copy keeps a material");
    assert_eq!(item.material(item_mat).unwrap().name, "Red");

    let mut dst = Document::new();
    let _other = solid(&mut dst, "Other", 50);
    dst.insert_document(
        &item,
        &InsertOptions {
            pose: Transform::IDENTITY,
            provenance: None,
            origin: None,
        },
    )
    .unwrap();
    let dst_inst = dst.instance_ids()[0];
    let dst_mat = dst.instance_material(dst_inst).flatten().unwrap();
    assert_eq!(dst.material(dst_mat).unwrap().name, "Red");
}

#[test]
fn instance_materials_round_trip_through_a_file() {
    let mut n = nest();
    let red = solid(&mut n.doc, "Red", 200);
    let blue = solid(&mut n.doc, "Blue", 20);
    n.doc.set_instance_material(n.c1, Some(blue)).unwrap();
    n.doc.set_instance_material(n.b1, Some(red)).unwrap();
    let bytes = n.doc.save();
    let loaded = Document::load(&bytes).unwrap();
    assert_eq!(loaded.state_hash(), n.doc.state_hash());

    let rows = loaded.occurrences(None, true).unwrap();
    let mat_name = |m: Option<MaterialId>| m.map(|m| loaded.material(m).unwrap().name.clone());
    let world = rows
        .iter()
        .find(|r| r.path.is_empty() && matches!(r.node, NodeId::Instance(_)))
        .unwrap();
    assert_eq!(mat_name(world.material).as_deref(), Some("Blue"));
    let leaf = rows
        .iter()
        .find(|r| matches!(r.node, NodeId::Object(_)) && r.path.len() == 3)
        .unwrap();
    assert_eq!(mat_name(leaf.material_effective).as_deref(), Some("Red"));
}

#[test]
fn a_document_without_instance_materials_writes_no_material_key() {
    let mut doc = Document::new();
    one_component(&mut doc);
    let manifest = manifest_of(&doc.save());
    for inst in manifest["instances"].as_array().unwrap() {
        assert!(inst.get("material").is_none());
    }
    assert_eq!(manifest["format_version"], 29);
}

#[test]
fn an_instance_material_smuggled_into_a_pre_v27_file_is_rejected() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    doc.set_instance_material(inst, Some(red)).unwrap();
    let bytes = doc.save();
    for declared in [22, 23, 24, 26] {
        let smuggled = patch_manifest(&bytes, |m| m["format_version"] = declared.into());
        assert!(
            matches!(
                Document::load(&smuggled),
                Err(LoadError::MalformedManifest { .. })
            ),
            "an instance material below v27 is a smuggled field (declared v{declared})"
        );
    }

    // The same file without the field still loads at v22.
    let clean = patch_manifest(&bytes, |m| {
        m["format_version"] = 22.into();
        for i in m["instances"].as_array_mut().unwrap() {
            i.as_object_mut().unwrap().remove("material");
        }
    });
    let old = Document::load(&clean).expect("a v22 file loads");
    assert_eq!(old.instance_material(old.instance_ids()[0]), Some(None));
}

#[test]
fn an_instance_material_out_of_range_is_rejected() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    doc.set_instance_material(inst, Some(red)).unwrap();
    let bad = patch_manifest(&doc.save(), |m| {
        m["instances"][0]["material"] = 99.into();
    });
    assert!(matches!(
        Document::load(&bad),
        Err(LoadError::DanglingReference { .. })
    ));
}

fn manifest_of(bytes: &[u8]) -> serde_json::Value {
    use std::io::{Cursor, Read as _};
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut s)
        .unwrap();
    serde_json::from_slice(&s).unwrap()
}

fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut manifest = manifest_of(bytes);
    patch(&mut manifest);
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    out.start_file("manifest.json", opts).unwrap();
    out.write_all(&patched).unwrap();
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..src.len() {
        let mut entry = src.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        if name == "manifest.json" {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        out.start_file(name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn state_hash_moves_only_with_an_instance_material() {
    let mut doc = Document::new();
    let (_, inst, _) = one_component(&mut doc);
    let red = solid(&mut doc, "Red", 200);
    let plain = doc.state_hash();
    doc.set_instance_material(inst, Some(red)).unwrap();
    assert_ne!(doc.state_hash(), plain);
    doc.set_instance_material(inst, None).unwrap();
    assert_eq!(
        doc.state_hash(),
        plain,
        "no instance material: the same state as before the field existed"
    );
}
