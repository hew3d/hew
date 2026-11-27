//! 6.1: the header rows of the field table, the size/count limits, and the
//! shape of the document as a whole.
mod common;
use common::*;
use serde_json::{Value, json};
use sources::{IndexError, parse_index};

// ---- `hew` -----------------------------------------------------------

#[test]
fn minimal_index_parses() {
    let idx = parse(&json!({ "hew": 1, "name": "N", "items": [] })).unwrap();
    assert_eq!(idx.hew, 1);
    assert_eq!(idx.name, "N");
    assert!(idx.items.is_empty());
    assert!(idx.plugins.is_empty());
    assert_eq!(idx.description, None);
    assert_eq!(idx.license, None);
}

#[test]
fn full_example_of_6_1_parses_into_its_fields() {
    let v = json!({
      "hew": 1, "name": "Shop Hardware",
      "description": "Hinges, slides and fasteners at real sizes.",
      "author": "Example Maker", "homepage": "https://example.com/parts",
      "license": "CC-BY-4.0", "licenseUrl": null,
      "items": [{
        "id": "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77",
        "path": "Components/butt-hinge-0b6f1c.hew",
        "name": "Butt hinge 75 mm", "category": "Components",
        "keywords": ["hinge", "brass"], "collection": "Hinges",
        "sha256": SHA, "bytes": 48213,
        "thumbnail": format!("thumbnails/{SHA}.png"), "license": null }],
      "plugins": [] });
    let idx = parse(&v).unwrap();
    assert_eq!(idx.author.as_deref(), Some("Example Maker"));
    assert_eq!(idx.homepage.as_deref(), Some("https://example.com/parts"));
    assert_eq!(idx.license.as_deref(), Some("CC-BY-4.0"));
    assert_eq!(idx.license_url, None);
    let it = &idx.items[0];
    assert_eq!(it.id, "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77");
    assert_eq!(it.path, "Components/butt-hinge-0b6f1c.hew");
    assert_eq!(it.name, "Butt hinge 75 mm");
    assert_eq!(it.keywords, vec!["hinge", "brass"]);
    assert_eq!(it.collection.as_deref(), Some("Hinges"));
    assert_eq!(it.bytes, 48213);
    assert_eq!(
        it.thumbnail.as_deref(),
        Some(&*format!("thumbnails/{SHA}.png"))
    );
    assert_eq!(it.license, None);
}

#[test]
fn hew_greater_than_known_is_index_too_new() {
    for n in [2u64, 3, 100, u64::MAX] {
        let e = parse(&with_top(|v| v["hew"] = json!(n))).unwrap_err();
        assert!(matches!(e, IndexError::TooNew { .. }), "hew {n}: {e:?}");
        assert_eq!(e.code(), "index_too_new");
    }
}

#[test]
fn hew_is_a_required_integer() {
    assert_header_invalid(&json!({ "name": "N", "items": [] }));
    for bad in [
        json!("1"),
        json!(1.5),
        json!(null),
        json!(true),
        json!(0),
        json!(-1),
        json!([1]),
    ] {
        assert_header_invalid(&with_top(|v| v["hew"] = bad.clone()));
    }
}

// ---- `name` ----------------------------------------------------------

#[test]
fn name_is_required_one_to_200_characters() {
    assert_header_invalid(&json!({ "hew": 1, "items": [] }));
    assert_header_invalid(&with_top(|v| v["name"] = json!("")));
    assert_header_invalid(&with_top(|v| v["name"] = json!(null)));
    assert_header_invalid(&with_top(|v| v["name"] = json!(7)));
    assert!(parse(&with_top(|v| v["name"] = json!("x"))).is_ok());
    assert!(parse(&with_top(|v| v["name"] = json!("x".repeat(200)))).is_ok());
    assert_header_invalid(&with_top(|v| v["name"] = json!("x".repeat(201))));
}

#[test]
fn name_length_counts_characters_not_bytes() {
    assert!(parse(&with_top(|v| v["name"] = json!("\u{e9}".repeat(200)))).is_ok());
    assert_header_invalid(&with_top(|v| v["name"] = json!("\u{e9}".repeat(201))));
}

// ---- optional header fields -------------------------------------------

#[test]
fn optional_header_fields_may_be_absent_or_null() {
    for key in ["description", "author", "homepage", "license", "licenseUrl"] {
        assert!(
            parse(&with_top(|v| v[key] = Value::Null)).is_ok(),
            "{key} null"
        );
        assert!(
            parse(&with_top(|v| {
                v.as_object_mut().unwrap().remove(key);
            }))
            .is_ok()
        );
    }
}

#[test]
fn license_is_optional_in_the_format() {
    // "Optional in the format, required for listing": listing is a catalog
    // check, not a parse failure.
    let idx = parse(&with_top(|v| {
        v.as_object_mut().unwrap().remove("license");
    }))
    .unwrap();
    assert_eq!(idx.license, None);
}

#[test]
fn homepage_must_be_an_https_url() {
    assert!(parse(&with_top(|v| v["homepage"] = json!("https://example.com/p"))).is_ok());
    for bad in [
        "http://example.com",
        "ftp://example.com",
        "javascript:alert(1)",
        "not a url",
        "",
        "//example.com",
        "example.com",
        "file:///etc/passwd",
    ] {
        assert_header_invalid(&with_top(|v| v["homepage"] = json!(bad)));
    }
}

// ---- `items` -----------------------------------------------------------

#[test]
fn items_is_required_and_an_array() {
    assert_header_invalid(&json!({ "hew": 1, "name": "N" }));
    for bad in [json!(null), json!({}), json!("x"), json!(3)] {
        assert_header_invalid(&with_top(|v| v["items"] = bad.clone()));
    }
}

#[test]
fn an_item_that_is_not_an_object_is_invalid_at_its_position() {
    let v = index(vec![item(0), json!("nope")]);
    assert!(assert_invalid(parse(&v)).1.is_some());
    let v = index(vec![json!(null)]);
    assert!(assert_invalid(parse(&v)).1.is_some());
}

fn n_items(n: u32) -> Value {
    // Short fixed-width paths keep 20,001 items well under 4,000,000 bytes.
    index(
        (0..n)
            .map(|i| {
                json!({ "id": uuid(i), "path": format!("p{i:05}.hew"), "name": "n",
                "category": "Models", "sha256": SHA, "bytes": 1 })
            })
            .collect(),
    )
}

#[test]
fn exactly_20000_items_are_accepted() {
    let v = n_items(20_000);
    assert!(
        serde_json::to_vec(&v).unwrap().len() < 4_000_000,
        "fixture must be under the size cap"
    );
    assert_eq!(parse(&v).unwrap().items.len(), 20_000);
}

#[test]
fn more_than_20000_items_are_invalid() {
    let v = n_items(20_001);
    assert!(
        serde_json::to_vec(&v).unwrap().len() < 4_000_000,
        "fixture must be under the size cap"
    );
    assert_header_invalid(&v);
}

// ---- size ---------------------------------------------------------------

#[test]
fn an_index_over_4_mb_is_index_too_large() {
    let mut v = index(vec![item(1)]);
    v["description"] = json!("d".repeat(4 * 1024 * 1024 + 1));
    let e = parse(&v).unwrap_err();
    assert_eq!(e, IndexError::TooLarge);
    assert_eq!(e.code(), "index_too_large");
}

/// A valid index serialized to exactly `target` bytes (padded in `description`).
fn index_of_exactly(target: usize) -> Vec<u8> {
    let mut v = index(vec![item(1)]);
    v["description"] = json!("");
    let base = serde_json::to_vec(&v).unwrap().len();
    v["description"] = json!("d".repeat(target - base));
    let bytes = serde_json::to_vec(&v).unwrap();
    assert_eq!(bytes.len(), target);
    bytes
}

#[test]
fn the_cap_is_exactly_4_194_304_bytes() {
    assert!(parse_index(&index_of_exactly(4_194_304)).is_ok());
    let e = parse_index(&index_of_exactly(4_194_305)).unwrap_err();
    assert_eq!(e, IndexError::TooLarge);
    assert_eq!(e.code(), "index_too_large");
}

#[test]
fn size_is_checked_before_parsing() {
    // Not JSON at all, but too big: the size answer wins.
    let junk = vec![b'x'; 4 * 1024 * 1024 + 1];
    assert_eq!(parse_index(&junk).unwrap_err(), IndexError::TooLarge);
}

#[test]
fn a_large_index_under_the_cap_is_accepted() {
    let mut v = index(vec![item(1)]);
    v["description"] = json!("d".repeat(3_900_000));
    assert!(parse(&v).is_ok());
}

// ---- plugins and unknown keys ---------------------------------------------

#[test]
fn malformed_plugins_entries_are_ignored_not_errors() {
    let v = with_top(|v| {
        v["plugins"] = json!([
            1, "x", null, true, [], {}, { "id": 5 }, { "id": "a" },
            { "id": "a", "version": 1, "package": "p.hewplugin" }
        ])
    });
    let idx = parse(&v).unwrap();
    assert!(
        idx.plugins.is_empty(),
        "every entry above is the wrong shape: {:?}",
        idx.plugins
    );
    assert_eq!(idx.items.len(), 1);
}

#[test]
fn a_well_shaped_plugin_entry_is_kept() {
    let v = with_top(|v| {
        v["plugins"] = json!([
            { "id": "com.example.cutlist", "version": "1.2.0", "package": "plugins/cutlist.hewplugin",
              "sha256": SHA, "bytes": 1234, "hew": 1, "signature": "c2ln", "key": "a2V5",
              "futureKey": true },
            7
        ])
    });
    let idx = parse(&v).unwrap();
    assert_eq!(idx.plugins.len(), 1);
    let p = &idx.plugins[0];
    assert_eq!(p.id, "com.example.cutlist");
    assert_eq!(p.version, "1.2.0");
    assert_eq!(p.package, "plugins/cutlist.hewplugin");
    assert_eq!(p.sha256, SHA);
    assert_eq!(p.bytes, 1234);
    assert_eq!(p.hew, 1);
    assert_eq!(p.signature, "c2ln");
    assert_eq!(p.key, "a2V5");
}

#[test]
fn a_plugins_value_that_is_not_an_array_is_ignored_too() {
    // "An entry of the wrong shape is ignored, not an error": the section
    // never lets `plugins` fail an otherwise good index.
    for bad in [json!("x"), json!(3), json!({}), json!(null)] {
        let idx = parse(&with_top(|v| v["plugins"] = bad.clone())).unwrap();
        assert!(idx.plugins.is_empty());
    }
}

#[test]
fn unknown_keys_are_ignored_everywhere() {
    let mut v = with_top(|v| {
        v["extra"] = json!({ "deep": [1, 2] });
        v["plugins"] = json!([{ "unknown": 1 }]);
    });
    v["items"][0]["futureField"] = json!([1, 2, 3]);
    let idx = parse(&v).unwrap();
    assert_eq!(idx.items.len(), 1);
}

// ---- the document as a whole -------------------------------------------

#[test]
fn not_an_object_or_not_json_is_invalid_not_a_panic() {
    for bytes in [
        &b""[..],
        b"{",
        b"[]",
        b"null",
        b"3",
        b"\"x\"",
        b"{\"hew\":1",
        b"\xff\xfe\x00\x01",
        b"{\"hew\":1,\"name\":\"N\",\"items\":[]} trailing",
    ] {
        assert_header_invalid_bytes(bytes);
    }
}

fn assert_header_invalid_bytes(bytes: &[u8]) {
    match parse_index(bytes) {
        Err(IndexError::Invalid { item: None, .. }) => {}
        other => panic!("{bytes:?}: expected header index_invalid, got {other:?}"),
    }
}

#[test]
fn deeply_nested_json_is_refused_without_overflowing_the_stack() {
    let mut s = String::from("{\"hew\":1,\"name\":\"N\",\"items\":[],\"x\":");
    s.push_str(&"[".repeat(200_000));
    s.push_str(&"]".repeat(200_000));
    s.push('}');
    // Must return (Ok or an error), never crash. Under the cap, so not TooLarge.
    let r = parse_index(s.as_bytes());
    assert!(!matches!(r, Err(IndexError::TooLarge)));
    assert!(r.is_err(), "pathological nesting is refused");
}

#[test]
fn error_codes_have_stable_machine_names() {
    assert_eq!(IndexError::TooLarge.code(), "index_too_large");
    assert_eq!(IndexError::TooNew { found: 2 }.code(), "index_too_new");
    assert_eq!(
        IndexError::Invalid {
            reason: "x".into(),
            item: None
        }
        .code(),
        "index_invalid"
    );
    // And the parser actually produces them (the checks above are constants).
    assert_eq!(parse_index(b"{").unwrap_err().code(), "index_invalid");
}
