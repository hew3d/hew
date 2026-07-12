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
//! poses, materials, and sketch contents are not.

use kernel::{
    CurveGeom, DecodeError, Document, Guide, ImageFormat, Material, NodeId, Object, Plane, Point3,
    Profile, Rgba8, SurfaceRef, Texture, Transform, Vec3, WatertightState, tol,
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
fn imprinted_circle_edge_claim_round_trips_through_geometry_buffer_v5() {
    // A circle imprinted on a solid face carries its analytic identity onto
    // the solid edges (playtest fix C3). Geometry buffer v5 persists that
    // claim so a save between the imprint and its push-through does not lose
    // the circle — verified both directly and end-to-end (the reloaded disk
    // still pushes through as a smooth cylinder).
    let h = 1.0;
    let radius = 0.5;
    let center = Point3::new(0.0, 0.0, h);
    let rect = vec![
        Point3::new(-2.0, -2.0, 0.0),
        Point3::new(2.0, -2.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(-2.0, 2.0, 0.0),
    ];
    let mut obj =
        Object::from_extrusion(&Profile::new(ground(), rect, vec![]).unwrap(), h).unwrap();
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| (p.z - h).abs() < tol::POINT_MERGE)
        })
        .map(|(id, _)| id)
        .expect("top cap");
    let circle: Vec<Point3> = (0..24)
        .map(|i| {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / 24.0;
            Point3::new(radius * a.cos(), radius * a.sin(), h)
        })
        .collect();
    obj.split_face_inner_with_curve(top, &circle, Some(CurveGeom { center, radius }))
        .unwrap();
    assert_eq!(
        obj.edges().values().filter(|e| e.curve.is_some()).count(),
        24,
        "the imprinted circle's 24 edges carry the claim"
    );

    let bytes = obj.encode(&|_| 0);
    assert_eq!(bytes, obj.encode(&|_| 0), "encode is deterministic");
    let decoded = Object::decode(&bytes, &|_| None).expect("decode");
    decoded.validate().unwrap();

    // The claim survives, bitwise.
    assert_eq!(
        decoded
            .edges()
            .values()
            .filter(|e| e.curve.is_some())
            .count(),
        24,
        "the edge claim survives the geometry-buffer v5 round-trip"
    );
    let g = decoded.edges().values().find_map(|e| e.curve).unwrap();
    assert_eq!(g.radius, radius);
    assert!(g.center.approx_eq(center, tol::POINT_MERGE));

    // End-to-end: the RELOADED disk still pushes through as a cylinder.
    let disk = decoded
        .faces()
        .iter()
        .find(|(_, f)| {
            f.inner_loops.is_empty()
                && f.plane
                    .normal()
                    .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
                && decoded.loop_positions(f.outer_loop).count() == 24
        })
        .map(|(id, _)| id)
        .expect("the reloaded disk sub-face");
    let drilled = decoded.push_through(disk, -(h + 1.0)).unwrap();
    let attributed = drilled
        .faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
        .count();
    assert_eq!(
        attributed, 24,
        "the reloaded imprint drills a smooth tunnel"
    );
}

#[test]
fn disagreeing_shared_edge_claims_are_rejected_on_load() {
    // Validating loader (adversarial review, major): a v5 buffer stores each
    // edge claim twice — once per incident face. A tampered/inconsistent file
    // whose two incident faces claim DIFFERENT circles for one shared edge
    // must be rejected typed, never silently resolved to whichever face loads
    // last. The circle center is offset so the (distinctive) radius appears in
    // the bytes ONLY as an edge-curve radius, never as a coordinate.
    let h = 1.0;
    let radius = 0.4321;
    let center = Point3::new(1.234, 0.567, h);
    let rect = vec![
        Point3::new(-2.0, -2.0, 0.0),
        Point3::new(2.0, -2.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(-2.0, 2.0, 0.0),
    ];
    let mut obj =
        Object::from_extrusion(&Profile::new(ground(), rect, vec![]).unwrap(), h).unwrap();
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| (p.z - h).abs() < tol::POINT_MERGE)
        })
        .map(|(id, _)| id)
        .expect("top cap");
    let circle: Vec<Point3> = (0..24)
        .map(|i| {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / 24.0;
            Point3::new(center.x + radius * a.cos(), center.y + radius * a.sin(), h)
        })
        .collect();
    obj.split_face_inner_with_curve(top, &circle, Some(CurveGeom { center, radius }))
        .unwrap();

    let mut bytes = obj.encode(&|_| 0);
    // A clean round-trip loads.
    assert!(Object::decode(&bytes, &|_| None).is_ok());

    // Tamper the LAST stored radius so one incident face disagrees with its
    // twin. (The radius appears only in edge-curve blocks with this center.)
    let pat = radius.to_le_bytes();
    let pos = bytes
        .windows(8)
        .rposition(|w| w == pat)
        .expect("the stored radius is present in the buffer");
    bytes[pos..pos + 8].copy_from_slice(&(radius + 0.1).to_le_bytes());

    // The loader rejects the disagreement typed — not silent last-writer-wins.
    match Object::decode(&bytes, &|_| None) {
        Err(DecodeError::Corrupt { what, .. }) => {
            assert!(
                what.contains("disagreeing"),
                "rejected for the wrong reason: {what}"
            );
        }
        other => panic!("expected a typed corruption error, got {other:?}"),
    }
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

// ------------------------------------------------------------ curve round-trip

/// Curve-chain membership survives save → load: the arc's facets come back
/// sharing one curve id, distinct from a second curve's, and plain lines
/// stay plain (manifest v7).
#[test]
fn curve_chains_round_trip_through_save_load() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
            .unwrap(); // plain
        sk.begin_curve();
        sk.add_segment(Point3::new(0.0, 1.0, 0.0), Point3::new(0.5, 1.2, 0.0))
            .unwrap();
        sk.add_segment(Point3::new(0.5, 1.2, 0.0), Point3::new(1.0, 1.0, 0.0))
            .unwrap();
        sk.end_curve();
        sk.begin_curve();
        sk.add_segment(Point3::new(0.0, 2.0, 0.0), Point3::new(0.5, 2.2, 0.0))
            .unwrap();
        sk.end_curve();
    }

    let bytes = doc.save();
    let doc2 = Document::load(&bytes).expect("round-trip");
    let s2 = doc2.sketch_ids()[0];
    let sk2 = doc2.sketch(s2).expect("live");

    let mut by_curve: std::collections::BTreeMap<Option<kernel::SketchCurveId>, usize> =
        std::collections::BTreeMap::new();
    for e in sk2.edges().values() {
        *by_curve.entry(e.curve).or_insert(0) += 1;
    }
    assert_eq!(by_curve.get(&None), Some(&1), "one plain line");
    let curve_sizes: Vec<usize> = by_curve
        .iter()
        .filter(|(k, _)| k.is_some())
        .map(|(_, &n)| n)
        .collect();
    assert_eq!(
        curve_sizes.iter().sum::<usize>(),
        3,
        "three curve-tagged edges"
    );
    assert!(
        curve_sizes.contains(&2) && curve_sizes.contains(&1),
        "two DISTINCT curves of sizes 2 and 1, not merged"
    );

    // Byte-stable: saving the loaded doc reproduces the bytes.
    assert_eq!(doc2.save(), bytes, "deterministic re-save");
}

// --------------------------------------------------------------- sketch round-trip

#[test]
fn save_load_preserves_sketch_after_partial_consumption() {
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
    // The extruded region's scaffolding is deleted; the sibling remains.
    assert_eq!(doc.extrudable_regions(s).unwrap().len(), 1);

    let loaded = Document::load(&doc.save()).expect("load");

    let ls = loaded.sketch_ids();
    assert_eq!(ls.len(), 1, "the sketch round-trips");
    assert_eq!(
        loaded.sketch(ls[0]).expect("live").edges().len(),
        4,
        "only the surviving rectangle's edges persist"
    );
    assert_eq!(
        loaded.extrudable_regions(ls[0]).unwrap().len(),
        1,
        "the surviving region is extrudable after load"
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
        guides: Vec::new(),
        tags: Vec::new(),
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

/// Tag METADATA (the registry of known tags + hidden flags, manifest v5)
/// round-trips: registered tags — including ones no node carries — survive
/// save/load with their hidden-by-default flags.
#[test]
fn tag_metadata_round_trips_through_save_load() {
    let mut doc = Document::new();

    let mock = vec!["Mock Walls".to_string()];
    let carpet = vec!["Carpet".to_string()];
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(box_recipe_with_tags(
            "carpet_box",
            vec![carpet.clone()],
        ))],
        guides: Vec::new(),
        tags: vec![
            kernel::ImportTag {
                path: mock.clone(),
                hidden: true,
            },
            kernel::ImportTag {
                path: carpet.clone(),
                hidden: false,
            },
        ],
    };
    doc.ingest(scene, vec![]).unwrap();
    assert!(doc.tag_hidden(&mock));
    assert!(!doc.tag_hidden(&carpet));

    // A user toggle persists too (registering unknown paths on the way).
    doc.set_tag_hidden(vec!["Later".to_string()], true);

    let loaded = Document::load(&doc.save()).expect("load");
    assert!(loaded.tag_hidden(&mock), "hidden flag survives reload");
    assert!(!loaded.tag_hidden(&carpet));
    assert!(loaded.tag_hidden(&["Later".to_string()]));
    // The empty "Mock Walls" layer survives even with no content on it.
    assert_eq!(loaded.tag_meta().count(), 3);
}

/// Undoing an import unregisters exactly the tags it added; redo restores
/// them. A pre-existing tag's hidden flag is never clobbered by an import.
#[test]
fn import_tag_registration_is_undone_and_never_clobbers() {
    let mut doc = Document::new();
    // The user already decided "Carpet" is hidden.
    doc.set_tag_hidden(vec!["Carpet".to_string()], true);

    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(box_recipe_with_tags(
            "b",
            vec![vec!["Carpet".to_string()]],
        ))],
        guides: Vec::new(),
        tags: vec![
            kernel::ImportTag {
                path: vec!["Carpet".to_string()],
                hidden: false, // visible in the source — must NOT clobber
            },
            kernel::ImportTag {
                path: vec!["Mock".to_string()],
                hidden: true,
            },
        ],
    };
    doc.ingest(scene, vec![]).unwrap();
    assert!(
        doc.tag_hidden(&["Carpet".to_string()]),
        "import must not flip a user-chosen hidden flag"
    );
    assert!(doc.tag_hidden(&["Mock".to_string()]));

    doc.undo().unwrap();
    assert_eq!(
        doc.tag_meta().count(),
        1,
        "undo removes only the import-registered tag"
    );
    assert!(doc.tag_hidden(&["Carpet".to_string()]));

    doc.redo().unwrap();
    assert!(doc.tag_hidden(&["Mock".to_string()]));
    assert_eq!(doc.tag_meta().count(), 2);
}

/// USER-hidden view state (manifest v6) round-trips: a node hidden via
/// set_node_user_hidden stays hidden across save/load, and an import
/// carrying hidden nodes seeds the registry.
#[test]
fn user_hidden_round_trips_through_save_load() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    doc.set_node_user_hidden(NodeId::Object(a), true);
    assert!(doc.node_user_hidden(NodeId::Object(a)));
    assert!(!doc.node_user_hidden(NodeId::Object(b)));

    let loaded = Document::load(&doc.save()).expect("load");
    let hidden_nodes = loaded.user_hidden_nodes();
    assert_eq!(hidden_nodes.len(), 1, "exactly one node stays user-hidden");
    assert!(matches!(hidden_nodes[0], NodeId::Object(_)));
    // Unhide persists too.
    let mut loaded = loaded;
    loaded.set_node_user_hidden(hidden_nodes[0], false);
    let again = Document::load(&loaded.save()).expect("reload");
    assert!(again.user_hidden_nodes().is_empty());
}

/// An import whose nodes carry `hidden` (a `.skp` hidden group/component)
/// materializes them user-hidden — imported, never dropped.
#[test]
fn import_hidden_nodes_arrive_user_hidden() {
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Group {
            name: "Drains".into(),
            children: vec![ImportNode::Mesh(box_recipe_with_tags("pipe", vec![]))],
            tags: Vec::new(),
            hidden: true,
        }],
        guides: Vec::new(),
        tags: Vec::new(),
    };
    let (report, _) = doc.ingest(scene, vec![]).unwrap();
    assert_eq!(report.objects_created, 1, "hidden content still imports");
    let hidden = doc.user_hidden_nodes();
    assert_eq!(hidden.len(), 1);
    assert!(matches!(hidden[0], NodeId::Group(_)));
    // And it persists.
    let loaded = Document::load(&doc.save()).expect("load");
    assert_eq!(loaded.user_hidden_nodes().len(), 1);
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

/// Rewrites a saved container's manifest through `patch` and returns the
/// re-zipped bytes — the shared scaffolding for old-manifest-shape specs.
fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};

    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    patch(&mut manifest);
    let patched_manifest = serde_json::to_vec_pretty(&manifest).unwrap();

    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut new_zip = zip::ZipWriter::new(out_cursor);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched_manifest).unwrap();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
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
    new_zip.finish().unwrap().into_inner()
}

/// Older manifests (≤ v10) stored sketch–solid claim data: a top-level
/// `consumed` list, per-object `footprints` polygons (v9/v10), and a v8
/// `source` pair — and they persisted consumed outlines as ordinary sketch
/// edges (tombstoned at runtime, not deleted). A v11 loader honors the
/// `consumed` index ONE final time by applying "becoming" retroactively:
/// each consumed region's exclusive scaffolding is deleted on load (shared
/// edges with surviving regions survive, exactly as at extrusion), an
/// emptied sketch ceases to exist, and the index itself is then discarded
/// — the gate stays derived from the standing solids. `footprints` and
/// `source` are ignored outright. Without this, every previously extruded
/// outline would resurrect as live, drawable geometry at load — the exact
/// zombie the model exists to kill.
#[test]
fn older_files_consumed_claims_become_deletion_on_load() {
    // Build the OLD file shape: sketches still carrying their consumed
    // outlines. Sketch 0 (dense 0): two rects sharing the x=2 wall — the
    // LEFT one will be marked consumed, so its 3 exclusive edges must go
    // while the shared wall survives with the live right region. Sketch 1
    // (dense 1): one lone rect, marked consumed — the emptied sketch must
    // cease to exist. A standing box (from a consumed-at-runtime sketch)
    // provides the solid the claims pointed at.
    let mut doc = Document::new();
    let s0 = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s0).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0)),
            (Point3::new(2.0, 0.0, 0.0), Point3::new(2.0, 2.0, 0.0)),
            (Point3::new(2.0, 2.0, 0.0), Point3::new(0.0, 2.0, 0.0)),
            (Point3::new(0.0, 2.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
            (Point3::new(2.0, 0.0, 0.0), Point3::new(4.0, 0.0, 0.0)),
            (Point3::new(4.0, 0.0, 0.0), Point3::new(4.0, 2.0, 0.0)),
            (Point3::new(4.0, 2.0, 0.0), Point3::new(2.0, 2.0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("segment");
        }
    }
    let s1 = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s1).unwrap();
        for (a, b) in [
            (Point3::new(6.0, 0.0, 0.0), Point3::new(7.0, 0.0, 0.0)),
            (Point3::new(7.0, 0.0, 0.0), Point3::new(7.0, 1.0, 0.0)),
            (Point3::new(7.0, 1.0, 0.0), Point3::new(6.0, 1.0, 0.0)),
            (Point3::new(6.0, 1.0, 0.0), Point3::new(6.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("segment");
        }
    }
    extrude_box(&mut doc, 10.0, 10.0, 12.0, 12.0, 0.0, 1.0);
    let bytes = doc.save();

    // The dense region index of s0's LEFT rect (outer vertices at x <= 2).
    let left_dense = {
        let sk = doc.sketch(s0).unwrap();
        sk.regions()
            .values()
            .enumerate()
            .find(|(_, r)| {
                r.outer
                    .iter()
                    .all(|&v| sk.vertices()[v].position.x <= 2.0 + 1e-9)
            })
            .map(|(i, _)| i)
            .expect("left region")
    };

    let patched_bytes = patch_manifest(&bytes, |manifest| {
        manifest["format_version"] = serde_json::json!(10);
        manifest["consumed"] = serde_json::json!([[0, left_dense], [1, 0]]);
        // Retired per-object fields ride along and must be ignored.
        let objs = manifest["objects"].as_array_mut().unwrap();
        let m = objs[0].as_object_mut().unwrap();
        m.insert("source".to_string(), serde_json::json!([0, 0]));
        m.insert(
            "footprints".to_string(),
            serde_json::json!([{
                "sketch": 0,
                "outer": [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]],
            }]),
        );
    });

    let loaded = Document::load(&patched_bytes).expect("old manifest loads");
    assert_eq!(loaded.visible_object_ids().len(), 1, "the solid loads");

    // Becoming, retroactively: the left rect's exclusive scaffolding is
    // gone, the shared wall survives with the live right region, and the
    // fully consumed second sketch ceased to exist.
    let sketches = loaded.sketch_ids();
    assert_eq!(sketches.len(), 1, "the emptied sketch is gone");
    let sk = loaded.sketch(sketches[0]).expect("live");
    assert_eq!(
        sk.edges().len(),
        4,
        "left rect deleted; shared wall + right rect's own edges survive"
    );
    assert_eq!(sk.regions().len(), 1, "the right region still closes");
    assert_eq!(
        loaded.extrudable_regions(sketches[0]).unwrap().len(),
        1,
        "the surviving region is extrudable (no frozen claim)"
    );

    // Nothing resurrected: no region overlapping the old consumed areas
    // exists, and no stored claim survived — resaving emits clean v11.
    let resaved = loaded.save();
    let resaved_manifest = {
        use std::io::Read as _;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&resaved[..])).unwrap();
        let mut s = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
    assert!(resaved_manifest.contains("\"format_version\": 11"));
    assert!(!resaved_manifest.contains("\"consumed\""));
    assert!(!resaved_manifest.contains("\"footprints\""));
    assert!(!resaved_manifest.contains("\"source\""));

    // And the resave is byte-stable: load(save) reproduces the bytes.
    let reloaded = Document::load(&resaved).expect("clean v11 reloads");
    assert_eq!(reloaded.save(), resaved, "deterministic clean-v11 resave");
}

/// The one-time retroactive consumption is VERSION-gated, not
/// presence-gated: `consumed` is meaningful only in files declaring a
/// format older than 11. A file that declares v11+ AND carries a
/// `consumed` field is malformed for its own version — hand-edited or
/// produced by a broken writer — and is rejected typed, never silently
/// "repaired" by deleting sketch geometry no standing solid claims.
#[test]
fn consumed_field_smuggled_into_a_v11_file_is_rejected() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("segment");
        }
    }
    let bytes = doc.save();

    // Add a resolvable consumed pair but LEAVE format_version at 11.
    let smuggled = patch_manifest(&bytes, |m| {
        m["consumed"] = serde_json::json!([[0, 0]]);
    });
    assert!(
        matches!(
            Document::load(&smuggled),
            Err(kernel::LoadError::MalformedManifest { .. })
        ),
        "a v11 file carrying consumed data is malformed, not repaired"
    );
}

/// A pre-v11 `consumed` pair that does not resolve — a dense sketch or
/// region index out of range — is a dangling reference and fails typed,
/// exactly like every other dangling id (never silently repaired, never a
/// partial load).
#[test]
fn older_files_dangling_consumed_pairs_are_rejected() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("segment");
        }
    }
    let bytes = doc.save();

    let bad_region = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(10);
        m["consumed"] = serde_json::json!([[0, 5]]);
    });
    assert!(
        matches!(
            Document::load(&bad_region),
            Err(kernel::LoadError::DanglingReference { .. })
        ),
        "out-of-range region index fails typed"
    );

    let bad_sketch = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(10);
        m["consumed"] = serde_json::json!([[7, 0]]);
    });
    assert!(
        matches!(
            Document::load(&bad_sketch),
            Err(kernel::LoadError::DanglingReference { .. })
        ),
        "out-of-range sketch index fails typed"
    );
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

// ------------------------------------------------------------ state_hash
//
// `Document::state_hash` is the canonical deterministic oracle (docs/DEVELOPMENT.md):
// a digest of the canonical `save` bytes. These pin the contract everything
// downstream (replay, log stamps, the guard) relies on.

#[test]
fn state_hash_is_stable_within_a_process() {
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    extrude_box(&mut doc, 3.0, 3.0, 4.0, 4.0, 0.0, 2.0);
    assert_eq!(
        doc.state_hash(),
        doc.state_hash(),
        "state_hash must be deterministic for an unchanged document"
    );
}

#[test]
fn state_hash_distinguishes_distinct_states() {
    let empty = Document::new().state_hash();

    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let one = doc.state_hash();
    assert_ne!(empty, one, "adding a box must change the state_hash");

    extrude_box(&mut doc, 3.0, 3.0, 4.0, 4.0, 0.0, 2.0);
    let two = doc.state_hash();
    assert_ne!(one, two, "adding a second box must change the state_hash");
}

#[test]
fn state_hash_survives_save_load_round_trip() {
    // The oracle is replay-stable: deserializing canonical bytes and re-hashing
    // reproduces the original hash (dense-id remap is itself deterministic), so a
    // golden hash frozen against a recorded session stays valid after reload.
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    extrude_box(&mut doc, 5.0, 5.0, 7.0, 6.0, 0.0, 3.0);
    let before = doc.state_hash();

    let loaded = Document::load(&doc.save()).expect("load");
    assert_eq!(
        before,
        loaded.state_hash(),
        "state_hash is preserved across a save/load round-trip"
    );
}

#[test]
fn state_hash_is_undo_redo_identity() {
    // `state_hash` tracks live, visible state, and undo/redo is an identity on
    // that state (document_specs::undo_redo_is_identity_on_visible_state), so a
    // round trip through the undo log must return to the exact same hash — the
    // property that lets a recorded session replay to a golden hash.
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    extrude_box(&mut doc, 5.0, 5.0, 7.0, 6.0, 0.0, 3.0);
    let full = doc.state_hash();

    doc.undo().expect("undo the second box");
    assert_ne!(full, doc.state_hash(), "undo changes the visible state");

    doc.redo().expect("redo the second box");
    assert_eq!(
        full,
        doc.state_hash(),
        "undo+redo restores the exact state_hash"
    );
}

/// Analytic curve geometry (manifest v10) round-trips exactly: a chain
/// opened with `begin_curve_with` comes back carrying its circle, an
/// identity-only chain comes back with none, and the re-save is
/// byte-stable.
#[test]
fn curve_geometry_round_trips_through_save_load() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        sk.begin_curve_with(kernel::CurveGeom {
            center: Point3::new(1.0, 2.0, 0.0),
            radius: 0.75,
        })
        .unwrap();
        sk.add_segment(Point3::new(1.75, 2.0, 0.0), Point3::new(1.0, 2.75, 0.0))
            .unwrap();
        sk.add_segment(Point3::new(1.0, 2.75, 0.0), Point3::new(0.25, 2.0, 0.0))
            .unwrap();
        sk.end_curve();
        // Identity-only chain alongside (pre-capture behavior).
        sk.begin_curve();
        sk.add_segment(Point3::new(5.0, 0.0, 0.0), Point3::new(6.0, 0.5, 0.0))
            .unwrap();
        sk.end_curve();
    }

    let bytes = doc.save();
    let doc2 = Document::load(&bytes).expect("round-trip");
    let s2 = doc2.sketch_ids()[0];
    let sk2 = doc2.sketch(s2).expect("live");

    let mut with_geom = 0;
    let mut without_geom = 0;
    let mut seen: std::collections::BTreeSet<kernel::SketchCurveId> =
        std::collections::BTreeSet::new();
    for e in sk2.edges().values() {
        let Some(cid) = e.curve else { continue };
        if !seen.insert(cid) {
            continue;
        }
        match sk2.curve_geom(cid) {
            Some(g) => {
                assert!(g.center.approx_eq(Point3::new(1.0, 2.0, 0.0), 1e-12));
                assert_eq!(g.radius, 0.75, "radius is exact through the manifest");
                with_geom += 1;
            }
            None => without_geom += 1,
        }
    }
    assert_eq!((with_geom, without_geom), (1, 1));

    assert_eq!(doc2.save(), bytes, "deterministic re-save");
}

/// A saved two-chain document for the curve tamper tests below: chain 0 has
/// analytic geometry, chain 1 is identity-only. One edge each.
fn saved_two_chain_doc() -> Vec<u8> {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        sk.begin_curve_with(kernel::CurveGeom {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 1.0,
        })
        .unwrap();
        sk.add_segment(Point3::new(1.0, 0.0, 0.0), Point3::new(0.0, 1.0, 0.0))
            .unwrap();
        sk.end_curve();
        sk.begin_curve();
        sk.add_segment(Point3::new(5.0, 0.0, 0.0), Point3::new(6.0, 0.0, 0.0))
            .unwrap();
        sk.end_curve();
    }
    doc.save()
}

/// Re-packs `bytes` with its manifest JSON transformed by `patch`.
fn with_patched_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    patch(&mut manifest);
    let patched_manifest = serde_json::to_vec_pretty(&manifest).unwrap();

    let out_cursor = Cursor::new(Vec::<u8>::new());
    let mut new_zip = zip::ZipWriter::new(out_cursor);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched_manifest).unwrap();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
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
    new_zip.finish().unwrap().into_inner()
}

/// A v10 manifest whose curve entry carries a non-positive radius is a
/// malformed file: rejected with a typed error, never defaulted or dropped.
#[test]
fn degenerate_curve_radius_in_manifest_is_rejected() {
    let bytes = saved_two_chain_doc();
    let patched = with_patched_manifest(&bytes, |m| {
        m["sketches"][0]["curves"][0]["radius"] = serde_json::json!(0.0);
    });
    assert!(
        Document::load(&patched).is_err(),
        "degenerate curve radius must be a typed load failure"
    );
}

/// A `curves[]` entry whose id no edge references — PAST the referenced
/// range — is a dangling definition: typed load failure, never silently
/// accepted.
#[test]
fn curves_entry_past_referenced_range_is_rejected() {
    let bytes = saved_two_chain_doc();
    let patched = with_patched_manifest(&bytes, |m| {
        m["sketches"][0]["curves"][0]["id"] = serde_json::json!(5);
    });
    assert!(
        Document::load(&patched).is_err(),
        "a curves[] entry no edge references must be a typed load failure"
    );
}

/// The gap case the naive `id < max_referenced` check misses: retag the
/// edges so their curve indices skip an id (0 and 2, no 1). Non-dense first
/// appearance is malformed per the spec — gap-filling a phantom chain would
/// also let a `curves[]` entry at the gap index load silently as a dangling
/// definition.
#[test]
fn non_dense_curve_indices_are_rejected() {
    let bytes = saved_two_chain_doc();
    let patched = with_patched_manifest(&bytes, |m| {
        // Second chain's edge: curve 1 -> 2, leaving index 1 a gap.
        let edges = m["sketches"][0]["edges"].as_array_mut().unwrap();
        for e in edges.iter_mut() {
            if e.get("curve").and_then(|c| c.as_u64()) == Some(1) {
                e["curve"] = serde_json::json!(2);
            }
        }
    });
    assert!(
        Document::load(&patched).is_err(),
        "a curve index skipping ahead of first-appearance density must be          a typed load failure"
    );
}

/// And a `curves[]` definition sitting IN such a gap must never load: the
/// combination the gap-filling loader accepted silently. With density
/// enforced the file already fails on the edge index; this pins the whole
/// tampered shape (gap indices + gap-addressed definition) as a load error.
#[test]
fn curves_entry_in_a_gap_is_rejected() {
    let bytes = saved_two_chain_doc();
    let patched = with_patched_manifest(&bytes, |m| {
        let edges = m["sketches"][0]["edges"].as_array_mut().unwrap();
        for e in edges.iter_mut() {
            if e.get("curve").and_then(|c| c.as_u64()) == Some(1) {
                e["curve"] = serde_json::json!(2);
            }
        }
        // Point the analytic definition at the gap index.
        m["sketches"][0]["curves"][0]["id"] = serde_json::json!(1);
    });
    assert!(
        Document::load(&patched).is_err(),
        "a curves[] definition in an index gap must be a typed load failure"
    );
}
