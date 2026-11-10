//! Saving back over the `.skp` a document was opened from: an unchanged
//! open-and-save keeps the file's structure — every definition and its
//! geometry as the file has it (soft edges, curves, texture placements,
//! faces' layers and back materials), placements and their ids, materials,
//! layers, scenes, styles and dictionaries — and an edit changes what it
//! touches and nothing else. Each source is opened as the app opens one
//! (`skp_import::import`, `ingest`, `forget_history`,
//! `SkpSource::after_open`).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use common::digest;
use kernel::{Document, NodeId};
use skp_export::{SkpSource, Target, WriteEnv, save};
use skpwright::{Model, Owner};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../skp-import/tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn all_fixtures() -> Vec<(String, Vec<u8>)> {
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

/// Open `bytes` as the app does.
fn open(bytes: &[u8]) -> Option<(Document, SkpSource)> {
    let out = skp_import::import(bytes).ok()?;
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    doc.forget_history().unwrap();
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    Some((doc, src))
}

fn container(bytes: &[u8]) -> Target {
    skp_export::target_of(bytes)
}

fn env() -> WriteEnv {
    WriteEnv {
        unix_time: 1_800_000_000,
        seed: 5,
    }
}

// ── A structural digest of a file ─────────────────────────────────────────

#[derive(Debug, PartialEq)]
struct FileD {
    /// Name, kind -> (vertices, edges, faces, soft, curves, textured faces,
    /// back-painted faces, placements), per definition; the root's under
    /// "" (its placements by pid).
    runs: BTreeMap<(String, String), Vec<usize>>,
    /// The root's placements by what they place (ids can change: a 2017
    /// file reuses ids inside definitions, and a root placement holding
    /// one takes a fresh id on any rewrite).
    root_placed: Vec<String>,
    /// Root placement ids no other entity of the file shares.
    root_pids: BTreeSet<u32>,
    materials: Vec<String>,
    layers: Vec<(String, bool)>,
    scenes: Vec<String>,
    styles: usize,
    /// Owner kind, dictionary name — `hew` aside (a save writes its own).
    dictionaries: BTreeMap<(String, String), usize>,
    texts: usize,
    dimensions: usize,
    sections: usize,
}

fn file_digest(m: &Model) -> FileD {
    let mut runs = BTreeMap::new();
    let name_of = |map: usize| {
        m.definitions
            .iter()
            .find(|d| d.map_index == Some(map))
            .map(|d| (d.name.clone(), format!("{:?}", d.kind)))
            .unwrap_or_default()
    };
    let mut root_pids = BTreeSet::new();
    let mut root_placed = Vec::new();
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for r in &m.geometry {
        for pid in r
            .mesh
            .vertex_pids
            .iter()
            .chain(r.mesh.edges.iter().map(|e| &e.pid))
            .chain(r.mesh.faces.iter().map(|f| &f.pid))
            .chain(r.placed.iter().map(|p| &p.pid))
        {
            *counts.entry(*pid).or_insert(0) += 1;
        }
    }
    for r in &m.geometry {
        let key = match r.def_index {
            Some(map) => name_of(map),
            None => {
                root_pids.extend(r.placed.iter().map(|p| p.pid).filter(|p| counts[p] == 1));
                root_placed.extend(r.placed.iter().map(|p| {
                    let (n, k) = name_of(p.defref as usize);
                    // Group definitions take names the writer may choose.
                    if k == "Group" { "group".to_string() } else { n }
                }));
                root_placed.sort();
                (String::new(), "root".into())
            }
        };
        // Group definitions repeat names ("Group#3"); fold them by name.
        let counts = vec![
            r.mesh.vertices.len(),
            r.mesh.edges.len(),
            r.mesh.faces.len(),
            r.mesh.edges.iter().filter(|e| e.soft).count(),
            r.curves.len(),
            r.mesh.faces.iter().filter(|f| f.texture.is_some()).count(),
            r.mesh
                .faces
                .iter()
                .filter(|f| f.back_material.is_some())
                .count(),
            r.placed.len(),
        ];
        runs.entry(key)
            .and_modify(|c: &mut Vec<usize>| {
                for (a, b) in c.iter_mut().zip(&counts) {
                    *a += b
                }
            })
            .or_insert(counts);
    }
    let mut dictionaries = BTreeMap::new();
    for a in &m.dictionaries {
        let kind = format!("{:?}", a.owner)
            .split('(')
            .next()
            .unwrap()
            .to_string();
        for d in &a.dictionaries {
            if d.name != "hew" {
                *dictionaries
                    .entry((kind.clone(), d.name.clone()))
                    .or_insert(0) += 1;
            }
        }
    }
    FileD {
        runs,
        root_placed,
        root_pids,
        // A solid's color alpha is derived from its opacity on write
        // (Skpwright `docs/WRITING.md`); SketchUp reads both the same way.
        materials: m
            .materials
            .iter()
            .map(|x| {
                let mut x = x.clone();
                if let skpwright::Material::Solid { rgba, .. } = &mut x {
                    rgba[3] = 0;
                }
                format!("{x:?}").chars().take(160).collect()
            })
            .collect(),
        layers: m
            .layers
            .iter()
            .map(|l| (l.name.clone(), l.visible))
            .collect(),
        scenes: m
            .scenes
            .iter()
            .map(|s| {
                format!(
                    "{} {:?} {:?} {}",
                    s.name,
                    s.saved,
                    s.style,
                    s.hidden_entities.len()
                )
            })
            .collect(),
        styles: m.styles.len(),
        dictionaries,
        texts: m.texts.len(),
        dimensions: m.dimensions.len(),
        sections: m.geometry.iter().map(|r| r.sections.len()).sum(),
    }
}

fn first_diff(a: &FileD, b: &FileD) -> String {
    let mut out = String::new();
    macro_rules! cmp {
        ($f:ident) => {
            if a.$f != b.$f {
                out += &format!("{}: {:?}\n  != {:?}\n", stringify!($f), a.$f, b.$f);
            }
        };
    }
    cmp!(runs);
    cmp!(root_placed);
    // Every id the file held alone, it still holds.
    if !a.root_pids.is_subset(&b.root_pids) {
        out += &format!(
            "root ids lost: {:?}\n",
            a.root_pids.difference(&b.root_pids).collect::<Vec<_>>()
        );
    }
    cmp!(materials);
    cmp!(layers);
    cmp!(scenes);
    cmp!(styles);
    cmp!(dictionaries);
    cmp!(texts);
    cmp!(dimensions);
    cmp!(sections);
    out.chars().take(4000).collect()
}

#[test]
fn an_unchanged_save_keeps_the_file() {
    let mut failures = Vec::new();
    let mut n = 0;
    for (label, bytes) in all_fixtures() {
        let Some((doc, src)) = open(&bytes) else {
            continue;
        };
        let target = container(&bytes);
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let before = Model::parse(&bytes).unwrap();
        let after = Model::parse(&saved.export.bytes).unwrap();
        n += 1;
        if after.diagnostics.iter().any(|d| d.is_desync()) {
            failures.push(format!("{label}: the save reads with desyncs"));
            continue;
        }
        let (a, b) = (file_digest(&before), file_digest(&after));
        let d = first_diff(&a, &b);
        if !d.is_empty() {
            failures.push(format!("{label}:\n{d}"));
        }
        // And it opens as the same document.
        let (back, _) = open(&saved.export.bytes).unwrap();
        let (x, y) = (digest(&doc), digest(&back));
        if x != y {
            let mut why = String::new();
            macro_rules! f {
                ($f:ident) => {
                    if x.$f != y.$f {
                        let (p, q) = (format!("{:?}", x.$f), format!("{:?}", y.$f));
                        let at = p.chars().zip(q.chars()).take_while(|(a, b)| a == b).count();
                        why += &format!(
                            " {} differs at {at}: ...{}... vs ...{}...",
                            stringify!($f),
                            p.chars()
                                .skip(at.saturating_sub(150))
                                .take(300)
                                .collect::<String>(),
                            q.chars()
                                .skip(at.saturating_sub(150))
                                .take(300)
                                .collect::<String>()
                        );
                    }
                };
            }
            f!(roots);
            f!(tags);
            f!(guides);
            f!(section);
            f!(camera);
            f!(scenes);
            f!(sketches);
            f!(attrs);
            failures.push(format!("{label}: reopens as another document:{why}"));
        }
    }
    assert!(n >= 40, "{n}");
    assert!(
        failures.is_empty(),
        "{} of {n}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a_second_unchanged_save_writes_the_same_model() {
    for name in ["house.skp", "2026/house.skp", "feature-pack.skp"] {
        let bytes = fixture(name);
        let (doc, src) = open(&bytes).unwrap();
        let target = container(&bytes);
        let first = save(&doc, target, env(), Some(&src)).unwrap();
        let second = save(&doc, target, env(), Some(&first.source)).unwrap();
        let (a, b) = (
            file_digest(&Model::parse(&first.export.bytes).unwrap()),
            file_digest(&Model::parse(&second.export.bytes).unwrap()),
        );
        let d = first_diff(&a, &b);
        assert!(d.is_empty(), "{name}:\n{d}");
    }
}

/// The top-level Object called `name`.
fn object_named(doc: &Document, name: &str) -> kernel::ObjectId {
    doc.visible_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some(name))
        .unwrap_or_else(|| panic!("no object '{name}'"))
}

#[test]
fn moving_one_object_rewrites_only_that_object() {
    let bytes = fixture("house.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let wall = object_named(&doc, "Back Wall");
    doc.transform_object(
        wall,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.5, 0.0)),
    )
    .unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let before = file_digest(&Model::parse(&bytes).unwrap());
    let after_model = Model::parse(&saved.export.bytes).unwrap();
    let after = file_digest(&after_model);
    // Every definition but the wall's keeps its content; the wall's group
    // definition is written anew (one new group definition).
    assert_eq!(before.materials, after.materials);
    assert_eq!(before.layers, after.layers);
    assert_eq!(before.scenes, after.scenes);
    assert!(
        before.root_pids.is_subset(&after.root_pids),
        "unshared ids are kept"
    );
    let (back, _) = open(&saved.export.bytes).unwrap();
    assert_eq!(digest(&doc), digest(&back), "the move is in the file");
}

#[test]
fn deleting_and_adding_objects_lands_in_the_file() {
    let bytes = fixture("2026/house.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let wall = object_named(&doc, "Back Wall");
    doc.delete_node(NodeId::Object(wall)).unwrap();
    let s = doc.add_sketch(
        kernel::Plane::from_point_normal(kernel::Point3::ORIGIN, kernel::Vec3::new(0.0, 0.0, 1.0))
            .unwrap(),
    );
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let p = |x: f64, y: f64| kernel::Point3::new(x, y, 0.0);
        for (a, b) in [
            ((20.0, 20.0), (21.0, 20.0)),
            ((21.0, 20.0), (21.0, 21.0)),
            ((21.0, 21.0), (20.0, 21.0)),
            ((20.0, 21.0), (20.0, 20.0)),
        ] {
            sk.add_segment(p(a.0, a.1), p(b.0, b.1)).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 0.5).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let (back, _) = open(&saved.export.bytes).unwrap();
    assert_eq!(digest(&doc), digest(&back));
    let before = file_digest(&Model::parse(&bytes).unwrap());
    let after = file_digest(&Model::parse(&saved.export.bytes).unwrap());
    assert_eq!(before.scenes.len(), after.scenes.len());
    assert_eq!(
        before.root_pids.len(),
        after.root_pids.len(),
        "one gone, one new"
    );
    assert_ne!(before.root_pids, after.root_pids);
}

#[test]
fn a_material_opacity_and_a_tag_visibility_change_in_place() {
    let bytes = fixture("house.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let (mid, m) = doc
        .materials()
        .iter()
        .find(|(_, m)| m.texture.is_none() && m.color.a == 255)
        .map(|(id, m)| (id, m.clone()))
        .unwrap();
    doc.set_material_alpha(mid, 128).unwrap();
    let tag = doc.tag_meta().next().map(|(p, _)| p.to_vec()).unwrap();
    let was_hidden = doc.tag_hidden(&tag);
    doc.set_tag_hidden(tag.clone(), !was_hidden);
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let rec = after
        .materials
        .iter()
        .find(|x| matches!(x, skpwright::Material::Solid { name, .. } if *name == m.name))
        .unwrap();
    assert!(
        matches!(rec, skpwright::Material::Solid { opacity, .. } if (*opacity - 128.0 / 255.0).abs() < 1e-3),
        "{rec:?}"
    );
    let layer = after.layers.iter().find(|l| l.name == tag[0]).unwrap();
    assert_eq!(layer.visible, was_hidden);
    let before = file_digest(&Model::parse(&bytes).unwrap());
    let after = file_digest(&after);
    assert_eq!(before.runs, after.runs, "no geometry rewritten");
}

#[test]
fn a_file_without_provenance_is_written_whole() {
    // A source whose provenance is empty (a raw file) cannot tell what is
    // unchanged: the save writes the document as it stands, never doubling
    // content.
    let bytes = fixture("house.skp");
    let (doc, src) = open(&bytes).unwrap();
    let bare = SkpSource {
        bytes: src.bytes.clone(),
        nodes: Default::default(),
        defs: Default::default(),
        loose: 0,
        annotations: Default::default(),
        unread: None,
        layers: BTreeMap::new(),
        scenes: BTreeMap::new(),
        loose_sketches: BTreeMap::new(),
    };
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&bare),
    )
    .unwrap();
    let (back, _) = open(&saved.export.bytes).unwrap();
    // Written whole, poses are baked in and taken out again: a micron of
    // rounding, as in the round-trip suite.
    let (a, b) = (
        format!("{:?}", digest(&doc)),
        format!("{:?}", digest(&back)),
    );
    assert!(
        a == b || common::near(&a, &b),
        "the document doubled or changed"
    );
}

#[test]
fn dictionaries_on_reused_placements_survive() {
    let bytes = fixture("feature-pack.skp");
    let before = Model::parse(&bytes).unwrap();
    let (doc, src) = open(&bytes).unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let count = |m: &Model| {
        m.dictionaries
            .iter()
            .filter(|a| matches!(a.owner, Owner::Definition(_) | Owner::Instance(..)))
            .flat_map(|a| a.dictionaries.iter())
            .filter(|d| d.name == "dynamic_attributes")
            .count()
    };
    assert!(count(&before) > 0, "the fixture holds a dynamic component");
    assert_eq!(count(&before), count(&after));
}

#[test]
fn saving_again_does_not_double_annotations() {
    // A document with Hew's own dimensions and texts, written and opened
    // as a .skp: each save writes them once, in place of the last save's.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native.hew");
    let mut doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let free = |x: f64| kernel::Anchor {
        node: None,
        point: kernel::Point3::new(x, 0.0, 0.0),
    };
    doc.add_leader_text(free(1.0), kernel::Vec3::new(0.1, 0.1, 0.0), "Note".into())
        .unwrap();
    let first = save(&doc, Target::Skp2026, env(), None).unwrap();
    let (mut opened, src) = open(&first.export.bytes).unwrap();
    assert_eq!(
        opened.annotation_ids().len(),
        1,
        "the text reads back as Hew's"
    );
    let second = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
    let third = save(&opened, Target::Skp2026, env(), Some(&second.source)).unwrap();
    for bytes in [&second.export.bytes, &third.export.bytes] {
        assert_eq!(Model::parse(bytes).unwrap().texts.len(), 1);
    }
    // Deleting it in Hew deletes it from the file.
    let id = opened.annotation_ids()[0];
    opened.delete_annotation(id).unwrap();
    let fourth = save(&opened, Target::Skp2026, env(), Some(&third.source)).unwrap();
    assert!(Model::parse(&fourth.export.bytes).unwrap().texts.is_empty());
}

#[test]
fn a_definitions_guide_point_is_not_added_again_on_each_save() {
    // A component whose definition holds a guide point: Hew shows it once
    // per placement, in the world. Each save keeps it in the definition
    // and adds none of those placements' copies to the model's own list.
    // (Each save added them all again, so the file grew with every save.)
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let mut model = skp_export::author(&doc).model;
    // A definition placed at the top level (the merge rewrites those
    // placements, so the guide must be read before it does).
    let top: BTreeSet<usize> = model
        .scene()
        .iter()
        .filter_map(|n| model.geometry[n.run?].def_index)
        .collect();
    let run = model
        .geometry
        .iter()
        .position(|r| r.def_index.is_some_and(|d| top.contains(&d)))
        .expect("a definition placed at the top level");
    model.geometry[run]
        .guide_points
        .push(skpwright::GuidePoint {
            pid: 900_001,
            point_m: [0.01, 0.02, 0.03],
            anchor_m: [0.01, 0.02, 0.03],
            ..Default::default()
        });
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let points = |d: &Document| {
        d.guide_ids()
            .into_iter()
            .filter(|&g| matches!(d.guide(g), Some(kernel::Guide::Point { .. })))
            .count()
    };
    let (opened, mut src) = open(&bytes).unwrap();
    let shown = points(&opened);
    assert!(shown >= 1, "the definition's point shows");
    for generation in 1..=3 {
        let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
        let m = Model::parse(&saved.export.bytes).unwrap();
        let in_defs: usize = m
            .geometry
            .iter()
            .filter(|r| r.def_index.is_some())
            .map(|r| r.guide_points.len())
            .sum();
        let at_root: usize = m
            .geometry
            .iter()
            .filter(|r| r.def_index.is_none())
            .map(|r| r.guide_points.len())
            .sum();
        // The zoo's own guide point is the model's one.
        assert_eq!((in_defs, at_root), (1, 1), "save {generation}");
        let (reopened, _) = open(&saved.export.bytes).unwrap();
        assert_eq!(points(&reopened), shown, "save {generation}");
        src = saved.source;
    }
}

#[test]
fn loose_geometry_keeps_its_layers_when_rewritten() {
    // layers.skp: three loose boxes, each on its own layer. Each opens as
    // an Object tagged with its layer; moving one rewrites the loose
    // geometry, and every box's faces stay on their layer (the moved one's
    // through its tag).
    let bytes = fixture("layers.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let loose: Vec<kernel::ObjectId> = doc
        .top_level_nodes()
        .into_iter()
        .filter_map(|n| match n {
            NodeId::Object(o) => Some(o),
            _ => None,
        })
        .collect();
    let mut tags: Vec<String> = loose
        .iter()
        .map(|&o| doc.node_tags(NodeId::Object(o)).concat().join("/"))
        .collect();
    tags.sort();
    assert_eq!(tags, ["BigLayer", "LittleLayer", "MediumLayer"]);
    let layers_by_z = |bytes: &[u8]| {
        let m = Model::parse(bytes).unwrap();
        let mut out: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
        for r in m.geometry.iter().filter(|r| r.def_index.is_none()) {
            for f in &r.mesh.faces {
                let name = m
                    .layer_of(f.layer)
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                let x = f
                    .outer
                    .iter()
                    .map(|&v| r.mesh.vertices[v as usize][0])
                    .fold(f64::MAX, f64::min);
                out.entry(name)
                    .or_default()
                    .insert((x * 1e3).round() as i64);
            }
        }
        out
    };
    let before = layers_by_z(&bytes);
    doc.transform_object(
        loose[0],
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 1.0)),
    )
    .unwrap();
    let saved = save(&doc, container(&bytes), env(), Some(&src)).unwrap();
    // (Each box's faces by their smallest x: a move up keeps x.)
    assert_eq!(layers_by_z(&saved.export.bytes), before);
    let (back, _) = open(&saved.export.bytes).unwrap();
    let mut back_tags: Vec<String> = back
        .top_level_nodes()
        .into_iter()
        .map(|n| back.node_tags(n).concat().join("/"))
        .collect();
    back_tags.sort();
    assert_eq!(back_tags, tags);
}

#[test]
fn a_textured_materials_average_color_survives_a_save() {
    // SketchUp draws a textured material in its average color with
    // textures off; Hew keeps none of its own, so a save keeps the file's.
    let avg = |bytes: &[u8]| -> BTreeMap<String, Option<[u8; 4]>> {
        Model::parse(bytes)
            .unwrap()
            .materials
            .iter()
            .filter_map(|m| match m {
                skpwright::Material::Textured { name, avg_rgba, .. } => {
                    Some((name.clone(), *avg_rgba))
                }
                _ => None,
            })
            .collect()
    };
    for name in ["house.skp", "2026/house.skp"] {
        let bytes = fixture(name);
        let (doc, src) = open(&bytes).unwrap();
        let saved = save(&doc, container(&bytes), env(), Some(&src)).unwrap();
        let before = avg(&bytes);
        assert!(
            before.values().any(|c| c.is_some_and(|c| c != [255; 4])),
            "{name}"
        );
        assert_eq!(avg(&saved.export.bytes), before, "{name}");
    }
}

#[test]
fn a_loose_objects_new_tag_and_hidden_state_reach_its_faces() {
    // layers.skp's loose boxes, none moved: one retagged, one hidden. Loose
    // geometry has no placement, so its faces carry both.
    let bytes = fixture("layers.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let tagged = |doc: &Document, name: &str| {
        doc.top_level_nodes()
            .into_iter()
            .find(|&n| doc.node_tags(n).concat().join("/") == name)
            .unwrap()
    };
    let big = tagged(&doc, "BigLayer");
    let medium = tagged(&doc, "MediumLayer");
    doc.remove_node_tag(big, &["BigLayer".to_string()]).unwrap();
    doc.add_node_tag(big, vec!["LittleLayer".to_string()])
        .unwrap();
    doc.set_node_user_hidden(medium, true);
    let saved = save(&doc, container(&bytes), env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let root = m.geometry.iter().find(|r| r.def_index.is_none()).unwrap();
    let mut by_layer: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for f in &root.mesh.faces {
        let name = m
            .layer_of(f.layer)
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let e = by_layer.entry(name).or_default();
        e.0 += 1;
        e.1 += usize::from(f.hidden);
    }
    // Six faces a box: the big one joined the little one's layer; the
    // medium one's faces are hidden.
    assert_eq!(by_layer.get("BigLayer"), None, "{by_layer:?}");
    assert_eq!(by_layer.get("LittleLayer"), Some(&(12, 0)), "{by_layer:?}");
    assert_eq!(by_layer.get("MediumLayer"), Some(&(6, 6)), "{by_layer:?}");
    let (back, _) = open(&saved.export.bytes).unwrap();
    let mut tags: Vec<String> = back
        .top_level_nodes()
        .into_iter()
        .map(|n| back.node_tags(n).concat().join("/"))
        .collect();
    tags.sort();
    // (Loose geometry on one layer reads back as one Object.)
    assert_eq!(tags, ["LittleLayer", "MediumLayer"]);
    // The hidden box reopens hidden, the others shown.
    for n in back.top_level_nodes() {
        let medium = back.node_tags(n).concat().join("/") == "MediumLayer";
        assert_eq!(back.node_user_hidden(n), medium);
    }
}

#[test]
fn a_model_guide_on_a_definitions_guide_line_is_kept() {
    // A definition's guide line and a model-level guide on the same world
    // line: the document shows two coincident guides, and each save keeps
    // both in the file (one for the definition, one for the model).
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let mut model = skp_export::author(&doc).model;
    let (run, world) = {
        let tree = model.scene();
        let n = tree
            .iter()
            .find(|n| n.run.is_some_and(|r| model.geometry[r].def_index.is_some()))
            .expect("a top-level placement");
        (n.run.unwrap(), n.world)
    };
    let di = model.geometry[run].def_index;
    let (local_p, local_d) = ([0.01, 0.02, 0.03], [1.0, 0.0, 0.0]);
    model.guides.push(skpwright::Guide {
        def_index: di,
        point_m: local_p,
        direction: local_d,
        pid: 900_002,
        bounds: [-1e30, 1e30],
        layer: 0,
        material: None,
        hidden: false,
        casts_shadows: true,
        receives_shadows: true,
        stipple: 0xFFFF,
    });
    let t = kernel::Transform::from_affine(&world[..12].try_into().unwrap());
    let wp = t.apply_point(kernel::Point3::new(local_p[0], local_p[1], local_p[2]));
    let wd = t
        .apply_vector(kernel::Vec3::new(local_d[0], local_d[1], local_d[2]))
        .normalized()
        .unwrap();
    model.guides.push(skpwright::Guide {
        def_index: None,
        point_m: [wp.x, wp.y, wp.z],
        direction: [wd.x, wd.y, wd.z],
        pid: 900_003,
        bounds: [-1e30, 1e30],
        layer: 0,
        material: None,
        hidden: false,
        casts_shadows: true,
        receives_shadows: true,
        stipple: 0xFFFF,
    });
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let counts = |bytes: &[u8]| {
        let m = Model::parse(bytes).unwrap();
        (
            m.guides.iter().filter(|g| g.def_index.is_some()).count(),
            m.guides.iter().filter(|g| g.def_index.is_none()).count(),
        )
    };
    let before = counts(&bytes);
    let (opened, mut src) = open(&bytes).unwrap();
    for generation in 1..=3 {
        let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
        assert_eq!(counts(&saved.export.bytes), before, "save {generation}");
        src = saved.source;
    }
}

#[test]
fn a_hidden_guide_is_kept_by_a_save_back_and_a_hidden_layers_opens_tagged() {
    // A model-level guide hidden with Edit > Hide: SketchUp does not draw
    // it, Hew does not open it (a Hew guide has no hidden state), and a
    // save back keeps it in the file as it is. One on a hidden layer opens
    // tagged with the layer, hidden through the tag, and a save back keeps
    // the file's record of it on that layer.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let mut model = skp_export::author(&doc).model;
    let write = |model: &Model| {
        skpwright::Document::from_model(model)
            .unwrap()
            .write()
            .unwrap()
    };
    let count = |bytes: &[u8]| Model::parse(bytes).unwrap().guides.len();
    let plain = write(&model);
    let (shown, lines) = (open(&plain).unwrap().0.guide_ids().len(), count(&plain));
    let guide = |pid: u32, layer: u16, hidden: bool| skpwright::Guide {
        def_index: None,
        point_m: [0.5, 0.25, 0.125],
        direction: [0.0, 0.0, 1.0],
        pid,
        bounds: [-1e30, 1e30],
        layer,
        material: None,
        hidden,
        casts_shadows: true,
        receives_shadows: true,
        stipple: 0xFFFF,
    };
    model.layers.push(skpwright::Layer {
        name: "Rig".to_string(),
        visible: false,
        rgba: [0, 0, 0, 255],
    });
    let slot = model.layer_links.iter().map(|(s, _)| *s).max().unwrap_or(0) + 1;
    model.layer_links.push((slot, model.layers.len() - 1));
    model.guides.push(guide(900_004, 0, true));
    model.guides.push(guide(900_005, slot, false));
    let bytes = write(&model);
    assert_eq!(count(&bytes), lines + 2);
    let (opened, mut src) = open(&bytes).unwrap();
    let ids = opened.guide_ids();
    assert_eq!(
        ids.len(),
        shown + 1,
        "the hidden-layer guide opens, the hidden one does not"
    );
    let on_rig = ids
        .iter()
        .copied()
        .find(|&g| opened.guide_tags(g) == [vec!["Rig".to_string()]])
        .expect("tagged with its layer");
    assert!(!opened.guide_shown(on_rig), "hidden through the tag");
    for generation in 1..=2 {
        let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
        let m = Model::parse(&saved.export.bytes).unwrap();
        assert_eq!(m.guides.len(), lines + 2, "save {generation}");
        assert!(
            m.guides
                .iter()
                .any(|g| g.pid == 900_005 && m.layer_of(g.layer).is_some_and(|l| l.name == "Rig")),
            "save {generation}: the file's guide stays on its layer"
        );
        assert!(
            m.guides.iter().any(|g| g.pid == 900_004 && g.hidden),
            "save {generation}"
        );
        src = saved.source;
    }
}

#[test]
fn a_save_back_keeps_the_files_own_units() {
    // The display units are Hew's preference; a SketchUp file's units are
    // its own setting, and a save back over it keeps them.
    for name in ["house.skp", "2026/house.skp"] {
        let bytes = fixture(name);
        let (doc, src) = open(&bytes).unwrap();
        let before = Model::parse(&bytes).unwrap().units;
        assert_ne!(before.length_unit, 2, "{name}: the fixture is not in mm");
        let saved = skp_export::save_in(
            &doc,
            container(&bytes),
            env(),
            Some(&src),
            skp_export::SaveUnits::New(skp_export::DisplayUnits::Millimeters),
        )
        .unwrap();
        let after = Model::parse(&saved.export.bytes).unwrap().units;
        assert_eq!(
            (
                after.length_format,
                after.length_unit,
                after.length_precision
            ),
            (
                before.length_format,
                before.length_unit,
                before.length_precision
            ),
            "{name}"
        );
        // Without a source to build on, the display units; asked for by
        // name, those units over the file's own too.
        for (source, units) in [
            (
                None,
                skp_export::SaveUnits::New(skp_export::DisplayUnits::Millimeters),
            ),
            (
                Some(&src),
                skp_export::SaveUnits::Set(skp_export::DisplayUnits::Millimeters),
            ),
        ] {
            let out = skp_export::save_in(&doc, container(&bytes), env(), source, units).unwrap();
            let u = Model::parse(&out.export.bytes).unwrap().units;
            assert_eq!(u.length_unit, 2, "{name} {units:?}");
            if source.is_some() {
                // Only the length settings change: the file's angle
                // settings and display switches stay its own.
                let rest = |u: &skpwright::Units| {
                    (
                        u.angle_precision,
                        u.angle_snap,
                        u.snap_angle,
                        u.suppress_units_display,
                        u.force_inch_display,
                    )
                };
                assert_eq!(rest(&u), rest(&before), "{name}");
            }
        }
        // Keeping, a file written from the document alone takes a new
        // SketchUp model's units.
        let kept = skp_export::save_in(
            &doc,
            container(&bytes),
            env(),
            None,
            skp_export::SaveUnits::Keep,
        )
        .unwrap();
        let (u, d) = (
            Model::parse(&kept.export.bytes).unwrap().units,
            skpwright::Units::sketchup_default(),
        );
        assert_eq!(
            (
                u.length_format,
                u.length_unit,
                u.length_precision,
                u.length_snap_length
            ),
            (
                d.length_format,
                d.length_unit,
                d.length_precision,
                d.length_snap_length
            ),
            "{name}"
        );
    }
}

#[test]
fn loose_objects_grouped_in_hew_are_saved_in_their_group() {
    for name in ["house.skp", "2026/house.skp"] {
        let bytes = fixture(name);
        let (mut doc, src) = open(&bytes).unwrap();
        let loose: Vec<NodeId> = doc
            .top_level_nodes()
            .into_iter()
            .filter(|&n| matches!(n, NodeId::Object(o) if doc.object_name(o) == Some("Model")))
            .collect();
        assert!(!loose.is_empty(), "{name}: the house has loose geometry");
        doc.group_nodes(&loose).unwrap();
        let saved = save(&doc, container(&bytes), env(), Some(&src)).unwrap();
        let (back, _) = open(&saved.export.bytes).unwrap();
        let (a, b) = (
            format!("{:?}", digest(&doc)),
            format!("{:?}", digest(&back)),
        );
        assert!(
            a == b || common::near(&a, &b),
            "{name}: the group survives:\n{}",
            where_differ(&a, &b)
        );
    }
}

#[test]
fn a_scene_hiding_a_member_of_an_unchanged_group_keeps_it_hidden() {
    use kernel::SceneProps;
    // A group made in Hew and saved: the file has it, and Hew has its
    // provenance.
    let bytes = fixture("house.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let walls: Vec<NodeId> = ["Front Wall", "Back Wall"]
        .iter()
        .map(|n| NodeId::Object(object_named(&doc, n)))
        .collect();
    doc.group_nodes(&walls).unwrap();
    let first = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let (mut doc, src) = open(&first.export.bytes).unwrap();
    let (g, m) = doc
        .top_level_nodes()
        .into_iter()
        .find_map(|n| match n {
            NodeId::Group(g) => doc.group_members(g)?.first().map(|&m| (g, m)),
            _ => None,
        })
        .expect("the group reopens");
    let _ = g;
    // Hide a member, capture a scene, show it again: the group is as the
    // file has it, and only the scene hides the member.
    doc.set_node_user_hidden(m, true);
    doc.add_scene(
        Some("Member hidden".into()),
        SceneProps::ALL,
        None,
        None,
        None,
    )
    .unwrap();
    doc.set_node_user_hidden(m, false);
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let (back, _) = open(&saved.export.bytes).unwrap();
    let hidden = |d: &Document| {
        d.scenes()
            .iter()
            .find(|s| s.name == "Member hidden")
            .and_then(|s| s.hidden_nodes.as_ref())
            .map_or(0, |h| h.len())
    };
    assert_eq!(hidden(&doc), 1);
    assert_eq!(hidden(&back), 1, "the member stays hidden in the scene");
}

/// The neighborhood where two digests first part.
fn where_differ(a: &str, b: &str) -> String {
    let at = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let win = |s: &str| {
        s.chars()
            .skip(at.saturating_sub(300))
            .take(600)
            .collect::<String>()
    };
    format!("at {at}:\n  {}\n  {}", win(a), win(b))
}

#[test]
fn sketchup_dimensions_stay_the_files_until_changed() {
    // feature-pack.skp: a dimension and two texts of SketchUp's own; Hew
    // reads the dimension and one text.
    let bytes = fixture("feature-pack.skp");
    let source = Model::parse(&bytes).unwrap();
    let pids = |m: &Model| -> (Vec<u32>, Vec<u32>) {
        (
            m.dimensions.iter().map(|d| d.pid).collect(),
            m.texts.iter().map(|t| t.pid).collect(),
        )
    };
    let hew_marked = |m: &Model| {
        m.dictionaries
            .iter()
            .filter(|a| matches!(a.owner, Owner::Dimension(_) | Owner::Text(_)))
            .count()
    };
    let (mut doc, src) = open(&bytes).unwrap();
    assert_eq!(doc.annotation_ids().len(), 2);
    assert_eq!(src.annotations.len(), 2);

    // Untouched: the file's own records stay, none written again, and a
    // reopen reads the same two (no copies beside them).
    let saved = save(&doc, container(&bytes), env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(pids(&m), pids(&source));
    assert_eq!(hew_marked(&m), 0);
    assert_eq!(saved.source.annotations.len(), 2);
    let (again, _) = open(&saved.export.bytes).unwrap();
    assert_eq!(again.annotation_ids().len(), 2);

    // Deleted in Hew: its record goes; the rest stay.
    let dim = doc
        .annotation_ids()
        .into_iter()
        .find(|&a| {
            matches!(
                doc.annotation(a),
                Some(kernel::Annotation::LinearDimension { .. })
            )
        })
        .unwrap();
    doc.delete_annotation(dim).unwrap();
    let saved = save(&doc, container(&bytes), env(), Some(&saved.source)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert!(m.dimensions.is_empty());
    assert_eq!(pids(&m).1, pids(&source).1);
    assert_eq!(saved.source.annotations.len(), 1);
}

#[test]
fn a_placed_image_stays_the_files_moved_with_its_instance() {
    // 2026/feature-pack.skp: one image among the model's own entities.
    let bytes = fixture("2026/feature-pack.skp");
    let source = Model::parse(&bytes).unwrap();
    // Each placed image with its definition's name in place of its index
    // (a writer numbers definitions its own way).
    let images = |m: &Model| -> Vec<(String, skpwright::ImagePlacement)> {
        m.geometry
            .iter()
            .flat_map(|r| r.images.iter().cloned())
            .map(|im| {
                let name = m
                    .definitions
                    .iter()
                    .find(|d| d.map_index == Some(im.defref as usize))
                    .map(|d| d.name.clone())
                    .unwrap_or_default();
                (name, skpwright::ImagePlacement { defref: 0, ..im })
            })
            .collect()
    };
    assert_eq!(images(&source).len(), 1);
    let (mut doc, src) = open(&bytes).unwrap();
    let image = doc
        .instance_ids()
        .into_iter()
        .find(|&i| {
            doc.sid_of(&kernel::EntityRef::Instance(i))
                .and_then(|s| src.nodes.get(&s))
                .is_some_and(|n| matches!(n.origin, skp_import::Origin::Image { .. }))
        })
        .expect("the image is an instance");
    let placed = |m: &Model| m.geometry.iter().map(|r| r.placed.len()).sum::<usize>();

    // Untouched: the file's image as it was, nothing written beside it.
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(images(&m), images(&source));
    assert_eq!(placed(&m), placed(&source));
    let (again, _) = open(&saved.export.bytes).unwrap();
    assert_eq!(again.instance_ids().len(), doc.instance_ids().len());

    // Moved: the file's image moves with it.
    doc.transform_instance(
        image,
        &kernel::Transform::translation(kernel::Vec3::new(1.0, 0.0, 0.0)),
    )
    .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&saved.source)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let (was, now) = (&images(&source)[0].1, &images(&m)[0].1);
    assert_eq!(now.pid, was.pid);
    assert!(
        (now.transform[9] - was.transform[9] - skpwright::INCH).abs() < 1e-6,
        "{now:?}"
    );
    assert_eq!(placed(&m), placed(&source));

    // Deleted: the image goes.
    doc.delete_node(NodeId::Instance(image)).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&saved.source)).unwrap();
    assert!(images(&Model::parse(&saved.export.bytes).unwrap()).is_empty());
}

#[test]
fn a_placement_hew_never_read_in_stays_on_save_back() {
    // box-group.skp with its box moved out among the model's own entities
    // and the group left holding bare edges, which the document never
    // reads in. A save back keeps the group, placed where it was, with
    // its persistent id.
    let mut m = Model::parse(&fixture("box-group.skp")).unwrap();
    let ri = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_some() && !r.mesh.faces.is_empty())
        .expect("the group's geometry");
    let root = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .unwrap();
    let mut loose = m.geometry[ri].mesh.clone();
    loose.vertex_pids.iter_mut().for_each(|p| *p = 0);
    loose.edges.iter_mut().for_each(|e| e.pid = 0);
    loose.faces.iter_mut().for_each(|f| f.pid = 0);
    m.geometry[root].mesh = loose;
    m.geometry[ri].mesh.faces.clear();
    let bytes = skpwright::StreamDocument::from_model(&m, skpwright::Release::R2017)
        .unwrap()
        .write();
    let source = Model::parse(&bytes).unwrap();
    let placed = |m: &Model| -> Vec<(u32, [f64; 13])> {
        let mut v: Vec<_> = m
            .geometry
            .iter()
            .flat_map(|r| r.placed.iter().map(|p| (p.pid, p.transform)))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    assert_eq!(placed(&source).len(), 1);
    let (doc, src) = open(&bytes).unwrap();
    assert_eq!(doc.live_object_ids().len(), 1, "the box is read in");
    assert!(doc.instance_ids().is_empty(), "the group is not");
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(placed(&m), placed(&source));
}

#[test]
fn a_second_save_after_editing_a_component_neither_doubles_nor_revives_its_members() {
    // nested-3-deep.skp: the outer component holds two middle placements,
    // each holding two of the leaf. A first save keeps the outer component
    // as the file has it; a later edit inside it rewrites its list, which
    // must hold the document's members only: none twice, none deleted back.
    let bytes = fixture("nested-3-deep.skp");
    // Placements as the model shows them, through every placement of each
    // definition (an edited definition's old record stays, unplaced).
    fn count(nodes: &[skpwright::Node]) -> usize {
        nodes.iter().map(|n| 1 + count(&n.children)).sum()
    }
    let placed = |m: &Model| count(&m.scene());
    let before = placed(&Model::parse(&bytes).unwrap());
    let (mut doc, src) = open(&bytes).unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    assert_eq!(placed(&Model::parse(&saved.export.bytes).unwrap()), before);

    // Inside the one world placement of the outer component: move one
    // middle placement, save; delete the other, save.
    let world = doc.instance_ids()[0];
    let middles = |doc: &Document| -> Vec<kernel::InstanceId> {
        doc.session_direct_members()
            .unwrap()
            .into_iter()
            .filter_map(|n| match n {
                NodeId::Instance(i) => Some(i),
                _ => None,
            })
            .collect()
    };
    doc.open_explode_session(world).unwrap();
    let inside = middles(&doc);
    assert_eq!(inside.len(), 2);
    doc.transform_instance(
        inside[0],
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 0.1)),
    )
    .unwrap();
    doc.close_explode_session().unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&saved.source),
    )
    .unwrap();
    let edited = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(placed(&edited), before, "moved");
    // The outer definition is rewritten in its place; the middle and the
    // leaf, untouched, stay the file's own: no definition written again
    // under another name, none left behind unplaced.
    let names = |m: &Model| -> Vec<String> {
        let mut v: Vec<String> = m
            .definitions
            .iter()
            .filter(|d| d.kind == skpwright::DefinitionKind::Component)
            .map(|d| d.name.clone())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(&edited), names(&Model::parse(&bytes).unwrap()));

    let world = doc.instance_ids()[0];
    doc.open_explode_session(world).unwrap();
    let inside = middles(&doc);
    doc.delete_node(NodeId::Instance(inside[1])).unwrap();
    doc.close_explode_session().unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&saved.source),
    )
    .unwrap();
    // One middle placement fewer, and the two leaves inside it.
    assert_eq!(
        placed(&Model::parse(&saved.export.bytes).unwrap()),
        before - 3,
        "deleted"
    );
}

#[test]
fn a_sketchup_dimension_survives_its_geometry_being_rewritten() {
    // box.skp's loose box with a dimension of SketchUp's own between two
    // of its vertices. Moving the box rewrites its geometry, whose vertices
    // then carry new ids: the file's record would name vertices the file no
    // longer holds, so the document's own dimension takes its place.
    let mut m = Model::parse(&fixture("box.skp")).unwrap();
    let root = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .unwrap();
    let mesh = &m.geometry[root].mesh;
    let (v0, v1) = (mesh.vertex_pids[0], mesh.vertex_pids[1]);
    // Measured along the line between them (frame 1 measures along x).
    let (p, q) = (mesh.vertices[0], mesh.vertices[1]);
    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let x_axis = d.map(|c| c / len);
    let normal = if x_axis[2].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let at = |entity| skpwright::Anchor {
        kind: 2,
        entity: Some(entity),
        ..Default::default()
    };
    m.dimensions = vec![skpwright::Dimension {
        start: at(v0),
        end: at(v1),
        normal,
        x_axis,
        offset_m: 0.2,
        variant: 1,
        ..Default::default()
    }];
    let bytes = skpwright::StreamDocument::from_model(&m, skpwright::Release::R2017)
        .unwrap()
        .write();
    let (mut doc, src) = open(&bytes).unwrap();
    assert_eq!(doc.annotation_ids().len(), 1, "the dimension opens");

    // Unchanged: the file's record stays.
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(m.dimensions.len(), 1);
    assert_eq!(m.dimensions[0].start.kind, 2);

    // The box moved: the dimension is still there, now the document's.
    let o = doc.visible_object_ids()[0];
    doc.transform_object(
        o,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 0.5)),
    )
    .unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&saved.source),
    )
    .unwrap();
    assert!(
        saved
            .export
            .writer_notes
            .iter()
            .all(|n| !n.contains("left out")),
        "{:?}",
        saved.export.writer_notes
    );
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(m.dimensions.len(), 1);
    assert_eq!(m.dimensions[0].start.kind, 1);
}

#[test]
fn a_sketchup_dimension_on_a_copy_made_unique_stays_on_that_copy() {
    // dim-second-instance.skp: a dimension SketchUp drew on the second of
    // two placements of one box component. Making that placement unique
    // gives it a definition of its own, whose vertices carry new ids: the
    // file's record would name vertices only the first placement still
    // holds, so the document's own dimension takes its place, where it was.
    let bytes = fixture("dim-second-instance.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let second = doc
        .instance_ids()
        .into_iter()
        .find(|&i| {
            doc.sid_of(&kernel::EntityRef::Instance(i))
                .and_then(|s| src.nodes.get(&s))
                .is_some_and(|n| n.origin == skp_import::Origin::Instance { pid: 4809 })
        })
        .expect("the second placement");
    doc.make_unique(second).unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    assert!(
        saved
            .export
            .writer_notes
            .iter()
            .all(|n| !n.contains("left out")),
        "{:?}",
        saved.export.writer_notes
    );
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(m.dimensions.len(), 1);
    let d = &m.dimensions[0];
    assert_eq!((d.start.kind, d.end.kind), (1, 1));
    let x = |a: &skpwright::Anchor| a.point_m[0] / 0.0254;
    assert!((x(&d.start) - 48.9375).abs() < 1e-6, "{}", x(&d.start));
    assert!((x(&d.end) - 87.6875).abs() < 1e-6, "{}", x(&d.end));
}

#[test]
fn a_sketchup_dimension_where_two_edges_meet_needs_both_edges_kept() {
    // box-group.skp with a copy of its box among the model's own entities,
    // and a dimension from a vertex of the group's box to where an edge of
    // that box meets an edge of the copy. Moving the copy rewrites its edges: the
    // group's edge is still written but the copy's is not, so the file's
    // record would lose its second edge, and the document's own dimension
    // takes its place.
    let mut m = Model::parse(&fixture("box-group.skp")).unwrap();
    let gi = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_some() && !r.mesh.faces.is_empty())
        .unwrap();
    let root = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .unwrap();
    let group = m.geometry[root].placed[0].pid;
    let mut loose = m.geometry[gi].mesh.clone();
    loose.vertex_pids.iter_mut().for_each(|p| *p = 0);
    loose.edges.iter_mut().for_each(|e| e.pid = 0);
    loose.faces.iter_mut().for_each(|f| f.pid = 0);
    m.geometry[root].mesh = loose;
    let bytes = skpwright::StreamDocument::from_model(&m, skpwright::Release::R2017)
        .unwrap()
        .write();
    // Ids as written: the copy's edges have theirs now.
    let mut m = Model::parse(&bytes).unwrap();
    let gi = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_some() && !r.mesh.faces.is_empty())
        .unwrap();
    let root = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .unwrap();
    let g = &m.geometry[gi].mesh;
    let first = &g.edges[0];
    let corner = g.vertices[first.v0 as usize];
    let r = &m.geometry[root].mesh;
    let other = r
        .edges
        .iter()
        .find(|e| {
            let (p, q) = (r.vertices[e.v0 as usize], r.vertices[e.v1 as usize]);
            let along = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
            (along(p, corner) || along(q, corner))
                && !(along(p, g.vertices[first.v1 as usize])
                    || along(q, g.vertices[first.v1 as usize]))
        })
        .expect("a copy's edge meeting the group's edge at its corner");
    let anchor = |kind, entity, path: Vec<u32>| skpwright::Anchor {
        kind,
        entity: Some(entity),
        path,
        ..Default::default()
    };
    // Measured along the group's edge (frame 1 measures along x).
    let q = g.vertices[first.v1 as usize];
    let d = [q[0] - corner[0], q[1] - corner[1], q[2] - corner[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let x_axis = d.map(|c| c / len);
    let normal = if x_axis[2].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    // From a vertex of the group's box, which stays as it is.
    let start = anchor(2, g.vertex_pids[first.v1 as usize], vec![group]);
    m.dimensions = vec![skpwright::Dimension {
        start,
        end: skpwright::Anchor {
            second: Some(other.pid),
            ..anchor(9, first.pid, vec![group])
        },
        normal,
        x_axis,
        variant: 1,
        ..Default::default()
    }];
    let bytes = skpwright::StreamDocument::from_model(&m, skpwright::Release::R2017)
        .unwrap()
        .write();
    let (mut doc, src) = open(&bytes).unwrap();
    assert_eq!(doc.annotation_ids().len(), 1, "the dimension opens");
    let copy = doc
        .top_level_nodes()
        .into_iter()
        .find_map(|n| match n {
            NodeId::Object(o) => Some(o),
            _ => None,
        })
        .expect("the copy");
    doc.transform_object(
        copy,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 0.5)),
    )
    .unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    assert!(
        saved
            .export
            .writer_notes
            .iter()
            .all(|n| !n.contains("left out")),
        "{:?}",
        saved.export.writer_notes
    );
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(m.dimensions.len(), 1);
    assert_eq!(m.dimensions[0].end.kind, 1);
}

#[test]
fn an_image_inside_a_group_stays_the_files_when_the_group_is_rewritten() {
    // image-in-group.skp: feature-pack's picture inside a group. Moving
    // the picture rewrites the group's list; the picture is written back
    // as the file's image (its id and definition), at the new place.
    for name in ["2026/image-in-group.skp", "image-in-group.skp"] {
        let bytes = fixture(name);
        let target = container(&bytes);
        let source = Model::parse(&bytes).unwrap();
        let images = |m: &Model| -> Vec<skpwright::ImagePlacement> {
            m.geometry
                .iter()
                .flat_map(|r| r.images.iter().cloned())
                .collect()
        };
        let was = images(&source);
        assert_eq!(was.len(), 1, "{name}");
        let (mut doc, src) = open(&bytes).unwrap();
        let image = doc
            .instance_ids()
            .into_iter()
            .find(|&i| {
                doc.sid_of(&kernel::EntityRef::Instance(i))
                    .and_then(|s| src.nodes.get(&s))
                    .is_some_and(|n| matches!(n.origin, skp_import::Origin::Image { .. }))
            })
            .unwrap_or_else(|| panic!("{name}: the image is an instance"));
        assert!(
            doc.node_parent(NodeId::Instance(image)).is_some(),
            "{name}: inside the group"
        );
        doc.transform_instance(
            image,
            &kernel::Transform::translation(kernel::Vec3::new(0.0, 2.0, 0.0)),
        )
        .unwrap();
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let m = Model::parse(&saved.export.bytes).unwrap();
        let now = images(&m);
        assert_eq!(
            now.len(),
            1,
            "{name}: still one image, no component in its place"
        );
        assert_eq!(now[0].pid, was[0].pid, "{name}");
        // Its definition is the file's image definition (a 2026 writer
        // numbers definitions anew, so by name and kind).
        let def_of = |m: &Model, defref: u32| {
            m.definitions
                .iter()
                .find(|d| d.map_index == Some(defref as usize))
                .map(|d| (d.kind, d.name.clone()))
        };
        assert_eq!(
            def_of(&m, now[0].defref),
            def_of(&source, was[0].defref),
            "{name}"
        );
        assert_eq!(
            def_of(&m, now[0].defref).map(|d| d.0),
            Some(skpwright::DefinitionKind::Image)
        );
        assert!(
            (now[0].transform[10] - was[0].transform[10] - 2.0 * skpwright::INCH).abs() < 1e-6,
            "{name}: moved 2 m along y: {:?}",
            now[0].transform
        );
        // It is still inside the group's list, not the root's.
        let run = m.geometry.iter().find(|r| !r.images.is_empty()).unwrap();
        assert!(
            run.def_index.is_some(),
            "{name}: the image stays in the group"
        );
        // Opened again, the picture is back where it was moved to.
        let (again, _) = open(&saved.export.bytes).unwrap();
        assert_eq!(
            again.instance_ids().len(),
            doc.instance_ids().len(),
            "{name}"
        );
    }
}

#[test]
fn a_group_of_lines_stays_the_files_and_goes_with_its_group() {
    // lines-group.skp: a group holding only edges, with a dimension and a
    // label of its own. An unchanged save keeps the group's list as it
    // is; deleting the group in Hew removes the lines from the file.
    for name in ["2026/lines-group.skp", "lines-group.skp"] {
        let bytes = fixture(name);
        let target = container(&bytes);
        let lines = |m: &Model| -> Vec<usize> {
            m.geometry
                .iter()
                .filter(|r| r.def_index.is_some() && r.mesh.faces.is_empty())
                .map(|r| r.mesh.edges.len())
                .filter(|&n| n > 0)
                .collect()
        };
        let source = Model::parse(&bytes).unwrap();
        assert_eq!(lines(&source), vec![5], "{name}");
        let (mut doc, src) = open(&bytes).unwrap();
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let m = Model::parse(&saved.export.bytes).unwrap();
        assert_eq!(lines(&m), vec![5], "{name}: unchanged, kept");
        assert_eq!(
            (m.dimensions.len(), m.texts.len()),
            (source.dimensions.len(), source.texts.len()),
            "{name}: its dimension and label stay the file's"
        );
        // Opened again: the same group of lines.
        let (again, _) = open(&saved.export.bytes).unwrap();
        let g = again
            .group_ids()
            .into_iter()
            .find(|&g| again.group_name(g) == Some("Lines"))
            .unwrap();
        assert_eq!(again.group_member_sketches(g).len(), 1, "{name}");

        let group = doc
            .group_ids()
            .into_iter()
            .find(|&g| doc.group_name(g) == Some("Lines"))
            .unwrap();
        doc.delete_node(NodeId::Group(group)).unwrap();
        let saved = save(&doc, target, env(), Some(&saved.source)).unwrap();
        let m = Model::parse(&saved.export.bytes).unwrap();
        assert!(
            lines(&m).is_empty(),
            "{name}: the lines went with the group: {:?}",
            lines(&m)
        );
    }
}

/// Drawing added at the root beside the file's loose geometry: the list
/// is marked as drawing, so the loose solids are written as groups and
/// come back as they were.
#[test]
fn drawing_added_at_the_root_keeps_the_loose_solids() {
    let bytes = fixture("layers.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let loose_before = doc.top_level_nodes().len();
    let s = doc.add_sketch(
        kernel::Plane::from_point_normal(kernel::Point3::ORIGIN, kernel::Vec3::new(0.0, 0.0, 1.0))
            .unwrap(),
    );
    doc.begin_sketch_gesture(s).unwrap();
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(
            kernel::Point3::new(20.0, 20.0, 0.0),
            kernel::Point3::new(21.0, 20.0, 0.0),
        )
        .unwrap();
    doc.end_sketch_gesture(s).unwrap();
    for target in [Target::Stream(skpwright::Release::R2017), Target::Skp2026] {
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let (back, _) = open(&saved.export.bytes).unwrap();
        assert_eq!(back.top_level_nodes().len(), loose_before, "{target:?}");
        assert_eq!(digest(&doc), digest(&back), "{target:?}");
    }
}

/// A drafting sketch's tag is its edges' layer in the file, and comes back
/// as the sketch's tag: edges of one plane on two layers open as two
/// sketches, each tagged with its layer, and an untagged sketch's edges
/// stay on the default layer.
#[test]
fn a_sketchs_tag_is_its_edges_layer_and_comes_back() {
    let ground =
        kernel::Plane::from_point_normal(kernel::Point3::ORIGIN, kernel::Vec3::new(0.0, 0.0, 1.0))
            .unwrap();
    let line = |x: f64, tags: Vec<Vec<String>>| kernel::ImportSketch {
        plane: ground,
        def: None,
        group: None,
        segments: vec![[
            kernel::Point3::new(x, 0.0, 0.0),
            kernel::Point3::new(x, 1.0, 0.0),
        ]],
        curves: Vec::new(),
        tags,
    };
    let site = vec!["Site".to_string()];
    let digging = vec!["Digging plan".to_string()];
    let scene = kernel::ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: Vec::new(),
        tags: vec![
            kernel::ImportTag {
                path: site.clone(),
                hidden: true,
            },
            kernel::ImportTag {
                path: digging.clone(),
                hidden: false,
            },
        ],
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        geo: None,
        extra: kernel::ImportSceneExtra {
            sketches: vec![
                line(0.0, Vec::new()),
                line(1.0, vec![site.clone()]),
                line(2.0, vec![digging.clone()]),
            ],
            ..Default::default()
        },
    };
    let mut doc = Document::new();
    doc.ingest(scene, Vec::new()).unwrap();
    for target in [Target::Stream(skpwright::Release::R2017), Target::Skp2026] {
        let saved = save(&doc, target, env(), None).unwrap();
        let model = Model::parse(&saved.export.bytes).unwrap();
        let mut layers: Vec<String> = model
            .geometry
            .iter()
            .filter(|r| r.def_index.is_none())
            .flat_map(|r| r.mesh.edges.iter())
            .map(|e| {
                model
                    .layer_of(e.layer)
                    .map(|l| l.name.clone())
                    .unwrap_or_default()
            })
            .collect();
        layers.sort();
        assert_eq!(layers, ["Digging plan", "Layer0", "Site"], "{target:?}");

        let (back, _) = open(&saved.export.bytes).unwrap();
        let mut tags: Vec<Vec<Vec<String>>> = back
            .sketch_ids()
            .into_iter()
            .map(|s| back.sketch_tags(s).to_vec())
            .collect();
        tags.sort();
        assert_eq!(
            tags,
            vec![Vec::new(), vec![digging.clone()], vec![site.clone()]],
            "{target:?}"
        );
        assert!(back.tag_hidden(&site), "{target:?}");
        assert_eq!(digest(&doc).sketches, digest(&back).sketches, "{target:?}");

        // Deleting the tag in Hew and saving back takes the layer off the
        // edges (the sketch reads as changed), so it does not come back.
        let (mut edited, src) = open(&saved.export.bytes).unwrap();
        edited.delete_tag(&site).unwrap();
        let resaved = save(&edited, target, env(), Some(&src)).unwrap();
        let (again, _) = open(&resaved.export.bytes).unwrap();
        assert!(
            again
                .sketch_ids()
                .into_iter()
                .all(|s| again.sketch_tags(s) != [site.clone()]),
            "{target:?}"
        );
        assert!(
            !again.tag_meta().any(|(p, _)| p == site.as_slice()),
            "{target:?}"
        );
    }
}

/// A root-level image put into a group in Hew is written into that
/// group's list, not left where the file had it.
#[test]
fn a_root_image_grouped_in_hew_is_written_into_its_group() {
    let bytes = fixture("feature-pack.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let image = doc
        .instance_ids()
        .into_iter()
        .find(|&i| doc.instance_def(i).and_then(|d| doc.component_name(d)) == Some("Image"))
        .expect("the file's image");
    doc.group_nodes(&[NodeId::Instance(image)]).unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let model = Model::parse(&saved.export.bytes).unwrap();
    let root = model
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .unwrap();
    assert!(
        model.geometry[root].images.is_empty(),
        "the image left the root list"
    );
    assert_eq!(
        model.geometry.iter().map(|r| r.images.len()).sum::<usize>(),
        1,
        "and is written once, in the group"
    );
    let (back, _) = open(&saved.export.bytes).unwrap();
    assert_eq!(digest(&doc), digest(&back));
}

/// A tag renamed keeps its layer (and what the file hung on it) even
/// when a new tag has taken its old name.
#[test]
fn a_renamed_tag_keeps_its_layer_when_a_new_tag_takes_its_old_name() {
    let bytes = fixture("layers.skp");
    let (mut doc, src) = open(&bytes).unwrap();
    let mut paths: Vec<Vec<String>> = doc.tag_meta().map(|(p, _)| p.to_vec()).collect();
    paths.sort();
    let old = paths[0].clone();
    let renamed = vec![format!("{} renamed", old[0])];
    doc.rename_tag(&old, renamed.clone()).unwrap();
    // A new box carries a new tag spelled like the old one.
    let s = doc.add_sketch(
        kernel::Plane::from_point_normal(kernel::Point3::ORIGIN, kernel::Vec3::new(0.0, 0.0, 1.0))
            .unwrap(),
    );
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let p = |x: f64, y: f64| kernel::Point3::new(x, y, 0.0);
        for (a, b) in [
            ((20.0, 20.0), (21.0, 20.0)),
            ((21.0, 20.0), (21.0, 21.0)),
            ((21.0, 21.0), (20.0, 21.0)),
            ((20.0, 21.0), (20.0, 20.0)),
        ] {
            sk.add_segment(p(a.0, a.1), p(b.0, b.1)).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    let (new_box, _) = doc.extrude_region(s, region, 0.5).unwrap();
    doc.set_tag_hidden(old.clone(), false);
    doc.add_node_tag(NodeId::Object(new_box), old.clone())
        .unwrap();
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let before = file_digest(&Model::parse(&bytes).unwrap());
    let after = file_digest(&Model::parse(&saved.export.bytes).unwrap());
    assert_eq!(
        after.layers.len(),
        before.layers.len() + 1,
        "{:?}",
        after.layers
    );
    // The old layer, renamed, keeps its position; the new tag's layer is
    // appended after it.
    let old_at = before
        .layers
        .iter()
        .position(|(n, _)| *n == old[0])
        .unwrap();
    assert_eq!(after.layers[old_at].0, renamed[0], "{:?}", after.layers);
    assert_eq!(after.layers.last().unwrap().0, old[0], "{:?}", after.layers);
    let (back, _) = open(&saved.export.bytes).unwrap();
    assert_eq!(digest(&doc), digest(&back));
}

/// Deleting a group whose definition held the file's dimension and text
/// records: the definition is pruned with its records, and the document's
/// annotations (anchored to points, so they outlive the group) are written
/// fresh at the root rather than kept in a list that is gone.
#[test]
fn annotations_kept_in_a_deleted_groups_definition_are_written_again() {
    for name in ["lines-group.skp", "2026/lines-group.skp"] {
        let bytes = fixture(name);
        let (mut doc, src) = open(&bytes).unwrap();
        let before = doc.annotation_ids().len();
        assert_eq!(before, 2, "{name}: the dimension and the label");
        let group = doc
            .group_ids()
            .into_iter()
            .find(|&g| doc.group_name(g) == Some("Lines"))
            .unwrap();
        doc.delete_node(NodeId::Group(group)).unwrap();
        assert_eq!(
            doc.annotation_ids().len(),
            before,
            "{name}: point anchors stay"
        );
        let target = container(&bytes);
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        let model = Model::parse(&saved.export.bytes).unwrap();
        assert_eq!(
            model.dimensions.len() + model.texts.len(),
            2,
            "{name}: both records are in the file"
        );
        assert!(
            model.dimensions.iter().all(|d| d.def_index.is_none())
                && model.texts.iter().all(|t| t.def_index.is_none()),
            "{name}: at the root, not in the pruned definition"
        );
        let (back, _) = open(&saved.export.bytes).unwrap();
        assert_eq!(back.annotation_ids().len(), 2, "{name}");
        assert_eq!(digest(&doc), digest(&back), "{name}");
    }
}

#[test]
fn a_loop_with_no_face_writes_its_edges_alone() {
    // A closed loop of drawing that has no face (a wire frame opened from
    // SketchUp, say) goes back as edges alone; one given its face by
    // drawing over an edge goes back as a face.
    use kernel::{Plane, Point3, Vec3};
    let mut doc = Document::new();
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(ground);
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
        sk.make_regions_faceless();
    }
    let faces = |doc: &Document| {
        let model = skp_export::author(doc).model;
        let bytes = skpwright::Document::from_model(&model)
            .unwrap()
            .write()
            .unwrap();
        Model::parse(&bytes)
            .unwrap()
            .geometry
            .iter()
            .map(|r| r.topology.faces)
            .sum::<usize>()
    };
    assert_eq!(faces(&doc), 0, "no face for a loop with none");
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    assert_eq!(faces(&doc), 1, "drawn over, the loop has a face to write");
}

#[test]
fn a_guides_first_tag_is_its_layer_in_a_write() {
    use kernel::{ImportGuide, ImportScene, ImportTag, Point3, Vec3};
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: vec![ImportGuide::Line {
            dash: kernel::GuideDash::default(),
            origin: Point3::ORIGIN,
            direction: Vec3::new(1.0, 0.0, 0.0),
            tags: vec![vec!["Rig".to_string()]],
            span: kernel::GuideSpan::INFINITE,
        }],
        tags: vec![ImportTag {
            path: vec!["Rig".to_string()],
            hidden: true,
        }],
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    doc.ingest(scene, Vec::new()).unwrap();
    let model = skp_export::author(&doc).model;
    assert_eq!(model.guides.len(), 1);
    let layer = model.layer_of(model.guides[0].layer).expect("a layer");
    assert_eq!(layer.name, "Rig");
    assert!(!layer.visible, "the hidden tag is a hidden layer");
}

#[test]
fn a_tag_deleted_in_hew_moves_the_files_guide_off_its_layer() {
    // A file guide on layer "Rig" opens tagged "Rig"; deleting the tag in
    // Hew untags the guide, and the save back writes the file's record of
    // it on the default layer, so SketchUp shows it as Hew does.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let mut model = skp_export::author(&doc).model;
    model.layers.push(skpwright::Layer {
        name: "Rig".to_string(),
        visible: true,
        rgba: [0, 0, 0, 255],
    });
    let slot = model.layer_links.iter().map(|(s, _)| *s).max().unwrap_or(0) + 1;
    model.layer_links.push((slot, model.layers.len() - 1));
    model.guides.push(skpwright::Guide {
        def_index: None,
        point_m: [0.5, 0.25, 0.125],
        direction: [0.0, 0.0, 1.0],
        pid: 900_006,
        bounds: [-1e30, 1e30],
        layer: slot,
        material: None,
        hidden: false,
        casts_shadows: true,
        receives_shadows: true,
        stipple: 0xFFFF,
    });
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let (mut opened, src) = open(&bytes).unwrap();
    let rig = opened
        .guide_ids()
        .into_iter()
        .find(|&g| opened.guide_tags(g) == [vec!["Rig".to_string()]])
        .expect("opened tagged");
    opened.delete_tag(&[String::from("Rig")]).unwrap();
    assert!(opened.guide_tags(rig).is_empty());
    let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let g = m
        .guides
        .iter()
        .find(|g| g.pid == 900_006)
        .expect("the file's guide is kept");
    assert_eq!(g.layer, 0, "and moved to the default layer");
}

#[test]
fn a_guide_segment_and_a_ray_open_with_their_stretch_and_save_back_as_they_were() {
    // SketchUp's Tape Measure makes guides that start at a point (a ray)
    // or run between two (a segment), and shows only that stretch. Two
    // collinear half-inch segments in a definition (a furniture model's
    // tick marks) and a model-level ray: each opens with its own stretch,
    // a save back keeps each once with its bounds, and a fresh write
    // writes the bounds Hew holds.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let mut model = skp_export::author(&doc).model;
    let run = {
        let tree = model.scene();
        tree.iter()
            .find(|n| n.run.is_some_and(|r| model.geometry[r].def_index.is_some()))
            .expect("a top-level placement")
            .run
            .unwrap()
    };
    let di = model.geometry[run].def_index;
    let guide = |def_index, point_m, direction, bounds, pid| skpwright::Guide {
        def_index,
        point_m,
        direction,
        pid,
        bounds,
        layer: 0,
        material: None,
        hidden: false,
        casts_shadows: true,
        receives_shadows: true,
        stipple: 0xFFFF,
    };
    model.guides.push(guide(
        di,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.5],
        900_010,
    ));
    model.guides.push(guide(
        di,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.5],
        900_011,
    ));
    model.guides.push(guide(
        None,
        [0.1, 0.2, 0.3],
        [0.0, 0.0, 1.0],
        [0.0, 1e30],
        900_012,
    ));
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let (opened, mut src) = open(&bytes).unwrap();
    let spans: Vec<kernel::GuideSpan> = opened
        .guide_ids()
        .iter()
        .filter_map(|&g| match *opened.guide(g)? {
            kernel::Guide::Line { span, .. } if !span.is_infinite() => Some(span),
            _ => None,
        })
        .collect();
    assert_eq!(spans.len(), 3, "{spans:?}");
    assert!(
        spans.iter().any(|s| s.from == Some(0.0) && s.to.is_none()),
        "the ray starts at its point and runs on: {spans:?}"
    );
    let segments: Vec<f64> = spans.iter().filter_map(|s| Some(s.to? - s.from?)).collect();
    assert_eq!(segments.len(), 2);
    let bounds_of = |bytes: &[u8]| {
        let mut b: Vec<[i64; 2]> = Model::parse(bytes)
            .unwrap()
            .guides
            .iter()
            .filter(|g| g.pid >= 900_010 || g.bounds != [-1e30, 1e30])
            .map(|g| g.bounds.map(|x| (x * 1e6).round() as i64))
            .collect();
        b.sort_unstable();
        b
    };
    let before = bounds_of(&bytes);
    assert_eq!(before.len(), 3);
    for generation in 1..=3 {
        let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
        assert_eq!(
            bounds_of(&saved.export.bytes),
            before,
            "save {generation} keeps each guide once, with its bounds"
        );
        src = saved.source;
    }
    // A fresh write (no source) writes the ray Hew holds with its bounds.
    let fresh = save(&opened, Target::Skp2026, env(), None).unwrap();
    let rays: Vec<[f64; 2]> = Model::parse(&fresh.export.bytes)
        .unwrap()
        .guides
        .iter()
        .filter(|g| g.def_index.is_none() && g.bounds[1] >= 1e29 && g.bounds[0].abs() < 1e-9)
        .map(|g| g.bounds)
        .collect();
    assert_eq!(rays.len(), 1, "the ray is written as a ray");
    let (reopened, _) = open(&fresh.export.bytes).unwrap();
    let respans = reopened
        .guide_ids()
        .iter()
        .filter(|&&g| {
            matches!(reopened.guide(g), Some(kernel::Guide::Line { span, .. }) if !span.is_infinite())
        })
        .count();
    assert_eq!(respans, 3, "every stretch comes back from Hew's own write");
}

// ── Every release saves back in its own ───────────────────────────────────

/// A file of any release `skpwright` lists opens and saves back in that
/// release (the header's own version, `target_of`), twice over, keeping
/// the Object count: written by `skpwright`'s own writer for each release,
/// then opened as the app opens a file.
#[test]
fn every_release_saves_back_in_its_own() {
    let native = Document::load(
        &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native.hew"))
            .unwrap(),
    )
    .unwrap();
    for release in skpwright::Release::ALL {
        let n = release.number();
        let first = skp_export::export(&native, Target::Stream(release), env())
            .unwrap_or_else(|e| panic!("release {n}: {e}"));
        let mut bytes = first.bytes;
        for round in 1..=2 {
            let target = skp_export::target_of(&bytes);
            assert_eq!(target, Target::Stream(release), "release {n} round {round}");
            let (doc, src) = open(&bytes).unwrap_or_else(|| panic!("release {n}: opens"));
            let objects = doc.live_object_ids().len();
            let saved = save(&doc, target, env(), Some(&src))
                .unwrap_or_else(|e| panic!("release {n} round {round}: {e}"));
            let (version, _) = skpwright::header_info(&saved.export.bytes).unwrap();
            assert_eq!(
                skpwright::Release::of_version(&version),
                Some(release),
                "release {n} round {round}: {version}"
            );
            assert!(
                saved
                    .export
                    .writer_notes
                    .iter()
                    .all(|w| !w.contains("is not one Skpwright writes")),
                "{:?}",
                saved.export.writer_notes
            );
            let (again, _) =
                open(&saved.export.bytes).unwrap_or_else(|| panic!("release {n}: reopens"));
            assert_eq!(
                again.live_object_ids().len(),
                objects,
                "release {n} round {round}"
            );
            bytes = saved.export.bytes;
        }
    }
}

/// A real file of release 2013 saves back as release 2013, not 2017.
#[test]
fn a_release_2013_file_saves_back_as_2013() {
    let bytes = fixture("box-v2013.skp");
    let target = skp_export::target_of(&bytes);
    assert_eq!(target, Target::Stream(skpwright::Release::R2013));
    let (doc, src) = open(&bytes).unwrap();
    let saved = save(&doc, target, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(m.release(), Some(skpwright::Release::R2013));
}

/// A stream file of a release `skpwright` does not list is read and saved
/// back as the nearest release it writes, and the writer's notes say so.
#[test]
fn an_unlisted_release_file_saves_as_the_nearest_with_a_note() {
    let mut bytes = fixture("box.skp");
    assert_eq!(bytes[36..44], [0x7b, 0, 0x31, 0, 0x37, 0, 0x2e, 0]);
    bytes[38] = b'2';
    bytes[40] = b'3';
    let target = skp_export::target_of(&bytes);
    assert_eq!(target, Target::Stream(skpwright::Release::R2020));
    let (doc, src) = open(&bytes).expect("an unlisted release is read");
    let saved = save(&doc, target, env(), Some(&src)).unwrap();
    assert!(
        saved
            .export
            .writer_notes
            .iter()
            .any(|n| n.contains("{23.") && n.contains("release 20")),
        "{:?}",
        saved.export.writer_notes
    );
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(m.release(), Some(skpwright::Release::R2020));
}

#[test]
fn a_guides_dash_opens_and_is_written_by_a_save_back_and_a_fresh_write() {
    // A file guide stippled dash-dot-dash opens with that dash, a save back
    // keeps it, and a fresh write gives each document guide its own dash
    // (short dashes for one Hew draws, as Hew draws it).
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    let mut model = skp_export::author(&doc).model;
    model.guides.push(skpwright::Guide {
        def_index: None,
        point_m: [0.5, 0.25, 0.125],
        direction: [0.0, 0.0, 1.0],
        pid: 900_010,
        bounds: [-1e30, 1e30],
        layer: 0,
        material: None,
        hidden: false,
        casts_shadows: true,
        receives_shadows: true,
        stipple: 0x1C47,
    });
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let (mut opened, src) = open(&bytes).unwrap();
    let dashes = |d: &Document| -> Vec<u16> {
        d.guide_ids()
            .into_iter()
            .filter(|&g| matches!(d.guide(g), Some(kernel::Guide::Line { .. })))
            .map(|g| d.guide_dash(g).mask())
            .collect()
    };
    assert!(dashes(&opened).contains(&0x1C47), "the file's dash opens");
    let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let kept = m.guides.iter().find(|g| g.pid == 900_010).expect("kept");
    assert_eq!(kept.stipple, 0x1C47, "a save back keeps the file's dash");

    // A guide Hew draws writes short dashes; a fresh write carries each.
    opened
        .add_guide_line(kernel::Point3::ORIGIN, kernel::Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    let fresh = skp_export::author(&opened).model;
    let mut written: Vec<u16> = fresh.guides.iter().map(|g| g.stipple).collect();
    written.sort_unstable();
    let mut held = dashes(&opened);
    held.sort_unstable();
    assert_eq!(written, held, "a fresh write gives each guide its dash");
    assert!(written.contains(&0x3333) && written.contains(&0x1C47));
    // And the save back writes the one Hew drew with its dash.
    let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert!(m.guides.iter().any(|g| g.stipple == 0x3333));
}

#[test]
fn a_match_photo_camera_keeps_its_image_width_ratio_and_2d_state_through_a_save_back() {
    // A camera set to an image's width and ratio, in two-point perspective
    // and zoomed: it opens with the width (in meters) and ratio, a save
    // back writes them and the 2D state unchanged, and a camera the user
    // moved is written as a plain camera (zero width and ratio, no 2D).
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let mut doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    doc.set_camera_state(kernel::CameraState::framed(
        kernel::CameraProjection::Perspective,
        35.0,
        kernel::Point3::new(4.0, -6.0, 3.0),
        kernel::Point3::new(0.0, 0.0, 0.0),
        kernel::Vec3::new(0.0, 0.0, 1.0),
    ));
    let mut model = skp_export::author(&doc).model;
    let cam = model.camera.as_mut().expect("a camera is written");
    cam.perspective = true;
    cam.fov_deg = 35.0;
    cam.image_width = 1000.0;
    cam.aspect_ratio = 1.5;
    cam.two_point_perspective = true;
    cam.scale_2d = 2.0;
    cam.offset_2d = [0.25, -0.125];
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let (mut opened, src) = open(&bytes).unwrap();
    let held = opened.camera_state().expect("a camera");
    assert!((held.image_width.unwrap() - 25.4).abs() < 1e-9);
    assert_eq!(held.aspect_ratio, Some(1.5));

    let back = |d: &Document| {
        let saved = save(d, Target::Skp2026, env(), Some(&src)).unwrap();
        Model::parse(&saved.export.bytes).unwrap().camera.unwrap()
    };
    // The app records its (unmoved) view at the save.
    opened.set_camera_state(kernel::CameraState {
        image_width: None,
        aspect_ratio: None,
        ..held
    });
    let c = back(&opened);
    assert!((c.image_width - 1000.0).abs() < 1e-6, "{}", c.image_width);
    assert_eq!(c.aspect_ratio, 1.5);
    assert!(c.two_point_perspective);
    assert_eq!((c.scale_2d, c.offset_2d), (2.0, [0.25, -0.125]));

    // Orbited: a plain camera.
    opened.set_camera_state(kernel::CameraState {
        eye: held.eye + kernel::Vec3::new(0.5, 0.0, 0.0),
        image_width: None,
        aspect_ratio: None,
        ..held
    });
    let c = back(&opened);
    assert_eq!((c.image_width, c.aspect_ratio), (0.0, 0.0));
    assert!(!c.two_point_perspective);
    assert_eq!((c.scale_2d, c.offset_2d), (1.0, [0.0, 0.0]));

    // A fresh write of a document holding a width writes it in inches.
    opened.set_camera_state(held);
    let fresh = skp_export::author(&opened).model.camera.unwrap();
    assert!((fresh.image_width - 1000.0).abs() < 1e-6);
    assert_eq!(fresh.aspect_ratio, 1.5);
}

#[test]
fn a_files_section_and_guide_display_settings_open_and_a_save_back_keeps_every_one() {
    // A file's style: a red cut outline 5 px wide, a filled cut, a green
    // active plane, blue guides, hidden geometry and objects, and a section
    // plane with a name and a symbol. Hew holds the cut, fill and guide
    // colors; the hidden settings, the plane's name and the display bits
    // have no Hew counterpart and stay the file's.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zoo.hew");
    let mut doc = Document::load(&std::fs::read(p).unwrap()).unwrap();
    doc.set_view_style(Some(kernel::ViewStyle {
        background: [10, 20, 30],
        sky: None,
        ground: None,
        axes: true,
        section: Default::default(),
        guides_color: None,
        guides_hidden: None,
    }));
    doc.set_section_plane(kernel::SectionPlaneState {
        origin: kernel::Point3::new(0.0, 0.0, 0.5),
        normal: kernel::Vec3::new(0.0, 0.0, 1.0),
        active: true,
    })
    .unwrap();
    let mut model = skp_export::author(&doc).model;
    let r = model.rendering.as_mut().expect("a style is written");
    r.section_cut_color = [200, 0, 0, 255];
    r.section_cut_width = 5;
    r.section_fill = Some(true);
    r.section_fill_color = Some([0, 0, 200, 128]);
    r.active_section_color = [0, 200, 0, 255];
    r.guides_color = [0, 0, 255, 255];
    r.guides_hidden = true;
    r.hidden_geometry = true;
    r.hidden_objects = true;
    r.section_display = 2;
    let sp = model
        .geometry
        .iter_mut()
        .find_map(|g| g.sections.first_mut())
        .expect("the cut is written as a plane");
    sp.name = "Cut A".to_string();
    sp.symbol = "A".to_string();
    let bytes = skpwright::Document::from_model(&model)
        .unwrap()
        .write()
        .unwrap();
    let (opened, src) = open(&bytes).unwrap();
    let style = opened.view_style().expect("the file's style opens");
    assert_eq!(style.section.cut_color, Some([200, 0, 0]));
    assert_eq!(style.section.cut_width, Some(5));
    assert_eq!(style.section.fill, Some(true));
    assert_eq!(style.section.fill_color, Some([0, 0, 200]));
    assert_eq!(style.section.active_color, Some([0, 200, 0]));
    assert_eq!(style.guides_color, Some([0, 0, 255]));
    assert_eq!(style.guides_hidden, Some(true));

    // A save back keeps all of it, the plane's name and symbol included.
    let saved = save(&opened, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let r = m.rendering.expect("the style is kept");
    assert_eq!(r.section_cut_color, [200, 0, 0, 255]);
    assert_eq!(r.section_cut_width, 5);
    assert_eq!(r.section_fill, Some(true));
    assert_eq!(
        r.section_fill_color,
        Some([0, 0, 200, 128]),
        "alpha is kept"
    );
    assert_eq!(r.active_section_color, [0, 200, 0, 255]);
    assert_eq!(r.guides_color, [0, 0, 255, 255]);
    assert!(r.guides_hidden);
    assert!(r.hidden_geometry && r.hidden_objects, "no Hew counterpart");
    assert_eq!(r.section_display, 2);
    let planes: Vec<_> = m.geometry.iter().flat_map(|g| g.sections.iter()).collect();
    assert_eq!(planes.len(), 1);
    assert_eq!(
        (planes[0].name.as_str(), planes[0].symbol.as_str()),
        ("Cut A", "A")
    );

    // A fresh write of the document writes the colors Hew holds.
    let fresh = skp_export::author(&opened).model.rendering.unwrap();
    assert_eq!(fresh.section_cut_color, [200, 0, 0, 255]);
    assert_eq!(fresh.section_cut_width, 5);
    assert_eq!(fresh.guides_color, [0, 0, 255, 255]);
    assert!(fresh.guides_hidden);
}

/// `m` with the dictionaries `dicts` put on `owners`, written and re-read.
fn with_dictionaries(m: &mut Model, owners: Vec<(Owner, &str)>) -> Vec<u8> {
    for (owner, name) in owners {
        m.dictionaries.push(skpwright::Attributes {
            owner,
            dictionaries: vec![skpwright::Dictionary {
                name: name.into(),
                entries: vec![("k".into(), skpwright::Value::Int(1))],
                flag: 0,
            }],
        });
    }
    let file = std::env::temp_dir().join(format!("hew-resave-{}.skp", std::process::id()));
    m.write(&file).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    bytes
}

fn dictionary_names(m: &Model, pick: impl Fn(&Owner) -> bool) -> Vec<(Owner, String)> {
    m.dictionaries
        .iter()
        .filter(|a| pick(&a.owner))
        .flat_map(|a| {
            a.dictionaries
                .iter()
                .filter(|d| d.name != "hew")
                .map(|d| (a.owner, d.name.clone()))
        })
        .collect()
}

#[test]
fn removing_a_root_image_drops_its_dictionaries_and_renumbers_the_later_ones() {
    let mut m = Model::parse(&fixture("2026/feature-pack.skp")).unwrap();
    let root = m
        .geometry
        .iter()
        .position(|r| !r.images.is_empty())
        .unwrap();
    // A second placement of the same image after the file's own.
    let mut second = m.geometry[root].images[0].clone();
    second.pid = m
        .geometry
        .iter()
        .flat_map(|r| {
            r.images
                .iter()
                .map(|i| i.pid)
                .chain(r.placed.iter().map(|p| p.pid))
        })
        .max()
        .unwrap()
        + 1000;
    m.geometry[root].images.push(second);
    let bytes = with_dictionaries(
        &mut m,
        vec![
            (Owner::Image(root, 0), "first"),
            (Owner::Image(root, 1), "second"),
        ],
    );
    let read = Model::parse(&bytes).unwrap();
    assert_eq!(read.geometry[root].images.len(), 2);
    let first_pid = read.geometry[root].images[0].pid;
    let second_pid = read.geometry[root].images[1].pid;

    let (mut doc, src) = open(&bytes).unwrap();
    let image = doc
        .instance_ids()
        .into_iter()
        .find(|&i| {
            doc.sid_of(&kernel::EntityRef::Instance(i))
                .and_then(|s| src.nodes.get(&s))
                .is_some_and(
                    |n| matches!(n.origin, skp_import::Origin::Image { pid } if pid == first_pid),
                )
        })
        .expect("the first image is an instance");
    doc.delete_node(NodeId::Instance(image)).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let run = after
        .geometry
        .iter()
        .position(|r| !r.images.is_empty())
        .unwrap();
    assert_eq!(after.geometry[run].images.len(), 1);
    assert_eq!(after.geometry[run].images[0].pid, second_pid);
    let names = dictionary_names(&after, |o| matches!(o, Owner::Image(..)));
    assert_eq!(names, vec![(Owner::Image(run, 0), "second".to_string())]);
}

#[test]
fn removing_a_run_renumbers_the_later_runs_vertex_dictionaries() {
    // The group "Lines" is a definition run of its own; deleting it takes
    // the run away and moves every later run's keys down.
    let mut m = Model::parse(&fixture("2026/lines-group.skp")).unwrap();
    let runs: Vec<usize> = (0..m.geometry.len())
        .filter(|&i| {
            m.geometry[i].def_index.is_some() && !m.geometry[i].mesh.vertex_pids.is_empty()
        })
        .collect();
    // Each run's vertex dictionary is named by the run's vertex count.
    let count = |m: &Model, i: usize| m.geometry[i].mesh.vertex_pids.len();
    let named: Vec<(Owner, String)> = runs
        .iter()
        .map(|&i| (Owner::Vertex(i, 0), format!("v{}", count(&m, i))))
        .collect();
    let owners = named.iter().map(|(o, n)| (*o, n.as_str())).collect();
    let bytes = with_dictionaries(&mut m, owners);
    let before = Model::parse(&bytes).unwrap();
    let (mut doc, src) = open(&bytes).unwrap();
    let group = doc
        .group_ids()
        .into_iter()
        .find(|&g| doc.group_name(g) == Some("Lines"))
        .unwrap();
    doc.delete_node(NodeId::Group(group)).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    assert!(after.geometry.len() < before.geometry.len(), "a run went");
    let names = dictionary_names(&after, |o| matches!(o, Owner::Vertex(..)));
    assert!(!names.is_empty());
    assert!(
        names.len() < runs.len(),
        "the removed run's dictionary went: {names:?}"
    );
    for (owner, name) in &names {
        let Owner::Vertex(k, 0) = owner else {
            panic!("{owner:?}")
        };
        assert_eq!(name, &format!("v{}", count(&after, *k)), "{names:?}");
    }
}
