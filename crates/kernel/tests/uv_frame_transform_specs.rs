//! A face's UV frame rides with the face (DEVELOPMENT.md rule 3 specs).
//!
//! A textured face placed by a UV frame (an imported texture, or a placed
//! image) keeps its picture where it was on the face when the face moves,
//! turns or scales: the frame is carried through every transform the
//! Object takes (`Object::apply_transform`, a component session's bake and
//! unbake, the Tape Measure's in-context resize).

use kernel::{
    DefRecipe, Document, ImageFormat, ImportNode, ImportScene, Material, MeshExtra, MeshRecipe,
    NO_MATERIAL, NodeId, Object, Point3, Rgba8, Texture, Transform, TransformError, UvFrame, Vec3,
};
use proptest::prelude::*;

const W: f64 = 2.0;
const H: f64 = 1.0;

/// A `W` x `H` quad in its definition's XY plane, painted with a picture
/// stretched over it once (the frame maps its corners to 0..1), placed by
/// `pose`.
fn framed_quad_scene(pose: Transform) -> ImportScene {
    let mat = Material::textured(
        "pic",
        Rgba8::rgba(255, 255, 255, 255),
        Texture {
            image: vec![1, 2, 3],
            format: ImageFormat::Png,
            world_size: [W, H],
        },
    );
    let mesh = MeshRecipe {
        name: "pic".into(),
        positions: vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(W, 0.0, 0.0),
            Point3::new(W, H, 0.0),
            Point3::new(0.0, H, 0.0),
        ],
        faces: vec![vec![0, 1, 2, 3]],
        face_materials: vec![0],
        face_uv_frames: vec![Some(UvFrame::new(
            Vec3::new(1.0 / W, 0.0, 0.0),
            Vec3::new(0.0, 1.0 / H, 0.0),
            0.0,
            0.0,
        ))],
        face_holes: vec![vec![]],
        base_material: NO_MATERIAL,
        tags: vec![],
        extra: MeshExtra::default(),
    };
    ImportScene {
        materials: vec![mat],
        defs: vec![DefRecipe::from_meshes(Some("pic".into()), vec![mesh])],
        roots: vec![ImportNode::Instance {
            def: 0,
            pose,
            name: None,
            tags: vec![],
            hidden: false,
            key: None,
        }],
        guides: vec![],
        tags: vec![],
        camera: None,
        scenes: vec![],
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    }
}

/// The texture coordinate at each outer corner of every framed face, by
/// corner order.
fn corner_uvs(obj: &Object) -> Vec<[f64; 2]> {
    obj.faces()
        .iter()
        .filter_map(|(_, f)| f.uv_frame.map(|fr| (f, fr)))
        .flat_map(|(f, fr)| {
            obj.loop_positions(f.outer_loop)
                .map(move |p| fr.apply(p))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn assert_uvs_close(got: &[[f64; 2]], want: &[[f64; 2]], tol: f64) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g[0] - w[0]).abs() < tol && (g[1] - w[1]).abs() < tol,
            "uv {g:?} is not {w:?} (all: {got:?})"
        );
    }
}

fn the_member(doc: &Document) -> kernel::ObjectId {
    let inst = doc.instance_ids()[0];
    let def = doc.instance_def(inst).unwrap();
    match doc.def_member_nodes(def).unwrap().as_slice() {
        [NodeId::Object(o)] => *o,
        other => panic!("one member expected, got {other:?}"),
    }
}

const CORNERS: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

#[test]
fn a_transformed_object_keeps_each_corners_texture_coordinate() {
    let mut doc = Document::new();
    doc.ingest(framed_quad_scene(Transform::IDENTITY), vec![])
        .unwrap();
    let obj = doc.object(the_member(&doc)).unwrap().clone();
    assert_uvs_close(&corner_uvs(&obj), &CORNERS, 1e-12);
    // Turned, stretched unevenly, and moved: still the same picture corners.
    let t = Transform::scale(Vec3::new(3.0, 0.5, 2.0))
        .then(&Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7).unwrap())
        .then(&Transform::translation(Vec3::new(5.0, -4.0, 1.5)));
    let mut moved = obj.clone();
    moved.apply_transform(&t).unwrap();
    assert_uvs_close(&corner_uvs(&moved), &CORNERS, 1e-9);
}

#[test]
fn opening_a_placed_picture_keeps_it_on_its_face() {
    let mut doc = Document::new();
    doc.ingest(
        framed_quad_scene(
            Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.4)
                .unwrap()
                .then(&Transform::translation(Vec3::new(5.0, 3.0, 0.0))),
        ),
        vec![],
    )
    .unwrap();
    let inst = doc.instance_ids()[0];
    let change = doc.open_explode_session(inst).unwrap();
    let baked: Vec<[f64; 2]> = change
        .objects_touched
        .iter()
        .filter_map(|&o| doc.object(o))
        .flat_map(corner_uvs)
        .collect();
    assert_uvs_close(&baked, &CORNERS, 1e-9);
    doc.close_explode_session().unwrap();
    let obj = doc.object(the_member(&doc)).unwrap();
    assert_uvs_close(&corner_uvs(obj), &CORNERS, 1e-9);
}

/// The Tape Measure's in-context resize scales the definition's face: the
/// picture stays stretched over the whole face, not tiled over it.
#[test]
fn resizing_a_placed_picture_keeps_it_stretched_over_the_face() {
    let mut doc = Document::new();
    doc.ingest(
        framed_quad_scene(Transform::translation(Vec3::new(5.0, 3.0, 0.0))),
        vec![],
    )
    .unwrap();
    let inst = doc.instance_ids()[0];
    doc.open_explode_session(inst).unwrap();
    doc.rescale_session(2.0, Point3::new(5.0, 3.0, 0.0))
        .unwrap();
    doc.close_explode_session().unwrap();
    let obj = doc.object(the_member(&doc)).unwrap();
    let span = obj
        .faces()
        .iter()
        .flat_map(|(_, f)| obj.loop_positions(f.outer_loop).collect::<Vec<_>>())
        .fold(0.0_f64, |m, p| m.max(p.x));
    assert!((span - 2.0 * W).abs() < 1e-9, "the face doubled: {span}");
    assert_uvs_close(&corner_uvs(obj), &CORNERS, 1e-9);
}

/// A shear is no similarity, and the frame still rides with the face: the
/// picture shears with it.
#[test]
fn a_sheared_object_keeps_each_corners_texture_coordinate() {
    let mut doc = Document::new();
    doc.ingest(framed_quad_scene(Transform::IDENTITY), vec![])
        .unwrap();
    let mut obj = doc.object(the_member(&doc)).unwrap().clone();
    let shear = Transform::from_affine(&[
        1.0, 0.8, 0.0, 2.0, //
        0.0, 1.0, 0.3, -1.0, //
        0.0, 0.0, 1.0, 0.5,
    ]);
    obj.apply_transform(&shear).unwrap();
    assert_uvs_close(&corner_uvs(&obj), &CORNERS, 1e-12);
}

/// A mirror is refused (it would turn a solid inside out), and the refusal
/// leaves the face's frame exactly as it was.
#[test]
fn a_reflection_is_refused_and_leaves_the_frame() {
    let mut doc = Document::new();
    doc.ingest(framed_quad_scene(Transform::IDENTITY), vec![])
        .unwrap();
    let mut obj = doc.object(the_member(&doc)).unwrap().clone();
    let before = obj.clone();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0));
    assert_eq!(
        obj.apply_transform(&mirror),
        Err(TransformError::Reflection)
    );
    assert_eq!(corner_uvs(&obj), corner_uvs(&before));
}

/// A stretch so extreme that the face's texture frame would no longer be
/// a frame (its gradient along the stretch past what it can hold, or its
/// axes sheared to parallel) is refused, typed, and the object is left
/// untouched; `check_transform` says the same without moving anything.
#[test]
fn an_extreme_stretch_that_would_squash_the_frame_is_refused() {
    let mut doc = Document::new();
    doc.ingest(framed_quad_scene(Transform::IDENTITY), vec![])
        .unwrap();
    let obj = doc.object(the_member(&doc)).unwrap().clone();
    let stretch = Transform::scale(Vec3::new(1e9, 1.0, 1.0));
    let shear = Transform::from_affine(&[
        1.0, 1e7, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ]);
    for t in [stretch, shear] {
        let mut moved = obj.clone();
        assert_eq!(
            moved.check_transform(&t),
            Err(TransformError::DegenerateTextureFrame)
        );
        assert_eq!(
            moved.apply_transform(&t),
            Err(TransformError::DegenerateTextureFrame)
        );
        let at = |o: &Object| -> Vec<Point3> {
            o.faces()
                .iter()
                .flat_map(|(_, f)| o.loop_positions(f.outer_loop).collect::<Vec<_>>())
                .collect()
        };
        assert_eq!(at(&moved), at(&obj), "no vertex moved");
        assert_eq!(corner_uvs(&moved), corner_uvs(&obj), "the frame is kept");
    }
    // A milder stretch of the same kind still carries the frame.
    let mut moved = obj.clone();
    moved
        .apply_transform(&Transform::scale(Vec3::new(1e3, 1.0, 1.0)))
        .unwrap();
    assert_uvs_close(&corner_uvs(&moved), &CORNERS, 1e-9);
}

/// A face whose texture frame is ALREADY invalid when it arrives (an
/// imported file can carry one: here its two axes are parallel) does not
/// block a move: the frame was not this map's doing, so a plain move or
/// scale goes ahead, and the frame rides along still invalid, neither
/// repaired nor dropped.
#[test]
fn an_already_invalid_frame_does_not_block_a_move() {
    let mut scene = framed_quad_scene(Transform::IDENTITY);
    let ImportNode::Mesh(mesh) = &mut scene.defs[0].children[0] else {
        panic!("the definition holds the quad");
    };
    let parallel = UvFrame::new(Vec3::new(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 0.0, 0.0);
    assert!(!parallel.is_valid());
    mesh.face_uv_frames = vec![Some(parallel)];
    let mut doc = Document::new();
    doc.ingest(scene, vec![]).unwrap();
    let obj = doc.object(the_member(&doc)).unwrap().clone();
    let frame = |o: &Object| o.faces().iter().next().unwrap().1.uv_frame;
    assert_eq!(frame(&obj), Some(parallel), "the frame arrived as it was");
    for t in [
        Transform::translation(Vec3::new(3.0, -1.0, 2.0)),
        Transform::scale(Vec3::new(2.0, 3.0, 1.0)),
    ] {
        let mut moved = obj.clone();
        assert_eq!(moved.check_transform(&t), Ok(()));
        moved.apply_transform(&t).unwrap();
        let carried = frame(&moved).expect("the frame rides along");
        assert!(!carried.is_valid(), "not repaired: {carried:?}");
    }
}

proptest! {
    /// Any invertible, orientation-keeping affine map carries the frame
    /// with the face.
    #[test]
    fn any_affine_map_carries_the_frame(
        sx in 0.1f64..10.0, sy in 0.1f64..10.0, sz in 0.1f64..10.0,
        ax in -1.0f64..1.0, ay in -1.0f64..1.0, angle in -3.0f64..3.0,
        tx in -50.0f64..50.0, ty in -50.0f64..50.0, tz in -50.0f64..50.0,
    ) {
        let mut doc = Document::new();
        doc.ingest(framed_quad_scene(Transform::IDENTITY), vec![]).unwrap();
        let mut obj = doc.object(the_member(&doc)).unwrap().clone();
        let t = Transform::scale(Vec3::new(sx, sy, sz))
            .then(&Transform::rotation(Vec3::new(ax, ay, 1.0), angle).unwrap())
            .then(&Transform::translation(Vec3::new(tx, ty, tz)));
        obj.apply_transform(&t).unwrap();
        assert_uvs_close(&corner_uvs(&obj), &CORNERS, 1e-7);
    }
}
