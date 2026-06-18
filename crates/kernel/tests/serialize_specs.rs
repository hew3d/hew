//! Executable specs for the native file format — `Document::save`/`load`
//! (HEW_FILE_FORMAT.md, ARCHITECTURE.md). The whole-document contract;
//! the per-object geometry buffer is spec'd in `op_specs.rs` (`serialize_*`).
//!
//! Every test is `#[ignore]`d because `save`/`load` are `todo!()` stubs. The
//! implementer (docs/DEVELOPMENT.md) un-ignores each in the SAME PR that makes
//! it pass, and never weakens an assertion to make it green (DEVELOPMENT.md rule 5).
//!
//! Cross-save/load identity is judged by *structure*, never by handle value:
//! slotmap handles and dense ids are free to differ; geometry, tree shape,
//! poses, materials, and the consumed set are not.

use kernel::{
    Document, ImageFormat, Material, NodeId, Object, Plane, Point3, Profile, Rgba8, Texture,
    Transform, WatertightState,
};

// ----------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane")
}

/// Extrude an axis-aligned box (base on `z = z_base`, swept up by `height`).
fn extrude_box(
    doc: &mut Document,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    z_base: f64,
    height: f64,
) -> kernel::ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z_base),
        Point3::new(1.0, 0.0, z_base),
        Point3::new(0.0, 1.0, z_base),
    ])
    .expect("offset plane");
    let s = doc.add_sketch(plane);
    let corners = [
        (Point3::new(x0, y0, z_base), Point3::new(x1, y0, z_base)),
        (Point3::new(x1, y0, z_base), Point3::new(x1, y1, z_base)),
        (Point3::new(x1, y1, z_base), Point3::new(x0, y1, z_base)),
        (Point3::new(x0, y1, z_base), Point3::new(x0, y0, z_base)),
    ];
    {
        let sk = doc.sketch_mut(s).expect("sketch live");
        for (a, b) in corners {
            sk.add_segment(a, b).expect("box segment");
        }
    }
    let r = doc.extrudable_regions(s).expect("regions")[0];
    doc.extrude_region(s, r, height).expect("extrude box").0
}

/// Multiset-of-faces equality (cyclic position match, winding preserved), the
/// same notion `op_specs`/`document_specs` use.
fn objects_equivalent(x: &Object, y: &Object) -> bool {
    fn polygons_of(obj: &Object) -> Vec<Vec<Point3>> {
        obj.faces()
            .iter()
            .map(|(_, f)| obj.loop_positions(f.outer_loop).collect::<Vec<_>>())
            .collect()
    }
    fn cyclic_match(a: &[Point3], b: &[Point3]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let n = a.len();
        (0..n).any(|off| {
            (0..n).all(|i| {
                let p = a[i];
                let q = b[(i + off) % n];
                (p.x - q.x).abs() < kernel::tol::POINT_MERGE
                    && (p.y - q.y).abs() < kernel::tol::POINT_MERGE
                    && (p.z - q.z).abs() < kernel::tol::POINT_MERGE
            })
        })
    }
    let xs = polygons_of(x);
    let mut ys = polygons_of(y);
    if xs.len() != ys.len() {
        return false;
    }
    for poly in &xs {
        match ys.iter().position(|cand| cyclic_match(poly, cand)) {
            Some(i) => {
                ys.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

/// The lone visible object of a freshly loaded single-object document.
fn only_visible(doc: &Document) -> &Object {
    let ids = doc.visible_object_ids();
    assert_eq!(ids.len(), 1, "expected exactly one visible object");
    doc.object(ids[0]).expect("visible object live")
}

fn affine_approx(a: [f64; 12], b: [f64; 12]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

// --------------------------------------------------------- geometry round-trip

#[test]
//#[ignore = "spec for Document::save/load: a single box round-trips"]
fn save_load_round_trips_a_single_box() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let original = doc.object(id).unwrap().clone();

    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load");

    let restored = only_visible(&loaded);
    assert!(objects_equivalent(restored, &original));
    assert_eq!(restored.watertight(), WatertightState::Watertight);
}

#[test]
//#[ignore = "spec for Document::save/load: multiple objects (separate buffers) round-trip"]
fn save_load_preserves_multiple_objects() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 5.0, 5.0, 7.0, 6.0, 0.0, 3.0);
    let oa = doc.object(a).unwrap().clone();
    let ob = doc.object(b).unwrap().clone();

    let loaded = Document::load(&doc.save()).expect("load");
    let ids = loaded.visible_object_ids();
    assert_eq!(ids.len(), 2, "both objects round-trip");

    // Each original reappears exactly once (geometry preserved; handles differ).
    let loaded_objs: Vec<&Object> = ids.iter().map(|&i| loaded.object(i).unwrap()).collect();
    for want in [&oa, &ob] {
        assert!(
            loaded_objs.iter().any(|got| objects_equivalent(got, want)),
            "object geometry preserved across the round-trip"
        );
    }
}

/// A 4×4 square with a centered 2×2 hole (hole wound CW seen from +z), on the
/// ground plane — its extrusion is a holed solid (top & bottom faces carry an
/// inner loop).
fn washer_profile() -> Profile {
    Profile::new(
        ground(),
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
        ],
        vec![vec![
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
        ]],
    )
    .expect("washer profile")
}

/// Adversarial coverage the architect added in review: every spec/golden object
/// is hole-free, leaving the geometry buffer's inner-loop (hole) encode/decode
/// path and `from_faces_with_holes`' hole rebuild completely untested. A holed
/// solid must round-trip through the *object buffer* with its holes intact.
#[test]
fn geometry_buffer_round_trips_a_holed_solid() {
    let washer = Object::from_extrusion(&washer_profile(), 1.0).expect("extrude washer");
    assert_eq!(washer.watertight(), WatertightState::Watertight);
    let holed_faces = washer
        .faces()
        .iter()
        .filter(|(_, f)| !f.inner_loops.is_empty())
        .count();
    assert!(holed_faces >= 2, "washer has holed top and bottom faces");

    // No materials -> the dense-material closures are never invoked.
    let bytes = washer.encode(&|_| 0);
    assert_eq!(bytes, washer.encode(&|_| 0), "encode is deterministic");
    let decoded = Object::decode(&bytes, &|_| None).expect("decode washer");
    decoded.validate().expect("rebuilt washer is valid");

    assert!(
        objects_equivalent(&decoded, &washer),
        "holed geometry (incl. inner loops) survives the round-trip"
    );
    assert_eq!(decoded.watertight(), WatertightState::Watertight);
    assert_eq!(
        decoded
            .faces()
            .iter()
            .filter(|(_, f)| !f.inner_loops.is_empty())
            .count(),
        holed_faces,
        "the same number of holed faces survive"
    );
}

#[test]
//#[ignore = "spec for Document::save: byte-for-byte determinism (golden-file contract)"]
fn save_is_byte_deterministic() {
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    extrude_box(&mut doc, 3.0, 3.0, 4.0, 4.0, 0.0, 2.0);
    assert_eq!(doc.save(), doc.save(), "save must be byte-deterministic");
}

#[test]
//#[ignore = "spec for Document::load: undo history is not persisted"]
fn loaded_document_has_empty_undo_stack() {
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    assert!(doc.can_undo());

    let loaded = Document::load(&doc.save()).expect("load");
    assert!(
        !loaded.can_undo(),
        "a freshly loaded document has no undo history"
    );
}

// --------------------------------------------------------------- tree round-trip

#[test]
//#[ignore = "spec for Document::save/load: merge-group membership round-trips"]
fn save_load_preserves_group_structure() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    doc.group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");

    let loaded = Document::load(&doc.save()).expect("load");

    assert_eq!(
        loaded.group_ids().len(),
        1,
        "one group survives the round-trip"
    );
    let g = loaded.group_ids()[0];
    assert_eq!(
        loaded.group_members(g).expect("members").len(),
        2,
        "the group still has both members"
    );
    assert_eq!(loaded.visible_object_ids().len(), 2);
}

#[test]
//#[ignore = "spec for Document::save/load: component def + instance poses round-trip"]
fn save_load_preserves_component_instances_with_poses() {
    let mut doc = Document::new();
    let base = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (component, inst0, _) = doc
        .make_component(&[NodeId::Object(base)])
        .expect("component");

    // A second instance with a mirrored (det < 0) pose — a pose is never baked,
    // so reflection must survive.
    let mirror =
        Transform::from_affine(&[-1.0, 0.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let (inst1, _) = doc
        .place_instance(component, mirror)
        .expect("place mirrored");

    let pose0 = doc.instance_pose(inst0).unwrap().to_affine();
    let pose1 = doc.instance_pose(inst1).unwrap().to_affine();

    let loaded = Document::load(&doc.save()).expect("load");

    assert_eq!(
        loaded.component_ids().len(),
        1,
        "the definition round-trips"
    );
    let mut poses: Vec<[f64; 12]> = loaded
        .instance_ids()
        .iter()
        .map(|&i| loaded.instance_pose(i).unwrap().to_affine())
        .collect();
    assert_eq!(poses.len(), 2, "both instances round-trip");
    // Each saved pose must reappear (order is not guaranteed).
    for want in [pose0, pose1] {
        let idx = poses
            .iter()
            .position(|p| affine_approx(*p, want))
            .expect("pose preserved exactly (incl. the mirror)");
        poses.swap_remove(idx);
    }
}

// ----------------------------------------------------------- materials round-trip

#[test]
//#[ignore = "spec for Document::save/load: palette, per-face material, and object base round-trip"]
fn save_load_preserves_materials_and_base() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);

    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 50, 40)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 60, 200)));

    let face = doc.object(o).unwrap().faces().keys().next().unwrap();
    doc.paint_face(o, face, Some(red)).expect("paint");
    doc.set_object_material(o, Some(blue))
        .expect("base material");

    let loaded = Document::load(&doc.save()).expect("load");
    let obj_id = loaded.visible_object_ids()[0];
    let obj = loaded.object(obj_id).unwrap();

    // Palette preserved by content (handles differ).
    let names: Vec<&str> = loaded
        .material_ids()
        .iter()
        .map(|&m| loaded.material(m).unwrap().name.as_str())
        .collect();
    assert!(names.contains(&"Red") && names.contains(&"Blue"));

    // Exactly one painted face, resolving to a "Red" material.
    let painted: Vec<_> = obj.faces().iter().filter_map(|(_, f)| f.material).collect();
    assert_eq!(painted.len(), 1, "the one painted face survives");
    assert_eq!(
        loaded.material(painted[0]).unwrap().color,
        Rgba8::rgb(220, 50, 40)
    );

    // Object base material resolves to "Blue".
    let base = obj.default_material().expect("base material set");
    assert_eq!(loaded.material(base).unwrap().name, "Blue");
}

#[test]
//#[ignore = "spec for Document::save/load: texture image bytes + world_size round-trip verbatim"]
fn save_load_preserves_textured_material() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let image = vec![0x89u8, 0x50, 0x4e, 0x47, 1, 2, 3, 4, 5]; // opaque blob to the kernel
    let tex = Texture {
        image: image.clone(),
        format: ImageFormat::Png,
        world_size: [0.5, 1.5],
    };
    let wood = doc.add_material(Material::textured("Wood", Rgba8::rgb(180, 120, 60), tex));
    let face = doc.object(o).unwrap().faces().keys().next().unwrap();
    doc.paint_face(o, face, Some(wood)).expect("paint");

    let loaded = Document::load(&doc.save()).expect("load");
    let m = loaded
        .material_ids()
        .into_iter()
        .map(|id| loaded.material(id).unwrap())
        .find(|m| m.name == "Wood")
        .expect("textured material survives");
    let t = m.texture.as_ref().expect("texture survives");
    assert_eq!(t.image, image, "image bytes are stored verbatim");
    assert_eq!(t.format, ImageFormat::Png);
    assert_eq!(t.world_size, [0.5, 1.5]);
}

// --------------------------------------------------------------- sketch round-trip

#[test]
//#[ignore = "spec for Document::save/load: sketches and the consumed-region set round-trip"]
fn save_load_preserves_sketch_and_consumed_region() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    // Two disjoint rectangles -> two regions; extrude (consume) one.
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
            (Point3::new(2.0, 0.0, 0.0), Point3::new(3.0, 0.0, 0.0)),
            (Point3::new(3.0, 0.0, 0.0), Point3::new(3.0, 1.0, 0.0)),
            (Point3::new(3.0, 1.0, 0.0), Point3::new(2.0, 1.0, 0.0)),
            (Point3::new(2.0, 1.0, 0.0), Point3::new(2.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("segment");
        }
    }
    let regions = doc.extrudable_regions(s).expect("regions");
    assert_eq!(regions.len(), 2);
    doc.extrude_region(s, regions[0], 1.0).expect("extrude one");
    // One region is now consumed; exactly one remains extrudable.
    assert_eq!(doc.extrudable_regions(s).unwrap().len(), 1);

    let loaded = Document::load(&doc.save()).expect("load");

    let ls = loaded.sketch_ids();
    assert_eq!(ls.len(), 1, "the sketch round-trips");
    assert_eq!(
        loaded.extrudable_regions(ls[0]).unwrap().len(),
        1,
        "the consumed region is still consumed after load"
    );
}

// ------------------------------------------------------------------- rejection

#[test]
//#[ignore = "spec for Document::load: non-container bytes are rejected, never panic"]
fn load_rejects_garbage() {
    assert!(Document::load(&[]).is_err());
    assert!(Document::load(b"not a zip at all").is_err());
}

#[test]
//#[ignore = "spec for Document::load: a truncated container is rejected, never panic"]
fn load_rejects_truncation() {
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let bytes = doc.save();
    assert!(Document::load(&bytes[..bytes.len() / 2]).is_err());
}
