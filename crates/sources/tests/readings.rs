//! Specs that rest on a READING of silence or ambiguity in the design doc.
//! Each test names its question (Q-numbers match the S1 oracle report). If
//! the lead decides the other way, change the test HERE, deliberately;
//! the implementer must not.
mod common;
use common::*;
use serde_json::json;
use sources::{IndexError, RefError, parse_index, parse_source_ref};

fn one(t: &str) -> sources::SourceRef {
    parse_source_ref(t).unwrap().remove(0)
}

// Q1: item positions in `index_invalid` are 0-based (a Rust `usize` index).
#[test]
fn q1_item_position_is_zero_based() {
    let mut third = item(3);
    third["bytes"] = json!(0);
    let (_, pos) = assert_invalid(parse(&index(vec![item(1), item(2), third, item(4)])));
    assert_eq!(pos, Some(2));
    // A duplicate id or path is reported at the LATER item.
    let a = item(1);
    let mut b = item(2);
    b["id"] = a["id"].clone();
    assert_eq!(assert_invalid(parse(&index(vec![a, b]))).1, Some(1));
}

// Q2: `hew` greater than known wins over every other fault, because a newer
// index may legitimately have a shape this build cannot read.
#[test]
fn q2_too_new_takes_precedence_over_shape_errors() {
    for body in [
        json!({ "hew": 2, "items": "not an array" }),
        json!({ "hew": 2 }),
        json!({ "hew": 99, "name": "", "items": [{ "id": 1 }] }),
    ] {
        let e = parse_index(&serde_json::to_vec(&body).unwrap()).unwrap_err();
        assert!(matches!(e, IndexError::TooNew { .. }), "{body}: {e:?}");
    }
}

// Q3: optional fields of the wrong JSON type are violations (index_invalid),
// not silently dropped. (Only `plugins` is documented as lenient.)
#[test]
fn q3_optional_fields_of_the_wrong_type_are_invalid() {
    for key in ["description", "author", "license", "licenseUrl", "homepage"] {
        assert_header_invalid(&with_top(|v| v[key] = json!(5)));
        assert_header_invalid(&with_top(|v| v[key] = json!(["x"])));
    }
}

// Q4: `.hew` / `.png` suffixes are case-sensitive.
#[test]
fn q4_extensions_are_case_sensitive() {
    assert_item_invalid(&with_item(|i| i["path"] = json!("a.HEW")));
    assert_item_invalid(&with_item(|i| i["path"] = json!("a.Hew")));
    assert_item_invalid(&with_item(|i| i["thumbnail"] = json!("a.PNG")));
}

// Q5: control characters and NUL are refused in item paths.
#[test]
fn q5_control_characters_in_paths_are_refused() {
    for p in [
        "a\0b.hew",
        "a\nb.hew",
        "a\tb.hew",
        "a\u{7f}b.hew",
        "a\r.hew",
    ] {
        assert_item_invalid(&with_item(|i| i["path"] = json!(p)));
    }
}

// Q6 (ruled): percent-encoded dot segments, and any `%XX` that decodes to
// `.`, `/` or `\`, are refused in index paths at parse time.
#[test]
fn q6_percent_escapes_of_dot_slash_backslash_are_refused_in_index_paths() {
    for p in [
        "%2e%2e/x.hew",
        "a/%2E%2E/b.hew",
        ".%2e/x.hew",
        "%2e/x.hew",
        "a%2fb.hew",
        "a%2Fb.hew",
        "a%5cb.hew",
        "a%5Cb.hew",
        "a%2eb.hew",
        "%2e%2e%2fx.hew",
    ] {
        assert_item_invalid(&with_item(|i| i["path"] = json!(p)));
    }
}

// Q7: a `gitlab:` first segment containing a dot is a host; without a dot it
// is a group (shorthand for gitlab.com).
#[test]
fn q7_gitlab_host_vs_group() {
    assert_eq!(one("gitlab:git.example.org/g/p").host, "git.example.org");
    assert_eq!(one("gitlab:mygroup/p").host, "gitlab.com");
    assert_eq!(one("gitlab:mygroup/p").repo, "mygroup/p");
}

// Q8: the three candidates of an unknown host read the URL path as the
// repository path (two segments here); fewer than two segments leaves only
// the web candidate (a repo needs owner and name).
#[test]
fn q8_unknown_host_with_a_short_path_is_web_only() {
    for u in [
        "https://example.com",
        "https://example.com/",
        "https://example.com/solo",
    ] {
        let v = parse_source_ref(u).unwrap();
        assert_eq!(v.len(), 1, "{u}: {v:?}");
        assert_eq!(v[0].kind, sources::Kind::Web);
    }
}

// Q9: an unknown-host URL already ending in `hew.json` is unambiguous: web.
#[test]
fn q9_unknown_host_hew_json_url_is_web_only() {
    let v = parse_source_ref("https://example.com/a/b/hew.json").unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].canonical, "web:https://example.com/a/b/hew.json");
}

// Q10: surrounding whitespace in pasted text is trimmed.
#[test]
fn q10_pasted_whitespace_is_trimmed() {
    assert_eq!(one("  github:o/r \n").canonical, "github:o/r");
    assert_eq!(one("\thttps://github.com/o/r\n").canonical, "github:o/r");
    assert_eq!(parse_source_ref("   ").unwrap_err(), RefError::Empty);
}

// Q11: credentials in a URL are refused.
#[test]
fn q11_userinfo_in_urls_is_refused() {
    for u in [
        "https://user:pw@github.com/o/r",
        "https://user@example.com/p",
        "web:https://u:p@example.com/p",
        "gitlab:https://u@git.example.org/g/p",
    ] {
        assert!(parse_source_ref(u).is_err(), "{u}");
    }
}

// Q12: a plain scheme word is case-insensitive.
#[test]
fn q12_scheme_prefixes_are_case_insensitive() {
    assert_eq!(one("GitHub:o/r").canonical, "github:o/r");
    assert_eq!(one("CODEBERG:o/r").canonical, "forgejo:codeberg.org/o/r");
}

// Q13: a trailing `.git` on a repository is not part of its name.
#[test]
fn q13_dot_git_suffix_is_dropped() {
    assert_eq!(one("https://github.com/o/r.git").canonical, "github:o/r");
    assert_eq!(one("github:o/r.git").canonical, "github:o/r");
}

// Q14 (ruled): the catalog's per-source `source` must itself be a canonical
// reference (round-trips through parse_source_ref to itself), and `snapshot`
// must be a safe relative path ending `.json` (the app follows it
// without deriving it, so a hostile catalog must not steer a request).
#[test]
fn q14_catalog_source_is_canonical_and_snapshot_is_a_safe_json_path() {
    use sources::parse_catalog;
    let entry = |source: &str, snapshot: &str| {
        json!({ "hew": 1, "built": "x", "removed": [], "sources": [{
            "source": source, "kind": "github", "name": "n", "items": 1, "bytes": 1,
            "categories": {}, "stale": false, "snapshot": snapshot }] })
    };
    for good in [
        "github:o/r",
        "github:o/r@main",
        "gitlab:gitlab.com/g/p//d",
        "web:https://example.com/p/hew.json",
    ] {
        let ok = entry(good, "sources/0123456789abcdef.json");
        let r = parse_catalog(&serde_json::to_vec(&ok).unwrap());
        assert!(r.is_ok(), "{good}: {r:?}");
    }
    for (s, p) in [
        ("github:O/R", "sources/a.json"), // not canonical (case)
        ("not a ref", "sources/a.json"),
        ("github:o/r", "../x.json"),
        ("github:o/r", "/etc/passwd"),
        ("github:o/r", "https://evil/x.json"),
        ("github:o/r", "sources/../../x.json"),
        ("github:o/r", "sources\\a.json"),
        ("github:o/r", "sources/a.json?x=1"),
        ("github:o/r", ""),
        ("github:o/r", "sources/a.txt"),
        ("github:o/r", "sources/a"),
        ("github:o/r", "sources/.hidden.json"),
        ("github:o/r", "sources//a.json"),
        ("github:o/r", "sources/%2e%2e/a.json"),
        ("https://github.com/o/r", "sources/a.json"), // accepted input form, not canonical
        ("github:o/r/", "sources/a.json"),
    ] {
        let r = parse_catalog(&serde_json::to_vec(&entry(s, p)).unwrap());
        assert!(
            matches!(r, Err(IndexError::Invalid { .. })),
            "{s:?} {p:?}: {r:?}"
        );
    }
}

// Q15: a removed entry's `link`, when present, is an https URL.
#[test]
fn q15_removed_link_must_be_https() {
    use sources::parse_catalog;
    for link in ["http://example.com", "javascript:alert(1)", "not a url", ""] {
        let v = json!({ "hew": 1, "built": "x", "sources": [], "removed": [
            { "source": "github:a/b", "severity": "removed", "reason": "x", "link": link }] });
        assert!(
            parse_catalog(&serde_json::to_vec(&v).unwrap()).is_err(),
            "{link}"
        );
    }
}

// Q16: addresses that are not globally routable but are not named in the
// brief are also not public.
#[test]
fn q16_other_non_public_ranges() {
    use std::net::IpAddr;
    for s in [
        "100.64.0.1",
        "100.127.255.255",
        "0.1.2.3",
        "192.0.0.1",
        "198.18.0.1",
        "240.0.0.1",
        "64:ff9b::a00:1",
    ] {
        // 64:ff9b::/96 (NAT64) embeds an IPv4 address; 10.0.0.1 here.
        assert!(
            !sources::is_public_host(s.parse::<IpAddr>().unwrap()),
            "{s}"
        );
    }
}

// Q17 (ruled): an index item with a nil UUID is still a UUID.
#[test]
fn q17_nil_uuid_is_accepted() {
    let idx = parse(&with_item(|i| {
        i["id"] = json!("00000000-0000-0000-0000-000000000000")
    }))
    .unwrap();
    assert_eq!(idx.items.len(), 1);
}

// Q19 (ruled): `same_source_ignoring_ref` compares canonical forms with any
// `@<ref>` set aside.
#[test]
fn q19_same_source_ignoring_ref() {
    use sources::same_source_ignoring_ref as same;
    assert!(same(&one("github:o/r"), &one("github:o/r@main")));
    assert!(same(&one("github:o/r@a"), &one("github:o/r@b")));
    assert!(same(
        &one("github:o/r//d@a"),
        &one("https://github.com/O/R/tree/z/d")
    ));
    assert!(same(&one("github:o/r"), &one("github:o/r")));
    assert!(same(&one("gitlab:g/p@x"), &one("gitlab:gitlab.com/g/p")));
    assert!(same(
        &one("web:https://example.com/p"),
        &one("web:https://example.com/p/hew.json")
    ));
    assert!(
        !same(&one("github:o/r"), &one("github:o/r//d")),
        "dir is not a ref"
    );
    assert!(!same(&one("github:o/r@main"), &one("github:o/s@main")));
    assert!(!same(&one("github:o/r"), &one("gitlab:g/r")));
    assert!(
        !same(&one("gitlab:g/P"), &one("gitlab:g/p")),
        "only GitHub paths fold case"
    );
    assert!(!same(
        &one("codeberg:o/r"),
        &one("forgejo:git.example.org/o/r")
    ));
}
