//! Reading a plugin package: a `.hewplugin` zip, or (in `store.rs`) an
//! unpacked folder, into a [`Package`] under one set of rules.
//!
//! Every path a package holds goes through [`normalize_path`], which
//! accepts only names that mean the same thing on every platform Hew runs
//! on: no absolute paths, no `.` or `..`, no backslashes, no drive letters
//! or alternate data streams, no Windows device names, no names a Windows
//! file system would silently change. Two entries that differ only in case
//! collide. A symbolic link, or anything else that is not a regular file or
//! a folder, is refused. Sizes are measured on the bytes actually read,
//! never taken from a header.

use crate::InstallError;
use plugin_manifest::Manifest;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

/// Most entries (files and folders) one package may hold.
pub const MAX_PACKAGE_ENTRIES: usize = 4096;
/// Largest package, in bytes: the zip itself, and the sum of its files.
pub const MAX_PACKAGE_BYTES: usize = 64 << 20;
/// Largest single file in a package, in bytes.
pub const MAX_ENTRY_BYTES: usize = 32 << 20;
/// Longest path inside a package, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// Most permissions a package may declare, `permissions` and
/// `optionalPermissions` together, so a consent sheet stays readable.
pub const MAX_PERMISSIONS: usize = 64;
/// The manifest's name, at the package root.
pub const MANIFEST_FILE: &str = "hew-plugin.json";

/// An in-memory package: the parsed manifest and every file by served path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Names Windows reserves for devices, in any case and with any
/// extension: `CON`, `con.txt` and `Com1.js` all open a device there.
const DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$", "COM0", "COM1", "COM2", "COM3",
    "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Characters no package path may hold: separators other than `/`, the
/// drive and alternate-data-stream colon, and the rest of what Windows
/// refuses in a name.
const FORBIDDEN: &[char] = &['\\', ':', '<', '>', '"', '|', '?', '*'];

/// Characters that change how text around them reads without showing
/// themselves: bidirectional controls and zero-width characters. Refused
/// in paths and removed from consent text ([`crate::clean_text`]).
pub(crate) fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// One path segment: non-empty, not `.` or `..`, no trailing dot or space
/// (Windows drops them, so `a.` and `a` would be one file), no control,
/// invisible or forbidden character, and not a device name.
fn segment_ok(segment: &str) -> bool {
    if segment.is_empty() || segment.ends_with('.') || segment.ends_with(' ') {
        return false;
    }
    if segment
        .chars()
        .any(|c| c.is_control() || is_invisible(c) || FORBIDDEN.contains(&c))
    {
        return false;
    }
    let stem = segment.split('.').next().unwrap_or(segment).trim_end();
    let upper = stem.to_uppercase();
    !DEVICE_NAMES.contains(&upper.as_str())
}

/// Checks a package path and returns it as stored: ASCII only, `/` separators, no
/// leading `/`, no `.` or `..` segments, no empty segments, no backslash,
/// no `:` anywhere, no Windows device name, no segment ending in a dot or
/// a space, no control or invisible character, at most
/// [`MAX_PATH_BYTES`]. Anything else is `PackagePathInvalid`. ASCII only
/// because file systems disagree about other names: macOS treats the
/// composed and decomposed forms of `é` as one file, so two entries that
/// differ only in Unicode normalization would land on one file there. A
/// path is
/// never rewritten into a different one: a name that is not already in
/// this form is refused, so what the user consented to is what is stored.
pub fn normalize_path(raw: &str) -> Result<String, InstallError> {
    let invalid = || InstallError::PackagePathInvalid(raw.to_string());
    if raw.is_empty() || raw.len() > MAX_PATH_BYTES || raw.starts_with('/') || !raw.is_ascii() {
        return Err(invalid());
    }
    if !raw.split('/').all(segment_ok) {
        return Err(invalid());
    }
    Ok(raw.to_string())
}

/// The key two paths collide under: lowercase, so `A.js` and `a.js` are
/// one file on a case-insensitive file system and are refused together.
fn fold(path: &str) -> String {
    path.to_lowercase()
}

/// Tracks the paths seen so far, refusing a repeat under case folding and
/// a path that is both a file and a folder (`a` and `a/b`).
#[derive(Default)]
struct PathSet {
    files: BTreeSet<String>,
    dirs: BTreeSet<String>,
}

impl PathSet {
    fn add(&mut self, path: &str, is_dir: bool) -> Result<(), InstallError> {
        let invalid = || InstallError::PackagePathInvalid(path.to_string());
        let key = fold(path);
        if self.files.contains(&key) {
            return Err(invalid());
        }
        if is_dir {
            // Only folders the archive lists as entries are recorded; two
            // such entries that fold to one name collide.
            if !self.dirs.insert(key) {
                return Err(invalid());
            }
            return Ok(());
        }
        if self.dirs.contains(&key) {
            return Err(invalid());
        }
        self.files.insert(key);
        Ok(())
    }

    /// After every entry is in: no entry's parent is itself a file.
    fn check_parents(&self) -> Result<(), InstallError> {
        for path in self.files.iter().chain(&self.dirs) {
            let mut at = 0;
            while let Some(slash) = path[at..].find('/') {
                let parent = &path[..at + slash];
                if self.files.contains(parent) {
                    return Err(InstallError::PackagePathInvalid(parent.to_string()));
                }
                at += slash + 1;
            }
        }
        Ok(())
    }
}

/// The file-type bits of a unix mode.
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

/// Where one file's bytes sit in a `.hewplugin`: `offset` is the first
/// data byte counted from the start of the archive, `size` the number of
/// bytes. Entries are stored, so `bytes[offset..offset + size]` is the
/// file, exactly as [`read_package_zip`] read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EntrySpan {
    pub offset: u64,
    pub size: u64,
}

/// Reads a `.hewplugin`: a zip whose entries are all STORED (method 0).
/// A compressed or encrypted entry is `package_not_store_only`. Paths go
/// through [`normalize_path`]; two entries equal after case folding, an
/// entry name repeated in the archive, and a symbolic link (or any entry
/// whose unix mode is not a regular file or a folder) are
/// `package_path_invalid`. Entry and total sizes are measured on the bytes
/// read (`package_too_large`). Then [`package_from_files`].
pub fn read_package_zip(bytes: &[u8]) -> Result<Package, InstallError> {
    read_package_zip_spans(bytes).map(|(package, _)| package)
}

/// [`read_package_zip`], with where each file's bytes sit in `bytes`, so a
/// host that keeps the archive (the web build, in the page) can store
/// exactly the bytes that were checked rather than reading the zip a
/// second time with another reader. Every span is checked against the
/// bytes read: the slice at the span is the file, byte for byte. No entry
/// (its local header and data) overlaps another, and none reaches into the
/// central directory; an archive where they do is `package_unreadable`.
pub fn read_package_zip_spans(
    bytes: &[u8],
) -> Result<(Package, BTreeMap<String, EntrySpan>), InstallError> {
    let unreadable = |why: &str| InstallError::PackageUnreadable(why.to_string());
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(InstallError::PackageTooLarge);
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| InstallError::PackageUnreadable(e.to_string()))?;
    if archive.len() > MAX_PACKAGE_ENTRIES {
        return Err(InstallError::PackageTooLarge);
    }
    // The central directory and each entry's local header must agree (a
    // name or size that differs between the two is how one archive shows
    // different contents to different readers), no two entries may share
    // a name, and no two may overlap.
    if let Err(e) = archive.validate_header_consistency() {
        let text = e.to_string();
        if text.contains("duplicate file names") {
            return Err(InstallError::PackagePathInvalid(
                "two entries with one name".to_string(),
            ));
        }
        return Err(InstallError::PackageUnreadable(text));
    }

    let mut seen = PathSet::default();
    let mut files = BTreeMap::new();
    let mut spans = BTreeMap::new();
    // Every entry's extent in the archive, local header through data.
    let mut extents: Vec<(u64, u64)> = Vec::new();
    let mut directory_start = u64::MAX;
    let mut total = 0usize;
    for index in 0..archive.len() {
        let (path, is_dir) = {
            let entry = archive
                .by_index_raw(index)
                .map_err(|e| InstallError::PackageUnreadable(e.to_string()))?;
            let raw = entry.name_raw();
            let name = std::str::from_utf8(raw).map_err(|_| {
                InstallError::PackagePathInvalid(String::from_utf8_lossy(raw).into_owned())
            })?;
            if entry.encrypted() || entry.compression() != zip::CompressionMethod::Stored {
                return Err(InstallError::PackageNotStoreOnly);
            }
            let kind = entry.unix_mode().map(|m| m & S_IFMT);
            let named_dir = name.ends_with('/');
            match kind {
                None | Some(0) => {}
                Some(S_IFREG) if !named_dir => {}
                Some(S_IFDIR) if named_dir => {}
                _ => return Err(InstallError::PackagePathInvalid(name.to_string())),
            }
            let path = if named_dir {
                normalize_path(&name[..name.len() - 1])?
            } else {
                normalize_path(name)?
            };
            (path, named_dir)
        };
        seen.add(&path, is_dir)?;
        let mut entry = archive
            .by_index(index)
            .map_err(|e| InstallError::PackageUnreadable(e.to_string()))?;
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_ENTRY_BYTES as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|e| InstallError::PackageUnreadable(e.to_string()))?;
        if data.len() > MAX_ENTRY_BYTES {
            return Err(InstallError::PackageTooLarge);
        }
        let offset = entry
            .data_start()
            .ok_or_else(|| unreadable("an entry's data has no position"))?;
        let size = data.len() as u64;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| unreadable("an entry runs past the end"))?;
        let slice = usize::try_from(offset)
            .ok()
            .zip(usize::try_from(end).ok())
            .and_then(|(from, to)| bytes.get(from..to));
        if entry.compressed_size() != size || slice != Some(&data[..]) {
            return Err(unreadable("an entry's bytes are not where its header says"));
        }
        extents.push((entry.header_start(), end));
        directory_start = directory_start.min(entry.central_header_start());
        drop(entry);
        if is_dir {
            if size != 0 {
                return Err(unreadable("a folder entry holds data"));
            }
            continue;
        }
        total += data.len();
        if total > MAX_PACKAGE_BYTES {
            return Err(InstallError::PackageTooLarge);
        }
        spans.insert(path.clone(), EntrySpan { offset, size });
        files.insert(path, data);
    }
    extents.sort_unstable();
    if extents.windows(2).any(|w| w[0].1 > w[1].0)
        || extents
            .last()
            .is_some_and(|&(_, end)| end > directory_start)
    {
        return Err(unreadable("entries overlap each other or the directory"));
    }
    seen.check_parents()?;
    Ok((package_from_files(files)?, spans))
}

/// Every file the package's manifest names must be in it. `rel` is
/// checked as a package path first; a name that is not one cannot be in
/// the package.
fn has_file(files: &BTreeMap<String, Vec<u8>>, rel: &str) -> bool {
    normalize_path(rel).is_ok_and(|p| files.contains_key(&p))
}

/// Builds a [`Package`] from files already read and named by package path
/// (each one passed [`normalize_path`]): `hew-plugin.json` must be at the
/// root (`manifest_missing`), be UTF-8, and parse
/// ([`plugin_manifest::parse`]); `main`, `icon`, `agent.notes` must name
/// files in the package and `locales.catalogs` a folder holding at least
/// one (`manifest_file_missing`); at most [`MAX_PERMISSIONS`] permissions
/// are declared (`too_many_permissions`); `agent.notes` is at most
/// [`plugin_manifest::MAX_AGENT_NOTES_BYTES`] (`package_too_large`). The
/// entry count and total size are checked again here so the folder reader
/// shares them.
pub fn package_from_files(files: BTreeMap<String, Vec<u8>>) -> Result<Package, InstallError> {
    if files.len() > MAX_PACKAGE_ENTRIES
        || files.values().map(Vec::len).sum::<usize>() > MAX_PACKAGE_BYTES
        || files.values().any(|f| f.len() > MAX_ENTRY_BYTES)
    {
        return Err(InstallError::PackageTooLarge);
    }
    let text = files
        .get(MANIFEST_FILE)
        .ok_or(InstallError::ManifestMissing)?;
    let text = std::str::from_utf8(text).map_err(|_| {
        InstallError::Manifest(plugin_manifest::ManifestError::Json(
            "the file is not UTF-8 text".to_string(),
        ))
    })?;
    let manifest = plugin_manifest::parse(text)?;
    let declared = manifest.permissions.len() + manifest.optional_permissions.len();
    if declared > MAX_PERMISSIONS {
        return Err(InstallError::TooManyPermissions(declared));
    }

    let mut named: Vec<&str> = vec![manifest.main.as_str()];
    named.extend(manifest.icon.as_deref());
    named.extend(manifest.agent.as_ref().map(|a| a.notes.as_str()));
    for rel in named {
        if !has_file(&files, rel) {
            return Err(InstallError::ManifestFileMissing(rel.to_string()));
        }
    }
    if let Some(notes) = manifest.agent.as_ref().map(|a| a.notes.as_str())
        && files
            .get(notes)
            .is_some_and(|f| f.len() > plugin_manifest::MAX_AGENT_NOTES_BYTES)
    {
        return Err(InstallError::PackageTooLarge);
    }
    if let Some(locales) = &manifest.locales {
        let folder = locales
            .catalogs
            .strip_suffix('/')
            .unwrap_or(&locales.catalogs);
        let ok = normalize_path(folder).is_ok_and(|folder| {
            let prefix = format!("{folder}/");
            files.keys().any(|k| k.starts_with(&prefix))
        });
        if !ok {
            return Err(InstallError::ManifestFileMissing(locales.catalogs.clone()));
        }
    }
    Ok(Package { manifest, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_characters_include_the_bidi_controls() {
        for c in ['\u{202E}', '\u{2066}', '\u{200B}', '\u{FEFF}', '\u{061C}'] {
            assert!(is_invisible(c), "{:04X}", c as u32);
        }
        for c in ['a', ' ', 'é', '日'] {
            assert!(!is_invisible(c));
        }
    }

    #[test]
    fn a_file_inside_a_file_is_refused() {
        let mut set = PathSet::default();
        set.add("a", false).unwrap();
        set.add("a/b", false).unwrap();
        assert!(set.check_parents().is_err());
    }
}
