//! Executable specs for the stored blend restoration (true-curves, D-111):
//! a blend records what it replaced; Remove and Resize replay it; a later
//! blend that ran into it is taken off and put back; the record rides
//! through save/load, transforms and booleans; a record that no longer
//! fits refuses; a blend without one restores by derivation.

mod common;

use std::io::{Cursor, Read as _};

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendSpec, BlendStops, BooleanOp, CornerStyle, Document, EdgeId,
    LoadError, Object, ObjectId, Plane, Point3, RadiusHandle, Transform, Vec3,
};

fn spec(edges: Vec<EdgeId>, profile: BlendProfile) -> BlendSpec {
    BlendSpec {
        edges,
        profile,
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

fn round(r: f64, n: u32) -> BlendProfile {
    BlendProfile::Round {
        radius: r,
        segments: n,
    }
}

fn front_top(obj: &Object) -> EdgeId {
    edge_between(obj, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0))
}

fn back_top(obj: &Object) -> EdgeId {
    edge_between(obj, p3(0.0, 1.0, 1.0), p3(1.0, 1.0, 1.0))
}

fn right_front_vertical(obj: &Object) -> EdgeId {
    edge_between(obj, p3(1.0, 0.0, 0.0), p3(1.0, 0.0, 1.0))
}

fn same(a: &Object, b: &Object) -> bool {
    kernel::same_geometry_for_test(a, b, kernel::tol::BLEND_VERIFY)
}

// ------------------------------------------------------------ records

#[test]
fn a_new_blend_carries_a_record_and_replays_it() {
    let obj = unit_box();
    let (f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    assert!(f.has_blend_record(report.blends[0]));
    let (back, edges) = f.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert_eq!(edges.len(), 1);
    assert!(same(&back, &obj));
    assert!(!back.has_blend_record(report.blends[0]));
}

#[test]
fn a_blend_without_a_record_still_restores_by_derivation() {
    // A blend saved before records existed: the derivation is the fallback.
    for s in [
        spec(vec![front_top(&unit_box())], round(0.1, 6)),
        spec(
            vec![front_top(&unit_box())],
            BlendProfile::Chamfer {
                setback_a: 0.1,
                setback_b: 0.05,
            },
        ),
    ] {
        let obj = unit_box();
        let (mut f, report) = obj.blend_edges(&s).unwrap();
        f.drop_blend_records_for_test();
        assert!(!f.has_blend_record(report.blends[0]));
        let fs = f.blend_features();
        assert_eq!(fs.len(), 1);
        assert!(fs[0].intact);
        let (back, _) = f.remove_blend_for_test(report.blends[0]).unwrap();
        assert!(same(&back, &obj));
    }
}

#[test]
fn one_sided_stop_removes_back() {
    // K-1: a fillet stopped at one end only could not be restored by
    // derivation; its record replays.
    let obj = unit_box();
    let mut s = spec(vec![front_top(&obj)], round(0.1, 6));
    s.stops = Some(BlendStops {
        start: 0.0,
        end: 0.2,
    });
    let (f, report) = obj.blend_edges(&s).unwrap();
    let fs = f.blend_features();
    assert_eq!(fs.len(), 1);
    assert!(fs[0].intact, "{fs:?}");
    assert_eq!(fs[0].size, Some(0.1));
    let (back, _) = f.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert!(same(&back, &obj));
    // Resize keeps the stop.
    let (resized, _) = f
        .resize_blend_for_test(report.blends[0], &round(0.15, 6))
        .unwrap();
    let mut t = spec(vec![front_top(&obj)], round(0.15, 6));
    t.stops = s.stops;
    let (fresh, _) = obj.blend_edges(&t).unwrap();
    assert!(same(&resized, &fresh));
}

#[test]
fn mid_edge_radius_point_removes_back_and_reads_its_size() {
    // K-2: a round with an interior radius point reported no size and could
    // not be removed; its record replays and a resize scales every radius.
    let obj = unit_box();
    let mut s = spec(vec![front_top(&obj)], round(0.05, 6));
    s.handles = vec![RadiusHandle {
        at: 0.3,
        radius: 0.03,
    }];
    let (f, report) = obj.blend_edges(&s).unwrap();
    let fs = f.blend_features();
    assert_eq!(fs.len(), 1);
    assert!(fs[0].intact, "{fs:?}");
    assert_eq!(fs[0].size, Some(0.05));
    let (back, _) = f.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert!(same(&back, &obj));
    let (resized, _) = f
        .resize_blend_for_test(report.blends[0], &round(0.1, 6))
        .unwrap();
    let mut t = spec(vec![front_top(&obj)], round(0.1, 6));
    t.handles = vec![RadiusHandle {
        at: 0.3,
        radius: 0.06,
    }];
    let (fresh, _) = obj.blend_edges(&t).unwrap();
    assert!(same(&resized, &fresh));
}

#[test]
fn dome_rim_fillet_removes_back() {
    // The rim fillet on a revolved dome's faceted sphere band could not be
    // restored by derivation (D-87's note).
    let dome = revolved_dome(6, 24);
    let rim = sharp_edges_at_height(&dome, 0.0);
    let picked = dome.logical_edge(rim[0], false);
    assert_eq!(picked.len(), 24);
    let (f, report) = dome.blend_edges(&spec(picked, round(0.02, 4))).unwrap();
    let fs = f.blend_features();
    assert!(fs.iter().all(|x| x.intact), "{fs:?}");
    let (back, _) = f.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert!(same(&back, &dome));
}

// ------------------------------------------------- later blends in the way

/// A box with its right front vertical edge rounded at 0.2, then its top
/// rim at 0.1 (the rim runs across the vertical round's end): the vertical
/// blend's id, the rim's id, and the result.
fn vertical_then_rim() -> (Object, kernel::BlendId, kernel::BlendId) {
    let obj = unit_box();
    let (v, _) = obj
        .blend_edges(&spec(vec![right_front_vertical(&obj)], round(0.2, 6)))
        .unwrap();
    let rim = sharp_edges_at_height(&v, 1.0);
    let (vr, _) = v.blend_edges(&spec(rim, round(0.1, 4))).unwrap();
    let fs = vr.blend_features();
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert!(fs.iter().all(|f| f.intact), "{fs:?}");
    let vid = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.2).abs() < 1e-9))
        .unwrap()
        .id;
    let rid = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.1).abs() < 1e-9))
        .unwrap()
        .id;
    (vr, vid, rid)
}

#[test]
fn removing_a_fillet_under_a_later_fillet_rebuilds_the_later_one() {
    // K-4: the rim round shortened the vertical blend; the vertical blend
    // is removable all the same, and the rim's corner is rebuilt sharp.
    let (vr, vid, rid) = vertical_then_rim();
    let (back, _) = vr.remove_blend_for_test(vid).unwrap();
    assert_valid_solid(&back);
    let obj = unit_box();
    let (fresh, _) = obj
        .blend_edges(&spec(sharp_edges_at_height(&obj, 1.0), round(0.1, 4)))
        .unwrap();
    assert!(same(&back, &fresh));
    let fs = back.blend_features();
    assert_eq!(fs.len(), 1);
    assert_eq!(fs[0].id, rid, "the rim keeps its id");
    assert!(fs[0].intact);
}

#[test]
fn resizing_a_fillet_under_a_later_fillet_puts_the_later_one_back() {
    let (vr, vid, _) = vertical_then_rim();
    let (resized, report) = vr.resize_blend_for_test(vid, &round(0.25, 6)).unwrap();
    assert_valid_solid(&resized);
    let obj = unit_box();
    let (v, _) = obj
        .blend_edges(&spec(vec![right_front_vertical(&obj)], round(0.25, 6)))
        .unwrap();
    let (fresh, _) = v
        .blend_edges(&spec(sharp_edges_at_height(&v, 1.0), round(0.1, 4)))
        .unwrap();
    assert!(same(&resized, &fresh));
    // The report's new faces are the resized blend's as they stand after
    // the rim is back on (the rim trims its top end).
    assert!(!report.new_faces.is_empty());
    assert!(report.new_faces.iter().all(|&f| {
        resized.faces()[f]
            .blend
            .is_some_and(|t| report.blends.contains(&t.id))
    }));
    assert_eq!(resized.blend_features().len(), 2);
}

#[test]
fn a_later_fillet_that_cannot_be_put_back_refuses_dependent_blend() {
    // Two parallel rounds on the top face, 0.2 each; the first resized to
    // 0.85 leaves no room for the second's rail (at 0.8 from its edge).
    let obj = unit_box();
    let (a, ra) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.2, 4)))
        .unwrap();
    let (ab, _) = a
        .blend_edges(&spec(vec![back_top(&a)], round(0.2, 4)))
        .unwrap();
    assert!(ab.blend_features().iter().all(|f| f.intact));
    let before = ab.to_polygons();
    assert!(matches!(
        ab.resize_blend_for_test(ra.blends[0], &round(0.85, 4)),
        Err(BlendError::DependentBlend)
    ));
    assert_eq!(ab.to_polygons(), before);
    // At 0.3 the second goes back on.
    let (resized, _) = ab
        .resize_blend_for_test(ra.blends[0], &round(0.3, 4))
        .unwrap();
    let (a2, _) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.3, 4)))
        .unwrap();
    let (fresh, _) = a2
        .blend_edges(&spec(vec![back_top(&a2)], round(0.2, 4)))
        .unwrap();
    assert!(same(&resized, &fresh));
    // And removing the first leaves the second alone on a sharp front edge.
    let (back, _) = ab.remove_blend_for_test(ra.blends[0]).unwrap();
    let (only_b, _) = obj
        .blend_edges(&spec(vec![back_top(&obj)], round(0.2, 4)))
        .unwrap();
    assert!(same(&back, &only_b));
}

#[test]
fn a_fresh_blend_never_takes_the_id_of_one_about_to_go_back() {
    // Vertical round (id 1), an unrelated bottom round (id 2), then the top
    // rim, which runs into the vertical round: the re-solve takes the
    // vertical round off, and the rim's fresh id must stay clear of the id
    // the vertical round goes back under. Three features, three ids.
    let obj = unit_box();
    let (a, _) = obj
        .blend_edges(&spec(vec![right_front_vertical(&obj)], round(0.2, 6)))
        .unwrap();
    let bottom_back = edge_between(&a, p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0));
    let (b, _) = a
        .blend_edges(&spec(vec![bottom_back], round(0.1, 4)))
        .unwrap();
    let (c, _) = b
        .blend_edges(&spec(sharp_edges_at_height(&b, 1.0), round(0.1, 4)))
        .unwrap();
    let fs = c.blend_features();
    assert_eq!(fs.len(), 3, "{fs:?}");
    let mut ids: Vec<u32> = fs.iter().map(|f| f.id.0).collect();
    ids.dedup();
    assert_eq!(ids.len(), 3, "{fs:?}");
    assert!(fs.iter().all(|f| f.intact), "{fs:?}");
    // The bottom round is still the 4-facet round it was.
    let bottom = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.1).abs() < 1e-9) && f.faces.len() < 10)
        .expect("the bottom round is its own feature");
    assert_eq!(bottom.faces.len(), 4);
    // And a resize keeps the resized blend's id.
    let vid = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.2).abs() < 1e-9))
        .unwrap()
        .id;
    let (d, report) = c.resize_blend_for_test(vid, &round(0.25, 6)).unwrap();
    assert_eq!(report.blends, vec![vid]);
    let fs = d.blend_features();
    assert_eq!(fs.len(), 3, "{fs:?}");
    assert!(fs.iter().all(|f| f.intact), "{fs:?}");
}

#[test]
fn the_order_follows_application_through_a_resize() {
    // Vertical round (first), the top rim that runs into it (second), an
    // unrelated bottom round (third). Resizing the vertical round takes the
    // rim off and puts it back on top of the new round: in the file the
    // ids are kept, the rim still comes after the vertical round, and
    // every blend stays editable.
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let v = right_front_vertical(doc.object(o).unwrap());
    doc.blend_edges(o, &spec(vec![v], round(0.2, 6))).unwrap();
    let rim = sharp_edges_at_height(doc.object(o).unwrap(), 1.0);
    doc.blend_edges(o, &spec(rim, round(0.1, 4))).unwrap();
    let bottom = edge_between(doc.object(o).unwrap(), p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0));
    doc.blend_edges(o, &spec(vec![bottom], round(0.05, 4)))
        .unwrap();
    let seqs = |doc: &Document| -> std::collections::BTreeMap<u64, u64> {
        let m = manifest_of(&doc.save());
        m["objects"][0]["blends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| (b["id"].as_u64().unwrap(), b["seq"].as_u64().unwrap()))
            .collect()
    };
    let before = seqs(&doc);
    assert_eq!(before.len(), 3, "{before:?}");
    let fs = doc.blend_features(o).unwrap();
    let vid = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.2).abs() < 1e-9))
        .unwrap()
        .id;
    let rid = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.1).abs() < 1e-9))
        .unwrap()
        .id;
    doc.resize_blend(o, vid, &round(0.25, 6)).unwrap();
    let after = seqs(&doc);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "ids are kept"
    );
    assert!(
        after[&u64::from(vid.0)] < after[&u64::from(rid.0)],
        "the rim stays after the round it runs into: {after:?}"
    );
    let fs = doc.blend_features(o).unwrap();
    assert_eq!(fs.len(), 3);
    assert!(fs.iter().all(|f| f.intact), "{fs:?}");
}

// ----------------------------------------------- edits elsewhere on a face

#[test]
fn a_pocket_pushed_beside_the_fillet_survives_its_removal() {
    // The record splices the replaced loop into the face as it is now, so
    // a pocket pushed into the top face after the fillet stays.
    let obj = unit_box();
    let (mut f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    let top = face_with_normal(&f, Vec3::new(0.0, 0.0, 1.0));
    let r = f
        .split_face_inner(
            top,
            &[
                p3(0.4, 0.4, 1.0),
                p3(0.6, 0.4, 1.0),
                p3(0.6, 0.6, 1.0),
                p3(0.4, 0.6, 1.0),
            ],
        )
        .unwrap();
    f.push_pull(r.sub_face, -0.1).unwrap();
    let fs = f.blend_features();
    assert_eq!(fs.len(), 1);
    assert!(fs[0].intact, "{fs:?}");
    let (back, _) = f.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert!((signed_volume(&back) - (1.0 - 0.1 * 0.2 * 0.2)).abs() < 1e-9);
    assert_eq!(back.faces().len(), 6 + 5);
    assert!(back.blend_features().is_empty());
}

#[test]
fn a_neighbour_pushed_through_the_fillet_refuses() {
    // The front face pushed out moves the rail with it: no record fits and
    // no round re-blends to it.
    let obj = unit_box();
    let (mut f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.2, 6)))
        .unwrap();
    let front = face_with_normal(&f, Vec3::new(0.0, -1.0, 0.0));
    f.push_pull(front, 0.1).unwrap();
    let before = f.to_polygons();
    assert!(matches!(
        f.remove_blend_for_test(report.blends[0]),
        Err(BlendError::BlendModified)
    ));
    assert_eq!(f.to_polygons(), before);
    assert!(!f.blend_features()[0].intact);
}

// ------------------------------------------------------- transforms

#[test]
fn records_follow_the_object_through_a_move_rotate_and_scale() {
    let obj = unit_box();
    let (f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    let moves = [
        Transform::translation(Vec3::new(2.0, -1.0, 0.5)),
        Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.7).unwrap(),
        Transform::uniform_scale(2.5),
    ];
    let (mut moved, mut plain) = (f.clone(), obj.clone());
    for t in &moves {
        moved.apply_transform(t).unwrap();
        plain.apply_transform(t).unwrap();
    }
    let fs = moved.blend_features();
    assert_eq!(fs.len(), 1);
    assert!(fs[0].intact, "{fs:?}");
    assert!((fs[0].size.unwrap() - 0.25).abs() < 1e-9, "the size scales");
    let (back, _) = moved.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert!(same(&back, &plain));
    // A non-uniform scale drops the tags, and the records with them.
    let mut squashed = f.clone();
    squashed
        .apply_transform(&Transform::scale(Vec3::new(1.0, 1.0, 2.0)))
        .unwrap();
    assert!(squashed.blend_features().is_empty());
    assert!(!squashed.has_blend_record(report.blends[0]));
}

// ------------------------------------------------- documents and files

fn build_box_at(doc: &mut Document, lo: Point3, hi: Point3) -> ObjectId {
    let plane = Plane::from_point_normal(lo, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, lo.z), Point3::new(b.0, b.1, lo.z))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, hi.z - lo.z).unwrap().0
}

fn build_box(doc: &mut Document) -> ObjectId {
    build_box_at(doc, p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 1.0))
}

fn manifest_of(bytes: &[u8]) -> serde_json::Value {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut buf = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn with_patched_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::Write as _;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        entries.push((e.name().unwrap().to_string(), buf));
    }
    let mut manifest = manifest_of(bytes);
    patch(&mut manifest);
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for (name, data) in entries {
        out.start_file(name.as_str(), opts).unwrap();
        if name == "manifest.json" {
            out.write_all(&serde_json::to_vec_pretty(&manifest).unwrap())
                .unwrap();
        } else {
            out.write_all(&data).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn records_ride_through_save_and_load() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let e = front_top(doc.object(o).unwrap());
    let (report, _) = doc.blend_edges(o, &spec(vec![e], round(0.1, 6))).unwrap();
    let bytes = doc.save();
    assert_eq!(bytes, doc.save(), "saving is deterministic");
    let m = manifest_of(&bytes);
    assert_eq!(m["format_version"], 31);
    let blends = m["objects"][0]["blends"].as_array().unwrap();
    assert_eq!(blends.len(), 1);
    assert_eq!(blends[0]["id"], report.blends[0].0);
    assert_eq!(blends[0]["spec"]["profile"]["kind"], "round");
    assert_eq!(blends[0]["spec"]["edges"].as_array().unwrap().len(), 1);
    assert!(!blends[0]["trimmed"].as_array().unwrap().is_empty());
    assert!(!blends[0]["marks"].as_array().unwrap().is_empty());

    let doc2 = Document::load(&bytes).unwrap();
    assert_eq!(doc2.save(), bytes, "a loaded file saves the same bytes");
    let o2 = doc2.visible_object_ids()[0];
    let fs = doc2.blend_features(o2).unwrap();
    assert_eq!(fs.len(), 1);
    assert!(fs[0].intact, "{fs:?}");
    assert!(doc2.object(o2).unwrap().has_blend_record(report.blends[0]));
    let mut doc2 = doc2;
    doc2.remove_blend(o2, report.blends[0]).unwrap();
    let mut plain = Document::new();
    let p = build_box(&mut plain);
    assert!(same(doc2.object(o2).unwrap(), plain.object(p).unwrap()));
}

#[test]
fn an_object_without_records_writes_no_blends_entry() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let face = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, 0.0, 1.0));
    doc.set_face_blend_for_test(
        o,
        face,
        Some(kernel::BlendTag {
            id: kernel::BlendId(1),
            kind: kernel::BlendKind::Fillet,
            role: kernel::BlendRole::Strip,
        }),
    );
    let m = manifest_of(&doc.save());
    assert!(m["objects"][0].get("blends").is_none());
}

#[test]
fn blend_records_are_gated_at_v31_and_checked_for_shape() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let e = front_top(doc.object(o).unwrap());
    doc.blend_edges(o, &spec(vec![e], round(0.1, 6))).unwrap();
    let bytes = doc.save();
    match Document::load(&with_patched_manifest(&bytes, |m| {
        m["format_version"] = 30.into();
    })) {
        Err(LoadError::MalformedManifest { what }) => {
            assert!(what.contains("blend records in a v30 manifest"), "{what}");
        }
        other => panic!("expected MalformedManifest, got {other:?}"),
    }
    let tampers: Vec<fn(&mut serde_json::Value)> = vec![
        |m| {
            let after = &mut m["objects"][0]["blends"][0]["trimmed"][0]["after"];
            *after = serde_json::json!([[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]]);
        },
        |m| m["objects"][0]["blends"][0]["spec"]["edges"] = serde_json::json!([]),
        |m| m["objects"][0]["blends"][0]["seq"] = serde_json::json!(4294967295u32),
        |m| m["objects"][0]["blends"][0]["spec"]["profile"]["segments"] = serde_json::json!(0),
        |m| m["objects"][0]["blends"][0]["spec"]["profile"]["segments"] = serde_json::json!(97),
        |m| {
            m["objects"][0]["blends"][0]["spec"]["primary"] =
                serde_json::json!({ "normal": [0.0, 0.0, 2.0], "offset": 1.0 });
        },
        |m| m["objects"][0]["blends"][0]["spec"]["profile"]["radius"] = (-0.1).into(),
        |m| {
            m["objects"][0]["blends"][0]["marks"][0]["curve"] = serde_json::json!({
                "center": [0.0, 0.0, 0.0], "radius": 0.0
            })
        },
        |m| {
            m["objects"][0]["blends"][0]["consumed"] = serde_json::json!([{
                "outer": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                "material": 99
            }]);
        },
        |m| {
            m["objects"][0]["blends"][0]["consumed"] = serde_json::json!([{
                "outer": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                "blend": { "id": 3, "kind": "bevel", "role": "strip" }
            }]);
        },
        |m| {
            m["objects"][0]["blends"][0]["consumed"] = serde_json::json!([{
                "outer": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                "surface": { "kind": "cone", "apex": [0.0, 0.0, 0.0],
                             "axis": [0.0, 0.0, 2.0], "half_angle": 0.5 }
            }]);
        },
        |m| {
            let b = m["objects"][0]["blends"].as_array().unwrap()[0].clone();
            m["objects"][0]["blends"].as_array_mut().unwrap().push(b);
        },
    ];
    for tamper in tampers {
        assert!(
            matches!(
                Document::load(&with_patched_manifest(&bytes, tamper)),
                Err(LoadError::MalformedManifest { .. })
            ),
            "a tampered record loads"
        );
    }
    // A record whose geometry no longer matches loads (the file is well
    // formed); using it falls back to the derivation, which restores this
    // plain round, and refuses only when that fails too.
    let moved = with_patched_manifest(&bytes, |m| {
        let before = &mut m["objects"][0]["blends"][0]["trimmed"][0]["before"];
        before[0][0] = 0.123.into();
    });
    let mut doc3 = Document::load(&moved).unwrap();
    let o3 = doc3.visible_object_ids()[0];
    let fs = doc3.blend_features(o3).unwrap();
    assert_eq!(fs.len(), 1);
    let id = fs[0].id;
    // The derivation still restores this one, record or not.
    assert!(fs[0].intact);
    doc3.remove_blend(o3, id).unwrap();
}

/// A thin plate whose painted end face a bullnose consumes whole: the
/// paint then lives only in the blend's record.
fn painted_bullnose(doc: &mut Document) -> (ObjectId, kernel::MaterialId, kernel::BlendId) {
    let o = build_box_at(doc, p3(0.0, 0.0, 0.0), p3(1.0, 0.1, 0.02));
    let red = doc.add_material(kernel::Material {
        name: "red".to_string(),
        color: kernel::Rgba8::rgb(200, 30, 30),
        texture: None,
        pbr: None,
    });
    let end = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, -1.0, 0.0));
    doc.paint_face(o, end, Some(red)).unwrap();
    let obj = doc.object(o).unwrap();
    let edges = vec![
        edge_between(obj, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        edge_between(obj, p3(0.0, 0.0, 0.02), p3(1.0, 0.0, 0.02)),
    ];
    let (report, _) = doc
        .blend_edges(o, &spec(edges, BlendProfile::Bullnose { segments: 8 }))
        .unwrap();
    assert!(
        doc.object(o)
            .unwrap()
            .faces()
            .values()
            .all(|f| f.material != Some(red)),
        "the painted face is consumed"
    );
    (o, red, report.blends[0])
}

#[test]
fn a_material_only_a_record_carries_counts_as_used_and_clears_on_delete() {
    let mut doc = Document::new();
    let (_, red, id) = painted_bullnose(&mut doc);
    assert!(
        !doc.unused_materials().contains(&red),
        "the record's paint is a use"
    );
    // Removing the blend brings the paint back.
    let mut back = Document::load(&doc.save()).unwrap();
    let ob = back.visible_object_ids()[0];
    let fs = back.blend_features(ob).unwrap();
    back.remove_blend(ob, fs[0].id).unwrap();
    let red_back = back.material_ids()[0];
    assert!(
        back.object(ob)
            .unwrap()
            .faces()
            .values()
            .any(|f| f.material == Some(red_back)),
        "the consumed face comes back painted"
    );
    // Deleting the material forgets it in the record too: saving and the
    // state hash work, and the face comes back unpainted.
    doc.delete_material(red).unwrap();
    let bytes = doc.save();
    let _ = doc.state_hash();
    let mut again = Document::load(&bytes).unwrap();
    let oa = again.visible_object_ids()[0];
    again.remove_blend(oa, id).unwrap();
    assert!(
        again
            .object(oa)
            .unwrap()
            .faces()
            .values()
            .all(|f| f.material.is_none()),
        "the deleted paint does not come back"
    );
}

#[test]
fn a_record_s_materials_follow_the_object_into_another_document() {
    let mut src = Document::new();
    let (_, _, id) = painted_bullnose(&mut src);
    let mut dst = Document::new();
    let _ = build_box(&mut dst);
    dst.insert_document(
        &src,
        &kernel::InsertOptions {
            pose: Transform::IDENTITY,
            provenance: None,
            origin: None,
            drawing: kernel::DrawingInsert::Skip,
            hidden_objects: Default::default(),
        },
    )
    .unwrap();
    let bytes = dst.save();
    let _ = dst.state_hash();
    let mut again = Document::load(&bytes).unwrap();
    let plate = again
        .visible_object_ids()
        .into_iter()
        .find(|&o| again.object(o).unwrap().has_blend_record(id))
        .expect("the inserted plate carries its record");
    again.remove_blend(plate, id).unwrap();
    let painted = again
        .object(plate)
        .unwrap()
        .faces()
        .values()
        .filter_map(|f| f.material)
        .collect::<Vec<_>>();
    assert_eq!(painted.len(), 1);
    assert!(
        again.material_ids().contains(&painted[0]),
        "the paint is the destination's"
    );
}

#[test]
fn an_asymmetric_chamfer_on_a_turned_object_saves_the_same_bytes_after_a_load() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    doc.transform_object(
        o,
        &Transform::rotation(Vec3::new(0.3, 0.4, 0.866).normalized().unwrap(), 0.7).unwrap(),
    )
    .unwrap();
    let obj = doc.object(o).unwrap();
    let e = obj.sharp_edges()[0];
    doc.blend_edges(
        o,
        &spec(
            vec![e],
            BlendProfile::Chamfer {
                setback_a: 0.1,
                setback_b: 0.05,
            },
        ),
    )
    .unwrap();
    let bytes = doc.save();
    let m = manifest_of(&bytes);
    assert!(m["objects"][0]["blends"][0]["spec"]["primary"]["offset"].is_number());
    assert_eq!(Document::load(&bytes).unwrap().save(), bytes);
}

#[test]
fn records_of_both_operands_survive_a_union() {
    // A's round on its top front edge, B's on its bottom back edge, neither
    // beside a face the union cuts: both stay removable on the result, B's
    // under its id moved behind A's.
    let mut doc = Document::new();
    let a = build_box(&mut doc);
    let b = build_box_at(&mut doc, p3(0.25, 0.5, -0.5), p3(0.75, 1.5, 0.5));
    let ea = front_top(doc.object(a).unwrap());
    doc.blend_edges(a, &spec(vec![ea], round(0.1, 6))).unwrap();
    let eb = edge_between(
        doc.object(b).unwrap(),
        p3(0.25, 1.5, -0.5),
        p3(0.75, 1.5, -0.5),
    );
    doc.blend_edges(b, &spec(vec![eb], round(0.05, 4))).unwrap();
    let (u, _) = doc.boolean(BooleanOp::Union, a, b).unwrap();
    let fs = doc.blend_features(u).unwrap();
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert!(fs.iter().all(|f| f.intact), "{fs:?}");
    let ids: Vec<u32> = fs.iter().map(|f| f.id.0).collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(
        fs.iter()
            .all(|f| doc.object(u).unwrap().has_blend_record(f.id))
    );
    let volume = signed_volume(doc.object(u).unwrap());
    for f in fs {
        doc.remove_blend(u, f.id).unwrap();
        assert_valid_solid(doc.object(u).unwrap());
    }
    assert!(signed_volume(doc.object(u).unwrap()) > volume);
    assert!(doc.blend_features(u).unwrap().is_empty());
}

#[test]
fn the_resize_preview_shows_the_later_fillet_put_back() {
    // Previews are what commits (S4): the preview of a resize under a later
    // fillet shows that fillet back on.
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let v = right_front_vertical(doc.object(o).unwrap());
    doc.blend_edges(o, &spec(vec![v], round(0.2, 6))).unwrap();
    let rim = sharp_edges_at_height(doc.object(o).unwrap(), 1.0);
    doc.blend_edges(o, &spec(rim, round(0.1, 4))).unwrap();
    let fs = doc.blend_features(o).unwrap();
    let vid = fs
        .iter()
        .find(|f| f.size.is_some_and(|s| (s - 0.2).abs() < 1e-9))
        .unwrap()
        .id;
    let preview = doc.resize_blend_preview(o, vid, &round(0.25, 6)).unwrap();
    assert_eq!(preview.after.blend_features().len(), 2);
    doc.resize_blend(o, vid, &round(0.25, 6)).unwrap();
    assert!(same(&preview.after, doc.object(o).unwrap()));
}
