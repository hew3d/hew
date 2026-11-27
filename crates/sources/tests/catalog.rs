//! 6.3: `catalog.json`.
mod common;
use common::SHA;
use serde_json::{Value, json};
use sources::{IndexError, Kind, Severity, parse_catalog};

fn source_entry() -> Value {
    json!({
        "source": "github:example/parts", "kind": "github", "commit": SHA[..40].to_string(),
        "name": "Shop Hardware", "description": "d", "author": "a", "homepage": "https://example.com",
        "license": "CC-BY-4.0", "licenseUrl": null, "items": 412, "bytes": 39_800_000u64,
        "categories": { "Components": 400, "Materials": 12 }, "stale": false,
        "snapshot": "sources/0123456789abcdef.json"
    })
}

fn catalog() -> Value {
    json!({ "hew": 1, "built": "2026-11-23T00:00:00Z", "sources": [source_entry()],
            "removed": [{ "source": "github:bad/actor", "severity": "critical",
                          "reason": "Malicious plugin.", "link": "https://example.com/advisory" }] })
}

fn parse(v: &Value) -> Result<sources::Catalog, IndexError> {
    parse_catalog(&serde_json::to_vec(v).unwrap())
}

#[test]
fn the_example_of_6_3_parses() {
    let c = parse(&catalog()).unwrap();
    assert_eq!(c.hew, 1);
    assert_eq!(c.built, "2026-11-23T00:00:00Z");
    assert_eq!(c.sources.len(), 1);
    let s = &c.sources[0];
    assert_eq!(s.source, "github:example/parts");
    assert_eq!(s.kind, Kind::Github);
    assert_eq!(s.commit.as_deref(), Some(&SHA[..40]));
    assert_eq!(s.items, 412);
    assert_eq!(s.bytes, 39_800_000);
    assert_eq!(s.categories["Components"], 400);
    assert!(!s.stale);
    assert_eq!(s.snapshot, "sources/0123456789abcdef.json");
    assert_eq!(c.removed.len(), 1);
    assert_eq!(c.removed[0].severity, Severity::Critical);
    assert_eq!(
        c.removed[0].link.as_deref(),
        Some("https://example.com/advisory")
    );
}

#[test]
fn an_empty_catalog_parses() {
    let c = parse(&json!({ "hew": 1, "built": "x", "sources": [], "removed": [] })).unwrap();
    assert!(c.sources.is_empty() && c.removed.is_empty());
}

#[test]
fn a_web_source_has_no_commit() {
    let mut e = source_entry();
    e["kind"] = json!("web");
    e["source"] = json!("web:https://example.com/p/hew.json");
    e.as_object_mut().unwrap().remove("commit");
    let c = parse(&json!({ "hew": 1, "built": "x", "sources": [e], "removed": [] })).unwrap();
    assert_eq!(c.sources[0].commit, None);
}

#[test]
fn removed_link_is_optional_and_both_severities_parse() {
    let c = parse(&json!({ "hew": 1, "built": "x", "sources": [], "removed": [
        { "source": "github:a/b", "severity": "removed", "reason": "Gone." },
        { "source": "github:c/d", "severity": "critical", "reason": "Bad.", "link": null } ] }))
    .unwrap();
    assert_eq!(c.removed[0].severity, Severity::Removed);
    assert_eq!(c.removed[0].link, None);
    assert_eq!(c.removed[1].severity, Severity::Critical);
}

#[test]
fn unknown_keys_are_ignored() {
    let mut v = catalog();
    v["future"] = json!(1);
    v["sources"][0]["future"] = json!([1]);
    v["removed"][0]["future"] = json!({});
    assert!(parse(&v).is_ok());
}

#[test]
fn catalog_hew_greater_than_known_is_too_new() {
    let mut v = catalog();
    v["hew"] = json!(2);
    let e = parse(&v).unwrap_err();
    assert!(matches!(e, IndexError::TooNew { .. }));
    assert_eq!(e.code(), "index_too_new");
}

#[test]
fn catalog_over_4_mb_is_too_large() {
    let mut v = catalog();
    v["built"] = json!("x".repeat(4 * 1024 * 1024 + 1));
    assert_eq!(parse(&v).unwrap_err(), IndexError::TooLarge);
    let junk = vec![b'x'; 4 * 1024 * 1024 + 1];
    assert_eq!(parse_catalog(&junk).unwrap_err(), IndexError::TooLarge);
}

#[test]
fn malformed_catalogs_are_invalid_not_panics() {
    for bytes in [
        &b""[..],
        b"{",
        b"[]",
        b"null",
        b"\xff",
        b"{\"hew\":1}",
        b"{\"hew\":\"1\",\"built\":\"x\",\"sources\":[],\"removed\":[]}",
    ] {
        match parse_catalog(bytes) {
            Err(IndexError::Invalid { item: _, reason }) => assert!(!reason.is_empty()),
            other => panic!("{bytes:?}: {other:?}"),
        }
    }
}

#[test]
fn required_fields_of_a_source_entry() {
    for key in [
        "source",
        "kind",
        "name",
        "items",
        "bytes",
        "categories",
        "stale",
        "snapshot",
    ] {
        let mut v = catalog();
        v["sources"][0].as_object_mut().unwrap().remove(key);
        assert!(
            matches!(parse(&v), Err(IndexError::Invalid { .. })),
            "missing {key}"
        );
    }
}

#[test]
fn wrong_types_in_a_source_entry_are_invalid() {
    for (key, bad) in [
        ("items", json!("412")),
        ("items", json!(-1)),
        ("bytes", json!(1.5)),
        ("stale", json!("no")),
        ("kind", json!("svn")),
        ("kind", json!("GitHub")),
        ("categories", json!([1])),
        ("categories", json!({ "Components": "x" })),
        ("name", json!(5)),
        ("snapshot", json!(5)),
    ] {
        let mut v = catalog();
        v["sources"][0][key] = bad.clone();
        assert!(
            matches!(parse(&v), Err(IndexError::Invalid { .. })),
            "{key}={bad}"
        );
    }
}

#[test]
fn removed_entries_need_a_known_severity_and_a_reason() {
    for bad in [
        json!({ "source": "github:a/b", "severity": "info", "reason": "x" }),
        json!({ "source": "github:a/b", "severity": "Removed", "reason": "x" }),
        json!({ "source": "github:a/b", "reason": "x" }),
        json!({ "source": "github:a/b", "severity": "removed" }),
        json!({ "severity": "removed", "reason": "x" }),
        json!("github:a/b"),
    ] {
        let v = json!({ "hew": 1, "built": "x", "sources": [], "removed": [bad.clone()] });
        assert!(
            matches!(parse(&v), Err(IndexError::Invalid { .. })),
            "{bad}"
        );
    }
}
