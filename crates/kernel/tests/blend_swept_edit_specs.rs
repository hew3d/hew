//! Executable specs for editable swept blends (K-6, decision D-228): a
//! rolling-ball blend (pipe tees, slanted cuts, imported meshes, generated
//! walls) records the pieces its cut leaves of each face it rolls across,
//! so Remove gives the sharp edges back exactly, Resize matches a fresh
//! blend at the new size, a later blend beside it is put back, and the
//! record rides through save and load. A record that no longer fits
//! refuses with a typed error and leaves the object as it was.

use crate::common;

use std::f64::consts::FRAC_PI_2;
use std::io::{Cursor, Read as _};

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendSpec, BooleanOp, CornerStyle, Document, EdgeId, LoadError,
    Object, SurfaceRef, Transform, Vec3,
};
use proptest::prelude::*;

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

fn round(r: f64) -> BlendProfile {
    BlendProfile::Round {
        radius: r,
        segments: 4,
    }
}

fn same(a: &Object, b: &Object) -> bool {
    kernel::same_geometry_for_test(a, b, 1e-9)
}

/// The edges between two cylinder claims whose axes cross (a tee's
/// junction).
fn junction(obj: &Object) -> Vec<EdgeId> {
    obj.edges()
        .keys()
        .filter(|&e| {
            let (x, y) = obj.edge_faces_for_test(e).unwrap();
            match (obj.faces()[x].surface, obj.faces()[y].surface) {
                (
                    Some(SurfaceRef::Cylinder { axis: p, .. }),
                    Some(SurfaceRef::Cylinder { axis: q, .. }),
                ) => p.dot(q).abs() < 0.5,
                _ => false,
            }
        })
        .collect()
}

/// Two perpendicular pipes (r 0.1 along z, r 0.07 along +y), the branch
/// turned by `phase` facets' fractions about its axis.
fn pipe_tee_at(phase: f64) -> Object {
    let mut a = cylinder(0.1, 1.0, 48);
    a.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, -0.5)))
        .unwrap();
    let mut b = cylinder(0.07, 0.5, 48);
    b.apply_transform(
        &Transform::rotation(
            Vec3::new(0.0, 0.0, 1.0),
            phase * std::f64::consts::TAU / 48.0,
        )
        .unwrap(),
    )
    .unwrap();
    b.apply_transform(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -FRAC_PI_2).unwrap())
        .unwrap();
    Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap()
}

fn pipe_tee() -> Object {
    pipe_tee_at(0.0)
}

/// The tee with everything past x = 0.01 cut away: the junction is an open
/// chain ending on the cut face.
fn half_tee() -> Object {
    let mut cutter = box_xyz(1.0, 1.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.01, -0.5, -1.0)))
        .unwrap();
    Object::boolean(
        BooleanOp::Subtract,
        &pipe_tee(),
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap()
}

/// A cylinder cut by a tilted plane, and the rim where the cut meets the
/// wall.
fn oblique_cut() -> (Object, Vec<EdgeId>) {
    let cyl = cylinder(0.3, 1.0, 48);
    let mut cutter = box_xyz(2.0, 2.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(-1.0, -1.0, 0.0)))
        .unwrap();
    cutter
        .apply_transform(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), 0.4).unwrap())
        .unwrap();
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, 0.6)))
        .unwrap();
    let cut = Object::boolean(BooleanOp::Subtract, &cyl, &cutter, &Transform::IDENTITY).unwrap();
    let rim = cut
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = cut.edge_faces_for_test(e).unwrap();
            let (p, q) = cut.edge_endpoints(e).unwrap();
            (cut.faces()[x].surface.is_some() != cut.faces()[y].surface.is_some())
                && p.z > 0.3
                && q.z > 0.3
        })
        .collect();
    (cut, rim)
}

/// An imported faceted sphere sunk in a box, and the loop where it leaves
/// the box's top.
fn sunk_sphere() -> (Object, Vec<EdgeId>) {
    let sphere = imported_sphere(16, 24, 0.3, 0);
    let mut bx = box_xyz(1.2, 1.2, 0.5);
    bx.apply_transform(&Transform::translation(Vec3::new(-0.6, -0.6, -0.63)))
        .unwrap();
    let u = Object::boolean(BooleanOp::Union, &sphere, &bx, &Transform::IDENTITY).unwrap();
    let top = face_with_normal(&u, Vec3::new(0.0, 0.0, 1.0));
    let edges = u
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = u.edge_faces_for_test(e).unwrap();
            let (p, q) = u.edge_endpoints(e).unwrap();
            (x == top) != (y == top) && p.x.hypot(p.y) < 0.35 && q.x.hypot(q.y) < 0.35
        })
        .collect();
    (u, edges)
}

fn ogee(width: f64) -> BlendProfile {
    BlendProfile::Custom {
        shape: (kernel::bundled_profiles()[0].2)(),
        width,
        arc_segments: 4,
    }
}

/// Blends `s` on `obj` and checks the result is one recorded, intact
/// blend whose removal gives `obj` back exactly, with as many restored
/// edges as were picked when the blend ran straight along them.
fn round_trip(obj: &Object, s: &BlendSpec) -> (Object, kernel::BlendId) {
    let (after, report) = obj
        .blend_edges(s)
        .unwrap_or_else(|e| panic!("blend: {e:?}"));
    assert_eq!(report.blends.len(), 1);
    let id = report.blends[0];
    assert!(after.has_blend_record(id), "a swept blend carries a record");
    let fs = after.blend_features();
    let f = fs.iter().find(|f| f.id == id).unwrap();
    assert!(f.intact, "{:?}", f.size);
    let (back, edges) = after
        .remove_blend_for_test(id)
        .unwrap_or_else(|e| panic!("remove: {e:?}"));
    assert_valid_solid(&back);
    assert!(same(&back, obj), "removal gives the sharp edges back");
    assert!(!edges.is_empty());
    (after, id)
}

// ------------------------------------------------------------ remove

#[test]
fn a_round_on_the_pipe_tee_removes_back_to_the_sharp_junction() {
    let tee = pipe_tee();
    let picked = junction(&tee);
    for r in [0.005, 0.01, 0.03] {
        let (after, id) = round_trip(&tee, &spec(picked.clone(), round(r)));
        let f = after
            .blend_features()
            .into_iter()
            .find(|f| f.id == id)
            .unwrap();
        assert_eq!(f.size, Some(r), "the size reads back");
        let (_, edges) = after.remove_blend_for_test(id).unwrap();
        assert_eq!(edges.len(), picked.len(), "{r}");
    }
}

#[test]
fn a_chamfer_and_a_router_profile_on_the_pipe_tee_remove_back() {
    let tee = pipe_tee();
    let picked = junction(&tee);
    round_trip(
        &tee,
        &spec(
            picked.clone(),
            BlendProfile::Chamfer {
                setback_a: 0.01,
                setback_b: 0.01,
            },
        ),
    );
    round_trip(&tee, &spec(picked, ogee(0.004)));
}

#[test]
fn an_open_swept_chain_on_the_half_tee_removes_back() {
    let half = half_tee();
    let picked = junction(&half);
    for p in [
        round(0.01),
        BlendProfile::Chamfer {
            setback_a: 0.008,
            setback_b: 0.008,
        },
        ogee(0.004),
    ] {
        round_trip(&half, &spec(picked.clone(), p));
    }
}

#[test]
fn a_round_on_a_slanted_cut_removes_back() {
    let (cut, rim) = oblique_cut();
    round_trip(&cut, &spec(rim, round(0.02)));
}

#[test]
fn a_round_on_an_imported_mesh_removes_back() {
    let (u, edges) = sunk_sphere();
    round_trip(&u, &spec(edges, round(0.01)));
}

// ------------------------------------------------------------ resize

/// A resize gives what removing the blend and blending again at the new
/// size gives, and keeps the blend's id.
fn resize_matches_fresh(obj: &Object, picked: &[EdgeId], from: BlendProfile, to: BlendProfile) {
    let (after, id) = round_trip(obj, &spec(picked.to_vec(), from));
    let (resized, report) = after
        .resize_blend_for_test(id, &to)
        .unwrap_or_else(|e| panic!("resize: {e:?}"));
    assert_valid_solid(&resized);
    assert_eq!(report.blends, vec![id], "a resize keeps the id");
    let (fresh, _) = obj.blend_edges(&spec(picked.to_vec(), to)).unwrap();
    assert!(same(&resized, &fresh), "a resize is a fresh blend");
    // And the resized blend is editable in turn.
    let (back, _) = resized.remove_blend_for_test(id).unwrap();
    assert!(same(&back, obj));
}

#[test]
fn a_round_on_the_pipe_tee_resizes_as_a_fresh_round() {
    let tee = pipe_tee();
    let picked = junction(&tee);
    resize_matches_fresh(&tee, &picked, round(0.01), round(0.02));
    resize_matches_fresh(&tee, &picked, round(0.03), round(0.005));
    // A profile change is a resize too.
    resize_matches_fresh(
        &tee,
        &picked,
        round(0.01),
        BlendProfile::Chamfer {
            setback_a: 0.01,
            setback_b: 0.01,
        },
    );
}

#[test]
fn an_open_chain_and_a_slanted_cut_resize_as_fresh_blends() {
    let half = half_tee();
    resize_matches_fresh(&half, &junction(&half), round(0.01), round(0.006));
    let (cut, rim) = oblique_cut();
    resize_matches_fresh(&cut, &rim, round(0.02), round(0.01));
}

#[test]
fn a_resize_past_what_rolls_refuses_typed_and_keeps_the_blend() {
    let tee = pipe_tee();
    let (after, id) = round_trip(&tee, &spec(junction(&tee), round(0.01)));
    let before = after.to_polygons();
    match after.resize_blend_for_test(id, &round(0.45)) {
        Err(BlendError::RadiusTooLarge { .. } | BlendError::UnsupportedVertex { .. }) => {}
        other => panic!("{:?}", other.map(|_| ())),
    }
    assert_eq!(after.to_polygons(), before, "the object is untouched");
    assert!(after.blend_features()[0].intact);
}

// ------------------------------------------------------ later work

/// The branch's far end rim, an analytic rim away from the junction.
fn branch_end_rim(obj: &Object) -> Vec<EdgeId> {
    obj.edges()
        .keys()
        .filter(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            (p.y - 0.5).abs() < 1e-9 && (q.y - 0.5).abs() < 1e-9 && !obj.edges()[e].soft
        })
        .collect()
}

#[test]
fn removing_the_junction_round_keeps_a_later_rim_round() {
    let tee = pipe_tee();
    let (rounded, junction_id) = round_trip(&tee, &spec(junction(&tee), round(0.01)));
    let rim = branch_end_rim(&rounded);
    assert!(!rim.is_empty());
    let (both, report) = rounded
        .blend_edges(&spec(rim.clone(), round(0.01)))
        .unwrap();
    let rim_id = report.blends[0];
    // Every blend on it stays editable.
    assert!(both.blend_features().iter().all(|f| f.intact));
    let (back, _) = both.remove_blend_for_test(junction_id).unwrap();
    assert_valid_solid(&back);
    let (rim_only, _) = tee
        .blend_edges(&spec(branch_end_rim(&tee), round(0.01)))
        .unwrap();
    assert!(same(&back, &rim_only));
    assert!(
        back.blend_features()
            .iter()
            .any(|f| f.id == rim_id && f.intact)
    );
    // Resizing the junction leaves the rim as it was.
    let (resized, _) = both
        .resize_blend_for_test(junction_id, &round(0.02))
        .unwrap();
    let (fresh, _) = rim_only
        .blend_edges(&spec(junction(&rim_only), round(0.02)))
        .unwrap();
    assert!(same(&resized, &fresh));
}

#[test]
fn a_cut_through_a_piece_refuses_typed_and_leaves_the_object() {
    // A notch through the main pipe's wall next to the junction round cuts
    // pieces the record kept: the record no longer fits, and nothing
    // derives this blend, so Remove and Resize refuse `BlendModified`.
    let tee = pipe_tee();
    let (after, id) = round_trip(&tee, &spec(junction(&tee), round(0.01)));
    let mut notch = box_xyz(0.04, 0.2, 0.04);
    notch
        .apply_transform(&Transform::translation(Vec3::new(-0.02, 0.0, 0.07)))
        .unwrap();
    let cut = Object::boolean(BooleanOp::Subtract, &after, &notch, &Transform::IDENTITY).unwrap();
    let polys = cut.to_polygons();
    assert_eq!(
        cut.remove_blend_for_test(id).map(|_| ()),
        Err(BlendError::BlendModified)
    );
    assert_eq!(
        cut.resize_blend_for_test(id, &round(0.02)).map(|_| ()),
        Err(BlendError::BlendModified)
    );
    assert_eq!(cut.to_polygons(), polys);
    let f = cut.blend_features();
    assert!(!f.iter().find(|f| f.id == id).unwrap().intact);
}

// ------------------------------------------------- documents and files

/// The tee as the app builds it, in a document, with its junction loop.
fn tee_document() -> (Document, kernel::ObjectId, Vec<EdgeId>) {
    app_pipe_tee(0.0, 0.0)
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

/// A unit box as drawn in a document.
fn box_document() -> (Document, kernel::ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    let id = doc.extrude_region(s, r, 1.0).unwrap().0;
    (doc, id)
}

#[test]
fn a_swept_record_rides_through_save_load_and_undo() {
    let (mut doc, o, picked) = tee_document();
    let plain = doc.object(o).unwrap().clone();
    let (report, _) = doc.blend_edges(o, &spec(picked, round(0.01))).unwrap();
    let id = report.blends[0];
    let bytes = doc.save();
    let m = manifest_of(&bytes);
    assert_eq!(m["format_version"], 33);
    let rec = &m["objects"][0]["blends"][0];
    assert!(
        !rec["pieces"].as_array().unwrap().is_empty(),
        "the pieces are in the file"
    );
    let mut loaded = Document::load(&bytes).unwrap();
    assert_eq!(loaded.save(), bytes, "a loaded file saves the same bytes");
    let lo = loaded.visible_object_ids()[0];
    assert!(loaded.blend_features(lo).unwrap()[0].intact);
    loaded.remove_blend(lo, id).unwrap();
    assert!(same(loaded.object(lo).unwrap(), &plain));
    loaded.undo().unwrap();
    assert!(loaded.blend_features(lo).unwrap()[0].intact);
    loaded
        .resize_blend(lo, id, &round(0.02))
        .unwrap_or_else(|e| panic!("{e:?}"));
    loaded.undo().unwrap();
    loaded.redo().unwrap();
    assert!(loaded.blend_features(lo).unwrap()[0].intact);
}

#[test]
fn pieces_are_checked_for_shape() {
    let (mut doc, o, picked) = tee_document();
    doc.blend_edges(o, &spec(picked, round(0.01))).unwrap();
    let bytes = doc.save();
    let tampers: Vec<fn(&mut serde_json::Value)> = vec![
        |m| m["objects"][0]["blends"][0]["pieces"][0] = serde_json::json!([]),
        |m| {
            m["objects"][0]["blends"][0]["pieces"][0][0] =
                serde_json::json!([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]])
        },
        |m| m["objects"][0]["blends"][0]["pieces"][0][0][0][0] = serde_json::json!("x"),
    ];
    for tamper in tampers {
        assert!(
            matches!(
                Document::load(&with_patched_manifest(&bytes, tamper)),
                Err(LoadError::MalformedManifest { .. })
            ),
            "a tampered piece loads"
        );
    }
    // A record whose pieces no longer match the geometry loads (the file is
    // well formed) and refuses to edit, typed, nothing changed.
    let moved = with_patched_manifest(&bytes, |m| {
        m["objects"][0]["blends"][0]["pieces"][0][0][0][0] = 0.123.into();
    });
    let mut d = Document::load(&moved).unwrap();
    let lo = d.visible_object_ids()[0];
    let id = d.blend_features(lo).unwrap()[0].id;
    assert!(!d.blend_features(lo).unwrap()[0].intact);
    let before = d.save();
    assert!(d.remove_blend(lo, id).is_err());
    assert_eq!(d.save(), before);
}

#[test]
fn a_record_written_before_pieces_still_opens_and_edits() {
    // A file written before pieces existed: a box with an analytic round,
    // its record trimmed loops only, with no `pieces` key at all.
    let (mut doc, o) = box_document();
    let plain = doc.object(o).unwrap().clone();
    let e = edge_between(doc.object(o).unwrap(), p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    let (report, _) = doc.blend_edges(o, &spec(vec![e], round(0.1))).unwrap();
    let old_bytes = with_patched_manifest(&doc.save(), |m| {
        let rec = m["objects"][0]["blends"][0].as_object_mut().unwrap();
        assert!(
            rec.remove("pieces").is_none(),
            "a trimmed round has no pieces"
        );
    });
    let mut old = Document::load(&old_bytes).unwrap();
    let lo = old.visible_object_ids()[0];
    assert!(old.blend_features(lo).unwrap()[0].intact);
    old.remove_blend(lo, report.blends[0]).unwrap();
    assert!(same(old.object(lo).unwrap(), &plain));
}

// ------------------------------------------------------------ properties

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    /// Wherever the facets fall and whatever the size, a swept round that
    /// builds removes back to the sharp tee exactly, and a resize to
    /// another size that builds is the fresh blend at that size.
    #[test]
    fn a_swept_round_that_builds_is_editable(
        phase in 0.0f64..1.0,
        r in 0.004f64..0.03,
        r2 in 0.004f64..0.03,
    ) {
        let tee = pipe_tee_at(phase);
        let picked = junction(&tee);
        let Ok((after, report)) = tee.blend_edges(&spec(picked.clone(), round(r))) else {
            return Ok(());
        };
        let id = report.blends[0];
        prop_assert!(after.has_blend_record(id));
        let (back, _) = after.remove_blend_for_test(id).unwrap();
        prop_assert!(same(&back, &tee));
        if let Ok((fresh, _)) = tee.blend_edges(&spec(picked, round(r2))) {
            let (resized, _) = after.resize_blend_for_test(id, &round(r2)).unwrap();
            prop_assert!(same(&resized, &fresh));
        }
    }
}

// ------------------------------------------------ review round (D-231)

/// The same solid somewhere else blends the same: a swept chain's start
/// follows the geometry (review F2, F14; the start rule itself is pinned
/// in `blend::chain::tests`). Moved a meter or more, the import's
/// coordinates round differently and the cut can gain a sliver face, a
/// float effect of the whole kernel with the start unchanged, so the
/// offset here stays small.
#[test]
fn a_moved_solid_blends_as_the_unmoved_one_moved() {
    let (u, edges) = sunk_sphere();
    let offset = Vec3::new(0.5, 0.25, 0.0);
    let t = Transform::translation(offset);
    let mut moved = u.clone();
    moved.apply_transform(&t).unwrap();
    let mut moved_edges = Vec::new();
    for &e in &edges {
        let (a, b) = u.edge_endpoints(e).unwrap();
        moved_edges.push(edge_between(&moved, a + offset, b + offset));
    }
    for r in [0.01, 0.02] {
        let here = u.blend_edges(&spec(edges.clone(), round(r)));
        let there = moved.blend_edges(&spec(moved_edges.clone(), round(r)));
        match (here, there) {
            (Ok((mut a, _)), Ok((b, _))) => {
                a.apply_transform(&t).unwrap();
                assert!(kernel::same_geometry_for_test(&a, &b, 1e-7), "{r}");
            }
            (Err(_), Err(_)) => {}
            (h, t) => panic!("{r}: here {:?}, there {:?}", h.map(|_| ()), t.map(|_| ())),
        }
    }
}

/// An imported disc (no claims): a 48-sided prism 30 mm round, 10 mm
/// thick, and its top rim.
fn imported_disc() -> (Object, Vec<EdgeId>) {
    let n = 48;
    let (r, h) = (0.03, 0.01);
    let mut pts = Vec::new();
    for z in [0.0, h] {
        for k in 0..n {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            pts.push(p3(r * a.cos(), r * a.sin(), z));
        }
    }
    let mut faces: Vec<Vec<usize>> = Vec::new();
    faces.push((0..n).rev().collect());
    faces.push((n..2 * n).collect());
    for k in 0..n {
        let j = (k + 1) % n;
        faces.push(vec![k, j, n + j]);
        faces.push(vec![k, n + j, n + k]);
    }
    let o = Object::from_polygons_import_for_test(&pts, &faces).unwrap();
    let rim = sharp_edges_at_height(&o, h);
    (o, rim)
}

/// A ball wider than the disc's top cannot round its rim: the round
/// refuses, typed, and never rests on the far side's rim edge (review F5).
/// Small rounds build and remove back.
#[test]
fn a_ball_wider_than_a_discs_top_refuses() {
    let (o, rim) = imported_disc();
    assert_eq!(rim.len(), 48);
    let (after, report) = o.blend_edges(&spec(rim.clone(), round(0.003))).unwrap();
    assert_valid_solid(&after);
    let (back, _) = after.remove_blend_for_test(report.blends[0]).unwrap();
    assert!(same(&back, &o));
    for r in [0.02, 0.04] {
        match o.blend_edges(&spec(rim.clone(), round(r))) {
            Err(BlendError::RadiusTooLarge { max, .. }) => {
                assert!(max < 0.01 + 1e-9, "{r}: {max}");
                assert!(o.blend_edges(&spec(rim.clone(), round(max))).is_ok());
            }
            Err(BlendError::UnsupportedVertex { .. }) => {}
            other => panic!("{r}: {:?}", other.map(|_| ())),
        }
    }
}

/// A swept record whose `pieces` were stripped (an earlier v28 build's
/// view of it) loads, reads as changed, and refuses to edit with the
/// document unchanged: replay never leaves the cut pieces behind (F18,
/// F20).
#[test]
fn a_swept_record_without_its_pieces_refuses_untouched() {
    let (mut doc, o, picked) = tee_document();
    doc.blend_edges(o, &spec(picked, round(0.01))).unwrap();
    let stripped = with_patched_manifest(&doc.save(), |m| {
        let rec = m["objects"][0]["blends"][0].as_object_mut().unwrap();
        assert!(rec.remove("pieces").is_some());
    });
    let mut d = Document::load(&stripped).unwrap();
    let lo = d.visible_object_ids()[0];
    let f = d.blend_features(lo).unwrap();
    assert!(!f[0].intact);
    let before = d.save();
    assert!(d.remove_blend(lo, f[0].id).is_err());
    assert!(d.resize_blend(lo, f[0].id, &round(0.02)).is_err());
    assert_eq!(d.save(), before);
}

/// Facets of one claim whose stored values differ by rounding are one
/// surface to the swept blend (review F4: sides compare claims with
/// `SurfaceRef::same_surface`, as the rest of the kernel groups them): a
/// slanted cut whose wall claims are nudged a few ulps rounds as the
/// untouched one does.
#[test]
fn claims_a_few_ulps_apart_are_one_surface() {
    let (cut, rim) = oblique_cut();
    let mut nudged = cut.clone();
    let faces: Vec<_> = cut.faces().keys().collect();
    for (i, f) in faces.into_iter().enumerate() {
        if let Some(SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius,
        }) = cut.faces()[f].surface
            && i % 2 == 0
        {
            nudged.set_face_surface_for_test(
                f,
                Some(SurfaceRef::Cylinder {
                    axis_point,
                    axis,
                    radius: radius * (1.0 + 4.0 * f64::EPSILON),
                }),
            );
        }
    }
    nudged.validate().unwrap();
    let (a, _) = cut.blend_edges(&spec(rim.clone(), round(0.02))).unwrap();
    let (b, report) = nudged.blend_edges(&spec(rim, round(0.02))).unwrap();
    assert!(kernel::same_geometry_for_test(&a, &b, 1e-9));
    let (back, _) = b.remove_blend_for_test(report.blends[0]).unwrap();
    assert!(same(&back, &nudged));
}

/// Every face's dictionary value under `org.test` / `n`, sorted.
fn face_tags(obj: &Object) -> Vec<i64> {
    let mut out: Vec<i64> = obj
        .faces()
        .keys()
        .filter_map(|f| match obj.face_attrs(f)?.get("org.test")?.get("n")? {
            kernel::attr::AttrValue::Int(n) => Some(*n),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out
}

/// The faces a blend consumed keep their attribute dictionaries in the
/// record (manifest v31, `consumed[].attrs`): through save and load, Remove
/// gives each face back the dictionary it had; a tampered entry refuses.
#[test]
fn consumed_faces_keep_their_dictionaries_through_save_and_load() {
    let (mut doc, o, picked) = tee_document();
    let faces: Vec<_> = doc.object(o).unwrap().faces().keys().collect();
    for (n, &f) in faces.iter().enumerate() {
        doc.face_attr_set(
            o,
            f,
            "org.test",
            "n",
            kernel::attr::AttrValue::Int(n as i64),
        )
        .unwrap();
    }
    let tagged = face_tags(doc.object(o).unwrap());
    assert_eq!(tagged.len(), faces.len());
    let (report, _) = doc.blend_edges(o, &spec(picked, round(0.01))).unwrap();
    let id = report.blends[0];
    let bytes = doc.save();
    let m = manifest_of(&bytes);
    let consumed = m["objects"][0]["blends"][0]["consumed"].as_array().unwrap();
    assert!(!consumed.is_empty(), "the swept blend consumed faces");
    assert!(
        consumed
            .iter()
            .all(|f| f["attrs"]["org.test"]["n"].is_i64()),
        "every consumed face carries its dictionary"
    );
    let mut loaded = Document::load(&bytes).unwrap();
    assert_eq!(loaded.save(), bytes, "a loaded file saves the same bytes");
    let lo = loaded.visible_object_ids()[0];
    loaded.remove_blend(lo, id).unwrap();
    assert_eq!(face_tags(loaded.object(lo).unwrap()), tagged);

    // A present but empty dictionary is malformed.
    let tampered = with_patched_manifest(&bytes, |m| {
        m["objects"][0]["blends"][0]["consumed"][0]["attrs"] = serde_json::json!({});
    });
    assert!(matches!(
        Document::load(&tampered),
        Err(LoadError::MalformedManifest { .. })
    ));
}
