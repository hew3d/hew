//! Specs for linked copies: wrapping an Object or Group as a linked set
//! (`wrap_linked`), copies that link by default (`LinkPolicy::Linked` on
//! the duplicating ops, one undo entry with the wraps), a linked set going
//! with its last placement, promote, edits through a placement, and the
//! change-set fields (`linked`, `copied_from`). DEVELOPMENT.md rule 3: the
//! properties at the end.

use std::collections::BTreeSet;
use std::num::NonZeroU32;

use kernel::{
    Anchor, AttrTarget, AttrValue, ChangeRecord, ChangeRef, ComponentId, Document, DocumentError,
    EntityRef, GroupId, InstanceId, KernelOp, LinkPolicy, LinkedWrap, Material, NodeId, ObjectId,
    Plane, Point3, Rgba8, Transform, Vec3,
};
use proptest::prelude::*;

use LinkPolicy::{Independent, Linked};

// ------------------------------------------------------------- helpers

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

fn board(doc: &mut Document, x: f64) -> ObjectId {
    extrude_box(doc, x, 0.0, x + 1.0, 1.0, 1.0)
}

fn tag(path: &[&str]) -> Vec<String> {
    path.iter().map(|s| s.to_string()).collect()
}

/// A board with a name, tags, and `com.example` attributes.
fn named_board(
    doc: &mut Document,
    x: f64,
    name: &str,
    tags: &[&[&str]],
    attrs: &[(&str, &str)],
) -> ObjectId {
    let o = board(doc, x);
    doc.set_node_name(NodeId::Object(o), Some(name.to_string()))
        .unwrap();
    for t in tags {
        doc.add_node_tag(NodeId::Object(o), tag(t)).unwrap();
    }
    for (k, v) in attrs {
        doc.attr_set(
            AttrTarget::Entity(EntityRef::Object(o)),
            "com.example",
            k,
            AttrValue::Text((*v).to_string()),
        )
        .unwrap();
    }
    o
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

fn polys(doc: &Document, o: ObjectId) -> Vec<Point3> {
    doc.object(o).expect("object is live").to_polygons().0
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
    polys(doc, object)
        .iter()
        .map(|p| p.z)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn entity_dict(doc: &Document, e: EntityRef) -> Option<kernel::AttrDict> {
    doc.attr_get(&AttrTarget::Entity(e)).unwrap().cloned()
}

fn ins(n: NodeId) -> InstanceId {
    match n {
        NodeId::Instance(i) => i,
        other => panic!("expected an instance, got {other:?}"),
    }
}

fn last_record(doc: &Document) -> ChangeRecord {
    doc.change_records_since_epoch(0)
        .expect("covered")
        .last()
        .expect("a record")
        .clone()
}

fn cref(doc: &Document, e: EntityRef) -> ChangeRef {
    ChangeRef {
        sid: doc.sid_of(&e).expect("a stable id"),
        entity: e,
    }
}

/// The world-space minimum x of every placement of `object`.
fn placed_min_x(doc: &Document, object: ObjectId) -> Vec<f64> {
    let local = polys(doc, object);
    let mut xs: Vec<f64> = doc
        .expanded_placements()
        .into_iter()
        .filter(|(o, _, _)| *o == object)
        .map(|(_, pose, _)| {
            local
                .iter()
                .map(|p| pose.apply_point(*p).x)
                .fold(f64::INFINITY, f64::min)
        })
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn last_label(doc: &Document) -> String {
    doc.history_entries()
        .undo
        .last()
        .expect("an undo entry")
        .label
        .clone()
}

/// What renders: visible world objects plus every expanded placement.
fn rendered(doc: &Document) -> usize {
    doc.visible_object_ids().len() + doc.expanded_placements().len()
}

// ------------------------------------------------------------------ wrap

#[test]
fn wrap_linked_keeps_the_object_id_name_tags_attributes_and_history() {
    let mut doc = Document::new();
    let o = named_board(
        &mut doc,
        0.0,
        "Leg",
        &[&["Structure", "Leg"]],
        &[("uuid", "b-1")],
    );
    let top = top_face(&doc, o);
    doc.apply_object_op(
        o,
        KernelOp::PushPull {
            face: top,
            distance: 0.5,
        },
    )
    .unwrap();
    let attrs = entity_dict(&doc, EntityRef::Object(o));

    let (def, _, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();

    assert_eq!(doc.def_members(def).unwrap(), vec![o], "same object id");
    assert_eq!(doc.object_owner_component(o), Some(def));
    assert_eq!(doc.object_name(o), Some("Leg"));
    assert_eq!(
        doc.node_tags(NodeId::Object(o)),
        [tag(&["Structure", "Leg"])]
    );
    assert_eq!(entity_dict(&doc, EntityRef::Object(o)), attrs);
    // The history came along: after the wrap is undone, the push/pull
    // before it still undoes on the same object.
    doc.undo().unwrap();
    assert!(approx(max_z(&doc, o), 1.5));
    doc.undo().unwrap();
    assert!(approx(max_z(&doc, o), 1.0));
}

#[test]
fn wrap_linked_makes_a_definition_named_after_the_node_carrying_hew_linked() {
    let mut doc = Document::new();
    let o = named_board(&mut doc, 0.0, "Shelf", &[], &[]);
    let (def, _, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    assert_eq!(doc.component_name(def), Some("Shelf"));
    assert!(doc.is_linked_set(def));
    let dict = entity_dict(&doc, EntityRef::Component(def)).expect("a dictionary");
    assert_eq!(
        dict.get(kernel::HEW_ATTR_NS)
            .and_then(|ns| ns.get(kernel::LINKED_ATTR_KEY)),
        Some(&AttrValue::Bool(true))
    );

    let nameless = board(&mut doc, 3.0);
    let (def2, _, _) = doc.wrap_linked(NodeId::Object(nameless)).unwrap();
    assert_eq!(
        doc.component_name(def2),
        None,
        "a nameless node, a nameless set"
    );
    assert!(doc.is_linked_set(def2));
}

#[test]
fn wrap_linked_leaves_geometry_untouched_and_the_instance_at_identity() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 2.0, 3.0, 4.0, 5.0, 1.5);
    let before = polys(&doc, o);
    let (_, instance, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    assert_eq!(polys(&doc, o), before, "no vertex moved");
    assert_eq!(doc.instance_pose(instance), Some(Transform::IDENTITY));
    assert_eq!(placed_min_x(&doc, o), vec![2.0]);
}

#[test]
fn wrap_linked_splices_the_instance_into_the_old_parent_at_the_old_position() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let c = board(&mut doc, 4.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)])
        .unwrap();
    let (_, instance, _) = doc.wrap_linked(NodeId::Object(b)).unwrap();
    assert_eq!(
        doc.group_members(g).unwrap(),
        vec![
            NodeId::Object(a),
            NodeId::Instance(instance),
            NodeId::Object(c)
        ]
    );
    assert_eq!(doc.node_parent(NodeId::Instance(instance)), Some(g));
}

#[test]
fn wrap_linked_instance_takes_the_nodes_tags_and_no_name_or_material() {
    let mut doc = Document::new();
    let m = doc.add_material(Material::solid("Oak", Rgba8::rgb(150, 100, 50)));
    let o = named_board(&mut doc, 0.0, "Leg", &[&["Legs"], &["Oak"]], &[]);
    doc.set_object_material(o, Some(m)).unwrap();
    let (_, instance, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    assert_eq!(
        doc.node_tags(NodeId::Instance(instance)),
        [tag(&["Legs"]), tag(&["Oak"])]
    );
    assert_eq!(doc.instance_name(instance), None);
    assert_eq!(doc.instance_material(instance), Some(None));
    assert_eq!(
        entity_dict(&doc, EntityRef::Instance(instance)),
        None,
        "an empty dictionary"
    );
    assert_eq!(
        doc.node_tags(NodeId::Object(o)),
        [tag(&["Legs"]), tag(&["Oak"])],
        "the node keeps its own copy"
    );
}

#[test]
fn wrap_linked_a_group_keeps_its_whole_subtree_and_nothing_is_flattened() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (inner, _) = doc.group_nodes(&[NodeId::Object(b)]).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Group(inner)])
        .unwrap();
    doc.set_node_name(NodeId::Group(g), Some("Blank".into()))
        .unwrap();
    let (def, _, _) = doc.wrap_linked(NodeId::Group(g)).unwrap();
    assert_eq!(doc.def_member_nodes(def).unwrap(), vec![NodeId::Group(g)]);
    assert_eq!(
        doc.group_members(g).unwrap(),
        vec![NodeId::Object(a), NodeId::Group(inner)]
    );
    assert_eq!(doc.group_members(inner).unwrap(), vec![NodeId::Object(b)]);
    assert_eq!(doc.object_owner_component(a), Some(def));
    assert_eq!(doc.object_owner_component(b), Some(def));
    assert_eq!(doc.group_name(g), Some("Blank"));
    assert_eq!(doc.component_name(def), Some("Blank"));
    assert_eq!(doc.owner_definition(NodeId::Group(inner)), Some(def));
}

#[test]
fn wrap_linked_refuses_an_instance_not_wrappable_and_stale_nodes_unknown() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (_, i, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let gone = board(&mut doc, 3.0);
    doc.delete_node(NodeId::Object(gone)).unwrap();
    let x = board(&mut doc, 6.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(x)]).unwrap();
    doc.delete_node(NodeId::Group(g)).unwrap();
    let hash = doc.state_hash();
    let generation = doc.history_generation();

    assert_eq!(
        doc.wrap_linked(NodeId::Instance(i)).unwrap_err(),
        DocumentError::NotWrappable
    );
    assert_eq!(
        doc.wrap_linked(NodeId::Object(gone)).unwrap_err(),
        DocumentError::UnknownObject
    );
    assert_eq!(
        doc.wrap_linked(NodeId::Group(g)).unwrap_err(),
        DocumentError::UnknownGroup
    );
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.history_generation(), generation);
}

#[test]
fn wrap_linked_a_definition_owned_node_makes_a_definition_owned_placement() {
    // L13: a group holding two placements, inside an outer definition.
    let mut doc = Document::new();
    let p = board(&mut doc, 0.0);
    let (def_p, i1, _) = doc.make_component(&[NodeId::Object(p)]).unwrap();
    let (i2, _) = doc.place_instance(def_p, shift(3.0)).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(i1), NodeId::Instance(i2)])
        .unwrap();
    let extra = board(&mut doc, 10.0);
    let (outer, o1, _) = doc
        .make_component(&[NodeId::Group(g), NodeId::Object(extra)])
        .unwrap();
    let before = placed_min_x(&doc, p);
    let hash = doc.state_hash();

    let (def, instance, change) = doc.wrap_linked(NodeId::Group(g)).unwrap();

    assert_eq!(
        doc.owner_definition(NodeId::Instance(instance)),
        Some(outer)
    );
    assert_eq!(doc.owner_definition(NodeId::Group(g)), Some(def));
    assert_eq!(doc.owner_definition(NodeId::Instance(i1)), Some(def));
    assert_eq!(
        doc.def_member_nodes(outer).unwrap(),
        vec![NodeId::Instance(instance), NodeId::Object(extra)],
        "the placement takes the group's spot"
    );
    assert!(doc.is_linked_set(def));
    assert_eq!(placed_min_x(&doc, p), before, "nothing moved");
    assert!(change.instances_touched.contains(&o1));
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn wrap_linked_leaves_hew_part_frame_on_the_wrapped_node() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let frame = AttrValue::Text("x:1,0,0;y:0,1,0".into());
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(o)),
        "hew.part",
        "frame",
        frame.clone(),
    )
    .unwrap();
    let (def, instance, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let dict = entity_dict(&doc, EntityRef::Object(o)).unwrap();
    assert_eq!(
        dict.get("hew.part").and_then(|ns| ns.get("frame")),
        Some(&frame)
    );
    assert!(entity_dict(&doc, EntityRef::Instance(instance)).is_none());
    let def_dict = entity_dict(&doc, EntityRef::Component(def)).unwrap();
    assert!(!def_dict.contains_key("hew.part"));
}

#[test]
fn wrap_linked_detaches_annotations_anchored_to_the_node() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let note = doc
        .add_leader_text(
            Anchor {
                node: Some(NodeId::Object(o)),
                point: Point3::new(0.5, 0.5, 1.0),
            },
            Vec3::new(0.0, 0.0, 1.0),
            "Leg".into(),
        )
        .unwrap();
    assert_eq!(doc.annotation_detached(note), Some(false));
    doc.wrap_linked(NodeId::Object(o)).unwrap();
    assert_eq!(doc.annotation_detached(note), Some(true));
    doc.undo().unwrap();
    assert_eq!(doc.annotation_detached(note), Some(false));
}

#[test]
fn undo_of_a_wrap_restores_the_loose_node_exactly() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let o = named_board(&mut doc, 2.0, "Leg", &[&["Legs"]], &[("uuid", "b-1")]);
    let c = board(&mut doc, 4.0);
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(o)),
        "hew.part",
        "frame",
        AttrValue::Text("f".into()),
    )
    .unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(o), NodeId::Object(c)])
        .unwrap();
    let members = doc.group_members(g).unwrap();
    let attrs = entity_dict(&doc, EntityRef::Object(o));
    let sketches = doc.object_owned_sketches(o);
    let hash = doc.state_hash();

    let (def, instance, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    doc.undo().unwrap();

    assert_eq!(doc.state_hash(), hash);
    assert!(doc.is_world_object(o));
    assert_eq!(doc.object_owner_component(o), None);
    assert_eq!(doc.object_name(o), Some("Leg"));
    assert_eq!(doc.node_tags(NodeId::Object(o)), [tag(&["Legs"])]);
    assert_eq!(entity_dict(&doc, EntityRef::Object(o)), attrs);
    assert_eq!(doc.group_members(g).unwrap(), members);
    assert_eq!(doc.node_parent(NodeId::Object(o)), Some(g));
    assert_eq!(doc.object_owned_sketches(o), sketches);
    assert_eq!(doc.instance_def(instance), None, "the placement is gone");
    assert!(!doc.component_ids().contains(&def));
}

#[test]
fn redo_of_a_wrap_reproduces_the_post_state_hash() {
    let mut doc = Document::new();
    let o = named_board(&mut doc, 0.0, "Leg", &[&["Legs"]], &[]);
    let (g, _) = doc.group_nodes(&[NodeId::Object(o)]).unwrap();
    let _ = g;
    doc.wrap_linked(NodeId::Object(o)).unwrap();
    let post = doc.state_hash();
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post);
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post, "stable across cycles");
}

#[test]
fn wrap_linked_reports_the_wrap_in_doc_change_linked() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (def, instance, change) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    assert_eq!(
        change.linked,
        vec![LinkedWrap {
            source: NodeId::Object(o),
            instance,
            definition: def,
        }]
    );
    assert!(change.instances_touched.contains(&instance));
    assert!(change.components_touched.contains(&def));
    assert!(change.objects_touched.contains(&o));
    let undo = doc.undo().unwrap();
    assert!(undo.linked.is_empty(), "an undo wraps nothing");
}

// ---------------------------------------------------------------- copies

#[test]
fn linked_copy_wraps_a_loose_object_and_returns_the_new_placements() {
    // L1.
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (root, change) = doc
        .duplicate_node(NodeId::Object(o), &shift(2.0), Linked)
        .unwrap();
    let copy = ins(root);
    assert_eq!(change.linked.len(), 1);
    let wrap = change.linked[0];
    assert_eq!(wrap.source, NodeId::Object(o));
    assert_eq!(doc.instance_def(copy), Some(wrap.definition));
    assert_eq!(doc.instance_def(wrap.instance), Some(wrap.definition));
    assert_eq!(doc.def_members(wrap.definition).unwrap(), vec![o]);
    assert_eq!(doc.instances_of(wrap.definition).len(), 2);
    assert_eq!(placed_min_x(&doc, o), vec![0.0, 2.0]);
    assert_eq!(doc.live_object_ids().len(), 1, "no geometry was copied");
}

#[test]
fn linked_copy_poses_compose_the_step_k_times_onto_the_identity() {
    // L3: a grid of six placements.
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let grid: Vec<Transform> = (0..6)
        .map(|k| {
            Transform::translation(Vec3::new(2.0 * (k % 3) as f64, 2.0 * (k / 3) as f64, 0.0))
                .then(&shift(0.0))
        })
        .collect();
    let (roots, change) = doc
        .duplicate_nodes_placements(&[NodeId::Object(o)], &grid, Linked)
        .unwrap();
    assert_eq!(roots.len(), 6);
    for (r, p) in roots.iter().zip(&grid) {
        assert_eq!(
            doc.instance_pose(ins(*r)),
            Some(Transform::IDENTITY.then(p))
        );
        assert_eq!(doc.instance_def(ins(*r)), Some(change.linked[0].definition));
    }
    // The array form composes `step^k` exactly as the independent array.
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let step = Transform::translation(Vec3::new(0.3, 0.1, 0.0));
    let (roots, _) = doc
        .duplicate_nodes_array(
            &[NodeId::Object(o)],
            &step,
            NonZeroU32::new(3).unwrap(),
            Linked,
        )
        .unwrap();
    let mut want = step;
    for (k, r) in roots.iter().enumerate() {
        if k > 0 {
            want = want.then(&step);
        }
        assert_eq!(
            doc.instance_pose(ins(*r)),
            Some(Transform::IDENTITY.then(&want))
        );
    }
}

#[test]
fn linked_array_of_n_makes_one_set_of_n_plus_one_placements() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (roots, change) = doc
        .duplicate_nodes_array(
            &[NodeId::Object(o)],
            &shift(2.0),
            NonZeroU32::new(4).unwrap(),
            Linked,
        )
        .unwrap();
    assert_eq!(roots.len(), 4);
    let def = change.linked[0].definition;
    assert_eq!(doc.instances_of(def).len(), 5);
    assert_eq!(placed_min_x(&doc, o), vec![0.0, 2.0, 4.0, 6.0, 8.0]);
}

#[test]
fn linked_copy_by_rotation_places_rotated_instances_and_bakes_nothing() {
    // L4.
    let mut doc = Document::new();
    let o = board(&mut doc, 2.0);
    let before = polys(&doc, o);
    let rot = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2).unwrap();
    let (roots, _) = doc
        .duplicate_nodes_array(
            &[NodeId::Object(o)],
            &rot,
            NonZeroU32::new(3).unwrap(),
            Linked,
        )
        .unwrap();
    assert_eq!(polys(&doc, o), before, "the shared member never bakes");
    let mut want = rot;
    for (k, r) in roots.iter().enumerate() {
        if k > 0 {
            want = want.then(&rot);
        }
        assert_eq!(
            doc.instance_pose(ins(*r)),
            Some(Transform::IDENTITY.then(&want))
        );
    }
}

#[test]
fn a_group_copy_wraps_as_one_set() {
    // L6.
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let (root, change) = doc
        .duplicate_node(NodeId::Group(g), &shift(5.0), Linked)
        .unwrap();
    assert_eq!(change.linked.len(), 1);
    let def = change.linked[0].definition;
    assert_eq!(doc.def_member_nodes(def).unwrap(), vec![NodeId::Group(g)]);
    assert_eq!(doc.instance_def(ins(root)), Some(def));
    assert_eq!(placed_min_x(&doc, a), vec![0.0, 5.0]);
    assert_eq!(placed_min_x(&doc, b), vec![2.0, 7.0]);
}

#[test]
fn several_objects_wrap_separately_four_boards_make_four_sets() {
    let mut doc = Document::new();
    let boards: Vec<ObjectId> = (0..4).map(|k| board(&mut doc, 2.0 * k as f64)).collect();
    let nodes: Vec<NodeId> = boards.iter().map(|&o| NodeId::Object(o)).collect();
    let (roots, change) = doc
        .duplicate_nodes_array(
            &nodes,
            &shift(0.0).then(&Transform::translation(Vec3::new(0.0, 3.0, 0.0))),
            NonZeroU32::new(1).unwrap(),
            Linked,
        )
        .unwrap();
    assert_eq!(change.linked.len(), 4);
    let defs: BTreeSet<ComponentId> = change.linked.iter().map(|w| w.definition).collect();
    assert_eq!(defs.len(), 4, "four linked sets, not one set of four");
    for (k, w) in change.linked.iter().enumerate() {
        assert_eq!(w.source, nodes[k], "wraps in node order");
        assert_eq!(doc.def_members(w.definition).unwrap(), vec![boards[k]]);
        assert_eq!(doc.instances_of(w.definition).len(), 2);
        assert_eq!(doc.instance_def(ins(roots[k])), Some(w.definition));
    }
}

#[test]
fn an_instance_root_copies_as_today_with_the_same_definition() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (def, i, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let defs = doc.component_ids().len();
    let (root, change) = doc
        .duplicate_node(NodeId::Instance(i), &shift(2.0), Linked)
        .unwrap();
    assert!(change.linked.is_empty());
    assert_eq!(doc.instance_def(ins(root)), Some(def));
    assert_eq!(
        doc.instance_pose(ins(root)),
        Some(Transform::IDENTITY.then(&shift(2.0)))
    );
    assert_eq!(doc.component_ids().len(), defs);
    assert!(!doc.is_linked_set(def), "a named component stays one");
}

/// The fixed scenario [`independent_policy_is_todays_behavior_exactly`]
/// runs; its hash was recorded from the build before linked copies.
fn independent_scenario(doc: &mut Document) -> Vec<NodeId> {
    let o = named_board(doc, 0.0, "Leg", &[&["Legs"]], &[("uuid", "b-1")]);
    let a = board(doc, 3.0);
    let b = board(doc, 5.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let c = board(doc, 8.0);
    let (_, i, _) = doc.make_component(&[NodeId::Object(c)]).unwrap();
    let step = Transform::translation(Vec3::new(0.0, 2.0, 0.0));
    let (roots, _) = doc
        .duplicate_nodes_array(
            &[NodeId::Object(o), NodeId::Group(g), NodeId::Instance(i)],
            &step,
            NonZeroU32::new(2).unwrap(),
            Independent,
        )
        .unwrap();
    roots
}

/// `state_hash` of [`independent_scenario`] on the build before linked
/// copies existed.
const PRE_STAGE_INDEPENDENT_HASH: u64 = 11_898_056_068_770_004_186;

#[test]
fn independent_policy_is_todays_behavior_exactly() {
    let mut doc = Document::new();
    let roots = independent_scenario(&mut doc);
    let kinds: Vec<&str> = roots
        .iter()
        .map(|r| match r {
            NodeId::Object(_) => "object",
            NodeId::Group(_) => "group",
            NodeId::Instance(_) => "instance",
        })
        .collect();
    assert_eq!(
        kinds,
        ["object", "group", "instance", "object", "group", "instance"],
        "baked copies of the same kind, placement-major"
    );
    assert_eq!(doc.component_ids().len(), 1, "no definition was created");
    assert_eq!(doc.state_hash(), PRE_STAGE_INDEPENDENT_HASH);
}

#[test]
fn a_linked_copy_is_one_undo_entry_with_the_array_and_keeps_the_label() {
    // L2.
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let generation = doc.history_generation();
    let depth = doc.undo_depth();
    doc.duplicate_nodes_array(
        &[NodeId::Object(o)],
        &shift(2.0),
        NonZeroU32::new(3).unwrap(),
        Linked,
    )
    .unwrap();
    assert_eq!(doc.history_generation(), generation + 1);
    assert_eq!(doc.undo_depth(), depth + 1);
    assert_eq!(last_label(&doc), "Copy ×3");

    let p = board(&mut doc, 10.0);
    doc.duplicate_node(NodeId::Object(p), &shift(2.0), Linked)
        .unwrap();
    assert_eq!(last_label(&doc), "Copy");
}

#[test]
fn undo_of_a_linked_copy_restores_the_loose_object_exactly() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let o = named_board(&mut doc, 2.0, "Leg", &[&["Legs"]], &[("uuid", "b-1")]);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(o)])
        .unwrap();
    let members = doc.group_members(g).unwrap();
    let attrs = entity_dict(&doc, EntityRef::Object(o));
    let hash = doc.state_hash();
    let defs = doc.component_ids();

    doc.duplicate_nodes_array(
        &[NodeId::Object(o)],
        &shift(3.0),
        NonZeroU32::new(2).unwrap(),
        Linked,
    )
    .unwrap();
    doc.undo().unwrap();

    assert_eq!(doc.state_hash(), hash);
    assert!(doc.is_world_object(o));
    assert_eq!(doc.object_name(o), Some("Leg"));
    assert_eq!(doc.node_tags(NodeId::Object(o)), [tag(&["Legs"])]);
    assert_eq!(entity_dict(&doc, EntityRef::Object(o)), attrs);
    assert_eq!(doc.group_members(g).unwrap(), members);
    assert_eq!(doc.component_ids(), defs);
}

#[test]
fn redo_of_a_linked_copy_reproduces_the_post_state_hash() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(o)]).unwrap();
    let b = board(&mut doc, 5.0);
    doc.duplicate_nodes_array(
        &[NodeId::Group(g), NodeId::Object(b)],
        &shift(2.0),
        NonZeroU32::new(2).unwrap(),
        Linked,
    )
    .unwrap();
    let post = doc.state_hash();
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post);
}

/// A doubling ladder of nested definitions to `2^19` expanded placements
/// (just over half the bound); returns its world instance.
fn half_full(doc: &mut Document) -> InstanceId {
    let a = extrude_box(doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    for _ in 0..19 {
        let (second, _) = doc.place_instance(def, shift(2.0)).unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    inst
}

#[test]
fn a_failing_linked_copy_is_atomic() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let gone = board(&mut doc, 3.0);
    doc.delete_node(NodeId::Object(gone)).unwrap();
    let hash = doc.state_hash();
    let generation = doc.history_generation();
    let singular = Transform::scale(Vec3::new(0.0, 1.0, 1.0));
    assert!(matches!(
        doc.duplicate_node(NodeId::Object(o), &singular, Linked),
        Err(DocumentError::Transform(_))
    ));
    assert_eq!(
        doc.duplicate_nodes_array(
            &[NodeId::Object(o), NodeId::Object(gone)],
            &shift(2.0),
            NonZeroU32::new(1).unwrap(),
            Linked
        )
        .unwrap_err(),
        DocumentError::UnknownObject
    );
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.history_generation(), generation);

    // The expansion cap: the wrap of `b` alone fits, copying the ladder too
    // does not — and `b` is not left wrapped.
    let mut doc = Document::new();
    let inst = half_full(&mut doc);
    let b = board(&mut doc, 50.0);
    let hash = doc.state_hash();
    let generation = doc.history_generation();
    assert_eq!(
        doc.duplicate_nodes_array(
            &[NodeId::Object(b), NodeId::Instance(inst)],
            &shift(100.0),
            NonZeroU32::new(1).unwrap(),
            Linked
        )
        .unwrap_err(),
        DocumentError::ComponentExpansionExceeded
    );
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.history_generation(), generation);
    assert!(doc.is_world_object(b));
}

#[test]
fn a_reflecting_step_makes_a_mirrored_placement_and_bakes_nothing() {
    let mut doc = Document::new();
    let o = board(&mut doc, 2.0);
    let before = polys(&doc, o);
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0));
    let (root, _) = doc
        .duplicate_node(NodeId::Object(o), &mirror, Linked)
        .unwrap();
    let pose = doc.instance_pose(ins(root)).unwrap();
    assert!(pose.determinant() < 0.0, "a mirrored placement");
    assert_eq!(
        polys(&doc, o),
        before,
        "the shared member's vertices are unchanged"
    );
    assert_eq!(placed_min_x(&doc, o), vec![-3.0, 2.0]);
}

#[test]
fn a_node_listed_with_its_ancestor_refuses_duplicate_member_when_linked() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let hash = doc.state_hash();
    let nodes = [NodeId::Group(g), NodeId::Object(a)];
    assert_eq!(
        doc.duplicate_nodes_array(&nodes, &shift(2.0), NonZeroU32::new(1).unwrap(), Linked)
            .unwrap_err(),
        DocumentError::DuplicateMember
    );
    assert_eq!(doc.state_hash(), hash);
    // Independent keeps today's behavior: both are cloned.
    let (roots, _) = doc
        .duplicate_nodes_array(
            &nodes,
            &shift(2.0),
            NonZeroU32::new(1).unwrap(),
            Independent,
        )
        .unwrap();
    assert_eq!(roots.len(), 2);
}

#[test]
fn a_linked_copy_inside_a_component_session_folds_back_as_a_nested_pair() {
    let mut doc = Document::new();
    let m = board(&mut doc, 0.0);
    let (c, ci, _) = doc.make_component(&[NodeId::Object(m)]).unwrap();
    doc.transform_instance(ci, &shift(10.0)).unwrap();
    let (c2, _) = doc.place_instance(c, shift(20.0)).unwrap();
    let before_open = doc.state_hash();

    doc.open_explode_session(ci).unwrap();
    let (root, change) = doc
        .duplicate_node(NodeId::Object(m), &shift(2.0), Linked)
        .unwrap();
    let wrap = change.linked[0];
    doc.close_explode_session().unwrap();

    let members = doc.def_member_nodes(c).unwrap();
    assert!(
        !members.contains(&NodeId::Object(m)),
        "m left the definition"
    );
    assert!(members.contains(&NodeId::Instance(wrap.instance)));
    assert!(members.contains(&root));
    assert_eq!(
        doc.owner_definition(NodeId::Instance(wrap.instance)),
        Some(c)
    );
    assert_eq!(doc.owner_definition(root), Some(c));
    assert_eq!(doc.object_owner_component(m), Some(wrap.definition));
    assert!(doc.is_linked_set(wrap.definition));
    // Every placement of the outer definition shows both copies, where the
    // session put them.
    assert_eq!(placed_min_x(&doc, m), vec![10.0, 12.0, 20.0, 22.0]);
    assert!(doc.instance_def(c2).is_some());

    // Undo the close, the copy, and the open: back to the start.
    doc.undo().unwrap();
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before_open);
    // And forward again.
    doc.redo().unwrap();
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(placed_min_x(&doc, m), vec![10.0, 12.0, 20.0, 22.0]);
}

#[test]
fn a_linked_copy_inside_a_group_session_folds_the_placement_into_the_group() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let before_open = doc.state_hash();
    doc.open_group_session(g).unwrap();
    let (root, change) = doc
        .duplicate_node(NodeId::Object(a), &shift(5.0), Linked)
        .unwrap();
    let wrap = change.linked[0];
    doc.close_group_session().unwrap();

    let members = doc.group_members(g).unwrap();
    assert_eq!(members.len(), 3);
    assert!(members.contains(&NodeId::Object(b)));
    assert!(members.contains(&NodeId::Instance(wrap.instance)));
    assert!(members.contains(&root));
    assert_eq!(doc.node_parent(root), Some(g));
    assert_eq!(doc.object_owner_component(a), Some(wrap.definition));
    assert_eq!(placed_min_x(&doc, a), vec![0.0, 5.0]);

    doc.undo().unwrap();
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before_open);
}

#[test]
fn a_linked_copy_of_a_definition_member_by_wrap_stays_inside_its_definition() {
    let mut doc = Document::new();
    let x = board(&mut doc, 0.0);
    let y = board(&mut doc, 2.0);
    let (d, di, _) = doc
        .make_component(&[NodeId::Object(x), NodeId::Object(y)])
        .unwrap();
    let (d2, _) = doc.place_instance(d, shift(10.0)).unwrap();
    let (def, instance, _) = doc.wrap_linked(NodeId::Object(x)).unwrap();
    assert_eq!(doc.owner_definition(NodeId::Instance(instance)), Some(d));
    assert_eq!(
        doc.def_member_nodes(d).unwrap(),
        vec![NodeId::Instance(instance), NodeId::Object(y)]
    );
    assert_eq!(doc.object_owner_component(x), Some(def));
    assert_eq!(placed_min_x(&doc, x), vec![0.0, 10.0]);
    assert!(doc.instance_def(di).is_some() && doc.instance_def(d2).is_some());
}

// ----------------------------------------------- delete, explode, unlink

/// A board wrapped as a linked set with `copies` more placements.
fn linked_set(
    doc: &mut Document,
    copies: u32,
) -> (ObjectId, ComponentId, InstanceId, Vec<InstanceId>) {
    let o = board(doc, 0.0);
    let (def, first, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let mut rest = Vec::new();
    if copies > 0 {
        let (roots, _) = doc
            .duplicate_nodes_array(
                &[NodeId::Instance(first)],
                &shift(2.0),
                NonZeroU32::new(copies).unwrap(),
                Linked,
            )
            .unwrap();
        rest = roots.into_iter().map(ins).collect();
    }
    (o, def, first, rest)
}

#[test]
fn deleting_the_last_placement_of_a_linked_set_deletes_the_definition() {
    let mut doc = Document::new();
    let (o, def, first, _) = linked_set(&mut doc, 0);
    let change = doc.delete_node(NodeId::Instance(first)).unwrap();
    assert!(!doc.component_ids().contains(&def));
    assert!(!doc.is_linked_set(def));
    assert!(doc.object(o).is_none(), "its member went with it");
    assert!(change.components_touched.contains(&def));
    // The document still saves and reloads.
    let re = Document::load(&doc.save()).unwrap();
    assert!(re.component_ids().is_empty());
}

#[test]
fn deleting_one_of_several_placements_keeps_the_definition() {
    let mut doc = Document::new();
    let (_, def, first, rest) = linked_set(&mut doc, 2);
    doc.delete_node(NodeId::Instance(rest[0])).unwrap();
    doc.delete_node(NodeId::Instance(first)).unwrap();
    assert!(doc.is_linked_set(def));
    assert_eq!(doc.instances_of(def), vec![rest[1]]);
}

#[test]
fn deleting_a_group_holding_the_last_placement_deletes_the_definition() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let other = board(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(o), NodeId::Object(other)])
        .unwrap();
    let (def, _, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    doc.delete_node(NodeId::Group(g)).unwrap();
    assert!(!doc.component_ids().contains(&def));
    assert!(doc.object(o).is_none());
}

#[test]
fn deleting_the_last_placement_of_a_named_component_keeps_it() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (def, i, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    doc.delete_node(NodeId::Instance(i)).unwrap();
    assert!(
        doc.component_ids().contains(&def),
        "a named component stays available"
    );
    assert!(doc.object(o).is_some());
}

#[test]
fn undo_of_deleting_the_last_placement_restores_definition_and_placement() {
    let mut doc = Document::new();
    let (o, def, first, _) = linked_set(&mut doc, 0);
    let hash = doc.state_hash();
    doc.delete_node(NodeId::Instance(first)).unwrap();
    let post = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert!(doc.is_linked_set(def));
    assert_eq!(doc.instance_def(first), Some(def));
    assert!(doc.object(o).is_some());
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post);
    // The selection-delete path takes it as one entry too.
    doc.undo().unwrap();
    doc.delete_selection(&[NodeId::Instance(first)]).unwrap();
    assert!(!doc.component_ids().contains(&def));
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn exploding_the_last_placement_of_a_linked_set_hides_the_definition() {
    let mut doc = Document::new();
    let (o, def, first, _) = linked_set(&mut doc, 0);
    let hash = doc.state_hash();
    let (created, _) = doc.explode_instance(first).unwrap();
    assert_eq!(created.len(), 1);
    assert!(doc.is_world_object(created[0]));
    assert!(!doc.component_ids().contains(&def));
    assert!(doc.object(o).is_none());
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert!(doc.is_linked_set(def));
    // With another placement left, the set stays.
    let (_, _, a, rest) = {
        let mut d2 = Document::new();
        let r = linked_set(&mut d2, 1);
        doc = d2;
        r
    };
    let def2 = doc.instance_def(a).unwrap();
    doc.explode_instance(a).unwrap();
    assert!(doc.is_linked_set(def2));
    assert_eq!(doc.instances_of(def2), rest);
}

#[test]
fn unlinking_the_last_placement_hides_the_old_definition() {
    let mut doc = Document::new();
    let (o, def, first, _) = linked_set(&mut doc, 0);
    let (new_def, leaf, remap, change) = doc.make_unique_by_path(&[first], true, false).unwrap();
    assert_eq!(leaf, first);
    assert!(
        !doc.component_ids().contains(&def),
        "the old set went with its last placement"
    );
    assert!(doc.object(o).is_none());
    assert!(doc.is_linked_set(new_def));
    assert_eq!(remap.components.get(&def), Some(&new_def));
    assert!(change.components_touched.contains(&def));
    assert!(Document::load(&doc.save()).is_ok());
}

#[test]
fn a_set_reduced_to_one_placement_stays_a_set() {
    // L9: unlink all but one.
    let mut doc = Document::new();
    let (o, def, first, rest) = linked_set(&mut doc, 2);
    for &c in &rest {
        let (forked, _, _, _) = doc.make_unique_by_path(&[c], true, false).unwrap();
        assert!(
            doc.is_linked_set(forked),
            "each unlinked copy is a set of one"
        );
    }
    assert!(doc.is_linked_set(def));
    assert_eq!(doc.instances_of(def), vec![first]);
    assert_eq!(doc.def_members(def).unwrap(), vec![o], "the row survives");
    assert!(doc.object(o).is_some());
}

#[test]
fn undo_of_an_unlink_restores_the_set() {
    let mut doc = Document::new();
    let (_, def, first, rest) = linked_set(&mut doc, 1);
    let hash = doc.state_hash();
    doc.make_unique_by_path(&[rest[0]], true, false).unwrap();
    doc.make_unique_by_path(&[first], true, false).unwrap();
    assert!(!doc.component_ids().contains(&def));
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.instances_of(def).len(), 2);
}

// ------------------------------------------------ nested linked sets

/// No live linked set is left with zero live placements (decision 7,
/// applied transitively). Only meaningful with no session open: a
/// definition on loan has its entered instance hidden by the bake.
fn assert_no_orphaned_linked_set(doc: &Document) {
    for c in doc.component_ids() {
        if doc.is_linked_set(c) {
            assert!(
                doc.definition_usage(c) > 0,
                "linked set {c:?} is live with no live placement"
            );
        }
    }
}

/// Repro A's shape: a linked set `SA` with placements `a1`, `a2`, grouped
/// as `G`, and `G` copied linked, so `G` is the member of a second set
/// `SG` with placements `g1` (the wrap) and `g2` (the copy).
struct Nested {
    o: ObjectId,
    sa: ComponentId,
    a: [InstanceId; 2],
    g: GroupId,
    sg: ComponentId,
    gp: [InstanceId; 2],
}

fn nested_sets(doc: &mut Document) -> Nested {
    let o = board(doc, 0.0);
    let (sa, a1, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (a2, _) = doc
        .duplicate_node(NodeId::Instance(a1), &shift(2.0), Linked)
        .unwrap();
    let a2 = ins(a2);
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(a1), NodeId::Instance(a2)])
        .unwrap();
    let (g2, change) = doc
        .duplicate_node(NodeId::Group(g), &shift(5.0), Linked)
        .unwrap();
    let wrap = change.linked[0];
    assert_eq!(wrap.source, NodeId::Group(g));
    assert_eq!(
        doc.owner_definition(NodeId::Instance(a1)),
        Some(wrap.definition)
    );
    assert_eq!(
        doc.owner_definition(NodeId::Instance(a2)),
        Some(wrap.definition)
    );
    Nested {
        o,
        sa,
        a: [a1, a2],
        g,
        sg: wrap.definition,
        gp: [wrap.instance, ins(g2)],
    }
}

#[test]
fn deleting_the_last_placements_of_an_outer_set_takes_the_inner_set_with_it() {
    // Repro A.
    let mut doc = Document::new();
    let n = nested_sets(&mut doc);
    doc.delete_node(NodeId::Instance(n.gp[0])).unwrap();
    assert!(doc.is_linked_set(n.sg) && doc.is_linked_set(n.sa));
    doc.delete_node(NodeId::Instance(n.gp[1])).unwrap();
    assert!(!doc.component_ids().contains(&n.sg));
    assert!(
        !doc.component_ids().contains(&n.sa),
        "the inner set lost its last placements with the outer one"
    );
    assert!(!doc.is_linked_set(n.sa));
    assert!(doc.object(n.o).is_none());
    assert_no_orphaned_linked_set(&doc);
}

#[test]
fn undo_and_redo_of_the_nested_delete_restore_and_rehide_both_definitions() {
    let mut doc = Document::new();
    let n = nested_sets(&mut doc);
    doc.delete_node(NodeId::Instance(n.gp[0])).unwrap();
    let mid = doc.state_hash();
    doc.delete_node(NodeId::Instance(n.gp[1])).unwrap();
    let post = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(
        doc.state_hash(),
        mid,
        "undo reveals both sets and every row"
    );
    assert!(doc.is_linked_set(n.sg) && doc.is_linked_set(n.sa));
    assert_eq!(doc.instance_def(n.a[0]), Some(n.sa));
    assert_eq!(doc.instance_def(n.a[1]), Some(n.sa));
    assert!(doc.object(n.o).is_some());
    assert_eq!(
        doc.def_member_nodes(n.sg).unwrap(),
        vec![NodeId::Group(n.g)]
    );
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post, "redo hides both again");
    assert_no_orphaned_linked_set(&doc);
}

#[test]
fn the_nested_delete_survives_save_and_load() {
    let mut doc = Document::new();
    let n = nested_sets(&mut doc);
    doc.delete_selection(&[NodeId::Instance(n.gp[0]), NodeId::Instance(n.gp[1])])
        .unwrap();
    assert!(!doc.component_ids().contains(&n.sa));
    let re = Document::load(&doc.save()).unwrap();
    assert!(re.component_ids().is_empty(), "neither set is saved");
    assert_no_orphaned_linked_set(&re);
    // Undone, both sets and their placements save and reload.
    doc.undo().unwrap();
    let re = Document::load(&doc.save()).unwrap();
    assert_eq!(re.component_ids().len(), 2);
    assert_no_orphaned_linked_set(&re);
}

#[test]
fn the_nested_delete_change_lists_every_member_hidden_with_the_sets() {
    // S3: per-node caches of the hidden members must hear of them.
    let mut doc = Document::new();
    let n = nested_sets(&mut doc);
    doc.delete_node(NodeId::Instance(n.gp[0])).unwrap();
    let check = |change: &kernel::DocChange, what: &str| {
        assert!(change.components_touched.contains(&n.sg), "{what}: SG");
        assert!(change.components_touched.contains(&n.sa), "{what}: SA");
        assert!(change.groups_touched.contains(&n.g), "{what}: G");
        assert!(change.objects_touched.contains(&n.o), "{what}: o");
        for a in n.a {
            assert!(change.instances_touched.contains(&a), "{what}: {a:?}");
        }
    };
    let change = doc.delete_node(NodeId::Instance(n.gp[1])).unwrap();
    check(&change, "delete");
    let change = doc.undo().unwrap();
    check(&change, "undo");
    let change = doc.redo().unwrap();
    check(&change, "redo");
}

#[test]
fn exploding_the_last_outer_placement_keeps_the_inner_set_placed_by_the_copies() {
    let mut doc = Document::new();
    let n = nested_sets(&mut doc);
    doc.delete_node(NodeId::Instance(n.gp[1])).unwrap();
    let hash = doc.state_hash();
    let (_, change) = doc.explode_instance(n.gp[0]).unwrap();
    assert!(!doc.component_ids().contains(&n.sg));
    // The explode copied a1 and a2 out as world placements of SA.
    assert!(doc.is_linked_set(n.sa));
    let placed = doc.instances_of(n.sa);
    assert_eq!(placed.len(), 2);
    assert!(!placed.contains(&n.a[0]) && !placed.contains(&n.a[1]));
    assert!(change.groups_touched.contains(&n.g));
    assert!(change.instances_touched.contains(&n.a[0]));
    assert_no_orphaned_linked_set(&doc);
    let post = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post);
}

#[test]
fn exploding_through_a_grouped_last_placement_hides_an_emptied_inner_set() {
    // An outer set whose only member is the inner set's only placement:
    // exploding the outer set's last placement bakes a copy of the inner
    // placement, so the inner set lives on.
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (sa, a1, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(a1)]).unwrap();
    let (sg, g1, _) = doc.wrap_linked(NodeId::Group(g)).unwrap();
    let hash = doc.state_hash();
    doc.explode_instance(g1).unwrap();
    assert!(!doc.component_ids().contains(&sg));
    assert!(doc.is_linked_set(sa));
    assert_eq!(doc.definition_usage(sa), 1);
    assert_no_orphaned_linked_set(&doc);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
}

/// Repro B's shape: an outer linked set `s` whose member group holds two
/// placements `a` of an inner linked set `sa`; `s` has one placement `p`
/// left (a second one was copied and deleted).
struct Outer {
    o: ObjectId,
    sa: ComponentId,
    a: [InstanceId; 2],
    s: ComponentId,
    p: InstanceId,
}

fn outer_holding_two_inner(doc: &mut Document) -> Outer {
    let o = board(doc, 0.0);
    let (sa, a1, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (a2, _) = doc
        .duplicate_node(NodeId::Instance(a1), &shift(2.0), Linked)
        .unwrap();
    let a2 = ins(a2);
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(a1), NodeId::Instance(a2)])
        .unwrap();
    let (s, p, _) = doc.wrap_linked(NodeId::Group(g)).unwrap();
    let (p2, _) = doc
        .duplicate_node(NodeId::Instance(p), &shift(5.0), Linked)
        .unwrap();
    doc.delete_node(p2).unwrap();
    assert_eq!(doc.definition_usage(s), 1);
    Outer {
        o,
        sa,
        a: [a1, a2],
        s,
        p,
    }
}

#[test]
fn a_deep_unlink_of_the_last_outer_placement_takes_the_inner_set_with_it() {
    // Repro B.
    let mut doc = Document::new();
    let n = outer_holding_two_inner(&mut doc);
    let (s, p) = (n.s, n.p);
    let hash = doc.state_hash();
    let (new_def, _, remap, change) = doc.make_unique_by_path(&[p], false, true).unwrap();
    assert!(
        !doc.component_ids().contains(&s),
        "the old outer set is gone"
    );
    assert!(
        !doc.component_ids().contains(&n.sa),
        "every copy of the inner placements moved to the private fork"
    );
    let inner_fork = *remap.components.get(&n.sa).expect("SA was forked");
    assert!(doc.is_linked_set(inner_fork));
    assert_eq!(doc.definition_usage(inner_fork), 2);
    assert!(doc.is_linked_set(new_def));
    assert!(doc.object(n.o).is_none());
    assert!(change.components_touched.contains(&n.sa));
    assert!(change.objects_touched.contains(&n.o));
    assert!(change.instances_touched.contains(&n.a[0]));
    assert_no_orphaned_linked_set(&doc);

    let post = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash, "undo reveals both definitions");
    assert!(doc.is_linked_set(s) && doc.is_linked_set(n.sa));
    assert_eq!(doc.instance_def(n.a[0]), Some(n.sa));
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post);
    assert_no_orphaned_linked_set(&doc);
    let re = Document::load(&doc.save()).unwrap();
    assert_eq!(re.component_ids().len(), 2, "the two forks");
    assert_no_orphaned_linked_set(&re);
}

#[test]
fn a_shallow_unlink_of_the_last_outer_placement_keeps_the_inner_set() {
    // The copies inside the fork still share SA, so SA stays.
    let mut doc = Document::new();
    let n = outer_holding_two_inner(&mut doc);
    doc.make_unique_by_path(&[n.p], false, false).unwrap();
    assert!(!doc.component_ids().contains(&n.s));
    assert!(doc.is_linked_set(n.sa));
    assert_eq!(doc.definition_usage(n.sa), 2);
    assert_no_orphaned_linked_set(&doc);
}

#[test]
fn deleting_a_definition_takes_a_linked_set_placed_only_inside_it() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (sa, a1, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (d, _, _) = doc.make_component(&[NodeId::Instance(a1)]).unwrap();
    let hash = doc.state_hash();
    let change = doc.delete_definition(d).unwrap();
    assert!(!doc.component_ids().contains(&sa));
    assert!(change.components_touched.contains(&sa));
    assert!(change.objects_touched.contains(&o));
    assert_no_orphaned_linked_set(&doc);
    let post = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert!(doc.is_linked_set(sa));
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), post);
    assert!(Document::load(&doc.save()).is_ok());
}

#[test]
fn purging_an_unused_definition_with_a_linked_set_inside_purges_both() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (sa, a1, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (d, di, _) = doc.make_component(&[NodeId::Instance(a1)]).unwrap();
    doc.delete_node(NodeId::Instance(di)).unwrap();
    // The named definition stays, and so does the set it places.
    assert!(doc.component_ids().contains(&d) && doc.is_linked_set(sa));
    let hash = doc.state_hash();
    let report = doc.purge_unused().unwrap();
    assert_eq!(report.definitions, 2);
    assert!(doc.component_ids().is_empty());
    assert_no_orphaned_linked_set(&doc);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
}

// ------------------------------------------- a wrap inside a session

#[test]
fn a_session_member_wrapped_mid_session_is_no_longer_session_geometry() {
    // S2: the scope reads agree with the close, which leaves a re-owned
    // member where the wrap put it.
    let mut doc = Document::new();
    let m = board(&mut doc, 0.0);
    let other = board(&mut doc, 3.0);
    let (_, ci, _) = doc
        .make_component(&[NodeId::Object(m), NodeId::Object(other)])
        .unwrap();
    doc.open_explode_session(ci).unwrap();
    let objects = doc.explode_session_objects().unwrap();
    assert!(objects.contains(&m) && objects.contains(&other));
    let (wrapped, _, _) = doc.wrap_linked(NodeId::Object(m)).unwrap();
    assert_eq!(doc.object_owner_component(m), Some(wrapped));
    let objects = doc.explode_session_objects().unwrap();
    assert!(!objects.contains(&m), "the wrapped member left the session");
    assert!(objects.contains(&other));
    // Session-scoped edits refuse it like any outside geometry.
    assert_eq!(
        doc.wrap_linked(NodeId::Object(m)).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    doc.close_explode_session().unwrap();
}

#[test]
fn a_wrap_or_linked_copy_refuses_while_component_frames_are_stacked() {
    // Found by the session fuzz: in a drill-down two frames walk the same
    // geometry, so a fold in the inner one changed what the siblings render.
    let mut doc = Document::new();
    let m = board(&mut doc, 0.0);
    let (_, inner, _) = doc.make_component(&[NodeId::Object(m)]).unwrap();
    let (_, outer, _) = doc.make_component(&[NodeId::Instance(inner)]).unwrap();
    doc.open_explode_session(outer).unwrap();
    doc.open_explode_session(inner).unwrap();
    let hash = doc.state_hash();
    assert_eq!(
        doc.wrap_linked(NodeId::Object(m)).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.duplicate_node(NodeId::Object(m), &shift(2.0), Linked)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(doc.state_hash(), hash);
    doc.close_explode_session().unwrap();
    doc.close_explode_session().unwrap();
}

// ---------------------------------------------------- attribute, promote

#[test]
fn hew_linked_survives_save_and_load_and_an_ordinary_definition_attribute_does_too() {
    // L14.
    let mut doc = Document::new();
    let o = named_board(&mut doc, 0.0, "Leg", &[], &[]);
    let (def, _, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let p = named_board(&mut doc, 5.0, "Top", &[], &[]);
    let (named, _, _) = doc.make_component(&[NodeId::Object(p)]).unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Component(named)),
        "com.example",
        "role",
        AttrValue::Text("top".into()),
    )
    .unwrap();
    let _ = def;
    let re = Document::load(&doc.save()).unwrap();
    let by_name = |d: &Document, n: &str| {
        d.component_ids()
            .into_iter()
            .find(|&c| d.component_name(c) == Some(n))
            .expect("the definition")
    };
    assert!(re.is_linked_set(by_name(&re, "Leg")));
    let top = by_name(&re, "Top");
    assert!(!re.is_linked_set(top));
    assert_eq!(
        entity_dict(&re, EntityRef::Component(top))
            .unwrap()
            .get("com.example")
            .and_then(|ns| ns.get("role")),
        Some(&AttrValue::Text("top".into()))
    );
}

#[test]
fn promote_linked_removes_the_attribute_in_one_undo_entry_and_keeps_placements() {
    let mut doc = Document::new();
    let (_, def, first, rest) = linked_set(&mut doc, 1);
    let depth = doc.undo_depth();
    doc.promote_linked(def).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    assert!(!doc.is_linked_set(def));
    assert_eq!(doc.instances_of(def).len(), 2);
    assert_eq!(doc.instance_def(first), Some(def));
    assert_eq!(doc.instance_def(rest[0]), Some(def));
    assert_eq!(
        doc.promote_linked(def).unwrap_err(),
        DocumentError::UnknownAttr
    );
    doc.undo().unwrap();
    assert!(doc.is_linked_set(def));
    // A promoted set is a named component: its last placement's delete
    // keeps it.
    doc.redo().unwrap();
    doc.delete_node(NodeId::Instance(first)).unwrap();
    doc.delete_node(NodeId::Instance(rest[0])).unwrap();
    assert!(doc.component_ids().contains(&def));
}

// ----------------------------------------------- edits through a placement

#[test]
fn push_pull_through_a_placement_reaches_every_placement() {
    // L7.
    let mut doc = Document::new();
    let (o, def, _, _) = linked_set(&mut doc, 2);
    let top = top_face(&doc, o);
    doc.apply_def_op(
        def,
        o,
        KernelOp::PushPull {
            face: top,
            distance: 1.0,
        },
    )
    .unwrap();
    assert!(approx(max_z(&doc, o), 2.0));
    let tops: Vec<f64> = doc
        .expanded_placements()
        .into_iter()
        .filter(|(m, _, _)| *m == o)
        .map(|(_, pose, _)| {
            polys(&doc, o)
                .iter()
                .map(|p| pose.apply_point(*p).z)
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect();
    assert_eq!(tops.len(), 3);
    assert!(tops.iter().all(|&z| approx(z, 2.0)), "every copy changed");
}

#[test]
fn painting_a_face_through_a_placement_shows_on_every_copy() {
    // L12.
    let mut doc = Document::new();
    let m = doc.add_material(Material::solid("Edge", Rgba8::rgb(200, 30, 30)));
    let (o, def, _, _) = linked_set(&mut doc, 1);
    let top = top_face(&doc, o);
    doc.paint_face(o, top, Some(m)).unwrap();
    assert_eq!(doc.face_material(o, top), Some(m));
    // One shared member, two placements: every copy shows the face.
    assert_eq!(doc.def_members(def).unwrap(), vec![o]);
    assert_eq!(doc.instances_of(def).len(), 2);
}

#[test]
fn a_negative_scale_on_a_placement_reflects_the_pose_and_bakes_nothing() {
    // L5.
    let mut doc = Document::new();
    let (o, _, _, rest) = linked_set(&mut doc, 1);
    let before = polys(&doc, o);
    doc.transform_instance(rest[0], &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .unwrap();
    assert!(doc.instance_pose(rest[0]).unwrap().determinant() < 0.0);
    assert_eq!(polys(&doc, o), before);
}

#[test]
fn name_material_and_attributes_are_per_placement_and_definition_attributes_are_shared() {
    // L11.
    let mut doc = Document::new();
    let m = doc.add_material(Material::solid("Walnut", Rgba8::rgb(90, 60, 40)));
    let (_, def, first, rest) = linked_set(&mut doc, 1);
    let copy = rest[0];
    doc.set_node_name(NodeId::Instance(copy), Some("Left".into()))
        .unwrap();
    doc.set_instance_material(copy, Some(m)).unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Instance(copy)),
        "com.example",
        "slot",
        AttrValue::Text("L".into()),
    )
    .unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Component(def)),
        "com.example",
        "species",
        AttrValue::Text("walnut".into()),
    )
    .unwrap();
    assert_eq!(doc.instance_name(copy), Some("Left"));
    assert_eq!(doc.instance_name(first), None);
    assert_eq!(doc.instance_material(copy), Some(Some(m)));
    assert_eq!(doc.instance_material(first), Some(None));
    assert!(entity_dict(&doc, EntityRef::Instance(first)).is_none());
    for i in [first, copy] {
        let d = doc.instance_def(i).unwrap();
        assert_eq!(
            entity_dict(&doc, EntityRef::Component(d))
                .unwrap()
                .get("com.example")
                .and_then(|ns| ns.get("species")),
            Some(&AttrValue::Text("walnut".into()))
        );
    }
}

#[test]
fn slice_def_member_through_a_placement_reaches_every_placement() {
    let mut doc = Document::new();
    let (o, def, _, rest) = linked_set(&mut doc, 1);
    let plane =
        Plane::from_point_normal(Point3::new(2.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    // The copy sits at x = 2..3, so the world plane x = 2.5 halves it.
    let ((a, b), _) = doc.slice_def_member(rest[0], o, &plane).unwrap();
    let members: BTreeSet<ObjectId> = doc
        .def_members(def)
        .unwrap()
        .into_iter()
        .filter(|&m| doc.object(m).is_some())
        .collect();
    assert_eq!(members, BTreeSet::from([a, b]));
    assert_eq!(
        doc.expanded_placements().len(),
        4,
        "both placements show both halves"
    );
}

// ------------------------------------------------------------ change sets

#[test]
fn change_set_of_a_linked_copy_lists_linked_wraps_and_copied_from_pairs() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (roots, change) = doc
        .duplicate_nodes_array(
            &[NodeId::Object(o)],
            &shift(2.0),
            NonZeroU32::new(2).unwrap(),
            Linked,
        )
        .unwrap();
    let wrap = change.linked[0];
    let r = last_record(&doc);
    assert_eq!(r.changes.linked, vec![wrap]);
    let src = cref(&doc, EntityRef::Instance(wrap.instance));
    for root in &roots {
        let new = cref(&doc, EntityRef::Instance(ins(*root)));
        assert!(r.changes.copied_from.contains(&(new.clone(), src.clone())));
        assert!(r.changes.added.contains(&new));
    }
    assert!(
        r.changes
            .added
            .contains(&cref(&doc, EntityRef::Component(wrap.definition)))
    );
    assert!(r.changes.added.contains(&src));
    assert!(
        r.changes
            .modified
            .contains(&cref(&doc, EntityRef::Object(o)))
    );
}

#[test]
fn change_set_of_an_independent_copy_lists_node_to_node_pairs() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (root, _) = doc
        .duplicate_node(NodeId::Object(o), &shift(2.0), Independent)
        .unwrap();
    let NodeId::Object(copy) = root else {
        panic!("an object copy")
    };
    let r = last_record(&doc);
    assert!(r.changes.linked.is_empty());
    assert_eq!(
        r.changes.copied_from,
        vec![(
            cref(&doc, EntityRef::Object(copy)),
            cref(&doc, EntityRef::Object(o))
        )]
    );
}

#[test]
fn change_set_of_an_unlink_lists_every_remap_pair() {
    let mut doc = Document::new();
    let (o, def, _, rest) = linked_set(&mut doc, 1);
    let (new_def, _, remap, _) = doc.make_unique_by_path(&[rest[0]], true, false).unwrap();
    let r = last_record(&doc);
    assert!(r.changes.copied_from.contains(&(
        cref(&doc, EntityRef::Component(new_def)),
        cref(&doc, EntityRef::Component(def))
    )));
    let copy = remap.objects[&o];
    assert!(r.changes.copied_from.contains(&(
        cref(&doc, EntityRef::Object(copy)),
        cref(&doc, EntityRef::Object(o))
    )));
    assert_eq!(
        r.changes.copied_from.len(),
        remap.objects.len() + remap.groups.len() + remap.instances.len() + remap.components.len()
    );
}

#[test]
fn an_undo_change_set_clears_linked_and_copied_from() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    doc.duplicate_node(NodeId::Object(o), &shift(2.0), Linked)
        .unwrap();
    let generation = doc.history_generation();
    doc.undo().unwrap();
    let r = last_record(&doc);
    assert_eq!(r.kind, kernel::ChangeKind::Undo);
    let undone = r.changes.swapped();
    assert!(undone.linked.is_empty());
    assert!(undone.copied_from.is_empty());
    let since = doc.changes_since_generation(generation).unwrap();
    assert!(since.linked.is_empty());
    assert!(since.copied_from.is_empty());
    // A redo reports the wrap again.
    doc.redo().unwrap();
    assert_eq!(last_record(&doc).changes.linked.len(), 1);
}

// ------------------------------------------------------------- properties

#[derive(Debug, Clone)]
struct Case {
    /// Boards along x; each is loose, or grouped with the next
    /// (`group_next`), or folded into a component (`component`).
    boards: usize,
    group_first_two: bool,
    component_last: bool,
    /// Which top-level nodes the copy lists (bitmask over them).
    pick: u8,
    step: (f64, f64, bool),
    count: u32,
    linked: bool,
}

fn arb_case() -> impl Strategy<Value = Case> {
    (
        2usize..5,
        any::<bool>(),
        any::<bool>(),
        1u8..=255,
        (-3.0f64..3.0, -3.0f64..3.0, any::<bool>()),
        1u32..4,
        any::<bool>(),
    )
        .prop_map(
            |(boards, group_first_two, component_last, pick, step, count, linked)| Case {
                boards,
                group_first_two,
                component_last,
                pick,
                step,
                count,
                linked,
            },
        )
}

fn build(case: &Case) -> (Document, Vec<NodeId>) {
    let mut doc = Document::new();
    let objs: Vec<ObjectId> = (0..case.boards)
        .map(|k| board(&mut doc, 2.0 * k as f64))
        .collect();
    let mut roots: Vec<NodeId> = objs.iter().map(|&o| NodeId::Object(o)).collect();
    if case.group_first_two {
        let (g, _) = doc.group_nodes(&[roots[0], roots[1]]).unwrap();
        roots.drain(0..2);
        roots.insert(0, NodeId::Group(g));
    }
    if case.component_last {
        let last = *roots.last().unwrap();
        let (_, i, _) = doc.make_component(&[last]).unwrap();
        *roots.last_mut().unwrap() = NodeId::Instance(i);
    }
    let picked: Vec<NodeId> = roots
        .iter()
        .enumerate()
        .filter(|(k, _)| case.pick & (1 << k) != 0)
        .map(|(_, &n)| n)
        .collect();
    let picked = if picked.is_empty() {
        vec![roots[0]]
    } else {
        picked
    };
    (doc, picked)
}

fn step_of(case: &Case) -> Transform {
    let t = Transform::translation(Vec3::new(case.step.0, case.step.1, 0.5));
    if case.step.2 {
        Transform::scale(Vec3::new(-1.0, 1.0, 1.0)).then(&t)
    } else {
        t
    }
}

proptest! {
    #[test]
    fn linked_copy_then_undo_is_the_identity(case in arb_case()) {
        let (mut doc, nodes) = build(&case);
        let before = doc.state_hash();
        let policy = if case.linked { Linked } else { Independent };
        let result = doc.duplicate_nodes_array(
            &nodes,
            &step_of(&case),
            NonZeroU32::new(case.count).unwrap(),
            policy,
        );
        match result {
            Ok(_) => {
                let post = doc.state_hash();
                doc.undo().unwrap();
                prop_assert_eq!(doc.state_hash(), before);
                doc.redo().unwrap();
                prop_assert_eq!(doc.state_hash(), post);
            }
            // An independent reflecting copy bakes a mirrored solid, which
            // is legal; any refusal must leave the document untouched.
            Err(_) => prop_assert_eq!(doc.state_hash(), before),
        }
    }

    #[test]
    fn linked_copy_preserves_total_solid_count_and_adds_only_placements(
        boards in 1usize..5,
        count in 1u32..4,
        dx in 0.5f64..4.0,
    ) {
        let mut doc = Document::new();
        let roots: Vec<NodeId> = (0..boards)
            .map(|k| NodeId::Object(board(&mut doc, 3.0 * k as f64)))
            .collect();
        let solids = doc.live_object_ids().len();
        let shown = rendered(&doc);
        let instances = doc.instance_ids().len();
        doc.duplicate_nodes_array(
            &roots,
            &Transform::translation(Vec3::new(0.0, dx, 0.0)),
            NonZeroU32::new(count).unwrap(),
            Linked,
        )
        .unwrap();
        prop_assert_eq!(doc.live_object_ids().len(), solids, "no solid was copied");
        prop_assert_eq!(rendered(&doc), shown + count as usize * boards);
        prop_assert_eq!(
            doc.instance_ids().len(),
            instances + boards * (count as usize + 1),
            "one placement per wrap plus the copies"
        );
    }

    #[test]
    fn unlink_all_then_undo_all_restores_the_document(copies in 1u32..4, order in any::<u64>()) {
        let mut doc = Document::new();
        let (_, def, first, rest) = linked_set(&mut doc, copies);
        let before = doc.state_hash();
        let mut all = vec![first];
        all.extend(rest);
        // A deterministic shuffle of the unlink order.
        let k = all.len();
        all.rotate_left((order as usize) % k);
        for &i in &all {
            doc.make_unique_by_path(&[i], true, false).unwrap();
        }
        prop_assert!(!doc.component_ids().contains(&def));
        for _ in 0..all.len() {
            doc.undo().unwrap();
        }
        prop_assert_eq!(doc.state_hash(), before);
    }
}

// -------------------------------------- make unique by path: hidden on the path

/// One row of the hidden-on-the-path table: `cascade` x the kind of hidden
/// (the slotmap flag of an undone or deleted record, or the user's display
/// flag), each at the outer and at the inner path element.
fn hidden_on_path(cascade: bool, slotmap_hidden: bool) -> Result<(), String> {
    // `a` (world) places A = { box, b }; `b` places B = { box }.
    fn two_deep(doc: &mut Document) -> (InstanceId, InstanceId) {
        let inner = board(doc, 0.0);
        let (_, b, _) = doc.make_component(&[NodeId::Object(inner)]).unwrap();
        let outer_box = board(doc, 4.0);
        let (_, a, _) = doc
            .make_component(&[NodeId::Object(outer_box), NodeId::Instance(b)])
            .unwrap();
        (a, b)
    }
    if slotmap_hidden {
        // Outer: an undone placement of A.
        let mut doc = Document::new();
        let (a, b) = two_deep(&mut doc);
        let def_a = doc.instance_def(a).unwrap();
        let (a2, _) = doc.place_instance(def_a, shift(10.0)).unwrap();
        doc.undo().unwrap();
        let hash = doc.state_hash();
        match doc.make_unique_by_path(&[a2, b], cascade, false) {
            Err(DocumentError::UnknownInstance) => {}
            other => return Err(format!("outer: {other:?}, not UnknownInstance")),
        }
        if doc.state_hash() != hash {
            return Err("outer: the refusal changed the document".into());
        }
        // Inner: `b` deleted from A inside an edit of `a`.
        let mut doc = Document::new();
        let (a, b) = two_deep(&mut doc);
        doc.open_explode_session(a).unwrap();
        doc.delete_node(NodeId::Instance(b)).unwrap();
        doc.close_explode_session().unwrap();
        let hash = doc.state_hash();
        match doc.make_unique_by_path(&[a, b], cascade, false) {
            Err(DocumentError::UnknownInstance) => {}
            other => return Err(format!("inner: {other:?}, not UnknownInstance")),
        }
        if doc.state_hash() != hash {
            return Err("inner: the refusal changed the document".into());
        }
        return Ok(());
    }
    // User-hidden: a display flag never refuses and the fork keeps it, on
    // either element.
    for hide_outer in [true, false] {
        let mut doc = Document::new();
        let (a, b) = two_deep(&mut doc);
        let hidden = if hide_outer { a } else { b };
        doc.set_node_user_hidden(NodeId::Instance(hidden), true);
        let (_, leaf, _, _) = doc
            .make_unique_by_path(&[a, b], cascade, false)
            .map_err(|e| format!("user-hidden outer={hide_outer}: refused {e:?}"))?;
        let still = if hide_outer {
            doc.node_user_hidden(NodeId::Instance(a))
        } else {
            doc.node_user_hidden(NodeId::Instance(leaf))
        };
        if !still {
            return Err(format!(
                "user-hidden outer={hide_outer}: the fork dropped the flag"
            ));
        }
    }
    Ok(())
}

#[test]
fn hidden_on_a_make_unique_path_answers_the_same_in_both_cascade_modes() {
    let mut failures = Vec::new();
    for cascade in [true, false] {
        for slotmap_hidden in [true, false] {
            if let Err(why) = hidden_on_path(cascade, slotmap_hidden) {
                failures.push(format!(
                    "cascade={cascade} slotmap_hidden={slotmap_hidden}: {why}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn made_unique_undo_and_redo_report_the_same_node_sets() {
    let mut doc = Document::new();
    let doc_nodes = |c: &kernel::DocChange| {
        (
            c.objects_touched.iter().copied().collect::<BTreeSet<_>>(),
            c.groups_touched.iter().copied().collect::<BTreeSet<_>>(),
            c.instances_touched.iter().copied().collect::<BTreeSet<_>>(),
            c.components_touched
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            c.sketches_touched.iter().copied().collect::<BTreeSet<_>>(),
        )
    };
    let x = board(&mut doc, 0.0);
    let y = board(&mut doc, 2.0);
    let (gy, _) = doc.group_nodes(&[NodeId::Object(y)]).unwrap();
    let (def_b, b, _) = doc
        .make_component(&[NodeId::Object(x), NodeId::Group(gy)])
        .unwrap();
    let z = board(&mut doc, 6.0);
    let (_, a1, _) = doc
        .make_component(&[NodeId::Object(z), NodeId::Instance(b)])
        .unwrap();
    let def_a = doc.instance_def(a1).unwrap();
    doc.place_instance(def_a, shift(20.0)).unwrap();
    let _ = def_b;
    // A single fork, deep inside the tree (cascade = false forks only `b`'s
    // definition, in place).
    doc.make_unique_by_path(&[a1, b], false, false).unwrap();
    let undo = doc.undo().unwrap();
    let redo = doc.redo().unwrap();
    assert_eq!(doc_nodes(&undo), doc_nodes(&redo));
    assert!(
        !undo.objects_touched.is_empty(),
        "the private copy's members"
    );
}

#[test]
fn make_unique_keeps_the_definition_name() {
    let mut doc = Document::new();
    let o = named_board(&mut doc, 0.0, "Leg", &[], &[]);
    let (def, i, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (copy, _) = doc
        .duplicate_node(NodeId::Instance(i), &shift(2.0), Linked)
        .unwrap();
    let (forked, _, _, _) = doc.make_unique_by_path(&[ins(copy)], true, false).unwrap();
    assert_eq!(doc.component_name(def), Some("Leg"));
    assert_eq!(
        doc.component_name(forked),
        Some("Leg"),
        "a copy is the same part"
    );
}

#[test]
fn group_ids_are_unchanged_by_a_wrap() {
    // A group wrapped as a set keeps its id; it is no longer a world group.
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let ids: Vec<GroupId> = doc.group_ids();
    assert_eq!(ids, vec![g]);
    doc.wrap_linked(NodeId::Group(g)).unwrap();
    assert!(doc.group_ids().is_empty());
    assert_eq!(doc.group_members(g).unwrap(), vec![NodeId::Object(a)]);
}
