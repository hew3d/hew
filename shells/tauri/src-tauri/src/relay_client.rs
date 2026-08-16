//! The desktop's "Open on Phone" relay client and the server setting it is
//! bound to (docs/design/self-hosting-relay.md §3).
//!
//! Four relay requests — identity, PUT, HEAD peek, DELETE — live here as
//! Tauri commands instead of going through `tauri-plugin-http` from the
//! webview, for two reasons that turned up when self-hosting was designed:
//!
//!   1. The plugin's TLS roots are bundled Mozilla roots, so a homelab CA
//!      trusted in the OS keychain is invisible to it. `reqwest` with
//!      `rustls-platform-verifier` (the same stack the auto-updater already
//!      uses) trusts what the operating system trusts — which is what makes
//!      "trust your own CA on the phone AND the desktop" the whole story.
//!   2. A plugin scope wide enough for "any self-hosted origin" is a
//!      LAN-wide network primitive for anything that ever runs in the
//!      webview. These commands never take a URL: they read the origin from
//!      the Rust-held `ServerSetting` below, which is only ever written
//!      through `set_server_setting`. The webview can choose *cloud* or
//!      *self-hosted + origin*, and after that it can only say "put these
//!      bytes on the configured relay". No arbitrary-URL capability exists.
//!
//! The setting persists as JSON in the app config directory
//! (`server.json`), like `library.json`; `set_server_setting` validates,
//! persists, and broadcasts `settings-changed` with a `server` payload so
//! the separate Settings webview and the main window stay in step. The
//! upload key is stored in plain text there — the pane says so — and is
//! only ever sent to the configured self-hosted origin, never to the cloud.
//!
//! Errors are typed (`RelayError.kind`) so the dialog can say the specific
//! thing: unreachable, certificate not trusted, key rejected, relay full,
//! too large, not a Hew relay. Nothing here logs a token, a body, or the key.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::ipc::InvokeBody;
use tauri::{Emitter, Manager};

/// The public origin the desktop uses in cloud mode: the hosted web app,
/// which also serves the relay under `/relay/` (a Workers route on the same
/// hostname — workers/share-relay/README.md's deploy checklist).
pub const CLOUD_ORIGIN: &str = "https://app.hew3d.com";

/// Where the relay lives relative to whichever origin serves the web app.
/// The one convention the whole design rests on: phone and desktop derive
/// everything from the origin (docs/design/self-hosting-relay.md §2).
const RELAY_PATH: &str = "/relay";

/// The whole PUT (32 MiB over a slow uplink) — generous, but bounded.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(180);
/// Identity probe, HEAD peek, DELETE — small requests, short leash.
const SHORT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerMode {
    Cloud,
    SelfHosted,
}

/// The persisted setting. `origin` and `upload_key` are kept even in
/// `Cloud` mode (inert there) so switching back to self-hosted does not make
/// the user retype them; the effective server (`Effective`) is what the
/// requests actually use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSetting {
    pub mode: ServerMode,
    /// A canonical origin — scheme + host [+ port], no path/query/fragment/
    /// userinfo — once it has been through `set_server_setting`. Only
    /// validated when `mode` is `SelfHosted`; in cloud mode it is stored
    /// verbatim (trimmed) as a draft.
    pub origin: String,
    /// `""` = none. Sent as `Authorization: Bearer …` on PUT, self-hosted
    /// mode only.
    pub upload_key: String,
}

impl Default for ServerSetting {
    fn default() -> Self {
        Self {
            mode: ServerMode::Cloud,
            origin: CLOUD_ORIGIN.to_owned(),
            upload_key: String::new(),
        }
    }
}

/// What the requests are bound to right now, derived from the setting.
struct Effective {
    origin: String,
    upload_key: Option<String>,
}

impl ServerSetting {
    fn effective(&self) -> Effective {
        match self.mode {
            ServerMode::Cloud => Effective {
                origin: CLOUD_ORIGIN.to_owned(),
                upload_key: None,
            },
            ServerMode::SelfHosted => Effective {
                origin: self.origin.clone(),
                upload_key: if self.upload_key.is_empty() {
                    None
                } else {
                    Some(self.upload_key.clone())
                },
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Errors — typed for the dialog's specific wording
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RelayErrorKind {
    /// The origin the user typed does not pass validation.
    InvalidOrigin,
    /// The setting could not be written to disk.
    Io,
    /// The request never completed at the transport level: DNS, refused,
    /// timeout, offline.
    Unreachable,
    /// TLS handshake failed — almost always "certificate not trusted".
    Tls,
    /// The relay rejected the upload key (401).
    Unauthorized,
    /// `503 {"error":"relay full"}`.
    Full,
    /// 413.
    TooLarge,
    /// The identity route did not answer like a Hew relay.
    NotARelay,
    /// Any other unexpected HTTP status.
    Status,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayError {
    pub kind: RelayErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

impl RelayError {
    fn new(kind: RelayErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
        }
    }

    fn status(status: reqwest::StatusCode) -> Self {
        Self {
            kind: RelayErrorKind::Status,
            message: format!("unexpected status {}", status.as_u16()),
            status: Some(status.as_u16()),
        }
    }

    /// Classifies a transport-level failure. TLS problems surface inside
    /// reqwest's connect error chain; matching on the chain's text is the
    /// version-independent way to spot them without depending on the exact
    /// `rustls` error type reqwest happens to wrap.
    fn transport(err: &reqwest::Error) -> Self {
        // Only the SOURCE chain — reqwest's own Display embeds the request
        // URL, and a host name could contain any keyword matched below.
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
                RelayErrorKind::Tls,
                "the server's certificate isn't trusted by this computer",
            );
        }
        if err.is_timeout() {
            return Self::new(RelayErrorKind::Unreachable, "timed out");
        }
        Self::new(RelayErrorKind::Unreachable, "could not reach the server")
    }
}

// ---------------------------------------------------------------------------
// Origin validation
// ---------------------------------------------------------------------------

/// Validates a user-typed origin and returns its canonical form
/// (`scheme://host[:port]`, lowercase host, default port dropped). Rejects
/// anything with a path, query, fragment, or credentials — the value is
/// only ever used as the base for `/relay/…` requests and the QR's
/// `/#recv=…` URL, and a stray path would silently break both. `http` is
/// admitted for LAN-only setups without TLS (the pane warns that the
/// phone's in-app scanner then needs the camera-app fallback).
pub fn canonical_origin(input: &str) -> Result<String, RelayError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "enter the server's address, e.g. https://hew.example.org",
        ));
    }
    let url = url::Url::parse(trimmed).map_err(|_| {
        RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "not a valid address — expected something like https://hew.example.org",
        )
    })?;
    match url.scheme() {
        "https" | "http" => {}
        other => {
            return Err(RelayError::new(
                RelayErrorKind::InvalidOrigin,
                format!("unsupported scheme \"{other}\" — use https:// (or http:// on a LAN)"),
            ));
        }
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "the address needs a host name",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "the address must not contain a username or password",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "the address must be a bare origin — no ?query or #fragment",
        ));
    }
    if !(url.path().is_empty() || url.path() == "/") {
        return Err(RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "the address must be a bare origin — no path after the host",
        ));
    }
    // `Origin::ascii_serialization` is exactly scheme://host[:port] with the
    // default port elided and the host lowercased/IDNA-encoded.
    let origin = url.origin();
    if !origin.is_tuple() {
        return Err(RelayError::new(
            RelayErrorKind::InvalidOrigin,
            "not a valid address",
        ));
    }
    Ok(origin.ascii_serialization())
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn setting_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("server.json"))
}

/// The persisted setting, or the default when the file is absent or
/// unreadable (a corrupt file is not worth failing startup over — it
/// reads as "cloud", the safe default, and the next save overwrites it).
pub fn load_setting(app: &tauri::AppHandle) -> ServerSetting {
    let Some(path) = setting_path(app) else {
        return ServerSetting::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ServerSetting::default();
    };
    match serde_json::from_str::<ServerSetting>(&text) {
        Ok(setting) => setting,
        Err(err) => {
            eprintln!(
                "hew: ignoring unreadable {} ({err}); using the cloud server",
                path.display()
            );
            ServerSetting::default()
        }
    }
}

fn save_setting(app: &tauri::AppHandle, setting: &ServerSetting) -> Result<(), RelayError> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Err(RelayError::new(
            RelayErrorKind::Io,
            "could not resolve the app config directory",
        ));
    };
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| RelayError::new(RelayErrorKind::Io, e.to_string()))?;
    let path = config_dir.join("server.json");
    let text = serde_json::to_string_pretty(setting)
        .map_err(|e| RelayError::new(RelayErrorKind::Io, e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| RelayError::new(RelayErrorKind::Io, e.to_string()))
}

// ---------------------------------------------------------------------------
// Managed state
// ---------------------------------------------------------------------------

pub struct RelayState {
    setting: Mutex<ServerSetting>,
    client: reqwest::Client,
}

impl RelayState {
    /// Built once at setup with the persisted setting. The client uses the
    /// platform verifier (reqwest's `rustls-no-provider` + the crate-level
    /// `rustls`/`ring` pair, mirroring the updater's stack) so a CA the user
    /// trusts in the OS keychain is trusted here.
    pub fn new(setting: ServerSetting) -> Self {
        // reqwest's `rustls-no-provider` feature does NOT fall back to the
        // single provider compiled into rustls (the way rustls itself would);
        // it insists on a process-default `CryptoProvider` and panics in
        // `Client::build()` without one. The updater installs the same `ring`
        // provider lazily on first use — but this client is built at setup,
        // before any of that. Installing here is idempotent: `install_default`
        // errs if a provider is already in place, which is fine either way
        // (it is the same `ring` provider), hence the ignored result.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(UPLOAD_TIMEOUT)
            .user_agent(concat!("Hew/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Self {
            setting: Mutex::new(setting),
            client,
        }
    }

    fn setting(&self) -> ServerSetting {
        self.setting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn effective(&self) -> Effective {
        self.setting().effective()
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The identity route's answer, plus the origin it came from (so the pane
/// can show "Reachable: hew.example.org").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayIdentity {
    pub origin: String,
    pub service: String,
    pub contract: u32,
    pub max_bytes: u64,
    pub ttl_ms: u64,
    /// `"none"` or `"bearer"`.
    pub auth: String,
    /// Whether this desktop's configured key was accepted (only meaningful
    /// when `auth == "bearer"`): `Some(true)` = the probe PUT was let past
    /// the key check, `Some(false)` = 401, `None` = not probed (server has no
    /// key requirement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_accepted: Option<bool>,
}

#[derive(Deserialize)]
struct IdentityBody {
    service: String,
    contract: u32,
    #[serde(rename = "maxBytes")]
    max_bytes: u64,
    #[serde(rename = "ttlMs")]
    ttl_ms: u64,
    auth: String,
}

#[tauri::command]
pub fn get_server_setting(state: tauri::State<'_, RelayState>) -> ServerSetting {
    state.setting()
}

/// Validate, persist, install, and broadcast. Returns the canonical form
/// (the origin normalized) so the pane can echo it back into its field.
#[tauri::command]
pub fn set_server_setting(
    app: tauri::AppHandle,
    state: tauri::State<'_, RelayState>,
    setting: ServerSetting,
) -> Result<ServerSetting, RelayError> {
    let normalized = ServerSetting {
        mode: setting.mode,
        origin: match setting.mode {
            ServerMode::SelfHosted => canonical_origin(&setting.origin)?,
            ServerMode::Cloud => setting.origin.trim().to_owned(),
        },
        upload_key: setting.upload_key.trim().to_owned(),
    };
    save_setting(&app, &normalized)?;
    *state.setting.lock().unwrap_or_else(|e| e.into_inner()) = normalized.clone();
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({ "key": "server", "server": normalized }),
    );
    Ok(normalized)
}

/// *Test connection*: `GET <origin>/relay/` (trailing slash — a contract,
/// see the design's note on nginx `try_files`), then, when the server wants
/// a key and this desktop has one configured, an empty-body `PUT /drop`
/// probe: the contract checks the key BEFORE the size, so `400 empty body`
/// means "key accepted" and `401` means rejected — without ever storing
/// anything on the relay.
#[tauri::command]
pub async fn relay_identity(
    state: tauri::State<'_, RelayState>,
) -> Result<RelayIdentity, RelayError> {
    let effective = state.effective();
    let url = format!("{}{}/", effective.origin, RELAY_PATH);
    let response = state
        .client
        .get(&url)
        .timeout(SHORT_TIMEOUT)
        .send()
        .await
        .map_err(|e| RelayError::transport(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(if status == reqwest::StatusCode::NOT_FOUND {
            RelayError::new(
                RelayErrorKind::NotARelay,
                "reachable, but no relay is served at /relay/ on that server",
            )
        } else {
            RelayError::status(status)
        });
    }
    let text = response
        .text()
        .await
        .map_err(|e| RelayError::transport(&e))?;
    let body: IdentityBody = serde_json::from_str(&text).map_err(|_| {
        RelayError::new(RelayErrorKind::NotARelay, "reachable, but not a Hew relay")
    })?;
    if body.service != "hew-relay" {
        return Err(RelayError::new(
            RelayErrorKind::NotARelay,
            "reachable, but not a Hew relay",
        ));
    }

    let mut key_accepted = None;
    if body.auth == "bearer" {
        if let Some(key) = &effective.upload_key {
            let probe = state
                .client
                .put(format!("{}{}/drop", effective.origin, RELAY_PATH))
                .bearer_auth(key)
                .timeout(SHORT_TIMEOUT)
                .body(Vec::<u8>::new())
                .send()
                .await
                .map_err(|e| RelayError::transport(&e))?;
            key_accepted = Some(probe.status() != reqwest::StatusCode::UNAUTHORIZED);
        }
    }

    Ok(RelayIdentity {
        origin: effective.origin,
        service: body.service,
        contract: body.contract,
        max_bytes: body.max_bytes,
        ttl_ms: body.ttl_ms,
        auth: body.auth,
        key_accepted,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayPutResult {
    pub token: String,
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// `PUT <origin>/relay/drop` with the raw request body as ciphertext. The
/// webview passes the bytes as the invoke's raw body (`invoke('relay_put',
/// bytes)`), not a JSON array — a 32 MiB document must not be serialized as
/// 33 million numbers.
#[tauri::command]
pub async fn relay_put(
    state: tauri::State<'_, RelayState>,
    request: tauri::ipc::Request<'_>,
) -> Result<RelayPutResult, RelayError> {
    let bytes: Vec<u8> = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(_) => {
            return Err(RelayError::new(
                RelayErrorKind::Status,
                "relay_put expects a raw byte body",
            ));
        }
    };
    if bytes.is_empty() {
        return Err(RelayError::new(RelayErrorKind::Status, "nothing to upload"));
    }
    let effective = state.effective();
    let mut builder = state
        .client
        .put(format!("{}{}/drop", effective.origin, RELAY_PATH))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(bytes);
    if let Some(key) = &effective.upload_key {
        builder = builder.bearer_auth(key);
    }
    let response = builder
        .send()
        .await
        .map_err(|e| RelayError::transport(&e))?;
    let status = response.status();
    match status {
        reqwest::StatusCode::OK => {
            // `text()` + serde_json rather than `Response::json()`: the latter
            // is behind reqwest's `json` feature, which only the updater
            // enables — an updater-less (`--no-default-features`) build must
            // still compile.
            let text = response
                .text()
                .await
                .map_err(|e| RelayError::transport(&e))?;
            let body: TokenBody = serde_json::from_str(&text).map_err(|_| {
                RelayError::new(RelayErrorKind::NotARelay, "malformed relay answer")
            })?;
            Ok(RelayPutResult { token: body.token })
        }
        reqwest::StatusCode::UNAUTHORIZED => Err(RelayError::new(
            RelayErrorKind::Unauthorized,
            "the server rejected the upload key",
        )),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE => Err(RelayError::new(
            RelayErrorKind::TooLarge,
            "the document is too large for this relay",
        )),
        reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            // A bare 503 also comes from a proxy with the relay down; only
            // the relay's own JSON body means "full" (design §3).
            let text = response.text().await.unwrap_or_default();
            let full = serde_json::from_str::<ErrorBody>(&text)
                .map(|b| b.error == "relay full")
                .unwrap_or(false);
            Err(if full {
                RelayError::new(RelayErrorKind::Full, "the relay is full")
            } else {
                RelayError::status(status)
            })
        }
        other => Err(RelayError::status(other)),
    }
}

/// `HEAD <origin>/relay/drop/<token>` — `"present"` on 200, `"gone"` on
/// 404; anything else is an error the dialog treats as a missed tick.
#[tauri::command]
pub async fn relay_peek(
    state: tauri::State<'_, RelayState>,
    token: String,
) -> Result<String, RelayError> {
    let effective = state.effective();
    let response = state
        .client
        .head(format!("{}{}/drop/{}", effective.origin, RELAY_PATH, token))
        .timeout(SHORT_TIMEOUT)
        .send()
        .await
        .map_err(|e| RelayError::transport(&e))?;
    match response.status() {
        reqwest::StatusCode::OK => Ok("present".to_owned()),
        reqwest::StatusCode::NOT_FOUND => Ok("gone".to_owned()),
        other => Err(RelayError::status(other)),
    }
}

/// `DELETE <origin>/relay/drop/<token>` — best-effort invalidation on
/// dialog close; the caller ignores the outcome.
#[tauri::command]
pub async fn relay_delete(
    state: tauri::State<'_, RelayState>,
    token: String,
) -> Result<(), RelayError> {
    let effective = state.effective();
    state
        .client
        .delete(format!("{}{}/drop/{}", effective.origin, RELAY_PATH, token))
        .timeout(SHORT_TIMEOUT)
        .send()
        .await
        .map_err(|e| RelayError::transport(&e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_origin_accepts_and_normalizes() {
        assert_eq!(
            canonical_origin("https://hew.example.org").unwrap(),
            "https://hew.example.org"
        );
        assert_eq!(
            canonical_origin("  HTTPS://Hew.Example.org/  ").unwrap(),
            "https://hew.example.org"
        );
        assert_eq!(
            canonical_origin("https://hew.example.org:443").unwrap(),
            "https://hew.example.org"
        );
        assert_eq!(
            canonical_origin("http://192.168.1.20:8080").unwrap(),
            "http://192.168.1.20:8080"
        );
        assert_eq!(
            canonical_origin("http://hew.lan").unwrap(),
            "http://hew.lan"
        );
    }

    #[test]
    fn canonical_origin_rejects_the_rest() {
        for bad in [
            "",
            "hew.example.org",
            "ftp://hew.example.org",
            "https://",
            "https://user:pw@hew.example.org",
            "https://hew.example.org/relay",
            "https://hew.example.org/?x=1",
            "https://hew.example.org/#recv",
            "not a url",
        ] {
            let err = canonical_origin(bad).unwrap_err();
            assert!(
                matches!(err.kind, RelayErrorKind::InvalidOrigin),
                "{bad}: {:?}",
                err.kind
            );
        }
    }

    #[test]
    fn effective_server_follows_the_mode() {
        let s = ServerSetting {
            mode: ServerMode::Cloud,
            origin: "https://hew.example.org".into(),
            upload_key: "k".into(),
        };
        let e = s.effective();
        assert_eq!(e.origin, CLOUD_ORIGIN);
        assert!(e.upload_key.is_none()); // never sent to the cloud
        let s = ServerSetting {
            mode: ServerMode::SelfHosted,
            ..s
        };
        let e = s.effective();
        assert_eq!(e.origin, "https://hew.example.org");
        assert_eq!(e.upload_key.as_deref(), Some("k"));
        let s = ServerSetting {
            upload_key: String::new(),
            ..s
        };
        assert!(s.effective().upload_key.is_none());
    }

    #[test]
    fn setting_json_shape_is_camel_case_kebab_mode() {
        let s = ServerSetting::default();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(
            json,
            r#"{"mode":"cloud","origin":"https://app.hew3d.com","uploadKey":""}"#
        );
        let back: ServerSetting =
            serde_json::from_str(r#"{"mode":"self-hosted","origin":"https://x","uploadKey":"k"}"#)
                .unwrap();
        assert_eq!(back.mode, ServerMode::SelfHosted);
    }
}
