//! Acceptance specs for the Model Health repairs (the import-health design
//! §4): capping open rims (`Document::cap_rims`), deleting faces from an
//! open object (`Document::delete_faces`) and marking a surface
//! (`Document::set_accepted_surface`) — what each makes, every refusal, and
//! one undo step for each.

use kernel::health::Verdict;
use kernel::repair::RepairError;
use kernel::{
    DefRecipe, Document, DocumentError, FaceId, ImportNode, ImportScene, KernelOp, MeshRecipe,
    NodeId, ObjectId, Point3, Transform, Vec3, WatertightState,
};
use proptest::prelude::*;

fn scene(roots: Vec<ImportNode>, defs: Vec<DefRecipe>) -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs,
        roots,
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
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

/// The six outward faces of a box over corners 0..8 (0–3 the bottom ring,
/// 4–7 the top): bottom, top, front, right, back, left.
fn box_faces() -> Vec<Vec<usize>> {
    vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
}

fn box_corners(w: f64, d: f64, h: f64) -> Vec<Point3> {
    vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(w, 0.0, 0.0),
        Point3::new(w, d, 0.0),
        Point3::new(0.0, d, 0.0),
        Point3::new(0.0, 0.0, h),
        Point3::new(w, 0.0, h),
        Point3::new(w, d, h),
        Point3::new(0.0, d, h),
    ]
}

/// A unit box without its lid (face 1).
fn open_box() -> MeshRecipe {
    let faces = box_faces()
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i != 1)
        .map(|(_, f)| f)
        .collect();
    mesh("Tank", box_corners(1.0, 1.0, 1.0), faces)
}

/// A square tube 2 wide with a 1-wide square bore, 1 high, open at both
/// ends: the outer and inner walls only, two islands.
fn thick_pipe() -> MeshRecipe {
    let mut positions = box_corners(2.0, 2.0, 1.0);
    positions.extend(
        box_corners(1.0, 1.0, 1.0)
            .into_iter()
            .map(|p| Point3::new(p.x + 0.5, p.y + 0.5, p.z)),
    );
    let walls = || box_faces().into_iter().skip(2);
    let mut faces: Vec<Vec<usize>> = walls().collect();
    // The bore's walls face the axis: the same quads, reversed.
    faces.extend(walls().map(|f| f.into_iter().rev().map(|i| i + 8).collect()));
    mesh("Pipe", positions, faces)
}

fn ingest(doc: &mut Document, roots: Vec<ImportNode>) {
    doc.ingest(scene(roots, Vec::new()), Vec::new()).unwrap();
}

fn only_object(doc: &Document) -> ObjectId {
    let ids = doc.live_object_ids();
    assert_eq!(ids.len(), 1, "one live object");
    ids[0]
}

fn signed_volume(doc: &Document, id: ObjectId) -> f64 {
    let obj = doc.object(id).unwrap();
    let mut six_v = 0.0;
    for f in obj.faces().values() {
        for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let p: Vec<Vec3> = obj.loop_positions(lid).map(|pt| pt.to_vec()).collect();
            for i in 1..p.len().saturating_sub(1) {
                six_v += p[0].dot(p[i].cross(p[i + 1]));
            }
        }
    }
    six_v / 6.0
}

fn face_count(doc: &Document, id: ObjectId) -> usize {
    doc.object(id).unwrap().faces().len()
}

fn watertight(doc: &Document, id: ObjectId) -> bool {
    doc.object(id).unwrap().watertight() == WatertightState::Watertight
}

/// The face of `id` whose outer loop passes `pick`.
fn face_where(doc: &Document, id: ObjectId, pick: impl Fn(Point3) -> bool) -> FaceId {
    let obj = doc.object(id).unwrap();
    obj.faces()
        .iter()
        .find(|(_, f)| obj.loop_positions(f.outer_loop).all(&pick))
        .map(|(fid, _)| fid)
        .expect("a face matches")
}

fn repair_err(e: DocumentError) -> RepairError {
    match e {
        DocumentError::Repair(r) => r,
        other => panic!("expected a repair refusal, got {other:?}"),
    }
}

#[test]
fn a_box_missing_its_lid_closes_with_one_face_and_pushes_afterwards() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(open_box())]);
    let id = only_object(&doc);
    assert_eq!(doc.object_health(id).unwrap().verdict, Verdict::Cappable);

    let (report, change) = doc.cap_rims(id, None).unwrap();
    assert_eq!(report.faces.len(), 1);
    assert!(report.closed && !report.reversed);
    assert_eq!(change.objects_touched, vec![id]);
    assert!(watertight(&doc, id));
    assert_eq!(face_count(&doc, id), 6);
    assert!((signed_volume(&doc, id) - 1.0).abs() < 1e-9);
    // The lid faces up, out of the box, like its neighbors.
    let lid = doc.object(id).unwrap().faces()[report.faces[0]].plane;
    assert!((lid.normal().z - 1.0).abs() < 1e-9);
    // The object keeps its name.
    assert_eq!(doc.object_name(id), Some("Tank"));

    // An ordinary face afterwards: it pushes.
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: report.faces[0],
            distance: 0.5,
        },
    )
    .unwrap();
    assert!((signed_volume(&doc, id) - 1.5).abs() < 1e-9);

    // Undo the push, then the cap: open again, as imported. Redo both.
    doc.undo().unwrap();
    let undone = doc.undo().unwrap();
    assert_eq!(undone.objects_touched, vec![id]);
    assert!(!watertight(&doc, id));
    assert_eq!(face_count(&doc, id), 5);
    assert_eq!(doc.object_health(id).unwrap().verdict, Verdict::Cappable);
    doc.redo().unwrap();
    assert!(watertight(&doc, id));
    doc.redo().unwrap();
    assert!((signed_volume(&doc, id) - 1.5).abs() < 1e-9);
}

#[test]
fn an_inside_out_box_is_turned_right_side_out_when_it_closes() {
    let mut inside_out = open_box();
    for f in &mut inside_out.faces {
        f.reverse();
    }
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(inside_out)]);
    let id = only_object(&doc);
    let (report, _) = doc.cap_rims(id, None).unwrap();
    assert!(report.closed && report.reversed);
    assert!((signed_volume(&doc, id) - 1.0).abs() < 1e-9);
    doc.undo().unwrap();
    assert!(!watertight(&doc, id));
}

#[test]
fn a_pipes_open_ends_close_as_rings_around_the_bore() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(thick_pipe())]);
    let id = only_object(&doc);
    assert_eq!(doc.object_health(id).unwrap().rims.len(), 4);
    let (report, _) = doc.cap_rims(id, None).unwrap();
    // Two faces, top and bottom, each with the bore as its hole.
    assert_eq!(report.faces.len(), 2);
    let obj = doc.object(id).unwrap();
    assert!(
        report
            .faces
            .iter()
            .all(|&f| obj.faces()[f].inner_loops.len() == 1)
    );
    assert!(report.closed);
    assert!((signed_volume(&doc, id) - 3.0).abs() < 1e-9);
}

#[test]
fn capping_one_end_ring_alone_would_cover_the_bore_and_refuses() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(thick_pipe())]);
    let id = only_object(&doc);
    let d = doc.object_health(id).unwrap();
    let obj = doc.object(id).unwrap();
    let z_of = |rim: usize| {
        let h = d.rims[rim].half_edges[0];
        obj.vertices()[obj.half_edges()[h].origin].position.z
    };
    let outer_top = (0..d.rims.len())
        .filter(|&r| z_of(r) > 0.5)
        .max_by(|&a, &b| d.rims[a].extent.total_cmp(&d.rims[b].extent))
        .unwrap();
    let err = doc.cap_rims(id, Some(&[outer_top])).unwrap_err();
    assert_eq!(repair_err(err), RepairError::CapsOverlap);
    assert_eq!(face_count(&doc, id), 8, "untouched");
}

#[test]
fn a_warped_rim_refuses_as_not_planar() {
    let mut positions = box_corners(1.0, 1.0, 1.0);
    positions[5].z = 1.2;
    positions[7].z = 1.2;
    let faces = box_faces().into_iter().filter(|f| f[0] != 4).collect();
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Warped", positions, faces))],
    );
    let id = only_object(&doc);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::RimNotPlanar);
}

#[test]
fn a_rim_that_crosses_itself_refuses() {
    // A bow-tie rim on z = 0 under a four-triangle tent: planar, closed,
    // branch-free, and crossing itself at (1, 1).
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(0.0, 2.0, 0.0),
        p(1.0, 1.0, 1.0),
    ];
    let faces = vec![vec![0, 1, 4], vec![1, 2, 4], vec![2, 3, 4], vec![3, 0, 4]];
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Tent", positions, faces))],
    );
    let id = only_object(&doc);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::RimSelfIntersecting);
}

#[test]
fn a_flat_sheet_refuses_a_cap_lying_on_itself() {
    let square = mesh(
        "Pane",
        box_corners(1.0, 1.0, 0.0)[..4].to_vec(),
        vec![vec![0, 1, 2, 3]],
    );
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(square)]);
    let id = only_object(&doc);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::CapWouldBeCoplanar);
}

#[test]
fn a_branching_rim_refuses_as_non_manifold() {
    // Two triangles meeting only at the origin: a bow-tie vertex.
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(-1.0, 0.0, 0.5),
        p(0.0, -1.0, 0.5),
    ];
    let faces = vec![vec![0, 1, 2], vec![0, 3, 4]];
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Bowtie", positions, faces))],
    );
    let id = only_object(&doc);
    assert_eq!(doc.object_health(id).unwrap().verdict, Verdict::NonManifold);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::NonManifoldRim);
}

#[test]
fn a_solid_and_a_stale_rim_refuse() {
    let mut doc = Document::new();
    let solid = mesh("Cube", box_corners(1.0, 1.0, 1.0), box_faces());
    ingest(&mut doc, vec![ImportNode::Mesh(solid)]);
    let id = only_object(&doc);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::ObjectIsSolid);
    let any_face = doc.object(id).unwrap().faces().keys().next().unwrap();
    let err = doc.delete_faces(id, &[any_face]).unwrap_err();
    assert_eq!(repair_err(err), RepairError::ObjectIsSolid);
    let err = doc.set_accepted_surface(id, true).unwrap_err();
    assert_eq!(repair_err(err), RepairError::ObjectIsSolid);

    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(open_box())]);
    let id = only_object(&doc);
    let err = doc.cap_rims(id, Some(&[7])).unwrap_err();
    assert_eq!(repair_err(err), RepairError::UnknownRim);
    let depth = doc.content_depth();
    assert!(doc.cap_rims(id, Some(&[])).is_err());
    assert_eq!(doc.content_depth(), depth, "a refusal records nothing");
}

#[test]
fn capping_a_definition_member_reaches_every_instance() {
    let place = |x: f64| ImportNode::Instance {
        def: 0,
        pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut doc = Document::new();
    doc.ingest(
        scene(
            vec![place(0.0), place(5.0)],
            vec![DefRecipe {
                name: Some("Tank".into()),
                children: vec![ImportNode::Mesh(open_box())],
            }],
        ),
        Vec::new(),
    )
    .unwrap();
    let id = only_object(&doc);
    let (_, change) = doc.cap_rims(id, None).unwrap();
    assert_eq!(change.components_touched.len(), 1);
    assert_eq!(change.instances_touched.len(), 2);
    assert!(watertight(&doc, id));
    assert!(doc.health_report(None).rows.is_empty());
    let undone = doc.undo().unwrap();
    assert_eq!(undone.instances_touched.len(), 2);
    assert_eq!(doc.health_report(None).rows.len(), 1);
}

#[test]
fn deleting_a_stray_sheet_leaves_the_solid_beside_it() {
    let mut positions = box_corners(1.0, 1.0, 1.0);
    positions.extend(
        box_corners(1.0, 1.0, 0.0)[..4]
            .iter()
            .map(|p| Point3::new(p.x + 3.0, p.y, p.z)),
    );
    let mut faces = box_faces();
    faces.push(vec![8, 9, 10, 11]);
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Crate", positions, faces))],
    );
    let id = only_object(&doc);
    assert!(!watertight(&doc, id));
    let sheet = face_where(&doc, id, |p| p.x > 2.0);
    let (split_off, change) = doc.delete_faces(id, &[sheet]).unwrap();
    assert!(split_off.is_empty());
    assert_eq!(change.objects_touched, vec![id]);
    assert!(watertight(&doc, id));
    assert!((signed_volume(&doc, id) - 1.0).abs() < 1e-9);
    doc.undo().unwrap();
    assert!(!watertight(&doc, id));
    assert_eq!(face_count(&doc, id), 7);
}

/// Three unit quads in a row along x.
fn strip() -> MeshRecipe {
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let positions = vec![
        p(0.0, 0.0),
        p(1.0, 0.0),
        p(2.0, 0.0),
        p(3.0, 0.0),
        p(0.0, 1.0),
        p(1.0, 1.0),
        p(2.0, 1.0),
        p(3.0, 1.0),
    ];
    mesh(
        "Strip",
        positions,
        vec![vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6]],
    )
}

#[test]
fn deleting_a_bridge_splits_the_rest_into_objects_in_the_same_group() {
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Group {
            name: "Wall".into(),
            children: vec![ImportNode::Mesh(strip())],
            tags: Vec::new(),
            hidden: false,
            key: None,
        }],
    );
    let id = only_object(&doc);
    let group = doc.group_ids()[0];
    let middle = face_where(&doc, id, |p| (1.0..=2.0).contains(&p.x));
    let (split_off, change) = doc.delete_faces(id, &[middle]).unwrap();
    assert_eq!(split_off.len(), 1);
    assert_eq!(change.groups_touched, vec![group]);
    let piece = split_off[0];
    assert_eq!(face_count(&doc, id), 1);
    assert_eq!(face_count(&doc, piece), 1);
    assert_eq!(doc.object_name(piece), Some("Strip"));
    assert_eq!(doc.node_parent(NodeId::Object(piece)), Some(group));
    assert!(
        doc.group_members(group)
            .unwrap()
            .contains(&NodeId::Object(piece))
    );

    doc.undo().unwrap();
    assert_eq!(face_count(&doc, id), 3);
    assert_eq!(doc.live_object_ids(), vec![id]);
    assert_eq!(doc.group_members(group).unwrap(), vec![NodeId::Object(id)]);
    doc.redo().unwrap();
    assert_eq!(doc.live_object_ids().len(), 2);
    assert!(
        doc.group_members(group)
            .unwrap()
            .contains(&NodeId::Object(piece))
    );
}

#[test]
fn deleting_every_face_deletes_the_object() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(strip())]);
    let id = only_object(&doc);
    let all: Vec<FaceId> = doc.object(id).unwrap().faces().keys().collect();
    let (split_off, _) = doc.delete_faces(id, &all).unwrap();
    assert!(split_off.is_empty());
    assert!(doc.live_object_ids().is_empty());
    doc.undo().unwrap();
    assert_eq!(doc.live_object_ids(), vec![id]);
}

#[test]
fn a_stale_face_refuses() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(strip())]);
    let id = only_object(&doc);
    let first = doc.object(id).unwrap().faces().keys().next().unwrap();
    doc.delete_faces(id, &[first]).unwrap();
    let err = doc.delete_faces(id, &[first]).unwrap_err();
    assert_eq!(repair_err(err), RepairError::UnknownFace);
}

#[test]
fn a_marked_surface_survives_save_and_load_on_a_definition_member() {
    let mut doc = Document::new();
    doc.ingest(
        scene(
            vec![ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            }],
            vec![DefRecipe {
                name: Some("Sign".into()),
                children: vec![ImportNode::Mesh(strip())],
            }],
        ),
        Vec::new(),
    )
    .unwrap();
    let id = only_object(&doc);
    doc.set_accepted_surface(id, true).unwrap();
    let depth = doc.content_depth();
    doc.set_accepted_surface(id, true).unwrap();
    assert_eq!(
        doc.content_depth(),
        depth,
        "setting it again records nothing"
    );
    assert!(doc.health_report(None).rows[0].accepted_surface);

    let loaded = Document::load(&doc.save()).unwrap();
    let member = only_object(&loaded);
    assert!(loaded.is_accepted_surface(member));

    doc.set_accepted_surface(id, false).unwrap();
    assert!(!doc.is_accepted_surface(id));
    doc.undo().unwrap();
    assert!(doc.is_accepted_surface(id));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any box missing any one face, at any size and orientation, wound
    /// either way, caps to the closed box: right side out, the box's
    /// volume, and one undo from where it was.
    #[test]
    fn any_open_box_caps_to_the_closed_box(
        w in 0.05f64..10.0,
        d in 0.05f64..10.0,
        h in 0.05f64..10.0,
        yaw in 0.0f64..std::f64::consts::TAU,
        pitch in 0.0f64..std::f64::consts::TAU,
        drop in 0usize..6,
        inside_out in proptest::bool::ANY,
    ) {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let turn = |p: Point3| {
            let (x, y, z) = (p.x * cy - p.y * sy, p.x * sy + p.y * cy, p.z);
            Point3::new(x, y * cp - z * sp, y * sp + z * cp)
        };
        let positions = box_corners(w, d, h).into_iter().map(turn).collect();
        let faces: Vec<Vec<usize>> = box_faces()
            .into_iter()
            .enumerate()
            .filter(|&(i, _)| i != drop)
            .map(|(_, mut f)| {
                if inside_out {
                    f.reverse();
                }
                f
            })
            .collect();
        let mut doc = Document::new();
        ingest(&mut doc, vec![ImportNode::Mesh(mesh("Box", positions, faces))]);
        let id = only_object(&doc);
        let (report, _) = doc.cap_rims(id, None).unwrap();
        prop_assert!(report.closed);
        prop_assert_eq!(report.reversed, inside_out);
        let v = signed_volume(&doc, id);
        prop_assert!((v - w * d * h).abs() <= 1e-9 * (w * d * h).max(1.0), "volume {} vs {}", v, w * d * h);
        doc.undo().unwrap();
        prop_assert!(!watertight(&doc, id));
        prop_assert_eq!(face_count(&doc, id), 5);
    }
}

fn pane() -> MeshRecipe {
    mesh(
        "Pane",
        box_corners(1.0, 1.0, 0.0)[..4].to_vec(),
        vec![vec![0, 1, 2, 3]],
    )
}

/// The z-range of an object's vertices.
fn z_range(doc: &Document, id: ObjectId) -> (f64, f64) {
    doc.object(id)
        .unwrap()
        .vertices()
        .values()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| {
            (lo.min(v.position.z), hi.max(v.position.z))
        })
}

#[test]
fn a_pane_pulls_into_a_slab_either_way_or_centered() {
    for (start, end, want) in [
        (0.0, 0.1, (0.0, 0.1)),
        (0.0, -0.1, (-0.1, 0.0)),
        (-0.05, 0.05, (-0.05, 0.05)),
    ] {
        let mut doc = Document::new();
        ingest(&mut doc, vec![ImportNode::Mesh(pane())]);
        let id = only_object(&doc);
        let (split_off, change) = doc.thicken_sheet(id, start, end).unwrap();
        assert!(split_off.is_empty());
        assert_eq!(change.objects_touched, vec![id]);
        assert!(watertight(&doc, id));
        assert_eq!(face_count(&doc, id), 6);
        assert!((signed_volume(&doc, id) - 0.1).abs() < 1e-9);
        let (lo, hi) = z_range(&doc, id);
        assert!((lo - want.0).abs() < 1e-12 && (hi - want.1).abs() < 1e-12);
        assert_eq!(doc.object_name(id), Some("Pane"));
        doc.undo().unwrap();
        assert_eq!(
            doc.object_health(id).unwrap().verdict,
            Verdict::FlatSheet { holes: 0 }
        );
    }
}

#[test]
fn a_subdivided_sheet_keeps_its_faces_on_both_sides() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(strip())]);
    let id = only_object(&doc);
    doc.thicken_sheet(id, 0.0, 0.2).unwrap();
    // Three faces below, three above, a wall under each of the 8 open edges.
    assert_eq!(face_count(&doc, id), 3 + 3 + 8);
    assert!((signed_volume(&doc, id) - 0.6).abs() < 1e-9);
}

#[test]
fn a_sheet_with_a_hole_pulls_into_a_frame() {
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let mut frame = mesh(
        "Frame",
        vec![
            p(0.0, 0.0),
            p(3.0, 0.0),
            p(3.0, 3.0),
            p(0.0, 3.0),
            p(1.0, 1.0),
            p(2.0, 1.0),
            p(2.0, 2.0),
            p(1.0, 2.0),
        ],
        vec![vec![0, 1, 2, 3]],
    );
    frame.face_holes = vec![vec![vec![4, 7, 6, 5]]];
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(frame)]);
    let id = only_object(&doc);
    assert_eq!(
        doc.object_health(id).unwrap().verdict,
        Verdict::FlatSheet { holes: 1 }
    );
    doc.thicken_sheet(id, 0.0, 0.5).unwrap();
    assert!(watertight(&doc, id));
    // Two holed faces plus four outer and four inner walls.
    assert_eq!(face_count(&doc, id), 10);
    assert!((signed_volume(&doc, id) - 8.0 * 0.5).abs() < 1e-9);
}

#[test]
fn a_textured_pane_keeps_its_texture_in_place_on_the_far_side() {
    let mut textured = pane();
    let uv = kernel::UvFrame::new(
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(0.0, 2.0, 0.5),
        0.25,
        0.75,
    );
    textured.face_uv_frames = vec![Some(uv)];
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(textured)]);
    let id = only_object(&doc);
    doc.thicken_sheet(id, 0.0, 0.4).unwrap();
    let obj = doc.object(id).unwrap();
    // Every capped face maps each of its corners to where the pane's own
    // corner below or above it mapped.
    let mut caps = 0;
    for face in obj.faces().values() {
        let Some(frame) = face.uv_frame else { continue };
        caps += 1;
        for q in obj.loop_positions(face.outer_loop) {
            let flat = Point3::new(q.x, q.y, 0.0);
            let (a, b) = (frame.apply(q), uv.apply(flat));
            assert!((a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12);
        }
    }
    assert_eq!(caps, 2);
}

#[test]
fn separate_sheets_pull_into_separate_solids() {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    // A pane on the ground and a wall standing apart from it.
    let two = mesh(
        "Panels",
        vec![
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(0.0, 2.0, 0.0),
            p(5.0, 0.0, 0.0),
            p(6.0, 0.0, 0.0),
            p(6.0, 0.0, 1.0),
            p(5.0, 0.0, 1.0),
        ],
        vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
    );
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(two)]);
    let id = only_object(&doc);
    let (split_off, _) = doc.thicken_sheet(id, 0.0, 0.1).unwrap();
    assert_eq!(split_off.len(), 1);
    // The larger island stays in place.
    assert!((signed_volume(&doc, id) - 0.4).abs() < 1e-9);
    assert!((signed_volume(&doc, split_off[0]) - 0.1).abs() < 1e-9);
    assert_eq!(doc.object_name(split_off[0]), Some("Panels"));
    doc.undo().unwrap();
    assert_eq!(doc.live_object_ids(), vec![id]);
}

#[test]
fn only_a_flat_sheet_thickens() {
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(open_box())]);
    let id = only_object(&doc);
    let err = doc.thicken_sheet(id, 0.0, 0.1).unwrap_err();
    assert_eq!(repair_err(err), RepairError::NotAFlatSheet);

    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(pane())]);
    let id = only_object(&doc);
    for (a, b) in [
        (0.0, 0.0),
        (0.0, 1e-9),
        (0.0, f64::NAN),
        (f64::INFINITY, 1.0),
    ] {
        let err = doc.thicken_sheet(id, a, b).unwrap_err();
        assert_eq!(repair_err(err), RepairError::ThicknessTooSmall);
    }
    doc.thicken_sheet(id, 0.0, 0.1).unwrap();
    let err = doc.thicken_sheet(id, 0.0, 0.1).unwrap_err();
    assert_eq!(repair_err(err), RepairError::ObjectIsSolid);
}

#[test]
fn thickening_a_definition_member_reaches_every_instance() {
    let place = |x: f64| ImportNode::Instance {
        def: 0,
        pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut doc = Document::new();
    doc.ingest(
        scene(
            vec![place(0.0), place(5.0), place(10.0)],
            vec![DefRecipe {
                name: Some("Sign".into()),
                children: vec![ImportNode::Mesh(pane())],
            }],
        ),
        Vec::new(),
    )
    .unwrap();
    let id = only_object(&doc);
    let (_, change) = doc.thicken_sheet(id, 0.0, 0.02).unwrap();
    assert_eq!(change.instances_touched.len(), 3);
    assert!(watertight(&doc, id));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any rectangle, in any orientation, pulled any thickness either way
    /// from any start, is a closed box of that area times that thickness.
    #[test]
    fn any_pane_pulls_into_a_box_of_its_area_times_the_thickness(
        w in 0.01f64..20.0,
        d in 0.01f64..20.0,
        start in -1.0f64..1.0,
        thickness in prop_oneof![-2.0f64..-0.001, 0.001f64..2.0],
        yaw in 0.0f64..std::f64::consts::TAU,
        pitch in 0.0f64..std::f64::consts::TAU,
    ) {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let turn = |p: Point3| {
            let (x, y, z) = (p.x * cy - p.y * sy, p.x * sy + p.y * cy, p.z);
            Point3::new(x, y * cp - z * sp, y * sp + z * cp)
        };
        let corners = box_corners(w, d, 0.0)[..4].iter().map(|&p| turn(p)).collect();
        let mut doc = Document::new();
        ingest(&mut doc, vec![ImportNode::Mesh(mesh("Pane", corners, vec![vec![0, 1, 2, 3]]))]);
        let id = only_object(&doc);
        doc.thicken_sheet(id, start, start + thickness).unwrap();
        prop_assert!(watertight(&doc, id));
        let v = signed_volume(&doc, id);
        let want = w * d * thickness.abs();
        prop_assert!((v - want).abs() <= 1e-9 * want.max(1.0), "volume {} vs {}", v, want);
    }
}

#[test]
fn a_sheet_beside_a_solid_thickens_and_the_solid_comes_along() {
    let mut positions = box_corners(1.0, 1.0, 1.0);
    positions.extend(
        box_corners(1.0, 1.0, 0.0)[..4]
            .iter()
            .map(|p| Point3::new(p.x + 3.0, p.y, p.z)),
    );
    let mut faces = box_faces();
    faces.push(vec![8, 9, 10, 11]);
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Crate", positions, faces))],
    );
    let id = only_object(&doc);
    assert_eq!(
        doc.object_health(id).unwrap().verdict,
        Verdict::FlatSheet { holes: 0 }
    );
    let (split_off, _) = doc.thicken_sheet(id, 0.0, 0.5).unwrap();
    assert_eq!(split_off.len(), 1);
    // The box (six faces) has the most faces, so it keeps the handle.
    assert!((signed_volume(&doc, id) - 1.0).abs() < 1e-9);
    assert!((signed_volume(&doc, split_off[0]) - 0.5).abs() < 1e-9);
    assert!(watertight(&doc, split_off[0]));
}

/// A unit box at `x0`, scaled by `s` about its corner, every face but
/// `drop` (6 keeps all), wound inward when `inside_out`.
fn box_mesh_at(x0: f64, s: f64, drop: usize, inside_out: bool) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let positions = box_corners(s, s, s)
        .into_iter()
        .map(|p| Point3::new(p.x + x0, p.y + x0.min(0.0), p.z + x0.min(0.0)))
        .collect();
    let faces = box_faces()
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i != drop)
        .map(|(_, mut f)| {
            if inside_out {
                f.reverse();
            }
            f
        })
        .collect();
    (positions, faces)
}

/// One mesh from several (positions concatenated, faces re-indexed).
fn merged(name: &str, parts: Vec<(Vec<Point3>, Vec<Vec<usize>>)>) -> MeshRecipe {
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    for (p, f) in parts {
        let base = positions.len();
        positions.extend(p);
        faces.extend(
            f.into_iter()
                .map(|f| f.into_iter().map(|i| i + base).collect::<Vec<_>>()),
        );
    }
    mesh(name, positions, faces)
}

#[test]
fn closing_an_inside_out_island_turns_only_that_island() {
    // A right-side-out closed box, and an inside-out open box beside it.
    let both = merged(
        "Pair",
        vec![
            box_mesh_at(0.0, 1.0, 6, false),
            box_mesh_at(3.0, 1.0, 1, true),
        ],
    );
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(both)]);
    let id = only_object(&doc);
    let (report, _) = doc.cap_rims(id, None).unwrap();
    assert!(report.closed && report.reversed);
    // Both right side out: the whole object's volume is both boxes'.
    assert!((signed_volume(&doc, id) - 2.0).abs() < 1e-9);
}

#[test]
fn a_closed_cavity_inside_a_box_stays_a_cavity() {
    // A 4 m box (closed, right side out) around an inside-out 1 m box
    // missing its lid: capped, the small one is a void, not a solid.
    let (mut inner_p, inner_f) = box_mesh_at(0.0, 1.0, 1, true);
    for p in &mut inner_p {
        *p = Point3::new(p.x + 1.5, p.y + 1.5, p.z + 1.5);
    }
    let shell = merged(
        "Shell",
        vec![box_mesh_at(0.0, 4.0, 6, false), (inner_p, inner_f)],
    );
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(shell)]);
    let id = only_object(&doc);
    let (report, _) = doc.cap_rims(id, None).unwrap();
    assert!(report.closed && !report.reversed);
    assert!((signed_volume(&doc, id) - (64.0 - 1.0)).abs() < 1e-9);
}

#[test]
fn a_cap_never_closes_over_a_warped_opening_on_its_plane() {
    // The pipe again, but the bore's top edge is warped (one corner
    // raised; triangle walls stay flat): capping the outer top would put a
    // face over the bore's opening.
    let mut positions = box_corners(2.0, 2.0, 1.0);
    let mut bore = box_corners(1.0, 1.0, 1.0);
    bore[6].z = 1.2;
    positions.extend(
        bore.into_iter()
            .map(|p| Point3::new(p.x + 0.5, p.y + 0.5, p.z)),
    );
    let mut faces: Vec<Vec<usize>> = box_faces().into_iter().skip(2).collect();
    for f in box_faces().into_iter().skip(2) {
        let q: Vec<usize> = f.into_iter().rev().map(|i| i + 8).collect();
        faces.push(vec![q[0], q[1], q[2]]);
        faces.push(vec![q[0], q[2], q[3]]);
    }
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Pipe", positions, faces))],
    );
    let id = only_object(&doc);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::CapsOverlap);
}

#[test]
fn removing_every_face_of_a_member_in_a_nested_group_deletes_it() {
    let mut doc = Document::new();
    doc.ingest(
        scene(
            vec![ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            }],
            vec![DefRecipe {
                name: Some("Sign".into()),
                children: vec![
                    ImportNode::Mesh(open_box()),
                    ImportNode::Group {
                        name: "Inner".into(),
                        children: vec![ImportNode::Mesh(pane())],
                        tags: Vec::new(),
                        hidden: false,
                        key: None,
                    },
                ],
            }],
        ),
        Vec::new(),
    )
    .unwrap();
    let sheet = doc
        .live_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some("Pane"))
        .unwrap();
    let all: Vec<FaceId> = doc.object(sheet).unwrap().faces().keys().collect();
    let (_, change) = doc.delete_faces(sheet, &all).unwrap();
    assert_eq!(change.instances_touched.len(), 1, "every placement redraws");
    assert!(doc.object(sheet).is_none(), "the inside wall is gone");
    assert_eq!(doc.live_object_ids().len(), 1, "the box stays");
    // It saves and reopens without the wall, and undo brings it back.
    let reopened = Document::load(&doc.save()).unwrap();
    assert_eq!(reopened.live_object_ids().len(), 1);
    doc.undo().unwrap();
    assert!(doc.object(sheet).is_some());
    let reopened = Document::load(&doc.save()).unwrap();
    assert_eq!(reopened.live_object_ids().len(), 2);
}

#[test]
fn a_gap_in_a_flat_plate_fills_with_a_face_facing_the_same_way() {
    // A 3 m plate with a 1 m square hole: the hole's rim runs through flat
    // faces facing up, so a face across it fills the gap.
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let mut plate = mesh(
        "Plate",
        vec![
            p(0.0, 0.0),
            p(3.0, 0.0),
            p(3.0, 3.0),
            p(0.0, 3.0),
            p(1.0, 1.0),
            p(2.0, 1.0),
            p(2.0, 2.0),
            p(1.0, 2.0),
        ],
        vec![vec![0, 1, 2, 3]],
    );
    plate.face_holes = vec![vec![vec![4, 7, 6, 5]]];
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(plate)]);
    let id = only_object(&doc);
    let d = doc.object_health(id).unwrap();
    let hole = d.rims.iter().position(|r| r.extent < 2.0).unwrap();
    let outer = 1 - hole;
    // The plate's outer edge is still a sheet's edge: no cap on its back.
    let err = doc.cap_rims(id, Some(&[outer])).unwrap_err();
    assert_eq!(repair_err(err), RepairError::CapWouldBeCoplanar);
    let (report, _) = doc.cap_rims(id, Some(&[hole])).unwrap();
    assert_eq!(report.faces.len(), 1);
    assert!(!report.closed);
    let obj = doc.object(id).unwrap();
    assert!((obj.faces()[report.faces[0]].plane.normal().z - 1.0).abs() < 1e-9);
    assert_eq!(
        doc.object_health(id).unwrap().verdict,
        Verdict::FlatSheet { holes: 0 }
    );
}

#[test]
fn a_cap_never_covers_a_face_already_on_its_plane() {
    // A box missing its lid, with a loose tile lying in the opening's
    // plane inside it: a lid there would sit on the tile.
    let tile: Vec<Point3> = box_corners(0.2, 0.2, 0.0)[..4]
        .iter()
        .map(|p| Point3::new(p.x + 0.4, p.y + 0.4, 1.0))
        .collect();
    let both = merged(
        "Tray",
        vec![
            box_mesh_at(0.0, 1.0, 1, false),
            (tile, vec![vec![0, 1, 2, 3]]),
        ],
    );
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(both)]);
    let id = only_object(&doc);
    let d = doc.object_health(id).unwrap();
    let lid = d.rims.iter().position(|r| r.extent > 1.0).unwrap();
    let err = doc.cap_rims(id, Some(&[lid])).unwrap_err();
    assert_eq!(repair_err(err), RepairError::CapsOverlap);
}

#[test]
fn a_basin_with_a_flat_lip_is_a_surface_not_an_open_box() {
    // A box missing its lid with a flat lip around its top edge: the only
    // opening is the lip's outer edge, and a face across it would lie on
    // the lip's back.
    let mut positions = box_corners(1.0, 1.0, 1.0);
    positions.extend([
        Point3::new(-0.5, -0.5, 1.0),
        Point3::new(1.5, -0.5, 1.0),
        Point3::new(1.5, 1.5, 1.0),
        Point3::new(-0.5, 1.5, 1.0),
    ]);
    let mut faces: Vec<Vec<usize>> = box_faces()
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i != 1)
        .map(|(_, f)| f)
        .collect();
    faces.extend([
        vec![4, 5, 9, 8],
        vec![5, 6, 10, 9],
        vec![6, 7, 11, 10],
        vec![7, 4, 8, 11],
    ]);
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![ImportNode::Mesh(mesh("Basin", positions, faces))],
    );
    let id = only_object(&doc);
    let d = doc.object_health(id).unwrap();
    assert_eq!(d.rims.len(), 1);
    assert!(d.rims[0].plane.is_some() && d.rims[0].sheet_edge);
    assert_eq!(d.verdict, Verdict::BentSheet);
    let err = doc.cap_rims(id, None).unwrap_err();
    assert_eq!(repair_err(err), RepairError::CapWouldBeCoplanar);
}

#[test]
fn a_solid_resting_on_the_opening_does_not_stop_the_lid() {
    // A box missing its lid, and a closed block sitting on the opening's
    // plane inside its outline (one object, two islands): the lid closes;
    // the block only touches it.
    let (mut block_p, block_f) = box_mesh_at(0.0, 0.5, 6, false);
    for p in &mut block_p {
        *p = Point3::new(p.x + 0.25, p.y + 0.25, p.z + 1.0);
    }
    let both = merged(
        "Crate",
        vec![box_mesh_at(0.0, 1.0, 1, false), (block_p, block_f)],
    );
    let mut doc = Document::new();
    ingest(&mut doc, vec![ImportNode::Mesh(both)]);
    let id = only_object(&doc);
    let (report, _) = doc.cap_rims(id, None).unwrap();
    assert!(report.closed);
    assert!((signed_volume(&doc, id) - (1.0 + 0.125)).abs() < 1e-9);
}
