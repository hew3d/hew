//! Differential validation: every fixture ships as a `.skp` + the
//! `.dae` SketchUp 2017 itself exported from the same model. Both importers
//! must agree on the world-space result — flattened face totals and the
//! world-space bounding box — even where they differ structurally (the `.dae`
//! path bakes/flattens hierarchy; the `.skp` path keeps shared definitions).
//!
//! Import-quality gaps surfacing here get filed upstream in OpenSKP, never
//! papered over in `skp-import` (ROADMAP).

use std::path::Path;

use kernel::{ImportNode, ImportScene, MeshRecipe, Point3, Transform};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()))
}

#[derive(Debug, PartialEq)]
struct Flat {
    faces: usize,
    lo: [f64; 3],
    hi: [f64; 3],
}

/// Flatten an `ImportScene` to world space: total face count and AABB over
/// every placed vertex (instances expanded through their poses).
fn flatten(scene: &ImportScene) -> Flat {
    let mut faces = 0usize;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];

    fn take(
        mesh: &MeshRecipe,
        tf: &Transform,
        faces: &mut usize,
        lo: &mut [f64; 3],
        hi: &mut [f64; 3],
    ) {
        *faces += mesh.faces.len();
        for &p in &mesh.positions {
            let q: Point3 = tf.apply_point(p);
            for (i, v) in [q.x, q.y, q.z].into_iter().enumerate() {
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
            }
        }
    }

    fn walk(
        nodes: &[ImportNode],
        scene: &ImportScene,
        faces: &mut usize,
        lo: &mut [f64; 3],
        hi: &mut [f64; 3],
    ) {
        for n in nodes {
            match n {
                ImportNode::Mesh(m) => take(m, &Transform::IDENTITY, faces, lo, hi),
                ImportNode::Instance { def, pose, .. } => {
                    for m in &scene.defs[*def].meshes {
                        take(m, pose, faces, lo, hi);
                    }
                }
                ImportNode::Group { children, .. } => walk(children, scene, faces, lo, hi),
            }
        }
    }

    walk(&scene.roots, scene, &mut faces, &mut lo, &mut hi);
    Flat { faces, lo, hi }
}

fn skp_flat(name: &str) -> Flat {
    let out = skp_import::import(&fixture(name)).expect(name);
    flatten(&out.scene)
}

fn dae_flat(name: &str) -> Flat {
    let images = dae_import::ImageMap::new();
    let scene = dae_import::import(&fixture(name), &images)
        .expect(name)
        .scene;
    flatten(&scene)
}

/// World-space AABBs agree to a millimetre.
const AABB_TOL: f64 = 1e-3;

fn assert_pair(stem: &str) {
    let s = skp_flat(&format!("{stem}.skp"));
    let d = dae_flat(&format!("{stem}.dae"));
    assert_eq!(
        s.faces, d.faces,
        "{stem}: flattened world face totals differ (skp {s:?} vs dae {d:?})"
    );
    for i in 0..3 {
        assert!(
            (s.lo[i] - d.lo[i]).abs() < AABB_TOL && (s.hi[i] - d.hi[i]).abs() < AABB_TOL,
            "{stem}: world AABB differs on axis {i} (skp {s:?} vs dae {d:?})"
        );
    }
}

macro_rules! differential {
    ($($test:ident => $stem:literal),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                assert_pair($stem);
            }
        )+
    };
}

differential! {
    diff_box => "box",
    diff_box_two_materials => "box-two-materials",
    diff_back_material => "back-material",
    diff_box_component => "box-component",
    diff_box_component_two_instances => "box-component-two-instances",
    diff_box_group => "box-group",
    diff_group => "group",
    diff_face_with_hole => "face-with-hole",
    diff_layers => "layers",
    diff_hidden_entities => "hidden-entities",
    diff_instance_scaled => "instance-scaled",
    diff_component_move => "component-move",
    diff_component_rotate => "component-rotate",
    diff_material_one_face => "material-one-face",
    // Nested components (a def placing another def via <instance_node>,
    // e.g. mixed-definition's half-box, nested-3-deep's 3-level chain, and
    // house's Front Entry Door + its nested door slab). Previously the `.dae`
    // path silently dropped this content and mis-composed nested transforms;
    // fixed by threading transform accumulation + <instance_node> handling
    // through `collect_meshes_from_node` (dae-import). Promoted from the
    // frozen `.skp`-side pairs these used to be.
    diff_nested_3_deep => "nested-3-deep",
    diff_mixed_definition => "mixed-definition",
    diff_house => "house",
}

// ── Full-scale stress model ──────────────────────────────────────────────────

/// The 10.7 MB third-party production model (theater), end-to-end. The file
/// is too large to freeze as a fixture; the test runs wherever the OpenSKP
/// dev clone sits next to the Hew checkout (every dev box) and skips
/// silently elsewhere.
#[test]
fn theater_production_model_end_to_end() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../openskp/corpus/third-party/theater-2017.skp");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipped: no ../openskp checkout");
        return;
    };
    let out = skp_import::import(&bytes).expect("theater parses");
    // A clean production file produces NO parser-recovery warnings. It DOES
    // produce non-manifold split notices (rule 4: decomposition is loud):
    // theater carries 28 non-manifold source meshes (ceiling speakers,
    // La-Z-Boy chair sides, the projector body, PVC fittings).
    assert!(
        !out.warnings.iter().any(|w| w.contains("parser recovered")),
        "clean production file, no parser recovery: {:?}",
        out.warnings
    );
    assert_eq!(
        out.warnings
            .iter()
            .filter(|w| w.contains("is non-manifold; imported as"))
            .count(),
        28
    );

    // Nothing missing: theater's shared-texture records (four, across three
    // owning materials) resolve to the owners' image bytes (OpenSKP 0.2.0
    // back-ref resolution), so the loud rule-4 channel stays empty.
    assert!(
        out.textures_missing.is_empty(),
        "shared-texture back-refs resolve: {:?}",
        out.textures_missing
    );

    let mut doc = kernel::Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    // Frozen with hidden-layer-content import + the non-manifold splitter
    // + per-node hidden import (hidden layers become hidden-by-default
    // tags whose content IMPORTS, non-manifold meshes decompose into open
    // shells instead of being rejected, and per-entity-hidden
    // groups/components import as USER-hidden nodes instead of dropping —
    // the original 524-object baseline did none of these): 738 objects
    // (602 watertight), NOTHING skipped, 1319 instances, 118 groups, 26
    // user-hidden nodes. If an OpenSKP or heal improvement moves these,
    // update deliberately with the rev bump.
    assert_eq!(report.objects_created, 738);
    assert_eq!(report.watertight, 602);
    assert_eq!(report.skipped.len(), 0);
    assert_eq!(doc.instance_ids().len(), 1319);
    assert_eq!(doc.group_ids().len(), 118);
    assert_eq!(doc.user_hidden_nodes().len(), 26);
    // The full 94-layer list survives as tags: 93 registered (Layer0 maps
    // to "untagged"), 40 of them hidden-by-default.
    let tags: Vec<(&[String], bool)> = doc.tag_meta().collect();
    assert_eq!(tags.len(), 93);
    assert_eq!(tags.iter().filter(|(_, hidden)| *hidden).count(), 40);
}
