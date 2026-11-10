//! Differential validation: every fixture ships as a `.skp` + the
//! `.dae` SketchUp 2017 itself exported from the same model, and as the
//! 2026 container SketchUp 2026 saved from the 2017 file
//! (`fixtures/2026/`). Both importers must agree on the world-space result
//! — flattened face totals and the world-space bounding box — from either
//! container, even where they differ structurally (the `.dae` path
//! bakes/flattens hierarchy; the `.skp` path keeps shared definitions).
//!
//! Import-quality gaps surfacing here get filed upstream in Skpwright, never
//! papered over in `skp-import` (docs/agents/ROADMAP.md).

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

    // Walks nested definitions: an Instance composes its pose and descends
    // into its def's OWN subtree (meshes, groups, nested instances).
    fn walk(
        nodes: &[ImportNode],
        tf: &Transform,
        scene: &ImportScene,
        faces: &mut usize,
        lo: &mut [f64; 3],
        hi: &mut [f64; 3],
    ) {
        for n in nodes {
            match n {
                ImportNode::Mesh(m) => take(m, tf, faces, lo, hi),
                ImportNode::Instance { def, pose, .. } => {
                    let composed = pose.then(tf);
                    walk(&scene.defs[*def].children, &composed, scene, faces, lo, hi);
                }
                ImportNode::Group { children, .. } => walk(children, tf, scene, faces, lo, hi),
            }
        }
    }

    walk(
        &scene.roots,
        &Transform::IDENTITY,
        scene,
        &mut faces,
        &mut lo,
        &mut hi,
    );
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

/// Every face SketchUp itself holds, each placed definition's once per
/// placement.
fn sketchup_faces(name: &str) -> usize {
    let m = skp_import::skpwright::Model::parse(&fixture(name)).expect(name);
    fn walk(nodes: &[skp_import::skpwright::Node], m: &skp_import::skpwright::Model) -> usize {
        nodes
            .iter()
            .map(|n| n.run.map_or(0, |r| m.geometry[r].mesh.faces.len()) + walk(&n.children, m))
            .sum()
    }
    let root: usize = m
        .geometry
        .iter()
        .filter(|r| r.def_index.is_none())
        .map(|r| r.mesh.faces.len())
        .sum();
    walk(&m.scene(), &m) + root
}

fn assert_pair(stem: &str) {
    let d = dae_flat(&format!("{stem}.dae"));
    for skp in [format!("{stem}.skp"), format!("2026/{stem}.skp")] {
        let s = skp_flat(&skp);
        // The `.skp` path keeps every face SketchUp holds; the `.dae` path
        // re-merges SketchUp's triangulated export, which also merges
        // coplanar faces SketchUp keeps apart (house's), so it may count
        // fewer.
        assert_eq!(
            s.faces,
            sketchup_faces(&skp),
            "{skp}: flattened world face total is not SketchUp's own"
        );
        assert!(
            d.faces <= s.faces,
            "{skp}: the .dae holds faces the .skp lacks (skp {s:?} vs dae {d:?})"
        );
        for i in 0..3 {
            assert!(
                (s.lo[i] - d.lo[i]).abs() < AABB_TOL && (s.hi[i] - d.hi[i]).abs() < AABB_TOL,
                "{skp}: world AABB differs on axis {i} (skp {s:?} vs dae {d:?})"
            );
        }
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

/// The 10.7 MB third-party production model (theater), end-to-end, from
/// the 2017 original and from its 2026 conversion (7.4 MB), which must
/// land the same frozen numbers. The files are too large to freeze as
/// fixtures; the tests run wherever the Skpwright dev clone sits next to the
/// Hew checkout (every dev box) and skip silently elsewhere.
#[test]
fn theater_production_model_end_to_end() {
    theater("theater-2017.skp");
}

#[test]
fn theater_2026_production_model_end_to_end() {
    theater("theater-2026.skp");
}

fn theater(file: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../skpwright/corpus/third-party")
        .join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipped: no ../skpwright checkout");
        return;
    };
    let out = skp_import::import(&bytes).expect("theater parses");
    // A clean production file produces NO parser-recovery warnings. It DOES
    // produce non-manifold split notices (rule 4: decomposition is loud):
    // theater carries 32 non-manifold source meshes (ceiling speakers,
    // La-Z-Boy chair sides, the projector body, PVC fittings).
    assert!(
        !out.warnings
            .iter()
            .any(|w| w.contains("parser recovered") || w.contains("reader skipped")),
        "clean production file, no reader recovery: {:?}",
        out.warnings
    );
    assert_eq!(
        out.warnings
            .iter()
            .filter(|w| w.contains("separate piece"))
            .count(),
        // Four more than when coplanar faces merged on import: merging
        // removed the edges that made them non-manifold, but none of the
        // four was a closed solid either way.
        32
    );

    // Nothing missing: theater's shared-texture records (four, across three
    // owning materials) resolve to the owners' image bytes (Skpwright 0.2.0
    // back-ref resolution), so the loud rule-4 channel stays empty.
    assert!(
        out.textures_missing.is_empty(),
        "shared-texture back-refs resolve: {:?}",
        out.textures_missing
    );

    let mut doc = kernel::Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    // Frozen with the group/component split (`is_group`): NESTED
    // definitions (manifest v15) still nest genuine components (e.g. the
    // "Automatic Door Bottom" family), but SketchUp GROUPS — previously
    // misclassified as shared definitions by the byte-scan-only heuristic
    // — are now independent containers, deep-copied per placement instead
    // of shared. That drops component_ids from 722 to 151 (only the real
    // components remain) and instance_ids from 962 to 295, while 29 groups
    // now reach the WORLD level as genuine `Group`s (nested ones live
    // inside their owning definition and don't count here — see
    // `group_ids`'s doc). Groups no longer sharing geometry through a def
    // means their member solids count once PER PLACEMENT, so
    // `objects_created` RISES (738 -> 979) even as the shared-definition
    // graph shrinks — conservation still holds (geometry parity against
    // the .dae ground truths above, unaffected by this change). The
    // 13 user-hidden nodes are untouched: hidden carries identically for
    // both node kinds. If an Skpwright or heal improvement moves these,
    // update deliberately with the rev bump.
    // Coplanar faces stay apart as SketchUp holds them, so non-manifold
    // meshes split into 16 more open pieces (979 -> 995, leaky 139 -> 155,
    // placements 692 -> 700, four more split meshes' groups 29 -> 33);
    // every watertight solid stays one (840).
    // Edges bounding no face are drafting now, held by their group: 12
    // world groups of lines alone arrive that used to be dropped
    // (33 -> 45).
    assert_eq!(report.objects_created, 995);
    assert_eq!(report.watertight, 840);
    assert_eq!(report.leaky, 155);
    assert_eq!(report.skipped.len(), 0);
    assert_eq!(doc.instance_ids().len(), 295);
    assert_eq!(doc.group_ids().len(), 45);
    assert_eq!(doc.user_hidden_nodes().len(), 13);
    assert_eq!(doc.component_ids().len(), 151);
    assert_eq!(doc.expanded_placements().len(), 700);
    assert_eq!(
        doc.component_ids()
            .iter()
            .map(|&c| doc.def_depth(c))
            .max()
            .unwrap_or(0),
        5,
        "the definition graph genuinely nests"
    );
    // The nested document survives a byte-stable v15 round trip.
    let bytes = doc.save();
    let re = kernel::Document::load(&bytes).expect("theater round-trips");
    assert_eq!(re.save(), bytes);
    // The full 94-layer list survives as tags: 93 registered (Layer0 maps
    // to "untagged"), 40 of them hidden-by-default.
    let tags: Vec<(&[String], bool)> = doc.tag_meta().collect();
    assert_eq!(tags.len(), 93);
    assert_eq!(tags.iter().filter(|(_, hidden)| *hidden).count(), 40);
}
