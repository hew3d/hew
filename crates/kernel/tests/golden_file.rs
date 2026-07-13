//! Golden-file tests for the native file format.
//!
//! Builds a representative document exercising the full feature surface
//! (a box, a group, a component with a mirrored instance, painted + textured +
//! base-material objects, a sketch with a consumed region, a construction
//! guide line + point), saves it, and asserts bytes equal a committed
//! fixture. Also asserts it re-loads.
//!
//! The golden file is generated from the serializer itself on first run (via
//! `REGENERATE_GOLDEN=1 cargo test`). From then on, any drift from the spec is
//! caught by the byte comparison.
//!
//! Run with `REGENERATE_GOLDEN=1` to regenerate the fixture intentionally
//! (on a version bump or spec change).

use std::path::PathBuf;

use kernel::{
    Document, ImageFormat, Material, NodeId, Plane, Point3, Rgba8, Texture, Transform, Vec3,
    WatertightState,
};

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/representative.hew")
}

/// Build the canonical representative document used for golden testing.
fn build_representative_doc() -> Document {
    let mut doc = Document::new();

    // ── Object A: a 1×1×1 box (watertight), base material = Blue ──────
    let plane_a = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let sa = doc.add_sketch(plane_a);
    {
        let sk = doc.sketch_mut(sa).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let ra = doc.extrudable_regions(sa).unwrap()[0];
    let (obj_a, _) = doc.extrude_region(sa, ra, 1.0).unwrap();

    // Object B: a 2×1×1 box at (5,5,0), painted top face with Red
    let plane_b = Plane::from_polygon(&[
        Point3::new(5.0, 5.0, 0.0),
        Point3::new(7.0, 5.0, 0.0),
        Point3::new(5.0, 6.0, 0.0),
    ])
    .unwrap();
    let sb = doc.add_sketch(plane_b);
    {
        let sk = doc.sketch_mut(sb).unwrap();
        for (a, b) in [
            (Point3::new(5.0, 5.0, 0.0), Point3::new(7.0, 5.0, 0.0)),
            (Point3::new(7.0, 5.0, 0.0), Point3::new(7.0, 6.0, 0.0)),
            (Point3::new(7.0, 6.0, 0.0), Point3::new(5.0, 6.0, 0.0)),
            (Point3::new(5.0, 6.0, 0.0), Point3::new(5.0, 5.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let rb = doc.extrudable_regions(sb).unwrap()[0];
    let (obj_b, _) = doc.extrude_region(sb, rb, 1.0).unwrap();

    // Materials
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 50, 40)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 60, 200)));
    let image = vec![0x89u8, 0x50, 0x4e, 0x47, 1, 2, 3, 4, 5]; // opaque PNG blob
    let wood = doc.add_material(Material::textured(
        "Wood",
        Rgba8::rgb(180, 120, 60),
        Texture {
            image,
            format: ImageFormat::Png,
            world_size: [0.5, 1.5],
        },
    ));

    // Paint the first face of obj_b with Red
    let face_b = doc.object(obj_b).unwrap().faces().keys().next().unwrap();
    doc.paint_face(obj_b, face_b, Some(red)).unwrap();

    // Set obj_a base material to Blue
    doc.set_object_material(obj_a, Some(blue)).unwrap();

    // Set obj_b one face to Wood texture (second face)
    let face_b2 = doc.object(obj_b).unwrap().faces().keys().nth(1).unwrap();
    doc.paint_face(obj_b, face_b2, Some(wood)).unwrap();

    // ── Group: obj_a and obj_b ──────────────────────────────────────────
    doc.group_nodes(&[NodeId::Object(obj_a), NodeId::Object(obj_b)])
        .unwrap();

    // ── Component with two instances (one identity, one mirrored) ───────
    let plane_c = Plane::from_polygon(&[
        Point3::new(10.0, 0.0, 0.0),
        Point3::new(11.0, 0.0, 0.0),
        Point3::new(10.0, 1.0, 0.0),
    ])
    .unwrap();
    let sc = doc.add_sketch(plane_c);
    {
        let sk = doc.sketch_mut(sc).unwrap();
        for (a, b) in [
            (Point3::new(10.0, 0.0, 0.0), Point3::new(11.0, 0.0, 0.0)),
            (Point3::new(11.0, 0.0, 0.0), Point3::new(11.0, 1.0, 0.0)),
            (Point3::new(11.0, 1.0, 0.0), Point3::new(10.0, 1.0, 0.0)),
            (Point3::new(10.0, 1.0, 0.0), Point3::new(10.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let rc = doc.extrudable_regions(sc).unwrap()[0];
    let (obj_c, _) = doc.extrude_region(sc, rc, 0.5).unwrap();
    let (comp, _inst0, _) = doc.make_component(&[NodeId::Object(obj_c)]).unwrap();
    // Mirrored instance (det < 0 pose allowed for instances)
    let mirror = Transform::from_affine(&[
        -1.0, 0.0, 0.0, 20.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ]);
    doc.place_instance(comp, mirror).unwrap();

    // ── Sketch with a consumed region ────────────────────────────────────
    let ground = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let sd = doc.add_sketch(ground);
    {
        let sk = doc.sketch_mut(sd).unwrap();
        // Rectangle 1: to be consumed by extrusion. Placed at (-2,0)-(-1,1),
        // clear of obj_a's base — its exact position is incidental to this
        // golden, which fixes the on-disk bytes.
        for (a, b) in [
            (Point3::new(-2.0, 0.0, 0.0), Point3::new(-1.0, 0.0, 0.0)),
            (Point3::new(-1.0, 0.0, 0.0), Point3::new(-1.0, 1.0, 0.0)),
            (Point3::new(-1.0, 1.0, 0.0), Point3::new(-2.0, 1.0, 0.0)),
            (Point3::new(-2.0, 1.0, 0.0), Point3::new(-2.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
        // A two-facet curve chain (v7: curve ids on edges) off to the side.
        sk.begin_curve();
        for (a, b) in [
            (Point3::new(5.0, 0.0, 0.0), Point3::new(5.5, 0.3, 0.0)),
            (Point3::new(5.5, 0.3, 0.0), Point3::new(6.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
        sk.end_curve();
        // Rectangle 2: remains extrudable
        for (a, b) in [
            (Point3::new(2.0, 0.0, 0.0), Point3::new(3.0, 0.0, 0.0)),
            (Point3::new(3.0, 0.0, 0.0), Point3::new(3.0, 1.0, 0.0)),
            (Point3::new(3.0, 1.0, 0.0), Point3::new(2.0, 1.0, 0.0)),
            (Point3::new(2.0, 1.0, 0.0), Point3::new(2.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let regions_d = doc.extrudable_regions(sd).unwrap();
    assert_eq!(regions_d.len(), 2);
    // Extrude one region (consumes it)
    doc.extrude_region(sd, regions_d[0], 0.5).unwrap();
    assert_eq!(doc.extrudable_regions(sd).unwrap().len(), 1);

    // ── Guides: one construction line + one construction point ───
    doc.add_guide_line(Point3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    doc.add_guide_point(Point3::new(2.0, 3.0, 0.0)).unwrap();

    doc
}

#[test]
fn golden_file_save_load_and_determinism() {
    let doc = build_representative_doc();
    let bytes = doc.save();

    // Determinism: saving twice gives identical bytes.
    assert_eq!(bytes, doc.save(), "save must be byte-deterministic");

    // Golden file check.
    let golden = golden_path();
    let regenerate = std::env::var("REGENERATE_GOLDEN").is_ok();

    if regenerate || !golden.exists() {
        // Write the golden file.
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &bytes).expect("failed to write golden file");
        eprintln!("Wrote golden file: {}", golden.display());
    } else {
        // Assert bytes match the golden file.
        let expected = std::fs::read(&golden).expect("failed to read golden file");
        assert_eq!(
            bytes, expected,
            "output differs from golden file — run with REGENERATE_GOLDEN=1 to update"
        );
    }

    // Round-trip: load returns a structurally equivalent document.
    let loaded = Document::load(&bytes).expect("load must succeed");

    // Objects preserved.
    assert!(
        loaded.visible_object_ids().len() >= 2,
        "visible objects round-trip"
    );

    // Watertight flag.
    for oid in loaded.visible_object_ids() {
        let obj = loaded.object(oid).unwrap();
        assert_eq!(
            obj.watertight(),
            WatertightState::Watertight,
            "all test objects are watertight"
        );
    }

    // Material palette preserved.
    let mat_names: Vec<&str> = loaded
        .material_ids()
        .iter()
        .map(|&m| loaded.material(m).unwrap().name.as_str())
        .collect();
    assert!(
        mat_names.contains(&"Red") && mat_names.contains(&"Blue") && mat_names.contains(&"Wood"),
        "material names survive: {mat_names:?}"
    );

    // Texture bytes preserved verbatim.
    let wood_mat = loaded
        .material_ids()
        .iter()
        .map(|&m| loaded.material(m).unwrap())
        .find(|m| m.name == "Wood")
        .expect("Wood material survives");
    let tex = wood_mat.texture.as_ref().expect("Wood has a texture");
    assert_eq!(tex.format, ImageFormat::Png);
    assert_eq!(tex.world_size, [0.5, 1.5]);
    assert_eq!(tex.image[0], 0x89); // PNG header byte

    // Groups: one group survives.
    assert_eq!(loaded.group_ids().len(), 1, "one group round-trips");
    let g = loaded.group_ids()[0];
    assert_eq!(
        loaded.group_members(g).unwrap().len(),
        2,
        "group has 2 members"
    );

    // Components: one component + 2 instances.
    assert_eq!(loaded.component_ids().len(), 1, "one component round-trips");
    assert_eq!(loaded.instance_ids().len(), 2, "two instances round-trip");

    // Sketch: the representative doc stores 4 sketches (sa, sb, sc, sd) and all
    // 4 round-trip in the file, but `sketch_ids` lists only the ones that still
    // EXIST as actionable sketches: a wholly-extruded sketch is consumed into
    // its solid and drops out (sketch-lifecycle fix). sa/sb/sc are fully
    // consumed; only sd (which kept an unextruded region) remains.
    let sketch_ids = loaded.sketch_ids();
    assert_eq!(
        sketch_ids.len(),
        1,
        "only the sketch with a surviving region is still actionable"
    );
    // The last sketch (sd) has 1 extrudable region after consuming one.
    // We find it by checking which sketch has exactly 1 extrudable region.
    let single_ext = sketch_ids
        .iter()
        .filter(|&&sid| loaded.extrudable_regions(sid).unwrap().len() == 1)
        .count();
    assert_eq!(
        single_ext, 1,
        "exactly one sketch should have 1 extrudable region (the consumed-region sketch)"
    );

    // Guides: one line + one point round-trip.
    let guide_ids = loaded.guide_ids();
    assert_eq!(guide_ids.len(), 2, "two guides round-trip");
    let mut saw_line = false;
    let mut saw_point = false;
    for gid in guide_ids {
        match loaded.guide(gid).unwrap() {
            kernel::Guide::Line { origin, direction } => {
                saw_line = true;
                assert!(origin.approx_eq(Point3::new(0.0, 0.0, 0.0), 1e-9));
                assert!(direction.approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9));
            }
            kernel::Guide::Point { position } => {
                saw_point = true;
                assert!(position.approx_eq(Point3::new(2.0, 3.0, 0.0), 1e-9));
            }
        }
    }
    assert!(saw_line && saw_point, "both guide kinds round-trip");

    // No undo history in loaded doc.
    assert!(!loaded.can_undo(), "loaded doc has empty undo stack");
}
