//! Library updates (OW-2): the content print, the state of a definition's
//! link to its Library item, and Update from Library.
//!
//! **The content print** is a fingerprint of what a definition *is*: each
//! live member in order, by kind; an Object's faces (loops of vertices
//! rounded to 1 µm, read counter-clockwise from the smallest vertex, holes
//! sorted), its surface claims, its True Curves marks (blend tags and what
//! their records ask for, creased and soft edges, circle and polygon
//! claims, the Smooth level, and the paint of the faces each blend
//! consumed), and each face's paint by value (color,
//! texture image bytes and tile size, physically based factors and maps),
//! never by palette slot; a member group by its own live members; a member
//! placement by its pose inside the definition, its own paint by value,
//! and the print of the definition it places (so a nested part's change
//! changes every print above it); and the definition's own drawing (every
//! live sketch it owns, by its edges). Definitions hold no guides. It is
//! blind to names, tags, attributes, ids of every kind, hidden flags, and
//! where the definition is placed.
//!
//! The kernel writes the canonical byte stream
//! ([`Document::content_print_stream`]) and never hashes it: the caller
//! hands in the digest ([`PrintDigest`], the API layer's SHA-256), and a
//! print is stored as text, `p1:<hex>` ([`CONTENT_PRINT_SCHEME`]). Prints
//! with another scheme never compare.
//!
//! **The link state** ([`library_link_state`]) compares three prints per
//! link: `base` (stamped on the definition when it was last in step with
//! the Library: at insert, at Save to Library, at each update), `mine`
//! (computed now) and `theirs` (what the Library item holds now, which the
//! caller reads from the item's `prints` metadata, [`LibraryPrints`]).
//!
//! **Update from Library** ([`Document::update_definitions_from_items`])
//! replaces a definition's contents with a copy of the item's, through the
//! same copy engine a Library insert uses, keeping the definition's
//! identity so every placement follows where it stands.

use super::*;

/// The scheme of every content print this kernel writes: a print is stored
/// as `p1:<lowercase hex digest>`. A print under another scheme does not
/// compare with one under this one ([`library_link_state`]).
pub const CONTENT_PRINT_SCHEME: &str = "p1";

/// The digest a content print is made with, supplied by the caller (the
/// API layer's SHA-256): the kernel writes the canonical stream and never
/// hashes, so it gains no dependency. A nested definition contributes its
/// own digest to the stream of the definition placing it.
pub type PrintDigest = fn(&[u8]) -> [u8; 32];

/// A digest as print text: `p1:` and the digest in lowercase hex.
pub fn content_print_text(digest: &[u8]) -> String {
    let mut out = String::with_capacity(CONTENT_PRINT_SCHEME.len() + 1 + digest.len() * 2);
    out.push_str(CONTENT_PRINT_SCHEME);
    out.push(':');
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The scheme part of a stored print (`p1` of `p1:ab…`), `None` for text
/// that is not a print.
fn print_scheme(print: &str) -> Option<&str> {
    print
        .split_once(':')
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty())
}

/// Whether `print` is a print under this kernel's scheme.
pub fn is_current_print(print: &str) -> bool {
    print_scheme(print) == Some(CONTENT_PRINT_SCHEME)
}

/// Where a definition linked to a Library item stands against it (D2's
/// table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LibraryLinkState {
    /// The definition holds what the item holds (whatever the base says).
    InStep,
    /// Unchanged here since it was last in step; the item has changed.
    LibraryNewer,
    /// Changed here since it was last in step; the item has not.
    EditedHere,
    /// Changed here and in the Library, and the two differ.
    BothChanged,
    /// The two differ and nothing says which side moved: the link has no
    /// base (made before content prints existed), or a print is under
    /// another scheme.
    Different,
    /// The Library no longer holds the item, or the item no longer holds
    /// this definition. The part keeps working; nothing is offered.
    ItemMissing,
}

impl LibraryLinkState {
    /// The state's wire name (`in_step`, `library_newer`, `edited_here`,
    /// `both_changed`, `different`, `item_missing`).
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryLinkState::InStep => "in_step",
            LibraryLinkState::LibraryNewer => "library_newer",
            LibraryLinkState::EditedHere => "edited_here",
            LibraryLinkState::BothChanged => "both_changed",
            LibraryLinkState::Different => "different",
            LibraryLinkState::ItemMissing => "item_missing",
        }
    }
}

/// The state of one link from its three prints (D2's table): `mine`
/// (computed now), `base` (stamped when last in step, if any) and `theirs`
/// (what the item holds now; `None` when the Library no longer holds the
/// item or the item no longer holds the definition). Pure.
///
/// - `theirs` absent: [`LibraryLinkState::ItemMissing`].
/// - `mine` and `theirs` under different schemes: they cannot compare,
///   [`LibraryLinkState::Different`].
/// - `mine == theirs`: [`LibraryLinkState::InStep`], whatever `base` says.
/// - no `base`, or a `base` under another scheme:
///   [`LibraryLinkState::Different`] (no claim about direction).
/// - otherwise by which side moved off `base`.
pub fn library_link_state(
    mine: &str,
    base: Option<&str>,
    theirs: Option<&str>,
) -> LibraryLinkState {
    let Some(theirs) = theirs else {
        return LibraryLinkState::ItemMissing;
    };
    if print_scheme(mine).is_none() || print_scheme(mine) != print_scheme(theirs) {
        return LibraryLinkState::Different;
    }
    if mine == theirs {
        return LibraryLinkState::InStep;
    }
    match base.filter(|b| print_scheme(b) == print_scheme(mine)) {
        None => LibraryLinkState::Different,
        Some(b) => match (mine == b, theirs == b) {
            (true, _) => LibraryLinkState::LibraryNewer,
            (false, true) => LibraryLinkState::EditedHere,
            (false, false) => LibraryLinkState::BothChanged,
        },
    }
}

/// What the Library says, for [`Document::library_links`]: an item's
/// current content prints by the stable id of each definition in the item
/// file, or `None` when the Library no longer holds the item. The kernel
/// does no I/O: the caller reads the prints from each item's `hew.library`
/// `prints` metadata, or computes them from the item file
/// ([`Document::content_prints_by_sid`]) for an item saved before prints
/// existed.
pub trait LibraryPrints {
    /// The prints of item `source_id`, or `None` when it is gone.
    fn item_prints(&self, source_id: &str) -> Option<&BTreeMap<u64, String>>;
}

impl LibraryPrints for BTreeMap<String, BTreeMap<u64, String>> {
    fn item_prints(&self, source_id: &str) -> Option<&BTreeMap<u64, String>> {
        self.get(source_id)
    }
}

/// A definition's `hew.library` stamp, as [`Document::library_stamp`]
/// reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStamp {
    /// The item's library id.
    pub source_id: String,
    /// The fingerprint of the item file the stamp was made from. Written
    /// so older readers keep working; nothing compares it.
    pub content_hash: Option<String>,
    /// The definition's stable id in the item file.
    pub def_sid: Option<u64>,
    /// The base print: what the definition was when last in step.
    pub print: Option<String>,
}

/// One linked definition and where it stands
/// ([`Document::library_links`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryLink {
    pub definition: ComponentId,
    /// The item's library id.
    pub source_id: String,
    /// The definition's stable id in the item file.
    pub def_sid: Option<u64>,
    /// The stamped base print, if the link has one.
    pub base: Option<String>,
    /// The definition's print now.
    pub mine: String,
    /// The item's print of the definition now, if the Library holds it.
    pub theirs: Option<String>,
    pub state: LibraryLinkState,
    /// How many live placements the definition has
    /// ([`Document::definition_usage`]).
    pub copies: usize,
    /// Whether a lock makes Update from Library skip the definition: the
    /// very check the update makes ([`Document::library_update_lock`]).
    pub frozen: bool,
}

/// One definition to update and the item to update it from
/// ([`Document::update_definitions_from_items`]).
#[derive(Debug, Clone)]
pub struct LibraryUpdate<'a> {
    /// A live definition carrying a `hew.library` stamp.
    pub definition: ComponentId,
    /// The Library item, loaded.
    pub item: &'a Document,
    /// The item's provenance: `source_id` must be the definition's stamped
    /// item id, `content_hash` fingerprints the item file now, and `prints`
    /// holds the item's prints by definition sid (stamped as the new base).
    pub provenance: LibraryProvenance,
}

/// What [`Document::update_definitions_from_items`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryUpdateReport {
    /// Requested definitions whose contents were replaced, in request
    /// order.
    pub updated: Vec<ComponentId>,
    /// Other definitions replaced in the same step: nested parts of an
    /// updated item that the model already held from the same item.
    pub also_updated: Vec<ComponentId>,
    /// Requested definitions that already hold the item's content: nothing
    /// changed for them.
    pub unchanged: Vec<ComponentId>,
    /// Requested definitions that were refused and left as they were, with
    /// why. The rest of the batch went through.
    pub skipped: Vec<(ComponentId, DocumentError)>,
    /// Definitions from the same item and definition as one the update's
    /// item nests that were left exactly as they were, because they were
    /// changed here (or the two differ with nothing to say which side
    /// moved) or a lock holds them: the updated part got a fresh copy of
    /// the item's definition instead.
    pub kept: Vec<ComponentId>,
    /// Definitions the item carried that the model did not hold, copied
    /// in.
    pub definitions_added: usize,
    /// Palette materials added (an item's paint the palette had no equal
    /// of). Like every palette addition, undo leaves them in the palette.
    pub materials_added: usize,
}

/// One definition whose contents an update replaced, for undo and redo
/// ([`DocAction::UpdatedFromLibrary`]).
#[derive(Debug, Clone)]
pub(super) struct DefinitionSwap {
    pub(super) def: ComponentId,
    /// The member list before the update (hidden rows included, exactly).
    pub(super) prior_members: Vec<NodeId>,
    /// The member list after: the new contents' roots.
    pub(super) next_members: Vec<NodeId>,
    /// The live member subtree the update hid, in pre-order.
    pub(super) retired: Vec<NodeId>,
    /// The definition's own live sketches (not an Object's or a group's)
    /// the update tombstoned.
    pub(super) retired_sketches: Vec<SketchId>,
    /// The `hew.library` namespace before and after the re-stamp.
    pub(super) prior_stamp: Option<BTreeMap<String, crate::attr::AttrValue>>,
    pub(super) next_stamp: Option<BTreeMap<String, crate::attr::AttrValue>>,
}

/// The quantum vertices and lengths are rounded to: one micrometer.
const QUANTUM: f64 = 1e-6;
/// The finer quantum for unitless numbers: directions, a placement's
/// linear part, texture gradients, angles.
const FINE: f64 = 1e-9;

fn q(x: f64) -> i64 {
    (x / QUANTUM).round() as i64
}

fn qf(x: f64) -> i64 {
    (x / FINE).round() as i64
}

fn qp(p: Point3) -> [i64; 3] {
    [q(p.x), q(p.y), q(p.z)]
}

/// A direction, normalized then rounded finely. `signed: false` (a line's
/// direction) makes the first nonzero component positive.
fn qdir(v: Vec3, signed: bool) -> [i64; 3] {
    let v = v.normalized().unwrap_or(v);
    let mut a = [qf(v.x), qf(v.y), qf(v.z)];
    if !signed && a.iter().find(|&&c| c != 0).is_some_and(|&c| c < 0) {
        a = a.map(|c| -c);
    }
    a
}

/// The ring rotated to start at its smallest vertex.
fn canonical_start(mut v: Vec<[i64; 3]>) -> Vec<[i64; 3]> {
    if let Some((k, _)) = v.iter().enumerate().min_by_key(|(_, p)| **p) {
        v.rotate_left(k);
    }
    v
}

/// The canonical byte stream under construction: little-endian integers,
/// every variable-length run prefixed with its length, so the encoding is
/// injective.
#[derive(Default)]
struct Stream(Vec<u8>);

impl Stream {
    fn tag(&mut self, b: u8) {
        self.0.push(b);
    }
    fn n(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn ints(&mut self, vs: &[i64]) {
        self.n(vs.len() as u64);
        for &v in vs {
            self.i(v);
        }
    }
    fn blob(&mut self, b: &[u8]) {
        self.n(b.len() as u64);
        self.0.extend_from_slice(b);
    }
}

fn image_format_code(f: crate::material::ImageFormat) -> u8 {
    match f {
        crate::material::ImageFormat::Png => 0,
        crate::material::ImageFormat::Jpeg => 1,
    }
}

/// A material by value: color, texture (format, tile size, image bytes)
/// and physically based properties (a plain set reads as none). Its name
/// is left out: two palettes with the same paint agree.
fn material_bytes(m: &Material) -> Vec<u8> {
    let mut s = Stream::default();
    s.0.extend_from_slice(&[m.color.r, m.color.g, m.color.b, m.color.a]);
    match &m.texture {
        None => s.tag(0),
        Some(t) => {
            s.tag(1);
            s.tag(image_format_code(t.format));
            s.i(qf(t.world_size[0]));
            s.i(qf(t.world_size[1]));
            s.blob(&t.image);
        }
    }
    match m.pbr.as_ref().filter(|p| !p.is_plain()) {
        None => s.tag(0),
        Some(p) => {
            s.tag(1);
            for v in [
                p.metallic,
                p.roughness,
                p.normal_scale,
                p.occlusion_strength,
            ] {
                s.n(u64::from(v.to_bits()));
            }
            for map in p.maps() {
                match map {
                    None => s.tag(0),
                    Some(m) => {
                        s.tag(1);
                        s.tag(image_format_code(m.format));
                        s.blob(&m.image);
                    }
                }
            }
        }
    }
    s.0
}

/// A memo of definitions' digests, for one print walk.
type DigestMemo = BTreeMap<ComponentId, [u8; 32]>;

/// A memo of definition pairs already compared ([`Document::content_equal`]).
type EqualMemo = BTreeMap<(ComponentId, ComponentId), bool>;

impl Document {
    // ------------------------------------------------------- content print

    /// The canonical byte stream of definition `def`'s content (this
    /// module's doc says what it covers). `nested` supplies the bytes a
    /// member placement contributes for the definition it places: its
    /// digest, for a print ([`Document::content_print`]). Read-only.
    ///
    /// # Errors
    /// [`DocumentError::UnknownComponent`] for a stale or hidden
    /// definition.
    pub fn content_print_stream(
        &self,
        def: ComponentId,
        nested: &mut dyn FnMut(ComponentId) -> Vec<u8>,
    ) -> Result<Vec<u8>, DocumentError> {
        let c = self
            .components
            .get(def)
            .filter(|c| !c.hidden)
            .ok_or(DocumentError::UnknownComponent)?;
        let members: Vec<NodeId> = c
            .members
            .iter()
            .copied()
            .filter(|&m| self.print_live(m))
            .collect();

        // The paint table: every material the definition's own content
        // references, by value, sorted, so a face names its paint by its
        // place in the table and never by palette slot.
        let mut used: BTreeSet<MaterialId> = BTreeSet::new();
        for &m in &members {
            self.print_collect_materials(m, &mut used);
        }
        let mut by_value: Vec<(Vec<u8>, MaterialId)> = used
            .into_iter()
            .map(|m| {
                let bytes = match self.materials.get(m) {
                    Some(mat) => material_bytes(mat),
                    None => vec![0xff],
                };
                (bytes, m)
            })
            .collect();
        by_value.sort();
        let mut table: Vec<&[u8]> = Vec::new();
        let mut index: BTreeMap<MaterialId, u64> = BTreeMap::new();
        for (bytes, m) in &by_value {
            if table.last() != Some(&bytes.as_slice()) {
                table.push(bytes.as_slice());
            }
            index.insert(*m, table.len() as u64);
        }

        let mut s = Stream::default();
        s.blob(b"hew.content-print.1");
        s.n(table.len() as u64);
        for bytes in &table {
            s.blob(bytes);
        }
        s.n(members.len() as u64);
        for &m in &members {
            self.print_member(m, &index, nested, &mut s);
        }

        // The definition's own drawing: every live sketch it owns (an
        // Object's face sketches and a group's lines included), each by its
        // edges, sorted; a sketch with no edge draws nothing.
        let mut sketches: Vec<Vec<i64>> = self
            .def_sketches
            .iter()
            .filter(|&(sid, &owner)| owner == def && !self.sketch_tombstoned(sid))
            .filter_map(|(&sid, _)| self.sketches.get(sid))
            .map(sketch_record)
            .filter(|r| !r.is_empty())
            .collect();
        sketches.sort();
        s.n(sketches.len() as u64);
        for r in &sketches {
            s.ints(r);
        }
        Ok(s.0)
    }

    /// Definition `def`'s content print, `p1:<hex>`, made with `digest`.
    ///
    /// # Errors
    /// [`DocumentError::UnknownComponent`] for a stale or hidden
    /// definition.
    pub fn content_print(
        &self,
        def: ComponentId,
        digest: PrintDigest,
    ) -> Result<String, DocumentError> {
        let d = self.content_digest(def, digest, &mut DigestMemo::new(), &mut BTreeSet::new())?;
        Ok(content_print_text(&d))
    }

    /// Every live definition's content print, keyed by definition. One
    /// walk: a definition placed in many others is printed once.
    pub fn content_prints(&self, digest: PrintDigest) -> BTreeMap<ComponentId, String> {
        let mut memo = DigestMemo::new();
        let mut out = BTreeMap::new();
        for (cid, c) in self.components.iter() {
            if c.hidden {
                continue;
            }
            if let Ok(d) = self.content_digest(cid, digest, &mut memo, &mut BTreeSet::new()) {
                out.insert(cid, content_print_text(&d));
            }
        }
        out
    }

    /// Every live definition's content print keyed by its stable id: the
    /// `prints` map a Library item carries in its `hew.library` metadata
    /// (each save to the Library writes it), and what a caller computes
    /// from an item file saved before prints existed.
    pub fn content_prints_by_sid(&self, digest: PrintDigest) -> BTreeMap<u64, String> {
        self.content_prints(digest)
            .into_iter()
            .filter_map(|(cid, p)| {
                self.sid_of(&EntityRef::Component(cid))
                    .filter(|&s| s != WRAPPED_DEF_SID)
                    .map(|s| (s, p))
            })
            .collect()
    }

    /// [`Document::content_print`]'s digest, memoized over one walk.
    fn content_digest(
        &self,
        def: ComponentId,
        digest: PrintDigest,
        memo: &mut DigestMemo,
        on_path: &mut BTreeSet<ComponentId>,
    ) -> Result<[u8; 32], DocumentError> {
        if let Some(d) = memo.get(&def) {
            return Ok(*d);
        }
        // A validated document's definitions form a DAG; a cycle met
        // mid-diagnosis refuses rather than recursing forever.
        if !on_path.insert(def) {
            return Err(DocumentError::ComponentCycle);
        }
        let mut failed = None;
        let stream = self.content_print_stream(def, &mut |child| match self
            .content_digest(child, digest, memo, on_path)
        {
            Ok(d) => d.to_vec(),
            Err(e) => {
                failed.get_or_insert(e);
                Vec::new()
            }
        });
        on_path.remove(&def);
        let stream = stream?;
        if let Some(e) = failed {
            return Err(e);
        }
        let d = digest(&stream);
        memo.insert(def, d);
        Ok(d)
    }

    /// Whether definition `def` here holds exactly what definition
    /// `other_def` of `other` holds, by content print, compared without
    /// hashing: the two streams must be equal and each nested definition
    /// pair must compare equal in turn. `other` may be this document.
    pub(crate) fn content_equal(
        &self,
        def: ComponentId,
        other: &Document,
        other_def: ComponentId,
    ) -> bool {
        self.content_equal_memo(def, other, other_def, &mut EqualMemo::new(), 0)
    }

    fn content_equal_memo(
        &self,
        a: ComponentId,
        other: &Document,
        b: ComponentId,
        memo: &mut EqualMemo,
        depth: usize,
    ) -> bool {
        if let Some(&r) = memo.get(&(a, b)) {
            return r;
        }
        if depth > MAX_COMPONENT_DEPTH + 1 {
            return false;
        }
        // Provisional while the pair is being compared: a malformed cycle
        // answers "different" rather than looping.
        memo.insert((a, b), false);
        let mut na: Vec<ComponentId> = Vec::new();
        let mut nb: Vec<ComponentId> = Vec::new();
        let sa = self.content_print_stream(a, &mut |c| {
            na.push(c);
            Vec::new()
        });
        let sb = other.content_print_stream(b, &mut |c| {
            nb.push(c);
            Vec::new()
        });
        let equal = match (sa, sb) {
            (Ok(x), Ok(y)) => {
                x == y
                    && na.len() == nb.len()
                    && na
                        .iter()
                        .zip(&nb)
                        .all(|(&x, &y)| self.content_equal_memo(x, other, y, memo, depth + 1))
            }
            _ => false,
        };
        memo.insert((a, b), equal);
        equal
    }

    /// Whether a member row is content: not a tombstone.
    fn print_live(&self, node: NodeId) -> bool {
        match node {
            NodeId::Object(o) => self.objects.get(o).is_some_and(|r| !r.hidden),
            NodeId::Group(g) => self.groups.get(g).is_some_and(|r| !r.hidden),
            NodeId::Instance(i) => self.instances.get(i).is_some_and(|r| !r.hidden),
        }
    }

    /// The materials `node`'s own content references (a group's members
    /// included; a placement's own paint, not its definition's).
    fn print_collect_materials(&self, node: NodeId, out: &mut BTreeSet<MaterialId>) {
        match node {
            NodeId::Object(o) => {
                let obj = &self.objects[o].object;
                for f in obj.faces().values() {
                    out.extend(f.material);
                    out.extend(f.back_material);
                }
                // The paint of the faces a blend consumed: taking the
                // blend off brings them back with it.
                for rec in obj.blend_records().values() {
                    for face in &rec.consumed {
                        out.extend(face.attrs.material);
                        out.extend(face.attrs.back_material);
                    }
                }
                out.extend(obj.default_material());
            }
            NodeId::Group(g) => {
                for &m in &self.groups[g].members {
                    if self.print_live(m) {
                        self.print_collect_materials(m, out);
                    }
                }
            }
            NodeId::Instance(i) => out.extend(self.instances[i].material),
        }
    }

    /// One member's record: its kind, then an Object's record, a group's
    /// live members in order, or a placement's pose, paint and nested
    /// print.
    fn print_member(
        &self,
        node: NodeId,
        index: &BTreeMap<MaterialId, u64>,
        nested: &mut dyn FnMut(ComponentId) -> Vec<u8>,
        s: &mut Stream,
    ) {
        match node {
            NodeId::Object(o) => {
                s.tag(1);
                let rec = self.object_print_record(o, index);
                s.blob(&rec);
            }
            NodeId::Group(g) => {
                s.tag(2);
                let live: Vec<NodeId> = self.groups[g]
                    .members
                    .iter()
                    .copied()
                    .filter(|&m| self.print_live(m))
                    .collect();
                s.n(live.len() as u64);
                for m in live {
                    self.print_member(m, index, nested, s);
                }
            }
            NodeId::Instance(i) => {
                s.tag(3);
                let rec = &self.instances[i];
                let a = rec.pose.to_affine();
                let pose: Vec<i64> = a
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| if k % 4 == 3 { q(v) } else { qf(v) })
                    .collect();
                s.ints(&pose);
                s.n(rec
                    .material
                    .and_then(|m| index.get(&m).copied())
                    .unwrap_or(0));
                let inner = nested(rec.def);
                s.blob(&inner);
            }
        }
    }

    /// An Object's record in its definition's frame: its faces (sorted),
    /// its edge marks (each sorted), its Smooth level and its base paint.
    fn object_print_record(&self, oid: ObjectId, index: &BTreeMap<MaterialId, u64>) -> Vec<u8> {
        let obj = &self.objects[oid].object;
        let paint = |m: Option<MaterialId>| -> i64 {
            m.and_then(|m| index.get(&m).copied())
                .map_or(0, |i| i as i64)
        };
        let mut faces: Vec<Vec<i64>> = Vec::with_capacity(obj.faces().len());
        for f in obj.faces().values() {
            let mut rec: Vec<i64> = Vec::new();
            match f.surface {
                None => rec.push(0),
                Some(crate::topo::SurfaceRef::Cylinder {
                    axis_point,
                    axis,
                    radius,
                }) => {
                    rec.push(1);
                    // The axis line by its direction and the point of it
                    // nearest the origin (any point names the same line).
                    let dir = axis.normalized().unwrap_or(axis);
                    let o = Point3::ORIGIN;
                    let foot = axis_point + dir * -((axis_point - o).dot(dir));
                    rec.extend(qdir(axis, false));
                    rec.extend(qp(foot));
                    rec.push(q(radius));
                }
                Some(crate::topo::SurfaceRef::Sphere { center, radius }) => {
                    rec.push(2);
                    rec.extend(qp(center));
                    rec.push(q(radius));
                }
                Some(crate::topo::SurfaceRef::Torus {
                    center,
                    axis,
                    major_radius,
                    minor_radius,
                }) => {
                    rec.push(3);
                    rec.extend(qp(center));
                    rec.extend(qdir(axis, false));
                    rec.push(q(major_radius));
                    rec.push(q(minor_radius));
                }
                Some(crate::topo::SurfaceRef::Cone {
                    apex,
                    axis,
                    half_angle,
                }) => {
                    rec.push(4);
                    rec.extend(qp(apex));
                    rec.extend(qdir(axis, true));
                    rec.push(qf(half_angle));
                }
            }
            let mut loops: Vec<Vec<[i64; 3]>> = std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .map(|l| canonical_start(obj.loop_positions(l).map(qp).collect()))
                .collect();
            if loops.len() > 1 {
                loops[1..].sort();
            }
            rec.push(loops.len() as i64);
            for l in &loops {
                rec.push(l.len() as i64);
                for v in l {
                    rec.extend(v);
                }
            }
            rec.push(paint(f.material));
            rec.push(paint(f.back_material));
            match f.uv_frame {
                None => rec.push(0),
                Some(uv) => {
                    rec.push(1);
                    rec.extend([qf(uv.s.x), qf(uv.s.y), qf(uv.s.z)]);
                    rec.extend([qf(uv.t.x), qf(uv.t.y), qf(uv.t.z)]);
                    rec.push(qf(uv.u0));
                    rec.push(qf(uv.v0));
                }
            }
            match f.blend {
                None => rec.push(0),
                Some(t) => {
                    let kind = match t.kind {
                        crate::topo::BlendKind::Fillet => 0,
                        crate::topo::BlendKind::Chamfer => 1,
                        crate::topo::BlendKind::Profile => 2,
                    };
                    let role = match t.role {
                        crate::topo::BlendRole::Strip => 0,
                        crate::topo::BlendRole::Corner => 1,
                        crate::topo::BlendRole::Runout => 2,
                    };
                    let digest = obj.blend_record_digest(t.id).unwrap_or_default();
                    rec.extend([1, kind, role, digest.len() as i64]);
                    rec.extend(digest.bytes().map(i64::from));
                    // The paint (front, back) of each face the blend
                    // consumed, sorted: a repaint there changes what
                    // taking the blend off gives back.
                    let mut consumed: Vec<[i64; 2]> = obj
                        .blend_records()
                        .get(&t.id)
                        .map(|r| {
                            r.consumed
                                .iter()
                                .map(|c| [paint(c.attrs.material), paint(c.attrs.back_material)])
                                .collect()
                        })
                        .unwrap_or_default();
                    consumed.sort_unstable();
                    rec.push(consumed.len() as i64);
                    for pair in consumed {
                        rec.extend(pair);
                    }
                }
            }
            faces.push(rec);
        }
        faces.sort();

        // Edge marks: each a sorted pair of rounded ends (a claim adds its
        // center and radius), each list sorted.
        let mut creases: Vec<[i64; 6]> = Vec::new();
        let mut soft: Vec<[i64; 6]> = Vec::new();
        let mut circles: Vec<[i64; 10]> = Vec::new();
        let mut polygons: Vec<[i64; 10]> = Vec::new();
        for (e, edge) in obj.edges().iter() {
            let Some((a, b)) = obj.edge_endpoints(e) else {
                continue;
            };
            let (qa, qb) = (qp(a), qp(b));
            let [lo, hi] = if qa <= qb { [qa, qb] } else { [qb, qa] };
            let pair = [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]];
            if edge.crease {
                creases.push(pair);
            }
            if edge.soft {
                soft.push(pair);
            }
            for (claim, list) in [(edge.curve, &mut circles), (edge.polygon, &mut polygons)] {
                if let Some(g) = claim {
                    let c = qp(g.center);
                    list.push([
                        pair[0],
                        pair[1],
                        pair[2],
                        pair[3],
                        pair[4],
                        pair[5],
                        c[0],
                        c[1],
                        c[2],
                        q(g.radius),
                    ]);
                }
            }
        }
        creases.sort();
        soft.sort();
        circles.sort();
        polygons.sort();

        let mut s = Stream::default();
        s.n(faces.len() as u64);
        for f in &faces {
            s.ints(f);
        }
        for list in [&creases, &soft] {
            s.n(list.len() as u64);
            for e in list.iter() {
                s.ints(e);
            }
        }
        for list in [&circles, &polygons] {
            s.n(list.len() as u64);
            for e in list.iter() {
                s.ints(e);
            }
        }
        s.i(i64::from(self.smooth_level(oid)));
        s.i(paint(obj.default_material()));
        s.0
    }

    // ---------------------------------------------------------- the links

    /// Definition `def`'s `hew.library` stamp, or `None` for a stale or
    /// hidden definition or one with no item id.
    pub fn library_stamp(&self, def: ComponentId) -> Option<LibraryStamp> {
        use crate::attr::AttrValue;
        self.components.get(def).filter(|c| !c.hidden)?;
        let ns = self
            .attrs
            .get(&AttrTarget::Entity(EntityRef::Component(def)))?
            .get(LIBRARY_ATTR_NS)?;
        let text = |key: &str| match ns.get(key) {
            Some(AttrValue::Text(s)) => Some(s.clone()),
            _ => None,
        };
        Some(LibraryStamp {
            source_id: text("source_id")?,
            content_hash: text("content_hash"),
            def_sid: text("def_sid").and_then(|s| s.parse().ok()),
            print: text(LIBRARY_PRINT_KEY),
        })
    }

    /// Every live definition linked to a Library item, in slotmap order: a
    /// `hew.library` stamp with an item id. A definition made from a whole
    /// imported file ([`WRAPPED_DEF_SID`]) stands for no item definition
    /// and is left out.
    pub fn linked_definitions(&self) -> Vec<ComponentId> {
        self.components
            .iter()
            .filter(|(_, c)| !c.hidden)
            .map(|(cid, _)| cid)
            .filter(|&cid| {
                self.library_stamp(cid)
                    .is_some_and(|s| s.def_sid != Some(WRAPPED_DEF_SID))
            })
            .collect()
    }

    /// Where every linked definition stands against the Library (D2's
    /// table, [`library_link_state`]). `library` says what each item holds
    /// now; `digest` makes `mine`. A definition open for editing (or one
    /// placing it) is left out until the session closes. Read-only.
    pub fn library_links(
        &self,
        library: &dyn LibraryPrints,
        digest: PrintDigest,
    ) -> Vec<LibraryLink> {
        let mut memo = DigestMemo::new();
        let mut out = Vec::new();
        // A definition open for editing has its members in world
        // coordinates until the session closes, so neither it nor one
        // placing it can be printed honestly: it is left out until then.
        let on_loan = self.defs_on_loan();
        for def in self.linked_definitions() {
            if on_loan
                .iter()
                .any(|&l| l == def || self.def_reaches(def, l))
            {
                continue;
            }
            let Some(stamp) = self.library_stamp(def) else {
                continue;
            };
            let Ok(d) = self.content_digest(def, digest, &mut memo, &mut BTreeSet::new()) else {
                continue;
            };
            let mine = content_print_text(&d);
            let theirs = library
                .item_prints(&stamp.source_id)
                .and_then(|prints| stamp.def_sid.and_then(|s| prints.get(&s)))
                .cloned();
            let state = library_link_state(&mine, stamp.print.as_deref(), theirs.as_deref());
            out.push(LibraryLink {
                definition: def,
                source_id: stamp.source_id,
                def_sid: stamp.def_sid,
                base: stamp.print,
                mine,
                theirs,
                state,
                copies: self.definition_usage(def),
                frozen: self.library_update_lock(def).is_some(),
            });
        }
        out
    }

    /// The live definition whose stable id is `sid`.
    pub(crate) fn component_by_sid(&self, sid: u64) -> Option<ComponentId> {
        self.sids.iter().find_map(|(entity, &s)| match entity {
            EntityRef::Component(cid)
                if s == sid && self.components.get(*cid).is_some_and(|c| !c.hidden) =>
            {
                Some(*cid)
            }
            _ => None,
        })
    }

    // ------------------------------------------------------------- the save

    /// For each component instance in `nodes`, the stable id that the copy
    /// of its definition has in the item [`Document::extract_item`] makes
    /// of `nodes` from this document as it is now: what a save of `nodes`
    /// to the Library links each definition to
    /// ([`Document::stamp_library_save`]). The extraction mints ids in a
    /// fixed order, so these are the ids of the item file a save writes.
    /// Read-only.
    ///
    /// # Errors
    /// As [`Document::extract_item`].
    pub fn library_save_def_sids(
        &self,
        nodes: &[NodeId],
    ) -> Result<BTreeMap<ComponentId, u64>, DocumentError> {
        let (item, copies) = self.extract_item_copy_mapped(nodes, true, None)?;
        let mut out = BTreeMap::new();
        for &n in nodes {
            if let NodeId::Instance(i) = n
                && let Some(def) = self.instances.get(i).filter(|r| !r.hidden).map(|r| r.def)
                && let Some(&copy) = copies.get(&def)
                && let Some(sid) = item.sid_of(&EntityRef::Component(copy))
            {
                out.insert(def, sid);
            }
        }
        Ok(out)
    }

    /// Marks the source of a just-saved Library item, as
    /// [`Document::stamp_library_source`] does, with each selected
    /// instance's definition linked to ITS OWN definition in the item:
    /// `def_sids` ([`Document::library_save_def_sids`]) gives its stable id
    /// there, and `prov.prints` the item's print of it, stamped as the
    /// base. A definition `def_sids` does not name is stamped with no
    /// definition id and no base. Not an undo step, like
    /// [`Document::stamp_library_source`].
    pub fn stamp_library_save(
        &mut self,
        nodes: &[NodeId],
        prov: &LibraryProvenance,
        def_sids: &BTreeMap<ComponentId, u64>,
    ) {
        info!(target: "kernel::op", op = "stamp_library_save");
        self.stamp_library_nodes(nodes, prov, &|def| def_sids.get(&def).copied());
    }

    /// Why a lock would make Update from Library skip definition `def`,
    /// or `None`: the update replaces its contents, so it refuses when a
    /// lock freezes the definition, when its contents hold a locked node,
    /// or when a set of linked copies only its contents place (which goes
    /// with them) holds one. [`Document::library_links`] reports the same
    /// check as `frozen`, so a part the status offers is one the update
    /// takes. `None` for a stale or hidden definition.
    pub fn library_update_lock(&self, def: ComponentId) -> Option<DocumentError> {
        self.components.get(def).filter(|c| !c.hidden)?;
        self.refuse_library_update_lock(def).err()
    }

    /// [`Document::library_update_lock`] as a refusal, for a live `def`.
    fn refuse_library_update_lock(&self, def: ComponentId) -> Result<(), DocumentError> {
        self.refuse_def_frozen(def)?;
        self.refuse_def_holds_lock(def)?;
        let mut old: Vec<NodeId> = Vec::new();
        for &m in &self.components[def].members {
            self.collect_def_subtree(m, &mut old);
        }
        self.refuse_emptied_sets_frozen(&old)
    }

    // -------------------------------------------------- update from Library

    /// "Update from Library" (OW-2): replaces each requested definition's
    /// contents with a copy of its Library item's definition, as ONE undo
    /// step ("Update from Library") however many it covers.
    ///
    /// Each definition keeps its identity: its name, its attributes (the
    /// `hew.library` stamp is re-made: the item's `content_hash`, and the
    /// item's print as the new base) and every placement, which follows
    /// where it stands, since an item's origin is its definition's origin.
    /// The new contents are built by the copy engine a Library insert uses:
    /// paint matched to the palette by value or added, the item's new tags
    /// registered (a tag already here keeps its visibility). A definition
    /// the item nests resolves against the model's definitions from the
    /// same item (same item id and definition sid), those the old contents
    /// place first, then by stable id:
    ///
    /// - one that already holds the item's content is placed as it is;
    /// - else one that is in step or has only changed in the Library (its
    ///   state from `digest`, [`library_link_state`]) and that no lock
    ///   holds ([`Document::library_update_lock`]) is updated in the same
    ///   step ([`LibraryUpdateReport::also_updated`]);
    /// - else the item's definition is copied in fresh, and the ones
    ///   changed here or held by a lock are left exactly as they were
    ///   ([`LibraryUpdateReport::kept`]); likewise when the model holds
    ///   none.
    ///
    /// The old contents are hidden, not erased, so undo brings them back
    /// exactly; a set of linked copies only the old contents placed goes
    /// with them, as with a delete.
    ///
    /// When the step replaces anything, it also brings up to date the base
    /// print of every definition it touched that already held the item's
    /// content but carried an older base (a requested one reported
    /// unchanged, or a nested one placed as it is), so a later change in
    /// the Library reads as newer there rather than as changed on both
    /// sides. One whose contents a lock freezes keeps its stamp. A call
    /// that would only do that records nothing and leaves the base as it
    /// is: the link reads in step either way, since its content equals the
    /// item's.
    ///
    /// A requested definition that already holds the item's content is
    /// reported unchanged. One that is refused is skipped and named with
    /// its error ([`LibraryUpdateReport::skipped`]) and left exactly as it
    /// was; the rest go through. When nothing changes no step is recorded.
    /// Refusals:
    ///
    /// - [`DocumentError::ExplodeSessionScope`]: a component is open for
    ///   editing (the update could reach a definition on loan to it);
    /// - [`DocumentError::UnknownComponent`]: the definition is stale or
    ///   hidden, carries no Library link, is linked to another item than
    ///   the one given, or the item does not hold its definition;
    /// - [`DocumentError::LockedNode`]: a lock freezes the definition, its
    ///   contents hold a locked node, or a linked set its contents retire
    ///   holds one (OW-1, [`Document::library_update_lock`]);
    /// - [`DocumentError::ComponentCycle`]: the result would place a
    ///   definition inside itself;
    /// - [`DocumentError::ComponentDepthExceeded`] /
    ///   [`DocumentError::ComponentExpansionExceeded`]: the result would
    ///   nest too deeply or place too much.
    ///
    /// # Errors
    /// None of its own: every refusal is per definition, in the report.
    pub fn update_definitions_from_items(
        &mut self,
        updates: &[LibraryUpdate<'_>],
        digest: PrintDigest,
    ) -> Result<(LibraryUpdateReport, DocChange), DocumentError> {
        info!(target: "kernel::op", op = "update_definitions_from_items", count = updates.len());
        // A refusal found only once a part's contents are in place (a
        // cycle, a depth or width bound, a frozen set it would retire)
        // puts the whole document back and runs the batch again without
        // that part, so every refused part leaves the document exactly as
        // it was. Each run excludes one more part, so this ends.
        let checkpoint = self.clone();
        let mut excluded: BTreeMap<usize, DocumentError> = BTreeMap::new();
        loop {
            match self.apply_library_updates(updates, &excluded, digest) {
                Ok(done) => return Ok(self.commit_library_updates(done)),
                Err((index, e)) => {
                    *self = checkpoint.clone();
                    excluded.insert(index, e);
                }
            }
        }
    }

    /// One run of [`Document::update_definitions_from_items`]: every part
    /// not `excluded`, in order. `Err((index, error))` when part `index`
    /// failed a check made after its contents were placed; the caller
    /// restores the document.
    fn apply_library_updates(
        &mut self,
        updates: &[LibraryUpdate<'_>],
        excluded: &BTreeMap<usize, DocumentError>,
        digest: PrintDigest,
    ) -> Result<LibraryUpdateRun, (usize, DocumentError)> {
        let mut run = LibraryUpdateRun::default();
        // Base prints to bring up to date if the step replaces anything:
        // (definition, update index, its stable id in the item).
        let mut restamps: Vec<(ComponentId, usize, u64)> = Vec::new();
        for (index, u) in updates.iter().enumerate() {
            if let Some(&e) = excluded.get(&index) {
                run.report.skipped.push((u.definition, e));
                continue;
            }
            if run.report.updated.contains(&u.definition)
                || run.report.also_updated.contains(&u.definition)
            {
                // Already brought in step earlier in this batch.
                run.report.unchanged.push(u.definition);
                continue;
            }
            let plan = match self.plan_library_update(u, digest) {
                Ok(plan) => plan,
                Err(e) => {
                    run.report.skipped.push((u.definition, e));
                    continue;
                }
            };
            restamps.extend(plan.restamp.iter().map(|&(x, sid)| (x, index, sid)));
            for &k in &plan.kept {
                if !run.report.kept.contains(&k) {
                    run.report.kept.push(k);
                }
            }
            if plan.replace.is_empty() {
                run.report.unchanged.push(u.definition);
                continue;
            }
            self.apply_library_plan(u, &plan, &mut run)
                .map_err(|e| (index, e))?;
            run.report.updated.push(u.definition);
            for &(x, _) in plan.replace.iter().skip(1) {
                if !run.report.also_updated.contains(&x) {
                    run.report.also_updated.push(x);
                }
            }
        }
        // Only a step that replaces something carries base refreshes: an
        // undo step for a change nobody can see would be noise.
        if !run.swaps.is_empty() {
            for (x, index, sid) in restamps {
                if run.swaps.iter().any(|w| w.def == x) {
                    continue;
                }
                self.restamp_library_base(x, &updates[index].provenance, sid, &mut run);
            }
        }
        Ok(run)
    }

    /// Re-stamps definition `x` (which holds what the item holds) with
    /// `prov` and item definition id `sid`, as a swap that changes no
    /// member, so undo and redo carry the stamp exactly.
    fn restamp_library_base(
        &mut self,
        x: ComponentId,
        prov: &LibraryProvenance,
        sid: u64,
        run: &mut LibraryUpdateRun,
    ) {
        let target = AttrTarget::Entity(EntityRef::Component(x));
        let prior_stamp = self
            .attrs
            .get(&target)
            .and_then(|d| d.get(LIBRARY_ATTR_NS))
            .cloned();
        self.stamp_library_provenance(EntityRef::Component(x), prov, Some(sid));
        let next_stamp = self
            .attrs
            .get(&target)
            .and_then(|d| d.get(LIBRARY_ATTR_NS))
            .cloned();
        let members = self.components[x].members.clone();
        run.swaps.push(DefinitionSwap {
            def: x,
            prior_members: members.clone(),
            next_members: members,
            retired: Vec::new(),
            retired_sketches: Vec::new(),
            prior_stamp,
            next_stamp,
        });
    }

    /// What updating `u` takes, checked before anything changes: the
    /// definitions to replace (the requested one first; none when it
    /// already holds the item's content), the item definitions the copy
    /// maps onto existing ones, the bases to bring up to date, and the
    /// definitions left as they were.
    fn plan_library_update(
        &self,
        u: &LibraryUpdate<'_>,
        digest: PrintDigest,
    ) -> Result<LibraryPlan, DocumentError> {
        self.refuse_during_component_session()?;
        let def = u.definition;
        let stamp = self
            .library_stamp(def)
            .ok_or(DocumentError::UnknownComponent)?;
        let sid = stamp
            .def_sid
            .filter(|&s| s != WRAPPED_DEF_SID)
            .ok_or(DocumentError::UnknownComponent)?;
        if stamp.source_id != u.provenance.source_id {
            return Err(DocumentError::UnknownComponent);
        }
        let item_def = u
            .item
            .component_by_sid(sid)
            .ok_or(DocumentError::UnknownComponent)?;
        let mut plan = LibraryPlan::default();
        if self.content_equal(def, u.item, item_def) {
            if self.needs_base_refresh(def, &stamp, &u.provenance, sid) {
                plan.restamp.push((def, sid));
            }
            return Ok(plan);
        }
        // A lock on the definition, on anything its old contents hold, or
        // on a linked set only they place refuses: the old contents go
        // (OW-1). The status reports the same check.
        self.refuse_library_update_lock(def)?;

        plan.replace.push((def, item_def));
        plan.map.push((item_def, def));
        // The item's definitions nested below this one, each matched to
        // the model's definition from the same item, if it holds one that
        // may take the item's content.
        let mut seen: BTreeSet<ComponentId> = BTreeSet::from([item_def]);
        let mut queue: Vec<ComponentId> = u.item.def_child_defs(item_def);
        while let Some(j) = queue.pop() {
            if !seen.insert(j) {
                continue;
            }
            queue.extend(u.item.def_child_defs(j));
            let Some(jsid) = u
                .item
                .sid_of(&EntityRef::Component(j))
                .filter(|&s| s != WRAPPED_DEF_SID)
            else {
                continue;
            };
            // Those the old contents place first, then by stable id, so
            // the choice never rests on slotmap order.
            let mut candidates: Vec<(bool, u64, ComponentId, LibraryStamp)> = self
                .components
                .iter()
                .filter(|&(cid, c)| !c.hidden && cid != def)
                .filter_map(|(cid, _)| {
                    let s = self.library_stamp(cid)?;
                    (s.source_id == stamp.source_id && s.def_sid == Some(jsid)).then(|| {
                        let sid = self.sid_of(&EntityRef::Component(cid)).unwrap_or(u64::MAX);
                        (!self.def_reaches(def, cid), sid, cid, s)
                    })
                })
                .collect();
            candidates.sort_by_key(|(far, sid, cid, _)| (*far, *sid, *cid));
            if let Some((_, _, same, s)) = candidates
                .iter()
                .find(|(_, _, e, _)| self.content_equal(*e, u.item, j))
            {
                plan.map.push((j, *same));
                if self.needs_base_refresh(*same, s, &u.provenance, jsid) {
                    plan.restamp.push((*same, jsid));
                }
                continue;
            }
            // Replaced in place only when nothing here would be lost: in
            // step or changed only in the Library, and no lock holds it.
            let theirs = u.item.content_print(j, digest).ok();
            let takes = |e: ComponentId, s: &LibraryStamp| -> bool {
                let (Ok(mine), Some(theirs)) = (self.content_print(e, digest), theirs.as_deref())
                else {
                    return false;
                };
                matches!(
                    library_link_state(&mine, s.print.as_deref(), Some(theirs)),
                    LibraryLinkState::InStep | LibraryLinkState::LibraryNewer
                ) && self.refuse_library_update_lock(e).is_ok()
            };
            match candidates.iter().find(|(_, _, e, s)| takes(*e, s)) {
                Some(&(_, _, e, _)) => {
                    plan.map.push((j, e));
                    plan.replace.push((e, j));
                }
                // Copied in fresh; the edited or locked ones stay as they
                // are.
                None => plan.kept.extend(candidates.iter().map(|(_, _, e, _)| *e)),
            }
        }
        Ok(plan)
    }

    /// Whether definition `x` (holding what the item's definition `sid`
    /// holds) carries a base other than the item's print of it, and may be
    /// re-stamped: no lock freezes it.
    fn needs_base_refresh(
        &self,
        x: ComponentId,
        stamp: &LibraryStamp,
        prov: &LibraryProvenance,
        sid: u64,
    ) -> bool {
        prov.prints
            .get(&sid)
            .is_some_and(|theirs| stamp.print.as_ref() != Some(theirs))
            && self.def_frozen(x).is_none()
    }

    /// Puts `plan` in place: each definition's old contents hidden, the
    /// item's copied in, the stamp re-made; then the checks only the result
    /// can answer.
    fn apply_library_plan(
        &mut self,
        u: &LibraryUpdate<'_>,
        plan: &LibraryPlan,
        run: &mut LibraryUpdateRun,
    ) -> Result<(), DocumentError> {
        use crate::attr::AttrValue;
        let mut ctx = LibraryCopy::new(Some(u.provenance.clone()));
        for &(item_c, model_c) in &plan.map {
            ctx.def_map.insert(item_c, model_c);
        }
        let mut retired_all: Vec<NodeId> = Vec::new();
        for &(x, j) in &plan.replace {
            let prior_members = self.components[x].members.clone();
            let mut retired = Vec::new();
            for &m in &prior_members {
                self.collect_def_subtree(m, &mut retired);
            }
            let retired_sketches: Vec<SketchId> = self
                .def_sketches
                .iter()
                .filter(|&(s, &c)| {
                    c == x
                        && !self.sketch_tombstoned(s)
                        && !self.object_sketches.contains_key(s)
                        && !self.group_sketches.contains_key(s)
                })
                .map(|(&s, _)| s)
                .collect();
            for &n in &retired {
                self.set_node_row_hidden(n, true);
            }
            for &s in &retired_sketches {
                self.hidden_sketches.insert(s);
            }
            let next_members = library_fill_def(self, u.item, j, x, &mut ctx);
            self.components[x].members = next_members.clone();

            let target = AttrTarget::Entity(EntityRef::Component(x));
            let prior_stamp = self
                .attrs
                .get(&target)
                .and_then(|d| d.get(LIBRARY_ATTR_NS))
                .cloned();
            self.stamp_library_provenance(
                EntityRef::Component(x),
                &u.provenance,
                u.item.sid_of(&EntityRef::Component(j)),
            );
            let next_stamp = self
                .attrs
                .get(&target)
                .and_then(|d| d.get(LIBRARY_ATTR_NS))
                .cloned();
            debug_assert!(
                next_stamp
                    .as_ref()
                    .is_some_and(|ns| matches!(ns.get("source_id"), Some(AttrValue::Text(_)))),
                "an updated definition keeps its link"
            );
            retired_all.extend(retired.iter().copied());
            run.swaps.push(DefinitionSwap {
                def: x,
                prior_members,
                next_members,
                retired,
                retired_sketches,
                prior_stamp,
                next_stamp,
            });
        }

        // Only the result can say whether it nests a definition in itself,
        // nests too deeply, or places too much.
        for &(x, _) in &plan.replace {
            if self.def_reaches(x, x) {
                return Err(DocumentError::ComponentCycle);
            }
            if self.def_ancestry_depth(x).saturating_add(self.def_depth(x)) > MAX_COMPONENT_DEPTH {
                return Err(DocumentError::ComponentDepthExceeded);
            }
        }
        if self.expanded_total() > MAX_EXPANDED_PLACEMENTS {
            return Err(DocumentError::ComponentExpansionExceeded);
        }
        // A set of linked copies placed only by the old contents goes with
        // them, unless a lock holds it.
        self.refuse_emptied_sets_frozen(&retired_all)?;
        let hidden = self.hide_emptied_linked_sets_of(&retired_all);
        run.hidden_defs.extend(hidden);

        // The item's tag registry, as an insert registers it: only new
        // paths, with the item's own hidden flags.
        for (path, &hidden) in u.item.tag_meta.iter() {
            if !path.is_empty() && !self.tag_meta.contains_key(path) {
                self.register_tag(path.clone(), hidden);
                run.tags.push((path.clone(), hidden));
            }
        }
        run.report.definitions_added += ctx.report.definitions_added;
        run.report.materials_added += ctx.report.materials_added;
        run.objects.extend(ctx.all_objects);
        run.groups.extend(ctx.all_groups);
        run.instances.extend(ctx.all_instances);
        run.components.extend(ctx.all_components);
        run.sketches.extend(ctx.all_sketches);
        Ok(())
    }

    /// Records a finished run as one undo step (none when nothing
    /// changed) and reports what it touched.
    fn commit_library_updates(
        &mut self,
        run: LibraryUpdateRun,
    ) -> (LibraryUpdateReport, DocChange) {
        let LibraryUpdateRun {
            report,
            swaps,
            objects,
            groups,
            instances,
            components,
            sketches,
            hidden_defs,
            tags,
        } = run;
        if swaps.is_empty() {
            return (report, DocChange::default());
        }
        // A linked set the old contents placed, left with one copy, has
        // that copy made plain in the same entry (the last-copy rule).
        let mut removed: Vec<NodeId> = swaps
            .iter()
            .flat_map(|s| s.retired.iter().copied())
            .collect();
        for (_, nodes) in &hidden_defs {
            removed.extend(nodes.iter().copied());
        }
        let seeds = self.sets_losing_copies(&removed, &instances);
        let action = DocAction::UpdatedFromLibrary {
            swaps,
            objects,
            groups,
            instances,
            components,
            sketches,
            hidden_defs,
            tags,
        };
        let mut change = self.library_update_change(&action);
        let bake = self.bake_lone_copies(&seeds, &[], &mut change);
        self.undo.push(with_last_copies(action, bake));
        self.commit_new_action();
        self.debug_validate();
        (report, change)
    }

    /// Hides or reveals one member row, nothing else.
    fn set_node_row_hidden(&mut self, node: NodeId, hidden: bool) {
        match node {
            NodeId::Object(id) => self.objects[id].hidden = hidden,
            NodeId::Group(id) => self.groups[id].hidden = hidden,
            NodeId::Instance(id) => self.instances[id].hidden = hidden,
        }
    }

    /// Writes a definition's `hew.library` namespace as recorded, removing
    /// it (and an emptied dictionary) for `None`.
    fn put_library_stamp(
        &mut self,
        def: ComponentId,
        stamp: &Option<BTreeMap<String, crate::attr::AttrValue>>,
    ) {
        let target = AttrTarget::Entity(EntityRef::Component(def));
        match stamp {
            Some(ns) => {
                self.attrs
                    .entry(target)
                    .or_default()
                    .insert(LIBRARY_ATTR_NS.to_string(), ns.clone());
            }
            None => {
                if let Some(dict) = self.attrs.get_mut(&target) {
                    dict.remove(LIBRARY_ATTR_NS);
                    if dict.is_empty() {
                        self.attrs.remove(&target);
                    }
                }
            }
        }
    }

    /// Undo (`forward: false`) or redo (`true`) of a
    /// [`DocAction::UpdatedFromLibrary`]: the exact rows, sketches, member
    /// lists and stamps flip back and forth. Palette materials the update
    /// added stay either way, like every palette addition.
    pub(super) fn replay_library_update(&mut self, action: &DocAction, forward: bool) -> DocChange {
        let DocAction::UpdatedFromLibrary {
            swaps,
            objects,
            groups,
            instances,
            components,
            sketches,
            hidden_defs,
            tags,
        } = action
        else {
            unreachable!("called with an UpdatedFromLibrary action");
        };
        let created = |doc: &mut Document, hidden: bool| {
            for &o in objects {
                doc.objects[o].hidden = hidden;
            }
            for &g in groups {
                doc.groups[g].hidden = hidden;
            }
            for &i in instances {
                doc.instances[i].hidden = hidden;
            }
            for &c in components {
                doc.components[c].hidden = hidden;
            }
            for &s in sketches {
                if hidden {
                    doc.hidden_sketches.insert(s);
                } else {
                    doc.hidden_sketches.remove(&s);
                }
            }
        };
        if forward {
            created(self, false);
            for (path, hidden) in tags {
                self.register_tag(path.clone(), *hidden);
            }
            for swap in swaps {
                for &n in &swap.retired {
                    self.set_node_row_hidden(n, true);
                }
                for &s in &swap.retired_sketches {
                    self.hidden_sketches.insert(s);
                }
                self.components[swap.def].members = swap.next_members.clone();
                self.put_library_stamp(swap.def, &swap.next_stamp);
            }
            for (d, nodes) in hidden_defs {
                self.set_definition_hidden(*d, nodes, true);
            }
        } else {
            for (d, nodes) in hidden_defs.iter().rev() {
                self.set_definition_hidden(*d, nodes, false);
            }
            for swap in swaps.iter().rev() {
                self.put_library_stamp(swap.def, &swap.prior_stamp);
                self.components[swap.def].members = swap.prior_members.clone();
                for &s in &swap.retired_sketches {
                    self.hidden_sketches.remove(&s);
                }
                for &n in &swap.retired {
                    self.set_node_row_hidden(n, false);
                }
            }
            for (path, _) in tags {
                self.tag_meta.remove(path);
            }
            created(self, true);
        }
        self.library_update_change(action)
    }

    /// What a [`DocAction::UpdatedFromLibrary`] (or its undo or redo)
    /// touched: the replaced definitions with every placement rendering
    /// them, the rows and sketches on both sides, and the sets it hid.
    fn library_update_change(&self, action: &DocAction) -> DocChange {
        let DocAction::UpdatedFromLibrary {
            swaps,
            objects,
            groups,
            instances,
            components,
            sketches,
            hidden_defs,
            ..
        } = action
        else {
            unreachable!("called with an UpdatedFromLibrary action");
        };
        let mut change = DocChange {
            objects_touched: objects.clone(),
            groups_touched: groups.clone(),
            instances_touched: instances.clone(),
            components_touched: components.clone(),
            sketches_touched: sketches.clone(),
            ..Default::default()
        };
        for swap in swaps {
            merge_unique(&mut change.components_touched, vec![swap.def]);
            // A swap that only re-stamps changes nothing a copy draws.
            if swap.prior_members != swap.next_members || !swap.retired.is_empty() {
                merge_unique(
                    &mut change.instances_touched,
                    self.placing_instances(swap.def),
                );
            }
            for &n in &swap.retired {
                match n {
                    NodeId::Object(o) => merge_unique(&mut change.objects_touched, vec![o]),
                    NodeId::Group(g) => merge_unique(&mut change.groups_touched, vec![g]),
                    NodeId::Instance(i) => merge_unique(&mut change.instances_touched, vec![i]),
                }
            }
            merge_unique(&mut change.sketches_touched, swap.retired_sketches.clone());
        }
        touch_hidden_defs(&mut change, hidden_defs);
        change
    }
}

/// [`Document::plan_library_update`]'s answer.
#[derive(Default)]
struct LibraryPlan {
    /// `(model definition, item definition)` pairs whose contents are
    /// replaced, the requested definition first; empty when it already
    /// holds the item's content.
    replace: Vec<(ComponentId, ComponentId)>,
    /// `(item definition, model definition)`: every item definition the
    /// copy places as an existing model definition (the replaced ones
    /// included).
    map: Vec<(ComponentId, ComponentId)>,
    /// `(model definition, item definition sid)`: definitions that hold
    /// the item's content but carry an older base.
    restamp: Vec<(ComponentId, u64)>,
    /// Definitions from the same item left as they were (changed here, or
    /// held by a lock); the item's definition is copied in fresh.
    kept: Vec<ComponentId>,
}

/// One run's accumulating result ([`Document::apply_library_updates`]).
#[derive(Default)]
struct LibraryUpdateRun {
    report: LibraryUpdateReport,
    swaps: Vec<DefinitionSwap>,
    objects: Vec<ObjectId>,
    groups: Vec<GroupId>,
    instances: Vec<InstanceId>,
    components: Vec<ComponentId>,
    sketches: Vec<SketchId>,
    hidden_defs: Vec<HiddenDef>,
    tags: Vec<(Vec<String>, bool)>,
}

/// A sketch's record: each edge as a sorted pair of rounded ends plus its
/// circle claim (center, radius) when it has one, sorted. Empty for a
/// sketch with no edge.
fn sketch_record(sk: &Sketch) -> Vec<i64> {
    let mut edges: Vec<[i64; 11]> = Vec::with_capacity(sk.edges().len());
    for (e, edge) in sk.edges().iter() {
        let (Some(a), Some(b)) = (sk.vertices().get(edge.from), sk.vertices().get(edge.to)) else {
            continue;
        };
        let (qa, qb) = (qp(a.position), qp(b.position));
        let [lo, hi] = if qa <= qb { [qa, qb] } else { [qb, qa] };
        let curve = sk.edge_curve(e).and_then(|c| sk.curve_geom(c));
        let (flag, c, r) = match curve {
            Some(g) => (1, qp(g.center), q(g.radius)),
            None => (0, [0; 3], 0),
        };
        edges.push([
            lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], flag, c[0], c[1], c[2], r,
        ]);
    }
    edges.sort();
    let mut out = Vec::with_capacity(edges.len() * 11 + 1);
    if edges.is_empty() {
        return out;
    }
    out.push(edges.len() as i64);
    for e in edges {
        out.extend(e);
    }
    out
}
