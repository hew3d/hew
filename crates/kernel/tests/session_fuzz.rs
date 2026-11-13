//! Structure-and-session fuzz harness (DEVELOPMENT.md rule 3): random
//! sequences of the tree-shaping ops — make component, group, ungroup,
//! reparent, explode, place, make unique (one instance, and by occurrence
//! path), duplicate, delete, delete definition, transform and copy
//! definition members in place — interleaved with component and group EDITING SESSIONS
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
//! - with no session open, `load(save())` succeeds too;
//! - a make-unique-by-path fork keeps the occurrence walk's instance row
//!   count, and undo then redo of it reproduces the state hash both ways
//!   (a refused one leaves the document unchanged).
//!
//! Drawing joins in too (wireframe faces §11): lines in 3D between lattice
//! points (the boxes' corners and points above them), each drafted on a
//! plane through it as Line re-homes a slanted segment, Detached or not,
//! and moves and copies of free sketches — the edits that run the close
//! pass. For each,
//! one undo restores the document exactly and one redo returns the edit's
//! state. Whole axis-aligned rectangles between lattice points, and Make
//! Solid on a free sketch's faces, drive the cell search (§5): closed faces
//! become solids or grow the boxes, in the same step.
//!
//! Failures shrink to a short op list. Run deeper than the default 256
//! cases with `PROPTEST_CASES=20000 cargo test -p kernel --test
//! session_fuzz`.

use kernel::{
    Document, InstanceId, LibraryProvenance, NodeId, ObjectId, Plane, Point3, Transform, Vec3,
};
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
    /// Fork one placement by its occurrence path (any depth).
    MakeUniqueByPath {
        sel: usize,
        cascade: bool,
        deep: bool,
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
    /// Move a member of a definition in the definition's frame
    /// (`transform_def_members`).
    TransformDefMember {
        comp: usize,
        sel: usize,
    },
    /// Copy a member of a definition into the same definition, twice
    /// (`duplicate_def_members_array`).
    DuplicateDefMembers {
        comp: usize,
        sel: usize,
    },
    /// Delete every object inside the open component session, so the close
    /// empties the definition (the grouped-sibling emptied-close defect).
    DeleteSessionMembers,
    /// Wrap a copy of a node's content as an unplaced definition
    /// (`insert_as_definition`); `key` picks one of three provenances so
    /// repeats reuse.
    InsertAsDefinition {
        sel: usize,
        key: u8,
    },
    CloseSession,
    Undo,
    Redo,
    /// A line between two lattice points, drafted at the top level on a
    /// plane through it (`tilt` picks which), Detached or not (a detached
    /// line closes against a box's edges ending on its shape, §16 R8).
    Line3D {
        a: usize,
        b: usize,
        tilt: u8,
        detached: bool,
    },
    /// A translation of a whole free sketch by a lattice step.
    MoveSketch {
        sel: usize,
        step: u8,
    },
    /// A copy of every island of a free sketch, a lattice step away.
    CopySketch {
        sel: usize,
        step: u8,
    },
    /// An axis-aligned rectangle with opposite corners at two lattice
    /// points, drafted at the top level on its plane (`flip`: facing the
    /// other way).
    Rect3D {
        a: usize,
        b: usize,
        flip: bool,
    },
    /// Make Solid on every face of a free sketch ([`Document::close_cells`]).
    CloseCells {
        sel: usize,
    },
    /// A half-metre box drawn as faces, one draft per face (its bottom
    /// only on the ground): on the ground, or on a box's top (where the
    /// faces close against it and grow it), Detached or not.
    FaceBox {
        at: u8,
        detached: bool,
    },
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
        2 => (sel(), any::<bool>(), any::<bool>())
            .prop_map(|(sel, cascade, deep)| Op::MakeUniqueByPath { sel, cascade, deep }),
        1 => sel().prop_map(|sel| Op::Duplicate { sel }),
        2 => sel().prop_map(|sel| Op::Delete { sel }),
        1 => sel().prop_map(|sel| Op::DeleteSelection { sel }),
        1 => sel().prop_map(|comp| Op::DeleteDefinition { comp }),
        2 => (sel(), sel()).prop_map(|(comp, sel)| Op::TransformDefMember { comp, sel }),
        1 => (sel(), sel()).prop_map(|(comp, sel)| Op::DuplicateDefMembers { comp, sel }),
        3 => sel().prop_map(|sel| Op::OpenComponentSession { sel }),
        3 => sel().prop_map(|sel| Op::OpenGroupSession { sel }),
        2 => Just(Op::DeleteSessionMembers),
        2 => (sel(), 0u8..3).prop_map(|(sel, key)| Op::InsertAsDefinition { sel, key }),
        3 => Just(Op::CloseSession),
        2 => Just(Op::Undo),
        2 => Just(Op::Redo),
        6 => (sel(), sel(), 0u8..3, any::<bool>())
            .prop_map(|(a, b, tilt, detached)| Op::Line3D { a, b, tilt, detached }),
        2 => (sel(), 0u8..6).prop_map(|(sel, step)| Op::MoveSketch { sel, step }),
        1 => (sel(), 0u8..6).prop_map(|(sel, step)| Op::CopySketch { sel, step }),
        4 => (sel(), sel(), any::<bool>()).prop_map(|(a, b, flip)| Op::Rect3D { a, b, flip }),
        1 => sel().prop_map(|sel| Op::CloseCells { sel }),
        3 => (any::<u8>(), any::<bool>()).prop_map(|(at, detached)| Op::FaceBox { at, detached }),
    ]
}

/// The drawing lattice: the boxes' corners (x = 3i, 3i + 1; y = 0, 1;
/// z = 0, 1) and the points 1 m above their tops and halfway across.
fn lattice() -> Vec<Point3> {
    let mut v = Vec::new();
    for x in [0.0, 0.5, 1.0, 3.0, 3.5, 4.0] {
        for y in [0.0, 0.5, 1.0] {
            for z in [0.0, 1.0, 2.0] {
                v.push(Point3::new(x, y, z));
            }
        }
    }
    v
}

/// A lattice step: one unit along an axis, either way.
fn step_of(step: u8) -> Vec3 {
    match step % 6 {
        0 => Vec3::new(1.0, 0.0, 0.0),
        1 => Vec3::new(-1.0, 0.0, 0.0),
        2 => Vec3::new(0.0, 1.0, 0.0),
        3 => Vec3::new(0.0, -1.0, 0.0),
        4 => Vec3::new(0.0, 0.0, 1.0),
        _ => Vec3::new(0.0, 0.0, -1.0),
    }
}

/// A plane through `a–b`: the vertical one (`tilt` 0), the one holding the
/// x axis direction (1), or the y axis direction (2); for a line along that
/// direction, the plane facing +x through it.
fn home_plane(a: Point3, b: Point3, tilt: u8) -> Option<Plane> {
    let d = b - a;
    let other = match tilt % 3 {
        0 => Vec3::new(0.0, 0.0, 1.0),
        1 => Vec3::new(1.0, 0.0, 0.0),
        _ => Vec3::new(0.0, 1.0, 0.0),
    };
    let n = d.cross(other);
    let n = if n.length() > 1e-6 {
        n
    } else {
        d.cross(Vec3::new(1.0, 1.0, 0.0))
    };
    Plane::from_point_normal(a, n).ok()
}

/// The free world sketches a move or copy picks from.
fn free_sketches(doc: &Document) -> Vec<kernel::SketchId> {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch_owner_object(s).is_none())
        .collect()
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

/// Every visible instance placement as its occurrence path (instance ids
/// from the world down, the placement itself last).
fn instance_paths(doc: &Document) -> Vec<Vec<InstanceId>> {
    doc.occurrences(None, false)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|o| match o.node {
            NodeId::Instance(i) => {
                let mut path = o.path;
                path.push(i);
                Some(path)
            }
            _ => None,
        })
        .collect()
}

fn instance_rows(doc: &Document) -> usize {
    doc.occurrences(None, true)
        .unwrap_or_default()
        .iter()
        .filter(|o| matches!(o.node, NodeId::Instance(_)))
        .count()
}

/// `make_unique_by_path` on the path `sel` picks. A refusal is legal; a
/// fork must keep the placement count and undo then redo exactly.
fn make_unique_by_path(
    doc: &mut Document,
    sel: usize,
    cascade: bool,
    deep: bool,
) -> Result<(), String> {
    let paths = instance_paths(doc);
    let Some(path) = (!paths.is_empty()).then(|| paths[sel % paths.len()].clone()) else {
        return Ok(());
    };
    let rows = instance_rows(doc);
    let before = doc.state_hash();
    if doc.make_unique_by_path(&path, cascade, deep).is_err() {
        if doc.state_hash() != before {
            return Err("a refused fork changed the document".to_string());
        }
        return Ok(());
    }
    if instance_rows(doc) != rows {
        return Err(format!(
            "the fork changed the placement count from {rows} to {}",
            instance_rows(doc)
        ));
    }
    let after = doc.state_hash();
    doc.undo()
        .map_err(|e| format!("undo of the fork refused: {e}"))?;
    if doc.state_hash() != before {
        return Err("undo of the fork did not restore the document".to_string());
    }
    doc.redo()
        .map_err(|e| format!("redo of the fork refused: {e}"))?;
    if doc.state_hash() != after {
        return Err("redo of the fork did not reproduce it".to_string());
    }
    Ok(())
}

/// Undo then redo of the op just committed must land on `before` and then
/// `after` exactly.
fn undo_redo_exact(doc: &mut Document, before: u64, what: &str) -> Result<(), String> {
    let after = doc.state_hash();
    doc.undo()
        .map_err(|e| format!("undo of the {what} refused: {e}"))?;
    if doc.state_hash() != before {
        return Err(format!("undo of the {what} did not restore the document"));
    }
    doc.redo()
        .map_err(|e| format!("redo of the {what} refused: {e}"))?;
    if doc.state_hash() != after {
        return Err(format!("redo of the {what} did not reproduce it"));
    }
    Ok(())
}

thread_local! {
    /// The state hash before the last undo step an op recorded, when the
    /// op records several (a [`Op::FaceBox`] drafts one face at a time).
    static LAST_STEP_BEFORE: std::cell::Cell<Option<(u64, u64)>> =
        const { std::cell::Cell::new(None) };
}

/// Applies one op. A typed refusal is a legal answer and is ignored; what
/// must never happen is a panic or a document that no longer reopens, both
/// checked by the caller.
fn apply(doc: &mut Document, op: &Op) -> Result<(), String> {
    // Drawing waits until every editing session is closed: a loose line
    // drawn on a member's face inside a component session leaves an owned
    // sketch that the autosave does not fold with its owner, so the bytes
    // do not reopen. That defect predates the close pass (reproduced on
    // the base without it) and is tracked on its own; until it is fixed,
    // the drawing ops here exercise the close pass at the top level only.
    if matches!(
        op,
        Op::Line3D { .. }
            | Op::MoveSketch { .. }
            | Op::CopySketch { .. }
            | Op::Rect3D { .. }
            | Op::CloseCells { .. }
            | Op::FaceBox { .. }
    ) && !doc.session_stack().is_empty()
    {
        return Ok(());
    }
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
        Op::MakeUniqueByPath { sel, cascade, deep } => {
            return make_unique_by_path(doc, sel, cascade, deep);
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
        Op::TransformDefMember { comp, sel } => {
            if let Some(c) = pick(&comps, comp)
                && let Some(n) = pick(&doc.def_member_nodes(c).unwrap_or_default(), sel)
            {
                let t = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.25)
                    .expect("rotation")
                    .then(&Transform::translation(Vec3::new(0.3, -0.2, 0.1)));
                let _ = doc.transform_def_members(c, &[n], &t);
            }
        }
        Op::DuplicateDefMembers { comp, sel } => {
            if let Some(c) = pick(&comps, comp)
                && let Some(n) = pick(&doc.def_member_nodes(c).unwrap_or_default(), sel)
            {
                let step = Transform::translation(Vec3::new(0.0, 1.5, 0.0));
                let count = std::num::NonZeroU32::new(2).expect("nonzero");
                let _ = doc.duplicate_def_members_array(c, &[n], &step, count);
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
        Op::InsertAsDefinition { sel, key } => {
            let Some(n) = pick(&nodes, sel) else {
                return Ok(());
            };
            let Ok(item) = doc.extract_item(&[n], false) else {
                return Ok(());
            };
            let prov = LibraryProvenance {
                source_id: "import:fuzz".to_string(),
                content_hash: key.to_string(),
            };
            let before = doc.state_hash();
            match doc.insert_as_definition(&item, Some("Wrapped"), prov) {
                Err(_) if doc.state_hash() != before => {
                    return Err("a refused insert_as_definition changed the document".to_string());
                }
                Err(_) => {}
                Ok((_, true, _)) if doc.state_hash() != before => {
                    return Err("a reused definition changed the document".to_string());
                }
                Ok((_, true, _)) => {}
                Ok((_, false, _)) => return undo_redo_exact(doc, before, "definition insert"),
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
        Op::Line3D {
            a,
            b,
            tilt,
            detached,
        } => {
            let pts = lattice();
            let (a, b) = (pts[a % pts.len()], pts[b % pts.len()]);
            if a == b {
                return Ok(());
            }
            let Some(h) = home_plane(a, b, tilt) else {
                return Ok(());
            };
            let edit = |w: &mut kernel::Sketch| w.add_segment(a, b).map(|_| ());
            let _ = if detached {
                doc.draft_detached_new(h, None, "Line", edit)
            } else {
                let objects = doc.drawable_objects_on_plane(h, None);
                doc.draft(h, None, &objects, "Line", edit)
            };
        }
        Op::MoveSketch { sel, step } => {
            if let Some(s) = pick(&free_sketches(doc), sel) {
                let _ = doc.transform_sketch(s, &Transform::translation(step_of(step)));
            }
        }
        Op::Rect3D { a, b, flip } => {
            let pts = lattice();
            let (a, b) = (pts[a % pts.len()], pts[b % pts.len()]);
            let d = b - a;
            let flat = [d.x, d.y, d.z].iter().filter(|c| c.abs() < 1e-12).count();
            if flat != 1 {
                return Ok(());
            }
            let n = if d.x.abs() < 1e-12 {
                Vec3::new(1.0, 0.0, 0.0)
            } else if d.y.abs() < 1e-12 {
                Vec3::new(0.0, 1.0, 0.0)
            } else {
                Vec3::new(0.0, 0.0, 1.0)
            };
            let n = if flip { n * -1.0 } else { n };
            let Ok(h) = Plane::from_point_normal(a, n) else {
                return Ok(());
            };
            // The other two corners: each varies one of the two free axes.
            let (c, e) = if d.x.abs() < 1e-12 {
                (Point3::new(a.x, b.y, a.z), Point3::new(a.x, a.y, b.z))
            } else if d.y.abs() < 1e-12 {
                (Point3::new(b.x, a.y, a.z), Point3::new(a.x, a.y, b.z))
            } else {
                (Point3::new(b.x, a.y, a.z), Point3::new(a.x, b.y, a.z))
            };
            let objects = doc.drawable_objects_on_plane(h, None);
            let _ = doc.draft(h, None, &objects, "Rectangle", |w| {
                for (p, q) in [(a, c), (c, b), (b, e), (e, a)] {
                    w.add_segment(p, q)?;
                }
                Ok(())
            });
        }
        Op::FaceBox { at, detached } => {
            let x0 = [0.0, 0.5, 3.0, 3.5][usize::from(at % 4)];
            let y0 = [0.0, 0.5][usize::from(at / 4 % 2)];
            let z0 = [0.0, 1.0][usize::from(at / 8 % 2)];
            let c = [
                Point3::new(x0, y0, z0),
                Point3::new(x0 + 0.5, y0, z0),
                Point3::new(x0 + 0.5, y0 + 0.5, z0),
                Point3::new(x0, y0 + 0.5, z0),
            ];
            let up = |q: Point3| Point3::new(q.x, q.y, z0 + 0.5);
            let mut faces: Vec<(Vec3, [Point3; 4])> = Vec::new();
            if z0 == 0.0 {
                faces.push((Vec3::new(0.0, 0.0, 1.0), c));
            }
            for i in 0..4 {
                let (u, v) = (c[i], c[(i + 1) % 4]);
                faces.push((
                    (v - u).cross(Vec3::new(0.0, 0.0, 1.0)),
                    [u, v, up(v), up(u)],
                ));
            }
            // The order the faces go down in follows `at` too.
            let k = usize::from(at / 16) % faces.len();
            faces.rotate_left(k);
            for (n, q) in faces {
                let Ok(h) = Plane::from_point_normal(q[0], n) else {
                    return Ok(());
                };
                LAST_STEP_BEFORE.set(Some((doc.state_hash(), doc.history_generation())));
                let edit = |w: &mut kernel::Sketch| {
                    for i in 0..4 {
                        w.add_segment(q[i], q[(i + 1) % 4])?;
                    }
                    Ok(())
                };
                let _ = if detached {
                    doc.draft_detached_new(h, None, "Rectangle", edit)
                } else {
                    let objects = doc.drawable_objects_on_plane(h, None);
                    doc.draft(h, None, &objects, "Rectangle", edit)
                };
            }
        }
        Op::CloseCells { sel } => {
            if let Some(s) = pick(&free_sketches(doc), sel) {
                let scope = kernel::CellScope::default();
                let out = doc.close_cells(&[kernel::CellSeed::Sketch(s)], &scope);
                // Every solid made validates.
                for o in out {
                    if let kernel::CellOutcome::Created(id)
                    | kernel::CellOutcome::Grew { to: id, .. }
                    | kernel::CellOutcome::Unjoined { object: id, .. } = o
                    {
                        let obj = doc.object(id).expect("a made solid is visible");
                        assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
                        obj.validate().expect("a made solid validates");
                    }
                }
            }
        }
        Op::CopySketch { sel, step } => {
            if let Some(s) = pick(&free_sketches(doc), sel) {
                let islands: Vec<_> = doc
                    .sketch(s)
                    .map(|sk| sk.islands().keys().collect())
                    .unwrap_or_default();
                let _ =
                    doc.copy_sketch_islands(s, &islands, &Transform::translation(step_of(step)));
            }
        }
    }
    Ok(())
}

/// For a drawing op that recorded a step (lens 2 of the design's review):
/// one undo restores the document exactly, one redo returns the edit's
/// state.
fn check_one_step(
    doc: &mut Document,
    before: (u64, u64),
    step: usize,
    op: &Op,
) -> Result<(), TestCaseError> {
    if !matches!(
        op,
        Op::Line3D { .. }
            | Op::MoveSketch { .. }
            | Op::CopySketch { .. }
            | Op::Rect3D { .. }
            | Op::CloseCells { .. }
            | Op::FaceBox { .. }
    ) || doc.history_generation() == before.1
    {
        return Ok(());
    }
    let after = doc.state_hash();
    if let Err(e) = doc.undo() {
        return Err(TestCaseError::fail(format!(
            "step {step} ({op:?}): undo refused: {e}"
        )));
    }
    if doc.state_hash() != before.0 {
        return Err(TestCaseError::fail(format!(
            "step {step} ({op:?}): one undo does not restore the document"
        )));
    }
    if let Err(e) = doc.redo() {
        return Err(TestCaseError::fail(format!(
            "step {step} ({op:?}): redo refused: {e}"
        )));
    }
    if doc.state_hash() != after {
        return Err(TestCaseError::fail(format!(
            "step {step} ({op:?}): redo does not return the edit's state"
        )));
    }
    Ok(())
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
        let mut before = (doc.state_hash(), doc.history_generation());
        LAST_STEP_BEFORE.set(None);
        apply(&mut doc, op)
            .map_err(|e| TestCaseError::fail(format!("step {step} ({op:?}): {e}")))?;
        if let Some(last) = LAST_STEP_BEFORE.take() {
            before = last;
        }
        check_one_step(&mut doc, before, step, op)?;
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
