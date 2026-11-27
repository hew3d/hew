//! The three Shared Library source exports (docs/design/shared-library-sources.md
//! 6.4): `source_parse_index`, `source_parse_ref` and `source_urls`. Each takes
//! and returns JSON text; a failure is `{ "error": { "code", "reason" } }`
//! (an index error also carries the 0-based `item` where it applies), never a
//! thrown exception and never a panic.

use serde_json::{Value, json};
use sources::{Kind, RefError, SourceRef};
use wasm_bindgen::prelude::*;

/// The error answer: `{ "error": { "code", "reason" } }`, with `item` added
/// for an index error that names one.
fn error_answer(code: &str, reason: &str, item: Option<usize>) -> String {
    let reason = if reason.is_empty() { code } else { reason };
    let mut body = json!({ "code": code, "reason": reason });
    if let (Some(i), Some(obj)) = (item, body.as_object_mut()) {
        obj.insert("item".to_owned(), json!(i));
    }
    json!({ "error": body }).to_string()
}

/// Serializes an answer; a value that cannot serialize (none of ours can) is
/// an error answer rather than a panic.
fn answer<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| error_answer("bad_input", "the answer could not be written", None))
}

/// Parses and validates the text of a `hew.json` (`sources::parse_index`).
/// Returns the parsed index as JSON (camelCase, 6.1), or an error object with
/// code `index_too_new`, `index_too_large` or `index_invalid`.
#[wasm_bindgen]
pub fn source_parse_index(json: &str) -> String {
    match sources::parse_index(json.as_bytes()) {
        Ok(index) => answer(&index),
        Err(sources::IndexError::TooNew { .. }) => {
            error_answer("index_too_new", "this index needs a newer Hew", None)
        }
        Err(sources::IndexError::TooLarge) => {
            error_answer("index_too_large", "the index is larger than 4 MB", None)
        }
        Err(sources::IndexError::Invalid { reason, item }) => {
            error_answer("index_invalid", &reason, item)
        }
    }
}

/// Parses a pasted source reference (`sources::parse_source_ref`). Returns a
/// JSON array of candidate references in probe order, or an error object with
/// code `ref_empty`, `ref_query_refused` or `ref_invalid`.
#[wasm_bindgen]
pub fn source_parse_ref(text: &str) -> String {
    match sources::parse_source_ref(text) {
        Ok(candidates) => answer(&candidates),
        Err(RefError::Empty) => error_answer("ref_empty", "the source reference is empty", None),
        Err(RefError::QueryRefused) => error_answer(
            "ref_query_refused",
            "a source reference cannot carry a query string",
            None,
        ),
        Err(RefError::Invalid { reason }) => error_answer("ref_invalid", &reason, None),
    }
}

/// The reference fields `source_urls` reads from `ref_json`.
struct RefFields {
    kind: String,
    host: String,
    repo: String,
    dir: String,
    git_ref: Option<String>,
    canonical: String,
}

/// Reads `ref_json` by hand: an object whose six fields have the right JSON
/// types. Anything else is `bad_input`.
fn read_ref(ref_json: &str) -> Result<RefFields, String> {
    let bad = |why: &str| error_answer("bad_input", why, None);
    let value: Value =
        serde_json::from_str(ref_json).map_err(|_| bad("the source reference is not JSON"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| bad("the source reference is not an object"))?;
    let text = |key: &str| -> Result<String, String> {
        obj.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| bad(&format!("the source reference's `{key}` is not a string")))
    };
    let git_ref = match obj.get("git_ref") {
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        _ => {
            return Err(bad(
                "the source reference's `git_ref` is not a string or null",
            ));
        }
    };
    Ok(RefFields {
        kind: text("kind")?,
        host: text("host")?,
        repo: text("repo")?,
        dir: text("dir")?,
        git_ref,
        canonical: text("canonical")?,
    })
}

/// The kind named by its JSON spelling, or `None` for any other word.
fn kind_of(name: &str) -> Option<Kind> {
    match name {
        "github" => Some(Kind::Github),
        "gitlab" => Some(Kind::Gitlab),
        "forgejo" => Some(Kind::Forgejo),
        "web" => Some(Kind::Web),
        _ => None,
    }
}

/// Re-derives the one candidate of `fields.canonical` whose fields equal the
/// reference's. Anything else is `ref_invalid`: no URL is built from a
/// reference whose fields do not follow from its canonical form.
fn matching_source(fields: &RefFields) -> Result<SourceRef, String> {
    let invalid = |why: &str| error_answer("ref_invalid", why, None);
    let candidates = sources::parse_source_ref(&fields.canonical)
        .map_err(|_| invalid("the source reference's canonical form does not parse"))?;
    let kind = kind_of(&fields.kind);
    let mut matches = candidates.into_iter().filter(|c| {
        Some(c.kind) == kind
            && c.host == fields.host
            && c.repo == fields.repo
            && c.dir == fields.dir
            && c.git_ref == fields.git_ref
    });
    match (matches.next(), matches.next()) {
        (Some(one), None) => Ok(one),
        _ => Err(invalid(
            "the source reference's fields do not match its canonical form",
        )),
    }
}

/// A commit as the URL builder may splice it: 40 or 64 lowercase hex.
fn is_commit(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The requests for one source. `ref_json` is one candidate as
/// `source_parse_ref` returned it; `paths_json` a JSON array of relative
/// paths; `commit` pins the reads. Returns
/// `{ "resolve": string | null, "index": string, "files": [string] }` (one
/// URL per path, in order), or an error object with code `bad_input`,
/// `ref_invalid` (the ref's fields do not equal the one candidate its
/// `canonical` parses to), `path_invalid` or `commit_invalid` (not 40 or 64
/// lowercase hex).
#[wasm_bindgen]
pub fn source_urls(ref_json: &str, paths_json: &str, commit: Option<String>) -> String {
    let fields = match read_ref(ref_json) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let paths: Vec<String> = match serde_json::from_str::<Value>(paths_json) {
        Ok(Value::Array(items)) => {
            let strings: Option<Vec<String>> = items
                .into_iter()
                .map(|v| match v {
                    Value::String(s) => Some(s),
                    _ => None,
                })
                .collect();
            match strings {
                Some(s) => s,
                None => return error_answer("bad_input", "a path is not a string", None),
            }
        }
        _ => return error_answer("bad_input", "the paths are not a JSON array", None),
    };
    let source = match matching_source(&fields) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if let Some(c) = commit.as_deref()
        && !is_commit(c)
    {
        return error_answer(
            "commit_invalid",
            "the commit is not 40 or 64 lowercase hex characters",
            None,
        );
    }
    let commit = commit.as_deref();
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        match sources::file_url(&source, path, commit) {
            Ok(url) => files.push(url.to_string()),
            Err(_) => {
                return error_answer("path_invalid", "a path escapes the source folder", None);
            }
        }
    }
    answer(&json!({
        "resolve": sources::resolve_url(&source).map(|u| u.to_string()),
        "index": sources::index_url(&source, commit).to_string(),
        "files": files,
    }))
}

/// The commit a resolve response names (`sources::commit_from_response`).
/// `kind` is `github`, `gitlab`, `forgejo` or `web`; `body_text` the response
/// body as text. Returns `{ "commit": "<40 or 64 lowercase hex>" }` or
/// `{ "commit": null }` (a body that names no commit, a web source, anything
/// hostile); an error object with code `bad_input` only for an unknown kind.
#[wasm_bindgen]
pub fn source_commit(kind: &str, body_text: &str) -> String {
    match kind_of(kind) {
        Some(k) => {
            let commit = sources::commit_from_response(k, body_text.as_bytes());
            answer(&json!({ "commit": commit }))
        }
        None => error_answer("bad_input", "the source kind is not known", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn item(n: u32) -> Value {
        json!({
            "id": format!("00000000-0000-4000-8000-{n:012x}"),
            "path": format!("Components/item-{n}.hew"),
            "name": format!("Item {n}"),
            "category": "Components",
            "keywords": ["a", "b"],
            "collection": "Hinges",
            "sha256": SHA,
            "bytes": 100,
            "thumbnail": format!("thumbnails/{SHA}.png"),
            "license": "CC0-1.0"
        })
    }

    fn full_index() -> Value {
        json!({
            "hew": 1,
            "name": "Shop Hardware",
            "description": "Hinges.",
            "author": "Example Maker",
            "homepage": "https://example.com/parts",
            "license": "LicenseRef-Mine",
            "licenseUrl": "https://example.com/license",
            "items": [item(1), item(2)],
            "plugins": []
        })
    }

    /// The parsed JSON of an answer.
    fn parsed(s: &str) -> Value {
        serde_json::from_str(s).unwrap_or_else(|e| panic!("answer is not JSON ({e}): {s}"))
    }

    /// The `error` object of an answer that must be an error: the only
    /// top-level key, with a non-empty string `code` and `reason`.
    #[track_caller]
    fn error_of(s: &str) -> Value {
        let v = parsed(s);
        let obj = v.as_object().expect("an error answer is an object");
        assert_eq!(obj.len(), 1, "an error answer has only `error`: {s}");
        let e = obj
            .get("error")
            .unwrap_or_else(|| panic!("no `error` in {s}"));
        assert!(e["code"].as_str().is_some_and(|c| !c.is_empty()), "{s}");
        assert!(e["reason"].as_str().is_some_and(|c| !c.is_empty()), "{s}");
        e.clone()
    }

    #[track_caller]
    fn code_of(s: &str) -> String {
        error_of(s)["code"].as_str().unwrap().to_owned()
    }

    fn is_error(s: &str) -> bool {
        parsed(s).get("error").is_some()
    }

    /// The candidates of a reference that must parse.
    fn candidates(text: &str) -> Vec<Value> {
        let out = source_parse_ref(text);
        parsed(&out)
            .as_array()
            .unwrap_or_else(|| panic!("not an array: {out}"))
            .clone()
    }

    fn urls_for(r: &Value, paths: &[&str], commit: Option<&str>) -> String {
        source_urls(
            &r.to_string(),
            &serde_json::to_string(paths).unwrap(),
            commit.map(str::to_owned),
        )
    }

    // ---- source_parse_index (6.1) ------------------------------------------

    /// 6.1: a valid index comes back as the parsed `SourceIndex`, camelCase.
    #[test]
    fn parse_index_round_trip() {
        let text = full_index().to_string();
        let out = parsed(&source_parse_index(&text));
        let expected =
            serde_json::to_value(sources::parse_index(text.as_bytes()).unwrap()).unwrap();
        assert_eq!(out, expected);
        assert_eq!(out["hew"], 1);
        assert_eq!(out["name"], "Shop Hardware");
        assert_eq!(out["licenseUrl"], "https://example.com/license");
        assert!(out.get("license_url").is_none());
        assert_eq!(out["items"].as_array().unwrap().len(), 2);
        assert_eq!(out["items"][0]["category"], "Components");
        assert_eq!(out["items"][1]["path"], "Components/item-2.hew");
        assert!(out["plugins"].as_array().unwrap().is_empty());
    }

    /// The answer of a valid index parses again (the serialized form is a
    /// document `parse_index` accepts, per the crate's contract).
    #[test]
    fn parse_index_answer_is_itself_a_valid_index() {
        let out = source_parse_index(&full_index().to_string());
        let again = source_parse_index(&out);
        assert_eq!(parsed(&again), parsed(&out));
    }

    /// 6.1: `hew` above the supported version is `index_too_new`, even when
    /// the rest of the shape is unreadable.
    #[test]
    fn parse_index_too_new() {
        let out = source_parse_index(&json!({ "hew": 2, "items": "who knows" }).to_string());
        assert_eq!(code_of(&out), "index_too_new");
    }

    /// 6.1: an index over 4 MB is `index_too_large`.
    #[test]
    fn parse_index_too_large() {
        let pad = "a".repeat(sources::MAX_INDEX_BYTES + 1);
        let text = json!({ "hew": 1, "name": "x", "items": [], "pad": pad }).to_string();
        assert_eq!(code_of(&source_parse_index(&text)), "index_too_large");
    }

    /// 6.1: any other violation is `index_invalid` with the item's 0-based
    /// position in `error.item` where it applies.
    #[test]
    fn parse_index_invalid_names_the_item() {
        let mut bad = item(2);
        bad["category"] = json!("Gadgets");
        let mut idx = full_index();
        idx["items"] = json!([item(1), bad]);
        let e = error_of(&source_parse_index(&idx.to_string()));
        assert_eq!(e["code"], "index_invalid");
        assert_eq!(e["item"], 1);
    }

    /// 6.1: a top-level violation is `index_invalid` with no item position.
    #[test]
    fn parse_index_invalid_without_item() {
        let e = error_of(&source_parse_index(&json!({ "hew": 1 }).to_string()));
        assert_eq!(e["code"], "index_invalid");
        assert!(e.get("item").is_none() || e["item"].is_null());
    }

    /// Text that is not JSON is the document being invalid, so
    /// `index_invalid` (not `bad_input`); so is JSON that is not an object.
    #[test]
    fn parse_index_not_json_is_index_invalid() {
        for text in ["", "not json", "[]", "null", "42", "{", "\u{0}"] {
            assert_eq!(
                code_of(&source_parse_index(text)),
                "index_invalid",
                "{text:?}"
            );
        }
    }

    /// 6.1 hostile items (traversal path, duplicate ids) stay refused through
    /// the export.
    #[test]
    fn parse_index_hostile_items_refused() {
        let mut traversal = item(1);
        traversal["path"] = json!("../../etc/passwd.hew");
        let mut idx = full_index();
        idx["items"] = json!([traversal]);
        assert_eq!(
            code_of(&source_parse_index(&idx.to_string())),
            "index_invalid"
        );

        idx["items"] = json!([item(1), item(1)]);
        assert_eq!(
            code_of(&source_parse_index(&idx.to_string())),
            "index_invalid"
        );
    }

    // ---- source_parse_ref (6.2) --------------------------------------------

    /// 6.2: a known-host reference is one candidate in canonical form; the
    /// answer equals the crate's own, serialized.
    #[test]
    fn parse_ref_github_one_candidate() {
        let c = candidates("github:o/r//dir@main");
        assert_eq!(c.len(), 1);
        let expected =
            serde_json::to_value(sources::parse_source_ref("github:o/r//dir@main").unwrap())
                .unwrap();
        assert_eq!(Value::Array(c.clone()), expected);
        assert_eq!(c[0]["kind"], "github");
        assert_eq!(c[0]["host"], "github.com");
        assert_eq!(c[0]["repo"], "o/r");
        assert_eq!(c[0]["dir"], "dir");
        assert_eq!(c[0]["git_ref"], "main");
        assert_eq!(c[0]["canonical"], "github:o/r//dir@main");
    }

    /// 6.2: an https URL on an unknown host is three candidates, in probe
    /// order web, forgejo, gitlab.
    #[test]
    fn parse_ref_unknown_https_host_is_three_candidates_in_order() {
        let c = candidates("https://example.com/owner/repo");
        let kinds: Vec<&str> = c.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["web", "forgejo", "gitlab"]);
        let expected = serde_json::to_value(
            sources::parse_source_ref("https://example.com/owner/repo").unwrap(),
        )
        .unwrap();
        assert_eq!(Value::Array(c), expected);
    }

    /// A one-segment URL can only be a web source: one candidate.
    #[test]
    fn parse_ref_one_segment_https_is_web_only() {
        let c = candidates("https://example.com/parts");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0]["kind"], "web");
    }

    /// Without an `@ref` the candidate's `git_ref` is null.
    #[test]
    fn parse_ref_no_ref_is_null() {
        let c = candidates("github:o/r");
        assert!(c[0]["git_ref"].is_null());
    }

    /// 6.2: refusals are typed. Empty, a query string, and anything else
    /// (unknown scheme, plain `http`, missing repo).
    #[test]
    fn parse_ref_error_codes() {
        assert_eq!(code_of(&source_parse_ref("")), "ref_empty");
        assert_eq!(code_of(&source_parse_ref("   ")), "ref_empty");
        assert_eq!(
            code_of(&source_parse_ref("https://github.com/o/r?tab=x")),
            "ref_query_refused"
        );
        assert_eq!(
            code_of(&source_parse_ref("ftp://example.com/x")),
            "ref_invalid"
        );
        assert_eq!(
            code_of(&source_parse_ref("http://example.com/x")),
            "ref_invalid"
        );
        assert_eq!(
            code_of(&source_parse_ref("github:onlyowner")),
            "ref_invalid"
        );
    }

    // ---- source_urls (6.2, 6.4) --------------------------------------------

    /// The three URL kinds equal the crate's `resolve_url`, `index_url` and
    /// `file_url`, per path in order, for a git source pinned to a commit.
    #[test]
    fn urls_round_trip_for_a_git_source() {
        let r = candidates("github:o/r//dir@main").remove(0);
        let paths = ["Components/a.hew", "thumbnails/x.png", "b.hew"];
        let out = parsed(&urls_for(&r, &paths, Some(COMMIT)));
        let sr = sources::parse_source_ref("github:o/r//dir@main")
            .unwrap()
            .remove(0);
        assert_eq!(out["resolve"], sources::resolve_url(&sr).unwrap().as_str());
        assert_eq!(out["index"], sources::index_url(&sr, Some(COMMIT)).as_str());
        let files = out["files"].as_array().unwrap();
        assert_eq!(files.len(), paths.len());
        for (got, p) in files.iter().zip(paths) {
            assert_eq!(
                got,
                sources::file_url(&sr, p, Some(COMMIT)).unwrap().as_str()
            );
        }
        assert_eq!(out.as_object().unwrap().len(), 3);
    }

    /// With no commit the reads use the ref (or HEAD), as the crate does.
    #[test]
    fn urls_without_a_commit() {
        let r = candidates("github:o/r").remove(0);
        let out = parsed(&urls_for(&r, &["a.hew"], None));
        let sr = sources::parse_source_ref("github:o/r").unwrap().remove(0);
        assert_eq!(out["index"], sources::index_url(&sr, None).as_str());
        assert_eq!(
            out["files"][0],
            sources::file_url(&sr, "a.hew", None).unwrap().as_str()
        );
    }

    /// 6.2: a `web` source has no resolve request: `resolve` is null.
    #[test]
    fn urls_web_has_null_resolve() {
        let r = candidates("web:https://example.com/parts/hew.json").remove(0);
        let out = parsed(&urls_for(&r, &["a.hew"], None));
        assert!(out["resolve"].is_null());
        let sr = sources::parse_source_ref("web:https://example.com/parts/hew.json")
            .unwrap()
            .remove(0);
        assert_eq!(out["index"], sources::index_url(&sr, None).as_str());
    }

    /// No paths: `files` is an empty array, `index` still present.
    #[test]
    fn urls_empty_paths() {
        let r = candidates("github:o/r").remove(0);
        let out = parsed(&urls_for(&r, &[], None));
        assert_eq!(out["files"], json!([]));
        assert!(out["index"].is_string());
    }

    /// 6.1 `path` rule: a path that escapes the folder is `path_invalid`, and
    /// no URLs come back for the good paths beside it.
    #[test]
    fn urls_traversal_path_is_path_invalid() {
        let r = candidates("github:o/r//dir").remove(0);
        for bad in [
            "../x.hew",
            "a/../../x.hew",
            "/abs.hew",
            "a\\b.hew",
            "%2e%2e/x.hew",
            "a/%2E%2E/%2E%2E/x.hew",
            "a?x.hew",
            ".hidden/x.hew",
            "",
        ] {
            let out = urls_for(&r, &["ok.hew", bad], Some(COMMIT));
            assert_eq!(code_of(&out), "path_invalid", "{bad:?}");
        }
    }

    /// Malformed or wrongly shaped input is `bad_input`, never a panic.
    #[test]
    fn urls_bad_input() {
        let good_ref = candidates("github:o/r").remove(0).to_string();
        let good_paths = "[\"a.hew\"]";
        for bad_ref in [
            "",
            "not json",
            "[]",
            "null",
            "{}",
            "{\"kind\":\"nope\"}",
            "\"github:o/r\"",
        ] {
            let out = source_urls(bad_ref, good_paths, None);
            assert_eq!(code_of(&out), "bad_input", "ref {bad_ref:?}");
        }
        for bad_paths in [
            "",
            "not json",
            "{}",
            "null",
            "\"a.hew\"",
            "[1,2]",
            "[null]",
            "[[\"a\"]]",
        ] {
            let out = source_urls(&good_ref, bad_paths, None);
            assert_eq!(code_of(&out), "bad_input", "paths {bad_paths:?}");
        }
    }

    /// `source_urls` takes only `canonical` from `ref_json`, re-parses it with
    /// `parse_source_ref`, and requires the `ref_json` fields to equal the one
    /// matching candidate. A hand-built reference whose fields do not follow
    /// from its canonical form is `ref_invalid`: no URL is built from it.
    #[test]
    fn urls_hostile_ref_json_is_ref_invalid() {
        let hostile = [
            json!({"kind":"github","host":"evil.example","repo":"../../x","dir":"../..","git_ref":null,"canonical":"github:o/r"}),
            json!({"kind":"github","host":"user:pw@evil.example","repo":"o/r","dir":"","git_ref":"a/../b","canonical":"x"}),
            json!({"kind":"github","host":"github.com","repo":"o/r\n#frag?q=1","dir":"d\\e","git_ref":"main?x=1","canonical":""}),
            json!({"kind":"gitlab","host":"127.0.0.1","repo":"g/p","dir":"","git_ref":null,"canonical":"x"}),
            json!({"kind":"forgejo","host":"169.254.169.254","repo":"o/r","dir":"","git_ref":"main","canonical":"github:o/r"}),
            json!({"kind":"web","host":"[::1]","repo":"x","dir":"","git_ref":null,"canonical":"web:https://example.com/hew.json"}),
        ];
        for r in hostile {
            let out = source_urls(&r.to_string(), "[\"a.hew\"]", Some(COMMIT.to_owned()));
            assert_eq!(code_of(&out), "ref_invalid", "{r}");
        }
        // Fields of the wrong JSON type cannot be read at all: bad_input or
        // ref_invalid, an error either way, never a URL.
        let typed =
            json!({"kind":"github","host":1,"repo":null,"dir":[],"git_ref":{},"canonical":false});
        let out = source_urls(&typed.to_string(), "[\"a.hew\"]", None);
        assert!(["bad_input", "ref_invalid"].contains(&code_of(&out).as_str()));
    }

    /// Each of repo, host, dir, git_ref and kind disagreeing with a valid
    /// `canonical` is `ref_invalid`.
    #[test]
    fn urls_ref_fields_must_agree_with_canonical() {
        let good = candidates("github:o/r//dir@main").remove(0);
        for (field, value) in [
            ("host", json!("evil.example")),
            ("repo", json!("x/y")),
            ("dir", json!("other")),
            ("git_ref", json!("dev")),
            ("git_ref", Value::Null),
            ("kind", json!("gitlab")),
        ] {
            let mut r = good.clone();
            r[field] = value;
            let out = source_urls(&r.to_string(), "[\"a.hew\"]", None);
            assert_eq!(code_of(&out), "ref_invalid", "{field}");
        }
        // And a canonical that is not a reference at all.
        let mut r = good.clone();
        r["canonical"] = json!("not a reference");
        assert_eq!(
            code_of(&source_urls(&r.to_string(), "[]", None)),
            "ref_invalid"
        );
    }

    /// A canonical form that parses to several candidates (an unknown https
    /// host) selects the one whose fields equal `ref_json`'s: every real
    /// candidate works, with its own URLs; a mixed-up one is `ref_invalid`.
    #[test]
    fn urls_multi_candidate_canonical_is_matched_by_fields() {
        let cs = candidates("https://example.com/owner/repo");
        assert_eq!(cs.len(), 3);
        for c in &cs {
            let out = parsed(&urls_for(c, &["a.hew"], None));
            assert!(out["index"].is_string(), "{c}");
        }
        let kinds: Vec<_> = cs.iter().map(|c| c["kind"].as_str().unwrap()).collect();
        let indexes: Vec<String> = cs
            .iter()
            .map(|c| {
                parsed(&urls_for(c, &[], None))["index"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(kinds, ["web", "forgejo", "gitlab"]);
        assert_eq!(indexes.len(), 3);
        assert!(indexes[0] != indexes[1] && indexes[1] != indexes[2] && indexes[0] != indexes[2]);

        // The forgejo candidate's fields under the web candidate's canonical
        // form (all three candidates share the pasted text's canonical only
        // if the crate says so; compare against the matching one by kind).
        // A canonical that is itself the pasted text parses to all three; the
        // fields choose the match.
        for c in &cs {
            let mut raw = c.clone();
            raw["canonical"] = json!("https://example.com/owner/repo");
            assert!(!is_error(&urls_for(&raw, &[], None)), "{c}");
        }
        let mut stray = cs[1].clone();
        stray["canonical"] = json!("https://example.com/owner/repo");
        stray["host"] = json!("other.example");
        assert_eq!(code_of(&urls_for(&stray, &[], None)), "ref_invalid");

        let mut mixed = cs[0].clone();
        mixed["kind"] = json!("gitlab");
        assert_eq!(code_of(&urls_for(&mixed, &[], None)), "ref_invalid");
        let mut other_host = cs[1].clone();
        other_host["host"] = json!("other.example");
        assert_eq!(code_of(&urls_for(&other_host, &[], None)), "ref_invalid");
        let mut other_repo = cs[2].clone();
        other_repo["repo"] = json!("not/there");
        assert_eq!(code_of(&urls_for(&other_repo, &[], None)), "ref_invalid");
    }

    /// Guard: `is_public_host` is not applied in the app, so a reference on a
    /// loopback or link-local host is not refused here; its URLs are built on
    /// that host.
    #[test]
    fn urls_do_not_refuse_local_hosts() {
        for (text, host) in [
            ("gitlab:https://127.0.0.1/g/p", "127.0.0.1"),
            ("forgejo:169.254.169.254/o/r@main", "169.254.169.254"),
            ("gitlab:https://localhost/g/p", "localhost"),
        ] {
            let c = candidates(text).remove(0);
            let out = parsed(&urls_for(&c, &["a.hew"], None));
            let url = url::Url::parse(out["index"].as_str().unwrap()).unwrap();
            assert_eq!(url.host_str(), Some(host), "{text}");
            assert_eq!(url.scheme(), "https");
        }
    }

    /// A commit is spliced into later request URLs, so it must be 40 or 64
    /// lowercase hex characters (the rule of `commit_from_response`): anything
    /// else is `commit_invalid`, with no URLs.
    #[test]
    fn urls_hostile_commit_is_commit_invalid() {
        let r = candidates("github:o/r").remove(0);
        for bad in [
            "../../evil?x=1#",
            "",
            "main",
            &COMMIT[..39],
            &format!("{COMMIT}0"),
            &COMMIT.to_uppercase(),
            &format!("{}g", &COMMIT[..39]),
        ] {
            let out = source_urls(&r.to_string(), "[\"a.hew\"]", Some(bad.to_owned()));
            assert_eq!(code_of(&out), "commit_invalid", "{bad:?}");
        }
        // A 64-hex commit is fine.
        let ok = source_urls(&r.to_string(), "[]", Some("a".repeat(64)));
        assert!(!is_error(&ok));
    }

    // ---- source_commit -------------------------------------------------------

    const GH: &str = "7fd1a60b01f91b314f59955a4e4d4e80d8edf11d";
    const GL: &str = "13dd661da0a5d40364b42bad39af78686deaa073";
    const CB: &str = "7d55578694c587995e8ac43b7e7e236ecda4e00a";

    fn commit(kind: &str, body: &str) -> Value {
        let out = source_commit(kind, body);
        let v = parsed(&out);
        assert_eq!(v.as_object().unwrap().len(), 1, "{out}");
        v["commit"].clone()
    }

    /// Each kind reads its own field and answers like the crate.
    #[test]
    fn commit_per_kind() {
        assert_eq!(
            commit("github", &json!({"sha": GH, "node_id": "x"}).to_string()),
            GH
        );
        assert_eq!(commit("github", GH), GH); // bare vnd.github.sha body
        assert_eq!(
            commit(
                "gitlab",
                &json!({"id": GL, "short_id": "13dd661d"}).to_string()
            ),
            GL
        );
        assert!(commit("forgejo", &json!({"commit": {"id": CB}}).to_string()).is_null());
        assert_eq!(commit("forgejo", &json!({"sha": CB}).to_string()), CB);
        let sha256 = "a".repeat(64);
        assert_eq!(
            commit("github", &json!({"sha": sha256}).to_string()),
            sha256.as_str()
        );
        let body = json!({"sha": GH}).to_string();
        assert_eq!(
            commit("github", &body).as_str().map(str::to_owned),
            sources::commit_from_response(sources::Kind::Github, body.as_bytes())
        );
    }

    /// A web source has no commit, whatever the body says.
    #[test]
    fn commit_web_is_null() {
        assert!(commit("web", &json!({"sha": GH}).to_string()).is_null());
    }

    /// Short, uppercase, spliced or unrecognized bodies are `null`, never an
    /// error object.
    #[test]
    fn commit_unrecognized_is_null() {
        let bad = [
            String::new(),
            "{}".into(),
            "[]".into(),
            "null".into(),
            "not json".into(),
            "<html>rate limited</html>".into(),
            json!({"sha": &GH[..12]}).to_string(),
            json!({"sha": GH.to_uppercase()}).to_string(),
            json!({"sha": format!("{GH}/../x?y#")}).to_string(),
            json!({"sha": 5}).to_string(),
            json!({"id": null}).to_string(),
            "\u{0}".into(),
            "{".repeat(100_000),
        ];
        for kind in ["github", "gitlab", "forgejo", "web"] {
            for b in &bad {
                assert!(
                    commit(kind, b).is_null(),
                    "{kind} {:?}",
                    &b[..b.len().min(40)]
                );
            }
        }
    }

    /// Only an unknown kind is an error: `bad_input`.
    #[test]
    fn commit_unknown_kind_is_bad_input() {
        for kind in ["", "svn", "GitHub", "gitea", "github "] {
            assert_eq!(code_of(&source_commit(kind, "{}")), "bad_input", "{kind:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn commit_never_panics(k in prop_oneof![Just("github".to_owned()), Just("gitlab".to_owned()), Just("forgejo".to_owned()), Just("web".to_owned()), ".*"], b in ".*") {
            check_answer(&source_commit(&k, &b));
        }
    }

    // ---- never panic ---------------------------------------------------------

    /// Every answer is JSON, and an error answer is well-formed.
    fn check_answer(out: &str) {
        let v: Value = serde_json::from_str(out).expect("answer is JSON");
        if v.get("error").is_some() {
            error_of(out);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn parse_index_never_panics(s in ".*") {
            check_answer(&source_parse_index(&s));
        }

        #[test]
        fn parse_ref_never_panics(s in ".*") {
            check_answer(&source_parse_ref(&s));
        }

        #[test]
        fn urls_never_panic(a in ".*", b in ".*", c in proptest::option::of(".*")) {
            check_answer(&source_urls(&a, &b, c));
        }

        /// Random strings after a real reference and real paths exercise the
        /// URL builder, not only the input parsing.
        #[test]
        fn urls_with_random_paths_never_panic(p in proptest::collection::vec(".*", 0..4), c in proptest::option::of(".*")) {
            let r = json!({"kind":"github","host":"github.com","repo":"o/r","dir":"d","git_ref":"main","canonical":"github:o/r//d@main"});
            check_answer(&source_urls(&r.to_string(), &serde_json::to_string(&p).unwrap(), c));
        }
    }
}
