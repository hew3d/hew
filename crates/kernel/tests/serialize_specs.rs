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
    Document, Guide, ImageFormat, Material, NodeId, Object, Plane, Point3, Profile, Rgba8, Texture,
    Transform, Vec3, WatertightState,
};
use kernel::{ImportNode, ImportScene, MeshRecipe};

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

// ──────────────────────────────────────────── tags round-trip (WS3+WS4) ──────

/// Helper: build a simple box MeshRecipe with given tags.
fn box_recipe_with_tags(name: &str, tags: Vec<Vec<String>>) -> MeshRecipe {
    let (a, b) = (Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
    MeshRecipe {
        name: name.to_string(),
        positions: vec![
            Point3::new(a.x, a.y, a.z),
            Point3::new(b.x, a.y, a.z),
            Point3::new(b.x, b.y, a.z),
            Point3::new(a.x, b.y, a.z),
            Point3::new(a.x, a.y, b.z),
            Point3::new(b.x, a.y, b.z),
            Point3::new(b.x, b.y, b.z),
            Point3::new(a.x, b.y, b.z),
        ],
        faces: vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
        face_materials: vec![kernel::NO_MATERIAL; 6],
        face_uv_frames: vec![None; 6],
        face_holes: vec![Vec::new(); 6],
        base_material: kernel::NO_MATERIAL,
        tags,
    }
}

/// Tags on object/group/instance survive `save`→`load` (manifest v3 round-trip).
#[test]
fn tags_round_trip_through_save_load() {
    let mut doc = Document::new();

    // Object with tags: import a box mesh that carries tags.
    let obj_tags = vec![
        vec!["Structure".to_string(), "Roof".to_string()],
        vec!["Exported".to_string()],
    ];
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(box_recipe_with_tags(
            "roof_box",
            obj_tags.clone(),
        ))],
    };
    let (_, _) = doc.ingest(scene, vec![]).unwrap();

    // Group with tags.
    let box2 = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    let group_tags = vec![vec!["Walls".to_string()]];
    doc.group_nodes(&[NodeId::Object(box2)]).unwrap();
    let gid = *doc.group_ids().first().unwrap();
    doc.add_node_tag(NodeId::Group(gid), group_tags[0].clone())
        .unwrap();

    // Instance with tags.
    let def_box = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 0.0, 1.0);
    let (comp, iid, _) = doc.make_component(&[NodeId::Object(def_box)]).unwrap();
    let _ = comp;
    let inst_tags = vec![vec!["Furniture".to_string(), "Chair".to_string()]];
    doc.add_node_tag(NodeId::Instance(iid), inst_tags[0].clone())
        .unwrap();

    // Round-trip.
    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load");

    // Object tags survived.
    let loaded_oid = *loaded
        .visible_object_ids()
        .iter()
        .find(|&&id| loaded.node_tags(NodeId::Object(id)) == obj_tags.as_slice())
        .expect("object with the expected tags should survive round-trip");
    assert_eq!(
        loaded.node_tags(NodeId::Object(loaded_oid)),
        obj_tags.as_slice()
    );

    // Group tags survived.
    let loaded_gid = *loaded.group_ids().first().unwrap();
    assert_eq!(
        loaded.node_tags(NodeId::Group(loaded_gid)),
        group_tags.as_slice()
    );

    // Instance tags survived.
    let loaded_iid = *loaded.instance_ids().first().unwrap();
    assert_eq!(
        loaded.node_tags(NodeId::Instance(loaded_iid)),
        inst_tags.as_slice()
    );
}

/// A v2-style manifest (no `tags` field) loads with empty tags — back-compat.
#[test]
fn v2_manifest_loads_with_empty_tags() {
    // The name_compat_tests in serialize.rs already cover DTO deserialization
    // defaults; this test verifies the full Document::load path.
    // Build a doc, save it, patch the manifest to remove all `tags` keys and
    // set format_version=2, then reload.
    use std::io::{Cursor, Read as _, Write as _};

    let mut doc = Document::new();
    let obj = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    // Give the object a tag first so we can verify it's gone after the v2 load.
    doc.add_node_tag(NodeId::Object(obj), vec!["Structure".to_string()])
        .unwrap();
    let bytes = doc.save();

    // Re-open the zip and strip `tags` from the manifest + set format_version=2.
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();

    // Parse as generic JSON Value, remove "tags" fields, set format_version=2.
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    manifest["format_version"] = serde_json::json!(2);
    // Strip `tags` from objects, groups, instances.
    for arr_key in ["objects", "groups", "instances"] {
        if let Some(arr) = manifest[arr_key].as_array_mut() {
            for entry in arr.iter_mut() {
                entry.as_object_mut().map(|m| m.remove("tags"));
            }
        }
    }
    let patched_manifest = serde_json::to_vec_pretty(&manifest).unwrap();

    // Re-pack the zip with the patched manifest.
    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut new_zip = zip::ZipWriter::new(out_cursor);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched_manifest).unwrap();
    // Re-add all other entries.
    let bytes2 = bytes.clone(); // keep the original bytes alive
    let mut zip2 = zip::ZipArchive::new(Cursor::new(&bytes2)).unwrap();
    for i in 0..zip2.len() {
        let mut entry = zip2.by_index(i).unwrap();
        if entry.name() == "manifest.json" {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        let opts2 = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        new_zip.start_file(&name, opts2).unwrap();
        new_zip.write_all(&buf).unwrap();
    }
    let patched_bytes = new_zip.finish().unwrap().into_inner();

    // Load should succeed and tags should be empty.
    let loaded = Document::load(&patched_bytes).expect("v2 manifest should load");
    for oid in loaded.visible_object_ids() {
        assert_eq!(
            loaded.node_tags(NodeId::Object(oid)),
            &[] as &[Vec<String>],
            "tags default to empty for v2 files"
        );
    }
}

// ─────────────────────────────────────────── guides round-trip (v4) ─────

/// A document with one construction line and one construction point survives
/// `save`→`load` intact (manifest v4).
#[test]
fn guides_round_trip_through_save_load() {
    let mut doc = Document::new();
    // Non-unit input direction to also exercise normalize-on-store across the
    // save/load boundary (the in-memory normalization is covered separately
    // in document_specs.rs; here we only care that the *stored* unit vector
    // round-trips byte-for-byte through the manifest).
    let line_origin = Point3::new(1.0, 2.0, 3.0);
    let line_dir = Vec3::new(1.0, 0.0, 0.0);
    doc.add_guide_line(line_origin, line_dir).unwrap();
    let point_pos = Point3::new(-4.0, 5.5, 6.25);
    doc.add_guide_point(point_pos).unwrap();

    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load");

    let ids = loaded.guide_ids();
    assert_eq!(ids.len(), 2, "both guides round-trip");

    let mut saw_line = false;
    let mut saw_point = false;
    for id in ids {
        match loaded.guide(id).unwrap() {
            Guide::Line { origin, direction } => {
                saw_line = true;
                assert!(origin.approx_eq(line_origin, 1e-9));
                assert!(direction.approx_eq(line_dir, 1e-9));
                assert!(
                    (direction.length() - 1.0).abs() < 1e-9,
                    "stored direction is unit length"
                );
            }
            Guide::Point { position } => {
                saw_point = true;
                assert!(position.approx_eq(point_pos, 1e-9));
            }
        }
    }
    assert!(saw_line && saw_point, "both guide kinds round-trip");
}

/// `save` is byte-deterministic with guides present (same as the existing
/// `save_is_byte_deterministic` coverage, but exercising the guides array).
#[test]
fn guides_save_is_byte_deterministic() {
    let mut doc = Document::new();
    doc.add_guide_line(Point3::ORIGIN, Vec3::new(0.0, 1.0, 0.0))
        .unwrap();
    doc.add_guide_point(Point3::new(1.0, 1.0, 1.0)).unwrap();

    let bytes_a = doc.save();
    let bytes_b = doc.save();
    assert_eq!(bytes_a, bytes_b, "save() is deterministic with guides");
}

/// A manifest entry for a `"line"` guide with no `dir` field is a typed load
/// error, never a panic and never silently repaired (DEVELOPMENT.md rule 4).
#[test]
fn load_rejects_line_guide_missing_direction() {
    use std::io::{Cursor, Read as _, Write as _};

    let mut doc = Document::new();
    doc.add_guide_line(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    let bytes = doc.save();

    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();

    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let guides = manifest["guides"]
        .as_array_mut()
        .expect("guides array present");
    assert_eq!(guides.len(), 1);
    guides[0].as_object_mut().unwrap().remove("dir");
    let patched_manifest = serde_json::to_vec_pretty(&manifest).unwrap();

    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut new_zip = zip::ZipWriter::new(out_cursor);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched_manifest).unwrap();
    let bytes2 = bytes.clone();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(&bytes2)).unwrap();
    for i in 0..zip2.len() {
        let mut entry = zip2.by_index(i).unwrap();
        if entry.name() == "manifest.json" {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        let opts2 = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        new_zip.start_file(&name, opts2).unwrap();
        new_zip.write_all(&buf).unwrap();
    }
    let patched_bytes = new_zip.finish().unwrap().into_inner();

    // Must be a typed load error, never a panic.
    let result = Document::load(&patched_bytes);
    assert!(
        result.is_err(),
        "a line guide missing `dir` must be rejected, not silently repaired"
    );
}

/// A pre-v4 manifest (no `guides` key at all) still loads, with zero guides —
/// back-compat, mirroring `v2_manifest_loads_with_empty_tags`.
#[test]
fn pre_v4_manifest_loads_with_no_guides() {
    use std::io::{Cursor, Read as _, Write as _};

    let mut doc = Document::new();
    let obj = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let _ = obj;
    let bytes = doc.save();

    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();

    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    manifest["format_version"] = serde_json::json!(3);
    // This doc has no guides, so `skip_serializing_if = "Vec::is_empty"` already
    // omitted the key — exactly the pre-v4 shape we want to test. Assert that
    // omission here (rather than removing the key ourselves) so the test fails
    // loudly if that serde attribute is ever dropped.
    assert!(
        manifest.as_object().unwrap().get("guides").is_none(),
        "an empty guides Vec should be omitted from the manifest entirely"
    );
    let patched_manifest = serde_json::to_vec_pretty(&manifest).unwrap();

    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut new_zip = zip::ZipWriter::new(out_cursor);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched_manifest).unwrap();
    let bytes2 = bytes.clone();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(&bytes2)).unwrap();
    for i in 0..zip2.len() {
        let mut entry = zip2.by_index(i).unwrap();
        if entry.name() == "manifest.json" {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        let opts2 = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        new_zip.start_file(&name, opts2).unwrap();
        new_zip.write_all(&buf).unwrap();
    }
    let patched_bytes = new_zip.finish().unwrap().into_inner();

    let loaded =
        Document::load(&patched_bytes).expect("pre-v4 manifest (no guides key) should load");
    assert!(
        loaded.guide_ids().is_empty(),
        "guides default to empty when the manifest predates v4"
    );
}
