//! What one [`DocAction`] touched, as a [`ChangeSet`]
//! (docs/agents/HEW_API.md §4.5; the change ring is [`crate::changes`]).
//!
//! The classification follows each variant's doc comment: an entity the
//! action created or placed is `added`; one it deleted, consumed,
//! exploded, ungrouped, or hid is `removed`; every other entity it names is
//! `modified`. A field that snapshots a container's member list (for an
//! exact undo of the order) names the container, which is `modified`; the
//! listed members themselves are not. A group whose membership changed is
//! `modified`. The match is exhaustive with no wildcard arm, so a new
//! variant forces a decision here.

use crate::changes::{AttrChange, AttrScope, ChangeRef, ChangeSet};
use crate::ids::{
    AnnotationId, ComponentId, GroupId, GuideId, InstanceId, MaterialId, ObjectId, SketchId,
};

use super::{AnnotationReanchor, DocAction, Document, EntityRef, NodeId};

/// Builds one [`ChangeSet`], resolving stable ids as it goes.
struct Builder<'d> {
    doc: &'d Document,
    set: ChangeSet,
}

#[derive(Clone, Copy)]
enum List {
    Added,
    Removed,
    Modified,
}

fn node_entity(node: NodeId) -> EntityRef {
    match node {
        NodeId::Object(o) => EntityRef::Object(o),
        NodeId::Group(g) => EntityRef::Group(g),
        NodeId::Instance(i) => EntityRef::Instance(i),
    }
}

impl Builder<'_> {
    fn put(&mut self, list: List, entity: EntityRef) {
        let Some(sid) = self.doc.sid_of(&entity) else {
            // Live and tombstoned entities keep their stable ids, so this
            // is unreachable; naming nothing would be a silent drop.
            self.set.full = true;
            return;
        };
        self.put_sid(list, entity, sid);
    }

    fn put_sid(&mut self, list: List, entity: EntityRef, sid: u64) {
        let target = match list {
            List::Added => &mut self.set.added,
            List::Removed => &mut self.set.removed,
            List::Modified => &mut self.set.modified,
        };
        let r = ChangeRef { entity, sid };
        if !target.contains(&r) {
            target.push(r);
        }
    }

    fn object(&mut self, list: List, id: ObjectId) {
        self.put(list, EntityRef::Object(id));
    }
    fn objects(&mut self, list: List, ids: impl IntoIterator<Item = ObjectId>) {
        for id in ids {
            self.object(list, id);
        }
    }
    fn sketch(&mut self, list: List, id: SketchId) {
        self.put(list, EntityRef::Sketch(id));
    }
    fn sketches(&mut self, list: List, ids: impl IntoIterator<Item = SketchId>) {
        for id in ids {
            self.sketch(list, id);
        }
    }
    fn group(&mut self, list: List, id: GroupId) {
        self.put(list, EntityRef::Group(id));
    }
    fn groups(&mut self, list: List, ids: impl IntoIterator<Item = GroupId>) {
        for id in ids {
            self.group(list, id);
        }
    }
    fn parent(&mut self, parent: Option<GroupId>) {
        if let Some(g) = parent {
            self.group(List::Modified, g);
        }
    }
    fn instance(&mut self, list: List, id: InstanceId) {
        self.put(list, EntityRef::Instance(id));
    }
    fn instances(&mut self, list: List, ids: impl IntoIterator<Item = InstanceId>) {
        for id in ids {
            self.instance(list, id);
        }
    }
    fn component(&mut self, list: List, id: ComponentId) {
        self.put(list, EntityRef::Component(id));
    }
    fn guide(&mut self, list: List, id: GuideId) {
        self.put(list, EntityRef::Guide(id));
    }
    fn material(&mut self, list: List, id: MaterialId) {
        self.put(list, EntityRef::Material(id));
    }
    fn annotation(&mut self, list: List, id: AnnotationId) {
        self.put(list, EntityRef::Annotation(id));
    }
    fn node(&mut self, list: List, node: NodeId) {
        self.put(list, node_entity(node));
    }
    fn nodes(&mut self, list: List, nodes: impl IntoIterator<Item = NodeId>) {
        for n in nodes {
            self.node(list, n);
        }
    }
    /// The members a group record lists now (an ungroup or a regroup
    /// reparented them).
    fn group_members(&mut self, group: GroupId) {
        let members = self
            .doc
            .groups
            .get(group)
            .map(|r| r.members.clone())
            .unwrap_or_default();
        self.nodes(List::Modified, members);
    }
    /// Annotations whose anchors or detached state the action rewrote.
    fn reanchored(&mut self, reanchored: &[AnnotationReanchor]) {
        for r in reanchored {
            self.annotation(List::Modified, r.annotation);
        }
    }
    /// A tag by path; a rename names it by either side, whichever the
    /// stable-id table holds now.
    fn tag(&mut self, list: List, paths: &[&Vec<String>]) {
        for path in paths {
            let entity = EntityRef::Tag((*path).clone());
            if let Some(sid) = self.doc.sid_of(&entity) {
                self.put_sid(list, entity, sid);
                return;
            }
        }
        self.set.full = true;
    }
    /// A face or edge dictionary namespace, named by its Object.
    fn element_attr(&mut self, object: ObjectId, scope: AttrScope, ns: &str) {
        let entity = EntityRef::Object(object);
        let Some(sid) = self.doc.sid_of(&entity) else {
            self.set.full = true;
            return;
        };
        let change = AttrChange {
            object: Some(ChangeRef { entity, sid }),
            target: None,
            scope,
            ns: ns.to_string(),
        };
        if !self.set.attrs.contains(&change) {
            self.set.attrs.push(change);
        }
    }
    /// The definitions owning any of the listed records, as `modified`: a
    /// member edited, copied, or replaced in place changes its definition
    /// (as [`DocAction::DefObjectOp`] reports). World records add nothing.
    fn owner_defs(&mut self, objects: &[ObjectId], groups: &[GroupId], instances: &[InstanceId]) {
        let (defs, _) = self.doc.def_owners_change(objects, groups, instances);
        for c in defs {
            self.component(List::Modified, c);
        }
    }
    fn union(&mut self, other: &ChangeSet) {
        self.set.union_with(other);
    }
    /// `(new, source)` pairs for `copied_from`, resolved to stable ids.
    fn copied(&mut self, pairs: impl IntoIterator<Item = (EntityRef, EntityRef)>) {
        for (new, src) in pairs {
            let (Some(n), Some(s)) = (self.doc.sid_of(&new), self.doc.sid_of(&src)) else {
                self.set.full = true;
                return;
            };
            let pair = (
                ChangeRef {
                    entity: new,
                    sid: n,
                },
                ChangeRef {
                    entity: src,
                    sid: s,
                },
            );
            if !self.set.copied_from.contains(&pair) {
                self.set.copied_from.push(pair);
            }
        }
    }
    fn node_pairs(&mut self, pairs: &[(NodeId, NodeId)]) {
        self.copied(pairs.iter().map(|&(n, s)| (node_entity(n), node_entity(s))));
    }
    /// A linked-set definition hidden with its last placement, and the
    /// member nodes hidden with it.
    fn hidden_def(&mut self, def: ComponentId, nodes: &[NodeId]) {
        self.component(List::Removed, def);
        self.nodes(List::Removed, nodes.iter().copied());
    }
}

impl Document {
    /// The entities `action` touched. See the module comment for the
    /// classification rule.
    pub(super) fn action_changes(&self, action: &DocAction) -> ChangeSet {
        let mut b = Builder {
            doc: self,
            set: ChangeSet::default(),
        };
        use List::{Added, Modified, Removed};
        match action {
            DocAction::Compound { actions, .. } | DocAction::PlaceTextCompound(actions) => {
                for child in actions {
                    let child = self.action_changes(child);
                    b.union(&child);
                }
            }
            DocAction::AttrSet { target, ns, .. } | DocAction::AttrDeleted { target, ns, .. } => {
                b.set
                    .attrs
                    .push(AttrChange::entity(target.clone(), ns.clone()));
            }
            DocAction::FaceAttrSet { object, ns, .. }
            | DocAction::FaceAttrDeleted { object, ns, .. } => {
                b.element_attr(*object, AttrScope::Face, ns);
            }
            DocAction::EdgeAttrSet { object, ns, .. }
            | DocAction::EdgeAttrDeleted { object, ns, .. } => {
                b.element_attr(*object, AttrScope::Edge, ns);
            }
            DocAction::CreatedObject {
                id,
                sketch,
                emptied,
                merged_base,
                reanchored,
                ..
            } => {
                b.object(Added, *id);
                b.sketch(if *emptied { Removed } else { Modified }, *sketch);
                if let Some(base) = merged_base {
                    b.object(Removed, *base);
                }
                b.reanchored(reanchored);
            }
            DocAction::CreatedObjectFromFaces { id } => {
                // The consumed scaffolding is its own rows beside this one.
                b.object(Added, *id);
                b.owner_defs(&[*id], &[], &[]);
            }
            DocAction::GrewObjectFromFaces {
                from,
                to,
                reanchored,
            } => {
                b.object(Added, *to);
                b.object(Removed, *from);
                b.owner_defs(&[*from, *to], &[], &[]);
                b.reanchored(reanchored);
            }
            DocAction::ObjectOp { object } => b.object(Modified, *object),
            DocAction::Boolean {
                result,
                operands,
                reanchored,
                ..
            } => {
                b.object(Added, *result);
                b.objects(Removed, operands.iter().copied());
                b.owner_defs(&[*result], &[], &[]);
                b.reanchored(reanchored);
            }
            DocAction::BooleanNodes {
                operands,
                hidden_operands,
                result_objects,
                result_group,
                reanchored,
                ..
            } => {
                b.objects(Added, result_objects.iter().copied());
                if let Some(g) = result_group {
                    b.group(Added, *g);
                }
                // The operand roots are in `hidden_operands` too.
                b.nodes(Removed, operands.iter().copied());
                b.nodes(Removed, hidden_operands.iter().copied());
                b.reanchored(reanchored);
            }
            DocAction::Sliced {
                source,
                a,
                b: other,
                reanchored,
            } => {
                b.objects(Added, [*a, *other]);
                b.object(Removed, *source);
                b.owner_defs(&[*source], &[], &[]);
                b.reanchored(reanchored);
            }
            DocAction::Repaired {
                object,
                split_off,
                parent,
                ..
            } => {
                b.object(Modified, *object);
                b.objects(Added, split_off.iter().copied());
                if !split_off.is_empty() {
                    b.parent(*parent);
                }
            }
            DocAction::PushThrough {
                source,
                results,
                reanchored,
            } => {
                b.objects(Added, results.iter().copied());
                b.object(Removed, *source);
                b.reanchored(reanchored);
            }
            DocAction::Transform {
                objects,
                sketches,
                reanchored,
                ..
            } => {
                b.objects(Modified, objects.iter().map(|(o, _)| *o));
                b.sketches(Modified, sketches.iter().map(|(s, _)| *s));
                b.reanchored(reanchored);
            }
            DocAction::TransformSketch { sketch, .. }
            | DocAction::TransformSketchIsland { sketch, .. }
            | DocAction::MovedSketchVertex { sketch, .. }
            | DocAction::RearrangedSketchVertex { sketch, .. } => b.sketch(Modified, *sketch),
            DocAction::DetachedSketchIsland {
                source, detached, ..
            } => {
                b.sketch(Added, *detached);
                b.sketch(Modified, *source);
            }
            DocAction::CopiedSketchIslands { copy } => b.sketch(Added, *copy),
            DocAction::SketchOwnerChanged { sketch, prev, next } => {
                b.sketch(Modified, *sketch);
                b.objects(Modified, prev.iter().chain(next.iter()).copied());
            }
            DocAction::SketchGroupChanged { sketch, prev, next } => {
                b.sketch(Modified, *sketch);
                b.groups(Modified, prev.iter().chain(next.iter()).copied());
            }
            DocAction::TransformSelection {
                objects,
                sketches,
                instances,
                reanchored,
                ..
            } => {
                b.objects(Modified, objects.iter().map(|(o, _)| *o));
                b.sketches(Modified, sketches.iter().map(|(s, _)| *s));
                b.instances(Modified, instances.iter().map(|(i, _)| *i));
                let objs: Vec<ObjectId> = objects.iter().map(|(o, _)| *o).collect();
                let insts: Vec<InstanceId> = instances.iter().map(|(i, _)| *i).collect();
                // A member group holding only drawing names its definition
                // through the group.
                let sks: Vec<SketchId> = sketches.iter().map(|(s, _)| *s).collect();
                let groups = self.holding_groups(&sks);
                b.owner_defs(&objs, &groups, &insts);
                b.reanchored(reanchored);
            }
            DocAction::Rescale { .. } => b.set.full = true,
            DocAction::RebasedDefinition {
                component,
                objects,
                sketches,
                nested,
                placing,
                ..
            } => {
                b.component(Modified, *component);
                b.objects(Modified, objects.iter().map(|(o, _)| *o));
                b.sketches(Modified, sketches.iter().map(|(s, _)| *s));
                b.instances(Modified, nested.iter().map(|(i, ..)| *i));
                b.instances(Modified, placing.iter().map(|(i, ..)| *i));
            }
            DocAction::SetAxes { .. } | DocAction::SetGeoReference { .. } => b.set.document = true,
            DocAction::SketchGesture {
                sketch, created, ..
            } => b.sketch(if *created { Added } else { Modified }, *sketch),
            DocAction::Grouped { group, parent, .. } => {
                b.group(Added, *group);
                b.parent(*parent);
                b.group_members(*group);
            }
            DocAction::Reparented {
                node,
                prev_parent,
                next_parent,
                ..
            } => {
                b.node(Modified, *node);
                b.parent(*prev_parent);
                b.parent(*next_parent);
            }
            DocAction::Ungrouped {
                group,
                parent,
                reanchored,
                ..
            } => {
                b.group(Removed, *group);
                b.parent(*parent);
                b.group_members(*group);
                b.reanchored(reanchored);
            }
            DocAction::Deleted {
                node,
                parent,
                hidden_subtree,
                reanchored,
                hidden_defs,
                ..
            } => {
                b.node(Removed, *node);
                b.nodes(Removed, hidden_subtree.iter().copied());
                b.parent(*parent);
                b.reanchored(reanchored);
                for (d, nodes) in hidden_defs {
                    b.hidden_def(*d, nodes);
                }
            }
            DocAction::MadeComponent {
                component,
                instance,
                selected,
                parent,
                member_prior_parents,
                folded_groups,
                folded_instances,
                reanchored,
                ..
            } => {
                b.component(Added, *component);
                b.instance(Added, *instance);
                // Folded into the definition, not hidden: same entities,
                // now definition members.
                b.nodes(Modified, selected.iter().copied());
                b.objects(Modified, member_prior_parents.iter().map(|(o, _)| *o));
                b.groups(
                    Modified,
                    member_prior_parents.iter().filter_map(|(_, g)| *g),
                );
                b.groups(Modified, folded_groups.iter().copied());
                b.instances(Modified, folded_instances.iter().copied());
                b.parent(*parent);
                b.reanchored(reanchored);
            }
            DocAction::WrappedLinked {
                def,
                instance,
                node,
                parent,
                owner_def,
                member_prior_parents,
                folded_groups,
                folded_instances,
                reanchored,
                ..
            } => {
                b.component(Added, *def);
                b.instance(Added, *instance);
                // The node is re-owned, not hidden: same entities, now
                // members of the new definition.
                b.node(Modified, *node);
                b.objects(Modified, member_prior_parents.iter().map(|(o, _)| *o));
                b.groups(Modified, folded_groups.iter().copied());
                b.instances(Modified, folded_instances.iter().copied());
                b.parent(*parent);
                if let Some(outer) = owner_def {
                    b.component(Modified, *outer);
                }
                b.reanchored(reanchored);
                b.set.linked.push(crate::document::LinkedWrap {
                    source: *node,
                    instance: *instance,
                    definition: *def,
                });
            }
            DocAction::LinkedCopies { wraps, array } => {
                for child in wraps.iter().chain(std::iter::once(array.as_ref())) {
                    let child = self.action_changes(child);
                    b.union(&child);
                }
            }
            DocAction::PlacedInstance { instance } => b.instance(Added, *instance),
            DocAction::JoinedLinked {
                joiner,
                instance,
                set,
                reanchored,
                keeper,
                cleared_base,
                restamped,
                ..
            } => {
                b.instance(Added, *instance);
                b.node(Removed, NodeId::Object(*joiner));
                // The member's base moved onto the set's other placements.
                if cleared_base.is_some() {
                    b.object(Modified, *keeper);
                    b.instances(Modified, restamped.iter().map(|&(i, _)| i));
                }
                b.reanchored(reanchored);
                b.copied([(EntityRef::Instance(*instance), EntityRef::Instance(*set))]);
            }
            DocAction::Duplicated {
                root,
                parent,
                objects,
                groups,
                instances,
                copied,
            } => {
                b.node(Added, *root);
                b.objects(Added, objects.iter().copied());
                b.groups(Added, groups.iter().copied());
                b.instances(Added, instances.iter().copied());
                b.parent(*parent);
                b.node_pairs(copied);
            }
            DocAction::DuplicatedArray {
                roots,
                objects,
                groups,
                instances,
                copied,
            } => {
                b.nodes(Added, roots.iter().map(|(n, _)| *n));
                b.objects(Added, objects.iter().copied());
                b.groups(Added, groups.iter().copied());
                b.instances(Added, instances.iter().copied());
                for (_, parent) in roots {
                    b.parent(*parent);
                }
                b.owner_defs(objects, groups, instances);
                b.node_pairs(copied);
            }
            DocAction::CreatedGuide { guide } => b.guide(Added, *guide),
            DocAction::DeletedGuide { guide } => b.guide(Removed, *guide),
            DocAction::DeletedGuides { guides } => {
                for g in guides {
                    b.guide(Removed, *g);
                }
            }
            DocAction::TransformedGuide { guide, .. } => b.guide(Modified, *guide),
            DocAction::DuplicatedGuides { guides } => {
                for g in guides {
                    b.guide(Added, *g);
                }
            }
            DocAction::CreatedAnnotation { annotation } => b.annotation(Added, *annotation),
            DocAction::DeletedAnnotation { annotation } => b.annotation(Removed, *annotation),
            DocAction::UpdatedAnnotation { annotation, .. } => {
                b.annotation(Modified, *annotation);
            }
            DocAction::DeletedSketch { sketch } => b.sketch(Removed, *sketch),
            DocAction::TransformInstance {
                instance,
                reanchored,
                ..
            } => {
                b.instance(Modified, *instance);
                b.reanchored(reanchored);
            }
            DocAction::DefObjectOp { component, object } => {
                b.object(Modified, *object);
                b.component(Modified, *component);
            }
            DocAction::Exploded {
                instance,
                created,
                created_groups,
                created_instances,
                created_roots,
                created_sketches,
                reanchored,
                // A baked base material is part of creating the copies.
                baked_materials: _,
                baked_instance_materials: _,
                hidden_defs,
            } => {
                for (d, nodes) in hidden_defs {
                    b.hidden_def(*d, nodes);
                }
                b.instance(Removed, *instance);
                // The created roots took the instance's place in its
                // group's member list. A hidden instance keeps its parent
                // link, so this reads the same on undo and redo.
                let parent = self.instances.get(*instance).and_then(|r| r.parent);
                b.parent(parent);
                b.nodes(Added, created_roots.iter().copied());
                b.objects(Added, created.iter().copied());
                b.groups(Added, created_groups.iter().copied());
                b.instances(Added, created_instances.iter().copied());
                b.sketches(Added, created_sketches.iter().copied());
                b.reanchored(reanchored);
            }
            DocAction::MadeUnique {
                instance,
                prev_def,
                new_def,
                copied,
                shared,
                hidden_defs,
                ..
            } => {
                b.component(Added, *new_def);
                b.instance(Modified, *instance);
                // The step took a linked set's last placement (`prev_def`
                // first, when it did), and any set placed only inside it.
                if !hidden_defs.iter().any(|(d, _)| d == prev_def) {
                    b.component(Modified, *prev_def);
                }
                for (d, nodes) in hidden_defs {
                    b.hidden_def(*d, nodes);
                }
                b.copied(copied.iter().cloned());
                // The private copy's members and cloned sketches are new
                // entities a consumer would otherwise never hear of.
                for (new, _) in copied {
                    b.put(Added, new.clone());
                }
                // Copies made earlier in the same call, repointed onto the
                // shared private definition.
                b.instances(Modified, shared.iter().copied());
            }
            // Entering or leaving an edit is bookkeeping, and its change
            // set is empty on purpose: the open bakes the component's
            // members into world ownership (and hides its instances) only
            // for the session, and the close folds them back, so neither is
            // an edit of its own. A host reports the session itself with
            // `context_changed`; the edits made inside it are their own
            // entries.
            DocAction::SessionOpened { .. } => {}
            // The close is bookkeeping too, except when it finds the
            // definition emptied: then the definition, the entered
            // instance, and the live sibling placements are deleted
            // outright and anchored annotations detach, which a save
            // writes and a subscriber must hear about.
            DocAction::SessionClosed {
                instance,
                component,
                emptied,
                emptied_instances,
                reanchored,
                ..
            } => {
                if *emptied {
                    b.component(Removed, *component);
                    b.instance(Removed, *instance);
                    b.instances(Removed, emptied_instances.iter().copied());
                    b.reanchored(reanchored);
                }
            }
            // A group session's open hides the group node and its close
            // brings it back; annotations anchored to the group are
            // detached or re-attached, which a save writes.
            DocAction::GroupSessionOpened { reanchored, .. }
            | DocAction::GroupSessionClosed { reanchored, .. } => b.reanchored(reanchored),
            DocAction::PaintFace { object, .. }
            | DocAction::SetObjectMaterial { object, .. }
            | DocAction::SetFaceUvFrame { object, .. } => b.object(Modified, *object),
            DocAction::SetInstanceMaterial {
                instance,
                moved_base,
                ..
            } => {
                b.instance(Modified, *instance);
                // The set's base moved onto its other copies first.
                if let Some(m) = moved_base {
                    b.object(Modified, m.keeper);
                    b.instances(Modified, m.restamped.iter().map(|&(i, _)| i));
                }
            }
            DocAction::ReplaceMaterial {
                from,
                to,
                faces,
                back_faces,
                defaults,
                instances,
            } => {
                b.objects(
                    Modified,
                    faces
                        .iter()
                        .chain(back_faces.iter())
                        .map(|(o, _)| *o)
                        .chain(defaults.iter().copied()),
                );
                b.instances(Modified, instances.iter().copied());
                for m in from.iter().chain(to.iter()) {
                    b.material(Modified, *m);
                }
            }
            DocAction::SetMaterialAlpha { material, .. }
            | DocAction::SetMaterialPbr { material, .. }
            | DocAction::MaterialRenamed { material, .. } => b.material(Modified, *material),
            DocAction::MaterialDeleted {
                material,
                faces,
                back_faces,
                defaults,
                instances,
            } => {
                b.material(Removed, *material);
                b.objects(
                    Modified,
                    faces
                        .iter()
                        .chain(back_faces.iter())
                        .map(|(o, _)| *o)
                        .chain(defaults.iter().copied()),
                );
                b.instances(Modified, instances.iter().copied());
            }
            DocAction::DefinitionDeleted {
                component,
                hidden_nodes,
                hidden_defs,
            } => {
                b.component(Removed, *component);
                b.nodes(Removed, hidden_nodes.iter().copied());
                for (d, nodes) in hidden_defs {
                    b.hidden_def(*d, nodes);
                }
            }
            DocAction::NodeMetaChanged { node, .. } => b.node(Modified, *node),
            DocAction::ComponentRenamed { component, .. } => b.component(Modified, *component),
            DocAction::TagDeleted {
                registry,
                identities,
                nodes,
                annotations,
                sketches,
                guides,
            } => {
                // A deleted tag's stable id leaves the table with it; the
                // action carries it.
                for (path, sid, _) in identities {
                    b.put_sid(Removed, EntityRef::Tag(path.clone()), *sid);
                }
                for (path, _) in registry {
                    if !identities.iter().any(|(p, _, _)| p == path) {
                        b.tag(Removed, &[path]);
                    }
                }
                b.nodes(Modified, nodes.iter().map(|(n, _, _)| *n));
                for (a, _, _) in annotations {
                    b.annotation(Modified, *a);
                }
                b.sketches(Modified, sketches.iter().map(|(s, _, _)| *s));
                for (g, _, _) in guides {
                    b.guide(Modified, *g);
                }
            }
            DocAction::TagRenamed {
                registry,
                nodes,
                annotations,
                sketches,
                guides,
            } => {
                for (before, after) in registry {
                    b.tag(Modified, &[after, before]);
                }
                b.nodes(Modified, nodes.iter().map(|(n, _, _)| *n));
                for (a, _, _) in annotations {
                    b.annotation(Modified, *a);
                }
                b.sketches(Modified, sketches.iter().map(|(s, _, _)| *s));
                for (g, _, _) in guides {
                    b.guide(Modified, *g);
                }
            }
            DocAction::FollowMeFace {
                result,
                merged_base,
                reanchored,
            } => {
                b.object(Added, *result);
                if let Some(base) = merged_base {
                    b.object(Removed, *base);
                }
                b.reanchored(reanchored);
            }
            DocAction::Imported { .. } => b.set.full = true,
            DocAction::DeletedDefMember {
                component,
                object,
                from_group,
            } => {
                b.object(Removed, *object);
                b.component(Modified, *component);
                if let Some((g, _)) = from_group {
                    b.group(Modified, *g);
                }
            }
            // The scratch sketch retires with the placement.
            DocAction::ConsumedScaffolding { sketch, .. } => b.sketch(Removed, *sketch),
        }
        if b.set.full {
            return ChangeSet::everything();
        }
        b.set
    }
}
