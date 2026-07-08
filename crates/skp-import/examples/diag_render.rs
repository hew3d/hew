//! Diagnostic: per-def-member render-visibility predictors — signed volume
//! (negative = inside-out under single-sided rendering) and instance pose
//! determinants (negative = mirrored placement).
//! Run: cargo run -p skp-import --example diag_render -- <file.skp>

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: diag_render <file.skp>");
    let bytes = std::fs::read(&path).expect("read skp");
    let out = skp_import::import(&bytes).expect("import");
    let mut doc = kernel::Document::new();
    let (_report, _) = doc.ingest(out.scene, out.textures_missing).expect("ingest");

    let mut inside_out = 0usize;
    let mut total = 0usize;
    for cid in doc.component_ids() {
        let name = doc.component_name(cid).unwrap_or("").to_string();
        let interesting = [
            "La-Z-Boy", "Door", "door", "Screen", "Drywall", "OSB", "Group#53",
        ]
        .iter()
        .any(|k| name.contains(k));
        for oid in doc.def_members(cid).unwrap_or_default() {
            let Some(obj) = doc.object(oid) else { continue };
            total += 1;
            let mesh = tessellate::tessellate(obj, doc.materials()).expect("tess");
            // signed volume via divergence theorem over triangles
            let mut vol = 0.0f64;
            let p = &mesh.positions;
            for t in mesh.indices.chunks_exact(3) {
                let (a, b, c) = (t[0] as usize * 3, t[1] as usize * 3, t[2] as usize * 3);
                let av = [p[a] as f64, p[a + 1] as f64, p[a + 2] as f64];
                let bv = [p[b] as f64, p[b + 1] as f64, p[b + 2] as f64];
                let cv = [p[c] as f64, p[c + 1] as f64, p[c + 2] as f64];
                vol += av[0] * (bv[1] * cv[2] - bv[2] * cv[1])
                    - av[1] * (bv[0] * cv[2] - bv[2] * cv[0])
                    + av[2] * (bv[0] * cv[1] - bv[1] * cv[0]);
            }
            let watertight = doc.object_solid(oid);
            if watertight && vol < 0.0 {
                inside_out += 1;
            }
            if interesting {
                println!(
                    "{name:?} member: watertight={watertight} signed_vol={:+.6}",
                    vol / 6.0
                );
            }
        }
    }
    println!("total members {total}, watertight-inside-out {inside_out}");

    // Instance pose determinants for interesting defs
    for iid in doc.instance_ids() {
        let Some(cid) = doc.instance_def(iid) else {
            continue;
        };
        let Some(pose) = doc.instance_pose(iid) else {
            continue;
        };
        let name = doc.component_name(cid).unwrap_or("").to_string();
        if ["La-Z-Boy", "Screen", "Drywall"]
            .iter()
            .any(|k| name.contains(k))
        {
            let a = pose.to_affine();
            let det = a[0] * (a[5] * a[10] - a[6] * a[9]) - a[1] * (a[4] * a[10] - a[6] * a[8])
                + a[2] * (a[4] * a[9] - a[5] * a[8]);
            println!("inst of {name:?}: det={det:+.4}");
        }
    }
}
