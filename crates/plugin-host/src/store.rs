//! The on-disk install store (feature `fs`): the desktop shell's and
//! `hew-cli`'s half. The web build keeps the same layout in OPFS.
//!
//! ```text
//! plugins/
//!   <id>/
//!     hew-plugin.json      the manifest the user consented to
//!     installed.json       the record (`Installed`)
//!     storage.json         the plugin's saved data (`storage_*`)
//!     files/…              the package's files (not for a folder install)
//!     previous/            the version before, with the same layout
//!   .staging/<id>-<token>/ an install being written; ignored by `list`
//! ```
//!
//! Every path the store touches is built from a validated plugin id and,
//! for package files, a path that passed [`crate::normalize_path`], so
//! nothing it reads or writes is outside `plugins/<id>/` (or, for a folder
//! install, outside that folder). An install is written whole into
//! `.staging/` and renamed into place; a failure part way leaves a staging
//! entry that `list` ignores and the next `stage` of the same id clears.
//!
//! A folder install (`InstallSource::Dir`) is read in place, and only
//! while developer mode is on (the host says which, [`DevMode`]). The
//! folder's canonical path, recorded when the person picked it, is the only
//! root: every read checks it again ([`read_jailed`]). When the folder's
//! manifest changes, [`apply_folder_manifest`] either refreshes the stored
//! copy (same permissions) or marks the record `needs_consent`, after
//! which nothing is read from the folder until the person reviews the
//! change ([`stage_review`], [`commit_review`]).
//!
//! A plugin's saved data belongs to the plugin, not to one version of it:
//! an update and a rollback carry the installed version's `storage.json`
//! to the version that takes its place, and uninstalling removes it with
//! the rest of the folder.

use crate::installed::{INSTALLED_SCHEMA, InstallSource, Installed};
use crate::package::{MANIFEST_FILE, MAX_ENTRY_BYTES, Package};
use crate::{ConsentPayload, InstallError};
use plugin_manifest::{Manifest, PermissionName};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The app's identifier, the folder name its data lives under.
const APP_IDENTIFIER: &str = "com.hew3d.Hew";
/// The store's folder name inside the app's data folder.
const STORE_FOLDER: &str = "plugins";
/// Where installs are written before they are renamed into place.
const STAGING: &str = ".staging";
const INSTALLED_FILE: &str = "installed.json";
/// The plugin's saved data, a JSON object of string keys to string values.
const STORAGE_FILE: &str = "storage.json";
const FILES: &str = "files";
const PREVIOUS: &str = "previous";
/// How long a pending install waits for an answer.
pub const PENDING_TTL_MS: u64 = 10 * 60 * 1000;
/// The environment variable that moves the store (tests, and a developer
/// keeping a second store). It exists only in a preview build, with the
/// rest of this crate.
pub const PLUGIN_DIR_ENV: &str = "HEW_PLUGIN_DIR";

/// The record field that marks a folder plugin whose manifest changed in a
/// way the person has not reviewed. Kept in [`Installed::extra`], so it
/// needs no new record format.
pub const NEEDS_CONSENT: &str = "needs_consent";
/// The record field that says why [`NEEDS_CONSENT`] was set, one of
/// [`FOLDER_CHANGE_REASONS`].
pub const NEEDS_CONSENT_REASON: &str = "needs_consent_reason";
/// Why a folder plugin waits for review: its permissions or optional
/// permissions differ; it newly asks to start with Hew; its manifest no
/// longer reads; it names another plugin; it now needs a Hew version or
/// other plugins that are not here; its name or the reason it gives for
/// starting with Hew changed; what its commands (or its other
/// contributions) may do changed; or one of its commands now has the name
/// of another plugin's.
pub const FOLDER_CHANGE_REASONS: &[&str] = &[
    "permissions_changed",
    "startup_added",
    "manifest_unreadable",
    "id_changed",
    "requirements_unmet",
    "consent_text_changed",
    "authority_changed",
    "command_name_taken",
];

/// Whether the host lets plugin code be read from a folder. Off, every
/// read of a folder install is refused `developer_mode_off`; installs from
/// a package are read either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevMode {
    Off,
    On,
}

/// Whether a record carries the review marker. Anything there but an
/// explicit `false` counts as marked, so a damaged record stays blocked.
pub fn needs_consent(installed: &Installed) -> bool {
    installed
        .extra
        .get(NEEDS_CONSENT)
        .is_some_and(|v| v != &serde_json::Value::Bool(false))
}

/// The marker's reason, when it is one this build writes.
pub fn needs_consent_reason(installed: &Installed) -> Option<&'static str> {
    let reason = installed.extra.get(NEEDS_CONSENT_REASON)?.as_str()?;
    FOLDER_CHANGE_REASONS.iter().copied().find(|r| *r == reason)
}

fn clear_marker(extra: &mut BTreeMap<String, serde_json::Value>) {
    extra.remove(NEEDS_CONSENT);
    extra.remove(NEEDS_CONSENT_REASON);
}

/// The store's root folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDir {
    root: PathBuf,
}

impl PluginDir {
    /// `HEW_PLUGIN_DIR` when set to an absolute path, else
    /// `app_data_dir()/plugins`. `None` when neither resolves.
    pub fn resolve() -> Option<PluginDir> {
        if let Some(dir) = std::env::var_os(PLUGIN_DIR_ENV) {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                return Some(PluginDir::at(dir));
            }
        }
        app_data_dir().map(|d| PluginDir::at(d.join(STORE_FOLDER)))
    }

    pub fn at(root: PathBuf) -> PluginDir {
        PluginDir { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `plugins/<id>`, for an id that passed the plugin id grammar.
    fn plugin(&self, id: &str) -> Result<PathBuf, InstallError> {
        checked_id(id)?;
        Ok(self.root.join(id))
    }

    fn staging(&self) -> PathBuf {
        self.root.join(STAGING)
    }
}

/// Tauri's `app_data_dir()` for `com.hew3d.Hew`, without a Tauri
/// dependency: macOS `~/Library/Application Support/com.hew3d.Hew`;
/// Windows `%APPDATA%\com.hew3d.Hew`; elsewhere `$XDG_DATA_HOME/com.hew3d.Hew`
/// or `~/.local/share/com.hew3d.Hew`.
pub fn app_data_dir() -> Option<PathBuf> {
    let absolute = |v: std::ffi::OsString| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .and_then(absolute)
            .map(|h| h.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").and_then(absolute)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .and_then(absolute)
            .or_else(|| {
                std::env::var_os("HOME")
                    .and_then(absolute)
                    .map(|h| h.join(".local").join("share"))
            })
    };
    base.map(|b| b.join(APP_IDENTIFIER))
}

/// An install waiting for the user's answer. The package's bytes stay
/// here; only the token and the consent payload go to the page.
#[derive(Debug, Clone)]
pub struct Pending {
    pub token: String,
    pub package: Package,
    pub source: InstallSource,
    pub consent: ConsentPayload,
    pub previous: Option<Installed>,
    pub created_at_ms: u64,
}

/// One row of [`list`]. Rows are few and short-lived, so the size gap
/// between the two variants costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Listed {
    Installed {
        installed: Installed,
        manifest: Manifest,
        /// The version `rollback` would go back to, when there is one.
        previous_version: Option<String>,
    },
    /// A folder named like a plugin whose record or manifest cannot be
    /// read: a newer Hew's record, say. Shown so it can be uninstalled;
    /// never rewritten.
    Unreadable { id: String, reason: String },
}

/// What the plugin index, git and URL sources will fetch through. Declared
/// for those sources; nothing implements it yet.
pub trait Fetcher {
    fn fetch_index(&self, url: &str) -> Result<Vec<u8>, String>;
    fn fetch_git(&self, url: &str, reference: &str, into: &Path) -> Result<(), String>;
    fn fetch_url(&self, url: &str) -> Result<Vec<u8>, String>;
}

fn io(e: std::io::Error) -> InstallError {
    InstallError::Io(e.to_string())
}

fn checked_id(id: &str) -> Result<(), InstallError> {
    plugin_manifest::validate_plugin_id(id).map_err(InstallError::Manifest)
}

/// A random token for a pending install: 128 bits from the standard
/// library's per-process random hash keys, as 32 lowercase hex digits.
fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    (0..2u64)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(nanos);
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// Whether `name` (an entry of `.staging/`) belongs to plugin `id`:
/// `<id>-<token>` exactly, with an optional `.<suffix>` the store adds for
/// its own scratch folders. `a.b-c-<token>` is not `a.b`'s.
fn staging_of(name: &str, id: &str) -> bool {
    let Some(rest) = name.strip_prefix(id).and_then(|r| r.strip_prefix('-')) else {
        return false;
    };
    let token = rest.split('.').next().unwrap_or(rest);
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Removes the staging entries of `id`. With `keep_stranded`, an entry
/// that may hold the only copy of an installed version is kept: one that
/// holds a `previous/` while `plugins/<id>/` is missing (an install or
/// rollback stopped after moving the current version into it), or a
/// `.old` entry while `plugins/<id>/previous/` is missing (the version
/// before, moved aside). [`purge`] removes everything.
fn clear_staging(dir: &PluginDir, id: &str, keep_stranded: bool) -> Result<(), InstallError> {
    let staging = dir.staging();
    let Ok(entries) = std::fs::read_dir(&staging) else {
        return Ok(());
    };
    let home = dir.root().join(id);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !staging_of(name, id) {
            continue;
        }
        let path = entry.path();
        let stranded = (exists(&path.join(PREVIOUS)) && !exists(&home))
            || (name.ends_with(".old") && !exists(&home.join(PREVIOUS)));
        if keep_stranded && stranded {
            continue;
        }
        remove_any(&path)?;
    }
    Ok(())
}

/// Removes a file, a folder tree, or a link (never following it).
fn remove_any(path: &Path) -> Result<(), InstallError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).map_err(io),
        Ok(_) => std::fs::remove_file(path).map_err(io),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}

/// Writes `bytes` to `path` through a temporary file in the same folder
/// and a rename, so a reader never sees half a record.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), InstallError> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// Reads a regular file of at most `cap` bytes, measured on the bytes
/// read; a link or anything else is refused.
fn read_capped(path: &Path, cap: usize) -> Result<Vec<u8>, InstallError> {
    let meta = std::fs::symlink_metadata(path).map_err(io)?;
    if !meta.file_type().is_file() {
        return Err(InstallError::PackagePathInvalid(path.display().to_string()));
    }
    let mut data = Vec::new();
    std::fs::File::open(path)
        .map_err(io)?
        .take(cap as u64 + 1)
        .read_to_end(&mut data)
        .map_err(io)?;
    if data.len() > cap {
        return Err(InstallError::PackageTooLarge);
    }
    Ok(data)
}

/// A folder plugin's recorded root, checked again: it must be an absolute
/// path that still exists as a folder (`folder_missing` otherwise), is not
/// itself a symbolic link, and canonicalizes to exactly itself, so no
/// folder on the way to it has become a link either
/// (`package_path_invalid`). Answers the root to read under.
pub fn checked_root(recorded: &Path) -> Result<PathBuf, InstallError> {
    let shown = || recorded.display().to_string();
    if !recorded.is_absolute() {
        return Err(InstallError::PackagePathInvalid(shown()));
    }
    let meta =
        std::fs::symlink_metadata(recorded).map_err(|_| InstallError::FolderMissing(shown()))?;
    if meta.file_type().is_symlink() {
        return Err(InstallError::PackagePathInvalid(shown()));
    }
    if !meta.is_dir() {
        return Err(InstallError::FolderMissing(shown()));
    }
    let canonical =
        std::fs::canonicalize(recorded).map_err(|_| InstallError::FolderMissing(shown()))?;
    if canonical != recorded {
        return Err(InstallError::PackagePathInvalid(shown()));
    }
    Ok(canonical)
}

/// The root of a folder the person just picked: not a link itself, a
/// folder, and then canonical.
fn picked_root(path: &Path) -> Result<PathBuf, InstallError> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|_| InstallError::FolderMissing(path.display().to_string()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(InstallError::PackagePathInvalid(path.display().to_string()));
    }
    std::fs::canonicalize(path).map_err(|_| InstallError::FolderMissing(path.display().to_string()))
}

/// Whether `rel` is a regular file under the folder `recorded`, by names
/// alone: the root passes [`checked_root`], every segment exists, none is a
/// symbolic link, folders on the way are folders and the last is a regular
/// file. Nothing is opened or read, so it is cheap enough for a list.
pub fn jailed_file_exists(recorded: &Path, rel: &str) -> bool {
    let Ok(root) = checked_root(recorded) else {
        return false;
    };
    let Ok(rel) = crate::normalize_path(rel) else {
        return false;
    };
    let segments: Vec<&str> = rel.split('/').collect();
    let mut at = root;
    for (n, segment) in segments.iter().enumerate() {
        at.push(segment);
        let Ok(meta) = std::fs::symlink_metadata(&at) else {
            return false;
        };
        let kind = meta.file_type();
        let last = n + 1 == segments.len();
        if kind.is_symlink() || (!last && !kind.is_dir()) || (last && !kind.is_file()) {
            return false;
        }
    }
    true
}

/// The one reader of a plugin's files: `rel` under `root`, a canonical
/// folder path ([`checked_root`]). `rel` passes [`crate::normalize_path`];
/// every segment under the root must exist and none may be a symbolic link
/// (folders on the way, a regular file at the end). The file is opened
/// once, and what was opened must be a regular file that canonicalizes to
/// a path inside the root; on Unix it must also be the very file the
/// checks saw and the file the canonical path names now (device and
/// inode), so a segment swapped for a link between the checks and the open
/// is refused rather than followed. Read to at most [`MAX_ENTRY_BYTES`]
/// (`package_too_large`). Anything else is `package_path_invalid`.
pub fn read_jailed(root: &Path, rel: &str) -> Result<Vec<u8>, InstallError> {
    let rel = crate::normalize_path(rel)?;
    let invalid = || InstallError::PackagePathInvalid(rel.clone());
    let segments: Vec<&str> = rel.split('/').collect();
    let mut at = root.to_path_buf();
    let mut checked = None;
    for (n, segment) in segments.iter().enumerate() {
        at.push(segment);
        let meta = std::fs::symlink_metadata(&at).map_err(|_| invalid())?;
        let kind = meta.file_type();
        let last = n + 1 == segments.len();
        if kind.is_symlink() || (!last && !kind.is_dir()) || (last && !kind.is_file()) {
            return Err(invalid());
        }
        if last {
            checked = Some(meta);
        }
    }
    let checked = checked.ok_or_else(invalid)?;
    let file = std::fs::File::open(&at).map_err(|_| invalid())?;
    let opened = file.metadata().map_err(|_| invalid())?;
    if !opened.is_file() {
        return Err(invalid());
    }
    let canonical = std::fs::canonicalize(&at).map_err(|_| invalid())?;
    if canonical == root || !canonical.starts_with(root) {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let named = std::fs::metadata(&canonical).map_err(|_| invalid())?;
        let same =
            |a: &std::fs::Metadata, b: &std::fs::Metadata| a.dev() == b.dev() && a.ino() == b.ino();
        if !same(&opened, &checked) || !same(&opened, &named) {
            return Err(invalid());
        }
    }
    #[cfg(not(unix))]
    let _ = checked;
    let mut data = Vec::new();
    file.take(MAX_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| invalid())?;
    if data.len() > MAX_ENTRY_BYTES {
        return Err(InstallError::PackageTooLarge);
    }
    Ok(data)
}

/// Reads an unpacked plugin folder under the same rules as a `.hewplugin`:
/// every file and folder name passes [`crate::normalize_path`], names that
/// collide under case folding are refused, a symbolic link (or any entry
/// that is not a regular file or folder) is `package_path_invalid`, and the
/// entry and size caps are measured on the bytes read. The folder itself
/// may not be a link. Every file is read through [`read_jailed`]. Then
/// [`crate::package_from_files`].
pub fn read_package_dir(path: &Path) -> Result<Package, InstallError> {
    read_folder(&picked_root(path)?)
}

/// [`read_package_dir`] under a canonical root.
fn read_folder(root: &Path) -> Result<Package, InstallError> {
    let mut files = BTreeMap::new();
    let mut folded = std::collections::BTreeSet::new();
    let mut entries = 0usize;
    let mut total = 0usize;
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        // The folder being listed is still the one under the root, not a
        // link something put in its place since it was looked at.
        if std::fs::canonicalize(&dir).ok().as_deref() != Some(dir.as_path()) {
            return Err(InstallError::PackagePathInvalid(prefix));
        }
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            entries += 1;
            if entries > crate::MAX_PACKAGE_ENTRIES {
                return Err(InstallError::PackageTooLarge);
            }
            let name = child.file_name();
            let Some(name) = name.to_str() else {
                return Err(InstallError::PackagePathInvalid(
                    name.to_string_lossy().into_owned(),
                ));
            };
            let rel = crate::normalize_path(&format!("{prefix}{name}"))?;
            if !folded.insert(rel.to_lowercase()) {
                return Err(InstallError::PackagePathInvalid(rel));
            }
            let kind = std::fs::symlink_metadata(child.path())
                .map_err(io)?
                .file_type();
            if kind.is_dir() {
                pending.push((child.path(), format!("{rel}/")));
            } else if kind.is_file() {
                let data = read_jailed(root, &rel)?;
                total += data.len();
                if total > crate::MAX_PACKAGE_BYTES {
                    return Err(InstallError::PackageTooLarge);
                }
                files.insert(rel, data);
            } else {
                return Err(InstallError::PackagePathInvalid(rel));
            }
        }
    }
    crate::package_from_files(files)
}

/// One entry of a [`FolderSnapshot`]: its kind (`d` folder, `f` file, `l`
/// link, `o` anything else), size, and modification time.
pub type SnapshotEntry = (char, u64, Option<std::time::SystemTime>);

/// What a folder holds, by relative path, as far as a watcher compares it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderSnapshot(pub BTreeMap<String, SnapshotEntry>);

/// Lists a folder plugin's recorded folder for a watcher: the root is
/// checked as for a read ([`checked_root`]), no link is followed (a link is
/// listed as an entry and not entered), a folder that is no longer the
/// same path canonically stops the walk (`package_path_invalid`), and the
/// walk stops at the package caps: more than
/// [`crate::MAX_PACKAGE_ENTRIES`] entries, a file over
/// [`MAX_ENTRY_BYTES`], or more than [`crate::MAX_PACKAGE_BYTES`] in all is
/// `package_too_large`. An entry whose name is not valid UTF-8 is
/// `package_path_invalid`, as it is for the package reader. Only names and
/// metadata are read, never contents. `keep_going` is asked before each
/// entry; when it answers false the walk stops with `None`, so a caller
/// that was told to stop is not held by a long walk.
pub fn folder_snapshot(recorded: &Path) -> Result<FolderSnapshot, InstallError> {
    folder_snapshot_until(recorded, &|| true).map(|s| s.unwrap_or_default())
}

/// [`folder_snapshot`], asking `keep_going` before each entry.
pub fn folder_snapshot_until(
    recorded: &Path,
    keep_going: &dyn Fn() -> bool,
) -> Result<Option<FolderSnapshot>, InstallError> {
    let root = checked_root(recorded)?;
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    let mut pending = vec![(root.clone(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        if std::fs::canonicalize(&dir).ok().as_deref() != Some(dir.as_path()) {
            return Err(InstallError::PackagePathInvalid(prefix));
        }
        let listing = std::fs::read_dir(&dir).map_err(|_| {
            if dir == root {
                InstallError::FolderMissing(recorded.display().to_string())
            } else {
                InstallError::PackagePathInvalid(prefix.clone())
            }
        })?;
        for child in listing {
            if !keep_going() {
                return Ok(None);
            }
            let Ok(child) = child else { continue };
            if out.len() >= crate::MAX_PACKAGE_ENTRIES {
                return Err(InstallError::PackageTooLarge);
            }
            // A name that is not text is refused, as the package reader
            // refuses it: two such names must not read as one key.
            let name = child.file_name();
            let Some(name) = name.to_str() else {
                return Err(InstallError::PackagePathInvalid(format!(
                    "{prefix}{}",
                    name.to_string_lossy()
                )));
            };
            let rel = format!("{prefix}{name}");
            // Gone between the listing and now: the next walk sees it.
            let Ok(meta) = std::fs::symlink_metadata(child.path()) else {
                continue;
            };
            let kind = meta.file_type();
            let tag = if kind.is_symlink() {
                'l'
            } else if kind.is_dir() {
                'd'
            } else if kind.is_file() {
                'f'
            } else {
                'o'
            };
            if tag == 'f' {
                if meta.len() > MAX_ENTRY_BYTES as u64 {
                    return Err(InstallError::PackageTooLarge);
                }
                total = total.saturating_add(meta.len());
                if total > crate::MAX_PACKAGE_BYTES as u64 {
                    return Err(InstallError::PackageTooLarge);
                }
            }
            if tag == 'd' {
                pending.push((child.path(), format!("{rel}/")));
            }
            let size = if tag == 'd' { 0 } else { meta.len() };
            out.insert(rel, (tag, size, meta.modified().ok()));
        }
    }
    Ok(Some(FolderSnapshot(out)))
}

/// Every installed plugin as `(record, manifest)`, sorted by id; plugins
/// that cannot be read are left out (see [`list`] for all of them).
fn readable(dir: &PluginDir) -> Vec<(Installed, Manifest)> {
    list(dir)
        .into_iter()
        .filter_map(|l| match l {
            Listed::Installed {
                installed,
                manifest,
                ..
            } => Some((installed, manifest)),
            Listed::Unreadable { .. } => None,
        })
        .collect()
}

/// Checks `package` for installing from `source` and holds it for the
/// user's answer: a file or folder install may not claim a reserved id;
/// `manifest.hew` must admit `running`; `requires` must be installed and
/// acyclic; no command may have another installed plugin's command name
/// (`command_name_taken`, the rule a folder's refresh follows too); an
/// installed plugin of the same id must be readable (a record
/// a newer Hew wrote is never replaced). Clears any staging entry an
/// interrupted install of the same id left. The consent payload compares
/// against the installed version, if any.
pub fn stage(
    dir: &PluginDir,
    package: Package,
    source: InstallSource,
    running: &semver::Version,
    now_ms: u64,
) -> Result<Pending, InstallError> {
    let manifest = &package.manifest;
    checked_id(&manifest.id)?;
    crate::check_source_id(manifest, &source)?;
    if !crate::hew_version_ok(manifest, running) {
        return Err(InstallError::HewVersionUnsupported {
            required: manifest
                .hew
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            running: running.to_string(),
        });
    }
    crate::check_requires(manifest, &readable(dir))?;
    check_command_names(dir, manifest)?;
    let previous = match installed(dir, &manifest.id) {
        Ok(found) => Some(found),
        Err(InstallError::NotInstalled(_)) => None,
        Err(e) => return Err(e),
    };
    clear_staging(dir, &manifest.id, true)?;
    let consent = crate::consent(
        manifest,
        &source,
        previous.as_ref().map(|(i, _)| i),
        previous.as_ref().map(|(_, m)| m),
        None,
    );
    Ok(Pending {
        token: new_token(),
        package,
        source,
        consent,
        previous: previous.map(|(i, _)| i),
        created_at_ms: now_ms,
    })
}

/// The record a fresh install, or an update over `previous`, writes. A
/// fresh install is enabled, with the manifest's permissions as its grant,
/// no optional grants, and startup not yet allowed. An update keeps the
/// installed `enabled`, `data_version` (its data has not been migrated
/// yet) and the optional grants the new manifest still declares; it keeps
/// `startup_allowed` unless the update newly asks to start with Hew.
fn record_for(pending: &Pending, now_ms: u64) -> Installed {
    let manifest = &pending.package.manifest;
    let startup_newly = pending
        .consent
        .update
        .as_ref()
        .is_some_and(|u| u.startup_newly);
    let previous = pending.previous.as_ref();
    Installed {
        schema: INSTALLED_SCHEMA,
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        source: pending.source.clone(),
        installed_at_ms: now_ms,
        grant: manifest.permissions.clone(),
        optional_granted: previous
            .map(|p| {
                p.optional_granted
                    .iter()
                    .filter(|o| manifest.optional_permissions.contains(o))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        enabled: previous.is_none_or(|p| p.enabled),
        startup_allowed: previous.is_some_and(|p| p.startup_allowed) && !startup_newly,
        data_version: previous.map_or(manifest.data.version, |p| p.data_version),
        signer: None,
        // A fresh consent to the whole manifest answers any review that
        // was waiting.
        extra: previous
            .map(|p| {
                let mut extra = p.extra.clone();
                clear_marker(&mut extra);
                extra
            })
            .unwrap_or_default(),
    }
}

/// `accept: false` drops the pending install and writes nothing.
/// `accept: true` refuses `pending_expired` after [`PENDING_TTL_MS`],
/// refuses `pending_stale` when the installed record of the id is not the
/// one `stage` saw (`pending.previous`, none included), checks `requires`
/// again against what is installed now
/// (`dependency_missing`, `dependency_cycle`), then writes `plugins/.staging/<id>-<token>/{hew-plugin.json, installed.json,
/// files/…}`, moves an existing `plugins/<id>/` to the new install's
/// `previous/` (dropping any older `previous/`), and renames the staging
/// folder into place. A folder (`Dir`) install stores no `files/`: it is
/// read from the folder.
pub fn commit(
    dir: &PluginDir,
    pending: Pending,
    accept: bool,
    now_ms: u64,
) -> Result<Option<Installed>, InstallError> {
    commit_with(dir, pending, accept, now_ms, &mut |from, to| {
        std::fs::rename(from, to)
    })
}

/// How the store moves folders: `std::fs::rename`, or a stand-in that
/// fails on purpose in the tests below.
type Rename<'a> = &'a mut dyn FnMut(&Path, &Path) -> std::io::Result<()>;

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn commit_with(
    dir: &PluginDir,
    pending: Pending,
    accept: bool,
    now_ms: u64,
    rename: Rename<'_>,
) -> Result<Option<Installed>, InstallError> {
    if !accept {
        return Ok(None);
    }
    if now_ms.saturating_sub(pending.created_at_ms) > PENDING_TTL_MS {
        return Err(InstallError::PendingExpired);
    }
    let id = pending.package.manifest.id.clone();
    let current = dir.plugin(&id)?;
    if !staging_of(&format!("{id}-{}", pending.token), &id) {
        return Err(InstallError::PendingUnknown);
    }
    // The consent compared against what was installed at `stage`; if that
    // changed (installed, removed, updated, rewritten), what the user
    // agreed to is no longer what this would do.
    let now_installed = match installed(dir, &id) {
        Ok((record, _)) => Some(record),
        Err(InstallError::NotInstalled(_)) => None,
        Err(e) => return Err(e),
    };
    if now_installed != pending.previous {
        return Err(InstallError::PendingStale);
    }
    // What else is installed may have changed too: a dependency
    // uninstalled, or one that now requires this plugin.
    crate::check_requires(&pending.package.manifest, &readable(dir))?;
    check_command_names(dir, &pending.package.manifest)?;
    let staging = dir.staging();
    std::fs::create_dir_all(&staging).map_err(io)?;
    let fresh = staging.join(format!("{id}-{}", pending.token));
    let aside = staging.join(format!("{id}-{}.old", pending.token));
    remove_any(&fresh)?;
    remove_any(&aside)?;
    std::fs::create_dir(&fresh).map_err(io)?;

    let record = record_for(&pending, now_ms);
    let written = write_install(&fresh, &pending, &record)
        .and_then(|()| carry_storage(&current, &fresh))
        .and_then(|()| commit_moves(&current, &fresh, &aside, rename));
    match written {
        Ok(discard) => {
            if discard {
                let _ = remove_any(&aside);
            }
            Ok(Some(record))
        }
        Err(e) => {
            // The moves were undone, so staging holds only the new copy;
            // unless an undo failed too, and the entry holds an installed
            // version, which stays for `clear_staging` to keep.
            if !exists(&fresh.join(PREVIOUS)) {
                let _ = remove_any(&fresh);
            }
            Err(e)
        }
    }
}

/// Writes the new install into its staging folder.
fn write_install(fresh: &Path, pending: &Pending, record: &Installed) -> Result<(), InstallError> {
    let manifest_bytes = pending
        .package
        .files
        .get(MANIFEST_FILE)
        .ok_or(InstallError::ManifestMissing)?;
    std::fs::write(fresh.join(MANIFEST_FILE), manifest_bytes).map_err(io)?;
    std::fs::write(fresh.join(INSTALLED_FILE), record.to_json()).map_err(io)?;
    if !matches!(pending.source, InstallSource::Dir { .. }) {
        let files = fresh.join(FILES);
        for (rel, bytes) in &pending.package.files {
            let rel = crate::normalize_path(rel)?;
            let target = rel.split('/').fold(files.clone(), |p, s| p.join(s));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            std::fs::write(&target, bytes).map_err(io)?;
        }
    }
    Ok(())
}

/// Moves a written install into place: the installed version's own
/// `previous/` aside to `aside`, the installed version into the new one's
/// `previous/`, the new one to `current`. A failed move undoes the ones
/// before it, best effort and in reverse order, so no version is left only
/// in staging. Answers whether `aside` holds an old `previous/` to remove.
fn commit_moves(
    current: &Path,
    fresh: &Path,
    aside: &Path,
    rename: Rename<'_>,
) -> Result<bool, InstallError> {
    let had_current = exists(current);
    let mut moved_previous = false;
    if had_current {
        if exists(&current.join(PREVIOUS)) {
            rename(&current.join(PREVIOUS), aside).map_err(io)?;
            moved_previous = true;
        }
        if let Err(e) = rename(current, &fresh.join(PREVIOUS)) {
            if moved_previous {
                let _ = rename(aside, &current.join(PREVIOUS));
            }
            return Err(io(e));
        }
    }
    if let Err(e) = rename(fresh, current) {
        if had_current {
            let _ = rename(&fresh.join(PREVIOUS), current);
            if moved_previous {
                let _ = rename(aside, &current.join(PREVIOUS));
            }
        }
        return Err(io(e));
    }
    Ok(moved_previous)
}

/// Reads one installed plugin's record and manifest from `folder`.
fn read_installed(folder: &Path, id: &str) -> Result<(Installed, Manifest), InstallError> {
    let record = match read_capped(&folder.join(INSTALLED_FILE), MAX_ENTRY_BYTES) {
        Ok(bytes) => bytes,
        Err(InstallError::Io(_)) if std::fs::symlink_metadata(folder).is_err() => {
            return Err(InstallError::NotInstalled(id.to_string()));
        }
        Err(e) => {
            return Err(InstallError::InstalledUnreadable {
                id: id.to_string(),
                reason: e.explanation(),
            });
        }
    };
    let installed = Installed::from_json(id, &record)?;
    let unreadable = |reason: String| InstallError::InstalledUnreadable {
        id: id.to_string(),
        reason,
    };
    let text = read_capped(&folder.join(MANIFEST_FILE), MAX_ENTRY_BYTES)
        .map_err(|e| unreadable(e.explanation()))?;
    let text =
        String::from_utf8(text).map_err(|_| unreadable("the manifest is not text".into()))?;
    let manifest = plugin_manifest::parse(&text).map_err(|e| unreadable(e.explanation()))?;
    if manifest.id != id {
        return Err(unreadable(format!(
            "its manifest names \"{}\"",
            manifest.id
        )));
    }
    Ok((installed, manifest))
}

/// Every plugin folder in the store, sorted by id. Entries whose name is
/// not a plugin id (`.staging/`, anything else) are ignored; a plugin
/// whose record or manifest cannot be read is listed as
/// [`Listed::Unreadable`].
pub fn list(dir: &PluginDir) -> Vec<Listed> {
    let Ok(entries) = std::fs::read_dir(dir.root()) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| plugin_manifest::validate_plugin_id(n).is_ok())
        .collect();
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let folder = dir.root().join(&id);
            match read_installed(&folder, &id) {
                Ok((installed, manifest)) => Listed::Installed {
                    previous_version: read_installed(&folder.join(PREVIOUS), &id)
                        .ok()
                        .map(|(p, _)| p.version.to_string()),
                    installed,
                    manifest,
                },
                Err(e) => Listed::Unreadable {
                    reason: match e {
                        InstallError::InstalledUnreadable { reason, .. } => reason,
                        other => other.explanation(),
                    },
                    id,
                },
            }
        })
        .collect()
}

/// One installed plugin's record and manifest: `not_installed` when there
/// is no such folder, `installed_unreadable` when it cannot be read.
pub fn installed(dir: &PluginDir, id: &str) -> Result<(Installed, Manifest), InstallError> {
    let folder = dir.plugin(id)?;
    if std::fs::symlink_metadata(&folder).is_err() {
        return Err(InstallError::NotInstalled(id.to_string()));
    }
    read_installed(&folder, id)
}

/// The record and manifest held in `previous/`: what [`rollback`] would
/// bring back. `not_installed` when the plugin is not installed,
/// `no_previous_version` when it has no `previous/`, and
/// `installed_unreadable` when that record cannot be read (a newer
/// schema, say).
pub fn previous(dir: &PluginDir, id: &str) -> Result<(Installed, Manifest), InstallError> {
    let folder = dir.plugin(id)?;
    if std::fs::symlink_metadata(&folder).is_err() {
        return Err(InstallError::NotInstalled(id.to_string()));
    }
    read_installed(&folder.join(PREVIOUS), id).map_err(|e| match e {
        InstallError::NotInstalled(_) => InstallError::NoPreviousVersion(id.to_string()),
        other => other,
    })
}

/// Where an installed plugin's files are read from: its store copy
/// (`plugins/<id>/files`), or the folder recorded at install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSource {
    root: PathBuf,
    folder: bool,
}

impl FileSource {
    /// Whether this is a folder install's folder.
    pub fn is_folder(&self) -> bool {
        self.folder
    }

    /// The recorded root (not yet checked again).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// One file, through [`read_jailed`]. A folder's root is checked again
    /// first ([`checked_root`]); the store's own copy may sit under a
    /// linked folder (a moved data folder), so it is canonicalized, and only
    /// `files/` itself may not be a link.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>, InstallError> {
        let root = if self.folder {
            checked_root(&self.root)?
        } else {
            let invalid = || InstallError::PackagePathInvalid(rel.to_string());
            let meta = std::fs::symlink_metadata(&self.root).map_err(|_| invalid())?;
            if !meta.is_dir() {
                return Err(invalid());
            }
            std::fs::canonicalize(&self.root).map_err(|_| invalid())?
        };
        read_jailed(&root, rel)
    }
}

/// The `hew-plugin.json` of the folder recorded at `recorded`, read under
/// the same rules as every read of a folder plugin ([`checked_root`], then
/// [`read_jailed`]). For the shell's comparison of a folder's manifest
/// with the stored one, which may need to read it before the record that
/// names the folder is in place (a rollback, say).
pub fn folder_manifest(recorded: &Path) -> Result<Vec<u8>, InstallError> {
    read_jailed(&checked_root(recorded)?, MANIFEST_FILE)
}

/// Where plugin `id`'s files are read from, decided from its record alone
/// (no folder is touched): a folder install is refused
/// `developer_mode_off` while developer mode is off, and
/// `dev_consent_pending` while its record is marked [`NEEDS_CONSENT`].
pub fn file_source(dir: &PluginDir, id: &str, dev: DevMode) -> Result<FileSource, InstallError> {
    let (installed, _) = installed(dir, id)?;
    match &installed.source {
        InstallSource::Dir { path } => {
            if dev == DevMode::Off {
                return Err(InstallError::DeveloperModeOff);
            }
            if needs_consent(&installed) {
                return Err(InstallError::DevConsentPending(id.to_string()));
            }
            Ok(FileSource {
                root: PathBuf::from(path),
                folder: true,
            })
        }
        _ => Ok(FileSource {
            root: dir.plugin(id)?.join(FILES),
            folder: false,
        }),
    }
}

/// A package file by served path, jailed: `rel` is checked exactly as
/// package paths are, then read through [`file_source`] and
/// [`FileSource::read`]: from `files/<rel>` under the plugin's folder, or
/// `<folder>/<rel>` for a folder install. No part of the path may be a
/// symbolic link, the file must be a regular file inside that root, and it
/// is read to at most [`MAX_ENTRY_BYTES`]. A bad path is
/// `package_path_invalid`; a folder install's folder that is gone is
/// `folder_missing`.
pub fn read_file(
    dir: &PluginDir,
    id: &str,
    rel: &str,
    dev: DevMode,
) -> Result<Vec<u8>, InstallError> {
    crate::normalize_path(rel)?;
    file_source(dir, id, dev)?.read(rel)
}

/// What [`apply_folder_manifest`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderOutcome {
    /// Nothing in the store changed: the manifest is the stored one (only
    /// code or other files changed), or the record was already marked for
    /// the reason the folder still gives.
    Unchanged,
    /// The manifest changed only in fields that need no review; the stored
    /// copy (and the record's version) now follow it. The grant is kept.
    Refreshed,
    /// The record is now marked [`NEEDS_CONSENT`], for this reason (one of
    /// [`FOLDER_CHANGE_REASONS`]). Nothing else in the store changed.
    Marked(&'static str),
    /// The record was marked, and the folder's manifest no longer differs
    /// from the stored one in anything that needs review (the person fixed
    /// the typo, or undid the change): the marker is gone, and the stored
    /// copy follows the folder in the fields that need no review.
    Cleared,
}

fn sorted(names: &[PermissionName]) -> std::collections::BTreeSet<&str> {
    names.iter().map(PermissionName::as_str).collect()
}

/// The permission a caller needs to run a declared command, as the
/// registry decides it: the declaration's own, or its class's default.
fn command_permission(c: &plugin_manifest::CommandContribution) -> &str {
    match (&c.permission, c.class) {
        (Some(p), _) => p.as_str(),
        (None, plugin_manifest::CommandContributionClass::Mutating) => "model.write",
        (None, plugin_manifest::CommandContributionClass::ReadOnly) => "model.read",
    }
}

/// Whether what the folder's commands may do changed: a command kept under
/// the same id now has another class or needs another permission, or an
/// added command carries a permission other than its class's default.
fn command_authority_changed(new: &Manifest, stored: &Manifest) -> bool {
    let before: BTreeMap<&str, &plugin_manifest::CommandContribution> = stored
        .contributes
        .commands
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    new.contributes
        .commands
        .iter()
        .any(|c| match before.get(c.id.as_str()) {
            Some(old) => old.class != c.class || command_permission(old) != command_permission(c),
            None => {
                let mut default = c.clone();
                default.permission = None;
                command_permission(&default) != command_permission(c)
            }
        })
}

/// Whether anything a plugin contributes besides its commands changed:
/// `presave` and the contribution kinds no stage acts on yet. What they
/// will be allowed to do is not settled, so any change needs review.
fn other_contributions_changed(new: &Manifest, stored: &Manifest) -> bool {
    let strip = |m: &Manifest| {
        let mut c = m.contributes.clone();
        c.commands.clear();
        c
    };
    strip(new) != strip(stored)
}

/// The first namespaced command name (`<plugin-id>.<command-id>`) of
/// `manifest` that another installed plugin already declares. Command ids
/// may hold dots, so `com.acme` with `tool.run` and `com.acme.tool` with
/// `run` are one name, and the registry would refuse whichever registers
/// second. Checked against every installed plugin, enabled or not, so
/// turning one on later cannot collide either.
pub fn command_name_taken(manifest: &Manifest, others: &[(Installed, Manifest)]) -> Option<String> {
    let taken: std::collections::BTreeSet<String> = others
        .iter()
        .filter(|(i, _)| i.id != manifest.id)
        .flat_map(|(i, m)| {
            m.contributes
                .commands
                .iter()
                .map(move |c| format!("{}.{}", i.id, c.id))
        })
        .collect();
    manifest
        .contributes
        .commands
        .iter()
        .map(|c| format!("{}.{}", manifest.id, c.id))
        .find(|name| taken.contains(name))
}

/// Refuses `command_name_taken` when [`command_name_taken`] finds one
/// among what is installed now.
fn check_command_names(dir: &PluginDir, manifest: &Manifest) -> Result<(), InstallError> {
    match command_name_taken(manifest, &readable(dir)) {
        Some(name) => Err(InstallError::CommandNameTaken(name)),
        None => Ok(()),
    }
}

/// Why the folder's manifest (`found`; `None` when it cannot be read or
/// parsed) needs the person's review against the stored one, or `None`
/// when every difference only refreshes the stored copy. A field is
/// reviewed when it carries authority or the consent sheet showed it.
///
/// | Field | A change |
/// |---|---|
/// | (the file cannot be read or parsed) | review: `manifest_unreadable` |
/// | `id` | review: `id_changed` |
/// | `permissions`, `optionalPermissions` (as sets) | review: `permissions_changed` |
/// | `activation` newly holding `on_startup` | review: `startup_added` |
/// | `name` (on the consent sheet) | review: `consent_text_changed` |
/// | `activationReason` (on the consent sheet) | review: `consent_text_changed` |
/// | a kept command's `class` or `permission` | review: `authority_changed` |
/// | an added command whose `permission` is not its class's default | review: `authority_changed` |
/// | `contributes.presave` and every other `contributes` kind | review: `authority_changed` |
/// | `hew`, `requires` | review when the new ones are not met here (`requirements_unmet`); otherwise refresh |
/// | a command whose namespaced name another installed plugin declares | review: `command_name_taken` |
/// | `contributes.commands` added (class default permission) or removed | refresh |
/// | a command's `summary`, `label`, `timeout_ms`, `concurrency`, `params`, `result`, `refusals` | refresh |
/// | `activation` otherwise (`on_startup` dropped, other events) | refresh |
/// | `version`, `main`, `icon`, `locales`, `description`, `author`, `license`, `homepage`, `source`, `api`, `data`, `agent`, `manifest` | refresh |
fn review_reason(
    dir: &PluginDir,
    id: &str,
    found: Option<&Manifest>,
    stored: &Manifest,
    running: &semver::Version,
) -> Option<&'static str> {
    use plugin_manifest::Activation::OnStartup;
    let Some(m) = found else {
        return Some("manifest_unreadable");
    };
    if m.id != id {
        return Some("id_changed");
    }
    if sorted(&m.permissions) != sorted(&stored.permissions)
        || sorted(&m.optional_permissions) != sorted(&stored.optional_permissions)
    {
        return Some("permissions_changed");
    }
    if m.activation.contains(&OnStartup) && !stored.activation.contains(&OnStartup) {
        return Some("startup_added");
    }
    if m.name != stored.name || m.activation_reason != stored.activation_reason {
        return Some("consent_text_changed");
    }
    if command_authority_changed(m, stored) || other_contributions_changed(m, stored) {
        return Some("authority_changed");
    }
    if (m.hew != stored.hew || m.requires != stored.requires)
        && (!crate::hew_version_ok(m, running) || crate::check_requires(m, &readable(dir)).is_err())
    {
        return Some("requirements_unmet");
    }
    if check_command_names(dir, m).is_err() {
        return Some("command_name_taken");
    }
    None
}

/// Settles a folder plugin's folder against the store, given what reading
/// its `hew-plugin.json` through the jail gave (`found`). The caller holds
/// whatever lock keeps other writers out. The shell runs it whenever the
/// folder changes and whenever the plugin starts being watched, so an edit
/// made while it was not watched is compared too.
///
/// A folder that is gone altogether is returned as `folder_missing` and
/// changes nothing. Otherwise the manifest is compared with the stored one
/// (`review_reason`, whose table lists every field):
///
/// - Something needs review: the record is marked [`NEEDS_CONSENT`] with
///   the reason. A record already marked for that reason is left as it
///   is; one marked for another reason takes the new one. The stored
///   manifest and the grant stay as they are.
/// - Nothing needs review: if the record was marked, the marker is cleared
///   (a mark describes a difference, and there is none now). If the
///   manifest differs from the stored copy at all, the stored copy is
///   replaced with its bytes and the record's `version` follows it. The
///   grant, the optional grants, `installed_at_ms` and everything else in
///   the record are kept. The manifest is written before the record, so
///   an interruption leaves any marker in place.
pub fn apply_folder_manifest(
    dir: &PluginDir,
    id: &str,
    found: Result<Vec<u8>, InstallError>,
    running: &semver::Version,
) -> Result<FolderOutcome, InstallError> {
    let (mut record, stored) = installed(dir, id)?;
    if !matches!(record.source, InstallSource::Dir { .. }) {
        return Ok(FolderOutcome::Unchanged);
    }
    let was_marked = needs_consent(&record);
    let bytes = match found {
        Ok(bytes) => Some(bytes),
        Err(e @ InstallError::FolderMissing(_)) => return Err(e),
        Err(_) => None,
    };
    let parsed = bytes.as_deref().and_then(|b| {
        std::str::from_utf8(b)
            .ok()
            .and_then(|t| plugin_manifest::parse(t).ok())
    });
    if let Some(reason) = review_reason(dir, id, parsed.as_ref(), &stored, running) {
        if was_marked && needs_consent_reason(&record) == Some(reason) {
            return Ok(FolderOutcome::Unchanged);
        }
        record
            .extra
            .insert(NEEDS_CONSENT.to_string(), serde_json::Value::Bool(true));
        record.extra.insert(
            NEEDS_CONSENT_REASON.to_string(),
            serde_json::Value::String(reason.to_string()),
        );
        write_record(dir, &record)?;
        return Ok(FolderOutcome::Marked(reason));
    }
    // No reason means the manifest was read and parsed.
    let (Some(manifest), Some(bytes)) = (parsed, bytes) else {
        return Ok(FolderOutcome::Unchanged);
    };
    let refresh = manifest != stored;
    if refresh {
        write_atomic(&dir.plugin(id)?.join(MANIFEST_FILE), &bytes)?;
    }
    if was_marked {
        clear_marker(&mut record.extra);
    }
    if was_marked || record.version != manifest.version {
        record.version = manifest.version.clone();
        write_record(dir, &record)?;
    }
    Ok(if was_marked {
        FolderOutcome::Cleared
    } else if refresh {
        FolderOutcome::Refreshed
    } else {
        FolderOutcome::Unchanged
    })
}

/// The recorded folder of folder plugin `id`, for the shell's own
/// comparison of its manifest ([`apply_folder_manifest`]) and nothing
/// else. Unlike [`file_source`] it answers for a marked record too, since
/// that comparison is what clears a mark. Refused `developer_mode_off`
/// while developer mode is off; `None` for a plugin that is not a folder
/// install. No folder is touched.
pub fn folder_of(
    dir: &PluginDir,
    id: &str,
    dev: DevMode,
) -> Result<Option<FileSource>, InstallError> {
    let (installed, _) = installed(dir, id)?;
    let InstallSource::Dir { path } = &installed.source else {
        return Ok(None);
    };
    if dev == DevMode::Off {
        return Err(InstallError::DeveloperModeOff);
    }
    Ok(Some(FileSource {
        root: PathBuf::from(path),
        folder: true,
    }))
}

/// Reads what the review of a folder plugin marked [`NEEDS_CONSENT`] will
/// show: its recorded folder read again under the package rules (no new
/// pick; the root checked as for any read). Changes nothing, so a host
/// calls it without holding its store lock, then [`stage_review`] under
/// it. Refused `developer_mode_off` while developer mode is off,
/// `no_review_pending` for a plugin that is not a marked folder install,
/// and `folder_id_changed` when the folder now names another plugin.
pub fn read_review(
    dir: &PluginDir,
    id: &str,
    dev: DevMode,
) -> Result<(Package, InstallSource), InstallError> {
    if dev == DevMode::Off {
        return Err(InstallError::DeveloperModeOff);
    }
    let (record, _) = installed(dir, id)?;
    let InstallSource::Dir { path } = &record.source else {
        return Err(InstallError::NoReviewPending(id.to_string()));
    };
    if !needs_consent(&record) {
        return Err(InstallError::NoReviewPending(id.to_string()));
    }
    let root = checked_root(Path::new(path))?;
    let package = read_folder(&root)?;
    if package.manifest.id != id {
        return Err(InstallError::FolderIdChanged(crate::clean_text(
            &package.manifest.id,
            crate::MAX_NAME_CHARS,
        )));
    }
    Ok((package, record.source))
}

/// Stages a review read by [`read_review`]: the record must still be the
/// marked folder install it was read from (`no_review_pending` otherwise),
/// then [`stage`] builds the consent against the installed record and
/// stored manifest, so it lists what was added and what was removed, and
/// the payload's `review` is set. The marker stays until
/// [`commit_review`].
pub fn stage_review(
    dir: &PluginDir,
    package: Package,
    source: InstallSource,
    running: &semver::Version,
    now_ms: u64,
) -> Result<Pending, InstallError> {
    let id = package.manifest.id.clone();
    let (record, _) = installed(dir, &id)?;
    if record.source != source || !needs_consent(&record) {
        return Err(InstallError::NoReviewPending(id));
    }
    let mut pending = stage(dir, package, source, running, now_ms)?;
    pending.consent.review = true;
    // A taken command name is refused at `stage` above, so a staged review
    // never has one to name.
    pending.consent.review_line = Some(crate::review_line(needs_consent_reason(&record), None));
    Ok(pending)
}

/// Accepts a staged review ([`stage_review`]) in place: the stored
/// manifest becomes the folder's, the grant becomes its `permissions`, the
/// optional grants keep those it still declares, `startup_allowed` is
/// dropped if it newly asks to start with Hew, the version follows the
/// manifest, and the marker is cleared. `installed_at_ms`, `enabled`,
/// `data_version`, saved data and `previous/` are untouched; nothing is
/// rotated. Refused `pending_unknown` for a pending that is not a review,
/// `pending_expired` after [`PENDING_TTL_MS`], `pending_stale` when the
/// record is no longer the marked one the review was staged against, and
/// `dependency_missing`/`dependency_cycle` as an install would be. The
/// manifest is written before the record, so an interruption leaves the
/// marker in place.
pub fn commit_review(
    dir: &PluginDir,
    pending: Pending,
    now_ms: u64,
) -> Result<Installed, InstallError> {
    if !pending.consent.review {
        return Err(InstallError::PendingUnknown);
    }
    if now_ms.saturating_sub(pending.created_at_ms) > PENDING_TTL_MS {
        return Err(InstallError::PendingExpired);
    }
    let manifest = &pending.package.manifest;
    let (current, _) = installed(dir, &manifest.id)?;
    if Some(&current) != pending.previous.as_ref()
        || current.source != pending.source
        || !needs_consent(&current)
    {
        return Err(InstallError::PendingStale);
    }
    crate::check_requires(manifest, &readable(dir))?;
    check_command_names(dir, manifest)?;
    let startup_newly = pending
        .consent
        .update
        .as_ref()
        .is_some_and(|u| u.startup_newly);
    let mut record = current;
    record.version = manifest.version.clone();
    record.grant = manifest.permissions.clone();
    record
        .optional_granted
        .retain(|o| manifest.optional_permissions.contains(o));
    record.startup_allowed = record.startup_allowed && !startup_newly;
    clear_marker(&mut record.extra);
    let bytes = pending
        .package
        .files
        .get(MANIFEST_FILE)
        .ok_or(InstallError::ManifestMissing)?;
    write_atomic(&dir.plugin(&manifest.id)?.join(MANIFEST_FILE), bytes)?;
    write_record(dir, &record)?;
    Ok(record)
}

fn write_record(dir: &PluginDir, installed: &Installed) -> Result<(), InstallError> {
    write_atomic(
        &dir.plugin(&installed.id)?.join(INSTALLED_FILE),
        &installed.to_json(),
    )
}

/// Enabled plugins whose `requires` names `id`, sorted by id.
pub fn dependents_of(dir: &PluginDir, id: &str) -> Vec<String> {
    readable(dir)
        .into_iter()
        .filter(|(i, m)| i.enabled && m.requires.iter().any(|r| r.id == id))
        .map(|(i, _)| i.id)
        .collect()
}

/// Turns a plugin on or off. Turning one on allows it to start with Hew
/// from then on (`startup_allowed`). Turning one off also turns off every
/// enabled plugin that requires it, directly or through others; those ids
/// are returned beside the record, in the order they were turned off.
pub fn set_enabled(
    dir: &PluginDir,
    id: &str,
    enabled: bool,
) -> Result<(Installed, Vec<String>), InstallError> {
    let (mut installed, _) = installed(dir, id)?;
    installed.enabled = enabled;
    if enabled {
        installed.startup_allowed = true;
    }
    write_record(dir, &installed)?;
    let mut also = Vec::new();
    if !enabled {
        let mut queue = vec![id.to_string()];
        while let Some(off) = queue.pop() {
            for dependent in dependents_of(dir, &off) {
                let (mut record, _) = self::installed(dir, &dependent)?;
                record.enabled = false;
                write_record(dir, &record)?;
                also.push(dependent.clone());
                queue.push(dependent);
            }
        }
    }
    Ok((installed, also))
}

/// Replaces the plugin's grant. Every permission must be one its manifest
/// declares, `grant` from `permissions` and `optional_granted` from
/// `optionalPermissions`; anything else is `unknown_permission`, so a grant
/// can never hold what the user was not shown.
pub fn set_grant(
    dir: &PluginDir,
    id: &str,
    grant: Vec<PermissionName>,
    optional_granted: Vec<PermissionName>,
) -> Result<Installed, InstallError> {
    let (mut installed, manifest) = installed(dir, id)?;
    let undeclared = grant
        .iter()
        .find(|p| !manifest.permissions.contains(p))
        .or_else(|| {
            optional_granted
                .iter()
                .find(|p| !manifest.optional_permissions.contains(p))
        });
    if let Some(p) = undeclared {
        return Err(InstallError::Manifest(
            plugin_manifest::ManifestError::UnknownPermission(p.as_str().to_string()),
        ));
    }
    installed.grant = grant;
    installed.optional_granted = optional_granted;
    write_record(dir, &installed)?;
    Ok(installed)
}

/// Removes everything the store holds for `id`: `plugins/<id>/`
/// (`previous/` included) and any staging entry. The one removal path
/// uninstall takes; stores added later join it here.
pub fn purge(dir: &PluginDir, id: &str) -> Result<(), InstallError> {
    let folder = dir.plugin(id)?;
    remove_any(&folder)?;
    clear_staging(dir, id, false)
}

/// Uninstalls `id`: refused `dependency_missing`, naming them, while
/// enabled plugins require it; `not_installed` when there is nothing to
/// remove. A plugin whose record cannot be read can still be uninstalled.
pub fn uninstall(dir: &PluginDir, id: &str) -> Result<(), InstallError> {
    let folder = dir.plugin(id)?;
    if std::fs::symlink_metadata(&folder).is_err() {
        return Err(InstallError::NotInstalled(id.to_string()));
    }
    let dependents = dependents_of(dir, id);
    if !dependents.is_empty() {
        return Err(InstallError::DependencyMissing(dependents));
    }
    purge(dir, id)
}

/// Whether the version in `previous/` may come back, by the rules an
/// install follows: its `hew` range must admit `running`
/// (`hew_version_unsupported`), what it `requires` must be installed and
/// acyclic (`dependency_missing`, `dependency_cycle`), and none of its
/// commands may have another installed plugin's command name
/// (`command_name_taken`).
/// [`rollback`] checks it before anything moves; a host can ask first, to
/// refuse before it asks the person.
pub fn check_rollback(
    dir: &PluginDir,
    id: &str,
    running: &semver::Version,
) -> Result<(), InstallError> {
    let (_, manifest) = self::previous(dir, id)?;
    if !crate::hew_version_ok(&manifest, running) {
        return Err(InstallError::HewVersionUnsupported {
            required: manifest
                .hew
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            running: running.to_string(),
        });
    }
    crate::check_requires(&manifest, &readable(dir))?;
    check_command_names(dir, &manifest)
}

/// Swaps `previous/` into place; the current version becomes its
/// `previous/`, so a second rollback goes forward again. Refused, with
/// nothing moved, as [`check_rollback`] refuses.
pub fn rollback(
    dir: &PluginDir,
    id: &str,
    running: &semver::Version,
) -> Result<Installed, InstallError> {
    rollback_with(dir, id, running, &mut |from, to| std::fs::rename(from, to))
}

fn rollback_with(
    dir: &PluginDir,
    id: &str,
    running: &semver::Version,
    rename: Rename<'_>,
) -> Result<Installed, InstallError> {
    let current = dir.plugin(id)?;
    installed(dir, id)?;
    check_rollback(dir, id, running)?;
    let previous = current.join(PREVIOUS);
    // The saved data stays the plugin's: the version coming back gets the
    // current copy, not the one it had when it was replaced.
    carry_storage(&current, &previous)?;
    let staging = dir.staging();
    std::fs::create_dir_all(&staging).map_err(io)?;
    // Named `.old` while it holds the version before, so an interrupted
    // swap is kept by `clear_staging`.
    let swap = staging.join(format!("{id}-{}.old", new_token()));
    rename(&previous, &swap).map_err(io)?;
    if let Err(e) = rename(&current, &swap.join(PREVIOUS)) {
        let _ = rename(&swap, &previous);
        return Err(io(e));
    }
    if let Err(e) = rename(&swap, &current) {
        let _ = rename(&swap.join(PREVIOUS), &current);
        let _ = rename(&swap, &previous);
        return Err(io(e));
    }
    installed(dir, id).map(|(i, _)| i)
}

/// Makes `to`'s `storage.json` a copy of `from`'s, through a temporary
/// file and a rename, or removes it when `from` has none. Only a regular
/// file is copied; anything else in its place counts as none.
fn carry_storage(from: &Path, to: &Path) -> Result<(), InstallError> {
    let source = from.join(STORAGE_FILE);
    let target = to.join(STORAGE_FILE);
    let is_file = std::fs::symlink_metadata(&source).is_ok_and(|m| m.file_type().is_file());
    if !is_file {
        return remove_any(&target);
    }
    let tmp = target.with_extension("json.tmp");
    std::fs::copy(&source, &tmp).map_err(io)?;
    std::fs::rename(&tmp, &target).map_err(io)
}

/// A plugin's saved data, by key.
type Storage = BTreeMap<String, String>;

fn checked_key(key: &str) -> Result<(), InstallError> {
    if key.is_empty() || key.len() > crate::MAX_STORAGE_KEY_BYTES {
        return Err(InstallError::StorageKeyInvalid);
    }
    Ok(())
}

/// `plugins/<id>/storage.json` of an installed plugin: `not_installed`
/// unless `plugins/<id>/` is a folder holding a record, so no call ever
/// creates a plugin's folder.
fn storage_path(dir: &PluginDir, id: &str) -> Result<PathBuf, InstallError> {
    let folder = dir.plugin(id)?;
    let installed = std::fs::symlink_metadata(&folder).is_ok_and(|m| m.is_dir())
        && std::fs::symlink_metadata(folder.join(INSTALLED_FILE))
            .is_ok_and(|m| m.file_type().is_file());
    if !installed {
        return Err(InstallError::NotInstalled(id.to_string()));
    }
    Ok(folder.join(STORAGE_FILE))
}

/// Reads a `storage.json`: empty when there is none; `storage_unreadable`
/// when it is not a regular file, is over the cap, or is not an object of
/// strings.
fn read_storage(path: &Path) -> Result<Storage, InstallError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Storage::new()),
        Err(e) => return Err(io(e)),
        Ok(m) if !m.file_type().is_file() => return Err(InstallError::StorageUnreadable),
        Ok(_) => {}
    }
    let bytes = read_capped(path, crate::MAX_STORAGE_BYTES).map_err(|e| match e {
        InstallError::Io(_) => e,
        _ => InstallError::StorageUnreadable,
    })?;
    serde_json::from_slice(&bytes).map_err(|_| InstallError::StorageUnreadable)
}

/// The value saved under `key`, if any.
pub fn storage_get(dir: &PluginDir, id: &str, key: &str) -> Result<Option<String>, InstallError> {
    checked_key(key)?;
    let mut storage = read_storage(&storage_path(dir, id)?)?;
    Ok(storage.remove(key))
}

/// Every saved key, sorted.
pub fn storage_list(dir: &PluginDir, id: &str) -> Result<Vec<String>, InstallError> {
    Ok(read_storage(&storage_path(dir, id)?)?.into_keys().collect())
}

/// Saves `value` under `key`, writing the whole file through a temporary
/// file and a rename. Refused `storage_quota`, with nothing written, when
/// the file would pass [`crate::MAX_STORAGE_BYTES`].
pub fn storage_set(dir: &PluginDir, id: &str, key: &str, value: &str) -> Result<(), InstallError> {
    checked_key(key)?;
    // Too large on its own: refused before the file is read.
    if key.len().saturating_add(value.len()) > crate::MAX_STORAGE_BYTES {
        return Err(InstallError::StorageQuota);
    }
    let path = storage_path(dir, id)?;
    let mut storage = read_storage(&path)?;
    storage.insert(key.to_string(), value.to_string());
    let bytes = serde_json::to_vec(&storage).map_err(|e| InstallError::Io(e.to_string()))?;
    if bytes.len() > crate::MAX_STORAGE_BYTES {
        return Err(InstallError::StorageQuota);
    }
    write_atomic(&path, &bytes)
}

/// Removes `key`; answers whether it was there. The file goes when its
/// last key does.
pub fn storage_delete(dir: &PluginDir, id: &str, key: &str) -> Result<bool, InstallError> {
    checked_key(key)?;
    let path = storage_path(dir, id)?;
    let mut storage = read_storage(&path)?;
    if storage.remove(key).is_none() {
        return Ok(false);
    }
    if storage.is_empty() {
        remove_any(&path)?;
    } else {
        let bytes = serde_json::to_vec(&storage).map_err(|e| InstallError::Io(e.to_string()))?;
        write_atomic(&path, &bytes)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "com.example.undo";

    fn running() -> semver::Version {
        semver::Version::new(1, 4, 0)
    }

    /// A scratch store removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let dir = std::env::temp_dir()
                .join(format!("hew-plugin-host-unit-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn store(&self) -> PluginDir {
            PluginDir::at(self.0.join("plugins"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn package(version: &str) -> Package {
        let manifest = serde_json::json!({
            "manifest": 1, "id": ID, "name": "Undo", "version": version, "api": 1,
            "main": "main.js"
        });
        crate::package_from_files(BTreeMap::from([
            (
                "hew-plugin.json".to_string(),
                manifest.to_string().into_bytes(),
            ),
            ("main.js".to_string(), format!("// {version}").into_bytes()),
        ]))
        .unwrap()
    }

    fn stage_version(dir: &PluginDir, version: &str) -> Pending {
        stage(
            dir,
            package(version),
            InstallSource::File { name: "u".into() },
            &semver::Version::new(1, 4, 0),
            0,
        )
        .unwrap()
    }

    fn install(dir: &PluginDir, version: &str) {
        commit(dir, stage_version(dir, version), true, 1).unwrap();
    }

    /// A rename that fails at call `fail_at` (counting from 1), and at
    /// every later call too when `and_after`.
    fn failing(fail_at: usize, and_after: bool) -> impl FnMut(&Path, &Path) -> std::io::Result<()> {
        let mut calls = 0;
        move |from, to| {
            calls += 1;
            if calls == fail_at || (and_after && calls > fail_at) {
                return Err(std::io::Error::other("refused on purpose"));
            }
            std::fs::rename(from, to)
        }
    }

    fn version(dir: &PluginDir) -> String {
        installed(dir, ID).unwrap().0.version.to_string()
    }

    fn previous_version(dir: &PluginDir) -> Option<String> {
        previous(dir, ID).ok().map(|(i, _)| i.version.to_string())
    }

    fn staging_names(dir: &PluginDir) -> Vec<String> {
        std::fs::read_dir(dir.staging())
            .map(|e| {
                e.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_failed_move_in_commit_is_undone_at_every_step() {
        // Installed 2.0.0 with 1.0.0 as previous: the update to 3.0.0 makes
        // three moves; fail each in turn.
        for step in 1..=3 {
            let scratch = Scratch::new(&format!("commit-{step}"));
            let dir = scratch.store();
            install(&dir, "1.0.0");
            install(&dir, "2.0.0");
            let pending = stage_version(&dir, "3.0.0");
            let result = commit_with(&dir, pending, true, 2, &mut failing(step, false));
            assert_eq!(result.unwrap_err().name(), "io", "step {step}");
            assert_eq!(version(&dir), "2.0.0", "step {step}");
            assert_eq!(
                previous_version(&dir).as_deref(),
                Some("1.0.0"),
                "step {step}"
            );
            assert!(
                staging_names(&dir).is_empty(),
                "step {step}: {:?}",
                staging_names(&dir)
            );
            assert_eq!(list(&dir).len(), 1);
        }
        // Installed with no previous: two moves.
        for step in 1..=2 {
            let scratch = Scratch::new(&format!("commit-np-{step}"));
            let dir = scratch.store();
            install(&dir, "1.0.0");
            let pending = stage_version(&dir, "2.0.0");
            assert!(commit_with(&dir, pending, true, 2, &mut failing(step, false)).is_err());
            assert_eq!(version(&dir), "1.0.0", "step {step}");
            assert_eq!(previous_version(&dir), None);
            assert!(staging_names(&dir).is_empty(), "step {step}");
        }
        // A fresh install: one move.
        let scratch = Scratch::new("commit-fresh");
        let dir = scratch.store();
        let pending = stage_version(&dir, "1.0.0");
        assert!(commit_with(&dir, pending, true, 2, &mut failing(1, false)).is_err());
        assert!(list(&dir).is_empty());
        assert!(staging_names(&dir).is_empty());
    }

    #[test]
    fn a_failed_move_in_rollback_is_undone_at_every_step() {
        for step in 1..=3 {
            let scratch = Scratch::new(&format!("rollback-{step}"));
            let dir = scratch.store();
            install(&dir, "1.0.0");
            install(&dir, "2.0.0");
            assert!(rollback_with(&dir, ID, &running(), &mut failing(step, false)).is_err());
            assert_eq!(version(&dir), "2.0.0", "step {step}");
            assert_eq!(
                previous_version(&dir).as_deref(),
                Some("1.0.0"),
                "step {step}"
            );
            assert!(staging_names(&dir).is_empty(), "step {step}");
            assert_eq!(
                rollback(&dir, ID, &running()).unwrap().version.to_string(),
                "1.0.0"
            );
        }
    }

    #[test]
    fn a_copy_stranded_in_staging_is_never_cleared() {
        // The last move of an update fails and so does every undo: the
        // installed 2.0.0 sits only in staging, inside the new copy's
        // `previous/`, and 1.0.0 only in the `.old` entry.
        let scratch = Scratch::new("stranded");
        let dir = scratch.store();
        install(&dir, "1.0.0");
        install(&dir, "2.0.0");
        let pending = stage_version(&dir, "3.0.0");
        assert!(commit_with(&dir, pending, true, 2, &mut failing(3, true)).is_err());
        assert!(!dir.root().join(ID).exists());
        let mut before = staging_names(&dir);
        before.sort();
        assert_eq!(before.len(), 2, "{before:?}");
        // The next stage of the id clears staging, but keeps both.
        let _ = stage_version(&dir, "3.0.0");
        let mut after = staging_names(&dir);
        after.sort();
        assert_eq!(after, before);
        // Uninstall removes everything.
        purge(&dir, ID).unwrap();
        assert!(staging_names(&dir).is_empty());

        // A rollback whose swap and undo fail after the first move: the
        // version before sits in a `.old` entry while its home is missing.
        let scratch = Scratch::new("stranded-rollback");
        let dir = scratch.store();
        install(&dir, "1.0.0");
        install(&dir, "2.0.0");
        assert!(rollback_with(&dir, ID, &running(), &mut failing(2, true)).is_err());
        assert_eq!(version(&dir), "2.0.0");
        assert_eq!(previous_version(&dir), None);
        let kept = staging_names(&dir);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].ends_with(".old"));
        let _ = stage_version(&dir, "3.0.0");
        assert_eq!(staging_names(&dir), kept, "the version before is kept");
    }

    #[test]
    fn staging_entries_belong_to_exactly_one_id() {
        let token = "0123456789abcdef0123456789abcdef";
        assert!(staging_of(&format!("a.b-{token}"), "a.b"));
        assert!(staging_of(&format!("a.b-{token}.old"), "a.b"));
        assert!(!staging_of(&format!("a.b-c-{token}"), "a.b"));
        assert!(!staging_of(&format!("a.bc-{token}"), "a.b"));
        assert!(!staging_of("a.b-short", "a.b"));
    }

    #[test]
    fn tokens_are_32_hex_digits_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
