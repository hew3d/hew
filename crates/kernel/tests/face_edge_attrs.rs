//! Specs for attribute dictionaries on solid faces and edges
//! (docs/agents/HEW_API.md §8; docs/agents/PLUGINS.md "Attributes";
//! docs/dev/HEW_FILE_FORMAT.md §3, geometry buffer v9).
//!
//! A face or edge dictionary is a value interned in its Object. Writes are
//! ordinary undoable document mutations; the dictionary rides the element
//! through every op by one rule per construction site: an element built
//! from an existing one takes its dictionary, one built from nothing has
//! none, a merge keeps the survivor's, and edges rebuilt from positions
//! (booleans, slices, component rebuilds) start without one.

use kernel::{
    AttrDict, AttrScope, AttrValue, BooleanOp, DecodeError, DefRecipe, Document, DocumentError,
    EdgeId, FaceId, ImportNode, ImportScene, KernelOp, KernelOpReport, MeshRecipe, NodeId, Object,
    ObjectId, Plane, Point3, RetracePath, Transform, Vec3,
};
use std::collections::BTreeMap;

const NS: &str = "com.example.t";

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

/// `{NS: {key: value}}`.
fn dict(key: &str, value: i64) -> AttrDict {
    BTreeMap::from([(
        NS.to_string(),
        BTreeMap::from([(key.to_string(), AttrValue::Int(value))]),
    )])
}

fn obj(doc: &Document, id: ObjectId) -> &Object {
    doc.object(id).expect("object is live")
}

fn centroid(o: &Object, f: FaceId) -> Point3 {
    let ring: Vec<Point3> = o.loop_positions(o.faces()[f].outer_loop).collect();
    let k = 1.0 / ring.len() as f64;
    ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
        Point3::new(acc.x + p.x * k, acc.y + p.y * k, acc.z + p.z * k)
    })
}

/// The face whose plane faces `n` and lies at offset `at` along it.
fn face_where(o: &Object, n: Vec3, at: f64) -> FaceId {
    let hits: Vec<FaceId> = o
        .faces()
        .iter()
        .filter(|(f, face)| {
            let c = centroid(o, *f);
            face.plane.normal().dot(n) > 0.999
                && ((c.x * n.x + c.y * n.y + c.z * n.z) - at).abs() < 1e-9
        })
        .map(|(f, _)| f)
        .collect();
    assert_eq!(hits.len(), 1, "one face faces {n:?} at {at}");
    hits[0]
}

/// The face holding `p` strictly inside it (on its plane, inside its
/// outer ring, outside its holes).
fn face_containing(o: &Object, p: Point3) -> FaceId {
    let hits: Vec<FaceId> = o
        .faces()
        .iter()
        .filter(|(_, face)| {
            if face.plane.signed_distance(p).abs() > 1e-9 {
                return false;
            }
            let n = face.plane.normal();
            // Inside a convex ring (every ring used here is convex): on the
            // same side of each of its edges, whichever way it winds.
            let inside = |l| {
                let ring: Vec<Point3> = o.loop_positions(l).collect();
                let sides: Vec<f64> = ring
                    .iter()
                    .enumerate()
                    .map(|(i, &a)| (ring[(i + 1) % ring.len()] - a).cross(p - a).dot(n))
                    .collect();
                sides.iter().all(|&x| x > 1e-12) || sides.iter().all(|&x| x < -1e-12)
            };
            inside(face.outer_loop) && !face.inner_loops.iter().any(|&l| inside(l))
        })
        .map(|(f, _)| f)
        .collect();
    assert_eq!(hits.len(), 1, "one face holds {p:?}");
    hits[0]
}

fn top(o: &Object, h: f64) -> FaceId {
    face_where(o, Vec3::new(0.0, 0.0, 1.0), h)
}

fn edge_between(o: &Object, a: Point3, b: Point3) -> EdgeId {
    o.edges()
        .keys()
        .find(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (p.approx_eq(a, 1e-9) && q.approx_eq(b, 1e-9))
                || (p.approx_eq(b, 1e-9) && q.approx_eq(a, 1e-9))
        })
        .expect("an edge joins the two points")
}

fn faces_with(o: &Object, d: &AttrDict) -> usize {
    o.faces()
        .keys()
        .filter(|&f| o.face_attrs(f) == Some(d))
        .count()
}

fn faces_with_any(o: &Object) -> usize {
    o.faces()
        .keys()
        .filter(|&f| o.face_attrs(f).is_some())
        .count()
}

fn edges_with_any(o: &Object) -> usize {
    o.edges()
        .keys()
        .filter(|&e| o.edge_attrs(e).is_some())
        .count()
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// Splits the unit box's top (height 1) straight across at x = 0.5.
fn split_top(doc: &mut Document, id: ObjectId) -> kernel::FaceSplitReport {
    let t = top(obj(doc, id), 1.0);
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFace {
                face: t,
                path: vec![p(0.5, 0.0, 1.0), p(0.5, 1.0, 1.0)],
                restore: None,
                curves: Vec::new(),
            },
        )
        .expect("split the top");
    match report {
        KernelOpReport::FaceSplit(r) => r,
        other => panic!("unexpected report {other:?}"),
    }
}

// ---------------------------------------------------------------- writes

#[test]
fn face_set_get_delete_with_undo_and_redo() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    assert_eq!(doc.face_attr_get(id, t).unwrap(), None);

    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.face_attr_set(id, t, NS, "j", AttrValue::Int(2))
        .unwrap();
    let both: AttrDict = BTreeMap::from([(
        NS.to_string(),
        BTreeMap::from([
            ("j".to_string(), AttrValue::Int(2)),
            ("k".to_string(), AttrValue::Int(1)),
        ]),
    )]);
    assert_eq!(doc.face_attr_get(id, t).unwrap(), Some(&both));
    // No other face picked anything up.
    assert_eq!(faces_with_any(obj(&doc, id)), 1);

    doc.face_attr_delete(id, t, NS, Some("k")).unwrap();
    assert_eq!(doc.face_attr_get(id, t).unwrap(), Some(&dict("j", 2)));

    doc.undo().unwrap();
    assert_eq!(doc.face_attr_get(id, t).unwrap(), Some(&both));
    doc.undo().unwrap();
    assert_eq!(doc.face_attr_get(id, t).unwrap(), Some(&dict("k", 1)));
    doc.undo().unwrap();
    // The write that created the namespace takes it away again.
    assert_eq!(doc.face_attr_get(id, t).unwrap(), None);

    doc.redo().unwrap();
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.face_attr_get(id, t).unwrap(), Some(&dict("j", 2)));

    // A whole namespace goes in one step, and comes back in one.
    doc.face_attr_delete(id, t, NS, None).unwrap();
    assert_eq!(doc.face_attr_get(id, t).unwrap(), None);
    doc.undo().unwrap();
    assert_eq!(doc.face_attr_get(id, t).unwrap(), Some(&dict("j", 2)));
}

#[test]
fn edge_set_get_delete_with_undo_and_redo() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    assert_eq!(doc.edge_attr_get(id, e).unwrap(), None);

    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(7))
        .unwrap();
    assert_eq!(doc.edge_attr_get(id, e).unwrap(), Some(&dict("e", 7)));
    assert_eq!(edges_with_any(obj(&doc, id)), 1);
    assert_eq!(faces_with_any(obj(&doc, id)), 0);

    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(8))
        .unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.edge_attr_get(id, e).unwrap(), Some(&dict("e", 7)));
    doc.redo().unwrap();
    assert_eq!(doc.edge_attr_get(id, e).unwrap(), Some(&dict("e", 8)));

    doc.edge_attr_delete(id, e, NS, Some("e")).unwrap();
    assert_eq!(doc.edge_attr_get(id, e).unwrap(), None);
    doc.undo().unwrap();
    assert_eq!(doc.edge_attr_get(id, e).unwrap(), Some(&dict("e", 8)));
}

#[test]
fn bad_writes_refuse_typed_and_leave_the_document_untouched() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    let depth = doc.undo_depth();
    let hash = doc.state_hash();

    let v = || AttrValue::Int(1);
    assert_eq!(
        doc.face_attr_set(id, t, "", "k", v()).unwrap_err(),
        DocumentError::InvalidAttrName
    );
    assert_eq!(
        doc.edge_attr_set(id, e, NS, "", v()).unwrap_err(),
        DocumentError::InvalidAttrName
    );
    assert_eq!(
        doc.face_attr_set(id, FaceId::default(), NS, "k", v())
            .unwrap_err(),
        DocumentError::UnknownFace
    );
    assert_eq!(
        doc.edge_attr_set(id, EdgeId::default(), NS, "k", v())
            .unwrap_err(),
        DocumentError::UnknownEdge
    );
    assert_eq!(
        doc.face_attr_set(id, t, NS, "k", AttrValue::Float(f64::NAN))
            .unwrap_err(),
        DocumentError::NonFiniteAttrValue
    );
    assert_eq!(
        doc.face_attr_delete(id, t, NS, Some("k")).unwrap_err(),
        DocumentError::UnknownAttr
    );
    assert_eq!(
        doc.edge_attr_delete(id, e, NS, None).unwrap_err(),
        DocumentError::UnknownAttr
    );
    assert_eq!(
        doc.face_attr_get(id, FaceId::default()).unwrap_err(),
        DocumentError::UnknownFace
    );
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.state_hash(), hash);

    // A hidden object (a boolean's operand) answers unknown_object.
    let other = extrude_box(&mut doc, 0.5, 0.5, 1.5, 1.5, 1.0);
    doc.boolean(BooleanOp::Union, id, other).unwrap();
    assert_eq!(
        doc.face_attr_set(id, t, NS, "k", v()).unwrap_err(),
        DocumentError::UnknownObject
    );
    assert_eq!(
        doc.edge_attr_get(id, e).unwrap_err(),
        DocumentError::UnknownObject
    );
}

#[test]
fn a_write_replays_after_a_structural_undo_redo_re_mints_its_face() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    // Split the top, then undo and redo the split: the halves come back
    // with fresh handles.
    let r = split_top(&mut doc, id);
    let left = r
        .new_faces
        .into_iter()
        .find(|&f| centroid(obj(&doc, id), f).x < 0.5)
        .unwrap();
    doc.face_attr_set(id, left, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.undo().unwrap(); // the write
    doc.undo().unwrap(); // the split
    doc.redo().unwrap(); // the split, re-minted
    doc.redo().unwrap(); // the write, found again by its geometry
    let o = obj(&doc, id);
    let left = face_containing(o, p(0.25, 0.5, 1.0));
    assert_eq!(o.face_attrs(left), Some(&dict("k", 1)));
    assert_eq!(faces_with_any(o), 1);
}

#[test]
fn a_definition_member_write_reports_its_placements() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (c, i, _) = doc.make_component(&[NodeId::Object(id)]).unwrap();
    let t = top(obj(&doc, id), 1.0);
    let change = doc
        .face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    assert_eq!(change.objects_touched, vec![id]);
    assert_eq!(change.components_touched, vec![c]);
    assert!(change.instances_touched.contains(&i));
}

#[test]
fn change_sets_name_the_object_with_a_face_or_edge_scope() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    let last = |doc: &Document| {
        doc.change_records_since_epoch(0)
            .unwrap()
            .last()
            .unwrap()
            .clone()
    };
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let r = last(&doc);
    assert_eq!(r.changes.attrs.len(), 1);
    let a = &r.changes.attrs[0];
    assert_eq!(a.scope, AttrScope::Face);
    assert_eq!(a.ns, NS);
    assert_eq!(
        a.object.as_ref().unwrap().entity,
        kernel::EntityRef::Object(id)
    );
    assert!(a.target.is_none());
    assert!(r.changes.modified.is_empty());

    doc.edge_attr_set(id, e, NS, "k", AttrValue::Int(1))
        .unwrap();
    assert_eq!(last(&doc).changes.attrs[0].scope, AttrScope::Edge);
    doc.edge_attr_delete(id, e, NS, None).unwrap();
    assert_eq!(last(&doc).changes.attrs[0].scope, AttrScope::Edge);
    doc.face_attr_delete(id, t, NS, None).unwrap();
    assert_eq!(last(&doc).changes.attrs[0].scope, AttrScope::Face);
}

// ----------------------------------------------------------- propagation

#[test]
fn an_imprinted_sub_face_inherits_and_its_loop_edges_have_none() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: vec![
                    p(0.25, 0.25, 1.0),
                    p(0.75, 0.25, 1.0),
                    p(0.75, 0.75, 1.0),
                    p(0.25, 0.75, 1.0),
                ],
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("imprint report")
    };
    let o = obj(&doc, id);
    assert_eq!(o.face_attrs(r.sub_face), Some(&dict("k", 1)));
    assert_eq!(o.face_attrs(r.parent), Some(&dict("k", 1)));
    assert!(r.new_edges.iter().all(|&e| o.edge_attrs(e).is_none()));
}

#[test]
fn split_halves_inherit_and_split_edge_fragments_inherit() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    // The cut lands mid-edge on the front top edge, which carries a
    // dictionary: both fragments keep it.
    let front = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, front, NS, "e", AttrValue::Int(7))
        .unwrap();
    let r = split_top(&mut doc, id);
    let o = obj(&doc, id);
    for f in r.new_faces {
        assert_eq!(o.face_attrs(f), Some(&dict("k", 1)));
    }
    assert!(r.new_edges.iter().all(|&e| o.edge_attrs(e).is_none()));
    let (_, frags) = r
        .split_boundary_edges
        .iter()
        .find(|(dead, _)| *dead == front)
        .expect("the front edge was split");
    for &e in frags {
        assert_eq!(o.edge_attrs(e), Some(&dict("e", 7)));
    }
    // The back edge carried nothing, and neither do its fragments.
    assert_eq!(edges_with_any(o), 2);
}

#[test]
fn heal_keeps_the_incoming_edges_dictionary() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let front = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, front, NS, "e", AttrValue::Int(7))
        .unwrap();
    let r = split_top(&mut doc, id);
    // Merging the halves back heals each split edge into one again.
    doc.apply_object_op(
        id,
        KernelOp::MergeFaces {
            edge: r.new_edges[0],
        },
    )
    .unwrap();
    let o = obj(&doc, id);
    let healed = edge_between(o, p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    assert_eq!(o.edge_attrs(healed), Some(&dict("e", 7)));
    assert_eq!(edges_with_any(o), 1);
}

#[test]
fn push_pull_walls_have_none_and_the_moved_face_keeps_its_own() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    // Imprint and boss a sub-face of the top: its walls are new.
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: vec![
                    p(0.25, 0.25, 1.0),
                    p(0.75, 0.25, 1.0),
                    p(0.75, 0.75, 1.0),
                    p(0.25, 0.75, 1.0),
                ],
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("imprint report")
    };
    doc.face_attr_set(id, r.sub_face, NS, "k", AttrValue::Int(2))
        .unwrap();
    doc.apply_object_op(
        id,
        KernelOp::ExtrudeSubFace {
            sub_face: r.sub_face,
            distance: 0.5,
        },
    )
    .unwrap();
    let o = obj(&doc, id);
    let raised = top(o, 1.5);
    assert_eq!(o.face_attrs(raised), Some(&dict("k", 2)));
    assert_eq!(faces_with(o, &dict("k", 1)), 1, "the parent keeps its own");
    assert_eq!(faces_with_any(o), 2, "the four boss walls have none");
    assert_eq!(edges_with_any(o), 0);

    // A plain push/pull of the whole box's top.
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: t,
            distance: 0.5,
        },
    )
    .unwrap();
    let o = obj(&doc, id);
    assert_eq!(o.face_attrs(top(o, 1.5)), Some(&dict("k", 1)));
    assert_eq!(faces_with_any(o), 1);
}

#[test]
fn boolean_result_faces_keep_their_sources_and_result_edges_have_none() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 0.5, 0.25, 1.5, 0.75, 2.0);
    let a_top = top(obj(&doc, a), 1.0);
    doc.face_attr_set(a, a_top, NS, "k", AttrValue::Int(1))
        .unwrap();
    // B's face inside A becomes a wall of the notch.
    let b_left = face_where(obj(&doc, b), Vec3::new(-1.0, 0.0, 0.0), -0.5);
    doc.face_attr_set(b, b_left, NS, "k", AttrValue::Int(2))
        .unwrap();
    let a_edge = edge_between(obj(&doc, a), p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0));
    doc.edge_attr_set(a, a_edge, NS, "e", AttrValue::Int(7))
        .unwrap();

    let (result, _) = doc.boolean(BooleanOp::Subtract, a, b).unwrap();
    let o = obj(&doc, result);
    assert_eq!(faces_with(o, &dict("k", 1)), 1, "A's notched top");
    assert_eq!(
        faces_with(o, &dict("k", 2)),
        1,
        "B's face as the notch wall"
    );
    assert_eq!(edges_with_any(o), 0, "result edges are rebuilt fresh");

    // Undo brings the operands back with their own dictionaries.
    doc.undo().unwrap();
    assert_eq!(obj(&doc, a).face_attrs(a_top), Some(&dict("k", 1)));
    assert_eq!(obj(&doc, a).edge_attrs(a_edge), Some(&dict("e", 7)));
}

#[test]
fn slice_pieces_keep_face_dictionaries_and_lose_edge_ones() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(7))
        .unwrap();
    let plane = Plane::from_point_normal(p(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let ((pos, neg), _) = doc.slice_node(id, &plane).unwrap();
    for piece in [pos, neg] {
        let o = obj(&doc, piece);
        assert_eq!(faces_with(o, &dict("k", 1)), 1, "each piece's top");
        assert_eq!(faces_with_any(o), 1, "the cut faces have none");
        assert_eq!(edges_with_any(o), 0);
    }
}

fn pane_scene() -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs: Vec::<DefRecipe>::new(),
        roots: vec![ImportNode::Mesh(MeshRecipe {
            name: "Pane".into(),
            positions: vec![
                p(0.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(1.0, 1.0, 0.0),
                p(0.0, 1.0, 0.0),
            ],
            faces: vec![vec![0, 1, 2, 3]],
            face_materials: vec![kernel::NO_MATERIAL],
            face_uv_frames: vec![None],
            face_holes: vec![Vec::new()],
            base_material: kernel::NO_MATERIAL,
            tags: Vec::new(),
            extra: Default::default(),
        })],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    }
}

#[test]
fn thickened_copies_take_the_sheets_dictionary_and_walls_have_none() {
    let mut doc = Document::new();
    doc.ingest(pane_scene(), Vec::new()).unwrap();
    let id = doc.live_object_ids()[0];
    let f = obj(&doc, id).faces().keys().next().unwrap();
    doc.face_attr_set(id, f, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.thicken_sheet(id, 0.0, 0.1).unwrap();
    let o = obj(&doc, id);
    assert_eq!(o.faces().len(), 6);
    assert_eq!(faces_with(o, &dict("k", 1)), 2, "both copies");
    assert_eq!(faces_with_any(o), 2, "the walls have none");
}

#[test]
fn retrace_splits_inherit_and_a_retrace_merge_keeps_the_survivors() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let path = vec![p(0.5, 0.0, 1.0), p(0.5, 1.0, 1.0)];
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::RetraceFace {
                face: t,
                insert: vec![RetracePath {
                    path: path.clone(),
                    curves: Vec::new(),
                    polygons: Vec::new(),
                }],
                remove: Vec::new(),
                restore: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceRetrace(r) = report else {
        panic!("retrace report")
    };
    assert_eq!(r.faces.len(), 2);
    for &f in &r.faces {
        assert_eq!(obj(&doc, id).face_attrs(f), Some(&dict("k", 1)));
    }
    // Give the right half its own, then remove the line again.
    let right = face_containing(obj(&doc, id), p(0.75, 0.5, 1.0));
    doc.face_attr_set(id, right, NS, "k", AttrValue::Int(2))
        .unwrap();
    let left = face_containing(obj(&doc, id), p(0.25, 0.5, 1.0));
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::RetraceFace {
                face: left,
                insert: Vec::new(),
                remove: vec![path],
                restore: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceRetrace(r) = report else {
        panic!("retrace report")
    };
    assert_eq!(r.faces.len(), 1);
    let o = obj(&doc, id);
    let merged = o.face_attrs(r.faces[0]).cloned();
    assert!(
        merged == Some(dict("k", 1)) || merged == Some(dict("k", 2)),
        "the survivor keeps its own: {merged:?}"
    );
    assert_eq!(faces_with_any(o), 1);
    // Undo restores each side's.
    doc.undo().unwrap();
    let o = obj(&doc, id);
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.25, 0.5, 1.0))),
        Some(&dict("k", 1))
    );
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.75, 0.5, 1.0))),
        Some(&dict("k", 2))
    );
}

#[test]
fn a_merge_keeps_face_as_and_undo_restores_the_other_side() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let r = split_top(&mut doc, id);
    let left = face_containing(obj(&doc, id), p(0.25, 0.5, 1.0));
    let right = face_containing(obj(&doc, id), p(0.75, 0.5, 1.0));
    doc.face_attr_set(id, left, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.face_attr_set(id, right, NS, "k", AttrValue::Int(2))
        .unwrap();
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::MergeFaces {
                edge: r.new_edges[0],
            },
        )
        .unwrap();
    let KernelOpReport::FaceMerge(m) = report else {
        panic!("merge report")
    };
    let expect = if m.merged_face == left {
        dict("k", 1)
    } else {
        assert_eq!(m.merged_face, right);
        dict("k", 2)
    };
    let o = obj(&doc, id);
    assert_eq!(o.face_attrs(m.merged_face), Some(&expect));
    assert_eq!(faces_with_any(o), 1);

    doc.undo().unwrap();
    let o = obj(&doc, id);
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.25, 0.5, 1.0))),
        Some(&dict("k", 1))
    );
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.75, 0.5, 1.0))),
        Some(&dict("k", 2))
    );
    doc.redo().unwrap();
    assert_eq!(faces_with(obj(&doc, id), &expect), 1);
}

#[test]
fn an_inner_face_dissolve_undoes_to_the_sub_faces_and_loop_edges_own() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let ring = vec![
        p(0.25, 0.25, 1.0),
        p(0.75, 0.25, 1.0),
        p(0.75, 0.75, 1.0),
        p(0.25, 0.75, 1.0),
    ];
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: ring.clone(),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("imprint report")
    };
    doc.face_attr_set(id, r.sub_face, NS, "k", AttrValue::Int(2))
        .unwrap();
    let loop_edge = edge_between(obj(&doc, id), ring[0], ring[1]);
    doc.edge_attr_set(id, loop_edge, NS, "e", AttrValue::Int(7))
        .unwrap();

    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::MergeInnerFace {
                sub_face: r.sub_face,
            },
        )
        .unwrap();
    let KernelOpReport::FaceMergeInner(m) = report else {
        panic!("dissolve report")
    };
    assert!(!m.sub_face_attrs.attrs.is_none());
    assert_eq!(m.edge_attrs.len(), 4);
    let o = obj(&doc, id);
    assert_eq!(o.face_attrs(m.parent), Some(&dict("k", 1)));
    assert_eq!(edges_with_any(o), 0);

    for _ in 0..2 {
        doc.undo().unwrap();
        let o = obj(&doc, id);
        let sub = face_containing(o, p(0.5, 0.5, 1.0));
        assert_eq!(o.face_attrs(sub), Some(&dict("k", 2)));
        assert_eq!(
            o.edge_attrs(edge_between(o, ring[0], ring[1])),
            Some(&dict("e", 7))
        );
        assert_eq!(edges_with_any(o), 1);
        doc.redo().unwrap();
        assert_eq!(edges_with_any(obj(&doc, id)), 0);
    }
}

#[test]
fn seam_cleanup_refuses_unequal_dictionaries_and_merges_equal_ones() {
    for (left_value, right_value, merges) in [(1, 2, 0), (1, 1, 1)] {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
        split_top(&mut doc, id);
        let left = face_containing(obj(&doc, id), p(0.25, 0.5, 1.0));
        let right = face_containing(obj(&doc, id), p(0.75, 0.5, 1.0));
        doc.face_attr_set(id, left, NS, "k", AttrValue::Int(left_value))
            .unwrap();
        doc.face_attr_set(id, right, NS, "k", AttrValue::Int(right_value))
            .unwrap();
        let mut o = obj(&doc, id).clone();
        let before = o.faces().len();
        assert_eq!(o.merge_coplanar_faces(&[]), merges);
        assert_eq!(o.faces().len(), before - merges);
        o.validate().unwrap();
    }
}

#[test]
fn undo_redo_proof_restores_dictionaries_on_both_fragments() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    split_top(&mut doc, id);
    let left = face_containing(obj(&doc, id), p(0.25, 0.5, 1.0));
    doc.face_attr_set(id, left, NS, "k", AttrValue::Int(2))
        .unwrap();
    let right = face_containing(obj(&doc, id), p(0.75, 0.5, 1.0));
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: right,
            distance: 0.25,
        },
    )
    .unwrap();
    let check = |doc: &Document, right_z: f64| {
        let o = obj(doc, id);
        assert_eq!(
            o.face_attrs(face_containing(o, p(0.25, 0.5, 1.0))),
            Some(&dict("k", 2))
        );
        assert_eq!(
            o.face_attrs(face_containing(o, p(0.75, 0.5, right_z))),
            Some(&dict("k", 1))
        );
        assert_eq!(faces_with_any(o), 2);
    };
    check(&doc, 1.25);
    let hash = doc.state_hash();
    doc.undo().unwrap();
    check(&doc, 1.0);
    doc.undo().unwrap(); // the write on the left half
    doc.undo().unwrap(); // the split
    assert_eq!(
        obj(&doc, id).face_attrs(top(obj(&doc, id), 1.0)),
        Some(&dict("k", 1))
    );
    doc.redo().unwrap();
    doc.redo().unwrap();
    check(&doc, 1.0);
    doc.redo().unwrap();
    check(&doc, 1.25);
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn clone_sites_carry_dictionaries() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(7))
        .unwrap();
    let carries = |doc: &Document, o: ObjectId| {
        let o = obj(doc, o);
        assert_eq!(faces_with(o, &dict("k", 1)), 1);
        assert_eq!(edges_with_any(o), 1);
    };

    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(id),
            &Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
            kernel::LinkPolicy::Independent,
        )
        .unwrap();
    let NodeId::Object(copy) = copy else {
        panic!("an object copy")
    };
    carries(&doc, copy);

    let (c, first, _) = doc.make_component(&[NodeId::Object(id)]).unwrap();
    let (second, _) = doc
        .place_instance(c, Transform::translation(Vec3::new(0.0, 3.0, 0.0)))
        .unwrap();
    let (unique, _) = doc.make_unique(second).unwrap();
    assert_ne!(unique, c);
    let (exploded, _) = doc.explode_instance(first).unwrap();
    assert_eq!(exploded.len(), 1);
    carries(&doc, exploded[0]);
    // The unique definition's member, through a second explode.
    let placements: Vec<_> = doc
        .instance_ids()
        .into_iter()
        .filter(|&i| doc.instance_def(i) == Some(unique))
        .collect();
    let (exploded, _) = doc.explode_instance(placements[0]).unwrap();
    carries(&doc, exploded[0]);
}

// ---------------------------------------------------------------- format

/// Every face's and edge's dictionary, keyed by position, for comparing
/// two Objects whose handles differ.
/// Dictionaries keyed by a position string, sorted.
type Keyed = Vec<(String, AttrDict)>;

fn dictionaries(o: &Object) -> (Keyed, Keyed) {
    let key = |q: Point3| {
        let r = |v: f64| (v * 1e6).round() as i64;
        format!("{},{},{}", r(q.x), r(q.y), r(q.z))
    };
    let mut faces: Keyed = o
        .faces()
        .keys()
        .filter_map(|f| Some((key(centroid(o, f)), o.face_attrs(f)?.clone())))
        .collect();
    let mut edges: Keyed = o
        .edges()
        .keys()
        .filter_map(|e| {
            let (a, b) = o.edge_endpoints(e).unwrap();
            let (a, b) = (key(a), key(b));
            let k = if a < b { a + "|" + &b } else { b + "|" + &a };
            Some((k, o.edge_attrs(e)?.clone()))
        })
        .collect();
    faces.sort_by(|x, y| x.0.cmp(&y.0));
    edges.sort_by(|x, y| x.0.cmp(&y.0));
    (faces, edges)
}

fn decorated() -> (Document, ObjectId) {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let o = obj(&doc, id);
    let t = top(o, 1.0);
    let bottom = face_where(o, Vec3::new(0.0, 0.0, -1.0), 0.0);
    let front = face_where(o, Vec3::new(0.0, -1.0, 0.0), 0.0);
    let e = edge_between(o, p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    let v = edge_between(o, p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let nested = AttrValue::from_json(&serde_json::json!({
        "part": "Shelf", "grain": [1, 0.5, null, true], "deep": {"x": "y"}
    }))
    .unwrap();
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.face_attr_set(id, bottom, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.face_attr_set(id, front, "org.other", "n", nested)
        .unwrap();
    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(7))
        .unwrap();
    doc.edge_attr_set(id, v, NS, "e", AttrValue::Float(0.25))
        .unwrap();
    (doc, id)
}

#[test]
fn save_load_round_trips_face_and_edge_dictionaries() {
    let (doc, id) = decorated();
    let bytes = doc.save();
    assert_eq!(bytes, doc.save(), "save is deterministic");
    let loaded = Document::load(&bytes).unwrap();
    let lid = loaded.visible_object_ids()[0];
    assert_eq!(dictionaries(obj(&loaded, lid)), dictionaries(obj(&doc, id)));
    assert_eq!(loaded.state_hash(), doc.state_hash());
    assert_eq!(
        loaded.save(),
        bytes,
        "a reopened document saves the same bytes"
    );

    // The object alone, through its geometry buffer.
    let o = obj(&doc, id);
    let buf = o.encode(&|_| 0);
    assert_eq!(&buf[4..8], &kernel::GEOMETRY_FORMAT_VERSION.to_le_bytes());
    let back = Object::decode(&buf, &|_| None).unwrap();
    assert_eq!(dictionaries(&back), dictionaries(o));
}

#[test]
fn the_writer_records_geometry_v9_and_manifest_v26() {
    use std::io::Read as _;
    assert_eq!(kernel::GEOMETRY_FORMAT_VERSION, 9);
    assert_eq!(kernel::MANIFEST_FORMAT_VERSION, 26);
    let (doc, _) = decorated();
    let bytes = doc.save();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut manifest = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let m: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(m["format_version"], 26);
    assert_eq!(m["geometry_version"], 9);
}

/// The first occurrence of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len())
        .position(|w| w == needle)
        .expect("needle present")
}

fn one_edge_dict_object() -> Object {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(7))
        .unwrap();
    obj(&doc, id).clone()
}

#[test]
fn a_shared_edge_whose_faces_disagree_is_rejected() {
    let o = one_edge_dict_object();
    let mut buf = o.encode(&|_| 0);
    let json = br#"{"com.example.t":{"e":7}}"#;
    // Written once by each of the edge's two faces.
    let at = find(&buf, json);
    assert!(find(&buf[at + 1..], json) > 0);
    let seven = at + json.len() - 3;
    assert_eq!(buf[seven], b'7');
    buf[seven] = b'8';
    assert!(matches!(
        Object::decode(&buf, &|_| None),
        Err(DecodeError::Corrupt { .. })
    ));

    // A malformed dictionary on one side is rejected as well.
    let mut buf = o.encode(&|_| 0);
    let at = find(&buf, json);
    buf[at + json.len() - 3] = b'x';
    assert!(matches!(
        Object::decode(&buf, &|_| None),
        Err(DecodeError::Corrupt { .. })
    ));
}

#[test]
fn pre_v9_files_load_without_dictionaries() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/follow-me-2.hew");
    let doc = Document::load(&std::fs::read(path).unwrap()).unwrap();
    for id in doc.live_object_ids() {
        let o = obj(&doc, id);
        assert_eq!(faces_with_any(o), 0);
        assert_eq!(edges_with_any(o), 0);
    }
    // And saves at the current versions, reopening unchanged.
    let again = Document::load(&doc.save()).unwrap();
    assert_eq!(again.state_hash(), doc.state_hash());
}

#[test]
fn state_hash_moves_only_with_a_face_or_edge_dictionary() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let bare = doc.state_hash();
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let with_face = doc.state_hash();
    assert_ne!(with_face, bare);
    doc.face_attr_delete(id, t, NS, None).unwrap();
    assert_eq!(doc.state_hash(), bare, "back to the bare hash");
    let e = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, e, NS, "e", AttrValue::Int(7))
        .unwrap();
    assert_ne!(doc.state_hash(), bare);
    assert_ne!(doc.state_hash(), with_face);
}

/// Hand-encodes a v9 geometry buffer (HEW_FILE_FORMAT.md §3.1) of
/// quad faces with an optional face dictionary each; no edge carries one.
fn craft_v9(positions: &[[f64; 3]], faces: &[(Vec<u32>, Option<&[u8]>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"HEWG");
    b.extend_from_slice(&9u32.to_le_bytes());
    b.push(1u8); // watertight
    b.extend_from_slice(&kernel::NO_MATERIAL.to_le_bytes());
    b.push(0u8); // native
    b.extend_from_slice(&(positions.len() as u32).to_le_bytes());
    for q in positions {
        for c in q {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    b.extend_from_slice(&(faces.len() as u32).to_le_bytes());
    for (outer, attrs) in faces {
        b.extend_from_slice(&kernel::NO_MATERIAL.to_le_bytes());
        b.push(0u8); // uv frame
        b.push(0u8); // surface
        b.extend_from_slice(&(outer.len() as u32).to_le_bytes());
        for i in outer {
            b.extend_from_slice(&i.to_le_bytes());
        }
        b.extend_from_slice(&0u32.to_le_bytes()); // holes
        b.extend_from_slice(&[0u8, 0, 0, 0]); // soft, curves, polygons, back paint
        match attrs {
            None => b.push(0u8),
            Some(json) => {
                b.push(1u8);
                b.extend_from_slice(&(json.len() as u32).to_le_bytes());
                b.extend_from_slice(json);
            }
        }
        b.push(0u8); // edge dictionaries
    }
    b
}

fn cube_faces(base: u32, top: Option<&[u8]>) -> Vec<(Vec<u32>, Option<&[u8]>)> {
    let idx = |l: [u32; 4]| l.iter().map(|&i| base + i).collect::<Vec<u32>>();
    vec![
        (idx([0, 3, 2, 1]), None),
        (idx([4, 5, 6, 7]), top),
        (idx([0, 1, 5, 4]), None),
        (idx([1, 2, 6, 5]), None),
        (idx([2, 3, 7, 6]), None),
        (idx([3, 0, 4, 7]), None),
    ]
}

#[test]
fn coincident_faces_are_ordered_by_their_dictionaries() {
    // Two coincident cubes on disjoint shells tie on every other payload;
    // only the top face's dictionary tells them apart. Whichever order the
    // input presents them in, the writer emits the same bytes.
    let cube = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    let positions: Vec<[f64; 3]> = cube.iter().chain(cube.iter()).copied().collect();
    let one: &[u8] = br#"{"com.example.t":{"k":1}}"#;
    let two: &[u8] = br#"{"com.example.t":{"k":2}}"#;
    let mut a = cube_faces(0, Some(one));
    a.extend(cube_faces(8, Some(two)));
    let mut b = cube_faces(8, Some(two));
    b.extend(cube_faces(0, Some(one)));
    let decode = |bytes: &[u8]| {
        let o = Object::decode(bytes, &|_| None).expect("crafted buffer decodes");
        o.validate().unwrap();
        o
    };
    let (oa, ob) = (
        decode(&craft_v9(&positions, &a)),
        decode(&craft_v9(&positions, &b)),
    );
    assert_eq!(faces_with(&oa, &dict("k", 1)), 1);
    assert_eq!(faces_with(&oa, &dict("k", 2)), 1);
    assert_eq!(oa.encode(&|_| 0), ob.encode(&|_| 0));
}

#[test]
fn malformed_dictionaries_are_rejected_on_load() {
    let cube = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    for bad in [
        &br#"{}"#[..],
        br#"{"":{"k":1}}"#,
        br#"{"com.example.t":{}}"#,
        br#"{"com.example.t":{"":1}}"#,
        br#"{"com.example.t":{"k":18446744073709551615}}"#,
        br#"[1]"#,
        br#"not json"#,
    ] {
        let bytes = craft_v9(&cube, &cube_faces(0, Some(bad)));
        assert!(
            matches!(
                Object::decode(&bytes, &|_| None),
                Err(DecodeError::Corrupt { .. })
            ),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
    // A length past the end of the buffer is truncation, not a panic.
    let mut bytes = craft_v9(&cube, &cube_faces(0, Some(br#"{"a":{"b":1}}"#)));
    let at = find(&bytes, br#"{"a":{"b":1}}"#) - 4;
    bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Object::decode(&bytes, &|_| None).is_err());
}

// ------------------------------------------- remaining propagation rows

/// F7: pushing one half of a split top up builds a wall on the seam (the
/// coplanar-sibling wall). The wall is new and has none; the moved half
/// keeps its own, the sibling keeps its own, and the seam's edge, which
/// stays with the sibling, keeps its dictionary.
#[test]
fn a_coplanar_sibling_wall_has_none() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let r = split_top(&mut doc, id);
    let left = face_containing(obj(&doc, id), p(0.25, 0.5, 1.0));
    let right = face_containing(obj(&doc, id), p(0.75, 0.5, 1.0));
    doc.face_attr_set(id, left, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.face_attr_set(id, right, NS, "k", AttrValue::Int(2))
        .unwrap();
    doc.edge_attr_set(id, r.new_edges[0], NS, "e", AttrValue::Int(7))
        .unwrap();
    let faces_before = obj(&doc, id).faces().len();
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: left,
            distance: 0.5,
        },
    )
    .unwrap();
    let o = obj(&doc, id);
    assert!(o.faces().len() > faces_before, "a wall was built");
    let wall = face_where(o, Vec3::new(1.0, 0.0, 0.0), 0.5);
    assert_eq!(o.face_attrs(wall), None, "the seam wall is new");
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.25, 0.5, 1.5))),
        Some(&dict("k", 1))
    );
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.75, 0.5, 1.0))),
        Some(&dict("k", 2))
    );
    assert_eq!(faces_with_any(o), 2);
    let seam = edge_between(o, p(0.5, 0.0, 1.0), p(0.5, 1.0, 1.0));
    assert_eq!(
        o.edge_attrs(seam),
        Some(&dict("e", 7)),
        "the sibling's edge keeps it"
    );
    assert_eq!(edges_with_any(o), 1, "the wall's new edges have none");
}

/// F9: a cut with both ends on one hole of a face cuts off a lune. The
/// lune and the face that keeps the hole both carry the face's dictionary.
#[test]
fn a_lune_cut_off_around_a_hole_inherits() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    doc.apply_object_op(
        id,
        KernelOp::SplitFaceInner {
            face: t,
            loop_path: vec![
                p(0.4, 0.4, 1.0),
                p(0.6, 0.4, 1.0),
                p(0.6, 0.6, 1.0),
                p(0.4, 0.6, 1.0),
            ],
            restore: None,
            curves: Vec::new(),
            edge_attrs: Vec::new(),
        },
    )
    .unwrap();
    let holed = obj(&doc, id)
        .faces()
        .iter()
        .find(|(_, f)| !f.inner_loops.is_empty())
        .map(|(f, _)| f)
        .unwrap();
    // From the hole's left side out and around to its bottom side.
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFace {
                face: holed,
                path: vec![
                    p(0.4, 0.5, 1.0),
                    p(0.2, 0.5, 1.0),
                    p(0.2, 0.2, 1.0),
                    p(0.5, 0.2, 1.0),
                    p(0.5, 0.4, 1.0),
                ],
                restore: None,
                curves: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplit(r) = report else {
        panic!("split report")
    };
    let o = obj(&doc, id);
    for f in r.new_faces {
        assert_eq!(o.face_attrs(f), Some(&dict("k", 1)));
    }
    assert!(r.new_edges.iter().all(|&e| o.edge_attrs(e).is_none()));
    // The sub-face, and the two halves of the old top: three faces.
    assert_eq!(faces_with(o, &dict("k", 1)), 3);
}

/// F3: capping a missing lid adds a face with no dictionary; the faces
/// and edges the box already had keep theirs.
#[test]
fn a_cap_has_none_and_the_rim_keeps_its_own() {
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
    // The box without its top.
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let n = faces.len();
    let mut scene = pane_scene();
    scene.roots = vec![ImportNode::Mesh(MeshRecipe {
        name: "Open box".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    })];
    let mut doc = Document::new();
    doc.ingest(scene, Vec::new()).unwrap();
    let id = doc.live_object_ids()[0];
    let front = face_where(obj(&doc, id), Vec3::new(0.0, -1.0, 0.0), 0.0);
    doc.face_attr_set(id, front, NS, "k", AttrValue::Int(1))
        .unwrap();
    let rim = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0));
    doc.edge_attr_set(id, rim, NS, "e", AttrValue::Int(7))
        .unwrap();
    let (report, _) = doc.cap_rims(id, None).unwrap();
    assert!(report.closed);
    let o = obj(&doc, id);
    assert_eq!(o.face_attrs(top(o, 1.0)), None, "the cap is new");
    assert_eq!(o.face_attrs(front), Some(&dict("k", 1)));
    assert_eq!(faces_with_any(o), 1);
    assert_eq!(
        o.edge_attrs(rim),
        Some(&dict("e", 7)),
        "the rim edge is kept"
    );
    assert_eq!(edges_with_any(o), 1);
}

/// B7: a Follow Me sweep around a face loop builds a new solid from
/// nothing: none of its faces or edges carries a dictionary, and the path
/// solid keeps its own.
#[test]
fn a_follow_me_solid_has_none() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 4.0, 4.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let rim = edge_between(obj(&doc, id), p(0.0, 0.0, 1.0), p(4.0, 0.0, 1.0));
    doc.edge_attr_set(id, rim, NS, "e", AttrValue::Int(7))
        .unwrap();
    // A standing profile on the y = 0 rim, straddling the top edge.
    let plane = Plane::from_point_normal(p(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let ps = doc.add_sketch(plane);
    let c = [
        p(2.0, -0.2, 0.8),
        p(2.0, 0.2, 0.8),
        p(2.0, 0.2, 1.2),
        p(2.0, -0.2, 1.2),
    ];
    let sk = doc.sketch_mut(ps).unwrap();
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).unwrap();
    }
    let region = doc.extrudable_regions(ps).unwrap()[0];
    let (swept, _) = doc
        .follow_me(
            ps,
            region,
            &kernel::FollowMePath::FaceLoop {
                object: id,
                face: t,
            },
        )
        .unwrap();
    let o = obj(&doc, swept);
    assert!(o.faces().len() > 4);
    assert_eq!(faces_with_any(o), 0);
    assert_eq!(edges_with_any(o), 0);
    let base = obj(&doc, id);
    assert_eq!(base.face_attrs(t), Some(&dict("k", 1)));
    assert_eq!(base.edge_attrs(rim), Some(&dict("e", 7)));
}

/// M3: merging a face into the hole it sits in keeps the hole's owner and
/// its dictionary; the dissolved face's comes back on undo.
#[test]
fn a_merge_into_a_hole_keeps_the_owners_and_undo_restores_the_inner() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    doc.face_attr_set(id, t, NS, "k", AttrValue::Int(1))
        .unwrap();
    let ring = vec![
        p(0.25, 0.25, 1.0),
        p(0.75, 0.25, 1.0),
        p(0.75, 0.75, 1.0),
        p(0.25, 0.75, 1.0),
    ];
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: ring.clone(),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("imprint report")
    };
    // Cut the imprint in two, so its left half sits in the hole sharing
    // only part of the hole's boundary.
    doc.apply_object_op(
        id,
        KernelOp::SplitFace {
            face: r.sub_face,
            path: vec![p(0.5, 0.25, 1.0), p(0.5, 0.75, 1.0)],
            restore: None,
            curves: Vec::new(),
        },
    )
    .unwrap();
    let left = face_containing(obj(&doc, id), p(0.375, 0.5, 1.0));
    doc.face_attr_set(id, left, NS, "k", AttrValue::Int(2))
        .unwrap();
    let side = edge_between(obj(&doc, id), ring[3], ring[0]);
    let (report, _) = doc
        .apply_object_op(id, KernelOp::MergeFaces { edge: side })
        .unwrap();
    let KernelOpReport::FaceMerge(m) = report else {
        panic!("merge report")
    };
    assert_eq!(m.merged_face, r.parent, "the hole's owner survives");
    let o = obj(&doc, id);
    assert_eq!(o.face_attrs(m.merged_face), Some(&dict("k", 1)));
    assert_eq!(
        faces_with(o, &dict("k", 2)),
        0,
        "the inner face's is dropped"
    );
    doc.undo().unwrap();
    let o = obj(&doc, id);
    assert_eq!(
        o.face_attrs(face_containing(o, p(0.375, 0.5, 1.0))),
        Some(&dict("k", 2)),
        "undo restores the inner face's"
    );
}

/// M8: flattening a boss reuses the shared loop edge, which keeps its
/// dictionary; the boss walls and their edges go.
#[test]
fn collapsing_a_boss_keeps_the_loop_edges_dictionary() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let t = top(obj(&doc, id), 1.0);
    let ring = vec![
        p(0.25, 0.25, 1.0),
        p(0.75, 0.25, 1.0),
        p(0.75, 0.75, 1.0),
        p(0.25, 0.75, 1.0),
    ];
    let (report, _) = doc
        .apply_object_op(
            id,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: ring.clone(),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = report else {
        panic!("imprint report")
    };
    let loop_edge = edge_between(obj(&doc, id), ring[0], ring[1]);
    doc.edge_attr_set(id, loop_edge, NS, "e", AttrValue::Int(7))
        .unwrap();
    doc.face_attr_set(id, r.sub_face, NS, "k", AttrValue::Int(2))
        .unwrap();
    doc.apply_object_op(
        id,
        KernelOp::ExtrudeSubFace {
            sub_face: r.sub_face,
            distance: 0.5,
        },
    )
    .unwrap();
    assert_eq!(
        edges_with_any(obj(&doc, id)),
        1,
        "the boss keeps one edge's"
    );
    doc.apply_object_op(
        id,
        KernelOp::CollapseSubFace {
            sub_face: r.sub_face,
        },
    )
    .unwrap();
    let o = obj(&doc, id);
    assert_eq!(
        o.edge_attrs(edge_between(o, ring[0], ring[1])),
        Some(&dict("e", 7)),
        "the shared loop edge keeps it"
    );
    assert_eq!(edges_with_any(o), 1);
    assert_eq!(o.face_attrs(r.sub_face), Some(&dict("k", 2)));
    assert_eq!(faces_with_any(o), 1, "no wall is left to carry anything");
}
