//! The imported mark through a `.skp` Hew writes: a foreign file opened in
//! Hew, edited and saved as a `.skp`, reopens with exactly the solids and
//! sketches that were still the foreign file's marked imported — whether
//! the file was written from the document alone or saved back over its
//! source — and drawing made in Hew (or a foreign solid or sketch the user
//! drew on) unmarked. A `.skp` an earlier Hew wrote, without the record,
//! reopens with nothing marked, as before.

use std::path::Path;

use kernel::{
    DefRecipe, Document, ImportNode, ImportScene, ImportSceneExtra, ImportSketch, MeshRecipe,
    NodeId, ObjectId, Plane, Point3, SketchId, Transform, Vec3,
};
use skp_export::{SkpSource, WriteEnv, export, save};
use skpwright::{Owner, Value};

mod common;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn env() -> WriteEnv {
    WriteEnv {
        unix_time: 1_800_000_000,
        seed: 11,
    }
}

fn ground() -> Plane {
    Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0., 0., 1.)).unwrap()
}

/// A unit box with its corner at `x0` on the ground.
fn box_at(name: &str, x0: f64) -> MeshRecipe {
    let (a, b) = (p(x0, 0., 0.), p(x0 + 1., 1., 1.));
    MeshRecipe {
        name: name.to_string(),
        positions: vec![
            p(a.x, a.y, a.z),
            p(b.x, a.y, a.z),
            p(b.x, b.y, a.z),
            p(a.x, b.y, a.z),
            p(a.x, a.y, b.z),
            p(b.x, a.y, b.z),
            p(b.x, b.y, b.z),
            p(a.x, b.y, b.z),
        ],
        faces: vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
        face_materials: vec![kernel::NO_MATERIAL; 6],
        face_uv_frames: vec![None; 6],
        face_holes: vec![Vec::new(); 6],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

fn line_at(y: f64) -> ImportSketch {
    ImportSketch {
        plane: ground(),
        def: None,
        group: None,
        segments: vec![[p(20., y, 0.), p(21., y, 0.)]],
        curves: Vec::new(),
        filled: Vec::new(),
        tags: Vec::new(),
        imported: false,
    }
}

fn draw(doc: &mut Document, on: Plane, segs: &[(Point3, Point3)]) {
    let objects = doc.drawable_objects_on_plane(on, None);
    let segs = segs.to_vec();
    doc.draft(on, None, &objects, "Line", move |w| {
        for &(a, b) in &segs {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .expect("the draft lowers");
}

fn top_of(x0: f64) -> Plane {
    Plane::from_point_normal(p(x0, 0., 1.), Vec3::new(0., 0., 1.)).unwrap()
}

/// A foreign import, then edits: "Kept" and the component member "Part"
/// untouched, "Drawn" drawn on, the line at y = 0 untouched, the line at
/// y = 2 drawn into, and a line drawn in Hew at y = 4.
fn edited_foreign_doc() -> Document {
    let mut doc = Document::new();
    doc.ingest(
        ImportScene {
            materials: Vec::new(),
            defs: vec![DefRecipe::from_meshes(
                Some("Part".into()),
                vec![box_at("Part", 0.)],
            )],
            roots: vec![
                ImportNode::Mesh(box_at("Kept", 0.)),
                ImportNode::Mesh(box_at("Drawn", 5.)),
                ImportNode::Instance {
                    def: 0,
                    pose: Transform::translation(Vec3::new(10., 0., 0.)),
                    name: None,
                    tags: Vec::new(),
                    hidden: false,
                    key: None,
                },
            ],
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            extra: ImportSceneExtra {
                sketches: vec![line_at(0.), line_at(2.)],
                ..Default::default()
            },
            geo: None,
        },
        Vec::new(),
    )
    .unwrap();
    // A rectangle on "Drawn"'s top face: it is the user's now.
    draw(
        &mut doc,
        top_of(5.),
        &[
            (p(5.2, 0.2, 1.), p(5.4, 0.2, 1.)),
            (p(5.4, 0.2, 1.), p(5.4, 0.4, 1.)),
            (p(5.4, 0.4, 1.), p(5.2, 0.4, 1.)),
            (p(5.2, 0.4, 1.), p(5.2, 0.2, 1.)),
        ],
    );
    // A line from the y = 2 line's end: that sketch is the user's now.
    draw(&mut doc, ground(), &[(p(21., 2., 0.), p(21., 3., 0.))]);
    // Drawing made in Hew, clear of the file's lines.
    draw(&mut doc, ground(), &[(p(30., 4., 0.), p(31., 4., 0.))]);
    doc
}

fn object_named(doc: &Document, name: &str) -> ObjectId {
    doc.visible_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some(name))
        .unwrap_or_else(|| panic!("an object named {name}"))
}

/// The component's one member Object.
fn member(doc: &Document) -> ObjectId {
    let defs = doc.component_ids();
    assert_eq!(defs.len(), 1, "one component");
    match doc.def_member_nodes(defs[0]).unwrap().as_slice() {
        [NodeId::Object(o)] => *o,
        other => panic!("one member object, got {other:?}"),
    }
}

/// The world sketch holding a vertex at `at`.
fn sketch_at(doc: &Document, at: Point3) -> SketchId {
    doc.sketch_ids()
        .into_iter()
        .find(|&s| {
            doc.sketch(s)
                .unwrap()
                .vertices()
                .values()
                .any(|v| v.position.approx_eq(at, 1e-6))
        })
        .unwrap_or_else(|| panic!("a sketch at {at:?}"))
}

/// Every mark the edited document holds, as `(what, marked)`.
fn marks(doc: &Document) -> Vec<(&'static str, bool)> {
    vec![
        ("Kept", doc.object_is_imported(object_named(doc, "Kept"))),
        ("Drawn", doc.object_is_imported(object_named(doc, "Drawn"))),
        ("Part", doc.object_is_imported(member(doc))),
        (
            "line y=0",
            doc.sketch_is_imported(sketch_at(doc, p(20., 0., 0.))),
        ),
        (
            "line y=2",
            doc.sketch_is_imported(sketch_at(doc, p(20., 2., 0.))),
        ),
        (
            "line y=4",
            doc.sketch_is_imported(sketch_at(doc, p(30., 4., 0.))),
        ),
    ]
}

const EXPECTED: [(&str, bool); 6] = [
    ("Kept", true),
    ("Drawn", false),
    ("Part", true),
    ("line y=0", true),
    ("line y=2", false),
    ("line y=4", false),
];

fn reopen(bytes: &[u8]) -> Document {
    let back = skp_import::import(bytes).unwrap();
    let mut doc = Document::new();
    doc.ingest(back.scene, back.textures_missing).unwrap();
    doc
}

#[test]
fn the_edited_document_holds_the_expected_marks() {
    assert_eq!(marks(&edited_foreign_doc()), EXPECTED);
}

/// Written from the document alone, in every container: exactly what was
/// still the foreign file's comes back marked.
#[test]
fn a_written_file_reopens_with_the_same_marks() {
    let doc = edited_foreign_doc();
    for target in common::all_targets() {
        let out = export(&doc, target, env()).unwrap();
        assert_eq!(marks(&reopen(&out.bytes)), EXPECTED, "{target:?}");
    }
}

/// A file Hew wrote before it recorded the marks (no `imported_marks`):
/// it reopens with nothing marked, as it always did.
#[test]
fn a_file_an_earlier_hew_wrote_reopens_with_nothing_marked() {
    let doc = edited_foreign_doc();
    let mut authored = skp_export::author(&doc);
    for a in &mut authored.model.dictionaries {
        if a.owner == Owner::Model {
            for d in &mut a.dictionaries {
                if d.name == "hew" {
                    d.entries.retain(|(k, _)| k != "imported_marks");
                    assert!(d.entries.iter().any(|(k, _)| k == "imported_sketches"));
                }
            }
        }
    }
    let files = [
        (
            "2026",
            skpwright::Document::from_model_with(&authored.model, env())
                .unwrap()
                .write()
                .unwrap(),
        ),
        (
            "2017",
            skpwright::StreamDocument::from_model_with(
                &authored.model,
                skpwright::Release::R2017,
                env(),
            )
            .unwrap()
            .write(),
        ),
    ];
    for (label, bytes) in files {
        let back = reopen(&bytes);
        assert!(
            marks(&back).iter().all(|&(_, m)| !m),
            "{label}: {:?}",
            marks(&back)
        );
    }
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../skp-import/tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Open `bytes` as the app does.
fn open(bytes: &[u8]) -> (Document, SkpSource) {
    let out = skp_import::import(bytes).unwrap();
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    doc.forget_history().unwrap();
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    (doc, src)
}

/// Saved back over a SketchUp file (its own records carried where nothing
/// changed): the file's solid comes back marked, a line drawn in Hew
/// unmarked; drawn on before the save, the solid comes back unmarked.
/// Both containers.
#[test]
fn a_save_back_over_a_sketchup_file_keeps_the_marks() {
    for name in ["box.skp", "2026/box.skp"] {
        let bytes = fixture(name);
        let target = skp_export::target_of(&bytes);

        let (mut doc, src) = open(&bytes);
        let solid = doc.visible_object_ids()[0];
        assert!(doc.object_is_imported(solid), "{name}");
        draw(&mut doc, ground(), &[(p(30., 0., 0.), p(31., 0., 0.))]);
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let back = reopen(&saved.export.bytes);
        let objects = back.visible_object_ids();
        assert_eq!(objects.len(), 1, "{name}");
        assert!(
            back.object_is_imported(objects[0]),
            "{name}: still the file's"
        );
        assert!(
            !back.sketch_is_imported(sketch_at(&back, p(30., 0., 0.))),
            "{name}: the line is Hew's"
        );

        let (mut doc, src) = open(&bytes);
        let solid = doc.visible_object_ids()[0];
        let top = doc
            .object(solid)
            .unwrap()
            .faces()
            .values()
            .map(|f| f.plane)
            .max_by(|a, b| {
                let h = |pl: &Plane| -pl.signed_distance(Point3::ORIGIN) * pl.normal().z;
                h(a).total_cmp(&h(b))
            })
            .unwrap();
        let c = doc
            .object(solid)
            .unwrap()
            .vertices()
            .values()
            .map(|v| v.position)
            .filter(|q| top.signed_distance(*q).abs() < 1e-9)
            .fold(Point3::ORIGIN, |acc, q| acc + (q - Point3::ORIGIN) * 0.25);
        let d = 0.01;
        draw(
            &mut doc,
            top,
            &[
                (p(c.x - d, c.y - d, c.z), p(c.x + d, c.y - d, c.z)),
                (p(c.x + d, c.y - d, c.z), p(c.x + d, c.y + d, c.z)),
                (p(c.x + d, c.y + d, c.z), p(c.x - d, c.y + d, c.z)),
                (p(c.x - d, c.y + d, c.z), p(c.x - d, c.y - d, c.z)),
            ],
        );
        assert!(!doc.object_is_imported(solid), "{name}: drawn on");
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let back = reopen(&saved.export.bytes);
        assert!(
            back.visible_object_ids()
                .iter()
                .all(|&o| !back.object_is_imported(o)),
            "{name}: the drawn-on solid is the user's"
        );
    }
}

/// The record a written file carries, for a reader to check.
#[test]
fn the_written_file_records_the_marks_in_the_hew_dictionary() {
    let authored = skp_export::author(&edited_foreign_doc());
    let model_hew = authored
        .model
        .dictionaries
        .iter()
        .filter(|a| a.owner == Owner::Model)
        .flat_map(|a| &a.dictionaries)
        .find(|d| d.name == "hew")
        .unwrap();
    let entry = |k: &str| {
        model_hew
            .entries
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
    };
    assert_eq!(entry("imported_marks"), Some(&Value::Bool(true)));
    assert!(matches!(entry("imported_sketches"), Some(Value::Array(v)) if v.len() == 1));
}
