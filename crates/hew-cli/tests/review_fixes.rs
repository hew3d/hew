//! Reproducing specs for the adversarial-review fixes to packages S3/S4
//! (docs/design/shared-library-sources.md 6.3, 6.4, 7.6): `catalog build`
//! validates `removed.json` as `parse_catalog` will, `library index` never
//! writes through a symlink, and a file with no `hew.library` category is
//! refused by `library index` and reported by `catalog check`.

mod common;

use common::Fixture;
use hew_cli::catalog::{BuildError, FetchPolicy, HttpFetcher, build_catalog, check_source, code};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

const BUILT: &str = "2026-11-23T12:00:00Z";

fn policy() -> FetchPolicy {
    FetchPolicy::new(Duration::from_millis(0), Duration::from_secs(10)).with_loopback()
}

fn try_build(
    checkout: &Path,
    out: &Path,
    previous: Option<&Path>,
) -> Result<hew_cli::catalog::BuildReport, BuildError> {
    let p = policy();
    build_catalog(
        checkout,
        out,
        &HttpFetcher::new(&p, None),
        &p,
        previous,
        BUILT,
    )
}

// ------------------------------------------------------------------ F2

fn bad_removed(entry: serde_json::Value, needle: &str) {
    let co = common::checkout(&[], &[], serde_json::json!([entry]));
    let out = common::scratch("out");
    let err = try_build(&co, &out, None).unwrap_err();
    match &err {
        BuildError::BadList { file, reason } => {
            assert_eq!(file, "removed.json", "{err:?}");
            assert!(reason.contains(needle), "{needle:?} not in {reason:?}");
        }
        other => panic!("expected BadList, got {other:?}"),
    }
    assert!(
        !out.join("catalog.json").exists(),
        "a refused build writes no catalog"
    );
}

#[test]
fn removed_entry_with_a_non_canonical_source_is_refused() {
    bad_removed(
        serde_json::json!({ "source": "github:A/B", "severity": "removed", "reason": "x" }),
        "github:A/B",
    );
}

#[test]
fn removed_entry_with_an_unparseable_source_is_refused() {
    bad_removed(
        serde_json::json!({ "source": "not a source", "severity": "removed", "reason": "x" }),
        "not a source",
    );
}

#[test]
fn removed_entry_with_an_http_link_is_refused() {
    bad_removed(
        serde_json::json!({ "source": "web:https://x.example/hew.json",
            "severity": "removed", "reason": "x", "link": "http://x.example" }),
        "link",
    );
}

#[test]
fn removed_entry_with_a_bad_severity_or_missing_reason_is_refused() {
    bad_removed(
        serde_json::json!({ "source": "web:https://x.example/hew.json",
            "severity": "gone", "reason": "x" }),
        "severity",
    );
    bad_removed(
        serde_json::json!({ "source": "web:https://x.example/hew.json", "severity": "removed" }),
        "reason",
    );
}

#[test]
fn the_error_names_which_entry_is_bad() {
    let co = common::checkout(
        &[],
        &[],
        serde_json::json!([
            { "source": "web:https://x.example/hew.json", "severity": "removed", "reason": "ok" },
            { "source": "github:A/B", "severity": "removed", "reason": "x" },
        ]),
    );
    let err = try_build(&co, &common::scratch("out"), None).unwrap_err();
    let BuildError::BadList { reason, .. } = err else {
        panic!("not BadList")
    };
    assert!(
        reason.contains("entry 2") || reason.contains("#2"),
        "{reason}"
    );
}

#[test]
fn a_catalog_with_stale_failed_and_removed_entries_reparses() {
    let a = Fixture::new("A", Some("MIT"));
    let b = Fixture::new("B", Some("MIT"));
    let c = Fixture::new("C", Some("MIT"));
    let gone = Fixture::new("Gone", Some("MIT"));
    // First build: only a is known.
    let co1 = common::checkout(&[a.reference()], &[], serde_json::json!([]));
    let first = common::scratch("first");
    try_build(&co1, &first, None).unwrap();
    // Second: a is down (stale), b is down with no snapshot (failed), c is
    // fresh, gone is removed (with a link).
    a.server.down();
    b.server.down();
    let removed = serde_json::json!([
        { "source": gone.reference(), "severity": "critical",
          "reason": "Pulled.", "link": "https://example.com/why" }
    ]);
    let co2 = common::checkout(
        &[
            a.reference(),
            b.reference(),
            c.reference(),
            gone.reference(),
        ],
        &[],
        removed,
    );
    let second = common::scratch("second");
    let report = try_build(&co2, &second, Some(&first)).unwrap();
    assert_eq!(report.stale, vec![a.reference()]);
    assert_eq!(report.failed, vec![b.reference()]);
    assert_eq!(report.removed, vec![gone.reference()]);
    let cat = sources::parse_catalog(&std::fs::read(second.join("catalog.json")).unwrap())
        .expect("the written catalog re-parses");
    assert_eq!(cat.removed.len(), 1);
    assert_eq!(cat.sources.len(), 2);
}

// ------------------------------------------------------------------ F5 / F8 helpers

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn folder() -> PathBuf {
    let dir = common::scratch("folder");
    copy_tree(common::item_tree(), &dir);
    dir
}

fn index(folder: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hew-cli"))
        .args(["library", "index", folder.to_str().unwrap(), "--name", "X"])
        .env_remove("HEW_LIBRARY_DIR")
        .output()
        .expect("hew-cli runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A Hew file with a Library id and name but no category.
fn categoryless_bytes() -> Vec<u8> {
    use kernel::{AttrTarget, AttrValue, Document};
    let mut doc = Document::new();
    for (k, v) in [
        ("id", "11111111-1111-4111-8111-111111111111"),
        ("name", "No Category"),
    ] {
        doc.attr_set(
            AttrTarget::Document,
            "hew.library",
            k,
            AttrValue::Text(v.into()),
        )
        .unwrap();
    }
    doc.save()
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
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
    walk(root, root, &mut v);
    v.sort();
    v
}

// ------------------------------------------------------------------ F5

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    fn sha_of_first_item(folder: &Path) -> String {
        fn find(d: &Path) -> Option<PathBuf> {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for p in entries {
                if p.is_dir() {
                    if let Some(f) = find(&p) {
                        return Some(f);
                    }
                } else if p.extension().is_some_and(|x| x == "hew") {
                    return Some(p);
                }
            }
            None
        }
        common::sha256_hex(&std::fs::read(find(folder).unwrap()).unwrap())
    }

    #[test]
    fn a_symlinked_thumbnails_directory_is_refused() {
        let f = folder();
        let outside = common::scratch("outside");
        symlink(&outside, f.join("thumbnails")).unwrap();
        let o = index(&f);
        assert!(!o.status.success(), "{}", text(&o));
        assert!(text(&o).contains("thumbnails"), "{}", text(&o));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        assert!(!f.join("hew.json").exists());
    }

    #[test]
    fn a_symlinked_thumbnail_file_is_refused() {
        let f = folder();
        let target = common::scratch("outside").join("victim");
        std::fs::write(&target, b"precious").unwrap();
        std::fs::create_dir_all(f.join("thumbnails")).unwrap();
        let png = format!("{}.png", sha_of_first_item(&f));
        symlink(&target, f.join("thumbnails").join(&png)).unwrap();
        let o = index(&f);
        assert!(!o.status.success(), "{}", text(&o));
        assert!(text(&o).contains(&png), "{}", text(&o));
        assert_eq!(std::fs::read(&target).unwrap(), b"precious");
        assert!(!f.join("hew.json").exists());
    }

    #[test]
    fn a_symlinked_index_file_is_refused() {
        let f = folder();
        // A real, valid index kept outside; hew.json is a link to it.
        let first = index(&f);
        assert!(first.status.success(), "{}", text(&first));
        let target = common::scratch("outside").join("victim.json");
        std::fs::rename(f.join("hew.json"), &target).unwrap();
        std::fs::remove_dir_all(f.join("thumbnails")).unwrap();
        symlink(&target, f.join("hew.json")).unwrap();
        let before = std::fs::read(&target).unwrap();
        let o = index(&f);
        assert!(!o.status.success(), "{}", text(&o));
        assert!(text(&o).contains("hew.json"), "{}", text(&o));
        assert_eq!(std::fs::read(&target).unwrap(), before);
        // Nothing else was written either.
        assert!(!f.join("thumbnails").exists());
    }
}

// ------------------------------------------------------------------ F8

#[test]
fn library_index_refuses_a_file_with_no_category_by_name() {
    let f = folder();
    std::fs::write(f.join("nocat.hew"), categoryless_bytes()).unwrap();
    let before = snapshot(&f);
    let o = index(&f);
    assert!(!o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("nocat.hew") && t.contains("category"), "{t}");
    assert_eq!(snapshot(&f), before, "nothing is written");
}

#[test]
fn catalog_check_reports_an_item_with_no_category_as_item_mismatch() {
    let f = Fixture::new("F", Some("MIT"));
    let bytes = categoryless_bytes();
    // Point item 0's file and entry at a file with the right id but no
    // category, so only the category rule can fail.
    let id = "11111111-1111-4111-8111-111111111111";
    f.server.set_body(&f.item_path(0), bytes.clone());
    f.edit_index(|v| {
        v["items"][0]["sha256"] = common::sha256_hex(&bytes).into();
        v["items"][0]["bytes"] = bytes.len().into();
        v["items"][0]["id"] = id.into();
        v["items"][0]["category"] = "Components".into();
    });
    let failures = check_source(
        &f.reference(),
        &HttpFetcher::new(&policy(), None),
        &policy(),
    );
    let codes: Vec<_> = failures.iter().map(|c| c.code).collect();
    assert_eq!(codes, vec![code::ITEM_MISMATCH], "{failures:?}");
}
