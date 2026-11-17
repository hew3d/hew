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

/// Reads an unpacked plugin folder under the same rules as a `.hewplugin`:
/// every file and folder name passes [`crate::normalize_path`], names that
/// collide under case folding are refused, a symbolic link (or any entry
/// that is not a regular file or folder) is `package_path_invalid`, and the
/// entry and size caps are measured on the bytes read. Then
/// [`crate::package_from_files`].
pub fn read_package_dir(path: &Path) -> Result<Package, InstallError> {
    let meta = std::fs::symlink_metadata(path).map_err(io)?;
    if !meta.is_dir() {
        return Err(InstallError::PackagePathInvalid(path.display().to_string()));
    }
    let mut files = BTreeMap::new();
    let mut folded = std::collections::BTreeSet::new();
    let mut entries = 0usize;
    let mut total = 0usize;
    let mut pending = vec![(path.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
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
                let data = read_capped(&child.path(), MAX_ENTRY_BYTES)?;
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
/// acyclic; an installed plugin of the same id must be readable (a record
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
        extra: previous.map(|p| p.extra.clone()).unwrap_or_default(),
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

/// A package file by served path, jailed: `rel` is checked exactly as
/// package paths are, then read from `files/<rel>` under the plugin's
/// folder (or `<dir>/<rel>` for a folder install). No part of the path may
/// be a symbolic link, the file must be a regular file inside that root,
/// and it is read to at most [`MAX_ENTRY_BYTES`]. Anything else is
/// `package_path_invalid`.
pub fn read_file(dir: &PluginDir, id: &str, rel: &str) -> Result<Vec<u8>, InstallError> {
    let invalid = || InstallError::PackagePathInvalid(rel.to_string());
    let rel = crate::normalize_path(rel)?;
    let (installed, _) = installed(dir, id)?;
    let base = match &installed.source {
        InstallSource::Dir { path } => PathBuf::from(path),
        _ => dir.plugin(id)?.join(FILES),
    };
    let mut at = base.clone();
    for segment in rel.split('/') {
        at.push(segment);
        let meta = std::fs::symlink_metadata(&at).map_err(|_| invalid())?;
        if meta.file_type().is_symlink() {
            return Err(invalid());
        }
    }
    let canonical_base = std::fs::canonicalize(&base).map_err(|_| invalid())?;
    let canonical = std::fs::canonicalize(&at).map_err(|_| invalid())?;
    if !canonical.starts_with(&canonical_base) {
        return Err(invalid());
    }
    read_capped(&canonical, MAX_ENTRY_BYTES).map_err(|e| match e {
        InstallError::PackageTooLarge => e,
        _ => invalid(),
    })
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

/// Swaps `previous/` into place; the current version becomes its
/// `previous/`, so a second rollback goes forward again.
pub fn rollback(dir: &PluginDir, id: &str) -> Result<Installed, InstallError> {
    rollback_with(dir, id, &mut |from, to| std::fs::rename(from, to))
}

fn rollback_with(dir: &PluginDir, id: &str, rename: Rename<'_>) -> Result<Installed, InstallError> {
    let current = dir.plugin(id)?;
    installed(dir, id)?;
    self::previous(dir, id)?;
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
            assert!(rollback_with(&dir, ID, &mut failing(step, false)).is_err());
            assert_eq!(version(&dir), "2.0.0", "step {step}");
            assert_eq!(
                previous_version(&dir).as_deref(),
                Some("1.0.0"),
                "step {step}"
            );
            assert!(staging_names(&dir).is_empty(), "step {step}");
            assert_eq!(rollback(&dir, ID).unwrap().version.to_string(), "1.0.0");
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
        assert!(rollback_with(&dir, ID, &mut failing(2, true)).is_err());
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
