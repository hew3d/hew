//! hew-relay — the self-hostable "Open on Phone" relay: a one-shot,
//! in-memory ciphertext dead-drop that speaks the same HTTP contract as the
//! Cloudflare Worker at `workers/share-relay` (that directory's README is the
//! contract; its `contract/` suite is what both implementations must pass).
//!
//! It only ever moves opaque ciphertext: the desktop encrypts before
//! uploading, the decryption key rides the QR's URL fragment and never
//! reaches any server, and a drop is forgotten the moment it is read once
//! (or after its TTL). It deliberately serves no static files — nginx (or
//! whatever fronts the web app) stays the canonical front and proxies
//! `/relay/` here (`shells/web/deploy/hew.d/relay.conf`); see
//! docs/SELF_HOSTING.md.
//!
//! Configuration is flags, each with an env twin (the systemd unit carries
//! the env; there is no config file):
//!
//!   --listen 127.0.0.1:8787            HEW_RELAY_LISTEN
//!   --allow-origin https://x (repeat)   HEW_RELAY_ALLOW_ORIGINS   (comma list; same-origin never needs it)
//!   --upload-key <key>                  HEW_RELAY_UPLOAD_KEY
//!   --max-bytes 33554432                HEW_RELAY_MAX_BYTES
//!   --max-total-bytes 268435456         HEW_RELAY_MAX_TOTAL_BYTES
//!   --ttl-secs 600                      HEW_RELAY_TTL_SECS
//!
//! Logs one line per request (method, route kind, status, payload size) —
//! never a token, a body, or the key. `RUST_LOG` filters as usual.

// Not a kernel crate: nothing here iterates a map into an output that must
// be bit-for-bit reproducible (the drop map is keyed lookups only, and the
// origin set is membership tests), so the workspace-wide determinism guard
// (clippy.toml) does not apply. Justified per its own comment.
#![allow(clippy::disallowed_types)]

mod http;
mod store;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;

use crate::http::AppState;
use crate::store::Store;

/// Per-drop cap, matching the Worker's `MAX_BYTES`.
const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Total in-memory cap across all live drops (docs/design §5).
const DEFAULT_MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// Matching the Worker's `TTL_MS`.
const DEFAULT_TTL_SECS: u64 = 600;
/// How long a connection may sit between the TCP accept (or the previous
/// response, on keep-alive) and a complete request head. This is the
/// slow-loris guard; the request-body/handler budget is `http.rs`'s
/// `REQUEST_TIMEOUT`, which only starts once the head has been parsed.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(15);
/// How long graceful shutdown waits for in-flight requests before exiting.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

#[derive(Parser, Debug)]
#[command(
    name = "hew-relay",
    version,
    about = "Self-hostable Open on Phone relay for Hew (one-shot ciphertext dead-drop)"
)]
struct Config {
    /// Address to listen on. Bind to loopback and let nginx proxy /relay/
    /// to it (the shipped deploy/hew.d/relay.conf does exactly that).
    #[arg(long, env = "HEW_RELAY_LISTEN", default_value = "127.0.0.1:8787")]
    listen: SocketAddr,

    /// Browser origin allowed to read responses cross-origin (repeatable;
    /// the env twin is a comma-separated list). A phone app served from the
    /// SAME origin as the relay never needs this, and the desktop's native
    /// client sends no Origin at all — set it only when app and relay live
    /// on different origins.
    #[arg(
        long = "allow-origin",
        env = "HEW_RELAY_ALLOW_ORIGINS",
        value_delimiter = ','
    )]
    allow_origins: Vec<String>,

    /// When set, PUT /drop requires `Authorization: Bearer <key>`. Empty
    /// means none (uploads open — the public relay's posture). Never logged.
    #[arg(long, env = "HEW_RELAY_UPLOAD_KEY", hide_env_values = true)]
    upload_key: Option<String>,

    /// Per-drop size cap in bytes (413 above it).
    #[arg(long, env = "HEW_RELAY_MAX_BYTES", default_value_t = DEFAULT_MAX_BYTES)]
    max_bytes: usize,

    /// Total memory cap across all live drops, in bytes. A PUT that would
    /// exceed it is refused with 503 {"error":"relay full"} + Retry-After —
    /// fail closed, never swap.
    #[arg(long, env = "HEW_RELAY_MAX_TOTAL_BYTES", default_value_t = DEFAULT_MAX_TOTAL_BYTES)]
    max_total_bytes: usize,

    /// Seconds an unread drop lives before it is forgotten.
    #[arg(long, env = "HEW_RELAY_TTL_SECS", default_value_t = DEFAULT_TTL_SECS)]
    ttl_secs: u64,
}

impl Config {
    fn validate(&self) -> Result<(), String> {
        if self.max_bytes == 0 {
            return Err("--max-bytes must be at least 1".into());
        }
        if self.max_total_bytes < self.max_bytes {
            return Err(format!(
                "--max-total-bytes ({}) must be at least --max-bytes ({}); otherwise no maximum-size drop could ever be stored",
                self.max_total_bytes, self.max_bytes
            ));
        }
        if self.ttl_secs == 0 {
            return Err("--ttl-secs must be at least 1".into());
        }
        for origin in &self.allow_origins {
            let trimmed = origin.trim();
            if trimmed.is_empty() {
                return Err("--allow-origin: empty origin".into());
            }
            if trimmed.ends_with('/')
                || !(trimmed.starts_with("https://")
                    || trimmed.starts_with("http://")
                    || trimmed.starts_with("tauri://"))
            {
                return Err(format!(
                    "--allow-origin '{trimmed}': expected a bare origin like https://hew.example.org (scheme + host, no trailing slash or path)"
                ));
            }
        }
        Ok(())
    }
}

/// How often the background sweeper drops expired entries: a quarter of the
/// TTL, clamped to [100 ms, 5 s], so a `--ttl-secs 2` test server sweeps
/// promptly and a production one is not spinning.
fn sweep_interval(ttl: Duration) -> Duration {
    (ttl / 4).clamp(Duration::from_millis(100), Duration::from_secs(5))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let config = Config::parse();
    if let Err(msg) = config.validate() {
        eprintln!("hew-relay: {msg}");
        std::process::exit(2);
    }

    let ttl = Duration::from_secs(config.ttl_secs);
    let upload_key = config
        .upload_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_owned);
    let allowed_origins: HashSet<String> = config
        .allow_origins
        .iter()
        .map(|o| o.trim().to_owned())
        .collect();

    let state = Arc::new(AppState {
        store: Store::new(config.max_total_bytes, ttl),
        max_bytes: config.max_bytes,
        allowed_origins,
        upload_key,
    });

    // Sweeper: TTL expiry is also enforced on every read, but an unread
    // drop must not hold memory (and the total cap) until someone asks.
    let sweeper_state = Arc::clone(&state);
    let sweeper = tokio::spawn(async move {
        let mut interval = tokio::time::interval(sweep_interval(ttl));
        loop {
            interval.tick().await;
            sweeper_state.store.sweep();
        }
    });

    let listener = match tokio::net::TcpListener::bind(config.listen).await {
        Ok(l) => l,
        Err(err) => {
            eprintln!("hew-relay: cannot listen on {}: {err}", config.listen);
            std::process::exit(1);
        }
    };
    tracing::info!(
        listen = %config.listen,
        max_bytes = state.max_bytes,
        max_total_bytes = config.max_total_bytes,
        ttl_secs = config.ttl_secs,
        auth = if state.upload_key.is_some() { "bearer" } else { "none" },
        allowed_origins = state.allowed_origins.len(),
        "hew-relay ready"
    );

    // Accept loop over hyper-util rather than `axum::serve`: the latter
    // installs no timer on the connection builder, which silently disables
    // hyper's `header_read_timeout` — so a client could hold a socket open
    // forever having sent half a request line. Each connection gets a timer
    // and the header timeout here; the router's own TimeoutLayer bounds the
    // rest of the request once the head is in.
    let router = http::router(Arc::clone(&state));
    let graceful = GracefulShutdown::new();
    let mut shutdown = pin!(shutdown_signal());
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _peer) = match accepted {
                    Ok(pair) => pair,
                    Err(err) => {
                        // EMFILE and friends: back off briefly rather than spin.
                        tracing::warn!(error = %err, "accept failed");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let service = TowerToHyperService::new(router.clone());
                let mut builder = ConnBuilder::new(TokioExecutor::new());
                builder
                    .http1()
                    .timer(TokioTimer::new())
                    .header_read_timeout(HEADER_READ_TIMEOUT);
                builder.http2().timer(TokioTimer::new());
                let conn = graceful.watch(
                    builder
                        .serve_connection(TokioIo::new(stream), service)
                        .into_owned(),
                );
                tokio::spawn(async move {
                    if let Err(err) = conn.await {
                        // Client went away mid-request, malformed HTTP, the
                        // header timeout — none of it is actionable here.
                        tracing::debug!(error = %err, "connection ended with error");
                    }
                });
            }
            _ = &mut shutdown => break,
        }
    }
    sweeper.abort();
    tracing::info!("hew-relay shutting down; draining in-flight requests");
    tokio::select! {
        _ = graceful.shutdown() => {}
        _ = tokio::time::sleep(SHUTDOWN_GRACE) => {
            tracing::warn!("shutdown grace period elapsed with requests still in flight");
        }
    }
    tracing::info!(live_drops = state.store.len(), "hew-relay stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_worker_contract() {
        let config = Config::parse_from(["hew-relay"]);
        assert_eq!(config.max_bytes, 32 * 1024 * 1024);
        assert_eq!(config.ttl_secs, 600);
        assert_eq!(
            config.listen,
            "127.0.0.1:8787".parse::<SocketAddr>().unwrap()
        );
        assert!(config.allow_origins.is_empty());
        assert!(config.upload_key.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn allow_origin_repeats_and_splits() {
        let config = Config::parse_from([
            "hew-relay",
            "--allow-origin",
            "https://a.example",
            "--allow-origin",
            "https://b.example,https://c.example",
        ]);
        assert_eq!(
            config.allow_origins,
            [
                "https://a.example",
                "https://b.example",
                "https://c.example"
            ]
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_nonsense() {
        assert!(
            Config::parse_from(["hew-relay", "--max-bytes", "0"])
                .validate()
                .is_err()
        );
        assert!(
            Config::parse_from(["hew-relay", "--ttl-secs", "0"])
                .validate()
                .is_err()
        );
        assert!(
            Config::parse_from(["hew-relay", "--max-bytes", "10", "--max-total-bytes", "5"])
                .validate()
                .is_err()
        );
        assert!(
            Config::parse_from(["hew-relay", "--allow-origin", "hew.example.org"])
                .validate()
                .is_err()
        );
        assert!(
            Config::parse_from(["hew-relay", "--allow-origin", "https://hew.example.org/"])
                .validate()
                .is_err()
        );
        assert!(
            Config::parse_from(["hew-relay", "--allow-origin", "https://hew.example.org"])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn sweep_interval_clamps() {
        assert_eq!(
            sweep_interval(Duration::from_secs(600)),
            Duration::from_secs(5)
        );
        assert_eq!(
            sweep_interval(Duration::from_secs(2)),
            Duration::from_millis(500)
        );
        assert_eq!(
            sweep_interval(Duration::from_millis(100)),
            Duration::from_millis(100)
        );
    }
}
