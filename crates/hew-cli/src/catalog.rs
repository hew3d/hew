//! `hew-cli catalog check` and `catalog build`
//! (docs/design/shared-library-sources.md 6.3, 6.4, 7.6). The specs live in
//! `tests/catalog_check.rs` and `tests/catalog_build.rs`.
//!
//! Every request carries `Origin: https://app.hew3d.com`, goes to an `https`
//! address that resolves only to public IPs, follows at most 5 redirects by
//! hand (each hop re-checked), sends no credentials, and is read against a
//! size cap. Only the build with the `test-loopback` feature has any
//! allowance for a loopback address.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::Url;
use sources::{CatalogSource, Category, SourceIndex, SourceRef};

/// The `Origin` every request carries, as the web app makes them (7.6: the
/// cross-origin header must be present on each).
pub const WEB_ORIGIN: &str = "https://app.hew3d.com";

/// Fetch policy.
///
/// The loopback allowance exists only when the crate is built with the
/// `test-loopback` Cargo feature, which only the package's own tests enable
/// (a dev-dependency on itself in Cargo.toml). Integration tests do not see
/// the library's `cfg(test)`, hence a feature. With it, a `https` reference
/// or URL whose host is loopback is requested over plain `http` and passes
/// the public-host check; `FetchPolicy::new` leaves it off, and the shipped
/// binary never has the feature.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    #[cfg(feature = "test-loopback")]
    pub allow_http_loopback: bool,
    /// Minimum spacing between two requests to the same host (host only,
    /// lowercase, no port), measured start to start.
    pub request_gap: Duration,
    /// Per-request time limit.
    pub timeout: Duration,
}

impl FetchPolicy {
    /// The production policy: no loopback allowance.
    pub fn new(request_gap: Duration, timeout: Duration) -> FetchPolicy {
        FetchPolicy {
            #[cfg(feature = "test-loopback")]
            allow_http_loopback: false,
            request_gap,
            timeout,
        }
    }

    /// The same policy with the loopback allowance on.
    #[cfg(feature = "test-loopback")]
    pub fn with_loopback(mut self) -> FetchPolicy {
        self.allow_http_loopback = true;
        self
    }
}

/// A fetched response. `final_url` is after redirects.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub final_url: String,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Why a request failed (mirrors the desktop `remote_fetch` codes, 6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    NotHttps,
    NotPublicHost,
    TooLarge,
    Timeout,
    TooManyRedirects,
    RateLimited,
    HttpStatus(u16),
    Network,
}

pub trait Fetcher {
    /// GET `url`, sending `Origin: origin`, reading at most `max_bytes`.
    /// 429, or 403 with a rate-limit header, is `FetchError::RateLimited`.
    fn get(&self, url: &str, max_bytes: usize, origin: &str) -> Result<Response, FetchError>;
}

/// The real fetcher: blocking reqwest, no cookies, no credentials, no
/// referrer, spaces requests per host by `policy.request_gap`.
pub struct HttpFetcher {
    policy: FetchPolicy,
    github_token: Option<String>,
    /// Start time of the last request per lowercase host (no port).
    last_start: Mutex<BTreeMap<String, Instant>>,
}

/// Redirects followed before a request fails (6.4, "Fetch rules").
const MAX_REDIRECTS: usize = 5;
/// The longest wait for a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The host of `url` as an address, when it is a literal one.
fn ip_literal(url: &Url) -> Option<std::net::IpAddr> {
    let host = url.host_str()?;
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

/// Addresses a request must be pinned to: the host name and what it
/// resolved to, all of it public.
type Pin = (String, Vec<std::net::SocketAddr>);

impl HttpFetcher {
    /// Installs the rustls `ring` provider (once per process; a second call
    /// is harmless). `github_token` is sent as an Authorization header only to
    /// `api.github.com` (see [`auth_header_for`]).
    pub fn new(policy: &FetchPolicy, github_token: Option<String>) -> HttpFetcher {
        // reqwest is built with `rustls-no-provider`; a second install
        // returns Err, which is exactly the harmless case.
        let _ = rustls::crypto::ring::default_provider().install_default();
        HttpFetcher {
            policy: policy.clone(),
            github_token,
            last_start: Mutex::new(BTreeMap::new()),
        }
    }

    /// True for a loopback host the test-only allowance permits.
    #[cfg(feature = "test-loopback")]
    fn loopback_allowed(&self, url: &Url) -> bool {
        self.policy.allow_http_loopback
            && match ip_literal(url) {
                Some(ip) => ip.is_loopback(),
                None => url
                    .host_str()
                    .is_some_and(|h| h.eq_ignore_ascii_case("localhost")),
            }
    }

    #[cfg(not(feature = "test-loopback"))]
    fn loopback_allowed(&self, _url: &Url) -> bool {
        false
    }

    /// Checks one hop: `https`, no userinfo, and only public addresses.
    fn admit(&self, url: &mut Url) -> Result<Option<Pin>, FetchError> {
        let loopback = self.loopback_allowed(url);
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            _ => return Err(FetchError::NotHttps),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::Network);
        }
        if loopback {
            // A loopback `https` reference is read over plain http.
            let _ = url.set_scheme("http");
            return Ok(None);
        }
        if let Some(ip) = ip_literal(url) {
            return if sources::is_public_host(ip) {
                Ok(None)
            } else {
                Err(FetchError::NotPublicHost)
            };
        }
        let host = url.host_str().ok_or(FetchError::NotHttps)?.to_string();
        let port = url.port_or_known_default().ok_or(FetchError::NotHttps)?;
        let addrs: Vec<std::net::SocketAddr> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| FetchError::Network)?
            .collect();
        if addrs.is_empty() {
            return Err(FetchError::Network);
        }
        if addrs.iter().any(|a| !sources::is_public_host(a.ip())) {
            return Err(FetchError::NotPublicHost);
        }
        Ok(Some((host, addrs)))
    }

    /// Waits until `request_gap` has passed since the last request to this
    /// host started, then records this one's start.
    fn space(&self, host: &str) {
        let key = host.to_ascii_lowercase();
        let mut last = self.last_start.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = last.get(&key) {
            let due = *prev + self.policy.request_gap;
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
        last.insert(key, Instant::now());
    }
}

fn is_rate_limited(status: reqwest::StatusCode, headers: &reqwest::header::HeaderMap) -> bool {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    status == reqwest::StatusCode::FORBIDDEN
        && (headers.contains_key(reqwest::header::RETRY_AFTER)
            || headers
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.trim() == "0"))
}

fn classify(err: &reqwest::Error) -> FetchError {
    if err.is_timeout() {
        FetchError::Timeout
    } else {
        FetchError::Network
    }
}

impl Fetcher for HttpFetcher {
    fn get(&self, url: &str, max_bytes: usize, origin: &str) -> Result<Response, FetchError> {
        let mut current = Url::parse(url).map_err(|_| FetchError::NotHttps)?;
        let started = Instant::now();
        let mut followed = 0usize;
        loop {
            let remaining = self
                .policy
                .timeout
                .checked_sub(started.elapsed())
                .filter(|d| !d.is_zero())
                .ok_or(FetchError::Timeout)?;
            let pin = self.admit(&mut current)?;
            let host = current.host_str().unwrap_or("").to_string();

            let mut builder = reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .connect_timeout(CONNECT_TIMEOUT.min(remaining))
                .timeout(remaining)
                .user_agent(format!("hew-cli/{}", crate::VERSION));
            if let Some((name, addrs)) = &pin {
                builder = builder.resolve_to_addrs(name, addrs);
            }
            let client = builder.build().map_err(|_| FetchError::Network)?;
            let mut request = client
                .get(current.clone())
                .header(reqwest::header::ORIGIN, origin);
            if current.port().is_none()
                && let Some((name, value)) = auth_header_for(&host, self.github_token.as_deref())
            {
                request = request.header(name, value);
            }

            self.space(&host);
            let response = request.send().map_err(|e| classify(&e))?;
            let status = response.status();

            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
                && let Some(location) = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            {
                if followed >= MAX_REDIRECTS {
                    return Err(FetchError::TooManyRedirects);
                }
                followed += 1;
                current = current.join(&location).map_err(|_| FetchError::Network)?;
                continue;
            }
            if is_rate_limited(status, response.headers()) {
                return Err(FetchError::RateLimited);
            }
            if !status.is_success() {
                return Err(FetchError::HttpStatus(status.as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|n| n > max_bytes as u64)
            {
                return Err(FetchError::TooLarge);
            }
            let headers = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_ascii_lowercase(), v.to_string()))
                })
                .collect();
            let mut body = Vec::new();
            response
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::TimedOut {
                        FetchError::Timeout
                    } else {
                        FetchError::Network
                    }
                })?;
            if body.len() > max_bytes {
                return Err(FetchError::TooLarge);
            }
            return Ok(Response {
                status: status.as_u16(),
                final_url: current.to_string(),
                headers,
                body,
            });
        }
    }
}

/// `Some((name, value))` when `token` is present and `host` is exactly
/// `api.github.com` (any case); `None` for every other host, including
/// `raw.githubusercontent.com`, `github.com` and lookalikes.
pub fn auth_header_for(host: &str, token: Option<&str>) -> Option<(String, String)> {
    let token = token.filter(|t| !t.is_empty())?;
    host.eq_ignore_ascii_case("api.github.com")
        .then(|| ("Authorization".to_string(), format!("Bearer {token}")))
}

/// The syntax-only license rule (7.6): `None` when `license` is either
/// `LicenseRef-[A-Za-z0-9.-]+` with a non-empty https `license_url`, or an
/// SPDX-expression-shaped string (tokens of `[A-Za-z0-9.+-]`, the operators
/// AND / OR / WITH, balanced parentheses). There is no table of SPDX ids.
fn license_problem(license: &str, license_url: Option<&str>) -> Option<&'static str> {
    if let Some(id) = license.strip_prefix("LicenseRef-") {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Some("is not a valid LicenseRef-<id>");
        }
        let https =
            license_url.is_some_and(|u| u.len() > "https://".len() && u.starts_with("https://"));
        return (!https).then_some("is a LicenseRef and needs an https licenseUrl on the index");
    }
    let bad = Some("is not an SPDX expression (use an id like CC-BY-4.0)");
    let mut depth = 0usize;
    // true while an id or "(" is due; false while an operator, ")" or the
    // end is due.
    let mut operand_due = true;
    for tok in license
        .replace('(', " ( ")
        .replace(')', " ) ")
        .split_whitespace()
    {
        match (tok, operand_due) {
            ("(", true) => depth += 1,
            (")", false) => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return Some("has unbalanced parentheses"),
            },
            ("AND" | "OR" | "WITH", false) => operand_due = true,
            (t, true)
                if !matches!(t, "(" | ")" | "AND" | "OR" | "WITH")
                    && t.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')) =>
            {
                operand_due = false
            }
            _ => return bad,
        }
    }
    if depth != 0 {
        return Some("has unbalanced parentheses");
    }
    if operand_due { bad } else { None }
}

/// One failed listing check: a stable `code` and a one-line `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckFailure {
    pub code: &'static str,
    pub message: String,
}

/// Codes, one per listing check of 7.6.
pub mod code {
    pub const REFERENCE: &str = "reference";
    pub const INDEX_INVALID: &str = "index_invalid";
    pub const LICENSE_MISSING: &str = "license_missing";
    pub const LICENSE_FORMAT: &str = "license_format";
    pub const FILE_INTEGRITY: &str = "file_integrity";
    pub const ITEM_MISMATCH: &str = "item_mismatch";
    pub const IDS_UNIQUE: &str = "ids_unique";
    pub const PUBLIC_HTTPS: &str = "public_https";
    pub const CORS: &str = "cors";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const UNREACHABLE: &str = "unreachable";
}

/// Output order of the failure lines.
const ORDER: [&str; 11] = [
    code::REFERENCE,
    code::INDEX_INVALID,
    code::LICENSE_MISSING,
    code::LICENSE_FORMAT,
    code::FILE_INTEGRITY,
    code::ITEM_MISMATCH,
    code::IDS_UNIQUE,
    code::PUBLIC_HTTPS,
    code::CORS,
    code::RATE_LIMITED,
    code::UNREACHABLE,
];

/// Collects what each check found; `finish` makes one failure per check.
#[derive(Default)]
struct Findings {
    details: BTreeMap<&'static str, Vec<String>>,
}

impl Findings {
    fn add(&mut self, code: &'static str, detail: impl Into<String>) {
        let one_line = detail
            .into()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.details.entry(code).or_default().push(one_line);
    }

    fn finish(mut self) -> Vec<CheckFailure> {
        ORDER
            .iter()
            .filter_map(|c| {
                let d = self.details.remove(c)?;
                let mut message = d[0].clone();
                if d.len() > 1 {
                    message.push_str(&format!(" (and {} more)", d.len() - 1));
                }
                Some(CheckFailure { code: c, message })
            })
            .collect()
    }
}

/// The reference check for a list file: every entry canonical, the list
/// sorted and unique. One failure (code `reference`) per violated property
/// is NOT required; at most one `reference` failure per bad entry or
/// ordering problem, each naming it.
pub fn check_list(references: &[String]) -> Vec<CheckFailure> {
    let mut out = Vec::new();
    let mut fail = |message: String| {
        out.push(CheckFailure {
            code: code::REFERENCE,
            message,
        })
    };
    for r in references {
        if let Some(why) = reference_problem(r) {
            fail(format!("\"{r}\": {why}"));
        }
    }
    let mut seen = BTreeSet::new();
    for r in references {
        if !seen.insert(r.as_str()) {
            fail(format!("\"{r}\" is listed more than once"));
        }
    }
    if let Some(w) = references.windows(2).find(|w| w[0] > w[1]) {
        fail(format!(
            "the list is not sorted: \"{}\" comes before \"{}\"",
            w[0], w[1]
        ));
    }
    out
}

/// Why `reference` is not a canonical source reference, if it is not.
fn reference_problem(reference: &str) -> Option<String> {
    match sources::parse_source_ref(reference) {
        Err(e) => Some(format!("not a source reference ({e:?})")),
        Ok(c) => match c.first() {
            Some(first) if first.canonical == reference => None,
            Some(first) => Some(format!(
                "not canonical, write it as \"{}\"",
                first.canonical
            )),
            None => Some("not a source reference".to_string()),
        },
    }
}

const INDEX_CAP: usize = 4 * 1024 * 1024;
const THUMBNAIL_CAP: usize = 1024 * 1024;
const RESOLVE_CAP: usize = 64 * 1024;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// What reading one source found.
struct Inspection {
    failures: Vec<CheckFailure>,
    /// The index, when it parsed.
    index: Option<SourceIndex>,
    source: Option<SourceRef>,
    commit: Option<String>,
}

/// Fetching with the cross-origin check applied to every answer.
struct Reader<'a> {
    fetcher: &'a dyn Fetcher,
    found: Findings,
    cors_misses: Vec<String>,
}

impl Reader<'_> {
    /// GETs `url`; a successful answer is checked for a usable
    /// `Access-Control-Allow-Origin` (`*` or exactly the web origin).
    fn get(&mut self, url: &Url, cap: usize, what: &str) -> Result<Response, FetchError> {
        let r = self.fetcher.get(url.as_str(), cap, WEB_ORIGIN)?;
        let ok = r.headers.iter().any(|(k, v)| {
            k == "access-control-allow-origin" && (v.trim() == "*" || v.trim() == WEB_ORIGIN)
        });
        if !ok {
            self.cors_misses.push(what.to_string());
        }
        Ok(r)
    }

    /// Records a failed request; true when the check must stop (the source
    /// cannot be read further).
    fn failed(&mut self, err: &FetchError, what: &str, item: bool) -> bool {
        match err {
            FetchError::NotHttps | FetchError::NotPublicHost => {
                self.found.add(
                    code::PUBLIC_HTTPS,
                    format!("{what} is not served over https from a public address"),
                );
                !item
            }
            FetchError::RateLimited => {
                self.found.add(
                    code::RATE_LIMITED,
                    format!("the host rate-limited the request for {what}"),
                );
                true
            }
            FetchError::HttpStatus(s) if item => {
                self.found
                    .add(code::FILE_INTEGRITY, format!("{what} answered HTTP {s}"));
                false
            }
            FetchError::TooLarge if item => {
                self.found.add(
                    code::FILE_INTEGRITY,
                    format!("{what} is larger than its limit"),
                );
                false
            }
            other => {
                self.found.add(
                    code::UNREACHABLE,
                    format!("{what} could not be read ({other:?})"),
                );
                true
            }
        }
    }
}

fn inspect(reference: &str, fetcher: &dyn Fetcher) -> Inspection {
    let mut r = Reader {
        fetcher,
        found: Findings::default(),
        cors_misses: Vec::new(),
    };
    let mut result = Inspection {
        failures: Vec::new(),
        index: None,
        source: None,
        commit: None,
    };
    read_source(reference, &mut r, &mut result);
    if let Some(first) = r.cors_misses.first() {
        let more = r.cors_misses.len() - 1;
        let tail = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        r.found.add(
            code::CORS,
            format!("no Access-Control-Allow-Origin of * or {WEB_ORIGIN} on {first}{tail}"),
        );
    }
    result.failures = r.found.finish();
    result
}

fn read_source(reference: &str, r: &mut Reader, result: &mut Inspection) {
    let source = match sources::parse_source_ref(reference) {
        Ok(c) if !c.is_empty() => c.into_iter().next().expect("non-empty"),
        _ => {
            r.found.add(
                code::REFERENCE,
                format!("\"{reference}\" is not a source reference"),
            );
            return;
        }
    };
    if source.canonical != reference {
        r.found.add(
            code::REFERENCE,
            format!(
                "\"{reference}\" is not canonical, write it as \"{}\"",
                source.canonical
            ),
        );
    }
    result.source = Some(source.clone());

    // Git sources: resolve the ref to a commit, then read at that commit.
    let mut commit: Option<String> = None;
    if let Some(url) = sources::resolve_url(&source) {
        match r.get(&url, RESOLVE_CAP, "the commit lookup") {
            Ok(resp) => match sources::commit_from_response(source.kind, &resp.body) {
                Some(c) => commit = Some(c),
                None => {
                    r.found
                        .add(code::UNREACHABLE, "the commit lookup gave no commit");
                    return;
                }
            },
            // 6.2: a rate-limited resolve reads at the ref instead.
            Err(FetchError::RateLimited) => {}
            Err(e) => {
                r.failed(&e, "the commit lookup", false);
                return;
            }
        }
    }
    result.commit = commit.clone();

    let index_url = sources::index_url(&source, commit.as_deref());
    let body = match r.get(&index_url, INDEX_CAP, "hew.json") {
        Ok(resp) => resp.body,
        Err(FetchError::TooLarge) => {
            r.found
                .add(code::INDEX_INVALID, "hew.json is larger than 4 MB");
            return;
        }
        Err(e) => {
            r.failed(&e, "hew.json", false);
            return;
        }
    };
    let index = match sources::parse_index(&body) {
        Ok(i) => i,
        Err(e) => {
            r.found
                .add(code::INDEX_INVALID, format!("{}: {e:?}", e.code()));
            return;
        }
    };

    if index.license.as_deref().is_none_or(|l| l.trim().is_empty()) {
        r.found
            .add(code::LICENSE_MISSING, "the index states no license");
    } else if let Some(l) = index.license.as_deref()
        && let Some(why) = license_problem(l, index.license_url.as_deref())
    {
        r.found
            .add(code::LICENSE_FORMAT, format!("the index license {why}"));
    }
    for item in &index.items {
        if let Some(l) = item.license.as_deref()
            && let Some(why) = license_problem(l, index.license_url.as_deref())
        {
            r.found.add(
                code::LICENSE_FORMAT,
                format!("the license of {} {why}", item.path),
            );
        }
    }
    let mut ids = BTreeSet::new();
    for item in &index.items {
        if !ids.insert(item.id.as_str()) {
            r.found.add(
                code::IDS_UNIQUE,
                format!("id {} is used more than once", item.id),
            );
        }
    }

    for item in &index.items {
        let Ok(url) = sources::file_url(&source, &item.path, commit.as_deref()) else {
            r.found.add(
                code::INDEX_INVALID,
                format!("{} is not a usable path", item.path),
            );
            continue;
        };
        match r.get(&url, item.bytes as usize, &item.path) {
            Ok(resp) => check_item(r, item, &resp.body),
            Err(e) => {
                if r.failed(&e, &item.path, true) {
                    break;
                }
            }
        }
        if let Some(thumb) = &item.thumbnail {
            let Ok(url) = sources::file_url(&source, thumb, commit.as_deref()) else {
                r.found
                    .add(code::INDEX_INVALID, format!("{thumb} is not a usable path"));
                continue;
            };
            match r.get(&url, THUMBNAIL_CAP, thumb) {
                Ok(resp) => {
                    if !resp.body.starts_with(PNG_MAGIC) {
                        r.found
                            .add(code::FILE_INTEGRITY, format!("{thumb} is not a PNG image"));
                    }
                }
                Err(e) => {
                    if r.failed(&e, thumb, true) {
                        break;
                    }
                }
            }
        }
    }
    result.index = Some(index);
}

/// Hash, size, load, id and category of one fetched item.
fn check_item(r: &mut Reader, item: &sources::IndexItem, body: &[u8]) {
    if body.len() as u64 != item.bytes || library::sha256_hex(body) != item.sha256 {
        r.found.add(
            code::FILE_INTEGRITY,
            format!("{} does not match its listed hash and size", item.path),
        );
        return;
    }
    let loaded = std::panic::catch_unwind(|| kernel::read_item_summary(body));
    let summary = match loaded {
        Ok(Ok(s)) => s,
        _ => {
            r.found.add(
                code::ITEM_MISMATCH,
                format!("{} does not load as a Hew file", item.path),
            );
            return;
        }
    };
    let raw = summary
        .doc_attrs
        .get("hew.library")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let meta = library::parse_item_meta(&raw);
    if meta.id.as_deref() != Some(item.id.as_str()) {
        r.found.add(
            code::ITEM_MISMATCH,
            format!("{} has a different id than its index entry", item.path),
        );
    }
    // The same mapping `library index` writes. A file with no category
    // would be listed under a guess the app can disagree with, so it fails.
    let Some(meta_category) = meta.category else {
        r.found.add(
            code::ITEM_MISMATCH,
            format!("{} has no hew.library category", item.path),
        );
        return;
    };
    let category = match meta_category {
        library::Category::Material => Category::Materials,
        library::Category::Model => Category::Models,
        library::Category::Profile => Category::Profiles,
        library::Category::Component => Category::Components,
    };
    if category != item.category {
        r.found.add(
            code::ITEM_MISMATCH,
            format!("{} is a different category than its index entry", item.path),
        );
    }
}

/// Runs every listing check on one source. Empty means it passes.
pub fn check_source(
    reference: &str,
    fetcher: &dyn Fetcher,
    _policy: &FetchPolicy,
) -> Vec<CheckFailure> {
    if let Some(why) = malformed_reference(reference) {
        return vec![CheckFailure {
            code: code::REFERENCE,
            message: why,
        }];
    }
    inspect(reference, fetcher).failures
}

/// Why `reference` cannot be a source reference before it is even parsed:
/// it begins with `-` (it would be read as a flag by any tool it is passed
/// to), or holds whitespace or a control character (a line of a list file
/// is one token). References come from pull requests, so they are untrusted.
fn malformed_reference(reference: &str) -> Option<String> {
    if reference.starts_with('-') {
        return Some(format!(
            "\"{}\" begins with \"-\"; a source reference never does",
            reference.escape_debug()
        ));
    }
    if reference
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Some(format!(
            "\"{}\" has whitespace or a control character",
            reference.escape_debug()
        ));
    }
    None
}

/// `hew-cli catalog check <ref>` after the fetcher is chosen: prints one
/// line per failure to `out`, formatted `"<code>: <message>"`, and returns
/// the exit code (0 none, 1 any failure, 2 usage). A reference that begins
/// with `-` or holds whitespace is a `reference` failure, never a flag.
pub fn run_check(
    args: &[String],
    fetcher: &dyn Fetcher,
    policy: &FetchPolicy,
    out: &mut dyn std::io::Write,
) -> i32 {
    let [reference] = args else {
        return 2;
    };
    let failures = check_source(reference, fetcher, policy);
    for f in &failures {
        if writeln!(out, "{}: {}", f.code, f.message).is_err() {
            return 1;
        }
    }
    i32::from(!failures.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildReport {
    /// Canonical references written fresh this run.
    pub fresh: Vec<String>,
    /// Canonical references kept from `previous` and marked stale.
    pub stale: Vec<String>,
    /// Canonical references written under `removed` only.
    pub removed: Vec<String>,
    /// Canonical references that failed with no previous snapshot to keep:
    /// omitted from `sources`.
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// A list file is unreadable or not a JSON array of strings.
    BadList {
        file: String,
        reason: String,
    },
    Io(String),
}

fn bad_list(file: &str, reason: impl Into<String>) -> BuildError {
    BuildError::BadList {
        file: file.to_string(),
        reason: reason.into(),
    }
}

fn read_json(checkout: &Path, file: &str) -> Result<serde_json::Value, BuildError> {
    let bytes = std::fs::read(checkout.join(file)).map_err(|e| bad_list(file, e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|e| bad_list(file, format!("not JSON ({e})")))
}

fn read_references(checkout: &Path, file: &str) -> Result<Vec<String>, BuildError> {
    let serde_json::Value::Array(items) = read_json(checkout, file)? else {
        return Err(bad_list(file, "not a JSON array"));
    };
    items
        .into_iter()
        .map(|v| match v {
            serde_json::Value::String(s) => Ok(s),
            _ => Err(bad_list(file, "an entry is not a string")),
        })
        .collect()
}

/// `removed.json`: entries kept verbatim, each checked exactly as
/// `sources::parse_catalog` will check it (a canonical source, a severity of
/// removed or critical, a reason, an https link when present), so a build
/// never writes a catalog its own parser refuses.
fn read_removed(checkout: &Path) -> Result<Vec<serde_json::Value>, BuildError> {
    const FILE: &str = "removed.json";
    let serde_json::Value::Array(entries) = read_json(checkout, FILE)? else {
        return Err(bad_list(FILE, "not a JSON array"));
    };
    for (n, e) in entries.iter().enumerate() {
        let at = |why: &str| bad_list(FILE, format!("entry {} (#{}): {why}", n + 1, n + 1));
        let Some(source) = e.get("source").and_then(|v| v.as_str()) else {
            return Err(at("needs a source"));
        };
        let canonical = matches!(
            sources::parse_source_ref(source).as_deref(),
            Ok([only]) if only.canonical == source
        );
        if !canonical {
            return Err(at(&format!(
                "{source:?} is not a canonical source reference"
            )));
        }
        if !matches!(
            e.get("severity").and_then(|v| v.as_str()),
            Some("removed" | "critical")
        ) {
            return Err(at("the severity must be removed or critical"));
        }
        if !e.get("reason").is_some_and(|v| v.is_string()) {
            return Err(at("needs a reason"));
        }
        match e.get("link") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(l))
                if Url::parse(l).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some()) => {
            }
            Some(_) => return Err(at("the link must be an https URL")),
        }
    }
    Ok(entries)
}

/// Recursively sorts object keys, so the output does not depend on map order.
fn canonical(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canonical(&m[k])))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn to_file_bytes(v: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(&canonical(v)).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

fn snapshot_rel(canonical_ref: &str) -> String {
    format!("sources/{}.json", snapshot_slug(canonical_ref))
}

/// The entry and snapshot a previous build kept for `reference`.
fn previous_snapshot(
    previous: Option<&Path>,
    catalog: Option<&sources::Catalog>,
    reference: &str,
) -> Option<(CatalogSource, Vec<u8>)> {
    let dir = previous?;
    let entry = catalog?.sources.iter().find(|s| s.source == reference)?;
    // The app follows `snapshot`, but a build only trusts the shape it writes.
    if entry.snapshot != snapshot_rel(reference) {
        return None;
    }
    let bytes = std::fs::read(dir.join(&entry.snapshot)).ok()?;
    sources::parse_index(&bytes).ok()?;
    Some((entry.clone(), bytes))
}

fn fresh_entry(source: &SourceRef, commit: Option<String>, index: &SourceIndex) -> CatalogSource {
    let mut categories = BTreeMap::new();
    for item in &index.items {
        *categories
            .entry(format!("{:?}", item.category))
            .or_insert(0u64) += 1;
    }
    CatalogSource {
        source: source.canonical.clone(),
        kind: source.kind,
        commit,
        name: index.name.clone(),
        description: index.description.clone(),
        author: index.author.clone(),
        homepage: index.homepage.clone(),
        license: index.license.clone(),
        license_url: index.license_url.clone(),
        items: index.items.len() as u64,
        bytes: index.items.iter().map(|i| i.bytes).sum(),
        categories,
        stale: false,
        snapshot: snapshot_rel(&source.canonical),
    }
}

/// Writes `catalog.json` and `sources/<slug>.json` into `out`.
/// `built` is the ISO time written to `catalog.json` (the injected clock).
/// A source that fails keeps its snapshot from `previous` (a directory with
/// the same layout) and is marked stale; a failure never aborts the build.
pub fn build_catalog(
    checkout: &Path,
    out: &Path,
    fetcher: &dyn Fetcher,
    _policy: &FetchPolicy,
    previous: Option<&Path>,
    built: &str,
) -> Result<BuildReport, BuildError> {
    let mut references: BTreeSet<String> = read_references(checkout, "library.json")?
        .into_iter()
        .collect();
    references.extend(read_references(checkout, "plugins.json")?);
    let removed_entries = read_removed(checkout)?;
    let removed_refs: Vec<SourceRef> = removed_entries
        .iter()
        .filter_map(|e| e["source"].as_str())
        .filter_map(|s| sources::parse_source_ref(s).ok()?.into_iter().next())
        .collect();

    let previous_catalog = previous
        .and_then(|d| std::fs::read(d.join("catalog.json")).ok())
        .and_then(|b| sources::parse_catalog(&b).ok());

    let mut report = BuildReport::default();
    let mut entries: Vec<CatalogSource> = Vec::new();
    // Snapshot file name -> bytes, all held in memory until the writes, so
    // that `previous` may be `out` itself.
    let mut snapshots: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for reference in &references {
        let parsed = sources::parse_source_ref(reference)
            .ok()
            .and_then(|c| c.into_iter().next());
        if let Some(p) = &parsed
            && removed_refs
                .iter()
                .any(|r| sources::same_source_ignoring_ref(r, p))
        {
            report.removed.push(reference.clone());
            continue;
        }
        let inspection = inspect(reference, fetcher);
        if let (true, Some(index), Some(source)) = (
            inspection.failures.is_empty(),
            inspection.index.as_ref(),
            inspection.source.as_ref(),
        ) {
            let entry = fresh_entry(source, inspection.commit.clone(), index);
            let value = serde_json::to_value(index)
                .map_err(|e| BuildError::Io(format!("cannot write a snapshot: {e}")))?;
            snapshots.insert(entry.snapshot.clone(), to_file_bytes(&value));
            entries.push(entry);
            report.fresh.push(reference.clone());
        } else if let Some((mut entry, bytes)) =
            previous_snapshot(previous, previous_catalog.as_ref(), reference)
        {
            entry.stale = true;
            snapshots.insert(entry.snapshot.clone(), bytes);
            entries.push(entry);
            report.stale.push(reference.clone());
        } else {
            report.failed.push(reference.clone());
        }
    }

    let catalog = serde_json::json!({
        "hew": sources::SUPPORTED_HEW,
        "built": built,
        "sources": entries,
        "removed": removed_entries,
    });
    write_output(out, &snapshots, &to_file_bytes(&catalog))
        .map_err(|e| BuildError::Io(format!("{}: {e}", out.display())))?;
    Ok(report)
}

fn write_output(
    out: &Path,
    snapshots: &BTreeMap<String, Vec<u8>>,
    catalog: &[u8],
) -> std::io::Result<()> {
    let dir = out.join("sources");
    std::fs::create_dir_all(&dir)?;
    // A snapshot nobody lists any more goes away.
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        let rel = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| format!("sources/{n}"));
        if path.is_file()
            && path.extension().is_some_and(|e| e == "json")
            && rel.is_some_and(|r| !snapshots.contains_key(&r))
        {
            std::fs::remove_file(&path)?;
        }
    }
    for (rel, bytes) in snapshots {
        std::fs::write(out.join(rel), bytes)?;
    }
    std::fs::write(out.join("catalog.json"), catalog)
}

/// The previous directory `catalog build` uses: the `--previous` value when
/// given, else `out` when it already exists as a directory, else `None`.
pub fn resolve_previous(flag: Option<&Path>, out: &Path) -> Option<PathBuf> {
    match flag {
        Some(p) => Some(p.to_path_buf()),
        None if out.is_dir() => Some(out.to_path_buf()),
        None => None,
    }
}

/// First 16 hex digits of the SHA-256 of the canonical reference.
pub fn snapshot_slug(canonical: &str) -> String {
    library::sha256_hex(canonical.as_bytes())[..16].to_string()
}
