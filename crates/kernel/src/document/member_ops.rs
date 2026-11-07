//! Definition members edited through an occurrence path
//! (docs/agents/HEW_API.md §7, `hew.entity.move`/`rotate`/`scale` and the
//! `hew.solid.*` path forms; ARCHITECTURE.md §2.11).
//!
//! A path names one placement of a definition; the ops here change the
//! definition itself, so every placement follows. Transforms take `t` in the
//! definition's own frame (the bus conjugates a world gesture through
//! [`Document::path_pose`]); the boolean and slice through a placement take
//! world geometry and map it through the path's composed pose themselves.
//!
//! Every op is one undo entry on an existing action:
//! [`DocAction::TransformSelection`] for transforms, [`DocAction::DuplicatedArray`]
//! for copies, [`DocAction::Boolean`] and [`DocAction::Sliced`] for the
//! booleans and slices. Their undo and redo arms name the owning definition
//! and its placing instances, found from the records' owners.

use std::collections::BTreeMap;

use tracing::info;

use crate::history::History;
use crate::ids::{ComponentId, GroupId, InstanceId, ObjectId, SketchId};
use crate::math::Plane;
use crate::ops::BooleanOp;
use crate::sketch::Sketch;
use crate::topo::Object;
use crate::transform::Transform;

use super::{
    CreatedClone, DocAction, DocChange, Document, DocumentError, EntityRef, MAX_COMPONENT_DEPTH,
    NodeId, ObjectOwner, ObjectRecord, merge_unique,
};

/// The tool of a subtract through a placement: a world Object's record,
/// consumed, or a transient world solid no record holds.
enum PlacementTool<'a> {
    Record(ObjectId),
    Transient(&'a Object),
}

impl Document {
    /// Whether any definition a path passes through — the definition each
    /// listed instance places — is on loan to an open component session.
    /// Its members are world nodes for the session's duration, so a path
    /// edit through it refuses ([`DocumentError::ExplodeSessionScope`]).
    pub fn path_on_loan(&self, path: &[InstanceId]) -> bool {
        path.iter().any(|&i| {
            self.instances
                .get(i)
                .is_some_and(|r| self.frame_on_loan(r.def).is_some())
        })
    }

    /// Whether `node` is a live member of `component`: a direct member, or a
    /// node inside a member group at any depth (all of them carry the
    /// definition as their owner).
    fn is_live_def_member(&self, component: ComponentId, node: NodeId) -> bool {
        match node {
            NodeId::Object(o) => self.objects.get(o).is_some_and(|r| {
                !r.hidden
                    && matches!(r.owner, ObjectOwner::Definition { def, .. } if def == component)
            }),
            NodeId::Group(g) => self
                .groups
                .get(g)
                .is_some_and(|r| !r.hidden && r.owner_def == Some(component)),
            NodeId::Instance(i) => self
                .instances
                .get(i)
                .is_some_and(|r| !r.hidden && r.owner_def == Some(component)),
        }
    }

    /// The checks every member edit makes before touching anything:
    /// `component` live, not on loan, `nodes` non-empty and each a live
    /// member of it.
    fn check_def_members(
        &self,
        component: ComponentId,
        nodes: &[NodeId],
    ) -> Result<(), DocumentError> {
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return Err(DocumentError::UnknownComponent);
        }
        if self.frame_on_loan(component).is_some() {
            return Err(DocumentError::ExplodeSessionScope);
        }
        if nodes.is_empty() {
            return Err(DocumentError::EmptySelection);
        }
        for &node in nodes {
            if !self.is_live_def_member(component, node) {
                return Err(match node {
                    NodeId::Object(_) => DocumentError::UnknownObject,
                    NodeId::Group(_) => DocumentError::UnknownGroup,
                    NodeId::Instance(_) => DocumentError::UnknownInstance,
                });
            }
        }
        Ok(())
    }

    /// The visible member objects beneath `node` (the node itself for an
    /// object), through member groups, stopping at instances — the
    /// definition-side twin of [`Document::leaf_objects_under`], which lists
    /// world objects only.
    fn collect_member_leaves(&self, node: NodeId, out: &mut Vec<ObjectId>) {
        match node {
            NodeId::Object(id) => {
                if self.objects.get(id).is_some_and(|r| !r.hidden) {
                    out.push(id);
                }
            }
            NodeId::Group(id) => {
                if let Some(rec) = self.groups.get(id).filter(|r| !r.hidden) {
                    for &m in &rec.members {
                        self.collect_member_leaves(m, out);
                    }
                }
            }
            NodeId::Instance(_) => {}
        }
    }

    /// The listed members flattened the way
    /// [`Document::transform_selection`] flattens a selection: leaf objects,
    /// leaf instances, and every group at or beneath a listed node, each
    /// once, in first-listing order.
    fn flatten_def_members(
        &self,
        nodes: &[NodeId],
    ) -> (Vec<ObjectId>, Vec<InstanceId>, Vec<GroupId>) {
        let mut objects: Vec<ObjectId> = Vec::new();
        let mut instances: Vec<InstanceId> = Vec::new();
        let mut groups: Vec<GroupId> = Vec::new();
        for &node in nodes {
            let mut leaves = Vec::new();
            self.collect_member_leaves(node, &mut leaves);
            merge_unique(&mut objects, leaves);
            merge_unique(&mut instances, self.leaf_instances_under(node));
            let mut found = Vec::new();
            self.collect_groups(node, &mut found);
            merge_unique(&mut groups, found);
        }
        (objects, instances, groups)
    }

    /// Transform members of `component` by `t`, expressed in the
    /// definition's own frame, as ONE undoable step. Each listed node must
    /// be a live member of `component`: a direct member object or instance,
    /// or a node inside a member group at any depth. Flattening follows
    /// [`Document::transform_selection`]: `t` is baked into every leaf object
    /// beneath a listed node (through [`Object::apply_transform`], the
    /// object's own sketches and the drawing its member groups hold riding
    /// along) and composed into the pose of every leaf instance beneath it
    /// (`pose' = pose.then(t)`); a leaf reached twice is transformed once.
    /// Every placement of `component` updates: the change names the
    /// definition and every instance placing it.
    ///
    /// Recorded as [`DocAction::TransformSelection`]; undo restores the
    /// recorded snapshots and poses verbatim, redo re-applies `t` to them
    /// (DEVELOPMENT.md rule 9).
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — stale or hidden.
    /// - [`DocumentError::ExplodeSessionScope`] — `component` is on loan to
    ///   an open component session (its members are world nodes then).
    /// - [`DocumentError::UnknownObject`] / [`DocumentError::UnknownGroup`] /
    ///   [`DocumentError::UnknownInstance`] — a node is not a live member of
    ///   `component`.
    /// - [`DocumentError::EmptySelection`] — nothing listed, or nothing
    ///   visible beneath the list.
    /// - [`DocumentError::Transform`] — `t` is singular, or leaves a face too
    ///   small to reopen.
    ///
    /// On `Err` the document is untouched.
    pub fn transform_def_members(
        &mut self,
        component: ComponentId,
        nodes: &[NodeId],
        t: &Transform,
    ) -> Result<DocChange, DocumentError> {
        info!(target: "kernel::op", op = "transform_def_members", nodes = nodes.len());
        self.check_def_members(component, nodes)?;
        t.inverse().map_err(DocumentError::Transform)?;

        let (objects, instances, groups) = self.flatten_def_members(nodes);
        let mut sketches: Vec<SketchId> = self.owned_sketches_of(&objects);
        merge_unique(&mut sketches, self.group_held_sketches_of(&groups));
        if objects.is_empty() && instances.is_empty() && sketches.is_empty() {
            return Err(DocumentError::EmptySelection);
        }

        // Snapshot before mutating (rule 9: undo restores these verbatim).
        let pre_objects: Vec<(ObjectId, Object)> = objects
            .iter()
            .map(|&id| (id, self.objects[id].object.clone()))
            .collect();
        let pre_sketches: Vec<(SketchId, Sketch)> = sketches
            .iter()
            .map(|&id| (id, self.sketches[id].clone()))
            .collect();
        // Bake on copies, then commit: one target refusing leaves all of
        // them as they were.
        let (baked_objects, baked_sketches) = self.scaled_copies(&objects, &sketches, t)?;
        for (id, object) in baked_objects {
            self.objects[id].object = object;
        }
        for (id, sketch) in baked_sketches {
            self.sketches[id] = sketch;
        }
        // Instance poses last: cannot fail once `t` is known invertible.
        let instance_prevs: Vec<(InstanceId, Transform)> = instances
            .iter()
            .map(|&i| (i, self.compose_instance_pose(i, t)))
            .collect();

        let touched: Vec<NodeId> = objects
            .iter()
            .map(|&o| NodeId::Object(o))
            .chain(instances.iter().map(|&i| NodeId::Instance(i)))
            .chain(groups.iter().map(|&g| NodeId::Group(g)))
            .collect();
        let reanchored = self.reanchor_touched(&touched, t);
        self.undo.push(DocAction::TransformSelection {
            objects: pre_objects,
            sketches: pre_sketches,
            instances: instance_prevs,
            forward: *t,
            reanchored,
        });
        self.commit_new_action();
        self.debug_validate();

        let mut instances_touched = instances;
        merge_unique(&mut instances_touched, self.placing_instances(component));
        Ok(DocChange {
            objects_touched: objects,
            sketches_touched: sketches,
            groups_touched: groups,
            instances_touched,
            components_touched: vec![component],
            guides_touched: Vec::new(),
            linked: Vec::new(),
        })
    }

    /// [`Document::duplicate_nodes_array`] inside a definition: `count`
    /// clones of each listed member at `step` multiples (`step`, `step²`, …,
    /// in the definition's frame), inserted as members of `component`
    /// (objects with `ObjectOwner::Definition`, instances and groups with
    /// `owner_def: Some(component)`). A clone of a direct member is appended
    /// to `ComponentDef.members`; a clone of a node inside a member group
    /// lands in that group. One [`DocAction::DuplicatedArray`], whose undo
    /// hides the clones (a hidden direct member stays listed, as
    /// [`Document::delete_def_member`] leaves one). Returns the clone roots
    /// in [`Document::duplicate_nodes_array`]'s order.
    ///
    /// # Errors
    /// As [`Document::transform_def_members`], plus:
    /// - [`DocumentError::ExplodeSessionScope`] — any editing session is
    ///   open (its close folds every node created since its open into the
    ///   open container, and a definition's clone is not session geometry).
    /// - [`DocumentError::DuplicateMember`] — the same node listed twice.
    /// - [`DocumentError::ComponentExpansionExceeded`] — copied instances
    ///   would push the model past the placement bound.
    ///
    /// On `Err` the document is untouched (partial clones are rolled back).
    pub fn duplicate_def_members_array(
        &mut self,
        component: ComponentId,
        nodes: &[NodeId],
        step: &Transform,
        count: std::num::NonZeroU32,
    ) -> Result<(Vec<NodeId>, DocChange), DocumentError> {
        let mut placements = Vec::with_capacity(count.get() as usize);
        let mut placement = *step;
        for k in 0..count.get() {
            if k > 0 {
                placement = placement.then(step);
            }
            placements.push(placement);
        }
        self.duplicate_def_members_placements(component, nodes, &placements)
    }

    /// [`Document::duplicate_nodes_placements`] inside a definition: one
    /// clone of each listed member per placement (definition frame),
    /// placement-major, as ONE [`DocAction::DuplicatedArray`] — the grid
    /// form of [`Document::duplicate_def_members_array`], which is this op
    /// over `[step, step², …]`. Same clone rules and errors.
    pub fn duplicate_def_members_placements(
        &mut self,
        component: ComponentId,
        nodes: &[NodeId],
        placements: &[Transform],
    ) -> Result<(Vec<NodeId>, DocChange), DocumentError> {
        info!(
            target: "kernel::op",
            op = "duplicate_def_members_placements",
            nodes = nodes.len(),
            placements = placements.len(),
        );
        self.refuse_during_any_session()?;
        self.check_def_members(component, nodes)?;
        if placements.is_empty() {
            return Err(DocumentError::EmptySelection);
        }
        for (i, &n) in nodes.iter().enumerate() {
            if nodes[..i].contains(&n) {
                return Err(DocumentError::DuplicateMember);
            }
        }
        let (objects, instances, _) = self.flatten_def_members(nodes);
        if objects.is_empty() && instances.is_empty() {
            return Err(DocumentError::EmptySelection);
        }
        for placement in placements {
            placement.inverse().map_err(DocumentError::Transform)?;
        }
        // A copied member instance renders once per placement of the
        // definition it is copied into.
        let renders = self.def_world_renders(component).max(1);
        self.refuse_expansion_growth(nodes, placements.len().saturating_mul(renders))?;

        let mut created = CreatedClone::default();
        let sid_mark = self.next_sid;
        let mut roots: Vec<(NodeId, Option<GroupId>)> = Vec::new();
        for placement in placements {
            for &node in nodes {
                let parent = self.node_parent(node);
                match self.clone_subtree(node, parent, Some(component), placement, &mut created) {
                    Ok(root) => roots.push((root, parent)),
                    Err(e) => {
                        self.rollback_clone(created, sid_mark);
                        return Err(e);
                    }
                }
            }
        }
        for &(root, parent) in &roots {
            match parent {
                Some(pg) => self.groups[pg].members.push(root),
                None => self.components[component].members.push(root),
            }
        }

        self.undo.push(DocAction::DuplicatedArray {
            roots: roots.clone(),
            objects: created.objects.clone(),
            groups: created.groups.clone(),
            instances: created.instances.clone(),
            copied: created.pairs.clone(),
        });
        self.commit_new_action();
        self.debug_validate();

        let mut change = DocChange {
            objects_touched: created.objects,
            groups_touched: created.groups,
            instances_touched: created.instances,
            components_touched: vec![component],
            ..Default::default()
        };
        for &(_, parent) in &roots {
            if let Some(pg) = parent
                && !change.groups_touched.contains(&pg)
            {
                change.groups_touched.push(pg);
            }
        }
        merge_unique(
            &mut change.instances_touched,
            self.placing_instances(component),
        );
        Ok((roots.into_iter().map(|(root, _)| root).collect(), change))
    }

    /// How many times `component` renders in the world right now: once per
    /// live world instance of it, plus once per placement of it nested in
    /// another definition, multiplied through every placement of that one.
    fn def_world_renders(&self, component: ComponentId) -> usize {
        let mut memo: BTreeMap<ComponentId, usize> = BTreeMap::new();
        let mut total = 0usize;
        for (_, r) in self.instances.iter() {
            if !r.hidden && r.owner_def.is_none() {
                total = total.saturating_add(self.renders_within(r.def, component, &mut memo, 0));
            }
        }
        total
    }

    /// How many times `target` renders inside one placement of `def`.
    fn renders_within(
        &self,
        def: ComponentId,
        target: ComponentId,
        memo: &mut BTreeMap<ComponentId, usize>,
        depth: usize,
    ) -> usize {
        if def == target {
            return 1;
        }
        if depth > MAX_COMPONENT_DEPTH {
            return 0;
        }
        if let Some(&n) = memo.get(&def) {
            return n;
        }
        let mut n = 0usize;
        for (_, r) in self.instances.iter() {
            if !r.hidden && r.owner_def == Some(def) {
                n = n.saturating_add(self.renders_within(r.def, target, memo, depth + 1));
            }
        }
        memo.insert(def, n);
        n
    }

    /// The one live member of `component` when that member is a direct
    /// Object and nothing else in the definition is live — a linked copy's
    /// shape, where a placement names its host without a leaf. `Ok(None)`
    /// when the definition holds several live parts, so the caller must
    /// name one.
    ///
    /// # Errors
    /// - [`DocumentError::UnknownComponent`] — `component` is stale or
    ///   hidden.
    /// - [`DocumentError::BooleanOperandEmpty`] — no live part at all.
    /// - [`DocumentError::GroupedOperand`] — the one live part is a group
    ///   (its objects sit inside it).
    /// - [`DocumentError::BooleanOperandHasInstance`] — the one live part is
    ///   a nested placement.
    pub fn sole_member_object(
        &self,
        component: ComponentId,
    ) -> Result<Option<ObjectId>, DocumentError> {
        let c = self
            .components
            .get(component)
            .filter(|c| !c.hidden)
            .ok_or(DocumentError::UnknownComponent)?;
        let mut live = c
            .members
            .iter()
            .copied()
            .filter(|&n| self.is_live_def_member(component, n));
        match (live.next(), live.next()) {
            (Some(NodeId::Object(o)), None) => Ok(Some(o)),
            (Some(NodeId::Group(_)), None) => Err(DocumentError::GroupedOperand),
            (Some(NodeId::Instance(_)), None) => Err(DocumentError::BooleanOperandHasInstance),
            (None, _) => Err(DocumentError::BooleanOperandEmpty),
            (Some(_), Some(_)) => Ok(None),
        }
    }

    /// The definition at the end of `path` and the path's composed pose
    /// (world ← that definition's frame), for the world-geometry ops below.
    fn placement_target(
        &self,
        path: &[InstanceId],
    ) -> Result<(ComponentId, Transform), DocumentError> {
        let &last = path.last().ok_or(DocumentError::UnknownInstance)?;
        let pose = self.path_pose(path).ok_or(DocumentError::UnknownInstance)?;
        let component = self.instances[last].def;
        if self.components.get(component).is_none_or(|c| c.hidden) {
            return Err(DocumentError::UnknownComponent);
        }
        Ok((component, pose))
    }

    /// A boolean whose host is a definition member, seen through one of its
    /// placements, and whose tool is a world Object (Addendum D §5's
    /// "Subtract or Trim, host is a placement"): the tool body is mapped
    /// into the definition's frame through the inverse of `path`'s composed
    /// pose, `host` minus the tool runs inside the definition (as
    /// [`Document::boolean_in_component`] does for two members), and the
    /// world tool is consumed. Every placement of the definition shows the
    /// result. The result is a new direct member that keeps the host's
    /// name, tags, and attributes; the host's own sketches carry to it, the
    /// tool's go with the tool. One [`DocAction::Boolean`] (one undo entry).
    ///
    /// `path` runs from a world instance down to the instance whose
    /// definition owns `host` — `[instance, …]` of the occurrence locator
    /// `[instance, …, host]`.
    ///
    /// A mirrored placement works: the tool maps through the reflecting
    /// pose and the boolean resolves it in the definition's frame.
    ///
    /// `op` is [`BooleanOp::Subtract`]. A union or intersect with a world
    /// tool would move world material into every placement of a shared
    /// definition, which Addendum D §5 leaves to unlinking (make the
    /// placements unique and explode them, then combine), so those refuse.
    ///
    /// # Errors
    /// - [`DocumentError::BooleanOperandHasInstance`] — `op` is a union or
    ///   intersect: the host is reached through a placement, and those ops
    ///   do not propagate to the other placements.
    /// - [`DocumentError::ExplodeSessionScope`] — an editing session is open
    ///   (the definition, or one the path passes through, may be on loan,
    ///   and a session close would fold the new member into the open
    ///   container).
    /// - [`DocumentError::UnknownInstance`] — `path` is empty or does not
    ///   resolve.
    /// - [`DocumentError::UnknownComponent`] — the definition is stale.
    /// - [`DocumentError::UnknownObject`] — `host` is not a live member of the
    ///   path's definition, or `tool` is not a live world Object.
    /// - [`DocumentError::GroupedOperand`] — `host` sits in a member group,
    ///   or `tool` in a world group.
    /// - [`DocumentError::Transform`] — the path's pose does not invert.
    /// - [`DocumentError::Boolean`] — the engine refused (non-solid operand,
    ///   empty result, a subtract that removes nothing, …).
    ///
    /// On `Err` the document is untouched.
    pub fn boolean_through_placement(
        &mut self,
        path: &[InstanceId],
        host: ObjectId,
        tool: ObjectId,
        op: BooleanOp,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "boolean_through_placement", boolean_op = ?op, depth = path.len());
        if op != BooleanOp::Subtract {
            return Err(DocumentError::BooleanOperandHasInstance);
        }
        self.cut_through_placement(path, host, PlacementTool::Record(tool))
    }

    /// [`Document::boolean_through_placement`] with a transient cutter: a
    /// watertight solid in WORLD coordinates that no record holds (the
    /// API's `SolidSpec` prism, Addendum B6), so a machining through a
    /// placement is one call with no temporary object. `host` minus the
    /// cutter runs in the definition's frame, every placement shows the
    /// cut, and the one [`DocAction::Boolean`] it records carries no `b`:
    /// undo brings back only the host.
    ///
    /// # Errors
    /// As [`Document::boolean_through_placement`], less the tool's own
    /// record checks; a cutter that is not a watertight solid refuses
    /// [`DocumentError::Boolean`] from the engine.
    ///
    /// On `Err` the document is untouched.
    pub fn subtract_solid_through_placement(
        &mut self,
        path: &[InstanceId],
        host: ObjectId,
        cutter: &Object,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "subtract_solid_through_placement", depth = path.len());
        self.cut_through_placement(path, host, PlacementTool::Transient(cutter))
    }

    /// The subtract both forms above share.
    fn cut_through_placement(
        &mut self,
        path: &[InstanceId],
        host: ObjectId,
        tool: PlacementTool<'_>,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        let op = BooleanOp::Subtract;
        self.refuse_during_any_session()?;
        let (component, pose) = self.placement_target(path)?;
        let rec_host = self
            .objects
            .get(host)
            .filter(|r| {
                !r.hidden
                    && matches!(r.owner, ObjectOwner::Definition { def, .. } if def == component)
            })
            .ok_or(DocumentError::UnknownObject)?;
        let (tool_id, tool_object) = match tool {
            PlacementTool::Record(tool) => {
                let rec_tool = self
                    .objects
                    .get(tool)
                    .filter(|r| !r.hidden && r.is_world())
                    .ok_or(DocumentError::UnknownObject)?;
                // A replacing op: a grouped operand would leave its group
                // listing a consumed node (`Document::boolean`'s posture).
                if rec_tool.group_parent().is_some() {
                    return Err(DocumentError::GroupedOperand);
                }
                (Some(tool), &rec_tool.object)
            }
            PlacementTool::Transient(cutter) => (None, cutter),
        };
        if rec_host.group_parent().is_some() {
            return Err(DocumentError::GroupedOperand);
        }
        let to_def = pose.inverse().map_err(DocumentError::Transform)?;

        // Keep the coplanar edges either operand already had (imprints), in
        // the definition's frame; dissolve the seams the boolean added.
        let result = Object::boolean_product(op, &rec_host.object, tool_object, &to_def)
            .map_err(DocumentError::Boolean)?;

        let (name, tags) = (rec_host.name.clone(), rec_host.tags.clone());
        let id = self.insert_object_record(ObjectRecord {
            object: result,
            history: History::new(),
            hidden: false,
            owner: ObjectOwner::Definition {
                def: component,
                parent: None,
            },
            name,
            tags,
        });
        self.copy_attrs(&EntityRef::Object(host), EntityRef::Object(id));
        self.objects[host].hidden = true;
        let mut consumed = vec![NodeId::Object(host)];
        if let Some(tool) = tool_id {
            self.objects[tool].hidden = true;
            consumed.push(NodeId::Object(tool));
        }
        // Membership is structural: the consumed host stays listed, hidden,
        // as `boolean_in_component` leaves its operands.
        self.components[component].members.push(NodeId::Object(id));
        let reanchored = self.reevaluate_liveness_recorded(&consumed);
        let carried = self.carry_owned_sketches(&[host], &[id]);
        let carried_sketches: Vec<SketchId> = carried
            .iter()
            .flat_map(DocAction::touched_sketches)
            .collect();
        self.push_with_followups(
            DocAction::Boolean {
                op,
                result: id,
                a: host,
                b: tool_id,
                reanchored,
            },
            carried,
        );
        self.commit_new_action();
        self.debug_validate();

        let mut change = DocChange {
            objects_touched: [host].into_iter().chain(tool_id).chain([id]).collect(),
            components_touched: vec![component],
            instances_touched: self.placing_instances(component),
            ..Default::default()
        };
        merge_unique(&mut change.sketches_touched, carried_sketches);
        Ok((id, change))
    }

    /// [`Document::boolean_in_component`] between two members of the
    /// definition at the end of `path`, both reached through that same
    /// path (the occurrence locators `[instance, …, a]` and
    /// `[instance, …, b]`): `op` runs in the definition's own frame, so no
    /// pose is involved, and every placement shows the result. One
    /// [`DocAction::Boolean`] (one undo entry).
    ///
    /// The path form adds the gates [`Document::boolean_through_placement`]
    /// makes; [`Document::boolean_in_component`] itself is unchanged for
    /// its other callers.
    ///
    /// # Errors
    /// - [`DocumentError::ExplodeSessionScope`] — an editing session is open
    ///   (a session close would fold the new member into the open
    ///   container).
    /// - [`DocumentError::UnknownInstance`] / [`DocumentError::UnknownComponent`]
    ///   — `path` is empty or does not resolve, or its definition is stale.
    /// - [`DocumentError::GroupedOperand`] — `a` or `b` sits in a member
    ///   group (a replacing op would leave the group listing a consumed
    ///   node).
    /// - As [`Document::boolean_in_component`] otherwise.
    ///
    /// On `Err` the document is untouched.
    pub fn boolean_members_through_placement(
        &mut self,
        path: &[InstanceId],
        a: ObjectId,
        b: ObjectId,
        op: BooleanOp,
    ) -> Result<(ObjectId, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "boolean_members_through_placement", boolean_op = ?op, depth = path.len());
        self.refuse_during_any_session()?;
        let (component, _pose) = self.placement_target(path)?;
        let grouped = |o: ObjectId| {
            self.objects.get(o).is_some_and(|r| {
                !r.hidden
                    && matches!(r.owner, ObjectOwner::Definition { def, parent: Some(_) } if def == component)
            })
        };
        if grouped(a) || grouped(b) {
            return Err(DocumentError::GroupedOperand);
        }
        self.boolean_in_component(component, a, b, op)
    }

    /// [`Document::slice_def_member`] through an occurrence path: cuts
    /// `object`, a live direct member of the definition at the end of
    /// `path`, by a WORLD `plane` mapped into the definition's frame through
    /// the path's composed pose. The two pieces become members of the same
    /// definition; every placement follows. Returns `(positive, negative)`,
    /// the piece on the world plane's normal side first.
    ///
    /// # Errors
    /// - [`DocumentError::ExplodeSessionScope`] — an editing session is open.
    /// - [`DocumentError::UnknownInstance`] / [`DocumentError::UnknownComponent`]
    ///   — `path` is empty or does not resolve, or its definition is stale.
    /// - [`DocumentError::UnknownObject`] — `object` is not a live member.
    /// - [`DocumentError::GroupedOperand`] — `object` sits in a member group.
    /// - [`DocumentError::Slice`] — the cut is degenerate or misses.
    ///
    /// On `Err` the document is untouched.
    pub fn slice_through_placement(
        &mut self,
        path: &[InstanceId],
        object: ObjectId,
        plane: &Plane,
    ) -> Result<((ObjectId, ObjectId), DocChange), DocumentError> {
        info!(target: "kernel::op", op = "slice_through_placement", depth = path.len());
        self.refuse_during_any_session()?;
        let (component, pose) = self.placement_target(path)?;
        let grouped = self.objects.get(object).is_some_and(|r| {
            !r.hidden
                && matches!(r.owner, ObjectOwner::Definition { def, parent: Some(_) } if def == component)
        });
        if grouped {
            return Err(DocumentError::GroupedOperand);
        }
        self.slice_member_through_pose(component, &pose, object, plane)
    }
}
