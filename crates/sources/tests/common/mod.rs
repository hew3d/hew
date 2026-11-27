//! Shared helpers for the executable specs. Not part of the oracle's claims.
#![allow(dead_code)]

use serde_json::{Value, json};
use sources::{IndexError, SourceIndex, parse_index};

pub const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A lowercase hyphenated UUID built from `n` (unique per `n`).
pub fn uuid(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

/// A fully valid item with every optional field absent; unique id and path
/// per `n`.
pub fn item(n: u32) -> Value {
    json!({
        "id": uuid(n),
        "path": format!("Components/item-{n}.hew"),
        "name": format!("Item {n}"),
        "category": "Components",
        "sha256": SHA,
        "bytes": 100
    })
}

/// A valid index holding `items`.
pub fn index(items: Vec<Value>) -> Value {
    json!({ "hew": 1, "name": "Shop Hardware", "items": items })
}

pub fn parse(v: &Value) -> Result<SourceIndex, IndexError> {
    parse_index(&serde_json::to_vec(v).unwrap())
}

/// The valid one-item index with `item` mutated by `f`.
pub fn with_item(f: impl FnOnce(&mut Value)) -> Value {
    let mut it = item(1);
    f(&mut it);
    index(vec![it])
}

/// The valid index with the top level mutated by `f`.
pub fn with_top(f: impl FnOnce(&mut Value)) -> Value {
    let mut v = index(vec![item(1)]);
    f(&mut v);
    v
}

#[track_caller]
pub fn assert_invalid(r: Result<SourceIndex, IndexError>) -> (String, Option<usize>) {
    match r {
        Err(IndexError::Invalid { reason, item }) => {
            assert!(!reason.is_empty(), "index_invalid must carry a reason");
            (reason, item)
        }
        other => panic!("expected index_invalid, got {other:?}"),
    }
}

/// Invalid, and the fault is attributed to some item position. (The exact
/// numbering base is asserted only in `readings.rs`.)
#[track_caller]
pub fn assert_item_invalid(v: &Value) {
    let (_, item) = assert_invalid(parse(v));
    assert!(item.is_some(), "fault is in an item: {v}");
}

/// Invalid, and the fault is in the header (no item position).
#[track_caller]
pub fn assert_header_invalid(v: &Value) {
    let (_, item) = assert_invalid(parse(v));
    assert_eq!(item, None, "fault is not an item's: {v}");
}

/// Relative paths that must never be accepted as an item `path`, a
/// `thumbnail`, or a `file_url` argument (6.1 `path` rule). Each ends in
/// `.hew` so that the extension rule is not what rejects it.
pub fn hostile_paths() -> Vec<String> {
    let mut v: Vec<String> = [
        "../x.hew",
        "a/../b.hew",
        "a/b/../../../etc/x.hew",
        "a/b/..",
        "..",
        "/etc/x.hew",
        "/x.hew",
        "a\\b.hew",
        "..\\x.hew",
        "C:/x.hew",
        "a:b.hew",
        "a?b.hew",
        "a/b.hew?x=1",
        "a#b.hew",
        "a/b.hew#frag",
        ".hidden/x.hew",
        "a/.x.hew",
        ".hew",
        "./x.hew",
        "a/./b.hew",
        "a//b.hew",
        "a/b.hew/",
        "//evil/x.hew",
        "https://evil/x.hew",
        "http://evil/x.hew",
        "file:///etc/x.hew",
        "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // Over 1,024 bytes by one.
    v.push(format!("{}.hew", "a".repeat(1_024 - 4 + 1)));
    // Under 1,024 characters but over 1,024 bytes (2-byte characters).
    v.push(format!("{}.hew", "\u{e9}".repeat(600)));
    v
}
