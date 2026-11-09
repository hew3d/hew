//! Writes the `.skp` acceptance set: every file a real SketchUp should open
//! without a dialog and save back equivalent. For each case it writes
//! `<name>.skp` and `<name>.expect.hew` (the document that was written);
//! after SketchUp's own save of each lands as `<name>-resaved.skp`,
//! `tests/sketchup_resaves.rs` holds what Hew reads back from it to the
//! expectation.
//!
//! ```text
//! cargo run --release -p skp-export --example skp_acceptance -- <out dir> [third-party .skp …]
//! ```
//!
//! Cases (both containers unless noted):
//! - `hew-native`, `hew-native-ann`, `hew-zoo`: documents drawn in Hew
//!   (`tests/fixtures/*.hew`), the last with a texture and hidden nodes;
//! - `fx-<stem>`: every `skp-import` fixture opened and saved back
//!   unchanged (its own container);
//! - `ex-<stem>`: rich fixtures written whole, without their source;
//! - `ed-<stem>`: rich fixtures (and any third-party files given) opened,
//!   edited — a solid moved, one deleted, a box added, a material made
//!   translucent, a tag hidden — and saved back;
//! - `ld-layers`: a loose box moved, so the loose geometry is rewritten
//!   (each box on its own layer);
//! - `dg-zoo`: a component definition holding a guide point, saved three
//!   times over;
//! - `un-<units>`: the zoo written in each of Hew's display units.

use std::path::{Path, PathBuf};

use kernel::{Document, NodeId};
use skp_export::{SkpSource, Target, WriteEnv, export, save};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(
        args.next()
            .expect("usage: skp_acceptance <out dir> [file.skp …]"),
    );
    std::fs::create_dir_all(&out).unwrap();
    let third_party: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("../skp-import/tests/fixtures");
    let env = WriteEnv {
        unix_time: 1_800_000_000,
        seed: 11,
    };
    let both = [
        (Target::Stream(skpwright::Release::R2017), "17"),
        (Target::Skp2026, "26"),
    ];
    let mut n = 0;

    // Documents drawn in Hew.
    let native = load(&root.join("tests/fixtures/native.hew"));
    let annotated = annotate(native.clone());
    let zoo = dress_zoo(load(&root.join("tests/fixtures/zoo.hew")), &fixtures);
    for (name, doc) in [
        ("hew-native", &native),
        ("hew-native-ann", &annotated),
        ("hew-zoo", &zoo),
    ] {
        for (t, c) in both {
            let bytes = export(doc, t, env).unwrap().bytes;
            write_case(&out, &format!("{name}-{c}"), &bytes, doc);
            n += 1;
        }
    }

    // The zoo written in each of Hew's display units.
    for name in ["m", "cm", "mm", "arch", "frac_in", "dec_in"] {
        let units = skp_export::DisplayUnits::from_name(name);
        for (t, c) in both {
            let bytes = skp_export::export_in(&zoo, t, env, units).unwrap().bytes;
            write_case(&out, &format!("un-{name}-{c}"), &bytes, &zoo);
            n += 1;
        }
    }

    // Every fixture saved back unchanged, and rich ones written whole.
    let rich = [
        "house",
        "feature-pack",
        "nested-3-deep",
        "two-scenes",
        "material-one-face",
    ];
    for dir in [fixtures.clone(), fixtures.join("2026")] {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "skp"))
            .collect();
        files.sort();
        for f in files {
            let bytes = std::fs::read(&f).unwrap();
            let stem = f.file_stem().unwrap().to_string_lossy().to_string();
            let Some((doc, src)) = open(&bytes) else {
                continue;
            };
            let (t, c) = container(&bytes);
            let saved = save(&doc, t, env, Some(&src)).unwrap();
            write_case(&out, &format!("fx-{stem}-{c}"), &saved.export.bytes, &doc);
            n += 1;
            if c == "17" && rich.contains(&stem.as_str()) {
                for (t, c) in both {
                    let bytes = export(&doc, t, env).unwrap().bytes;
                    write_case(&out, &format!("ex-{stem}-{c}"), &bytes, &doc);
                    n += 1;
                }
            }
            if rich[..2].contains(&stem.as_str()) {
                let (mut doc, src) = open(&bytes).unwrap();
                edit(&mut doc);
                let saved = save(&doc, t, env, Some(&src)).unwrap();
                write_case(&out, &format!("ed-{stem}-{c}"), &saved.export.bytes, &doc);
                n += 1;
            }
        }
    }

    // Loose geometry rewritten: a loose box moved (each box on its own
    // layer, which its faces carry).
    for (stem, file) in [
        ("layers", fixtures.join("layers.skp")),
        ("layers", fixtures.join("2026/layers.skp")),
    ] {
        let bytes = std::fs::read(&file).unwrap();
        let (mut doc, src) = open(&bytes).unwrap();
        let (t, c) = container(&bytes);
        let first = doc.top_level_nodes().into_iter().find_map(|n| match n {
            NodeId::Object(o) => Some(o),
            _ => None,
        });
        if let Some(o) = first {
            doc.transform_object(
                o,
                &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 0.5)),
            )
            .unwrap();
        }
        let saved = save(&doc, t, env, Some(&src)).unwrap();
        write_case(&out, &format!("ld-{stem}-{c}"), &saved.export.bytes, &doc);
        n += 1;
    }

    // A component definition holding a guide point, saved three times.
    for (t, c) in both {
        let mut model = skp_export::author(&zoo).model;
        let tree = model.scene();
        let mut placed = std::collections::BTreeSet::new();
        let mut stack: Vec<&skpwright::Node> = tree.iter().collect();
        while let Some(nd) = stack.pop() {
            if let Some(di) = nd.run.and_then(|ri| model.geometry[ri].def_index) {
                placed.insert(di);
            }
            stack.extend(nd.children.iter());
        }
        drop(stack);
        drop(tree);
        let run = model
            .geometry
            .iter()
            .position(|r| r.def_index.is_some_and(|d| placed.contains(&d)))
            .unwrap();
        model.geometry[run]
            .guide_points
            .push(skpwright::GuidePoint {
                pid: 900_001,
                point_m: [0.01, 0.02, 0.03],
                anchor_m: [0.01, 0.02, 0.03],
                ..Default::default()
            });
        let bytes = match t {
            Target::Skp2026 => skpwright::Document::from_model(&model)
                .unwrap()
                .write()
                .unwrap(),
            stream => skpwright::StreamDocument::from_model(&model, stream.release().unwrap())
                .unwrap()
                .write(),
        };
        let (doc, mut src) = open(&bytes).unwrap();
        let mut last = Vec::new();
        for _ in 0..3 {
            let saved = save(&doc, t, env, Some(&src)).unwrap();
            last = saved.export.bytes;
            src = saved.source;
        }
        write_case(&out, &format!("dg-zoo-{c}"), &last, &doc);
        n += 1;
    }

    // Third-party files, edited and saved back.
    for f in third_party {
        let bytes = std::fs::read(&f).unwrap();
        let Some((mut doc, src)) = open(&bytes) else {
            eprintln!("{}: does not import", f.display());
            continue;
        };
        let (t, c) = container(&bytes);
        let stem: String = f
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        edit(&mut doc);
        let saved = save(&doc, t, env, Some(&src)).unwrap();
        write_case(&out, &format!("ed-{stem}-{c}"), &saved.export.bytes, &doc);
        n += 1;
    }
    println!("{n} cases in {}", out.display());
}

fn load(p: &Path) -> Document {
    Document::load(&std::fs::read(p).unwrap()).unwrap()
}

fn open(bytes: &[u8]) -> Option<(Document, SkpSource)> {
    let out = skp_import::import(bytes).ok()?;
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).ok()?;
    doc.forget_history().ok()?;
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    Some((doc, src))
}

fn container(bytes: &[u8]) -> (Target, String) {
    let target = skp_export::target_of(bytes);
    let label = match target {
        Target::Skp2026 => "26".to_string(),
        Target::Skp2026Compat => "26c".to_string(),
        Target::Stream(r) => r.number().to_string(),
    };
    (target, label)
}

fn write_case(out: &Path, name: &str, bytes: &[u8], doc: &Document) {
    std::fs::write(out.join(format!("{name}.skp")), bytes).unwrap();
    std::fs::write(
        out.join(format!("{name}.expect.hew")),
        doc.save_for_persistence(),
    )
    .unwrap();
}

/// A linear dimension, a leader text, and radius dimensions on a drawn and
/// on an undrawn circle.
fn annotate(mut doc: Document) -> Document {
    use kernel::{Anchor, CapturedCurve, Plane, Point3, RadialKind, Vec3};
    let free = |p: Point3| Anchor {
        node: None,
        point: p,
    };
    let flat = |z: f64| {
        Plane::from_point_normal(Point3::new(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).unwrap()
    };
    doc.add_linear_dimension(
        free(Point3::new(0.0, 0.0, 0.0)),
        free(Point3::new(1.0, 0.0, 0.0)),
        Vec3::new(0.0, -0.2, 0.0),
        flat(0.0),
        None,
    )
    .unwrap();
    doc.add_leader_text(
        free(Point3::new(2.0, 0.3, 0.8)),
        Vec3::new(0.3, 0.0, 0.3),
        "Glass top".into(),
    )
    .unwrap();
    for (c, r, kind) in [
        (Point3::new(2.0, 0.3, 0.8), 0.25, RadialKind::Diameter),
        (Point3::new(9.0, 9.0, 0.0), 0.5, RadialKind::Radius),
    ] {
        doc.add_radial_dimension(
            free(Point3::new(c.x + r, c.y, c.z)),
            kind,
            CapturedCurve {
                center: c,
                radius: r,
                plane: flat(c.z),
            },
            Vec3::new(1.0, 0.0, 0.0),
            None,
        )
        .unwrap();
    }
    doc
}

/// The zoo with what its drawing script cannot set: a texture (house.skp's
/// first) painted on the slab's top, and a hidden instance.
fn dress_zoo(mut doc: Document, fixtures: &Path) -> Document {
    let house = open(&std::fs::read(fixtures.join("house.skp")).unwrap())
        .unwrap()
        .0;
    let tex = house
        .materials()
        .iter()
        .find(|(_, m)| m.texture.is_some())
        .map(|(_, m)| m.clone())
        .unwrap();
    let mid = doc.add_material(tex);
    let slab = doc
        .visible_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some("Pocketed Slab"))
        .unwrap();
    let top = doc
        .object(slab)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.99 && f.inner_loops.is_empty().not())
        .or_else(|| {
            doc.object(slab)
                .unwrap()
                .faces()
                .iter()
                .find(|(_, f)| f.plane.normal().z > 0.99)
        })
        .map(|(id, _)| id)
        .unwrap();
    doc.paint_face(slab, top, Some(mid)).unwrap();
    if let Some(i) = doc.instance_ids().into_iter().next() {
        doc.set_node_user_hidden(NodeId::Instance(i), true);
    }
    doc
}

trait Not {
    fn not(self) -> bool;
}
impl Not for bool {
    fn not(self) -> bool {
        !self
    }
}

/// The edits a person makes: move a solid up, delete another, draw a new
/// box, make a material translucent, hide a tag.
fn edit(doc: &mut Document) {
    use kernel::{Plane, Point3, Transform, Vec3};
    let solids: Vec<kernel::ObjectId> = doc
        .top_level_nodes()
        .into_iter()
        .filter_map(|n| match n {
            NodeId::Object(o) if doc.object_name(o) != Some("Model") => Some(o),
            _ => None,
        })
        .collect();
    if let Some(&o) = solids.first() {
        doc.transform_object(o, &Transform::translation(Vec3::new(0.0, 0.0, 0.5)))
            .unwrap();
    }
    if let Some(&o) = solids.get(1) {
        doc.delete_node(NodeId::Object(o)).unwrap();
    }
    let s =
        doc.add_sketch(Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
        let ring = [
            p(-30.0, -30.0),
            p(-29.0, -30.0),
            p(-29.0, -29.0),
            p(-30.0, -29.0),
        ];
        for k in 0..4 {
            sk.add_segment(ring[k], ring[(k + 1) % 4]).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap();
    let solid = doc
        .materials()
        .iter()
        .find(|(_, m)| m.texture.is_none() && m.color.a == 255)
        .map(|(id, _)| id);
    if let Some(mid) = solid {
        doc.set_material_alpha(mid, 128).unwrap();
    }
    let first_tag = doc.tag_meta().map(|(p, _)| p.to_vec()).next();
    if let Some(path) = first_tag {
        doc.set_tag_hidden(path, true);
    }
}
