//! The desktop's Help ▸ Report Bug submission client
//! (docs/design/report-bug.md §8 — the wire contract this must match
//! exactly).
//!
//! One Tauri command, `report_submit`, modeled on `relay_client.rs`'s
//! request plumbing: `reqwest` with the platform verifier stack (a homelab
//! CA trusted in the OS keychain is irrelevant here — the URL is fixed to
//! the Hew cloud — but the client is built the same way so there is only
//! one TLS story in this binary), typed errors, no new crates. Unlike the
//! relay, there is no per-user setting: every build mode sends to the same
//! `https://app.hew3d.com/report/` (design §5 — even a self-hosted desktop
//! reports bugs to Hew's developer, not the homelab). `HEW_REPORT_URL`
//! overrides that, but only in debug builds, so a release binary can never
//! be pointed at an attacker-controlled endpoint via the environment.
//!
//! The webview gzips the bundle, and the bytes ride the invoke as a raw body
//! (`tauri::ipc::Request`), the same reason `relay_put` does: an upload of
//! hundreds of megabytes must not be serialized as hundreds of millions of
//! numbers first.
//! This command then uploads them in pieces of `PIECE_BYTES` — start with the
//! first piece, each following piece in order, commit — because the intake
//! service gives each request only 10 ms of CPU (design §4). It emits
//! `report-progress` after each piece. Nothing here logs the body — not even
//! on error, since it can carry the user's own model and the files they
//! imported.

use std::time::Duration;

use serde::Serialize;
use tauri::ipc::InvokeBody;
use tauri::Emitter;

/// The intake service's fixed origin (docs/design/report-bug.md §8).
const REPORT_URL: &str = "https://app.hew3d.com/report/";

/// The upload piece size, one storage row on the intake service (§8).
const PIECE_BYTES: usize = 1_900_000;

/// One request carries at most one piece; two minutes covers that on a slow
/// uplink and still ends a stalled connection (design §8).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Errors — one kind per §8 status, plus the transport-level ones
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportErrorKind {
    /// 400 — not gzip, a head that doesn't parse, `format` ≠ 1, or
    /// description length out of range.
    Invalid,
    /// 413 — over the 1 GiB compressed cap.
    TooLarge,
    /// 429 — per-client rate limit.
    RateLimited,
    /// 507 — stored reports over the ceiling.
    Full,
    /// The request never completed at the transport level: DNS, refused,
    /// timeout, offline.
    Unreachable,
    /// TLS handshake failed.
    Tls,
    /// Any other unexpected HTTP status (403, 404, 409, 411, …) or a
    /// malformed response.
    Status,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportError {
    pub kind: ReportErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Seconds from a 429's `Retry-After`, when the service sent one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl ReportError {
    pub(crate) fn new(kind: ReportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
            retry_after: None,
        }
    }

    pub(crate) fn status(status: reqwest::StatusCode, message: impl Into<String>) -> Self {
        Self {
            kind: ReportErrorKind::Status,
            message: message.into(),
            status: Some(status.as_u16()),
            retry_after: None,
        }
    }

    pub(crate) fn malformed() -> Self {
        Self::new(
            ReportErrorKind::Status,
            "malformed response from the report service",
        )
    }

    /// Maps one of §8's JSON error bodies (`{"error": "...", ...}`) plus the
    /// HTTP status onto a typed kind. Falls back to `Status` for anything
    /// that doesn't match the contract — a proxy's own error page, an
    /// unversioned server — rather than guessing.
    pub(crate) fn from_status(status: reqwest::StatusCode, body: &str) -> Self {
        #[derive(serde::Deserialize)]
        struct ErrorBody {
            error: String,
        }
        let code = serde_json::from_str::<ErrorBody>(body)
            .map(|b| b.error)
            .unwrap_or_default();
        match (status, code.as_str()) {
            (reqwest::StatusCode::BAD_REQUEST, "invalid") => Self::new(
                ReportErrorKind::Invalid,
                "the report was rejected as invalid",
            ),
            (reqwest::StatusCode::PAYLOAD_TOO_LARGE, "too-large") => {
                Self::new(ReportErrorKind::TooLarge, "the report is too large to send")
            }
            (reqwest::StatusCode::TOO_MANY_REQUESTS, "rate-limited") => Self::new(
                ReportErrorKind::RateLimited,
                "too many reports sent recently",
            ),
            (reqwest::StatusCode::INSUFFICIENT_STORAGE, "full") => Self::new(
                ReportErrorKind::Full,
                "the report service is temporarily full",
            ),
            (reqwest::StatusCode::FORBIDDEN, "forbidden") => {
                Self::status(status, crate::follow::FORBIDDEN_MESSAGE)
            }
            _ if status.is_server_error() => Self::new(
                ReportErrorKind::Unreachable,
                "the report service is unavailable",
            ),
            _ => Self::status(status, format!("unexpected status {}", status.as_u16())),
        }
    }

    /// Classifies a transport-level failure — see `relay_client::RelayError::transport`,
    /// which this mirrors exactly (same reqwest/rustls stack, same reasoning
    /// for walking the source chain instead of matching reqwest's own
    /// Display, which embeds the request URL).
    pub(crate) fn transport(err: &reqwest::Error) -> Self {
        let mut text = String::new();
        let mut source = std::error::Error::source(err);
        while let Some(s) = source {
            text.push(' ');
            text.push_str(&s.to_string().to_ascii_lowercase());
            source = s.source();
        }
        if text.contains("certificate")
            || text.contains("unknownissuer")
            || text.contains("tls handshake")
            || text.contains("received fatal alert")
        {
            return Self::new(
                ReportErrorKind::Tls,
                "the server's certificate isn't trusted",
            );
        }
        if err.is_timeout() {
            return Self::new(ReportErrorKind::Unreachable, "timed out");
        }
        Self::new(
            ReportErrorKind::Unreachable,
            "could not reach the report service",
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSubmitResult {
    pub id: String,
    /// Whether a follow record was stored (the reporter asked to follow and
    /// the service minted a secret).
    pub following: bool,
}

/// How much of the compressed report has been uploaded, emitted as
/// `report-progress` after each piece.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportProgress {
    sent_bytes: u64,
    total_bytes: u64,
}

/// Resolves the URL to post to: the fixed cloud origin in every release
/// build, or `HEW_REPORT_URL` when set AND this is a debug build (design
/// §8's "Clients" subsection) — for testing against `wrangler dev` without
/// ever giving a release binary an environment-controlled endpoint. Always
/// ends in `/`, so piece and commit paths join onto it.
pub(crate) fn report_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("HEW_REPORT_URL") {
        if !url.is_empty() {
            return with_trailing_slash(url);
        }
    }
    REPORT_URL.to_owned()
}

fn with_trailing_slash(mut url: String) -> String {
    if !url.ends_with('/') {
        url.push('/');
    }
    url
}

/// The `[start, end)` byte ranges of the upload's pieces, in order; the
/// first carries the head the intake service validates.
fn piece_ranges(total: usize) -> Vec<(usize, usize)> {
    (0..total)
        .step_by(PIECE_BYTES)
        .map(|start| (start, (start + PIECE_BYTES).min(total)))
        .collect()
}

/// `HEW-` plus two groups of four Crockford base32 characters. The ID is
/// joined into request paths, so anything else is refused.
pub(crate) fn is_report_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 13
        && id.starts_with("HEW-")
        && bytes[8] == b'-'
        && bytes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= 4 && *i != 8)
            .all(|(_, b)| b.is_ascii_digit() || b.is_ascii_uppercase())
}

/// 32 random bytes, base64url without padding: 43 characters. The token
/// rides a header, so anything else is refused.
pub(crate) fn is_upload_token(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(crate) fn build_client() -> reqwest::Client {
    // Same idempotent provider install as relay_client.rs — see its doc
    // comment for why this can't just rely on rustls's own default.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(format!("Hew/{}", crate::version::VERSION))
        .build()
        .expect("reqwest client")
}

/// §8's retry rule: a network error or a 5xx is retried once. Every step
/// tolerates a repeat: the service accepts a retry of the last piece, and a
/// commit repeated with the upload's token answers 201 again.
async fn send_with_retry(
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, ReportError> {
    let first = build().send().await;
    match first {
        Ok(response) if !response.status().is_server_error() => Ok(response),
        _ => build().send().await.map_err(|e| ReportError::transport(&e)),
    }
}

pub(crate) async fn error_from(response: reqwest::Response) -> ReportError {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    ReportError::from_status(status, &text)
}

fn emit_progress(app: &tauri::AppHandle, sent: usize, total: usize) {
    // Progress is cosmetic; a failed emit never fails the upload.
    let _ = app.emit(
        "report-progress",
        ReportProgress {
            sent_bytes: sent as u64,
            total_bytes: total as u64,
        },
    );
}

/// What START answered, plus the finished upload's report ID.
pub(crate) struct Uploaded {
    pub id: String,
    /// The follow secret START minted, when `Hew-Follow: 1` was sent and
    /// the Worker understood it. Never leaves Rust.
    pub follow: Option<String>,
}

/// The chunked upload shared by `report_submit` and `follow_attach`: start
/// with the first piece (plus `start_headers`), each following piece in
/// order, then commit. Emits `report-progress` after every piece.
pub(crate) async fn upload_pieces(
    app: &tauri::AppHandle,
    bytes: &[u8],
    start_headers: &[(&'static str, String)],
) -> Result<Uploaded, ReportError> {
    if bytes.is_empty() {
        return Err(ReportError::new(
            ReportErrorKind::Invalid,
            "nothing to send",
        ));
    }

    let client = build_client();
    let base = report_url();
    let total = bytes.len();
    let pieces = piece_ranges(total);

    // Start: the first piece and the whole compressed length.
    let (first_start, first_end) = pieces[0];
    let response = send_with_retry(|| {
        let mut request = client
            .post(&base)
            .header(reqwest::header::CONTENT_TYPE, "application/gzip")
            .header("Hew-Upload-Length", total.to_string());
        for (name, value) in start_headers {
            request = request.header(*name, value);
        }
        request.body(bytes[first_start..first_end].to_vec())
    })
    .await?;
    if response.status() != reqwest::StatusCode::CREATED {
        return Err(error_from(response).await);
    }
    #[derive(serde::Deserialize)]
    struct StartBody {
        id: String,
        token: String,
        #[serde(default)]
        follow: Option<String>,
    }
    let text = response
        .text()
        .await
        .map_err(|e| ReportError::transport(&e))?;
    let started: StartBody = serde_json::from_str(&text).map_err(|_| ReportError::malformed())?;
    if !is_report_id(&started.id) || !is_upload_token(&started.token) {
        return Err(ReportError::malformed());
    }
    // A malformed secret is dropped rather than failing a report that is
    // otherwise going through; the caller then simply does not follow.
    let follow = started.follow.filter(|f| is_upload_token(f));
    emit_progress(app, first_end, total);

    for (index, &(start, end)) in pieces.iter().enumerate().skip(1) {
        let url = format!("{base}{}/{index}", started.id);
        let response = send_with_retry(|| {
            client
                .put(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/gzip")
                .header("Hew-Upload-Token", &started.token)
                .body(bytes[start..end].to_vec())
        })
        .await?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(error_from(response).await);
        }
        emit_progress(app, end, total);
    }

    let url = format!("{base}{}/commit", started.id);
    let response =
        send_with_retry(|| client.post(&url).header("Hew-Upload-Token", &started.token)).await?;
    if response.status() != reqwest::StatusCode::CREATED {
        return Err(error_from(response).await);
    }
    Ok(Uploaded {
        id: started.id,
        follow,
    })
}

/// The raw gzip body of a `tauri::ipc::Request`.
pub(crate) fn raw_body(request: &tauri::ipc::Request<'_>) -> Result<Vec<u8>, ReportError> {
    match request.body() {
        InvokeBody::Raw(bytes) => Ok(bytes.clone()),
        InvokeBody::Json(_) => Err(ReportError::new(
            ReportErrorKind::Invalid,
            "expected a raw byte body",
        )),
    }
}

/// Uploads the gzip-compressed report: start with the first piece, each
/// following piece in order, then commit. The webview passes the bytes as
/// the invoke's raw body (`invoke('report_submit', bytes)`), not a JSON
/// array, for the same reason `relay_put` does — see the module doc comment.
/// Options (follow-this-report, excerpt, file name) ride in the optional
/// `hew-options` request header (see `follow::parse_options_header`).
#[tauri::command]
pub async fn report_submit(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<ReportSubmitResult, ReportError> {
    let bytes = raw_body(&request)?;
    let options = crate::follow::parse_options_header(request.headers());
    let headers = crate::follow::start_headers(options.follow, None);
    let uploaded = upload_pieces(&app, &bytes, &headers).await?;
    let following = options.follow
        && uploaded.follow.is_some_and(|secret| {
            crate::follow::record_submitted(&app, &uploaded.id, secret, options)
        });
    Ok(ReportSubmitResult {
        id: uploaded.id,
        following,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_status_to_its_kind() {
        let cases: &[(reqwest::StatusCode, &str, ReportErrorKind)] = &[
            (
                reqwest::StatusCode::BAD_REQUEST,
                r#"{"error":"invalid","message":"x"}"#,
                ReportErrorKind::Invalid,
            ),
            (
                reqwest::StatusCode::PAYLOAD_TOO_LARGE,
                r#"{"error":"too-large","maxBytes":94371840}"#,
                ReportErrorKind::TooLarge,
            ),
            (
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":"rate-limited"}"#,
                ReportErrorKind::RateLimited,
            ),
            (
                reqwest::StatusCode::INSUFFICIENT_STORAGE,
                r#"{"error":"full"}"#,
                ReportErrorKind::Full,
            ),
            (
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error":"unavailable"}"#,
                ReportErrorKind::Unreachable,
            ),
            (
                reqwest::StatusCode::BAD_GATEWAY,
                r#"{"error":"unavailable"}"#,
                ReportErrorKind::Unreachable,
            ),
        ];
        for (status, body, expected) in cases {
            let err = ReportError::from_status(*status, body);
            assert!(
                matches!(&err.kind, k if std::mem::discriminant(k) == std::mem::discriminant(expected)),
                "{status}: got {:?}, expected {:?}",
                err.kind,
                expected
            );
        }
    }

    #[test]
    fn maps_piece_refusals_to_the_status_kind() {
        for (status, body) in [
            (reqwest::StatusCode::FORBIDDEN, r#"{"error":"forbidden"}"#),
            (reqwest::StatusCode::NOT_FOUND, r#"{"error":"not-found"}"#),
            (
                reqwest::StatusCode::CONFLICT,
                r#"{"error":"out-of-order","expected":3}"#,
            ),
            (
                reqwest::StatusCode::LENGTH_REQUIRED,
                r#"{"error":"length-required"}"#,
            ),
        ] {
            let err = ReportError::from_status(status, body);
            assert!(matches!(err.kind, ReportErrorKind::Status), "{status}");
            assert_eq!(err.status, Some(status.as_u16()));
        }
    }

    #[test]
    fn falls_back_to_status_kind_for_an_unrecognized_body() {
        let err = ReportError::from_status(reqwest::StatusCode::BAD_REQUEST, "not json at all");
        assert!(matches!(err.kind, ReportErrorKind::Status));
    }

    #[test]
    fn splits_an_upload_into_contiguous_pieces() {
        assert_eq!(piece_ranges(7), vec![(0, 7)]);
        assert_eq!(piece_ranges(PIECE_BYTES), vec![(0, PIECE_BYTES)]);
        assert_eq!(
            piece_ranges(PIECE_BYTES + 1),
            vec![(0, PIECE_BYTES), (PIECE_BYTES, PIECE_BYTES + 1)]
        );
        let cap = 90 * 1024 * 1024;
        let pieces = piece_ranges(cap);
        assert_eq!(pieces.len(), cap.div_ceil(PIECE_BYTES));
        assert!(pieces.windows(2).all(|w| w[0].1 == w[1].0));
        assert_eq!(pieces.last().map(|p| p.1), Some(cap));
        assert!(pieces.iter().all(|(s, e)| e - s <= PIECE_BYTES));
    }

    #[test]
    fn accepts_only_well_formed_ids_and_tokens() {
        assert!(is_report_id("HEW-7K3F-Q9XB"));
        assert!(!is_report_id("HEW-7K3F-Q9X"));
        assert!(!is_report_id("HEW-7K3F/Q9XB"));
        assert!(!is_report_id("HEW-../-Q9XB"));
        assert!(!is_report_id("hew-7k3f-q9xb"));

        assert!(is_upload_token(&"aZ09-_".repeat(8)[..43]));
        assert!(!is_upload_token(&"a".repeat(42)));
        assert!(!is_upload_token(&format!("{}\n", "a".repeat(42))));
    }

    #[test]
    fn report_url_defaults_to_the_cloud_origin_with_a_trailing_slash() {
        std::env::remove_var("HEW_REPORT_URL");
        assert_eq!(report_url(), REPORT_URL);
        assert_eq!(
            with_trailing_slash("http://127.0.0.1:8799/report".to_owned()),
            "http://127.0.0.1:8799/report/"
        );
    }
}
