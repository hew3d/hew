//! Request URLs and resolve responses (6.2, 6.4), in the shapes S0 recorded
//! (docs/design/shared-library-sources-s0-results.md).
//!
//! Every `forgejo` host (Forgejo, Gitea, Codeberg) resolves through the one
//! `git/commits/{ref}` request (field `sha`), with `HEAD` for no ref; it
//! answers a tag, a branch, a sha and `HEAD` alike. Files read the same way
//! on all of them.
//!
//! Every piece interpolated into a URL is percent-encoded (everything but
//! `A-Z a-z 0-9 - . _ ~`), so a name can never add a segment, a query or a
//! fragment. A `SourceRef` is expected to come from `parse_source_ref`; its
//! repo and dir are not re-checked here, but a host `parse_source_ref` could
//! never produce sends every request to a placeholder that never resolves,
//! so no builder here can fail or panic.

use serde_json::Value;
use url::Url;

use crate::paths::{check_rel_path, is_commit_hex};
use crate::{Kind, PathError, SourceRef};

/// The revision read when neither a commit nor a ref is known, and the ref a
/// source with none resolves: `HEAD` is the one spelling the raw hosts and
/// every resolve API (GitHub, GitLab, Forgejo, Gitea) accept for it.
const DEFAULT_REV: &str = "HEAD";
/// Largest resolve response considered (6.4: 64 KB).
const MAX_RESOLVE_BYTES: usize = 64 * 1024;

/// Percent-encodes `s`, leaving only unreserved characters.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Encodes each `/`-separated segment, keeping the slashes.
fn enc_path(p: &str) -> String {
    p.split('/').map(enc).collect::<Vec<_>>().join("/")
}

/// The revision to read: the commit, else the source's ref, else `HEAD`. A
/// value that is empty or only dots could act as a dot segment, so it is not
/// used.
fn rev<'a>(source: &'a SourceRef, commit: Option<&'a str>) -> &'a str {
    commit
        .or(source.git_ref.as_deref())
        .filter(|r| r.bytes().any(|b| b != b'.'))
        .unwrap_or(DEFAULT_REV)
}

/// The host of every request for a hand-built `SourceRef` whose host fails
/// [`host_ok`]: `.invalid` is reserved and never resolves (RFC 2606).
const PLACEHOLDER_HOST: &str = "invalid.";
/// The whole URL used when a built URL does not parse.
const PLACEHOLDER_URL: &str = "https://invalid./";

/// True for a host of the shape `parse_source_ref` produces: dot-separated,
/// non-empty labels of lowercase ASCII letters, digits and `-`, then an
/// optional `:<port>` of 1 to 65535. Nothing else can reach a request, so a
/// host can never add a path, a query, userinfo or a second host.
fn host_ok(host: &str) -> bool {
    let (name, port) = match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    let name_ok = name.split('.').all(|label| {
        !label.is_empty()
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let port_ok = port.is_none_or(|p| {
        !p.is_empty()
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u16>().is_ok_and(|n| n != 0)
    });
    name_ok && port_ok
}

/// The source's host, or the placeholder when it fails [`host_ok`].
fn host_of(source: &SourceRef) -> &str {
    if host_ok(&source.host) {
        &source.host
    } else {
        PLACEHOLDER_HOST
    }
}

/// The placeholder URL. `Url` has no infallible constructor, and parsing this
/// constant cannot fail (it parses on the first pass; a unit test pins it).
/// The retry stands where an `expect` would, so that no panicking call is
/// compiled into code that hostile input reaches.
fn placeholder_url() -> Url {
    loop {
        if let Ok(u) = Url::parse(PLACEHOLDER_URL) {
            return u;
        }
    }
}

/// Parses a URL built above. The host passed [`host_ok`] and every other
/// piece is validated or encoded, so this does not fail for a reference from
/// `parse_source_ref`; one that does not parse (a hand-built host such as
/// `1.2.3.999`) gets the placeholder.
fn to_url(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|_| placeholder_url())
}

/// The ref-to-commit request (6.2). `dir` plays no part. `None` for `web`,
/// which has no commit. A git source with no `@<ref>` resolves `HEAD`; a
/// Forgejo source takes the same `git/commits/{ref}` shape on every host.
pub fn resolve_url(source: &SourceRef) -> Option<Url> {
    let r = enc(rev(source, None));
    let host = host_of(source);
    let s = match source.kind {
        Kind::Web => return None,
        Kind::Github => format!(
            "https://api.github.com/repos/{}/commits/{r}",
            enc_path(&source.repo)
        ),
        Kind::Gitlab => format!(
            "https://{host}/api/v4/projects/{}/repository/commits/{r}",
            enc(&source.repo)
        ),
        Kind::Forgejo => format!(
            "https://{host}/api/v1/repos/{}/git/commits/{r}?stat=false&verification=false&files=false",
            enc_path(&source.repo)
        ),
    };
    Some(to_url(&s))
}

/// The URL of `rel` (already validated) inside the source's folder.
fn read_url(source: &SourceRef, rel: &str, commit: Option<&str>) -> Url {
    let full = if source.dir.is_empty() {
        rel.to_owned()
    } else {
        format!("{}/{rel}", source.dir)
    };
    let r = enc(rev(source, commit));
    let host = host_of(source);
    let s = match source.kind {
        Kind::Github => format!(
            "https://raw.githubusercontent.com/{}/{r}/{}",
            enc_path(&source.repo),
            enc_path(&full)
        ),
        Kind::Gitlab => format!(
            "https://{host}/api/v4/projects/{}/repository/files/{}/raw?ref={r}",
            enc(&source.repo),
            enc(&full)
        ),
        Kind::Forgejo => format!(
            "https://{host}/api/v1/repos/{}/raw/{}?ref={r}",
            enc_path(&source.repo),
            enc_path(&full)
        ),
        // A web source has no commit; `repo` is the folder holding the index.
        Kind::Web if source.repo.is_empty() => {
            format!("https://{host}/{}", enc_path(&full))
        }
        Kind::Web => format!(
            "https://{host}/{}/{}",
            enc_path(&source.repo),
            enc_path(&full)
        ),
    };
    to_url(&s)
}

/// Where `hew.json` is read. A commit pins the read; without one the source's
/// ref (or `HEAD`) is used, as after a rate-limited resolve (6.2). A `web`
/// source ignores the commit.
pub fn index_url(source: &SourceRef, commit: Option<&str>) -> Url {
    read_url(source, "hew.json", commit)
}

/// Where the file at `rel_path` (relative to the index) is read. Refuses
/// anything that is not a safe relative path under the 6.1 `path` rule,
/// including percent-encoded traversal, so the URL stays inside the source's
/// folder and host.
pub fn file_url(
    source: &SourceRef,
    rel_path: &str,
    commit: Option<&str>,
) -> Result<Url, PathError> {
    check_rel_path(rel_path).map_err(|reason| PathError::Invalid { reason })?;
    Ok(read_url(source, rel_path, commit))
}

/// The commit named by a resolve response; `None` when the body has none.
///
/// GitHub answers `sha` (or, with the `vnd.github.sha` Accept header, the
/// bare SHA); GitLab `id`; Forgejo and Gitea `sha`. A commit is
/// returned only when it is 40 or 64 lowercase hex characters, because it is
/// interpolated into later request URLs. A `web` source has none.
pub fn commit_from_response(kind: Kind, body: &[u8]) -> Option<String> {
    if kind == Kind::Web || body.len() > MAX_RESOLVE_BYTES {
        return None;
    }
    let found = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(obj)) => match kind {
            Kind::Github => obj.get("sha"),
            Kind::Gitlab => obj.get("id"),
            _ => obj.get("sha"),
        }
        .and_then(Value::as_str)
        .map(str::to_owned),
        // A bare SHA body (GitHub's `vnd.github.sha` answer) is not JSON, or
        // is all digits and parses as a number.
        Ok(_) | Err(_) if kind == Kind::Github => {
            std::str::from_utf8(body).ok().map(|s| s.trim().to_owned())
        }
        _ => None,
    }?;
    is_commit_hex(&found).then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_parses_on_the_first_pass() {
        assert!(Url::parse(PLACEHOLDER_URL).is_ok());
        assert_eq!(placeholder_url().host_str(), Some(PLACEHOLDER_HOST));
    }

    #[test]
    fn a_hand_built_hostile_host_never_reaches_a_request() {
        for host in [
            "evil.example/x?y#",
            "a@b.example",
            "EXAMPLE.com",
            "",
            "a..b",
            "example.com:0",
            "example.com:99999",
            "example.com:",
            "[::1]",
            "1.2.3.999",
        ] {
            let s = SourceRef {
                kind: Kind::Web,
                host: host.to_owned(),
                repo: "p".to_owned(),
                dir: String::new(),
                git_ref: None,
                canonical: String::new(),
            };
            let u = index_url(&s, None);
            assert_eq!(u.host_str(), Some(PLACEHOLDER_HOST), "{host:?}: {u}");
        }
    }
}
