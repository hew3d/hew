//! Structure-and-session fuzz harness (DEVELOPMENT.md rule 3): random
//! sequences of the tree-shaping ops — make component, group, ungroup,
//! reparent, explode, place, make unique, duplicate, delete, delete
//! definition — interleaved with component and group EDITING SESSIONS
//! (opened, stacked, closed innermost-first) and undo/redo, over a document
//! of plain boxes.
//!
//! `document_fuzz.rs` covers geometry-heavy ops but never opens an editing
//! session, so a session that closed into an inconsistent tree went
//! unnoticed there. This harness exists for that gap; every defect it found
//! (a group close re-homing a node a component had taken, an emptied
//! definition tombstoned under a live nested placement, a placement gate
//! that ignored a group frame) saved a file the loader then refused.
//!
//! Invariants, after every op (applied or refused typed):
//! - the always-on validator (torture mode) and the debug tree assertions
//!   do not fire;
//! - `load(save_for_persistence())` succeeds — the autosave path, which
//!   serializes as if every open session were closed;
//! - with no session open, `load(save())` succeeds too.
//!
//! Failures shrink to a short op list. Run deeper than the default 256
//! cases with `PROPTEST_CASES=20000 cargo test -p kernel --test
//! session_fuzz`.

use kernel::{Document, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
use proptest::prelude::*;

/// Abstract op; selectors resolve against live ids at apply time, so a
/// shrunk sequence stays meaningful.
#[derive(Debug, Clone)]
enum Op {
    MakeComponent {
        sel: usize,
    },
    MakeComponentPair {
        a: usize,
        b: usize,
    },
    Group {
        a: usize,
        b: usize,
    },
    Ungroup {
        sel: usize,
    },
    Reparent {
        sel: usize,
        parent: usize,
    },
    Explode {
        sel: usize,
    },
    Place {
        comp: usize,
    },
    MakeUnique {
        sel: usize,
    },
    Duplicate {
        sel: usize,
    },
    Delete {
        sel: usize,
    },
    DeleteSelection {
        sel: usize,
    },
    DeleteDefinition {
        comp: usize,
    },
    OpenComponentSession {
        sel: usize,
    },
    OpenGroupSession {
        sel: usize,
    },
    /// Delete every object inside the open component session, so the close
    /// empties the definition (the grouped-sibling emptied-close defect).
    DeleteSessionMembers,
    CloseSession,
    Undo,
    Redo,
}

fn arb_op() -> impl Strategy<Value = Op> {
    let sel = || any::<usize>();
    prop_oneof![
        3 => sel().prop_map(|sel| Op::MakeComponent { sel }),
        1 => (sel(), sel()).prop_map(|(a, b)| Op::MakeComponentPair { a, b }),
        2 => (sel(), sel()).prop_map(|(a, b)| Op::Group { a, b }),
        2 => sel().prop_map(|sel| Op::Ungroup { sel }),
        2 => (sel(), sel()).prop_map(|(sel, parent)| Op::Reparent { sel, parent }),
        1 => sel().prop_map(|sel| Op::Explode { sel }),
        2 => sel().prop_map(|comp| Op::Place { comp }),
        1 => sel().prop_map(|sel| Op::MakeUnique { sel }),
        1 => sel().prop_map(|sel| Op::Duplicate { sel }),
        2 => sel().prop_map(|sel| Op::Delete { sel }),
        1 => sel().prop_map(|sel| Op::DeleteSelection { sel }),
        1 => sel().prop_map(|comp| Op::DeleteDefinition { comp }),
        3 => sel().prop_map(|sel| Op::OpenComponentSession { sel }),
        3 => sel().prop_map(|sel| Op::OpenGroupSession { sel }),
        2 => Just(Op::DeleteSessionMembers),
        3 => Just(Op::CloseSession),
        2 => Just(Op::Undo),
        2 => Just(Op::Redo),
    ]
}

fn add_box(doc: &mut Document, x: f64) -> ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined");
    let s = doc.add_sketch(plane);
    let corners = [(x, 0.0), (x + 1.0, 0.0), (x + 1.0, 1.0), (x, 1.0)];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .expect("segment");
    }
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], 1.0).expect("extrude").0
}

/// Every live node, in a stable order, for selectors to index into.
fn live_nodes(doc: &Document) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = doc
        .live_object_ids()
        .into_iter()
        .map(NodeId::Object)
        .collect();
    v.extend(doc.group_ids().into_iter().map(NodeId::Group));
    v.extend(doc.instance_ids().into_iter().map(NodeId::Instance));
    v
}

fn pick<T: Copy>(items: &[T], sel: usize) -> Option<T> {
    (!items.is_empty()).then(|| items[sel % items.len()])
}

/// Applies one op. A typed refusal is a legal answer and is ignored; what
/// must never happen is a panic or a document that no longer reopens, both
/// checked by the caller.
fn apply(doc: &mut Document, op: &Op) {
    let nodes = live_nodes(doc);
    let instances = doc.instance_ids();
    let groups = doc.group_ids();
    let comps = doc.component_ids();
    match *op {
        Op::MakeComponent { sel } => {
            if let Some(n) = pick(&nodes, sel) {
                let _ = doc.make_component(&[n]);
            }
        }
        Op::MakeComponentPair { a, b } => {
            if let (Some(a), Some(b)) = (pick(&nodes, a), pick(&nodes, b)) {
                let _ = doc.make_component(&[a, b]);
            }
        }
        Op::Group { a, b } => {
            if let (Some(a), Some(b)) = (pick(&nodes, a), pick(&nodes, b)) {
                let _ = doc.group_nodes(&[a, b]);
            }
        }
        Op::Ungroup { sel } => {
            if let Some(g) = pick(&groups, sel) {
                let _ = doc.ungroup(g);
            }
        }
        Op::Reparent { sel, parent } => {
            if let Some(n) = pick(&nodes, sel) {
                // One extra slot past the groups means "to the top level".
                let at = parent % (groups.len() + 1);
                let target = groups.get(at).copied();
                let _ = doc.reparent_nodes(&[n], target);
            }
        }
        Op::Explode { sel } => {
            if let Some(i) = pick(&instances, sel) {
                let _ = doc.explode_instance(i);
            }
        }
        Op::Place { comp } => {
            if let Some(c) = pick(&comps, comp) {
                let _ = doc.place_instance(c, Transform::translation(Vec3::new(1.0, 2.0, 0.0)));
            }
        }
        Op::MakeUnique { sel } => {
            if let Some(i) = pick(&instances, sel) {
                let _ = doc.make_unique(i);
            }
        }
        Op::Duplicate { sel } => {
            if let Some(n) = pick(&nodes, sel) {
                let _ = doc.duplicate_node(n, &Transform::translation(Vec3::new(0.5, 0.5, 0.0)));
            }
        }
        Op::Delete { sel } => {
            if let Some(n) = pick(&nodes, sel) {
                let _ = doc.delete_node(n);
            }
        }
        Op::DeleteSelection { sel } => {
            if let Some(n) = pick(&nodes, sel) {
                let _ = doc.delete_selection(&[n]);
            }
        }
        Op::DeleteDefinition { comp } => {
            if let Some(c) = pick(&comps, comp) {
                let _ = doc.delete_definition(c);
            }
        }
        Op::OpenComponentSession { sel } => {
            if let Some(i) = pick(&instances, sel) {
                let _ = doc.open_explode_session(i);
            }
        }
        Op::OpenGroupSession { sel } => {
            if let Some(g) = pick(&groups, sel) {
                let _ = doc.open_group_session(g);
            }
        }
        Op::DeleteSessionMembers => {
            for o in doc.explode_session_objects().unwrap_or_default() {
                let _ = doc.delete_node(NodeId::Object(o));
            }
        }
        Op::CloseSession => {
            let _ = doc.close_innermost_session();
        }
        Op::Undo => {
            let _ = doc.undo();
        }
        Op::Redo => {
            let _ = doc.redo();
        }
    }
}

fn check_reopens(doc: &Document, step: usize, op: &Op) -> Result<(), TestCaseError> {
    if let Err(e) = Document::load(&doc.save_for_persistence()) {
        return Err(TestCaseError::fail(format!(
            "step {step} ({op:?}): the autosave bytes do not reopen: {e}"
        )));
    }
    let closed = doc
        .session_stack()
        .is_empty()
        .then(|| Document::load(&doc.save()));
    if let Some(Err(e)) = closed {
        return Err(TestCaseError::fail(format!(
            "step {step} ({op:?}): the saved bytes do not reopen: {e}"
        )));
    }
    Ok(())
}

/// Live sibling placements (ARCHITECTURE.md 2.11): with a component frame
/// open, every placement a renderer would draw — `expanded_placements` —
/// is a LIVE, WORLD-owned object (a sibling renders the frame's world
/// geometry, never a tombstoned or still-definition-owned record), its
/// placing instance is live, and every `session_placements` entry is a
/// live world instance. With nothing open, `session_placements` is empty.
fn check_sibling_render(doc: &Document, step: usize, op: &Op) -> Result<(), TestCaseError> {
    let fail = |what: String| TestCaseError::fail(format!("step {step} ({op:?}): {what}"));
    let live_instances = doc.instance_ids();
    let live_objects = doc.live_object_ids();
    for (o, _, i) in doc.expanded_placements() {
        if !live_instances.contains(&i) {
            return Err(fail(format!("placement by a non-live instance {i:?}")));
        }
        if !live_objects.contains(&o) {
            return Err(fail(format!(
                "a placement renders a non-live object {o:?} through {i:?}"
            )));
        }
    }
    let placements = doc.session_placements();
    if doc.session_stack().is_empty() && !placements.is_empty() {
        return Err(fail("session placements with no session open".to_string()));
    }
    for &i in &placements {
        if !live_instances.contains(&i) {
            return Err(fail(format!(
                "session placement {i:?} is not a live instance"
            )));
        }
    }
    // The innermost component frame's definition renders, on every live
    // sibling, exactly the session's live world objects (not a stale
    // def-local member, not a tombstone): the live-sibling contract itself.
    if let Some(def) = doc.explode_session_component() {
        // `explode_session_objects` lists what the frame's actions created,
        // liveness-filtered only — a creation that a deeper frame has since
        // folded into ITS definition is def-owned now and renders as a
        // member of that nested placement, so compare world objects only.
        let mut expected: Vec<ObjectId> = doc
            .explode_session_objects()
            .unwrap_or_default()
            .into_iter()
            .filter(|&o| doc.is_world_object(o))
            .collect();
        // A nested instance the frame surfaced or placed renders ITS
        // definition; when that definition is on loan to an OUTER frame
        // (a component holding a copy of the component being edited two
        // frames up), it renders that frame's in-progress world geometry
        // through its own pose: the on-loan rule one level down. Those
        // world objects are expected too, reached the same way a renderer
        // reaches them. The frame's direct members are its top-level
        // nodes; a surfaced member group is descended, because the open
        // surfaces the instances inside it as well.
        let mut pending = doc.session_direct_members().unwrap_or_default();
        while let Some(node) = pending.pop() {
            match node {
                NodeId::Group(g) => pending.extend(doc.group_members(g).unwrap_or_default()),
                NodeId::Instance(i) => {
                    let Some(nested) = doc.instance_def(i) else {
                        continue;
                    };
                    expected.extend(
                        doc.expanded_def_placements(nested)
                            .iter()
                            .map(|(o, _)| *o)
                            .filter(|&o| doc.is_world_object(o)),
                    );
                }
                NodeId::Object(_) => {}
            }
        }
        expected.sort();
        expected.dedup();
        // A nested member instance surfaced by the frame expands to ITS
        // definition's def-owned members through its pose, so the world
        // objects among the rendered set are exactly the session's, and
        // every other rendered object is a live definition member.
        let rendered: Vec<ObjectId> = doc
            .expanded_def_placements(def)
            .iter()
            .map(|(o, _)| *o)
            .collect();
        for &o in &rendered {
            if !live_objects.contains(&o) {
                return Err(fail(format!(
                    "a sibling of the open definition renders non-live object {o:?}"
                )));
            }
        }
        let mut rendered_world: Vec<ObjectId> = rendered
            .iter()
            .copied()
            .filter(|&o| doc.is_world_object(o))
            .collect();
        rendered_world.sort();
        rendered_world.dedup();
        if rendered_world != expected {
            return Err(fail(format!(
                "siblings render world objects {rendered_world:?} but the session holds {expected:?}"
            )));
        }
        // World placements only: a def-owned placement of the open
        // definition inside another definition renders through THAT
        // definition's world instances, which `placing_instances` lists.
        for i in doc.instances_of(def) {
            if !doc.instance_is_world(i) {
                continue;
            }
            if !placements.contains(&i) {
                return Err(fail(format!(
                    "live sibling {i:?} missing from session_placements"
                )));
            }
        }
    }
    // Every live group lists only live members (the invariant an emptied
    // close with a grouped sibling once broke).
    let groups = doc.group_ids();
    for &g in &groups {
        for m in doc.group_members(g).unwrap_or_default() {
            let live = match m {
                NodeId::Object(o) => live_objects.contains(&o),
                NodeId::Group(gg) => groups.contains(&gg),
                NodeId::Instance(i) => live_instances.contains(&i),
            };
            if !live {
                return Err(fail(format!("group {g:?} lists non-live member {m:?}")));
            }
        }
    }
    Ok(())
}

fn run_case(boxes: usize, ops: Vec<Op>) -> Result<(), TestCaseError> {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    for i in 0..boxes {
        add_box(&mut doc, 3.0 * i as f64);
    }
    for (step, op) in ops.iter().enumerate() {
        apply(&mut doc, op);
        check_reopens(&doc, step, op)?;
        check_sibling_render(&doc, step, op)?;
    }
    // Close whatever is still open: the close itself is an op that can
    // corrupt the tree, and the result must reopen too.
    while !doc.session_stack().is_empty() {
        let before = doc.session_stack().len();
        let _ = doc.close_innermost_session();
        if doc.session_stack().len() >= before {
            break;
        }
        check_reopens(&doc, ops.len(), &Op::CloseSession)?;
    }
    Ok(())
}

proptest! {
    /// Random tree-shaping and session sequences never leave a document the
    /// loader refuses, and never trip the validator.
    #[test]
    fn structure_and_session_sequences_stay_reopenable(
        boxes in 3usize..6,
        ops in proptest::collection::vec(arb_op(), 1..40),
    ) {
        run_case(boxes, ops)?;
    }
}

/// KB-41 (nightly fuzz): with a component, a group inside it, and a
/// component inside that group all open, the
/// innermost component's siblings render the OUTER frame's world objects through
/// the nested copy it holds, which the check did not expect.
#[test]
fn nested_component_group_component_renders_the_outer_frame_through_the_nested_copy() {
    let ops = vec![
        Op::Duplicate { sel: 0 },
        Op::Group {
            a: 772239617138806239,
            b: 9941832662847844180,
        },
        Op::MakeComponent {
            sel: 15742533718512779,
        },
        Op::MakeComponent {
            sel: 6736377549977266289,
        },
        Op::Explode { sel: 0 },
        Op::Undo,
        Op::Place {
            comp: 6095189737473883388,
        },
        Op::OpenComponentSession {
            sel: 2515554890444926613,
        },
        Op::OpenGroupSession { sel: 0 },
        Op::OpenComponentSession { sel: 0 },
    ];
    run_case(4, ops).unwrap();
}

/// KB-41's first sighting (nightly fuzz): the same shape with a group
/// session closed earlier in the sequence.
#[test]
fn nested_component_group_component_after_a_closed_group_session() {
    let ops = vec![
        Op::Group {
            a: 26031354693265861,
            b: 5039752818948471677,
        },
        Op::OpenGroupSession { sel: 0 },
        Op::Delete { sel: 0 },
        Op::CloseSession,
        Op::MakeComponent {
            sel: 3669795555711896687,
        },
        Op::Place { comp: 0 },
        Op::MakeComponent {
            sel: 5298141792758286970,
        },
        Op::OpenComponentSession {
            sel: 589821356543176086,
        },
        Op::OpenGroupSession { sel: 0 },
        Op::OpenComponentSession { sel: 0 },
    ];
    run_case(3, ops).unwrap();
}

/// The nested copy sits INSIDE a surfaced member group of the innermost
/// component: the copy is not a top-level session node, but a sibling of
/// the inner component still renders the outer frame through it.
#[test]
fn nested_copy_inside_a_surfaced_group_renders_the_outer_frame() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let b1 = add_box(&mut doc, 0.0);
    let b2 = add_box(&mut doc, 3.0);
    let b3 = add_box(&mut doc, 6.0);
    // A = { G1 { b1, b2 } }, placed as a1.
    let (g1, _) = doc
        .group_nodes(&[NodeId::Object(b1), NodeId::Object(b2)])
        .expect("group");
    let (comp_a, a1, _) = doc
        .make_component(&[NodeId::Group(g1)])
        .expect("component A");
    // B = { G2 { a1, b3 } }, placed as b_inst.
    let (g2, _) = doc
        .group_nodes(&[NodeId::Instance(a1), NodeId::Object(b3)])
        .expect("group 2");
    let (_comp_b, b_inst, _) = doc
        .make_component(&[NodeId::Group(g2)])
        .expect("component B");
    // A second copy of A to edit; the check compares B's definition
    // rendering against the session, so B needs no sibling of its own.
    let (a2, _) = doc
        .place_instance(comp_a, Transform::translation(Vec3::new(0.0, 10.0, 0.0)))
        .expect("place A");
    doc.open_explode_session(a2).expect("open A");
    doc.open_group_session(g1).expect("open G1");
    doc.open_explode_session(b_inst).expect("open B");
    check_reopens(&doc, 0, &Op::CloseSession).unwrap();
    check_sibling_render(&doc, 0, &Op::CloseSession).unwrap();
}
