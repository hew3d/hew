//! Executable specs for the blend's document entry points (WP05 §10):
//! commit through the snapshot undo, the clamping preview, and the
//! in-instance form.

use crate::common;
use kernel::trig;

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendSpec, CornerStyle, Document, DocumentError, EdgeId, NodeId,
    ObjectId, Point3, Transform, Vec3,
};

fn build_box(doc: &mut Document) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

fn front_top(doc: &Document, o: ObjectId) -> EdgeId {
    edge_between(doc.object(o).unwrap(), p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0))
}

fn round(radius: f64) -> BlendSpec {
    BlendSpec {
        edges: Vec::new(),
        profile: BlendProfile::Round {
            radius,
            segments: 6,
        },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

#[test]
fn resizing_a_chamfer_band_is_not_limited_by_its_own_width() {
    // Playtest verification run, F-72, reported a 3 mm chamfer on a free
    // box edge resized to 5 mm previewing "limited to 4.24 mm by other
    // geometry" (the band's own diagonal). The kernel path does not
    // reproduce it, at this size or the playtest's; this guards that it
    // never counts the band it is replacing.
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut s = round(0.03);
    s.edges = vec![front_top(&doc, o)];
    s.profile = BlendProfile::Chamfer {
        setback_a: 0.03,
        setback_b: 0.03,
    };
    let (report, _) = doc.blend_edges(o, &s).unwrap();
    let bigger = BlendProfile::Chamfer {
        setback_a: 0.05,
        setback_b: 0.05,
    };
    let preview = doc
        .resize_blend_preview(o, report.blends[0], &bigger)
        .unwrap();
    assert!(
        preview.clamped.is_none(),
        "clamped to {:?}",
        preview.clamped.map(|l| (l.max, l.limited_by))
    );
    let (resized, _) = doc.resize_blend(o, report.blends[0], &bigger).unwrap();
    assert!((resized.profile.size().unwrap() - 0.05).abs() < 1e-12);
}

#[test]
fn blend_commits_and_undoes_bit_exact() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut spec = round(0.1);
    spec.edges = vec![front_top(&doc, o)];
    let before = doc.state_hash();
    let (report, change) = doc.blend_edges(o, &spec).unwrap();
    assert_eq!(change.objects_touched, vec![o]);
    assert_eq!(report.new_faces.len(), 6);
    assert_eq!(doc.object(o).unwrap().faces().len(), 12);
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    for _ in 0..3 {
        doc.redo().unwrap();
        assert_eq!(doc.state_hash(), after);
        doc.undo().unwrap();
        assert_eq!(doc.state_hash(), before);
    }
}

#[test]
fn blend_refusal_leaves_document_unchanged() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut spec = round(2.0);
    spec.edges = vec![front_top(&doc, o)];
    let before = doc.state_hash();
    assert!(matches!(
        doc.blend_edges(o, &spec),
        Err(DocumentError::Blend(BlendError::RadiusTooLarge { .. }))
    ));
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn preview_clamps_to_the_limit() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut spec = round(2.0);
    spec.edges = vec![front_top(&doc, o)];
    let before = doc.state_hash();
    let p = doc.blend_preview(o, &spec).unwrap();
    assert_eq!(doc.state_hash(), before, "preview is pure");
    let limit = p.clamped.expect("clamped");
    assert!(limit.max > 0.9 && limit.max <= 1.0, "{}", limit.max);
    match p.applied {
        BlendProfile::Round { radius, .. } => assert_eq!(radius, limit.max),
        other => panic!("{other:?}"),
    }
    assert_eq!(p.replaced.len(), 4, "top, front and both ends");
    assert_eq!(p.new_faces.len(), 6);
    assert_valid_solid(&p.after);

    let mut fits = round(0.1);
    fits.edges = spec.edges.clone();
    assert!(doc.blend_preview(o, &fits).unwrap().clamped.is_none());
}

#[test]
fn in_instance_divides_lengths_by_the_scale() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    doc.transform_instance(inst, &Transform::uniform_scale(2.0))
        .unwrap();
    let mut spec = round(0.2);
    spec.edges = vec![front_top(&doc, o)];
    let (report, _) = doc.blend_edges_in_instance(inst, o, &spec).unwrap();
    match report.profile {
        BlendProfile::Round { radius, .. } => assert!((radius - 0.1).abs() < 1e-15),
        other => panic!("{other:?}"),
    }
}

#[test]
fn in_instance_refuses_non_uniform_pose() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    doc.transform_instance(inst, &Transform::scale(Vec3::new(1.0, 2.0, 1.0)))
        .unwrap();
    let mut spec = round(0.1);
    spec.edges = vec![front_top(&doc, o)];
    assert_eq!(
        doc.blend_edges_in_instance(inst, o, &spec).unwrap_err(),
        DocumentError::Blend(BlendError::NonUniformInstance)
    );
}

fn top_rim(doc: &Document, o: ObjectId) -> Vec<EdgeId> {
    let obj = doc.object(o).unwrap();
    let c = [
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ];
    (0..4)
        .map(|i| edge_between(obj, c[i], c[(i + 1) % 4]))
        .collect()
}

#[test]
fn push_pull_grows_and_removes() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut s = round(0.1);
    s.edges = vec![front_top(&doc, o)];
    let (report, _) = doc.blend_edges(o, &s).unwrap();
    // The middle strip facet (N = 6): push it in by δ.
    let obj = doc.object(o).unwrap();
    let mid = report.new_faces[2];
    let n = obj.faces()[mid].plane.normal();
    let theta = std::f64::consts::FRAC_PI_2;
    let gap = 1.0 / trig::sin(theta / 2.0) - 1.0;
    let delta = 0.01;
    doc.push_pull_blend(o, mid, -delta).unwrap();
    let feats = doc.blend_features(o).unwrap();
    assert_eq!(feats.len(), 1);
    let r = feats[0].size.unwrap();
    assert!((r - (0.1 * gap + delta) / gap).abs() < 1e-9, "{r}");
    let _ = n;
    // Pull out past the gap: the sharp edge comes back.
    let face = doc
        .object(o)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.blend.is_some())
        .unwrap()
        .0;
    doc.push_pull_blend(o, face, 1.0).unwrap();
    assert!(doc.blend_features(o).unwrap().is_empty());
    assert_eq!(doc.object(o).unwrap().faces().len(), 6);
}

#[test]
fn blend_aware_neighbor_push() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut s = round(0.1);
    s.edges = top_rim(&doc, o);
    doc.blend_edges(o, &s).unwrap();
    let before = doc.state_hash();
    let top = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, 0.0, 1.0));
    doc.push_pull_with_blends(o, top, 0.1).unwrap();
    // Equal to the rim fillet on the taller box.
    let mut tall = box_xyz(1.0, 1.0, 1.1);
    let c = [
        p3(0.0, 0.0, 1.1),
        p3(1.0, 0.0, 1.1),
        p3(1.0, 1.0, 1.1),
        p3(0.0, 1.0, 1.1),
    ];
    let rim: Vec<EdgeId> = (0..4)
        .map(|i| edge_between(&tall, c[i], c[(i + 1) % 4]))
        .collect();
    tall = tall
        .blend_edges(&BlendSpec {
            edges: rim,
            ..round(0.1)
        })
        .unwrap()
        .0;
    assert!(kernel::same_geometry_for_test(
        doc.object(o).unwrap(),
        &tall,
        1e-9
    ));
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before, "one undo step");
}

#[test]
fn blend_aware_falls_back() {
    // No blend beside the face: the caller gets Err and uses the ordinary
    // Push/Pull.
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let top = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, 0.0, 1.0));
    let before = doc.state_hash();
    assert!(doc.push_pull_with_blends(o, top, 0.1).is_err());
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn logical_paint() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut s = round(0.1);
    s.edges = vec![front_top(&doc, o)];
    let (report, _) = doc.blend_edges(o, &s).unwrap();
    let mat = doc.add_material(kernel::Material::solid(
        "Red",
        kernel::Rgba8::rgb(220, 30, 30),
    ));
    let before = doc.state_hash();
    doc.paint_logical_face(o, report.new_faces[0], Some(mat))
        .unwrap();
    let obj = doc.object(o).unwrap();
    for f in obj.faces().values().filter(|f| f.blend.is_some()) {
        assert_eq!(f.material, Some(mat));
    }
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn every_facet_of_a_rim_fillet_resizes_it_after_a_neighbor_push() {
    // A face's whole top rim, then the top pushed up with the fillet kept:
    // every facet of the fillet (straight strips and the mitered corners)
    // is still tagged, and Push/Pull from any of them resizes the one
    // fillet rather than moving that facet alone.
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut s = round(0.1);
    s.edges = top_rim(&doc, o);
    doc.blend_edges(o, &s).unwrap();
    let top = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, 0.0, 1.0));
    doc.push_pull_with_blends(o, top, 0.5).unwrap();
    let obj = doc.object(o).unwrap().clone();
    let curved: Vec<_> = obj
        .faces()
        .iter()
        .filter(|(_, f)| f.surface.is_some())
        .collect();
    assert!(!curved.is_empty());
    assert!(
        curved.iter().all(|(_, f)| f.blend.is_some()),
        "every rounded facet is tagged"
    );
    let faces_before = obj.faces().len();
    for (fid, f) in obj.faces() {
        if f.blend.is_none() {
            continue;
        }
        let mut d = doc.clone();
        d.push_pull_blend(o, fid, -0.01).unwrap();
        let feats = d.blend_features(o).unwrap();
        assert_eq!(feats.len(), 1);
        assert!(feats[0].size.unwrap() > 0.1, "the whole fillet grew");
        assert_eq!(d.object(o).unwrap().faces().len(), faces_before);
    }
}

/// The armed preview resolves corners the way the commit does: verticals
/// filleted over an existing rim fillet re-solve into rolled corners, and
/// the preview is the committed geometry.
#[test]
fn preview_resolves_corners_as_the_commit_does() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let top = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, 0.0, 1.0));
    let rim = doc
        .object(o)
        .unwrap()
        .face_loop_edges(top, p3(0.5, 0.0, 1.0));
    let mut rim_spec = round(0.05);
    rim_spec.edges = rim;
    doc.blend_edges(o, &rim_spec).unwrap();

    let obj = doc.object(o).unwrap();
    let ends: Vec<EdgeId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(obj, p3(x, y, 0.0), p3(x, y, 0.95)))
        .collect();
    let mut spec = round(0.2);
    spec.edges = ends;

    let preview = doc.blend_preview(o, &spec).unwrap();
    let mut committed = doc.clone();
    committed.blend_edges(o, &spec).unwrap();
    assert!(
        kernel::same_geometry_for_test(&preview.after, committed.object(o).unwrap(), 1e-9),
        "the preview must show the corners the commit makes"
    );
    assert_valid_solid(&preview.after);
}

/// A box whose top rim is rounded at `rim` and whose four vertical edges are
/// then rounded at `verticals` (the corners re-solve into rolled ones).
fn rim_then_verticals(rim: f64, verticals: f64) -> (Document, ObjectId) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut rim_spec = round(rim);
    rim_spec.edges = top_rim(&doc, o);
    doc.blend_edges(o, &rim_spec).unwrap();
    let spec = vertical_spec(&doc, o, rim, verticals);
    doc.blend_edges(o, &spec).unwrap();
    (doc, o)
}

/// The four vertical edges of a box whose top rim is rounded at `rim`.
fn vertical_spec(doc: &Document, o: ObjectId, rim: f64, size: f64) -> BlendSpec {
    let obj = doc.object(o).unwrap();
    let mut spec = round(size);
    spec.edges = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(obj, p3(x, y, 0.0), p3(x, y, 1.0 - rim)))
        .collect();
    spec
}

/// The id of the blend on `o` whose size reads back as `size`.
fn blend_sized(doc: &Document, o: ObjectId, size: f64) -> kernel::BlendId {
    doc.blend_features(o)
        .unwrap()
        .into_iter()
        .find(|f| f.size.is_some_and(|s| (s - size).abs() < 1e-9))
        .expect("a blend of that size")
        .id
}

/// `spec` at `size`.
fn sized(spec: &BlendSpec, size: f64) -> BlendSpec {
    let mut s = spec.clone();
    s.profile = spec.profile.scaled(size / spec.profile.size().unwrap());
    s
}

/// How far past a limit a size must be for the commit to refuse it: the
/// limit search stops within `POINT_MERGE` of the edge, so a micron clears
/// it.
const PAST_LIMIT: f64 = 1e-6;

/// A commit past `limit` refuses as too large and names `limit` (to within
/// the search's tolerance).
fn assert_too_large<T: std::fmt::Debug>(result: Result<T, BlendError>, limit: f64) {
    match result {
        Err(BlendError::RadiusTooLarge { max, .. }) => {
            assert!((max - limit).abs() <= 1e-8, "names {max}, limit {limit}")
        }
        other => panic!("expected RadiusTooLarge near {limit}, got {other:?}"),
    }
}

/// [`assert_too_large`] for a document call.
fn assert_doc_too_large<T: std::fmt::Debug>(result: Result<T, DocumentError>, limit: f64) {
    assert_too_large(
        result.map_err(|e| match e {
            DocumentError::Blend(b) => b,
            other => panic!("expected a blend refusal, got {other:?}"),
        }),
        limit,
    );
}

/// A box with its top rim rounded at `rim`.
fn rim_box(rim: f64) -> (Document, ObjectId) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut rim_spec = round(rim);
    rim_spec.edges = top_rim(&doc, o);
    doc.blend_edges(o, &rim_spec).unwrap();
    (doc, o)
}

/// The limit at a corner that re-solves (verticals filleted over a rounded
/// rim, too large for the box) is the one the commit has: the commit
/// re-solves the corner, so the limit search re-solves it too. Before, the
/// search planned the verticals against the rim fillet in one pass, which
/// the planner refuses outright, and the preview failed instead of
/// clamping.
#[test]
fn preview_clamps_a_rolled_corner_to_what_the_commit_accepts() {
    let (doc, o) = rim_box(0.05);
    let spec = vertical_spec(&doc, o, 0.05, 0.6);

    let preview = doc.blend_preview(o, &spec).unwrap();
    let limit = preview.clamped.expect("0.6 is too large for a 1 m box");
    assert!(
        (limit.max - 0.5).abs() < 1e-6,
        "half the box's width, got {}",
        limit.max
    );
    assert_eq!(preview.applied.size(), Some(limit.max));

    let mut at_limit = doc.clone();
    at_limit.blend_edges(o, &sized(&spec, limit.max)).unwrap();
    assert!(
        kernel::same_geometry_for_test(&preview.after, at_limit.object(o).unwrap(), 1e-9),
        "the clamped preview is what the commit makes at the limit"
    );
    assert_doc_too_large(
        doc.clone()
            .blend_edges(o, &sized(&spec, limit.max + PAST_LIMIT)),
        limit.max,
    );
    assert_valid_solid(&preview.after);
}

/// The object-level limit is the commit's at a re-solved corner, and the
/// commit's refusal past it names that limit rather than the single-pass
/// planner's refusal of the corner.
#[test]
fn rolled_corner_limit_and_refusal_match_the_commit() {
    let (doc, o) = rim_box(0.05);
    let spec = vertical_spec(&doc, o, 0.05, 0.6);
    let obj = doc.object(o).unwrap();

    let limit = obj.max_blend_size(&spec).unwrap();
    assert!(obj.blend_edges(&sized(&spec, limit.max)).is_ok());
    assert_too_large(
        obj.blend_edges(&sized(&spec, limit.max + PAST_LIMIT)),
        limit.max,
    );
    match obj.blend_edges(&spec) {
        Err(BlendError::RadiusTooLarge { max, .. }) => assert_eq!(max, limit.max),
        other => panic!("expected RadiusTooLarge naming the limit, got {other:?}"),
    }
}

/// Where the rim is large, the sizes the corner re-solve builds start near
/// the rim's size: below it the verticals would go on after the rim, which
/// the planner cannot build. The limit search finds that window instead of
/// halving past it to "max 0": the commit's refusal names the real limit
/// and the preview clamps to it.
#[test]
fn the_limit_finds_the_sizes_that_build_above_a_large_rim() {
    for (rim, asked) in [(0.4, 0.6), (0.45, 0.6), (0.3, 2.0)] {
        let (doc, o) = rim_box(rim);
        let spec = vertical_spec(&doc, o, rim, asked);
        let obj = doc.object(o).unwrap();
        let limit = obj.max_blend_size(&spec).unwrap();
        assert!(
            limit.max > rim && (limit.max - 0.5).abs() < 1e-6,
            "rim {rim}: limit {}",
            limit.max
        );
        assert!(obj.blend_edges(&sized(&spec, limit.max)).is_ok());
        assert_too_large(obj.blend_edges(&spec), limit.max);
        assert_too_large(
            obj.blend_edges(&sized(&spec, limit.max + PAST_LIMIT)),
            limit.max,
        );
        let preview = doc.blend_preview(o, &spec).unwrap();
        assert_eq!(preview.clamped.map(|l| l.max), Some(limit.max));
        let mut at_limit = doc.clone();
        at_limit.blend_edges(o, &sized(&spec, limit.max)).unwrap();
        assert!(kernel::same_geometry_for_test(
            &preview.after,
            at_limit.object(o).unwrap(),
            1e-9
        ));
    }
}

/// A chamfer of `size` on both faces.
fn chamfer(size: f64) -> BlendProfile {
    BlendProfile::Chamfer {
        setback_a: size,
        setback_b: size,
    }
}

/// A box whose top rim is `rim` (rounded or chamfered), with chamfered
/// verticals asked at `asked`.
fn chamfer_verticals_over(rim: BlendProfile, asked: f64) -> (Document, ObjectId, BlendSpec) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let size = rim.size().unwrap();
    let mut rim_spec = round(size);
    rim_spec.profile = rim;
    rim_spec.edges = top_rim(&doc, o);
    doc.blend_edges(o, &rim_spec).unwrap();
    let mut spec = vertical_spec(&doc, o, size, asked);
    spec.profile = chamfer(asked);
    (doc, o, spec)
}

/// Chamfered verticals over a rounded rim build only from about twice the
/// rim's size (0.4025..0.4975 over a 0.2 rim, 0.1025..0.4975 over 0.05):
/// the limit search finds that window from any asked size, and where no
/// size up to the asked one builds, the refusal is the planner's
/// `UnsupportedVertex`, as the commit's is, not "too large, at most 0".
#[test]
fn the_limit_finds_a_window_well_above_the_neighbours_size() {
    for (rim, asked, fits) in [
        (0.2, 2.0, Some(0.4975)),
        (0.2, 0.3, None),
        (0.05, 2.0, Some(0.4975)),
        (0.05, 0.1, None),
    ] {
        let (doc, o, spec) = chamfer_verticals_over(
            BlendProfile::Round {
                radius: rim,
                segments: 6,
            },
            asked,
        );
        let obj = doc.object(o).unwrap();
        match fits {
            Some(top) => {
                let limit = obj.max_blend_size(&spec).unwrap().max;
                assert!(
                    limit >= top && limit <= 0.5,
                    "rim {rim}, asked {asked}: {limit}"
                );
                assert!(obj.blend_edges(&sized(&spec, limit)).is_ok());
                assert_too_large(obj.blend_edges(&spec), limit);
                assert_too_large(obj.blend_edges(&sized(&spec, limit + PAST_LIMIT)), limit);
                let preview = doc.blend_preview(o, &spec).unwrap();
                assert_eq!(preview.clamped.map(|l| l.max), Some(limit));
            }
            None => {
                assert!(
                    matches!(
                        obj.max_blend_size(&spec),
                        Err(BlendError::UnsupportedVertex { .. })
                    ),
                    "rim {rim}, asked {asked}: {:?}",
                    obj.max_blend_size(&spec)
                );
                assert!(matches!(
                    obj.blend_edges(&spec),
                    Err(BlendError::UnsupportedVertex { .. })
                ));
            }
        }
    }
    // Over the 0.05 rim, 0.3 is inside the window: it fits as asked.
    let (doc, o, spec) = chamfer_verticals_over(
        BlendProfile::Round {
            radius: 0.05,
            segments: 6,
        },
        0.3,
    );
    assert_eq!(
        doc.object(o).unwrap().max_blend_size(&spec).unwrap().max,
        0.3
    );
}

/// Over a 0.2 chamfered rim, chamfered verticals build at 0.2 alone (they
/// merge with the rim) and again from 0.4025 to 0.4975: the limit is the
/// top of the larger window, not the single size where they merge; asked
/// between the two, the merge size is the largest that builds.
#[test]
fn the_limit_is_not_held_at_an_equal_size_merge() {
    let (doc, o, spec) = chamfer_verticals_over(chamfer(0.2), 2.0);
    let obj = doc.object(o).unwrap();
    let limit = obj.max_blend_size(&spec).unwrap().max;
    assert!((0.4975..=0.5).contains(&limit), "{limit}");
    assert!(obj.blend_edges(&sized(&spec, limit)).is_ok());
    assert_too_large(obj.blend_edges(&spec), limit);

    let between = sized(&spec, 0.3);
    let limit = obj.max_blend_size(&between).unwrap().max;
    assert!((limit - 0.2).abs() < 1e-6, "{limit}");
    assert!(obj.blend_edges(&sized(&spec, limit)).is_ok());
}

/// A clamped preview takes the limit its refusal names: one search, and
/// the preview's limit is the commit's refusal's.
#[test]
fn a_clamped_preview_searches_for_its_limit_once() {
    let (doc, o) = rim_box(0.05);
    let spec = vertical_spec(&doc, o, 0.05, 0.6);
    let before = kernel::limit_searches_for_test();
    let preview = doc.blend_preview(o, &spec).unwrap();
    assert_eq!(kernel::limit_searches_for_test() - before, 1, "one search");
    let limit = preview.clamped.expect("clamped");
    match doc.object(o).unwrap().blend_edges(&spec) {
        Err(BlendError::RadiusTooLarge { max, .. }) => assert_eq!(max, limit.max),
        other => panic!("{other:?}"),
    }
}

/// The re-solve's refusal replaces the planner's only where the planner
/// refused the corner outright (`UnsupportedVertex`) and a smaller size
/// builds; every other refusal at a corner is the plain blend's, as before.
#[test]
fn corner_refusals_keep_their_kind() {
    let (doc, o) = rim_box(0.05);
    let obj = doc.object(o).unwrap();

    // A tangent rail of the rim fillet: smooth, whatever the size.
    let rail = obj
        .edges()
        .keys()
        .find(|&e| obj.edge_smoothness(e) == kernel::EdgeSmoothness::Tangent)
        .expect("a tangent rail");
    let mut smooth = round(0.01);
    smooth.edges = vec![rail];
    assert!(matches!(
        obj.blend_edges(&smooth),
        Err(BlendError::EdgeIsSmooth)
    ));

    // A segment count out of range at a corner that re-solves.
    let mut bad = vertical_spec(&doc, o, 0.05, 0.6);
    bad.profile = BlendProfile::Round {
        radius: 0.6,
        segments: 0,
    };
    assert!(matches!(
        obj.blend_edges(&bad),
        Err(BlendError::SegmentsOutOfRange { .. })
    ));
    assert!(matches!(
        doc.blend_preview(o, &bad),
        Err(DocumentError::Blend(BlendError::SegmentsOutOfRange { .. }))
    ));

    // Verticals smaller than a large rim: no size builds that corner, so
    // the planner's refusal stands, on the commit and the preview.
    let (doc, o) = rim_box(0.3);
    let small = vertical_spec(&doc, o, 0.3, 0.1);
    assert!(matches!(
        doc.object(o).unwrap().blend_edges(&small),
        Err(BlendError::UnsupportedVertex { .. })
    ));
    assert!(matches!(
        doc.blend_preview(o, &small),
        Err(DocumentError::Blend(BlendError::UnsupportedVertex { .. }))
    ));
}

/// A clamped preview of a variable-radius blend is what the commit makes at
/// the size it shows: the handles scale with the size, as the limit search
/// scaled them. Before, the clamp scaled the profile alone.
#[test]
fn clamped_previews_scale_radius_handles_as_the_commit() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut spec = round(0.05);
    spec.edges = vec![front_top(&doc, o)];
    spec.handles = vec![kernel::RadiusHandle {
        at: 0.3,
        radius: 0.03,
    }];
    let (report, _) = doc.blend_edges(o, &spec).unwrap();
    let id = report.blends[0];

    let preview = doc
        .resize_blend_preview(
            o,
            id,
            &BlendProfile::Round {
                radius: 1.5,
                segments: 6,
            },
        )
        .unwrap();
    assert!(preview.clamped.is_some());
    let mut committed = doc.clone();
    committed.resize_blend(o, id, &preview.applied).unwrap();
    assert!(
        kernel::same_geometry_for_test(&preview.after, committed.object(o).unwrap(), 1e-9),
        "the clamped resize preview is the resize at the size it shows"
    );

    // A fresh fillet, clamped: the commit at the limit with the handles
    // scaled by the same factor.
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut spec = round(2.0);
    spec.edges = vec![front_top(&doc, o)];
    spec.handles = vec![kernel::RadiusHandle {
        at: 0.3,
        radius: 1.2,
    }];
    let preview = doc.blend_preview(o, &spec).unwrap();
    let limit = preview.clamped.expect("clamped").max;
    let mut at_limit = sized(&spec, limit);
    for h in &mut at_limit.handles {
        h.radius *= limit / 2.0;
    }
    let mut committed = doc.clone();
    committed.blend_edges(o, &at_limit).unwrap();
    assert!(kernel::same_geometry_for_test(
        &preview.after,
        committed.object(o).unwrap(),
        1e-9
    ));
}

/// The edge of `obj` starting at `from` and running along `dir`.
fn edge_from(obj: &kernel::Object, from: Point3, dir: Vec3) -> EdgeId {
    obj.edges()
        .keys()
        .find(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            let on = |x: Point3| {
                let d = x - from;
                (d - dir * d.dot(dir)).length() < 1e-9
            };
            on(p) && on(q) && ((p - from).length() < 1e-9 || (q - from).length() < 1e-9)
        })
        .expect("an edge from that point")
}

/// Resizing a stopped blend that a later fillet ran into puts that fillet
/// back in the preview as the commit does. Before, the preview kept no
/// record of the blend it had just made, putting the later fillet back
/// could not restore it, and the preview failed `DependentBlend`.
#[test]
fn resize_preview_of_a_stopped_blend_puts_the_later_fillet_back() {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut stopped = round(0.05);
    stopped.edges = vec![front_top(&doc, o)];
    stopped.stops = Some(kernel::BlendStops {
        start: 0.0,
        end: 0.3,
    });
    let (report, _) = doc.blend_edges(o, &stopped).unwrap();
    let id = report.blends[0];
    let obj = doc.object(o).unwrap();
    let mut later = round(0.04);
    later.edges = vec![edge_from(obj, p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))];
    doc.blend_edges(o, &later).unwrap();

    for size in [0.03, 0.08] {
        let profile = BlendProfile::Round {
            radius: size,
            segments: 6,
        };
        let preview = doc.resize_blend_preview(o, id, &profile).unwrap();
        let mut committed = doc.clone();
        committed.resize_blend(o, id, &profile).unwrap();
        assert!(kernel::same_geometry_for_test(
            &preview.after,
            committed.object(o).unwrap(),
            1e-9
        ));
    }
}

/// Resizing a blend another blend was put back over marks only the
/// resized blend's faces, as the commit reports them, and the preview
/// carries the commit's blend ids: the resized blend keeps its id and the
/// one put back keeps its own. Before, the preview's resized blend took a
/// fresh id equal to the put-back rim's, so the rim's faces counted as new.
#[test]
fn resize_preview_marks_the_faces_the_commit_reports() {
    let (doc, o) = rim_then_verticals(0.05, 0.2);
    let verticals = blend_sized(&doc, o, 0.2);
    let bigger = BlendProfile::Round {
        radius: 0.3,
        segments: 6,
    };
    let preview = doc.resize_blend_preview(o, verticals, &bigger).unwrap();
    assert!(preview.clamped.is_none());
    let mut committed = doc.clone();
    let (report, _) = committed.resize_blend(o, verticals, &bigger).unwrap();
    let after = committed.object(o).unwrap();
    assert!(kernel::same_geometry_for_test(&preview.after, after, 1e-9));

    let mut shown = preview.new_faces.clone();
    shown.sort();
    let mut made = report.new_faces.clone();
    made.sort();
    assert_eq!(shown, made, "the preview marks the resized blend only");
    let tags = |obj: &kernel::Object| {
        let mut t: Vec<_> = obj
            .faces()
            .iter()
            .filter_map(|(f, face)| face.blend.map(|b| (f, b.id)))
            .collect();
        t.sort();
        t
    };
    assert_eq!(tags(&preview.after), tags(after), "the commit's blend ids");
}

/// Blend tags of `obj`: how many faces each blend id has.
fn tag_counts(obj: &kernel::Object) -> std::collections::BTreeMap<kernel::BlendId, usize> {
    let mut m = std::collections::BTreeMap::new();
    for face in obj.faces().values() {
        if let Some(t) = face.blend {
            *m.entry(t.id).or_insert(0) += 1;
        }
    }
    m
}

/// The face counts per blend, without the ids (a fresh fillet takes a
/// fresh id where a resize keeps its own).
fn tag_sizes(obj: &kernel::Object) -> Vec<usize> {
    let mut v: Vec<usize> = tag_counts(obj).into_values().collect();
    v.sort();
    v
}

/// A resize re-solves corners as a fresh fillet does (editing a size is
/// removing the blend and filleting again at the new size): the rim over
/// verticals is limited where a fresh fillet of its edges on the box
/// without it is, and at the verticals' own size it merges with them into
/// one blend, as a fresh fillet does. Before, a resize re-blended in place
/// against the verticals and refused their size.
#[test]
fn resize_preview_clamps_to_a_fresh_fillets_limit() {
    let (doc, o) = rim_then_verticals(0.3, 0.35);
    let rim = blend_sized(&doc, o, 0.3);
    let too_big = BlendProfile::Round {
        radius: 0.6,
        segments: 6,
    };
    let preview = doc.resize_blend_preview(o, rim, &too_big).unwrap();
    let limit = preview.clamped.expect("clamped").max;

    let (removed, edges) = doc.object(o).unwrap().remove_blend_for_test(rim).unwrap();
    let mut fresh = round(0.6);
    fresh.edges = edges;
    let fresh_limit = removed.max_blend_size(&fresh).unwrap().max;
    assert!(
        (limit - fresh_limit).abs() <= 1e-9,
        "resize limit {limit}, fresh fillet's {fresh_limit}"
    );
    assert_doc_too_large(doc.clone().resize_blend(o, rim, &too_big), limit);

    let at = BlendProfile::Round {
        radius: limit,
        segments: 6,
    };
    let mut committed = doc.clone();
    committed.resize_blend(o, rim, &at).unwrap();
    assert!(kernel::same_geometry_for_test(
        &preview.after,
        committed.object(o).unwrap(),
        1e-9
    ));
    let past = BlendProfile::Round {
        radius: limit + PAST_LIMIT,
        segments: 6,
    };
    assert_doc_too_large(doc.clone().resize_blend(o, rim, &past), limit);

    // At the verticals' size: one blend, as a fresh fillet makes it.
    let equal = BlendProfile::Round {
        radius: 0.35,
        segments: 6,
    };
    let mut resized = doc.clone();
    resized.resize_blend(o, rim, &equal).unwrap();
    let mut fresh = round(0.35);
    fresh.edges = doc.object(o).unwrap().remove_blend_for_test(rim).unwrap().1;
    let (fresh_after, _) = removed.blend_edges(&fresh).unwrap();
    let after = resized.object(o).unwrap();
    assert_eq!(tag_counts(after).len(), 1, "one blend over all the faces");
    assert!(kernel::same_geometry_for_test(after, &fresh_after, 1e-9));
    assert_eq!(tag_sizes(after), tag_sizes(&fresh_after));
}

/// A resize gives what removing the blend and filleting its edges again
/// gives, keeping its own id; where that fresh fillet refuses, the resize
/// refuses with the same limit.
#[test]
fn resize_equals_remove_then_fresh_blend() {
    let (doc, o) = rim_then_verticals(0.3, 0.35);
    let rim = blend_sized(&doc, o, 0.3);
    let obj = doc.object(o).unwrap();
    let (removed, edges) = obj.remove_blend_for_test(rim).unwrap();
    for size in [0.2, 0.45] {
        let profile = BlendProfile::Round {
            radius: size,
            segments: 6,
        };
        let mut fresh = round(size);
        fresh.edges = edges.clone();
        let resized = obj.resize_blend_for_test(rim, &profile);
        match (resized, removed.blend_edges(&fresh)) {
            (Ok((a, _)), Ok((b, _))) => {
                assert!(kernel::same_geometry_for_test(&a, &b, 1e-9), "{size}");
                assert_eq!(tag_sizes(&a), tag_sizes(&b));
                assert!(tag_counts(&a).contains_key(&rim), "the resize keeps its id");
            }
            (
                Err(BlendError::RadiusTooLarge { max: a, .. }),
                Err(BlendError::RadiusTooLarge { max: b, .. }),
            ) => assert!((a - b).abs() <= 1e-9, "{size}: {a} vs {b}"),
            (a, b) => panic!("{size}: resize {:?}, fresh {:?}", a.err(), b.err()),
        }
    }
}

/// Three top-edge fillets: the right one (0.3) first, then the front one
/// (0.2) running into it, then the left one (0.1) running into the front
/// one. The front fillet has a fillet put back after it (the left) and
/// touches one made before it (the right).
fn three_top_fillets() -> (Document, ObjectId) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    for (size, from, dir) in [
        (0.3, p3(1.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
        (0.2, p3(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)),
        (0.1, p3(0.0, 1.0, 1.0), Vec3::new(0.0, -1.0, 0.0)),
    ] {
        let mut spec = round(size);
        spec.edges = vec![edge_from(doc.object(o).unwrap(), from, dir)];
        doc.blend_edges(o, &spec).unwrap();
    }
    (doc, o)
}

/// The edges of `obj` lying on the line through `from` along `dir`.
fn edges_along(obj: &kernel::Object, from: Point3, dir: Vec3) -> Vec<EdgeId> {
    obj.edges()
        .keys()
        .filter(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            let on = |x: Point3| {
                let d = x - from;
                (d - dir * d.dot(dir)).length() < 1e-9
            };
            on(p) && on(q)
        })
        .collect()
}

/// Resizing a blend with a fillet put back after it and a neighbour made
/// before it re-solves the corner with the neighbour, then puts the later
/// fillet back, the preview showing the commit: what removing the blend and
/// filleting its edge again gives. At the neighbour's size the two merge
/// into one blend, as a fresh fillet merges them (a resize in place
/// refused that size).
#[test]
fn resize_with_a_later_fillet_and_a_neighbour_matches_a_fresh_fillet() {
    let (doc, o) = three_top_fillets();
    let front = blend_sized(&doc, o, 0.2);
    let right = blend_sized(&doc, o, 0.3);
    let left = blend_sized(&doc, o, 0.1);
    for size in [0.25, 0.3] {
        let profile = BlendProfile::Round {
            radius: size,
            segments: 6,
        };
        let preview = doc.resize_blend_preview(o, front, &profile).unwrap();
        assert!(preview.clamped.is_none(), "{size}");
        let mut resized = doc.clone();
        let (report, _) = resized.resize_blend(o, front, &profile).unwrap();
        let after = resized.object(o).unwrap();
        assert!(kernel::same_geometry_for_test(&preview.after, after, 1e-9));
        let mut shown = preview.new_faces.clone();
        shown.sort();
        let mut made = report.new_faces.clone();
        made.sort();
        assert_eq!(shown, made, "{size}");
        let tags = tag_counts(after);
        assert!(tags.contains_key(&left), "the later fillet keeps its id");
        if size < 0.3 {
            assert!(tags.contains_key(&front) && tags.contains_key(&right));
            let mut fresh_doc = doc.clone();
            fresh_doc.remove_blend(o, front).unwrap();
            let mut fresh = round(size);
            fresh.edges = edges_along(
                fresh_doc.object(o).unwrap(),
                p3(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 0.0),
            );
            fresh_doc.blend_edges(o, &fresh).unwrap();
            let fresh_after = fresh_doc.object(o).unwrap();
            assert!(kernel::same_geometry_for_test(after, fresh_after, 1e-9));
            assert_eq!(tag_sizes(after), tag_sizes(fresh_after));
        } else {
            assert!(
                !tags.contains_key(&front) && !tags.contains_key(&right),
                "front and right merge under a fresh id: {tags:?}"
            );
            assert_eq!(tags.len(), 2, "{tags:?}");
        }
    }
}

/// A unit box with its first `n` vertical edges rounded at 0.35: the
/// right-front one, then the left-front and the two back ones.
fn box_with_verticals(n: usize) -> (Document, ObjectId) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let mut spec = round(0.35);
    spec.edges = [(1.0, 0.0), (0.0, 0.0), (1.0, 1.0), (0.0, 1.0)][..n]
        .iter()
        .map(|&(x, y)| edge_between(doc.object(o).unwrap(), p3(x, y, 0.0), p3(x, y, 1.0)))
        .collect();
    doc.blend_edges(o, &spec).unwrap();
    (doc, o)
}

/// `edges` of `obj` filleted fresh at `size`.
fn fresh_fillet(obj: &kernel::Object, edges: &[EdgeId], size: f64) -> kernel::Object {
    let mut s = round(size);
    s.edges = edges.to_vec();
    obj.blend_edges(&s).unwrap().0
}

/// A fillet made on the front-top edge of a box whose right-front vertical
/// is rounded follows the tangent run through the vertical's end arc to
/// the right-top edge, so its record holds that run (the line, six arc
/// chords and the right edge, 48 faces). Resizing it resizes the run, as a
/// fresh fillet of those edges: at the vertical's size it merges with it
/// into one blend (not what filleting the line alone at that size gives,
/// which merges first and never wraps the arc), and below it the line alone
/// wraps the same run. With all four verticals rounded the run is the whole
/// top loop, which resizes the same way.
#[test]
fn resize_of_a_fillet_that_wrapped_a_neighbour_keeps_its_run() {
    let (mut doc, o) = box_with_verticals(1);
    let line = edge_from(
        doc.object(o).unwrap(),
        p3(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    );
    let mut spec = round(0.3);
    spec.edges = vec![line];
    doc.blend_edges(o, &spec).unwrap();
    let id = blend_sized(&doc, o, 0.3);
    let made = doc.object(o).unwrap();
    assert_eq!(tag_counts(made)[&id], 48, "the run, six faces an edge");

    let (removed, run) = made.remove_blend_for_test(id).unwrap();
    assert_eq!(run.len(), 8, "the line, six arc chords and the right edge");
    let right = edge_from(&removed, p3(1.0, 0.35, 1.0), Vec3::new(0.0, 1.0, 0.0));
    assert!(run.contains(&right), "the run reaches the right edge");
    let line = edge_from(&removed, p3(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0));
    let resize = |size: f64| {
        made.resize_blend_for_test(
            id,
            &BlendProfile::Round {
                radius: size,
                segments: 6,
            },
        )
        .unwrap()
        .0
    };

    let at_35 = resize(0.35);
    let run_35 = fresh_fillet(&removed, &run, 0.35);
    assert!(kernel::same_geometry_for_test(&at_35, &run_35, 1e-9));
    assert_eq!(tag_counts(&at_35).len(), 1, "one blend with the vertical");
    assert_eq!(tag_sizes(&at_35), tag_sizes(&run_35));
    assert!(!kernel::same_geometry_for_test(
        &at_35,
        &fresh_fillet(&removed, &[line], 0.35),
        1e-9
    ));

    let at_20 = resize(0.2);
    let line_20 = fresh_fillet(&removed, &[line], 0.2);
    assert!(kernel::same_geometry_for_test(&at_20, &line_20, 1e-9));
    assert_eq!(tag_sizes(&at_20), tag_sizes(&line_20));

    // Four verticals: the run is the whole top loop.
    let (mut doc, o) = box_with_verticals(4);
    let line = edge_from(
        doc.object(o).unwrap(),
        p3(0.35, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    );
    let mut spec = round(0.3);
    spec.edges = vec![line];
    doc.blend_edges(o, &spec).unwrap();
    let id = blend_sized(&doc, o, 0.3);
    let made = doc.object(o).unwrap();
    let (removed, run) = made.remove_blend_for_test(id).unwrap();
    assert_eq!(run.len(), 28, "four lines and four arcs of six chords");
    assert_eq!(tag_counts(made)[&id], 6 * run.len());
    for size in [0.2, 0.35] {
        let resized = made
            .resize_blend_for_test(
                id,
                &BlendProfile::Round {
                    radius: size,
                    segments: 6,
                },
            )
            .unwrap()
            .0;
        let fresh = fresh_fillet(&removed, &run, size);
        assert!(
            kernel::same_geometry_for_test(&resized, &fresh, 1e-9),
            "{size}"
        );
        assert_eq!(tag_sizes(&resized), tag_sizes(&fresh), "{size}");
    }
}

/// Builds a document with an object to edit.
type Setup = fn() -> (Document, ObjectId);

/// A resize that re-solves a corner undoes and redoes bit-exact.
#[test]
fn resize_then_undo_restores_the_state() {
    let cases: [(Setup, f64, f64); 3] = [
        (three_top_fillets, 0.2, 0.25),
        (three_top_fillets, 0.2, 0.3),
        (|| rim_then_verticals(0.3, 0.35), 0.3, 0.35),
    ];
    for (make, from, to) in cases {
        let (mut doc, o) = make();
        let id = blend_sized(&doc, o, from);
        let before = doc.state_hash();
        doc.resize_blend(
            o,
            id,
            &BlendProfile::Round {
                radius: to,
                segments: 6,
            },
        )
        .unwrap();
        let after = doc.state_hash();
        for _ in 0..2 {
            doc.undo().unwrap();
            assert_eq!(doc.state_hash(), before, "{from} -> {to}");
            doc.redo().unwrap();
            assert_eq!(doc.state_hash(), after, "{from} -> {to}");
        }
    }
}
