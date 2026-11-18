//! Executable specs for the imported mark: provenance that tells a viewer
//! which drawing came in from a foreign file and which was made in Hew.
//!
//! The contract under test:
//! - `Document::ingest` marks every object and sketch it builds as
//!   imported, unless the import says it is Hew's own work
//!   (`ImportSceneExtra::native`: a `.skp` Hew wrote, a placed picture);
//! - the marks survive save and load (manifest v32 `objects[].imported`,
//!   `sketches[].imported`); a manifest declaring an older version that
//!   carries one is refused, and an older file loads with nothing marked;
//! - the first drawing on an imported object clears its mark, and drawing
//!   into an imported sketch clears the sketch's, each inside the draft's
//!   own undo step (undo marks it again, redo clears it);
//! - erasing, moving and copying clear nothing; a copy keeps the mark;
//! - drawing on an imported sketch's plane whose extent lies entirely
//!   clear of it goes to a sketch of its own, leaving the imported one
//!   marked; drawing whose extent meets it (around it, inside it, or
//!   touching it) joins it as before, so shapes still nest, and clears it;
//! - a gesture that only removes edges keeps the mark, as the eraser does;
//! - any drawing on an imported object clears its mark, a dangling line
//!   kept as the object's own sketch included.

use kernel::{
    Document, DraftReport, ImportNode, ImportScene, ImportSceneExtra, ImportSketch, MeshRecipe,
    NodeId, ObjectId, Plane, Point3, SketchId, Transform, Vec3,
};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn ground() -> Plane {
    Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0., 0., 1.)).unwrap()
}

fn top() -> Plane {
    Plane::from_point_normal(p(0., 0., 1.), Vec3::new(0., 0., 1.)).unwrap()
}

/// A unit box whose top face is split in two by the line x = 0.5 — the
/// shape a SketchUp model has where a hard edge divides coplanar faces.
fn split_top_box() -> MeshRecipe {
    MeshRecipe {
        name: "Box".to_string(),
        positions: vec![
            p(0., 0., 0.),
            p(1., 0., 0.),
            p(1., 1., 0.),
            p(0., 1., 0.),
            p(0., 0., 1.),
            p(1., 0., 1.),
            p(1., 1., 1.),
            p(0., 1., 1.),
            p(0.5, 0., 1.),
            p(0.5, 1., 1.),
        ],
        faces: vec![
            vec![0, 3, 2, 1],    // bottom
            vec![4, 8, 9, 7],    // top, left half
            vec![8, 5, 6, 9],    // top, right half
            vec![0, 1, 5, 8, 4], // front
            vec![1, 2, 6, 5],    // right
            vec![2, 3, 7, 9, 6], // back
            vec![3, 0, 4, 7],    // left
        ],
        face_materials: vec![kernel::NO_MATERIAL; 7],
        face_uv_frames: vec![None; 7],
        face_holes: vec![Vec::new(); 7],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

/// An open L of two loose lines on the ground, away from the box.
fn loose_lines() -> ImportSketch {
    ImportSketch {
        plane: ground(),
        def: None,
        group: None,
        segments: vec![
            [p(3., 0., 0.), p(4., 0., 0.)],
            [p(4., 0., 0.), p(4., 1., 0.)],
        ],
        curves: Vec::new(),
        filled: Vec::new(),
        tags: Vec::new(),
        imported: false,
    }
}

fn scene(native: bool) -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(split_top_box())],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: ImportSceneExtra {
            sketches: vec![loose_lines()],
            native,
            ..Default::default()
        },
        geo: None,
    }
}

/// A document holding one foreign import: the box and the loose lines.
fn imported_doc() -> (Document, ObjectId, SketchId) {
    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene(false), Vec::new()).unwrap();
    let o = doc.visible_object_ids()[0];
    let s = report.sketches[0].expect("the lines imported");
    (doc, o, s)
}

fn draw(doc: &mut Document, on: Plane, segs: &[(Point3, Point3)]) -> DraftReport {
    let objects = doc.drawable_objects_on_plane(on, None);
    let segs = segs.to_vec();
    doc.draft(on, None, &objects, "Line", move |w| {
        for &(a, b) in &segs {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .expect("the draft lowers")
    .0
}

fn erase(doc: &mut Document, on: Plane, a: Point3, b: Point3) {
    let objects = doc.drawable_objects_on_plane(on, None);
    doc.draft(on, None, &objects, "Erase", move |w| {
        let e = w.edge_at_positions(a, b).expect("the edge is drawn");
        w.remove_edge(e).map(|_| ())
    })
    .expect("the erase lowers");
}

fn ring(pts: &[Point3]) -> Vec<(Point3, Point3)> {
    (0..pts.len())
        .map(|i| (pts[i], pts[(i + 1) % pts.len()]))
        .collect()
}

fn manifest(bytes: &[u8]) -> serde_json::Value {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// `bytes` with its manifest replaced by `patch` of it.
fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut m = manifest(bytes);
    patch(&mut m);
    let text = serde_json::to_vec_pretty(&m).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..src.len() {
        let mut entry = src.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        if name == "manifest.json" {
            buf = text.clone();
        }
        out.start_file(name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

// ───────────────────────────────────────────────────────────── marking ──

#[test]
fn a_foreign_import_marks_its_objects_and_sketches() {
    let (doc, o, s) = imported_doc();
    assert!(doc.object_is_imported(o));
    assert!(doc.sketch_is_imported(s));
    assert!(doc.sketch(s).unwrap().imported());
    // The split the source drew is what a viewer would otherwise take for
    // a shape drawn in Hew.
    assert!(!doc.object_drawing_planes(o).is_empty());
}

#[test]
fn hew_s_own_import_marks_nothing() {
    let mut doc = Document::new();
    let (report, _) = doc.ingest(scene(true), Vec::new()).unwrap();
    let o = doc.visible_object_ids()[0];
    let s = report.sketches[0].unwrap();
    assert!(!doc.object_is_imported(o));
    assert!(!doc.sketch_is_imported(s));
}

#[test]
fn drawing_made_in_hew_is_not_marked() {
    let mut doc = Document::new();
    let r = draw(&mut doc, ground(), &[(p(0., 0., 0.), p(1., 0., 0.))]);
    assert!(!doc.sketch_is_imported(r.sketches[0]));
}

#[test]
fn stale_handles_are_not_imported() {
    let (mut doc, o, _) = imported_doc();
    doc.undo().unwrap(); // the import
    assert!(!doc.object_is_imported(o), "a hidden object answers false");
    doc.redo().unwrap();
    assert!(
        doc.object_is_imported(o),
        "the mark rides through undo/redo"
    );
}

// ──────────────────────────────────────────────────────── save and load ──

#[test]
fn the_marks_survive_save_and_load() {
    let (doc, _, _) = imported_doc();
    let bytes = doc.save();
    let m = manifest(&bytes);
    assert_eq!(m["format_version"], 33);
    assert_eq!(m["objects"][0]["imported"], true);
    assert_eq!(m["sketches"][0]["imported"], true);

    let loaded = Document::load(&bytes).expect("loads");
    let o = loaded.visible_object_ids()[0];
    let s = loaded.sketch_ids()[0];
    assert!(loaded.object_is_imported(o));
    assert!(loaded.sketch_is_imported(s));
    assert_eq!(loaded.save(), bytes, "a load saves back identically");
}

#[test]
fn nothing_imported_writes_no_key() {
    let mut doc = Document::new();
    doc.ingest(scene(true), Vec::new()).unwrap();
    let m = manifest(&doc.save());
    assert!(m["objects"][0].get("imported").is_none());
    assert!(m["sketches"][0].get("imported").is_none());
}

#[test]
fn an_older_file_loads_with_nothing_marked() {
    let mut doc = Document::new();
    doc.ingest(scene(true), Vec::new()).unwrap();
    let v31 = patch_manifest(&doc.save(), |m| m["format_version"] = serde_json::json!(31));
    let loaded = Document::load(&v31).expect("a v31 file loads");
    assert!(!loaded.object_is_imported(loaded.visible_object_ids()[0]));
    assert!(!loaded.sketch_is_imported(loaded.sketch_ids()[0]));
}

#[test]
fn the_marks_are_refused_in_an_older_manifest() {
    let (doc, _, _) = imported_doc();
    let bytes = doc.save();
    let only_object = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(31);
        m["sketches"][0].as_object_mut().unwrap().remove("imported");
    });
    assert!(
        Document::load(&only_object).is_err(),
        "objects[].imported at v31"
    );
    let only_sketch = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(31);
        m["objects"][0].as_object_mut().unwrap().remove("imported");
    });
    assert!(
        Document::load(&only_sketch).is_err(),
        "sketches[].imported at v31"
    );
}

// ───────────────────────────────────────────────────────────── clearing ──

#[test]
fn drawing_on_an_imported_object_clears_its_mark_in_the_same_step() {
    let (mut doc, o, _) = imported_doc();
    let report = draw(
        &mut doc,
        top(),
        &ring(&[
            p(0.1, 0.1, 1.),
            p(0.3, 0.1, 1.),
            p(0.3, 0.3, 1.),
            p(0.1, 0.3, 1.),
        ]),
    );
    assert!(
        report.objects.contains(&o),
        "the rectangle imprinted the box"
    );
    assert!(!doc.object_is_imported(o), "drawing made it the user's");

    doc.undo().unwrap();
    assert!(doc.object_is_imported(o), "one undo restores the mark");
    doc.redo().unwrap();
    assert!(!doc.object_is_imported(o), "redo clears it again");
}

#[test]
fn erasing_on_an_imported_object_keeps_its_mark() {
    let (mut doc, o, _) = imported_doc();
    erase(&mut doc, top(), p(0.5, 0., 1.), p(0.5, 1., 1.));
    assert!(
        doc.object_drawing_planes(o).is_empty(),
        "the source's split is gone"
    );
    assert!(doc.object_is_imported(o), "erasing drew nothing");
}

#[test]
fn moving_and_copying_keep_the_marks() {
    let (mut doc, o, s) = imported_doc();
    let t = Transform::translation(Vec3::new(5., 0., 0.));
    doc.transform_object(o, &t).unwrap();
    doc.transform_sketch(s, &t).unwrap();
    assert!(doc.object_is_imported(o));
    assert!(doc.sketch_is_imported(s));

    let (copy, _) = doc
        .duplicate_node(NodeId::Object(o), &t, kernel::LinkPolicy::Independent)
        .unwrap();
    let NodeId::Object(copy) = copy else {
        panic!("a copied object is an object")
    };
    assert!(doc.object_is_imported(copy), "a copy keeps the mark");
}

#[test]
fn drawing_into_an_imported_sketch_clears_its_mark_in_the_same_step() {
    let (mut doc, _, s) = imported_doc();
    // A line from the L's free end: it joins the sketch.
    draw(&mut doc, ground(), &[(p(4., 1., 0.), p(3., 1., 0.))]);
    assert!(!doc.sketch_is_imported(s), "the sketch is the user's now");

    doc.undo().unwrap();
    assert!(doc.sketch_is_imported(s), "one undo restores the mark");
    doc.redo().unwrap();
    assert!(!doc.sketch_is_imported(s), "redo clears it again");
}

#[test]
fn erasing_from_an_imported_sketch_keeps_its_mark() {
    let (mut doc, _, s) = imported_doc();
    erase(&mut doc, ground(), p(4., 0., 0.), p(4., 1., 0.));
    let sk = doc.sketch(s).expect("one line is left");
    assert_eq!(sk.edges().len(), 1);
    assert!(sk.imported(), "erasing drew nothing");
}

#[test]
fn drawing_beside_an_imported_sketch_leaves_it_marked() {
    let (mut doc, _, s) = imported_doc();
    let r = draw(
        &mut doc,
        ground(),
        &ring(&[p(6., 0., 0.), p(7., 0., 0.), p(7., 1., 0.), p(6., 1., 0.)]),
    );
    assert!(
        doc.sketch_is_imported(s),
        "the file's lines stay the file's"
    );
    assert!(!r.sketches.contains(&s), "the rectangle is drawn elsewhere");
    assert!(
        r.sketches.iter().all(|&t| !doc.sketch_is_imported(t)),
        "the rectangle is drawing made in Hew"
    );
}

#[test]
fn a_gesture_that_changes_an_imported_sketch_clears_its_mark() {
    let (mut doc, _, s) = imported_doc();
    doc.begin_sketch_gesture(s).unwrap();
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(p(4., 1., 0.), p(3., 1., 0.))
        .unwrap();
    doc.end_sketch_gesture(s).unwrap();
    assert!(!doc.sketch_is_imported(s));
    doc.undo().unwrap();
    assert!(doc.sketch_is_imported(s));
}

#[test]
fn a_gesture_that_only_removes_edges_keeps_the_mark() {
    let (mut doc, _, s) = imported_doc();
    doc.begin_sketch_gesture(s).unwrap();
    let sk = doc.sketch_mut(s).unwrap();
    let e = sk.edge_at_positions(p(4., 0., 0.), p(4., 1., 0.)).unwrap();
    sk.remove_edge(e).unwrap();
    doc.end_sketch_gesture(s).unwrap();
    let sk = doc.sketch(s).unwrap();
    assert_eq!(sk.edges().len(), 1, "the gesture removed a line");
    assert!(sk.imported(), "removing drew nothing");
}

#[test]
fn a_dangling_line_on_an_imported_object_clears_its_mark() {
    let (mut doc, o, _) = imported_doc();
    let r = draw(&mut doc, top(), &[(p(0.1, 0.1, 1.), p(0.2, 0.2, 1.))]);
    assert!(
        r.sketches
            .iter()
            .any(|&s| doc.sketch_owner_object(s) == Some(o)),
        "the line is kept as the box's own drawing"
    );
    assert!(
        !doc.object_is_imported(o),
        "drawing on it made it the user's"
    );
    doc.undo().unwrap();
    assert!(doc.object_is_imported(o), "one undo restores the mark");
    doc.redo().unwrap();
    assert!(!doc.object_is_imported(o));
}

/// An imported faceless 12-gon of radius 0.5 about (10, 0) on the ground.
fn imported_ring_doc() -> (Document, SketchId) {
    let n = 12;
    let pts: Vec<Point3> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            p(10. + 0.5 * a.cos(), 0.5 * a.sin(), 0.)
        })
        .collect();
    let mut sc = scene(false);
    sc.roots.clear();
    sc.extra.sketches = vec![ImportSketch {
        plane: ground(),
        def: None,
        group: None,
        segments: (0..pts.len())
            .map(|i| [pts[i], pts[(i + 1) % pts.len()]])
            .collect(),
        curves: Vec::new(),
        filled: Vec::new(),
        tags: Vec::new(),
        imported: false,
    }];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(sc, Vec::new()).unwrap();
    (doc, report.sketches[0].unwrap())
}

#[test]
fn a_rectangle_around_an_imported_shape_nests_with_it_and_clears_its_mark() {
    let (mut doc, s) = imported_ring_doc();
    let r = draw(
        &mut doc,
        ground(),
        &ring(&[
            p(9., -1., 0.),
            p(11., -1., 0.),
            p(11., 1., 0.),
            p(9., 1., 0.),
        ]),
    );
    assert!(
        r.sketches.contains(&s),
        "the rectangle joins the shape's sketch"
    );
    assert!(
        !doc.sketch_is_imported(s),
        "drawing into it made it the user's"
    );
    let sk = doc.sketch(s).unwrap();
    let (rid, _) = sk
        .regions()
        .iter()
        .find(|(_, r)| r.outer.len() == 4)
        .expect("the rectangle closes a region");
    assert_eq!(sk.regions()[rid].holes.len(), 1, "the shape is its hole");
    let (o, _) = doc.extrude_region(s, rid, 1.0).unwrap();
    assert!(
        doc.object(o)
            .unwrap()
            .faces()
            .values()
            .any(|f| !f.inner_loops.is_empty()),
        "extruding it makes a ring, not a filled block"
    );
}

#[test]
fn a_rectangle_touching_an_imported_sketch_s_extent_joins_it() {
    let (mut doc, _, s) = imported_doc();
    // Clear of the L's lines, but its extent touches the L's at x = 3.
    let r = draw(
        &mut doc,
        ground(),
        &ring(&[
            p(2., 0.2, 0.),
            p(3., 0.2, 0.),
            p(3., 0.6, 0.),
            p(2., 0.6, 0.),
        ]),
    );
    assert!(r.sketches.contains(&s), "it joins the imported sketch");
    assert!(!doc.sketch_is_imported(s));
}
