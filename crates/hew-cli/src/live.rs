//! `--live` (docs/HEW_API.md §11.2, §12): the client side of the local
//! socket transport. `hew-cli` never listens — it discovers a running
//! desktop instance's discovery file, connects, sends the mandatory
//! token-bearing `hew.meta.hello` as the very first wire frame, and from
//! then on forwards whatever envelope the headless (embedded) path would
//! otherwise have dispatched locally.
//!
//! This module owns exactly three things: discovery (finding and
//! validating `<runtime-dir>/hew/instance-*.json`), the newline-delimited
//! JSON-RPC transport (a Unix domain socket on macOS/Linux; a named pipe
//! on Windows — `crates/hew-cli`'s only Windows-specific dependency,
//! `windows-sys`, lives entirely behind this module's `#[cfg(windows)]`
//! transport, target-gated in Cargo.toml so other platforms gain nothing),
//! and `--launch` (spawning a desktop instance and polling for it).

use api::{DispatchOutcome, Profile, Request, RequestId, Response};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a raw socket connect is allowed to take (docs/HEW_API.md §12
/// implies this is a client concern; connecting to a local Unix socket is
/// normally instantaneous, so this mostly guards against a wedged/never-
/// accepting listener).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long one request is allowed to wait for its reply.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `--launch` waits for the newly spawned instance to appear and
/// answer `hello` before giving up.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// `--live [--launch] [--instance <pid>]`, parsed once in `main.rs` and
/// threaded through to whichever subcommand needs it.
#[derive(Debug, Clone, Default)]
pub struct LiveOptions {
    pub launch: bool,
    pub instance: Option<u32>,
}

/// One discovery file's contents (docs/HEW_API.md §11.2): the socket path,
/// a per-launch token, the app's pid, and its version — parsed leniently,
/// ignoring unknown fields (§4.1's posture applied to discovery too).
#[derive(Debug, Clone)]
pub struct Instance {
    pub socket: String,
    pub token: String,
    pub pid: u32,
    pub version: String,
    /// The discovery file's own path — kept only so a caller could, in
    /// principle, act on it; discovery itself already deletes stale files
    /// as it finds them.
    pub file: PathBuf,
}

/// Everything that can go wrong finding, launching, or talking to a live
/// instance. Every variant's `Display` is a complete, user-facing
/// explanation — callers just `eprintln!("hew-cli ...: {e}")`.
#[derive(Debug)]
pub enum LiveError {
    Io(String),
    Timeout(String),
    Protocol(String),
    /// The remote's `hello` itself answered an error (bad protocol version,
    /// bad token, etc.) — carries its message.
    HelloRefused(String),
    /// No dependency-free transport exists on this platform yet.
    Unsupported(String),
    /// The connection closed before a reply arrived.
    Closed,
    NoInstances,
    Ambiguous(Vec<Instance>),
    InstanceNotFound(u32),
    LaunchFailed(String),
}

impl std::fmt::Display for LiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiveError::Io(msg) | LiveError::Timeout(msg) | LiveError::Protocol(msg) => {
                write!(f, "{msg}")
            }
            LiveError::HelloRefused(msg) => write!(f, "the running instance refused hello: {msg}"),
            LiveError::Unsupported(msg) => write!(f, "{msg}"),
            LiveError::Closed => write!(f, "the connection closed before a reply arrived"),
            LiveError::NoInstances => write!(
                f,
                "no running Hew instance was found. Start Hew and try again, or pass --launch to start it automatically."
            ),
            LiveError::Ambiguous(instances) => {
                writeln!(
                    f,
                    "multiple Hew instances are running; pass --instance <pid> to pick one:"
                )?;
                for (i, inst) in instances.iter().enumerate() {
                    let sep = if i + 1 == instances.len() { "" } else { "\n" };
                    write!(f, "  pid {} — version {}{sep}", inst.pid, inst.version)?;
                }
                Ok(())
            }
            LiveError::InstanceNotFound(pid) => {
                write!(f, "no running Hew instance has pid {pid}")
            }
            LiveError::LaunchFailed(msg) => write!(f, "--launch failed: {msg}"),
        }
    }
}

impl LiveError {
    fn io(e: std::io::Error) -> LiveError {
        LiveError::Io(e.to_string())
    }
}

// ------------------------------------------------------------- discovery

/// The runtime directory files live under (docs/HEW_API.md §11.2):
/// `$XDG_RUNTIME_DIR` (falling back to `/tmp`) on Linux, `~/Library/
/// Application Support/Hew/run` on macOS, `%LOCALAPPDATA%\Hew\run` on
/// Windows — or `HEW_RUNTIME_DIR`, an env override honored on every
/// platform as a test/dev hook (never a real deployment path).
pub fn runtime_dir() -> PathBuf {
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

#[cfg(target_os = "windows")]
fn platform_runtime_dir() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    local.join("Hew").join("run")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // §11.2 names only $XDG_RUNTIME_DIR, which is unset on plenty
            // of real Linux logins (ssh without a systemd session). A bare
            // /tmp/hew would be world-writable — any local user could
            // create it first and plant a discovery file pointing at their
            // own socket, and every envelope this client forwards would go
            // to them. The fallback is per-uid, and `owned_by_us` rejects
            // a directory or file that is not ours regardless.
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

/// Whether `path` exists and belongs to this user — the check that makes
/// a discovery entry trustworthy. §11.2's guarantee that no other user on
/// the machine can drive a document rests on the discovery directory and
/// its files being ours; anything else is another user's plant, and the
/// token inside it authenticates us TO THEM, not them to us.
#[cfg(unix)]
fn owned_by_us(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|m| m.uid() == current_uid())
}

/// Whether a discovery `socket` field is the strictly local, non-UNC pipe
/// name shape `\\.\pipe\<name>` this transport ever publishes — pure
/// string logic, so (unlike [`is_under_real_local_appdata`]) it is
/// defined unconditionally and its test runs on every platform, not only
/// Windows. Adversarial review found the Windows client trusting the
/// `socket` field verbatim: without this check, a discovery file (planted
/// by another local user, or by anything that can write under this
/// user's own `%LOCALAPPDATA%`) naming `\\attacker-host\pipe\x` would
/// have this client dial a REMOTE machine and hand it every envelope it
/// forwards — `\\.\` is the only host segment this transport's own
/// server (`shells/tauri/src-tauri/src/live.rs`'s `pipe_name`) ever
/// writes, so anything else is rejected outright, no allowlist of
/// "known-safe" remote hosts needed.
#[allow(dead_code)]
fn is_local_pipe_path(socket: &str) -> bool {
    let Some(rest) = socket.strip_prefix(r"\\.\pipe\") else {
        return false;
    };
    // A pipe's name is a single flat component in the pipe filesystem
    // namespace — it has no meaningful `..`/`/`/`\` segments the way a
    // real filesystem path might. Rejecting them outright (rather than
    // relying on the pipe namespace simply not interpreting them) means
    // this check does not depend on that namespace never growing
    // redirection semantics.
    !rest.is_empty() && !rest.contains(['\\', '/', '\0'])
}

/// Windows has no single-syscall uid-style ownership check this client
/// can use without a heavier windows-sys footprint
/// (`GetNamedSecurityInfoW` + SID comparison) than the rest of this
/// transport otherwise needs, so `path` is validated by KIND instead of
/// by a literal ownership query:
///
/// - A pipe name (`\\.\pipe\...`, the only shape the `socket` field ever
///   holds on Windows): [`is_local_pipe_path`] — rejects every UNC form
///   except the strictly local one, defeating a discovery entry that
///   names a remote pipe.
/// - Anything else (the runtime directory or an `instance-*.json` file,
///   both real filesystem paths): [`is_under_real_local_appdata`] —
///   confirms the path, once every symlink/junction in it is resolved,
///   still lies under this process's own `%LOCALAPPDATA%`.
///
/// What this does NOT check, honestly: the discretionary ACL on the
/// runtime directory or its files (unix's actual uid-ownership
/// equivalent) — a `%LOCALAPPDATA%` an administrator has made writable
/// to other users would not be caught here. See this module's residual-
/// gap note and docs/HEW_API.md §11.2 for the same caveat stated for
/// readers of the spec.
#[cfg(windows)]
fn owned_by_us(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\") {
        return is_local_pipe_path(&s);
    }
    is_under_real_local_appdata(path)
}

/// Confirms `path`, once symlinks/junctions are resolved, genuinely lies
/// under this process's own `%LOCALAPPDATA%` — see [`owned_by_us`]'s doc
/// comment for what this check does and does not stand in for.
/// `std::fs::canonicalize` on Windows resolves every reparse point along
/// the way (`GetFinalPathNameByHandleW`), so this also defeats a
/// directory anywhere in the chain being a junction/symlink planted to
/// redirect an otherwise plausible-looking path elsewhere. A path that
/// does not exist (or `%LOCALAPPDATA%` itself being unset) cannot be
/// resolved at all and is treated as untrusted rather than assumed safe.
#[cfg(windows)]
fn is_under_real_local_appdata(path: &Path) -> bool {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return false;
    };
    let Ok(local_real) = std::fs::canonicalize(&local) else {
        return false;
    };
    let Ok(path_real) = std::fs::canonicalize(path) else {
        return false;
    };
    path_real.starts_with(&local_real)
}

#[cfg(not(any(unix, windows)))]
fn owned_by_us(_path: &Path) -> bool {
    true
}

/// `<runtime-dir>/hew`, the directory discovery files actually live in
/// (docs/HEW_API.md §11.2).
fn instances_dir() -> PathBuf {
    runtime_dir().join("hew")
}

/// Enumerates every `instance-*.json` file under [`instances_dir`],
/// validate-then-use (§11.2): parses leniently, confirms the pid is
/// alive, and — for any file whose pid is dead — deletes it (a single
/// `std::fs::remove_file`, never a directory sweep) before moving on.
/// A missing directory or an unreadable/malformed file is silently
/// skipped, not an error: discovery finding nothing is the normal case
/// when no app is running.
pub fn discover() -> Vec<Instance> {
    discover_in(&instances_dir())
}

/// [`discover`]'s logic against an explicit directory — split out so
/// tests can point it at a fixture directory directly, independent of
/// `HEW_RUNTIME_DIR`/env-var races between parallel tests.
fn discover_in(dir: &Path) -> Vec<Instance> {
    // A discovery directory owned by someone else is not ours to trust —
    // and must not be swept either (deleting inside it is their business,
    // not ours). Treat it as "no instances".
    if !owned_by_us(dir) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !(name.starts_with("instance-") && name.ends_with(".json")) {
            continue;
        }
        if !owned_by_us(&path) {
            // Another user's file inside our directory (possible if the
            // directory's permissions were ever loose): never trusted,
            // never deleted.
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let (Some(socket), Some(token), Some(pid), Some(version)) = (
            value.get("socket").and_then(Value::as_str),
            value.get("token").and_then(Value::as_str),
            value.get("pid").and_then(Value::as_u64),
            value.get("version").and_then(Value::as_str),
        ) else {
            // Missing a required field — not a shape this discovery
            // protocol produces; leave it alone rather than guess.
            continue;
        };
        let pid = pid as u32;
        if !is_pid_alive(pid) {
            // Stale: the process that wrote this file is gone. Delete it
            // AND the socket it named, when that socket sits in this same
            // directory and is ours — otherwise deleting only the JSON
            // orphans the socket file permanently, since nothing else
            // knows the pairing once the entry is gone.
            let sock = Path::new(socket);
            if sock.parent() == Some(dir) && owned_by_us(sock) {
                let _ = std::fs::remove_file(sock);
            }
            let _ = std::fs::remove_file(&path);
            continue;
        }
        // The socket path comes off disk, so it is untrusted input: honor
        // it only when the socket it names is OURS. Path containment would
        // be the cruder version of this check; ownership is the property
        // that actually matters, since connecting to another user's socket
        // is what hands them every envelope this client forwards. Checked
        // AFTER the staleness sweep above, so a dead entry is still
        // cleaned up no matter what its socket field points at.
        if !owned_by_us(Path::new(socket)) {
            continue;
        }
        found.push(Instance {
            socket: socket.to_string(),
            token: token.to_string(),
            pid,
            version: version.to_string(),
            file: path,
        });
    }
    found.sort_by_key(|i| i.pid);
    found
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    // `kill -0` sends no signal; it just probes whether the pid exists and
    // is ours to signal — the standard std-only liveness check on unix.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `OpenProcess` with the narrowest right that still answers liveness
/// (`PROCESS_QUERY_LIMITED_INFORMATION` needs no special privilege, even
/// against a process owned by another user) plus `GetExitCodeProcess`,
/// rather than shelling out to `tasklist` and string-matching its NUL-
/// padded, locale-dependent table output. A pid this process cannot even
/// open (permission denied, or it never existed) is treated as dead — the
/// same "any failure means not alive" posture the unix `kill -0` check
/// takes, so a discovery entry this process cannot positively confirm is
/// never trusted either way.
#[cfg(windows)]
fn is_pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE as u32
    }
}

/// Picks one instance per `--instance <pid>` (or, absent that, requires
/// there to be exactly one candidate).
fn select(mut instances: Vec<Instance>, want_pid: Option<u32>) -> Result<Instance, LiveError> {
    if let Some(pid) = want_pid {
        return instances
            .into_iter()
            .find(|i| i.pid == pid)
            .ok_or(LiveError::InstanceNotFound(pid));
    }
    if instances.len() > 1 {
        return Err(LiveError::Ambiguous(instances));
    }
    instances.pop().ok_or(LiveError::NoInstances)
}

// -------------------------------------------------------------- transport

#[cfg(unix)]
mod transport {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    pub struct RawConn {
        stream: UnixStream,
        reader: BufReader<UnixStream>,
    }

    impl RawConn {
        pub fn connect(instance: &Instance) -> Result<RawConn, LiveError> {
            let stream = connect_with_timeout(&instance.socket, CONNECT_TIMEOUT)?;
            stream
                .set_read_timeout(Some(REPLY_TIMEOUT))
                .map_err(LiveError::io)?;
            stream
                .set_write_timeout(Some(REPLY_TIMEOUT))
                .map_err(LiveError::io)?;
            let reader_half = stream.try_clone().map_err(LiveError::io)?;
            Ok(RawConn {
                stream,
                reader: BufReader::new(reader_half),
            })
        }

        /// Writes one newline-delimited JSON-RPC frame and blocks for its
        /// reply, also one line.
        pub fn send_receive(&mut self, request: &Request) -> Result<Response, LiveError> {
            let mut line = serde_json::to_string(request)
                .map_err(|e| LiveError::Protocol(format!("encoding request: {e}")))?;
            line.push('\n');
            self.stream
                .write_all(line.as_bytes())
                .map_err(LiveError::io)?;
            self.stream.flush().map_err(LiveError::io)?;

            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf).map_err(|e| {
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) {
                    LiveError::Timeout(format!(
                        "waiting for a reply timed out after {REPLY_TIMEOUT:?}"
                    ))
                } else {
                    LiveError::io(e)
                }
            })?;
            if n == 0 {
                return Err(LiveError::Closed);
            }
            let response: Response = serde_json::from_str(buf.trim_end())
                .map_err(|e| LiveError::Protocol(format!("decoding reply: {e}")))?;
            // Correlate: a reply whose id is not the one we just sent
            // means the far side desynchronized (a timed-out dispatch
            // whose late answer arrived after we moved on). Accepting it
            // would hand the caller another command's result under its own
            // id, so refuse instead — the session is no longer trustworthy.
            if request.id.is_some() && response.id != request.id {
                return Err(LiveError::Protocol(format!(
                    "reply id {:?} does not match request id {:?} — the connection desynchronized",
                    response.id, request.id
                )));
            }
            Ok(response)
        }
    }

    /// A local Unix domain socket connect is normally instantaneous — no
    /// network handshake — but `std::os::unix::net::UnixStream` has no
    /// `connect_timeout`, so a background thread plus a bounded channel
    /// receive gives the 5s bound the spec calls for without adding a
    /// dependency.
    fn connect_with_timeout(path: &str, timeout: Duration) -> Result<UnixStream, LiveError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let owned_path = path.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(UnixStream::connect(&owned_path));
        });
        match rx.recv_timeout(timeout) {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(e)) => Err(LiveError::Io(format!("connecting to {path}: {e}"))),
            Err(_) => Err(LiveError::Timeout(format!(
                "connecting to {path} timed out after {timeout:?}"
            ))),
        }
    }
}

#[cfg(windows)]
mod transport {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, GetLastError,
        HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, ReadFile, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT, WriteFile,
    };
    use windows_sys::Win32::System::IO::CancelSynchronousIo;
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// A raw pipe `HANDLE`. `windows_sys::Win32::Foundation::HANDLE` is
    /// `*mut c_void`, which does not auto-implement `Send` — but
    /// `ReadFile`/`WriteFile`/`CloseHandle` are documented thread-agnostic
    /// (the handle names a process-wide kernel object, not a thread-local
    /// one), so it is sound to hand a copy to the short-lived reader thread
    /// [`RawConn::read_chunk_with_timeout`] spawns for each read.
    #[derive(Clone, Copy)]
    struct RawHandle(HANDLE);
    unsafe impl Send for RawHandle {}

    pub struct RawConn {
        handle: RawHandle,
        /// Bytes already pulled off the pipe but not yet consumed as a
        /// complete line — a named pipe `HANDLE` has no built-in line
        /// buffering the way `BufReader<UnixStream>` gets for free on the
        /// unix transport, so this module provides the same thing by hand.
        leftover: Vec<u8>,
    }

    impl Drop for RawConn {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle.0);
            }
        }
    }

    impl RawConn {
        /// Opens the pipe named by `instance.socket` (the discovery file's
        /// `socket` field doubles as the connect string on every platform —
        /// here it is `\\.\pipe\hew-<pid>`, not a filesystem path). Bounded
        /// by [`CONNECT_TIMEOUT`]: `CreateFileW` fails immediately with
        /// `ERROR_PIPE_BUSY` when every server-side instance is already
        /// serving a client — retried via `WaitNamedPipeW`'s native timeout
        /// until the deadline, the standard Windows named-pipe client
        /// pattern. `ERROR_FILE_NOT_FOUND` (the pipe does not exist, or no
        /// longer does) is NOT retried: unix's equivalent connect against a
        /// missing socket path fails immediately rather than waiting out
        /// the full timeout, and a vanished instance is exactly as
        /// unrecoverable here — stalling [`CONNECT_TIMEOUT`] on it would
        /// only make the caller wait longer to learn what is already known.
        /// `--launch`'s own retry loop (`launch_and_wait`, a layer above
        /// this one) is what actually needs to tolerate a not-yet-existing
        /// pipe during startup, and it already polls `discover()` +
        /// `connect` from scratch rather than looping inside a single
        /// `connect` call.
        ///
        /// The SQOS flags (`SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`)
        /// on `CreateFileW`'s `dwFlagsAndAttributes` cap the impersonation
        /// level the server side (`shells/tauri/src-tauri/src/live.rs`) is
        /// granted over this connecting thread's security context to
        /// "identify" — without them, `CreateFileW` defaults to the more
        /// permissive `SecurityImpersonation` level, letting the pipe
        /// server impersonate this client's full token rather than merely
        /// read its identity.
        pub fn connect(instance: &Instance) -> Result<RawConn, LiveError> {
            let name = wide(&instance.socket);
            let deadline = Instant::now() + CONNECT_TIMEOUT;
            loop {
                let handle = unsafe {
                    CreateFileW(
                        name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        0,
                        ptr::null_mut(),
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                        ptr::null_mut(),
                    )
                };
                if handle != INVALID_HANDLE_VALUE {
                    return Ok(RawConn {
                        handle: RawHandle(handle),
                        leftover: Vec::new(),
                    });
                }
                let err = unsafe { GetLastError() };
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(LiveError::Timeout(format!(
                        "connecting to {} timed out after {CONNECT_TIMEOUT:?}",
                        instance.socket
                    )));
                }
                if err == ERROR_PIPE_BUSY {
                    let wait_ms = remaining.as_millis().min(u128::from(u32::MAX)) as u32;
                    unsafe { WaitNamedPipeW(name.as_ptr(), wait_ms) };
                    continue;
                }
                // `ERROR_FILE_NOT_FOUND` (and everything else) fails
                // immediately, matching unix's connect against a missing
                // socket path — see this function's doc comment on why a
                // vanished instance is not retried here.
                return Err(LiveError::io(std::io::Error::from_raw_os_error(err as i32)));
            }
        }

        /// Writes one newline-delimited JSON-RPC frame and blocks for its
        /// reply, also one line — mirrors the unix transport's
        /// `send_receive` exactly, including reply-id correlation.
        pub fn send_receive(&mut self, request: &Request) -> Result<Response, LiveError> {
            let mut line = serde_json::to_string(request)
                .map_err(|e| LiveError::Protocol(format!("encoding request: {e}")))?;
            line.push('\n');
            self.write_all(line.as_bytes())?;

            let deadline = Instant::now() + REPLY_TIMEOUT;
            let line = self.read_line(deadline)?;
            let response: Response = serde_json::from_str(line.trim_end())
                .map_err(|e| LiveError::Protocol(format!("decoding reply: {e}")))?;
            if request.id.is_some() && response.id != request.id {
                return Err(LiveError::Protocol(format!(
                    "reply id {:?} does not match request id {:?} — the connection desynchronized",
                    response.id, request.id
                )));
            }
            Ok(response)
        }

        fn write_all(&mut self, mut buf: &[u8]) -> Result<(), LiveError> {
            while !buf.is_empty() {
                let mut n: u32 = 0;
                let ok = unsafe {
                    WriteFile(
                        self.handle.0,
                        buf.as_ptr(),
                        buf.len().min(u32::MAX as usize) as u32,
                        &mut n,
                        ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    let err = unsafe { GetLastError() };
                    return Err(LiveError::io(std::io::Error::from_raw_os_error(err as i32)));
                }
                buf = &buf[n as usize..];
            }
            Ok(())
        }

        /// Reads one newline-delimited frame, bounded by `deadline`
        /// (`REPLY_TIMEOUT` past the request write). A synchronous
        /// (non-overlapped) `ReadFile` has no timeout parameter of its
        /// own, so each chunk is read on a throwaway background thread —
        /// [`Self::read_chunk_with_timeout`] — that this call waits on
        /// with a bounded `recv_timeout`, exactly the "background thread +
        /// bounded channel receive" shape the unix transport's
        /// `connect_with_timeout` already uses for its own bound.
        fn read_line(&mut self, deadline: Instant) -> Result<String, LiveError> {
            loop {
                if let Some(pos) = self.leftover.iter().position(|&b| b == b'\n') {
                    let mut line: Vec<u8> = self.leftover.drain(..=pos).collect();
                    line.pop(); // the newline itself
                    return String::from_utf8(line).map_err(|e| {
                        LiveError::Protocol(format!("reply was not valid UTF-8: {e}"))
                    });
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(LiveError::Timeout(format!(
                        "waiting for a reply timed out after {REPLY_TIMEOUT:?}"
                    )));
                }
                let chunk = self.read_chunk_with_timeout(remaining)?;
                if chunk.is_empty() {
                    return Err(LiveError::Closed);
                }
                self.leftover.extend_from_slice(&chunk);
            }
        }

        /// One bounded `ReadFile` of up to 4 KiB. Unlike the unix
        /// transport (a real socket read timeout via `set_read_timeout`),
        /// a plain `HANDLE` has no such option; instead a background
        /// thread performs the blocking read while this thread waits on a
        /// channel with `recv_timeout`, and — the piece that keeps a
        /// wedged/silent peer from leaking a blocked thread for the rest
        /// of a long-running `mcp --live` session — `CancelSynchronousIo`,
        /// aimed at the READER THREAD's own native handle (not the pipe),
        /// forces its in-flight synchronous `ReadFile` to return early on
        /// timeout so the thread can exit instead of blocking forever.
        fn read_chunk_with_timeout(&mut self, timeout: Duration) -> Result<Vec<u8>, LiveError> {
            let handle = self.handle;
            let (tx, rx) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                // Rebinding forces this closure to capture the whole
                // `RawHandle` newtype (and so use its `unsafe impl Send`)
                // rather than Rust 2021's disjoint-capture rule reaching
                // straight through to the bare `HANDLE` field below, which
                // is a raw pointer and does not implement `Send` itself.
                let handle = handle;
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
                let result = if ok != 0 {
                    Ok(buf[..n as usize].to_vec())
                } else {
                    let err = unsafe { GetLastError() };
                    if err == ERROR_BROKEN_PIPE {
                        // The far end closed — the same signal a 0-byte
                        // unix `read` carries.
                        Ok(Vec::new())
                    } else {
                        Err(err)
                    }
                };
                let _ = tx.send(result);
            });
            let native = reader.as_raw_handle() as HANDLE;
            match rx.recv_timeout(timeout) {
                Ok(Ok(bytes)) => {
                    let _ = reader.join();
                    Ok(bytes)
                }
                Ok(Err(err)) => {
                    let _ = reader.join();
                    Err(LiveError::io(std::io::Error::from_raw_os_error(err as i32)))
                }
                Err(_) => {
                    // `CancelSynchronousIo` itself has a documented failure
                    // mode — `ERROR_NOT_FOUND` when it races the reader
                    // thread's own `ReadFile` call (this thread has
                    // started but not yet entered its blocking read) —
                    // that is not a real error so much as a benign timing
                    // window; its BOOL result is checked so that window is
                    // told apart from a genuine failure, but either way
                    // this function must not then fall back to an
                    // unconditional `join()`. That was the exact bug: if
                    // the cancel lands before `ReadFile` is entered, the
                    // reader thread just starts its (now uncancelled)
                    // blocking read and `join()` waits for it forever —
                    // reintroducing the hang this timeout exists to
                    // prevent. Instead, this gives the reader thread one
                    // short bounded grace period to actually finish
                    // (cancelled, or it simply completed/broke naturally)
                    // and, failing that, DETACHES it: dropping `reader`'s
                    // `JoinHandle` without joining leaves that OS thread
                    // running independently, still holding this same pipe
                    // `HANDLE` value inside its blocking `ReadFile` call,
                    // until that read eventually completes or errors —
                    // which happens once `RawConn::drop` closes the
                    // handle (or the process exits), not by anything here
                    // waiting on it.
                    let cancel_ok = unsafe { CancelSynchronousIo(native) } != 0;
                    let grace = if cancel_ok {
                        Duration::from_millis(500)
                    } else {
                        Duration::from_millis(50)
                    };
                    if rx.recv_timeout(grace).is_ok() {
                        let _ = reader.join();
                    } else {
                        drop(reader);
                    }
                    Err(LiveError::Timeout(format!(
                        "waiting for a reply timed out after {REPLY_TIMEOUT:?}"
                    )))
                }
            }
        }
    }
}

/// No transport exists for anything other than unix or Windows — a clear,
/// typed refusal rather than a compile failure on some exotic target.
#[cfg(not(any(unix, windows)))]
mod transport {
    use super::*;

    pub struct RawConn;

    impl RawConn {
        pub fn connect(_instance: &Instance) -> Result<RawConn, LiveError> {
            Err(LiveError::Unsupported(
                "live mode has no transport on this platform".to_string(),
            ))
        }

        pub fn send_receive(&mut self, _request: &Request) -> Result<Response, LiveError> {
            unreachable!("RawConn::connect always errors before a RawConn exists on this platform")
        }
    }
}

// ---------------------------------------------------------------- session

/// A connected, hello'd live connection — the token handshake is already
/// behind it, so every subsequent [`LiveSession::dispatch`] call is a
/// plain forward.
pub struct LiveSession {
    conn: transport::RawConn,
    pub instance: Instance,
    /// The remote's `hew.meta.hello` reply, verbatim (docs/HEW_API.md
    /// §4.2's app profile, protocol, documents, ...).
    pub hello_response: Response,
}

impl LiveSession {
    /// Connects to `instance` and performs the mandatory first-frame
    /// handshake (§11.2): `hello_request` must itself be a
    /// `hew.meta.hello` request — the discovery token is injected into its
    /// `params.token`, every other param preserved — and is sent as the
    /// very first frame on the wire. Fails typed if the remote's hello
    /// itself answers an error.
    fn handshake(instance: Instance, mut hello_request: Request) -> Result<LiveSession, LiveError> {
        if hello_request.method != "hew.meta.hello" {
            return Err(LiveError::Protocol(
                "the first live frame must be hew.meta.hello".to_string(),
            ));
        }
        let mut conn = transport::RawConn::connect(&instance)?;

        let mut params = hello_request.params.take().unwrap_or_else(|| json!({}));
        match &mut params {
            Value::Object(map) => {
                map.insert("token".to_string(), Value::String(instance.token.clone()));
            }
            _ => {
                params = json!({ "token": instance.token });
            }
        }
        hello_request.params = Some(params);

        let hello_response = conn.send_receive(&hello_request)?;
        if let Some(err) = &hello_response.error {
            return Err(LiveError::HelloRefused(format!(
                "{} (code {})",
                err.message, err.code
            )));
        }
        Ok(LiveSession {
            conn,
            instance,
            hello_response,
        })
    }

    /// The profile the remote granted this connection (docs/HEW_API.md
    /// §12: live connections are always `app`-granted by the application),
    /// read back from the hello reply rather than assumed, so a future
    /// remote answering something else is still reflected honestly.
    pub fn granted_profile(&self) -> Profile {
        match self
            .hello_response
            .result
            .as_ref()
            .and_then(|r| r.get("profile"))
            .and_then(Value::as_str)
        {
            Some("core") => Profile::Core,
            _ => Profile::App,
        }
    }

    /// Forwards one envelope to the remote and returns its reply, mirroring
    /// [`api::Connection::dispatch`]'s shape exactly: a notification (no
    /// `id`) is dropped unexecuted without ever touching the wire — §4.1's
    /// "no mutation ever rides a fire-and-forget frame" applies identically
    /// to a live connection.
    pub fn dispatch(&mut self, request: Request) -> Result<DispatchOutcome, LiveError> {
        if request.id.is_none() {
            return Ok(DispatchOutcome::Dropped);
        }
        let response = self.conn.send_receive(&request)?;
        Ok(DispatchOutcome::Reply(response))
    }
}

/// A `hew.meta.hello` request built by `hew-cli` itself (as opposed to one
/// coming from a `run` script, which supplies its own) — `dispatch` and
/// `mcp` both start a live connection this way.
pub fn build_hello_request(client_name: &str) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text("live-hello".to_string())),
        method: "hew.meta.hello".to_string(),
        params: Some(json!({
            "protocol": 1,
            "client": { "name": client_name, "version": env!("CARGO_PKG_VERSION") },
            "encodings": ["json"],
        })),
    }
}

/// Resolves `opts` (discovery, disambiguation, optionally `--launch`) to a
/// connected, hello'd [`LiveSession`] — the one entry point `run.rs` and
/// `mcp.rs` both call. `hello_request` is the frame that goes out first on
/// the wire, token injected (§11.2).
pub fn connect_live(opts: &LiveOptions, hello_request: Request) -> Result<LiveSession, LiveError> {
    let found = discover();
    if found.is_empty() {
        if !opts.launch {
            return Err(LiveError::NoInstances);
        }
        return launch_and_wait(opts, hello_request);
    }
    let instance = select(found, opts.instance)?;
    LiveSession::handshake(instance, hello_request)
}

/// `--launch`: spawns a desktop instance detached, then polls discovery
/// until it appears and answers `hello`, up to [`LAUNCH_TIMEOUT`]. Launch
/// command: `open -a Hew` on macOS if installed, else the executable named
/// by `HEW_APP` (documented override — no hardcoded private paths); other
/// platforms require `HEW_APP` today.
fn launch_and_wait(opts: &LiveOptions, hello_request: Request) -> Result<LiveSession, LiveError> {
    spawn_app()?;
    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    loop {
        let found = discover();
        if !found.is_empty() {
            let instance = select(found, opts.instance)?;
            match LiveSession::handshake(instance, hello_request.clone()) {
                Ok(session) => return Ok(session),
                Err(_) if Instant::now() < deadline => {
                    // The discovery file appeared but the socket isn't
                    // accepting/answering hello yet — keep polling.
                }
                Err(e) => return Err(e),
            }
        }
        if Instant::now() >= deadline {
            return Err(LiveError::LaunchFailed(format!(
                "timed out waiting for the launched instance to become ready ({LAUNCH_TIMEOUT:?})"
            )));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn spawn_app() -> Result<(), LiveError> {
    if let Ok(custom) = std::env::var("HEW_APP") {
        return std::process::Command::new(&custom)
            .spawn()
            .map(|_| ())
            .map_err(|e| {
                LiveError::LaunchFailed(format!("launching \"{custom}\" (from HEW_APP): {e}"))
            });
    }
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .args(["-a", "Hew"])
            .status()
            .map_err(|e| LiveError::LaunchFailed(format!("\"open -a Hew\": {e}")))?;
        if !status.success() {
            return Err(LiveError::LaunchFailed(
                "\"open -a Hew\" failed — is Hew installed? Set HEW_APP to override the launch command.".to_string(),
            ));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        // No invented install path: Windows has no single canonical
        // install location (NSIS installer, winget, per-user vs
        // per-machine, a portable zip, ...), so guessing one and being
        // wrong would be worse than asking. HEW_APP (checked above) is the
        // one honest answer until there's a real installed-location
        // lookup (e.g. reading the NSIS uninstall registry key) to add.
        Err(LiveError::LaunchFailed(
            "no default launch command on Windows — set HEW_APP to the full path of the installed hew.exe"
                .to_string(),
        ))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(LiveError::LaunchFailed(
            "no default launch command on this platform — set HEW_APP to the Hew executable path"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;

    /// A fresh, process-unique scratch directory under `std::env::temp_dir()`.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hew-cli-live-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir creates");
        dir
    }

    /// A short path for a real `UnixListener` bind: `std::env::temp_dir()`
    /// (already used by `scratch_dir` above for discovery-file fixtures) is
    /// too long on macOS once a test-specific subdirectory and filename are
    /// appended — `AF_UNIX` paths are capped at `SUN_LEN` (~104 bytes on
    /// macOS/BSD), so socket fixtures bind directly under `/tmp` instead.
    fn short_socket_path(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        PathBuf::from("/tmp").join(format!("hew-{tag}-{}-{n}.sock", std::process::id()))
    }

    fn write_instance_file(dir: &Path, pid: u32, socket: &str, token: &str, version: &str) {
        // Discovery only trusts an entry whose socket file is ours, so a
        // fixture must materialize the path it advertises (a plain file is
        // enough — nothing here connects).
        if !socket.is_empty() && !Path::new(socket).exists() {
            let _ = std::fs::write(socket, b"");
        }
        let path = dir.join(format!("instance-{pid}.json"));
        let body = json!({ "socket": socket, "token": token, "pid": pid, "version": version });
        std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    }

    /// A discovery entry whose socket belongs to someone else (or does
    /// not exist at all) is never trusted: connecting to it would forward
    /// every envelope to whoever owns that socket. Adversarial review
    /// found the client trusting the `socket` field verbatim.
    #[test]
    fn discover_in_skips_an_entry_whose_socket_is_not_ours() {
        let dir = scratch_dir("foreign-socket");
        let path = dir.join(format!("instance-{}.json", std::process::id()));
        // /dev/null is a real path this user does not own (root does).
        let body = json!({
            "socket": "/dev/null", "token": "t", "pid": std::process::id(), "version": "0.5.0",
        });
        std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();

        assert!(
            discover_in(&dir).is_empty(),
            "an entry pointing at a socket we do not own must not be trusted"
        );
        assert!(
            path.exists(),
            "an untrusted-but-live entry is left alone, not deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sweeping a stale entry removes its socket too — deleting only the
    /// JSON orphans the socket permanently, since nothing else knows the
    /// pairing once the entry is gone. Found while tearing down a live
    /// verification run.
    #[test]
    fn discover_in_sweeps_a_stale_entrys_socket_alongside_its_file() {
        let dir = scratch_dir("stale-socket");
        let dead_pid = 999_998_u32;
        let sock = dir.join(format!("instance-{dead_pid}.sock"));
        std::fs::write(&sock, b"").unwrap();
        write_instance_file(&dir, dead_pid, sock.to_str().unwrap(), "tok", "0.5.0");
        let json = dir.join(format!("instance-{dead_pid}.json"));

        assert!(discover_in(&dir).is_empty());
        assert!(!json.exists(), "the stale discovery file is deleted");
        assert!(!sock.exists(), "and so is the socket it named");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Staleness cleanup must not depend on the socket check: a dead pid's
    /// file is deleted whatever its socket field names (the ordering bug
    /// the socket check introduced when it ran first).
    #[test]
    fn discover_in_still_deletes_a_dead_entry_with_an_unusable_socket() {
        let dir = scratch_dir("dead-unusable");
        let dead_pid = 999_999_u32;
        let path = dir.join(format!("instance-{dead_pid}.json"));
        let body = json!({
            "socket": "/dev/null", "token": "t", "pid": dead_pid, "version": "0.5.0",
        });
        std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();

        assert!(discover_in(&dir).is_empty());
        assert!(!path.exists(), "a dead pid's file is swept regardless");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_in_ignores_a_directory_that_does_not_exist() {
        let dir = std::env::temp_dir().join("hew-cli-live-test-nonexistent-dir-really");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(discover_in(&dir).is_empty());
    }

    #[test]
    fn discover_in_finds_a_live_instance_and_ignores_non_matching_files() {
        let dir = scratch_dir("basic");
        write_instance_file(
            &dir,
            std::process::id(),
            "/tmp/whatever.sock",
            "tok-a",
            "1.2.3",
        );
        // Noise that must not be picked up.
        std::fs::write(dir.join("not-an-instance.json"), b"{}").unwrap();
        std::fs::write(dir.join("instance-not-json.txt"), b"nope").unwrap();

        let found = discover_in(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, std::process::id());
        assert_eq!(found[0].token, "tok-a");
        assert_eq!(found[0].version, "1.2.3");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_in_ignores_unknown_fields_and_a_malformed_file() {
        let dir = scratch_dir("lenient");
        let sock = short_socket_path("lenient");
        std::fs::write(&sock, b"").unwrap();
        let path = dir.join(format!("instance-{}.json", std::process::id()));
        let body = json!({
            "socket": sock, "token": "t", "pid": std::process::id(), "version": "0.5.0",
            "future_field_we_have_never_heard_of": { "nested": true },
        });
        std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
        std::fs::write(dir.join("instance-garbage.json"), b"{not json at all").unwrap();

        let found = discover_in(&dir);
        assert_eq!(
            found.len(),
            1,
            "the malformed sibling is skipped, not fatal"
        );
        assert_eq!(found[0].token, "t");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A discovery file naming a pid that is not (and, for a fixed high
    /// constant, will essentially never be) a running process is stale:
    /// discovery both excludes it AND deletes the file, per §11.2's
    /// validate-then-use contract.
    #[test]
    fn discover_in_deletes_a_stale_file_for_a_dead_pid() {
        let dir = scratch_dir("stale");
        // A pid vanishingly unlikely to be alive.
        const DEAD_PID: u32 = 999_999;
        write_instance_file(&dir, DEAD_PID, "/tmp/dead.sock", "tok-dead", "0.5.0");
        let file_path = dir.join(format!("instance-{DEAD_PID}.json"));
        assert!(file_path.exists());

        let found = discover_in(&dir);
        assert!(found.is_empty(), "a dead pid's instance is not a candidate");
        assert!(!file_path.exists(), "the stale file was deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------- windows pipe-path shape
    //
    // `is_local_pipe_path` is pure string logic and runs on every
    // platform (see its doc comment) — this is the compile-checked-
    // everywhere, run-everywhere counterpart to the Windows-only
    // `owned_by_us`/`is_under_real_local_appdata` tests further down,
    // which need a real Windows filesystem.

    #[test]
    fn is_local_pipe_path_accepts_the_documented_shape() {
        assert!(is_local_pipe_path(r"\\.\pipe\hew-1234"));
    }

    #[test]
    fn is_local_pipe_path_rejects_a_remote_unc_host() {
        assert!(!is_local_pipe_path(r"\\attacker-host\pipe\x"));
    }

    #[test]
    fn is_local_pipe_path_rejects_the_bare_dot_with_no_pipe_name() {
        assert!(!is_local_pipe_path(r"\\.\pipe\"));
    }

    #[test]
    fn is_local_pipe_path_rejects_a_non_pipe_path() {
        assert!(!is_local_pipe_path(r"C:\Windows\System32"));
        assert!(!is_local_pipe_path("/tmp/whatever.sock"));
    }

    #[test]
    fn is_local_pipe_path_rejects_embedded_separators() {
        assert!(!is_local_pipe_path(r"\\.\pipe\hew-1234\..\other"));
        assert!(!is_local_pipe_path(r"\\.\pipe\hew-1234/other"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_owned_by_us_accepts_a_local_pipe_path() {
        assert!(owned_by_us(Path::new(r"\\.\pipe\hew-1234")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_owned_by_us_rejects_a_remote_unc_pipe_path() {
        assert!(!owned_by_us(Path::new(r"\\attacker-host\pipe\x")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_is_under_real_local_appdata_accepts_local_appdata_itself() {
        let local = std::env::var_os("LOCALAPPDATA").expect("set on any real Windows session");
        assert!(is_under_real_local_appdata(Path::new(&local)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_is_under_real_local_appdata_rejects_a_path_outside_it() {
        // The Windows drive root is never under %LOCALAPPDATA%.
        let local = std::env::var_os("LOCALAPPDATA").expect("set on any real Windows session");
        let root = Path::new(&local)
            .components()
            .next()
            .map(|c| Path::new(c.as_os_str()))
            .expect("a drive-letter root component");
        assert!(!is_under_real_local_appdata(root));
    }

    #[test]
    fn select_with_zero_candidates_is_no_instances() {
        assert!(matches!(
            select(Vec::new(), None),
            Err(LiveError::NoInstances)
        ));
    }

    #[test]
    fn select_with_multiple_candidates_and_no_instance_flag_is_ambiguous() {
        let a = Instance {
            socket: "s1".into(),
            token: "t1".into(),
            pid: 100,
            version: "0.5.0".into(),
            file: PathBuf::new(),
        };
        let b = Instance {
            socket: "s2".into(),
            token: "t2".into(),
            pid: 200,
            version: "0.5.1".into(),
            file: PathBuf::new(),
        };
        let err = select(vec![a, b], None).unwrap_err();
        match err {
            LiveError::Ambiguous(instances) => assert_eq!(instances.len(), 2),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn select_with_instance_flag_picks_the_matching_pid() {
        let a = Instance {
            socket: "s1".into(),
            token: "t1".into(),
            pid: 100,
            version: "0.5.0".into(),
            file: PathBuf::new(),
        };
        let b = Instance {
            socket: "s2".into(),
            token: "t2".into(),
            pid: 200,
            version: "0.5.1".into(),
            file: PathBuf::new(),
        };
        let picked = select(vec![a, b], Some(200)).expect("pid 200 exists");
        assert_eq!(picked.pid, 200);
        assert_eq!(picked.token, "t2");
    }

    #[test]
    fn select_with_instance_flag_and_no_match_is_instance_not_found() {
        let a = Instance {
            socket: "s1".into(),
            token: "t1".into(),
            pid: 100,
            version: "0.5.0".into(),
            file: PathBuf::new(),
        };
        let err = select(vec![a], Some(999)).unwrap_err();
        assert!(matches!(err, LiveError::InstanceNotFound(999)));
    }

    // ------------------------------------------------- loopback handshake

    /// A fake desktop instance: a background thread accepts exactly one
    /// connection on a real `UnixListener`, asserts the first frame is
    /// `hew.meta.hello` carrying the expected token, replies a canned hello
    /// result, then echoes back one canned reply for whatever envelope it
    /// receives next — proving `LiveSession`'s handshake + forwarding
    /// without any real Hew app involved.
    #[cfg(unix)]
    #[test]
    fn live_session_handshakes_with_the_token_and_forwards_one_envelope() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixListener;

        let socket_path = short_socket_path("loopback");
        let listener = UnixListener::bind(&socket_path).expect("bind the fake socket");

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept the one connection");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;

            // First frame: hello, carrying the injected token.
            let mut line = String::new();
            reader.read_line(&mut line).expect("read hello frame");
            let hello: Value = serde_json::from_str(line.trim_end()).unwrap();
            assert_eq!(hello["method"], "hew.meta.hello");
            assert_eq!(hello["params"]["token"], "secret-token");
            // Other hello params are preserved through the injection.
            assert_eq!(hello["params"]["client"]["name"], "hew-cli:test");

            let hello_reply = json!({
                "jsonrpc": "2.0", "id": hello["id"],
                "result": {
                    "protocol": 1, "app": { "name": "hew", "version": "0.5.0" },
                    "profile": "app", "encoding": "json", "documents": [],
                },
            });
            writeln!(writer, "{hello_reply}").unwrap();

            // One forwarded envelope, echoed back as a canned reply.
            let mut line2 = String::new();
            reader.read_line(&mut line2).expect("read forwarded frame");
            let forwarded: Value = serde_json::from_str(line2.trim_end()).unwrap();
            assert_eq!(forwarded["method"], "hew.query.scene");

            let canned_reply = json!({
                "jsonrpc": "2.0", "id": forwarded["id"],
                "result": { "tree": [] },
            });
            writeln!(writer, "{canned_reply}").unwrap();
        });

        let instance = Instance {
            socket: socket_path.to_str().unwrap().to_string(),
            token: "secret-token".to_string(),
            pid: std::process::id(),
            version: "0.5.0".to_string(),
            file: PathBuf::new(),
        };
        let hello_request = build_hello_request("hew-cli:test");
        let mut session =
            LiveSession::handshake(instance, hello_request).expect("handshake succeeds");
        assert_eq!(session.granted_profile(), Profile::App);
        assert_eq!(
            session.hello_response.result.as_ref().unwrap()["profile"],
            "app"
        );

        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("q1".to_string())),
            method: "hew.query.scene".to_string(),
            params: Some(json!({})),
        };
        let outcome = session.dispatch(request).expect("forward succeeds");
        let DispatchOutcome::Reply(response) = outcome else {
            panic!("a request with an id must reply");
        };
        assert_eq!(response.result.unwrap()["tree"], json!([]));

        server
            .join()
            .expect("fake instance thread completes cleanly");
        let _ = std::fs::remove_file(&socket_path);
    }

    /// A notification (no `id`) is dropped without ever touching the wire —
    /// mirrors `api::Connection::dispatch`'s own contract (§4.1).
    #[cfg(unix)]
    #[test]
    fn live_session_drops_a_notification_without_writing_to_the_socket() {
        use std::io::BufReader;
        use std::os::unix::net::UnixListener;

        let socket_path = short_socket_path("loopback-notif");
        let listener = UnixListener::bind(&socket_path).expect("bind the fake socket");

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            let hello: Value = serde_json::from_str(line.trim_end()).unwrap();
            let hello_reply = json!({
                "jsonrpc": "2.0", "id": hello["id"],
                "result": { "protocol": 1, "app": {"name":"hew","version":"0.5.0"}, "profile": "app", "encoding": "json", "documents": [] },
            });
            writeln!(writer, "{hello_reply}").unwrap();
            // Nothing else should ever arrive — the test proves this by
            // simply not reading again and letting the client-side
            // assertion (Dropped, no hang) be the real check.
        });

        let instance = Instance {
            socket: socket_path.to_str().unwrap().to_string(),
            token: "tok".to_string(),
            pid: std::process::id(),
            version: "0.5.0".to_string(),
            file: PathBuf::new(),
        };
        let mut session = LiveSession::handshake(instance, build_hello_request("hew-cli:test"))
            .expect("handshake succeeds");

        let notification = Request {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "hew.event.whatever".to_string(),
            params: None,
        };
        let outcome = session
            .dispatch(notification)
            .expect("dropping never errors");
        assert!(matches!(outcome, DispatchOutcome::Dropped));

        server
            .join()
            .expect("fake instance thread completes cleanly");
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn build_hello_request_carries_protocol_1_and_the_client_name() {
        let req = build_hello_request("hew-cli:test");
        assert_eq!(req.method, "hew.meta.hello");
        let params = req.params.unwrap();
        assert_eq!(params["protocol"], 1);
        assert_eq!(params["client"]["name"], "hew-cli:test");
    }

    // -------------------------------------------------- windows transport
    //
    // These compile-check on every platform this crate targets but only
    // RUN on Windows (`cargo test -p hew-cli` on macOS/Linux silently skips
    // them, same as the `#[cfg(unix)]` loopback tests above skip on
    // Windows) — this file's module doc explains why a real Windows run is
    // still needed beyond `cargo check --target x86_64-pc-windows-msvc`.

    #[cfg(windows)]
    #[test]
    fn windows_is_pid_alive_is_true_for_the_current_process() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_is_pid_alive_is_false_for_a_pid_that_is_almost_certainly_dead() {
        assert!(!is_pid_alive(999_999));
    }

    /// The Windows analogue of `live_session_handshakes_with_the_token_and_
    /// forwards_one_envelope` above: a fake desktop instance built directly
    /// from `windows-sys` named-pipe calls (not through
    /// `shells/tauri`, a separate crate this one cannot depend on) accepts
    /// one connection, asserts the first frame is `hew.meta.hello` carrying
    /// the injected token, replies, then echoes one forwarded envelope —
    /// proving `RawConn`/`LiveSession` end to end against a real pipe.
    #[cfg(windows)]
    #[test]
    fn live_session_handshakes_over_a_named_pipe_and_forwards_one_envelope() {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
        use windows_sys::Win32::System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_WAIT,
        };

        struct SendHandle(HANDLE);
        unsafe impl Send for SendHandle {}

        fn wide(s: &str) -> Vec<u16> {
            OsStr::new(s)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }

        fn read_line(handle: HANDLE) -> String {
            let mut buf = [0u8; 4096];
            let mut n: u32 = 0;
            let ok = unsafe {
                ReadFile(
                    handle,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut n,
                    ptr::null_mut(),
                )
            };
            assert_ne!(ok, 0, "ReadFile failed");
            String::from_utf8_lossy(&buf[..n as usize])
                .trim_end()
                .to_string()
        }

        fn write_line(handle: HANDLE, line: &str) {
            let mut bytes = line.as_bytes().to_vec();
            bytes.push(b'\n');
            let mut n: u32 = 0;
            let ok = unsafe {
                WriteFile(
                    handle,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut n,
                    ptr::null_mut(),
                )
            };
            assert_ne!(ok, 0, "WriteFile failed");
        }

        let pipe_name = format!(r"\\.\pipe\hew-cli-test-{}", std::process::id());
        let name_w = wide(&pipe_name);
        let handle = unsafe {
            CreateNamedPipeW(
                name_w.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                ptr::null_mut(),
            )
        };
        assert_ne!(handle, INVALID_HANDLE_VALUE, "CreateNamedPipeW failed");
        let server_handle = SendHandle(handle);

        let server = std::thread::spawn(move || {
            // Force whole-value capture of `server_handle` (see the
            // matching comment on `RawConn::read_chunk_with_timeout`
            // above) before projecting into its raw `HANDLE` field.
            let server_handle = server_handle;
            let handle = server_handle.0;
            let connected = unsafe { ConnectNamedPipe(handle, ptr::null_mut()) };
            let _ = connected; // ERROR_PIPE_CONNECTED (already connected) is fine too

            let hello_line = read_line(handle);
            let hello: Value = serde_json::from_str(&hello_line).unwrap();
            assert_eq!(hello["method"], "hew.meta.hello");
            assert_eq!(hello["params"]["token"], "secret-token");
            assert_eq!(hello["params"]["client"]["name"], "hew-cli:test");

            write_line(
                handle,
                &json!({
                    "jsonrpc": "2.0", "id": hello["id"],
                    "result": {
                        "protocol": 1, "app": { "name": "hew", "version": "0.5.0" },
                        "profile": "app", "encoding": "json", "documents": [],
                    },
                })
                .to_string(),
            );

            let forwarded_line = read_line(handle);
            let forwarded: Value = serde_json::from_str(&forwarded_line).unwrap();
            assert_eq!(forwarded["method"], "hew.query.scene");
            write_line(
                handle,
                &json!({
                    "jsonrpc": "2.0", "id": forwarded["id"],
                    "result": { "tree": [] },
                })
                .to_string(),
            );

            unsafe { CloseHandle(handle) };
        });

        let instance = Instance {
            socket: pipe_name,
            token: "secret-token".to_string(),
            pid: std::process::id(),
            version: "0.5.0".to_string(),
            file: PathBuf::new(),
        };
        let hello_request = build_hello_request("hew-cli:test");
        let mut session =
            LiveSession::handshake(instance, hello_request).expect("handshake succeeds");
        assert_eq!(session.granted_profile(), Profile::App);

        let request = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Text("q1".to_string())),
            method: "hew.query.scene".to_string(),
            params: Some(json!({})),
        };
        let outcome = session.dispatch(request).expect("forward succeeds");
        let DispatchOutcome::Reply(response) = outcome else {
            panic!("a request with an id must reply");
        };
        assert_eq!(response.result.unwrap()["tree"], json!([]));

        server
            .join()
            .expect("fake instance thread completes cleanly");
    }
}
