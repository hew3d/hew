//! The HTTP surface: routing, CORS, and the five handlers, mirroring
//! `workers/share-relay/src/handlers.ts` one for one — that file is the
//! contract's spec, and `workers/share-relay/contract/` is the black-box suite
//! both implementations must pass. Anything that changes an observable
//! status, header, or body here changes there too.
//!
//! One `fallback` handler classifies the request itself (`route`) rather
//! than an axum route table, so the dispatch reads exactly like the Worker's
//! `route()`: OPTIONS is a preflight on ANY path, the `/relay` prefix is
//! stripped exactly once, and everything unmatched is a CORS-wrapped 404.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ORIGIN,
    RETRY_AFTER, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use subtle::ConstantTimeEq;
use tower_http::timeout::TimeoutLayer;

use crate::store::{PutError, Store, is_valid_token};

/// The relay contract version reported by the identity route. Bumped only
/// for an incompatible change; the Worker reports the same number.
pub const CONTRACT_VERSION: u32 = 1;

/// Optional path prefix — self-hosting serves the relay from the web app's
/// origin under `/relay/`; nginx's `proxy_pass` with a URI part strips it
/// before it gets here, a proxy that forwards verbatim does not, and both
/// must work.
pub const RELAY_PREFIX: &str = "/relay";

/// `Retry-After` on a full-relay 503 (docs/design/self-hosting-relay.md §5).
const RETRY_AFTER_SECS: &str = "60";

/// Whole-request timeout (headers already read, body streaming in, handler
/// running). A screen-locked phone or a stalled upload can not hold a
/// connection open indefinitely — the property the old `tiny_http` LAN
/// server lacked.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// How long an early-rejected PUT's body is drained (discarded) before the
/// connection is dropped — see `reject_early`.
const LINGER_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AppState {
    pub store: Store,
    pub max_bytes: usize,
    pub allowed_origins: HashSet<String>,
    /// `None` = uploads are open (the public relay's posture). `Some` = PUT
    /// requires `Authorization: Bearer <key>`. Never logged.
    pub upload_key: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .fallback(handle)
        .with_state(state)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
}

// ---------------------------------------------------------------------------
// Routing (pure)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Route<'a> {
    Preflight,
    Identity,
    Put,
    Get(&'a str),
    Peek(&'a str),
    Delete(&'a str),
    NotFound,
}

impl Route<'_> {
    /// Generic name for the log line — never carries the token.
    fn kind(&self) -> &'static str {
        match self {
            Route::Preflight => "preflight",
            Route::Identity => "identity",
            Route::Put => "put",
            Route::Get(_) => "get",
            Route::Peek(_) => "peek",
            Route::Delete(_) => "delete",
            Route::NotFound => "not-found",
        }
    }
}

/// Strips one leading `RELAY_PREFIX` segment: `/relay` → `/`, `/relay/x` →
/// `/x`; anything else (including `/relayx`) is unchanged. Applied exactly
/// once — `/relay/relay/drop` is NOT `/drop`.
pub fn strip_relay_prefix(path: &str) -> &str {
    if path == RELAY_PREFIX {
        return "/";
    }
    match path.strip_prefix(RELAY_PREFIX) {
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    }
}

pub fn route<'a>(method: &Method, raw_path: &'a str) -> Route<'a> {
    if method == Method::OPTIONS {
        return Route::Preflight;
    }
    let path = strip_relay_prefix(raw_path);
    if path == "/" && method == Method::GET {
        return Route::Identity;
    }
    if path == "/drop" && method == Method::PUT {
        return Route::Put;
    }
    if let Some(token) = path.strip_prefix("/drop/") {
        if token.is_empty() || token.contains('/') {
            return Route::NotFound;
        }
        return match *method {
            Method::GET => Route::Get(token),
            Method::HEAD => Route::Peek(token),
            Method::DELETE => Route::Delete(token),
            _ => Route::NotFound,
        };
    }
    Route::NotFound
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn json(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("static response")
}

fn empty(status: StatusCode) -> Response {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("static response")
}

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

fn origin_allowed<'a>(origin: Option<&'a str>, allowed: &HashSet<String>) -> Option<&'a str> {
    origin.filter(|o| allowed.contains(*o))
}

fn preflight(origin: Option<&str>, allowed: &HashSet<String>) -> Response {
    let Some(origin) = origin_allowed(origin, allowed) else {
        return empty(StatusCode::FORBIDDEN);
    };
    let Ok(origin_value) = HeaderValue::from_str(origin) else {
        return empty(StatusCode::FORBIDDEN);
    };
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, origin_value)
        .header(
            ACCESS_CONTROL_ALLOW_METHODS,
            "PUT, GET, HEAD, DELETE, OPTIONS",
        )
        .header(ACCESS_CONTROL_ALLOW_HEADERS, "content-type, authorization")
        .header(ACCESS_CONTROL_MAX_AGE, "86400")
        .header(VARY, "origin")
        .body(Body::empty())
        .expect("static response")
}

fn with_cors(mut response: Response, origin: Option<&str>, allowed: &HashSet<String>) -> Response {
    if let Some(value) = origin_allowed(origin, allowed).and_then(|o| HeaderValue::from_str(o).ok())
    {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);
        response
            .headers_mut()
            .insert(VARY, HeaderValue::from_static("origin"));
    }
    response
}

// ---------------------------------------------------------------------------
// Upload key
// ---------------------------------------------------------------------------

/// `Authorization: Bearer <key>` against the configured key, constant-time.
/// Always true when no key is configured. Scheme is case-sensitive with
/// exactly one space, as the Worker's.
pub fn upload_authorized(headers: &HeaderMap, key: Option<&str>) -> bool {
    let Some(key) = key else { return true };
    let Some(header) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(presented) = header.strip_prefix("Bearer ") else {
        return false;
    };
    // `ct_eq` on slices of unequal length short-circuits on the length —
    // acceptable: the key's LENGTH is not the secret, its bytes are.
    presented.as_bytes().ct_eq(key.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn identity(state: &AppState) -> Response {
    let mut response = json(
        StatusCode::OK,
        serde_json::json!({
            "service": "hew-relay",
            "contract": CONTRACT_VERSION,
            "maxBytes": state.max_bytes,
            "ttlMs": state.store.ttl().as_millis() as u64,
            "auth": if state.upload_key.is_some() { "bearer" } else { "none" },
        }),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Answers a PUT that is refused BEFORE its body is read (401, header-only
/// 413, header-only 503) — and, nginx-`lingering_close`-style, keeps reading
/// and discarding the body in the background for a bounded budget (up to
/// `max_bytes` or `LINGER_TIMEOUT`) instead of dropping it on the spot.
///
/// Why: dropping an unread request body makes hyper close a socket that
/// still has unread data, which the kernel turns into a TCP RST rather than
/// a FIN — and a RST discards the peer's receive buffer, response included.
/// A client mid-upload then sees EPIPE/ECONNRESET and never the status:
/// the desktop would report "could not reach the server" for what was a
/// perfectly good `503 relay full`. Draining for a moment lets the client
/// read the answer and stop sending on its own; the budget keeps a hostile
/// multi-gigabyte body from turning this into a free bandwidth sink (it is
/// discarded, never buffered). Observed empirically with a raw-socket
/// client against this very server; the conformance suite's oversized-body
/// case is the regression check.
fn reject_early(response: Response, body: Body, drain_budget: usize) -> Response {
    tokio::spawn(async move {
        let mut body = body;
        let mut drained = 0usize;
        let deadline = tokio::time::Instant::now() + LINGER_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, body.frame()).await {
                Ok(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        drained += data.len();
                        if drained >= drain_budget {
                            break;
                        }
                    }
                }
                // Body finished, errored, or the deadline passed — done.
                _ => break,
            }
        }
    });
    response
}

fn relay_full() -> Response {
    let mut response = json(
        StatusCode::SERVICE_UNAVAILABLE,
        serde_json::json!({ "error": "relay full" }),
    );
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static(RETRY_AFTER_SECS));
    response
}

/// `PUT /drop`. Order matters and matches the Worker: auth first (an
/// unauthorized upload costs a header read, nothing more), then the declared
/// `Content-Length` against the per-drop cap (413) and the remaining total
/// cap (503) — both BEFORE the body is read — then the body itself, capped
/// while streaming so an oversized or lying upload never buffers past
/// `max_bytes`, then the same two checks against the real length.
async fn put(state: &AppState, req: Request) -> (Response, usize) {
    if !upload_authorized(req.headers(), state.upload_key.as_deref()) {
        let response = json(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "error": "unauthorized" }),
        );
        return (reject_early(response, req.into_body(), state.max_bytes), 0);
    }

    let declared = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if let Some(declared) = declared {
        if declared > state.max_bytes as u64 {
            let response = json(
                StatusCode::PAYLOAD_TOO_LARGE,
                serde_json::json!({ "error": "payload too large" }),
            );
            return (reject_early(response, req.into_body(), state.max_bytes), 0);
        }
        if declared > state.store.remaining() as u64 {
            return (
                reject_early(relay_full(), req.into_body(), state.max_bytes),
                0,
            );
        }
    }

    // Read the body frame by frame, capping while streaming: an oversized
    // (or lying-length / chunked) upload never buffers past `max_bytes`,
    // and on overflow the SAME body handle goes to `reject_early` so the
    // client gets its 413 instead of a reset — a chunked request has no
    // Content-Length for the fast path above to catch.
    let mut body = req.into_body();
    let mut buf: Vec<u8> =
        Vec::with_capacity(declared.map_or(0, |d| d as usize).min(state.max_bytes));
    loop {
        match body.frame().await {
            None => break,
            Some(Ok(frame)) => {
                if let Some(data) = frame.data_ref() {
                    if buf.len() + data.len() > state.max_bytes {
                        let response = json(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            serde_json::json!({ "error": "payload too large" }),
                        );
                        return (reject_early(response, body, state.max_bytes), 0);
                    }
                    buf.extend_from_slice(data);
                }
            }
            Some(Err(_)) => {
                return (
                    json(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "error": "bad body" }),
                    ),
                    0,
                );
            }
        }
    }
    let bytes = buf;
    if bytes.is_empty() {
        return (
            json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": "empty body" }),
            ),
            0,
        );
    }
    let size = bytes.len();
    match state.store.put(bytes) {
        Ok(token) => (
            json(StatusCode::OK, serde_json::json!({ "token": token })),
            size,
        ),
        Err(PutError::Full) => (relay_full(), 0),
    }
}

/// `GET /drop/<token>` — one-shot; unknown, taken, and expired all read as
/// the same 404 (no information leak either way).
fn get(state: &AppState, token: &str) -> (Response, usize) {
    if !is_valid_token(token) {
        return (
            json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": "invalid token" }),
            ),
            0,
        );
    }
    match state.store.take(token) {
        None => (
            json(
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "not found" }),
            ),
            0,
        ),
        Some(bytes) => {
            let size = bytes.len();
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(CACHE_CONTROL, "no-store")
                .body(Body::from(bytes))
                .expect("static response");
            (response, size)
        }
    }
}

/// `HEAD /drop/<token>` — non-consuming existence check, status only.
fn peek(state: &AppState, token: &str) -> Response {
    if !is_valid_token(token) {
        return empty(StatusCode::BAD_REQUEST);
    }
    if state.store.peek(token) {
        empty(StatusCode::OK)
    } else {
        empty(StatusCode::NOT_FOUND)
    }
}

/// `DELETE /drop/<token>` — 204 always, malformed token included.
fn delete(state: &AppState, token: &str) -> Response {
    if is_valid_token(token) {
        state.store.delete(token);
    }
    empty(StatusCode::NO_CONTENT)
}

async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let origin = req
        .headers()
        .get(ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let matched = route(&method, &path);
    let kind = matched.kind();

    let (response, size) = match matched {
        Route::Preflight => (preflight(origin.as_deref(), &state.allowed_origins), 0),
        Route::Identity => (identity(&state), 0),
        Route::Put => put(&state, req).await,
        Route::Get(token) => get(&state, token),
        Route::Peek(token) => (peek(&state, token), 0),
        Route::Delete(token) => (delete(&state, token), 0),
        Route::NotFound => (empty(StatusCode::NOT_FOUND), 0),
    };
    // One line per request: method, route kind, status, payload size. Never
    // the path (it carries the token), never a body, never the key.
    tracing::info!(
        method = %method,
        route = kind,
        status = response.status().as_u16(),
        bytes = size,
    );
    if matches!(matched, Route::Preflight) {
        return response;
    }
    with_cors(response, origin.as_deref(), &state.allowed_origins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn state(upload_key: Option<&str>) -> Arc<AppState> {
        Arc::new(AppState {
            store: Store::new(64, Duration::from_secs(60)),
            max_bytes: 16,
            allowed_origins: HashSet::from(["https://app.hew3d.com".to_owned()]),
            upload_key: upload_key.map(str::to_owned),
        })
    }

    async fn send(
        app: &Router,
        method: Method,
        path: &str,
        body: Vec<u8>,
        headers: &[(&str, &str)],
    ) -> Response {
        let mut req = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        app.clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn route_classification_matches_the_worker() {
        assert_eq!(route(&Method::OPTIONS, "/anything"), Route::Preflight);
        assert_eq!(route(&Method::GET, "/"), Route::Identity);
        assert_eq!(route(&Method::GET, "/relay"), Route::Identity);
        assert_eq!(route(&Method::GET, "/relay/"), Route::Identity);
        assert_eq!(route(&Method::HEAD, "/"), Route::NotFound);
        assert_eq!(route(&Method::PUT, "/drop"), Route::Put);
        assert_eq!(route(&Method::PUT, "/relay/drop"), Route::Put);
        assert_eq!(route(&Method::GET, "/drop/abc"), Route::Get("abc"));
        assert_eq!(route(&Method::HEAD, "/relay/drop/abc"), Route::Peek("abc"));
        assert_eq!(route(&Method::DELETE, "/drop/abc"), Route::Delete("abc"));
        assert_eq!(route(&Method::GET, "/drop"), Route::NotFound);
        assert_eq!(route(&Method::GET, "/drop/"), Route::NotFound);
        assert_eq!(route(&Method::GET, "/drop/a/b"), Route::NotFound);
        assert_eq!(route(&Method::PUT, "/drop/abc"), Route::NotFound);
        assert_eq!(route(&Method::PUT, "/relay/relay/drop"), Route::NotFound);
        assert_eq!(route(&Method::PUT, "/relayx/drop"), Route::NotFound);
        assert_eq!(route(&Method::GET, "/relayx"), Route::NotFound);
        assert_eq!(route(&Method::GET, "/nope"), Route::NotFound);
    }

    #[test]
    fn prefix_strip() {
        assert_eq!(strip_relay_prefix("/relay"), "/");
        assert_eq!(strip_relay_prefix("/relay/"), "/");
        assert_eq!(strip_relay_prefix("/relay/drop/t"), "/drop/t");
        assert_eq!(strip_relay_prefix("/drop"), "/drop");
        assert_eq!(strip_relay_prefix("/relayx"), "/relayx");
        assert_eq!(strip_relay_prefix("/"), "/");
    }

    #[test]
    fn bearer_check() {
        let mut h = HeaderMap::new();
        assert!(upload_authorized(&h, None));
        assert!(!upload_authorized(&h, Some("k")));
        h.insert(AUTHORIZATION, HeaderValue::from_static("Bearer k"));
        assert!(upload_authorized(&h, Some("k")));
        assert!(!upload_authorized(&h, Some("kk")));
        h.insert(AUTHORIZATION, HeaderValue::from_static("bearer k"));
        assert!(!upload_authorized(&h, Some("k")));
        h.insert(AUTHORIZATION, HeaderValue::from_static("Bearer  k"));
        assert!(!upload_authorized(&h, Some("k")));
        h.insert(AUTHORIZATION, HeaderValue::from_static("Basic k"));
        assert!(!upload_authorized(&h, Some("k")));
    }

    #[tokio::test]
    async fn put_get_round_trip_under_both_prefixes() {
        let app = router(state(None));
        for prefix in ["", "/relay"] {
            let res = send(
                &app,
                Method::PUT,
                &format!("{prefix}/drop"),
                vec![1, 2, 3],
                &[],
            )
            .await;
            assert_eq!(res.status(), StatusCode::OK);
            let token = body_json(res).await["token"].as_str().unwrap().to_owned();
            assert!(is_valid_token(&token));
            let other = if prefix.is_empty() { "/relay" } else { "" };
            let head = send(
                &app,
                Method::HEAD,
                &format!("{other}/drop/{token}"),
                vec![],
                &[],
            )
            .await;
            assert_eq!(head.status(), StatusCode::OK);
            let get = send(
                &app,
                Method::GET,
                &format!("{prefix}/drop/{token}"),
                vec![],
                &[],
            )
            .await;
            assert_eq!(get.status(), StatusCode::OK);
            assert_eq!(get.headers()[CONTENT_TYPE], "application/octet-stream");
            assert_eq!(get.headers()[CACHE_CONTROL], "no-store");
            let bytes = get.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(bytes.as_ref(), &[1, 2, 3]);
            let again = send(
                &app,
                Method::GET,
                &format!("{prefix}/drop/{token}"),
                vec![],
                &[],
            )
            .await;
            assert_eq!(again.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn size_checks_in_order() {
        let app = router(state(None));
        // Empty → 400.
        let res = send(&app, Method::PUT, "/drop", vec![], &[]).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({ "error": "empty body" })
        );
        // Exactly max (16) → 200.
        let res = send(&app, Method::PUT, "/drop", vec![0; 16], &[]).await;
        assert_eq!(res.status(), StatusCode::OK);
        // max + 1 → 413.
        let res = send(&app, Method::PUT, "/drop", vec![0; 17], &[]).await;
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({ "error": "payload too large" })
        );
        // Declared over max → 413 before reading.
        let res = send(
            &app,
            Method::PUT,
            "/drop",
            vec![1],
            &[("content-length", "1000")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn full_relay_is_503_with_retry_after() {
        // Cap 64, max 16 → four full drops fill it.
        let app = router(state(None));
        let mut tokens = vec![];
        for _ in 0..4 {
            let res = send(&app, Method::PUT, "/drop", vec![0; 16], &[]).await;
            assert_eq!(res.status(), StatusCode::OK);
            tokens.push(body_json(res).await["token"].as_str().unwrap().to_owned());
        }
        let res = send(&app, Method::PUT, "/drop", vec![0; 1], &[]).await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(res.headers()[RETRY_AFTER], RETRY_AFTER_SECS);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({ "error": "relay full" })
        );
        // Declared length that would overflow → 503 before reading.
        let res = send(
            &app,
            Method::PUT,
            "/drop",
            vec![1],
            &[("content-length", "8")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Free one, and it accepts again.
        let res = send(
            &app,
            Method::DELETE,
            &format!("/drop/{}", tokens[0]),
            vec![],
            &[],
        )
        .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let res = send(&app, Method::PUT, "/drop", vec![0; 16], &[]).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn upload_key_gates_put_only() {
        let app = router(state(Some("s3cret")));
        let res = send(&app, Method::GET, "/relay/", vec![], &[]).await;
        assert_eq!(body_json(res).await["auth"], "bearer");
        let res = send(&app, Method::PUT, "/drop", vec![1], &[]).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({ "error": "unauthorized" })
        );
        let res = send(
            &app,
            Method::PUT,
            "/drop",
            vec![1],
            &[("authorization", "Bearer wrong")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // 401 wins over 413.
        let res = send(
            &app,
            Method::PUT,
            "/drop",
            vec![1],
            &[("content-length", "1000")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let res = send(
            &app,
            Method::PUT,
            "/drop",
            vec![1],
            &[("authorization", "Bearer s3cret")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let token = body_json(res).await["token"].as_str().unwrap().to_owned();
        assert_eq!(
            send(&app, Method::HEAD, &format!("/drop/{token}"), vec![], &[])
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            send(&app, Method::GET, &format!("/drop/{token}"), vec![], &[])
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            send(&app, Method::DELETE, &format!("/drop/{token}"), vec![], &[])
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn identity_and_not_found() {
        let app = router(state(None));
        for path in ["/", "/relay", "/relay/"] {
            let res = send(&app, Method::GET, path, vec![], &[]).await;
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert_eq!(res.headers()[CACHE_CONTROL], "no-store");
            let body = body_json(res).await;
            assert_eq!(body["service"], "hew-relay");
            assert_eq!(body["contract"], CONTRACT_VERSION);
            assert_eq!(body["maxBytes"], 16);
            assert_eq!(body["ttlMs"], 60_000);
            assert_eq!(body["auth"], "none");
        }
        assert_eq!(
            send(&app, Method::HEAD, "/", vec![], &[]).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&app, Method::GET, "/nope", vec![], &[]).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&app, Method::GET, "/drop/short", vec![], &[])
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send(&app, Method::HEAD, "/drop/short", vec![], &[])
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send(&app, Method::DELETE, "/drop/short", vec![], &[])
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn cors_echo_and_preflight() {
        let app = router(state(None));
        let res = send(
            &app,
            Method::OPTIONS,
            "/whatever",
            vec![],
            &[("origin", "https://app.hew3d.com")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            res.headers()[ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://app.hew3d.com"
        );
        assert_eq!(
            res.headers()[ACCESS_CONTROL_ALLOW_METHODS],
            "PUT, GET, HEAD, DELETE, OPTIONS"
        );
        assert_eq!(
            res.headers()[ACCESS_CONTROL_ALLOW_HEADERS],
            "content-type, authorization"
        );
        assert_eq!(res.headers()[ACCESS_CONTROL_MAX_AGE], "86400");
        let res = send(
            &app,
            Method::OPTIONS,
            "/drop",
            vec![],
            &[("origin", "https://evil.example")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert!(res.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        let res = send(&app, Method::OPTIONS, "/drop", vec![], &[]).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let res = send(
            &app,
            Method::GET,
            "/nope",
            vec![],
            &[("origin", "https://app.hew3d.com")],
        )
        .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            res.headers()[ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://app.hew3d.com"
        );
        assert_eq!(res.headers()[VARY], "origin");
        let res = send(
            &app,
            Method::GET,
            "/nope",
            vec![],
            &[("origin", "https://evil.example")],
        )
        .await;
        assert!(res.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }
}
