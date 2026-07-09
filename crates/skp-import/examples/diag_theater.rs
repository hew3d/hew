//! Scratch diagnostic: quantify theater-2017 import losses stage by stage.
//! Run: cargo run -p skp-import --example diag_theater

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use kernel::{ImportNode, ImportScene, MeshRecipe, Point3, Transform};

fn flatten(scene: &ImportScene) -> (usize, [f64; 3], [f64; 3]) {
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
    (faces, lo, hi)
}

fn collect_tags(scene: &ImportScene, out: &mut BTreeSet<String>) {
    fn node_tags(n: &ImportNode, out: &mut BTreeSet<String>) {
        match n {
            ImportNode::Mesh(m) => {
                for t in &m.tags {
                    out.insert(t.join("/"));
                }
            }
            ImportNode::Instance { tags, .. } => {
                for t in tags {
                    out.insert(t.join("/"));
                }
            }
            ImportNode::Group { children, tags, .. } => {
                for t in tags {
                    out.insert(t.join("/"));
                }
                for c in children {
                    node_tags(c, out);
                }
            }
        }
    }
    for n in &scene.roots {
        node_tags(n, out);
    }
    for d in &scene.defs {
        for m in &d.meshes {
            for t in &m.tags {
                out.insert(t.join("/"));
            }
        }
    }
}

fn main() {
    let skp_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../openskp/corpus/third-party/theater-2017.skp");
    let dae_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../openskp/corpus/third-party/theater-2017.dae");
    let skp_bytes = std::fs::read(&skp_path).expect("theater skp");

    // ── Stage 0: raw OpenSKP model ──
    let model = openskp::Model::parse(&skp_bytes).expect("openskp parse");
    println!("== OpenSKP raw ==");
    println!("layers: {}", model.layers.len());
    println!("definitions: {}", model.definitions.len());
    println!("instances (records): {}", model.instances.len());
    println!("geometry runs: {}", model.geometry.len());
    let scene_nodes = model.scene();
    fn count_nodes(nodes: &[openskp::Node]) -> usize {
        nodes.iter().map(|n| 1 + count_nodes(&n.children)).sum()
    }
    println!(
        "composed scene nodes (recursive): {}",
        count_nodes(&scene_nodes)
    );

    // Layer slot usage: which layers are referenced by scene nodes / faces
    let mut node_slots: BTreeSet<u16> = BTreeSet::new();
    fn slots(nodes: &[openskp::Node], out: &mut BTreeSet<u16>) {
        for n in nodes {
            out.insert(n.layer);
            slots(&n.children, out);
        }
    }
    slots(&scene_nodes, &mut node_slots);
    let mut face_slots: BTreeSet<u16> = BTreeSet::new();
    for run in &model.geometry {
        for f in &run.mesh.faces {
            face_slots.insert(f.layer);
        }
    }
    let used: BTreeSet<u16> = node_slots.union(&face_slots).copied().collect();
    println!(
        "layer slots used by scene nodes: {}, by faces: {}, union: {}",
        node_slots.len(),
        face_slots.len(),
        used.len()
    );
    let hidden: Vec<&str> = model
        .layers
        .iter()
        .filter(|l| !l.visible)
        .map(|l| l.name.as_str())
        .collect();
    println!("hidden layers ({}): {:?}", hidden.len(), hidden);

    // Map slot -> layer name to see which layers are entirely unreferenced.
    // layer_of(slot) resolves slots; enumerate all possible slots seen.
    let mut named_used: BTreeSet<String> = BTreeSet::new();
    for &s in &used {
        if let Some(l) = model.layer_of(s) {
            named_used.insert(l.name.clone());
        }
    }
    let all_names: BTreeSet<String> = model.layers.iter().map(|l| l.name.clone()).collect();
    let unreferenced: Vec<&String> = all_names.difference(&named_used).collect();
    println!(
        "layers never referenced by any node/face: {} -> {:?}",
        unreferenced.len(),
        unreferenced
    );

    // ── Stage 1: skp-import scene ──
    let out = skp_import::import(&skp_bytes).expect("skp import");
    let (s_faces, s_lo, s_hi) = flatten(&out.scene);
    let mut skp_tags = BTreeSet::new();
    collect_tags(&out.scene, &mut skp_tags);
    println!("\n== skp-import scene ==");
    println!("flattened faces: {s_faces}");
    println!("AABB lo {s_lo:?} hi {s_hi:?}");
    println!("distinct tags in scene: {}", skp_tags.len());
    println!("warnings: {}", out.warnings.len());

    let missing_tags: Vec<&String> = all_names
        .iter()
        .filter(|n| !n.is_empty() && !skp_tags.contains(*n))
        .collect();
    println!(
        "layers with NO tag in scene: {} -> {:?}",
        missing_tags.len(),
        missing_tags
    );

    // ── Stage 2: kernel ingest ──
    let mut doc = kernel::Document::new();
    let (report, _) = doc
        .ingest(out.scene, out.textures_missing.clone())
        .expect("ingest");
    println!("\n== kernel ingest ==");
    println!(
        "objects: {} (watertight {}, leaky {})",
        report.objects_created, report.watertight, report.leaky
    );
    println!("instances: {}", doc.instance_ids().len());
    println!("groups: {}", doc.group_ids().len());
    println!("skipped meshes: {}", report.skipped.len());
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for s in &report.skipped {
        *reasons.entry(s.reason.clone()).or_default() += 1;
    }
    for (r, c) in &reasons {
        println!("  {c}x {r}");
    }
    println!("skipped names:");
    for s in &report.skipped {
        println!("  - {}", s.name);
    }

    // ── Ground truth: SketchUp's own .dae export ──
    if let Ok(dae_bytes) = std::fs::read(&dae_path) {
        let images = dae_import::ImageMap::new();
        let dscene = dae_import::import(&dae_bytes, &images)
            .expect("dae import")
            .scene;
        let (d_faces, d_lo, d_hi) = flatten(&dscene);
        let mut dae_tags = BTreeSet::new();
        collect_tags(&dscene, &mut dae_tags);
        println!("\n== dae ground truth ==");
        println!("flattened faces: {d_faces}");
        println!("AABB lo {d_lo:?} hi {d_hi:?}");
        println!("distinct tags in dae scene: {}", dae_tags.len());
        println!(
            "\nface delta skp vs dae: {} vs {} ({:+})",
            s_faces,
            d_faces,
            s_faces as i64 - d_faces as i64
        );
        let only_dae: Vec<&String> = dae_tags.difference(&skp_tags).collect();
        let only_skp: Vec<&String> = skp_tags.difference(&dae_tags).collect();
        println!("tags only in dae: {only_dae:?}");
        println!("tags only in skp: {only_skp:?}");
    } else {
        println!("\n(no .dae ground truth found)");
    }
}
