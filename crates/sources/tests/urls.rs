//! 6.2 / 6.4 request URLs. Only what 6.2 states outright is specified here;
//! the GitLab and Forgejo request shapes await S0.
mod common;
use common::hostile_paths;
use sources::{
    Kind, SourceRef, commit_from_response, file_url, index_url, parse_source_ref, resolve_url,
};

fn one(text: &str) -> SourceRef {
    parse_source_ref(text).unwrap().remove(0)
}

// ---- resolve_url -----------------------------------------------------------

#[test]
fn web_sources_have_no_resolve_request() {
    assert_eq!(resolve_url(&one("web:https://example.com/p")), None);
    assert_eq!(resolve_url(&one("https://example.com/p/hew.json")), None);
}

#[test]
fn git_sources_have_a_resolve_request_over_https() {
    for s in [
        "github:o/r",
        "github:o/r@main",
        "gitlab:g/p",
        "gitlab:git.example.org/g/p@v1",
        "codeberg:o/r@main",
        "forgejo:git.example.org/o/r@dev",
    ] {
        let u = resolve_url(&one(s)).unwrap_or_else(|| panic!("{s}: no resolve url"));
        assert_eq!(u.scheme(), "https", "{s}");
        assert_eq!(u.fragment(), None, "{s}");
    }
}

// ---- hosts -----------------------------------------------------------------

#[test]
fn github_files_are_read_from_the_raw_host() {
    let s = one("github:Owner/Repo//dir@main");
    for u in [
        index_url(&s, Some("abc123")),
        index_url(&s, None),
        file_url(&s, "Components/a.hew", Some("abc123")).unwrap(),
        file_url(&s, "Components/a.hew", None).unwrap(),
    ] {
        assert_eq!(u.scheme(), "https");
        assert_eq!(u.host_str(), Some("raw.githubusercontent.com"), "{u}");
    }
}

#[test]
fn github_urls_name_the_repo_the_dir_and_the_commit() {
    let s = one("github:Owner/Repo//dir/sub@main");
    let u = index_url(&s, Some("0123abcd"));
    assert!(u.path().starts_with("/owner/repo/"), "{u}");
    assert!(
        u.path().contains("/0123abcd/"),
        "the commit pins the read: {u}"
    );
    assert!(u.path().ends_with("/dir/sub/hew.json"), "{u}");
    let f = file_url(&s, "Components/a.hew", Some("0123abcd")).unwrap();
    assert!(f.path().ends_with("/dir/sub/Components/a.hew"), "{f}");
    assert!(f.path().contains("/0123abcd/"), "{f}");
}

#[test]
fn without_a_commit_the_given_ref_is_read() {
    // 6.2: "If the resolve request is refused for rate limiting, the check
    // reads at the ref."
    let s = one("github:o/r@release-1");
    let u = index_url(&s, None);
    assert!(u.path().contains("/release-1/"), "{u}");
}

#[test]
fn web_urls_are_built_from_the_canonical_form() {
    let s = one("web:https://Example.com/parts/hew.json");
    assert_eq!(
        index_url(&s, None).as_str(),
        "https://example.com/parts/hew.json"
    );
    assert_eq!(
        file_url(&s, "Components/a.hew", None).unwrap().as_str(),
        "https://example.com/parts/Components/a.hew"
    );
    let root = one("web:https://example.com");
    assert_eq!(
        index_url(&root, None).as_str(),
        "https://example.com/hew.json"
    );
    assert_eq!(
        file_url(&root, "a.hew", None).unwrap().as_str(),
        "https://example.com/a.hew"
    );
}

#[test]
fn web_urls_ignore_a_commit() {
    // "A `web` source has no commit."
    let s = one("web:https://example.com/parts");
    assert_eq!(index_url(&s, Some("deadbeef")), index_url(&s, None));
    assert_eq!(
        file_url(&s, "a.hew", Some("deadbeef")).unwrap(),
        file_url(&s, "a.hew", None).unwrap()
    );
}

#[test]
fn file_url_encodes_spaces_and_never_leaves_one_raw() {
    let w = one("web:https://example.com/parts");
    let u = file_url(&w, "Some Folder/a b.hew", None).unwrap();
    assert!(!u.as_str().contains(' '), "{u}");
    assert_eq!(
        u.as_str(),
        "https://example.com/parts/Some%20Folder/a%20b.hew"
    );
    let g = one("github:o/r");
    let u = file_url(&g, "a b.hew", Some("c0ffee")).unwrap();
    assert!(!u.as_str().contains(' '));
    assert!(u.path().ends_with("/a%20b.hew"), "{u}");
}

#[test]
fn file_url_treats_percent_in_a_name_as_data() {
    let w = one("web:https://example.com/parts");
    let u = file_url(&w, "100%.hew", None).unwrap();
    assert!(u.path().ends_with("/100%25.hew"), "{u}");
    assert_eq!(u.query(), None);
    assert_eq!(u.fragment(), None);
}

// ---- hostile paths ----------------------------------------------------------

#[test]
fn file_url_refuses_every_hostile_path_for_every_kind() {
    let sources = [
        one("github:o/r"),
        one("github:o/r//d@main"),
        one("gitlab:git.example.org/g/p"),
        one("codeberg:o/r"),
        one("web:https://example.com/parts"),
    ];
    for s in &sources {
        for p in hostile_paths() {
            let shown: String = p.chars().take(50).collect();
            assert!(
                file_url(s, &p, Some("abc123")).is_err(),
                "{}: accepted {shown:?}",
                s.canonical
            );
            assert!(
                file_url(s, &p, None).is_err(),
                "{}: accepted {shown:?}",
                s.canonical
            );
        }
    }
}

#[test]
fn file_url_refuses_percent_encoded_traversal() {
    // The URL parser treats %2e as a dot segment, so an encoded `..` would
    // escape the folder if it were passed through.
    let w = one("web:https://example.com/parts");
    for p in [
        "%2e%2e/x.hew",
        "%2E%2E/x.hew",
        "a/%2e%2e/b.hew",
        ".%2e/x.hew",
        "%2e./x.hew",
        "a/%2e%2e",
        "%2e/x.hew",
        "a%2fb.hew",
        "a%2Fb.hew",
        "a%5cb.hew",
        "a%5Cb.hew",
    ] {
        assert!(file_url(&w, p, None).is_err(), "accepted {p:?}");
    }
}

#[test]
fn file_url_never_changes_host_or_scheme() {
    let w = one("web:https://example.com/parts");
    for p in [
        "a.hew",
        "a/b.hew",
        "x y/z.png",
        "\u{e9}/\u{e9}.hew",
        "a@b.hew",
        "a%b.hew",
        "evil.com@x.hew",
    ] {
        if let Ok(u) = file_url(&w, p, None) {
            assert_eq!(u.scheme(), "https");
            assert_eq!(u.host_str(), Some("example.com"), "{p:?} -> {u}");
            assert_eq!(u.username(), "");
            assert!(u.path().starts_with("/parts/"), "{u}");
        }
    }
}

#[test]
fn file_url_accepts_ordinary_paths_and_thumbnails() {
    let w = one("web:https://example.com/parts");
    for p in [
        "a.hew",
        "Components/butt-hinge-0b6f1c.hew",
        "thumbnails/abcdef.png",
        "a.b/c.hew",
    ] {
        assert!(file_url(&w, p, None).is_ok(), "{p}");
    }
    let at_limit = format!("{}.hew", "a".repeat(1_024 - 4));
    assert!(file_url(&w, &at_limit, None).is_ok());
}

// ---- commit_from_response --------------------------------------------------

#[test]
fn a_body_with_no_commit_gives_none() {
    for kind in [Kind::Github, Kind::Gitlab, Kind::Forgejo, Kind::Web] {
        for body in [
            &b""[..],
            b"{}",
            b"[]",
            b"null",
            b"not json",
            b"\xff\xfe",
            b"{\"sha\":5}",
            b"{\"id\":null}",
            b"<html>rate limited</html>",
        ] {
            assert_eq!(commit_from_response(kind, body), None, "{kind:?} {body:?}");
        }
    }
}

#[test]
fn a_web_source_has_no_commit() {
    assert_eq!(
        commit_from_response(
            Kind::Web,
            b"{\"sha\":\"0123456789abcdef0123456789abcdef01234567\"}"
        ),
        None
    );
}

#[test]
fn a_huge_body_is_handled() {
    let big = vec![b'{'; 5_000_000];
    for kind in [Kind::Github, Kind::Gitlab, Kind::Forgejo] {
        assert_eq!(commit_from_response(kind, &big), None);
    }
}

// ---- S0-recorded request shapes -------------------------------------------
//
// Source: docs/design/shared-library-sources-s0-results.md (probed with real
// requests). Only recorded shapes are specified. `Kind` has no Gitea variant,
// so every `forgejo` host resolves through the one `git/commits/{ref}`
// request (field `sha`); see tests/forgejo_resolve.rs. Files read the same
// way on all of them.

const GH_SHA: &str = "7fd1a60b01f91b314f59955a4e4d4e80d8edf11d";
const GL_SHA: &str = "13dd661da0a5d40364b42bad39af78686deaa073";
const CB_SHA: &str = "7d55578694c587995e8ac43b7e7e236ecda4e00a";
const GT_SHA: &str = "397e6300ec93937b7745ebcc19b95ca9cb30568b";
const SPACED: &str = "dir with space/caf\u{e9} notes.md";

#[test]
fn resolve_urls_by_kind() {
    let url = |s: &str| resolve_url(&one(s)).unwrap().to_string();
    assert_eq!(
        url("github:octocat/hello-world@master"),
        "https://api.github.com/repos/octocat/hello-world/commits/master"
    );
    assert_eq!(
        url("gitlab:gitlab.com/gitlab-org/cli@main"),
        "https://gitlab.com/api/v4/projects/gitlab-org%2Fcli/repository/commits/main"
    );
    assert_eq!(
        url("codeberg:codeberg/Community@main"),
        "https://codeberg.org/api/v1/repos/codeberg/Community/git/commits/main?stat=false&verification=false&files=false"
    );
    assert_eq!(
        url("forgejo:gitea.com/gitea/tea@main"),
        "https://gitea.com/api/v1/repos/gitea/tea/git/commits/main?stat=false&verification=false&files=false"
    );
}

#[test]
fn resolve_url_for_a_gitlab_subgroup_encodes_every_slash() {
    assert_eq!(
        resolve_url(&one("gitlab:git.example.org/group/sub/project@v1"))
            .unwrap()
            .as_str(),
        "https://git.example.org/api/v4/projects/group%2Fsub%2Fproject/repository/commits/v1"
    );
}

#[test]
fn a_self_hosted_forgejo_resolves_through_git_commits_like_codeberg() {
    assert_eq!(
        resolve_url(&one("forgejo:git.example.org/o/r@dev"))
            .unwrap()
            .as_str(),
        "https://git.example.org/api/v1/repos/o/r/git/commits/dev?stat=false&verification=false&files=false"
    );
}

#[test]
fn resolve_url_ignores_dir() {
    assert_eq!(
        resolve_url(&one("github:octocat/hello-world//docs@master")),
        resolve_url(&one("github:octocat/hello-world@master"))
    );
}

#[test]
fn github_index_and_file_urls() {
    let root = one("github:octocat/hello-world@master");
    assert_eq!(
        index_url(&root, Some(GH_SHA)).as_str(),
        format!("https://raw.githubusercontent.com/octocat/hello-world/{GH_SHA}/hew.json")
    );
    let sub = one("github:octocat/hello-world//docs@master");
    assert_eq!(
        index_url(&sub, Some(GH_SHA)).as_str(),
        format!("https://raw.githubusercontent.com/octocat/hello-world/{GH_SHA}/docs/hew.json")
    );
    assert_eq!(
        file_url(&root, SPACED, Some(GH_SHA)).unwrap().as_str(),
        format!(
            "https://raw.githubusercontent.com/octocat/hello-world/{GH_SHA}/dir%20with%20space/caf%C3%A9%20notes.md"
        )
    );
    assert_eq!(
        file_url(&sub, "a.hew", Some(GH_SHA)).unwrap().as_str(),
        format!("https://raw.githubusercontent.com/octocat/hello-world/{GH_SHA}/docs/a.hew")
    );
    // Recorded: a branch name in place of the sha also reads.
    assert_eq!(
        file_url(&root, "README", None).unwrap().as_str(),
        "https://raw.githubusercontent.com/octocat/hello-world/master/README"
    );
}

#[test]
fn gitlab_index_and_file_urls() {
    let root = one("gitlab:gitlab.com/gitlab-org/cli@main");
    let base = "https://gitlab.com/api/v4/projects/gitlab-org%2Fcli/repository/files";
    assert_eq!(
        index_url(&root, Some(GL_SHA)).as_str(),
        format!("{base}/hew.json/raw?ref={GL_SHA}")
    );
    let sub = one("gitlab:gitlab.com/gitlab-org/cli//docs@main");
    assert_eq!(
        index_url(&sub, Some(GL_SHA)).as_str(),
        format!("{base}/docs%2Fhew.json/raw?ref={GL_SHA}")
    );
    assert_eq!(
        file_url(&root, SPACED, Some(GL_SHA)).unwrap().as_str(),
        format!("{base}/dir%20with%20space%2Fcaf%C3%A9%20notes.md/raw?ref={GL_SHA}")
    );
    assert_eq!(
        file_url(&sub, "a.hew", Some(GL_SHA)).unwrap().as_str(),
        format!("{base}/docs%2Fa.hew/raw?ref={GL_SHA}")
    );
    // Recorded: `ref=main` (a branch) also reads.
    assert_eq!(
        file_url(&root, "README.md", None).unwrap().as_str(),
        format!("{base}/README.md/raw?ref=main")
    );
}

#[test]
fn forgejo_and_gitea_index_and_file_urls() {
    for (src, sub_src, host, repo, sha) in [
        (
            "codeberg:codeberg/Community@main",
            "codeberg:codeberg/Community//docs@main",
            "codeberg.org",
            "codeberg/Community",
            CB_SHA,
        ),
        (
            "forgejo:gitea.com/gitea/tea@main",
            "forgejo:gitea.com/gitea/tea//docs@main",
            "gitea.com",
            "gitea/tea",
            GT_SHA,
        ),
    ] {
        let base = format!("https://{host}/api/v1/repos/{repo}/raw");
        let root = one(src);
        let sub = one(sub_src);
        assert_eq!(
            index_url(&root, Some(sha)).as_str(),
            format!("{base}/hew.json?ref={sha}")
        );
        assert_eq!(
            index_url(&sub, Some(sha)).as_str(),
            format!("{base}/docs/hew.json?ref={sha}")
        );
        assert_eq!(
            file_url(&root, SPACED, Some(sha)).unwrap().as_str(),
            format!("{base}/dir%20with%20space/caf%C3%A9%20notes.md?ref={sha}")
        );
        assert_eq!(
            file_url(&sub, "a.hew", Some(sha)).unwrap().as_str(),
            format!("{base}/docs/a.hew?ref={sha}")
        );
        assert_eq!(
            file_url(&root, "README.md", Some(sha)).unwrap().as_str(),
            format!("{base}/README.md?ref={sha}")
        );
    }
}

#[test]
fn web_file_urls_encode_spaces_and_non_ascii() {
    let w = one("web:https://example.com/parts");
    assert_eq!(
        file_url(&w, SPACED, None).unwrap().as_str(),
        "https://example.com/parts/dir%20with%20space/caf%C3%A9%20notes.md"
    );
}

#[test]
fn commit_from_response_github() {
    let json =
        format!("{{\"sha\":\"{GH_SHA}\",\"node_id\":\"x\",\"commit\":{{\"message\":\"m\"}}}}");
    assert_eq!(
        commit_from_response(Kind::Github, json.as_bytes()).as_deref(),
        Some(GH_SHA)
    );
    // `Accept: application/vnd.github.sha` answers the bare sha.
    assert_eq!(
        commit_from_response(Kind::Github, GH_SHA.as_bytes()).as_deref(),
        Some(GH_SHA)
    );
}

#[test]
fn commit_from_response_gitlab_reads_id_not_short_id() {
    let json = format!("{{\"id\":\"{GL_SHA}\",\"short_id\":\"13dd661d\",\"title\":\"t\"}}");
    assert_eq!(
        commit_from_response(Kind::Gitlab, json.as_bytes()).as_deref(),
        Some(GL_SHA)
    );
    assert_eq!(
        commit_from_response(Kind::Gitlab, b"{\"short_id\":\"13dd661d\"}"),
        None
    );
}

#[test]
fn commit_from_response_forgejo_reads_the_top_level_sha() {
    // `git/commits/{ref}` on every host: top-level `sha`.
    let cb = format!("{{\"sha\":\"{CB_SHA}\",\"url\":\"u\"}}");
    assert_eq!(
        commit_from_response(Kind::Forgejo, cb.as_bytes()).as_deref(),
        Some(CB_SHA)
    );
    let gt = format!("{{\"sha\":\"{GT_SHA}\",\"url\":\"u\"}}");
    assert_eq!(
        commit_from_response(Kind::Forgejo, gt.as_bytes()).as_deref(),
        Some(GT_SHA)
    );
}

#[test]
fn commit_from_response_does_not_cross_read_kinds() {
    let gl = format!("{{\"id\":\"{GL_SHA}\"}}");
    assert_eq!(commit_from_response(Kind::Github, gl.as_bytes()), None);
    let cb = format!("{{\"commit\":{{\"id\":\"{CB_SHA}\"}}}}");
    assert_eq!(commit_from_response(Kind::Github, cb.as_bytes()), None);
    assert_eq!(commit_from_response(Kind::Gitlab, cb.as_bytes()), None);
}

/// Ruled: a commit is returned only when it is 40 or 64 lowercase hex
/// characters, because it is interpolated into later request URLs.
fn body_for(kind: Kind, commit: &str) -> String {
    match kind {
        Kind::Github => format!("{{\"sha\":{}}}", serde_json::json!(commit)),
        Kind::Gitlab => format!("{{\"id\":{}}}", serde_json::json!(commit)),
        _ => format!("{{\"sha\":{}}}", serde_json::json!(commit)),
    }
}

#[test]
fn a_commit_is_exactly_40_or_64_lowercase_hex() {
    for kind in [Kind::Github, Kind::Gitlab, Kind::Forgejo] {
        for good in [GH_SHA.to_string(), "a".repeat(64), "0".repeat(40)] {
            assert_eq!(
                commit_from_response(kind, body_for(kind, &good).as_bytes()).as_deref(),
                Some(good.as_str()),
                "{kind:?} {good}"
            );
        }
        let bad = [
            "".to_string(),
            "7fd1a60".to_string(),
            GH_SHA[..39].to_string(),
            format!("{GH_SHA}a"),
            "a".repeat(63),
            "a".repeat(65),
            GH_SHA.to_uppercase(),
            format!("{}g", &GH_SHA[..39]),
            format!("{} ", &GH_SHA[..39]),
            format!("{}\n", GH_SHA),
            format!("{}/", &GH_SHA[..39]),
            format!("../{}", &GH_SHA[..37]),
            format!("{}..", &GH_SHA[..38]),
            format!("{}?", &GH_SHA[..39]),
            format!("{}#", &GH_SHA[..39]),
            format!("{}%", &GH_SHA[..39]),
            "main".to_string(),
        ];
        for b in bad {
            assert_eq!(
                commit_from_response(kind, body_for(kind, &b).as_bytes()),
                None,
                "{kind:?} accepted {b:?}"
            );
        }
    }
}

#[test]
fn a_bare_github_sha_body_is_held_to_the_same_rule() {
    for bad in [
        "7fd1a60",
        "7FD1A60B01F91B314F59955A4E4D4E80D8EDF11D",
        "../etc/passwd",
        "a/b",
        "x?y",
        "",
    ] {
        assert_eq!(
            commit_from_response(Kind::Github, bad.as_bytes()),
            None,
            "{bad:?}"
        );
    }
    let sha256 = "b".repeat(64);
    assert_eq!(
        commit_from_response(Kind::Github, sha256.as_bytes()).as_deref(),
        Some(sha256.as_str())
    );
}

// S0 UNTESTED, therefore NOT specified here (listed in the oracle report):
// positive index and file fetches; root `hew.json` on GitLab/Codeberg/Gitea;
// resolve with no `@ref` (OPEN QUESTION for the design owner: the default
// branch needs a second request, and its use is not settled); a ref containing `/`; tags
// on forgejo (only `branches/{branch}` was probed); `ref=<branch>` without a
// commit on forgejo hosts; rate-limit refusal shapes; redirects; whether a
// (the 40/64 lowercase hex rule is ruled and specified above); the Forgejo `git/refs/heads` array form.
