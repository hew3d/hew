//! Diagnostic: resolved material alphas for named defs — a near-zero alpha
//! renders faces invisible (the "wireframe" look) while edges stay.
//! Run: cargo run -p skp-import --example diag_alpha -- <file.skp>

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: diag_alpha <file.skp>");
    let bytes = std::fs::read(&path).expect("read skp");
    let out = skp_import::import(&bytes).expect("import");
    let mut doc = kernel::Document::new();
    let (_report, _) = doc.ingest(out.scene, out.textures_missing).expect("ingest");

    for cid in doc.component_ids() {
        let name = doc.component_name(cid).unwrap_or("").to_string();
        let interesting = [
            "La-Z-Boy",
            "Door",
            "door",
            "AT Screen",
            "Drywall",
            "Screen Wall",
        ]
        .iter()
        .any(|k| name.contains(k));
        if !interesting {
            continue;
        }
        for oid in doc.def_members(cid).unwrap_or_default() {
            let Some(obj) = doc.object(oid) else { continue };
            let base = obj.default_material();
            let base_desc = base
                .and_then(|m| doc.material(m))
                .map(|m| format!("{} a={}", m.name, m.color.a))
                .unwrap_or_else(|| "none".into());
            let mut face_mats: std::collections::BTreeMap<String, usize> = Default::default();
            for (_, f) in obj.faces() {
                let key = f
                    .material
                    .and_then(|m| doc.material(m))
                    .map(|m| format!("{} a={}", m.name, m.color.a))
                    .unwrap_or_else(|| "default".into());
                *face_mats.entry(key).or_default() += 1;
            }
            println!("{name:?}: base=[{base_desc}] faces={face_mats:?}");
        }
    }
}
