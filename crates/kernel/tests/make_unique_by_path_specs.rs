//! Specs for "Make Unique" by occurrence path (`make_unique_by_path`) and
//! its id remap, plus the identity fixes that ship with it: a forked
//! definition's copies keep their names, tags, and attributes, and a
//! boolean result keeps operand `a`'s identity (DEVELOPMENT.md rule 3).

use std::collections::BTreeSet;

use kernel::{
    AttrTarget, AttrValue, BooleanOp, ComponentId, DefRecipe, Document, DocumentError, EntityRef,
    GroupId, ImportNode, ImportScene, ImportSketch, ImportTag, InstanceId, KernelOp, MeshRecipe,
    NodeId, ObjectId, Plane, Point3, Remap, Transform, Vec3,
};
use proptest::prelude::*;

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

fn make_component_of(doc: &mut Document, nodes: &[NodeId]) -> (ComponentId, InstanceId) {
    let (c, i, _) = doc.make_component(nodes).expect("make_component");
    (c, i)
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

/// `A` (placed twice: `a1`, `a2`) places `DefA` = { box, `b` }; `b` places
/// `DefB` = { box, `c` }; `c` places `DefC` = { box }.
struct ThreeDeep {
    def_a: ComponentId,
    def_b: ComponentId,
    def_c: ComponentId,
    a1: InstanceId,
    a2: InstanceId,
    b: InstanceId,
    c: InstanceId,
    box_b: ObjectId,
}

fn three_deep(doc: &mut Document) -> ThreeDeep {
    let box_c = extrude_box(doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (def_c, c) = make_component_of(doc, &[NodeId::Object(box_c)]);
    let box_b = extrude_box(doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (def_b, b) = make_component_of(doc, &[NodeId::Object(box_b), NodeId::Instance(c)]);
    let box_a = extrude_box(doc, 4.0, 0.0, 5.0, 1.0, 1.0);
    let (def_a, a1) = make_component_of(doc, &[NodeId::Object(box_a), NodeId::Instance(b)]);
    let (a2, _) = doc.place_instance(def_a, shift(20.0)).unwrap();
    ThreeDeep {
        def_a,
        def_b,
        def_c,
        a1,
        a2,
        b,
        c,
        box_b,
    }
}

fn top_face(doc: &Document, object: ObjectId) -> kernel::FaceId {
    let obj = doc.object(object).expect("object is live");
    obj.faces()
        .iter()
        .max_by(|(_, a), (_, b)| {
            a.plane
                .normal()
                .z
                .partial_cmp(&b.plane.normal().z)
                .expect("finite normals")
        })
        .map(|(id, _)| id)
        .expect("object has faces")
}

fn max_z(doc: &Document, object: ObjectId) -> f64 {
    doc.object(object)
        .expect("object is live")
        .to_polygons()
        .0
        .iter()
        .map(|p| p.z)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// The objects rendered through world instance `outer`.
fn rendered_through(doc: &Document, outer: InstanceId) -> BTreeSet<ObjectId> {
    doc.expanded_placements()
        .into_iter()
        .filter(|(_, _, i)| *i == outer)
        .map(|(o, _, _)| o)
        .collect()
}

fn entity_dict(doc: &Document, e: EntityRef) -> Option<kernel::AttrDict> {
    doc.attr_get(&AttrTarget::Entity(e)).unwrap().cloned()
}

fn tag(path: &[&str]) -> Vec<String> {
    path.iter().map(|s| s.to_string()).collect()
}

// ------------------------------------------------------------- one level

#[test]
fn single_instance_path_matches_make_unique_and_returns_the_remap() {
    let mut doc = Document::new();
    let o1 = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let o2 = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (def, a1) = make_component_of(&mut doc, &[NodeId::Object(o1), NodeId::Object(o2)]);
    let (a2, _) = doc.place_instance(def, shift(10.0)).unwrap();

    let (new_def, leaf, remap, change) = doc.make_unique_by_path(&[a1], true, false).unwrap();

    assert_eq!(leaf, a1, "a world instance is repointed in place");
    assert_eq!(doc.instance_def(a1), Some(new_def));
    assert_eq!(doc.instance_def(a2), Some(def));
    let members = doc.def_members(new_def).unwrap();
    assert_eq!(members.len(), 2);
    assert!(
        !members.contains(&o1) && !members.contains(&o2),
        "fresh objects"
    );
    assert_eq!(remap.objects.len(), 2);
    assert_eq!(
        remap.objects.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([o1, o2])
    );
    assert_eq!(
        remap.objects.values().copied().collect::<BTreeSet<_>>(),
        members.iter().copied().collect::<BTreeSet<_>>()
    );
    assert_eq!(remap.components.len(), 1);
    assert_eq!(remap.components.get(&def), Some(&new_def));
    assert!(remap.instances.is_empty());
    assert!(remap.groups.is_empty());
    assert!(change.instances_touched.contains(&a1));
    assert!(change.components_touched.contains(&def));
    assert!(change.components_touched.contains(&new_def));
}

#[test]
fn make_unique_keeps_the_top_level_objects_name_and_tags() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("Leg".to_string()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(o), tag(&["Structure", "Leg"]))
        .unwrap();
    let (def, a1) = make_component_of(&mut doc, &[NodeId::Object(o)]);
    doc.place_instance(def, shift(5.0)).unwrap();

    let (_, _, remap, _) = doc.make_unique_by_path(&[a1], true, false).unwrap();
    let copy = remap.objects[&o];
    assert_eq!(doc.object_name(copy), Some("Leg"));
    assert_eq!(
        doc.node_tags(NodeId::Object(copy)),
        [tag(&["Structure", "Leg"])]
    );

    // The one-instance entry point carries the same fix.
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("Leg".to_string()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(o), tag(&["Structure", "Leg"]))
        .unwrap();
    let (_, a1) = make_component_of(&mut doc, &[NodeId::Object(o)]);
    let (new_def, _) = doc.make_unique(a1).unwrap();
    let copy = doc.def_members(new_def).unwrap()[0];
    assert_eq!(doc.object_name(copy), Some("Leg"));
    assert_eq!(
        doc.node_tags(NodeId::Object(copy)),
        [tag(&["Structure", "Leg"])]
    );
}

#[test]
fn make_unique_keeps_attributes_on_copied_members() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let inner = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(inner)]).unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(o)),
        "com.example",
        "uuid",
        AttrValue::Text("board-1".into()),
    )
    .unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Group(g)),
        "com.example",
        "role",
        AttrValue::Text("panel".into()),
    )
    .unwrap();
    let (_, a1) = make_component_of(&mut doc, &[NodeId::Object(o), NodeId::Group(g)]);

    let (_, _, remap, _) = doc.make_unique_by_path(&[a1], true, false).unwrap();
    assert_eq!(
        entity_dict(&doc, EntityRef::Object(remap.objects[&o])),
        entity_dict(&doc, EntityRef::Object(o))
    );
    assert!(entity_dict(&doc, EntityRef::Object(o)).is_some());
    let g_copy: GroupId = remap.groups[&g];
    assert_eq!(
        entity_dict(&doc, EntityRef::Group(g_copy)),
        entity_dict(&doc, EntityRef::Group(g))
    );
    assert!(entity_dict(&doc, EntityRef::Group(g)).is_some());
    assert!(
        remap.objects.contains_key(&inner),
        "the group's leaf is copied too"
    );
}

#[test]
fn a_user_hidden_member_stays_hidden_in_the_fork() {
    let mut doc = Document::new();
    let shown = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let hidden = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (_, a1) = make_component_of(&mut doc, &[NodeId::Object(shown), NodeId::Object(hidden)]);
    doc.set_node_user_hidden(NodeId::Object(hidden), true);
    assert!(doc.node_user_hidden(NodeId::Object(hidden)));

    let (_, _, remap, _) = doc.make_unique_by_path(&[a1], true, false).unwrap();
    assert!(doc.node_user_hidden(NodeId::Object(remap.objects[&hidden])));
    assert!(!doc.node_user_hidden(NodeId::Object(remap.objects[&shown])));
}

#[test]
fn forked_definition_keeps_the_linked_attribute_and_drops_the_library_stamp() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (def, a1) = make_component_of(&mut doc, &[NodeId::Object(o)]);
    let target = AttrTarget::Entity(EntityRef::Component(def));
    doc.attr_set(target.clone(), "hew", "linked", AttrValue::Bool(true))
        .unwrap();
    doc.attr_set(
        target.clone(),
        "hew.library",
        "item",
        AttrValue::Text("abc".into()),
    )
    .unwrap();
    let before = entity_dict(&doc, EntityRef::Component(def));

    let (new_def, _, _, _) = doc.make_unique_by_path(&[a1], true, false).unwrap();
    let dict = entity_dict(&doc, EntityRef::Component(new_def)).expect("attributes copied");
    assert_eq!(
        dict.get("hew").and_then(|ns| ns.get("linked")),
        Some(&AttrValue::Bool(true))
    );
    assert!(!dict.contains_key("hew.library"));
    assert_eq!(entity_dict(&doc, EntityRef::Component(def)), before);
}

// ------------------------------------------------------------- cascade

#[test]
fn cascade_true_forks_every_instance_on_the_path_outermost_first() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);

    let (comp, leaf, remap, _) = doc.make_unique_by_path(&[t.a1, t.b], true, false).unwrap();
    let new_a = remap.components[&t.def_a];
    let new_b = remap.components[&t.def_b];
    assert_eq!(doc.instance_def(t.a1), Some(new_a));
    let b_copy = remap.instances[&t.b];
    assert_eq!(doc.instance_def(b_copy), Some(new_b));
    assert_eq!((comp, leaf), (new_b, b_copy));
    assert!(
        doc.def_member_nodes(new_a)
            .unwrap()
            .contains(&NodeId::Instance(b_copy))
    );
    assert_eq!(remap.components.len(), 2);
}

#[test]
fn cascade_true_leaves_the_other_placements_of_the_outer_definition_untouched() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);

    let (comp, _, remap, _) = doc.make_unique_by_path(&[t.a1, t.b], true, false).unwrap();
    assert_eq!(doc.instance_def(t.a2), Some(t.def_a));
    assert_eq!(doc.instance_def(t.b), Some(t.def_b));

    let pushed = remap.objects[&t.box_b];
    assert!(doc.def_members(comp).unwrap().contains(&pushed));
    let original_height = max_z(&doc, t.box_b);
    let top = top_face(&doc, pushed);
    doc.apply_def_op(
        comp,
        pushed,
        KernelOp::PushPull {
            face: top,
            distance: 0.5,
        },
    )
    .unwrap();
    // A push/pull keeps the object id; the copy grows, the source does not.
    assert!(max_z(&doc, pushed) > original_height + 0.4);
    assert_eq!(max_z(&doc, t.box_b), original_height);
    let via_a1 = rendered_through(&doc, t.a1);
    let via_a2 = rendered_through(&doc, t.a2);
    assert!(via_a1.contains(&pushed) && !via_a1.contains(&t.box_b));
    assert!(via_a2.contains(&t.box_b) && !via_a2.contains(&pushed));
}

#[test]
fn cascade_false_forks_only_the_last_definition_in_place() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);

    let (comp, leaf, remap, change) = doc.make_unique_by_path(&[t.a1, t.b], false, false).unwrap();
    assert_eq!(leaf, t.b);
    assert_eq!(doc.instance_def(t.a1), Some(t.def_a));
    assert_eq!(doc.instance_def(t.b), Some(comp));
    assert_ne!(comp, t.def_b);
    assert_eq!(
        remap.components.keys().copied().collect::<Vec<_>>(),
        vec![t.def_b]
    );
    assert!(!remap.instances.contains_key(&t.b));
    // The shared outer definition shows the fork through every placement.
    let box_copy = remap.objects[&t.box_b];
    assert!(rendered_through(&doc, t.a2).contains(&box_copy));
    assert!(rendered_through(&doc, t.a1).contains(&box_copy));
    assert!(change.instances_touched.contains(&t.a1));
    assert!(change.instances_touched.contains(&t.a2));
}

#[test]
fn cascade_false_on_a_loaned_owner_definition_refuses_explode_session_scope() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    doc.open_explode_session(t.a1).unwrap();
    let hash = doc.state_hash();
    let generation = doc.history_generation();

    for cascade in [false, true] {
        assert_eq!(
            doc.make_unique_by_path(&[t.a2, t.b], cascade, false)
                .unwrap_err(),
            DocumentError::ExplodeSessionScope,
            "cascade {cascade}"
        );
        assert_eq!(
            doc.make_unique_by_path(&[t.a2], cascade, true).unwrap_err(),
            DocumentError::ExplodeSessionScope,
            "cascade {cascade}"
        );
    }
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.history_generation(), generation);
}

#[test]
fn a_deep_fork_refuses_when_a_nested_definition_is_on_loan() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    // A world placement of the innermost definition, in a session.
    let (world_c, _) = doc.place_instance(t.def_c, shift(-10.0)).unwrap();
    doc.open_explode_session(world_c).unwrap();
    let hash = doc.state_hash();
    assert_eq!(
        doc.make_unique_by_path(&[t.a2], true, true).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(doc.state_hash(), hash);
}

// ------------------------------------------------------------- deep

#[test]
fn deep_false_shares_nested_definitions_with_the_source() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    let (_, _, remap, _) = doc.make_unique_by_path(&[t.a1], true, false).unwrap();
    assert_eq!(doc.instance_def(remap.instances[&t.b]), Some(t.def_b));
    assert_eq!(remap.components.len(), 1);
}

#[test]
fn deep_true_forks_nested_definitions_recursively_and_remaps_them() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    let before: Vec<_> = [t.def_a, t.def_b, t.def_c]
        .iter()
        .map(|&d| doc.def_member_nodes(d).unwrap())
        .collect();

    let (_, _, remap, _) = doc.make_unique_by_path(&[t.a1], true, true).unwrap();
    assert_eq!(remap.components.len(), 3);
    let new_b = remap.components[&t.def_b];
    let new_c = remap.components[&t.def_c];
    let b_copy = remap.instances[&t.b];
    let c_copy = remap.instances[&t.c];
    assert_eq!(doc.instance_def(b_copy), Some(new_b));
    assert_eq!(doc.instance_def(c_copy), Some(new_c));
    assert!(
        doc.def_member_nodes(new_b)
            .unwrap()
            .contains(&NodeId::Instance(c_copy))
    );
    let after: Vec<_> = [t.def_a, t.def_b, t.def_c]
        .iter()
        .map(|&d| doc.def_member_nodes(d).unwrap())
        .collect();
    assert_eq!(
        before, after,
        "no member of the original definitions changed"
    );
    assert_eq!(doc.instance_def(t.b), Some(t.def_b));
    assert_eq!(doc.instance_def(t.c), Some(t.def_c));
}

#[test]
fn deep_true_shares_one_private_copy_per_old_definition_inside_the_forked_tree() {
    let mut doc = Document::new();
    let leg = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (def_b, b1) = make_component_of(&mut doc, &[NodeId::Object(leg)]);
    let (b2, _) = doc.place_instance(def_b, shift(3.0)).unwrap();
    let top = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (def_a, a1) = make_component_of(
        &mut doc,
        &[
            NodeId::Object(top),
            NodeId::Instance(b1),
            NodeId::Instance(b2),
        ],
    );
    doc.place_instance(def_a, shift(20.0)).unwrap();

    let (_, _, remap, _) = doc.make_unique_by_path(&[a1], true, true).unwrap();
    assert_eq!(remap.components.len(), 2);
    let new_b = remap.components[&def_b];
    assert_eq!(doc.instance_def(remap.instances[&b1]), Some(new_b));
    assert_eq!(doc.instance_def(remap.instances[&b2]), Some(new_b));
    assert_eq!(remap.objects.len(), 2, "the top board and one leg copy");

    // One undo restores both shared repoints.
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.instance_def(b1), Some(def_b));
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    assert_eq!(doc.instance_def(remap.instances[&b2]), Some(new_b));
}

#[test]
fn a_nested_deep_fork_keeps_its_instance_name() {
    let mut doc = Document::new();
    let leg = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (def_b, b1) = make_component_of(&mut doc, &[NodeId::Object(leg)]);
    doc.set_node_name(NodeId::Instance(b1), Some("Front leg".into()))
        .unwrap();
    let top = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (_, a1) = make_component_of(&mut doc, &[NodeId::Object(top), NodeId::Instance(b1)]);
    let def_b_name = doc.component_name(def_b).unwrap().to_string();

    let (_, _, remap, _) = doc.make_unique_by_path(&[a1], true, true).unwrap();
    let b_copy = remap.instances[&b1];
    assert_eq!(doc.instance_name(b_copy), Some("Front leg"));
    assert_eq!(
        doc.component_name(remap.components[&def_b]),
        Some(format!("{def_b_name} Copy").as_str())
    );
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.instance_name(b_copy), Some("Front leg"));
}

// ------------------------------------------------------------- validation

#[test]
fn broken_path_refuses_and_leaves_the_document_untouched() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    let (gone, _) = doc.place_instance(t.def_a, shift(40.0)).unwrap();
    doc.delete_node(NodeId::Instance(gone)).unwrap();
    let hash = doc.state_hash();
    let generation = doc.history_generation();

    let cases: Vec<(&str, Vec<InstanceId>, DocumentError)> = vec![
        ("empty", vec![], DocumentError::EmptySelection),
        ("hidden", vec![gone], DocumentError::UnknownInstance),
        ("def-owned first", vec![t.b], DocumentError::UnknownInstance),
        (
            "second not owned by the first",
            vec![t.a1, t.a2],
            DocumentError::UnknownInstance,
        ),
        (
            "skips a level",
            vec![t.a1, t.c],
            DocumentError::UnknownInstance,
        ),
        (
            "stale",
            vec![InstanceId::default()],
            DocumentError::UnknownInstance,
        ),
        (
            "stale inner",
            vec![t.a1, InstanceId::default()],
            DocumentError::UnknownInstance,
        ),
    ];
    for (what, path, want) in cases {
        for cascade in [true, false] {
            for deep in [true, false] {
                assert_eq!(
                    doc.make_unique_by_path(&path, cascade, deep).unwrap_err(),
                    want,
                    "{what} (cascade {cascade}, deep {deep})"
                );
                assert_eq!(doc.state_hash(), hash, "{what}");
                assert_eq!(doc.history_generation(), generation, "{what}");
            }
        }
    }
}

#[test]
fn make_unique_of_a_def_owned_instance_through_the_old_entry_point_still_refuses() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    let hash = doc.state_hash();
    assert_eq!(
        doc.make_unique(t.b).unwrap_err(),
        DocumentError::UnknownInstance
    );
    assert_eq!(doc.state_hash(), hash);
}

// ------------------------------------------------------------- history

#[test]
fn one_undo_entry_for_a_three_deep_cascade() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    let before = doc.state_hash();
    let depth = doc.history_entries().undo.len();

    doc.make_unique_by_path(&[t.a1, t.b, t.c], true, false)
        .unwrap();
    let after = doc.state_hash();
    assert_ne!(before, after);
    assert_eq!(doc.history_entries().undo.len(), depth + 1);
    assert_eq!(doc.peek_undo_meta().unwrap().label, "Make Unique");

    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert_eq!(doc.history_entries().undo.len(), depth);
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
}

#[test]
fn a_single_fork_records_the_plain_made_unique_action() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    doc.make_unique_by_path(&[t.a1], true, false).unwrap();
    assert!(doc.peek_undo_meta().is_none(), "not a labeled compound");
    let entries = doc.history_entries();
    assert_eq!(entries.undo.last().unwrap().kind.id, "make-unique");

    // A cascade:false fork of a nested placement is one fork too.
    doc.make_unique_by_path(&[t.a2, t.b], false, false).unwrap();
    assert!(doc.peek_undo_meta().is_none());
    assert_eq!(
        doc.history_entries().undo.last().unwrap().kind.id,
        "make-unique"
    );
}

#[test]
fn instance_name_is_promoted_to_the_definition_name_at_each_step() {
    let mut doc = Document::new();
    let box_b = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (def_b, b) = make_component_of(&mut doc, &[NodeId::Object(box_b)]);
    doc.set_node_name(NodeId::Instance(b), Some("Drawer".into()))
        .unwrap();
    let box_a = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (def_a, a1) = make_component_of(&mut doc, &[NodeId::Object(box_a), NodeId::Instance(b)]);
    doc.set_node_name(NodeId::Instance(a1), Some("Table".into()))
        .unwrap();
    let (a2, _) = doc.place_instance(def_a, shift(10.0)).unwrap();
    let (a3, _) = doc.place_instance(def_a, shift(20.0)).unwrap();
    let def_b_name = doc.component_name(def_b).unwrap().to_string();

    let (comp, leaf, remap, _) = doc.make_unique_by_path(&[a1, b], true, false).unwrap();
    assert_eq!(doc.component_name(remap.components[&def_a]), Some("Table"));
    assert_eq!(doc.component_name(comp), Some("Drawer"));
    assert_eq!(doc.instance_name(a1), None);
    assert_eq!(doc.instance_name(leaf), None);
    assert_eq!(
        doc.instance_name(b),
        Some("Drawer"),
        "the source keeps its name"
    );
    assert_eq!(doc.component_name(def_b), Some(def_b_name.as_str()));

    let base = doc.component_name(def_a).unwrap().to_string();
    let (first, _, _, _) = doc.make_unique_by_path(&[a2], true, false).unwrap();
    assert_eq!(
        doc.component_name(first),
        Some(format!("{base} Copy").as_str())
    );
    let (second, _, _, _) = doc.make_unique_by_path(&[a3], true, false).unwrap();
    assert_eq!(
        doc.component_name(second),
        Some(format!("{base} Copy 2").as_str())
    );

    // Undo of the cascade restores both promoted names.
    doc.undo().unwrap();
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.instance_name(a1), Some("Table"));
}

#[test]
fn result_instance_is_the_placement_at_the_end_of_the_path_after_the_call() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);

    let (_, leaf, _, _) = doc.make_unique_by_path(&[t.a1], true, false).unwrap();
    assert_eq!(leaf, t.a1);
    doc.undo().unwrap();

    let (comp, leaf, _, _) = doc.make_unique_by_path(&[t.a1, t.b], false, false).unwrap();
    assert_eq!(leaf, t.b);
    assert_eq!(doc.instance_def(t.b), Some(comp));
    doc.undo().unwrap();

    let (comp, leaf, remap, _) = doc.make_unique_by_path(&[t.a1, t.b], true, false).unwrap();
    assert_eq!(leaf, remap.instances[&t.b]);
    assert_eq!(doc.instance_def(leaf), Some(comp));
    let outer = doc.instance_def(t.a1).unwrap();
    assert!(
        doc.def_member_nodes(outer)
            .unwrap()
            .contains(&NodeId::Instance(leaf))
    );
    assert!(doc.instance_pose(leaf).is_some(), "a live record");
}

// ------------------------------------------------------------- sketches

fn cube_mesh(x0: f64) -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x0 + x, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
    ];
    let faces: Vec<Vec<usize>> = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let n = faces.len();
    MeshRecipe {
        name: "Cube".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

#[test]
fn copied_definition_sketches_and_their_tags_follow_the_forked_definition() {
    let layer = tag(&["Drafting"]);
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: Vec::new(),
        defs: vec![DefRecipe::from_meshes(
            Some("Panel".into()),
            vec![cube_mesh(0.0)],
        )],
        roots: vec![ImportNode::Instance {
            def: 0,
            pose: Transform::IDENTITY,
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: None,
        }],
        guides: Vec::new(),
        tags: vec![ImportTag {
            path: layer.clone(),
            hidden: false,
        }],
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: kernel::ImportSceneExtra {
            sketches: vec![ImportSketch {
                plane: ground(),
                def: Some(0),
                group: None,
                segments: vec![[Point3::new(3.0, 0.0, 0.0), Point3::new(3.0, 1.0, 0.0)]],
                curves: Vec::new(),
                tags: vec![layer.clone()],
            }],
            ..Default::default()
        },
    };
    let (report, _) = doc.ingest(scene, Vec::new()).unwrap();
    let def = report.defs[0].unwrap();
    let tagged = report.sketches[0].unwrap();
    let a1 = doc.instance_ids()[0];
    doc.place_instance(def, shift(10.0)).unwrap();
    assert_eq!(doc.sketch_tags(tagged), std::slice::from_ref(&layer));

    // A consumed sketch inside the definition (drawn, then extruded).
    let (consumed, _) = doc.begin_sketch_on_plane_in_instance(a1, ground()).unwrap();
    {
        let sk = doc.sketch_mut(consumed).unwrap();
        let c = [
            Point3::new(5.0, 0.0, 0.0),
            Point3::new(6.0, 0.0, 0.0),
            Point3::new(6.0, 1.0, 0.0),
            Point3::new(5.0, 1.0, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).unwrap();
        }
    }
    let region = doc.extrudable_regions(consumed).unwrap()[0];
    doc.extrude_region_in_instance(a1, consumed, region, 1.0)
        .unwrap();
    assert!(doc.sketch(consumed).is_none(), "the extrude consumed it");
    let listed_before = doc.def_member_sketches(def).unwrap();
    let live_before: Vec<_> = listed_before
        .iter()
        .copied()
        .filter(|&s| doc.sketch(s).is_some())
        .collect();
    assert_eq!(live_before, vec![tagged]);

    let (new_def, _, _, change) = doc.make_unique_by_path(&[a1], true, false).unwrap();
    let cloned = doc.def_member_sketches(new_def).unwrap();
    assert_eq!(
        cloned.len(),
        live_before.len(),
        "only live sketches are cloned"
    );
    assert!(!cloned.contains(&tagged) && !cloned.contains(&consumed));
    assert!(
        cloned
            .iter()
            .any(|&s| doc.sketch_tags(s) == [layer.clone()]),
        "the tag is kept"
    );
    for &s in &cloned {
        assert!(change.sketches_touched.contains(&s));
    }
    assert_eq!(doc.sketch_owner_component(tagged), Some(def));
    assert_eq!(doc.sketch_tags(tagged), std::slice::from_ref(&layer));
    assert_eq!(doc.def_member_sketches(def).unwrap(), listed_before);
}

// ------------------------------------------------------------- save

#[test]
fn save_and_reload_after_a_cascade_fork_round_trips() {
    let mut doc = Document::new();
    let t = three_deep(&mut doc);
    let (_, _, remap, _) = doc.make_unique_by_path(&[t.a1, t.b], true, true).unwrap();
    assert!(!remap.components.is_empty());
    let loaded = Document::load(&doc.save()).unwrap();
    assert_eq!(loaded.state_hash(), doc.state_hash());
    assert_eq!(loaded.component_ids().len(), doc.component_ids().len());
    assert_eq!(loaded.instance_ids().len(), doc.instance_ids().len());
    let live_components = doc.component_ids();
    for new in remap.components.values() {
        assert!(live_components.contains(new));
    }
    for &new in remap.instances.values() {
        assert!(doc.instance_def(new).is_some(), "a live instance");
    }
}

// ------------------------------------------------------------- booleans

fn named_tagged_box(doc: &mut Document, x: f64, name: &str, layer: &[&str]) -> ObjectId {
    let o = extrude_box(doc, x, 0.0, x + 2.0, 1.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some(name.to_string()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(o), tag(layer)).unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(o)),
        "com.example",
        "part",
        AttrValue::Text(name.to_string()),
    )
    .unwrap();
    o
}

#[test]
fn boolean_in_component_result_keeps_operand_a_name_tags_and_attributes() {
    let mut doc = Document::new();
    let a = named_tagged_box(&mut doc, 0.0, "Rail", &["Frame"]);
    let b = named_tagged_box(&mut doc, 1.0, "Cutter", &["Tools"]);
    let a_dict = entity_dict(&doc, EntityRef::Object(a));
    let (def, _) = make_component_of(&mut doc, &[NodeId::Object(a), NodeId::Object(b)]);

    let (result, _) = doc
        .boolean_in_component(def, a, b, BooleanOp::Union)
        .unwrap();
    assert_eq!(doc.object_name(result), Some("Rail"));
    assert_eq!(doc.node_tags(NodeId::Object(result)), [tag(&["Frame"])]);
    assert_eq!(entity_dict(&doc, EntityRef::Object(result)), a_dict);
}

#[test]
fn boolean_in_component_result_is_exactly_restored_by_undo() {
    let mut doc = Document::new();
    let a = named_tagged_box(&mut doc, 0.0, "Rail", &["Frame"]);
    let b = named_tagged_box(&mut doc, 1.0, "Cutter", &["Tools"]);
    let (def, _) = make_component_of(&mut doc, &[NodeId::Object(a), NodeId::Object(b)]);
    let before = doc.state_hash();

    let (result, _) = doc
        .boolean_in_component(def, a, b, BooleanOp::Subtract)
        .unwrap();
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert!(doc.object(result).is_none(), "the result is hidden");
    assert_eq!(doc.object_name(a), Some("Rail"));
    assert_eq!(doc.object_name(b), Some("Cutter"));
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    assert!(doc.def_members(def).unwrap().contains(&result));
    assert_eq!(doc.object_name(result), Some("Rail"));
}

#[test]
fn boolean_result_keeps_operand_a_name_tags_and_attributes() {
    let mut doc = Document::new();
    let a = named_tagged_box(&mut doc, 0.0, "Rail", &["Frame"]);
    let b = named_tagged_box(&mut doc, 1.0, "Cutter", &["Tools"]);
    let a_dict = entity_dict(&doc, EntityRef::Object(a));
    let (result, _) = doc.boolean(BooleanOp::Union, a, b).unwrap();
    assert_eq!(doc.object_name(result), Some("Rail"));
    assert_eq!(doc.node_tags(NodeId::Object(result)), [tag(&["Frame"])]);
    assert_eq!(entity_dict(&doc, EntityRef::Object(result)), a_dict);
}

#[test]
fn boolean_result_does_not_take_operand_b_identity() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 2.0, 1.0, 1.0);
    let b = named_tagged_box(&mut doc, 1.0, "Cutter", &["Tools"]);
    let (result, _) = doc.boolean(BooleanOp::Subtract, a, b).unwrap();
    assert_eq!(doc.object_name(result), None);
    assert!(doc.node_tags(NodeId::Object(result)).is_empty());
    assert_eq!(entity_dict(&doc, EntityRef::Object(result)), None);

    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 2.0, 1.0, 1.0);
    let b = named_tagged_box(&mut doc, 1.0, "Cutter", &["Tools"]);
    let (def, _) = make_component_of(&mut doc, &[NodeId::Object(a), NodeId::Object(b)]);
    let (result, _) = doc
        .boolean_in_component(def, a, b, BooleanOp::Subtract)
        .unwrap();
    assert_eq!(doc.object_name(result), None);
    assert!(doc.node_tags(NodeId::Object(result)).is_empty());
    assert_eq!(entity_dict(&doc, EntityRef::Object(result)), None);
}

#[test]
fn boolean_result_is_exactly_restored_by_undo() {
    let mut doc = Document::new();
    let a = named_tagged_box(&mut doc, 0.0, "Rail", &["Frame"]);
    let b = named_tagged_box(&mut doc, 1.0, "Cutter", &["Tools"]);
    let before = doc.state_hash();
    let (result, _) = doc.boolean(BooleanOp::Intersect, a, b).unwrap();
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert!(doc.object(result).is_none());
    assert_eq!(doc.object_name(a), Some("Rail"));
    assert_eq!(doc.object_name(b), Some("Cutter"));
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    assert_eq!(doc.object_name(result), Some("Rail"));
}

// ------------------------------------------------------------- properties

/// One level of a generated tree: its own boxes, whether it holds a group
/// (of one box), and how many placements of the level below it holds.
#[derive(Debug, Clone)]
struct Level {
    boxes: usize,
    group: bool,
    nested: usize,
}

#[derive(Debug, Clone)]
struct Case {
    levels: Vec<Level>,
    placements: usize,
    sel: usize,
    cascade: bool,
    deep: bool,
}

fn arb_case() -> impl Strategy<Value = Case> {
    let level = (1usize..=2, any::<bool>(), 1usize..=2).prop_map(|(boxes, group, nested)| Level {
        boxes,
        group,
        nested,
    });
    (
        proptest::collection::vec(level, 1..=3),
        1usize..=3,
        any::<usize>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(levels, placements, sel, cascade, deep)| Case {
            levels,
            placements,
            sel,
            cascade,
            deep,
        })
}

/// Builds the case's document bottom-up and returns it with the chosen
/// path (instance ids from the world down, the picked placement last).
fn build(case: &Case) -> (Document, Vec<InstanceId>) {
    let mut doc = Document::new();
    let mut x = 0.0;
    let mut next_box = |doc: &mut Document| {
        let o = extrude_box(doc, x, 0.0, x + 1.0, 1.0, 1.0);
        x += 2.0;
        o
    };
    let mut below: Option<(ComponentId, InstanceId)> = None;
    for level in &case.levels {
        let mut nodes: Vec<NodeId> = Vec::new();
        for _ in 0..level.boxes {
            nodes.push(NodeId::Object(next_box(&mut doc)));
        }
        if level.group {
            let inner = next_box(&mut doc);
            let (g, _) = doc.group_nodes(&[NodeId::Object(inner)]).unwrap();
            nodes.push(NodeId::Group(g));
        }
        if let Some((def, first)) = below {
            nodes.push(NodeId::Instance(first));
            for n in 1..level.nested {
                let (i, _) = doc.place_instance(def, shift(100.0 * n as f64)).unwrap();
                nodes.push(NodeId::Instance(i));
            }
        }
        let (def, inst, _) = doc.make_component(&nodes).unwrap();
        below = Some((def, inst));
    }
    let (root, _) = below.unwrap();
    for n in 1..case.placements {
        doc.place_instance(root, shift(1000.0 * n as f64)).unwrap();
    }
    let rows: Vec<Vec<InstanceId>> = doc
        .occurrences(None, false)
        .unwrap()
        .into_iter()
        .filter_map(|o| match o.node {
            NodeId::Instance(i) => {
                let mut p = o.path.clone();
                p.push(i);
                Some(p)
            }
            _ => None,
        })
        .collect();
    let path = rows[case.sel % rows.len()].clone();
    (doc, path)
}

#[derive(Default)]
struct Ids {
    objects: BTreeSet<ObjectId>,
    groups: BTreeSet<GroupId>,
    instances: BTreeSet<InstanceId>,
    components: BTreeSet<ComponentId>,
}

/// Every id the document can name: the live ones plus every node the
/// occurrence walk reaches, hidden included.
fn all_ids(doc: &Document) -> Ids {
    let mut ids = Ids {
        objects: doc.live_object_ids().into_iter().collect(),
        groups: doc.group_ids().into_iter().collect(),
        instances: doc.instance_ids().into_iter().collect(),
        components: doc.component_ids().into_iter().collect(),
    };
    for o in doc.occurrences(None, true).unwrap() {
        match o.node {
            NodeId::Object(id) => {
                ids.objects.insert(id);
            }
            NodeId::Group(id) => {
                ids.groups.insert(id);
            }
            NodeId::Instance(id) => {
                ids.instances.insert(id);
            }
        }
        if let Some(d) = o.definition {
            ids.components.insert(d);
        }
    }
    ids
}

fn check_map<K: Ord + Copy + std::fmt::Debug>(
    what: &str,
    map: &std::collections::BTreeMap<K, K>,
    before: &BTreeSet<K>,
) -> Result<(), TestCaseError> {
    let values: BTreeSet<K> = map.values().copied().collect();
    prop_assert_eq!(values.len(), map.len(), "{} values are distinct", what);
    for (k, v) in map {
        prop_assert!(before.contains(k), "{} key {:?} existed before", what, k);
        prop_assert!(!before.contains(v), "{} value {:?} is new", what, v);
        prop_assert!(!map.contains_key(v), "{} value {:?} is not a key", what, v);
    }
    Ok(())
}

fn instance_rows(doc: &Document) -> usize {
    doc.occurrences(None, true)
        .unwrap()
        .iter()
        .filter(|o| matches!(o.node, NodeId::Instance(_)))
        .count()
}

fn world_roots(doc: &Document) -> BTreeSet<NodeId> {
    doc.occurrences(None, true)
        .unwrap()
        .iter()
        .filter(|o| o.depth == 0)
        .map(|o| o.node)
        .collect()
}

fn check_remap(remap: &Remap, before: &Ids, in_place: &[InstanceId]) -> Result<(), TestCaseError> {
    check_map("objects", &remap.objects, &before.objects)?;
    check_map("groups", &remap.groups, &before.groups)?;
    check_map("instances", &remap.instances, &before.instances)?;
    check_map("components", &remap.components, &before.components)?;
    for i in in_place {
        prop_assert!(!remap.instances.contains_key(i));
        prop_assert!(!remap.instances.values().any(|v| v == i));
    }
    Ok(())
}

proptest! {
    #[test]
    fn remap_is_a_bijection_onto_fresh_ids(case in arb_case()) {
        let (mut doc, path) = build(&case);
        let before = all_ids(&doc);
        let hash = doc.state_hash();
        let generation = doc.history_generation();
        match doc.make_unique_by_path(&path, case.cascade, case.deep) {
            Ok((comp, leaf, remap, _)) => {
                let in_place: Vec<InstanceId> = if case.cascade {
                    vec![path[0]]
                } else {
                    vec![*path.last().unwrap()]
                };
                check_remap(&remap, &before, &in_place)?;
                prop_assert_eq!(doc.instance_def(leaf), Some(comp));
                prop_assert!(!remap.components.is_empty());
            }
            Err(_) => {
                prop_assert_eq!(doc.state_hash(), hash);
                prop_assert_eq!(doc.history_generation(), generation);
            }
        }
    }

    #[test]
    fn placement_count_is_unchanged_by_make_unique_by_path(case in arb_case()) {
        let (mut doc, path) = build(&case);
        let rows = instance_rows(&doc);
        let roots = world_roots(&doc);
        doc.make_unique_by_path(&path, case.cascade, case.deep).unwrap();
        prop_assert_eq!(instance_rows(&doc), rows);
        prop_assert_eq!(world_roots(&doc), roots);
    }

    #[test]
    fn undo_then_redo_reproduces_the_state_hash_after_make_unique_by_path(case in arb_case()) {
        let (mut doc, path) = build(&case);
        let before = doc.state_hash();
        doc.make_unique_by_path(&path, case.cascade, case.deep).unwrap();
        let after = doc.state_hash();
        doc.undo().unwrap();
        prop_assert_eq!(doc.state_hash(), before);
        doc.redo().unwrap();
        prop_assert_eq!(doc.state_hash(), after);
    }
}
