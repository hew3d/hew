//! Hew [`Document`] -> SketchUp `.skp`, through the clean-room OpenSKP
//! writer: the 2026 container every current SketchUp saves, or the 2017
//! format SketchUp Make 2017 and every later release open.
//!
//! The mapping is the one `skp-import` reads back as the same document, so
//! a document survives Hew -> `.skp` -> Hew:
//!
//! - every world **Object** becomes a SketchUp **group** of its own, placed
//!   at the identity: SketchUp's loose geometry sticks together where it
//!   touches, and Hew's solids must stay discrete. The group carries the
//!   Object's name, tag (as its layer), hidden state and base material (as
//!   the placement's paint, which SketchUp's default-material faces
//!   inherit exactly as Hew's unpainted faces resolve to the base);
//! - a **merge group** becomes a SketchUp group whose definition places its
//!   members (world coordinates, identity placement);
//! - a **component definition** becomes a SketchUp component definition: a
//!   lone member Object is the definition's own geometry, several are
//!   nested groups; member groups, member instances and member sketches go
//!   in as they would at world level, in definition-local coordinates;
//! - an **instance** becomes a component placement with its pose;
//! - a **sketch** (drawn, not extruded) becomes loose edges and faces in its
//!   owner's entity list — SketchUp's own form for unextruded drawing;
//! - **tags** become layers (a path joins with `/`), **materials** keep
//!   name, color, opacity and texture, **scenes** keep camera, hidden
//!   entities, hidden tags and section cut, **guides** keep lines and
//!   points, and the section plane, camera and axes come along.
//!
//! What `.skp` cannot hold is reported in [`Export::simplified`] — the
//! app's one-time notice reads it.
//!
//! I/O-free: a document in, bytes out (DEVELOPMENT.md rule 1 posture).

use std::collections::{BTreeMap, BTreeSet};

mod curves;
mod dicts;
mod merge;
mod source;
mod texture;

pub use merge::{SaveUnits, Saved, save, save_in};

pub use source::{DefSource, NodeSource, Origin, SkpSource, def_hash, geometry_hash, node_hash};

use dicts::HewDict;
use kernel::{
    AttrTarget, CameraProjection, ComponentId, Document, EntityRef, GroupId, Guide, GuideId,
    InstanceId, MaterialId, NodeId, Object, ObjectId, Point3, SectionPlaneState, SketchId,
    Transform,
};
use openskp::{
    Definition, DefinitionKind, Dictionary, GeometryRun, GuidePoint, Instance, Layer, Material,
    Mesh, MeshEdge, MeshFace, Model, Owner, PlacedInstance, SceneProperties, SectionPlane, Value,
};

/// Which `.skp` container to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The 2026 container (a ZIP archive): what current SketchUp saves.
    Skp2026,
    /// The 2017 format: opens in SketchUp Make 2017 and every later release.
    Skp2017,
}

/// A written `.skp` file.
#[derive(Debug, Clone)]
pub struct Export {
    /// The file.
    pub bytes: Vec<u8>,
    /// What the document holds that the `.skp` could not carry exactly, as
    /// short user-facing phrases, deduplicated and sorted. Empty when the
    /// file holds everything.
    pub simplified: Vec<String>,
    /// The OpenSKP writer's own notes (what a 2026 model lost on the way
    /// to the 2017 format).
    pub writer_notes: Vec<String>,
    /// The persistent id each written node took: Hew stable id -> `.skp`
    /// persistent id, for objects, groups and instances.
    pub pids: BTreeMap<u64, u32>,
}

/// Why a document could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// The writer refused the model (a bug here: the model this crate
    /// builds must always be writable).
    Write(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Write(m) => write!(f, "could not write the SketchUp file: {m}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<openskp::Error> for ExportError {
    fn from(e: openskp::Error) -> Self {
        ExportError::Write(e.0)
    }
}

pub use openskp::WriteEnv;

/// Write `doc` as a `.skp` file in the `target` container. Any open
/// group or component editing session is closed first, exactly as a `.hew`
/// save does. `env` is the save time and seed the file records
/// ([`WriteEnv::system`] where the platform has a clock; the browser
/// passes its own).
pub fn export(doc: &Document, target: Target, env: WriteEnv) -> Result<Export, ExportError> {
    export_in(doc, target, env, None)
}

/// [`export`], the file opening in `units` (a new SketchUp model's units
/// without).
pub fn export_in(
    doc: &Document,
    target: Target,
    env: WriteEnv,
    units: Option<DisplayUnits>,
) -> Result<Export, ExportError> {
    let doc = doc.session_closed();
    let mut authored = author(&doc);
    if let Some(u) = units {
        authored.model.units = u.units();
    }
    let (bytes, writer_notes) = write(&authored.model, target, env)?;
    Ok(Export {
        bytes,
        simplified: authored.simplified.into_iter().collect(),
        writer_notes,
        pids: authored.pids,
    })
}

/// The length units a written file opens in (SketchUp's Model Info ▸
/// Units) — Hew's display formats, each at the precision Hew shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayUnits {
    Meters,
    Centimeters,
    Millimeters,
    /// Feet and inches, to 1/16".
    Architectural,
    /// Inches with a fraction, to 1/16".
    FractionalInches,
    DecimalInches,
}

impl DisplayUnits {
    /// From the app's format names: `m`, `cm`, `mm`, `arch`, `frac_in`,
    /// `dec_in`.
    pub fn from_name(name: &str) -> Option<DisplayUnits> {
        Some(match name {
            "m" => DisplayUnits::Meters,
            "cm" => DisplayUnits::Centimeters,
            "mm" => DisplayUnits::Millimeters,
            "arch" => DisplayUnits::Architectural,
            "frac_in" => DisplayUnits::FractionalInches,
            "dec_in" => DisplayUnits::DecimalInches,
            _ => return None,
        })
    }

    /// SketchUp's settings for them: format (0 decimal, 1 architectural,
    /// 3 fractional), unit (0 in, 2 mm, 3 cm, 4 m), precision (decimal
    /// places, or 4 for 1/16"), length snapping to one step of that
    /// precision (stored in inches, as SketchUp's own metric files hold
    /// it), angle snapping to 15°, and area and volume in the same unit.
    pub fn units(self) -> openskp::Units {
        self.over(openskp::Units::sketchup_default())
    }

    /// `units` with these length settings (format, unit, precision,
    /// snapping step, area and volume) and every other setting as it was:
    /// a file's own angle precision and snapping survive a change of unit.
    pub fn over(self, units: openskp::Units) -> openskp::Units {
        use DisplayUnits::*;
        const IN: f64 = 0.0254;
        let (format, unit, precision, snap) = match self {
            Meters => (0, 4, 3, 0.001 / IN),
            Centimeters => (0, 3, 2, 0.0001 / IN),
            Millimeters => (0, 2, 1, 0.0001 / IN),
            Architectural => (1, 0, 4, 0.0625),
            FractionalInches => (3, 0, 4, 0.0625),
            DecimalInches => (0, 0, 3, 0.001),
        };
        openskp::Units {
            length_format: format,
            length_unit: unit,
            length_precision: precision,
            length_snap_length: snap,
            area_unit: Some(unit),
            volume_unit: Some(unit),
            area_precision: Some(2),
            volume_precision: Some(2),
            ..units
        }
    }
}

/// The model in `target`'s container: the bytes, and what a 2017 file
/// could not take.
pub(crate) fn write(
    model: &Model,
    target: Target,
    env: WriteEnv,
) -> Result<(Vec<u8>, Vec<String>), ExportError> {
    Ok(match target {
        Target::Skp2026 => (
            openskp::Document::from_model_with(model, env)?.write()?,
            Vec::new(),
        ),
        Target::Skp2017 => {
            let d = openskp::Document17::from_model_with(model, env)?;
            let notes = d.notes().to_vec();
            (d.write(), notes)
        }
    })
}

/// A document as an OpenSKP model, before it is written.
pub struct Authored {
    pub model: Model,
    pub simplified: BTreeSet<String>,
    pub pids: BTreeMap<u64, u32>,
    /// Where each node written now sits in the file (by stable id).
    pub origins: BTreeMap<u64, Origin>,
    /// Each component definition's file definition: name and paint.
    pub def_sources: BTreeMap<u64, (String, u16)>,
    /// Objects written as the file's loose geometry.
    pub loose_written: usize,
    /// Annotations left to the file's own records.
    pub kept_annotations: BTreeSet<kernel::AnnotationId>,
    /// Tag stable id -> the written layer's name ([`SkpSource::layers`]).
    pub tag_layers: BTreeMap<u64, String>,
    /// The source's loose geometry was kept as it was.
    pub loose_kept: bool,
    /// The source file's placements carried unread ([`SkpSource::unread`]).
    pub unread: BTreeSet<u32>,
}

/// Build the OpenSKP model for `doc` (which must have no open session —
/// [`export`] closes it).
pub fn author(doc: &Document) -> Authored {
    author_with(Author::new(doc))
}

/// Run an author over its document: from nothing, or (in reuse mode) into
/// the source file's own model.
fn author_with(mut a: Author) -> Authored {
    let doc = a.doc;
    let reuse = a.reuse.is_some();
    if reuse {
        a.materials_on_source();
        a.layers_on_source();
    } else {
        a.materials();
        a.layers();
    }
    let mut root = if reuse {
        a.root_on_source()
    } else {
        RunBuf::default()
    };
    for node in doc.top_level_nodes() {
        a.place_node(&mut root, node, Frame::World);
    }
    if reuse {
        a.loose_on_source(&mut root);
        a.images_on_source();
    }
    // Free drawing (a group's goes with its group). The file's loose
    // edges standing for a sketch unchanged since the open stay as they
    // are when the loose geometry is kept.
    let loose_kept = a.reuse.as_ref().is_some_and(|r| r.keep_loose);
    let sketches: Vec<SketchId> = doc
        .sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch_owner_group(s).is_none())
        .filter(|&s| {
            !(loose_kept
                && a.reuse.as_ref().is_some_and(|r| {
                    doc.sid_of(&EntityRef::Sketch(s))
                        .is_some_and(|sid| r.source.loose_sketches.contains_key(&sid))
                }))
        })
        .collect();
    if !sketches.is_empty() {
        a.model_hew.set("loose", Value::Text("sketches".into()));
    }
    for sk in sketches {
        a.sketch_into(&mut root, sk);
    }
    a.loose_kept = loose_kept;
    let sections = if reuse {
        a.guides_on_source(&mut root);
        a.sections_on_source(&mut root)
    } else {
        a.guides(&mut root);
        a.section_planes(&mut root)
    };
    if reuse {
        a.scenes_on_source(&sections);
        a.view();
        a.camera_on_source();
        a.model_dictionaries_on_source();
        a.annotations_on_source(&root);
    } else {
        a.scenes(&sections);
        a.view();
    }
    a.annotations();
    let root_map = None;
    a.push_run(root, root_map);
    if reuse {
        a.prune_unplaced_groups_on_source();
        a.prune_layers_on_source();
    }
    a.model.definition_links.sort_by_key(|l| l.0);
    a.finish_dictionaries();
    // The layer each tag stands for in the written file, by stable id.
    let tag_layers: BTreeMap<u64, String> = a
        .layer_slot
        .iter()
        .filter_map(|(path, &slot)| {
            let sid = doc.sid_of(&EntityRef::Tag(path.clone()))?;
            let name = a.model.layer_of(slot)?.name.clone();
            Some((sid, name))
        })
        .collect();
    Authored {
        tag_layers,
        loose_kept: a.loose_kept,
        model: a.model,
        simplified: a.simplified,
        pids: a.pids,
        origins: a.origins,
        def_sources: a.def_sources,
        loose_written: a.loose_written,
        kept_annotations: a.kept_annotations,
        unread: a.reuse.map(|r| r.unread).unwrap_or_default(),
    }
}

/// Whether node coordinates are world (a world tree node) or a
/// definition's local frame. Only used to word notes today; the geometry
/// itself is already in the right frame either way.
#[derive(Clone, Copy)]
enum Frame {
    World,
    Definition,
}

struct Author<'d> {
    doc: &'d Document,
    model: Model,
    /// The next persistent id to hand out (ids start at 1; 0 = none).
    next_pid: u32,
    /// The next definition map index (the key placements name).
    next_def: u32,
    /// Hew material -> `.skp` face material slot (slot k+1 = materials[k]).
    mat_slot: BTreeMap<MaterialId, u16>,
    /// Tag path -> `.skp` layer slot (slot 0 = the default layer).
    layer_slot: BTreeMap<Vec<String>, u16>,
    /// Component definition -> its `.skp` definition map index.
    def_of: BTreeMap<ComponentId, u32>,
    /// Groups written so far, for SketchUp's "Group#N" definition names.
    groups_written: u32,
    simplified: BTreeSet<String>,
    pids: BTreeMap<u64, u32>,
    /// Dictionaries of placements, by pid (their run and place in it are
    /// known only once every run is written).
    placement_dicts: Vec<(u32, Vec<Dictionary>)>,
    /// Dictionaries of definitions, by map index.
    def_dicts: Vec<(u32, Vec<Dictionary>)>,
    /// The model's `hew` dictionary.
    model_hew: HewDict,
    /// Reuse mode: writing into the source file's own model.
    reuse: Option<merge::Reuse>,
    /// The world Object group being written, while its arcs are recorded.
    arc_run: Option<u32>,
    /// Arcs of world Objects: where a radius dimension can hang.
    arc_sites: Vec<ArcSite>,
    /// Where each node written sits in the file (by stable id).
    origins: BTreeMap<u64, Origin>,
    /// Each component definition's file definition: name and paint.
    def_sources: BTreeMap<u64, (String, u16)>,
    loose_written: usize,
    /// Annotations a save back leaves to the file's own record (unchanged
    /// since it was read from it), not written again.
    kept_annotations: BTreeSet<kernel::AnnotationId>,
    loose_kept: bool,
}

impl<'d> Author<'d> {
    fn new(doc: &'d Document) -> Self {
        Author {
            doc,
            model: Model::default(),
            next_pid: 1,
            next_def: 1,
            mat_slot: BTreeMap::new(),
            layer_slot: BTreeMap::new(),
            def_of: BTreeMap::new(),
            groups_written: 0,
            simplified: BTreeSet::new(),
            pids: BTreeMap::new(),
            placement_dicts: Vec::new(),
            def_dicts: Vec::new(),
            model_hew: HewDict::default(),
            reuse: None,
            arc_run: None,
            arc_sites: Vec::new(),
            origins: BTreeMap::new(),
            def_sources: BTreeMap::new(),
            loose_written: 0,
            kept_annotations: BTreeSet::new(),
            loose_kept: false,
        }
    }

    /// The Hew attributes of `target` as SketchUp dictionaries.
    fn attrs(&self, target: AttrTarget) -> Vec<Dictionary> {
        match self.doc.attr_get(&target) {
            Ok(Some(d)) => dicts::from_attrs(d),
            _ => Vec::new(),
        }
    }

    /// Whether the root list's loose geometry is Hew's: written from the
    /// document, or kept from a source whose loose geometry Hew built.
    fn loose_built(&self) -> bool {
        self.reuse
            .as_ref()
            .is_none_or(|r| !r.keep_loose || r.source_loose_built)
    }

    /// Push a finished entity list into the model, its faces'
    /// dictionaries with it.
    fn push_run(&mut self, mut run: RunBuf, def: Option<u32>) -> usize {
        let ri = self.model.geometry.len();
        let face_dicts = std::mem::take(&mut run.face_dicts);
        for (fi, d) in face_dicts {
            self.model.dictionaries.push(openskp::Attributes {
                owner: Owner::Face(ri, fi),
                dictionaries: vec![d],
            });
        }
        for (ci, d) in std::mem::take(&mut run.curve_dicts) {
            self.model.dictionaries.push(openskp::Attributes {
                owner: Owner::Curve(ri, ci),
                dictionaries: vec![d],
            });
        }
        let planes = std::mem::take(&mut run.sketch_planes);
        if planes.len() > 1 {
            let mut seen = BTreeSet::new();
            for (ei, si) in std::mem::take(&mut run.sketch_edges) {
                // An edge two sketches share is written once, and belongs
                // to the first.
                if !seen.insert(ei) {
                    continue;
                }
                let mut h = HewDict::default();
                h.set("sketch", Value::Int(si as i32));
                self.model.dictionaries.push(openskp::Attributes {
                    owner: Owner::Edge(ri, ei),
                    dictionaries: h.into_dictionary().into_iter().collect(),
                });
            }
        }
        // Geometry Hew built (not a file's own, carried as it was): a reader
        // takes it as written, without the repair a foreign mesh needs.
        if def.is_none() && self.loose_built() {
            self.model_hew.set("loose_built", Value::Bool(true));
        }
        match def {
            Some(map) => {
                let mut h = HewDict::default();
                h.set("built", Value::Bool(true));
                if let Some(loose) = run.loose {
                    h.set("loose", Value::Text(loose.into()));
                }
                if !planes.is_empty() {
                    h.set("sketch_planes", Value::Array(planes));
                }
                self.def_dicts
                    .extend(h.into_dictionary().map(|d| (map, vec![d])));
            }
            None if !planes.is_empty() => self.model_hew.set("sketch_planes", Value::Array(planes)),
            None => {}
        }
        // The root list in reuse mode, and a definition rewritten in its
        // file record's place, replace the file's run: what the document
        // does not hold of it (images, polylines, a definition's own
        // section planes and guide points) stays.
        let replace = match (&self.reuse, def) {
            (Some(r), None) => Some(r.root),
            (Some(r), Some(map)) => r.replace_runs.get(&map).copied(),
            (None, _) => None,
        };
        match replace {
            Some(at) => {
                let new = run.finish(def);
                // The file's images the document read in are written as
                // the document has them (`image_into`); one it could not
                // show stays as it was.
                let read_in: BTreeSet<u32> = self
                    .reuse
                    .as_ref()
                    .map(|r| {
                        r.source
                            .nodes
                            .values()
                            .filter_map(|n| match n.origin {
                                Origin::Image { pid } => Some(pid),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let old = &mut self.model.geometry[at];
                let keep_own = def.is_some();
                old.mesh = new.mesh;
                old.curves = new.curves;
                old.placed = new.placed;
                // (The root list's own images are edited where they are,
                // `reuse_node`; a rewritten definition's are written anew.)
                if keep_own {
                    old.images.retain(|im| !read_in.contains(&im.pid));
                }
                old.images.extend(new.images);
                let carried = self.carried_into(at);
                self.model.geometry[at].placed.extend(carried);
                // (A group definition reframed to the identity keeps none
                // of its old sections or guide points: they were in the
                // old frame.)
                let reframed = self.reuse.as_ref().is_some_and(|r| {
                    def.is_some_and(|map| r.reframed_defs.contains(&(map as usize)))
                });
                let old = &mut self.model.geometry[at];
                old.curve_members.clear();
                old.relationships.clear();
                if !keep_own || reframed || !new.sections.is_empty() {
                    old.sections = new.sections;
                }
                if !keep_own || reframed || !new.guide_points.is_empty() {
                    old.guide_points = new.guide_points;
                }
                if def.is_none() {
                    old.active_section = new.active_section;
                }
                // Dictionaries authored for this run were keyed to `ri`.
                for a in self.model.dictionaries.iter_mut() {
                    match &mut a.owner {
                        Owner::Face(x, _) | Owner::Edge(x, _) | Owner::Curve(x, _) if *x == ri => {
                            *x = at
                        }
                        _ => {}
                    }
                }
                at
            }
            None => {
                self.model.geometry.push(run.finish(def));
                ri
            }
        }
    }

    /// Resolve every pending dictionary to its owner and add the model's.
    fn finish_dictionaries(&mut self) {
        let mut at: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
        for (ri, r) in self.model.geometry.iter().enumerate() {
            for (pi, p) in r.placed.iter().enumerate() {
                at.insert(p.pid, (ri, pi));
            }
        }
        for (pid, dictionaries) in std::mem::take(&mut self.placement_dicts) {
            if let Some(&(ri, pi)) = at.get(&pid) {
                self.model.dictionaries.push(openskp::Attributes {
                    owner: Owner::Instance(ri, pi),
                    dictionaries,
                });
            }
        }
        // One dictionary per name and definition: a definition's `hew`
        // entries come from more than one place (its member, its run).
        let mut by_def: BTreeMap<u32, Vec<Dictionary>> = BTreeMap::new();
        for (map, ds) in std::mem::take(&mut self.def_dicts) {
            let list = by_def.entry(map).or_default();
            for d in ds {
                match list.iter_mut().find(|e| e.name == d.name) {
                    Some(e) => e.entries.extend(d.entries),
                    None => list.push(d),
                }
            }
        }
        for (map, dictionaries) in by_def {
            if let Some(di) = self
                .model
                .definitions
                .iter()
                .position(|d| d.map_index == Some(map as usize))
            {
                self.model.dictionaries.push(openskp::Attributes {
                    owner: Owner::Definition(di),
                    dictionaries,
                });
            }
        }
        let mut model = if self.reuse.is_some() {
            Vec::new()
        } else {
            self.attrs(AttrTarget::Document)
        };
        self.model_hew.set("version", Value::Int(1));
        if let Some(h) = std::mem::take(&mut self.model_hew).into_dictionary() {
            model.push(h);
        }
        self.model.dictionaries.push(openskp::Attributes {
            owner: Owner::Model,
            dictionaries: model,
        });
    }

    fn pid(&mut self) -> u32 {
        let p = self.next_pid;
        self.next_pid += 1;
        p
    }

    fn note(&mut self, what: &str) {
        self.simplified.insert(what.to_string());
    }

    // ── Materials and layers ──────────────────────────────────────────────

    fn materials(&mut self) {
        let mut taken: BTreeSet<String> = BTreeSet::new();
        for (id, m) in self.doc.materials().iter() {
            let name = unique_name(&m.name, "Material", &mut taken);
            let written = material_record(&name, m);
            self.model.materials.push(written);
            let slot = self.model.materials.len() as u16;
            self.model
                .material_links
                .push((slot, self.model.materials.len() - 1));
            self.mat_slot.insert(id, slot);
            let d = self.attrs(AttrTarget::Entity(EntityRef::Material(id)));
            if !d.is_empty() {
                self.model.dictionaries.push(openskp::Attributes {
                    owner: Owner::Material(self.model.materials.len() - 1),
                    dictionaries: d,
                });
            }
        }
    }

    fn mat(&self, m: Option<MaterialId>) -> Option<u16> {
        m.and_then(|id| self.mat_slot.get(&id).copied())
    }

    fn layers(&mut self) {
        self.model.layers.push(Layer {
            name: "Layer0".into(),
            visible: true,
            rgba: [255, 255, 255, 255],
        });
        self.model.layer_links.push((0, 0));
        let tags: Vec<(Vec<String>, bool)> = self
            .doc
            .tag_meta()
            .map(|(p, hidden)| (p.to_vec(), hidden))
            .collect();
        let mut taken: BTreeSet<String> = ["Layer0".to_string()].into();
        let mut tag_paths = Vec::new();
        for (path, hidden) in tags {
            if path.is_empty() {
                continue;
            }
            let name = unique_name(&path.join("/"), "Tag", &mut taken);
            tag_paths.push(Value::Array(vec![
                Value::Text(name.clone()),
                dicts::path_value(&path),
            ]));
            let d = self.attrs(AttrTarget::Entity(EntityRef::Tag(path.clone())));
            if !d.is_empty() {
                self.model.dictionaries.push(openskp::Attributes {
                    owner: Owner::Layer(self.model.layers.len()),
                    dictionaries: d,
                });
            }
            self.model.layers.push(Layer {
                name,
                visible: !hidden,
                rgba: layer_color(self.model.layers.len()),
            });
            let i = self.model.layers.len() - 1;
            self.model.layer_links.push((i as u16, i));
            self.layer_slot.insert(path, i as u16);
        }
        if !tag_paths.is_empty() {
            self.model_hew.set("tag_paths", Value::Array(tag_paths));
        }
    }

    /// The layer slot for a node's tags: SketchUp gives an entity one layer,
    /// so the first tag wins and any others are reported.
    fn node_layer(&mut self, node: NodeId) -> u16 {
        let tags = self.doc.node_tags(node);
        if tags.len() > 1 {
            self.note("objects with more than one tag show only their first in SketchUp");
        }
        tags.first()
            .and_then(|p| self.layer_slot.get(p).copied())
            .unwrap_or(0)
    }

    // ── The node tree ──────────────────────────────────────────────────────

    /// Place one tree node into `run` (the world root, a group's definition,
    /// or a component definition).
    fn place_node(&mut self, run: &mut RunBuf, node: NodeId, frame: Frame) {
        if self.reuse.is_some() && (self.reuse_node(run, node, frame) || self.image_into(run, node))
        {
            return;
        }
        match node {
            NodeId::Object(id) => {
                if let Some(p) = self.object_group(id, frame) {
                    run.placed.push(p);
                }
            }
            NodeId::Group(id) => {
                if let Some(p) = self.merge_group(id, frame) {
                    run.placed.push(p);
                }
            }
            NodeId::Instance(id) => {
                if let Some(p) = self.instance(id) {
                    run.placed.push(p);
                }
            }
        }
    }

    /// A new definition: pushes it with its link and returns its map index.
    fn definition(&mut self, name: String, kind: DefinitionKind) -> u32 {
        let map = self.next_def;
        self.next_def += 1;
        self.model.definitions.push(Definition {
            name,
            kind,
            map_index: Some(map as usize),
            ..Default::default()
        });
        self.model
            .definition_links
            .push((map, self.model.definitions.len() - 1));
        map
    }

    /// A group definition's name: the node's own name where it has one
    /// (a reader that names a group after its definition then still gets
    /// it), else SketchUp's own "Group#N".
    fn group_def_name(&mut self, own: Option<&str>) -> String {
        let n = self.groups_written;
        self.groups_written += 1;
        match own.filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => format!("Group#{n}"),
        }
    }

    /// A placement of definition `defref`, its pid recorded against the
    /// Hew node's stable id, and its name (SketchUp keeps a placement's
    /// name on the instance list, by pid).
    fn placement(
        &mut self,
        node: NodeId,
        defref: u32,
        transform: [f64; 13],
        is_group: bool,
        material: u16,
        name: Option<&str>,
    ) -> PlacedInstance {
        let pid = self.claim_pid(node).unwrap_or_else(|| self.pid());
        self.placement_with_pid(node, defref, transform, is_group, material, name, pid)
    }

    /// [`Author::placement`] with its persistent id chosen.
    #[allow(clippy::too_many_arguments)]
    fn placement_with_pid(
        &mut self,
        node: NodeId,
        defref: u32,
        transform: [f64; 13],
        is_group: bool,
        material: u16,
        name: Option<&str>,
        pid: u32,
    ) -> PlacedInstance {
        let entity = match node {
            NodeId::Object(id) => EntityRef::Object(id),
            NodeId::Group(id) => EntityRef::Group(id),
            NodeId::Instance(id) => EntityRef::Instance(id),
        };
        if let Some(sid) = self.doc.sid_of(&entity) {
            self.pids.insert(sid, pid);
            self.origins.insert(
                sid,
                match node {
                    NodeId::Object(_) => Origin::GroupGeometry { pid },
                    NodeId::Group(_) => Origin::Group { pid },
                    NodeId::Instance(_) => Origin::Instance { pid },
                },
            );
        }
        let mut dictionaries = self.attrs(AttrTarget::Entity(entity));
        let mut hew = HewDict::default();
        match node {
            NodeId::Object(_) => hew.set("kind", Value::Text("object".into())),
            NodeId::Group(_) => hew.set("kind", Value::Text("group".into())),
            NodeId::Instance(_) => {}
        }
        if name.is_none_or(str::is_empty) {
            hew.set("unnamed", Value::Bool(true));
        }
        let tags = self.doc.node_tags(node);
        if tags.len() > 1 {
            hew.set(
                "tags",
                Value::Array(tags.iter().map(|p| dicts::path_value(p)).collect()),
            );
        }
        if let Some(d) = hew.into_dictionary() {
            dictionaries.push(d);
        }
        if !dictionaries.is_empty() {
            self.placement_dicts.push((pid, dictionaries));
        }
        // One name record per pid.
        self.model.instances.retain(|i| i.pid != pid);
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            self.model.instances.push(Instance {
                pid,
                definition: None,
                defref,
                offset: 0,
                translation_m: [transform[9], transform[10], transform[11]]
                    .map(|v| v / openskp::INCH),
                transform,
                material: Some(material),
                name: Some(n.to_string()),
                is_group: Some(is_group),
                hidden: Some(self.doc.node_user_hidden(node)),
                casts_shadows: Some(true),
                receives_shadows: Some(true),
                locked: Some(false),
                layer: None,
                slot: None,
                guid: String::new(),
            });
        }
        let layer = self.node_layer(node);
        PlacedInstance {
            pid,
            defref,
            transform,
            is_group,
            material,
            hidden: self.doc.node_user_hidden(node),
            layer,
            ..Default::default()
        }
    }

    /// One Object as a group of its own (identity placement), its faces
    /// and edges the group definition's geometry.
    fn object_group(&mut self, id: ObjectId, frame: Frame) -> Option<PlacedInstance> {
        let obj = self.doc.object(id)?;
        if obj.faces().is_empty() {
            return None;
        }
        let name = self.doc.object_name(id).map(str::to_string);
        let def_name = self.group_def_name(name.as_deref());
        let map = self.definition(def_name, DefinitionKind::Group);
        let mut run = RunBuf::default();
        // A world Object's group sits at the identity: its arcs are where a
        // world radius dimension can hang.
        self.arc_run = matches!(frame, Frame::World).then_some(map);
        self.object_into(&mut run, obj);
        self.arc_run = None;
        self.push_run(run, Some(map));
        let material = self.mat(obj.default_material()).unwrap_or(0);
        Some(self.placement(
            NodeId::Object(id),
            map,
            transform13(&Transform::IDENTITY),
            true,
            material,
            name.as_deref(),
        ))
    }

    /// A merge group: a SketchUp group placing its members.
    fn merge_group(&mut self, id: GroupId, frame: Frame) -> Option<PlacedInstance> {
        let members = self.doc.group_members(id)?;
        let name = self.doc.group_name(id).map(str::to_string);
        // A group the file already has is rewritten in its own definition.
        let map = match self
            .reuse
            .is_some()
            .then(|| self.group_in_place(id))
            .flatten()
        {
            Some(map) => map,
            None => {
                let def_name = self.group_def_name(name.as_deref());
                self.definition(def_name, DefinitionKind::Group)
            }
        };
        let mut run = RunBuf::default();
        for m in members {
            self.place_node(&mut run, m, frame);
        }
        // The group's own drawing, as loose edges of its list.
        let sketches = self.doc.group_member_sketches(id);
        if !sketches.is_empty() {
            run.loose = Some("sketches");
        }
        for sk in sketches {
            self.sketch_into(&mut run, sk);
        }
        self.push_run(run, Some(map));
        Some(self.placement(
            NodeId::Group(id),
            map,
            transform13(&Transform::IDENTITY),
            true,
            0,
            name.as_deref(),
        ))
    }

    /// A component instance: a placement of its (written-once) definition.
    fn instance(&mut self, id: InstanceId) -> Option<PlacedInstance> {
        let def = self.doc.instance_def(id)?;
        let pose = self.doc.instance_pose(id)?;
        let map = self.component(def)?;
        // The placement's own material, else (for a reused file
        // definition) the paint the definition passes down.
        let own = self
            .doc
            .instance_material(id)
            .flatten()
            .and_then(|m| self.mat(Some(m)));
        let paint = own.unwrap_or_else(|| {
            self.reuse
                .as_ref()
                .and_then(|r| r.def_paint.get(&def).copied())
                .unwrap_or(0)
        });
        let name = self.doc.instance_name(id).map(str::to_string);
        Some(self.placement(
            NodeId::Instance(id),
            map,
            transform13(&pose),
            false,
            paint,
            name.as_deref(),
        ))
    }

    /// A component definition, written on first use.
    fn component(&mut self, def: ComponentId) -> Option<u32> {
        if let Some(&m) = self.def_of.get(&def) {
            return Some(m);
        }
        if self.reuse.is_some()
            && let Some(m) = self.reuse_component(def)
        {
            self.def_of.insert(def, m);
            return Some(m);
        }
        let members = self.doc.def_member_nodes(def)?;
        // The definition's own drawing: a sketch one of its groups holds
        // is written with that group (`merge_group`), not here too.
        let sketches: Vec<SketchId> = self
            .doc
            .def_member_sketches(def)
            .unwrap_or_default()
            .into_iter()
            .filter(|&s| self.doc.sketch_owner_group(s).is_none())
            .collect();
        let name = self
            .doc
            .component_name(def)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Component#{}", self.def_of.len()));
        // The name a reader gave the solid it read from this definition's
        // geometry, before a save back numbers the definition apart from
        // the file's own.
        let own_name = name.clone();
        let in_place = if self.reuse.is_some() {
            self.in_place(def)
        } else {
            None
        };
        let (map, name) = match in_place {
            Some((map, _)) => (map, name),
            None => {
                let name = if self.reuse.is_some() {
                    self.unique_def_name(name)
                } else {
                    name
                };
                (
                    self.definition(name.clone(), DefinitionKind::Component),
                    name,
                )
            }
        };
        if let Some(sid) = self.doc.sid_of(&EntityRef::Component(def)) {
            let written = match in_place {
                Some(_) => self
                    .model
                    .definitions
                    .iter()
                    .find(|d| d.map_index == Some(map as usize))
                    .map_or(name.clone(), |d| d.name.clone()),
                None => name.clone(),
            };
            self.def_sources.insert(sid, (written, 0));
        }
        // Registered before the members are written, so a (malformed)
        // self-placing definition terminates.
        self.def_of.insert(def, map);
        let mut run = RunBuf::default();
        // A lone member solid is written as the definition's own geometry,
        // as SketchUp draws a component — when nothing of the solid's own
        // would be lost there: no name of its own (a reader names the
        // definition's geometry after the definition), no tag, not hidden.
        let lone_object = match members.as_slice() {
            [NodeId::Object(o)]
                if sketches.is_empty()
                    && self
                        .doc
                        .object_name(*o)
                        .is_none_or(|n| n.is_empty() || n == own_name)
                    && self.doc.node_tags(NodeId::Object(*o)).is_empty()
                    && !self.doc.node_user_hidden(NodeId::Object(*o))
                    && self
                        .attrs(AttrTarget::Entity(EntityRef::Object(*o)))
                        .is_empty() =>
            {
                Some(*o)
            }
            _ => None,
        };
        match lone_object {
            // A lone solid is the definition's own geometry, as SketchUp
            // draws a component; its base material is a face-level fact
            // here (there is no placement of its own to carry it).
            Some(o) => {
                if let Some(obj) = self.doc.object(o) {
                    let base = self.mat(obj.default_material());
                    self.object_into_with_base(&mut run, obj, base);
                }
                run.loose = Some("object");
                // The definition's geometry takes the definition's name in
                // a reader; record that the solid had none of its own.
                if self.doc.object_name(o).is_none_or(str::is_empty) {
                    let mut h = HewDict::default();
                    h.set("unnamed", Value::Bool(true));
                    self.def_dicts
                        .extend(h.into_dictionary().map(|d| (map, vec![d])));
                }
            }
            None => {
                for m in members {
                    self.place_node(&mut run, m, Frame::Definition);
                }
                if !sketches.is_empty() {
                    run.loose = Some("sketches");
                }
                for sk in sketches {
                    self.sketch_into(&mut run, sk);
                }
            }
        }
        let d = self.attrs(AttrTarget::Entity(EntityRef::Component(def)));
        if !d.is_empty() {
            self.def_dicts.push((map, d));
        }
        self.push_run(run, Some(map));
        Some(map)
    }

    // ── Geometry ───────────────────────────────────────────────────────────

    fn object_into(&mut self, run: &mut RunBuf, obj: &Object) {
        self.object_into_with_base(run, obj, None);
    }

    /// An Object's faces and edges into `run`. `base` paints the faces that
    /// carry no material of their own (used where no placement carries the
    /// Object's base material).
    fn object_into_with_base(&mut self, run: &mut RunBuf, obj: &Object, base: Option<u16>) {
        let he = obj.half_edges();
        let verts = obj.vertices();
        let mut index: BTreeMap<kernel::VertexId, u32> = BTreeMap::new();
        let mut vi = |run: &mut RunBuf, v: kernel::VertexId| -> u32 {
            *index
                .entry(v)
                .or_insert_with(|| run.vertex(verts[v].position))
        };
        // Circle claims, keyed by the circle, for welding below.
        let mut claimed: BTreeMap<[i64; 4], (kernel::CurveGeom, Vec<curves::Member>)> =
            BTreeMap::new();
        let surface_of = |h: kernel::HalfEdgeId| {
            let face = obj.loops()[he[h].loop_id].face;
            obj.faces()[face].surface
        };
        for (_, e) in obj.edges().iter() {
            let h = he[e.half_edge];
            let a = vi(run, h.origin);
            let b = vi(run, he[h.next].origin);
            let (pa, pb) = (verts[h.origin].position, verts[he[h.next].origin].position);
            let near = surface_of(e.half_edge);
            let far = e.twin_half_edge.and_then(surface_of);
            // A seam between two facets of one curved surface is what
            // SketchUp draws soft and smooth — the tessellator smooths
            // across exactly these.
            let seam = matches!((near, far), (Some(x), Some(y)) if x.same_surface(&y));
            let Some(idx) = run.edge(a, b, e.soft || seam) else {
                continue;
            };
            // The circle the edge is a chord of: its own claim, else the
            // rim where a cylinder wall meets a face off that cylinder.
            let claim = e.curve.or_else(|| match (near, far) {
                (Some(w), other) | (other, Some(w))
                    if !seam && other.is_none_or(|o| !o.same_surface(&w)) =>
                {
                    rim_circle(&w, pa, pb)
                }
                _ => None,
            });
            if let Some(g) = claim {
                let key = [g.center.x, g.center.y, g.center.z, g.radius]
                    .map(|v| (v * 1e9).round() as i64);
                claimed
                    .entry(key)
                    .or_insert_with(|| (g, Vec::new()))
                    .1
                    .push(curves::Member {
                        edge: idx,
                        a,
                        b,
                        pa,
                        pb,
                    });
            }
        }
        for (_, (g, members)) in claimed {
            let chains = curves::chains(
                &members,
                curves::Claim::Circle {
                    center: g.center,
                    radius: g.radius,
                },
            );
            let welded = run.weld(chains);
            if let Some(map) = self.arc_run {
                for ci in welded {
                    let Some(arc) = run.curves[ci].arc.clone() else {
                        continue;
                    };
                    let Some(e) = run
                        .mesh
                        .edges
                        .iter()
                        .position(|e| e.curve == Some(ci as u32))
                    else {
                        continue;
                    };
                    let pid = self.pid();
                    run.mesh.edges[e].pid = pid;
                    self.arc_sites.push(ArcSite {
                        arc,
                        edge_pid: pid,
                        map,
                    });
                }
            }
        }
        for (_, f) in obj.faces().iter() {
            let ring = |run: &mut RunBuf,
                        vi: &mut dyn FnMut(&mut RunBuf, kernel::VertexId) -> u32,
                        l: kernel::LoopId|
             -> Vec<u32> {
                obj.loop_half_edges(l)
                    .map(|h| vi(run, he[h].origin))
                    .collect()
            };
            let outer = ring(run, &mut vi, f.outer_loop);
            let holes: Vec<Vec<u32>> = f
                .inner_loops
                .iter()
                .map(|&l| ring(run, &mut vi, l))
                .collect();
            let mut material = self.mat(f.material).or(base);
            if f.surface.is_some() {
                self.note("curved surfaces are written as smooth facets");
            }
            let n = f.plane.normal();
            // A textured face carries its placement explicitly, so SketchUp
            // maps the image exactly as Hew draws it. The face is painted
            // with its effective material for that: SketchUp places an
            // inherited texture by its own default, not by a face's FTC.
            let effective = f.material.or(obj.default_material());
            let texture = effective
                .and_then(|id| Some((id, self.doc.materials().get(id)?.texture.as_ref()?)))
                .and_then(|(id, tex)| {
                    let frame = f
                        .uv_frame
                        .unwrap_or_else(|| planar_frame(n, tex.world_size));
                    let corners: Vec<Point3> = obj
                        .loop_half_edges(f.outer_loop)
                        .map(|h| verts[he[h].origin].position)
                        .collect();
                    let size = (inches(tex.world_size[0]), inches(tex.world_size[1]));
                    let placement = face_texture(&frame, n, &corners, size)?;
                    material = self.mat(Some(id));
                    Some(placement)
                });
            run.face(outer, holes, [n.x, n.y, n.z], material);
            if texture.is_some() {
                run.mesh.faces.last_mut().expect("just pushed").texture = texture.map(Box::new);
            }
            // A painted back goes on the back. Without one Hew draws a
            // face's material on both its sides: a closed solid's inner side
            // is never seen; an open shell's is, so its faces carry the
            // material (and its placement) on the back too — SketchUp draws
            // an unpainted back in its back-face color.
            if let Some(back) = f.back_material {
                let back = self.mat(Some(back));
                if let Some(face) = run.mesh.faces.last_mut() {
                    face.back_material = back;
                }
            } else if obj.watertight() == kernel::WatertightState::Open
                && let Some(face) = run.mesh.faces.last_mut()
            {
                face.back_material = face.front_material;
                if let Some(t) = face.texture.as_mut() {
                    t.back = t.front;
                }
            }
            if let Some(kernel::SurfaceRef::Cylinder {
                axis_point: p,
                axis: d,
                radius,
            }) = f.surface
            {
                let mut h = HewDict::default();
                h.set(
                    "surface",
                    Value::Array(
                        [p.x, p.y, p.z, d.x, d.y, d.z, radius]
                            .into_iter()
                            .map(Value::Double)
                            .collect(),
                    ),
                );
                let fi = run.mesh.faces.len() - 1;
                run.face_dicts.extend(h.into_dictionary().map(|d| (fi, d)));
            }
        }
    }

    /// A sketch's edges and regions as loose geometry in `run`. The
    /// sketch's tag is its edges' layer (SketchUp gives an entity one
    /// layer: the first tag's, like a node's).
    fn sketch_into(&mut self, run: &mut RunBuf, id: SketchId) {
        let Some(sk) = self.doc.sketch(id) else {
            return;
        };
        let tags = self.doc.sketch_tags(id);
        if tags.len() > 1 {
            self.note("drawing with more than one tag shows only its first in SketchUp");
        }
        let layer = tags
            .first()
            .and_then(|p| self.layer_slot.get(p).copied())
            .unwrap_or(0);
        let n = sk.plane().normal();
        let o = sk.plane().point();
        let sketch_index = run.sketch_planes.len();
        run.sketch_planes.push(Value::Array(
            [o.x, o.y, o.z, n.x, n.y, n.z]
                .into_iter()
                .map(Value::Double)
                .collect(),
        ));
        let mut index = BTreeMap::new();
        for (vid, v) in sk.vertices().iter() {
            index.insert(vid, run.vertex(v.position));
        }
        let mut chained: BTreeMap<kernel::SketchCurveId, Vec<curves::Member>> = BTreeMap::new();
        for (_, e) in sk.edges().iter() {
            if let (Some(&a), Some(&b)) = (index.get(&e.from), index.get(&e.to)) {
                let Some(idx) = run.edge(a, b, false) else {
                    continue;
                };
                // An edge two sketches share keeps the first one's layer.
                if run.mesh.edges[idx].layer == 0 {
                    run.mesh.edges[idx].layer = layer;
                }
                run.sketch_edges.push((idx, sketch_index));
                if let Some(c) = e.curve {
                    chained.entry(c).or_default().push(curves::Member {
                        edge: idx,
                        a,
                        b,
                        pa: sk.vertices()[e.from].position,
                        pb: sk.vertices()[e.to].position,
                    });
                }
            }
        }
        for (c, members) in chained {
            let claim = match sk.curve_analytic(c) {
                Some(kernel::CurveAnalytic {
                    geom,
                    kind: kernel::SketchCurveKind::Circle,
                }) => curves::Claim::Circle {
                    center: geom.center,
                    radius: geom.radius,
                },
                _ => curves::Claim::Plain,
            };
            let welded = run.weld(curves::chains(&members, claim));
            // A polygon's sides weld as a plain curve; its drawn center
            // and circumradius ride in the curve's `hew` dictionary.
            if let Some(kernel::CurveAnalytic {
                geom,
                kind: kernel::SketchCurveKind::Polygon,
            }) = sk.curve_analytic(c)
            {
                for ci in welded {
                    let mut h = HewDict::default();
                    h.set(
                        "polygon",
                        Value::Array(
                            [geom.center.x, geom.center.y, geom.center.z, geom.radius]
                                .into_iter()
                                .map(Value::Double)
                                .collect(),
                        ),
                    );
                    run.curve_dicts.extend(h.into_dictionary().map(|d| (ci, d)));
                }
            }
        }
        // (A loop with no face — `sketch.rs` rule 6 — writes its edges
        // alone, as SketchUp holds it.)
        for (_, r) in sk.regions().iter().filter(|(_, r)| !r.faceless) {
            let ring = |vs: &[kernel::SketchVertexId]| -> Option<Vec<u32>> {
                vs.iter().map(|v| index.get(v).copied()).collect()
            };
            let Some(outer) = ring(&r.outer) else {
                continue;
            };
            let holes: Option<Vec<Vec<u32>>> = r.holes.iter().map(|h| ring(h)).collect();
            let Some(holes) = holes else {
                continue;
            };
            run.face(outer, holes, [n.x, n.y, n.z], None);
        }
    }

    // ── Helpers, view state ────────────────────────────────────────────────

    /// The document's guides among the model's own entities. A guide's tag
    /// is its layer (SketchUp gives an entity one layer: the first tag's,
    /// like a node's).
    fn guides(&mut self, root: &mut RunBuf) {
        for gid in self.doc.guide_ids() {
            let Some(g) = self.doc.guide(gid).copied() else {
                continue;
            };
            let pid = self.pid();
            let layer = self.guide_layer(gid);
            match g {
                Guide::Line {
                    origin,
                    direction,
                    span,
                } => {
                    let d = self.attrs(AttrTarget::Entity(EntityRef::Guide(gid)));
                    if !d.is_empty() {
                        self.model.dictionaries.push(openskp::Attributes {
                            owner: Owner::Guide(self.model.guides.len()),
                            dictionaries: d,
                        });
                    }
                    self.model.guides.push(openskp::Guide {
                        def_index: None,
                        point_m: [origin.x, origin.y, origin.z],
                        direction: [direction.x, direction.y, direction.z],
                        pid,
                        bounds: guide_bounds(span),
                        layer,
                        material: None,
                        hidden: false,
                        casts_shadows: true,
                        receives_shadows: true,
                    })
                }
                Guide::Point { position } => {
                    let p = [position.x, position.y, position.z];
                    root.guide_points.push(GuidePoint {
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

    /// The layer slot a guide is written on: its first tag's, the default
    /// layer for an untagged guide (or a tag no layer stands for).
    fn guide_layer(&mut self, gid: GuideId) -> u16 {
        let tags = self.doc.guide_tags(gid);
        if tags.len() > 1 {
            self.note("a guide with more than one tag shows only its first in SketchUp");
        }
        tags.first()
            .and_then(|p| self.layer_slot.get(p).copied())
            .unwrap_or(0)
    }

    /// Every distinct section plane — the document's and each scene's — as
    /// a section plane among the model's own entities. Returns each plane's
    /// pid by its geometry.
    fn section_planes(&mut self, root: &mut RunBuf) -> Vec<(SectionPlaneState, u32)> {
        let mut planes: Vec<(SectionPlaneState, u32)> = Vec::new();
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
            let pid = self.pid();
            root.sections.push(SectionPlane {
                pid,
                plane: skp_plane(s),
                ..Default::default()
            });
            planes.push((*s, pid));
        }
        if let Some(s) = doc_plane.filter(|s| s.active) {
            root.active_section = planes
                .iter()
                .find(|(p, _)| same_plane(p, &s))
                .map(|&(_, pid)| pid);
        }
        planes
    }

    fn scenes(&mut self, planes: &[(SectionPlaneState, u32)]) {
        // Tag stable id -> layer index, through the registry's paths.
        let tag_layer: BTreeMap<u64, usize> = self
            .doc
            .sids()
            .filter_map(|(e, sid)| match e {
                // A scene names layers by index (the slot is what
                // entities carry; a file's own may differ).
                EntityRef::Tag(path) => self.layer_slot.get(path).and_then(|&slot| {
                    let index = self.model.layer_links.iter().find(|l| l.0 == slot)?.1;
                    Some((sid, index))
                }),
                _ => None,
            })
            .collect();
        let scenes: Vec<kernel::Scene> = self.doc.scenes().to_vec();
        let mut displays = Vec::new();
        for s in scenes {
            let mut saved = 0u32;
            let camera = s.camera.as_ref().map(camera);
            if camera.is_some() {
                saved |= SceneProperties::CAMERA;
            }
            let mut hidden_entities = Vec::new();
            if let Some(nodes) = &s.hidden_nodes {
                saved |= SceneProperties::HIDDEN_GEOMETRY
                    | SceneProperties::HIDDEN_GEOMETRY_2
                    | SceneProperties::HIDDEN_OBJECTS;
                hidden_entities = nodes
                    .iter()
                    .filter_map(|sid| self.pids.get(sid).copied())
                    .collect();
            }
            let mut hidden_layers = Vec::new();
            if let Some(tags) = &s.hidden_tags {
                saved |= SceneProperties::VISIBLE_LAYERS;
                hidden_layers = tags
                    .iter()
                    .filter_map(|sid| tag_layer.get(sid).copied())
                    .collect();
            }
            let mut active_section_planes = Vec::new();
            if let Some(section) = s.section {
                saved |= SceneProperties::ACTIVE_SECTION_PLANES;
                if let Some(p) = section {
                    if p.active {
                        active_section_planes.extend(
                            planes
                                .iter()
                                .filter(|(q, _)| same_plane(q, &p))
                                .map(|&(_, pid)| pid),
                        );
                    } else {
                        self.note("a scene's switched-off section plane is saved as no cut");
                    }
                }
            }
            if let Some(d) = s.display {
                displays.push(Value::Array(vec![
                    Value::Text(s.name.clone()),
                    Value::Bool(d.grid),
                    Value::Bool(d.axes),
                    Value::Bool(d.guides),
                ]));
            }
            // The scene's look: a style of SketchUp's defaults with the
            // backdrop and axes the scene captured.
            let rendering = s.view_style.map(|v| {
                saved |= SceneProperties::STYLE_AND_FOG;
                let mut r = openskp::RenderingOptions::sketchup_default();
                style_into(&mut r, v);
                r
            });
            self.model.scenes.push(openskp::Scene {
                name: s.name.clone(),
                description: s.description.clone(),
                saved: SceneProperties(saved),
                camera,
                rendering,
                hidden_entities,
                hidden_layers,
                active_section_planes,
                ..Default::default()
            });
        }
        if !displays.is_empty() {
            self.model_hew.set("scene_display", Value::Array(displays));
        }
    }

    /// Dimensions and leader texts, among the model's own entities. Their
    /// anchors are written as points: a SketchUp dimension attached to an
    /// entity names it by persistent id, which a written Object's vertices
    /// do not carry yet.
    /// Dimensions and leader texts, among the model's own entities, each
    /// with the Hew annotation it came from in its `hew` dictionary (how a
    /// later open, or save back over the file, knows it). Anchors are
    /// written as points: a SketchUp dimension attached to an entity names
    /// it by persistent id, which a written Object's vertices do not carry.
    /// A radius or diameter dimension hangs on an edge of the arc it
    /// measures, inside that Object's group.
    fn annotations(&mut self) {
        let m = |p: Point3| [p.x, p.y, p.z];
        // A point in space is anchor kind 1, as SketchUp writes one; kind
        // 0 is no anchor at all, which SketchUp reads as the origin.
        let free = |p: Point3| openskp::Anchor {
            kind: 1,
            point_m: m(p),
            ..Default::default()
        };
        let mut unwritten = Vec::new();
        for id in self.doc.annotation_ids() {
            if self.kept_annotations.contains(&id) {
                continue;
            }
            let Some(a) = self.doc.annotation(id).cloned() else {
                continue;
            };
            // SketchUp gives an entity one layer: the first tag's.
            let layer = self
                .doc
                .annotation_tags(id)
                .first()
                .and_then(|p| self.layer_slot.get(p).copied())
                .unwrap_or(0);
            let mut hew = HewDict::default();
            hew.set("annotation", dicts::encode_annotation(&a));
            let attached = |x: &kernel::Anchor| x.node.is_some();
            let following =
                "dimensions are placed in SketchUp but do not follow the geometry they measure";
            match a {
                kernel::Annotation::LinearDimension {
                    a,
                    b,
                    offset,
                    plane,
                    text_override,
                } => {
                    let Ok(x) = (b.point - a.point).normalized() else {
                        continue;
                    };
                    if attached(&a) || attached(&b) {
                        self.note(following);
                    }
                    let mut n = plane.normal();
                    // The dimension line lies off the measured line along
                    // n × x; the normal's sign puts the offset on that side.
                    if n.cross(x).dot(offset) < 0.0 {
                        n = n * -1.0;
                    }
                    let pid = self.pid();
                    self.model.dimensions.push(openskp::Dimension {
                        pid,
                        text_override: dimension_text(text_override.as_deref()),
                        start: free(a.point),
                        end: free(b.point),
                        normal: m(Point3::ORIGIN + n),
                        x_axis: m(Point3::ORIGIN + x),
                        offset_m: n.cross(x).dot(offset),
                        aligned: true,
                        layer,
                        ..Default::default()
                    });
                    let i = self.model.dimensions.len() - 1;
                    self.annotation_dict(Owner::Dimension(i), hew);
                }
                kernel::Annotation::LeaderText {
                    anchor,
                    offset,
                    text,
                } => {
                    if attached(&anchor) {
                        self.note(following);
                    }
                    let pid = self.pid();
                    self.model.texts.push(openskp::Text {
                        pid,
                        content: text,
                        anchor: free(anchor.point),
                        leader_offset: [offset.x, offset.y, offset.z],
                        leader: openskp::Leader::Pushpin,
                        arrow: 2,
                        layer,
                        ..Default::default()
                    });
                    let i = self.model.texts.len() - 1;
                    self.annotation_dict(Owner::Text(i), hew);
                }
                kernel::Annotation::RadialDimension {
                    anchor,
                    kind,
                    curve,
                    leader_dir,
                    text_override,
                } => {
                    let n = curve.plane.normal();
                    let on_circle: Vec<&ArcSite> = self
                        .arc_sites
                        .iter()
                        .filter(|s| {
                            let c = Point3::new(
                                s.arc.center_m[0],
                                s.arc.center_m[1],
                                s.arc.center_m[2],
                            );
                            let an = kernel::Vec3::new(
                                s.arc.normal[0],
                                s.arc.normal[1],
                                s.arc.normal[2],
                            );
                            (c - curve.center).length() < 1e-6
                                && (s.arc.radius_m - curve.radius).abs() < 1e-6
                                && an.dot(n).abs() > 1.0 - 1e-6
                        })
                        .collect();
                    let arcs: Vec<&openskp::Arc> = on_circle.iter().map(|s| &s.arc).collect();
                    let site = radial_arc(&arcs, anchor.point)
                        .map(|i| on_circle[i])
                        .map(|s| ArcSite {
                            arc: s.arc.clone(),
                            edge_pid: s.edge_pid,
                            map: s.map,
                        });
                    let Some(site) = site else {
                        self.note(
                            "radius dimensions of circles that are not drawn are kept for Hew only",
                        );
                        unwritten.push(hew.into_value());
                        continue;
                    };
                    if attached(&anchor) {
                        self.note(following);
                    }
                    // The leader's angle in the arc's own frame, and its end
                    // a quarter radius off the curve, out or in as Hew draws
                    // it.
                    let c = Point3::new(
                        site.arc.center_m[0],
                        site.arc.center_m[1],
                        site.arc.center_m[2],
                    );
                    let xa = kernel::Vec3::new(
                        site.arc.x_axis_m[0],
                        site.arc.x_axis_m[1],
                        site.arc.x_axis_m[2],
                    );
                    let ya = kernel::Vec3::new(
                        site.arc.y_axis_m[0],
                        site.arc.y_axis_m[1],
                        site.arc.y_axis_m[2],
                    );
                    let d = anchor.point - c;
                    let angle = d.dot(ya).atan2(d.dot(xa));
                    let out = leader_dir.dot(d) >= 0.0;
                    let off = inches(curve.radius * 0.25) * if out { -1.0 } else { 1.0 };
                    let (map, edge) = (site.map, site.edge_pid);
                    let pid = self.pid();
                    self.model.radial_dimensions.push(openskp::RadialDimension {
                        pid,
                        def_index: Some(map as usize),
                        edge: Some(edge),
                        diameter: kind == kernel::RadialKind::Diameter,
                        leader: [angle, off],
                        text_override: dimension_text(text_override.as_deref()),
                        layer,
                        ..Default::default()
                    });
                    let i = self.model.radial_dimensions.len() - 1;
                    self.annotation_dict(Owner::RadialDimension(i), hew);
                }
            }
        }
        if !unwritten.is_empty() {
            self.model_hew.set("annotations", Value::Array(unwritten));
        }
    }

    fn annotation_dict(&mut self, owner: Owner, hew: HewDict) {
        if let Some(d) = hew.into_dictionary() {
            self.model.dictionaries.push(openskp::Attributes {
                owner,
                dictionaries: vec![d],
            });
        }
    }

    fn view(&mut self) {
        self.model.camera = self.doc.camera_state().as_ref().map(camera);
        // The document's view style sets the backdrop and the axes in the
        // display settings (the rest stays as the file has it, or as a
        // new SketchUp document's); a file without one is written with
        // SketchUp's own look, and says so to a reader of Hew's.
        if let Some(v) = self.doc.view_style() {
            let r = self
                .model
                .rendering
                .get_or_insert_with(openskp::RenderingOptions::sketchup_default);
            style_into(r, v);
            self.model_hew.set("view_style", Value::Bool(true));
        }
        let ax = self.doc.axes();
        let z = ax.x.cross(ax.y);
        self.model.axes = Some(openskp::Axes {
            origin_m: [ax.origin.x, ax.origin.y, ax.origin.z],
            x: [ax.x.x, ax.x.y, ax.x.z],
            y: [ax.y.x, ax.y.y, ax.y.z],
            z: [z.x, z.y, z.z],
        });
    }
}

/// An arc of a world Object, where a radius dimension can hang: its
/// geometry, the persistent id given to one of its edges, and the group
/// definition holding it.
struct ArcSite {
    arc: openskp::Arc,
    edge_pid: u32,
    map: u32,
}

/// Which of a circle's welded arcs a radius dimension hangs on: the one
/// under its `point` (a circle SketchUp drew as several arcs keeps each
/// one's dimension on it), else the longest.
fn radial_arc(arcs: &[&openskp::Arc], point: Point3) -> Option<usize> {
    let holds = |a: &openskp::Arc| {
        let v = |m: [f64; 3]| kernel::Vec3::new(m[0], m[1], m[2]);
        let d = point - Point3::new(a.center_m[0], a.center_m[1], a.center_m[2]);
        // Measured from the arc's start, so a point a hair before its first
        // vertex reads as just below zero rather than wrapping to a full
        // turn.
        let angle = d.dot(v(a.y_axis_m)).atan2(d.dot(v(a.x_axis_m))) - a.start_rad;
        let angle = angle.rem_euclid(std::f64::consts::TAU);
        let angle = if std::f64::consts::TAU - angle < 1e-9 {
            0.0
        } else {
            angle
        };
        angle <= a.end_rad - a.start_rad + 1e-9
    };
    (0..arcs.len()).find(|&i| holds(arcs[i])).or_else(|| {
        (0..arcs.len()).max_by(|&i, &j| {
            let sweep = |a: &openskp::Arc| a.end_rad - a.start_rad;
            sweep(arcs[i]).total_cmp(&sweep(arcs[j]))
        })
    })
}

/// One entity list under construction: a mesh whose vertices are welded by
/// exact position and whose edges are unique per vertex pair, plus the
/// list's placements and helpers.
#[derive(Default)]
struct RunBuf {
    mesh: Mesh,
    at: BTreeMap<[u64; 3], u32>,
    edge_at: BTreeMap<(u32, u32), usize>,
    placed: Vec<PlacedInstance>,
    sections: Vec<SectionPlane>,
    guide_points: Vec<GuidePoint>,
    active_section: Option<u32>,
    curves: Vec<openskp::Curve>,
    /// The file's placed images the list keeps (a save back only).
    images: Vec<openskp::ImagePlacement>,
    /// Dictionaries of faces, by index in `mesh.faces`.
    face_dicts: Vec<(usize, Dictionary)>,
    /// What the list's loose geometry is, for the `hew` dictionary.
    loose: Option<&'static str>,
    /// The planes of the sketches drawn into the list, `[origin, normal]`.
    sketch_planes: Vec<Value>,
    /// Dictionaries of curves, by index in `curves`.
    curve_dicts: Vec<(usize, Dictionary)>,
    /// Each sketch edge's sketch, `(edge index, index into
    /// sketch_planes)`: written per edge when the list holds more than one
    /// sketch, since two sketches can share a plane.
    sketch_edges: Vec<(usize, usize)>,
}

impl RunBuf {
    /// Start from an existing list's geometry (new geometry welds onto it).
    fn seed(&mut self, mesh: &Mesh, curves: &[openskp::Curve]) {
        self.mesh = mesh.clone();
        self.curves = curves.to_vec();
        for (i, v) in mesh.vertices.iter().enumerate() {
            self.at.insert(v.map(|c| (c + 0.0).to_bits()), i as u32);
        }
        for (i, e) in mesh.edges.iter().enumerate() {
            self.edge_at.insert((e.v0.min(e.v1), e.v0.max(e.v1)), i);
        }
    }

    fn vertex(&mut self, p: Point3) -> u32 {
        // -0.0 and 0.0 are one position.
        let key = [p.x, p.y, p.z].map(|c| (c + 0.0).to_bits());
        if let Some(&i) = self.at.get(&key) {
            return i;
        }
        let i = self.mesh.vertices.len() as u32;
        self.mesh.vertices.push([p.x, p.y, p.z]);
        self.at.insert(key, i);
        i
    }

    fn edge(&mut self, a: u32, b: u32, soft: bool) -> Option<usize> {
        if a == b {
            return None;
        }
        let key = (a.min(b), a.max(b));
        if let Some(&i) = self.edge_at.get(&key) {
            let e = &mut self.mesh.edges[i];
            e.soft |= soft;
            e.smooth |= soft;
            return Some(i);
        }
        let i = self.mesh.edges.len();
        self.edge_at.insert(key, i);
        self.mesh.edges.push(MeshEdge {
            v0: a,
            v1: b,
            // SketchUp draws a curved surface's facet seams soft AND
            // smooth; Hew's soft edge is exactly that seam.
            soft,
            smooth: soft,
            ..Default::default()
        });
        Some(i)
    }

    /// Weld chains into curves: each member edge is turned to run along
    /// its chain and names the chain's curve.
    /// Returns the indices of the curves it added.
    fn weld(&mut self, chains: Vec<curves::Chain>) -> std::ops::Range<usize> {
        let first = self.curves.len();
        for c in chains {
            let ci = self.curves.len() as u32;
            for &(e, from, to) in &c.members {
                let edge = &mut self.mesh.edges[e];
                // An edge already in a curve keeps its first one.
                if edge.curve.is_some() {
                    continue;
                }
                edge.v0 = from;
                edge.v1 = to;
                edge.curve = Some(ci);
            }
            self.curves.push(c.curve);
        }
        first..self.curves.len()
    }

    fn face(
        &mut self,
        outer: Vec<u32>,
        holes: Vec<Vec<u32>>,
        normal: [f64; 3],
        material: Option<u16>,
    ) {
        // Every ring side needs its edge (the writer frames loops by edge
        // use); a kernel face's sides are its edges already, a sketch
        // region's too — this only guards against a degenerate ring.
        for ring in std::iter::once(&outer).chain(holes.iter()) {
            if ring.len() < 3 {
                return;
            }
            for k in 0..ring.len() {
                let _ = self.edge(ring[k], ring[(k + 1) % ring.len()], false);
            }
        }
        // OpenSKP's rings all run counter-clockwise about the normal, holes
        // included (its writer turns holes back to SketchUp's clockwise
        // storage); Hew's holes run clockwise.
        let holes = holes
            .into_iter()
            .map(|mut h| {
                h.reverse();
                h
            })
            .collect();
        self.mesh.faces.push(MeshFace {
            outer,
            holes,
            normal,
            front_material: material,
            ..Default::default()
        });
    }

    fn finish(self, def: Option<u32>) -> GeometryRun {
        GeometryRun {
            def_index: def.map(|d| d as usize),
            mesh: self.mesh,
            placed: self.placed,
            sections: self.sections,
            guide_points: self.guide_points,
            active_section: self.active_section,
            curves: self.curves,
            images: self.images,
            ..Default::default()
        }
    }
}

/// The circle a chord `pa`–`pb` of a cylinder's rim lies on: the chord
/// must be square to the axis (both ends at one height) with both ends on
/// the cylinder. `None` for any other edge of the wall (a seam, a slanted
/// cut).
fn rim_circle(wall: &kernel::SurfaceRef, pa: Point3, pb: Point3) -> Option<kernel::CurveGeom> {
    let kernel::SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    } = *wall;
    let ha = (pa - axis_point).dot(axis);
    let hb = (pb - axis_point).dot(axis);
    let tol = 1e-6 * radius.max(1.0);
    if (ha - hb).abs() > tol {
        return None;
    }
    let center = axis_point + axis * ha;
    let on = |p: Point3| ((p - center).length() - radius).abs() <= tol;
    (on(pa) && on(pb)).then_some(kernel::CurveGeom { center, radius })
}

/// Hew's own placement of a texture on a face without a UV frame: the
/// planar projection onto the face's plane basis, one tile per
/// `world_size` (the tessellator's rule, as a frame).
fn planar_frame(n: kernel::Vec3, world_size: [f64; 2]) -> kernel::UvFrame {
    let (u, v) = tessellate::plane_basis(n);
    kernel::UvFrame::new(
        u * (1.0 / world_size[0]),
        v * (1.0 / world_size[1]),
        0.0,
        0.0,
    )
}

/// A Hew UV frame as `.skp`'s face texture placement (`SKP_FORMAT.md` §4v):
/// the projective matrix `K` whose inverse takes face-local inches
/// `(x, y, 1)` to texture inches, so that texture inches ÷ the applied
/// size `size` are the frame's UV. The frame is affine, so `K` is too.
///
/// A reader measures each corner in the face's plane only (`x`, `y`, not
/// its height off the plane), so on a face a hair off planar (a large
/// model's rounded coordinates) the frame's slope along `n` would shift
/// the corners' UV. Two placements are tried against the UV Hew draws at
/// each of the face's `corners`, and the closer one is written: the
/// frame's own map taken at the corners' mean height, and a map fitted to
/// the corners (exact for a triangle, but ill-conditioned on a sliver).
fn face_texture(
    frame: &kernel::UvFrame,
    n: kernel::Vec3,
    corners: &[Point3],
    size: (f64, f64),
) -> Option<openskp::FaceTexture> {
    let (u_axis, v_axis) = face_frame(n);
    let inch = |p: Point3| p.to_vec() * (1.0 / 0.0254);
    let local = |p: Point3| (inch(p).dot(u_axis), inch(p).dot(v_axis));
    let want = |p: Point3| {
        let uv = frame.apply(p);
        [uv[0] * size.0, uv[1] * size.1]
    };
    // How far the reader's UV, in texture inches, lands from Hew's.
    let miss = |kinv: &[f64; 9]| {
        corners.iter().fold(0.0f64, |worst, &p| {
            let (x, y) = local(p);
            let t = want(p);
            let got = [
                x * kinv[0] + y * kinv[3] + kinv[6],
                x * kinv[1] + y * kinv[4] + kinv[7],
            ];
            worst.max((got[0] - t[0]).abs()).max((got[1] - t[1]).abs())
        })
    };
    // A face point in inches is x·u + y·v + d·n; UV is s·p_m + u0 with
    // p_m = p_in · 0.0254.
    let k = corners.len().max(1) as f64;
    let d = corners.iter().map(|&p| inch(p).dot(n)).sum::<f64>() / k;
    let m = 0.0254;
    let (s, t) = (frame.s, frame.t);
    let own = [
        size.0 * m * s.dot(u_axis),
        size.1 * m * t.dot(u_axis),
        0.0,
        size.0 * m * s.dot(v_axis),
        size.1 * m * t.dot(v_axis),
        0.0,
        size.0 * (m * d * s.dot(n) + frame.u0),
        size.1 * (m * d * t.dot(n) + frame.v0),
        1.0,
    ];
    let kinv = match fitted_map(corners.iter().map(|&p| (local(p), want(p)))) {
        Some(fit) if miss(&fit) < miss(&own) => fit,
        _ => own,
    };
    let k = inv3(&kinv)?;
    Some(openskp::FaceTexture {
        front: k,
        back: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        front_extra: [0.0; 3],
        back_extra: [0.0; 3],
        front_pins: Vec::new(),
        back_pins: Vec::new(),
        flags: [0, 0],
    })
}

/// The affine map (as `face_texture`'s `K⁻¹`) taking each face-local
/// point to its texture inches, by least squares on centered coordinates;
/// `None` for fewer than three points or (nearly) collinear ones.
fn fitted_map(pts: impl Iterator<Item = ((f64, f64), [f64; 2])>) -> Option<[f64; 9]> {
    let pts: Vec<_> = pts.collect();
    if pts.len() < 3 {
        return None;
    }
    let k = pts.len() as f64;
    let (mx, my) = pts
        .iter()
        .fold((0.0, 0.0), |(x, y), ((px, py), _)| (x + px / k, y + py / k));
    let mt = [0, 1].map(|i| pts.iter().map(|(_, t)| t[i] / k).sum::<f64>());
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    let (mut sxt, mut syt) = ([0.0; 2], [0.0; 2]);
    for ((x, y), t) in &pts {
        let (dx, dy) = (x - mx, y - my);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
        for i in 0..2 {
            sxt[i] += dx * (t[i] - mt[i]);
            syt[i] += dy * (t[i] - mt[i]);
        }
    }
    let det = sxx * syy - sxy * sxy;
    if !(det.is_finite() && det > 1e-12 * (sxx * syy).max(f64::MIN_POSITIVE)) {
        return None;
    }
    let ab = |i: usize| {
        let a = (sxt[i] * syy - syt[i] * sxy) / det;
        let b = (syt[i] * sxx - sxt[i] * sxy) / det;
        (a, b, mt[i] - a * mx - b * my)
    };
    let ((au, bu, cu), (av, bv, cv)) = (ab(0), ab(1));
    Some([au, av, 0.0, bu, bv, 0.0, cu, cv, 1.0])
}

/// The face-local texture frame `.skp` measures placements in (§4v): a
/// horizontal face takes `v = +Y`; any other takes world Z projected onto
/// its plane; `u = v × n`.
fn face_frame(n: kernel::Vec3) -> (kernel::Vec3, kernel::Vec3) {
    let v = if n.z.abs() > 1.0 - 1e-9 {
        kernel::Vec3::new(0.0, 1.0, 0.0)
    } else {
        let raw = kernel::Vec3::new(-n.z * n.x, -n.z * n.y, 1.0 - n.z * n.z);
        raw * (1.0 / raw.length())
    };
    (v.cross(n), v)
}

/// Row-major 3×3 inverse; `None` when singular.
fn inv3(k: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = *k;
    let ca = e * i - f * h;
    let cb = -(d * i - f * g);
    let cc = d * h - e * g;
    let det = a * ca + b * cb + c * cc;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    Some([
        ca / det,
        -(b * i - c * h) / det,
        (b * f - c * e) / det,
        cb / det,
        (a * i - c * g) / det,
        -(a * f - c * d) / det,
        cc / det,
        -(a * h - b * g) / det,
        (a * e - b * d) / det,
    ])
}

/// A Hew material as a `.skp` material record named `name`.
fn material_record(name: &str, m: &kernel::Material) -> Material {
    let name = name.to_string();
    let opacity = f64::from(m.color.a) / 255.0;
    let rgba = [m.color.r, m.color.g, m.color.b, 255];
    match &m.texture {
        None => Material::Solid {
            name,
            rgba,
            opacity,
            colorize_type: 0,
        },
        Some(t) => {
            let ext = match t.format {
                kernel::ImageFormat::Png => "png",
                kernel::ImageFormat::Jpeg => "jpg",
            };
            Material::Textured {
                texture: Some(format!("{name}.{ext}")),
                name,
                applied_size_in: Some((inches(t.world_size[0]), inches(t.world_size[1]))),
                jpeg_quality: None,
                image_bytes: Some(t.image.clone()),
                // SketchUp draws the material in its image's average color
                // with textures off (Hew's own color is the white tint).
                avg_rgba: Some(texture::average_rgba(&t.image, t.format).unwrap_or(rgba)),
                colorized: false,
                texture_avg_rgba: None,
                colorize_type: 0,
                opacity,
            }
        }
    }
}

/// Meters -> the inches whose meter value is exactly `m` again where one
/// exists (the reader multiplies by 0.0254): the quotient, or a neighbor.
fn inches(m: f64) -> f64 {
    let q = m / 0.0254;
    for c in [q, q.next_up(), q.next_down()] {
        if c * 0.0254 == m {
            return c;
        }
    }
    q
}

/// A kernel pose as `.skp`'s 13-f64 transform: the 3×3 row-major (first
/// row first), the translation in inches, and the weight.
fn transform13(t: &Transform) -> [f64; 13] {
    let a = t.to_affine();
    [
        a[0],
        a[1],
        a[2],
        a[4],
        a[5],
        a[6],
        a[8],
        a[9],
        a[10],
        inches(a[3]),
        inches(a[7]),
        inches(a[11]),
        1.0,
    ]
}

fn camera(c: &kernel::CameraState) -> openskp::Camera {
    let eye = [c.eye.x, c.eye.y, c.eye.z];
    let target = [c.target.x, c.target.y, c.target.z];
    // SketchUp keeps the camera's up square to the view; Hew may hold the
    // world up it was given. The same view either way.
    let view = (c.target - c.eye).normalized().ok();
    let up = view
        .and_then(|v| (c.up - v * c.up.dot(v)).normalized().ok())
        .unwrap_or(c.up);
    openskp::Camera {
        eye_m: eye,
        target_m: target,
        up: [up.x, up.y, up.z],
        perspective: c.projection == CameraProjection::Perspective,
        fov_deg: c.fov_deg,
        parallel_height_m: c.parallel_height,
        two_point_perspective: false,
        description: String::new(),
        undecoded: [1.0, 1000.0],
        aspect_ratio: 0.0,
        scale_2d: 1.0,
        offset_2d: [0.0, 0.0],
    }
}

/// A Hew cut as a SketchUp plane `[a, b, c, d]` (inches): SketchUp's
/// normal points at the side the cut keeps, a Hew cut's at the side it
/// removes, so the normal turns over.
pub(crate) fn skp_plane(s: &SectionPlaneState) -> [f64; 4] {
    let n = -s.normal;
    let d = -(n.x * inches(s.origin.x) + n.y * inches(s.origin.y) + n.z * inches(s.origin.z));
    [n.x, n.y, n.z, d]
}

fn same_plane(a: &SectionPlaneState, b: &SectionPlaneState) -> bool {
    a.origin == b.origin && a.normal == b.normal
}

/// `name`, or `name` with a SketchUp-style `#n` suffix when another
/// material already took it; an empty name falls back to `fallback`.
fn unique_name(name: &str, fallback: &str, taken: &mut BTreeSet<String>) -> String {
    let base = if name.trim().is_empty() {
        fallback
    } else {
        name
    };
    let mut candidate = base.to_string();
    let mut n = 1;
    while taken.contains(&candidate) {
        candidate = format!("{base}#{n}");
        n += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

/// A display color for layer `i`: SketchUp gives each new layer a color
/// of its own; these cycle through a fixed set.
fn layer_color(i: usize) -> [u8; 4] {
    const SET: [[u8; 4]; 6] = [
        [255, 84, 84, 255],
        [84, 168, 255, 255],
        [120, 200, 90, 255],
        [255, 190, 60, 255],
        [170, 110, 220, 255],
        [70, 200, 200, 255],
    ];
    SET[i % SET.len()]
}

/// Sets the part of a style Hew keeps (`kernel::ViewStyle`) into a
/// file's rendering options: the backdrop and the model axes; the rest
/// stays as it is.
pub(crate) fn style_into(r: &mut openskp::RenderingOptions, v: kernel::ViewStyle) {
    let rgba = |c: [u8; 3]| [c[0], c[1], c[2], 255];
    r.background = rgba(v.background);
    r.sky = v.sky.is_some();
    if let Some(c) = v.sky {
        r.sky_color = rgba(c);
    }
    r.ground = v.ground.is_some();
    if let Some(c) = v.ground {
        r.ground_color = rgba(c);
    }
    r.model_axes = v.axes;
}

/// A guide line's stretch as a `.skp` file holds it: line parameters in
/// inches along the (unit) direction, ±1e30 for an end that runs on
/// forever.
pub(crate) fn guide_bounds(span: kernel::GuideSpan) -> [f64; 2] {
    [
        span.from.map_or(-1e30, |a| a * openskp::INCH),
        span.to.map_or(1e30, |b| b * openskp::INCH),
    ]
}

/// A `.skp` guide line's stretch in meters along its direction once
/// posed, `direction_length` being how long the file's direction is after
/// the pose (the inverse of [`guide_bounds`]).
pub(crate) fn file_guide_span(bounds: [f64; 2], direction_length: f64) -> kernel::GuideSpan {
    const OPEN: f64 = 1.0e29;
    let end = |b: f64| (b.abs() < OPEN).then_some(b / openskp::INCH * direction_length);
    kernel::GuideSpan {
        from: end(bounds[0]),
        to: end(bounds[1]),
    }
}

/// Whether a file guide line, at `p` along unit `d` over `file`, is the
/// same stretch of the same line as a document guide: collinear, and the
/// same ends once read along the document guide's direction (either way
/// round).
pub(crate) fn same_guide_line(
    p: kernel::Point3,
    d: kernel::Vec3,
    file: kernel::GuideSpan,
    origin: kernel::Point3,
    direction: kernel::Vec3,
    span: kernel::GuideSpan,
) -> bool {
    let along = (origin - p).cross(d).length() < 1e-6 && d.cross(direction).length() < 1e-6;
    if !along {
        return false;
    }
    let off = (p - origin).dot(direction);
    let mapped = if d.dot(direction) > 0.0 {
        kernel::GuideSpan {
            from: file.from.map(|a| off + a),
            to: file.to.map(|b| off + b),
        }
    } else {
        kernel::GuideSpan {
            from: file.to.map(|b| off - b),
            to: file.from.map(|a| off - a),
        }
    };
    let end = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-6,
        (None, None) => true,
        _ => false,
    };
    end(mapped.from, span.from) && end(mapped.to, span.to)
}

/// The text a dimension record carries in a `.skp`: Hew's override as typed,
/// minus a leading `~` (Hew's mark for a rounded readout, which SketchUp
/// has no use for). No override is the empty string, which SketchUp reads as
/// "show the measured length".
fn dimension_text(text: Option<&str>) -> String {
    let t = text.unwrap_or_default();
    t.strip_prefix('~').map_or(t, str::trim_start).to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn dimension_text_drops_a_leading_tilde() {
        assert_eq!(dimension_text(None), "");
        assert_eq!(dimension_text(Some("1' 2\"")), "1' 2\"");
        assert_eq!(dimension_text(Some("~3/16\"")), "3/16\"");
        assert_eq!(dimension_text(Some("~ 0.33 m")), "0.33 m");
        assert_eq!(dimension_text(Some("~")), "");
        // Only a leading mark goes.
        assert_eq!(dimension_text(Some("about ~3")), "about ~3");
    }

    use super::*;

    #[test]
    fn a_placement_holds_every_corners_uv_on_a_face_a_hair_off_planar() {
        // A tilted triangle far from the origin, one corner 2 µm off the
        // plane the others share (a large model's rounded coordinates),
        // under a frame that slopes along the normal: a reader measuring
        // corners in the plane gets back the UV Hew draws at each one.
        let n = kernel::Vec3::new(0.0, -0.6, 0.8);
        let corners = [
            Point3::new(-8.98, 0.44, 32.36),
            Point3::new(-8.90, 1.32, 33.02),
            Point3::new(-8.95, 0.88, 32.69) + n * 2e-6,
        ];
        let frame = kernel::UvFrame::new(
            kernel::Vec3::new(4.9, 0.3, 1.1),
            kernel::Vec3::new(-0.2, 3.1, 2.7),
            0.25,
            -0.5,
        );
        let size = (24.0, 36.0);
        let k = face_texture(&frame, n, &corners, size)
            .expect("a placement")
            .front;
        let kinv = inv3(&k).unwrap();
        let (u_axis, v_axis) = face_frame(n);
        for p in corners {
            let q = p.to_vec() * (1.0 / 0.0254);
            let (x, y) = (q.dot(u_axis), q.dot(v_axis));
            let w = x * kinv[2] + y * kinv[5] + kinv[8];
            let got = [
                (x * kinv[0] + y * kinv[3] + kinv[6]) / w / size.0,
                (x * kinv[1] + y * kinv[4] + kinv[7]) / w / size.1,
            ];
            let want = frame.apply(p);
            for i in 0..2 {
                assert!((got[i] - want[i]).abs() < 1e-9, "{got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn a_unit_change_keeps_a_files_other_unit_settings() {
        let own = openskp::Units {
            angle_precision: 3,
            snap_angle: 7.5,
            angle_snap: false,
            suppress_units_display: true,
            force_inch_display: true,
            length_snap: false,
            ..openskp::Units::sketchup_default()
        };
        let u = DisplayUnits::Millimeters.over(own.clone());
        assert_eq!(
            (u.length_format, u.length_unit, u.length_precision),
            (0, 2, 1)
        );
        assert_eq!(
            (
                u.angle_precision,
                u.snap_angle,
                u.angle_snap,
                u.suppress_units_display,
                u.force_inch_display,
                u.length_snap
            ),
            (3, 7.5, false, true, true, false)
        );
    }

    #[test]
    fn a_radius_dimension_hangs_on_the_arc_under_its_point() {
        // A unit circle at the origin welded as two arcs, 0°–90° and
        // 90°–360°; a point at 45° is on the first, one at 200° on the
        // second, and with no arc under the point, the longest.
        let arc = |from_deg: f64, sweep_deg: f64| {
            let t = from_deg.to_radians();
            openskp::Arc {
                center_m: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                x_axis_m: [t.cos(), t.sin(), 0.0],
                y_axis_m: [-t.sin(), t.cos(), 0.0],
                radius_m: 1.0,
                start_rad: 0.0,
                end_rad: sweep_deg.to_radians(),
            }
        };
        let (a, b) = (arc(0.0, 90.0), arc(90.0, 270.0));
        let at = |deg: f64| {
            let t = f64::to_radians(deg);
            Point3::new(t.cos(), t.sin(), 0.0)
        };
        assert_eq!(radial_arc(&[&b, &a], at(45.0)), Some(1));
        assert_eq!(radial_arc(&[&a, &b], at(200.0)), Some(1));
        let (c, d) = (arc(0.0, 30.0), arc(60.0, 60.0));
        assert_eq!(radial_arc(&[&c, &d], at(45.0)), Some(1));
        assert_eq!(radial_arc(&[], at(45.0)), None);
        // A point on an arc's first vertex, a hair before it in float
        // noise, is still on that arc.
        let (e, f) = (arc(0.0, 120.0), arc(180.0, 90.0));
        let t = 180f64.to_radians() - 1e-12;
        let p = Point3::new(t.cos(), t.sin(), 0.0);
        assert_eq!(radial_arc(&[&e, &f], p), Some(1));
    }

    #[test]
    fn a_sliver_keeps_its_texture_placed_exactly() {
        // A planar triangle a micrometre wide and ten meters long, on the
        // diagonal: too thin to fit a map to, so the frame's own map is
        // written, exact at every corner.
        let n = kernel::Vec3::new(0.0, 0.0, 1.0);
        let d = kernel::Vec3::new(1.0, 1.0, 0.0) * std::f64::consts::FRAC_1_SQRT_2;
        let w = kernel::Vec3::new(-1.0, 1.0, 0.0) * std::f64::consts::FRAC_1_SQRT_2;
        let o = Point3::new(3.0, -2.0, 0.5);
        let corners = [o, o + d * 10.0, o + d * 5.0 + w * 1e-6];
        let frame = kernel::UvFrame::new(
            kernel::Vec3::new(1.0, 0.2, 0.1),
            kernel::Vec3::new(-0.1, 0.9, 0.3),
            0.25,
            -0.5,
        );
        let size = (24.0, 36.0);
        let k = face_texture(&frame, n, &corners, size)
            .expect("a placement")
            .front;
        let kinv = inv3(&k).unwrap();
        let (u_axis, v_axis) = face_frame(n);
        for p in corners {
            let q = p.to_vec() * (1.0 / 0.0254);
            let (x, y) = (q.dot(u_axis), q.dot(v_axis));
            let got = [
                (x * kinv[0] + y * kinv[3] + kinv[6]) / size.0,
                (x * kinv[1] + y * kinv[4] + kinv[7]) / size.1,
            ];
            let want = frame.apply(p);
            for i in 0..2 {
                assert!((got[i] - want[i]).abs() < 1e-9, "{got:?} vs {want:?}");
            }
        }
    }
}
