//! Golden-file test for the USDZ writer (docs/design/shop-mode.md §5): a
//! representative document — a plain unpainted box (exercises the shared
//! default-gray fallback), a multi-colored box (exercises `GeomSubset`
//! face-group binding, module doc "Materials"), and a component instanced
//! twice with one instance mirrored (exercises the instance/winding-flip
//! path [`mesh_export::collect_export_solids`] shares with 3MF/GLB) — the
//! same shape `mesh-export`'s own
//! `export_3mf_handles_a_painted_multi_object_instanced_model` checks
//! itself against, exported once and pinned byte-for-byte against a
//! committed fixture.
//!
//! Regenerate intentionally (a version bump — the layer's `doc = "Hew
//! <version>"` metadata line changes with it — or a deliberate spec
//! change) with:
//!
//! ```sh
//! REGENERATE_GOLDEN=1 cargo test -p mesh-export --test usdz_golden
//! ```

use std::path::PathBuf;

use kernel::{
    Document, ImportNode, ImportScene, Material, MeshRecipe, NodeId, Point3, Rgba8, Transform, Vec3,
};

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/representative.usdz")
}

/// A unit-cube mesh recipe (CCW-from-outside faces) — the same fixture
/// `mesh-export`'s own unit tests and `crates/hew-cli/src/host.rs`'s tests
/// use; redeclared here since it is private to the lib crate.
fn unit_box_mesh(name: &str) -> MeshRecipe {
    let positions = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let face_count = faces.len();
    MeshRecipe {
        name: name.to_string(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; face_count],
        face_uv_frames: vec![None; face_count],
        face_holes: vec![Vec::new(); face_count],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
    }
}

/// Builds the canonical representative document this golden pins.
fn build_representative_doc() -> Document {
    let mut doc = Document::new();

    // A plain top-level box: every face falls back to the shared default
    // gray material.
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(unit_box_mesh("Box"))],
        guides: Vec::new(),
        tags: Vec::new(),
    };
    doc.ingest(scene, Vec::new()).expect("ingest a plain box");

    // A painted top-level box: face 0 red, face 1 blue, faces 2-5 default
    // gray — three distinct colors on one part, forcing the GeomSubset path.
    let mut painted_mesh = unit_box_mesh("Painted");
    painted_mesh.face_materials[0] = 0;
    painted_mesh.face_materials[1] = 1;
    let scene = ImportScene {
        materials: vec![
            Material::solid("Red", Rgba8::rgb(0xff, 0x00, 0x00)),
            Material::solid("Blue", Rgba8::rgb(0x00, 0x00, 0xff)),
        ],
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(painted_mesh)],
        guides: Vec::new(),
        tags: Vec::new(),
    };
    doc.ingest(scene, Vec::new()).expect("ingest a painted box");

    // A component (plain box) placed twice: identity translation, then a
    // mirrored (negative-determinant) pose — exercises the instance
    // traversal and winding-flip path.
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(unit_box_mesh("LegSrc"))],
        guides: Vec::new(),
        tags: Vec::new(),
    };
    doc.ingest(scene, Vec::new())
        .expect("ingest the component source box");
    let leg_src_id = *doc
        .visible_object_ids()
        .iter()
        .find(|&&id| doc.object_name(id) == Some("LegSrc"))
        .expect("the component source object is present");
    let (component, _first_instance, _change) = doc
        .make_component(&[NodeId::Object(leg_src_id)])
        .expect("make a component from the plain box");
    doc.place_instance(component, Transform::translation(Vec3::new(4.0, 0.0, 0.0)))
        .expect("place a second instance");
    let mirrored = Transform::scale(Vec3::new(-1.0, 1.0, 1.0));
    doc.place_instance(component, mirrored)
        .expect("place a mirrored third instance");

    doc
}

#[test]
fn usdz_export_matches_the_committed_golden_fixture() {
    let doc = build_representative_doc();
    let bytes =
        mesh_export::export_usdz(&doc, 0, true).expect("the representative doc exports to usdz");

    let path = golden_path();
    if std::env::var("REGENERATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create tests/golden");
        std::fs::write(&path, &bytes).expect("write golden fixture");
    }

    let expected = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden fixture at {}; run with REGENERATE_GOLDEN=1 to create it",
            path.display()
        )
    });
    assert_eq!(
        bytes,
        expected,
        "USDZ export drifted from the committed golden fixture at {}; if intentional \
         (a version bump, a deliberate spec change), regenerate with REGENERATE_GOLDEN=1",
        path.display()
    );
}
