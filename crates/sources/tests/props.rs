//! Robustness properties: hostile or random input never panics, and what is
//! accepted never escapes. (Under `unimplemented!()` these fail by panic.)
mod common;
use common::*;
use proptest::prelude::*;
use serde_json::json;
use sources::{Kind, commit_from_response, file_url, parse_catalog, parse_index, parse_source_ref};

proptest! {
    #[test]
    fn parse_index_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = parse_index(&bytes);
    }

    #[test]
    fn parse_catalog_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = parse_catalog(&bytes);
    }

    #[test]
    fn parse_source_ref_never_panics(s in "\\PC{0,200}") {
        let _ = parse_source_ref(&s);
    }

    #[test]
    fn parse_source_ref_never_panics_on_urlish_text(
        s in "(github:|gitlab:|codeberg:|forgejo:|web:|https://|http://)?[A-Za-z0-9./@:?#%\\\\ -]{0,80}"
    ) {
        let _ = parse_source_ref(&s);
    }

    #[test]
    fn parse_index_never_panics_on_mutated_valid_documents(
        cut in 0usize..400, flip in 0usize..400, byte in any::<u8>()
    ) {
        let mut bytes = serde_json::to_vec(&index(vec![item(1), item(2)])).unwrap();
        if flip < bytes.len() { bytes[flip] = byte; }
        let _ = parse_index(&bytes);
        bytes.truncate(cut.min(bytes.len()));
        let _ = parse_index(&bytes);
    }

    #[test]
    fn commit_from_response_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        for k in [Kind::Github, Kind::Gitlab, Kind::Forgejo, Kind::Web] {
            let _ = commit_from_response(k, &bytes);
        }
    }

    /// An accepted reference re-parses to itself, whatever the input form.
    #[test]
    fn github_references_canonicalize_idempotently(
        owner in "[A-Za-z0-9][A-Za-z0-9-]{0,10}",
        repo in "[A-Za-z0-9][A-Za-z0-9_-]{0,10}",
        dir in prop::option::of("[A-Za-z0-9]([A-Za-z0-9 _-]{0,7}[A-Za-z0-9])?(/[A-Za-z0-9]([A-Za-z0-9_-]{0,7}[A-Za-z0-9])?){0,2}"),
        git_ref in prop::option::of("[A-Za-z0-9]([A-Za-z0-9_-]{0,6}[A-Za-z0-9])?(\\.[A-Za-z0-9]{1,3})?"),
    ) {
        let mut t = format!("github:{owner}/{repo}");
        if let Some(d) = &dir { t.push_str(&format!("//{d}")); }
        if let Some(r) = &git_ref { t.push_str(&format!("@{r}")); }
        let first = parse_source_ref(&t).unwrap();
        prop_assert_eq!(first.len(), 1);
        let want = format!("github:{}/{}", owner.to_lowercase(), repo.to_lowercase());
        prop_assert!(first[0].canonical.starts_with(&want), "{} vs {}", first[0].canonical, want);
        prop_assert_eq!(first[0].git_ref.is_some(), git_ref.is_some());
        let again = parse_source_ref(&first[0].canonical).unwrap();
        prop_assert_eq!(&again[0], &first[0]);
        // The https URL form of the same repo is the same source.
        let u = parse_source_ref(&format!("https://github.com/{owner}/{repo}")).unwrap();
        prop_assert_eq!(&u[0].repo, &first[0].repo);
    }

    /// Whatever `file_url` accepts stays on the source's host, under its
    /// folder, with no dot segment.
    #[test]
    fn file_url_never_escapes(p in "[A-Za-z0-9 ./\\\\%:?#@_-]{0,60}") {
        let w = parse_source_ref("web:https://example.com/parts").unwrap().remove(0);
        if let Ok(u) = file_url(&w, &p, None) {
            prop_assert_eq!(u.scheme(), "https");
            prop_assert_eq!(u.host_str(), Some("example.com"));
            prop_assert!(u.path().starts_with("/parts/"), "{}", u);
            prop_assert!(!u.path().split('/').any(|s| s == ".." || s == "."), "{}", u);
            prop_assert!(!u.path().to_lowercase().contains("%2e"), "{}", u);
            prop_assert_eq!(u.query(), None);
            prop_assert_eq!(u.fragment(), None);
        }
        if p.split('/').any(|s| s == ".." || s.is_empty() || s.starts_with('.')) {
            prop_assert!(file_url(&w, &p, None).is_err(),
                "a path with an empty, dot or dot-dot segment must be refused: {:?}", p);
        }
    }

    /// Any item path an index accepts is relative, safe, and one `file_url`
    /// also accepts: the two layers agree.
    #[test]
    fn accepted_index_paths_are_accepted_by_file_url(p in "[A-Za-z0-9 ./\\\\%:?#@_-]{0,40}\\.hew") {
        let w = parse_source_ref("web:https://example.com/parts").unwrap().remove(0);
        if parse(&with_item(|i| i["path"] = json!(p.clone()))).is_ok() {
            prop_assert!(file_url(&w, &p, None).is_ok(), "{:?}", p);
        }
    }
}
