//! Diagnostic: any .skp end-to-end through skp-import + kernel ingest.
//! Run: cargo run -p skp-import --example diag_import -- <file.skp>

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: diag_import <file.skp>");
    let bytes = std::fs::read(&path).expect("read skp");
    println!("parsing {} ({} bytes)…", path, bytes.len());

    let out = skp_import::import(&bytes).expect("import");
    println!("warnings: {}", out.warnings.len());
    for w in out.warnings.iter().take(5) {
        println!("  {w}");
    }

    let mut doc = kernel::Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).expect("ingest");
    println!(
        "objects: {} (watertight {}, leaky {}), skipped {}, instances {}, groups {}",
        report.objects_created,
        report.watertight,
        report.leaky,
        report.skipped.len(),
        doc.instance_ids().len(),
        doc.group_ids().len()
    );
}
