//! The desktop's `remote_fetch` command: the one way the webview reads a
//! Library source (docs/design/shared-library-sources.md decisions 24, 25
//! and 28, section 6.4 "Fetch rules", section 7.1).
//!
//! Rules, desktop column: `https` on the first URL and on every redirect;
//! no credentials, cookies or referrer; at most 5 redirects; 10 s to
//! connect and 120 s in all; the body is read against `maxBytes` and cut
//! off there; HTTP 429, or 403 with a rate-limit header, is `rate_limited`.
//! The client is built as `report_client.rs` builds its own (platform
//! TLS verifier, system proxy, `Hew/<version>` user agent).
//!
//! Debug builds only: if `HEW_EXTRA_CA_PEM` names a PEM file, its
//! certificates are trusted IN ADDITION to the platform roots (reqwest's
//! `tls_certs_merge`, never `tls_certs_only`), so desktop acceptance tests
//! can reach a local https fixture without touching the user's keychain.
//! That code exists only under `cfg(debug_assertions)`, so a release binary
//! has no such variable to honor (the same discipline as `HEW_REPORT_URL`
//! in `report_client.rs`). It widens trust only: `https` is still required
//! and nothing else here changes. An unreadable or invalid file is a
//! `Network` error, never ignored, and neither the variable's value nor
//! the path is ever logged.
//!
//! Nothing in this module logs, prints or traces anything that contains
//! the URL (decision 28): a source may sit on a private address and
//! Report Bug bundles carry the log. A test below scans this file's
//! non-test source for logging macros to keep it that way.

use std::time::Duration;

use serde::Serialize;

/// The error the command rejects with. Serialized as an internally tagged
/// object whose `error` member is the design doc's snake_case code, which
/// is what `RemoteFetcher.fetchBytes` rejects with on the app side:
///
/// ```json
/// {"error":"not_https"}
/// {"error":"http_status","code":403}
/// ```
///
/// (Not camelCase: the doc names the codes `not_https`, `too_large`,
/// `too_many_redirects`, `rate_limited`, `http_status`.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum RemoteFetchError {
    NotHttps,
    TooLarge,
    Timeout,
    TooManyRedirects,
    RateLimited,
    HttpStatus { code: u16 },
    Network,
}

/// Knobs for `fetch_bytes`. The command uses `Default`; tests shrink the
/// timeouts and, under `cfg(test)` only, may allow plain `http` to a
/// loopback host (`127.0.0.1`, `::1`, `localhost`) on the first URL and on
/// redirects. No non-test build contains that field.
#[derive(Debug, Clone)]
pub(crate) struct FetchOpts {
    #[cfg(test)]
    pub allow_http_loopback: bool,
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_redirects: usize,
    /// Debug builds only: a PEM file of extra trusted roots, merged with
    /// the platform roots. `Default` fills it from `HEW_EXTRA_CA_PEM`.
    #[cfg(debug_assertions)]
    pub extra_root_pem: Option<std::path::PathBuf>,
}

impl Default for FetchOpts {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            allow_http_loopback: false,
            connect_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(120),
            max_redirects: 5,
            #[cfg(debug_assertions)]
            extra_root_pem: extra_root_pem_path(
                cfg!(debug_assertions),
                std::env::var("HEW_EXTRA_CA_PEM").ok().as_deref(),
            ),
        }
    }
}

/// The extra-root PEM path for a given build mode and variable value:
/// `None` unless `is_debug` and the value is non-empty. The production
/// call site passes `cfg!(debug_assertions)`.
#[cfg(debug_assertions)]
fn extra_root_pem_path(is_debug: bool, env_value: Option<&str>) -> Option<std::path::PathBuf> {
    match env_value {
        Some(v) if is_debug && !v.is_empty() => Some(std::path::PathBuf::from(v)),
        _ => None,
    }
}

/// Statuses that carry a `Location` the client follows by hand.
fn is_followed_redirect(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

/// Checks one hop's URL: `https` only (plain `http` to loopback in tests),
/// and no userinfo, which would put credentials on the wire.
fn check_url(url: &url::Url, opts: &FetchOpts) -> Result<(), RemoteFetchError> {
    let allowed = match url.scheme() {
        "https" => true,
        #[cfg(test)]
        "http" => {
            opts.allow_http_loopback
                && match url.host() {
                    Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
                    Some(url::Host::Ipv4(a)) => a.is_loopback(),
                    Some(url::Host::Ipv6(a)) => a.is_loopback(),
                    None => false,
                }
        }
        _ => false,
    };
    #[cfg(not(test))]
    let _ = opts;
    if !allowed {
        return Err(RemoteFetchError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RemoteFetchError::Network);
    }
    Ok(())
}

fn classify(err: &reqwest::Error) -> RemoteFetchError {
    if err.is_timeout() {
        RemoteFetchError::Timeout
    } else {
        RemoteFetchError::Network
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

/// Fetches `url` and returns its body, enforcing every rule above.
pub(crate) async fn fetch_bytes(
    url: &str,
    max_bytes: usize,
    opts: &FetchOpts,
) -> Result<Vec<u8>, RemoteFetchError> {
    let mut current = url::Url::parse(url).map_err(|_| RemoteFetchError::NotHttps)?;
    check_url(&current, opts)?;

    // Same idempotent provider install as report_client.rs. No cookie
    // store (the `cookies` feature is off), and redirects are followed by
    // hand below so every hop is checked and no referer is added.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = reqwest::Client::builder()
        .connect_timeout(opts.connect_timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("Hew/{}", crate::version::VERSION));
    #[cfg(debug_assertions)]
    let builder = match &opts.extra_root_pem {
        Some(path) => {
            let pem = std::fs::read(path).map_err(|_| RemoteFetchError::Network)?;
            let certs = reqwest::Certificate::from_pem_bundle(&pem)
                .map_err(|_| RemoteFetchError::Network)?;
            if certs.is_empty() {
                return Err(RemoteFetchError::Network);
            }
            builder.tls_certs_merge(certs)
        }
        None => builder,
    };
    let client = builder.build().map_err(|_| RemoteFetchError::Network)?;

    let started = std::time::Instant::now();
    let mut followed = 0usize;
    loop {
        let remaining = opts
            .total_timeout
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or(RemoteFetchError::Timeout)?;
        let mut response = client
            .get(current.clone())
            .timeout(remaining)
            .send()
            .await
            .map_err(|e| classify(&e))?;
        let status = response.status();

        if is_followed_redirect(status) {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            if let Some(location) = location {
                if followed >= opts.max_redirects {
                    return Err(RemoteFetchError::TooManyRedirects);
                }
                followed += 1;
                current = current
                    .join(&location)
                    .map_err(|_| RemoteFetchError::Network)?;
                check_url(&current, opts)?;
                continue;
            }
        }

        if is_rate_limited(status, response.headers()) {
            return Err(RemoteFetchError::RateLimited);
        }
        if !status.is_success() {
            return Err(RemoteFetchError::HttpStatus {
                code: status.as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|n| n > max_bytes as u64)
        {
            return Err(RemoteFetchError::TooLarge);
        }

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| classify(&e))? {
            if body.len() + chunk.len() > max_bytes {
                // Dropping `response` closes the connection.
                return Err(RemoteFetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(body);
    }
}

#[tauri::command]
pub async fn remote_fetch(
    url: String,
    max_bytes: usize,
) -> Result<tauri::ipc::Response, RemoteFetchError> {
    fetch_bytes(&url, max_bytes, &FetchOpts::default())
        .await
        .map(tauri::ipc::Response::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    // ---- hand-rolled loopback HTTP server ---------------------------------

    /// What the handler is told about one request.
    struct Req {
        /// 0-based count of connections accepted so far.
        #[allow(dead_code)]
        index: usize,
        /// The raw request head (request line + headers).
        head: String,
    }

    impl Req {
        fn path(&self) -> &str {
            self.head.split_whitespace().nth(1).unwrap_or("")
        }
    }

    struct Server {
        port: u16,
        heads: Arc<Mutex<Vec<String>>>,
        connections: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl Server {
        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{}", self.port, path)
        }
        fn heads(&self) -> Vec<String> {
            self.heads.lock().unwrap().clone()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Starts a server; every accepted connection runs `handler` on its own
    /// thread after the request head has been read and recorded.
    fn serve(handler: impl Fn(Req, &mut TcpStream) + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        {
            let (heads, connections, stop) = (heads.clone(), connections.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let index = connections.fetch_add(1, Ordering::SeqCst);
                            let (heads, handler) = (heads.clone(), handler.clone());
                            std::thread::spawn(move || {
                                let mut head = Vec::new();
                                let mut byte = [0u8; 1];
                                while !head.ends_with(b"\r\n\r\n") {
                                    match stream.read(&mut byte) {
                                        Ok(1) => head.push(byte[0]),
                                        _ => return,
                                    }
                                }
                                let head = String::from_utf8_lossy(&head).into_owned();
                                heads.lock().unwrap().push(head.clone());
                                handler(Req { index, head }, &mut stream);
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
        }
        Server {
            port,
            heads,
            connections,
            stop,
        }
    }

    fn respond(s: &mut TcpStream, status: &str, extra: &[&str], body: &[u8]) {
        let mut out = format!("HTTP/1.1 {status}\r\n");
        for h in extra {
            out.push_str(h);
            out.push_str("\r\n");
        }
        out.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        let _ = s.write_all(out.as_bytes());
        let _ = s.write_all(body);
        let _ = s.flush();
    }

    fn loopback() -> FetchOpts {
        FetchOpts {
            allow_http_loopback: true,
            connect_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(10),
            ..FetchOpts::default()
        }
    }

    fn run(url: &str, max: usize, opts: &FetchOpts) -> Result<Vec<u8>, RemoteFetchError> {
        tauri::async_runtime::block_on(fetch_bytes(url, max, opts))
    }

    fn has_header(head: &str, name: &str) -> bool {
        head.lines().skip(1).any(|l| {
            l.split(':')
                .next()
                .map(|n| n.trim().eq_ignore_ascii_case(name))
                == Some(true)
        })
    }

    fn header_value(head: &str, name: &str) -> Option<String> {
        head.lines().skip(1).find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    }

    // ---- defaults and serialization ---------------------------------------

    #[test]
    fn default_opts_are_ten_seconds_connect_two_minutes_total_five_redirects() {
        let o = FetchOpts::default();
        assert_eq!(o.connect_timeout, Duration::from_secs(10));
        assert_eq!(o.total_timeout, Duration::from_secs(120));
        assert_eq!(o.max_redirects, 5);
        assert!(!o.allow_http_loopback);
    }

    #[test]
    fn errors_serialize_as_the_documented_snake_case_codes() {
        let j = |e: RemoteFetchError| serde_json::to_string(&e).unwrap();
        assert_eq!(j(RemoteFetchError::NotHttps), r#"{"error":"not_https"}"#);
        assert_eq!(j(RemoteFetchError::TooLarge), r#"{"error":"too_large"}"#);
        assert_eq!(j(RemoteFetchError::Timeout), r#"{"error":"timeout"}"#);
        assert_eq!(
            j(RemoteFetchError::TooManyRedirects),
            r#"{"error":"too_many_redirects"}"#
        );
        assert_eq!(
            j(RemoteFetchError::RateLimited),
            r#"{"error":"rate_limited"}"#
        );
        assert_eq!(
            j(RemoteFetchError::HttpStatus { code: 403 }),
            r#"{"error":"http_status","code":403}"#
        );
        assert_eq!(j(RemoteFetchError::Network), r#"{"error":"network"}"#);
    }

    // ---- scheme -----------------------------------------------------------

    #[test]
    fn a_body_comes_back_whole_on_success() {
        let s = serve(|_, c| respond(c, "200 OK", &[], b"hello"));
        assert_eq!(run(&s.url("/a"), 100, &loopback()).unwrap(), b"hello");
    }

    #[test]
    fn plain_http_is_refused_when_the_loopback_allowance_is_off() {
        let s = serve(|_, c| respond(c, "200 OK", &[], b"x"));
        let opts = FetchOpts {
            allow_http_loopback: false,
            ..loopback()
        };
        assert_eq!(
            run(&s.url("/a"), 100, &opts),
            Err(RemoteFetchError::NotHttps)
        );
        assert_eq!(
            s.connections.load(Ordering::SeqCst),
            0,
            "no request may leave"
        );
    }

    #[test]
    fn other_schemes_are_refused_even_with_the_allowance() {
        for u in [
            "ftp://127.0.0.1/a",
            "file:///etc/passwd",
            "ws://127.0.0.1/a",
        ] {
            assert_eq!(
                run(u, 100, &loopback()),
                Err(RemoteFetchError::NotHttps),
                "{u}"
            );
        }
    }

    #[test]
    fn the_allowance_covers_loopback_only_not_other_http_hosts() {
        assert_eq!(
            run("http://example.com/a", 100, &loopback()),
            Err(RemoteFetchError::NotHttps)
        );
    }

    #[test]
    fn a_redirect_to_plain_http_is_refused_without_following_it() {
        // The first hop is allowed loopback http; the target is a
        // non-loopback http URL, which must be refused before any request.
        let s = serve(|_, c| respond(c, "302 Found", &["Location: http://example.com/next"], b""));
        assert_eq!(
            run(&s.url("/a"), 100, &loopback()),
            Err(RemoteFetchError::NotHttps)
        );
        assert_eq!(s.connections.load(Ordering::SeqCst), 1);
    }

    // ---- credentials, cookies, referrer -----------------------------------

    #[test]
    fn no_authorization_cookie_or_referer_is_sent() {
        let s = serve(|_, c| respond(c, "200 OK", &[], b"x"));
        run(&s.url("/a"), 100, &loopback()).unwrap();
        let heads = s.heads();
        assert_eq!(heads.len(), 1);
        for h in ["authorization", "cookie", "referer", "proxy-authorization"] {
            assert!(!has_header(&heads[0], h), "sent {h}:\n{}", heads[0]);
        }
    }

    #[test]
    fn a_url_with_userinfo_is_refused_as_network_without_connecting() {
        let s = serve(|_, c| respond(c, "200 OK", &[], b"x"));
        let url = format!("http://user:secret@127.0.0.1:{}/a", s.port);
        assert_eq!(run(&url, 100, &loopback()), Err(RemoteFetchError::Network));
        assert_eq!(
            s.connections.load(Ordering::SeqCst),
            0,
            "no request may leave"
        );
    }

    #[test]
    fn a_set_cookie_is_not_replayed_on_a_later_call() {
        let s = serve(|_, c| respond(c, "200 OK", &["Set-Cookie: sid=abc; Path=/"], b"x"));
        run(&s.url("/a"), 100, &loopback()).unwrap();
        run(&s.url("/a"), 100, &loopback()).unwrap();
        let heads = s.heads();
        assert_eq!(heads.len(), 2);
        assert!(!has_header(&heads[1], "cookie"), "{}", heads[1]);
    }

    #[test]
    fn a_cookie_set_on_a_redirect_is_not_sent_on_the_next_hop_and_no_referer_is_added() {
        let s = serve(|r, c| {
            if r.path() == "/a" {
                respond(
                    c,
                    "302 Found",
                    &["Location: /b", "Set-Cookie: sid=abc; Path=/"],
                    b"",
                );
            } else {
                respond(c, "200 OK", &[], b"done");
            }
        });
        assert_eq!(run(&s.url("/a"), 100, &loopback()).unwrap(), b"done");
        let heads = s.heads();
        assert_eq!(heads.len(), 2);
        assert!(!has_header(&heads[1], "cookie"), "{}", heads[1]);
        assert!(!has_header(&heads[1], "referer"), "{}", heads[1]);
    }

    #[test]
    fn the_user_agent_is_hew_slash_version() {
        let s = serve(|_, c| respond(c, "200 OK", &[], b"x"));
        run(&s.url("/a"), 100, &loopback()).unwrap();
        assert_eq!(
            header_value(&s.heads()[0], "user-agent"),
            Some(format!("Hew/{}", crate::version::VERSION))
        );
    }

    // ---- redirects --------------------------------------------------------

    /// `/r/N` redirects to `/r/N-1`; `/r/0` answers 200.
    fn chain_server() -> Server {
        serve(|r, c| {
            let n: usize = r.path().trim_start_matches("/r/").parse().unwrap_or(0);
            if n == 0 {
                respond(c, "200 OK", &[], b"end");
            } else {
                respond(c, "302 Found", &[&format!("Location: /r/{}", n - 1)], b"");
            }
        })
    }

    #[test]
    fn five_redirects_are_followed() {
        let s = chain_server();
        assert_eq!(run(&s.url("/r/5"), 100, &loopback()).unwrap(), b"end");
    }

    #[test]
    fn a_sixth_redirect_is_too_many() {
        let s = chain_server();
        assert_eq!(
            run(&s.url("/r/6"), 100, &loopback()),
            Err(RemoteFetchError::TooManyRedirects)
        );
    }

    // ---- time -------------------------------------------------------------

    #[test]
    fn a_server_that_accepts_and_stalls_times_out_within_the_total_timeout() {
        let s = serve(|_, c| {
            // Never answer; hold the socket until the client gives up.
            let mut b = [0u8; 1];
            let _ = c.read(&mut b);
        });
        let opts = FetchOpts {
            total_timeout: Duration::from_millis(500),
            ..loopback()
        };
        let t = Instant::now();
        assert_eq!(
            run(&s.url("/a"), 100, &opts),
            Err(RemoteFetchError::Timeout)
        );
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "took {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn a_body_that_trickles_past_the_total_timeout_times_out() {
        let s = serve(|_, c| {
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n");
            for _ in 0..100 {
                if c.write_all(b"x").is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let opts = FetchOpts {
            total_timeout: Duration::from_millis(500),
            ..loopback()
        };
        let t = Instant::now();
        assert_eq!(
            run(&s.url("/a"), 5000, &opts),
            Err(RemoteFetchError::Timeout)
        );
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "took {:?}",
            t.elapsed()
        );
    }

    #[test]
    #[ignore = "needs a non-routable address; environment dependent"]
    fn connecting_to_a_black_hole_times_out_within_the_connect_timeout() {
        // 10.255.255.1 normally drops SYNs. Environment-dependent: a host
        // with no route answers "unreachable" at once; the assertion on
        // elapsed time still holds there, so only the code is relaxed to
        // allow Network when the route is missing outright.
        let opts = FetchOpts {
            connect_timeout: Duration::from_millis(300),
            total_timeout: Duration::from_secs(30),
            ..FetchOpts::default()
        };
        let t = Instant::now();
        let r = run("https://10.255.255.1/a", 100, &opts);
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "took {:?}",
            t.elapsed()
        );
        assert!(
            matches!(
                r,
                Err(RemoteFetchError::Timeout) | Err(RemoteFetchError::Network)
            ),
            "{r:?}"
        );
    }

    // ---- size -------------------------------------------------------------

    #[test]
    fn exactly_max_bytes_is_accepted() {
        let s = serve(|_, c| respond(c, "200 OK", &[], &[7u8; 1000]));
        assert_eq!(run(&s.url("/a"), 1000, &loopback()).unwrap().len(), 1000);
    }

    #[test]
    fn one_byte_over_max_bytes_is_too_large() {
        let s = serve(|_, c| respond(c, "200 OK", &[], &[7u8; 1001]));
        assert_eq!(
            run(&s.url("/a"), 1000, &loopback()),
            Err(RemoteFetchError::TooLarge)
        );
    }

    #[test]
    fn a_content_length_over_max_bytes_is_too_large_before_any_body_is_read() {
        let s = serve(|_, c| {
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n");
            let _ = c.flush();
            // No body: a reader that waits for one would hit the timeout.
            std::thread::sleep(Duration::from_secs(3));
        });
        let t = Instant::now();
        assert_eq!(
            run(&s.url("/a"), 1000, &loopback()),
            Err(RemoteFetchError::TooLarge)
        );
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "took {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn an_endless_body_is_cut_off_and_the_server_sees_the_connection_close() {
        let written = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let (w, f) = (written.clone(), finished.clone());
        let s = serve(move |_, c| {
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let chunk = [b'x'; 16 * 1024];
            // Stop on a write error (client closed) or after a 256 MB
            // guard, which a client that reads everything would reach.
            while w.load(Ordering::SeqCst) < 256 * 1024 * 1024 {
                if c.write_all(&chunk).is_err() {
                    break;
                }
                w.fetch_add(chunk.len(), Ordering::SeqCst);
            }
            f.store(true, Ordering::SeqCst);
        });
        assert_eq!(
            run(&s.url("/a"), 1000, &loopback()),
            Err(RemoteFetchError::TooLarge)
        );
        let t = Instant::now();
        while !finished.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            finished.load(Ordering::SeqCst),
            "server never saw the connection close"
        );
        assert!(
            written.load(Ordering::SeqCst) < 128 * 1024 * 1024,
            "client kept reading: {} bytes",
            written.load(Ordering::SeqCst)
        );
    }

    // ---- status mapping ---------------------------------------------------

    fn status_case(
        status: &'static str,
        extra: &'static [&'static str],
    ) -> Result<Vec<u8>, RemoteFetchError> {
        let s = serve(move |_, c| respond(c, status, extra, b"no"));
        run(&s.url("/a"), 100, &loopback())
    }

    #[test]
    fn http_429_is_rate_limited() {
        assert_eq!(
            status_case("429 Too Many Requests", &[]),
            Err(RemoteFetchError::RateLimited)
        );
    }

    #[test]
    fn a_403_with_an_exhausted_rate_limit_header_is_rate_limited() {
        assert_eq!(
            status_case("403 Forbidden", &["x-ratelimit-remaining: 0"]),
            Err(RemoteFetchError::RateLimited)
        );
    }

    #[test]
    fn a_403_with_retry_after_is_rate_limited() {
        assert_eq!(
            status_case("403 Forbidden", &["Retry-After: 60"]),
            Err(RemoteFetchError::RateLimited)
        );
    }

    #[test]
    fn a_403_with_rate_limit_remaining_above_zero_is_a_plain_403() {
        assert_eq!(
            status_case("403 Forbidden", &["x-ratelimit-remaining: 12"]),
            Err(RemoteFetchError::HttpStatus { code: 403 })
        );
    }

    #[test]
    fn plain_403_404_500_carry_their_code() {
        assert_eq!(
            status_case("403 Forbidden", &[]),
            Err(RemoteFetchError::HttpStatus { code: 403 })
        );
        assert_eq!(
            status_case("404 Not Found", &[]),
            Err(RemoteFetchError::HttpStatus { code: 404 })
        );
        assert_eq!(
            status_case("500 Internal Server Error", &[]),
            Err(RemoteFetchError::HttpStatus { code: 500 })
        );
    }

    // ---- connection failure ----------------------------------------------

    #[test]
    fn a_refused_connection_is_a_network_error() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let url = format!("http://127.0.0.1:{port}/a");
        assert_eq!(run(&url, 100, &loopback()), Err(RemoteFetchError::Network));
    }

    // ---- no URL in any log (decision 28) ----------------------------------

    #[test]
    fn the_non_test_source_has_no_logging_or_printing() {
        let src = include_str!("remote_fetch.rs");
        let end = src
            .find("#[cfg(test)]\nmod tests")
            .expect("test module marker");
        for (n, line) in src[..end].lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for banned in [
                "println!",
                "eprintln!",
                "log::",
                "tracing::",
                "dbg!",
                "print!",
                "eprint!",
            ] {
                assert!(!code.contains(banned), "line {}: {banned} in {code}", n + 1);
            }
        }
    }

    // ---- debug-only extra trusted root -------------------------------------

    /// The scanner behind the test below. Returns how many lines mention
    /// the extra-root machinery and the ones not under a gate.
    ///
    /// A use is under a gate when `#[cfg(debug_assertions)]` is in the
    /// attribute stack (the unbroken run of `#[...]` lines directly above)
    /// of the line itself or of one of its enclosing lines, an enclosing
    /// line being the nearest earlier line with smaller indentation, then
    /// the one enclosing that, and so on. A gate on a sibling statement,
    /// which has the same indentation and is not an ancestor, does not
    /// count, whether or not a blank line separates them.
    fn ungated_extra_root_uses(src: &str) -> (usize, Vec<String>) {
        let needles = [
            "HEW_EXTRA_CA_PEM",
            "add_root_certificate",
            "tls_certs_merge",
            "extra_root",
            "Certificate::",
        ];
        let lines: Vec<&str> = src.lines().collect();
        let indent = |l: &str| l.len() - l.trim_start().len();
        let attribute_gate = |at: usize| -> bool {
            let mut i = at;
            while i > 0 && lines[i - 1].trim_start().starts_with("#[") {
                i -= 1;
                if lines[i].trim() == "#[cfg(debug_assertions)]" {
                    return true;
                }
            }
            false
        };
        let mut seen = 0;
        let mut bad = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || !needles.iter().any(|k| code.contains(k)) {
                continue;
            }
            seen += 1;
            let mut gated = attribute_gate(n);
            let mut at = n;
            while !gated && at > 0 {
                let want = indent(lines[at]);
                let parent = (0..at).rev().find(|&i| {
                    let t = lines[i].trim_start();
                    !t.is_empty()
                        && !t.starts_with("//")
                        && !t.starts_with("#[")
                        && indent(lines[i]) < want
                });
                match parent {
                    Some(p) => {
                        gated = attribute_gate(p);
                        at = p;
                    }
                    None => break,
                }
            }
            if !gated {
                bad.push(format!(
                    "line {}: not under #[cfg(debug_assertions)]: {code}",
                    n + 1
                ));
            }
        }
        (seen, bad)
    }

    // The shipped binary must carry no way to add a trusted root: every
    // line that mentions the variable, `add_root_certificate`,
    // `tls_certs_merge`, `extra_root` or `Certificate::` must be under a
    // `#[cfg(debug_assertions)]` on the same item or statement, or on one
    // that encloses it (see `ungated_extra_root_uses`). A gate on a
    // neighboring statement does not count.
    #[test]
    fn every_use_of_the_extra_root_sits_under_cfg_debug_assertions() {
        let src = include_str!("remote_fetch.rs");
        let end = src
            .find("#[cfg(test)]\nmod tests")
            .expect("test module marker");
        let (seen, bad) = ungated_extra_root_uses(&src[..end]);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
        assert!(seen >= 3, "the scan found nothing to check ({seen})");
    }

    #[test]
    fn the_scanner_reports_a_use_gated_only_by_a_sibling_statement() {
        let ungated = "fn f() {\n    #[cfg(debug_assertions)]\n    let _a = 1;\n    let _b = std::env::var(\"HEW_EXTRA_CA_PEM\");\n}\n";
        let (seen, bad) = ungated_extra_root_uses(ungated);
        assert_eq!(seen, 1);
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(bad[0].starts_with("line 4:"), "{bad:?}");
        // The same use under its own gate, or inside a gated fn, passes.
        let own = "fn f() {\n    #[cfg(debug_assertions)]\n    let _b = std::env::var(\"HEW_EXTRA_CA_PEM\");\n}\n";
        assert_eq!(ungated_extra_root_uses(own), (1, vec![]));
        let enclosing =
            "#[cfg(debug_assertions)]\nfn f() {\n    let x = 1;\n    let _b = extra_root();\n}\n";
        assert_eq!(ungated_extra_root_uses(enclosing), (1, vec![]));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn extra_root_path_is_none_in_a_release_build_whatever_the_variable_says() {
        assert_eq!(extra_root_pem_path(false, Some("/tmp/ca.pem")), None);
        assert_eq!(extra_root_pem_path(false, None), None);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn extra_root_path_is_the_variable_in_a_debug_build() {
        assert_eq!(
            extra_root_pem_path(true, Some("/tmp/ca.pem")),
            Some(std::path::PathBuf::from("/tmp/ca.pem"))
        );
        assert_eq!(extra_root_pem_path(true, None), None);
        assert_eq!(extra_root_pem_path(true, Some("")), None);
    }

    // Mints a throwaway CA and a 30-day leaf for localhost/127.0.0.1 signed
    // by it, in memory; returns the leaf chain and key as DER plus the CA
    // PEM. Done at run time (not checked in) because macOS refuses a leaf
    // valid for more than 398 days even under an added root.
    #[cfg(debug_assertions)]
    fn mint_leaf() -> (
        rustls::pki_types::CertificateDer<'static>,
        rustls::pki_types::PrivateKeyDer<'static>,
        String,
    ) {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
            KeyPair, KeyUsagePurpose,
        };
        let now = ::time::OffsetDateTime::now_utc();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Hew test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca_params.not_before = now - ::time::Duration::days(1);
        ca_params.not_after = now + ::time::Duration::days(30);
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::new(ca_params, ca_key);

        let mut leaf_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        leaf_params.is_ca = IsCa::ExplicitNoCa;
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.not_before = now - ::time::Duration::days(1);
        leaf_params.not_after = now + ::time::Duration::days(30);
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
        (
            leaf.der().clone(),
            rustls::pki_types::PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
            ca.pem(),
        )
    }

    // A real TLS server on loopback: a freshly minted CA and leaf, the rustls the
    // crate already depends on, and plain blocking std sockets.
    #[cfg(debug_assertions)]
    // Returns the port and the path of the CA PEM to trust it with.
    fn serve_https(body: &'static [u8]) -> (u16, std::path::PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let (leaf, key, ca_pem) = mint_leaf();
        let ca_path = std::env::temp_dir().join(format!(
            "hew-remote-fetch-ca-{}-{}.pem",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::write(&ca_path, ca_pem).unwrap();
        let certs = vec![leaf];
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let config = config.clone();
                std::thread::spawn(move || {
                    let Ok(conn) = rustls::ServerConnection::new(config) else {
                        return;
                    };
                    let mut tls = rustls::StreamOwned::new(conn, stream);
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match tls.read(&mut byte) {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    let out = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = tls.write_all(out.as_bytes());
                    let _ = tls.write_all(body);
                    let _ = tls.flush();
                    tls.conn.send_close_notify();
                    let _ = tls.flush();
                });
            }
        });
        (port, ca_path)
    }

    #[cfg(debug_assertions)]
    #[test]
    fn https_loopback_fails_without_the_extra_root_and_works_with_it() {
        let (port, ca_path) = serve_https(b"secret-body");
        let url = format!("https://127.0.0.1:{port}/a");
        let without = FetchOpts {
            extra_root_pem: None,
            ..loopback()
        };
        assert_eq!(run(&url, 100, &without), Err(RemoteFetchError::Network));
        let with = FetchOpts {
            extra_root_pem: Some(ca_path.clone()),
            ..loopback()
        };
        assert_eq!(run(&url, 100, &with).unwrap(), b"secret-body");
        // The name form works too (the leaf also names `localhost`).
        let url = format!("https://localhost:{port}/a");
        assert_eq!(run(&url, 100, &with).unwrap(), b"secret-body");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn an_unreadable_or_invalid_extra_root_is_a_network_error() {
        let (port, _ca) = serve_https(b"x");
        let url = format!("https://127.0.0.1:{port}/a");
        let missing = FetchOpts {
            extra_root_pem: Some("/definitely/not/here.pem".into()),
            ..loopback()
        };
        assert_eq!(run(&url, 100, &missing), Err(RemoteFetchError::Network));
        let bad = std::env::temp_dir().join("hew-remote-fetch-bad-root.pem");
        std::fs::write(&bad, b"not a pem at all").unwrap();
        let invalid = FetchOpts {
            extra_root_pem: Some(bad.clone()),
            ..loopback()
        };
        assert_eq!(run(&url, 100, &invalid), Err(RemoteFetchError::Network));
        let _ = std::fs::remove_file(bad);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_extra_root_does_not_relax_the_https_requirement() {
        let s = serve(|_, c| respond(c, "200 OK", &[], b"x"));
        let (_port, ca_path) = serve_https(b"x");
        let opts = FetchOpts {
            allow_http_loopback: false,
            extra_root_pem: Some(ca_path),
            ..loopback()
        };
        assert_eq!(
            run(&s.url("/a"), 100, &opts),
            Err(RemoteFetchError::NotHttps)
        );
    }
}
