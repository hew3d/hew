//! SketchUp `.skp` import: clean-room Skpwright reader -> Hew Objects/Instances.
//!
//! Provenance: the `skpwright` dependency is **Skpwright**
//! (https://github.com/hew3d/skpwright), a clean-room reader for SketchUp
//! files — the 2026 container (`{26.x}`, a ZIP holding a tagged record
//! tree) that every current SketchUp release saves, the free web app
//! included, and the 2017 `CArchive` stream (`{17.x}`) of older files —
//! derived solely from self-authored corpora, their COLLADA ground-truth
//! exports, and public knowledge of MFC `CArchive` serialization and the
//! ZIP format. Its only runtime dependency is a DEFLATE decoder. **No
//! Trimble/SketchUp SDK material exists anywhere in this crate's dependency
//! chain** — that is the admissibility test docs/dev/DEVELOPMENT.md rule 7
//! states, and it must hold for every future dependency added here.
//!
//! Shape mirrors `dae-import`: depends on `kernel` (builds Objects), never the
//! reverse; takes bytes, no filesystem or network. The entry point is
//! [`import`]; it emits a `kernel::ImportScene` recipe which
//! `Document::ingest` materialises. Import-quality gaps are fixed upstream in
//! Skpwright and the rev pin advanced — never papered over here ( rule).

pub use skpwright;

mod convert;
mod extras;
mod material;
pub use material::pbr_of;

/// Typed failures from [`import`].
#[derive(Debug, Clone)]
pub enum SkpError {
    /// The bytes are not a SketchUp file at all.
    NotSkp,
    /// A SketchUp file the reader could not read: a `CArchive` stream
    /// from a release Skpwright does not list (releases 9 to 12 and any
    /// later stream are unobserved) that Skpwright itself refused, or an
    /// intact file whose container is neither layout and whose version is
    /// not one the reader decodes. Carries the file's own version string
    /// (e.g. `"{21.1.0}"`) so the UI can say which SketchUp wrote it and
    /// suggest saving it again from a current SketchUp (the free web app
    /// writes the 2026 container). Any file in the 2026 ZIP container is
    /// read, whatever its version says: that is the layout current
    /// releases share; a stream container of an unlisted release is read
    /// when Skpwright can read it, with a warning.
    UnsupportedVersion {
        /// The header's version string, brace form.
        version: String,
    },
    /// A recognised container that failed to parse.
    Parse(String),
    /// The file parsed but yielded NO importable content (no meshes, no
    /// placements) — with the recovery-note count when the parser had to
    /// resync, since heavy recovery is the usual cause. Failing loudly here
    /// beats a silent empty document pretending the import succeeded.
    EmptyScene {
        /// How many parser recovery notes accompanied the empty result.
        recovery_notes: usize,
    },
    /// The file is cut short or damaged and nothing of its model could be
    /// read: a SketchUp header with no readable model behind it, a 2026
    /// container missing its ZIP end record, or a stream file the
    /// whole-file grammar refused for running out of data that yielded
    /// nothing. A file that is cut
    /// short (a download or copy that did not finish) is the usual cause,
    /// so the copy asks for a fresh copy rather than a re-save.
    Damaged,
}

impl std::fmt::Display for SkpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkpError::NotSkp => write!(f, "not a SketchUp .skp file"),
            SkpError::UnsupportedVersion { version } => write!(
                f,
                "unsupported SketchUp version {version}: this reader could not read \
                 a file of that release. Open it in SketchUp (the free web app \
                 works) and save it again as a current .skp, then import that"
            ),
            SkpError::Parse(msg) => write!(f, ".skp parse error: {msg}"),
            SkpError::EmptyScene { recovery_notes: 0 } => {
                write!(f, "the .skp file contains no importable geometry")
            }
            SkpError::EmptyScene { recovery_notes } => write!(
                f,
                "no importable geometry survived parsing ({recovery_notes} \
                 malformed sections skipped) — the file may be damaged; \
                 re-saving it from SketchUp usually rewrites it cleanly"
            ),
            SkpError::Damaged => write!(
                f,
                "this .skp file looks incomplete or damaged. It may not have \
                 finished downloading or copying, so get a fresh copy and open that"
            ),
        }
    }
}

impl std::error::Error for SkpError {}

/// A parsed `.skp`, ready for `Document::ingest`.
pub struct SkpScene {
    /// The import recipe (`ingest` consumes it).
    pub scene: kernel::ImportScene,
    /// Texture names whose embedded image bytes were absent (their materials
    /// fall back to the average color) — pass to `ingest` as
    /// `textures_missing`.
    pub textures_missing: Vec<String>,
    /// Reader recovery notes: non-empty means the reader skipped a damaged
    /// or unknown record (2026) or had to resync inside a malformed section
    /// (2017) and content may be missing. Clean SketchUp files of either
    /// container produce none (an Skpwright regression guarantee). Surface
    /// these in the import report; fixes belong upstream in Skpwright.
    pub warnings: Vec<String>,
    /// Where each imported node and definition came from in the file, by
    /// importer key ([`kernel::ImportReport::keys`]) and definition recipe
    /// index ([`kernel::ImportReport::defs`]): what a later save back to
    /// the file needs to put unchanged content back where it was.
    pub provenance: Provenance,
}

/// Where an imported node came from in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// The model's loose geometry (an Object).
    Loose,
    /// The geometry of a group placement standing alone for the group (an
    /// Object), by the placement's persistent id.
    GroupGeometry { pid: u32 },
    /// A group placement's own geometry beside its children (an Object).
    GroupOwnGeometry { pid: u32 },
    /// A group placement (a Group).
    Group { pid: u32 },
    /// A component placement (an Instance).
    Instance { pid: u32 },
    /// A placed image among the model's own entities (an Instance of the
    /// image's definition): a save back keeps the file's image, moved
    /// where the Instance is.
    Image { pid: u32 },
    /// A component definition's own geometry (an Object inside a
    /// definition): the definition's index in the file's definition list
    /// and the paint its placement passed down.
    DefinitionGeometry { definition: usize, slot: u16 },
}

/// Provenance of an import (see [`SkpScene::provenance`]).
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    /// Importer key -> origins. A key can name a group placement's geometry
    /// and the group both; the node's kind tells which origin is its own.
    pub nodes: Vec<(u64, Origin)>,
    /// Definition recipe index -> (the file definition's name, the paint
    /// its placements passed down): each Hew definition is one material
    /// variant of a file definition. Names identify a component definition
    /// in SketchUp (they are unique), and survive a rewrite of the file
    /// where list positions do not.
    pub defs: Vec<(usize, String, u16)>,
    /// Annotation index (into the scene's annotations) -> the SketchUp
    /// text or dimension it was read from: a save back keeps that record
    /// while the annotation is unchanged.
    pub annotations: Vec<(usize, AnnotationOrigin)>,
    /// Tag path -> the file layer it was read from, by name: a save back
    /// renames the layer in place when the tag is renamed, and knows a
    /// layer no tag stands for any more.
    pub tags: Vec<(Vec<String>, String)>,
    /// Scene recipe index (into the scene's scenes) -> the index of the
    /// file scene it was read from: a save back gives the document's scene
    /// the page state that file scene holds, whatever either is named now.
    pub scenes: Vec<(usize, usize)>,
    /// Sketch indices (into the scene's sketches) read from the file's
    /// loose edges: a save back keeps those edges as they are while the
    /// sketches are unchanged.
    pub loose_sketches: Vec<usize>,
}

/// The tag-path segment of each of sibling layer folders, in order: the
/// folder's own name, except that a later sibling of the same name takes
/// `Name (2)`, `Name (3)`, … (the first free), so each folder is a tag of
/// its own. Deterministic in the siblings alone, so a save back derives
/// the same segments from the file and writes each folder under the name
/// the file gave it.
pub fn folder_segments<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    names
        .iter()
        .map(|n| {
            let name = n.as_ref();
            let mut seg = name.to_string();
            let mut k = 2;
            while !used.insert(seg.clone()) {
                seg = format!("{name} ({k})");
                k += 1;
            }
            seg
        })
        .collect()
}

/// The SketchUp record an imported annotation was read from: its list in
/// the file, its index there and its persistent id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnnotationOrigin {
    Text { index: usize, pid: u32 },
    Dimension { index: usize, pid: u32 },
    RadialDimension { index: usize, pid: u32 },
}

/// Parse SketchUp `.skp` bytes — the 2026 container or the stream container —
/// into a kernel `ImportScene` recipe.
///
/// I/O-free; textures are embedded in the format, so unlike `dae-import` no
/// host-resolved image map is needed. Per-mesh geometry issues are NOT errors
/// here — meshes are emitted as-is and `Document::ingest` skips and reports
/// any it rejects (rule 4: reported, never repaired).
pub fn import(bytes: &[u8]) -> Result<SkpScene, SkpError> {
    let mut job = import_begin(bytes)?;
    job.step(usize::MAX);
    job.finish()
}

/// Start an import: read the file (the one step that cannot be split)
/// and set up the conversion. See [`ImportJob`].
pub fn import_begin(bytes: &[u8]) -> Result<ImportJob, SkpError> {
    // Container gate: identify ANY SketchUp file from its header and let
    // its body decide, not its version: a stream container (the MFC
    // `CArchive` layout of releases 3 to 8 and 2013 to 2020) or the 2026
    // ZIP container is handed to Skpwright whatever version the header
    // says. A stream file of a release Skpwright does not list (9 to 12, or
    // later than 2020) is read as far as Skpwright can read it, with a
    // warning, and refused only when Skpwright itself refuses it. A header
    // whose body is neither layout is damage when its major is one the
    // reader decodes, an unsupported version otherwise.
    let (version, _guid) = skpwright::header_info(bytes).ok_or(SkpError::NotSkp)?;
    let container = skpwright::detect_container(bytes);
    match container {
        skpwright::Container::Zip | skpwright::Container::Stream => {}
        skpwright::Container::Unknown if decoded_major(&version) => {
            return Err(SkpError::Damaged);
        }
        skpwright::Container::Unknown => {
            return Err(SkpError::UnsupportedVersion { version });
        }
    }
    let unlisted = matches!(container, skpwright::Container::Stream)
        && skpwright::Release::of_version(&version).is_none();

    let model = skpwright::Model::parse(bytes).map_err(|e| {
        // A 2026 container keeps its ZIP directory at the very end, so a
        // file cut short loses it first.
        if matches!(container, skpwright::Container::Zip) && !has_zip_end_record(bytes) {
            SkpError::Damaged
        } else if unlisted {
            // Skpwright refused a release it does not list: say which.
            SkpError::UnsupportedVersion {
                version: version.clone(),
            }
        } else {
            SkpError::Parse(e.to_string())
        }
    })?;
    // A DAMAGED parse that composes NOTHING importable is a loud failure,
    // not a silent empty success (the failure mode that motivated this: a
    // 57 MB house model desyncing 32k times and importing as an empty
    // document). A CLEAN parse that yields nothing is trusted — genuinely
    // empty models and guide-only files exist, and their uninstanced
    // template-figure faces are correctly not imported.
    let recovery_notes = model.diagnostics.iter().filter(|d| d.is_desync()).count();
    // The whole-file grammar reads every complete 2017 file it can decode
    // from header to last byte, and refuses a file cut short for running out
    // of data, often after the walk fallback has already found every shape
    // in it. The refusal is informational to the reader, not a desync, but
    // that one means the file is incomplete: refused outright when nothing
    // came of it, said out loud when something did. A refusal for anything
    // else (a structure the grammar does not decode, in a healthy file)
    // leaves the walk's result as it was.
    let incomplete = model.diagnostics.iter().any(|d| {
        matches!(d, skpwright::Diagnostic::WholeFileRefused { detail } if ran_out_of_data(detail))
    });
    Ok(ImportJob {
        job: convert::ConvertJob::begin(std::rc::Rc::new(model)),
        recovery_notes,
        incomplete,
        unlisted: unlisted.then_some(version),
    })
}

/// The first of [`SkpScene::warnings`] when the file is incomplete or damaged
/// but something of it was read: the 2017 whole-file grammar refused it for
/// running out of data and the walk fallback served it (see [`import_begin`]). Exported so a caller
/// can tell this note from the rest and show it on its own.
pub const INCOMPLETE_NOTE: &str = "this .skp file looks incomplete or damaged, so parts \
     of the model may be missing. If it was downloaded or copied, a fresh copy may \
     open whole";

/// True when a whole-file refusal's `detail` says the stream ran out: a
/// read past the last byte, a declared count larger than the bytes left, or
/// a string longer than them. These are Skpwright 0.5's three wordings (its
/// `ar17` reader); the cut-short specs in `tests/import_specs.rs` fail if a
/// later release rewords them.
fn ran_out_of_data(detail: &str) -> bool {
    detail.contains("stream ends at")
        || detail.contains("bytes that remain")
        || detail.contains("runs past the stream")
}

/// True when `bytes` end with a ZIP end-of-central-directory record: its
/// signature 22 bytes plus the record's own comment length before the end.
/// (The signature alone also turns up inside compressed entries.)
fn has_zip_end_record(bytes: &[u8]) -> bool {
    const SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const FIXED: usize = 22;
    let lowest = bytes.len().saturating_sub(FIXED + 0xffff);
    (lowest..=bytes.len().saturating_sub(FIXED)).rev().any(|p| {
        bytes.len() >= FIXED
            && bytes[p..p + 4] == SIG
            && p + FIXED + usize::from(u16::from_le_bytes([bytes[p + 20], bytes[p + 21]]))
                == bytes.len()
    })
}

/// An import under way: the file is read ([`import_begin`]); its shapes
/// are then built a few at a time ([`ImportJob::step`]), so an open can
/// say how far it has come, and [`ImportJob::finish`] hands the recipe
/// over.
pub struct ImportJob {
    job: convert::ConvertJob,
    recovery_notes: usize,
    /// The 2017 whole-file grammar refused the file for running out of
    /// data (see [`import_begin`]).
    incomplete: bool,
    /// The header's version when the file is a stream container of a
    /// release Skpwright does not list (read anyway, with a warning).
    unlisted: Option<String>,
}

impl ImportJob {
    /// Items built so far, out of all of them: the file's runs of loose
    /// geometry and its top-level placements.
    pub fn progress(&self) -> (usize, usize) {
        self.job.progress()
    }

    /// Shapes (definition recipes) built so far.
    pub fn shapes(&self) -> usize {
        self.job.shapes()
    }

    /// Build up to `items` more; true once everything is built.
    pub fn step(&mut self, items: usize) -> bool {
        self.job.step(items)
    }

    /// Build whatever is left and hand the recipe over.
    pub fn finish(self) -> Result<SkpScene, SkpError> {
        let mut out = self.job.finish();
        let empty = out.scene.roots.is_empty()
            && out.scene.defs.is_empty()
            && out.scene.guides.is_empty()
            && out.scene.extra.sketches.is_empty();
        // A release Skpwright does not list that yielded nothing was not
        // read at all, however clean the reader found it.
        if let (true, Some(version)) = (empty, &self.unlisted) {
            return Err(SkpError::UnsupportedVersion {
                version: version.clone(),
            });
        }
        if empty && self.incomplete {
            return Err(SkpError::Damaged);
        }
        if empty && self.recovery_notes > 0 {
            return Err(SkpError::EmptyScene {
                recovery_notes: self.recovery_notes,
            });
        }
        if let Some(version) = &self.unlisted {
            out.warnings.insert(0, unlisted_note(version));
        }
        if self.incomplete {
            out.warnings.insert(0, INCOMPLETE_NOTE.to_string());
        }
        Ok(SkpScene {
            scene: out.scene,
            textures_missing: out.textures_missing,
            warnings: out.warnings,
            provenance: out.provenance,
        })
    }
}

/// The first of [`SkpScene::warnings`] when the file is a stream container
/// of a release Skpwright does not list but could read: nothing has been
/// checked against a real file of that release.
pub fn unlisted_note(version: &str) -> String {
    format!(
        "this file is release {version}, which Skpwright does not list; it was read as the \
         stream container anyway, so parts of the model may be missing or wrong"
    )
}

/// True when the header's major version is one whose container the reader
/// decodes: a stream release (3 to 8, 2013 to 2020) or 26 and later (the
/// ZIP container). A file of such a version whose body is neither layout
/// is damaged rather than newer than the reader.
fn decoded_major(version: &str) -> bool {
    match version_major(version) {
        Some(m) => m >= 26 || skpwright::Release::of_version(version).is_some(),
        None => false,
    }
}

/// Major version from the header's brace form: `"{17.3.116}"` -> 17.
fn version_major(version: &str) -> Option<u32> {
    version
        .trim_start_matches('{')
        .split(['.', '}'])
        .next()?
        .parse()
        .ok()
}

/// Topology counts of each geometry run in a `.skp` byte stream.
///
/// The thinnest end-to-end use of the clean-room reader ( probe); kept
/// as a cheap pre-import inspection alongside the full import pipeline.
pub fn probe_topology(bytes: &[u8]) -> Vec<skpwright::Topology> {
    skpwright::geometry_runs(bytes)
        .into_iter()
        .map(|run| run.topology)
        .collect()
}
