//! True Curves marks across the operations that link, clone or re-home an
//! Object: a shape's blend records, crease flags and Smooth level are part
//! of what it is. "Link identical copies" never joins two look-alikes that
//! differ in one of them (the shared solid would lose it), and every
//! operation that copies or moves an Object into or out of a definition
//! keeps all three.

use kernel::sketch::{CurveGeom, SketchCurveKind};
use kernel::trig;
use kernel::{
    BlendProfile, BlendSpec, CornerStyle, Document, DocumentError, EdgeId, NodeId, ObjectId, Plane,
    Point3, SurfaceRef, Transform, Vec3,
};

// ------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

/// A 1 m board with its corner at `(x, 0, 0)`.
fn board(doc: &mut Document, x: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let c = [(x, 0.0), (x + 1.0, 0.0), (x + 1.0, 1.0), (x, 1.0)];
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

/// The edge of `o` running along `y = y0`, `z = z0` (one of a board's four
/// edges parallel to x).
fn edge_along_x(doc: &Document, o: ObjectId, y0: f64, z0: f64) -> EdgeId {
    let obj = doc.object(o).unwrap();
    obj.edges()
        .keys()
        .find(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            (a.y - y0).abs() < 1e-9
                && (b.y - y0).abs() < 1e-9
                && (a.z - z0).abs() < 1e-9
                && (b.z - z0).abs() < 1e-9
        })
        .expect("the board has that edge")
}

fn fillet(doc: &mut Document, o: ObjectId) {
    let e = edge_along_x(doc, o, 0.0, 1.0);
    doc.blend_edges(
        o,
        &BlendSpec {
            edges: vec![e],
            profile: BlendProfile::Round {
                radius: 0.1,
                segments: 4,
            },
            primary: None,
            corners: CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        },
    )
    .unwrap();
}

fn crease(doc: &mut Document, o: ObjectId) {
    let e = edge_along_x(doc, o, 1.0, 0.0);
    doc.set_crease(o, &[e], true).unwrap();
}

/// What an Object carries: how many of its blends have a record, how many
/// edges are creased, its Smooth level.
fn marks(doc: &Document, o: ObjectId) -> (usize, usize, u8) {
    let obj = doc.object(o).unwrap();
    let mut blends: Vec<_> = obj
        .faces()
        .values()
        .filter_map(|f| f.blend)
        .map(|t| t.id)
        .collect();
    blends.sort();
    blends.dedup();
    let records = blends
        .iter()
        .filter(|&&b| obj.blend_record_digest(b).is_some())
        .count();
    let creases = obj.edges().values().filter(|e| e.crease).count();
    (records, creases, doc.smooth_level(o))
}

/// The fully marked board: a filleted edge with its record, a creased
/// edge, Smooth level 1.
fn marked_board(doc: &mut Document, x: f64) -> ObjectId {
    let o = board(doc, x);
    fillet(doc, o);
    crease(doc, o);
    doc.set_smooth_level(o, 1).unwrap();
    assert_eq!(marks(doc, o), (1, 1, 1));
    o
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

/// The member objects of the definition `instance` places.
fn members(doc: &Document, instance: kernel::InstanceId) -> Vec<ObjectId> {
    let def = doc.instance_def(instance).unwrap();
    doc.def_member_nodes(def)
        .unwrap()
        .into_iter()
        .filter_map(|n| match n {
            NodeId::Object(o) => Some(o),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------- A. link identical copies

#[test]
fn look_alikes_that_differ_in_a_record_a_crease_or_a_smooth_level_do_not_link() {
    // A record: both filleted, but a reflection there and back drops the
    // second one's record (the tags stay, the geometry is the same).
    let mut doc = Document::new();
    let keeper = board(&mut doc, 0.0);
    fillet(&mut doc, keeper);
    let joiner = board(&mut doc, 3.0);
    fillet(&mut doc, joiner);
    let mirror =
        Transform::reflection(Point3::new(3.5, 0.5, 0.5), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    doc.transform_object(joiner, &mirror).unwrap();
    doc.transform_object(joiner, &mirror).unwrap();
    assert_eq!(
        marks(&doc, joiner).0,
        0,
        "the reflection dropped the record"
    );
    assert_eq!(marks(&doc, keeper).0, 1);
    let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
    let before = doc.state_hash();
    assert_eq!(
        doc.join_linked(set, joiner, shift(3.0)).unwrap_err(),
        DocumentError::NotIdentical
    );
    assert_eq!(doc.state_hash(), before, "untouched");

    // A crease.
    let mut doc = Document::new();
    let keeper = board(&mut doc, 0.0);
    let joiner = board(&mut doc, 3.0);
    crease(&mut doc, joiner);
    let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
    assert_eq!(
        doc.join_linked(set, joiner, shift(3.0)).unwrap_err(),
        DocumentError::NotIdentical
    );

    // A Smooth level.
    let mut doc = Document::new();
    let keeper = board(&mut doc, 0.0);
    let joiner = board(&mut doc, 3.0);
    doc.set_smooth_level(joiner, 2).unwrap();
    let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
    assert_eq!(
        doc.join_linked(set, joiner, shift(3.0)).unwrap_err(),
        DocumentError::NotIdentical
    );
}

#[test]
fn identically_marked_copies_link_and_keep_their_marks() {
    let mut doc = Document::new();
    let keeper = marked_board(&mut doc, 0.0);
    let joiner = marked_board(&mut doc, 3.0);
    let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
    let (join, _) = doc.join_linked(set, joiner, shift(3.0)).unwrap();
    let shared = members(&doc, join.instance);
    assert_eq!(shared, members(&doc, set));
    assert_eq!(
        marks(&doc, shared[0]),
        (1, 1, 1),
        "the shared solid keeps them"
    );
}

/// An `n`-gon of circumradius `r` about `center` on a horizontal plane,
/// drawn on `w` as one curve chain of `kind` (a circle's facets or a
/// regular polygon's sides), or as plain segments through the same corners
/// when `kind` is `None`.
fn ngon(
    w: &mut kernel::Sketch,
    center: Point3,
    r: f64,
    n: usize,
    kind: Option<SketchCurveKind>,
) -> Result<(), kernel::SketchError> {
    let at = |k: usize| {
        let a = std::f64::consts::TAU * k as f64 / n as f64;
        Point3::new(
            center.x + r * trig::cos(a),
            center.y + r * trig::sin(a),
            center.z,
        )
    };
    if let Some(kind) = kind {
        w.begin_curve_with_kind(CurveGeom { center, radius: r }, kind)?;
    }
    for k in 0..n {
        w.add_segment(at(k), at(k + 1))?;
    }
    if kind.is_some() {
        w.end_curve();
    }
    Ok(())
}

/// A 24-sided prism 1 m tall on a 0.5 m circumcircle centered at
/// `(x, 0, 0)`: extruded from a drawn circle (cylinder-claimed walls) when
/// `circle`, else from plain segments through the same corners (no claim).
fn prism(doc: &mut Document, x: f64, circle: bool) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    let kind = circle.then_some(SketchCurveKind::Circle);
    ngon(
        doc.sketch_mut(s).unwrap(),
        Point3::new(x, 0.0, 0.0),
        0.5,
        24,
        kind,
    )
    .unwrap();
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

/// A board with an `n`-gon drawn on its top face: as a circle
/// (circle-claimed edges), a regular polygon (polygon-claimed edges), or
/// plain segments (no claim).
fn ringed_board(doc: &mut Document, x: f64, n: usize, kind: Option<SketchCurveKind>) -> ObjectId {
    let o = board(doc, x);
    let top =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    doc.draft(top, None, &[o], "Ring", |w| {
        ngon(w, Point3::new(x + 0.5, 0.5, 1.0), 0.3, n, kind)
    })
    .unwrap();
    o
}

/// What analytic claims an Object carries: cylinder-claimed faces,
/// circle-claimed edges, polygon-claimed edges.
fn claims(doc: &Document, o: ObjectId) -> (usize, usize, usize) {
    let obj = doc.object(o).unwrap();
    let cylinders = obj
        .faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
        .count();
    let circles = obj.edges().values().filter(|e| e.curve.is_some()).count();
    let polygons = obj.edges().values().filter(|e| e.polygon.is_some()).count();
    (cylinders, circles, polygons)
}

/// Links `joiner` into a set wrapped around `keeper` at `pose` and
/// expects the refusal, with the document untouched.
fn assert_not_identical(doc: &mut Document, keeper: ObjectId, joiner: ObjectId, pose: Transform) {
    let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
    let before = doc.state_hash();
    assert_eq!(
        doc.join_linked(set, joiner, pose).unwrap_err(),
        DocumentError::NotIdentical
    );
    assert_eq!(doc.state_hash(), before, "untouched");
}

#[test]
fn look_alikes_that_differ_in_a_cylinder_claim_do_not_link() {
    // Either way round: the claimed shape kept and the plain one joining
    // (it would gain claims it never had), and the plain one kept and the
    // claimed one joining (the shared solid would lose them).
    for claimed_keeps in [true, false] {
        let mut doc = Document::new();
        let claimed = prism(&mut doc, 0.0, true);
        let plain = prism(&mut doc, 3.0, false);
        assert_eq!(claims(&doc, claimed).0, 24, "the drawn circle's walls");
        assert_eq!(claims(&doc, plain), (0, 0, 0));
        if claimed_keeps {
            assert_not_identical(&mut doc, claimed, plain, shift(3.0));
        } else {
            assert_not_identical(&mut doc, plain, claimed, shift(-3.0));
        }
    }
}

#[test]
fn look_alikes_that_differ_in_a_circle_or_polygon_edge_claim_do_not_link() {
    for (n, kind) in [(24, SketchCurveKind::Circle), (6, SketchCurveKind::Polygon)] {
        for claimed_keeps in [true, false] {
            let mut doc = Document::new();
            let claimed = ringed_board(&mut doc, 0.0, n, Some(kind));
            let plain = ringed_board(&mut doc, 3.0, n, None);
            let (_, circles, polygons) = claims(&doc, claimed);
            match kind {
                SketchCurveKind::Circle => assert!(circles > 0 && polygons == 0),
                SketchCurveKind::Polygon => assert!(polygons > 0 && circles == 0),
            }
            assert_eq!(claims(&doc, plain), (0, 0, 0), "{kind:?}");
            if claimed_keeps {
                assert_not_identical(&mut doc, claimed, plain, shift(3.0));
            } else {
                assert_not_identical(&mut doc, plain, claimed, shift(-3.0));
            }
        }
    }
}

#[test]
fn identical_drawn_circles_and_polygons_link_and_keep_their_claims() {
    type Make = fn(&mut Document, f64) -> ObjectId;
    let makers: [(&str, Make); 3] = [
        ("extruded circle", |d, x| prism(d, x, true)),
        ("circle on a face", |d, x| {
            ringed_board(d, x, 24, Some(SketchCurveKind::Circle))
        }),
        ("polygon on a face", |d, x| {
            ringed_board(d, x, 6, Some(SketchCurveKind::Polygon))
        }),
    ];
    for (what, make) in makers {
        let mut doc = Document::new();
        let keeper = make(&mut doc, 0.0);
        let joiner = make(&mut doc, 3.0);
        let want = claims(&doc, keeper);
        assert_ne!(want, (0, 0, 0), "{what}");
        assert_eq!(claims(&doc, joiner), want, "{what}");
        let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
        let (join, _) = doc
            .join_linked(set, joiner, shift(3.0))
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let shared = members(&doc, join.instance);
        assert_eq!(
            claims(&doc, shared[0]),
            want,
            "{what}: the shared solid keeps them"
        );
    }
    // A quarter turn about the drawn circle's own axis lands the same
    // corners (24 sides) and the same claims: it links too.
    let mut doc = Document::new();
    let keeper = prism(&mut doc, 0.0, true);
    let joiner = prism(&mut doc, 3.0, true);
    let (_, set, _) = doc.wrap_linked(NodeId::Object(keeper)).unwrap();
    let quarter =
        Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2).unwrap();
    doc.join_linked(set, joiner, quarter.then(&shift(3.0)))
        .unwrap();
}

// ------------------------------- B. clone and re-home keep every mark

#[test]
fn wrap_linked_and_make_component_keep_the_marks() {
    let mut doc = Document::new();
    let o = marked_board(&mut doc, 0.0);
    let (_, set, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    for m in members(&doc, set) {
        assert_eq!(marks(&doc, m), (1, 1, 1), "wrap_linked");
    }

    let mut doc = Document::new();
    let o = marked_board(&mut doc, 0.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    for m in members(&doc, inst) {
        assert_eq!(marks(&doc, m), (1, 1, 1), "make_component");
    }
}

#[test]
fn unlink_explode_and_make_unique_keep_the_marks() {
    // Unlink (explode one placement of a linked set into its own solid).
    let mut doc = Document::new();
    let o = marked_board(&mut doc, 0.0);
    let (_, set, _) = doc.wrap_linked(NodeId::Object(o)).unwrap();
    let (copies, _) = doc.explode_instance(set).unwrap();
    assert!(!copies.is_empty());
    for c in copies {
        assert_eq!(marks(&doc, c), (1, 1, 1), "explode / unlink");
    }

    // Make Unique: a private copy of the definition.
    let mut doc = Document::new();
    let o = marked_board(&mut doc, 0.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let second = doc
        .place_instance(doc.instance_def(inst).unwrap(), shift(3.0))
        .unwrap()
        .0;
    doc.make_unique(second).unwrap();
    for m in members(&doc, second) {
        assert_eq!(marks(&doc, m), (1, 1, 1), "make_unique");
    }
}

#[test]
fn a_mirror_keeps_creases_and_smooth_and_drops_only_the_record() {
    let mut doc = Document::new();
    let o = marked_board(&mut doc, 0.0);
    let mirror =
        Transform::reflection(Point3::new(0.5, 0.5, 0.5), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    doc.transform_object(o, &mirror).unwrap();
    assert_eq!(marks(&doc, o), (0, 1, 1));
    // The blend still edits, by derivation from its tagged faces.
    let obj = doc.object(o).unwrap();
    let id = obj.faces().values().find_map(|f| f.blend).unwrap().id;
    doc.remove_blend(o, id).unwrap();
}

#[test]
fn rejoin_keeps_the_largest_pieces_crease_and_smooth_level() {
    use kernel::{ImportNode, ImportScene, MeshRecipe};
    let p = Point3::new;
    // The box [x0, x1] × [0, 1] × [0, 1] without the faces in `drops`.
    let piece = |name: &str, x0: f64, x1: f64, drops: &[usize]| {
        let positions = vec![
            p(x0, 0.0, 0.0),
            p(x1, 0.0, 0.0),
            p(x1, 1.0, 0.0),
            p(x0, 1.0, 0.0),
            p(x0, 0.0, 1.0),
            p(x1, 0.0, 1.0),
            p(x1, 1.0, 1.0),
            p(x0, 1.0, 1.0),
        ];
        let faces: Vec<Vec<usize>> = [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ]
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !drops.contains(i))
        .map(|(_, f)| f)
        .collect();
        let n = faces.len();
        MeshRecipe {
            name: name.into(),
            positions,
            faces,
            face_materials: vec![kernel::NO_MATERIAL; n],
            face_uv_frames: vec![None; n],
            face_holes: vec![Vec::new(); n],
            base_material: kernel::NO_MATERIAL,
            tags: Vec::new(),
            extra: Default::default(),
        }
    };
    let mut doc = Document::new();
    doc.ingest(
        ImportScene {
            geo: None,
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![
                ImportNode::Mesh(piece("Left", 0.0, 1.5, &[3])),
                ImportNode::Mesh(piece("Right", 1.5, 2.0, &[5])),
            ],
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            extra: Default::default(),
        },
        Vec::new(),
    )
    .unwrap();
    let left = doc
        .live_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some("Left"))
        .unwrap();
    // The vertical edge at x = 0, y = 0: away from the cut.
    let obj = doc.object(left).unwrap();
    let e = obj
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            a.x.abs() < 1e-9 && b.x.abs() < 1e-9 && a.y.abs() < 1e-9 && b.y.abs() < 1e-9
        })
        .unwrap();
    doc.set_crease(left, &[e], true).unwrap();
    doc.set_smooth_level(left, 1).unwrap();
    let family = doc.health_report(None).families[0].members.clone();
    let (result, _) = doc.rejoin_as_one_solid(&family).unwrap();
    let (_, creases, smooth) = marks(&doc, result);
    assert_eq!((creases, smooth), (1, 1));
}
