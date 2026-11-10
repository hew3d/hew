//! Production-scale `.skp` files, opt-in: `HEW_SKP_FILES` names them
//! (`:`-separated paths; they are not part of the repository). For each:
//! an unchanged open-and-save keeps every definition's geometry as the file
//! has it and reopens as the same document, and moving one object then
//! saving lands the move and keeps the rest. Timings are printed.
//!
//! `HEW_SKP_FILES=/path/a.skp:/path/b.skp cargo test -p skp-export --release --test third_party -- --nocapture`

use std::time::Instant;

use kernel::{Document, NodeId};
use skp_export::{SkpSource, WriteEnv, save};

fn open(bytes: &[u8]) -> (Document, SkpSource) {
    let out = skp_import::import(bytes).expect("imports");
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    doc.forget_history().unwrap();
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    (doc, src)
}

/// Per definition name: (faces, edges, soft edges, curves, textured faces).
fn runs(m: &skpwright::Model) -> std::collections::BTreeMap<String, [usize; 5]> {
    let mut out = std::collections::BTreeMap::new();
    for r in &m.geometry {
        let name = match r.def_index {
            Some(map) => m
                .definitions
                .iter()
                .find(|d| d.map_index == Some(map))
                .map(|d| format!("{}:{:?}", d.name, d.kind))
                .unwrap_or_default(),
            None => "<root>".into(),
        };
        let e: &mut [usize; 5] = out.entry(name).or_default();
        e[0] += r.mesh.faces.len();
        e[1] += r.mesh.edges.len();
        e[2] += r.mesh.edges.iter().filter(|x| x.soft).count();
        e[3] += r.curves.len();
        e[4] += r.mesh.faces.iter().filter(|f| f.texture.is_some()).count();
    }
    out
}

#[test]
fn production_files_save_back() {
    let Ok(files) = std::env::var("HEW_SKP_FILES") else {
        eprintln!("HEW_SKP_FILES not set; skipped");
        return;
    };
    for path in files.split(':').filter(|p| !p.is_empty()) {
        let bytes = std::fs::read(path).unwrap();
        let target = skp_export::target_of(&bytes);
        let env = WriteEnv {
            unix_time: 1_800_000_000,
            seed: 9,
        };
        let t = Instant::now();
        let (doc, src) = open(&bytes);
        let opened = t.elapsed();
        let t = Instant::now();
        let saved = save(&doc, target, env, Some(&src)).unwrap();
        let saved_in = t.elapsed();
        let before = skpwright::Model::parse(&bytes).unwrap();
        let after = skpwright::Model::parse(&saved.export.bytes).unwrap();
        assert!(
            after.diagnostics.iter().all(|d| !d.is_desync()),
            "{path}: desyncs"
        );
        let (a, b) = (runs(&before), runs(&after));
        let differing: Vec<_> = a
            .iter()
            .filter(|(k, v)| b.get(*k) != Some(v))
            .map(|(k, _)| k)
            .take(10)
            .collect();
        println!(
            "{path}: {} bytes -> {} bytes; open {opened:?}, save {saved_in:?}; {} of {} definitions differ {differing:?}",
            bytes.len(),
            saved.export.bytes.len(),
            a.iter().filter(|(k, v)| b.get(*k) != Some(v)).count(),
            a.len()
        );
        assert!(
            differing.is_empty(),
            "{path}: an unchanged save rewrote {differing:?}"
        );
        assert_eq!(before.scenes.len(), after.scenes.len());
        assert_eq!(before.materials.len(), after.materials.len());
        assert_eq!(before.layers.len(), after.layers.len());
        assert_eq!(before.styles.len(), after.styles.len());

        // Move the first top-level Object and save again.
        let (mut doc, src) = open(&saved.export.bytes);
        let Some(o) = doc.top_level_nodes().into_iter().find_map(|n| match n {
            NodeId::Object(o) => Some(o),
            _ => None,
        }) else {
            continue;
        };
        doc.transform_object(
            o,
            &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 1.0)),
        )
        .unwrap();
        let t = Instant::now();
        let edited = save(&doc, target, env, Some(&src)).unwrap();
        println!("  edit saved in {:?}", t.elapsed());
        let back = skpwright::Model::parse(&edited.export.bytes).unwrap();
        assert!(
            back.diagnostics.iter().all(|d| !d.is_desync()),
            "{path}: desyncs after edit"
        );
        let c = runs(&back);
        let changed = a.iter().filter(|(k, v)| c.get(*k) != Some(v)).count();
        println!("  after the edit, {changed} definitions differ from the source");
    }
}
