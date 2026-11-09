//! The post-2017 features of a 2026-container file survive an edit in Hew:
//! layer folders and page extras are keyed by layer and scene index, so a
//! deleted tag or scene must not leave them pointing at the wrong record.

use std::path::Path;

use kernel::Document;
use skp_export::{SkpSource, Target, WriteEnv, save};
use skpwright::Model;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../skp-import/tests/fixtures/2027")
            .join(name),
    )
    .unwrap()
}

fn open(bytes: &[u8]) -> (Document, SkpSource) {
    let out = skp_import::import(bytes).unwrap();
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    doc.forget_history().unwrap();
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    (doc, src)
}

fn env() -> WriteEnv {
    WriteEnv {
        unix_time: 1_800_000_000,
        seed: 5,
    }
}

fn folder_layers(m: &Model) -> Vec<(String, Vec<String>)> {
    m.features
        .layer_folders
        .iter()
        .map(|f| {
            (
                f.name.clone(),
                f.layers.iter().map(|&i| m.layers[i].name.clone()).collect(),
            )
        })
        .collect()
}

#[test]
fn an_unchanged_save_keeps_the_layer_folders() {
    let bytes = fixture("f-layer-folder.skp");
    let before = folder_layers(&Model::parse(&bytes).unwrap());
    let (doc, src) = open(&bytes);
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(folder_layers(&after), before);
}

#[test]
fn a_deleted_tag_leaves_the_folders_on_the_right_layers() {
    let bytes = fixture("f-layer-folder.skp");
    let before = folder_layers(&Model::parse(&bytes).unwrap());
    let (mut doc, src) = open(&bytes);
    // (The layer InA is the tag Folder A / InA since layer folders open
    // as tag paths.)
    doc.delete_tag(&[String::from("Folder A"), String::from("InA")])
        .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let want: Vec<_> = before
        .into_iter()
        .map(|(n, ls)| (n, ls.into_iter().filter(|l| l != "InA").collect::<Vec<_>>()))
        .collect();
    assert_eq!(folder_layers(&after), want);
}

#[test]
fn a_deleted_scene_leaves_the_page_extras_on_the_right_scenes() {
    let bytes = fixture("g-page-folders.skp");
    let m0 = Model::parse(&bytes).unwrap();
    let names = |m: &Model, s: &[(usize, skpwright::PageExtras)]| {
        s.iter()
            .map(|(i, e)| (m.scenes[*i].name.clone(), e.hidden_layer_folders.clone()))
            .collect::<Vec<_>>()
    };
    let before = names(&m0, &m0.features.scene_pages);
    assert!(!before.is_empty());
    let (mut doc, src) = open(&bytes);
    let first = doc.scenes().first().map(|s| s.sid).expect("a scene");
    doc.remove_scene(first).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m1 = Model::parse(&saved.export.bytes).unwrap();
    let gone = m0.scenes[0].name.clone();
    let want: Vec<_> = before.into_iter().filter(|(n, _)| *n != gone).collect();
    assert_eq!(names(&m1, &m1.features.scene_pages), want);
}

fn path(segs: &[&str]) -> Vec<String> {
    segs.iter().map(|s| s.to_string()).collect()
}

fn tags(doc: &Document) -> Vec<(Vec<String>, bool)> {
    doc.tag_meta().map(|(p, h)| (p.to_vec(), h)).collect()
}

fn folder_tree(m: &Model) -> Vec<(String, bool, Vec<String>, usize)> {
    fn walk(
        m: &Model,
        list: &[skpwright::LayerFolder],
        out: &mut Vec<(String, bool, Vec<String>, usize)>,
    ) {
        for f in list {
            out.push((
                f.name.clone(),
                f.hidden,
                f.layers.iter().map(|&i| m.layers[i].name.clone()).collect(),
                f.folders.len(),
            ));
            walk(m, &f.folders, out);
        }
    }
    let mut out = Vec::new();
    walk(m, &m.features.layer_folders, &mut out);
    out
}

#[test]
fn layer_folders_open_as_nested_tag_paths() {
    let (doc, _) = open(&fixture("f-layer-folder.skp"));
    let t = tags(&doc);
    // A layer in a folder takes the folders' names before its own; a
    // layer in none stays top level; a hidden folder is a hidden tag
    // (hiding its subtree), a visible one needs no tag of its own.
    assert!(t.contains(&(path(&["Folder A", "InA"]), false)), "{t:?}");
    assert!(t.contains(&(path(&["Folder A", "Sub B", "InB"]), false)));
    assert!(t.contains(&(path(&["Folder A", "Sub B"]), true)));
    assert!(t.contains(&(path(&["Loose"]), false)));
    assert!(!t.iter().any(|(p, _)| p == &path(&["InA"])));
}

#[test]
fn an_empty_hidden_folder_opens_as_a_hidden_tag() {
    let (doc, _) = open(&fixture("k-folder-hidden.skp"));
    assert_eq!(tags(&doc), vec![(path(&["F"]), true)]);
}

#[test]
fn an_unchanged_save_keeps_empty_and_hidden_folders() {
    for name in [
        "k-folder-hidden.skp",
        "f-layer-folder.skp",
        "g-page-folders.skp",
    ] {
        let bytes = fixture(name);
        let before = Model::parse(&bytes).unwrap();
        let (doc, src) = open(&bytes);
        let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
        let after = Model::parse(&saved.export.bytes).unwrap();
        assert_eq!(folder_tree(&after), folder_tree(&before), "{name}");
        assert_eq!(
            after.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
            before.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
            "{name}"
        );
        assert_eq!(
            after.features.layer_folders, before.features.layer_folders,
            "{name}"
        );
    }
}

#[test]
fn a_tag_moved_under_a_new_parent_writes_a_new_folder() {
    let bytes = fixture("f-layer-folder.skp");
    let (mut doc, src) = open(&bytes);
    doc.rename_tag(&path(&["Loose"]), path(&["Fresh", "Loose"]))
        .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let tree = folder_tree(&m);
    assert!(
        tree.contains(&("Fresh".into(), false, vec!["Loose".into()], 0)),
        "{tree:?}"
    );
    // The file's own folders and their layers did not move, and the layer
    // kept its name.
    assert!(tree.contains(&("Folder A".into(), false, vec!["InA".into()], 1)));
    // It opens again as the same paths.
    let (again, _) = open(&saved.export.bytes);
    assert!(tags(&again).contains(&(path(&["Fresh", "Loose"]), false)));
    // The provenance maps the path to the file's layer.
    assert!(saved.source.layers.values().any(|n| n == "Loose"));
}

#[test]
fn a_hidden_folder_survives_as_hidden_in_a_save() {
    let bytes = fixture("f-layer-folder.skp");
    let (mut doc, src) = open(&bytes);
    doc.set_tag_hidden(path(&["Folder A"]), true);
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let tree = folder_tree(&m);
    assert!(tree.iter().any(|(n, h, _, _)| n == "Folder A" && *h));
    // The folder is a folder, not also a layer.
    assert!(!m.layers.iter().any(|l| l.name == "Folder A"));
}

#[test]
fn a_new_document_writes_its_tag_paths_as_folders() {
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["Structure", "Roof"]), false);
    doc.set_tag_hidden(path(&["Structure", "Walls"]), true);
    doc.set_tag_hidden(path(&["Site", "Walls"]), false);
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let tree = folder_tree(&m);
    // Layers are named by the last segment; the same one under two
    // folders is made unique.
    let layers_of = |f: &str| tree.iter().find(|t| t.0 == f).unwrap().2.clone();
    let (structure, site) = (layers_of("Structure"), layers_of("Site"));
    assert_eq!((structure.len(), site.len()), (2, 1), "{tree:?}");
    assert!(structure.contains(&"Roof".to_string()));
    let walls: Vec<_> = structure
        .iter()
        .chain(&site)
        .filter(|n| n.starts_with("Walls"))
        .collect();
    assert_eq!(walls.len(), 2, "{tree:?}");
    assert_ne!(walls[0], walls[1]);
    let (again, _) = open(&saved.export.bytes);
    let t = tags(&again);
    assert!(t.contains(&(path(&["Structure", "Walls"]), true)));
    assert!(t.contains(&(path(&["Site", "Walls"]), false)));
}

#[test]
fn a_2017_save_keeps_flat_layers() {
    let bytes = fixture("f-layer-folder.skp");
    let (doc, src) = open(&bytes);
    let saved = save(
        &doc,
        Target::Stream(skpwright::Release::R2017),
        env(),
        Some(&src),
    )
    .unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert!(m.features.is_empty());
    let names: Vec<_> = m.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains(&"Folder A/InA"), "{names:?}");
    assert!(names.contains(&"Folder A/Sub B/InB"), "{names:?}");
    let (again, _) = open(&saved.export.bytes);
    assert!(
        tags(&again)
            .iter()
            .any(|(p, _)| p == &path(&["Folder A", "InA"]))
    );
}

#[test]
fn a_page_hiding_a_folder_opens_as_a_hidden_tag_subtree() {
    let (doc, _) = open(&fixture("g-page-folders.skp"));
    let s2 = doc.scenes().iter().find(|s| s.name == "S2").unwrap();
    let resolved = doc.resolve_scene(s2.sid).unwrap();
    let hidden = resolved.hidden_tag_paths.unwrap();
    assert!(hidden.contains(&path(&["Folder A"])), "{hidden:?}");
    let s1 = doc.scenes().iter().find(|s| s.name == "S1").unwrap();
    assert!(
        doc.resolve_scene(s1.sid)
            .unwrap()
            .hidden_tag_paths
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_page_hiding_a_folder_keeps_it_through_a_save() {
    let bytes = fixture("g-page-folders.skp");
    let (doc, src) = open(&bytes);
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let pages: Vec<_> = m
        .features
        .scene_pages
        .iter()
        .map(|(i, e)| (m.scenes[*i].name.clone(), e.hidden_layer_folders.clone()))
        .collect();
    assert_eq!(pages, vec![("S2".to_string(), vec![0])]);
    let s2 = m.scenes.iter().find(|s| s.name == "S2").unwrap();
    assert_eq!(s2.hidden_layers, vec![1]);
}

#[test]
fn a_page_hiding_a_new_folder_writes_it() {
    let bytes = fixture("f-layer-folder.skp");
    let (mut doc, src) = open(&bytes);
    let sid = doc
        .add_scene(
            Some("Pg".into()),
            kernel::SceneProps {
                hidden_tags: true,
                ..kernel::SceneProps::NONE
            },
            None,
            None,
            None,
        )
        .unwrap();
    let _ = sid;
    doc.set_tag_hidden(path(&["Folder A"]), true);
    let s = doc.scenes().iter().find(|s| s.name == "Pg").unwrap().sid;
    doc.update_scene(
        s,
        kernel::SceneProps {
            hidden_tags: true,
            ..kernel::SceneProps::NONE
        },
        None,
        None,
    )
    .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let i = m.scenes.iter().position(|s| s.name == "Pg").unwrap();
    let page = m
        .features
        .scene_pages
        .iter()
        .find(|(k, _)| *k == i)
        .map(|(_, e)| e)
        .expect("a page entry");
    // Folder A is folder 0, Sub B (under it, so hidden with it) is 1.
    assert_eq!(page.hidden_layer_folders, vec![0, 1]);
    let names: Vec<_> = m.scenes[i]
        .hidden_layers
        .iter()
        .map(|&l| m.layers[l].name.as_str())
        .collect();
    assert!(
        names.contains(&"InA") && names.contains(&"InB"),
        "{names:?}"
    );
}

#[test]
fn scene_timing_opens_and_saves_back() {
    let bytes = fixture("k-page-times.skp");
    let (mut doc, src) = open(&bytes);
    let sid = doc.scenes()[0].sid;
    assert_eq!(doc.scenes()[0].transition_s, Some(1.25));
    assert_eq!(doc.scenes()[0].delay_s, Some(3.5));
    // Unchanged save keeps them.
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let e = &m.features.scene_pages[0].1;
    assert_eq!((e.transition_s, e.delay_s), (Some(1.25), Some(3.5)));
    // Changed and cleared.
    doc.set_scene_transition(sid, Some(4.0)).unwrap();
    doc.set_scene_delay(sid, None).unwrap();
    assert!(doc.set_scene_delay(sid, Some(-1.0)).is_err());
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let e = &m.features.scene_pages[0].1;
    assert_eq!((e.transition_s, e.delay_s), (Some(4.0), None));
    // A fresh document writes them too.
    let mut fresh = Document::new();
    let sid = fresh
        .add_scene(Some("A".into()), kernel::SceneProps::NONE, None, None, None)
        .unwrap();
    fresh.set_scene_transition(sid, Some(2.0)).unwrap();
    fresh.set_scene_delay(sid, Some(0.5)).unwrap();
    let saved = save(&fresh, Target::Skp2026, env(), None).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let e = &m.features.scene_pages[0].1;
    assert_eq!((e.transition_s, e.delay_s), (Some(2.0), Some(0.5)));
    let (again, _) = open(&saved.export.bytes);
    assert_eq!(again.scenes()[0].transition_s, Some(2.0));
}

#[test]
fn scene_timing_survives_the_hew_file() {
    let (mut doc, _) = open(&fixture("k-page-times.skp"));
    let sid = doc.scenes()[0].sid;
    doc.set_scene_delay(sid, Some(7.0)).unwrap();
    let bytes = doc.save();
    let loaded = Document::load(&bytes).unwrap();
    assert_eq!(loaded.scenes()[0].transition_s, Some(1.25));
    assert_eq!(loaded.scenes()[0].delay_s, Some(7.0));
}

/// Each hatched placement's pattern, by the placement's pid: the writer
/// names an instance by its place among the runs' placements, so that is
/// the order read here, and the pattern by its name.
fn hatch_by_pid(m: &Model) -> std::collections::BTreeMap<u32, String> {
    let pids: Vec<u32> = m
        .geometry
        .iter()
        .flat_map(|r| r.placed.iter().map(|p| p.pid))
        .collect();
    m.features
        .instance_hatch
        .iter()
        .map(|&(i, h)| (pids[i], m.features.hatch_patterns[h].name.clone()))
        .collect()
}

#[test]
fn a_hatched_group_keeps_its_hatch_through_a_save() {
    for name in ["g-group-hatch.skp", "i-group-hatch2.skp"] {
        let bytes = fixture(name);
        let before = hatch_by_pid(&Model::parse(&bytes).unwrap());
        assert!(!before.is_empty(), "{name}: has a hatched placement");
        let (doc, src) = open(&bytes);
        let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
        let after = hatch_by_pid(&Model::parse(&saved.export.bytes).unwrap());
        assert_eq!(after, before, "{name}: hatch by pid");
    }
}

#[test]
fn a_hatched_group_keeps_its_hatch_when_it_is_renamed() {
    for name in ["g-group-hatch.skp", "i-group-hatch2.skp"] {
        let bytes = fixture(name);
        let before = hatch_by_pid(&Model::parse(&bytes).unwrap());
        let (mut doc, src) = open(&bytes);
        // A rename rewrites the placement rather than putting it back.
        for node in doc.top_level_nodes() {
            doc.set_node_name(node, Some("renamed".into())).unwrap();
        }
        let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
        let after = hatch_by_pid(&Model::parse(&saved.export.bytes).unwrap());
        assert_eq!(after, before, "{name}: hatch by pid");
    }
}

#[test]
fn a_page_that_stops_saving_its_tags_drops_its_hidden_folders() {
    let bytes = fixture("g-page-folders.skp");
    let (mut doc, src) = open(&bytes);
    let s2 = doc.scenes().iter().find(|s| s.name == "S2").unwrap().sid;
    let mut props = doc.scene(s2).unwrap().props();
    assert!(props.hidden_tags, "S2 captures its tags");
    props.hidden_tags = false;
    doc.set_scene_props(s2, props, None, None).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert!(
        m.features
            .scene_pages
            .iter()
            .all(|(_, e)| e.hidden_layer_folders.is_empty())
    );
}

/// Save a document of the tags `[A, x]` and `[B, y]` as a 2026 file, then
/// move layer `x` as SketchUp would: into folder B, or out to the top.
fn moved_layer_file(into: Option<&str>) -> Vec<u8> {
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["A", "x"]), false);
    doc.set_tag_hidden(path(&["B", "y"]), false);
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let mut m = Model::parse(&saved.export.bytes).unwrap();
    let x = m.layers.iter().position(|l| l.name == "x").unwrap();
    for f in &mut m.features.layer_folders {
        f.layers.retain(|&i| i != x);
    }
    if let Some(name) = into {
        m.features
            .layer_folders
            .iter_mut()
            .find(|f| f.name == name)
            .unwrap()
            .layers
            .push(x);
    }
    let file = std::env::temp_dir().join(format!("hew-moved-layer-{}.skp", into.unwrap_or("top")));
    m.write(&file).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    bytes
}

#[test]
fn a_layer_moved_between_folders_opens_where_the_file_has_it() {
    let bytes = moved_layer_file(Some("B"));
    let (doc, _) = open(&bytes);
    let t = tags(&doc);
    assert!(t.contains(&(path(&["B", "x"]), false)), "{t:?}");
    assert!(!t.iter().any(|(p, _)| *p == path(&["A", "x"])), "{t:?}");
}

#[test]
fn a_layer_moved_out_of_its_folder_opens_at_the_top() {
    let bytes = moved_layer_file(None);
    let (doc, _) = open(&bytes);
    let t = tags(&doc);
    assert!(t.contains(&(path(&["x"]), false)), "{t:?}");
    assert!(!t.iter().any(|(p, _)| *p == path(&["A", "x"])), "{t:?}");
}

#[test]
fn a_page_hiding_a_folder_that_is_no_tag_of_its_own_hides_its_tags() {
    // A file Hew wrote with the folders, where the folder is implied by
    // its layers' paths (so it is no tag of its own), then a page of
    // SketchUp's hiding that folder.
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["A", "x"]), false);
    doc.set_tag_hidden(path(&["B", "y"]), false);
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let mut m = Model::parse(&saved.export.bytes).unwrap();
    let mut scene = m.scenes.first().cloned().unwrap_or_default();
    scene.name = "Pg".into();
    scene.saved =
        skpwright::SceneProperties(scene.saved.0 | skpwright::SceneProperties::VISIBLE_LAYERS);
    m.scenes.push(scene);
    let k = m.scenes.len() - 1;
    let a = m
        .features
        .layer_folders
        .iter()
        .position(|f| f.name == "A")
        .unwrap();
    m.features.scene_pages.push((
        k,
        skpwright::PageExtras {
            hidden_layer_folders: vec![a],
            ..Default::default()
        },
    ));
    let file = std::env::temp_dir().join("hew-page-folder.skp");
    m.write(&file).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    let (doc, _) = open(&bytes);
    let pg = doc.scenes().iter().find(|s| s.name == "Pg").unwrap();
    let hidden = doc.resolve_scene(pg.sid).unwrap().hidden_tag_paths.unwrap();
    assert!(hidden.contains(&path(&["A", "x"])), "{hidden:?}");
    assert!(!hidden.contains(&path(&["B", "y"])), "{hidden:?}");
}

fn folder_attrs(m: &Model) -> Vec<(String, Vec<(String, skpwright::Value)>)> {
    let mut pre = Vec::new();
    fn walk(list: &[skpwright::LayerFolder], out: &mut Vec<String>) {
        for f in list {
            out.push(f.name.clone());
            walk(&f.folders, out);
        }
    }
    walk(&m.features.layer_folders, &mut pre);
    m.dictionaries
        .iter()
        .filter_map(|a| match a.owner {
            skpwright::Owner::LayerFolder(i) => Some((
                pre[i].clone(),
                a.dictionaries
                    .iter()
                    .flat_map(|d| d.entries.clone())
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_removed_folder_leaves_the_other_folders_attributes_on_their_folders() {
    let mut doc = Document::new();
    for p in [["A", "x"], ["B", "y"], ["C", "z"]] {
        doc.set_tag_hidden(path(&p), false);
    }
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let mut m = Model::parse(&saved.export.bytes).unwrap();
    let c = m
        .features
        .layer_folders
        .iter()
        .position(|f| f.name == "C")
        .unwrap();
    m.dictionaries.push(skpwright::Attributes {
        owner: skpwright::Owner::LayerFolder(c),
        dictionaries: vec![skpwright::Dictionary {
            name: "d".into(),
            entries: vec![("k".into(), skpwright::Value::Text("v".into()))],
            flag: 0,
        }],
    });
    let file = std::env::temp_dir().join("hew-folder-attrs.skp");
    m.write(&file).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    assert_eq!(folder_attrs(&Model::parse(&bytes).unwrap())[0].0, "C");
    let (mut doc, src) = open(&bytes);
    doc.delete_tag(&path(&["B", "y"])).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let attrs = folder_attrs(&after);
    assert_eq!(attrs.len(), 1, "{attrs:?}");
    assert_eq!(attrs[0].0, "C", "{attrs:?}");
}

#[test]
fn the_attributes_of_a_tag_that_is_only_a_folder_move_to_the_folder() {
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["A", "x"]), false);
    doc.set_tag_hidden(path(&["A"]), false);
    doc.attr_set(
        kernel::AttrTarget::Entity(kernel::EntityRef::Tag(path(&["A"]))),
        "ns",
        "k",
        kernel::AttrValue::Int(7),
    )
    .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let m = Model::parse(&saved.export.bytes).unwrap();
    let attrs = folder_attrs(&m);
    assert_eq!(attrs.len(), 1, "{attrs:?}");
    assert_eq!(attrs[0].0, "A");
    assert!(attrs[0].1.iter().any(|(k, _)| k == "k"), "{attrs:?}");
}

#[test]
fn a_tag_attribute_on_a_released_folder_keeps_the_folders_other_dictionaries() {
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["A", "x"]), false);
    doc.set_tag_hidden(path(&["A"]), false);
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let mut m = Model::parse(&saved.export.bytes).unwrap();
    let a = m
        .features
        .layer_folders
        .iter()
        .position(|f| f.name == "A")
        .unwrap();
    m.dictionaries.push(skpwright::Attributes {
        owner: skpwright::Owner::LayerFolder(a),
        dictionaries: vec![skpwright::Dictionary {
            name: "own".into(),
            entries: vec![("keep".into(), skpwright::Value::Int(3))],
            flag: 0,
        }],
    });
    let file = std::env::temp_dir().join("hew-folder-own-dict.skp");
    m.write(&file).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    let (mut doc, src) = open(&bytes);
    doc.set_tag_hidden(path(&["A"]), false);
    doc.attr_set(
        kernel::AttrTarget::Entity(kernel::EntityRef::Tag(path(&["A"]))),
        "ns",
        "k",
        kernel::AttrValue::Int(7),
    )
    .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let attrs = folder_attrs(&after);
    assert_eq!(attrs.len(), 1, "{attrs:?}");
    let keys: Vec<&str> = attrs[0].1.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        keys.contains(&"keep"),
        "the folder's own dictionary: {keys:?}"
    );
    assert!(keys.contains(&"k"), "the tag's attribute: {keys:?}");
}

#[test]
fn a_layer_kept_for_unread_content_stays_in_its_folder() {
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["A", "x"]), false);
    doc.set_tag_hidden(path(&["A", "w"]), false);
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    let mut m = Model::parse(&saved.export.bytes).unwrap();
    let x = m.layers.iter().position(|l| l.name == "x").unwrap();
    let slot = m.layer_links.iter().find(|l| l.1 == x).unwrap().0;
    // A deactivated section plane on the layer: content Hew does not read.
    m.geometry[0].sections.push(skpwright::SectionPlane {
        pid: 900,
        plane: [0.0, 0.0, 1.0, 0.0],
        layer: slot,
        hidden: true,
        ..Default::default()
    });
    let file = std::env::temp_dir().join("hew-unread-layer.skp");
    m.write(&file).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    let (mut doc, src) = open(&bytes);
    doc.delete_tag(&path(&["A", "x"])).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let tree = folder_tree(&after);
    let a = tree.iter().find(|t| t.0 == "A").unwrap();
    assert!(a.2.contains(&"x".to_string()), "{tree:?}");
    assert!(a.2.contains(&"w".to_string()), "{tree:?}");
}

/// A scene as the file holds it, apart from its name.
fn scene_without_name(m: &Model, i: usize) -> skpwright::Scene {
    let mut s = m.scenes[i].clone();
    s.name = String::new();
    s
}

fn page_of(m: &Model, i: usize) -> Option<skpwright::PageExtras> {
    m.features
        .scene_pages
        .iter()
        .find(|(k, _)| *k == i)
        .map(|(_, e)| e.clone())
}

#[test]
fn a_renamed_scene_keeps_the_page_state_the_file_holds() {
    for name in ["k-page-times.skp", "g-page-folders.skp"] {
        let bytes = fixture(name);
        let (mut doc, src) = open(&bytes);
        let plain = Model::parse(
            &save(&doc, Target::Skp2026, env(), Some(&src))
                .unwrap()
                .export
                .bytes,
        )
        .unwrap();
        let n = plain.scenes.len();
        assert!(n > 0, "{name}");
        for i in 0..n {
            let sid = doc.scenes()[i].sid;
            doc.rename_scene(sid, format!("Renamed {i}")).unwrap();
        }
        let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
        let after = Model::parse(&saved.export.bytes).unwrap();
        assert_eq!(after.scenes.len(), n);
        for i in 0..n {
            assert_eq!(after.scenes[i].name, format!("Renamed {i}"));
            assert_eq!(
                scene_without_name(&after, i),
                scene_without_name(&plain, i),
                "{name}: scene {i}"
            );
            assert_eq!(page_of(&after, i), page_of(&plain, i), "{name}: page {i}");
        }
        // The old behaviour (no record of where a scene came from, matched
        // by name) lost it: this is what the file holds that a fresh scene
        // does not.
        let mut blind = src.clone();
        blind.scenes.clear();
        let lost = Model::parse(
            &save(&doc, Target::Skp2026, env(), Some(&blind))
                .unwrap()
                .export
                .bytes,
        )
        .unwrap();
        let differs = (0..n).any(|i| scene_without_name(&lost, i) != scene_without_name(&plain, i));
        assert!(
            differs,
            "{name}: the control must show what a name match loses"
        );
    }
}

#[test]
fn a_deleted_scene_does_not_donate_its_state_to_a_scene_reusing_its_name() {
    let bytes = fixture("k-page-times.skp");
    let m0 = Model::parse(&bytes).unwrap();
    let (mut doc, src) = open(&bytes);
    let first = doc.scenes()[0].clone();
    assert_eq!(first.transition_s, Some(1.25));
    doc.remove_scene(first.sid).unwrap();
    let fresh = doc
        .add_scene(
            Some(first.name.clone()),
            kernel::SceneProps::NONE,
            None,
            None,
            None,
        )
        .unwrap();
    assert_ne!(fresh, first.sid);
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let m1 = Model::parse(&saved.export.bytes).unwrap();
    let i = m1.scenes.iter().position(|s| s.name == first.name).unwrap();
    // Nothing of the deleted scene's record rides on the new scene.
    assert!(
        m0.scenes[0].rendering.is_some(),
        "sanity: the file scene has a style"
    );
    assert!(m1.scenes[i].rendering.is_none());
    assert!(m1.scenes[i].shadows.is_none());
    let e = page_of(&m1, i).unwrap_or_default();
    assert_eq!((e.transition_s, e.delay_s), (None, None));
}

#[test]
fn scene_identity_survives_a_save_and_the_blob() {
    let bytes = fixture("g-page-folders.skp");
    let (mut doc, src) = open(&bytes);
    assert_eq!(src.scenes.len(), doc.scenes().len());
    let back = SkpSource::from_blob(&src.to_blob()).unwrap();
    assert_eq!(back.scenes, src.scenes);
    // Rename, save, rename again, save: each save still finds the scene.
    let sid = doc.scenes()[0].sid;
    doc.rename_scene(sid, "One".into()).unwrap();
    let first = save(&doc, Target::Skp2026, env(), Some(&back)).unwrap();
    doc.rename_scene(sid, "Two".into()).unwrap();
    let second = save(&doc, Target::Skp2026, env(), Some(&first.source)).unwrap();
    let (plain_doc, plain_src) = open(&bytes);
    let m0 = Model::parse(
        &save(&plain_doc, Target::Skp2026, env(), Some(&plain_src))
            .unwrap()
            .export
            .bytes,
    )
    .unwrap();
    let m2 = Model::parse(&second.export.bytes).unwrap();
    assert_eq!(m2.scenes[0].name, "Two");
    assert_eq!(scene_without_name(&m2, 0), scene_without_name(&m0, 0));
    assert_eq!(page_of(&m2, 0), page_of(&m0, 0));
}

/// A 2026 file from a Hew document of the tags `paths` (each visible),
/// opened as a model for the test to reshape and write back out.
fn authored_model(paths: &[&[&str]]) -> Model {
    let mut doc = Document::new();
    for p in paths {
        doc.set_tag_hidden(path(p), false);
    }
    let saved = save(&doc, Target::Skp2026, env(), None).unwrap();
    Model::parse(&saved.export.bytes).unwrap()
}

/// `m` written by the format writer itself.
fn written(m: &Model) -> Vec<u8> {
    skpwright::Document::from_model(m).unwrap().write().unwrap()
}

fn layer_index(m: &Model, name: &str) -> usize {
    m.layers.iter().position(|l| l.name == name).unwrap()
}

#[test]
fn a_folder_and_a_root_layer_of_one_name_are_one_tag_hidden_when_either_is() {
    for (folder_hidden, layer_visible) in [(true, true), (false, false), (true, false)] {
        let mut m = authored_model(&[&["A", "x"], &["Q"]]);
        let q = layer_index(&m, "Q");
        m.layers[q].name = "A".into();
        m.layers[q].visible = layer_visible;
        m.features.layer_folders[0].hidden = folder_hidden;
        let (doc, _) = open(&written(&m));
        let t = tags(&doc);
        assert!(
            t.contains(&(path(&["A"]), folder_hidden || !layer_visible)),
            "folder hidden {folder_hidden}, layer visible {layer_visible}: {t:?}"
        );
        assert!(t.contains(&(path(&["A", "x"]), false)), "{t:?}");
        assert_eq!(t.iter().filter(|(p, _)| *p == path(&["A"])).count(), 1);
    }
}

#[test]
fn a_folder_and_a_root_layer_of_one_name_are_both_kept_on_save() {
    let mut m = authored_model(&[&["A", "x"], &["Q"]]);
    let q = layer_index(&m, "Q");
    m.layers[q].name = "A".into();
    m.features.layer_folders[0].hidden = true;
    let bytes = written(&m);
    let (doc, src) = open(&bytes);
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    // The folder A (hidden) holds x, and the layer A stands beside it.
    assert_eq!(
        folder_tree(&after),
        vec![("A".to_string(), true, vec!["x".to_string()], 0)]
    );
    assert!(after.layers.iter().any(|l| l.name == "A"));
    assert!(
        after.features.layer_folders[0]
            .layers
            .iter()
            .all(|&i| after.layers[i].name != "A")
    );
    // And a second save changes nothing.
    let (doc2, src2) = open(&saved.export.bytes);
    let again = save(&doc2, Target::Skp2026, env(), Some(&src2)).unwrap();
    assert_eq!(
        folder_tree(&Model::parse(&again.export.bytes).unwrap()),
        folder_tree(&after)
    );
}

#[test]
fn an_empty_folder_beside_a_layer_of_its_name_survives_a_save() {
    let mut m = authored_model(&[&["Q"], &["Z"]]);
    let q = layer_index(&m, "Q");
    m.layers[q].name = "A".into();
    m.features.layer_folders.push(skpwright::LayerFolder {
        name: "A".into(),
        ..Default::default()
    });
    let (doc, src) = open(&written(&m));
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(
        folder_tree(&after),
        vec![("A".to_string(), false, Vec::<String>::new(), 0)]
    );
    assert!(after.layers.iter().any(|l| l.name == "A"));
}

fn two_folders_of_one_name() -> Model {
    let mut m = authored_model(&[&["F", "x"], &["G", "y"]]);
    m.features.layer_folders[1].name = "F".into();
    m
}

#[test]
fn sibling_folders_of_one_name_open_as_distinct_tags_and_save_back_by_name() {
    let m = two_folders_of_one_name();
    let before = folder_tree(&m);
    assert_eq!(
        before.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        vec!["F", "F"]
    );
    let bytes = written(&m);
    let (doc, src) = open(&bytes);
    let t = tags(&doc);
    assert!(t.contains(&(path(&["F", "x"]), false)), "{t:?}");
    assert!(t.contains(&(path(&["F (2)", "y"]), false)), "{t:?}");
    // The same segments on every open.
    let (again, _) = open(&bytes);
    assert_eq!(tags(&again), t);
    // An unchanged save writes the file's own folder names.
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    assert_eq!(folder_tree(&after), before);
    // So does a save of the saved file.
    let (doc2, src2) = open(&saved.export.bytes);
    assert_eq!(tags(&doc2), t);
    let twice = save(&doc2, Target::Skp2026, env(), Some(&src2)).unwrap();
    assert_eq!(
        folder_tree(&Model::parse(&twice.export.bytes).unwrap()),
        before
    );
}

#[test]
fn hiding_the_second_of_two_same_named_folders_hides_only_that_one() {
    let bytes = written(&two_folders_of_one_name());
    let (mut doc, src) = open(&bytes);
    doc.set_tag_hidden(path(&["F (2)"]), true);
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let after = Model::parse(&saved.export.bytes).unwrap();
    let tree = folder_tree(&after);
    assert_eq!(
        tree.iter().map(|f| (f.0.as_str(), f.1)).collect::<Vec<_>>(),
        vec![("F", false), ("F", true)]
    );
}

#[test]
fn deleting_a_same_named_folders_tag_leaves_the_other_folder_alone() {
    let bytes = written(&two_folders_of_one_name());
    let (mut doc, src) = open(&bytes);
    doc.delete_tag(&path(&["F", "x"])).unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    let tree = folder_tree(&Model::parse(&saved.export.bytes).unwrap());
    assert_eq!(
        tree.iter()
            .map(|f| (f.0.as_str(), f.2.clone()))
            .collect::<Vec<_>>(),
        vec![("F", vec!["y".to_string()])]
    );
}

#[test]
fn folder_segments_are_deterministic_and_distinct() {
    assert_eq!(
        skp_import::folder_segments(&["F", "F", "G", "F"]),
        vec!["F", "F (2)", "G", "F (3)"]
    );
    assert_eq!(
        skp_import::folder_segments(&["F", "F (2)", "F"]),
        vec!["F", "F (2)", "F (3)"]
    );
}

#[test]
fn a_hidden_layer_hides_the_tag_it_shares_with_a_visible_folder() {
    // The folder (visible, empty) is listed before the layer (hidden).
    let mut m = authored_model(&[&["Q"], &["Z"]]);
    let q = layer_index(&m, "Q");
    m.layers[q].name = "A".into();
    m.layers[q].visible = false;
    m.features.layer_folders.push(skpwright::LayerFolder {
        name: "A".into(),
        ..Default::default()
    });
    let (doc, _) = open(&written(&m));
    let t = tags(&doc);
    assert!(t.contains(&(path(&["A"]), true)), "{t:?}");
}

/// The writer's notes of a save of `name` to `target`, one entry per kind of
/// content the target drops.
fn notes_of(name: &str, target: Target) -> Vec<String> {
    let (doc, src) = open(&fixture(name));
    save(&doc, target, env(), Some(&src))
        .unwrap()
        .export
        .writer_notes
}

#[test]
fn a_save_to_a_stream_release_itemises_the_post_2017_content_it_drops() {
    let r2017 = Target::Stream(skpwright::Release::R2017);
    let has = |notes: &[String], what: &str| notes.iter().any(|n| n.starts_with(what));

    let folders = notes_of("f-layer-folder.skp", r2017);
    assert!(has(&folders, "layer folders"), "{folders:?}");

    let hatch = notes_of("g-group-hatch.skp", r2017);
    assert!(has(&hatch, "hatch patterns"), "{hatch:?}");
    assert!(has(&hatch, "instance hatch patterns"), "{hatch:?}");

    let pages = notes_of("g-page-folders.skp", r2017);
    assert!(has(&pages, "layer folders"), "{pages:?}");
    assert!(has(&pages, "page extras"), "{pages:?}");
}

#[test]
fn a_revision_17_save_itemises_what_only_revision_18_holds() {
    let notes = notes_of("g-page-folders.skp", Target::Skp2026Compat);
    assert!(
        notes
            .iter()
            .any(|n| n.contains("does not hold layer folders")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.contains("does not hold page extras")),
        "{notes:?}"
    );
    // And the file is a revision-17 one: it reads as such, and its save
    // back keeps to revision 17.
    let (doc, src) = open(&fixture("g-page-folders.skp"));
    let saved = save(&doc, Target::Skp2026Compat, env(), Some(&src)).unwrap();
    assert_eq!(
        skp_export::target_of(&saved.export.bytes),
        Target::Skp2026Compat
    );
    let m = Model::parse(&saved.export.bytes).unwrap();
    assert!(m.features.is_empty());
}

#[test]
fn a_save_that_drops_nothing_reports_nothing() {
    for name in [
        "f-layer-folder.skp",
        "g-group-hatch.skp",
        "g-page-folders.skp",
        "k-page-times.skp",
    ] {
        // By content, the file keeps all it holds.
        let notes = notes_of(name, Target::Skp2026);
        assert!(notes.is_empty(), "{name}: {notes:?}");
        let bytes = fixture(name);
        let (doc, src) = open(&bytes);
        let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
        assert_eq!(
            skp_export::target_of(&saved.export.bytes),
            Target::Skp2026,
            "{name}"
        );
    }
    // A model with no post-2017 content loses nothing to revision 17 or 2017.
    let doc = Document::new();
    for t in [
        Target::Skp2026,
        Target::Skp2026Compat,
        Target::Stream(skpwright::Release::R2017),
    ] {
        let out = skp_export::export(&doc, t, env()).unwrap();
        assert!(out.writer_notes.is_empty(), "{t:?}: {:?}", out.writer_notes);
        assert!(out.simplified.is_empty(), "{t:?}: {:?}", out.simplified);
    }
}

#[test]
fn nested_tags_flattened_for_a_target_without_folders_are_noted() {
    let mut doc = Document::new();
    doc.set_tag_hidden(path(&["Walls", "Exterior"]), false);
    let flat = skp_export::export(&doc, Target::Skp2026Compat, env()).unwrap();
    assert!(
        flat.simplified
            .iter()
            .any(|n| n.starts_with("layer folders")),
        "{:?}",
        flat.simplified
    );
    let stream =
        skp_export::export(&doc, Target::Stream(skpwright::Release::R2017), env()).unwrap();
    assert!(
        stream
            .simplified
            .iter()
            .any(|n| n.starts_with("layer folders")),
        "{:?}",
        stream.simplified
    );
    let full = skp_export::export(&doc, Target::Skp2026, env()).unwrap();
    assert!(full.simplified.is_empty(), "{:?}", full.simplified);
}
