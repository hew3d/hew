//! 6.1: one test per item row of the field table.
mod common;
use common::*;
use serde_json::{Value, json};

fn ok(f: impl FnOnce(&mut Value)) {
    parse(&with_item(f)).unwrap();
}
fn bad(f: impl FnOnce(&mut Value)) {
    assert_item_invalid(&with_item(f));
}

// ---- id ------------------------------------------------------------------

#[test]
fn id_is_required() {
    bad(|i| {
        i.as_object_mut().unwrap().remove("id");
    });
    bad(|i| i["id"] = Value::Null);
    bad(|i| i["id"] = json!(5));
}

#[test]
fn id_is_a_lowercase_hyphenated_uuid() {
    ok(|i| i["id"] = json!("0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77"));
}

#[test]
fn id_uppercase_uuid_is_refused() {
    bad(|i| i["id"] = json!("0B6F1C1E-5A2F-4C0F-9A53-1F0C8D1E2A77"));
    bad(|i| i["id"] = json!("0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2A77"));
}

#[test]
fn id_that_is_not_a_uuid_is_refused() {
    for s in [
        "",
        "x",
        "not-a-uuid",
        "0b6f1c1e5a2f4c0f9a531f0c8d1e2a77",
        "{0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77}",
        "urn:uuid:0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77",
        "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a7",
        "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a777",
        "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a7g",
        "0b6f1c1e_5a2f_4c0f_9a53_1f0c8d1e2a77",
        "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77 ",
        " 0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77",
        "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a77\n",
        "../../etc/passwd",
        "0b6f1c1e-5a2f-4c0f-9a53-1f0c8d1e2a7\u{e9}",
    ] {
        bad(|i| i["id"] = json!(s));
    }
}

#[test]
fn duplicate_ids_are_refused_at_the_second_item() {
    let a = item(1);
    let mut b = item(2);
    b["id"] = a["id"].clone();
    let (_, pos) = assert_invalid(parse(&index(vec![a, b])));
    assert!(pos.is_some(), "duplicate id names an item position");
}

#[test]
fn ids_equal_only_up_to_case_are_not_a_special_case() {
    // Uppercase is refused outright (above), so a case-variant pair is
    // refused as non-lowercase, never silently merged.
    let a = item(1);
    let mut b = item(2);
    b["id"] = json!(a["id"].as_str().unwrap().to_uppercase());
    assert_invalid(parse(&index(vec![a, b])));
}

// ---- path ----------------------------------------------------------------

#[test]
fn path_is_required_unique_and_ends_in_hew() {
    bad(|i| {
        i.as_object_mut().unwrap().remove("path");
    });
    bad(|i| i["path"] = Value::Null);
    bad(|i| i["path"] = json!(3));
    bad(|i| i["path"] = json!("Components/a.txt"));
    bad(|i| i["path"] = json!("Components/a"));
    bad(|i| i["path"] = json!("Components/a.hew.png"));
    ok(|i| i["path"] = json!("a.hew"));
    ok(|i| i["path"] = json!("Any/Folder/Layout/at all/a b.hew"));
    ok(|i| i["path"] = json!("a.b/c.d.hew"));
}

#[test]
fn duplicate_paths_are_refused() {
    let a = item(1);
    let mut b = item(2);
    b["path"] = a["path"].clone();
    let (_, pos) = assert_invalid(parse(&index(vec![a, b])));
    assert!(pos.is_some());
}

#[test]
fn every_hostile_path_is_refused_as_an_item_path() {
    for p in hostile_paths() {
        let shown: String = p.chars().take(60).collect();
        assert!(
            parse(&with_item(|i| i["path"] = json!(p))).is_err(),
            "accepted path {shown:?}"
        );
    }
}

#[test]
fn path_length_limit_is_1024_bytes_inclusive() {
    let at = format!("{}.hew", "a".repeat(1_024 - 4));
    assert_eq!(at.len(), 1_024);
    ok(|i| i["path"] = json!(at));
    let over = format!("{}.hew", "a".repeat(1_024 - 4 + 1));
    bad(|i| i["path"] = json!(over));
    // 300 two-byte characters + ".hew" = 604 bytes: fine.
    ok(|i| i["path"] = json!(format!("{}.hew", "\u{e9}".repeat(300))));
    // 600 two-byte characters = 1,204 bytes: over, though under 1,024 characters.
    bad(|i| i["path"] = json!(format!("{}.hew", "\u{e9}".repeat(600))));
}

// ---- name ----------------------------------------------------------------

#[test]
fn item_name_is_required_one_to_200_characters() {
    bad(|i| {
        i.as_object_mut().unwrap().remove("name");
    });
    bad(|i| i["name"] = json!(""));
    bad(|i| i["name"] = Value::Null);
    bad(|i| i["name"] = json!(1));
    ok(|i| i["name"] = json!("x".repeat(200)));
    bad(|i| i["name"] = json!("x".repeat(201)));
    ok(|i| i["name"] = json!("\u{e9}".repeat(200)));
    bad(|i| i["name"] = json!("\u{e9}".repeat(201)));
}

// ---- category ------------------------------------------------------------

#[test]
fn category_is_one_of_four_in_exact_case() {
    for c in ["Components", "Materials", "Models", "Profiles"] {
        ok(|i| i["category"] = json!(c));
    }
    for c in [
        "components",
        "COMPONENTS",
        "component",
        "Component",
        "material",
        "Material",
        "model",
        "Model",
        "profile",
        "Profile",
        "materials",
        "models",
        "profiles",
        "Components ",
        " Models",
        "Other",
        "",
        "Textures",
    ] {
        bad(|i| i["category"] = json!(c));
    }
    bad(|i| {
        i.as_object_mut().unwrap().remove("category");
    });
    bad(|i| i["category"] = Value::Null);
    bad(|i| i["category"] = json!(1));
}

#[test]
fn category_maps_to_the_enum() {
    use sources::Category::*;
    for (s, c) in [
        ("Components", Components),
        ("Materials", Materials),
        ("Models", Models),
        ("Profiles", Profiles),
    ] {
        let idx = parse(&with_item(|i| i["category"] = json!(s))).unwrap();
        assert_eq!(idx.items[0].category, c);
    }
}

// ---- sha256 --------------------------------------------------------------

#[test]
fn sha256_is_64_lowercase_hex() {
    ok(|i| i["sha256"] = json!("a".repeat(64)));
    ok(|i| i["sha256"] = json!("0".repeat(64)));
    bad(|i| i["sha256"] = json!("A".repeat(64)));
    bad(|i| i["sha256"] = json!(format!("{}A", "a".repeat(63))));
    bad(|i| i["sha256"] = json!("a".repeat(63)));
    bad(|i| i["sha256"] = json!("a".repeat(65)));
    bad(|i| i["sha256"] = json!(format!("{}g", "a".repeat(63))));
    bad(|i| i["sha256"] = json!(format!("sha256:{}", "a".repeat(64))));
    bad(|i| i["sha256"] = json!(""));
    bad(|i| i["sha256"] = json!(format!("{} ", "a".repeat(63))));
    bad(|i| i["sha256"] = Value::Null);
    bad(|i| {
        i.as_object_mut().unwrap().remove("sha256");
    });
}

// ---- bytes ---------------------------------------------------------------

#[test]
fn bytes_is_an_integer_from_1_to_67108864() {
    ok(|i| i["bytes"] = json!(1));
    ok(|i| i["bytes"] = json!(67_108_864u64));
    bad(|i| i["bytes"] = json!(0));
    bad(|i| i["bytes"] = json!(67_108_865u64));
    bad(|i| i["bytes"] = json!(-1));
    bad(|i| i["bytes"] = json!(u64::MAX));
    bad(|i| i["bytes"] = json!(1.5));
    bad(|i| i["bytes"] = json!("100"));
    bad(|i| i["bytes"] = Value::Null);
    bad(|i| {
        i.as_object_mut().unwrap().remove("bytes");
    });
}

// ---- keywords ------------------------------------------------------------

#[test]
fn keywords_are_optional_at_most_32_each_at_most_64_characters() {
    let idx = parse(&with_item(|_| {})).unwrap();
    assert!(idx.items[0].keywords.is_empty());
    ok(|i| i["keywords"] = json!([]));
    ok(|i| i["keywords"] = json!((0..32).map(|n| format!("k{n}")).collect::<Vec<_>>()));
    bad(|i| i["keywords"] = json!((0..33).map(|n| format!("k{n}")).collect::<Vec<_>>()));
    ok(|i| i["keywords"] = json!(["k".repeat(64)]));
    bad(|i| i["keywords"] = json!(["k".repeat(65)]));
    ok(|i| i["keywords"] = json!(["\u{e9}".repeat(64)]));
    bad(|i| i["keywords"] = json!(["\u{e9}".repeat(65)]));
    bad(|i| i["keywords"] = json!([1, 2]));
    bad(|i| i["keywords"] = json!("hinge"));
    bad(|i| i["keywords"] = json!([null]));
}

// ---- collection and license ----------------------------------------------

#[test]
fn collection_and_item_license_are_optional_strings() {
    for key in ["collection", "license"] {
        ok(|i| i[key] = json!("Hinges"));
        ok(|i| i[key] = Value::Null);
        bad(|i| i[key] = json!(5));
        bad(|i| i[key] = json!(["a"]));
    }
    let idx = parse(&with_item(|i| i["license"] = json!("MIT"))).unwrap();
    assert_eq!(idx.items[0].license.as_deref(), Some("MIT"));
}

// ---- thumbnail -----------------------------------------------------------

#[test]
fn thumbnail_is_optional_and_a_png_path() {
    ok(|i| i["thumbnail"] = Value::Null);
    ok(|i| i["thumbnail"] = json!(format!("thumbnails/{SHA}.png")));
    ok(|i| i["thumbnail"] = json!("t.png"));
    bad(|i| i["thumbnail"] = json!("thumbnails/x.jpg"));
    bad(|i| i["thumbnail"] = json!("thumbnails/x"));
    bad(|i| i["thumbnail"] = json!("thumbnails/x.hew"));
    bad(|i| i["thumbnail"] = json!("thumbnails/x.png.exe"));
    bad(|i| i["thumbnail"] = json!(5));
    bad(|i| i["thumbnail"] = json!(""));
}

#[test]
fn thumbnail_follows_the_same_path_rules() {
    for p in hostile_paths() {
        // Swap the extension so only the path rule can reject it.
        let t = match p.strip_suffix(".hew") {
            Some(stem) => format!("{stem}.png"),
            None => p.clone(),
        };
        let shown: String = t.chars().take(60).collect();
        assert!(
            parse(&with_item(|i| i["thumbnail"] = json!(t))).is_err(),
            "accepted thumbnail {shown:?}"
        );
    }
}

#[test]
fn thumbnail_length_limit_is_1024_bytes_inclusive() {
    ok(|i| i["thumbnail"] = json!(format!("{}.png", "a".repeat(1_024 - 4))));
    bad(|i| i["thumbnail"] = json!(format!("{}.png", "a".repeat(1_024 - 3))));
}
