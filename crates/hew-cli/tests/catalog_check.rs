//! `hew-cli catalog check` (docs/design/shared-library-sources.md 6.4, 7.6;
//! acceptance line 53). The oracle for package S4, check half: every spec
//! here fails until `hew_cli::catalog` is implemented.
//!
//! Sources are served by a loopback server (`tests/common`) and requested
//! with `allow_http_loopback: true` (`web:https://127.0.0.1:<port>/…`
//! is read over http). Failure codes are `hew_cli::catalog::code::*`.

mod common;

use common::{Fixture, Route};
use hew_cli::catalog::{
    FetchPolicy, HttpFetcher, WEB_ORIGIN, check_list, check_source, code, run_check,
};
use std::time::Duration;

fn policy(loopback: bool) -> FetchPolicy {
    let p = FetchPolicy::new(Duration::from_millis(0), Duration::from_secs(10));
    if loopback { p.with_loopback() } else { p }
}

fn check(f: &Fixture, loopback: bool) -> Vec<hew_cli::catalog::CheckFailure> {
    let p = policy(loopback);
    check_source(&f.reference(), &HttpFetcher::new(&p, None), &p)
}

fn codes(v: &[hew_cli::catalog::CheckFailure]) -> Vec<&'static str> {
    v.iter().map(|f| f.code).collect()
}

fn good() -> Fixture {
    Fixture::new("Fixture", Some("CC0-1.0"))
}

// -------------------------------------------------- a good source passes

#[test]
fn a_valid_licensed_source_passes_every_check() {
    let f = good();
    assert_eq!(check(&f, true), vec![]);
}

#[test]
fn every_failure_is_one_line() {
    let f = Fixture::new("Fixture", None);
    f.server.set_cors_all(false);
    for failure in check(&f, true) {
        assert!(!failure.message.contains('\n'), "{failure:?}");
        assert!(!failure.message.is_empty());
    }
}

// ------------------------------- reference canonical, sorted and unique

#[test]
fn a_canonical_sorted_unique_list_passes() {
    let list = vec!["github:a/b".to_string(), "github:c/d".to_string()];
    assert_eq!(check_list(&list), vec![]);
}

#[test]
fn a_non_canonical_reference_in_a_list_fails() {
    // Owner and repo are lowercased in the canonical form.
    let list = vec!["github:Owner/Repo".to_string()];
    assert_eq!(codes(&check_list(&list)), vec![code::REFERENCE]);
}

#[test]
fn an_unparseable_reference_in_a_list_fails() {
    let list = vec!["not a reference".to_string()];
    assert_eq!(codes(&check_list(&list)), vec![code::REFERENCE]);
}

#[test]
fn an_unsorted_list_fails() {
    let list = vec!["github:c/d".to_string(), "github:a/b".to_string()];
    assert_eq!(codes(&check_list(&list)), vec![code::REFERENCE]);
}

#[test]
fn a_duplicate_reference_fails() {
    let list = vec!["github:a/b".to_string(), "github:a/b".to_string()];
    assert_eq!(codes(&check_list(&list)), vec![code::REFERENCE]);
}

#[test]
fn check_source_refuses_a_reference_that_is_not_canonical() {
    let f = good();
    let p = policy(true);
    // The same source as a bare https address: parses, but is not canonical.
    let bare = format!("https://127.0.0.1:{}/fixture", f.server.port);
    let failures = check_source(&bare, &HttpFetcher::new(&p, None), &p);
    assert!(codes(&failures).contains(&code::REFERENCE), "{failures:?}");
}

// ----------------------------------------------------------- index valid

#[test]
fn an_invalid_index_fails_the_index_check() {
    let f = good();
    f.server.set_body(&f.index_path(), br#"{"hew":1}"#.to_vec());
    assert_eq!(codes(&check(&f, true)), vec![code::INDEX_INVALID]);
}

#[test]
fn an_index_that_needs_a_newer_hew_fails_the_index_check() {
    let f = good();
    f.edit_index(|v| v["hew"] = 2.into());
    assert_eq!(codes(&check(&f, true)), vec![code::INDEX_INVALID]);
}

// ---------------------------------------------------------- license present

#[test]
fn an_unlicensed_source_prints_the_license_line() {
    let f = Fixture::new("Fixture", None);
    assert_eq!(codes(&check(&f, true)), vec![code::LICENSE_MISSING]);
}

// ----------------------- fetch within the cap and match the hash

#[test]
fn a_wrong_hash_item_fails_the_integrity_check() {
    let f = good();
    let mut body = f.server.body(&f.item_path(0));
    let last = body.len() - 1;
    body[last] ^= 0xff; // same length, different bytes
    f.server.set_body(&f.item_path(0), body);
    assert_eq!(codes(&check(&f, true)), vec![code::FILE_INTEGRITY]);
}

#[test]
fn an_item_longer_than_its_bytes_fails_the_integrity_check() {
    let f = good();
    let mut body = f.server.body(&f.item_path(0));
    body.extend_from_slice(&[0u8; 4096]);
    f.server.set_body(&f.item_path(0), body);
    assert_eq!(codes(&check(&f, true)), vec![code::FILE_INTEGRITY]);
}

#[test]
fn a_wrong_hash_thumbnail_fails_the_integrity_check() {
    let f = good();
    f.server.set_body(&f.thumb_path(0), b"not the png".to_vec());
    assert_eq!(codes(&check(&f, true)), vec![code::FILE_INTEGRITY]);
}

#[test]
fn a_thumbnail_over_one_megabyte_fails_the_integrity_check() {
    let f = good();
    f.server
        .set_body(&f.thumb_path(0), vec![0u8; 1024 * 1024 + 1]);
    assert_eq!(codes(&check(&f, true)), vec![code::FILE_INTEGRITY]);
}

#[test]
fn a_missing_item_file_fails_the_integrity_check() {
    let f = good();
    f.server.set_status(&f.item_path(1), 404);
    assert_eq!(codes(&check(&f, true)), vec![code::FILE_INTEGRITY]);
}

// ---------------- every item loads; id and category match the entry

#[test]
fn an_entry_with_the_wrong_id_fails_the_item_check() {
    let f = good();
    f.edit_index(|v| v["items"][0]["id"] = "00000000-0000-4000-8000-000000000000".into());
    assert_eq!(codes(&check(&f, true)), vec![code::ITEM_MISMATCH]);
}

#[test]
fn an_entry_with_the_wrong_category_fails_the_item_check() {
    let f = good();
    // item 0 and 1 are a component and a model; swap item 0's listing.
    let other = if f.index["items"][0]["category"] == "Models" {
        "Components"
    } else {
        "Models"
    };
    f.edit_index(|v| v["items"][0]["category"] = other.into());
    assert_eq!(codes(&check(&f, true)), vec![code::ITEM_MISMATCH]);
}

#[test]
fn an_item_file_that_does_not_load_fails_the_item_check() {
    let f = good();
    // Replace the file AND the entry's hash and size, so only loading fails.
    let junk = b"this is not a hew file".to_vec();
    f.server.set_body(&f.item_path(0), junk.clone());
    f.edit_index(|v| {
        v["items"][0]["sha256"] = common::sha256_hex(&junk).into();
        v["items"][0]["bytes"] = junk.len().into();
    });
    assert_eq!(codes(&check(&f, true)), vec![code::ITEM_MISMATCH]);
}

// ----------------------------------------------------------- ids unique

#[test]
fn duplicate_ids_fail() {
    let f = good();
    f.edit_index(|v| {
        let id = v["items"][0]["id"].clone();
        v["items"][1]["id"] = id;
    });
    let c = codes(&check(&f, true));
    // The index validator may refuse duplicates first (index_invalid) or the
    // dedicated check may; either way the line is printed and it is one line.
    assert!(
        c == vec![code::IDS_UNIQUE] || c == vec![code::INDEX_INVALID],
        "{c:?}"
    );
}

// ------------------------------ final URLs https on a public address

#[test]
fn a_loopback_source_fails_the_public_host_check_when_the_allowance_is_off() {
    let f = good();
    let c = codes(&check(&f, false));
    assert!(c.contains(&code::PUBLIC_HTTPS), "{c:?}");
    // Refused before any request is sent.
    assert!(f.server.requests().is_empty(), "{:?}", f.server.requests());
}

#[test]
fn a_loopback_source_passes_the_public_host_check_when_the_allowance_is_on() {
    let f = good();
    assert!(!codes(&check(&f, true)).contains(&code::PUBLIC_HTTPS));
}

// ------------------------------------------------------ cross-origin header

#[test]
fn every_request_carries_an_origin_header() {
    let f = good();
    assert_eq!(check(&f, true), vec![]);
    let reqs = f.server.requests();
    assert!(!reqs.is_empty());
    for r in &reqs {
        assert_eq!(r.origin.as_deref(), Some(WEB_ORIGIN), "{r:?}");
    }
    // The index, both items and both thumbnails were all requested.
    for p in [
        f.index_path(),
        f.item_path(0),
        f.item_path(1),
        f.thumb_path(0),
        f.thumb_path(1),
    ] {
        assert!(reqs.iter().any(|r| r.path == p), "never requested {p}");
    }
}

#[test]
fn a_source_without_the_cross_origin_header_prints_the_cors_line() {
    let f = good();
    f.server.set_cors_all(false);
    assert_eq!(codes(&check(&f, true)), vec![code::CORS]);
}

#[test]
fn an_allow_origin_for_another_origin_fails_the_cors_check() {
    let f = good();
    f.server.set_acao(&f.index_path(), "https://other.example");
    assert_eq!(codes(&check(&f, true)), vec![code::CORS]);
}

#[test]
fn an_allow_origin_equal_to_the_request_origin_passes() {
    let f = good();
    f.server.set_acao(&f.index_path(), WEB_ORIGIN);
    assert_eq!(check(&f, true), vec![]);
}

#[test]
fn many_requests_lacking_the_header_still_print_one_cors_line() {
    let f = good();
    f.server.set_cors_all(false);
    let c = codes(&check(&f, true));
    assert_eq!(c.iter().filter(|x| **x == code::CORS).count(), 1, "{c:?}");
}

#[test]
fn constructing_two_fetchers_installs_the_tls_provider_without_panicking() {
    let p = policy(true);
    let _a = HttpFetcher::new(&p, None);
    let _b = HttpFetcher::new(&p, Some("t".into()));
    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
}

#[test]
fn a_missing_header_on_the_index_alone_fails_the_cors_check() {
    let f = good();
    f.server.set_cors(&f.index_path(), false);
    assert_eq!(codes(&check(&f, true)), vec![code::CORS]);
}

#[test]
fn a_missing_header_on_an_item_alone_fails_the_cors_check() {
    let f = good();
    f.server.set_cors(&f.item_path(0), false);
    assert_eq!(codes(&check(&f, true)), vec![code::CORS]);
}

#[test]
fn a_missing_header_on_a_thumbnail_alone_fails_the_cors_check() {
    let f = good();
    f.server.set_cors(&f.thumb_path(0), false);
    assert_eq!(codes(&check(&f, true)), vec![code::CORS]);
}

// ---------------------------------------------------- rate limiting

#[test]
fn a_rate_limited_source_fails_the_check() {
    let f = good();
    f.server.rate_limit_all();
    let c = codes(&check(&f, true));
    assert!(c.contains(&code::RATE_LIMITED), "{c:?}");
}

#[test]
fn a_source_that_is_down_fails_the_check() {
    let f = good();
    f.server.down();
    assert!(!check(&f, true).is_empty());
}

// ------------------------- acceptance line 53, and the CLI output

#[test]
fn no_license_wrong_hash_and_no_cors_print_those_three_lines() {
    let f = Fixture::new("Fixture", None);
    let mut body = f.server.body(&f.item_path(0));
    let last = body.len() - 1;
    body[last] ^= 0xff;
    f.server.set(
        &f.item_path(0),
        Route {
            cors: false,
            ..Route::ok(body)
        },
    );
    // The index itself still answers with the header, so the cors failure is
    // carried by the item request and the thumbnails.
    f.server.set_cors(&f.thumb_path(0), false);

    let p = policy(true);
    let mut out = Vec::new();
    let status = run_check(&[f.reference()], &HttpFetcher::new(&p, None), &p, &mut out);
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_ne!(status, 0, "{text}");
    assert_eq!(lines.len(), 3, "{text}");
    for c in [code::LICENSE_MISSING, code::FILE_INTEGRITY, code::CORS] {
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with(&format!("{c}: ")))
                .count(),
            1,
            "{c} in {text}"
        );
    }
}

#[test]
fn a_clean_source_prints_nothing_and_exits_zero() {
    let f = good();
    let p = policy(true);
    let mut out = Vec::new();
    let status = run_check(&[f.reference()], &HttpFetcher::new(&p, None), &p, &mut out);
    assert_eq!(status, 0);
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
}

#[test]
fn check_with_no_reference_is_a_usage_error() {
    let p = policy(true);
    let mut out = Vec::new();
    assert_eq!(run_check(&[], &HttpFetcher::new(&p, None), &p, &mut out), 2);
}

/// The real binary has no loopback allowance, so a loopback source is
/// refused as non-public: exit non-zero and a `public_https` line.
#[test]
fn the_binary_refuses_a_loopback_source_as_not_public() {
    let f = good();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hew-cli"))
        .args(["catalog", "check", &f.reference()])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{text}");
    assert!(
        text.lines().any(|l| l.starts_with("public_https: ")),
        "{text}"
    );
}

#[test]
fn the_binary_lists_catalog_in_its_usage() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hew-cli"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("catalog"));
}
