//! Every `forgejo` source resolves through one shape on every host:
//! `GET https://{host}/api/v1/repos/{o}/{r}/git/commits/{ref}?stat=false&verification=false&files=false`
//! (`HEAD` with no ref), and the commit is the top-level `sha`.
use sources::{Kind, SourceRef, commit_from_response, parse_source_ref, resolve_url};

const SHA: &str = "7d55578694c587995e8ac43b7e7e236ecda4e00a";
const TAIL: &str = "?stat=false&verification=false&files=false";

fn one(text: &str) -> SourceRef {
    parse_source_ref(text).unwrap().remove(0)
}

fn url(s: &SourceRef) -> String {
    resolve_url(s).unwrap().to_string()
}

#[test]
fn the_resolve_shape_is_identical_on_codeberg_and_gitea() {
    for r in ["v1.0", "main", SHA] {
        for host in ["codeberg.org", "gitea.com"] {
            assert_eq!(
                url(&one(&format!("forgejo:{host}/o/r@{r}"))),
                format!("https://{host}/api/v1/repos/o/r/git/commits/{r}{TAIL}"),
                "{host} @{r}"
            );
        }
    }
}

#[test]
fn no_ref_resolves_head_on_every_host() {
    for host in ["codeberg.org", "gitea.com", "git.example.org"] {
        assert_eq!(
            url(&one(&format!("forgejo:{host}/o/r"))),
            format!("https://{host}/api/v1/repos/o/r/git/commits/HEAD{TAIL}"),
            "{host}"
        );
    }
}

#[test]
fn a_self_hosted_host_uses_the_same_shape() {
    assert_eq!(
        url(&one("forgejo:git.example.org/o/r@dev")),
        format!("https://git.example.org/api/v1/repos/o/r/git/commits/dev{TAIL}")
    );
}

#[test]
fn a_slash_ref_is_one_path_segment() {
    // The parser refuses a ref holding `/`, so build the source by hand.
    let mut s = one("forgejo:codeberg.org/o/r@x");
    s.git_ref = Some("release/1.0".to_owned());
    assert_eq!(
        url(&s),
        format!("https://codeberg.org/api/v1/repos/o/r/git/commits/release%2F1.0{TAIL}")
    );
}

#[test]
fn commit_from_response_reads_only_the_top_level_sha() {
    let ok = format!("{{\"sha\":\"{SHA}\",\"url\":\"u\"}}");
    assert_eq!(
        commit_from_response(Kind::Forgejo, ok.as_bytes()).as_deref(),
        Some(SHA)
    );
    let old = format!("{{\"commit\":{{\"id\":\"{SHA}\"}}}}");
    assert_eq!(commit_from_response(Kind::Forgejo, old.as_bytes()), None);
}
