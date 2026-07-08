//! Scratch diagnostic: trace what happens to theater nodes on specific layers.
//! Run: cargo run -p skp-import --example diag_layers

use std::collections::BTreeSet;
use std::path::Path;

const TARGETS: &[&str] = &[
    "Projector",
    "Projector Backer Box",
    "Screen Wall Panels",
    "Custom Door",
    "Custom Door Frame",
    "Chairs",
];

fn main() {
    let skp_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../openskp/corpus/third-party/theater-2017.skp");
    let bytes = std::fs::read(&skp_path).expect("theater skp");
    let model = openskp::Model::parse(&bytes).expect("parse");

    // Which meshes did ingest reject? (names)
    let out = skp_import::import(&bytes).expect("import");
    let mut doc = kernel::Document::new();
    let (report, _) = doc.ingest(out.scene, vec![]).expect("ingest");
    let skipped: BTreeSet<&str> = report.skipped.iter().map(|s| s.name.as_str()).collect();

    let scene = model.scene();

    fn dump(n: &openskp::Node, depth: usize, model: &openskp::Model, skipped: &BTreeSet<&str>) {
        let lname = model
            .layer_of(n.layer)
            .map(|l| l.name.as_str())
            .unwrap_or("<default>");
        let def = n.definition.as_deref().unwrap_or("<unnamed>");
        let run_faces = n
            .run
            .map(|ri| model.geometry[ri].mesh.faces.len())
            .unwrap_or(0);
        let layer_visible = model.layer_of(n.layer).map(|l| l.visible).unwrap_or(true);
        println!(
            "{:indent$}[{lname}] def={def:?} hidden={} layer_visible={} run_faces={} children={} skipped_by_ingest={}",
            "",
            n.hidden,
            layer_visible,
            run_faces,
            n.children.len(),
            skipped.contains(def),
            indent = depth * 2
        );
        for c in &n.children {
            dump(c, depth + 1, model, skipped);
        }
    }

    // Only dump subtrees for the interesting parents; chairs/panels are
    // repetitive, so dedup by definition name.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    fn visit(
        nodes: &[openskp::Node],
        model: &openskp::Model,
        skipped: &BTreeSet<&str>,
        seen: &mut BTreeSet<String>,
    ) {
        for n in nodes {
            let lname = model
                .layer_of(n.layer)
                .map(|l| l.name.as_str())
                .unwrap_or("<default>");
            let def = n.definition.clone().unwrap_or_default();
            if TARGETS.contains(&lname) && seen.insert(def) {
                dump(n, 0, model, skipped);
                println!();
            } else {
                visit(&n.children, model, skipped, seen);
            }
        }
    }
    visit(&scene, &model, &skipped, &mut seen);

    println!(
        "\nleaky objects: {} (these render but are not watertight)",
        report.leaky
    );
}
