//! Executable specs for Library updates (OW-2, v1.5 design D2), the kernel
//! half: the content print, the link state, the re-insert that no longer
//! duplicates after a rename, and Update from Library.
//!
//! The contract under test:
//! - a definition's content print is what it IS: inserting an item gives a
//!   definition whose print equals the item's; renaming, re-tagging,
//!   attributes, moving or hiding a placement, re-saving and a `.hew`
//!   round trip leave it alone; changing a vertex, a face's paint or a
//!   nested part changes it; two palettes holding the same paint agree;
//! - the link state follows D2's table, every row, links and items without
//!   prints included;
//! - re-inserting an item that was only renamed reuses its definition;
//! - an update replaces a definition's contents in place: it keeps its id
//!   and every placement's pose, is one undo step whose undo restores the
//!   exact prior state hash, updates a nested part from the same item in
//!   the same step, and skips and names a refused part (locked, frozen
//!   through a copy, open for editing, unknown, would nest itself) leaving
//!   the document exactly as it was. DEVELOPMENT.md rule 3: properties at
//!   the end.

use std::collections::BTreeMap;

use kernel::{
    ComponentId, Document, DocumentError, EntityRef, GroupId, InsertOptions, InstanceId, KernelOp,
    LibraryLinkState, LibraryProvenance, LibraryUpdate, Material, NodeId, ObjectId, Plane, Point3,
    Rgba8, Transform, Vec3, library_link_state,
};
use proptest::prelude::*;

// ------------------------------------------------------------- helpers

/// A deterministic stand-in for the API layer's SHA-256 (the kernel has no
/// hash of its own): four FNV-1a lanes with different seeds.
fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for lane in 0..4u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ lane.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let at = lane as usize * 8;
        out[at..at + 8].copy_from_slice(&h.to_le_bytes());
    }
    out
}

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// A box `w` wide on the ground at `x`, `h` tall.
fn boxed(doc: &mut Document, x: f64, w: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + w, 0.0, 0.0),
        Point3::new(x + w, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    let region = doc.extrudable_regions(s).expect("sketch is live")[0];
    doc.extrude_region(s, region, h).expect("extrude").0
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

/// A one-box component: the definition and its one placement.
fn part(doc: &mut Document, h: f64) -> (ComponentId, InstanceId) {
    let o = boxed(doc, 0.0, 1.0, h);
    let (c, i, _) = doc.make_component(&[NodeId::Object(o)]).expect("component");
    (c, i)
}

/// A component item: one box `h` tall, wrapped, extracted.
fn item_of_height(h: f64) -> Document {
    let mut src = Document::new();
    let (_, i) = part(&mut src, h);
    src.extract_item(&[NodeId::Instance(i)], true)
        .expect("extract")
}

/// The item's one definition and its stable id.
fn item_def(item: &Document) -> (ComponentId, u64) {
    let c = item.component_ids()[0];
    (c, item.sid_of(&EntityRef::Component(c)).expect("sid"))
}

fn prov(id: &str, hash: &str, item: &Document) -> LibraryProvenance {
    LibraryProvenance {
        source_id: id.to_string(),
        content_hash: hash.to_string(),
        prints: item.content_prints_by_sid(digest),
    }
}

fn insert(
    doc: &mut Document,
    item: &Document,
    p: LibraryProvenance,
    at: f64,
) -> kernel::InsertReport {
    doc.insert_document(
        item,
        &InsertOptions {
            pose: shift(at),
            provenance: Some(p),
            origin: None,
            drawing: kernel::DrawingInsert::Skip,
            hidden_objects: Default::default(),
        },
    )
    .expect("insert")
    .0
}

fn print(doc: &Document, c: ComponentId) -> String {
    doc.content_print(c, digest).expect("print")
}

/// The face of `o` whose normal points up.
fn top_face(doc: &Document, o: ObjectId) -> kernel::FaceId {
    let obj = doc.object(o).expect("object");
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9)
        .map(|(id, _)| id)
        .expect("a top face")
}

/// Raise a definition's first member's top face by `d`.
fn raise(doc: &mut Document, c: ComponentId, d: f64) {
    let o = doc.def_members(c).expect("live")[0];
    let face = top_face(doc, o);
    doc.apply_def_op(c, o, KernelOp::PushPull { face, distance: d })
        .expect("push/pull");
}

fn library(entries: &[(&str, &Document)]) -> BTreeMap<String, BTreeMap<u64, String>> {
    entries
        .iter()
        .map(|(id, item)| (id.to_string(), item.content_prints_by_sid(digest)))
        .collect()
}

fn state_of(
    doc: &Document,
    lib: &BTreeMap<String, BTreeMap<u64, String>>,
    c: ComponentId,
) -> LibraryLinkState {
    doc.library_links(lib, digest)
        .into_iter()
        .find(|l| l.definition == c)
        .expect("a linked definition")
        .state
}

fn update<'a>(def: ComponentId, item: &'a Document, p: LibraryProvenance) -> LibraryUpdate<'a> {
    LibraryUpdate {
        definition: def,
        item,
        provenance: p,
    }
}

fn instance_poses(doc: &Document) -> Vec<(InstanceId, [f64; 12])> {
    doc.instance_ids()
        .into_iter()
        .map(|i| (i, doc.instance_pose(i).expect("pose").to_affine()))
        .collect()
}

// ------------------------------------------------------- the content print

#[test]
fn an_inserted_item_prints_as_the_item_does_and_is_stamped_with_it() {
    let item = item_of_height(1.0);
    let (ic, isid) = item_def(&item);
    let mut doc = Document::new();
    insert(&mut doc, &item, prov("leg", "h1", &item), 0.0);
    let c = doc.component_ids()[0];
    assert_eq!(print(&doc, c), print(&item, ic));
    assert!(print(&doc, c).starts_with("p1:"));
    let stamp = doc.library_stamp(c).expect("linked");
    assert_eq!(stamp.source_id, "leg");
    assert_eq!(stamp.def_sid, Some(isid));
    assert_eq!(
        stamp.print,
        Some(print(&item, ic)),
        "the base is the item's print"
    );
    assert_eq!(stamp.content_hash.as_deref(), Some("h1"));
}

#[test]
fn the_print_ignores_names_tags_attributes_placement_and_hiding() {
    let mut doc = Document::new();
    let (c, i) = part(&mut doc, 1.0);
    let before = print(&doc, c);
    let o = doc.def_members(c).unwrap()[0];
    doc.set_component_name(c, Some("Oak leg".into())).unwrap();
    doc.set_node_name(NodeId::Instance(i), Some("Front left".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Instance(i), vec!["Legs".into()])
        .unwrap();
    doc.attr_set(
        kernel::AttrTarget::Entity(EntityRef::Component(c)),
        "acme.parts",
        "sku",
        kernel::AttrValue::Text("L-1".into()),
    )
    .unwrap();
    doc.attr_set(
        kernel::AttrTarget::Entity(EntityRef::Object(o)),
        "acme.parts",
        "grain",
        kernel::AttrValue::Text("long".into()),
    )
    .unwrap();
    doc.transform_instance(i, &shift(5.0)).unwrap();
    doc.place_instance(c, shift(9.0)).unwrap();
    doc.set_node_user_hidden(NodeId::Instance(i), true);
    assert_eq!(print(&doc, c), before);

    // Re-saved and reopened: the same print.
    let reopened = Document::load(&doc.save()).expect("round trip");
    let rc = reopened.component_ids()[0];
    assert_eq!(print(&reopened, rc), before);
}

#[test]
fn the_print_changes_with_a_vertex_a_paint_or_a_nested_part() {
    let mut doc = Document::new();
    let (c, _) = part(&mut doc, 1.0);
    let p0 = print(&doc, c);
    raise(&mut doc, c, 0.25);
    let p1 = print(&doc, c);
    assert_ne!(p0, p1, "a moved vertex");

    let oak = doc.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
        pbr: None,
    });
    let o = doc.def_members(c).unwrap()[0];
    let face = top_face(&doc, o);
    doc.paint_face(o, face, Some(oak)).unwrap();
    let p2 = print(&doc, c);
    assert_ne!(p1, p2, "a face's paint");

    // A table placing the leg: changing the leg changes the table.
    let (leg, leg_i) = part(&mut doc, 2.0);
    let (table, _, _) = doc.make_component(&[NodeId::Instance(leg_i)]).unwrap();
    let t0 = print(&doc, table);
    raise(&mut doc, leg, 0.5);
    assert_ne!(print(&doc, table), t0, "a nested part");
}

#[test]
fn two_palettes_with_the_same_paint_print_alike() {
    let paint = |doc: &mut Document, extra_first: bool| -> ComponentId {
        if extra_first {
            doc.add_material(Material {
                name: "Unused".into(),
                color: Rgba8::rgb(1, 2, 3),
                texture: None,
                pbr: None,
            });
        }
        let m = doc.add_material(Material {
            // A different name for the same paint.
            name: if extra_first {
                "Walnut".into()
            } else {
                "Brown".into()
            },
            color: Rgba8::rgb(90, 60, 30),
            texture: None,
            pbr: None,
        });
        let (c, _) = part(doc, 1.0);
        let o = doc.def_members(c).unwrap()[0];
        doc.set_object_material(o, Some(m)).unwrap();
        c
    };
    let mut a = Document::new();
    let ca = paint(&mut a, false);
    let mut b = Document::new();
    let cb = paint(&mut b, true);
    assert_eq!(print(&a, ca), print(&b, cb));
}

// ----------------------------------------------------------- link states

#[test]
fn the_state_table_every_row() {
    use LibraryLinkState::*;
    let (a, b, c) = ("p1:aa", "p1:bb", "p1:cc");
    // mine vs base, theirs vs base
    assert_eq!(library_link_state(a, Some(a), Some(a)), InStep);
    assert_eq!(library_link_state(a, Some(a), Some(b)), LibraryNewer);
    assert_eq!(library_link_state(b, Some(a), Some(a)), EditedHere);
    assert_eq!(library_link_state(b, Some(a), Some(c)), BothChanged);
    // mine == theirs, whatever base says
    assert_eq!(library_link_state(b, Some(a), Some(b)), InStep);
    assert_eq!(library_link_state(b, None, Some(b)), InStep);
    // no base: no claim about direction
    assert_eq!(library_link_state(a, None, Some(b)), Different);
    // a base or a print under another scheme
    assert_eq!(library_link_state(a, Some("p0:aa"), Some(b)), Different);
    assert_eq!(library_link_state(a, Some(a), Some("p2:aa")), Different);
    // the item is gone
    assert_eq!(library_link_state(a, Some(a), None), ItemMissing);
    assert_eq!(library_link_state(a, None, None), ItemMissing);
}

#[test]
fn links_read_every_state_against_the_library() {
    use LibraryLinkState::*;
    let v1 = item_of_height(1.0);
    let mut v2 = v1.clone();
    let (v2c, _) = item_def(&v2);
    raise(&mut v2, v2c, 0.5);

    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("leg", "h1", &v1), 0.0);
    let c = doc.component_ids()[0];

    assert_eq!(state_of(&doc, &library(&[("leg", &v1)]), c), InStep);
    assert_eq!(state_of(&doc, &library(&[("leg", &v2)]), c), LibraryNewer);
    assert_eq!(state_of(&doc, &library(&[]), c), ItemMissing);
    // An item that no longer holds this definition.
    let other = item_of_height(3.0);
    let mut not_it = BTreeMap::new();
    not_it.insert(
        "leg".to_string(),
        BTreeMap::from([(u64::MAX - 7, print(&other, item_def(&other).0))]),
    );
    assert_eq!(state_of(&doc, &not_it, c), ItemMissing);

    raise(&mut doc, c, 0.25);
    assert_eq!(state_of(&doc, &library(&[("leg", &v1)]), c), EditedHere);
    assert_eq!(state_of(&doc, &library(&[("leg", &v2)]), c), BothChanged);

    // Undo the edit: back in step because the content is.
    doc.undo().unwrap();
    assert_eq!(state_of(&doc, &library(&[("leg", &v1)]), c), InStep);

    let links = doc.library_links(&library(&[("leg", &v2)]), digest);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].copies, 1);
    assert!(!links[0].frozen);
    assert_eq!(links[0].theirs, Some(print(&v2, v2c)));
}

#[test]
fn a_link_made_before_prints_reads_mine_against_theirs() {
    use LibraryLinkState::*;
    let v1 = item_of_height(1.0);
    let mut v2 = v1.clone();
    let (v2c, _) = item_def(&v2);
    raise(&mut v2, v2c, 0.5);
    let mut doc = Document::new();
    // Inserted the way 1.4 did: no prints, so no base.
    let old = LibraryProvenance {
        source_id: "leg".into(),
        content_hash: "h1".into(),
        ..Default::default()
    };
    insert(&mut doc, &v1, old, 0.0);
    let c = doc.component_ids()[0];
    assert_eq!(doc.library_stamp(c).unwrap().print, None);
    // An item saved before prints: the caller computes them from the file.
    assert_eq!(state_of(&doc, &library(&[("leg", &v1)]), c), InStep);
    assert_eq!(state_of(&doc, &library(&[("leg", &v2)]), c), Different);
}

// ------------------------------------------------------- save to Library

/// A model holding two different parts, A (a box 1 tall) and B (a box 2
/// tall), and both placements.
fn two_parts() -> (Document, ComponentId, InstanceId, ComponentId, InstanceId) {
    let mut doc = Document::new();
    let (a, ai) = part(&mut doc, 1.0);
    let o = boxed(&mut doc, 3.0, 1.0, 2.0);
    let (b, bi, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    (doc, a, ai, b, bi)
}

#[test]
fn saving_several_parts_links_each_to_its_own_definition_in_the_item() {
    use LibraryLinkState::*;
    let (mut doc, a, ai, b, bi) = two_parts();
    let nodes = [NodeId::Instance(ai), NodeId::Instance(bi)];
    let bytes = doc.extract_item(&nodes, true).unwrap().save();
    let item = Document::load(&bytes).unwrap();
    let sids = doc.library_save_def_sids(&nodes).unwrap();
    assert_eq!(sids.len(), 2);
    assert_ne!(sids[&a], sids[&b]);
    // Each id names the item's copy of that definition.
    for (&def, &sid) in &sids {
        let copy = item
            .component_ids()
            .into_iter()
            .find(|&c| item.sid_of(&EntityRef::Component(c)) == Some(sid));
        assert_eq!(
            print(&item, copy.expect("the item holds it")),
            print(&doc, def)
        );
    }
    doc.stamp_library_save(&nodes, &prov("pair", "h", &item), &sids);
    for def in [a, b] {
        let stamp = doc.library_stamp(def).unwrap();
        assert_eq!(stamp.def_sid, Some(sids[&def]));
        assert_eq!(
            stamp.print,
            Some(print(&doc, def)),
            "the base is its own print"
        );
    }
    let lib = library(&[("pair", &item)]);
    assert_eq!(state_of(&doc, &lib, a), InStep);
    assert_eq!(state_of(&doc, &lib, b), InStep);

    // The Library changes B: only B reads newer, and an update takes it.
    let mut v2 = item.clone();
    let b_copy = v2
        .component_ids()
        .into_iter()
        .find(|&c| v2.sid_of(&EntityRef::Component(c)) == Some(sids[&b]))
        .unwrap();
    raise(&mut v2, b_copy, 0.5);
    let v2 = Document::load(&v2.save()).unwrap();
    let lib2 = library(&[("pair", &v2)]);
    assert_eq!(state_of(&doc, &lib2, a), InStep);
    assert_eq!(state_of(&doc, &lib2, b), LibraryNewer);
    let (r, _) = doc
        .update_definitions_from_items(&[update(b, &v2, prov("pair", "h2", &v2))], digest)
        .unwrap();
    assert_eq!(r.updated, vec![b]);
    assert_eq!(state_of(&doc, &lib2, b), InStep);

    // Re-inserting the item reuses both definitions.
    let mut fresh = Document::load(&doc.save()).unwrap();
    let before = fresh.component_ids().len();
    let again = insert(&mut fresh, &v2, prov("pair", "h2", &v2), 9.0);
    assert_eq!(again.definitions_reused, 2);
    assert_eq!(fresh.component_ids().len(), before);
}

#[test]
fn the_old_single_id_stamp_misreads_a_second_part() {
    // What a save stamped before each definition got its own id: every
    // definition took the item's first one, so B read "changed here"
    // right after the save. Kept as the regression this guards.
    use LibraryLinkState::*;
    let (mut doc, _a, ai, b, bi) = two_parts();
    let nodes = [NodeId::Instance(ai), NodeId::Instance(bi)];
    let bytes = doc.extract_item(&nodes, true).unwrap().save();
    let item = Document::load(&bytes).unwrap();
    let first: u64 = kernel::read_item_summary(&bytes)
        .unwrap()
        .first_component_sid
        .unwrap()
        .parse()
        .unwrap();
    doc.stamp_library_source(&nodes, &prov("pair", "h", &item), Some(first));
    let lib = library(&[("pair", &item)]);
    assert_eq!(state_of(&doc, &lib, b), EditedHere);
    // The save path's stamp puts it right.
    let sids = doc.library_save_def_sids(&nodes).unwrap();
    doc.stamp_library_save(&nodes, &prov("pair", "h", &item), &sids);
    assert_eq!(state_of(&doc, &lib, b), InStep);
}

#[test]
fn saving_a_part_that_nests_another_links_the_outer_one_to_its_own_id() {
    let mut doc = Document::new();
    let (leg, leg_i) = part(&mut doc, 1.0);
    let (table, table_i, _) = doc.make_component(&[NodeId::Instance(leg_i)]).unwrap();
    let nodes = [NodeId::Instance(table_i)];
    let item = Document::load(&doc.extract_item(&nodes, true).unwrap().save()).unwrap();
    let sids = doc.library_save_def_sids(&nodes).unwrap();
    assert_eq!(sids.keys().copied().collect::<Vec<_>>(), vec![table]);
    doc.stamp_library_save(&nodes, &prov("table", "t", &item), &sids);
    let lib = library(&[("table", &item)]);
    assert_eq!(state_of(&doc, &lib, table), LibraryLinkState::InStep);
    assert!(
        doc.library_stamp(leg).is_none(),
        "only the saved selection links"
    );
}

// ------------------------------------------------- re-insert after rename

#[test]
fn reinserting_a_renamed_item_reuses_its_definition() {
    let item = item_of_height(1.0);
    let mut doc = Document::new();
    insert(&mut doc, &item, prov("leg", "h1", &item), 0.0);

    // Renamed in the Library: new metadata, so a new file hash; the same
    // content.
    let mut renamed = item.clone();
    renamed
        .attr_set(
            kernel::AttrTarget::Document,
            "hew.library",
            "name",
            kernel::AttrValue::Text("Oak leg (renamed)".into()),
        )
        .unwrap();
    let renamed = Document::load(&renamed.save()).unwrap();
    let second = insert(&mut doc, &renamed, prov("leg", "h2-renamed", &renamed), 3.0);
    assert_eq!(second.definitions_reused, 1);
    assert_eq!(second.definitions_added, 0);
    assert_eq!(doc.component_ids().len(), 1, "no duplicate definition");
    assert_eq!(doc.instance_ids().len(), 2);
}

#[test]
fn reinserting_after_an_edit_here_copies_the_item_fresh() {
    let item = item_of_height(1.0);
    let mut doc = Document::new();
    insert(&mut doc, &item, prov("leg", "h1", &item), 0.0);
    let c = doc.component_ids()[0];
    raise(&mut doc, c, 0.5);
    let second = insert(&mut doc, &item, prov("leg", "h1", &item), 3.0);
    assert_eq!(
        second.definitions_reused, 0,
        "the edited copy is not the item"
    );
    assert_eq!(doc.component_ids().len(), 2);
}

// ------------------------------------------------------------- the update

/// A model with the v1 leg inserted twice (one definition, two copies),
/// plus the v2 item.
fn two_legs() -> (Document, ComponentId, Document, Document) {
    let v1 = item_of_height(1.0);
    let mut v2 = v1.clone();
    let (v2c, _) = item_def(&v2);
    raise(&mut v2, v2c, 0.5);
    let v2 = Document::load(&v2.save()).unwrap();
    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("leg", "h1", &v1), 0.0);
    insert(&mut doc, &v1, prov("leg", "h1", &v1), 4.0);
    let c = doc.component_ids()[0];
    assert_eq!(doc.instance_ids().len(), 2);
    (doc, c, v1, v2)
}

#[test]
fn an_update_replaces_the_contents_in_place_and_nothing_moves() {
    let (mut doc, c, _v1, v2) = two_legs();
    let poses = instance_poses(&doc);
    let depth = doc.undo_depth();
    let (report, change) = doc
        .update_definitions_from_items(&[update(c, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    assert_eq!(report.updated, vec![c]);
    assert!(report.skipped.is_empty() && report.unchanged.is_empty());
    assert_eq!(
        doc.component_ids(),
        vec![c],
        "the definition keeps its identity"
    );
    assert_eq!(
        instance_poses(&doc),
        poses,
        "every copy stays where it stands"
    );
    assert_eq!(print(&doc, c), print(&v2, item_def(&v2).0));
    let stamp = doc.library_stamp(c).unwrap();
    assert_eq!(
        stamp.print,
        Some(print(&v2, item_def(&v2).0)),
        "re-stamped: base = the item's print"
    );
    assert_eq!(stamp.content_hash.as_deref(), Some("h2"));
    assert_eq!(
        state_of(&doc, &library(&[("leg", &v2)]), c),
        LibraryLinkState::InStep
    );
    assert_eq!(doc.undo_depth(), depth + 1, "one undo step");
    let entries = doc.history_entries();
    assert_eq!(entries.undo.last().unwrap().label, "Update from Library");
    assert_eq!(entries.undo.last().unwrap().kind.id, "update-from-library");
    assert!(change.components_touched.contains(&c));
    for (i, _) in &poses {
        assert!(change.instances_touched.contains(i), "every copy redraws");
    }
}

#[test]
fn undo_restores_the_exact_prior_state_and_redo_the_update() {
    let (mut doc, c, _v1, v2) = two_legs();
    let before = doc.state_hash();
    doc.update_definitions_from_items(&[update(c, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    let after = doc.state_hash();
    assert_ne!(before, after);
    doc.undo().unwrap();
    assert_eq!(
        doc.state_hash(),
        before,
        "undo restores the exact prior state"
    );
    assert_eq!(
        doc.library_stamp(c).unwrap().content_hash.as_deref(),
        Some("h1")
    );
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    // The undone state saves and reopens.
    doc.undo().unwrap();
    let reopened = Document::load(&doc.save()).expect("round trip");
    assert_eq!(reopened.state_hash(), before);
}

#[test]
fn an_update_that_adds_paint_leaves_only_that_palette_entry_after_undo() {
    let (mut doc, c, _v1, mut v2) = two_legs();
    let (v2c, _) = item_def(&v2);
    let red = v2.add_material(Material {
        name: "Red".into(),
        color: Rgba8::rgb(200, 10, 10),
        texture: None,
        pbr: None,
    });
    let o = v2.def_members(v2c).unwrap()[0];
    v2.set_object_material(o, Some(red)).unwrap();
    let before_palette = doc.material_ids().len();
    let before_members = doc.def_member_nodes(c).unwrap();
    let (report, _) = doc
        .update_definitions_from_items(&[update(c, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    assert_eq!(report.materials_added, 1);
    doc.undo().unwrap();
    assert_eq!(doc.def_member_nodes(c).unwrap(), before_members);
    // Like every palette addition, the paint stays in the palette.
    assert_eq!(doc.material_ids().len(), before_palette + 1);
}

#[test]
fn an_update_that_changes_nothing_records_nothing() {
    let (mut doc, c, v1, _v2) = two_legs();
    let depth = doc.undo_depth();
    let hash = doc.state_hash();
    let (report, change) = doc
        .update_definitions_from_items(&[update(c, &v1, prov("leg", "h1-again", &v1))], digest)
        .unwrap();
    assert_eq!(report.unchanged, vec![c]);
    assert!(report.updated.is_empty());
    assert_eq!(change, kernel::DocChange::default());
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.state_hash(), hash);
}

/// An item "Table" whose definition places its own "Leg" definition, and
/// the same item with a taller leg.
fn table_items() -> (Document, Document) {
    let mut src = Document::new();
    let (_, leg_i) = part(&mut src, 1.0);
    let (_, table_i, _) = src.make_component(&[NodeId::Instance(leg_i)]).unwrap();
    let v1 = src
        .extract_item(&[NodeId::Instance(table_i)], true)
        .unwrap();
    let v1 = Document::load(&v1.save()).unwrap();
    let mut v2 = v1.clone();
    // The leg is the definition the table places.
    let table_def = v2
        .component_ids()
        .into_iter()
        .find(|&c| {
            v2.def_member_nodes(c)
                .unwrap()
                .iter()
                .any(|n| matches!(n, NodeId::Instance(_)))
        })
        .unwrap();
    let leg_def = v2
        .component_ids()
        .into_iter()
        .find(|&c| c != table_def)
        .unwrap();
    raise(&mut v2, leg_def, 0.5);
    let v2 = Document::load(&v2.save()).unwrap();
    (v1, v2)
}

/// The table and leg definitions of a model holding one inserted table.
fn table_and_leg(doc: &Document) -> (ComponentId, ComponentId) {
    let table = doc
        .component_ids()
        .into_iter()
        .find(|&c| {
            doc.def_member_nodes(c)
                .unwrap()
                .iter()
                .any(|n| matches!(n, NodeId::Instance(_)))
        })
        .unwrap();
    let leg = doc
        .component_ids()
        .into_iter()
        .find(|&c| c != table)
        .unwrap();
    (table, leg)
}

/// An update whose new contents no longer place a linked set the old ones
/// did: the set's one remaining copy is made plain in the same step, by
/// the rule a delete follows, and undo restores the set exactly.
#[test]
fn an_update_that_drops_a_linked_copy_makes_the_last_copy_plain() {
    // v1: a table holding one copy of a linked set (a board).
    let mut src = Document::new();
    let o = boxed(&mut src, 0.0, 1.0, 1.0);
    let (_, copy, _) = src.wrap_linked(NodeId::Object(o)).unwrap();
    let (_, table_i, _) = src.make_component(&[NodeId::Instance(copy)]).unwrap();
    let v1 = src
        .extract_item(&[NodeId::Instance(table_i)], true)
        .unwrap();
    let v1 = Document::load(&v1.save()).unwrap();
    // v2: the same table with the copy exploded into a plain board.
    let mut v2 = v1.clone();
    let placed = v2
        .instance_ids()
        .into_iter()
        .find(|&i| v2.owner_definition(NodeId::Instance(i)).is_none())
        .expect("the item's placement");
    v2.open_explode_session(placed).unwrap();
    let inner = v2
        .instance_ids()
        .into_iter()
        .find(|&i| i != placed && v2.instance_def(i).is_some_and(|d| v2.is_linked_set(d)))
        .expect("the copy, surfaced");
    v2.explode_instance(inner).unwrap();
    v2.close_explode_session().unwrap();
    let v2 = Document::load(&v2.save()).unwrap();

    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("table", "t1", &v1), 0.0);
    let set = doc
        .component_ids()
        .into_iter()
        .find(|&c| doc.is_linked_set(c))
        .expect("the linked set");
    let table = doc
        .component_ids()
        .into_iter()
        .find(|&c| c != set)
        .expect("the table");
    // The set's other copy stands alone in the model.
    let (lone, _) = doc.place_instance(set, shift(8.0)).unwrap();
    assert_eq!(doc.definition_usage(set), 2);
    let before = doc.state_hash();
    let depth = doc.undo_depth();

    let (report, change) = doc
        .update_definitions_from_items(&[update(table, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert_eq!(report.updated, vec![table]);
    assert_eq!(doc.undo_depth(), depth + 1, "one step");
    assert_eq!(
        doc.history_entries().undo.last().unwrap().label,
        "Update from Library"
    );
    assert!(doc.instance_def(lone).is_none(), "the last copy is plain");
    assert!(!doc.is_linked_set(set), "and no set of one is left");
    assert!(change.instances_touched.contains(&lone));
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert_eq!(doc.definition_usage(set), 2);
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
}

#[test]
fn a_nested_part_from_the_same_item_updates_in_the_same_step() {
    let (v1, v2) = table_items();
    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("table", "t1", &v1), 0.0);
    let (table, leg) = table_and_leg(&doc);
    // The leg also stands alone in the model.
    let lone = doc.place_instance(leg, shift(8.0)).unwrap().0;
    let lone_pose = doc.instance_pose(lone).unwrap().to_affine();
    let before = doc.state_hash();
    let depth = doc.undo_depth();

    let (report, _) = doc
        .update_definitions_from_items(&[update(table, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert_eq!(report.updated, vec![table]);
    assert_eq!(report.also_updated, vec![leg]);
    assert_eq!(
        report.definitions_added, 0,
        "the leg the model holds is reused"
    );
    assert_eq!(doc.undo_depth(), depth + 1, "one step for both");
    assert_eq!(doc.component_ids().len(), 2);
    let (v2_table, v2_leg) = table_and_leg(&v2);
    assert_eq!(print(&doc, table), print(&v2, v2_table));
    assert_eq!(print(&doc, leg), print(&v2, v2_leg));
    assert_eq!(doc.instance_pose(lone).unwrap().to_affine(), lone_pose);
    let lib = library(&[("table", &v2)]);
    assert_eq!(state_of(&doc, &lib, table), LibraryLinkState::InStep);
    assert_eq!(state_of(&doc, &lib, leg), LibraryLinkState::InStep);

    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn a_locked_part_is_skipped_and_named_and_the_rest_go_through() {
    let (mut doc, leg, _v1, v2) = two_legs();
    let shelf_v1 = item_of_height(0.2);
    let mut shelf_v2 = shelf_v1.clone();
    let (s2c, _) = item_def(&shelf_v2);
    raise(&mut shelf_v2, s2c, 0.1);
    insert(&mut doc, &shelf_v1, prov("shelf", "s1", &shelf_v1), 10.0);
    let shelf = doc.component_ids().into_iter().find(|&c| c != leg).unwrap();
    // Lock the leg through one of its copies: the whole shape is locked.
    let a_leg = doc
        .instance_ids()
        .into_iter()
        .find(|&i| doc.instance_def(i) == Some(leg))
        .unwrap();
    doc.set_nodes_locked(&[NodeId::Instance(a_leg)], true)
        .unwrap();
    let leg_print = print(&doc, leg);
    let depth = doc.undo_depth();

    let (report, _) = doc
        .update_definitions_from_items(
            &[
                update(leg, &v2, prov("leg", "h2", &v2)),
                update(shelf, &shelf_v2, prov("shelf", "s2", &shelf_v2)),
            ],
            digest,
        )
        .unwrap();
    assert_eq!(report.updated, vec![shelf]);
    assert_eq!(report.skipped.len(), 1);
    let (skipped, why) = report.skipped[0];
    assert_eq!(skipped, leg);
    assert!(matches!(why, DocumentError::LockedNode { .. }), "{why:?}");
    assert_eq!(print(&doc, leg), leg_print, "the locked part is untouched");
    assert_eq!(doc.undo_depth(), depth + 1, "still one step");
}

#[test]
fn a_part_with_a_frozen_copy_is_skipped() {
    let (mut doc, leg, _v1, v2) = two_legs();
    // One copy goes into a group that is then locked: the shape is frozen
    // through that copy.
    let a_leg = doc.instance_ids()[0];
    let (g, _): (GroupId, _) = doc.group_nodes(&[NodeId::Instance(a_leg)]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    assert!(doc.def_frozen(leg).is_some());
    let hash = doc.state_hash();
    let (report, _) = doc
        .update_definitions_from_items(&[update(leg, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    assert!(report.updated.is_empty());
    assert!(matches!(
        report.skipped[..],
        [(d, DocumentError::LockedNode { via_shared_shape: true, .. })] if d == leg
    ));
    assert_eq!(doc.state_hash(), hash);
    let links = doc.library_links(&library(&[("leg", &v2)]), digest);
    assert!(links[0].frozen, "the status says the part is locked");
}

#[test]
fn refused_parts_leave_the_document_untouched() {
    let (mut doc, leg, _v1, v2) = two_legs();
    let hash = doc.state_hash();
    let depth = doc.undo_depth();
    let check = |doc: &Document, report: &kernel::LibraryUpdateReport, want: DocumentError| {
        assert!(report.updated.is_empty());
        assert_eq!(report.skipped.len(), 1, "{report:?}");
        assert_eq!(report.skipped[0].1, want);
        assert_eq!(doc.state_hash(), hash);
        assert_eq!(doc.undo_depth(), depth);
    };

    // Linked to another item than the one given.
    let (r, _) = doc
        .update_definitions_from_items(&[update(leg, &v2, prov("not-leg", "x", &v2))], digest)
        .unwrap();
    check(&doc, &r, DocumentError::UnknownComponent);

    // An item that does not hold the definition.
    let empty = Document::new();
    let (r, _) = doc
        .update_definitions_from_items(&[update(leg, &empty, prov("leg", "x", &empty))], digest)
        .unwrap();
    check(&doc, &r, DocumentError::UnknownComponent);

    // A definition with no Library link.
    let (plain, _) = part(&mut doc, 1.0);
    let hash2 = doc.state_hash();
    let (r, _) = doc
        .update_definitions_from_items(&[update(plain, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    assert_eq!(r.skipped, vec![(plain, DocumentError::UnknownComponent)]);
    assert_eq!(doc.state_hash(), hash2);
}

#[test]
fn an_update_while_a_component_is_open_is_refused() {
    let (mut doc, leg, _v1, v2) = two_legs();
    let a_leg = doc.instance_ids()[0];
    doc.open_explode_session(a_leg).unwrap();
    let hash = doc.state_hash();
    let (r, _) = doc
        .update_definitions_from_items(&[update(leg, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    assert_eq!(r.skipped, vec![(leg, DocumentError::ExplodeSessionScope)]);
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn a_part_open_for_editing_is_left_out_of_the_links_until_it_closes() {
    let (mut doc, leg, v1, _v2) = two_legs();
    let lib = library(&[("leg", &v1)]);
    doc.open_explode_session(doc.instance_ids()[0]).unwrap();
    assert!(doc.library_links(&lib, digest).is_empty());
    doc.close_innermost_session().unwrap();
    assert_eq!(state_of(&doc, &lib, leg), LibraryLinkState::InStep);
}

#[test]
fn an_open_part_reads_as_if_closed() {
    let (mut doc, leg, v1, _v2) = two_legs();
    let lib = library(&[("leg", &v1)]);
    doc.open_explode_session(doc.instance_ids()[0]).unwrap();
    let hash = doc.state_hash();
    let links = doc.library_links_as_if_closed(&lib, digest);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].definition, leg);
    assert_eq!(links[0].state, LibraryLinkState::InStep);
    assert_eq!(links[0].copies, 2);
    assert_eq!(doc.state_hash(), hash, "reading changes nothing");
}

#[test]
fn an_open_part_keeps_its_placement_count() {
    let (mut doc, leg, _v1, _v2) = two_legs();
    assert_eq!(doc.definition_usage(leg), 2);
    doc.open_explode_session(doc.instance_ids()[0]).unwrap();
    assert_eq!(doc.definition_usage(leg), 2, "uses are model-wide");
    doc.close_innermost_session().unwrap();
    assert_eq!(doc.definition_usage(leg), 2);
}

#[test]
fn an_open_part_still_previews_as_it_would_once_closed() {
    let (mut doc, leg, _v1, _v2) = two_legs();
    doc.open_explode_session(doc.instance_ids()[0]).unwrap();
    let o = doc.explode_session_objects().expect("open")[0];
    let face = top_face(&doc, o);
    doc.apply_object_op(
        o,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .unwrap();
    assert!(
        doc.definition_preview(leg).is_err(),
        "the live preview refuses"
    );
    let closed = doc.session_closed();
    assert!(
        closed.definition_preview(leg).is_ok(),
        "the closed view previews"
    );
}

#[test]
fn an_edit_inside_an_open_part_reads_changed_here_before_it_closes() {
    let (mut doc, leg, v1, _v2) = two_legs();
    let lib = library(&[("leg", &v1)]);
    doc.open_explode_session(doc.instance_ids()[0]).unwrap();
    let o = doc.explode_session_objects().expect("open")[0];
    let face = top_face(&doc, o);
    doc.apply_object_op(
        o,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .unwrap();
    let open = doc.library_links_as_if_closed(&lib, digest);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].state, LibraryLinkState::EditedHere);
    doc.close_innermost_session().unwrap();
    assert_eq!(state_of(&doc, &lib, leg), LibraryLinkState::EditedHere);
    assert_eq!(doc.library_links(&lib, digest), open);
}

#[test]
fn a_part_placing_an_open_part_reads_as_if_closed() {
    let (mut doc, leg, v1, _v2) = two_legs();
    let second = doc.instance_ids()[1];
    let (table, table_i, _) = doc.make_component(&[NodeId::Instance(second)]).unwrap();
    let t_print = print(&doc, table);
    let base = LibraryProvenance {
        source_id: "table".into(),
        content_hash: "t1".into(),
        prints: BTreeMap::from([(7u64, t_print.clone())]),
    };
    doc.stamp_library_source(&[NodeId::Instance(table_i)], &base, Some(7));
    let mut lib = library(&[("leg", &v1)]);
    lib.insert("table".into(), BTreeMap::from([(7u64, t_print)]));
    let leg_copy = doc.instance_ids()[0];
    doc.open_explode_session(leg_copy).unwrap();
    assert!(doc.library_links(&lib, digest).is_empty());
    let defs: Vec<ComponentId> = doc
        .library_links_as_if_closed(&lib, digest)
        .iter()
        .map(|l| l.definition)
        .collect();
    assert!(defs.contains(&leg) && defs.contains(&table), "{defs:?}");
}

#[test]
fn with_no_session_the_two_agree() {
    let (doc, _leg, v1, _v2) = two_legs();
    let lib = library(&[("leg", &v1)]);
    assert_eq!(
        doc.library_links_as_if_closed(&lib, digest),
        doc.library_links(&lib, digest)
    );
}

#[test]
fn an_update_that_would_nest_a_part_in_itself_is_refused() {
    // The item: K places J, J places a box (J3).
    let mut src = Document::new();
    let (_, j3_i) = part(&mut src, 1.0);
    let (_, j_i, _) = src.make_component(&[NodeId::Instance(j3_i)]).unwrap();
    let (k, _, _) = src.make_component(&[NodeId::Instance(j_i)]).unwrap();
    let item = Document::load(&src.save()).unwrap();
    let sid = |c: ComponentId| src.sid_of(&EntityRef::Component(c)).unwrap();
    let j = src.instance_def(j_i).unwrap();
    let (k_sid, j_sid) = (sid(k), sid(j));

    // The model: D holds a box (what J3 holds) and E places D (what J
    // holds), stamped as the item's K and J. Updating D places J's match,
    // E, which places D.
    let mut doc = Document::new();
    let (d, d_i) = part(&mut doc, 1.0);
    let (e, e_i, _) = doc.make_component(&[NodeId::Instance(d_i)]).unwrap();
    let base = LibraryProvenance {
        source_id: "table".into(),
        content_hash: "t1".into(),
        ..Default::default()
    };
    let inner = doc.def_member_nodes(e).unwrap()[0];
    doc.stamp_library_source(&[inner], &base, Some(k_sid));
    doc.stamp_library_source(&[NodeId::Instance(e_i)], &base, Some(j_sid));
    assert_eq!(doc.library_stamp(d).unwrap().def_sid, Some(k_sid));
    let hash = doc.state_hash();
    let depth = doc.undo_depth();
    let (r, _) = doc
        .update_definitions_from_items(&[update(d, &item, prov("table", "t2", &item))], digest)
        .unwrap();
    assert_eq!(r.skipped, vec![(d, DocumentError::ComponentCycle)]);
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.undo_depth(), depth);
}

#[test]
fn a_batch_names_a_part_twice_once() {
    let (mut doc, leg, _v1, v2) = two_legs();
    let (r, _) = doc
        .update_definitions_from_items(
            &[
                update(leg, &v2, prov("leg", "h2", &v2)),
                update(leg, &v2, prov("leg", "h2", &v2)),
            ],
            digest,
        )
        .unwrap();
    assert_eq!(r.updated, vec![leg]);
    assert_eq!(r.unchanged, vec![leg]);
}

#[test]
fn a_nested_part_changed_here_is_kept_and_the_update_copies_the_items_fresh() {
    let (v1, v2) = table_items();
    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("table", "t1", &v1), 0.0);
    let (table, leg) = table_and_leg(&doc);
    // The leg is changed here; the Library changed it too.
    raise(&mut doc, leg, 0.25);
    let lib = library(&[("table", &v2)]);
    assert_eq!(state_of(&doc, &lib, leg), LibraryLinkState::BothChanged);
    let leg_print = print(&doc, leg);
    let before = doc.state_hash();
    let depth = doc.undo_depth();

    let (r, _) = doc
        .update_definitions_from_items(&[update(table, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert_eq!(r.updated, vec![table]);
    assert!(r.also_updated.is_empty(), "the edited leg is not replaced");
    assert_eq!(r.kept, vec![leg]);
    assert_eq!(r.definitions_added, 1, "the item's leg is copied in fresh");
    assert_eq!(print(&doc, leg), leg_print, "the edit here survives");
    let (v2_table, _) = table_and_leg(&v2);
    assert_eq!(print(&doc, table), print(&v2, v2_table));
    assert_eq!(doc.undo_depth(), depth + 1);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn a_nested_part_linked_before_prints_that_differs_is_kept() {
    // No base: nothing says which side moved, so it is not overwritten.
    let (v1, v2) = table_items();
    let mut doc = Document::new();
    let old = LibraryProvenance {
        source_id: "table".into(),
        content_hash: "t1".into(),
        ..Default::default()
    };
    insert(&mut doc, &v1, old, 0.0);
    let (table, leg) = table_and_leg(&doc);
    let (r, _) = doc
        .update_definitions_from_items(&[update(table, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert_eq!(r.updated, vec![table]);
    assert_eq!(r.kept, vec![leg]);
    assert!(r.also_updated.is_empty());
}

/// Two tables from the same item, each with its own leg definition from
/// the same item definition, all in step: `(table1, leg1, table2, leg2)`,
/// the first pair made first.
fn two_tables(v1: &Document) -> (Document, ComponentId, ComponentId, ComponentId, ComponentId) {
    let mut doc = Document::new();
    insert(&mut doc, v1, prov("table", "t1", v1), 0.0);
    let (t1, l1) = table_and_leg(&doc);
    // An edit here makes the second insert copy fresh; the edit is then
    // walked back so both pairs hold the item again.
    raise(&mut doc, l1, 0.5);
    insert(&mut doc, v1, prov("table", "t1", v1), 6.0);
    raise(&mut doc, l1, -0.5);
    // The second insert's table: the definition other than the first
    // table that places a leg.
    let t2 = doc
        .component_ids()
        .into_iter()
        .find(|&c| {
            c != t1
                && doc
                    .def_member_nodes(c)
                    .unwrap()
                    .iter()
                    .any(|n| matches!(n, NodeId::Instance(_)))
        })
        .expect("a second table");
    let l2 = doc
        .component_ids()
        .into_iter()
        .find(|&c| c != l1 && doc.def_reaches(t2, c))
        .expect("its own leg");
    let lib = library(&[("table", v1)]);
    for c in [t1, l1, t2, l2] {
        assert_eq!(state_of(&doc, &lib, c), LibraryLinkState::InStep);
    }
    (doc, t1, l1, t2, l2)
}

#[test]
fn the_nested_part_replaced_is_the_one_the_old_contents_place() {
    let (v1, v2) = table_items();
    let (mut doc, _t1, l1, t2, l2) = two_tables(&v1);
    let l1_print = print(&doc, l1);
    let (r, _) = doc
        .update_definitions_from_items(&[update(t2, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert_eq!(r.updated, vec![t2]);
    assert_eq!(r.also_updated, vec![l2], "the leg this table places");
    assert_eq!(
        print(&doc, l1),
        l1_print,
        "the other table's leg is untouched"
    );
    let (_, v2_leg) = table_and_leg(&v2);
    assert_eq!(print(&doc, l2), print(&v2, v2_leg));
}

#[test]
fn a_nested_part_a_lock_holds_elsewhere_is_kept_and_the_rest_updates() {
    let (v1, v2) = table_items();
    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("table", "t1", &v1), 0.0);
    let (table, leg) = table_and_leg(&doc);
    // A copy of the leg inside a locked group freezes the leg's shape,
    // but nothing in the table is locked.
    let lone = doc.place_instance(leg, shift(8.0)).unwrap().0;
    let (g, _): (GroupId, _) = doc.group_nodes(&[NodeId::Instance(lone)]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    assert!(doc.def_frozen(leg).is_some());
    let lib = library(&[("table", &v2)]);
    let links = doc.library_links(&lib, digest);
    let row = |c| links.iter().find(|l| l.definition == c).unwrap();
    assert!(!row(table).frozen, "the status offers the table");
    assert!(row(leg).frozen);
    let leg_print = print(&doc, leg);

    let (r, _) = doc
        .update_definitions_from_items(&[update(table, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert_eq!(r.updated, vec![table], "{r:?}");
    assert_eq!(r.kept, vec![leg]);
    assert_eq!(print(&doc, leg), leg_print, "the locked leg is untouched");
}

#[test]
fn the_status_says_locked_when_the_contents_hold_a_lock() {
    let (v1, v2) = table_items();
    let mut doc = Document::new();
    insert(&mut doc, &v1, prov("table", "t1", &v1), 0.0);
    let (table, leg) = table_and_leg(&doc);
    // Locking a lone copy of the leg locks the leg, which the table's
    // contents place: the table's update would retire a locked node.
    let lone = doc.place_instance(leg, shift(8.0)).unwrap().0;
    doc.set_nodes_locked(&[NodeId::Instance(lone)], true)
        .unwrap();
    assert!(
        doc.def_frozen(table).is_none(),
        "the table itself is not frozen"
    );
    let lib = library(&[("table", &v2)]);
    let link = doc
        .library_links(&lib, digest)
        .into_iter()
        .find(|l| l.definition == table)
        .unwrap();
    assert_eq!(link.state, LibraryLinkState::LibraryNewer);
    assert!(link.frozen, "the status predicts the refusal");
    assert!(doc.library_update_lock(table).is_some());
    let hash = doc.state_hash();
    let (r, _) = doc
        .update_definitions_from_items(&[update(table, &v2, prov("table", "t2", &v2))], digest)
        .unwrap();
    assert!(r.updated.is_empty());
    assert!(matches!(
        r.skipped[..],
        [(d, DocumentError::LockedNode { .. })] if d == table
    ));
    assert_eq!(Some(r.skipped[0].1), doc.library_update_lock(table));
    assert_eq!(doc.state_hash(), hash);
}

/// The v1 leg inserted, then edited here into exactly what v2 holds: in
/// step by content, with the v1 base still stamped.
fn leg_edited_into_v2() -> (Document, ComponentId, Document, Document) {
    let (mut doc, c, v1, v2) = two_legs();
    raise(&mut doc, c, 0.5);
    assert_eq!(print(&doc, c), print(&v2, item_def(&v2).0));
    assert_eq!(
        doc.library_stamp(c).unwrap().print,
        Some(print(&v1, item_def(&v1).0))
    );
    (doc, c, v1, v2)
}

#[test]
fn an_update_that_would_only_refresh_a_base_records_nothing() {
    let (mut doc, c, v1, v2) = leg_edited_into_v2();
    let lib = library(&[("leg", &v2)]);
    assert_eq!(state_of(&doc, &lib, c), LibraryLinkState::InStep);
    let depth = doc.undo_depth();
    let hash = doc.state_hash();
    let (r, change) = doc
        .update_definitions_from_items(&[update(c, &v2, prov("leg", "h2", &v2))], digest)
        .unwrap();
    assert_eq!(r.unchanged, vec![c]);
    assert_eq!(change, kernel::DocChange::default());
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.state_hash(), hash);
    // The base stays old, and the link still reads in step: its content
    // is the item's.
    assert_eq!(
        doc.library_stamp(c).unwrap().print,
        Some(print(&v1, item_def(&v1).0))
    );
    assert_eq!(state_of(&doc, &lib, c), LibraryLinkState::InStep);
}

#[test]
fn a_step_that_updates_something_also_refreshes_an_old_base() {
    use LibraryLinkState::*;
    let (mut doc, leg, _v1, v2) = leg_edited_into_v2();
    let shelf_v1 = item_of_height(0.2);
    let mut shelf_v2 = shelf_v1.clone();
    let (s2c, _) = item_def(&shelf_v2);
    raise(&mut shelf_v2, s2c, 0.1);
    insert(&mut doc, &shelf_v1, prov("shelf", "s1", &shelf_v1), 10.0);
    let shelf = doc.component_ids().into_iter().find(|&c| c != leg).unwrap();
    let before = doc.state_hash();
    let leg_members = doc.def_member_nodes(leg).unwrap();

    let (r, change) = doc
        .update_definitions_from_items(
            &[
                update(leg, &v2, prov("leg", "h2", &v2)),
                update(shelf, &shelf_v2, prov("shelf", "s2", &shelf_v2)),
            ],
            digest,
        )
        .unwrap();
    assert_eq!(r.unchanged, vec![leg]);
    assert_eq!(r.updated, vec![shelf]);
    assert_eq!(
        doc.def_member_nodes(leg).unwrap(),
        leg_members,
        "no content change"
    );
    assert_eq!(
        doc.library_stamp(leg).unwrap().print,
        Some(print(&v2, item_def(&v2).0))
    );
    assert!(change.components_touched.contains(&leg));

    // A later change in the Library now reads as newer, not as changed on
    // both sides.
    let mut v3 = v2.clone();
    let (v3c, _) = item_def(&v3);
    raise(&mut v3, v3c, 0.25);
    assert_eq!(state_of(&doc, &library(&[("leg", &v3)]), leg), LibraryNewer);

    // One step; its undo puts the old base back exactly.
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert_eq!(state_of(&doc, &library(&[("leg", &v3)]), leg), BothChanged);
}

// ------------------------------------------------------- blends and paint

fn plate(doc: &mut Document, color: Rgba8, name: &str) -> ComponentId {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 0.1), (0.0, 0.1)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    let o = doc.extrude_region(s, r, 0.02).unwrap().0;
    let paint = doc.add_material(Material {
        name: name.into(),
        color,
        texture: None,
        pbr: None,
    });
    let obj = doc.object(o).unwrap();
    let end = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().y < -0.9)
        .map(|(id, _)| id)
        .unwrap();
    doc.paint_face(o, end, Some(paint)).unwrap();
    let edge = |a: Point3, b: Point3| {
        let obj = doc.object(o).unwrap();
        obj.edges()
            .keys()
            .find(|&e| {
                let (p, q) = obj.edge_endpoints(e).unwrap();
                (p.approx_eq(a, 1e-9) && q.approx_eq(b, 1e-9))
                    || (p.approx_eq(b, 1e-9) && q.approx_eq(a, 1e-9))
            })
            .unwrap()
    };
    let edges = vec![
        edge(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
        edge(Point3::new(0.0, 0.0, 0.02), Point3::new(1.0, 0.0, 0.02)),
    ];
    doc.blend_edges(
        o,
        &kernel::BlendSpec {
            edges,
            profile: kernel::BlendProfile::Bullnose { segments: 8 },
            primary: None,
            corners: kernel::CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        },
    )
    .unwrap();
    assert!(
        doc.object(o)
            .unwrap()
            .faces()
            .values()
            .all(|f| f.material != Some(paint)),
        "the painted end face is consumed"
    );
    doc.make_component(&[NodeId::Object(o)]).unwrap().0
}

#[test]
fn the_print_sees_the_paint_of_a_face_a_blend_consumed() {
    let mut red = Document::new();
    let r = plate(&mut red, Rgba8::rgb(200, 30, 30), "Red");
    let mut blue = Document::new();
    let b = plate(&mut blue, Rgba8::rgb(30, 30, 200), "Blue");
    assert_ne!(
        print(&red, r),
        print(&blue, b),
        "only the consumed face's paint differs"
    );
    // The same paint under another name prints alike.
    let mut crimson = Document::new();
    let c = plate(&mut crimson, Rgba8::rgb(200, 30, 30), "Crimson");
    assert_eq!(print(&red, r), print(&crimson, c));
    // And a round trip keeps it.
    let reopened = Document::load(&red.save()).unwrap();
    assert_eq!(
        print(&reopened, reopened.component_ids()[0]),
        print(&red, r)
    );
}

// ------------------------------------------------------------- properties

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Whatever placement, name and visibility the copies are given, the
    /// definition prints the same; and an update brings it in step and its
    /// undo restores the exact prior state.
    #[test]
    fn prop_print_is_blind_to_placement_and_update_undo_is_exact(
        moves in proptest::collection::vec(-20.0f64..20.0, 1..4),
        names in proptest::collection::vec("[a-z]{1,6}", 1..3),
        hide in any::<bool>(),
        rise in 0.01f64..2.0,
    ) {
        let (mut doc, leg, _v1, _) = two_legs();
        let p0 = print(&doc, leg);
        for (k, dx) in moves.iter().enumerate() {
            let i = doc.instance_ids()[k % 2];
            doc.transform_instance(i, &shift(*dx)).unwrap();
        }
        for n in &names {
            doc.set_component_name(leg, Some(n.clone())).unwrap();
        }
        doc.set_node_user_hidden(NodeId::Instance(doc.instance_ids()[0]), hide);
        prop_assert_eq!(print(&doc, leg), p0.clone());

        let mut v2 = item_of_height(1.0);
        let (v2c, _) = item_def(&v2);
        raise(&mut v2, v2c, rise);
        prop_assert_ne!(print(&v2, v2c), p0);
        let poses = instance_poses(&doc);
        let before = doc.state_hash();
        let (r, _) = doc
            .update_definitions_from_items(&[update(leg, &v2, prov("leg", "h2", &v2))], digest)
            .unwrap();
        prop_assert_eq!(r.updated, vec![leg]);
        prop_assert_eq!(instance_poses(&doc), poses);
        prop_assert_eq!(print(&doc, leg), print(&v2, v2c));
        doc.undo().unwrap();
        prop_assert_eq!(doc.state_hash(), before);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Status read inside a posed, open part equals the closed document's;
    /// entering and leaving without an edit reads in step.
    #[test]
    fn prop_open_part_status_matches_the_closed_document(
        dx in -20.0f64..20.0,
        dy in -20.0f64..20.0,
        dz in -20.0f64..20.0,
        angle in 0.0f64..std::f64::consts::TAU,
        scale in 0.25f64..4.0,
        push in proptest::option::of(0.05f64..2.0),
    ) {
        let (mut doc, _leg, v1, _v2) = two_legs();
        let lib = library(&[("leg", &v1)]);
        let i = doc.instance_ids()[0];
        let pose = Transform::uniform_scale(scale)
            .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), angle).unwrap())
            .then(&Transform::translation(Vec3::new(dx, dy, dz)));
        doc.transform_instance(i, &pose).unwrap();
        doc.open_explode_session(i).unwrap();
        if let Some(d) = push {
            let o = doc.explode_session_objects().expect("open")[0];
            let face = top_face(&doc, o);
            doc.apply_object_op(o, KernelOp::PushPull { face, distance: d }).unwrap();
        }
        let open = doc.library_links_as_if_closed(&lib, digest);
        let closed = {
            let mut c = doc.clone();
            c.close_innermost_session().unwrap();
            c.library_links(&lib, digest)
        };
        prop_assert_eq!(&open, &closed);
        if push.is_none() {
            prop_assert!(open.iter().all(|l| l.state == LibraryLinkState::InStep), "{:?}", open);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// A posed (scaled, rotated) part that places a posed, open part reads
    /// the same states as the closed document, edited or not: the open part
    /// is read as if it were closed, nested placements and all.
    #[test]
    fn prop_a_posed_part_placing_an_open_part_matches_the_closed_document(
        dx in -20.0f64..20.0,
        angle in 0.0f64..std::f64::consts::TAU,
        scale in 0.25f64..4.0,
        table_angle in 0.0f64..std::f64::consts::TAU,
        table_scale in 0.25f64..4.0,
        push in proptest::option::of(0.05f64..2.0),
    ) {
        let (mut doc, leg, v1, _v2) = two_legs();
        let pose = |a: f64, s: f64, x: f64| {
            Transform::uniform_scale(s)
                .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), a).unwrap())
                .then(&Transform::translation(Vec3::new(x, 0.0, 0.0)))
        };
        let second = doc.instance_ids()[1];
        doc.transform_instance(second, &pose(angle, scale, dx)).unwrap();
        let (table, table_i, _) = doc.make_component(&[NodeId::Instance(second)]).unwrap();
        doc.transform_instance(table_i, &pose(table_angle, table_scale, -dx)).unwrap();
        let t_print = print(&doc, table);
        let base = LibraryProvenance {
            source_id: "table".into(),
            content_hash: "t1".into(),
            prints: BTreeMap::from([(7u64, t_print.clone())]),
        };
        doc.stamp_library_source(&[NodeId::Instance(table_i)], &base, Some(7));
        let mut lib = library(&[("leg", &v1)]);
        lib.insert("table".into(), BTreeMap::from([(7u64, t_print)]));

        let leg_copy = doc
            .instance_ids()
            .into_iter()
            .find(|i| doc.instance_def(*i) == Some(leg))
            .expect("a root copy of the leg");
        doc.open_explode_session(leg_copy).unwrap();
        if let Some(d) = push {
            let o = doc.explode_session_objects().expect("open")[0];
            let face = top_face(&doc, o);
            doc.apply_object_op(o, KernelOp::PushPull { face, distance: d }).unwrap();
        }
        let open = doc.library_links_as_if_closed(&lib, digest);
        let closed = {
            let mut c = doc.clone();
            c.close_innermost_session().unwrap();
            c.library_links(&lib, digest)
        };
        prop_assert_eq!(open.len(), 2, "both the leg and the table are linked: {:?}", open);
        prop_assert_eq!(&open, &closed);
        let want = if push.is_some() { LibraryLinkState::EditedHere } else { LibraryLinkState::InStep };
        prop_assert!(open.iter().all(|l| l.state == want), "{:?}", open);
    }
}
