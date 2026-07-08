//! Diagnostic: material fidelity of a .skp import.
//! Run: cargo run -p skp-import --example diag_materials -- <file.skp>

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: diag_materials <file.skp>");
    let bytes = std::fs::read(&path).expect("read skp");
    let out = skp_import::import(&bytes).expect("import");
    let textured = out
        .scene
        .materials
        .iter()
        .filter(|m| m.texture.is_some())
        .count();
    println!(
        "materials: {} ({} textured), textures_missing: {}",
        out.scene.materials.len(),
        textured,
        out.textures_missing.len()
    );
    let mut with_mat = 0usize;
    let mut faces = 0usize;
    for d in &out.scene.defs {
        for m in &d.meshes {
            for &fm in &m.face_materials {
                faces += 1;
                if fm != kernel::NO_MATERIAL {
                    with_mat += 1;
                }
            }
        }
    }
    println!("def faces with a material: {with_mat}/{faces}");
    for m in out.scene.materials.iter().take(12) {
        println!("  mat {:?} textured={}", m.name, m.texture.is_some());
    }
    for name in ["Southern_yellowpine", "OSB Seamless"] {
        let hit = out
            .scene
            .materials
            .iter()
            .any(|m| m.name == name && m.texture.is_some());
        println!("{name}: textured present = {hit}");
    }
}
