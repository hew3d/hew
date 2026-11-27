//! The sources file and its cache (docs/design/shared-library-sources.md
//! 6.5, 7.3): `.sources.json` beside the items and the `.sources/` cache
//! directory. Neither is an item; both are dot-names, so they are never
//! listed and never valid item names.

use crate::dir::LibraryDir;
use crate::naming::valid_source_cache_key;
use crate::ops::write_atomic;
use std::io;
use std::path::PathBuf;

/// `.sources.json` beside the items (not an item; never listed).
pub const SOURCES_FILE: &str = ".sources.json";
/// `.sources/` cache directory beside it (not an item; never listed).
pub const SOURCES_CACHE_DIR: &str = ".sources";

/// Pure merge rule. `current` is the file as just re-read (`None` or empty
/// = no file yet, treated as `{"hew":1,"sources":[]}`). Returns the new file
/// bytes in which only the entry whose `"source"` equals `source` changed:
/// replaced in place if present, appended if absent, removed when `entry`
/// is `None`. Every other entry, and unknown top-level keys, survive.
/// Errors (never repairs silently): `current` is not valid JSON or not
/// `{"sources":[..]}` shaped; `entry` is not an object whose `"source"`
/// field equals `source`.
pub fn merge_source_entry(
    current: Option<&[u8]>,
    source: &str,
    entry: Option<&serde_json::Value>,
) -> Result<Vec<u8>, String> {
    use serde_json::{Value, json};
    let mut doc: Value = match current {
        None => json!({ "hew": 1, "sources": [] }),
        Some([]) => json!({ "hew": 1, "sources": [] }),
        Some(b) => {
            serde_json::from_slice(b).map_err(|e| format!("sources file is not JSON: {e}"))?
        }
    };
    if let Some(e) = entry {
        let ok = e
            .as_object()
            .and_then(|o| o.get("source"))
            .and_then(Value::as_str)
            == Some(source);
        if !ok {
            return Err(format!(
                "source entry must be an object whose \"source\" is {source:?}"
            ));
        }
    }
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| "sources file is not a JSON object".to_string())?;
    let list = obj
        .entry("sources")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "\"sources\" is not an array".to_string())?;
    let at = list
        .iter()
        .position(|s| s.get("source").and_then(Value::as_str) == Some(source));
    match (at, entry) {
        (Some(i), Some(e)) => list[i] = e.clone(),
        (None, Some(e)) => list.push(e.clone()),
        (Some(i), None) => {
            list.remove(i);
        }
        (None, None) => {}
    }
    obj.entry("hew").or_insert_with(|| json!(1));
    serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())
}

fn read_optional(path: &std::path::Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Raw bytes of `<library>/.sources.json`, `None` when absent.
pub fn read_sources(dir: &LibraryDir) -> io::Result<Option<Vec<u8>>> {
    read_optional(&dir.root().join(SOURCES_FILE))
}

/// Re-reads `.sources.json`, applies [`merge_source_entry`], writes the
/// result atomically (temp file in the same folder + rename).
pub fn write_sources(
    dir: &LibraryDir,
    source: &str,
    entry: Option<&serde_json::Value>,
) -> io::Result<()> {
    let path = dir.root().join(SOURCES_FILE);
    let current = read_optional(&path)?;
    let merged = merge_source_entry(current.as_deref(), source, entry)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::create_dir_all(dir.root())?;
    write_atomic(&path, &merged)
}

/// `<library>/.sources/<key>` (no extension). Does not validate.
pub fn source_cache_path(dir: &LibraryDir, key: &str) -> PathBuf {
    dir.root().join(SOURCES_CACHE_DIR).join(key)
}

fn check_key(key: &str) -> io::Result<()> {
    if valid_source_cache_key(key) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid source cache key",
        ))
    }
}

/// Cache bytes by key, `None` when absent; `InvalidInput` for a bad key.
pub fn read_source_cache(dir: &LibraryDir, key: &str) -> io::Result<Option<Vec<u8>>> {
    check_key(key)?;
    read_optional(&source_cache_path(dir, key))
}

/// Delete a cache entry by key; absent is fine. `InvalidInput` for a bad key.
pub fn remove_source_cache(dir: &LibraryDir, key: &str) -> io::Result<()> {
    check_key(key)?;
    match std::fs::remove_file(source_cache_path(dir, key)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Atomic cache write, creating `.sources/` on first use; `InvalidInput`
/// for a bad key.
pub fn write_source_cache(dir: &LibraryDir, key: &str, bytes: &[u8]) -> io::Result<()> {
    check_key(key)?;
    let path = source_cache_path(dir, key);
    std::fs::create_dir_all(dir.root().join(SOURCES_CACHE_DIR))?;
    write_atomic(&path, bytes)
}
