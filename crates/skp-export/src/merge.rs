//! Saving a document back over the `.skp` it came from: the model starts
//! as the source file's own, and the document's tree is written into it
//! reusing the source's records for everything that has not changed.
//!
//! - **Kept as the file has them**: styles, rendering and shadow settings,
//!   fonts and annotation defaults, units, classifications, watermarks,
//!   texts and dimensions, every definition (a component SketchUp keeps in
//!   its library stays, placed or not), images, polylines, section planes,
//!   layer colors, materials' undecoded parts, and every dictionary the
//!   document has no say over.
//! - **Reused**: an unchanged Object keeps its group's definition and
//!   placement (pid, GUID) — every soft edge, curve, texture placement,
//!   back material and per-face layer of it; an unchanged group keeps its
//!   whole subtree; an instance of an unchanged definition places the
//!   file's definition, with the paint its placement passed down; the
//!   model's loose geometry stays when every Object standing for it is
//!   unchanged and present.
//! - **Rewritten from the document**: anything changed or new, written as
//!   [`crate::author`] writes it (a changed definition in place of its
//!   file record, keeping its name and GUID), with the persistent ids of
//!   the placements it replaces, so scenes keep naming them.
//! - **Merged**: materials and layers by name (the document's colors,
//!   visibility and attributes win); scenes by name (the document's
//!   camera, hidden nodes, hidden layers and cut; the file's style,
//!   shadows, axes, thumbnail and hidden faces stay); guides and section
//!   planes by position.

use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    AttrTarget, ComponentId, Document, EntityRef, Guide, GuideId, NodeId, Point3, SectionPlaneState,
};
use skpwright::{Model, Owner, PlacedInstance, SceneProperties, Value};

use crate::source::{DefSource, NodeSource, entity};
use crate::{
    Author, Export, ExportError, Frame, Origin, RunBuf, SkpSource, Target, WriteEnv, camera, dicts,
    same_look, same_plane, skp_plane, transform13,
};

/// What the merge knows about the source file.
pub(crate) struct Reuse {
    /// The hatch pattern of each hatched placement of the file, by the
    /// placement's pid: the file names one by its place among the runs'
    /// placements, which the save moves, so the hatches are set aside at
    /// the open and put back by pid ([`Author::instance_hatch_on_source`]).
    pub(crate) hatch_by_pid: BTreeMap<u32, usize>,
    /// Persistent ids of the file's placements the document never read in
    /// ([`SkpSource::unread`]): the file's, carried as they are wherever
    /// their list is rewritten.
    pub(crate) unread: BTreeSet<u32>,
    /// The root list's placements never read in, with their dictionaries.
    pub(crate) carried_root: Vec<(PlacedInstance, Vec<skpwright::Dictionary>)>,
    /// A rewritten definition's placements never read in, by its run.
    pub(crate) carried_runs: BTreeMap<usize, Vec<(PlacedInstance, Vec<skpwright::Dictionary>)>>,
    pub(crate) source: SkpSource,
    /// The source's root list.
    pub(crate) root: usize,
    /// Every placement reached from the root through groups, by pid, with
    /// its world pose (row-major 4×4, meters).
    pub(crate) placements: BTreeMap<u32, (PlacedInstance, [f64; 16])>,
    /// The file's placed images, by pid, wherever they were: what an
    /// image's instance is written back as when its list is rewritten.
    pub(crate) images: BTreeMap<u32, skpwright::ImagePlacement>,
    /// Pids written so far: one placement per pid.
    pub(crate) used: BTreeSet<u32>,
    /// Pids some other entity of the file also carries (a 2017 file reuses
    /// ids inside definitions). The writer would renumber a root placement
    /// that took one; such a placement takes a fresh id here instead, so
    /// the next save knows it.
    pub(crate) shared: BTreeSet<u32>,
    /// Component definitions by name.
    pub(crate) def_by_name: BTreeMap<String, usize>,
    /// Definitions rewritten in place: map index -> their run.
    pub(crate) replace_runs: BTreeMap<u32, usize>,
    /// Source definitions already claimed for an in-place rewrite.
    pub(crate) claimed_defs: BTreeSet<usize>,
    /// The paint a reused definition's placements pass down.
    pub(crate) def_paint: BTreeMap<ComponentId, u16>,
    /// Keep the source's loose geometry as it is.
    pub(crate) keep_loose: bool,
    /// Objects standing for the source's loose geometry that are written
    /// loose again (when it is not kept).
    pub(crate) loose_objects: Vec<kernel::ObjectId>,
    /// What the source's loose faces and edges carried that a document
    /// does not, for the ones written again where they were.
    pub(crate) loose_attrs: LooseAttrs,
    /// The source's definition guides as the document shows them.
    pub(crate) def_guides: DefinitionGuides,
    /// The source's loose geometry is Hew's own (`loose_built`).
    pub(crate) source_loose_built: bool,
    /// Group definitions rewritten in place whose placement moved to the
    /// identity (the file posed the group elsewhere), by map index: the
    /// records left in them — dimensions, texts, guides, guide points,
    /// section planes — were in the old frame, so the document's own are
    /// written instead.
    pub(crate) reframed_defs: BTreeSet<usize>,
}

/// Definition guide lines (point, direction) and points, in the world,
/// each with the map index of its definition.
pub(crate) type DefinitionGuides = (
    Vec<(usize, Point3, kernel::Vec3, kernel::GuideSpan)>,
    Vec<(usize, Point3)>,
);

/// Per-face and per-edge state of the source's loose geometry — layer,
/// hidden, shadows — keyed by where each is (Hew keeps a tag per Object,
/// not per face, and no hidden faces). Rewritten loose geometry takes it
/// back wherever a face or an edge is still where it was.
#[derive(Default)]
pub(crate) struct LooseAttrs {
    faces: BTreeMap<Vec<[i64; 3]>, (u16, bool, bool, bool)>,
    edges: BTreeMap<[[i64; 3]; 2], (u16, bool)>,
}

fn micron(p: [f64; 3]) -> [i64; 3] {
    p.map(|v| (v * 1e6).round() as i64)
}

impl LooseAttrs {
    fn face_key(mesh: &skpwright::Mesh, outer: &[u32]) -> Vec<[i64; 3]> {
        let mut k: Vec<[i64; 3]> = outer
            .iter()
            .map(|&i| micron(mesh.vertices[i as usize]))
            .collect();
        k.sort();
        k
    }

    fn edge_key(mesh: &skpwright::Mesh, a: u32, b: u32) -> [[i64; 3]; 2] {
        let mut k = [
            micron(mesh.vertices[a as usize]),
            micron(mesh.vertices[b as usize]),
        ];
        k.sort();
        k
    }

    pub(crate) fn of(mesh: &skpwright::Mesh) -> Self {
        let mut out = Self::default();
        for f in &mesh.faces {
            out.faces.insert(
                Self::face_key(mesh, &f.outer),
                (f.layer, f.hidden, f.casts_shadows, f.receives_shadows),
            );
        }
        for e in &mesh.edges {
            out.edges
                .insert(Self::edge_key(mesh, e.v0, e.v1), (e.layer, e.hidden));
        }
        out
    }

    /// Faces from `faces` and edges from `edges` on — one Object's, tagged
    /// `layer` and `hidden` or not — take the source's state where they
    /// are still where they were. The Object's tag wins when it no longer
    /// names the one layer its source faces shared (retagged in Hew, or a
    /// tag given to faces that were on several), and hiding the Object
    /// hides them all.
    pub(crate) fn apply(
        &self,
        mesh: &mut skpwright::Mesh,
        faces: usize,
        edges: usize,
        layer: u16,
        hidden: bool,
    ) {
        let found: Vec<Option<(u16, bool, bool, bool)>> = (faces..mesh.faces.len())
            .map(|i| {
                self.faces
                    .get(&Self::face_key(mesh, &mesh.faces[i].outer))
                    .copied()
            })
            .collect();
        let source_layers: BTreeSet<u16> = found.iter().flatten().map(|s| s.0).collect();
        let retagged = match source_layers.len() {
            0 => true,
            1 => source_layers.first() != Some(&layer),
            _ => layer != 0,
        };
        for (i, s) in (faces..mesh.faces.len()).zip(found) {
            let f = &mut mesh.faces[i];
            f.layer = layer;
            if let Some((l, h, casts, receives)) = s {
                if !retagged {
                    f.layer = l;
                }
                f.hidden = h;
                f.casts_shadows = casts;
                f.receives_shadows = receives;
            }
            f.hidden |= hidden;
        }
        for i in edges..mesh.edges.len() {
            let key = Self::edge_key(mesh, mesh.edges[i].v0, mesh.edges[i].v1);
            let e = &mut mesh.edges[i];
            e.layer = layer;
            if let Some(&(l, h)) = self.edges.get(&key) {
                if !retagged {
                    e.layer = l;
                }
                e.hidden = h;
            }
            e.hidden |= hidden;
        }
    }
}

/// A save: the file, and the source the next save builds on.
pub struct Saved {
    pub export: Export,
    pub source: SkpSource,
}

/// Save `doc` as a `.skp` in `target`. With `source` (the file the
/// document was opened from or last saved as, and its provenance) the save
/// builds on that file — see the module docs; without one, or when the
/// source cannot be read, it authors the file from the document alone, as
/// [`crate::export`] does. Either way the result carries the source for the
/// next save.
pub fn save(
    doc: &Document,
    target: Target,
    env: WriteEnv,
    source: Option<&SkpSource>,
) -> Result<Saved, ExportError> {
    save_in(doc, target, env, source, SaveUnits::Keep)
}

/// The display units a save writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SaveUnits {
    /// A save back keeps the file's own units; a file written from the
    /// document alone takes SketchUp's defaults.
    #[default]
    Keep,
    /// A save back keeps the file's own units; a file written from the
    /// document alone takes these (the app's display units).
    New(crate::DisplayUnits),
    /// These, over a file's own units too (asked for by name).
    Set(crate::DisplayUnits),
}

/// [`save`] with the display units the file opens in ([`SaveUnits`]).
pub fn save_in(
    doc: &Document,
    target: Target,
    env: WriteEnv,
    source: Option<&SkpSource>,
    units: SaveUnits,
) -> Result<Saved, ExportError> {
    let doc = doc.session_closed();
    let source_in = source;
    let base = source.and_then(|s| {
        let m = Model::parse(&s.bytes).ok()?;
        // A model read only in part cannot be written back as the whole.
        let partial = m.diagnostics.iter().any(|d| {
            matches!(d, skpwright::Diagnostic::ContinuousFallback { .. }) || d.is_desync()
        });
        // Without provenance nothing can be told unchanged: building on a
        // file with content of its own would keep it beside the document's
        // copy of it. (A file whose content the document could not take —
        // loose edges only, or nothing — has none to double.)
        let known = !s.nodes.is_empty() || !s.defs.is_empty();
        let content = m
            .geometry
            .iter()
            .filter(|r| r.def_index.is_none())
            .any(|r| !r.placed.is_empty() || !r.mesh.faces.is_empty());
        (!partial && (known || !content)).then_some((m, s.clone()))
    });
    let fresh = base.is_none();
    let mut a = match base {
        Some((model, src)) => Author::on_source(&doc, model, src),
        None => Author::new(&doc),
    };
    a.folders = target == Target::Skp2026;
    let mut authored = crate::author_with(a);
    let units = match units {
        SaveUnits::Set(u) => Some(u),
        SaveUnits::New(u) if fresh => Some(u),
        _ => None,
    };
    if let Some(u) = units {
        authored.model.units = u.over(authored.model.units.clone());
    }
    let (bytes, mut writer_notes) = crate::write(&authored.model, target, env)?;
    if let (Some(s), Target::Stream(r)) = (source_in, target) {
        writer_notes.extend(crate::unlisted_note(&s.bytes, r));
    }
    let mut source = SkpSource {
        nodes: authored
            .origins
            .iter()
            .filter_map(|(&sid, &origin)| {
                let node = node_of(&doc, sid)?;
                Some((
                    sid,
                    NodeSource {
                        origin,
                        geometry: match node {
                            NodeId::Object(o) => {
                                doc.object(o).map_or(0, |ob| crate::geometry_hash(&doc, ob))
                            }
                            _ => 0,
                        },
                        node: crate::node_hash(&doc, node),
                    },
                ))
            })
            .collect(),
        defs: authored
            .def_sources
            .iter()
            .filter_map(|(&sid, (name, slot))| {
                let def = component_of(&doc, sid)?;
                Some((
                    sid,
                    DefSource {
                        name: name.clone(),
                        slot: *slot,
                        hash: crate::def_hash(&doc, def),
                    },
                ))
            })
            .collect(),
        loose: authored.loose_written,
        // The placements carried as the file's stay unread for the next
        // save, which cannot tell them from the document's own otherwise.
        unread: Some({
            let written: BTreeSet<u32> = authored
                .model
                .geometry
                .iter()
                .flat_map(|r| r.placed.iter().map(|p| p.pid))
                .collect();
            authored.unread.intersection(&written).copied().collect()
        }),
        // The file's records left in place still stand for the annotations
        // read from them.
        annotations: source
            .map(|s| {
                s.annotations
                    .iter()
                    .filter(|(sid, _)| {
                        crate::source::annotation_of(&doc, **sid)
                            .is_some_and(|id| authored.kept_annotations.contains(&id))
                    })
                    .map(|(&h, a)| (h, a.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        layers: authored.tag_layers.clone(),
        scenes: authored
            .scene_sids
            .iter()
            .enumerate()
            .map(|(i, &sid)| (sid, i))
            .collect(),
        // The file's loose edges still standing for the document's drawing.
        loose_sketches: source
            .filter(|_| authored.loose_kept)
            .map(|s| {
                s.loose_sketches
                    .iter()
                    .filter(|(sid, _)| {
                        doc.sketch_ids()
                            .into_iter()
                            .any(|x| doc.sid_of(&EntityRef::Sketch(x)) == Some(**sid))
                    })
                    .map(|(&sid, &h)| (sid, h))
                    .collect()
            })
            .unwrap_or_default(),
        bytes: bytes.clone(),
    };
    // What the earlier source knew and this save left as it was: a
    // definition reused whole, the definitions and placements inside it,
    // a group put back with its members. This save wrote none of them, so
    // it recorded nothing about them; without the earlier record the next
    // save could not tell an untouched nested definition from a new one
    // and would write it again under another name.
    if let Some(old) = source_in.filter(|_| !fresh) {
        let in_file: BTreeSet<u32> = authored
            .model
            .geometry
            .iter()
            .flat_map(|r| {
                r.placed
                    .iter()
                    .map(|p| p.pid)
                    .chain(r.images.iter().map(|im| im.pid))
            })
            .collect();
        let def_names: BTreeSet<&str> = authored
            .model
            .definitions
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        for (&sid, ds) in &old.defs {
            if !source.defs.contains_key(&sid)
                && component_of(&doc, sid).is_some()
                && def_names.contains(ds.name.as_str())
            {
                source.defs.insert(sid, ds.clone());
            }
        }
        for (&sid, ns) in &old.nodes {
            // (Kept loose geometry records its Objects as it is written.)
            let still_placed = origin_pid(ns.origin).is_some_and(|p| in_file.contains(&p));
            if !source.nodes.contains_key(&sid) && still_placed && node_of(&doc, sid).is_some() {
                source.nodes.insert(sid, ns.clone());
            }
        }
    }
    Ok(Saved {
        export: Export {
            bytes,
            simplified: authored.simplified.into_iter().collect(),
            writer_notes,
            pids: authored.pids,
        },
        source,
    })
}

fn node_of(doc: &Document, sid: u64) -> Option<NodeId> {
    doc.sids()
        .find(|&(_, s)| s == sid)
        .and_then(|(e, _)| match e {
            EntityRef::Object(id) => Some(NodeId::Object(*id)),
            EntityRef::Group(id) => Some(NodeId::Group(*id)),
            EntityRef::Instance(id) => Some(NodeId::Instance(*id)),
            _ => None,
        })
}

fn component_of(doc: &Document, sid: u64) -> Option<ComponentId> {
    doc.sids()
        .find(|&(_, s)| s == sid)
        .and_then(|(e, _)| match e {
            EntityRef::Component(id) => Some(*id),
            _ => None,
        })
}

/// The 13-f64 transform as a row-major 4×4, translation in meters.
fn mat_of(t: &[f64; 13]) -> [f64; 16] {
    let m = 0.0254;
    [
        t[0],
        t[1],
        t[2],
        t[9] * m,
        t[3],
        t[4],
        t[5],
        t[10] * m,
        t[6],
        t[7],
        t[8],
        t[11] * m,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn mat_mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut out = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            out[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    out
}

/// A row-major 4×4 world pose (meters) as a `.skp` 13-f64 transform.
fn transform13_of(w: &[f64; 16]) -> [f64; 13] {
    let rows: [f64; 12] = w[..12].try_into().expect("12 affine entries");
    transform13(&kernel::Transform::from_affine(&rows))
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

impl<'d> Author<'d> {
    /// An author that writes `doc` into `model`, the source file's own,
    /// reusing what `source` says is unchanged.
    pub(crate) fn on_source(doc: &'d Document, mut model: Model, source: SkpSource) -> Self {
        // (Before anything below rewrites the root list.)
        let def_guides = expanded_definition_guides(&model);
        let root = match model.geometry.iter().position(|r| r.def_index.is_none()) {
            Some(r) => r,
            None => {
                model.geometry.push(skpwright::GeometryRun::default());
                model.geometry.len() - 1
            }
        };
        // Placements reachable from the root through groups, with world
        // poses: where an unchanged Object or group is put back.
        let run_of: BTreeMap<usize, usize> = model
            .geometry
            .iter()
            .enumerate()
            .filter_map(|(ri, r)| Some((r.def_index?, ri)))
            .collect();
        let mut placements = BTreeMap::new();
        let mut stack: Vec<(usize, [f64; 16], usize)> = vec![(root, IDENTITY, 0)];
        while let Some((ri, world, depth)) = stack.pop() {
            if depth > 64 {
                continue;
            }
            for p in &model.geometry[ri].placed {
                let w = mat_mul(&world, &mat_of(&p.transform));
                placements.insert(p.pid, (p.clone(), w));
                if p.is_group
                    && let Some(&child) = run_of.get(&(p.defref as usize))
                {
                    stack.push((child, w, depth + 1));
                }
            }
        }
        // Every pid carried by anything but a root placement, and how often
        // root placements carry each.
        let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
        for (ri, r) in model.geometry.iter().enumerate() {
            let placed = r.placed.iter().map(|p| p.pid).filter(|_| ri != root);
            for pid in r
                .mesh
                .vertex_pids
                .iter()
                .copied()
                .chain(r.mesh.edges.iter().map(|e| e.pid))
                .chain(r.mesh.faces.iter().map(|f| f.pid))
                .chain(r.curves.iter().map(|c| c.pid))
                .chain(r.sections.iter().map(|x| x.pid))
                .chain(r.guide_points.iter().map(|x| x.pid))
                .chain(r.polylines.iter().map(|x| x.pid))
                .chain(r.images.iter().map(|x| x.pid))
                .chain(placed)
            {
                *seen.entry(pid).or_insert(0) += 1;
            }
            if ri == root {
                for p in &r.placed {
                    *seen.entry(p.pid).or_insert(0) += 1;
                }
            }
        }
        for pid in model
            .texts
            .iter()
            .map(|x| x.pid)
            .chain(model.dimensions.iter().map(|x| x.pid))
            .chain(model.radial_dimensions.iter().map(|x| x.pid))
            .chain(model.guides.iter().map(|x| x.pid))
        {
            *seen.entry(pid).or_insert(0) += 1;
        }
        let shared: BTreeSet<u32> = seen
            .into_iter()
            .filter(|&(pid, n)| pid != 0 && n > 1)
            .map(|(pid, _)| pid)
            .collect();
        let def_by_name = model
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.kind == skpwright::DefinitionKind::Component)
            .map(|(i, d)| (d.name.clone(), i))
            .collect();
        let source_loose_built = model
            .dictionaries
            .iter()
            .filter(|a| a.owner == Owner::Model)
            .flat_map(|a| a.dictionaries.iter())
            .filter(|d| d.name == dicts::HEW)
            .any(|d| {
                d.entries
                    .iter()
                    .any(|(k, v)| k == "loose_built" && *v == Value::Bool(true))
            });
        let loose_is_sketches = model
            .dictionaries
            .iter()
            .filter(|a| a.owner == Owner::Model)
            .flat_map(|a| a.dictionaries.iter())
            .filter(|d| d.name == dicts::HEW)
            .any(|d| {
                d.entries
                    .iter()
                    .any(|(k, v)| k == "loose" && *v == Value::Text("sketches".into()))
            });
        // The source's loose geometry stays when every Object standing for
        // it is still at the top level, unchanged.
        let loose_now: Vec<kernel::ObjectId> = doc
            .top_level_nodes()
            .into_iter()
            .filter_map(|n| match n {
                NodeId::Object(o) => Some(o),
                _ => None,
            })
            .filter(|&o| {
                let sid = doc.sid_of(&EntityRef::Object(o));
                sid.and_then(|s| source.nodes.get(&s))
                    .is_some_and(|n| n.origin == Origin::Loose)
            })
            .collect();
        let unchanged = |o: kernel::ObjectId| {
            let Some(n) = doc
                .sid_of(&EntityRef::Object(o))
                .and_then(|s| source.nodes.get(&s))
            else {
                return false;
            };
            // The whole node: a loose Object retagged or hidden in place
            // rewrites the loose geometry too (its faces carry both).
            crate::node_hash(doc, NodeId::Object(o)) == n.node
        };
        let has_loose = !model.geometry[root].mesh.faces.is_empty();
        // The file's loose edges the document read as drawing stay with
        // the loose geometry while each is still a free sketch, unchanged.
        let loose_drawing_kept = source.loose_sketches.iter().all(|(&sid, &hash)| {
            doc.sketch_ids().into_iter().any(|s| {
                doc.sid_of(&EntityRef::Sketch(s)) == Some(sid)
                    && doc.sketch_owner_group(s).is_none()
                    && crate::source::sketch_hash(doc, s) == Some(hash)
            })
        });
        // Drawing new at the root is written beside the loose geometry,
        // which then goes as a group per solid (`loose_on_source`): the
        // file's loose geometry is kept only while every free root sketch
        // is one of its own.
        let root_drawing_new = doc.sketch_ids().into_iter().any(|s| {
            doc.sketch_owner_group(s).is_none()
                && !doc
                    .sid_of(&EntityRef::Sketch(s))
                    .is_some_and(|sid| source.loose_sketches.contains_key(&sid))
        });
        let keep_loose = !loose_is_sketches
            && (source.loose > 0 || !has_loose)
            && loose_now.len() == source.loose
            && loose_now.iter().all(|&o| unchanged(o))
            && loose_drawing_kept
            && !root_drawing_new;

        let images: BTreeMap<u32, skpwright::ImagePlacement> = model
            .geometry
            .iter()
            .flat_map(|r| r.images.iter().map(|im| (im.pid, im.clone())))
            .collect();
        let next_pid = max_pid(&model).saturating_add(1);
        let next_def = model
            .definitions
            .iter()
            .filter_map(|d| d.map_index)
            .chain(model.definition_links.iter().map(|l| l.0 as usize))
            .max()
            .unwrap_or(0) as u32
            + 1;

        // The placements the document never read in stay the file's: as
        // an earlier save handed them on, or (first after an open) every
        // placement but those the document's nodes came from.
        let unread = source.unread.clone().unwrap_or_else(|| {
            let read: BTreeSet<u32> = source
                .nodes
                .values()
                .filter_map(|n| origin_pid(n.origin))
                .collect();
            model
                .geometry
                .iter()
                .flat_map(|r| r.placed.iter().map(|p| p.pid))
                .filter(|p| *p != 0 && !read.contains(p))
                .collect()
        });
        let carried_root = carry_unread(&model, root, &unread);

        // What the document rewrites is dropped from the file's records:
        // the root list's placements (rewritten from the tree), its loose
        // geometry when not kept, and the dictionaries hanging on them.
        let hatch_by_pid: BTreeMap<u32, usize> = std::mem::take(&mut model.features.instance_hatch)
            .into_iter()
            .filter_map(|(i, h)| Some((model.instances.get(i)?.pid, h)))
            .filter(|&(pid, _)| pid != 0)
            .collect();
        let root_pids: BTreeSet<u32> = model.geometry[root]
            .placed
            .iter()
            .map(|p| p.pid)
            .filter(|p| !unread.contains(p))
            .collect();
        drop_instances(&mut model, |i| root_pids.contains(&i.pid));
        let loose_attrs = if keep_loose {
            LooseAttrs::default()
        } else {
            LooseAttrs::of(&model.geometry[root].mesh)
        };
        let r = &mut model.geometry[root];
        r.placed.clear();
        if !keep_loose {
            r.mesh = skpwright::Mesh::default();
            r.curves.clear();
            r.curve_members.clear();
            r.relationships.clear();
        }
        model.dictionaries.retain(|a| match a.owner {
            Owner::Instance(ri, _) => ri != root,
            Owner::Face(ri, _)
            | Owner::Edge(ri, _)
            | Owner::Curve(ri, _)
            | Owner::Vertex(ri, _) => ri != root || keep_loose,
            _ => true,
        });
        // (Names live on the instance list, by pid: the root placements'
        // were dropped above and are rewritten with them; the names of
        // placements inside definitions stay with those definitions.)

        let mut a = Author::new(doc);
        a.model = model;
        a.next_pid = next_pid;
        a.next_def = next_def;
        a.reuse = Some(Reuse {
            hatch_by_pid,
            unread,
            carried_root,
            carried_runs: BTreeMap::new(),
            source,
            root,
            placements,
            images,
            used: BTreeSet::new(),
            shared,
            def_by_name,
            replace_runs: BTreeMap::new(),
            claimed_defs: BTreeSet::new(),
            def_paint: BTreeMap::new(),
            keep_loose,
            loose_objects: Vec::new(),
            loose_attrs,
            def_guides,
            source_loose_built,
            reframed_defs: BTreeSet::new(),
        });
        a
    }

    fn source_of(&self, node: NodeId) -> Option<&NodeSource> {
        let sid = self.doc.sid_of(&entity(node))?;
        self.reuse.as_ref()?.source.nodes.get(&sid)
    }

    /// The source pid a node may take back: its placement's, when no other
    /// node has taken it yet.
    pub(crate) fn claim_pid(&mut self, node: NodeId) -> Option<u32> {
        let pid = match self.source_of(node)?.origin {
            Origin::GroupGeometry { pid } | Origin::Group { pid } | Origin::Instance { pid } => pid,
            _ => return None,
        };
        let r = self.reuse.as_mut()?;
        (!r.shared.contains(&pid) && r.used.insert(pid)).then_some(pid)
    }

    /// Put an unchanged node back as the file has it; `true` when done.
    /// Only at the world level, where the file's world poses apply: inside
    /// a definition being rewritten, nodes are rewritten too.
    pub(crate) fn reuse_node(&mut self, run: &mut RunBuf, node: NodeId, frame: Frame) -> bool {
        if !matches!(frame, Frame::World) {
            return false;
        }
        let Some(src) = self.source_of(node).cloned() else {
            return false;
        };
        match (node, src.origin) {
            // Loose geometry still at the top level: kept as the file has
            // it, or written loose again with the rest of it. (Grouped in
            // Hew since, it is written in its group.)
            (NodeId::Object(o), Origin::Loose) if self.doc.node_parent(node).is_none() => {
                let r = self.reuse.as_mut().expect("reuse mode");
                if !r.keep_loose {
                    r.loose_objects.push(o);
                }
                true
            }
            (NodeId::Object(o), Origin::GroupGeometry { pid }) => {
                let unchanged = self
                    .doc
                    .object(o)
                    .is_some_and(|ob| crate::geometry_hash(self.doc, ob) == src.geometry);
                unchanged && self.put_back(run, node, pid)
            }
            (NodeId::Group(_), Origin::Group { pid }) => {
                let unchanged = crate::node_hash(self.doc, node) == src.node;
                unchanged && self.put_back(run, node, pid)
            }
            // A placed image still at the top level stays the file's:
            // moved, tagged and hidden as the document has it when that
            // has changed. (Grouped in Hew since, it is written into its
            // group, `image_into`.)
            (NodeId::Instance(i), Origin::Image { pid })
                if self.doc.node_parent(node).is_none() =>
            {
                let changed = crate::node_hash(self.doc, node) != src.node;
                let pose = self.doc.instance_pose(i);
                let layer = self.node_layer(node);
                let hidden = self.doc.node_user_hidden(node);
                let Some(r) = self.reuse.as_mut() else {
                    return false;
                };
                let root = r.root;
                // Only an image still in the root list is handled here;
                // one inside a group or definition is written back by
                // `image_into` when its list is rewritten.
                if !self.model.geometry[root]
                    .images
                    .iter()
                    .any(|im| im.pid == pid)
                {
                    return false;
                }
                if !r.used.insert(pid) {
                    return false;
                }
                if let Some(im) = self.model.geometry[root]
                    .images
                    .iter_mut()
                    .find(|im| im.pid == pid)
                {
                    if changed {
                        if let Some(pose) = pose {
                            im.transform = transform13(&pose);
                        }
                        im.layer = layer;
                        im.hidden = hidden;
                    }
                    if let Some(sid) = self.doc.sid_of(&entity(node)) {
                        self.pids.insert(sid, pid);
                        self.origins.insert(sid, Origin::Image { pid });
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// An instance read from one of the file's placed images, written
    /// back as that image (its definition, shadows) at the document's pose,
    /// layer and visibility into the list being written — a group or
    /// definition rewritten around it, or a root image grouped since.
    /// `true` when done.
    pub(crate) fn image_into(&mut self, run: &mut RunBuf, node: NodeId) -> bool {
        let NodeId::Instance(i) = node else {
            return false;
        };
        let Some(Origin::Image { pid }) = self.source_of(node).map(|s| s.origin) else {
            return false;
        };
        let Some(pose) = self.doc.instance_pose(i) else {
            return false;
        };
        let layer = self.node_layer(node);
        let hidden = self.doc.node_user_hidden(node);
        let Some(r) = self.reuse.as_mut() else {
            return false;
        };
        let Some(template) = r.images.get(&pid).cloned() else {
            return false;
        };
        if !r.used.insert(pid) {
            return false;
        }
        // The root list's copy, if the image was there, goes with it.
        self.remove_root_images(|im| im.pid == pid);
        run.images.push(skpwright::ImagePlacement {
            pid,
            defref: template.defref,
            transform: transform13(&pose),
            hidden,
            layer,
            casts_shadows: template.casts_shadows,
            receives_shadows: template.receives_shadows,
        });
        if let Some(sid) = self.doc.sid_of(&entity(node)) {
            self.pids.insert(sid, pid);
            self.origins.insert(sid, Origin::Image { pid });
        }
        true
    }

    /// The file's placed images the document read in and has since
    /// deleted go; the rest stay (see [`Author::reuse_node`]).
    pub(crate) fn images_on_source(&mut self) {
        let Some(r) = self.reuse.as_ref() else {
            return;
        };
        let read_in: BTreeSet<u32> = r
            .source
            .nodes
            .values()
            .filter_map(|n| match n.origin {
                Origin::Image { pid } => Some(pid),
                _ => None,
            })
            .collect();
        let gone: BTreeSet<u32> = read_in.difference(&r.used).copied().collect();
        self.remove_root_images(|im| gone.contains(&im.pid));
    }

    /// Removes the root list's placed images `gone` picks. Their attribute
    /// dictionaries are keyed by position in that list, so the removed
    /// images' go and the later ones' keys close up.
    fn remove_root_images(&mut self, gone: impl Fn(&skpwright::ImagePlacement) -> bool) {
        let Some(root) = self.reuse.as_ref().map(|r| r.root) else {
            return;
        };
        let removed: Vec<usize> = self.model.geometry[root]
            .images
            .iter()
            .enumerate()
            .filter(|(_, im)| gone(im))
            .map(|(k, _)| k)
            .collect();
        if removed.is_empty() {
            return;
        }
        self.model.geometry[root].images.retain(|im| !gone(im));
        self.model.dictionaries.retain_mut(|a| match &mut a.owner {
            Owner::Image(x, k) if *x == root => match removed.binary_search(k) {
                Ok(_) => false,
                Err(before) => {
                    *k -= before;
                    true
                }
            },
            _ => true,
        });
    }

    /// Puts the file's instance hatches back on the placements that still
    /// carry their pid, by their place among the runs' placements as the
    /// written file has them; a hatch whose placement is gone goes too.
    pub(crate) fn instance_hatch_on_source(&mut self) {
        let Some(r) = self.reuse.as_ref() else {
            return;
        };
        let patterns = self.model.features.hatch_patterns.len();
        self.model.features.instance_hatch = self
            .model
            .geometry
            .iter()
            .flat_map(|run| run.placed.iter())
            .enumerate()
            .filter_map(|(i, p)| Some((i, *r.hatch_by_pid.get(&p.pid)?)))
            .filter(|&(_, h)| h < patterns)
            .collect();
    }

    /// The file's placement `pid` (its definition, and so its whole
    /// content, as the file has it), at its world pose, with the node's
    /// name, tag, hidden state, paint and attributes.
    fn put_back(&mut self, run: &mut RunBuf, node: NodeId, pid: u32) -> bool {
        // A group put back whole takes its members with it, as the file has
        // them: their ids are the file's, which scenes hiding them name and
        // the next save builds on.
        let mut members: Vec<(u64, Origin)> = Vec::new();
        if let (NodeId::Group(g), Some(r)) = (node, self.reuse.as_ref()) {
            let mut stack = self.doc.group_members(g).unwrap_or_default();
            while let Some(m) = stack.pop() {
                if let NodeId::Group(inner) = m {
                    stack.extend(self.doc.group_members(inner).unwrap_or_default());
                }
                if let Some(sid) = self.doc.sid_of(&entity(m))
                    && let Some(src) = r.source.nodes.get(&sid)
                {
                    members.push((sid, src.origin));
                }
            }
        }
        let Some(r) = self.reuse.as_mut() else {
            return false;
        };
        let Some((placed, world)) = r.placements.get(&pid).cloned() else {
            return false;
        };
        if !r.used.insert(pid) {
            return false;
        }
        let new_pid = if r.shared.contains(&pid) {
            None
        } else {
            Some(pid)
        };
        for &(sid, origin) in &members {
            if let Origin::GroupGeometry { pid }
            | Origin::Group { pid }
            | Origin::Instance { pid } = origin
            {
                r.used.insert(pid);
                self.pids.insert(sid, pid);
            }
            self.origins.insert(sid, origin);
        }
        let material = match node {
            NodeId::Object(o) => self
                .doc
                .object(o)
                .and_then(|ob| self.mat(ob.default_material()))
                .unwrap_or(0),
            _ => placed.material,
        };
        let name = match node {
            NodeId::Object(o) => self.doc.object_name(o).map(str::to_string),
            NodeId::Group(g) => self.doc.group_name(g).map(str::to_string),
            NodeId::Instance(i) => self.doc.instance_name(i).map(str::to_string),
        };
        let pid = new_pid.unwrap_or_else(|| self.pid());
        let fresh = self.placement_with_pid(
            node,
            placed.defref,
            transform13_of(&world),
            placed.is_group,
            material,
            name.as_deref(),
            pid,
        );
        run.placed.push(PlacedInstance {
            // Everything of the file's placement the document has no say
            // over (GUID, shadows, lock) stays.
            guid: placed.guid.clone(),
            casts_shadows: placed.casts_shadows,
            receives_shadows: placed.receives_shadows,
            locked: placed.locked,
            ..fresh
        });
        true
    }

    /// The file's definition for an unchanged Hew definition, when there
    /// is one: its map index (the paint is in `def_paint`).
    pub(crate) fn reuse_component(&mut self, def: ComponentId) -> Option<u32> {
        let sid = self.doc.sid_of(&EntityRef::Component(def))?;
        let r = self.reuse.as_ref()?;
        let ds = r.source.defs.get(&sid)?;
        let di = *r.def_by_name.get(&ds.name)?;
        if crate::def_hash(self.doc, def) != ds.hash {
            return None;
        }
        let map = self.model.definitions[di].map_index? as u32;
        let (name, slot) = (ds.name.clone(), ds.slot);
        let r = self.reuse.as_mut()?;
        r.def_paint.insert(def, slot);
        self.def_sources.insert(sid, (name, slot));
        Some(map)
    }

    /// A changed Hew definition that is the only claimant of its file
    /// definition is rewritten in that definition's place (keeping its
    /// record: name, GUID, behavior, dictionaries the document has no say
    /// over). Returns the map index and the run to replace.
    pub(crate) fn in_place(&mut self, def: ComponentId) -> Option<(u32, usize)> {
        let sid = self.doc.sid_of(&EntityRef::Component(def))?;
        let r = self.reuse.as_ref()?;
        let ds = r.source.defs.get(&sid)?;
        let di = *r.def_by_name.get(&ds.name)?;
        // A file definition several Hew definitions stand for (one per
        // paint its placements pass down) stays as it is for the unchanged
        // ones: an edited one gets a definition of its own.
        let variants = r.source.defs.values().filter(|d| d.name == ds.name).count();
        if r.claimed_defs.contains(&di) || variants > 1 {
            return None;
        }
        let map = self.model.definitions[di].map_index? as u32;
        let ri = self
            .model
            .geometry
            .iter()
            .position(|g| g.def_index == Some(map as usize))?;
        let carried = carry_unread(&self.model, ri, &self.reuse.as_ref()?.unread);
        let r = self.reuse.as_mut()?;
        r.claimed_defs.insert(di);
        r.replace_runs.insert(map, ri);
        r.carried_runs.insert(ri, carried);
        self.drop_run_dictionaries(ri, di);
        if let Some(name) = self.doc.component_name(def).filter(|n| !n.is_empty()) {
            self.model.definitions[di].name = name.to_string();
        }
        Some((map, ri))
    }

    /// A changed Hew group that came from one of the file's group
    /// placements is rewritten in that group's own definition (keeping its
    /// record: name, GUID, dictionaries the document has no say over)
    /// rather than as a new definition beside it. Returns the map index.
    pub(crate) fn group_in_place(&mut self, id: kernel::GroupId) -> Option<u32> {
        let Origin::Group { pid } = self.source_of(NodeId::Group(id))?.origin else {
            return None;
        };
        let r = self.reuse.as_ref()?;
        let (placed, world) = r.placements.get(&pid)?;
        let defref = placed.defref;
        let di = self.model.definitions.iter().position(|d| {
            d.map_index == Some(defref as usize) && d.kind == skpwright::DefinitionKind::Group
        })?;
        if r.claimed_defs.contains(&di) {
            return None;
        }
        // A definition other placements share (a group copied without
        // Make Unique) stays as it is for them: this group gets a
        // definition of its own.
        let placements = self
            .model
            .geometry
            .iter()
            .flat_map(|g| g.placed.iter())
            .filter(|p| p.defref == defref)
            .count();
        if placements > 1 {
            return None;
        }
        let map = defref;
        let ri = self
            .model
            .geometry
            .iter()
            .position(|g| g.def_index == Some(map as usize))?;
        let carried = carry_unread(&self.model, ri, &r.unread);
        // The group is written at the identity with its members in the
        // world: a placement that was posed elsewhere leaves the records
        // kept in its definition in the old frame, so they are not kept.
        let moved = world
            .iter()
            .zip(IDENTITY.iter())
            .any(|(a, b)| (a - b).abs() > 1e-9);
        let r = self.reuse.as_mut()?;
        r.claimed_defs.insert(di);
        r.replace_runs.insert(map, ri);
        r.carried_runs.insert(ri, carried);
        if moved {
            r.reframed_defs.insert(map as usize);
        }
        self.drop_run_dictionaries(ri, di);
        if let Some(name) = self.doc.group_name(id).filter(|n| !n.is_empty()) {
            self.model.definitions[di].name = name.to_string();
        }
        Some(map)
    }

    /// A rewritten run's old dictionaries go with it, and the definition's
    /// own are the document's now.
    fn drop_run_dictionaries(&mut self, ri: usize, di: usize) {
        self.model.dictionaries.retain(|a| match a.owner {
            Owner::Face(x, _)
            | Owner::Edge(x, _)
            | Owner::Curve(x, _)
            | Owner::Vertex(x, _)
            | Owner::Instance(x, _)
            | Owner::Image(x, _)
            | Owner::SectionPlane(x, _)
            | Owner::GuidePoint(x, _)
            | Owner::Polyline(x, _) => x != ri,
            Owner::Definition(x) => x != di,
            _ => true,
        });
    }

    /// A new definition's name, unique among the file's.
    pub(crate) fn unique_def_name(&self, name: String) -> String {
        let taken: BTreeSet<&str> = self
            .model
            .definitions
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        if !taken.contains(name.as_str()) {
            return name;
        }
        (1..)
            .map(|n| format!("{name}#{n}"))
            .find(|c| !taken.contains(c.as_str()))
            .expect("a free name")
    }

    // ── Materials and layers onto the file's ──────────────────────────────

    pub(crate) fn materials_on_source(&mut self) {
        let by_name: BTreeMap<String, usize> = self
            .model
            .materials
            .iter()
            .enumerate()
            .map(|(i, m)| (material_name(m).to_string(), i))
            .collect();
        let mut next_slot = self
            .model
            .material_links
            .iter()
            .map(|l| l.0)
            .max()
            .unwrap_or(0);
        let mats: Vec<(kernel::MaterialId, kernel::Material)> = self
            .doc
            .materials()
            .iter()
            .map(|(id, m)| (id, m.clone()))
            .collect();
        for (id, m) in mats {
            let written = crate::material_record(&m.name, &m);
            let index = match by_name.get(&m.name) {
                Some(&i) => {
                    // The document's color, opacity and texture win; the
                    // rest of the file's record stays.
                    update_material(&mut self.model.materials[i], &written);
                    i
                }
                None => {
                    self.model.materials.push(written);
                    self.model.materials.len() - 1
                }
            };
            self.pbr_on_source(index, m.pbr.as_ref());
            let slot = match self.model.material_links.iter().find(|l| l.1 == index) {
                Some(l) => l.0,
                None => {
                    next_slot += 1;
                    self.model.material_links.push((next_slot, index));
                    next_slot
                }
            };
            self.mat_slot.insert(id, slot);
            let d = dicts::with_unread(
                &self.model.dictionaries,
                &Owner::Material(index),
                self.attrs(AttrTarget::Entity(EntityRef::Material(id))),
            );
            self.model
                .dictionaries
                .retain(|a| a.owner != Owner::Material(index));
            if !d.is_empty() {
                self.model.dictionaries.push(skpwright::Attributes {
                    owner: Owner::Material(index),
                    dictionaries: d,
                });
            }
        }
        self.model.material_links.sort_by_key(|l| l.0);
    }

    /// The document's physically based properties onto the file's record
    /// for material `index`: only what changed, a new record when the file
    /// had none and the document has properties.
    fn pbr_on_source(&mut self, index: usize, doc: Option<&kernel::Pbr>) {
        let list = &mut self.model.features.material_pbr;
        match list.iter_mut().find(|(i, _)| *i == index) {
            Some((_, rec)) => crate::pbr::apply(rec, doc, false),
            None if doc.is_some() => {
                let mut rec = skpwright::PbrMaterial::default();
                crate::pbr::apply(&mut rec, doc, true);
                list.push((index, rec));
            }
            None => {}
        }
    }

    /// The file's layers under the document's tags. A tag stands for the
    /// layer it was read from (by stable id), else for the layer whose
    /// recorded tag path or name it matches, else gets a layer of its own.
    /// A tag renamed in the document renames its layer in place, keeping
    /// the layer's slot and every scene's and entity's reference to it; a
    /// layer no tag stands for any more goes at the end of the save when
    /// nothing written uses it ([`Author::prune_layers_on_source`]).
    pub(crate) fn layers_on_source(&mut self) {
        // A layer stands for the tag path a Hew-written file recorded for
        // it, else its own name.
        let recorded: BTreeMap<String, Vec<String>> = self
            .model
            .dictionaries
            .iter()
            .filter(|a| a.owner == Owner::Model)
            .flat_map(|a| a.dictionaries.iter())
            .filter(|d| d.name == dicts::HEW)
            .flat_map(|d| d.entries.iter())
            .filter(|(k, _)| k == "tag_paths")
            .flat_map(|(_, v)| match v {
                Value::Array(items) => items.clone(),
                _ => Vec::new(),
            })
            .filter_map(|item| match item {
                Value::Array(pair) => match pair.as_slice() {
                    [Value::Text(name), Value::Array(segs)] => Some((
                        name.clone(),
                        segs.iter()
                            .filter_map(|s| match s {
                                Value::Text(t) => Some(t.clone()),
                                _ => None,
                            })
                            .collect(),
                    )),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // (A file's own folders name the rest: the path of their names,
        // then the layer's.)
        let (in_folders, src_pre) = crate::folders::source_paths(&self.model);
        self.src_folder_paths = src_pre;
        let by_path: BTreeMap<Vec<String>, usize> = self
            .model
            .layers
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, l)| {
                (
                    recorded
                        .get(&l.name)
                        .or_else(|| in_folders.get(&l.name))
                        .cloned()
                        .unwrap_or_else(|| vec![l.name.clone()]),
                    i,
                )
            })
            .collect();
        // The layer each tag was read from, by its stable id. Layers are
        // matched by name because the import itself reads a tag per name: two
        // layers of one name (which SketchUp does not normally allow) are one
        // tag in the document, so the first of them stands for it and the
        // other is left to the unused-layer pruning at the end of the save.
        let by_sid: BTreeMap<u64, usize> = self
            .reuse
            .as_ref()
            .map(|r| {
                r.source
                    .layers
                    .iter()
                    .filter_map(|(&sid, name)| {
                        let i = self
                            .model
                            .layers
                            .iter()
                            .skip(1)
                            .position(|l| &l.name == name)?
                            + 1;
                        Some((sid, i))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let tags: Vec<(Vec<String>, bool, Option<u64>)> = self
            .doc
            .tag_meta()
            .map(|(p, hidden)| {
                let sid = self.doc.sid_of(&EntityRef::Tag(p.to_vec()));
                (p.to_vec(), hidden, sid)
            })
            .collect();
        let mut taken: BTreeSet<String> =
            self.model.layers.iter().map(|l| l.name.clone()).collect();
        let mut claimed: BTreeSet<usize> = BTreeSet::new();
        // Each tag's layer: the one it was read from, for every tag that
        // has one, then by path for the rest — so a tag renamed keeps its
        // layer when a new tag has taken its old name.
        let mut found: Vec<Option<usize>> = Vec::with_capacity(tags.len());
        for (_, _, sid) in &tags {
            let i = sid
                .and_then(|s| by_sid.get(&s).copied())
                .filter(|i| claimed.insert(*i));
            found.push(i);
        }
        for (k, (path, _, _)) in tags.iter().enumerate() {
            if found[k].is_none()
                && let Some(&i) = by_path.get(path)
                && claimed.insert(i)
            {
                found[k] = Some(i);
            }
        }
        let mut tag_paths = Vec::new();
        // A layer no tag stands for any more (a deleted tag's, pruned at
        // the end of the save when nothing uses it) gives its name up to a
        // tag that wants it, and takes a numbered one meanwhile.
        for k in 1..self.model.layers.len() {
            if claimed.contains(&k) {
                continue;
            }
            let name = self.model.layers[k].name.clone();
            let wanted_by_tag = tags
                .iter()
                .any(|(path, _, _)| self.layer_name_for(path) == name);
            if wanted_by_tag {
                taken.remove(&name);
                let fresh = crate::unique_name(&format!("{name} (unused)"), "Tag", &mut taken);
                self.model.layers[k].name = fresh;
            }
        }
        // Tags with a layer first, so a renamed layer has given its old
        // name up before a new tag spelled that way takes a layer.
        let mut order: Vec<_> = tags.into_iter().zip(found).collect();
        order.sort_by_key(|(_, found)| found.is_none());
        for ((path, hidden, _), found) in order {
            if path.is_empty() {
                continue;
            }
            let i = match found {
                Some(i) => {
                    self.model.layers[i].visible = !hidden;
                    // A renamed tag renames its layer, in place: the name
                    // its path spells, unless another layer has it.
                    // (With folders, a layer named by the last segment or by
                    // the joined path already stands for its tag.)
                    let wanted = self.layer_name_for(&path);
                    let current = &self.model.layers[i].name;
                    let stands = *current == wanted || (self.folders && *current == path.join("/"));
                    if !stands {
                        taken.remove(&self.model.layers[i].name);
                        let name = crate::unique_name(&wanted, "Tag", &mut taken);
                        self.model.layers[i].name = name;
                    }
                    i
                }
                None => {
                    let name = crate::unique_name(&self.layer_name_for(&path), "Tag", &mut taken);
                    self.new_tag_layers.insert(path.clone());
                    self.model.layers.push(skpwright::Layer {
                        name,
                        visible: !hidden,
                        rgba: crate::layer_color(self.model.layers.len()),
                    });
                    self.model.layers.len() - 1
                }
            };
            claimed.insert(i);
            let slot = match self.model.layer_links.iter().find(|l| l.1 == i) {
                Some(l) => l.0,
                None => {
                    let s = self
                        .model
                        .layer_links
                        .iter()
                        .map(|l| l.0)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    self.model.layer_links.push((s, i));
                    s
                }
            };
            self.layer_slot.insert(path.clone(), slot);
            tag_paths.push(Value::Array(vec![
                Value::Text(self.model.layers[i].name.clone()),
                dicts::path_value(&path),
            ]));
            let d = dicts::with_unread(
                &self.model.dictionaries,
                &Owner::Layer(i),
                self.attrs(AttrTarget::Entity(EntityRef::Tag(path))),
            );
            self.model
                .dictionaries
                .retain(|a| a.owner != Owner::Layer(i));
            if !d.is_empty() {
                self.model.dictionaries.push(skpwright::Attributes {
                    owner: Owner::Layer(i),
                    dictionaries: d,
                });
            }
        }
        if !tag_paths.is_empty() {
            self.model_hew.set("tag_paths", Value::Array(tag_paths));
        }
    }

    /// Drops the file's group definitions nothing places any more (a
    /// group deleted in Hew): a SketchUp group is its one placement, and
    /// its definition is nothing on its own. Component definitions stay
    /// placed or not, as SketchUp keeps them. Runs once everything is
    /// written, and renumbers what refers to definitions and lists by
    /// position (the links and the dictionaries).
    pub(crate) fn prune_unplaced_groups_on_source(&mut self) {
        // A group's definition places the groups nested in it: once it
        // goes, theirs are unplaced too, so the pass repeats until
        // nothing more goes.
        loop {
            let placed: BTreeSet<u32> = self
                .model
                .geometry
                .iter()
                .flat_map(|r| {
                    r.placed
                        .iter()
                        .map(|p| p.defref)
                        .chain(r.images.iter().map(|im| im.defref))
                })
                .collect();
            let gone: Vec<usize> = self
                .model
                .definitions
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.kind == skpwright::DefinitionKind::Group
                        && d.map_index.is_some_and(|m| !placed.contains(&(m as u32)))
                })
                .map(|(i, _)| i)
                .rev()
                .collect();
            if gone.is_empty() {
                return;
            }
            for di in gone {
                self.remove_definition(di);
            }
        }
    }

    /// Removes definition `di` and its list from the model, renumbering
    /// every reference to the ones after it.
    fn remove_definition(&mut self, di: usize) {
        let m = &mut self.model;
        let map = m.definitions[di].map_index;
        m.definitions.remove(di);
        // The features keyed by definition index: a removed definition's
        // snaps and procedure go with it, the rest move down.
        let down = |d: usize| if d > di { d - 1 } else { d };
        m.features.snaps.retain(|s| s.definition != Some(di));
        for s in &mut m.features.snaps {
            s.definition = s.definition.map(down);
        }
        m.features
            .procedures
            .retain(|p| p.definition != di && p.results.iter().all(|r| r.definition != di));
        for p in &mut m.features.procedures {
            p.definition = down(p.definition);
            for r in &mut p.results {
                r.definition = down(r.definition);
            }
        }
        m.definition_links.retain(|l| l.1 != di);
        for l in &mut m.definition_links {
            if l.1 > di {
                l.1 -= 1;
            }
        }
        // The records of its list go with it: annotations and guides in
        // the definition, and the placement entries of its instance list.
        // Their dictionaries are keyed by index, so the survivors'
        // dictionaries move down with them.
        if let Some(map) = map {
            fn drop_where<T>(v: &mut Vec<T>, gone: impl Fn(&T) -> bool) -> Vec<Option<usize>> {
                let mut remap = Vec::with_capacity(v.len());
                let mut next = 0;
                for item in v.iter() {
                    if gone(item) {
                        remap.push(None);
                    } else {
                        remap.push(Some(next));
                        next += 1;
                    }
                }
                let mut i = 0;
                v.retain(|_| {
                    let keep = remap[i].is_some();
                    i += 1;
                    keep
                });
                remap
            }
            let dims = drop_where(&mut m.dimensions, |d| d.def_index == Some(map));
            let radials = drop_where(&mut m.radial_dimensions, |d| d.def_index == Some(map));
            let texts = drop_where(&mut m.texts, |t| t.def_index == Some(map));
            let guides = drop_where(&mut m.guides, |g| g.def_index == Some(map));
            m.dictionaries.retain_mut(|a| {
                let (remap, i) = match a.owner {
                    Owner::Dimension(i) => (&dims, i),
                    Owner::RadialDimension(i) => (&radials, i),
                    Owner::Text(i) => (&texts, i),
                    Owner::Guide(i) => (&guides, i),
                    _ => return true,
                };
                match remap.get(i).copied().flatten() {
                    Some(n) => {
                        a.owner = match a.owner {
                            Owner::Dimension(_) => Owner::Dimension(n),
                            Owner::RadialDimension(_) => Owner::RadialDimension(n),
                            Owner::Text(_) => Owner::Text(n),
                            _ => Owner::Guide(n),
                        };
                        true
                    }
                    None => false,
                }
            });
        }
        if let Some(ri) =
            map.and_then(|map| m.geometry.iter().position(|r| r.def_index == Some(map)))
        {
            let pids: BTreeSet<u32> = m.geometry[ri].placed.iter().map(|p| p.pid).collect();
            drop_instances(m, |i| pids.contains(&i.pid));
            m.geometry.remove(ri);
            m.dictionaries.retain(|a| {
                !matches!(a.owner,
                    Owner::Face(x, _)
                    | Owner::Edge(x, _)
                    | Owner::Curve(x, _)
                    | Owner::Vertex(x, _)
                    | Owner::Instance(x, _)
                    | Owner::Image(x, _)
                    | Owner::SectionPlane(x, _)
                    | Owner::GuidePoint(x, _)
                    | Owner::Polyline(x, _) if x == ri)
            });
            for a in &mut m.dictionaries {
                match &mut a.owner {
                    Owner::Face(x, _)
                    | Owner::Edge(x, _)
                    | Owner::Curve(x, _)
                    | Owner::Vertex(x, _)
                    | Owner::Instance(x, _)
                    | Owner::Image(x, _)
                    | Owner::SectionPlane(x, _)
                    | Owner::GuidePoint(x, _)
                    | Owner::Polyline(x, _)
                        if *x > ri =>
                    {
                        *x -= 1;
                    }
                    _ => {}
                }
            }
        }
        m.dictionaries
            .retain(|a| !matches!(a.owner, Owner::Definition(x) if x == di));
        for a in &mut m.dictionaries {
            if let Owner::Definition(x) = &mut a.owner
                && *x > di
            {
                *x -= 1;
            }
        }
    }

    /// Drops the file's layers no tag of the document stands for (a tag
    /// deleted in Hew; a tag renamed is its layer renamed), when nothing
    /// written uses them — content the document never read in may still
    /// be on one, and then it stays. Runs once everything is written, and
    /// renumbers what refers to layers by position: the slot links,
    /// scenes' hidden-layer lists, and the layers' own dictionaries.
    pub(crate) fn prune_layers_on_source(&mut self) {
        let claimed: BTreeSet<usize> = self
            .layer_slot
            .values()
            .filter_map(|&s| {
                self.model
                    .layer_links
                    .iter()
                    .find(|l| l.0 == s)
                    .map(|l| l.1)
            })
            .collect();
        let m = &self.model;
        let used = crate::folders::used_slots(m);
        let slot_of = |i: usize| m.layer_links.iter().find(|l| l.1 == i).map(|l| l.0);
        let gone: BTreeSet<usize> = (1..m.layers.len())
            .filter(|i| !claimed.contains(i))
            .filter(|&i| slot_of(i).is_none_or(|s| !used.contains(&s)))
            .collect();
        if gone.is_empty() {
            return;
        }
        let mut remap: Vec<Option<usize>> = Vec::with_capacity(m.layers.len());
        let mut next = 0usize;
        for i in 0..m.layers.len() {
            if gone.contains(&i) {
                remap.push(None);
            } else {
                remap.push(Some(next));
                next += 1;
            }
        }
        let m = &mut self.model;
        let layers = std::mem::take(&mut m.layers);
        m.layers = layers
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !gone.contains(i))
            .map(|(_, l)| l)
            .collect();
        m.layer_links = m
            .layer_links
            .iter()
            .filter_map(|&(s, i)| Some((s, remap[i]?)))
            .collect();
        // The active layer follows its layer (a pruned one leaves the
        // writers' default, layer 0).
        m.active_layer = m.active_layer.and_then(|i| remap.get(i).copied().flatten());
        for sc in &mut m.scenes {
            sc.hidden_layers = sc
                .hidden_layers
                .iter()
                .filter_map(|&i| remap.get(i).copied().flatten())
                .collect();
        }
        // The features keyed by layer index follow their layers; a folder
        // loses the layers that are gone.
        fn remap_folders(list: &mut [skpwright::LayerFolder], remap: &[Option<usize>]) {
            for f in list {
                f.layers = f
                    .layers
                    .iter()
                    .filter_map(|&i| remap.get(i).copied().flatten())
                    .collect();
                remap_folders(&mut f.folders, remap);
            }
        }
        remap_folders(&mut m.features.layer_folders, &remap);
        m.features.layer_line_styles = m
            .features
            .layer_line_styles
            .iter()
            .filter_map(|&(l, s)| Some((remap.get(l).copied().flatten()?, s)))
            .collect();
        m.features.layer_page_behaviour = m
            .features
            .layer_page_behaviour
            .iter()
            .filter_map(|&(l, v)| Some((remap.get(l).copied().flatten()?, v)))
            .collect();
        m.dictionaries
            .retain(|a| !matches!(a.owner, Owner::Layer(i) if gone.contains(&i)));
        for a in &mut m.dictionaries {
            if let Owner::Layer(i) = a.owner
                && let Some(Some(n)) = remap.get(i)
            {
                a.owner = Owner::Layer(*n);
            }
        }
    }

    // ── The root list ─────────────────────────────────────────────────────

    /// The root list under construction: the file's loose geometry when it
    /// is kept (new loose geometry welds onto it), else empty.
    pub(crate) fn root_on_source(&mut self) -> RunBuf {
        let r = self.reuse.as_mut().expect("reuse mode");
        let run = &self.model.geometry[r.root];
        let mut buf = RunBuf::default();
        if r.keep_loose {
            buf.seed(&run.mesh, &run.curves);
        }
        for (p, dicts) in std::mem::take(&mut r.carried_root) {
            r.used.insert(p.pid);
            if !dicts.is_empty() {
                self.placement_dicts.push((p.pid, dicts));
            }
            buf.placed.push(p);
        }
        buf
    }

    /// A rewritten definition's placements the document never read in,
    /// put back after its own (see [`Reuse::unread`]).
    pub(crate) fn carried_into(&mut self, ri: usize) -> Vec<PlacedInstance> {
        let Some(r) = self.reuse.as_mut() else {
            return Vec::new();
        };
        let carried = r.carried_runs.remove(&ri).unwrap_or_default();
        let mut out = Vec::with_capacity(carried.len());
        for (p, dicts) in carried {
            r.used.insert(p.pid);
            if !dicts.is_empty() {
                self.placement_dicts.push((p.pid, dicts));
            }
            out.push(p);
        }
        out
    }

    /// The Objects standing for the file's loose geometry, written loose
    /// again (the loose geometry was not kept as it was).
    pub(crate) fn loose_on_source(&mut self, root: &mut RunBuf) {
        let objs = std::mem::take(&mut self.reuse.as_mut().expect("reuse mode").loose_objects);
        // Loose geometry rewritten beside drawing is written as a group
        // per solid, as a new file is: a list whose loose geometry is
        // drawn sketches reads back as those sketches alone.
        let drawing_written = self
            .doc
            .sketch_ids()
            .into_iter()
            .any(|s| self.doc.sketch_owner_group(s).is_none());
        if drawing_written {
            for o in objs {
                if let Some(p) = self.object_group(o, Frame::World) {
                    root.placed.push(p);
                }
            }
            return;
        }
        for o in objs {
            if let Some(obj) = self.doc.object(o) {
                let base = self.mat(obj.default_material());
                let (faces, edges) = (root.mesh.faces.len(), root.mesh.edges.len());
                self.object_into_with_base(root, obj, base);
                // Loose geometry has no placement to carry the Object's tag:
                // its faces and edges carry the layer — each the file's own
                // where it is still where it was.
                let layer = self.node_layer(NodeId::Object(o));
                let hidden = self.doc.node_user_hidden(NodeId::Object(o));
                self.reuse.as_ref().expect("reuse mode").loose_attrs.apply(
                    &mut root.mesh,
                    faces,
                    edges,
                    layer,
                    hidden,
                );
                if let Some(sid) = self.doc.sid_of(&EntityRef::Object(o)) {
                    self.origins.insert(sid, Origin::Loose);
                    self.loose_written += 1;
                }
            }
        }
        if self.reuse.as_ref().is_some_and(|r| r.keep_loose) {
            // Kept loose geometry: its Objects keep their origin.
            let kept: Vec<u64> = self
                .reuse
                .as_ref()
                .expect("reuse mode")
                .source
                .nodes
                .iter()
                .filter(|(_, n)| n.origin == Origin::Loose)
                .map(|(&s, _)| s)
                .collect();
            self.loose_written += kept.len();
            for s in kept {
                self.origins.insert(s, Origin::Loose);
            }
        }
    }

    /// Guides: the file's model-level guides the document still has stay
    /// (with their dictionaries), the others go; a definition's guides
    /// stay (the document sees them once per placement); the document's
    /// new guides are added. Guide points likewise, in the root list.
    pub(crate) fn guides_on_source(&mut self, root: &mut RunBuf) {
        let doc_guide_ids: Vec<GuideId> = self
            .doc
            .guide_ids()
            .into_iter()
            .filter(|&g| self.doc.guide(g).is_some())
            .collect();
        let doc_guides: Vec<Guide> = doc_guide_ids
            .iter()
            .filter_map(|&g| self.doc.guide(g).copied())
            .collect();
        // Each document guide's layer: its first tag's. A file guide the
        // document still has goes on that layer (a tag deleted or renamed in
        // Hew moves the guide), and a file guide on the line of two document
        // guides takes the one on its own layer first.
        let doc_layers: Vec<u16> = doc_guide_ids.iter().map(|&g| self.guide_layer(g)).collect();
        // Each document guide's dash: what the file's guide is drawn with
        // once matched (the document carries the file's stipple from
        // opening, so an untouched guide keeps its own).
        let doc_dashes: Vec<u16> = doc_guide_ids
            .iter()
            .map(|&g| self.doc.guide_dash(g).mask())
            .collect();
        let r = self.reuse.as_ref().expect("reuse mode");
        let ri = r.root;
        let near = |a: Point3, b: [f64; 3]| (a - Point3::new(b[0], b[1], b[2])).length() < 1e-6;
        let line_matches = |g: &skpwright::Guide, h: &Guide| match *h {
            Guide::Line {
                origin,
                direction,
                span,
            } => {
                let d = kernel::Vec3::new(g.direction[0], g.direction[1], g.direction[2]);
                let p = Point3::new(g.point_m[0], g.point_m[1], g.point_m[2]);
                d.normalized().is_ok_and(|u| {
                    crate::same_guide_line(
                        p,
                        u,
                        crate::file_guide_span(g.bounds, d.length()),
                        origin,
                        direction,
                        span,
                    )
                })
            }
            Guide::Point { .. } => false,
        };
        // Which of the document's guides the file already has: model-level
        // ones matched one to one, definition guides seen through every
        // placement of the definition.
        let mut matched = vec![false; doc_guides.len()];
        // (A reframed group definition's guides are not in the file any
        // more: the document's are written as its own.)
        let reframed = r.reframed_defs.clone();
        let (mut expanded, mut expanded_points) = r.def_guides.clone();
        expanded.retain(|(m, _, _, _)| !reframed.contains(m));
        expanded_points.retain(|(m, _)| !reframed.contains(m));
        for (hi, h) in doc_guides.iter().enumerate() {
            match *h {
                // One document line per placement of a definition's line
                // (a model-level guide on the same line is its own).
                Guide::Line {
                    origin,
                    direction,
                    span,
                } => {
                    if let Some(k) = expanded.iter().position(|&(_, p, d, s)| {
                        crate::same_guide_line(p, d, s, origin, direction, span)
                    }) {
                        expanded.swap_remove(k);
                        matched[hi] = true;
                    }
                }
                // One document point per placement of a definition's point.
                Guide::Point { position } => {
                    if let Some(k) = expanded_points
                        .iter()
                        .position(|(_, p)| (position - *p).length() < 1e-6)
                    {
                        expanded_points.swap_remove(k);
                        matched[hi] = true;
                    }
                }
            }
        }
        // A guide hidden with Edit > Hide was never in the document (a
        // Hew guide has no hidden state; one on a hidden layer opens tagged
        // and is matched like any other): the file keeps it as it is.
        let unseen = |_layer: u16, hidden: bool| hidden;
        let mut keep: Vec<bool> = Vec::new();
        let mut relayer: Vec<(usize, u16, u16)> = Vec::new();
        for (gi, g) in self.model.guides.iter().enumerate() {
            if let Some(m) = g.def_index {
                keep.push(!reframed.contains(&m));
                continue;
            }
            // (Kept before any matching: an unseen guide on the line of a
            // seen one, or of one drawn in Hew, must not take its match.)
            if unseen(g.layer, g.hidden) {
                keep.push(true);
                continue;
            }
            let candidates = || {
                doc_guides
                    .iter()
                    .enumerate()
                    .filter(|(hi, h)| !matched[*hi] && line_matches(g, h))
            };
            let hit = candidates()
                .find(|(hi, _)| doc_layers[*hi] == g.layer)
                .or_else(|| candidates().next());
            match hit {
                Some((hi, _)) => {
                    matched[hi] = true;
                    relayer.push((gi, doc_layers[hi], doc_dashes[hi]));
                    keep.push(true);
                }
                None => keep.push(false),
            }
        }
        for (gi, layer, stipple) in relayer {
            self.model.guides[gi].layer = layer;
            self.model.guides[gi].stipple = stipple;
        }
        // Guide points among the root list's own.
        let points = std::mem::take(&mut self.model.geometry[ri].guide_points);
        for mut gp in points {
            if unseen(gp.layer, gp.hidden) {
                root.guide_points.push(gp);
                continue;
            }
            let candidates = || {
                doc_guides.iter().enumerate().filter(|(hi, h)| {
                    !matched[*hi]
                        && matches!(**h, Guide::Point { position } if near(position, gp.point_m))
                })
            };
            let hit = candidates()
                .find(|(hi, _)| doc_layers[*hi] == gp.layer)
                .or_else(|| candidates().next());
            if let Some((hi, _)) = hit {
                matched[hi] = true;
                gp.layer = doc_layers[hi];
                root.guide_points.push(gp);
            }
        }
        // Drop the unmatched model-level guides, remapping the guide
        // dictionaries' indices.
        let mut new_index = Vec::with_capacity(keep.len());
        let mut n = 0;
        for &k in &keep {
            new_index.push(k.then_some(n));
            if k {
                n += 1;
            }
        }
        let mut i = 0;
        self.model.guides.retain(|_| {
            let k = keep[i];
            i += 1;
            k
        });
        self.model.dictionaries.retain_mut(|a| match a.owner {
            Owner::Guide(g) => match new_index.get(g).copied().flatten() {
                Some(ng) => {
                    a.owner = Owner::Guide(ng);
                    true
                }
                None => false,
            },
            Owner::GuidePoint(x, _) => x != ri,
            _ => true,
        });
        // The document's new guides.
        for (hi, h) in doc_guides.into_iter().enumerate() {
            if matched[hi] {
                continue;
            }
            let pid = self.pid();
            // (On its first tag's layer, as a fresh write puts it.)
            let layer = doc_layers[hi];
            match h {
                Guide::Line {
                    origin,
                    direction,
                    span,
                } => self.model.guides.push(skpwright::Guide {
                    def_index: None,
                    point_m: [origin.x, origin.y, origin.z],
                    direction: [direction.x, direction.y, direction.z],
                    pid,
                    bounds: crate::guide_bounds(span),
                    layer,
                    material: None,
                    hidden: false,
                    casts_shadows: true,
                    receives_shadows: true,
                    stipple: doc_dashes[hi],
                }),
                Guide::Point { position } => {
                    let p = [position.x, position.y, position.z];
                    root.guide_points.push(skpwright::GuidePoint {
                        pid,
                        point_m: p,
                        anchor_m: p,
                        layer,
                        ..Default::default()
                    });
                }
            }
        }
    }

    /// Section planes: the file's stay; the document's cut (and each
    /// scene's) names a file plane at the same place, or a new one.
    pub(crate) fn sections_on_source(
        &mut self,
        root: &mut RunBuf,
    ) -> Vec<(SectionPlaneState, u32)> {
        let ri = self.reuse.as_ref().expect("reuse mode").root;
        root.sections = std::mem::take(&mut self.model.geometry[ri].sections);
        let mut planes: Vec<(SectionPlaneState, u32)> = Vec::new();
        // A file plane is the cut when it holds the cut's origin and keeps
        // the side the cut keeps (its normal is the cut's turned over).
        let file_plane = |sp: &skpwright::SectionPlane, s: &SectionPlaneState| {
            let [a, b, c, d] = sp.plane;
            let n = kernel::Vec3::new(a, b, c);
            let on = (n.dot(s.origin.to_vec()) * 39.37007874015748 + d).abs() < 1e-4;
            on && (n + s.normal).length() < 1e-6
        };
        let doc_plane = self.doc.section_plane();
        let scene_planes: Vec<SectionPlaneState> = self
            .doc
            .scenes()
            .iter()
            .filter_map(|s| s.section.flatten())
            .collect();
        for s in doc_plane.iter().chain(scene_planes.iter()) {
            if planes.iter().any(|(p, _)| same_plane(p, s)) {
                continue;
            }
            let pid = match root.sections.iter().find(|sp| file_plane(sp, s)) {
                Some(sp) => sp.pid,
                None => {
                    let pid = self.pid();
                    root.sections.push(skpwright::SectionPlane {
                        pid,
                        plane: skp_plane(s),
                        ..Default::default()
                    });
                    pid
                }
            };
            planes.push((*s, pid));
        }
        root.active_section = doc_plane.filter(|s| s.active).and_then(|s| {
            planes
                .iter()
                .find(|(p, _)| same_plane(p, &s))
                .map(|&(_, pid)| pid)
        });
        planes
    }

    /// Scenes, by identity (by name for one the source has no record of): the document's camera, hidden nodes, hidden layers
    /// and cut over the file's scene of the same name (whose style,
    /// shadows, axes, thumbnail and hidden faces and edges stay); the
    /// document's order; a scene the document dropped goes.
    pub(crate) fn scenes_on_source(&mut self, planes: &[(SectionPlaneState, u32)]) {
        let file_scenes = std::mem::take(&mut self.model.scenes);
        // Pids the document's tree speaks for: a file scene's hidden pid
        // that names none of them (a face, an edge) stays hidden.
        let node_pids: BTreeSet<u32> = self
            .reuse
            .as_ref()
            .expect("reuse mode")
            .placements
            .keys()
            .copied()
            .chain(self.pids.values().copied())
            .collect();
        self.scenes(planes);
        let authored = std::mem::take(&mut self.model.scenes);
        // The file scene each written scene stands for: by identity (the
        // scene the document's own came from), else by name for a scene
        // the source has no record of. A file scene the source accounts
        // for goes only to its own scene: one the document deleted donates
        // nothing to a new scene that reuses its name.
        let src = &self.reuse.as_ref().expect("reuse mode").source;
        let accounted: BTreeSet<usize> = src.scenes.values().copied().collect();
        let mut taken: BTreeSet<usize> = BTreeSet::new();
        let mut matched: Vec<Option<usize>> = vec![None; authored.len()];
        for (k, m) in matched.iter_mut().enumerate() {
            if let Some(&fi) = self.scene_sids.get(k).and_then(|sid| src.scenes.get(sid))
                && fi < file_scenes.len()
                && taken.insert(fi)
            {
                *m = Some(fi);
            }
        }
        for (k, m) in matched.iter_mut().enumerate() {
            let known = self
                .scene_sids
                .get(k)
                .is_some_and(|sid| src.scenes.contains_key(sid));
            if m.is_some() || known {
                continue;
            }
            if let Some(fi) = file_scenes.iter().enumerate().position(|(fi, f)| {
                f.name == authored[k].name && !accounted.contains(&fi) && !taken.contains(&fi)
            }) {
                taken.insert(fi);
                *m = Some(fi);
            }
        }
        // Where each file scene lands in the written list, for what is
        // keyed by scene index.
        let mut landing: Vec<Option<usize>> = vec![None; file_scenes.len()];
        for (k, m) in matched.iter().enumerate() {
            if let Some(fi) = *m {
                landing[fi] = Some(k);
            }
        }
        for (k, mut s) in authored.into_iter().enumerate() {
            if let Some(f) = matched[k].map(|fi| &file_scenes[fi]) {
                let mut merged = f.clone();
                merged.name = s.name.clone();
                // The properties the document captures, in the file's own
                // terms: a hidden set the file already saves keeps the
                // file's bits for it (2017 has one, 2026 two).
                let ours = s.saved;
                let mut bits = f.saved.0;
                let mut set = |bit: u32, on: bool| {
                    if on {
                        bits |= bit;
                    } else {
                        bits &= !bit;
                    }
                };
                set(SceneProperties::CAMERA, ours.has(SceneProperties::CAMERA));
                set(
                    SceneProperties::VISIBLE_LAYERS,
                    ours.has(SceneProperties::VISIBLE_LAYERS),
                );
                set(
                    SceneProperties::ACTIVE_SECTION_PLANES,
                    ours.has(SceneProperties::ACTIVE_SECTION_PLANES),
                );
                set(
                    SceneProperties::STYLE_AND_FOG,
                    ours.has(SceneProperties::STYLE_AND_FOG),
                );
                // The scene's look as the document has it goes onto the
                // file's style for the scene (the rest of it stays).
                if let Some(theirs) = s.rendering.take() {
                    match &mut merged.rendering {
                        Some(r) => {
                            r.background = theirs.background;
                            r.sky = theirs.sky;
                            r.sky_color = theirs.sky_color;
                            r.ground = theirs.ground;
                            r.ground_color = theirs.ground_color;
                            r.model_axes = theirs.model_axes;
                        }
                        None => merged.rendering = Some(theirs),
                    }
                }
                let hidden = SceneProperties::HIDDEN_GEOMETRY
                    | SceneProperties::HIDDEN_GEOMETRY_2
                    | SceneProperties::HIDDEN_OBJECTS;
                if !ours.has(SceneProperties::HIDDEN_GEOMETRY) {
                    bits &= !hidden;
                } else if bits & hidden == 0 {
                    bits |= SceneProperties::HIDDEN_GEOMETRY;
                }
                merged.saved = SceneProperties(bits);
                merged.description = std::mem::take(&mut s.description);
                // The scene's own camera, with what Hew holds no state for
                // (two-point perspective, 2D zoom and pan) kept while its
                // view is the file's own.
                let file_camera = merged.camera.take();
                merged.camera = s.camera.take();
                if let (Some(old), Some(new)) = (file_camera.as_ref(), merged.camera.as_mut())
                    && same_look(old, new)
                {
                    new.two_point_perspective = old.two_point_perspective;
                    new.scale_2d = old.scale_2d;
                    new.offset_2d = old.offset_2d;
                    new.description = old.description.clone();
                    new.undecoded = old.undecoded;
                }
                let kept: Vec<u32> = f
                    .hidden_entities
                    .iter()
                    .copied()
                    .filter(|p| !node_pids.contains(p))
                    .collect();
                merged.hidden_entities = kept
                    .into_iter()
                    .chain(s.hidden_entities.drain(..))
                    .collect();
                merged.hidden_layers = std::mem::take(&mut s.hidden_layers);
                merged.active_section_planes = std::mem::take(&mut s.active_section_planes);
                self.model.scenes.push(merged);
            } else {
                self.model.scenes.push(s);
            }
        }
        let m = &mut self.model;
        m.features.scene_pages = m
            .features
            .scene_pages
            .iter()
            .filter_map(|(i, e)| Some((landing.get(*i).copied().flatten()?, e.clone())))
            .collect();
        m.features.scene_environment = m
            .features
            .scene_environment
            .iter()
            .filter_map(|(i, e)| Some((landing.get(*i).copied().flatten()?, e.clone())))
            .collect();
        m.dictionaries.retain(|a| match a.owner {
            Owner::Scene(i)
            | Owner::SceneRendering(i)
            | Owner::SceneShadowInfo(i)
            | Owner::SceneAxes(i) => landing.get(i).copied().flatten().is_some(),
            _ => true,
        });
        for a in &mut m.dictionaries {
            match a.owner {
                Owner::Scene(i) => a.owner = Owner::Scene(landing[i].unwrap_or(i)),
                Owner::SceneRendering(i) => {
                    a.owner = Owner::SceneRendering(landing[i].unwrap_or(i))
                }
                Owner::SceneShadowInfo(i) => {
                    a.owner = Owner::SceneShadowInfo(landing[i].unwrap_or(i))
                }
                Owner::SceneAxes(i) => a.owner = Owner::SceneAxes(landing[i].unwrap_or(i)),
                _ => {}
            }
        }
        m.selected_scene = m
            .selected_scene
            .and_then(|i| landing.get(i).copied().flatten());
    }

    /// The model's dictionaries: the file's own (SketchUp's and anything
    /// the document does not hold) with the document's attributes over
    /// them.
    pub(crate) fn model_dictionaries_on_source(&mut self) {
        let doc_attrs = self.attrs(AttrTarget::Document);
        let ours: BTreeSet<String> = doc_attrs.iter().map(|d| d.name.clone()).collect();
        for a in self
            .model
            .dictionaries
            .iter_mut()
            .filter(|a| a.owner == Owner::Model)
        {
            // What the document imported from the file it now owns: a
            // dictionary it no longer has was deleted in it.
            a.dictionaries.retain(|d| {
                d.name != dicts::HEW && !ours.contains(&d.name) && is_sketchup_owned(&d.name)
            });
        }
        self.model
            .dictionaries
            .retain(|a| a.owner != Owner::Model || !a.dictionaries.is_empty());
        if !doc_attrs.is_empty() {
            self.model.dictionaries.push(skpwright::Attributes {
                owner: Owner::Model,
                dictionaries: doc_attrs,
            });
        }
    }

    /// The file's dimensions and texts Hew wrote (marked in their `hew`
    /// dictionary) go: the document's own annotations are written in their
    /// place. SketchUp's own stay as the file has them, except one the
    /// document read in and has since changed or deleted, which goes too
    /// (a changed one is written from the document); one it read in and
    /// left alone stays the file's and is not written again.
    pub(crate) fn annotations_on_source(&mut self, root: &RunBuf) {
        let marked = |m: &Model, owner: Owner| {
            m.dictionaries
                .iter()
                .filter(|a| a.owner == owner)
                .flat_map(|a| a.dictionaries.iter())
                .any(|d| d.name == dicts::HEW && d.entries.iter().any(|(k, _)| k == "annotation"))
        };
        // Where an anchor lands in what the file will hold: from the root
        // list as it is being rewritten, through the placements its path
        // names (each into the run its definition now has), to the list
        // holding its entity, as a writer resolves it; a run no placement
        // reaches (a rewritten definition's old record) holds nothing.
        let model = &self.model;
        let at_root = self.reuse.as_ref().map(|r| r.root);
        let run_of_ref = |defref: u32| -> Option<usize> {
            let &(_, di) = model.definition_links.iter().find(|l| l.0 == defref)?;
            let map = model.definitions.get(di)?.map_index?;
            model
                .geometry
                .iter()
                .position(|r| r.def_index == Some(map))
                .filter(|&ri| Some(ri) != at_root)
        };
        let placed = |list: Option<usize>| -> &[PlacedInstance] {
            match list {
                None => &root.placed,
                Some(ri) => &model.geometry[ri].placed,
            }
        };
        let holds = |list: Option<usize>, e: u32| {
            let (mesh, curves) = match list {
                None => (&root.mesh, &root.curves[..]),
                Some(ri) => (&model.geometry[ri].mesh, &model.geometry[ri].curves[..]),
            };
            mesh.vertex_pids.contains(&e)
                || mesh.edges.iter().any(|x| x.pid == e)
                || mesh.faces.iter().any(|x| x.pid == e)
                || curves.iter().any(|x| x.pid == e)
        };
        let mut reachable: BTreeSet<usize> = BTreeSet::new();
        let mut todo = vec![None];
        while let Some(list) = todo.pop() {
            for p in placed(list) {
                if let Some(ri) = run_of_ref(p.defref)
                    && reachable.insert(ri)
                {
                    todo.push(Some(ri));
                }
            }
        }
        let lands = |entity: Option<u32>, path: &[u32]| -> bool {
            let Some(e) = entity else {
                return path.is_empty();
            };
            let mut list = None;
            for &step in path {
                let Some(next) = placed(list)
                    .iter()
                    .find(|p| p.pid == step)
                    .and_then(|p| run_of_ref(p.defref))
                else {
                    return false;
                };
                list = Some(next);
            }
            // With no path a writer looks in the root list, then through
            // its placements.
            holds(list, e) || (path.is_empty() && reachable.iter().any(|&ri| holds(Some(ri), e)))
        };
        let held = |a: &skpwright::Anchor| {
            (a.entity.is_none() || lands(a.entity, &a.path))
                && (a.second.is_none() || lands(a.second, &a.second_path))
        };
        // A record in a group definition rewritten at the identity from
        // elsewhere is in the old frame: the document's own is written.
        let reframed = self
            .reuse
            .as_ref()
            .map(|r| r.reframed_defs.clone())
            .unwrap_or_default();
        // A group definition nothing places any more goes at the end of
        // the save (`prune_unplaced_groups_on_source`), its records with
        // it: an annotation kept there would be written nowhere.
        let placed_now: BTreeSet<u32> = model
            .geometry
            .iter()
            .flat_map(|r| r.placed.iter().map(|p| p.defref))
            .chain(root.placed.iter().map(|p| p.defref))
            .collect();
        let pruned: BTreeSet<usize> = model
            .definitions
            .iter()
            .filter(|d| d.kind == skpwright::DefinitionKind::Group)
            .filter_map(|d| d.map_index)
            .filter(|m| !placed_now.contains(&(*m as u32)))
            .collect();
        let in_frame = |def_index: Option<usize>| {
            def_index.is_none_or(|m| !reframed.contains(&m) && !pruned.contains(&m))
        };
        let anchored = |key: (u8, u32)| -> bool {
            match key {
                (0, pid) => model
                    .texts
                    .iter()
                    .filter(|t| t.pid == pid)
                    .all(|t| in_frame(t.def_index) && held(&t.anchor)),
                (1, pid) => model
                    .dimensions
                    .iter()
                    .filter(|d| d.pid == pid)
                    .all(|d| in_frame(d.def_index) && held(&d.start) && held(&d.end)),
                // A radius dimension hangs on an edge of its own list.
                (_, pid) => model
                    .radial_dimensions
                    .iter()
                    .filter(|d| d.pid == pid)
                    .all(|d| {
                        let list = d.def_index.and_then(|map| {
                            model
                                .geometry
                                .iter()
                                .position(|r| r.def_index == Some(map))
                                .filter(|ri| reachable.contains(ri))
                        });
                        in_frame(d.def_index)
                            && match (d.def_index, list, d.edge) {
                                (_, _, None) => true,
                                (None, _, Some(e)) => holds(None, e),
                                (Some(_), Some(ri), Some(e)) => holds(Some(ri), e),
                                (Some(_), None, Some(_)) => false,
                            }
                    }),
            }
        };
        // The file's records the document read in, by list and pid: kept
        // when the annotation is unchanged and what it is attached to is
        // still written, dropped otherwise (the document writes its own:
        // a record whose entity is gone would be left out, or attached
        // elsewhere).
        let mut dropped: BTreeSet<(u8, u32)> = BTreeSet::new();
        let read_in: Vec<(u64, crate::source::AnnotationSource)> = self
            .reuse
            .as_ref()
            .map(|r| {
                r.source
                    .annotations
                    .iter()
                    .map(|(&h, a)| (h, a.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (sid, src) in read_in {
            let key = match src.origin {
                skp_import::AnnotationOrigin::Text { pid, .. } => (0, pid),
                skp_import::AnnotationOrigin::Dimension { pid, .. } => (1, pid),
                skp_import::AnnotationOrigin::RadialDimension { pid, .. } => (2, pid),
            };
            let kept = crate::source::annotation_of(self.doc, sid).filter(|&id| {
                crate::source::annotation_hash(self.doc, id) == src.hash && anchored(key)
            });
            match kept {
                Some(id) => {
                    self.kept_annotations.insert(id);
                }
                None => {
                    dropped.insert(key);
                }
            }
        }
        // Index maps for the survivors, per list.
        let keep_texts: Vec<bool> = (0..self.model.texts.len())
            .map(|i| {
                !marked(&self.model, Owner::Text(i))
                    && !dropped.contains(&(0, self.model.texts[i].pid))
            })
            .collect();
        let keep_dims: Vec<bool> = (0..self.model.dimensions.len())
            .map(|i| {
                !marked(&self.model, Owner::Dimension(i))
                    && !dropped.contains(&(1, self.model.dimensions[i].pid))
            })
            .collect();
        let keep_radials: Vec<bool> = (0..self.model.radial_dimensions.len())
            .map(|i| {
                !marked(&self.model, Owner::RadialDimension(i))
                    && !dropped.contains(&(2, self.model.radial_dimensions[i].pid))
            })
            .collect();
        let remap = |keep: &[bool]| -> Vec<Option<usize>> {
            let mut n = 0;
            keep.iter()
                .map(|&k| {
                    let r = k.then_some(n);
                    n += usize::from(k);
                    r
                })
                .collect()
        };
        let (rt, rd, rr) = (remap(&keep_texts), remap(&keep_dims), remap(&keep_radials));
        fn retain<T>(v: &mut Vec<T>, keep: &[bool]) {
            let mut i = 0;
            v.retain(|_| {
                let k = keep[i];
                i += 1;
                k
            });
        }
        retain(&mut self.model.texts, &keep_texts);
        retain(&mut self.model.dimensions, &keep_dims);
        retain(&mut self.model.radial_dimensions, &keep_radials);
        self.model.dictionaries.retain_mut(|a| {
            let (map, i) = match a.owner {
                Owner::Text(i) => (&rt, i),
                Owner::Dimension(i) => (&rd, i),
                Owner::RadialDimension(i) => (&rr, i),
                _ => return true,
            };
            match map.get(i).copied().flatten() {
                Some(n) => {
                    a.owner = match a.owner {
                        Owner::Text(_) => Owner::Text(n),
                        Owner::Dimension(_) => Owner::Dimension(n),
                        _ => Owner::RadialDimension(n),
                    };
                    true
                }
                None => false,
            }
        });
    }

    pub(crate) fn camera_on_source(&mut self) {
        if let Some(c) = self.doc.camera_state() {
            let mut cam = camera(&c);
            // The file's camera keeps what Hew holds no state for (two-point
            // perspective, the 2D zoom and pan, the description) for as long
            // as the view is the file's own; a camera the user moved starts
            // again from the defaults, with no image width or ratio unless
            // the document still holds them.
            if let Some(file) = self.model.camera.as_ref().filter(|f| same_look(f, &cam)) {
                cam.two_point_perspective = file.two_point_perspective;
                cam.scale_2d = file.scale_2d;
                cam.offset_2d = file.offset_2d;
                cam.description = file.description.clone();
                cam.undecoded = file.undecoded;
            }
            self.model.camera = Some(cam);
        } else {
            self.model.camera = None;
        }
    }
}

/// The placement persistent id an origin names, if any.
fn origin_pid(o: Origin) -> Option<u32> {
    match o {
        Origin::GroupGeometry { pid }
        | Origin::GroupOwnGeometry { pid }
        | Origin::Group { pid }
        | Origin::Instance { pid }
        | Origin::Image { pid } => Some(pid),
        Origin::Loose | Origin::DefinitionGeometry { .. } => None,
    }
}

/// The placements of run `ri` the document never read in, each with its
/// dictionaries.
fn carry_unread(
    model: &Model,
    ri: usize,
    unread: &BTreeSet<u32>,
) -> Vec<(PlacedInstance, Vec<skpwright::Dictionary>)> {
    model.geometry[ri]
        .placed
        .iter()
        .enumerate()
        .filter(|(_, p)| unread.contains(&p.pid))
        .map(|(pi, p)| {
            let dicts = model
                .dictionaries
                .iter()
                .filter(|a| a.owner == Owner::Instance(ri, pi))
                .flat_map(|a| a.dictionaries.iter().cloned())
                .collect();
            (p.clone(), dicts)
        })
        .collect()
}

/// As `skp-import` judges it: a model dictionary SketchUp keeps for itself.
fn is_sketchup_owned(name: &str) -> bool {
    matches!(name, "ModelProperties" | "GeoReference")
        || name.starts_with("GSU_")
        || name.starts_with("SU_")
}

fn material_name(m: &skpwright::Material) -> &str {
    match m {
        skpwright::Material::Solid { name, .. } | skpwright::Material::Textured { name, .. } => {
            name
        }
    }
}

/// The document's color, opacity and texture onto the file's record; a
/// texture the document changed (or added, or removed) replaces the
/// record's shape.
fn update_material(file: &mut skpwright::Material, doc: &skpwright::Material) {
    use skpwright::Material::{Solid, Textured};
    match (&mut *file, doc) {
        (
            Solid { rgba, opacity, .. },
            Solid {
                rgba: r,
                opacity: o,
                ..
            },
        ) => {
            if rgba[..3] != r[..3] {
                *rgba = *r;
            }
            if (*opacity - o).abs() > 1.0 / 255.0 {
                *opacity = *o;
            }
        }
        (
            Textured {
                image_bytes,
                applied_size_in,
                avg_rgba,
                opacity,
                ..
            },
            Textured {
                image_bytes: ib,
                applied_size_in: asi,
                avg_rgba: ar,
                opacity: o,
                ..
            },
        ) => {
            // The file's average color (what SketchUp draws with textures
            // off) stays with its image: Hew keeps none of its own (its
            // tint is white), so only a new image takes the document's.
            if image_bytes != ib {
                *image_bytes = ib.clone();
                *avg_rgba = *ar;
            }
            let close = |a: Option<(f64, f64)>, b: Option<(f64, f64)>| match (a, b) {
                (Some(a), Some(b)) => (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6,
                _ => a.is_none() && b.is_none(),
            };
            if !close(*applied_size_in, *asi) {
                *applied_size_in = *asi;
            }
            if (*opacity - o).abs() > 1.0 / 255.0 {
                *opacity = *o;
            }
        }
        _ => *file = doc.clone(),
    }
}

/// The highest persistent id anywhere in the model.
fn max_pid(m: &Model) -> u32 {
    m.geometry
        .iter()
        .flat_map(|r| {
            r.mesh
                .vertex_pids
                .iter()
                .copied()
                .chain(r.mesh.edges.iter().map(|e| e.pid))
                .chain(r.mesh.faces.iter().map(|f| f.pid))
                .chain(r.placed.iter().map(|p| p.pid))
                .chain(r.sections.iter().map(|s| s.pid))
                .chain(r.curves.iter().map(|c| c.pid))
                .chain(r.guide_points.iter().map(|g| g.pid))
                .chain(r.polylines.iter().map(|g| g.pid))
                .chain(r.images.iter().map(|i| i.pid))
        })
        .chain(m.texts.iter().map(|x| x.pid))
        .chain(m.dimensions.iter().map(|d| d.pid))
        .chain(m.radial_dimensions.iter().map(|d| d.pid))
        .chain(m.guides.iter().map(|g| g.pid))
        .chain(m.instances.iter().map(|i| i.pid))
        .max()
        .unwrap_or(0)
}

/// Every definition guide line, and every definition guide point,
/// carried to the world through each placement of its definition, as the
/// importer shows them — read from the source before the merge rewrites
/// its root list.
fn expanded_definition_guides(model: &Model) -> DefinitionGuides {
    let tree = model.scene();
    let mut placements: BTreeMap<usize, Vec<[f64; 16]>> = BTreeMap::new();
    let mut stack: Vec<&skpwright::Node> = tree.iter().collect();
    while let Some(n) = stack.pop() {
        if let Some(di) = n.run.and_then(|ri| model.geometry.get(ri)?.def_index) {
            placements.entry(di).or_default().push(n.world);
        }
        stack.extend(n.children.iter());
    }
    let mut out = Vec::new();
    for g in &model.guides {
        let Some(di) = g.def_index else { continue };
        for w in placements.get(&di).map(Vec::as_slice).unwrap_or(&[]) {
            let rows: [f64; 12] = w[..12].try_into().expect("12 entries");
            let t = kernel::Transform::from_affine(&rows);
            let p = t.apply_point(Point3::new(g.point_m[0], g.point_m[1], g.point_m[2]));
            let posed = t.apply_vector(kernel::Vec3::new(
                g.direction[0],
                g.direction[1],
                g.direction[2],
            ));
            if let Ok(d) = posed.normalized() {
                out.push((di, p, d, crate::file_guide_span(g.bounds, posed.length())));
            }
        }
    }
    let mut points = Vec::new();
    for run in &model.geometry {
        let Some(di) = run.def_index else { continue };
        for gp in &run.guide_points {
            for w in placements.get(&di).map(Vec::as_slice).unwrap_or(&[]) {
                let rows: [f64; 12] = w[..12].try_into().expect("12 entries");
                let t = kernel::Transform::from_affine(&rows);
                points.push((
                    di,
                    t.apply_point(Point3::new(gp.point_m[0], gp.point_m[1], gp.point_m[2])),
                ));
            }
        }
    }
    (out, points)
}

/// Removes the instances `gone` names from the model, renumbering the
/// features keyed by instance index.
fn drop_instances(m: &mut skpwright::Model, gone: impl Fn(&skpwright::Instance) -> bool) {
    let mut remap = Vec::with_capacity(m.instances.len());
    let mut next = 0usize;
    for i in &m.instances {
        if gone(i) {
            remap.push(None);
        } else {
            remap.push(Some(next));
            next += 1;
        }
    }
    let mut k = 0;
    m.instances.retain(|_| {
        let keep = remap[k].is_some();
        k += 1;
        keep
    });
    m.features.instance_hatch = m
        .features
        .instance_hatch
        .iter()
        .filter_map(|&(i, h)| Some((remap.get(i).copied().flatten()?, h)))
        .collect();
}
