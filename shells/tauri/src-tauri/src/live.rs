//! `--live` (docs/agents/HEW_API.md §11.2): the desktop half of the local socket
//! transport. This module owns exactly the transport and the token gate —
//! it never touches a `kernel::Document` or an `api::Connection` itself
//! (those live in `crates/wasm-api`, inside the webview's WASM sandbox).
//! Its job is: listen on a per-pid Unix domain socket, publish a discovery
//! file a client can find, verify the mandatory first-frame `hello` token,
//! and shuttle newline-delimited JSON-RPC frames between the socket and
//! the webview via Tauri events — `hew://api-frame` (Rust → JS),
//! `hew://api-reply` (JS → Rust), `hew://api-connection-open`/`-close`
//! (Rust → JS, connection lifecycle), and `hew://api-event` (JS → Rust, a
//! server-originated notification for one connection, docs/agents/HEW_API.md
//! §4.5). Each connection, on either transport, has one writer thread fed
//! by a channel (`LiveApi::outbound`): replies and events go through it,
//! so a frame is never split and events interleave between replies. While
//! a frame waits for its reply the connection's events are held
//! (`EventGate`) and follow that reply, so an event never arrives ahead of
//! the reply to the request that caused it. A write that stalls for
//! `WRITE_TIMEOUT` (a peer that stopped reading) or fails ends the writer,
//! which shuts the transport down so the connection's reader ends too.
//! `serve_connection` is that whole per-connection life, written once
//! against any `BufRead` and any writer; only accept, open, and teardown
//! differ per transport. `crates/wasm-api/src/live.rs`'s
//! module doc comment is the JS-facing half of this same contract.
//!
//! Unix domain sockets on macOS/Linux, a named pipe on Windows
//! (`\\.\pipe\hew-<pid>`); `crates/hew-cli/src/live.rs` is the client side
//! of both. The two transports share everything above the stream: the
//! token gate (`check_and_sanitize_hello`, `strip_token`), the
//! frame-forwarding Tauri-event shapes (`FramePayload`/`ConnPayload`/
//! `ReplyPayload`), `LiveApi`'s bookkeeping, and the per-connection
//! reader/writer loop (`serve_connection`, reached through `ConnHost` so
//! its ordering and teardown are unit-tested against a fake app). Only
//! accept, the stream itself, and teardown are written per platform, the
//! Windows half in the `mod windows` at the bottom of this file, with
//! `#[cfg(windows)]` tests of its own. These tests are the verification
//! of record: no transport's current code has been driven against the
//! real app yet.
//!
//! The Windows pipe is overlapped: every instance is created with
//! `FILE_FLAG_OVERLAPPED`, accepted with an overlapped `ConnectNamedPipe`,
//! and read and written through two `windows::PipeIo` halves, each with
//! its own `OVERLAPPED` and event, driven by `ReadFile`/`WriteFile`,
//! `WaitForSingleObject`, `GetOverlappedResult`, and `CancelIoEx`. A
//! synchronous handle serializes every I/O on it, so a writer's reply or
//! event would wait behind the reader's `ReadFile`, which waits on the
//! client's next frame; overlapped, the two run independently.
//!
//! The accept loop, every per-connection reader, and every writer run on
//! plain `std::thread`s on both platforms (each overlapped operation is
//! waited for on the thread that started it) rather than reaching for the
//! `tokio` already in
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
use std::io::BufRead;
#[cfg(unix)]
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager};

/// How long a per-connection reader waits for the webview's
/// `hew://api-reply` before giving up on that one frame: it answers a
/// synthesized internal-fault reply and closes the connection (the
/// dispatch may still be running in the webview, and replies are
/// correlated by connection, so a late one must not be handed to the next
/// request). The clock restarts whenever the webview reports
/// `hew://api-started` for the connection: when a frame queued behind a
/// plugin command is dispatched, and each time a plugin command ahead of it
/// finishes (docs/agents/PLUGINS.md "Compounds"). So a client waiting its
/// turn is timed out only by one command outlasting the clock, never by the
/// length of the queue. On a timeout the
/// window is told `hew://api-abort`, which aborts a compound held for the
/// connection's forward, so a closed socket never leaves the document
/// held. Above plugin commands' own 50 s ceiling, so a client sees a
/// plugin's timeout before this one.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// What the webview reports about a frame a connection is waiting on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyEvent {
    /// `hew://api-started`: a queued frame was just dispatched; its reply
    /// clock starts now.
    Started,
    /// `hew://api-reply`: the frame's reply.
    Reply(String),
}

/// Reads one of the two reply-channel events (`hew://api-reply`,
/// `hew://api-started`) into the connection it is for and what it says;
/// `None` for another event or a payload that does not parse.
pub(crate) fn route_reply_event(event: &str, payload: &str) -> Option<(u32, ReplyEvent)> {
    match event {
        "hew://api-reply" => {
            let p = serde_json::from_str::<ReplyPayload>(payload).ok()?;
            Some((p.conn_id, ReplyEvent::Reply(p.frame)))
        }
        "hew://api-started" => {
            let p = serde_json::from_str::<ConnIdPayload>(payload).ok()?;
            Some((p.conn_id, ReplyEvent::Started))
        }
        _ => None,
    }
}

/// Registers the reply channel's listeners: the webview calls back into
/// Rust with a plain `emit` (no Tauri command), and these are the only
/// places that ever drain `pending_replies`. A reply takes the waiting
/// slot; a start leaves it for the reply that follows.
fn listen_for_replies(app: &AppHandle) {
    for event in ["hew://api-reply", "hew://api-started"] {
        let handle = app.clone();
        app.listen_any(event, move |e| {
            let Some((conn_id, reply)) = route_reply_event(event, e.payload()) else {
                return;
            };
            let state = handle.state::<LiveApi>();
            let mut pending = state.pending_replies.lock().expect("pending_replies mutex");
            let tx = match reply {
                ReplyEvent::Started => pending.get(&conn_id).cloned(),
                ReplyEvent::Reply(_) => pending.remove(&conn_id),
            };
            drop(pending);
            if let Some(tx) = tx {
                let _ = tx.send(reply);
            }
        });
    }
}

/// Live API dispatch bookkeeping, managed as Tauri state. Session-only:
/// nothing here is persisted or fed into a document's canonical bytes.
#[derive(Default)]
pub(crate) struct LiveApi {
    pub(crate) next_conn_id: AtomicU32,
    /// The window a connection is currently attached to: its accept-time
    /// front window until the client says otherwise (`hew.doc.attach`,
    /// `hew.doc.new`/`open` — see `live_router.rs`). Never re-resolved
    /// from window focus: a user switching windows mid-session must not
    /// move an open connection's next mutation to a different document.
    /// Absent = detached (the document's window closed, §4.2).
    pub(crate) conn_window: Mutex<HashMap<u32, String>>,
    /// One slot per frame currently awaiting its `hew://api-reply` — filled
    /// just before emitting `hew://api-frame`, drained by the global reply
    /// listener registered in `start`.
    pub(crate) pending_replies: Mutex<HashMap<u32, mpsc::Sender<ReplyEvent>>>,
    /// The document registry (ids, client labels, dirty marks).
    pub(crate) docs: Mutex<crate::live_router::DocState>,
    pub(crate) next_op_id: AtomicU32,
    /// Held by `hew.doc.new`/`open`/`close` for their whole run (see
    /// `live_router::lifecycle_lock`).
    pub(crate) window_ops: Mutex<()>,
    /// Webview-side operations (`hew://api-op`) awaiting their
    /// `hew://api-op-reply`.
    pub(crate) pending_ops: Mutex<HashMap<u32, mpsc::Sender<serde_json::Value>>>,
    /// Each connection's writer: every line sent here is written to the
    /// connection whole, replies and events alike, in the order sent.
    pub(crate) outbound: Mutex<HashMap<u32, EventGate>>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FramePayload<'a> {
    pub(crate) conn_id: u32,
    pub(crate) frame: &'a str,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnPayload {
    pub(crate) conn_id: u32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyPayload {
    conn_id: u32,
    frame: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnIdPayload {
    conn_id: u32,
}

/// `hew://api-event`'s payload: one notification frame for one connection.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventPayload {
    conn_id: u32,
    frame: String,
}

/// Reads a `hew://api-event` payload into the connection it is for and
/// the frame; `None` for a payload that does not parse.
pub(crate) fn route_event_payload(payload: &str) -> Option<(u32, String)> {
    let p = serde_json::from_str::<EventPayload>(payload).ok()?;
    Some((p.conn_id, p.frame))
}

/// Registers the event listener: a notification the webview owes a
/// connection goes onto that connection's writer, between its replies. A
/// connection that has gone (or never had a writer) drops it.
fn listen_for_events(app: &AppHandle) {
    let handle = app.clone();
    app.listen_any("hew://api-event", move |e| {
        let Some((conn_id, frame)) = route_event_payload(e.payload()) else {
            return;
        };
        let state = handle.state::<LiveApi>();
        let mut outbound = state.outbound.lock().expect("outbound mutex");
        if let Some(gate) = outbound.get_mut(&conn_id) {
            gate.offer(frame);
        }
    });
}

/// The names a `hew.event.subscribe`/`unsubscribe` request left the
/// connection subscribed to, from its reply; `None` when `line` is another
/// method or the reply is a refusal.
pub(crate) fn subscribed_after(line: &str, reply: &str) -> Option<Vec<String>> {
    let request = serde_json::from_str::<serde_json::Value>(line).ok()?;
    match request.get("method")?.as_str()? {
        "hew.event.subscribe" | "hew.event.unsubscribe" => {}
        _ => return None,
    }
    let reply = serde_json::from_str::<serde_json::Value>(reply).ok()?;
    let names = reply.get("result")?.get("subscribed")?.as_array()?;
    Some(
        names
            .iter()
            .filter_map(|n| n.as_str().map(str::to_string))
            .collect(),
    )
}

/// Records what `conn_id`'s session in window `label` is now subscribed to,
/// when `line`'s `reply` says.
pub(crate) fn note_subscriptions(
    app: &AppHandle,
    conn_id: u32,
    label: &str,
    line: &str,
    reply: &str,
) {
    let Some(names) = subscribed_after(line, reply) else {
        return;
    };
    if let Some(gate) = app
        .state::<LiveApi>()
        .outbound
        .lock()
        .expect("outbound mutex")
        .get_mut(&conn_id)
    {
        gate.set_window_subscriptions(label, names);
    }
}

/// Window `label`'s session for `conn_id` ended: forget its subscriptions.
pub(crate) fn clear_window_subscriptions(app: &AppHandle, conn_id: u32, label: &str) {
    if let Some(gate) = app
        .state::<LiveApi>()
        .outbound
        .lock()
        .expect("outbound mutex")
        .get_mut(&conn_id)
    {
        gate.clear_window(label);
    }
}

/// `hew://api-broadcast`'s payload: a document lifecycle frame and the
/// connections its window already sent it through their own sessions.
#[derive(serde::Deserialize)]
struct BroadcastPayload {
    frame: String,
    except: Vec<u32>,
}

/// Reads a `hew://api-broadcast` payload into the event's name and what
/// to send; `None` for a payload that does not parse or a frame that is
/// not a `hew.event.<name>` notification.
pub(crate) fn route_broadcast_payload(payload: &str) -> Option<(String, String, Vec<u32>)> {
    let p = serde_json::from_str::<BroadcastPayload>(payload).ok()?;
    let method = serde_json::from_str::<serde_json::Value>(&p.frame)
        .ok()?
        .get("method")?
        .as_str()?
        .to_string();
    let name = method.strip_prefix("hew.event.")?.to_string();
    Some((name, p.frame, p.except))
}

/// Offers `frame` (event `name`) to every connection subscribed to it
/// except `except`: the lifecycle of one window reaches a first-party
/// listener whose session lives in another (docs/agents/HEW_API.md §4.5).
/// Every socket connection is first-party, so its grant allows the frame
/// as it is, `path` included.
pub(crate) fn broadcast_lifecycle(
    outbound: &mut HashMap<u32, EventGate>,
    name: &str,
    frame: &str,
    except: &[u32],
) {
    for (conn_id, gate) in outbound.iter_mut() {
        if !except.contains(conn_id) && gate.subscribed_to(name) {
            gate.offer(frame.to_string());
        }
    }
}

fn listen_for_broadcasts(app: &AppHandle) {
    let handle = app.clone();
    app.listen_any("hew://api-broadcast", move |e| {
        let Some((name, frame, except)) = route_broadcast_payload(e.payload()) else {
            return;
        };
        let state = handle.state::<LiveApi>();
        let mut outbound = state.outbound.lock().expect("outbound mutex");
        for gate in outbound.values_mut() {
            gate.retain_windows(|label| handle.get_webview_window(label).is_some());
        }
        broadcast_lifecycle(&mut outbound, &name, &frame, &except);
    });
}

/// How long one write to a connection may stall before the writer gives
/// up on the peer. Bounds how long a connection that stopped reading can
/// hold its threads, its socket, and its queued frames.
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// One connection's way out for events. The webview reports a frame's
/// reply and the events that frame caused on separate channels, so the
/// events could reach the writer first; while a frame awaits its reply
/// (`hold` .. `release`) events are kept here, and `release`, called once
/// the reply is queued, sends them after it.
pub(crate) struct EventGate {
    tx: mpsc::Sender<String>,
    held: Option<Vec<String>>,
    /// The event names the client has subscribed to, per window session, as
    /// each session's last `hew.event.subscribe`/`unsubscribe` reply listed
    /// them (a reply lists only its own session). A connection is
    /// subscribed to the union. Document
    /// lifecycle events of a window the connection holds no session in
    /// reach it through this (see [`broadcast_lifecycle`]).
    subscribed: HashMap<String, std::collections::HashSet<String>>,
}

impl EventGate {
    pub(crate) fn new(tx: mpsc::Sender<String>) -> EventGate {
        EventGate {
            tx,
            held: None,
            subscribed: HashMap::new(),
        }
    }

    /// Window `label`'s session now holds exactly `names`.
    pub(crate) fn set_window_subscriptions(&mut self, label: &str, names: Vec<String>) {
        self.subscribed
            .insert(label.to_string(), names.into_iter().collect());
    }

    /// Window `label`'s session ended or detached: its subscriptions stop
    /// counting.
    pub(crate) fn clear_window(&mut self, label: &str) {
        self.subscribed.remove(label);
    }

    /// Keeps only the windows `keep` accepts (closed windows drop out).
    pub(crate) fn retain_windows(&mut self, keep: impl Fn(&str) -> bool) {
        self.subscribed.retain(|label, _| keep(label));
    }

    /// Whether any of this connection's window sessions is subscribed to `name`.
    pub(crate) fn subscribed_to(&self, name: &str) -> bool {
        self.subscribed.values().any(|names| names.contains(name))
    }

    /// A frame went to the webview: hold events until its reply is queued.
    pub(crate) fn hold(&mut self) {
        self.held.get_or_insert_with(Vec::new);
    }

    /// An event for this connection: queued now, or held behind a reply.
    pub(crate) fn offer(&mut self, frame: String) {
        match &mut self.held {
            Some(held) => held.push(frame),
            None => {
                let _ = self.tx.send(frame);
            }
        }
    }

    /// The reply is queued (or the frame needed none): what was held
    /// follows it, in order, and events flow straight through again.
    pub(crate) fn release(&mut self) {
        for frame in self.held.take().unwrap_or_default() {
            let _ = self.tx.send(frame);
        }
    }
}

/// A connection's writer: the channel every reply and event for it goes
/// through, and the thread that writes them. The thread's result says
/// whether it ended cleanly (every queued line written) or on a failed
/// or stalled write.
pub(crate) type Writer = (mpsc::Sender<String>, std::thread::JoinHandle<bool>);

/// Spawns `stream`'s writer thread (see [`spawn_writer_on`]) with writes
/// bounded by `timeout`. A write that fails or stalls shuts the socket
/// down, so the connection's reader, blocked on the peer's next frame,
/// ends too.
#[cfg(unix)]
pub(crate) fn spawn_writer(
    stream: std::os::unix::net::UnixStream,
    timeout: Duration,
) -> std::io::Result<Writer> {
    stream.set_write_timeout(Some(timeout))?;
    let control = stream.try_clone()?;
    Ok(spawn_writer_on(stream, move || {
        let _ = control.shutdown(std::net::Shutdown::Both);
    }))
}

/// Spawns the writer thread over any transport: [`write_lines`] on `out`,
/// then `on_failure` when it stopped on a failed write rather than at the
/// channel's close. `on_failure` is how a transport tears itself down
/// for a peer that stopped reading: it must make the connection's
/// pending read return.
pub(crate) fn spawn_writer_on<W, F>(out: W, on_failure: F) -> Writer
where
    W: std::io::Write + Send + 'static,
    F: FnOnce() + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<String>();
    let writer = std::thread::spawn(move || {
        let clean = write_lines(rx, out);
        if !clean {
            on_failure();
        }
        clean
    });
    (tx, writer)
}

/// A connection's writer thread: writes each line it receives, newline
/// framed, until the channel closes (every sender dropped, once the queue
/// is empty: `true`) or a write fails (`false`).
pub(crate) fn write_lines(rx: mpsc::Receiver<String>, mut out: impl std::io::Write) -> bool {
    for line in rx {
        let mut framed = line;
        framed.push('\n');
        if out.write_all(framed.as_bytes()).is_err() || out.flush().is_err() {
            return false;
        }
    }
    true
}

/// The next frame a connection sent: one line, trailing whitespace (a
/// CRLF's `\r` included) trimmed, blank lines skipped, and any
/// `params.token` stripped (see [`strip_token`]). `None` at the peer's
/// close, on a read error (a transport torn down by its writer included),
/// or on a line that is not UTF-8: each ends the connection.
pub(crate) fn next_frame(reader: &mut impl BufRead) -> Option<String> {
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            // The token gate runs once, on the first frame, but a later
            // frame (a second `hello`, or anything else) may still CARRY
            // a token field, and the webview must never see the secret.
            return Some(strip_token(trimmed));
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpReplyPayload {
    op_id: u32,
    #[serde(flatten)]
    outcome: serde_json::Value,
}

/// Hands one webview operation's answer to the thread waiting on it.
fn deliver_op_reply(app: &AppHandle, event_payload: &str) {
    let Ok(payload) = serde_json::from_str::<OpReplyPayload>(event_payload) else {
        return;
    };
    let tx = app
        .state::<LiveApi>()
        .pending_ops
        .lock()
        .expect("pending_ops mutex")
        .remove(&payload.op_id);
    if let Some(tx) = tx {
        let _ = tx.send(payload.outcome);
    }
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
        version: crate::version::VERSION,
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

    // The reply channel (see `listen_for_replies`).
    listen_for_replies(app);
    listen_for_events(app);
    listen_for_broadcasts(app);

    let op_handle = app.clone();
    app.listen_any("hew://api-op-reply", move |event| {
        deliver_op_reply(&op_handle, event.payload());
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

/// One accepted unix connection: [`serve_connection`] over the socket,
/// then shut it down.
#[cfg(unix)]
fn handle_connection(
    app: AppHandle,
    stream: std::os::unix::net::UnixStream,
    token: String,
    window_label: String,
) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let _ = serve_connection(
        &app,
        BufReader::new(read_half),
        &token,
        &window_label,
        || spawn_writer(stream.try_clone()?, WRITE_TIMEOUT),
    );
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// One accepted connection's whole lifetime, on any transport: the
/// mandatory `hello`+token handshake (§11.2 — anything else on the first
/// frame is dropped silently, no bytes written back), then the
/// frame-at-a-time loop until the peer closes, the reader fails, or a
/// write fails. Routing — which window a frame goes to — is
/// `live_router`'s; this only reads frames from `reader` and hands every
/// outgoing line to the one writer `open_writer` starts, after the
/// handshake passed.
///
/// Returns once both halves are done: the reader stopped and the writer
/// thread was joined (it finishes what was queued, a timeout's
/// synthesized reply included, within `WRITE_TIMEOUT` of a peer that
/// stopped reading). `None` when the connection never got a writer (the
/// handshake failed, or the writer could not start); otherwise whether the
/// writer ended cleanly, every queued line written. The caller owns the
/// transport's own teardown.
fn serve_connection<H: ConnHost>(
    host: &H,
    mut reader: impl BufRead,
    token: &str,
    window_label: &str,
    open_writer: impl FnOnce() -> std::io::Result<Writer>,
) -> Option<bool> {
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).unwrap_or(0) == 0 {
        return None; // closed before sending anything
    }
    // Wrong method, missing/wrong token, or malformed — drop silently.
    let sanitized_hello = check_and_sanitize_hello(&first_line, token)?;

    // One writer per connection: replies and events go through the same
    // channel, so a frame is never split and events land between replies.
    // Opened before the connection is registered, so a transport that
    // cannot start one (a failed socket clone) tells the webview nothing.
    let (tx, writer) = open_writer().ok()?;
    let (conn_id, mut route) = host.register(window_label);
    host.gates().insert(conn_id, EventGate::new(tx.clone()));
    let mut write = |out: &str| -> bool { tx.send(out.to_string()).is_ok() };
    // Every frame holds this connection's events until its reply is queued.
    let gate = |f: fn(&mut EventGate)| {
        if let Some(g) = host.gates().get_mut(&conn_id) {
            f(g);
        }
    };
    gate(EventGate::hold);
    let mut ok = host.serve(&mut route, &sanitized_hello, &mut write);
    gate(EventGate::release);
    while ok {
        let Some(frame) = next_frame(&mut reader) else {
            break; // peer closed / read error / transport torn down
        };
        gate(EventGate::hold);
        ok = host.serve(&mut route, &frame, &mut write);
        gate(EventGate::release);
    }

    // Unregistering drops the connection's gate, and with it the gate's
    // sender; then the last sender goes and the writer finishes what was
    // queued and exits.
    host.unregister(conn_id, &mut route);
    drop(tx);
    drop(reader);
    Some(writer.join().unwrap_or(false))
}

/// What [`serve_connection`] needs from the app: the app itself
/// (`AppHandle`), or a fake in tests, so the connection loop's ordering
/// and teardown are tested without a webview.
trait ConnHost {
    type Route;
    /// Registers a connection bound to `window_label`.
    fn register(&self, window_label: &str) -> (u32, Self::Route);
    /// Every connection's event gate (`LiveApi::outbound`).
    fn gates(&self) -> std::sync::MutexGuard<'_, HashMap<u32, EventGate>>;
    /// Routes one frame; `false` ends the connection (see [`serve_frame`]).
    fn serve(
        &self,
        route: &mut Self::Route,
        frame: &str,
        write: &mut dyn FnMut(&str) -> bool,
    ) -> bool;
    /// Forgets the connection, its gate included.
    fn unregister(&self, conn_id: u32, route: &mut Self::Route);
}

impl ConnHost for AppHandle {
    type Route = crate::live_router::ConnRoute;
    fn register(&self, window_label: &str) -> (u32, Self::Route) {
        register_connection(self, window_label)
    }
    fn gates(&self) -> std::sync::MutexGuard<'_, HashMap<u32, EventGate>> {
        self.state::<LiveApi>()
            .inner()
            .outbound
            .lock()
            .expect("outbound mutex")
    }
    fn serve(
        &self,
        route: &mut Self::Route,
        frame: &str,
        write: &mut dyn FnMut(&str) -> bool,
    ) -> bool {
        serve_frame(self, route, frame, write)
    }
    fn unregister(&self, conn_id: u32, route: &mut Self::Route) {
        unregister_connection(self, conn_id, route);
    }
}

/// Registers a freshly accepted connection: binds it to `window_label` (the
/// front window at accept time) and opens its session there.
fn register_connection(
    app: &AppHandle,
    window_label: &str,
) -> (u32, crate::live_router::ConnRoute) {
    let state = app.state::<LiveApi>();
    let id = state.next_conn_id.fetch_add(1, Ordering::Relaxed);
    state
        .conn_window
        .lock()
        .expect("conn_window mutex")
        .insert(id, window_label.to_string());
    let _ = app.emit_to(
        window_label,
        "hew://api-connection-open",
        ConnPayload { conn_id: id },
    );
    (id, crate::live_router::ConnRoute::new(id, window_label))
}

/// Forgets a finished connection and closes its session in every window it
/// touched.
fn unregister_connection(app: &AppHandle, conn_id: u32, route: &mut crate::live_router::ConnRoute) {
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
    state
        .outbound
        .lock()
        .expect("outbound mutex")
        .remove(&conn_id);
    route.close_sessions(app);
}

/// Routes one frame (`live_router::handle_frame`) and writes whatever
/// comes back through `write`. Returns `false` when the connection should
/// end: a failed write, a hard transport failure, or a reply timeout
/// (whose synthesized answer is written first — the dispatch is still
/// running in the webview, replies are correlated by connection, and a
/// late one must not be handed to the NEXT request).
fn serve_frame(
    app: &AppHandle,
    route: &mut crate::live_router::ConnRoute,
    line: &str,
    write: &mut dyn FnMut(&str) -> bool,
) -> bool {
    use crate::live_router::Outcome;
    match crate::live_router::handle_frame(app, route, line) {
        Outcome::NoReply => true,
        Outcome::Reply(reply) => write(&reply),
        Outcome::ReplyThenClose(reply) => {
            let _ = write(&reply);
            false
        }
        Outcome::Close => false,
    }
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
/// this reply (see `serve_frame`).
pub(crate) fn synthesize_timeout_reply(request: &str) -> String {
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
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_IO_PENDING,
        ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, FALSE, HANDLE, INVALID_HANDLE_VALUE,
        STILL_ACTIVE, TRUE, WAIT_OBJECT_0,
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
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, INVALID_FILE_ATTRIBUTES,
        PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetExitCodeProcess, OpenProcess, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

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
    /// pointer.
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
            version: crate::version::VERSION,
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
    ///
    /// Every instance is overlapped (`FILE_FLAG_OVERLAPPED`): see
    /// [`PipeIo`] for why a synchronous handle cannot carry events.
    fn create_pipe_instance(name: &[u16], first: bool) -> std::io::Result<HANDLE> {
        let (sa, guard) = owner_only_security_attributes()?;
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
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

        // The reply channel: identical to unix's, whichever transport is
        // forwarding frames.
        super::listen_for_replies(app);
        super::listen_for_events(app);
        super::listen_for_broadcasts(app);
        let op_handle = app.clone();
        app.listen_any("hew://api-op-reply", move |event| {
            deliver_op_reply(&op_handle, event.payload());
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
    /// The NEXT instance is created before this one is connected, so an
    /// unconnected instance is waiting for the whole time the current one
    /// is in use: a pipe instance is connectable from the moment
    /// `CreateNamedPipeW` returns it, and a name with zero waiting
    /// instances turns a racing client's `CreateFileW` from
    /// `ERROR_PIPE_BUSY` (which it retries via `WaitNamedPipeW`) into
    /// `ERROR_FILE_NOT_FOUND` (which, correctly, it does NOT retry — see
    /// `crates/hew-cli/src/live.rs`'s `RawConn::connect`).
    ///
    /// Creating an instance can fail transiently (resource exhaustion).
    /// The loop never gives up on that — the discovery file keeps
    /// advertising this pipe, so stopping would strand every later
    /// client: a failed pre-create is tried again right after the current
    /// instance connects, before it is handed off, and a failed create of
    /// the current instance is retried with a backoff up to
    /// `ACCEPT_RETRY_MAX`. Only while those retries keep failing does the
    /// name have no waiting instance.
    fn spawn_accept_loop(
        app: AppHandle,
        name: String,
        token: String,
        first_handle: SendableHandle,
    ) {
        const ACCEPT_RETRY_MIN: Duration = Duration::from_millis(100);
        const ACCEPT_RETRY_MAX: Duration = Duration::from_secs(5);
        std::thread::spawn(move || {
            let name_w = wide(&name);
            let mut backoff = ACCEPT_RETRY_MIN;
            // The accept loop's own event for its overlapped
            // `ConnectNamedPipe`; lives as long as the loop. The first
            // instance already exists, so clients queue on it meanwhile.
            let accept_event = loop {
                match Event::new() {
                    Ok(e) => break e,
                    Err(e) => {
                        eprintln!(
                            "hew: live API accept loop — could not create an event (retrying in {backoff:?}): {e}"
                        );
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(ACCEPT_RETRY_MAX);
                    }
                }
            };
            backoff = ACCEPT_RETRY_MIN;
            let mut waiting = Some(first_handle);
            loop {
                let handle = match waiting.take() {
                    Some(h) => h,
                    None => match create_pipe_instance(&name_w, false) {
                        Ok(h) => SendableHandle(h),
                        Err(e) => {
                            eprintln!(
                                "hew: live API accept loop — could not create a pipe instance (retrying in {backoff:?}): {e}"
                            );
                            std::thread::sleep(backoff);
                            backoff = (backoff * 2).min(ACCEPT_RETRY_MAX);
                            continue;
                        }
                    },
                };
                backoff = ACCEPT_RETRY_MIN;

                let pre_create = || match create_pipe_instance(&name_w, false) {
                    Ok(h) => Some(SendableHandle(h)),
                    Err(e) => {
                        eprintln!(
                            "hew: live API accept loop — could not pre-create the next pipe instance (retrying): {e}"
                        );
                        None
                    }
                };
                let mut next_handle = pre_create();
                let connected = connect_client(handle, &accept_event);
                if next_handle.is_none() {
                    // The current instance is about to stop waiting: try
                    // again now, so no client meets a name with no
                    // instance while the next one could exist.
                    next_handle = pre_create();
                }

                if connected {
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

                waiting = next_handle;
            }
        });
    }

    /// Waits for a client on `handle`, an overlapped instance: an
    /// overlapped `ConnectNamedPipe` waited on with `event`. `true` when a
    /// client is connected, `ERROR_PIPE_CONNECTED` included: a client that
    /// connected between this instance's `CreateNamedPipeW` and this call is
    /// a benign, documented Win32 race, not a failure.
    fn connect_client(handle: SendableHandle, event: &Event) -> bool {
        // SAFETY: OVERLAPPED is plain data; all-zero is its documented
        // initial state.
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = event.0;
        if unsafe { ConnectNamedPipe(handle.0, &mut ov) } != 0 {
            return true;
        }
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => true,
            ERROR_IO_PENDING => {
                // Wait for the client: `ov` and the event must outlive the
                // pending operation, and this does.
                let mut n: u32 = 0;
                unsafe { GetOverlappedResult(handle.0, &ov, &mut n, TRUE) != 0 }
            }
            _ => false,
        }
    }

    /// One accepted connection: [`serve_connection`] over the pipe, read
    /// and written through two [`PipeIo`] halves, then the pipe's own
    /// teardown.
    fn handle_connection(
        app: AppHandle,
        handle: SendableHandle,
        token: String,
        window_label: String,
    ) {
        let pipe = Arc::new(Pipe::new(handle));
        let outcome = match PipeIo::new(pipe.clone(), None) {
            Ok(read_half) => serve_connection(
                &app,
                std::io::BufReader::new(read_half),
                &token,
                &window_label,
                || {
                    let write_half = PipeIo::new(pipe.clone(), Some(WRITE_TIMEOUT))?;
                    let control = pipe.clone();
                    Ok(spawn_writer_on(write_half, move || control.abort()))
                },
            ),
            Err(_) => None,
        };
        // Both halves are done here: the reader returned and the writer was
        // joined, so no I/O is pending on the handle.
        pipe.disconnect(outcome == Some(true));
    }

    /// A manual-reset event for one overlapped operation at a time, closed
    /// on drop.
    struct Event(HANDLE);

    // SAFETY: an event handle names a process-wide kernel object, usable
    // from any thread.
    unsafe impl Send for Event {}

    impl Event {
        fn new() -> std::io::Result<Event> {
            // Manual reset: `GetOverlappedResult` waits on it, and every
            // overlapped `ReadFile`/`WriteFile`/`ConnectNamedPipe` resets it
            // when the operation starts.
            let event = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, ptr::null()) };
            if event.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Event(event))
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    /// One connected pipe instance, shared by its reader, its writer, and
    /// the writer's failure hook; the last owner closes the handle.
    struct Pipe {
        handle: HANDLE,
        /// Set once by [`Pipe::abort`]: no new I/O starts, and any I/O that
        /// started anyway is cancelled (see [`PipeIo::run`]).
        aborted: AtomicBool,
    }

    // SAFETY: the pipe handle names a process-wide kernel object;
    // overlapped ReadFile/WriteFile/CancelIoEx on it are documented safe
    // from any thread, each operation with its own OVERLAPPED and event.
    unsafe impl Send for Pipe {}
    unsafe impl Sync for Pipe {}

    impl Pipe {
        fn new(handle: SendableHandle) -> Pipe {
            Pipe {
                handle: handle.0,
                aborted: AtomicBool::new(false),
            }
        }

        /// Tears the connection down from the writer's side: every read or
        /// write pending on the handle, from any thread, completes with
        /// `ERROR_OPERATION_ABORTED`, and none starts after. The writer
        /// calls this when a write failed or stalled past `WRITE_TIMEOUT`,
        /// so the reader, waiting on the peer's next frame, ends too.
        fn abort(&self) {
            self.aborted.store(true, Ordering::SeqCst);
            unsafe { CancelIoEx(self.handle, ptr::null()) };
        }

        /// The end of a connection, once nothing is pending on the handle.
        /// `flush` waits for the client to read what was written before
        /// `DisconnectNamedPipe`, which discards unread bytes (unlike
        /// closing a unix socket): without it, the synthesized reply a
        /// reply timeout writes right before closing could be destroyed
        /// before the client sees it. It is passed only when the writer
        /// ended cleanly: a writer that stalled already gave up on a peer
        /// that stopped reading, and flushing to that peer would block
        /// forever. The residual: a client that stops reading in the last
        /// moments, with a clean writer, can still hold this one
        /// connection's thread in the flush; no other connection or the app
        /// waits on it.
        fn disconnect(&self, flush: bool) {
            unsafe {
                if flush {
                    FlushFileBuffers(self.handle);
                }
                DisconnectNamedPipe(self.handle);
            }
        }
    }

    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// One direction of a connected pipe as `Read` or `Write`: the reader
    /// and the writer each own one, with its own `OVERLAPPED` and event.
    ///
    /// Why overlapped: a synchronous pipe handle serializes every I/O on
    /// it, so a writer thread's reply or event would wait behind the
    /// reader thread's `ReadFile`, which waits on the client's next frame,
    /// possibly forever (an `events --live` client never sends one). With
    /// `FILE_FLAG_OVERLAPPED` the two run independently. Each call starts
    /// one operation and waits for it to finish, so from the caller's side
    /// it is a plain blocking read or write, and the buffer and the
    /// `OVERLAPPED` are never released while the operation is in flight:
    /// a timed-out or aborted operation is cancelled and then waited for
    /// before the call returns.
    struct PipeIo {
        pipe: Arc<Pipe>,
        event: Event,
        /// A write's bound (`WRITE_TIMEOUT`); `None` waits as long as the
        /// operation takes (a read waits on the client's next frame, as
        /// unix's does).
        timeout: Option<Duration>,
    }

    impl PipeIo {
        fn new(pipe: Arc<Pipe>, timeout: Option<Duration>) -> std::io::Result<PipeIo> {
            Ok(PipeIo {
                pipe,
                event: Event::new()?,
                timeout,
            })
        }

        /// Starts one overlapped operation through `start` and waits for
        /// it: bytes transferred, or the error it ended with. A timeout
        /// cancels it and answers `TimedOut`; an aborted pipe answers
        /// `ConnectionAborted`.
        fn run(
            &mut self,
            start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> i32,
        ) -> std::io::Result<u32> {
            let handle = self.pipe.handle;
            if self.pipe.aborted.load(Ordering::SeqCst) {
                return Err(std::io::ErrorKind::ConnectionAborted.into());
            }
            // SAFETY: OVERLAPPED is plain data; all-zero is its documented
            // initial state. It stays on this frame until the operation is
            // finished (below), so its address is stable for the kernel.
            let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
            ov.hEvent = self.event.0;
            // TRUE: the operation already finished, so there is nothing to
            // wait for or cancel; the event is only waited on for an
            // operation that is still pending (see the wait below).
            let pending = if start(handle, &mut ov) == 0 {
                let err = unsafe { GetLastError() };
                if err != ERROR_IO_PENDING {
                    return Err(std::io::Error::from_raw_os_error(err as i32));
                }
                true
            } else {
                false
            };
            // An abort that raced the start (it ran between the check
            // above and the operation reaching the kernel) is caught here:
            // `abort` sets the flag before it cancels, so either its
            // `CancelIoEx` saw this operation or this load sees the flag.
            let mut cancelled = false;
            if pending && self.pipe.aborted.load(Ordering::SeqCst) {
                unsafe { CancelIoEx(handle, &ov) };
                cancelled = true;
            }
            let mut timed_out = false;
            if let (Some(timeout), true, false) = (self.timeout, pending, cancelled) {
                let ms = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
                if unsafe { WaitForSingleObject(self.event.0, ms) } != WAIT_OBJECT_0 {
                    unsafe { CancelIoEx(handle, &ov) };
                    timed_out = true;
                }
            }
            // Waits for the operation to finish, cancelled or not; after
            // this the kernel no longer touches `ov` or the buffer. For a
            // finished operation it returns at once: it reads the status
            // from `ov` and waits on the event only while that status is
            // still pending. (The event is signalled on completion either
            // way; only `FILE_SKIP_SET_EVENT_ON_HANDLE`, never set here,
            // changes signalling, and that one affects the file handle,
            // not `hEvent`. Skipping the wait for a synchronous completion
            // keeps this correct without relying on it.)
            let mut n: u32 = 0;
            if unsafe { GetOverlappedResult(handle, &ov, &mut n, TRUE) } != 0 {
                // Finished before the cancel landed: the bytes count.
                return Ok(n);
            }
            let err = unsafe { GetLastError() };
            if err == ERROR_OPERATION_ABORTED {
                if timed_out {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                return Err(std::io::ErrorKind::ConnectionAborted.into());
            }
            Err(std::io::Error::from_raw_os_error(err as i32))
        }
    }

    impl std::io::Read for PipeIo {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let len = buf.len().min(u32::MAX as usize) as u32;
            let ptr = buf.as_mut_ptr();
            match self.run(|handle, ov| unsafe { ReadFile(handle, ptr, len, ptr::null_mut(), ov) })
            {
                Ok(n) => Ok(n as usize),
                // The client closed its end: end of stream, as a 0-byte
                // unix read.
                Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
                Err(e) => Err(e),
            }
        }
    }

    /// The most one overlapped `WriteFile` carries. A pipe write finishes
    /// only once the client has taken its bytes, so the write bound is per
    /// chunk: like unix's per-`write` socket timeout, it trips on a client
    /// that stopped reading, not on one still reading a large frame.
    const WRITE_CHUNK: usize = 64 * 1024;

    impl std::io::Write for PipeIo {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let len = buf.len().min(WRITE_CHUNK) as u32;
            let ptr = buf.as_ptr();
            self.run(|handle, ov| unsafe { WriteFile(handle, ptr, len, ptr::null_mut(), ov) })
                .map(|n| n as usize)
        }

        /// Every write has finished by the time it returns; nothing is
        /// buffered here.
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
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

        // ------------------------------------------- overlapped pipe I/O

        /// A connected pair over a fresh overlapped instance: the server
        /// end as `create_pipe_instance` and `connect_client` make it, the
        /// client end opened overlapped the way `hew-cli` opens it.
        fn pipe_pair(tag: &str) -> (Arc<Pipe>, Arc<Pipe>) {
            use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
            use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
            let name = format!(
                r"\\.\pipe\hew-live-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let name_w = wide(&name);
            let server = SendableHandle(create_pipe_instance(&name_w, true).unwrap());
            let accept = std::thread::spawn(move || {
                let server = server;
                let event = Event::new().unwrap();
                assert!(connect_client(server, &event));
                server
            });
            let client = unsafe {
                CreateFileW(
                    name_w.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    ptr::null_mut(),
                )
            };
            assert_ne!(client, INVALID_HANDLE_VALUE, "the client end opens");
            let server = accept.join().unwrap();
            (
                Arc::new(Pipe::new(server)),
                Arc::new(Pipe::new(SendableHandle(client))),
            )
        }

        /// Runs `f` on a thread and waits at most `bound` for it.
        fn within<T: Send + 'static>(
            bound: Duration,
            f: impl FnOnce() -> T + Send + 'static,
        ) -> Option<T> {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(f());
            });
            rx.recv_timeout(bound).ok()
        }

        /// A thread waiting on `server`'s next frame.
        fn pending_reader(server: &Arc<Pipe>) -> std::thread::JoinHandle<Option<String>> {
            let pipe = server.clone();
            std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(PipeIo::new(pipe, None).unwrap());
                next_frame(&mut reader)
            })
        }

        /// The reason for overlapped I/O: while the reader waits on the
        /// client's next frame, the writer's frames still go out, whole and
        /// in order, and the reader then gets the client's frame.
        #[test]
        fn a_pending_read_does_not_hold_up_the_writer() {
            let (server, client) = pipe_pair("pending-read");
            let reader = pending_reader(&server);
            std::thread::sleep(Duration::from_millis(100)); // the read is pending
            let (tx, writer) = spawn_writer_on(
                PipeIo::new(server.clone(), Some(Duration::from_secs(5))).unwrap(),
                || {},
            );
            tx.send(r#"{"id":1,"result":{}}"#.into()).unwrap();
            tx.send(r#"{"method":"hew.event.units_changed"}"#.into())
                .unwrap();
            let client_reader = client.clone();
            let got = within(Duration::from_secs(5), move || {
                let mut r = std::io::BufReader::new(PipeIo::new(client_reader, None).unwrap());
                let mut lines = Vec::new();
                for _ in 0..2 {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    lines.push(line);
                }
                lines
            })
            .expect("the writer's frames arrive while a read is pending");
            assert_eq!(
                got,
                vec![
                    "{\"id\":1,\"result\":{}}\n".to_string(),
                    "{\"method\":\"hew.event.units_changed\"}\n".to_string()
                ]
            );
            let mut client_writer = PipeIo::new(client.clone(), None).unwrap();
            std::io::Write::write_all(&mut client_writer, b"{\"id\":2}\r\n").unwrap();
            assert_eq!(reader.join().unwrap().as_deref(), Some("{\"id\":2}"));
            drop(tx);
            assert!(writer.join().unwrap(), "a drained writer ends cleanly");
        }

        /// `abort` cancels a read pending on another thread, and no I/O
        /// starts after it.
        #[test]
        fn abort_ends_a_pending_read() {
            let (server, _client) = pipe_pair("abort");
            let reader = pending_reader(&server);
            std::thread::sleep(Duration::from_millis(100));
            server.abort();
            let ended = within(Duration::from_secs(5), move || reader.join().unwrap());
            assert_eq!(ended, Some(None), "the pending read ends on abort");
            let mut late = PipeIo::new(server, None).unwrap();
            let err = std::io::Read::read(&mut late, &mut [0u8; 8]).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::ConnectionAborted);
        }

        /// A client that stops reading: the write stalls past its bound,
        /// the writer ends unclean, and its failure hook ends the reader.
        #[test]
        fn a_peer_that_stops_reading_ends_the_writer_and_the_reader() {
            let (server, _client) = pipe_pair("stalled");
            let reader = pending_reader(&server);
            let control = server.clone();
            let (tx, writer) = spawn_writer_on(
                PipeIo::new(server.clone(), Some(Duration::from_millis(100))).unwrap(),
                move || control.abort(),
            );
            let big = "x".repeat(1 << 20);
            for _ in 0..4 {
                let _ = tx.send(big.clone());
            }
            drop(tx);
            let clean = within(Duration::from_secs(5), move || writer.join().unwrap());
            assert_eq!(clean, Some(false), "the stalled write gives up in time");
            let ended = within(Duration::from_secs(5), move || reader.join().unwrap());
            assert_eq!(ended, Some(None), "the reader ends with the writer");
        }

        /// The client closing its end is the end of the stream.
        #[test]
        fn the_client_closing_is_end_of_stream() {
            let (server, client) = pipe_pair("close");
            let reader = pending_reader(&server);
            std::thread::sleep(Duration::from_millis(100));
            drop(client);
            let ended = within(Duration::from_secs(5), move || reader.join().unwrap());
            assert_eq!(ended, Some(None));
            server.disconnect(false);
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
    fn reply_events_route_to_their_connection() {
        assert_eq!(
            route_reply_event("hew://api-reply", r#"{"connId":4,"frame":"{}"}"#),
            Some((4, ReplyEvent::Reply("{}".into())))
        );
        assert_eq!(
            route_reply_event("hew://api-started", r#"{"connId":9}"#),
            Some((9, ReplyEvent::Started))
        );
        assert_eq!(route_reply_event("hew://api-started", "nope"), None);
        assert_eq!(
            route_reply_event("hew://api-reply", r#"{"connId":4}"#),
            None
        );
        assert_eq!(
            route_reply_event("hew://api-queued", r#"{"connId":4}"#),
            None
        );
    }

    #[test]
    fn event_payloads_route_to_their_connection() {
        assert_eq!(
            route_event_payload(r#"{"connId":3,"frame":"{\"jsonrpc\":\"2.0\"}"}"#),
            Some((3, r#"{"jsonrpc":"2.0"}"#.to_string()))
        );
        assert_eq!(route_event_payload(r#"{"connId":3}"#), None);
        assert_eq!(route_event_payload("nope"), None);
    }

    #[test]
    fn subscriptions_are_read_from_the_reply() {
        let sub = r#"{"jsonrpc":"2.0","id":2,"method":"hew.event.subscribe","params":{"events":["document_opened"]}}"#;
        let ok = r#"{"jsonrpc":"2.0","id":2,"result":{"subscribed":["document_closed","document_opened"],"denied":[]}}"#;
        assert_eq!(
            subscribed_after(sub, ok),
            Some(vec![
                "document_closed".to_string(),
                "document_opened".to_string()
            ])
        );
        let bad =
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"unknown event name: x"}}"#;
        assert_eq!(subscribed_after(sub, bad), None);
        let other = r#"{"jsonrpc":"2.0","id":3,"method":"hew.meta.hello","params":{}}"#;
        assert_eq!(subscribed_after(other, ok), None);
    }

    #[test]
    fn a_lifecycle_broadcast_reaches_subscribers_not_already_served() {
        let frame = r#"{"jsonrpc":"2.0","method":"hew.event.document_opened","params":{"document":"doc_2"}}"#;
        let payload = serde_json::json!({ "frame": frame, "except": [2] }).to_string();
        let (name, f, except) = route_broadcast_payload(&payload).unwrap();
        assert_eq!(name, "document_opened");
        assert_eq!(f, frame);
        assert_eq!(
            route_broadcast_payload(r#"{"frame":"{}","except":[]}"#),
            None
        );
        assert_eq!(route_broadcast_payload("nope"), None);

        let mut outbound = HashMap::new();
        let mut rxs = Vec::new();
        for (id, subs) in [
            (1u32, vec!["document_opened"]),
            (2, vec!["document_opened"]),
            (3, vec!["document_closed"]),
            (4, vec![]),
        ] {
            let (tx, rx) = mpsc::channel();
            let mut gate = EventGate::new(tx);
            gate.set_window_subscriptions("A", subs.into_iter().map(String::from).collect());
            outbound.insert(id, gate);
            rxs.push((id, rx));
        }
        broadcast_lifecycle(&mut outbound, &name, &f, &except);
        for (id, rx) in rxs {
            let got: Vec<String> = rx.try_iter().collect();
            match id {
                1 => assert_eq!(got, vec![frame.to_string()]),
                _ => assert!(got.is_empty(), "conn {id} got {got:?}"),
            }
        }
    }

    #[test]
    fn subscriptions_are_kept_per_window_and_unioned() {
        let (tx, _rx) = mpsc::channel();
        let mut gate = EventGate::new(tx);
        // document_opened in window A, selection_changed in window B: the
        // connection is subscribed to both.
        gate.set_window_subscriptions("A", vec!["document_opened".to_string()]);
        gate.set_window_subscriptions("B", vec!["selection_changed".to_string()]);
        assert!(gate.subscribed_to("document_opened"));
        assert!(gate.subscribed_to("selection_changed"));
        // B's later reply replaces only B's set.
        gate.set_window_subscriptions("B", vec![]);
        assert!(gate.subscribed_to("document_opened"));
        assert!(!gate.subscribed_to("selection_changed"));
        // Re-attaching A with a different set replaces A's, nothing else.
        gate.set_window_subscriptions("B", vec!["document_closed".to_string()]);
        gate.set_window_subscriptions("A", vec!["tool_changed".to_string()]);
        assert!(!gate.subscribed_to("document_opened"));
        assert!(gate.subscribed_to("document_closed"));
        // A closed session or detached window stops counting.
        gate.clear_window("B");
        assert!(!gate.subscribed_to("document_closed"));
        assert!(gate.subscribed_to("tool_changed"));
        // Windows that no longer exist are dropped.
        gate.retain_windows(|label| label != "A");
        assert!(!gate.subscribed_to("tool_changed"));
    }

    #[test]
    fn a_broadcast_follows_the_union_across_windows() {
        let frame = r#"{"jsonrpc":"2.0","method":"hew.event.document_opened","params":{}}"#;
        let (tx, rx) = mpsc::channel();
        let mut gate = EventGate::new(tx);
        gate.set_window_subscriptions("A", vec!["document_opened".to_string()]);
        gate.set_window_subscriptions("B", vec!["selection_changed".to_string()]);
        let mut outbound = HashMap::from([(1u32, gate)]);
        broadcast_lifecycle(&mut outbound, "document_opened", frame, &[]);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![frame.to_string()]);
    }

    /// The writer writes each line whole, newline framed, in send order,
    /// and finishes the queue after the last sender goes.
    #[test]
    fn write_lines_frames_each_line_whole_and_drains_on_close() {
        let (tx, rx) = mpsc::channel::<String>();
        let out = std::sync::Arc::new(Mutex::new(Vec::<u8>::new()));
        struct Shared(std::sync::Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let sink = Shared(out.clone());
        let event_tx = tx.clone();
        tx.send(r#"{"id":1,"result":{}}"#.into()).unwrap();
        event_tx
            .send(r#"{"method":"hew.event.units_changed"}"#.into())
            .unwrap();
        tx.send(r#"{"id":2,"result":{}}"#.into()).unwrap();
        drop(tx);
        drop(event_tx);
        write_lines(rx, sink);
        assert_eq!(
            String::from_utf8(out.lock().unwrap().clone()).unwrap(),
            "{\"id\":1,\"result\":{}}\n{\"method\":\"hew.event.units_changed\"}\n{\"id\":2,\"result\":{}}\n"
        );
    }

    /// Events offered while a frame awaits its reply follow that reply;
    /// between frames they go straight out.
    #[test]
    fn events_wait_behind_the_reply_their_frame_is_owed() {
        let (tx, rx) = mpsc::channel::<String>();
        let mut gate = EventGate::new(tx.clone());
        gate.offer("before".into());
        gate.hold();
        gate.offer("caused".into());
        tx.send("reply".into()).unwrap();
        gate.release();
        gate.offer("after".into());
        drop(gate);
        drop(tx);
        assert_eq!(
            rx.iter().collect::<Vec<_>>(),
            vec!["before", "reply", "caused", "after"]
        );
    }

    /// A peer that stops reading cannot hold the writer: a stalled write
    /// gives up after the write timeout, so joining it is bounded.
    #[cfg(unix)]
    #[test]
    fn a_stalled_peer_releases_the_writer_within_the_timeout() {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let (tx, writer) = spawn_writer(ours, Duration::from_millis(100)).unwrap();
        let big = "x".repeat(1 << 20);
        for _ in 0..16 {
            let _ = tx.send(big.clone());
        }
        drop(tx);
        let started = std::time::Instant::now();
        writer.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A failed write ends the writer, so the connection's sends start
    /// failing and its reader stops.
    #[test]
    fn write_lines_stops_on_a_failed_write() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (tx, rx) = mpsc::channel::<String>();
        let writer = std::thread::spawn(move || write_lines(rx, Broken));
        tx.send("a".into()).unwrap();
        writer.join().unwrap();
        assert!(tx.send("b".into()).is_err());
    }

    /// The writer's failure hook runs when a write fails, never when the
    /// queue drains cleanly; the thread's result says which it was.
    #[test]
    fn spawn_writer_on_runs_the_failure_hook_only_on_a_failed_write() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let hook = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let h = hook.clone();
        let (tx, writer) = spawn_writer_on(Broken, move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        tx.send("a".into()).unwrap();
        assert!(!writer.join().unwrap());
        assert_eq!(hook.load(Ordering::SeqCst), 1);

        let h = hook.clone();
        let (tx, writer) = spawn_writer_on(Vec::<u8>::new(), move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        tx.send("a".into()).unwrap();
        drop(tx);
        assert!(writer.join().unwrap());
        assert_eq!(hook.load(Ordering::SeqCst), 1, "a clean drain runs no hook");
    }

    /// Frames come out trimmed (CRLF too), blank lines skipped, tokens
    /// stripped; the stream's end, a read error, and a non-UTF-8 line each
    /// end it.
    #[test]
    fn next_frame_reads_one_clean_frame_at_a_time() {
        let input =
            b"\r\n\n{\"id\":1}\r\n  \n{\"id\":2,\"params\":{\"token\":\"secret\"}}\n{\"id\":3}";
        let mut reader = std::io::Cursor::new(&input[..]);
        assert_eq!(next_frame(&mut reader).as_deref(), Some("{\"id\":1}"));
        let second = next_frame(&mut reader).unwrap();
        assert!(!second.contains("secret"), "{second}");
        // A last line without its newline is still a frame, as unix's
        // `read_line` gives it.
        assert_eq!(next_frame(&mut reader).as_deref(), Some("{\"id\":3}"));
        assert_eq!(next_frame(&mut reader), None);

        let mut bad = std::io::Cursor::new(&b"\xff\xfe\n{\"id\":4}\n"[..]);
        assert_eq!(next_frame(&mut bad), None);

        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::ConnectionAborted.into())
            }
        }
        assert_eq!(next_frame(&mut std::io::BufReader::new(Failing)), None);
    }

    /// A peer that stops reading ends the whole connection, not just the
    /// writer: the stalled writer shuts the socket down, so the reader
    /// waiting on the peer's next frame returns too.
    #[cfg(unix)]
    #[test]
    fn a_stalled_peer_ends_the_pending_reader_too() {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let read_half = ours.try_clone().unwrap();
        let reader = std::thread::spawn(move || next_frame(&mut BufReader::new(read_half)));
        let (tx, writer) = spawn_writer(ours, Duration::from_millis(100)).unwrap();
        let big = "x".repeat(1 << 20);
        for _ in 0..4 {
            let _ = tx.send(big.clone());
        }
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let clean = writer.join().unwrap();
            let frame = reader.join().unwrap();
            let _ = done_tx.send((clean, frame));
        });
        let (clean, frame) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the writer and the reader both end in time");
        assert!(!clean);
        assert_eq!(frame, None);
        drop(tx);
    }

    /// Replies and events from different threads go through one writer:
    /// each line arrives whole, in the order the channel took them, while
    /// the reader on the same socket is still waiting on the peer.
    #[cfg(unix)]
    #[test]
    fn one_writer_keeps_frames_whole_while_a_read_is_pending() {
        use std::io::BufRead as _;
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let read_half = ours.try_clone().unwrap();
        let reader = std::thread::spawn(move || next_frame(&mut BufReader::new(read_half)));
        let (tx, writer) = spawn_writer(ours, Duration::from_secs(5)).unwrap();
        let mut gate = EventGate::new(tx.clone());
        let event_thread = std::thread::spawn(move || {
            for i in 0..50 {
                gate.offer(format!("{{\"method\":\"hew.event.e\",\"n\":{i}}}"));
            }
        });
        for i in 0..50 {
            tx.send(format!("{{\"id\":{i},\"result\":{{}}}}")).unwrap();
        }
        event_thread.join().unwrap();
        drop(tx);
        let mut lines = Vec::new();
        let mut peer = BufReader::new(theirs.try_clone().unwrap());
        for _ in 0..100 {
            let mut line = String::new();
            peer.read_line(&mut line).unwrap();
            let v: serde_json::Value = serde_json::from_str(&line).expect("a whole frame");
            lines.push(v);
        }
        let replies: Vec<u64> = lines.iter().filter_map(|v| v.get("id")?.as_u64()).collect();
        let events: Vec<u64> = lines.iter().filter_map(|v| v.get("n")?.as_u64()).collect();
        assert_eq!(replies, (0..50).collect::<Vec<u64>>());
        assert_eq!(events, (0..50).collect::<Vec<u64>>());
        assert!(writer.join().unwrap());
        // The peer's frame reaches the reader that waited throughout.
        std::io::Write::write_all(&mut &theirs, b"{\"id\":99}\n").unwrap();
        assert_eq!(reader.join().unwrap().as_deref(), Some("{\"id\":99}"));
    }

    // ------------------------------------------------- serve_connection

    /// A stand-in for the app: it logs what `serve_connection` asks of it
    /// and, while serving a frame, offers the event that frame caused
    /// before answering, the way the webview's `hew://api-event` can beat
    /// its `hew://api-reply`.
    #[derive(Default)]
    struct FakeHost {
        log: Mutex<Vec<String>>,
        outbound: Mutex<HashMap<u32, EventGate>>,
        /// Bytes of padding on each reply (to stall a peer that never
        /// reads).
        reply_padding: usize,
    }

    const FAKE_CONN: u32 = 7;

    impl ConnHost for FakeHost {
        type Route = ();
        fn register(&self, window_label: &str) -> (u32, ()) {
            self.log
                .lock()
                .unwrap()
                .push(format!("register {window_label}"));
            (FAKE_CONN, ())
        }
        fn gates(&self) -> std::sync::MutexGuard<'_, HashMap<u32, EventGate>> {
            self.outbound.lock().unwrap()
        }
        fn serve(&self, _: &mut (), frame: &str, write: &mut dyn FnMut(&str) -> bool) -> bool {
            self.log.lock().unwrap().push(format!("serve {frame}"));
            let id = serde_json::from_str::<serde_json::Value>(frame).unwrap()["id"].clone();
            if let Some(gate) = self.gates().get_mut(&FAKE_CONN) {
                gate.offer(format!("event for {id}"));
            }
            write(&format!("reply {id}{}", " ".repeat(self.reply_padding)))
        }
        fn unregister(&self, conn_id: u32, _: &mut ()) {
            self.log.lock().unwrap().push("unregister".into());
            self.outbound.lock().unwrap().remove(&conn_id);
        }
    }

    const HELLO: &str =
        "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hew.meta.hello\",\"params\":{\"token\":\"tok\"}}\n";

    /// A `Write` into a shared buffer, so the test reads what went out.
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The whole connection loop: the event each frame causes goes out
    /// after that frame's reply, never ahead of it; the token never
    /// reaches the host; the connection is unregistered (its gate gone)
    /// and the writer has written everything by the time the call
    /// returns.
    #[test]
    fn serve_connection_sends_each_frames_events_after_its_reply_and_drains_on_close() {
        let host = FakeHost::default();
        let input =
            format!("{HELLO}{{\"id\":1}}\n\n{{\"id\":2,\"params\":{{\"token\":\"tok\"}}}}\n");
        let sink = Sink::default();
        let out = sink.clone();
        let outcome = serve_connection(
            &host,
            std::io::Cursor::new(input.into_bytes()),
            "tok",
            "main",
            || Ok(spawn_writer_on(out, || {})),
        );
        assert_eq!(outcome, Some(true));
        assert_eq!(
            String::from_utf8(sink.0.lock().unwrap().clone()).unwrap(),
            "reply 0\nevent for 0\nreply 1\nevent for 1\nreply 2\nevent for 2\n"
        );
        let log = host.log.lock().unwrap().clone();
        assert_eq!(log.first().map(String::as_str), Some("register main"));
        assert_eq!(log.last().map(String::as_str), Some("unregister"));
        assert_eq!(log.len(), 5, "{log:?}");
        assert!(log.iter().all(|l| !l.contains("tok\"")), "{log:?}");
        assert!(host.outbound.lock().unwrap().is_empty());
    }

    /// A failed hello, or a writer that cannot start, registers nothing:
    /// the webview never hears of the connection.
    #[test]
    fn serve_connection_registers_nothing_without_a_hello_and_a_writer() {
        let host = FakeHost::default();
        let bad = "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hew.meta.hello\",\"params\":{\"token\":\"no\"}}\n";
        let outcome = serve_connection(
            &host,
            std::io::Cursor::new(bad.as_bytes()),
            "tok",
            "main",
            || Ok(spawn_writer_on(Sink::default(), || {})),
        );
        assert_eq!(outcome, None);
        let outcome = serve_connection(
            &host,
            std::io::Cursor::new(HELLO.as_bytes()),
            "tok",
            "main",
            || Err(std::io::ErrorKind::Other.into()),
        );
        assert_eq!(outcome, None);
        assert!(host.log.lock().unwrap().is_empty());
    }

    /// A peer that sends frames but never reads: the writer stalls past
    /// its timeout and shuts the socket, the loop sees the end of the
    /// stream, unregisters, and returns with an unclean writer, in time.
    #[cfg(unix)]
    #[test]
    fn serve_connection_ends_when_the_writer_gives_up_on_the_peer() {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        std::io::Write::write_all(&mut &theirs, HELLO.as_bytes()).unwrap();
        for i in 1..=4 {
            std::io::Write::write_all(&mut &theirs, format!("{{\"id\":{i}}}\n").as_bytes())
                .unwrap();
        }
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let host = FakeHost {
                reply_padding: 1 << 20,
                ..FakeHost::default()
            };
            let read_half = ours.try_clone().unwrap();
            let outcome = serve_connection(&host, BufReader::new(read_half), "tok", "main", || {
                spawn_writer(ours.try_clone()?, Duration::from_millis(100))
            });
            let log = host.log.lock().unwrap().clone();
            let _ = done_tx.send((outcome, log));
        });
        let (outcome, log) = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the connection ends in time");
        assert_eq!(outcome, Some(false));
        assert_eq!(log.last().map(String::as_str), Some("unregister"));
        drop(theirs);
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
        // A notification-shaped frame (no "id") must never be treated as
        // needing a reply. The routing itself (`live_router::classify`,
        // tested there) needs a live AppHandle to run end to end; this
        // pins the wire-level fact it is built on.
        let with_id: serde_json::Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"x"}"#).unwrap();
        let without_id: serde_json::Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"x"}"#).unwrap();
        assert!(with_id.get("id").is_some());
        assert!(without_id.get("id").is_none());
    }
}
