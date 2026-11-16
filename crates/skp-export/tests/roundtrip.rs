//! Hew -> `.skp` -> Hew: a document written by `skp-export` and read back
//! by `skp-import` is the same document, on both containers.
//!
//! Sources: every `skp-import` fixture (2017 and its 2026 twin) imported
//! into a fresh document, and `fixtures/native.hew`, drawn in Hew. The
//! comparison is a canonical digest of what the user sees and edits — the
//! node tree with names, tags, hidden state, poses and attributes, each
//! Object's faces (positions to a micron, each face's effective material,
//! textured corners' UVs), its soft edges, circle claims and curved faces,
//! definitions, sketches with their curves, the tag registry, guides, the
//! section plane, the camera, the scenes with their display toggles, and
//! the document's own attributes. What the digest normalizes away, and why:
//!
//! - ring start corners and face/sibling order (no meaning in either file);
//! - unused palette materials: the importer brings in referenced materials
//!   only, so the palettes compare by the materials faces resolve to.

mod common;

use std::path::Path;

use common::{digest, near};
use kernel::Document;
use skp_export::{Target, WriteEnv, export};

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../skp-import/tests/fixtures");
    let mut out = Vec::new();
    for dir in [root.clone(), root.join("2026")] {
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "skp"))
            .collect();
        names.sort();
        for p in names {
            let label = p.strip_prefix(&root).unwrap().display().to_string();
            out.push((label, std::fs::read(&p).unwrap()));
        }
    }
    out
}

fn import(bytes: &[u8]) -> Document {
    let out = skp_import::import(bytes).expect("imports");
    let mut doc = Document::new();
    doc.ingest(out.scene, out.textures_missing)
        .expect("ingests");
    doc
}

fn round_trip(label: &str, source: &Document, target: Target) -> Result<(), String> {
    let written =
        export(source, target, WriteEnv::system()).map_err(|e| format!("{label}: {e}"))?;
    let model =
        skpwright::Model::parse(&written.bytes).map_err(|e| format!("{label}: reread: {e:?}"))?;
    let desync: Vec<_> = model.diagnostics.iter().filter(|d| d.is_desync()).collect();
    if !desync.is_empty() {
        return Err(format!(
            "{label}: written file reads with desyncs: {desync:?}"
        ));
    }
    if target != Target::Skp2026 {
        skpwright::verify(&written.bytes).map_err(|e| format!("{label}: 2017 grammar: {e:?}"))?;
    }
    let back = import(&written.bytes);
    let a = digest(source);
    let mut b = digest(&back);
    // A file always holds a view; a document without one takes the
    // writer's default, which is not a difference.
    if a.camera.is_none() {
        b.camera = None;
    }
    if a != b && !near(&format!("{a:?}"), &format!("{b:?}")) {
        let mut msg = format!("{label} ({target:?}) differs after the round trip:\n");
        macro_rules! cmp {
            ($f:ident) => {
                if a.$f != b.$f {
                    msg += &format!(
                        "  {}:\n    source: {:#?}\n    back:   {:#?}\n",
                        stringify!($f),
                        a.$f,
                        b.$f
                    );
                }
            };
        }
        cmp!(roots);
        cmp!(tags);
        cmp!(guides);
        cmp!(section);
        cmp!(camera);
        cmp!(scenes);
        cmp!(sketches);
        cmp!(attrs);
        cmp!(annotations);
        return Err(msg);
    }
    Ok(())
}

#[test]
fn every_fixture_survives_hew_to_skp_to_hew() {
    let mut failures = Vec::new();
    let mut n = 0;
    for (label, bytes) in fixtures() {
        // Fixtures the importer refuses (pre-2017 versions) are not sources.
        let Ok(out) = skp_import::import(&bytes) else {
            continue;
        };
        let mut source = Document::new();
        source
            .ingest(out.scene, out.textures_missing)
            .expect("ingests");
        for target in common::all_targets() {
            n += 1;
            if let Err(e) = round_trip(&label, &source, target) {
                failures.push(e);
            }
        }
    }
    let native = Document::load(
        &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native.hew"))
            .unwrap(),
    )
    .unwrap();
    for target in common::all_targets() {
        n += 1;
        if let Err(e) = round_trip("native.hew", &native, target) {
            failures.push(e);
        }
    }
    assert!(n >= 80, "too few round trips ran: {n}");
    if !failures.is_empty() {
        if let Ok(dir) = std::env::var("SKP_EXPORT_DIFF_DIR") {
            for (i, f) in failures.iter().enumerate() {
                std::fs::write(format!("{dir}/diff-{i}.txt"), f).unwrap();
            }
        }
        let shown: String = failures
            .iter()
            .take(6)
            .map(|f| {
                let mut s = f.clone();
                s.truncate(6000);
                s
            })
            .collect::<Vec<_>>()
            .join("\n");
        panic!("{} of {n} round trips differ:\n{shown}", failures.len());
    }
}

#[test]
fn an_empty_document_writes_to_both_containers() {
    let doc = Document::new();
    for target in common::all_targets() {
        round_trip("empty", &doc, target).unwrap();
    }
}

#[test]
fn a_face_is_written_with_its_back_paint() {
    // back-material.skp: one face painted differently on each side; it is
    // written back with both paints. A face without back paint on an open
    // shell is written painted on both sides (Hew draws its material on
    // both; SketchUp would draw an unpainted side in its back-face color),
    // and a closed solid's face, whose inside is never seen, on its front
    // only.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../skp-import/tests/fixtures");
    let names = |m: &skpwright::Model| -> Vec<(Option<String>, Option<String>)> {
        let name = |s: Option<u16>| {
            s.and_then(|s| m.material_of(s)).map(|x| match x {
                skpwright::Material::Solid { name, .. }
                | skpwright::Material::Textured { name, .. } => name.clone(),
            })
        };
        let mut v: Vec<_> = m
            .geometry
            .iter()
            .flat_map(|r| r.mesh.faces.iter())
            .filter(|f| f.front_material.is_some())
            .map(|f| (name(f.front_material), name(f.back_material)))
            .collect();
        v.sort();
        v
    };
    let bytes = std::fs::read(root.join("back-material.skp")).unwrap();
    let source = skpwright::Model::parse(&bytes).unwrap();
    let painted: Vec<_> = names(&source)
        .into_iter()
        .filter(|(f, b)| b.is_some() && b != f)
        .collect();
    assert_eq!(painted.len(), 1, "{:?}", names(&source));
    let doc = import(&bytes);
    for target in common::all_targets() {
        let written = export(&doc, target, WriteEnv::system()).unwrap();
        let back = names(&skpwright::Model::parse(&written.bytes).unwrap());
        assert!(back.contains(&painted[0]), "{target:?}: {back:?}");
    }
    let doc = import(&std::fs::read(root.join("box-two-materials.skp")).unwrap());
    for target in common::all_targets() {
        let written = export(&doc, target, WriteEnv::system()).unwrap();
        let m = skpwright::Model::parse(&written.bytes).unwrap();
        let faces: Vec<_> = m
            .geometry
            .iter()
            .flat_map(|r| r.mesh.faces.iter())
            .collect();
        assert!(faces.iter().any(|f| f.front_material.is_some()));
        for f in faces.iter().filter(|f| f.front_material.is_some()) {
            assert_eq!(f.back_material, None, "{target:?}");
        }
    }
}

#[test]
fn a_stretched_components_circles_survive() {
    // A component of two solids with circles (native.hew's drilled block,
    // and its group holding a cylinder) placed with a non-uniform scale.
    // The definition's members are written as groups inside it and read
    // back in the definition's own frame, so their circle claims stay:
    // the stretch is the placement's, not theirs.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native.hew");
    let mut doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let tops = doc.top_level_nodes();
    let block = tops
        .iter()
        .copied()
        .find(|n| matches!(n, kernel::NodeId::Object(_)))
        .unwrap();
    let group = tops
        .iter()
        .copied()
        .find(|n| matches!(n, kernel::NodeId::Group(_)))
        .unwrap();
    let (_, instance, _) = doc.make_component(&[block, group]).unwrap();
    let stretch = kernel::Transform::from_affine(&[
        1.2, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 0.7, 0.0,
    ]);
    doc.transform_instance(instance, &stretch).unwrap();
    for target in common::all_targets() {
        round_trip("stretched component", &doc, target).unwrap();
    }
}

#[test]
fn a_production_models_whole_export_reads_back_as_written() {
    // theater: SketchUp meshes that are not manifold open split into
    // pieces, some closed. Written whole and read back, each piece is the
    // faces Hew wrote, turned as Hew had them. (The file sits beside the
    // Skpwright dev clone; skipped elsewhere.)
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../skpwright/corpus/third-party/theater-2017.skp");
    let Ok(bytes) = std::fs::read(&p) else {
        eprintln!("skipped: no ../skpwright checkout");
        return;
    };
    let doc = import(&bytes);
    for target in common::all_targets() {
        round_trip("theater", &doc, target).unwrap();
    }
}

/// A box whose top holds a circle tangent to its front edge at one point:
/// the disc is a face and the top holds it as a hole sharing that vertex
/// with its outer loop (loops of a face may share isolated vertices). Hew
/// writes it as a face with a touching inner loop, which SketchUp holds
/// natively, and reads it back as topology on both containers — the same
/// faces, the touching hole kept, nothing dropped to sketch drawing.
fn tangent_circle_box() -> Document {
    use kernel::{CurveGeom, Plane, Point3};
    let mut doc = Document::new();
    let ground = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(ground);
    let c = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    for i in 0..4 {
        doc.sketch_mut(s)
            .unwrap()
            .add_segment(c[i], c[(i + 1) % 4])
            .unwrap();
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    let (id, _) = doc.extrude_region(s, r, 1.0).unwrap();
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .unwrap();
    // 72 facets about (1, 0.5), vertex 0 exactly on the front edge at (1, 0).
    let ring: Vec<Point3> = (0..72)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 72.0;
            Point3::new(1.0 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin(), 1.0)
        })
        .collect();
    let mut ring = ring;
    ring[0] = Point3::new(1.0, 0.0, 1.0);
    doc.draft(top, None, &[id], "Circle", |sk| {
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(1.0, 0.5, 1.0),
            radius: 0.5,
        })?;
        for i in 0..ring.len() {
            sk.add_segment(ring[i], ring[(i + 1) % ring.len()])?;
        }
        sk.end_curve();
        Ok(())
    })
    .unwrap();
    doc
}

/// Whether some face of some visible object holds two loops sharing a vertex.
fn has_touching_hole(doc: &Document) -> bool {
    doc.visible_object_ids().into_iter().any(|id| {
        let obj = doc.object(id).unwrap();
        obj.faces().values().any(|face| {
            let loops: Vec<Vec<kernel::VertexId>> = std::iter::once(face.outer_loop)
                .chain(face.inner_loops.iter().copied())
                .map(|l| {
                    obj.loop_half_edges(l)
                        .map(|h| obj.half_edges()[h].origin)
                        .collect()
                })
                .collect();
            (0..loops.len()).any(|i| {
                (i + 1..loops.len()).any(|j| loops[i].iter().any(|v| loops[j].contains(v)))
            })
        })
    })
}

#[test]
fn a_hole_touching_its_faces_outer_loop_survives_hew_to_skp_to_hew() {
    let source = tangent_circle_box();
    assert!(has_touching_hole(&source));
    for target in common::all_targets() {
        round_trip("tangent circle", &source, target).unwrap_or_else(|e| panic!("{e}"));
        let written = export(&source, target, WriteEnv::system()).expect("writes");
        let back = import(&written.bytes);
        assert!(
            has_touching_hole(&back),
            "{target:?}: the touching hole is topology"
        );
        for id in back.visible_object_ids() {
            back.object(id).unwrap().validate().expect("valid");
            assert!(
                back.object_owned_sketches(id).is_empty(),
                "{target:?}: nothing dropped to sketch drawing"
            );
        }
    }
}

/// Each sketch's regions as `(outer ring length, holes, has a face)`,
/// sorted, with its plane (a definition's in its own frame) and imported
/// mark; world sketches and every definition's.
fn faced_regions(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = doc
        .sketch_ids()
        .into_iter()
        .chain(
            doc.component_ids()
                .into_iter()
                .flat_map(|c| doc.def_member_sketches(c).unwrap_or_default()),
        )
        .map(|s| {
            let sk = doc.sketch(s).unwrap();
            let mut regions: Vec<(usize, usize, bool)> = sk
                .regions()
                .values()
                .map(|r| (r.outer.len(), r.holes.len(), !r.faceless))
                .collect();
            regions.sort();
            let n = sk.plane().normal();
            format!(
                "n [{:.3} {:.3} {:.3}] d {:.4} imported {} regions {regions:?}",
                n.x,
                n.y,
                n.z,
                sk.plane().signed_distance(kernel::Point3::ORIGIN),
                doc.sketch_is_imported(s),
            )
        })
        .collect();
    out.sort();
    out
}

/// Drawing made in Hew: a rectangle with a rectangular hole (the hole's
/// own region filled too), a circle on a wall, a hexagon, a closed arc
/// segment, one loop of bare edges with no face, and a rectangle in a
/// group and in a placed component.
fn drawn_regions_doc() -> Document {
    use kernel::{CurveGeom, Plane, Point3, SketchCurveKind, Vec3};
    let p = Point3::new;
    let mut doc = Document::new();
    let ring = |pts: &[Point3]| -> Vec<(Point3, Point3)> {
        (0..pts.len())
            .map(|i| (pts[i], pts[(i + 1) % pts.len()]))
            .collect()
    };
    // The rectangle with a hole, on the ground.
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0., 0., 1.)).unwrap();
    let s = doc.add_sketch(ground);
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in ring(&[p(0., 0., 0.), p(2., 0., 0.), p(2., 1., 0.), p(0., 1., 0.)])
        .into_iter()
        .chain(ring(&[
            p(0.5, 0.25, 0.),
            p(1.0, 0.25, 0.),
            p(1.0, 0.75, 0.),
            p(0.5, 0.75, 0.),
        ]))
    {
        sk.add_segment(a, b).unwrap();
    }
    // A circle on the wall x = 5.
    let wall = Plane::from_point_normal(p(5., 0., 0.), Vec3::new(1., 0., 0.)).unwrap();
    let s = doc.add_sketch(wall);
    let sk = doc.sketch_mut(s).unwrap();
    let (c, r, n) = (p(5., 1., 1.), 0.4, 24);
    sk.begin_curve_with(CurveGeom {
        center: c,
        radius: r,
    })
    .unwrap();
    let rim: Vec<Point3> = (0..n)
        .map(|i| {
            let t = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            p(5., c.y + r * t.cos(), c.z + r * t.sin())
        })
        .collect();
    for (a, b) in ring(&rim) {
        sk.add_segment(a, b).unwrap();
    }
    sk.end_curve();
    // A hexagon (a polygon claim) on a raised plane.
    let raised = Plane::from_point_normal(p(0., 0., 2.), Vec3::new(0., 0., 1.)).unwrap();
    let s = doc.add_sketch(raised);
    let sk = doc.sketch_mut(s).unwrap();
    let (c, r) = (p(0., 5., 2.), 0.5);
    sk.begin_curve_with_kind(
        CurveGeom {
            center: c,
            radius: r,
        },
        SketchCurveKind::Polygon,
    )
    .unwrap();
    let hex: Vec<Point3> = (0..6)
        .map(|i| {
            let t = std::f64::consts::TAU * f64::from(i) / 6.;
            p(c.x + r * t.cos(), c.y + r * t.sin(), 2.)
        })
        .collect();
    for (a, b) in ring(&hex) {
        sk.add_segment(a, b).unwrap();
    }
    sk.end_curve();
    // A closed arc segment (a half disk: the arc and its chord), on the
    // wall y = 8.
    let back = Plane::from_point_normal(p(0., 8., 0.), Vec3::new(0., 1., 0.)).unwrap();
    let s = doc.add_sketch(back);
    let sk = doc.sketch_mut(s).unwrap();
    let (c, r) = (p(3., 8., 1.), 0.5);
    sk.begin_curve_with(CurveGeom {
        center: c,
        radius: r,
    })
    .unwrap();
    let arc: Vec<Point3> = (0..=12)
        .map(|i| {
            let t = std::f64::consts::PI * f64::from(i) / 12.;
            p(c.x + r * t.cos(), 8., c.z + r * t.sin())
        })
        .collect();
    for w in arc.windows(2) {
        sk.add_segment(w[0], w[1]).unwrap();
    }
    sk.end_curve();
    sk.add_segment(arc[12], arc[0]).unwrap();
    // A loop of bare edges with no face, on a plane of its own.
    let high = Plane::from_point_normal(p(0., 0., 4.), Vec3::new(0., 0., 1.)).unwrap();
    let s = doc.add_sketch(high);
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in ring(&[p(0., 0., 4.), p(1., 0., 4.), p(1., 1., 4.), p(0., 1., 4.)]) {
        sk.add_segment(a, b).unwrap();
    }
    sk.make_regions_faceless();
    // A filled rectangle in a group, and one in a component placed off
    // the origin: each list's drawing reads back in its own frame.
    let mut boxed = |x0: f64| {
        let foot = Plane::from_point_normal(p(x0, 0., 0.), Vec3::new(0., 0., 1.)).unwrap();
        let s = doc.add_sketch(foot);
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in ring(&[
            p(x0, 0., 0.),
            p(x0 + 1., 0., 0.),
            p(x0 + 1., 1., 0.),
            p(x0, 1., 0.),
        ]) {
            sk.add_segment(a, b).unwrap();
        }
        let region = sk.regions().keys().next().unwrap();
        let (o, _) = doc.extrude_region(s, region, 1.).unwrap();
        let (g, _) = doc.group_nodes(&[kernel::NodeId::Object(o)]).unwrap();
        let lid = Plane::from_point_normal(p(x0, 0., 3.), Vec3::new(0., 0., 1.)).unwrap();
        let s = doc.add_sketch(lid);
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in ring(&[
            p(x0, 0., 3.),
            p(x0 + 0.5, 0., 3.),
            p(x0 + 0.5, 0.5, 3.),
            p(x0, 0.5, 3.),
        ]) {
            sk.add_segment(a, b).unwrap();
        }
        doc.set_sketch_group(s, Some(g)).unwrap();
        g
    };
    boxed(20.);
    let g = boxed(30.);
    let (_, inst, _) = doc.make_component(&[kernel::NodeId::Group(g)]).unwrap();
    doc.transform_instance(
        inst,
        &kernel::Transform::translation(Vec3::new(0., 10., 0.5)),
    )
    .unwrap();
    doc
}

/// A region drawn in Hew has a face, and a `.skp` Hew writes holds it as
/// a SketchUp face on the region's edges: reopened, the region is filled
/// again, exactly as before the save. A loop with no face stays a loop
/// alone, and nothing drawn in Hew comes back marked imported.
#[test]
fn drawn_regions_reopen_with_their_faces() {
    let source = drawn_regions_doc();
    let before = faced_regions(&source);
    assert_eq!(
        before.iter().filter(|s| s.contains("true)")).count(),
        6,
        "six filled shapes: {before:#?}"
    );
    assert!(
        before
            .iter()
            .any(|s| s.contains("d -4.0000") && s.contains("(4, 0, false)")),
        "the bare loop has no face: {before:#?}"
    );
    for target in common::all_targets() {
        let written = export(&source, target, WriteEnv::system()).expect("writes");
        let back = import(&written.bytes);
        assert_eq!(faced_regions(&back), before, "{target:?}");
        round_trip("drawn regions", &source, target).unwrap_or_else(|e| panic!("{e}"));
    }
}
