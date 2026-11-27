//! Executable specs for booleans over any number of operands (D-106 item
//! 4): a Union is the union of all, a Subtract the first minus the union of
//! the rest, an Intersect the intersection of all, each as one undo step,
//! at the top level and inside a component definition.

use kernel::{
    BooleanError, BooleanOp, Document, DocumentError, ImportNode, ImportScene, MeshRecipe, NodeId,
    Object, ObjectId, Operand, Plane, Point3, Vec3, WatertightState,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// `validate()` passes and the object is watertight.
fn assert_valid_solid(obj: &Object) {
    obj.validate().expect("object validates");
    assert_eq!(
        obj.watertight(),
        WatertightState::Watertight,
        "object is watertight"
    );
}

/// The enclosed volume by the divergence theorem over every face loop
/// (holes wind opposite, so they subtract).
fn signed_volume(obj: &Object) -> f64 {
    let mut v = 0.0;
    for f in obj.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let ring: Vec<Point3> = obj.loop_positions(l).collect();
            let o = ring[0].to_vec();
            for i in 1..ring.len() - 1 {
                let (a, b) = (ring[i].to_vec(), ring[i + 1].to_vec());
                v += o.dot(a.cross(b)) / 6.0;
            }
        }
    }
    v
}

fn build_box_at(doc: &mut Document, lo: Point3, hi: Point3) -> ObjectId {
    let plane = Plane::from_point_normal(lo, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, lo.z), Point3::new(b.0, b.1, lo.z))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, hi.z - lo.z).unwrap().0
}

/// A row of three unit boxes overlapping by half: x 0..1, 0.5..1.5, 1..2.
fn three_in_a_row(doc: &mut Document) -> [NodeId; 3] {
    let a = build_box_at(doc, p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0));
    let b = build_box_at(doc, p3(0.5, 0.0, 0.0), p3(1.5, 1.0, 1.0));
    let c = build_box_at(doc, p3(1.0, 0.0, 0.0), p3(2.0, 1.0, 1.0));
    [NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)]
}

fn volume_of(doc: &Document, node: NodeId) -> f64 {
    match node {
        NodeId::Object(o) => signed_volume(doc.object(o).unwrap()),
        NodeId::Group(g) => doc
            .group_members(g)
            .unwrap()
            .iter()
            .map(|m| volume_of(doc, *m))
            .sum(),
        NodeId::Instance(_) => unreachable!("no instances here"),
    }
}

#[test]
fn union_of_three_is_one_solid_in_one_undo_step() {
    let mut doc = Document::new();
    let nodes = three_in_a_row(&mut doc);
    let depth = doc.undo_depth();
    let (root, _) = doc.boolean_nodes_many(BooleanOp::Union, &nodes).unwrap();
    let NodeId::Object(o) = root else {
        panic!("one solid, not a group: {root:?}")
    };
    assert_valid_solid(doc.object(o).unwrap());
    assert!((signed_volume(doc.object(o).unwrap()) - 2.0).abs() < 1e-9);
    assert_eq!(doc.visible_object_ids().len(), 1);
    assert_eq!(doc.undo_depth(), depth + 1, "one undo step");
    doc.undo().unwrap();
    assert_eq!(
        doc.visible_object_ids().len(),
        3,
        "undo brings all three back"
    );
    doc.redo().unwrap();
    assert_eq!(doc.visible_object_ids().len(), 1);
}

#[test]
fn subtract_takes_the_rest_off_the_first() {
    // The first box minus two cutters that overlap it from both sides.
    let mut doc = Document::new();
    let a = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0));
    let b = build_box_at(&mut doc, p3(-0.5, -0.5, -0.5), p3(0.25, 1.5, 1.5));
    let c = build_box_at(&mut doc, p3(0.75, -0.5, -0.5), p3(1.5, 1.5, 1.5));
    let (root, _) = doc
        .boolean_nodes_many(
            BooleanOp::Subtract,
            &[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)],
        )
        .unwrap();
    assert!(
        (volume_of(&doc, root) - 0.5).abs() < 1e-9,
        "{}",
        volume_of(&doc, root)
    );
    assert_eq!(doc.visible_object_ids().len(), 1);
}

#[test]
fn cutters_that_only_touch_each_other_still_come_off_one_at_a_time() {
    // Two cutters that share only an edge cannot be one volume, so the
    // subtract takes them in turn; one of them never reaches the bar and
    // takes nothing off.
    let mut doc = Document::new();
    let a = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(3.0, 1.0, 1.0));
    let b = build_box_at(&mut doc, p3(0.5, -1.0, -1.0), p3(1.0, 2.0, 2.0));
    let c = build_box_at(&mut doc, p3(1.0, 2.0, -1.0), p3(1.5, 3.0, 2.0));
    let (root, _) = doc
        .boolean_nodes_many(
            BooleanOp::Subtract,
            &[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)],
        )
        .unwrap();
    assert!(
        (volume_of(&doc, root) - 2.5).abs() < 1e-9,
        "{}",
        volume_of(&doc, root)
    );
}

/// A bar, two cutters that share only an edge inside it, and a third
/// that overlaps both. In the order given here the two touching cutters
/// come before the one that bridges them.
fn bar_with_touching_cutters_and_a_bridge(doc: &mut Document) -> [NodeId; 4] {
    let bar = build_box_at(doc, p3(0.0, 0.0, 0.0), p3(6.0, 1.0, 1.0));
    let t1 = build_box_at(doc, p3(1.0, -1.0, -1.0), p3(2.0, 0.5, 2.0));
    let t2 = build_box_at(doc, p3(2.0, 0.5, -1.0), p3(3.0, 2.0, 2.0));
    let bridge = build_box_at(doc, p3(1.5, 0.25, -1.0), p3(2.5, 0.75, 2.0));
    [
        NodeId::Object(bar),
        NodeId::Object(t1),
        NodeId::Object(t2),
        NodeId::Object(bridge),
    ]
}

#[test]
fn the_order_of_touching_operands_and_their_bridge_does_not_matter() {
    // Whether the bridge is picked before or after the second touching
    // cutter, the three are one cutting volume and the answer is the same.
    for (op, expected) in [(BooleanOp::Subtract, 4.75), (BooleanOp::Union, 14.5)] {
        for order in [[0, 1, 2, 3], [0, 1, 3, 2], [0, 2, 1, 3], [0, 3, 2, 1]] {
            let mut doc = Document::new();
            let n = bar_with_touching_cutters_and_a_bridge(&mut doc);
            let picked: Vec<NodeId> = order.iter().map(|&i| n[i]).collect();
            let (root, _) = doc
                .boolean_nodes_many(op, &picked)
                .unwrap_or_else(|e| panic!("{op:?} in order {order:?}: {e}"));
            let v = volume_of(&doc, root);
            assert!(
                (v - expected).abs() < 1e-9,
                "{op:?} in order {order:?}: {v}"
            );
            if let NodeId::Object(o) = root {
                assert_valid_solid(doc.object(o).unwrap());
            }
        }
    }
}

#[test]
fn a_union_of_solids_that_only_touch_along_an_edge_is_still_refused() {
    // Nothing bridges the pair, so no order makes them one solid; the
    // refusal leaves all three where they were.
    let mut doc = Document::new();
    let a = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0));
    let b = build_box_at(&mut doc, p3(0.5, 0.0, 0.0), p3(2.0, 1.0, 1.0));
    let c = build_box_at(&mut doc, p3(2.0, 1.0, 0.0), p3(3.0, 2.0, 1.0));
    let before = doc.state_hash();
    for order in [[a, b, c], [a, c, b], [c, a, b]] {
        let picked: Vec<NodeId> = order.iter().map(|&o| NodeId::Object(o)).collect();
        let err = doc
            .boolean_nodes_many(BooleanOp::Union, &picked)
            .unwrap_err();
        assert!(
            matches!(err, DocumentError::Boolean(BooleanError::DegenerateContact)),
            "{err}"
        );
        assert_eq!(doc.state_hash(), before);
    }
}

#[test]
fn touching_cutters_that_all_miss_refuse_nothing_removed_untouched() {
    // The same edge-touching pair as above, both beside the bar: taken in
    // turn, neither removes anything, so the subtract is refused and the
    // document is left as it was.
    let mut doc = Document::new();
    let a = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(3.0, 1.0, 1.0));
    let b = build_box_at(&mut doc, p3(0.5, 2.0, -1.0), p3(1.0, 3.0, 2.0));
    let c = build_box_at(&mut doc, p3(1.0, 3.0, -1.0), p3(1.5, 4.0, 2.0));
    let hash = doc.state_hash();
    let depth = doc.undo_depth();
    let err = doc
        .boolean_nodes_many(
            BooleanOp::Subtract,
            &[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)],
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err, DocumentError::Boolean(BooleanError::NothingRemoved));
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.undo_depth(), depth);
}

#[test]
fn a_cutter_that_misses_beside_one_that_cuts_is_left_out() {
    // Two separate cutters, one through the bar and one well clear of it:
    // the one that cuts is enough.
    let mut doc = Document::new();
    let a = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(3.0, 1.0, 1.0));
    let b = build_box_at(&mut doc, p3(0.5, -1.0, -1.0), p3(1.0, 2.0, 2.0));
    let c = build_box_at(&mut doc, p3(5.0, 5.0, 5.0), p3(6.0, 6.0, 6.0));
    let (root, _) = doc
        .boolean_nodes_many(
            BooleanOp::Subtract,
            &[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)],
        )
        .unwrap();
    assert!(
        (volume_of(&doc, root) - 2.5).abs() < 1e-9,
        "{}",
        volume_of(&doc, root)
    );
}

#[test]
fn operand_places_read_as_english_ordinals() {
    let words: Vec<String> = [0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 100, 101, 111]
        .into_iter()
        .map(|i| Operand::at(i).ordinal())
        .collect();
    assert_eq!(
        words,
        [
            "first", "second", "third", "4th", "11th", "12th", "13th", "14th", "21st", "22nd",
            "23rd", "24th", "101st", "102nd", "112th",
        ]
    );
}

#[test]
fn subtract_whose_cutters_sever_names_every_operand_in_the_group() {
    // A bar cut twice across: three pieces in a group named after all three.
    let mut doc = Document::new();
    let bar = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(3.0, 1.0, 1.0));
    let k1 = build_box_at(&mut doc, p3(0.9, -0.5, -0.5), p3(1.1, 1.5, 1.5));
    let k2 = build_box_at(&mut doc, p3(1.9, -0.5, -0.5), p3(2.1, 1.5, 1.5));
    doc.set_node_name(NodeId::Object(bar), Some("Bar".to_string()))
        .unwrap();
    doc.set_node_name(NodeId::Object(k1), Some("Saw".to_string()))
        .unwrap();
    doc.set_node_name(NodeId::Object(k2), Some("Saw 2".to_string()))
        .unwrap();
    let (root, _) = doc
        .boolean_nodes_many(
            BooleanOp::Subtract,
            &[NodeId::Object(bar), NodeId::Object(k1), NodeId::Object(k2)],
        )
        .unwrap();
    let NodeId::Group(g) = root else {
        panic!("three pieces arrive in a group: {root:?}")
    };
    assert_eq!(doc.group_members(g).unwrap().len(), 3);
    assert_eq!(doc.group_name(g), Some("Bar \u{2212} Saw \u{2212} Saw 2"));
    assert!((volume_of(&doc, root) - 2.6).abs() < 1e-9);
}

#[test]
fn intersect_of_three_keeps_what_all_share() {
    let mut doc = Document::new();
    let a = build_box_at(&mut doc, p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0));
    let b = build_box_at(&mut doc, p3(0.25, 0.0, 0.0), p3(1.25, 1.0, 1.0));
    let c = build_box_at(&mut doc, p3(0.5, 0.0, 0.0), p3(1.5, 1.0, 1.0));
    let (root, _) = doc
        .boolean_nodes_many(
            BooleanOp::Intersect,
            &[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)],
        )
        .unwrap();
    assert!((volume_of(&doc, root) - 0.5).abs() < 1e-9);
}

#[test]
fn two_operands_are_the_old_boolean_exactly() {
    let mut doc1 = Document::new();
    let [a, b, _] = three_in_a_row(&mut doc1);
    let (r1, _) = doc1.boolean_nodes(BooleanOp::Subtract, a, b).unwrap();
    let mut doc2 = Document::new();
    let [a2, b2, _] = three_in_a_row(&mut doc2);
    let (r2, _) = doc2
        .boolean_nodes_many(BooleanOp::Subtract, &[a2, b2])
        .unwrap();
    let (NodeId::Object(_), NodeId::Object(_)) = (r1, r2) else {
        panic!("one solid each")
    };
    // Built the same way, the two documents are the same document.
    assert_eq!(doc1.state_hash(), doc2.state_hash());
}

#[test]
fn fewer_than_two_or_a_repeated_operand_refuses_untouched() {
    let mut doc = Document::new();
    let [a, b, _] = three_in_a_row(&mut doc);
    let hash = doc.state_hash();
    assert!(matches!(
        doc.boolean_nodes_many(BooleanOp::Union, &[a]),
        Err(DocumentError::BooleanNeedsTwo)
    ));
    assert!(matches!(
        doc.boolean_nodes_many(BooleanOp::Union, &[]),
        Err(DocumentError::BooleanNeedsTwo)
    ));
    assert!(matches!(
        doc.boolean_nodes_many(BooleanOp::Union, &[a, b, a]),
        Err(DocumentError::Boolean(BooleanError::DegenerateContact))
    ));
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn a_failing_operand_leaves_every_other_operand_untouched() {
    // The third operand lies far away: a Union still works (pieces), but an
    // Intersect of nothing shared refuses, and nothing is hidden.
    let mut doc = Document::new();
    let [a, b, _] = three_in_a_row(&mut doc);
    let far = NodeId::Object(build_box_at(
        &mut doc,
        p3(10.0, 0.0, 0.0),
        p3(11.0, 1.0, 1.0),
    ));
    let hash = doc.state_hash();
    assert!(matches!(
        doc.boolean_nodes_many(BooleanOp::Intersect, &[a, b, far]),
        Err(DocumentError::Boolean(_))
    ));
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.visible_object_ids().len(), 4);
}

/// An open shell (a box missing its top) ingested at `at`: the one honest
/// way a leaky object exists in a document.
fn open_box_at(doc: &mut Document, at: f64) -> ObjectId {
    let p = |x: f64, y: f64, z: f64| Point3::new(at + x, y, z);
    let open = MeshRecipe {
        name: "open".into(),
        positions: vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ],
        faces: vec![
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
        face_materials: vec![kernel::NO_MATERIAL; 5],
        face_uv_frames: vec![None; 5],
        face_holes: vec![Vec::new(); 5],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    };
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(open)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        geo: None,
        extra: Default::default(),
    };
    let (_, change) = doc.ingest(scene, vec![]).unwrap();
    change.objects_touched[0]
}

#[test]
fn a_leaky_third_operand_is_named_as_the_third() {
    let mut doc = Document::new();
    let [a, b, _] = three_in_a_row(&mut doc);
    let leaky = NodeId::Object(open_box_at(&mut doc, 10.0));
    let hash = doc.state_hash();
    let err = doc
        .boolean_nodes_many(BooleanOp::Union, &[a, b, leaky])
        .map(|_| ())
        .unwrap_err();
    assert_eq!(
        err,
        DocumentError::BooleanOperandNotSolid {
            which: Operand::Nth(3)
        }
    );
    assert!(err.to_string().contains("third selection"), "{err}");
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn members_of_one_component_combine_in_one_step() {
    let mut doc = Document::new();
    let nodes = three_in_a_row(&mut doc);
    let (def, _inst, _) = doc.make_component(&nodes).unwrap();
    let members: Vec<ObjectId> = doc.def_members(def).unwrap();
    assert_eq!(members.len(), 3);
    let depth = doc.undo_depth();
    let (id, _) = doc
        .boolean_in_component_many(def, &members, BooleanOp::Union)
        .unwrap();
    assert!((signed_volume(doc.object(id).unwrap()) - 2.0).abs() < 1e-9);
    // Consumed members stay listed, hidden (the definition's tombstone
    // rule); the live ones are what counts.
    let live = |doc: &Document| {
        doc.def_members(def)
            .unwrap()
            .iter()
            .filter(|&&o| doc.object(o).is_some())
            .count()
    };
    assert_eq!(live(&doc), 1);
    assert_eq!(doc.undo_depth(), depth + 1);
    doc.undo().unwrap();
    assert_eq!(live(&doc), 3);
    doc.redo().unwrap();
    assert_eq!(live(&doc), 1);
    assert!(matches!(
        doc.boolean_in_component_many(def, &members[..1], BooleanOp::Union),
        Err(DocumentError::BooleanNeedsTwo)
    ));
}
