//! Acceptance specs for Model Health's "Rejoin as one solid"
//! (`Document::rejoin_as_one_solid`): a seam family (the pieces an
//! importer split one shape into, and the inside walls between them) is
//! capped at its shared rims, its walls are deleted, and its pieces are
//! unioned into one watertight Object, as one undo entry, keeping the
//! largest piece's name, tags, attributes, and group.
//!
//! The fixtures are ingested meshes (a solid refuses `delete_faces`, so a
//! split box cannot be carved out of an extruded one): open boxes that
//! share a rim at each cut, with a flat partition sheet in the cut.

use kernel::{
    AttrTarget, AttrValue, BooleanOp, Document, DocumentError, EntityRef, ImportNode, ImportScene,
    MeshRecipe, NodeId, ObjectId, Point3, WatertightState,
};
use proptest::prelude::*;

fn scene(roots: Vec<ImportNode>) -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots,
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    }
}

fn mesh(name: &str, positions: Vec<Point3>, faces: Vec<Vec<usize>>) -> MeshRecipe {
    let n = faces.len();
    MeshRecipe {
        name: name.into(),
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

/// The box `[x0, x1] × [0, d] × [0, h]` without every face in `drops`
/// (0 bottom, 1 top, 2 the y = 0 side, 3 the x = x1 side, 4 the y = d
/// side, 5 the x = x0 side).
fn piece(name: &str, x0: f64, x1: f64, d: f64, h: f64, drops: &[usize]) -> MeshRecipe {
    let p = Point3::new;
    let positions = vec![
        p(x0, 0.0, 0.0),
        p(x1, 0.0, 0.0),
        p(x1, d, 0.0),
        p(x0, d, 0.0),
        p(x0, 0.0, h),
        p(x1, 0.0, h),
        p(x1, d, h),
        p(x0, d, h),
    ];
    let faces: Vec<Vec<usize>> = [
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
    .into_iter()
    .enumerate()
    .filter(|(i, _)| !drops.contains(i))
    .map(|(_, f)| f)
    .collect();
    mesh(name, positions, faces)
}

/// A unit-deep, unit-high piece.
fn half(name: &str, x0: f64, x1: f64, drops: &[usize]) -> MeshRecipe {
    piece(name, x0, x1, 1.0, 1.0, drops)
}

/// The flat sheet in the cut at `x`, `d` deep and `h` high.
fn wall_at(name: &str, x: f64, d: f64, h: f64) -> MeshRecipe {
    let p = Point3::new;
    mesh(
        name,
        vec![p(x, 0.0, 0.0), p(x, d, 0.0), p(x, d, h), p(x, 0.0, h)],
        vec![vec![0, 1, 2, 3]],
    )
}

fn wall(x: f64) -> MeshRecipe {
    wall_at("Wall", x, 1.0, 1.0)
}

fn by_name(doc: &Document, name: &str) -> ObjectId {
    let found: Vec<ObjectId> = doc
        .live_object_ids()
        .into_iter()
        .filter(|&o| doc.object_name(o) == Some(name))
        .collect();
    assert_eq!(found.len(), 1, "exactly one live object named {name}");
    found[0]
}

fn ingest(roots: Vec<ImportNode>) -> Document {
    let mut doc = Document::new();
    doc.ingest(scene(roots), Vec::new()).unwrap();
    doc
}

/// The box from x = 0 to 2 split at x = 1: Left, Right, and the Wall.
fn split_box() -> Document {
    ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
        ImportNode::Mesh(wall(1.0)),
    ])
}

fn family(doc: &Document) -> Vec<ObjectId> {
    let report = doc.health_report(None);
    assert_eq!(report.families.len(), 1, "the fixture is one family");
    report.families[0].members.clone()
}

fn volume(doc: &Document, o: ObjectId) -> f64 {
    doc.object(o).unwrap().volume().expect("a solid")
}

fn assert_one_solid(doc: &Document, o: ObjectId, want_volume: f64) {
    let obj = doc.object(o).expect("the result is live");
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    assert!(
        (volume(doc, o) - want_volume).abs() < 1e-9,
        "volume {} is the pieces' sum {want_volume}",
        volume(doc, o)
    );
    assert_eq!(doc.live_object_ids(), vec![o], "one Object remains");
    assert!(doc.health_report(None).rows.is_empty(), "nothing is open");
}

#[test]
fn two_halves_and_a_wall_rejoin_to_one_watertight_solid() {
    let mut doc = split_box();
    let members = family(&doc);
    assert_eq!(members.len(), 3);
    let (result, change) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_one_solid(&doc, result, 2.0);
    assert!(change.objects_touched.contains(&result));
    // The boolean dissolves the seams it made: one box, six faces, one
    // connected shell.
    assert_eq!(doc.object(result).unwrap().faces().len(), 6);
    assert_eq!(shell_count(&doc, result), 1);
}

#[test]
fn two_touching_halves_with_no_wall_rejoin_to_one_solid() {
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
    ]);
    let members = family(&doc);
    assert_eq!(members.len(), 2);
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_one_solid(&doc, result, 2.0);
}

#[test]
fn three_cells_rejoin_largest_first() {
    // Big is in the middle: it is the first operand, and each neighbor
    // touches it.
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Small", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Big", 1.0, 3.5, &[3, 5])),
        ImportNode::Mesh(half("Medium", 3.5, 5.5, &[5])),
        ImportNode::Mesh(wall(1.0)),
        ImportNode::Mesh(wall_at("Wall 2", 3.5, 1.0, 1.0)),
    ]);
    let members = family(&doc);
    assert_eq!(members.len(), 5);
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_one_solid(&doc, result, 5.5);
    assert_eq!(doc.object_name(result), Some("Big"));
    assert_eq!(doc.object(result).unwrap().faces().len(), 6, "one box");

    // The smallest piece in the middle: the two larger ones meet only
    // through it, so the first union is of two pieces that do not touch.
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Medium", 0.0, 2.0, &[3])),
        ImportNode::Mesh(half("Small", 2.0, 2.5, &[3, 5])),
        ImportNode::Mesh(half("Big", 2.5, 5.5, &[5])),
        ImportNode::Mesh(wall(2.0)),
        ImportNode::Mesh(wall_at("Wall 2", 2.5, 1.0, 1.0)),
    ]);
    let members = family(&doc);
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_one_solid(&doc, result, 5.5);
    assert_eq!(doc.object_name(result), Some("Big"));
    assert_eq!(doc.object(result).unwrap().faces().len(), 6, "one box");
}

#[test]
fn the_result_keeps_the_largest_cells_name_tags_and_attributes() {
    let mut big = half("Big", 0.0, 1.5, &[3]);
    big.tags = vec![vec!["Wood".to_string()]];
    let mut small = half("Small", 1.5, 2.0, &[5]);
    small.tags = vec![vec!["Metal".to_string()]];
    // Listed smallest first: the order of `members` does not decide.
    let mut doc = ingest(vec![
        ImportNode::Mesh(small),
        ImportNode::Mesh(wall(1.5)),
        ImportNode::Mesh(big),
    ]);
    let (b, s) = (by_name(&doc, "Big"), by_name(&doc, "Small"));
    let target = |o| AttrTarget::Entity(EntityRef::Object(o));
    doc.attr_set(target(b), "shop", "species", AttrValue::Text("oak".into()))
        .unwrap();
    doc.attr_set(target(s), "shop", "finish", AttrValue::Text("oil".into()))
        .unwrap();
    doc.attr_set(target(s), "other", "k", AttrValue::Bool(true))
        .unwrap();

    let (result, _) = doc.rejoin_as_one_solid(&family(&doc)).unwrap();
    assert_one_solid(&doc, result, 2.0);
    assert_eq!(doc.object_name(result), Some("Big"));
    assert_eq!(
        doc.node_tags(NodeId::Object(result)),
        &[vec!["Wood".to_string()]]
    );
    let attrs = doc.attr_get(&target(result)).unwrap().expect("attributes");
    assert_eq!(
        attrs.get("shop").and_then(|ns| ns.get("species")),
        Some(&AttrValue::Text("oak".into()))
    );
    assert!(attrs.get("shop").and_then(|ns| ns.get("finish")).is_none());
    assert!(
        attrs.get("other").is_none(),
        "nothing of the smaller piece's"
    );
}

#[test]
fn the_result_keeps_the_cells_group_parent() {
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Outside", 5.0, 6.0, &[])),
        ImportNode::Group {
            name: "Board".into(),
            children: vec![
                ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
                ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
                ImportNode::Mesh(wall(1.0)),
            ],
            tags: Vec::new(),
            hidden: false,
            key: None,
        },
    ]);
    let left = by_name(&doc, "Left");
    let group = doc.node_parent(NodeId::Object(left)).expect("grouped");
    let (result, _) = doc.rejoin_as_one_solid(&family(&doc)).unwrap();
    assert_eq!(doc.node_parent(NodeId::Object(result)), Some(group));
    assert_eq!(
        doc.group_members(group),
        Some(vec![NodeId::Object(result)]),
        "the group holds the one solid"
    );
    let outside = by_name(&doc, "Outside");
    assert_eq!(doc.node_parent(NodeId::Object(outside)), None);
    assert!((volume(&doc, result) - 2.0).abs() < 1e-9);
}

#[test]
fn one_undo_entry_restores_every_piece_and_the_wall_exactly() {
    let mut doc = split_box();
    let members = family(&doc);
    let before = doc.state_hash();
    let depth = doc.undo_depth();
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(
        doc.peek_undo_meta().map(|m| m.label.as_str()),
        Some("Rejoin as one solid")
    );
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before, "exactly as before");
    assert_eq!(doc.undo_depth(), depth);
    assert!(doc.object(result).is_none(), "the solid is gone");
    for m in &members {
        assert!(doc.object(*m).is_some(), "every member is back");
    }
    assert_eq!(family(&doc).len(), 3, "the family reads as it did");
}

#[test]
fn redo_reproduces_the_post_state_hash() {
    let mut doc = split_box();
    let members = family(&doc);
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    let after = doc.state_hash();
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    assert_one_solid(&doc, result, 2.0);
}

/// Two pieces split by a bent partition (in at x = 1 at the bottom and
/// the top, out to x = 1.3 halfway up): their shared rim is not planar,
/// so it cannot take a cap.
fn bent_split() -> Document {
    let p = Point3::new;
    let left = mesh(
        "Left",
        vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.3, 0.0, 0.5),
            p(1.0, 0.0, 1.0),
            p(0.0, 0.0, 1.0),
            p(0.0, 1.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(1.3, 1.0, 0.5),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ],
        vec![
            vec![0, 1, 2, 3, 4],
            vec![9, 8, 7, 6, 5],
            vec![0, 5, 6, 1],
            vec![4, 3, 8, 9],
            vec![0, 4, 9, 5],
        ],
    );
    let right = mesh(
        "Right",
        vec![
            p(1.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.3, 0.0, 0.5),
            p(1.0, 1.0, 0.0),
            p(2.0, 1.0, 0.0),
            p(2.0, 1.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(1.3, 1.0, 0.5),
        ],
        vec![
            vec![0, 1, 2, 3, 4],
            vec![9, 8, 7, 6, 5],
            vec![0, 5, 6, 1],
            vec![3, 2, 7, 8],
            vec![1, 6, 7, 2],
        ],
    );
    ingest(vec![ImportNode::Mesh(left), ImportNode::Mesh(right)])
}

#[test]
fn a_family_with_an_uncappable_seam_refuses_untouched() {
    let mut doc = bent_split();
    let members = family(&doc);
    assert_eq!(members.len(), 2);
    let (before, depth) = (doc.state_hash(), doc.undo_depth());
    let err = doc.rejoin_as_one_solid(&members).unwrap_err();
    assert!(
        matches!(err, DocumentError::Repair(_)),
        "the cap's refusal passes through typed: {err:?}"
    );
    assert_eq!(doc.state_hash(), before);
    assert_eq!(doc.undo_depth(), depth);
    assert!(members.iter().all(|&m| doc.object(m).is_some()));
}

#[test]
fn members_from_two_families_refuse_rejoin_not_possible() {
    let far = |name: &str, x0: f64, x1: f64, drop: usize| {
        let mut m = half(name, x0, x1, &[drop]);
        m.positions.iter_mut().for_each(|q| q.y += 5.0);
        m
    };
    let (far_left, far_right) = (far("Far left", 0.0, 1.0, 3), far("Far right", 1.0, 2.0, 5));
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
        ImportNode::Mesh(far_left),
        ImportNode::Mesh(far_right),
    ]);
    assert_eq!(doc.health_report(None).families.len(), 2);
    let members: Vec<ObjectId> = ["Left", "Right", "Far left", "Far right"]
        .iter()
        .map(|n| by_name(&doc, n))
        .collect();
    let (before, depth) = (doc.state_hash(), doc.undo_depth());
    assert_eq!(
        doc.rejoin_as_one_solid(&members).unwrap_err(),
        DocumentError::RejoinNotPossible
    );
    assert_eq!((doc.state_hash(), doc.undo_depth()), (before, depth));
    // A member that is in no family (a solid) refuses the same way.
    let solid = {
        let mut d = ingest(vec![
            ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
            ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
            ImportNode::Mesh(half("Solid", 5.0, 6.0, &[])),
        ]);
        let ids: Vec<ObjectId> = ["Left", "Right", "Solid"]
            .iter()
            .map(|n| by_name(&d, n))
            .collect();
        d.rejoin_as_one_solid(&ids).unwrap_err()
    };
    assert_eq!(solid, DocumentError::RejoinNotPossible);
}

#[test]
fn a_single_cell_refuses_rejoin_not_possible() {
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(wall(1.0)),
    ]);
    let members = family(&doc);
    assert_eq!(members.len(), 2, "a piece and its wall are a family");
    let (before, depth) = (doc.state_hash(), doc.undo_depth());
    assert_eq!(
        doc.rejoin_as_one_solid(&members).unwrap_err(),
        DocumentError::RejoinNotPossible
    );
    assert_eq!((doc.state_hash(), doc.undo_depth()), (before, depth));
}

#[test]
fn a_shape_drawn_with_an_opening_refuses_because_the_union_stays_open() {
    // A sleeve open at both ends, split at x = 1: capping the seam leaves
    // each half open at its far end.
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3, 5])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[3, 5])),
        ImportNode::Mesh(wall(1.0)),
    ]);
    let members = family(&doc);
    let (before, depth) = (doc.state_hash(), doc.undo_depth());
    assert_eq!(
        doc.rejoin_as_one_solid(&members).unwrap_err(),
        DocumentError::RejoinNotPossible
    );
    assert_eq!((doc.state_hash(), doc.undo_depth()), (before, depth));
    assert!(members.iter().all(|&m| doc.object(m).is_some()));
}

#[test]
fn the_union_of_exactly_touching_halves_is_not_degenerate_contact() {
    // Addendum B7 said two halves that exactly touch refuse as
    // `DegenerateContact`; the boolean resolves coplanar contact instead.
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
    ]);
    let (l, r) = (by_name(&doc, "Left"), by_name(&doc, "Right"));
    doc.cap_rims(l, None).unwrap();
    doc.cap_rims(r, None).unwrap();
    let (result, _) = doc
        .boolean(BooleanOp::Union, l, r)
        .expect("exactly touching solids union");
    assert_one_solid(&doc, result, 2.0);
}

#[test]
fn cells_under_different_parents_refuse_rejoin_not_possible() {
    let mut doc = ingest(vec![
        ImportNode::Group {
            name: "Box".into(),
            children: vec![ImportNode::Mesh(half("Left", 0.0, 1.0, &[3]))],
            tags: Vec::new(),
            hidden: false,
            key: None,
        },
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
        ImportNode::Mesh(wall(1.0)),
    ]);
    let members = family(&doc);
    let (before, depth) = (doc.state_hash(), doc.undo_depth());
    assert_eq!(
        doc.rejoin_as_one_solid(&members).unwrap_err(),
        DocumentError::RejoinNotPossible
    );
    assert_eq!((doc.state_hash(), doc.undo_depth()), (before, depth));
}

#[test]
fn a_definitions_split_shape_rejoins_inside_the_definition() {
    // The split pieces are members of one component, placed twice: the
    // rejoin is a definition edit every placement shows.
    let instance = |x: f64| ImportNode::Instance {
        def: 0,
        pose: kernel::Transform::translation(kernel::Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut s = scene(vec![instance(0.0), instance(10.0)]);
    let mut big = half("Big", 0.0, 1.5, &[3]);
    big.tags = vec![vec!["Wood".to_string()]];
    let mut small = half("Small", 1.5, 2.0, &[5]);
    small.tags = vec![vec!["Metal".to_string()]];
    s.defs = vec![kernel::DefRecipe {
        name: Some("Board".into()),
        children: vec![
            ImportNode::Mesh(small),
            ImportNode::Mesh(big),
            ImportNode::Mesh(wall(1.5)),
        ],
    }];
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    let members = family(&doc);
    let component = doc.object_owner_component(members[0]).expect("a member");
    let before = doc.state_hash();
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_eq!(doc.object_owner_component(result), Some(component));
    assert!((volume(&doc, result) - 2.0).abs() < 1e-9);
    assert_eq!(doc.object_name(result), Some("Big"));
    assert_eq!(
        doc.node_tags(NodeId::Object(result)),
        &[vec!["Wood".to_string()]]
    );
    assert!(doc.health_report(None).rows.is_empty());
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// A random box split into two or three pieces (with or without the
    /// walls) rejoins to one solid of the box's volume, and one undo
    /// restores the exact state; redo returns the same solid.
    #[test]
    fn rejoin_then_undo_is_the_identity(
        len in 1.0f64..8.0,
        depth in 0.5f64..4.0,
        height in 0.5f64..4.0,
        first in 0.15f64..0.45,
        second in 0.55f64..0.85,
        three in any::<bool>(),
        walls in any::<bool>(),
    ) {
        let mut cuts = vec![first * len];
        if three {
            cuts.push(second * len);
        }
        let mut xs = vec![0.0];
        xs.extend(cuts.iter().copied());
        xs.push(len);
        let n = xs.len() - 1;
        let mut roots = Vec::new();
        for i in 0..n {
            let mut drops = Vec::new();
            if i > 0 {
                drops.push(5);
            }
            if i + 1 < n {
                drops.push(3);
            }
            roots.push(ImportNode::Mesh(piece(
                &format!("Piece {i}"),
                xs[i],
                xs[i + 1],
                depth,
                height,
                &drops,
            )));
        }
        if walls {
            for (k, &x) in cuts.iter().enumerate() {
                roots.push(ImportNode::Mesh(wall_at(&format!("Wall {k}"), x, depth, height)));
            }
        }
        let mut doc = ingest(roots);
        let members = family(&doc);
        prop_assert_eq!(members.len(), n + if walls { cuts.len() } else { 0 });
        let before = doc.state_hash();
        let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
        let want = len * depth * height;
        prop_assert_eq!(doc.object(result).unwrap().watertight(), WatertightState::Watertight);
        prop_assert_eq!(shell_count(&doc, result), 1);
        prop_assert!((volume(&doc, result) - want).abs() < 1e-9 * want.max(1.0));
        prop_assert_eq!(doc.live_object_ids(), vec![result]);
        let after = doc.state_hash();
        doc.undo().unwrap();
        prop_assert_eq!(doc.state_hash(), before);
        doc.redo().unwrap();
        prop_assert_eq!(doc.state_hash(), after);
    }
}

// ---------------------------------------------------------------- review

/// How many face-connected shells `o` has (faces joined across twins).
fn shell_count(doc: &Document, o: ObjectId) -> usize {
    let obj = doc.object(o).expect("live");
    let faces: Vec<kernel::FaceId> = obj.faces().keys().collect();
    let index = |f: kernel::FaceId| faces.iter().position(|&g| g == f).unwrap();
    let mut root: Vec<usize> = (0..faces.len()).collect();
    fn find(root: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    for (_, he) in obj.half_edges().iter() {
        let Some(twin) = he.twin else { continue };
        let a = index(obj.loops()[he.loop_id].face);
        let b = index(obj.loops()[obj.half_edges()[twin].loop_id].face);
        let (ra, rb) = (find(&mut root, a), find(&mut root, b));
        root[ra] = rb;
    }
    (0..faces.len())
        .filter(|&i| find(&mut root, i) == i)
        .count()
}

fn group(name: &str, children: Vec<ImportNode>) -> ImportNode {
    ImportNode::Group {
        name: name.into(),
        children,
        tags: Vec::new(),
        hidden: false,
        key: None,
    }
}

fn instance_at_origin() -> ImportNode {
    ImportNode::Instance {
        def: 0,
        pose: kernel::Transform::IDENTITY,
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    }
}

fn assert_refused_untouched(doc: &mut Document, members: &[ObjectId]) {
    let (before, depth) = (doc.state_hash(), doc.undo_depth());
    assert_eq!(
        doc.rejoin_as_one_solid(members).unwrap_err(),
        DocumentError::RejoinNotPossible
    );
    assert_eq!((doc.state_hash(), doc.undo_depth()), (before, depth));
    assert!(members.iter().all(|&m| doc.object(m).is_some()));
}

#[test]
fn a_subset_of_a_family_refuses_rejoin_not_possible_untouched() {
    // Without the Wall, a rejoin would leave it behind as an open sheet.
    let mut doc = split_box();
    let pieces = vec![by_name(&doc, "Left"), by_name(&doc, "Right")];
    assert_refused_untouched(&mut doc, &pieces);
    assert_eq!(family(&doc).len(), 3, "the family reads as it did");
    // The whole family, listed twice over and in another order, rejoins.
    let mut members = family(&doc);
    members.reverse();
    members.extend(members.clone());
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert_one_solid(&doc, result, 2.0);
}

#[test]
fn a_wall_in_another_group_than_the_pieces_refuses_untouched() {
    // The Wall alone in its own group: deleting it would leave that group
    // empty, and a wall the result does not go back beside is not part of
    // the pieces' shape to remove.
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
        group("Wall group", vec![ImportNode::Mesh(wall(1.0))]),
    ]);
    let members = family(&doc);
    assert_eq!(members.len(), 3, "the grouped wall is in the family");
    let groups = doc.group_ids();
    assert_refused_untouched(&mut doc, &members);
    assert_eq!(doc.group_ids(), groups, "the wall's group stays");
}

#[test]
fn a_definition_wall_in_a_group_beside_top_level_pieces_refuses_untouched() {
    let mut s = scene(vec![instance_at_origin()]);
    s.defs = vec![kernel::DefRecipe {
        name: Some("Board".into()),
        children: vec![
            ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
            ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
            group("Inner", vec![ImportNode::Mesh(wall(1.0))]),
        ],
    }];
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    let members = family(&doc);
    assert_eq!(members.len(), 3);
    assert_refused_untouched(&mut doc, &members);
}

#[test]
fn a_definition_wall_is_never_in_a_family_with_world_pieces() {
    // Families are read per definition, so a placed wall over the seam of
    // two top-level pieces is not one of their walls.
    let mut s = scene(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
        instance_at_origin(),
    ]);
    s.defs = vec![kernel::DefRecipe {
        name: Some("Wall".into()),
        children: vec![ImportNode::Mesh(wall(1.0))],
    }];
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    let wall_id = by_name(&doc, "Wall");
    let members = family(&doc);
    assert!(!members.contains(&wall_id));
    let mut with_wall = members.clone();
    with_wall.push(wall_id);
    assert_refused_untouched(&mut doc, &with_wall);
    let (result, _) = doc.rejoin_as_one_solid(&members).unwrap();
    assert!(
        doc.object(wall_id).is_some(),
        "the placed wall is untouched"
    );
    assert!((volume(&doc, result) - 2.0).abs() < 1e-9);
}

#[test]
fn a_family_joined_only_through_a_wall_refuses_rather_than_two_shells() {
    // Left (y 0..1) and Right (y 2..3) are both open at x = 1 and never
    // touch; the wall at x = 1 is two rectangles, one in each opening.
    let mut right = piece("Right", 1.0, 2.0, 1.0, 1.0, &[5]);
    right.positions.iter_mut().for_each(|q| q.y += 2.0);
    let p = Point3::new;
    let w = mesh(
        "Wall",
        vec![
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(1.0, 1.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 2.0, 0.0),
            p(1.0, 3.0, 0.0),
            p(1.0, 3.0, 1.0),
            p(1.0, 2.0, 1.0),
        ],
        vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
    );
    let mut doc = ingest(vec![
        ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
        ImportNode::Mesh(right),
        ImportNode::Mesh(w),
    ]);
    let members = family(&doc);
    assert_eq!(members.len(), 3, "the wall joins the two into one family");
    assert_refused_untouched(&mut doc, &members);
}

#[test]
fn a_group_holding_only_the_pieces_keeps_the_result() {
    let mut doc = ingest(vec![group(
        "Board",
        vec![
            ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
            ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
        ],
    )]);
    let g = doc
        .node_parent(NodeId::Object(by_name(&doc, "Left")))
        .expect("grouped");
    let (result, _) = doc.rejoin_as_one_solid(&family(&doc)).unwrap();
    assert_one_solid(&doc, result, 2.0);
    assert_eq!(doc.node_parent(NodeId::Object(result)), Some(g));
    assert_eq!(doc.group_members(g), Some(vec![NodeId::Object(result)]));
}

#[test]
fn nested_groups_keep_the_result_in_the_inner_group() {
    let mut doc = ingest(vec![group(
        "Outer",
        vec![group(
            "Inner",
            vec![
                ImportNode::Mesh(half("Left", 0.0, 1.0, &[3])),
                ImportNode::Mesh(half("Right", 1.0, 2.0, &[5])),
                ImportNode::Mesh(wall(1.0)),
            ],
        )],
    )]);
    let inner = doc
        .node_parent(NodeId::Object(by_name(&doc, "Left")))
        .expect("grouped");
    let outer = doc.node_parent(NodeId::Group(inner)).expect("nested");
    let before = doc.state_hash();
    let (result, _) = doc.rejoin_as_one_solid(&family(&doc)).unwrap();
    assert_one_solid(&doc, result, 2.0);
    assert_eq!(doc.node_parent(NodeId::Object(result)), Some(inner));
    assert_eq!(doc.node_parent(NodeId::Group(inner)), Some(outer));
    assert_eq!(doc.group_members(inner), Some(vec![NodeId::Object(result)]));
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
}
