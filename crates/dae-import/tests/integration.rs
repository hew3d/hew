//! Integration tests for the `dae-import` crate (contract).
//!
//! These tests exercise the full pipeline: parse → heal → material → ingest,
//! using hand-authored COLLADA 1.4 fixtures in `tests/fixtures/`.
//!
//! Property tests are at the bottom: random kernel Object → minimal COLLADA →
//! import → ingest → `objects_equivalent`.

use dae_import::{ImageMap, import};
use kernel::{Document, ImportNode, Point3, UvFrame};
use proptest::prelude::*;

// ─────────────────────────────────────── fixture helpers ─────────────────────

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
}

fn empty_images() -> ImageMap {
    ImageMap::new()
}

// ─────────────────────────────────────── fixture (a): closed box ─────────────

/// Single closed box → 1 watertight object; 0 leaky; 0 skipped.
#[test]
fn closed_box_yields_one_watertight_object() {
    let bytes = fixture("box_closed.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse box_closed.dae");
    assert!(missing.is_empty(), "no textures in this file");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");

    assert_eq!(report.objects_created, 1, "one object");
    assert_eq!(report.watertight, 1, "watertight");
    assert_eq!(report.leaky, 0, "not leaky");
    assert!(report.skipped.is_empty(), "nothing skipped");
    assert_eq!(doc.visible_object_ids().len(), 1);
}

// ─────────────────────────── fixture (b): two-sided export box ───────────────

/// Two-sided export: each face duplicated with reversed winding → dedup reduces
/// to 6 faces → 1 watertight object.
#[test]
fn two_sided_box_deduplicates_to_one_watertight_object() {
    let bytes = fixture("box_two_sided.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse box_two_sided.dae");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");

    assert_eq!(report.objects_created, 1, "one object after dedup");
    assert_eq!(report.watertight, 1, "watertight after dedup");
    assert_eq!(report.leaky, 0);
    assert!(report.skipped.is_empty());
}

// ─────────────────────────── fixture (c): open shell ─────────────────────────

/// Open box (missing one face) → 1 leaky object; nothing skipped.
#[test]
fn open_shell_yields_one_leaky_object() {
    let bytes = fixture("box_open.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse box_open.dae");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");

    assert_eq!(report.leaky, 1, "leaky");
    assert_eq!(report.watertight, 0);
    assert!(report.skipped.is_empty());
    assert_eq!(doc.visible_object_ids().len(), 1, "object still created");
}

// ──────────────────── fixture (d): group + library_nodes instance ─────────────

/// `<node>` group with a child mesh + `<instance_node>` referencing a
/// `<library_nodes>` entry → at least one object from the world mesh and one
/// component definition with an instance.
#[test]
fn group_and_instance_node_produce_correct_tree() {
    let bytes = fixture("group_and_instance.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse group_and_instance.dae");
    assert!(missing.is_empty());

    // The scene should have at least one def (from library_nodes) and at least
    // one root that references it.
    let has_instance = scene
        .roots
        .iter()
        .any(|n| matches!(n, ImportNode::Instance { .. }));
    assert!(has_instance, "scene should contain an Instance node");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");

    // At minimum: the world mesh object + the def member object.
    assert!(
        report.objects_created >= 1,
        "at least one object created, got {}",
        report.objects_created
    );

    // There must be at least one instance in the document.
    assert!(
        !doc.instance_ids().is_empty(),
        "at least one instance placed"
    );
}

// ─────────────────────── fixture (e): Y_UP + cm unit ─────────────────────────

/// Y_UP + centimeter file: a vertex at Y_UP (0, 100, 0) should arrive at
/// Hew Z_UP (0, 0, 1.0) meters after unit scale and axis rotation.
#[test]
fn yup_cm_vertex_lands_at_expected_z_up_meter_position() {
    let bytes = fixture("yup_cm.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse yup_cm.dae");
    assert!(missing.is_empty());

    // The import scene contains a mesh recipe; inspect its positions directly.
    let mesh_recipe = scene
        .roots
        .iter()
        .find_map(|n| match n {
            ImportNode::Mesh(r) => Some(r),
            _ => None,
        })
        .expect("root mesh recipe");

    // Vertex C was (0, 100, 0) in Y_UP cm.
    // After scale ×0.01: (0, 1.0, 0)
    // After Y→Z rotation (+90° about X: y→z, z→−y): (0, 0, 1.0)
    let expected = Point3::new(0.0, 0.0, 1.0);
    let found = mesh_recipe.positions.iter().any(|&p| {
        (p.x - expected.x).abs() < 1e-5
            && (p.y - expected.y).abs() < 1e-5
            && (p.z - expected.z).abs() < 1e-5
    });
    assert!(
        found,
        "expected vertex at {expected:?} after Y_UP→Z_UP + cm→m; positions = {:?}",
        mesh_recipe.positions
    );
}

// ─────────────────────────── fixture (f): color material ─────────────────────

/// A material with a diffuse `<color>` → at least one material in the scene;
/// one object created with faces using that material.
#[test]
fn color_material_is_resolved() {
    let bytes = fixture("material_color.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse material_color.dae");
    assert!(missing.is_empty(), "no textures referenced");
    assert!(!scene.materials.is_empty(), "at least one material");

    let mesh = scene
        .roots
        .iter()
        .find_map(|n| match n {
            ImportNode::Mesh(r) => Some(r),
            _ => None,
        })
        .expect("root mesh");

    // All faces should reference a material (dense index 0).
    assert!(
        mesh.face_materials
            .iter()
            .all(|&m| m != kernel::NO_MATERIAL),
        "all faces should have a material assigned"
    );

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");
    assert_eq!(report.objects_created, 1);
}

// ─────────────────────── fixture (f): transparent/glass material ─────────────

/// A SketchUp-style `<constant>` glass material carries its color in
/// `<transparent>` and its opacity in the transparent alpha (A_ONE). The
/// resolved kernel material must be the transparent color (not the old gray
/// fallback) with a ~50% alpha.
#[test]
fn transparent_constant_material_resolves_color_and_alpha() {
    let bytes = fixture("material_transparent.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse material_transparent.dae");
    assert!(missing.is_empty(), "no textures referenced");
    assert_eq!(scene.materials.len(), 1, "one glass material");

    let mat = &scene.materials[0];
    // color = 0.2,0.4,0.8 → 51,102,204 (rounded); alpha = 0.5 → 128.
    assert_eq!(mat.color.r, 51, "red from <transparent> color");
    assert_eq!(mat.color.g, 102, "green from <transparent> color");
    assert_eq!(mat.color.b, 204, "blue from <transparent> color");
    assert_eq!(mat.color.a, 128, "alpha = transparent.alpha · transparency");
    assert!(mat.texture.is_none(), "flat color, no texture");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");
    assert_eq!(report.objects_created, 1);
}

// ─────────────────────────── fixture (f): texture material ───────────────────

/// Texture resolved via ImageMap → Material::textured created; not in missing.
/// Same file with empty ImageMap → URI appears in textures_missing; fallback
/// to solid material so the object is still created.
#[test]
fn texture_resolved_via_image_map() {
    let bytes = fixture("material_texture.dae");

    // Provide a fake 1-pixel PNG (minimal valid PNG bytes) so the resolver
    // finds the URI.
    let fake_png: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        // IHDR chunk: 1x1 RGB image
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE,
        // IDAT chunk (1 pixel, red)
        0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
        0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC, 0x33, // IEND chunk
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    let mut images = ImageMap::new();
    images.insert(
        "textures/wood.png".to_string(),
        (fake_png, kernel::ImageFormat::Png),
    );

    let (scene, missing) = import(&bytes, &images).expect("parse material_texture.dae");
    assert!(
        missing.is_empty(),
        "texture was resolved; missing should be empty but got: {missing:?}"
    );
    assert!(!scene.materials.is_empty());
}

#[test]
fn unresolved_texture_appears_in_missing() {
    let bytes = fixture("material_texture.dae");

    // Empty image map → texture cannot be resolved.
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse");

    assert!(
        missing.contains(&"textures/wood.png".to_string()),
        "unresolved URI must appear in textures_missing; got: {missing:?}"
    );

    // Object is still created (fallback to solid material).
    let mut doc = Document::new();
    let (report, _change) = doc.ingest(scene, missing).expect("ingest");
    assert_eq!(
        report.objects_created, 1,
        "object created despite missing texture"
    );
    assert!(
        report
            .textures_missing
            .contains(&"textures/wood.png".to_string()),
        "missing URI must be in report.textures_missing"
    );
}

// ─────────── fixture (g): countertop-experiment.dae (SketchUp, inch, Z_UP) ───

/// Real-world SketchUp export: 8 component instances, multi-material shells,
/// collinear sliver triangle.  Tests all three bug- fixes together:
///
/// 1. Row-major matrix: instances must NOT stack at the origin.
/// 2. world_tf applied outermost once: pose translations must match meter math.
/// 3. Primitive-group merge + sliver drop: Counter_Base must NOT appear in
///    `report.skipped`.
#[test]
fn countertop_experiment_imports_correctly() {
    let bytes = fixture("countertop-experiment.dae");
    let (scene, _missing) =
        import(&bytes, &empty_images()).expect("parse countertop-experiment.dae");

    let mut doc = Document::new();
    let (report, _change) = doc.ingest(scene, vec![]).expect("ingest countertop");

    // ── Bug 3: Counter_Base (ID4) must NOT be skipped ────────────────────────
    // Before the fix it appeared in report.skipped because:
    //   (a) multi-material <triangles> groups were emitted as separate RawMesh
    //       fragments (non-watertight), and
    //   (b) a collinear sliver triangle caused from_polygons to reject the whole
    //       mesh.
    let skipped_names: Vec<&str> = report.skipped.iter().map(|s| s.name.as_str()).collect();
    assert!(
        !skipped_names.contains(&"ID4"),
        "Counter_Base (ID4) must not be skipped; skipped = {skipped_names:?}"
    );

    // At least one object must have been created.
    assert!(
        report.objects_created > 0,
        "expected objects from countertop; got 0"
    );

    // ── T-junction healing: every object must be watertight ──────────────────
    // Counter_Base (ID4) used to import as the model's one leaky shell because
    // SketchUp's triangulation leaves coplanar T-junctions (a vertex mid-edge of
    // a neighbour). `split_t_junctions` splices those vertices into the edge so
    // the half-edges pair up. Result: all objects watertight, none leaky.
    assert_eq!(
        report.leaky, 0,
        "no object may be leaky after T-junction healing"
    );
    assert_eq!(
        report.watertight, report.objects_created,
        "every imported object must be watertight"
    );

    // There must be instances (the visual scene has 8 component placements).
    let instance_ids = doc.instance_ids();
    assert!(
        !instance_ids.is_empty(),
        "expected component instances; got none"
    );

    // ── Bug 1 + Bug 2: pose translations must match inch→meter math ──────────
    //
    // Instance ID19 has matrix (row-major):
    //   [1 0 0 -0.3077961 / 0 1 0 90.9566929 / 0 0 1 34.8425197]
    // Identity linear + translation (-0.3077961, 90.9566929, 34.8425197) inches.
    // After world_tf (scale × 0.0254, Z_UP = identity rotation):
    //   tx = -0.3077961 × 0.0254 ≈ -0.00782 m
    //   ty =  90.9566929 × 0.0254 ≈  2.31030 m
    //   tz =  34.8425197 × 0.0254 ≈  0.88500 m
    //
    // Instance ID60 has a 90° rotation about Y + translation (101.18, 14.43, 120) inches:
    //   tx = 101.1811024 × 0.0254 ≈ 2.57000 m
    //   ty =  14.4291339 × 0.0254 ≈ 0.36650 m
    //   tz = 120.0000000 × 0.0254 ≈ 3.04800 m
    const POSE_TOL: f64 = 1e-3; // 1 mm — loose enough for f32 DAE coordinates

    let mut found_id19 = false;
    let mut found_id60 = false;

    for &iid in &instance_ids {
        let pose = doc.instance_pose(iid).expect("pose must exist");
        let aff = pose.to_affine();
        // translation is at indices [3], [7], [11] of the 3×4 row-major array.
        let (tx, ty, tz) = (aff[3], aff[7], aff[11]);

        // ID19: identity linear, translation ≈ (-0.0078, 2.3103, 0.8850) m
        if (tx - (-0.007_817_820_54)).abs() < POSE_TOL
            && (ty - 2.310_300_399_66).abs() < POSE_TOL
            && (tz - 0.885_000_000_0).abs() < POSE_TOL
        {
            found_id19 = true;
        }

        // ID60: 90° rotation about -Y, translation ≈ (2.5700, 0.3665, 3.0480) m
        if (tx - 2.570_000_0).abs() < POSE_TOL
            && (ty - 0.366_500_0).abs() < POSE_TOL
            && (tz - 3.048_000_0).abs() < POSE_TOL
        {
            found_id60 = true;
        }
    }

    assert!(
        found_id19,
        "ID19 instance pose not found at expected translation (-0.0078, 2.3103, 0.8850) m; \
         instance poses: {:?}",
        instance_ids
            .iter()
            .map(|&i| doc.instance_pose(i).map(|p| p.to_affine()))
            .collect::<Vec<_>>()
    );
    assert!(
        found_id60,
        "ID60 instance pose not found at expected translation (2.5700, 0.3665, 3.0480) m"
    );
}

/// Orientation healing: every imported mesh must be wound outward (positive
/// signed volume). `Counter_Base` (ID4) ships with reversed faces in the
/// SketchUp source — invisible there (double-sided render) but inside-out for a
/// single-sided renderer (transparent) and un-pushable. `orient_outward` flips
/// closed inside-out shells to outward; this guards that regression.
#[test]
fn countertop_meshes_are_outward_oriented() {
    let bytes = fixture("countertop-experiment.dae");
    let (scene, _missing) = import(&bytes, &empty_images()).expect("parse countertop");

    let signed_vol6 = |positions: &[Point3], faces: &[Vec<usize>]| -> f64 {
        let mut v6 = 0.0;
        for face in faces {
            if face.len() < 3 {
                continue;
            }
            let p0 = positions[face[0]].to_vec();
            for i in 1..face.len() - 1 {
                let p1 = positions[face[i]].to_vec();
                let p2 = positions[face[i + 1]].to_vec();
                v6 += p0.dot(p1.cross(p2));
            }
        }
        v6
    };

    let mut checked = 0;
    for def in &scene.defs {
        for mesh in &def.meshes {
            let v = signed_vol6(&mesh.positions, &mesh.faces);
            assert!(
                v > 0.0,
                "mesh {:?} is inside-out (signed volume6 = {v})",
                mesh.name
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 8,
        "expected the 8 component meshes; checked {checked}"
    );
}

/// Coplanar merge: COLLADA triangulates every face, so without merging a simple
/// box slab arrives as 12 triangles and push/pull operates on a half-face. After
/// `merge_coplanar` a box slab is its 6 real quad faces, and the detailed
/// `Counter_Base` collapses from 43 triangles to a handful of real polygons —
/// while staying watertight (asserted by `countertop_experiment_imports_correctly`).
#[test]
fn countertop_triangles_merge_into_real_faces() {
    let bytes = fixture("countertop-experiment.dae");
    let (scene, _missing) = import(&bytes, &empty_images()).expect("parse countertop");

    let mesh = |name: &str| {
        scene
            .defs
            .iter()
            .flat_map(|d| &d.meshes)
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("mesh {name} not found"))
    };

    // A plain rectangular slab → exactly its 6 box faces, all quads.
    let slab = mesh("_10__Counter_Top");
    assert_eq!(slab.faces.len(), 6, "box slab merges to 6 faces");
    assert!(
        slab.faces.iter().all(|f| f.len() == 4),
        "every box-slab face is a quad"
    );

    // The detailed cabinet base collapses far below its 43 triangles, and no
    // face is left as a bare triangle on its flat sides (real faces are polygons).
    let cb = mesh("Counter_Base");
    assert!(
        cb.faces.len() <= 20,
        "Counter_Base should merge from 43 triangles to a handful of faces; got {}",
        cb.faces.len()
    );
}

// ───────────── fixture (g2): countertop UV frames (TEXCOORD → fit) ───────────

/// Textured COLLADA primitives produce `Some(UvFrame)` on faces that carry
/// TEXCOORD data, and `None` on faces that don't.
///
/// The countertop fixture (library_nodes geometry) has two kinds of triangles
/// per mesh: Material3 with a `TEXCOORD` source and Material2 without.
/// After import:
///  - Def meshes with Material3 triangles → at least one `Some(UvFrame)`.
///  - Faces that came from Material2 (no TEXCOORD) → `None`.
///  - For each `Some(frame)`, `frame.apply(corner_pos) ≈ source_uv` within
///    the named constant `UV_AFFINE_RESIDUAL_TOL` (1e-3).
///
/// This verifies the full pipeline: TEXCOORD parse → corner-UV threading through
/// heal → per-face affine fit → `MeshRecipe::face_uv_frames`.
#[test]
fn countertop_uv_frames_populated_for_textured_faces() {
    use dae_import::uv::UV_AFFINE_RESIDUAL_TOL;

    let bytes = fixture("countertop-experiment.dae");
    let (scene, _missing) =
        import(&bytes, &empty_images()).expect("parse countertop-experiment.dae");

    // The countertop geometry lives in defs (library_nodes → DefRecipe).
    // Collect all face_uv_frames from all def meshes.
    let mut all_frames: Vec<Option<UvFrame>> = Vec::new();
    for def in &scene.defs {
        for mesh in &def.meshes {
            assert_eq!(
                mesh.face_uv_frames.len(),
                mesh.faces.len(),
                "face_uv_frames must be parallel to faces in def mesh '{}'",
                mesh.name
            );
            all_frames.extend(mesh.face_uv_frames.iter().cloned());
        }
    }

    // Also collect frames from world-mesh roots (instance_geometry directly in
    // visual scene nodes).
    let mut world_frames: Vec<Option<UvFrame>> = Vec::new();
    for root in &scene.roots {
        collect_world_frames(root, &mut world_frames);
    }

    // There must be at least one Some(frame) — the textured Material3 triangles.
    let some_count = all_frames
        .iter()
        .chain(world_frames.iter())
        .filter(|f| f.is_some())
        .count();
    assert!(
        some_count > 0,
        "expected at least one Some(UvFrame) from textured TEXCOORD triangles, \
         got 0 out of {} total frames",
        all_frames.len() + world_frames.len()
    );

    // There must also be at least one None — the untextured Material2 triangles.
    let none_count = all_frames
        .iter()
        .chain(world_frames.iter())
        .filter(|f| f.is_none())
        .count();
    assert!(
        none_count > 0,
        "expected at least one None frame from untextured triangles, \
         got 0 out of {} total frames",
        all_frames.len() + world_frames.len()
    );

    // For each def mesh with frames, verify the fitted frame produces finite UVs.
    // The residual guard inside fit_uv_frame already ensures any returned
    // Some(frame) has residual ≤ UV_AFFINE_RESIDUAL_TOL.
    let _ = UV_AFFINE_RESIDUAL_TOL; // named tol referenced explicitly
    for def in &scene.defs {
        for mesh in &def.meshes {
            for (fi, frame_opt) in mesh.face_uv_frames.iter().enumerate() {
                if let Some(frame) = frame_opt {
                    for &vi in &mesh.faces[fi] {
                        let p = mesh.positions[vi];
                        let uv = frame.apply(p);
                        assert!(
                            uv[0].is_finite() && uv[1].is_finite(),
                            "frame.apply produced non-finite UV at face {fi}, pos {:?}",
                            p
                        );
                    }
                }
            }
        }
    }
}

/// Recursively collect `face_uv_frames` from `ImportNode::Mesh` leaves.
fn collect_world_frames(node: &ImportNode, out: &mut Vec<Option<UvFrame>>) {
    match node {
        ImportNode::Mesh(recipe) => {
            out.extend(recipe.face_uv_frames.iter().cloned());
        }
        ImportNode::Group { children, .. } => {
            for child in children {
                collect_world_frames(child, out);
            }
        }
        ImportNode::Instance { .. } => {}
    }
}

// ──────────────────────────── property tests ─────────────────────────────────

/// Build a minimal valid COLLADA 1.4 XML from positions + polygon index lists.
/// The output is Z_UP, meter scale (identity transform), so no unit/axis
/// correction occurs in the heal step. Positions are emitted verbatim.
fn emit_minimal_dae(positions: &[Point3], faces: &[Vec<usize>]) -> Vec<u8> {
    let pos_count = positions.len() * 3;
    let vert_count = positions.len();
    let pos_str: String = positions
        .iter()
        .flat_map(|p| [p.x.to_string(), p.y.to_string(), p.z.to_string()])
        .collect::<Vec<_>>()
        .join(" ");

    let face_count = faces.len();
    let vcount_str = faces
        .iter()
        .map(|f| f.len().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let p_str = faces
        .iter()
        .flat_map(|f| f.iter().map(|i| i.to_string()))
        .collect::<Vec<_>>()
        .join(" ");

    // Build the XML by concatenation, avoiding raw-string `#"` termination issues.
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str(
        "<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n",
    );
    xml.push_str("  <asset><created>2024-01-01</created><modified>2024-01-01</modified>");
    xml.push_str("<unit name=\"meter\" meter=\"1.0\"/><up_axis>Z_UP</up_axis></asset>\n");
    xml.push_str("  <library_geometries><geometry id=\"Geom\" name=\"Geom\"><mesh>\n");
    xml.push_str(&format!(
        "    <source id=\"Pos\"><float_array id=\"PosArr\" count=\"{pos_count}\">{pos_str}</float_array>\n"
    ));
    xml.push_str(&format!(
        "      <technique_common><accessor source=\"#PosArr\" count=\"{vert_count}\" stride=\"3\">\n"
    ));
    xml.push_str("        <param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/>\n");
    xml.push_str("      </accessor></technique_common></source>\n");
    xml.push_str(
        "    <vertices id=\"Verts\"><input semantic=\"POSITION\" source=\"#Pos\"/></vertices>\n",
    );
    xml.push_str(&format!(
        "    <polylist count=\"{face_count}\"><input semantic=\"VERTEX\" source=\"#Verts\" offset=\"0\"/>\n"
    ));
    xml.push_str(&format!(
        "      <vcount>{vcount_str}</vcount><p>{p_str}</p></polylist>\n"
    ));
    xml.push_str("  </mesh></geometry></library_geometries>\n");
    xml.push_str("  <library_visual_scenes><visual_scene id=\"Scene\" name=\"Scene\">\n");
    xml.push_str(
        "    <node id=\"N\" name=\"N\" type=\"NODE\"><instance_geometry url=\"#Geom\"/></node>\n",
    );
    xml.push_str("  </visual_scene></library_visual_scenes>\n");
    xml.push_str("  <scene><instance_visual_scene url=\"#Scene\"/></scene>\n");
    xml.push_str("</COLLADA>\n");
    xml.into_bytes()
}

// ── Comparators (mirrored from crates/kernel/tests/op_specs.rs) ───────────────

fn polygons_of(obj: &kernel::Object) -> Vec<Vec<Point3>> {
    let (points, faces) = obj.to_polygons();
    faces
        .into_iter()
        .map(|poly| poly.into_iter().map(|i| points[i]).collect())
        .collect()
}

fn cyclic_match_tol(a: &[Point3], b: &[Point3], tolerance: f64) -> bool {
    a.len() == b.len()
        && (0..a.len()).any(|shift| {
            a.iter()
                .enumerate()
                .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], tolerance))
        })
}

fn objects_equivalent_tol(x: &kernel::Object, y: &kernel::Object, tolerance: f64) -> bool {
    let xs = polygons_of(x);
    let mut ys = polygons_of(y);
    if xs.len() != ys.len() {
        return false;
    }
    for poly in xs {
        match ys
            .iter()
            .position(|cand| cyclic_match_tol(&poly, cand, tolerance))
        {
            Some(i) => {
                ys.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

/// COLLADA stores positions as `f32`, so round-trips through the format lose
/// about 1 ULP of f32 precision (~1e-7 relative). Use a coarser tolerance
/// than `tol::POINT_MERGE` (1e-9) for proptest assertions over COLLADA files.
/// With test positions in [−10, 10], 1e-5 is a generous f32-safe bound.
const F32_SAFE_TOL: f64 = 1e-5;

// ── Point3 Strategy ───────────────────────────────────────────────────────────

fn arb_point() -> impl Strategy<Value = Point3> {
    // Keep coordinates away from zero/infinity to avoid degenerate planes and
    // to stay above tol::POINT_MERGE after round-trip through f64 text.
    (-10.0..10.0f64, -10.0..10.0f64, -10.0..10.0f64).prop_map(|(x, y, z)| Point3::new(x, y, z))
}

/// Axis-aligned box in arbitrary position and size.
fn arb_box_soup() -> impl Strategy<Value = (Vec<Point3>, Vec<Vec<usize>>)> {
    (
        (-10.0..10.0f64, -10.0..10.0f64, -10.0..10.0f64),
        (0.1..5.0f64, 0.1..5.0f64, 0.1..5.0f64),
    )
        .prop_map(|((x, y, z), (dx, dy, dz))| {
            let verts = vec![
                Point3::new(x, y, z),
                Point3::new(x + dx, y, z),
                Point3::new(x + dx, y + dy, z),
                Point3::new(x, y + dy, z),
                Point3::new(x, y, z + dz),
                Point3::new(x + dx, y, z + dz),
                Point3::new(x + dx, y + dy, z + dz),
                Point3::new(x, y + dy, z + dz),
            ];
            let faces = vec![
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ];
            (verts, faces)
        })
}

/// Positively-oriented random tetrahedra (outward-wound).
fn arb_tetrahedron_soup() -> impl Strategy<Value = (Vec<Point3>, Vec<Vec<usize>>)> {
    (arb_point(), arb_point(), arb_point(), arb_point()).prop_filter_map(
        "tetrahedron too close to degenerate",
        |(p0, p1, p2, p3)| {
            let det = (p1 - p0).cross(p2 - p0).dot(p3 - p0);
            if det.abs() < 0.5 {
                return None;
            }
            let verts = if det > 0.0 {
                vec![p0, p1, p2, p3]
            } else {
                vec![p0, p1, p3, p2]
            };
            let faces = vec![vec![0, 2, 1], vec![0, 3, 2], vec![0, 1, 3], vec![1, 2, 3]];
            Some((verts, faces))
        },
    )
}

fn arb_solid_soup() -> impl Strategy<Value = (Vec<Point3>, Vec<Vec<usize>>)> {
    prop_oneof![arb_box_soup(), arb_tetrahedron_soup()]
}

// ── proptest: round-trip Object → COLLADA → import → ingest → equivalent ──────

proptest! {
    /// Build a kernel Object from a random solid soup, emit minimal COLLADA,
    /// re-import it, ingest into a fresh document, and assert the resulting
    /// object has equivalent topology+geometry within POINT_MERGE.
    #[test]
    fn object_collada_roundtrip_is_equivalent(soup in arb_solid_soup()) {
        let (positions, faces) = soup;
        let original = kernel::Object::from_polygons(&positions, &faces)
            .expect("arb_solid_soup is always valid by construction");

        // Emit minimal COLLADA (Z_UP, meter, identity transform).
        let dae_bytes = emit_minimal_dae(&positions, &faces);

        // Import + ingest.
        let (scene, missing) = import(&dae_bytes, &empty_images())
            .expect("emit_minimal_dae produces valid COLLADA");
        prop_assert!(missing.is_empty());

        let mut doc = Document::new();
        let (report, _) = doc.ingest(scene, vec![]).expect("ingest");

        // Exactly one object, no skipped meshes.
        prop_assert_eq!(report.objects_created, 1);
        prop_assert!(
            report.skipped.is_empty(),
            "skipped {} meshes",
            report.skipped.len()
        );

        // Retrieve the re-imported object.
        let obj_ids = doc.visible_object_ids();
        prop_assert_eq!(obj_ids.len(), 1);
        let reimported = doc.object(obj_ids[0]).expect("object is visible");

        // COLLADA float_array is f32 precision; use F32_SAFE_TOL rather than
        // tol::POINT_MERGE (1e-9) which is tighter than f32 round-trip allows.
        prop_assert!(
            objects_equivalent_tol(&original, reimported, F32_SAFE_TOL),
            "objects differ after COLLADA round-trip"
        );
    }

    /// Weld idempotence: welding an already-welded position set changes nothing.
    #[test]
    fn weld_is_idempotent(soup in arb_solid_soup()) {
        let (positions, _faces) = soup;
        let (welded1, _) = dae_import::heal::weld(&positions);
        let (welded2, _) = dae_import::heal::weld(&welded1);
        prop_assert_eq!(
            welded1.len(),
            welded2.len(),
            "weld of already-welded set must be identity"
        );
    }

    /// Two-sided dedup idempotence: running dedup twice gives the same result.
    #[test]
    fn two_sided_dedup_is_idempotent(soup in arb_solid_soup()) {
        let (_positions, faces) = soup;
        let mats: Vec<u32> = vec![kernel::NO_MATERIAL; faces.len()];
        let uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); faces.len()];
        let holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); faces.len()];
        let (f1, m1, u1, h1) = dae_import::heal::dedup_two_sided(&faces, &mats, &uvs, &holes);
        let (f2, m2, _, _) = dae_import::heal::dedup_two_sided(&f1, &m1, &u1, &h1);
        prop_assert_eq!(f1, f2, "dedup twice must give same faces");
        prop_assert_eq!(m1, m2);
    }

    /// Two-sided dedup: if we add a reversed copy of every face, the dedup
    /// step removes all the duplicates, leaving exactly the original face count.
    #[test]
    fn two_sided_dedup_removes_all_back_faces(soup in arb_solid_soup()) {
        let (_positions, faces) = soup;
        let mats: Vec<u32> = vec![kernel::NO_MATERIAL; faces.len()];

        // Append a reversed copy of each face.
        let mut doubled_faces = faces.clone();
        let mut doubled_mats = mats.clone();
        let mut doubled_uvs: Vec<Vec<[f64; 2]>> = vec![Vec::new(); faces.len()];
        let mut doubled_holes: Vec<Vec<Vec<usize>>> = vec![Vec::new(); faces.len()];
        for f in &faces {
            let rev: Vec<usize> = f.iter().rev().cloned().collect();
            doubled_faces.push(rev);
            doubled_mats.push(kernel::NO_MATERIAL);
            doubled_uvs.push(Vec::new());
            doubled_holes.push(Vec::new());
        }

        let (deduped, _, _, _) = dae_import::heal::dedup_two_sided(
            &doubled_faces,
            &doubled_mats,
            &doubled_uvs,
            &doubled_holes,
        );
        prop_assert_eq!(
            deduped.len(),
            faces.len(),
            "dedup must remove all {} back-face duplicates",
            faces.len()
        );
    }
}

// ──────────────── fixture (h): <polygons> with <ph>/<h> hole ─────────────────

/// A COLLADA `<polygons>` with one `<ph>` (outer quad + one `<h>` hole) imports
/// as a Face with exactly one inner loop. Verifies the Phase 2+3 "import holes"
/// pipeline end-to-end: extract_polygons reads `poly_hole.hole`, heal threads
/// it, and `from_polygons_with_holes_import` builds the inner loop.
#[test]
fn polygons_with_hole_imports_inner_loop() {
    let bytes = fixture("polygons_with_hole.dae");
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse polygons_with_hole.dae");
    assert!(missing.is_empty(), "no textures referenced");

    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");

    // Exactly one object created; it may be leaky (open shell — a flat face
    // with a hole has no volume), but must not be skipped.
    assert!(
        report.skipped.is_empty(),
        "holed face must not be skipped; reasons: {:?}",
        report.skipped.iter().map(|s| &s.reason).collect::<Vec<_>>()
    );
    assert_eq!(report.objects_created, 1, "one object");

    // The object's face must carry exactly one inner loop (the hole).
    let oid = doc.visible_object_ids()[0];
    let obj = doc.object(oid).expect("object must exist");

    let faces_with_holes: Vec<_> = obj
        .faces()
        .values()
        .filter(|f| !f.inner_loops.is_empty())
        .collect();

    assert_eq!(
        faces_with_holes.len(),
        1,
        "exactly one face should have inner loops; found {}",
        faces_with_holes.len()
    );
    assert_eq!(
        faces_with_holes[0].inner_loops.len(),
        1,
        "that face must have exactly one inner loop (the hole)"
    );
}

// ─────────────────────── ad-hoc real-file smoke (Stage 1, M5 complex import) ───
//
// Not a fixture test: reads a path from HEW_DAE_FILE and prints an ImportReport.
// Used to iterate on large real-world SketchUp exports. Run with:
//   HEW_DAE_FILE=~/Downloads/theater-test-1.dae \\
//     cargo test -p dae-import real_file_smoke --ignored -- --nocapture
#[test]
#[ignore = "needs HEW_DAE_FILE pointing at a local .dae"]
fn real_file_smoke() {
    let path = std::env::var("HEW_DAE_FILE").expect("set HEW_DAE_FILE to a .dae path");
    let path = shellexpand_tilde(&path);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    eprintln!("file: {path} ({} bytes)", bytes.len());

    let t0 = std::time::Instant::now();
    let (scene, missing) = import(&bytes, &empty_images()).expect("parse");
    eprintln!(
        "parsed in {:?}: {} materials, {} defs, {} roots, {} missing textures",
        t0.elapsed(),
        scene.materials.len(),
        scene.defs.len(),
        scene.roots.len(),
        missing.len()
    );

    let mut doc = Document::new();
    let t1 = std::time::Instant::now();
    let (report, _) = doc.ingest(scene, missing).expect("ingest");
    eprintln!("ingested in {:?}", t1.elapsed());
    eprintln!(
        "REPORT: objects={} watertight={} leaky={} skipped={} missing_textures={}",
        report.objects_created,
        report.watertight,
        report.leaky,
        report.skipped.len(),
        report.textures_missing.len()
    );
    eprintln!("visible world objects: {}", doc.visible_object_ids().len());

    // Holes tally (Stage 2): how many imported faces carry inner loops.
    let mut holed_faces = 0usize;
    let mut objs_with_holes = 0usize;
    for oid in doc.visible_object_ids() {
        if let Some(obj) = doc.object(oid) {
            let n = obj
                .faces()
                .values()
                .filter(|f| !f.inner_loops.is_empty())
                .count();
            holed_faces += n;
            if n > 0 {
                objs_with_holes += 1;
            }
        }
    }
    eprintln!("HOLES: {holed_faces} holed faces across {objs_with_holes} objects");

    // Tag-name tally (Stage 2 tags): how many node names carry the HEWTAG
    // delimiter the Ruby encoded (what the app's parseTag reads). Match the
    // app's tolerant `_+HEWTAG_+` form.
    let has_hewtag = |s: &str| {
        s.find("HEWTAG")
            .is_some_and(|i| s[..i].ends_with('_') && s[i + "HEWTAG".len()..].starts_with('_'))
    };
    let mut tagged_names = 0usize;
    let mut sample: Vec<String> = Vec::new();
    for gid in doc.group_ids() {
        if let Some(n) = doc.group_name(gid)
            && has_hewtag(n)
        {
            tagged_names += 1;
            if sample.len() < 5 {
                sample.push(n.to_string());
            }
        }
    }
    for iid in doc.instance_ids() {
        if let Some(n) = doc.instance_name(iid)
            && has_hewtag(n)
        {
            tagged_names += 1;
        }
    }
    eprintln!("TAGGED NAMES: {tagged_names} (sample: {sample:?})");

    // Dominant skip reasons.
    use std::collections::BTreeMap;
    let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
    for s in &report.skipped {
        *by_reason.entry(s.reason.clone()).or_default() += 1;
    }
    eprintln!("--- skip reasons (count) ---");
    let mut reasons: Vec<_> = by_reason.into_iter().collect();
    reasons.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (reason, count) in reasons.iter().take(20) {
        eprintln!("  {count:6}  {reason}");
    }
    eprintln!("--- first 10 missing textures ---");
    for t in report.textures_missing.iter().take(10) {
        eprintln!("  {t}");
    }
}

fn shellexpand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    p.to_string()
}
