//! Locks from a `.skp` (OW-1). SketchUp locks one placement at a time;
//! Hew locks a shape. A group placement is a shape of its own, so its lock
//! carries over as it is; a component is locked when any of its copies was,
//! and the open report says so in one line when that locks copies the file
//! left unlocked.
//!
//! The files are the corpus fixtures with chosen placements locked, written
//! back in their own container by Skpwright.

use std::path::Path;

use kernel::{Document, NodeId};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()))
}

/// `name` with the placements of persistent id in `lock` locked, and,
/// when `copy` names one, a further copy of that placement beside it
/// (unlocked, a fresh id), written back in the fixture's own container.
fn relocked(name: &str, lock: &[u32], copy: Option<u32>) -> Vec<u8> {
    let bytes = fixture(name);
    let mut m = skpwright::Model::parse(&bytes).unwrap();
    let mut found = 0;
    for run in &mut m.geometry {
        for p in &mut run.placed {
            if lock.contains(&p.pid) {
                p.locked = true;
                found += 1;
            }
        }
        if let Some(pid) = copy
            && let Some(p) = run.placed.iter().find(|p| p.pid == pid).cloned()
        {
            let mut twin = p;
            twin.pid = 9_000;
            twin.guid = String::new();
            twin.locked = false;
            twin.transform[9] += 100.0;
            run.placed.push(twin);
        }
    }
    assert_eq!(found, lock.len(), "{name}: every pid to lock is placed");
    for i in &mut m.instances {
        if lock.contains(&i.pid) {
            i.locked = Some(true);
        }
    }
    let env = skpwright::WriteEnv {
        unix_time: 1_800_000_000,
        seed: 7,
    };
    let version = skpwright::header_info(&bytes).unwrap().0;
    match skpwright::Release::of_version(&version) {
        Some(release)
            if matches!(
                skpwright::detect_container(&bytes),
                skpwright::Container::Stream
            ) =>
        {
            skpwright::StreamDocument::from_model_with(&m, release, env)
                .unwrap()
                .write()
        }
        _ => skpwright::Document::from_model_with(&m, env)
            .unwrap()
            .write()
            .unwrap(),
    }
}

/// Open `bytes` into a fresh document; the open's notes too.
fn open(bytes: &[u8]) -> (Document, Vec<String>) {
    let out = skp_import::import(bytes).unwrap();
    let warnings = out.warnings.clone();
    let mut doc = Document::new();
    doc.ingest(out.scene, out.textures_missing).unwrap();
    (doc, warnings)
}

fn note(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w == skp_import::LOCKED_COPIES_NOTE)
}

#[test]
fn a_locked_group_opens_locked_in_both_containers() {
    for name in ["box-group.skp", "2026/box-group.skp"] {
        let pid = skpwright::Model::parse(&fixture(name)).unwrap().geometry[2].placed[0].pid;
        let (doc, warnings) = open(&relocked(name, &[pid], None));
        // The one-solid group opens as a bare Object, which takes its lock.
        let objects = doc.visible_object_ids();
        assert_eq!(objects.len(), 1, "{name}");
        assert!(doc.node_locked(NodeId::Object(objects[0])), "{name}");
        assert!(!note(&warnings), "{name}: a group is a shape of its own");

        // The same file unlocked opens unlocked.
        let (doc, _) = open(&relocked(name, &[], None));
        assert!(
            !doc.node_locked(NodeId::Object(doc.visible_object_ids()[0])),
            "{name}"
        );
    }
}

#[test]
fn one_locked_copy_of_three_locks_the_component_and_says_so() {
    for name in [
        "box-component-two-instances.skp",
        "2026/box-component-two-instances.skp",
    ] {
        let m = skpwright::Model::parse(&fixture(name)).unwrap();
        let root = m.geometry.iter().find(|g| g.def_index.is_none()).unwrap();
        let (first, second) = (root.placed[0].pid, root.placed[1].pid);

        let (doc, warnings) = open(&relocked(name, &[first], Some(second)));
        assert_eq!(doc.instance_ids().len(), 3, "{name}: three copies");
        assert_eq!(doc.component_ids().len(), 1, "{name}");
        assert!(doc.component_locked(doc.component_ids()[0]), "{name}");
        for i in doc.instance_ids() {
            assert!(doc.node_locked(NodeId::Instance(i)), "{name}: every copy");
        }
        assert!(note(&warnings), "{name}: {warnings:?}");
        assert_eq!(
            warnings
                .iter()
                .filter(|w| *w == skp_import::LOCKED_COPIES_NOTE)
                .count(),
            1,
            "{name}: one line"
        );

        // Every copy locked: nothing changes, so nothing is said.
        let (doc, warnings) = open(&relocked(name, &[first, second], None));
        assert!(doc.component_locked(doc.component_ids()[0]), "{name}");
        assert!(!note(&warnings), "{name}: {warnings:?}");

        // No copy locked: the component is not.
        let (doc, warnings) = open(&relocked(name, &[], Some(second)));
        assert!(!doc.component_locked(doc.component_ids()[0]), "{name}");
        assert!(!note(&warnings), "{name}");
    }
}

#[test]
fn a_locked_copy_inside_a_definition_locks_the_shape_it_places() {
    // nested-3-deep: the leaf is placed twice inside the middle
    // definition, which is placed twice inside the outer one. Locking one
    // leaf placement locks the leaf definition: all four placements of it
    // in the world are locked; the definitions above are not.
    let name = "nested-3-deep.skp";
    let m = skpwright::Model::parse(&fixture(name)).unwrap();
    let leaf_run = m
        .geometry
        .iter()
        .find(|g| g.placed.len() == 2 && g.placed.iter().all(|p| !p.is_group))
        .unwrap();
    let pid = leaf_run.placed[0].pid;
    let (doc, warnings) = open(&relocked(name, &[pid], None));
    let locked: Vec<_> = doc
        .component_ids()
        .into_iter()
        .filter(|&c| doc.component_locked(c))
        .collect();
    assert_eq!(locked.len(), 1, "only the shape the locked copy places");
    assert_eq!(doc.def_depth(locked[0]), 1, "the leaf");
    assert!(note(&warnings), "{warnings:?}");
    // The world placement places the outer definition, which is free.
    let world: Vec<NodeId> = doc
        .top_level_nodes()
        .into_iter()
        .filter(|n| matches!(n, NodeId::Instance(_)))
        .collect();
    assert_eq!(world.len(), 1);
    assert!(!doc.node_locked(world[0]));
}
