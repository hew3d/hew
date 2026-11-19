//! The fuzz harnesses' lock oracle (OW-1), independent of the kernel's own
//! commit audit. Across a forward step, a node frozen before it:
//!
//! - is still there: a frozen node is never deleted. Two exceptions, both
//!   a linked set going away with its members: a Make Unique that took the
//!   set's last copy (the lock rides on the new shape's copies), and a
//!   step that left the set one copy, which is made plain again. There the
//!   member comes out as a new node, and one that is frozen with the same
//!   name, tags and vertex count must be there in its place;
//! - is still frozen, unless the step was an unlock: nothing moves a node
//!   out of its lock (out of a locked group, or out of a locked shape);
//! - keeps its geometry where it stands (in its own frame, through
//!   placements' poses), its name and its tags.
//!
//! Record equality is the wrong test: a linked copy of a locked node
//! re-owns it into a new definition (its lock moves onto the shape)
//! without changing any of these.

#![allow(dead_code)]

use kernel::{ComponentId, Document, NodeId, Transform};

/// What a lock keeps on one node.
pub type Kept = (Vec<[i64; 3]>, Option<String>, Vec<Vec<String>>);

/// One node frozen before a step: what it must keep, and the definition
/// owning it (an Object member), for the Make Unique exception.
#[derive(Debug, Clone)]
pub struct Frozen {
    pub node: NodeId,
    pub kept: Kept,
    pub owner: Option<ComponentId>,
}

/// What kind of step ran, for the two transitions a step may make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Any other forward step.
    Edit,
    /// An unlock (or a lock op that may unlock): a node may stop being
    /// frozen.
    Unlock,
    /// A Make Unique (or Unlink): it promotes a placement's own name onto
    /// its new shape, and may take an emptied linked set with it.
    MakeUnique,
}

/// Every live node a harness can name: every live Object (definition
/// members included), and the world groups and placements.
pub fn all_live_nodes(doc: &Document) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = doc
        .live_object_ids()
        .into_iter()
        .map(NodeId::Object)
        .collect();
    v.extend(doc.group_ids().into_iter().map(NodeId::Group));
    v.extend(doc.instance_ids().into_iter().map(NodeId::Instance));
    v
}

/// Whether `node` is live (not deleted, not a tombstone), wherever it is
/// owned now.
pub fn live(doc: &Document, node: NodeId) -> bool {
    match node {
        NodeId::Object(o) => doc.object(o).is_some(),
        NodeId::Group(g) => doc.group_members(g).is_some(),
        NodeId::Instance(i) => doc.instance_def(i).is_some(),
    }
}

/// Every vertex `node` renders in its own frame, quantized and sorted.
pub fn points(doc: &Document, node: NodeId) -> Vec<[i64; 3]> {
    let mut out = Vec::new();
    let mut stack = vec![(node, Transform::IDENTITY)];
    while let Some((n, xf)) = stack.pop() {
        match n {
            NodeId::Object(o) => {
                if let Some(object) = doc.object(o) {
                    for v in object.vertices().values() {
                        let p = xf.apply_point(v.position);
                        out.push([
                            (p.x * 1e6).round() as i64,
                            (p.y * 1e6).round() as i64,
                            (p.z * 1e6).round() as i64,
                        ]);
                    }
                }
            }
            NodeId::Group(g) => {
                for m in doc.group_members(g).unwrap_or_default() {
                    stack.push((m, xf));
                }
            }
            NodeId::Instance(i) => {
                if let (Some(pose), Some(def)) = (doc.instance_pose(i), doc.instance_def(i)) {
                    for m in doc.def_member_nodes(def).unwrap_or_default() {
                        stack.push((m, pose.then(&xf)));
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out
}

pub fn kept(doc: &Document, n: NodeId) -> Kept {
    let name = match n {
        NodeId::Object(o) => doc.object_name(o),
        NodeId::Group(g) => doc.group_name(g),
        NodeId::Instance(i) => doc.instance_name(i),
    };
    (
        points(doc, n),
        name.map(str::to_string),
        doc.node_tags(n).to_vec(),
    )
}

/// The frozen nodes and what each must keep.
pub fn frozen_snapshot(doc: &Document) -> Vec<Frozen> {
    all_live_nodes(doc)
        .into_iter()
        .filter(|&n| doc.node_frozen(n).is_some())
        .map(|node| Frozen {
            node,
            kept: kept(doc, node),
            owner: match node {
                NodeId::Object(o) => doc.object_owner_component(o),
                _ => None,
            },
        })
        .collect()
}

/// Checks every node of `before` against `doc` after one forward step of
/// kind `step` (see the module comment).
pub fn check_frozen_kept(doc: &Document, before: &[Frozen], step: StepKind) -> Result<(), String> {
    for f in before {
        let n = f.node;
        if !live(doc, n) {
            // A Make Unique that emptied a linked set hid the set with its
            // members; nothing else removes a frozen node.
            let set_went = step == StepKind::MakeUnique
                && f.owner.is_some_and(|d| doc.def_member_nodes(d).is_none());
            if set_went {
                continue;
            }
            // The set's last copy was made plain: the lock is carried onto
            // the node the member became.
            let baked = f.owner.is_some_and(|d| doc.def_member_nodes(d).is_none())
                && all_live_nodes(doc).into_iter().any(|m| {
                    doc.node_frozen(m).is_some() && {
                        let now = kept(doc, m);
                        now.0.len() == f.kept.0.len() && now.1 == f.kept.1 && now.2 == f.kept.2
                    }
                });
            if baked {
                continue;
            }
            return Err(format!("{n:?} was deleted while frozen"));
        }
        if doc.node_frozen(n).is_none() {
            if step == StepKind::Unlock {
                continue;
            }
            return Err(format!("{n:?} was moved out of its lock"));
        }
        let now = kept(doc, n);
        if now.0 != f.kept.0 {
            return Err(format!("{n:?} moved or changed shape while frozen"));
        }
        if now.2 != f.kept.2 {
            return Err(format!("{n:?} was retagged while frozen"));
        }
        if now.1 != f.kept.1 && !(step == StepKind::MakeUnique && matches!(n, NodeId::Instance(_)))
        {
            return Err(format!("{n:?} was renamed while frozen"));
        }
    }
    Ok(())
}

/// The oracle's own specs: it must see a frozen node removed or moved out
/// of its lock. The kernel refuses both, so each hole is made the only way
/// the public API allows: the snapshot is taken locked, and the document
/// it is checked against reached the removal through an unlock the check
/// is not told about, as one unguarded step would.
mod self_test {
    use super::*;
    use kernel::{Plane, Point3};

    fn board(doc: &mut Document, x: f64) -> NodeId {
        let ground = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .expect("ground plane");
        let s = doc.add_sketch(ground);
        let c = [
            Point3::new(x, 0.0, 0.0),
            Point3::new(x + 1.0, 0.0, 0.0),
            Point3::new(x + 1.0, 1.0, 0.0),
            Point3::new(x, 1.0, 0.0),
        ];
        let sk = doc.sketch_mut(s).expect("sketch is live");
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
        let region = doc.extrudable_regions(s).expect("sketch is live")[0];
        NodeId::Object(doc.extrude_region(s, region, 1.0).expect("extrude").0)
    }

    #[test]
    fn the_oracle_sees_a_deleted_frozen_node() {
        let mut doc = Document::new();
        let n = board(&mut doc, 0.0);
        doc.set_nodes_locked(&[n], true).unwrap();
        let before = frozen_snapshot(&doc);
        doc.set_nodes_locked(&[n], false).unwrap();
        doc.delete_node(n).unwrap();
        for step in [StepKind::Edit, StepKind::Unlock, StepKind::MakeUnique] {
            let err = check_frozen_kept(&doc, &before, step).unwrap_err();
            assert!(err.contains("deleted"), "{err}");
        }
    }

    #[test]
    fn the_oracle_sees_a_node_moved_out_of_its_lock() {
        let mut doc = Document::new();
        let n = board(&mut doc, 0.0);
        let m = board(&mut doc, 3.0);
        let (g, _) = doc.group_nodes(&[n, m]).unwrap();
        doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
        let before = frozen_snapshot(&doc);
        assert!(before.iter().any(|f| f.node == n));
        doc.set_nodes_locked(&[NodeId::Group(g)], false).unwrap();
        doc.reparent_nodes(&[n], None).unwrap();
        let err = check_frozen_kept(&doc, &before, StepKind::Edit).unwrap_err();
        assert!(err.contains("moved out of its lock"), "{err}");
        // An unlock is the one step after which a node may be free.
        check_frozen_kept(&doc, &before, StepKind::Unlock).unwrap();
    }
}
