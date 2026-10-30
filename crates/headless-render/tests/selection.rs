//! `selection` and `background` through the real renderers.

use api::{LineDrawingFormat, LineDrawingParams, SnapshotParams, StandardView, locate::RenderRoot};
use kernel::{Document, EntityRef, NodeId, ObjectId, Plane, Point3, Transform, Vec3};

fn extrude_box(doc: &mut Document, x0: f64) -> ObjectId {
    let ground = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(ground);
    let sk = doc.sketch_mut(s).unwrap();
    let c = [
        Point3::new(x0, 0.0, 0.0),
        Point3::new(x0 + 1.0, 0.0, 0.0),
        Point3::new(x0 + 1.0, 1.0, 0.0),
        Point3::new(x0, 1.0, 0.0),
    ];
    for k in 0..4 {
        sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], 1.0).unwrap().0
}

fn public(doc: &Document, e: EntityRef) -> String {
    api::ids::public_id(&e, doc.sid_of(&e).unwrap())
}

fn occurrence(doc: &Document, o: ObjectId) -> RenderRoot {
    RenderRoot::Occurrence(doc.occurrence_at(&[], NodeId::Object(o)).unwrap())
}

fn params(selection: Option<Vec<RenderRoot>>) -> SnapshotParams {
    SnapshotParams {
        width: 64,
        height: 64,
        camera: None,
        view: None,
        scene: None,
        include_ids: true,
        include_depth: false,
        depth_bits: 8,
        include_normals: false,
        path: None,
        selection,
        background: softrender::BACKGROUND_RGBA,
    }
}

fn ids(r: &api::SnapshotResult) -> Vec<u16> {
    r.id_buffer
        .as_ref()
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect()
}

#[test]
fn a_selection_snapshot_frames_only_the_selection() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let _b = extrude_box(&mut doc, 30.0);
    let r = headless_render::snapshot(&doc, &params(Some(vec![occurrence(&doc, a)]))).unwrap();
    assert_eq!(r.id_palette, vec![public(&doc, EntityRef::Object(a))]);
    let ids = ids(&r);
    assert!(ids.iter().any(|i| *i != 0), "something is drawn");
    assert!(ids.iter().all(|i| *i <= 1), "only box A is in the picture");
    // Fitted to the box: it covers far more of the frame than a framing of
    // the whole document (two boxes 30 m apart) would give it.
    let covered = ids.iter().filter(|i| **i == 1).count();
    assert!(covered > 64 * 64 / 10, "covered {covered}");
}

#[test]
fn a_selection_without_a_camera_ignores_the_saved_working_camera() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    // A saved camera looking at empty space far away.
    doc.set_camera_state(kernel::CameraState {
        eye: Point3::new(500.0, 500.0, 500.0),
        target: Point3::new(600.0, 600.0, 600.0),
        up: Vec3::new(0.0, 0.0, 1.0),
        projection: kernel::CameraProjection::Perspective,
        fov_deg: 35.0,
        parallel_height: 2.0,
    });
    let r = headless_render::snapshot(&doc, &params(Some(vec![occurrence(&doc, a)]))).unwrap();
    assert!(ids(&r).contains(&1));
}

#[test]
fn a_definition_root_renders_once_under_the_definitions_own_id() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let (def, _first, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.place_instance(def, Transform::translation(Vec3::new(40.0, 0.0, 0.0)))
        .unwrap();
    let cmp = public(&doc, EntityRef::Component(def));
    let r =
        headless_render::snapshot(&doc, &params(Some(vec![RenderRoot::Definition(def)]))).unwrap();
    assert!(!r.id_palette.is_empty());
    assert!(
        r.id_palette.iter().all(|p| *p == cmp),
        "tagged with the definition, not an instance: {:?}",
        r.id_palette
    );
    // The whole document, by contrast, reports its instances.
    let whole = headless_render::snapshot(&doc, &params(None)).unwrap();
    assert!(whole.id_palette.iter().all(|p| p.starts_with("ins_")));
}

#[test]
fn a_hidden_selection_has_nothing_to_render() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let _b = extrude_box(&mut doc, 5.0);
    doc.set_node_user_hidden(NodeId::Object(a), true);
    let err = headless_render::snapshot(&doc, &params(Some(vec![occurrence(&doc, a)])))
        .expect_err("nothing visible");
    assert_eq!(err.name, "nothing_to_render");
}

#[test]
fn a_transparent_background_clears_untouched_pixels_only() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let mut p = params(Some(vec![occurrence(&doc, a)]));
    p.background = [0, 0, 0, 0];
    let r = headless_render::snapshot(&doc, &p).unwrap();
    let png = r.png;
    assert_eq!(&png[1..4], b"PNG");
    // Decode through the rasterizer's own buffers instead: render the same
    // items directly and compare the corner and center.
    let items = softrender::selection_items_hiding(
        &doc,
        p.selection.as_ref().unwrap(),
        &softrender::HiddenLeaves::of_document(&doc),
    )
    .unwrap();
    let render_items: Vec<_> = items
        .iter()
        .map(|it| softrender::RenderItem {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();
    let cam = softrender::Camera::standard_view(
        softrender::StandardView::Iso,
        softrender::items_bbox(&items).unwrap(),
    );
    let raw =
        softrender::render_with_background(&render_items, &cam, 64, 64, [0, 0, 0, 0]).unwrap();
    assert_eq!(&raw.rgba[0..4], &[0, 0, 0, 0]);
    assert_eq!(raw.ids[0], 0);
    assert_eq!(raw.rgba[(32 * 64 + 32) * 4 + 3], 255);
}

#[test]
fn a_line_drawing_of_a_selection_stays_within_its_projected_extent() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let _b = extrude_box(&mut doc, 3.0);
    let p = LineDrawingParams {
        camera: None,
        view: Some(StandardView::Top),
        scene: None,
        include_hidden: false,
        include_soft: false,
        format: LineDrawingFormat::Segments,
        scale: 1.0,
        path: None,
        selection: Some(vec![occurrence(&doc, a)]),
    };
    let r = headless_render::line_drawing(&doc, &p).unwrap();
    assert!(r.count > 0);
    // Box A is a unit square seen from above: every segment is within a
    // 1 m by 1 m footprint (centered on the view target).
    for s in &r.segments {
        for v in s {
            assert!(v.abs() <= 0.5 + 1e-6, "{s:?}");
        }
    }
}

#[test]
fn the_default_background_matches_the_rasterizers() {
    assert_eq!(
        api::commands::doc::DEFAULT_SNAPSHOT_BACKGROUND,
        softrender::BACKGROUND_RGBA
    );
}

// ------------------------------------------------------- root gathering

fn hidden_of(doc: &Document) -> softrender::HiddenLeaves {
    softrender::HiddenLeaves::of_document(doc)
}

fn items(doc: &Document, roots: &[RenderRoot]) -> Vec<softrender::DocumentItem> {
    softrender::selection_items_hiding(doc, roots, &hidden_of(doc)).unwrap()
}

#[test]
fn a_group_root_and_one_of_its_members_list_the_member_once() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 3.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let g = RenderRoot::Occurrence(doc.occurrence_at(&[], NodeId::Group(group)).unwrap());
    let both = items(&doc, std::slice::from_ref(&g));
    assert_eq!(both.len(), 2);
    let overlap = items(&doc, &[g, occurrence(&doc, a)]);
    assert_eq!(overlap.len(), 2, "the member is not listed twice");
}

#[test]
fn a_hidden_member_of_a_selected_group_is_skipped() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 3.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    doc.set_node_user_hidden(NodeId::Object(b), true);
    let g = RenderRoot::Occurrence(doc.occurrence_at(&[], NodeId::Group(group)).unwrap());
    assert_eq!(items(&doc, &[g]).len(), 1);
}

#[test]
fn an_instance_root_lists_its_leaves_at_the_instance_pose() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let (def, first, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (moved, _) = doc
        .place_instance(def, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .unwrap();
    let at = |i| {
        let root = RenderRoot::Occurrence(doc.occurrence_at(&[], NodeId::Instance(i)).unwrap());
        softrender::items_bbox(&items(&doc, &[root])).unwrap()
    };
    let (before, after) = (at(first), at(moved));
    assert!(
        (after.0.x - before.0.x - 10.0).abs() < 1e-6,
        "{before:?} {after:?}"
    );
}

#[test]
fn a_definition_root_lists_at_identity_once_however_often_it_is_placed() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let (def, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.place_instance(def, Transform::translation(Vec3::new(40.0, 0.0, 0.0)))
        .unwrap();
    doc.place_instance(def, Transform::translation(Vec3::new(-40.0, 0.0, 0.0)))
        .unwrap();
    let one = items(&doc, &[RenderRoot::Definition(def)]);
    assert_eq!(one.len(), 1);
    let (lo, hi) = softrender::items_bbox(&one).unwrap();
    assert!(lo.x > -5.0 && hi.x < 5.0, "identity pose: {lo:?} {hi:?}");
    let twice = items(
        &doc,
        &[RenderRoot::Definition(def), RenderRoot::Definition(def)],
    );
    assert_eq!(twice.len(), 1, "an id named twice is listed once");
}

#[test]
fn a_definition_root_ignores_tags_but_honors_a_members_own_hidden_flag() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 3.0);
    doc.add_node_tag(NodeId::Object(a), vec!["Hidden".into()])
        .unwrap();
    let (def, _, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    doc.set_tag_hidden(vec!["Hidden".into()], true);
    let root = RenderRoot::Definition(def);
    assert_eq!(
        items(&doc, std::slice::from_ref(&root)).len(),
        2,
        "tags are ignored"
    );
    let members = doc.def_member_nodes(def).unwrap();
    let NodeId::Object(first) = members[0] else {
        panic!("a leaf member")
    };
    doc.set_node_user_hidden(NodeId::Object(first), true);
    assert_eq!(
        items(&doc, &[root]).len(),
        1,
        "a member's own flag is honored"
    );
}

#[test]
fn a_mirrored_instance_reaches_the_item_pose_reflected() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let (def, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (mirrored, _) = doc
        .place_instance(def, Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .unwrap();
    let root = RenderRoot::Occurrence(doc.occurrence_at(&[], NodeId::Instance(mirrored)).unwrap());
    let got = items(&doc, &[root]);
    assert_eq!(got.len(), 1);
    assert!(got[0].pose.determinant() < 0.0, "the reflection survives");
}

#[test]
fn a_line_drawing_of_a_selection_ignores_the_saved_camera() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    doc.set_camera_state(kernel::CameraState {
        eye: Point3::new(500.0, 500.0, 500.0),
        target: Point3::new(600.0, 600.0, 600.0),
        up: Vec3::new(0.0, 0.0, 1.0),
        projection: kernel::CameraProjection::Perspective,
        fov_deg: 35.0,
        parallel_height: 2.0,
    });
    let mut p = LineDrawingParams {
        camera: None,
        view: None,
        scene: None,
        include_hidden: false,
        include_soft: false,
        format: LineDrawingFormat::Segments,
        scale: 1.0,
        path: None,
        selection: Some(vec![occurrence(&doc, a)]),
    };
    let implied = headless_render::line_drawing(&doc, &p).unwrap();
    p.view = Some(StandardView::Iso);
    let explicit = headless_render::line_drawing(&doc, &p).unwrap();
    assert_eq!(implied.segments, explicit.segments);
}
