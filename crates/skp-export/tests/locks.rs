//! Locks through a `.skp` (OW-1). Hew locks a shape; SketchUp locks one
//! placement at a time. A file written from Hew carries the shape's lock on
//! every copy of it: an Object's or group's own lock, an instance's
//! definition's. A save back over an existing file writes the document's
//! lock instead of keeping the file's. A release too old to store locks
//! drops them, and the writer's notes say so.

mod common;

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use common::digest;
use kernel::{
    ComponentId, Document, EntityRef, LinkPolicy, NodeId, ObjectId, Plane, Point3, Transform, Vec3,
};
use skp_export::{SkpSource, Target, WriteEnv, export, save};

// ── Helpers ───────────────────────────────────────────────────────────────

fn env() -> WriteEnv {
    WriteEnv {
        unix_time: 1_800_000_000,
        seed: 11,
    }
}

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

/// A unit box at `x` on the ground.
fn board(doc: &mut Document, x: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).unwrap();
    }
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap().0
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

fn entity(node: NodeId) -> EntityRef {
    match node {
        NodeId::Object(o) => EntityRef::Object(o),
        NodeId::Group(g) => EntityRef::Group(g),
        NodeId::Instance(i) => EntityRef::Instance(i),
    }
}

/// Every placement in a written file, by persistent id: its lock.
fn file_locks(bytes: &[u8]) -> BTreeMap<u32, bool> {
    let m = skpwright::Model::parse(bytes).unwrap();
    m.geometry
        .iter()
        .flat_map(|g| g.placed.iter().map(|p| (p.pid, p.locked)))
        .collect()
}

/// The lock a written file gives `node` (through the pid the write took).
fn written_lock(doc: &Document, pids: &BTreeMap<u64, u32>, bytes: &[u8], node: NodeId) -> bool {
    let sid = doc.sid_of(&entity(node)).unwrap();
    let pid = *pids
        .get(&sid)
        .unwrap_or_else(|| panic!("{node:?} was written with a pid"));
    let locks = file_locks(bytes);
    *locks
        .get(&pid)
        .unwrap_or_else(|| panic!("{node:?}: pid {pid} is placed in {locks:?}"))
}

/// Open `bytes` as the app does.
fn open(bytes: &[u8]) -> (Document, SkpSource, Vec<String>) {
    let out = skp_import::import(bytes).unwrap();
    let warnings = out.warnings.clone();
    let provenance = out.provenance.clone();
    let mut doc = Document::new();
    let (report, _) = doc.ingest(out.scene, out.textures_missing).unwrap();
    doc.forget_history().unwrap();
    let src = SkpSource::after_open(bytes.to_vec(), &doc, &provenance, &report);
    (doc, src, warnings)
}

/// What the document holds:
/// - a locked Object and a free one;
/// - a locked group of two boards;
/// - three copies of a component, locked through one of them (so all are);
/// - two copies of a free component;
/// - with `legs`, a linked set of two copies, locked.
struct Scene {
    doc: Document,
    locked_object: NodeId,
    free_object: NodeId,
    group: NodeId,
    group_members: Vec<NodeId>,
    window: ComponentId,
    windows: Vec<NodeId>,
    doors: Vec<NodeId>,
    legs: Vec<NodeId>,
    leg_def: Option<ComponentId>,
}

fn scene(with_legs: bool) -> Scene {
    let mut doc = Document::new();
    let locked_object = NodeId::Object(board(&mut doc, 0.0));
    let free_object = NodeId::Object(board(&mut doc, 3.0));
    let a = NodeId::Object(board(&mut doc, 6.0));
    let b = NodeId::Object(board(&mut doc, 8.0));
    let (g, _) = doc.group_nodes(&[a, b]).unwrap();
    let group = NodeId::Group(g);

    let w = NodeId::Object(board(&mut doc, 12.0));
    let (window, w1, _) = doc.make_component(&[w]).unwrap();
    let (w2, _) = doc.place_instance(window, shift(2.0)).unwrap();
    let (w3, _) = doc.place_instance(window, shift(4.0)).unwrap();
    let windows = vec![
        NodeId::Instance(w1),
        NodeId::Instance(w2),
        NodeId::Instance(w3),
    ];

    let d = NodeId::Object(board(&mut doc, 20.0));
    let (door, d1, _) = doc.make_component(&[d]).unwrap();
    let (d2, _) = doc.place_instance(door, shift(2.0)).unwrap();
    let doors = vec![NodeId::Instance(d1), NodeId::Instance(d2)];

    let (legs, leg_def) = if with_legs {
        let leg = NodeId::Object(board(&mut doc, 30.0));
        let (copies, change) = doc
            .duplicate_nodes_array(
                &[leg],
                &shift(2.0),
                NonZeroU32::new(1).unwrap(),
                LinkPolicy::Linked,
            )
            .unwrap();
        let legs = vec![NodeId::Instance(change.linked[0].instance), copies[0]];
        (legs, Some(change.linked[0].definition))
    } else {
        (Vec::new(), None)
    };

    let mut to_lock = vec![locked_object, group, windows[1]];
    to_lock.extend(legs.last().copied());
    doc.set_nodes_locked(&to_lock, true).unwrap();
    let group_members = doc.group_members(g).unwrap();
    Scene {
        doc,
        locked_object,
        free_object,
        group,
        group_members,
        window,
        windows,
        doors,
        legs,
        leg_def,
    }
}

fn targets() -> Vec<Target> {
    vec![
        Target::Skp2026,
        Target::Skp2026Compat,
        Target::Stream(skpwright::Release::R2017),
        Target::Stream(skpwright::Release::R5),
    ]
}

// ── Export ────────────────────────────────────────────────────────────────

#[test]
fn every_copy_of_a_locked_shape_is_written_locked() {
    let s = scene(true);
    let leg_def = s.leg_def.unwrap();
    assert!(s.doc.component_locked(s.window));
    assert!(s.doc.component_locked(leg_def));
    for target in targets() {
        let out = export(&s.doc, target, env()).unwrap();
        // Twelve placements: two Objects, the group and its two members,
        // three windows, two doors, two legs. Seven locked: the Object,
        // the group, the windows and the legs.
        let locks = file_locks(&out.bytes);
        assert_eq!(locks.len(), 12, "{target:?}");
        assert_eq!(locks.values().filter(|&&l| l).count(), 7, "{target:?}");
        assert!(
            out.writer_notes.is_empty(),
            "{target:?}: {:?}",
            out.writer_notes
        );
        // A stream release numbers its placements afresh as it writes
        // them: the pids the export reports are the archive's.
        if !target.is_archive() {
            continue;
        }
        let lock = |n: NodeId| written_lock(&s.doc, &out.pids, &out.bytes, n);
        assert!(lock(s.locked_object), "{target:?}");
        assert!(!lock(s.free_object), "{target:?}");
        assert!(lock(s.group), "{target:?}");
        // The members of a locked group are frozen by it, not locked.
        for &m in &s.group_members {
            assert!(!lock(m), "{target:?}");
        }
        for &w in &s.windows {
            assert!(lock(w), "{target:?}: every copy of the locked window");
        }
        for &d in &s.doors {
            assert!(!lock(d), "{target:?}");
        }
        for &l in &s.legs {
            assert!(lock(l), "{target:?}: every copy of the locked set");
        }
        // The linked set's member carries no lock of its own: only its
        // placements do. (A definition of one solid holds it as the
        // definition's own geometry, with no placement to lock.)
        let member = s.doc.def_member_nodes(leg_def).unwrap()[0];
        assert!(!s.doc.node_locked(member));
        let sid = s.doc.sid_of(&entity(member)).unwrap();
        assert!(!out.pids.contains_key(&sid), "{target:?}");
    }
}

#[test]
fn locks_survive_hew_to_skp_to_hew() {
    let s = scene(false);
    let before = digest(&s.doc);
    for target in targets() {
        let out = export(&s.doc, target, env()).unwrap();
        let (back, _, warnings) = open(&out.bytes);
        assert_eq!(before.roots, digest(&back).roots, "{target:?}");
        // Every copy was locked alike, so nothing was spread.
        assert!(
            !warnings.iter().any(|w| w == skp_import::LOCKED_COPIES_NOTE),
            "{target:?}"
        );
    }
    // A linked set comes back as a component (a `.skp` has no linked
    // sets), locked as the set was, its member free.
    let s = scene(true);
    for target in targets() {
        let out = export(&s.doc, target, env()).unwrap();
        let (back, _, _) = open(&out.bytes);
        let copies: Vec<NodeId> = back
            .top_level_nodes()
            .into_iter()
            .filter(|n| matches!(n, NodeId::Instance(_)))
            .collect();
        assert_eq!(copies.len(), 7, "{target:?}");
        assert_eq!(
            copies.iter().filter(|&&n| back.node_locked(n)).count(),
            5,
            "{target:?}: three windows and two legs"
        );
        for c in back.component_ids() {
            for m in back.def_member_nodes(c).unwrap() {
                assert!(!back.node_locked(m), "{target:?}");
            }
        }
    }
}

#[test]
fn a_release_too_old_for_locks_drops_them_and_says_so() {
    let s = scene(true);
    let out = export(&s.doc, Target::Stream(skpwright::Release::R4), env()).unwrap();
    assert!(
        file_locks(&out.bytes).values().all(|&l| !l),
        "release 4 has no lock to write"
    );
    assert!(
        out.writer_notes
            .iter()
            .any(|n| n.starts_with("locks are not stored by release 4")),
        "{:?}",
        out.writer_notes
    );
    // Release 5 is the first that stores them, and drops nothing.
    let out = export(&s.doc, Target::Stream(skpwright::Release::R5), env()).unwrap();
    assert!(file_locks(&out.bytes).values().any(|&l| l));
    assert!(
        !out.writer_notes.iter().any(|n| n.contains("lock")),
        "{:?}",
        out.writer_notes
    );
}

// ── A file SketchUp locked one copy at a time ─────────────────────────────

/// A group holding one copy of a component, two more copies of it beside
/// the group, written from Hew unlocked; then, as SketchUp would, only the
/// copy inside the group locked. Returns the file, and the pid of the copy
/// locked.
fn locked_copy_in_a_group(target: Target) -> (Vec<u8>, u32) {
    let mut doc = Document::new();
    let o = NodeId::Object(board(&mut doc, 0.0));
    let (def, inner, _) = doc.make_component(&[o]).unwrap();
    let wall = NodeId::Object(board(&mut doc, 5.0));
    let (g, _) = doc.group_nodes(&[NodeId::Instance(inner), wall]).unwrap();
    doc.place_instance(def, shift(10.0)).unwrap();
    doc.place_instance(def, shift(12.0)).unwrap();
    let inner = doc
        .group_members(g)
        .unwrap()
        .into_iter()
        .find(|n| matches!(n, NodeId::Instance(_)))
        .unwrap();
    let out = export(&doc, target, env()).unwrap();
    let pid = out.pids[&doc.sid_of(&entity(inner)).unwrap()];
    let mut m = skpwright::Model::parse(&out.bytes).unwrap();
    let mut n = 0;
    for run in &mut m.geometry {
        for p in &mut run.placed {
            if p.pid == pid {
                p.locked = true;
                n += 1;
            }
        }
    }
    assert_eq!(n, 1);
    let bytes = match target {
        Target::Stream(r) => skpwright::StreamDocument::from_model_with(&m, r, env())
            .unwrap()
            .write(),
        _ => skpwright::Document::from_model_with(&m, env())
            .unwrap()
            .write()
            .unwrap(),
    };
    (bytes, pid)
}

/// Every placement of the file's one component definition, by pid: its
/// lock.
fn component_copy_locks(bytes: &[u8]) -> Vec<bool> {
    let m = skpwright::Model::parse(bytes).unwrap();
    let comp = m
        .definitions
        .iter()
        .filter(|d| d.kind == skpwright::DefinitionKind::Component)
        .filter_map(|d| d.map_index)
        .collect::<Vec<_>>();
    assert_eq!(comp.len(), 1, "one component definition");
    let mut locks: Vec<bool> = m
        .geometry
        .iter()
        .flat_map(|g| g.placed.iter())
        .filter(|p| p.defref as usize == comp[0])
        .map(|p| p.locked)
        .collect();
    locks.sort();
    locks
}

#[test]
fn a_locked_copy_inside_a_group_locks_every_copy_on_open() {
    for target in [Target::Skp2026, Target::Stream(skpwright::Release::R2017)] {
        let (bytes, _) = locked_copy_in_a_group(target);
        assert_eq!(component_copy_locks(&bytes), vec![false, false, true]);
        let (doc, _, warnings) = open(&bytes);
        let comps = doc.component_ids();
        assert_eq!(comps.len(), 1, "{target:?}");
        assert!(doc.component_locked(comps[0]), "{target:?}");
        let mut copies = 0;
        for n in doc.top_level_nodes() {
            match n {
                NodeId::Instance(_) => {
                    assert!(doc.node_locked(n), "{target:?}");
                    copies += 1;
                }
                NodeId::Group(g) => {
                    assert!(!doc.node_locked(n), "{target:?}: the group is free");
                    for m in doc.group_members(g).unwrap() {
                        if let NodeId::Instance(_) = m {
                            assert!(doc.node_locked(m), "{target:?}");
                            copies += 1;
                        }
                    }
                }
                NodeId::Object(_) => {}
            }
        }
        assert_eq!(copies, 3, "{target:?}");
        assert!(
            warnings.iter().any(|w| w == skp_import::LOCKED_COPIES_NOTE),
            "{target:?}: {warnings:?}"
        );
    }
}

// ── Save back over a file ─────────────────────────────────────────────────

/// An unchanged save back writes the document's lock on every copy: the
/// copies the file left unlocked come back locked, since the open locked
/// the component. Unlocking in Hew and saving again unlocks every copy,
/// the one inside the (otherwise untouched) group too. A copy placed
/// since is written with the shape's lock.
#[test]
fn a_save_back_writes_the_documents_locks_over_the_files() {
    for target in [Target::Skp2026, Target::Stream(skpwright::Release::R2017)] {
        let (bytes, _) = locked_copy_in_a_group(target);
        let (doc, src, _) = open(&bytes);
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        assert_eq!(
            component_copy_locks(&saved.export.bytes),
            vec![true, true, true],
            "{target:?}: an unchanged save writes the shape's lock"
        );

        // Unlocked in Hew: every copy unlocked, inside the group too.
        let mut unlocked = doc.clone();
        let def = unlocked.component_ids()[0];
        let any_copy = unlocked
            .top_level_nodes()
            .into_iter()
            .find(|n| matches!(n, NodeId::Instance(_)))
            .unwrap();
        unlocked.set_nodes_locked(&[any_copy], false).unwrap();
        assert!(!unlocked.component_locked(def));
        let saved = save(&unlocked, target, env(), Some(&src)).unwrap();
        assert_eq!(
            component_copy_locks(&saved.export.bytes),
            vec![false, false, false],
            "{target:?}: the document's lock wins over the file's"
        );

        // A copy placed since takes the shape's lock.
        let mut more = doc.clone();
        let def = more.component_ids()[0];
        let (new_copy, _) = more.place_instance(def, shift(20.0)).unwrap();
        let saved = save(&more, target, env(), Some(&src)).unwrap();
        assert_eq!(
            component_copy_locks(&saved.export.bytes),
            vec![true, true, true, true],
            "{target:?}"
        );
        assert!(written_lock(
            &more,
            &saved.export.pids,
            &saved.export.bytes,
            NodeId::Instance(new_copy)
        ));
    }
}

/// A group put back unchanged keeps its placement record (its GUID) and
/// takes the document's lock: locked in the file, unlocked in Hew, the
/// save writes it unlocked. A lock set in Hew on a member of an unchanged
/// group rewrites the group, so the member is written locked.
#[test]
fn a_put_back_group_and_its_members_take_the_documents_locks() {
    let target = Target::Skp2026;
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let b = NodeId::Object(board(&mut doc, 2.0));
    let (g, _) = doc.group_nodes(&[a, b]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    let file = export(&doc, target, env()).unwrap().bytes;
    let group_placement = |bytes: &[u8]| -> skpwright::PlacedInstance {
        let m = skpwright::Model::parse(bytes).unwrap();
        let root = m.geometry.iter().find(|r| r.def_index.is_none()).unwrap();
        assert_eq!(root.placed.len(), 1);
        root.placed[0].clone()
    };
    let original = group_placement(&file);
    assert!(original.locked);

    let (opened, src, _) = open(&file);
    let group = opened.top_level_nodes()[0];
    assert!(opened.node_locked(group));
    let saved = save(&opened, target, env(), Some(&src)).unwrap();
    let kept = group_placement(&saved.export.bytes);
    assert!(kept.locked);
    assert_eq!(kept.guid, original.guid, "put back, not rewritten");

    let mut unlocked = opened.clone();
    unlocked.set_nodes_locked(&[group], false).unwrap();
    let saved = save(&unlocked, target, env(), Some(&src)).unwrap();
    let kept = group_placement(&saved.export.bytes);
    assert!(!kept.locked, "the document's lock, not the file's");
    assert_eq!(kept.guid, original.guid, "still put back");

    // Lock one member of the now unlocked group.
    let saved_unlocked = saved;
    let (mut reopened, src, _) = open(&saved_unlocked.export.bytes);
    let NodeId::Group(rg) = reopened.top_level_nodes()[0] else {
        panic!("a group")
    };
    let member = reopened.group_members(rg).unwrap()[0];
    reopened.set_nodes_locked(&[member], true).unwrap();
    let saved = save(&reopened, target, env(), Some(&src)).unwrap();
    assert!(written_lock(
        &reopened,
        &saved.export.pids,
        &saved.export.bytes,
        member
    ));
    let locks = file_locks(&saved.export.bytes);
    assert_eq!(
        locks.values().filter(|&&l| l).count(),
        1,
        "only the member: {locks:?}"
    );
}

// ── Loose geometry ────────────────────────────────────────────────────────

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../skp-import/tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// The top-level Objects of `doc`, by the layer they are tagged with.
fn loose_by_tag(doc: &Document) -> BTreeMap<String, NodeId> {
    doc.top_level_nodes()
        .into_iter()
        .filter(|n| matches!(n, NodeId::Object(_)))
        .map(|n| (doc.node_tags(n).concat().join("/"), n))
        .collect()
}

/// SketchUp's loose geometry has no placement to carry a lock, so a save
/// back writes a loose Object locked in Hew as a group carrying the lock
/// (as a new file writes every Object), and it reopens locked. The rest
/// of the loose geometry stays loose, whether it is kept as the file had
/// it or rewritten beside a moved box.
#[test]
fn a_loose_object_locked_in_hew_is_saved_locked() {
    let bytes = fixture("layers.skp");
    let target = skp_export::target_of(&bytes);
    for moved in [false, true] {
        let (mut doc, src, _) = open(&bytes);
        let loose = loose_by_tag(&doc);
        assert_eq!(loose.len(), 3, "layers.skp: three loose boxes");
        let big = loose["BigLayer"];
        doc.set_nodes_locked(&[big], true).unwrap();
        if moved {
            let NodeId::Object(little) = loose["LittleLayer"] else {
                unreachable!()
            };
            doc.transform_object(little, &Transform::translation(Vec3::new(0.0, 0.0, 1.0)))
                .unwrap();
        }
        let saved = save(&doc, target, env(), Some(&src)).unwrap();
        assert!(
            written_lock(&doc, &saved.export.pids, &saved.export.bytes, big),
            "moved {moved}: the locked box has a placement carrying its lock"
        );
        let m = skpwright::Model::parse(&saved.export.bytes).unwrap();
        let root = m.geometry.iter().find(|r| r.def_index.is_none()).unwrap();
        assert_eq!(
            root.mesh.faces.len(),
            12,
            "moved {moved}: the two free boxes stay loose"
        );
        let (back, _, _) = open(&saved.export.bytes);
        let back_loose = loose_by_tag(&back);
        assert!(
            back.node_locked(back_loose["BigLayer"]),
            "moved {moved}: it reopens locked"
        );
        assert!(!back.node_locked(back_loose["LittleLayer"]));
        assert!(!back.node_locked(back_loose["MediumLayer"]));
    }
}

/// SketchUp's placed image carries no lock, so a save back writes a
/// locked image unlocked and its notes say so; an unlocked one says
/// nothing.
#[test]
fn a_locked_placed_image_is_saved_unlocked_with_a_note() {
    const NOTE: &str =
        "placed images cannot be locked in SketchUp; locked images are saved unlocked";
    let bytes = fixture("2026/feature-pack.skp");
    let (mut doc, src, _) = open(&bytes);
    let image = doc
        .instance_ids()
        .into_iter()
        .find(|&i| {
            doc.sid_of(&EntityRef::Instance(i))
                .and_then(|s| src.nodes.get(&s))
                .is_some_and(|n| matches!(n.origin, skp_import::Origin::Image { .. }))
        })
        .expect("the image is an instance");
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    assert!(!saved.export.simplified.iter().any(|n| n == NOTE));
    doc.set_nodes_locked(&[NodeId::Instance(image)], true)
        .unwrap();
    let saved = save(&doc, Target::Skp2026, env(), Some(&src)).unwrap();
    assert!(
        saved.export.simplified.iter().any(|n| n == NOTE),
        "{:?}",
        saved.export.simplified
    );
}
