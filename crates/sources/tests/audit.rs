//! Audit specs (added after a review of the S1 implementation). Each test
//! names the finding it pins (H1, PORTS, M1, M2, L1 to L5, host ranges).
//! Tests labelled GUARD already pass and must keep passing; every other
//! test failed against the implementation it was written for.
//!
//! Rulings recorded here:
//! - Canonical form is explicit (`gitlab:https://<host>/...`,
//!   `forgejo:https://<host>/...`) when the host has no dot or carries a
//!   port; otherwise the existing shorthand (`gitlab:<host>/...`).
//! - `SourceRef.host` includes the port when one is given and not 443.
//! - Ref-less Forgejo-kind sources have no resolve request; their index and
//!   file URLs read `?ref=HEAD`.
mod common;
use common::*;
use proptest::prelude::*;
use serde_json::{Value, json};
use sources::{
    IndexError, Kind, SourceRef, file_url, index_url, is_public_host, parse_catalog, parse_index,
    parse_source_ref, resolve_url,
};

fn all(t: &str) -> Vec<SourceRef> {
    parse_source_ref(t).unwrap_or_else(|e| panic!("{t:?} refused: {e:?}"))
}
fn one(t: &str) -> SourceRef {
    let mut v = all(t);
    assert_eq!(v.len(), 1, "{t:?}: {v:?}");
    v.remove(0)
}
fn canons(t: &str) -> Vec<String> {
    all(t).into_iter().map(|r| r.canonical).collect()
}

// ---- H1: canonical forms re-parse to themselves ------------------------------

#[test]
fn h1_dotless_host_url_keeps_its_host_in_every_candidate() {
    let v = all("https://gitserver/g/p");
    assert_eq!(
        v.iter().map(|r| r.kind).collect::<Vec<_>>(),
        vec![Kind::Web, Kind::Forgejo, Kind::Gitlab]
    );
    assert_eq!(
        canons("https://gitserver/g/p"),
        vec![
            "web:https://gitserver/g/p/hew.json",
            "forgejo:https://gitserver/g/p",
            "gitlab:https://gitserver/g/p"
        ]
    );
    for r in &v {
        assert_eq!(r.host, "gitserver");
        let again = all(&r.canonical);
        assert_eq!(
            again,
            vec![r.clone()],
            "{} must re-parse to itself",
            r.canonical
        );
    }
}

#[test]
fn h1_explicit_gitlab_with_a_dotless_host_never_becomes_gitlab_com() {
    let r = one("gitlab:https://localhost/g/p");
    assert_eq!(r.host, "localhost");
    assert_eq!(r.canonical, "gitlab:https://localhost/g/p");
    assert_eq!(one(&r.canonical), r);
}

#[test]
fn h1_explicit_forgejo_with_a_dotless_host() {
    let r = one("forgejo:https://nas/o/r//d@v1");
    assert_eq!(r.host, "nas");
    assert_eq!(r.canonical, "forgejo:https://nas/o/r//d@v1");
    assert_eq!(one(&r.canonical), r);
}

#[test]
fn h1_dotted_hosts_keep_the_shorthand_canonical_form() {
    // GUARD: the existing canonical forms are unchanged for dotted hosts.
    assert_eq!(
        one("gitlab:https://git.example.org/g/p").canonical,
        "gitlab:git.example.org/g/p"
    );
    assert_eq!(
        one("forgejo:https://git.example.org/o/r").canonical,
        "forgejo:git.example.org/o/r"
    );
    assert_eq!(
        one("https://gitlab.com/g/p").canonical,
        "gitlab:gitlab.com/g/p"
    );
}

const HOSTS: &[&str] = &[
    "git.example.org",
    "gitserver",
    "localhost",
    "nas.local:8443",
    "host:8443",
    "GitServer",
    "example.com:443",
    "gitlab.com",
    "codeberg.org",
    "gitea.com",
    "a-b.c-d.example",
];
const TAILS: &[&str] = &["", "@v1", "//d", "//d/e@main"];

proptest! {
    /// Whatever the parser yields re-parses to the very same reference.
    #[test]
    fn h1_every_candidate_canonical_form_round_trips(
        host in prop::sample::select(HOSTS),
        a in "[A-Za-z0-9][A-Za-z0-9_-]{0,6}",
        b in "[A-Za-z0-9][A-Za-z0-9_-]{0,6}",
        form in 0usize..6,
        tail in prop::sample::select(TAILS),
    ) {
        let text = match form {
            0 => format!("https://{host}/{a}/{b}"),
            1 => format!("gitlab:https://{host}/{a}/{b}{tail}"),
            2 => format!("forgejo:https://{host}/{a}/{b}{tail}"),
            3 => format!("web:https://{host}/{a}/{b}"),
            4 => format!("gitlab:{host}/{a}/{b}{tail}"),
            _ => format!("forgejo:{host}/{a}/{b}{tail}"),
        };
        if let Ok(cands) = parse_source_ref(&text) {
            for r in cands {
                let again = parse_source_ref(&r.canonical);
                prop_assert_eq!(again, Ok(vec![r.clone()]), "input {} canonical {}", text, r.canonical);
            }
        }
    }
}

// ---- PORTS ---------------------------------------------------------------

#[test]
fn ports_in_web_and_unknown_host_urls() {
    assert_eq!(
        canons("https://nas.local:8443/a/b"),
        vec![
            "web:https://nas.local:8443/a/b/hew.json",
            "forgejo:https://nas.local:8443/a/b",
            "gitlab:https://nas.local:8443/a/b"
        ]
    );
    let w = one("web:https://nas.local:8443/p/hew.json");
    assert_eq!(w.canonical, "web:https://nas.local:8443/p/hew.json");
    assert_eq!(w.host, "nas.local:8443");
    assert_eq!(one(&w.canonical), w);
    assert_eq!(
        index_url(&w, None).as_str(),
        "https://nas.local:8443/p/hew.json"
    );
}

#[test]
fn ports_in_explicit_git_forms_use_the_explicit_canonical_form() {
    for (input, canon, host) in [
        (
            "gitlab:https://host:8443/g/p",
            "gitlab:https://host:8443/g/p",
            "host:8443",
        ),
        (
            "forgejo:https://host:8443/o/r",
            "forgejo:https://host:8443/o/r",
            "host:8443",
        ),
        (
            "gitlab:https://git.example.org:8443/g/p",
            "gitlab:https://git.example.org:8443/g/p",
            "git.example.org:8443",
        ),
        (
            "forgejo:https://git.example.org:8443/o/r//d@v",
            "forgejo:https://git.example.org:8443/o/r//d@v",
            "git.example.org:8443",
        ),
    ] {
        let r = one(input);
        assert_eq!(r.canonical, canon, "{input}");
        assert_eq!(r.host, host, "{input}");
        assert_eq!(one(canon), r, "{input} must re-parse to itself");
    }
}

#[test]
fn a_first_segment_with_a_valid_port_is_a_host_in_shorthand_forms() {
    let g = one("gitlab:nas.local:8443/g/p");
    assert_eq!(
        (g.host.as_str(), g.repo.as_str()),
        ("nas.local:8443", "g/p")
    );
    assert_eq!(g.canonical, "gitlab:https://nas.local:8443/g/p");
    let g = one("gitlab:host:8443/g/p");
    assert_eq!((g.host.as_str(), g.repo.as_str()), ("host:8443", "g/p"));
    let f = one("forgejo:host:8443/o/r");
    assert_eq!((f.host.as_str(), f.repo.as_str()), ("host:8443", "o/r"));
    assert_eq!(f.canonical, "forgejo:https://host:8443/o/r");
}

#[test]
fn the_default_port_443_is_dropped_from_the_canonical_form() {
    assert_eq!(
        canons("web:https://example.com:443/p"),
        vec!["web:https://example.com/p/hew.json"]
    );
    assert_eq!(one("https://github.com:443/o/r").canonical, "github:o/r");
    assert_eq!(
        one("gitlab:https://git.example.org:443/g/p").canonical,
        "gitlab:git.example.org/g/p"
    );
    assert_eq!(
        one("forgejo:https://git.example.org:443/o/r").canonical,
        "forgejo:git.example.org/o/r"
    );
    assert_eq!(
        one("https://example.com:443/a/b/hew.json").canonical,
        "web:https://example.com/a/b/hew.json"
    );
}

#[test]
fn bad_ports_are_refused() {
    // GUARD: passes today only because every port is refused. It must keep
    // passing once valid ports are accepted (the valid ones are above).
    for p in [
        "0", "65536", "99999", "100000", "abc", "", "-1", "+80", "8 0", "08443", "00443", "0443",
        "443a", "0x1bb",
    ] {
        for text in [
            format!("web:https://example.com:{p}/x"),
            format!("https://example.com:{p}/a/b"),
            format!("gitlab:https://git.example.org:{p}/g/p"),
            format!("forgejo:https://git.example.org:{p}/o/r"),
        ] {
            assert!(parse_source_ref(&text).is_err(), "accepted {text:?}");
        }
    }
    for text in [
        "gitlab:host:0/g/p",
        "gitlab:host:99999/g/p",
        "forgejo:host:abc/o/r",
        "forgejo:host:08443/o/r",
        "gitlab:host:/g/p",
    ] {
        assert!(parse_source_ref(text).is_err(), "accepted {text:?}");
    }
}

#[test]
fn valid_port_extremes_are_accepted() {
    assert_eq!(one("web:https://example.com:1/p").host, "example.com:1");
    assert_eq!(
        one("web:https://example.com:65535/p").host,
        "example.com:65535"
    );
}

#[test]
fn ipv6_literals_and_userinfo_stay_refused() {
    for text in [
        "https://[::1]/a/b",
        "web:https://[::1]:8443/p",
        "gitlab:https://[2001:db8::1]/g/p",
        "forgejo:https://[::1]/o/r",
        "https://[::ffff:10.0.0.1]/a/b",
        "https://user@nas.local:8443/a/b",
        "gitlab:https://u:p@host:8443/g/p",
    ] {
        assert!(parse_source_ref(text).is_err(), "accepted {text:?}");
    }
}

// ---- M1: `.git` stripping must not leave `.` or `..` ----------------------

#[test]
fn m1_a_repo_that_becomes_a_dot_segment_after_dot_git_is_refused() {
    for t in [
        "gitlab:g/...git",
        "gitlab:g/..git",
        "https://gitlab.com/g/..git",
        "https://gitlab.com/g/...git",
        "https://example.com/g/...git",
        "https://example.com/g/..git",
        "github:o/...git",
        "github:o/..git",
        "https://github.com/o/..git",
        "https://github.com/o/...git",
        "codeberg:o/...git",
        "codeberg:o/..git",
        "forgejo:git.example.org/o/...git",
        "https://codeberg.org/o/...git",
        "https://codeberg.org/o/..git",
        "gitlab:https://git.example.org/g/...git",
    ] {
        assert!(parse_source_ref(t).is_err(), "accepted {t:?}");
    }
}

// ---- M2: ref-less Forgejo reads HEAD -----------------------------------------

#[test]
fn m2_a_ref_less_forgejo_source_resolves_head() {
    for t in [
        "codeberg:o/r",
        "forgejo:git.example.org/o/r",
        "forgejo:gitea.com/gitea/tea",
        "https://codeberg.org/o/r",
        "forgejo:https://gitea.com/o/r//d",
    ] {
        let u = resolve_url(&one(t)).unwrap_or_else(|| panic!("{t}: no resolve url"));
        assert!(u.path().ends_with("/git/commits/HEAD"), "{t}: {u}");
    }
}

#[test]
fn m2_a_forgejo_source_with_a_ref_still_resolves() {
    // GUARD
    assert_eq!(
        resolve_url(&one("codeberg:codeberg/Community@main"))
            .unwrap()
            .as_str(),
        "https://codeberg.org/api/v1/repos/codeberg/Community/git/commits/main?stat=false&verification=false&files=false"
    );
    assert!(resolve_url(&one("forgejo:gitea.com/gitea/tea@main")).is_some());
}

#[test]
fn m2_ref_less_forgejo_urls_read_head_when_there_is_no_commit() {
    // GUARD: the implementation already reads HEAD; this pins it.
    let s = one("codeberg:codeberg/Community");
    let base = "https://codeberg.org/api/v1/repos/codeberg/Community/raw";
    assert_eq!(
        index_url(&s, None).as_str(),
        format!("{base}/hew.json?ref=HEAD")
    );
    assert_eq!(
        file_url(&s, "README.md", None).unwrap().as_str(),
        format!("{base}/README.md?ref=HEAD")
    );
    let sub = one("forgejo:gitea.com/gitea/tea//docs");
    assert_eq!(
        index_url(&sub, None).as_str(),
        "https://gitea.com/api/v1/repos/gitea/tea/raw/docs/hew.json?ref=HEAD"
    );
    // A commit still wins, and a given ref is still read.
    assert!(index_url(&s, Some("abc")).as_str().ends_with("?ref=abc"));
    assert!(
        index_url(&one("codeberg:o/r@dev"), None)
            .as_str()
            .ends_with("?ref=dev")
    );
}

#[test]
fn m2_github_and_gitlab_ref_less_behavior_is_unchanged() {
    // GUARD
    assert!(resolve_url(&one("github:o/r")).is_some());
    assert!(resolve_url(&one("gitlab:g/p")).is_some());
}

// ---- L1: a trailing dot on a host ----------------------------------------

#[test]
fn l1_trailing_dot_on_a_host_is_stripped() {
    assert_eq!(
        one("https://codeberg.org./o/r"),
        one("https://codeberg.org/o/r")
    );
    assert_eq!(
        one("https://codeberg.org./o/r").canonical,
        "forgejo:codeberg.org/o/r"
    );
    assert_eq!(
        one("https://gitea.com./o/r").canonical,
        "forgejo:gitea.com/o/r"
    );
    assert_eq!(one("https://github.com./o/r").canonical, "github:o/r");
    assert_eq!(
        one("https://gitlab.com./g/p").canonical,
        "gitlab:gitlab.com/g/p"
    );
    assert_eq!(
        one("gitlab:https://git.example.org./g/p").canonical,
        "gitlab:git.example.org/g/p"
    );
    assert_eq!(
        canons("https://example.com./a/b")[0],
        "web:https://example.com/a/b/hew.json"
    );
    assert_eq!(
        one("web:https://example.com./p").canonical,
        "web:https://example.com/p/hew.json"
    );
}

#[test]
fn l1_fullwidth_dot_is_a_dot_too() {
    assert_eq!(
        one("https://codeberg\u{3002}org/o/r").canonical,
        "forgejo:codeberg.org/o/r"
    );
    assert_eq!(
        one("https://codeberg.org\u{3002}/o/r").canonical,
        "forgejo:codeberg.org/o/r"
    );
    assert_eq!(one("https://github\u{3002}com/o/r").canonical, "github:o/r");
}

// ---- L2 / L3: licenseUrl, license ----------------------------------------

#[test]
fn l2_index_license_url_must_be_https_when_present() {
    assert!(
        parse(&with_top(
            |v| v["licenseUrl"] = json!("https://example.com/license")
        ))
        .is_ok()
    );
    assert!(parse(&with_top(|v| v["licenseUrl"] = Value::Null)).is_ok()); // GUARD
    for bad in [
        "http://example.com/l",
        "javascript:alert(1)",
        "not a url",
        "",
        "//example.com/l",
        "file:///etc/passwd",
        "ftp://example.com/l",
    ] {
        assert_header_invalid(&with_top(|v| v["licenseUrl"] = json!(bad)));
    }
}

#[test]
fn l2_catalog_license_url_must_be_https_when_present() {
    let cat = |lu: Value| {
        json!({ "hew": 1, "built": "x", "removed": [], "sources": [{
            "source": "github:o/r", "kind": "github", "name": "n", "items": 1, "bytes": 1,
            "categories": {}, "stale": false, "snapshot": "sources/a.json", "licenseUrl": lu }] })
    };
    let p = |v: Value| parse_catalog(&serde_json::to_vec(&v).unwrap());
    assert!(p(cat(json!("https://example.com/l"))).is_ok());
    assert!(p(cat(Value::Null)).is_ok()); // GUARD
    for bad in [
        "http://example.com/l",
        "javascript:alert(1)",
        "not a url",
        "",
    ] {
        assert!(
            matches!(p(cat(json!(bad))), Err(IndexError::Invalid { .. })),
            "{bad}"
        );
    }
}

#[test]
fn l3_guard_free_text_license_is_not_checked_at_parse() {
    // GUARD: license format is a catalog-check job, not parse_index's.
    for lic in [
        "Free text, any words",
        "not-an-spdx-id",
        "LicenseRef-x",
        "CC-BY-4.0 OR MIT",
        "",
    ] {
        assert!(
            parse(&with_top(|v| v["license"] = json!(lic))).is_ok(),
            "{lic:?}"
        );
        assert!(
            parse(&with_item(|i| i["license"] = json!(lic))).is_ok(),
            "{lic:?}"
        );
    }
}

// ---- L4: edge spaces in dir segments -----------------------------------------

#[test]
fn l4_spaces_at_the_edge_of_a_dir_segment_are_refused() {
    for t in [
        "github:o/r//d /",
        "github:o/r//d /x",
        "github:o/r// d",
        "github:o/r//a/ b",
        "github:o/r//a /b",
        "gitlab:g/p//d /",
        "gitlab:g/p//a/ b",
        "codeberg:o/r//d /x",
        "codeberg:o/r// d",
        "github:o/r//d \u{a0}/x",
    ] {
        assert!(parse_source_ref(t).is_err(), "accepted {t:?}");
    }
}

#[test]
fn l4_guard_interior_spaces_in_a_dir_still_round_trip() {
    // GUARD
    let r = one("github:o/r//my dir/sub dir");
    assert_eq!(r.dir, "my dir/sub dir");
    assert_eq!(one(&r.canonical), r);
}

// ---- L5: catalog entries -------------------------------------------------------

fn entry(f: impl FnOnce(&mut Value)) -> Result<sources::Catalog, IndexError> {
    let mut e = json!({ "source": "github:o/r", "kind": "github", "name": "n", "items": 1,
        "bytes": 1, "categories": { "Components": 1 }, "stale": false, "snapshot": "sources/a.json" });
    f(&mut e);
    parse_catalog(
        &serde_json::to_vec(&json!({ "hew": 1, "built": "x", "removed": [], "sources": [e] }))
            .unwrap(),
    )
}

#[test]
fn l5_a_web_source_has_no_commit() {
    assert!(
        entry(|e| {
            e["kind"] = json!("web");
            e["source"] = json!("web:https://example.com/p/hew.json");
            e["commit"] = json!("0123456789abcdef0123456789abcdef01234567");
        })
        .is_err()
    );
    // GUARD: no commit, or a null one, is fine on web; git kinds keep theirs.
    assert!(
        entry(|e| {
            e["kind"] = json!("web");
            e["source"] = json!("web:https://example.com/p/hew.json");
        })
        .is_ok()
    );
    assert!(
        entry(|e| {
            e["kind"] = json!("web");
            e["source"] = json!("web:https://example.com/p/hew.json");
            e["commit"] = Value::Null;
        })
        .is_ok()
    );
    assert!(entry(|e| e["commit"] = json!("0123456789abcdef0123456789abcdef01234567")).is_ok());
}

#[test]
fn l5_category_keys_must_be_known_categories() {
    for bad in [
        "Other",
        "components",
        "COMPONENTS",
        "Component",
        "",
        "Textures",
        "Components ",
    ] {
        assert!(
            matches!(
                entry(|e| e["categories"] = json!({ bad: 1 })),
                Err(IndexError::Invalid { .. })
            ),
            "{bad:?}"
        );
    }
    assert!(entry(|e| e["categories"] = json!({ "Components": 1, "Other": 2 })).is_err());
    // GUARD: the four names, any subset, are fine.
    assert!(
        entry(|e| e["categories"] =
            json!({ "Components": 1, "Materials": 2, "Models": 3, "Profiles": 4 }))
        .is_ok()
    );
    assert!(entry(|e| e["categories"] = json!({})).is_ok());
}

#[test]
fn l5_guard_kind_is_not_cross_checked_against_the_source_prefix() {
    // GUARD: the existing spec forbids this cross-check.
    assert!(
        entry(|e| {
            e["source"] = json!("gitlab:gitlab.com/g/p");
            e["kind"] = json!("github");
        })
        .is_ok()
    );
}

// ---- is_public_host: more special ranges -----------------------------------

#[test]
fn more_special_ranges_are_not_public() {
    for s in [
        "::ffff:0:127.0.0.1",
        "::ffff:0:10.0.0.1",
        "::ffff:0:192.168.1.1",
        "::ffff:0:169.254.169.254",
        "2001::1",
        "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        "2001:0:ffff:ffff:ffff:ffff:ffff:ffff",
        "2001:2::1",
        "2001:2:0:ffff::1",
        "2001:10::1",
        "2001:1f:ffff::1",
        "2001:20::1",
        "2001:2f::1",
        "fec0::1",
        "feff::1",
        "fecf:ffff::1",
        "3fff::1",
        "3fff:fff::1",
        "5f00::1",
        "5f00:ffff::1",
        "192.88.99.0",
        "192.88.99.1",
        "192.88.99.255",
    ] {
        assert!(
            !is_public_host(s.parse().unwrap()),
            "{s} must NOT be public"
        );
    }
}

#[test]
fn guard_neighbours_of_those_ranges_and_known_publics_stay_public() {
    // GUARD
    for s in [
        "8.8.8.8",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
        "192.88.98.255",
        "192.88.100.0",
        "4000::1",
        "5e00::1",
        "5f01::1",
    ] {
        assert!(is_public_host(s.parse().unwrap()), "{s} is public");
    }
}

// keep helpers referenced
#[allow(dead_code)]
fn _uses() {
    let _ = parse_index(b"");
}
