//! `installed.json`, the record Hew keeps beside each installed plugin, and
//! the checks an install makes against what is already installed.

use crate::InstallError;
use plugin_manifest::{Manifest, PermissionName};
use std::collections::{BTreeMap, BTreeSet};

/// The `installed.json` format this build writes and reads. A record with
/// a higher number is listed as unreadable and never rewritten, so a file
/// a newer Hew wrote survives a trip through an older one.
pub const INSTALLED_SCHEMA: u32 = 1;

/// Ids under this prefix are kept for plugins Hew itself publishes; a file
/// or folder install may not claim one.
pub const RESERVED_ID_PREFIX: &str = "org.hew3d.";

/// Where an installed plugin came from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallSource {
    /// A `.hewplugin` file; `name` is its file name, for display.
    File { name: String },
    /// An unpacked folder, read in place (desktop only).
    Dir { path: String },
    /// Declared for the plugin index; nothing installs from it yet.
    Index { id: String, url: String },
    /// Declared for git sources; nothing installs from it yet.
    Git { url: String, reference: String },
    /// Declared for URL sources; nothing installs from it yet.
    Url { url: String },
}

/// `installed.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Installed {
    /// [`INSTALLED_SCHEMA`] when written by this build. Serialized first.
    pub schema: u32,
    pub id: String,
    pub version: semver::Version,
    pub source: InstallSource,
    pub installed_at_ms: u64,
    /// Permissions the user accepted (the manifest's `permissions` at
    /// consent time).
    pub grant: Vec<PermissionName>,
    /// Optional permissions granted later, at run time.
    #[serde(default)]
    pub optional_granted: Vec<PermissionName>,
    pub enabled: bool,
    /// Whether the plugin may start with Hew (`on_startup`). False at
    /// install; the first time the user turns the plugin on by hand sets
    /// it, so a package cannot start itself just by being installed.
    #[serde(default)]
    pub startup_allowed: bool,
    pub data_version: u32,
    #[serde(default)]
    pub signer: Option<String>,
    /// Fields this build does not know, kept and written back unchanged.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Installed {
    /// Reads an `installed.json`. A record whose `schema` is missing, not a
    /// number, or higher than [`INSTALLED_SCHEMA`] is `installed_unreadable`
    /// without the rest being looked at; so is one that does not parse, or
    /// whose `id` is not `id` (the folder it was found in).
    pub fn from_json(id: &str, bytes: &[u8]) -> Result<Installed, InstallError> {
        let unreadable = |reason: String| InstallError::InstalledUnreadable {
            id: id.to_string(),
            reason,
        };
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| unreadable(e.to_string()))?;
        match value.get("schema").and_then(serde_json::Value::as_u64) {
            Some(s) if s <= u64::from(INSTALLED_SCHEMA) => {}
            Some(s) => return Err(unreadable(format!("record format {s}"))),
            None => return Err(unreadable("no record format".to_string())),
        }
        let installed: Installed =
            serde_json::from_value(value).map_err(|e| unreadable(e.to_string()))?;
        if installed.id != id {
            return Err(unreadable(format!("it names \"{}\"", installed.id)));
        }
        Ok(installed)
    }

    /// The record as written to disk: pretty JSON, `schema` first, unknown
    /// fields after the known ones.
    pub fn to_json(&self) -> Vec<u8> {
        let mut out = serde_json::to_vec_pretty(self).expect("an installed record serializes");
        out.push(b'\n');
        out
    }
}

/// A file or folder install may not claim an id under
/// [`RESERVED_ID_PREFIX`] (`reserved_id`). Index installs, which do not
/// exist yet, are the only way such an id will arrive.
pub fn check_source_id(manifest: &Manifest, source: &InstallSource) -> Result<(), InstallError> {
    let from_user = matches!(
        source,
        InstallSource::File { .. } | InstallSource::Dir { .. }
    );
    if from_user && manifest.id.starts_with(RESERVED_ID_PREFIX) {
        return Err(InstallError::ReservedId(manifest.id.clone()));
    }
    Ok(())
}

/// The install's `requires` against what is installed: every required id
/// must be installed in a version its range admits (`dependency_missing`,
/// listing each as `id range`), and the `requires` graph over the
/// installed plugins plus this one (replacing any installed copy of it)
/// must be acyclic (`dependency_cycle`, listing the cycle from this plugin
/// back to itself).
pub fn check_requires(
    manifest: &Manifest,
    installed: &[(Installed, Manifest)],
) -> Result<(), InstallError> {
    let missing: Vec<String> = manifest
        .requires
        .iter()
        .filter(|r| {
            !installed
                .iter()
                .any(|(i, _)| i.id == r.id && r.range.matches(&i.version))
        })
        .map(|r| format!("{} {}", r.id, r.range))
        .collect();
    if !missing.is_empty() {
        return Err(InstallError::DependencyMissing(missing));
    }
    let mut graph: BTreeMap<&str, Vec<&str>> = installed
        .iter()
        .filter(|(i, _)| i.id != manifest.id)
        .map(|(i, m)| {
            (
                i.id.as_str(),
                m.requires.iter().map(|r| r.id.as_str()).collect(),
            )
        })
        .collect();
    graph.insert(
        manifest.id.as_str(),
        manifest.requires.iter().map(|r| r.id.as_str()).collect(),
    );
    // Every cycle a new edge can close passes through the new plugin, so a
    // depth-first walk from it that comes back to it finds the one to name.
    let start = manifest.id.as_str();
    let mut path = vec![start];
    let mut done = BTreeSet::new();
    if let Some(cycle) = walk(&graph, start, start, &mut path, &mut done) {
        return Err(InstallError::DependencyCycle(cycle));
    }
    Ok(())
}

fn walk<'a>(
    graph: &BTreeMap<&'a str, Vec<&'a str>>,
    start: &'a str,
    at: &'a str,
    path: &mut Vec<&'a str>,
    done: &mut BTreeSet<&'a str>,
) -> Option<Vec<String>> {
    for &next in graph.get(at).map(Vec::as_slice).unwrap_or_default() {
        if next == start {
            let mut cycle: Vec<String> = path.iter().map(|s| s.to_string()).collect();
            cycle.push(start.to_string());
            return Some(cycle);
        }
        if path.contains(&next) || !done.insert(next) {
            continue;
        }
        path.push(next);
        if let Some(cycle) = walk(graph, start, next, path, done) {
            return Some(cycle);
        }
        path.pop();
    }
    None
}
