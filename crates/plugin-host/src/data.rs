//! What a plugin keeps that is not its package (feature `fs`): a tree
//! beside the install store that no version swap touches.
//!
//! ```text
//! plugin-data/                        beside `plugins/` (`PluginDir::data_root`)
//!   <id>/
//!     documents/<document-id>.json    per-document storage, one object per document
//!     console.log                     the plugin's console, one JSON line each
//!     console.log.1                   the console before the last rotation
//!     activity.json                   the last state changes the hosts reported
//!     usage.json                      the running total the quota is checked against
//! ```
//!
//! The rules:
//!
//! - Every path is built from a plugin id that passed the plugin id grammar
//!   and, for a document, a document id in the 36-character lowercase UUID
//!   form ([`valid_document_id`]). Keys are only ever map keys.
//! - Every write refuses `not_installed` for an id with no record in
//!   `plugins/<id>/`, before anything is created, so no data is ever made
//!   for a plugin that is not there.
//! - Installing, updating, rolling back and the developer-folder paths
//!   never look at this tree. Uninstalling removes `plugin-data/<id>/`
//!   before the plugin's record ([`crate::store::purge`]).
//! - A link, or anything that is not a regular file or a folder, where the
//!   tree expects a file or a folder is refused `storage_unreadable`: never
//!   followed, never rewritten. A file is written whole through a fresh
//!   temporary file (created only if nothing is there, so a planted link is
//!   not followed) and a rename.
//! - One quota for the whole tree: [`MAX_PLUGIN_DATA_BYTES`], refused
//!   `storage_quota`. Every file counts at least [`MIN_FILE_COST`], so the
//!   tree can hold only so many files. The total is kept in `usage.json`
//!   rather than measured on every write; see [`charge`] for how it stays
//!   an upper bound of what is on disk, and why a missing or stale total
//!   can only refuse too early, never let the tree grow past the quota.
//!
//! The caller holds whatever lock keeps other writers out (the desktop
//! shell's store lock): this module is the tree's only writer.

use crate::store::{
    PluginDir, checked_id, checked_key, io, read_capped, read_storage, remove_any, require_record,
};
use crate::{InstallError, MAX_PLUGIN_DATA_BYTES, MAX_STORAGE_BYTES, clean_text};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The tree's folder name beside a store folder named `plugins`.
pub const DATA_FOLDER: &str = "plugin-data";
/// Per-document storage, inside `plugin-data/<id>/`.
pub const DOCUMENTS_FOLDER: &str = "documents";
pub const ACTIVITY_FILE: &str = "activity.json";
pub const CONSOLE_FILE: &str = "console.log";
pub const CONSOLE_ROTATED_FILE: &str = "console.log.1";
/// The running total of what the tree holds, for the quota.
pub const USAGE_FILE: &str = "usage.json";

/// The most one document's storage file may hold, as written.
pub const MAX_DOCUMENT_STORAGE_BYTES: usize = MAX_STORAGE_BYTES;
/// What a file counts toward the quota at the least, whatever its size: a
/// disk block. It bounds how many files the tree can hold.
pub const MIN_FILE_COST: u64 = 4096;
/// How many entries `activity.json` keeps; the oldest go first.
pub const MAX_ACTIVITY_ENTRIES: usize = 500;
/// The largest `activity.json` read back.
const MAX_ACTIVITY_FILE_BYTES: usize = 1 << 20;
/// The size at which `console.log` is rotated to `console.log.1`.
pub const MAX_CONSOLE_BYTES: u64 = 1 << 20;
/// The most characters of one console line's text that are kept.
pub const MAX_CONSOLE_TEXT_CHARS: usize = 2000;
/// The most lines one append writes; the rest of a longer batch is dropped.
pub const MAX_CONSOLE_LINES_PER_APPEND: usize = 64;
/// The most lines [`console_read`] answers.
pub const MAX_CONSOLE_READ: usize = 1000;
/// The largest console file read back.
const MAX_CONSOLE_FILE_BYTES: usize = 2 << 20;
/// The most characters of a state report's reason that are kept.
pub const MAX_STATE_REASON_CHARS: usize = 160;
/// The longest window label an activity entry carries.
pub const MAX_WINDOW_LABEL_BYTES: usize = 64;

/// The states a host reports for a plugin in one window.
pub const PLUGIN_STATES: &[&str] = &[
    "enabled",
    "activating",
    "activated",
    "crashed",
    "blocked",
    "idle",
    "unloaded",
];

/// The levels a console line may carry.
pub const CONSOLE_LEVELS: &[&str] = &["log", "info", "warn", "error"];

/// More entries than this in the tree means it is over the quota whatever
/// they hold (each file counts at least [`MIN_FILE_COST`]), so a walk
/// stops there.
const MAX_TREE_ENTRIES: u64 = MAX_PLUGIN_DATA_BYTES / MIN_FILE_COST + 64;

/// Whether `doc` is a document id in the form the `.hew` manifest's
/// `document_id` takes: 36 characters, lowercase hexadecimal digits in
/// groups of 8, 4, 4, 4 and 12, joined by hyphens.
pub fn valid_document_id(doc: &str) -> bool {
    doc.len() == 36
        && doc.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

/// Refuses `document_id_invalid` unless [`valid_document_id`].
pub fn checked_document_id(doc: &str) -> Result<(), InstallError> {
    if valid_document_id(doc) {
        Ok(())
    } else {
        Err(InstallError::DocumentIdInvalid)
    }
}

/// What is at a path, looked at without following a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Missing,
    Folder,
    File(u64),
    /// A link, or anything else that is not a regular file or a folder.
    Other,
}

fn kind(path: &Path) -> Result<Kind, InstallError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Kind::Missing),
        Err(e) => Err(io(e)),
        Ok(m) if m.file_type().is_dir() => Ok(Kind::Folder),
        Ok(m) if m.file_type().is_file() => Ok(Kind::File(m.len())),
        Ok(_) => Ok(Kind::Other),
    }
}

/// The size of the regular file at `path`, `None` when there is none; a
/// folder, a link or anything else is `storage_unreadable`.
fn file_len(path: &Path) -> Result<Option<u64>, InstallError> {
    match kind(path)? {
        Kind::Missing => Ok(None),
        Kind::File(len) => Ok(Some(len)),
        Kind::Folder | Kind::Other => Err(InstallError::StorageUnreadable),
    }
}

/// Whether `path` is a folder (`false` when nothing is there); a link or
/// anything else is `storage_unreadable`.
fn folder_exists(path: &Path) -> Result<bool, InstallError> {
    match kind(path)? {
        Kind::Missing => Ok(false),
        Kind::Folder => Ok(true),
        Kind::File(_) | Kind::Other => Err(InstallError::StorageUnreadable),
    }
}

/// Makes `path` a folder when nothing is there; a folder already there is
/// kept, anything else is `storage_unreadable`. The parent must exist.
fn ensure_folder(path: &Path) -> Result<(), InstallError> {
    if folder_exists(path)? {
        return Ok(());
    }
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if folder_exists(path)? {
                Ok(())
            } else {
                Err(InstallError::StorageUnreadable)
            }
        }
        Err(e) => Err(io(e)),
    }
}

fn no_data_root() -> InstallError {
    InstallError::Io("there is no folder beside the plugin store to keep plugin data in".into())
}

/// `plugin-data/<id>` for a write: the id checked, the plugin installed,
/// and `plugin-data/` and `plugin-data/<id>/` made folders if they are not.
fn tree_for_write(dir: &PluginDir, id: &str) -> Result<PathBuf, InstallError> {
    checked_id(id)?;
    require_record(dir, id)?;
    let root = dir.data_root().ok_or_else(no_data_root)?;
    ensure_folder(&root)?;
    let tree = root.join(id);
    ensure_folder(&tree)?;
    Ok(tree)
}

/// `plugin-data/<id>` for a read: the id checked and the plugin installed;
/// `None` when the tree is not there yet.
fn tree_for_read(dir: &PluginDir, id: &str) -> Result<Option<PathBuf>, InstallError> {
    checked_id(id)?;
    require_record(dir, id)?;
    let Some(root) = dir.data_root() else {
        return Ok(None);
    };
    if !folder_exists(&root)? {
        return Ok(None);
    }
    let tree = root.join(id);
    Ok(folder_exists(&tree)?.then_some(tree))
}

/// The temporary file a write of `path` goes through.
fn temp_of(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Writes `bytes` to `path` through a fresh temporary file and a rename.
/// `path` must be a regular file or nothing; whatever is at the temporary
/// name (a leftover, or a link) is removed first, never followed, and the
/// temporary file is created only if nothing is there.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), InstallError> {
    file_len(path)?;
    let tmp = temp_of(path);
    remove_any(&tmp)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(io)?;
    let written = file.write_all(bytes).and_then(|()| file.flush());
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(e)
    })
}

/// Appends `bytes` to the regular file at `path`, creating it when nothing
/// is there. A link or anything else is refused; on Unix the file opened
/// must also be the very file looked at, so one swapped for a link in
/// between is refused rather than followed.
fn append_file(path: &Path, bytes: &[u8]) -> Result<(), InstallError> {
    let mut file = match kind(path)? {
        Kind::Missing => std::fs::OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)
            .map_err(io)?,
        Kind::File(_) => {
            let looked = std::fs::symlink_metadata(path).map_err(io)?;
            let file = std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(io)?;
            let opened = file.metadata().map_err(io)?;
            if !opened.is_file() {
                return Err(InstallError::StorageUnreadable);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if looked.dev() != opened.dev() || looked.ino() != opened.ino() {
                    return Err(InstallError::StorageUnreadable);
                }
            }
            #[cfg(not(unix))]
            let _ = looked;
            file
        }
        Kind::Folder | Kind::Other => return Err(InstallError::StorageUnreadable),
    };
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(io)
}

/// What a file of `len` bytes (or none) counts toward the quota.
fn cost(len: Option<u64>) -> u64 {
    len.map_or(0, |n| n.max(MIN_FILE_COST))
}

/// The running total in `usage.json`, or `None` when it is missing,
/// unreadable, or not a regular file.
fn read_tally(tree: &Path) -> Option<u64> {
    let bytes = read_capped(&tree.join(USAGE_FILE), 256).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("bytes")?.as_u64()
}

fn write_tally(tree: &Path, total: u64) -> Result<(), InstallError> {
    write_file(
        &tree.join(USAGE_FILE),
        serde_json::json!({ "bytes": total }).to_string().as_bytes(),
    )
}

/// What the tree holds now, by walking it: every file (a link counts as
/// the link, never followed) at [`cost`], folders at nothing, the running
/// total's own file left out. Only names and sizes are read. A tree with
/// more entries than the quota could hold answers `u64::MAX`.
fn measure(tree: &Path) -> Result<u64, InstallError> {
    let usage = [
        std::ffi::OsString::from(USAGE_FILE),
        temp_of(Path::new(USAGE_FILE)).into_os_string(),
    ];
    let mut total = 0u64;
    let mut seen = 0u64;
    let mut pending = vec![tree.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let listing = match std::fs::read_dir(&folder) {
            Ok(listing) => listing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(e)),
        };
        for entry in listing {
            let entry = entry.map_err(io)?;
            seen += 1;
            if seen > MAX_TREE_ENTRIES {
                return Ok(u64::MAX);
            }
            if folder == tree && usage.contains(&entry.file_name()) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_dir() {
                pending.push(path);
            } else {
                total = total.saturating_add(cost(Some(meta.len())));
            }
        }
    }
    Ok(total)
}

/// Runs `write`, which turns files of the tree that count `before` toward
/// the quota into files that count `after`, under the tree's quota.
///
/// The total in `usage.json` is kept at or above what is on disk at every
/// moment, so an interruption anywhere leaves it too high, never too low:
///
/// 1. The total is read; when it is missing or unreadable the tree is
///    walked ([`measure`]).
/// 2. A write that grows the tree past the quota by that total is checked
///    again against a walk (an interruption may have left the total too
///    high) and refused `storage_quota` if it still does, with the measured
///    total saved. A write that does not grow the tree is never refused.
/// 3. Before the write, the total is raised to cover the old and the new
///    copies both (a write goes through a temporary file, and a rotation
///    keeps the old file until it is renamed), so a write cut short at any
///    point leaves a total that covers what is there.
/// 4. After the write, the total is lowered to what the tree now holds. If
///    that fails, the higher total stays, which only refuses sooner.
///
/// Nothing but this module writes the tree, so the total can be too low
/// only if something outside Hew changes the folder.
fn charge(
    tree: &Path,
    before: u64,
    after: u64,
    write: impl FnOnce() -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    charge_with(tree, before, after, true, write)
}

/// [`charge`] for Hew's own records (`activity.json`, `console.log` and its
/// rotation). They are bounded by their own caps, so a full tree never
/// refuses them (a crash record must not be lost to a plugin's documents),
/// but they still count toward the total that documents are charged
/// against.
fn charge_own(
    tree: &Path,
    before: u64,
    after: u64,
    write: impl FnOnce() -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    charge_with(tree, before, after, false, write)
}

fn charge_with(
    tree: &Path,
    before: u64,
    after: u64,
    enforce: bool,
    write: impl FnOnce() -> Result<(), InstallError>,
) -> Result<(), InstallError> {
    let total = match read_tally(tree) {
        Some(total) => total,
        None => measure(tree)?,
    };
    let mut rest = total.saturating_sub(before);
    if enforce && after > before && rest.saturating_add(after) > MAX_PLUGIN_DATA_BYTES {
        let exact = measure(tree)?;
        rest = exact.saturating_sub(before);
        if rest.saturating_add(after) > MAX_PLUGIN_DATA_BYTES {
            let _ = write_tally(tree, exact);
            return Err(InstallError::DataQuota);
        }
    }
    write_tally(tree, rest.saturating_add(before).saturating_add(after))?;
    // The tally is written first, as an upper bound, so a crash between the
    // two steps over-counts (the next measure corrects it) rather than
    // under-counts.
    if let Err(e) = write() {
        // The write did not land, so the bound would only grow with each
        // retry. Put back what is really there; when that cannot be
        // measured, the bound stays (the safe side).
        if let Ok(exact) = measure(tree) {
            let _ = write_tally(tree, exact);
        }
        return Err(e);
    }
    let _ = write_tally(tree, rest.saturating_add(after));
    Ok(())
}

/// Removes `plugin-data/<id>/` and everything in it, never following a
/// link: when `plugin-data/` itself is not a folder (a link, say), nothing
/// in it is Hew's and nothing is removed through it. Part of
/// [`crate::store::purge`].
pub fn remove(dir: &PluginDir, id: &str) -> Result<(), InstallError> {
    checked_id(id)?;
    let Some(root) = dir.data_root() else {
        return Ok(());
    };
    if kind(&root)? != Kind::Folder {
        return Ok(());
    }
    remove_any(&root.join(id))
}

// ---------------------------------------------------------------------------
// Per-document storage
// ---------------------------------------------------------------------------

/// `documents/<document-id>.json` under `tree`.
fn document_file(tree: &Path, doc: &str) -> PathBuf {
    tree.join(DOCUMENTS_FOLDER).join(format!("{doc}.json"))
}

/// The document's saved data, empty when there is none.
fn read_document(
    dir: &PluginDir,
    id: &str,
    doc: &str,
) -> Result<BTreeMap<String, String>, InstallError> {
    let Some(tree) = tree_for_read(dir, id)? else {
        return Ok(BTreeMap::new());
    };
    if !folder_exists(&tree.join(DOCUMENTS_FOLDER))? {
        return Ok(BTreeMap::new());
    }
    read_storage(&document_file(&tree, doc))
}

/// The value plugin `id` saved under `key` for document `doc`, if any.
/// Checked in order: the plugin id, the document id
/// (`document_id_invalid`), the key (`storage_key_invalid`), then that the
/// plugin is installed (`not_installed`).
pub fn doc_storage_get(
    dir: &PluginDir,
    id: &str,
    doc: &str,
    key: &str,
) -> Result<Option<String>, InstallError> {
    checked_id(id)?;
    checked_document_id(doc)?;
    checked_key(key)?;
    Ok(read_document(dir, id, doc)?.remove(key))
}

/// Every key plugin `id` saved for document `doc`, sorted.
pub fn doc_storage_list(dir: &PluginDir, id: &str, doc: &str) -> Result<Vec<String>, InstallError> {
    checked_id(id)?;
    checked_document_id(doc)?;
    Ok(read_document(dir, id, doc)?.into_keys().collect())
}

/// Saves `value` under `key` for document `doc`, writing the document's
/// whole file through a temporary file and a rename. Refused
/// `storage_quota`, with nothing written, when the file would pass
/// [`MAX_DOCUMENT_STORAGE_BYTES`] or the plugin's tree would pass
/// [`MAX_PLUGIN_DATA_BYTES`].
pub fn doc_storage_set(
    dir: &PluginDir,
    id: &str,
    doc: &str,
    key: &str,
    value: &str,
) -> Result<(), InstallError> {
    checked_id(id)?;
    checked_document_id(doc)?;
    checked_key(key)?;
    // Too large on its own: refused before anything is read or made.
    if key.len().saturating_add(value.len()) > MAX_DOCUMENT_STORAGE_BYTES {
        return Err(InstallError::StorageQuota);
    }
    let tree = tree_for_write(dir, id)?;
    ensure_folder(&tree.join(DOCUMENTS_FOLDER))?;
    let path = document_file(&tree, doc);
    let before = file_len(&path)?;
    let mut data = read_storage(&path)?;
    data.insert(key.to_string(), value.to_string());
    let bytes = serde_json::to_vec(&data).map_err(|e| InstallError::Io(e.to_string()))?;
    if bytes.len() > MAX_DOCUMENT_STORAGE_BYTES {
        return Err(InstallError::StorageQuota);
    }
    charge(&tree, cost(before), cost(Some(bytes.len() as u64)), || {
        write_file(&path, &bytes)
    })
}

/// Removes `key` from document `doc`'s saved data; answers whether it was
/// there. The document's file goes when its last key does.
pub fn doc_storage_delete(
    dir: &PluginDir,
    id: &str,
    doc: &str,
    key: &str,
) -> Result<bool, InstallError> {
    checked_id(id)?;
    checked_document_id(doc)?;
    checked_key(key)?;
    let Some(tree) = tree_for_read(dir, id)? else {
        return Ok(false);
    };
    if !folder_exists(&tree.join(DOCUMENTS_FOLDER))? {
        return Ok(false);
    }
    let path = document_file(&tree, doc);
    let before = file_len(&path)?;
    let mut data = read_storage(&path)?;
    if data.remove(key).is_none() {
        return Ok(false);
    }
    if data.is_empty() {
        charge(&tree, cost(before), 0, || remove_any(&path))?;
    } else {
        let bytes = serde_json::to_vec(&data).map_err(|e| InstallError::Io(e.to_string()))?;
        charge(&tree, cost(before), cost(Some(bytes.len() as u64)), || {
            write_file(&path, &bytes)
        })?;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------

/// One state change a host reported for a plugin: when (milliseconds since
/// the Unix epoch, the shell's clock), the window it runs in, one of
/// [`PLUGIN_STATES`], and a short reason.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActivityEntry {
    pub at: u64,
    pub window: String,
    pub state: String,
    pub reason: Option<String>,
}

/// A report's reason, cleaned ([`clean_text`]) and cut to
/// [`MAX_STATE_REASON_CHARS`]; `None` when nothing is left.
pub fn clean_reason(reason: Option<&str>) -> Option<String> {
    reason
        .map(|r| clean_text(r, MAX_STATE_REASON_CHARS))
        .filter(|r| !r.is_empty())
}

/// Whether `label` is a window label an entry may carry: 1 to
/// [`MAX_WINDOW_LABEL_BYTES`] ASCII letters, digits, `-` and `_`.
pub fn valid_window_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_WINDOW_LABEL_BYTES
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// An entry as it is written, or `None` for one that breaks the rules
/// (an unknown state, a bad window label).
fn clean_entry(entry: &ActivityEntry) -> Option<ActivityEntry> {
    if !PLUGIN_STATES.contains(&entry.state.as_str()) || !valid_window_label(&entry.window) {
        return None;
    }
    Some(ActivityEntry {
        at: entry.at,
        window: entry.window.clone(),
        state: entry.state.clone(),
        reason: clean_reason(entry.reason.as_deref()),
    })
}

/// The entries in `activity.json`, oldest first. A file that does not
/// parse is Hew's own log damaged, read as empty (and replaced by the next
/// append); a link or anything else in its place is `storage_unreadable`.
fn read_activity_file(path: &Path) -> Result<Vec<ActivityEntry>, InstallError> {
    if file_len(path)?.is_none() {
        return Ok(Vec::new());
    }
    let bytes = match read_capped(path, MAX_ACTIVITY_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(InstallError::Io(e)) => return Err(InstallError::Io(e)),
        Err(_) => return Ok(Vec::new()),
    };
    let entries: Vec<ActivityEntry> = serde_json::from_slice(&bytes).unwrap_or_default();
    Ok(entries.iter().filter_map(clean_entry).collect())
}

/// Adds `entries` to `activity.json`, keeping the last
/// [`MAX_ACTIVITY_ENTRIES`]. Entries that break the rules are left out.
pub fn activity_append(
    dir: &PluginDir,
    id: &str,
    entries: &[ActivityEntry],
) -> Result<(), InstallError> {
    checked_id(id)?;
    require_record(dir, id)?;
    let fresh: Vec<ActivityEntry> = entries.iter().filter_map(clean_entry).collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let tree = tree_for_write(dir, id)?;
    let path = tree.join(ACTIVITY_FILE);
    let before = file_len(&path)?;
    let mut all = read_activity_file(&path)?;
    all.extend(fresh);
    let over = all.len().saturating_sub(MAX_ACTIVITY_ENTRIES);
    all.drain(..over);
    let bytes = serde_json::to_vec(&all).map_err(|e| InstallError::Io(e.to_string()))?;
    charge_own(&tree, cost(before), cost(Some(bytes.len() as u64)), || {
        write_file(&path, &bytes)
    })
}

/// The entries in `activity.json`, newest first.
pub fn activity_read(dir: &PluginDir, id: &str) -> Result<Vec<ActivityEntry>, InstallError> {
    let Some(tree) = tree_for_read(dir, id)? else {
        return Ok(Vec::new());
    };
    let mut entries = read_activity_file(&tree.join(ACTIVITY_FILE))?;
    entries.reverse();
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

/// One console line: when (milliseconds since the Unix epoch), one of
/// [`CONSOLE_LEVELS`], and the text, cleaned and cut to
/// [`MAX_CONSOLE_TEXT_CHARS`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConsoleLine {
    pub at: u64,
    pub level: String,
    pub text: String,
}

fn clean_line(line: &ConsoleLine) -> Option<ConsoleLine> {
    CONSOLE_LEVELS
        .contains(&line.level.as_str())
        .then(|| ConsoleLine {
            at: line.at,
            level: line.level.clone(),
            text: clean_text(&line.text, MAX_CONSOLE_TEXT_CHARS),
        })
}

/// Appends up to [`MAX_CONSOLE_LINES_PER_APPEND`] of `lines` to
/// `console.log`, one JSON object per line, each text cleaned again here.
/// Lines with a level not in [`CONSOLE_LEVELS`] are left out. When the
/// file would pass [`MAX_CONSOLE_BYTES`] it is first renamed to
/// `console.log.1` (replacing the one before). Answers how many lines were
/// written.
pub fn console_append(
    dir: &PluginDir,
    id: &str,
    lines: &[ConsoleLine],
) -> Result<usize, InstallError> {
    checked_id(id)?;
    require_record(dir, id)?;
    let mut bytes = Vec::new();
    let mut written = 0;
    for line in lines
        .iter()
        .take(MAX_CONSOLE_LINES_PER_APPEND)
        .filter_map(clean_line)
    {
        serde_json::to_writer(&mut bytes, &line).map_err(|e| InstallError::Io(e.to_string()))?;
        bytes.push(b'\n');
        written += 1;
    }
    if written == 0 {
        return Ok(0);
    }
    let tree = tree_for_write(dir, id)?;
    let log = tree.join(CONSOLE_FILE);
    let rotated = tree.join(CONSOLE_ROTATED_FILE);
    let current = file_len(&log)?;
    let previous = file_len(&rotated)?;
    let added = bytes.len() as u64;
    match current {
        Some(len) if len > 0 && len.saturating_add(added) > MAX_CONSOLE_BYTES => charge_own(
            &tree,
            cost(current).saturating_add(cost(previous)),
            cost(current).saturating_add(cost(Some(added))),
            || {
                std::fs::rename(&log, &rotated).map_err(io)?;
                append_file(&log, &bytes)
            },
        )?,
        _ => charge_own(
            &tree,
            cost(current),
            cost(Some(current.unwrap_or(0).saturating_add(added))),
            || append_file(&log, &bytes),
        )?,
    }
    Ok(written)
}

/// The newest `limit` console lines (at most [`MAX_CONSOLE_READ`]), newest
/// first, from `console.log` and then `console.log.1`. Lines that do not
/// parse, or carry an unknown level, are skipped; texts are cleaned again.
pub fn console_read(
    dir: &PluginDir,
    id: &str,
    limit: usize,
) -> Result<Vec<ConsoleLine>, InstallError> {
    let Some(tree) = tree_for_read(dir, id)? else {
        return Ok(Vec::new());
    };
    let limit = limit.min(MAX_CONSOLE_READ);
    let mut out = Vec::new();
    for name in [CONSOLE_FILE, CONSOLE_ROTATED_FILE] {
        if out.len() >= limit {
            break;
        }
        let path = tree.join(name);
        if file_len(&path)?.is_none() {
            continue;
        }
        let bytes = match read_capped(&path, MAX_CONSOLE_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(InstallError::Io(e)) => return Err(InstallError::Io(e)),
            // Larger than Hew writes it: not Hew's log as written.
            Err(_) => continue,
        };
        for raw in bytes.split(|b| *b == b'\n').rev() {
            if out.len() >= limit {
                break;
            }
            if let Some(line) = serde_json::from_slice::<ConsoleLine>(raw)
                .ok()
                .as_ref()
                .and_then(clean_line)
            {
                out.push(line);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_ids_take_exactly_the_lowercase_uuid_form() {
        assert!(valid_document_id("3f2c9107-5ed4-40a8-806b-1ec297405d83"));
        for bad in [
            "",
            "3F2C9107-5ED4-40A8-806B-1EC297405D83",
            "3f2c9107-5ed4-40a8-806b-1ec297405d8",
            "3f2c9107-5ed4-40a8-806b-1ec297405d833",
            "3f2c91075ed4-40a8-806b-1ec297405d83-",
            "3f2c9107-5ed4-40a8-806b-1ec297405d8g",
            "../../../../../../../../../../../../.",
            "3f2c9107/5ed4-40a8-806b-1ec297405d83",
            "3f2c9107-5ed4-40a8-806b-1ec297405d8\u{e9}",
        ] {
            assert!(!valid_document_id(bad), "{bad:?}");
            assert_eq!(
                checked_document_id(bad),
                Err(InstallError::DocumentIdInvalid)
            );
        }
        assert_eq!(
            InstallError::DocumentIdInvalid.name(),
            "document_id_invalid"
        );
    }

    #[test]
    fn a_file_counts_at_least_a_block() {
        assert_eq!(cost(None), 0);
        assert_eq!(cost(Some(0)), MIN_FILE_COST);
        assert_eq!(cost(Some(10)), MIN_FILE_COST);
        assert_eq!(cost(Some(MIN_FILE_COST + 1)), MIN_FILE_COST + 1);
    }

    #[test]
    fn window_labels_and_reasons_are_bounded() {
        assert!(valid_window_label("main"));
        assert!(valid_window_label("main-12"));
        assert!(!valid_window_label(""));
        assert!(!valid_window_label("main 2"));
        assert!(!valid_window_label(&"m".repeat(MAX_WINDOW_LABEL_BYTES + 1)));
        assert_eq!(clean_reason(Some("  \u{202e}boom\n")), Some("boom".into()));
        assert_eq!(clean_reason(Some(" \n")), None);
        let long = clean_reason(Some(&"x".repeat(1000))).unwrap();
        assert_eq!(long.chars().count(), MAX_STATE_REASON_CHARS);
    }

    #[test]
    fn a_failed_write_does_not_leave_the_tally_inflated() {
        let tree = std::env::temp_dir().join(format!("hew-charge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tree);
        std::fs::create_dir_all(&tree).unwrap();
        write_tally(&tree, 1_000).unwrap();
        let base = measure(&tree).unwrap();
        for _ in 0..5 {
            let r = charge_with(&tree, 0, 4_000, true, || {
                Err(InstallError::Io("disk full".into()))
            });
            assert!(r.is_err());
            // What is really there, not the bound that grows per retry.
            assert_eq!(read_tally(&tree), Some(base));
        }
        // A write that lands counts as before.
        charge_with(&tree, 0, 4_000, true, || Ok(())).unwrap();
        assert_eq!(read_tally(&tree), Some(base + 4_000));
        let _ = std::fs::remove_dir_all(&tree);
    }
}
