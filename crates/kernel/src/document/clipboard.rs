//! Copy, cut and paste of **drawing** (v1.5 decisions D3, KB-46).
//!
//! A clipboard item is a saved [`Document`], and a document already holds
//! world sketches, so the copied drawing travels as the item's world
//! sketches: one per source plane for ordinary drawing, one per detached
//! source sketch ([`Document::extract_clipboard_item_with_drawing`]). No
//! file-format change.
//!
//! A paste of drawing is a **draw at the destination**
//! ([`DrawingInsert::Draw`]): each carried sketch is posed and drawn through
//! the drafting engine on the posed plane, with the participants a draw
//! there would have ([`Document::draft_participants`]), so a shape pasted
//! onto a solid's face imprints it, a shape pasted over other drawing
//! arranges with it, and a shape pasted in space is a free sketch. A
//! detached shape stays detached: it lands as one new detached sketch and
//! arranges with nothing. The test of the rule is that a paste gives what
//! Move with the copy modifier gives for the same selection and
//! displacement.

use super::draft::planes_coincide;
use super::*;
use crate::sketch::{SketchEdgeId, SketchIslandId};

/// The label of the one undo step a paste of drawing records (the app
/// shows its localized twin through the `paste` history kind).
pub(crate) const PASTE_LABEL: &str = "Paste";

/// What a clipboard copy takes from the drawing
/// ([`Document::extract_clipboard_item_with_drawing`]).
#[derive(Debug, Clone, PartialEq)]
pub enum DrawnSource {
    /// A whole free sketch of the world frame, its detached flag kept.
    Sketch(SketchId),
    /// Some islands of a free sketch of the world frame (a ring's hole is
    /// its own island: name it to keep the ring a ring).
    Islands(SketchId, Vec<SketchIslandId>),
    /// The drawing edges a plane's listing names by key
    /// ([`Document::plane_units`] over `plane`, `frame`, `objects`): free
    /// drawing and drawing on solids alike, read through a read-only lift.
    /// Only the world frame (`frame == None`) copies; drawing inside a
    /// component definition is refused as a copy inside a component is.
    PlaneKeys {
        /// The plane the keys were listed on.
        plane: Plane,
        /// The frame they were listed in.
        frame: Option<ComponentId>,
        /// The objects that took part in the listing.
        objects: Vec<ObjectId>,
        /// The listing keys of the edges to copy.
        keys: Vec<u64>,
    },
}

/// What [`Document::insert_document`] does with the item's world sketches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DrawingInsert {
    /// Not carried, counted in [`InsertReport::world_sketches_skipped`]:
    /// a Library insert.
    #[default]
    Skip,
    /// Drawn at the destination: a clipboard paste (module docs).
    Draw,
}

/// One carried sketch a paste drew ([`InsertReport::drawn`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnInsert {
    /// The posed plane it was drawn on.
    pub plane: Plane,
    /// The new detached sketch, for a detached shape; `None` for ordinary
    /// drawing, which arranged with the plane's.
    pub detached: Option<SketchId>,
    /// Every pasted segment, posed (world coordinates): where the pasted
    /// shapes are found again afterwards, by geometry.
    pub anchors: Vec<(Point3, Point3)>,
}

impl Document {
    /// The clipboard copy of a selection of nodes AND drawing:
    /// [`Document::extract_clipboard_item`] of `roots`, with `drawing`
    /// carried as the item's world sketches (module docs) in item
    /// coordinates. The item re-origins to the bottom center of the nodes
    /// and the drawing together (axes at identity), so
    /// [`Document::extract_placement_with_drawing`] and the cursor anchor
    /// agree for drawing-only and mixed copies. Read-only.
    ///
    /// `roots` may be empty when `drawing` is not.
    ///
    /// # Errors
    /// As [`Document::extract_item`], and:
    /// - [`DocumentError::EmptyComponent`] — both are empty, or the drawing
    ///   resolves to no edge and `roots` is empty.
    /// - [`DocumentError::ExplodeSessionScope`] — a component session is
    ///   open, or a source is drawing inside a component definition.
    /// - [`DocumentError::UnknownSketch`] — a stale, deleted sketch.
    /// - [`DocumentError::Sketch`] — a stale island or key
    ///   ([`SketchError::UnknownIsland`], [`SketchError::UnknownEdge`]).
    pub fn extract_clipboard_item_with_drawing(
        &self,
        roots: &[NodeId],
        drawing: &[DrawnSource],
        origin: &str,
    ) -> Result<Document, DocumentError> {
        self.extract_item_origin(roots, drawing, false, Some(origin))
    }

    /// [`Document::extract_placement`] for a copy of nodes and drawing
    /// ([`Document::extract_clipboard_item_with_drawing`]): the item→world
    /// transform that lands a paste exactly over what was copied.
    pub fn extract_placement_with_drawing(
        &self,
        nodes: &[NodeId],
        drawing: &[DrawnSource],
    ) -> Option<Transform> {
        self.extract_placement_drawing(nodes, drawing)
    }

    /// The world sketches of an item that a paste of drawing carries
    /// ([`DrawingInsert::Draw`]): the live free sketches of its world
    /// frame, in id order. A placement ghost draws these too.
    pub fn carried_drawing(&self) -> Vec<SketchId> {
        self.sketches
            .keys()
            .filter(|s| self.is_free_world_sketch(*s))
            .collect()
    }

    /// Whether `s` is a live free sketch of the world frame: not deleted,
    /// and held by no object, group, or definition.
    pub(crate) fn is_free_world_sketch(&self, s: SketchId) -> bool {
        self.sketches.contains_key(s)
            && !self.sketch_tombstoned(&s)
            && !self.object_sketches.contains_key(&s)
            && !self.group_sketches.contains_key(&s)
            && !self.def_sketches.contains_key(&s)
    }

    /// A copy's source sketch: live and of the world frame.
    fn clipboard_source_sketch(&self, s: SketchId) -> Result<&Sketch, DocumentError> {
        if !self.sketches.contains_key(s) || self.sketch_tombstoned(&s) {
            return Err(DocumentError::UnknownSketch);
        }
        if self.def_sketches.contains_key(&s) {
            return Err(DocumentError::ExplodeSessionScope);
        }
        Ok(&self.sketches[s])
    }

    /// The drawing `drawing` names, as world-space sketches: ordinary
    /// drawing merged into one sketch per plane (in first-seen order), then
    /// one sketch per detached source. Sources naming no edge contribute
    /// nothing. Read-only.
    pub(crate) fn clipboard_drawing(
        &self,
        drawing: &[DrawnSource],
    ) -> Result<Vec<Sketch>, DocumentError> {
        let mut plain: Vec<Sketch> = Vec::new();
        let mut detached: Vec<Sketch> = Vec::new();
        for src in drawing {
            let sk = match src {
                DrawnSource::Sketch(s) => {
                    let sk = self.clipboard_source_sketch(*s)?;
                    let edges: Vec<SketchEdgeId> = sk.edges().keys().collect();
                    let mut out = sk.blank_like();
                    sk.replay_edges_into(&edges, &Transform::IDENTITY, &mut out)
                        .map_err(DocumentError::Sketch)?;
                    out
                }
                DrawnSource::Islands(s, islands) => {
                    let sk = self.clipboard_source_sketch(*s)?;
                    sk.rebuild_islands_transformed(islands, &Transform::IDENTITY, sk.plane())
                        .map_err(DocumentError::Sketch)?
                }
                DrawnSource::PlaneKeys {
                    plane,
                    frame,
                    objects,
                    keys,
                } => {
                    if frame.is_some() {
                        return Err(DocumentError::ExplodeSessionScope);
                    }
                    self.plane_drawing_by_keys(*plane, *frame, objects, keys)?
                }
            };
            if sk.edges().is_empty() {
                continue;
            }
            if sk.detached() {
                detached.push(sk);
                continue;
            }
            match plain
                .iter_mut()
                .find(|q| planes_coincide(&q.plane(), &sk.plane()))
            {
                Some(into) => {
                    let edges: Vec<SketchEdgeId> = sk.edges().keys().collect();
                    sk.replay_edges_into(&edges, &Transform::IDENTITY, into)
                        .map_err(DocumentError::Sketch)?;
                }
                None => plain.push(sk),
            }
        }
        plain.extend(detached);
        Ok(plain)
    }

    /// The drawing half of a paste ([`DrawingInsert::Draw`]): each of
    /// `item`'s carried sketches posed by `pose` and drawn at the
    /// destination (module docs), in item order. The caller holds the
    /// history bracket. Returns what was drawn, the change, and whether any
    /// draft recorded a change.
    pub(crate) fn draw_carried(
        &mut self,
        item: &Document,
        carried: &[SketchId],
        pose: &Transform,
        hidden: &BTreeSet<ObjectId>,
    ) -> Result<(Vec<DrawnInsert>, DocChange, bool), DocumentError> {
        let mut drawn = Vec::with_capacity(carried.len());
        let mut change = DocChange::default();
        let mut changed = false;
        for &sid in carried {
            let src = &item.sketches[sid];
            let plane = pose
                .apply_plane_keeping_side(&src.plane())
                .map_err(DocumentError::Transform)?;
            let edges: Vec<SketchEdgeId> = src.edges().keys().collect();
            let depth = self.undo.actions.len();
            let replay = |w: &mut Sketch| src.replay_edges_into(&edges, pose, w);
            let (report, ch) = if src.detached() {
                self.draft_detached_new(plane, None, PASTE_LABEL, replay)?
            } else {
                let objects: Vec<ObjectId> = self
                    .draft_participants(plane, None)
                    .into_iter()
                    .filter(|o| !hidden.contains(o))
                    .collect();
                self.draft(plane, None, &objects, PASTE_LABEL, replay)?
            };
            changed |= self.undo.actions.len() != depth;
            let anchors = src
                .edges()
                .values()
                .map(|e| {
                    (
                        pose.apply_point(src.vertices()[e.from].position),
                        pose.apply_point(src.vertices()[e.to].position),
                    )
                })
                .collect();
            drawn.push(DrawnInsert {
                plane,
                detached: if src.detached() {
                    report.sketches.first().copied()
                } else {
                    None
                },
                anchors,
            });
            merge_change(&mut change, ch);
        }
        Ok((drawn, change, changed))
    }
}

/// Folds `from` into `into`, each id once.
pub(crate) fn merge_change(into: &mut DocChange, from: DocChange) {
    merge_unique(&mut into.objects_touched, from.objects_touched);
    merge_unique(&mut into.sketches_touched, from.sketches_touched);
    merge_unique(&mut into.groups_touched, from.groups_touched);
    merge_unique(&mut into.instances_touched, from.instances_touched);
    merge_unique(&mut into.components_touched, from.components_touched);
    merge_unique(&mut into.guides_touched, from.guides_touched);
}
