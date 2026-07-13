//! Builds the bundled sample models through the public [`Document`] API —
//! the same operations the app performs — and writes them as `.hew` files.
//! Because the kernel is deterministic, regenerating produces byte-stable
//! output; the samples double as living exercises of the real modeling
//! paths (analytic curves, extrude, imprint + recess, components with
//! rotated poses, groups, tags, textured materials, guides).
//!
//! Usage: `cargo run -p kernel --example build_samples -- <out-dir>`
//! (the repo's bundled copies live in `app/public/samples/`).
//!
//! Texture assets are CC0 color maps embedded at build time — see
//! `sample_assets/README.md` for provenance.
//!
//! Two hero models:
//! - `wall-clock.hew` — a finished showcase: an oak-rimmed clock built from
//!   true circles (smooth cylinder walls), twelve brass hour markers as
//!   rotated instances of one component, charcoal hands in a group, and a
//!   glass cover with per-material opacity on a toggleable tag.
//! - `cafe-table.hew` — a round café table: textured oak top on four walnut
//!   cylinder legs (one component, four poses), crossed stretchers (a second
//!   component), a printable terracotta pen cup resting on top, and
//!   construction guides on the leg axes.

use kernel::{
    CurveGeom, Document, FaceId, ImageFormat, KernelOp, KernelOpReport, Material, MaterialId,
    NodeId, ObjectId, Plane, Point3, Rgba8, SketchId, Texture, Transform, Vec3,
};

/// Oak color map (CC0, ambientCG Wood048 — sample_assets/README.md).
const OAK_JPG: &[u8] = include_bytes!("sample_assets/oak.jpg");
/// Walnut color map (CC0, ambientCG Wood062 — sample_assets/README.md).
const WALNUT_JPG: &[u8] = include_bytes!("sample_assets/walnut.jpg");

fn ground() -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-formed")
}

/// Segments per full turn for a drawn circle of `radius` meters — the app's
/// draw-time rule (app/src/tools/arcMath.ts `segmentsPerTurn`): the coarsest
/// count whose chord sagitta stays within 0.5 mm, rounded up to a multiple
/// of 4, clamped to [24, 96]. Mirrored here so the samples' curves carry
/// exactly the facet density the tools would have produced.
fn segments_per_turn(radius: f64) -> usize {
    let ratio = 5e-4 / radius;
    if ratio >= 1.0 {
        return 24;
    }
    let exact = std::f64::consts::PI / (1.0 - ratio).acos();
    let ceil4 = ((exact / 4.0).ceil() as usize) * 4;
    ceil4.clamp(24, 96)
}

/// CCW regular polygon on z = `z`, first vertex at angle 0.
fn circle_points(cx: f64, cy: f64, r: f64, n: usize, z: f64) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), z)
        })
        .collect()
}

/// Draw a closed loop into a sketch as plain segments (rectangles, boxes).
fn draw_loop(doc: &mut Document, sketch: SketchId, pts: &[Point3]) {
    let s = doc.sketch_mut(sketch).expect("sketch exists");
    for i in 0..pts.len() {
        s.add_segment(pts[i], pts[(i + 1) % pts.len()])
            .expect("sample loop segments are valid");
    }
}

/// Draw a TRUE circle: the facet loop committed as one analytic curve chain
/// (center + exact radius), the same bracket the Circle tool commits — so
/// the extruded walls render smooth and re-facet exactly on export.
fn draw_circle(doc: &mut Document, sketch: SketchId, cx: f64, cy: f64, r: f64) -> usize {
    let n = segments_per_turn(r);
    let pts = circle_points(cx, cy, r, n, 0.0);
    let s = doc.sketch_mut(sketch).expect("sketch exists");
    s.begin_curve_with(CurveGeom {
        center: Point3::new(cx, cy, 0.0),
        radius: r,
    })
    .expect("sample circle geometry is valid");
    for i in 0..n {
        s.add_segment(pts[i], pts[(i + 1) % n])
            .expect("sample circle segments are valid");
    }
    s.end_curve();
    n
}

/// Axis-aligned rectangle corners on the ground plane, CCW from (x0, y0).
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point3> {
    vec![
        Point3::new(x0, y0, 0.0),
        Point3::new(x1, y0, 0.0),
        Point3::new(x1, y1, 0.0),
        Point3::new(x0, y1, 0.0),
    ]
}

/// The face of `object` whose plane normal points +Z at the highest z —
/// the "top" a person would draw on. Assumes the object stands on or above
/// the ground plane; every sample here is a ground-up extrusion.
fn top_face(doc: &Document, object: ObjectId) -> FaceId {
    let obj = doc.object(object).expect("object exists");
    obj.faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().z > 0.9)
        .max_by(|(_, a), (_, b)| {
            let za = a.plane.signed_distance(Point3::new(0.0, 0.0, 0.0)).abs();
            let zb = b.plane.signed_distance(Point3::new(0.0, 0.0, 0.0)).abs();
            za.partial_cmp(&zb).expect("finite plane offsets")
        })
        .map(|(id, _)| id)
        .expect("a solid extrusion has a top face")
}

/// Extrude the single newest region of `sketch` by `distance`.
fn extrude_only_region(doc: &mut Document, sketch: SketchId, distance: f64) -> ObjectId {
    let regions = doc.extrudable_regions(sketch).expect("sketch exists");
    let region = *regions.last().expect("the drawn loop closed into a region");
    let (object, _) = doc
        .extrude_region(sketch, region, distance)
        .expect("sample profiles extrude cleanly");
    object
}

/// Imprint a true circle (`center` on the face plane, radius `r`) on `face`
/// of `object` with its analytic claim, and recess it by `depth`, returning
/// the recessed floor face.
fn recess_circle(
    doc: &mut Document,
    object: ObjectId,
    face: FaceId,
    center: Point3,
    r: f64,
    depth: f64,
) -> FaceId {
    let n = segments_per_turn(r);
    let (report, _) = doc
        .apply_object_op(
            object,
            KernelOp::SplitFaceInner {
                face,
                loop_path: circle_points(center.x, center.y, r, n, center.z),
                restore: None,
                curve: Some(CurveGeom { center, radius: r }),
            },
        )
        .expect("inner rim imprints");
    let KernelOpReport::FaceSplitInner(inner) = report else {
        panic!("SplitFaceInner reports FaceSplitInner");
    };
    doc.apply_object_op(
        object,
        KernelOp::ExtrudeSubFace {
            sub_face: inner.sub_face,
            distance: -depth,
        },
    )
    .expect("recess carves the solid");
    inner.sub_face
}

/// A pose rotating by `angle` radians about the vertical axis through
/// (`cx`, `cy`) — how the clock markers and table legs fan out from one
/// component definition.
fn rotate_about_z(cx: f64, cy: f64, angle: f64) -> Transform {
    let to_origin = Transform::translation(Vec3::new(-cx, -cy, 0.0));
    let rot = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), angle)
        .expect("vertical axis rotation is valid");
    let back = Transform::translation(Vec3::new(cx, cy, 0.0));
    to_origin.then(&rot).then(&back)
}

fn oak_material(doc: &mut Document) -> MaterialId {
    doc.add_material(Material::textured(
        "Oak",
        Rgba8 {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        },
        Texture {
            image: OAK_JPG.to_vec(),
            format: ImageFormat::Jpeg,
            world_size: [0.5, 0.5],
        },
    ))
}

fn walnut_material(doc: &mut Document) -> MaterialId {
    doc.add_material(Material::textured(
        "Walnut",
        Rgba8 {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        },
        Texture {
            image: WALNUT_JPG.to_vec(),
            format: ImageFormat::Jpeg,
            world_size: [0.4, 0.4],
        },
    ))
}

fn solid(doc: &mut Document, name: &str, r: u8, g: u8, b: u8) -> MaterialId {
    doc.add_material(Material::solid(name, Rgba8 { r, g, b, a: 255 }))
}

fn assert_watertight(doc: &Document, object: ObjectId, what: &str) {
    assert_eq!(
        doc.object(object).expect("object exists").watertight(),
        kernel::WatertightState::Watertight,
        "{what} must be a watertight solid"
    );
}

fn name_node(doc: &mut Document, node: NodeId, name: &str) {
    doc.set_node_name(node, Some(name.to_string()))
        .expect("sample nodes accept names");
}

// ------------------------------------------------------------- wall clock

/// A finished wall clock, lying face-up the way it would be modeled:
/// oak-textured rim (a true 160 mm-radius cylinder), a recessed cream face,
/// twelve brass hour markers as rotated instances of one component, charcoal
/// hands in a group, a brass center cap, and a translucent glass cover on a
/// toggleable "Glass" tag.
fn build_wall_clock() -> Document {
    let mut doc = Document::new();

    let oak = oak_material(&mut doc);
    let brass = solid(&mut doc, "Brass", 196, 158, 84);
    let cream = solid(&mut doc, "Clock Face", 244, 239, 226);
    let charcoal = solid(&mut doc, "Charcoal", 42, 42, 46);
    let glass = solid(&mut doc, "Glass", 202, 224, 233);
    doc.set_material_alpha(glass, 110)
        .expect("glass material accepts opacity");

    // Body: a true circle, extruded 35 mm — the rim renders as one smooth
    // cylinder wall (the facets carry their analytic identity).
    let body_sketch = doc.add_sketch(ground());
    draw_circle(&mut doc, body_sketch, 0.0, 0.0, 0.160);
    let body = extrude_only_region(&mut doc, body_sketch, 0.035);
    doc.set_object_material(body, Some(oak))
        .expect("body accepts a default material");
    name_node(&mut doc, NodeId::Object(body), "Clock Body");

    // Face: imprint a true inner circle on the top and recess it 6 mm; the
    // recessed floor is painted cream (per-face paint over the oak default).
    let top = top_face(&doc, body);
    let floor = recess_circle(
        &mut doc,
        body,
        top,
        Point3::new(0.0, 0.0, 0.035),
        0.140,
        0.006,
    );
    doc.paint_face(body, floor, Some(cream))
        .expect("clock face floor takes paint");
    assert_watertight(&doc, body, "the clock body");

    // Hour markers: one brass block component, twelve placements rotated
    // 30° apart about the clock center. The definition is authored at the
    // 12-o'clock position on the recessed face.
    let marker_sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, marker_sketch, &rect(-0.004, 0.106, 0.004, 0.130));
    let marker = extrude_only_region(&mut doc, marker_sketch, 0.0025);
    assert_watertight(&doc, marker, "an hour marker");
    doc.transform_object(marker, &Transform::translation(Vec3::new(0.0, 0.0, 0.029)))
        .expect("marker lifts onto the clock face");
    doc.set_object_material(marker, Some(brass))
        .expect("marker accepts a default material");
    name_node(&mut doc, NodeId::Object(marker), "Hour Marker");
    let (marker_def, first_marker, _) = doc
        .make_component(&[NodeId::Object(marker)])
        .expect("the marker becomes a component");
    name_node(&mut doc, NodeId::Instance(first_marker), "Marker 12");
    let mut marker_nodes = vec![NodeId::Instance(first_marker)];
    for hour in 1..12usize {
        let angle = -(std::f64::consts::TAU * hour as f64 / 12.0);
        let (inst, _) = doc
            .place_instance(marker_def, rotate_about_z(0.0, 0.0, angle))
            .expect("markers place at valid poses");
        name_node(&mut doc, NodeId::Instance(inst), &format!("Marker {hour}"));
        marker_nodes.push(NodeId::Instance(inst));
    }
    let (markers_group, _) = doc.group_nodes(&marker_nodes).expect("the markers group");
    name_node(&mut doc, NodeId::Group(markers_group), "Hour Markers");

    // Hands, set to ten past ten. Each is a slab rotated about the center;
    // the minute hand rides just above the hour hand so they read cleanly.
    let hour_sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, hour_sketch, &rect(-0.006, -0.014, 0.006, 0.078));
    let hour_hand = extrude_only_region(&mut doc, hour_sketch, 0.002);
    assert_watertight(&doc, hour_hand, "the hour hand");
    let hour_angle = -(std::f64::consts::TAU * (10.0 + 10.0 / 60.0) / 12.0);
    doc.transform_object(
        hour_hand,
        &rotate_about_z(0.0, 0.0, hour_angle)
            .then(&Transform::translation(Vec3::new(0.0, 0.0, 0.0295))),
    )
    .expect("hour hand poses onto the face");
    doc.set_object_material(hour_hand, Some(charcoal))
        .expect("hour hand accepts a default material");
    name_node(&mut doc, NodeId::Object(hour_hand), "Hour Hand");

    let minute_sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, minute_sketch, &rect(-0.005, -0.016, 0.005, 0.118));
    let minute_hand = extrude_only_region(&mut doc, minute_sketch, 0.002);
    assert_watertight(&doc, minute_hand, "the minute hand");
    let minute_angle = -(std::f64::consts::TAU * 10.0 / 60.0);
    doc.transform_object(
        minute_hand,
        &rotate_about_z(0.0, 0.0, minute_angle)
            .then(&Transform::translation(Vec3::new(0.0, 0.0, 0.032))),
    )
    .expect("minute hand poses onto the face");
    doc.set_object_material(minute_hand, Some(charcoal))
        .expect("minute hand accepts a default material");
    name_node(&mut doc, NodeId::Object(minute_hand), "Minute Hand");

    let (hands_group, _) = doc
        .group_nodes(&[NodeId::Object(hour_hand), NodeId::Object(minute_hand)])
        .expect("the hands group");
    name_node(&mut doc, NodeId::Group(hands_group), "Hands");

    // Center cap: a small true cylinder pinning the hands.
    let cap_sketch = doc.add_sketch(ground());
    draw_circle(&mut doc, cap_sketch, 0.0, 0.0, 0.009);
    let cap = extrude_only_region(&mut doc, cap_sketch, 0.003);
    doc.transform_object(cap, &Transform::translation(Vec3::new(0.0, 0.0, 0.0315)))
        .expect("cap lifts onto the hands");
    doc.set_object_material(cap, Some(brass))
        .expect("cap accepts a default material");
    name_node(&mut doc, NodeId::Object(cap), "Center Cap");
    assert_watertight(&doc, cap, "the center cap");

    // Glass cover: a thin translucent disc over the face, tagged so the
    // Tags panel can hide it to look inside.
    let glass_sketch = doc.add_sketch(ground());
    draw_circle(&mut doc, glass_sketch, 0.0, 0.0, 0.150);
    let cover = extrude_only_region(&mut doc, glass_sketch, 0.002);
    doc.transform_object(cover, &Transform::translation(Vec3::new(0.0, 0.0, 0.0355)))
        .expect("cover rests on the rim");
    doc.set_object_material(cover, Some(glass))
        .expect("cover accepts a default material");
    name_node(&mut doc, NodeId::Object(cover), "Glass Cover");
    doc.add_node_tag(NodeId::Object(cover), vec!["Glass".to_string()])
        .expect("cover accepts a tag");

    // Structural self-checks: the scene must demonstrate what the welcome
    // screen promises (true curves, components, groups, opacity, a tag).
    assert_eq!(doc.instance_ids().len(), 12, "twelve hour markers");
    assert_eq!(doc.component_ids().len(), 1, "one shared marker definition");
    assert_eq!(doc.group_ids().len(), 2, "markers and hands groups");
    assert_eq!(
        doc.material_ids().len(),
        5,
        "oak, brass, face, charcoal, glass"
    );
    assert!(
        doc.materials().get(oak).expect("oak exists").has_texture(),
        "the rim wood is a real image texture"
    );
    doc
}

// ------------------------------------------------------------- café table

/// A round café table: textured oak top (a true 300 mm-radius cylinder),
/// four walnut cylinder legs as rotated placements of one component, crossed
/// stretchers as a second component, a printable terracotta pen cup resting
/// on top, and construction guides on the leg axes.
fn build_cafe_table() -> Document {
    let mut doc = Document::new();

    let oak = oak_material(&mut doc);
    let walnut = walnut_material(&mut doc);
    let terracotta = solid(&mut doc, "Terracotta", 204, 110, 78);

    // One leg: a true 20 mm-radius cylinder, 700 mm tall, authored on the
    // +X axis at the leg circle's radius; the other three placements rotate
    // about the table center.
    let leg_sketch = doc.add_sketch(ground());
    draw_circle(&mut doc, leg_sketch, 0.20, 0.0, 0.020);
    let leg = extrude_only_region(&mut doc, leg_sketch, 0.700);
    assert_watertight(&doc, leg, "a table leg");
    doc.set_object_material(leg, Some(walnut))
        .expect("leg accepts a default material");
    name_node(&mut doc, NodeId::Object(leg), "Leg");
    let (leg_def, first_leg, _) = doc
        .make_component(&[NodeId::Object(leg)])
        .expect("the leg becomes a component");
    name_node(&mut doc, NodeId::Instance(first_leg), "Leg — east");
    let mut base_nodes = vec![NodeId::Instance(first_leg)];
    for (k, name) in [
        (1usize, "Leg — north"),
        (2, "Leg — west"),
        (3, "Leg — south"),
    ] {
        let angle = std::f64::consts::TAU * k as f64 / 4.0;
        let (inst, _) = doc
            .place_instance(leg_def, rotate_about_z(0.0, 0.0, angle))
            .expect("legs place at valid poses");
        name_node(&mut doc, NodeId::Instance(inst), name);
        base_nodes.push(NodeId::Instance(inst));
    }

    // Stretchers: one bar between opposite legs, a second placement rotated
    // 90° — the crossed brace under the top.
    let stretcher_sketch = doc.add_sketch(ground());
    draw_loop(
        &mut doc,
        stretcher_sketch,
        &rect(-0.20, -0.014, 0.20, 0.014),
    );
    let stretcher = extrude_only_region(&mut doc, stretcher_sketch, 0.024);
    assert_watertight(&doc, stretcher, "a stretcher");
    doc.transform_object(
        stretcher,
        &Transform::translation(Vec3::new(0.0, 0.0, 0.140)),
    )
    .expect("stretcher lifts to brace height");
    doc.set_object_material(stretcher, Some(walnut))
        .expect("stretcher accepts a default material");
    name_node(&mut doc, NodeId::Object(stretcher), "Stretcher");
    let (stretcher_def, first_stretcher, _) = doc
        .make_component(&[NodeId::Object(stretcher)])
        .expect("the stretcher becomes a component");
    name_node(
        &mut doc,
        NodeId::Instance(first_stretcher),
        "Stretcher — east-west",
    );
    let (cross, _) = doc
        .place_instance(
            stretcher_def,
            rotate_about_z(0.0, 0.0, std::f64::consts::FRAC_PI_2),
        )
        .expect("the crossed stretcher places");
    name_node(&mut doc, NodeId::Instance(cross), "Stretcher — north-south");
    base_nodes.push(NodeId::Instance(first_stretcher));
    base_nodes.push(NodeId::Instance(cross));

    let (base_group, _) = doc.group_nodes(&base_nodes).expect("the base groups");
    name_node(&mut doc, NodeId::Group(base_group), "Base");

    // Top: a true 300 mm-radius disc, 25 mm thick, lifted onto the legs.
    let top_sketch = doc.add_sketch(ground());
    draw_circle(&mut doc, top_sketch, 0.0, 0.0, 0.300);
    let top = extrude_only_region(&mut doc, top_sketch, 0.025);
    doc.transform_object(top, &Transform::translation(Vec3::new(0.0, 0.0, 0.700)))
        .expect("the top lifts onto the legs");
    doc.set_object_material(top, Some(oak))
        .expect("top accepts a default material");
    name_node(&mut doc, NodeId::Object(top), "Tabletop");
    assert_watertight(&doc, top, "the tabletop");

    // A pen cup resting on the table: a printable object in its own right
    // (true cylinder, imprinted + recessed rim, 4 mm walls and floor).
    let cup_sketch = doc.add_sketch(ground());
    draw_circle(&mut doc, cup_sketch, 0.0, 0.0, 0.040);
    let cup = extrude_only_region(&mut doc, cup_sketch, 0.090);
    let cup_top = top_face(&doc, cup);
    recess_circle(
        &mut doc,
        cup,
        cup_top,
        Point3::new(0.0, 0.0, 0.090),
        0.036,
        0.086,
    );
    doc.transform_object(
        cup,
        &Transform::translation(Vec3::new(0.130, -0.090, 0.725)),
    )
    .expect("the cup rests on the tabletop");
    doc.set_object_material(cup, Some(terracotta))
        .expect("cup accepts a default material");
    name_node(&mut doc, NodeId::Object(cup), "Pen Cup");
    doc.add_node_tag(NodeId::Object(cup), vec!["Accessories".to_string()])
        .expect("cup accepts a tag");
    assert_watertight(&doc, cup, "the pen cup");

    // Construction guides: the two leg axes and the cup's resting point.
    for angle in [0.0, std::f64::consts::FRAC_PI_2] {
        doc.add_guide_line(
            Point3::new(0.0, 0.0, 0.0),
            Vec3::new(angle.cos(), angle.sin(), 0.0),
        )
        .expect("guide lines are finite");
    }
    doc.add_guide_point(Point3::new(0.130, -0.090, 0.725))
        .expect("guide point is finite");

    // Structural self-checks.
    assert_eq!(doc.instance_ids().len(), 6, "four legs, two stretchers");
    assert_eq!(
        doc.component_ids().len(),
        2,
        "leg and stretcher definitions"
    );
    assert_eq!(doc.group_ids().len(), 1, "the grouped base");
    assert_eq!(doc.guide_ids().len(), 3, "two axis lines and a point");
    assert_eq!(doc.material_ids().len(), 3, "oak, walnut, terracotta");
    doc
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .expect("usage: build_samples <out-dir>");
    let out = std::path::Path::new(&out_dir);
    std::fs::create_dir_all(out).expect("output directory is writable");

    for (name, doc) in [
        ("wall-clock.hew", build_wall_clock()),
        ("cafe-table.hew", build_cafe_table()),
    ] {
        let bytes = doc.save();
        // Self-check: a sample that can't load back must never ship.
        Document::load(&bytes).expect("sample round-trips through load");
        let path = out.join(name);
        std::fs::write(&path, &bytes).expect("sample file is writable");
        println!("wrote {} ({} bytes)", path.display(), bytes.len());
    }
}
