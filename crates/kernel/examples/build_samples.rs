//! Builds the bundled sample models through the public [`Document`] API —
//! the same operations the app performs — and writes them as `.hew` files.
//! Because the kernel is deterministic, regenerating produces byte-stable
//! output; the samples double as living exercises of the real modeling
//! paths (sketch → extrude → imprint → recess, components, groups,
//! materials, guides).
//!
//! Usage: `cargo run -p kernel --example build_samples -- <out-dir>`
//! (the repo's bundled copies live in `app/public/samples/`).
//!
//! Two models, per the roadmap:
//! - `pen-cup.hew` — a finished, printable object: a faceted cup, solid and
//!   watertight, sized for a real desk (80 mm across, 100 mm tall, 4 mm
//!   walls).
//! - `side-table.hew` — a mid-construction scene: a tabletop slab, four
//!   legs as instances of one component definition, a group, materials,
//!   construction guides, and a still-unextruded shelf profile waiting on
//!   the ground sketch.

use kernel::{
    Document, FaceId, KernelOp, KernelOpReport, Material, NodeId, ObjectId, Plane, Point3, Rgba8,
    Transform, Vec3,
};

fn ground() -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-formed")
}

/// CCW regular polygon on z = `z`, first vertex at angle 0.
fn circle(cx: f64, cy: f64, r: f64, n: usize, z: f64) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), z)
        })
        .collect()
}

/// Draw a closed loop into a sketch as segments.
fn draw_loop(doc: &mut Document, sketch: kernel::SketchId, pts: &[Point3]) {
    let s = doc.sketch_mut(sketch).expect("sketch exists");
    for i in 0..pts.len() {
        s.add_segment(pts[i], pts[(i + 1) % pts.len()])
            .expect("sample loop segments are valid");
    }
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
/// the ground plane (the |offset| ranking would misorder +Z faces below
/// z = 0); every sample here is a ground-up extrusion.
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
fn extrude_only_region(doc: &mut Document, sketch: kernel::SketchId, distance: f64) -> ObjectId {
    let regions = doc.extrudable_regions(sketch).expect("sketch exists");
    let region = *regions.last().expect("the drawn loop closed into a region");
    let (object, _) = doc
        .extrude_region(sketch, region, distance)
        .expect("sample profiles extrude cleanly");
    object
}

/// A finished, printable pen cup: 24-gon cup, 40 mm outer radius, 100 mm
/// tall, 4 mm walls and floor via an imprinted recess.
fn build_pen_cup() -> Document {
    let mut doc = Document::new();

    let sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, sketch, &circle(0.0, 0.0, 0.040, 24, 0.0));
    let cup = extrude_only_region(&mut doc, sketch, 0.100);

    // Imprint the inner rim on the top face and recess it, leaving a 4 mm
    // floor — the same draw-on-a-face + push/pull path a user takes.
    let top = top_face(&doc, cup);
    let (report, _) = doc
        .apply_object_op(
            cup,
            KernelOp::SplitFaceInner {
                face: top,
                loop_path: circle(0.0, 0.0, 0.036, 24, 0.100),
            },
        )
        .expect("inner rim imprints");
    let KernelOpReport::FaceSplitInner(inner) = report else {
        panic!("SplitFaceInner reports FaceSplitInner");
    };
    doc.apply_object_op(
        cup,
        KernelOp::ExtrudeSubFace {
            sub_face: inner.sub_face,
            distance: -0.096,
        },
    )
    .expect("recess carves the cup");

    let terracotta = doc.add_material(Material::solid(
        "Terracotta",
        Rgba8 {
            r: 204,
            g: 110,
            b: 78,
            a: 255,
        },
    ));
    doc.set_object_material(cup, Some(terracotta))
        .expect("cup accepts a default material");
    doc.set_node_name(NodeId::Object(cup), Some("Pen Cup".to_string()))
        .expect("cup accepts a name");

    assert_eq!(
        doc.object(cup).expect("cup exists").watertight(),
        kernel::WatertightState::Watertight,
        "the printable sample must be a watertight solid"
    );
    doc
}

/// A mid-construction side table: top slab (grouped), four legs as
/// instances of one Leg component, wood materials, guides marking the leg
/// grid, and an unextruded shelf profile still on the ground sketch.
fn build_side_table() -> Document {
    let mut doc = Document::new();

    let walnut = doc.add_material(Material::solid(
        "Walnut",
        Rgba8 {
            r: 92,
            g: 61,
            b: 42,
            a: 255,
        },
    ));
    let oak = doc.add_material(Material::solid(
        "Oak",
        Rgba8 {
            r: 190,
            g: 152,
            b: 102,
            a: 255,
        },
    ));

    // One leg, 40×40 mm × 350 mm, built at the origin corner: the component
    // definition every placement shares.
    let leg_sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, leg_sketch, &rect(0.0, 0.0, 0.040, 0.040));
    let leg = extrude_only_region(&mut doc, leg_sketch, 0.350);
    doc.set_object_material(leg, Some(walnut))
        .expect("leg accepts a default material");
    doc.set_node_name(NodeId::Object(leg), Some("Leg".to_string()))
        .expect("leg accepts a name");
    let (component, first_leg, _) = doc
        .make_component(&[NodeId::Object(leg)])
        .expect("the leg becomes a component");
    doc.set_node_name(
        NodeId::Instance(first_leg),
        Some("Leg — front left".to_string()),
    )
    .expect("instance accepts a name");

    // The other three corners of a 500×400 mm top, legs flush with its
    // edges (a Parsons-table stance).
    let places = [
        (0.460, 0.0, "Leg — front right"),
        (0.0, 0.360, "Leg — back left"),
        (0.460, 0.360, "Leg — back right"),
    ];
    for (x, y, name) in places {
        let (inst, _) = doc
            .place_instance(component, Transform::translation(Vec3::new(x, y, 0.0)))
            .expect("legs place at valid poses");
        doc.set_node_name(NodeId::Instance(inst), Some(name.to_string()))
            .expect("instance accepts a name");
    }

    // The top: a 500×400×20 mm slab extruded on the ground, lifted onto the
    // legs, grouped so it reads as one piece in the outliner.
    let top_sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, top_sketch, &rect(0.0, 0.0, 0.500, 0.400));
    let top = extrude_only_region(&mut doc, top_sketch, 0.020);
    doc.transform_object(top, &Transform::translation(Vec3::new(0.0, 0.0, 0.350)))
        .expect("the top lifts onto the legs");
    doc.set_object_material(top, Some(oak))
        .expect("top accepts a default material");
    doc.set_node_name(NodeId::Object(top), Some("Tabletop".to_string()))
        .expect("top accepts a name");
    let (top_group, _) = doc
        .group_nodes(&[NodeId::Object(top)])
        .expect("the top groups");
    doc.set_node_name(NodeId::Group(top_group), Some("Top".to_string()))
        .expect("group accepts a name");

    // Construction guides on the legs' x-centerlines, plus a point at the
    // center of the planned shelf.
    for x in [0.020, 0.480] {
        doc.add_guide_line(Point3::new(x, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0))
            .expect("guide lines are finite");
    }
    doc.add_guide_point(Point3::new(0.250, 0.200, 0.150))
        .expect("guide point is finite");

    // Mid-construction: the lower shelf is drawn but not yet extruded — the
    // scene opens with a profile waiting on the ground sketch.
    let shelf_sketch = doc.add_sketch(ground());
    draw_loop(&mut doc, shelf_sketch, &rect(0.060, 0.060, 0.440, 0.340));

    // Structural self-checks: the scene must actually demonstrate what the
    // roadmap bullet promises (components, a group, materials, guides, and
    // an open profile).
    assert_eq!(doc.instance_ids().len(), 4, "four placed legs");
    assert_eq!(doc.component_ids().len(), 1, "one shared Leg definition");
    assert_eq!(doc.group_ids().len(), 1, "the grouped top");
    assert_eq!(doc.guide_ids().len(), 3, "two guide lines and a point");
    assert_eq!(doc.material_ids().len(), 2, "walnut and oak");
    assert!(
        !doc.extrudable_regions(shelf_sketch)
            .expect("shelf sketch exists")
            .is_empty(),
        "the shelf profile is still open for extrusion"
    );

    doc
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .expect("usage: build_samples <out-dir>");
    let out = std::path::Path::new(&out_dir);
    std::fs::create_dir_all(out).expect("output directory is writable");

    for (name, doc) in [
        ("pen-cup.hew", build_pen_cup()),
        ("side-table.hew", build_side_table()),
    ] {
        let bytes = doc.save();
        // Self-check: a sample that can't load back must never ship.
        Document::load(&bytes).expect("sample round-trips through load");
        let path = out.join(name);
        std::fs::write(&path, &bytes).expect("sample file is writable");
        println!("wrote {} ({} bytes)", path.display(), bytes.len());
    }
}
