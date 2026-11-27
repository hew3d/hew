//! Executable specs for `Document::node_effectively_hidden`: a node is
//! hidden by its own user-hidden flag, a group ancestor's flag, or a hidden
//! tag (or a hidden prefix of one) on the node or any ancestor.

use kernel::{Document, NodeId, Plane, Point3};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// A box at `x`: a rectangle extruded half a meter.
fn boxed(doc: &mut Document, x: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], 0.5).expect("extrude");
    *doc.live_object_ids().last().expect("an object")
}

fn path(segments: &[&str]) -> Vec<String> {
    segments.iter().map(|s| s.to_string()).collect()
}

/// Two boxes grouped: the group, and the first box inside it.
fn grouped() -> (Document, NodeId, NodeId, NodeId) {
    let mut doc = Document::new();
    let a = NodeId::Object(boxed(&mut doc, 0.0));
    let b = NodeId::Object(boxed(&mut doc, 3.0));
    let (g, _) = doc.group_nodes(&[a, b]).expect("group");
    (doc, NodeId::Group(g), a, b)
}

#[test]
fn own_flag() {
    let mut doc = Document::new();
    let a = NodeId::Object(boxed(&mut doc, 0.0));
    assert!(!doc.node_effectively_hidden(a));
    doc.set_node_user_hidden(a, true);
    assert!(doc.node_effectively_hidden(a));
    doc.set_node_user_hidden(a, false);
    assert!(!doc.node_effectively_hidden(a));
}

#[test]
fn group_ancestor() {
    let (mut doc, g, a, b) = grouped();
    doc.set_node_user_hidden(g, true);
    assert!(doc.node_effectively_hidden(g));
    assert!(doc.node_effectively_hidden(a));
    assert!(doc.node_effectively_hidden(b));
    // Hiding a member hides only that member, not the group or its sibling.
    doc.set_node_user_hidden(g, false);
    doc.set_node_user_hidden(a, true);
    assert!(!doc.node_effectively_hidden(g));
    assert!(doc.node_effectively_hidden(a));
    assert!(!doc.node_effectively_hidden(b));
}

#[test]
fn tag_on_node() {
    let mut doc = Document::new();
    let a = NodeId::Object(boxed(&mut doc, 0.0));
    let b = NodeId::Object(boxed(&mut doc, 3.0));
    doc.add_node_tag(a, path(&["Walls"])).expect("tag");
    assert!(!doc.node_effectively_hidden(a));
    doc.set_tag_hidden(path(&["Walls"]), true);
    assert!(doc.node_effectively_hidden(a));
    assert!(
        !doc.node_effectively_hidden(b),
        "an untagged node stays shown"
    );
}

#[test]
fn tag_prefix_hidden() {
    let mut doc = Document::new();
    let a = NodeId::Object(boxed(&mut doc, 0.0));
    doc.add_node_tag(a, path(&["Structure", "Roof"]))
        .expect("tag");
    // A hidden sibling path does not cover it.
    doc.set_tag_hidden(path(&["Structure", "Walls"]), true);
    assert!(!doc.node_effectively_hidden(a));
    // Hiding the parent path covers every tag under it.
    doc.set_tag_hidden(path(&["Structure"]), true);
    assert!(doc.node_effectively_hidden(a));
    // A hidden descendant path does not cover the node's own, shorter path.
    doc.set_tag_hidden(path(&["Structure"]), false);
    doc.set_tag_hidden(path(&["Structure", "Roof", "Ridge"]), true);
    assert!(!doc.node_effectively_hidden(a));
}

#[test]
fn tag_on_ancestor() {
    let (mut doc, g, a, b) = grouped();
    doc.add_node_tag(g, path(&["Furniture", "Chairs"]))
        .expect("tag");
    doc.set_tag_hidden(path(&["Furniture"]), true);
    assert!(doc.node_effectively_hidden(g));
    assert!(doc.node_effectively_hidden(a));
    assert!(doc.node_effectively_hidden(b));
}

#[test]
fn a_stale_node_is_not_hidden() {
    let mut doc = Document::new();
    let a = boxed(&mut doc, 0.0);
    doc.undo().expect("undo the extrude");
    assert!(!doc.node_effectively_hidden(NodeId::Object(a)));
}

/// A definition member is hidden by its own flag and tags and those of the
/// groups above it inside the definition — the same rule the renderer draws
/// placements by (`expanded_def_placements_shown`), so the two agree.
#[test]
fn definition_members_agree_with_the_drawn_placements() {
    let mut doc = Document::new();
    let a = NodeId::Object(boxed(&mut doc, 0.0));
    let b = NodeId::Object(boxed(&mut doc, 3.0));
    let (g, _) = doc.group_nodes(&[a, b]).expect("group");
    let (def, _instance, _) = doc
        .make_component(&[NodeId::Group(g)])
        .expect("make a component");
    let members: Vec<NodeId> = doc
        .expanded_def_placements(def)
        .into_iter()
        .map(|(o, _)| NodeId::Object(o))
        .collect();
    assert_eq!(members.len(), 2);
    let agree = |doc: &Document| {
        let shown = doc.expanded_def_placements_shown(def);
        for (member, shown) in members.iter().zip(shown) {
            assert_eq!(doc.node_effectively_hidden(*member), !shown, "{member:?}");
        }
    };
    agree(&doc);
    doc.add_node_tag(members[0], path(&["Hardware", "Screws"]))
        .expect("tag a member");
    doc.set_tag_hidden(path(&["Hardware"]), true);
    agree(&doc);
    assert!(doc.node_effectively_hidden(members[0]));
    doc.set_tag_hidden(path(&["Hardware"]), false);
    doc.set_node_user_hidden(members[1], true);
    agree(&doc);
    doc.set_node_user_hidden(members[1], false);
    // The definition's own group, hidden, hides both members.
    let inner = doc
        .node_parent(members[0])
        .map(NodeId::Group)
        .expect("the members sit in the definition's group");
    doc.add_node_tag(inner, path(&["Box"]))
        .expect("tag the group");
    doc.set_tag_hidden(path(&["Box"]), true);
    agree(&doc);
    assert!(members.iter().all(|m| doc.node_effectively_hidden(*m)));
}
