//! The `.skp` a document was opened from or last saved as, and where each
//! of its nodes and definitions came from in it: what a save back to that
//! file builds on (see [`crate::save`]).
//!
//! Change is detected by content: each node and definition is hashed when
//! the file is opened (and again after each save), and a save reuses the
//! file's own records for whatever still hashes the same.

use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    AnnotationId, AttrTarget, ComponentId, Document, EntityRef, NodeId, Object, Point3, SketchId,
};
pub use skp_import::{AnnotationOrigin, Origin};

/// A source file and its provenance.
#[derive(Debug, Clone)]
pub struct SkpSource {
    /// The file.
    pub bytes: Vec<u8>,
    /// Node stable id -> where it came from.
    pub nodes: BTreeMap<u64, NodeSource>,
    /// Component definition stable id -> where it came from.
    pub defs: BTreeMap<u64, DefSource>,
    /// How many Objects stood for the file's loose geometry.
    pub loose: usize,
    /// Annotation handle -> the SketchUp text or dimension it was read
    /// from, while the file still holds it as read.
    pub annotations: BTreeMap<u64, AnnotationSource>,
    /// Persistent ids of the file's placements the document never read in
    /// (a group of edges or text only, a component with nothing Hew
    /// imports), which a save back carries as they are. `None` until the
    /// first save works it out from what `nodes` read in; each save hands
    /// it on, since what it writes keeps no record of having read them.
    pub unread: Option<BTreeSet<u32>>,
    /// Tag stable id -> the file layer it stands for, by name: a renamed
    /// tag renames its layer in place, and a layer no tag stands for goes
    /// when nothing written uses it.
    pub layers: BTreeMap<u64, String>,
    /// Sketch stable id -> its drawing's hash, for the sketches read from
    /// the file's loose edges: kept as those edges while unchanged.
    pub loose_sketches: BTreeMap<u64, u64>,
}

/// Where one annotation came from, and its content then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationSource {
    pub origin: AnnotationOrigin,
    /// [`annotation_hash`].
    pub hash: u64,
}

/// The live annotation with stable id `sid`, if any.
pub fn annotation_of(doc: &Document, sid: u64) -> Option<AnnotationId> {
    doc.annotation_ids()
        .into_iter()
        .find(|&id| doc.sid_of(&EntityRef::Annotation(id)) == Some(sid))
}

/// Where one node came from, and its content then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSource {
    pub origin: Origin,
    /// An Object's geometry hash ([`geometry_hash`]); 0 for other nodes.
    pub geometry: u64,
    /// The whole node's hash ([`node_hash`]).
    pub node: u64,
}

/// Where one definition came from (a material variant of a file
/// definition), and its content then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefSource {
    /// The file definition's name (unique among SketchUp components).
    pub name: String,
    /// The paint its placements passed down.
    pub slot: u16,
    /// [`def_hash`].
    pub hash: u64,
}

impl SkpSource {
    /// The provenance of a document just built by importing `bytes`:
    /// `provenance` from the importer, `report` from `Document::ingest`.
    pub fn after_open(
        bytes: Vec<u8>,
        doc: &Document,
        provenance: &skp_import::Provenance,
        report: &kernel::ImportReport,
    ) -> SkpSource {
        let mut origins: BTreeMap<u64, Vec<Origin>> = BTreeMap::new();
        for &(k, o) in &provenance.nodes {
            origins.entry(k).or_default().push(o);
        }
        let mut nodes = BTreeMap::new();
        let mut loose = 0;
        for (key, built) in &report.keys {
            for &node in built {
                let origin = origins.get(key).and_then(|os| {
                    os.iter().copied().find(|o| {
                        matches!(
                            (node, o),
                            (
                                NodeId::Object(_),
                                Origin::Loose
                                    | Origin::GroupGeometry { .. }
                                    | Origin::GroupOwnGeometry { .. }
                                    | Origin::DefinitionGeometry { .. },
                            ) | (NodeId::Group(_), Origin::Group { .. })
                                | (
                                    NodeId::Instance(_),
                                    Origin::Instance { .. } | Origin::Image { .. }
                                )
                        )
                    })
                });
                let (Some(origin), Some(sid)) = (origin, doc.sid_of(&entity(node))) else {
                    continue;
                };
                if origin == Origin::Loose {
                    loose += 1;
                }
                nodes.insert(
                    sid,
                    NodeSource {
                        origin,
                        geometry: match node {
                            NodeId::Object(o) => {
                                doc.object(o).map_or(0, |ob| geometry_hash(doc, ob))
                            }
                            _ => 0,
                        },
                        node: node_hash(doc, node),
                    },
                );
            }
        }
        let mut defs = BTreeMap::new();
        for (recipe, name, slot) in &provenance.defs {
            let Some(Some(cid)) = report.defs.get(*recipe) else {
                continue;
            };
            if let Some(sid) = doc.sid_of(&EntityRef::Component(*cid)) {
                defs.insert(
                    sid,
                    DefSource {
                        name: name.clone(),
                        slot: *slot,
                        hash: def_hash(doc, *cid),
                    },
                );
            }
        }
        let mut annotations = BTreeMap::new();
        for &(index, origin) in &provenance.annotations {
            if let Some(Some(id)) = report.annotations.get(index)
                && let Some(sid) = doc.sid_of(&EntityRef::Annotation(*id))
            {
                annotations.insert(
                    sid,
                    AnnotationSource {
                        origin,
                        hash: annotation_hash(doc, *id),
                    },
                );
            }
        }
        let layers = provenance
            .tags
            .iter()
            .filter_map(|(path, name)| {
                Some((doc.sid_of(&EntityRef::Tag(path.clone()))?, name.clone()))
            })
            .collect();
        let loose_sketches = provenance
            .loose_sketches
            .iter()
            .filter_map(|&i| {
                let id = (*report.sketches.get(i)?)?;
                let sid = doc.sid_of(&EntityRef::Sketch(id))?;
                Some((sid, sketch_hash(doc, id)?))
            })
            .collect();
        SkpSource {
            bytes,
            nodes,
            defs,
            loose,
            annotations,
            unread: None,
            layers,
            loose_sketches,
        }
    }

    /// The source as one opaque blob (for crash recovery): a magic line,
    /// the provenance as JSON, then the file.
    pub fn to_blob(&self) -> Vec<u8> {
        let nodes: Vec<serde_json::Value> = self
            .nodes
            .iter()
            .map(|(sid, n)| {
                let (kind, a, b) = origin_parts(n.origin);
                serde_json::json!([sid, kind, a, b, n.geometry.to_string(), n.node.to_string()])
            })
            .collect();
        let defs: Vec<serde_json::Value> = self
            .defs
            .iter()
            .map(|(sid, d)| serde_json::json!([sid, d.name, d.slot, d.hash.to_string()]))
            .collect();
        let annotations: Vec<serde_json::Value> = self
            .annotations
            .iter()
            .map(|(sid, a)| {
                let (kind, index, pid) = annotation_origin_parts(a.origin);
                serde_json::json!([sid.to_string(), kind, index, pid, a.hash.to_string()])
            })
            .collect();
        let layers: Vec<serde_json::Value> = self
            .layers
            .iter()
            .map(|(sid, name)| serde_json::json!([sid, name]))
            .collect();
        let loose_sketches: Vec<serde_json::Value> = self
            .loose_sketches
            .iter()
            .map(|(sid, hash)| serde_json::json!([sid, hash.to_string()]))
            .collect();
        let json = serde_json::json!({
            "nodes": nodes,
            "defs": defs,
            "loose": self.loose,
            "annotations": annotations,
            "unread": self.unread,
            "layers": layers,
            "loose_sketches": loose_sketches,
        })
        .to_string();
        let mut out = Vec::with_capacity(MAGIC.len() + 8 + json.len() + self.bytes.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(json.as_bytes());
        out.extend_from_slice(&self.bytes);
        out
    }

    /// [`SkpSource::to_blob`] read back; `None` for anything else.
    pub fn from_blob(blob: &[u8]) -> Option<SkpSource> {
        let rest = blob.strip_prefix(MAGIC)?;
        let len = u64::from_le_bytes(rest.get(..8)?.try_into().ok()?) as usize;
        let json = std::str::from_utf8(rest.get(8..8usize.checked_add(len)?)?).ok()?;
        let bytes = rest.get(8 + len..)?.to_vec();
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let num = |x: &serde_json::Value| -> Option<u64> {
            x.as_u64().or_else(|| x.as_str()?.parse().ok())
        };
        let mut nodes = BTreeMap::new();
        for n in v.get("nodes")?.as_array()? {
            let n = n.as_array()?;
            let origin = origin_of(n.get(1)?.as_str()?, num(n.get(2)?)?, num(n.get(3)?)?)?;
            nodes.insert(
                num(n.first()?)?,
                NodeSource {
                    origin,
                    geometry: num(n.get(4)?)?,
                    node: num(n.get(5)?)?,
                },
            );
        }
        let mut defs = BTreeMap::new();
        for d in v.get("defs")?.as_array()? {
            let d = d.as_array()?;
            defs.insert(
                num(d.first()?)?,
                DefSource {
                    name: d.get(1)?.as_str()?.to_string(),
                    slot: u16::try_from(num(d.get(2)?)?).ok()?,
                    hash: num(d.get(3)?)?,
                },
            );
        }
        let mut annotations = BTreeMap::new();
        // Absent from a blob written before annotations were read.
        for a in v
            .get("annotations")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            let a = a.as_array()?;
            let index = usize::try_from(num(a.get(2)?)?).ok()?;
            let pid = u32::try_from(num(a.get(3)?)?).ok()?;
            let origin = match a.get(1)?.as_str()? {
                "text" => AnnotationOrigin::Text { index, pid },
                "dimension" => AnnotationOrigin::Dimension { index, pid },
                "radial_dimension" => AnnotationOrigin::RadialDimension { index, pid },
                _ => return None,
            };
            annotations.insert(
                num(a.first()?)?,
                AnnotationSource {
                    origin,
                    hash: num(a.get(4)?)?,
                },
            );
        }
        let mut layers = BTreeMap::new();
        for l in v
            .get("layers")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            let l = l.as_array()?;
            layers.insert(num(l.first()?)?, l.get(1)?.as_str()?.to_string());
        }
        let mut loose_sketches = BTreeMap::new();
        for l in v
            .get("loose_sketches")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            let l = l.as_array()?;
            loose_sketches.insert(num(l.first()?)?, num(l.get(1)?)?);
        }
        Some(SkpSource {
            bytes,
            nodes,
            defs,
            loose: usize::try_from(num(v.get("loose")?)?).ok()?,
            annotations,
            // Absent (or null) until a save worked it out.
            unread: match v.get("unread") {
                Some(serde_json::Value::Array(u)) => Some(
                    u.iter()
                        .map(|p| u32::try_from(num(p)?).ok())
                        .collect::<Option<_>>()?,
                ),
                _ => None,
            },
            layers,
            loose_sketches,
        })
    }
}

// Version 2: annotations are keyed by their stable id (a version 1 blob
// keyed them by a runtime handle, which a reload renumbers).
const MAGIC: &[u8] = b"HEW-SKP-SOURCE 2\n";

fn origin_parts(o: Origin) -> (&'static str, u64, u64) {
    match o {
        Origin::Loose => ("loose", 0, 0),
        Origin::GroupGeometry { pid } => ("group_geometry", pid.into(), 0),
        Origin::GroupOwnGeometry { pid } => ("group_own_geometry", pid.into(), 0),
        Origin::Group { pid } => ("group", pid.into(), 0),
        Origin::Instance { pid } => ("instance", pid.into(), 0),
        Origin::Image { pid } => ("image", pid.into(), 0),
        Origin::DefinitionGeometry { definition, slot } => {
            ("definition_geometry", definition as u64, slot.into())
        }
    }
}

fn annotation_origin_parts(o: AnnotationOrigin) -> (&'static str, usize, u32) {
    match o {
        AnnotationOrigin::Text { index, pid } => ("text", index, pid),
        AnnotationOrigin::Dimension { index, pid } => ("dimension", index, pid),
        AnnotationOrigin::RadialDimension { index, pid } => ("radial_dimension", index, pid),
    }
}

fn origin_of(kind: &str, a: u64, b: u64) -> Option<Origin> {
    let pid = || u32::try_from(a).ok();
    Some(match kind {
        "loose" => Origin::Loose,
        "group_geometry" => Origin::GroupGeometry { pid: pid()? },
        "group_own_geometry" => Origin::GroupOwnGeometry { pid: pid()? },
        "group" => Origin::Group { pid: pid()? },
        "instance" => Origin::Instance { pid: pid()? },
        "image" => Origin::Image { pid: pid()? },
        "definition_geometry" => Origin::DefinitionGeometry {
            definition: usize::try_from(a).ok()?,
            slot: u16::try_from(b).ok()?,
        },
        _ => return None,
    })
}

pub(crate) fn entity(node: NodeId) -> EntityRef {
    match node {
        NodeId::Object(id) => EntityRef::Object(id),
        NodeId::Group(id) => EntityRef::Group(id),
        NodeId::Instance(id) => EntityRef::Instance(id),
    }
}

// ── Content hashes ────────────────────────────────────────────────────────
// FNV-1a over a canonical byte form: deterministic, order-independent where
// order carries no meaning (faces, edges, members), exact (bit patterns,
// never rounded) so any edit at all reads as a change.

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn eat(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn u64(&mut self, v: u64) {
        self.eat(&v.to_le_bytes());
    }
    fn f(&mut self, v: f64) {
        self.u64((v + 0.0).to_bits());
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.eat(s.as_bytes());
    }
}

/// A material by identity, not content: a save maps the document's
/// materials onto the file's by name and writes their colors there, so a
/// recolored material is not a change to the geometry painted with it.
fn material_hash(doc: &Document, m: Option<kernel::MaterialId>) -> u64 {
    let mut h = Fnv::new();
    match m.and_then(|id| doc.materials().get(id).map(|mat| (id, mat))) {
        Some((id, mat)) => {
            h.str(&mat.name);
            // Same-named materials are told apart by their order among
            // those.
            let ordinal = doc
                .materials()
                .iter()
                .take_while(|(other, _)| *other != id)
                .filter(|(_, o)| o.name == mat.name)
                .count();
            // The first of a name hashes as it always has, so hashes
            // stored in existing files stay valid.
            if ordinal > 0 {
                h.u64(ordinal as u64);
            }
        }
        None => h.u64(0),
    }
    h.0
}

/// An Object's geometry: every face (rings from their least vertex, holes,
/// effective material, UV frame, surface claim) and edge (ends, soft flag,
/// circle claim), and its base material — order-independent.
pub fn geometry_hash(doc: &Document, obj: &Object) -> u64 {
    let he = obj.half_edges();
    let v = obj.vertices();
    let pos = |id: kernel::VertexId| {
        let p = v[id].position;
        [p.x, p.y, p.z].map(|c| (c + 0.0).to_bits())
    };
    let ring = |l: kernel::LoopId| -> Vec<[u64; 3]> {
        let mut r: Vec<[u64; 3]> = obj.loop_half_edges(l).map(|h| pos(he[h].origin)).collect();
        if let Some(i) = r
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| **p)
            .map(|(i, _)| i)
        {
            r.rotate_left(i);
        }
        r
    };
    let mut faces: Vec<u64> = obj
        .faces()
        .iter()
        .map(|(_, f)| {
            let mut h = Fnv::new();
            for p in ring(f.outer_loop) {
                p.iter().for_each(|&c| h.u64(c));
            }
            let mut holes: Vec<Vec<[u64; 3]>> = f.inner_loops.iter().map(|&l| ring(l)).collect();
            holes.sort();
            for hole in holes {
                h.u64(u64::MAX);
                hole.iter().flatten().for_each(|&c| h.u64(c));
            }
            h.u64(material_hash(doc, f.material));
            h.u64(material_hash(doc, f.back_material));
            if let Some(uv) = f.uv_frame {
                for c in [uv.s.x, uv.s.y, uv.s.z, uv.t.x, uv.t.y, uv.t.z, uv.u0, uv.v0] {
                    h.f(c);
                }
            }
            if let Some(kernel::SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            }) = f.surface
            {
                for c in [
                    axis_point.x,
                    axis_point.y,
                    axis_point.z,
                    axis.x,
                    axis.y,
                    axis.z,
                    radius,
                ] {
                    h.f(c);
                }
            }
            h.0
        })
        .collect();
    faces.sort_unstable();
    let mut edges: Vec<u64> = obj
        .edges()
        .iter()
        .map(|(_, e)| {
            let mut h = Fnv::new();
            let mut ends = [
                pos(he[e.half_edge].origin),
                pos(he[he[e.half_edge].next].origin),
            ];
            ends.sort();
            ends.iter().flatten().for_each(|&c| h.u64(c));
            h.u64(u64::from(e.soft));
            if let Some(g) = e.curve {
                for c in [g.center.x, g.center.y, g.center.z, g.radius] {
                    h.f(c);
                }
            }
            h.0
        })
        .collect();
    edges.sort_unstable();
    let mut h = Fnv::new();
    faces.iter().for_each(|&f| h.u64(f));
    h.u64(u64::MAX);
    edges.iter().for_each(|&e| h.u64(e));
    h.u64(material_hash(doc, obj.default_material()));
    h.0
}

fn attrs_hash(doc: &Document, e: EntityRef, h: &mut Fnv) {
    h.str(&format!(
        "{:?}",
        doc.attr_get(&AttrTarget::Entity(e)).ok().flatten()
    ));
}

/// A whole node: an Object's geometry, a group's members, an instance's
/// definition and pose — with the node's name, tags, hidden state and
/// attributes, and its members' all the way down.
pub fn node_hash(doc: &Document, node: NodeId) -> u64 {
    let mut h = Fnv::new();
    for path in doc.node_tags(node) {
        h.str(&path.join("\u{1f}"));
    }
    h.u64(u64::from(doc.node_user_hidden(node)));
    attrs_hash(doc, entity(node), &mut h);
    match node {
        NodeId::Object(id) => {
            h.str(doc.object_name(id).unwrap_or(""));
            h.u64(doc.object(id).map_or(0, |o| geometry_hash(doc, o)));
        }
        NodeId::Group(id) => {
            h.str(doc.group_name(id).unwrap_or(""));
            let mut members: Vec<u64> = doc
                .group_members(id)
                .unwrap_or_default()
                .into_iter()
                .map(|m| node_hash(doc, m))
                .collect();
            members.sort_unstable();
            members.iter().for_each(|&m| h.u64(m));
            // The group's own drawing.
            let mut sketches: Vec<u64> = doc
                .group_member_sketches(id)
                .into_iter()
                .filter_map(|s| sketch_hash(doc, s))
                .collect();
            sketches.sort_unstable();
            h.u64(u64::MAX);
            sketches.iter().for_each(|&s| h.u64(s));
        }
        NodeId::Instance(id) => {
            h.str(doc.instance_name(id).unwrap_or(""));
            if let Some(d) = doc.instance_def(id) {
                h.u64(def_hash(doc, d));
            }
            if let Some(p) = doc.instance_pose(id) {
                p.to_affine().iter().for_each(|&c| h.f(c));
            }
            // The placement's own paint: repainting a placement (its
            // member faces take it where they have none of their own) is a
            // change to the node.
            h.u64(u64::MAX - 1);
            h.u64(material_hash(doc, doc.instance_material(id).flatten()));
        }
    }
    h.0
}

/// A definition: its name, attributes, members (each a whole node) and
/// drawn sketches.
/// An annotation's content and tags: any edit reads as a change. 0 for a
/// stale handle.
pub fn annotation_hash(doc: &Document, id: AnnotationId) -> u64 {
    let Some(a) = doc.annotation(id) else {
        return 0;
    };
    let mut h = Fnv::new();
    // The derived Debug form prints every f64 exactly (shortest round-trip).
    h.str(&format!("{a:?}"));
    for t in doc.annotation_tags(id) {
        h.u64(t.len() as u64);
        for seg in t {
            h.str(seg);
        }
    }
    h.0
}

pub fn def_hash(doc: &Document, def: ComponentId) -> u64 {
    let mut h = Fnv::new();
    h.str(doc.component_name(def).unwrap_or(""));
    attrs_hash(doc, EntityRef::Component(def), &mut h);
    let mut members: Vec<u64> = doc
        .def_member_nodes(def)
        .unwrap_or_default()
        .into_iter()
        .map(|m| node_hash(doc, m))
        .collect();
    members.sort_unstable();
    members.iter().for_each(|&m| h.u64(m));
    let mut sketches: Vec<u64> = doc
        .def_member_sketches(def)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| sketch_hash(doc, s))
        .collect();
    sketches.sort_unstable();
    sketches.iter().for_each(|&s| h.u64(s));
    h.0
}

/// A sketch's drawing: its segments, as unordered pairs of positions, the
/// loops that have a face (a loop given its face by drawing over an edge
/// writes a face where the file had edges alone, so it reads as a change)
/// and its tags (a tag rename or delete changes the layer its edges go on,
/// so it reads as a change). `None` for a stale handle.
pub fn sketch_hash(doc: &Document, id: SketchId) -> Option<u64> {
    let sk = doc.sketch(id)?;
    let bits = |p: Point3| [p.x, p.y, p.z].map(|c| (c + 0.0).to_bits());
    let mut segs: Vec<[u64; 6]> = sk
        .edges()
        .iter()
        .map(|(_, e)| {
            let a = sk.vertices()[e.from].position;
            let b = sk.vertices()[e.to].position;
            let mut s = [bits(a), bits(b)];
            s.sort();
            [s[0][0], s[0][1], s[0][2], s[1][0], s[1][1], s[1][2]]
        })
        .collect();
    segs.sort_unstable();
    let mut sh = Fnv::new();
    segs.iter().flatten().for_each(|&c| sh.u64(c));
    // A faced loop as the sorted set of its ring's points: the same loop
    // whichever vertex the ring starts at.
    let mut faced: Vec<Vec<[u64; 3]>> = sk
        .regions()
        .values()
        .filter(|r| !r.faceless)
        .map(|r| {
            let mut ring: Vec<[u64; 3]> = r
                .outer
                .iter()
                .map(|&v| bits(sk.vertices()[v].position))
                .collect();
            ring.sort_unstable();
            ring
        })
        .collect();
    faced.sort_unstable();
    for ring in &faced {
        sh.u64(ring.len() as u64);
        ring.iter().flatten().for_each(|&c| sh.u64(c));
    }
    for t in doc.sketch_tags(id) {
        sh.str(&t.join("\u{1f}"));
    }
    Some(sh.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repaint of a placement changes its hash (and so the hash of a
    /// group holding it); a repaint of a definition changes the members'
    /// objects, which were already hashed.
    #[test]
    fn repainting_a_placement_changes_its_hash() {
        use kernel::{Material, Plane, Point3, Rgba8};
        let mut doc = Document::new();
        let ground = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let s = doc.add_sketch(ground);
        let c = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        for i in 0..4 {
            doc.sketch_mut(s)
                .unwrap()
                .add_segment(c[i], c[(i + 1) % 4])
                .unwrap();
        }
        let region = doc.extrudable_regions(s).unwrap()[0];
        doc.extrude_region(s, region, 1.0).unwrap();
        let object = *doc.live_object_ids().last().unwrap();
        let (_, instance, _) = doc.make_component(&[NodeId::Object(object)]).unwrap();
        let red = doc.add_material(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
        let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(0, 0, 200)));
        let node = NodeId::Instance(instance);

        let plain = node_hash(&doc, node);
        doc.set_instance_material(instance, Some(red)).unwrap();
        let painted_red = node_hash(&doc, node);
        assert_ne!(plain, painted_red, "painting a placement is a change");
        doc.set_instance_material(instance, Some(blue)).unwrap();
        assert_ne!(painted_red, node_hash(&doc, node), "so is repainting it");
        doc.set_instance_material(instance, None).unwrap();
        assert_eq!(
            plain,
            node_hash(&doc, node),
            "clearing it restores the hash"
        );
    }

    /// Two palette materials with one name are different materials: a
    /// repaint from one to the other is a change.
    #[test]
    fn same_named_materials_hash_apart() {
        use kernel::{Material, Rgba8};
        let mut doc = Document::new();
        let a = doc.add_material(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
        let b = doc.add_material(Material::solid("Red", Rgba8::rgb(120, 0, 0)));
        assert_ne!(material_hash(&doc, Some(a)), material_hash(&doc, Some(b)));
        assert_ne!(material_hash(&doc, Some(a)), material_hash(&doc, None));
    }

    #[test]
    fn a_blob_reads_back() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            7,
            NodeSource {
                origin: Origin::GroupGeometry { pid: 42 },
                geometry: u64::MAX - 3,
                node: 99,
            },
        );
        nodes.insert(
            8,
            NodeSource {
                origin: Origin::Loose,
                geometry: 1,
                node: 2,
            },
        );
        let mut defs = BTreeMap::new();
        defs.insert(
            9,
            DefSource {
                name: "Chair".into(),
                slot: 3,
                hash: u64::MAX,
            },
        );
        let mut annotations = BTreeMap::new();
        annotations.insert(
            u64::MAX - 7,
            AnnotationSource {
                origin: AnnotationOrigin::Dimension {
                    index: 41,
                    pid: 150_283,
                },
                hash: u64::MAX,
            },
        );
        let s = SkpSource {
            bytes: vec![1, 2, 3, 0, 255],
            nodes,
            defs,
            loose: 1,
            annotations,
            unread: Some([4807, 52_979].into()),
            layers: [(7u64, "Walls".to_string())].into(),
            loose_sketches: [(9u64, 77u64)].into(),
        };
        let back = SkpSource::from_blob(&s.to_blob()).expect("reads");
        assert_eq!(back.bytes, s.bytes);
        assert_eq!(back.nodes, s.nodes);
        assert_eq!(back.defs, s.defs);
        assert_eq!(back.layers, s.layers);
        assert_eq!(back.loose_sketches, s.loose_sketches);
        assert_eq!(back.annotations, s.annotations);
        assert_eq!(back.unread, s.unread);
        assert_eq!(back.loose, 1);
        assert!(SkpSource::from_blob(&[1, 2, 3]).is_none());
    }
}
