//! A document drawn in Hew — not read from a `.skp` — written to both
//! containers, checked at the level of the written file: what SketchUp
//! will see. (`roundtrip.rs` holds imported documents to the same
//! document after Hew -> `.skp` -> Hew; this file covers what the
//! importer does not read back yet: edges, curves, sketches.)
//!
//! `tests/fixtures/native.hew` holds: a block drilled through by a circle
//! (red base material), a cylinder with a glass top in a group with a hex
//! nut, the cylinder on the nested tag `Structure/Posts`, a hidden tag
//! `Hidden Stuff`, a component placed twice (once rotated), a guide line
//! and a guide point, two scenes, two loose sketches — a circle and a
//! rectangle — and attributes on the block, the component and the
//! document.

use std::path::Path;

use kernel::Document;
use skpwright::{DefinitionKind, Model};
mod common;

use skp_export::{Target, WriteEnv, export};

fn native() -> Document {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native.hew");
    Document::load(&std::fs::read(p).unwrap()).unwrap()
}

fn written(target: Target) -> (Model, skp_export::Export) {
    let out = export(
        &native(),
        target,
        WriteEnv {
            unix_time: 1_800_000_000,
            seed: 1,
        },
    )
    .unwrap();
    let m = Model::parse(&out.bytes).unwrap();
    assert!(
        m.diagnostics.iter().all(|d| !d.is_desync()),
        "{target:?}: {:?}",
        m.diagnostics
    );
    (m, out)
}

fn arcs(m: &Model) -> Vec<(f64, f64, u32)> {
    m.geometry
        .iter()
        .flat_map(|r| r.curves.iter())
        .filter_map(|c| {
            c.arc
                .as_ref()
                .map(|a| (a.radius_m, a.end_rad - a.start_rad, c.members))
        })
        .collect()
}

#[test]
fn circles_become_arc_curves() {
    for target in common::all_targets() {
        let (m, _) = written(target);
        let arcs = arcs(&m);
        let full = |r: f64, n: u32| {
            arcs.iter()
                .filter(|(rr, sweep, members)| {
                    (rr - r).abs() < 1e-9
                        && (sweep - std::f64::consts::TAU).abs() < 1e-9
                        && *members == n
                })
                .count()
        };
        // The drilled hole: its rim on the top and the bottom.
        assert_eq!(full(0.12, 24), 2, "{target:?}: hole rims in {arcs:?}");
        // The cylinder's two caps.
        assert_eq!(full(0.25, 32), 2, "{target:?}: cylinder caps in {arcs:?}");
        // The loose sketch circle.
        assert_eq!(full(0.3, 24), 1, "{target:?}: sketch circle in {arcs:?}");
        // Every member edge of every curve names it, oriented head to tail.
        for r in &m.geometry {
            for (ci, c) in r.curves.iter().enumerate() {
                let mut members: Vec<_> = r
                    .mesh
                    .edges
                    .iter()
                    .filter(|e| e.curve == Some(ci as u32))
                    .collect();
                assert_eq!(members.len() as u32, c.members, "{target:?}");
                // Oriented head to tail: on a closed curve every member
                // starts where another ends; an open one has one start.
                let ends: std::collections::BTreeSet<u32> = members.iter().map(|e| e.v1).collect();
                let starts = members.iter().filter(|e| !ends.contains(&e.v0)).count();
                let closed = c.arc.as_ref().is_some_and(|a| {
                    (a.end_rad - a.start_rad - std::f64::consts::TAU).abs() < 1e-9
                });
                assert_eq!(
                    starts,
                    usize::from(!closed),
                    "{target:?}: curve {ci}'s chain"
                );
                members.clear();
            }
        }
    }
}

#[test]
fn curved_walls_are_soft_and_smooth() {
    for target in common::all_targets() {
        let (m, _) = written(target);
        let soft: usize = m
            .geometry
            .iter()
            .flat_map(|r| r.mesh.edges.iter())
            .filter(|e| e.soft && e.smooth)
            .count();
        // 32 cylinder seams + 24 hole-wall seams.
        assert_eq!(soft, 32 + 24, "{target:?}");
    }
}

#[test]
fn structure_layers_and_scenes_survive() {
    for target in common::all_targets() {
        let (m, _) = written(target);
        let layer = |n: &str| m.layers.iter().find(|l| l.name == n);
        // The 2026 container has folders: the tag path Structure/Posts is
        // the folder Structure holding the layer Posts; a stream release
        // has one flat layer named by the joined path.
        if target == Target::Skp2026 {
            assert!(layer("Posts").is_some_and(|l| l.visible), "{target:?}");
            let folder = &m.features.layer_folders[0];
            assert_eq!(folder.name, "Structure", "{target:?}");
            assert_eq!(
                folder
                    .layers
                    .iter()
                    .map(|&i| m.layers[i].name.as_str())
                    .collect::<Vec<_>>(),
                ["Posts"],
                "{target:?}"
            );
        } else {
            assert!(
                layer("Structure/Posts").is_some_and(|l| l.visible),
                "{target:?}"
            );
        }
        assert!(
            layer("Hidden Stuff").is_some_and(|l| !l.visible),
            "{target:?}"
        );
        let names: Vec<&str> = m.scenes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Overview", "Top"], "{target:?}");
        assert!(
            m.scenes[1].camera.as_ref().is_some_and(|c| !c.perspective),
            "{target:?}"
        );
        // The component: one definition, placed twice from the root.
        let comp: Vec<_> = m
            .definitions
            .iter()
            .filter(|d| d.kind == DefinitionKind::Component)
            .collect();
        assert_eq!(comp.len(), 1, "{target:?}");
        let tree = m.scene();
        let placements = tree
            .iter()
            .filter(|n| n.definition.as_deref() == Some(comp[0].name.as_str()))
            .count();
        assert_eq!(placements, 2, "{target:?}");
        // The named objects and the group.
        for n in ["Drilled Block", "Post and Nut"] {
            assert!(
                m.instances.iter().any(|i| i.name.as_deref() == Some(n)),
                "{target:?}: {n}"
            );
        }
        // Guides: the line, and the point in the root's list.
        assert_eq!(m.guides.len(), 1, "{target:?}");
        let root = m.geometry.iter().find(|r| r.def_index.is_none()).unwrap();
        assert_eq!(root.guide_points.len(), 1, "{target:?}");
        // The loose sketches: the circle's face and the rectangle's.
        assert_eq!(root.mesh.faces.len(), 2, "{target:?}");
    }
}

#[test]
fn the_same_environment_writes_the_same_bytes() {
    for target in common::all_targets() {
        let env = WriteEnv {
            unix_time: 1_800_000_000,
            seed: 3,
        };
        let a = export(&native(), target, env).unwrap().bytes;
        let b = export(&native(), target, env).unwrap().bytes;
        assert!(
            a == b,
            "{target:?}: the writer is deterministic given its environment"
        );
    }
}

fn dict<'m>(
    m: &'m Model,
    owner: skpwright::Owner,
    name: &str,
) -> Option<&'m skpwright::Dictionary> {
    m.dictionaries
        .iter()
        .filter(|a| a.owner == owner)
        .flat_map(|a| a.dictionaries.iter())
        .find(|d| d.name == name)
}

fn entry<'d>(d: &'d skpwright::Dictionary, key: &str) -> Option<&'d skpwright::Value> {
    d.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

#[test]
fn attributes_and_the_hew_dictionary_are_written() {
    use skpwright::{Owner, Value};
    for target in common::all_targets() {
        let (m, _) = written(target);
        // The document's own attributes and Hew's model dictionary.
        let project = dict(&m, Owner::Model, "project").expect("document attributes");
        assert_eq!(
            entry(project, "client"),
            Some(&Value::Text("Test".into())),
            "{target:?}"
        );
        let hew = dict(&m, Owner::Model, "hew").expect("hew dictionary");
        assert_eq!(entry(hew, "version"), Some(&Value::Int(1)), "{target:?}");
        assert_eq!(
            entry(hew, "loose"),
            Some(&Value::Text("sketches".into())),
            "{target:?}"
        );
        let paths = format!("{:?}", entry(hew, "tag_paths"));
        let layer = if target == Target::Skp2026 {
            "Posts"
        } else {
            "Structure/Posts"
        };
        assert!(
            paths.contains(&format!("Text(\"{layer}\"), Array([Text(\"Structure\")"))
                && paths.contains("\"Posts\""),
            "{target:?}: {paths}"
        );
        // The block's placement: its attributes, a map as JSON.
        let (ri, pi) = m
            .geometry
            .iter()
            .enumerate()
            .flat_map(|(ri, r)| {
                r.placed
                    .iter()
                    .enumerate()
                    .map(move |(pi, p)| (ri, pi, p.pid))
            })
            .find(|&(_, _, pid)| {
                m.instances
                    .iter()
                    .any(|i| i.pid == pid && i.name.as_deref() == Some("Drilled Block"))
            })
            .map(|(ri, pi, _)| (ri, pi))
            .expect("the block's placement");
        let shop = dict(&m, Owner::Instance(ri, pi), "shop").expect("block attributes");
        assert_eq!(
            entry(shop, "stock"),
            Some(&Value::Text("Birch ply".into())),
            "{target:?}"
        );
        assert_eq!(
            entry(shop, "meta"),
            Some(&Value::Text(r#"{"count":3,"grain":"x"}"#.into())),
            "{target:?}"
        );
        assert_eq!(
            entry(shop, "hew_json"),
            Some(&Value::Array(vec![Value::Text("meta".into())])),
            "{target:?}"
        );
        let kind = dict(&m, Owner::Instance(ri, pi), "hew").and_then(|d| entry(d, "kind"));
        assert_eq!(kind, Some(&Value::Text("object".into())), "{target:?}");
        // The component definition's attributes.
        let di = m
            .definitions
            .iter()
            .position(|d| d.kind == DefinitionKind::Component)
            .unwrap();
        let parts = dict(&m, Owner::Definition(di), "parts").expect("component attributes");
        assert_eq!(entry(parts, "sku"), Some(&Value::Int(4021)), "{target:?}");
        // Cylinder walls name their surface: 32 of the post, 24 of the hole.
        let surfaces = m
            .dictionaries
            .iter()
            .filter(|a| matches!(a.owner, Owner::Face(..)))
            .filter(|a| a.dictionaries.iter().any(|d| d.name == "hew"))
            .count();
        assert_eq!(surfaces, 32 + 24, "{target:?}");
    }
}

#[test]
fn dimensions_and_leader_text_are_written() {
    use kernel::{Anchor, Plane, Point3, Vec3};
    let mut doc = native();
    let a = Anchor {
        node: None,
        point: Point3::new(0.0, 0.0, 0.0),
    };
    let b = Anchor {
        node: None,
        point: Point3::new(1.0, 0.0, 0.0),
    };
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    doc.add_linear_dimension(a, b, Vec3::new(0.0, -0.2, 0.0), ground, None)
        .unwrap();
    doc.add_leader_text(
        Anchor {
            node: None,
            point: Point3::new(2.0, 0.3, 0.8),
        },
        Vec3::new(0.3, 0.0, 0.3),
        "Glass top".into(),
    )
    .unwrap();
    for target in common::all_targets() {
        let out = export(
            &doc,
            target,
            WriteEnv {
                unix_time: 1_800_000_000,
                seed: 2,
            },
        )
        .unwrap();
        let m = Model::parse(&out.bytes).unwrap();
        assert_eq!(m.dimensions.len(), 1, "{target:?}");
        let d = &m.dimensions[0];
        let p = |a: &skpwright::Anchor| a.point_m.map(|v| (v * 1e6).round() as i64);
        assert_eq!(
            (p(&d.start), p(&d.end)),
            ([0, 0, 0], [1_000_000, 0, 0]),
            "{target:?}"
        );
        // The dimension line lies 0.2 m toward −Y: offset along n × x.
        let n = d.normal;
        let x = d.x_axis;
        let side = [
            n[1] * x[2] - n[2] * x[1],
            n[2] * x[0] - n[0] * x[2],
            n[0] * x[1] - n[1] * x[0],
        ];
        let off: Vec<i64> = side
            .iter()
            .map(|s| (s * d.offset_m * 1e6).round() as i64)
            .collect();
        assert_eq!(off, [0, -200_000, 0], "{target:?}");
        assert_eq!(m.texts.len(), 1, "{target:?}");
        assert_eq!(m.texts[0].content, "Glass top", "{target:?}");
        // Points in space, as SketchUp stores them (kind 0 is no anchor at
        // all, which SketchUp draws at the origin).
        let kinds = [d.start.kind, d.end.kind, m.texts[0].anchor.kind];
        assert_eq!(kinds, [1, 1, 1], "{target:?}");
    }
}

fn annotated() -> Document {
    use kernel::{Anchor, CapturedCurve, Plane, Point3, RadialKind, Vec3};
    let mut doc = native();
    let free = |p: Point3| Anchor {
        node: None,
        point: p,
    };
    let flat = |z: f64| {
        Plane::from_point_normal(Point3::new(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).unwrap()
    };
    doc.add_linear_dimension(
        free(Point3::new(0.0, 0.0, 0.0)),
        free(Point3::new(1.0, 0.0, 0.0)),
        Vec3::new(0.0, -0.2, 0.0),
        flat(0.0),
        Some("1 m".into()),
    )
    .unwrap();
    doc.add_leader_text(
        free(Point3::new(2.0, 0.3, 0.8)),
        Vec3::new(0.3, 0.0, 0.3),
        "Glass top".into(),
    )
    .unwrap();
    // On the post's drawn top rim, and on a circle nothing draws.
    for (c, r, kind) in [
        (Point3::new(2.0, 0.3, 0.8), 0.25, RadialKind::Diameter),
        (Point3::new(9.0, 9.0, 0.0), 0.5, RadialKind::Radius),
    ] {
        doc.add_radial_dimension(
            free(Point3::new(c.x + r, c.y, c.z)),
            kind,
            CapturedCurve {
                center: c,
                radius: r,
                plane: flat(c.z),
            },
            Vec3::new(1.0, 0.0, 0.0),
            None,
        )
        .unwrap();
    }
    doc
}

#[test]
fn annotations_survive_hew_to_skp_to_hew() {
    let doc = annotated();
    for target in common::all_targets() {
        let out = export(
            &doc,
            target,
            WriteEnv {
                unix_time: 1_800_000_000,
                seed: 4,
            },
        )
        .unwrap();
        let m = Model::parse(&out.bytes).unwrap();
        assert_eq!(
            (m.dimensions.len(), m.texts.len(), m.radial_dimensions.len()),
            (1, 1, 1),
            "{target:?}: the drawn circle's radius dimension is a SketchUp one"
        );
        let r = &m.radial_dimensions[0];
        assert!(
            r.diameter && r.edge.is_some() && r.def_index.is_some(),
            "{target:?}: {r:?}"
        );
        let back = skp_import::import(&out.bytes).unwrap();
        let mut d2 = Document::new();
        d2.ingest(back.scene, back.textures_missing).unwrap();
        let (a, b) = (
            common::digest(&doc).annotations,
            common::digest(&d2).annotations,
        );
        assert_eq!(a, b, "{target:?}");
    }
}

#[test]
fn a_hidden_object_comes_back_a_hidden_object() {
    use kernel::NodeId;
    let mut doc = native();
    let block = doc
        .visible_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some("Drilled Block"))
        .unwrap();
    doc.set_node_user_hidden(NodeId::Object(block), true);
    for target in common::all_targets() {
        let out = export(
            &doc,
            target,
            WriteEnv {
                unix_time: 1_800_000_000,
                seed: 6,
            },
        )
        .unwrap();
        let back = skp_import::import(&out.bytes).unwrap();
        let mut d2 = Document::new();
        d2.ingest(back.scene, back.textures_missing).unwrap();
        // Same node kinds, hidden state and attributes (no wrapper group,
        // no doubled dictionary).
        assert_eq!(
            common::digest(&doc).roots,
            common::digest(&d2).roots,
            "{target:?}"
        );
    }
}

/// Skpwright's rings all run counter-clockwise about the face normal, holes
/// included — its 2026 writer turns holes back to SketchUp's clockwise
/// storage. (A reader normalizes every ring, so this is checked on the
/// model before it is written.)
#[test]
fn hole_rings_are_handed_to_the_writer_counter_clockwise() {
    let authored = skp_export::author(&native());
    let mut holes = 0;
    for r in &authored.model.geometry {
        let v = &r.mesh.vertices;
        for f in &r.mesh.faces {
            for ring in std::iter::once(&f.outer).chain(&f.holes) {
                let mut n = [0.0f64; 3];
                for k in 0..ring.len() {
                    let (a, b) = (v[ring[k] as usize], v[ring[(k + 1) % ring.len()] as usize]);
                    n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                    n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                    n[2] += (a[0] - b[0]) * (a[1] + b[1]);
                }
                let d = n[0] * f.normal[0] + n[1] * f.normal[1] + n[2] * f.normal[2];
                assert!(d > 0.0, "a ring runs clockwise: {ring:?}");
            }
            holes += f.holes.len();
        }
    }
    assert!(holes >= 2, "the drilled block's top and bottom have holes");
}

#[test]
fn a_written_file_opens_in_the_display_units() {
    // Each of Hew's display formats, in both containers: SketchUp's Model
    // Info ▸ Units shows the same unit, format, precision and snapping.
    use skp_export::{DisplayUnits, export_in};
    // Each with its snapping step in meters: SketchUp keeps it in inches
    // whatever the unit (a 0.1 mm step is 0.003937").
    let cases = [
        ("m", (0, 4, 3), 0.001),
        ("cm", (0, 3, 2), 0.0001),
        ("mm", (0, 2, 1), 0.0001),
        ("arch", (1, 0, 4), 0.0625 * 0.0254),
        ("frac_in", (3, 0, 4), 0.0625 * 0.0254),
        ("dec_in", (0, 0, 3), 0.001 * 0.0254),
    ];
    for (name, want, step_m) in cases {
        let units = DisplayUnits::from_name(name);
        assert!(units.is_some(), "{name}");
        for target in common::all_targets() {
            let out = export_in(&native(), target, WriteEnv::system(), units).unwrap();
            let u = Model::parse(&out.bytes).unwrap().units;
            assert_eq!(
                (u.length_format, u.length_unit, u.length_precision),
                want,
                "{name} {target:?}"
            );
            assert!(u.length_snap && u.angle_snap, "{name} {target:?}");
            assert!(
                (u.length_snap_length * 0.0254 - step_m).abs() < 1e-12,
                "{name} {target:?}: snaps {}\"",
                u.length_snap_length
            );
        }
    }
    assert_eq!(DisplayUnits::from_name("furlongs"), None);
}

#[test]
fn every_definition_hew_wrote_says_so_in_one_dictionary() {
    // A reader takes a definition marked `built` as written (no repair);
    // the mark sits in the definition's one `hew` dictionary, beside any
    // other entry (an unnamed lone solid's `unnamed`).
    for target in common::all_targets() {
        let (m, _) = written(target);
        assert!(!m.definitions.is_empty());
        for di in 0..m.definitions.len() {
            let hew: Vec<_> = m
                .dictionaries
                .iter()
                .filter(|a| a.owner == skpwright::Owner::Definition(di))
                .flat_map(|a| a.dictionaries.iter())
                .filter(|d| d.name == "hew")
                .collect();
            assert_eq!(hew.len(), 1, "{target:?} definition {di}");
            assert!(
                hew[0]
                    .entries
                    .iter()
                    .any(|(k, v)| k == "built" && *v == skpwright::Value::Bool(true)),
                "{target:?} definition {di}: {:?}",
                hew[0].entries
            );
        }
    }
}

/// A placement's own material is written on the placement: repainting a
/// component instance changes the written file, and clearing the paint
/// restores it byte for byte.
#[test]
fn a_repainted_placement_writes_its_own_material() {
    let env = WriteEnv {
        unix_time: 1_800_000_000,
        seed: 1,
    };
    let plain = export(&native(), Target::Skp2026, env).unwrap().bytes;

    let mut doc = native();
    let instance = *doc.instance_ids().first().expect("a placed component");
    let (material, _) = doc.materials().iter().next().expect("a palette material");
    doc.set_instance_material(instance, Some(material)).unwrap();
    let painted = export(&doc, Target::Skp2026, env).unwrap().bytes;
    assert_ne!(plain, painted, "the placement's paint reaches the file");

    doc.set_instance_material(instance, None).unwrap();
    let cleared = export(&doc, Target::Skp2026, env).unwrap().bytes;
    assert_eq!(plain, cleared, "clearing it restores the file");
}

/// Each stream target writes its own release, and `target_of` names it
/// back from the written bytes, for every release `skpwright` lists.
#[test]
fn each_stream_target_writes_its_own_release() {
    for release in skpwright::Release::ALL {
        let target = Target::Stream(release);
        let (m, out) = written(target);
        assert_eq!(m.release(), Some(release), "{target:?}");
        assert_eq!(skp_export::target_of(&out.bytes), target);
        let (version, _) = skpwright::header_info(&out.bytes).unwrap();
        assert_eq!(skpwright::Release::of_version(&version), Some(release));
    }
    let (_, out) = written(Target::Skp2026);
    assert_eq!(skp_export::target_of(&out.bytes), Target::Skp2026);
}

/// A stream file of a release Skpwright does not list saves back as the
/// nearest release it writes, and the writer's notes say so; a listed
/// release raises no such note.
#[test]
fn an_unlisted_release_saves_as_the_nearest_written_one() {
    let (_, out) = written(Target::Stream(skpwright::Release::R2017));
    let stamp = |major: &str| {
        let mut b = out.bytes.clone();
        // The header's UTF-16 version record: "{17." at 36.
        assert_eq!(b[36..44], [0x7b, 0, 0x31, 0, 0x37, 0, 0x2e, 0]);
        b[38] = major.as_bytes()[0];
        b[40] = major.as_bytes()[1];
        b
    };
    for (major, nearest) in [("21", 20u8), ("25", 20), ("10", 8), ("12", 13), ("99", 20)] {
        let b = stamp(major);
        let t = skp_export::target_of(&b);
        assert_eq!(
            t.release().map(|r| r.number()),
            Some(nearest),
            "{major} -> {t:?}"
        );
        let note = skp_export::unlisted_note(&b, t.release().unwrap()).expect("a note");
        assert!(
            note.contains(&format!("{{{major}.")) && note.contains(&format!("release {nearest}")),
            "{note}"
        );
    }
    assert_eq!(
        skp_export::unlisted_note(&stamp("17"), skpwright::Release::R2017),
        None
    );
    assert_eq!(
        skp_export::unlisted_note(&stamp("20"), skpwright::Release::R2020),
        None
    );
}

/// A Hew document writes in every release `skpwright` lists and reopens as
/// the same shapes: the container and the release number are the
/// target's, the Object count survives, and the file imports whole. (The
/// oldest releases hold less than the newer ones; the notes say what.)
#[test]
fn every_release_writes_and_reopens() {
    let doc = native();
    let objects = doc.live_object_ids().len();
    assert!(objects > 0);
    for release in skpwright::Release::ALL {
        let target = Target::Stream(release);
        let (model, out) = written(target);
        assert_eq!(
            skpwright::detect_container(&out.bytes),
            skpwright::Container::Stream,
            "{target:?}"
        );
        assert_eq!(model.release(), Some(release), "{target:?}");
        assert_eq!(skp_export::target_of(&out.bytes), target);
        let scene = skp_import::import(&out.bytes)
            .unwrap_or_else(|e| panic!("release {}: {e}", release.number()));
        assert!(
            scene.warnings.is_empty(),
            "release {}: {:?}",
            release.number(),
            scene.warnings
        );
        let mut back = Document::new();
        back.ingest(scene.scene, scene.textures_missing).unwrap();
        // Release 3 holds no hidden state: the fixture's one hidden Object
        // comes back visible.
        let expected = objects + usize::from(release == skpwright::Release::R3);
        assert_eq!(
            back.live_object_ids().len(),
            expected,
            "release {}: Object count",
            release.number()
        );
    }
}
