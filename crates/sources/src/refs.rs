//! Source references: parsing pasted text into canonical form (6.2).
//!
//! Accepted input is conservative on purpose. Every refusal is a typed
//! [`RefError`]; nothing is repaired silently (DEVELOPMENT.md rule 4), except
//! the normalizations 6.2 names (host lowercased, GitHub owner and repo
//! lowercased, trailing slash, fragment and `.git` suffix dropped), a
//! trailing dot on a host and the default port 443, both dropped.
//!
//! Every canonical form re-parses to the same reference. A `gitlab` or
//! `forgejo` host with no dot, or with a port, cannot be told apart from a
//! group in the shorthand, so its canonical form is the explicit one,
//! `gitlab:https://<host>/<path>[//<dir>][@<ref>]` (likewise `forgejo:`);
//! a dotted host without a port keeps the shorthand `gitlab:<host>/<path>`.

use url::Url;

use crate::{Kind, RefError, SourceRef};

/// Longest reference accepted, in bytes.
const MAX_REF_BYTES: usize = 2_048;
const HEW_JSON: &str = "hew.json";
/// The port an `https` host has when none is written; dropped when written.
const DEFAULT_PORT: u16 = 443;

fn invalid<T>(reason: impl Into<String>) -> Result<T, RefError> {
    Err(RefError::Invalid {
        reason: reason.into(),
    })
}

/// ASCII letters, digits and `. _ - + ~`; never `.` or `..`.
fn seg_chars_ok(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+' | b'~'))
}

/// A repository path segment (owner, repo, group, web folder): see
/// [`seg_chars_ok`], and still not `.` or `..` once any `.git` suffix is
/// dropped (so `..git` and `...git` are refused).
fn repo_seg_ok(s: &str) -> bool {
    seg_chars_ok(s) && seg_chars_ok(strip_dot_git(s))
}

/// A `dir` segment: letters and digits (any script), spaces inside (never at
/// either edge), and `. _ - + ~`; no dot-leading segment (so no `.` or `..`).
/// Excludes `\ : ? # % @ /` and controls.
fn dir_seg_ok(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.starts_with(' ')
        && !s.ends_with(' ')
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | '+' | '~'))
}

/// A ref: ASCII letters, digits and `. _ - + ~`, no leading dot (so never `.` or `..`). No
/// `/`, so a ref is always one path segment in the URL forms.
fn ref_ok(s: &str) -> bool {
    seg_chars_ok(s) && !s.starts_with('.')
}

fn check_ref(r: &str) -> Result<String, RefError> {
    if ref_ok(r) {
        Ok(r.to_owned())
    } else {
        invalid(format!("{r:?} is not a usable ref"))
    }
}

/// Joins validated dir segments; empty input is the repository root.
fn check_dir<'a>(segs: impl IntoIterator<Item = &'a str>) -> Result<String, RefError> {
    let segs: Vec<&str> = segs.into_iter().collect();
    match segs.iter().find(|s| !dir_seg_ok(s)) {
        Some(bad) => invalid(format!("{bad:?} is not a usable folder name")),
        None => Ok(segs.join("/")),
    }
}

/// Drops any `.git` suffixes from a repository name (a bare `.git` stays).
fn strip_dot_git(repo: &str) -> &str {
    let mut r = repo;
    while r.len() > 4 && r.ends_with(".git") {
        r = &r[..r.len() - 4];
    }
    r
}

/// A port: 1 to 65535 in decimal digits, with no sign and no leading zero.
fn parse_port(p: &str) -> Result<u16, RefError> {
    let digits = !p.is_empty() && !p.starts_with('0') && p.bytes().all(|b| b.is_ascii_digit());
    match p.parse::<u16>() {
        Ok(n) if digits && n != 0 => Ok(n),
        _ => invalid(format!("{p:?} is not a usable port")),
    }
}

/// A host after the `url` crate's normalization (lowercase, punycode, any
/// full-width dot read as a dot), with one trailing dot dropped and a port
/// other than 443 kept as `:<port>`. Refuses userinfo, brackets (so IPv6
/// literals), an empty label, and anything that is not a plain name or IPv4
/// address.
fn normalize_host(authority: &str) -> Result<String, RefError> {
    if authority.is_empty() {
        return invalid("the address has no host");
    }
    if authority.contains(['@', '[', ']', '%', '\\', ' ', '?', '#']) {
        return invalid("credentials and odd characters are not accepted in a host");
    }
    let (name, port) = match authority.split_once(':') {
        Some((name, port)) => (name, Some(parse_port(port)?)),
        None => (authority, None),
    };
    let parsed = Url::parse(&format!("https://{name}/")).ok();
    let host = parsed.as_ref().and_then(Url::host_str).unwrap_or("");
    let host = host.strip_suffix('.').unwrap_or(host);
    let usable = !host.is_empty()
        && host.split('.').all(|label| !label.is_empty())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    if !usable {
        return invalid(format!("{authority:?} is not a usable host"));
    }
    Ok(match port {
        Some(p) if p != DEFAULT_PORT => format!("{host}:{p}"),
        _ => host.to_owned(),
    })
}

/// True when a shorthand's first segment names a host rather than a group:
/// it has a dot or a port (Q7).
fn names_a_host(first: &str) -> bool {
    first.contains(['.', ':'])
}

/// True when a git host must be written in the explicit
/// `<kind>:https://<host>/…` form: without a dot or with a port it would not
/// read back as a host in the shorthand.
fn needs_explicit_form(host: &str) -> bool {
    host.contains(':') || !host.contains('.')
}

/// Builds the canonical form of a git reference.
fn git_ref_of(kind: Kind, host: &str, repo: &str, dir: &str, git_ref: Option<&str>) -> SourceRef {
    let word = if kind == Kind::Gitlab {
        "gitlab"
    } else {
        "forgejo"
    };
    let mut canonical = match kind {
        Kind::Github => format!("github:{repo}"),
        _ if needs_explicit_form(host) => format!("{word}:https://{host}/{repo}"),
        _ => format!("{word}:{host}/{repo}"),
    };
    if !dir.is_empty() {
        canonical.push_str("//");
        canonical.push_str(dir);
    }
    if let Some(r) = git_ref {
        canonical.push('@');
        canonical.push_str(r);
    }
    SourceRef {
        kind,
        host: host.to_owned(),
        repo: repo.to_owned(),
        dir: dir.to_owned(),
        git_ref: git_ref.map(str::to_owned),
        canonical,
    }
}

fn web_ref_of(host: &str, folder: &[&str]) -> SourceRef {
    let repo = folder.join("/");
    let canonical = if repo.is_empty() {
        format!("web:https://{host}/{HEW_JSON}")
    } else {
        format!("web:https://{host}/{repo}/{HEW_JSON}")
    };
    SourceRef {
        kind: Kind::Web,
        host: host.to_owned(),
        repo,
        dir: String::new(),
        git_ref: None,
        canonical,
    }
}

// ---- repository forms, from path segments ----------------------------------

/// Validates a GitHub owner and repo and builds the reference (both folded to
/// lowercase).
fn github(
    owner: &str,
    repo: &str,
    dir: &str,
    git_ref: Option<&str>,
) -> Result<SourceRef, RefError> {
    let repo = strip_dot_git(repo);
    if !repo_seg_ok(owner) || !repo_seg_ok(repo) {
        return invalid("a GitHub reference needs an owner and a repository");
    }
    let path = format!("{owner}/{repo}").to_ascii_lowercase();
    Ok(git_ref_of(Kind::Github, "github.com", &path, dir, git_ref))
}

/// A GitLab project path: two or more valid segments. A lone `-` is refused:
/// it is GitLab's own separator (`/-/tree/…`), never a group or project, and
/// accepting it would let a canonical form read back differently.
fn gitlab(
    host: &str,
    repo: &[&str],
    dir: &str,
    git_ref: Option<&str>,
) -> Result<SourceRef, RefError> {
    match repo.split_last() {
        Some((last, groups))
            if !groups.is_empty() && repo.iter().all(|s| repo_seg_ok(s) && *s != "-") =>
        {
            let mut segs: Vec<&str> = groups.to_vec();
            segs.push(strip_dot_git(last));
            Ok(git_ref_of(
                Kind::Gitlab,
                host,
                &segs.join("/"),
                dir,
                git_ref,
            ))
        }
        _ => invalid("a GitLab reference needs a group and a project"),
    }
}

fn forgejo(
    host: &str,
    owner: &str,
    repo: &str,
    dir: &str,
    git_ref: Option<&str>,
) -> Result<SourceRef, RefError> {
    let repo = strip_dot_git(repo);
    if !repo_seg_ok(owner) || !repo_seg_ok(repo) {
        return invalid("a Forgejo reference needs an owner and a repository");
    }
    Ok(git_ref_of(
        Kind::Forgejo,
        host,
        &format!("{owner}/{repo}"),
        dir,
        git_ref,
    ))
}

/// `https://github.com/<owner>/<repo>[/tree/<ref>/<dir>]`.
fn github_url(segs: &[&str]) -> Result<SourceRef, RefError> {
    match segs {
        [owner, repo] => github(owner, repo, "", None),
        [owner, repo, "tree", r, dir @ ..] => github(
            owner,
            repo,
            &check_dir(dir.iter().copied())?,
            Some(&check_ref(r)?),
        ),
        _ => invalid("not a GitHub repository or folder address"),
    }
}

/// `https://<host>/<group>/…/<project>[/-/tree/<ref>/<dir>]`.
fn gitlab_url(host: &str, segs: &[&str]) -> Result<SourceRef, RefError> {
    match segs.iter().position(|s| *s == "-") {
        None => gitlab(host, segs, "", None),
        Some(at) => {
            let (repo, rest) = segs.split_at(at);
            match rest {
                ["-", "tree", r, dir @ ..] => gitlab(
                    host,
                    repo,
                    &check_dir(dir.iter().copied())?,
                    Some(&check_ref(r)?),
                ),
                _ => invalid("not a GitLab repository or folder address"),
            }
        }
    }
}

/// `https://<host>/<owner>/<repo>[/src/branch/<ref>/<dir>]`.
fn forgejo_url(host: &str, segs: &[&str]) -> Result<SourceRef, RefError> {
    match segs {
        [owner, repo] => forgejo(host, owner, repo, "", None),
        [owner, repo, "src", "branch", r, dir @ ..] => forgejo(
            host,
            owner,
            repo,
            &check_dir(dir.iter().copied())?,
            Some(&check_ref(r)?),
        ),
        _ => invalid("not a Forgejo repository or folder address"),
    }
}

/// A `web` source: the folder holding `hew.json`; a trailing `hew.json` is
/// the index itself and is not doubled.
fn web_url(host: &str, segs: &[&str]) -> Result<SourceRef, RefError> {
    let folder = match segs.split_last() {
        Some((&HEW_JSON, rest)) => rest,
        _ => segs,
    };
    match folder.iter().find(|s| !repo_seg_ok(s)) {
        Some(bad) => invalid(format!("{bad:?} is not usable in a web address")),
        None => Ok(web_ref_of(host, folder)),
    }
}

// ---- https addresses ---------------------------------------------------------

/// The part after `https://`, if `s` starts with it (scheme in any case).
fn strip_https(s: &str) -> Option<&str> {
    let (head, tail) = (s.get(..8)?, s.get(8..)?);
    head.eq_ignore_ascii_case("https://").then_some(tail)
}

/// The normalized host and the path of the text after `https://`. The
/// fragment is dropped; a query is refused; userinfo is refused.
fn split_authority(body: &str) -> Result<(String, &str), RefError> {
    let body = body.split('#').next().unwrap_or("");
    if body.contains('?') {
        return Err(RefError::QueryRefused);
    }
    let (authority, path) = body.split_once('/').unwrap_or((body, ""));
    Ok((normalize_host(authority)?, path))
}

/// The segments of a path: a trailing slash is dropped; an empty, `.` or
/// `..` segment anywhere is refused.
fn path_segs(path: &str) -> Result<Vec<&str>, RefError> {
    let path = path.trim_end_matches('/');
    let segs: Vec<&str> = if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    };
    if segs.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
        return invalid("the address has an empty or dot segment");
    }
    Ok(segs)
}

/// Host and path segments of the text after `https://`.
fn split_https(body: &str) -> Result<(String, Vec<&str>), RefError> {
    let (host, path) = split_authority(body)?;
    Ok((host, path_segs(path)?))
}

/// The text after `gitlab:https://` or `forgejo:https://`: the host, the
/// repository segments, and the `//<dir>` and `@<ref>` of the explicit
/// canonical form, split off the path only (never the host).
type Explicit<'a> = (String, Vec<&'a str>, Option<&'a str>, Option<String>);

fn split_explicit(body: &str) -> Result<Explicit<'_>, RefError> {
    let (host, path) = split_authority(body)?;
    let (head, dir, git_ref) = split_shorthand(path)?;
    Ok((host, path_segs(head)?, dir, git_ref))
}

/// A pasted `https://…` address: one candidate for a known host, three
/// (`web`, `forgejo`, `gitlab`) for an unknown one (6.2).
fn parse_https(body: &str) -> Result<Vec<SourceRef>, RefError> {
    let (host, segs) = split_https(body)?;
    match host.as_str() {
        "github.com" => Ok(vec![github_url(&segs)?]),
        "gitlab.com" => Ok(vec![gitlab_url(&host, &segs)?]),
        "codeberg.org" | "gitea.com" => Ok(vec![forgejo_url(&host, &segs)?]),
        _ => {
            let web = web_url(&host, &segs)?;
            let mut out = vec![web];
            // A URL already naming `hew.json` is unambiguous (Q9); a path
            // with fewer than two segments cannot name a repository (Q8).
            if segs.last() != Some(&HEW_JSON) {
                out.extend(forgejo_url(&host, &segs).ok());
                out.extend(gitlab_url(&host, &segs).ok());
            }
            Ok(out)
        }
    }
}

// ---- `scheme:` shorthand forms -----------------------------------------------

/// Splits `rest` of a shorthand into `(repository part, dir, ref)`: the ref
/// after the last `@`, the dir after the first `//`. Trailing slashes go.
fn split_shorthand(rest: &str) -> Result<(&str, Option<&str>, Option<String>), RefError> {
    let (body, git_ref) = match rest.rsplit_once('@') {
        Some((b, r)) => (b, Some(check_ref(r)?)),
        None => (rest, None),
    };
    let (head, dir) = match body.split_once("//") {
        Some((h, d)) => (h, Some(d.trim_end_matches('/'))),
        None => (body, None),
    };
    Ok((head.trim_end_matches('/'), dir, git_ref))
}

/// `github:<owner>/<repo>[//<dir>][@<ref>]`, and `github:<owner>/<repo>/<dir>`.
fn github_shorthand(rest: &str) -> Result<SourceRef, RefError> {
    let (head, dir, git_ref) = split_shorthand(rest)?;
    let segs: Vec<&str> = head.split('/').collect();
    match segs.as_slice() {
        [owner, repo] => github(owner, repo, &dir_of(dir)?, git_ref.as_deref()),
        // `owner/repo/dir`: the short form. A leading `tree` or `blob`
        // would read as a web address, so it is refused.
        [owner, repo, first, rest @ ..] if dir.is_none() && !matches!(*first, "tree" | "blob") => {
            let dir = check_dir(std::iter::once(*first).chain(rest.iter().copied()))?;
            github(owner, repo, &dir, git_ref.as_deref())
        }
        _ => invalid("not a GitHub reference"),
    }
}

/// The dir of a `//dir` part (`None` or empty is the root).
fn dir_of(dir: Option<&str>) -> Result<String, RefError> {
    match dir {
        None | Some("") => Ok(String::new()),
        Some(d) => check_dir(d.split('/')),
    }
}

/// `gitlab:[<host>/]<group>/…/<project>[//<dir>][@<ref>]` or
/// `gitlab:https://<host>/…`, the latter either a GitLab address or the
/// explicit canonical form with `//<dir>` and `@<ref>`. A first segment with
/// a dot or a port is a host (Q7); otherwise it is a group on `gitlab.com`.
fn gitlab_shorthand(rest: &str) -> Result<SourceRef, RefError> {
    if let Some(body) = strip_https(rest) {
        let (host, segs, dir, git_ref) = split_explicit(body)?;
        return if dir.is_none() && git_ref.is_none() {
            gitlab_url(&host, &segs)
        } else {
            gitlab(&host, &segs, &dir_of(dir)?, git_ref.as_deref())
        };
    }
    let (head, dir, git_ref) = split_shorthand(rest)?;
    let segs: Vec<&str> = head.split('/').collect();
    let (host, repo) = match segs.split_first() {
        Some((first, tail)) if names_a_host(first) => (normalize_host(first)?, tail),
        _ => ("gitlab.com".to_owned(), segs.as_slice()),
    };
    gitlab(&host, repo, &dir_of(dir)?, git_ref.as_deref())
}

/// `forgejo:<host>/<owner>/<repo>[//<dir>][@<ref>]` or
/// `forgejo:https://<host>/…` (a Forgejo address or the explicit canonical
/// form), and `codeberg:<owner>/<repo>…` (host `codeberg.org`).
fn forgejo_shorthand(rest: &str, fixed_host: Option<&str>) -> Result<SourceRef, RefError> {
    if fixed_host.is_none()
        && let Some(body) = strip_https(rest)
    {
        let (host, segs, dir, git_ref) = split_explicit(body)?;
        return match (dir, git_ref, segs.as_slice()) {
            (None, None, _) => forgejo_url(&host, &segs),
            (dir, git_ref, [owner, name]) => {
                forgejo(&host, owner, name, &dir_of(dir)?, git_ref.as_deref())
            }
            _ => invalid("a Forgejo reference needs an owner and a repository"),
        };
    }
    let (head, dir, git_ref) = split_shorthand(rest)?;
    let segs: Vec<&str> = head.split('/').collect();
    let (host, repo) = match (fixed_host, segs.split_first()) {
        (Some(h), _) => (h.to_owned(), segs.as_slice()),
        (None, Some((first, tail))) if names_a_host(first) => (normalize_host(first)?, tail),
        _ => return invalid("a Forgejo reference starts with its host"),
    };
    match repo {
        [owner, name] => forgejo(&host, owner, name, &dir_of(dir)?, git_ref.as_deref()),
        _ => invalid("a Forgejo reference needs an owner and a repository"),
    }
}

/// `web:https://<host>/<path>[/hew.json]`.
fn web_shorthand(rest: &str) -> Result<SourceRef, RefError> {
    match strip_https(rest) {
        Some(body) => {
            let (host, segs) = split_https(body)?;
            web_url(&host, &segs)
        }
        None => invalid("a web source is an https address"),
    }
}

/// Parses pasted text into source candidates, in probe order; exactly one
/// unless an `https` URL names a host Hew does not know (6.2).
///
/// Surrounding whitespace is trimmed (Q10). Refused with a typed error: empty
/// text, a query string, a scheme other than `https` (and the forms below),
/// credentials, an IPv6 literal or a malformed port (outside 1 to 65535, or
/// with a leading zero) in a URL, control characters, anything over 2,048
/// bytes, and a repository, dir or ref that is empty or could climb out of its
/// folder (`..`, a dot-leading segment, `\`, `:`, `%`, an empty segment, a
/// name that is `..` once `.git` is dropped, a space at a dir segment's edge).
/// Scheme words are case-insensitive (Q12). Refs hold no `/`, so a branch
/// named `a/b` cannot be written.
pub fn parse_source_ref(text: &str) -> Result<Vec<SourceRef>, RefError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(RefError::Empty);
    }
    if text.len() > MAX_REF_BYTES {
        return invalid("the reference is too long");
    }
    if text.chars().any(char::is_control) {
        return invalid("the reference contains a control character");
    }
    let Some((scheme, rest)) = text.split_once(':') else {
        return invalid(
            "a reference starts with github:, gitlab:, codeberg:, forgejo:, web: or https://",
        );
    };
    let one = |r: Result<SourceRef, RefError>| r.map(|r| vec![r]);
    match scheme.to_ascii_lowercase().as_str() {
        "https" => match rest.strip_prefix("//") {
            Some(body) => parse_https(body),
            None => invalid("not an https address"),
        },
        "github" => one(github_shorthand(rest)),
        "gitlab" => one(gitlab_shorthand(rest)),
        "forgejo" => one(forgejo_shorthand(rest, None)),
        "codeberg" => one(forgejo_shorthand(rest, Some("codeberg.org"))),
        "web" => one(web_shorthand(rest)),
        other => invalid(format!(
            "{other:?} is not a source scheme; only https is accepted"
        )),
    }
}

/// True when `a` and `b` are the same source with any `@<ref>` set aside:
/// their canonical forms are equal once the ref is removed (6.2, the removed
/// list's match rule).
pub fn same_source_ignoring_ref(a: &SourceRef, b: &SourceRef) -> bool {
    fn without_ref(s: &SourceRef) -> &str {
        s.git_ref
            .as_deref()
            .and_then(|r| {
                s.canonical
                    .strip_suffix(r)
                    .and_then(|c| c.strip_suffix('@'))
            })
            .unwrap_or(&s.canonical)
    }
    without_ref(a) == without_ref(b)
}
