//! Diagnostic: per-object triangle yield after a .skp import — a "wireframe"
//! object is one whose tessellation yields edges but zero face triangles.
//! Run: cargo run -p skp-import --example diag_tris -- <file.skp>

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: diag_tris <file.skp>");
    let bytes = std::fs::read(&path).expect("read skp");
    let out = skp_import::import(&bytes).expect("import");
    let mut doc = kernel::Document::new();
    let (_report, _) = doc.ingest(out.scene, out.textures_missing).expect("ingest");

    let mut zero_tri = 0usize;
    let mut partial = 0usize;
    let mut total = 0usize;
    let mut all: Vec<kernel::ObjectId> = doc.visible_object_ids();
    for cid in doc.component_ids() {
        all.extend(doc.def_members(cid).unwrap_or_default());
    }
    for oid in all {
        let Some(obj) = doc.object(oid) else { continue };
        total += 1;
        let name = format!("{oid:?}");
        match tessellate::tessellate(obj, doc.materials()) {
            Ok(mesh) => {
                let tris = mesh.indices.len() / 3;
                let nfaces = obj.faces().len();
                // count faces that produced no triangles
                if tris == 0 {
                    zero_tri += 1;
                    println!("ZERO-TRI: {name:?} ({nfaces} faces)");
                } else if tris < nfaces {
                    partial += 1;
                    if partial <= 15 {
                        println!("PARTIAL: {name:?} ({nfaces} faces -> {tris} tris)");
                    }
                }
            }
            Err(e) => println!("TESS-ERR: {name:?}: {e:?}"),
        }
    }
    println!("total {total}, zero-tri {zero_tri}, partial {partial}");
}
