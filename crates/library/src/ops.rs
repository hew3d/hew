//! File operations over one resolved [`LibraryDir`]: listing (with parsed
//! metadata and manifest summaries), reading, atomic writing, removal, and
//! the thumbnail-cache path/hashing helpers. Ports of
//! `shells/tauri/src-tauri/src/main.rs`'s `library_list`/`library_read`/
//! `library_write`/`library_delete`/`library_thumb_read`/`library_thumb_write`
//! plus the summary-parsing half of `app/src/library/libraryModel.ts`.

use crate::dir::LibraryDir;
use crate::meta::{self, Category, ItemMeta};
use crate::naming::{CATEGORY_DIRS, valid_item_name, valid_thumb_key};
use kernel::ItemSummary;
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};

/// One fully-listed library item: its relative path, parsed metadata,
/// manifest summary, and file stats — or, for a file that failed to parse,
/// the error instead of a summary. Never dropped from a listing just
/// because it failed to parse (docs/design/v1.1-cycle.md's Lane B: "never
/// dropped").
#[derive(Debug, Clone)]
pub struct ListedItem {
    /// Relative to the library folder, forward-slash, including the
    /// category subfolder when there is one (`"Components/chair-3f2a.hew"`).
    pub rel_path: String,
    /// Parsed `hew.library` metadata (empty when the file has none or
    /// failed to parse).
    pub meta: ItemMeta,
    /// The resolved category: `meta.category` when set, else derived from
    /// the summary's shape ([`meta::derive_category`]). `None` when the
    /// file failed to parse (there is no summary to derive from).
    pub category: Option<Category>,
    /// The resolved display name ([`meta::display_name`]), or the bare
    /// file stem for a file that failed to parse.
    pub display_name: String,
    pub summary: Option<ItemSummary>,
    pub size: u64,
    pub mtime_ms: u64,
    /// Set instead of `summary` for a file that isn't a valid `.hew`
    /// container (truncated download, foreign zip, unsupported format
    /// version, ...).
    pub error: Option<String>,
}

/// Lists every `.hew` item in the library folder: legacy flat files
/// directly in the folder, plus each of the three category subfolders —
/// mirrors `library_list`'s file discovery, but ALSO reads and parses each
/// file's manifest summary (`kernel::read_item_summary`) and `hew.library`
/// metadata, unlike the Tauri command (which leaves that to the caller). A
/// missing library folder lists as empty, not an error — nothing has been
/// saved there yet.
pub fn list(dir: &LibraryDir) -> Vec<ListedItem> {
    let mut out = Vec::new();
    list_into(dir.root(), None, &mut out);
    for category in CATEGORY_DIRS {
        list_into(&dir.root().join(category), Some(category), &mut out);
    }
    out
}

fn list_into(scan_dir: &Path, prefix: Option<&str>, out: &mut Vec<ListedItem>) {
    let Ok(entries) = std::fs::read_dir(scan_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let rel_path = match prefix {
            Some(prefix) => format!("{prefix}/{file_name}"),
            None => file_name,
        };
        if !valid_item_name(&rel_path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let size = metadata.len();

        // A seekable `File`, not `std::fs::read`'s whole-file `Vec<u8>`:
        // `read_item_summary_from_reader` (via `zip`'s own lazy
        // central-directory reads) then only pulls `manifest.json` and
        // each material's texture asset off disk — a listing never loads
        // an item's geometry buffers just to summarize it.
        match std::fs::File::open(entry.path())
            .map_err(|e| e.to_string())
            .and_then(|file| {
                kernel::read_item_summary_from_reader(file).map_err(|e| format!("{e:?}"))
            }) {
            Ok(summary) => {
                let raw_meta = summary
                    .doc_attrs
                    .get("hew.library")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let meta = meta::parse_item_meta(&raw_meta);
                let category = meta
                    .category
                    .unwrap_or_else(|| meta::derive_category(&summary));
                let display_name = meta::display_name(&meta, &summary, &rel_path);
                out.push(ListedItem {
                    rel_path,
                    meta,
                    category: Some(category),
                    display_name,
                    summary: Some(summary),
                    size,
                    mtime_ms,
                    error: None,
                });
            }
            Err(e) => {
                out.push(ListedItem {
                    display_name: meta::file_stem(&rel_path),
                    rel_path,
                    meta: ItemMeta::default(),
                    category: None,
                    summary: None,
                    size,
                    mtime_ms,
                    error: Some(e),
                });
            }
        }
    }
}

/// Reads one item's raw bytes by relative path. Refuses (with a plain
/// `io::Error`) a `rel_path` that fails [`valid_item_name`] — every caller
/// here is trusted (a host that has already resolved a client-facing `item`
/// id/path into a concrete listing entry), but the check stays cheap
/// insurance against a hand-built path ever reaching `Path::join`.
pub fn read(dir: &LibraryDir, rel_path: &str) -> io::Result<Vec<u8>> {
    check_name(rel_path)?;
    std::fs::read(dir.root().join(rel_path))
}

/// Writes an item's bytes, creating the library folder — and, for a
/// two-segment path, its category subfolder — on first use. Atomic: temp
/// file in the same directory, then rename, so a crash mid-write can never
/// leave a torn file.
pub fn write(dir: &LibraryDir, rel_path: &str, bytes: &[u8]) -> io::Result<()> {
    check_name(rel_path)?;
    let path = dir.root().join(rel_path);
    let parent = path.parent().unwrap_or(dir.root());
    std::fs::create_dir_all(parent)?;
    write_atomic(&path, bytes)
}

/// Deletes an item. Deleting an already-absent file is not an error — the
/// caller's goal ("this name is gone") is already satisfied.
pub fn remove(dir: &LibraryDir, rel_path: &str) -> io::Result<()> {
    check_name(rel_path)?;
    match std::fs::remove_file(dir.root().join(rel_path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The cached-thumbnail path for a content-hash key
/// (`<library>/.thumbnails/<key>.png`). Does not check the key is valid —
/// callers that accept a key from outside this crate validate it with
/// [`crate::naming::valid_thumb_key`] first.
pub fn thumbnail_path(dir: &LibraryDir, key: &str) -> PathBuf {
    dir.root().join(".thumbnails").join(format!("{key}.png"))
}

/// Reads a cached thumbnail PNG by content-hash key, or `None` if absent.
pub fn read_thumbnail(dir: &LibraryDir, key: &str) -> io::Result<Option<Vec<u8>>> {
    if !valid_thumb_key(key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid thumbnail key",
        ));
    }
    match std::fs::read(thumbnail_path(dir, key)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes a cached thumbnail PNG, creating `.thumbnails/` on first use.
pub fn write_thumbnail(dir: &LibraryDir, key: &str, png: &[u8]) -> io::Result<()> {
    if !valid_thumb_key(key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid thumbnail key",
        ));
    }
    let path = thumbnail_path(dir, key);
    let parent = path.parent().expect("thumbnail_path always has a parent");
    std::fs::create_dir_all(parent)?;
    write_atomic(&path, png)
}

/// SHA-256 of `bytes`, lowercase hex — the content-hash scheme every
/// caller (kernel `LibraryProvenance`, the thumbnail cache key, the UI's
/// `sha256Hex`) agrees on.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn check_name(rel_path: &str) -> io::Result<()> {
    if valid_item_name(rel_path) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a valid library item name: {rel_path:?}"),
        ))
    }
}

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// then rename — mirrors `write_atomic` in
/// `shells/tauri/src-tauri/src/main.rs`.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Unique per writer, not just per target path: two concurrent saves to
    // the SAME item name (two `hew-cli` processes, or a `--live` save
    // racing a Tauri-window save) previously shared one `<name>.tmp`
    // sibling and could clobber each other's temp file mid-write. Pid +
    // nanosecond timestamp makes collision astronomically unlikely without
    // pulling in a UUID/tempfile crate for it.
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.{unique}.tmp"),
        None => format!("{unique}.tmp"),
    });
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::Document;

    fn scratch_dir(tag: &str) -> LibraryDir {
        let dir = std::env::temp_dir().join(format!(
            "hew-library-ops-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        LibraryDir::at(dir)
    }

    #[test]
    fn write_read_remove_round_trip() {
        let dir = scratch_dir("roundtrip");
        write(&dir, "Components/x-abcdef.hew", b"hello").unwrap();
        assert_eq!(read(&dir, "Components/x-abcdef.hew").unwrap(), b"hello");
        remove(&dir, "Components/x-abcdef.hew").unwrap();
        assert!(read(&dir, "Components/x-abcdef.hew").is_err());
        // Removing again is not an error.
        remove(&dir, "Components/x-abcdef.hew").unwrap();
        let _ = std::fs::remove_dir_all(dir.root());
    }

    #[test]
    fn write_rejects_an_invalid_name() {
        let dir = scratch_dir("badname");
        assert!(write(&dir, "../escape.hew", b"x").is_err());
        assert!(!dir.root().parent().unwrap().join("escape.hew").exists());
    }

    #[test]
    fn list_reads_summaries_and_never_drops_an_unreadable_file() {
        let dir = scratch_dir("list");
        // A genuine empty document, saved as a "model" item.
        let doc = Document::new();
        let bytes = doc.save();
        write(&dir, "Models/empty-model-000000.hew", &bytes).unwrap();
        // A garbage file that isn't a valid container at all.
        write(&dir, "Models/garbage-000000.hew", b"not a zip").unwrap();

        let items = list(&dir);
        assert_eq!(items.len(), 2);
        let good = items
            .iter()
            .find(|i| i.rel_path.contains("empty-model"))
            .unwrap();
        assert!(good.error.is_none());
        assert!(good.summary.is_some());
        let bad = items
            .iter()
            .find(|i| i.rel_path.contains("garbage"))
            .unwrap();
        assert!(bad.error.is_some());
        assert!(bad.summary.is_none());

        let _ = std::fs::remove_dir_all(dir.root());
    }

    #[test]
    fn list_scans_profiles_and_derives_their_category() {
        let dir = scratch_dir("profiles");
        let (_, _, make) = kernel::bundled_profiles()[0];
        let bytes = Document::profile_document_from_shape(&make(), 0.05)
            .unwrap()
            .save();
        write(&dir, "Profiles/ogee-000000.hew", &bytes).unwrap();
        let items = list(&dir);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].category, Some(meta::Category::Profile));
        let _ = std::fs::remove_dir_all(dir.root());
    }

    #[test]
    fn sha256_hex_matches_a_known_vector() {
        // SHA-256("") — the empty-string test vector.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn thumbnail_write_read_round_trip() {
        let dir = scratch_dir("thumb");
        let key = "deadbeefcafef00d";
        assert_eq!(read_thumbnail(&dir, key).unwrap(), None);
        write_thumbnail(&dir, key, b"png-bytes").unwrap();
        assert_eq!(
            read_thumbnail(&dir, key).unwrap().as_deref(),
            Some(&b"png-bytes"[..])
        );
        let _ = std::fs::remove_dir_all(dir.root());
    }
}
