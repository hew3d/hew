//! Occurrences: every placement in the model with its world pose
//! (docs/agents/PLUGINS.md "Reading parts"; docs/agents/HEW_API.md §7,
//! `hew.query.occurrences`).
//!
//! The render walks ([`Document::expanded_placements`] and friends) answer
//! "which leaf objects draw where". This walk answers "which placements
//! exist": every object, group, and instance reachable from the world,
//! nested definitions expanded through each instance that places them,
//! each row named by its instance path. It is read-only and iterative (an
//! explicit stack, so a deep model cannot overflow).
//!
//! **Open editing sessions.** The walk reports the model as the kernel
//! holds it. A group session's group is a hidden row (its members are
//! world rows already, so nothing is listed under it). An explode session's
//! entered instance and its siblings are hidden rows that still expand from
//! the definition record, at the pose each placement would have: the
//! definition's members hold world-baked coordinates while the session is
//! open, so entering a definition on loan composes the session's inverse
//! pose first. `include_hidden: false` is the as-displayed view.

use std::collections::BTreeSet;

use crate::ids::{ComponentId, GroupId, InstanceId, ObjectId};
use crate::material::FaceMaterial;
use crate::math::Point3;
use crate::transform::Transform;

use super::{Document, DocumentError, MAX_COMPONENT_DEPTH, NodeId, ObjectOwner, SessionFrame};

/// The most rows one [`Document::occurrences`] call returns, counted across
/// every root. Past it the walk refuses [`DocumentError::TooManyOccurrences`]
/// rather than hand back a truncated list.
pub const MAX_OCCURRENCES: usize = 200_000;

/// One placement of a node: where it sits in the model and its world pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    /// Instance ids from the world down to (not including) `node`; empty for
    /// a world node.
    pub path: Vec<InstanceId>,
    pub node: NodeId,
    /// World pose of `node`'s own frame: composed through `path`, then the
    /// instance's own pose when `node` is an instance. Objects and groups
    /// carry the composed path pose (their geometry is already in their
    /// owner's frame).
    pub pose: Transform,
    /// The definition an instance places; `None` for objects and groups.
    pub definition: Option<ComponentId>,
    /// The definition that owns `node` in this placement; `None` for a
    /// world node.
    pub owner_def: Option<ComponentId>,
    /// The containing group within the owner's tree, if any.
    pub parent_group: Option<GroupId>,
    /// `path.len()`.
    pub depth: usize,
    /// Effective visibility: the node is effectively hidden
    /// ([`Document::node_effectively_hidden`]), hidden by an open editing
    /// session, or any instance on `path` is.
    pub hidden: bool,
    /// An instance's own material; `None` for objects and groups.
    pub material: FaceMaterial,
    /// The material the node's unpainted faces show, before any face's
    /// own paint: for an object its base, else what it inherits along
    /// `path` ([`Document::inherited_material`]); for an instance its own,
    /// else what it inherits; `None` for a group.
    pub material_effective: FaceMaterial,
}

/// What a render or export draws: one placement and everything under it,
/// or a definition's leaves at the identity pose (a definition has no
/// placement of its own).
#[derive(Debug, Clone, PartialEq)]
pub enum RenderRoot {
    Occurrence(Occurrence),
    Definition(ComponentId),
}

/// One pending node in the walk.
struct Item {
    path: Vec<InstanceId>,
    node: NodeId,
    /// The pose of the frame `node` is expressed in.
    base: Transform,
    owner_def: Option<ComponentId>,
    parent: Option<GroupId>,
    path_hidden: bool,
}

/// The nodes an open session hides without tombstoning.
struct SessionHidden {
    instances: BTreeSet<InstanceId>,
    groups: BTreeSet<GroupId>,
}

impl Document {
    fn session_hidden(&self) -> SessionHidden {
        let mut instances = BTreeSet::new();
        let mut groups = BTreeSet::new();
        for frame in &self.sessions {
            match frame {
                SessionFrame::Component(s) => {
                    instances.extend(
                        s.hidden_instances
                            .iter()
                            .copied()
                            .filter(|&i| self.instances.contains_key(i)),
                    );
                }
                SessionFrame::Group(g) => {
                    if self.groups.contains_key(g.group) {
                        groups.insert(g.group);
                    }
                }
            }
        }
        SessionHidden { instances, groups }
    }

    /// The frame pose for the members of `def` when the definition's own
    /// frame sits at `pose`: `pose` itself, or — for a definition on loan
    /// to an open component session, whose members hold world-baked
    /// coordinates — the session's inverse pose composed first.
    fn member_base(&self, def: ComponentId, pose: &Transform) -> Transform {
        for frame in &self.sessions {
            if let SessionFrame::Component(s) = frame
                && s.component == def
            {
                return s.pose_inv.then(pose);
            }
        }
        *pose
    }

    /// Whether `node` has a record the walk lists: live, or hidden only by
    /// an open session.
    fn occurrence_present(&self, node: NodeId, sh: &SessionHidden) -> bool {
        match node {
            NodeId::Object(o) => self.objects.get(o).is_some_and(|r| !r.hidden),
            NodeId::Group(g) => self
                .groups
                .get(g)
                .is_some_and(|r| !r.hidden || sh.groups.contains(&g)),
            NodeId::Instance(i) => self
                .instances
                .get(i)
                .is_some_and(|r| !r.hidden || sh.instances.contains(&i)),
        }
    }

    /// The row for one walk item, or `None` when the node has no listable
    /// record.
    fn occurrence_row(&self, item: &Item, sh: &SessionHidden) -> Option<Occurrence> {
        if !self.occurrence_present(item.node, sh) {
            return None;
        }
        let (pose, definition, session_hidden) = match item.node {
            NodeId::Instance(i) => {
                let rec = &self.instances[i];
                (
                    rec.pose.then(&item.base),
                    Some(rec.def),
                    sh.instances.contains(&i),
                )
            }
            NodeId::Group(g) => (item.base, None, sh.groups.contains(&g)),
            NodeId::Object(_) => (item.base, None, false),
        };
        let hidden = item.path_hidden || session_hidden || self.node_effectively_hidden(item.node);
        let (material, material_effective) = match item.node {
            NodeId::Instance(i) => {
                let own = self.instances[i].material;
                (own, own.or_else(|| self.inherited_material(&item.path)))
            }
            NodeId::Object(o) => (
                None,
                self.objects[o]
                    .object
                    .default_material()
                    .or_else(|| self.inherited_material(&item.path)),
            ),
            NodeId::Group(_) => (None, None),
        };
        Some(Occurrence {
            material,
            material_effective,
            depth: item.path.len(),
            path: item.path.clone(),
            node: item.node,
            pose,
            definition,
            owner_def: item.owner_def,
            parent_group: item.parent,
            hidden,
        })
    }

    /// Pushes `row`'s children (in recorded order, reversed for the stack).
    /// `path_hidden` is the row's own item's: whether an instance on its
    /// path is hidden (a group's members re-derive the group's own state).
    fn occurrence_children(
        &self,
        row: &Occurrence,
        path_hidden: bool,
        sh: &SessionHidden,
        stack: &mut Vec<Item>,
    ) {
        match row.node {
            NodeId::Object(_) => {}
            NodeId::Group(g) => {
                // A session-hidden group's members are world rows already.
                if sh.groups.contains(&g) {
                    return;
                }
                for &m in self.groups[g].members.iter().rev() {
                    stack.push(Item {
                        path: row.path.clone(),
                        node: m,
                        base: row.pose,
                        owner_def: row.owner_def,
                        parent: Some(g),
                        path_hidden,
                    });
                }
            }
            NodeId::Instance(i) => {
                if row.path.len() + 1 > MAX_COMPONENT_DEPTH {
                    return;
                }
                let def = self.instances[i].def;
                let Some(c) = self.components.get(def).filter(|c| !c.hidden) else {
                    return;
                };
                let base = self.member_base(def, &row.pose);
                let mut path = row.path.clone();
                path.push(i);
                for &m in c.members.iter().rev() {
                    stack.push(Item {
                        path: path.clone(),
                        node: m,
                        base,
                        owner_def: Some(def),
                        parent: None,
                        path_hidden: row.hidden,
                    });
                }
            }
        }
    }

    /// The world's top-level nodes as the walk lists them: world objects,
    /// then groups, then instances, each in slotmap order — the order of
    /// [`Document::top_level_nodes`], plus the top-level nodes an open
    /// session hides.
    fn occurrence_world_roots(&self, sh: &SessionHidden) -> Vec<NodeId> {
        let objects = self
            .objects
            .iter()
            .filter(|(_, r)| !r.hidden && r.is_world() && r.group_parent().is_none())
            .map(|(id, _)| NodeId::Object(id));
        let groups = self
            .groups
            .iter()
            .filter(|(id, r)| {
                (!r.hidden || sh.groups.contains(id)) && r.parent.is_none() && r.owner_def.is_none()
            })
            .map(|(id, _)| NodeId::Group(id));
        let instances = self
            .instances
            .iter()
            .filter(|(id, r)| {
                (!r.hidden || sh.instances.contains(id))
                    && r.parent.is_none()
                    && r.owner_def.is_none()
            })
            .map(|(id, _)| NodeId::Instance(id));
        objects.chain(groups).chain(instances).collect()
    }

    /// Every placement reachable from `roots` (`None` = the whole world),
    /// depth-first, members in their recorded order, nested definitions
    /// expanded through every instance that places them, capped at
    /// [`MAX_COMPONENT_DEPTH`] and [`MAX_OCCURRENCES`].
    ///
    /// `roots`: each root is `(path, node)` as [`Document::occurrence_at`]
    /// resolves it; the walk visits roots in the given order, emits each
    /// root's own row and then its subtree depth-first, and skips a
    /// `(path, node)` already emitted (one root inside another lists
    /// nothing twice). The caps count the total across roots.
    ///
    /// `include_hidden: false` skips hidden occurrences AND does not
    /// descend into a hidden instance or group. Definitions on loan to an
    /// open session are expanded from their definition record (the render
    /// walks skip them; a query must still answer), at the pose each
    /// placement would have — see the module doc.
    ///
    /// # Errors
    /// - [`DocumentError::TooManyOccurrences`] — the walk would list more
    ///   than [`MAX_OCCURRENCES`] rows.
    /// - [`DocumentError::UnknownInstance`] — a root does not resolve
    ///   ([`Document::occurrence_at`] answers `None`).
    pub fn occurrences(
        &self,
        roots: Option<&[(Vec<InstanceId>, NodeId)]>,
        include_hidden: bool,
    ) -> Result<Vec<Occurrence>, DocumentError> {
        let sh = self.session_hidden();
        let mut starts: Vec<Item> = Vec::new();
        match roots {
            None => {
                for node in self.occurrence_world_roots(&sh) {
                    starts.push(Item {
                        path: Vec::new(),
                        node,
                        base: Transform::IDENTITY,
                        owner_def: None,
                        parent: self.raw_parent(node),
                        path_hidden: false,
                    });
                }
            }
            Some(list) => {
                for (path, node) in list {
                    let item = self
                        .occurrence_item(path, *node, &sh)
                        .ok_or(DocumentError::UnknownInstance)?;
                    starts.push(item);
                }
            }
        }

        let mut out: Vec<Occurrence> = Vec::new();
        let mut seen: BTreeSet<(Vec<InstanceId>, NodeId)> = BTreeSet::new();
        for start in starts.into_iter() {
            let mut stack = vec![start];
            while let Some(item) = stack.pop() {
                let Some(row) = self.occurrence_row(&item, &sh) else {
                    continue;
                };
                if row.hidden && !include_hidden {
                    continue;
                }
                if !seen.insert((row.path.clone(), row.node)) {
                    continue;
                }
                if out.len() >= MAX_OCCURRENCES {
                    return Err(DocumentError::TooManyOccurrences);
                }
                self.occurrence_children(&row, item.path_hidden, &sh, &mut stack);
                out.push(row);
            }
        }
        Ok(out)
    }

    /// A node's containing group as recorded, live or not.
    fn raw_parent(&self, node: NodeId) -> Option<GroupId> {
        match node {
            NodeId::Object(o) => self.objects.get(o).and_then(|r| r.group_parent()),
            NodeId::Group(g) => self.groups.get(g).and_then(|r| r.parent),
            NodeId::Instance(i) => self.instances.get(i).and_then(|r| r.parent),
        }
    }

    /// Finds `target` among `def`'s members (member groups descended):
    /// `Some(parent group)` when found (`Some(None)` for a direct member).
    fn find_in_def(
        &self,
        def: ComponentId,
        target: NodeId,
        sh: &SessionHidden,
    ) -> Option<Option<GroupId>> {
        let c = self.components.get(def).filter(|c| !c.hidden)?;
        let mut stack: Vec<(NodeId, Option<GroupId>)> =
            c.members.iter().rev().map(|&m| (m, None)).collect();
        while let Some((node, parent)) = stack.pop() {
            if !self.occurrence_present(node, sh) {
                continue;
            }
            if node == target {
                return Some(parent);
            }
            if let NodeId::Group(g) = node
                && !sh.groups.contains(&g)
            {
                for &m in self.groups[g].members.iter().rev() {
                    stack.push((m, Some(g)));
                }
            }
        }
        None
    }

    /// Whether `node` is a world node (not owned by a definition) the walk
    /// lists.
    fn is_world_node(&self, node: NodeId, sh: &SessionHidden) -> bool {
        if !self.occurrence_present(node, sh) {
            return false;
        }
        match node {
            NodeId::Object(o) => matches!(self.objects[o].owner, ObjectOwner::World { .. }),
            NodeId::Group(g) => self.groups[g].owner_def.is_none(),
            NodeId::Instance(i) => self.instances[i].owner_def.is_none(),
        }
    }

    /// Resolves a path: the first instance must be a world instance; each
    /// later one a member (directly or inside nested groups) of the previous
    /// instance's definition; `leaf` a member of the last instance's
    /// definition, or a world node for an empty path. Returns the matching
    /// [`Occurrence`] (the row [`Document::occurrences`] lists for it), or
    /// `None`.
    pub fn occurrence_at(&self, path: &[InstanceId], leaf: NodeId) -> Option<Occurrence> {
        let sh = self.session_hidden();
        let item = self.occurrence_item(path, leaf, &sh)?;
        self.occurrence_row(&item, &sh)
    }

    /// The walk item [`Document::occurrence_at`] resolves to.
    fn occurrence_item(
        &self,
        path: &[InstanceId],
        leaf: NodeId,
        sh: &SessionHidden,
    ) -> Option<Item> {
        if path.len() > MAX_COMPONENT_DEPTH {
            return None;
        }
        let mut base = Transform::IDENTITY;
        let mut owner: Option<ComponentId> = None;
        let mut path_hidden = false;
        let steps = path
            .iter()
            .map(|&i| NodeId::Instance(i))
            .chain(std::iter::once(leaf));
        let mut parent: Option<GroupId> = None;
        for (k, node) in steps.enumerate() {
            parent = match owner {
                None => {
                    if !self.is_world_node(node, sh) {
                        return None;
                    }
                    self.raw_parent(node)
                }
                Some(def) => self.find_in_def(def, node, sh)?,
            };
            if k == path.len() {
                break;
            }
            let NodeId::Instance(i) = node else {
                return None;
            };
            let rec = &self.instances[i];
            let pose = rec.pose.then(&base);
            path_hidden =
                path_hidden || sh.instances.contains(&i) || self.node_effectively_hidden(node);
            base = self.member_base(rec.def, &pose);
            owner = Some(rec.def);
        }
        // The leaf itself must be listable.
        if !self.occurrence_present(leaf, sh) {
            return None;
        }
        Some(Item {
            path: path.to_vec(),
            node: leaf,
            base,
            owner_def: owner,
            parent,
            path_hidden,
        })
    }

    /// Every leaf object placement under one render root, with its world
    /// pose: the root's own subtree for an occurrence (hidden rows
    /// included when `include_hidden`), or a definition's leaves at the
    /// identity pose. Each entry is `(object, world pose, the occurrence's
    /// path)`.
    pub fn render_root_leaves(
        &self,
        root: &RenderRoot,
        include_hidden: bool,
    ) -> Result<Vec<(ObjectId, Transform, Vec<InstanceId>)>, DocumentError> {
        let rows = match root {
            RenderRoot::Occurrence(occ) => {
                self.occurrences(Some(&[(occ.path.clone(), occ.node)]), include_hidden)?
            }
            RenderRoot::Definition(def) => self.definition_rows(*def, include_hidden)?,
        };
        Ok(rows
            .into_iter()
            .filter_map(|r| match r.node {
                NodeId::Object(o) => Some((o, r.pose, r.path)),
                _ => None,
            })
            .collect())
    }

    /// The rows of `def`'s own members, expanded, with the definition's
    /// frame at identity. Paths start at the member instances (the
    /// definition itself has no placement).
    fn definition_rows(
        &self,
        def: ComponentId,
        include_hidden: bool,
    ) -> Result<Vec<Occurrence>, DocumentError> {
        let sh = self.session_hidden();
        let Some(c) = self.components.get(def).filter(|c| !c.hidden) else {
            return Ok(Vec::new());
        };
        let base = self.member_base(def, &Transform::IDENTITY);
        let mut stack: Vec<Item> = c
            .members
            .iter()
            .rev()
            .map(|&m| Item {
                path: Vec::new(),
                node: m,
                base,
                owner_def: Some(def),
                parent: None,
                path_hidden: false,
            })
            .collect();
        let mut out = Vec::new();
        while let Some(item) = stack.pop() {
            let Some(row) = self.occurrence_row(&item, &sh) else {
                continue;
            };
            if row.hidden && !include_hidden {
                continue;
            }
            if out.len() >= MAX_OCCURRENCES {
                return Err(DocumentError::TooManyOccurrences);
            }
            self.occurrence_children(&row, item.path_hidden, &sh, &mut stack);
            out.push(row);
        }
        Ok(out)
    }

    /// Definition-local AABB of every leaf placement of `def` (nested
    /// included), or `None` if it has none (or its walk is past
    /// [`MAX_OCCURRENCES`]; [`Document::definition_bounds_checked`] says
    /// which). Agrees with [`Document::expanded_def_placements`] outside
    /// sessions; a definition on loan to an open session still answers its
    /// definition-local box.
    pub fn definition_bounds(&self, def: ComponentId) -> Option<(Point3, Point3)> {
        self.definition_bounds_checked(def).ok().flatten()
    }

    /// [`Document::definition_bounds`], with the walk's cap reported.
    ///
    /// # Errors
    /// [`DocumentError::TooManyOccurrences`] — the definition expands to
    /// more than [`MAX_OCCURRENCES`] rows.
    pub fn definition_bounds_checked(
        &self,
        def: ComponentId,
    ) -> Result<Option<(Point3, Point3)>, DocumentError> {
        let rows = self.definition_rows(def, true)?;
        let mut acc: Option<(Point3, Point3)> = None;
        for r in rows {
            let NodeId::Object(o) = r.node else {
                continue;
            };
            let Some(obj) = self.object(o) else {
                continue;
            };
            for v in obj.vertices().values() {
                let p = r.pose.apply_point(v.position);
                acc = Some(match acc {
                    None => (p, p),
                    Some((lo, hi)) => (
                        Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                        Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
                    ),
                });
            }
        }
        Ok(acc)
    }
}
