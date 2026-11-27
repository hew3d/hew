//! Oracle for package S9 (docs/design/shared-library-sources.md 6.5, 7.3):
//! the `.sources.json` file and `.sources/` cache primitives.

use library::{
    LibraryDir, SOURCES_CACHE_DIR, SOURCES_FILE, list, merge_source_entry, read_source_cache,
    read_sources, remove_source_cache, source_cache_path, valid_item_name, valid_source_cache_key,
    write, write_source_cache, write_sources,
};
use serde_json::{Value, json};
use std::path::Path;

fn scratch(tag: &str) -> LibraryDir {
    let dir = std::env::temp_dir().join(format!(
        "hew-library-sources-test-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    LibraryDir::at(dir)
}

fn entry(source: &str, name: &str) -> Value {
    json!({
        "source": source, "kind": "github", "name": name, "followed": false,
        "addedAt": "2026-11-23T00:00:00Z",
        "lastCheck": { "at": "2026-11-23T00:00:00Z", "ok": true, "code": null },
        "skipped": [], "kept": {}
    })
}

fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("file is valid JSON")
}

fn sources_of(v: &Value) -> Vec<Value> {
    v["sources"].as_array().expect("sources array").clone()
}

fn names_in(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

// --- the pure merge rule ---------------------------------------------------

#[test]
fn merge_into_no_file_makes_a_version_1_file_with_just_that_entry() {
    let a = entry("github:a/a", "A");
    let out = parse(&merge_source_entry(None, "github:a/a", Some(&a)).unwrap());
    assert_eq!(out["hew"], json!(1));
    assert_eq!(sources_of(&out), vec![a]);
}

#[test]
fn merge_treats_an_empty_file_like_no_file() {
    let a = entry("github:a/a", "A");
    let out = parse(&merge_source_entry(Some(b""), "github:a/a", Some(&a)).unwrap());
    assert_eq!(sources_of(&out), vec![a]);
}

#[test]
fn merge_changes_only_the_named_source_and_keeps_order_and_unknown_fields() {
    let mut a = entry("github:a/a", "A");
    a["futureField"] = json!({ "x": 1 });
    let b = entry("github:b/b", "B");
    let c = entry("github:c/c", "C");
    let current = serde_json::to_vec(&json!({
        "hew": 1, "extraTop": true, "sources": [a.clone(), b.clone(), c.clone()]
    }))
    .unwrap();
    let mut b2 = b.clone();
    b2["followed"] = json!(true);
    b2["skipped"] = json!(["id-1"]);
    let out = parse(&merge_source_entry(Some(&current), "github:b/b", Some(&b2)).unwrap());
    assert_eq!(sources_of(&out), vec![a, b2, c]);
    assert_eq!(out["extraTop"], json!(true));
    assert_eq!(out["hew"], json!(1));
}

#[test]
fn merge_appends_an_unknown_source_after_the_existing_ones() {
    let a = entry("github:a/a", "A");
    let b = entry("github:b/b", "B");
    let current = merge_source_entry(None, "github:a/a", Some(&a)).unwrap();
    let out = parse(&merge_source_entry(Some(&current), "github:b/b", Some(&b)).unwrap());
    assert_eq!(sources_of(&out), vec![a, b]);
}

#[test]
fn merge_with_none_removes_only_that_source() {
    let a = entry("github:a/a", "A");
    let b = entry("github:b/b", "B");
    let current = serde_json::to_vec(&json!({ "hew": 1, "sources": [a.clone(), b] })).unwrap();
    let out = parse(&merge_source_entry(Some(&current), "github:b/b", None).unwrap());
    assert_eq!(sources_of(&out), vec![a]);
    // Removing a source that is not there is not an error and changes nothing.
    let again = parse(&merge_source_entry(Some(&current), "github:zzz/zzz", None).unwrap());
    assert_eq!(sources_of(&again).len(), 2);
}

#[test]
fn two_writers_for_different_sources_both_survive_whichever_goes_second() {
    // Writer 1 and writer 2 each hold the SAME stale snapshot `base`; each
    // applies its own entry to whatever the file holds when it writes
    // (the re-read), so neither erases the other.
    let base = merge_source_entry(
        None,
        "github:base/base",
        Some(&entry("github:base/base", "Base")),
    )
    .unwrap();
    let a = entry("github:a/a", "A");
    let b = entry("github:b/b", "B");
    let after_a = merge_source_entry(Some(&base), "github:a/a", Some(&a)).unwrap();
    let after_b = parse(&merge_source_entry(Some(&after_a), "github:b/b", Some(&b)).unwrap());
    let refs: Vec<String> = sources_of(&after_b)
        .iter()
        .map(|s| s["source"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(refs, vec!["github:base/base", "github:a/a", "github:b/b"]);
    // And the other order keeps all three too.
    let after_b2 = merge_source_entry(Some(&base), "github:b/b", Some(&b)).unwrap();
    let after_a2 = parse(&merge_source_entry(Some(&after_b2), "github:a/a", Some(&a)).unwrap());
    assert_eq!(sources_of(&after_a2).len(), 3);
}

#[test]
fn merge_refuses_a_corrupt_file_instead_of_overwriting_it() {
    let a = entry("github:a/a", "A");
    assert!(merge_source_entry(Some(b"{not json"), "github:a/a", Some(&a)).is_err());
    assert!(
        merge_source_entry(
            Some(br#"{"hew":1,"sources":"nope"}"#),
            "github:a/a",
            Some(&a)
        )
        .is_err()
    );
}

#[test]
fn merge_refuses_an_entry_that_names_a_different_source() {
    let a = entry("github:a/a", "A");
    assert!(merge_source_entry(None, "github:other/other", Some(&a)).is_err());
    assert!(merge_source_entry(None, "github:a/a", Some(&json!("not an object"))).is_err());
}

// --- the file on disk ---------------------------------------------------------

#[test]
fn sources_file_names_are_the_documented_ones() {
    assert_eq!(SOURCES_FILE, ".sources.json");
    assert_eq!(SOURCES_CACHE_DIR, ".sources");
}

#[test]
fn read_sources_is_none_when_the_file_is_absent() {
    let dir = scratch("read-none");
    assert_eq!(read_sources(&dir).unwrap(), None);
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn write_then_read_sources_round_trips_in_the_library_root() {
    let dir = scratch("roundtrip");
    let a = entry("github:a/a", "A");
    write_sources(&dir, "github:a/a", Some(&a)).unwrap();
    assert!(dir.root().join(".sources.json").is_file());
    let bytes = read_sources(&dir).unwrap().expect("file exists");
    let v = parse(&bytes);
    assert_eq!(v["hew"], json!(1));
    assert_eq!(sources_of(&v), vec![a]);
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn write_sources_re_reads_the_file_so_an_entry_written_meanwhile_survives() {
    let dir = scratch("reread");
    let a = entry("github:a/a", "A");
    let b = entry("github:b/b", "B");
    write_sources(&dir, "github:a/a", Some(&a)).unwrap();
    // Another process adds C directly to the file between our two writes.
    let c = entry("github:c/c", "C");
    let mut v = parse(&read_sources(&dir).unwrap().unwrap());
    v["sources"].as_array_mut().unwrap().push(c.clone());
    std::fs::write(
        dir.root().join(".sources.json"),
        serde_json::to_vec(&v).unwrap(),
    )
    .unwrap();
    write_sources(&dir, "github:b/b", Some(&b)).unwrap();
    let out = parse(&read_sources(&dir).unwrap().unwrap());
    assert_eq!(sources_of(&out), vec![a, c, b]);
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn write_sources_with_none_removes_the_entry() {
    let dir = scratch("remove");
    let a = entry("github:a/a", "A");
    let b = entry("github:b/b", "B");
    write_sources(&dir, "github:a/a", Some(&a)).unwrap();
    write_sources(&dir, "github:b/b", Some(&b)).unwrap();
    write_sources(&dir, "github:a/a", None).unwrap();
    assert_eq!(
        sources_of(&parse(&read_sources(&dir).unwrap().unwrap())),
        vec![b]
    );
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn write_sources_leaves_no_temp_file_behind_on_success() {
    let dir = scratch("no-tmp");
    write_sources(&dir, "github:a/a", Some(&entry("github:a/a", "A"))).unwrap();
    write_sources(&dir, "github:b/b", Some(&entry("github:b/b", "B"))).unwrap();
    assert_eq!(names_in(dir.root()), vec![".sources.json".to_string()]);
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn a_failed_write_never_leaves_a_partial_or_overwritten_file() {
    // A corrupt existing file: the write errors, the bytes are untouched,
    // and no temp sibling is left.
    let dir = scratch("corrupt");
    std::fs::write(dir.root().join(".sources.json"), b"{broken").unwrap();
    let err = write_sources(&dir, "github:a/a", Some(&entry("github:a/a", "A")));
    assert!(err.is_err());
    assert_eq!(
        std::fs::read(dir.root().join(".sources.json")).unwrap(),
        b"{broken"
    );
    assert_eq!(names_in(dir.root()), vec![".sources.json".to_string()]);
    let _ = std::fs::remove_dir_all(dir.root());

    // The target cannot be replaced (it is a directory): error, no stray temp.
    let dir = scratch("blocked");
    std::fs::create_dir(dir.root().join(".sources.json")).unwrap();
    assert!(write_sources(&dir, "github:a/a", Some(&entry("github:a/a", "A"))).is_err());
    assert_eq!(names_in(dir.root()), vec![".sources.json".to_string()]);
    let _ = std::fs::remove_dir_all(dir.root());
}

// --- never an item -------------------------------------------------------------

#[test]
fn the_sources_file_and_cache_never_appear_in_list_and_are_not_item_names() {
    let dir = scratch("not-item");
    write_sources(&dir, "github:a/a", Some(&entry("github:a/a", "A"))).unwrap();
    write_source_cache(&dir, "index-0123abcd", b"cached").unwrap();
    // Even a `.hew`-looking file inside the cache or a dot-named root file.
    std::fs::write(dir.root().join(".sources").join("stray.hew"), b"x").unwrap();
    std::fs::write(dir.root().join(".hidden.hew"), b"x").unwrap();
    assert!(list(&dir).is_empty(), "dot names are never items");
    for name in [
        ".sources.json",
        ".sources/x.hew",
        ".sources/index.json",
        "Components/.sources.json",
    ] {
        assert!(!valid_item_name(name), "{name}");
    }
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn an_item_can_never_be_written_over_the_sources_file_or_into_its_cache() {
    let dir = scratch("write-guard");
    for name in [
        ".sources.json",
        ".sources/x.hew",
        "Components/.sources.json",
        ".sources.json.hew",
    ] {
        assert!(write(&dir, name, b"x").is_err(), "{name}");
    }
    assert!(!dir.root().join(".sources.json").exists());
    assert!(!dir.root().join(".sources").exists());
    let _ = std::fs::remove_dir_all(dir.root());
}

// --- the cache ----------------------------------------------------------------

#[test]
fn cache_key_charset_is_lowercase_alnum_dash_underscore_1_to_96() {
    for ok in [
        "catalog",
        "index-0123abcdef",
        "thumb_9f",
        "a",
        &"a".repeat(96),
        "0123456789abcdef",
    ] {
        assert!(valid_source_cache_key(ok), "{ok}");
    }
    for bad in [
        "",
        &"a".repeat(97),
        ".",
        "..",
        "../x",
        "a/b",
        "a\\b",
        "a.b",
        ".hidden",
        "-lead",
        "_lead",
        "UPPER",
        "sp ace",
        "colon:x",
        "nul\0x",
        "x/",
        "é",
    ] {
        assert!(!valid_source_cache_key(bad), "{bad:?}");
    }
}

#[test]
fn cache_round_trips_under_dot_sources_and_misses_are_none() {
    let dir = scratch("cache");
    assert_eq!(read_source_cache(&dir, "catalog").unwrap(), None);
    write_source_cache(&dir, "catalog", b"one").unwrap();
    write_source_cache(&dir, "index-0123abcd", b"two").unwrap();
    assert_eq!(
        read_source_cache(&dir, "catalog").unwrap().as_deref(),
        Some(&b"one"[..])
    );
    assert_eq!(
        read_source_cache(&dir, "index-0123abcd")
            .unwrap()
            .as_deref(),
        Some(&b"two"[..])
    );
    assert_eq!(
        source_cache_path(&dir, "catalog"),
        dir.root().join(".sources").join("catalog")
    );
    assert_eq!(
        std::fs::read(dir.root().join(".sources/catalog")).unwrap(),
        b"one"
    );
    // Overwrite replaces; no temp files linger.
    write_source_cache(&dir, "catalog", b"three").unwrap();
    assert_eq!(
        read_source_cache(&dir, "catalog").unwrap().as_deref(),
        Some(&b"three"[..])
    );
    assert_eq!(
        names_in(&dir.root().join(".sources")),
        vec!["catalog", "index-0123abcd"]
    );
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn cache_rejects_traversal_and_bad_keys_without_touching_the_disk() {
    let dir = scratch("cache-bad");
    for bad in ["../escape", "a/b", "..", "", "UPPER", "a.b"] {
        let r = read_source_cache(&dir, bad);
        assert_eq!(
            r.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "{bad}"
        );
        let w = write_source_cache(&dir, bad, b"x");
        assert_eq!(
            w.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "{bad}"
        );
        let d = remove_source_cache(&dir, bad);
        assert_eq!(
            d.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "{bad}"
        );
    }
    assert!(!dir.root().join(".sources").exists());
    assert!(!dir.root().parent().unwrap().join("escape").exists());
    let _ = std::fs::remove_dir_all(dir.root());
}

#[test]
fn cache_remove_deletes_one_entry_and_tolerates_an_absent_one() {
    let dir = scratch("cache-remove");
    write_source_cache(&dir, "thumb-aa", b"1").unwrap();
    write_source_cache(&dir, "idx-bb", b"2").unwrap();
    remove_source_cache(&dir, "thumb-aa").unwrap();
    assert_eq!(read_source_cache(&dir, "thumb-aa").unwrap(), None);
    assert_eq!(
        read_source_cache(&dir, "idx-bb").unwrap().as_deref(),
        Some(&b"2"[..])
    );
    remove_source_cache(&dir, "thumb-aa").unwrap();
    remove_source_cache(&dir, "never-written").unwrap();
    let _ = std::fs::remove_dir_all(dir.root());
}
