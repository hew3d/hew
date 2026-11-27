//! 6.2: every accepted input form, normalization, equality, candidates.
use sources::{Kind, RefError, SourceRef, parse_source_ref};

fn one(text: &str) -> SourceRef {
    let mut v = parse_source_ref(text).unwrap_or_else(|e| panic!("{text:?} refused: {e:?}"));
    assert_eq!(
        v.len(),
        1,
        "{text:?} should yield exactly one candidate: {v:?}"
    );
    v.remove(0)
}

fn canon(text: &str) -> String {
    one(text).canonical
}

#[track_caller]
fn assert_canon(input: &str, want: &str) {
    assert_eq!(canon(input), want, "input {input:?}");
}

// ---- github --------------------------------------------------------------

#[test]
fn github_canonical_forms_round_trip() {
    for c in [
        "github:owner/repo",
        "github:owner/repo//dir",
        "github:owner/repo//dir/sub",
        "github:owner/repo@v1.2",
        "github:owner/repo//dir@main",
    ] {
        assert_canon(c, c);
    }
}

#[test]
fn github_fields() {
    let r = one("github:owner/repo//dir/sub@v1");
    assert_eq!(r.kind, Kind::Github);
    assert_eq!(r.host, "github.com");
    assert_eq!(r.repo, "owner/repo");
    assert_eq!(r.dir, "dir/sub");
    assert_eq!(r.git_ref.as_deref(), Some("v1"));
    let r = one("github:owner/repo");
    assert_eq!(r.dir, "");
    assert_eq!(r.git_ref, None);
}

#[test]
fn github_short_dir_form() {
    assert_canon("github:owner/repo/dir", "github:owner/repo//dir");
    assert_canon("github:owner/repo/dir/sub", "github:owner/repo//dir/sub");
    assert_canon("github:owner/repo/dir@v2", "github:owner/repo//dir@v2");
}

#[test]
fn github_urls() {
    assert_canon("https://github.com/owner/repo", "github:owner/repo");
    assert_canon("https://github.com/owner/repo/", "github:owner/repo");
    assert_canon(
        "https://github.com/owner/repo/tree/main",
        "github:owner/repo@main",
    );
    assert_canon(
        "https://github.com/owner/repo/tree/main/dir",
        "github:owner/repo//dir@main",
    );
    assert_canon(
        "https://github.com/owner/repo/tree/v1.0/dir/sub",
        "github:owner/repo//dir/sub@v1.0",
    );
}

#[test]
fn github_owner_and_repo_are_lowercased_dir_and_ref_keep_case() {
    assert_canon("github:Owner/Repo", "github:owner/repo");
    assert_canon(
        "github:OWNER/REPO//Dir/Sub@Main",
        "github:owner/repo//Dir/Sub@Main",
    );
    assert_canon(
        "https://GitHub.com/Owner/Repo/tree/Main/Dir",
        "github:owner/repo//Dir@Main",
    );
}

#[test]
fn github_no_trailing_slash_in_any_form() {
    assert_canon("github:owner/repo/", "github:owner/repo");
    assert_canon("github:owner/repo//dir/", "github:owner/repo//dir");
    assert_canon(
        "https://github.com/owner/repo/tree/main/dir/",
        "github:owner/repo//dir@main",
    );
}

#[test]
fn github_fragment_is_dropped() {
    assert_canon("https://github.com/owner/repo#readme", "github:owner/repo");
    assert_canon(
        "https://github.com/owner/repo/tree/main/dir#top",
        "github:owner/repo//dir@main",
    );
}

#[test]
fn ref_is_kept_only_when_given() {
    assert!(!canon("github:owner/repo").contains('@'));
    assert!(!canon("https://github.com/owner/repo").contains('@'));
    assert!(!canon("gitlab:g/p").contains('@'));
    assert!(!canon("codeberg:o/r").contains('@'));
    assert!(
        canon("github:owner/repo@main").ends_with("@main"),
        "a given ref is never dropped"
    );
}

// ---- gitlab --------------------------------------------------------------

#[test]
fn gitlab_canonical_forms_round_trip() {
    for c in [
        "gitlab:gitlab.com/group/project",
        "gitlab:gitlab.com/group/sub/project//dir@main",
        "gitlab:git.example.org/g/p",
        "gitlab:git.example.org/g/p@v1",
    ] {
        assert_canon(c, c);
    }
}

#[test]
fn gitlab_fields() {
    let r = one("gitlab:git.example.org/group/sub/project//dir/x@main");
    assert_eq!(r.kind, Kind::Gitlab);
    assert_eq!(r.host, "git.example.org");
    assert_eq!(r.repo, "group/sub/project");
    assert_eq!(r.dir, "dir/x");
    assert_eq!(r.git_ref.as_deref(), Some("main"));
}

#[test]
fn gitlab_shorthand_means_gitlab_com() {
    assert_canon("gitlab:group/project", "gitlab:gitlab.com/group/project");
    assert_canon(
        "gitlab:group/sub/project",
        "gitlab:gitlab.com/group/sub/project",
    );
    assert_eq!(one("gitlab:group/project").host, "gitlab.com");
}

#[test]
fn gitlab_https_forms() {
    assert_canon(
        "gitlab:https://git.example.org/g/p",
        "gitlab:git.example.org/g/p",
    );
    assert_canon(
        "gitlab:https://git.example.org/g/sub/p/",
        "gitlab:git.example.org/g/sub/p",
    );
    assert_canon("https://gitlab.com/g/p", "gitlab:gitlab.com/g/p");
    assert_canon(
        "https://gitlab.com/g/p/-/tree/main",
        "gitlab:gitlab.com/g/p@main",
    );
    assert_canon(
        "https://gitlab.com/g/p/-/tree/main/dir",
        "gitlab:gitlab.com/g/p//dir@main",
    );
    assert_canon(
        "https://gitlab.com/g/sub/p/-/tree/main/dir/x",
        "gitlab:gitlab.com/g/sub/p//dir/x@main",
    );
}

#[test]
fn gitlab_host_lowercased_path_keeps_case() {
    assert_canon(
        "gitlab:GIT.Example.ORG/Group/Project",
        "gitlab:git.example.org/Group/Project",
    );
    assert_canon(
        "https://GitLab.com/Group/Project",
        "gitlab:gitlab.com/Group/Project",
    );
    assert_canon("gitlab:Group/Project", "gitlab:gitlab.com/Group/Project");
}

// ---- forgejo -------------------------------------------------------------

#[test]
fn forgejo_canonical_forms_round_trip() {
    for c in [
        "forgejo:codeberg.org/owner/repo",
        "forgejo:git.example.org/owner/repo//dir@main",
        "forgejo:codeberg.org/Owner/Repo",
    ] {
        assert_canon(c, c);
    }
}

#[test]
fn forgejo_fields() {
    let r = one("forgejo:git.example.org/Owner/Repo//dir@v3");
    assert_eq!(r.kind, Kind::Forgejo);
    assert_eq!(r.host, "git.example.org");
    assert_eq!(r.repo, "Owner/Repo");
    assert_eq!(r.dir, "dir");
    assert_eq!(r.git_ref.as_deref(), Some("v3"));
}

#[test]
fn codeberg_shorthand() {
    assert_canon("codeberg:owner/repo", "forgejo:codeberg.org/owner/repo");
    assert_canon(
        "codeberg:Owner/Repo//dir@main",
        "forgejo:codeberg.org/Owner/Repo//dir@main",
    );
    assert_eq!(one("codeberg:owner/repo").host, "codeberg.org");
}

#[test]
fn forgejo_https_forms() {
    assert_canon(
        "forgejo:https://git.example.org/o/r",
        "forgejo:git.example.org/o/r",
    );
    assert_canon("https://codeberg.org/o/r", "forgejo:codeberg.org/o/r");
    assert_canon("https://gitea.com/o/r", "forgejo:gitea.com/o/r");
    assert_canon(
        "https://codeberg.org/o/r/src/branch/main",
        "forgejo:codeberg.org/o/r@main",
    );
    assert_canon(
        "https://codeberg.org/o/r/src/branch/main/dir",
        "forgejo:codeberg.org/o/r//dir@main",
    );
    assert_canon(
        "https://gitea.com/o/r/src/branch/dev/dir/x",
        "forgejo:gitea.com/o/r//dir/x@dev",
    );
}

#[test]
fn forgejo_host_lowercased_owner_repo_keep_case() {
    assert_canon(
        "https://CodeBerg.ORG/Owner/Repo",
        "forgejo:codeberg.org/Owner/Repo",
    );
}

// ---- web -----------------------------------------------------------------

#[test]
fn web_canonical_form_round_trips() {
    for c in [
        "web:https://example.com/parts/hew.json",
        "web:https://example.com/hew.json",
        "web:https://example.com/a/b/c/hew.json",
    ] {
        assert_canon(c, c);
    }
}

#[test]
fn web_url_without_hew_json_gets_it_appended() {
    assert_canon(
        "web:https://example.com/parts",
        "web:https://example.com/parts/hew.json",
    );
    assert_canon(
        "web:https://example.com",
        "web:https://example.com/hew.json",
    );
    assert_canon(
        "web:https://example.com/",
        "web:https://example.com/hew.json",
    );
}

#[test]
fn web_url_with_hew_json_is_not_doubled() {
    assert_canon(
        "web:https://example.com/parts/hew.json",
        "web:https://example.com/parts/hew.json",
    );
    let c = canon("web:https://example.com/parts/hew.json");
    assert!(!c.ends_with("hew.json/hew.json"));
}

#[test]
fn web_normalization() {
    assert_canon(
        "web:https://EXAMPLE.com/Parts/",
        "web:https://example.com/Parts/hew.json",
    );
    assert_canon(
        "web:https://example.com/parts/#frag",
        "web:https://example.com/parts/hew.json",
    );
    assert_canon(
        "web:https://example.com/parts/hew.json#x",
        "web:https://example.com/parts/hew.json",
    );
}

#[test]
fn web_fields() {
    let r = one("web:https://Example.com/parts/hew.json");
    assert_eq!(r.kind, Kind::Web);
    assert_eq!(r.host, "example.com");
    assert_eq!(r.git_ref, None);
    assert_eq!(r.dir, "");
    assert!(
        r.repo.trim_matches('/') == "parts",
        "repo is the URL path of the folder: {:?}",
        r.repo
    );
}

// ---- unknown https host: three candidates --------------------------------

#[test]
fn unknown_https_host_yields_web_then_forgejo_then_gitlab() {
    let v = parse_source_ref("https://example.com/a/b").unwrap();
    let kinds: Vec<Kind> = v.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, vec![Kind::Web, Kind::Forgejo, Kind::Gitlab]);
    assert_eq!(v[0].canonical, "web:https://example.com/a/b/hew.json");
    assert_eq!(v[1].canonical, "forgejo:example.com/a/b");
    assert_eq!(v[2].canonical, "gitlab:example.com/a/b");
}

#[test]
fn unknown_host_candidates_are_in_a_stable_order() {
    let a = parse_source_ref("https://Example.com/a/b").unwrap();
    let b = parse_source_ref("https://example.com/a/b/").unwrap();
    assert_eq!(a, b);
}

#[test]
fn known_hosts_yield_a_single_candidate() {
    for u in [
        "https://github.com/o/r",
        "https://gitlab.com/g/p",
        "https://codeberg.org/o/r",
        "https://gitea.com/o/r",
        "github:o/r",
        "gitlab:g/p",
        "codeberg:o/r",
        "forgejo:git.example.org/o/r",
        "gitlab:https://git.example.org/g/p",
        "forgejo:https://git.example.org/o/r",
        "web:https://example.com/p",
    ] {
        assert_eq!(parse_source_ref(u).unwrap().len(), 1, "{u}");
    }
}

// ---- equality --------------------------------------------------------------

#[test]
fn two_references_are_the_same_source_when_canonical_forms_are_equal() {
    let base = one("github:owner/repo");
    for f in [
        "https://github.com/Owner/Repo",
        "https://github.com/owner/repo/",
        "https://github.com/owner/repo#x",
        "github:OWNER/repo/",
    ] {
        assert_eq!(one(f), base, "{f}");
    }
    assert_eq!(one("github:o/r/dir"), one("github:o/r//dir"));
    assert_eq!(one("codeberg:o/r"), one("https://codeberg.org/o/r"));
    assert_eq!(one("gitlab:g/p"), one("https://gitlab.com/g/p"));
    assert_eq!(
        one("https://example.com/p/").canonical,
        one("web:https://example.com/p/hew.json").canonical
    );
}

#[test]
fn different_canonical_forms_are_different_sources() {
    assert_ne!(
        one("github:o/r"),
        one("github:o/r@main"),
        "a ref is part of the form"
    );
    assert_ne!(one("github:o/r"), one("github:o/r//dir"));
    assert_ne!(one("github:o/r@a"), one("github:o/r@b"));
    assert_ne!(
        one("gitlab:g/Proj"),
        one("gitlab:g/proj"),
        "only GitHub paths are case-folded"
    );
    assert_ne!(one("codeberg:O/r"), one("codeberg:o/r"));
    assert_ne!(one("github:o/r//Dir"), one("github:o/r//dir"));
    assert_ne!(one("github:o/r@Main"), one("github:o/r@main"));
}

#[test]
fn parse_is_idempotent_on_its_own_canonical_output() {
    for input in [
        "github:Owner/Repo/dir",
        "https://github.com/o/r/tree/main/d",
        "gitlab:g/p",
        "https://gitlab.com/g/p/-/tree/m/d",
        "codeberg:o/r",
        "https://gitea.com/o/r/src/branch/m/d",
        "forgejo:https://h.example/o/r",
        "gitlab:https://h.example/g/s/p",
        "https://example.com/p",
        "web:https://Example.com/P/",
    ] {
        for first in parse_source_ref(input).unwrap() {
            let again = parse_source_ref(&first.canonical).unwrap();
            assert_eq!(again.len(), 1, "{} has one reading", first.canonical);
            assert_eq!(again[0], first, "{input}");
        }
    }
}

// ---- refusals ------------------------------------------------------------

#[test]
fn a_query_string_is_refused() {
    for u in [
        "https://github.com/o/r?tab=readme",
        "https://github.com/o/r/tree/main/d?x=1",
        "https://gitlab.com/g/p?x",
        "https://codeberg.org/o/r?x=1",
        "https://example.com/p?x=1",
        "https://example.com/p/hew.json?token=1",
        "web:https://example.com/p?x=1",
        "gitlab:https://git.example.org/g/p?x=1",
        "https://example.com/p?",
    ] {
        assert_eq!(
            parse_source_ref(u).unwrap_err(),
            RefError::QueryRefused,
            "{u}"
        );
    }
}

#[test]
fn empty_and_junk_are_refused() {
    assert_eq!(parse_source_ref("").unwrap_err(), RefError::Empty);
    for s in [
        "github:",
        "github:owner",
        "github:owner/",
        "github:/repo",
        "gitlab:",
        "gitlab:onlyone",
        "codeberg:",
        "codeberg:owner",
        "forgejo:",
        "forgejo:host.example",
        "web:",
        "web:example.com",
        "nonsense",
        "owner/repo",
        "ftp://example.com/x",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/plain,x",
        "https://",
        "https:///x",
        "https://github.com",
        "https://github.com/owner",
        "github:o/r/tree/main",
    ] {
        assert!(parse_source_ref(s).is_err(), "accepted {s:?}");
    }
}

#[test]
fn only_https_is_accepted() {
    for s in [
        "http://github.com/o/r",
        "http://example.com/p",
        "web:http://example.com/p",
        "gitlab:http://git.example.org/g/p",
        "forgejo:http://git.example.org/o/r",
        "HTTP://example.com/p",
        "//example.com/p",
        "ws://example.com/p",
    ] {
        assert!(parse_source_ref(s).is_err(), "accepted {s:?}");
    }
}

#[test]
fn a_dir_that_could_escape_the_repository_is_refused() {
    for s in [
        "github:o/r//..",
        "github:o/r//../x",
        "github:o/r//a/../b",
        "github:o/r//a/..",
        "github:o/r//a\\b",
        "github:o/r//a//b",
        "github:o/r//.hidden",
        "github:o/r//a/.b",
        "github:o/r//./a",
        "github:o/r//a?b",
        "github:o/r//a#b",
        "github:o/r//%2e%2e/x",
        "github:o/r//a:b",
        "https://github.com/o/r/tree/main/../x",
        "https://github.com/o/r/tree/main/a/../b",
        "gitlab:g/p//..",
        "codeberg:o/r//../x",
    ] {
        assert!(parse_source_ref(s).is_err(), "accepted {s:?}");
    }
}

#[test]
fn a_repo_path_with_traversal_is_refused() {
    for s in [
        "github:o/../r",
        "github:../r",
        "github:o/..",
        "gitlab:g/../p",
        "codeberg:o/..",
        "forgejo:git.example.org/../r",
        "https://example.com/../x",
        "https://example.com/a/../b",
        "web:https://example.com/a/../b",
        "https://github.com/o/../r",
    ] {
        assert!(parse_source_ref(s).is_err(), "accepted {s:?}");
    }
}

#[test]
fn a_ref_that_is_empty_or_unsafe_is_refused() {
    for s in [
        "github:o/r@",
        "github:o/r//d@",
        "github:o/r@a b",
        "github:o/r@a?b",
        "github:o/r@a#b",
        "github:o/r@..",
        "github:o/r@a/../b",
    ] {
        assert!(parse_source_ref(s).is_err(), "accepted {s:?}");
    }
}

#[test]
fn control_characters_and_nul_are_refused() {
    for s in [
        "github:o/r\0",
        "github:o\n/r",
        "https://example.com/p\0",
        "github:o/r//d\u{7f}",
        "github:o/r//d\0e",
    ] {
        assert!(parse_source_ref(s).is_err(), "accepted {s:?}");
    }
}

#[test]
fn a_very_long_reference_does_not_panic() {
    let long = format!("github:o/{}", "r".repeat(100_000));
    let _ = parse_source_ref(&long);
    let long = format!("https://example.com/{}", "a/".repeat(50_000));
    let _ = parse_source_ref(&long);
}
