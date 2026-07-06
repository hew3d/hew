//! SketchUp `.skp` import: clean-room OpenSKP reader -> Hew Objects/Instances.
//!
//! Provenance : the
//! `openskp` dependency is **OpenSKP** (https://github.com/hew3d/openskp,
//! formerly the unhosted sibling `OpenSKP`), a zero-dependency reader for
//! SketchUp 2017 (v17.3.116) files, derived solely from self-authored corpora,
//! their COLLADA ground-truth exports, and public knowledge of MFC `CArchive`
//! serialization. **No Trimble/SketchUp SDK material exists anywhere in this
//! crate's dependency chain** — that is the admissibility test docs/DEVELOPMENT.md
//! rule 7 states, and it must hold for every future dependency added here.
//!
//! Shape mirrors `dae-import`: depends on `kernel` (builds Objects), never the
//! reverse; takes bytes, no filesystem or network. The entry point is
//! [`import`]; it emits a `kernel::ImportScene` recipe which
//! `Document::ingest` materialises. Import-quality gaps are fixed upstream in
//! OpenSKP and the rev pin advanced — never papered over here ( rule).

pub use openskp;

mod convert;
mod material;

/// Typed failures from [`import`].
#[derive(Debug, Clone)]
pub enum SkpError {
    /// The bytes are not a SketchUp file at all.
    NotSkp,
    /// A SketchUp file from an unsupported version (only 2017, `{17.x}`, is
    /// fully decoded). Carries the file's own version string (e.g.
    /// `"{26.2.0}"`) so the UI can say which SketchUp wrote it and suggest
    /// **File ▸ Save As ▸ SketchUp Version 2017** — every modern SketchUp can.
    UnsupportedVersion {
        /// The header's version string, brace form.
        version: String,
    },
    /// A 2017 container that failed to parse.
    Parse(String),
}

impl std::fmt::Display for SkpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkpError::NotSkp => write!(f, "not a SketchUp .skp file"),
            SkpError::UnsupportedVersion { version } => write!(
                f,
                "unsupported SketchUp version {version}: open it in SketchUp and \
                 File \u{25b8} Save As \u{25b8} SketchUp Version 2017, then import that"
            ),
            SkpError::Parse(msg) => write!(f, ".skp parse error: {msg}"),
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
    /// Parser recovery notes: non-empty means the reader had to resync inside
    /// a malformed section and content may be missing. Clean SketchUp 2017
    /// files produce none (an OpenSKP regression guarantee). Surface these in
    /// the import report; fixes belong upstream in OpenSKP.
    pub warnings: Vec<String>,
}

/// Parse SketchUp 2017 `.skp` bytes into a kernel `ImportScene` recipe.
///
/// I/O-free; textures are embedded in the format, so unlike `dae-import` no
/// host-resolved image map is needed. Per-mesh geometry issues are NOT errors
/// here — meshes are emitted as-is and `Document::ingest` skips and reports
/// any it rejects (rule 4: reported, never repaired).
pub fn import(bytes: &[u8]) -> Result<SkpScene, SkpError> {
    // Version gate: identify ANY SketchUp file (works on post-2017 ZIP
    // containers too), then accept only the fully-decoded 2017 layout.
    let (version, _guid) = openskp::header_info(bytes).ok_or(SkpError::NotSkp)?;
    if version_major(&version) != Some(17) {
        return Err(SkpError::UnsupportedVersion { version });
    }

    let model = openskp::Model::parse(bytes).map_err(|e| SkpError::Parse(e.to_string()))?;
    let out = convert::convert(&model);
    Ok(SkpScene {
        scene: out.scene,
        textures_missing: out.textures_missing,
        warnings: out.warnings,
    })
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
pub fn probe_topology(bytes: &[u8]) -> Vec<openskp::Topology> {
    openskp::geometry_runs(bytes)
        .into_iter()
        .map(|run| run.topology)
        .collect()
}
