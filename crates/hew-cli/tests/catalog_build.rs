//! `hew-cli catalog build` (docs/design/shared-library-sources.md 6.3, 6.4,
//! 7.6; acceptance lines 54 and 55). The oracle for package S4, build half:
//! every spec fails until `hew_cli::catalog` is implemented.

mod common;

use common::Fixture;
use hew_cli::catalog::{
    FetchPolicy, HttpFetcher, auth_header_for, build_catalog, resolve_previous, snapshot_slug,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BUILT: &str = "2026-11-23T12:00:00Z";

fn policy(gap_ms: u64) -> FetchPolicy {
    FetchPolicy::new(Duration::from_millis(gap_ms), Duration::from_secs(10)).with_loopback()
}

fn build(
    checkout: &Path,
    out: &Path,
    previous: Option<&Path>,
    p: &FetchPolicy,
    built: &str,
) -> hew_cli::catalog::BuildReport {
    build_catalog(
        checkout,
        out,
        &HttpFetcher::new(p, None),
        p,
        previous,
        built,
    )
    .unwrap()
}

fn catalog_json(out: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(out.join("catalog.json")).unwrap()).unwrap()
}

fn two() -> (Fixture, Fixture) {
    (
        Fixture::new("First", Some("CC0-1.0")),
        Fixture::new("Second", Some("MIT")),
    )
}

fn entry<'a>(cat: &'a serde_json::Value, reference: &str) -> &'a serde_json::Value {
    cat["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["source"] == reference)
        .unwrap_or_else(|| panic!("no entry for {reference} in {cat}"))
}

// ------------------------------------------- acceptance line 54

#[test]
fn two_sources_write_two_entries_and_two_snapshots() {
    let (a, b) = two();
    let co = common::checkout(&[a.reference(), b.reference()], &[], serde_json::json!([]));
    let out = common::scratch("out");
    let report = build(&co, &out, None, &policy(0), BUILT);
    assert_eq!(report.fresh.len(), 2);
    assert!(report.stale.is_empty() && report.removed.is_empty());

    let cat = catalog_json(&out);
    assert_eq!(cat["sources"].as_array().unwrap().len(), 2);
    for (f, name) in [(&a, "First"), (&b, "Second")] {
        let e = entry(&cat, &f.reference());
        let slug = snapshot_slug(&f.reference());
        assert_eq!(e["snapshot"], format!("sources/{slug}.json"));
        assert_eq!(e["name"], name);
        assert_eq!(e["kind"], "web");
        assert_eq!(e["stale"], false);
        assert!(e["commit"].is_null(), "a web source has no commit: {e}");
        assert_eq!(e["items"], 2);
        let bytes: u64 = f.index["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["bytes"].as_u64().unwrap())
            .sum();
        assert_eq!(e["bytes"], bytes);
        assert_eq!(e["categories"]["Components"], 1);
        assert_eq!(e["categories"]["Models"], 1);
        assert!(out.join("sources").join(format!("{slug}.json")).is_file());
    }
    assert_eq!(std::fs::read_dir(out.join("sources")).unwrap().count(), 2);
    assert_eq!(cat["built"], BUILT);
    assert_eq!(cat["hew"], 1);
    assert_eq!(cat["removed"], serde_json::json!([]));
}

#[test]
fn the_slug_is_the_first_sixteen_hex_of_the_sha256_of_the_reference() {
    let r = "web:https://example.com/p/hew.json";
    assert_eq!(snapshot_slug(r), common::sha256_hex(r.as_bytes())[..16]);
}

#[test]
fn a_snapshot_is_the_index_as_the_build_read_it() {
    let (a, _) = two();
    let co = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    let out = common::scratch("out");
    build(&co, &out, None, &policy(0), BUILT);
    let snap =
        std::fs::read(out.join(format!("sources/{}.json", snapshot_slug(&a.reference())))).unwrap();
    let served = a.server.body(&a.index_path());
    // Same index: it parses, and carries the same items.
    let parsed = sources::parse_index(&snap).unwrap();
    let original = sources::parse_index(&served).unwrap();
    assert_eq!(parsed.items.len(), original.items.len());
    assert_eq!(parsed.name, original.name);
}

#[test]
fn catalog_json_parses_with_the_sources_crate() {
    let (a, b) = two();
    let removed = serde_json::json!([
        { "source": "web:https://gone.example/hew.json", "severity": "critical",
          "reason": "Pulled.", "link": "https://example.com/why" }
    ]);
    let co = common::checkout(&[a.reference(), b.reference()], &[], removed);
    let out = common::scratch("out");
    build(&co, &out, None, &policy(0), BUILT);
    let cat = sources::parse_catalog(&std::fs::read(out.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(cat.sources.len(), 2);
    assert_eq!(cat.removed.len(), 1);
    assert_eq!(cat.built, BUILT);
}

// ------------------------------------------- stale snapshots

#[test]
fn a_source_that_is_down_keeps_its_last_snapshot_and_is_stale() {
    let (a, b) = two();
    let co = common::checkout(&[a.reference(), b.reference()], &[], serde_json::json!([]));
    let first = common::scratch("first");
    build(&co, &first, None, &policy(0), "2026-11-23T06:00:00Z");
    let before =
        std::fs::read(first.join(format!("sources/{}.json", snapshot_slug(&b.reference()))))
            .unwrap();

    b.server.down();
    let second = common::scratch("second");
    let report = build(&co, &second, Some(&first), &policy(0), BUILT);
    assert_eq!(report.stale, vec![b.reference()]);
    assert_eq!(report.fresh, vec![a.reference()]);

    let cat = catalog_json(&second);
    assert_eq!(cat["sources"].as_array().unwrap().len(), 2);
    assert_eq!(entry(&cat, &a.reference())["stale"], false);
    let e = entry(&cat, &b.reference());
    assert_eq!(e["stale"], true);
    assert_eq!(
        e["name"], "Second",
        "the entry keeps what the last snapshot said"
    );
    assert_eq!(e["items"], 2);
    let kept = std::fs::read(second.join(e["snapshot"].as_str().unwrap())).unwrap();
    assert_eq!(
        kept, before,
        "the last good snapshot is carried over unchanged"
    );
}

#[test]
fn a_rate_limited_source_counts_as_failed_for_that_run() {
    let (a, b) = two();
    let co = common::checkout(&[a.reference(), b.reference()], &[], serde_json::json!([]));
    let first = common::scratch("first");
    build(&co, &first, None, &policy(0), "2026-11-23T06:00:00Z");
    b.server.rate_limit_all();
    let second = common::scratch("second");
    let report = build(&co, &second, Some(&first), &policy(0), BUILT);
    assert_eq!(report.stale, vec![b.reference()]);
    assert_eq!(entry(&catalog_json(&second), &b.reference())["stale"], true);
}

#[test]
fn a_source_that_fails_its_checks_keeps_its_last_snapshot() {
    // A wrong-hash item is a failed source for this run, same as down.
    let (a, _) = two();
    let co = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    let first = common::scratch("first");
    build(&co, &first, None, &policy(0), "2026-11-23T06:00:00Z");
    a.server.set_body(&a.item_path(0), b"changed".to_vec());
    let second = common::scratch("second");
    let report = build(&co, &second, Some(&first), &policy(0), BUILT);
    assert_eq!(report.stale, vec![a.reference()]);
}

#[test]
fn a_recovered_source_clears_stale() {
    let (a, _) = two();
    let co = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    let first = common::scratch("first");
    build(&co, &first, None, &policy(0), "2026-11-23T06:00:00Z");
    a.server.down();
    let second = common::scratch("second");
    build(
        &co,
        &second,
        Some(&first),
        &policy(0),
        "2026-11-23T07:00:00Z",
    );
    a.server.up();
    let third = common::scratch("third");
    build(&co, &third, Some(&second), &policy(0), BUILT);
    assert_eq!(entry(&catalog_json(&third), &a.reference())["stale"], false);
}

#[test]
fn a_failing_source_with_no_previous_snapshot_is_omitted_and_reported_failed() {
    let (a, b) = two();
    b.server.down();
    let co = common::checkout(&[a.reference(), b.reference()], &[], serde_json::json!([]));
    let out = common::scratch("out");
    let report = build(&co, &out, None, &policy(0), BUILT);
    assert_eq!(report.failed, vec![b.reference()]);
    assert_eq!(report.fresh, vec![a.reference()]);
    assert!(report.stale.is_empty());
    let cat = catalog_json(&out);
    assert_eq!(cat["sources"].as_array().unwrap().len(), 1);
    assert!(
        !out.join(format!("sources/{}.json", snapshot_slug(&b.reference())))
            .exists()
    );
}

#[test]
fn previous_defaults_to_the_out_directory_when_it_exists() {
    let out = common::scratch("out");
    assert_eq!(resolve_previous(None, &out), Some(out.clone()));
    let missing = out.join("not-yet");
    assert_eq!(resolve_previous(None, &missing), None);
    let other = common::scratch("other");
    assert_eq!(resolve_previous(Some(&other), &out), Some(other));
}

// ------------------------------------------- acceptance line 55

#[test]
fn a_removed_source_is_written_under_removed_and_left_out_of_sources() {
    let (a, b) = two();
    let removed = serde_json::json!([
        { "source": b.reference(), "severity": "removed", "reason": "Asked to be removed." }
    ]);
    let co = common::checkout(&[a.reference(), b.reference()], &[], removed.clone());
    let out = common::scratch("out");
    let report = build(&co, &out, None, &policy(0), BUILT);
    assert_eq!(report.removed, vec![b.reference()]);
    assert_eq!(report.fresh, vec![a.reference()]);

    let cat = catalog_json(&out);
    assert_eq!(cat["sources"].as_array().unwrap().len(), 1);
    assert_eq!(cat["sources"][0]["source"], a.reference().as_str());
    assert_eq!(
        cat["removed"], removed,
        "the entries of removed.json, verbatim"
    );
    assert!(
        !out.join(format!("sources/{}.json", snapshot_slug(&b.reference())))
            .exists()
    );
    // A removed source is not even requested.
    assert!(b.server.requests().is_empty(), "{:?}", b.server.requests());
}

#[test]
fn removed_matches_with_any_ref_set_aside() {
    // 6.2: the removed list matches when canonical forms are equal ignoring @ref.
    let removed = serde_json::json!([
        { "source": "github:someone/parts", "severity": "removed", "reason": "Gone." }
    ]);
    let co = common::checkout(&["github:someone/parts@v2".to_string()], &[], removed);
    let out = common::scratch("out");
    let p = policy(0);
    // A removed source must never be fetched, so a fetcher that cannot
    // reach anything must still let the build succeed.
    let report = build(&co, &out, None, &p, BUILT);
    assert_eq!(report.removed, vec!["github:someone/parts@v2".to_string()]);
    assert!(catalog_json(&out)["sources"].as_array().unwrap().is_empty());
}

// ------------------------------------------- inputs

#[test]
fn plugins_json_is_read_and_a_bad_list_is_a_typed_error() {
    let co = common::checkout(&[], &[], serde_json::json!([]));
    std::fs::write(co.join("library.json"), b"{\"not\":\"an array\"}").unwrap();
    let p = policy(0);
    let err = build_catalog(
        &co,
        &common::scratch("out"),
        &HttpFetcher::new(&p, None),
        &p,
        None,
        BUILT,
    )
    .unwrap_err();
    assert!(
        matches!(err, hew_cli::catalog::BuildError::BadList { .. }),
        "{err:?}"
    );
}

#[test]
fn a_missing_list_file_is_a_typed_error() {
    let co = common::checkout(&[], &[], serde_json::json!([]));
    std::fs::remove_file(co.join("removed.json")).unwrap();
    let p = policy(0);
    let r = build_catalog(
        &co,
        &common::scratch("out"),
        &HttpFetcher::new(&p, None),
        &p,
        None,
        BUILT,
    );
    assert!(r.is_err());
}

#[test]
fn an_empty_catalog_is_valid() {
    let co = common::checkout(&[], &[], serde_json::json!([]));
    let out = common::scratch("out");
    build(&co, &out, None, &policy(0), BUILT);
    let cat = sources::parse_catalog(&std::fs::read(out.join("catalog.json")).unwrap()).unwrap();
    assert!(cat.sources.is_empty() && cat.removed.is_empty());
}

// ------------------------------------------- determinism

fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&p).unwrap(),
                ));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

#[test]
fn output_is_deterministic_apart_from_built() {
    let (a, b) = two();
    let co = common::checkout(&[b.reference(), a.reference()], &[], serde_json::json!([]));
    let o1 = common::scratch("o1");
    let o2 = common::scratch("o2");
    build(&co, &o1, None, &policy(0), "2026-11-23T06:00:00Z");
    build(&co, &o2, None, &policy(0), "2030-01-01T00:00:00Z");
    let strip = |mut v: Vec<(String, Vec<u8>)>| {
        for (name, bytes) in &mut v {
            if name == "catalog.json" {
                let mut j: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                j["built"] = "".into();
                *bytes = serde_json::to_vec(&j).unwrap();
            }
        }
        v
    };
    assert_eq!(strip(tree(&o1)), strip(tree(&o2)));
}

#[test]
fn built_comes_from_the_injected_clock() {
    let co = common::checkout(&[], &[], serde_json::json!([]));
    let out = common::scratch("out");
    build(&co, &out, None, &policy(0), "2031-02-03T04:05:06Z");
    assert_eq!(catalog_json(&out)["built"], "2031-02-03T04:05:06Z");
}

// ------------------------------------------- per-host spacing

#[test]
fn requests_to_one_host_are_spaced_by_the_request_gap() {
    let (a, _) = two();
    let co = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    let out = common::scratch("out");
    let gap = Duration::from_millis(120);
    build(&co, &out, None, &policy(120), BUILT);
    let reqs = a.server.requests();
    assert!(
        reqs.len() >= 5,
        "index, two items, two thumbnails: {reqs:?}"
    );
    for w in reqs.windows(2) {
        let d = w[1].at.duration_since(w[0].at);
        // 10% slack for scheduler jitter on the server's clock.
        assert!(
            d >= gap.mul_f32(0.9),
            "{d:?} between {} and {}",
            w[0].path,
            w[1].path
        );
    }
}

#[test]
fn two_servers_on_the_same_host_are_spaced_as_one_host() {
    // Same host (127.0.0.1), different ports: per-host spacing is by host
    // only, so the two servers' requests interleave at no less than the gap.
    let (a, b) = two();
    let co = common::checkout(&[a.reference(), b.reference()], &[], serde_json::json!([]));
    let gap = Duration::from_millis(120);
    build(&co, &common::scratch("out"), None, &policy(120), BUILT);
    let mut all: Vec<_> = a
        .server
        .requests()
        .into_iter()
        .chain(b.server.requests())
        .collect();
    all.sort_by_key(|r| r.at);
    assert!(all.len() >= 10, "{all:?}");
    for w in all.windows(2) {
        assert!(w[1].at.duration_since(w[0].at) >= gap.mul_f32(0.9), "{w:?}");
    }
}

#[test]
fn every_build_request_carries_an_origin_header() {
    let (a, _) = two();
    let co = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    build(&co, &common::scratch("out"), None, &policy(0), BUILT);
    let reqs = a.server.requests();
    assert!(!reqs.is_empty());
    assert!(reqs.iter().all(|r| r.origin.is_some()), "{reqs:?}");
}

// ------------------------------------------- GITHUB_TOKEN

#[test]
fn the_github_token_goes_only_to_the_github_api_host() {
    let (name, value) =
        auth_header_for("api.github.com", Some("tok")).expect("header for the API host");
    assert_eq!(name.to_ascii_lowercase(), "authorization");
    assert!(value.contains("tok"), "{value}");
    assert!(auth_header_for("API.GITHUB.COM", Some("tok")).is_some());
    for host in [
        "raw.githubusercontent.com",
        "github.com",
        "api.github.com.evil.example",
        "evilapi.github.com",
        "gitlab.com",
        "codeberg.org",
        "127.0.0.1",
        "127.0.0.1:8080",
        "",
    ] {
        assert_eq!(auth_header_for(host, Some("tok")), None, "{host}");
    }
}

#[test]
fn no_token_means_no_header_even_for_the_api_host() {
    assert_eq!(auth_header_for("api.github.com", None), None);
    assert_eq!(auth_header_for("api.github.com", Some("")), None);
}

#[test]
fn a_token_is_never_sent_to_a_source_server() {
    let (a, _) = two();
    let co = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    let p = policy(0);
    let fetcher = HttpFetcher::new(&p, Some("secret-token".to_string()));
    build_catalog(&co, &common::scratch("out"), &fetcher, &p, None, BUILT).unwrap();
    let reqs = a.server.requests();
    assert!(!reqs.is_empty());
    assert!(reqs.iter().all(|r| r.authorization.is_none()), "{reqs:?}");
}

// ------------------------------------------- the binary

#[test]
fn the_binary_builds_an_empty_catalog() {
    let co: PathBuf = common::checkout(&[], &[], serde_json::json!([]));
    let out = common::scratch("out");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_hew-cli"))
        .args([
            "catalog",
            "build",
            &co.to_string_lossy(),
            "--out",
            &out.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cat = sources::parse_catalog(&std::fs::read(out.join("catalog.json")).unwrap()).unwrap();
    assert!(cat.sources.is_empty());
}
