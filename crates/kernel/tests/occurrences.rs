//! Specs for the occurrence walk (`Document::occurrences`,
//! `Document::occurrence_at`, `Document::definition_bounds`): every
//! placement in the model with its composed world pose, the paths that name
//! them, hidden pruning, roots, the caps, and open editing sessions.

use kernel::{
    ComponentId, Document, DocumentError, InstanceId, MAX_COMPONENT_DEPTH, NodeId, ObjectId,
    Occurrence, Plane, Point3, Transform, Vec3,
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

/// The shared fixture: a world box, a group of two boxes, and a component
/// `outer` holding a nested component instance and a nested group, placed
/// three times — once plainly, once mirrored, once inside another
/// component.
struct Fixture {
    doc: Document,
    a: ObjectId,
    g: kernel::GroupId,
    b: ObjectId,
    c: ObjectId,
    d: ObjectId,
    e: ObjectId,
    h: kernel::GroupId,
    inner: ComponentId,
    outer: ComponentId,
    i1: InstanceId,
    o1: InstanceId,
    o2: InstanceId,
    o3: InstanceId,
    w1: InstanceId,
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(b), NodeId::Object(c)])
        .unwrap();
    let d = extrude_box(&mut doc, 6.0, 0.0, 7.0, 2.0, 0.5);
    let (inner, i1, _) = doc.make_component(&[NodeId::Object(d)]).unwrap();
    // A non-trivial inner pose so composition order shows.
    let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
        .unwrap()
        .then(&Transform::translation(Vec3::new(0.0, 3.0, 0.0)));
    doc.transform_instance(i1, &turn).unwrap();
    let e = extrude_box(&mut doc, 8.0, 0.0, 9.0, 1.0, 1.0);
    let (h, _) = doc.group_nodes(&[NodeId::Object(e)]).unwrap();
    let (outer, o1, _) = doc
        .make_component(&[NodeId::Instance(i1), NodeId::Group(h)])
        .unwrap();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(20.0, 0.0, 0.0)));
    let (o2, _) = doc.place_instance(outer, mirror).unwrap();
    let (o3, _) = doc
        .place_instance(outer, Transform::translation(Vec3::new(0.0, 10.0, 0.0)))
        .unwrap();
    let (_, w1, _) = doc.make_component(&[NodeId::Instance(o3)]).unwrap();
    doc.transform_instance(w1, &Transform::translation(Vec3::new(0.0, 0.0, 5.0)))
        .unwrap();
    Fixture {
        doc,
        a,
        g,
        b,
        c,
        d,
        e,
        h,
        inner,
        outer,
        i1,
        o1,
        o2,
        o3,
        w1,
    }
}

fn key(o: &Occurrence) -> (Vec<InstanceId>, NodeId) {
    (o.path.clone(), o.node)
}

fn row<'a>(rows: &'a [Occurrence], path: &[InstanceId], node: NodeId) -> &'a Occurrence {
    rows.iter()
        .find(|o| o.path == path && o.node == node)
        .unwrap_or_else(|| panic!("no row for {path:?} / {node:?}"))
}

fn first_vertex(doc: &Document, o: ObjectId) -> Point3 {
    doc.object(o)
        .unwrap()
        .vertices()
        .values()
        .next()
        .unwrap()
        .position
}

fn close(a: Point3, b: Point3) -> bool {
    (a - b).length() < 1e-9
}

#[test]
fn the_whole_world_lists_every_placement_with_its_path() {
    let f = fixture();
    let rows = f.doc.occurrences(None, true).unwrap();
    let ob = NodeId::Object;
    let expected: Vec<(Vec<InstanceId>, NodeId)> = vec![
        (vec![], ob(f.a)),
        (vec![], NodeId::Group(f.g)),
        (vec![], ob(f.b)),
        (vec![], ob(f.c)),
        (vec![], NodeId::Instance(f.o1)),
        (vec![f.o1], NodeId::Instance(f.i1)),
        (vec![f.o1, f.i1], ob(f.d)),
        (vec![f.o1], NodeId::Group(f.h)),
        (vec![f.o1], ob(f.e)),
        (vec![], NodeId::Instance(f.o2)),
        (vec![f.o2], NodeId::Instance(f.i1)),
        (vec![f.o2, f.i1], ob(f.d)),
        (vec![f.o2], NodeId::Group(f.h)),
        (vec![f.o2], ob(f.e)),
        (vec![], NodeId::Instance(f.w1)),
        (vec![f.w1], NodeId::Instance(f.o3)),
        (vec![f.w1, f.o3], NodeId::Instance(f.i1)),
        (vec![f.w1, f.o3, f.i1], ob(f.d)),
        (vec![f.w1, f.o3], NodeId::Group(f.h)),
        (vec![f.w1, f.o3], ob(f.e)),
    ];
    assert_eq!(rows.iter().map(key).collect::<Vec<_>>(), expected);
    for r in &rows {
        assert_eq!(r.depth, r.path.len());
        assert!(!r.hidden);
    }
    // Owners, parents, definitions.
    assert_eq!(row(&rows, &[], ob(f.b)).parent_group, Some(f.g));
    assert_eq!(row(&rows, &[f.o1], ob(f.e)).parent_group, Some(f.h));
    assert_eq!(row(&rows, &[f.o1], ob(f.e)).owner_def, Some(f.outer));
    assert_eq!(row(&rows, &[f.o1, f.i1], ob(f.d)).owner_def, Some(f.inner));
    assert_eq!(row(&rows, &[], ob(f.a)).owner_def, None);
    assert_eq!(
        row(&rows, &[f.w1], NodeId::Instance(f.o3)).definition,
        Some(f.outer)
    );
}

#[test]
fn poses_compose_through_the_path_in_order() {
    let f = fixture();
    let rows = f.doc.occurrences(None, true).unwrap();
    let v = first_vertex(&f.doc, f.d);
    let i1 = f.doc.instance_pose(f.i1).unwrap();
    for (outer_path, outer_world) in [
        (vec![f.o1], f.doc.instance_pose(f.o1).unwrap()),
        (vec![f.o2], f.doc.instance_pose(f.o2).unwrap()),
        (vec![f.w1, f.o3], f.doc.instance_pose(f.o3).unwrap()),
    ] {
        let mut path = outer_path.clone();
        path.push(f.i1);
        let r = row(&rows, &path, NodeId::Object(f.d));
        let mut expected = outer_world.apply_point(i1.apply_point(v));
        if outer_path.len() == 2 {
            expected = f.doc.instance_pose(f.w1).unwrap().apply_point(expected);
        }
        assert!(
            close(r.pose.apply_point(v), expected),
            "{path:?}: {:?} vs {expected:?}",
            r.pose.apply_point(v)
        );
    }
    // An instance row carries its own pose composed in; the world box sits
    // at identity.
    let o2 = row(&rows, &[], NodeId::Instance(f.o2));
    assert_eq!(o2.pose, f.doc.instance_pose(f.o2).unwrap());
    assert_eq!(
        row(&rows, &[], NodeId::Object(f.a)).pose,
        Transform::IDENTITY
    );
}

#[test]
fn a_mirrored_placement_mirrors_every_row_under_it() {
    let f = fixture();
    let rows = f.doc.occurrences(None, true).unwrap();
    for r in &rows {
        let mirrored = r.pose.determinant() < 0.0;
        let under_o2 = r.path.first() == Some(&f.o2) || r.node == NodeId::Instance(f.o2);
        assert_eq!(mirrored, under_o2, "{:?}", key(r));
    }
}

#[test]
fn a_hidden_instance_hides_its_subtree_and_include_hidden_false_prunes_it() {
    let mut f = fixture();
    f.doc.set_node_user_hidden(NodeId::Instance(f.i1), true);
    let rows = f.doc.occurrences(None, true).unwrap();
    for r in &rows {
        let expect = r.node == NodeId::Instance(f.i1) || r.path.contains(&f.i1);
        assert_eq!(r.hidden, expect, "{:?}", key(r));
    }
    let shown = f.doc.occurrences(None, false).unwrap();
    assert_eq!(shown.len(), rows.len() - 6);
    assert!(shown.iter().all(|r| !r.hidden));
    assert!(shown.iter().all(|r| !r.path.contains(&f.i1)));
}

#[test]
fn a_hidden_tag_on_a_path_element_hides_everything_under_it() {
    let mut f = fixture();
    f.doc
        .add_node_tag(NodeId::Instance(f.o2), vec!["Mirror".into()])
        .unwrap();
    f.doc.set_tag_hidden(vec!["Mirror".into()], true);
    let rows = f.doc.occurrences(None, true).unwrap();
    for r in &rows {
        let expect = r.node == NodeId::Instance(f.o2) || r.path.first() == Some(&f.o2);
        assert_eq!(r.hidden, expect, "{:?}", key(r));
    }
    let shown = f.doc.occurrences(None, false).unwrap();
    assert!(shown.iter().all(|r| r.node != NodeId::Instance(f.o2)));
    assert_eq!(shown.len(), rows.len() - 5);
}

#[test]
fn occurrence_at_round_trips_every_row() {
    let mut f = fixture();
    f.doc.set_node_user_hidden(NodeId::Group(f.h), true);
    for r in f.doc.occurrences(None, true).unwrap() {
        assert_eq!(f.doc.occurrence_at(&r.path, r.node), Some(r.clone()));
    }
    // Paths that do not lead anywhere answer None.
    assert_eq!(f.doc.occurrence_at(&[], NodeId::Object(f.d)), None);
    assert_eq!(f.doc.occurrence_at(&[f.o1], NodeId::Object(f.a)), None);
    assert_eq!(f.doc.occurrence_at(&[f.i1], NodeId::Object(f.d)), None);
    assert_eq!(
        f.doc.occurrence_at(&[f.o1, f.o2], NodeId::Object(f.d)),
        None
    );
}

#[test]
fn overlapping_roots_list_a_member_once_under_the_first() {
    let f = fixture();
    let roots = vec![(vec![], NodeId::Group(f.g)), (vec![], NodeId::Object(f.b))];
    let rows = f.doc.occurrences(Some(&roots), true).unwrap();
    assert_eq!(
        rows.iter().map(key).collect::<Vec<_>>(),
        vec![
            (vec![], NodeId::Group(f.g)),
            (vec![], NodeId::Object(f.b)),
            (vec![], NodeId::Object(f.c)),
        ]
    );
}

#[test]
fn disjoint_roots_list_in_root_order() {
    let f = fixture();
    let roots = vec![
        (vec![], NodeId::Instance(f.o2)),
        (vec![], NodeId::Object(f.a)),
    ];
    let rows = f.doc.occurrences(Some(&roots), true).unwrap();
    let keys: Vec<_> = rows.iter().map(key).collect();
    assert_eq!(keys.first(), Some(&(vec![], NodeId::Instance(f.o2))));
    assert_eq!(keys.last(), Some(&(vec![], NodeId::Object(f.a))));
    assert_eq!(keys.len(), 6);
}

#[test]
fn an_instance_path_root_lists_that_placement_and_its_subtree_only() {
    let f = fixture();
    let roots = vec![(vec![f.w1], NodeId::Instance(f.o3))];
    let rows = f.doc.occurrences(Some(&roots), true).unwrap();
    let whole = f.doc.occurrences(None, true).unwrap();
    assert_eq!(
        rows.iter().map(key).collect::<Vec<_>>(),
        vec![
            (vec![f.w1], NodeId::Instance(f.o3)),
            (vec![f.w1, f.o3], NodeId::Instance(f.i1)),
            (vec![f.w1, f.o3, f.i1], NodeId::Object(f.d)),
            (vec![f.w1, f.o3], NodeId::Group(f.h)),
            (vec![f.w1, f.o3], NodeId::Object(f.e)),
        ]
    );
    // The same rows the whole-world walk lists for those paths.
    for r in &rows {
        assert_eq!(r, row(&whole, &r.path, r.node));
    }
    // A root that does not resolve refuses typed.
    assert_eq!(
        f.doc
            .occurrences(Some(&[(vec![], NodeId::Object(f.d))]), true),
        Err(DocumentError::UnknownInstance)
    );
}

#[test]
fn definition_bounds_matches_expanded_placements() {
    let f = fixture();
    for def in [f.inner, f.outer] {
        let mut acc: Option<(Point3, Point3)> = None;
        for (o, local) in f.doc.expanded_def_placements(def) {
            for v in f.doc.object(o).unwrap().vertices().values() {
                let p = local.apply_point(v.position);
                acc = Some(match acc {
                    None => (p, p),
                    Some((lo, hi)) => (
                        Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                        Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
                    ),
                });
            }
        }
        assert_eq!(f.doc.definition_bounds(def), acc);
        assert!(acc.is_some());
    }
}

#[test]
fn the_depth_cap_lists_every_level_the_loader_allows() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let mut levels = 1;
    while let Ok((_, next, _)) = doc.make_component(&[NodeId::Instance(inst)]) {
        inst = next;
        levels += 1;
        if levels > MAX_COMPONENT_DEPTH + 2 {
            break;
        }
    }
    assert!(levels <= MAX_COMPONENT_DEPTH, "the op bounds nesting");
    let rows = doc.occurrences(None, true).unwrap();
    // One instance row per level plus the leaf object.
    assert_eq!(rows.len(), levels + 1);
    let deepest = rows.iter().map(|r| r.depth).max().unwrap();
    assert_eq!(deepest, levels);
    for r in &rows {
        assert_eq!(doc.occurrence_at(&r.path, r.node).as_ref(), Some(r));
    }
    // A path longer than the cap never resolves.
    let long: Vec<InstanceId> = vec![inst; MAX_COMPONENT_DEPTH + 1];
    assert_eq!(doc.occurrence_at(&long, NodeId::Object(a)), None);
}

#[test]
fn too_many_occurrences_refuses_instead_of_truncating() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    // 17 doublings: 2^17 leaves plus every instance row above them.
    for _ in 0..17 {
        let (second, _) = doc
            .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    assert_eq!(
        doc.occurrences(None, true),
        Err(DocumentError::TooManyOccurrences)
    );
    let _ = def;
}

#[test]
fn an_explode_session_lists_displayed_rows_and_hidden_placements_at_their_poses() {
    let mut doc = Document::new();
    let m = extrude_box(&mut doc, 0.0, 0.0, 1.0, 2.0, 0.5);
    let (k, k1, _) = doc.make_component(&[NodeId::Object(m)]).unwrap();
    doc.transform_instance(k1, &Transform::translation(Vec3::new(1.0, 2.0, 0.0)))
        .unwrap();
    let k2_pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5)
        .unwrap()
        .then(&Transform::translation(Vec3::new(5.0, 0.0, 0.0)));
    let (k2, _) = doc.place_instance(k, k2_pose).unwrap();
    let local: Vec<Point3> = doc
        .object(m)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    let bounds_before = doc.definition_bounds(k).unwrap();

    doc.open_explode_session(k1).unwrap();

    let shown = doc.occurrences(None, false).unwrap();
    assert_eq!(
        shown.iter().map(key).collect::<Vec<_>>(),
        vec![
            (vec![], NodeId::Object(m)),
            (vec![], NodeId::Instance(k2)),
            (vec![k2], NodeId::Object(m)),
        ],
        "as displayed: the baked member and the live sibling placement"
    );
    let all = doc.occurrences(None, true).unwrap();
    assert_eq!(
        all.iter().map(key).collect::<Vec<_>>(),
        vec![
            (vec![], NodeId::Object(m)),
            (vec![], NodeId::Instance(k1)),
            (vec![k1], NodeId::Object(m)),
            (vec![], NodeId::Instance(k2)),
            (vec![k2], NodeId::Object(m)),
        ]
    );
    // The entered placement is hidden; the sibling stays live (ARCHITECTURE.md
    // 2.11), so its rows are not hidden.
    for r in &all[1..] {
        let sibling = r.path.first() == Some(&k2) || r.node == NodeId::Instance(k2);
        assert_eq!(r.hidden, !sibling, "{:?}", key(r));
    }
    // A baked member reached under the hidden sibling maps each vertex to
    // where that placement would draw it.
    let under_k2 = row(&all, &[k2], NodeId::Object(m));
    let baked: Vec<Point3> = doc
        .object(m)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    for (b, l) in baked.iter().zip(&local) {
        let got = under_k2.pose.apply_point(*b);
        assert!(close(got, k2_pose.apply_point(*l)), "{got:?}");
    }
    // The definition still answers its own local box.
    let bounds_now = doc.definition_bounds(k).unwrap();
    assert!(close(bounds_now.0, bounds_before.0) && close(bounds_now.1, bounds_before.1));
    for r in &all {
        assert_eq!(doc.occurrence_at(&r.path, r.node).as_ref(), Some(r));
    }
}

#[test]
fn a_group_session_lists_the_group_hidden_and_its_members_as_world_rows() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    doc.open_group_session(g).unwrap();
    let all = doc.occurrences(None, true).unwrap();
    assert_eq!(
        all.iter().map(key).collect::<Vec<_>>(),
        vec![
            (vec![], NodeId::Object(a)),
            (vec![], NodeId::Object(b)),
            (vec![], NodeId::Group(g)),
        ]
    );
    assert!(all[2].hidden);
    assert!(all[0].parent_group.is_none());
    let shown = doc.occurrences(None, false).unwrap();
    assert_eq!(shown.len(), 2);
}
