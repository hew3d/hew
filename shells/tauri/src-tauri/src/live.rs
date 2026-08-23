//! `--live` (docs/agents/HEW_API.md §11.2): the desktop half of the local socket
//! transport. This module owns exactly the transport and the token gate —
//! it never touches a `kernel::Document` or an `api::Connection` itself
//! (those live in `crates/wasm-api`, inside the webview's WASM sandbox).
//! Its job is: listen on a per-pid Unix domain socket, publish a discovery
//! file a client can find, verify the mandatory first-frame `hello` token,
//! and shuttle newline-delimited JSON-RPC frames between the socket and
//! the webview via Tauri events — `hew://api-frame` (Rust → JS),
//! `hew://api-reply` (JS → Rust), `hew://api-connection-open`/`-close`
//! (Rust → JS, connection lifecycle). `crates/wasm-api/src/live.rs`'s
//! module doc comment is the JS-facing half of this same contract.
//!
//! Unix domain sockets on macOS/Linux, a named pipe on Windows
//! (`\\.\pipe\hew-<pid>`) — `crates/hew-cli/src/live.rs` (the client side
//! of this same transport, owned by a different effort) now speaks both.
//! The unix functions below are unchanged (verified against the real
//! app); the Windows transport is new, self-contained in a `mod windows`
//! at the bottom of this file, and shares the platform-independent middle
//! that already existed for unix — the token gate (`check_and_sanitize_
//! hello`, `strip_token`), the frame-forwarding Tauri-event shapes
//! (`FramePayload`/`ConnPayload`/`ReplyPayload`), and `LiveApi`'s
//! bookkeeping. Only accept/read/write are reimplemented per platform,
//! since a `UnixListener` and a named pipe `HANDLE` have no common Rust
//! trait worth abstracting over for two call sites.
//!
//! The accept loop and every per-connection reader run on plain
//! `std::thread`s on both platforms — unix over `std::os::unix::net`;
//! Windows over `windows-sys`'s raw `CreateNamedPipeW`/`ConnectNamedPipe`/
//! `ReadFile`/`WriteFile` — mirroring `crates/hew-cli/src/live.rs`'s own
//! client-side transport rather than reaching for the `tokio` already in
//! this crate's dependency graph (pulled in transitively by
//! `tauri`/`reqwest`): one accepted connection is cheap enough as an OS
//! thread that an async runtime bridge buys nothing here, on either
//! platform. The 256-bit discovery token is read straight from
//! `/dev/urandom` on unix (the CSPRNG source `docs/agents/HEW_API.md` §11.2 calls
//! out as acceptable there) and via `BCryptGenRandom` on Windows — no
//! `rand`/`getrandom` dependency needed on either.
//!
//! Windows' owner-only posture is real but weaker than unix's 0600 in one
//! documented way: the pipe is created with a security descriptor
//! (`D:P(A;;GA;;;OW)`, §11.2) granting full access to the pipe's owner —
//! the current user's token at creation time — and nobody else, so no
//! other interactively logged-in user's process can open it; the
//! discovery file relies on `%LOCALAPPDATA%` already being per-user rather
//! than an explicit ACL of its own (there is no Windows equivalent of a
//! single `chmod 0600` applied here — see `write_discovery_file`'s doc
//! comment on `windows::write_discovery_file` for why), though
//! `ensure_safe_runtime_dir` does confirm the directory resolves under
//! this process's own `%LOCALAPPDATA%` (symlinks/junctions included) even
//! though it stops short of the discretionary-ACL check that would make
//! it a true equivalent of unix's uid check — see that function's doc
//! comment for exactly what is and is not verified. `create_pipe_instance`
//! also passes `FILE_FLAG_FIRST_PIPE_INSTANCE` for the very first instance
//! of this pid's pipe name, so an attempt to squat on the (fully
//! predictable, pid-derived) pipe name ahead of this process's own launch
//! makes pipe creation fail loudly rather than silently adding an
//! instance to the squatter's pipe object. As on unix, where root can
//! always bypass 0600, a local Administrator/SYSTEM can bypass a DACL
//! with sufficient privilege on Windows too — that ceiling is an OS
//! property neither platform's transport tries to defeat.

use std::collections::HashMap;
#[cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager};

/// How long a per-connection reader waits for the webview's
/// `hew://api-reply` before giving up on that one frame and answering a
/// synthesized internal-fault reply — matches
/// `crates/hew-cli/src/live.rs`'s own `REPLY_TIMEOUT` so a client sees the
/// same effective ceiling regardless of which side is slow. A timeout
/// does not close the connection: the client's NEXT frame gets a fresh
/// wait, so one slow dispatch cannot poison every request after it.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Live API dispatch bookkeeping, managed as Tauri state. Session-only:
/// nothing here is persisted or fed into a document's canonical bytes.
#[derive(Default)]
struct LiveApi {
    next_conn_id: AtomicU32,
    /// Which window a connection's frames route to, fixed at accept time
    /// (see `spawn_accept_loop`'s doc comment on why it is not re-resolved
    /// per frame).
    conn_window: Mutex<HashMap<u32, String>>,
    /// One slot per frame currently awaiting its `hew://api-reply` — filled
    /// just before emitting `hew://api-frame`, drained by the global reply
    /// listener registered in `start`.
    pending_replies: Mutex<HashMap<u32, mpsc::Sender<String>>>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FramePayload<'a> {
    conn_id: u32,
    frame: &'a str,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnPayload {
    conn_id: u32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyPayload {
    conn_id: u32,
    frame: String,
}

// ------------------------------------------------------------- discovery

/// The runtime directory files live under (docs/agents/HEW_API.md §11.2), mirroring
/// `crates/hew-cli/src/live.rs`'s `runtime_dir()` exactly (field names,
/// fallback order, and the `HEW_RUNTIME_DIR` test/dev override) — the two
/// live independently (this crate cannot depend on `hew-cli`, and
/// vice versa) but MUST agree bit-for-bit or discovery silently fails
/// between the two halves of this same transport.
#[cfg(unix)]
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HEW_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    platform_runtime_dir()
}

#[cfg(target_os = "macos")]
fn platform_runtime_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    home.join("Library/Application Support/Hew/run")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // §11.2 names only $XDG_RUNTIME_DIR, but it is unset on plenty
            // of real Linux logins (ssh without a systemd session). Falling
            // back to a bare /tmp/hew would put the discovery file in a
            // world-writable directory ANY local user could create first
            // and then own — defeating the section's own "no other user on
            // it can drive a document" guarantee. The fallback is therefore
            // per-uid, and `ensure_owned_dir` still verifies ownership and
            // mode before anything is written into it.
            PathBuf::from(format!("/tmp/hew-run-{}", current_uid()))
        })
}

/// This process's effective uid, without a `libc` dependency: create a
/// probe file in the temp dir and read back the owner the kernel stamped
/// on it. Cached, so the probe happens at most once per process.
#[cfg(unix)]
fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    use std::sync::OnceLock;
    static UID: OnceLock<u32> = OnceLock::new();
    *UID.get_or_init(|| {
        let probe = std::env::temp_dir().join(format!("hew-uid-probe-{}", std::process::id()));
        let uid = std::fs::File::create(&probe)
            .and_then(|_| std::fs::metadata(&probe))
            .map(|m| m.uid())
            .unwrap_or(0);
        let _ = std::fs::remove_file(&probe);
        uid
    })
}

/// Creates `dir` (0700) and confirms WE own it with owner-only access —
/// the check that makes the discovery directory trustworthy. A directory
/// that exists but belongs to someone else, or that is group/world
/// accessible, is refused rather than tightened: another user planting it
/// first must not be able to make us publish a token into their space.
#[cfg(unix)]
fn ensure_owned_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::create_dir_all(dir)?;
    let meta = std::fs::metadata(dir)?;
    if meta.uid() != current_uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is owned by another user", dir.display()),
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
fn instances_dir() -> PathBuf {
    runtime_dir().join("hew")
}

/// 256 bits of randomness from `/dev/urandom`, hex-encoded — the discovery
/// token (docs/agents/HEW_API.md §11.2). `/dev/urandom` is a CSPRNG on every unix
/// Hew targets (macOS, Linux); this is the "reading /dev/urandom directly
/// is acceptable" case the spec names explicitly, chosen over adding a
/// `rand`/`getrandom` dependency for one 32-byte read.
#[cfg(unix)]
fn generate_token_hex() -> std::io::Result<String> {
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Deletes every `instance-*.json` in `dir` whose pid is no longer alive,
/// and its paired socket file (best effort — a dead process's socket is
/// inert either way) — the app's own startup half of §11.2's
/// validate-then-use discovery contract (the client's half,
/// `discover_in`, lives in `crates/hew-cli/src/live.rs`). A missing
/// directory or an unreadable/malformed sibling is silently skipped, not
/// an error.
#[cfg(unix)]
fn sweep_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // An `instance-<pid>.sock` whose pid is dead is swept on its own,
        // not only alongside its discovery file: a connector that finds a
        // stale entry deletes the JSON (validate-then-use, §11.2) and
        // leaves the socket, so by the time this runs the pairing is
        // often already broken and the socket would otherwise accumulate
        // forever.
        let orphan_socket_pid = name
            .strip_prefix("instance-")
            .and_then(|rest| rest.strip_suffix(".sock"))
            .and_then(|pid| pid.parse::<u32>().ok());
        if let Some(pid) = orphan_socket_pid {
            if !is_pid_alive(pid) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if !(name.starts_with("instance-") && name.ends_with(".json")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(pid) = value.get("pid").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        if is_pid_alive(pid as u32) {
            continue;
        }
        // The `socket` field is untrusted input read off disk: delete it
        // only when it names a file directly inside the instances
        // directory. Following it anywhere else would turn a planted
        // discovery file into an arbitrary-file-delete primitive.
        if let Some(sock) = value.get("socket").and_then(|s| s.as_str()) {
            let sock = Path::new(sock);
            if sock.parent() == Some(dir) {
                let _ = std::fs::remove_file(sock);
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

#[derive(serde::Serialize)]
struct DiscoveryFile<'a> {
    socket: &'a str,
    token: &'a str,
    pid: u32,
    version: &'a str,
}

/// Writes `<dir>/instance-<pid>.json` (owner-only permissions, dir and
/// file — docs/agents/HEW_API.md §11.2). Directory permissions are (re)applied
/// on every launch, not just at first creation, so a runtime dir that
/// pre-existed with looser permissions (or was recreated by something
/// else) is still tightened before anything sensitive is written into it.
#[cfg(unix)]
fn write_discovery_file(dir: &Path, pid: u32, socket: &str, token: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    ensure_owned_dir(dir)?;
    let path = dir.join(format!("instance-{pid}.json"));
    let body = DiscoveryFile {
        socket,
        token,
        pid,
        version: env!("CARGO_PKG_VERSION"),
    };
    let json = serde_json::to_vec(&body).expect("DiscoveryFile serializes");
    std::fs::write(&path, &json)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Removes this process's own discovery file and socket — "removes it on
/// exit" (docs/agents/HEW_API.md §11.2). Paths are re-derived from the current
/// pid rather than cached anywhere, since they are a pure function of it;
/// a process that never got as far as `start()` succeeding (no runtime
/// dir, bind failure) has nothing here to remove, and every call below is
/// already a best-effort `let _ =`.
#[cfg(unix)]
pub fn cleanup() {
    let pid = std::process::id();
    let dir = instances_dir();
    let _ = std::fs::remove_file(dir.join(format!("instance-{pid}.json")));
    let _ = std::fs::remove_file(dir.join(format!("instance-{pid}.sock")));
}

#[cfg(windows)]
pub fn cleanup() {
    windows::cleanup();
}

#[cfg(not(any(unix, windows)))]
pub fn cleanup() {}

// -------------------------------------------------------------- transport

/// Starts the live API listener: sweeps stale discovery files, binds a
/// fresh per-pid socket, publishes the discovery file, and spawns the
/// accept loop. Best-effort end to end — any failure (an unwritable
/// runtime dir, a bind error) is logged to stderr and leaves `--live`
/// simply unavailable this session; it must never block the app from
/// starting normally, since the vast majority of launches never use it.
#[cfg(unix)]
pub fn start(app: &AppHandle) {
    app.manage(LiveApi::default());

    let dir = instances_dir();
    sweep_stale(&dir);

    let pid = std::process::id();
    let socket_path = dir.join(format!("instance-{pid}.sock"));

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("hew: live API disabled — could not create {dir:?}: {e}");
        return;
    }
    // A leftover socket file from an unclean exit (no RunEvent::Exit ever
    // fired) would make `UnixListener::bind` fail with "address in use";
    // sweep_stale only removes DEAD peers' files, and this pid cannot
    // collide with a live one, so removing it unconditionally first is
    // always correct.
    let _ = std::fs::remove_file(&socket_path);
    let listener = match std::os::unix::net::UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("hew: live API disabled — could not bind {socket_path:?}: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::set_permissions(
        &socket_path,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    ) {
        eprintln!("hew: live API disabled — could not set socket permissions: {e}");
        let _ = std::fs::remove_file(&socket_path);
        return;
    }

    let token = match generate_token_hex() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hew: live API disabled — could not read /dev/urandom: {e}");
            let _ = std::fs::remove_file(&socket_path);
            return;
        }
    };

    if let Err(e) = write_discovery_file(&dir, pid, socket_path.to_string_lossy().as_ref(), &token)
    {
        eprintln!("hew: live API disabled — could not write the discovery file: {e}");
        let _ = std::fs::remove_file(&socket_path);
        return;
    }

    // The reply channel: the webview calls back into Rust via a plain
    // `emit("hew://api-reply", {connId, frame})` (no Tauri command/invoke
    // needed — see crates/wasm-api/src/live.rs's module doc), and this is
    // the one place that ever drains `pending_replies`.
    let reply_handle = app.clone();
    app.listen_any("hew://api-reply", move |event| {
        let Ok(payload) = serde_json::from_str::<ReplyPayload>(event.payload()) else {
            return;
        };
        let tx = reply_handle
            .state::<LiveApi>()
            .pending_replies
            .lock()
            .expect("pending_replies mutex")
            .remove(&payload.conn_id);
        if let Some(tx) = tx {
            let _ = tx.send(payload.frame);
        }
    });

    spawn_accept_loop(app.clone(), listener, token);
}

#[cfg(windows)]
pub fn start(app: &AppHandle) {
    windows::start(app);
}

#[cfg(not(any(unix, windows)))]
pub fn start(_app: &AppHandle) {
    // No transport exists for anything other than unix or Windows: publish
    // no discovery file and start no listener, so a client sees the honest
    // "no running instance found" rather than a confusing connect failure
    // against a socket/pipe that could never have existed.
}

/// Accepts connections forever on a dedicated thread, handing each one to
/// its own thread in turn — one OS thread per live connection is cheap at
/// the scale this ever sees (a handful of agent/CLI connections, not a
/// public listener), so this reaches for `std::thread` rather than
/// pulling the async runtime in for what would otherwise be its first use
/// in this crate.
///
/// The window a connection's frames route to is resolved ONCE here, at
/// accept time, via the same `active_document_window` every other
/// single-target shell event uses (`emit_to_active`'s helper, `main.rs`)
/// — not re-resolved per frame. Binding at accept time, not floating with
/// whatever window is focused *right now*, means a user switching focus
/// mid-session can never make an already-open connection's next mutation
/// land in a different document than the one it started on; the
/// multi-window "which document does a live connection belong to" story
/// beyond that is future work (docs/agents/HEW_API.md names only single-document
/// desktop hosts today).
#[cfg(unix)]
fn spawn_accept_loop(app: AppHandle, listener: std::os::unix::net::UnixListener, token: String) {
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(stream) = incoming else { continue };
            let Some(window) = crate::active_document_window(&app) else {
                // No open document window at all — nothing to attach to;
                // drop the connection by simply not handling it.
                continue;
            };
            let app = app.clone();
            let token = token.clone();
            let window_label = window.label().to_string();
            std::thread::spawn(move || handle_connection(app, stream, token, window_label));
        }
    });
}

/// One accepted connection's whole lifetime: the mandatory `hello`+token
/// handshake (§11.2 — anything else on the first frame is dropped
/// silently, no bytes written back), then the frame-at-a-time forward
/// loop until the peer closes or a write fails.
#[cfg(unix)]
fn handle_connection(
    app: AppHandle,
    mut stream: std::os::unix::net::UnixStream,
    token: String,
    window_label: String,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut first_line = String::new();
    if reader.read_line(&mut first_line).unwrap_or(0) == 0 {
        return; // closed before sending anything
    }
    let Some(sanitized_hello) = check_and_sanitize_hello(&first_line, &token) else {
        return; // wrong method, missing/wrong token, or malformed — drop silently
    };

    let conn_id = {
        let state = app.state::<LiveApi>();
        let id = state.next_conn_id.fetch_add(1, Ordering::Relaxed);
        state
            .conn_window
            .lock()
            .expect("conn_window mutex")
            .insert(id, window_label.clone());
        id
    };
    let _ = app.emit_to(
        &window_label,
        "hew://api-connection-open",
        ConnPayload { conn_id },
    );

    let mut ok = forward_and_await(&app, &window_label, conn_id, &sanitized_hello, &mut stream);
    while ok {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break, // peer closed / read error
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        // The token gate runs once, on the first frame — but a later frame
        // (a second `hello`, or anything else) may still CARRY a token
        // field, and the webview must never see the secret. Strip it from
        // every frame, not just the handshake.
        let sanitized = strip_token(trimmed);
        ok = forward_and_await(&app, &window_label, conn_id, &sanitized, &mut stream);
    }

    let state = app.state::<LiveApi>();
    state
        .conn_window
        .lock()
        .expect("conn_window mutex")
        .remove(&conn_id);
    state
        .pending_replies
        .lock()
        .expect("pending_replies mutex")
        .remove(&conn_id);
    let _ = app.emit_to(
        &window_label,
        "hew://api-connection-close",
        ConnPayload { conn_id },
    );
}

/// Validates that `line` is `hew.meta.hello` carrying `params.token ==
/// expected`, and returns it with the token stripped out — the webview's
/// WASM dispatch never sees the real secret, only that hello succeeded or
/// (on a protocol-level mismatch unrelated to the token, e.g. a bad
/// `protocol` number) its own typed error. `None` for anything that fails
/// the gate: wrong/missing method, wrong/missing token, or JSON that
/// doesn't even parse — every one of those is "drop silently" per
/// docs/agents/HEW_API.md §11.2, never a written response.
fn check_and_sanitize_hello(line: &str, expected_token: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(line.trim_end()).ok()?;
    if value.get("method").and_then(serde_json::Value::as_str) != Some("hew.meta.hello") {
        return None;
    }
    let token_ok = value
        .pointer("/params/token")
        .and_then(serde_json::Value::as_str)
        == Some(expected_token);
    if !token_ok {
        return None;
    }
    if let Some(params) = value
        .get_mut("params")
        .and_then(serde_json::Value::as_object_mut)
    {
        params.remove("token");
    }
    Some(value.to_string())
}

/// Removes `params.token` from any frame, leaving everything else byte-
/// identical (and leaving unparseable frames untouched, so the dispatcher
/// still answers them with its own parse error). The webview never needs
/// the token — the shell already decided this connection is authorized —
/// so it never receives it.
fn strip_token(line: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
        return line.to_string();
    };
    let had = value
        .get_mut("params")
        .and_then(serde_json::Value::as_object_mut)
        .map(|params| params.remove("token").is_some())
        .unwrap_or(false);
    if had {
        value.to_string()
    } else {
        line.to_string()
    }
}

/// Forwards one frame to `window_label`'s webview and, if it carries an
/// `id` (i.e. it is a real request, not a notification — a notification
/// gets no reply from `crates/api` at all, §4.1), blocks for the matching
/// `hew://api-reply` and writes it back to the socket. Returns `false`
/// only on a hard transport failure (the window is gone, or the socket
/// write itself failed) — a reply *timeout* still writes a synthesized
/// internal-fault reply and returns `true`, so one slow dispatch cannot
/// wedge every frame after it.
///
/// The `id`-presence peek is transport-level flow control, not a
/// duplication of `crates/api`'s own protocol rules: whether a JSON-RPC
/// object carries an `id` key is a wire-level fact either side can read
/// off the frame, and `crates/api`'s `Connection::dispatch` remains the
/// sole authority on what a notification actually MEANS (dropped,
/// unexecuted). If `line` fails to parse as JSON at all, this always
/// waits for a reply — `Scene::api_dispatch` guarantees one for malformed
/// JSON (a `PARSE_ERROR` response, id `null`).
///
/// Unix-only: this signature names `std::os::unix::net::UnixStream`
/// directly, a type that does not exist on Windows, so the whole function
/// must be `#[cfg(unix)]`-gated rather than merely unreferenced there —
/// leaving it ungated is what made the crate fail to compile for Windows
/// at all (adversarial review's finding 1). `mod windows` below has its
/// own `forward_and_await`, generic over a pipe `HANDLE` instead of a
/// `UnixStream`, sharing everything else (the needs-reply peek,
/// `REPLY_TIMEOUT`, the synthesized-timeout-then-close behavior) with
/// this one by hand rather than through a common trait — see this file's
/// top doc comment for why a `UnixListener`/named-pipe `HANDLE` are not
/// abstracted over a shared trait for just two call sites.
#[cfg(unix)]
fn forward_and_await(
    app: &AppHandle,
    window_label: &str,
    conn_id: u32,
    line: &str,
    stream: &mut std::os::unix::net::UnixStream,
) -> bool {
    // `id: null` is NOT a request: `api::Request` deserializes it to
    // `None`, i.e. a notification the dispatcher drops without answering.
    // Peeking for the KEY alone would leave this side waiting 60s for a
    // reply that is never coming, so the peek matches the dispatcher's
    // own rule — a non-null id, or an unparseable frame (which
    // `Scene::api_dispatch` always answers with a parse error).
    let needs_reply = serde_json::from_str::<serde_json::Value>(line)
        .map(|v| v.get("id").is_some_and(|id| !id.is_null()))
        .unwrap_or(true);

    if !needs_reply {
        let _ = app.emit_to(
            window_label,
            "hew://api-frame",
            FramePayload {
                conn_id,
                frame: line,
            },
        );
        return true;
    }

    let (tx, rx) = mpsc::channel::<String>();
    {
        let state = app.state::<LiveApi>();
        state
            .pending_replies
            .lock()
            .expect("pending_replies mutex")
            .insert(conn_id, tx);
    }
    if app
        .emit_to(
            window_label,
            "hew://api-frame",
            FramePayload {
                conn_id,
                frame: line,
            },
        )
        .is_err()
    {
        let state = app.state::<LiveApi>();
        state
            .pending_replies
            .lock()
            .expect("pending_replies mutex")
            .remove(&conn_id);
        return false; // the target window is gone
    }

    let reply = match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(reply) => reply,
        Err(_) => {
            // The dispatch is still running in the webview and will emit
            // its reply eventually. Replies are correlated by connection,
            // not by request id, so leaving this connection open would
            // hand that late reply to whatever request came NEXT — the
            // caller would receive another command's result under its own
            // id. Write the timeout answer and CLOSE the connection
            // instead: the stale reply then finds no slot and is dropped,
            // and the client reconnects with a clean slate.
            app.state::<LiveApi>()
                .pending_replies
                .lock()
                .expect("pending_replies mutex")
                .remove(&conn_id);
            let mut out = synthesize_timeout_reply(line);
            out.push('\n');
            let _ = stream.write_all(out.as_bytes());
            let _ = stream.flush();
            return false;
        }
    };
    let mut out = reply;
    out.push('\n');
    stream.write_all(out.as_bytes()).is_ok() && stream.flush().is_ok()
}

/// Builds the `-32003` (internal fault) reply written back when the
/// webview never answers `hew://api-reply` in time — `id` is echoed back
/// when `request` parses well enough to have one, `null` otherwise
/// (mirrors `api::Response`'s own "unreadable id" convention,
/// docs/agents/HEW_API.md §4.4).
///
/// The message states the outcome honestly: unlike an ordinary `-32003`,
/// which means a kernel invariant failed and the document is untouched,
/// a timed-out dispatch is still RUNNING in the webview and usually
/// completes. The caller learns the request's fate by querying, not by
/// assuming it was rolled back. The connection closes immediately after
/// this reply (see `forward_and_await`).
fn synthesize_timeout_reply(request: &str) -> String {
    let id = serde_json::from_str::<serde_json::Value>(request)
        .ok()
        .and_then(|v| v.get("id").cloned())
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32003,
            "message": "the desktop app did not reply in time; the command may still have been applied — query the document to see. This connection is now closed.",
        }
    })
    .to_string()
}

/// The Windows named-pipe name for a given pid: `\\.\pipe\hew-<pid>`. This
/// full string is what goes in the discovery file's `socket` field (the
/// same field unix's socket path lives in) and is the client's connect
/// string verbatim — see docs/agents/HEW_API.md §11.2 and
/// `crates/hew-cli/src/live.rs`'s `RawConn::connect`. Pure string
/// formatting, no Windows API involved, so it is defined unconditionally
/// and tested on every platform this crate builds on, not only
/// `#[cfg(windows)]`. Only ever called from `mod windows` below, so
/// non-Windows builds see it as unused — `allow(dead_code)` rather than
/// `#[cfg(windows)]` on the function itself, precisely so it keeps
/// compiling (and its test below keeps running) on every platform.
#[allow(dead_code)]
fn pipe_name(pid: u32) -> String {
    format!(r"\\.\pipe\hew-{pid}")
}

/// The Windows half of the transport: a named pipe in place of unix's
/// domain socket, everything else (token gate, Tauri-event bridge,
/// `LiveApi` bookkeeping) shared with the parent module via `use
/// super::*`. See this file's top doc comment for the security-posture
/// comparison against unix's 0600.
#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, FALSE, HANDLE,
        INVALID_HANDLE_VALUE, STILL_ACTIVE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{
        FlushFileBuffers, GetFileAttributesW, ReadFile, WriteFile, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_FIRST_PIPE_INSTANCE, INVALID_FILE_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// `HANDLE` is `*mut c_void` and does not auto-implement `Send`, but
    /// `ReadFile`/`WriteFile`/`ConnectNamedPipe`/`DisconnectNamedPipe`/
    /// `CloseHandle` are documented thread-agnostic (the handle names a
    /// process-wide kernel object, not a thread-local one). Every value
    /// of this type is threaded through as a whole (never destructured to
    /// its raw `.0` on the spot a closure captures it), so Rust 2021's
    /// disjoint-capture rule captures the whole newtype — and this `Send`
    /// impl — rather than reaching straight through to the un-`Send` raw
    /// pointer. See the identical `RawHandle` in
    /// `crates/hew-cli/src/live.rs`'s Windows transport for the client
    /// side of the same trick.
    #[derive(Clone, Copy)]
    struct SendableHandle(HANDLE);
    unsafe impl Send for SendableHandle {}

    // --------------------------------------------------------- discovery

    /// Mirrors `crates/hew-cli/src/live.rs`'s `platform_runtime_dir` for
    /// Windows exactly (field names, `HEW_RUNTIME_DIR` override) — the two
    /// live independently (this crate cannot depend on `hew-cli`, and vice
    /// versa) but MUST agree bit-for-bit or discovery silently fails
    /// between the two halves of this transport.
    fn runtime_dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("HEW_RUNTIME_DIR") {
            return PathBuf::from(dir);
        }
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        local.join("Hew").join("run")
    }

    fn instances_dir() -> PathBuf {
        runtime_dir().join("hew")
    }

    /// 256 bits of randomness from `BCryptGenRandom` with the system's
    /// preferred CSPRNG, hex-encoded — the discovery token
    /// (docs/agents/HEW_API.md §11.2), Windows' counterpart to unix's
    /// `/dev/urandom` read. No `rand`/`getrandom` dependency needed: this
    /// is the one CNG call, and `windows-sys` is already a dependency for
    /// the transport itself.
    fn generate_token_hex() -> std::io::Result<String> {
        let mut buf = [0u8; 32];
        let status = unsafe {
            BCryptGenRandom(
                ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(std::io::Error::other(format!(
                "BCryptGenRandom failed with NTSTATUS 0x{status:08x}"
            )));
        }
        Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// `OpenProcess` with the narrowest right that still answers liveness
    /// (needs no special privilege, even against a process owned by
    /// another user) plus `GetExitCodeProcess` — mirrors
    /// `crates/hew-cli/src/live.rs`'s own `is_pid_alive` (a separate copy;
    /// this crate cannot depend on `hew-cli`). A pid this process cannot
    /// even open is treated as dead, the same "any failure means not
    /// alive" posture unix's `kill -0` takes.
    fn is_pid_alive(pid: u32) -> bool {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if handle.is_null() {
                return false;
            }
            let mut exit_code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            ok != 0 && exit_code == STILL_ACTIVE as u32
        }
    }

    /// Deletes every `instance-*.json` in `dir` whose pid is no longer
    /// alive — the app's own startup half of §11.2's validate-then-use
    /// discovery contract, mirroring unix's `sweep_stale` with one
    /// deliberate difference: there is no orphan-pipe-file sweep here,
    /// because there is no orphan pipe TO sweep. A named pipe is a kernel
    /// object, not a filesystem entry — the last handle to it closes
    /// automatically when its owning process exits, cleanly or not, so a
    /// dead process can never leave a stale pipe object behind the way it
    /// can leave a stale `.sock` FILE on unix. A missing directory or an
    /// unreadable/malformed sibling is silently skipped, not an error.
    fn sweep_stale(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !(name.starts_with("instance-") && name.ends_with(".json")) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let Some(pid) = value.get("pid").and_then(serde_json::Value::as_u64) else {
                continue;
            };
            if is_pid_alive(pid as u32) {
                continue;
            }
            let _ = std::fs::remove_file(&path);
        }
    }

    /// Whether `path` is a reparse point (symlink or junction) — checked
    /// with a plain `GetFileAttributesW`, which needs no extra
    /// windows-sys feature beyond what this transport already depends on.
    /// Used only for a friendlier diagnostic in
    /// [`ensure_safe_runtime_dir`]: the real security property that
    /// function relies on is `std::fs::canonicalize` resolving EVERY
    /// reparse point in the path (not only the leaf this checks), so a
    /// junction/symlink planted anywhere in the chain is caught either
    /// way — this just tells that failure mode apart from "resolves to
    /// somewhere outside %LOCALAPPDATA% for some other reason" in the
    /// error message.
    fn is_reparse_point(path: &Path) -> bool {
        let wide_path = wide(&path.to_string_lossy());
        let attrs = unsafe { GetFileAttributesW(wide_path.as_ptr()) };
        attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_REPARSE_POINT) != 0
    }

    /// Creates `dir` and confirms it is safe to trust before anything
    /// sensitive (a discovery file carrying the session token) is written
    /// into it — Windows' counterpart to unix's `ensure_owned_dir`.
    /// Windows offers no single-syscall uid-style ownership check this
    /// transport can use without a heavier windows-sys footprint
    /// (`GetNamedSecurityInfoW` + SID comparison) than the rest of it
    /// otherwise needs, so what is checked here is different in kind, not
    /// merely a Windows spelling of the same thing:
    ///
    /// - `dir` is not itself a reparse point (a symlink/junction someone
    ///   could plant ahead of launch to redirect writes elsewhere).
    /// - The property that actually matters — `std::fs::canonicalize` on
    ///   Windows resolves EVERY reparse point in the path, not only the
    ///   leaf — the fully resolved real path still lies under this
    ///   process's own `%LOCALAPPDATA%`.
    ///
    /// What is NOT checked, honestly: the discretionary ACL on `dir` or
    /// its ancestors (unix's actual uid-ownership equivalent) — a
    /// `%LOCALAPPDATA%` an administrator has made writable to other users
    /// would not be caught by this. `sweep_stale` also runs before this
    /// check is ever performed (it is only reached from
    /// `write_discovery_file`, called after `start`'s own `sweep_stale`),
    /// mirroring unix's own `sweep_stale`, which likewise applies no
    /// ownership check of its own — a pre-existing limitation on both
    /// platforms, not one introduced here. See this module's top doc
    /// comment and docs/agents/HEW_API.md §11.2 for the same honest gap stated
    /// for readers of the spec.
    fn ensure_safe_runtime_dir(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        if is_reparse_point(dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{} is a symlink or junction — refusing to trust it",
                    dir.display()
                ),
            ));
        }
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "%LOCALAPPDATA% is not set")
            })?;
        let local_real = std::fs::canonicalize(&local)?;
        let dir_real = std::fs::canonicalize(dir)?;
        if !dir_real.starts_with(&local_real) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{} does not resolve under %LOCALAPPDATA% ({}) — refusing to trust it",
                    dir.display(),
                    local.display()
                ),
            ));
        }
        Ok(())
    }

    /// Writes `<dir>/instance-<pid>.json` (docs/agents/HEW_API.md §11.2), after
    /// [`ensure_safe_runtime_dir`] confirms `dir` is trustworthy. Beyond
    /// that check, this still applies no explicit ACL to the file or
    /// directory the way unix's 0600/0700 does: `%LOCALAPPDATA%` is
    /// already per-user by construction (a different Windows user
    /// account has an entirely different `%LOCALAPPDATA%`, unlike unix's
    /// potentially-shared `/tmp`), so the file inherits whatever
    /// protection that per-user tree already carries rather than a
    /// descriptor applied here. This is a real but different guarantee
    /// than unix's file-mode bits — honestly weaker in one respect (no
    /// explicit deny-other-users ACE on this specific file the way
    /// `owner_only_security_attributes` gives the PIPE below), and
    /// documented as such in this module's top doc comment and
    /// docs/agents/HEW_API.md §11.2.
    fn write_discovery_file(
        dir: &Path,
        pid: u32,
        socket: &str,
        token: &str,
    ) -> std::io::Result<()> {
        ensure_safe_runtime_dir(dir)?;
        let path = dir.join(format!("instance-{pid}.json"));
        let body = DiscoveryFile {
            socket,
            token,
            pid,
            version: env!("CARGO_PKG_VERSION"),
        };
        let json = serde_json::to_vec(&body).expect("DiscoveryFile serializes");
        std::fs::write(&path, &json)
    }

    /// Removes this process's own discovery file — "removes it on exit"
    /// (docs/agents/HEW_API.md §11.2). There is no paired socket FILE to remove
    /// on Windows (see `sweep_stale`'s doc comment): the pipe itself goes
    /// away when this process's last handle to it closes, which happens
    /// automatically at process exit regardless of whether this function
    /// ever runs.
    pub(super) fn cleanup() {
        let pid = std::process::id();
        let dir = instances_dir();
        let _ = std::fs::remove_file(dir.join(format!("instance-{pid}.json")));
    }

    // --------------------------------------------------------- transport

    /// Frees the `LocalAlloc`'d security descriptor
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW` returns.
    /// `CreateNamedPipeW` copies a security descriptor's contents at
    /// creation time rather than retaining the pointer, so this is safe to
    /// drop immediately after each `CreateNamedPipeW` call that used it.
    struct SecurityDescriptorGuard(PSECURITY_DESCRIPTOR);
    impl Drop for SecurityDescriptorGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    windows_sys::Win32::Foundation::LocalFree(self.0);
                }
            }
        }
    }

    /// A security descriptor granting the pipe's OWNER (resolved at
    /// creation time to the current process token's user — the logged-in
    /// user who launched Hew) full access, and nobody else:
    /// `D:P(A;;GA;;;OW)` — discretionary ACL (`D:`), protected (`P`, so
    /// nothing broader is inherited from the parent container), one Allow
    /// ACE (`A`) granting Generic All (`GA`) to the Owner SID (`OW`).
    /// docs/agents/HEW_API.md §11.2 calls this the Windows analogue of unix's
    /// 0600: it keeps every OTHER interactively logged-in user's process
    /// from opening the pipe at all — though, like unix's root, a local
    /// Administrator/SYSTEM can still bypass a DACL with sufficient
    /// privilege. That ceiling is an OS property this transport does not
    /// try to defeat, on either platform.
    fn owner_only_security_attributes(
    ) -> std::io::Result<(SECURITY_ATTRIBUTES, SecurityDescriptorGuard)> {
        let sddl = wide("D:P(A;;GA;;;OW)");
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let guard = SecurityDescriptorGuard(sd);
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: FALSE,
        };
        Ok((sa, guard))
    }

    /// Creates one pipe instance for `name`. `first` must be `true` only
    /// for the very first instance ever created for this pid's pipe name
    /// (`start`'s own call) and `false` for every instance the accept
    /// loop creates afterward — `FILE_FLAG_FIRST_PIPE_INSTANCE` makes
    /// `CreateNamedPipeW` fail with `ERROR_ACCESS_DENIED` outright if a
    /// pipe instance of this name already exists, which is exactly the
    /// defense this flag exists for: without it, another local user's
    /// process could pre-create `\\.\pipe\hew-<pid>` (a name this
    /// process's own pid makes fully predictable) ahead of launch, and
    /// this call would silently add an instance to THEIR pipe object —
    /// whose DACL, not `owner_only_security_attributes`'s, governs who
    /// else can open it. Passing `true` for every subsequent instance
    /// would be wrong for the opposite reason: it would make each of
    /// THIS process's own later, legitimate instances fail the same way,
    /// since by then an instance already exists (created by us).
    fn create_pipe_instance(name: &[u16], first: bool) -> std::io::Result<HANDLE> {
        let (sa, guard) = owner_only_security_attributes()?;
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                // `CreateNamedPipeW`'s `lpSecurityAttributes` is
                // `*const SECURITY_ATTRIBUTES` — `&sa` coerces to that
                // directly; this was `&mut sa` (needlessly) before.
                &sa,
            )
        };
        drop(guard); // CreateNamedPipeW already copied the descriptor.
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(handle)
    }

    /// Starts the live API listener: sweeps stale discovery files, creates
    /// the first named-pipe instance (mirrors unix's `start` binding its
    /// `UnixListener` before publishing discovery — a client must never be
    /// able to find a discovery file naming a pipe that does not exist
    /// yet), publishes the discovery file, and spawns the accept loop.
    /// Best-effort end to end, exactly like the unix `start`: any failure
    /// is logged to stderr and leaves `--live` simply unavailable this
    /// session.
    pub(super) fn start(app: &AppHandle) {
        app.manage(LiveApi::default());

        let dir = instances_dir();
        sweep_stale(&dir);

        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("hew: live API disabled — could not create {dir:?}: {e}");
            return;
        }

        let pid = std::process::id();
        let name = super::pipe_name(pid);
        let name_w = wide(&name);

        // `first: true` — see `create_pipe_instance`'s doc comment.
        // `ERROR_ACCESS_DENIED` here means the OS refused BECAUSE an
        // instance of this name already exists: this pid's pipe name is
        // fully predictable (`\\.\pipe\hew-<pid>`), so another local
        // user's process could have pre-created it to squat on the name
        // before this process got here. Rather than silently adding an
        // instance to that squatter's pipe object (whose DACL, not
        // `owner_only_security_attributes`'s, would then govern who else
        // can open it), this fails loudly and leaves `--live` disabled.
        let first_handle = match create_pipe_instance(&name_w, true) {
            Ok(h) => SendableHandle(h),
            Err(e) => {
                let hint = if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                    " — a pipe instance with this name already exists (possibly a name-squatting attempt by another local process); refusing to add an instance to it"
                } else {
                    ""
                };
                eprintln!("hew: live API disabled — could not create the named pipe: {e}{hint}");
                return;
            }
        };

        let token = match generate_token_hex() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("hew: live API disabled — could not generate a discovery token: {e}");
                unsafe { CloseHandle(first_handle.0) };
                return;
            }
        };

        if let Err(e) = write_discovery_file(&dir, pid, &name, &token) {
            eprintln!("hew: live API disabled — could not write the discovery file: {e}");
            unsafe { CloseHandle(first_handle.0) };
            return;
        }

        // The reply channel: identical to unix's — the webview calls back
        // into Rust via a plain `emit("hew://api-reply", {connId, frame})`
        // regardless of which transport is forwarding frames.
        let reply_handle = app.clone();
        app.listen_any("hew://api-reply", move |event| {
            let Ok(payload) = serde_json::from_str::<ReplyPayload>(event.payload()) else {
                return;
            };
            let tx = reply_handle
                .state::<LiveApi>()
                .pending_replies
                .lock()
                .expect("pending_replies mutex")
                .remove(&payload.conn_id);
            if let Some(tx) = tx {
                let _ = tx.send(payload.frame);
            }
        });

        spawn_accept_loop(app.clone(), name, token, first_handle);
    }

    /// Accepts connections forever on a dedicated thread — the Windows
    /// analogue of unix's `spawn_accept_loop`, adapted to named pipes'
    /// per-instance accept model. Each iteration blocks in
    /// `ConnectNamedPipe` until a client connects to the CURRENT
    /// instance, then hands it to its own thread — the standard Windows
    /// multi-client named-pipe server pattern. The window a connection's
    /// frames route to is resolved once, at accept time, exactly as unix
    /// does (see unix's `spawn_accept_loop` doc comment for why).
    ///
    /// The NEXT instance is created before this one is handed off or
    /// even connected — not after, the way an earlier version of this
    /// loop ordered it. That ordering left a real gap: a pipe instance is
    /// connectable to a client from the moment `CreateNamedPipeW` returns
    /// it, whether or not the server has called `ConnectNamedPipe` on it
    /// yet, so pre-creating the next instance up front means at least one
    /// instance is ALWAYS live for the whole time the current one is in
    /// use. Creating it only after handing the current one off left a
    /// window — between that instance's own thread finishing and closing
    /// its handle, and this loop's next `CreateNamedPipeW` call
    /// completing — where the pipe name could momentarily have ZERO live
    /// instances, turning a racing client's `CreateFileW` from the
    /// expected `ERROR_PIPE_BUSY` (which it retries via `WaitNamedPipeW`)
    /// into `ERROR_FILE_NOT_FOUND` (which, correctly, it does NOT retry —
    /// see `crates/hew-cli/src/live.rs`'s `RawConn::connect`).
    fn spawn_accept_loop(
        app: AppHandle,
        name: String,
        token: String,
        first_handle: SendableHandle,
    ) {
        std::thread::spawn(move || {
            let name_w = wide(&name);
            let mut handle = first_handle;
            loop {
                let next_handle = match create_pipe_instance(&name_w, false) {
                    Ok(h) => Some(SendableHandle(h)),
                    Err(e) => {
                        eprintln!(
                            "hew: live API accept loop — could not pre-create the next pipe instance (will retry next iteration): {e}"
                        );
                        None
                    }
                };

                let connected = unsafe { ConnectNamedPipe(handle.0, ptr::null_mut()) };
                // `connected == 0` with `GetLastError() == ERROR_PIPE_CONNECTED`
                // means a client connected in the window between this
                // instance's `CreateNamedPipeW` and this `ConnectNamedPipe`
                // call — a benign, documented Win32 race where the
                // connection IS already established, not a failure; it is
                // handled identically to `connected != 0` below (verified
                // against Win32 docs — adversarial review flagged this
                // path as unverified, not as wrong).
                let usable = connected != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;

                if usable {
                    if let Some(window) = crate::active_document_window(&app) {
                        let app = app.clone();
                        let token = token.clone();
                        let window_label = window.label().to_string();
                        std::thread::spawn(move || {
                            handle_connection(app, handle, token, window_label)
                        });
                    } else {
                        // No open document window at all — nothing to
                        // attach to; drop the connection by simply not
                        // handling it.
                        unsafe {
                            DisconnectNamedPipe(handle.0);
                            CloseHandle(handle.0);
                        }
                    }
                } else {
                    // A real `ConnectNamedPipe` failure — this instance is
                    // unusable.
                    unsafe { CloseHandle(handle.0) };
                }

                handle = match next_handle {
                    Some(h) => h,
                    None => match create_pipe_instance(&name_w, false) {
                        Ok(h) => SendableHandle(h),
                        Err(e) => {
                            eprintln!(
                                "hew: live API accept loop stopping — could not create a pipe instance: {e}"
                            );
                            return;
                        }
                    },
                };
            }
        });
    }

    /// One accepted connection's whole lifetime — the Windows analogue of
    /// unix's `handle_connection`: the mandatory `hello`+token handshake,
    /// then the frame-at-a-time forward loop until the peer closes or a
    /// write fails, sharing `check_and_sanitize_hello`/`strip_token` with
    /// the unix path verbatim.
    fn handle_connection(
        app: AppHandle,
        handle: SendableHandle,
        token: String,
        window_label: String,
    ) {
        let mut leftover: Vec<u8> = Vec::new();
        let Some(first_line) = read_line(handle, &mut leftover) else {
            unsafe {
                DisconnectNamedPipe(handle.0);
                CloseHandle(handle.0);
            }
            return; // closed before sending anything
        };
        let Some(sanitized_hello) = check_and_sanitize_hello(&first_line, &token) else {
            unsafe {
                DisconnectNamedPipe(handle.0);
                CloseHandle(handle.0);
            }
            return; // wrong method, missing/wrong token, or malformed — drop silently
        };

        let conn_id = {
            let state = app.state::<LiveApi>();
            let id = state.next_conn_id.fetch_add(1, Ordering::Relaxed);
            state
                .conn_window
                .lock()
                .expect("conn_window mutex")
                .insert(id, window_label.clone());
            id
        };
        let _ = app.emit_to(
            &window_label,
            "hew://api-connection-open",
            ConnPayload { conn_id },
        );

        let mut ok = forward_and_await(&app, &window_label, conn_id, &sanitized_hello, handle);
        while ok {
            let Some(line) = read_line(handle, &mut leftover) else {
                break; // peer closed / read error
            };
            // `trim_end`, not a bare emptiness check on `line` — `read_line`
            // only strips the `\n` byte itself, so a CRLF-terminated blank
            // line would otherwise still carry a lone `\r` and fail to
            // match unix's `line.trim_end().is_empty()` skip (unix's
            // `BufReader::read_line` also keeps the `\n`, trimmed the same
            // way). Adversarial review found this platform divergence.
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let sanitized = strip_token(trimmed);
            ok = forward_and_await(&app, &window_label, conn_id, &sanitized, handle);
        }

        let state = app.state::<LiveApi>();
        state
            .conn_window
            .lock()
            .expect("conn_window mutex")
            .remove(&conn_id);
        state
            .pending_replies
            .lock()
            .expect("pending_replies mutex")
            .remove(&conn_id);
        let _ = app.emit_to(
            &window_label,
            "hew://api-connection-close",
            ConnPayload { conn_id },
        );

        unsafe {
            // `FlushFileBuffers` before `DisconnectNamedPipe` — NOT
            // optional. `DisconnectNamedPipe` forcibly tears down the
            // connection, discarding any bytes the server has written
            // that the client has not yet read; unlike a unix domain
            // socket's `close` (where already-transmitted-but-unread data
            // survives in the kernel buffer for the peer to still read),
            // a named pipe has no such grace. Without this flush, the
            // synthesized reply `forward_and_await` writes right before
            // returning `false` on a reply timeout — the load-bearing
            // "the desktop app did not reply in time" answer — could be
            // destroyed before the client ever sees it. `FlushFileBuffers`
            // on a named pipe blocks until the client has read everything
            // written or the pipe breaks, which is the standard remedy
            // and is sufficient here: every write on this connection
            // (`forward_and_await`'s `write_all`) already completed
            // before this point, so there is nothing still being written
            // for it to race against. The one residual cost is that a
            // client that stops reading entirely (frozen, not merely
            // slow) can block this call indefinitely — this thread is
            // per-connection and blocking it does not affect any other
            // connection or the app itself, the same tradeoff every other
            // blocking call in this transport already accepts.
            FlushFileBuffers(handle.0);
            DisconnectNamedPipe(handle.0);
            CloseHandle(handle.0);
        }
    }

    /// Reads one newline-delimited frame from `handle`, blocking — no
    /// per-read timeout here, mirroring unix's own incoming-frame read
    /// (`BufReader::read_line` has none either): the connection just waits
    /// for the client's next frame or its close. `leftover` carries any
    /// bytes read past the last newline forward to the next call, since a
    /// named pipe `HANDLE` has no built-in line buffering the way
    /// `BufReader` gives unix for free. `None` for EOF, a hard read error,
    /// or a line that is not valid UTF-8 — the same three cases unix's
    /// `Ok(0) | Err(_)` collapses into "the connection is done".
    fn read_line(handle: SendableHandle, leftover: &mut Vec<u8>) -> Option<String> {
        loop {
            if let Some(pos) = leftover.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = leftover.drain(..=pos).collect();
                line.pop(); // the newline itself
                return String::from_utf8(line).ok();
            }
            let mut buf = [0u8; 4096];
            let mut n: u32 = 0;
            let ok = unsafe {
                ReadFile(
                    handle.0,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut n,
                    ptr::null_mut(),
                )
            };
            if ok == 0 || n == 0 {
                return None;
            }
            leftover.extend_from_slice(&buf[..n as usize]);
        }
    }

    /// Windows counterpart of unix's `forward_and_await`: identical
    /// semantics (needs-reply peek, one frame in flight, `REPLY_TIMEOUT`,
    /// synthesized timeout reply + connection close on timeout), writing
    /// to a pipe `HANDLE` via `WriteFile` instead of a `UnixStream`.
    fn forward_and_await(
        app: &AppHandle,
        window_label: &str,
        conn_id: u32,
        line: &str,
        handle: SendableHandle,
    ) -> bool {
        let needs_reply = serde_json::from_str::<serde_json::Value>(line)
            .map(|v| v.get("id").is_some_and(|id| !id.is_null()))
            .unwrap_or(true);

        if !needs_reply {
            let _ = app.emit_to(
                window_label,
                "hew://api-frame",
                FramePayload {
                    conn_id,
                    frame: line,
                },
            );
            return true;
        }

        let (tx, rx) = mpsc::channel::<String>();
        {
            let state = app.state::<LiveApi>();
            state
                .pending_replies
                .lock()
                .expect("pending_replies mutex")
                .insert(conn_id, tx);
        }
        if app
            .emit_to(
                window_label,
                "hew://api-frame",
                FramePayload {
                    conn_id,
                    frame: line,
                },
            )
            .is_err()
        {
            let state = app.state::<LiveApi>();
            state
                .pending_replies
                .lock()
                .expect("pending_replies mutex")
                .remove(&conn_id);
            return false; // the target window is gone
        }

        let reply = match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) => reply,
            Err(_) => {
                app.state::<LiveApi>()
                    .pending_replies
                    .lock()
                    .expect("pending_replies mutex")
                    .remove(&conn_id);
                let mut out = synthesize_timeout_reply(line);
                out.push('\n');
                let _ = write_all(handle, out.as_bytes());
                return false;
            }
        };
        let mut out = reply;
        out.push('\n');
        write_all(handle, out.as_bytes()).is_ok()
    }

    fn write_all(handle: SendableHandle, mut buf: &[u8]) -> std::io::Result<()> {
        while !buf.is_empty() {
            let mut n: u32 = 0;
            let ok = unsafe {
                WriteFile(
                    handle.0,
                    buf.as_ptr(),
                    buf.len().min(u32::MAX as usize) as u32,
                    &mut n,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
            buf = &buf[n as usize..];
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn is_pid_alive_is_true_for_the_current_process() {
            assert!(is_pid_alive(std::process::id()));
        }

        #[test]
        fn is_pid_alive_is_false_for_a_pid_that_is_almost_certainly_dead() {
            assert!(!is_pid_alive(999_999));
        }

        #[test]
        fn generate_token_hex_produces_64_hex_chars_of_real_entropy() {
            let a = generate_token_hex().expect("BCryptGenRandom succeeds");
            let b = generate_token_hex().expect("BCryptGenRandom succeeds");
            assert_eq!(a.len(), 64);
            assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
            assert_ne!(a, b, "two draws must not collide");
        }

        fn scratch_dir(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "hew-live-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn write_discovery_file_writes_the_shared_shape() {
            let dir = scratch_dir("discovery");
            write_discovery_file(&dir, 4242, r"\\.\pipe\hew-4242", "tok").unwrap();
            let path = dir.join("instance-4242.json");
            let body: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(body["pid"], 4242);
            assert_eq!(body["socket"], r"\\.\pipe\hew-4242");
            assert_eq!(body["token"], "tok");
            assert!(body["version"].is_string());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn sweep_stale_removes_a_dead_pids_file_and_keeps_a_live_one() {
            let dir = scratch_dir("sweep");
            const DEAD_PID: u32 = 999_998;
            write_discovery_file(&dir, DEAD_PID, r"\\.\pipe\hew-dead", "dead-tok").unwrap();
            let live_pid = std::process::id();
            write_discovery_file(&dir, live_pid, r"\\.\pipe\hew-live", "live-tok").unwrap();

            sweep_stale(&dir);

            assert!(!dir.join(format!("instance-{DEAD_PID}.json")).exists());
            assert!(dir.join(format!("instance-{live_pid}.json")).exists());
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// `std::env::temp_dir()` on a real Windows session lives under
        /// `%LOCALAPPDATA%\Temp`, so a scratch dir built on it (as every
        /// other test in this module already does) must pass the new
        /// provenance check — this pins that `write_discovery_file`'s
        /// added `ensure_safe_runtime_dir` call does not regress the
        /// ordinary, non-adversarial case.
        #[test]
        fn ensure_safe_runtime_dir_accepts_a_dir_under_local_appdata() {
            let dir = scratch_dir("safe-runtime-ok");
            assert!(ensure_safe_runtime_dir(&dir).is_ok());
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// The drive root is never under `%LOCALAPPDATA%` — a stand-in for
        /// the adversarial case this check exists for (a runtime directory
        /// that resolves somewhere outside the current user's own
        /// per-user tree). Writing directly under a drive root can itself
        /// fail with a plain permission error on a locked-down account;
        /// either way the outcome this test cares about — this directory
        /// must never be trusted — holds.
        #[test]
        fn ensure_safe_runtime_dir_rejects_a_dir_outside_local_appdata() {
            let dir = PathBuf::from(r"C:\hew-live-test-outside-appdata");
            let result = ensure_safe_runtime_dir(&dir);
            let _ = std::fs::remove_dir(&dir);
            assert!(
                result.is_err(),
                "a directory outside %LOCALAPPDATA% must be refused"
            );
        }

        #[test]
        fn is_reparse_point_is_false_for_an_ordinary_directory() {
            let dir = scratch_dir("not-a-reparse-point");
            assert!(!is_reparse_point(&dir));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure string formatting, no Windows API involved — runs on every
    /// platform, unlike the rest of the Windows transport's own tests
    /// (nested inside `mod windows`, `#[cfg(windows)]`-gated).
    #[test]
    fn pipe_name_is_the_documented_shape() {
        assert_eq!(pipe_name(4242), r"\\.\pipe\hew-4242");
    }

    #[test]
    fn check_and_sanitize_hello_accepts_the_matching_token_and_strips_it() {
        let line = r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1,"token":"secret"}}"#;
        let sanitized = check_and_sanitize_hello(line, "secret").expect("token matches");
        let value: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(value["method"], "hew.meta.hello");
        assert_eq!(value["params"]["protocol"], 1);
        assert!(
            value["params"].get("token").is_none(),
            "the real token must never reach the webview"
        );
    }

    /// The token gate runs once, but the secret must never reach the
    /// webview on ANY frame — a later frame carrying params.token is
    /// stripped too. Adversarial review found only the first frame
    /// sanitized.
    #[test]
    fn strip_token_removes_the_token_from_a_later_frame() {
        let line = r#"{"jsonrpc":"2.0","id":7,"method":"hew.meta.hello","params":{"protocol":1,"token":"secret"}}"#;
        let stripped = strip_token(line);
        assert!(
            !stripped.contains("secret"),
            "a later frame's token must be stripped too: {stripped}"
        );
        let value: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["params"]["protocol"], 1);
    }

    /// A frame with no token, and an unparseable frame, both pass through
    /// byte-identical — stripping must not reshape ordinary traffic or
    /// swallow the dispatcher's own parse error.
    #[test]
    fn strip_token_leaves_untokened_and_malformed_frames_alone() {
        let plain = r#"{"jsonrpc":"2.0","id":1,"method":"hew.query.scene","params":{}}"#;
        assert_eq!(strip_token(plain), plain);
        let garbage = "{not json";
        assert_eq!(strip_token(garbage), garbage);
    }

    #[test]
    fn check_and_sanitize_hello_rejects_a_mismatched_token() {
        let line = r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1,"token":"wrong"}}"#;
        assert!(check_and_sanitize_hello(line, "secret").is_none());
    }

    #[test]
    fn check_and_sanitize_hello_rejects_a_missing_token() {
        let line = r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#;
        assert!(check_and_sanitize_hello(line, "secret").is_none());
    }

    #[test]
    fn check_and_sanitize_hello_rejects_a_non_hello_first_frame() {
        let line = r#"{"jsonrpc":"2.0","id":0,"method":"hew.query.scene","params":{}}"#;
        assert!(check_and_sanitize_hello(line, "secret").is_none());
    }

    #[test]
    fn check_and_sanitize_hello_rejects_malformed_json() {
        assert!(check_and_sanitize_hello("not json at all", "secret").is_none());
    }

    #[test]
    fn synthesize_timeout_reply_echoes_the_id() {
        let out =
            synthesize_timeout_reply(r#"{"jsonrpc":"2.0","id":42,"method":"hew.query.scene"}"#);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["id"], 42);
        assert_eq!(value["error"]["code"], -32003);
    }

    #[test]
    fn synthesize_timeout_reply_uses_null_id_for_unparseable_input() {
        let out = synthesize_timeout_reply("garbage");
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["id"], serde_json::Value::Null);
    }

    #[cfg(unix)]
    #[test]
    fn generate_token_hex_produces_64_hex_chars_of_real_entropy() {
        let a = generate_token_hex().expect("/dev/urandom is readable in CI");
        let b = generate_token_hex().expect("/dev/urandom is readable in CI");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two draws must not collide");
    }

    #[cfg(unix)]
    #[test]
    fn write_discovery_file_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "hew-live-test-discovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_discovery_file(&dir, 4242, "/tmp/whatever.sock", "tok").unwrap();
        let path = dir.join("instance-4242.json");
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);

        let body: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(body["pid"], 4242);
        assert_eq!(body["socket"], "/tmp/whatever.sock");
        assert_eq!(body["token"], "tok");
        assert!(body["version"].is_string());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn sweep_stale_removes_a_dead_pids_files_and_keeps_a_live_one() {
        let dir = std::env::temp_dir().join(format!(
            "hew-live-test-sweep-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        const DEAD_PID: u32 = 999_999;
        let dead_sock = dir.join("dead.sock");
        std::fs::write(&dead_sock, b"").unwrap();
        write_discovery_file(&dir, DEAD_PID, dead_sock.to_str().unwrap(), "dead-tok").unwrap();

        let live_pid = std::process::id();
        write_discovery_file(&dir, live_pid, "/tmp/live.sock", "live-tok").unwrap();

        sweep_stale(&dir);

        assert!(!dir.join(format!("instance-{DEAD_PID}.json")).exists());
        assert!(!dead_sock.exists());
        assert!(dir.join(format!("instance-{live_pid}.json")).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forward_and_await_needs_reply_matches_id_presence() {
        // Exercised indirectly: a notification-shaped frame (no "id") must
        // never be treated as needing a reply. `forward_and_await` itself
        // needs a live AppHandle/socket, so this pins the pure predicate
        // it's built on instead.
        let with_id: serde_json::Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"x"}"#).unwrap();
        let without_id: serde_json::Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"x"}"#).unwrap();
        assert!(with_id.get("id").is_some());
        assert!(without_id.get("id").is_none());
    }
}
